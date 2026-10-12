//! 由剧本生成镜头表（分镜重构第 3 步「确认镜头」）。
//!
//! 见 `docs/plans/2026-09-13-分镜重构方案.md`。两次 LM 调用：
//!
//! 1. **分场**：剧本里有【】标题（场景标题、第几幕）时**按标题直接切**，不问模型；
//!    没有标题才让模型按「同一地点、时间连续」切，每场原文必须**逐字**抄 ——
//!    代码核对抄的内容确实在剧本里、按顺序、没有漏掉成段的正文。
//! 2. **每场出镜头表**：一场一次调用（一次问一整集会把本机模型的思考撑爆，
//!    2026-09-13 实测过）。出来的镜头先过 [`crate::shot_table::validate`]，
//!    有问题就把问题原文反馈给模型重试一次，取问题更少的那份。
//!
//! # 这里不做的事
//!
//! - **不拼任何发给视频模型的提示词**。那是「合成提示词」那一步、按线路各自做。
//! - **不替模型补内容**。模型写错的地方由校验报出来，交给重试或者用户改。
//!
//! LM 调用通过参数注入，测试里换成假的 —— 真跑模型的验证靠看表，不靠单元测试。

use crate::cinema::{DialogueLine, StoryboardShot};
use crate::error::{Error, Result};
use crate::shot_table::{self, RefSlot, RefUsage, SceneItem, ShotIssue, ShotSegment, UnlinkedAsset};
use serde::{Deserialize, Serialize};
use std::future::Future;

/// 剧本里的一场。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneChunk {
    pub name: String,
    pub text: String,
}

/// 资产库里一个可用的资产，给提示词和名字换 id 用。
#[derive(Debug, Clone, PartialEq)]
pub struct AssetRef {
    pub id: String,
    pub kind: crate::assets::AssetKind,
    pub name: String,
    pub has_image: bool,
    /// 资产库里写的外貌 / 陈设描述。进提示词的资产清单 ——
    /// 只给名字的话，模型会自己编长相（2026-09-13 实跑：把披肩直发写成「长发盘起」、
    /// 把剧本明写的男装官服写成「齐胸襦裙」），编出来的写进参考图的「控制」里，和图打架。
    pub description: String,
    /// 这份描述和图用的是哪个形象。空 = 主形象。见 [`crate::looks`]
    pub look_id: String,
    /// 角色的所有形象（主形象在第一个）。单镜换形象时从这里取描述；场景 / 道具为空
    pub looks: Vec<LookRef>,
    /// 角色资产上写死的 H3 声音设定（中文）。**整集一份，跨场不变。**
    ///
    /// 给提示词的资产清单带上它，逐句的 `voice` 就只用写这一句的变化。
    /// 不带的话模型每场重编一次 —— 实测 E04 李白从「中年沙哑」变成「青年清亮」。
    pub h3_voice: String,
}

/// 一个形象发给模型的东西。
#[derive(Debug, Clone, PartialEq)]
pub struct LookRef {
    pub id: String,
    pub name: String,
    /// 完整描述（人物本身 + 形象描述）
    pub description: String,
    pub has_image: bool,
}

/// 生成结果。`issues` 是重试之后仍然存在的问题，界面上要显示出来。
#[derive(Debug, Clone, Serialize)]
pub struct ShotTableResult {
    pub scenes: usize,
    pub shots: usize,
    pub issues: Vec<ShotIssue>,
    /// 模型写了但资产库里没有的名字、被丢掉的参考图等。**必须说出来**。
    pub notes: Vec<String>,
}

/// 去掉所有空白后比较 —— 模型抄原文时经常改动空格和换行，那不算改了原文。
fn squash(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// 【序幕】【第一幕】【尾声】这类是结构标记，不是地点。
fn is_act_marker(title: &str) -> bool {
    let t = title.trim();
    matches!(t, "序幕" | "楔子" | "尾声" | "终幕" | "序章" | "尾章")
        || ((t.starts_with('第')) && (t.ends_with('幕') || t.ends_with('场') || t.ends_with('章') || t.ends_with('集')) && t.chars().count() <= 6)
}

/// 剧本自己写了【】标题时，按标题切场 —— **不问模型**。
///
/// # 为什么
///
/// 2026-09-13：同一份剧本、温度 0，模型一次切成 4 场（按幕），下一次切成 2 场
/// （把第一、二、三幕并成一场，而第三幕已经到了殿外）。用户看到的是「场少了一大半」。
/// 剧本里明明白白写着标题，场记本来就是照标题记场的，没有理由让模型每次重新猜。
///
/// # 规则
///
/// - 每个【】标记开一场；**挨着的几个标记算同一处**（【序幕】【现代 · 深夜办公室】是一场）。
/// - 场名取这一处的地点标题；只有【第二幕】这种结构标记时，沿用上一个地点并注明幕次。
/// - 第一个标记之前的内容（片名、类型、时长建议）是企划信息，不进任何一场。
/// - 切出来少于 2 场（没有标题，或全篇只有一个地点）返回 `None`，交给模型切。
pub fn split_by_headings(script: &str) -> Option<Vec<SceneChunk>> {
    // (标记起点, 标记终点, 标题)
    let mut marks: Vec<(usize, usize, String)> = Vec::new();
    let mut rest = 0;
    while let Some(open) = script[rest..].find('【') {
        let open = rest + open;
        let Some(close) = script[open..].find('】') else { break };
        let close = open + close + '】'.len_utf8();
        let title = script[open + '【'.len_utf8()..close - '】'.len_utf8()].trim().to_string();
        marks.push((open, close, title));
        rest = close;
    }
    // 挨着的标记并成一组：(这组起点, 这组标题们, 正文起点)
    let mut groups: Vec<(usize, Vec<String>, usize)> = Vec::new();
    for (open, close, title) in marks {
        match groups.last_mut() {
            Some((_, titles, body_start)) if script[*body_start..open].trim().is_empty() => {
                titles.push(title);
                *body_start = close;
            }
            _ => groups.push((open, vec![title], close)),
        }
    }
    let mut scenes = Vec::new();
    let mut place = String::new();
    for (i, (start, titles, body_start)) in groups.iter().enumerate() {
        let end = groups.get(i + 1).map_or(script.len(), |g| g.0);
        if script[*body_start..end].trim().is_empty() {
            continue;
        }
        let act = titles.iter().find(|t| is_act_marker(t));
        let name = match titles.iter().rev().find(|t| !is_act_marker(t)) {
            Some(p) => {
                place = p.clone();
                p.clone()
            }
            None if !place.is_empty() => format!("{place}（{}）", act.map_or("续", |a| a.as_str())),
            None => act.cloned().unwrap_or_default(),
        };
        scenes.push(SceneChunk { name, text: script[*start..end].trim().to_string() });
    }
    (scenes.len() >= 2).then_some(scenes)
}

/// 解析分场结果，并核对「逐字、按顺序、没漏正文」。
///
/// 核对不过返回 `Err`，错误信息里写清是哪一场、哪一句对不上 —— 调用方拿它重试。
pub fn parse_scenes(raw: &str, script: &str) -> std::result::Result<Vec<SceneChunk>, String> {
    #[derive(Deserialize)]
    struct Wire {
        scenes: Vec<SceneChunk>,
    }
    let json = crate::prompts::extract_json(raw).ok_or("分场结果里没有 JSON")?;
    let wire: Wire = serde_json::from_str(json).map_err(|e| format!("分场 JSON 解析失败：{e}"))?;
    let scenes: Vec<SceneChunk> = wire
        .scenes
        .into_iter()
        .filter(|s| !s.text.trim().is_empty())
        .collect();
    if scenes.is_empty() {
        return Err("一场都没有切出来".into());
    }
    let whole = squash(script);
    let mut cursor = 0usize;
    let mut covered = 0usize;
    for (i, s) in scenes.iter().enumerate() {
        let piece = squash(&s.text);
        match whole[cursor..].find(&piece) {
            Some(at) => {
                cursor += at + piece.len();
                covered += piece.len();
            }
            None => {
                let head: String = s.text.trim().chars().take(20).collect();
                return Err(format!("第 {} 场「{}」的原文在剧本里找不到（或顺序不对），开头是「{head}」—— 必须逐字抄原文", i + 1, s.name));
            }
        }
    }
    // 企划信息（类型、时长建议）允许不抄，但正文不能漏一大截。
    if covered * 10 < whole.len() * 7 {
        return Err(format!("分出来的场只覆盖了剧本的 {}%，漏了正文", covered * 100 / whole.len().max(1)));
    }
    Ok(scenes)
}

/// 模型返回的一镜。字段名和提示词里的例子一一对应。
#[derive(Debug, Deserialize)]
struct WireShot {
    #[serde(default)]
    duty: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    duration: f32,
    #[serde(default)]
    scale: String,
    #[serde(default)]
    angle: String,
    #[serde(default, rename = "move")]
    movement: String,
    #[serde(default)]
    camera_note: String,
    #[serde(default)]
    lighting: String,
    #[serde(default)]
    scene_items: Vec<SceneItem>,
    #[serde(default)]
    segments: Vec<ShotSegment>,
    #[serde(default)]
    dialogue: Vec<WireLine>,
    #[serde(default)]
    ambient_sound: String,
    #[serde(default)]
    sound_effect: String,
    #[serde(default)]
    assets: WireAssets,
    #[serde(default)]
    ref_slots: Vec<WireSlot>,
}

#[derive(Debug, Deserialize)]
struct WireLine {
    #[serde(default)]
    speaker: String,
    #[serde(default)]
    line: String,
    /// 这一句怎么说：情绪、音量、语速。中文。
    ///
    /// **这是 H3 唯一能拿到的表演指示。** 不给的话它只知道说话人的性别和年龄，
    /// 念出来是播音腔 —— 2026-09-18 用户在成片里听出来的就是这个。
    #[serde(default)]
    emotion: String,
    /// 音色 / 口音。中文。
    #[serde(default)]
    voice: String,
    #[serde(default)]
    off_screen: bool,
}

#[derive(Debug, Default, Deserialize)]
struct WireAssets {
    #[serde(default)]
    characters: Vec<String>,
    /// 这一镜换了形象的人物：人物名 → 形象名（资产清单里「形象：」那几个）。没换的不写。
    /// 剧本写了换装、受伤、外壳破损时用（2026-10-05：《回收师》E03 嫁衣改的礼服、E17 / E28 外壳破损）
    #[serde(default)]
    looks: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    scene: String,
    #[serde(default)]
    props: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct WireSlot {
    #[serde(default)]
    asset: String,
    #[serde(default)]
    usage: String,
    #[serde(default)]
    control: String,
    #[serde(default)]
    not_control: String,
}

/// 参考图用途。模型常写取值表外的词（2026-09-13 实跑写了 `prop`，严格解析直接把整场判失败）。
///
/// - 表内的照收；
/// - `prop` / `object` / `item` 这类指「这件东西长什么样」，就是道具的身份，收成 `identity`；
/// - 别的认不出就返回 `None`，调用方丢掉这张参考图并说出来 —— **不猜**。
fn ref_usage(raw: &str) -> Option<RefUsage> {
    let t = raw.trim().to_ascii_lowercase();
    if let Ok(u) = serde_json::from_value::<RefUsage>(serde_json::Value::String(t.clone())) {
        return Some(u);
    }
    match t.as_str() {
        "prop" | "props" | "object" | "item" | "appearance" => Some(RefUsage::Identity),
        _ => None,
    }
}

/// 去掉模型在字符串开头多写的 `>`。
///
/// 2026-09-13 实跑：qwen3.6 时不时写出 `"asset":">手机"`、`"sound":">键盘声"`
/// （加不加 JSON 约束都有）。资产名带着它就对不上资产库，参考图被丢。
/// 合法 JSON 里未转义的 `"` 只出现在字符串边界上，所以「`:` `,` `[` 后面紧跟 `">`」
/// 一定是字符串开头多了一个 `>`，不会误伤正文。
fn strip_quote_gt(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    let mut chars = json.char_indices().peekable();
    let mut last_sig = ' ';
    let mut in_str = false;
    let mut escaped = false;
    while let Some((_, c)) = chars.next() {
        out.push(c);
        if in_str {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
                last_sig = '"';
            }
            continue;
        }
        if c == '"' {
            in_str = true;
            if matches!(last_sig, ':' | ',' | '[') {
                if let Some((_, '>')) = chars.peek() {
                    chars.next();
                }
            }
        } else if !c.is_whitespace() {
            last_sig = c;
        }
    }
    out
}

/// 名字末尾的括号注释去掉：「皇帝（苏菲饰）」→「皇帝」。没有注释返回 `None`。
fn strip_note(name: &str) -> Option<&str> {
    let t = name.trim();
    let open = if t.ends_with('）') { t.rfind('（') } else if t.ends_with(')') { t.rfind('(') } else { None }?;
    let head = t[..open].trim();
    (!head.is_empty()).then_some(head)
}

/// 台词开头的括号提示拆出来：`（咬牙）所以满意度是零。` → `("所以满意度是零。", "咬牙")`。
///
/// # 为什么必须拆
///
/// **不拆的话那几个字会被念出来。** 台词是逐字发给 H3 的（`<d>[Chinese] …</d>`），
/// 它不认识「这是舞台说明」—— 2026-09-18 查成片配音时顺出来的一处静默失败：
/// 任务成功、字幕正常、只有声音里多了两个字，看画面完全发现不了。
///
/// 拆出来的那句还能当表演指示用，正好是我们原来缺的东西。
///
/// # 只拆开头，不拆结尾，也不碰说话人
///
/// - 结尾的括号可能是台词的一部分（「他说了一句……（大概吧）」）；
/// - **说话人名字里的括号是正经内容** —— 「林婉清（盛唐）」是资产全名，
///   照着剥会把角色剥没（`strip_note` 那条路有资产表兜底，这里没有）。
///
/// 整句都是括号的（`（画外音，激昂）`）不拆：那是模型把舞台说明当成了台词，
/// 是错误，由 `source_coverage` 报出来，拆掉等于把错误藏起来。
fn split_leading_tone(line: &str) -> (String, String) {
    let t = line.trim();
    let (open, close) = if t.starts_with('（') {
        ('（', '）')
    } else if t.starts_with('(') {
        ('(', ')')
    } else {
        return (t.to_string(), String::new());
    };
    let Some(at) = t.find(close) else { return (t.to_string(), String::new()) };
    let note = t[open.len_utf8()..at].trim();
    let rest = t[at + close.len_utf8()..].trim_start_matches(['：', ':', ' ', '\u{3000}']).trim();
    // 括号后面没词了 → 整句都是舞台说明，不动它
    if rest.is_empty() || note.is_empty() {
        return (t.to_string(), String::new());
    }
    (rest.to_string(), note.to_string())
}

/// 按名字找资产。先精确找；找不到时去掉末尾的括号注释再精确找一次。
///
/// 2026-09-13 实跑：模型把「皇帝」写成「皇帝（苏菲饰）」（把演员表里的人塞进了括号），
/// 人物和参考图全被丢掉。资产本身带括号的（「林婉清（盛唐）」）先精确匹配，不受影响。
/// 返回的布尔表示是不是去掉注释才找到的 —— 调用方要把这件事写进备注。
fn lookup<'a>(assets: &'a [AssetRef], kind: Option<crate::assets::AssetKind>, name: &str) -> Option<(&'a AssetRef, bool)> {
    let find = |n: &str| assets.iter().find(|a| kind.is_none_or(|k| a.kind == k) && a.name == n);
    let name = name.trim();
    find(name)
        .map(|a| (a, false))
        .or_else(|| strip_note(name).and_then(find).map(|a| (a, true)))
        .or_else(|| {
            // 第三级：人物名字前面多了头衔（「主管老秦」→ 资产「老秦」）。只认唯一一个。
            // **只管人物** —— 场景「金銮殿门口」以「门口」结尾，按这条就会被错归到「门口」
            let hits: Vec<&AssetRef> = assets
                .iter()
                .filter(|a| a.kind == crate::assets::AssetKind::Character && kind.is_none_or(|k| a.kind == k) && titled(&a.name, name))
                .collect();
            (hits.len() == 1).then(|| (hits[0], true))
        })
}

/// `given` 是不是在资产名 `asset` 前面加了个头衔：「主管老秦」之于「老秦」。
///
/// 2026-09-26《回收师》E01：剧本写「主管老秦翘着腿」，模型就把说话人和出场人物都写成「主管老秦」，
/// 资产库里叫「老秦」—— 整场主管办公室的老秦都没挂上参考图，四次生成里三次都这样。
/// 限制：头衔最多 4 个字、资产名至少 2 个字（「邓」这种单字太容易误配）。
fn titled(asset: &str, given: &str) -> bool {
    let (asset, given) = (asset.trim(), given.trim());
    asset.chars().count() >= 2
        && given != asset
        && given.strip_suffix(asset).is_some_and(|title| {
            let n = title.chars().count();
            (1..=4).contains(&n) && !title.contains(['（', '(', '·', ' '])
        })
}

