"""
Anima 神经流式数字人推理 Sidecar (Apple Silicon Metal / MPS 优化)
============================================================
负责管理单图活化（LivePortrait）与视频底模（MuseTalk）引擎，
通过 WebSocket 以 30~60 FPS 实时向桌面透明渲染前端推送高清渲染帧，
同时实时处理 TTS 音频分块实现毫秒级唇形同步。
"""

import asyncio
import io
import json
import logging
import os
import sys
import time
from typing import Optional, Union

import cv2
from fastapi import FastAPI, WebSocket, WebSocketDisconnect
from fastapi.middleware.cors import CORSMiddleware
from pydantic import BaseModel
import uvicorn

from pathlib import Path

from fastapi import Request

from audio_processor import AudioProcessor
from engine_portrait import PortraitEngine
from engine_video import VideoEngine
from engine_wav2lip import Wav2LipEngine

logging.basicConfig(level=logging.INFO, format="%(asctime)s [%(levelname)s] %(message)s")
logger = logging.getLogger("anima.digital_human")

app = FastAPI(title="Anima Digital Human Neural Sidecar", version="0.2.0")

app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

HERE = Path(__file__).resolve().parent
NEURAL_DIR = HERE.parent.parent / "avatars" / "neural"
CACHE_DIR = HERE / ".cache"

# 权重不进版本库（205MB）。放 models/ 下，或用环境变量指到别处
CHECKPOINT = Path(os.environ.get("ANIMA_WAV2LIP_CKPT", HERE / "models" / "wav2lip256.pth"))

# 一套体态素材：<前缀>_idle / _speaking / _listening
CLIP_STATES = ("idle", "speaking", "listening")


def resolve_clips(model_id: str, file_path: str) -> tuple[str, dict[str, str]]:
    """这个素材该按几段来用。

    **任何素材都能做口型** —— wav2lip 是逐帧工作的，底模是三段视频、一段视频
    还是一张静图，对它没区别，动的是嘴。早先只认 `_idle/_speaking/_listening`
    那套命名，于是静图和单段视频全都悄悄退回了形变引擎，
    表现是「换个形态就完全没口型，只有下巴在位移」。

    有整套三段就带体态切换，没有就只有一段 —— 口型照做。
    """
    stem = Path(model_id).stem
    suffix = Path(model_id).suffix or ".mp4"
    for state in CLIP_STATES:
        if stem.endswith(f"_{state}"):
            prefix = stem[: -len(state) - 1]
            clips = {
                s: str(NEURAL_DIR / f"{prefix}_{s}{suffix}")
                for s in CLIP_STATES
                if (NEURAL_DIR / f"{prefix}_{s}{suffix}").exists()
            }
            if "idle" in clips:
                return prefix, clips
            break
    return stem, {"idle": file_path}


class LoadAvatarRequest(BaseModel):
    model_id: str
    target_fps: Optional[int] = 30

class DigitalHumanManager:
    def __init__(self):
        self.device = "mps" if sys.platform == "darwin" else "cpu"
        self.active_engine: Optional[Union[PortraitEngine, VideoEngine]] = None
        self.audio_processor = AudioProcessor()
        self.current_model_id = ""
        self.target_fps = 30
        self.is_video = False
        self.config = {
            "fit_mode": "contain",
            "scale": 1.0,
            "offset_x": 0,
            "offset_y": 0,
            "transparent": True,
        }

        # 默认尝试加载 Hinata 或 SeeU
        self._auto_load_default()

    def _auto_load_default(self):
        base_dir = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", "avatars", "neural"))
        default_hinata = os.path.join(base_dir, "HyuugaHinata.jpg")
        if os.path.exists(default_hinata):
            try:
                self.load_avatar("neural/HyuugaHinata.jpg", default_hinata)
            except Exception as e:
                logger.warning(f"Failed to auto-load default avatar: {e}")

    def load_avatar(self, model_id: str, file_path: str):
        if not os.path.exists(file_path):
            raise FileNotFoundError(f"Avatar file not found: {file_path}")

        logger.info(f"Loading avatar [{model_id}] from {file_path} on {self.device.upper()}...")
        lower = file_path.lower()

        # **优先走真口型。** 有权重、且素材是一套 _idle/_speaking/_listening 时，
        # 用 wav2lip 重绘嘴部；否则退回原来那套（只播视频、嘴不动）
        if CHECKPOINT.exists():
            prefix, clips = resolve_clips(model_id, file_path)
            try:
                self.active_engine = Wav2LipEngine(clips, str(CHECKPOINT), self.device, CACHE_DIR / prefix)
                self.is_video = True
                self.target_fps = int(self.active_engine.fps)
                self.current_model_id = model_id
                logger.info("口型引擎就绪：%s，%d 段，%.0ffps", prefix, len(clips), self.active_engine.fps)
                return
            except Exception as err:
                # 检测不到人脸之类。**要吼出来** —— 静默退回形变引擎的话，
                # 用户看到的只是"这个形态没口型"，找不到任何线索
                logger.warning("口型引擎起不来（%s），退回形变引擎：%s", model_id, err)

        if lower.endswith((".mp4", ".mov", ".webm", ".avi", ".mkv")):
            self.active_engine = VideoEngine(file_path, device=self.device)
            self.is_video = True
        else:
            self.active_engine = PortraitEngine(file_path, device=self.device)
            self.is_video = False

        self.current_model_id = model_id
        logger.info(f"Successfully loaded avatar: {model_id} (is_video={self.is_video})")

    def process_audio(self, pcm_bytes: bytes) -> dict:
        return self.audio_processor.process_pcm_chunk(pcm_bytes)

    def render_current_frame(self, audio_features: dict) -> bytes:
        if not self.active_engine:
            # 黑色兜底帧
            img = (255 * (1 - 0.5)).astype("uint8")
            _, buf = cv2.imencode(".jpg", img)
            return buf.tobytes()

        if isinstance(self.active_engine, Wav2LipEngine):
            # 它输出的就是 BGR（OpenCV 一路读进来的），不要再转一次
            bgr_frame = self.active_engine.render_frame()
        else:
            rgb_frame = self.active_engine.render_frame(audio_features, self.config)
            bgr_frame = cv2.cvtColor(rgb_frame, cv2.COLOR_RGB2BGR)

        # 编码为 JPEG 发送
        encode_params = [int(cv2.IMWRITE_JPEG_QUALITY), 88]
        _, buf = cv2.imencode(".jpg", bgr_frame, encode_params)
        return buf.tobytes()

