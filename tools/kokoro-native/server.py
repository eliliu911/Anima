"""Kokoro TTS 的 OpenAI 兼容服务 —— 原生跑，走 Metal/MPS。

## 为什么不用官方的 Docker 镜像

`ghcr.io/remsky/kokoro-fastapi-cpu` 装的是 `torch+cpu`，在 M4 Max 上实测
每句 1.5~3 秒、10 个核跑满、常驻 1.7GB，镜像本身 4.58GB。Kokoro 只有 82M
参数，这个开销完全不合理 —— 它是被 CPU 拖住的，不是模型慢。

换成本机 torch + MPS 之后实测 **30 倍实时**（0.127s 合成 3.83s 音频），
而且顺带去掉了整个 Docker 依赖。

## 接口契约

对着 `src-tauri/src/tts/openai.rs` 写的，Rust 侧零改动：

- `POST /v1/audio/speech` ← `{model, input, voice, response_format, speed}`
  → WAV 字节流。**WAV 头里的长度字段必须是真实值**：Rust 侧要靠头部算总时长
  来估词边界（`wav_duration_ms`），流式 WAV 那种 `0xFFFFFFFF` 占位会让它
  拿不到时长，于是整句没有词边界 —— 之前 Kokoro-FastAPI 就是这么坑过一次。
- `GET /v1/audio/voices` → `{"voices": [...]}`，界面上的音色下拉框读这个
- `GET /v1/models` → 健康检查用
"""

from __future__ import annotations

import io
import logging
import re
import threading
import time

import numpy as np
import soundfile as sf
import torch
from fastapi import FastAPI, HTTPException
from fastapi.responses import Response
from kokoro import KPipeline
from pydantic import BaseModel

SAMPLE_RATE = 24_000
REPO_ID = "hexgrad/Kokoro-82M"

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
log = logging.getLogger("kokoro")


def pick_device() -> str:
    """MPS 可用就用 MPS。

    退回 CPU 而不是报错：换机器、torch 装成 CPU 版都可能发生，
    那时候慢一点也比彻底不能说话好。
    """
    if torch.backends.mps.is_available():
        return "mps"
    log.warning("MPS 不可用，退回 CPU —— 合成会慢一个数量级")
    return "cpu"


DEVICE = pick_device()


def lang_of(voice: str) -> str:
    """从音色名推断语种代码。

    Kokoro 的音色名带语种前缀：`zf_xiaoyi` 是中文女声、`af_bella` 是美音女声、
    `bf_emma` 是英音。语种代码决定用哪套 G2P —— 用错的话中文会被按字母念。
    """
    prefix = voice[:1].lower()
    # a=美音 b=英音 z=中文 j=日语 e=西语 f=法语 h=印地语 i=意语 p=葡语
    return prefix if prefix in "abzjefhip" else "a"


_pipelines: dict[str, KPipeline] = {}
# KPipeline 不保证线程安全，而 uvicorn 会并发进来。
# 合成本身只有 0.1 秒级，串行完全够用 —— 加锁比排查偶发的张量错乱划算。
_lock = threading.Lock()


def pipeline_for(lang: str) -> KPipeline:
    if lang not in _pipelines:
        log.info("加载 %s 语种管线（device=%s）…", lang, DEVICE)
        t0 = time.time()
        _pipelines[lang] = KPipeline(lang_code=lang, repo_id=REPO_ID, device=DEVICE)
        log.info("%s 管线就绪，用了 %.1fs", lang, time.time() - t0)
    return _pipelines[lang]


def synthesize(text: str, voice: str, speed: float) -> np.ndarray:
    lang = lang_of(voice)
    with _lock:
        pipe = pipeline_for(lang)
        chunks = [
            (audio.detach().cpu().numpy() if hasattr(audio, "detach") else np.asarray(audio))
            for _, _, audio in pipe(text, voice=voice, speed=speed)
        ]
    if not chunks:
        return np.zeros(0, dtype=np.float32)
    return np.concatenate(chunks).astype(np.float32)


class SpeechRequest(BaseModel):
    input: str
    voice: str = "zf_xiaoyi"
    # 这三个字段协议里有、我们不真的需要，但必须能收下，
    # 否则 pydantic 会因为多余字段直接 422 —— 客户端一定会发它们
    model: str = "kokoro"
    response_format: str = "wav"
    speed: float = 1.0


app = FastAPI(title="anima-kokoro")


@app.on_event("startup")
def warm_up() -> None:
    """启动时先合成一句丢掉。

    MPS 第一次跑要编译内核，实测 7.2 秒 —— 不预热的话用户说的第一句话
    要等 7 秒，看起来就是"坏了"。第二句起 0.13 秒。
    """
    def run() -> None:
        for voice in ("zf_xiaoyi", "af_bella"):
            try:
                t0 = time.time()
                synthesize("预热", voice, 1.0)
                log.info("预热 %s 完成，用了 %.1fs", voice, time.time() - t0)
            except Exception:  # noqa: BLE001 — 预热失败不该让服务起不来
                log.exception("预热 %s 失败", voice)

    # 放到后台线程：否则端口要等十几秒才开，启动脚本会误判成"没起来"
    threading.Thread(target=run, daemon=True).start()


@app.get("/v1/models")
def models() -> dict:
    return {
        "object": "list",
        "data": [{"id": "kokoro", "object": "model", "owned_by": "hexgrad"}],
    }


# 音色文件名形如 `voices/zf_xiaoyi.pt`
_VOICE_FILE = re.compile(r"^voices/([a-z]{2}_[a-z0-9_]+)\.pt$")


@app.get("/v1/audio/voices")
def voices() -> dict:
    """从 HF 仓库的文件列表读音色名。

    不写死清单：仓库加了新音色应该自动出现。读不到就返回空数组而不是编一份 ——
    Rust 侧会把空列表显示成"需手工填音色名"，那是准确的信息。
    """
    try:
        from huggingface_hub import list_repo_files

        names = sorted(
            m.group(1) for f in list_repo_files(REPO_ID) if (m := _VOICE_FILE.match(f))
        )
    except Exception:  # noqa: BLE001
        log.exception("读取音色列表失败")
        names = []
    return {"voices": names}


@app.post("/v1/audio/speech")
def speech(req: SpeechRequest) -> Response:
    text = req.input.strip()
    if not text:
        raise HTTPException(status_code=400, detail="input 是空的")

    # 协议规定的合法区间。夹住而不是报错：上游算出越界值时，
    # 用边界值说出来比让整句话失败好
    speed = min(4.0, max(0.25, req.speed))

    t0 = time.time()
    try:
        audio = synthesize(text, req.voice, speed)
    except Exception as err:  # noqa: BLE001
        log.exception("合成失败 voice=%s", req.voice)
        # 把原因带回去 —— Rust 侧会把响应体前 300 字显示出来，
        # 只回一个 500 的话查不出是音色名写错还是别的
        raise HTTPException(status_code=500, detail=f"合成失败：{err}") from err

    if audio.size == 0:
        raise HTTPException(status_code=500, detail="合成出的音频是空的")

    buf = io.BytesIO()
    # soundfile 写完会回填真实的 RIFF/data 长度 —— Rust 侧靠它算时长
    sf.write(buf, audio, SAMPLE_RATE, format="WAV", subtype="PCM_16")
    data = buf.getvalue()

    dur = audio.size / SAMPLE_RATE
    took = time.time() - t0
    log.info(
        "合成 %d 字 → %.2fs 音频，用了 %.3fs（%.1fx 实时）voice=%s",
        len(text), dur, took, dur / took if took else 0, req.voice,
    )
    return Response(content=data, media_type="audio/wav")
