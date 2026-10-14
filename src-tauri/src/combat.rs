//! **打斗编排 + 真人表演合同**（2026-10-12）。
//!
//! # 用户要的
//!
//! 「目前 Anima 和后端缺乏打斗设定……无论是当前剧本还是后面新剧本，都要用」；
//! 「我们目前的出片 AI 感太强」—— 研究真人表演的细节处理（技术角度，不涉及成人剧情）。
//! 方案和核对见 `docs/plans/2026-10-12-H3打斗编排与战斗LoRA测试方案.md`。
//!
//! # 一套因果，两种用法
//!
//! - **表演合同**（[`Performance`]，挂在每个时间段）：触发 → 视线 → 面部 / 呼吸身体 → 手 → 物理细节 → 结果 → 怎么拍得见。
//! - **攻防交换**（[`Exchange`]，打斗镜的时间段）：开场站位 → 意图 → 攻击路径 → 防守 → 结果 → 新站位 → 镜头与音效。
//!
//! 两者都是「上一个的结果就是下一个的开场」。打斗不另起一套「更快更激烈」的写法，只是在表演的因果上加了受力和交互。
//!
//! 规则写成我们自己的话（`combat-choreography.md` 没有许可文件，只借思路）。生产里的三步（镜头表 / 细化时间线 / 合成提示词）
//! 各自的模板通过占位符 `{PERFORMANCE}` / `{COMBAT}` 拿到这里的规则，设置里改模板、删掉占位符也不报错（可选占位符）。

use crate::shot_table::{Exchange, Performance, ShotSegment};

/// 打斗镜的动作类型（`StoryboardShot::combat_style`）
pub const STYLES: [(&str, &str); 5] = [
    ("unarmed", "徒手"),
    ("melee", "冷兵器"),
    ("gunfu", "枪斗"),
    ("energy", "能量武器"),
    ("xianxia", "仙侠"),
];

/// 模型写的动作类型 → 我们认得的 id；认不出的当不是打斗镜（空串），不猜
pub fn normalize_style(raw: &str) -> String {
    let t = raw.trim().to_lowercase();
    if t.is_empty() {
        return String::new();
    }
    for (id, zh) in STYLES {
        if t == id || t == zh {
            return id.into();
        }
    }
    let alias: [(&str, &[&str]); 5] = [
        ("unarmed", &["fist", "hand-to-hand", "martial", "拳", "格斗"]),
        ("melee", &["blade", "sword", "knife", "weapon", "刀", "剑", "冷兵"]),
        ("gunfu", &["gun", "firearm", "rifle", "pistol", "枪"]),
        ("energy", &["laser", "beam", "plasma", "能量"]),
        ("xianxia", &["wuxia", "cultivation", "仙", "武侠"]),
    ];
    for (id, words) in alias {
        if words.iter().any(|w| t.contains(w)) {
            return id.into();
        }
    }
    String::new()
}

pub fn style_label(id: &str) -> &'static str {
    STYLES.iter().find(|(k, _)| *k == id).map(|(_, v)| *v).unwrap_or("")
}

// ---------------------------------------------------------------------------
// 给三步模板的规则（中文模板用中文，线路 B 写手用英文）
// ---------------------------------------------------------------------------

/// 镜头表 / 细化时间线：怎么写表演合同（中文）
pub const PERFORMANCE_RULES_ZH: &str = "\
## 表演合同（每段的 performance，可选）
有人物表演的段（台词、反应、对峙、受伤、受力）才写；纯环境、纯运镜的段不写，不硬塞。
- 字段：performer（谁）、trigger（听见 / 看见 / 受了什么力）、gaze（视线从哪到哪）、face（面部信号，最多 2 个：慢眨、下眼睑收紧、眉心短收、嘴角停住、短促失笑……）、\
body（呼吸 / 身体，最多 2 个：吸气、吞咽、肩线起伏、重心后移、手指收紧后放开……）、hand（手实际在做什么）、\
material（物理细节，只在有来源时写：source 来源、motion 看得见的运动、light 哪侧光让它看得见）、aftermath（之后留下的状态）、camera_proof（什么景别 / 焦点 / 停留能拍到它）。
- **一段可见表演信号 2~4 个**（face + body + hand + material 合起来），多了就是过度表演。
- **情绪要落成看得见的行为**：不写「紧张」「愤怒」本身，写它让人做了什么。
- **有台词**：一个说之前的准备或说的时候的动作，加一个说完后的反应（说完至少留 0.3~1 秒反应再切）。
- **远景、全景不写面部**（五官看不清，写了会崩脸），只写姿态、重心、轮廓、大动作。
- **汗、水、泪、血、灰必须有来源、有运动、有看得见它的光**；没有来源就不写。肌肉只写受力的结果，不写「肌肉细腻」。
- aftermath 要进下一段的开场 / 约束。";

