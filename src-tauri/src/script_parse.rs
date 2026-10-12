//! 剧本行分类：**台词 / 动作 / 屏幕文字 / 场景标题**。
//!
//! # 为什么需要这一层
//!
//! 2026-09-19 Codex Astra 的审查抓到：`dialogue_coverage` 只扫
//! `“…”` / `「…」`，而它是**双向错**的。
//!
//! 实测 E01 的剧本：
//!
//! ```text
//! 橘猫：各位领导，下面我做年度述职。          ← 真台词，一个引号都没有
//! 镜头切到门口……快递单上写着「昨天签收」。     ← 唯一的「」，是屏幕文字
//! ```
//!
//! 于是那个检查：
//!
//! - **真台词一句都查不到**（它只找引号）；
//! - **屏幕文字被当成「漏掉的台词」报出来**，喂给模型当重试反馈。
//!
//! 它的模块注释写着它是为了防「模型没往台词栏写任何东西」——
//! 那条防线当时完全是空的，还在反向误导重试。
//!
//! # 认两种台词写法
//!
//! 提示词规定的是「角色名单独一行，下一行是台词」（`SCREENPLAY_FROM_SOURCE`），
//! 但模型实际产出的是行内的 `角色：台词`。**两种都得认** ——
//! 只认规定的那种，等于这个检查对现有四集全部失效。
//!
//! # 屏幕文字不是台词
//!
//! `「昨天签收」`（快递单）、`「已超时 32 分钟」`（手机）、`「上班」`（纸条）
//! 都是**画面里的字**，是笑点的落点，但不该被念出来，也不该被要求写进台词栏。
//! 它们单独归一类，后续由出图/后期处理（见 `docs/plans/2026-09-19-审查整改总表.md` C4-d）。

use serde::Serialize;

/// 剧本里的一行（或一组行）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum ScriptLine {
    /// `【场景：客厅 · 夜】`
    Heading(String),
    /// 有人说话
    Dialogue {
        speaker: String,
        /// 括号里的语气提示（`（咬牙）`），没有就是空串
        tone: String,
        text: String,
    },
    /// 动作 / 环境描写。`screen_text` 是这一段里出现的**画面文字**
    Action { text: String, screen_text: Vec<String> },
}

/// 标题里的分隔符。**名字里不会出现这些**，出现了就说明这是场景标题。
///
/// `现代 · 深夜办公室`、`盛唐 · 金銮殿 · 日`、`01｜陌生的家` —— 三种真实格式，
/// 都是没有句读又短，不靠这一条挡不住（2026-09-19 拿十集真实剧本跑出来的）。
const SEPARATORS: &[char] = &['·', '•', '·', '｜', '|', '丨', '—', '–'];

/// 企划信息的键。这些后面跟冒号的不是台词。
///
/// 只列**确定不会是角色名**的几个。宁可漏掉几种写法，也不要把真角色挡在外面 ——
/// 漏掉只是少一条检查，挡错会让真台词被报成「漏了」，而那会误导模型重写。
///
/// 2026-09-19 拿十集真实剧本跑出来的：更旧的剧本用 `01｜陌生的家` 当场标题
/// （不是 `【】`），所以「跳过第一个【】之前」那条对它不生效，
/// 于是 `类型： 温馨 / 轻喜剧` `时长： 约 3 分钟` 被当成了台词。
const PLAN_KEYS: &[&str] = &["类型", "时长", "主要角色", "片名", "标题", "基调", "风格", "题材", "简介", "梗概", "logline", "一句话"];

/// 说话人名字最长几个字。
///
/// **用来分辨「角色：台词」和普通带冒号的句子。** 太松的话
/// 「镜头切到门口，一个拆开的快递箱里：整整齐齐码着……」这种也会被当成台词。
/// 12 个字足够放下「林婉清（盛唐）」这类全名。
const MAX_SPEAKER_CHARS: usize = 12;

