//! AI 润色 —— 草稿提示词 → 双语高质量提示词。
//!
//! # 这个模块存在的理由
//!
//! `prompts.rs` 里的 `StoryboardPolish` / `VideoPolish` / `R2vPolish` 三个槽位
//! **从写下来那天起就没有任何调用点**，而 `prompt_slots` 把它们全列给了
//! 设置界面。用户能打开「出图提示词润色」认真改一段提示词、保存成功 ——
//! 然后什么都不会发生。这正是这个项目最典型的失败形状：
//! 接受了、成功了、什么都没做。
//!
//! # 三条硬规矩
//!
//! **1. 失败必须响。** 没有静默回落到原文这种事。模型没配好、返回不是
//! JSON、少了某个 key —— 每一种都有自己的说法，因为用户对这四种情况
//! 该做的动作完全不同（去设置里配 / 重试 / 换个模型 / 简化草稿）。
//!
//! **2. 「模型原样退回」是警告不是错误。** 本地小模型很爱把输入抄一遍
//! 交差。这时候结果本身是能用的（就是原文），所以照样交给前端显示，
//! 只是挂个黄色警告让用户去补一句具体要求。当成错误吞掉的话，
//! 用户会以为是链路坏了。
//!
//! **3. 润色结果不自动写回。** 交给用户点「应用」。自动写回会造成
//! 「我看到的提示词和实际发出去的不一样」—— 那是我们刚在资产工作台
//! 修掉的同一类问题。

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::prompts::{PromptSlot, extract_json};

/// 润色哪一条链路。和 `PromptSlot` 一一对应 —— **不要在这里再造一套提示词**，
/// 用户在设置里改的就是那三个槽位，两份真相会让「我改了没生效」重演。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolishKind {
    /// 分镜草稿 → 出图提示词
    Storyboard,
    /// 草稿 → 图生视频（**有首帧**）
    Video,
    /// 草稿 → 文生视频（**没有首帧**）。见 [`PromptSlot::T2vPolish`]。
    T2v,
    /// 草稿 → 参考生视频
    R2v,
    /// 关键词串 → 连贯句子（创作台）。见 [`PromptSlot::ProsePolish`]。
    Prose,
}

impl PolishKind {
    pub fn slot(self) -> PromptSlot {
        match self {
            PolishKind::Storyboard => PromptSlot::StoryboardPolish,
            PolishKind::Video => PromptSlot::VideoPolish,
            PolishKind::T2v => PromptSlot::T2vPolish,
            PolishKind::R2v => PromptSlot::R2vPolish,
            PolishKind::Prose => PromptSlot::ProsePolish,
        }
    }
}

/// 一次润色的结果。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Polished {
    pub prompt_cn: String,
    pub prompt_en: String,
    /// 非空 = 结果能用但有话要说（目前只有「模型基本没改」）。
    /// **不要把它做成错误** —— 原文本身是可用的，只是没进步。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

/// 「模型基本没改」的判定阈值。
///
/// 0.95 是 LumenX 的经验值，我们沿用：再低会误伤「只做了轻微改动」的
/// 合理结果，再高会放过「换几个标点就交差」。
const ECHO_THRESHOLD: f32 = 0.95;

/// 「这两段是不是几乎同一段文本」的判据（0~1）。最长公共子序列 / 平均长度。
///
/// **不是通用相似度函数，别拿它做别的判断。** 长度比低于阈值时直接返回
/// `0.0` 而不做 DP —— 那不表示「毫不相干」，只表示「肯定达不到阈值」。
/// 润色正常干活时结果会明显变长，这条短路挡掉的正是绝大多数调用：
/// 否则 1200 字的草稿每次都要跑 144 万次比较，去得出一个显而易见的结论。
fn similarity(a: &str, b: &str) -> f32 {
    let a: Vec<char> = a.trim().to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    let b: Vec<char> = b.trim().to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    let (short, long) = if a.len() < b.len() { (a.len(), b.len()) } else { (b.len(), a.len()) };
    // 长度差一倍以上不可能达到 0.95，省掉这趟 DP
    if (short as f32) / (long as f32) < ECHO_THRESHOLD {
        return 0.0;
    }
    // 滚动数组 LCS
    let mut prev = vec![0u32; b.len() + 1];
    let mut cur = vec![0u32; b.len() + 1];
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            cur[j] = if a[i - 1] == b[j - 1] {
                prev[j - 1] + 1
            } else {
                prev[j].max(cur[j - 1])
            };
        }
        std::mem::swap(&mut prev, &mut cur);
        cur.iter_mut().for_each(|v| *v = 0);
    }
    2.0 * prev[b.len()] as f32 / (a.len() + b.len()) as f32
}

