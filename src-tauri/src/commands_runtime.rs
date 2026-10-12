//! Agent runtime commands.

use tauri::State;

use crate::agent_runtime::{
    runtime_mcp_prompts as list_runtime_mcp_prompts,
    runtime_mcp_resources as list_runtime_mcp_resources,
    runtime_mcp_tool_infos as list_runtime_mcp_tool_infos,
    ApprovalRequest,
    Job,
    RuntimeCreateJobArgs,
    RuntimeMcpToolInfo,
    RuntimeResolveApprovalArgs,
    RuntimeRunJobArgs,
};
use crate::error::Result;
use crate::AppState;

#[tauri::command]
pub async fn runtime_mcp_tool_infos(state: State<'_, AppState>) -> Result<Vec<RuntimeMcpToolInfo>> {
    let mcp = state.mcp.read().await;
    Ok(list_runtime_mcp_tool_infos(&mcp))
}

#[tauri::command]
pub async fn runtime_mcp_resources(state: State<'_, AppState>) -> Result<Vec<serde_json::Value>> {
    let mcp = state.mcp.read().await;
    list_runtime_mcp_resources(&mcp).await
}

#[tauri::command]
pub async fn runtime_mcp_prompts(state: State<'_, AppState>) -> Result<Vec<serde_json::Value>> {
    let mcp = state.mcp.read().await;
    list_runtime_mcp_prompts(&mcp).await
}

#[tauri::command]
pub async fn runtime_list_pending_approvals(
    state: State<'_, AppState>,
) -> Result<Vec<ApprovalRequest>> {
    Ok(state.runtime.list_pending_approvals().await)
}

#[tauri::command]
pub async fn runtime_resolve_approval(
    state: State<'_, AppState>,
    args: RuntimeResolveApprovalArgs,
) -> Result<bool> {
    state
        .runtime
        .resolve_approval(&args.id, args.decision, args.scope, args.reason)
        .await
}

#[tauri::command]
pub async fn runtime_create_job(
    state: State<'_, AppState>,
    args: RuntimeCreateJobArgs,
) -> Result<Job> {
    state.runtime.create_job(args).await
}

#[tauri::command]
pub async fn runtime_list_jobs(state: State<'_, AppState>) -> Result<Vec<Job>> {
    Ok(state.runtime.list_jobs().await)
}

#[tauri::command]
pub async fn runtime_start_job_runner(
    state: State<'_, AppState>,
    args: RuntimeRunJobArgs,
) -> Result<Job> {
    state.runtime.start_job_runner(args).await
}

#[tauri::command]
pub async fn runtime_cancel_job(
    state: State<'_, AppState>,
    job_id: String,
    reason: Option<String>,
) -> Result<Job> {
    state.runtime.cancel_job(&job_id, reason).await
}

#[tauri::command]
pub async fn runtime_pause_job(
    state: State<'_, AppState>,
    job_id: String,
    reason: Option<String>,
) -> Result<Job> {
    state.runtime.pause_job(&job_id, reason).await
}

#[tauri::command]
pub async fn runtime_resume_job(state: State<'_, AppState>, job_id: String) -> Result<Job> {
    state.runtime.resume_job(&job_id).await
}

#[tauri::command]
pub async fn runtime_export_job_log(state: State<'_, AppState>, job_id: String) -> Result<String> {
    state.runtime.export_job_log(&job_id).await
}

