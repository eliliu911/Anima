//! 单个 MCP server 的连接。
//!
//! 基于官方 `rmcp` SDK，不手搓 JSON-RPC —— MCP 的协议版本还在演进，
//! 自己实现等于持续追一个移动靶。
//!
//! 本地 server 走子进程 stdio（绝大多数 MCP server 是 Node/Python 写的 CLI），
//! 远程 server 走 streamable HTTP。

use std::collections::BTreeMap;

use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::ServiceExt;
use serde::Serialize;

use super::config::{McpServerConfig, McpTransport};
use crate::error::{Error, Result};

/// 单次工具结果回填给模型的字符上限。
///
/// **必须有这个上限。** 工具结果会作为 `role=tool` 消息进入下一轮请求，
/// 而很多工具的输出没有天然边界：`directory_tree` 撞上 node_modules 或
/// Rust 的 target/ 就是几十万条路径。实际踩过一次 —— 用户问"项目里有哪些目录"，
/// 请求膨胀到 157 万 token，后端直接返回 exceed_context_size_error，
/// 整轮对话失败且没有任何有用输出。
///
/// 12000 字符约合 3000~6000 token：足够装下一份正常的目录列表或文件内容，
/// 又给对话历史和后续轮次留出余量。截断优于失败：模型拿到前半段仍然能回答，
/// 而请求被拒的话它什么都拿不到。
const MAX_RESULT_CHARS: usize = 12_000;

/// 一个 server 导出的工具（已归一化成网关能用的形状）
#[derive(Debug, Clone, Serialize)]
pub struct McpTool {
    /// server 内的原始名字
    pub name: String,
    pub description: String,
    /// 参数的 JSON Schema
    pub input_schema: serde_json::Value,
}

pub struct McpClient {
    server_id: String,
    service: RunningService<RoleClient, ()>,
}

impl McpClient {
    /// 连接一个 server。
    ///
    /// 会在这里完成 MCP 的 initialize 握手；失败即返回错误，
    /// 不做"先连上再慢慢重试"——上层需要立刻知道这个 server 能不能用。
    pub async fn connect(
        config: &McpServerConfig,
        secrets: &BTreeMap<String, String>,
    ) -> Result<Self> {
        let service = match &config.transport {
            McpTransport::Stdio {
                command,
                args,
                env,
                env_from_secrets,
            } => {
                let cmd = tokio::process::Command::new(command);
                let args = args.clone();
                let env = env.clone();
                // 凭据从 Keychain/环境变量注入，不落进 config.json 明文
                let injected: Vec<(String, String)> = env_from_secrets
                    .iter()
                    .filter_map(|(var, secret_id)| {
                        secrets.get(secret_id).map(|v| (var.clone(), v.clone()))
                    })
                    .collect();

                let transport = TokioChildProcess::new(cmd.configure(|c| {
                    for a in &args {
                        c.arg(a);
                    }
                    for (k, v) in &env {
                        c.env(k, v);
                    }
                    for (k, v) in &injected {
                        c.env(k, v);
                    }
                }))
                .map_err(|e| {
                    Error::Io(format!(
                        "启动 MCP server「{}」失败（命令：{command}）：{e}",
                        config.label
                    ))
                })?;

                ().serve(transport).await.map_err(|e| {
                    Error::Io(format!("与 MCP server「{}」握手失败：{e}", config.label))
                })?
            }

            McpTransport::Http {
                url,
                headers,
                headers_from_secrets,
            } => {
                use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
                use rmcp::transport::streamable_http_client::{
                    StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
                };

                let mut reqwest_headers = HeaderMap::new();
                for (k, v) in headers {
                    if let (Ok(name), Ok(value)) = (
                        HeaderName::from_bytes(k.as_bytes()),
                        HeaderValue::from_str(v),
                    ) {
                        reqwest_headers.insert(name, value);
                    }
                }

                for (env_var, secret_id) in headers_from_secrets {
                    if let Some(secret_value) = secrets.get(secret_id) {
                        if let (Ok(name), Ok(value)) = (
                            HeaderName::from_bytes(env_var.as_bytes()),
                            HeaderValue::from_str(secret_value),
                        ) {
                            reqwest_headers.insert(name, value);
                        }
                    }
                }

                let http_client = reqwest::Client::builder()
                    .default_headers(reqwest_headers)
                    .build()
                    .map_err(|e| Error::Io(format!("创建 HTTP client 失败: {e}")))?;

                let transport_config = StreamableHttpClientTransportConfig::with_uri(url.as_str());
                let transport =
                    StreamableHttpClientTransport::with_client(http_client, transport_config);

                ().serve(transport).await.map_err(|e| {
                    Error::Io(format!("与 MCP server「{}」握手失败：{e}", config.label))
                })?
            }
        };

        tracing::info!(server = %config.id, "MCP server 已连接");
        Ok(Self {
            server_id: config.id.clone(),
            service,
        })
    }

