"""判断一个节点输入是「控件」还是「连线插槽」—— 转换器和校验器共用这一份。

**各写一份必然会漂**：转换器把某个输入当控件、校验器当插槽（或反过来），
生成的工作流就会缺参数，而校验器还会说它没问题。

已经踩过的三种形状：

| `object_info` 里的样子 | 含义 | 踩的坑 |
|---|---|---|
| `["COMBO", {"options": [...]}]` | 枚举 | 新版格式；只认老版 `[[...]]` 会把 "COMBO" 当连线类型，采样器名丢空 |
| `[["a","b"], {...}]` | 枚举（老版） | — |
| `["FLOAT,INT", {...}]` | 复合数值 | `LTXVEmptyLatentAudio.frame_rate` 就是这个，不认就丢 frame_rate |
| `["COMFY_DYNAMICCOMBO_V3", {options}]` | 动态组合 | `SaveVideo.codec`，不认会被当成"缺必需输入" |
"""

PRIMITIVES = {"INT", "FLOAT", "STRING", "BOOLEAN"}


def is_widget(spec) -> bool:
    t = spec[0] if isinstance(spec, list) else spec
    # COMFY_DYNAMICCOMBO_V3：选项本身还能带子输入（`SaveVideo.codec`、
    # `LTXVImgToVideoInplaceKJ.num_images` 都是）。选 "auto" 这类无子输入的
    # 选项时当普通控件即可；要用带子输入的选项，得手工在界面里连。
    if isinstance(t, list) or t in ("COMBO", "COMFY_DYNAMICCOMBO_V3"):
        return True
    if isinstance(t, str):
        # "FLOAT,INT" 这种复合数值也是控件 —— 只要每一段都是基础类型
        parts = [p.strip() for p in t.split(",")]
        return bool(parts) and all(p in PRIMITIVES for p in parts)
    return False


def widget_names(node_info: dict) -> list:
    d = node_info["input"]
    pairs = list((d.get("required") or {}).items()) + list((d.get("optional") or {}).items())
    return [name for name, spec in pairs if is_widget(spec)]


def slot_pairs(node_info: dict) -> list:
    """(名字, 类型) 的连线插槽列表，顺序即插槽顺序。"""
    d = node_info["input"]
    pairs = list((d.get("required") or {}).items()) + list((d.get("optional") or {}).items())
    out = []
    for name, spec in pairs:
        if is_widget(spec):
            continue
        t = spec[0] if isinstance(spec, list) else spec
        out.append((name, t if isinstance(t, str) else "*"))
    return out


def widget_default(spec):
    """控件的默认值。**不能用 None 顶替** —— 界面会渲染成 0 或空，
    然后报「无效输入 · 输入值类型错误」，工作流根本跑不了。"""
    if isinstance(spec, list) and len(spec) > 1 and isinstance(spec[1], dict):
        if "default" in spec[1]:
            return spec[1]["default"]
        opts = spec[1].get("options")
        if isinstance(opts, list) and opts:
            return opts[0]
    if isinstance(spec, list) and spec and isinstance(spec[0], list) and spec[0]:
        return spec[0][0]
    return None


def specs_of(node_info: dict) -> dict:
    d = node_info["input"]
    return dict(list((d.get("required") or {}).items()) + list((d.get("optional") or {}).items()))