manager = DigitalHumanManager()

@app.get("/health")
async def health_check():
    return {
        "status": "online",
        "device": manager.device,
        "current_model": manager.current_model_id,
        "is_video": manager.is_video,
        "target_fps": manager.target_fps,
        # 引擎类型要报出来 —— 「口型没生效」时这是第一个该看的东西
        "engine": type(manager.active_engine).__name__ if manager.active_engine else None,
        "lipsync": isinstance(manager.active_engine, Wav2LipEngine),
        "checkpoint": CHECKPOINT.exists(),
    }

@app.post("/load_avatar")
async def api_load_avatar(req: LoadAvatarRequest):
    model_id = req.model_id
    base_dir = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
    
    # 查找物理文件路径
    clean_id = model_id.replace("neural/", "")
    candidates = [
        os.path.join(base_dir, "avatars", "neural", clean_id),
        os.path.join(base_dir, "avatars", clean_id),
        os.path.abspath(model_id),
    ]

    target_path = None
    for cand in candidates:
        if os.path.exists(cand):
            target_path = cand
            break

    if not target_path:
        return {"success": False, "error": f"File not found for model: {model_id}"}

    try:
        await asyncio.to_thread(manager.load_avatar, model_id, target_path)
        # **口型引擎的帧率由素材决定，不接受外部覆盖。**
        # 素材是 16fps，按 30fps 推流等于每秒多跑一倍推理（21ms×30=630ms）
        # 去发重复帧 —— 白烧一半算力，画面一点没变好
        if req.target_fps and not isinstance(manager.active_engine, Wav2LipEngine):
            manager.target_fps = req.target_fps
        return {
            "success": True,
            "model_id": model_id,
            "is_video": manager.is_video,
            "fps": manager.target_fps
        }
    except Exception as e:
        logger.error(f"Error loading avatar {model_id}: {e}")
        return {"success": False, "error": str(e)}

# ── 给 Rust 侧用的控制面 ───────────────────────────────────────
#
# 为什么是 Rust 直接调，而不是经前端转发：**时间原点在 Rust 手里**。
# 播放器交出第一个采样时才知道"音频此刻开始响了"，绕一圈到前端再回来
# 会多出一段不确定的延迟，而口型差几十毫秒就看得出来。


def _lipsync() -> Wav2LipEngine | None:
    return manager.active_engine if isinstance(manager.active_engine, Wav2LipEngine) else None


@app.post("/lipsync/sentence")
async def api_sentence(request: Request, id: int, ):
    """收一整句的音频（编码字节，wav/mp3 都行）。**要在播放之前送到。**"""
    engine = _lipsync()
    if engine is None:
        return {"ok": False, "error": "当前不是口型引擎"}
    raw = await request.body()
    try:
        seconds = await asyncio.to_thread(engine.push_audio, id, raw)
    except Exception as err:
        logger.warning("解码句子 %d 失败：%s", id, err)
        return {"ok": False, "error": str(err)}
    return {"ok": True, "seconds": seconds}


@app.post("/lipsync/started")
async def api_started(id: int):
    """这一句**此刻**开始响 —— 对齐的时间原点。"""
    engine = _lipsync()
    return {"ok": bool(engine and engine.start(id))}


@app.post("/lipsync/clip")
async def api_clip(name: str):
    """体态：idle / speaking / listening。"""
    engine = _lipsync()
    if engine:
        engine.set_clip(name)
    return {"ok": engine is not None}


