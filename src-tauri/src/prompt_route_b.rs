//! 线路 B：H3 官方参考生视频**六段结构**。
//!
//! 结构照 2026-09-13 试片 B（用户看片通过，评价略好于 A）和 H3 官方 Full-reference 指南
//! （`skills/film-director/reference/h3-reference-mode.md`、drama-skills `minimax-h3.md`）：
//!
//! ```text
//! subject_definitions / summary / retention_analysis / detailed_description / overall_soundscape / non_diegetic_music
//! ```
//!
//! **分工**：段名、段序、`<Subject N>` / `<Picture N>` 编号、保留强度、`non_diegetic_music: N/A`
//! 全由代码按参考图槽位定；LM（`PromptSlot::RouteBWriter`）只写英文措辞 ——
//! 主体外貌、控制范围的英文、summary、detailed_description、声音、静帧提示词。
//! 这样编号永远和出片时挂图的顺序对得上（编号错了模型收到互相矛盾的说明，接口不报错）。
//!
//! **按生产方式用对应的官方规范**（2026-10-06，见 [`crate::h3_spec`]）：
//! 纯文生 = T2VA、首帧 = I2VA、首尾帧 = FL2VA，都是官方基础模式的三字段
//! （`integrated_multimodal_description` / `overall_soundscape` / `non_diegetic_music`，首帧两种前面多一行对齐指令，
//! **不用 `<Subject N>`**）；挂资产参考图 / 接上一镜 / 长镜头窗口 = Ref2VA 六段。
//! 以前首帧出片也写成六段 + `<Subject N>`，两种格式混用，不在官方的格式里（`docs/reports/2026-10-06-H3提示词规范核查.md`）。
//!
//! **物理隔离**：不 import 线路 A，不用任何共用的拼装函数（有测试钉着）。输入只读 [`ShotContext`]。

use crate::error::{Error, Result};
use crate::prompt_input::{fields_hash, short_description, ShotContext};
use crate::h3_spec::{last_shot_number, official_timing, parse_clock, screen_quotes};
use crate::shot_table::{RefUsage, RoutePrompts};
use serde::Deserialize;
use std::future::Future;

/// 一张参考图在正文里的标签。
#[derive(Debug, Clone, PartialEq)]
struct Tag {
    /// `<Picture N>` 的 N，就是槽位编号 = 出片时挂图的顺序
    picture: u8,
    /// `<Subject N>` 的 N。起始帧 / 结束帧 / 构图图只当帧锚点，不定义主体（官方指南：图本身是帧锚点才单开 Picture）
    subject: Option<u32>,
    usage: RefUsage,
    /// 人 / 地点 / 物件。标签表里要写明 —— 实跑第 11 镜只挂了场景图，模型把 <Subject 1> 当成了女主
    kind: Option<crate::assets::AssetKind>,
    name: String,
    description: String,
    control: String,
    not_control: String,
}

fn tags(ctx: &ShotContext) -> Vec<Tag> {
    let mut next_subject = 0;
    let out: Vec<Tag> = ctx.slots()
        .into_iter()
        .map(|(slot, asset)| {
            let frame = matches!(slot.usage, RefUsage::StartFrame | RefUsage::EndFrame | RefUsage::Composition);
            let subject = (!frame).then(|| {
                next_subject += 1;
                next_subject
            });
            Tag {
                picture: slot.order,
                subject,
                usage: slot.usage,
                kind: asset.map(|a| a.kind),
                name: asset.map_or_else(|| slot.asset_id.clone(), |a| a.name.clone()),
                description: asset.map(|a| short_description(a, CHAR_DESC_MAX)).unwrap_or_default(),
                control: slot.control.trim().to_string(),
                not_control: slot.not_control.trim().to_string(),
            }
        })
        .collect();
    // **首帧出片不再把首帧里的人定义成 `<Subject N>`**（2026-10-06 改）：官方 I2VA / FL2VA 是三字段，
    // 不用主体标签，人按「the young woman shown in <Picture 1>」这样写（官方 Case 2）。
    // 以前那套（2026-09 起「首帧里认出来的主体」）把参考模式的六段套在首帧出片上，两种格式混用。
    // 当时要防的事（文字和首帧打架时文字赢、首帧里没有的人被随便指一个）照旧在 `tag_table` / `shot_brief` 里管。
    out
}

/// 正文里每个 `<Subject N>` 是谁：`(N, 资产名, 资产描述)`。编号和合成时给写手的标签表是同一套。
///
/// 出首帧用：`<Subject N>` 是 H3 的写法，出图模型（Qwen 2.1）不认，见 `cinema::resolve_subject_tags`。
pub(crate) fn subject_names(ctx: &ShotContext) -> Vec<(u32, String, String)> {
    tags(ctx)
        .into_iter()
        .filter_map(|t| {
            let n = t.subject?;
            // 只给没挂上图的人用（群演多、槽位满了），一句认得出是谁就够 —— 四个打手各 200 字会把正文淹掉
            let description = ctx.assets.iter().find(|a| a.name == t.name).map(|a| short_description(a, FRAME_DESC_MAX)).unwrap_or_default();
            Some((n, t.name, description))
        })
        .collect()
}

/// 首帧出片的正文**开头**（`[Shot 1]` 到第一个不是 00:00 的时间码）里写的景别 / 机位词。
///
/// 「推到 / 落成 / 变成 …medium shot」是终点，不算；只抓当成开场写的。
fn opening_framing_words(desc: &str) -> Vec<&'static str> {
    const WORDS: [&str; 16] = [
        "extreme close-up", "close-up", "close up", "medium close-up", "medium shot", "wide shot", "full shot", "full-body shot",
        "long shot", "establishing shot", "low-angle", "low angle", "high-angle", "high angle", "bird's-eye", "eye-level",
    ];
    let Some(start) = desc.find("[Shot 1]") else { return Vec::new() };
    let body = &desc[start..];
    let end = timecode_positions(body).into_iter().find(|(_, v)| *v > 0.05).map_or(body.len(), |(i, _)| i);
    let opening = body[..end].to_lowercase();
    let mut out: Vec<&'static str> = Vec::new();
    for w in WORDS {
        for (i, _) in opening.match_indices(w) {
            let mut from = i.saturating_sub(24);
            while !opening.is_char_boundary(from) {
                from -= 1;
            }
            let before = &opening[from..i];
            let is_end = ["into ", " to ", "becomes", "settl", "ends", "until", "toward"].iter().any(|k| before.contains(k));
            if !is_end && !out.iter().any(|o| o.contains(w) || w.contains(o)) {
                out.push(w);
            }
        }
    }
    out
}

/// 官方保留强度。按用途定：身份、造型要全保留；地理只保空间关系；风格、特效是属性迁移。
fn retention(u: RefUsage) -> &'static str {
    match u {
        RefUsage::Identity | RefUsage::Outfit | RefUsage::StartFrame | RefUsage::EndFrame | RefUsage::Composition => "fully_preserved",
        RefUsage::Geography | RefUsage::Scale => "partially_preserved",
        RefUsage::Effect | RefUsage::Style => "attribute_transfer",
    }
}

fn usage_en(u: RefUsage) -> &'static str {
    match u {
        RefUsage::Identity => "identity",
        RefUsage::Outfit => "outfit and wear",
        RefUsage::Geography => "spatial layout",
        RefUsage::Composition => "composition",
        RefUsage::Scale => "scale",
        RefUsage::Effect => "visual effect",
        RefUsage::StartFrame => "opening frame",
        RefUsage::EndFrame => "ending frame",
        RefUsage::Style => "visual style",
    }
}

/// 说话人 → `(Sx)`，按台词里第一次出现的顺序编号；是参考主体的同时给出 `<Subject N>`。
///
/// **按人去重 —— 所以这里不能放任何「这一句」的属性。** 编号是按人的（陆川两句都是 S1），
/// 但「这句是不是画外音」是按句的。2026-10-01《回收师》EP01 第 7 镜：陆川先出镜说
/// 「聊不出钱，聊得出感情」，再内心独白「两万四……这辈子拆不完」。原来这里写的是
/// `(Some(n), true) => "<Subject n> (Sn), off-screen"` —— 去重只留第一句，第二句就借用了
/// 第一句的「出镜」绑定，画外标记静默丢失，H3 给内心独白对上了口型，不报错。
/// 画外音现在在 [`line_markers`] 里逐句决定（动词 + 嘴唇不动）。
///
/// **没有主体标签的说话人**（官方基础模式全都是；参考模式里没挂图的人）按官方「a stable voice description followed by (Sx)」写：
/// 画面里的人 `the 23-year-old woman (S1)`，首帧出片时 `the 23-year-old woman shown in <Picture 1> (S1)`，
/// 不在画面里的 `an off-screen 40-year-old man's voice (S2)`。
/// 以前一律写成 `(S1), an off-screen voice` —— 纯文生的镜头里**每个说话的人都被标成了画外音**。
///
/// 回 `(名字, 写法, 画面里看得见)`：看得见的人说画外音那句才要写「嘴唇不动」。
fn speakers(ctx: &ShotContext, tags: &[Tag]) -> Vec<(String, String, bool)> {
    let mut out: Vec<(String, String, bool)> = Vec::new();
    for l in &ctx.shot.dialogue_lines {
        let name = l.speaker.trim();
        if l.line.trim().is_empty() || out.iter().any(|(n, _, _)| n == name) {
            continue;
        }
        let s = format!("(S{})", out.len() + 1);
        let tag = tags.iter().find(|t| t.name == name && t.subject.is_some()).and_then(|t| t.subject);
        let (how, visible) = match tag {
            Some(n) => (format!("<Subject {n}> {s}"), true),
            None => {
                let v = ctx.voice(name);
                let who = match (v.female, v.age) {
                    (Some(f), Some(age)) => format!("{age}-year-old {}", if f { "woman" } else { "man" }),
                    (Some(f), None) => (if f { "woman" } else { "man" }).to_string(),
                    _ => "speaker".to_string(),
                };
                let cast = ctx.characters().into_iter().find(|c| c.name == name);
                let in_shot = cast.is_some() && !ctx.shot.dialogue_lines.iter().filter(|x| x.speaker.trim() == name && !x.line.trim().is_empty()).all(|x| x.off_screen);
                if ctx.keyframe_video() {
                    match ctx.shot.absent_from_opening(name) {
                        None if cast.is_some() => (format!("the {who} shown in <Picture 1> {s}"), true),
                        Some(crate::cinema::OpeningAbsence::EntersLater) => (format!("the {who} who enters later {s}"), true),
                        _ => (format!("an off-screen {who}'s voice {s}"), false),
                    }
                } else if in_shot {
                    (format!("the {who} {s}"), true)
                } else {
                    (format!("an off-screen {who}'s voice {s}"), false)
                }
            }
        };
        out.push((name.to_string(), how, visible));
    }
    out
}

/// 表演指示的占位。模型把它换成英文措辞，**别的一个字都不许动**。
///
/// # 为什么是占位，不是让模型自由发挥
///
/// `<Subject N> (SN)` 这套绑定错一个字，模型收到的就是互相矛盾的说明，
/// 而接口不报错（见 `speakers` 的注释）。所以整句仍然是「照抄」，
/// 只在中间开一个洞让它翻译 —— 代码保住结构，模型只干它擅长的那部分。
///
/// # 模型忘了填也不失败
///
/// 没换掉的话 [`fill_lines`] 会把占位连同前面的逗号一起抹掉，退回到
/// 只有性别年龄的老样子。为此**不**写进 `check_wording` ——
/// 那里失败三次会让整镜报错，为一句措辞赔上一镜不值。
const DELIVERY: &str = "<DELIVERY>";

/// 人物描述给写手看多少字。
///
/// 原来是 120：《回收师》星澜的描述 198 字，截到 120 正好丢掉「胸口的椭圆形协议铭牌、
/// 锁骨指节手腕的银色接缝」，陆川丢掉「护目镜挂在额头、叼着没点的烟、机油渍」——
/// 两个主角最有辨识度的细节都在服装那半句里，而这条线路**不读色值锁**，文字锚点只有这一段
/// （2026-09-26）。参考图是整张设定卡，小物件模型不一定抓得到。
const CHAR_DESC_MAX: usize = 200;
/// 道具同理：60 字只够写个名字加材质。
const PROP_DESC_MAX: usize = 100;

/// 出首帧时没挂上图的主体，正文里补多少字描述（见 [`subject_names`]）。
const FRAME_DESC_MAX: usize = 60;

/// 词数超出上限多少以内算软问题（照样反馈让它删，重试用完交得出去）。见 `check_wording` 里词数那段
const WORDS_SOFT_OVER: f32 = 1.10;

/// 合成没通过时最多重试几次（不含第一次）。用户定的：最多三次。
pub const RETRIES: usize = 3;

/// 长镜头窗口的末尾留多少秒不说话。
///
/// # 为什么要留
///
/// 窗口之间有一段重叠（推荐 22 帧 ≈ 0.92 秒），发布时会被裁掉 /
/// 被下一个窗口重采。台词正好说到那里的话，半个字卡在裁切点上，
/// 结果不可预测（咔哒、重复、或者吞字）。
///
/// 2 秒 = 重叠 0.92 秒再加一倍余量。**这是估计不是实测** ——
/// 第一次真跑长镜头之后按接点的实际表现调。
pub const LONG_TAKE_TAIL_SECS: f32 = 2.0;

/// 一句台词在正文里的占位。**台词由代码填，不让模型抄**。
///
/// 2026-09-13 实跑：第 9、12 镜模型两次都把台词整句漏掉，正文里一个 `<d>` 都没有；
/// 重写也常常重复同一个错。台词本来就是镜头表里的字段、要逐字发，没有理由交给模型转写。
/// 模型只在说话的时刻写 `<Subject 1> (S1) says, [LINE 1]`，代码把 `[LINE 1]` 换成 `<d>[Chinese] 原话</d>`；
/// 模型漏写了占位，代码把整句补在 `Sound:` 之前。
struct LineMarker {
    marker: String,
    /// 说话人写法：`<Subject 1> (S1)` / `(S2), an off-screen voice`。
    /// 有表演指示时**里面带一个 `<DELIVERY>` 占位**，由模型译成英文填掉。
    who: String,
    line: String,
    /// 这一句怎么说，**中文**（镜头表的 `emotion` + `voice`）。空 = 没有指示。
    ///
    /// 本段正文强制英文（`check_wording` 会拒中文），所以这句不能直接写进去，
    /// 只能给模型当翻译源 —— 这也是它和线路 A 唯一的处理差别。
    tone: String,
    /// 角色资产上的身份音色（中文，整集一份）。和 `delta` 分开翻：固定的那一半每一镜措辞都一样。
    timbre: String,
    /// 这一句的变化：逐句 `voice` + `emotion`（中文）。
    delta: String,
    /// 说话的动词。画面里的人只出声不动嘴（画外音 / 内心独白）时是
    /// `says in an off-screen voiceover:`，其余 `says,`。
    verb: &'static str,
    /// 紧跟在 `</d>` 后面的「嘴唇不动」，只给画外音那句用；其余为空。
    lips: String,
}

/// 普通说话。
const SAYS: &str = "says,";
/// **画外音的固定说法**（`h3-prompt-format.md`「旁白用固定说法」）：
/// `The man (S1) says in an off-screen voiceover: <d>…</d> while his lips remain completely closed.`
const SAYS_VOICEOVER: &str = "says in an off-screen voiceover:";

fn line_markers(ctx: &ShotContext, tags: &[Tag]) -> Vec<LineMarker> {
    let speakers = speakers(ctx, tags);
    ctx.shot
        .dialogue_lines
        .iter()
        .filter(|l| !l.line.trim().is_empty())
        .enumerate()
        .map(|(i, l)| {
            let (how, visible) = speakers.iter().find(|(n, _, _)| n == l.speaker.trim()).map(|(_, how, vis)| (how.clone(), *vis)).unwrap_or_default();
            // 嗓音写在 <d> 外（官方指南）。实跑第 4 镜两个人说话，只有 (S1)(S2)，女主变成了男声
            let v = ctx.voice(&l.speaker);
            // 有主体标签的才补性别年龄；没标签的写法本身就带了（`the 23-year-old woman (S1)`）
            let voice = match (v.female, v.age) {
                _ if !how.starts_with("<Subject ") => String::new(),
                (Some(f), Some(age)) => format!(" in a {age}-year-old {}'s voice", if f { "woman" } else { "man" }),
                (Some(f), None) => format!(" in a {}'s voice", if f { "woman" } else { "man" }),
                _ => String::new(),
            };
            // **表演指示**。只给性别年龄的话 H3 没有任何方向，念出来是播音腔
            // （2026-09-18 用户在成片里听出来的）。官方指南本来就要求写
            // 音高 / 音色 / 语速 / 口音，我们一直只发了性别和年龄。
            // **角色资产上的声音设定排在最前**：它整集一份、跨场不变，
            // 而逐句的 `voice` / `emotion` 只写这一句的变化（醉意、压低、气音）。
            // 都塞进同一个翻译源，模型一次译成一句英文措辞。
            let tone = [v.h3.trim(), l.voice.trim(), l.emotion.trim()]
                .iter()
                .filter(|t| !t.is_empty())
                .copied()
                .collect::<Vec<_>>()
                .join("，");
            // **台词原文也交给翻译**（2026-10-11 用户：「四维配音法」里的停顿、重音，「直接把 3、4 补进来」）：
            // 只有情绪描述的话，翻译看不到这句话本身，没法判断重音落在哪、哪里停一拍。
            // 原文只当判断依据，译出来的是按位置描述的英文（「stressing the last word」），不引中文 —— 中文会被 H3 当台词念
            // **没写情绪的句子不加任何语气**（用户：「对话也不是每句都要带情绪」「不是一定要全补充」）——
            // 只带身份音色；交去翻译的话模型会自己编一个语气出来
            let line = l.line.trim();
            let who = if tone.is_empty() { format!("{how}{voice}") } else { format!("{how}{voice}, {DELIVERY}") };
            let feel = [l.voice.trim(), l.emotion.trim()].iter().filter(|t| !t.is_empty()).copied().collect::<Vec<_>>().join("，");
            let delta = if feel.is_empty() || line.is_empty() { feel } else { format!("{feel}｜台词：{line}") };
            // **画外音按这一句自己的标记定，不按人。** 同一个人一镜里可以先出镜说、再内心独白。
            //
            // 只在说话人画面里看得见时才写「嘴唇不动」—— 那正是会被错配口型的那个人。
            // 画面里没有的（`an off-screen …'s voice (S2)`，纯旁白），写「his lips」反而指向一个不存在的人。
            let (verb, lips) = if l.off_screen && visible {
                let whose = match v.female {
                    Some(true) => "her",
                    Some(false) => "his",
                    None => "their",
                };
                (SAYS_VOICEOVER, format!(" while {whose} lips remain completely closed"))
            } else {
                (SAYS, String::new())
            };
            LineMarker { marker: format!("[LINE {}]", i + 1), who, line: l.line.trim().to_string(), tone, timbre: v.h3.trim().to_string(), delta, verb, lips }
        })
        .collect()
}

