//! MCP server 的管理 command。对齐 Claude Desktop：用户填配置就能接工具，
//! 不需要改代码、不需要重启应用。

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::config::secrets;
use crate::error::{Error, Result};
use crate::mcp::{McpRegistry, McpServerConfig, ServerStatus, ToolInfo};
use crate::AppState;

/// MCP 面板要展示的全部信息
#[derive(Debug, Serialize)]
pub struct McpView {
    /// 配置里的 server（含未启用的）
    pub servers: Vec<McpServerConfig>,
    /// 本次连接的结果。**要展示 error 字段** —— server 起不来的原因都在里面
    pub statuses: Vec<ServerStatus>,
    /// 已发现的工具及闸门对它们的判定
    pub tools: Vec<ToolInfo>,
}

#[tauri::command]
pub async fn mcp_view(state: State<'_, AppState>) -> Result<McpView> {
    let config = state.gateway.config();
    let servers = config.read().await.mcp_servers.clone();
    let registry = state.mcp.read().await;
    Ok(McpView {
        servers,
        statuses: registry.statuses().to_vec(),
        tools: registry.describe_tools(),
    })
}

/// 连接（或重连）全部启用的 server。
///
/// 每次都整体重建而不是增量补差：MCP 连接是子进程，增量维护要处理
/// "配置变了但进程还是老参数"这类状态，重建简单且不会留下幽灵进程。
#[tauri::command]
pub async fn mcp_reconnect(state: State<'_, AppState>) -> Result<Vec<ServerStatus>> {
    let config = state.gateway.config();
    let servers = config.read().await.mcp_servers.clone();

    // 凭据从环境变量/Keychain 读，不落进 config.json
    let mut store = std::collections::BTreeMap::new();
    for server in &servers {
        for secret_id in server.transport.secret_ids() {
            // 环境变量优先、Keychain 兜底（同 provider key 的策略，
            // 见 secrets.rs —— tauri dev 下反复弹钥匙串授权的坑就在那儿记着）
            if let Ok(Some(value)) = secrets::get_secret(&secret_id) {
                store.insert(secret_id, value);
            }
        }
    }

    let fresh = McpRegistry::connect(&servers, &store).await;
    let statuses = fresh.statuses().to_vec();

    // 先换上新的，再关旧的：中间不留"没有任何工具可用"的空窗
    let old = std::mem::replace(&mut *state.mcp.write().await, fresh);
    old.shutdown().await;

    Ok(statuses)
}

#[derive(Debug, Deserialize)]
pub struct UpsertServerArgs {
    pub server: McpServerConfig,
}

/// 新增或更新一个 server 配置，然后重连。
#[tauri::command]
pub async fn mcp_upsert_server(
    state: State<'_, AppState>,
    args: UpsertServerArgs,
) -> Result<Vec<ServerStatus>> {
    crate::mcp::config::validate_server_id(&args.server.id).map_err(Error::config)?;

    {
        let config = state.gateway.config();
        let mut config = config.write().await;
        match config
            .mcp_servers
            .iter_mut()
            .find(|s| s.id == args.server.id)
        {
            Some(existing) => *existing = args.server,
            None => config.mcp_servers.push(args.server),
        }
        config.save(&state.config_path)?;
    }

    mcp_reconnect(state).await
}

#[tauri::command]
pub async fn mcp_remove_server(
    state: State<'_, AppState>,
    server_id: String,
) -> Result<Vec<ServerStatus>> {
    {
        let config = state.gateway.config();
        let mut config = config.write().await;
        config.mcp_servers.retain(|s| s.id != server_id);
        config.save(&state.config_path)?;
    }
    mcp_reconnect(state).await
}

#[derive(Debug, Deserialize)]
pub struct PermissionArgs {
    pub server_id: String,
    pub permissions: crate::mcp::ServerPermissions,
    pub approval_mode: crate::mcp::guard::ApprovalMode,
    pub commands: crate::mcp::guard::CommandPolicy,
}

/// 改一个 server 的权限设置。
///
/// 改完直接落盘并热更新，不需要重连 —— 闸门是纯判断，
/// 但它在 registry 里，所以要重建 registry 的 guard。这里图省事整体重连。
#[tauri::command]
pub async fn mcp_set_permissions(
    state: State<'_, AppState>,
    args: PermissionArgs,
) -> Result<Vec<ServerStatus>> {
    {
        let config = state.gateway.config();
        let mut config = config.write().await;
        let server = config
            .mcp_servers
            .iter_mut()
            .find(|s| s.id == args.server_id)
            .ok_or_else(|| Error::config(format!("找不到 server「{}」", args.server_id)))?;
        server.permissions = args.permissions;
        server.approval_mode = args.approval_mode;
        server.commands = args.commands;
        config.save(&state.config_path)?;
    }
    mcp_reconnect(state).await
}

