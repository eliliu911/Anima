"""把 API 格式的图转成 ComfyUI 编辑器能打开的 UI 格式。

API 格式（`/prompt` 收的那种）只有 class_type + inputs；
UI 格式要 nodes[] + links[] + 每个节点的 inputs/outputs 插槽和坐标。
这里按 `/object_info` 的真实签名生成插槽，顺序必须和 ComfyUI 的一致 ——
错了会连错线，而且**不报错**，只是出来的图不对。
"""
import json, os, sys, urllib.request
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

API = json.load(open(sys.argv[1]))
OUT = sys.argv[2]
TITLE = sys.argv[3] if len(sys.argv) > 3 else ""

info = json.load(urllib.request.urlopen("http://127.0.0.1:8188/object_info", timeout=60))

from comfy_widget_types import widget_names, slot_pairs, widget_default, specs_of


def sig(ct):
    ni = info[ct]
    return (slot_pairs(ni), widget_names(ni),
            [o for o in ni.get("output", [])], ni.get("output_name") or [], specs_of(ni))

nodes, links, lid = [], [], 0
ids = sorted(API, key=lambda x: int(x) if x.isdigit() else 0)
pos = {}
for i, nid in enumerate(ids):
    pos[nid] = [80 + (i % 6) * 300, 80 + (i // 6) * 240]

# 先建节点
byid = {}
for nid in ids:
    n = API[nid]
    ct = n["class_type"]
    slots, widgets, outs, out_names, specs = sig(ct)
    node = {
        "id": int(nid), "type": ct, "pos": pos[nid], "size": [270, 100],
        "flags": {}, "order": ids.index(nid), "mode": 0,
        "inputs": [{"name": s, "type": t, "link": None} for s, t in slots],
        "outputs": [{"name": (out_names[k] if k < len(out_names) else o), "type": o, "links": []}
                    for k, o in enumerate(outs)],
        "properties": {"Node name for S&R": ct},
        # 没显式给的控件用 schema 的默认值，不能留 None（见 widget_default）
        "widgets_values": [
            n["inputs"][w] if w in n["inputs"] else widget_default(specs.get(w))
            for w in widgets
        ],
    }
    nodes.append(node); byid[nid] = node

# 再连线
for nid in ids:
    for name, v in API[nid]["inputs"].items():
        if not (isinstance(v, list) and len(v) == 2 and isinstance(v[0], str) and v[0] in API):
            continue
        src, slot = v
        tgt = byid[nid]
        ti = next((k for k, s in enumerate(tgt["inputs"]) if s["name"] == name), None)
        if ti is None:      # 该输入在签名里是控件位，补一个连线插槽
            tgt["inputs"].append({"name": name, "type": "*", "link": None}); ti = len(tgt["inputs"]) - 1
        lid += 1
        tgt["inputs"][ti]["link"] = lid
        byid[src]["outputs"][slot]["links"].append(lid)
        links.append([lid, int(src), slot, int(nid), ti, tgt["inputs"][ti]["type"]])

json.dump({"last_node_id": max(int(i) for i in ids), "last_link_id": lid,
           "nodes": nodes, "links": links, "groups": [], "config": {},
           "extra": {"title": TITLE}, "version": 0.4},
          open(OUT, "w"), ensure_ascii=False, indent=1)
print(f"  {OUT}: {len(nodes)} 节点 / {len(links)} 连线")
