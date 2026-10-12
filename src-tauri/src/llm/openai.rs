//! OpenAI `/v1/chat/completions` 适配器。
//!
//! 同一份实现服务三类后端，差别只在 base_url / 鉴权 / 能力声明：
//!   - OpenAI 官方
//!   - LM Studio（默认本地后端，OpenAI 兼容层成熟，流式 tool_call 可用）
//!   - MLX server（`mlx_lm.server`，工具调用不成熟，构造时声明 supports_tools=false）
//!
//! 刻意**不**用于 Ollama：其 OpenAI 兼容层在 stream=true 时会静默丢弃 tool_call
//! （见 docs/DECISIONS.md），Ollama 走 `ollama.rs` 的原生 `/api/chat`。

use async_trait::async_trait;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

use super::sse::{decode_response, ensure_success, Frame};
use super::{
    ChatChunk, ChatRequest, ChatStream, FinishReason, LlmConnector, Message, Role, ThinkingMode,
    Tool, ToolCall, Usage,
};
use crate::error::{Error, Result};

pub struct OpenAiConnector {
    id: String,
    base_url: String,
    model: String,
    api_key: Option<String>,
    supports_tools: bool,
    http: reqwest::Client,
}

impl OpenAiConnector {
    pub fn new(
        id: impl Into<String>,
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key: Option<String>,
        supports_tools: bool,
        http: reqwest::Client,
    ) -> Self {
        Self {
            id: id.into(),
            // 统一去掉尾斜杠，免得拼出 `//v1/chat/completions`
            base_url: base_url.into().trim_end_matches('/').to_string(),
            model: model.into(),
            api_key,
            supports_tools,
            http,
        }
    }

