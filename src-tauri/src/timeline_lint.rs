//! 时间线（镜头表分段）里两类「H3 会照着画出错」的写法。都只提示、不挡出片（`advice`）。
//!
//! 规则借自 Toonflow（MIT，`packages/skills/workflow/SKILL.md`，HBAI-Ltd/Toonflow-app）：
//! - **A6.6 表演通道适配**：「只写本镜头实际能看见的表演通道」—— 背影只写重心、肩颈、呼吸。
//! - **A2.1 源剧情账本**：时间线里改变人物 / 位置的句子要能对回原文，对不上的是 AI 新增剧情。
//!
//! 我们自己的实例：
//! - 2026-10-10 EP04 第 7 镜：镜头说明「从老秦肩后看被按墙的星澜」，第 2 段却写「挺起胸口的工牌」「侧脸转向镜头可见嘴部」——
//!   机位在他背后，这些看不见；H3 另画了一个正脸的老秦来演，6.5 秒起两个老秦同框到结尾。
//! - 2026-10-09 EP03 第 10 镜：片尾多出一个没人认识的人。

use crate::shot_table::ShotSegment;

/// 机位在他背后时**一定**看不见、或者等于要他转过来的写法。
///
/// 侧脸、眼睛朝向、嘴角这类**不算**：过肩镜头里前景人物的脸颊轮廓和视线方向本来就看得到一部分 ——
/// 第一版把它们也算进去，四集报了 30 条，几乎都是正常的过肩写法。第 7 镜真正出事的是胸前的工牌、
/// 「侧脸转向镜头可见嘴部」：前者只有正面看得到，后者是要背对的人转过来。手也不算：背后常能看见手。
const FRONT_WORDS: &[&str] = &[
    "胸口", "胸前", "正脸", "整张脸", "露出脸", "露出嘴", "可见嘴", "嘴部", "转向镜头", "面向镜头", "面朝镜头", "脸朝镜头", "看向镜头", "望向镜头", "朝向镜头",
];
/// 一句里有这些就是在说「看不到」，不算写了看不见的东西
const NEGATIONS: &[&str] = &["看不到", "看不见", "不露", "不见", "遮住", "挡住", "不可见", "不转身", "不给", "背对", "不出现", "不入", "藏着", "hidden"];
/// 镜头说明 / 运镜里「机位在某人背后」的说法，`{}` = 名字
const BEHIND: &[&str] = &["从{}肩后", "{}肩后", "{}的肩后", "越过{}", "过{}的肩", "{}身后", "{}背后", "{}的背后"];
/// 运镜里换到这个人前面去了：这一段不再算背对
const COME_AROUND: &[&str] = &["绕到{}身前", "绕到{}正面", "转到{}正面", "绕到{}前面"];

/// 进场的说法
const ENTER_WORDS: &[&str] = &["走进", "进门", "闯进", "冲进", "推门进", "进屋", "入画", "走入", "踏进", "出现在", "冒出", "走来", "现身", "跟进来", "钻进", "进来"];
/// 剧本原文里有这些就算「这一镜有人进场」（宽松：宁可少报）
const SOURCE_ENTER: &[&str] = &["进", "来了", "出现", "赶到", "到了", "回来", "门开", "推门"];
/// 一句话说的是人（不是「倒影出现在屏幕上」这种）
const PERSON_WORDS: &[&str] = &["人", "男人", "女人", "保安", "邻居", "路人", "陌生", "身影", "人影", "群演", "打手", "警察", "店员", "孩子"];

/// 「这个人不在画面里」：整句（到句号 / 分号）里有这些就不算他在画里。按整句看，因为「陆川跪在星澜身后，不入画」被逗号拆成了两半
///
/// 「镜头左外」「镜头外」「画框外」「出画」也算（2026-10-11 E05 第 12 镜：「陆川始终位于镜头左外」被当成他在画里）
const OFF_FRAME: &[&str] = &[
    "不入画", "画外", "画面外", "不在画面", "不出画", "出画", "镜头外", "镜头左外", "镜头右外", "画框外",
    "无其他", "没有其他", "没有别的", "无人", "没有人",
];

fn sentences(text: &str) -> impl Iterator<Item = &str> {
    text.split(['。', '；', '！', '？', ';']).map(str::trim).filter(|c| !c.is_empty())
}

