//! 线路 A：libtv 中文结构，**纯填空，不经过 LM**。
//!
//! 结构照 2026-09-13 试片 A（用户看片通过）：出场角色 → 背景场景 → 参考图使用规则 → 前一个分镜
//! → 分时间段描述 → 台词 → 输出约束，另补 `non_diegetic_music: N/A`（不写这一行 H3 会自己配乐）
//! 和「参考图 N 是谁」（多张图时模型分不清哪张是谁）。
//!
//! 模板在 `shared/prompt-templates/route-a-*.txt`。
//!
//! **物理隔离**：不 import `prompt_route_b`，不用任何共用的拼装函数（有测试钉着）。
//! 输入只读 [`crate::prompt_input::ShotContext`]。

use crate::assets::AssetKind;
use crate::prompt_input::{fields_hash, short_description, ShotContext, TextOnly};
use crate::shot_table::{RefUsage, RoutePrompts};

const VIDEO_TEMPLATE: &str = include_str!("../../shared/prompt-templates/route-a-video.txt");
const FRAME_TEMPLATE: &str = include_str!("../../shared/prompt-templates/route-a-frame.txt");

/// 参考图使用规则。原文照抄 libtv 脚本（试片 A 用的就是这三条），按本镜实际挂了哪类图才写哪条。
const RULE_CHARACTER: &str = "- 角色参考图：只参考此图中的角色形象；不参考角度、构图、姿势、表情；若此参考图是角色多视图，则必须确保角色不能重复出现（除非明确要求角色分身）。";
const RULE_SCENE: &str = "- 场景参考图：只参考此图中的场景位置、大小、形状、结构关系；不参考角度、构图、多角色站位；氛围、色调、光影、元素风格等根据剧情内容来酌情参考；若此参考图是场景多视图或全景多视图，则必须确保场景的空间结构合理，且符合客观逻辑，不允许出现镜像或复制等视觉效果。";
const RULE_PROP: &str = "- 道具参考图：只参考此图中的道具大小、形状、结构；不参考角度、构图、位置；若此参考图是道具多视图，则必须确保与角色/场景的关联是符合客观逻辑的，特别是书/信/手机/显示器/电脑/门窗/贴纸/壁画等所有具有正反面或多角度差异的道具或物品，需要符合正反面逻辑，尽量从多视图中提取正面或反面或角度图来适配客观逻辑。";

/// 秒数写成「4」「6.5」，不写「4.0」。
fn secs(v: f32) -> String {
    let r = (v * 10.0).round() / 10.0;
    if r.fract() == 0.0 { format!("{}", r as i64) } else { format!("{r}") }
}

fn usage_zh(u: RefUsage) -> &'static str {
    u.label()
}

fn camera_line(ctx: &ShotContext) -> String {
    let s = ctx.shot;
    let mut parts: Vec<String> = Vec::new();
    for (group, id) in [("shotSize", &s.scale), ("angle", &s.camera_angle), ("move", &s.camera_movement)] {
        let name = crate::shot_language::display(group, id);
        if !name.is_empty() {
            parts.push(name);
        }
    }
    let mut line = parts.join("，");
    let camera_note = s.camera_prompt_note();
    if !camera_note.is_empty() {
        if !line.is_empty() {
            line.push('，');
        }
        line.push_str(&camera_note);
    }
    if line.is_empty() { "固定机位".into() } else { line }
}

/// 兜底标注：没有参考图的东西要明说「按文字描述」，模型才知道这里没有图可参考、要照字面画。
fn fallback_note(t: &TextOnly) -> &'static str {
    if t.in_library { "（没有参考图，按此描述）" } else { "（资产库里没有，按剧本描述）" }
}

fn text_line(t: &TextOnly) -> String {
    if t.description.is_empty() { format!("{}{}", t.name, fallback_note(t)) } else { format!("{}：{}{}", t.name, t.description, fallback_note(t)) }
}

