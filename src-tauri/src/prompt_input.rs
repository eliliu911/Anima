//! 合成提示词的输入：两条线路**唯一共用**的东西。
//!
//! 见 `docs/plans/2026-09-13-分镜重构方案.md` 「两条提示词线路，物理隔离」：
//! 共用的只有镜头表字段、参考图槽位；两条线路各自一个模块（`prompt_route_a` / `prompt_route_b`），
//! 互相不 import、不共用拼装函数。所以这里**只放数据和查找**，不放任何把字段写成句子的代码。

use crate::assets::AssetKind;
use crate::cinema::StoryboardShot;
use crate::shot_table::RefSlot;
use crate::storyboard_gen::AssetRef;

/// 合成一镜提示词要读的全部东西。
pub struct ShotContext<'a> {
    pub shot: &'a StoryboardShot,
    /// 同一集里的上一镜（第一镜为 `None`）
    pub prev: Option<&'a StoryboardShot>,
    /// 这一集能用的资产
    pub assets: &'a [AssetRef],
    /// 项目当前的画面风格（风格那一步选的，实际生效的正向词）。空 = 没选
    pub style: &'a str,
    /// **这一镜是长镜头里的一个窗口。**
    ///
    /// 长镜头的窗口首尾要接上下一个，所以正文里要多一条约束：
    /// **台词必须在重叠区之前说完**。见 [`crate::long_take`] 和
    /// `prompt_route_b::assemble`。
    ///
    /// 不是长镜头时为 `false`，指纹里也不会多出键（老分集一条都不许误报）。
    pub long_take: bool,
}

/// 关键帧出片时首帧 / 尾帧那一格「管什么」。写进提示词的 `<Picture N> is the opening frame of this shot: …`
pub const KEYFRAME_FIRST_CONTROL: &str = "the video begins exactly on this image — its composition, camera position, staging, lighting, and every person's face, hair and costume";
pub const KEYFRAME_LAST_CONTROL: &str = "the video ends exactly on this image — its composition, camera position and final poses";

impl<'a> ShotContext<'a> {
    /// 这一镜按关键帧出片（挂了首帧）。**长镜头的窗口不算** —— 整段出片（`cinema::run_long_take`）
    /// 靠窗口之间的重叠帧衔接，不吃关键帧。
    pub fn keyframe_video(&self) -> bool {
        self.shot.keyframes.drives_video() && !self.long_take
    }

