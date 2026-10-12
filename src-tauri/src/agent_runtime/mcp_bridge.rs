//! MCP-to-runtime bridge helpers.
//!
//! This is the migration layer between the legacy MCP registry and the runtime
//! tool system. It keeps MCP execution in the old registry for now, while
//! normalizing metadata, resources, and prompts into runtime-friendly shapes.

use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

use super::{
    RuntimeContext, ToolCallRequest, ToolFailure, ToolFailureKind, ToolHandler, ToolKind,
    ToolOutcome, ToolSpec,
};
use crate::error::Result;
use crate::mcp::{Authorization, McpRegistry, Operation};

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeMcpToolInfo {
    pub qualified_name: String,
    pub server_id: String,
    pub name: String,
    pub description: String,
    pub operation: Operation,
    pub access: crate::mcp::Access,
    pub reason: crate::mcp::DecisionReason,
    pub exposed_to_model: bool,
}

pub fn runtime_mcp_tool_infos(registry: &McpRegistry) -> Vec<RuntimeMcpToolInfo> {
    let exposed = registry
        .tool_declarations()
        .into_iter()
        .map(|tool| tool.name)
        .collect::<std::collections::HashSet<_>>();
    registry
        .describe_tools()
        .into_iter()
        .map(|tool| RuntimeMcpToolInfo {
            exposed_to_model: exposed.contains(&tool.qualified_name),
            qualified_name: tool.qualified_name,
            server_id: tool.server_id,
            name: tool.name,
            description: tool.description,
            operation: tool.operation,
            access: tool.access,
            reason: tool.reason,
        })
        .collect()
}

pub fn runtime_mcp_specs(registry: &McpRegistry) -> Vec<ToolSpec> {
    registry
        .tool_declarations()
        .into_iter()
        .filter_map(|decl| {
            let info = registry
                .describe_tools()
                .into_iter()
                .find(|info| info.qualified_name == decl.name)?;
            Some(
                ToolSpec::new(decl.name, decl.description, ToolKind::Mcp, info.operation)
                    .with_parameters(decl.parameters),
            )
        })
        .collect()
}

pub async fn runtime_mcp_resources(registry: &McpRegistry) -> Result<Vec<Value>> {
    crate::mcp::resources::list_resources(registry).await
}

pub async fn runtime_mcp_prompts(registry: &McpRegistry) -> Result<Vec<Value>> {
    crate::mcp::prompts::list_prompts(registry).await
}

#[derive(Clone)]
pub struct McpRuntimeTool {
    registry: Arc<RwLock<McpRegistry>>,
    spec: ToolSpec,
}

impl McpRuntimeTool {
    pub fn new(registry: Arc<RwLock<McpRegistry>>, spec: ToolSpec) -> Self {
        Self { registry, spec }
    }
}

