"""F5-TTS 的 OpenAI 兼容服务 —— 零样本音色克隆，原生跑 + MPS。

## 为什么是 F5-TTS 而不是那些"评分更高"的模型

在 Apple Silicon 上，决定 TTS 能不能用的是**架构而不是评分**：自回归模型在 MPS
上会慢到不可用。实测（M4 Max，同一台机器，中句约 3~4 秒音频）：

| 模型 | 架构 | 合成耗时 | 实时倍率 |
|---|---|---|---|
| Kokoro | 非自回归（StyleTTS2） | 0.11s | 35× |
| **F5-TTS** | **非自回归（flow matching + DiT）** | **0.98~3.87s** | **1.1~4.5×** |
| IndexTTS2 | 自回归 + Qwen 情绪模型 | 13.7s | 0.25× |

IndexTTS2 的官方加速手段（deepspeed、flash-attn）全是 CUDA 专用。而 MPS 上的
自回归解码本身就有已知的延迟病态（实测 fp16 第二次跑出 77 秒），所以这条路
不是"调一调就能快"。

F5-TTS 和 Kokoro 同属非自回归，这是它能用的根本原因。

## 它比 Kokoro 多给了什么

**零样本音色克隆**：给一段 5~10 秒的参考音频 + 那段音频的文字，就能用那个音色
说任何话。Kokoro 只有固定的 54 个音色。

代价是慢 7~30 倍，以及**必须提供参考音频的文字**（模型要靠它对齐）。

## nfe_step 是速度旋钮

流匹配的采样步数。实测 32 → 3.87s、16 → 1.93s、8 → 0.98s，而音频长度不变。
默认取 16：那是"听不出明显退化"与"一秒出声"的折中。

## 许可

代码是 MIT，但**权重是 CC-BY-NC-4.0，禁止商用**。个人使用没问题。
"""

from __future__ import annotations

import io
import logging
import os
import pathlib
import threading
import time
from pathlib import Path

import numpy as np
import re
import soundfile as sf
import torch
from fastapi import FastAPI, HTTPException
from fastapi.responses import Response
from pydantic import BaseModel

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
log = logging.getLogger("f5tts")

# 参考音频放在**共享音色库**里，不在这个工具目录下。
#
# 理由：参考音频不是 F5-TTS 的私有资产。F5-TTS 和 GPT-SoVITS 都靠"参考音频 +
# 那段音频的文字"做零样本克隆，同一条参考应当两边都能用，而不是各存一份几百 MB。
# 混在 tools/f5-tts/ 里还会让人以为它是 F5-TTS 的一部分，换引擎时跟着一起删掉。
#
# 用固定目录而不是让请求带路径：请求里带任意文件路径等于开了一个任意文件读取的口子。
#
# 位置是项目根目录的 voice-library/（内容已 gitignore）。anima.sh 会显式传
# ANIMA_VOICES；直接手工起服务时按相对路径推断。
_PROJECT_ROOT = Path(__file__).resolve().parents[2]
VOICES_DIR = Path(os.environ.get("ANIMA_VOICES", _PROJECT_ROOT / "voice-library"))

# 素材库子目录。引擎**不列它** —— 那是 tools/voices/organize.py 的输入，
# 一个人有 1200 个"音色"的下拉框没法用
LIBRARY_DIR = "library"

# 流匹配的采样步数 —— **首包时间的主要旋钮**。
#
# 实测（M4 Max，yaemiko/default 8 秒参考，同一句 22 字）：
#
# | 参考长度 | nfe=16 | nfe=8 | nfe=4 |
# |---|---|---|---|
# | 8s | 2.91s | 1.46s | —— |
# | 5s | 1.76s | 1.18s | 0.74s |
# | 3s | 1.41s | 0.86s | 0.35s |
#
# 两件事同时决定耗时：nfe 步数（线性），以及**参考音频的长度** ——
# 它每次合成都被重新编码一遍，所以 8 秒的参考比 3 秒的贵一倍多。
# 这也是为什么 voice-library 的 README 里把参考上限定在 12 秒：
# 那不只是"会被截断"，而是每一句都要为它付钱。
#
# **默认取 8。** 用户对着四份对比样本听过，判定 8 步"够用"——
# 那是把首包从 2.91s 砍到 1.46s 的一半，白拿的。想换回音质优先：
#     ANIMA_F5_NFE=16 ./scripts/anima.sh restart
DEFAULT_NFE = int(os.environ.get("ANIMA_F5_NFE", "8"))

