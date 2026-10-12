//! 多个 MCP server 的聚合层：把所有 server 的工具汇成一张表交给模型，
//! 再把模型的调用路由回对应 server —— 并且**必须经过闸门**。
//!
//! ## 「需要确认」用类型强制，而不是靠调用方自觉
//!
//! 闸门([`Guard`])只产出裁决，真正阻止执行得靠这里。如果 `execute()` 直接收
//! 工具名和参数，那么调用方少写一个 `if verdict.allowed()` 就等于闸门完全失效，
//! 而这种遗漏编译器发现不了、测试也容易漏。
//!
//! 所以 [`McpRegistry::execute`] 只接受 [`ApprovedCall`]，而它**无法凭空构造**：
//!
//! ```text
//! authorize() ─→ Ready(ApprovedCall)              直接可执行
//!             ├→ NeedsConfirmation(PendingCall)    只有 .approve() 能换出 ApprovedCall
//!             └→ Rejected(Verdict)                 拿不到 ApprovedCall，执行不了
//! ```
//!
//! 想跳过确认就得显式写出 `.approve()` —— 那是有意为之，不会是手滑。

use std::collections::BTreeMap;

use serde::Serialize;

use super::client::{McpClient, McpTool};
use super::config::{qualified_tool_name, split_tool_name, McpServerConfig};
use super::guard::{Guard, Verdict};
use super::permission::{Access, DecisionReason, Operation};
use crate::error::{Error, Result};
use crate::llm::Tool;

/// 单个 server 的连接状态，给设置界面看。
#[derive(Debug, Clone, Serialize)]
pub struct ServerStatus {
    pub id: String,
    pub label: String,
    pub connected: bool,
    pub tool_count: usize,
    /// 连接失败的原因。**要展示给用户** —— server 起不来的原因基本都在这句里
    /// （命令拼错、缺依赖、缺凭据），藏起来会让人完全没法排查。
    pub error: Option<String>,
}

/// 一个工具在界面上的呈现（含闸门对它的静态判定）
#[derive(Debug, Clone, Serialize)]
pub struct ToolInfo {
    /// 给模型看的名字：`server__tool`
    pub qualified_name: String,
    pub server_id: String,
    pub name: String,
    pub description: String,
    pub operation: Operation,
    pub access: Access,
    /// 判定依据（配的还是猜的）。猜的会出错，用户有权知道
    pub reason: DecisionReason,
}

pub struct ConnectedServer {
    pub label: String,
    pub tools: Vec<McpTool>,
    pub guard: Guard,
    /// `None` 表示只有目录、没有活连接。
    ///
    /// 做成 Option 而不是必填，是为了让「工具声明、闸门裁决、调用路由」这些
    /// 纯逻辑能在不起真实子进程的情况下测到 —— 这些恰恰是安全相关的部分，
    /// 不能因为"要起 server 才能测"就不测。执行时若为 None 会明确报错。
    pub client: Option<McpClient>,
    /// 这个 server 声明的可访问根目录（连上时问一次的结果）。
    ///
    /// **必须告诉模型。** 不说的话它只能猜路径 —— 实测：用户问
    /// `/Users/you/AI/workspace`，模型传了相对路径 `workspace`，
    /// 而 filesystem 服务把相对路径解析到**第一个**允许目录（Documents），
    /// 于是 ENOENT；更糟的是在那之前它已经回答"找到了，它在 xxx 里面"，
    /// 而那句话没有任何一次成功的工具调用支撑。
    pub roots: Option<String>,
}

/// 已批准、可以执行的调用。只能由 [`McpRegistry::authorize`] 产出。
#[derive(Debug, Clone)]
pub struct ApprovedCall {
    server_id: String,
    tool: String,
    arguments: serde_json::Value,
}

impl ApprovedCall {
    pub fn qualified_name(&self) -> String {
        qualified_tool_name(&self.server_id, &self.tool)
    }
}