/// 给写手的「每句台词在哪儿、怎么写」。**动词按句**：画外音那句是
/// [`SAYS_VOICEOVER`]，其余 [`SAYS`]（见 [`line_markers`]）。
fn speakers_instruction(markers: &[LineMarker]) -> String {
    if markers.is_empty() {
        return "Nobody speaks.".into();
    }
    markers
        .iter()
        .map(|m| {
            let base = format!("{}: at the moment it is spoken, write exactly \"{} {} {}\"", m.marker, m.who, m.verb, m.marker);
            if m.tone.is_empty() || !m.who.contains(DELIVERY) {
                base
            } else {
                // 中文只作为翻译源出现在指令里，正文里不许留（check_wording 会拒）
                format!("{base} — replace {DELIVERY} with an English delivery phrase of 6-16 words meaning 「{}」: begin with this character's fixed voice (pitch, timbre, accent — use the same words every time this character speaks), then this line's volume, pace and emotion. Translate it; never copy the Chinese. Change nothing else in the sentence.", m.tone)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 把占位换成逐字台词；漏写的补在 `Sound:` 之前（没有 `Sound:` 就补在末尾）。
fn fill_lines(desc: &str, markers: &[LineMarker]) -> String {
    let mut out = desc.to_string();
    let mut missing = Vec::new();
    for m in markers {
        // 「嘴唇不动」**由代码紧跟在 `</d>` 后面写**，不交给模型：规范要求紧接着写，
        // 而模型会自己另起一句（实跑写出过 `This is internal offscreen speech…`），位置和措辞都不稳。
        let d = format!("<d>[Chinese] {}</d>{}", m.line, m.lips);
        if let Some(at) = out.find(&m.marker) {
            // **「谁、怎么说」也由代码写死。** 写手被要求照抄 `who says,`，但它会偷懒成 `(S1) says,` ——
            // 2026-10-11 EP05 重合成后第 10~15 镜台词前只剩 `(S1) says,`，翻好的音色 / 语气 / 重音 / 停顿整段丢了，没有任何检查拦。
            // 找到台词前最近的那个说话人标记（`(S1)`），从它到台词之间换成代码这一份
            // 翻译失败（`who` 里还留着占位）时语气只有写手自己写的那份 —— 不覆盖
            let ours = !m.who.contains(DELIVERY);
            // 往回找的标记和台词之间**不能隔着别的台词**：同一个人一镜说两句、第二句写手没写标记时，
            // 往回会找到第一句的 `(S1)`，把第一句整段盖掉（测试「剧本里真的重复两遍」抓到的）
            let clean = |i: usize| at - i < 600 && !out[i..at].contains("</d>") && !out[i..at].contains("[LINE");
            if let Some(tag_at) = speaker_tag(&m.who).filter(|_| ours).and_then(|tag| out[..at].rfind(&tag).filter(|&i| clean(i))) {
                let tail = &m.who[m.who.find(&speaker_tag(&m.who).unwrap()).unwrap()..];
                out = format!("{}{tail} {} {}", &out[..tag_at], m.verb, &out[at..]);
            }
            out = out.replacen(&m.marker, &d, 1).replace(&m.marker, "");
        } else {
            missing.push(format!("{} {} {d}.", m.who, m.verb));
        }
    }
    if missing.is_empty() {
        return strip_delivery(&out);
    }
    let add = missing.join(" ");
    let out = match out.rfind("Sound:") {
        Some(at) => format!("{} {add} {}", out[..at].trim_end(), &out[at..]),
        None => format!("{} {add}", out.trim_end()),
    };
    strip_delivery(&out)
}

/// 说话人写法里的 `(S1)` 那一段（没有就 None）
fn speaker_tag(who: &str) -> Option<String> {
    let i = who.find("(S")?;
    let j = who[i..].find(')')? + i;
    who[i + 2..j].chars().all(|c| c.is_ascii_digit()).then(|| who[i..=j].to_string())
}

/// 已经翻过的中文 → 英文。**进程内共用** —— 同一个角色的身份音色每一镜都拿到同一串英文，
/// 不会第 3 镜「low husky」、第 9 镜「deep raspy」。温度是 0，跨次启动译出来也基本一样。
fn voice_cache() -> &'static std::sync::Mutex<std::collections::HashMap<String, String>> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, String>>> = std::sync::OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// 解析语气翻译的结果：`{"items": [...]}`，条数必须对得上，每条是不带中文的英文短语。对不上就 `None`。
pub fn parse_voice_directions(raw: &str, expect: usize) -> Option<Vec<String>> {
    let json = crate::prompts::extract_json(raw)?;
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let items: Vec<String> = v.get("items")?.as_array()?.iter().map(|x| x.as_str().unwrap_or("").trim().trim_end_matches(['.', ',']).to_string()).collect();
    let ok = items.len() == expect
        && items.iter().all(|t| {
            let words = t.split_whitespace().count();
            // 上限放宽到 32：实跑星澜的身份音色译成了 25 个词，内容全对，卡 18 就整份作废、退回老路（2026-09-27）
            (1..=32).contains(&words) && !t.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)) && !t.contains(DELIVERY)
        });
    ok.then_some(items)
}

/// **合成之前先把每句的语气翻好，代码直接拼进台词前面**（见 [`crate::prompts::PromptSlot::VoiceDirection`]）。
///
/// 固定音色和这一句的变化分开翻：音色走缓存，同一角色整集一串措辞。
/// 翻译失败就什么都不改 —— 标记里还留着 `<DELIVERY>`，退回原来的「写手顺手翻」。
async fn fill_delivery<F, Fut>(markers: &mut [LineMarker], call: &F)
where
    F: Fn(crate::prompts::PromptSlot, Vec<(String, String)>) -> Fut,
    Fut: Future<Output = Result<String>>,
{
    let mut todo: Vec<String> = Vec::new();
    {
        let cache = voice_cache().lock().unwrap();
        for m in markers.iter() {
            for t in [&m.timbre, &m.delta] {
                if !t.is_empty() && !cache.contains_key(t) && !todo.contains(t) {
                    todo.push(t.clone());
                }
            }
        }
    }
    if !todo.is_empty() {
        let items = todo.iter().enumerate().map(|(i, t)| format!("{}. {t}", i + 1)).collect::<Vec<_>>().join("\n");
        // 不合格（条数不对、超长、夹了中文）再试一次；两次都不行退回老路，**原始回复写进日志**
        // —— 10-11 EP05 两镜退回，日志只有一句「没拿到合格结果」，查不出是哪条不合格
        let mut got = None;
        for attempt in 1..=2 {
            match call(crate::prompts::PromptSlot::VoiceDirection, vec![("ITEMS".into(), items.clone())]).await {
                Ok(raw) => match parse_voice_directions(&raw, todo.len()) {
                    Some(v) => {
                        got = Some(v);
                        break;
                    }
                    None => tracing::warn!(attempt, expect = todo.len(), raw = %raw.chars().take(1500).collect::<String>(), "台词语气翻译不合格"),
                },
                Err(e) => tracing::warn!(attempt, error = %e, "台词语气翻译调用失败"),
            }
        }
        match got {
            Some(en) => {
                let mut cache = voice_cache().lock().unwrap();
                for (zh, en) in todo.into_iter().zip(en) {
                    cache.insert(zh, en);
                }
            }
            None => {
                tracing::warn!("台词语气翻译没拿到合格结果，这一镜退回让写手顺手翻");
                return;
            }
        }
    }
    let cache = voice_cache().lock().unwrap();
    for m in markers.iter_mut() {
        if !m.who.contains(DELIVERY) {
            continue;
        }
        let phrase = [&m.timbre, &m.delta]
            .iter()
            .filter(|t| !t.is_empty())
            .filter_map(|t| cache.get(*t).cloned())
            .collect::<Vec<_>>()
            .join(", ");
        if !phrase.is_empty() {
            m.who = m.who.replace(DELIVERY, &phrase);
        }
    }
}

/// 模型没换掉的 [`DELIVERY`] 占位连同前面的逗号一起抹掉。
///
/// **漏一个就会被 H3 当成台词的一部分念出来**，或者至少作为噪声进编码器。
/// 兜底路径（代码自己补的那句）里也有占位，所以这一步必须在最后统一做。
fn strip_delivery(text: &str) -> String {
    if !text.contains(DELIVERY) {
        return text.to_string();
    }
    text.replace(&format!(", {DELIVERY}"), "").replace(DELIVERY, "")
}

/// 画风是不是真人写实。**按风格词判断，不按预设 id** —— 自定义风格词也要认得出来，
/// 而提示词这一层拿不到预设 id（`ShotContext` 只有风格正文）。
///
/// 动画词优先：「cinematic」在动画风格里也常写（「cinematic lighting」），
/// 只看写实词会把所有带电影感的动画都判成真人。
pub fn is_live_action(style: &str) -> bool {
    let s = style.to_lowercase();
    const ANIMATED: &[&str] = &["anime", "cel-shaded", "cel shading", "animated", "animation", "cartoon", "illustration", "graphic novel", "ink wash", "stop-motion", "stop motion", "3d animat", "painterly", "动画", "动漫", "卡通", "插画", "水墨", "赛璐璐", "定格"];
    if ANIMATED.iter().any(|k| s.contains(k)) {
        return false;
    }
    // 「cinema」「film」放在这里是安全的：上面的动画词先过一遍，
    // 「animated film look」「cinematic lighting」那些已经被挡掉了
    const REAL: &[&str] = &["photoreal", "photo-real", "live-action", "live action", "film still", "film look", "photography", "realistic human", "real human skin", "cinematic realism", "noir", "cinema", "写实", "真人", "实拍", "照片"];
    REAL.iter().any(|k| s.contains(k))
}

/// 远景、大远景里人的脸只有二三十像素，模型内部只占两三格，**画不出五官、逐帧乱猜**，脸会扭。
/// 超分救不了（放大的是已经歪了的脸）。
///
/// # 写实和动画不一样（2026-09-15，用户在别人的片子上看出来的）
///
/// **3D 卡通 / 动画的脸远距离基本不坏**（五官是几块色块，本来就不靠细节），动画片里远景照样给正脸。
/// 真人和类真人写实才严重扭。所以：
///
/// - **写实**：既不给正脸（背影、侧影、剪影、低头、遮挡），又压景深 —— 行业里的常规做法是远处虚化、
///   要脸就切近景特写。虚化一层还能把「没画准的五官」直接糊掉。
/// - **动画 / 卡通**：允许给正脸，只要求表情做大做简单（远处的细微表情看不见），不强行背对镜头 ——
///   一刀切会白白牺牲这一镜的表演。
fn distant_face_rule(scale: &str, style: &str) -> Option<&'static str> {
    if !matches!(scale, "wide" | "extreme_wide") {
        return None;
    }
    Some(if is_live_action(style) {
        "Distance rule (photoreal faces are unreadable at this distance and will distort): stage every character with the face NOT turned toward the camera — seen from behind, in profile, silhouetted against light, head lowered, or partly hidden by hat, hair, sleeves or foreground objects, or in motion. Identify them by costume color, hairstyle and silhouette. Shoot it with a shallow depth of field: distant figures sit slightly soft / out of focus while a foreground or mid-ground element holds the sharp focus. Do not describe facial features or facial expressions in this shot; express emotion through posture and body movement."
    } else {
        "Distance rule (stylized faces survive this distance, but fine expression does not): faces may be turned toward the camera, but keep every expression broad and simple — no subtle micro-expression, no detailed facial description. Read emotion mainly from posture, gesture and silhouette; keep characters recognizable by costume color, hairstyle and silhouette."
    })
}

/// 给 LM 看的这一镜（中文字段原样给，由它翻成英文措辞）。
/// 一镜出的镜头（[`crate::one_take`]）给写手的那一条。**放在镜头说明的第一行**。
///
/// 模板里写着「one continuous take, no cuts」和「never write cuts or [Shot 2]」—— 那是按一镜一个机位写的。
/// 模板是用户能改的（设置 → 提示词），不能在代码里改它的原文，所以在这里**明说这一条盖过那两句**。
///
/// **切镜按官方写**（2026-10-06 改）：`[Shot 2] At 00:03.500, the camera cuts to …`（基础指南 4.2）。
/// 以前是同一段里的「At 00:SS.S, hard cut to …」—— Kotte 方案 D 两次打样这么写 H3 也切了，但不是官方格式；
/// 写手要是照旧这么写，拼装时由代码换成官方写法（[`official_timing`]）。
pub const ONE_TAKE_NOTE: &str = "ONE-TAKE MONTAGE — this overrides \"one continuous take, no cuts\" and \"never write cuts\" above: \
this single generation contains hard cuts at the times given in the segments below. Write each cut the official way: \
start a new shot `[Shot 2] At 00:03.500, the camera cuts to` + the new shot size and who or what it shows (strictly increasing times, three decimals), then continue. \
The same place, the same people in the same clothes, the same light and the same exposure, contrast and colour grade carry across every cut; things stay where they were.";

fn shot_brief(ctx: &ShotContext) -> String {
    let s = ctx.shot;
    let mut lines = Vec::new();
    if s.one_take {
        lines.push(ONE_TAKE_NOTE.to_string());
    }
    let style = ctx.style_for_shot();
    if !style.is_empty() {
        lines.push(format!("Visual style chosen for the whole project (translate it; the style sentences of detailed_description and the frame prompt must follow it): {style}"));
    }
    let vocab = |g: &str, id: &str| crate::shot_language::term_in(g, id).map(|t| t.prompt.en.clone()).unwrap_or_default();
    if ctx.keyframe_video() {
        // **首帧出片：开场构图就是首帧，不把景别 / 机位当成开场写。** 写手看不到图，镜头表的景别是打算，首帧不一定照着出
        // （2026-10-04 EP02 第 2 镜：表上「机械靴低位近景起拍上摇」，首帧是全身背影；正文照表写开场，H3 边动边重画房间，运镜卡顿）
        lines.push(format!(
            "Camera movement: {}. Intended framing (for direction only — do NOT write it as the opening; the opening is <Picture 1>): {}; {}. Notes: {}",
            vocab("move", &s.camera_movement), vocab("shotSize", &s.scale), vocab("angle", &s.camera_angle), s.camera_prompt_note()
        ));
    } else {
        lines.push(format!("Camera: {}; {}; {}. {}", vocab("shotSize", &s.scale), vocab("angle", &s.camera_angle), vocab("move", &s.camera_movement), s.camera_prompt_note()));
    }
    if let Some(rule) = distant_face_rule(&s.scale, ctx.style) {
        lines.push(rule.to_string());
    }
    if let Some(scene) = ctx.scene() {
        let has_tag = ctx.slots().iter().any(|(_, a)| a.is_some_and(|a| a.id == scene.id));
        lines.push(format!("Scene{}: {} — {}", if has_tag { "" } else { " (no reference image; describe it in words)" }, scene.name, short_description(scene, 120)));
    }
    // 没挂参考图的人没有标签：明说「用文字描述、不许给标签」。
    // 实跑第 11 镜：只挂了 1 张图，画面里 3 个人，模型自己编了 <Subject 2>、<Subject 3>
    let tags = tags(ctx);
    for c in ctx.characters() {
        let tag = tags.iter().find(|t| t.name == c.name);
        let absent = if ctx.keyframe_video() { ctx.shot.absent_from_opening(&c.name) } else { None };
        match absent {
            Some(crate::cinema::OpeningAbsence::VoiceOnly) => {
                lines.push(format!(
                    "Off-screen voice only, NOT in <Picture 1> and never seen in this shot (no tag): {}. \
                     Nobody visible in <Picture 1> is this person, and nobody visible moves their lips for these lines.",
                    c.name
                ));
                continue;
            }
            Some(crate::cinema::OpeningAbsence::EntersLater) => {
                lines.push(format!(
                    "Character NOT in <Picture 1> — enters partway through the shot (no tag, no reference image): {} — {}. \
                     Nobody already visible in <Picture 1> is this person: never write that they are visible in <Picture 1>, \
                     never give their actions to someone already in the frame; when they appear, say a new person enters and describe their look in words.",
                    c.name,
                    short_description(c, CHAR_DESC_MAX)
                ));
                continue;
            }
            None => {}
        }
        // 首帧出片（官方 I2VA / FL2VA）：首帧里的人没有标签，按「身份 + 在首帧里的位置」称呼，长相全看首帧
        if ctx.keyframe_video() {
            lines.push(format!(
                "Person already visible in <Picture 1> (the first frame; no tag in this format): {} — refer to them by role and position in the frame, \
                 e.g. \"the young man crouching by the door in <Picture 1>\", the same words every time. Their face, hair, costume and colours come only \
                 from <Picture 1>: never describe them.",
                c.name
            ));
            continue;
        }
        match tag.and_then(|t| t.subject) {
            Some(n) => lines.push(format!("Character in frame: {} = <Subject {n}> — {}", c.name, short_description(c, CHAR_DESC_MAX))),
            None => lines.push(format!("Character in frame WITHOUT a tag (describe in words, never give it a <Subject> tag): {} — {}", c.name, short_description(c, CHAR_DESC_MAX))),
        }
    }
    for p in ctx.props() {
        match tags.iter().find(|t| t.name == p.name).and_then(|t| t.subject) {
            Some(n) => lines.push(format!("Prop in frame: {} = <Subject {n}> — {}", p.name, short_description(p, PROP_DESC_MAX))),
            // 关键帧出片：道具多半已经画在首帧里了（第 8 镜的铭牌：描述写粉光，首帧是青色能量核心）
            None if ctx.keyframe_video() => lines.push(format!(
                "Prop in frame: {} — if it is already visible in <Picture 1>, its look (shape, colour, glow) comes from the picture and is not described; \
                 only if it enters later, describe it as: {}",
                p.name, short_description(p, PROP_DESC_MAX)
            )),
            None => lines.push(format!("Prop in frame WITHOUT a tag (no reference image; describe its look in words): {} — {}", p.name, short_description(p, PROP_DESC_MAX))),
        }
    }
    // 兜底：资产库里根本没有的，也要写进画面，按文字描述
    for t in ctx.text_only().iter().filter(|t| !t.in_library) {
        let what = match t.kind {
            crate::assets::AssetKind::Character => "Person",
            crate::assets::AssetKind::Scene => "Place",
            _ => "Object",
        };
        lines.push(format!("{what} in frame, not in the asset library, no reference image (describe in words, never give it a tag): {} — {}", t.name, t.description));
    }
    for item in &s.scene_items {
        lines.push(format!("Placement: {} — {}", item.name.trim(), item.detail.trim()));
    }
    // 这一场的固定布景：人和灯在场里的位置整场不变，换机位只换落在画面哪一边（translate it; keep left/right consistent with it）
    let stage = s.stage.lines();
    if !stage.is_empty() {
        lines.push(format!(
            "Scene staging, fixed for the whole scene (translate; everyone's position and every light's direction follow it — a new camera angle only changes which side of the frame they fall on): {}",
            stage.join(" | ")
        ));
    }
    lines.push(format!("Lighting: {}", s.lighting.trim()));
    // **段号 + 起止秒**：正文要按段号逐段写、每段开头写 `At 00:SS.S,`，代码逐段核对（见 `segment_coverage`）
    for (i, seg) in s.segments.iter().enumerate() {
        lines.push(format!(
            "Segment {} ({}-{}s)「{}」 visual: {} | must not: {} | keep: {} | blocking: {} | camera: {} | sound: {}",
            i + 1, seg.start, seg.end, seg.title.trim(), seg.visual.trim(), seg.forbid.trim(), seg.constraints.trim(), seg.blocking.trim(), seg.camera.trim(), seg.sound.trim()
        ));
    }
    for (i, l) in s.dialogue_lines.iter().filter(|l| !l.line.trim().is_empty()).enumerate() {
        lines.push(format!("[LINE {}] spoken by {}{} (meaning, for timing only — do not copy the words): {}", i + 1, l.speaker.trim(), if l.off_screen { ", off-screen" } else { "" }, l.line.trim()));
    }
    if s.dialogue_lines.iter().all(|l| l.line.trim().is_empty()) {
        lines.push("Nobody speaks in this shot.".into());
    }
    lines.push(format!("Ambience: {} | Effects: {}", s.ambient_sound.trim(), s.sound_effect.trim()));
    lines.join("\n")
}

fn tag_table(tags: &[Tag]) -> String {
    if tags.is_empty() {
        // **说三遍。** 实跑 E05 第 2 镜：这一镜一张参考图都没有，模型照样
        // 写了一堆 <Subject 1> / <Subject 2>（它在抄模板里的例子），
        // 九条「用了没有定义的标签」，重试三次全废。
        return "THIS SHOT HAS NO REFERENCE IMAGES AND NO TAGS.\n\
                Write no <Subject N> and no <Picture N> anywhere — not in subjects, not in summary, \
                not in detailed_description. Return \"subjects\": [] and describe every person and \
                thing in plain English words.\n\
                Allowed tags in this shot: none."
            .into();
    }
    use crate::assets::AssetKind;
    let mut lines: Vec<String> = tags
        .iter()
        .map(|t| {
            let what = match t.kind {
                Some(AssetKind::Character) => "a PERSON",
                Some(AssetKind::Scene) => "a PLACE, not a person",
                Some(AssetKind::Prop) => "an OBJECT, not a person",
                _ => "a reference",
            };
            match t.subject {
                Some(n) => format!("<Subject {n}> = {}, which is {what} ({}), from <Picture {}>. Description: {}. Controls: {}. Does not control: {}.", t.name, usage_en(t.usage), t.picture, t.description, t.control, t.not_control),
                None => format!("<Picture {}> = the {} of this shot. Controls: {}.", t.picture, usage_en(t.usage), t.control),
            }
        })
        .collect();
    let allowed: Vec<String> = tags
        .iter()
        .flat_map(|t| t.subject.map(|n| format!("<Subject {n}>")).into_iter().chain(std::iter::once(format!("<Picture {}>", t.picture))))
        .fold(Vec::new(), |mut v: Vec<String>, t| {
            if !v.contains(&t) {
                v.push(t);
            }
            v
        });
    if tags.iter().any(|t| t.usage == RefUsage::StartFrame && t.subject.is_none()) {
        // 首帧 / 首尾帧：写的是**从首帧长出来的运动**，不是把图再描述一遍（h3-prompt-format 第 6 节）
        let end = tags.iter().find(|t| t.usage == RefUsage::EndFrame).map(|t| format!(" and lands exactly on <Picture {}> at the end", t.picture)).unwrap_or_default();
        lines.push(
            "OFFICIAL KEYFRAME FORMAT (I2VA / FL2VA): this format has NO <Subject N> tags. Return \"subjects\": [] and never write <Subject N>. \
             Refer to each person already in <Picture 1> by role and position (\"the young man crouching by the door in <Picture 1>\"), \
             the same words every time; the dialogue sentences already name the speakers this way."
                .into(),
        );
        lines.push(format!(
            "KEYFRAME SHOT: the video starts exactly on <Picture 1>{end}. Do not re-describe the picture detail by detail — \
             write the motion that grows out of it: how each person moves, how the camera moves, how the light changes. \
             Everyone already in <Picture 1> stays in the same place, costume and look unless the action moves them.\n\
             LOOKS COME ONLY FROM <Picture 1>: where a segment below describes how someone looks — hair, eye colour, skin, costume, \
             fabric, colours, the colour of a glow on their clothes or body — do not translate that part; write \"exactly as in <Picture 1>\" instead. \
             Words that disagree with the picture make the video redraw the person after the first second. \
             A crowd or a row of similar figures keeps its own look, and each tagged subject stays visibly different from them.\n\
             THE OPENING FRAMING IS <Picture 1> ITSELF: you cannot see the picture, and the shot size, angle and \"starts on ...\" in the \
             camera notes below are what was intended — the picture may be framed differently. Never state the opening shot size or angle \
             and never write what the camera starts on (no \"opens on a close-up of her boots\", no \"a low-angle medium shot\" at 00:00). \
             Write only how the camera moves away from the opening frame, and where it ends. \
             Words that disagree with the picture make the camera jerk while the model redraws the room."
        ));
    }
    lines.push(format!("Allowed tags in this shot: {}. Any other tag number is an error; everyone else is described in plain words.", allowed.join(", ")));
    lines.join("\n")
}

#[derive(Debug, Default, Clone, Deserialize)]
struct Wording {
    #[serde(default)]
    subjects: Vec<SubjectWording>,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    detailed_description: String,
    #[serde(default)]
    overall_soundscape: String,
    #[serde(default)]
    frame: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct SubjectWording {
    #[serde(default)]
    tag: String,
    #[serde(default)]
    look: String,
    #[serde(default)]
    control: String,
    #[serde(default)]
    not_control: String,
}

/// **写手漏写了某个主体的外貌，补一句英文「照 <Picture N> 里那样」**，不让整镜作废。回补上的是哪几个（`<Subject N>`, 名字）。
///
/// 2026-10-03 EP02 第 22 镜：本机模型（ornith）连着几次都没给场景主体「下城巷子墙边」写 look，
/// 硬检查「没写外貌」三次重试全挂，用户合成了好几轮。
///
/// **不补资产的中文描述**（2026-10-06 改）：以前补的是 `室外，日/夜，雨。窄巷两侧霓虹招牌…`，
/// 理由是「H3 读得懂中文」—— 但 H3 会把正文里的中文当台词念出来（EP03 第 5 镜 Whisper 实测，
/// 见 `segment_coverage`）。参考图就挂在这个标签上，长相交给图，文字只说「照图」。
///
/// 资产描述是空的也不补 —— 那说明资产本身没建好，照旧是硬问题。
fn fill_missing_looks(tags: &[Tag], w: &mut Wording) -> Vec<(u32, String)> {
    let mut filled = Vec::new();
    for t in tags {
        let Some(n) = t.subject else { continue };
        let desc = t.description.trim();
        if desc.is_empty() {
            continue;
        }
        let tag = format!("<Subject {n}>");
        let look = picture_look(t);
        match w.subjects.iter_mut().find(|s| s.tag.trim() == tag) {
            Some(s) if !s.look.trim().is_empty() => continue,
            Some(s) => {
                s.look = look;
                if s.control.trim().is_empty() {
                    s.control = control_en(t);
                }
                if s.not_control.trim().is_empty() {
                    s.not_control = not_control_en(t);
                }
            }
            None => w.subjects.push(SubjectWording { tag: tag.clone(), look, control: control_en(t), not_control: not_control_en(t) }),
        }
        filled.push((n, t.name.clone()));
    }
    filled
}

/// 写手没写「控制什么」时的兜底。镜头表里的那份（[`Tag::control`]）多半是中文 ——
/// 2026-10-06 EP03 第 11、12 镜：补外貌时把「南北距离、铁皮屋和巷口方位」原样抄进了主体定义和
/// retention 那一行，H3 会把正文里的中文念出来。是中文就换成按类型的英文说法，英文的照用。
fn control_en(t: &Tag) -> String {
    if !t.control.chars().any(is_cjk) && !t.control.trim().is_empty() {
        return t.control.clone();
    }
    match t.kind {
        Some(crate::assets::AssetKind::Character) => "facial structure, hairstyle, build and costume".into(),
        Some(crate::assets::AssetKind::Scene) => "the spatial layout, furnishings and materials".into(),
        Some(crate::assets::AssetKind::Prop) => "its shape, color and material".into(),
        _ => "its appearance".into(),
    }
}

/// 见 [`control_en`]
fn not_control_en(t: &Tag) -> String {
    if !t.not_control.chars().any(is_cjk) {
        return t.not_control.clone();
    }
    match t.kind {
        Some(crate::assets::AssetKind::Scene) => "character positions, lighting intensity or composition".into(),
        _ => "pose, position, expression or composition in this shot".into(),
    }
}

/// 漏写外貌时的英文兜底：`the place exactly as shown in <Picture 4>`。见 [`fill_missing_looks`]。
fn picture_look(t: &Tag) -> String {
    let what = match t.kind {
        Some(crate::assets::AssetKind::Character) => "the person",
        Some(crate::assets::AssetKind::Scene) => "the place",
        Some(crate::assets::AssetKind::Prop) => "the object",
        _ => "the subject",
    };
    format!("{what} exactly as shown in <Picture {}>", t.picture)
}

/// 把一份模型输出按生产链路解析一遍，只回「行不行」。
///
/// 给「设置 → 提示词设置」的**试跑**用：试跑回一段文本不等于正式合成能过，
/// 真正会拦住的是解析这一步。见 [`crate::prompts::TryOutcome`]。
pub fn try_parse(raw: &str) -> std::result::Result<(), String> {
    parse_wording(raw).map(|_| ())
}

fn parse_wording(raw: &str) -> std::result::Result<Wording, String> {
    let json = crate::prompts::extract_json(raw).ok_or("返回里没有 JSON")?;
    serde_json::from_str(json).map_err(|e| format!("JSON 解析失败：{e}"))
}

/// 按标签和措辞拼出最终正文。**结构全在这里**，LM 的措辞只填进固定位置。
/// `long_take_secs` = 这一镜是长镜头里的一个窗口时，它有多少秒。
/// `None` = 不是长镜头。见 [`LONG_TAKE_TAIL_SECS`]。
/// 这一镜按官方哪种模式写（见 [`crate::h3_spec`] 的对照表）。
fn mode(ctx: &ShotContext, tags: &[Tag]) -> crate::h3_spec::Mode {
    use crate::h3_spec::Mode;
    if ctx.keyframe_video() {
        if ctx.shot.keyframes.last { Mode::Fl2va } else { Mode::I2va }
    } else if tags.is_empty() {
        Mode::T2va
    } else {
        Mode::Ref2va
    }
}

/// 按官方格式拼整份提示词。段名、段序、标签、对齐指令全由代码写；写手只给措辞（[`Wording`]）。
///
/// - 基础模式（T2VA / I2VA / FL2VA）：对齐指令（首帧两种才有）+ 三字段，画风写在 `[Shot 1]` 开头（官方 4.1）；
/// - Ref2VA：六段，画风写在 `[Shot 1]` 之前（官方 5.2）。
///
/// 两种都：时间写法规整成官方的（[`official_timing`]），画面文字的「」换成英文双引号（官方 4.5）。
/// 代码要补的话（质感锁、长镜头窗口的说话时限）写成正文里的英文句子，不再塞进 subject_definitions 当一行「定义」。
fn assemble(ctx: &ShotContext, tags: &[Tag], markers: &[LineMarker], w: &Wording, continuity: crate::cinema::Continuity) -> String {
    use crate::h3_spec::Mode;
    let m = mode(ctx, tags);
    let filled = fill_lines(w.detailed_description.trim(), markers);
    let desc = screen_quotes(&official_timing(&filled, ctx.shot.one_take));
    let soundscape = screen_quotes(w.overall_soundscape.trim());
    let extras = notes_for_body(ctx, m).join(" ");
    let (style, body) = match desc.find("[Shot 1]") {
        Some(at) => (desc[..at].trim().to_string(), desc[at + "[Shot 1]".len()..].trim().to_string()),
        None => (String::new(), desc.trim().to_string()),
    };
    if m.is_base() {
        let opening = [style.as_str(), extras.as_str(), body.as_str()].iter().filter(|t| !t.is_empty()).copied().collect::<Vec<_>>().join(" ");
        let core = format!("integrated_multimodal_description: [Shot 1] {opening}\n\noverall_soundscape: {soundscape}\n\nnon_diegetic_music: N/A");
        return match m {
            Mode::I2va => format!("{}\n\n{core}", crate::h3_spec::I2VA_INSTRUCTION),
            Mode::Fl2va => format!("{}\n\n{core}", crate::h3_spec::fl2va_instruction(last_shot_number(&desc), ctx.shot.duration.max(1.0))),
            _ => core,
        };
    }
    let wording = |n: u32| w.subjects.iter().find(|s| s.tag.trim() == format!("<Subject {n}>"));
    let mut defs = Vec::new();
    let mut keeps = Vec::new();
    for t in tags {
        match t.subject {
            Some(n) => {
                let sw = wording(n);
                let look = sw.map(|s| s.look.trim()).filter(|l| !l.is_empty()).map_or_else(|| picture_look(t), str::to_string);
                // 写手没写就用代码的英文兜底 —— **不用镜头表里的中文**（[`control_en`]）
                let control = sw.map(|s| s.control.trim()).filter(|c| !c.is_empty()).map_or_else(|| control_en(t), str::to_string);
                // **官方写法**：`<Subject 1> is the young woman in <Picture 1>, with long dark hair…`（ref 指南 2.1），一行一个标签。
                // 以前多一句非官方的「<Picture N> only defines … ; it does not define …」，偶尔还拼成病句；控制范围挪进 retention。
                // 外貌不改大小写 —— 它常以人名开头（Shen Zhaozhao），小写就错了。
                let look = look.trim_end_matches('.');
                let src = format!("<Picture {}>", t.picture);
                defs.push(if look.contains(&src) { format!("<Subject {n}> is {look}.") } else { format!("<Subject {n}> is {look}, as shown in {src}.") });
                keeps.push(format!("<Subject {n}> (appears in [Shot 1]): {} - {}.", retention(t.usage), lower_first(control.trim_end_matches('.'))));
            }
            None => {
                // 帧锚点图（官方 2.2）：`<Picture 2> is the first frame of [Shot 1], showing …`；retention 写 `([Shot 1] first frame)`
                let role = frame_role(t.usage);
                defs.push(format!("<Picture {}> is the {role} of [Shot 1]: {}.", t.picture, t.control.trim_end_matches('.')));
                keeps.push(format!("<Picture {}> ([Shot 1] {role}): {} - {}.", t.picture, retention(t.usage), t.control.trim_end_matches('.')));
            }
        }
    }
    // **接上一镜：`<Video 1>` 的定义由代码写，不交给模型。**
    //
    // 2026-09-20 实测确认了两件事，都写进这段话里：
    //
    // 1. **只绑数据不写标签等于没传**（反过来也一样）。官方示例工作流
    //    `h3_ref2va_balance.json` 里 `ReferenceEncode` 的提示词就明写
    //    `<Video 1> defines the target motion rhythm, camera behavior...`。
    // 2. **必须写成「续接」而不是「模仿运镜」。** 不加最后那句
    //    "it does not define what happens next" 的话，它有可能把上一镜的
    //    动作也照抄一遍。实测加了之后第 2 镜从熄屏开始演新内容，没有重复。
    //
    // 同一次实测也确认它**管不了身份** —— 所以这句话只说机位、光线、布景，
    // 不提长相。人物长相靠 `<Picture N>`。
    //
    // **长镜头里绝不发 `<Video 1>`**：整段出片（`cinema::run_long_take`）不绑任何视频输入，
    // 窗口之间的衔接是 H3 潜变量的重叠帧，写了就是一个不存在的 `<Video 1>`，而它不报错。
    let continues = continuity == crate::cinema::Continuity::RefVideo && !ctx.long_take;
    if continues {
        defs.push(
            "<Video 1> is the end of the immediately preceding shot in the same scene. \
             Continue directly from it: keep the identical camera position and lens, \
             the identical lighting and colour, and the identical placement of everything \
             already in frame. <Video 1> defines continuity of camera, staging and lighting only; \
             it does not define what happens next, and it does not define anyone's appearance."
                .into(),
        );
        keeps.push("<Video 1> (continuation source): partially_preserved - camera, lighting and staging only.".into());
    }
    // 任务类型前缀（官方 3 节）：按每份参考实际起的作用，` + ` 连接、不重复
    let mut task = Vec::new();
    if continues {
        task.push("video continuation");
    }
    if tags.iter().any(|t| t.subject.is_some()) {
        task.push("reference generation");
    }
    if tags.iter().any(|t| t.subject.is_none()) {
        task.push("keyframe completion");
    }
    let cast = cast_note(tags).unwrap_or_default();
    let opening = [style.as_str(), extras.as_str(), cast.as_str()].iter().filter(|t| !t.is_empty()).copied().collect::<Vec<_>>().join(" ");
    format!(
        "subject_definitions:\n{}\n\nsummary:\n[{}] {}\n\nretention_analysis:\n{}\n\ndetailed_description:\n{}{}[Shot 1] {}\n\noverall_soundscape: {}\n\nnon_diegetic_music: N/A",
        defs.join("\n"),
        task.join(" + "),
        screen_quotes(strip_task_prefix(w.summary.trim())),
        keeps.join("\n"),
        opening,
        if opening.is_empty() { "" } else { "\n" },
        body,
        soundscape
    )
}

/// 写手自己在 summary 开头写了任务前缀（`[reference generation] …`）就去掉 —— 前缀由代码按参考的实际作用定，
/// 不去掉就成了 `[reference generation] [reference generation] …`（2026-10-06 EP03 第 2 镜首次按官方格式合成时出现）。
fn strip_task_prefix(summary: &str) -> &str {
    const TASKS: [&str; 6] = ["reference generation", "keyframe completion", "video editing", "video continuation", "audio reuse", "audio reference"];
    let mut s = summary.trim_start();
    while let Some(rest) = s.strip_prefix('[') {
        let Some(close) = rest.find(']') else { break };
        if !TASKS.iter().any(|t| rest[..close].contains(t)) {
            break;
        }
        s = rest[close + 1..].trim_start();
    }
    s
}

/// 帧锚点图在官方写法里叫什么（`first frame` / `last frame` / `composition anchor`）。
fn frame_role(u: RefUsage) -> &'static str {
    match u {
        RefUsage::EndFrame => "last frame",
        RefUsage::Composition => "composition anchor",
        _ => "first frame",
    }
}

/// 代码要补进正文的话（英文整句，放在画风后面、动作前面）。
///
/// **画面质感锁**（2026-10-04，用户：「这种失真的情况这是第二次发生」）：EP01 第 8 镜、EP02 第 17 镜都是首帧出片，
/// 都在 3.00 秒出了一个硬切，切过去之后亮度 +32% / +35%、锐度 +30% —— 首帧是唯一管质感的东西，切走之后就没人管了。
/// 首帧出片：质感照 <Picture 1> 一直到最后；一镜出（允许硬切）：每次切镜前后一样。参考图出片的普通镜头不加。
///
/// **长镜头窗口：台词必须在重叠区之前说完。** 窗口之间的重叠区要被裁掉 / 被下一窗口重采，半个字卡在裁切点上结果不可预测；
/// 镜头表的容量检查只管「说多少字」，不管「说在哪几秒」，所以只能在提示词里约束位置。
///
/// 以前这两句作为单独一行塞在 subject_definitions 里，那一段官方只放标签定义（2026-10-06 改）。
fn notes_for_body(ctx: &ShotContext, m: crate::h3_spec::Mode) -> Vec<String> {
    let grade = "exposure, brightness, contrast, colour grade, sharpness and film grain";
    let mut out = Vec::new();
    match (m.is_base() && m != crate::h3_spec::Mode::T2va, ctx.shot.one_take) {
        (true, true) => out.push(format!("The {grade} stay exactly as in <Picture 1> for the entire video, including after every cut — a cut changes the angle, never the look.")),
        (true, false) => out.push(format!("This is one continuous take with no cuts; the {grade} stay exactly as in <Picture 1> from the first frame to the last.")),
        (false, true) => out.push(format!("Across every cut, the {grade} stay the same as just before the cut — a cut changes the angle, never the look.")),
        (false, false) => {}
    }
    if ctx.long_take {
        let deadline = (ctx.shot.duration - LONG_TAKE_TAIL_SECS).max(1.0);
        out.push(format!(
            "This shot is one window of a continuous long take. All speech must finish before the {deadline:.0}-second mark. \
             The final {LONG_TAKE_TAIL_SECS:.0} seconds carry motion and ambience only — nobody speaks, and no line starts there."
        ));
    }
    out
}

/// 句中片段首字母小写（接在冒号后面）。「A 28-year-old」这种冠词开头的外貌也跟着小写，读起来是一句话。
fn lower_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) if f.is_ascii_uppercase() && !c.clone().next().is_some_and(|n| n.is_ascii_uppercase()) => f.to_ascii_lowercase().to_string() + c.as_str(),
        Some(f) => f.to_string() + c.as_str(),
        None => String::new(),
    }
}

/// 中日韩文字（台词标签外不许出现在 LM 写的英文措辞里）。
fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3400..=0x9FFF | 0xF900..=0xFAFF | 0x3000..=0x303F | 0xFF00..=0xFFEF)
}

