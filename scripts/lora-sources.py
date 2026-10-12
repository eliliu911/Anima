#!/usr/bin/env python3
"""从 ComfyUI/models/loras/_anima_notes.json（界面上显示的也是它）生成 docs/LORA_SOURCES.md。

以后新装 LoRA、补了出处或示例图：改备注（界面上或直接改 json）→ 跑 `python3 scripts/lora-sources.py`。
**文档不要手改**，下次生成会被覆盖；要记的东西写进备注。"""
import json, os, datetime
L = os.path.expanduser("~/AI/ComfyUI/models/loras")
A = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
notes = json.load(open(os.path.join(L, "_anima_notes.json")))

# 用户 2026-09-17 整理的清单（按文件名搜的出处）。和备注里的地址（按文件 SHA256 在 Civitai 反查、或下载时记的）不同时，两个都列出来
import glob, re
INVENTORY = sorted(glob.glob(os.path.join(A, "docs/plans/2026-09-17-*lora-inventory/*.md")))
inv = {}
for f in INVENTORY:
    for line in open(f):
        m = re.search(r"`([^`]+)`", line); urls = re.findall(r"\((https?://[^)\s]+)\)", line)
        if line.startswith("|") and m and urls:
            inv.setdefault(m.group(1).split("/")[-1].replace(".safetensors", ""), urls[0])
def same(a, b):
    key = lambda u: u.split("?")[0].rstrip("/").split("/models/")[-1].split("/")[0]
    return key(a) == key(b)
ENGINE = {"zimage": "Z-Image Turbo", "krea": "Krea 2", "sdxl": "SDXL", "h3": "MiniMax H3（视频）", "ltx25": "LTX 2.5（视频）",
          "LTX-2.5": "LTX 2.5（视频）", "qwen": "Qwen-Image-Edit", "": "MiniMax H3（视频）"}
rows, gone = [], []
for name in sorted(notes):
    n = notes[name]; path = os.path.join(L, name)
    sub = name.split("/")[0] if "/" in name else ""
    if not os.path.exists(path):
        gone.append(name); continue
    base = os.path.join(L, "_anima_thumbs", name.rsplit(".", 1)[0])
    thumb = "视频" if os.path.exists(base + ".mp4") else ("有" if os.path.exists(base + ".jpg") else "")
    size = os.path.getsize(path) / 1e6
    src = n.get("source_url", "")
    alt = inv.get(os.path.basename(name)[:-12], "")
    alt_cell = f"[清单里的]({alt})" if alt and src and not same(src, alt) else ""
    rows.append((ENGINE.get(sub, sub), name, n.get("category", "") or "—", f"{size:,.0f} MB" if size < 1000 else f"{size/1000:.2f} GB",
                 f"[链接]({src})" if src else "**未知**", alt_cell, thumb or "—", "成人向" if n.get("nsfw") else ""))
# 没写备注的 LoRA 也列出来（不然表里看不到它存在）
for root, _, files in os.walk(L):
    if "_anima_thumbs" in root: continue
    for f in files:
        if not f.endswith(".safetensors"): continue
        rel = os.path.relpath(os.path.join(root, f), L)
        if rel not in notes:
            sub = rel.split("/")[0] if "/" in rel else ""
            rows.append((ENGINE.get(sub, sub), rel, "—", f"{os.path.getsize(os.path.join(root, f))/1e6:,.0f} MB", "**未知（没写备注）**", "", "—", ""))
rows.sort()
out = ["# LoRA 出处表", "",
       f"> **自动生成，不要手改。** 来源 `ComfyUI/models/loras/_anima_notes.json`，生成脚本 `scripts/lora-sources.py`（{datetime.date.today()}）。",
       "> 新装 LoRA / 补出处 / 补示例图 → 改备注 → 重跑脚本。示例在 `loras/_anima_thumbs/`（风格窗口显示的就是它）：",
       "> 图是 `.jpg`，视频类 LoRA 是 `.mp4`（作者样例视频；HF 仓库没有的，用作者论文项目页上对应能力的演示，控制输入和结果上下拼）。",
       "> 「出处」优先是**按文件 SHA256 在 Civitai 反查到的版本页**（哈希对上 = 就是这个文件）；「清单里的」是 2026-09-17 那几份"
       "按文件名搜的清单里写的地址，和前者不同时才列。每个 LoRA 作者说它是什么、实测效果，看那几份清单：",
       *[f"> - [{os.path.relpath(f, os.path.join(A, 'docs'))}]({os.path.relpath(f, os.path.join(A, 'docs'))})" for f in INVENTORY], "",
       f"共 {len(rows)} 个，出处未知 {sum(1 for r in rows if '未知' in r[4])} 个。", "",
       "| 引擎 | 文件（`models/loras/` 下） | 分类 | 大小 | 出处 | 清单里的（不同时） | 示例图 | |", "|---|---|---|---:|---|---|---|---|"]
out += [f"| {e} | `{n}` | {c} | {s} | {u} | {a} | {t} | {x} |" for e, n, c, s, u, a, t, x in rows]
if gone:
    out += ["", "## 备注里有、文件已经删了的", "", *[f"- `{g}`" for g in gone]]
open(os.path.join(A, "docs/LORA_SOURCES.md"), "w").write("\n".join(out) + "\n")
print(f"{len(rows)} 行，出处未知 {sum(1 for r in rows if '未知' in r[4])}，已删 {len(gone)}")