#[async_trait::async_trait]
impl ToolHandler for McpRuntimeTool {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    async fn execute(&self, ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let started = Instant::now();
        let raw_arguments =
            serde_json::to_string(&request.arguments).unwrap_or_else(|_| "{}".to_string());
        if let Some(command) = mcp_command_argument(&request.arguments) {
            if let Some(player) = super::local_tools::command_uses_system_audio_player(command) {
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(
                        ToolFailureKind::SandboxDenied,
                        format!(
                            "禁止通过 MCP 命令工具调用 `{player}` 播放音频。请改用专用 `play_audio` 工具，并把音频文件绝对路径或直链放入 `url_or_path`。"
                        ),
                    )
                    .with_hint("system__run_command 不会触发前端播放器抽屉和播放状态同步。"),
                    started.elapsed(),
                );
            }
        }
        let registry = self.registry.read().await;
        let authorization = match registry.authorize(&request.tool_name, &raw_arguments) {
            Ok(authorization) => authorization,
            Err(err) => {
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(ToolFailureKind::InvalidParams, err.to_string()),
                    started.elapsed(),
                );
            }
        };

        let approved = match authorization {
            Authorization::Ready(call) => call,
            Authorization::Rejected(verdict) => {
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(
                        ToolFailureKind::SandboxDenied,
                        format!("MCP 工具被安全策略拒绝：{}", verdict.reason()),
                    ),
                    started.elapsed(),
                );
            }
            Authorization::NeedsConfirmation {
                call: pending,
                verdict,
            } => {
                if !ctx.is_tool_call_approved(&request.id).await {
                    return ToolOutcome::failed(
                        &request,
                        ToolFailure::new(
                            ToolFailureKind::ApprovalDenied,
                            format!("MCP 工具需要用户确认：{}", verdict.reason()),
                        ),
                        started.elapsed(),
                    );
                }
                pending.approve()
            }
        };

        // 对齐 Codex / Claude Code 沙箱标准：
        // 任何外部 MCP 工具若携带 path / file / dir 等参数，均必须经过 RuntimePolicy 沙箱校验，
        // 杜绝外部 MCP 工具越界读写非当前工作区的系统目录。
        if let Some(target_path_str) = request
            .arguments
            .get("path")
            .or_else(|| request.arguments.get("file"))
            .or_else(|| request.arguments.get("dir"))
            .or_else(|| request.arguments.get("filepath"))
            .and_then(|v| v.as_str())
        {
            let resolved_path = super::local_tools::resolve_workspace_path(ctx, target_path_str);
            let capability = match self.spec.operation {
                crate::mcp::Operation::Write | crate::mcp::Operation::Delete => {
                    super::FileCapability::Write
                }
                _ => super::FileCapability::Read,
            };
            let verdict = ctx.policy.evaluate_path(&resolved_path, capability);
            if !verdict.allowed() {
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(
                        ToolFailureKind::SandboxDenied,
                        format!(
                            "MCP 工具 '{}' 访问路径 '{}' 被工作区沙箱拦截：{}",
                            request.tool_name,
                            resolved_path.display(),
                            verdict.reason()
                        ),
                    ),
                    started.elapsed(),
                );
            }
        }

        match registry.execute(approved).await {
            Ok((text, images)) => ToolOutcome::ok(&request, text, images, started.elapsed()),
            Err(err) => ToolOutcome::failed(
                &request,
                classify_mcp_error(&err.to_string()),
                started.elapsed(),
            ),
        }
    }
}

fn mcp_command_argument(arguments: &Value) -> Option<&str> {
    const KEYS: &[&str] = &["command", "cmd", "script", "shell", "input", "code"];
    let Value::Object(map) = arguments else {
        return None;
    };
    for key in KEYS {
        if let Some(Value::String(command)) = map.get(*key) {
            return Some(command);
        }
    }
    None
}