/// 剧本 → 分类好的行。
pub fn parse_script(text: &str) -> Vec<ScriptLine> {
    let mut out: Vec<ScriptLine> = Vec::new();
    let all: Vec<&str> = text.lines().collect();
    // **第一个【】之前的是企划信息，不是剧本。**
    //
    // `SCREENPLAY_FROM_SOURCE` 的格式里明写了这一条（片名、类型、时长建议）。
    // 不跳过的话 `类型：温馨 / 轻喜剧 / 萌宠`、`时长：约 3 分钟`、
    // `* 小花：一只三个月大的小三花猫` 全会被当成台词 ——
    // 2026-09-19 拿仓库里十集真实剧本跑了一遍，两集旧的正是这么错的。
    //
    // 只在**真的有**标题时跳；场级文本（已经切好的一场）没有标题，一行都不少。
    let lines: Vec<&str> = match all.iter().position(|l| l.trim_start().starts_with('【')) {
        Some(at) if at > 0 => all[at..].to_vec(),
        _ => all,
    };
    let mut i = 0;
    while i < lines.len() {
        let raw = lines[i].trim();
        i += 1;
        if raw.is_empty() {
            continue;
        }
        if raw.starts_with('【') {
            out.push(ScriptLine::Heading(raw.trim_matches(['【', '】']).to_string()));
            continue;
        }
        // 行内写法：`角色：台词` / `角色（咬牙）：台词`
        if let Some(d) = inline_dialogue(raw) {
            out.push(d);
            continue;
        }
        // 「角色名：」单独成行，台词在下一行（带引号）。
        //
        // 旧剧本里有这种写法：`小花心声：` 换行 `“这里……是哪儿？”`。
        // **要求下一行带引号**：只看「这一行以冒号结尾」的话，
        // `他心想：` 后面跟一段叙述也会被当成台词。
        if let Some(head) = raw.strip_suffix(['：', ':']).map(str::trim).filter(|h| bare_speaker(h).is_some()) {
            if let Some(body) = lines
                .get(i)
                .map(|l| l.trim())
                .filter(|l| l.starts_with(['“', '「']))
            {
                out.push(ScriptLine::Dialogue {
                    speaker: head.to_string(),
                    tone: String::new(),
                    text: body.trim_matches(['“', '”', '「', '」']).trim().to_string(),
                });
                i += 1;
                continue;
            }
        }
        // 分行写法：角色名单独一行，可选一行括号语气，再一行台词
        if let Some(name) = bare_speaker(raw) {
            let mut j = i;
            let mut tone = String::new();
            if let Some(t) = lines.get(j).map(|l| l.trim()).filter(|l| is_paren_only(l)) {
                tone = t.trim_matches(['（', '）', '(', ')']).to_string();
                j += 1;
            }
            if let Some(body) = lines.get(j).map(|l| l.trim()).filter(|l| !l.is_empty() && bare_speaker(l).is_none()) {
                out.push(ScriptLine::Dialogue { speaker: name, tone, text: body.to_string() });
                i = j + 1;
                continue;
            }
        }
        // 叙述句里还可能夹着台词：`她抬头：“我还能撑。”`
        // 拆出来的引号内容按下面的判据分成台词和屏幕文字
        let (quoted_lines, screen) = split_quotes(raw);
        out.push(ScriptLine::Action { text: raw.to_string(), screen_text: screen });
        for text in quoted_lines {
            // **说话人留空**：从「大臣厉声」「抬头，高声吟诵」里猜名字必然出错，
            // 而猜错会报一条「说话人错了」的假问题 —— 比不检查更糟。
            // 覆盖检查看到空说话人时只比台词本身（和 2026-09-19 之前的行为一致）。
            out.push(ScriptLine::Dialogue { speaker: String::new(), tone: String::new(), text });
        }
    }
    out
}