/// 模型是不是把输入抄了一遍。
///
/// **中英两版都要比。** 只比英文版的话，第一次润色（草稿是中文）
/// 永远判不出来 —— 中文原文 vs 英文译文相似度必然极低。
/// 而「把中文草稿原样塞进 prompt_cn」恰恰是本地小模型最常见的偷懒方式。
pub fn is_echo(p: &Polished, draft: &str) -> bool {
    similarity(&p.prompt_en, draft) >= ECHO_THRESHOLD
        || similarity(&p.prompt_cn, draft) >= ECHO_THRESHOLD
}

/// 发给模型的那段草稿正文。
///
/// # 为什么迭代时要把上一版的中英**两版**都给它
///
/// 用户的反馈是中文（「镜头再慢一点」），而要改的是双语两版。
/// 只给英文版的话，模型得先把中文反馈映射到英文文本上，
/// 实测容易改跑偏 —— 改完中文版和英文版说的不是一回事。
/// 给了中文锚点，它是在「同一句话的两个版本」上做同一处修改。
///
/// # 为什么塞进 `{DRAFT}` 而不是新加占位符
///
/// 三个槽位的占位符是**列在设置界面上**的，加一个就等于要求用户
/// 在自定义提示词里也照顾它，忘了就静默不生效。迭代信息本来就属于
/// 「这次要处理的草稿」，放进 DRAFT 语义上也对。
pub fn compose_draft(draft: &str, prev_cn: &str, feedback: &str) -> String {
    let feedback = feedback.trim();
    if feedback.is_empty() {
        return draft.trim().to_string();
    }
    let prev_cn = prev_cn.trim();
    if prev_cn.is_empty() {
        return format!(
            "[当前提示词]\n{}\n\n[用户反馈]\n{}\n\n\
             请按反馈修改，只改用户指出的地方，其余保持不变。",
            draft.trim(),
            feedback
        );
    }
    format!(
        "[当前提示词-中文]\n{prev_cn}\n\n[当前提示词-英文]\n{}\n\n[用户反馈]\n{feedback}\n\n\
         请按反馈同步修改中英两版，只改用户指出的地方，其余保持不变。",
        draft.trim()
    )
}

/// 资产 / 参考槽列表渲染成给模型看的清单。
///
/// **空列表要明说。** 留空的话模型会自己发明角色名 ——
/// 而那些名字在资产库里不存在，标签解析不到，参考图挂不上，不报错。
pub fn render_list(items: &[String]) -> String {
    let items: Vec<&str> = items.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    if items.is_empty() {
        return "（这一集还没有已入库的资产 —— 不要臆造任何角色名或场景名）".into();
    }
    items.iter().map(|s| format!("- {s}")).collect::<Vec<_>>().join("\n")
}

/// 把模型输出解析成双语结果。
///
/// 三种失败分开说，因为用户该做的事完全不同：
/// 不是 JSON → 重试或换模型；少 key → 重试；两版都空 → 草稿太短。
pub fn parse_polished(raw: &str) -> Result<Polished> {
    let json = extract_json(raw).ok_or_else(|| {
        let head: String = raw.chars().take(200).collect();
        Error::runtime(format!(
            "模型没返回 JSON，重试一次或换个后台模型。它返回的是：{head}"
        ))
    })?;
    let v: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| Error::runtime(format!("模型返回的 JSON 解析不了（{e}），重试一次")))?;
    let cn = v.get("prompt_cn").and_then(|x| x.as_str()).unwrap_or("").trim();
    let en = v.get("prompt_en").and_then(|x| x.as_str()).unwrap_or("").trim();
    if cn.is_empty() && en.is_empty() {
        return Err(Error::runtime(
            "模型返回的双语两版都是空的 —— 草稿可能太短，多写两句再润色",
        ));
    }
    if cn.is_empty() || en.is_empty() {
        return Err(Error::runtime(format!(
            "模型只返回了{}一版，双语不完整，重试一次",
            if cn.is_empty() { "英文" } else { "中文" }
        )));
    }
    Ok(Polished { prompt_cn: cn.into(), prompt_en: en.into(), warning: None })
}

