//! Ollama 原生 `/api/chat` 适配器。
//!
//! 为什么不复用 `openai.rs` 走 Ollama 的 OpenAI 兼容层：
//! `/v1/chat/completions` 在 `stream=true` 时会静默丢弃 tool_call
//! （见 docs/DECISIONS.md）。静默丢弃比报错更糟 —— 模型看起来"忘了用工具"，
//! 排查时很难联想到是兼容层的问题。所以这里直接对接原生协议。
//!
//! 协议差异：响应是 NDJSON（一行一个完整 JSON），不是 SSE；
//! 工具调用一次性给全（不分片），所以不需要累积器。

use async_trait::async_trait;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};

use super::sse::{decode_response, ensure_success, Frame};
use super::{
    ChatChunk, ChatRequest, ChatStream, FinishReason, LlmConnector, Message, Role,
    Tool, ToolCall, Usage,
};
use crate::error::{Error, Result};

pub struct OllamaConnector {
    base_url: String,
    model: String,
    http: reqwest::Client,
}

impl OllamaConnector {
    pub fn new(
        base_url: impl Into<String>,
        model: impl Into<String>,
        http: reqwest::Client,
    ) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            model: model.into(),
            http,
        }
    }

    fn body(&self, req: &ChatRequest) -> Value {
        let mut body = json!({
            "model": self.model,
            "messages": req.messages.iter().map(encode_message).collect::<Vec<_>>(),
            "stream": true,
        });

        if !req.tools.is_empty() {
            body["tools"] = Value::Array(req.tools.iter().map(encode_tool).collect());
        }

        // 原生协议的思考开关。实测 qwen3:32b 加上 think:false 之后
        // 响应里不再有 thinking 字段，正文立刻开始。
        //
        // 原生协议只有布尔没有档位，档位怎么塌缩成布尔在 thinking 模块里判 ——
        // 不要在这里写第二套规则。
        if let Some(think) = crate::llm::thinking::ollama_think(req.thinking, &self.model) {
            body["think"] = json!(think);
        }

        if req.json_output {
            // 原生协议的 JSON 约束。见 `ChatRequest::json_output`；给了 schema 就按 schema 约束（Ollama 的 format 认 JSON schema）
            body["format"] = req.json_schema.clone().unwrap_or_else(|| json!("json"));
        }

        // Ollama 把采样参数放在 options 子对象里，不是顶层
        let mut options = serde_json::Map::new();
        // 关键：给 Ollama 设置充足的上下文窗口 (num_ctx: 16384)，
        // 默认的 2048 太小，在包含联网搜索结果时极易在输出中途截断！
        options.insert("num_ctx".into(), json!(16384));
        if let Some(t) = req.temperature {
            options.insert("temperature".into(), json!(t));
        }
        if let Some(m) = req.max_tokens {
            // Ollama 里叫 num_predict
            options.insert("num_predict".into(), json!(m));
        }
        if !options.is_empty() {
            body["options"] = Value::Object(options);
        }
        body
    }
}

fn encode_message(msg: &Message) -> Value {
    let role = match msg.role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    };

    let mut out = json!({ "role": role, "content": msg.content });
    if !msg.images.is_empty() {
        let b64_images: Vec<String> = msg
            .images
            .iter()
            .map(|img| {
                if let Some(rest) = img.strip_prefix("data:") {
                    if let Some((_, b64)) = rest.split_once(";base64,") {
                        return b64.to_string();
                    }
                }
                img.clone()
            })
            .collect();
        out["images"] = json!(b64_images);
    }

    if !msg.tool_calls.is_empty() {
        out["tool_calls"] = Value::Array(
            msg.tool_calls
                .iter()
                .map(|c| {
                    // 原生协议要求 arguments 是对象，不是 JSON 字符串
                    let args: Value =
                        serde_json::from_str(&c.arguments).unwrap_or_else(|_| json!({}));
                    json!({ "function": { "name": c.name, "arguments": args } })
                })
                .collect(),
        );
    }
    out
}

fn encode_tool(tool: &Tool) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": tool.name,
            "description": tool.description,
            "parameters": tool.parameters,
        }
    })
}

#[derive(Deserialize)]
struct ChatResponse {
    #[serde(default)]
    message: Option<ResponseMessage>,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    done_reason: Option<String>,
    #[serde(default)]
    prompt_eval_count: Option<u32>,
    #[serde(default)]
    eval_count: Option<u32>,
    /// 出错时 Ollama 在 200 的流里返回 {"error": "..."}
    #[serde(default)]
    error: Option<String>,
}