fn classify_mcp_error(message: &str) -> ToolFailure {
    let kind = if message.contains("断开") || message.contains("没有活连接") {
        ToolFailureKind::Transport
    } else if message.contains("合法 JSON") || message.contains("schema") {
        ToolFailureKind::InvalidParams
    } else if message.contains("拒绝") || message.contains("权限") {
        ToolFailureKind::SandboxDenied
    } else {
        ToolFailureKind::Upstream
    };
    ToolFailure::new(kind, message.to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::*;
    use crate::mcp::registry::ConnectedServer;
    use crate::mcp::{Guard, McpTool};

    #[test]
    fn bridge_marks_declared_tools_as_exposed() {
        let mut servers = BTreeMap::new();
        servers.insert(
            "fs".to_string(),
            ConnectedServer {
                label: "文件".to_string(),
                tools: vec![McpTool {
                    name: "read_file".to_string(),
                    description: "读取文件".to_string(),
                    input_schema: json!({"type":"object"}),
                }],
                guard: Guard::default(),
                client: None,
                roots: None,
            },
        );
        let registry = McpRegistry {
            servers,
            statuses: Vec::new(),
        };

        let infos = runtime_mcp_tool_infos(&registry);
        assert_eq!(infos.len(), 1);
        assert!(infos[0].exposed_to_model);
        assert_eq!(runtime_mcp_specs(&registry).len(), 1);
    }

    #[tokio::test]
    async fn mcp_runtime_tool_reports_missing_connection_as_transport() {
        let mut servers = BTreeMap::new();
        servers.insert(
            "fs".to_string(),
            ConnectedServer {
                label: "文件".to_string(),
                tools: vec![McpTool {
                    name: "read_file".to_string(),
                    description: "读取文件".to_string(),
                    input_schema: json!({"type":"object"}),
                }],
                guard: Guard::default(),
                client: None,
                roots: None,
            },
        );
        let registry = McpRegistry {
            servers,
            statuses: Vec::new(),
        };
        let spec = runtime_mcp_specs(&registry).remove(0);
        let handler = McpRuntimeTool::new(Arc::new(RwLock::new(registry)), spec);
        let dir = tempfile::tempdir().unwrap();
        let ctx = RuntimeContext::new(
            dir.path(),
            crate::agent_runtime::RuntimePolicy::workspace_write(dir.path()),
        );
        let outcome = handler
            .execute(
                &ctx,
                ToolCallRequest::new("fs__read_file", json!({"path": "a.txt"})),
            )
            .await;

        assert_eq!(outcome.status, crate::agent_runtime::ToolStatus::Failed);
        assert_eq!(
            outcome.failure.as_ref().unwrap().kind,
            ToolFailureKind::Transport
        );
    }

    #[tokio::test]
    async fn mcp_runtime_tool_blocks_out_of_boundary_paths() {
        let mut servers = BTreeMap::new();
        servers.insert(
            "custom".to_string(),
            ConnectedServer {
                label: "外部服务".to_string(),
                tools: vec![McpTool {
                    name: "read_custom".to_string(),
                    description: "读取文件".to_string(),
                    input_schema: json!({"type":"object"}),
                }],
                guard: Guard::default(),
                client: None,
                roots: None,
            },
        );
        let registry = McpRegistry {
            servers,
            statuses: Vec::new(),
        };
        let spec = runtime_mcp_specs(&registry).remove(0);
        let handler = McpRuntimeTool::new(Arc::new(RwLock::new(registry)), spec);
        let dir = tempfile::tempdir().unwrap();
        let ctx = RuntimeContext::new(
            dir.path(),
            crate::agent_runtime::RuntimePolicy::workspace_write(dir.path()),
        );

        // 试图访问超出 workspace_root 的外部路径
        let outcome = handler
            .execute(
                &ctx,
                ToolCallRequest::new("custom__read_custom", json!({"path": "/etc/passwd"})),
            )
            .await;

        assert_eq!(outcome.status, crate::agent_runtime::ToolStatus::Blocked);
        assert_eq!(
            outcome.failure.as_ref().unwrap().kind,
            ToolFailureKind::SandboxDenied
        );
        assert!(outcome.content.contains("工作区沙箱拦截"));
    }

    #[tokio::test]
    async fn mcp_runtime_tool_blocks_system_audio_player_commands() {
        let mut servers = BTreeMap::new();
        servers.insert(
            "system".to_string(),
            ConnectedServer {
                label: "系统".to_string(),
                tools: vec![McpTool {
                    name: "run_command".to_string(),
                    description: "在 shell 里执行一条命令".to_string(),
                    input_schema: json!({"type":"object"}),
                }],
                guard: Guard::default(),
                client: None,
                roots: None,
            },
        );
        let registry = McpRegistry {
            servers,
            statuses: Vec::new(),
        };
        let spec = runtime_mcp_specs(&registry).remove(0);
        let handler = McpRuntimeTool::new(Arc::new(RwLock::new(registry)), spec);
        let dir = tempfile::tempdir().unwrap();
        let ctx = RuntimeContext::new(
            dir.path(),
            crate::agent_runtime::RuntimePolicy::workspace_write(dir.path()),
        );

        let outcome = handler
            .execute(
                &ctx,
                ToolCallRequest::new(
                    "system__run_command",
                    json!({ "command": "nohup afplay /tmp/song.flac >/dev/null 2>&1 &" }),
                ),
            )
            .await;

        assert_eq!(outcome.status, crate::agent_runtime::ToolStatus::Blocked);
        assert_eq!(
            outcome.failure.as_ref().unwrap().kind,
            ToolFailureKind::SandboxDenied
        );
        assert!(outcome.content.contains("play_audio"), "{}", outcome.content);
    }
}