/// 两段字的编辑距离（按字）。台词很短，直接 DP。
fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(ca != cb)).min(row[j] + 1).min(row[j + 1] + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

/// 已经生成好的分镜里，人物名字前面带了头衔、没挂上资产的（「主管老秦」），认回资产并补上身份参考。
///
/// 新生成的分镜在解析时就会认对（`lookup` 的第三级）；这个给**已经存下来的**旧数据用。返回改了几镜。
pub fn relink_titled(shots: &mut [StoryboardShot], assets: &[AssetRef]) -> usize {
    use crate::assets::AssetKind;
    use crate::shot_table::{RefSlot, RefUsage};
    let mut fixed = 0;
    for shot in shots {
        let mut changed = false;
        let mut keep = Vec::new();
        for u in std::mem::take(&mut shot.unlinked_assets) {
            let hit = (u.kind == "character").then(|| lookup(assets, Some(AssetKind::Character), &u.name)).flatten();
            let Some((a, _)) = hit else {
                keep.push(u);
                continue;
            };
            if !shot.character_ids.contains(&a.id) {
                shot.character_ids.push(a.id.clone());
            }
            if a.has_image && !shot.ref_slots.iter().any(|r| r.asset_id == a.id) {
                let brief: String = a.description.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(40).collect();
                shot.ref_slots.push(RefSlot {
                    order: shot.ref_slots.len() as u8 + 1,
                    asset_id: a.id.clone(),
                    usage: RefUsage::Identity,
                    control: brief,
                    not_control: "姿势、构图、表情".into(),
                });
            }
            changed = true;
        }
        shot.unlinked_assets = keep;
        for l in &mut shot.dialogue_lines {
            if l.off_screen {
                continue;
            }
            if let Some((a, true)) = lookup(assets, Some(AssetKind::Character), &l.speaker) {
                l.speaker = a.name.clone();
                if !shot.character_ids.contains(&a.id) {
                    shot.character_ids.push(a.id.clone());
                }
                changed = true;
            }
        }
        if changed {
            if shot.character_id.is_empty() {
                shot.character_id = shot.character_ids.first().cloned().unwrap_or_default();
            }
            fixed += 1;
        }
    }
    fixed
}

/// **台词差一两个字的，换回剧本原句。**
///
/// 模型抄台词时会顺手「润色」：「拆完你**就**不用再伺候……」写成「拆完你不用再伺候……」，
/// 「你说**那是**我第三次……」写成「你说我第三次……」。校验会判「不是剧本原话」+「剧本这句没拍到」两个问题，
/// 带着问题重写也经常改不回来（《回收师》E01 四次生成都有）。这种一眼能对上的，代码直接换回去；
/// 这一镜原文（source）里同样那一段也一起换。差得多的不猜，留给校验报出来。
pub fn snap_lines_to_script(shots: &mut [StoryboardShot], scene_text: &str) -> usize {
    let script: Vec<(String, String)> = crate::script_parse::dialogue_of(scene_text);
    let mut fixed = 0;
    for shot in shots {
        for i in 0..shot.dialogue_lines.len() {
            let line = shot.dialogue_lines[i].line.trim().to_string();
            let key = spoken(&line);
            if key.chars().count() < 4 || script.iter().any(|(_, l)| spoken(l) == key) {
                continue;
            }
            let near: Vec<&(String, String)> = script
                .iter()
                .filter(|(_, l)| {
                    let k = spoken(l);
                    let d = edit_distance(&k, &key);
                    d > 0 && d <= (k.chars().count() / 8).max(2)
                })
                .collect();
            if near.len() != 1 {
                continue;
            }
            let original = near[0].1.trim().to_string();
            if shot.source.contains(&line) {
                shot.source = shot.source.replacen(&line, &original, 1);
            }
            shot.dialogue_lines[i].line = original;
            fixed += 1;
        }
    }
    fixed
}

/// 数字字段。其余字段都是文字（或文字数组、对象）。
const NUMBER_KEYS: [&str; 3] = ["duration", "start", "end"];

/// 把模型写错类型的值规整过来，**只改类型，不改内容**。
///
/// 实跑里出现过：文字字段写成整数（`2400018756`）、时长写成 `"8"`、字段写成 `null`。
/// 严格反序列化遇到任何一个都把整场判失败 —— 一个无关紧要的字段写错类型，
/// 不值得扔掉一整场镜头、再让用户等一次重试。
///
/// - 数字字段：`"8"`、`"8秒"` 取开头的数；取不出就删掉这个键（按没写处理，由校验报出来）。
/// - 其余字段：数字、布尔转成文字；`null` 删掉按没写处理。
/// - `off_screen` 是布尔：`"true"` / `"是"` 算真。
fn normalize_types(v: &mut serde_json::Value) {
    use serde_json::Value;
    match v {
        Value::Array(items) => items.iter_mut().for_each(|item| match item {
            Value::Number(n) => *item = Value::String(n.to_string()),
            Value::Bool(b) => *item = Value::String(b.to_string()),
            _ => normalize_types(item),
        }),
        Value::Object(map) => {
            map.retain(|_, val| !val.is_null());
            for (key, val) in map.iter_mut() {
                if NUMBER_KEYS.contains(&key.as_str()) {
                    if let Value::String(text) = val {
                        let head: String = text.trim().chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
                        *val = head.parse::<f64>().ok().and_then(serde_json::Number::from_f64).map_or(Value::Null, Value::Number);
                    }
                } else if key == "off_screen" {
                    if let Value::String(text) = val {
                        *val = Value::Bool(matches!(text.trim(), "true" | "True" | "是" | "画外"));
                    }
                } else {
                    match val {
                        Value::Number(n) => *val = Value::String(n.to_string()),
                        Value::Bool(b) => *val = Value::String(b.to_string()),
                        _ => normalize_types(val),
                    }
                }
            }
            map.retain(|_, val| !val.is_null());
        }
        _ => {}
    }
}

/// 词表 id 必须在词表里；认不出就留空，交给校验和用户 —— **不猜一个默认值**。
fn vocab_or_empty(group: &str, id: &str, notes: &mut Vec<String>, shot_num: u32) -> String {
    let id = id.trim();
    if id.is_empty() {
        return String::new();
    }
    let known = crate::shot_language::group(group).is_some_and(|g| g.items.iter().any(|t| t.id == id));
    if known {
        id.to_string()
    } else {
        notes.push(format!("第 {shot_num} 镜的{}「{id}」不在词表里，已留空", crate::shot_language::group(group).map(|g| g.name.as_str()).unwrap_or(group)));
        String::new()
    }
}

/// 把一场的镜头表 JSON 变成镜头。资产名在这里换成 id；换不上的记进 `notes`。
///
/// `first_num` 是这一场第一镜的镜号（全集连续编号）。
pub fn parse_shots(raw: &str, assets: &[AssetRef], first_num: u32, notes: &mut Vec<String>) -> std::result::Result<Vec<StoryboardShot>, String> {
    use crate::assets::AssetKind;
    #[derive(Deserialize)]
    struct Wire {
        shots: Vec<WireShot>,
    }
    let json = crate::prompts::extract_json(raw).ok_or("镜头表结果里没有 JSON")?;
    let mut value: serde_json::Value = serde_json::from_str(&strip_quote_gt(json)).map_err(|e| format!("镜头表 JSON 解析失败：{e}"))?;
    // 场级的固定布景：整场一份，下面挂到这一场的每一镜上。**宽松读**，写成字符串或数组都认，
    // 缺了不算解析失败（老模板、用户改过的模板没有这一项）—— 缺没缺由核对报出来
    let stage = value.get("stage").map(crate::shot_table::SceneStage::from_value).unwrap_or_default();
    // **只写了布景就收工**（2026-09-28~29 七次）：整份输出只有 `{"stage":{…}}`。
    // serde 的原话是英文的 `missing field shots`，反馈给模型它看不懂自己错在哪 —— 说清楚
    if value.get("stage").is_some() && value.get("shots").is_none() {
        return Err("只写了这一场的 stage（固定布景）就结束了，没有写 shots —— stage 后面必须接着写 shots 数组，把这一场的每一镜都写出来".into());
    }
    if let Some(map) = value.as_object_mut() {
        map.remove("stage");
    }
    normalize_types(&mut value);
    let wire: Wire = serde_json::from_value(value).map_err(|e| format!("镜头表 JSON 结构不对：{e}"))?;
    if wire.shots.is_empty() {
        return Err("这一场一个镜头都没有".into());
    }
    let find = |kind: AssetKind, name: &str, num: u32, notes: &mut Vec<String>| {
        lookup(assets, Some(kind), name).map(|(a, stripped)| {
            if stripped {
                notes.push(format!("第 {num} 镜把「{}」写成了「{}」，按资产「{}」算", a.name, name.trim(), a.name));
            }
            a
        })
    };
    let mut out = Vec::new();
    for (i, w) in wire.shots.into_iter().enumerate() {
        let num = first_num + i as u32;
        let mut character_ids = Vec::new();
        let mut unlinked: Vec<UnlinkedAsset> = Vec::new();
        for name in &w.assets.characters {
            match find(AssetKind::Character, name, num, notes) {
                Some(a) if !character_ids.contains(&a.id) => character_ids.push(a.id.clone()),
                Some(_) => {}
                None => {
                    notes.push(format!("第 {num} 镜的人物「{}」在资产库里没有，提示词里按文字描述", name.trim()));
                    unlinked.push(UnlinkedAsset { kind: "character".into(), name: name.trim().to_string() });
                }
            }
        }
        // 形象：名字对上这个人物的某个形象才换；对不上说出来、用默认形象
        let mut look_overrides = std::collections::BTreeMap::new();
        for (who, look_name) in &w.assets.looks {
            let look_name = look_name.trim();
            let Some(a) = assets.iter().find(|a| a.kind == AssetKind::Character && character_ids.contains(&a.id) && a.name == who.trim()) else {
                notes.push(format!("第 {num} 镜给「{}」写了形象「{look_name}」，但这一镜的人物里没有他，没换", who.trim()));
                continue;
            };
            match a.looks.iter().find(|l| l.name.trim() == look_name || (l.id == crate::assets::MAIN_LOOK && look_name == "主形象")) {
                Some(l) => {
                    look_overrides.insert(a.id.clone(), l.id.clone());
                }
                None => notes.push(format!("第 {num} 镜给「{}」写的形象「{look_name}」资产里没有，用默认形象", a.name)),
            }
        }
        let mut scene_id = match w.assets.scene.trim() {
            "" => None,
            name => match find(AssetKind::Scene, name, num, notes) {
                Some(a) => Some(a.id.clone()),
                None => {
                    // 「金銮殿门口」「金銮殿（局部）」是金銮殿的一部分：资产库里**只有一个**场景名被它包含时，就归到那个场景。
                    // 两个以上都沾边就不猜。
                    let parts: Vec<&AssetRef> = assets.iter().filter(|a| a.kind == AssetKind::Scene && !a.name.is_empty() && name.contains(a.name.as_str())).collect();
                    match parts.as_slice() {
                        [a] => {
                            notes.push(format!("第 {num} 镜的场景写成「{name}」，按资产「{}」算", a.name));
                            Some(a.id.clone())
                        }
                        _ => {
                            notes.push(format!("第 {num} 镜的场景「{name}」在资产库里没有，提示词里按文字描述"));
                            unlinked.push(UnlinkedAsset { kind: "scene".into(), name: name.to_string() });
                            None
                        }
                    }
                }
            },
        };
        let mut prop_ids = Vec::new();
        for name in &w.assets.props {
            match find(AssetKind::Prop, name, num, notes) {
                Some(a) if !prop_ids.contains(&a.id) => prop_ids.push(a.id.clone()),
                Some(_) => {}
                None => {
                    notes.push(format!("第 {num} 镜的道具「{}」在资产库里没有，提示词里按文字描述", name.trim()));
                    unlinked.push(UnlinkedAsset { kind: "prop".into(), name: name.trim().to_string() });
                }
            }
        }
        // 参考图：按模型给的顺序编号；资产换不上 id、或者资产没有图的，丢掉并说出来。
        let mut ref_slots: Vec<RefSlot> = Vec::new();
        for slot in w.ref_slots {
            let Some(usage) = ref_usage(&slot.usage) else {
                notes.push(format!("第 {num} 镜的参考图「{}」用途写成了「{}」，不在取值表里，这张参考图没挂", slot.asset.trim(), slot.usage.trim()));
                continue;
            };
            let hit = lookup(assets, None, &slot.asset).map(|(a, _)| a);
            // 挂了参考图就是这一镜看得见它：没写进出场资产的补上（模型常只写在参考图里）
            if let Some(a) = hit {
                match a.kind {
                    AssetKind::Character if !character_ids.contains(&a.id) => character_ids.push(a.id.clone()),
                    AssetKind::Scene if scene_id.is_none() => scene_id = Some(a.id.clone()),
                    AssetKind::Prop if !prop_ids.contains(&a.id) => prop_ids.push(a.id.clone()),
                    _ => {}
                }
            }
            // **没图的也记下来**：合成时没图就按文字描述，补上图（生成或上传）再合成就会挂上。
            // 以前没图直接丢，之后补了图也回不来 —— 只能重新生成整张镜头表。
            match hit {
                Some(a) => {
                    if !a.has_image {
                        notes.push(format!("第 {num} 镜要挂「{}」的参考图，但它还没有图 —— 先按文字描述；补上图后重新合成提示词就会挂上", a.name));
                    }
                    ref_slots.push(RefSlot {
                        order: ref_slots.len() as u8 + 1,
                        asset_id: a.id.clone(),
                        usage,
                        control: slot.control,
                        not_control: slot.not_control,
                    });
                }
                None => notes.push(format!("第 {num} 镜的参考图「{}」在资产库里没有", slot.asset.trim())),
            }
        }
        let movement = w.movement;
        // 说话人写简称（「林婉清」）而出场人物是「林婉清（盛唐）」：本镜里只有一个对得上就换成全名。
        // 这是名字写法不同，不是写错了人 —— 不值得为它让模型重写一整场。
        let names_in_shot: Vec<&str> = character_ids.iter().filter_map(|id| assets.iter().find(|a| &a.id == id)).map(|a| a.name.as_str()).collect();
        let full_name = |speaker: &str| -> String {
            if names_in_shot.contains(&speaker) {
                return speaker.to_string();
            }
            if let Some(head) = strip_note(speaker).filter(|h| names_in_shot.contains(h)) {
                return head.to_string();
            }
            let hits: Vec<&&str> = names_in_shot
                .iter()
                .filter(|n| n.strip_prefix(speaker).is_some_and(|rest| rest.starts_with('（') || rest.starts_with('(')))
                .collect();
            match hits.as_slice() {
                [one] => one.to_string(),
                _ => speaker.to_string(),
            }
        };
        let dialogue_lines: Vec<DialogueLine> = w
            .dialogue
            .into_iter()
            .filter(|l| !l.line.trim().is_empty())
            .map(|l| {
                let (line, from_paren) = split_leading_tone(l.line.trim());
                DialogueLine {
                    speaker: full_name(l.speaker.trim()),
                    line,
                    // 模型没单独填就用台词开头那个括号（见 `split_leading_tone`）
                    emotion: if l.emotion.trim().is_empty() { from_paren } else { l.emotion.trim().into() },
                    voice: l.voice.trim().into(),
                    // 逐句覆盖不由模型决定 —— 它是用户在界面上标的例外
                    voice_override: String::new(),
                    off_screen: l.off_screen,
                }
            })
            .collect();
        let shot = StoryboardShot {
            id: format!("shot_{}", uuid::Uuid::new_v4().simple()),
            shot_num: num,
            scale: vocab_or_empty("shotSize", &w.scale, notes, num),
            camera_angle: vocab_or_empty("angle", &w.angle, notes, num),
            camera_movement: vocab_or_empty("move", &movement, notes, num),
            camera_note: w.camera_note.trim().into(),
            // 旧的整句台词字段：字幕和导出还读它。由结构化台词派生，不是另一份真相。
            dialogue: dialogue_lines.iter().map(|l| format!("{}：{}", l.speaker, l.line)).collect::<Vec<_>>().join("\n"),
            character_id: character_ids.first().cloned().unwrap_or_default(),
            character_ids,
            scene_id,
            prop_ids,
            // 旧的画面字段：列表摘要还读它。取第一段画面，不是另一份真相。
            prompt: w.segments.first().map(|s| s.visual.trim().to_string()).unwrap_or_default(),
            duration: w.duration,
            duty: w.duty.trim().into(),
            source: w.source.trim().into(),
            lighting: w.lighting.trim().into(),
            scene_items: w.scene_items,
            stage: stage.clone(),
            segments: w.segments,
            ref_slots,
            unlinked_assets: unlinked,
            look_overrides,
            dialogue_lines,
            ambient_sound: w.ambient_sound.trim().into(),
            sound_effect: w.sound_effect.trim().into(),
            status: "pending".into(),
            ..blank_shot()
        };
        out.push(shot);
    }
    Ok(out)
}

/// 一个所有字段都是空的镜头，给结构体更新语法用。
fn blank_shot() -> StoryboardShot {
    serde_json::from_value(serde_json::json!({
        "id": "", "shot_num": 0, "scale": "", "camera_movement": "", "dialogue": "",
        "prompt": "", "duration": 0.0, "status": "pending"
    }))
    .expect("空镜头一定能反序列化")
}

/// 短到该考虑合并的镜头长度。
///
/// **不是硬上限**，是「低于这个又没有台词，多半该并进隔壁」的提醒线。
/// `MIN_SHOT_SECS = 4` 是 H3 的原生下限，不是创作判据 —— 两者不是一回事。
pub const MERGE_HINT_SECS: f32 = 7.0;

/// 「承接上一场」这类过场镜的典型措辞。出现在 duty 里基本说明这一镜不该存在。
const FILLER_DUTY: [&str; 6] = ["承接", "落回", "余韵", "过渡", "铺垫上一场", "接上一场"];

/// **连续几镜都没有台词、而且都很短 —— 多半本来是一镜。**
///
/// # 为什么要有这一条
///
/// 2026-09-19 用户指出：「经常完全可以一镜出来的内容被分成了两到三镜，
/// 其中一到两镜都是没有对话内容的，这完全可以放到一镜里，而且更饱满，
/// 拆分成两到三镜反而觉得空洞，而且还会造成一致性问题。」
///
/// E04 的数据把模式摆得很清楚 —— 十二镜只有五镜有台词，而且是稳定的
/// 「承接过场 + 5 秒台词」对：
///
/// ```text
/// 第 7镜 6s 台词0  承接上一场杜甫发问的余韵，落回李白这边
/// 第 8镜 5s 台词1  抛出李白的回话
/// ```
///
/// 光靠提示词自觉不行 —— 判据得看得见。这一条**只报不拦**：
/// 它是创作判断，不是错误，用户可能真的要一个空镜。
///
/// # 为什么不看 duty 就够
///
/// duty 里写「承接」是最明显的那种，但模型换个说法就绕过去了。
/// **「连续两镜没台词又都短」是结构事实**，改措辞绕不开。
/// 两者都查：duty 命中的单独说，因为那条更有把握。
pub fn shot_density(shots: &[StoryboardShot]) -> Vec<ShotIssue> {
    let mut out = Vec::new();
    let silent = |s: &StoryboardShot| s.dialogue_lines.iter().all(|l| l.line.trim().is_empty());
    for s in shots {
        if FILLER_DUTY.iter().any(|w| s.duty.contains(w)) && silent(s) {
            out.push(ShotIssue {
                shot_num: s.shot_num,
                field: "duty",
                message: format!(
                    "第 {} 镜没有台词，职责写的是「{}」—— 这种承接/过渡写进下一镜的第一段 segment 就行，不用单独占一镜",
                    s.shot_num,
                    s.duty.trim()
                ),
            });
        }
    }
    for w in shots.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if silent(a) && silent(b) && a.duration < MERGE_HINT_SECS && b.duration < MERGE_HINT_SECS {
            out.push(ShotIssue {
                shot_num: a.shot_num,
                field: "duration",
                message: format!(
                    "第 {} 和第 {} 镜都没有台词，各只有 {:.0} 秒和 {:.0} 秒 —— 多半本来是一镜，并起来画面更饱满，也少一次身份漂移",
                    a.shot_num, b.shot_num, a.duration, b.duration
                ),
            });
        }
    }
    out
}

/// **跳切**：同一场里相邻两镜拍同一批人，景别和机位都一样。
///
/// 判据用 ai-film 的说法（2026-09-29 统一成一套之后）：「每次切镜带来观看距离、视点、反应、省略或节奏变化」。
/// 景别机位都一样、拍的同一批人，接在一起只是画面跳了一下。**不再写「景别跳一级」** —— ai-film 原文说没有固定景别阶梯，
/// 这条反馈要是还按老规则写，重试时就又把老规则喂回给模型。
///
/// 一镜出的窗口不算 —— 那是同一个长镜头被切成几段生成，本来就该接得上。**只报不拦**，和 [`shot_density`] 一样。
pub fn jump_cuts(shots: &[StoryboardShot]) -> Vec<ShotIssue> {
    let people = |s: &StoryboardShot| {
        let mut v = s.all_character_ids();
        v.sort();
        v.dedup();
        v
    };
    let mut out = Vec::new();
    for w in shots.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        let same_scene = a.scene_no == b.scene_no && (a.scene_no != 0 || (!a.scene_name.is_empty() && a.scene_name == b.scene_name));
        if !same_scene || a.one_take || b.one_take || a.scale.is_empty() || a.scale != b.scale || a.camera_angle != b.camera_angle {
            continue;
        }
        let (pa, pb) = (people(a), people(b));
        if pa.is_empty() || pa != pb {
            continue;
        }
        let d = |g: &str, v: &str| crate::shot_language::display(g, v);
        out.push(ShotIssue {
            shot_num: b.shot_num,
            field: "scale",
            message: format!(
                "第 {} 和第 {} 镜同一场、拍的是同一批人，景别和机位都是「{} · {}」—— 每次切镜要带来观看距离、视点、反应、省略或节奏的变化，这两镜接在一起只是画面跳了一下：并成一镜，或者让后一镜的观看真的变了（换距离、换视点、换拍反应）",
                a.shot_num,
                b.shot_num,
                d("shotSize", &a.scale),
                d("angle", &a.camera_angle)
            ),
        });
    }
    out
}

