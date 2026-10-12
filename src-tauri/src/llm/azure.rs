//! Azure OpenAI 适配器。
//!
//! 响应格式与 OpenAI 完全一致（所以流式解码直接复用 `openai::into_chunks`），
//! 但请求侧有四处不同，每一处都实测确认过：
//!
//!   1. **URL**：模型名在路径里而不是 body 里 ——
//!      `{endpoint}/openai/deployments/{deployment}/chat/completions?api-version=…`
//!   2. **鉴权**：`api-key` 请求头，不是 `Authorization: Bearer`。
//!   3. **token 上限字段**：新一代部署（gpt-5.x / gpt-chat-latest）只认
//!      `max_completion_tokens`，发 `max_tokens` 会直接 400。实测 gpt-4o
//!      同时接受两种写法，所以这里统一只发 `max_completion_tokens`，
//!      不需要按部署名分支。
//!   4. **列举模型**：`/openai/deployments` 只在 `api-version=2023-03-15-preview`
//!      下可用（2024-10-21 及更新的版本返回 404），所以列举用的版本号是写死的，
//!      与对话用的 `api_version` 分开。

use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{json, Value};

use super::openai::{
    apply_thinking, encode_message, encode_tool, into_chunks, rejected_reasoning_effort,
    remember_unsupported, ModelsResponse,
};
use super::sse::{decode_response, ensure_success};
use super::{ChatRequest, ChatStream, LlmConnector};
use crate::error::{Error, Result};

/// 对话接口的默认 api-version
pub const DEFAULT_API_VERSION: &str = "2024-10-21";

/// 列举 deployments 唯一可用的 api-version（见模块文档第 4 点）
const LIST_API_VERSION: &str = "2023-03-15-preview";

pub struct AzureOpenAiConnector {
    /// 资源根地址，如 `https://xxx.openai.azure.com`
    endpoint: String,
    /// Azure 的"部署名"，占据 OpenAI 语义里 model 的位置
    deployment: String,
    api_version: String,
    api_key: String,
    supports_tools: bool,
    http: reqwest::Client,
}

impl AzureOpenAiConnector {
    pub fn new(
        endpoint: impl Into<String>,
        deployment: impl Into<String>,
        api_version: Option<String>,
        api_key: impl Into<String>,
        supports_tools: bool,
        http: reqwest::Client,
    ) -> Self {
        Self {
            endpoint: endpoint.into().trim_end_matches('/').to_string(),
            deployment: deployment.into(),
            api_version: api_version
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_API_VERSION.to_string()),
            api_key: api_key.into(),
            supports_tools,
            http,
        }
    }

    fn chat_url(&self) -> String {
        format!(
            "{}/openai/deployments/{}/chat/completions?api-version={}",
            self.endpoint, self.deployment, self.api_version
        )
    }

    fn list_url(&self) -> String {
        format!(
            "{}/openai/deployments?api-version={}",
            self.endpoint, LIST_API_VERSION
        )
    }

    fn body(&self, req: &ChatRequest) -> Value {
        // 注意：没有 "model" 字段 —— 部署名已经在 URL 里
        let mut body = json!({
            "messages": req.messages.iter().map(encode_message).collect::<Vec<_>>(),
            "stream": true,
            "stream_options": { "include_usage": true },
        });

        if !req.tools.is_empty() && self.supports_tools {
            body["tools"] = Value::Array(req.tools.iter().map(encode_tool).collect());
        }
        // GPT-5 / o 系列只认默认温度，发了 0 直接 400（见 `thinking::accepts_temperature`）
        if let Some(t) = req.temperature.filter(|_| crate::llm::thinking::accepts_temperature(&self.deployment)) {
            body["temperature"] = json!(t);
        }
        if let Some(m) = req.max_tokens {
            body["max_completion_tokens"] = json!(m);
        }
        // 实测：gpt-5.x 部署接受 reasoning_effort，gpt-4o 直接 400。
        // 所以这里照发，被拒了在 chat_stream 里去掉参数重试并记住。
        apply_thinking(&mut body, req.thinking, self.id(), &self.deployment);
        body
    }
}

