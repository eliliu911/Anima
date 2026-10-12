//! Claude (Anthropic Messages API) 适配器。
//!
//! 与 OpenAI 的三个关键差异，也是这个文件存在的原因：
//!   1. system prompt 是顶层字段，不是 messages 里的一条；
//!   2. 流式协议是带 `event:` 类型的 SSE，工具参数走 `input_json_delta` 增量；
//!   3. 工具结果作为 user 轮次里的 `tool_result` content block 回填，没有 role=tool。

use async_trait::async_trait;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};

use super::sse::{decode_response, ensure_success, Frame};
use super::{
    ChatChunk, ChatRequest, ChatStream, FinishReason, LlmConnector, Message, Role, Tool, ToolCall,
    Usage,
};
use crate::error::{Error, Result};

/// Anthropic 要求显式声明 API 版本
const API_VERSION: &str = "2023-06-01";

/// 扩展思考的最小预算，Anthropic API 的硬下限。低于它请求会被拒。
const MIN_THINKING_BUDGET: u32 = 1024;

/// Messages API 强制要求 max_tokens，未指定时给一个够用的默认值
const DEFAULT_MAX_TOKENS: u32 = 4096;

pub struct AnthropicConnector {
    base_url: String,
    model: String,
    api_key: String,
    http: reqwest::Client,
}

impl AnthropicConnector {
    pub fn new(
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key: impl Into<String>,
        http: reqwest::Client,
    ) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            model: model.into(),
            api_key: api_key.into(),
            http,
        }
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{}{}", self.base_url, path))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
    }

    fn body(&self, req: &ChatRequest) -> Value {
        let (system, turns) = split_system(&req.messages);

        let mut body = json!({
            "model": self.model,
            "messages": turns,
            "stream": true,
            "max_tokens": req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
        });

        if !system.is_empty() {
            body["system"] = json!(system);
        }
        if !req.tools.is_empty() {
            body["tools"] = Value::Array(req.tools.iter().map(encode_tool).collect());
        }
        // 扩展思考。和 temperature 是 if/else 而不是并列：开了 thinking 之后
        // Anthropic 要求 temperature 必须是 1，传别的值直接 400。
        //
        // 两条硬约束：`budget_tokens` 必须小于 `max_tokens`（否则 400），
        // 而且不能小于 1024（API 的下限）。所以思考最多占一半 ——
        // 只满足「小于 max_tokens」是不够的，budget=1024 / max=1500 虽然合法，
        // 但只剩 476 token 写正文，等于开了个没用的思考。
        let requested = crate::llm::thinking::anthropic_budget_tokens(req.thinking);
        let max_tokens = req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS);
        let granted = requested
            .map(|b| b.min(max_tokens / 2))
            .filter(|b| *b >= MIN_THINKING_BUDGET);

        // Claude 5 代起整个不收 temperature（发 1 也 400），两条分支都不发（见 `thinking::accepts_temperature`）
        let temperature_ok = crate::llm::thinking::accepts_temperature(&self.model);
        match granted {
            Some(budget) => {
                body["thinking"] = json!({ "type": "enabled", "budget_tokens": budget });
                if temperature_ok {
                    body["temperature"] = json!(1);
                }
            }
            // 挤不出下限就退回不思考，保留调用方设的 temperature
            None => {
                if let Some(t) = req.temperature.filter(|_| temperature_ok) {
                    body["temperature"] = json!(t);
                }
            }
        }
        body
    }
}