/// 这一场没写固定布景（`stage`），或者布景里写的是**画面位置**。
///
/// 后一条是实跑查出来的（2026-09-28，EP01 拆机间两场）：模型把固定物写成「拆机台：画面左下方」，
/// 第 3 场照抄了第 2 场的布景，两人的画面左右却又对调了 —— 布景里存的是某个机位下的投影，不是场里的位置，
/// 换了机位就没法推。布景只写场里的位置，画面左右留给每一镜的站位。
pub fn missing_stage(shots: &[StoryboardShot]) -> Vec<ShotIssue> {
    let Some(first) = shots.first() else { return Vec::new() };
    if shots.iter().all(|s| s.stage.is_empty()) {
        return vec![ShotIssue {
            shot_num: first.shot_num,
            field: "stage",
            message: "这一场没有写 stage（固定布景）—— 先写方向基准、固定物、光源、主关系和机位侧、开场站位、时段，每一镜的站位和光都从它推".into(),
        }];
    }
    let st = &shots.iter().find(|s| !s.stage.is_empty()).unwrap_or(first).stage;
    let screen: Vec<&str> = st.anchors.iter().chain(&st.lights).chain(&st.people).map(String::as_str).filter(|x| x.contains("画面")).collect();
    match screen.first() {
        Some(x) => vec![ShotIssue {
            shot_num: first.shot_num,
            field: "stage",
            message: format!(
                "stage 里写了画面位置（「{}」）—— 固定布景只写场里的位置（按方向基准的前后左右、靠哪面墙），画面左右是机位决定的，留给每一镜的 blocking",
                x.chars().take(30).collect::<String>()
            ),
        }],
        None => Vec::new(),
    }
}

/// 只留可发声的字（去掉空白和标点），比较台词用。
fn spoken(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).collect()
}

/// 这一场剧本里的每一句台词，都要出现在这一场某一镜的台词栏里。
///
/// # 为什么要有这一条
///
/// 2026-09-13 实跑：两个 qwen 模型都没往台词栏写任何东西 —— 一个把台词塞进了「声音」，
/// 一个只写「大臣厉声高喊」「低声说出台词」，原文一句没有。校验只检查「写了的台词」，
/// 台词栏整个是空的它就无从报起，于是显示 0 个问题。
/// 视频模型要靠台词栏生成同步配音和口型，这里漏了，片子就是哑的。
///
/// # 2026-09-19 重写：原来的判据是**双向错**的
///
/// 原来只扫 `“…”` / `「…」`。而实际剧本长这样：
///
/// ```text
/// 橘猫：各位领导，下面我做年度述职。          ← 真台词，一个引号都没有
/// ……快递单上写着「昨天签收」。                 ← 唯一的「」，是屏幕文字
/// ```
///
/// 于是这道防线**一句真台词都查不到**（形同虚设），
/// 反而把**屏幕文字报成「漏掉的台词」**喂给模型重试。
/// Codex Astra 的只读审查抓到的，实测四集都中。
///
/// 现在走 [`crate::script_parse`]，并且**按说话人和出现次数**比对：
///
/// - 只比拼接字符串的子串，同一句说两遍只写一遍会被放过；
/// - 不比说话人，模型把甲的台词安到乙头上也查不出来（而那会让口型配错人）。
pub fn dialogue_coverage(scene_text: &str, shots: &[StoryboardShot]) -> Vec<ShotIssue> {
    let first = shots.first().map(|s| s.shot_num).unwrap_or(0);
    // 镜头表里写了的：(说话人, 可发声的字)。**保留重复**，用来数次数
    // 第三项是这句写在哪一镜：说话人写错要报在**那一镜**上，不是这一场的第一镜 —— 那一镜的「修复」按钮才改得到它
    let mut written: Vec<(String, String, u32)> = shots
        .iter()
        .flat_map(|s| s.dialogue_lines.iter().map(move |l| (s.shot_num, l)))
        .filter(|(_, l)| !l.line.trim().is_empty())
        .map(|(n, l)| (spoken(&l.speaker), spoken(&l.line), n))
        .collect();
    let mut out = Vec::new();
    for (speaker, line) in crate::script_parse::dialogue_of(scene_text) {
        let key = spoken(&line);
        if key.chars().count() < 2 {
            continue;
        }
        let who = spoken(&speaker);
        // **说话人为空 = 叙述句里夹的台词**（`她抬头：“我还能撑。”`），
        // 那种写法里的名字猜不准，只比台词本身。见 `script_parse::split_quotes`。
        if who.is_empty() {
            if let Some(i) = written.iter().position(|(_, t, _)| t == &key) {
                written.remove(i);
            } else {
                out.push(ShotIssue {
                    shot_num: first,
                    field: "dialogue",
                    message: format!("剧本台词「{}」没有写进这一场任何一镜的台词栏（要逐字写进 dialogue，不能写进声音或画面）", line.trim()),
                });
            }
            continue;
        }
        // 先找说话人也对得上的那一条；找不到再放宽到只对台词
        let exact = written.iter().position(|(w, t, _)| t == &key && (w == &who || w.contains(&who) || who.contains(w)));
        if let Some(i) = exact {
            written.remove(i);
            continue;
        }
        if let Some(i) = written.iter().position(|(_, t, _)| t == &key) {
            let (wrong, _, at) = written.remove(i);
            out.push(ShotIssue {
                shot_num: at,
                field: "dialogue",
                message: format!("台词「{}」写成了「{wrong}」说的，剧本里是「{speaker}」—— 说话人错了，出片会配错人的口型", line.trim()),
            });
            continue;
        }
        out.push(ShotIssue {
            shot_num: first,
            field: "dialogue",
            message: format!("剧本台词「{}」（{speaker}）没有写进这一场任何一镜的台词栏（要逐字写进 dialogue，不能写进声音或画面）", line.trim()),
        });
    }
    out
}

/// 每镜的「原文」必须逐字出自这一场，而且这一场的每一句正文都要有一镜拍到。
///
/// # 为什么要有这一条
///
/// 2026-09-13 实跑同时出了三种错，台词校验一种也抓不住：
/// - **漏拍**：第三幕开头「殿外风起。长安街市灯火连绵。」没有任何一镜；
/// - **编造**：一镜的原文写「（接上镜）镜头拉远，展现林婉清孤身一人跪在空旷大殿中央」，剧本里没有；
/// - **串了提示词**：一镜的原文里抄进了「【这一场】…【上一场的结尾】…」。
///
/// 比较时只看可发声的字（去掉标点、空白、引号样式），模型把 “” 抄成 '' 不算改原文。
pub fn source_coverage(scene_text: &str, shots: &[StoryboardShot]) -> Vec<ShotIssue> {
    let body = strip_marks(scene_text);
    let whole = spoken(&body);
    let mut out = Vec::new();
    for shot in shots {
        if spoken(&shot.source).is_empty() {
            out.push(ShotIssue { shot_num: shot.shot_num, field: "source", message: "没写原文 —— source 要逐字引这一镜负责拍的剧本句子".into() });
        }
        // 逐句比：一镜引了两处不相邻的原句也算逐字（顺序由镜头自己决定），编出来的句子一定对不上
        if let Some(bad) = sentences(&shot.source).into_iter().find(|s| !whole.contains(&spoken(s))) {
            let head: String = bad.trim().chars().take(24).collect();
            out.push(ShotIssue {
                shot_num: shot.shot_num,
                field: "source",
                message: format!("原文「{head}」不是逐字出自这一场剧本 —— 只能引剧本原句，不能自己写"),
            });
        }
        for line in &shot.dialogue_lines {
            let key = spoken(&line.line);
            if !key.is_empty() && !whole.contains(&key) {
                out.push(ShotIssue {
                    shot_num: shot.shot_num,
                    field: "dialogue",
                    message: format!("台词「{}」不是剧本原话 —— 台词栏只放剧本里引号中的原话，语气、画外这类说明不要写进去", line.line.trim()),
                });
            }
        }
    }
    let covered: String = shots.iter().map(|s| spoken(&s.source)).collect();
    let first = shots.first().map(|s| s.shot_num).unwrap_or(0);
    for sentence in sentences(&body) {
        let key = spoken(&sentence);
        if key.chars().count() >= 2 && !covered.contains(&key) {
            out.push(ShotIssue {
                shot_num: first,
                field: "source",
                message: format!("剧本里「{}」没有任何一镜拍到 —— 要么并进某一镜并在 source 里引它，要么单独拆一镜", sentence.trim()),
            });
        }
    }
    out
}

/// 把模型抄进「原文」的场名和【】标记去掉，只留剧本句子。
///
/// 实跑：原文写成「现代 · 深夜办公室。键盘声急促。…」「盛唐 · 金銮殿 · 日（第三幕）\n【这一场】\n林婉清目光坚定。」。
/// 场名和标记不是要拍的内容，去掉之后剩下的句子照常逐字核对 —— 编造的句子去不掉，仍然会被报出来。
pub fn clean_sources(shots: &mut [StoryboardShot], scene_name: &str) {
    for shot in shots {
        // 场名只从开头去：场名可能是「办公室」这种短词，正文里出现的不能动
        let trim = |t: &str| t.trim_start_matches(|c: char| c.is_whitespace() || "。，、：;；.".contains(c)).to_string();
        let mut s = trim(&strip_marks(&shot.source));
        for name in [scene_name.trim(), strip_note(scene_name).unwrap_or("")] {
            if !name.is_empty() {
                if let Some(rest) = s.strip_prefix(name) {
                    s = trim(rest);
                }
            }
        }
        shot.source = s.trim().to_string();
    }
}

/// 常见英文镜头 / 灯光术语 → 中文。模型写中文时顺手夹进来的就是这些（实跑：Pan、Rim Light、POV、OTS）。
const ENGLISH_TERMS: [(&str, &str); 22] = [
    ("Rim Light", "轮廓光"), ("Key Light", "主光"), ("Fill Light", "补光"), ("Back Light", "逆光"), ("Backlight", "逆光"),
    ("Close-up", "特写"), ("Close up", "特写"), ("Medium Shot", "中景"), ("Wide Shot", "远景"), ("Long Shot", "远景"),
    ("Dolly", "推轨"), ("Zoom", "变焦"), ("Tilt", "俯仰摇"), ("Pan", "摇镜"), ("Truck", "横移"), ("Crane", "升降"),
    ("POV", "主观视角"), ("OTS", "过肩"), ("Bokeh", "焦外虚化"), ("Tyndall", "丁达尔效应"), ("Slow Motion", "慢动作"), ("Handheld", "手持"),
];

/// 一镜内「切镜」的写法换成运镜说法。模型写「切至」多半是想让镜头转过去，不是真要切（真要切会拆成下一镜）。
const CUT_REWRITES: [(&str, &str); 6] = [("切换到", "转向"), ("镜头切至", "镜头转向"), ("画面切至", "画面转向"), ("切至", "转向"), ("切到", "转到"), ("切回", "转回")];

/// 去掉括号里全是英文的注释：「左右轻微摇摄（Pan）」→「左右轻微摇摄」。
fn strip_english_parens(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let open = rest.find(['（', '(']);
        let Some(at) = open else {
            out.push_str(rest);
            break;
        };
        let open_len = rest[at..].chars().next().unwrap().len_utf8();
        let after = &rest[at + open_len..];
        match after.find(['）', ')']) {
            Some(close) if !after[..close].trim().is_empty() && after[..close].chars().all(|c| c.is_ascii_alphabetic() || c == ' ' || c == '-' || c == '/') => {
                out.push_str(&rest[..at]);
                let close_len = after[close..].chars().next().unwrap().len_utf8();
                rest = &after[close + close_len..];
            }
            _ => {
                out.push_str(&rest[..at + open_len]);
                rest = after;
            }
        }
    }
    out
}

/// 去掉二选一：「（或……）」整段删；「，或者……」删到句末（保留第一种写法）。
fn strip_alternatives(text: &str) -> String {
    let mut s = text.to_string();
    // 括号里的「或…」
    for (open, close) in [('（', '）'), ('(', ')')] {
        while let Some(at) = s.find(&format!("{open}或")) {
            match s[at..].find(close) {
                Some(end) => s.replace_range(at..at + end + close.len_utf8(), ""),
                None => break,
            }
        }
    }
    // 写给自己的说明（「（根据前文'灯火连绵'调整为夜景）」）：括号整段删，不带括号的删掉那一句
    for meta in ["根据前文", "根据剧情", "视情况", "可根据"] {
        while let Some(at) = s.find(meta) {
            let open = s[..at].rfind(['（', '(']);
            let close_paren = s[at..].find(['）', ')']).map(|e| at + e);
            match (open, close_paren) {
                (Some(o), Some(c)) if !s[o..at].contains(['）', ')']) => {
                    let clen = s[c..].chars().next().unwrap().len_utf8();
                    s.replace_range(o..c + clen, "");
                }
                _ => {
                    let start = s[..at].rfind(['。', '；', '，']).map_or(0, |i| i + s[i..].chars().next().unwrap().len_utf8());
                    let end = s[at..].find(['。', '；']).map_or(s.len(), |e| at + e);
                    s.replace_range(start..end, "");
                }
            }
        }
    }
    // 句中的「或者 / 或是 / 抑或」：删到这一句结束
    for w in ["或者", "或是", "抑或", "，或"] {
        while let Some(at) = s.find(w) {
            let start = s[..at].trim_end_matches(['，', ',', '、', ' ']).len();
            let end = s[at..].find(['。', '；', '！', '？', '\n']).map_or(s.len(), |e| at + e);
            s.replace_range(start..end, "");
        }
    }
    s
}

/// 去掉声音 / 画面里又写了一遍的台词。「大臣厉声：'大逆不道！'」→「大臣厉声说话」。
fn strip_repeated_line(text: &str, line: &str) -> String {
    let line = line.trim();
    let bare = line.trim_end_matches(['。', '！', '？', '…', '—', '.', '!', '?']);
    if bare.chars().count() < 3 {
        return text.to_string();
    }
    let Some(at) = text.find(bare) else { return text.to_string() };
    let quotes = ['\'', '"', '“', '”', '「', '」', '‘', '’', '：', ':'];
    // 往前吞掉引号，往后吞掉台词剩下的标点和收尾引号
    let head = text[..at].trim_end_matches(|c| quotes.contains(&c) && c != '：' && c != ':');
    let tail_start = at + bare.len();
    let tail = text[tail_start..].trim_start_matches(|c: char| "。！？…—.!?".contains(c) || quotes.contains(&c) && c != '：' && c != ':');
    let head = match head.strip_suffix('：').or_else(|| head.strip_suffix(':')) {
        Some(h) if !h.trim().is_empty() => format!("{}说话", h.trim_end()),
        Some(h) => h.to_string(),
        None => head.to_string(),
    };
    format!("{head}{tail}").trim_matches(|c: char| c.is_whitespace() || c == '，' || c == ',').to_string()
}

/// **代码直接修掉模型反复犯的固定写法**，返回改了几处。
///
/// 用户反馈（2026-09-14）：这几类问题「反复生成也会报错」—— 重试一次模型还是那么写。
/// 这些写法有固定规律、修法唯一，交给代码比让模型重写一整场可靠：
///
/// - 夹英文（Pan、Rim Light）：括号里的英文注释去掉，常见术语换中文；剩下的仍由校验报
/// - 二选一（「（或…）」「或者…」）：保留第一种写法
/// - 台词在声音 / 画面里又写一遍：删掉重复的那句
/// - 一镜内「切至 / 切回」：改成「转向 / 转回」的运镜说法
/// - 说话人不在出场人物里：说话的人就在画面里，补进出场人物；简称按本场用得最多的全名对上
/// - 时间段只写到 7 秒、时长却填了 15 秒：**时长跟时间段走**（写出来的是时间段，15 是随手填的上限）。
///   2026-09-29 换成 ai-film 原文后实跑，第 6 场十四镜里五镜这样；时间段不到一次生成的下限就不动，留给核对报
pub fn repair_shots(shots: &mut [StoryboardShot], assets: &[AssetRef]) -> usize {
    use crate::assets::AssetKind;
    let mut fixed = 0;
    // **一镜出的镜头不改切镜词** —— 那是它的写法本身（镜内按时间轴切），改成「转向」H3 就不切了
    let fix_text = |t: &mut String, lines: &[String], fixed: &mut usize, keep_cuts: bool| {
        let mut s = strip_alternatives(&strip_english_parens(t));
        for (en, zh) in ENGLISH_TERMS {
            if s.contains(en) {
                s = s.replace(en, zh);
            }
        }
        for (from, to) in CUT_REWRITES.iter().filter(|_| !keep_cuts) {
            s = s.replace(from, to);
        }
        for line in lines {
            s = strip_repeated_line(&s, line);
        }
        if s != *t {
            *t = s;
            *fixed += 1;
        }
    };
    // 本场里每个人物被用了几次，简称对全名时取用得最多的
    let usage = |id: &str, shots: &[StoryboardShot]| shots.iter().filter(|s| s.all_character_ids().iter().any(|c| c == id)).count();
    let snapshot: Vec<StoryboardShot> = shots.to_vec();
    for shot in shots.iter_mut() {
        // 一镜出的时长由代码定（`one_take::fit_duration`），不跟着改
        if !shot.one_take {
            if let Some(end) = shot.segments.last().map(|g| g.end) {
                if end + 0.5 < shot.duration && end >= crate::shot_table::MIN_SHOT_SECS {
                    shot.duration = end;
                    fixed += 1;
                }
            }
        }
        let lines: Vec<String> = shot.dialogue_lines.iter().map(|l| l.line.clone()).filter(|l| !l.trim().is_empty()).collect();
        let keep_cuts = shot.one_take;
        for seg in shot.segments.iter_mut() {
            for t in [&mut seg.visual, &mut seg.forbid, &mut seg.constraints, &mut seg.blocking, &mut seg.camera] {
                fix_text(t, &[], &mut fixed, keep_cuts);
            }
            fix_text(&mut seg.visual, &lines, &mut fixed, keep_cuts);
            fix_text(&mut seg.sound, &lines, &mut fixed, keep_cuts);
        }
        fix_text(&mut shot.lighting, &[], &mut fixed, keep_cuts);
        // 说话人
        for i in 0..shot.dialogue_lines.len() {
            let l = &shot.dialogue_lines[i];
            if l.off_screen || l.speaker.trim().is_empty() {
                continue;
            }
            let speaker = l.speaker.trim().to_string();
            let in_shot = shot.all_character_ids().iter().filter_map(|id| assets.iter().find(|a| &a.id == id)).any(|a| a.name == speaker);
            if in_shot {
                continue;
            }
            let mut candidates: Vec<&AssetRef> = assets
                .iter()
                .filter(|a| a.kind == AssetKind::Character && (a.name == speaker || titled(&a.name, &speaker) || a.name.strip_prefix(speaker.as_str()).is_some_and(|r| r.starts_with('（') || r.starts_with('('))))
                .collect();
            candidates.sort_by_key(|a| std::cmp::Reverse(usage(&a.id, &snapshot)));
            if let Some(a) = candidates.first() {
                // 几个造型都没在本场出现过、分不出是哪一个，就不猜
                if candidates.len() > 1 && usage(&a.id, &snapshot) == usage(&candidates[1].id, &snapshot) {
                    continue;
                }
                if !shot.character_ids.contains(&a.id) {
                    shot.character_ids.push(a.id.clone());
                }
                shot.dialogue_lines[i].speaker = a.name.clone();
                fixed += 1;
            }
        }
    }
    fixed
}

/// 「说话人写错了」那条问题的修法：这一镜第几句台词、该改成谁说。认不出这条消息或找不到那句返回 `None`。
///
/// 消息由 [`dialogue_coverage`] 生成（同一个文件），格式是
/// `台词「X」写成了「W」说的，剧本里是「S」—— …`；按台词和错的说话人一起找，同一句台词两个人都说过时不会改错那一句。
///
/// **改成的名字先对角色名**（`names`）：剧本「大臣厉声：“……”」这种写法解析出来的说话人是「大臣厉声」，
/// 照抄就改错了。忽略标点一致的（「铭牌（合成音）」对「铭牌合成音」）用角色名；剧本名字以某个角色名开头的用最长的那个；
/// 都对不上才照抄剧本
pub fn speaker_fix(shot: &StoryboardShot, message: &str, names: &[&str]) -> Option<(usize, String)> {
    let rest = message.strip_prefix("台词「")?;
    let (line, rest) = rest.split_once("」写成了「")?;
    let (wrong, rest) = rest.split_once("」说的，剧本里是「")?;
    let (speaker, _) = rest.split_once("」")?;
    let (key, who) = (spoken(line), spoken(wrong));
    let i = shot.dialogue_lines.iter().position(|l| spoken(&l.line) == key && spoken(&l.speaker) == who)?;
    let target = spoken(speaker);
    let named = names.iter().find(|n| spoken(n) == target).or_else(|| {
        names.iter().filter(|n| !spoken(n).is_empty() && target.starts_with(&spoken(n))).max_by_key(|n| spoken(n).chars().count())
    });
    Some((i, named.map_or_else(|| speaker.to_string(), |n| n.to_string())))
}

