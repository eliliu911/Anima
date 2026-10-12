//! MCP server 的配置与持久化。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::guard::{ApprovalMode, CommandPolicy};
use super::permission::ServerPermissions;

impl McpTransport {
    /// 这个 server 需要哪些凭据。启动前用来从 Keychain/环境变量取值，
    /// 凭据本身不进 config.json。
    pub fn secret_ids(&self) -> Vec<String> {
        match self {
            Self::Stdio {
                env_from_secrets, ..
            } => env_from_secrets.values().cloned().collect(),
            Self::Http {
                headers_from_secrets,
                ..
            } => headers_from_secrets.values().cloned().collect(),
        }
    }
}

/// 连接方式
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum McpTransport {
    /// 本地子进程，通过 stdio 通信。绝大多数 MCP server 是这种
    /// （Node/Python 写的 CLI）。
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        /// 额外环境变量。**API Key 之类不要写在这里** —— 这份配置会落到
        /// config.json 明文。用 `env_from_keychain` 指定要注入的凭据名。
        #[serde(default)]
        env: BTreeMap<String, String>,
        /// 需要从 Keychain / 环境变量注入的凭据：
        /// `{ "环境变量名": "凭据 id" }`
        #[serde(default)]
        env_from_secrets: BTreeMap<String, String>,
    },
    /// 远程 server（streamable HTTP）
    Http {
        url: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
        /// 同上，避免把 token 明文写进配置
        #[serde(default)]
        headers_from_secrets: BTreeMap<String, String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// 唯一标识。同时用作**工具名的命名空间前缀**，因为不同 server
    /// 很可能导出同名工具（好几个 server 都有 `search`）。
    pub id: String,
    pub label: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub transport: McpTransport,
    #[serde(default)]
    pub permissions: ServerPermissions,
    /// 审批模式：保守（白名单外都问）或自动（读写免问）。
    /// 无论哪档，高风险操作都仍然要确认 —— 见 guard.rs
    #[serde(default)]
    pub approval_mode: ApprovalMode,
    /// 命令级白名单/黑名单，只对执行类工具生效
    #[serde(default)]
    pub commands: CommandPolicy,
    /// 启动超时（秒）。本地 server 首次启动可能要装依赖，给宽一点。
    #[serde(default = "default_timeout")]
    pub startup_timeout_secs: u64,
}

fn default_true() -> bool {
    true
}
fn default_timeout() -> u64 {
    30
}

/// 工具在暴露给模型时的全名：`server__tool`。
///
/// 用双下划线而不是点或斜杠：OpenAI 的函数名只允许
/// `[a-zA-Z0-9_-]`，点号会被拒。
pub fn qualified_tool_name(server_id: &str, tool: &str) -> String {
    format!("{server_id}__{tool}")
}

/// 从全名还原出 `(server_id, tool)`。
///
/// 用 `split_once` 而不是 `rsplit_once`：**工具名本身可能含双下划线**，
/// 而 server id 由我们约束（不允许含双下划线），所以从左边切才是对的。
pub fn split_tool_name(qualified: &str) -> Option<(&str, &str)> {
    qualified.split_once("__")
}

/// server id 的合法性。不合法会破坏工具名的命名空间。
pub fn validate_server_id(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("server id 不能为空".into());
    }
    if id.contains("__") {
        return Err("server id 不能包含双下划线（会与工具名的命名空间分隔符冲突）".into());
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("server id 只能包含字母、数字、连字符和下划线".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qualifies_and_splits_round_trip() {
        let q = qualified_tool_name("bc", "list_orders");
        assert_eq!(q, "bc__list_orders");
        assert_eq!(split_tool_name(&q), Some(("bc", "list_orders")));
    }

    #[test]
    fn splits_from_the_left_so_tool_names_may_contain_double_underscore() {
        // 工具名自己带 __ 的情况：必须从左边切，否则 server id 会被切错
        let q = qualified_tool_name("bc", "list__orders__v2");
        assert_eq!(split_tool_name(&q), Some(("bc", "list__orders__v2")));
    }

    #[test]
    fn split_returns_none_without_separator() {
        assert_eq!(split_tool_name("no_separator"), None);
    }

    #[test]
    fn rejects_ids_that_would_break_the_namespace() {
        assert!(validate_server_id("bc").is_ok());
        assert!(validate_server_id("business-central_1").is_ok());
        assert!(validate_server_id("").is_err());
        // 双下划线会让 split 切错位置
        assert!(validate_server_id("bad__id").is_err());
        // 点号会被 OpenAI 的函数名校验拒掉
        assert!(validate_server_id("bad.id").is_err());
        assert!(validate_server_id("bad id").is_err());
    }

    #[test]
    fn config_fills_safe_defaults_from_partial_json() {
        let json = r#"{
            "id": "term",
            "label": "终端",
            "transport": { "kind": "stdio", "command": "npx" }
        }"#;
        let c: McpServerConfig = serde_json::from_str(json).expect("应当能补齐默认值");
        assert!(c.enabled);
        assert_eq!(c.startup_timeout_secs, 30);
        // 权限必须补成安全默认，而不是全放开
        assert_eq!(c.permissions.delete, super::super::permission::Access::Deny);
        assert_eq!(c.permissions.write, super::super::permission::Access::Ask);
    }

    #[test]
    fn http_transport_deserializes() {
        let json = r#"{
            "id": "remote",
            "label": "远程",
            "transport": { "kind": "http", "url": "https://example.com/mcp" }
        }"#;
        let c: McpServerConfig = serde_json::from_str(json).unwrap();
        match c.transport {
            McpTransport::Http { url, .. } => assert_eq!(url, "https://example.com/mcp"),
            _ => panic!("应当解析成 Http"),
        }
    }
}

#[cfg(test)]
mod transport_wire_format {
    use super::*;

    /// 传输方式的判别字段名是 **`kind`**，不是 `type`。
    ///
    /// 写错的后果不是"这一项读不出来"，而是**整份 config.json 解析失败、
    /// 所有设置退回默认值**（实际踩过一次：手写配置时用了 `type`）。
    /// 前端 src/api/mcp.ts 的 McpTransport 必须跟这里一致。
    #[test]
    fn transport_is_tagged_with_kind() {
        let stdio = McpTransport::Stdio {
            command: "npx".into(),
            args: vec![],
            env: Default::default(),
            env_from_secrets: Default::default(),
        };
        let json = serde_json::to_value(&stdio).unwrap();
        assert_eq!(
            json.get("kind").and_then(|v| v.as_str()),
            Some("stdio"),
            "判别字段必须叫 kind：{json}"
        );
        assert!(json.get("type").is_none(), "不该有 type 字段：{json}");

        // 反向：用 kind 的 JSON 必须读得回来
        let parsed: McpTransport =
            serde_json::from_str(r#"{"kind":"stdio","command":"npx","args":[]}"#)
                .expect("kind 形式必须能解析");
        assert!(matches!(parsed, McpTransport::Stdio { .. }));

        // 用 type 的必须失败 —— 免得以后有人"顺手改成 type"却没人发现
        assert!(
            serde_json::from_str::<McpTransport>(r#"{"type":"stdio","command":"npx","args":[]}"#)
                .is_err(),
            "type 形式不该被接受"
        );
    }
}
