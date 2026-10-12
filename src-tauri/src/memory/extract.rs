//! 从对话里提取「事实」，供长期记忆存储。
//!
//! 按 ARCHITECTURE.md 3.4：一轮对话结束后**异步**启动一个模型，把值得长期记住的
//! 事实抽成结构化数据（如 `{"subject":"用户","fact":"养了一只叫小黑的猫"}`），
//! 再向量化存进 LanceDB。
//!
//! 两个设计要点：
//!
//! 1. **走后台分层的模型**，不占实时链路。提取慢一点没关系，用户不在等它。
//! 2. **解析必须极其宽容**。本地小模型经常不老实：给 JSON 裹上 ```json 围栏、
//!    前后加一段解释、字段名大小写不一致。提取失败不能影响对话，所以任何解析
//!    问题都退化成「这轮没提取到事实」，只记一条日志。

use serde::Deserialize;

use crate::config::Tier;
use crate::error::Result;
use crate::llm::gateway::Gateway;
use crate::llm::{ChatChunk, ChatRequest, Message, ThinkingMode};

/// 提取出的一条事实（还没有 id / 时间戳）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedFact {
    pub subject: String,
    pub fact: String,
}

/// 提取用的提示词。
///
/// 刻意写得啰嗦并给了反例 —— 本地小模型很容易把「今天天气不错」这种一次性闲聊
/// 也当成值得永久记住的事实，导致记忆库被垃圾淹没、召回质量崩掉。
const EXTRACT_PROMPT: &str = r#"你是一个记忆提取器。从下面的对话里找出**值得长期记住**的事实。

只提取这几类：
- 用户的个人信息（姓名、职业、所在地、家庭成员、宠物）
- 用户稳定的偏好与习惯（喜欢/讨厌什么、作息、口味）
- 用户正在进行的长期项目或目标
- 用户明确要求记住的事情

**不要**提取：
- 一次性的闲聊（如"今天天气不错"、"现在几点"）
- 助手自己说的话或助手的推测
- 临时的情绪状态
- 从对话里推断不出、需要猜的内容

严格只输出 JSON，不要任何解释、不要 markdown 围栏：
{"facts":[{"subject":"用户","fact":"养了一只叫小黑的猫"}]}

没有值得记的就输出：{"facts":[]}"#;

/// 在项目里工作时用的提取提示词。
///
/// ## 为什么需要第二套
///
/// 上面那套是**陪伴型**记忆的白名单：个人信息、偏好、长期目标。实测在
/// coding 会话里它的提取率接近零，而且那是**正确行为** —— 「这个项目用
/// `node --test` 不用 vitest」不属于它列的任何一类，模型老老实实输出了
/// `{"facts":[]}`。
///
/// 本机真实数据：40 条被处理过的消息里 37 条提取出 0 条事实，唯一具体的
/// 一条是「我們有西瓜汁，很好喝哦」。问题不在模型，在白名单的定位。
///
/// ## 排除项比包含项更重要
///
/// 没有那段排除，本地小模型会把「用户让我实现一个 LRU 缓存」记下来 ——
/// 一次性的任务描述，下次召回时纯属干扰。而且这类内容特别容易被提取：
/// 它就在对话最显眼的位置。
const EXTRACT_PROMPT_PROJECT: &str = r#"你是一个记忆提取器。当前对话发生在一个**软件项目**里。
从下面的对话里找出**下次接着做这个项目时仍然有用**的事实。

只提取这几类：
- 工具链与命令约定（怎么跑测试、怎么构建、用哪个包管理器、装了什么依赖）
- 技术选型及其理由（"用 X 而不是 Y，因为……"）
- 明确的约束与禁忌（"不要改这个文件"、"这里必须保持向后兼容"）
- 用户对这个项目的偏好（代码风格、注释语言、提交信息格式）
- 踩过的坑及其结论（"在这台机器上 Z 跑不起来，要换成 W"）

**不要**提取：
- 代码里直接读得到的东西（目录结构、函数签名、某个文件有多少行）——
  那些下次读代码就知道，记下来只会过时
- 一次性的任务描述（"帮我实现一个 LRU 缓存"、"修一下这个 bug"）
- 工具的输出、报错原文、中间过程
- 助手自己的计划或推测
- 需要猜的内容

每条都要能独立看懂：写"测试用 npm test（node --test，零依赖）"，
不要写"用上面说的那个命令跑测试"。

严格只输出 JSON，不要任何解释、不要 markdown 围栏：
{"facts":[{"subject":"项目","fact":"测试用 npm test，底层是 node --test，不装 vitest"}]}

没有值得记的就输出：{"facts":[]}"#;

#[derive(Deserialize)]
struct FactsEnvelope {
    #[serde(default, alias = "Facts")]
    facts: Vec<RawFact>,
}

#[derive(Deserialize)]
struct RawFact {
    #[serde(default, alias = "Subject")]
    subject: Option<String>,
    #[serde(default, alias = "Fact", alias = "content", alias = "text")]
    fact: Option<String>,
}

/// 事实内容的长度上限。超长的基本是模型把整段对话复述了一遍，
/// 存进去只会污染召回。
const MAX_FACT_CHARS: usize = 200;