fn characters(ctx: &ShotContext) -> String {
    let text = ctx.text_only();
    let mut lines: Vec<String> = ctx
        .characters()
        .iter()
        .map(|a| match text.iter().find(|t| t.name == a.name) {
            Some(t) => text_line(t),
            None => {
                let d = short_description(a, 120);
                if d.is_empty() { a.name.clone() } else { format!("{}：{d}", a.name) }
            }
        })
        .collect();
    // 资产库里没有的人物也要写上，不然这个人在提示词里就消失了
    lines.extend(text.iter().filter(|t| !t.in_library && t.kind == AssetKind::Character).map(text_line));
    if lines.is_empty() { "本镜没有出场角色".into() } else { lines.join("\n") }
}

fn scene(ctx: &ShotContext) -> String {
    let text = ctx.text_only();
    let mut lines = Vec::new();
    let unlinked_scene = text.iter().find(|t| !t.in_library && t.kind == AssetKind::Scene);
    match (ctx.scene(), unlinked_scene) {
        (Some(a), _) => match text.iter().find(|t| t.name == a.name) {
            Some(t) => lines.push(text_line(t)),
            None => {
                let d = short_description(a, 120);
                lines.push(if d.is_empty() { a.name.clone() } else { format!("{}：{d}", a.name) });
            }
        },
        (None, Some(t)) => lines.push(text_line(t)),
        (None, None) if !ctx.shot.scene_name.is_empty() => lines.push(ctx.shot.scene_name.clone()),
        (None, None) => {}
    }
    // 道具：库里有但没图的、库里没有的，都按文字写（有图的由参考图管外形，这里不重复）
    for t in text.iter().filter(|t| t.kind == AssetKind::Prop) {
        lines.push(format!("道具 {}", text_line(t)));
    }
    for item in &ctx.shot.scene_items {
        if !item.name.trim().is_empty() {
            lines.push(if item.detail.trim().is_empty() { item.name.trim().to_string() } else { format!("{}，{}", item.name.trim(), item.detail.trim()) });
        }
    }
    // 这一场的固定布景（整场共用）：人和灯在场里的位置不因换机位而变。
    // **「开场站位」只给一场的第一镜。** 它是这一场开头人在哪 —— 2026-09-29 第 6 场第 13 镜的线路 A 里，
    // 星澜明明蹲在螺丝列旁，布景里还写着「被陆川横抱，从南墙房门进入」，模型同时收到两个站位。
    // 后面的镜头站位以分段里的 blocking 为准（线路 B 由写手自己取舍，不受影响）
    let mut stage = ctx.shot.stage.clone();
    if ctx.same_scene_prev().is_some() {
        stage.people.clear();
    }
    lines.extend(stage.lines());
    if lines.is_empty() { "未指定场景".into() } else { lines.join("\n") }
}

fn refs(ctx: &ShotContext) -> String {
    let slots = ctx.slots();
    let text = ctx.text_only();
    let no_image = if text.is_empty() {
        String::new()
    } else {
        format!("以下没有参考图，只按文字描述画：{}。", text.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join("、"))
    };
    if slots.is_empty() {
        return if no_image.is_empty() { "本镜不挂参考图。".into() } else { format!("本镜不挂参考图。{no_image}") };
    }
    let mut lines = Vec::new();
    let (mut has_char, mut has_scene, mut has_prop) = (false, false, false);
    for (slot, asset) in &slots {
        let name = asset.map_or(slot.asset_id.as_str(), |a| a.name.as_str());
        let mut line = format!("参考图{}：{}（{}）。控制：{}", slot.order, name, usage_zh(slot.usage), slot.control.trim());
        if !slot.not_control.trim().is_empty() {
            line.push_str(&format!("；不控制：{}", slot.not_control.trim()));
        }
        line.push('。');
        lines.push(line);
        match asset.map(|a| a.kind) {
            Some(AssetKind::Character) => has_char = true,
            Some(AssetKind::Scene) => has_scene = true,
            Some(AssetKind::Prop) => has_prop = true,
            _ => {}
        }
    }
    for (has, rule) in [(has_char, RULE_CHARACTER), (has_scene, RULE_SCENE), (has_prop, RULE_PROP)] {
        if has {
            lines.push(rule.to_string());
        }
    }
    if !no_image.is_empty() {
        lines.push(no_image);
    }
    lines.join("\n")
}