fn clauses(text: &str) -> impl Iterator<Item = &str> {
    text.split(['，', '。', '；', '！', '？', '：', ',', ';']).map(str::trim).filter(|c| !c.is_empty())
}

fn fill(pattern: &str, name: &str) -> String {
    pattern.replace("{}", name)
}

/// 句首的修饰（「前景老秦」「画面左侧的陆川」）
const LEADS: &[&str] = &["前景", "后景", "画面左侧的", "画面右侧的", "画面左侧", "画面右侧", "随即", "同时", "此时", "而"];

/// 这一句的主语。**只认句首**：句子以某个名字开头（允许前面有「前景」这类修饰）就是他；
/// 以「他 / 她」开头、或者一个名字都没有，沿用上一句的主语；名字在句中间（「浅粉色眼睛直视陆川」里陆川是宾语）→ 不知道是谁，不判。
fn subject<'a>(clause: &str, cast: &[&'a str], prev: Option<&'a str>) -> Option<&'a str> {
    let mut c = clause;
    while let Some(rest) = LEADS.iter().find_map(|l| c.strip_prefix(l)) {
        c = rest;
    }
    if let Some(n) = cast.iter().find(|n| c.starts_with(**n) && !c[n.len()..].starts_with('的')) {
        return Some(n);
    }
    if c.starts_with('他') || c.starts_with('她') || !cast.iter().any(|n| clause.contains(n)) {
        return prev;
    }
    None
}

/// 机位在某人背后，时间线却写了他背对镜头时看不见的东西。返回 `(段号从 1 起, 名字, 命中的词, 那一句)`。
///
/// 「背后」从两处来：整镜的镜头说明（`camera_note`，管所有段）和每段自己的运镜；
/// 某段运镜明说绕到他身前，这一段就不算。只看画面和站位两栏 —— 禁止 / 约束里写「老秦露出正脸」是在禁它。
pub fn hidden_side_actions(camera_note: &str, segments: &[ShotSegment], cast: &[&str]) -> Vec<(usize, String, String, String)> {
    let behind_in = |text: &str, name: &str| BEHIND.iter().any(|p| text.contains(&fill(p, name)));
    let mut out = Vec::new();
    for (i, seg) in segments.iter().enumerate() {
        for &name in cast {
            let behind = (behind_in(camera_note, name) || behind_in(&seg.camera, name))
                && !COME_AROUND.iter().any(|p| seg.camera.contains(&fill(p, name)));
            if !behind {
                continue;
            }
            for field in [&seg.visual, &seg.blocking] {
                let mut owner: Option<&str> = None;
                for c in clauses(field) {
                    owner = subject(c, cast, owner);
                    if owner != Some(name) || NEGATIONS.iter().any(|n| c.contains(n)) {
                        continue;
                    }
                    if let Some(w) = FRONT_WORDS.iter().find(|w| c.contains(*w)) {
                        out.push((i + 1, name.to_string(), w.to_string(), c.to_string()));
                        break;
                    }
                }
            }
        }
    }
    out.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
    out
}