/// 去掉 `<d>…</d>` 里的内容。
fn outside_dialogue(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(open) = rest.find("<d>") {
        out.push_str(&rest[..open]);
        match rest[open..].find("</d>") {
            Some(close) => rest = &rest[open + close + 4..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// 自然语速，可发声字 / 秒。和 `shot_table::MAX_SPOKEN_CPS`、
/// `h3_prompt::NATURAL_CPS` 是同一个数 —— 三处读同一份才有意义。
const SPOKEN_CPS: f32 = 4.1;

/// 台词时刻核对的容差（秒）。**和时间码的精度一样：0.1 秒。**
///
/// 正文里的时间码只写到一位小数。原来容差是 0.05，而 [`feasible_starts`] 给的建议时刻
/// 每个都要四舍五入成一位小数才能写进去 —— 前一句往上舍、后一句往下舍，误差能叠到将近 0.1，
/// 照着建议写也过不了。2026-10-03 EP02 第 10 镜：建议「第 1 句 00:2.2、第 2 句 00:4.1」，
/// 照写之后 2.2 + 1.95 = 4.15 撞上 4.1 + 0.05，三次重试都卡在这 0.001 秒上。
/// 比时间码精度还小的重叠写不出来，也听不出来。
const TIMING_TOLERANCE: f32 = 0.1;

/// 一套排得下的开口时刻：从镜尾往前倒推每句最晚几秒开口，再从头往后排 ——
/// 每句尽量保留写的时刻，晚于最晚时刻的提前，并且不早于前一句说完。整镜台词本身就说不完返回 `None`。
pub fn feasible_starts(timings: &[LineTiming], deadline: f32) -> Option<Vec<(usize, f32)>> {
    let mut latest = vec![0.0f32; timings.len()];
    let mut next = deadline;
    for (i, t) in timings.iter().enumerate().rev() {
        latest[i] = next - t.needs;
        next = latest[i];
    }
    if latest.first().is_some_and(|l| *l < 0.0) {
        return None;
    }
    let mut out = Vec::with_capacity(timings.len());
    let mut free = 0.0f32;
    for (i, t) in timings.iter().enumerate() {
        // 不在这里取整：写成一位小数时每个时刻各有 ≤ 0.05 秒的舍入误差，相邻两句叠起来 < 0.1 秒，在核对的容差（TIMING_TOLERANCE）里
        let at = t.at.min(latest[i]).max(free);
        out.push((t.n, at));
        free = at + t.needs;
    }
    Some(out)
}

/// 正文里每一句台词的开口时刻。
#[derive(Debug, Clone, PartialEq)]
pub struct LineTiming {
    /// 第几句（1 起，对应 `[LINE N]`）
    pub n: usize,
    /// 写在正文里的开口秒数
    pub at: f32,
    /// 这一句有多少可发声字
    pub chars: usize,
    /// 按自然语速要多少秒
    pub needs: f32,
}

/// 从正文里把 `At 00:SS.S, … <d>[Chinese] 台词</d>` 的时刻和字数扒出来。
///
/// # 为什么要扒而不是自己定
///
/// 用户 2026-09-20 指出的：台词该在第几秒，应该在分镜那一层就定好，
/// 不该出片时求模型自觉。
///
/// 完全由代码定要给 `ShotSegment` 加「这一段说哪一句」，而已有的
/// E01~E08 镜头表都没有那个字段 —— 得整批重新生成。
///
/// 所以先走中间路线：**模型定位置，代码验可行性**。模型有上下文，
/// 知道哪句该早哪句该晚；代码只负责回答「这个时刻放得下这句话吗」——
/// 而那是个纯算术问题，模型恰恰算不准。
///
/// 放不下就进重试反馈让它往前挪，三次都不行才报错。
pub fn line_timings(video: &str) -> Vec<LineTiming> {
    let mut out = Vec::new();
    let mut n = 0;
    let mut rest = video;
    while let Some(open) = rest.find("<d>") {
        let Some(close) = rest[open..].find("</d>") else { break };
        n += 1;
        let body = &rest[open + 3..open + close];
        // 剥掉 `[Chinese]` 这类语言标签再数字
        let body = match (body.trim_start().strip_prefix('['), body.find(']')) {
            (Some(_), Some(at)) => &body[at + 1..],
            _ => body,
        };
        let chars = spoken(body).chars().count();
        // 往前找最近的 `At 00:SS.S,`
        if let Some(at) = last_timecode(&rest[..open]) {
            out.push(LineTiming { n, at, chars, needs: chars as f32 / SPOKEN_CPS });
        }
        rest = &rest[open + close + 4..];
    }
    out
}

/// `text` 末尾最近的一个时刻（见 [`timecodes`]），回它的秒数。
fn last_timecode(text: &str) -> Option<f32> {
    timecodes(text).last().copied()
}

/// `text` 里所有的时刻，按出现顺序：官方切镜 `At 00:03.500`、镜内节拍 `Around the 3.5-second mark`（2026-10-06 起），
/// 以及老提示词里的 `At 00:03.5`。**只认 `around the` / `at the` 开头的 `-second mark`** ——
/// FL2VA 对齐指令的 `aligns with the 8.00-second mark`、长镜头窗口的 `before the 8-second mark` 不是节拍。
fn timecodes(text: &str) -> Vec<f32> {
    timecode_positions(text).into_iter().map(|(_, v)| v).collect()
}

/// 同 [`timecodes`]，带每个时刻在 `text` 里的字节位置。
fn timecode_positions(text: &str) -> Vec<(usize, f32)> {
    let mut out = Vec::new();
    for (i, _) in text.match_indices("At ") {
        if let Some((v, _)) = parse_clock(&text[i + 3..]) {
            out.push((i, v));
        }
    }
    let lower = text.to_ascii_lowercase();
    for (i, _) in lower.match_indices("-second mark") {
        let head = &lower[..i];
        let num_start = head.rfind(|c: char| !(c.is_ascii_digit() || c == '.')).map_or(0, |p| p + 1);
        let prefix = &head[..num_start];
        if !(prefix.ends_with("around the ") || prefix.ends_with("at the ")) {
            continue;
        }
        if let Ok(v) = head[num_start..].parse::<f32>() {
            let start = if prefix.ends_with("around the ") { num_start - "around the ".len() } else { num_start - "at the ".len() };
            out.push((start, v));
        }
    }
    out.sort_by_key(|(p, _)| *p);
    out
}

/// 段开头的时间码允许差多少秒（模型会把 3.0 写成 3.2 这种）。
const SEGMENT_TIMECODE_TOLERANCE: f32 = 0.35;

/// **逐段核对**：镜头表的每一段，正文里都要有一个落在它起点附近的 `At 00:SS.S`。
/// 回没找到的那些段（段号从 1 数）。
///
/// # 为什么要有
///
/// 用户 2026-09-27：「同样 12 秒的视频……把时间线写好，描述细致，而不是一笔带过。」
/// 镜头表那一层写细了，这一步翻成英文时写手照样可以把 4 段压成 2 段 —— 以前的检查只看
/// `[Shot 1]`、词数、台词时刻、标签，漏段、并段都能静默过。
///
/// 时间码是能数的替身：段写没写到，看它开头有没有落在这一段的时刻上。内容翻得全不全代码判断不了，
/// 这一条只保证**每一段都被写到了**。
///
/// **进 `soft`，不进 `hard`**：软问题照样带着反馈重试（见合成循环），只是重试用完不作废这一镜 ——
/// 新要求先软着跑，E05 八镜废六镜就是新下限直接做硬的结果（见 `check_wording` 里台词时刻那条）。
///
/// **提前开口不算漏段**（2026-10-06 EP03 第 5 镜）：写手把吴峥的台词从第 4.0 秒挪到 3.2 秒开口，
/// 这一段照样写全了，只是早了 0.8 秒 —— 旧判据只认起点 ±0.35 秒，判成「漏了第 2 段」，
/// 重试用完又按镜头表的中文原文补了一句，H3 把那句舞台说明当台词念了出来。
/// 所以时间码可以比段起点早到 [`SEGMENT_EARLY_SLACK`] 秒（晚仍然只容 0.35 秒），
/// 但**每个时间码只能算给一段**、也不能早过上一段的起点 —— 并段照样抓得到。
pub fn segment_coverage(segments: &[crate::shot_table::ShotSegment], desc: &str) -> Vec<usize> {
    let codes = timecodes(desc);
    let mut used = vec![false; codes.len()];
    // 从 0 秒开始的那一段：`[Shot 1]` 后面的正文开头就是第 0 秒，不写 `At 00:00.0` 也不算漏
    let opens = desc.contains("[Shot 1]");
    let mut missing = Vec::new();
    for (i, seg) in segments.iter().enumerate() {
        if opens && seg.start <= SEGMENT_TIMECODE_TOLERANCE {
            continue;
        }
        let floor = i
            .checked_sub(1)
            .and_then(|p| segments.get(p))
            .map_or(f32::MIN, |prev| prev.start + SEGMENT_TIMECODE_TOLERANCE)
            .max(seg.start - SEGMENT_EARLY_SLACK);
        let hit = codes
            .iter()
            .enumerate()
            .find(|(k, t)| !used[*k] && (seg.start - **t).abs() <= SEGMENT_TIMECODE_TOLERANCE)
            .or_else(|| codes.iter().enumerate().find(|(k, t)| !used[*k] && **t > floor && **t < seg.start));
        match hit {
            Some((k, _)) => used[k] = true,
            None => missing.push(i + 1),
        }
    }
    missing
}

/// 段开头的时间码最多可以**提前**多少秒（写手把台词往前挪，见 [`segment_coverage`]）。
const SEGMENT_EARLY_SLACK: f32 = 1.0;

/// 画面文字最多这么长（字符）。
///
/// 超过就不是「快递单上印着四个字」了，多半是模型把整段中文正文
/// 塞进 `「」` 来绕开中文检查。
const MAX_SCREEN_TEXT_CHARS: usize = 20;

/// 去掉 `「…」` 里的内容 —— 画面里**本来就是中文**的字。
///
/// # 为什么必须开这个口子
///
/// 模板第 4 条明着要求「keep X in the original Chinese, character for character」：
/// 快递单、手机屏幕、招牌、图表标题上的字是包袱落点，翻成英文就没了。
/// 而下面那个中文检查会把这几个字判成违规打回去 —— 模型照模板写就被拒，
/// 改成英文又把包袱丢了。**两条规则互相否定，而 `check_wording` 重试三次后
/// 整镜作废**（2026-09-19 Codex Astra 的第三份审查抓到）。
///
/// 所以要有一个机器认得出的记号。`「…」` 是中文里本来就当引号用的一对，
/// 模型不用学新语法；台词那套用 `<d>`，两者不会混。
///
/// 和 [`outside_dialogue`] 有一处**故意的不同**：`「` 找不到配对的 `」` 时
/// **不吞掉后面的内容**。`<d>` 是代码自己填的，不配平是我们的 bug；
/// `「` 是模型写的，一个落单的开引号不该让整段中文正文跟着隐身 ——
/// 那会把「中文没写进标签」这个真问题藏起来。
///
/// **官方写法是英文双引号**（基础指南 4.5：`A red neon sign reading "营业中"`）—— 2026-10-06 起拼装时把「」换成双引号
/// （[`screen_quotes`]），所以这里两种都认：双引号里**有中文**的才算画面文字，纯英文的引号照常按正文查。
fn outside_screen_text(text: &str) -> String {
    let text = &outside_cjk_quotes(text);
    let mut out = String::new();
    let mut rest = text.as_str();
    while let Some(open) = rest.find('\u{300c}') {
        match rest[open..].find('\u{300d}') {
            Some(close) => {
                out.push_str(&rest[..open]);
                rest = &rest[open + close + '\u{300d}'.len_utf8()..];
            }
            // 没闭合：后面的原样留下，交给中文检查去报
            None => break,
        }
    }
    out.push_str(rest);
    out
}

/// 台词 `<d>` 和画面文字 `「」` 以外的第一串中文（最多 14 个字，给提示用），没有就是 `None`。
fn stray_chinese(text: &str) -> Option<String> {
    let plain = outside_screen_text(&outside_dialogue(text));
    let start = plain.find(|c: char| is_cjk(c))?;
    let run: String = plain[start..].chars().take_while(|c| !c.is_ascii_alphabetic() && *c != '<').take(14).collect();
    Some(run.trim().to_string())
}

/// 去掉英文双引号里**带中文**的那几段（画面文字的官方写法）。落单的引号原样留下。
fn outside_cjk_quotes(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('"') {
        let Some(close) = rest[open + 1..].find('"') else { break };
        let inner = &rest[open + 1..open + 1 + close];
        if inner.chars().any(is_cjk) {
            out.push_str(&rest[..open]);
        } else {
            out.push_str(&rest[..open + 1 + close + 1]);
        }
        rest = &rest[open + 1 + close + 1..];
    }
    out.push_str(rest);
    out
}

/// 画面文字逐段：`「…」` 和带中文的英文双引号。
fn screen_text_spans(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    {
        let mut rest = text;
        while let Some(open) = rest.find('"') {
            let Some(close) = rest[open + 1..].find('"') else { break };
            let inner = &rest[open + 1..open + 1 + close];
            if inner.chars().any(is_cjk) {
                out.push(inner.to_string());
            }
            rest = &rest[open + 1 + close + 1..];
        }
    }
    let mut rest = text;
    while let Some(open) = rest.find('\u{300c}') {
        let after = &rest[open + '\u{300c}'.len_utf8()..];
        match after.find('\u{300d}') {
            Some(close) => {
                out.push(after[..close].to_string());
                rest = &after[close + '\u{300d}'.len_utf8()..];
            }
            None => break,
        }
    }
    out
}

/// 只留可发声的字，比台词用。
fn spoken(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).collect()
}

/// 一份措辞的问题，**分两档**。
///
/// # 为什么必须分档
///
/// 2026-09-19 实跑 E05：八镜里六镜合成失败，重试三次全废。原因几乎全是
/// 「detailed_description 只有 279 个英文词，太薄了 —— 要 320~520 词」。
/// 模型稳定写 270~360 词，而当天刚把下限从 150 提到 220/320/380 ——
/// **一条模型满足不了的检查，比没有检查更糟**：它不产出任何东西。
///
/// 所以判据要分开问两件事：
///
/// - `hard`：这份正文**发给 H3 会出错的东西**。台词没逐字写进去、
///   用了不存在的 `<Subject N>` 标签、缺段落、薄到不成句。留着就是废片。
/// - `soft`：**该更好，但能用**。词数没到推荐区间、正文里漏了几个中文
///   （H3 的编码器是 Qwen3-VL，中文读得懂）。
///
/// 重试时两档都写进反馈 —— 要的就是让它改。重试用完之后只看 `hard`：
/// 一份 279 词、带着「小青」两个汉字的正文能出片，一个错误不能。
/// `soft` 跟着结果一起交出去，落在合成结果的 notes 里。
#[derive(Debug, Default)]
pub struct Problems {
    pub hard: Vec<String>,
    pub soft: Vec<String>,
}

impl Problems {
    fn all(&self) -> Vec<String> {
        self.hard.iter().chain(self.soft.iter()).cloned().collect()
    }
}

/// 正文薄到这个数以下就不是「写短了」，是没写。
///
/// 这是 2026-09-19 之前唯一的下限，实跑从没被它卡住过 ——
/// 推荐区间（[`word_budget`]）比它高得多，但那一档是 `soft`。
pub const HARD_MIN_WORDS: usize = 150;

/// 检查 LM 写的措辞和拼出来的正文。`hard` 空 = 能出片。
fn check_wording(ctx: &ShotContext, tags: &[Tag], w: &Wording, video: &str) -> Problems {
    let mut out = Problems::default();
    let desc = w.detailed_description.trim();
    if !desc.contains("[Shot 1]") {
        out.hard.push("detailed_description 没有 [Shot 1]".into());
    }
    // 一镜出（允许切镜）按官方写 `[Shot N] At 00:SS.mmm, the camera cuts to …`；普通镜头一镜不切
    if desc.contains("[Shot 2]") && !ctx.shot.one_take {
        out.hard.push("一镜不切，detailed_description 里不能有 [Shot 2]".into());
    }
    // **上下限都要管。** 原来只拒绝少于 150 词，没有上限 ——
    // 而「写长一点」不是保证信息不丢的办法：一段 900 词的正文里，
    // 真正约束画面的那几句被稀释在一堆修辞里，H3 反而抓不住重点
    // （2026-09-19 Codex Astra 指出）。
    //
    // 短镜头还要额外收紧：5 秒的镜头写 500 词，等于每 0.01 秒一个词。
    //
    // **下限是软的**，理由见 [`Problems`]。上限是硬的 —— 写超了是真的
    // 会把要紧的几句稀释掉，而「删到 X 词」是模型做得到的事。
    let words = outside_dialogue(desc).split_whitespace().count();
    let (lo, hi) = word_budget_for(ctx.shot.duration, &ctx.shot.segments);
    if words < HARD_MIN_WORDS {
        out.hard.push(format!("detailed_description 只有 {words} 个英文词 —— 这不算写完，至少 {HARD_MIN_WORDS} 词"));
    } else if words < lo {
        out.soft.push(format!("detailed_description 只有 {words} 个英文词，偏薄 —— 这一镜 {:.0} 秒，写到 {lo}~{hi} 词更稳", ctx.shot.duration));
    } else if words > hi && words as f32 <= hi as f32 * WORDS_SOFT_OVER {
        // **超一点点是软的**（2026-09-30 EP02 第 6 镜）：写手在「台词排不开」和「词数 542 > 520」之间来回跳，
        // 改对了时间就多写二十个词、删了词又把时间改回去，三次重试全废，整镜没保存。超 4% 不会稀释掉什么
        out.soft.push(format!(
            "detailed_description 有 {words} 个英文词，略超 {lo}~{hi} 词 —— 压一压，要紧的几句别被稀释"
        ));
    } else if words > hi {
        out.hard.push(format!(
            "detailed_description 有 {words} 个英文词，这一镜只有 {:.0} 秒 —— 压到 {lo}~{hi} 词。写长不等于信息更多，要紧的几句会被稀释",
            ctx.shot.duration
        ));
    }
    // **二选一在合成时就喂回写手**（软问题：照样重试让它改，改不掉也交得出去、带着话）。
    // 以前只在校验时提示，已经合成好的没有修法，要用户自己去改英文（2026-09-30 EP02 第 22、26 镜）
    if let Some(around) = english_alternative(video) {
        out.soft.push(alternative_message(&around));
    }
    if let Some(around) = identity_recolor(desc) {
        out.soft.push(recolor_message(&around));
    }
    {
        let all = [w.summary.as_str(), desc, w.overall_soundscape.as_str(), w.frame.as_str()].into_iter()
            .chain(w.subjects.iter().flat_map(|s| [s.look.as_str(), s.control.as_str(), s.not_control.as_str()]))
            .collect::<Vec<_>>().join("\n");
        if let Some((n, around)) = compass_words(&all) {
            out.soft.push(compass_message(n, &around));
        }
    }
    let missing = segment_coverage(&ctx.shot.segments, desc);
    if !missing.is_empty() {
        let which: Vec<String> = missing
            .iter()
            .filter_map(|n| ctx.shot.segments.get(n - 1).map(|seg| format!("Segment {n}（00:{:04.1} 起「{}」）", seg.start, seg.title.trim())))
            .collect();
        out.soft.push(format!(
            "detailed_description 没写到 {} —— 第 1 段紧跟 [Shot 1]，后面每一段都以它起点的 `Around the N.N-second mark,` 开头，按顺序把那一段写全；不许并段，不许压成一句",
            which.join("、")
        ));
    }
    // **中文有两个合法出口：台词 `<d>…</d>` 和画面文字 `「…」`。**
    // 后者是 2026-09-19 补的 —— 在那之前模板要求逐字保留画面上的中文，
    // 这里却把它判成违规，模型照模板写就被打回。见 [`outside_screen_text`]。
    //
    // 剩下的漏网中文（人物名、一个形容词）走 `soft`：照样带着反馈重试，重试用完不作废这一镜
    // （E05 八镜废六镜的教训，见台词时刻那条）。**但它不只是体例问题**：H3 的编码器 Qwen3-VL
    // 读得懂中文，也**会把它念出来** —— EP03 第 5 镜整句舞台说明被当台词念了（2026-10-06 Whisper 实测）。
    // 交出去的正文里还有中文的，校验那边另有一条提示（[`advisories`]）。
    for (field, text) in [("summary", w.summary.as_str()), ("detailed_description", desc), ("overall_soundscape", w.overall_soundscape.as_str()), ("frame", w.frame.as_str())] {
        let plain = outside_screen_text(&outside_dialogue(text));
        let cjk: String = plain.chars().filter(|c| is_cjk(*c)).take(12).collect();
        if !cjk.is_empty() {
            out.soft.push(format!(
                "{field} 在标签外面写了中文「{cjk}」—— 台词写进 <d>…</d>，画面上本来就是中文的字（快递单、手机屏幕、招牌）放进英文双引号 \"…\"，其余一律英文（H3 会把正文里的中文当台词念出来）"
            ));
        }
        for span in screen_text_spans(text) {
            let n = span.chars().count();
            if n > MAX_SCREEN_TEXT_CHARS {
                let head: String = span.chars().take(12).collect();
                out.soft.push(format!(
                    "{field} 的画面文字「{head}…」有 {n} 个字 —— 引号里只放画面上真看得见的字，不超过 {MAX_SCREEN_TEXT_CHARS} 个；正文写英文"
                ));
            }
        }
    }
    for sw in &w.subjects {
        let cjk: String = [&sw.look, &sw.control, &sw.not_control]
            .iter()
            .flat_map(|t| outside_screen_text(t).chars().collect::<Vec<_>>())
            .filter(|c| is_cjk(*c))
            .take(12)
            .collect();
        if !cjk.is_empty() {
            out.soft.push(format!("{} 的外貌或控制范围写了中文「{cjk}」—— 写英文（衣服上印的字可以放进英文双引号）", sw.tag));
        }
    }
    // **逐句验台词放不放得下。** 用户 2026-09-20 指出的那件事。
    //
    // 镜头表那层的容量检查算的是**整镜能说多少字**；这里算的是
    // **这一句从第几秒开口、到第几秒说完、会不会撞上下一句或镜尾**。
    // 前者答「字数超没超」，后者答「改哪一句、往前挪多少」。
    //
    // 判据里没有任何模型的判断 —— 纯算术（字数 ÷ 4.1 字/秒），
    // 而这恰恰是模型算不准的那一类。所以它进 `hard`：
    // 重试时把具体数字喂回去，模型只要把时间码往前挪就能过。
    {
        let timings = line_timings(video);
        let tail = if ctx.long_take { LONG_TAKE_TAIL_SECS } else { 0.0 };
        let deadline = (ctx.shot.duration - tail).max(1.0);
        let before = out.hard.len();
        for (i, t) in timings.iter().enumerate() {
            let ends = t.at + t.needs;
            // 下一句开口之前必须说完
            if let Some(next) = timings.get(i + 1) {
                if ends > next.at + TIMING_TOLERANCE {
                    out.hard.push(format!(
                        "第 {} 句台词 {} 个字（自然语速要 {:.1} 秒），写的是第 {:.1} 秒开口 —— 要说到 {:.1} 秒，\
                         而第 {} 句第 {:.1} 秒就开口了。把前一句的开口时刻往前挪，或者让后一句晚点",
                        t.n, t.chars, t.needs, t.at, ends, next.n, next.at
                    ));
                    continue;
                }
            }
            if ends > deadline + TIMING_TOLERANCE {
                out.hard.push(format!(
                    "第 {} 句台词 {} 个字（自然语速要 {:.1} 秒），写的是第 {:.1} 秒开口 —— 要说到 {:.1} 秒，\
                     而这一镜只有 {:.0} 秒{}。把开口时刻往前挪到第 {:.1} 秒之前",
                    t.n, t.chars, t.needs, t.at, ends, ctx.shot.duration,
                    if tail > 0.0 { format!("（长镜头窗口，最后 {tail:.0} 秒不许说话）") } else { String::new() },
                    (deadline - t.needs).max(0.0)
                ));
            }
        }
        // **一次给出整套排得下的时刻。** 逐句报错会互相打架：第 2 句说「往前挪到 8.4 之前」，挪了又撞上第 1 句，
        // 第 1 句说「往前挪或者后一句晚点」—— 2026-09-29 gpt-5.6 写第 6 场第 13 镜四次在两条之间来回跳，
        // 而真正的解（第 1 句提前到 4.7）从没被说出来。从镜尾往前倒推每句最晚几秒开口，一次说清
        if out.hard.len() > before {
            if let Some(plan) = feasible_starts(&timings, deadline) {
                let s: Vec<String> = plan.iter().map(|(n, at)| format!("第 {n} 句 Around the {at:.1}-second mark")).collect();
                out.hard.push(format!("按字数排得下的一种开口时刻：{} —— 照这个写（可以更早，不能更晚；前一句说完才轮到下一句）", s.join("、")));
            } else {
                let total: f32 = timings.iter().map(|t| t.needs).sum();
                out.hard.push(format!("这一镜的台词按自然语速一共要 {total:.1} 秒，镜头只有 {deadline:.0} 秒 —— 怎么排都说不完，要回到镜头表拆镜"));
            }
        }
        // **「一个时间码都没写」只是软问题，不拦。**
        //
        // 它是 2026-09-20 才加进模板的新要求，模型会不会照做没验过。
        // 做成硬的话，模型不配合 = 整集废掉 —— 这个项目上一周刚为同样的
        // 形状（词数下限从 150 提到 320）废过 E05 八镜里的六镜。
        //
        // 先软着跑几轮，确认模型真的照写，再考虑收紧。
        if timings.is_empty() && !ctx.shot.dialogue_lines.iter().all(|l| l.line.trim().is_empty()) {
            out.soft.push(
                "台词前面没有 `Around the N.N-second mark,` 的开口时刻 —— 写上之后代码才能核对这一句说不说得完".into(),
            );
        }
    }
    for t in tags {
        if let Some(n) = t.subject {
            if !w.subjects.iter().any(|s| s.tag.trim() == format!("<Subject {n}>") && !s.look.trim().is_empty()) {
                out.hard.push(format!("<Subject {n}>（{}）没写外貌", t.name));
            }
        }
    }
    // 首帧出片：正文开头（第一个不是 00:00 的时间码之前）写了开场景别 / 机位 —— 写手看不到图，那是在猜（见 `tag_table` 的 OPENING FRAMING）
    if ctx.keyframe_video() {
        let said = opening_framing_words(desc);
        if !said.is_empty() {
            out.soft.push(format!(
                "首帧出片的开场构图就是 <Picture 1>，正文开头写了「{}」—— 你看不到首帧，写了就可能和图打架、出片时运镜卡顿。\
                 删掉开场的景别 / 机位 / 从哪起拍，只写镜头从首帧出发怎么动、停在哪 —— the opening framing is <Picture 1>; do not state the opening shot size or angle",
                said.join("、")
            ));
        }
    }
    if w.frame.trim().is_empty() {
        out.hard.push("frame（静帧提示词）是空的".into());
    }
    if w.overall_soundscape.trim().is_empty() {
        out.hard.push("overall_soundscape 是空的".into());
    }
    out.hard.extend(check(ctx, &RoutePrompts { video: video.to_string(), frame: w.frame.clone(), ..Default::default() }));
    out
}


/// 正文的词数区间，**按镜头时长算**。
///
/// # 为什么要有这一个函数
///
/// 2026-09-19 之前：模板无条件写「Length 350-500 English words」，
/// 而检查给 ≤6 秒的镜头卡 420 上限。**一个 450 词的 5 秒镜头既合模板又被程序拒**，
/// 错误反馈还叫它「压到 350~500 词」—— 模型照着改，改出来还是非法
/// （Codex Astra 的审查抓到）。
///
/// 三处（模板、检查、反馈）必须读**同一份数**，所以拎成一个函数，
/// 由 `{MIN_WORDS}` / `{MAX_WORDS}` 注进模板。
///
/// 数值按「每秒多少词」定：短镜头写太满，模型会把要紧的几句稀释在修辞里。
///
/// **上限跟官方走**（2026-10-06）：ref 指南 5.2「For generation tasks, `detailed_description` is normally 350-500 English words」。
/// 以前长镜头给到 620、按镜头表放宽后常见 700~950 词。
pub fn word_budget(secs: f32) -> (usize, usize) {
    if secs <= 6.0 {
        (220, 400)
    } else if secs <= 10.0 {
        (300, 500)
    } else {
        (350, 500)
    }
}

/// 镜头表写得再细，正文也不超过这个数。官方 500 词，多 60 留给说话人和语气那几句（台词本身不计词数）
pub const OFFICIAL_MAX_WORDS: usize = 560;

/// 镜头表这一镜写了多少字（画面 + 站位 + 运镜，去掉空白）。
///
/// **不算约束**：约束是「这一段必须保持不变的东西」，细化后每段都原样重复一遍
/// （「烟始终没点，护目镜始终在额头」× 5 段），算进去就把重复当成了细（2026-09-27 第 3 镜实跑）。
pub fn source_chars(segments: &[crate::shot_table::ShotSegment]) -> usize {
    segments
        .iter()
        .map(|seg| [&seg.visual, &seg.blocking, &seg.camera].iter().map(|t| t.chars().filter(|c| !c.is_whitespace()).count()).sum::<usize>())
        .sum()
}

/// 按**镜头表写了多少**放宽的词数区间。模板、检查、反馈都读这一份。
///
/// # 为什么
///
/// [`word_budget`] 只按时长给上限，而上限是硬的。镜头表写细之后（用户 2026-09-27：「描述细致，而不是一笔带过」），
/// 12 秒写到 Kotte 母版那种密度（每秒约 42 字 ≈ 500 字），翻成英文就要四五百词 ——
/// 固定的 620 词上限会逼写手把细节压回一句，前面写细等于白写。
///
/// 原文只算画面、站位、运镜（见 [`source_chars`]）。描述性的中文翻成英文大约一字一词（「嘴角挂着诡异、紧绷的微笑」11 字 ≈ 11 词），
/// 再留 150 词给开头的画风句、`Sound:` 和台词引子。**只放宽不收紧**：
/// 镜头表写得薄的镜头（现在 EP01 大多数）区间和以前一样。
/// 下限按原文六成抬一点（软的）：原文写了 500 字、正文只有 250 词，多半是压掉了。
pub fn word_budget_for(secs: f32, segments: &[crate::shot_table::ShotSegment]) -> (usize, usize) {
    let (lo, hi) = word_budget(secs);
    let src = source_chars(segments);
    let hi = hi.max(src * 11 / 10 + 150).min(OFFICIAL_MAX_WORDS);
    let lo = lo.max(src * 6 / 10).min(hi.saturating_sub(100));
    (lo, hi)
}

/// **只提示、不阻断**的那些。
///
/// # 为什么必须和 `check` 分开
///
/// `check_wording` 里有一句 `out.extend(check(...))` —— 所以任何放进
/// `check` 的东西都会让合成重试三次后**报错，整镜作废**。
///
/// 2026-09-19 我把表演指示检查放进了 `check`，注释却写着「只提示不阻断」
/// —— 两者正好相反，而且要到真的有一镜报废才看得出来（Codex Astra 的
/// 第二份审查抓到）。**注释挡不住调用链**，判据得在结构上分开。
///
/// 下面三条都是**启发式**：措辞对不上、正则找错了 `(SN)`、`or` 是程度副词，
/// 都会误报。误报在这里只是多一条提示；放进 `check` 就是白烧一次生成。
/// 正文里英文的二选一（`A or B`），回 `or` 前后一小段原文；没有、或者都是放过的写法回 `None`。
///
/// `advisories`（校验时，也覆盖用户手改过的）和 `check_wording`（合成时当软问题喂回写手）共用这一份。
/// 放过的写法见 `advisories` 里那段注释：`or so` / `or more` 这类程度副词，和否定句里的 `or`。
pub(crate) fn english_alternative(v: &str) -> Option<String> {
    let low = v.to_lowercase();
    let mut at = 0;
    while let Some(i) = low[at..].find(" or ") {
        let p0 = at + i;
        let tail = &low[p0 + 4..];
        let benign = ["so ", "more", "less", "two", "three", "fewer", "thereabouts"]
            .iter()
            .any(|w| tail.starts_with(w));
        // 同一句里、`or` 之前有否定词 → 它是「两样都不」，不是二选一
        let sentence_start = low[..p0].rfind(['.', ';', ':', '\n']).map(|i| i + 1).unwrap_or(0);
        let clause = &low[sentence_start..p0];
        let negated = ["never", "not ", "n't", " no ", "nor ", "without", "nothing", "nobody"]
            .iter()
            .any(|w| clause.contains(w));
        if !benign && !negated {
            // **按字符边界截**：正文里夹着 <d> 里的中文台词，按字节截会切在一个汉字中间直接崩
            let mut a = p0.saturating_sub(24);
            while !low.is_char_boundary(a) {
                a -= 1;
            }
            let mut b = (p0 + 28).min(low.len());
            while !low.is_char_boundary(b) {
                b += 1;
            }
            return Some(low[a..b].trim().to_string());
        }
        at = p0 + 4;
    }
    None
}

/// 二选一那条问题的原文（问题清单里的修复按钮按这句认，见 `cinema::issue_fixes`）
pub(crate) fn alternative_message(around: &str) -> String {
    format!("正文里有二选一「…{around}…」—— 只写一个确定的版本（模型会两边都画一点）")
}

/// 参考图里的**人**（场景、道具不算）：`(<Subject N>, 资产名)`。
fn character_subjects(tags: &[Tag]) -> Vec<(u32, String)> {
    tags.iter()
        .filter(|t| t.kind == Some(crate::assets::AssetKind::Character))
        .filter_map(|t| t.subject.map(|n| (n, t.name.clone())))
        .collect()
}

/// **人物基数句**：这一镜有几个人、每个人只出现一次（Ref2VA、两个人以上才加，代码写，不交给写手）。
///
/// 2026-10-06 EP03 第 2 镜：原生 Ref2VA 三条都把苏晴画成第二个星澜。H3 没有负向提示词（cfg 蒸馏过），
/// 只能正面说清「几个人、各是一个」。人物参考图是三视图卡 —— 一张图里同一个人出现三四次（用户 10-06 定了不换单人图），
/// 所以把「一张图从几个角度画同一个人，仍是一个人」也说出来。场景、道具、没挂图的人不算。
fn cast_note(tags: &[Tag]) -> Option<String> {
    let people = character_subjects(tags);
    if people.len() < 2 {
        return None;
    }
    let list: Vec<String> = people.iter().map(|(n, _)| format!("<Subject {n}>")).collect();
    Some(format!(
        "This shot features exactly {} different people: {}. Each of them is exactly one person who appears once, \
         keeps their own face, hair and clothes from start to end, and is never duplicated or blended with anyone else; \
         a character picture that shows a person from several angles still shows one person.",
        people.len(),
        list.join(", ")
    ))
}

/// 正文里把**人的颜色**写成被光改掉了（「her face becomes green-cast」）。回那一句的开头。
///
/// 2026-10-06 EP03 第 2 镜：「her initially pale face becomes visibly green-cast」—— 苏晴的脸被写成绿的，
/// 而星澜的配色正是银白 + 薄荷绿；原生 Ref2VA 那几条都把她画成了星澜。光要写成落在衣服、墙上，人的肤色发色不变。
/// 否定句（never / unchanged / remain）放过 —— 那正是推荐的写法。
fn identity_recolor(text: &str) -> Option<String> {
    const BODY: [&str; 5] = [" face", " skin", "complexion", " hair", " cheeks"];
    const TURN: [&str; 7] = ["turns", "turned", "becomes", "became", "goes", "tinted", "takes"];
    const COLOR: [&str; 12] = ["green", "blue", "red", "purple", "cyan", "teal", "pink", "orange", "yellow", "mint", "magenta", "violet"];
    const OK: [&str; 5] = ["never", "unchanged", "remain", "stays ", "not "];
    // **颜色按整词认，不按子串** —— 10-07 EP03 第 13 镜「silver hair sliding between」被判成改色：
    // `covered` / `layered` / `shared` 里都有 `red`。整词 + 「变成」那个词后面四个词以内才算
    let is_color = |w: &str| COLOR.iter().any(|c| w == *c || w.strip_prefix(c).is_some_and(|r| r.starts_with('-') || r == "ish"));
    // **角色定义那几行不查**：那是这个人本来的样子（10-07 星澜「silver-white hair with mint-tinted ends」—— 发梢本来就是薄荷色）
    let body: String = text.lines().filter(|l| !(l.trim_start().starts_with("<Subject ") && l.contains("> is "))).collect::<Vec<_>>().join("\n");
    for sentence in body.split(['.', ';', '\n']) {
        let l = format!(" {}", sentence.to_lowercase());
        if !BODY.iter().any(|b| l.contains(b)) || OK.iter().any(|o| l.contains(o)) {
            continue;
        }
        let words: Vec<&str> = l.split(|c: char| !c.is_ascii_alphabetic() && c != '-').filter(|w| !w.is_empty()).collect();
        // `-tinted` 不算：「mint-tinted ends」是长相；光照改色的说法是 `green-cast`
        let cast = words.iter().any(|w| w.ends_with("-cast") && is_color(w));
        let turned = words.iter().enumerate().any(|(i, w)| {
            TURN.contains(w) && (*w != "takes" || words.get(i + 1) == Some(&"on")) && words[i + 1..].iter().take(4).any(|x| is_color(x))
        });
        if cast || turned {
            return Some(sentence.trim().chars().take(80).collect());
        }
    }
    None
}

fn recolor_message(around: &str) -> String {
    format!(
        "正文把人的颜色写成被光改了（「{around}」）—— 视频会当成另一个人（EP03 第 2 镜苏晴被写成脸发绿，结果画成了第二个星澜）。\
         光写成落在衣服、墙上，并写明这个人自然的肤色和发色不变"
    )
}

/// 正文里的东南西北（`north` / `southern` / `eastward` / `west side` …）。回出现了几次和第一处附近的原文。
///
/// 2026-10-06 EP03 第 3 镜「车门朝西开」开进了墙里：H3 不知道画面里哪边是西。镜头表用中文方位保持场与场之间一致，
/// 写手要按这一段的机位翻成画面方向（左右、前后景、离镜头远近、某个固定物那一侧）。`East Asian` / `Southeast Asian` 不算。
fn compass_words(text: &str) -> Option<(usize, String)> {
    // 族裔、风格说法不是方位（Codex 复查 §23.1：`Western-style` 会误报）
    // 10-07 EP03 第 6 / 13 镜：画风那句「thirty-percent refined eastern stylization」被当方位拦下 —— 「东方 / 西方 + 风格类名词」都不算
    let mut lower = text.to_lowercase().replace("southeast asian", "ethnic").replace("east asian", "ethnic");
    for dir in ["eastern", "western", "east", "west"] {
        // 10-11 E05 第 3 镜（gpt-5.6-terra）：「refined eastern animation aesthetics」—— 画风里的「东方动画 / CG / 韵味」也不是方位
        for noun in [
            "style", "stylization", "stylisation", "aesthetic", "aesthetics", "fashion", "cut", "tailoring", "features", "art", "painting", "influence",
            "animation", "animated", "anime", "cg", "elegance", "beauty", "flavor", "flavour", "inspired", "sensibility", "design", "motifs", "charm",
        ] {
            lower = lower.replace(&format!("{dir}-{noun}"), noun).replace(&format!("{dir} {noun}"), noun);
        }
    }
    let mut hits = Vec::new();
    let mut word_start: Option<usize> = None;
    for (i, c) in lower.char_indices().chain(std::iter::once((lower.len(), ' '))) {
        match (c.is_ascii_alphabetic(), word_start) {
            (true, None) => word_start = Some(i),
            (false, Some(st)) => {
                let w = &lower[st..i];
                let core = w.strip_suffix("wards").or_else(|| w.strip_suffix("ward")).or_else(|| w.strip_suffix("ern")).unwrap_or(w);
                if matches!(core, "north" | "south" | "east" | "west" | "northeast" | "northwest" | "southeast" | "southwest") {
                    hits.push(st);
                }
                word_start = None;
            }
            _ => {}
        }
    }
    let first = *hits.first()?;
    let mut from = first.saturating_sub(40);
    while !lower.is_char_boundary(from) { from -= 1; }
    let mut to = (first + 40).min(lower.len());
    while !lower.is_char_boundary(to) { to += 1; }
    Some((hits.len(), lower[from..to].replace('\n', " ")))
}

fn compass_message(n: usize, around: &str) -> String {
    format!(
        "正文里有 {n} 处东南西北（「…{around}…」）—— H3 不知道画面里哪边是北（EP03 第 3 镜车门朝西开进了墙里）。\
         按这一段的机位翻成画面方向：画面左 / 右、前景 / 背景、离镜头近 / 远、朝向 / 背对镜头，或者某个固定物（巷口、铁皮门、砖墙）那一侧"
    )
}

/// 最终正文 subject_definitions 里每个 `<Subject N>` 的外貌写了几个词（去掉代码补的 `, as shown in <Picture N>`）。
fn look_words(video: &str, n: u32) -> Option<usize> {
    let head = format!("<Subject {n}> is ");
    let line = video.lines().find(|l| l.trim_start().starts_with(&head))?;
    let look = line.trim_start()[head.len()..].split(", as shown in <Picture").next().unwrap_or_default();
    if look.contains("exactly as shown in") {
        return None; // 代码兜底的「照图」，没有可比的
    }
    Some(look.split_whitespace().count())
}

pub fn advisories(ctx: &ShotContext, p: &RoutePrompts) -> Vec<String> {
    let mut adv = Vec::new();
    let v = &p.video;
    // **英文侧也要拒二选一。**
    //
    // 中文那边早就禁了「或者」「（或…）」，英文这边一直没查 ——
    // 实测 E04 第 9 镜写了「bronze **or** ceramic」，模板明确禁止而校验没拦住
    // （2026-09-19 Codex Astra 指出）。模型收到二选一会两边都画一点。
    //
    // 放 `check()` 不放 `check_wording()`：这里也覆盖**用户手改过**的提示词。
    //
    // `or so` / `or more` / `or two` 是程度副词，不是二选一，放过。
    //
    // **否定句里的 `or` 也放过**（2026-09-20 实跑抓到的误报）：
    // `the phones never move or fall` 说的是两样都不发生，是最确定的写法，
    // 恰恰不是「模型会两边都画一点」。同类还有 `without A or B`、
    // `no A or B`、`does not A or B`。
    //
    // 这条误报以前会**挡住整镜出片**（挂的字段是 `prompts`），
    // 现在只是多一行提示 —— 但一条每份正文都要挂的假提示，
    // 看几次之后就没人看真的那些了。
    if let Some(around) = english_alternative(v) {
        adv.push(alternative_message(&around));
    }
    // **正文里台词以外的中文会被 H3 念出来**（2026-10-06 EP03 第 5 镜：吴峥台词后面多念了一句
    // 「眼睛仍越过镜头许飞」，是补段时塞进去的中文舞台说明）。放这里也覆盖已经合成好的和用户手改的。
    if let Some(cjk) = stray_chinese(v) {
        adv.push(format!("正文里有台词以外的中文「{cjk}…」—— H3 会把它当台词念出来，重新合成这一镜（或手改成英文）"));
    }
    // **每句台词绑的是不是对的人。**
    //
    // 原来只查标签编号范围和台词出现次数，没核对「这句话前面写的是谁」。
    // 串了的话 H3 会给错的人配口型 —— 画面上是甲在动嘴，声音是乙的台词。
    // 当前样本没发现串台词，属于**验证缺口**（2026-09-19 Codex Astra 指出）。
    //
    // 判据：台词的 `<d>` 块往前找最近的 `(SN)`，它应当是代码给这一句定的那个。
    {
        let markers = line_markers(ctx, &tags(ctx));
        for (i, m) in markers.iter().enumerate() {
            let d = format!("<d>[Chinese] {}</d>", m.line);
            let Some(at) = v.find(&d) else { continue };
            // 往前看一段，找最近的 (SN)
            let before = &v[..at];
            let sn = before.rfind("(S").and_then(|p| {
                let tail = &before[p + 2..];
                tail.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse::<usize>().ok()
            });
            // 代码给这一句定的是第几个说话人
            let want = speakers(ctx, &tags(ctx))
                .iter()
                .position(|(n, _, _)| n == ctx.shot.dialogue_lines.iter().filter(|l| !l.line.trim().is_empty()).nth(i).map(|l| l.speaker.trim()).unwrap_or_default())
                .map(|p| p + 1);
            if let (Some(got), Some(want)) = (sn, want) {
                if got != want {
                    adv.push(format!(
                        "第 {} 句「{}」前面写的是 (S{got})，而它应该是 (S{want}) 说的 —— 说话人串了，出片会给错的人配口型",
                        i + 1,
                        m.line.chars().take(12).collect::<String>()
                    ));
                }
            }
        }
    }
    // **表演指示丢了要说出来。**
    //
    // 代码给的是 `<Subject 1> (S1) in a 40-year-old man's voice, <DELIVERY> says,`，
    // 模型该把 `<DELIVERY>` 换成英文措辞。它忘了的话 `strip_delivery` 会连着
    // 前面的逗号一起抹掉，留下**光秃秃的** `…voice says,` —— 那正好是
    // 「没有表演指示」的形状，可以直接认出来。
    //
    // 这一条**不写进 `check_wording`**（那里失败三次会让整镜报错，
    // 为一句措辞赔上一镜不值）。放在这里：合成照常成功，但界面上能看见
    // 「这一镜的表演指示没生效」。2026-09-19 之前是完全静默的 ——
    // Codex Astra 的审查点出 E03 第 3/5/6 镜、E04 第 6/10/12 镜都有这个退化。
    for l in ctx.shot.dialogue_lines.iter().filter(|l| !l.line.trim().is_empty()) {
        let tone = [l.voice.trim(), l.emotion.trim()].join("");
        if tone.is_empty() {
            continue;
        }
        let v0 = ctx.voice(&l.speaker);
        let voice_phrase = match (v0.female, v0.age) {
            (Some(f), Some(age)) => format!(" in a {age}-year-old {}'s voice", if f { "woman" } else { "man" }),
            (Some(f), None) => format!(" in a {}'s voice", if f { "woman" } else { "man" }),
            _ => String::new(),
        };
        if voice_phrase.is_empty() {
            continue;
        }
        if v.contains(&format!("{voice_phrase} {SAYS}")) || v.contains(&format!("{voice_phrase} {SAYS_VOICEOVER}")) {
            adv.push(format!(
                "「{}」的表演指示（{tone}）没写进提示词 —— 模型没换掉占位，这一句只剩性别年龄。重新合成这一镜可以再试一次",
                l.speaker.trim()
            ));
        }
    }
    // **换了装、没选造型**：镜头表说身份图不管服装，挂的却是主形象（EP03 星澜整集穿晚礼服，送进去的是银链纱裙那张）
    for (slot, asset) in ctx.slots() {
        if crate::prompt_input::outfit_mismatch(&slot, asset) {
            let name = asset.map(|a| a.name.as_str()).unwrap_or_default();
            adv.push(format!(
                "「{name}」镜头表写着不控制「{}」，但这一镜没选造型、挂的是主形象 —— 参考图上的衣服是错的。在镜头详情里给「{name}」选这一集的造型，再重新合成",
                slot.not_control.trim()
            ));
        }
    }
    // **多人镜头里谁的外貌写得薄、谁就容易被顶掉**（2026-10-06 EP03 第 2 镜：苏晴 33 词、星澜 95 词，原生 Ref2VA 把苏晴画成了星澜）
    let all_tags = tags(ctx);
    let people = character_subjects(&all_tags);
    if people.len() >= 2 {
        let counted: Vec<(&str, usize)> = people.iter().filter_map(|(n, name)| look_words(v, *n).map(|w| (name.as_str(), w))).collect();
        if let Some(&(rich, most)) = counted.iter().max_by_key(|(_, w)| *w) {
            for &(name, w) in &counted {
                if w < THIN_LOOK_WORDS && (w as f32) < most as f32 * THIN_LOOK_RATIO {
                    adv.push(format!(
                        "「{name}」的外貌只写了 {w} 个词，「{rich}」写了 {most} 个 —— 多人镜头里写得薄的人容易被写得厚的顶掉。\
                         重新合成这一镜；资产描述本身就薄的话，先在资产里补上脸型、发型发色、服装颜色款式"
                    ));
                }
            }
        }
        // **短镜头里人太多**：5 秒要绑六个参考人物、还要运镜和台词，是这一次串人的那种镜头
        if people.len() > MANY_PEOPLE && ctx.shot.duration <= SHORT_SHOT_SECS {
            adv.push(format!(
                "这一镜 {:.0} 秒里要绑 {} 个参考人物 —— 身份串的风险高。可以拆成主角镜头 + 反应镜头，或者只挂这一镜真正有戏的人",
                ctx.shot.duration,
                people.len()
            ));
        }
    }
    if let Some(around) = identity_recolor(v) {
        adv.push(recolor_message(&around));
    }
    if let Some((n, around)) = compass_words(v) {
        adv.push(compass_message(n, &around));
    }
    adv
}

/// 外貌词数少于这个、而且不到最厚那个的一半，算薄（EP03 第 2 镜苏晴 33 词 / 星澜 95 词）
const THIN_LOOK_WORDS: usize = 45;
const THIN_LOOK_RATIO: f32 = 0.5;
/// 超过这么多参考人物、镜头又不长于 [`SHORT_SHOT_SECS`]，提醒串人风险
const MANY_PEOPLE: usize = 3;
const SHORT_SHOT_SECS: f32 = 6.0;

/// 最终正文的结构检查（也给界面上改过的提示词用）。空 = 合格。
pub fn check(ctx: &ShotContext, p: &RoutePrompts) -> Vec<String> {
    let mut out = Vec::new();
    let v = &p.video;
    let slots = ctx.slots();
    let all_tags = tags(ctx);
    let m = mode(ctx, &all_tags);
    // 段落按官方模式查：基础模式三字段，Ref2VA 六段（见 `crate::h3_spec`）
    let heads: &[&str] = if m.is_base() {
        &["integrated_multimodal_description:", "overall_soundscape:", "non_diegetic_music:"]
    } else {
        &["subject_definitions:", "summary:", "retention_analysis:", "detailed_description:", "overall_soundscape:", "non_diegetic_music:"]
    };
    // 首帧 / 首尾帧：第一行必须是官方对齐指令（基础指南 2.1）
    let first_line = v.lines().next().unwrap_or_default();
    match m {
        crate::h3_spec::Mode::I2va if first_line != crate::h3_spec::I2VA_INSTRUCTION => out.push("首帧出片的第一行必须是官方 I2VA 对齐指令".into()),
        crate::h3_spec::Mode::Fl2va if !first_line.starts_with("How the reference pictures align with the target video — Picture 1 (from Shot 1)") => {
            out.push("首尾帧出片的第一行必须是官方 FL2VA 对齐指令".into())
        }
        _ => {}
    }
    let mut last = 0;
    for h in heads {
        match v.find(h) {
            Some(at) if at >= last => last = at,
            Some(_) => out.push(format!("段落顺序不对：「{h}」位置错了")),
            None => out.push(format!("缺少段落「{h}」")),
        }
    }
    if !v.trim_end().ends_with("non_diegetic_music: N/A") {
        out.push("最后一段必须是 non_diegetic_music: N/A（不写 H3 会自己配乐）".into());
    }
    // 标签：用到的都要在范围内
    let pictures = slots.len() as u32;
    // 主体数按 `tags` 算：基础模式（含首帧出片）一个主体都没有，写了 `<Subject N>` 就报
    let subjects = all_tags.iter().filter(|t| t.subject.is_some()).count() as u32;
    for (kind, max) in [("Picture", pictures), ("Subject", subjects)] {
        let pat = format!("<{kind} ");
        let mut rest = v.as_str();
        while let Some(at) = rest.find(&pat) {
            let tail = &rest[at + pat.len()..];
            let n: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(n) = n.parse::<u32>() {
                if n == 0 || n > max {
                    out.push(format!("用了没有定义的标签 <{kind} {n}>（这一镜只有 {max} 个）—— used undefined tag <{kind} {n}>; describe that person or thing in plain words"));
                }
            }
            rest = tail;
        }
    }
    // 台词：每一句都要逐字出现在 <d> 里。
    //
    // **按「块」整块比，不能把所有 <d> 拼成一串再数子串。**
    //
    // 2026-09-17 真卡死过《每日小离谱》E01 第 2 镜，重试三次全失败。
    // 那一镜的三句是：
    //
    //   [1]「全年打碎玻璃杯三个，陶瓷碗一个，花瓶……算半个。」
    //   [2]「半个？」
    //   [3]「它现在还立着，只是有点歪。」
    //
    // 归一化去标点之后，**短句「半个」同时是长句的一个子串**。旧写法把所有
    // <d> 块拼成一整串再 `matches().count()`，于是「半个」必然数到 2 —— 
    // 一次在长句里、一次是它自己 —— 被判成「模型自己又抄了一遍」。
    // **模型写得完全正确，是这条检查不可能被满足**，所以重试多少次都一样。
    //
    // 一句台词在同一镜里重复两遍是合法的（强调、回声），所以判据是
    // 「等于这句的块数 == 剧本里这句的条数」，不是「== 1」。
    let mut blocks: Vec<String> = Vec::new();
    let mut rest = v.as_str();
    while let Some(open) = rest.find("<d>") {
        let Some(close) = rest[open..].find("</d>") else { break };
        let body = &rest[open + 3..open + close];
        // 块里是 `[Chinese] 原话` —— **语言标签要先剥掉再比**。
        // 旧写法数子串时这个前缀无所谓，改成整块相等就必须处理，
        // 否则每一句都会被判成「没有逐字写进」。
        let body = match (body.trim_start().strip_prefix('['), body.find(']')) {
            (Some(_), Some(at)) => &body[at + 1..],
            _ => body,
        };
        blocks.push(spoken(body));
        rest = &rest[open + close + 4..];
    }
    // 剧本里每句台词各要几个块。保留一份原文用来写错误信息。
    let mut want: Vec<(String, &str, usize)> = Vec::new();
    for l in &ctx.shot.dialogue_lines {
        let key = spoken(&l.line);
        if key.is_empty() {
            continue;
        }
        match want.iter_mut().find(|(k, _, _)| *k == key) {
            Some((_, _, n)) => *n += 1,
            None => want.push((key, l.line.trim(), 1)),
        }
    }
    for (key, raw, n) in &want {
        let got = blocks.iter().filter(|b| *b == key).count();
        if got == 0 {
            out.push(format!("台词「{raw}」没有逐字写进 <d>[Chinese] …</d>"));
        } else if got > *n {
            // 模型自己又抄了一遍台词：出片会念两遍
            out.push(format!(
                "台词「{raw}」出现了 {got} 次（剧本里只有 {n} 句）—— 只写 [LINE N] 占位，不要自己写台词"
            ));
        } else if got < *n {
            out.push(format!(
                "台词「{raw}」在剧本里有 {n} 句，<d> 里只写了 {got} 次"
            ));
        }
    }
    if p.frame.trim().is_empty() {
        out.push("分镜提示词是空的".into());
    }
    out
}

/// 合成这一镜。LM 措辞不合格就带着问题重写，最多重试 [`RETRIES`] 次。
///
/// 回 `(提示词, 还差点意思的地方)`。第二项非空 = **这一镜能出片，但有话要说**
/// —— 词数偏薄、正文里漏了几个汉字。它跟着合成结果的 notes 交给用户。
///
/// # 为什么不是「不合格就报错」
///
/// 原来是。2026-09-19 实跑 E05 八镜废了六镜，全废在「词数没到推荐区间」上，
/// 而那个区间是同一天刚调高的。理由和分档见 [`Problems`]。
/// 硬问题（台词没逐字、用了不存在的标签、缺段落）仍然报错，**不拿废的存下来**。
pub async fn synthesize<F, Fut>(ctx: &ShotContext<'_>, call: F, now: u64) -> Result<(RoutePrompts, Vec<String>)>
where
    F: Fn(crate::prompts::PromptSlot, Vec<(String, String)>) -> Fut,
    Fut: Future<Output = Result<String>>,
{
    // **长镜头 + 接上一镜：在叫模型之前就拦下。**
    //
    // 这两个设置是互斥的：长镜头的衔接靠窗口之间的重叠帧，而
    // `run_long_take` 不绑任何视频输入。合出来的正文里那句
    // `<Video 1> is the end of the immediately preceding shot` 会指向
    // 一个不存在的输入，**而且不报错**。
    //
    // **不放进 `hard`**：那一档是喂回模型让它改写的，而这是用户的设置，
    // 模型改不了 —— 一条模型满足不了的检查，比没有检查更糟。
    if ctx.long_take && ctx.shot.effective_continuity() != crate::cinema::Continuity::None {
        return Err(Error::config(format!(
            "第 {} 镜在整段出的场里，但「接上一镜」选的是「{}」—— 长镜头的衔接靠窗口之间的重叠帧，不用参考视频。把这一镜改成「不接」再合成",
            ctx.shot.shot_num,
            ctx.shot.effective_continuity().label(),
        )));
    }
    let tags = tags(ctx);
    let h3_mode = mode(ctx, &tags);
    let mut markers = line_markers(ctx, &tags);
    fill_delivery(&mut markers, &call).await;
    let mut prev = ctx
        .prev
        .and_then(|p| p.segments.last().map(|s| s.visual.trim().to_string()))
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "This is the first shot of the episode.".into());
    // 同一场：光线、服装、道具位置要接得上（实跑镜与镜之间不连贯）
    if let Some(p) = ctx.same_scene_prev() {
        prev.push_str(&format!(
            "\nSame scene, continuing directly: open this shot from that end state, keep the same lighting ({}), the same prop positions, and every costume color exactly as in the reference pictures.",
            p.lighting.trim()
        ));
    }
    let vars = |feedback: &str| -> Vec<(String, String)> {
        vec![
            ("SECS".into(), format!("{}", ctx.shot.duration)),
            ("SHOT".into(), shot_brief(ctx)),
            ("TAGS".into(), tag_table(&tags)),
            ("SPEAKERS".into(), speakers_instruction(&markers)),
            ("PREV".into(), prev.clone()),
            // 怎么把定好的镜头译成提示词：cinematic 原文（分工版，见 `cinematic::compile_method`）
            ("METHOD".into(), crate::cinematic::compile_method()),
            // **这种生产方式对应的官方规范原文**（2026-10-06）：首帧 = I2VA、首尾帧 = FL2VA、文生 = T2VA、参考 = Ref2VA
            ("H3_MODE".into(), h3_mode.label().to_string()),
            ("H3_SPEC".into(), crate::h3_spec::writer_rules(h3_mode)),
            // **词数和检查读同一份数**（见 `word_budget` 的注释）
            ("MIN_WORDS".into(), word_budget_for(ctx.shot.duration, &ctx.shot.segments).0.to_string()),
            ("MAX_WORDS".into(), word_budget_for(ctx.shot.duration, &ctx.shot.segments).1.to_string()),
            ("FEEDBACK".into(), feedback.to_string()),
        ]
    };
    let mut feedback = String::new();
    let mut last_problems = Vec::new();
    // 没有硬问题、只剩软问题的那一份。**留着** —— 重试用完就交它，
    // 而不是让整镜作废。软问题最少的那一份胜出。
    // 只剩软问题的那几份里挑哪份：**先比漏了几段**，再比软问题几条 —— 漏段比「词数略少」要紧得多
    // 排序键：（正文夹中文, 漏几段, 软问题几条），越小越好
    type Rank = (bool, usize, usize);
    let mut best: Option<(RoutePrompts, Vec<String>, Wording, Rank)> = None;
    for _ in 0..=RETRIES {
        let raw = call(crate::prompts::PromptSlot::RouteBWriter, vars(&feedback)).await.map_err(|e| format!("模型调用失败：{e}"));
        // 被内容过滤拦了不重试（见 `prompts::is_content_filter`）：有一份能用的就交它，没有就直接报
        if let Err(e) = &raw {
            if crate::prompts::is_content_filter(e) {
                let earlier: Vec<String> = last_problems.drain(..).filter(|m: &String| !m.starts_with("这一次：")).collect();
                last_problems = std::iter::once(format!("这一次：{e}")).chain(earlier).collect();
                break;
            }
        }
        match raw.as_ref().map_err(Clone::clone).and_then(|raw| parse_wording(raw)) {
            Ok(mut w) => {
                let filled = fill_missing_looks(&tags, &mut w);
                // 关键帧出片走 FL2VA 分区，那里不接参考视频（见 `weetodd::build_h3_graph`）——
                // 写了 `<Video 1>` 就是指向一个不存在的输入
                let continuity = if ctx.keyframe_video() { crate::cinema::Continuity::None } else { ctx.shot.effective_continuity() };
                let video = assemble(ctx, &tags, &markers, &w, continuity);
                let mut problems = check_wording(ctx, &tags, &w, &video);
                // 补上的那几个，控制范围是代码给的 —— 「外貌或控制范围写了中文」这一条不再算写手的账
                problems.soft.retain(|m| !filled.iter().any(|(n, _)| m.starts_with(&format!("<Subject {n}> 的外貌或控制范围写了中文"))));
                let filled_notes: Vec<String> = filled
                    .iter()
                    .map(|(n, name)| format!("第 {} 镜：写手没写 <Subject {n}>（{name}）的外貌，已补成「照 <Picture> 里那样」（不写中文描述，H3 会念出来）", ctx.shot.shot_num))
                    .collect();
                if problems.hard.is_empty() {
                    let p = RoutePrompts { frame: w.frame.trim().to_string(), video, synthesized_at: now, fields_hash: fields_hash(ctx) };
                    if problems.soft.is_empty() {
                        return Ok((p, filled_notes));
                    }
                    let notes: Vec<String> = problems
                        .soft
                        .iter()
                        .map(|m| format!("第 {} 镜：{m}", ctx.shot.shot_num))
                        .chain(filled_notes.iter().cloned())
                        .collect();
                    let missing = segment_coverage(&ctx.shot.segments, &w.detailed_description).len();
                    // 挑哪一份：**正文没有会被念出来的中文** > 不漏段 > 软问题少。
                    // 漏一段动作是画面少一拍，夹一句中文是 H3 多念一句（EP03 第 5 镜）—— 后者更糟
                    let key = (stray_chinese(&p.video).is_some(), missing, notes.len());
                    if best.as_ref().map_or(true, |(_, _, _, k)| key < *k) {
                        best = Some((p, notes, w.clone(), key));
                    }
                }
                // 不合格时把模型原文整段写进日志：界面上只有一句「台词没进 <d>」，看不到原文就不知道它写成了什么
                tracing::warn!(shot = ctx.shot.shot_num, hard = ?problems.hard, soft = ?problems.soft, raw = raw.as_deref().unwrap_or_default(), "线路 B 合成有问题，模型原文附后");
                last_problems = problems.all();
            }
            Err(e) => {
                // **解析不了的那一次不能把前几次的真实问题盖掉**（2026-09-30 EP02 第 6 镜）：界面上只剩
                // 「返回里没有 JSON」，看不出前三次都卡在「台词排不开」和「词数超了」。前几次的照样留着、照样喂回去
                let head: String = raw.as_deref().unwrap_or_default().chars().take(2000).collect();
                tracing::warn!(shot = ctx.shot.shot_num, error = %e, raw = %head, "线路 B 合成这一次的返回用不了，原文附后");
                let earlier: Vec<String> = last_problems.drain(..).filter(|m| !m.starts_with("这一次：")).collect();
                last_problems = std::iter::once(format!("这一次：{e}")).chain(earlier).collect();
            }
        }
        feedback = format!("## Problems in your last answer (fix all of them)\n{}", last_problems.join("\n"));
    }
    // 重试用完：有一份只剩软问题的就交它，带着话
    if let Some((p, notes, w, (_, missing, _))) = best {
        if missing == 0 {
            return Ok((p, notes));
        }
        // **还漏段也交写手这一份，不补中文**（2026-10-06 EP03 第 5 镜）。
        // 以前按镜头表的中文原文补一句 `At 00:04.0, 约第4秒吴峥开口，偏头幅度很小…`，理由是「H3 读得懂中文」——
        // 读得懂，而且**会念出来**：项目自己的 Whisper 转写里，吴峥那句台词后面紧跟着「眼睛仍越过镜头许飞」。
        // H3 的编码器把正文里的中文都当成要说的话，漏一段动作远没有一句乱念的旁白糟。
        let which = segment_coverage(&ctx.shot.segments, &w.detailed_description).iter().map(|n| format!("第 {n} 段")).collect::<Vec<_>>().join("、");
        let mut notes: Vec<String> = notes.into_iter().filter(|m| !m.contains("没写到")).collect();
        notes.push(format!(
            "第 {} 镜：写手重试 {RETRIES} 次都没写到{which}，这一段的动作可能被压进前后段或丢了 —— 没有补中文（H3 会把正文里的中文当台词念出来）；介意就重新合成",
            ctx.shot.shot_num
        ));
        return Ok((p, notes));
    }
    if last_problems.first().is_some_and(|m| crate::prompts::is_content_filter(m)) {
        return Err(Error::runtime(format!("第 {} 镜线路 B 合成被内容过滤拦下（没有重试）：{}", ctx.shot.shot_num, last_problems.join("；"))));
    }
    Err(Error::runtime(format!("第 {} 镜线路 B 合成没通过检查（重试过 {RETRIES} 次）：{}", ctx.shot.shot_num, last_problems.join("；"))))
}

#[cfg(test)]
mod voice_direction_live {
    /// 2026-09-29 第 6 场第 13 镜：两句台词在 13 秒里按写的时刻排不下，逐句报错互相打架。
    /// 倒推出的整套时刻要两条都满足
    #[test]
    fn 台词排不下时给出整套能排下的开口时刻() {
        let t = |n: usize, at: f32, chars: usize| super::LineTiming { n, at, chars, needs: chars as f32 / super::SPOKEN_CPS };
        let lines = [t(1, 5.0, 15), t(2, 8.3, 19)];
        let plan = super::feasible_starts(&lines, 13.0).unwrap();
        assert!((plan[0].1 - 4.71).abs() < 0.02 && (plan[1].1 - 8.37).abs() < 0.02, "{plan:?}");
        // 照着排：前一句说完才轮到下一句，最后一句在镜尾前说完（容差和核对用同一个）
        let r = |x: f32| (x * 10.0).round() / 10.0;
        assert!(r(plan[0].1) + lines[0].needs <= r(plan[1].1) + super::TIMING_TOLERANCE);
        assert!(r(plan[1].1) + lines[1].needs <= 13.0 + super::TIMING_TOLERANCE);
        // 本来就排得下的，时刻不动
        let ok = [t(1, 1.0, 8), t(2, 5.0, 8)];
        assert_eq!(super::feasible_starts(&ok, 13.0).unwrap(), vec![(1, 1.0), (2, 5.0)]);
        // 台词比镜头还长：排不下
        assert!(super::feasible_starts(&[t(1, 0.0, 60)], 13.0).is_none());
    }

    /// **2026-10-03 EP02 第 10 镜的回归。** 建议时刻要四舍五入成一位小数才写得进正文，
    /// 前一句往上舍（2.18 → 2.2）、后一句往下舍（4.13 → 4.1），照着写也撞了 ——
    /// 三次重试全卡在这里。照建议写出来的时刻必须过得了核对。
    #[test]
    fn 照着建议写的时刻一定过得了核对() {
        let t = |n: usize, at: f32, chars: usize| super::LineTiming { n, at, chars, needs: chars as f32 / super::SPOKEN_CPS };
        let r = |x: f32| (x * 10.0).round() / 10.0;
        let lines = [t(1, 2.18, 8), t(2, 4.08, 10)];
        let plan = super::feasible_starts(&lines, 9.0).unwrap();
        let written: Vec<f32> = plan.iter().map(|(_, at)| r(*at)).collect();
        assert_eq!(written, vec![2.2, 4.1], "这就是当时给的建议");
        assert!(written[0] + lines[0].needs <= written[1] + super::TIMING_TOLERANCE, "照建议写还是撞：{written:?}");
        // 扫一片：任何两句、任何写法，建议值舍成一位小数后都要过
        for a10 in 0..40 {
            for gap10 in 0..30 {
                for c in 1..20 {
                    let a = a10 as f32 / 10.0 + 0.03;
                    let ls = [t(1, a, c), t(2, a + gap10 as f32 / 10.0, 6)];
                    let Some(p) = super::feasible_starts(&ls, 15.0) else { continue };
                    let w: Vec<f32> = p.iter().map(|(_, at)| r(*at)).collect();
                    assert!(w[0] + ls[0].needs <= w[1] + super::TIMING_TOLERANCE, "{ls:?} → {w:?}");
                }
            }
        }
    }

    /// 实跑：直接调一次语气翻译槽位，看本机后台模型回什么、能不能解析。
    #[tokio::test]
    #[ignore = "要本机后台模型"]
    async fn 语气翻译实跑() {
        let config_dir = dirs::config_dir().unwrap().join("com.anima.desktop");
        let app_cfg = crate::config::AppConfig::load(&config_dir.join("config.json")).unwrap();
        let gateway = crate::llm::gateway::Gateway::new(app_cfg).unwrap();
        let cfg = crate::prompts::load(&config_dir);
        let items = "1. 二十出头的年轻女声，音色清亮柔和，干净得近乎没有瑕疵，吐字格外标准，像精心调校过的播报音；语速平稳\n2. 平静中带一丝了然，语速正常，尾音放轻";
        let raw = crate::prompts::run(&gateway, &cfg, crate::prompts::PromptSlot::VoiceDirection, &[("ITEMS", items)]).await;
        println!("原文：{raw:?}");
        println!("解析：{:?}", raw.as_deref().ok().and_then(|r| super::parse_voice_directions(r, 2)));
    }
}

#[cfg(test)]
mod voice_direction_tests {
    use super::*;

    #[test]
    fn 解析语气翻译_条数和语言都要对() {
        assert_eq!(parse_voice_directions(r#"{"items": ["low, slightly smoky male voice", "soft, slow, nearly a whisper."]}"#, 2),
            Some(vec!["low, slightly smoky male voice".into(), "soft, slow, nearly a whisper".into()]));
        assert!(parse_voice_directions(r#"{"items": ["only one"]}"#, 2).is_none(), "条数对不上");
        assert!(parse_voice_directions(r#"{"items": ["温柔地说"]}"#, 1).is_none(), "不许留中文");
        assert!(parse_voice_directions("not json", 1).is_none());
        // 实跑时星澜的音色译成 25 个词、内容全对 —— 不能因为长就整份作废
        let long = "Young female voice in her early twenties, clear and soft timbre, flawlessly clean with especially standard diction like a finely tuned broadcast tone, steady pace";
        assert!(parse_voice_directions(&serde_json::json!({"items": [long]}).to_string(), 1).is_some());
    }

    fn marker(timbre: &str, delta: &str) -> LineMarker {
        LineMarker {
            marker: "[LINE 1]".into(),
            who: format!("<Subject 1> (S1) in a 26-year-old man's voice, {DELIVERY}"),
            line: "知道了。".into(),
            tone: format!("{timbre}，{delta}"),
            timbre: timbre.into(),
            delta: delta.into(),
            verb: SAYS,
            lips: String::new(),
        }
    }

    /// 《回收师》E01：42 句台词只有 1 句带上了语气，其余的 <DELIVERY> 被静默删掉。
    #[tokio::test]
    async fn 翻好的语气由代码拼进台词前_音色走缓存() {
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let c = calls.clone();
        let call = move |_slot: crate::prompts::PromptSlot, vars: Vec<(String, String)>| {
            let c = c.clone();
            async move {
                let items = vars[0].1.clone();
                c.lock().unwrap().push(items.clone());
                let n = items.lines().count();
                let out: Vec<String> = (0..n).map(|i| format!("phrase number {i}")).collect();
                Ok(serde_json::json!({ "items": out }).to_string())
            }
        };
        let timbre = "测试专用：二十六岁男声，烟嗓";
        let mut ms = vec![marker(timbre, "平静，语速正常"), marker(timbre, "温柔，近乎耳语")];
        fill_delivery(&mut ms, &call).await;
        assert!(ms.iter().all(|m| !m.who.contains(DELIVERY)), "占位要被填掉：{:?}", ms.iter().map(|m| &m.who).collect::<Vec<_>>());
        assert!(ms[0].who.starts_with("<Subject 1> (S1) in a 26-year-old man's voice, phrase number"));
        // 两句的音色部分是同一串
        let timbre_en = voice_cache().lock().unwrap().get(timbre).cloned().unwrap();
        assert!(ms[0].who.contains(&timbre_en) && ms[1].who.contains(&timbre_en));
        assert_eq!(calls.lock().unwrap()[0].lines().count(), 3, "音色只翻一次：1 条音色 + 2 条语气");
        // 下一镜同一个角色：音色不再翻
        let mut next = vec![marker(timbre, "急促，压低声音")];
        fill_delivery(&mut next, &call).await;
        assert_eq!(calls.lock().unwrap()[1].lines().count(), 1);
        assert!(next[0].who.contains(&timbre_en));
    }

    /// 翻译失败：什么都不改，退回原来的「写手顺手翻」（占位还在）
    #[tokio::test]
    async fn 翻译失败就保持原样() {
        let call = |_s: crate::prompts::PromptSlot, _v: Vec<(String, String)>| async { Ok::<String, crate::error::Error>("乱七八糟".into()) };
        let mut ms = vec![marker("测试专用：翻不出来的音色", "翻不出来的语气")];
        fill_delivery(&mut ms, &call).await;
        assert!(ms[0].who.contains(DELIVERY));
    }
}

#[cfg(test)]
mod desc_limit_tests {
    use super::*;
    use crate::storyboard_gen::AssetRef;

    /// 《回收师》星澜的真实描述（198 字）：截到 120 会丢掉协议铭牌和银色接缝。
    #[test]
    fn 主角的标志性细节不被截掉() {
        let d = "描述：零号原型机，云穹伴侣型旗舰款外壳的陪伴机器人，女性，视觉年龄二十二岁左右，身高168厘米，身形匀称，银白色长直发，垂到腰，银白色，鹅蛋脸，五官柔和，浅粉色瞳孔（出厂模式），近乎完美的瓷白。\n服装：上衣粉白配色、柔光面料的出厂陪伴服，胸口嵌着椭圆形协议铭牌，下装同色配套下装，配套鞋履，无帽，关节处有细细的银色接缝，锁骨下方、指节、手腕处有极细的银色接缝。";
        let a = AssetRef { id: "c".into(), kind: crate::assets::AssetKind::Character, name: "星澜".into(), has_image: true, description: d.into(), look_id: String::new(), looks: Vec::new(), h3_voice: String::new() };
        let got = short_description(&a, CHAR_DESC_MAX);
        assert!(got.contains("协议铭牌"), "铭牌被截掉了：{got}");
        assert!(!short_description(&a, 120).contains("协议铭牌"), "对照：旧的 120 确实会截掉 —— 这条测试钉的就是这件事");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::AssetKind;
    use crate::cinema::{DialogueLine, StoryboardShot};
    use crate::shot_table::{RefSlot, ShotSegment};
    use crate::storyboard_gen::AssetRef;
    use std::sync::Mutex;

    fn assets() -> Vec<AssetRef> {
        vec![
            AssetRef { id: "char_shen".into(), kind: AssetKind::Character, name: "林婉清（盛唐）".into(), has_image: true, description: "描述：盛唐女官，女性，28岁".into(), look_id: String::new(), looks: Vec::new(), h3_voice: String::new() },
            AssetRef { id: "char_emperor".into(), kind: AssetKind::Character, name: "皇帝".into(), has_image: true, description: "描述：盛唐皇帝，男性，40岁".into(), look_id: String::new(), looks: Vec::new(), h3_voice: String::new() },
            AssetRef { id: "scene_palace".into(), kind: AssetKind::Scene, name: "金銮殿".into(), has_image: true, description: "内景，盛唐金銮殿".into(), look_id: String::new(), looks: Vec::new(), h3_voice: String::new() },
        ]
    }

    fn shot() -> StoryboardShot {
        let mut s: StoryboardShot = serde_json::from_value(serde_json::json!({
            "id": "s6", "shot_num": 6, "scale": "medium", "camera_movement": "push_in", "dialogue": "",
            "prompt": "", "duration": 10.0, "status": "pending"
        }))
        .unwrap();
        s.camera_angle = "low_angle".into();
        s.character_ids = vec!["char_emperor".into(), "char_shen".into()];
        s.scene_id = Some("scene_palace".into());
        s.lighting = "龙椅处最亮，皇帝面部半阴影".into();
        s.segments = vec![ShotSegment { start: 0.0, end: 10.0, title: "圣上下令".into(), visual: "皇帝抬手，林婉清跪地仰望".into(), ..Default::default() }];
        s.dialogue_lines = vec![DialogueLine { speaker: "皇帝".into(), line: "既言才华盖世，当殿作诗。若不能——斩。".into(), ..Default::default() }];
        s.ref_slots = vec![
            RefSlot { order: 1, asset_id: "char_emperor".into(), usage: RefUsage::Identity, control: "方脸、龙袍、帝冠".into(), not_control: "姿势、表情".into() },
            RefSlot { order: 2, asset_id: "char_shen".into(), usage: RefUsage::Identity, control: "瓜子脸、男装官服".into(), not_control: "姿势".into() },
            RefSlot { order: 3, asset_id: "scene_palace".into(), usage: RefUsage::Geography, control: "龙椅位置、立柱".into(), not_control: "光线".into() },
        ];
        s
    }

    /// 把一段 detailed_description 包成结构合格的六段正文 ——
    /// 不包的话 `check` 先在「缺少段落」上报一堆，测不到要测的那条。
    fn full(desc: &str) -> RoutePrompts {
        let v = format!(
            "subject_definitions:\n<Subject 1>: an emperor\n<Subject 2>: a woman\n<Subject 3>: a hall\n\
             summary:\nThe emperor speaks.\n\
             retention_analysis:\n<Picture 1> fully_preserved\n\
             detailed_description:\n{desc}\n\
             overall_soundscape:\nA silent hall.\n\
             non_diegetic_music: N/A"
        );
        RoutePrompts { video: v, frame: "a still".into(), ..Default::default() }
    }

    fn long_desc() -> String {
        let filler = "The camera pushes in slowly while the hall stays silent and the light holds steady on the throne. ".repeat(20);
        format!("A realistic cinematic style with warm gold light. [Shot 1] A low-angle medium shot frames <Subject 1> on the throne above <Subject 2>. {filler} <Subject 1> (S1) says, [LINE 1] Sound: the robe rustles.")
    }

    fn good_reply() -> String {
        serde_json::json!({
            "subjects": [
                {"tag": "<Subject 1>", "look": "a 40-year-old emperor with a square face", "control": "square face, dragon robe, crown", "not_control": "pose, expression"},
                {"tag": "<Subject 2>", "look": "a slim 28-year-old woman in a male official's robe", "control": "oval face, official robe", "not_control": "pose"},
                {"tag": "<Subject 3>", "look": "a vast palace hall with tall pillars", "control": "throne position, pillars", "not_control": "light"}
            ],
            "summary": "The emperor orders <Subject 2> to compose a poem.",
            "detailed_description": long_desc(),
            "overall_soundscape": "A silent vast hall with faint rustling robes.",
            "frame": "Low-angle medium shot of an emperor on a throne in a golden palace hall."
        })
        .to_string()
    }

    /// 首帧 / 首尾帧出片（官方 I2VA / FL2VA）的写手回复：没有 <Subject N>，人按首帧里的位置称呼
    fn keyframe_reply() -> String {
        let mut r: serde_json::Value = serde_json::from_str(&good_reply()).unwrap();
        r["subjects"] = serde_json::json!([]);
        let d = r["detailed_description"].as_str().unwrap()
            .replace("<Subject 1> (S1)", "the 40-year-old man shown in <Picture 1> (S1)")
            .replace("<Subject 1>", "the emperor on the throne in <Picture 1>")
            .replace("<Subject 2>", "the kneeling woman in <Picture 1>")
            .replace("<Subject 3>", "the hall");
        r["detailed_description"] = serde_json::json!(d);
        r["summary"] = serde_json::json!("The emperor orders the woman to compose a poem.");
        r.to_string()
    }

    struct FakeLm {
        replies: Mutex<Vec<String>>,
        seen: Mutex<Vec<Vec<(String, String)>>>,
        /// 语气翻译那一步的回复（见 `fill_delivery`）。**单独一条队列、不记进 `seen`**：
        /// 没给就当翻译失败 —— 老测试照旧只看写手那一次调用
        voice: Mutex<Vec<String>>,
    }
    impl FakeLm {
        fn new(r: Vec<String>) -> Self {
            Self { replies: Mutex::new(r.into_iter().rev().collect()), seen: Mutex::new(Vec::new()), voice: Mutex::new(Vec::new()) }
        }
        async fn call(&self, slot: crate::prompts::PromptSlot, vars: Vec<(String, String)>) -> Result<String> {
            if slot == crate::prompts::PromptSlot::VoiceDirection {
                return self.voice.lock().unwrap().pop().ok_or_else(|| Error::runtime("没给翻译回复"));
            }
            self.seen.lock().unwrap().push(vars);
            self.replies.lock().unwrap().pop().ok_or_else(|| Error::runtime("没有更多回复"))
        }
    }

    /// **2026-10-03 EP02 第 22 镜。** 写手（本机 ornith）几次都没给场景主体写 look，
    /// 硬检查「没写外貌」让整镜作废、用户合成了好几轮。漏了就按资产描述补，不重试、不作废
    #[tokio::test]
    async fn 写手漏写主体外貌_按资产描述补上() {
        let s = shot();
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let mut r: serde_json::Value = serde_json::from_str(&good_reply()).unwrap();
        // 第 3 个主体（场景）的 look 空着 —— 有的模型干脆整条不写，两种都要补
        r["subjects"][2]["look"] = serde_json::json!("");
        let lm = FakeLm::new(vec![r.to_string()]);
        let (p, notes) = synthesize(&ctx, |slot, v| lm.call(slot, v), 7).await.expect("漏写外貌不该让整镜作废");
        assert!(notes.iter().any(|m| m.contains("<Subject 3>") && m.contains("已补成")), "{notes:?}");
        assert!(!notes.iter().any(|m| m.contains("写了中文")), "{notes:?}");
        // **补英文「照图」，不补资产的中文描述**：H3 会把正文里的中文念出来（2026-10-06）
        let scene = a.iter().find(|x| x.kind == crate::assets::AssetKind::Scene).unwrap();
        let first = scene.description.chars().filter(|c| !c.is_whitespace()).take(6).collect::<String>();
        assert!(!p.video.contains(&first), "资产的中文描述进了正文：{first}");
        assert!(p.video.contains("is the place exactly as shown in <Picture"), "{}", p.video);
        // 控制范围也不抄镜头表的中文（EP03 第 11、12 镜：「南北距离、铁皮屋和巷口方位」进了主体定义和 retention）
        assert_eq!(stray_chinese(&p.video), None, "补外貌后正文里还有中文：{}", p.video);

        // 整条主体都没写，也补
        let mut r2: serde_json::Value = serde_json::from_str(&good_reply()).unwrap();
        r2["subjects"].as_array_mut().unwrap().remove(2);
        let lm2 = FakeLm::new(vec![r2.to_string()]);
        let (p2, notes2) = synthesize(&ctx, |slot, v| lm2.call(slot, v), 7).await.expect("整条漏写也该补上");
        assert!(notes2.iter().any(|m| m.contains("<Subject 3>")), "{notes2:?}");
        // 整条漏写：控制范围也是代码补的，不能抄镜头表里的「龙椅位置、立柱」
        assert_eq!(stray_chinese(&p2.video), None, "{}", p2.video);
        assert!(p2.video.contains("<Subject 3> (appears in [Shot 1]): partially_preserved - the spatial layout, furnishings and materials."), "{}", p2.video);
    }

    #[tokio::test]
    async fn 六段结构_编号和挂图顺序一致_台词在_d_标签里() {
        let s = shot();
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let lm = FakeLm::new(vec![good_reply()]);
        let (p, _soft) = synthesize(&ctx, |slot, v| lm.call(slot, v), 7).await.unwrap();
        let v = &p.video;
        let order: Vec<usize> = ["subject_definitions:", "summary:", "retention_analysis:", "detailed_description:", "overall_soundscape:", "non_diegetic_music: N/A"].iter().map(|h| v.find(h).unwrap()).collect();
        assert!(order.windows(2).all(|w| w[0] < w[1]), "段序不对：{v}");
        // 官方写法（ref 指南 2.1）：一行一个标签，图的出处写进定义里；控制范围进 retention。没有非官方的 only defines / does not define
        assert!(v.contains("<Subject 1> is a 40-year-old emperor with a square face, as shown in <Picture 1>."), "{v}");
        assert!(v.contains("<Subject 1> (appears in [Shot 1]): fully_preserved - square face, dragon robe, crown."), "{v}");
        assert!(!v.contains("only defines") && !v.contains("does not define pose"), "{v}");
        assert!(v.contains("<Subject 3> is a vast palace hall with tall pillars, as shown in <Picture 3>."), "{v}");
        let defs = &v[..v.find("summary:").unwrap()];
        assert!(!defs.chars().any(is_cjk), "有英文外貌时主体定义里不该夹中文名：{defs}");
        assert_eq!(lower_first("Oval face"), "oval face");
        assert_eq!(lower_first("TV screen"), "TV screen", "缩写不改");
        assert!(v.contains("summary:\n[reference generation] The emperor orders"), "{v}");
        assert!(v.contains("<Subject 1> (appears in [Shot 1]): fully_preserved - square face, dragon robe, crown."), "{v}");
        assert!(v.contains("<Subject 3> (appears in [Shot 1]): partially_preserved - throne position, pillars."), "地理只保空间关系：{v}");
        assert!(v.trim_end().ends_with("non_diegetic_music: N/A"));
        assert_eq!(check(&ctx, &p), Vec::<String>::new());
        assert_eq!((p.synthesized_at, p.fields_hash.as_str()), (7, fields_hash(&ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false }).as_str()));
        // 台词是代码填进去的：占位换成逐字原话，只出现一次
        // 写手写成 `(S1) says,`，代码按占位表补回完整说话人（v0.43.38，见 `fill_lines`）
        assert!(v.contains("<Subject 1> (S1) in a 40-year-old man's voice says, <d>[Chinese] 既言才华盖世，当殿作诗。若不能——斩。</d> Sound:"), "{v}");
        assert!(!v.contains("[LINE"), "占位没换掉：{v}");
        // 给模型的占位表：皇帝是 <Subject 1>，第一个说话 → S1；台词原文只给意思，不让它抄
        let seen = lm.seen.lock().unwrap();
        let speakers = &seen[0].iter().find(|(k, _)| k == "SPEAKERS").unwrap().1;
        assert_eq!(speakers, "[LINE 1]: at the moment it is spoken, write exactly \"<Subject 1> (S1) in a 40-year-old man's voice says, [LINE 1]\"");
        let brief = &seen[0].iter().find(|(k, _)| k == "SHOT").unwrap().1;
        assert!(brief.contains("[LINE 1] spoken by 皇帝"), "{brief}");
        assert!(!brief.contains("<d>"), "给模型看的不该有现成的 <d> 标签，它会照抄：{brief}");
    }

    /// **表演指示要真的发出去。** 中文只能出现在给模型的指令里（当翻译源），
    /// 正文强制英文 —— 这是线路 B 和线路 A 唯一的处理差别。
    #[tokio::test]
    async fn 有语气时给模型占位和中文翻译源() {
        let mut s = shot();
        s.dialogue_lines[0].emotion = "压低声音，几乎是气音".into();
        s.dialogue_lines[0].voice = "沙哑，语速偏慢".into();
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let lm = FakeLm::new(vec![good_reply()]);
        let _ = synthesize(&ctx, |slot, vars| lm.call(slot, vars), 7).await.unwrap();

        let seen = lm.seen.lock().unwrap();
        let sp = &seen[0].iter().find(|(k, _)| k == "SPEAKERS").unwrap().1;
        // **占位必须在「照抄这句」的引号里面**，不能只在指令正文里 ——
        // 只查 contains(DELIVERY) 是假绿：指令那半句本来就带这个词，
        // 就算 who 里一个占位都没有也照样通过（2026-09-18 变异测试当场抓到）。
        assert!(
            sp.contains(r#"write exactly "<Subject 1> (S1) in a 40-year-old man's voice, <DELIVERY> says, [LINE 1]""#),
            "占位没插进要照抄的那句里，表演指示发不出去：{sp}"
        );
        assert!(sp.contains("沙哑，语速偏慢，压低声音，几乎是气音"), "中文语气没当翻译源给出去：{sp}");
    }

    /// 翻译那一步成功：说话人写法里直接是译好的英文，不再让写手翻（2026-09-27）
    #[tokio::test]
    async fn 语气先翻好_说话人写法里直接是英文() {
        let mut s = shot();
        s.dialogue_lines[0].emotion = "测试专用：压低声音".into();
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let lm = FakeLm::new(vec![good_reply()]);
        lm.voice.lock().unwrap().push(r#"{"items": ["low and hushed"]}"#.into());
        let _ = synthesize(&ctx, |slot, vars| lm.call(slot, vars), 7).await.unwrap();
        let seen = lm.seen.lock().unwrap();
        let sp = &seen[0].iter().find(|(k, _)| k == "SPEAKERS").unwrap().1;
        assert!(sp.contains("in a 40-year-old man's voice, low and hushed says, [LINE 1]"), "{sp}");
        assert!(!sp.contains(DELIVERY), "翻好了就不该再让写手翻：{sp}");
    }

    /// **没写情绪的句子一个字都不加**（用户：「对话也不是每句都要带情绪」）。绝大多数镜头属于这种，回归风险全在这儿。
    #[tokio::test]
    async fn 没有语气时和以前完全一样() {
        let s = shot();
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let lm = FakeLm::new(vec![good_reply()]);
        let _ = synthesize(&ctx, |slot, vars| lm.call(slot, vars), 7).await.unwrap();
        let seen = lm.seen.lock().unwrap();
        let sp = &seen[0].iter().find(|(k, _)| k == "SPEAKERS").unwrap().1;
        assert_eq!(sp, "[LINE 1]: at the moment it is spoken, write exactly \"<Subject 1> (S1) in a 40-year-old man's voice says, [LINE 1]\"");
    }

    /// **写手把说话人写短了，代码补回来**（2026-10-11 EP05 第 10~15 镜只剩 `(S1) says,`）；
    /// 翻译失败、语气只有写手那份时不覆盖
    #[test]
    fn 写手省掉的语气由代码补回() {
        let m = |who: &str| LineMarker { marker: "[LINE 1]".into(), who: who.into(), line: "本机拒绝。".into(), tone: String::new(), timbre: String::new(), delta: String::new(), verb: SAYS, lips: String::new() };
        let full = "<Subject 1> (S1) in a woman's voice, calm, stressing the final word, no pause";
        let out = fill_lines("[Shot 1] She turns. <Subject 1> (S1) says, [LINE 1] Sound: wind", &[m(full)]);
        assert_eq!(out, "[Shot 1] She turns. <Subject 1> (S1) in a woman's voice, calm, stressing the final word, no pause says, <d>[Chinese] 本机拒绝。</d> Sound: wind");
        // 写手照抄了：不变
        let out2 = fill_lines(&format!("[Shot 1] She turns. {full} says, [LINE 1] Sound: wind"), &[m(full)]);
        assert_eq!(out2, out);
        // 翻译失败（占位还在）：写手自己翻的那份留着
        let fallback = format!("<Subject 1> (S1) in a woman's voice, {DELIVERY}");
        let out3 = fill_lines("[Shot 1] <Subject 1> (S1) in a woman's voice, cold and flat says, [LINE 1]", &[m(&fallback)]);
        assert!(out3.contains("cold and flat says, <d>"), "{out3}");
    }

    /// 交给翻译的是「情绪｜台词：原文」—— 没有原文就判断不了重音落在哪
    #[test]
    fn 翻译源带上台词原文() {
        let mut s = shot();
        s.dialogue_lines[0].emotion = "冷笑，一字一顿".into();
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let ms = line_markers(&ctx, &tags(&ctx));
        assert_eq!(ms[0].delta, "冷笑，一字一顿｜台词：既言才华盖世，当殿作诗。若不能——斩。");
        // 没写情绪：不交翻译、不留占位
        let s2 = shot();
        let ctx2 = ShotContext { shot: &s2, prev: None, assets: &a, style: "", long_take: false };
        let ms2 = line_markers(&ctx2, &tags(&ctx2));
        assert!(ms2[0].delta.is_empty() && !ms2[0].who.contains(DELIVERY), "{:?}", ms2[0].who);
    }

    /// **这三条一个都不许进 `check`。**
    ///
    /// `check_wording` 里有 `out.extend(check(...))` —— 放进去就等于
    /// 合成重试三次后整镜报错。而它们是**启发式**（措辞对不上、
    /// 正则找错 `(SN)`、`or` 是程度副词），误报就是白烧一次生成。
    ///
    /// 2026-09-19 我把它们放进了 `check`，注释却写着「只提示不阻断」——
    /// 正好相反，要到真有一镜报废才看得出来（Codex Astra 的第二份审查抓到）。
    /// **注释挡不住调用链**，所以用测试钉住。
    #[test]
    fn 这几条不许让整镜报废() {
        let mut s = shot();
        s.dialogue_lines[0].emotion = "厉声".into();
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        // 一份「表演指示丢了 + 有二选一 + 说话人串了」的正文
        let bad = full(&long_desc()
            .replace("[LINE 1]", "<d>[Chinese] 既言才华盖世，当殿作诗。若不能——斩。</d>")
            .replace("<Subject 1> (S1) says,", "<Subject 1> (S1) in a 40-year-old man's voice says,")
            .replace("A realistic cinematic style", "A bronze or ceramic vessel in a realistic style"));
        assert!(!advisories(&ctx, &bad).is_empty(), "提示该有");
        for m in check(&ctx, &bad) {
            assert!(
                !m.contains("表演指示") && !m.contains("二选一") && !m.contains("串了"),
                "这一条跑进 check 了，会让整镜报废：{m}"
            );
        }
    }

    /// **说话人串了要查得出来。**
    ///
    /// 原来只查标签编号范围和台词出现次数，没核对「这句话前面写的是谁」。
    /// 串了的话 H3 给错的人配口型 —— 画面上甲在动嘴，声音是乙的台词。
    #[test]
    fn 说话人串了要报() {
        let mut s = shot();
        s.character_ids = vec!["char_emperor".into(), "char_shen".into()];
        s.dialogue_lines = vec![
            DialogueLine { speaker: "皇帝".into(), line: "斩。".into(), ..Default::default() },
            DialogueLine { speaker: "林婉清（盛唐）".into(), line: "臣女不服。".into(), ..Default::default() },
        ];
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let base = long_desc().replace(
            "<Subject 1> (S1) says, [LINE 1]",
            "<Subject 1> (S1) says, <d>[Chinese] 斩。</d> Then <Subject 2> (S2) says, <d>[Chinese] 臣女不服。</d>",
        );
        assert!(!advisories(&ctx, &full(&base)).iter().any(|m| m.contains("串了")), "{:?}", advisories(&ctx, &full(&base)));

        // 第二句绑到了 (S1) —— 串了
        let bad = base.replace("Then <Subject 2> (S2) says, <d>[Chinese] 臣女不服。", "Then <Subject 2> (S1) says, <d>[Chinese] 臣女不服。");
        let msgs = advisories(&ctx, &full(&bad));
        assert!(msgs.iter().any(|m| m.contains("串了")), "{msgs:?}");
    }

    /// **英文侧也要拒二选一。** 中文那边早禁了，英文一直没查 ——
    /// 实测 E04 第 9 镜写了「bronze or ceramic」，模板明确禁止而校验没拦。
    #[test]
    fn 英文里的二选一要报() {
        let s = shot();
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        // 基线：台词填好的正文（不填的话先被「台词没写进 <d>」报掉）
        let base = long_desc().replace("[LINE 1]", "<d>[Chinese] 既言才华盖世，当殿作诗。若不能——斩。</d>");
        assert!(!advisories(&ctx, &full(&base)).iter().any(|m| m.contains("二选一")), "{:?}", advisories(&ctx, &full(&base)));

        let bad = full(&base.replace("A realistic cinematic style", "A bronze or ceramic vessel in a realistic style"));
        assert!(advisories(&ctx, &bad).iter().any(|m| m.contains("二选一")), "{:?}", advisories(&ctx, &bad));

        // **程度副词不是二选一**，放过 —— 不然每份正文都会挂一条假警告
        let ok = full(&base.replace("A realistic cinematic style", "About five seconds or so into a realistic style"));
        assert!(!advisories(&ctx, &ok).iter().any(|m| m.contains("二选一")), "{:?}", advisories(&ctx, &ok));
    }
    // ---- 台词时间码（2026-09-20，用户：「应该在分镜里提前处理好」）----

    /// **从正文里扒得出每一句的开口时刻和字数。**
    #[test]
    fn 时间码和字数扒得出来() {
        let v = "detailed_description:\n[Shot 1] At 00:00.0, a room. \
                 At 00:03.0, <Subject 1> (S1) says, <d>[Chinese] 你好啊</d> \
                 At 00:07.5, <Subject 2> (S2) says, <d>[Chinese] 我也好</d> Sound: wind.";
        let t = line_timings(v);
        assert_eq!(t.len(), 2);
        assert_eq!((t[0].n, t[0].at, t[0].chars), (1, 3.0, 3));
        assert_eq!((t[1].n, t[1].at, t[1].chars), (2, 7.5, 3));
        assert!((t[0].needs - 0.73).abs() < 0.02, "{}", t[0].needs);
    }

    /// **一句话说到镜尾之后 → 硬问题，而且要说清挪到哪。**
    ///
    /// 纯算术（字数 ÷ 4.1），模型恰恰算不准 —— 代码算完把数字喂回去让它挪。
    #[test]
    fn 说不完要报_并给出该挪到几秒() {
        let mut s = shot();
        s.duration = 15.0;
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let v = "detailed_description:\n[Shot 1] At 00:13.0, <Subject 1> (S1) says, <d>[Chinese] 十个字的台词刚好十个字</d> Sound: x.";
        let w = Wording {
            subjects: vec![], summary: "x".into(), detailed_description: v.into(),
            overall_soundscape: "y".into(), frame: "f".into(),
        };
        let p = check_wording(&ctx, &[], &w, v);
        let hit = p.hard.iter().find(|m| m.contains("说到")).unwrap_or_else(|| panic!("没报说不完：{:?}", p.hard));
        assert!(hit.contains("把开口时刻往前挪到"), "没说该挪到哪：{hit}");
    }

    /// 撞上下一句也要报，并且指出是和第几句撞的。
    #[test]
    fn 撞上下一句要报() {
        let s = shot();
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let v = "detailed_description:\n[Shot 1] At 00:01.0, a says, <d>[Chinese] 十个字的台词刚好十个字</d> \
                 At 00:02.0, b says, <d>[Chinese] 嗯</d> Sound: x.";
        let w = Wording {
            subjects: vec![], summary: "x".into(), detailed_description: v.into(),
            overall_soundscape: "y".into(), frame: "f".into(),
        };
        let p = check_wording(&ctx, &[], &w, v);
        assert!(p.hard.iter().any(|m| m.contains("第 2 句")), "{:?}", p.hard);
    }

    /// **逐段核对**（2026-09-27）：每一段的起点附近要有一个时间码；缺了就是写手并段 / 漏段。
    #[test]
    fn 每一段都要有自己的时间码() {
        let segs = vec![
            ShotSegment { start: 0.0, end: 3.0, title: "钩子".into(), visual: "a".into(), ..Default::default() },
            ShotSegment { start: 3.0, end: 7.5, title: "开口".into(), visual: "b".into(), ..Default::default() },
            ShotSegment { start: 7.5, end: 12.0, title: "收尾".into(), visual: "c".into(), ..Default::default() },
        ];
        // 3.2 算落在 3.0 上（容差 0.35）；段内多写的时间码不影响
        assert!(segment_coverage(&segs, "[Shot 1] At 00:00.0, x. At 00:03.2, y. At 00:05.0, z. At 00:07.5, w.").is_empty());
        // 第 2、3 段被并掉了
        assert_eq!(segment_coverage(&segs, "[Shot 1] At 00:00.0, x and then y and z."), vec![2, 3]);
        // 第 1 段从 0 秒开始：[Shot 1] 开头就是第 0 秒，没写 At 00:00.0 也不算漏；没有 [Shot 1] 就算
        assert_eq!(segment_coverage(&segs, "[Shot 1] A wide shot. At 00:03.0, y. At 00:07.5, z."), Vec::<usize>::new());
        assert_eq!(segment_coverage(&segs, "A wide shot. At 00:03.0, y. At 00:07.5, z."), vec![1]);
        // 晚 0.6 秒不算
        assert_eq!(segment_coverage(&segs, "At 00:00.0, x. At 00:03.6, y. At 00:07.5, z."), vec![2]);
        // **早 0.8 秒算**：写手把台词往前挪（EP03 第 5 镜 4.0 → 3.2），这一段照样写到了
        assert!(segment_coverage(&segs, "[Shot 1] x. At 00:02.2, y. At 00:06.7, z.").is_empty());
        // 早过 1 秒不算
        assert_eq!(segment_coverage(&segs, "[Shot 1] x. At 00:01.9, y. At 00:07.5, z."), vec![2]);
        // 一个时间码只能算给一段：3.0 给了第 2 段，第 3 段（7.5）不能再拿它；并段照样抓
        let tight = vec![
            ShotSegment { start: 0.0, end: 3.0, ..Default::default() },
            ShotSegment { start: 3.0, end: 3.8, ..Default::default() },
            ShotSegment { start: 3.8, end: 8.0, ..Default::default() },
        ];
        assert_eq!(segment_coverage(&tight, "[Shot 1] x. At 00:03.0, y and z."), vec![3]);
        // 也不能早过上一段的起点
        assert_eq!(segment_coverage(&tight, "[Shot 1] x. At 00:03.1, y. At 00:03.5, z."), Vec::<usize>::new());
        assert_eq!(segment_coverage(&tight, "[Shot 1] x. At 00:02.9, y and z."), vec![3]);
        // 两段起点只差 0.4 秒：3.2 落在两段的容差里，也只能算一段
        let close = vec![
            ShotSegment { start: 0.0, end: 3.0, ..Default::default() },
            ShotSegment { start: 3.0, end: 3.4, ..Default::default() },
            ShotSegment { start: 3.4, end: 8.0, ..Default::default() },
        ];
        assert_eq!(segment_coverage(&close, "[Shot 1] x. At 00:03.2, y and z."), vec![3]);
    }

    /// 漏段是**软**问题：带着段号和标题进反馈重试，但不作废这一镜；段都写到了就不报。
    #[test]
    fn 漏段只提醒不作废() {
        let mut s = shot();
        s.duration = 12.0;
        s.dialogue_lines.clear();
        s.segments = vec![
            ShotSegment { start: 0.0, end: 4.0, title: "抬手".into(), visual: "皇帝抬手".into(), ..Default::default() },
            ShotSegment { start: 4.0, end: 12.0, title: "跪地".into(), visual: "林婉清跪地".into(), ..Default::default() },
        ];
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let words = "word ".repeat(400);
        let w = |desc: String| Wording { subjects: vec![], summary: "x".into(), detailed_description: desc, overall_soundscape: "y".into(), frame: "f".into() };
        let merged = w(format!("[Shot 1] At 00:00.0, {words}"));
        let p = check_wording(&ctx, &[], &merged, &merged.detailed_description);
        let msg = p.soft.iter().find(|m| m.contains("没写到")).expect("漏段没报");
        assert!(msg.contains("Segment 2") && msg.contains("00:04.0") && msg.contains("跪地"), "{msg}");
        assert!(!p.hard.iter().any(|m| m.contains("没写到")), "漏段不该拦：{:?}", p.hard);
        let full = w(format!("[Shot 1] At 00:00.0, {words} At 00:04.0, more."));
        assert!(!check_wording(&ctx, &[], &full, &full.detailed_description).soft.iter().any(|m| m.contains("没写到")));
    }

    /// 词数上限按镜头表写了多少放宽：写得薄的和以前一样；写细了上限跟着抬，不逼写手压回一句。
    #[test]
    fn 词数上限跟着镜头表的描述量放宽() {
        let seg = |n: usize| ShotSegment { start: 0.0, end: 12.0, visual: "字".repeat(n), ..Default::default() };
        // EP01 现在的中位密度：12 秒约 300 字 —— 区间不变
        assert_eq!(word_budget_for(12.0, &[seg(300)]), word_budget(12.0));
        // Kotte 母版的密度：12 秒约 500 字
        let (lo, hi) = word_budget_for(12.0, &[seg(500)]);
        // 2026-10-06 起封顶在官方 500 词附近（OFFICIAL_MAX_WORDS），写得再细也不再放到 700+
        assert!(hi == OFFICIAL_MAX_WORDS && hi > word_budget(12.0).1, "{hi}");
        assert!(lo >= 300 && lo + 100 <= hi, "{lo} {hi}");
        // 站位、运镜也算原文；空白和约束不算（约束每段原样重复，不是细节）
        let rich = ShotSegment { start: 0.0, end: 12.0, visual: "字 ".repeat(200), blocking: "站".repeat(200), camera: "推".repeat(100), constraints: "保".repeat(100), ..Default::default() };
        assert_eq!(source_chars(&[rich]), 500);
    }

    /// **长镜头窗口的截止时间要减掉尾巴。** 同一句普通镜头能过、长镜头窗口不能过。
    #[test]
    fn 长镜头窗口的截止时间更早() {
        let mut s = shot();
        s.duration = 15.0;
        let a = assets();
        // 4 字 ≈ 0.98 秒：12.5+0.98 = 13.48 —— 普通镜头（15 秒）过，长镜头窗口（13 秒截止）不过
        let v = "detailed_description:\n[Shot 1] At 00:12.5, a says, <d>[Chinese] 一二三四</d> Sound: x.";
        let w = Wording {
            subjects: vec![], summary: "x".into(), detailed_description: v.into(),
            overall_soundscape: "y".into(), frame: "f".into(),
        };
        let plain = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        assert!(!check_wording(&plain, &[], &w, v).hard.iter().any(|m| m.contains("说到")), "普通镜头不该拦");

        let lt = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: true };
        let hit = check_wording(&lt, &[], &w, v);
        assert!(hit.hard.iter().any(|m| m.contains("长镜头窗口")), "长镜头窗口没拦：{:?}", hit.hard);
    }

    /// **一个时间码都没写只是软问题，不许拦。**
    ///
    /// 新加的要求，模型会不会照做没验过。做成硬的 = 模型不配合就整集废 ——
    /// 上一周刚为同样的形状（词数下限）废过 E05 六镜。
    #[test]
    fn 没写时间码不拦_只提示() {
        let s = shot();
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let v = "detailed_description:\n[Shot 1] a says, <d>[Chinese] 你好</d> Sound: x.";
        let w = Wording {
            subjects: vec![], summary: "x".into(), detailed_description: v.into(),
            overall_soundscape: "y".into(), frame: "f".into(),
        };
        let p = check_wording(&ctx, &[], &w, v);
        assert!(p.soft.iter().any(|m| m.contains("Around the N.N-second mark")), "该提示的没提示：{:?}", p.soft);
        assert!(!p.hard.iter().any(|m| m.contains("At 00:SS.S")), "不该拦却拦了：{:?}", p.hard);
    }



    /// **长镜头窗口的正文里要写清「台词什么时候必须说完」。**
    ///
    /// 用户 2026-09-20：「假如一个对话正好在两个 15 之间？」
    ///
    /// 一句台词不会被劈成两半（每个窗口只看自己的提示词），但**位置**管不住 ——
    /// 镜头表那层的容量检查限制的是说多少字，不是说在哪几秒。
    /// 这句话是唯一能约束位置的地方。
    #[test]
    fn 长镜头窗口的正文里要写清什么时候说完() {
        use crate::cinema::Continuity;
        let a = assets();
        let s = shot();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let t = tags(&ctx);
        let w = Wording {
            subjects: vec![], summary: "x".into(),
            detailed_description: "[Shot 1] x".into(),
            overall_soundscape: "y".into(), frame: "f".into(),
        };
        let off = assemble(&ctx, &t, &[], &w, Continuity::None);
        assert!(!off.contains("long take"), "不是长镜头却写了长镜头约束");

        // 15 秒窗口 → 13 秒之前说完（留 2 秒给重叠区）
        let mut s15 = shot();
        s15.duration = 15.0;
        let on = assemble(&ShotContext { shot: &s15, prev: None, assets: &a, style: "", long_take: true }, &t, &[], &w, Continuity::None);
        assert!(on.contains("one window of a continuous long take"), "{on}");
        assert!(on.contains("before the 13-second mark"), "截止时间算错了：{on}");
        assert!(on.contains("nobody speaks, and no line starts there"), "没说尾巴不许起新句：{on}");
        // 2026-10-06 起写进正文（[Shot 1] 之前），不再是 subject_definitions 里的一行
        let body = &on[on.find("detailed_description:").unwrap()..];
        assert!(body.contains("one window of a continuous long take") && body.find("one window").unwrap() < body.find("[Shot 1]").unwrap(), "{on}");

        // 短窗口也不能算出负数或 0
        let mut s3 = shot();
        s3.duration = 3.0;
        let short = assemble(&ShotContext { shot: &s3, prev: None, assets: &a, style: "", long_take: true }, &t, &[], &w, Continuity::None);
        assert!(short.contains("before the 1-second mark"), "3 秒窗口算崩了：{short}");
    }

    /// **接上一镜时 `<Video 1>` 必须出现在正文里。**
    ///
    /// 只把视频绑进输入、提示词里不写标签 —— H3 收不到它，等于白跑一镜
    /// （官方文档明说，2026-09-20 Codex 查到、我在示例工作流里核对过）。
    /// 反过来只写标签不传数据也一样，另一半由
    /// `weetodd::接上一镜时视频参考串在图片之后` 钉着。
    #[test]
    fn 接上一镜时正文里要有video1() {
        use crate::cinema::Continuity;
        let a = assets();
        let mut s = shot();
        let markers = Vec::new();
        let w = Wording {
            subjects: vec![],
            summary: "x".into(),
            detailed_description: "[Shot 1] x".into(),
            overall_soundscape: "y".into(),
            frame: "f".into(),
        };
        s.continuity = Continuity::None;
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let t = tags(&ctx);
        let off = assemble(&ctx, &t, &markers, &w, Continuity::None);
        assert!(!off.contains("<Video 1>"), "不接上一镜却写了 <Video 1>");

        let on = assemble(&ctx, &t, &markers, &w, Continuity::RefVideo);
        assert!(on.contains("<Video 1>"), "接了上一镜，正文里没有 <Video 1>");
        // 官方任务前缀：续接源视频 = video continuation（ref 指南 3 节）
        assert!(on.contains("[video continuation + reference generation]"), "{on}");

        // **长镜头里绝不发 `<Video 1>`。**
        //
        // `cinema::run_long_take` 一行都没读 continuity，也不绑任何视频输入 ——
        // 窗口之间的衔接是 H3 潜变量的重叠帧。在长镜头的窗口里写这句话，
        // 等于给 H3 一个根本不存在的 `<Video 1>`，而它不报错
        // （2026-09-20 用户问「选长镜头后接上一镜是不是就不能用了」时查出来的）。
        let mut s15 = shot();
        s15.duration = 15.0;
        let both = assemble(&ShotContext { shot: &s15, prev: None, assets: &a, style: "", long_take: true }, &t, &markers, &w, Continuity::RefVideo);
        assert!(!both.contains("<Video 1>"), "长镜头窗口里写了个没人绑的 <Video 1>：{both}");
        assert!(both.contains("one window of a continuous long take"), "长镜头约束丢了：{both}");
        // **必须写成「续接」，不是「模仿运镜」** —— 少了这句它会把上一镜的动作也抄一遍
        assert!(on.contains("does not define what happens next"), "少了「不要抄动作」那句：{on}");
        // 它管不了身份，别让模型以为它管
        assert!(on.contains("does not define anyone's appearance"), "没说清它管不了长相");
        // 保留强度那一段也要有它，否则编号对不上
        assert!(on.contains("<Video 1> (continuation source): partially_preserved"), "retention_analysis 里漏了 <Video 1>");
    }

    /// **否定句里的 `or` 不是二选一。** 2026-09-20 实跑 E05 抓到的误报：
    /// `the phones never move or fall` 说的是两样都不发生，是最确定的写法。
    ///
    /// 这条误报当时还挂在会挡出片的字段上，整集一镜都出不了。
    #[test]
    fn 否定句里的or不算二选一() {
        let s = shot();
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let ok = |t: &str| {
            let p = full(&format!("[Shot 1] {t} {}", long_desc()));
            !advisories(&ctx, &p).iter().any(|m| m.contains("二选一"))
        };
        assert!(ok("The phones never move or fall."), "never");
        assert!(ok("Nothing in the frame shifts or tilts."), "nothing");
        assert!(ok("The camera does not pan or tilt."), "does not");
        assert!(ok("The table is bare, without cups or plates."), "without");
        // 句号之后否定就结束了 —— 下一句的 or 照样要报
        let p = full(&format!("[Shot 1] The camera never moves. A bronze or ceramic cup sits there. {}", long_desc()));
        assert!(advisories(&ctx, &p).iter().any(|m| m.contains("二选一")), "跨句被放过了：{:?}", advisories(&ctx, &p));
    }

    /// **表演指示丢了要能在界面上看见。**
    ///
    /// 模型忘了换 `<DELIVERY>` 时 `strip_delivery` 会抹掉它，合成照常成功 ——
    /// 2026-09-19 之前这是完全静默的。Codex Astra 的审查指出
    /// E03 第 3/5/6 镜、E04 第 6/10/12 镜都有这个退化，而界面上一个字都没有。
    #[test]
    fn 表演指示没写进提示词要报出来() {
        let mut s = shot();
        s.dialogue_lines[0].emotion = "厉声，几乎是吼出来".into();
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };

        // 模型换掉了占位 —— 不该报
        let ok = full(&long_desc().replace(
            "<Subject 1> (S1) says,",
            "<Subject 1> (S1) in a 40-year-old man's voice, in a harsh near-shout, says,",
        ));
        assert!(!advisories(&ctx, &ok).iter().any(|m| m.contains("表演指示")), "{:?}", advisories(&ctx, &ok));

        // 模型忘了换，strip_delivery 抹掉之后剩光秃秃的 `voice says,` —— 要报
        let lost = full(&long_desc().replace(
            "<Subject 1> (S1) says,",
            "<Subject 1> (S1) in a 40-year-old man's voice says,",
        ));
        let msgs = advisories(&ctx, &lost);
        assert!(msgs.iter().any(|m| m.contains("表演指示") && m.contains("厉声")), "{msgs:?}");
    }

    /// **没有表演指示的镜头不该被报。** 绝大多数老镜头是这种，
    /// 误报的话每一镜都挂一条假警告，等于这条检查没有。
    #[test]
    fn 没有表演指示时不报() {
        let s = shot(); // emotion / voice 都是空的
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let p = full(&long_desc().replace(
            "<Subject 1> (S1) says,",
            "<Subject 1> (S1) in a 40-year-old man's voice says,",
        ));
        assert!(!advisories(&ctx, &p).iter().any(|m| m.contains("表演指示")), "{:?}", advisories(&ctx, &p));
    }

    /// **模型忘了换占位不能让镜头报废**，也不能把占位留在正文里
    /// —— 留着会被 H3 当噪声吃进去。退回到只有性别年龄的老样子就行。
    #[test]
    fn 没换掉的占位被抹掉_连同前面的逗号() {
        let raw = "<Subject 1> (S1) in a 40-year-old man's voice, <DELIVERY> says, [LINE 1] Sound: wind.";
        let out = fill_lines(raw, &[LineMarker { marker: "[LINE 1]".into(), who: "x".into(), line: "斩。".into(), tone: "厉声".into() , timbre: String::new(), delta: String::new(), verb: SAYS, lips: String::new() }]);
        assert!(!out.contains("DELIVERY"), "占位漏进正文：{out}");
        assert!(out.contains("man's voice says,"), "抹掉之后句子断了：{out}");
        assert!(out.contains("<d>[Chinese] 斩。</d>"), "台词没填：{out}");
    }

    // ---- 中文的两个合法出口（2026-09-19）----

    /// **模板要求的写法必须过检查。** 这条盯的是「模板和检查互相否定」这一类：
    /// 模板第 4 条明着要求把画面上的中文逐字保留，而检查一度把它判成违规，
    /// 模型照模板写就被打回，三次后整镜作废。
    #[test]
    fn 模板教的画面文字写法_检查放行() {
        let tpl = crate::prompts::PromptSlot::RouteBWriter.default_text();
        // 模板里教的是官方写法（英文双引号，基础指南 4.5）；检查两种都认（老提示词里是「」）
        assert!(tpl.contains("put the Chinese inside English double quotation marks"), "模板没教画面文字怎么写");
        let desc = format!(
            "A phone screen shows the phrase \u{300c}\u{672a}\u{8bfb}\u{6d88}\u{606f}\u{300d} printed in white. {}",
            "The camera holds steady on the desk and nothing moves at all. ".repeat(30)
        );
        let s = shot();
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let w = Wording {
            subjects: vec![],
            summary: "A phone on a desk.".into(),
            detailed_description: format!("[Shot 1] {desc}"),
            overall_soundscape: "A quiet room.".into(),
            frame: "a desk".into(),
        };
        let p = check_wording(&ctx, &[], &w, "");
        assert!(
            !p.all().iter().any(|m| m.contains("\u{672a}\u{8bfb}")),
            "\u{300c}\u{300d} 里的画面文字被判成违规了：{:?}",
            p.all()
        );
    }

    #[test]
    fn 括号外的中文照样报_但只是软问题() {
        let s = shot();
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let w = Wording {
            subjects: vec![],
            summary: "\u{5c0f}\u{84dd} waves.".into(),
            detailed_description: format!("[Shot 1] {}", "The camera holds steady and nothing moves. ".repeat(40)),
            overall_soundscape: "quiet".into(),
            frame: "a desk".into(),
        };
        let p = check_wording(&ctx, &[], &w, "");
        assert!(p.soft.iter().any(|m| m.contains("\u{5c0f}\u{84dd}")), "漏网的中文没报：{:?}", p.soft);
        assert!(!p.hard.iter().any(|m| m.contains("\u{5c0f}\u{84dd}")), "中文不该是硬问题：{:?}", p.hard);
    }

    /// 整段正文塞进 `\u{300c}\u{300d}` 逃避中文检查 —— 要认出来。
    #[test]
    fn 画面文字太长要报() {
        let long: String = std::iter::repeat_n('\u{5b57}', MAX_SCREEN_TEXT_CHARS + 1).collect();
        let s = shot();
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let w = Wording {
            subjects: vec![],
            summary: format!("A sign reads \u{300c}{long}\u{300d}."),
            detailed_description: format!("[Shot 1] {}", "The camera holds steady and nothing moves. ".repeat(40)),
            overall_soundscape: "quiet".into(),
            frame: "a desk".into(),
        };
        assert!(check_wording(&ctx, &[], &w, "").soft.iter().any(|m| m.contains("\u{4e2a}\u{5b57}")), "超长的画面文字没报");
    }

    /// 落单的开引号**不能**把后面的中文一起藏掉 —— 那会把真问题变成静默通过。
    #[test]
    fn 没闭合的开引号不吞后面的中文() {
        let got = outside_screen_text("a \u{300c}\u{672a}\u{8bfb} and then \u{5c0f}\u{84dd}");
        assert!(got.contains('\u{672a}') && got.contains('\u{5c0f}'), "被吞了：{got}");
        assert_eq!(outside_screen_text("a \u{300c}\u{672a}\u{8bfb}\u{300d} b"), "a  b");
    }

    // ---- 硬 / 软分档（2026-09-19 实跑 E05 八镜废了六镜）----

    /// **词数没到推荐区间不该让整镜作废。** 实跑 E05：模型稳定写 270~360 词，
    /// 而当天把下限提到了 320/380 —— 六镜全废在这一条上。
    #[tokio::test]
    async fn 只差词数的正文_重试用完照样交出去_并带话() {
        let s = shot();
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        // 200 词：过硬下限（150），没到 10 秒镜头的推荐区间（320）
        let filler = "the hall stays silent and the light holds steady on the throne ".repeat(20);
        let thin = serde_json::json!({
            "subjects": [
                {"tag": "<Subject 1>", "look": "a 40-year-old emperor", "control": "square face", "not_control": "pose"},
                {"tag": "<Subject 2>", "look": "a slim woman", "control": "oval face", "not_control": "pose"},
                {"tag": "<Subject 3>", "look": "a vast hall", "control": "throne", "not_control": "light"}
            ],
            "summary": "The emperor speaks.",
            "detailed_description": format!("[Shot 1] A low-angle shot frames <Subject 1> above <Subject 2>. {filler} <Subject 1> (S1) says, [LINE 1] Sound: the robe rustles."),
            "overall_soundscape": "A silent hall.",
            "frame": "Low-angle medium shot."
        })
        .to_string();
        let lm = FakeLm::new(vec![thin.clone(), thin.clone(), thin.clone(), thin.clone()]);
        let (p, soft) = synthesize(&ctx, |slot, v| lm.call(slot, v), 1)
            .await
            .expect("只差词数不该让整镜作废");
        assert!(p.video.contains("<d>[Chinese] \u{65e2}\u{8a00}\u{624d}\u{534e}\u{76d6}\u{4e16}"), "台词没进去");
        assert!(soft.iter().any(|m| m.contains("\u{504f}\u{8584}")), "没把「偏薄」带出来：{soft:?}");
        assert!(soft.iter().all(|m| m.contains("\u{7b2c} 6 \u{955c}")), "带出来的话没说是哪一镜：{soft:?}");
        // 四次都不合格，说明它**确实重试到用完**才交的
        assert_eq!(lm.seen.lock().unwrap().len(), 4);
    }

    /// 2026-09-30 EP02 第 6 镜：写手在「台词排不开」和「词数 542 > 520」之间来回跳，三次全废。
    /// 超出上限 10% 以内是软的（照样反馈、重试用完交得出去），超多了还是硬的
    #[test]
    fn 词数略超是软的_超多了是硬的() {
        let s = shot();
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let (_, hi) = word_budget_for(s.duration, &s.segments);
        let wording = |n: usize| Wording {
            subjects: vec![],
            summary: "x".into(),
            detailed_description: format!("[Shot 1] {}", "word ".repeat(n)),
            overall_soundscape: "quiet".into(),
            frame: "a desk".into(),
        };
        let a_bit = check_wording(&ctx, &[], &wording(hi + hi / 25), "");
        assert!(a_bit.soft.iter().any(|m| m.contains("略超")), "{:?}", a_bit.soft);
        assert!(!a_bit.hard.iter().any(|m| m.contains("英文词")), "{:?}", a_bit.hard);
        let way = check_wording(&ctx, &[], &wording(hi * 13 / 10), "");
        assert!(way.hard.iter().any(|m| m.contains("压到")), "{:?}", way.hard);
    }

    /// 最后一次返回解析不了，**前几次的真实问题不能被盖掉** —— 界面上原来只剩「返回里没有 JSON」
    #[tokio::test]
    async fn 最后一次没有_json_前几次的问题照样报出来() {
        let s = shot();
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let (_, hi) = word_budget_for(s.duration, &s.segments);
        let long = serde_json::json!({
            "subjects": [], "summary": "x",
            "detailed_description": format!("[Shot 1] {}", "word ".repeat(hi * 2)),
            "overall_soundscape": "quiet", "frame": "a desk"
        })
        .to_string();
        let lm = FakeLm::new(vec![long.clone(), long.clone(), long, "抱歉，我无法完成".into()]);
        let err = synthesize(&ctx, |slot, v| lm.call(slot, v), 1).await.expect_err("应该失败").to_string();
        assert!(err.contains("这一次：返回里没有 JSON"), "{err}");
        assert!(err.contains("压到"), "前几次的问题被盖掉了：{err}");
    }

    /// **被内容过滤拦了不重试**：同一份输入再发还会被拦，每次都是一整轮计费调用（2026-10-03 审查）。
    #[tokio::test]
    async fn 内容过滤不重试_直接报() {
        let s = shot();
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let msg = crate::prompts::content_filter_message(crate::prompts::PromptSlot::RouteBWriter, 120);
        // 只数写手那一步（语气翻译每镜先调一次，它失败了退回写手翻，不算重试）
        let err = synthesize(&ctx, |slot, _| {
            if slot == crate::prompts::PromptSlot::RouteBWriter {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            let m = msg.clone();
            async move { Err::<String, _>(Error::runtime(m)) }
        }, 1).await.expect_err("应该失败").to_string();
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1, "被拦了还在重试");
        assert!(err.contains("内容过滤") && err.contains("没有重试") && !err.contains("重试过"), "{err}");
    }

    /// 二选一：合成时当软问题喂回写手；`or` 挨着中文台词时按字符截，不会切在汉字中间崩掉
    #[test]
    fn 二选一合成时就反馈_挨着中文不崩() {
        let v = "<d>[Chinese] 你好啊你好啊你好啊</d> or 他们来了，左边是红色";
        assert!(english_alternative(v).is_some());
        assert_eq!(english_alternative("the phones never move or fall"), None, "否定句里的 or 放过");
        assert_eq!(english_alternative("two or three people"), None);
        let s = shot();
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let w = Wording { subjects: vec![], summary: "x".into(), detailed_description: "[Shot 1] weak cyan-purple or dark-red neon edges".into(), overall_soundscape: "q".into(), frame: "f".into() };
        let got = check_wording(&ctx, &[], &w, "weak cyan-purple or dark-red neon edges");
        assert!(got.soft.iter().any(|m| m.contains("二选一")), "{:?}", got.soft);
    }

    /// 写手四次都漏第 2 段 → **交写手那份、不补中文**，并说一声（2026-10-06 改）。
    /// 以前按镜头表的中文原文补一句，H3 把它当台词念了（EP03 第 5 镜 Whisper 实测）。
    #[tokio::test]
    async fn 重试用完还漏段_交写手那份_不补中文() {
        let mut s = shot();
        s.segments = vec![
            ShotSegment { start: 0.0, end: 4.0, title: "圣上下令".into(), visual: "皇帝抬手".into(), ..Default::default() },
            ShotSegment { start: 4.0, end: 10.0, title: "跪地".into(), visual: "林婉清跪地仰望".into(), camera: "低机位缓推".into(), ..Default::default() },
        ];
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let filler = "the hall stays silent and the light holds steady on the throne ".repeat(30);
        let reply = |desc: String| {
            serde_json::json!({
                "subjects": [
                    {"tag": "<Subject 1>", "look": "a 40-year-old emperor", "control": "square face", "not_control": "pose"},
                    {"tag": "<Subject 2>", "look": "a slim woman", "control": "oval face", "not_control": "pose"},
                    {"tag": "<Subject 3>", "look": "a vast hall", "control": "throne", "not_control": "light"}
                ],
                "summary": "The emperor speaks.", "detailed_description": desc,
                "overall_soundscape": "A silent hall.", "frame": "Low-angle medium shot."
            })
            .to_string()
        };
        let skip = reply(format!("[Shot 1] At 00:00.0, <Subject 1> raises a hand. {filler} At 00:05.5, <Subject 1> (S1) says, [LINE 1] Sound: the robe rustles."));
        let lm = FakeLm::new(vec![skip.clone(), skip.clone(), skip.clone(), skip]);
        let (p, notes) = synthesize(&ctx, |slot, v| lm.call(slot, v), 1).await.expect("漏段不该整镜作废");
        assert!(!p.video.contains("林婉清跪地仰望"), "镜头表的中文原文不能进正文：H3 会念出来");
        assert_eq!(stray_chinese(&p.video), None, "正文里不该有台词以外的中文：{}", p.video);
        assert!(notes.iter().any(|m| m.contains("没写到第 2 段") && m.contains("没有补中文")), "{notes:?}");
        assert_eq!(lm.seen.lock().unwrap().len(), 4, "先重试到用完");
    }

    /// 挑哪一份：没漏段的优先于「软问题更少但漏段」的
    #[tokio::test]
    async fn 挑结果时不漏段的优先() {
        let mut s = shot();
        s.segments = vec![
            ShotSegment { start: 0.0, end: 4.0, title: "a".into(), visual: "皇帝抬手".into(), ..Default::default() },
            ShotSegment { start: 4.0, end: 10.0, title: "b".into(), visual: "林婉清跪地".into(), ..Default::default() },
        ];
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let filler = "the hall stays silent and the light holds steady on the throne ".repeat(30);
        let mk = |desc: String| serde_json::json!({
            "subjects": [
                {"tag": "<Subject 1>", "look": "a 40-year-old emperor", "control": "square face", "not_control": "pose"},
                {"tag": "<Subject 2>", "look": "a slim woman", "control": "oval face", "not_control": "pose"},
                {"tag": "<Subject 3>", "look": "a vast hall", "control": "throne", "not_control": "light"}
            ],
            "summary": "The emperor speaks.", "detailed_description": desc, "overall_soundscape": "A silent hall.", "frame": "Low-angle medium shot."
        }).to_string();
        // 漏段、别的都干净
        let skip = mk(format!("[Shot 1] At 00:00.0, a. {filler} At 00:05.5, <Subject 1> (S1) says, [LINE 1] Sound: x."));
        // 不漏段，但写了二选一（软问题）
        let full = mk(format!("[Shot 1] At 00:00.0, a. {filler} At 00:04.0, she kneels on bronze or ceramic tiles. At 00:05.5, <Subject 1> (S1) says, [LINE 1] Sound: x."));
        let lm = FakeLm::new(vec![skip.clone(), full, skip.clone(), skip.clone()]);
        let (p, _) = synthesize(&ctx, |slot, v| lm.call(slot, v), 1).await.unwrap();
        assert!(p.video.contains("she kneels"), "应该交不漏段的那份");

        // 但**不漏段而夹了中文**的那份要输给漏段的：中文 H3 会念出来，比少一拍动作糟（2026-10-06 EP03 第 5 镜）
        let cjk = mk(format!("[Shot 1] At 00:00.0, a. {filler} At 00:04.0, she kneels 慢慢地. At 00:05.5, <Subject 1> (S1) says, [LINE 1] Sound: x."));
        let lm2 = FakeLm::new(vec![skip.clone(), cjk, skip.clone(), skip]);
        let (p2, _) = synthesize(&ctx, |slot, v| lm2.call(slot, v), 1).await.unwrap();
        assert!(!p2.video.contains("慢慢地"), "夹中文的那份不该被挑中：{}", p2.video);
    }

    /// 正文里台词 / 画面文字以外的中文：校验提示（覆盖已经合成好的老正文和手改的）
    #[test]
    fn 正文夹中文_校验会提示() {
        assert_eq!(stray_chinese("At 00:01.0, <Subject 1> (S1) says, <d>[Chinese] 菲菲，这就是你那个前男友？</d> A sign reads 「旧货」."), None);
        assert_eq!(
            stray_chinese("At 00:03.2, he says <d>[Chinese] 走</d>. At 00:04.0, 约第4秒吴峥开口，偏头幅度很小。 Sound: x.").as_deref(),
            Some("约第4秒吴峥开口，偏头幅度很")
        );
        let s = shot();
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let p = RoutePrompts { video: "[Shot 1] At 00:04.0, 眼睛仍越过镜头盯着陆川. Sound: x.".into(), ..Default::default() };
        assert!(advisories(&ctx, &p).iter().any(|m| m.contains("台词以外的中文") && m.contains("念出来")), "{:?}", advisories(&ctx, &p));
    }

    /// 反过来：硬问题照样报错不存 —— 分档不是把检查放水。
    #[tokio::test]
    async fn 薄到不成句_重试用完还是报错不存() {
        let s = shot();
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        // 17 个词，连硬下限 150 都不到
        let tiny = serde_json::json!({
            "subjects": [
                {"tag": "<Subject 1>", "look": "a 40-year-old emperor", "control": "square face", "not_control": "pose"},
                {"tag": "<Subject 2>", "look": "a slim woman", "control": "oval face", "not_control": "pose"},
                {"tag": "<Subject 3>", "look": "a vast hall", "control": "throne", "not_control": "light"}
            ],
            "summary": "The emperor speaks.",
            "detailed_description": "[Shot 1] A low-angle shot frames <Subject 1>. <Subject 1> (S1) says, [LINE 1] Sound: wind.",
            "overall_soundscape": "A silent hall.",
            "frame": "Low-angle medium shot."
        })
        .to_string();
        let lm = FakeLm::new(vec![tiny.clone(), tiny.clone(), tiny.clone(), tiny.clone()]);
        let err = synthesize(&ctx, |slot, v| lm.call(slot, v), 1).await.unwrap_err();
        assert!(err.to_string().contains("\u{8fd9}\u{4e0d}\u{7b97}\u{5199}\u{5b8c}"), "{err}");
    }

    /// 一张参考图都没有的镜头：`{TAGS}` 必须**明着禁止**写标签。
    ///
    /// 实跑 E05 第 2 镜就栽在这里 —— 模型抄模板里的例子写了一堆
    /// `<Subject 1>`，九条「用了没有定义的标签」，重试三次全废。
    #[test]
    fn 没有参考图时_标签表要明着说一个标签都不许写() {
        let t = tag_table(&[]);
        assert!(t.contains("NO REFERENCE IMAGES"), "{t}");
        assert!(t.contains("<Subject N>") && t.contains("<Picture N>"), "没点名这两种标签：{t}");
        assert!(t.contains("subjects"), "没说 subjects 要留空：{t}");
    }

    #[tokio::test]
    async fn 措辞不合格就带着问题重写一次_两次都不行报错不存() {
        let s = shot();
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        // 台词没进 <d>、正文里夹中文、太短
        let bad = serde_json::json!({
            "subjects": [{"tag": "<Subject 1>", "look": "emperor"}, {"tag": "<Subject 2>", "look": "woman"}, {"tag": "<Subject 3>", "look": "hall"}],
            "summary": "x", "detailed_description": "[Shot 1] 皇帝说话。", "overall_soundscape": "quiet", "frame": "hall"
        })
        .to_string();
        let lm = FakeLm::new(vec![bad.clone(), good_reply()]);
        let (p, _soft) = synthesize(&ctx, |slot, v| lm.call(slot, v), 1).await.unwrap();
        assert!(p.video.contains("<d>[Chinese] 既言才华盖世"));
        let seen = lm.seen.lock().unwrap();
        let fb = &seen[1].iter().find(|(k, _)| k == "FEEDBACK").unwrap().1;
        // 反馈里两档都要有 —— 重试的目的就是让它把软问题也改掉
        assert!(fb.contains("中文") && fb.contains("这不算写完"), "{fb}");
        drop(seen);

        // 用户定的：最多重试三次 —— 第四次合格照样收；四次都不合格才报错
        let lm = FakeLm::new(vec![bad.clone(), bad.clone(), bad.clone(), good_reply()]);
        assert!(synthesize(&ctx, |slot, v| lm.call(slot, v), 1).await.is_ok());
        assert_eq!(lm.seen.lock().unwrap().len(), 4);
        let lm = FakeLm::new(vec![bad.clone(), bad.clone(), bad.clone(), bad.clone(), good_reply()]);
        let err = synthesize(&ctx, |slot, v| lm.call(slot, v), 1).await.unwrap_err();
        assert!(err.to_string().contains("第 6 镜") && err.to_string().contains("重试过 3 次"), "{err}");
        assert_eq!(lm.seen.lock().unwrap().len(), 4, "重试次数超了");
    }

    /// **长镜头 + 接上一镜：在叫模型之前就拦下。**
    ///
    /// 这两个设置互斥：长镜头的衔接靠窗口之间的重叠帧，而 `run_long_take`
    /// 不绑任何视频输入。合出来的正文里那句 `<Video 1> is the end of the
    /// immediately preceding shot` 会指向一个不存在的输入，**而且不报错**。
    ///
    /// **不放进 `hard`** —— 那一档是喂回模型让它改写的，而这是用户的设置，
    /// 模型改不了。一条模型满足不了的检查，比没有检查更糟。
    #[tokio::test]
    async fn 长镜头里选了接上一镜_合成之前就拦下() {
        use crate::cinema::Continuity;
        let a = assets();
        let mut s = shot();
        s.continuity = Continuity::RefVideo;

        // 不是长镜头：照常合成
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let lm = FakeLm::new(vec![good_reply()]);
        assert!(synthesize(&ctx, |slot, v| lm.call(slot, v), 1).await.is_ok());

        // 长镜头：一次都不叫模型
        let lt = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: true };
        let lm = FakeLm::new(vec![good_reply()]);
        let err = synthesize(&lt, |slot, v| lm.call(slot, v), 1).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("整段出") && msg.contains("接上一镜") && msg.contains("不接"), "报错没说清怎么改：{msg}");
        assert_eq!(lm.seen.lock().unwrap().len(), 0, "白叫了一次模型 —— 这个检查该在叫它之前");

        // 「接住最后一帧」也一样挡 —— 它同样没人绑
        let mut f = shot();
        f.continuity = Continuity::FirstFrame;
        let lt2 = ShotContext { shot: &f, prev: None, assets: &a, style: "", long_take: true };
        let lm = FakeLm::new(vec![good_reply()]);
        assert!(synthesize(&lt2, |slot, v| lm.call(slot, v), 1).await.is_err());
    }

    /// 2026-09-13 实跑第 9、12 镜：模型两次都把台词整句漏掉。
    #[tokio::test]
    async fn 模型漏写台词占位_代码补在声音之前_自己抄了台词要报() {
        let s = shot();
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let forgot = good_reply().replace("<Subject 1> (S1) says, [LINE 1] ", "");
        let lm = FakeLm::new(vec![forgot]);
        let (p, _soft) = synthesize(&ctx, |slot, v| lm.call(slot, v), 1).await.unwrap();
        assert!(p.video.contains("steady on the throne. <Subject 1> (S1) in a 40-year-old man's voice says, <d>[Chinese] 既言才华盖世，当殿作诗。若不能——斩。</d>. Sound: the robe rustles."), "{}", p.video);
        // 模型写了占位又自己抄了一遍台词：会念两遍，要报
        let t = tags(&ctx);
        let markers = line_markers(&ctx, &t);
        let w = Wording { detailed_description: "[Shot 1] <Subject 1> (S1) says, [LINE 1] and again <d>[Chinese] 既言才华盖世，当殿作诗。若不能——斩。</d>".into(), frame: "f".into(), ..Default::default() };
        let v = assemble(&ctx, &t, &markers, &w, crate::cinema::Continuity::None);
        // 钉的是**可执行的那句指示**，不是措辞 —— 钉散文的话改一个字就红
        assert!(check(&ctx, &RoutePrompts { video: v, frame: "f".into(), ..Default::default() })
            .iter()
            .any(|i| i.contains("只写 [LINE N] 占位")));
        // 旁白没有主体标签
        let mut narr = shot();
        narr.dialogue_lines[0].speaker = "旁白".into();
        narr.dialogue_lines[0].off_screen = true;
        let ctx2 = ShotContext { shot: &narr, prev: None, assets: &a, style: "", long_take: false };
        let m = line_markers(&ctx2, &tags(&ctx2));
        // 官方「a stable voice description followed by (Sx)」：没标签、不在画面里的写成稳定的嗓音描述
        assert_eq!(fill_lines("[Shot 1] x. Sound: y", &m), "[Shot 1] x. an off-screen speaker's voice (S1) says, <d>[Chinese] 既言才华盖世，当殿作诗。若不能——斩。</d>. Sound: y");
    }

    /// **同一个人一镜里先出镜说、再内心独白 —— 画外那句必须仍按画外写。**
    ///
    /// 2026-10-01《回收师》EP01 第 7 镜、EP02 第 29 镜：画外标记是按人去重时取的，
    /// 只记第一句。第二句（内心独白）借用了第一句的「出镜」绑定，H3 给它对了口型，不报错。
    #[test]
    fn 同一人先出镜后画外_画外那句仍按画外写() {
        let mut s = shot();
        s.dialogue_lines = vec![
            DialogueLine { speaker: "皇帝".into(), line: "平身。".into(), ..Default::default() },
            DialogueLine { speaker: "皇帝".into(), line: "这一跪，跪得太早了。".into(), off_screen: true, ..Default::default() },
        ];
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let m = line_markers(&ctx, &tags(&ctx));
        // 编号仍按人：两句都是 S1
        assert!(m[0].who.starts_with("<Subject 1> (S1)") && m[1].who.starts_with("<Subject 1> (S1)"), "{:?}", m.iter().map(|x| &x.who).collect::<Vec<_>>());
        assert_eq!((m[0].verb, m[0].lips.as_str()), (SAYS, ""), "出镜那句不该带画外写法");
        assert_eq!(m[1].verb, SAYS_VOICEOVER, "画外那句丢了画外动词");
        assert_eq!(m[1].lips, " while his lips remain completely closed");
        // 发给写手的指令里，画外那句的动词也得是画外的
        let ins = speakers_instruction(&m);
        assert!(ins.contains("says in an off-screen voiceover: [LINE 2]"), "{ins}");
        assert!(ins.contains("says, [LINE 1]"), "{ins}");
        // 写手照指令写了之后，代码在 </d> 后面紧接着补「嘴唇不动」—— 规范要求紧接着写
        let desc = format!("[Shot 1] {} {} [LINE 1] then {} {} [LINE 2] Sound: y", m[0].who, m[0].verb, m[1].who, m[1].verb);
        let v = fill_lines(&desc, &m);
        assert!(v.contains("says in an off-screen voiceover: <d>[Chinese] 这一跪，跪得太早了。</d> while his lips remain completely closed"), "{v}");
        assert!(v.contains("says, <d>[Chinese] 平身。</d> then"), "出镜那句后面不该多出嘴唇不动：{v}");
    }

    /// **反过来也一样：先画外、后出镜 —— 出镜那句不能被标成画外。**
    ///
    /// 2026-10-01《回收师》EP02 第 12 镜：陆川先内心独白，再当面问「你听见我心里想什么了？」。
    /// 旧写法第一句定了 `, off-screen`，第二句（当面说的）也跟着被标成画外 —— H3 可能不对口型。
    #[test]
    fn 同一人先画外后出镜_出镜那句不能被标成画外() {
        let mut s = shot();
        s.dialogue_lines = vec![
            DialogueLine { speaker: "皇帝".into(), line: "朕方才在心里说的。".into(), off_screen: true, ..Default::default() },
            DialogueLine { speaker: "皇帝".into(), line: "你听见了？".into(), ..Default::default() },
        ];
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let m = line_markers(&ctx, &tags(&ctx));
        assert_eq!(m[0].verb, SAYS_VOICEOVER);
        assert_eq!((m[1].verb, m[1].lips.as_str()), (SAYS, ""), "当面说的那句被标成了画外");
        assert!(!m[1].who.contains("off-screen"), "{}", m[1].who);
    }

    /// 「嘴唇不动」按说话人性别写；**不是画面里的主体就不写**（纯旁白画面里没有他）。
    #[test]
    fn 画外的嘴唇按性别写_纯旁白不写嘴唇() {
        let mut s = shot();
        s.dialogue_lines = vec![DialogueLine { speaker: "林婉清（盛唐）".into(), line: "我早知道。".into(), off_screen: true, ..Default::default() }];
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let m = line_markers(&ctx, &tags(&ctx));
        assert_eq!(m[0].lips, " while her lips remain completely closed", "{:?}", m[0].who);
        // 旁白：不是参考主体，`(S1), an off-screen voice` —— 写法照旧，不加嘴唇
        s.dialogue_lines = vec![DialogueLine { speaker: "旁白".into(), line: "那一年，雪下得很早。".into(), off_screen: true, ..Default::default() }];
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let m = line_markers(&ctx, &tags(&ctx));
        assert_eq!((m[0].verb, m[0].lips.as_str()), (SAYS, ""), "旁白画面里没有人，不该写谁的嘴唇");
    }

    /// **短台词是长台词的子串时，检查不能误判成「抄了两遍」。**
    ///
    /// 2026-09-17 真卡死过《每日小离谱》E01 第 2 镜，重试三次全失败。
    /// 三句台词里「半个？」同时是「…花瓶……算半个。」的子串，
    /// 旧写法把所有 <d> 块拼成一整串数子串，「半个」必然数到 2。
    /// **模型写得完全正确，是检查不可能被满足。**
    #[test]
    fn 短台词是长台词的子串时不算重复() {
        let a = assets();
        let mut s = shot();
        s.character_ids = vec!["char_emperor".into()];
        s.dialogue_lines = vec![
            DialogueLine { speaker: "皇帝".into(), line: "全年打碎玻璃杯三个，陶瓷碗一个，花瓶……算半个。".into(), ..Default::default() },
            DialogueLine { speaker: "皇帝".into(), line: "半个？".into(), ..Default::default() },
            DialogueLine { speaker: "皇帝".into(), line: "它现在还立着，只是有点歪。".into(), ..Default::default() },
        ];
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let t = tags(&ctx);
        let markers = line_markers(&ctx, &t);
        // 模型规规矩矩只写占位，三句各一次
        let w = Wording {
            detailed_description: "[Shot 1] <Subject 1> (S1) says, [LINE 1] then [LINE 2] then [LINE 3]".into(),
            frame: "f".into(),
            ..Default::default()
        };
        let v = assemble(&ctx, &t, &markers, &w, crate::cinema::Continuity::None);
        let issues = check(&ctx, &RoutePrompts { video: v, frame: "f".into(), ..Default::default() });
        assert!(
            !issues.iter().any(|i| i.contains("半个")),
            "「半个」是长句的子串，不该被判成重复：{issues:?}"
        );
    }

    /// 同一句台词在一镜里出现两次是**合法的**（强调 / 回声）——
    /// 判据是「块数 == 剧本里的条数」，不是「== 1」。
    #[test]
    fn 剧本里真的重复两遍时两个块都合格() {
        let a = assets();
        let mut s = shot();
        s.character_ids = vec!["char_emperor".into()];
        s.dialogue_lines = vec![
            DialogueLine { speaker: "皇帝".into(), line: "走。".into(), ..Default::default() },
            DialogueLine { speaker: "皇帝".into(), line: "走。".into(), ..Default::default() },
        ];
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let t = tags(&ctx);
        let markers = line_markers(&ctx, &t);
        let two = Wording {
            detailed_description: "[Shot 1] <Subject 1> (S1) says, [LINE 1] then [LINE 2]".into(),
            frame: "f".into(),
            ..Default::default()
        };
        let v = assemble(&ctx, &t, &markers, &two, crate::cinema::Continuity::None);
        let ok = check(&ctx, &RoutePrompts { video: v.clone(), frame: "f".into(), ..Default::default() });
        assert!(!ok.iter().any(|i| i.contains("走")), "剧本里就是两句，两个块合格：{ok:?}");

        // 少写一次要报。**不能走 assemble** —— 它会自动补齐模型漏掉的台词
        // （见 `模型漏写台词占位_代码补在声音之前_自己抄了台词要报`），
        // 所以那条路上永远到不了「少了一次」。这里直接构造正文 ——
        // `check` 本来就也给界面上手改过的提示词用。
        let one_block = v.replacen("<d>[Chinese] 走。</d>", "", 1);
        let short = check(&ctx, &RoutePrompts { video: one_block, frame: "f".into(), ..Default::default() });
        assert!(
            short.iter().any(|i| i.contains("只写了 1 次")),
            "剧本里两句、正文只剩一句，要报出来：{short:?}"
        );
    }

    #[test]
    fn 远景和大远景加距离规则_其他景别不加() {
        let a = assets();
        for (scale, want) in [("wide", true), ("extreme_wide", true), ("full_body", false), ("medium", false), ("close_up", false)] {
            let mut s = shot();
            s.scale = scale.into();
            let brief = shot_brief(&ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false });
            assert_eq!(brief.contains("Distance rule"), want, "{scale}：{brief}");
        }
    }

    /// 写实远景要躲脸 + 压景深；动画远景允许正脸，只要求表情做大
    #[test]
    fn 写实远景躲脸压景深_动画远景可以给正脸() {
        let real = distant_face_rule("wide", "cinematic photorealism, live-action film still").expect("写实远景要有规则");
        assert!(real.contains("NOT turned toward the camera") && real.contains("shallow depth of field"), "{real}");
        let toon = distant_face_rule("extreme_wide", "modern cel-shaded anime, clean linework").expect("动画远景也有规则");
        assert!(toon.contains("faces may be turned toward the camera") && !toon.contains("shallow depth of field"), "{toon}");
        // 近景两边都不加
        assert!(distant_face_rule("close_up", "cinematic photorealism").is_none());
    }

    /// **每个内置风格预设都要判对。** 判错不报错：动画项目被按写实处理，远景全变背影，
    /// 白白丢掉表演；写实项目被当动画，远景的脸照样扭。
    #[test]
    fn 内置风格预设的画风都判得对() {
        const SEMI_REALISTIC_PRESETS: &[&str] = &["bjd_3d_guoman", "xianxia_3d_guoman"];
        for p in crate::style::all_presets() {
            let got = is_live_action(&p.positive_prompt);
            // BJD 人偶国漫不是实拍，但它是**类真人半写实**：五官细节多，远处的脸一样会扭，
            // 远景要按写实处理（躲脸、压景深）。3D 卡通那种色块五官才适合远景给正脸。
            let want = p.category == "live_action" || SEMI_REALISTIC_PRESETS.contains(&p.id.as_str());
            assert_eq!(got, want, "{}（{}，{}）判成了{}", p.id, p.category, p.name_zh, if got { "写实" } else { "动画" });
        }
    }

    #[test]
    fn 认画风_动画词优先于电影感() {
        for s in ["cinematic photorealism, live-action film still", "Japanese live-action film look", "超写实国风，质感光照", "真人实拍质感"] {
            assert!(is_live_action(s), "{s} 应该判成写实");
        }
        for s in [
            "modern cel-shaded anime, clean linework",
            "stylized feature-quality 3D animation, expressive rounded characters",
            // 动画风格里也常写 cinematic：不能因为这个词判成真人
            "warm hand-drawn animated film look, cinematic lighting, painterly watercolor",
            "Chinese ink fantasy illustration, flowing ink wash",
            "黏土定格动画",
            "",
        ] {
            assert!(!is_live_action(s), "{s} 不该判成写实");
        }
    }

    #[test]
    fn 项目风格写进给模型看的镜头() {
        let s = shot();
        let a = assets();
        let brief = shot_brief(&ShotContext { shot: &s, prev: None, assets: &a, style: "水墨国风", long_take: false });
        assert!(brief.starts_with("Visual style chosen for the whole project") && brief.contains("水墨国风"), "{brief}");
        assert!(!shot_brief(&ShotContext { shot: &s, prev: None, assets: &a, style: " ", long_take: false }).contains("Visual style"));
    }

    #[test]
    fn 一镜出的镜头告诉写手可以硬切_普通镜头不说() {
        let mut s = shot();
        let a = assets();
        let plain = shot_brief(&ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false });
        assert!(!plain.contains("ONE-TAKE"), "普通镜头不能放开切镜：{plain}");
        s.one_take = true;
        let brief = shot_brief(&ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false });
        assert!(brief.starts_with("ONE-TAKE MONTAGE"), "要放在第一行，盖过模板里的「不切镜」：{brief}");
        // 切镜按官方写新的 [Shot N]（基础指南 4.2）
        assert!(brief.contains("[Shot 2] At 00:03.500, the camera cuts to"), "{brief}");
        // 指纹跟着变 —— 改成一镜出之后旧提示词要判过期
        let h = |s: &StoryboardShot| crate::prompt_input::fields_hash(&ShotContext { shot: s, prev: None, assets: &a, style: "", long_take: false });
        let mut off = s.clone();
        off.one_take = false;
        assert_ne!(h(&s), h(&off));
    }

    /// **时间写法按官方**（基础指南 4.2 / ref 指南 5.1，2026-10-06）：`[Shot 1]` 不带时间码；
    /// 镜内时刻写成 `Around the N.N-second mark,`（官方 `At MM:SS.mmm` 只标切镜）；一镜出的切镜写成 `[Shot N] At 00:SS.mmm, the camera cuts to`。
    #[test]
    fn 时间写法规整成官方的() {
        let d = "Style. [Shot 1] At 00:00.0, a hall. At 00:03.0, he rises. At 00:05.5, he says <d>[Chinese] 走</d>.";
        assert_eq!(
            official_timing(d, false),
            "Style. [Shot 1] a hall. Around the 3.0-second mark, he rises. Around the 5.5-second mark, he says <d>[Chinese] 走</d>."
        );
        // 一镜出：写手照老习惯写的硬切换成官方切镜；已经按官方写的只规整三位小数
        let cut = "[Shot 1] a hall. At 00:03.0, hard cut to a close-up of her hands. [Shot 3] At 00:06.5, the camera cuts to the door.";
        assert_eq!(
            official_timing(cut, true),
            "[Shot 1] a hall. [Shot 2] At 00:03.000, the camera cuts to a close-up of her hands. [Shot 3] At 00:06.500, the camera cuts to the door."
        );
        // 普通镜头不许切：同一句只当镜内时刻
        assert!(official_timing(cut, false).contains("Around the 3.0-second mark, hard cut to"));
        // 不是时钟的 At 原样
        assert_eq!(official_timing("[Shot 1] At dawn, rain.", false), "[Shot 1] At dawn, rain.");
    }

    /// 时刻解析认新旧两种写法，不把对齐指令 / 长镜头说话时限当节拍
    #[test]
    fn 时刻解析认新旧写法_不认对齐指令() {
        assert_eq!(timecodes("[Shot 1] x. Around the 3.0-second mark, y. At 00:05.5, z. [Shot 2] At 00:07.250, the camera cuts to w."), vec![3.0, 5.5, 7.25]);
        assert!(timecodes("aligns with the 8.00-second mark of the target video. All speech must finish before the 13-second mark.").is_empty());
        assert_eq!(timecodes("At 00:0.5, x"), vec![0.5], "老提示词的一位秒数照样认");
        let segs = vec![
            ShotSegment { start: 0.0, end: 3.0, ..Default::default() },
            ShotSegment { start: 3.0, end: 7.5, ..Default::default() },
            ShotSegment { start: 7.5, end: 12.0, ..Default::default() },
        ];
        assert!(segment_coverage(&segs, "[Shot 1] x. Around the 3.0-second mark, y. Around the 7.5-second mark, z.").is_empty());
        let t = line_timings("[Shot 1] x. Around the 2.5-second mark, he says, <d>[Chinese] 你好啊</d>");
        assert_eq!(t.first().map(|t| t.at), Some(2.5));
    }

    /// 画面文字：拼装时「」换成英文双引号（官方 4.5），**台词里的不动**（官方 4.4 逐字）；检查认双引号里的中文
    #[test]
    fn 画面文字用英文双引号_台词里的引号不动() {
        assert_eq!(
            screen_quotes("A note reads 「未读消息」. She says <d>[Chinese] 你屋里的「玩具」跑出来了！</d>"),
            "A note reads \"未读消息\". She says <d>[Chinese] 你屋里的「玩具」跑出来了！</d>"
        );
        assert_eq!(stray_chinese("A neon sign reading \"营业中\" glows above the door."), None);
        assert_eq!(stray_chinese("He says \"hello\" and 走了。").as_deref(), Some("走了。"));
        assert_eq!(screen_text_spans("A sign reading \"营业中\" and a \"plain\" word"), vec!["营业中".to_string()]);
    }

    /// **纯文生的说话人不能都是画外音**（2026-10-06 查出的老 bug）：画面里的人写成 `the 40-year-old man (S1)`，
    /// 不在画面里的才是 `an off-screen …'s voice (S2)`（官方「a stable voice description followed by (Sx)」）
    #[test]
    fn 文生和没挂图的说话人按画面里看不看得见写() {
        let mut s = shot();
        s.ref_slots.clear();
        s.character_ids = vec!["char_emperor".into()];
        s.dialogue_lines.push(DialogueLine { speaker: "旁白".into(), line: "那一年。".into(), off_screen: true, ..Default::default() });
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let t = tags(&ctx);
        assert_eq!(mode(&ctx, &t), crate::h3_spec::Mode::T2va);
        let m = line_markers(&ctx, &t);
        assert!(m[0].who.starts_with("the 40-year-old man (S1)"), "{}", m[0].who);
        assert_eq!(m[0].verb, SAYS);
        assert!(m[1].who.starts_with("an off-screen speaker's voice (S2)"), "{}", m[1].who);
        assert!(m[1].lips.is_empty(), "画面里没有旁白这个人，不写嘴唇");
        // 拼出来是官方 T2VA：没有对齐指令，直接三字段
        let w = Wording { detailed_description: "Live-action. [Shot 1] He stands.".into(), overall_soundscape: "wind".into(), frame: "f".into(), ..Default::default() };
        let v = assemble(&ctx, &t, &m, &w, crate::cinema::Continuity::None);
        assert!(v.starts_with("integrated_multimodal_description: [Shot 1] Live-action. He stands."), "{v}");
    }

    /// 写手自己带了任务前缀不重复（2026-10-06 EP03 第 2 镜：`[reference generation] [reference generation] …`）
    #[test]
    fn summary_任务前缀不重复() {
        assert_eq!(strip_task_prefix("[reference generation] A single take."), "A single take.");
        assert_eq!(strip_task_prefix("[video continuation + reference generation] [reference generation] x"), "x");
        assert_eq!(strip_task_prefix("[Shot 1] stays"), "[Shot 1] stays", "不是任务前缀的方括号不动");
        let a = assets();
        let s = shot();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let w = Wording { summary: "[reference generation] The emperor speaks.".into(), detailed_description: "[Shot 1] x".into(), overall_soundscape: "y".into(), frame: "f".into(), ..Default::default() };
        let v = assemble(&ctx, &tags(&ctx), &[], &w, crate::cinema::Continuity::None);
        assert!(v.contains("summary:\n[reference generation] The emperor speaks."), "{v}");
    }

    /// 质感锁按出片方式写：首帧出片锁到 <Picture 1>；一镜出（允许硬切）说「切了也不变」；参考图出片的普通镜头不加。
    /// 2026-10-06 起是正文里的一句英文（不再是 subject_definitions 里的一行，那一段官方只放标签定义）
    #[test]
    fn 质感锁按出片方式写() {
        let a = assets();
        let notes = |s: &StoryboardShot, long_take: bool| {
            let ctx = ShotContext { shot: s, prev: None, assets: &a, style: "", long_take };
            notes_for_body(&ctx, mode(&ctx, &tags(&ctx))).join(" ")
        };
        let mut s = shot();
        assert_eq!(notes(&s, false), "", "参考图出片的普通镜头不加");
        s.one_take = true;
        assert!(notes(&s, false).contains("Across every cut") && !notes(&s, false).contains("<Picture 1>"));
        s.keyframes = crate::cinema::Keyframes { first: true, last: false };
        assert!(notes(&s, false).contains("as in <Picture 1> for the entire video, including after every cut"));
        s.one_take = false;
        assert!(notes(&s, false).contains("one continuous take with no cuts"), "{}", notes(&s, false));
        // 长镜头里挂了首帧也不走首帧出片：按参考图出片算，只有窗口说话时限
        let lt = notes(&s, true);
        assert!(!lt.contains("<Picture 1>") && lt.contains("one window of a continuous long take"), "{lt}");
        // 拼出来在正文里：基础模式在 [Shot 1] 开头、画风后面
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let w = Wording { detailed_description: "Gritty 3D CG. [Shot 1] He turns.".into(), overall_soundscape: "rain".into(), frame: "f".into(), ..Default::default() };
        let v = assemble(&ctx, &tags(&ctx), &[], &w, crate::cinema::Continuity::None);
        assert!(v.contains("integrated_multimodal_description: [Shot 1] Gritty 3D CG. This is one continuous take with no cuts;"), "{v}");
    }

    #[tokio::test]
    async fn 同一场紧接上一镜_告诉模型保持光线道具服装() {
        let mut prev = shot();
        prev.scene_no = 2;
        prev.segments[0].visual = "皇帝抬手".into();
        prev.lighting = "龙椅处最亮".into();
        let mut s = shot();
        s.scene_no = 2;
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: Some(&prev), assets: &a, style: "", long_take: false };
        let lm = FakeLm::new(vec![good_reply()]);
        synthesize(&ctx, |slot, v| lm.call(slot, v), 1).await.unwrap();
        let seen = lm.seen.lock().unwrap();
        let p = &seen[0].iter().find(|(k, _)| k == "PREV").unwrap().1;
        assert!(p.starts_with("皇帝抬手\nSame scene, continuing directly") && p.contains("keep the same lighting (龙椅处最亮)") && p.contains("costume color"), "{p}");
    }

    /// 实跑第 11 镜：只挂了 1 张图、画面里 3 个人，模型给没挂图的人编了标签。
    #[test]
    fn 没挂图的人物明说不许给标签() {
        let mut s = shot();
        s.ref_slots.retain(|r| r.asset_id == "char_emperor");
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let brief = shot_brief(&ctx);
        assert!(brief.contains("Character in frame: 皇帝 = <Subject 1>"), "{brief}");
        assert!(brief.contains("Character in frame WITHOUT a tag (describe in words, never give it a <Subject> tag): 林婉清（盛唐）"), "{brief}");
    }

    /// 实跑第 11 镜：只挂了场景图，模型把 <Subject 1> 当成女主，又给另外两个人编了 2、3。
    #[test]
    fn 标签表写明是人还是地点_并列出允许的标签() {
        let mut s = shot();
        s.ref_slots.retain(|r| r.asset_id == "scene_palace");
        s.ref_slots[0].order = 1;
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let table = tag_table(&tags(&ctx));
        assert!(table.contains("<Subject 1> = 金銮殿, which is a PLACE, not a person"), "{table}");
        assert!(table.ends_with("Allowed tags in this shot: <Subject 1>, <Picture 1>. Any other tag number is an error; everyone else is described in plain words."), "{table}");
        let full = tag_table(&tags(&ShotContext { shot: &shot(), prev: None, assets: &a, style: "", long_take: false }));
        assert!(full.contains("<Subject 1> = 皇帝, which is a PERSON"), "{full}");
    }

    /// 用户要求的兜底：没图的不给标签、按文字写；库里没有的也写进画面；编号跟着实际挂的图走。
    #[tokio::test]
    async fn 没图和库里没有的按文字兜底_编号跟实际挂图走() {
        use crate::shot_table::UnlinkedAsset;
        let mut a = assets();
        a[1].has_image = false; // 皇帝（槽位 1）还没图
        let mut s = shot();
        s.unlinked_assets = vec![UnlinkedAsset { kind: "prop".into(), name: "玉笏".into() }];
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let t = tags(&ctx);
        assert_eq!(t.iter().map(|t| (t.picture, t.name.as_str())).collect::<Vec<_>>(), [(1, "林婉清（盛唐）"), (2, "金銮殿")], "皇帝没图不挂，后面往前补");
        let brief = shot_brief(&ctx);
        assert!(brief.contains("Character in frame WITHOUT a tag (describe in words, never give it a <Subject> tag): 皇帝"), "{brief}");
        assert!(brief.contains("Object in frame, not in the asset library, no reference image (describe in words, never give it a tag): 玉笏"), "{brief}");
        // 正文里用了 <Subject 3>（按原来 3 张图的编号）要报 —— 现在只挂了 2 张
        let p = RoutePrompts { video: "subject_definitions:\nx\n\nsummary:\nx\n\nretention_analysis:\nx\n\ndetailed_description:\n[Shot 1] <Subject 3> <d>[Chinese] 既言才华盖世，当殿作诗。若不能——斩。</d>\n\noverall_soundscape: x\n\nnon_diegetic_music: N/A".into(), frame: "f".into(), ..Default::default() };
        assert!(check(&ctx, &p).iter().any(|i| i.contains("<Subject 3>")), "{:?}", check(&ctx, &p));
    }

    #[tokio::test]
    async fn 没有参考图时写官方_base_三段() {
        let mut s = shot();
        s.ref_slots.clear();
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let reply = good_reply().replace("<Subject 1> (S1)", "(S1) the emperor").replace("<Subject 1>", "the emperor").replace("<Subject 2>", "the woman");
        let lm = FakeLm::new(vec![reply]);
        let (p, _soft) = synthesize(&ctx, |slot, v| lm.call(slot, v), 1).await.unwrap();
        assert!(p.video.starts_with("integrated_multimodal_description: [Shot 1] A realistic cinematic style"), "风格句要挪到 [Shot 1] 之后：{}", p.video);
        assert!(!p.video.contains("subject_definitions"));
        assert_eq!(check(&ctx, &p), Vec::<String>::new());
    }

    #[test]
    fn 结构检查挡住没定义的标签_缺配乐行_漏台词() {
        let s = shot();
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let p = RoutePrompts { video: "subject_definitions:\n<Subject 4> x\n\nsummary:\nx\n\nretention_analysis:\nx\n\ndetailed_description:\n[Shot 1] <Picture 9>\n\noverall_soundscape: x".into(), frame: "f".into(), ..Default::default() };
        let issues = check(&ctx, &p);
        assert!(issues.iter().any(|i| i.contains("<Subject 4>")), "{issues:?}");
        assert!(issues.iter().any(|i| i.contains("<Picture 9>")), "{issues:?}");
        assert!(issues.iter().any(|i| i.contains("non_diegetic_music")), "{issues:?}");
        assert!(issues.iter().any(|i| i.contains("既言才华盖世")), "{issues:?}");
    }

    #[test]
    fn 起始帧只当帧锚点_不占主体编号() {
        let mut s = shot();
        s.ref_slots.insert(0, RefSlot { order: 1, asset_id: "frame".into(), usage: RefUsage::StartFrame, control: "opening composition".into(), not_control: String::new() });
        for (i, slot) in s.ref_slots.iter_mut().enumerate() {
            slot.order = i as u8 + 1;
        }
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let t = tags(&ctx);
        assert_eq!(t.iter().map(|t| (t.picture, t.subject)).collect::<Vec<_>>(), [(1, None), (2, Some(1)), (3, Some(2)), (4, Some(3))]);
        let w = Wording { summary: "s".into(), detailed_description: "d".into(), overall_soundscape: "o".into(), ..Default::default() };
        let v = assemble(&ctx, &t, &[], &w, crate::cinema::Continuity::None);
        assert!(v.contains("summary:\n[reference generation + keyframe completion]"), "{v}");
        // 官方帧锚点写法（ref 指南 2.2 / 4.1）
        assert!(v.contains("<Picture 1> is the first frame of [Shot 1]: opening composition."), "{v}");
        assert!(v.contains("<Picture 1> ([Shot 1] first frame): fully_preserved - opening composition."), "{v}");
    }

    /// **挂了首帧：只挂首帧（和尾帧），人物从首帧里认成主体，开头是官方对齐指令。**
    ///
    /// 用户 2026-09-27：首帧 / 首尾帧按导入情况自动识别。
    #[tokio::test]
    async fn 挂了首帧_走首帧出片_人物从首帧认_不接参考视频() {
        let mut s = shot();
        s.keyframes = crate::cinema::Keyframes { first: true, last: false };
        // 接上一镜在关键帧出片时不生效（FL2VA 分区不吃参考视频）
        s.continuity = crate::cinema::Continuity::RefVideo;
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let slots = ctx.slots();
        assert_eq!(slots.len(), 1, "资产图不挂，只挂首帧");
        assert_eq!(slots[0].0.usage, RefUsage::StartFrame);
        let t = tags(&ctx);
        // 官方 I2VA 没有 <Subject N>：只有首帧这一张图
        assert_eq!(t.iter().map(|t| (t.picture, t.subject, t.name.as_str())).collect::<Vec<_>>(), [(1, None, "")]);
        let brief = shot_brief(&ctx);
        assert!(brief.contains("Scene (no reference image; describe it in words): 金銮殿"), "场景不挂图就按文字写：{brief}");
        assert!(brief.contains("Person already visible in <Picture 1> (the first frame; no tag in this format): 皇帝"), "{brief}");

        let lm = FakeLm::new(vec![keyframe_reply()]);
        let (p, _) = synthesize(&ctx, |slot, v| lm.call(slot, v), 7).await.unwrap();
        let v = &p.video;
        assert!(v.starts_with("For the target video, at 0.00 seconds into the target video, <Picture 1> (from [Shot 1]) is fully referenced.\n\nintegrated_multimodal_description: [Shot 1] "), "{v}");
        assert!(!v.contains("subject_definitions") && !v.contains("summary:") && !v.contains("<Subject"), "{v}");
        assert!(v.contains("the 40-year-old man shown in <Picture 1> (S1)"), "说话人是首帧里的人，不是画外音：{v}");
        assert!(!v.contains("<Video 1>"), "关键帧出片不接参考视频：{v}");
        // 质感锁是正文开头的一句话（2026-10-04 首帧出片 3 秒硬切后变亮变锐）
        assert!(v.contains("This is one continuous take with no cuts; the exposure, brightness, contrast, colour grade, sharpness and film grain stay exactly as in <Picture 1> from the first frame to the last."), "{v}");
        assert_eq!(check(&ctx, &p), Vec::<String>::new());
        let seen = lm.seen.lock().unwrap();
        let var = |k: &str| seen[0].iter().find(|(n, _)| n == k).unwrap().1.clone();
        let tag_table = var("TAGS");
        assert!(tag_table.contains("KEYFRAME SHOT: the video starts exactly on <Picture 1>."), "{tag_table}");
        assert!(tag_table.contains("Allowed tags in this shot: <Picture 1>."), "{tag_table}");
        // 写手拿到的是这种生产方式对应的官方规范原文
        assert_eq!(var("H3_MODE"), "I2VA");
        assert!(var("H3_SPEC").contains("### 3.1 I2VA: Begin from the Image and Develop Forward") && !var("H3_SPEC").contains("### 3.2 FL2VA"));
    }

    /// **首帧出片：开场构图就是首帧，不照镜头表写开场景别。**（2026-10-04 EP02 第 2 镜运镜卡顿）
    #[test]
    fn 首帧出片不写开场景别() {
        let mut s = shot();
        s.scale = "close_up".into();
        s.camera_angle = "low_angle".into();
        s.keyframes = crate::cinema::Keyframes { first: true, last: false };
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let brief = shot_brief(&ctx);
        assert!(brief.contains("do NOT write it as the opening; the opening is <Picture 1>") && !brief.contains("\nCamera: "), "{brief}");
        assert!(tag_table(&tags(&ctx)).contains("THE OPENING FRAMING IS <Picture 1> ITSELF"));
        let wording = |d: &str| Wording { subjects: vec![], summary: "x".into(), detailed_description: d.into(), overall_soundscape: "y".into(), frame: "f".into() };
        let soft = |ctx: &ShotContext, d: &str| check_wording(ctx, &[], &wording(d), d).soft.into_iter().filter(|m| m.contains("开场构图")).collect::<Vec<_>>();
        // 当开场写的景别 / 机位：报
        let bad = "[Shot 1] At 00:00.0, the video begins exactly on <Picture 1>. In one continuous take, a low-angle close-up looks upward from her boots. At 00:00.8, the camera tilts up.";
        let hit = soft(&ctx, bad);
        assert_eq!(hit.len(), 1, "{hit:?}");
        assert!(hit[0].contains("low-angle") && hit[0].contains("close-up"), "{hit:?}");
        // 终点的景别、第一个时间码之后的不报；中文台词旁边不崩
        let ok = "[Shot 1] At 00:00.0, the video begins exactly on <Picture 1>; the camera pushes in slowly and settles into a two-person medium shot. 她说<d>[Chinese] 主人早上好</d> At 00:03.0, a close-up of the bowl.";
        assert!(soft(&ctx, ok).is_empty(), "{:?}", soft(&ctx, ok));
        // 参考图出片（没挂首帧）：开场景别本来就要写，不报
        let mut r = s.clone();
        r.keyframes = crate::cinema::Keyframes::NONE;
        let rctx = ShotContext { shot: &r, prev: None, assets: &a, style: "", long_take: false };
        assert!(soft(&rctx, bad).is_empty());
        assert!(shot_brief(&rctx).contains("Camera: "));
    }

    /// **首帧里没有的人不能说成「首帧里的某某」。**
    ///
    /// 2026-10-05 EP02 第 11 镜：陆川 3.5 秒才从屋里冲出来，首帧里没有他，可人物表里的每个人
    /// 都被当成首帧里看得见的人。写手只好编「<Subject 2> 是首帧南边门口那个男人」，
    /// H3 在图里找了个邻居演陆川，把星澜推进了屋。
    #[test]
    fn 关键帧出片_首帧里没有的人不给标签_按文字写() {
        let mut s = shot();
        s.keyframes = crate::cinema::Keyframes { first: true, last: false };
        s.segments = vec![
            ShotSegment { start: 0.0, end: 4.0, title: "圣上下令".into(), visual: "皇帝抬手，殿内群臣低头".into(), ..Default::default() },
            ShotSegment { start: 4.0, end: 10.0, title: "闯殿".into(), visual: "林婉清从殿门冲进来跪下".into(), blocking: "林婉清从画面左侧入画".into(), ..Default::default() },
        ];
        s.stage.people = vec!["皇帝：坐在龙椅上".into(), "林婉清：在殿外，尚未出现在殿内".into()];
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        assert_eq!(s.absent_from_opening("林婉清"), Some(crate::cinema::OpeningAbsence::EntersLater));
        assert_eq!(s.absent_from_opening("皇帝"), None);
        let t = tags(&ctx);
        assert!(t.iter().all(|t| t.subject.is_none()), "官方 I2VA 没有主体标签：{t:?}");
        let brief = shot_brief(&ctx);
        assert!(brief.contains("Person already visible in <Picture 1> (the first frame; no tag in this format): 皇帝"), "{brief}");
        assert!(brief.contains("Character NOT in <Picture 1> — enters partway through the shot (no tag, no reference image): 林婉清（盛唐）"), "{brief}");
        assert!(!brief.contains("Person already visible in <Picture 1> (the first frame; no tag in this format): 林婉清"), "{brief}");
        // 说话人写法：中途进场的「who enters later」，不说成首帧里的人
        let m = line_markers(&ctx, &t);
        assert!(m.iter().all(|m| !m.who.contains("shown in <Picture 1>") || m.who.contains("man")), "{:?}", m.iter().map(|m| &m.who).collect::<Vec<_>>());
        // 中途进场的人长相只能靠文字：资产描述要给
        let shen = a.iter().find(|x| x.id == "char_shen").unwrap();
        assert!(brief.contains(&short_description(shen, CHAR_DESC_MAX)), "{brief}");

        // 只出声的：开场没写到、台词全是画外 → 不给标签，说话人写成画外音
        let mut v = s.clone();
        v.stage.people.clear();
        v.segments[1].visual = "殿门紧闭".into();
        v.segments[1].blocking.clear();
        v.dialogue_lines.push(DialogueLine { speaker: "林婉清".into(), line: "臣女在此。".into(), off_screen: true, ..Default::default() });
        assert_eq!(v.absent_from_opening("林婉清"), Some(crate::cinema::OpeningAbsence::VoiceOnly));
        let vctx = ShotContext { shot: &v, prev: None, assets: &a, style: "", long_take: false };
        let brief = shot_brief(&vctx);
        assert!(brief.contains("Off-screen voice only, NOT in <Picture 1> and never seen in this shot (no tag): 林婉清（盛唐）"), "{brief}");
        let vt = tags(&vctx);
        let m = line_markers(&vctx, &vt);
        assert!(m.iter().any(|m| m.line == "臣女在此。" && m.who.starts_with("an off-screen") && m.who.contains("voice (S")), "{:?}", m.iter().map(|m| &m.who).collect::<Vec<_>>());

        // 开场写到了的人，哪怕台词是画外（内心独白），也在画面里
        let mut w = v.clone();
        w.segments[0].visual = "皇帝抬手，林婉清跪地不语".into();
        assert_eq!(w.absent_from_opening("林婉清"), None);
        // 站位里没说开场不在的，后面才点名也当在画面里（群演常常开场不点名）
        let mut g = s.clone();
        g.stage.people = vec!["林婉清：跪在殿中".into()];
        assert_eq!(g.absent_from_opening("林婉清"), None);
        // 不是首帧出片：照旧按参考图挂标签
        let mut r = s.clone();
        r.keyframes = crate::cinema::Keyframes::NONE;
        let rctx = ShotContext { shot: &r, prev: None, assets: &a, style: "", long_take: false };
        assert!(tags(&rctx).iter().any(|t| t.name == "林婉清（盛唐）" && t.subject.is_some()));
        assert!(!shot_brief(&rctx).contains("NOT in <Picture 1>"));
    }

    /// **关键帧出片：长相只看首帧，资产的文字描述不给写手。**
    ///
    /// EP01 第 8 镜（2026-09-28）：资产描述是旧的「粉白出厂服」，首帧照 v8 设定卡画的纱裙吊袜。
    /// 写手照描述写了「powder-white factory companion」，和身后那排仿生人同一个写法，
    /// 出片第 9 帧起星澜就换成了那排仿生人的衣服。
    #[test]
    fn 关键帧出片_长相只看首帧_不带资产描述() {
        let mut s = shot();
        s.keyframes = crate::cinema::Keyframes { first: true, last: false };
        let mut a = assets();
        a.push(AssetRef { id: "char_prop".into(), kind: AssetKind::Prop, name: "协议铭牌".into(), has_image: false, description: "椭圆铭牌，透出粉光".into(), look_id: String::new(), looks: Vec::new(), h3_voice: String::new() });
        s.prop_ids = vec!["char_prop".into()];
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let brief = shot_brief(&ctx);
        assert!(brief.contains("Person already visible in <Picture 1> (the first frame; no tag in this format): 皇帝"), "{brief}");
        assert!(!brief.contains("盛唐皇帝，男性"), "资产描述不能进关键帧出片的说明：{brief}");
        assert!(brief.contains("if it is already visible in <Picture 1>, its look (shape, colour, glow) comes from the picture"), "{brief}");
        let t = tags(&ctx);
        assert!(t.iter().all(|t| t.subject.is_none()), "官方 I2VA 没有 <Subject N>：{t:?}");
        let table = tag_table(&t);
        assert!(table.contains("LOOKS COME ONLY FROM <Picture 1>"), "{table}");
        assert!(table.contains("this format has NO <Subject N> tags"), "{table}");
        assert!(!table.contains("盛唐女官") && !table.contains("盛唐皇帝"), "资产描述不能进标签表：{table}");
        let plain = long_desc().replace("<Subject 1>", "the emperor in <Picture 1>").replace("<Subject 2>", "the woman");
        let w = Wording { summary: "s".into(), detailed_description: plain.clone(), overall_soundscape: "o".into(), frame: "f".into(), ..Default::default() };
        let markers = line_markers(&ctx, &t);
        let v = assemble(&ctx, &t, &markers, &w, crate::cinema::Continuity::None);
        // **官方 I2VA**（基础指南 2.1 / Case 2）：第一行对齐指令，空一行，三字段；没有六段、没有 <Subject N>
        assert!(v.starts_with(&format!("{}\n\nintegrated_multimodal_description: [Shot 1] ", crate::h3_spec::I2VA_INSTRUCTION)), "{v}");
        assert!(!v.contains("subject_definitions") && !v.contains("<Subject"), "{v}");
        assert!(v.contains("\n\noverall_soundscape: o\n\nnon_diegetic_music: N/A"), "{v}");
        assert!(check(&ctx, &RoutePrompts { video: v.clone(), frame: "f".into(), ..Default::default() }).is_empty(), "{:?}", check(&ctx, &RoutePrompts { video: v.clone(), frame: "f".into(), ..Default::default() }));
        // 第一行不是官方对齐指令要报
        let no_head = v.replacen(crate::h3_spec::I2VA_INSTRUCTION, "", 1);
        assert!(check(&ctx, &RoutePrompts { video: no_head, frame: "f".into(), ..Default::default() }).iter().any(|m| m.contains("I2VA 对齐指令")));
        // 写手照老习惯写了 <Subject 1>：这个格式里没有，要报
        let bad = Wording { detailed_description: format!("{plain} <Subject 1> waves."), ..w.clone() };
        let vb = assemble(&ctx, &t, &markers, &bad, crate::cinema::Continuity::None);
        assert!(check(&ctx, &RoutePrompts { video: vb, frame: "f".into(), ..Default::default() }).iter().any(|m| m.contains("<Subject 1>")));
        // **首尾帧 = 官方 FL2VA**：对齐指令逐字（Picture / Shot 不带括号），尾帧对齐成片时长
        let mut fl = s.clone();
        fl.keyframes = crate::cinema::Keyframes { first: true, last: true };
        let fctx = ShotContext { shot: &fl, prev: None, assets: &a, style: "", long_take: false };
        let vf = assemble(&fctx, &tags(&fctx), &line_markers(&fctx, &tags(&fctx)), &w, crate::cinema::Continuity::None);
        assert!(vf.starts_with(&format!("{}\n\nintegrated_multimodal_description:", crate::h3_spec::fl2va_instruction(1, fl.duration))), "{vf}");
        assert!(check(&fctx, &RoutePrompts { video: vf, frame: "f".into(), ..Default::default() }).is_empty());
        // 不是关键帧出片：资产描述照旧给
        s.keyframes = crate::cinema::Keyframes::default();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        assert!(shot_brief(&ctx).contains("盛唐皇帝，男性"));
    }

    #[tokio::test]
    async fn 首尾帧_对齐指令写尾帧的秒数_长镜头窗口不走关键帧() {
        let mut s = shot();
        s.keyframes = crate::cinema::Keyframes { first: true, last: true };
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        assert_eq!(ctx.slots().iter().map(|(x, _)| x.usage).collect::<Vec<_>>(), [RefUsage::StartFrame, RefUsage::EndFrame]);
        let lm = FakeLm::new(vec![keyframe_reply()]);
        let (p, _) = synthesize(&ctx, |slot, v| lm.call(slot, v), 7).await.unwrap();
        // 官方 FL2VA 对齐指令逐字：这一行 Picture / Shot 不带括号（基础指南 2.1）
        assert!(p.video.starts_with("How the reference pictures align with the target video — Picture 1 (from Shot 1) aligns with the 0.00-second mark of the target video; Picture 2 (from Shot 1) aligns with the 10.00-second mark of the target video.\n\nintegrated_multimodal_description: [Shot 1] "), "{}", p.video);
        assert_eq!(check(&ctx, &p), Vec::<String>::new());
        // 只有尾帧 = 不按关键帧出（出片那边会拦下让补首帧）
        s.keyframes = crate::cinema::Keyframes { first: false, last: true };
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        assert!(!ctx.keyframe_video());
        // 长镜头窗口整段出，不吃关键帧
        s.keyframes = crate::cinema::Keyframes { first: true, last: false };
        let lt = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: true };
        assert!(!lt.keyframe_video());
        assert!(lt.slots().iter().all(|(x, _)| x.usage != RefUsage::StartFrame));
    }

    /// 挂 / 摘关键帧要让提示词过期（写法不同）；没挂的老镜头指纹一个字不变。
    #[test]
    fn 关键帧进指纹_没挂时老指纹不变() {
        let mut s = shot();
        let a = assets();
        let before = fields_hash(&ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false });
        assert!(!crate::prompt_input::fields_json(&ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false }).as_object().unwrap().contains_key("keyframes"));
        s.keyframes.first = true;
        let first = fields_hash(&ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false });
        s.keyframes.last = true;
        let both = fields_hash(&ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false });
        assert!(before != first && first != both);
    }

    /// 两条线路物理隔离：不互相 import、不共用拼装函数。
    #[test]
    fn 不引用线路_a() {
        let src = include_str!("prompt_route_b.rs");
        let prod: String = src[..src.find("#[cfg(test)]").unwrap()].lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
        assert!(!prod.contains("prompt_route_a"), "线路 B 引用了线路 A");
        assert!(!prod.contains("h3_prompt"), "线路 B 用了旧的拼装模块");
    }

    /// 2026-10-07（EP03 第 2 镜苏晴被画成第二个星澜）：两个人以上的参考镜头，正文开头由代码写「几个人、各一个」
    #[tokio::test]
    async fn 多人参考镜头写明人物基数() {
        let s = shot();
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let lm = FakeLm::new(vec![good_reply()]);
        let (p, _) = synthesize(&ctx, |slot, v| lm.call(slot, v), 7).await.unwrap();
        let v = &p.video;
        let note = "This shot features exactly 2 different people: <Subject 1>, <Subject 2>.";
        let at = v.find(note).unwrap_or_else(|| panic!("没有人物基数句：{v}"));
        let body = v.find("detailed_description:").unwrap();
        assert!(at > body && at < body + v[body..].find("[Shot 1]").unwrap(), "要在正文开头、[Shot 1] 之前：{v}");
        assert!(v.contains("still shows one person"), "三视图卡那句没写");
        // 场景不算人
        assert!(!v[at..at + note.len()].contains("<Subject 3>"));
        // 只有一个人：不写
        let one: Vec<Tag> = tags(&ctx).into_iter().filter(|t| t.name != "皇帝").collect();
        assert_eq!(cast_note(&one), None);
    }

    #[test]
    fn 人的颜色被光改掉要挑出来() {
        // EP03 第 2 镜的原话
        assert!(identity_recolor("The neon flares, and her initially pale face becomes visibly green-cast.").is_some());
        assert!(identity_recolor("Her skin turns blue under the screen light.").is_some());
        // 推荐写法：光落在表面、写明肤色不变
        assert_eq!(identity_recolor("A faint green reflection falls across the shadow side of her beige jacket, while her natural fair skin tone and chestnut hair remain unchanged."), None);
        assert_eq!(identity_recolor("The monitor's cold blue light falls across her pale face."), None, "只是光照在脸上，没说脸变色");
        assert_eq!(identity_recolor("Her face never turns green."), None);
        assert_eq!(identity_recolor("The traffic light turns green."), None, "不是人");
        // 10-07 EP03 第 13 镜误报：颜色词按子串认，`covered` / `layered` 里的 red 也算
        assert_eq!(identity_recolor("At the 9.0-second mark, she presses her shoulder against his, silver hair sliding between them, layered collar covered"), None);
        assert_eq!(identity_recolor("Her hair goes still as the covered lamp sways"), None, "goes 后面的 covered 不是红色");
        assert_eq!(identity_recolor("Her face turns away from the window and the old neon sign glows red"), None, "颜色离「变成」太远不算");
        // 10-07 EP03 第 2 / 13 / 17 镜误报：角色定义里的发色是长相，不是被光改色
        assert_eq!(identity_recolor("subject_definitions:\n<Subject 1> is a robot with waist-length silver-white hair with mint-tinted ends, as shown in <Picture 1>\n"), None);
        assert_eq!(identity_recolor("Her silver hair has mint-tinted ends that catch the light"), None);
        assert_eq!(identity_recolor("<Subject 1> is a robot whose silver hair turns pink toward the ends, as shown in <Picture 1>"), None, "定义行里的「渐变成粉」是长相");
        assert!(identity_recolor("<Subject 1> is a robot.\nUnder the sign her face turns green").is_some(), "定义行之外照查");
        assert!(identity_recolor("Her cheeks take on nothing; her face takes on a sickly greenish glow").is_some());
        assert!(identity_recolor("a green-cast face in the dark").is_some());
    }

    #[test]
    fn 人的颜色被改掉在合成时就喂回写手() {
        let s = shot();
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let mut w: Wording = serde_json::from_str(&good_reply()).unwrap();
        w.detailed_description = w.detailed_description.replace("Sound:", "Her face becomes green-cast in the neon. Sound:");
        let probs = check_wording(&ctx, &tags(&ctx), &w, &w.detailed_description);
        assert!(probs.soft.iter().any(|m| m.contains("人的颜色")), "{probs:?}");
        assert!(!probs.hard.iter().any(|m| m.contains("人的颜色")), "不该让整镜作废");
    }

    /// 定义写成「<Subject N> is …」的六段正文（[`full`] 那份是旧写法，量不到外貌词数）
    fn with_looks(l1: &str, l2: &str) -> RoutePrompts {
        let mut p = full(&long_desc());
        p.video = p.video.replace(
            "<Subject 1>: an emperor\n<Subject 2>: a woman\n",
            &format!("<Subject 1> is {l1}, as shown in <Picture 1>.\n<Subject 2> is {l2}, as shown in <Picture 2>.\n"),
        );
        p
    }

    #[test]
    fn 多人镜头里外貌写薄的要提示() {
        let s = shot();
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let rich = "a broad-shouldered 40-year-old East Asian emperor with a square face, heavy brows, narrow dark eyes, a trimmed black beard, \
                    black hair gathered under a gold crown, wearing a yellow dragon robe with red trim, a jade belt and black boots, \
                    and a scar across his left eyebrow that nobody else has, standing tall with a slow deliberate posture";
        let thin = "a slim 28-year-old woman in an official robe";
        let adv = advisories(&ctx, &with_looks(rich, thin));
        assert!(adv.iter().any(|m| m.contains("「林婉清（盛唐）」的外貌只写了") && m.contains("「皇帝」")), "{adv:?}");
        // 两个差不多厚：不提示
        let adv = advisories(&ctx, &with_looks(thin, "a broad 40-year-old emperor in a dragon robe"));
        assert!(!adv.iter().any(|m| m.contains("外貌只写了")), "{adv:?}");
        // 代码兜底的「照图」不参与比较
        let adv = advisories(&ctx, &with_looks(rich, "the person exactly as shown in <Picture 2>"));
        assert!(!adv.iter().any(|m| m.contains("外貌只写了")), "{adv:?}");
    }

    #[test]
    fn 短镜头里参考人物太多要提示() {
        let mut s = shot();
        let mut a = assets();
        for i in 0..3 {
            a.push(AssetRef { id: format!("char_x{i}"), kind: AssetKind::Character, name: format!("路人{i}"), has_image: true, description: "描述：路人".into(), look_id: String::new(), looks: Vec::new(), h3_voice: String::new() });
            s.ref_slots.push(RefSlot { order: 4 + i as u8, asset_id: format!("char_x{i}"), usage: RefUsage::Identity, control: String::new(), not_control: String::new() });
            s.character_ids.push(format!("char_x{i}"));
        }
        s.duration = 5.0;
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        assert!(advisories(&ctx, &full(&long_desc())).iter().any(|m| m.contains("5 秒里要绑 5 个参考人物")));
        s.duration = 12.0;
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        assert!(!advisories(&ctx, &full(&long_desc())).iter().any(|m| m.contains("参考人物")), "长镜头不提示");
    }

    /// 2026-10-07：正文不许写东南西北（EP03 第 3 镜车门朝西开进墙里）；East Asian 不算
    #[test]
    fn 东南西北要挑出来() {
        assert_eq!(compass_words("A 23-year-old East Asian woman and a Southeast Asian man stand at frame left.").map(|x| x.0), None);
        let (n, around) = compass_words("The orange rolls north toward the northern end, past the east wall and westward.").unwrap();
        assert_eq!(n, 4);
        assert!(around.contains("north"), "{around}");
        assert_eq!(compass_words("Northwest of the door"), Some((1, "northwest of the door".into())));
        assert_eq!(compass_words("the northernmost lamp"), None, "不是方位词的长词不算");
        assert_eq!(compass_words("the western end").map(|x| x.0), Some(1), "western 在提示词里就是方位");
        assert_eq!(compass_words("a Western-style suit and a western style jacket"), None, "风格说法不算");
        // 10-07 EP03 第 6 / 13 镜：画风那句被拦
        assert_eq!(compass_words("thirty-percent refined eastern stylization, eastern aesthetics, Western fashion"), None);
        assert_eq!(compass_words("an eye-level western-side profile").map(|x| x.0), Some(1), "这才是方位");
        // 10-11 E05 第 3 镜（Terra）：画风写成「东方动画美学」
        assert_eq!(compass_words("seventy percent cinematic realism with refined eastern animation aesthetics, Eastern CG charm"), None);
        assert_eq!(compass_words("light enters from the eastern window").map(|x| x.0), Some(1), "窗户在东边：还是方位");
    }

    #[test]
    fn 东南西北在合成时就喂回写手() {
        let s = shot();
        let a = assets();
        let ctx = ShotContext { shot: &s, prev: None, assets: &a, style: "", long_take: false };
        let mut w: Wording = serde_json::from_str(&good_reply()).unwrap();
        w.detailed_description = w.detailed_description.replace("Sound:", "<Subject 2> walks to the west side of the hall. Sound:");
        let probs = check_wording(&ctx, &tags(&ctx), &w, &w.detailed_description);
        assert!(probs.soft.iter().any(|m| m.contains("东南西北")), "{probs:?}");
        assert!(!probs.hard.iter().any(|m| m.contains("东南西北")));
        // 音效段也查（Codex 复查 §23.1）
        let mut w2: Wording = serde_json::from_str(&good_reply()).unwrap();
        w2.overall_soundscape = "Wind from the north end of the hall.".into();
        assert!(check_wording(&ctx, &tags(&ctx), &w2, &w2.detailed_description).soft.iter().any(|m| m.contains("东南西北")));
        // 写手模板里有这条规矩
        assert!(crate::prompts::PromptSlot::RouteBWriter.default_text().contains("Never write compass directions"));
    }
}

