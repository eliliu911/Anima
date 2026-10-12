//! 短期工作记忆：滚动摘要。
//!
//! ## 补的是什么缺口
//!
//! 原先的短期记忆只有"按轮数裁历史"（`trim_history`），而裁掉的部分是
//! `drain(..drop)` —— **真的就没了**。表现是"半小时前说过的事它完全不记得"，
//! 而长期记忆那条路只捞得到被判定为"值得长期记住的事实"，
//! 捞不到"刚才在聊什么"。
//!
//! 所以这里做的事很具体：**历史将被裁掉之前，先让后台模型把要裁的那部分压成
//! 几句话，摘要留在上下文里。**
//!
//! ## 三条刻意的设计
//!
//! **1. 摘要走后台链路，而且不阻塞这一轮。**
//! 压缩要调一次模型（实测本地模型两三秒）。如果同步做，用户会在某一轮突然
//! 多等三秒 —— 而那一轮和别的轮没有任何区别，体验上就是"偶发卡顿"。
//! 所以裁剪立刻发生，摘要在后台补上，下一轮才用得到。
//!
//! **2. 摘要是累积的，不是每次重写。**
//! 新摘要 = 旧摘要 + 这次裁掉的部分，一起压。否则第二次裁剪时旧摘要会被丢掉，
//! 越聊越忘 —— 那正是要修的毛病。
//!
//! **3. 待办用结构化字段，不塞进自然语言。**
//! "用户让我查 X，还没查完"这种事塞进摘要段落里，模型会漏。所以单独一段
//! 列出来，见 [`Summary::focus`]。

use serde::{Deserialize, Serialize};

use crate::config::Tier;
use crate::llm::gateway::Gateway;
use crate::llm::{ChatRequest, Message, ThinkingMode};

/// 摘要压缩到多少字以内。
///
/// 上限不是省钱，是**它每轮都进上下文**：太长会挤占历史、推高首字延迟
/// （本项目量过首包构成，prompt 长度是实打实的成本）。
/// 三百字够装下"聊过什么、结论是什么、还欠什么"。
const MAX_SUMMARY_CHARS: usize = 300;

/// 少于这么多条消息就不值得压缩 —— 压出来的摘要不会比原文短多少。
const MIN_MESSAGES_TO_SUMMARIZE: usize = 4;

/// 会话内的工作记忆。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    /// 已经聊过什么。累积压缩的结果
    pub text: String,
    /// 当前未完成的事。**结构化而不是塞进 text** —— 自然语言里的待办模型会漏。
    ///
    /// 典型场景：用户说"帮我查 X"，中途岔开聊了别的，回来还该记得 X 没查完。
    pub focus: Vec<String>,
}

impl Summary {
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty() && self.focus.is_empty()
    }

    /// 拼成能直接塞进 System Prompt 的一段。空的时候返回 None ——
    /// 注入一句"（暂无摘要）"只是噪音。
    pub fn to_prompt(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let mut out = String::from("## 之前聊过的（这是摘要，不是原话）\n");
        if !self.text.trim().is_empty() {
            out.push_str(self.text.trim());
            out.push('\n');
        }
        if !self.focus.is_empty() {
            out.push_str("\n还没完成的事：\n");
            for item in &self.focus {
                out.push_str("- ");
                out.push_str(item);
                out.push('\n');
            }
        }
        Some(out)
    }
}

/// 给模型的压缩指令。
///
/// 几处刻意的约束：
/// - **要求 JSON**：待办必须是数组，用自然语言让它"列出待办"会拿到一段散文
/// - **明确"保留具体信息"**：不加这句的话模型会压成
///   "用户和助手讨论了一些技术问题"，那等于没有摘要
/// - **明确"第三人称"**：不说的话它会用"我"，而这段摘要之后是给它自己看的，
///   人称混乱会让它把摘要里的"我"当成用户
fn prompt(previous: &Summary, dropped: &[Message]) -> String {
    let transcript = dropped
        .iter()
        .map(|m| {
            let who = match m.role {
                crate::llm::Role::User => "用户",
                crate::llm::Role::Assistant => "助手",
                _ => "系统",
            };
            format!("{who}：{}", m.content.trim())
        })
        .filter(|line| !line.ends_with('：'))
        .collect::<Vec<_>>()
        .join("\n");

    let prev = if previous.text.trim().is_empty() {
        "（没有）".to_string()
    } else {
        previous.text.trim().to_string()
    };
    let prev_focus = if previous.focus.is_empty() {
        "（没有）".to_string()
    } else {
        previous.focus.join("；")
    };

    format!(
        "把下面这段对话压缩成简短的摘要，供之后的对话参考。\n\n\
         已有的摘要：\n{prev}\n\n\
         已有的未完成事项：{prev_focus}\n\n\
         新增的对话：\n{transcript}\n\n\
         只输出一个 JSON 对象，不要解释、不要代码围栏：\n\
         {{\"summary\":\"……\",\"focus\":[\"……\"]}}\n\n\
         硬性要求：\n\
         - summary 要**把已有摘要和新增对话合起来**压缩，不是只写新增的部分\n\
         - summary 不超过 {MAX_SUMMARY_CHARS} 字，用**第三人称**（「用户」「助手」），不要用「我」\n\
         - **保留具体信息**：人名、数字、结论、偏好、决定。\
           压成「讨论了一些技术问题」这种没有信息量的句子等于没压\n\
         - focus 是**还没做完的事**的数组：用户要求过但没完成的、明确说要稍后再谈的。\
           没有就给空数组，不要编\n\
         - 已经做完的事从 focus 里去掉"
    )
}