_model = None
# F5TTS 实例不保证线程安全，而 uvicorn 会并发进来。
# 单次合成 1~2 秒，串行够用 —— 加锁比排查偶发的张量错乱划算
_lock = threading.Lock()


def pick_device() -> str:
    if torch.backends.mps.is_available():
        return "mps"
    log.warning("MPS 不可用，退回 CPU —— 合成会慢一个数量级")
    return "cpu"


def model():
    global _model
    if _model is None:
        from f5_tts.api import F5TTS

        t0 = time.time()
        _model = F5TTS(model="F5TTS_v1_Base", device=pick_device())
        log.info("F5-TTS 就绪，用了 %.1fs（device=%s）", time.time() - t0, _model.device)
    return _model


class Voice(BaseModel):
    """一个音色 = 参考音频 + 那段音频的文字。

    文字是必需的：模型要靠它把参考音频对齐到音素上。写错会明显影响相似度。
    """

    name: str
    audio: str
    text: str


# 参考文本的扩展名。
#
# **`.lab` 必须支持**：语音素材包（游戏语音、公开数据集）普遍用 `.lab` 存转写稿，
# 每个 wav 配一个同名 lab、里面就是那句话的纯文本。用户下载的八重神子素材包
# 就是 1265 对 wav+lab。只认 `.txt` 的话每个音色都要手工改扩展名。
#
# 注意还有另一种 `.lab`：HTK 风格的"起止时间 + 音素"逐行对齐标注（DiffSinger /
# NNSVS / UTAU 用），那个**不能**当参考文本 —— 里面是音素不是句子。
# 所以读进来之后要判一下形状，见 read_ref_text。
TEXT_SUFFIXES = (".txt", ".lab", ".rtf")


# 游戏语音包的转写稿里混着模板标记，**那些字并没有被念出来**。
#
# 实测撞到过：`#早上好，{NICKNAME}。看你的气色…` —— `#` 是原神标记"这句带占位符"，
# `{NICKNAME}` 在实际语音里是玩家名（或者干脆是停顿）。把这些字当成参考文本，
# 模型会按错误的音素去对齐参考音频，克隆质量下降而且很难归因。
#
# 处理原则：**能确定没被念出来的就删掉**（标记、占位符、富文本标签），
# 剩下的原样保留 —— 猜不出来的宁可留着。
MARKUP_PATTERNS = (
    re.compile(r"\{[^{}]*\}"),          # {NICKNAME} {F#她}{M#他} 之类的占位符
    re.compile(r"<[^<>]{1,40}>"),       # <color=#FFD780>…</color> 这类富文本标签
    re.compile(r"\[[^\[\]]{1,40}\]"),    # [提示] 之类的方括号标记
)


def strip_markup(text: str) -> tuple[str, bool]:
    """去掉转写稿里的模板标记。返回（清理后的文本, 是否动过）。"""
    out = text
    # 行首的 # 是"这句带占位符"的标记，本身不发音
    out = re.sub(r"^\s*#\s*", "", out)
    for pat in MARKUP_PATTERNS:
        out = pat.sub("", out)
    # 删掉占位符后可能留下连续的标点或空白
    out = re.sub(r"\s{2,}", " ", out).strip()
    return out, out != text.strip()


