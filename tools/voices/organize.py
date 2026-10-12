#!/usr/bin/env python3
"""整理共享音色库：把原始素材归到 library/，挑出几条命名好的参考。

    uv run --project ../f5-tts python organize.py            # 整理所有人
    uv run --project ../f5-tts python organize.py yaemiko    # 只整理一个人
    uv run --project ../f5-tts python organize.py check      # 只体检，不动文件

## 音色库为什么放在工具目录之外

参考音频不是某个引擎的私有资产：F5-TTS 和 GPT-SoVITS 都靠"参考音频 + 那段音频的
文字"做零样本克隆，同一条参考该两边都能用，而不是各存一份几百 MB。混在
`tools/f5-tts/` 里还会让人以为它是 F5-TTS 的一部分，换引擎时跟着一起删掉。

位置是项目根目录的 `voice-library/`。**内容不进版本库** ——
`.gitignore` 里 `/voice-library/*` 只放行 README，因为语音包动辄 1~2GB，
一旦被 commit 就很难从 git 历史里清掉。

## 目录结构

    voices/
      <人名>/
        default.wav + default.txt     ← 参考音频（少数几条，命名过）
        calm.wav    + calm.txt
        teasing.wav + teasing.txt
        library/                       ← 原始素材库（随便多少条）
          vo_xxx.wav + vo_xxx.lab
          战斗语音 - Battle/...

**"参考"和"素材库"是两回事，必须物理分开。** 引擎只列顶层那几条 ——
一个人有 1200 个"音色"的下拉框没法用。library/ 是这个脚本的输入。

## 变体是为了语气

零样本克隆每次只用一条参考，所以"同一个人的很多条"本身不提升相似度。
但参考音频决定的不只是音色，**还有语气** —— 拿一条调笑的台词做参考，
合成出来就是调笑的语气。所以一个人该有几个语气变体，让情感轴去选。
"""

from __future__ import annotations

import os
import shutil
import sys
from pathlib import Path

import numpy as np
import soundfile as sf

# 音色库在项目里：素材就在手边，能跟项目一起浏览备份。
# 内容不进版本库 —— .gitignore 里 /voice-library/* 只放行 README
PROJECT_ROOT = Path(__file__).resolve().parents[2]
VOICES_ROOT = Path(os.environ.get("ANIMA_VOICES", PROJECT_ROOT / "voice-library"))

# 原始素材归到这个子目录。名字用 library 而不是 raw/source：
# 它在引擎眼里是"不要列出来的东西"，在这个脚本眼里是"候选池"
LIBRARY = "library"

AUDIO_SUFFIXES = {".wav", ".mp3", ".m4a", ".flac"}
TEXT_SUFFIXES = (".lab", ".txt", ".rtf")

# 参考音频的理想区间（秒）。上限 12：F5-TTS 会截断更长的，
# 而且参考长度直接加进每次合成的耗时
MIN_SEC, IDEAL_SEC, MAX_SEC = 4.0, 8.0, 12.0
MIN_CHARS = 8

# 自动挑几条变体。和引擎侧的 MAX_VARIANTS_PER_PERSON 对齐，别超过
DEFAULT_VARIANTS = 4


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


def read_ref_text(path: Path) -> str | None:
    """读转写稿：挡住 HTK 对齐标注，清掉游戏模板标记。"""
    import re

    raw = path.read_text(encoding="utf-8", errors="replace").strip()
    if not raw:
        return None
    # macOS 的 Finder / TextEdit 默认产出 RTF（"MacOS 文稿"），普通用户很难顺手
    # 建一个纯 .txt —— 用户实际卡在这里过。看内容而不是只看扩展名：
    # 有人会把 RTF 内容存成 .txt
    if raw.lstrip().startswith("{\\rtf"):
        raw = _strip_rtf(raw)
    lines = [ln for ln in raw.splitlines() if ln.strip()]
    # HTK 风格的"起 止 音素"逐行标注 —— 那不是句子，拿去当参考文本会让克隆跑偏
    if len(lines) > 2:
        def aligned(ln: str) -> bool:
            p = ln.split()
            return len(p) >= 3 and p[0].isdigit() and p[1].isdigit()

        if sum(aligned(ln) for ln in lines) > len(lines) * 0.8:
            return None
    out = " ".join(ln.strip() for ln in lines)
    # 游戏模板标记并没有被念出来：`#` 前缀、{NICKNAME} 占位符、富文本标签
    out = re.sub(r"^\s*#\s*", "", out)
    for pat in (r"\{[^{}]*\}", r"<[^<>]{1,40}>", r"\[[^\[\]]{1,40}\]"):
        out = re.sub(pat, "", out)
    out = re.sub(r"\s{2,}", " ", out).strip()
    return out or None