#[cfg(test)]
mod voice_prosody_live {
    /// 真模型跑一次「台词语气」（含重音 / 停顿），看能不能过 `parse_voice_directions`。
    /// `ANIMA_VD_ITEMS=/abs/items.txt ../scripts/with-env.sh cargo test --lib 台词语气真模型 -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "调真实的后台模型"]
    async fn 台词语气真模型() {
        let config_dir = dirs::config_dir().unwrap().join("com.anima.desktop");
        let app_cfg = crate::config::AppConfig::load(&config_dir.join("config.json")).unwrap();
        let gateway = crate::llm::gateway::Gateway::new(app_cfg).unwrap();
        let cfg = crate::prompts::load(&config_dir);
        let items = std::fs::read_to_string(std::env::var("ANIMA_VD_ITEMS").unwrap()).unwrap();
        let n = items.lines().count();
        let raw = crate::prompts::run(&gateway, &cfg, crate::prompts::PromptSlot::VoiceDirection, &[("ITEMS", items.as_str())]).await.unwrap();
        println!("{raw}");
        let got = super::parse_voice_directions(&raw, n).expect("没过 parse_voice_directions —— 生产里整镜会退回让写手翻");
        for (i, g) in got.iter().enumerate() {
            println!("{}. ({} 词) {g}", i + 1, g.split_whitespace().count());
        }
    }
}
