"""把 LTX-2.5-Director 的官方示例工作流改造成本机（Apple Silicon / MPS）能跑的版本。

改四类东西：
1. **摘掉 CUDA 专用节点** —— SageAttention 两个 patch、Triton 相关的 tuner。
   它们都是 model 直通节点，摘掉后要把上游直接接到下游，否则图断链。
2. **int8 权重换 bf16** —— MPS 缺 `aten::_int_mm`，int8 一加载就报错。
3. **换掉没装的节点包** —— VAELoaderKJ→VAELoader、VHS_VideoCombine→CreateVideo+SaveVideo、
   Power Lora Loader(rgthree)→直接摘掉（我们的 transformer 本身就是 distilled 版）。
4. **去掉 Windows 路径前缀** —— `vae_ltx\\x.safetensors` 在 macOS 上会被当成文件名的一部分。
"""
import json, sys, copy

SRC = '/Users/you/AI/ComfyUI/custom_nodes/LTX-2.5-Director/example_workflows/LTX_2_5_Director_CS.json'
DST = sys.argv[1]

w = json.load(open(SRC))
nodes = {n['id']: n for n in w['nodes']}
links = {l[0]: l for l in w['links']}     # [id, from_node, from_slot, to_node, to_slot, type]

def origin_of(link_id):
    l = links.get(link_id)
    return (l[1], l[2]) if l else None

def bypass(node_id, in_slot=0):
    """摘掉一个直通节点：把它输入的来源，直接接到它输出的每个去处。

    **出边必须扫 `links` 表，不能信节点自己的 `outputs[].links`。**
    连着摘一串直通节点时，前一次摘除会改掉 links 记录的来源，
    却没法同步更新每个源节点的数组 —— 于是轮到后面某一个时，
    它的数组里全是已删除的 link id，一条都匹配不上，**整条下游链断源**。

    实际后果：Director 工作流里 `LTX Director CS (2.5)` 报
    「缺少必需的输入: model」，而界面上看不出线是在哪一步丢的。
    """
    n = nodes[node_id]
    lin = (n.get('inputs') or [])[in_slot].get('link')
    src = origin_of(lin)
    if src is None:
        return False
    for lid, l in list(links.items()):
        if l[1] == node_id and lid != lin:
            l[1], l[2] = src[0], src[1]
    if lin in links:
        del links[lin]
    del nodes[node_id]
    return True

# ── 1. 摘掉 CUDA 专用 / 可选的直通节点 ──
DROP_PASSTHROUGH = ['PathchSageAttentionKJ', 'LTX2MemoryEfficientSageAttentionPatch',
                    'LTXVChunkFeedForward', 'LTX2AttentionTunerPatch',
                    'Power Lora Loader (rgthree)', 'LTX2SamplingPreviewOverride',
                    # 示例用的是 dev 版 transformer + 蒸馏 LoRA；
                    # 本机那份 transformer 本身就是 distilled，再挂一次是重复的，
                    # 而且这个 LoRA 文件盘上没有 —— 留着会直接报 Value not in list。
                    'LoraLoaderModelOnly']
removed = []
for t in DROP_PASSTHROUGH:
    for nid in [i for i, n in nodes.items() if n['type'] == t]:
        if bypass(nid):
            removed.append(f"{t}({nid})")

# ── 2. 预览用的 tiny VAE 没有对应文件，且它唯一的下游已摘除 ──
for nid in [i for i, n in nodes.items() if n['type'] == 'VAELoaderKJ'
            and 'taeltx2_3' in str(n.get('widgets_values'))]:
    for out in (nodes[nid].get('outputs') or []):
        for lid in (out.get('links') or []):
            links.pop(lid, None)
    del nodes[nid]
    removed.append(f"VAELoaderKJ/taeltx2_3({nid})")

# ── 3. VAELoaderKJ → 内置 VAELoader（只有 vae_name 一个参数）──
for nid, n in list(nodes.items()):
    if n['type'] == 'VAELoaderKJ':
        name = n['widgets_values'][0].split('\\')[-1].split('/')[-1]
        n['type'] = 'VAELoader'
        n['widgets_values'] = [name]
        n['inputs'] = []

# ── 4. int8 → bf16，并去掉 Windows 目录前缀 ──
SWAP = {
    'ltx-2.5-22b-dev-transformer-comfy-int8-convrot.safetensors':
        'ltx-2.5-22b-distilled-transformer-bf16.safetensors',
    'gemma4-12b-with-proj-ltx-2.5-comfy-int8-convrot.safetensors':
        'gemma4-12b-with-proj-ltx-2.5-bf16.safetensors',
}
for n in nodes.values():
    wv = n.get('widgets_values')
    if isinstance(wv, list):
        for i, v in enumerate(wv):
            if isinstance(v, str) and ('.safetensors' in v):
                base = v.split('\\')[-1].split('/')[-1]
                wv[i] = SWAP.get(base, base)

# ── 5. VHS_VideoCombine → CreateVideo + SaveVideo（都是内置）──
for nid, n in list(nodes.items()):
    if n['type'] != 'VHS_VideoCombine':
        continue
    ins = {i['name']: i.get('link') for i in (n.get('inputs') or [])}
    img_src, aud_src = origin_of(ins.get('images')), origin_of(ins.get('audio'))
    new_id = max(nodes) + 1
    n['type'] = 'CreateVideo'
    n['widgets_values'] = [24.0]
    n['inputs'] = [{'name': 'images', 'type': 'IMAGE', 'link': ins.get('images')},
                   {'name': 'audio', 'type': 'AUDIO', 'shape': 7, 'link': ins.get('audio')}]
    n['outputs'] = [{'name': 'VIDEO', 'type': 'VIDEO', 'links': [90001]}]
    nodes[new_id] = {
        'id': new_id, 'type': 'SaveVideo', 'pos': [n['pos'][0] + 420, n['pos'][1]],
        'size': [320, 120], 'flags': {}, 'order': 999, 'mode': 0,
        'inputs': [{'name': 'video', 'type': 'VIDEO', 'link': 90001}],
        'outputs': [], 'properties': {}, 'widgets_values': ['LTX25_director', 'auto', 'auto'],
    }
    links[90001] = [90001, nid, 0, new_id, 0, 'VIDEO']
    # 保留原来接进来的两条链接（images/audio），别的（frame_rate 等）丢掉
    for name, lid in ins.items():
        if name not in ('images', 'audio') and lid in links:
            del links[lid]
    removed.append(f"VHS_VideoCombine({nid})→CreateVideo+SaveVideo({new_id})")

# ── 收尾：清掉指向已删节点的悬空链接，并同步各节点的 link 记录 ──
alive = set(nodes)
links = {k: l for k, l in links.items() if l[1] in alive and l[3] in alive}
for n in nodes.values():
    for i in (n.get('inputs') or []):
        if i.get('link') not in links:
            i['link'] = None
    for o in (n.get('outputs') or []):
        if o.get('links'):
            o['links'] = [x for x in o['links'] if x in links]

w['nodes'] = list(nodes.values())
w['links'] = list(links.values())
w['last_node_id'] = max(nodes)
w['last_link_id'] = max(links) if links else 0
json.dump(w, open(DST, 'w'), ensure_ascii=False, indent=1)

print("  摘除:", ", ".join(removed))
print("  节点数:", len(nodes), " 链接数:", len(links))