/// 从模型的原始输出里尽力解析出事实列表。
///
/// 解析不出来就返回空 —— 记忆提取是「有则更好」的增强，绝不能因为它失败
/// 而影响对话。
pub fn parse_facts(raw: &str) -> Vec<ExtractedFact> {
    let Some(json) = extract_json_object(raw) else {
        tracing::debug!(raw = %truncate(raw, 300), "提取输出里找不到 JSON 对象");
        return Vec::new();
    };

    let envelope: FactsEnvelope = match serde_json::from_str(&json) {
        Ok(e) => e,
        Err(err) => {
            tracing::debug!(%err, json = %truncate(&json, 300), "事实 JSON 解析失败");
            return Vec::new();
        }
    };

    let mut out: Vec<ExtractedFact> = Vec::new();
    for raw_fact in envelope.facts {
        let fact = raw_fact.fact.unwrap_or_default().trim().to_string();
        if fact.is_empty() || fact.chars().count() > MAX_FACT_CHARS {
            continue;
        }
        let subject = raw_fact.subject.unwrap_or_default().trim().to_string();

        out.push(ExtractedFact {
            // 主体缺失时按「用户」兜底 —— 绝大多数事实都是关于用户的
            subject: if subject.is_empty() {
                "用户".to_string()
            } else {
                subject
            },
            fact,
        });
    }

    // 同一轮里模型偶尔会把同一件事重复输出。用 retain 而非 dedup：
    // dedup 只能去掉**相邻**重复，模型的重复往往不相邻。
    let mut seen = std::collections::HashSet::new();
    out.retain(|f| seen.insert(f.fact.clone()));
    out
}

/// 从可能夹带解释文字/markdown 围栏的输出里抠出第一个完整 JSON 对象。
///
/// 用括号配对扫描而不是正则：事实内容里可能带 `{}`，正则很容易截断。
/// 同时跳过字符串字面量内部的括号与转义。
fn extract_json_object(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let start = raw.find('{')?;

    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (offset, &byte) in bytes.iter().enumerate().skip(start) {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }

        match byte {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    // offset 落在 `}` 上，切片要含它
                    return Some(raw[start..=offset].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        text.chars().take(max).collect::<String>() + "…"
    }
}

/// 调后台分层的模型做一次提取。
/// 按是否在项目里选提取提示词。
///
/// 单独一个函数是为了能被测试直接断言 —— 两套提示词的**区别**才是这里的
/// 实质内容，而那个区别很容易在编辑中被抹平（例如复制粘贴时漏掉排除段）。
fn prompt_for(in_project: bool) -> &'static str {
    if in_project {
        EXTRACT_PROMPT_PROJECT
    } else {
        EXTRACT_PROMPT
    }
}

