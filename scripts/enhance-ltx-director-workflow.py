"""给适配后的 Director 工作流加上：分块 VAE 解码 / RealESRGAN 最终超分 / NAG / RIFE 补帧。

每插一个节点都从 `links` 表重建插槽引用（见 comfy-repair-ui-links.py），
**不信任节点自己的 outputs[].links** —— 那正是上一版把 Director 的 model
接线搞丢的原因。
"""
import json, sys

P = sys.argv[1]
w = json.load(open(P))


def rebuild():
    N = {n["id"]: n for n in w["nodes"]}
    alive = set(N)
    w["links"] = [l for l in w["links"] if l[1] in alive and l[3] in alive]
    for n in w["nodes"]:
        for i in n.get("inputs") or []:
            i["link"] = None
        for o in n.get("outputs") or []:
            o["links"] = []
    for l in w["links"]:
        lid, src, ss, tgt, ts = l[:5]
        if ts < len(N[tgt].get("inputs") or []):
            N[tgt]["inputs"][ts]["link"] = lid
        if ss < len(N[src].get("outputs") or []):
            N[src]["outputs"][ss]["links"].append(lid)
    return N


def new_ids():
    return max(n["id"] for n in w["nodes"]) + 1, max((l[0] for l in w["links"]), default=0) + 1


def insert_after(src_id, src_slot, node, in_slot=0, out_slot=0):
    """把 node 插到 (src_id, src_slot) 的下游：原来吃这个输出的全改吃 node。"""
    _, lid = new_ids()
    moved = [l for l in w["links"] if l[1] == src_id and l[2] == src_slot]
    for l in moved:
        l[1], l[2] = node["id"], out_slot
    w["links"].append([lid, src_id, src_slot, node["id"], in_slot,
                       node["inputs"][in_slot]["type"]])
    w["nodes"].append(node)
    rebuild()
    return moved


rebuild()

# ── 1. VAEDecode → LTXVTiledVAEDecode（原版用的就是这个，内存吃紧时靠它）──
conv = 0
for n in w["nodes"]:
    if n["type"] == "VAEDecode":
        n["type"] = "LTXVTiledVAEDecode"
        n["properties"] = {"Node name for S&R": "LTXVTiledVAEDecode"}
        n["inputs"] = [{"name": "vae", "type": "VAE", "link": None},
                       {"name": "latents", "type": "LATENT", "link": None}]
        n["widgets_values"] = [4, 4, 8, False, "auto", "auto"]
        conv += 1

# 换了插槽名和顺序，得按原来的连线重接：VAEDecode 是 (samples, vae)，Tiled 是 (vae, latents)
for l in w["links"]:
    tgt = next((n for n in w["nodes"] if n["id"] == l[3]), None)
    if tgt and tgt["type"] == "LTXVTiledVAEDecode":
        l[4] = 1 if l[5] == "LATENT" else 0
rebuild()

nid, _ = new_ids()

# ── 2. NAG：蒸馏模型 cfg=1 时 CFG 是关的，负向词本来完全不起作用 ──
#
# **必须插在 Director 之后，不能插在 UNETLoader 之后。**
# Director 吃 model、吐 model/positive/negative；插在它前面的话，
# NAG 的负向条件只能取自下游 → `Director → NAG → Conditioning → Director` 成环，
# ComfyUI 直接报 `dependency_cycle` 拒绝执行。实际踩过。
director = next((n for n in w["nodes"] if n["type"] == "LTXDirectorCS25"), None)
anchor_node = director if director else next(n for n in w["nodes"] if n["type"] == "UNETLoader")
unet = anchor_node
cond = director if director else next(n for n in w["nodes"] if n["type"] == "LTXVConditioning")
nag = {"id": nid, "type": "LTX2_NAG", "pos": [unet["pos"][0] + 300, unet["pos"][1] + 160],
       "size": [300, 140], "flags": {}, "order": 800, "mode": 0,
       "inputs": [{"name": "model", "type": "MODEL", "link": None},
                  {"name": "nag_cond_video", "type": "CONDITIONING", "shape": 7, "link": None}],
       "outputs": [{"name": "MODEL", "type": "MODEL", "links": []}],
       "properties": {"Node name for S&R": "LTX2_NAG"},
       "widgets_values": [11.0, 0.25, 2.5, True]}
insert_after(unet["id"], 0, nag)
_, lid = new_ids()
neg_slot = 2 if cond["type"] == "LTXDirectorCS25" else 1
w["links"].append([lid, cond["id"], neg_slot, nag["id"], 1, "CONDITIONING"])
rebuild()

nid, _ = new_ids()

# ── 3. RealESRGAN 最终超分 ──
cv = next(n for n in w["nodes"] if n["type"] == "CreateVideo")
img_l = next((l for l in w["links"] if l[3] == cv["id"] and l[5] == "IMAGE"), None)
if img_l:
    loader = {"id": nid, "type": "UpscaleModelLoader", "pos": [cv["pos"][0] - 640, cv["pos"][1] + 170],
              "size": [300, 60], "flags": {}, "order": 900, "mode": 0, "inputs": [],
              "outputs": [{"name": "UPSCALE_MODEL", "type": "UPSCALE_MODEL", "links": []}],
              "properties": {"Node name for S&R": "UpscaleModelLoader"},
              "widgets_values": ["RealESRGAN_x2plus.pth"]}
    up = {"id": nid + 1, "type": "ImageUpscaleWithModel", "pos": [cv["pos"][0] - 320, cv["pos"][1]],
          "size": [280, 60], "flags": {}, "order": 901, "mode": 0,
          "inputs": [{"name": "upscale_model", "type": "UPSCALE_MODEL", "link": None},
                     {"name": "image", "type": "IMAGE", "link": None}],
          "outputs": [{"name": "IMAGE", "type": "IMAGE", "links": []}],
          "properties": {"Node name for S&R": "ImageUpscaleWithModel"}, "widgets_values": []}
    w["nodes"].append(loader)
    insert_after(img_l[1], img_l[2], up, in_slot=1)
    _, lid = new_ids()
    w["links"].append([lid, loader["id"], 0, up["id"], 0, "UPSCALE_MODEL"])
    rebuild()
    nid, _ = new_ids()

    # ── 4. RIFE 2x 补帧，帧率随之翻倍 ──
    rife = {"id": nid, "type": "RIFE VFI", "pos": [cv["pos"][0] - 320, cv["pos"][1] + 200],
            "size": [320, 220], "flags": {}, "order": 902, "mode": 0,
            "inputs": [{"name": "frames", "type": "IMAGE", "link": None}],
            "outputs": [{"name": "IMAGE", "type": "IMAGE", "links": []}],
            "properties": {"Node name for S&R": "RIFE VFI"},
            "widgets_values": ["rife47.pth", 10, 2, True, True, 1.0, "float32", False, 1]}
    insert_after(up["id"], 0, rife)
    if cv.get("widgets_values"):
        cv["widgets_values"][0] = 48.0     # RIFE 2x 后不翻倍的话动作会变慢一半

rebuild()
w["last_node_id"] = max(n["id"] for n in w["nodes"])
w["last_link_id"] = max(l[0] for l in w["links"])
json.dump(w, open(P, "w"), ensure_ascii=False, indent=1)
print(f"  VAEDecode→LTXVTiledVAEDecode {conv} 处；NAG / RealESRGAN / RIFE 已加")
print(f"  节点 {len(w['nodes'])}  连线 {len(w['links'])}")