/// 从模型输出里解析摘要。
///
/// 容错到底：这是个后台任务，失败只意味着"这次没压缩成"，不该让对话出错。
/// 所以能救的都救 —— 剥代码围栏、截断到上限、过滤空待办。
pub fn parse(raw: &str) -> Option<Summary> {
    let cleaned = strip_fence(raw.trim());
    let value: serde_json::Value = serde_json::from_str(cleaned).ok()?;
    let text = value.get("summary")?.as_str()?.trim().to_string();
    let focus: Vec<String> = value
        .get("focus")
        .and_then(|f| f.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    if text.is_empty() && focus.is_empty() {
        return None;
    }
    // 超长就按字符截断。宁可截断也不要让它无限增长 —— 它每轮都进上下文
    let text = if text.chars().count() > MAX_SUMMARY_CHARS {
        text.chars().take(MAX_SUMMARY_CHARS).collect()
    } else {
        text
    };
    Some(Summary { text, focus })
}

fn strip_fence(s: &str) -> &str {
    let s = s.trim();
    if !s.starts_with("```") {
        return s;
    }
    let after = match s.find('\n') {
        Some(i) => &s[i + 1..],
        None => return s,
    };
    match after.rfind("```") {
        Some(i) => after[..i].trim(),
        None => after,
    }
}

/// 该不该为这批被裁掉的消息做摘要。
pub fn worth_summarizing(dropped: &[Message]) -> bool {
    if dropped.len() < MIN_MESSAGES_TO_SUMMARIZE {
        return false;
    }
    // 全是空内容（比如只有工具调用的轮次）没什么可压的
    dropped.iter().any(|m| !m.content.trim().is_empty())
}

/// 压缩一批被裁掉的消息，得到新的累积摘要。
///
/// 走**后台链路**：这不是实时对话，不该占用实时模型，也不该让用户等。
pub async fn compress(
    gateway: &Gateway,
    previous: &Summary,
    dropped: &[Message],
) -> Option<Summary> {
    use futures_util::StreamExt;

    let mut req = ChatRequest::new(vec![Message::user(prompt(previous, dropped))]);
    // 压缩不需要思考过程，而思考会让这个后台任务拖很久
    req.thinking = ThinkingMode::Off;
    // 要的是忠实压缩，不是创作
    req.temperature = Some(0.2);

    let stream_id = uuid::Uuid::new_v4().to_string();
    let (_, mut stream) = gateway
        .chat_stream(stream_id, Tier::Background, req)
        .await
        .map_err(|e| tracing::warn!(%e, "摘要请求失败"))
        .ok()?;

    let mut raw = String::new();
    while let Some(item) = stream.next().await {
        match item {
            Ok(crate::llm::ChatChunk::Text { delta }) => raw.push_str(&delta),
            Ok(_) => {}
            Err(err) => {
                tracing::warn!(%err, "摘要流中断");
                return None;
            }
        }
    }

    match parse(&raw) {
        Some(summary) => {
            tracing::info!(
                chars = summary.text.chars().count(),
                focus = summary.focus.len(),
                dropped = dropped.len(),
                "滚动摘要已更新"
            );
            Some(summary)
        }
        None => {
            // 把原文带出来 —— 解析失败时不知道它输出了什么最难查
            tracing::warn!(
                raw = %raw.chars().take(200).collect::<String>(),
                "摘要解析失败，这次不更新"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: crate::llm::Role, text: &str) -> Message {
        Message {
            role,
            content: text.to_string(),
            images: Vec::new(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    #[test]
    fn an_empty_summary_injects_nothing() {
        // 注入一句"（暂无摘要）"只是噪音，而且每轮都占位置
        assert!(Summary::default().to_prompt().is_none());
        assert!(Summary {
            text: "   ".into(),
            focus: vec![]
        }
        .to_prompt()
        .is_none());
    }

    #[test]
    fn unfinished_items_are_listed_separately_from_the_prose() {
        // 待办塞进段落里模型会漏 —— 所以必须单独成段
        let s = Summary {
            text: "用户在装一个语音助手。".into(),
            focus: vec!["查一下 F5-TTS 的许可".into()],
        };
        let p = s.to_prompt().unwrap();
        assert!(p.contains("用户在装一个语音助手。"));
        assert!(p.contains("还没完成的事"));
        assert!(p.contains("查一下 F5-TTS 的许可"));
    }

    #[test]
    fn only_focus_still_produces_a_prompt() {
        // 只有待办、没有正文也该注入 —— 待办是这里最不能丢的东西
        let s = Summary {
            text: String::new(),
            focus: vec!["回头看看那个 panic".into()],
        };
        let p = s.to_prompt().unwrap();
        assert!(p.contains("回头看看那个 panic"));
    }

    #[test]
    fn parsing_accepts_a_code_fence() {
        // 模型爱加 ```json
        let s = parse("```json\n{\"summary\":\"聊了音色\",\"focus\":[\"试听对比\"]}\n```").unwrap();
        assert_eq!(s.text, "聊了音色");
        assert_eq!(s.focus, vec!["试听对比".to_string()]);
    }

    #[test]
    fn parsing_tolerates_a_missing_focus_field() {
        let s = parse(r#"{"summary":"只有正文"}"#).unwrap();
        assert_eq!(s.text, "只有正文");
        assert!(s.focus.is_empty());
    }

    #[test]
    fn parsing_drops_blank_focus_items() {
        let s = parse(r#"{"summary":"x","focus":["  ","真的待办",""]}"#).unwrap();
        assert_eq!(s.focus, vec!["真的待办".to_string()]);
    }

    #[test]
    fn a_useless_response_is_rejected_rather_than_stored() {
        // 存一个空摘要会把上一次的好摘要覆盖掉 —— 那比不更新更糟
        assert!(parse(r#"{"summary":"","focus":[]}"#).is_none());
        assert!(parse("这不是 JSON").is_none());
        assert!(parse("").is_none());
    }

    #[test]
    fn an_overlong_summary_is_truncated_not_kept() {
        // 它每轮都进上下文，无限增长会挤占历史、推高首字延迟
        let long = "啊".repeat(MAX_SUMMARY_CHARS * 3);
        let s = parse(&format!(r#"{{"summary":"{long}"}}"#)).unwrap();
        assert_eq!(s.text.chars().count(), MAX_SUMMARY_CHARS);
    }

    #[test]
    fn short_or_empty_drops_are_not_worth_a_model_call() {
        use crate::llm::Role;
        assert!(!worth_summarizing(&[]));
        assert!(!worth_summarizing(&[msg(Role::User, "你好")]));
        // 够多但全是空内容（只有工具调用的轮次）
        let blanks: Vec<Message> = (0..6).map(|_| msg(Role::Assistant, "  ")).collect();
        assert!(!worth_summarizing(&blanks));
    }

    #[test]
    fn a_real_batch_is_worth_summarizing() {
        use crate::llm::Role;
        let batch = vec![
            msg(Role::User, "帮我看看首包时间"),
            msg(Role::Assistant, "TTS 占大头"),
            msg(Role::User, "那怎么优化"),
            msg(Role::Assistant, "降 nfe"),
        ];
        assert!(worth_summarizing(&batch));
    }

    #[test]
    fn the_prompt_carries_the_previous_summary_forward() {
        // **摘要必须是累积的。** 只压新增部分的话，第二次裁剪会把旧摘要丢掉 ——
        // 那正是这个模块要修的毛病（越聊越忘）
        use crate::llm::Role;
        let previous = Summary {
            text: "用户在调 TTS 引擎。".into(),
            focus: vec!["对比 nfe 8 和 16".into()],
        };
        let dropped = vec![msg(Role::User, "换到 F5-TTS"), msg(Role::Assistant, "好")];
        let p = prompt(&previous, &dropped);
        assert!(p.contains("用户在调 TTS 引擎。"), "旧摘要要带进提示词");
        assert!(p.contains("对比 nfe 8 和 16"), "旧待办也要带进去");
        assert!(p.contains("换到 F5-TTS"), "新对话要在里面");
        assert!(p.contains("合起来"), "要明确要求合并而不是只写新增");
        assert!(p.contains("第三人称"), "人称要求会影响它之后怎么读这段摘要");
    }

    #[test]
    fn the_prompt_skips_messages_with_no_content() {
        // 只有工具调用的轮次内容是空的，带进去只是噪音
        use crate::llm::Role;
        let dropped = vec![
            msg(Role::User, "查一下"),
            msg(Role::Assistant, ""),
            msg(Role::Assistant, "查到了"),
        ];
        let p = prompt(&Summary::default(), &dropped);
        assert!(p.contains("用户：查一下"));
        assert!(p.contains("助手：查到了"));
        assert!(!p.contains("助手：\n"), "空内容不该留下一行光秃秃的角色名");
    }
}