/// 镜头表 / 细化时间线：怎么写打斗（中文）
pub const COMBAT_RULES_ZH: &str = "\
## 打斗（有攻防动作的镜）
- 这一镜写 combat_style：unarmed 徒手 / melee 冷兵器 / gunfu 枪斗 / energy 能量武器 / xianxia 仙侠；不是打斗镜不写。
- **一段最多一个完整攻防交换**（exchange），8 秒以内的镜最多 2 个交换，不要在一镜里堆五六种招。
- exchange 字段：attacker（发起方）、opening（谁在画左 / 画右、距离、高度、架势、武器状态）、intent（读到了什么、想抢什么）、\
attack（哪只手 / 什么武器、从哪起、什么轨迹、打向哪里）、defense（格 / 挡 / 闪 / 卸 / 盾 / 舍）、\
result（命中 / 擦过 / 落空 / 对撞 / 打断，力和环境的反应）、new_state（脚下、距离、朝向、武器状态、露出的破绽）、\
camera_sound（证明它的镜头 + 同步音效）、contact_at（接触那一刻，本镜内的秒）。
- **上一个交换的 new_state 就是下一个的 opening**；同一交换里画面左右不翻。
- **对话和接触动作分开**：要说话的镜，攻击在台词之前结束；打斗段只做动作。
- **一段一个主运镜**，镜头服从动作，不要同时要 FPV、环绕、翻滚、慢动作。
- 特效只写在接触点（火花、冲击、碎屑），黑白冲击帧 / 闪白 / 震屏留给后期。
- 打斗段的表演合同写受力：视线先锁对方武器、受力时哪里绷紧、接触后哪里回弹。";

/// 线路 B 写手：怎么用表演合同和攻防交换（英文，正文是英文）
pub const PERFORMANCE_RULES_EN: &str = "\
Performance contract (segments that carry `performance`): write it as visible behaviour in that segment, in this order —
what triggers it, where the gaze goes, at most two face signals and two breath/body signals, what the hands actually do,
any material detail only with its source, motion and the light that shows it, then what state remains afterwards.
Never write the emotion word alone, never write face detail in a wide shot, never add skin/sweat/realism keywords.
When a line is spoken, keep one action before or during it and one reaction after it, and hold the reaction before any cut.";

pub const COMBAT_RULES_EN: &str = "\
Combat (segments that carry `exchange`): write each exchange as one readable cause-and-effect beat —
opening positions (who is screen-left / screen-right, distance, guard, weapon state), the attack path (which limb or weapon, from where, along what line, at what target),
the defence, the result with its force and environmental reaction, then the new positions that open the next exchange.
Keep screen direction fixed within an exchange. One main camera move per segment, chosen to prove the action. Effects only at the contact point.
Finish attacks before any dialogue in the same shot. Do not invent extra moves beyond the exchanges given.";

// ---------------------------------------------------------------------------
// 合成时写进简报的那几行
// ---------------------------------------------------------------------------

/// 一段的表演合同 → 简报里的一行（写手翻成英文正文）。空合同回 None
pub fn performance_brief(p: &Performance) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut push = |k: &str, v: &str| {
        if !v.trim().is_empty() {
            parts.push(format!("{k}: {}", v.trim()));
        }
    };
    push("performer", &p.performer);
    push("trigger", &p.trigger);
    push("gaze", &p.gaze);
    push("face", &p.face.join(", "));
    push("breath/body", &p.body.join(", "));
    push("hands", &p.hand);
    if let Some(m) = &p.material {
        let t = [m.source.trim(), m.motion.trim(), m.light.trim()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" / ");
        push("material (source / motion / light)", &t);
    }
    push("afterwards", &p.aftermath);
    push("camera proof", &p.camera_proof);
    (!parts.is_empty()).then(|| parts.join(" | "))
}