    pub fn asset(&self, id: &str) -> Option<&'a AssetRef> {
        self.assets.iter().find(|a| a.id == id)
    }

    /// **这一镜用的画风文字**：项目画风里点名了某个角色、而这个角色不在这一镜的，那一句不发。
    ///
    /// # 为什么（2026-10-06 EP03 第 3、5 镜）
    ///
    /// 《回收师》的项目画风里有一句「星澜保持精致非人的伴侣机器人气质，以裸肤色、珍珠白和青蓝能量核心从脏暗环境中突出」，
    /// 每一镜都带着。第 3、5 镜没有星澜，出首帧时画面里唯一的年轻女性是苏晴 —— Qwen 把「珍珠白、青蓝能量核心的机器人」
    /// 套到了她身上，首帧里苏晴穿着星澜的机甲紧身衣，首帧出片照着演，苏晴变成了机器人。
    ///
    /// 按句号切，含「不在这一镜的角色名」的句子去掉；其余一字不动。只在发出去的正文里用 ——
    /// 指纹仍按原画风算（[`fields_hash`]），已经合成好的提示词不会因此全部过期。
    pub fn style_for_shot(&self) -> String {
        let style = self.style.trim();
        if style.is_empty() {
            return String::new();
        }
        let cast: Vec<&str> = self.characters().iter().map(|a| a.name.as_str()).collect();
        let absent: Vec<&str> = self.assets.iter()
            .filter(|a| a.kind == AssetKind::Character && a.name.chars().count() >= 2 && !cast.contains(&a.name.as_str()))
            .map(|a| a.name.as_str())
            .collect();
        let mut out = String::with_capacity(style.len());
        for sentence in style.split_inclusive('。') {
            if absent.iter().any(|n| sentence.contains(n)) {
                continue;
            }
            out.push_str(sentence);
        }
        out.trim().to_string()
    }

    /// 出场人物，按镜头表里的顺序。
    pub fn characters(&self) -> Vec<&'a AssetRef> {
        self.shot.all_character_ids().iter().filter_map(|id| self.asset(id)).filter(|a| a.kind == AssetKind::Character).collect()
    }

    pub fn scene(&self) -> Option<&'a AssetRef> {
        self.shot.scene_id.as_deref().and_then(|id| self.asset(id))
    }

    pub fn props(&self) -> Vec<&'a AssetRef> {
        self.shot.prop_ids.iter().filter_map(|id| self.asset(id)).collect()
    }

    /// **这一次真正挂上的参考图**：槽位按编号排好，只留资产在库里、而且有图的，编号从 1 重排。
    ///
    /// 没图的槽位留在镜头表里（补上图再合成就挂上），这里跳过，交给 [`Self::text_only`] 按文字写。
    /// 两条线路和出片都用这一份 —— 提示词里的 `<Picture N>` / 「参考图 N」和实际挂图的顺序才对得上。
    ///
    /// 起始帧 / 结束帧不是资产库里的资产（是这一镜自己出的图），照留，资产为 `None`。
    pub fn slots(&self) -> Vec<(RefSlot, Option<&'a AssetRef>)> {
        use crate::shot_table::RefUsage;
        // **挂了首帧就走关键帧出片**（H3 的 FL2VA 分区），那条路只吃首帧 / 尾帧，
        // 不吃资产参考图 —— 分区不同，两种输入不能同时进（见 `weetodd::H3Mode`）。
        // 人长什么样由首帧定，资产图在出首帧那一步已经用过了。
        let kf = self.shot.keyframes;
        if self.keyframe_video() {
            let frame = |order: u8, usage: RefUsage, control: &str| RefSlot { order, asset_id: String::new(), usage, control: control.into(), not_control: String::new() };
            let mut out = vec![(frame(1, RefUsage::StartFrame, KEYFRAME_FIRST_CONTROL), None)];
            if kf.last {
                out.push((frame(2, RefUsage::EndFrame, KEYFRAME_LAST_CONTROL), None));
            }
            return out;
        }
        let mut v: Vec<&RefSlot> = self.shot.ref_slots.iter().collect();
        v.sort_by_key(|s| s.order);
        v.into_iter()
            .filter_map(|s| {
                let asset = self.asset(&s.asset_id);
                let frame = matches!(s.usage, RefUsage::StartFrame | RefUsage::EndFrame);
                match asset {
                    Some(a) if a.has_image => Some((s, Some(a))),
                    None if frame => Some((s, None)),
                    _ => None,
                }
            })
            // H3 官方规格：一次最多 9 张参考图。超出的按文字写（见 text_only），不能静默丢
            .take(crate::agent_runtime::video_tool::MAX_REFERENCE_IMAGES)
            .enumerate()
            .map(|(i, (s, a))| (with_outfit(RefSlot { order: i as u8 + 1, ..s.clone() }, a), a))
            .collect()
    }

    /// 同一场的上一镜（场号相同）。换场了就是 `None` —— 换场不要求光线、道具延续。
    pub fn same_scene_prev(&self) -> Option<&'a StoryboardShot> {
        self.prev.filter(|p| self.shot.scene_no > 0 && p.scene_no == self.shot.scene_no)
    }

    /// 说话人的嗓音：按资产描述里的性别、年龄。找不到这个人或描述里没写就是空的。
    ///
    /// 2026-09-14 实跑第 4 镜：大臣和林婉清都说话，提示词只有 (S1)(S2)，女主变成了男声。
    /// 官方指南：嗓音、性别写在 `<d>` 外面。
    pub fn voice(&self, speaker: &str) -> Voice {
        let Some(a) = self.assets.iter().find(|a| a.kind == AssetKind::Character && a.name == speaker.trim()) else {
            return Voice::default();
        };
        let h3 = a.h3_voice.trim().to_string();
        let d = &a.description;
        let female = if d.contains("女性") || d.contains("，女，") || d.contains("（女") { Some(true) } else if d.contains("男性") || d.contains("，男，") || d.contains("（男") { Some(false) } else { None };
        let age = d.find('岁').and_then(|at| {
            let digits: String = d[..at].chars().rev().take_while(|c| c.is_ascii_digit()).collect::<Vec<_>>().into_iter().rev().collect();
            digits.parse().ok()
        });
        Voice { female, age, h3 }
    }
    /// **兜底**：这一镜里要出现、但没有参考图可挂的东西，提示词里要按文字写清楚。
    ///
    /// - 资产在库里但还没有图（人物 / 场景 / 道具）：描述取资产库的；
    /// - 镜头表里写了、资产库里没有（[`crate::shot_table::UnlinkedAsset`]）：描述取镜头表「场景物件」里同名那条，没有就只有名字。
    pub fn text_only(&self) -> Vec<TextOnly> {
        let mut out: Vec<TextOnly> = Vec::new();
        // 有图但排在第 9 张之后、挂不上的，也按文字写
        let hung: Vec<String> = self.slots().iter().filter_map(|(_, a)| a.map(|a| a.id.clone())).collect();
        let frames = self.keyframe_video();
        for slot in &self.shot.ref_slots {
            if let Some(a) = self.asset(&slot.asset_id).filter(|a| a.has_image && !hung.contains(&a.id)).filter(|a| !(frames && a.kind == AssetKind::Character)) {
                if !out.iter().any(|t| t.name == a.name) {
                    out.push(TextOnly { kind: a.kind, name: a.name.clone(), description: short_description(a, 120), in_library: true });
                }
            }
        }
        let mut ids: Vec<String> = self.shot.all_character_ids();
        ids.extend(self.shot.scene_id.clone());
        ids.extend(self.shot.prop_ids.iter().cloned());
        // 关键帧出片时人物由首帧定义成主体（见 `prompt_route_b::tags`），不在这里重复。
        for id in &ids {
            if let Some(a) = self.asset(id).filter(|a| !a.has_image).filter(|a| !(frames && a.kind == AssetKind::Character)) {
                if !out.iter().any(|t| t.name == a.name) {
                    out.push(TextOnly { kind: a.kind, name: a.name.clone(), description: short_description(a, 120), in_library: true });
                }
            }
        }
        for u in &self.shot.unlinked_assets {
            let kind = match u.kind.as_str() {
                "character" => AssetKind::Character,
                "scene" => AssetKind::Scene,
                _ => AssetKind::Prop,
            };
            let detail = self.shot.scene_items.iter().find(|i| i.name.trim() == u.name.trim()).map(|i| i.detail.trim().to_string()).unwrap_or_default();
            if !u.name.trim().is_empty() && !out.iter().any(|t| t.name == u.name.trim()) {
                out.push(TextOnly { kind, name: u.name.trim().to_string(), description: detail, in_library: false });
            }
        }
        out
    }
}


/// 一个说话人的嗓音线索。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Voice {
    pub female: Option<bool>,
    pub age: Option<u32>,
    /// 角色资产上写死的声音设定（中文）。整集一份，跨场不变。
    ///
    /// 有它的时候**它说了算**：性别年龄只是兜底，而它已经把这些包含在内了。
    pub h3: String,
}

/// 人物的身份参考图**必须管服装**：「不控制」里去掉服装，「控制」里没提就补上。
///
/// 2026-09-14 实跑第 4 镜衣服变蓝：镜头表把「服装细节」写进身份图的不控制（第 2 镜实例），
/// 等于告诉模型别从图里拿衣服。同一场里人物换装是剧本才能决定的事，要换就用「造型状态」那张图。
///
/// **例外：图是主形象、角色另有造型、镜头表又明说不控制服装** —— 那是「这一集换了装、但没选造型」，
/// 图上的衣服是错的，**不能**反过来让它管服装（[`outfit_mismatch`]）。2026-10-07 EP03：星澜整集穿「嫁衣改晚礼服」，
/// 镜头表写着「不控制：晚礼服具体改制样式」，但没选造型、挂的是主形象（银链纱裙 + 吊袜带），这里把不控制删了、
/// 又补上「服装颜色和款式」—— 原生 Ref2VA 两条对照里苏晴穿上了星澜的银链纱裙。这种情况原样保留，并在校验里提示去选造型。
fn with_outfit(mut slot: RefSlot, asset: Option<&AssetRef>) -> RefSlot {
    use crate::shot_table::RefUsage;
    if slot.usage != RefUsage::Identity || !asset.is_some_and(|a| a.kind == AssetKind::Character) {
        return slot;
    }
    if outfit_mismatch(&slot, asset) {
        return slot;
    }
    let is_outfit = is_outfit_word;
    slot.not_control = slot
        .not_control
        .split(['、', '，', ',', '；'])
        .map(str::trim)
        .filter(|t| !t.is_empty() && !is_outfit(t))
        .collect::<Vec<_>>()
        .join("、");
    if !is_outfit(&slot.control) {
        slot.control = if slot.control.trim().is_empty() { "服装颜色和款式".into() } else { format!("{}、服装颜色和款式", slot.control.trim().trim_end_matches('。')) };
    }
    slot
}