/// 模型**没干活、而是回了一句拒绝**。
///
/// # 为什么单独判这一种
///
/// [`is_echo`] 抓的是「原样退回」。拒绝不是原样退回 —— 它是一段
/// 通顺的、和草稿完全不一样的新文本，回声检测**一定放行**。
/// 然后那句「我无法协助这个请求」就被当成润色结果写进提示词框，
/// 再原样发给出图模型。图会出，画面里可能还真出现一行字。
///
/// # 判据分两类
///
/// - **必须开头**的（「抱歉」「我无法」「i'm sorry」）：拒绝语开门见山。
///   用「以它开头」而不是「前 N 个字里有」，是因为中英的字符长度差太多，
///   固定窗口中英只能顾一头。
/// - **哪儿都算**的（「无法协助」「cannot assist」）：这几句在画面描述里
///   几乎不可能出现，用来接住「作为一个 AI 助手，我无法…」这种
///   前面还垫了半句的写法。
///
/// 两类都要求整段不超过 60 字 —— 拒绝语就一两句。
///
/// 第一版写的是「开头 40 字内出现即算」，测试当场抓到误判：
/// 「少女站在雨里，**抱歉**的表情写在脸上，霓虹灯映在积水中」——
/// 那是一句完全正常的画面描述。
///
/// **已知的取舍**：一段以「抱歉」开头的短画面描述会被误判。
/// 这个方向是故意的 —— 误判会当场报错、用户看得见、手动改一下就过；
/// 漏判则是把「我无法协助」这句话原样发给出图模型，而那不报错。
pub fn is_refusal(text: &str) -> bool {
    /// **必须出现在开头**的那些词。中英的字符长度差太多，
    /// 用固定窗口（前 N 个字）中英只能顾一头 —— 所以直接要求「以它开头」。
    const STARTERS: [&str; 12] = [
        "抱歉",
        "很抱歉",
        "对不起",
        "我不能",
        "我无法",
        "我不会",
        "作为一个",
        "作为 ai",
        "i'm sorry",
        "i am sorry",
        "i can't",
        "i cannot",
    ];
    /// 出现在**哪儿都算**的那几句。它们在画面描述里几乎不可能出现，
    /// 所以不需要位置约束 —— 用来接住「作为一个 AI 助手，我无法…」
    /// 这种前面还垫了半句的写法。
    const ANYWHERE: [&str; 8] = [
        "无法协助",
        "不能协助",
        "无法生成这",
        "不能生成这",
        "不适合生成",
        "cannot assist",
        "can't help with",
        "won't be able to help",
    ];

    let t = text.trim().to_lowercase();
    if t.is_empty() {
        return false;
    }
    // 拒绝语是短的。长文本里出现这些词多半是画面描述的一部分。
    if t.chars().count() > 60 {
        return false;
    }
    STARTERS.iter().any(|m| t.starts_with(m)) || ANYWHERE.iter().any(|m| t.contains(m))
}

/// 模型回了拒绝时挂的那句话。
pub const REFUSAL_WARNING: &str =
    "模型没有改写，而是回了一句拒绝 —— 这段话不能当提示词用。\
     润色走的是设置里选的后端；换一个本机模型再试。";

/// 模型基本没改时挂的那句话。单拎出来是为了能在测试里对得上。
pub const ECHO_WARNING: &str =
    "模型几乎原样退回了草稿。在下面的反馈框里补一句具体要求（运镜、光影、情绪）再优化一次。";

// ---------------------------------------------------------------------------
// Tauri command
// ---------------------------------------------------------------------------

