//! LoRA 通路：把 LoRA 适配器接进 ComfyUI 的生成图。
//!
//! 文生图和文生/图生视频共用这一份 —— 两边插的节点不同
//! （`LoraLoader` 带 CLIP，`LoraLoaderModelOnly` 不带），但**解析名字**和
//! **串成链**这两件事是一样的，各写一份必然会漂。
//!
//! # 为什么名字要跟 ComfyUI 现场核对
//!
//! `video_tool.rs` 出过一次事故：代码把 `"wan2.1_i2v"` 当成文件名发了出去，
//! 而盘上叫 `wan2.1_i2v_480p_14B_fp8_scaled.safetensors`。ComfyUI 回一句
//! `Value not in list`，用户完全看不出是缺文件还是名字写错。
//!
//! 所以这里的规矩是：**只信 `object_info` 的枚举，不猜文件名**；对不上的时候
//! 把这台机器上**实际有哪些**列回去，让模型自己改对再来一次，
//! 而不是把上游那句无信息量的报错透传出去。
//!
//! # 和 `cinema_sheet.rs` 的关系（有重叠，是有意留着的）
//!
//! `cinema_sheet.rs` 里已经有一套 LoRA 接线（`find_lora_sized` + 手写的两级
//! 串联），但那套和这套解决的不是同一个问题：
//!
//! - 它**按关键词自动挑**（`find_asian_lora` / `find_layout_lora`），是角色
//!   设定卡那条流水线的内部决策；这里是**模型点名要哪个**。
//! - 它**扫文件系统**（`loras_dir()`）；这里**查 ComfyUI 的枚举**。
//!   枚举才是权威 —— 校验就是拿它比对，而且它认 `extra_model_paths.yaml`
//!   指出去的目录，扫死目录会漂。
//! - 它写死两条（`shot.lora` / `shot.lora2`，节点 id 固定 `"2"` / `"16"`，
//!   图里别处还引用着）；这里是任意条。
//!
//! 没有把它改过来，是因为那条路有测试、在跑，而这次的范围是"把通路加出来"。
//! 但**两处都在回答「本机有哪些 LoRA」这一个问题，来源不同就会漂** ——
//! 将来动 cinema 那边时，优先把它换成 [`available_loras`]。

use serde_json::{json, Value};

const DEFAULT_COMFY_URL: &str = "http://127.0.0.1:8188";

/// 模型请求的一条 LoRA。名字允许是文件名的一部分，由 [`resolve_loras`] 对齐。
#[derive(Debug, Clone, PartialEq)]
pub struct RequestedLora {
    pub name: String,
    pub strength: f64,
}

/// 已经和 ComfyUI 的枚举对齐过的一条 LoRA。
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedLora {
    /// ComfyUI 枚举里的完整文件名
    pub name: String,
    pub strength: f64,
}

/// 从工具参数里读 `loras`。
///
/// 缺省、类型不对、条目不是对象，一律当成"没要 LoRA"跳过 —— 本地小模型
/// 经常把数组写成字符串，为这个直接让整次生成失败不划算。
pub fn parse_requested(args: &Value) -> Vec<RequestedLora> {
    let Some(list) = args.get("loras").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|item| {
            let name = item.get("name").and_then(|v| v.as_str())?.trim();
            if name.is_empty() {
                return None;
            }
            Some(RequestedLora {
                name: name.to_string(),
                // 0.8 是社区惯例的起点；给 0 等于没挂，这里不拦，那是用户的选择
                strength: item.get("strength").and_then(|v| v.as_f64()).unwrap_or(0.8),
            })
        })
        .collect()
}