/// 一段的攻防交换 → 简报里的一行
pub fn exchange_brief(e: &Exchange) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut push = |k: &str, v: &str| {
        if !v.trim().is_empty() {
            parts.push(format!("{k}: {}", v.trim()));
        }
    };
    push("attacker", &e.attacker);
    push("opening", &e.opening);
    push("intent", &e.intent);
    push("attack path", &e.attack);
    push("defence", &e.defense);
    push("result", &e.result);
    push("new positions", &e.new_state);
    push("camera & sound", &e.camera_sound);
    if let Some(t) = e.contact_at {
        parts.push(format!("contact at {t:.1}s"));
    }
    (!parts.is_empty()).then(|| parts.join(" | "))
}

/// 一段里能看见的表演信号数（面部 + 身体 + 手 + 物理细节）
pub fn signal_count(p: &Performance) -> usize {
    p.face.iter().filter(|s| !s.trim().is_empty()).count()
        + p.body.iter().filter(|s| !s.trim().is_empty()).count()
        + usize::from(!p.hand.trim().is_empty())
        + usize::from(p.material.as_ref().is_some_and(|m| !m.source.trim().is_empty() || !m.motion.trim().is_empty()))
}

// ---------------------------------------------------------------------------
// 出片：战斗 LoRA 预设
// ---------------------------------------------------------------------------