    /// 查这个后端有哪些模型支持视觉（能吃图）。
    ///
    /// **走 LM Studio 自己的 `/api/v0/models`，不是 OpenAI 兼容的 `/v1/models`。**
    /// 前者的每条记录带一个 `type` 字段：`"vlm"` 表示视觉语言模型、`"llm"` 表示纯文本。
    /// 后者只有 `id`，什么都看不出来。
    ///
    /// **不能靠模型名推断。** 实测同一个基座 `qwen/qwen3.8-27b` 是 `vlm`、
    /// 而 `qwen3.8-27b-uncensored` 是 `llm` —— 差别只在打包时有没有把
    /// `mmproj`（视觉编码器 + 投影层）一起放进来，名字上看不出来。
    /// 我用扫磁盘上 mmproj 文件的方式独立验过，两边结果完全一致。
    ///
    /// 返回 `None` 表示**查不到**（这个后端没有这个接口），不是"没有视觉模型" ——
    /// 这两件事必须分开，否则前端会对着云端后端谎报"不支持视觉"。
    pub(crate) async fn vision_models(&self) -> Option<Vec<String>> {
        #[derive(Deserialize)]
        struct Entry {
            id: String,
            #[serde(default)]
            r#type: Option<String>,
        }
        #[derive(Deserialize)]
        struct Resp {
            #[serde(default)]
            data: Vec<Entry>,
        }
        let resp = self
            .authorized(self.http.get(self.endpoint("/api/v0/models")))
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let body: Resp = resp.json().await.ok()?;
        Some(
            body.data
                .into_iter()
                .filter(|e| e.r#type.as_deref() == Some("vlm"))
                .map(|e| e.id)
                .collect(),
        )
    }

    /// 这个后端**当前加载的**模型有多大上下文窗口（token）。
    ///
    /// # 为什么必须读，不能让用户填
    ///
    /// 历史预算原来是写死的 48000 **字符**，而窗口是 **token** ——
    /// 单位就不是一个东西。中文实测约 1.92 字符/token，48000 字符
    /// 大约 25000 token；再加 system prompt、技能正文、工具定义和
    /// 留给回复的余量，撞穿 33024 的窗口毫无难度。
    /// 实测症状是上游直接返回一个 HTML 的 500，而不是结构化错误。
    ///
    /// 让用户手填也不行：他在 LM Studio 里一改加载参数，填的那个数就漂了。
    ///
    /// # `loaded_context_length` 而不是 `max_context_length`
    ///
    /// 前者是**这次加载实际给了多少**，后者是模型能力上限。实测
    /// `ornith-1.5-35b-a3b` 两个数分别是 33024 和 262144 ——
    /// 按后者算预算会得出一个塞不进去的数，那是现在这个 bug 的放大版。
    ///
    /// 返回 `None` = **查不到**（这个后端没有这个接口），不是"窗口是 0"。
    /// 调用方据此回落到默认预算，而不是算出一个 0 来。
    pub(crate) async fn context_window(&self, model: &str) -> Option<u32> {
        #[derive(Deserialize)]
        struct Entry {
            id: String,
            #[serde(default)]
            state: Option<String>,
            #[serde(default)]
            loaded_context_length: Option<u32>,
        }
        #[derive(Deserialize)]
        struct Resp {
            #[serde(default)]
            data: Vec<Entry>,
        }
        let resp = self
            .authorized(self.http.get(self.endpoint("/api/v0/models")))
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let body: Resp = resp.json().await.ok()?;
        body.data
            .into_iter()
            // 点名那个模型；没点中就退而求其次找任何一个已加载的
            .find(|e| e.id == model && e.state.as_deref() == Some("loaded"))
            .or(None)
            .and_then(|e| e.loaded_context_length)
            .filter(|n| *n > 0)
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    fn authorized(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        // 本地 LM Studio 默认不需要 key，但新版本可以开 token 校验；
        // 所以这里是"有就带上"，而不是"必须有"。
        match &self.api_key {
            Some(key) if !key.is_empty() => builder.bearer_auth(key),
            _ => builder,
        }
    }

    fn body(&self, req: &ChatRequest) -> Value {
        let mut body = json!({
            "model": self.model,
            "messages": req.messages.iter().map(encode_message).collect::<Vec<_>>(),
            "stream": req.stream,
        });
        if req.stream {
            // 要 usage 统计需要显式开，否则最后一帧不带 usage。
            // **非流式不能带这个键** —— 那条响应本来就带 usage，
            // 而有的后端见到它会报参数错误。
            body["stream_options"] = json!({ "include_usage": true });
        }

        if !req.tools.is_empty() && self.supports_tools {
            body["tools"] = Value::Array(req.tools.iter().map(encode_tool).collect());
        }
        // GPT-5 / o 系列只认默认温度，发了 0 直接 400（见 `thinking::accepts_temperature`）
        if let Some(t) = req.temperature.filter(|_| crate::llm::thinking::accepts_temperature(&self.model)) {
            body["temperature"] = json!(t);
        }
        // 显式指定充足的 max_tokens (默认 8192)，防止 LM Studio 等本地服务采用 512/1024 默认值提前截断
        body["max_tokens"] = json!(req.max_tokens.unwrap_or(8192));
        if req.json_output {
            // LM Studio 只认 json_schema（不认 json_object）。schema 只写「是个对象」：
            // 字段怎么填由提示词管，这里只保证语法合法 —— 写死完整 schema 的话
            // 每改一次镜头表字段都得同步两处。
            let schema = req.json_schema.clone().unwrap_or_else(|| json!({ "type": "object" }));
            body["response_format"] = json!({
                "type": "json_schema",
                "json_schema": { "name": "output", "strict": false, "schema": schema }
            });
        }
        apply_thinking(&mut body, req.thinking, &self.id, &self.model);
        body
    }
}

/// 关闭思考的参数名。实测 LM Studio 上 `reasoning_effort: "none"` 能让推理归零，
/// 而 `chat_template_kwargs.enable_thinking` / `reasoning.enabled` / 顶层
/// `enable_thinking` / `/no_think` 都无效或只是减少推理量。
pub(crate) const REASONING_EFFORT: &str = "reasoning_effort";

/// 记下哪些「后端/模型」不认 `reasoning_effort`，避免每轮都白撞一次 400。
///
/// 需要这张表是因为同一个后端的不同模型行为不一样：实测 Azure 的 gpt-5.x 部署
/// 接受该参数，而 gpt-4o 直接返回 400「Unrecognized request argument」。
/// 与其让用户自己去配「这个模型支不支持」，不如撞一次记住。
fn unsupported() -> &'static std::sync::RwLock<std::collections::HashSet<String>> {
    static SET: std::sync::OnceLock<std::sync::RwLock<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    SET.get_or_init(Default::default)
}

fn cache_key(provider_id: &str, model: &str) -> String {
    format!("{provider_id}/{model}")
}

/// 按需给请求体加上思考档位。
///
/// 档位怎么翻译成 `reasoning_effort` 的值、以及某个模型到底支持哪几档，
/// **全部在 [`crate::llm::thinking`] 里判**，这里只负责写进 body 和跳过
/// 运行期学到的不支持组合。
pub(crate) fn apply_thinking(
    body: &mut Value,
    effort: ThinkingMode,
    provider_id: &str,
    model: &str,
) {
    let Some(value) = crate::llm::thinking::openai_reasoning_effort(effort, model) else {
        return;
    };
    if unsupported()
        .read()
        .unwrap()
        .contains(&cache_key(provider_id, model))
    {
        return;
    }
    body[REASONING_EFFORT] = json!(value);
}

/// 上游是不是在抱怨我们多发了 `reasoning_effort`。
///
/// 只认「参数不被识别/不被支持」这一类报错 —— 其它 400（模型不存在、上下文超长）
/// 必须原样报给用户，不能被这里悄悄吞掉重试。
pub(crate) fn rejected_reasoning_effort(err: &Error) -> bool {
    let Error::Upstream { body, .. } = err else {
        return false;
    };
    body.contains(REASONING_EFFORT)
        && (body.contains("Unrecognized request argument")
            || body.contains("unsupported_parameter")
            || body.contains("Unsupported parameter")
            || body.contains("unknown_parameter"))
}

/// 记下这个组合不支持，下次不再发
pub(crate) fn remember_unsupported(provider_id: &str, model: &str) {
    tracing::info!(
        provider = %provider_id,
        model = %model,
        "该模型不支持 reasoning_effort，本轮去掉该参数重试，后续不再发送"
    );
    unsupported()
        .write()
        .unwrap()
        .insert(cache_key(provider_id, model));
}

pub(crate) fn normalize_image_for_llm(img: &str) -> String {
    if !img.starts_with("data:image/") {
        return img.to_string();
    }
    // LM Studio / Ollama / OpenAI 端点严格要求 JPEG 或 PNG 格式 Base64
    // 如果是 webp / bmp / gif / svg，解码并转换为标准 JPEG
    if img.starts_with("data:image/webp")
        || img.starts_with("data:image/bmp")
        || img.starts_with("data:image/gif")
        || img.starts_with("data:image/svg")
    {
        if let Some((_, b64_part)) = img.split_once(";base64,") {
            use base64::Engine;
            if let Ok(raw_bytes) = base64::engine::general_purpose::STANDARD.decode(b64_part) {
                if let Ok(dyn_img) = image::load_from_memory(&raw_bytes) {
                    let mut jpeg_buf = std::io::Cursor::new(Vec::new());
                    if dyn_img.write_to(&mut jpeg_buf, image::ImageFormat::Jpeg).is_ok() {
                        let new_b64 = base64::engine::general_purpose::STANDARD.encode(jpeg_buf.into_inner());
                        return format!("data:image/jpeg;base64,{new_b64}");
                    }
                }
            }
            return format!("data:image/jpeg;base64,{b64_part}");
        }
    }
    img.to_string()
}

pub(crate) fn encode_message(msg: &Message) -> Value {
    let role = match msg.role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    };

