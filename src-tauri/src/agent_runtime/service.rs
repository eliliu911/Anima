//! Application-level runtime service.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::Emitter;
use tokio::sync::{oneshot, Mutex, RwLock};

use super::{
    register_cinema_audio_tools, register_cinema_http_tools, register_cinema_tools,
    register_comfyui_tools, register_local_tools, runtime_mcp_specs, ApprovalDecision,
    ApprovalRequest, ApprovalScope, Artifact, EventLog, EvidenceBundle, Job, JobKind, JobStatus,
    JobStep, JobStepAction, McpRuntimeTool, RetryPolicy, RuntimeContext, RuntimeEvent, RuntimeIds,
    RuntimePolicy, StepStatus, ToolCallRequest, ToolFailure, ToolFailureKind, ToolOutcome,
    ToolRegistry, ToolSpec, ToolStatus,
};
use crate::error::{Error, Result};
use crate::llm::Tool;
use crate::mcp::McpRegistry;

pub const RUNTIME_APPROVAL_EVENT: &str = "runtime://approval_required";

#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeToolCallArgs {
    pub workspace_path: String,
    pub tool_name: String,
    #[serde(default)]
    pub arguments: Value,
    #[serde(default)]
    pub network_enabled: bool,
    #[serde(default)]
    pub approval_mode: RuntimeApprovalMode,
    /// 模型能力分级："local" (默认) 或 "cloud"
    #[serde(default)]
    pub model_tier: String,
}