/// 一段叙述里的引号内容，分成**台词**和**屏幕文字**。
///
/// # 判据：引号前面紧挨着的是不是冒号
///
/// 拿仓库里八集真实剧本的 15 处引号核过，这一条全中：
///
/// | 台词（冒号在前） | 屏幕文字（没有冒号） |
/// |---|---|
/// | `大臣厉声：「大逆不道！」` | `快递单上写着「昨天签收」` |
/// | `林婉清震惊低语：「这是……唐朝？」` | `手机屏幕上是「已超时 32 分钟」` |
/// | `她抬头：「我还能撑。」` | `卡面印着「终身会员」` |
/// | `皇帝低声：「此诗，当传万世。」` | `标题是「本季度入府人数」` |
/// | `抬头，高声吟诵：「君不见……」` | `屏幕上弹出「相识、订婚、分手…」` |
///
/// **默认归屏幕文字**（没有冒号时）。方向是有意的：
/// 屏幕文字被漏报只是少一条提醒，而把画面上的字当成台词报给模型重试，
/// 会让它去给一张快递单写口型 —— 2026-09-19 之前就是这么错的。
fn split_quotes(text: &str) -> (Vec<String>, Vec<String>) {
    let (mut said, mut shown) = (Vec::new(), Vec::new());
    let mut rest = text;
    let mut consumed = 0usize;
    while let Some(open) = rest.find(['“', '「']) {
        let ch = rest[open..].chars().next().unwrap();
        let close_ch = if ch == '“' { '”' } else { '」' };
        let after = &rest[open + ch.len_utf8()..];
        let Some(close) = after.find(close_ch) else { break };
        let inner = after[..close].trim().to_string();
        let before = &text[..consumed + open];
        if !inner.is_empty() {
            if before.trim_end().ends_with(['：', ':']) {
                said.push(inner);
            } else {
                shown.push(inner);
            }
        }
        consumed += open + ch.len_utf8() + close + close_ch.len_utf8();
        rest = &after[close + close_ch.len_utf8()..];
    }
    (said, shown)
}

/// `角色：台词` / `角色（咬牙）：台词`。不是这个形状回 `None`。
fn inline_dialogue(line: &str) -> Option<ScriptLine> {
    let at = line.find(['：', ':'])?;
    let head = line[..at].trim();
    // 台词外面可能还套一层引号：`大臣厉声：「大逆不道！」`。剥掉 ——
    // 留着的话镜头表里写的是没引号的原话，比对时永远对不上
    let body = line[at + line[at..].chars().next()?.len_utf8()..]
        .trim()
        .trim_matches(['“', '”', '「', '」'])
        .trim();
    if body.is_empty() || head.is_empty() {
        return None;
    }
    // **正文里有间隔号的不是台词，是被剥了方括号的场景标题**
    // （`场景：客厅 · 夜`、`现代 · 深夜办公室`）。模型分的场经常不保留 `【】`。
    // 真台词里出现间隔号极少见，而误判的代价是把整段动作描写报成「漏台词」。
    if body.chars().any(|c| c == '·' || c == '•' || c == '·') {
        return None;
    }
    // 名字部分不能太长、不能带句读 —— 否则普通叙述句会被当成台词
    let (name, tone) = split_tone(head);
    if name.chars().count() > MAX_SPEAKER_CHARS || name.chars().count() < 1 {
        return None;
    }
    if name.chars().any(|c| "。！？；，、…—“”「」《》".contains(c)) {
        return None;
    }
    // 场景名也可能带冒号（`【场景：客厅 · 夜】` 去掉方括号之后）
    if name.chars().any(|c| SEPARATORS.contains(&c)) {
        return None;
    }
    // `* 小花：一只三个月大的小三花猫` —— 列表项是人物表，不是台词
    if line.trim_start().starts_with(['*', '-', '·', '•', '+']) {
        return None;
    }
    if PLAN_KEYS.iter().any(|k| name.eq_ignore_ascii_case(k)) {
        return None;
    }
    Some(ScriptLine::Dialogue { speaker: name, tone, text: body.to_string() })
}

/// `林婉清（盛唐）` → `("林婉清（盛唐）", "")`；`小林（咬牙）` → 也是整体。
///
/// **不在这里拆括号。** 角色全名本来就可能带括号（「林婉清（盛唐）」是资产名），
/// 在这里猜哪个是语气、哪个是名字必然出错。语气交给
/// `storyboard_gen::split_leading_tone` 和镜头表的 `emotion` 字段。
fn split_tone(head: &str) -> (String, String) {
    (head.to_string(), String::new())
}