def _strip_rtf(raw: str) -> str:
    """把 RTF 剥成纯文本。只做到够用 —— 参考文本只是一句话，不需要完整解析器。"""
    import re as _re

    # \uNNNN? 是真正的 Unicode 码点，后面跟一个替代字符要一起吃掉
    def _uni(m: "_re.Match[str]") -> str:
        n = int(m.group(1))
        return chr(n + 65536 if n < 0 else n)

    out = _re.sub(r"\\u(-?\d+)\??", _uni, raw)
    out = _re.sub(r"\\'([0-9a-fA-F]{2})", lambda m: chr(int(m.group(1), 16)), out)
    out = _re.sub(r"\\(par|line|tab)\b", " ", out)
    out = _re.sub(r"\\[a-zA-Z]+-?\d* ?", "", out)   # 其余控制字
    out = out.replace("\\\\", "\\").replace("{", "").replace("}", "")
    return " ".join(out.split())


def read_ref_text(path: pathlib.Path) -> str | None:
    """读参考文本：挡住 HTK 对齐标注，清掉游戏模板标记。

    HTK 标注长这样（起 止 音素，逐行）：
        0 1250000 sil
        1250000 2100000 zh
    那不是句子，拿它当 ref_text 会让克隆完全跑偏，而且症状很难归因 ——
    音色像但吐字乱。宁可拒收并说清楚。
    """
    raw = path.read_text(encoding="utf-8", errors="replace").strip()
    if not raw:
        return None
    # macOS 的 Finder / TextEdit 默认产出 RTF（"MacOS 文稿"），普通用户很难顺手
    # 建一个纯 .txt —— 用户实际卡在这里过。看内容而不是只看扩展名：
    # 有人会把 RTF 内容存成 .txt
    if raw.lstrip().startswith("{\\rtf"):
        raw = _strip_rtf(raw)
    lines = [ln for ln in raw.splitlines() if ln.strip()]
    # 多行、且每行都是"数 数 词"的形状 → HTK 对齐标注
    if len(lines) > 2:
        def looks_aligned(ln: str) -> bool:
            parts = ln.split()
            return len(parts) >= 3 and parts[0].isdigit() and parts[1].isdigit()

        if sum(looks_aligned(ln) for ln in lines) > len(lines) * 0.8:
            log.warning(
                "%s 是 HTK 风格的音素对齐标注，不是转写稿 —— 参考文本需要的是那句话本身",
                path.name,
            )
            return None
    # 正常的转写稿可能被软换行断成几行，接起来
    joined = " ".join(ln.strip() for ln in lines)
    cleaned, changed = strip_markup(joined)
    if changed:
        log.info("%s 清掉了模板标记：%r → %r", path.name, joined, cleaned)
    return cleaned or None


AUDIO_SUFFIXES = {".wav", ".mp3", ".m4a", ".flac"}

# 一个人最多暴露几个语气变体。
#
# **"素材库"和"语气变体"是两回事。** 变体是刻意挑出来、起了名字的少数几条
# （calm / teasing / sad），用来让情感轴选语气；素材库是几百上千条原始资源，
# 那是 import-voice.py 的**输入**，不该直接当音色列出来。
#
# 实测撞到过：用户把整个 1291 条的语音包拷进 voices/yaemiko/，于是音色列表里
# 冒出 1291 项，设置面板的下拉框直接没法用。
#
# 超过这个数就只暴露 default（并明确说清怎么办）—— 按文件名取前 12 个毫无意义，
# 那 12 条不是"最好的 12 条"，只是名字排在前面的。
MAX_VARIANTS_PER_PERSON = 12


_voices_cache: tuple[tuple, dict[str, Voice]] | None = None


def voices_fingerprint() -> tuple:
    """目录内容的指纹，用来决定缓存要不要失效。

    只看**文件名与 mtime**，不读内容：素材库有上千个文件，每个请求都去读一遍
    转写稿会明显拖慢 `/v1/audio/voices`（那个接口是设置面板和探活都在调的）。
    """
    if not VOICES_DIR.is_dir():
        return ()
    items = []
    for p in sorted(VOICES_DIR.iterdir()):
        targets = [p] if p.is_file() else sorted(
            # **跳过 library/**：素材库有上千个文件，每个请求都去 stat 一遍
            # 会让 /v1/audio/voices 明显变慢，而那个接口设置面板和探活都在调
            c for c in p.iterdir() if c.is_file()
        ) if p.is_dir() else []
        for t in targets:
            try:
                items.append((str(t), t.stat().st_mtime_ns))
            except OSError:
                continue
    return tuple(items)


