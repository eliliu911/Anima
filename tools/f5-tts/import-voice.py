#!/usr/bin/env python3
"""从一个语音素材包里挑出最适合当参考的一条，装进 voices/。

    uv run python import-voice.py ~/Downloads/八重神子 yaemiko

## 为什么需要挑，而不是随便拿一条

零样本克隆的相似度**几乎完全取决于这一条参考音频**。素材包动辄上千条
（用户那份八重神子是 1265 条，时长 0.3~47.1 秒），随手拿一条大概率踩到：

- **太短**（<4 秒）：模型没听够，音色抓不稳
- **太长**（>12 秒）：F5-TTS 会截断，而且参考越长推理越慢 —— 参考音频的长度
  是要算进每次合成的
- **太吵 / 削波**：参考里的噪声会被一起克隆进去
- **静音占比高**：有效信息其实很少
- **带情绪爆发**（喊、笑）：那个语气会渗进之后所有的合成

所以这里按可量化的指标打分，把最合适的那条选出来，并且把前几名都打印出来 ——
听感这件事最终得你自己定，脚本只负责把候选缩到三五条。

## `.lab` 就是转写稿

素材包普遍用 `.lab` 存文字（每个 wav 配一个同名 lab）。另一种 HTK 风格的
"起止时间 + 音素"对齐标注也叫 `.lab`，那种不能用 —— 里面是音素不是句子。
这里会判形状并跳过。
"""

from __future__ import annotations

import sys
from pathlib import Path

import numpy as np
import soundfile as sf

# F5-TTS 参考音频的理想区间（秒）。
#
# 下限 4：再短模型抓不稳音色。上限 12：F5-TTS 会截断更长的参考，
# 而且参考长度直接加进每次合成的耗时 —— 12 秒的参考比 6 秒的慢一倍。
MIN_SEC = 4.0
MAX_SEC = 12.0
IDEAL_SEC = 8.0

# 字数下限。同样长度里字更多说明语速正常、不是一堆停顿
MIN_CHARS = 8


def read_ref_text(path: Path) -> str | None:
    """读转写稿，挡住 HTK 风格的对齐标注。"""
    raw = path.read_text(encoding="utf-8", errors="replace").strip()
    if not raw:
        return None
    lines = [ln for ln in raw.splitlines() if ln.strip()]
    if len(lines) > 2:
        def aligned(ln: str) -> bool:
            p = ln.split()
            return len(p) >= 3 and p[0].isdigit() and p[1].isdigit()

        if sum(aligned(ln) for ln in lines) > len(lines) * 0.8:
            return None
    return " ".join(ln.strip() for ln in lines)