/// 一行只有一个角色名（分行写法的第一行）。不像就回 `None`。
fn bare_speaker(line: &str) -> Option<String> {
    let t = line.trim();
    if t.is_empty() || t.chars().count() > MAX_SPEAKER_CHARS {
        return None;
    }
    if t.contains(['：', ':']) || is_paren_only(t) || t.starts_with('【') {
        return None;
    }
    // 带句读的不是名字，是短句
    if t.chars().any(|c| "。！？；，、…—".contains(c)) {
        return None;
    }
    // **名字里不含空格和间隔号。**
    //
    // 场景名长这样：`现代 · 深夜办公室`、`盛唐 · 金銮殿 · 日` ——
    // 没有句读、又短，不卡这一条的话它会被当成说话人，
    // 把下一行的动作描写整段当成它的台词报成「漏台词」。
    // 2026-09-19 就是这么错的，已有测试当场抓到。
    if t.chars().any(|c| c.is_whitespace() || SEPARATORS.contains(&c)) {
        return None;
    }
    Some(t.to_string())
}

/// 整行只有一对括号：`（压低声音）`
fn is_paren_only(line: &str) -> bool {
    let t = line.trim();
    (t.starts_with('（') && t.ends_with('）') || t.starts_with('(') && t.ends_with(')')) && t.chars().count() > 2
}

/// 一段动作描写里的**画面文字**：`「昨天签收」`、`「终身会员」`。
///
/// 冒号后面那些是台词，不在这里（见 [`split_quotes`]）。
pub fn screen_texts(text: &str) -> Vec<String> {
    split_quotes(text).1
}

/// 这一段剧本里的台词，按出现顺序。
pub fn dialogue_of(text: &str) -> Vec<(String, String)> {
    parse_script(text)
        .into_iter()
        .filter_map(|l| match l {
            ScriptLine::Dialogue { speaker, text, .. } => Some((speaker, text)),
            _ => None,
        })
        .collect()
}