impl Default for RuntimeToolCallArgs {
    fn default() -> Self {
        Self {
            workspace_path: String::new(),
            tool_name: String::new(),
            arguments: Value::Null,
            network_enabled: false,
            approval_mode: RuntimeApprovalMode::default(),
            model_tier: "local".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeToolCallResult {
    pub outcome: ToolOutcome,
    pub events: Vec<RuntimeEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum RuntimeApprovalMode {
    #[default]
    Wait,
    Deny,
}


#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeCreateJobArgs {
    pub kind: JobKind,
    pub title: String,
    pub steps: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeCreateCinemaJobArgs {
    pub kind: JobKind,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeSetJobStatusArgs {
    pub job_id: String,
    pub status: JobStatus,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeUpdateJobStepArgs {
    pub job_id: String,
    pub step_id: String,
    pub status: StepStatus,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    #[serde(default)]
    pub provenance: Vec<Artifact>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeSetJobStepActionArgs {
    pub job_id: String,
    pub step_id: String,
    #[serde(default)]
    pub action: Option<JobStepAction>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeRunJobArgs {
    pub job_id: String,
    #[serde(default)]
    pub workspace_path: Option<String>,
    #[serde(default)]
    pub network_enabled: bool,
    #[serde(default)]
    pub approval_mode: RuntimeApprovalMode,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeResolveApprovalArgs {
    pub id: String,
    pub decision: ApprovalDecision,
    #[serde(default)]
    pub scope: Option<ApprovalScope>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone)]
struct ApprovalResolution {
    decision: ApprovalDecision,
    reason: Option<String>,
}

struct PendingApproval {
    request: ApprovalRequest,
    workspace_path: PathBuf,
    tx: oneshot::Sender<ApprovalResolution>,
}

#[derive(Debug, Clone)]
struct ApprovalGrant {
    scope: ApprovalScope,
    workspace_path: Option<PathBuf>,
    tool_name: String,
    // **这里不存参数指纹。** Session / Workspace 授权按工具名匹配，
    // 理由见 `is_preapproved` 的注释。存一个谁也不读的字段会让人以为
    // 授权比实际更细。
}

#[derive(Clone)]
pub struct RuntimeService {
    registry: ToolRegistry,
    events: Arc<Mutex<EventLog>>,
    jobs: Arc<Mutex<BTreeMap<String, Job>>>,
    pending_approvals: Arc<Mutex<HashMap<String, PendingApproval>>>,
    approval_grants: Arc<Mutex<Vec<ApprovalGrant>>>,
    running_jobs: Arc<Mutex<HashSet<String>>>,
    app_handle: Arc<Mutex<Option<tauri::AppHandle>>>,
    job_store_path: Option<PathBuf>,
    evidence_store_path: Option<PathBuf>,
    session_id: String,
    /// 本会话已经读过的文件。
    ///
    /// **必须挂在 service 上，不能挂在 `RuntimeContext` 上** —— 每次工具调用都新建
    /// 一个 ctx，跟着 ctx 走的话 `view_file` 的标记活不到 `fs_write_text` 那一刻，
    /// 表现为模型永远无法覆写任何已存在的文件（全部错杀，不是漏放行）。
    /// service 正好是 per-session 的（单个 `session_id`），粒度对得上「本会话已读」。
    read_files: Arc<std::sync::Mutex<HashSet<PathBuf>>>,
    gateway: Arc<Mutex<Option<Arc<crate::llm::gateway::Gateway>>>>,
}

/// 会话档位 —— 决定**声明期**给模型看哪些工具。
///
/// 只影响声明，不影响注册：语音会话即使因为别的路径调到了编码工具，
/// 走的仍然是同一套策略与审批，而不是"工具不存在"这种不好排查的错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolProfile {
    /// 编码会话（带工作区）：全集。
    Coding,
    /// 陪伴 / 语音会话：不含重型编码运行时工具。
    Companion,
}

impl ToolProfile {
    fn filter(self, tools: Vec<Tool>) -> Vec<Tool> {
        let tools: Vec<Tool> = tools
            .into_iter()
            .filter(|tool| {
                !super::local_tools::HIDDEN_FROM_MODEL_TOOLS.contains(&tool.name.as_str())
            })
            .collect();

        match self {
            ToolProfile::Coding => tools,
            ToolProfile::Companion => tools
                .into_iter()
                .filter(|tool| {
                    !super::local_tools::CODING_RUNTIME_TOOLS.contains(&tool.name.as_str())
                })
                .collect(),
        }
    }
}

impl RuntimeService {
    pub fn new() -> Self {
        Self::new_with_app_handle(None)
    }

    pub fn with_app_handle_config_dir_and_system_tools(
        app_handle: tauri::AppHandle,
        config_dir: impl Into<PathBuf>,
        http: reqwest::Client,
        music_player: Arc<Mutex<Option<Arc<crate::audio::playback::Player>>>>,
    ) -> Self {
        let config_dir = config_dir.into();
        let mut service = Self::new_with_app_handle_and_job_store(
            Some(app_handle.clone()),
            Some(config_dir.join("runtime").join("jobs.json")),
        );
        crate::system_tools::register_runtime_tools(
            &mut service.registry,
            crate::system_tools::SystemToolDeps {
                app: app_handle.clone(),
                http: http.clone(),
                music_player,
            },
        );
        register_cinema_http_tools(
            &mut service.registry,
            config_dir,
            http,
            Some(app_handle.clone()),
        );
        service
    }

    /// 生产路径的构造器。
    ///
    /// `memory` 是 `AppState::memory` 那个**同一个**惰性格子（不是 `Arc<Memory>`）——
    /// 记忆系统要探嵌入后端的维度才能建库，启动这一刻它还是空的，
    /// 而 `search_knowledge` 被调用时已经填好了。传 `Arc<Memory>` 在这里根本拿不到。
    pub fn with_app_handle_config_dir_gateway_and_system_tools(
        app_handle: tauri::AppHandle,
        config_dir: impl Into<PathBuf>,
        http: reqwest::Client,
        music_player: Arc<Mutex<Option<Arc<crate::audio::playback::Player>>>>,
        gateway: Arc<crate::llm::gateway::Gateway>,
        memory: super::MemoryCell,
    ) -> Self {
        let config_dir = config_dir.into();
        let app_handle_for_studio = app_handle.clone();
        let mut service = Self::with_app_handle_config_dir_and_system_tools(
            app_handle,
            config_dir.clone(),
            http.clone(),
            music_player,
        );
        service.set_gateway(Arc::clone(&gateway));
        // Anima Chat 调 Studio 的工具（建项目、导剧本、建资产……）
        super::studio_provider::register_studio_tools(&mut service.registry, config_dir.clone(), Some(app_handle_for_studio), Some(Arc::clone(&gateway)));
        register_cinema_audio_tools(&mut service.registry, config_dir, gateway, http);
        service
            .registry
            .register(super::SearchKnowledgeTool::new(memory));
        service
    }

    pub fn set_gateway(&mut self, gateway: Arc<crate::llm::gateway::Gateway>) {
        // 子智能体的循环也要网关。它走进程级 cell，不跟着 service 走 ——
        // 工具是在 service 建好之前就注册进 registry 的，传不进去。
        super::subagent::set_gateway(Arc::clone(&gateway));
        if let Ok(mut g) = self.gateway.try_lock() {
            *g = Some(gateway);
        }
    }

    fn new_with_app_handle(app_handle: Option<tauri::AppHandle>) -> Self {
        Self::new_with_app_handle_and_job_store(app_handle, None)
    }

    fn new_with_app_handle_and_job_store(
        app_handle: Option<tauri::AppHandle>,
        job_store_path: Option<PathBuf>,
    ) -> Self {
        let mut registry = ToolRegistry::new();
        register_local_tools(&mut registry);
        if let Some(path) = &job_store_path {
            if let Some(config_dir) = path.parent().and_then(|runtime_dir| runtime_dir.parent()) {
                register_comfyui_tools(&mut registry, config_dir);
                register_cinema_tools(&mut registry, config_dir);
            }
        }
        let evidence_store_path = job_store_path.as_ref().map(|path| {
            path.parent()
                .map(|runtime_dir| runtime_dir.join("evidence_bundles.jsonl"))
                .unwrap_or_else(|| PathBuf::from("evidence_bundles.jsonl"))
        });
        let jobs = job_store_path
            .as_deref()
            .and_then(load_jobs_from_file)
            .unwrap_or_default();
        Self {
            registry,
            events: Arc::new(Mutex::new(EventLog::default())),
            jobs: Arc::new(Mutex::new(jobs)),
            pending_approvals: Arc::new(Mutex::new(HashMap::new())),
            approval_grants: Arc::new(Mutex::new(Vec::new())),
            running_jobs: Arc::new(Mutex::new(HashSet::new())),
            app_handle: Arc::new(Mutex::new(app_handle)),
            job_store_path,
            evidence_store_path,
            session_id: uuid::Uuid::new_v4().to_string(),
            read_files: Arc::new(std::sync::Mutex::new(HashSet::new())),
            gateway: Arc::new(Mutex::new(None)),
        }
    }

    #[cfg(test)]
    pub fn with_job_store(path: impl Into<PathBuf>) -> Self {
        Self::new_with_app_handle_and_job_store(None, Some(path.into()))
    }

    pub fn specs(&self) -> Vec<ToolSpec> {
        self.registry.specs()
    }

    pub fn contains_tool(&self, name: &str) -> bool {
        self.registry.contains(name)
    }

    pub fn tool_declarations(&self) -> Vec<Tool> {
        tool_declarations_from_specs(self.registry.specs())
    }

    pub async fn tool_declarations_with_mcp(&self, mcp: Arc<RwLock<McpRegistry>>) -> Vec<Tool> {
        tool_declarations_from_specs(self.registry_with_mcp(mcp).await.specs())
    }

    /// 按会话档位声明工具。注册表始终是全集，过滤只发生在这里 ——
    /// 因为 schema 是**按请求**付 token 的，而注册表在应用启动时只建一次。
    pub fn tool_declarations_for(&self, profile: ToolProfile) -> Vec<Tool> {
        profile.filter(self.tool_declarations())
    }

    pub async fn tool_declarations_with_mcp_for(
        &self,
        mcp: Arc<RwLock<McpRegistry>>,
        profile: ToolProfile,
    ) -> Vec<Tool> {
        profile.filter(self.tool_declarations_with_mcp(mcp).await)
    }

    pub async fn contains_tool_with_mcp(&self, name: &str, mcp: Arc<RwLock<McpRegistry>>) -> bool {
        self.registry_with_mcp(mcp).await.contains(name)
    }

    pub async fn recent_events(&self) -> Vec<RuntimeEvent> {
        self.events.lock().await.recent()
    }

    pub async fn list_pending_approvals(&self) -> Vec<ApprovalRequest> {
        self.pending_approvals
            .lock()
            .await
            .values()
            .map(|pending| pending.request.clone())
            .collect()
    }

    pub async fn resolve_approval(
        &self,
        id: &str,
        decision: ApprovalDecision,
        scope: Option<ApprovalScope>,
        reason: Option<String>,
    ) -> Result<bool> {
        let Some(pending) = self.pending_approvals.lock().await.remove(id) else {
            return Ok(false);
        };
        let mut scope = pending.request.clamp_scope(scope);

        // ── 「本项目允许」要真的落盘 ──
        //
        // `approval_grants` 是 `RuntimeService` 的内存 Vec，而 service 是
        // **per-session** 的 —— 重启应用或换会话就没了，用户会被重新问一遍
        // 而且不会说为什么。**按钮承诺了持久，行为没有。**
        //
        // 落盘的家是 `project.json` 的 `auto_approve_commands`：那本来就是
        // 「这个项目免审批什么」的唯一真相，而且已经被 policy.rs 执行 ——
        // 写进去之后连审批流程都不会再进入。
        //
        // 命令没法泛化成前缀时（复合命令、子 shell）**降级成会话级**，
        // 并把原因带回给前端。静默按项目级处理会让用户以为永久授权了，
        // 而下次又来问。
        let mut downgraded: Option<&'static str> = None;
        if decision == ApprovalDecision::Allow && scope == ApprovalScope::Workspace {
            match pending.request.grant_prefix.as_deref() {
                Some(prefix) if !pending.workspace_path.as_os_str().is_empty() => {
                    match crate::workspace::add_auto_approve_prefix(
                        &pending.workspace_path.to_string_lossy(),
                        prefix,
                    )
                    .await
                    {
                        Ok(_) => {}
                        Err(err) => {
                            tracing::warn!(%err, "写项目白名单失败，降级为会话级");
                            scope = ApprovalScope::Session;
                            downgraded = Some("写项目配置失败");
                        }
                    }
                }
                // 命令类但没法泛化（复合命令 / 子 shell / 程序名带路径）：
                // 降级成会话级。把复合命令降成一个前缀会顺带放行后半截。
                _ if pending.request.command_preview.is_some() => {
                    scope = ApprovalScope::Session;
                    downgraded = Some("这条命令没法泛化成白名单前缀");
                }
                // **非命令类工具（apply_patch / fs_write_text）按工具名落盘。**
                //
                // 它们没有可泛化的前缀，此前只记内存 —— 而 `apply_patch` 恰好是
                // 最常触发审批的那个，于是「本项目允许」在用户眼里完全没生效。
                _ if !pending.workspace_path.as_os_str().is_empty() => {
                    if let Err(err) = crate::workspace::add_auto_approve_tool(
                        &pending.workspace_path.to_string_lossy(),
                        &pending.request.tool_name,
                    )
                    .await
                    {
                        tracing::warn!(%err, "写项目免审批工具名单失败，降级为会话级");
                        scope = ApprovalScope::Session;
                        downgraded = Some("写项目配置失败");
                    }
                }
                _ => {
                    scope = ApprovalScope::Session;
                    downgraded = Some("这一轮没有绑定项目工作区");
                }
            }
        }
        if let Some(why) = downgraded {
            tracing::info!(why, tool = %pending.request.tool_name, "「本项目允许」已降级为本次会话");
        }

        if decision == ApprovalDecision::Allow && scope != ApprovalScope::Once {
            self.approval_grants.lock().await.push(ApprovalGrant {
                scope,
                workspace_path: (scope == ApprovalScope::Workspace)
                    .then(|| pending.workspace_path.clone()),
                tool_name: pending.request.tool_name.clone(),
            });
        }
        let _ = pending.tx.send(ApprovalResolution { decision, reason });
        Ok(true)
    }

    pub async fn create_job(&self, args: RuntimeCreateJobArgs) -> Result<Job> {
        let steps = args.steps.into_iter().map(JobStep::new).collect();
        let job = Job::new(args.kind, args.title, steps);
        self.insert_job(job).await
    }

    pub async fn create_cinema_job(&self, args: RuntimeCreateCinemaJobArgs) -> Result<Job> {
        let title = args
            .title
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| default_cinema_job_title(&args.kind).to_string());
        let steps = default_cinema_job_steps(&args.kind)
            .into_iter()
            .map(JobStep::new)
            .collect();
        let job = Job::new(args.kind, title, steps);
        self.insert_job(job).await
    }

    async fn insert_job(&self, job: Job) -> Result<Job> {
        let kind = serde_json::to_value(&job.kind)
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "generic".to_string());
        self.events.lock().await.push(RuntimeEvent::JobCreated {
            job_id: job.id.clone(),
            kind,
            title: job.title.clone(),
        });
        self.jobs.lock().await.insert(job.id.clone(), job.clone());
        self.persist_jobs().await?;
        Ok(job)
    }

    pub async fn list_jobs(&self) -> Vec<Job> {
        self.jobs.lock().await.values().cloned().collect()
    }

    pub async fn get_job(&self, job_id: &str) -> Option<Job> {
        self.jobs.lock().await.get(job_id).cloned()
    }

    pub async fn set_job_status(&self, args: RuntimeSetJobStatusArgs) -> Result<Job> {
        let mut jobs = self.jobs.lock().await;
        let job = jobs
            .get_mut(&args.job_id)
            .ok_or_else(|| Error::runtime(format!("未知 runtime job：{}", args.job_id)))?;
        match args.status {
            JobStatus::Pending => job.status = JobStatus::Pending,
            JobStatus::Running => job.resume(),
            JobStatus::Paused => job.pause(args.message.clone().unwrap_or_default()),
            JobStatus::WaitingApproval => job.mark_waiting_approval(),
            JobStatus::Succeeded => job.status = JobStatus::Succeeded,
            JobStatus::Failed => {
                job.status = JobStatus::Failed;
                job.error = args.message.clone();
            }
            JobStatus::Cancelled => {
                job.cancel(
                    args.message
                        .clone()
                        .unwrap_or_else(|| "用户取消任务".to_string()),
                );
            }
        }
        job.updated_at_ms = current_time_ms();
        let updated = job.clone();
        drop(jobs);
        self.events
            .lock()
            .await
            .push(RuntimeEvent::JobStatusChanged {
                job_id: updated.id.clone(),
                status: updated.status.clone(),
            });
        self.persist_jobs().await?;
        Ok(updated)
    }

    pub async fn update_job_step(&self, args: RuntimeUpdateJobStepArgs) -> Result<Job> {
        let mut jobs = self.jobs.lock().await;
        let job = jobs
            .get_mut(&args.job_id)
            .ok_or_else(|| Error::runtime(format!("未知 runtime job：{}", args.job_id)))?;
        let changed = match args.status {
            StepStatus::Pending => {
                if let Some(step) = job.steps.iter_mut().find(|step| step.id == args.step_id) {
                    step.status = StepStatus::Pending;
                    step.message = args.message.clone();
                    true
                } else {
                    false
                }
            }
            StepStatus::Running => job.start_step(&args.step_id),
            StepStatus::Succeeded => job.complete_step(&args.step_id, args.artifacts.clone()),
            StepStatus::Failed => job.fail_step(
                &args.step_id,
                args.message
                    .clone()
                    .unwrap_or_else(|| "步骤执行失败".to_string()),
            ),
            StepStatus::Skipped => job.skip_step(&args.step_id, args.message.clone()),
        };
        if !changed {
            return Err(Error::runtime(format!(
                "无法更新 job step：{} / {}，请检查步骤是否存在以及依赖是否完成",
                args.job_id, args.step_id
            )));
        }
        if !args.provenance.is_empty() {
            if let Some(step) = job.steps.iter_mut().find(|step| step.id == args.step_id) {
                step.provenance.extend(args.provenance.clone());
            }
        }
        let updated = job.clone();
        drop(jobs);
        self.events.lock().await.push(RuntimeEvent::JobStepChanged {
            job_id: updated.id.clone(),
            step_id: args.step_id,
            status: serde_json::to_value(args.status)
                .ok()
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .unwrap_or_else(|| "unknown".to_string()),
            message: args.message,
        });
        self.persist_jobs().await?;
        Ok(updated)
    }

    pub async fn set_job_step_action(&self, args: RuntimeSetJobStepActionArgs) -> Result<Job> {
        let mut jobs = self.jobs.lock().await;
        let job = jobs
            .get_mut(&args.job_id)
            .ok_or_else(|| Error::runtime(format!("未知 runtime job：{}", args.job_id)))?;
        if !job.set_step_action(&args.step_id, args.action) {
            return Err(Error::runtime(format!(
                "未知 runtime job step：{} / {}",
                args.job_id, args.step_id
            )));
        }
        let updated = job.clone();
        drop(jobs);
        self.persist_jobs().await?;
        Ok(updated)
    }

    pub async fn start_job_runner(&self, args: RuntimeRunJobArgs) -> Result<Job> {
        {
            let mut running = self.running_jobs.lock().await;
            if !running.insert(args.job_id.clone()) {
                return self
                    .get_job(&args.job_id)
                    .await
                    .ok_or_else(|| Error::runtime(format!("未知 runtime job：{}", args.job_id)));
            }
        }

        let job = self
            .set_job_status(RuntimeSetJobStatusArgs {
                job_id: args.job_id.clone(),
                status: JobStatus::Running,
                message: None,
            })
            .await?;
        let service = self.clone();
        let job_id = args.job_id.clone();
        tokio::spawn(async move {
            if let Err(err) = service.run_job_until_blocked(args).await {
                let _ = service
                    .set_job_status(RuntimeSetJobStatusArgs {
                        job_id: job_id.clone(),
                        status: JobStatus::Failed,
                        message: Some(err.to_string()),
                    })
                    .await;
            }
            service.running_jobs.lock().await.remove(&job_id);
        });
        Ok(job)
    }

    async fn run_job_until_blocked(&self, args: RuntimeRunJobArgs) -> Result<()> {
        let workspace_path = args
            .workspace_path
            .clone()
            .unwrap_or_else(default_runtime_workspace_path);
        loop {
            let Some(job) = self.get_job(&args.job_id).await else {
                return Err(Error::runtime(format!("未知 runtime job：{}", args.job_id)));
            };
            if matches!(
                job.status,
                JobStatus::Paused
                    | JobStatus::WaitingApproval
                    | JobStatus::Succeeded
                    | JobStatus::Failed
                    | JobStatus::Cancelled
            ) {
                return Ok(());
            }

            let Some(step) = next_runnable_step(&job) else {
                if job
                    .steps
                    .iter()
                    .all(|step| matches!(step.status, StepStatus::Succeeded | StepStatus::Skipped))
                {
                    self.set_job_status(RuntimeSetJobStatusArgs {
                        job_id: job.id,
                        status: JobStatus::Succeeded,
                        message: None,
                    })
                    .await?;
                } else {
                    self.pause_job(
                        &job.id,
                        Some("没有可执行步骤，等待依赖或人工处理".to_string()),
                    )
                    .await?;
                }
                return Ok(());
            };

            let Some(action) = step.action.clone() else {
                self.pause_job(
                    &job.id,
                    Some(format!("步骤「{}」尚未配置 runtime action", step.title)),
                )
                .await?;
                return Ok(());
            };

            self.update_job_step(RuntimeUpdateJobStepArgs {
                job_id: job.id.clone(),
                step_id: step.id.clone(),
                status: StepStatus::Running,
                message: Some("runtime runner 正在执行".to_string()),
                artifacts: Vec::new(),
                provenance: Vec::new(),
            })
            .await?;

            let outcome = match action {
                JobStepAction::RuntimeTool {
                    tool_name,
                    arguments,
                    network_enabled,
                } => {
                    self.execute_job_tool_step(
                        &workspace_path,
                        &job.id,
                        &step,
                        tool_name,
                        arguments,
                        args.network_enabled || network_enabled,
                        args.approval_mode,
                    )
                    .await?
                }
            };

            if outcome.status == ToolStatus::Ok {
                let artifacts = artifacts_from_tool_outcome(&outcome);
                let provenance = provenance_from_tool_outcome(&outcome);
                self.update_job_step(RuntimeUpdateJobStepArgs {
                    job_id: job.id.clone(),
                    step_id: step.id.clone(),
                    status: StepStatus::Succeeded,
                    message: Some("步骤完成".to_string()),
                    artifacts,
                    provenance,
                })
                .await?;
            } else {
                let failure_kind = outcome
                    .failure
                    .as_ref()
                    .map(|failure| format!("{:?}", failure.kind))
                    .unwrap_or_else(|| "unknown".to_string());
                self.update_job_step(RuntimeUpdateJobStepArgs {
                    job_id: job.id.clone(),
                    step_id: step.id.clone(),
                    status: StepStatus::Failed,
                    message: Some(format!("{}：{}", failure_kind, outcome.content)),
                    artifacts: Vec::new(),
                    provenance: vec![Artifact::Json {
                        label: "runtime_tool_failure".to_string(),
                        value: serde_json::json!({
                            "tool_name": outcome.tool_name,
                            "call_id": outcome.call_id,
                            "status": outcome.status,
                            "failure": outcome.failure,
                            "elapsed_ms": outcome.elapsed_ms,
                        }),
                    }],
                })
                .await?;
                return Ok(());
            }
        }
    }

    async fn execute_job_tool_step(
        &self,
        workspace_path: &str,
        job_id: &str,
        step: &JobStep,
        tool_name: String,
        arguments: Value,
        network_enabled: bool,
        approval_mode: RuntimeApprovalMode,
    ) -> Result<ToolOutcome> {
        let permissions =
            crate::workspace::workspace_get_permissions(workspace_path.to_string()).await?;
        let mut policy = RuntimePolicy::from_project_permissions(workspace_path, &permissions);
        if network_enabled {
            policy.network = super::NetworkPolicy::Enabled;
        }

        let ctx = RuntimeContext::with_shared_events(
            PathBuf::from(workspace_path),
            policy,
            RuntimeIds {
                session_id: self.session_id.clone(),
                turn_id: uuid::Uuid::new_v4().to_string(),
                job_id: Some(job_id.to_string()),
            },
            Arc::clone(&self.events),
            Arc::clone(&self.read_files),
        );
        ctx.emit(RuntimeEvent::TurnStarted {
            session_id: ctx.ids.session_id.clone(),
            turn_id: ctx.ids.turn_id.clone(),
        })
        .await;

        let request = ToolCallRequest::new(tool_name, arguments);
        if self.is_preapproved(&ctx, &request).await {
            ctx.grant_tool_call(request.id.clone()).await;
        }
        let outcome = self
            .dispatch_with_approval_and_retry(
                &ctx,
                request,
                self.registry.clone(),
                approval_mode,
                step.retry_policy,
            )
            .await?;
        self.record_evidence_from_outcome(&outcome).await?;

        ctx.emit(RuntimeEvent::TurnFinished {
            session_id: ctx.ids.session_id.clone(),
            turn_id: ctx.ids.turn_id.clone(),
        })
        .await;
        Ok(outcome)
    }

    pub async fn cancel_job(&self, job_id: &str, reason: Option<String>) -> Result<Job> {
        self.set_job_status(RuntimeSetJobStatusArgs {
            job_id: job_id.to_string(),
            status: JobStatus::Cancelled,
            message: reason,
        })
        .await
    }

    pub async fn pause_job(&self, job_id: &str, reason: Option<String>) -> Result<Job> {
        self.set_job_status(RuntimeSetJobStatusArgs {
            job_id: job_id.to_string(),
            status: JobStatus::Paused,
            message: reason.or_else(|| Some("用户暂停任务".to_string())),
        })
        .await
    }

    pub async fn resume_job(&self, job_id: &str) -> Result<Job> {
        self.set_job_status(RuntimeSetJobStatusArgs {
            job_id: job_id.to_string(),
            status: JobStatus::Running,
            message: None,
        })
        .await
    }

    pub async fn export_job_log(&self, job_id: &str) -> Result<String> {
        let job = self
            .get_job(job_id)
            .await
            .ok_or_else(|| Error::runtime(format!("未知 runtime job：{job_id}")))?;
        let events = self.recent_events().await;
        let related_events = events
            .into_iter()
            .filter(|event| event_job_id(event).is_some_and(|id| id == job_id))
            .collect::<Vec<_>>();
        serde_json::to_string_pretty(&serde_json::json!({
            "job": job,
            "events": related_events,
        }))
        .map_err(Error::from)
    }

    pub async fn list_evidence_bundles(&self) -> Result<Vec<EvidenceBundle>> {
        let Some(path) = &self.evidence_store_path else {
            return Ok(Vec::new());
        };
        load_evidence_bundles_from_file(path)
    }

    pub async fn execute_tool(&self, args: RuntimeToolCallArgs) -> Result<RuntimeToolCallResult> {
        self.execute_tool_with_registry(args, self.registry.clone())
            .await
    }

    pub async fn execute_tool_with_mcp(
        &self,
        args: RuntimeToolCallArgs,
        mcp: Arc<RwLock<McpRegistry>>,
    ) -> Result<RuntimeToolCallResult> {
        let registry = self.registry_with_mcp(mcp).await;
        self.execute_tool_with_registry(args, registry).await
    }

    async fn execute_tool_with_registry(
        &self,
        args: RuntimeToolCallArgs,
        registry: ToolRegistry,
    ) -> Result<RuntimeToolCallResult> {
        let permissions =
            crate::workspace::workspace_get_permissions(args.workspace_path.clone()).await?;
        let mut policy =
            RuntimePolicy::from_project_permissions(&args.workspace_path, &permissions);
        if args.network_enabled {
            policy.network = super::NetworkPolicy::Enabled;
        }

        let tier = match args.model_tier.as_str() {
            "cloud" => super::ModelTier::Cloud,
            _ => super::ModelTier::Local,
        };

        let ctx = RuntimeContext::with_shared_events(
            PathBuf::from(&args.workspace_path),
            policy,
            RuntimeIds {
                session_id: self.session_id.clone(),
                turn_id: uuid::Uuid::new_v4().to_string(),
                job_id: None,
            },
            Arc::clone(&self.events),
            Arc::clone(&self.read_files),
        )
        .with_model_tier(tier);

        ctx.emit(RuntimeEvent::TurnStarted {
            session_id: ctx.ids.session_id.clone(),
            turn_id: ctx.ids.turn_id.clone(),
        })
        .await;

        let request = ToolCallRequest::new(args.tool_name, args.arguments);
        if self.is_preapproved(&ctx, &request).await {
            ctx.grant_tool_call(request.id.clone()).await;
        }
        let outcome = self
            .dispatch_with_approval(&ctx, request, registry, args.approval_mode)
            .await?;
        self.record_evidence_from_outcome(&outcome).await?;

        ctx.emit(RuntimeEvent::TurnFinished {
            session_id: ctx.ids.session_id.clone(),
            turn_id: ctx.ids.turn_id.clone(),
        })
        .await;

        Ok(RuntimeToolCallResult {
            outcome,
            events: ctx.recent_events().await,
        })
    }

    async fn registry_with_mcp(&self, mcp: Arc<RwLock<McpRegistry>>) -> ToolRegistry {
        let mut registry = self.registry.clone();
        let specs = {
            let guard = mcp.read().await;
            runtime_mcp_specs(&guard)
        };
        for spec in specs {
            // 对齐 Codex / Claude Code 标准：工作区内建工具（view_file, fs_list_directory, fs_write_text 等）
            // 是文件操作的第一公民。屏蔽外部 MCP 重叠的 filesystem 工具，
            // 避免模型面对同类工具发生命名竞争与相对路径解析歧义。
            let is_duplicate_fs_tool = spec.name.starts_with("filesystem__")
                && matches!(
                    spec.name.as_str(),
                    "filesystem__list_directory"
                        | "filesystem__directory_tree"
                        | "filesystem__read_file"
                        | "filesystem__read_text_file"
                        | "filesystem__write_file"
                        | "filesystem__list_allowed_directories"
                );
            if !is_duplicate_fs_tool {
                registry.register(McpRuntimeTool::new(Arc::clone(&mcp), spec));
            }
        }
        let all_specs = registry.specs();
        registry.register(super::local_tools::ToolSearchTool::new(all_specs));
        registry
    }

    async fn dispatch_with_approval(
        &self,
        ctx: &RuntimeContext,
        request: ToolCallRequest,
        registry: ToolRegistry,
        approval_mode: RuntimeApprovalMode,
    ) -> Result<ToolOutcome> {
        self.dispatch_with_approval_and_retry(
            ctx,
            request,
            registry,
            approval_mode,
            RetryPolicy::default(),
        )
        .await
    }

    async fn dispatch_with_approval_and_retry(
        &self,
        ctx: &RuntimeContext,
        request: ToolCallRequest,
        registry: ToolRegistry,
        approval_mode: RuntimeApprovalMode,
        retry_policy: RetryPolicy,
    ) -> Result<ToolOutcome> {
        let router = super::ToolRouter::new(registry.clone()).with_retry_policy(retry_policy);
        let mut outcome = router.dispatch(ctx, request.clone()).await;
        if !is_approval_block(&outcome) {
            return Ok(outcome);
        }

        if approval_mode == RuntimeApprovalMode::Deny {
            return Ok(ToolOutcome::failed(
                &request,
                ToolFailure::new(
                    ToolFailureKind::ApprovalDenied,
                    approval_denied_without_prompt_message(&request, &outcome),
                ),
                Duration::from_millis(0),
            ));
        }

        let spec = self
            .registry
            .spec(&request.tool_name)
            .or_else(|| registry.spec(&request.tool_name))
            .ok_or_else(|| Error::runtime(format!("未知 runtime 工具：{}", request.tool_name)))?;
        let failure = outcome
            .failure
            .clone()
            .ok_or_else(|| Error::runtime("工具请求审批失败，但没有返回失败详情"))?;
        let resolution = self
            .request_approval(ctx, &request, &spec, failure.message)
            .await?;

        match resolution.decision {
            ApprovalDecision::Allow => {
                ctx.grant_tool_call(request.id.clone()).await;
                outcome = router.dispatch(ctx, request).await;
                Ok(outcome)
            }
            ApprovalDecision::Deny => Ok(ToolOutcome::failed(
                &request,
                ToolFailure::new(
                    ToolFailureKind::ApprovalDenied,
                    resolution
                        .reason
                        .filter(|reason| !reason.trim().is_empty())
                        .unwrap_or_else(|| "用户拒绝执行该工具调用".to_string()),
                ),
                Duration::from_millis(0),
            )),
        }
    }

    async fn request_approval(
        &self,
        ctx: &RuntimeContext,
        request: &ToolCallRequest,
        spec: &ToolSpec,
        reason: String,
    ) -> Result<ApprovalResolution> {
        let approval = ApprovalRequest::new(
            &request.tool_name,
            spec.operation,
            spec.risk,
            reason,
            &request.arguments,
        );
        let (tx, rx) = oneshot::channel();

        self.pending_approvals.lock().await.insert(
            approval.id.clone(),
            PendingApproval {
                request: approval.clone(),
                workspace_path: ctx.workspace_root.clone(),
                tx,
            },
        );

        ctx.emit(RuntimeEvent::ApprovalRequested {
            turn_id: ctx.ids.turn_id.clone(),
            request: approval.clone(),
        })
        .await;

        if let Some(app) = self.app_handle.lock().await.clone() {
            let _ = app.emit(RUNTIME_APPROVAL_EVENT, &approval);
        }

        rx.await
            .map_err(|_| Error::runtime("审批请求已取消，工具未执行"))
    }

    /// 这次调用是不是已经被授权过了。
    ///
    /// ## 授权粒度：Session / Workspace 按**工具名**，不按参数指纹
    ///
    /// 原先三个范围都要求 `arguments_fingerprint` 完全相同。对命令类工具还算能用
    /// （同一条命令会重复出现），但对 `apply_patch` **几乎完全无效** ——
    /// 每个补丁的参数都不一样，指纹就不一样，于是点了「本次会话允许」
    /// 下一个补丁照样问。用户看到的就是"授权根本没生效"。
    ///
    /// 按钮上写的是「本次会话允许」/「本项目允许」，那个语义本来就是
    /// **这个工具**，不是"这一次调用的复制品"。
    ///
    /// `Once` **不进这张表**：批准的那一次由 `grant_tool_call` 直接放行，
    /// 放行的就是手里那一个 pending 请求本身，不需要拿参数去比对。
    ///
    /// （这里原来写着「`Once` 仍然按指纹匹配」，但指纹算出来之后
    /// 全仓库没有一处读它 —— 那句话从来没有对应的代码。）
    ///
    /// **免审批不等于免策略。** 这里跳过的只有「问用户」，工具自己照样调
    /// `evaluate_path` / `evaluate_command`，`deny_paths` 和不可逆命令名单
    /// 照样拦得住。
    async fn is_preapproved(&self, ctx: &RuntimeContext, request: &ToolCallRequest) -> bool {
        // 项目配置里的免审批工具名（落盘的那份，重启后仍然有效）
        if ctx
            .policy
            .auto_approve_tools
            .iter()
            .any(|t| t == &request.tool_name)
        {
            return true;
        }
        self.approval_grants.lock().await.iter().any(|grant| {
            if grant.tool_name != request.tool_name {
                return false;
            }
            match grant.scope {
                // Once 是一次性的，批准的那一次由 `grant_tool_call` 直接放行，
                // 不进这张表
                ApprovalScope::Once => false,
                ApprovalScope::Session => true,
                ApprovalScope::Workspace => grant
                    .workspace_path
                    .as_ref()
                    .is_some_and(|path| path == &ctx.workspace_root),
            }
        })
    }

    async fn persist_jobs(&self) -> Result<()> {
        let Some(path) = &self.job_store_path else {
            return Ok(());
        };
        let jobs = self.jobs.lock().await.clone();
        save_jobs_to_file(path, &jobs)
    }

    async fn record_evidence_from_outcome(&self, outcome: &ToolOutcome) -> Result<()> {
        if outcome.status != ToolStatus::Ok
            || !matches!(outcome.tool_name.as_str(), "web_search" | "search_web")
        {
            return Ok(());
        }
        let Ok(bundle) = serde_json::from_str::<EvidenceBundle>(&outcome.content) else {
            return Ok(());
        };
        self.persist_evidence_bundle(&bundle).await
    }

    async fn persist_evidence_bundle(&self, bundle: &EvidenceBundle) -> Result<()> {
        let Some(path) = &self.evidence_store_path else {
            return Ok(());
        };
        append_evidence_bundle_to_file(path, bundle)
    }
}

fn default_runtime_workspace_path() -> String {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .to_string_lossy()
        .to_string()
}

fn tool_declarations_from_specs(specs: Vec<ToolSpec>) -> Vec<Tool> {
    specs
        .into_iter()
        .map(|spec| Tool {
            name: spec.name,
            description: spec.description,
            parameters: spec.parameters,
        })
        .collect()
}

fn is_approval_block(outcome: &ToolOutcome) -> bool {
    outcome.status == ToolStatus::Blocked
        && outcome
            .failure
            .as_ref()
            .is_some_and(|failure| failure.kind == ToolFailureKind::ApprovalDenied)
}

fn approval_denied_without_prompt_message(
    request: &ToolCallRequest,
    outcome: &ToolOutcome,
) -> String {
    let reason = outcome
        .failure
        .as_ref()
        .map(|failure| failure.message.as_str())
        .unwrap_or("工具调用需要用户确认");
    if let Some((server_id, tool_name)) = crate::mcp::config::split_tool_name(&request.tool_name) {
        return format!(
            "{reason}。语音链路不会弹出确认框。请向用户解释风险并询问是否允许；如果用户明确口头同意，请调用 `mcp_grant_permission`，传入 server_id: \"{server_id}\" 和 tool_name: \"{tool_name}\"，然后重试原工具。"
        );
    }
    format!("{reason}。语音链路不会弹出确认框，请改用无需确认的方式或询问用户。")
}

fn current_time_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn load_jobs_from_file(path: &Path) -> Option<BTreeMap<String, Job>> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn save_jobs_to_file(path: &Path, jobs: &BTreeMap<String, Job>) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(jobs)?;
    std::fs::write(path, bytes)?;
    Ok(())
}

fn load_evidence_bundles_from_file(path: &Path) -> Result<Vec<EvidenceBundle>> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Ok(Vec::new());
    };
    let mut bundles = Vec::new();
    for line in content.lines().filter(|line| !line.trim().is_empty()) {
        bundles.push(serde_json::from_str(line)?);
    }
    Ok(bundles)
}

fn append_evidence_bundle_to_file(path: &Path, bundle: &EvidenceBundle) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut line = serde_json::to_string(bundle)?;
    line.push('\n');
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(line.as_bytes())?;
    Ok(())
}

fn next_runnable_step(job: &Job) -> Option<JobStep> {
    job.steps
        .iter()
        .find(|step| {
            step.status == StepStatus::Pending
                && step.depends_on.iter().all(|dep| {
                    job.steps.iter().any(|candidate| {
                        candidate.id == *dep && candidate.status == StepStatus::Succeeded
                    })
                })
        })
        .cloned()
}

fn artifacts_from_tool_outcome(outcome: &ToolOutcome) -> Vec<Artifact> {
    let mut artifacts = Vec::new();
    if !outcome.content.trim().is_empty() {
        artifacts.push(Artifact::Text {
            label: "tool_output".to_string(),
            content: outcome.content.clone(),
        });
    }
    for path in &outcome.artifacts {
        if path.starts_with("http://") || path.starts_with("https://") {
            artifacts.push(Artifact::Url {
                label: "tool_artifact".to_string(),
                url: path.clone(),
            });
        } else {
            artifacts.push(Artifact::File {
                label: "tool_artifact".to_string(),
                path: path.clone(),
            });
        }
    }
    artifacts
}

fn provenance_from_tool_outcome(outcome: &ToolOutcome) -> Vec<Artifact> {
    let mut provenance = vec![Artifact::Json {
        label: "runtime_tool".to_string(),
        value: serde_json::json!({
            "tool_name": outcome.tool_name,
            "call_id": outcome.call_id,
            "elapsed_ms": outcome.elapsed_ms,
        }),
    }];
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&outcome.content) {
        provenance.push(Artifact::Json {
            label: "runtime_tool_output".to_string(),
            value,
        });
    }
    provenance
}