fn prev(ctx: &ShotContext) -> String {
    let end = match ctx.prev {
        None => return "当前分镜为故事开篇".into(),
        Some(p) => p.segments.last().map(|s| s.visual.trim().to_string()).filter(|v| !v.is_empty()).unwrap_or_else(|| p.prompt.trim().to_string()),
    };
    // 同一场：光线、服装、道具位置要和上一镜接得上（实跑镜与镜之间不连贯）
    match ctx.same_scene_prev() {
        Some(p) => {
            let mut keep = vec!["人物服装颜色款式与参考图一致".to_string()];
            if !p.lighting.trim().is_empty() {
                keep.insert(0, format!("光影与上一镜一致（{}）", p.lighting.trim().trim_end_matches('。')));
            }
            if !p.scene_items.is_empty() {
                keep.push(format!("道具位置与上一镜一致（{}）", p.scene_items.iter().map(|i| i.name.trim()).collect::<Vec<_>>().join("、")));
            }
            format!("{end}\n本镜紧接上一镜，同一场景，保持不变：{}。", keep.join("；"))
        }
        None => end,
    }
}

fn segments(ctx: &ShotContext) -> String {
    ctx.shot
        .segments
        .iter()
        .map(|seg| {
            let mut lines = vec![format!(
                "时间段：{}-{}秒。{}",
                secs(seg.start),
                secs(seg.end),
                seg.title.trim().trim_end_matches('。')
            )];
            for (label, text) in [("画面", &seg.visual), ("禁止", &seg.forbid), ("约束", &seg.constraints), ("站位与朝向", &seg.blocking), ("运镜", &seg.camera), ("音效", &seg.sound)] {
                if !text.trim().is_empty() {
                    lines.push(format!("{label}：{}", text.trim()));
                }
            }
            lines.join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn dialogue(ctx: &ShotContext) -> String {
    let lines = &ctx.shot.dialogue_lines;
    if lines.iter().all(|l| l.line.trim().is_empty()) {
        // 正面写成可见状态，不写「无对白」这类排除句（drama-skills minimax-h3.md「无对白镜」）
        return "本镜没有台词，画面里的人物嘴部保持闭合。".into();
    }
    lines
        .iter()
        .filter(|l| !l.line.trim().is_empty())
        .map(|l| {
            // 嗓音写明：两个人都说话时模型会配错（实跑第 4 镜女主变男声）
            //
            // **性别年龄之后还要给表演指示。** 只给「女声，约28岁」的话
            // H3 没有任何方向可循，念出来是播音腔 —— 2026-09-18 用户在成片里
            // 听出来的就是这个，而那时候 `emotion` / `voice` 两个字段
            // 填也没填、读也没读。顺序按官方指南：先身份，再音色，最后这一句的语气。
            let v = ctx.voice(&l.speaker);
            let mut bits: Vec<String> = Vec::new();
            // **角色资产上写死的声音设定优先。** 它整集一份、跨场不变，
            // 而性别年龄只是从描述里抠出来的兜底 —— 有前者时后者是多余的。
            // 不这么做的话模型每场重编一次：实测 E04 李白从「中年沙哑」
            // 变成「青年清亮」（2026-09-19 Codex Astra 的审查指出）。
            if !v.h3.trim().is_empty() {
                bits.push(v.h3.trim().trim_end_matches(['。', '，']).into());
            } else {
                match (v.female, v.age) {
                    (Some(f), Some(age)) => bits.push(format!("{}，约{age}岁", if f { "女声" } else { "男声" })),
                    (Some(f), None) => bits.push(if f { "女声" } else { "男声" }.into()),
                    _ => {}
                }
            }
            for extra in [l.voice.trim(), l.emotion.trim()] {
                if !extra.is_empty() {
                    bits.push(extra.trim_end_matches(['。', '，']).into());
                }
            }
            let voice = if bits.is_empty() { String::new() } else { format!("（{}）", bits.join("，")) };
            format!("{}{}{voice}：“{}”", l.speaker.trim(), if l.off_screen { "（画外音）" } else { "" }, l.line.trim())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn constraints(ctx: &ShotContext) -> String {
    let mut items = vec!["必须保持名称一致。".to_string()];
    for (slot, asset) in ctx.slots() {
        if let Some(a) = asset.filter(|_| !slot.control.trim().is_empty()) {
            items.push(format!("必须保持{}的{}。", a.name, slot.control.trim().trim_end_matches('。')));
        }
    }
    if !ctx.shot.lighting.trim().is_empty() {
        items.push(format!("必须保持光影：{}。", ctx.shot.lighting.trim().trim_end_matches('。')));
    }
    if !ctx.shot.dialogue_lines.is_empty() {
        items.push("台词必须逐字说出，普通话，不增不减。".into());
    }
    items.push("画面中不出现字幕、文字和水印。".into());
    items.iter().enumerate().map(|(i, t)| format!("{}. {t}", i + 1)).collect::<Vec<_>>().join("\n")
}

fn fill(template: &str, vars: &[(&str, String)]) -> String {
    let mut out = template.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out.trim().to_string()
}

/// 合成这一镜的两条提示词。
pub fn synthesize(ctx: &ShotContext, now: u64) -> RoutePrompts {
    let s = ctx.shot;
    let ambient = [s.ambient_sound.trim(), s.sound_effect.trim()].into_iter().filter(|t| !t.is_empty()).collect::<Vec<_>>().join("；");
    let video = fill(
        VIDEO_TEMPLATE,
        &[
            ("CHARACTERS", characters(ctx)),
            ("SCENE", scene(ctx)),
            ("REFS", refs(ctx)),
            ("PREV", prev(ctx)),
            ("STYLE", ctx.style_for_shot()),
            ("CAMERA", camera_line(ctx)),
            ("LIGHTING", if s.lighting.trim().is_empty() { "按场景自然光".into() } else { s.lighting.trim().to_string() }),
            ("AMBIENT", if ambient.is_empty() { "安静".into() } else { ambient }),
            ("SEGMENTS", segments(ctx)),
            ("DIALOGUE", dialogue(ctx)),
            ("CONSTRAINTS", constraints(ctx)),
        ],
    )
    // 没选风格就不留一行空的「画面风格：」
    .lines()
    .filter(|l| l.trim() != "画面风格：")
    .collect::<Vec<_>>()
    .join("\n");
    // 分镜提示词是一张静帧：只取开场那一段的画面，不写运镜（静帧里没有「推」）
    let still: Vec<String> = [("shotSize", &s.scale), ("angle", &s.camera_angle)]
        .into_iter()
        .map(|(g, id)| crate::shot_language::display(g, id))
        .filter(|n| !n.is_empty())
        .collect();
    let items = s.scene_items.iter().filter(|i| !i.name.trim().is_empty()).map(|i| format!("{}，{}", i.name.trim(), i.detail.trim())).collect::<Vec<_>>().join("；");
    // 风格放在最前面：放在构图后面会被摄影词淹没，出成写实图（资产出图踩过）
    let frame = fill(
        FRAME_TEMPLATE,
        &[
            ("STYLE", ctx.style_for_shot()),
            ("CAMERA", if still.is_empty() { "中景".into() } else { still.join("，") }),
            ("SCENE", ctx.scene().map(|a| a.name.clone()).unwrap_or_else(|| s.scene_name.clone())),
            ("VISUAL", s.segments.first().map(|g| g.visual.trim().to_string()).unwrap_or_default()),
            // **站位要进首帧。** 以前只有画面没有站位 —— 挂了首帧的镜头整镜都照首帧的站位演，
            // 首帧里谁在左谁在右画错了，后面怎么写都拉不回来（2026-09-28 对照报告第 0 节）
            ("BLOCKING", s.segments.first().map(|g| g.blocking.trim().to_string()).unwrap_or_default()),
            ("ITEMS", items),
            ("LIGHTING", s.lighting.trim().to_string()),
        ],
    )
    .lines()
    .filter(|l| !l.trim().is_empty() && l.trim() != "光影：" && l.trim() != "站位：")
    .collect::<Vec<_>>()
    .join("\n");
    RoutePrompts { frame, video, synthesized_at: now, fields_hash: fields_hash(ctx) }
}

/// 合成结果的结构检查。返回问题列表，空 = 合格。
pub fn check(ctx: &ShotContext, p: &RoutePrompts) -> Vec<String> {
    let mut out = Vec::new();
    for head in ["出场角色：", "背景场景：", "参考图使用规则：", "前一个分镜描述：", "当前分镜的分段描述：", "台词：", "输出约束：", "non_diegetic_music: N/A"] {
        if !p.video.contains(head) {
            out.push(format!("视频提示词缺少「{head}」"));
        }
    }
    if p.video.contains('{') || p.frame.contains('{') {
        out.push("模板里还有没填上的占位".into());
    }
    for (slot, _) in ctx.slots() {
        if !p.video.contains(&format!("参考图{}：", slot.order)) {
            out.push(format!("参考图 {} 没交代是谁", slot.order));
        }
    }
    for l in &ctx.shot.dialogue_lines {
        if !l.line.trim().is_empty() && !p.video.contains(l.line.trim()) {
            out.push(format!("台词「{}」没有逐字进提示词", l.line.trim()));
        }
    }
    if p.frame.trim().is_empty() {
        out.push("分镜提示词是空的".into());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cinema::{DialogueLine, StoryboardShot};
    use crate::shot_table::{RefSlot, SceneItem, ShotSegment};
    use crate::storyboard_gen::AssetRef;

    fn assets() -> Vec<AssetRef> {
        vec![
            AssetRef { id: "char_shen".into(), kind: AssetKind::Character, name: "林婉清（现代）".into(), has_image: true, description: "描述：现代办公室职员，女性，28岁，身高165厘米".into(), look_id: String::new(), looks: Vec::new(), h3_voice: String::new() },
            AssetRef { id: "scene_office".into(), kind: AssetKind::Scene, name: "现代办公室".into(), has_image: true, description: "内景，深夜办公室".into(), look_id: String::new(), looks: Vec::new(), h3_voice: String::new() },
            AssetRef { id: "prop_phone".into(), kind: AssetKind::Prop, name: "手机".into(), has_image: true, description: "现代智能手机".into(), look_id: String::new(), looks: Vec::new(), h3_voice: String::new() },
        ]
    }

    pub(crate) fn shot() -> StoryboardShot {
        let mut s: StoryboardShot = serde_json::from_value(serde_json::json!({
            "id": "s1", "shot_num": 1, "scale": "medium", "camera_movement": "static", "dialogue": "",
            "prompt": "", "duration": 8.0, "status": "pending"
        }))
        .unwrap();
        s.camera_angle = "eye_level".into();
        s.character_ids = vec!["char_shen".into()];
        s.scene_id = Some("scene_office".into());
        s.prop_ids = vec!["prop_phone".into()];
        s.lighting = "电脑屏幕冷蓝光从正面打脸，背景全暗".into();
        s.ambient_sound = "空调低频嗡嗡声".into();
        s.scene_items = vec![SceneItem { name: "手机".into(), detail: "长约15厘米，放在键盘右边".into() }];
        s.segments = vec![
            ShotSegment { start: 0.0, end: 4.0, title: "打字与震动".into(), visual: "双手疯狂敲击键盘，手机不断震动".into(), forbid: "手离开键盘".into(), constraints: "电脑蓝光为主光源".into(), blocking: "直视前方电脑".into(), camera: "固定机位".into(), sound: "急促的键盘声".into() },
            ShotSegment { start: 4.0, end: 8.0, title: "晕倒与切黑".into(), visual: "身体向前倒下，最后0.5秒黑屏".into(), ..Default::default() },
        ];
        s.dialogue_lines = vec![DialogueLine { speaker: "林婉清（现代）".into(), line: "我还能撑。".into(), ..Default::default() }];
        s.ref_slots = vec![
            RefSlot { order: 1, asset_id: "char_shen".into(), usage: RefUsage::Identity, control: "脸型、披肩直发、白衬衫".into(), not_control: "姿势、构图".into() },
            RefSlot { order: 2, asset_id: "prop_phone".into(), usage: RefUsage::Identity, control: "手机外形".into(), not_control: String::new() },
        ];
        s
    }

    #[test]
    /// **表演指示必须发出去。** 只给「女声，约28岁」的话 H3 没有方向，
    /// 念出来是播音腔 —— 2026-09-18 用户在成片里听出来的就是这个。
    /// 顺序按官方指南：身份 → 音色 → 这一句的语气。
    /// **角色资产上的声音设定优先。**
    ///
    /// 镜头表一场一次调用，模型看不到别的场 —— 要求它「整集一致」等于
    /// 要求它遵守一个看不见的约束。实测 E04 李白第 2 镜「中年沙哑」、
    /// 第 8 镜「青年清亮」，不是醉意的差别，是换了个人。
    #[test]
    fn 资产上的声音设定优先于性别年龄() {
        let mut s = shot();
        s.dialogue_lines[0].emotion = "带醉意".into();
        let mut a = assets();
        for x in a.iter_mut() {
            if x.name.starts_with("林婉清") {
                x.h3_voice = "嗓音沙哑偏低的中年女声，语速偏慢".into();
            }
        }
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let v = synthesize(&ctx, 42).video;
        assert!(v.contains("（嗓音沙哑偏低的中年女声，语速偏慢，带醉意）"), "资产声音没顶上：{v}");
        assert!(!v.contains("女声，约28岁"), "资产有声音设定时不该再写性别年龄：{v}");
    }

    /// 开场站位只给一场的第一镜（09-29 第 13 镜：布景里的开场站位和这一镜的站位打架）
    #[test]
    fn 开场站位只进一场的第一镜() {
        let mut s = shot();
        s.scene_no = 6;
        s.stage.people = vec!["星澜：被陆川横抱，从南墙房门进入".into()];
        s.stage.facing = "从南墙房门往里看".into();
        let a = assets();
        let first = synthesize(&ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false }, 42).video;
        assert!(first.contains("开场站位：星澜：被陆川横抱"), "{first}");
        let mut prev = s.clone();
        prev.shot_num = 12;
        let later = synthesize(&ShotContext { shot: &s, prev: Some(&prev), assets: &a, style: "", long_take: false }, 42).video;
        assert!(!later.contains("开场站位"), "同一场后面的镜头不该带开场站位：{later}");
        assert!(later.contains("方向基准：从南墙房门往里看"), "别的布景照带");
    }

    #[test]
    fn 语气和音色写进台词行() {
        let mut s = shot();
        s.dialogue_lines[0].voice = "嗓音沙哑偏低".into();
        s.dialogue_lines[0].emotion = "压低声音，几乎是气音。".into();
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let v = synthesize(&ctx, 42).video;
        assert!(
            v.contains("林婉清（现代）（女声，约28岁，嗓音沙哑偏低，压低声音，几乎是气音）：“我还能撑。”"),
            "语气/音色没写进去，或顺序不对：{v}"
        );
    }

    #[test]
    fn 按试片_a_的结构填空_每段都有内容() {
        let s = shot();
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let p = synthesize(&ctx, 42);
        assert_eq!(check(&ctx, &p), Vec::<String>::new(), "{}", p.video);
        let v = &p.video;
        assert!(v.starts_with("出场角色：\n林婉清（现代）：现代办公室职员，女性，28岁"), "{v}");
        assert!(v.contains("背景场景：\n现代办公室：内景，深夜办公室\n手机，长约15厘米，放在键盘右边"), "{v}");
        assert!(v.contains("参考图1：林婉清（现代）（身份）。控制：脸型、披肩直发、白衬衫；不控制：姿势、构图。"), "{v}");
        assert!(v.contains(RULE_CHARACTER) && v.contains(RULE_PROP), "挂了人物和道具图，对应规则要写");
        assert!(!v.contains(RULE_SCENE), "没挂场景图就不写场景规则");
        assert!(v.contains("前一个分镜描述：\n当前分镜为故事开篇"));
        assert!(v.contains("镜头：中景，平视，固定"), "{v}");
        assert!(v.contains("时间段：0-4秒。打字与震动\n画面：双手疯狂敲击键盘，手机不断震动\n禁止：手离开键盘\n约束：电脑蓝光为主光源\n站位与朝向：直视前方电脑\n运镜：固定机位\n音效：急促的键盘声"), "{v}");
        assert!(v.contains("时间段：4-8秒。晕倒与切黑\n画面：身体向前倒下，最后0.5秒黑屏\n\n台词："), "空字段不该留下空行标签：{v}");
        assert!(v.contains("林婉清（现代）（女声，约28岁）：“我还能撑。”"), "嗓音要写明：{v}");
        assert!(v.contains("必须保持林婉清（现代）的脸型、披肩直发、白衬衫。"));
        assert!(v.trim_end().ends_with("non_diegetic_music: N/A"), "配乐通道必须显式关掉");
        assert!(!p.frame.contains("固定"), "静帧不写运镜：{}", p.frame);
        assert!(p.frame.starts_with("中景，平视。现代办公室。双手疯狂敲击键盘"), "{}", p.frame);
        assert!(p.frame.contains("双手疯狂敲击键盘，手机不断震动\n站位：直视前方电脑\n"), "首帧要带第一段的站位：{}", p.frame);
        assert_eq!((p.synthesized_at, p.fields_hash.as_str()), (42, fields_hash(&ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false }).as_str()));
    }

    #[test]
    fn 上一镜_无台词_无参考图() {
        let mut prev = shot();
        prev.segments[1].visual = "她倒在桌上，黑屏".into();
        let mut s = shot();
        s.dialogue_lines.clear();
        s.ref_slots.clear();
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: Some(&prev), assets: &a, style: "", long_take: false };
        let p = synthesize(&ctx, 1);
        assert!(p.video.contains("前一个分镜描述：\n她倒在桌上，黑屏"));
        assert!(p.video.contains("本镜没有台词，画面里的人物嘴部保持闭合。"));
        assert!(p.video.contains("参考图使用规则：\n本镜不挂参考图。"));
        assert!(!p.video.contains("台词必须逐字说出"));
        assert_eq!(check(&ctx, &p), Vec::<String>::new());
    }

    #[test]
    fn 结构检查能挡住缺段和漏台词() {
        let s = shot();
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let mut p = synthesize(&ctx, 1);
        p.video = p.video.replace("non_diegetic_music: N/A", "").replace("我还能撑。", "").replace("参考图2：", "");
        let issues = check(&ctx, &p);
        assert!(issues.iter().any(|i| i.contains("non_diegetic_music")), "{issues:?}");
        assert!(issues.iter().any(|i| i.contains("我还能撑")), "{issues:?}");
        assert!(issues.iter().any(|i| i.contains("参考图 2")), "{issues:?}");
    }

    /// 用户要求的兜底：没图的不挂参考图、按文字写；库里没有的也写上，编号跟着实际挂的图走。
    #[test]
    fn 没图和库里没有的按文字兜底() {
        use crate::shot_table::UnlinkedAsset;
        let mut a = assets();
        a[0].has_image = false; // 林婉清还没图
        let mut s = shot();
        s.unlinked_assets = vec![
            UnlinkedAsset { kind: "character".into(), name: "老板".into() },
            UnlinkedAsset { kind: "prop".into(), name: "工牌".into() },
        ];
        s.scene_items.push(SceneItem { name: "工牌".into(), detail: "挂在她脖子上".into() });
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let p = synthesize(&ctx, 1);
        let v = &p.video;
        assert_eq!(check(&ctx, &p), Vec::<String>::new(), "{v}");
        assert!(v.contains("林婉清（现代）：现代办公室职员，女性，28岁，身高165厘米（没有参考图，按此描述）"), "{v}");
        assert!(v.contains("老板（资产库里没有，按剧本描述）"), "{v}");
        assert!(v.contains("道具 工牌：挂在她脖子上（资产库里没有，按剧本描述）"), "{v}");
        assert!(v.contains("参考图1：手机（身份）"), "林婉清没图，手机要顶到参考图 1：{v}");
        assert!(!v.contains("参考图2："), "{v}");
        assert!(!v.contains(RULE_CHARACTER), "没挂人物图就不写人物图规则：{v}");
        assert!(v.contains("以下没有参考图，只按文字描述画：林婉清（现代）、老板、工牌。"), "{v}");
        assert!(!v.contains("必须保持林婉清（现代）的"), "没图的不该要求「保持参考图里的」：{v}");
    }

    /// 实跑镜与镜之间不连贯：同一场的上一镜，光影、道具、服装要写进本镜；换场不写。
    #[test]
    fn 同一场紧接上一镜_写明保持不变的东西() {
        let mut prev = shot();
        prev.scene_no = 1;
        prev.segments[1].visual = "她倒在桌上".into();
        let mut s = shot();
        s.scene_no = 1;
        let a = assets();
        let v = synthesize(&ShotContext { shot: &s, prev: Some(&prev), assets: &a, style: "", long_take: false }, 1).video;
        assert!(v.contains("前一个分镜描述：\n她倒在桌上\n本镜紧接上一镜，同一场景，保持不变：光影与上一镜一致（电脑屏幕冷蓝光从正面打脸，背景全暗）；人物服装颜色款式与参考图一致；道具位置与上一镜一致（手机）。"), "{v}");
        s.scene_no = 2;
        let v = synthesize(&ShotContext { shot: &s, prev: Some(&prev), assets: &a, style: "", long_take: false }, 1).video;
        assert!(v.contains("前一个分镜描述：\n她倒在桌上\n\n---"), "换场不要求延续：{v}");
    }

    #[test]
    fn 风格写进视频和分镜提示词_没选不留空行() {
        let s = shot();
        let a = assets();
        let p = synthesize(&ShotContext { shot: &s, prev: None, assets: &a, style: "水墨国风，淡雅留白", long_take: false }, 1);
        assert!(p.video.contains("当前分镜的分段描述：\n画面风格：水墨国风，淡雅留白\n镜头："), "{}", p.video);
        assert!(p.frame.starts_with("水墨国风，淡雅留白\n中景"), "风格要在构图前面：{}", p.frame);
        let none = synthesize(&ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false }, 1);
        assert!(!none.video.contains("画面风格"), "{}", none.video);
        assert!(none.frame.starts_with("中景"), "{}", none.frame);
        assert_ne!(p.fields_hash, none.fields_hash);
        let mut e = shot();
        e.segments[0].blocking.clear();
        let f = synthesize(&ShotContext { shot: &e, prev: None, assets: &a, style: "", long_take: false }, 1).frame;
        assert!(!f.contains("站位"), "没写站位就不留空标签：{f}");
    }

    /// 两条线路物理隔离：不互相 import、不共用拼装函数。
    #[test]
    fn 不引用线路_b() {
        let src = include_str!("prompt_route_a.rs");
        // 只看代码，不看注释（模块注释里要说明隔离规则，会提到对方的名字）
        let prod: String = src[..src.find("#[cfg(test)]").unwrap()].lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
        assert!(!prod.contains("prompt_route_b"), "线路 A 引用了线路 B");
        assert!(!prod.contains("h3_prompt"), "线路 A 用了旧的拼装模块");
    }
}