fn is_outfit_word(t: &str) -> bool {
    ["服", "衣", "穿着", "衫", "裤", "裙", "袍", "褂", "甲"].iter().any(|w| t.contains(w))
}

/// 镜头表说这张身份图**不管服装**，而挂的是主形象、这个角色又另有造型 —— 多半是这一集换了装、没选造型。
///
/// **只认「没选」**（`look_id` 为空）。在镜头详情里明确选了主形象的（`main`）就是这一镜穿主形象，图上的衣服是对的 ——
/// 2026-10-10 EP04 第 19 镜：同一集第 14~17 镜都给星澜选了主形象，这一镜照样报「没选造型」，
/// 而且提示词里身份图还保留「不控制服装褶皱」、不补「服装颜色和款式」，选了也改不掉。
pub fn outfit_mismatch(slot: &RefSlot, asset: Option<&AssetRef>) -> bool {
    let Some(a) = asset else { return false };
    let on_main = a.look_id.is_empty();
    let has_other = a.looks.iter().any(|l| !l.id.is_empty() && l.id != crate::assets::MAIN_LOOK);
    let says_not_outfit = slot.not_control.split(['、', '，', ',', '；']).map(str::trim).any(|t| !t.is_empty() && is_outfit_word(t));
    a.kind == AssetKind::Character && on_main && has_other && says_not_outfit
}

/// 没有参考图、只能按文字写进提示词的一样东西。
#[derive(Debug, Clone, PartialEq)]
pub struct TextOnly {
    pub kind: AssetKind,
    pub name: String,
    pub description: String,
    /// 在资产库里（只是还没图）还是库里根本没有
    pub in_library: bool,
}

/// 镜头表字段的指纹。合成时存进提示词；字段改过、指纹对不上，界面提示重新合成。
///
/// 只算合成真正会读的字段 —— 出片状态、缩略图版本这类变了不影响提示词。
/// 用 FNV-1a 而不是 `DefaultHasher`：后者不保证跨 Rust 版本稳定，升级工具链会让所有镜头都显示「需要重新合成」。
///
/// 另外算进「哪几个参考图资产现在有图」：给资产补上图（生成或上传）之后，
/// 这一镜的提示词要从文字兜底换成挂图，所以要报「要重新合成」。
///
/// 风格也算进去：换了风格，提示词里的风格描述就过期了。
///
/// **合成时读到的别处内容也算进去**（2026-09-14 用户要求补的两个漏洞）：
/// - 上一镜的结尾画面（两条线路都写进「前一个分镜」），同一场时还有上一镜的光影和道具 ——
///   改了上一镜，这一镜的衔接就过期了；
/// - 用到的资产的文字描述（外貌、陈设写进了提示词）—— 改了描述也过期。
/// **合成规则的版本**，进指纹。规则有实质变化（写法会不一样）时手动加一，旧提示词就判过期、要重新合成。
///
/// 2026-10-07（Codex 复查 §19，用户同意）：v0.42.177 禁了东南西北，可 EP03 存着的 24 镜提示词还是 13:18 按旧规则合的，
/// 指纹只看镜头表和资产，没人知道它们过期了。**只改措辞、不影响输出的别加** —— 一加就是全库每一镜都要重合成。
///
/// | 版本 | 什么时候 | 变了什么 |
/// |---|---|---|
/// | `route-b-2026-10-07` | v0.42.178 | 人物基数句、外貌一样细、彩光不改肤色（v0.42.171）；换装没选造型不反转服装（v0.42.175）；不写东南西北（v0.42.177） |
pub const COMPILER_REV: &str = "route-b-2026-10-07";

