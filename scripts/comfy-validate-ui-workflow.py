"""把 UI 格式工作流按 ComfyUI 前端的规则转回 API 格式，提交给 ComfyUI 校验。

**这一步不能省。** 之前只比对「UI 文件 ↔ 已验证的 API 图」，比的是连线和控件值
**是否与源一致**，但源里没给的可选控件被写成了 null —— 一致，却是非法值。
交付出去的工作流一打开就是红框：「无效输入 · 输入值类型错误」。

按控件顺序把 widgets_values 填回参数名，再走一次 /prompt 校验，
才能证明「这份文件真的能跑」。
"""
import json, os, sys, urllib.request, urllib.error
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from comfy_widget_types import widget_names

UI = json.load(open(sys.argv[1]))
info = json.load(urllib.request.urlopen("http://127.0.0.1:8188/object_info", timeout=60))
L = {l[0]: l for l in UI["links"]}
SKIP = {"Note", "MarkdownNote", "Reroute", "PrimitiveNode", "GetNode", "SetNode"}

# ── 解析 KJNodes 的 Set/Get 虚拟节点 ──
#
# 它们是纯前端 JS 实现，不在 `object_info` 里，但**连线要经它们中转**。
# 直接跳过会变成「引用了不存在的节点」，报出来的是假错
# （Director 那份 31 个虚拟节点，之前就这么误报过）。
#
# 语义很简单：`SetNode` 把输入登记到一个名字下，`GetNode` 把那个名字取出来。
# 所以把每条经 GetNode 的连线，改接到对应 SetNode 的**输入来源**上。
def resolve_virtual(nodes, links):
    by_id = {n["id"]: n for n in nodes}
    L = {l[0]: l for l in links}
    setters = {}          # 名字 → (源节点, 源插槽)
    for n in nodes:
        if n["type"] != "SetNode":
            continue
        key = (n.get("widgets_values") or [None])[0]
        lin = (n.get("inputs") or [{}])[0].get("link")
        if key is not None and lin in L:
            src = L[lin]
            setters[key] = (src[1], src[2])
    # GetNode 的出边改接到 setter 的源；一个 GetNode 可能又接到另一个 SetNode，
    # 所以要迭代到不动点（实测 Director 那份有两级中转）
    for _ in range(8):
        changed = False
        for n in nodes:
            if n["type"] != "GetNode":
                continue
            key = (n.get("widgets_values") or [None])[0]
            if key not in setters:
                continue
            src_id, src_slot = setters[key]
            if by_id.get(src_id, {}).get("type") in ("GetNode", "SetNode"):
                continue          # 源还是虚拟节点，下一轮再解
            for l in links:
                if l[1] == n["id"]:
                    l[1], l[2] = src_id, src_slot
                    changed = True
        if not changed:
            break
    unresolved = [n["id"] for n in nodes if n["type"] == "GetNode"
                  and (n.get("widgets_values") or [None])[0] not in setters]
    return unresolved


virtual = [n for n in UI["nodes"] if n["type"] in {"GetNode", "SetNode"}]
if virtual:
    unresolved = resolve_virtual(UI["nodes"], UI["links"])
    L = {l[0]: l for l in UI["links"]}
    print(f"  · 解析了 {len(virtual)} 个前端虚拟节点（Set/GetNode）"
          + (f"，{len(unresolved)} 个 GetNode 找不到对应的 SetNode: {unresolved[:6]}" if unresolved else ""))

api = {}
for n in UI["nodes"]:
    ct = n["type"]
    if ct in SKIP or ct not in info:
        continue
    # **控件被「转成输入」时，值会从 widgets_values 里移走。**
    #
    # ComfyUI 允许把任一控件转成输入接口。转了之后节点的 `inputs` 里会多一项同名的，
    # 而 `widgets_values` 里那一格**消失**。不排掉的话后面所有值错位一格 ——
    # 实际踩过：Director 的 `global_prompt` 被转成输入，于是
    # `custom_height` 拿到 "stretch to fit"、`resize_method` 拿到 32。
    # 判据是**有没有 `widget` 标记**，不是"在不在 inputs 里"：
    #   inputs 里带 {"widget": {"name": ...}} 的 → 仍占 widgets_values 一格
    #   inputs 里不带 widget 标记的      → 已被转成接口，那一格**消失**
    # 只按"在不在 inputs 里"排的话，UNETLoader 的 unet_name 也会被排掉（它带标记）。
    converted = {i["name"] for i in (n.get("inputs") or []) if not i.get("widget")}
    wnames = [k for k in widget_names(info[ct]) if k not in converted]
    ins = {}
    for k, v in zip(wnames, n.get("widgets_values") or []):
        ins[k] = v
    for i in n.get("inputs") or []:
        if i.get("link") in L:
            l = L[i["link"]]
            ins[i["name"]] = [str(l[1]), l[2]]
    api[str(n["id"])] = {"class_type": ct, "inputs": ins}

try:
    urllib.request.urlopen("http://127.0.0.1:8188/prompt",
                           json.dumps({"prompt": api, "client_id": "validate_only"}).encode(),
                           timeout=60)
    print(f"  ✓ {sys.argv[1]} 通过 ComfyUI 校验（已提交，可在队列里取消）")
except urllib.error.HTTPError as e:
    d = json.loads(e.read())
    print(f"  ✗ {sys.argv[1]} 校验失败:")
    for k, v in (d.get("node_errors") or {}).items():
        for er in v.get("errors", []):
            print(f"      节点 {k} ({v.get('class_type')}): {er.get('type')} — {er.get('details')}")
    if not d.get("node_errors"):
        print("     ", json.dumps(d.get("error"), ensure_ascii=False)[:300])
    sys.exit(1)