@app.get("/lipsync/state")
async def api_lipsync_state():
    """现在到底在不在做口型、体态是哪一段。

    **给排查用的。** 「嘴不动」这条链路横跨 Rust → HTTP → Python → 模型，
    没有这个接口就只能从两头猜是哪一段断了。
    """
    engine = _lipsync()
    if engine is None:
        return {"lipsync": False, "speaking": False, "clip": None}
    return {
        "lipsync": True,
        "speaking": engine.is_speaking,
        "clip": engine.clip,
        # 有哪些体态可切。单段素材只有 idle —— 这时切 speaking 是无效操作，
        # 不是故障
        "clips": sorted(engine.avatars),
        # 检测到的人脸倾角。模型是在正立人脸上训练的，这个值大就说明
        # 素材是歪着的，我们靠旋转把它摆正了
        "angle": round(engine.avatars[engine.clip].angle, 1),
    }


@app.post("/lipsync/stop")
async def api_stop():
    """被打断了：立刻停口型，别把剩下的帧播完。"""
    engine = _lipsync()
    if engine:
        engine.stop()
    return {"ok": engine is not None}


def _hello() -> dict:
    engine = _lipsync()
    return {
        "type": "hello",
        "lipsync": engine is not None,
        "fps": manager.target_fps,
        "clips": sorted(engine.avatars) if engine else [],
        "model_id": manager.current_model_id,
    }


@app.websocket("/ws/stream")
async def websocket_stream_endpoint(websocket: WebSocket):
    await websocket.accept()
    logger.info("Client connected to Neural Stream WebSocket")

    latest_audio_features = {
        "energy": 0.0,
        "mouth_open": 0.0,
        "mouth_form": 0.0,
        "is_speaking": False,
    }

    async def receive_loop():
        nonlocal latest_audio_features
        try:
            while True:
                data = await websocket.receive()
                if "bytes" in data and data["bytes"]:
                    latest_audio_features = manager.process_audio(data["bytes"])
                elif "text" in data and data["text"]:
                    try:
                        msg = json.loads(data["text"])
                        mtype = msg.get("type")
                        if mtype == "config":
                            if "fps" in msg:
                                manager.target_fps = int(msg["fps"])
                            manager.config.update(msg)
                        elif mtype == "viseme":
                            # 前端直接发送的口型权重 (aa, O, E, I, U)
                            latest_audio_features["mouth_open"] = float(msg.get("mouth_open", msg.get("aa", 0.0)))
                            latest_audio_features["mouth_form"] = float(msg.get("mouth_form", 0.0))
                            latest_audio_features["is_speaking"] = latest_audio_features["mouth_open"] > 0.02
                        elif mtype == "load":
                            model_id = msg.get("model_id")
                            if model_id:
                                base_dir = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
                                clean = model_id.replace("neural/", "")
                                for cand in [
                                    os.path.join(base_dir, "avatars", "neural", clean),
                                    os.path.join(base_dir, "avatars", clean),
                                ]:
                                    if os.path.exists(cand):
                                        await asyncio.to_thread(manager.load_avatar, model_id, cand)
                                        await websocket.send_text(json.dumps(_hello()))
                                        break
                    except Exception as e:
                        logger.warning(f"Error parsing text message: {e}")
        except WebSocketDisconnect:
            logger.info("WebSocket receive loop disconnected")
        except Exception as e:
            logger.error(f"Receive loop error: {e}")

    async def send_loop():
        try:
            # **先自报能力再推帧。** 前端要靠这个决定画布该不该盖在
            # 三段体态视频上面：能做真口型才盖，只能做形变就让位 ——
            # 那种情况下盖上去等于让 speaking/listening 两段永远不出现
            await websocket.send_text(json.dumps(_hello()))
            while True:
                start_t = time.time()
                # 渲染单帧
                frame_bytes = manager.render_current_frame(latest_audio_features)
                await websocket.send_bytes(frame_bytes)

                # 帧率等待
                target_interval = 1.0 / max(10, manager.target_fps)
                calc_time = time.time() - start_t
                sleep_time = max(0.001, target_interval - calc_time)
                await asyncio.sleep(sleep_time)
        except WebSocketDisconnect:
            logger.info("WebSocket send loop disconnected")
        except Exception as e:
            logger.error(f"Send loop error: {e}")

    # 同时运行接收与发送
    recv_task = asyncio.create_task(receive_loop())
    send_task = asyncio.create_task(send_loop())

    done, pending = await asyncio.wait(
        [recv_task, send_task],
        return_when=asyncio.FIRST_COMPLETED
    )
    for task in pending:
        task.cancel()

if __name__ == "__main__":
    port = int(os.environ.get("ANIMA_DH_PORT", 8765))
    logger.info(f"Starting Anima Digital Human Neural Sidecar on http://127.0.0.1:{port}")
    uvicorn.run(app, host="127.0.0.1", port=port, log_level="info")