/// 时间线里有、剧本这一镜里没有的人。返回给人看的话（每条一句）。
///
/// - 资产库里的角色名写进了时间线，却既不在出场名单、剧本原文也没提他；
/// - 时间线写了有人进场（「走进」「入画」……），剧本原文这一镜没有任何进场的说法。
///
/// 只看画面和站位两栏。`library` = 资产库里所有角色的本名。
pub fn unsourced_people(source: &str, segments: &[ShotSegment], cast: &[&str], library: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    // 名单外的角色**当主语**（句首、后面不跟「的」）才算他在画面里：「陆川的铁门」「陆川屋内暖光」是地方，不是人
    let outsiders: Vec<&str> = library.iter().copied().filter(|n| !n.is_empty() && !cast.contains(n) && !source.contains(*n)).collect();
    'name: for &name in &outsiders {
        for seg in segments {
            for field in [&seg.visual, &seg.blocking] {
                let on_screen = sentences(field).filter(|st| !OFF_FRAME.iter().any(|w| st.contains(w)));
                if on_screen.flat_map(clauses).any(|c| subject(c, &[name], None) == Some(name)) {
                    out.push(format!("时间线写到了「{name}」，但他不在这一镜的出场名单里，剧本这一镜也没提他"));
                    continue 'name;
                }
            }
        }
    }
    // 进场只管名单外的人：名单里的人中途进场是正常的（开场不在画面里另有 `absent_from_opening` 管）
    if !SOURCE_ENTER.iter().any(|w| source.contains(w)) {
        'seg: for (i, seg) in segments.iter().enumerate() {
            for field in [&seg.visual, &seg.blocking] {
                for c in sentences(field).filter(|st| !OFF_FRAME.iter().any(|w| st.contains(w))).flat_map(clauses) {
                    let enters = ENTER_WORDS.iter().any(|w| c.contains(w));
                    let person = !cast.iter().any(|n| c.contains(n)) && (outsiders.iter().any(|n| c.contains(n)) || PERSON_WORDS.iter().any(|w| c.contains(w)));
                    if enters && person && !NEGATIONS.iter().any(|n| c.contains(n)) && !c.contains("没有人") && !c.contains("无人") {
                        out.push(format!("第 {} 段写了有人进场（「{c}」），剧本这一镜没有人进场", i + 1));
                        break 'seg;
                    }
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 表演合同 / 打斗的检查（2026-10-12，Codex 方案「真人表演 QC」+ 打斗 QC）
//
// **只查填了合同 / 交换的段**：老镜头没有这两个字段，扫它们的画面文字会刷屏
// （10-10 第一版时间线检查在真数据上误报 43 条，教训）。都是 advice，不挡出片。
// ---------------------------------------------------------------------------

const WIDE: &[&str] = &["远景", "全景", "大全景", "大远景", "wide", "extreme long"];
const FROZEN: &[&str] = &["100%", "完全不变", "纹丝不动", "zero change", "frozen", "一动不动"];
const WET: &[&str] = &["汗", "泪", "血", "水珠", "水滴", "雨水", "灰尘"];

fn is_wide(scale: &str) -> bool {
    let s = scale.to_lowercase();
    WIDE.iter().any(|w| s.contains(w))
}

/// 表演合同的检查。回「第 N 段：……」
pub fn performance_advice(shot: &crate::cinema::StoryboardShot) -> Vec<String> {
    let mut out = Vec::new();
    let wide = is_wide(&shot.scale);
    for (i, seg) in shot.segments.iter().enumerate() {
        let Some(p) = seg.performance.as_ref() else { continue };
        let n = i + 1;
        let signals = crate::combat::signal_count(p);
        if signals > 4 {
            out.push(format!("第 {n} 段：表演信号 {signals} 个，超过 4 个 —— 过度表演，H3 会每样都演一点、哪样都不像。留最能说明这一刻的 2~3 个"));
        }
        if signals == 0 && p.gaze.trim().is_empty() {
            out.push(format!("第 {n} 段：表演合同只写了谁，没有任何看得见的行为（视线 / 面部 / 呼吸 / 手）—— 情绪要落成行为"));
        }
        if wide && !p.face.is_empty() {
            out.push(format!("第 {n} 段：这一镜是{}，合同里写了面部（{}）—— 远处五官看不清，写了会崩脸。改成姿态、重心、轮廓", shot.scale, p.face.join("、")));
        }
        if let Some(m) = &p.material {
            if m.source.trim().is_empty() || m.motion.trim().is_empty() {
                out.push(format!("第 {n} 段：物理细节缺了来源或运动（来源「{}」/ 运动「{}」）—— 没有来源的汗和水会变成一层油亮皮肤", m.source.trim(), m.motion.trim()));
            } else if m.light.trim().is_empty() {
                out.push(format!("第 {n} 段：物理细节「{}」没写哪侧光让它看得见 —— 看不见的水滴等于没写", m.source.trim()));
            }
        } else if WET.iter().any(|w| p.body.iter().any(|b| b.contains(w)) || p.face.iter().any(|f| f.contains(w))) {
            out.push(format!("第 {n} 段：合同里写了汗 / 泪 / 血 / 水，却没有物理细节（来源、运动、光）"));
        }
        if FROZEN.iter().any(|w| seg.constraints.contains(w)) && signals > 0 {
            out.push(format!("第 {n} 段：约束里要「完全不变」，合同里又要微表演 —— 两样互相抵消，H3 多半选冻住。约束只锁身份和服装，别锁表情"));
        }
    }
    // 说完就切：有台词、写了合同，却没有一段写「之后留下的状态」
    let has_line = shot.dialogue_lines.iter().any(|l| !l.line.trim().is_empty());
    let any_contract = shot.segments.iter().any(|s| s.performance.is_some());
    if has_line && any_contract && !shot.segments.iter().any(|s| s.performance.as_ref().is_some_and(|p| !p.aftermath.trim().is_empty())) {
        out.push("这一镜有台词，表演合同里没有一段写「说完之后」—— 台词一结束就切，人物像念完稿。至少留 0.3~1 秒反应".into());
    }
    out
}

/// 人物在画左还是画右（按「名字 … 画左 / 画面左侧 …」粗认，认不出回 None）
fn side_of(text: &str, name: &str) -> Option<&'static str> {
    if name.trim().is_empty() {
        return None;
    }
    let at = text.find(name)?;
    let tail: String = text[at + name.len()..].chars().take(12).collect();
    if ["画左", "画面左", "左侧", "左边"].iter().any(|w| tail.contains(w)) {
        Some("左")
    } else if ["画右", "画面右", "右侧", "右边"].iter().any(|w| tail.contains(w)) {
        Some("右")
    } else {
        None
    }
}

/// 打斗的检查
pub fn combat_advice(shot: &crate::cinema::StoryboardShot) -> Vec<String> {
    let mut out = Vec::new();
    let ex: Vec<(usize, &crate::shot_table::ShotSegment, &crate::shot_table::Exchange)> =
        shot.segments.iter().enumerate().filter_map(|(i, s)| s.exchange.as_ref().map(|e| (i + 1, s, e))).collect();
    if !shot.combat_style.is_empty() && ex.is_empty() {
        out.push("这一镜标了打斗，却没有一段写攻防交换 —— 观众会觉得「动作连贯却不像在打」。至少写一个：谁从哪打向哪、怎么挡、结果".into());
    }
    if ex.len() > 2 && shot.duration <= 8.0 {
        out.push(format!("{} 秒的镜写了 {} 个攻防交换 —— 太密，H3 会每招都做一半。8 秒以内最多 2 个，其余拆到下一镜", shot.duration, ex.len()));
    }
    for (k, (n, seg, e)) in ex.iter().enumerate() {
        if e.result.trim().is_empty() {
            out.push(format!("第 {n} 段：攻防交换没写结果（命中 / 擦过 / 落空 / 对撞）—— 没有接触的打斗等于两个人各自挥舞"));
        }
        if e.attack.trim().is_empty() || e.defense.trim().is_empty() {
            out.push(format!("第 {n} 段：攻防交换缺了攻击路径或防守 —— 要写清谁从哪边、什么角度、打向哪里，另一人怎么避 / 挡"));
        }
        if seg.sound.trim().is_empty() && e.camera_sound.trim().is_empty() {
            out.push(format!("第 {n} 段：有接触动作，声音一栏是空的 —— 冲击、衣料、呼吸、环境反应至少写一个"));
        }
        if let Some((m, _, next)) = ex.get(k + 1) {
            if e.new_state.trim().is_empty() || next.opening.trim().is_empty() {
                out.push(format!("第 {n} 段 → 第 {m} 段：上一个交换的「新站位」或下一个的「开场」没写 —— 接不上，下一招会从中立姿势重新开始"));
            } else if let (Some(a), Some(b)) = (side_of(&e.new_state, &e.attacker), side_of(&next.opening, &e.attacker)) {
                if a != b {
                    out.push(format!("第 {n} 段 → 第 {m} 段：{} 上一招结束在画{a}，下一招开场却在画{b} —— 屏幕方向跳了，观众会以为换了人", e.attacker));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(visual: &str, blocking: &str, camera: &str) -> ShotSegment {
        ShotSegment { performance: None, exchange: None, start: 0.0, end: 1.0, title: String::new(), visual: visual.into(), forbid: String::new(), constraints: String::new(), blocking: blocking.into(), camera: camera.into(), sound: String::new() }
    }

    const CAST: &[&str] = &["星澜", "陆川", "老秦"];
    const NOTE: &str = "从老秦肩后看被按墙的星澜，保持轴线东侧；黑屏后停住看老秦反应。";

    /// EP04 第 7 镜改之前的原文：第 2 段要胸口工牌、侧脸露嘴 —— 要报；第 1 段星澜的嘴唇不算老秦的
    #[test]
    fn 过肩镜头写了背对者看不见的部位_报() {
        let segs = vec![
            seg("星澜在两名保安的按压下只把头侧转过来，长发沿肩背滑开，露出后颈数据线；她平静直视老秦，嘴唇以播报般准确的节奏发问。陆川仍被压在地面，只在画面下缘虚化出现。",
                "星澜位于画面右侧北墙前，身体朝北、头转向西南看老秦；两名保安分处她两侧；老秦肩背占画面左前景，面向星澜；终端位于左下；陆川在下方远前景。",
                "老秦肩后中近景起，极缓推向星澜的脸"),
            seg("老秦先怔一下，随即挺起胸口的工牌，转向星澜强撑着回答；他的手抓紧数据线，声音比动作更有底气。",
                "老秦仍在画面左前景，侧脸转向镜头可见嘴部；星澜在画面右侧直视他；终端在左下持续运行。",
                "推进停止，焦点从星澜移到老秦前景侧脸和晃动的工牌。"),
        ];
        let hits = hidden_side_actions(NOTE, &segs, CAST);
        assert_eq!(hits.iter().map(|h| (h.0, h.1.as_str())).collect::<Vec<_>>(), [(2, "老秦")], "{hits:?}");
        assert_eq!(hits[0].2, "胸口");
    }

    /// 改过之后的原文：只写后脑和右肩，「看不到他的脸」是否定 —— 不报
    #[test]
    fn 只写背后看得见的_或者明说看不到的_不报() {
        let segs = vec![
            seg("老秦先怔一下，右肩随即一挺、后脖子往前一梗，背对镜头强撑着朝星澜回话；从他右肩后只看得到后脑、右耳和右肩，看不到他的脸、胸前工牌和手。",
                "老秦仍在画面左前景、背对镜头，只露后脑、右耳和右肩；星澜在画面右侧直视他。",
                "推进停止，焦点从星澜移到老秦前景的后脑和右肩；机位留在他右肩后，不绕到他身前。"),
            seg("终端进度条在百分之一处骤停。前景老秦的后脑微微低向屏幕，右肩一下僵住；星澜仍平静看他。", "", ""),
        ];
        assert!(hidden_side_actions(NOTE, &segs, CAST).is_empty());
        // 机位不在任何人背后：照常写脸
        assert!(hidden_side_actions("固定双人侧面中景", &[seg("老秦挺起胸口的工牌", "", "")], CAST).is_empty());
        // 过肩镜头的正常写法：前景人物的侧脸、视线；名字是宾语（「浅粉色眼睛直视陆川」是星澜的眼睛）
        let ots = "从陆川肩后看床上的星澜";
        assert!(hidden_side_actions(ots, &[seg("陆川侧脸被粉紫水光划过；浅粉色眼睛直视陆川，嘴角只松开一点", "", "")], CAST).is_empty());
        // 前一句主语是陆川，这一句名字在中间（星澜是主语的一部分）：不知道是谁，不算陆川的胸口
        assert!(hidden_side_actions(ots, &[seg("陆川停住；铭牌在星澜胸口亮起", "", "")], CAST).is_empty());
        // 明说看不到的（沿用上一句主语、同一句里有否定）
        assert!(hidden_side_actions(NOTE, &[seg("老秦背对镜头一僵，看不到他的胸前工牌", "", "")], CAST).is_empty());
        // 对照：同样的句子去掉否定就要报 —— 上面两条不是因为别的原因才没报
        assert_eq!(hidden_side_actions(NOTE, &[seg("老秦背对镜头一僵，胸前工牌一晃", "", "")], CAST).len(), 1);
        // 运镜绕到他身前：这一段可以写脸
        assert!(hidden_side_actions(NOTE, &[seg("老秦脸色一沉", "", "镜头绕到老秦身前")], CAST).is_empty());
    }

    #[test]
    fn 剧本没有的人和进场_报() {
        let library = ["星澜", "陆川", "老秦", "圆婶"];
        let segs = vec![seg("星澜看向门口；圆婶端着盆探头", "", ""), seg("一个陌生男人推门进屋", "", "")];
        let out = unsourced_people("星澜侧过头，平静地看着老秦。", &segs, CAST, &library);
        assert_eq!(out.len(), 2, "{out:?}");
        assert!(out[0].contains("圆婶") && out[1].contains("第 2 段"), "{out:?}");
        // 剧本里写了进场：不报进场；圆婶在原文里：不报
        let ok = unsourced_people("圆婶推门进来，嚷着要押注。", &segs, CAST, &library);
        assert!(ok.is_empty(), "{ok:?}");
        // 「陆川的铁门」是地方不是人；名单里的人进场是正常的
        let place = vec![seg("竹竿挤到陆川的铁门前", "", ""), seg("老秦推门进屋", "", "")];
        assert!(unsourced_people("竹竿挤进人群。", &place, &["竹竿", "老秦"], &library).is_empty());
        // 明说不在画面里（逗号后面才说「不入画」）
        let off = vec![seg("", "圆婶跪在星澜身后，不入画；星澜位于东侧画外", ""), seg("无其他人物入画", "", ""), seg("", "圆婶始终位于镜头左外", "")];
        assert!(unsourced_people("星澜看着老秦。", &off, CAST, &library).is_empty());
        // 「没有人进来」是否定；屏幕上的倒影不是人
        let neg = vec![seg("门口没有人进来", "", ""), seg("倒影出现在屏幕上", "", "")];
        assert!(unsourced_people("星澜看着老秦。", &neg, CAST, &library).is_empty());
    }

    fn shot_with(segs: Vec<ShotSegment>) -> crate::cinema::StoryboardShot {
        let mut s: crate::cinema::StoryboardShot = serde_json::from_value(serde_json::json!({"id":"s","shot_num":1,"scale":"近景","camera_movement":"","dialogue":"","prompt":"","duration":8.0,"status":"pending"})).unwrap();
        s.segments = segs;
        s
    }

    #[test]
    fn 表演检查_只查填了合同的段() {
        use crate::shot_table::{MaterialEvent, Performance};
        let plain = shot_with(vec![seg("她紧张地攥紧枪，满头大汗", "", "")]);
        assert!(performance_advice(&plain).is_empty(), "老镜头没合同不查：{:?}", performance_advice(&plain));
        let mut a = seg("", "", "");
        a.performance = Some(Performance { face: vec!["慢眨".into(), "下眼睑收紧".into()], body: vec!["吸气".into(), "吞咽".into()], hand: "手指松开枪柄".into(), ..Default::default() });
        let mut b = seg("", "", "");
        b.performance = Some(Performance { performer: "星澜".into(), ..Default::default() });
        let mut c = seg("", "", "");
        c.performance = Some(Performance { gaze: "看他".into(), material: Some(MaterialEvent { source: "雨".into(), ..Default::default() }), ..Default::default() });
        let msgs = performance_advice(&shot_with(vec![a, b, c]));
        assert!(msgs.iter().any(|m| m.starts_with("第 1 段") && m.contains("5 个")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.starts_with("第 2 段") && m.contains("没有任何看得见的行为")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.starts_with("第 3 段") && m.contains("来源或运动")), "{msgs:?}");
        // 远景写面部
        let mut w = shot_with(vec![seg("", "", "")]);
        w.scale = "全景".into();
        w.segments[0].performance = Some(Performance { face: vec!["眉心短收".into()], ..Default::default() });
        assert!(performance_advice(&w).iter().any(|m| m.contains("崩脸")));
    }

    #[test]
    fn 打斗检查() {
        use crate::shot_table::Exchange;
        let mut s = shot_with(vec![seg("", "", "")]);
        s.combat_style = "melee".into();
        assert!(combat_advice(&s).iter().any(|m| m.contains("没有一段写攻防交换")));
        let mut a = seg("", "", "");
        a.sound = "金属撞击".into();
        a.exchange = Some(Exchange { attacker: "猎手".into(), attack: "右臂刀斜劈".into(), defense: "横枪".into(), result: "刀砍在枪身上".into(), new_state: "猎手在画右，退半步".into(), ..Default::default() });
        let mut b = seg("", "", "");
        b.sound = "破风".into();
        b.exchange = Some(Exchange { attacker: "猎手".into(), opening: "猎手在画左，压低重心".into(), attack: "左刃直刺".into(), defense: "晶片盾".into(), result: "刺在盾心".into(), ..Default::default() });
        s.segments = vec![a, b];
        let msgs = combat_advice(&s);
        assert!(msgs.iter().any(|m| m.contains("屏幕方向跳了")), "{msgs:?}");
        assert!(!msgs.iter().any(|m| m.contains("没写结果") || m.contains("声音一栏")), "{msgs:?}");
    }

}