    let valid_images: Vec<String> = msg
        .images
        .iter()
        .filter(|img| {
            img.starts_with("data:image/")
                || img.starts_with("http://")
                || img.starts_with("https://")
        })
        .map(|img| normalize_image_for_llm(img))
        .collect();

    let content = if valid_images.is_empty() {
        json!(msg.content)
    } else {
        let mut parts = vec![json!({"type": "text", "text": msg.content})];
        for img in valid_images {
            parts.push(json!({"type": "image_url", "image_url": {"url": img}}));
        }
        json!(parts)
    };

    let mut out = json!({ "role": role, "content": content });

    if !msg.tool_calls.is_empty() {
        out["tool_calls"] = Value::Array(
            msg.tool_calls
                .iter()
                .map(|c| {
                    json!({
                        "id": c.id,
                        "type": "function",
                        "function": { "name": c.name, "arguments": c.arguments },
                    })
                })
                .collect(),
        );
    }
    if let Some(id) = &msg.tool_call_id {
        out["tool_call_id"] = json!(id);
    }
    out
}

pub(crate) fn encode_tool(tool: &Tool) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": tool.name,
            "description": tool.description,
            "parameters": tool.parameters,
        }
    })
}

// ---------------------------------------------------------------------------
// 流式响应反序列化
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct StreamEvent {
    #[serde(default)]
    choices: Vec<StreamChoice>,
    #[serde(default)]
    usage: Option<ApiUsage>,
    /// 出错时上游可能在 200 的流里塞一个 error 对象
    #[serde(default)]
    error: Option<Value>,
}