def load_voices() -> dict[str, Voice]:
    """扫描音色目录（带缓存）。加/删/改文件会自动失效。"""
    global _voices_cache
    fp = voices_fingerprint()
    if _voices_cache is not None and _voices_cache[0] == fp:
        return _voices_cache[1]
    result = scan_voices()
    _voices_cache = (fp, result)
    return result


def scan_voices() -> dict[str, Voice]:
    """真正的扫描，**支持一层子目录**。

    两种放法都认：

        voices/anima.wav          + anima.txt        → 音色 "anima"
        voices/yaemiko/calm.wav   + calm.lab         → 音色 "yaemiko/calm"
        voices/yaemiko/teasing.wav                   → 音色 "yaemiko/teasing"

    ## 为什么要支持子目录

    F5-TTS 每次合成**只用一条**参考音频，所以"同一个人的很多条"本身不提升
    相似度 —— 它不会把多条平均起来。

    但参考音频决定的不只是音色，**还有语气**：拿一条调笑的台词做参考，
    合成出来就是调笑的语气；拿一条低落的，就是低落的。所以一个人应当有几个
    "语气变体"，而不是一条。这正是子目录的用途，也是让情感轴驱动声音的路径 ——
    心情低就选那条低落的参考。

    同一个人的目录名之外，还会注册一个不带变体的名字（`yaemiko`）指向
    **默认变体**：优先叫 default 的那条，否则按文件名排第一的。这样上游只想
    "用这个人的声音"时不必关心有几个变体。
    """
    out: dict[str, Voice] = {}
    if not VOICES_DIR.is_dir():
        return out

    def load_one(audio: pathlib.Path, name: str) -> Voice | None:
        for suffix in TEXT_SUFFIXES:
            candidate = audio.with_suffix(suffix)
            if candidate.is_file():
                text = read_ref_text(candidate)
                if text:
                    return Voice(name=name, audio=str(audio), text=text)
        log.warning(
            "音色 %s 没有可用的参考文本（同名 %s），跳过",
            audio.name,
            " 或 ".join(TEXT_SUFFIXES),
        )
        return None

    # 顶层：一条音频就是一个音色
    for audio in sorted(VOICES_DIR.iterdir()):
        if audio.is_file() and audio.suffix.lower() in AUDIO_SUFFIXES:
            v = load_one(audio, audio.stem)
            if v:
                out[v.name] = v

    # 子目录：一个人的多个语气变体。**只看顶层** —— library/ 是素材库，不列
    for person in sorted(p for p in VOICES_DIR.iterdir() if p.is_dir()):
        audios = sorted(
            a for a in person.iterdir()
            if a.is_file() and a.suffix.lower() in AUDIO_SUFFIXES
        )
        # 一整个素材库被拷进来了 —— 只认 default，其余不列
        bulk = len(audios) > MAX_VARIANTS_PER_PERSON
        if bulk:
            log.warning(
                "%s/ 里有 %d 条音频，看起来是整个素材库而不是几个语气变体。"
                "只会用其中的 default —— 想要多个变体请用 "
                "`uv run python import-voice.py <素材目录> %s/<变体名>` "
                "从里面挑出来命名。",
                person.name, len(audios), person.name,
            )
            audios = [a for a in audios if a.stem == "default"]
            if not audios:
                log.warning(
                    "%s/ 里也没有 default.wav，这个人整个跳过", person.name
                )
                continue

        variants: dict[str, Voice] = {}
        for audio in audios:
            v = load_one(audio, f"{person.name}/{audio.stem}")
            if v:
                variants[audio.stem] = v
        if not variants:
            continue
        # 素材库模式下不注册带斜杠的名字 —— 只有 `<人名>` 一项，
        # 免得下拉框里出现一个孤零零的 "yaemiko/default"
        if not bulk:
            out.update({v.name: v for v in variants.values()})
        # 不带变体的名字指向默认变体
        default = variants.get("default") or next(iter(variants.values()))
        if person.name not in out:
            out[person.name] = Voice(
                name=person.name, audio=default.audio, text=default.text
            )
    return out