/// 跑一次润色。
///
/// `assets` 给分镜润色用（`{ASSETS}`），`slots` 给参考生视频用（`{SLOTS}`）,
/// `engine` 决定 `{MAX_CHARS}` —— **逐引擎，不是常数**：SDXL 超过 150 字
/// 会被 CLIP 摊薄，而那个失败是静默的。
#[tauri::command]
pub async fn cinema_polish_prompt(
    state: tauri::State<'_, crate::AppState>,
    kind: PolishKind,
    draft: String,
    prev_cn: Option<String>,
    feedback: Option<String>,
    assets: Option<Vec<String>>,
    slots: Option<Vec<String>>,
    engine: Option<String>,
) -> Result<Polished> {
    let draft = draft.trim().to_string();
    if draft.is_empty() {
        return Err(Error::runtime("草稿是空的 —— 先写点什么再润色"));
    }
    let prev_cn = prev_cn.unwrap_or_default();
    let feedback = feedback.unwrap_or_default();
    let body = compose_draft(&draft, &prev_cn, &feedback);
    let listed = render_list(&assets.or(slots).unwrap_or_default());
    let max_chars =
        crate::prompts::polish_max_chars(engine.as_deref().unwrap_or("zimage")).to_string();

    let cfg = crate::prompts::load(&state.config_dir);
    let raw = crate::prompts::run(
        &state.gateway,
        &cfg,
        kind.slot(),
        &[
            ("DRAFT", body.as_str()),
            ("ASSETS", listed.as_str()),
            ("SLOTS", listed.as_str()),
            ("MAX_CHARS", max_chars.as_str()),
        ],
    )
    .await?;

    let mut out = parse_polished(&raw)?;
    // 比的是**用户这次给的草稿**，不是拼过迭代信息的那一坨 ——
    // 后者永远不会被原样退回，那样这个检查等于没有。
    if is_refusal(&out.prompt_cn) || is_refusal(&out.prompt_en) {
        // **不能当结果用。** 回声只是「没进步」，拒绝是「这不是提示词」——
        // 放行的话那句「我无法协助」会被原样发给出图模型。
        return Err(Error::runtime(REFUSAL_WARNING));
    }
    if is_echo(&out, &draft) {
        out.warning = Some(ECHO_WARNING.into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- 拒绝检测 ----

    /// **拒绝不是回声。** 它是一段通顺的、和草稿完全不一样的新文本，
    /// 回声检测一定放行 —— 然后那句「我无法协助」就被当成润色结果
    /// 写进提示词框，再原样发给出图模型。
    #[test]
    fn 模型回拒绝时认得出来() {
        for t in [
            "抱歉，我无法协助这个请求。",
            "很抱歉，我不能生成这类内容",
            "对不起，我不会处理这样的要求。",
            "I'm sorry, I can't help with that.",
            "I cannot assist with this request.",
            // 前面垫了半句的写法
            "作为一个 AI 助手，我无法协助这类请求。",
            "很遗憾，这个请求我不能协助。",
        ] {
            assert!(is_refusal(t), "没认出拒绝：{t}");
        }
    }

    /// **正常的画面描述里出现「无法」「不能」不算拒绝。**
    ///
    /// 判据是开头那一小段 + 整段够短。全文包含式的判断会误伤 ——
    /// 「箱盖没有完全合拢，看不清里面」这种句子完全正常。
    #[test]
    fn 画面描述里的否定词不算拒绝() {
        for t in [
            "一个纸箱放在木地板上，箱盖没有完全合拢，无法看清里面的东西，光线柔和",
            "少女站在雨里，抱歉的表情写在脸上，霓虹灯映在积水中，长镜头缓慢推近，背景虚化",
            "",
            "   ",
        ] {
            assert!(!is_refusal(t), "误判成拒绝：{t}");
        }
    }

    /// 长文本一律不算拒绝 —— 拒绝语通常就一两句。
    #[test]
    fn 长文本不算拒绝() {
        let long = format!("抱歉{}", "，画面里有一只三米小团蜷在纸箱里".repeat(10));
        assert!(long.chars().count() > 60);
        assert!(!is_refusal(&long), "长描述被判成拒绝了");
    }

    /// **判据必须钉在开头**，不能是「前 40 个字里出现过」。
    ///
    /// 这条是测试当场抓出来的：第一版写的是前 40 字内包含即算，
    /// 而「少女站在雨里，抱歉的表情写在脸上」直接中招。
    #[test]
    fn 拒绝词出现在句子中间不算() {
        assert!(!is_refusal("少女站在雨里，抱歉的表情写在脸上，霓虹映在积水中"));
        assert!(!is_refusal("他摇摇头，我不能去，声音很轻"));
    }

    #[test]
    fn 每种润色各自对应一个槽位_不重复() {
        let slots = [
            PolishKind::Storyboard.slot(),
            PolishKind::Video.slot(),
            PolishKind::T2v.slot(),
            PolishKind::R2v.slot(),
            PolishKind::Prose.slot(),
        ];
        for (i, a) in slots.iter().enumerate() {
            for b in slots.iter().skip(i + 1) {
                assert_ne!(a.key(), b.key(), "两种润色指向了同一个槽位");
            }
        }
    }

    /// **这条盯的是「槽位不再是死的」。** 三个润色槽位曾经从定义那天起
    /// 就没有任何调用点，而设置界面照样把它们列出来给用户改。
    #[test]
    fn 每种润色用到的占位符都在它槽位的默认文案里() {
        for k in [PolishKind::Storyboard, PolishKind::Video, PolishKind::T2v, PolishKind::R2v] {
            let text = k.slot().default_text();
            assert!(text.contains("{DRAFT}"), "{} 的默认文案里没有 {{DRAFT}}", k.slot().key());
            for ph in k.slot().placeholders() {
                assert!(text.contains(ph), "{} 少了 {ph}", k.slot().key());
            }
        }
    }

    #[test]
    fn 首次润色时草稿原样送出() {
        assert_eq!(compose_draft("  一个纸箱在动  ", "", ""), "一个纸箱在动");
        // 有上一版但没反馈 = 还是首次形态，不该塞迭代块
        assert_eq!(compose_draft("一个纸箱在动", "上一版", "   "), "一个纸箱在动");
    }

    #[test]
    fn 迭代时把中英两版和反馈一起给模型() {
        let out = compose_draft("A box trembles", "纸箱在颤动", "镜头再慢一点");
        assert!(out.contains("纸箱在颤动"), "缺中文锚点");
        assert!(out.contains("A box trembles"), "缺英文原版");
        assert!(out.contains("镜头再慢一点"), "缺用户反馈");
        assert!(out.contains("只改用户指出的地方"), "没约束改动范围");
    }

    #[test]
    fn 没有上一版中文时退回单语迭代形态() {
        let out = compose_draft("A box trembles", "", "镜头再慢一点");
        assert!(out.contains("[当前提示词]"), "应该走单语形态");
        assert!(!out.contains("[当前提示词-中文]"), "没有中文锚点却写了双语标题");
        assert!(out.contains("镜头再慢一点"));
    }

    #[test]
    fn 资产列表为空时明确禁止臆造() {
        let s = render_list(&[]);
        assert!(s.contains("不要臆造"), "空列表必须明说，否则模型会自己发明角色名");
        let s = render_list(&["小花".into(), "  ".into(), "纸箱".into()]);
        assert_eq!(s, "- 小花\n- 纸箱", "空白项应该被滤掉");
    }

    #[test]
    fn 解析双语结果() {
        let p = parse_polished(r#"{"prompt_cn":"中文版","prompt_en":"English"}"#).unwrap();
        assert_eq!(p.prompt_cn, "中文版");
        assert_eq!(p.prompt_en, "English");
        assert!(p.warning.is_none());
    }

    /// 本地小模型最爱包一层围栏，直接 `from_str` 会失败，
    /// 而那个失败看起来像「这个模型不支持」。
    #[test]
    fn 解析带_markdown_围栏和前言的输出() {
        let raw = "好的，这是结果：\n```json\n{\"prompt_cn\":\"中\",\"prompt_en\":\"en\"}\n```";
        let p = parse_polished(raw).unwrap();
        assert_eq!(p.prompt_cn, "中");
    }

    #[test]
    fn 三种解析失败各说各的() {
        let e = parse_polished("我不太明白你的意思").unwrap_err().to_string();
        assert!(e.contains("没返回 JSON"), "{e}");
        assert!(e.contains("我不太明白"), "要把模型原话带出去，否则用户不知道发生了什么");

        let e = parse_polished(r#"{"prompt_cn":"中"}"#).unwrap_err().to_string();
        assert!(e.contains("只返回了中文"), "少哪一版要说清楚：{e}");
        let e = parse_polished(r#"{"prompt_en":"en"}"#).unwrap_err().to_string();
        assert!(e.contains("只返回了英文"), "少哪一版要说清楚：{e}");

        let e = parse_polished(r#"{"prompt_cn":"","prompt_en":"  "}"#).unwrap_err().to_string();
        assert!(e.contains("都是空的"), "{e}");
    }

    #[test]
    fn 原样退回被判成_echo() {
        let draft = "室内地板上，一个棕色硬纸箱微微晃动，箱口边缘轻轻颤动。";
        let p = Polished {
            prompt_cn: draft.into(),
            prompt_en: "A brown box trembles on the floor.".into(),
            warning: None,
        };
        assert!(is_echo(&p, draft), "中文版原样退回没被认出来");
    }

    #[test]
    fn 英文版原样退回也算_echo() {
        let draft = "A brown cardboard box trembles slightly on the indoor floor.";
        let p = Polished {
            prompt_cn: "一个棕色纸箱在室内地板上轻轻颤动。".into(),
            prompt_en: format!("{draft} "),
            warning: None,
        };
        assert!(is_echo(&p, draft));
    }

    #[test]
    fn 真的改过就不是_echo() {
        let draft = "一个纸箱在动";
        let p = Polished {
            prompt_cn: "室内地板上，一个棕色硬纸箱微微晃动，箱口边缘轻轻颤动，\
                        仿佛有生命在内部蠕动。镜头缓缓推近。"
                .into(),
            prompt_en: "On an indoor floor, a brown cardboard box trembles slightly, \
                        its flaps quivering. The camera slowly pushes in."
                .into(),
            warning: None,
        };
        assert!(!is_echo(&p, draft));
    }

    /// 第一次润色时草稿是中文、结果的英文版是译文 ——
    /// **只比英文版的话这里永远判不出 echo**，那个检查等于没有。
    #[test]
    fn 中译英不算_echo() {
        let draft = "一个棕色纸箱在地板上轻轻颤动";
        let p = Polished {
            prompt_cn: "一个棕色纸箱在地板上轻轻颤动，光线温暖。".into(),
            prompt_en: "A brown cardboard box trembles gently on the floor.".into(),
            warning: None,
        };
        assert!(!is_echo(&p, draft));
    }

    #[test]
    fn 相似度_长度差一倍以上直接判不同() {
        assert_eq!(similarity("abc", "abcabcabcabc"), 0.0);
        assert_eq!(similarity("", "abc"), 0.0);
        assert_eq!(similarity("abc", "abc"), 1.0);
        // 大小写和空白不算差异
        assert_eq!(similarity(" ABC ", "abc"), 1.0);
    }

    /// 长度差得多时短路返回 0 —— 这是「肯定不是同一段」，不是「毫不相干」。
    /// 润色真干了活的时候结果都会明显变长，走的就是这条路。
    #[test]
    fn 相似度_长度拉开后直接短路() {
        let a = "the quick brown fox jumps over the lazy dog";
        let b = "the quick brown fox leaps gracefully over the sleepy dog \
                 while warm afternoon light spills across the yard";
        assert_eq!(similarity(a, b), 0.0);
    }

    /// 长度接近但确实改过词：要真的算一遍 DP，且落在阈值之下。
    /// 这条是防「短路把所有情况都吃掉、DP 那段其实从没跑过」。
    #[test]
    fn 相似度_长度接近但改过词要落在阈值之下() {
        let a = "the quick brown fox jumps over the lazy dog tonight";
        let b = "the quick brown cat leaps above the noisy dog tonight";
        let s = similarity(a, b);
        assert!(s > 0.0, "长度接近就该真算，不该被短路吃掉：{s}");
        assert!(s < ECHO_THRESHOLD, "改了三个词不该判成原样退回：{s}");
    }
}