/// 待确认的调用。用户点了「允许」之后调 [`PendingCall::approve`] 换成 [`ApprovedCall`]。
#[derive(Debug, Clone)]
pub struct PendingCall {
    call: ApprovedCall,
}

impl PendingCall {
    /// 用户已确认。**只应在真的拿到用户许可后调用。**
    pub fn approve(self) -> ApprovedCall {
        self.call
    }
    pub fn qualified_name(&self) -> String {
        self.call.qualified_name()
    }
    pub fn arguments(&self) -> &serde_json::Value {
        &self.call.arguments
    }
}

/// 闸门对一次调用的处置
#[derive(Debug)]
pub enum Authorization {
    /// 可以直接执行
    Ready(ApprovedCall),
    /// 需要用户确认；`verdict.reason()` 是要展示的说明
    NeedsConfirmation {
        call: PendingCall,
        verdict: Box<Verdict>,
    },
    /// 拒绝执行。拿不到 `ApprovedCall`，所以执行不了
    Rejected(Box<Verdict>),
}

#[derive(Default)]
pub struct McpRegistry {
    pub servers: BTreeMap<String, ConnectedServer>,
    pub statuses: Vec<ServerStatus>,
}

impl McpRegistry {
    /// 连接全部启用的 server。
    ///
    /// **单个 server 失败不影响其它 server** —— 一个配错的 server 不该让所有工具
    /// 都不可用；失败原因记进 [`ServerStatus::error`] 交给界面。
    pub async fn connect(configs: &[McpServerConfig], secrets: &BTreeMap<String, String>) -> Self {
        let mut servers = BTreeMap::new();
        let mut statuses = Vec::new();

        for config in configs {
            if !config.enabled {
                continue;
            }
            let mut status = ServerStatus {
                id: config.id.clone(),
                label: config.label.clone(),
                connected: false,
                tool_count: 0,
                error: None,
            };

            // 启动超时要真的生效：MCP server 起不来时 stdio 握手会一直挂着，
            // 没有超时就会把整个启动流程拖死
            let timeout = std::time::Duration::from_secs(config.startup_timeout_secs);
            match tokio::time::timeout(timeout, McpClient::connect(config, secrets)).await {
                Err(_) => {
                    status.error = Some(format!(
                        "启动超时（{} 秒）。首次启动可能要装依赖，可在设置里调大超时。",
                        config.startup_timeout_secs
                    ));
                }
                Ok(Err(err)) => status.error = Some(err.to_string()),
                Ok(Ok(client)) => match client.list_tools().await {
                    Err(err) => {
                        status.error = Some(format!("已连接但读取工具列表失败：{err}"));
                        client.shutdown().await;
                    }
                    Ok(tools) => {
                        status.connected = true;
                        status.tool_count = tools.len();

                        // 会自报可访问范围的 server 就问一次，把答案带给模型。
                        //
                        // 按**工具是否存在**来判断，而不是按 server id ——
                        // 任何 server 都可以提供这个工具，写死 id 只对 filesystem 有效。
                        let roots = if tools.iter().any(|t| t.name == "list_allowed_directories") {
                            client
                                .call("list_allowed_directories", serde_json::json!({}))
                                .await
                                .ok()
                                .map(|(t, _)| t)
                        } else {
                            None
                        };
                        tracing::info!(
                            server = %config.id,
                            tools = tools.len(),
                            "MCP server 就绪"
                        );
                        servers.insert(
                            config.id.clone(),
                            ConnectedServer {
                                label: config.label.clone(),
                                client: Some(client),
                                roots,
                                tools,
                                guard: Guard {
                                    mode: config.approval_mode,
                                    permissions: config.permissions.clone(),
                                    commands: config.commands.clone(),
                                    session_allowed_tools: std::collections::HashSet::new(),
                                },
                            },
                        );
                    }
                },
            }

            if let Some(err) = &status.error {
                tracing::warn!(server = %config.id, %err, "MCP server 不可用");
            }
            statuses.push(status);
        }

        Self { servers, statuses }
    }