class SpeechRequest(BaseModel):
    input: str
    voice: str = ""
    # 协议里有、我们不真需要，但必须收下，否则 pydantic 会 422
    model: str = "f5-tts"
    response_format: str = "wav"
    speed: float = 1.0


app = FastAPI(title="anima-f5tts")


@app.on_event("startup")
def warm_up() -> None:
    """后台预热。

    加载模型要 28 秒，第一次合成还要额外的 MPS 内核编译 —— 不预热的话用户
    说的第一句话要等半分钟。放后台线程：否则端口要等半分钟才开，
    启动脚本会误判成"没起来"。
    """

    def run() -> None:
        try:
            voices = load_voices()
            if not voices:
                log.warning("音色目录为空（%s），预热跳过合成", VOICES_DIR)
                model()
                return
            v = next(iter(voices.values()))
            t0 = time.time()
            synth("预热", v, DEFAULT_NFE, 1.0)
            log.info("预热完成，用了 %.1fs（音色 %s）", time.time() - t0, v.name)
        except Exception:  # noqa: BLE001 — 预热失败不该让服务起不来
            log.exception("预热失败")

    threading.Thread(target=run, daemon=True).start()


def synth(text: str, voice: Voice, nfe: int, speed: float) -> tuple[np.ndarray, int]:
    with _lock:
        wav, sr, _ = model().infer(
            ref_file=voice.audio,
            ref_text=voice.text,
            gen_text=text,
            nfe_step=nfe,
            speed=speed,
            remove_silence=False,
            show_info=lambda *a, **k: None,
        )
    return np.asarray(wav, dtype=np.float32), int(sr)


@app.get("/v1/models")
def models() -> dict:
    return {"object": "list", "data": [{"id": "f5-tts", "object": "model"}]}


@app.get("/v1/audio/voices")
def voices() -> dict:
    """音色列表 = 目录里有哪些参考音频。

    空列表是准确的信息（"还没放参考音频"），不编一份假清单出来。
    """
    return {"voices": sorted(load_voices().keys())}


@app.post("/v1/audio/speech")
def speech(req: SpeechRequest) -> Response:
    text = req.input.strip()
    if not text:
        raise HTTPException(status_code=400, detail="input 是空的")

    available = load_voices()
    if not available:
        raise HTTPException(
            status_code=503,
            detail=(
                f"还没有任何参考音色。往 {VOICES_DIR} 放 <名字>.wav 和同名 "
                f"<名字>.txt 或 <名字>.lab（那段音频的文字）即可。"
            ),
        )
    # 没指定就用第一个，而不是报错：刚放进一个音色时应当直接能用
    voice = available.get(req.voice) or next(iter(available.values()))
    if req.voice and req.voice not in available:
        log.warning("没有音色 %s，改用 %s", req.voice, voice.name)

    speed = min(2.0, max(0.5, req.speed))
    t0 = time.time()
    try:
        wav, sr = synth(text, voice, DEFAULT_NFE, speed)
    except Exception as err:  # noqa: BLE001
        log.exception("合成失败 voice=%s", voice.name)
        # 把原因带回去 —— 上游会把响应体前 300 字显示出来
        raise HTTPException(status_code=500, detail=f"合成失败：{err}") from err

    if wav.size == 0:
        raise HTTPException(status_code=500, detail="合成出的音频是空的")

    buf = io.BytesIO()
    # 头部长度字段必须是真实值：上游要靠它算总时长来估词边界
    sf.write(buf, wav, sr, format="WAV", subtype="PCM_16")
    dur = wav.size / sr
    took = time.time() - t0
    log.info(
        "合成 %d 字 → %.2fs 音频，用了 %.2fs（%.1fx 实时）voice=%s",
        len(text), dur, took, dur / took if took else 0, voice.name,
    )
    return Response(content=buf.getvalue(), media_type="audio/wav")
