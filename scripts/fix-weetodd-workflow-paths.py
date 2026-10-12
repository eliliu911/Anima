#!/usr/bin/env python3
"""把 WeeTodd 自带工作流里的模型路径改成本机实际位置。

**为什么需要这个**：q8-paged 权重被挪进了 `models/weetodd-q8/`
（理由见 APPLE_SILICON_VIDEO_STACK.md 7.9 —— ComfyUI 递归扫描，
放在默认位置会往原生 UNETLoader 下拉框塞 49 条分页噪声）。
代价是 WeeTodd 自带的工作流写的还是默认路径，Load 进去每个框都是红的。

**只改能对上的**：某个名字在磁盘上找得到（basename 唯一）才改，
真缺的模型原样留着 —— 把「缺文件」改成一个也不存在的路径，
只会把一个看得见的错误变成一个看不见的。

用法:
    python3 scripts/fix-weetodd-workflow-paths.py --dry-run   # 先看要改什么
    python3 scripts/fix-weetodd-workflow-paths.py             # 真改，原文件留 .bak
"""
from __future__ import annotations
import argparse, json, os, shutil, sys
from pathlib import Path

MODELS = Path("/Users/you/AI/ComfyUI/models")
WORKFLOWS = Path("/Users/you/AI/ComfyUI/custom_nodes/WeeTodd-Nodes/workflows")
SUFFIXES = (".safetensors", ".gguf", ".ckpt", ".pth", ".pt", ".onnx")


def build_index() -> dict[str, list[str]]:
    """basename → 所有匹配的 models/ 相对路径。目录和文件都要（分页权重是目录）。"""
    idx: dict[str, list[str]] = {}
    for root, dirs, files in os.walk(MODELS):
        dirs[:] = [d for d in dirs if d != ".cache"]
        rel_root = Path(root).relative_to(MODELS)
        for name in files:
            if name.endswith(SUFFIXES):
                idx.setdefault(name, []).append(str(rel_root / name))
        for name in dirs:
            # 分页权重就是一个带 paged_manifest.json 的目录
            if (Path(root) / name / "paged_manifest.json").is_file():
                idx.setdefault(name, []).append(str(rel_root / name))
    return idx


def remap(value: str, idx: dict[str, list[str]]) -> str | None:
    """只改**分页权重目录**，别的一律不动。

    WeeTodd 的解析顺序是：先按字段的 ComfyUI 分类目录找（video_vae 去
    `models/vae/`、lora 去 `models/loras/`），找不到再退到 `models/LTX-2.5/`
    和 `models/` 根（`ltx25_nodes.py:790` 起）。

    所以裸文件名的 VAE、`taeh3.safetensors` 这些**本来就解析得到** ——
    把它们改成 `vae/xxx` 是在修一个不存在的问题，而每一处不必要的改动
    都是一次把能用的东西改坏的机会。

    唯一真解析不到的是 q8 分页目录：它们在 `models/weetodd-q8/` 下，
    而 `weetodd-q8` 不是 ComfyUI 的注册分类，不在任何搜索根里。
    这是我自己挪出来的（见 APPLE_SILICON_VIDEO_STACK.md 7.9），
    所以也由这个脚本负责补上。
    """
    if not isinstance(value, str) or not value or len(value) > 200:
        return None
    if (MODELS / value).exists():
        return None                      # 已经能直接对上
    hits = idx.get(os.path.basename(value.rstrip("/")))
    if not hits or len(hits) != 1:
        return None                      # 没有、或有歧义 —— 都不猜
    target = hits[0]
    # **只认分页目录。** 其它对不上的要么本来就能按分类解析，
    # 要么是真缺文件 —— 后者改成一个别的路径只会把看得见的错误藏起来。
    if not (MODELS / target / "paged_manifest.json").is_file():
        return None
    return target if target != value else None


def walk_widgets(node: dict, idx, changes: list):
    """UI 工作流里模型名有两处：widgets_values 和 properties.models。
    只改一处会得到「跑起来用的是 A、界面校验的是 B」——上次修工作流就栽在这。"""
    wv = node.get("widgets_values")
    if isinstance(wv, list):
        for i, v in enumerate(wv):
            new = remap(v, idx)
            if new:
                wv[i] = new
                changes.append((v, new))
    models = node.get("properties", {}).get("models")
    if isinstance(models, list):
        for m in models:
            if isinstance(m, dict):
                new = remap(m.get("name", ""), idx)
                if new:
                    changes.append((m["name"], new))
                    m["name"] = new


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()

    idx = build_index()
    total_files = total_changes = 0
    for wf in sorted(WORKFLOWS.rglob("*.json")):
        try:
            doc = json.loads(wf.read_text(encoding="utf-8"))
        except json.JSONDecodeError as exc:
            print(f"  ⚠ 读不了 {wf.name}: {exc}")
            continue
        nodes = doc.get("nodes")
        if not isinstance(nodes, list):
            continue                       # 不是 UI 格式，跳过
        changes: list = []
        for node in nodes:
            walk_widgets(node, idx, changes)
        if not changes:
            continue
        total_files += 1
        total_changes += len(changes)
        print(f"\n  {wf.relative_to(WORKFLOWS)}")
        for old, new in changes:
            print(f"    {old}\n      → {new}")
        if not args.dry_run:
            bak = wf.with_suffix(".json.bak")
            if not bak.exists():
                shutil.copy2(wf, bak)
            wf.write_text(json.dumps(doc, ensure_ascii=False, indent=2), encoding="utf-8")
    verb = "会改" if args.dry_run else "已改"
    print(f"\n  {verb} {total_files} 份工作流，共 {total_changes} 处")
    return 0


if __name__ == "__main__":
    sys.exit(main())