def score(path: Path, text: str) -> tuple[float, dict] | None:
    """给一条素材打分。返回 None 表示直接淘汰。"""
    try:
        audio, sr = sf.read(path, dtype="float32", always_2d=True)
    except Exception:
        return None
    x = audio.mean(axis=1)
    dur = x.size / sr
    if not (MIN_SEC <= dur <= MAX_SEC):
        return None
    if len(text) < MIN_CHARS:
        return None

    peak = float(np.abs(x).max())
    if peak <= 0:
        return None
    # 削波：满幅采样占比。参考里的削波会被一起克隆
    clipped = float((np.abs(x) > 0.995).mean())
    if clipped > 0.0005:
        return None

    rms = float(np.sqrt((x**2).mean()))
    # 静音占比：按 20ms 窗算，低于峰值 2% 的算静音
    win = max(1, sr // 50)
    frames = x.size // win
    env = np.sqrt((x[: frames * win].reshape(frames, win) ** 2).mean(axis=1))
    silence = float((env < peak * 0.02).mean())

    # 动态范围：包络的标准差 / 均值。太大说明有喊叫或忽大忽小
    dyn = float(env.std() / max(env.mean(), 1e-9))

    # 语速：字/秒。正常中文朗读 3~6，太快太慢都不是好参考
    cps = len(text) / dur

    s = 0.0
    # 时长越接近理想值越好
    s += 3.0 * (1.0 - abs(dur - IDEAL_SEC) / (MAX_SEC - MIN_SEC))
    # 响度适中（0.06~0.15 是舒服的区间），过响过轻都扣分
    s += 2.0 * (1.0 - abs(rms - 0.10) / 0.10)
    # 静音越少越好
    s += 2.0 * (1.0 - min(silence, 0.5) / 0.5)
    # 动态范围适中
    s += 1.5 * (1.0 - min(abs(dyn - 0.8), 1.2) / 1.2)
    # 语速在 3~6 字/秒之间加分
    s += 1.5 * (1.0 - min(abs(cps - 4.5), 3.0) / 3.0)

    return s, {
        "dur": dur,
        "sr": sr,
        "rms": rms,
        "peak": peak,
        "silence": silence,
        "dyn": dyn,
        "cps": cps,
        "chars": len(text),
    }


def main() -> int:
    if len(sys.argv) < 3:
        print(__doc__)
        print("用法：import-voice.py <素材目录> <音色名> [第几名，默认 1]")
        print("     音色名可以带变体：yaemiko/calm、yaemiko/teasing")
        return 2
    src = Path(sys.argv[1]).expanduser()
    name = sys.argv[2]
    pick = int(sys.argv[3]) if len(sys.argv) > 3 else 1

    if not src.is_dir():
        print(f"找不到目录：{src}")
        return 1

    wavs = sorted(
        p for p in src.rglob("*") if p.suffix.lower() in {".wav", ".flac", ".mp3"}
    )
    if not wavs:
        print(f"{src} 里没有音频文件")
        return 1
    print(f"扫描 {len(wavs)} 条素材…")

    scored: list[tuple[float, Path, str, dict]] = []
    no_text = 0
    for w in wavs:
        text = None
        for suffix in (".lab", ".txt"):
            c = w.with_suffix(suffix)
            if c.is_file():
                text = read_ref_text(c)
                if text:
                    break
        if not text:
            no_text += 1
            continue
        got = score(w, text)
        if got:
            scored.append((got[0], w, text, got[1]))

    if no_text:
        print(f"  跳过 {no_text} 条：没有可用的转写稿（同名 .lab / .txt）")
    if not scored:
        print(f"没有一条落在 {MIN_SEC}~{MAX_SEC} 秒且指标合格 —— 放宽区间再试")
        return 1

    scored.sort(key=lambda t: -t[0])
    print(f"  合格 {len(scored)} 条，前 5 名：\n")
    for i, (s, p, text, m) in enumerate(scored[:5], 1):
        mark = "→" if i == pick else " "
        print(
            f"{mark} {i}. {s:.2f}分  {m['dur']:.1f}s  {m['sr']}Hz  "
            f"RMS {m['rms']:.3f}  静音 {m['silence']*100:.0f}%  "
            f"{m['cps']:.1f}字/秒  {p.name}"
        )
        print(f"      「{text[:60]}{'…' if len(text) > 60 else ''}」")

    if pick < 1 or pick > len(scored):
        print(f"\n没有第 {pick} 名")
        return 1

    _, chosen, text, m = scored[pick - 1]
    # 名字支持 `<人名>/<变体名>` —— 这是让情感轴选语气的路径：
    # 同一个人的 calm / teasing / sad 各一条参考，心情低就选低落的那条
    out_dir = Path(__file__).parent / "voices"
    target = out_dir / f"{name}.wav"
    target.parent.mkdir(parents=True, exist_ok=True)
    out_wav = target
    out_txt = target.with_suffix(".txt")

    # 统一转成 24kHz 单声道再写出。
    #
    # F5-TTS 内部会重采样，但**先转好有两个实际好处**：参考音频的长度直接
    # 加进每次合成的耗时（48kHz 的数据量是 24kHz 的两倍），而且写进 voices/
    # 的东西规格一致，出问题时少一个变量。
    audio, sr = sf.read(chosen, dtype="float32", always_2d=True)
    x = audio.mean(axis=1)
    if sr != 24_000:
        n = int(round(x.size * 24_000 / sr))
        # 线性重采样够用：参考音频只是给模型听音色的，不进最终输出
        idx = np.linspace(0, x.size - 1, n)
        x = np.interp(idx, np.arange(x.size), x).astype(np.float32)
        sr = 24_000
    sf.write(out_wav, x, sr, subtype="PCM_16")
    out_txt.write_text(text, encoding="utf-8")

    print(f"\n已装入音色「{name}」：")
    print(f"  {out_wav}  （{x.size / sr:.1f}s，{sr}Hz 单声道）")
    print(f"  {out_txt}  「{text}」")
    print(f"\n原始文件：{chosen}")
    print("\n重启 F5-TTS 服务后生效：./scripts/anima.sh restart")
    print(f"不满意就换一名：uv run python import-voice.py {src} {name} 2")
    # GPT-SoVITS 用的是同一条参考，但路径填在引擎配置里
    print(f"\nGPT-SoVITS 用同一条：设置里把参考音频填 {out_wav}，参考文本填上面那句")
    return 0


if __name__ == "__main__":
    sys.exit(main())