fn event_job_id(event: &RuntimeEvent) -> Option<&str> {
    match event {
        RuntimeEvent::JobCreated { job_id, .. }
        | RuntimeEvent::JobStatusChanged { job_id, .. }
        | RuntimeEvent::JobStepChanged { job_id, .. } => Some(job_id),
        _ => None,
    }
}

fn default_cinema_job_title(kind: &JobKind) -> &'static str {
    match kind {
        JobKind::ScriptPlanning => "剧本企划",
        JobKind::CharacterAssets => "角色资产",
        JobKind::Storyboard => "导演分镜",
        JobKind::ShotVideo => "动态镜头视频",
        JobKind::AudioGeneration => "音频生成",
        JobKind::EpisodeAssembly => "成片合成",
        JobKind::ExportJianying => "剪映导出",
        JobKind::Generic => "通用任务",
    }
}

fn default_cinema_job_steps(kind: &JobKind) -> Vec<&'static str> {
    match kind {
        JobKind::ScriptPlanning => vec![
            "解析短剧题材与目标受众",
            "生成故事梗概与人物小传",
            "拆解集数结构",
            "输出影视分镜表草案",
        ],
        JobKind::CharacterAssets => vec![
            "确认角色底图与身份锚点",
            "生成全身三视图",
            "生成表情模组",
            "生成动态姿势",
            "生成面部机位",
            "质检一致性与可用素材",
        ],
        JobKind::Storyboard => vec![
            "读取剧本与角色资产",
            "生成导演分镜",
            "生成镜头提示词",
            "标注机位、运动与转场",
            "质检镜头连续性",
        ],
        JobKind::ShotVideo => vec![
            "选择视频工作流与模型",
            "准备首帧/尾帧/参考图",
            "生成动态镜头视频",
            "质检人物稳定性与镜头运动",
            "失败重试或替换工作流",
        ],
        JobKind::AudioGeneration => vec![
            "生成台词与旁白文本",
            "选择音色与情绪参数",
            "生成语音文件",
            "生成背景音与音效",
            "混音并对齐镜头时长",
        ],
        JobKind::EpisodeAssembly => vec![
            "收集镜头视频与音频",
            "按分镜顺序拼接",
            "生成字幕",
            "统一色彩与响度",
            "导出成片",
        ],
        JobKind::ExportJianying => vec![
            "收集成片视频、对白音频与字幕",
            "生成剪映 draft_content.json",
            "校验素材路径与时间线",
            "导出剪映工程目录",
        ],
        JobKind::Generic => vec!["准备任务", "执行任务", "检查结果"],
    }
}