    pub fn server_id(&self) -> &str {
        &self.server_id
    }

    /// 列举该 server 的全部工具
    pub async fn list_tools(&self) -> Result<Vec<McpTool>> {
        let tools = self
            .service
            .list_all_tools()
            .await
            .map_err(|e| Error::Io(format!("读取 MCP 工具列表失败：{e}")))?;

        Ok(tools
            .into_iter()
            .map(|t| McpTool {
                name: t.name.to_string(),
                description: t.description.map(|d| d.to_string()).unwrap_or_default(),
                // rmcp 把 schema 存成 Arc<JsonObject>，转成 Value 交给网关
                input_schema: serde_json::Value::Object((*t.input_schema).clone()),
            })
            .collect())
    }

    /// 调用一个工具，返回**扁平化成文本**的结果。
    ///
    /// 之所以拍平成字符串：工具结果最终要作为一条 `role=tool` 的消息回填给模型，
    /// 而各家 API 的工具结果字段都是纯文本。MCP 的富返回（图片、资源引用）
    /// 这里先只取文本部分，后续要用多模态再扩展。
    pub async fn call(
        &self,
        tool: &str,
        arguments: serde_json::Value,
    ) -> Result<(String, Vec<String>)> {
        let arguments = match arguments {
            serde_json::Value::Object(map) => Some(map),
            // 模型偶尔会把参数发成 null 或空字符串
            serde_json::Value::Null => None,
            other => {
                return Err(Error::config(format!(
                    "工具参数必须是 JSON 对象，实际是：{other}"
                )))
            }
        };

        let result = self
            .service
            // CallToolRequestParams 是 #[non_exhaustive]，只能走 new + builder，
            // 不能用结构体字面量（SDK 以后加字段不会破坏调用方）
            .call_tool({
                let params = rmcp::model::CallToolRequestParams::new(tool.to_string());
                match arguments {
                    Some(map) => params.with_arguments(map),
                    None => params,
                }
            })
            .await
            .map_err(|e| Error::Io(format!("调用工具 {tool} 失败：{e}")))?;

        let mut text = String::new();
        let mut images = Vec::new();
        for content in &result.content {
            if let Some(t) = content.as_text() {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&t.text);
            }
            if let rmcp::model::ContentBlock::Image(img) = content {
                images.push(format!("data:{};base64,{}", img.mime_type, img.data));
            }
        }

        let truncated = truncate_result(&text);