pub async fn extract(
    gateway: &Gateway,
    user_text: &str,
    assistant_text: &str,
    in_project: bool,
) -> Result<Vec<ExtractedFact>> {
    let conversation = format!("用户：{user_text}\n助手：{assistant_text}");

    let mut req = ChatRequest::new(vec![
        Message::system(prompt_for(in_project)),
        Message::user(conversation),
    ]);
    // 提取要的是稳定可复现的结构化输出，不要创造性
    req.temperature = Some(0.0);
    req.max_tokens = Some(512);
    // 思考对提取没帮助，只会拖慢并吃掉 token 预算
    req.thinking = ThinkingMode::Off;

    let stream_id = format!("memory-extract-{}", uuid::Uuid::new_v4());
    let (_, mut stream) = gateway
        .chat_stream(stream_id, Tier::Background, req)
        .await?;

    use futures_util::StreamExt;
    let mut text = String::new();
    while let Some(chunk) = stream.next().await {
        match chunk? {
            ChatChunk::Text { delta } => text.push_str(&delta),
            ChatChunk::Done { .. } => break,
            _ => {}
        }
    }

    Ok(parse_facts(&text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(subject: &str, text: &str) -> ExtractedFact {
        ExtractedFact {
            subject: subject.into(),
            fact: text.into(),
        }
    }

    #[test]
    fn parses_clean_json() {
        let raw = r#"{"facts":[{"subject":"用户","fact":"养了一只叫小黑的猫"}]}"#;
        assert_eq!(parse_facts(raw), vec![fact("用户", "养了一只叫小黑的猫")]);
    }

    #[test]
    fn strips_markdown_fences() {
        // 本地模型最常见的不老实形式
        let raw = "```json\n{\"facts\":[{\"subject\":\"用户\",\"fact\":\"住在上海\"}]}\n```";
        assert_eq!(parse_facts(raw), vec![fact("用户", "住在上海")]);
    }

    #[test]
    fn ignores_prose_around_the_json() {
        let raw = "好的，我分析了对话，提取到以下事实：\n\
                   {\"facts\":[{\"subject\":\"用户\",\"fact\":\"是程序员\"}]}\n\
                   希望这对你有帮助！";
        assert_eq!(parse_facts(raw), vec![fact("用户", "是程序员")]);
    }

    #[test]
    fn handles_braces_inside_fact_text() {
        // 事实内容里带花括号时，括号配对扫描不能被带偏
        let raw = r#"{"facts":[{"subject":"用户","fact":"在写一个用 {} 做占位的模板引擎"}]}"#;
        let facts = parse_facts(raw);
        assert_eq!(facts.len(), 1);
        assert!(facts[0].fact.contains("{}"), "实际：{}", facts[0].fact);
    }

    #[test]
    fn handles_escaped_quotes_inside_strings() {
        let raw = r#"{"facts":[{"subject":"用户","fact":"说过\"我讨厌香菜\""}]}"#;
        let facts = parse_facts(raw);
        assert_eq!(facts.len(), 1);
        assert!(facts[0].fact.contains("香菜"), "实际：{}", facts[0].fact);
    }

    #[test]
    fn empty_facts_array_yields_nothing() {
        assert!(parse_facts(r#"{"facts":[]}"#).is_empty());
    }

    #[test]
    fn tolerates_alternate_field_names() {
        // 模型偶尔用大写字段名或 content/text 代替 fact
        let raw = r#"{"Facts":[{"Subject":"用户","content":"喜欢喝手冲咖啡"}]}"#;
        assert_eq!(parse_facts(raw), vec![fact("用户", "喜欢喝手冲咖啡")]);
    }

    #[test]
    fn defaults_missing_subject_to_user() {
        let raw = r#"{"facts":[{"fact":"每天六点起床"}]}"#;
        assert_eq!(parse_facts(raw), vec![fact("用户", "每天六点起床")]);
    }

    #[test]
    fn drops_empty_and_overlong_facts() {
        let long = "很长".repeat(200);
        let raw = format!(
            r#"{{"facts":[{{"fact":""}},{{"fact":"   "}},{{"fact":"{long}"}},{{"fact":"有效事实"}}]}}"#
        );
        // 空的、纯空白的、超长的都要丢掉，只留有效那条
        assert_eq!(parse_facts(&raw), vec![fact("用户", "有效事实")]);
    }

    #[test]
    fn garbage_input_yields_nothing_instead_of_panicking() {
        // 提取失败绝不能影响对话，所以这些都必须安静地返回空
        for raw in [
            "",
            "我不知道该提取什么",
            "{",
            "{\"facts\":",
            "{\"facts\":\"不是数组\"}",
            "null",
            "[]",
        ] {
            assert!(parse_facts(raw).is_empty(), "输入「{raw}」应当返回空");
        }
    }

    #[test]
    fn deduplicates_repeated_facts_even_when_not_adjacent() {
        let raw = r#"{"facts":[
            {"fact":"养猫"},{"fact":"养狗"},{"fact":"养猫"}
        ]}"#;
        let facts = parse_facts(raw);
        assert_eq!(facts.len(), 2, "非相邻重复也要去掉：{facts:?}");
    }

    #[test]
    fn extract_json_object_finds_first_complete_object() {
        let raw = "前缀 {\"a\":{\"b\":1}} 后缀 {\"c\":2}";
        assert_eq!(
            extract_json_object(raw).as_deref(),
            Some(r#"{"a":{"b":1}}"#)
        );
    }

    #[test]
    fn extract_json_object_returns_none_when_unbalanced() {
        assert_eq!(extract_json_object("{\"a\":1"), None);
        assert_eq!(extract_json_object("没有括号"), None);
    }

    #[test]
    fn brace_inside_string_does_not_close_the_object() {
        // 字符串里的 `}` 不能被当成结束符
        let raw = r#"{"fact":"结尾有个 } 符号","ok":true}"#;
        assert_eq!(extract_json_object(raw).as_deref(), Some(raw));
    }

    #[test]
    fn 项目里和项目外用的是两套白名单() {
        assert_ne!(prompt_for(true), prompt_for(false));
        // 陪伴型那套收的是"关于人"的
        assert!(prompt_for(false).contains("个人信息"));
        assert!(prompt_for(false).contains("宠物"));
        // 项目那套收的是"关于这个仓库"的
        assert!(prompt_for(true).contains("工具链与命令约定"));
        assert!(prompt_for(true).contains("技术选型"));
        // 而且不能反过来 —— 项目里提取"养了只猫"没有意义
        assert!(!prompt_for(true).contains("宠物"));
    }

    #[test]
    fn 项目白名单必须带排除段() {
        // **排除项比包含项更重要。** 没有它，本地小模型会把
        // "帮我实现一个 LRU 缓存"这种一次性任务描述记进长期记忆 ——
        // 它就在对话最显眼的位置，特别容易被提取，而下次召回时纯属干扰。
        let p = prompt_for(true);
        assert!(p.contains("**不要**提取"), "项目提示词丢了排除段");
        assert!(p.contains("一次性的任务描述"));
        // 代码里读得到的东西记下来只会过时
        assert!(p.contains("代码里直接读得到"));
        // 两套都要求纯 JSON —— 解析器对两者是同一套
        for p in [prompt_for(true), prompt_for(false)] {
            assert!(p.contains(r#"{"facts":[]}"#), "少了空结果的样例");
            assert!(p.contains("不要 markdown 围栏"));
        }
    }
}