    pub fn statuses(&self) -> &[ServerStatus] {
        &self.statuses
    }

    pub fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }

    /// 交给模型的工具声明。
    ///
    /// 被策略 Deny 的工具**连声明都不出现** —— 看不到就不会尝试，比"看得到但被拒"
    /// 少一轮往返，也不会让模型反复重试同一个必然失败的调用。
    pub fn tool_declarations(&self) -> Vec<Tool> {
        let mut out = Vec::new();
        for (server_id, server) in &self.servers {
            for tool in &server.tools {
                let decision = server
                    .guard
                    .permissions
                    .decide(&tool.name, &tool.description);
                if !decision.exposed() {
                    continue;
                }
                out.push(Tool {
                    name: qualified_tool_name(server_id, &tool.name),
                    // 把 server 名带进描述：模型面对几十个工具时，
                    // 知道它属于哪个系统才选得准
                    description: if tool.description.is_empty() {
                        format!("[{}]", server.label)
                    } else {
                        format!("[{}] {}", server.label, tool.description)
                    },
                    parameters: tool.input_schema.clone(),
                });
            }
        }
        out
    }

    /// 全部工具（含被 Deny 的）及其判定，给设置界面展示。
    pub fn describe_tools(&self) -> Vec<ToolInfo> {
        let mut out = Vec::new();
        for (server_id, server) in &self.servers {
            for tool in &server.tools {
                let decision = server
                    .guard
                    .permissions
                    .decide(&tool.name, &tool.description);
                out.push(ToolInfo {
                    qualified_name: qualified_tool_name(server_id, &tool.name),
                    server_id: server_id.clone(),
                    name: tool.name.clone(),
                    description: tool.description.clone(),
                    operation: decision.operation,
                    access: decision.access,
                    reason: decision.reason,
                });
            }
        }
        out
    }

    /// 针对特定工具开启本次会话的临时放行。用于响应口头授权。
    pub fn grant_session_permission(&mut self, server_id: &str, tool_name: &str) {
        if let Some(server) = self.servers.get_mut(server_id) {
            server
                .guard
                .session_allowed_tools
                .insert(tool_name.to_string());
        }
    }

    /// 过一次闸门。参数是模型给的**原始 JSON 字符串**（流式拼接而成）。
    pub fn authorize(&self, qualified: &str, raw_arguments: &str) -> Result<Authorization> {
        let (server_id, tool) = split_tool_name(qualified).ok_or_else(|| {
            Error::config(format!("工具名「{qualified}」缺少 server 前缀，无法路由"))
        })?;
        let server = self.servers.get(server_id).ok_or_else(|| {
            Error::config(format!(
                "找不到 MCP server「{server_id}」（工具 {qualified}）"
            ))
        })?;

        // 模型时不时会给空串或非法 JSON。空串按无参数处理；
        // 真的解析不了要如实报错，绝不能"当成空参数"就执行 ——
        // 那等于在参数未知的情况下放行，而参数正是风险所在
        let arguments = if raw_arguments.trim().is_empty() {
            serde_json::Value::Object(Default::default())
        } else {
            serde_json::from_str(raw_arguments)
                .map_err(|e| Error::config(format!("工具 {qualified} 的参数不是合法 JSON：{e}")))?
        };

        let description = server
            .tools
            .iter()
            .find(|t| t.name == tool)
            .map(|t| t.description.as_str())
            .unwrap_or_default();

        let verdict = server.guard.evaluate(tool, description, &arguments);
        let call = ApprovedCall {
            server_id: server_id.to_string(),
            tool: tool.to_string(),
            arguments,
        };

        Ok(if verdict.allowed() {
            Authorization::Ready(call)
        } else if verdict.needs_confirmation() {
            Authorization::NeedsConfirmation {
                call: PendingCall { call },
                verdict: Box::new(verdict),
            }
        } else {
            Authorization::Rejected(Box::new(verdict))
        })
    }

    /// 执行一次已批准的调用。
    pub async fn execute(&self, call: ApprovedCall) -> Result<(String, Vec<String>)> {
        let server = self
            .servers
            .get(&call.server_id)
            .ok_or_else(|| Error::config(format!("MCP server「{}」已断开", call.server_id)))?;
        let client = server.client.as_ref().ok_or_else(|| {
            Error::config(format!("MCP server「{}」当前没有活连接", call.server_id))
        })?;
        // **参数和结果都要记。**
        //
        // 原先只记工具名，于是"工具调了但没生效"这种情况在日志里看不出是
        // 参数错了、还是工具自己失败了 —— 实测撞到过：用户让开终端，
        // 日志显示 open_app 被调用、模型也说打开了，但终端没开，
        // 而日志里没有任何能区分这两种可能的信息。
        let args = serde_json::to_string(&call.arguments).unwrap_or_default();
        tracing::info!(
            server = %call.server_id,
            tool = %call.tool,
            args = %args.chars().take(400).collect::<String>(),
            "执行 MCP 工具"
        );
        let result = client.call(&call.tool, call.arguments).await;
        match &result {
            Ok((text, _)) => tracing::info!(
                server = %call.server_id,
                tool = %call.tool,
                result = %text.chars().take(400).collect::<String>(),
                "MCP 工具返回"
            ),
            Err(err) => tracing::warn!(
                server = %call.server_id,
                tool = %call.tool,
                %err,
                "MCP 工具失败"
            ),
        }
        result
    }

    /// 给系统提示词用的一段「工具使用须知」。没有可说的就返回 None。
    ///
    /// **存在的理由是模型会猜路径。** 实测：用户问
    /// `/Users/you/AI/workspace`，模型传了相对路径 `workspace`，
    /// filesystem 服务把它解析到第一个允许目录（Documents）→ ENOENT。
    /// 而在那之前模型已经回答"找到了，它在 xxx 里面" —— 那句话没有任何
    /// 一次成功的工具调用支撑。
    ///
    /// 告诉它能访问哪些根、并要求用绝对路径，比指望它先去问一次可靠得多。
    pub fn usage_hint(&self) -> Option<String> {
        let roots: Vec<&str> = self
            .servers
            .values()
            .filter_map(|s| s.roots.as_deref())
            .collect();
        if roots.is_empty() {
            return None;
        }
        Some(format!(
            "文件工具只能访问下列目录（这是硬限制，越界会被拒绝）：\n{}\n\
             调用文件工具时必须用**绝对路径**。相对路径会被解析到上面第一个目录，\
             那几乎总不是用户想要的。\n\
             不确定某个路径存不存在时，先用工具查，查到了再回答；\
             没查到就直说没找到，不要凭猜测宣布已经找到。",
            roots.join("\n")
        ))
    }

    pub async fn shutdown(self) {
        for (_, server) in self.servers {
            if let Some(client) = server.client {
                client.shutdown().await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::guard::{ApprovalMode, CommandPolicy};
    use crate::mcp::permission::ServerPermissions;
    use serde_json::json;

    /// 只有目录、没有活连接的注册表 —— 闸门与路由的逻辑不需要真起 server
    fn registry_with(tools: Vec<McpTool>, guard: Guard) -> McpRegistry {
        let mut servers = BTreeMap::new();
        servers.insert(
            "shell".to_string(),
            ConnectedServer {
                label: "本机终端".into(),
                client: None,
                roots: None,
                tools,
                guard,
            },
        );
        McpRegistry {
            servers,
            statuses: vec![],
        }
    }

    fn tool(name: &str, description: &str) -> McpTool {
        McpTool {
            name: name.into(),
            description: description.into(),
            input_schema: json!({"type":"object"}),
        }
    }

    #[test]
    fn denied_tools_are_not_declared_to_the_model() {
        let r = registry_with(
            vec![
                tool("list_files", "列出文件"),
                tool("delete_file", "删除文件"),
            ],
            Guard::default(), // delete = Deny
        );
        let names: Vec<String> = r.tool_declarations().into_iter().map(|t| t.name).collect();
        assert_eq!(names, vec!["shell__list_files"]);
        // 但界面上要能看到它、看到它为什么被禁
        assert_eq!(r.describe_tools().len(), 2);
    }

    #[test]
    fn declarations_are_namespaced_and_labelled() {
        let r = registry_with(vec![tool("search", "搜索")], Guard::default());
        let decl = &r.tool_declarations()[0];
        assert_eq!(decl.name, "shell__search");
        assert!(
            decl.description.starts_with("[本机终端]"),
            "{}",
            decl.description
        );
    }

    #[test]
    fn rejected_calls_cannot_be_executed() {
        let r = registry_with(
            vec![tool("run", "运行 shell 命令")],
            Guard {
                mode: ApprovalMode::Auto,
                permissions: ServerPermissions::permissive(),
                commands: CommandPolicy::default(),
                ..Default::default()
            },
        );
        let auth = r
            .authorize("shell__run", r#"{"command":"rm -rf /"}"#)
            .unwrap();
        match auth {
            Authorization::Rejected(v) => {
                assert!(v.reason().contains("递归删除根目录"), "{}", v.reason())
            }
            _ => panic!("抹掉根目录必须被拒绝"),
        }
    }

    #[test]
    fn whitelisted_command_is_ready_without_confirmation() {
        let r = registry_with(
            vec![tool("run", "运行 shell 命令")],
            Guard {
                mode: ApprovalMode::Conservative,
                permissions: ServerPermissions::permissive(),
                commands: CommandPolicy::default(),
                ..Default::default()
            },
        );
        let auth = r
            .authorize("shell__run", r#"{"command":"git status"}"#)
            .unwrap();
        assert!(matches!(auth, Authorization::Ready(_)));
    }

    #[test]
    fn unwhitelisted_command_needs_confirmation_and_says_what_it_is() {
        let r = registry_with(
            vec![tool("run", "运行 shell 命令")],
            Guard {
                mode: ApprovalMode::Auto,
                permissions: ServerPermissions::permissive(),
                commands: CommandPolicy::default(),
                ..Default::default()
            },
        );
        match r
            .authorize("shell__run", r#"{"command":"brew install cowsay"}"#)
            .unwrap()
        {
            Authorization::NeedsConfirmation { call, verdict } => {
                assert!(verdict.reason().contains("brew install"));
                // 只有显式 approve 才能拿到可执行的调用
                assert_eq!(call.approve().qualified_name(), "shell__run");
            }
            _ => panic!("非白名单命令必须确认"),
        }
    }

    #[test]
    fn malformed_arguments_error_out_instead_of_being_treated_as_empty() {
        // 参数解析不了就报错，绝不能当成空参数放行 —— 风险恰恰在参数里
        let r = registry_with(vec![tool("run", "运行 shell 命令")], Guard::default());
        let err = r.authorize("shell__run", "{not json").unwrap_err();
        assert!(err.to_string().contains("合法 JSON"), "{err}");
    }

    #[test]
    fn empty_arguments_are_accepted_as_no_arguments() {
        let r = registry_with(vec![tool("get_time", "读取当前时间")], Guard::default());
        assert!(matches!(
            r.authorize("shell__get_time", "").unwrap(),
            Authorization::Ready(_)
        ));
    }

    #[test]
    fn unknown_server_or_missing_prefix_is_reported() {
        let r = registry_with(vec![tool("run", "")], Guard::default());
        assert!(r.authorize("nosuch__run", "{}").is_err());
        assert!(r
            .authorize("run", "{}")
            .unwrap_err()
            .to_string()
            .contains("缺少 server 前缀"));
    }

    #[test]
    fn tool_names_containing_double_underscore_still_route() {
        // 还原时从左边切，所以工具名自身含双下划线也没问题
        let r = registry_with(vec![tool("read__file", "读取文件")], Guard::default());
        assert!(matches!(
            r.authorize("shell__read__file", "{}").unwrap(),
            Authorization::Ready(_)
        ));
    }
}