/// 原文里不是剧本原句的部分删掉，只留逐句对得上这一场剧本的句子。返回改了几镜。
///
/// 「原文」只是这一镜对应剧本哪几句的记录，不进提示词，删掉编的句子不会丢画面。
/// 旧表里出现过：「大殿死寂。（接上镜）镜头拉远，展现…」、「盛唐 · 金銮殿 · 日（第二幕）【这一场】…【上一场的结尾】…」。
/// **漏拍的句子这里不补**：那是少了一个镜头，塞进别的镜头的原文只会让检查通过、片子里照样没有。
pub fn repair_sources(shots: &mut [StoryboardShot], scene_name: &str, scene_text: &str) -> usize {
    let whole = spoken(&strip_marks(scene_text));
    let mut fixed = 0;
    clean_sources(shots, scene_name);
    for shot in shots.iter_mut() {
        // 先去掉【…】标记和它后面跟着的提示词标签行（「【这一场】」「【上一场的结尾】」）
        let kept: Vec<String> = sentences(&strip_marks(&shot.source))
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| {
                let k = spoken(s);
                !k.is_empty() && whole.contains(&k)
            })
            .collect();
        let next = kept.join(" ");
        if next != shot.source.trim() {
            shot.source = next;
            fixed += 1;
        }
    }
    fixed
}

