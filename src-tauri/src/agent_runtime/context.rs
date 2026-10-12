//! Runtime context for a single agent turn or durable job step.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{EventLog, RuntimeEvent, RuntimePolicy};

/// 模型能力分级。影响工具默认行为（如 view_file 默认读取行数）。
///
/// - **Local**：本地模型（LM Studio / Ollama / 7B~120B），上下文窗口有限，
///   默认读取 300 行 / 12KB，防止显存与注意力计算超载。
/// - **Cloud**：云端模型（Claude / GPT-4o / DeepSeek），高吞吐大窗口，
///   默认放宽至 800 行 / 45KB。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModelTier {
    /// 本地模型，保守策略
    #[default]
    Local,
    /// 云端模型，宽松策略
    Cloud,
}

impl ModelTier {
    /// 未指定行号时默认读取的最大行数
    pub fn default_line_limit(self) -> usize {
        match self {
            Self::Local => 300,
            Self::Cloud => 800,
        }
    }

    /// 未指定行号时默认的字节截断上限
    pub fn default_char_limit(self) -> usize {
        match self {
            Self::Local => 12_000,
            Self::Cloud => 45_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeIds {
    pub session_id: String,
    pub turn_id: String,
    pub job_id: Option<String>,
}

impl RuntimeIds {
    pub fn new_session() -> Self {
        Self {
            session_id: Uuid::new_v4().to_string(),
            turn_id: Uuid::new_v4().to_string(),
            job_id: None,
        }
    }

    pub fn next_turn(&self) -> Self {
        Self {
            session_id: self.session_id.clone(),
            turn_id: Uuid::new_v4().to_string(),
            job_id: self.job_id.clone(),
        }
    }

}

#[derive(Clone)]
pub struct RuntimeContext {
    pub ids: RuntimeIds,
    pub workspace_root: PathBuf,
    pub policy: RuntimePolicy,
    pub events: Arc<Mutex<EventLog>>,
    pub cancellation: CancellationToken,
    pub model_tier: ModelTier,
    approved_tool_calls: Arc<Mutex<HashSet<String>>>,
    /// 本轮已读过的文件路径集合。`fs_write_text` 覆写已存在文件时要求先读过。
    read_files: Arc<std::sync::Mutex<HashSet<std::path::PathBuf>>>,
}

impl RuntimeContext {
    pub fn new(workspace_root: impl Into<PathBuf>, policy: RuntimePolicy) -> Self {
        let root = workspace_root.into();
        Self {
            ids: RuntimeIds::new_session(),
            workspace_root: root,
            policy,
            events: Arc::new(Mutex::new(EventLog::default())),
            cancellation: CancellationToken::new(),
            model_tier: ModelTier::default(),
            approved_tool_calls: Arc::new(Mutex::new(HashSet::new())),
            read_files: Arc::new(std::sync::Mutex::new(HashSet::new())),
        }
    }

    /// `read_files` 由调用方传入，**不能在这里新建**。
    ///
    /// 生产路径上每一次工具调用都会新建一个 `RuntimeContext`
    /// （`service.rs` 的 `execute_tool_with_registry`），如果读集合跟着 ctx 走，
    /// `view_file` 标记在 ctx#1、`fs_write_text` 检查 ctx#2，集合永远是空的 ——
    /// 表现为**模型永远无法覆写任何已存在的文件**，而且是全部错杀不是漏放行。
    /// 所以它必须挂在跨调用存活的东西上（`RuntimeService` 是 per-session 的，正合适）。
    ///
    /// `service.rs` 里有走生产路径的回归测试盯着这条。
    pub fn with_shared_events(
        workspace_root: impl Into<PathBuf>,
        policy: RuntimePolicy,
        ids: RuntimeIds,
        events: Arc<Mutex<EventLog>>,
        read_files: Arc<std::sync::Mutex<HashSet<std::path::PathBuf>>>,
    ) -> Self {
        Self {
            ids,
            workspace_root: workspace_root.into(),
            policy,
            events,
            cancellation: CancellationToken::new(),
            model_tier: ModelTier::default(),
            approved_tool_calls: Arc::new(Mutex::new(HashSet::new())),
            read_files,
        }
    }

    /// 设置模型能力分级
    pub fn with_model_tier(mut self, tier: ModelTier) -> Self {
        self.model_tier = tier;
        self
    }

    pub fn for_next_turn(&self) -> Self {
        Self {
            ids: self.ids.next_turn(),
            workspace_root: self.workspace_root.clone(),
            policy: self.policy.clone(),
            events: Arc::clone(&self.events),
            cancellation: CancellationToken::new(),
            model_tier: self.model_tier,
            approved_tool_calls: Arc::clone(&self.approved_tool_calls),
            read_files: Arc::clone(&self.read_files),
        }
    }

    pub async fn emit(&self, event: RuntimeEvent) {
        self.events.lock().await.push(event);
    }

    pub async fn recent_events(&self) -> Vec<RuntimeEvent> {
        self.events.lock().await.recent()
    }

    pub async fn grant_tool_call(&self, call_id: impl Into<String>) {
        self.approved_tool_calls.lock().await.insert(call_id.into());
    }

    pub async fn is_tool_call_approved(&self, call_id: &str) -> bool {
        self.approved_tool_calls.lock().await.contains(call_id)
    }

    /// 标记一个文件已被读取。
    pub fn mark_file_read(&self, path: &std::path::Path) {
        self.read_files.lock().unwrap().insert(path.to_path_buf());
    }

    /// 检查文件是否已在本轮被读取过。
    pub fn has_read_file(&self, path: &std::path::Path) -> bool {
        self.read_files.lock().unwrap().contains(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn contexts_share_event_log_across_turns() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = RuntimeContext::new(dir.path(), RuntimePolicy::workspace_write(dir.path()));
        let next = ctx.for_next_turn();

        next.emit(RuntimeEvent::Log {
            target: "test".into(),
            message: "hello".into(),
        })
        .await;

        assert_eq!(ctx.recent_events().await.len(), 1);
        assert_ne!(ctx.ids.turn_id, next.ids.turn_id);
        assert_eq!(ctx.ids.session_id, next.ids.session_id);
    }
}