/// 一种打斗类型挂的 LoRA + 触发词（项目出片设置 `RenderSettings::combat`）
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct CombatPreset {
    #[serde(default)]
    pub loras: Vec<PresetLora>,
    /// 加在发给 H3 的提示词最前面（例：Fight Flow 的 `Melee weapon interaction.`）。空 = 不加
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub trigger: String,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct PresetLora {
    /// ComfyUI 枚举里的完整名（含子目录，如 `h3-combat/H3_Combat_V2.safetensors`）
    pub name: String,
    pub strength: f64,
}

pub fn is_default_sampler(s: &crate::agent_runtime::weetodd::H3Sampler) -> bool {
    *s == crate::agent_runtime::weetodd::H3Sampler::default()
}

/// 这一镜出片时的额外设置：项目的采样方法；是打斗镜就挂它那一类的预设
pub fn extras_for(render: &crate::cinema::RenderSettings, combat_style: &str) -> crate::agent_runtime::weetodd::H3Extras {
    let preset = (!combat_style.is_empty()).then(|| render.combat.get(combat_style)).flatten();
    crate::agent_runtime::weetodd::H3Extras {
        sampler: render.sampling,
        loras: preset.map(|p| p.loras.iter().filter(|l| !l.name.trim().is_empty() && l.strength != 0.0).map(|l| (l.name.clone(), l.strength)).collect()).unwrap_or_default(),
        trigger: preset.map(|p| p.trigger.trim().to_string()).unwrap_or_default(),
    }
}

// ---------------------------------------------------------------------------
// FCPX 冲击帧标记
// ---------------------------------------------------------------------------

/// 一镜里的接触点（本镜内的秒, 说明）：给 FCPX 导出打 marker。
/// 交换写了 `contact_at` 就用它；没写的取这一段的 60% 处（多数接触落在段后半），说明里写明是估的
pub fn contact_points(segments: &[ShotSegment]) -> Vec<(f32, String)> {
    segments
        .iter()
        .filter_map(|s| {
            let e = s.exchange.as_ref()?;
            let (t, est) = match e.contact_at {
                Some(t) if t >= s.start - 0.01 && t <= s.end + 0.01 => (t, false),
                _ => (s.start + (s.end - s.start) * 0.6, true),
            };
            let what = if e.result.trim().is_empty() { "接触".to_string() } else { e.result.trim().chars().take(24).collect() };
            Some((t, format!("冲击点{}：{what}（后期：黑白冲击 2 帧 / 闪白 1 帧 / 震屏 / 冲击音）", if est { "（估）" } else { "" })))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shot_table::MaterialEvent;

    #[test]
    fn 动作类型认得中英文和常见说法_认不出的不猜() {
        assert_eq!(normalize_style("melee"), "melee");
        assert_eq!(normalize_style("冷兵器"), "melee");
        assert_eq!(normalize_style("Sword fight"), "melee");
        assert_eq!(normalize_style("rifle close quarters"), "gunfu");
        assert_eq!(normalize_style("仙侠斗法"), "xianxia");
        assert_eq!(normalize_style(""), "");
        assert_eq!(normalize_style("dialogue"), "", "不是打斗不猜成打斗");
    }

    #[test]
    fn 简报只写填了的字段() {
        let p = Performance { performer: "星澜".into(), gaze: "从他的脸移到他手里的零件".into(), face: vec!["下眼睑收紧".into()], body: vec!["一次克制的吸气".into()], ..Default::default() };
        let b = performance_brief(&p).unwrap();
        assert!(b.contains("performer: 星澜") && b.contains("face: 下眼睑收紧") && !b.contains("hands"), "{b}");
        assert!(performance_brief(&Performance::default()).is_none());
        let e = Exchange { attacker: "猎手".into(), attack: "右臂刀从右上斜劈向她右肩".into(), defense: "步枪横架".into(), contact_at: Some(2.4), ..Default::default() };
        let b = exchange_brief(&e).unwrap();
        assert!(b.contains("attack path: 右臂刀") && b.contains("contact at 2.4s"), "{b}");
    }

    #[test]
    fn 表演信号计数() {
        let p = Performance { face: vec!["慢眨".into(), "".into()], body: vec!["吞咽".into()], hand: "手指松开枪柄".into(), material: Some(MaterialEvent { source: "雨".into(), ..Default::default() }), ..Default::default() };
        assert_eq!(signal_count(&p), 4);
    }

    #[test]
    fn 打斗镜挂它那一类的预设_不是打斗镜不挂() {
        use crate::agent_runtime::weetodd::H3Sampler;
        let mut r = crate::cinema::RenderSettings::default();
        r.sampling = H3Sampler::ResMultistep;
        r.combat.insert("melee".into(), CombatPreset {
            loras: vec![PresetLora { name: "h3-combat/H3_Combat_V2.safetensors".into(), strength: 0.7 }, PresetLora { name: "h3-combat/H3_Fight_Flow_V1.safetensors".into(), strength: 0.0 }],
            trigger: "Melee weapon interaction.".into(),
        });
        let e = extras_for(&r, "melee");
        assert_eq!(e.loras, vec![("h3-combat/H3_Combat_V2.safetensors".to_string(), 0.7)], "强度 0 的不挂");
        assert_eq!((e.sampler, e.trigger.as_str()), (H3Sampler::ResMultistep, "Melee weapon interaction."));
        let plain = extras_for(&r, "");
        assert!(plain.loras.is_empty() && plain.trigger.is_empty() && plain.sampler == H3Sampler::ResMultistep, "不是打斗镜：只有采样方法");
        assert!(extras_for(&r, "gunfu").loras.is_empty(), "这一类没配预设");
        // 老项目的出片设置读得出来、写回去不多出字段
        let old: crate::cinema::RenderSettings = serde_json::from_str(r#"{"quality":"staged","resolution":"480p"}"#).unwrap();
        let back = serde_json::to_value(&old).unwrap();
        assert!(back.get("sampling").is_none() && back.get("combat").is_none(), "{back}");
    }

    #[test]
    fn 接触点_写了用写的_没写取段的六成处并标估() {
        let mut a = ShotSegment { start: 0.0, end: 2.0, ..Default::default() };
        a.exchange = Some(Exchange { result: "刀砍在枪身上，火花下落".into(), contact_at: Some(1.2), ..Default::default() });
        let mut b = ShotSegment { start: 2.0, end: 5.0, ..Default::default() };
        b.exchange = Some(Exchange::default());
        let c = ShotSegment { start: 5.0, end: 6.0, ..Default::default() };
        let pts = contact_points(&[a, b, c]);
        assert_eq!(pts.len(), 2);
        assert!((pts[0].0 - 1.2).abs() < 1e-6 && !pts[0].1.contains("（估）"));
        assert!((pts[1].0 - 3.8).abs() < 1e-5 && pts[1].1.contains("（估）"), "{:?}", pts[1]);
    }
}