#[async_trait]
impl LlmConnector for AzureOpenAiConnector {
    fn id(&self) -> &str {
        "azure"
    }

    fn model(&self) -> &str {
        &self.deployment
    }

    fn supports_streaming_tools(&self) -> bool {
        self.supports_tools
    }

    async fn chat_stream(&self, req: ChatRequest) -> Result<ChatStream> {
        if self.api_key.is_empty() {
            return Err(Error::Credential(
                "Azure OpenAI 需要 API Key，请先在设置里填写".into(),
            ));
        }

        let url = self.chat_url();
        let send = |body: serde_json::Value| {
            let builder = self
                .http
                .post(&url)
                .header("api-key", &self.api_key)
                .json(&body);
            async move { ensure_success(builder.send().await?).await }
        };

        let resp = match send(self.body(&req)).await {
            Ok(resp) => resp,
            Err(err) if rejected_reasoning_effort(&err) => {
                remember_unsupported(self.id(), &self.deployment);
                send(self.body(&req)).await?
            }
            Err(err) => return Err(err),
        };

        Ok(into_chunks(decode_response(resp).boxed()).boxed())
    }

    async fn health_check(&self) -> Result<Vec<String>> {
        if self.api_key.is_empty() {
            return Err(Error::Credential(
                "Azure OpenAI 需要 API Key，请先在设置里填写".into(),
            ));
        }

        let resp = self
            .http
            .get(self.list_url())
            .header("api-key", &self.api_key)
            .send()
            .await?;
        let resp = ensure_success(resp).await?;

        // 响应形状与 OpenAI 的 /v1/models 一致：{ "data": [{ "id": … }] }
        let parsed: ModelsResponse = resp.json().await?;
        Ok(parsed.data.into_iter().map(|m| m.id).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::Message;

    fn connector() -> AzureOpenAiConnector {
        AzureOpenAiConnector::new(
            "https://example.openai.azure.com/",
            "gpt-4o",
            None,
            "k",
            true,
            reqwest::Client::new(),
        )
    }

    /// gpt-5.6-sol 收到 temperature=0 直接 400（2026-09-29 实测）；gpt-4o 照常发
    #[test]
    fn 推理模型的部署不发温度() {
        let mut req = ChatRequest::new(vec![Message::user("x")]);
        req.temperature = Some(0.0);
        let gpt5 = AzureOpenAiConnector::new("https://example.openai.azure.com/", "gpt-5.6-sol", None, "k", true, reqwest::Client::new());
        assert!(gpt5.body(&req).get("temperature").is_none());
        assert_eq!(connector().body(&req)["temperature"], 0.0);
    }

    #[test]
    fn puts_deployment_in_path_not_body() {
        let c = connector();
        assert_eq!(
            c.chat_url(),
            format!(
                "https://example.openai.azure.com/openai/deployments/gpt-4o/chat/completions?api-version={DEFAULT_API_VERSION}"
            )
        );

        let body = c.body(&ChatRequest::new(vec![Message::user("hi")]));
        assert!(
            body.get("model").is_none(),
            "Azure 不应带 model 字段：{body}"
        );
    }

    #[test]
    fn uses_max_completion_tokens_not_max_tokens() {
        let mut req = ChatRequest::new(vec![Message::user("hi")]);
        req.max_tokens = Some(128);

        let body = connector().body(&req);
        // gpt-5.x / gpt-chat-latest 部署发 max_tokens 会 400
        assert_eq!(body["max_completion_tokens"], 128);
        assert!(body.get("max_tokens").is_none());
    }

    #[test]
    fn listing_uses_its_own_pinned_api_version() {
        // 列举 deployments 只有旧版本可用，不能跟着对话版本走
        assert!(connector().list_url().contains(LIST_API_VERSION));
    }
}