impl Default for RuntimeService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::agent_runtime::ToolStatus;

    #[tokio::test]
    async fn service_executes_local_tools_with_shared_events() {
        let dir = tempfile::tempdir().unwrap();
        crate::workspace::workspace_init(crate::workspace::WorkspaceInitReq {
            path: dir.path().to_string_lossy().to_string(),
            project_name: "runtime-test".to_string(),
            template_type: None,
        })
        .await
        .unwrap();

        let service = RuntimeService::new();
        let result = service
            .execute_tool(RuntimeToolCallArgs {
                workspace_path: dir.path().to_string_lossy().to_string(),
                tool_name: "fs_write_text".to_string(),
                arguments: json!({ "path": "hello.txt", "content": "hi" }),
                network_enabled: false,
                approval_mode: RuntimeApprovalMode::Wait,
                model_tier: "local".to_string(),
            })
            .await
            .unwrap();

        assert_eq!(result.outcome.status, ToolStatus::Ok);
        assert!(dir.path().join("hello.txt").is_file());
        assert!(service.recent_events().await.len() >= 4);
    }

    /// 「先读后写」必须走**生产路径**验证：`view_file` 和 `fs_write_text` 是两次
    /// 独立的 `execute_tool` 调用，各自新建一个 `RuntimeContext`。
    ///
    /// `local_tools.rs` 里那些测试都是「造一个 ctx 连续 dispatch」，读集合天然共享，
    /// **抓不到这个** —— 曾经就是这样：测试全绿，而生产上模型永远无法覆写任何
    /// 已存在的文件。所以这条测试的价值全在「两次调用」这个形状上，别改成共用 ctx。
    #[tokio::test]
    async fn read_marker_survives_across_tool_calls_in_production_path() {
        let dir = tempfile::tempdir().unwrap();
        crate::workspace::workspace_init(crate::workspace::WorkspaceInitReq {
            path: dir.path().to_string_lossy().to_string(),
            project_name: "rbw-test".to_string(),
            template_type: None,
        })
        .await
        .unwrap();
        std::fs::write(dir.path().join("existing.txt"), "old content\n").unwrap();

        let service = RuntimeService::new();
        let ws = dir.path().to_string_lossy().to_string();
        let call = |tool: &str, args: serde_json::Value| RuntimeToolCallArgs {
            workspace_path: ws.clone(),
            tool_name: tool.to_string(),
            arguments: args,
            network_enabled: false,
            approval_mode: RuntimeApprovalMode::Wait,
            model_tier: "local".to_string(),
        };

        let read = service
            .execute_tool(call("view_file", json!({ "path": "existing.txt" })))
            .await
            .unwrap();
        assert_eq!(read.outcome.status, ToolStatus::Ok, "读取应当成功");

        let write = service
            .execute_tool(call(
                "fs_write_text",
                json!({ "path": "existing.txt", "content": "new content\n" }),
            ))
            .await
            .unwrap();
        assert_eq!(
            write.outcome.status,
            ToolStatus::Ok,
            "已读标记必须跨工具调用存活，否则模型永远无法覆写任何已存在的文件。实际：{}",
            write.outcome.content
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("existing.txt")).unwrap(),
            "new content\n"
        );
    }

    /// 反向：没读过就写，仍然要被拦。修 bug 时很容易把闸门整个拆掉。
    #[tokio::test]
    async fn write_without_read_is_still_blocked_in_production_path() {
        let dir = tempfile::tempdir().unwrap();
        crate::workspace::workspace_init(crate::workspace::WorkspaceInitReq {
            path: dir.path().to_string_lossy().to_string(),
            project_name: "rbw-neg".to_string(),
            template_type: None,
        })
        .await
        .unwrap();
        std::fs::write(dir.path().join("untouched.txt"), "precious\n").unwrap();

        let service = RuntimeService::new();
        let out = service
            .execute_tool(RuntimeToolCallArgs {
                workspace_path: dir.path().to_string_lossy().to_string(),
                tool_name: "fs_write_text".to_string(),
                arguments: json!({ "path": "untouched.txt", "content": "clobbered\n" }),
                network_enabled: false,
                approval_mode: RuntimeApprovalMode::Wait,
                model_tier: "local".to_string(),
            })
            .await
            .unwrap();

        assert_ne!(out.outcome.status, ToolStatus::Ok, "未读就覆写必须被拦");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("untouched.txt")).unwrap(),
            "precious\n",
            "被拦下时文件内容不能有任何改动"
        );
    }

    /// 新建文件不受约束——路径不存在时直接放行。
    #[tokio::test]
    async fn creating_a_new_file_needs_no_prior_read() {
        let dir = tempfile::tempdir().unwrap();
        crate::workspace::workspace_init(crate::workspace::WorkspaceInitReq {
            path: dir.path().to_string_lossy().to_string(),
            project_name: "rbw-new".to_string(),
            template_type: None,
        })
        .await
        .unwrap();

        let service = RuntimeService::new();
        let out = service
            .execute_tool(RuntimeToolCallArgs {
                workspace_path: dir.path().to_string_lossy().to_string(),
                tool_name: "fs_write_text".to_string(),
                arguments: json!({ "path": "brand_new.txt", "content": "hello\n" }),
                network_enabled: false,
                approval_mode: RuntimeApprovalMode::Wait,
                model_tier: "local".to_string(),
            })
            .await
            .unwrap();

        assert_eq!(
            out.outcome.status,
            ToolStatus::Ok,
            "{}",
            out.outcome.content
        );
        assert!(dir.path().join("brand_new.txt").is_file());
    }

    #[tokio::test]
    async fn service_tracks_jobs() {
        let service = RuntimeService::new();
        let job = service
            .create_job(RuntimeCreateJobArgs {
                kind: crate::agent_runtime::JobKind::Generic,
                title: "代码开发任务".to_string(),
                steps: vec!["读取代码".to_string(), "运行测试".to_string()],
            })
            .await
            .unwrap();

        assert_eq!(job.steps.len(), 2);
        assert!(service.get_job(&job.id).await.is_some());
        assert_eq!(service.list_jobs().await.len(), 1);
    }

    #[tokio::test]
    async fn service_persists_jobs_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runtime").join("jobs.json");
        let service = RuntimeService::with_job_store(&path);
        let job = service
            .create_job(RuntimeCreateJobArgs {
                kind: crate::agent_runtime::JobKind::Storyboard,
                title: "导演分镜".to_string(),
                steps: vec!["镜头表".to_string()],
            })
            .await
            .unwrap();
        assert!(path.is_file());

        let restored = RuntimeService::with_job_store(&path);
        let loaded = restored.get_job(&job.id).await.unwrap();
        assert_eq!(loaded.title, "导演分镜");
    }

    #[tokio::test]
    async fn service_persists_evidence_bundles_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runtime").join("jobs.json");
        let service = RuntimeService::with_job_store(&path);
        let bundle = EvidenceBundle::new(crate::agent_runtime::ResearchQuery::new(
            crate::agent_runtime::ResearchTrigger::DependencyResolution,
            "ComfyUI video workflow",
            "选择默认工作流需要证据",
        ))
        .with_item(crate::agent_runtime::EvidenceItem::new(
            "ComfyUI docs",
            "https://example.com/comfyui",
            "workflow summary",
            "docs",
        ))
        .with_conclusion("可作为候选工作流。");

        service.persist_evidence_bundle(&bundle).await.unwrap();
        let restored = RuntimeService::with_job_store(&path);
        let bundles = restored.list_evidence_bundles().await.unwrap();
        assert_eq!(bundles.len(), 1);
        assert_eq!(bundles[0].query.query, "ComfyUI video workflow");
    }

    #[tokio::test]
    async fn service_creates_cinema_job_templates() {
        let service = RuntimeService::new();
        let job = service
            .create_cinema_job(RuntimeCreateCinemaJobArgs {
                kind: crate::agent_runtime::JobKind::CharacterAssets,
                title: None,
            })
            .await
            .unwrap();

        assert_eq!(job.title, "角色资产");
        assert!(job.steps.iter().any(|step| step.title == "生成全身三视图"));
        assert!(job
            .steps
            .iter()
            .any(|step| step.title == "质检一致性与可用素材"));

        let export = service
            .create_cinema_job(RuntimeCreateCinemaJobArgs {
                kind: crate::agent_runtime::JobKind::ExportJianying,
                title: None,
            })
            .await
            .unwrap();
        assert_eq!(export.title, "剪映导出");
        assert!(export
            .steps
            .iter()
            .any(|step| step.title == "生成剪映 draft_content.json"));
    }

    #[tokio::test]
    async fn service_registers_cinema_runtime_tools_with_config_dir() {
        let dir = tempfile::tempdir().unwrap();
        let service = RuntimeService::with_job_store(dir.path().join("runtime").join("jobs.json"));
        assert!(service.contains_tool("cinema_export_jianying"));
        assert!(service.contains_tool("cinema_render_shot_video"));
        assert!(service.contains_tool("cinema_stitch_episode"));

        crate::workspace::workspace_init(crate::workspace::WorkspaceInitReq {
            path: dir.path().to_string_lossy().to_string(),
            project_name: "cinema-runtime-tool-test".to_string(),
            template_type: None,
        })
        .await
        .unwrap();

        let result = service
            .execute_tool(RuntimeToolCallArgs {
                workspace_path: dir.path().to_string_lossy().to_string(),
                tool_name: "cinema_export_jianying".to_string(),
                arguments: json!({ "project_id": "p1" }),
                network_enabled: false,
                approval_mode: RuntimeApprovalMode::Deny,
                model_tier: "local".to_string(),
            })
            .await
            .unwrap();
        assert_eq!(result.outcome.status, ToolStatus::Failed);
        assert!(result.outcome.content.contains("episode_id"));

        let result = service
            .execute_tool(RuntimeToolCallArgs {
                workspace_path: dir.path().to_string_lossy().to_string(),
                tool_name: "cinema_render_shot_video".to_string(),
                arguments: json!({ "project_id": "p1", "episode_id": "e1" }),
                network_enabled: false,
                approval_mode: RuntimeApprovalMode::Deny,
                model_tier: "local".to_string(),
            })
            .await
            .unwrap();
        assert_eq!(result.outcome.status, ToolStatus::Failed);
        assert!(result.outcome.content.contains("shot_id"));

        let result = service
            .execute_tool(RuntimeToolCallArgs {
                workspace_path: dir.path().to_string_lossy().to_string(),
                tool_name: "cinema_stitch_episode".to_string(),
                arguments: json!({ "project_id": "p1" }),
                network_enabled: false,
                approval_mode: RuntimeApprovalMode::Deny,
                model_tier: "local".to_string(),
            })
            .await
            .unwrap();
        assert_eq!(result.outcome.status, ToolStatus::Failed);
        assert!(result.outcome.content.contains("episode_id"));
    }

    #[tokio::test]
    async fn service_pauses_resumes_and_records_step_provenance() {
        let service = RuntimeService::new();
        let job = service
            .create_job(RuntimeCreateJobArgs {
                kind: crate::agent_runtime::JobKind::ShotVideo,
                title: "镜头视频".to_string(),
                steps: vec!["生成视频".to_string()],
            })
            .await
            .unwrap();
        let step_id = job.steps[0].id.clone();

        let paused = service
            .pause_job(&job.id, Some("等待工作流选择".to_string()))
            .await
            .unwrap();
        assert_eq!(paused.status, JobStatus::Paused);

        let resumed = service.resume_job(&job.id).await.unwrap();
        assert_eq!(resumed.status, JobStatus::Running);

        let updated = service
            .update_job_step(RuntimeUpdateJobStepArgs {
                job_id: job.id.clone(),
                step_id: step_id.clone(),
                status: StepStatus::Running,
                message: Some("使用 Wan I2V".to_string()),
                artifacts: Vec::new(),
                provenance: vec![Artifact::Json {
                    label: "workflow".to_string(),
                    value: json!({"name": "wan_i2v_default"}),
                }],
            })
            .await
            .unwrap();
        let step = updated
            .steps
            .iter()
            .find(|step| step.id == step_id)
            .unwrap();
        assert_eq!(step.provenance.len(), 1);
    }

    #[tokio::test]
    async fn service_runner_executes_configured_runtime_tool_steps() {
        let dir = tempfile::tempdir().unwrap();
        crate::workspace::workspace_init(crate::workspace::WorkspaceInitReq {
            path: dir.path().to_string_lossy().to_string(),
            project_name: "runner-test".to_string(),
            template_type: None,
        })
        .await
        .unwrap();

        let service = RuntimeService::new();
        let job = service
            .create_job(RuntimeCreateJobArgs {
                kind: crate::agent_runtime::JobKind::Generic,
                title: "runner".to_string(),
                steps: vec!["写入文件".to_string()],
            })
            .await
            .unwrap();
        let step_id = job.steps[0].id.clone();
        service
            .set_job_step_action(RuntimeSetJobStepActionArgs {
                job_id: job.id.clone(),
                step_id,
                action: Some(JobStepAction::RuntimeTool {
                    tool_name: "fs_write_text".to_string(),
                    arguments: json!({ "path": "runner-output.txt", "content": "ok" }),
                    network_enabled: false,
                }),
            })
            .await
            .unwrap();

        service
            .start_job_runner(RuntimeRunJobArgs {
                job_id: job.id.clone(),
                workspace_path: Some(dir.path().to_string_lossy().to_string()),
                network_enabled: false,
                approval_mode: RuntimeApprovalMode::Wait,
            })
            .await
            .unwrap();

        let finished = wait_for_job_status(&service, &job.id, JobStatus::Succeeded).await;
        assert_eq!(finished.progress(), 1.0);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("runner-output.txt")).unwrap(),
            "ok"
        );
        assert!(finished.steps[0].provenance.iter().any(
            |artifact| matches!(artifact, Artifact::Json { label, .. } if label == "runtime_tool")
        ));
    }

    #[tokio::test]
    async fn service_runner_pauses_unconfigured_steps() {
        let dir = tempfile::tempdir().unwrap();
        crate::workspace::workspace_init(crate::workspace::WorkspaceInitReq {
            path: dir.path().to_string_lossy().to_string(),
            project_name: "runner-paused-test".to_string(),
            template_type: None,
        })
        .await
        .unwrap();

        let service = RuntimeService::new();
        let job = service
            .create_job(RuntimeCreateJobArgs {
                kind: crate::agent_runtime::JobKind::CharacterAssets,
                title: "角色资产".to_string(),
                steps: vec!["生成三视图".to_string()],
            })
            .await
            .unwrap();

        service
            .start_job_runner(RuntimeRunJobArgs {
                job_id: job.id.clone(),
                workspace_path: Some(dir.path().to_string_lossy().to_string()),
                network_enabled: false,
                approval_mode: RuntimeApprovalMode::Deny,
            })
            .await
            .unwrap();

        let paused = wait_for_job_status(&service, &job.id, JobStatus::Paused).await;
        assert!(paused
            .error
            .as_deref()
            .unwrap_or("")
            .contains("尚未配置 runtime action"));
    }

    #[tokio::test]
    async fn service_suspends_and_resumes_tool_approval() {
        let dir = tempfile::tempdir().unwrap();
        crate::workspace::workspace_init(crate::workspace::WorkspaceInitReq {
            path: dir.path().to_string_lossy().to_string(),
            project_name: "approval-test".to_string(),
            template_type: None,
        })
        .await
        .unwrap();

        let service = RuntimeService::new();
        let runner = service.clone();
        let workspace_path = dir.path().to_string_lossy().to_string();
        let task = tokio::spawn(async move {
            runner
                .execute_tool(RuntimeToolCallArgs {
                    workspace_path,
                    tool_name: "shell_command".to_string(),
                    arguments: json!({ "command": "echo approved" }),
                    network_enabled: false,
                    approval_mode: RuntimeApprovalMode::Wait,
                    model_tier: "local".to_string(),
                })
                .await
        });

        let approval = wait_for_pending_approval(&service).await;
        assert_eq!(approval.tool_name, "shell_command");
        assert!(approval.reason.contains("确认") || approval.reason.contains("执行"));
        assert!(service
            .resolve_approval(&approval.id, ApprovalDecision::Allow, None, None)
            .await
            .unwrap());

        let result = tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(result.outcome.status, ToolStatus::Ok);
        assert!(result.outcome.content.contains("approved"));
        assert!(service.list_pending_approvals().await.is_empty());
        assert!(service.recent_events().await.iter().any(|event| {
            matches!(event, RuntimeEvent::ApprovalRequested { request, .. } if request.id == approval.id)
        }));
    }

    #[tokio::test]
    async fn service_can_deny_with_reason() {
        let dir = tempfile::tempdir().unwrap();
        crate::workspace::workspace_init(crate::workspace::WorkspaceInitReq {
            path: dir.path().to_string_lossy().to_string(),
            project_name: "deny-test".to_string(),
            template_type: None,
        })
        .await
        .unwrap();

        let service = RuntimeService::new();
        let runner = service.clone();
        let workspace_path = dir.path().to_string_lossy().to_string();
        let task = tokio::spawn(async move {
            runner
                .execute_tool(RuntimeToolCallArgs {
                    workspace_path,
                    tool_name: "shell_command".to_string(),
                    arguments: json!({ "command": "echo denied" }),
                    network_enabled: false,
                    approval_mode: RuntimeApprovalMode::Wait,
                    model_tier: "local".to_string(),
                })
                .await
        });

        let approval = wait_for_pending_approval(&service).await;
        assert!(service
            .resolve_approval(
                &approval.id,
                ApprovalDecision::Deny,
                None,
                Some("这次命令不需要执行".to_string()),
            )
            .await
            .unwrap());

        let result = tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(result.outcome.status, ToolStatus::Blocked);
        assert!(result.outcome.content.contains("这次命令不需要执行"));
    }

    #[tokio::test]
    async fn service_reuses_session_approval_for_same_tool_arguments() {
        let dir = tempfile::tempdir().unwrap();
        crate::workspace::workspace_init(crate::workspace::WorkspaceInitReq {
            path: dir.path().to_string_lossy().to_string(),
            project_name: "session-approval-test".to_string(),
            template_type: None,
        })
        .await
        .unwrap();

        let service = RuntimeService::new();
        let runner = service.clone();
        let workspace_path = dir.path().to_string_lossy().to_string();
        let task = tokio::spawn(async move {
            runner
                .execute_tool(RuntimeToolCallArgs {
                    workspace_path,
                    tool_name: "shell_command".to_string(),
                    arguments: json!({ "command": "echo cached" }),
                    network_enabled: false,
                    approval_mode: RuntimeApprovalMode::Wait,
                    model_tier: "local".to_string(),
                })
                .await
        });

        let approval = wait_for_pending_approval(&service).await;
        assert_eq!(approval.command_preview.as_deref(), Some("echo cached"));
        assert!(service
            .resolve_approval(
                &approval.id,
                ApprovalDecision::Allow,
                Some(ApprovalScope::Session),
                None,
            )
            .await
            .unwrap());

        let first = tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(first.outcome.status, ToolStatus::Ok);

        let second = service
            .execute_tool(RuntimeToolCallArgs {
                workspace_path: dir.path().to_string_lossy().to_string(),
                tool_name: "shell_command".to_string(),
                arguments: json!({ "command": "echo cached" }),
                network_enabled: false,
                approval_mode: RuntimeApprovalMode::Wait,
                model_tier: "local".to_string(),
            })
            .await
            .unwrap();
        assert_eq!(second.outcome.status, ToolStatus::Ok);
        assert!(second.outcome.content.contains("cached"));
        assert!(service.list_pending_approvals().await.is_empty());
    }

    #[tokio::test]
    async fn service_requires_approval_before_applying_patch() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("note.txt"), "alpha\nbeta\n").unwrap();
        crate::workspace::workspace_init(crate::workspace::WorkspaceInitReq {
            path: dir.path().to_string_lossy().to_string(),
            project_name: "patch-approval-test".to_string(),
            template_type: None,
        })
        .await
        .unwrap();

        let service = RuntimeService::new();
        let runner = service.clone();
        let workspace_path = dir.path().to_string_lossy().to_string();
        let patch =
            "*** Begin Patch\n*** Update File: note.txt\n@@\n alpha\n-beta\n+BETA\n*** End Patch\n";
        let task = tokio::spawn(async move {
            runner
                .execute_tool(RuntimeToolCallArgs {
                    workspace_path,
                    tool_name: "apply_patch".to_string(),
                    arguments: json!({ "patch": patch }),
                    network_enabled: false,
                    approval_mode: RuntimeApprovalMode::Wait,
                    model_tier: "local".to_string(),
                })
                .await
        });

        let approval = wait_for_pending_approval(&service).await;
        assert_eq!(approval.tool_name, "apply_patch");
        assert!(approval
            .diff_preview
            .as_deref()
            .unwrap_or("")
            .contains("BETA"));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("note.txt")).unwrap(),
            "alpha\nbeta\n"
        );
        assert!(service
            .resolve_approval(&approval.id, ApprovalDecision::Allow, None, None)
            .await
            .unwrap());

        let result = tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(result.outcome.status, ToolStatus::Ok);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("note.txt")).unwrap(),
            "alpha\nBETA\n"
        );
    }

    async fn wait_for_pending_approval(service: &RuntimeService) -> ApprovalRequest {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(request) = service.list_pending_approvals().await.into_iter().next() {
                return request;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "approval request was not created"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn wait_for_job_status(service: &RuntimeService, job_id: &str, status: JobStatus) -> Job {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            let job = service.get_job(job_id).await.unwrap();
            if job.status == status {
                return job;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "job did not reach expected status"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

#[cfg(test)]
mod tool_profile_tests {
    use super::*;

    fn tool(name: &str) -> Tool {
        Tool {
            name: name.to_string(),
            description: String::new(),
            parameters: serde_json::json!({}),
        }
    }

    /// 回归：内部工具对**所有**档位都不声明 —— 包括 Coding。
    ///
    /// `worktree_create` 属于这一档：沙箱创建归调度器托管。让模型自己去建沙箱，
    /// 就退回到了「靠提示词让它记得开」的老路。
    /// 注意是**保留注册、只收声明** —— 所以这里同时断言它仍在注册表里。
    #[test]
    fn internal_tools_are_never_declared_to_the_model() {
        let all: Vec<Tool> = super::super::local_tools::HIDDEN_FROM_MODEL_TOOLS
            .iter()
            .map(|n| tool(n))
            .chain([tool("worktree_merge"), tool("fs_read_text")])
            .collect();
        for profile in [ToolProfile::Coding, ToolProfile::Companion] {
            let names: Vec<String> = profile
                .filter(all.clone())
                .into_iter()
                .map(|t| t.name)
                .collect();
            for internal in super::super::local_tools::HIDDEN_FROM_MODEL_TOOLS {
                assert!(
                    !names.iter().any(|n| n == internal),
                    "{profile:?} 档位仍然声明了内部工具 `{internal}`"
                );
            }
            // 正名：采纳/否决提案的接口必须留给**编码会话**的主 Agent。
            // 少了这条，"只藏 worktree_create"就可能悄悄变成"三个全藏"，
            // 而那意味着有真实改动的沙箱永远无法被接受 —— 且不报任何错。
            // （Companion 档另说：`worktree_merge` 在 CODING_RUNTIME_TOOLS 里，
            //   语音会话本来就该滤掉它。）
            if profile == ToolProfile::Coding {
                assert!(
                    names.iter().any(|n| n == "worktree_merge"),
                    "Coding 档位把 worktree_merge 也滤掉了 —— 沙箱成果将无法采纳"
                );
            }
        }

        // 保留注册：万一有别的路径调到它，走的仍是同一套策略与审批
        let mut registry = super::super::ToolRegistry::new();
        super::super::local_tools::register_local_tools(&mut registry);
        for internal in super::super::local_tools::HIDDEN_FROM_MODEL_TOOLS {
            assert!(
                registry.contains(internal),
                "`{internal}` 该保留注册，只是不声明"
            );
        }
    }

    /// 回归：Companion 档位必须滤掉重型编码工具，且**只**滤掉它们。
    ///
    /// 这条盯的是「过滤发生在声明期」这个前提。注册表在应用启动时只建一次，
    /// 而 schema 是按请求付 token 的 —— 如果哪天有人把过滤挪回注册期，
    /// 语音会话会重新背上全部 schema，而且不会有任何报错。
    #[test]
    fn companion_profile_drops_only_the_coding_runtime_tools() {
        let all: Vec<Tool> = super::super::local_tools::CODING_RUNTIME_TOOLS
            .iter()
            .map(|n| tool(n))
            .chain([tool("fs_read_text"), tool("apply_patch"), tool("search_web")])
            .collect();

        let coding = ToolProfile::Coding.filter(all.clone());
        let coding_names: Vec<&str> = coding.iter().map(|t| t.name.as_str()).collect();
        assert!(
            !coding_names.contains(&"worktree_create"),
            "Coding 档位对模型声明时应隐藏内部 worktree_* 工具"
        );

        let companion = ToolProfile::Companion.filter(all);
        let names: Vec<&str> = companion.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["fs_read_text", "apply_patch", "search_web"],
            "Companion 档位只该滤掉 CODING_RUNTIME_TOOLS 里的工具"
        );
    }
}
