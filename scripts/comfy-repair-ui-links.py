"""从权威的 `links` 表重建 UI 工作流里每个节点的插槽引用。

**修的是这类 bug**：改图脚本改了 `links` 记录里的来源/去向，却忘了同步
节点自己的 `inputs[].link` 和 `outputs[].links` 数组。两边一旦不一致：

- 界面按节点上的数组画线 → 看起来少了一条
- 后续脚本读 `outputs[].links` 拿到空表 → 再插节点就接到空气上

实际后果：Director 工作流里 `LTX2_NAG` 输出为空，
`LTX Director CS (2.5)` 报「缺少必需的输入: model」。

`links` 表是唯一权威（ComfyUI 前端加载时也以它为准），所以按它重建两边。
"""
import json, sys

P = sys.argv[1]
w = json.load(open(P))
N = {n["id"]: n for n in w["nodes"]}
alive = set(N)

# 先扔掉指向已删节点的悬空记录
links = [l for l in w["links"] if l[1] in alive and l[3] in alive]

before_in = sum(1 for n in w["nodes"] for i in (n.get("inputs") or []) if i.get("link") is not None)
before_out = sum(len(o.get("links") or []) for n in w["nodes"] for o in (n.get("outputs") or []))

for n in w["nodes"]:
    for i in n.get("inputs") or []:
        i["link"] = None
    for o in n.get("outputs") or []:
        o["links"] = []

for l in links:
    lid, src, sslot, tgt, tslot, _ = l[:6]
    tn, sn = N[tgt], N[src]
    if tslot < len(tn.get("inputs") or []):
        tn["inputs"][tslot]["link"] = lid
    if sslot < len(sn.get("outputs") or []):
        sn["outputs"][sslot]["links"].append(lid)

w["links"] = links
json.dump(w, open(P, "w"), ensure_ascii=False, indent=1)

after_in = sum(1 for n in w["nodes"] for i in (n.get("inputs") or []) if i.get("link") is not None)
after_out = sum(len(o.get("links") or []) for n in w["nodes"] for o in (n.get("outputs") or []))
print(f"  {P}")
print(f"     输入引用 {before_in} → {after_in}   输出引用 {before_out} → {after_out}   连线 {len(links)}")
