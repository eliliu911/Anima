"""
真口型引擎：wav2lip 重绘嘴部，三段体态各自预处理。

## 和被它替掉的那套的区别

`engine_video.py` 用 `grid_sample` 对一个写死的矩形做拉伸 —— 那是**重采样
已有像素**。闭着的嘴里没有牙齿和口腔，再怎么调参数也变不出来。
实测张到最大和闭嘴只差 3.3 个灰阶，肉眼分不出，看起来就是「只是在播视频」。

wav2lip 是把下半张脸**重画**出来的，所以能长出牙齿。

## 时间原点

口型对不对齐，全看 `start()` 是什么时候调的。Rust 侧的播放器在**交出第一个
采样**时报 `PlaybackSignal::Started` —— 那才是音频真正开始响的时刻，
而不是"合成完了"或"入队了"。用别的时刻当原点，嘴型会整体偏几百毫秒。

## 渲染跑在线程里

MPS 推理是阻塞的（batch=1 实测 12ms）。16fps 的预算是 62ms，够用，
但不能占着 asyncio 的事件循环 —— 那会让 WebSocket 收发一起卡。
"""

from __future__ import annotations

import io
import logging
import threading
import time
from pathlib import Path

import cv2
import librosa
import numpy as np
import soundfile as sf
import torch

from wav2lip.audio import MEL_STEP_SIZE, SAMPLE_RATE, chunk_for_frame, melspectrogram
from wav2lip.avatar import Avatar
from wav2lip.model import load as load_model

logger = logging.getLogger("anima.digital_human.wav2lip")

# 一句话最多缓存多久的梅尔，超过就丢。防止某次没收到 start 时无限堆积
MAX_PENDING_SECONDS = 60.0

# 推流帧率上限。25 是留了余量的：单帧推理 12~21ms，25fps 的预算是 40ms
MAX_RENDER_FPS = 25.0


class Wav2LipEngine:
    def __init__(self, clips: dict[str, str], checkpoint: str, device: str, cache_root: Path):
        self.device = torch.device(device)
        self.model = load_model(checkpoint, self.device)
        self._lock = threading.Lock()

        self.avatars: dict[str, Avatar] = {}
        for name, path in clips.items():
            t = time.time()
            self.avatars[name] = Avatar.build(path, cache_root / name)
            av = self.avatars[name]
            logger.info("预处理 %s：%d 帧 @%.0ffps，人脸倾角 %+.1f°，%.1fs",
                        name, av.count, av.fps, av.angle, time.time() - t)
        if "idle" not in self.avatars:
            raise ValueError("至少要有 idle 这一段")

        self.clip = "idle"
        # 推流帧率封顶。**每一帧都要过一次模型**（实测 12~21ms），
        # 60fps 的素材照原帧率推等于每秒要算 1.26 秒 —— 永远追不上，
        # 表现是口型越拖越后。底模帧序号是按墙钟走的，降帧率只是跳过几帧，
        # 不会让动作变慢
        self.fps = min(self.avatars["idle"].fps, MAX_RENDER_FPS)
        # 帧序号在各段之间共用 —— 切段时不跳回开头，体态过渡才连得上
        self.frame_idx = 0
        self._last_tick = time.time()

        self._mels: dict[int, tuple[np.ndarray, float]] = {}   # id -> (mel, 收到的时刻)
        self._active: tuple[int, float] | None = None          # (id, 播放开始的时刻)
        # start 有可能比音频先到 —— 第一句尤其，POST 音频和播放开始几乎同时。
        # 这时候**不能把 start 丢掉**，丢了就是这一句完全没有口型
        self._pending_start: tuple[int, float] | None = None

    # ── 外部输入 ────────────────────────────────────────────

    def set_clip(self, name: str) -> None:
        if name in self.avatars and name != self.clip:
            self.clip = name

    def push_audio(self, sentence_id: int, raw: bytes) -> float:
        """收下一整句的音频（编码字节，wav/mp3 都行），算好梅尔存着。

        **在播放之前送到**，这样 `start()` 一到就能立刻出帧。
        """
        wav, sr = sf.read(io.BytesIO(raw), dtype="float32", always_2d=False)
        if wav.ndim > 1:
            wav = wav.mean(axis=1)
        if sr != SAMPLE_RATE:
            wav = librosa.resample(wav, orig_sr=sr, target_sr=SAMPLE_RATE)

        mel = melspectrogram(wav)
        now = time.time()
        with self._lock:
            self._mels[sentence_id] = (mel, now)
            # 这一句的 start 已经来过了：用它当时记下的时刻当原点，
            # 而不是现在 —— 否则口型会整体晚上一个解码时长
            if self._pending_start and self._pending_start[0] == sentence_id:
                self._active = self._pending_start
                self._pending_start = None
            # 过期的清掉：没收到 start 的句子（被打断了）不该一直占着
            self._mels = {k: v for k, v in self._mels.items() if now - v[1] < MAX_PENDING_SECONDS}
        return len(wav) / SAMPLE_RATE

    def start(self, sentence_id: int) -> bool:
        """这一句的音频**此刻**开始响。这是对齐的时间原点。"""
        now = time.time()
        with self._lock:
            if sentence_id not in self._mels:
                # 音频还在路上。记下**此刻**，等它到了再接上
                self._pending_start = (sentence_id, now)
                return False
            self._active = (sentence_id, now)
        return True

    def stop(self) -> None:
        """被打断了：立刻停口型，不要把剩下的帧播完。"""
        with self._lock:
            self._active = None
            self._pending_start = None
            self._mels.clear()

    # ── 渲染 ────────────────────────────────────────────────

    def render_frame(self) -> np.ndarray:
        av = self.avatars.get(self.clip, self.avatars["idle"])

        now = time.time()
        step = int((now - self._last_tick) * self.fps)
        if step >= 1:
            self.frame_idx += step
            self._last_tick = now
        base = self.frame_idx % av.count

        chunk = self._chunk_now(now)
        if chunk is None:
            return np.array(av.frames[base])

        face = torch.from_numpy(av.batch_input([base])).float().to(self.device)
        mel = torch.from_numpy(chunk[None]).float().unsqueeze(1).to(self.device)
        with torch.no_grad():
            out = self.model(mel, face)
        redrawn = (out.cpu().numpy().transpose(0, 2, 3, 1)[0] * 255).astype(np.uint8)
        return av.paste(base, redrawn)

    def _chunk_now(self, now: float) -> np.ndarray | None:
        """当前这一刻该用哪 16 列梅尔。没有正在播的句子就返回 None。"""
        with self._lock:
            if self._active is None:
                return None
            sentence_id, t0 = self._active
            entry = self._mels.get(sentence_id)
            if entry is None:
                self._active = None
                return None
            mel = entry[0]

        elapsed = now - t0
        # 按经过的时间去取，而不是按渲染了多少帧 —— 渲染可能掉帧，
        # 跟着帧数走会让口型越拖越后
        idx = int(elapsed * self.fps)
        chunk = chunk_for_frame(mel, idx, self.fps)
        if chunk is None or chunk.shape[1] < MEL_STEP_SIZE:
            with self._lock:
                # 这句放完了，回到闭嘴
                if self._active and self._active[0] == sentence_id:
                    self._active = None
                    self._mels.pop(sentence_id, None)
            return None
        return chunk

    @property
    def is_speaking(self) -> bool:
        with self._lock:
            return self._active is not None