def score(path: Path, text: str) -> tuple[float, dict] | None:
    """给一条素材打分。None = 直接淘汰。"""
    try:
        audio, sr = sf.read(path, dtype="float32", always_2d=True)
    except Exception:
        return None
    x = audio.mean(axis=1)
    dur = x.size / sr
    if not (MIN_SEC <= dur <= MAX_SEC) or len(text) < MIN_CHARS:
        return None
    peak = float(np.abs(x).max())
    if peak <= 0:
        return None
    # 参考里的削波会被一起克隆进去
    if float((np.abs(x) > 0.995).mean()) > 0.0005:
        return None

    rms = float(np.sqrt((x**2).mean()))
    win = max(1, sr // 50)
    frames = x.size // win
    env = np.sqrt((x[: frames * win].reshape(frames, win) ** 2).mean(axis=1))
    silence = float((env < peak * 0.02).mean())
    dyn = float(env.std() / max(env.mean(), 1e-9))
    cps = len(text) / dur

    s = 3.0 * (1.0 - abs(dur - IDEAL_SEC) / (MAX_SEC - MIN_SEC))
    s += 2.0 * (1.0 - abs(rms - 0.10) / 0.10)
    s += 2.0 * (1.0 - min(silence, 0.5) / 0.5)
    s += 1.5 * (1.0 - min(abs(dyn - 0.8), 1.2) / 1.2)
    s += 1.5 * (1.0 - min(abs(cps - 4.5), 3.0) / 3.0)
    return s, {"dur": dur, "sr": sr, "rms": rms, "silence": silence, "cps": cps}


# 引擎实际用得到的高频上限（Hz）。
#
# F5-TTS 内部重采样到 24kHz（奈奎斯特 12kHz），GPT-SoVITS 是 32kHz（16kHz）。
# 所以有损压缩砍在 16kHz **无所谓**，砍在 8kHz 就真丢内容 —— 而 8~12kHz 正是
# 齿音（s/sh/x/z）所在的频段，丢了会让克隆出来的声音发闷。
USABLE_HZ = 12_000

# 判定砖墙截断的最小跌幅（dB）与截断之后的地板线。
#
# **必须找断崖，不能看"能量到哪儿为止"。** 第一版用"累积能量达到 99.5% 的频率"，
# 结果所有纯 WAV 参考都被误报 —— 那个指标测的是语音能量的自然分布
# （女声能量本来就集中在 7~12kHz 以下），不是编码截断。
#
# 校准过：48kHz 原始 wav 和最高质量 mp3 都判"无截断"，
# compression_level=0.5 检出 16.5kHz/47dB、=0.9 检出 8.0kHz/59dB。零误报。
CLIFF_DROP_DB = 30.0
CLIFF_FLOOR_DB = -70.0


def band_spectrum_db(x: np.ndarray, sr: int, band_hz: int = 500):
    """按 band_hz 分档的平均能量（dB，相对最强档）。"""
    n = 1 << 15
    seg = x[:n] if x.size >= n else np.pad(x, (0, n - x.size))
    power = np.abs(np.fft.rfft(seg * np.hanning(n))) ** 2
    freqs = np.fft.rfftfreq(n, 1 / sr)
    edges = np.arange(0, sr / 2 + band_hz, band_hz)
    vals = np.array(
        [
            power[(freqs >= lo) & (freqs < hi)].mean()
            if ((freqs >= lo) & (freqs < hi)).any()
            else 0.0
            for lo, hi in zip(edges, edges[1:])
        ]
    )
    ref = max(vals.max(), 1e-30)
    with np.errstate(divide="ignore"):
        return edges[:-1], 10 * np.log10(np.maximum(vals, 1e-30) / ref)


def find_cliff(x: np.ndarray, sr: int) -> tuple[float, float]:
    """找砖墙式截断，返回（截断频率, 跌幅 dB）。没有就返回 (0, 0)。

    判据是两条同时成立：相邻档之间跌幅够大，**而且跌下去之后一直趴在地板上** ——
    自然的高频滚降是平缓的，不会突然掉下去再也不上来。
    """
    freqs, db = band_spectrum_db(x, sr)
    best = (0.0, 0.0)
    for i in range(2, len(db) - 1):
        drop = db[i - 1] - db[i]
        if drop > best[1] and drop > CLIFF_DROP_DB and db[i:].max() < CLIFF_FLOOR_DB:
            best = (float(freqs[i]), float(drop))
    return best


def audit(audio: Path) -> str:
    """给一条现成的参考做体检，返回一行人能看懂的结论。"""
    try:
        x, sr = sf.read(audio, dtype="float32", always_2d=True)
    except Exception as err:  # noqa: BLE001
        return f"✗ 读不出来（{err}）"
    m = x.mean(axis=1)
    dur = m.size / sr
    peak = float(np.abs(m).max())
    if peak <= 0:
        return "✗ 全是静音"
    win = max(1, sr // 50)
    frames = m.size // win
    env = np.sqrt((m[: frames * win].reshape(frames, win) ** 2).mean(axis=1))
    silence = float((env < peak * 0.02).mean())
    cutoff, drop = find_cliff(m, sr)

    flags = []
    if dur < MIN_SEC:
        flags.append(f"太短({dur:.1f}s)")
    if dur > MAX_SEC:
        flags.append(f"太长({dur:.1f}s，会被截断)")
    if silence > 0.35:
        flags.append(f"静音{silence*100:.0f}%")
    if peak > 0.995:
        flags.append("削波")
    # 只有截断落在引擎用得到的范围里才是问题。
    # 砍在 16kHz 无所谓 —— F5-TTS 重采样到 24kHz 之后那部分本来就没了
    if cutoff > 0 and cutoff < USABLE_HZ:
        # 这一条最容易被忽略：文件能读、能合成，只是声音发闷
        flags.append(
            f"高频在 {cutoff/1000:.1f}kHz 被砍断（跌 {drop:.0f}dB，低码率有损压缩？会让克隆发闷）"
        )

    band = f"截断{cutoff/1000:4.1f}k" if cutoff > 0 else "无截断  "
    head = f"{dur:5.1f}s {sr}Hz 静音{silence*100:2.0f}% {band}"
    return f"{head}  {'⚠ ' + ' / '.join(flags) if flags else '✓'}"


def find_text(audio: Path) -> str | None:
    for suffix in TEXT_SUFFIXES:
        c = audio.with_suffix(suffix)
        if c.is_file():
            t = read_ref_text(c)
            if t:
                return t
    return None


def move_into_library(person: Path) -> int:
    """把顶层的原始素材挪进 library/。

    判据是**有没有配套的转写稿**：参考音频由这个脚本写出，它的 .txt 是脚本
    自己生成的；原始素材带的是 .lab。但更可靠的判据是"不在已命名变体列表里"，
    所以这里只挪那些名字看起来像原始资源 ID 的（含 `vo_` 或很长的下划线名）。
    保守起见：**只挪，不删**，挪错了自己搬回来就行。
    """
    lib = person / LIBRARY
    moved = 0
    for item in list(person.iterdir()):
        if item.name == LIBRARY:
            continue
        # 素材库里的子目录（战斗语音、其它语音…）整个挪进去
        if item.is_dir():
            lib.mkdir(exist_ok=True)
            shutil.move(str(item), str(lib / item.name))
            moved += 1
            continue
        if item.suffix.lower() not in AUDIO_SUFFIXES:
            continue
        # 已经是命名好的变体（default / calm / teasing…）就留在顶层
        if "_" not in item.stem and len(item.stem) <= 24:
            continue
        lib.mkdir(exist_ok=True)
        for suffix in (item.suffix, *TEXT_SUFFIXES):
            src = item.with_suffix(suffix)
            if src.is_file():
                shutil.move(str(src), str(lib / src.name))
        moved += 1
    return moved


def pick_variants(person: Path, count: int) -> list[tuple[str, Path, str, dict]]:
    """从 library/ 里挑 count 条当变体。

    挑的时候**刻意拉开彼此的差距**：只取分数最高的几条会拿到四条语气几乎一样的
    素材，那样变体就失去了意义。这里按分数排序后隔开取样。
    """
    lib = person / LIBRARY
    if not lib.is_dir():
        return []
    scored: list[tuple[float, Path, str, dict]] = []
    for audio in sorted(lib.rglob("*")):
        if not (audio.is_file() and audio.suffix.lower() in AUDIO_SUFFIXES):
            continue
        text = find_text(audio)
        if not text:
            continue
        got = score(audio, text)
        if got:
            scored.append((got[0], audio, text, got[1]))
    if not scored:
        return []
    scored.sort(key=lambda t: -t[0])
    # 从前 25% 里等距取样：既保证质量，又让几条之间的语气不至于雷同
    pool = scored[: max(count, len(scored) // 4)]
    step = max(1, len(pool) // count)
    chosen = pool[::step][:count]
    names = ["default", "b", "c", "d", "e", "f", "g", "h"]
    return [(names[i], p, t, m) for i, (_, p, t, m) in enumerate(chosen)]


def write_ref(dest_stem: Path, src: Path, text: str) -> float:
    """写出参考音频（统一 24kHz 单声道）与它的文字。返回时长。"""
    audio, sr = sf.read(src, dtype="float32", always_2d=True)
    x = audio.mean(axis=1)
    if sr != 24_000:
        # 线性重采样够用：参考音频只给模型听音色，不进最终输出。
        # 先转好是因为参考长度直接加进每次合成的耗时，48kHz 的数据量是两倍
        n = int(round(x.size * 24_000 / sr))
        x = np.interp(np.linspace(0, x.size - 1, n), np.arange(x.size), x).astype(
            np.float32
        )
        sr = 24_000
    sf.write(dest_stem.with_suffix(".wav"), x, sr, subtype="PCM_16")
    dest_stem.with_suffix(".txt").write_text(text, encoding="utf-8")
    return x.size / sr


def organize(person: Path, count: int) -> None:
    print(f"\n══ {person.name} ══")
    moved = move_into_library(person)
    if moved:
        print(f"  归档 {moved} 项原始素材 → {LIBRARY}/")

    refs = sorted(
        p for p in person.iterdir()
        if p.is_file() and p.suffix.lower() in AUDIO_SUFFIXES
    )
    existing = [p.stem for p in refs]
    if refs:
        print("  已有参考（保留不动）：")
        for r in refs:
            print(f"    {r.name:<20} {audit(r)}")

    picks = pick_variants(person, count)
    if not picks:
        lib = person / LIBRARY
        n = len(list(lib.rglob("*.wav"))) if lib.is_dir() else 0
        print(f"  ⚠ 没能从 {LIBRARY}/ 里挑出合格的参考（扫到 {n} 条 wav）")
        print(f"    合格条件：{MIN_SEC}~{MAX_SEC} 秒、有转写稿、无削波、≥{MIN_CHARS} 字")
        return

    for name, src, text, m in picks:
        if name in existing:
            print(f"  跳过 {name}：已存在")
            continue
        dur = write_ref(person / name, src, text)
        print(
            f"  ＋ {name:<8} {dur:.1f}s  静音 {m['silence']*100:.0f}%  "
            f"{m['cps']:.1f}字/秒  ← {src.name}"
        )
        print(f"      「{text[:52]}{'…' if len(text) > 52 else ''}」")


def main() -> int:
    VOICES_ROOT.mkdir(parents=True, exist_ok=True)
    print(f"音色库：{VOICES_ROOT}")

    # `check` 只体检，不动文件 —— 想确认"我放的这条 mp3 行不行"时用它
    args = sys.argv[1:]
    check_only = "check" in args
    wanted = [a for a in args if a != "check"]
    people = [
        p
        for p in sorted(VOICES_ROOT.iterdir())
        if p.is_dir() and (not wanted or p.name in wanted)
    ]
    if not people:
        print(f"  {VOICES_ROOT} 下没有人物目录。")
        print("  放法：voices/<人名>/ 里丢音频和同名 .lab/.txt，然后跑这个脚本")
        return 1

    for person in people:
        if check_only:
            print(f"\n══ {person.name} ══")
            refs = sorted(
                p for p in person.iterdir()
                if p.is_file() and p.suffix.lower() in AUDIO_SUFFIXES
            )
            if not refs:
                print("  没有参考音频")
            for r in refs:
                print(f"  {r.name:<20} {audit(r)}")
            continue
        organize(person, DEFAULT_VARIANTS)

    if not check_only:
        print("\n重启服务后生效：./scripts/anima.sh restart")
    return 0


if __name__ == "__main__":
    sys.exit(main())