/// 这一段剧本里的所有画面文字，去重、按出现顺序。
pub fn screen_text_of(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in parse_script(text) {
        if let ScriptLine::Action { screen_text, .. } = l {
            for s in screen_text {
                if !out.contains(&s) {
                    out.push(s);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// E01 的真实剧本片段。**这是这个模块存在的理由** ——
    /// 原来的检查在这段文字上：真台词一句都查不到，`「昨天签收」`反而被报成漏台词。
    const E01: &str = "【场景：客厅 · 夜】\n\
        暖黄的落地灯下，一只圆滚滚的橘猫端坐在茶几上，面前立着一块小小的翻页板。\n\
        橘猫：各位领导，下面我做年度述职。\n\
        小林：半个？\n\
        镜头切到门口，一个拆开的快递箱里码着十二罐猫罐头，快递单上写着「昨天签收」。\n\
        小林（咬牙）：所以用户满意度是零。";

    #[test]
    fn 行内写法的台词认得出来_而且不漏说话人() {
        assert_eq!(
            dialogue_of(E01),
            vec![
                ("橘猫".to_string(), "各位领导，下面我做年度述职。".to_string()),
                ("小林".to_string(), "半个？".to_string()),
                ("小林（咬牙）".to_string(), "所以用户满意度是零。".to_string()),
            ]
        );
    }

    /// **屏幕文字不是台词。** 原来它被当成漏掉的台词报出来，还喂给模型重试。
    #[test]
    fn 屏幕文字单独归一类_不混进台词() {
        assert_eq!(screen_text_of(E01), vec!["昨天签收".to_string()]);
        assert!(!dialogue_of(E01).iter().any(|(_, t)| t.contains("昨天签收")));
    }

    /// 提示词规定的分行写法也要认 —— 只认行内的话，格式正确的剧本反而失效。
    #[test]
    fn 分行写法也认_含单独一行的语气提示() {
        let s = "老茶师\n（压低声音）\n这茶，要等。\n\n年轻人\n等多久？";
        assert_eq!(
            parse_script(s),
            vec![
                ScriptLine::Dialogue { speaker: "老茶师".into(), tone: "压低声音".into(), text: "这茶，要等。".into() },
                ScriptLine::Dialogue { speaker: "年轻人".into(), tone: String::new(), text: "等多久？".into() },
            ]
        );
    }

    /// **普通叙述句里的冒号不能被当成台词。**
    /// 太松的话「他想了想：算了」这类会混进台词栏，比漏检更难查。
    #[test]
    fn 带冒号的叙述句不是台词() {
        for s in [
            "镜头切到门口，一个拆开的快递箱里：整整齐齐码着十二罐猫罐头。",
            "她抬头，看向远处的山：雾气正从谷底升起，像有人在下面烧水。",
        ] {
            assert!(dialogue_of(s).is_empty(), "把叙述句当成台词了：{s}");
        }
    }

    /// 名字里本来就带括号的（「林婉清（盛唐）」是资产全名）不能被拆坏。
    #[test]
    fn 名字带括号的角色不拆() {
        assert_eq!(dialogue_of("林婉清（盛唐）：这是……唐朝？")[0].0, "林婉清（盛唐）");
    }

    /// 场景标题单独一类，不进台词也不进动作的屏幕文字。
    #[test]
    fn 场景标题单独一类() {
        let out = parse_script("【场景：客厅 · 夜】\n他坐下。");
        assert_eq!(out[0], ScriptLine::Heading("场景：客厅 · 夜".into()));
        assert!(matches!(out[1], ScriptLine::Action { .. }));
    }

    /// 同一句台词出现两次要保留两条 —— 去重的话覆盖检查会放过「只写了一遍」。
    #[test]
    fn 重复的台词保留两条() {
        assert_eq!(dialogue_of("甲：走。\n乙：嗯。\n甲：走。").len(), 3);
    }

    /// 引号没闭合时不能卡死，也不能把后面整段当成屏幕文字。
    #[test]
    fn 引号没闭合不崩也不吞掉后文() {
        assert_eq!(screen_texts("牌子上写着「上班"), Vec::<String>::new());
        assert_eq!(screen_texts("先是「上班」，后来又写「下班」"), vec!["上班".to_string(), "下班".to_string()]);
    }

    /// **场景名不是说话人。**
    ///
    /// `现代 · 深夜办公室` 没有句读、又短，不卡住的话会被当成说话人，
    /// 把下一行的动作描写整段当成它的台词报成「漏台词」——
    /// 2026-09-19 改这里时当场踩到。
    #[test]
    fn 场景名不会被当成说话人() {
        for s in [
            "现代 · 深夜办公室
键盘声急促。林婉清伏案加班。",
            "盛唐 · 金銮殿 · 日
鼓声震天。",
            "场景：客厅 · 夜
他坐下。",
        ] {
            assert!(dialogue_of(s).is_empty(), "把场景名当成说话人了：{s}");
        }
    }

    /// 叙述句里夹的台词要认出来，**而且和屏幕文字分开** ——
    /// 判据是引号前面紧挨着的是不是冒号。拿八集真实剧本的 15 处核过。
    #[test]
    fn 冒号后的引号是台词_其余是屏幕文字() {
        let said = |s: &str| dialogue_of(s).into_iter().map(|(_, t)| t).collect::<Vec<_>>();
        assert_eq!(said("她抬头：“我还能撑。”"), vec!["我还能撑。"]);
        assert_eq!(said("大臣厉声：「大逆不道！」"), vec!["大逆不道！"]);
        // 夹在叙述句中间的也要认出来（这一条是整行写法认不到的）
        assert_eq!(said("鼓声震天。她抬头：“我还能撑。”"), vec!["我还能撑。"]);
        assert_eq!(said("快递单上写着「昨天签收」。"), Vec::<String>::new());
        assert_eq!(screen_text_of("快递单上写着「昨天签收」。"), vec!["昨天签收"]);
        assert_eq!(screen_text_of("她抬头：“我还能撑。”"), Vec::<String>::new());
        // 一行里两样都有
        let both = "手机屏幕上是「已超时 32 分钟」，他低声：“完了。”";
        assert_eq!(said(both), vec!["完了。"]);
        assert_eq!(screen_text_of(both), vec!["已超时 32 分钟"]);
    }

    /// 叙述句里夹的台词**说话人留空** —— 从「大臣厉声」「抬头，高声吟诵」
    /// 里猜名字必然出错，而猜错会报一条假的「说话人错了」，比不检查更糟。
    #[test]
    fn 叙述里夹的台词不猜说话人() {
        // 整行就是「谁：台词」时，名字是确定的 —— 台词外面套不套引号都一样
        assert_eq!(dialogue_of("大臣厉声：「大逆不道！」")[0], ("大臣厉声".into(), "大逆不道！".into()));
        assert_eq!(dialogue_of("大臣：大逆不道！")[0].0, "大臣");
        // 而夹在叙述句中间的，名字猜不准，留空
        assert_eq!(dialogue_of("鼓声震天。大臣厉声：「大逆不道！」")[0].0, "");
    }

    /// **第一个【】之前的是企划信息，不是剧本。**
    ///
    /// 不跳的话 `类型：温馨 / 轻喜剧` `时长：约 3 分钟` 会被当成台词 ——
    /// 2026-09-19 拿十集真实剧本跑，两集旧的正是这么错的。
    #[test]
    fn 企划信息不进台词() {
        let s = "《小花回家》
类型：温馨 / 轻喜剧 / 萌宠
时长：约 3 分钟
                 * 小花：一只三个月大的小三花猫

【场景：客厅 · 日】
小雨：你好呀，小花。";
        assert_eq!(dialogue_of(s), vec![("小雨".to_string(), "你好呀，小花。".to_string())]);
    }

    /// 更旧的剧本用 `01｜陌生的家` 当场标题，「跳过【】之前」那条对它不生效。
    /// 两条窄防护兜住：列表项不是台词、企划信息的键不是说话人。
    #[test]
    fn 旧格式里的企划信息和人物表也不进台词() {
        let s = "类型： 温馨 / 轻喜剧 / 萌宠
时长： 约 3 分钟
主要角色：
                 * 小花：一只三个月大的小三花猫
- 小雨：10岁
01｜陌生的家
小雨：你好呀。";
        assert_eq!(dialogue_of(s), vec![("小雨".to_string(), "你好呀。".to_string())]);
    }

    /// **不要把真角色挡在外面。** 这个白名单宁可漏几种写法，
    /// 也不能让真台词被报成「漏了」—— 那会误导模型重写。
    #[test]
    fn 名字里含企划关键词但不等于它的照常算台词() {
        assert_eq!(dialogue_of("类型管理员：请出示证件。").len(), 1);
        assert_eq!(dialogue_of("小时长：我来了。").len(), 1);
    }

    /// 没有【】时一行都不能少 —— **场级文本就是这种**（标题已经被切掉了）。
    #[test]
    fn 没有标题时不跳过任何一行() {
        assert_eq!(dialogue_of("小雨：你好。
小花：喵。").len(), 2);
    }

    /// 「角色名：」单独成行、台词在下一行的写法（旧剧本里有）。
    #[test]
    fn 冒号结尾单独成行_下一行带引号的台词() {
        assert_eq!(
            dialogue_of("小花心声：
“这里……是哪儿？”"),
            vec![("小花心声".to_string(), "这里……是哪儿？".to_string())]
        );
    }

    /// **下一行不带引号就不算。** 只看「以冒号结尾」的话，
    /// `他心想：` 后面跟一段叙述会被整段当成台词。
    #[test]
    fn 冒号结尾但下一行是叙述_不算台词() {
        assert!(dialogue_of("他心想：
窗外的雨一直没停，街上没有人。").is_empty());
    }

    /// 空行、纯空白不产生任何一行。
    #[test]
    fn 空行不产生条目() {
        assert!(parse_script("\n\n   \n").is_empty());
    }
}