/// 在候选里找一个名字。先精确、再忽略大小写精确、最后子串。
///
/// 子串这一档是给模型留的余地：它记得住 `asianGirlsFace`，记不住
/// `asianGirlsFace_v1.safetensors`。但**只在唯一命中时**才算数 ——
/// 两个都像的时候挑一个，就是在替用户瞎猜。
pub fn match_name(requested: &str, available: &[String]) -> Option<String> {
    if let Some(hit) = available.iter().find(|n| n.as_str() == requested) {
        return Some(hit.clone());
    }
    let lower = requested.to_lowercase();
    if let Some(hit) = available.iter().find(|n| n.to_lowercase() == lower) {
        return Some(hit.clone());
    }
    let mut subs = available.iter().filter(|n| n.to_lowercase().contains(&lower));
    let first = subs.next()?;
    if subs.next().is_some() {
        return None; // 多个都像 —— 交回去让模型说清楚，不替它挑
    }
    Some(first.clone())
}

/// 这批请求里，**真的挂得上**的是哪些。
///
/// 用来决定引擎：**一个根本不会被挂上的 LoRA，不该有权把引擎改掉。**
///
/// 踩过的坑：用户说「触发词 j1v_w0m」，模型把它当 LoRA 名字传了进来。
/// 引擎判定按「认不出的算 SDXL」强制回落，而后面真去解析时盘上没有这个文件，
/// 又被丢掉 —— 结果是**引擎掉到 SDXL、LoRA 一个没挂**，两头都输，
/// 而且界面上只显示一个 SDXL 徽章，看不出发生了什么。
pub fn attachable_names(requested: &[String], available: &[String]) -> Vec<String> {
    requested
        .iter()
        .filter_map(|n| match_name(n, available))
        .collect()
}

