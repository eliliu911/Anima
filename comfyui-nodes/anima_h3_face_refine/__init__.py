"""
把 **ComfyUI-H3-FaceRefine**（Carasibana）搬到 WeeTodd 的 MLX H3 上：远景脸崩的修法（2026-10-08，用户：「做 B」）。

## 为什么要这个节点

FaceRefine 的做法：逐帧跟踪人脸 → 裁到 768 画布让脸占满 → **H3 低强度重绘** → 贴回原片。
它的跟踪 / 裁切 / 贴回（`H3FaceSelect` / `H3FaceTrackCrop` / `H3FaceStitch`）是纯图像处理，直接能用；
只有中间重绘那两个节点（`H3InjectVideoLatent` / `H3PerFrameDenoise`）绑死 ComfyUI **原生** H3（PyTorch），
本机没有那套权重（删了 —— MPS 上 5 秒片要 2 小时 19 分）。

WeeTodd 已经有「对现成的 latent 加噪再去噪」：`WeeToddH3LatentHiresFix`（底下是采样器的 `refinement_source`），
**缺的只是把一段外部画面编码成 `H3Latents`**。这个节点补的就是这一步：

```text
H3FaceTrackCrop(画布 768) ─crops─▶ AnimaH3EncodeVideoLatents(缩到 512 编码) ─▶ WeeToddH3LatentHiresFix(1.5x → 768, 强度 ~0.35)
                                                                             ─▶ WeeToddH3VideoVAEDecode ─▶ H3FaceStitch ─▶ 成片
```

## 三条硬约束（都来自 WeeTodd 采样器，`sampling.py` 的 refinement 校验）

1. spatial 精修要求**目标尺寸严格大于源尺寸** —— 所以先缩到 `画布 / 1.5` 编码，精修放大 1.5 倍回到画布。
   512 × 1.5 = 768 正好（`resolve_hires_canvas` 按 32 取整，这里先核一遍对不对得上）。
2. 帧数必须在 17k+5 网格上，而且 `generation_config.duration_seconds` 要换算回同一个帧数（时长 2.5~15 秒）。
3. 源 latent 的 `transformer_spec` 必须和精修用的主干一致 —— 用同一个 components 建。

## 和原版的差别（已知更弱的地方）

- **音频**：原版全程拿干净的原声当条件（noise mask 只给视频）。WeeTodd 精修时音频也跟着去噪、最后换回原声
  （`preserve_initial_audio`）—— 口型靠低强度重绘保住源视频里的嘴型，不是被原声重新驱动。
- **逐帧强度**（脸越小重绘越狠）：没有，整段一个强度。
"""

from __future__ import annotations

import math

FPS = 24
SAMPLE_RATE = 32000
HIRES_SCALE = 1.5


def is_h3_frame_count(n: int) -> bool:
    """H3 视频 VAE 只收 17k+5 帧"""
    return n >= 5 and n % 17 == 5


def align_frames(n: int) -> int:
    """照抄 `minimax_h3_mlx.packing.align_num_frames`：往上对齐到 17k+5"""
    while n % 17 != 5:
        n += 1
    return n


def duration_for_frames(n: int) -> float:
    """找一个合法时长（2.5~15 秒），让采样器那边 `align(round(d * 24))` 换算回正好 n 帧。找不到就报清楚"""
    if not is_h3_frame_count(n):
        raise ValueError(f"帧数 {n} 不在 H3 的 17k+5 网格上 —— 用 H3FaceTrackCrop 的输出（它会补齐）")
    for d in (min(15.0, n / FPS), round(n / FPS, 3)):
        if 2.5 <= d <= 15.0 and align_frames(round(d * FPS)) == n:
            return d
    for k in range(60, 361):
        d = k / FPS
        if align_frames(k) == n:
            return d
    raise ValueError(f"{n} 帧（{n / FPS:.2f} 秒）超出 H3 能精修的 2.5~15 秒（73~362 帧）")


def hires_target(w: int, h: int, scale: float = HIRES_SCALE) -> tuple[int, int]:
    """照抄 `minimax_h3_mlx.hires_fix.resolve_hires_canvas` 的取整（不含上限检查）"""
    return (max(32, int(math.floor(w * scale / 32 + 0.5)) * 32), max(32, int(math.floor(h * scale / 32 + 0.5)) * 32))


def source_size_for(canvas_w: int, canvas_h: int) -> tuple[int, int]:
    """编码尺寸：精修放大 1.5 倍之后**正好**等于裁切画布。对不上（贴回会错位）就直接报"""
    w = max(32, round(canvas_w / HIRES_SCALE / 32) * 32)
    h = max(32, round(canvas_h / HIRES_SCALE / 32) * 32)
    if hires_target(w, h) != (canvas_w, canvas_h):
        raise ValueError(
            f"画布 {canvas_w}×{canvas_h} 没法由 32 的倍数放大 1.5 倍得到（最近是 {w}×{h} → {hires_target(w, h)}）。"
            "H3FaceTrackCrop 用 manual、画布设 768×768（或 576×576 / 960×960 这种 1.5 倍对得上的）"
        )
    return w, h