#[derive(Deserialize)]
struct StreamChoice {
    #[serde(default)]
    delta: Delta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Default, Deserialize)]
struct Delta {
    #[serde(default)]
    content: Option<String>,
    /// 推理模型的思考过程。字段名各家不一：DeepSeek-R1 用 reasoning_content，
    /// 部分 OpenAI 兼容实现用 reasoning，两个都收。
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    reasoning: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<DeltaToolCall>>,
}

#[derive(Deserialize)]
struct DeltaToolCall {
    /// 流式 tool_call 用 index 标识这是第几个调用，后续增量只带 index
    #[serde(default)]
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<DeltaFunction>,
}

#[derive(Deserialize)]
struct DeltaFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Deserialize)]
struct ApiUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
    /// 实测 LM Studio 的收尾帧里有这一段：
    /// `"completion_tokens_details": {"reasoning_tokens": 73}`
    /// 不是所有后端都给，所以整段都是 Option。
    #[serde(default)]
    completion_tokens_details: Option<ApiCompletionDetails>,
}

#[derive(Deserialize)]
struct ApiCompletionDetails {
    #[serde(default)]
    reasoning_tokens: Option<u32>,
}

#[derive(Deserialize)]
pub(crate) struct ModelsResponse {
    #[serde(default)]
    pub data: Vec<ModelEntry>,
}

#[derive(Deserialize)]
pub(crate) struct ModelEntry {
    pub id: String,
}

/// 累积流式工具调用。
///
/// OpenAI 的流式 tool_call 是按 index 分片下发的：第一片带 id 和 name，
/// 后续若干片只带 arguments 的字符串增量。必须攒齐才能交给上层执行，
/// 所以用 BTreeMap 按 index 归档（BTree 而非 Hash：收尾时要按调用顺序吐出）。
#[derive(Default)]
struct ToolCallAccumulator {
    slots: BTreeMap<usize, ToolCall>,
}

impl ToolCallAccumulator {
    fn ingest(&mut self, delta: &DeltaToolCall) {
        let slot = self.slots.entry(delta.index).or_insert_with(|| ToolCall {
            id: String::new(),
            name: String::new(),
            arguments: String::new(),
        });

        if let Some(id) = &delta.id {
            slot.id = id.clone();
        }
        if let Some(f) = &delta.function {
            if let Some(name) = &f.name {
                slot.name.push_str(name);
            }
            if let Some(args) = &f.arguments {
                slot.arguments.push_str(args);
            }
        }
    }

    fn drain(&mut self) -> Vec<ToolCall> {
        std::mem::take(&mut self.slots).into_values().collect()
    }
}

/// 把 `Frame` 流转成归一化的 `ChatChunk` 流。
///
/// 状态机而不是 `poll_fn` 里手写循环：一帧上游事件可能产出 0 到 N 个 chunk
/// （收尾帧就会一次吐出多个 ToolCall 再加一个 Done），需要一个显式的待发队列。
/// 把非流式响应改写成流式那一帧的形状。
///
/// 两者只差一个键名：非流式是 `choices[].message`（一整条），流式是
/// `choices[].delta`（增量）。里面的字段完全一样（`content` /
/// `reasoning_content` / `tool_calls`），所以把 `message` 挪成 `delta`
/// 之后，下游那套增量处理拿到它就等价于「一次收完的流」。
///
/// `usage` 和 `finish_reason` 原样留着 —— 它们在两种响应里同名同位。
fn as_stream_shaped(mut body: Value) -> Value {
    if let Some(choices) = body.get_mut("choices").and_then(|c| c.as_array_mut()) {
        for c in choices {
            // **整个键移掉，不是 `take()`** —— `take()` 会留下
            // `"message": null`，下游多一个永远为空的字段。
            if let Some(msg) = c.as_object_mut().and_then(|o| o.remove("message")) {
                c["delta"] = msg;
            }
        }
    }
    body
}