#[derive(Deserialize)]
struct ResponseMessage {
    #[serde(default)]
    content: String,
    /// Qwen3 等带 thinking 的模型会填这个字段
    #[serde(default)]
    thinking: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ResponseToolCall>,
}

#[derive(Deserialize)]
struct ResponseToolCall {
    function: ResponseFunction,
}

#[derive(Deserialize)]
struct ResponseFunction {
    name: String,
    /// 原生协议这里已经是解析好的对象
    #[serde(default)]
    arguments: Value,
}

fn into_chunks(
    frames: futures_util::stream::BoxStream<'static, Result<Frame>>,
) -> impl futures_util::Stream<Item = Result<ChatChunk>> + Send {
    struct State {
        frames: futures_util::stream::BoxStream<'static, Result<Frame>>,
        pending: std::collections::VecDeque<Result<ChatChunk>>,
        /// Ollama 原生协议不给 tool_call id，自己编一个，
        /// 供上层把工具结果和调用对应起来。
        next_call_seq: usize,
        closed: bool,
    }

    let state = State {
        frames,
        pending: std::collections::VecDeque::new(),
        next_call_seq: 0,
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

            let payload = match st.frames.next().await {
                Some(Err(err)) => {
                    st.closed = true;
                    return Some((Err(err), st));
                }
                Some(Ok(Frame::Data(p))) => p,
                // Ollama 用 done:true 收尾；连接直接断掉说明是异常中断
                Some(Ok(Frame::Done)) | None => {
                    st.closed = true;
                    st.pending.push_back(Ok(ChatChunk::Done {
                        reason: FinishReason::Stop,
                        usage: None,
                    }));
                    continue;
                }
            };

            let resp: ChatResponse = match serde_json::from_str(&payload) {
                Ok(r) => r,
                Err(err) => {
                    tracing::debug!(%err, payload = %payload, "跳过无法解析的流式帧");
                    continue;
                }
            };

            if let Some(err) = resp.error {
                st.closed = true;
                return Some((Err(Error::upstream(200, err)), st));
            }

            if let Some(msg) = resp.message {
                if let Some(thinking) = msg.thinking.filter(|t| !t.is_empty()) {
                    st.pending
                        .push_back(Ok(ChatChunk::Reasoning { delta: thinking }));
                }
                if !msg.content.is_empty() {
                    st.pending
                        .push_back(Ok(ChatChunk::Text { delta: msg.content }));
                }
                for call in msg.tool_calls {
                    st.next_call_seq += 1;
                    st.pending.push_back(Ok(ChatChunk::ToolCall {
                        call: ToolCall {
                            id: format!("ollama-call-{}", st.next_call_seq),
                            name: call.function.name,
                            arguments: call.function.arguments.to_string(),
                        },
                    }));
                }
            }

            if resp.done {
                st.closed = true;
                st.pending.push_back(Ok(ChatChunk::Done {
                    reason: match resp.done_reason.as_deref() {
                        Some("length") => FinishReason::Length,
                        _ => FinishReason::Stop,
                    },
                    usage: Some(Usage {
                        // Ollama 原生协议不给思考 token 数
                        reasoning_tokens: None,
                        prompt_tokens: resp.prompt_eval_count.unwrap_or(0),
                        completion_tokens: resp.eval_count.unwrap_or(0),
                    }),
                }));
            }
        }
    })
}

#[async_trait]
impl LlmConnector for OllamaConnector {
    fn id(&self) -> &str {
        "ollama"
    }

    fn model(&self) -> &str {
        &self.model
    }

    async fn chat_stream(&self, req: ChatRequest) -> Result<ChatStream> {
        let resp = self
            .http
            .post(format!("{}/api/chat", self.base_url))
            .json(&self.body(&req))
            .send()
            .await?;
        let resp = ensure_success(resp).await?;

        Ok(into_chunks(decode_response(resp).boxed()).boxed())
    }

    async fn health_check(&self) -> Result<Vec<String>> {
        let resp = self
            .http
            .get(format!("{}/api/tags", self.base_url))
            .send()
            .await?;
        let resp = ensure_success(resp).await?;

        #[derive(Deserialize)]
        struct TagsResponse {
            #[serde(default)]
            models: Vec<TagEntry>,
        }
        #[derive(Deserialize)]
        struct TagEntry {
            name: String,
        }

        let parsed: TagsResponse = resp.json().await?;
        Ok(parsed.models.into_iter().map(|m| m.name).collect())
    }
}