def stereo_32k(waveform, sample_rate: int, num_frames: int):
    """ComfyUI AUDIO（(批, 声道, 采样点)）→ H3 要的 (2, 采样点) 32 kHz，长度正好对上 num_frames 帧。线性插值重采样"""
    import numpy as np

    wav = np.asarray(waveform, dtype=np.float32)
    if wav.ndim == 3:
        wav = wav[0]
    if wav.ndim == 1:
        wav = wav[None]
    if wav.shape[0] == 1:
        wav = np.repeat(wav, 2, axis=0)
    wav = wav[:2]
    want = round(num_frames / FPS * SAMPLE_RATE)
    if sample_rate != SAMPLE_RATE and wav.shape[1] > 1:
        src_t = np.arange(wav.shape[1]) / float(sample_rate)
        dst_t = np.arange(round(wav.shape[1] * SAMPLE_RATE / sample_rate)) / float(SAMPLE_RATE)
        wav = np.stack([np.interp(dst_t, src_t, ch) for ch in wav]).astype(np.float32)
    if wav.shape[1] < want:
        wav = np.pad(wav, ((0, 0), (0, want - wav.shape[1])))
    return np.ascontiguousarray(wav[:, :want], dtype=np.float32)


def resized_uint8(images, w: int, h: int):
    """ComfyUI IMAGE（(帧, 高, 宽, 3)，0~1）→ (帧, h, w, 3) uint8。面积插值缩小（缩小不会糊成马赛克）"""
    import numpy as np
    import torch
    import torch.nn.functional as F

    x = images if isinstance(images, torch.Tensor) else torch.as_tensor(np.asarray(images))
    x = x.float().permute(0, 3, 1, 2)
    if (x.shape[2], x.shape[3]) != (h, w):
        x = F.interpolate(x, size=(h, w), mode="area")
    x = (x.clamp(0, 1) * 255.0).round().to(torch.uint8).permute(0, 2, 3, 1)
    return x.cpu().numpy()


class AnimaH3EncodeVideoLatents:
    """一段现成画面 + 原声 → WeeTodd `H3Latents`，接 `WeeToddH3LatentHiresFix` 做低强度重绘"""

    @classmethod
    def INPUT_TYPES(cls):
        return {
            "required": {
                "components": ("WEETODD_H3_COMPONENTS",),
                "config": ("WEETODD_H3_CONFIG",),
                "images": ("IMAGE",),
                "canvas_width": ("INT", {"default": 768, "min": 96, "max": 1920, "step": 32,
                                         "tooltip": "H3FaceTrackCrop 的画布宽（精修放大 1.5 倍后要正好等于它）"}),
                "canvas_height": ("INT", {"default": 768, "min": 96, "max": 1920, "step": 32}),
            },
            "optional": {
                "audio": ("AUDIO", {"tooltip": "原片的声音（H3FaceSelect 的 audio）。不接 = 静音"}),
            },
        }

    RETURN_TYPES = ("WEETODD_H3_LATENTS", "STRING")
    RETURN_NAMES = ("source_latents", "info")
    FUNCTION = "encode"
    CATEGORY = "Anima/H3 Face Refine"
    DESCRIPTION = "把 FaceRefine 裁出来的脸编码成 WeeTodd H3 latent（先缩到画布/1.5），接 WeeToddH3LatentHiresFix 做 1.5 倍低强度重绘。"

    def encode(self, components, config, images, canvas_width, canvas_height, audio=None):
        import json
        from dataclasses import replace

        import numpy as np
        from wee_todd_nodes.decoding import AUDIO_VAE_RUNTIME, VIDEO_VAE_RUNTIME, H3AudioVAESpec, H3VideoVAESpec
        from wee_todd_nodes.sampling import H3Latents, H3TransformerSpec

        n = int(images.shape[0])
        if (int(images.shape[2]), int(images.shape[1])) != (canvas_width, canvas_height):
            raise ValueError(f"裁出来的画面是 {int(images.shape[2])}×{int(images.shape[1])}，和填的画布 {canvas_width}×{canvas_height} 不一致")
        w, h = source_size_for(canvas_width, canvas_height)
        duration = duration_for_frames(n)
        frames = resized_uint8(images, w, h)
        video = VIDEO_VAE_RUNTIME.encode_continuation(H3VideoVAESpec.from_components(components), frames)
        if audio is not None:
            wav = stereo_32k(audio["waveform"], int(audio["sample_rate"]), n)
        else:
            wav = np.zeros((2, round(n / FPS * SAMPLE_RATE)), dtype=np.float32)
        audio_latents = AUDIO_VAE_RUNTIME.encode_continuation(H3AudioVAESpec.from_components(components), wav, num_frames=n)
        gen = replace(config, width=w, height=h, duration_seconds=duration)
        gen.validate()
        latents = H3Latents(
            video=video, audio=audio_latents, num_frames=n, width=w, height=h, fps=FPS, sample_rate=SAMPLE_RATE,
            transformer_evaluations=0, seconds_per_evaluation=0.0, total_seconds=0.0,
            transformer_spec=H3TransformerSpec.from_components(components), generation_config=gen,
        )
        info = {"frames": n, "duration_seconds": duration, "encoded": [w, h], "refine_to": list(hires_target(w, h)),
                "audio": "original" if audio is not None else "silent"}
        return (latents, json.dumps(info, ensure_ascii=False))


NODE_CLASS_MAPPINGS = {"AnimaH3EncodeVideoLatents": AnimaH3EncodeVideoLatents}
NODE_DISPLAY_NAME_MAPPINGS = {"AnimaH3EncodeVideoLatents": "Anima · H3 画面编码成 latent（FaceRefine MLX）"}
