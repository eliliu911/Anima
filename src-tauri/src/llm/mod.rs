//! 多模型适配层。
//!
//! 设计目标（见 docs/ARCHITECTURE.md 3.2）：上层（语音管线、MCP 调度、性格引擎）
//! 只依赖 [`LlmConnector`] 这一个抽象，不关心背后是 OpenAI、Claude，还是本机的
//! LM Studio / Ollama / MLX server。

pub mod thinking;
pub use thinking::ThinkingEffort;
/// 旧名，保留是为了不动一堆调用点。新代码请直接用 [`ThinkingEffort`]。
pub type ThinkingMode = ThinkingEffort;
pub mod anthropic;
pub mod azure;
pub mod gateway;
pub mod ollama;
pub mod openai;
pub mod sse;

use async_trait::async_trait;
use futures_util::stream::BoxStream;
use serde::{Deserialize, Serialize};

use crate::error::Result;

// ---------------------------------------------------------------------------
// 对话数据结构
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    /// 工具执行结果回填给模型
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    /// 附加的图片数据（Base64 Data URLs），用于多模态支持
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    /// 助手轮次里模型请求调用的工具（role = Assistant 时可能非空）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// role = Tool 时，标明这条结果对应哪次调用
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self::plain(Role::System, content)
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::plain(Role::User, content)
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::plain(Role::Assistant, content)
    }

    /// 工具结果消息。调用方还要自己填 `tool_call_id` —— 少了它各家后端
    /// 都配不上对应的调用。
    pub fn plain_tool(content: impl Into<String>) -> Self {
        Self::plain(Role::Tool, content)
    }

    fn plain(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            images: Vec::new(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }
}

/// 工具声明。字段刻意贴近 JSON Schema，方便直接转发 MCP server 导出的 schema。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    /// 参数的 JSON Schema（object 类型）
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// 原始 JSON 字符串。流式场景下由多个增量片段拼接而成，
    /// 拼完才可能是合法 JSON，所以这里不提前解析成 Value。
    pub arguments: String,
}

/// 是否让模型「先思考再回答」。
///
/// 为什么需要这个开关：思考阶段的输出没法送去 TTS，语音管线真正等的是第一个
/// **正文** token。本机实测 qwen3.6-35b-a3b-uncensored 的一句闲聊：
/// 思考开着要 6.99s 才吐第一个正文字，关掉是 0.08s —— 87 倍。
///
/// 档位、模型能力与各后端线格式**全部集中在 [`thinking`] 模块**，
/// 不要在适配器里各写一套：三家的表达方式（`reasoning_effort` /
/// `think` / `thinking.budget_tokens`）不同，散着写必然分叉。

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    pub messages: Vec<Message>,
    #[serde(default)]
    pub tools: Vec<Tool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub thinking: ThinkingEffort,
    /// 要不要流式。**默认要** —— 对话必须流式，不然用户盯着空屏等。
    ///
    /// 一次性拿整段结构化产出（抽分镜、逐镜精修、提取实体）时关掉：
    /// 那些调用把流拼回一个字符串**就扔掉增量**，流式对它们没有任何好处。
    /// LumenX 这条路也是非流式的。
    ///
    /// **它不会让生成变快** —— 速度由模型吐 token 的速率决定，
    /// 流式只决定是一个个推过来还是攒完一次给。关掉换来的是两件小事：
    /// 少一层 SSE 解析，以及**报错来得干净** —— 流式下上游会在一个
    /// 200 响应的流中间塞一个 error 对象，非流式直接是 4xx。
    #[serde(default = "default_stream")]
    pub stream: bool,
    /// 要求输出必须是合法 JSON（后端在采样时按语法约束，模型写不出坏 JSON）。
    ///
    /// 2026-09-13 实测：镜头表一场 8~9 千字，qwen3.6 在最长那场连续两次写出
    /// 坏 JSON（`key must be a string`），整集生成失败。靠提示词叮嘱管不住，
    /// 只能在采样层约束。OpenAI 兼容后端发 `response_format: json_schema`，
    /// Ollama 发 `format: "json"`；其它后端忽略。
    #[serde(default)]
    pub json_output: bool,
    /// `json_output` 时用的 schema。空着就只约束「是个 JSON 对象」。
    ///
    /// 2026-09-29：镜头表那一问有 7 次只写了这一场的 `stage` 就把 JSON 合上，
    /// 一镜都没写（`missing field shots`）。「是个对象」拦不住 —— 要在采样层写明哪几个键必填。
    /// **只写必填键和数值范围**，字段怎么填仍由提示词管，免得每改一次字段都要同步两处。
    #[serde(default)]
    pub json_schema: Option<serde_json::Value>,
}

fn default_stream() -> bool {
    true
}

impl ChatRequest {
    pub fn new(messages: Vec<Message>) -> Self {
        Self {
            messages,
            tools: Vec::new(),
            temperature: None,
            max_tokens: None,
            thinking: ThinkingMode::Auto,
            stream: true,
            json_output: false,
            json_schema: None,
        }
    }
}