/// 把 system 消息提取到顶层，其余消息转成 Anthropic 的 content block 格式。
///
/// 多条 system 消息（性格引擎可能分段拼接：人设 + 当前情绪 + 记忆召回）
/// 用换行合并成一条。
fn split_system(messages: &[Message]) -> (String, Vec<Value>) {
    let mut system = Vec::new();
    let mut turns: Vec<Value> = Vec::new();

    for msg in messages {
        match msg.role {
            Role::System => system.push(msg.content.clone()),

            Role::User => {
                let mut blocks = vec![json!({ "type": "text", "text": msg.content })];
                for img in &msg.images {
                    let (media_type, data) = if let Some(rest) = img.strip_prefix("data:") {
                        if let Some((m_type, b64)) = rest.split_once(";base64,") {
                            (m_type, b64)
                        } else {
                            ("image/png", rest)
                        }
                    } else {
                        ("image/png", img.as_str())
                    };
                    blocks.push(json!({
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": media_type,
                            "data": data
                        }
                    }));
                }
                // **连续两条 user 必须并进同一轮。**
                //
                // Anthropic 要求角色严格交替。工具结果是挂在 user 轮次里的
                // （见下面的 `Role::Tool`），所以「assistant(tool_use) →
                // tool 结果 → 用户的下一句」这条再普通不过的历史，展开后就是
                // 两条连着的 user，请求会被上游直接拒掉。
                //
                // 前端把最近两轮工具调用按协议原样回填之后，这条路径每次
                // 多轮对话都会走到。
                match turns.last_mut() {
                    Some(last) if last["role"] == "user" => {
                        if let Some(arr) = last["content"].as_array_mut() {
                            arr.extend(blocks);
                        }
                    }
                    _ => turns.push(json!({ "role": "user", "content": blocks })),
                }
            }

            Role::Assistant => {
                let mut blocks = Vec::new();
                if !msg.content.is_empty() {
                    blocks.push(json!({ "type": "text", "text": msg.content }));
                }
                for call in &msg.tool_calls {
                    // 回填历史时参数必须是 JSON 对象；解析失败就退化成空对象，
                    // 否则整个请求会被上游拒掉。
                    let input: Value =
                        serde_json::from_str(&call.arguments).unwrap_or_else(|_| json!({}));
                    blocks.push(json!({
                        "type": "tool_use",
                        "id": call.id,
                        "name": call.name,
                        "input": input,
                    }));
                }
                turns.push(json!({ "role": "assistant", "content": blocks }));
            }

            // Anthropic 没有 role=tool，工具结果挂在 user 轮次里
            Role::Tool => {
                let block = json!({
                    "type": "tool_result",
                    "tool_use_id": msg.tool_call_id.clone().unwrap_or_default(),
                    "content": msg.content,
                });
                // 连续多个工具结果要合并进同一个 user 轮次，
                // 否则会出现连续两条 user 消息，上游会报错。
                match turns.last_mut() {
                    Some(last) if last["role"] == "user" => {
                        if let Some(arr) = last["content"].as_array_mut() {
                            arr.push(block);
                        }
                    }
                    _ => turns.push(json!({ "role": "user", "content": [block] })),
                }
            }
        }
    }

    (system.join("\n\n"), turns)
}

fn encode_tool(tool: &Tool) -> Value {
    json!({
        "name": tool.name,
        "description": tool.description,
        "input_schema": tool.parameters,
    })
}

// ---------------------------------------------------------------------------
// 流式事件
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum StreamEvent {
    MessageStart {
        #[serde(default)]
        message: MessageStartBody,
    },
    ContentBlockStart {
        index: usize,
        content_block: ContentBlock,
    },
    ContentBlockDelta {
        index: usize,
        delta: BlockDelta,
    },
    ContentBlockStop {
        index: usize,
    },
    MessageDelta {
        #[serde(default)]
        delta: MessageDeltaBody,
        #[serde(default)]
        usage: Option<ApiUsage>,
    },
    MessageStop,
    Ping,
    Error {
        #[serde(default)]
        error: Value,
    },
    /// 忽略将来新增的事件类型，而不是让整个流失败
    #[serde(other)]
    Unknown,
}