/// OpenAI / Azure 的 `finish_reason` → 我们的结束原因。**认不出的才当正常结束。**
///
/// `content_filter` 原来就是落进 `_ => Stop` 的那个：输出被上游的内容过滤掐断，
/// 半截文字被当成写完了（2026-10-03 EP02 线路 B 报「返回里没有 JSON」）。
pub(crate) fn finish_reason(reason: &str) -> FinishReason {
    match reason {
        "tool_calls" => FinishReason::ToolCalls,
        "length" => FinishReason::Length,
        "content_filter" => FinishReason::ContentFilter,
        _ => FinishReason::Stop,
    }
}

pub(crate) fn into_chunks(
    frames: futures_util::stream::BoxStream<'static, Result<Frame>>,
) -> impl futures_util::Stream<Item = Result<ChatChunk>> + Send {
    struct State {
        frames: futures_util::stream::BoxStream<'static, Result<Frame>>,
        tools: ToolCallAccumulator,
        usage: Option<Usage>,
        finish: FinishReason,
        pending: std::collections::VecDeque<Result<ChatChunk>>,
        /// Done 已经发出，之后即使上游还有帧也不再产出
        closed: bool,
    }

    let state = State {
        frames,
        tools: ToolCallAccumulator::default(),
        usage: None,
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

            match st.frames.next().await {
                Some(Err(err)) => {
                    st.closed = true;
                    return Some((Err(err), st));
                }
                Some(Ok(Frame::Data(payload))) => {
                    let event: StreamEvent = match serde_json::from_str(&payload) {
                        Ok(ev) => ev,
                        // 单帧解析失败不该终止整个流：本地推理服务偶尔会插入
                        // 非标准帧（心跳、日志行）。记一笔然后继续。
                        Err(err) => {
                            tracing::debug!(%err, payload = %payload, "跳过无法解析的流式帧");
                            continue;
                        }
                    };

                    if let Some(err) = event.error {
                        st.closed = true;
                        return Some((Err(Error::upstream(200, err.to_string())), st));
                    }
                    if let Some(u) = event.usage {
                        st.usage = Some(Usage {
                            prompt_tokens: u.prompt_tokens,
                            completion_tokens: u.completion_tokens,
                            reasoning_tokens: u
                                .completion_tokens_details
                                .and_then(|d| d.reasoning_tokens),
                        });
                    }

                    // 带 usage 的收尾帧没有 choices，属于正常情况
                    let Some(choice) = event.choices.first() else {
                        continue;
                    };

                    if let Some(reason) = &choice.finish_reason {
                        st.finish = finish_reason(reason);
                    }

                    if let Some(deltas) = &choice.delta.tool_calls {
                        for d in deltas {
                            st.tools.ingest(d);
                        }
                    }

                    let reasoning = choice
                        .delta
                        .reasoning_content
                        .as_ref()
                        .or(choice.delta.reasoning.as_ref());
                    if let Some(delta) = reasoning.filter(|d| !d.is_empty()) {
                        st.pending.push_back(Ok(ChatChunk::Reasoning {
                            delta: delta.clone(),
                        }));
                    }
                    if let Some(delta) = choice.delta.content.as_ref().filter(|d| !d.is_empty()) {
                        st.pending.push_back(Ok(ChatChunk::Text {
                            delta: delta.clone(),
                        }));
                    }
                }
                // 显式 [DONE] 与连接自然结束走同一套收尾逻辑
                Some(Ok(Frame::Done)) | None => {
                    st.closed = true;
                    for call in st.tools.drain() {
                        st.pending.push_back(Ok(ChatChunk::ToolCall { call }));
                    }
                    st.pending.push_back(Ok(ChatChunk::Done {
                        reason: st.finish,
                        usage: st.usage,
                    }));
                }
            }
        }
    })
}