/// 去掉【】标记（场景标题、第几幕）—— 那是结构，不是要拍的内容。
fn strip_marks(text: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in text.chars() {
        match c {
            '【' => depth += 1,
            '】' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// 按句末标点切句。引号里的句末标点不切 —— 一句台词算一整句。
fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = 0;
    for c in text.chars() {
        cur.push(c);
        match c {
            '“' | '「' => quoted += 1,
            '”' | '」' => {
                quoted = (quoted - 1).max(0);
                if quoted == 0 {
                    out.push(std::mem::take(&mut cur));
                }
            }
            '。' | '！' | '？' | '\n' if quoted == 0 => out.push(std::mem::take(&mut cur)),
            _ => {}
        }
    }
    out.push(cur);
    out.into_iter().filter(|s| !s.trim().is_empty()).collect()
}

/// 按资产库校验一批镜头（不读盘，测试和生成都用它）。
pub fn validate_shots(shots: &[StoryboardShot], assets: &[AssetRef]) -> Vec<ShotIssue> {
    let known: std::collections::BTreeSet<String> = assets.iter().map(|a| a.id.clone()).collect();
    let mut out = Vec::new();
    for shot in shots {
        let characters = shot.all_character_ids();
        let mut asset_ids: Vec<&str> = characters.iter().map(String::as_str).collect();
        asset_ids.extend(shot.scene_id.as_deref());
        asset_ids.extend(shot.prop_ids.iter().map(String::as_str));
        let character_names: Vec<&str> = characters
            .iter()
            .filter_map(|id| assets.iter().find(|a| &a.id == id && a.kind == crate::assets::AssetKind::Character))
            .map(|a| a.name.as_str())
            .collect();
        let view = shot_table::ShotView {
            shot_num: shot.shot_num,
            duration: shot.duration,
            segments: &shot.segments,
            lines: shot.dialogue_lines.iter().map(|d| (d.speaker.as_str(), d.line.as_str(), d.off_screen)).collect(),
            asset_ids,
            character_names,
            ref_slots: &shot.ref_slots,
            lighting: &shot.lighting,
            one_take: shot.one_take,
        };
        out.extend(shot_table::validate(&view, &known));
    }
    out
}

/// 资产清单写进提示词的样子：一行一个，标出有没有图。
pub fn asset_catalog(assets: &[AssetRef]) -> String {
    use crate::assets::AssetKind;
    let mut lines = Vec::new();
    for (kind, label) in [(AssetKind::Character, "人物"), (AssetKind::Scene, "场景"), (AssetKind::Prop, "道具")] {
        let names: Vec<String> = assets
            .iter()
            .filter(|a| a.kind == kind)
            .map(|a| {
                let tag = if a.has_image { "有图" } else { "无图" };
                let desc: String = a.description.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(80).collect();
                // **声音设定跟着角色走**，整集一份。带上它，模型逐句的 voice
                // 就只用写这一句的变化，不用（也不该）重编一个身份音色出来。
                let voice = (!a.h3_voice.trim().is_empty())
                    .then(|| format!("｜声音：{}", a.h3_voice.trim()))
                    .unwrap_or_default();
                // **形象**（换装、受损）：有不止一个的列出名字，镜头里按剧本选（`assets.looks`）
                let looks = (a.kind == AssetKind::Character && a.looks.len() > 1)
                    .then(|| {
                        let names: Vec<String> = a.looks.iter().map(|l| if l.name.trim().is_empty() { "主形象".to_string() } else { l.name.trim().to_string() }).collect();
                        format!("｜形象：{}", names.join("、"))
                    })
                    .unwrap_or_default();
                if desc.is_empty() { format!("- {}（{tag}）{voice}{looks}", a.name) } else { format!("- {}（{tag}）：{desc}{voice}{looks}", a.name) }
            })
            .collect();
        lines.push(format!("{label}：\n{}", if names.is_empty() { "无".into() } else { names.join("\n") }));
    }
    lines.join("\n")
}

/// 词表写进提示词的样子：`id=名字`，用顿号隔开。
pub fn vocab_line(group: &str) -> String {
    crate::shot_language::group(group)
        .map(|g| g.items.iter().map(|t| format!("{}={}", t.id, t.name)).collect::<Vec<_>>().join("、"))
        .unwrap_or_default()
}

/// **每一场的时长预算**：整集目标时长按每场剧本的字数分下去，但至少留够台词。
///
/// # 为什么要有
///
/// 镜头表的提示词只规定**每一镜** 4~15 秒，从没说**这一场**该多长 —— 模型就按自己的节奏铺。
/// 《回收师》E01 目标 240 秒，同一份剧本三次生成出了 459 / 404 / 488 秒，
/// 多出来的大半是「怔住」「凝视」这种没有台词、没有新信息的段落（2026-09-26）。
///
/// 下限是台词念完要的时间 × 1.4：预算不能逼着删台词，台词是剧本定死的。
/// `target_secs` 为 0（没设目标）就不给预算，和以前一样。
pub fn scene_budgets(scenes: &[SceneChunk], target_secs: u32) -> Vec<Option<f32>> {
    if target_secs == 0 {
        return vec![None; scenes.len()];
    }
    let size = |s: &SceneChunk| s.text.chars().filter(|c| !c.is_whitespace()).count().max(1) as f32;
    let total: f32 = scenes.iter().map(size).sum();
    scenes
        .iter()
        .map(|s| {
            let share = target_secs as f32 * size(s) / total;
            let talk = crate::script_parse::dialogue_of(&s.text)
                .iter()
                .map(|(_, l)| spoken(l).chars().count() as f32)
                .sum::<f32>()
                / 4.0;
            Some(share.max(talk * 1.4).max(shot_table::MIN_SHOT_SECS).round())
        })
        .collect()
}

/// 预算写进提示词的样子。
pub fn scene_budget_line(secs: Option<f32>) -> String {
    match secs {
        Some(b) => format!(
            "这一场所有镜头加起来约 {b} 秒（上下两成以内，最多 {} 秒）。超了先删没有台词、没有新信息的段落和镜头，再压缩动作 —— 台词一句都不删。",
            (b * OVER_BUDGET).round()
        ),
        None => "不限，按剧情需要。".into(),
    }
}

/// 超出预算多少倍算问题。
const OVER_BUDGET: f32 = 1.25;

/// 这一场超预算了吗 —— 算成一个问题，交给重试（带着这句重写，取问题最少的那一版）。
pub fn over_budget(shots: &[StoryboardShot], scene_secs: Option<f32>) -> Vec<ShotIssue> {
    let (Some(b), Some(first)) = (scene_secs, shots.first()) else { return Vec::new() };
    let total: f32 = shots.iter().map(|s| s.duration).sum();
    if total <= b * OVER_BUDGET {
        return Vec::new();
    }
    vec![ShotIssue {
        shot_num: first.shot_num,
        field: "duration",
        message: format!(
            "这一场一共 {total} 秒，预算约 {b} 秒（最多 {} 秒）—— 删掉没有台词、没有新信息的段落和镜头，把相邻的短镜并起来；台词一句都不删",
            (b * OVER_BUDGET).round()
        ),
    }]
}

/// 每一场要不要一镜出：`(场号, 这一场, 一共几场) → Some((几镜, 每镜几秒))`。见 [`crate::one_take::plan`]。
pub type OneTakePlanFn<'a> = dyn Fn(u32, &SceneChunk, usize) -> Option<(u32, f32)> + Sync + 'a;

/// 由剧本生成整集镜头表。`call(slot, vars)` 跑一次 LM，返回原始文本。
///
/// 分场核对不过、或者某场镜头表解析不出来时，各重试一次；仍然不行就报错 ——
/// **不退回任何兜底切分**（以前的「按行切分」兜底会产出一份看起来像分镜、内容全是模板的表）。
pub async fn generate<F, Fut>(
    script: &str,
    assets: &[AssetRef],
    call: F,
    mut progress: impl FnMut(usize, usize, &str),
    one_take: &OneTakePlanFn<'_>,
    target_secs: u32,
) -> Result<(Vec<StoryboardShot>, ShotTableResult)>
where
    F: Fn(crate::prompts::PromptSlot, Vec<(String, String)>) -> Fut,
    Fut: Future<Output = Result<String>>,
{
    use crate::prompts::PromptSlot;
    let script = script.trim();
    if script.is_empty() {
        return Err(Error::runtime("这一集还没有剧本 —— 先在剧本那一步写好"));
    }

    let mut feedback = String::new();
    // 剧本自己写了标题就照标题切，不问模型；没有标题才让模型切（两次机会）
    let mut scenes = split_by_headings(script);
    let tries = if scenes.is_some() { 0 } else { 2 };
    for _ in 0..tries {
        // 模型调用本身报错（本机模型偶尔吐不出合法格式）也算一次失败，给重试机会，不直接中断整集。
        let raw = call(PromptSlot::SceneSplit, vec![("TEXT".into(), script.into()), ("FEEDBACK".into(), feedback.clone())])
            .await
            .map_err(|e| format!("模型调用失败：{e}"));
        match raw.and_then(|raw| parse_scenes(&raw, script)) {
            Ok(s) => {
                scenes = Some(s);
                break;
            }
            Err(e) => feedback = format!("【上次的问题】{e}"),
        }
    }
    let scenes = scenes.ok_or_else(|| Error::runtime(format!("剧本分场没通过核对（重试过一次）：{}", feedback.trim_start_matches("【上次的问题】"))))?;

    let catalog = asset_catalog(assets);
    let budgets = scene_budgets(&scenes, target_secs);
    let mut coverage_issues: Vec<ShotIssue> = Vec::new();
    let mut all: Vec<StoryboardShot> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let mut prev_end = "这是本集第一场".to_string();
    for (i, scene) in scenes.iter().enumerate() {
        progress(i, scenes.len(), &scene.name);
        let first_num = all.len() as u32 + 1;
        let take = one_take(i as u32 + 1, scene, scenes.len());
        let (shots, scene_notes) = generate_scene(scene, i as u32 + 1, first_num, &prev_end, assets, &catalog, &call, "", SCENE_RETRIES, take, budgets[i])
            .await
            .map_err(|e| Error::runtime(format!("第 {} 场「{}」的镜头表没生成出来（重试过一次）：{e}", i + 1, scene.name)))?;
        notes.extend(scene_notes);
        coverage_issues.extend(dialogue_coverage(&scene.text, &shots));
        coverage_issues.extend(source_coverage(&scene.text, &shots));
        coverage_issues.extend(shot_density(&shots));
        coverage_issues.extend(jump_cuts(&shots));
        coverage_issues.extend(missing_stage(&shots));
        if let Some(last) = shots.last() {
            prev_end = scene_end(last);
        }
        all.extend(shots);
    }
    progress(scenes.len(), scenes.len(), "");

    let mut issues = validate_shots(&all, assets);
    issues.extend(coverage_issues);
    let result = ShotTableResult { scenes: scenes.len(), shots: all.len(), issues, notes };
    Ok((all, result))
}

/// 给下一场的「上一场的结尾」：地点、光线、固定布景、画面停在哪。
///
/// **要带空间和光线，不只是最后一段画面。** 只给一句画面的话，模型看不出「这个地方还在」，
/// 于是每场开头都要重新立一遍空间 —— E04 十二镜里有四镜的 duty 是
/// 「承接上一场…落回…」，全是无台词的过场（2026-09-19 用户指出）。
///
/// **固定布景也要带。** EP01「拆机间」第 2 场陆川在左、竹竿在右，第 3 场同一个地点两人对调了 ——
/// 每场各写各的布景，下一场不知道上一场把门、灯、人放在哪（2026-09-28）。
/// 整集生成和单独重新生成一场都走这里，两边给模型的是同一份。
pub fn scene_end(last: &StoryboardShot) -> String {
    let visual = last.segments.last().map(|s| s.visual.clone()).unwrap_or_else(|| last.prompt.clone());
    let stage = last.stage.lines();
    [
        (!last.scene_name.is_empty()).then(|| format!("地点：{}", last.scene_name)),
        (!last.lighting.trim().is_empty()).then(|| format!("光线：{}", last.lighting.trim())),
        (!stage.is_empty()).then(|| format!("那一场的固定布景：{}", stage.join("；"))),
        (!visual.trim().is_empty()).then(|| format!("画面停在：{}", visual.trim())),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("；")
}

/// 解析失败时把模型原文整段写进日志。
///
/// 界面上只有一句「JSON 解析失败：line 9 column 98」，不看原文根本定位不了是哪种写错 ——
/// 而模型输出不可复现，事后重跑多半就不坏了（2026-09-13 就是这样查不下去的）。
/// 一场最多重写几次（不算第一次）。
///
/// 以前是 1：第一次要是白写了（比如只写了布景），第二次写成什么样就留什么样，问题带着落盘
/// （2026-09-29 查 EP01：6 场里 4 场带着问题）。多一次按问题修正的机会，最坏每场多三四分钟。
pub const SCENE_RETRIES: usize = 2;

fn log_bad_output(scene: &str, error: &str, raw: &str) {
    tracing::warn!(scene, error, raw, "镜头表解析失败，模型原文附后");
}

/// 生成一场的镜头表：第一次不带问题，之后把上一次的问题原文反馈给模型重写，最多重试 `retries` 次，
/// **取问题最少的那一份**（一样少取后出的）。每次都先经过代码修正（原文去标记、固定写法）再算问题。
///
/// `initial_feedback` 非空时第一次就带上 —— 单独重新生成一场时，把这一场现有的问题（比如漏拍了哪几句）先告诉模型。
/// 所有次数都没解析出来才报错。
#[allow(clippy::too_many_arguments)]
pub async fn generate_scene<F, Fut>(
    scene: &SceneChunk,
    scene_no: u32,
    first_num: u32,
    prev_end: &str,
    assets: &[AssetRef],
    catalog: &str,
    call: &F,
    initial_feedback: &str,
    retries: usize,
    one_take: Option<(u32, f32)>,
    scene_secs: Option<f32>,
) -> std::result::Result<(Vec<StoryboardShot>, Vec<String>), String>
where
    F: Fn(crate::prompts::PromptSlot, Vec<(String, String)>) -> Fut,
    Fut: Future<Output = Result<String>>,
{
    use crate::prompts::PromptSlot;
    // **一镜出（`one_take`）的场换一个槽位**：镜数和每镜时长由代码给，模型只写内容，镜内允许切镜
    let slot = if one_take.is_some() { PromptSlot::ShotTableOneTake } else { PromptSlot::ShotTable };
    let vars = |feedback: &str| -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> = vec![
            ("SCENE_NAME".into(), scene.name.clone()),
            ("SCENE_TEXT".into(), scene.text.clone()),
            ("PREV".into(), prev_end.to_string()),
            ("ASSETS".into(), catalog.to_string()),
            // 拍法只有这一个来源：ai-film 分镜导演的原文（见 `ai_film` 模块注释）
            ("METHOD".into(), crate::ai_film::shot_table_method()),
            ("SHOT_SIZES".into(), vocab_line("shotSize")),
            ("ANGLES".into(), vocab_line("angle")),
            ("MOVES".into(), vocab_line("move")),
            ("FEEDBACK".into(), feedback.to_string()),
        ];
        match one_take {
            Some((n, secs)) => {
                v.push(("SHOT_COUNT".into(), n.to_string()));
                v.push(("SHOT_SECS".into(), format!("{secs}")));
                v.push(("TOTAL_SECS".into(), format!("{}", n as f32 * secs)));
            }
            None => {
                v.push(("MIN_SECS".into(), format!("{}", shot_table::MIN_SHOT_SECS)));
                v.push(("MAX_SECS".into(), format!("{}", shot_table::MAX_SHOT_SECS)));
                v.push(("SCENE_SECS".into(), scene_budget_line(scene_secs)));
            }
        }
        v
    };
    let scene_issues = |shots: &[StoryboardShot]| {
        let mut v = validate_shots(shots, assets);
        v.extend(dialogue_coverage(&scene.text, shots));
        v.extend(shot_density(shots));
        v.extend(jump_cuts(shots));
        v.extend(missing_stage(shots));
        v.extend(source_coverage(&scene.text, shots));
        if one_take.is_none() {
            v.extend(over_budget(shots, scene_secs));
        }
        if let Some((n, _)) = one_take {
            if shots.len() != n as usize {
                v.push(ShotIssue {
                    shot_num: shots.first().map(|s| s.shot_num).unwrap_or(0),
                    field: "duration",
                    message: format!("这一场要写成 {n} 镜（一镜出），写成了 {} 镜 —— 不要自己拆镜头，要换机位就在镜内按时间写切镜", shots.len()),
                });
            }
        }
        v
    };
    let mut feedback = if initial_feedback.trim().is_empty() { String::new() } else { format!("## 这一场现在的问题，这次必须改掉\n{initial_feedback}") };
    let mut best: Option<(Vec<StoryboardShot>, Vec<String>, usize)> = None;
    let mut last_err = String::new();
    for _ in 0..=retries {
        let mut notes = Vec::new();
        let attempt = call(slot, vars(&feedback))
            .await
            .map_err(|e| format!("模型调用失败：{e}"))
            .and_then(|raw| parse_shots(&raw, assets, first_num, &mut notes).inspect_err(|e| log_bad_output(&scene.name, e, &raw)))
            .map(|mut shots| {
                // 一镜出的标记**先打上再修写法** —— 修写法会把「切至」改成「转向」，一镜出的镜头不能改
                if let Some((_, secs)) = one_take {
                    for s in &mut shots {
                        s.one_take = true;
                        crate::one_take::fit_duration(s, secs);
                    }
                }
                clean_sources(&mut shots, &scene.name);
                snap_lines_to_script(&mut shots, &scene.text);
                repair_shots(&mut shots, assets);
                shots
            });
        match attempt {
            Ok(shots) => {
                let issues = scene_issues(&shots);
                let n = issues.len();
                if best.as_ref().is_none_or(|(_, _, b)| n <= *b) {
                    best = Some((shots, notes, n));
                }
                if n == 0 {
                    break;
                }
                feedback = format!("## 上次的问题，这次必须改掉\n{}", issues.iter().map(|i| format!("第 {} 镜：{}", i.shot_num, i.message)).collect::<Vec<_>>().join("；"));
            }
            Err(e) => {
                feedback = format!("## 上次的问题，这次必须改掉\n{e}");
                last_err = e;
            }
        }
    }
    let (mut shots, notes, _) = best.ok_or(last_err)?;
    for shot in &mut shots {
        shot.scene_no = scene_no;
        shot.scene_name = scene.name.clone();
    }
    Ok((shots, notes))
}

/// 这一集能用的资产：全局 + 本项目 + 本集。`has_image` 看选中的图文件真的在盘上。
pub fn load_assets(config_dir: &std::path::Path, project_id: &str, episode_id: &str) -> Vec<AssetRef> {
    use crate::assets::Scope;
    let mut out: Vec<AssetRef> = Vec::new();
    // **从近到远**：按名字找资产（`lookup`）取第一个，全局库排在前面的话，
    // 全局库里一个同名角色会顶掉这一集自己的（2026-09-15 查出来的）
    for scope in [
        Scope::Episode { project_id: project_id.into(), episode_id: episode_id.into() },
        Scope::Project { project_id: project_id.into() },
        Scope::Global,
    ] {
        for a in crate::assets::load_store(config_dir, &scope).assets {
            let has_img = |unit: &crate::assets::AssetUnit| unit.selected_image().is_some_and(|v| crate::assets::variant_abs_path(config_dir, &scope, v).is_file());
            // 角色和道具都有形象（道具：协议铭牌的「星澜专属」款）。场景没有
            let looks: Vec<LookRef> = if matches!(a.kind, crate::assets::AssetKind::Character | crate::assets::AssetKind::Prop) && !a.looks.is_empty() {
                std::iter::once(LookRef { id: crate::assets::MAIN_LOOK.into(), name: a.main_look_name.clone(), description: a.look_description(None), has_image: has_img(&a.unit) })
                    .chain(a.looks.iter().map(|l| LookRef { id: l.id.clone(), name: l.name.clone(), description: a.look_description(Some(&l.id)), has_image: has_img(&l.unit) }))
                    .collect()
            } else {
                Vec::new()
            };
            let has_image = has_img(&a.unit);
            // 角色资产上写死的 H3 声音设定 —— 带进资产清单，模型就不用每场重编一次
            let h3_voice = a.character.as_ref().and_then(|c| c.h3_voice.clone()).unwrap_or_default();
            out.push(AssetRef { id: a.id.clone(), kind: a.kind, name: a.name.clone(), has_image, description: a.look_description(None), look_id: String::new(), looks, h3_voice });
        }
    }
    // 项目的默认形象换进去 —— 后面所有按资产取描述、取图的地方拿到的就是默认形象
    let defaults = crate::looks::project_default_looks(config_dir, project_id);
    crate::looks::apply(&out, &defaults)
}

#[cfg(test)]
mod live {
    #[tokio::test]
    #[ignore = "要本机后台模型（LM Studio）和真实项目数据"]
    async fn 镜头表实跑() {
        let config_dir = dirs::config_dir().unwrap().join("com.anima.desktop");
        let project = std::env::var("ANIMA_LIVE_PROJECT").unwrap_or_else(|_| "drama_1789250410".into());
        let episode = std::env::var("ANIMA_LIVE_EPISODE").unwrap_or_else(|_| "ep_01".into());
        let ep_file = crate::cinema::cinema_projects_dir(&config_dir).join(&project).join("episodes").join(&episode).join("episode.json");
        let ep: crate::cinema::EpisodeData = serde_json::from_str(&std::fs::read_to_string(ep_file).unwrap()).unwrap();
        let assets = super::load_assets(&config_dir, &project, &episode);
        let mut app_cfg = crate::config::AppConfig::load(&config_dir.join("config.json")).unwrap();
        // 对比模型时临时换后台模型，不改用户的配置文件
        if let Ok(m) = std::env::var("ANIMA_LIVE_MODEL") {
            app_cfg.background_model = Some(m);
        }
        let model = app_cfg.background_model.clone().unwrap_or_default();
        println!("后台模型：{model}");
        let gateway = std::sync::Arc::new(crate::llm::gateway::Gateway::new(app_cfg).unwrap());
        let cfg = crate::prompts::load(&config_dir);
        let started = std::time::Instant::now();
        let call = |slot: crate::prompts::PromptSlot, vars: Vec<(String, String)>| {
            let gateway = gateway.clone();
            let cfg = cfg.clone();
            async move {
                let t = std::time::Instant::now();
                let vars_ref: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                let out = crate::prompts::run(&gateway, &cfg, slot, &vars_ref).await;
                println!("  {:?} 用时 {:.1}s，返回 {} 字", slot, t.elapsed().as_secs_f32(), out.as_ref().map(|s| s.chars().count()).unwrap_or(0));
                out
            }
        };
        let (shots, result) = super::generate(&ep.script_text, &assets, call, |d, t, s| println!("场 {d}/{t} {s}"), &|_, _, _| None, 0).await.unwrap();
        println!("总用时 {:.0}s：{} 场 {} 镜，问题 {} 条，备注 {} 条", started.elapsed().as_secs_f32(), result.scenes, result.shots, result.issues.len(), result.notes.len());
        let out = std::env::temp_dir().join(format!("anima-shot-table-live-{}.json", model.replace('/', "_")));
        std::fs::write(&out, serde_json::to_string_pretty(&serde_json::json!({
            "model": model, "seconds": started.elapsed().as_secs(), "result": result, "shots": shots
        })).unwrap()).unwrap();
        println!("结果写在 {}", out.display());
    }

    /// **只重跑几场的镜头表，只读**：看固定布景有没有写、同一个地点跨场左右和灯的方向稳不稳。
    ///
    /// `./scripts/with-env.sh cargo test --lib 镜头表几场实跑 -- --ignored --nocapture`
    /// （`ANIMA_LIVE_PROJECT` / `ANIMA_LIVE_EPISODE` 默认《回收师》EP01，`ANIMA_LIVE_SCENES` 默认 `2,3` —— 拆机间两场）
    #[tokio::test]
    #[ignore = "要本机后台模型（LM Studio）和真实项目数据"]
    async fn 镜头表几场实跑() {
        let config_dir = dirs::config_dir().unwrap().join("com.anima.desktop");
        let project = std::env::var("ANIMA_LIVE_PROJECT").unwrap_or_else(|_| "drama_1790397900".into());
        let episode = std::env::var("ANIMA_LIVE_EPISODE").unwrap_or_else(|_| "ep_01".into());
        let wanted: Vec<u32> = std::env::var("ANIMA_LIVE_SCENES").unwrap_or_else(|_| "2,3".into()).split(',').filter_map(|x| x.trim().parse().ok()).collect();
        let ep = crate::cinema::read_episode(&config_dir, &project, &episode).unwrap();
        let scenes = super::split_by_headings(&ep.script_text).expect("剧本要有【】标题");
        let assets = super::load_assets(&config_dir, &project, &episode);
        let catalog = super::asset_catalog(&assets);
        let mut app_cfg = crate::config::AppConfig::load(&config_dir.join("config.json")).unwrap();
        // `ANIMA_LIVE_MODEL`：换一个后台模型跑（比模型用，不改用户的配置文件）。换了就不替它加载 —— 先手动 `lms load`
        // `ANIMA_LIVE_PROVIDER` / `ANIMA_LIVE_BASE_URL`：换后端、换那个后端的地址（比云端模型用；
        // Azure 上的 Claude 要走 Anthropic 格式 —— 把 claude 后端指到 `<Azure 端点>/anthropic`）
        if let Ok(p) = std::env::var("ANIMA_LIVE_PROVIDER") {
            println!("后台后端换成 {p}");
            if let Ok(url) = std::env::var("ANIMA_LIVE_BASE_URL") {
                if let Some(pr) = app_cfg.providers.iter_mut().find(|x| x.id == p) {
                    pr.base_url = url;
                }
            }
            app_cfg.background_provider = p;
        }
        match std::env::var("ANIMA_LIVE_MODEL") {
            Ok(m) => {
                println!("后台模型换成 {m}");
                app_cfg.background_model = Some(m);
            }
            Err(_) => crate::llm_memory::ensure_background_loaded(&config_dir).unwrap(),
        }
        let gateway = crate::llm::gateway::Gateway::new(app_cfg).unwrap();
        let mut cfg = crate::prompts::load(&config_dir);
        // `ANIMA_LIVE_TEMPLATE`：拿一份文件当镜头表模板跑（新旧模板对比用，不动用户的提示词设置）
        if let Ok(path) = std::env::var("ANIMA_LIVE_TEMPLATE") {
            let text = std::fs::read_to_string(&path).unwrap();
            println!("镜头表模板换成 {path}（{} 字）", text.chars().count());
            cfg.overrides.insert(crate::prompts::PromptSlot::ShotTable.key().to_string(), text);
        }
        let call = |slot: crate::prompts::PromptSlot, vars: Vec<(String, String)>| {
            let (gateway, cfg) = (&gateway, &cfg);
            async move {
                let t = std::time::Instant::now();
                let v: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                let out = crate::prompts::run(gateway, cfg, slot, &v).await;
                println!("  {slot:?} 用时 {:.0}s", t.elapsed().as_secs_f32());
                out
            }
        };
        let mut sorted = ep.storyboard.clone();
        sorted.sort_by_key(|s| s.shot_num);
        let first = wanted[0];
        let mut prev_end = sorted.iter().filter(|s| s.scene_no < first).last().map(super::scene_end).unwrap_or_else(|| "这是本集第一场".into());
        let mut first_num = sorted.iter().filter(|s| s.scene_no < first).count() as u32 + 1;
        let budgets = super::scene_budgets(&scenes, ep.target_secs);
        let mut all = Vec::new();
        for no in wanted {
            let scene = &scenes[(no - 1) as usize];
            println!("\n== 第 {no} 场「{}」", scene.name);
            let (shots, notes) = super::generate_scene(scene, no, first_num, &prev_end, &assets, &catalog, &call, "", super::SCENE_RETRIES, None, budgets[(no - 1) as usize]).await.unwrap();
            println!("固定布景：\n  {}", shots[0].stage.lines().join("\n  "));
            for s in &shots {
                println!("第 {} 镜 {} · {} · {}，{} 秒\n  光：{}", s.shot_num, s.scale, s.camera_angle, s.camera_movement, s.duration, s.lighting);
                for g in &s.segments {
                    println!("  {:>4.1}~{:<4.1} 站位：{}\n             运镜：{}", g.start, g.end, g.blocking, g.camera);
                }
            }
            // 和生成时挑「问题最少的一份」用的是同一组核对
            let mut issues = super::validate_shots(&shots, &assets);
            issues.extend(super::dialogue_coverage(&scene.text, &shots));
            issues.extend(super::shot_density(&shots));
            issues.extend(super::jump_cuts(&shots));
            issues.extend(super::missing_stage(&shots));
            issues.extend(super::source_coverage(&scene.text, &shots));
            issues.extend(super::over_budget(&shots, budgets[(no - 1) as usize]));
            println!("问题 {} 条：", issues.len());
            for i in &issues {
                println!("  第 {} 镜 {}：{}", i.shot_num, i.field, i.message);
            }
            println!("备注：{notes:?}");
            first_num += shots.len() as u32;
            prev_end = super::scene_end(shots.last().unwrap());
            all.extend(shots);
        }
        let out = std::env::var("ANIMA_LIVE_OUT").map(std::path::PathBuf::from).unwrap_or_else(|_| std::env::temp_dir().join("anima-shot-table-scenes-live.json"));
        std::fs::write(&out, serde_json::to_string_pretty(&all).unwrap()).unwrap();
        println!("\n结果写在 {}", out.display());
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    fn scene(text: &str) -> SceneChunk {
        SceneChunk { name: "x".into(), text: text.into() }
    }

    /// 2026-09-26：E01 目标 240 秒，三次生成出了 459 / 404 / 488 秒 —— 提示词里从没说这一场该多长。
    #[test]
    fn 整集目标按字数分到每一场_台词念完是下限() {
        let long = scene(&"动作描写。".repeat(80));
        let short = scene("陆川：你好。");
        let b = scene_budgets(&[long, short], 240);
        let (l, s) = (b[0].unwrap(), b[1].unwrap());
        assert!(l > s * 10.0, "字多的场预算大：{l} vs {s}");
        // 台词多但字数少的场：预算不能少于台词念完 × 1.4
        let talky = scene(&"星澜：本机提供陪聊、按摩、情绪价值三种套餐，请问主人需要哪一种。\n".repeat(6));
        let quiet = scene(&"雨一直在下。".repeat(200));
        let b = scene_budgets(&[talky.clone(), quiet], 60);
        let talk: f32 = crate::script_parse::dialogue_of(&talky.text).iter().map(|(_, l)| spoken(l).chars().count() as f32).sum::<f32>() / 4.0;
        assert!(b[0].unwrap() >= (talk * 1.4).floor(), "预算逼着删台词了：{:?} < {}", b[0], talk * 1.4);
        // 没设目标 = 不给预算
        assert!(scene_budgets(&[scene("x")], 0)[0].is_none());
    }

    #[test]
    fn 头衔加名字认得出_单字和括号不乱配() {
        assert!(titled("老秦", "主管老秦"));
        assert!(titled("铁头", "打手铁头"));
        assert!(!titled("老秦", "老秦"), "一样的名字走精确匹配");
        assert!(!titled("邓", "老秦"), "单字资产名太容易误配");
        assert!(!titled("陆川", "云穹回收站夜班回收师陆川"), "前面多出太多不算头衔");
        assert!(!titled("星澜", "星澜（战斗款）"));
    }

    fn line_shot(line: &str, source: &str) -> StoryboardShot {
        let mut s: StoryboardShot = serde_json::from_value(serde_json::json!({
            "id": "s", "shot_num": 1, "scale": "medium", "camera_movement": "static", "dialogue": "",
            "prompt": "", "duration": 8.0, "status": "pending", "source": source,
            "dialogue_lines": [{"speaker": "陆川", "line": line}]
        })).unwrap();
        s.source = source.into();
        s
    }

    /// 《回收师》E01 四次生成都有：「拆完你就不用」抄成「拆完你不用」，被判成两个问题还改不回来。
    #[test]
    fn 差一两个字的台词换回剧本原句_差多了不猜() {
        let script = "陆川：别怕，就一下。拆完你就不用再伺候上城那帮人了。\n竹竿：卧槽，停电了？";
        let mut shots = vec![line_shot("别怕，就一下。拆完你不用再伺候上城那帮人了。", "陆川：别怕，就一下。拆完你不用再伺候上城那帮人了。")];
        assert_eq!(snap_lines_to_script(&mut shots, script), 1);
        assert_eq!(shots[0].dialogue_lines[0].line, "别怕，就一下。拆完你就不用再伺候上城那帮人了。");
        assert!(shots[0].source.contains("拆完你就不用"), "原文里那一段也要一起换：{}", shots[0].source);
        // 差得多的不猜
        let mut other = vec![line_shot("我们明天再说吧，今天太晚了。", "")];
        assert_eq!(snap_lines_to_script(&mut other, script), 0);
        // 本来就对的不动
        let mut ok = vec![line_shot("卧槽，停电了？", "")];
        assert_eq!(snap_lines_to_script(&mut ok, script), 0);
    }

    #[test]
    fn 超预算算问题_没超不算() {
        let shot = |d: f32| -> StoryboardShot {
            serde_json::from_value(serde_json::json!({
                "id": "s", "shot_num": 3, "scale": "medium", "camera_movement": "static", "dialogue": "",
                "prompt": "", "duration": d, "status": "pending"
            })).unwrap()
        };
        assert!(over_budget(&[shot(10.0), shot(12.0)], Some(20.0)).is_empty(), "22 ≤ 25 不算");
        let i = over_budget(&[shot(15.0), shot(15.0)], Some(20.0));
        assert_eq!(i.len(), 1);
        assert!(i[0].message.contains("台词一句都不删"));
        assert!(over_budget(&[shot(99.0)], None).is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::AssetKind;
    use std::sync::Mutex;

    const SCRIPT: &str = "《我在盛唐》\n**类型**\n古风\n\n【现代 · 深夜办公室】\n键盘声急促。林婉清伏案加班。\n手机提示音不断。她抬头，视线模糊——黑屏。\n【盛唐 · 金銮殿 · 日】\n鼓声震天。林婉清猛然睁眼，已跪在大殿中央。\n大臣厉声：大逆不道！";

    /// 去掉【】—— 走「模型分场」那条路的测试用。有两个以上标题的剧本不会问模型。
    fn plain(s: &str) -> String {
        s.replace(['【', '】'], "")
    }

    fn assets() -> Vec<AssetRef> {
        vec![
            AssetRef { id: "char_shen".into(), kind: AssetKind::Character, name: "林婉清".into(), has_image: true, description: String::new(), look_id: String::new(), looks: Vec::new(), h3_voice: String::new() },
            AssetRef { id: "char_minister".into(), kind: AssetKind::Character, name: "大臣".into(), has_image: false, description: String::new(), look_id: String::new(), looks: Vec::new(), h3_voice: String::new() },
            AssetRef { id: "scene_office".into(), kind: AssetKind::Scene, name: "现代办公室".into(), has_image: false, description: String::new(), look_id: String::new(), looks: Vec::new(), h3_voice: String::new() },
            AssetRef { id: "prop_phone".into(), kind: AssetKind::Prop, name: "手机".into(), has_image: true, description: String::new(), look_id: String::new(), looks: Vec::new(), h3_voice: String::new() },
        ]
    }

    fn scenes_json(first: &str) -> String {
        serde_json::json!({"scenes": [
            {"name": "现代办公室 · 深夜", "text": first},
            {"name": "金銮殿 · 日", "text": "【盛唐 · 金銮殿 · 日】\n鼓声震天。林婉清猛然睁眼，已跪在大殿中央。\n大臣厉声：大逆不道！"}
        ]}).to_string()
    }

    const OFFICE: &str = "【现代 · 深夜办公室】\n键盘声急促。林婉清伏案加班。\n手机提示音不断。她抬头，视线模糊——黑屏。";

    /// 换掉原文。场级校验要求原文逐字出自剧本、整场都被引到。
    fn sourced(mut shot: serde_json::Value, source: &str) -> serde_json::Value {
        shot["source"] = serde_json::json!(source);
        shot
    }

    /// 换掉台词。场级校验要求台词是剧本原话。
    fn said(mut shot: serde_json::Value, line: Option<&str>) -> serde_json::Value {
        let speaker = shot["dialogue"][0]["speaker"].clone();
        shot["dialogue"] = match line {
            Some(l) => serde_json::json!([{"speaker": speaker, "line": l}]),
            None => serde_json::json!([]),
        };
        shot
    }

    fn good_shot(speaker: &str) -> serde_json::Value {
        serde_json::json!({
            "duty": "交代她撑到极限", "source": "键盘声急促。", "duration": 8,
            "scale": "medium", "angle": "eye_level", "move": "static", "camera_note": "固定",
            "lighting": "显示器在她面前，冷蓝光打脸，背景全暗",
            "scene_items": [{"name": "手机", "detail": "长约 15 厘米，放在键盘右边"}],
            "segments": [
                {"start": 0, "end": 4, "title": "打字", "visual": "双手敲键盘", "constraints": "蓝光为主光源"},
                {"start": 4, "end": 8, "title": "晕倒", "visual": "向前倒下，黑屏"}
            ],
            "dialogue": [{"speaker": speaker, "line": "我还能撑。"}],
            "assets": {"characters": ["林婉清"], "scene": "现代办公室", "props": ["手机"]},
            "ref_slots": [{"asset": "林婉清", "usage": "identity", "control": "脸型、发型、白衬衫", "not_control": "姿势、构图"}]
        })
    }

    /// 场级的固定布景。镜头表每场都要写 —— 夹具不带的话每一场都会被报「没写 stage」。
    fn test_stage() -> serde_json::Value {
        serde_json::json!({
            "facing": "从办公室门口往里看",
            "anchors": ["工位：正对门口靠窗"],
            "lights": ["显示器：在她面前，冷蓝光打脸"],
            "axis": "只有她一个人：机位在工位正前方",
            "people": ["林婉清：坐在工位前，面朝显示器"],
            "time": "深夜"
        })
    }

    #[test]
    fn 分场必须逐字抄原文_按顺序() {
        let ok = parse_scenes(&scenes_json(OFFICE), SCRIPT).unwrap();
        assert_eq!(ok.len(), 2);
        // 空白和换行变了不算改原文
        assert!(parse_scenes(&scenes_json(&OFFICE.replace('\n', "  ")), SCRIPT).is_ok());
        // 改了一个字
        let err = parse_scenes(&scenes_json(&OFFICE.replace("加班", "工作")), SCRIPT).unwrap_err();
        assert!(err.contains("第 1 场") && err.contains("逐字"), "{err}");
        // 顺序颠倒
        let swapped = serde_json::json!({"scenes": [
            {"name": "b", "text": "鼓声震天。"}, {"name": "a", "text": "键盘声急促。"}
        ]}).to_string();
        assert!(parse_scenes(&swapped, SCRIPT).is_err());
    }

    #[test]
    fn 分场漏了大段正文要报() {
        let only_first = serde_json::json!({"scenes": [{"name": "a", "text": "键盘声急促。"}]}).to_string();
        let err = parse_scenes(&only_first, SCRIPT).unwrap_err();
        assert!(err.contains("漏了正文"), "{err}");
    }

    #[test]
    fn 镜头表的资产名换成id_没图的参考图留着_库里没有的记下来() {
        let mut shot = good_shot("林婉清");
        shot["ref_slots"] = serde_json::json!([
            {"asset": "现代办公室", "usage": "geography", "control": "工位"},
            {"asset": "林婉清", "usage": "identity", "control": "脸"},
            {"asset": "皇帝", "usage": "identity", "control": "脸"}
        ]);
        shot["assets"]["props"] = serde_json::json!(["手机", "玉佩"]);
        let raw = serde_json::json!({"stage": test_stage(), "shots": [shot]}).to_string();
        let mut notes = Vec::new();
        let shots = parse_shots(&raw, &assets(), 5, &mut notes).unwrap();
        let s = &shots[0];
        assert_eq!(s.shot_num, 5);
        assert_eq!(s.character_ids, ["char_shen"]);
        assert_eq!(s.scene_id.as_deref(), Some("scene_office"));
        assert_eq!(s.prop_ids, ["prop_phone"]);
        // 没图的办公室留着（合成时按文字写、补了图再合成就挂上）；库里没有的皇帝不占槽位
        assert_eq!(s.ref_slots.iter().map(|r| (r.order, r.asset_id.as_str())).collect::<Vec<_>>(), [(1, "scene_office"), (2, "char_shen")]);
        assert!(notes.iter().any(|n| n.contains("现代办公室") && n.contains("还没有图") && n.contains("重新合成")), "{notes:?}");
        assert!(notes.iter().any(|n| n.contains("皇帝")), "{notes:?}");
        assert!(notes.iter().any(|n| n.contains("玉佩") && n.contains("按文字描述")), "{notes:?}");
        assert_eq!(s.unlinked_assets, vec![UnlinkedAsset { kind: "prop".into(), name: "玉佩".into() }], "库里没有的道具要记下来，不能丢");
        assert_eq!(s.segments.len(), 2);
        assert_eq!(s.dialogue, "林婉清：我还能撑。");
    }

    /// **这一场的固定布景挂到这一场的每一镜上**；写成字符串、数组、对象都认，不写也不算解析失败。
    /// 时间段只写到 8 秒、时长却填了 15：时长跟时间段走。时间段短到不够一次生成的，不动（留给核对报）；一镜出的不动
    #[test]
    fn 时长填满了但时间段没写到_时长跟时间段走() {
        let mut v = good_shot("林婉清");
        v["duration"] = serde_json::json!(15);
        let raw = serde_json::json!({"shots": [v.clone()]}).to_string();
        let mut shots = parse_shots(&raw, &assets(), 1, &mut Vec::new()).unwrap();
        repair_shots(&mut shots, &assets());
        assert_eq!(shots[0].duration, 8.0);

        let mut short = v.clone();
        short["segments"] = serde_json::json!([{"start": 0, "end": 3, "title": "打字", "visual": "双手敲键盘"}]);
        let mut shots = parse_shots(&serde_json::json!({"shots": [short]}).to_string(), &assets(), 1, &mut Vec::new()).unwrap();
        repair_shots(&mut shots, &assets());
        assert_eq!(shots[0].duration, 15.0, "不到 4 秒的时间段不拿来当时长");

        let mut shots = parse_shots(&raw, &assets(), 1, &mut Vec::new()).unwrap();
        shots[0].one_take = true;
        repair_shots(&mut shots, &assets());
        assert_eq!(shots[0].duration, 15.0, "一镜出的时长由代码定");
    }

    /// 2026-09-28~29 七次：模型写完 stage 就把 JSON 合上。反馈给它的要是看得懂的中文，不是 serde 的 `missing field shots`
    #[test]
    fn 只写了布景没写镜头_报一句看得懂的话() {
        let raw = serde_json::json!({"stage": {"facing": "从门口往里看", "time": "夜"}}).to_string();
        let err = parse_shots(&raw, &assets(), 1, &mut Vec::new()).unwrap_err();
        assert!(err.contains("没有写 shots") && err.contains("stage 后面必须接着写 shots"), "{err}");
        assert!(!err.contains("missing field"), "{err}");
    }

    #[test]
    fn 固定布景挂到每一镜_宽松读_没写要报() {
        let raw = serde_json::json!({
            "stage": {"facing": "从门口往里看", "anchors": "工位在窗边；打印机在门右手边", "lights": [{"光源": "显示器", "位置": "她面前"}], "axis": ["她 ↔ 显示器", "机位在工位前"], "time": "深夜"},
            "shots": [good_shot("林婉清"), good_shot("林婉清")]
        })
        .to_string();
        let shots = parse_shots(&raw, &assets(), 1, &mut Vec::new()).unwrap();
        assert!(shots.iter().all(|s| s.stage == shots[0].stage), "一场一份，每一镜都一样");
        let st = &shots[0].stage;
        assert_eq!(st.anchors, ["工位在窗边", "打印机在门右手边"], "字符串按分号拆开");
        assert!(st.lights.len() == 1 && st.lights[0].contains("显示器") && st.lights[0].contains("她面前"), "对象把值连起来：{:?}", st.lights);
        assert_eq!(st.axis, "她 ↔ 显示器；机位在工位前", "数组连成一句");
        assert!(st.people.is_empty() && st.time == "深夜");
        assert_eq!(missing_stage(&shots), vec![]);
        // 布景里写成画面位置要报：换了机位就推不出来（实跑查出来的）
        let mut screen = shots.clone();
        screen[1].stage.anchors.push("拆机台：画面左下方".into());
        screen[0].stage = screen[1].stage.clone();
        let m = missing_stage(&screen);
        assert!(m.len() == 1 && m[0].message.contains("拆机台：画面左下方") && m[0].message.contains("场里的位置"), "{m:?}");
        // 老模板 / 改过的模板没这一项：照常解析，但要报出来
        let old = serde_json::json!({"shots": [good_shot("林婉清")]}).to_string();
        let shots = parse_shots(&old, &assets(), 5, &mut Vec::new()).unwrap();
        assert!(shots[0].stage.is_empty());
        let m = missing_stage(&shots);
        assert_eq!((m.len(), m[0].shot_num, m[0].field), (1, 5, "stage"));
        // 不写盘：老分集存回去一个字不多
        assert!(!serde_json::to_string(&shots[0]).unwrap().contains("\"stage\""));
    }

    /// **跳切**：同一场、同一批人、景别机位都一样的相邻两镜。一镜出的窗口和换了场的不算。
    #[test]
    fn 同场同人同景别同机位的相邻两镜报跳切() {
        let mk = |num: u32, scene: u32, scale: &str| {
            let mut v = good_shot("林婉清");
            v["scale"] = serde_json::json!(scale);
            let mut s = parse_shots(&serde_json::json!({"shots": [v]}).to_string(), &assets(), num, &mut Vec::new()).unwrap().remove(0);
            s.scene_no = scene;
            s
        };
        let shots = vec![mk(1, 1, "medium"), mk(2, 1, "medium"), mk(3, 1, "close_up"), mk(4, 2, "close_up")];
        let j = jump_cuts(&shots);
        assert_eq!(j.len(), 1, "{j:?}");
        assert_eq!(j[0].shot_num, 2);
        assert!(j[0].message.contains("第 1 和第 2 镜") && j[0].message.contains("中景 · 平视") && j[0].message.contains("观看距离、视点、反应"), "{}", j[0].message);
        assert!(!j[0].message.contains("景别跳一级"), "反馈里又是老规则：{}", j[0].message);
        // 换了机位就不算
        let mut other = shots.clone();
        other[1].camera_angle = "over_shoulder".into();
        assert_eq!(jump_cuts(&other), vec![]);
        // 一镜出的窗口不算
        let mut take = shots.clone();
        take[0].one_take = true;
        take[1].one_take = true;
        assert_eq!(jump_cuts(&take), vec![]);
        // 人不一样不算
        let mut two = shots.clone();
        two[1].character_ids.push("char_minister".into());
        assert_eq!(jump_cuts(&two), vec![]);
    }

    #[test]
    fn 剧本里的台词必须写进台词栏() {
        let text = "大臣厉声：“大逆不道！假称诗才惊世！”\n林婉清震惊低语：“这是……唐朝？”";
        let mut notes = Vec::new();
        let mut shot = good_shot("林婉清");
        shot["dialogue"] = serde_json::json!([{"speaker": "林婉清", "line": "这是……唐朝？"}]);
        let shots = parse_shots(&serde_json::json!({"stage": test_stage(), "shots": [shot]}).to_string(), &assets(), 3, &mut notes).unwrap();
        let issues = dialogue_coverage(text, &shots);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert!(issues[0].message.contains("大逆不道") && issues[0].shot_num == 3);
        // 标点、空格不同不算漏
        let mut both = good_shot("林婉清");
        both["dialogue"] = serde_json::json!([{"speaker": "大臣", "line": "大逆不道，假称诗才惊世"}, {"speaker": "林婉清", "line": "这是唐朝"}]);
        let shots = parse_shots(&serde_json::json!({"stage": test_stage(), "shots": [both]}).to_string(), &assets(), 3, &mut notes).unwrap();
        assert_eq!(dialogue_coverage(text, &shots), vec![]);
        // 台词写进声音字段不算
        let mut in_sound = good_shot("林婉清");
        in_sound["dialogue"] = serde_json::json!([]);
        in_sound["segments"][0]["sound"] = serde_json::json!("大臣厉声：大逆不道！假称诗才惊世！");
        let shots = parse_shots(&serde_json::json!({"stage": test_stage(), "shots": [in_sound]}).to_string(), &assets(), 3, &mut notes).unwrap();
        assert_eq!(dialogue_coverage(text, &shots).len(), 2);
    }

    /// 「说话人写错」要挂在台词**所在的那一镜**上（原来挂在这一场第一镜），那一镜的「修复」按钮才改得到它；
    /// 修法按台词和错的说话人一起找到第几句（2026-09-30 EP02 第 10 镜「协议铭牌」→「铭牌（合成音）」）
    #[test]
    fn 说话人写错挂在台词所在那一镜_修法找得到那一句() {
        let text = "大臣厉声：“大逆不道！假称诗才惊世！”\n林婉清震惊低语：“这是……唐朝？”";
        let mut notes = Vec::new();
        let mut a = good_shot("林婉清");
        a["dialogue"] = serde_json::json!([{"speaker": "林婉清", "line": "这是……唐朝？"}]);
        let mut b = good_shot("林婉清");
        b["dialogue"] = serde_json::json!([{"speaker": "林婉清", "line": "这是……唐朝？"}, {"speaker": "林婉清", "line": "大逆不道！假称诗才惊世！"}]);
        let shots = parse_shots(&serde_json::json!({"stage": test_stage(), "shots": [a, b]}).to_string(), &assets(), 3, &mut notes).unwrap();
        let issues = dialogue_coverage(text, &shots);
        let wrong = issues.iter().find(|i| i.message.contains("说话人错了")).expect("没报说话人错");
        assert_eq!(wrong.shot_num, 4, "要挂在台词所在的第 4 镜，不是这一场第一镜：{issues:?}");
        let shot = shots.iter().find(|s| s.shot_num == 4).unwrap();
        // 剧本「大臣厉声：」解析出来的说话人带着「厉声」—— 有角色名就用角色名
        assert_eq!(speaker_fix(shot, &wrong.message, &["林婉清", "大臣"]), Some((1, "大臣".to_string())));
        assert_eq!(speaker_fix(shot, &wrong.message, &[]), Some((1, "大臣厉声".to_string())), "对不上角色名才照抄剧本");
        assert_eq!(speaker_fix(shot, "别的问题", &[]), None);
        // 忽略标点一致的：「铭牌（合成音）」对资产「铭牌合成音」
        let mut plate = shot.clone();
        plate.dialogue_lines = vec![DialogueLine { speaker: "协议铭牌".into(), line: "本机无武器。".into(), ..Default::default() }];
        let msg = "台词「本机无武器。」写成了「协议铭牌」说的，剧本里是「铭牌（合成音）」—— 说话人错了，出片会配错人的口型";
        assert_eq!(speaker_fix(&plate, msg, &["铭牌合成音", "星澜"]), Some((0, "铭牌合成音".to_string())));
    }

    #[test]
    fn 资产清单带上外貌描述和有没有图() {
        let mut list = assets();
        list[0].description = "现代办公室职员，28岁，黑发披肩直发，白色衬衫".into();
        let text = asset_catalog(&list);
        assert!(text.contains("- 林婉清（有图）：现代办公室职员，28岁，黑发披肩直发，白色衬衫"), "{text}");
        assert!(text.contains("- 大臣（无图）"), "{text}");
        let long = "很长".repeat(100);
        list[0].description = long;
        let line = asset_catalog(&list).lines().find(|l| l.contains("林婉清")).unwrap().to_string();
        assert!(line.chars().count() < 110, "描述没截短：{line}");
    }

    /// 换装 / 受损按镜选形象（2026-10-05：《回收师》E03 嫁衣改礼服、E17 / E28 外壳破损，原来只能一镜一镜手点）。
    /// 资产清单列出形象名；镜头表 `assets.looks` 写了就进 `look_overrides`；写错的说出来、用默认形象
    #[test]
    fn 镜头表按剧本选形象() {
        let look = |id: &str, name: &str| LookRef { id: id.into(), name: name.into(), description: format!("{name}的描述"), has_image: false };
        let mut list = assets();
        list[0].looks = vec![look(crate::assets::MAIN_LOOK, ""), look("look_gown", "嫁衣改晚礼服"), look("look_broken", "外壳破损款")];
        let catalog = asset_catalog(&list);
        assert!(catalog.contains("｜形象：主形象、嫁衣改晚礼服、外壳破损款"), "{catalog}");
        assert!(!catalog.lines().find(|l| l.contains("大臣")).unwrap().contains("形象"), "只有一个形象的不列");

        let mut shot = good_shot("林婉清");
        shot["assets"]["looks"] = serde_json::json!({"林婉清": "嫁衣改晚礼服"});
        let mut other = good_shot("林婉清");
        other["assets"]["looks"] = serde_json::json!({"林婉清": "不存在的形象", "大臣": "外壳破损款"});
        let plain = good_shot("林婉清");
        let raw = serde_json::json!({"stage": test_stage(), "shots": [shot, other, plain]}).to_string();
        let mut notes = Vec::new();
        let shots = parse_shots(&raw, &list, 1, &mut notes).unwrap();
        assert_eq!(shots[0].look_overrides.get("char_shen").map(String::as_str), Some("look_gown"));
        assert!(shots[1].look_overrides.is_empty() && shots[2].look_overrides.is_empty());
        assert!(notes.iter().any(|n| n.contains("第 2 镜") && n.contains("不存在的形象") && n.contains("用默认形象")), "{notes:?}");
        assert!(notes.iter().any(|n| n.contains("「大臣」") && n.contains("人物里没有")), "{notes:?}");
        // 换了形象，提示词那边拿到的就是那个形象的描述（`looks::for_shot`）
        let a = crate::looks::for_shot(&list, &shots[0]);
        assert_eq!(a[0].description, "嫁衣改晚礼服的描述");
    }

    #[test]
    fn 词表外的景别运镜留空并说出来_不猜默认值() {
        let mut shot = good_shot("林婉清");
        shot["scale"] = serde_json::json!("中景");
        shot["move"] = serde_json::json!("fly_through");
        let raw = serde_json::json!({"stage": test_stage(), "shots": [shot]}).to_string();
        let mut notes = Vec::new();
        let s = &parse_shots(&raw, &assets(), 1, &mut notes).unwrap()[0];
        assert_eq!(s.scale, "");
        assert_eq!(s.camera_movement, "");
        assert_eq!(s.camera_angle, "eye_level");
        assert_eq!(notes.len(), 2, "{notes:?}");
    }

    /// 用户这份剧本的真实结构：【序幕】【地点】挨着、【第二幕】【第三幕】写在行中间、前面有企划信息。
    #[test]
    fn 有标题就按标题分场_不问模型() {
        let script = "《我在长安写诗词》\n**类型**\n古风\n\n【序幕】\n【现代 · 深夜办公室】\n键盘声急促。 黑屏。\n【第一幕】\n【盛唐 · 金銮殿 · 日】\n鼓声震天。 大殿死寂。 【第二幕】 林婉清缓缓起身。 【第三幕】 殿外风起。 盛唐山河展开。";
        let scenes = split_by_headings(script).unwrap();
        assert_eq!(
            scenes.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["现代 · 深夜办公室", "盛唐 · 金銮殿 · 日", "盛唐 · 金銮殿 · 日（第二幕）", "盛唐 · 金銮殿 · 日（第三幕）"]
        );
        assert_eq!(scenes[0].text, "【序幕】\n【现代 · 深夜办公室】\n键盘声急促。 黑屏。", "企划信息混进来了，或者标记没留在原文里");
        assert_eq!(scenes[2].text, "【第二幕】 林婉清缓缓起身。");
        assert_eq!(scenes[3].text, "【第三幕】 殿外风起。 盛唐山河展开。");
        // 原文一个字不少：各场拼起来就是剧本从第一个标记开始的部分（去掉空白比）
        let joined: String = scenes.iter().map(|s| squash(&s.text)).collect();
        assert_eq!(joined, squash(&script[script.find('【').unwrap()..]));
    }

    #[test]
    fn 标题少于两场交给模型() {
        assert_eq!(split_by_headings("没有任何标题的剧本。"), None);
        assert_eq!(split_by_headings("【办公室】\n林婉清抬头。"), None);
        // 标记后面没有正文的不算一场
        assert_eq!(split_by_headings("【办公室】\n林婉清抬头。\n【尾声】"), None);
    }

    #[test]
    fn 结构标记和地点标题分得开() {
        for t in ["序幕", "尾声", "第一幕", "第十二场", "第3幕"] {
            assert!(is_act_marker(t), "{t}");
        }
        for t in ["现代 · 深夜办公室", "盛唐 · 金銮殿 · 日", "第一次见面的咖啡馆"] {
            assert!(!is_act_marker(t), "{t}");
        }
    }

    /// 2026-09-13 用户实跑：模型把用途写成 `prop`，整场判失败、整集生成失败。
    #[test]
    fn 参考图用途写了表外的词_不让整场失败() {
        let mut shot = good_shot("林婉清");
        shot["ref_slots"] = serde_json::json!([
            {"asset": "手机", "usage": "prop", "control": "外形"},
            {"asset": "林婉清", "usage": "vibe", "control": "气质"},
            {"asset": "林婉清", "usage": " Identity ", "control": "脸"}
        ]);
        let mut notes = Vec::new();
        let s = &parse_shots(&serde_json::json!({"stage": test_stage(), "shots": [shot]}).to_string(), &assets(), 1, &mut notes).unwrap()[0];
        assert_eq!(s.ref_slots.iter().map(|r| (r.asset_id.as_str(), r.usage)).collect::<Vec<_>>(),
            [("prop_phone", RefUsage::Identity), ("char_shen", RefUsage::Identity)]);
        assert_eq!(s.ref_slots.iter().map(|r| r.order).collect::<Vec<_>>(), [1, 2], "丢掉的不占编号");
        assert!(notes.iter().any(|n| n.contains("vibe")), "{notes:?}");
    }

    /// 同一天实跑：`"asset":">手机"`，名字对不上资产库，参考图被丢。
    #[test]
    fn 字符串开头多写的大于号去掉_正文里的不动() {
        let raw = r#"{"shots":[{"duration":8,"sound_effect":">键盘声","assets":{"characters":[">林婉清"],"props":[ ">手机"]},"ref_slots":[{"asset":">手机","usage":"identity","control":"a > b 的\">\"符号"}],"segments":[{"start":0,"end":8,"visual":"x"}]}]}"#;
        let mut notes = Vec::new();
        let s = &parse_shots(raw, &assets(), 1, &mut notes).unwrap()[0];
        assert_eq!(s.sound_effect, "键盘声");
        assert_eq!(s.character_ids, ["char_shen"]);
        assert_eq!(s.prop_ids, ["prop_phone"]);
        assert_eq!(s.ref_slots[0].asset_id, "prop_phone");
        assert_eq!(s.ref_slots[0].control, "a > b 的\">\"符号", "正文里的 > 被误删了");
        assert!(notes.is_empty(), "{notes:?}");
    }

    /// 2026-09-13 实跑的三种错：漏拍、编造、抄进提示词标记。
    #[test]
    fn 原文必须逐字出自剧本_整场每句都要有一镜拍到() {
        let scene = "【第三幕】 殿外风起。 长安街市灯火连绵。 皇帝低声： “此诗，当传万世。” 林婉清目光坚定。";
        let mk = |num: u32, source: &str| {
            let raw = serde_json::json!({"stage": test_stage(), "shots": [said(sourced(good_shot("林婉清"), source), None)]}).to_string();
            let mut s = parse_shots(&raw, &assets(), num, &mut Vec::new()).unwrap().remove(0);
            s.shot_num = num;
            s
        };
        // 引号样式、空白不同不算改原文；【第三幕】是标记不用拍
        let full = [mk(1, "殿外风起。长安街市灯火连绵。"), mk(2, "皇帝低声：'此诗，当传万世。'林婉清目光坚定。")];
        assert_eq!(source_coverage(scene, &full), vec![]);
        // 漏拍：第一句没人引
        let missing = source_coverage(scene, &[mk(2, "长安街市灯火连绵。 皇帝低声： “此诗，当传万世。” 林婉清目光坚定。")]);
        assert_eq!(missing.len(), 1, "{missing:?}");
        assert!(missing[0].message.contains("殿外风起"), "{missing:?}");
        // 编造 / 抄进提示词标记
        let made_up = source_coverage(scene, &[mk(1, "（接上镜）镜头拉远，展现林婉清孤身一人"), mk(2, "【这一场】殿外风起。【上一场的结尾】长安街市灯火连绵。")]);
        assert!(made_up.iter().any(|i| i.shot_num == 1 && i.message.contains("接上镜")), "{made_up:?}");
        assert!(made_up.iter().any(|i| i.shot_num == 2 && i.message.contains("这一场")), "{made_up:?}");
        // 没写原文
        assert!(source_coverage(scene, &[mk(3, "")]).iter().any(|i| i.shot_num == 3 && i.message.contains("没写原文")));
    }

    #[test]
    fn 切句时引号里的标点不切() {
        assert_eq!(
            sentences("大臣厉声： “大逆不道！当斩！” 林婉清：“这是……唐朝？”黑屏。"),
            ["大臣厉声： “大逆不道！当斩！”", " 林婉清：“这是……唐朝？”", "黑屏。"]
        );
    }

    #[test]
    fn 说话人写简称_本镜只有一个对得上就换成全名() {
        let mut list = assets();
        list.push(AssetRef { id: "char_shen_tang".into(), kind: AssetKind::Character, name: "林婉清（盛唐）".into(), has_image: true, description: String::new(), look_id: String::new(), looks: Vec::new(), h3_voice: String::new() });
        list.push(AssetRef { id: "char_shen_now".into(), kind: AssetKind::Character, name: "林婉清（现代）".into(), has_image: true, description: String::new(), look_id: String::new(), looks: Vec::new(), h3_voice: String::new() });
        let mut shot = good_shot("林婉清");
        shot["assets"]["characters"] = serde_json::json!(["林婉清（盛唐）"]);
        shot["ref_slots"] = serde_json::json!([]);
        let s = parse_shots(&serde_json::json!({"stage": test_stage(), "shots": [shot.clone()]}).to_string(), &list, 1, &mut Vec::new()).unwrap().remove(0);
        assert_eq!(s.dialogue_lines[0].speaker, "林婉清（盛唐）");
        assert_eq!(validate_shots(&[s], &list), vec![]);
        // 两个造型都在这一镜：不猜
        shot["assets"]["characters"] = serde_json::json!(["林婉清（盛唐）", "林婉清（现代）"]);
        let s = parse_shots(&serde_json::json!({"stage": test_stage(), "shots": [shot.clone()]}).to_string(), &list, 1, &mut Vec::new()).unwrap().remove(0);
        assert_eq!(s.dialogue_lines[0].speaker, "林婉清");
        // 只是前缀相同的另一个人不算
        shot["assets"]["characters"] = serde_json::json!(["林婉清（盛唐）"]);
        shot["dialogue"] = serde_json::json!([{"speaker": "沈昭", "line": "我还能撑。"}]);
        let s = parse_shots(&serde_json::json!({"stage": test_stage(), "shots": [shot]}).to_string(), &list, 1, &mut Vec::new()).unwrap().remove(0);
        assert_eq!(s.dialogue_lines[0].speaker, "沈昭");
    }

    #[test]
    fn 场景写成某处的一部分_资产库里只有一个对得上就归过去() {
        let mut list = assets();
        list.push(AssetRef { id: "scene_palace".into(), kind: AssetKind::Scene, name: "金銮殿".into(), has_image: true, description: String::new(), look_id: String::new(), looks: Vec::new(), h3_voice: String::new() });
        let parse = |scene: &str, list: &[AssetRef]| {
            let mut shot = good_shot("林婉清");
            shot["assets"]["scene"] = serde_json::json!(scene);
            let mut notes = Vec::new();
            let s = parse_shots(&serde_json::json!({"stage": test_stage(), "shots": [shot]}).to_string(), list, 1, &mut notes).unwrap().remove(0);
            (s.scene_id, notes)
        };
        let (id, notes) = parse("金銮殿门口", &list);
        assert_eq!(id.as_deref(), Some("scene_palace"));
        assert!(notes[0].contains("按资产「金銮殿」算"), "{notes:?}");
        assert_eq!(parse("金銮殿（局部）", &list).0.as_deref(), Some("scene_palace"));
        assert_eq!(parse("长安街市", &list).0, None, "对不上的不猜");
        // 两个场景都被包含：不猜
        list.push(AssetRef { id: "scene_gate".into(), kind: AssetKind::Scene, name: "门口".into(), has_image: true, description: String::new(), look_id: String::new(), looks: Vec::new(), h3_voice: String::new() });
        assert_eq!(parse("金銮殿门口", &list).0, None);
    }

    /// 2026-09-13 实跑：文字字段里写了整数 `2400018756`，整场判失败。
    #[test]
    fn 字段类型写错_只规整类型_不让整场失败() {
        let mut shot = good_shot("林婉清");
        shot["duration"] = serde_json::json!("8秒");
        shot["camera_note"] = serde_json::json!(2400018756u64);
        shot["lighting"] = serde_json::Value::Null;
        shot["scene_items"][0]["detail"] = serde_json::json!(15);
        shot["segments"][0]["start"] = serde_json::json!("0");
        shot["segments"][0]["title"] = serde_json::json!(1);
        shot["assets"]["props"] = serde_json::json!(["手机", 42]);
        shot["dialogue"][0]["off_screen"] = serde_json::json!("true");
        let mut notes = Vec::new();
        let s = parse_shots(&serde_json::json!({"stage": test_stage(), "shots": [shot]}).to_string(), &assets(), 1, &mut notes).unwrap().remove(0);
        assert_eq!(s.duration, 8.0);
        assert_eq!(s.camera_note, "2400018756");
        assert_eq!(s.lighting, "");
        assert_eq!(s.scene_items[0].detail, "15");
        assert_eq!((s.segments[0].start, s.segments[0].title.as_str()), (0.0, "1"));
        assert_eq!(s.prop_ids, ["prop_phone"]);
        assert!(notes.iter().any(|n| n.contains("42")), "{notes:?}");
        assert!(s.dialogue_lines[0].off_screen);
        // 时长写成完全不是数的文字：按没写处理，交给校验报
        let mut bad = good_shot("林婉清");
        bad["duration"] = serde_json::json!("很长");
        let s = parse_shots(&serde_json::json!({"stage": test_stage(), "shots": [bad]}).to_string(), &assets(), 1, &mut Vec::new()).unwrap().remove(0);
        assert_eq!(s.duration, 0.0);
        assert!(validate_shots(&[s], &assets()).iter().any(|i| i.field == "duration"));
    }

    /// 2026-09-13 实跑原样：场名、【这一场】抄进了原文；一镜引了两处不相邻的原句。
    #[test]
    fn 原文里的场名和标记去掉_不相邻的原句也算逐字() {
        let scene_name = "盛唐 · 金銮殿 · 日（第三幕）";
        let scene = "【第三幕】 殿外风起。 长安街市灯火连绵。 皇帝低声： “此诗，当传万世。” 林婉清目光坚定。 镜头推远。 盛唐山河展开。";
        let mk = |num: u32, source: &str| {
            let mut s = parse_shots(&serde_json::json!({"stage": test_stage(), "shots": [sourced(good_shot("林婉清"), source)]}).to_string(), &assets(), num, &mut Vec::new()).unwrap().remove(0);
            s.dialogue_lines.clear();
            s
        };
        let mut shots = vec![
            mk(1, "盛唐 · 金銮殿 · 日（第三幕）\n【这一场】\n林婉清目光坚定。"),
            mk(2, "镜头推远。 盛唐山河展开。\n殿外风起。 长安街市灯火连绵。"),
            mk(3, "盛唐 · 金銮殿 · 日。皇帝低声： “此诗，当传万世。”"),
        ];
        clean_sources(&mut shots, scene_name);
        assert_eq!(shots[0].source, "林婉清目光坚定。");
        assert_eq!(shots[2].source, "皇帝低声： “此诗，当传万世。”");
        assert_eq!(source_coverage(scene, &shots), vec![]);
        // 编的句子夹在原句中间，去不掉，照样报
        let mut fake = vec![mk(4, "殿外风起。群臣跪地高呼万岁。")];
        clean_sources(&mut fake, scene_name);
        assert!(source_coverage(scene, &fake).iter().any(|i| i.message.contains("群臣跪地")));
    }

    /// 台词开头的括号**必须**拆走 —— 留着会被 H3 逐字念出来（静默失败）。
    fn 镜(num: u32, secs: f32, duty: &str, 有台词: bool) -> StoryboardShot {
        let mut s: StoryboardShot = serde_json::from_value(serde_json::json!({
            "id": format!("s{num}"), "shot_num": num, "scale": "", "camera_movement": "",
            "dialogue": "", "prompt": "", "duration": secs, "status": "pending"
        })).unwrap();
        s.duty = duty.into();
        if 有台词 {
            s.dialogue_lines = vec![DialogueLine { speaker: "甲".into(), line: "走。".into(), ..Default::default() }];
        }
        s
    }

    /// **连续两镜都没台词又都短 —— 多半本来是一镜。**
    ///
    /// E04 的真实模式：十二镜只有五镜有台词，稳定的「承接过场 + 5 秒台词」对。
    /// 用户原话：「拆分成两到三镜反而觉得空洞，而且还会造成一致性问题」。
    #[test]
    fn 连续两个无台词短镜要提示合并() {
        let out = shot_density(&[镜(1, 6.0, "手机亮起", false), 镜(2, 5.0, "他开口", true)]);
        assert!(out.iter().all(|i| i.field != "duration"), "只有一镜没台词，不该提示：{out:?}");

        let out = shot_density(&[镜(1, 6.0, "手机亮起", false), 镜(2, 5.0, "他转头", false)]);
        assert_eq!(out.len(), 1, "{out:?}");
        assert!(out[0].message.contains("第 1 和第 2 镜"), "{:?}", out[0]);
    }

    /// **长镜不提示。** 一个 12 秒的无台词空镜可能是有意的（气氛、蒙太奇），
    /// 提示了只会变成噪音。判据是「短」+「没台词」两条一起。
    #[test]
    fn 长的无台词镜不提示() {
        assert!(shot_density(&[镜(1, 12.0, "空镜", false), 镜(2, 10.0, "空镜", false)])
            .iter()
            .all(|i| i.field != "duration"));
    }

    /// duty 里写「承接」「落回」的无台词镜单独报 —— 那一条最有把握。
    #[test]
    fn 承接类职责的空镜单独报() {
        let out = shot_density(&[镜(1, 6.0, "承接上一场的沉浸状态，落回杜甫草堂", false)]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].field, "duty");
        assert!(out[0].message.contains("不用单独占一镜"), "{:?}", out[0]);
    }

    /// **有台词的镜头一律不报**，哪怕 duty 里写了「承接」——
    /// 它有台词就有存在的理由，报了是误伤。
    #[test]
    fn 有台词的镜头一律不报() {
        assert!(shot_density(&[镜(1, 5.0, "承接上一场", true), 镜(2, 5.0, "落回李白", true)]).is_empty());
    }

    #[test]
    fn 台词开头的括号拆成语气不被念出来() {
        assert_eq!(split_leading_tone("（咬牙）所以用户满意度是零。"), ("所以用户满意度是零。".into(), "咬牙".into()));
        // 半角括号、括号后带冒号，都要认
        assert_eq!(split_leading_tone("(压低声音)：这茶，要等。"), ("这茶，要等。".into(), "压低声音".into()));
        // 没有括号：原样返回，语气为空
        assert_eq!(split_leading_tone("半个？"), ("半个？".into(), String::new()));
    }

    /// 整句都是舞台说明时**不拆** —— 那是模型写错了，`source_coverage` 要能报出来。
    /// 拆掉等于把台词变成空串，错误被藏起来。
    #[test]
    fn 整句都是括号的不拆_留给检查报错() {
        assert_eq!(split_leading_tone("（画外音，激昂）"), ("（画外音，激昂）".into(), String::new()));
        assert_eq!(split_leading_tone("（沉默）"), ("（沉默）".into(), String::new()));
    }

    /// 结尾的括号**不能**拆 —— 那可能是台词的一部分。
    #[test]
    fn 结尾的括号不拆() {
        assert_eq!(split_leading_tone("他说了一句……（大概吧）"), ("他说了一句……（大概吧）".into(), String::new()));
    }

    /// **这两个字段必须一路带到 `DialogueLine`。**
    ///
    /// 它们在数据结构里躺了很久却没人填也没人读，成片配音因此一直是
    /// 「性别 + 年龄」的播音腔（2026-09-18）。这条测试盯着接上的那一头。
    #[test]
    fn 语气和音色从镜头表带进台词行() {
        let mut shot = good_shot("林婉清");
        shot["dialogue"] = serde_json::json!([{
            "speaker": "林婉清", "line": "我还能撑。",
            "emotion": "压低声音，几乎是气音", "voice": "沙哑，语速偏慢"
        }]);
        let out = parse_shots(&serde_json::json!({"stage": test_stage(), "shots": [shot]}).to_string(), &assets(), 1, &mut Vec::new()).unwrap();
        let l = &out[0].dialogue_lines[0];
        assert_eq!(l.emotion, "压低声音，几乎是气音");
        assert_eq!(l.voice, "沙哑，语速偏慢");
    }

    /// 模型自己填了 `emotion` 时，台词开头那个括号只是兜底，**不能把它顶掉**。
    #[test]
    fn 模型填了语气就用模型的_括号只是兜底() {
        let mut shot = good_shot("林婉清");
        shot["dialogue"] = serde_json::json!([{"speaker": "林婉清", "line": "（咬牙）我还能撑。", "emotion": "带笑"}]);
        let out = parse_shots(&serde_json::json!({"stage": test_stage(), "shots": [shot]}).to_string(), &assets(), 1, &mut Vec::new()).unwrap();
        let l = &out[0].dialogue_lines[0];
        assert_eq!(l.emotion, "带笑", "模型填的被括号顶掉了");
        assert_eq!(l.line, "我还能撑。", "括号还是得从台词里拆走，否则会被念出来");
    }

    #[test]
    fn 台词栏里的舞台说明不是剧本原话_要报() {
        let scene = "抬头，高声吟诵： “君不见黄河之水天上来——”";
        let mut s = parse_shots(&serde_json::json!({"stage": test_stage(), "shots": [sourced(good_shot("林婉清"), "抬头，高声吟诵： “君不见黄河之水天上来——”")]}).to_string(), &assets(), 1, &mut Vec::new()).unwrap().remove(0);
        s.dialogue_lines = vec![
            DialogueLine { speaker: "林婉清".into(), line: "君不见黄河之水天上来——".into(), ..Default::default() },
            DialogueLine { speaker: "林婉清".into(), line: "（画外音，激昂）".into(), off_screen: true, ..Default::default() },
        ];
        let issues = source_coverage(scene, &[s]);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert!(issues[0].message.contains("画外音，激昂"));
    }

    #[test]
    fn 名字带括号注释_去掉注释能对上就算_参考图的资产补进出场() {
        let mut list = assets();
        list.push(AssetRef { id: "char_emperor".into(), kind: AssetKind::Character, name: "皇帝".into(), has_image: true, description: String::new(), look_id: String::new(), looks: Vec::new(), h3_voice: String::new() });
        list.push(AssetRef { id: "scene_palace".into(), kind: AssetKind::Scene, name: "金銮殿".into(), has_image: true, description: String::new(), look_id: String::new(), looks: Vec::new(), h3_voice: String::new() });
        let mut shot = good_shot("皇帝（苏菲饰）");
        shot["assets"] = serde_json::json!({"characters": ["皇帝（苏菲饰）"], "scene": "", "props": []});
        shot["ref_slots"] = serde_json::json!([
            {"asset": "皇帝（苏菲饰）", "usage": "identity", "control": "脸"},
            {"asset": "金銮殿", "usage": "geography", "control": "龙椅位置"}
        ]);
        let mut notes = Vec::new();
        let s = parse_shots(&serde_json::json!({"stage": test_stage(), "shots": [shot]}).to_string(), &list, 1, &mut notes).unwrap().remove(0);
        assert_eq!(s.character_ids, ["char_emperor"]);
        assert_eq!(s.scene_id.as_deref(), Some("scene_palace"), "参考图挂了场景，出场资产里没补上");
        assert_eq!(s.ref_slots.iter().map(|r| r.asset_id.as_str()).collect::<Vec<_>>(), ["char_emperor", "scene_palace"]);
        assert_eq!(s.dialogue_lines[0].speaker, "皇帝");
        assert!(notes.iter().any(|n| n.contains("皇帝（苏菲饰）") && n.contains("按资产「皇帝」算")), "{notes:?}");
        assert_eq!(validate_shots(&[s], &list), vec![]);
        // 资产本身带括号的照常精确匹配；注释去掉也对不上的不猜
        assert_eq!(strip_note("林婉清（盛唐）"), Some("林婉清"));
        assert!(lookup(&list, None, "太监（李公公饰）").is_none());
        assert_eq!(strip_note("（旁白）"), None);
    }

    /// 用户列的五类反复出现的问题，原句照搬。
    #[test]
    fn 代码修掉反复出现的固定写法() {
        let mut list = assets();
        list.push(AssetRef { id: "char_tang".into(), kind: AssetKind::Character, name: "林婉清（盛唐）".into(), has_image: true, description: String::new(), look_id: String::new(), looks: Vec::new(), h3_voice: String::new() });
        list.push(AssetRef { id: "char_now".into(), kind: AssetKind::Character, name: "林婉清（现代）".into(), has_image: true, description: String::new(), look_id: String::new(), looks: Vec::new(), h3_voice: String::new() });
        let mk = |num: u32, chars: Vec<&str>| {
            let mut s = parse_shots(&serde_json::json!({"stage": test_stage(), "shots": [good_shot("林婉清")]}).to_string(), &list, num, &mut Vec::new()).unwrap().remove(0);
            s.character_ids = chars.into_iter().map(String::from).collect();
            s.character_id.clear();
            s.dialogue_lines.clear();
            s
        };
        // 第 3 镜：夹英文
        let mut s3 = mk(3, vec!["char_tang"]);
        s3.segments[1].camera = "镜头跟随她的视线左右轻微摇摄（Pan），展示两侧百官".into();
        s3.lighting = "侧逆光勾勒轮廓（Rim Light），Rim Light 很亮".into();
        // 第 4 镜：二选一 + 台词写进声音 + 说话人简称不在出场人物里
        let mut s4 = mk(4, vec!["char_minister"]);
        s4.segments[0].visual = "镜头焦点在前景手指和背景林婉清之间轻微切换（或保持林婉清清晰）。林婉清抬头".into();
        s4.segments[0].sound = "大臣厉声：'大逆不道！假称诗才惊世，欺君罔上，当斩！'".into();
        s4.segments[1].visual = "她眼神闪过慌乱，或者定格在震惊。随后低头".into();
        s4.segments[1].sound = "林婉清震惊低语：“这是……唐朝？”".into();
        s4.dialogue_lines = vec![
            DialogueLine { speaker: "大臣".into(), line: "大逆不道！假称诗才惊世，欺君罔上，当斩！".into(), ..Default::default() },
            DialogueLine { speaker: "林婉清".into(), line: "这是……唐朝？".into(), ..Default::default() },
        ];
        // 第 10 镜：一镜内切镜
        let mut s10 = mk(10, vec!["char_tang"]);
        s10.segments[1].visual = "镜头切至侧前方，扫过两侧大臣".into();
        let mut shots = vec![s3, s4, s10];
        let n = repair_shots(&mut shots, &list);
        assert!(n >= 6, "{n}");
        assert_eq!(shots[0].segments[1].camera, "镜头跟随她的视线左右轻微摇摄，展示两侧百官");
        assert_eq!(shots[0].lighting, "侧逆光勾勒轮廓，轮廓光 很亮");
        assert_eq!(shots[1].segments[0].visual, "镜头焦点在前景手指和背景林婉清之间轻微切换。林婉清抬头");
        assert_eq!(shots[1].segments[1].visual, "她眼神闪过慌乱。随后低头");
        assert_eq!(shots[1].segments[0].sound, "大臣厉声说话");
        assert_eq!(shots[1].segments[1].sound, "林婉清震惊低语说话");
        assert_eq!(shots[1].dialogue_lines[1].speaker, "林婉清（盛唐）", "本场用的是盛唐造型");
        assert!(shots[1].character_ids.contains(&"char_tang".to_string()));
        assert_eq!(shots[2].segments[1].visual, "镜头转向侧前方，扫过两侧大臣");
        // 修完这几类都不再报
        let issues = validate_shots(&shots, &list);
        assert!(!issues.iter().any(|i| i.message.contains("英文") || i.message.contains("（或") || i.message.contains("又写了一遍") || i.message.contains("切至") || i.message.contains("不是这一镜的出场人物")), "{issues:?}");
        // 写给自己的说明（实跑第 14 镜光影原句）
        let mut meta = vec![mk(14, vec!["char_tang"])];
        meta[0].lighting = "从室内昏暗过渡到室外盛唐白昼的明亮阳光，或夜晚灯火通明的辉煌（根据前文'灯火连绵'调整为夜景或晨曦）".into();
        repair_shots(&mut meta, &list);
        assert_eq!(meta[0].lighting, "从室内昏暗过渡到室外盛唐白昼的明亮阳光");
        // 切黑不是切镜，不动
        let mut black = vec![mk(1, vec!["char_tang"])];
        black[0].segments[1].visual = "向前倒下，切黑".into();
        repair_shots(&mut black, &list);
        assert_eq!(black[0].segments[1].visual, "向前倒下，切黑");
    }

    #[test]
    fn 简称对不上唯一造型时不猜() {
        let mut list = assets();
        list.push(AssetRef { id: "a".into(), kind: AssetKind::Character, name: "林婉清（盛唐）".into(), has_image: true, description: String::new(), look_id: String::new(), looks: Vec::new(), h3_voice: String::new() });
        list.push(AssetRef { id: "b".into(), kind: AssetKind::Character, name: "林婉清（现代）".into(), has_image: true, description: String::new(), look_id: String::new(), looks: Vec::new(), h3_voice: String::new() });
        let mut s = parse_shots(&serde_json::json!({"stage": test_stage(), "shots": [good_shot("林婉清")]}).to_string(), &list, 1, &mut Vec::new()).unwrap().remove(0);
        s.character_ids = vec!["char_minister".into()];
        s.character_id.clear();
        s.dialogue_lines = vec![DialogueLine { speaker: "林婉清".into(), line: "我还能撑。".into(), ..Default::default() }];
        let mut shots = vec![s];
        repair_shots(&mut shots, &list);
        assert_eq!(shots[0].dialogue_lines[0].speaker, "林婉清");
        assert_eq!(shots[0].character_ids, ["char_minister"]);
    }

    /// 旧表里第 7、8 镜的原文原样。
    #[test]
    fn 原文里编的句子和标记删掉_漏拍的不补() {
        let scene = "【第一幕】【盛唐 · 金銮殿 · 日】\n鼓声震天。 刀光映入她瞳孔。 大殿死寂。 【第二幕】 林婉清缓缓起身。 呼吸渐稳。 殿外风起。";
        let mk = |num: u32, source: &str| {
            let mut s = parse_shots(&serde_json::json!({"stage": test_stage(), "shots": [said(sourced(good_shot("林婉清"), "x"), None)]}).to_string(), &assets(), num, &mut Vec::new()).unwrap().remove(0);
            s.source = source.into();
            s
        };
        let mut shots = vec![
            mk(7, "大殿死寂。（接上镜）镜头拉远，展现林婉清孤身一人跪在空旷大殿中央的渺小。"),
            mk(8, "盛唐 · 金銮殿 · 日（第二幕）\n【这一场】林婉清缓缓起身。 呼吸渐稳。\n【上一场的结尾】镜头从林婉清头顶迅速升高并向后拉远"),
        ];
        let n = repair_sources(&mut shots, "盛唐 · 金銮殿 · 日（第二幕）", scene);
        assert_eq!(n, 2);
        assert_eq!(shots[0].source, "大殿死寂。");
        assert_eq!(shots[1].source, "林婉清缓缓起身。 呼吸渐稳。");
        let issues = source_coverage(scene, &shots);
        assert!(!issues.iter().any(|i| i.message.contains("不是逐字出自")), "{issues:?}");
        assert!(issues.iter().any(|i| i.message.contains("殿外风起")), "漏拍的照样要报：{issues:?}");
    }

    type SeenCall = (crate::prompts::PromptSlot, Vec<(String, String)>);

    /// 假的 LM：按调用顺序依次吐出预设的回复，同时记下每次收到的变量。
    struct FakeLm {
        replies: Mutex<Vec<String>>,
        seen: Mutex<Vec<SeenCall>>,
    }

    impl FakeLm {
        fn new(replies: Vec<String>) -> Self {
            Self { replies: Mutex::new(replies.into_iter().rev().collect()), seen: Mutex::new(Vec::new()) }
        }
        async fn call(&self, slot: crate::prompts::PromptSlot, vars: Vec<(String, String)>) -> Result<String> {
            self.seen.lock().unwrap().push((slot, vars));
            self.replies.lock().unwrap().pop().ok_or_else(|| Error::runtime("假 LM 没有更多回复"))
        }
    }

    /// 一镜出的场：换槽位、镜数时长由代码给，写多了要重来，时长不对按比例拉，镜内切镜不报。
    #[tokio::test]
    async fn 一镜出_镜数时长由代码定_镜内可以切() {
        let whole = "键盘声急促。林婉清伏案加班。 手机提示音不断。她抬头，视线模糊——黑屏。";
        // 第一次：模型自己拆成了两镜 —— 要被打回去
        let split = serde_json::json!({"stage": test_stage(), "shots": [
            said(sourced(good_shot("林婉清"), "键盘声急促。林婉清伏案加班。"), None),
            said(sourced(good_shot("林婉清"), "手机提示音不断。她抬头，视线模糊——黑屏。"), None)
        ]}).to_string();
        // 第二次：一镜 8 秒（时长不对），镜内写了切镜
        let mut one = said(sourced(good_shot("林婉清"), whole), None);
        one["segments"][1]["visual"] = serde_json::json!("硬切到手机特写，屏幕一直亮，镜头切至她模糊的脸，黑屏");
        let one = serde_json::json!({"stage": test_stage(), "shots": [one]}).to_string();
        let mut palace_shot = said(sourced(good_shot("大臣"), "鼓声震天。林婉清猛然睁眼，已跪在大殿中央。大臣厉声：大逆不道！"), Some("大逆不道！"));
        palace_shot["assets"]["characters"] = serde_json::json!(["林婉清", "大臣"]);
        let palace = serde_json::json!({"stage": test_stage(), "shots": [palace_shot]}).to_string();
        let lm = FakeLm::new(vec![split, one, palace]);
        // 只有第一场一镜出，1 镜 × 15 秒
        let take = |no: u32, _: &SceneChunk, _: usize| (no == 1).then_some((1, 15.0));
        let (shots, result) = generate(SCRIPT, &assets(), |s, v| lm.call(s, v), |_, _, _| {}, &take, 0).await.unwrap();

        use crate::prompts::PromptSlot;
        let seen = lm.seen.lock().unwrap();
        let var = |i: usize, k: &str| seen[i].1.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone()).unwrap_or_default();
        assert_eq!(seen.iter().map(|(s, _)| *s).collect::<Vec<_>>(), [PromptSlot::ShotTableOneTake, PromptSlot::ShotTableOneTake, PromptSlot::ShotTable]);
        assert_eq!((var(0, "SHOT_COUNT"), var(0, "SHOT_SECS"), var(0, "TOTAL_SECS")), ("1".into(), "15".into(), "15".into()));
        assert!(var(1, "FEEDBACK").contains("写成了 2 镜"), "自己拆镜要被打回：{}", var(1, "FEEDBACK"));
        assert!(!var(2, "MIN_SECS").is_empty() && var(2, "SHOT_COUNT").is_empty(), "普通场照旧");

        assert_eq!(shots.len(), 2);
        let s = &shots[0];
        assert!(s.one_take && !shots[1].one_take);
        assert_eq!(s.duration, 15.0, "时长按给定的");
        assert_eq!((s.segments[0].end, s.segments[1].end), (7.5, 15.0), "按比例拉长，节奏不变");
        assert_eq!(result.issues, vec![], "一镜出的镜内切镜不报问题");
        assert!(s.segments[1].visual.contains("镜头切至她模糊的脸"), "修写法不能把一镜出的切镜改成「转向」：{}", s.segments[1].visual);
    }

    #[tokio::test]
    async fn 整集生成_镜号跨场连续() {
        // 第二镜换成特写：两镜同一个人、景别机位都一样会被报跳切（见 `jump_cuts`）
        let mut close = said(sourced(good_shot("林婉清"), "手机提示音不断。她抬头，视线模糊——黑屏。"), None);
        close["scale"] = serde_json::json!("close_up");
        let office = serde_json::json!({"stage": test_stage(), "shots": [
            said(sourced(good_shot("林婉清"), "键盘声急促。林婉清伏案加班。"), None),
            close
        ]}).to_string();
        let mut palace_shot = said(sourced(good_shot("大臣"), "鼓声震天。林婉清猛然睁眼，已跪在大殿中央。大臣厉声：大逆不道！"), Some("大逆不道！"));
        palace_shot["assets"]["characters"] = serde_json::json!(["林婉清", "大臣"]);
        let palace = serde_json::json!({"stage": test_stage(), "shots": [palace_shot]}).to_string();
        // 剧本有标题：不问模型分场，第一次调用就是第一场的镜头表
        let lm = FakeLm::new(vec![office, palace]);
        let (shots, result) = generate(SCRIPT, &assets(), |s, v| lm.call(s, v), |_, _, _| {}, &|_, _, _| None, 0).await.unwrap();
        assert_eq!(shots.iter().map(|s| s.shot_num).collect::<Vec<_>>(), [1, 2, 3]);
        assert_eq!(shots.iter().map(|s| (s.scene_no, s.scene_name.as_str())).collect::<Vec<_>>(),
            [(1, "现代 · 深夜办公室"), (1, "现代 · 深夜办公室"), (2, "盛唐 · 金銮殿 · 日")]);
        assert_eq!((result.scenes, result.shots), (2, 3));
        assert_eq!(result.issues, vec![]);
        // 第二场的「上一场结尾」是第一场最后一镜最后一段的画面
        let seen = lm.seen.lock().unwrap();
        assert!(seen.iter().all(|(slot, _)| *slot == crate::prompts::PromptSlot::ShotTable), "有标题还去问模型分场");
        // **上一场的结尾要带空间和光线**，不只是最后一段画面 ——
        // 只给一句画面的话模型看不出「这个地方还在」，于是每场开头都要
        // 重新立一遍空间，多出一堆没台词的过场镜（2026-09-19 用户指出）
        let prev = seen[1].1.iter().find(|(k, _)| k == "PREV").unwrap();
        assert!(prev.1.contains("地点：现代 · 深夜办公室"), "{}", prev.1);
        assert!(prev.1.contains("光线：") && prev.1.contains("冷蓝光"), "{}", prev.1);
        assert!(prev.1.contains("画面停在：向前倒下，黑屏"), "{}", prev.1);
        // 固定布景也带给下一场：同一个地点再出现时门、灯、人不挪位置（EP01 拆机间第 2 / 3 场两人对调过）
        assert!(prev.1.contains("那一场的固定布景：方向基准：从办公室门口往里看"), "{}", prev.1);
    }

    #[tokio::test]
    async fn 有问题就把问题反馈给模型重试一次_取问题更少的那份() {
        let whole = plain(&format!("{OFFICE}\n【盛唐 · 金銮殿 · 日】\n鼓声震天。林婉清猛然睁眼，已跪在大殿中央。\n大臣厉声：大逆不道！"));
        let bad = serde_json::json!({"stage": test_stage(), "shots": [said(sourced(good_shot("皇帝"), &whole), Some("大逆不道！"))]}).to_string(); // 说话人不在镜头里
        // **好的那份，说话人要和剧本一致。** 剧本里这句是「大臣厉声：」说的，
        // 夹具原来写成林婉清 —— 2026-09-19 给覆盖检查加上说话人比对之后，
        // 它被正确地报成「说话人错了」。那是夹具旧了，不是检查错了。
        let mut minister = said(sourced(good_shot("大臣"), &whole), Some("大逆不道！"));
        minister["assets"]["characters"] = serde_json::json!(["林婉清", "大臣"]);
        let good = serde_json::json!({"stage": test_stage(), "shots": [minister]}).to_string();
        let one_scene = serde_json::json!({"scenes": [{"name": "办公室", "text": whole}]}).to_string();
        let lm = FakeLm::new(vec![one_scene, bad, good]);
        let (shots, result) = generate(&plain(SCRIPT), &assets(), |s, v| lm.call(s, v), |_, _, _| {}, &|_, _, _| None, 0).await.unwrap();
        assert_eq!(result.issues, vec![], "取了有问题的那份");
        assert_eq!(shots[0].dialogue_lines[0].speaker, "大臣");
        let seen = lm.seen.lock().unwrap();
        let feedback = &seen[2].1.iter().find(|(k, _)| k == "FEEDBACK").unwrap().1;
        assert!(feedback.contains("皇帝") && feedback.contains("必须改掉"), "问题没反馈给模型：{feedback}");
    }

    /// 台词写空（或写进声音）要当成问题反馈给模型重写 —— 2026-09-13 实跑两个模型都这么干过。
    #[tokio::test]
    async fn 台词没写进台词栏也要反馈重试() {
        let script = "【办公室】\n林婉清抬头：“我还能撑。”";
        let scene = serde_json::json!({"scenes": [{"name": "办公室", "text": script}]}).to_string();
        let mut empty = sourced(good_shot("林婉清"), "林婉清抬头：“我还能撑。”");
        empty["dialogue"] = serde_json::json!([]);
        empty["segments"][0]["sound"] = serde_json::json!("林婉清：我还能撑。");
        let first = serde_json::json!({"stage": test_stage(), "shots": [empty]}).to_string();
        let second = serde_json::json!({"stage": test_stage(), "shots": [sourced(good_shot("林婉清"), "林婉清抬头：“我还能撑。”")]}).to_string();
        let lm = FakeLm::new(vec![scene, first, second]);
        let (shots, result) = generate(script, &assets(), |s, v| lm.call(s, v), |_, _, _| {}, &|_, _, _| None, 0).await.unwrap();
        assert_eq!(result.issues, vec![], "{:?}", result.issues);
        assert_eq!(shots[0].dialogue_lines.len(), 1, "用了台词写空的那份");
        let seen = lm.seen.lock().unwrap();
        assert_eq!(seen.len(), 3, "台词写空时没有重试");
        let feedback = &seen[2].1.iter().find(|(k, _)| k == "FEEDBACK").unwrap().1;
        assert!(feedback.contains("我还能撑"), "{feedback}");
    }

    #[tokio::test]
    async fn 重试后更差就留第一份_仍有问题要报出来() {
        let whole = plain(&format!("{OFFICE}\n【盛唐 · 金銮殿 · 日】\n鼓声震天。林婉清猛然睁眼，已跪在大殿中央。\n大臣厉声：大逆不道！"));
        let mut worse = said(sourced(good_shot("皇帝"), &whole), Some("大逆不道！"));
        worse["duration"] = serde_json::json!(30);
        let first = serde_json::json!({"stage": test_stage(), "shots": [said(sourced(good_shot("皇帝"), &whole), Some("大逆不道！"))]}).to_string();
        let second = serde_json::json!({"stage": test_stage(), "shots": [worse]}).to_string();
        let one_scene = serde_json::json!({"scenes": [{"name": "办公室", "text": whole}]}).to_string();
        let lm = FakeLm::new(vec![one_scene, first, second]);
        let (shots, result) = generate(&plain(SCRIPT), &assets(), |s, v| lm.call(s, v), |_, _, _| {}, &|_, _, _| None, 0).await.unwrap();
        assert_eq!(shots[0].duration, 8.0, "留了更差的那份");
        // 两条都是真问题：说话人不在镜头里（皇帝不在出场人物），
        // 而且剧本里这句是「大臣厉声：」说的 —— 后一条是 2026-09-19
        // 给覆盖检查加上说话人比对之后才查得出来的
        assert_eq!(result.issues.len(), 2, "{:?}", result.issues);
        assert!(result.issues.iter().any(|i| i.message.contains("说话人错了")), "{:?}", result.issues);
        assert_eq!(result.issues[0].field, "dialogue");
    }

    #[tokio::test]
    async fn 分场核对两次都不过就报错_不退回兜底切分() {
        let bad = serde_json::json!({"scenes": [{"name": "a", "text": "编出来的内容"}]}).to_string();
        let lm = FakeLm::new(vec![bad.clone(), bad]);
        let err = generate(&plain(SCRIPT), &assets(), |s, v| lm.call(s, v), |_, _, _| {}, &|_, _, _| None, 0).await.unwrap_err();
        assert!(err.to_string().contains("分场没通过核对"), "{err}");
        let seen = lm.seen.lock().unwrap();
        assert_eq!(seen.len(), 2, "应当正好重试一次");
        let feedback = &seen[1].1.iter().find(|(k, _)| k == "FEEDBACK").unwrap().1;
        assert!(feedback.contains("逐字"), "{feedback}");
    }
}