        // 工具报错时也要把内容回填给模型 —— 模型看到错误信息才能自我修正，
        // 直接抛错会让它完全不知道发生了什么
        if result.is_error.unwrap_or(false) {
            return Ok((format!("工具执行出错：{truncated}"), images));
        }
        if truncated.is_empty() && images.is_empty() {
            return Ok(("（工具执行成功，无输出）".to_string(), vec![]));
        }
        Ok((truncated, images))
    }

    pub async fn list_resources(&self) -> Result<serde_json::Value> {
        let result = self
            .service
            .list_resources(None)
            .await
            .map_err(|e| Error::Io(format!("读取资源列表失败：{e}")))?;
        Ok(serde_json::to_value(result.resources).unwrap_or(serde_json::Value::Null))
    }

    pub async fn read_resource(&self, uri: &str) -> Result<serde_json::Value> {
        let params = rmcp::model::ReadResourceRequestParams::new(uri.to_string());
        let result = self
            .service
            .read_resource(params)
            .await
            .map_err(|e| Error::Io(format!("读取资源失败：{e}")))?;
        Ok(serde_json::to_value(result.contents).unwrap_or(serde_json::Value::Null))
    }

    pub async fn list_prompts(&self) -> Result<serde_json::Value> {
        let result = self
            .service
            .list_prompts(None)
            .await
            .map_err(|e| Error::Io(format!("读取提示词列表失败：{e}")))?;
        Ok(serde_json::to_value(result.prompts).unwrap_or(serde_json::Value::Null))
    }

    pub async fn get_prompt(
        &self,
        name: &str,
        arguments: Option<std::collections::BTreeMap<String, String>>,
    ) -> Result<serde_json::Value> {
        let mut params = rmcp::model::GetPromptRequestParams::new(name.to_string());
        if let Some(args) = arguments {
            let mut map = serde_json::Map::new();
            for (k, v) in args {
                map.insert(k, serde_json::Value::String(v));
            }
            params = params.with_arguments(map);
        }
        let result = self
            .service
            .get_prompt(params)
            .await
            .map_err(|e| Error::Io(format!("获取提示词失败：{e}")))?;
        Ok(serde_json::to_value(result).unwrap_or(serde_json::Value::Null))
    }

    /// 主动断开。drop 也会断，但显式关闭能让子进程更快退出。
    pub async fn shutdown(self) {
        if let Err(err) = self.service.cancel().await {
            tracing::warn!(%err, server = %self.server_id, "关闭 MCP server 时出错");
        }
    }
}

/// 按**字符**而不是字节截断，并在末尾说明发生了什么。
///
/// 按字节切会把多字节字符切成半个，产生乱码；而截断说明必须给模型看到 ——
/// 否则它会把不完整的列表当成完整结果，据此给出错误结论
/// （"项目里只有这几个目录"）。
fn truncate_result(text: &str) -> String {
    let total = text.chars().count();
    if total <= MAX_RESULT_CHARS {
        return text.to_string();
    }
    let head: String = text.chars().take(MAX_RESULT_CHARS).collect();
    format!(
        "{head}\n\n[结果过长已截断：共 {total} 字符，仅返回前 {MAX_RESULT_CHARS} 字符。\
         这不是完整结果 —— 如需其余内容请缩小查询范围（指定子目录、加过滤条件、\
         或分页），不要把上面的内容当成全部。]"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_results_pass_through_unchanged() {
        assert_eq!(truncate_result("hello"), "hello");
        assert_eq!(truncate_result(""), "");
    }

    #[test]
    fn long_results_are_truncated_with_an_explicit_notice() {
        let long = "x".repeat(MAX_RESULT_CHARS + 500);
        let out = truncate_result(&long);
        assert!(out.chars().count() < MAX_RESULT_CHARS + 400, "应当明显变短");
        // 说明必须在里面：模型不知道被截断的话会把半截列表当成全部
        assert!(out.contains("已截断"), "要告知模型结果不完整");
        assert!(out.contains("不要把上面的内容当成全部"));
    }

    #[test]
    fn truncation_counts_characters_not_bytes() {
        // 按字节切会把中文切成半个字符，产生乱码
        let cjk = "目录".repeat(MAX_RESULT_CHARS);
        let out = truncate_result(&cjk);
        assert!(out.starts_with("目录"), "开头不该出现乱码");
        assert!(
            !out.contains('\u{FFFD}'),
            "不该有替换字符：说明切在了字符中间"
        );
    }
}