/// 拉取 ComfyUI 认得的 LoRA 文件名。
pub async fn available_loras(client: &reqwest::Client) -> Vec<String> {
    let url = format!("{DEFAULT_COMFY_URL}/object_info/LoraLoader");
    let Ok(resp) = client.get(&url).send().await else {
        return Vec::new();
    };
    let Ok(json) = resp.json::<Value>().await else {
        return Vec::new();
    };
    json.pointer("/LoraLoader/input/required/lora_name/0")
        .and_then(|v| v.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// 把请求的名字对齐到 ComfyUI 的枚举。
///
/// 返回 `(对齐好的, 可选的警告)`。
///
/// **对不上的名字不再让整次生成失败 —— 跳过它，照常出图，然后把问题大声说出来。**
///
/// 第一版是硬失败。真实后果：本地 27B 模型经常猜 LoRA 名字，一猜错整次生成
/// 就废了，连 ComfyUI 都没提交；而前端的失败态只认 ComfyUI 的进度事件，
/// 于是卡片**永远转骨架屏**。用户看到的是「第二次生成一直卡在生成阶段」。
///
/// 为一个装饰性参数付掉整张图，这个代价定错了。关键的词是**静默** ——
/// 不静默地跳过既拿得到图，又能让模型下一轮用对名字。
pub fn resolve_loras(
    requested: &[RequestedLora],
    available: &[String],
) -> (Vec<ResolvedLora>, Option<String>) {
    if requested.is_empty() {
        return (Vec::new(), None);
    }
    if available.is_empty() {
        return (
            Vec::new(),
            Some(
            "这台机器的 ComfyUI 里没有任何 LoRA（models/loras/ 是空的，或 ComfyUI 没在跑）。\
             不挂 LoRA 重试，或者先把 .safetensors 放进 models/loras/。"
                    .to_string(),
            ),
        );
    }

    let mut resolved = Vec::with_capacity(requested.len());
    let mut missing = Vec::new();
    for r in requested {
        match match_name(&r.name, available) {
            Some(name) => resolved.push(ResolvedLora {
                name,
                strength: r.strength,
            }),
            None => missing.push(r.name.clone()),
        }
    }
    let warning = (!missing.is_empty()).then(|| {
        format!(
            "这些 LoRA 对不上（写错了，或者不唯一），**已跳过，图仍然生成了**：{}。\n             本机可用的是：\n{}\n             想用它们的话，用上面列表里的**准确名字**重新调用一次。",
            missing.join("、"),
            available
                .iter()
                .map(|n| format!("  - {n}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    });
    (resolved, warning)
}

/// LoRA 链接进图之后，下游该从哪里取 model / clip。
#[derive(Debug, Clone, PartialEq)]
pub struct LoraChain {
    /// 要插进 `prompt` 对象的节点，键是节点 id
    pub nodes: Vec<(String, Value)>,
    /// 下游 `model` 输入应该连的地方
    pub model_ref: Value,
    /// 下游 `clip` 输入应该连的地方；`LoraLoaderModelOnly` 那条路是 `None`
    pub clip_ref: Option<Value>,
}

/// 串一条 LoRA 链。
///
/// 节点 id 从 `first_id` 开始往上加 —— 调用方要保证这段不和图里已有的 id 撞。
/// 两个工具都用 900 起，图本身只用到个位数。
///
/// `clip_src` 给 `Some` 时用 `LoraLoader`（同时改 model 和 clip，文生图那条）；
/// 给 `None` 时用 `LoraLoaderModelOnly`（UNETLoader 不产 CLIP，视频那条 ——
/// MiniMax H3 的官方图里那个必需的 turbo LoRA 就是这么挂的）。
pub fn build_chain(
    loras: &[ResolvedLora],
    first_id: u32,
    model_src: Value,
    clip_src: Option<Value>,
) -> LoraChain {
    let mut nodes = Vec::new();
    let mut model_ref = model_src;
    let mut clip_ref = clip_src;

    for (i, lora) in loras.iter().enumerate() {
        let id = (first_id + i as u32).to_string();
        let node = match &clip_ref {
            Some(clip) => json!({
                "class_type": "LoraLoader",
                "inputs": {
                    "lora_name": lora.name,
                    "strength_model": lora.strength,
                    "strength_clip": lora.strength,
                    "model": model_ref,
                    "clip": clip,
                }
            }),
            None => json!({
                "class_type": "LoraLoaderModelOnly",
                "inputs": {
                    "lora_name": lora.name,
                    "strength_model": lora.strength,
                    "model": model_ref,
                }
            }),
        };
        nodes.push((id.clone(), node));
        // 下一条接在这一条后面 —— 链式叠加，这是 ComfyUI 里多 LoRA 的标准接法
        model_ref = json!([id, 0]);
        if clip_ref.is_some() {
            clip_ref = Some(json!([id, 1]));
        }
    }

    LoraChain {
        nodes,
        model_ref,
        clip_ref,
    }
}

/// 工具 schema 里 `loras` 那一段。两边共用，免得描述漂。
pub fn schema_fragment() -> Value {
    json!({
        "type": "array",
        "description": "Optional LoRA adapters, applied in order. Each entry is {\"name\": \"...\", \"strength\": 0.8}. The name may be a distinctive fragment of the filename. If a name does not match, the error lists every LoRA installed on this machine — read that list and retry with an exact name rather than guessing again.",
        "items": {
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "LoRA filename, or a distinctive part of it" },
                "strength": { "type": "number", "default": 0.8, "description": "Typically 0.6-1.0" }
            },
            "required": ["name"]
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn avail() -> Vec<String> {
        [
            "Asian_Beauty XL.safetensors",
            "CharTurnXLRetag_v4.safetensors",
            "add-detail-xl.safetensors",
            "asianGirlsFace_v1.safetensors",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    #[test]
    fn 精确名字直接命中() {
        assert_eq!(
            match_name("add-detail-xl.safetensors", &avail()).as_deref(),
            Some("add-detail-xl.safetensors")
        );
    }

    /// 模型记得住 `asianGirlsFace`，记不住带版本号和扩展名的全称。
    #[test]
    fn 片段唯一时可以命中() {
        assert_eq!(
            match_name("asianGirlsFace", &avail()).as_deref(),
            Some("asianGirlsFace_v1.safetensors")
        );
        assert_eq!(
            match_name("ASIANGIRLSFACE_V1.SAFETENSORS", &avail()).as_deref(),
            Some("asianGirlsFace_v1.safetensors")
        );
    }

    /// **两个都像的时候必须放弃。**
    ///
    /// 挑一个就是替用户瞎猜 —— 而猜错的表现是"图生成成功、风格完全不对"，
    /// 不报任何错。宁可让模型说清楚再来一次。
    #[test]
    fn 片段命中多个时不猜() {
        assert_eq!(match_name("xl", &avail()), None);
        let (resolved, warn) = resolve_loras(
            &[RequestedLora { name: "xl".into(), strength: 0.8 }],
            &avail(),
        );
        // **跳过它，不是让整次生成失败。**
        assert!(resolved.is_empty());
        let warn = warn.expect("对不上时必须给出告警");
        // 告警必须把本机有哪些列出来，否则模型下一轮还是只能猜
        assert!(warn.contains("asianGirlsFace_v1.safetensors"), "{warn}");
        assert!(warn.contains("add-detail-xl.safetensors"), "{warn}");
    }

    #[test]
    fn 没装任何lora时给的是可操作的提示() {
        let (resolved, warn) = resolve_loras(
            &[RequestedLora { name: "whatever".into(), strength: 1.0 }],
            &[],
        );
        assert!(resolved.is_empty());
        assert!(warn.unwrap().contains("models/loras/"));
    }

    /// **一个名字写错，不能把其它写对的一起废掉，更不能把整张图废掉。**
    ///
    /// 硬失败那一版的真实后果：本地模型经常猜 LoRA 名字，一猜错整次生成就废，
    /// 连 ComfyUI 都没提交；而前端的失败态只认 ComfyUI 的进度事件，
    /// 于是卡片永远转骨架屏 —— 用户看到的是「第二次生成一直卡在生成阶段」。
    #[test]
    fn 一个名字写错不影响其它的和出图() {
        let (resolved, warn) = resolve_loras(
            &[
                RequestedLora { name: "add-detail-xl".into(), strength: 0.6 },
                RequestedLora { name: "根本不存在的".into(), strength: 1.0 },
            ],
            &avail(),
        );
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].name, "add-detail-xl.safetensors");
        let w = warn.expect("跳过了就要说");
        assert!(w.contains("根本不存在的"), "{w}");
        assert!(w.contains("仍然生成了"), "告警要说清图还是出了：{w}");
    }

    #[test]
    fn 全都对得上时没有告警() {
        let (resolved, warn) = resolve_loras(
            &[RequestedLora { name: "add-detail-xl".into(), strength: 0.6 }],
            &avail(),
        );
        assert_eq!(resolved.len(), 1);
        assert!(warn.is_none());
    }

    #[test]
    fn 不要lora时不插任何节点() {
        let chain = build_chain(&[], 900, json!(["1", 0]), Some(json!(["1", 1])));
        assert!(chain.nodes.is_empty());
        assert_eq!(chain.model_ref, json!(["1", 0]));
        assert_eq!(chain.clip_ref, Some(json!(["1", 1])));
    }

    /// 多条 LoRA 要**串**起来：第二条接第一条的输出，不是各自接底座。
    /// 各自接底座的话只有最后一条生效，而且不报错。
    #[test]
    fn 多条lora串成链且出口指向最后一条() {
        let loras = vec![
            ResolvedLora { name: "a.safetensors".into(), strength: 0.8 },
            ResolvedLora { name: "b.safetensors".into(), strength: 0.5 },
        ];
        let chain = build_chain(&loras, 900, json!(["1", 0]), Some(json!(["1", 1])));

        assert_eq!(chain.nodes.len(), 2);
        assert_eq!(chain.nodes[0].0, "900");
        assert_eq!(chain.nodes[1].0, "901");

        // 第一条接底座
        assert_eq!(chain.nodes[0].1["inputs"]["model"], json!(["1", 0]));
        assert_eq!(chain.nodes[0].1["inputs"]["clip"], json!(["1", 1]));
        // 第二条接第一条
        assert_eq!(chain.nodes[1].1["inputs"]["model"], json!(["900", 0]));
        assert_eq!(chain.nodes[1].1["inputs"]["clip"], json!(["900", 1]));
        // 下游取最后一条
        assert_eq!(chain.model_ref, json!(["901", 0]));
        assert_eq!(chain.clip_ref, Some(json!(["901", 1])));

        assert_eq!(chain.nodes[1].1["inputs"]["strength_model"], json!(0.5));
    }

    /// 视频那条走 `LoraLoaderModelOnly`：UNETLoader 不产 CLIP，
    /// 用带 clip 的 `LoraLoader` 会因为缺输入被 ComfyUI 拒掉。
    #[test]
    fn 没有clip时用modelonly节点() {
        let loras = vec![ResolvedLora { name: "turbo.safetensors".into(), strength: 1.0 }];
        let chain = build_chain(&loras, 900, json!(["1", 0]), None);
        assert_eq!(chain.nodes[0].1["class_type"], "LoraLoaderModelOnly");
        assert!(chain.nodes[0].1["inputs"].get("clip").is_none());
        assert_eq!(chain.clip_ref, None);
    }

    #[test]
    fn 文生图那条用带clip的节点() {
        let loras = vec![ResolvedLora { name: "a.safetensors".into(), strength: 1.0 }];
        let chain = build_chain(&loras, 900, json!(["1", 0]), Some(json!(["1", 1])));
        assert_eq!(chain.nodes[0].1["class_type"], "LoraLoader");
    }

    /// 本地小模型经常把 `loras` 写成字符串或漏字段。为这个让整次生成失败不划算 ——
    /// 跳过就行，用户看到的是一张没挂 LoRA 的图，而不是一次失败。
    #[test]
    fn 参数写歪了当作没要lora() {
        assert!(parse_requested(&json!({})).is_empty());
        assert!(parse_requested(&json!({ "loras": "asianGirlsFace" })).is_empty());
        assert!(parse_requested(&json!({ "loras": [{ "strength": 0.8 }] })).is_empty());
        assert!(parse_requested(&json!({ "loras": [{ "name": "   " }] })).is_empty());
    }

    #[test]
    fn 没写强度时用默认值() {
        let got = parse_requested(&json!({ "loras": [{ "name": "a" }] }));
        assert_eq!(got, vec![RequestedLora { name: "a".into(), strength: 0.8 }]);
    }
}

#[cfg(test)]
mod attachable_tests {
    use super::*;

    fn avail() -> Vec<String> {
        vec![
            "Straight_hair_zimgt.safetensors".into(),
            "Pintrest_ZiT.safetensors".into(),
            "hanfu_tang_sdxl.safetensors".into(),
        ]
    }

    #[test]
    fn 盘上没有的名字挂不上() {
        // 真实现场：用户说「触发词 j1v_w0m」，模型把它当 LoRA 名字传了进来
        assert!(attachable_names(&["j1v_w0m".to_string()], &avail()).is_empty());
    }

    #[test]
    fn 挂得上的原样返回真实文件名() {
        let got = attachable_names(&["Straight_hair_zimgt".to_string()], &avail());
        assert_eq!(got, vec!["Straight_hair_zimgt.safetensors".to_string()]);
    }

    #[test]
    fn 一真一假只留下真的() {
        // 这一条决定引擎：只要还剩一个真的 SDXL LoRA，就该回落；
        // 全是假的就不该动引擎
        let got = attachable_names(
            &["hanfu_tang_sdxl".to_string(), "j1v_w0m".to_string()],
            &avail(),
        );
        assert_eq!(got, vec!["hanfu_tang_sdxl.safetensors".to_string()]);
    }

    #[test]
    fn 什么都没请求就是空() {
        assert!(attachable_names(&[], &avail()).is_empty());
    }
}