#[async_trait]
impl LlmConnector for OpenAiConnector {
    fn id(&self) -> &str {
        &self.id
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn supports_streaming_tools(&self) -> bool {
        self.supports_tools
    }

    async fn chat_stream(&self, req: ChatRequest) -> Result<ChatStream> {
        let url = self.endpoint("/v1/chat/completions");

        let send = |body: Value| {
            let builder = self.authorized(self.http.post(&url)).json(&body);
            async move { ensure_success(builder.send().await?).await }
        };

        let resp = match send(self.body(&req)).await {
            Ok(resp) => resp,
            // 该模型不认 reasoning_effort：记下来、去掉参数重试一次。
            // 报错发生在流开始之前，所以这次重试对用户是不可见的。
            Err(err) if rejected_reasoning_effort(&err) => {
                remember_unsupported(&self.id, &self.model);
                send(self.body(&req)).await?
            }
            Err(err) => return Err(err),
        };

        if !req.stream {
            // **一次拿完，再当成「只有一帧的流」交出去。**
            // 调用方拿到的还是 `ChatStream`，一行都不用改；而增量拼接、
            // 工具调用累积、usage 收尾全部走原来那套 —— 不另写一份
            // 非流式的解析，两份迟早会漂移。
            let body: Value = resp.json().await?;
            let frame = Frame::Data(as_stream_shaped(body).to_string());
            let frames = futures_util::stream::once(async move { Ok(frame) }).boxed();
            return Ok(into_chunks(frames).boxed());
        }
        Ok(into_chunks(decode_response(resp).boxed()).boxed())
    }

    async fn health_check(&self) -> Result<Vec<String>> {
        let resp = self
            .authorized(self.http.get(self.endpoint("/v1/models")))
            .send()
            .await?;
        let resp = ensure_success(resp).await?;
        let parsed: ModelsResponse = resp.json().await?;
        Ok(parsed.data.into_iter().map(|m| m.id).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;


    /// 上游掐断的几种原因都要认出来，不能当成「正常说完」
    #[test]
    fn 结束原因认得出内容过滤() {
        assert_eq!(finish_reason("content_filter"), FinishReason::ContentFilter);
        assert_eq!(finish_reason("length"), FinishReason::Length);
        assert_eq!(finish_reason("tool_calls"), FinishReason::ToolCalls);
        assert_eq!(finish_reason("stop"), FinishReason::Stop);
        assert_eq!(finish_reason("别的什么"), FinishReason::Stop);
    }

    /// **非流式响应要能原样走流式那条解析路。**
    ///
    /// 两份解析各写一份的话迟早漂移 —— 表现是「关掉流式之后工具调用丢了」
    /// 或者「usage 统计没了」，而且不报错。
    #[test]
    fn 非流式响应改写成流式那一帧() {
        let body = json!({
            "choices": [{"index": 0, "finish_reason": "stop",
                         "message": {"role": "assistant", "content": "好的",
                                     "reasoning_content": "想了想"}}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 2}
        });
        let v = as_stream_shaped(body);
        let c = &v["choices"][0];
        assert!(c.get("message").is_none(), "message 没被挪走：{v}");
        assert_eq!(c["delta"]["content"], "好的");
        assert_eq!(c["delta"]["reasoning_content"], "想了想", "思考内容丢了");
        assert_eq!(c["finish_reason"], "stop", "结束原因丢了");
        assert_eq!(v["usage"]["completion_tokens"], 2, "usage 丢了");
        // 真的能被流式那套解析器读进去
        let ev: StreamEvent = serde_json::from_value(v).expect("改写完解析不了");
        assert_eq!(ev.choices.len(), 1);
    }

    /// 请求体里的 `stream` 要跟着 `req` 走，而 `stream_options`
    /// **只在流式时发** —— 非流式带着它有的后端会 400。
    #[test]
    fn 非流式请求不带_stream_options() {
        let c = OpenAiConnector::new("lmstudio", "http://127.0.0.1:1234", "m", None, true, reqwest::Client::new());
        let mut req = ChatRequest::new(vec![Message::system("x")]);
        req.stream = false;
        let b = c.body(&req);
        assert_eq!(b["stream"], false);
        assert!(b.get("stream_options").is_none(), "非流式带上了 stream_options");

        req.stream = true;
        let b2 = c.body(&req);
        assert_eq!(b2["stream"], true);
        assert!(b2.get("stream_options").is_some(), "流式没开 include_usage，usage 会丢");
    }

    #[test]
    fn 要求_json_时带上语法约束_默认不带() {
        let c = OpenAiConnector::new("lmstudio", "http://127.0.0.1:1234", "m", None, true, reqwest::Client::new());
        let mut req = ChatRequest::new(vec![Message::user("x")]);
        assert!(c.body(&req).get("response_format").is_none(), "对话也被约束成 JSON 了");
        req.json_output = true;
        let b = c.body(&req);
        assert_eq!(b["response_format"]["type"], "json_schema");
        assert_eq!(b["response_format"]["json_schema"]["schema"]["type"], "object");
        // 给了 schema 就按 schema 发（镜头表要写明 shots 必填）
        req.json_schema = Some(json!({"type": "object", "required": ["shots"]}));
        assert_eq!(c.body(&req)["response_format"]["json_schema"]["schema"]["required"], json!(["shots"]));
    }

    #[test]
    fn 解析lm_studio实测的收尾帧包括思考token() {
        // 这段是本机实测抓下来的真实响应（qwen/qwen3.8-27b，90 帧里只有末帧带 usage）。
        // OpenAI 流式协议里 include_usage 的语义是"末尾追加一个只含 usage、
        // 没有 choices 的帧"，不是逐帧累计 —— 所以流式期间拿不到精确 token。
        let raw = r#"{"choices":[],"usage":{"prompt_tokens":60,"completion_tokens":91,
            "total_tokens":151,"completion_tokens_details":{"reasoning_tokens":73}}}"#;
        let ev: StreamEvent = serde_json::from_str(raw).expect("应当能解析");
        let u = ev.usage.expect("末帧必须带 usage");
        assert_eq!(u.prompt_tokens, 60);
        assert_eq!(u.completion_tokens, 91);
        assert_eq!(
            u.completion_tokens_details.and_then(|d| d.reasoning_tokens),
            Some(73),
            "reasoning_tokens 要解析出来 —— 它回答「这个档位把多少预算烧在思考上」"
        );
    }

    #[test]
    fn 没有details段时思考token是none而不是零() {
        // "不知道"和"没思考"是两件事。变成 0 的话前端会显示"思考 0"，
        // 让人以为模型没思考，而实际是这个后端不报这个数。
        let raw = r#"{"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":20}}"#;
        let ev: StreamEvent = serde_json::from_str(raw).expect("应当能解析");
        let u = ev.usage.unwrap();
        assert!(u.completion_tokens_details.is_none());
    }

    #[test]
    fn details存在但字段缺失也不能崩() {
        let raw = r#"{"choices":[],"usage":{"prompt_tokens":1,"completion_tokens":2,
            "completion_tokens_details":{}}}"#;
        let ev: StreamEvent = serde_json::from_str(raw).expect("应当能解析");
        assert_eq!(
            ev.usage
                .unwrap()
                .completion_tokens_details
                .and_then(|d| d.reasoning_tokens),
            None
        );
    }

    #[test]
    fn encode_message_ignores_plain_file_paths_in_images() {
        let msg = Message {
            role: Role::Tool,
            content: "file content".to_string(),
            images: vec![
                "/Users/you/demo/package.json".to_string(),
                "data:image/png;base64,AAAA".to_string(),
            ],
            tool_calls: Vec::new(),
            tool_call_id: Some("call_1".to_string()),
        };

        let encoded = encode_message(&msg);
        let content = encoded["content"]
            .as_array()
            .expect("content should be an array");
        // 应该只有 text 与 1 个合法 base64 data image_url，普通文件路径被过滤
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image_url");
        assert_eq!(content[1]["image_url"]["url"], "data:image/png;base64,AAAA");
    }
}