// ---------------------------------------------------------------------------
// 流式响应
// ---------------------------------------------------------------------------

/// 各家 API 的流式事件归一化后的结果。
///
/// 有意做得比较"薄"：只区分上层真正需要分别处理的几种情况。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChatChunk {
    /// 正文增量。语音管线据此做分句 → TTS。
    Text { delta: String },
    /// 推理模型（如 o 系列、Qwen3 thinking）的思考过程增量，UI 可折叠显示。
    Reasoning { delta: String },
    /// 一次完整的工具调用（适配器内部已把增量参数拼接完毕）
    ToolCall { call: ToolCall },
    /// 工具执行的完整结果。前端用于跨轮保留工具返回值。
    /// 与 ToolActivity::Finished 的 preview 不同：preview 截断到 200 字符供日志，
    /// 这里保留完整内容（上限 20000 字符）供压缩和上下文重建使用。
    ToolResult {
        call_id: String,
        name: String,
        content: String,
        /// 这次工具执行成没成。
        ///
        /// **`Option` 不是随手写的。** 前端本来就把它当三态处理
        /// （`ok === false` / `ok === true` / 否则退回文本启发式），
        /// 而这边原先声明成「必有，缺省 true」—— 两边对不上，
        /// 而且对一个「成功还是失败」的标志，**缺省成「成功」是不安全的那一侧**。
        ///
        /// 现在发出去时永远是 `Some`；`None` 只可能来自反序列化的历史数据，
        /// 那时前端退回文本兜底，而不是被告知「成功了」。
        #[serde(default)]
        ok: Option<bool>,
    },
    /// 本轮结束
    Done {
        reason: FinishReason,
        usage: Option<Usage>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    /// 正常说完
    Stop,
    /// 撞到 max_tokens
    Length,
    /// 模型要求执行工具
    ToolCalls,
    /// 被用户打断（barge-in）或上层主动取消
    Cancelled,
    /// 工具往返次数用尽。
    ///
    /// **不是模型说完了，是我们不让它继续了。** 撞上限时 `agent.rs` 会摘掉
    /// 工具让它用已有信息作答，收尾看起来和正常结束一样 —— 但内容往往是
    /// 半截的（实测见过模型把 `exec_command` 的参数当正文写出来：
    /// 一行命令、下一行 `30000`，那是 timeout_ms 的默认值）。
    ///
    /// 之前这件事只写日志，界面一个字都不说，用户只能看到"话没说完就停了"。
    /// 前端要据此显式提示，否则没人分得清是模型的问题还是我们截断了它。
    ToolRoundsExhausted,
    /// **被服务商的内容过滤掐断**（OpenAI / Azure 的 `finish_reason: "content_filter"`）。
    ///
    /// 输出写到一半就停，文字是半截的。原来它落进 `_ => Stop`，被当成「正常说完」：
    /// 2026-10-03 EP02 线路 B 一批 28 镜里 7 镜报「返回里没有 JSON」，查了半天，
    /// 实打一次才看到 Azure 回的是 `content_filter` —— 原因在上游，报错却指向我们的解析。
    ContentFilter,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    /// 输出 token 里有多少花在思考上。
    ///
    /// **只有部分后端给**（实测 LM Studio 在
    /// `completion_tokens_details.reasoning_tokens` 里给，Ollama 原生不给），
    /// 所以是 Option —— 拿不到时 UI 不显示，而不是显示 0 让人误以为没思考。
    ///
    /// 有了它才能回答「这个模型/档位把多少预算烧在思考上」——
    /// 在此之前只能靠脚本一个个测。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u32>,
}

pub type ChatStream = BoxStream<'static, Result<ChatChunk>>;

// ---------------------------------------------------------------------------
// 连接器抽象
// ---------------------------------------------------------------------------

#[async_trait]
pub trait LlmConnector: Send + Sync {
    /// 供 UI 展示的后端标识，如 "lmstudio"、"claude"
    fn id(&self) -> &str;

    /// 该后端当前使用的模型名
    fn model(&self) -> &str;

    /// 后端是否可靠支持流式工具调用。
    ///
    /// 网关据此决定要不要把带 tools 的请求降级/改道 —— 比如 MLX server 目前
    /// 工具调用不成熟，Ollama 的 OpenAI 兼容层在 stream=true 时会丢 tool_call
    /// （见 docs/DECISIONS.md）。
    fn supports_streaming_tools(&self) -> bool {
        true
    }

    /// 流式文本交互。返回的 stream 被 drop 即代表取消，适配器内部不需要额外处理。
    async fn chat_stream(&self, req: ChatRequest) -> Result<ChatStream>;

    /// 探活 + 拉取可用模型列表，用于设置面板的"测试连接"。
    async fn health_check(&self) -> Result<Vec<String>>;
}