#[derive(Default, Deserialize)]
struct MessageStartBody {
    #[serde(default)]
    usage: Option<ApiUsage>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentBlock {
    Text,
    Thinking,
    ToolUse {
        id: String,
        name: String,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BlockDelta {
    TextDelta {
        text: String,
    },
    ThinkingDelta {
        thinking: String,
    },
    /// 工具参数的增量 JSON 字符串片段
    InputJsonDelta {
        partial_json: String,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Default, Deserialize)]
struct MessageDeltaBody {
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Default, Deserialize)]
struct ApiUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

/// 正在累积的 tool_use block
struct PartialToolUse {
    id: String,
    name: String,
    arguments: String,
}

fn into_chunks(
    frames: futures_util::stream::BoxStream<'static, Result<Frame>>,
) -> impl futures_util::Stream<Item = Result<ChatChunk>> + Send {
    struct State {
        frames: futures_util::stream::BoxStream<'static, Result<Frame>>,
        /// index -> 正在累积的工具调用。Anthropic 的 block index 是稀疏的
        /// （文本块和工具块混编），所以按 index 建表而不是用 Vec。
        tools: std::collections::BTreeMap<usize, PartialToolUse>,
        prompt_tokens: u32,
        completion_tokens: u32,
        finish: FinishReason,
        pending: std::collections::VecDeque<Result<ChatChunk>>,
        closed: bool,
    }

    let state = State {
        frames,
        tools: std::collections::BTreeMap::new(),
        prompt_tokens: 0,
        completion_tokens: 0,
        finish: FinishReason::Stop,
        pending: std::collections::VecDeque::new(),
        closed: false,
    };

    futures_util::stream::unfold(state, |mut st| async move {
        loop {
            if let Some(item) = st.pending.pop_front() {
                return Some((item, st));
            }
            if st.closed {
                return None;
            }

            let frame = st.frames.next().await;
            let payload = match frame {
                Some(Err(err)) => {
                    st.closed = true;
                    return Some((Err(err), st));
                }
                Some(Ok(Frame::Data(p))) => p,
                Some(Ok(Frame::Done)) | None => {
                    st.closed = true;
                    st.pending.push_back(Ok(ChatChunk::Done {
                        reason: st.finish,
                        usage: Some(Usage {
                            // Anthropic 的 usage 里没有这一项（思考内容走 thinking block，不单独计数）
                            reasoning_tokens: None,
                            prompt_tokens: st.prompt_tokens,
                            completion_tokens: st.completion_tokens,
                        }),
                    }));
                    continue;
                }
            };

            let event: StreamEvent = match serde_json::from_str(&payload) {
                Ok(ev) => ev,
                Err(err) => {
                    tracing::debug!(%err, payload = %payload, "跳过无法解析的流式帧");
                    continue;
                }
            };

            match event {
                StreamEvent::MessageStart { message } => {
                    if let Some(u) = message.usage {
                        st.prompt_tokens = u.input_tokens;
                    }
                }

                StreamEvent::ContentBlockStart {
                    index,
                    content_block,
                } => {
                    if let ContentBlock::ToolUse { id, name } = content_block {
                        st.tools.insert(
                            index,
                            PartialToolUse {
                                id,
                                name,
                                arguments: String::new(),
                            },
                        );
                    }
                }

                StreamEvent::ContentBlockDelta { index, delta } => match delta {
                    BlockDelta::TextDelta { text } if !text.is_empty() => {
                        st.pending.push_back(Ok(ChatChunk::Text { delta: text }));
                    }
                    BlockDelta::ThinkingDelta { thinking } if !thinking.is_empty() => {
                        st.pending
                            .push_back(Ok(ChatChunk::Reasoning { delta: thinking }));
                    }
                    BlockDelta::InputJsonDelta { partial_json } => {
                        if let Some(slot) = st.tools.get_mut(&index) {
                            slot.arguments.push_str(&partial_json);
                        }
                    }
                    _ => {}
                },

                // 一个 block 结束就可以把它作为完整工具调用发出，
                // 不用等到 message_stop —— 上层能更早开始执行工具。
                StreamEvent::ContentBlockStop { index } => {
                    if let Some(slot) = st.tools.remove(&index) {
                        st.pending.push_back(Ok(ChatChunk::ToolCall {
                            call: ToolCall {
                                id: slot.id,
                                name: slot.name,
                                // 参数为空时补一个空对象，保证下游拿到的总是合法 JSON
                                arguments: if slot.arguments.trim().is_empty() {
                                    "{}".to_string()
                                } else {
                                    slot.arguments
                                },
                            },
                        }));
                    }
                }

                StreamEvent::MessageDelta { delta, usage } => {
                    if let Some(u) = usage {
                        st.completion_tokens = u.output_tokens;
                    }
                    if let Some(reason) = delta.stop_reason {
                        st.finish = match reason.as_str() {
                            "tool_use" => FinishReason::ToolCalls,
                            "max_tokens" => FinishReason::Length,
                            // Claude 拒答（`refusal`）和 Azure 的 content_filter 是一回事：半截不能当写完了往下传
                            "refusal" => FinishReason::ContentFilter,
                            _ => FinishReason::Stop,
                        };
                    }
                }

                StreamEvent::MessageStop => {
                    st.closed = true;
                    st.pending.push_back(Ok(ChatChunk::Done {
                        reason: st.finish,
                        usage: Some(Usage {
                            // Anthropic 的 usage 里没有这一项（思考内容走 thinking block，不单独计数）
                            reasoning_tokens: None,
                            prompt_tokens: st.prompt_tokens,
                            completion_tokens: st.completion_tokens,
                        }),
                    }));
                }

                StreamEvent::Error { error } => {
                    st.closed = true;
                    return Some((Err(Error::upstream(200, error.to_string())), st));
                }

                StreamEvent::Ping | StreamEvent::Unknown => {}
            }
        }
    })
}

#[async_trait]
impl LlmConnector for AnthropicConnector {
    fn id(&self) -> &str {
        "claude"
    }

    fn model(&self) -> &str {
        &self.model
    }

    async fn chat_stream(&self, req: ChatRequest) -> Result<ChatStream> {
        if self.api_key.is_empty() {
            return Err(Error::Credential(
                "Claude 需要 API Key，请先在设置里填写".into(),
            ));
        }

        let resp = self
            .request(reqwest::Method::POST, "/v1/messages")
            .json(&self.body(&req))
            .send()
            .await?;
        let resp = ensure_success(resp).await?;

        Ok(into_chunks(decode_response(resp).boxed()).boxed())
    }

    async fn health_check(&self) -> Result<Vec<String>> {
        let resp = self
            .request(reqwest::Method::GET, "/v1/models")
            .send()
            .await?;
        let resp = ensure_success(resp).await?;

        #[derive(Deserialize)]
        struct ModelsResponse {
            #[serde(default)]
            data: Vec<ModelEntry>,
        }
        #[derive(Deserialize)]
        struct ModelEntry {
            id: String,
        }

        let parsed: ModelsResponse = resp.json().await?;
        Ok(parsed.data.into_iter().map(|m| m.id).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ThinkingEffort;

    /// 「assistant(tool_use) → 工具结果 → 用户下一句」是最普通的一段历史，
    /// 但工具结果在 Anthropic 这边挂在 user 轮次里，展开后就是两条连着的
    /// user —— 角色不再交替，上游直接拒。必须并成一轮。
    #[test]
    fn consecutive_user_turns_are_merged() {
        let mut call = Message::assistant("在生成");
        call.tool_calls.push(ToolCall {
            id: "call-1".into(),
            name: "generate_image".into(),
            arguments: "{}".into(),
        });
        let mut result = Message::plain_tool("图已生成");
        result.tool_call_id = Some("call-1".into());

        let (_sys, turns) = split_system(&[
            Message::user("画一张"),
            call,
            result,
            Message::user("裙子改白色"),
        ]);

        let roles: Vec<&str> = turns.iter().map(|t| t["role"].as_str().unwrap()).collect();
        assert_eq!(
            roles,
            vec!["user", "assistant", "user"],
            "角色必须严格交替，实际：{roles:?}"
        );

        // 合并不能吞内容：工具结果和用户新问题都要在最后那一轮里
        let last = turns.last().unwrap()["content"].as_array().unwrap();
        let kinds: Vec<&str> = last.iter().map(|b| b["type"].as_str().unwrap()).collect();
        assert_eq!(kinds, vec!["tool_result", "text"]);
        assert_eq!(last[1]["text"], "裙子改白色");
    }

    /// Opus 5.5 收到 temperature 就 400（「deprecated for this model」，2026-09-29 实测）—— 思考开没开都不发；4.5 照常发
    #[test]
    fn claude_5_代起不发温度() {
        let opus = AnthropicConnector::new("https://x/anthropic", "claude-opus-5-5", "k", reqwest::Client::new());
        let mut req = ChatRequest::new(vec![Message::user("hi")]);
        req.temperature = Some(0.0);
        assert!(opus.body(&req).get("temperature").is_none());
        req.thinking = ThinkingEffort::High;
        req.max_tokens = Some(16000);
        let b = opus.body(&req);
        assert!(b.get("thinking").is_some() && b.get("temperature").is_none(), "{b}");
        req.thinking = ThinkingEffort::Off;
        assert_eq!(conn().body(&req)["temperature"], 0.0, "4.5 照常发");
    }

    fn conn() -> AnthropicConnector {
        AnthropicConnector::new(
            "https://api.anthropic.com",
            "claude-sonnet-4-5",
            "k",
            reqwest::Client::new(),
        )
    }

    fn req_with(effort: ThinkingEffort, max_tokens: Option<u32>) -> ChatRequest {
        let mut r = ChatRequest::new(vec![Message::user("hi")]);
        r.thinking = effort;
        r.temperature = Some(0.7);
        r.max_tokens = max_tokens;
        r
    }

    #[test]
    fn 关和auto都不发thinking块() {
        for e in [ThinkingEffort::Off, ThinkingEffort::Auto] {
            let b = conn().body(&req_with(e, None));
            assert!(b.get("thinking").is_none(), "{e:?}");
            // 没开思考时 temperature 要照常带上
            assert!(
                (b["temperature"].as_f64().unwrap() - 0.7).abs() < 1e-6,
                "{e:?}"
            );
        }
    }

    #[test]
    fn 开了思考必须把temperature改成1() {
        // Anthropic 的硬要求：thinking 开启时 temperature 只能是 1，
        // 传 0.7 会直接 400。
        let b = conn().body(&req_with(ThinkingEffort::Medium, Some(8192)));
        assert_eq!(b["thinking"]["type"], "enabled");
        assert_eq!(b["temperature"], 1);
    }

    #[test]
    fn 预算必须小于max_tokens否则会400() {
        let b = conn().body(&req_with(ThinkingEffort::High, Some(8192)));
        let budget = b["thinking"]["budget_tokens"].as_u64().unwrap();
        assert!(budget < 8192, "budget {budget} 必须小于 max_tokens");
        assert!(budget >= 1024, "Anthropic 的最小可用预算是 1024");
    }

    #[test]
    fn max_tokens太小时宁可不开思考也不发非法请求() {
        // 挤不出 1024 的预算就不开 —— 发一个 budget >= max_tokens 的请求
        // 只会拿到 400，还不如退回不思考。
        let b = conn().body(&req_with(ThinkingEffort::High, Some(1500)));
        assert!(b.get("thinking").is_none(), "{b}");
        assert!(
            (b["temperature"].as_f64().unwrap() - 0.7).abs() < 1e-6,
            "退回时 temperature 要保留原值"
        );
    }

    #[test]
    fn hoists_system_prompt_out_of_messages() {
        let (system, turns) = split_system(&[
            Message::system("你是 Anima"),
            Message::system("当前心情：愉快"),
            Message::user("你好"),
        ]);
        assert_eq!(system, "你是 Anima\n\n当前心情：愉快");
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0]["role"], "user");
    }

    #[test]
    fn merges_consecutive_tool_results_into_one_user_turn() {
        let turns = split_system(&[
            Message::user("查一下天气"),
            Message {
                role: Role::Assistant,
                content: String::new(),
                tool_calls: vec![
                    ToolCall {
                        id: "a".into(),
                        name: "weather".into(),
                        arguments: "{}".into(),
                    },
                    ToolCall {
                        id: "b".into(),
                        name: "time".into(),
                        arguments: "{}".into(),
                    },
                ],
                tool_call_id: None,
                images: Vec::new(),
            },
            Message {
                role: Role::Tool,
                content: "晴".into(),
                tool_calls: vec![],
                tool_call_id: Some("a".into()),
                images: Vec::new(),
            },
            Message {
                role: Role::Tool,
                content: "12:00".into(),
                tool_calls: vec![],
                tool_call_id: Some("b".into()),
                images: Vec::new(),
            },
        ])
        .1;

        // user / assistant / user(两个 tool_result 合并)
        assert_eq!(turns.len(), 3);
        assert_eq!(turns[2]["content"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn assistant_tool_call_with_invalid_json_falls_back_to_empty_object() {
        let turns = split_system(&[Message {
            role: Role::Assistant,
            content: String::new(),
            images: Vec::new(),
            // 流被打断时可能只攒到半截参数
            tool_calls: vec![ToolCall {
                id: "x".into(),
                name: "f".into(),
                arguments: "{\"half".into(),
            }],
            tool_call_id: None,
        }])
        .1;

        assert_eq!(turns[0]["content"][0]["input"], json!({}));
    }
}