pub fn fields_hash(ctx: &ShotContext) -> String {
    let fields = fields_json(ctx);
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in fields.to_string().bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// 指纹算的是哪些字段。**往这里加键会让所有已经合成过的镜头指纹一起变**（全部误报要重新合成），
/// 新键只能在有值时才加。`compiler`（[`COMPILER_REV`]）是故意无条件加的：合成规则升级就该全部重合成。
pub(crate) fn fields_json(ctx: &ShotContext) -> serde_json::Value {
    let (shot, assets, style) = (ctx.shot, ctx.assets, ctx.style);
    let with_image: Vec<&str> = shot.ref_slots.iter().filter(|s| assets.iter().any(|a| a.id == s.asset_id && a.has_image)).map(|s| s.asset_id.as_str()).collect();
    let mut used: Vec<String> = shot.all_character_ids();
    used.extend(shot.scene_id.clone());
    used.extend(shot.prop_ids.iter().cloned());
    used.extend(shot.ref_slots.iter().map(|s| s.asset_id.clone()));
    used.sort();
    used.dedup();
    let descriptions: Vec<(&str, &str)> = used.iter().filter_map(|id| assets.iter().find(|a| &a.id == id)).map(|a| (a.name.as_str(), a.description.as_str())).collect();
    let prev_end = ctx.prev.map(|p| (p.segments.last().map(|s| s.visual.as_str()).unwrap_or_default(), p.prompt.as_str()));
    let prev_scene = ctx.same_scene_prev().map(|p| (p.lighting.as_str(), p.scene_items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>()));
    let fields = serde_json::json!({
        "descriptions": descriptions,
        "prev_end": prev_end,
        "prev_scene": prev_scene,
        "with_image": with_image,
        "style": style.trim(),
        "unlinked": shot.unlinked_assets,
        "duration": shot.duration,
        "scale": shot.scale,
        "angle": shot.camera_angle,
        "move": shot.camera_movement,
        "camera_note": shot.camera_prompt_note(),
        "lighting": shot.lighting,
        "scene_items": shot.scene_items,
        "segments": shot.segments,
        "dialogue": shot.dialogue_lines.iter().map(|l| (&l.speaker, &l.line, l.off_screen)).collect::<Vec<_>>(),
        "ambient_sound": shot.ambient_sound,
        "sound_effect": shot.sound_effect,
        "characters": shot.all_character_ids(),
        "scene": shot.scene_id,
        "props": shot.prop_ids,
        "ref_slots": shot.ref_slots,
        "duty": shot.duty,
        "compiler": COMPILER_REV,
    });
    // 形象描述碰巧相同、图不同时也要标出来。**只在真的用了非主形象时才进指纹** ——
    // 无条件加这个键的话，所有合成过的镜头指纹一起变、全部误报「要重新合成」（2026-09-15 自己踩的）
    let looks: Vec<(&str, &str)> = used.iter().filter_map(|id| assets.iter().find(|a| &a.id == id)).filter(|a| !a.look_id.is_empty()).map(|a| (a.id.as_str(), a.look_id.as_str())).collect();
    let mut fields = fields;
    if !looks.is_empty() {
        fields["looks"] = serde_json::json!(looks);
    }
    // **长镜头窗口进指纹。** 它的正文里多一条「台词要在重叠区之前说完」——
    // 不进指纹的话，勾上长镜头之后旧提示词不判过期，出片用的还是没那条约束的版本。
    // 同样只在真的是长镜头时才加键。
    if ctx.long_take {
        fields["long_take"] = serde_json::json!(true);
    }
    // 一镜出的镜头正文里允许写硬切（见 `prompt_route_b` 的 ONE-TAKE 说明）。同样只在是的时候加键
    if shot.one_take {
        fields["one_take"] = serde_json::json!(true);
    }
    // 这一场的固定布景进了正文（线路 A 的背景场景、线路 B 的 Scene staging）。
    // **只在有的时候加键** —— 老镜头没有，指纹一个字不变，不误报「要重新合成」
    if !shot.stage.is_empty() {
        fields["stage"] = serde_json::json!(shot.stage);
    }
    // **关键帧的有无决定出片模式**（参考生视频 ↔ 首帧 ↔ 首尾帧），三种的正文写法不同。
    // 只记有没有、不记是哪张：换一张首帧不用重新合成。同样只在挂了的时候加键
    if !shot.keyframes.is_none() {
        fields["keyframes"] = serde_json::json!([shot.keyframes.first, shot.keyframes.last]);
    }
    // **接上一镜的话，接的是哪一条 take 也要进指纹。**
    //
    // 用户 2026-09-20 要的是「从第二镜开始有个复选框，要不要以上一镜作为参考」。
    // Codex 同时点出了配套的一条：「重做前段后，后段不能继续接旧版本」。
    //
    // 这两件事在这里合成一条：`continuity != None` 时，把**上一镜最新那条
    // take 的创建时间**放进指纹。上一镜一重出，这一镜自动标「要重新合成」——
    // 不然这一镜的提示词里写着 `<Video 1>` 指的是旧那条片子，而出片时
    // 拿到的是新的，两边对不上，**而且不报错**。
    //
    // 同样**只在真的接了上一镜时才加键** —— 无条件加会让所有老镜头误报。
    //
    // # 2026-09-27 改：**上一镜成片的时间不再进指纹**，只留「接不接、怎么接」
    //
    // 上面那条理由不成立：`<Video 1>` 的定义是代码写死的通用一句
    // （「同一场上一镜的结尾」，见 `prompt_route_b`），**不描述是哪一条片子**。
    // 上一镜出没出、重出几次，这一镜的提示词一个字都不变。
    //
    // 而把 take 时间放进来的后果是：**同一场设了接上一镜的镜头，批量出片永远只出得了第一镜** ——
    // 第一镜一出完，第二镜就被判「要重新合成」跳过（《回收师》E01 第一次按场出片撞上）。
    // 「上一镜重出之后后面要跟着重出」是**成片**层面的事，不该卡在合成这一步。
    //
    // 第二个元素留一个定值 0，数组形状不变：已经合成的镜头当时上一镜都还没出片、存的就是 0，
    // 这样它们的指纹不变，不会全部误报过期。
    if shot.effective_continuity().needs_prev_take() {
        fields["continuity"] = serde_json::json!([format!("{:?}", shot.effective_continuity()), 0]);
    }
    // **角色的 H3 身份音色也要进指纹。**
    //
    // 2026-09-20 查出来的，而且是同一个坑的第二次：`h3_voice` 09-19 加进来时
    // 同时喂给了线路 A（`prompt_route_a` 的嗓音描述）和线路 B
    // （`{SPEAKERS}` 里那段 DELIVERY），**却没进指纹**。
    // 后果和当初 `emotion` / `voice` 漏掉时一模一样：用户改了角色的嗓音，
    // 提示词不判过期，出片照用旧的那一份，而且不报错。
    //
    // 同样**只在真的有值时才加** —— 无条件加会让所有已合成的镜头一起误报。
    let voices: Vec<(&str, &str)> = used
        .iter()
        .filter_map(|id| assets.iter().find(|a| &a.id == id))
        .filter(|a| !a.h3_voice.trim().is_empty())
        .map(|a| (a.id.as_str(), a.h3_voice.as_str()))
        .collect();
    if !voices.is_empty() {
        fields["h3_voices"] = serde_json::json!(voices);
    }
    // **表演指示也要进指纹。** 2026-09-18 加 `emotion` / `voice` 时漏了这里 ——
    // 后果是用户改了语气，提示词**不判过期**，出片照用旧的那一份，而且不报错。
    // 界面上显示「已改」，出来的片子是旧的，从现象上完全查不回来。
    //
    // 和 `looks` 同样的办法：**只在真的有值时才加**。无条件加的话，
    // 已经合成过的镜头（E01/E02 那 15 句全空）指纹一起变、全部误报要重合成。
    let performance: Vec<(&str, &str, &str)> = shot
        .dialogue_lines
        .iter()
        .map(|l| (l.voice.as_str(), l.emotion.as_str(), l.voice_override.as_str()))
        .collect();
    if performance.iter().any(|(v, e, o)| !v.trim().is_empty() || !e.trim().is_empty() || !o.trim().is_empty()) {
        fields["performance"] = serde_json::json!(performance);
    }
    fields
}

/// 资产描述去掉「描述：」这类前缀，多行并成一行，截到 `max` 个字。
///
/// **按句截，不截在半句中间**：实跑里按字数硬截出过「服装：上衣白…」，模型读到的是半句话。
/// 超长时截到 `max` 以内最后一个句读（。；，）为止；一个句读都没有才硬截。
pub fn short_description(a: &AssetRef, max: usize) -> String {
    let d = a.description.trim();
    let d = d.strip_prefix("描述：").or_else(|| d.strip_prefix("描述:")).unwrap_or(d).trim();
    let joined = d
        .lines()
        .map(|l| l.trim().trim_end_matches(['。', '；', '，']))
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("；");
    if joined.chars().count() <= max {
        return joined;
    }
    let head: String = joined.chars().take(max).collect();
    match head.rfind(['。', '；', '，']) {
        Some(at) if at > 0 => head[..at].to_string(),
        _ => head,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 合成规则的版本进指纹（Codex 复查 §19）：规则升级，旧提示词判过期
    #[test]
    fn 合成规则版本进指纹() {
        let shot: crate::cinema::StoryboardShot = serde_json::from_value(serde_json::json!({
            "id": "s1", "shot_num": 1, "scale": "", "camera_movement": "", "dialogue": "", "prompt": "", "duration": 3.0, "status": "pending"
        })).unwrap();
        let ctx = ShotContext { shot: &shot, prev: None, assets: &[], style: "", long_take: false };
        assert_eq!(fields_json(&ctx)["compiler"], COMPILER_REV);
        let mut j = fields_json(&ctx);
        let before = fields_hash(&ctx);
        j["compiler"] = serde_json::json!("route-b-old");
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in j.to_string().bytes() { h ^= b as u64; h = h.wrapping_mul(0x0100_0000_01b3); }
        assert_ne!(format!("{h:016x}"), before, "换了版本号指纹要变");
    }

    /// 2026-10-07 EP03：星澜整集穿晚礼服但没选造型，镜头表写「不控制：晚礼服具体改制样式」——
    /// 以前这里把不控制删了、又补「服装颜色和款式」，等于让主形象的银链纱裙管服装
    #[test]
    fn 换了装没选造型时不反转服装控制() {
        let slot = RefSlot { order: 1, asset_id: "c".into(), usage: crate::shot_table::RefUsage::Identity, control: "外貌、长发".into(), not_control: "晚礼服具体改制样式、姿势".into() };
        let look = |id: &str| crate::storyboard_gen::LookRef { id: id.into(), name: id.into(), description: String::new(), has_image: true };
        let mut a = asset("c", AssetKind::Character, "星澜", true);
        a.looks = vec![look(crate::assets::MAIN_LOOK), look("look_gown")];
        // 主形象 + 另有造型 + 说了不控制服装：原样保留，并认出是换装没选造型
        assert!(outfit_mismatch(&slot, Some(&a)));
        let kept = with_outfit(slot.clone(), Some(&a));
        assert_eq!((kept.control.as_str(), kept.not_control.as_str()), ("外貌、长发", "晚礼服具体改制样式、姿势"));
        // 选了造型：图就是这一镜的衣服，照 09-14 的规矩让它管服装
        a.look_id = "look_gown".into();
        assert!(!outfit_mismatch(&slot, Some(&a)));
        let fixed = with_outfit(slot.clone(), Some(&a));
        assert!(fixed.control.contains("服装颜色和款式") && !fixed.not_control.contains("晚礼服"), "{fixed:?}");
        // 镜头详情里明确选了主形象（10-10 EP04 第 19 镜）：图就是这一镜的衣服，不算没选
        a.look_id = crate::assets::MAIN_LOOK.into();
        assert!(!outfit_mismatch(&slot, Some(&a)), "选了主形象还报没选造型");
        assert!(with_outfit(slot.clone(), Some(&a)).control.contains("服装颜色和款式"));
        // 没有别的造型（09-14 那种：镜头表误把服装写进不控制）：照旧去掉
        let mut b = asset("c", AssetKind::Character, "陆川", true);
        b.looks = vec![look(crate::assets::MAIN_LOOK)];
        assert!(with_outfit(slot, Some(&b)).control.contains("服装颜色和款式"));
    }

    /// 没用非主形象的镜头，指纹字段里不能多出 looks 键 —— 否则老项目所有镜头误报要重新合成
    #[test]
    fn 指纹只在用了形象时才多出_looks() {
        let shot: crate::cinema::StoryboardShot = serde_json::from_value(serde_json::json!({
            "id": "s1", "shot_num": 1, "scale": "", "camera_movement": "", "dialogue": "", "prompt": "", "duration": 3.0, "status": "pending", "character_ids": ["c"]
        })).unwrap();
        let mut a = asset("c", AssetKind::Character, "皇帝", true);
        let ctx = |a: &AssetRef| fields_json(&ShotContext { shot: &shot, prev: None, assets: std::slice::from_ref(a), style: "", long_take: false });
        assert!(ctx(&a).get("looks").is_none(), "没用形象不许有 looks 键");
        a.look_id = "look_1".into();
        assert_eq!(ctx(&a)["looks"], serde_json::json!([["c", "look_1"]]));
    }

    /// **勾上长镜头之后，这一场的提示词要标过期。**
    ///
    /// 长镜头窗口的正文里多一条「台词要在重叠区之前说完」
    /// （`prompt_route_b::assemble`）。不进指纹的话，勾上之后旧提示词
    /// 不判过期，出片用的还是没那条约束的版本 —— **而且不报错**。
    ///
    /// 这条是变异测试逼出来的：去掉指纹里那一行，当时没有任何测试变红。
    #[test]
    fn 长镜头窗口进指纹_不是的不多出键() {
        let s = shot();
        let plain = fields_json(&ShotContext { shot: &s, prev: None, assets: &[], style: "", long_take: false });
        assert!(plain.get("long_take").is_none(), "不是长镜头却多出了 long_take 键 —— 老分集会全部误报");

        let lt = fields_json(&ShotContext { shot: &s, prev: None, assets: &[], style: "", long_take: true });
        assert_eq!(lt["long_take"], serde_json::json!(true));
        assert_ne!(
            fields_hash(&ShotContext { shot: &s, prev: None, assets: &[], style: "", long_take: false }),
            fields_hash(&ShotContext { shot: &s, prev: None, assets: &[], style: "", long_take: true }),
            "勾上长镜头，指纹没变 —— 出片会用没有「台词要早点说完」那条约束的旧提示词"
        );
    }

    /// **上一镜出片 / 重出，接它的这一镜不算过期**（2026-09-27 改）。
    ///
    /// 以前上一镜一出片这一镜就标「要重新合成」—— 批量出片于是永远只出得了同一场的第一镜
    /// （《回收师》E01 第一次按场出片撞上：「出片 1/2 镜，第 2 镜合成之后改过……跳过」）。
    /// `<Video 1>` 那句是代码写死的通用定义，不描述哪一条片子，提示词不随上一镜变。
    ///
    /// 仍然钉住：**不接的镜头不许多出这个键**（老分集会全部误报），**换衔接档位要变**。
    #[test]
    fn 上一镜出片不让这一镜过期_换档位才变() {
        use crate::cinema::{Continuity, VideoTake};
        let mut prev = shot();
        prev.shot_num = 1;
        let mut cur = shot();
        cur.shot_num = 2;
        let ctx = |c: &StoryboardShot, p: &StoryboardShot| {
            fields_json(&ShotContext { shot: c, prev: Some(p), assets: &[], style: "", long_take: false })
        };

        // 不接：不许多出键（老分集一条都不能误报）
        assert!(ctx(&cur, &prev).get("continuity").is_none(), "不接的镜头多出了 continuity 键");

        cur.continuity = Continuity::RefVideo;
        let a = ctx(&cur, &prev);
        assert_eq!(a["continuity"], serde_json::json!(["RefVideo", 0]), "形状要和已合成的老镜头一样，否则全部误报过期");

        // 上一镜出片、重出：都不变
        prev.takes.push(VideoTake { path: "/1.mp4".into(), created_at: 100, ..Default::default() });
        assert_eq!(a["continuity"], ctx(&cur, &prev)["continuity"], "上一镜出片把这一镜弄过期了 —— 批量出片会卡在第二镜");
        prev.takes.push(VideoTake { path: "/2.mp4".into(), created_at: 200, ..Default::default() });
        assert_eq!(a["continuity"], ctx(&cur, &prev)["continuity"]);

        // 换一档要变
        cur.continuity = Continuity::FirstFrame;
        assert_ne!(a["continuity"], ctx(&cur, &prev)["continuity"], "换了衔接档位，指纹没变");
    }

    /// **角色的 H3 嗓音改了，指纹必须跟着变。**
    ///
    /// 它同时喂给线路 A 的嗓音描述和线路 B 的 `{SPEAKERS}` DELIVERY ——
    /// 不进指纹的话，用户改了嗓音、提示词不判过期，出片照用旧的那一份，
    /// **而且不报错**。2026-09-19 给 `emotion` / `voice` 修过同一个坑，
    /// 同一天加进来的 `h3_voice` 又漏了一次。
    #[test]
    fn 指纹跟着h3嗓音走_没设的不多出键() {
        let shot: crate::cinema::StoryboardShot = serde_json::from_value(serde_json::json!({
            "id": "s1", "shot_num": 1, "scale": "", "camera_movement": "", "dialogue": "", "prompt": "", "duration": 3.0, "status": "pending", "character_ids": ["c"]
        })).unwrap();
        let mut a = asset("c", AssetKind::Character, "皇帝", true);
        let ctx = |a: &AssetRef| fields_json(&ShotContext { shot: &shot, prev: None, assets: std::slice::from_ref(a), style: "", long_take: false });
        // 没设嗓音的老项目不许多出键，否则所有镜头一起误报要重新合成
        assert!(ctx(&a).get("h3_voices").is_none(), "没设嗓音不许有 h3_voices 键");
        a.h3_voice = "嗓音低沉沙哑的老年男声".into();
        assert_eq!(ctx(&a)["h3_voices"], serde_json::json!([["c", "嗓音低沉沙哑的老年男声"]]));
        // 换一把嗓子，指纹要变
        let h1 = fields_hash(&ShotContext { shot: &shot, prev: None, assets: std::slice::from_ref(&a), style: "", long_take: false });
        a.h3_voice = "清亮的少年音".into();
        let h2 = fields_hash(&ShotContext { shot: &shot, prev: None, assets: std::slice::from_ref(&a), style: "", long_take: false });
        assert_ne!(h1, h2, "改了嗓音指纹没变 —— 出片会静默用旧的");
    }


    fn shot() -> StoryboardShot {
        serde_json::from_value(serde_json::json!({
            "id": "s1", "shot_num": 1, "scale": "medium", "camera_movement": "static", "dialogue": "",
            "prompt": "", "duration": 8.0, "status": "pending"
        }))
        .unwrap()
    }

    fn 带台词(voice: &str, emotion: &str, over: &str) -> StoryboardShot {
        let mut s = shot();
        s.dialogue_lines = vec![crate::cinema::DialogueLine {
            speaker: "皇帝".into(),
            line: "斩。".into(),
            voice: voice.into(),
            emotion: emotion.into(),
            voice_override: over.into(),
            off_screen: false,
        }];
        s
    }
    fn 指纹(s: &StoryboardShot) -> String {
        fields_hash(&ShotContext { shot: s, prev: None, assets: &[], style: "", long_take: false })
    }

    /// **改了表演指示必须让提示词过期。**
    ///
    /// 2026-09-18 加 `emotion` / `voice` 时漏了指纹这一处：用户改了语气，
    /// 提示词不判过期，出片照用旧的 —— 界面显示「已改」，片子是旧的，
    /// 从现象上完全查不回来。2026-09-19 Codex Astra 的审查里点出来的。
    #[test]
    fn 改了语气音色和逐句覆盖都要重新合成() {
        let 原 = 带台词("", "", "");
        for 改过 in [带台词("沙哑", "", ""), 带台词("", "咬牙", ""), 带台词("", "", "shen-teen")] {
            assert_ne!(指纹(&原), 指纹(&改过), "改了表演指示却没判过期");
        }
        // 三项之间也要能互相区分，不是「有值就算一种」
        assert_ne!(指纹(&带台词("沙哑", "", "")), 指纹(&带台词("清亮", "", "")));
        assert_ne!(指纹(&带台词("", "咬牙", "")), 指纹(&带台词("", "带笑", "")));
    }

    /// **三项全空时不许多出 performance 键。**
    ///
    /// 无条件加的话，已经合成过的老镜头（E01/E02 那 15 句全空）指纹一起变、
    /// 全部误报「要重新合成」。和 `looks` 是同一个坑，2026-09-15 已经踩过一次。
    #[test]
    fn 没有表演指示时不多出键_老镜头不误报() {
        let ctx = |s: &StoryboardShot| fields_json(&ShotContext { shot: s, prev: None, assets: &[], style: "", long_take: false });
        assert!(ctx(&带台词("", "", "")).get("performance").is_none(), "全空还加键，老镜头会全部误报");
        assert!(ctx(&shot()).get("performance").is_none(), "没有台词更不该有");
        // 只有空白也算没有
        assert!(ctx(&带台词("  ", " ", "")).get("performance").is_none());
        assert!(ctx(&带台词("沙哑", "", "")).get("performance").is_some());
    }

    #[test]
    fn 指纹只跟合成会读的字段走() {
        let a = shot();
        let mut b = shot();
        b.status = "done".into();
        b.video_url = Some("x.mp4".into());
        assert_eq!(fields_hash(&ShotContext { shot: &a, prev: None, assets: &[], style: "", long_take: false }), fields_hash(&ShotContext { shot: &b, prev: None, assets: &[], style: "", long_take: false }), "出片状态变了不该要求重新合成");
        b.lighting = "冷蓝光".into();
        assert_ne!(fields_hash(&ShotContext { shot: &a, prev: None, assets: &[], style: "", long_take: false }), fields_hash(&ShotContext { shot: &b, prev: None, assets: &[], style: "", long_take: false }));
        let mut c = shot();
        c.duration = 9.0;
        assert_ne!(fields_hash(&ShotContext { shot: &a, prev: None, assets: &[], style: "", long_take: false }), fields_hash(&ShotContext { shot: &c, prev: None, assets: &[], style: "", long_take: false }));
        assert_eq!(fields_hash(&ShotContext { shot: &a, prev: None, assets: &[], style: "", long_take: false }), fields_hash(&ShotContext { shot: &shot(), prev: None, assets: &[], style: "", long_take: false }));
        assert_eq!(fields_hash(&ShotContext { shot: &a, prev: None, assets: &[], style: "", long_take: false }).len(), 16);
        assert_ne!(fields_hash(&ShotContext { shot: &a, prev: None, assets: &[], style: "", long_take: false }), fields_hash(&ShotContext { shot: &a, prev: None, assets: &[], style: "水墨国风", long_take: false }), "换了风格要重新合成");
    }

    fn asset(id: &str, kind: AssetKind, name: &str, has_image: bool) -> AssetRef {
        AssetRef { id: id.into(), kind, name: name.into(), has_image, description: format!("{name}的描述"), look_id: String::new(), looks: Vec::new(), h3_voice: String::new() }
    }

    /// 2026-10-06 EP03 第 3、5 镜：项目画风里写星澜的那一句，在没有星澜的镜头里把苏晴画成了机器人
    #[test]
    fn 画风里点名的角色不在这一镜_那句不发() {
        let assets = vec![asset("c1", AssetKind::Character, "星澜", true), asset("c2", AssetKind::Character, "苏晴", true)];
        let style = "半写实国漫3D CG。星澜保持精致非人的伴侣机器人气质，以珍珠白和青蓝能量核心突出。电影级物理灯光。";
        let mut s = shot();
        s.character_ids = vec!["c2".into()];
        let ctx = ShotContext { shot: &s, prev: None, assets: &assets, style, long_take: false };
        assert_eq!(ctx.style_for_shot(), "半写实国漫3D CG。电影级物理灯光。");
        // 星澜在这一镜：原样
        s.character_ids = vec!["c1".into(), "c2".into()];
        let ctx = ShotContext { shot: &s, prev: None, assets: &assets, style, long_take: false };
        assert_eq!(ctx.style_for_shot(), style);
        // 指纹照旧按原画风算 —— 已合成的提示词不会因为这条全部过期
        let a = fields_hash(&ctx);
        assert_eq!(a, fields_hash(&ShotContext { style, ..ctx }));
    }

    /// 用户要求的兜底：没图、库里没有的都要按文字写；补上图后要报「要重新合成」。
    #[test]
    fn 只挂有图的参考图_编号重排_其余按文字兜底() {
        use crate::shot_table::{RefUsage, SceneItem, UnlinkedAsset};
        let assets = vec![
            asset("c1", AssetKind::Character, "皇帝", false),
            asset("c2", AssetKind::Character, "林婉清", true),
            asset("s1", AssetKind::Scene, "金銮殿", true),
            asset("p1", AssetKind::Prop, "龙椅", false),
        ];
        let mut s = shot();
        s.character_ids = vec!["c1".into(), "c2".into()];
        s.scene_id = Some("s1".into());
        s.prop_ids = vec!["p1".into()];
        let slot = |order: u8, id: &str| RefSlot { order, asset_id: id.into(), usage: RefUsage::Identity, control: "脸".into(), not_control: String::new() };
        s.ref_slots = vec![slot(3, "s1"), slot(1, "c1"), slot(2, "c2")];
        s.unlinked_assets = vec![UnlinkedAsset { kind: "prop".into(), name: "玉笏".into() }];
        s.scene_items = vec![SceneItem { name: "玉笏".into(), detail: "白玉，大臣双手持在胸前".into() }];
        let ctx = ShotContext { shot: &s, prev: None, assets: &assets, style: "", long_take: false };
        assert_eq!(ctx.slots().iter().map(|(r, a)| (r.order, a.map(|a| a.name.as_str()))).collect::<Vec<_>>(), [(1, Some("林婉清")), (2, Some("金銮殿"))], "没图的皇帝不挂，后面的往前补");
        // 起始帧不是资产，照留
        let mut with_frame = s.clone();
        with_frame.ref_slots.push(RefSlot { order: 4, asset_id: "shot_frame".into(), usage: RefUsage::StartFrame, control: "开场构图".into(), not_control: String::new() });
        let ctx3 = ShotContext { shot: &with_frame, prev: None, assets: &assets, style: "", long_take: false };
        assert_eq!(ctx3.slots().iter().map(|(r, a)| (r.order, a.is_none())).collect::<Vec<_>>(), [(1, false), (2, false), (3, true)]);
        let text = ctx.text_only();
        assert_eq!(text.iter().map(|t| (t.name.as_str(), t.in_library)).collect::<Vec<_>>(), [("皇帝", true), ("龙椅", true), ("玉笏", false)]);
        assert_eq!(text[2].description, "白玉，大臣双手持在胸前");
        assert_eq!(text[0].description, "皇帝的描述");
        // 给皇帝补上图：指纹变，提示词要重新合成
        let before = fields_hash(&ShotContext { shot: &s, prev: None, assets: &assets, style: "", long_take: false });
        let mut after_assets = assets.clone();
        after_assets[0].has_image = true;
        assert_ne!(before, fields_hash(&ShotContext { shot: &s, prev: None, assets: &after_assets, style: "", long_take: false }));
        let ctx2 = ShotContext { shot: &s, prev: None, assets: &after_assets, style: "", long_take: false };
        assert_eq!(ctx2.slots().len(), 3);
        assert!(ctx2.text_only().iter().all(|t| t.name != "皇帝"));
    }

    /// 用户要求补的漏洞：改了上一镜的结尾 / 光影、改了资产描述，这一镜的提示词都要算过期。
    #[test]
    fn 上一镜和资产描述改了也算过期() {
        let assets = vec![asset("c1", AssetKind::Character, "皇帝", true)];
        let mut prev = shot();
        prev.scene_no = 1;
        prev.segments = vec![crate::shot_table::ShotSegment { start: 0.0, end: 8.0, visual: "皇帝抬手".into(), ..Default::default() }];
        prev.lighting = "暖光".into();
        let mut s = shot();
        s.scene_no = 1;
        s.character_ids = vec!["c1".into()];
        let h = |prev: &StoryboardShot, assets: &[AssetRef]| fields_hash(&ShotContext { shot: &s, prev: Some(prev), assets, style: "", long_take: false });
        let base = h(&prev, &assets);
        let mut p2 = prev.clone();
        p2.segments[0].visual = "皇帝起身".into();
        assert_ne!(base, h(&p2, &assets), "上一镜结尾改了");
        let mut p3 = prev.clone();
        p3.lighting = "冷光".into();
        assert_ne!(base, h(&p3, &assets), "同一场上一镜光影改了");
        let mut other_scene = p3.clone();
        other_scene.scene_no = 2;
        let mut other_scene_base = prev.clone();
        other_scene_base.scene_no = 2;
        assert_eq!(h(&other_scene_base, &assets), h(&other_scene, &assets), "换场了，上一镜光影不影响这一镜");
        let mut a2 = assets.clone();
        a2[0].description = "盛唐皇帝，龙袍".into();
        assert_ne!(base, h(&prev, &a2), "资产描述改了");
        let mut unrelated = assets.clone();
        unrelated.push(asset("x", AssetKind::Prop, "别的道具", true));
        assert_eq!(base, h(&prev, &unrelated), "没用到的资产改了不影响");
    }

    /// 实跑第 4 镜衣服变色：身份图的「不控制」里写了服装。
    #[test]
    fn 人物身份图必须管服装() {
        use crate::shot_table::RefUsage;
        let assets = vec![asset("c1", AssetKind::Character, "林婉清", true), asset("s1", AssetKind::Scene, "金銮殿", true)];
        let mut s = shot();
        s.character_ids = vec!["c1".into()];
        s.ref_slots = vec![
            RefSlot { order: 1, asset_id: "c1".into(), usage: RefUsage::Identity, control: "眼部特征、黑发".into(), not_control: "全身姿态、服装细节".into() },
            RefSlot { order: 2, asset_id: "s1".into(), usage: RefUsage::Geography, control: "立柱".into(), not_control: "服装".into() },
        ];
        let ctx = ShotContext { shot: &s, prev: None, assets: &assets, style: "", long_take: false };
        let slots = ctx.slots();
        assert_eq!((slots[0].0.control.as_str(), slots[0].0.not_control.as_str()), ("眼部特征、黑发、服装颜色和款式", "全身姿态"));
        assert_eq!(slots[1].0.not_control, "服装", "场景图不动");
        // 控制里已经写了服装的不重复补
        s.ref_slots[0].control = "瓜子脸、男装官服".into();
        let ctx = ShotContext { shot: &s, prev: None, assets: &assets, style: "", long_take: false };
        assert_eq!(ctx.slots()[0].0.control, "瓜子脸、男装官服");
    }

    #[test]
    fn 嗓音按资产描述的性别年龄() {
        let mut assets = vec![asset("c1", AssetKind::Character, "林婉清（盛唐）", true), asset("c2", AssetKind::Character, "大臣", true)];
        assets[0].description = "描述：盛唐女官，女性，28岁，身高165厘米".into();
        assets[1].description = "描述：盛唐大臣，男性，50岁".into();
        let s = shot();
        let ctx = ShotContext { shot: &s, prev: None, assets: &assets, style: "", long_take: false };
        assert_eq!(ctx.voice("林婉清（盛唐）"), Voice { female: Some(true), age: Some(28), h3: String::new() });
        assert_eq!(ctx.voice("大臣"), Voice { female: Some(false), age: Some(50), h3: String::new() });
        assert_eq!(ctx.voice("旁白"), Voice::default());
    }

    #[test]
    fn 同一场的上一镜才算延续() {
        let mut prev = shot();
        prev.scene_no = 2;
        let mut s = shot();
        s.scene_no = 2;
        let ctx = ShotContext { shot: &s, prev: Some(&prev), assets: &[], style: "", long_take: false };
        assert!(ctx.same_scene_prev().is_some());
        prev.scene_no = 1;
        let ctx = ShotContext { shot: &s, prev: Some(&prev), assets: &[], style: "", long_take: false };
        assert!(ctx.same_scene_prev().is_none());
    }

    /// H3 一次最多 9 张参考图：第 10 张起按文字写，不静默丢。
    #[test]
    fn 参考图最多九张_超出的按文字兜底() {
        use crate::shot_table::RefUsage;
        let assets: Vec<AssetRef> = (1..=11).map(|i| asset(&format!("c{i}"), AssetKind::Character, &format!("人{i}"), true)).collect();
        let mut s = shot();
        s.character_ids = assets.iter().map(|a| a.id.clone()).collect();
        s.ref_slots = (1..=11u8).map(|i| RefSlot { order: i, asset_id: format!("c{i}"), usage: RefUsage::Identity, control: "脸".into(), not_control: String::new() }).collect();
        let ctx = ShotContext { shot: &s, prev: None, assets: &assets, style: "", long_take: false };
        assert_eq!(ctx.slots().len(), 9);
        assert_eq!(ctx.text_only().iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["人10", "人11"]);
    }

    #[test]
    fn 描述去掉前缀并截短() {
        let a = AssetRef { id: "c".into(), kind: AssetKind::Character, name: "皇帝".into(), has_image: true, description: "描述：盛唐皇帝，男性，40岁".into(), look_id: String::new(), looks: Vec::new(), h3_voice: String::new() };
        assert_eq!(short_description(&a, 100), "盛唐皇帝，男性，40岁");
        assert_eq!(short_description(&a, 9), "盛唐皇帝，男性", "截在句读上，不留半句「4」");
        assert_eq!(short_description(&a, 3), "盛唐皇", "没有句读才硬截");
        let multi = AssetRef { description: "描述：盛唐女官，28岁。\n服装：上衣男装官服，下装官服下摆".into(), ..a };
        assert_eq!(short_description(&multi, 100), "盛唐女官，28岁；服装：上衣男装官服，下装官服下摆");
        assert_eq!(short_description(&multi, 20), "盛唐女官，28岁；服装：上衣男装官服");
    }
}
