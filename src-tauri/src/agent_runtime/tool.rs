//! Tool execution contracts and retry semantics.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::mcp::{Operation, Risk};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    BuiltIn,
    Mcp,
    Shell,
    FileSystem,
    WebSearch,
    ComfyUi,
    Cinema,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    #[serde(default = "default_parameters_schema")]
    pub parameters: Value,
    pub kind: ToolKind,
    pub operation: Operation,
    pub risk: Risk,
    /// Safe to retry without duplicating side effects.
    pub idempotent: bool,
    pub timeout_ms: u64,
}

impl ToolSpec {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        kind: ToolKind,
        operation: Operation,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters: default_parameters_schema(),
            kind,
            operation,
            risk: risk_for_operation(operation),
            idempotent: operation == Operation::Read,
            timeout_ms: 60_000,
        }
    }

    pub fn requires_network(&self) -> bool {
        matches!(self.kind, ToolKind::WebSearch)
    }

    pub fn with_idempotent(mut self, idempotent: bool) -> Self {
        self.idempotent = idempotent;
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout_ms = timeout.as_millis() as u64;
        self
    }

    pub fn with_parameters(mut self, parameters: Value) -> Self {
        self.parameters = parameters;
        self
    }
}

fn default_parameters_schema() -> Value {
    json!({
        "type": "object",
        "properties": {}
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRequest {
    pub id: String,
    pub tool_name: String,
    pub arguments: Value,
    /// Stable key for deduping repeated UI submits or resumed jobs.
    pub idempotency_key: String,
    pub attempt: u32,
}

impl ToolCallRequest {
    pub fn new(tool_name: impl Into<String>, arguments: Value) -> Self {
        let id = Uuid::new_v4().to_string();
        Self {
            id: id.clone(),
            tool_name: tool_name.into(),
            arguments,
            idempotency_key: id,
            attempt: 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    Ok,
    Failed,
    Blocked,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolFailureKind {
    InvalidParams,
    MissingDependency,
    SandboxDenied,
    ApprovalDenied,
    Timeout,
    Transport,
    Upstream,
    Execution,
    /// 前置条件不满足（如覆写文件前未先读取）
    Precondition,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFailure {
    pub kind: ToolFailureKind,
    pub message: String,
    /// A short, user-facing remediation hint.
    pub hint: Option<String>,
}

impl ToolFailure {
    pub fn new(kind: ToolFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            hint: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutcome {
    pub call_id: String,
    pub tool_name: String,
    pub status: ToolStatus,
    pub content: String,
    pub artifacts: Vec<String>,
    pub failure: Option<ToolFailure>,
    pub elapsed_ms: u64,
}

impl ToolOutcome {
    pub fn ok(
        request: &ToolCallRequest,
        content: impl Into<String>,
        artifacts: Vec<String>,
        elapsed: Duration,
    ) -> Self {
        Self {
            call_id: request.id.clone(),
            tool_name: request.tool_name.clone(),
            status: ToolStatus::Ok,
            content: content.into(),
            artifacts,
            failure: None,
            elapsed_ms: elapsed.as_millis() as u64,
        }
    }

    pub fn failed(request: &ToolCallRequest, failure: ToolFailure, elapsed: Duration) -> Self {
        let status = match failure.kind {
            ToolFailureKind::SandboxDenied | ToolFailureKind::ApprovalDenied => ToolStatus::Blocked,
            ToolFailureKind::Cancelled => ToolStatus::Cancelled,
            _ => ToolStatus::Failed,
        };
        Self {
            call_id: request.id.clone(),
            tool_name: request.tool_name.clone(),
            status,
            content: failure.message.clone(),
            artifacts: Vec::new(),
            failure: Some(failure),
            elapsed_ms: elapsed.as_millis() as u64,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub base_delay_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 2,
            base_delay_ms: 800,
        }
    }
}

impl RetryPolicy {
    pub fn should_retry(&self, spec: &ToolSpec, outcome: &ToolOutcome, attempt: u32) -> bool {
        if attempt >= self.max_attempts || !spec.idempotent {
            return false;
        }

        let Some(failure) = &outcome.failure else {
            return false;
        };

        matches!(
            failure.kind,
            ToolFailureKind::Timeout | ToolFailureKind::Transport | ToolFailureKind::Upstream
        )
    }

    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let multiplier = 2u64.saturating_pow(attempt.saturating_sub(1));
        Duration::from_millis(self.base_delay_ms.saturating_mul(multiplier))
    }
}

fn risk_for_operation(operation: Operation) -> Risk {
    match operation {
        Operation::Read => Risk::Low,
        Operation::Write => Risk::Medium,
        Operation::Delete | Operation::Execute => Risk::High,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use super::*;

    #[test]
    fn idempotent_transient_failures_are_retryable() {
        let spec = ToolSpec::new("comfyui_status", "", ToolKind::ComfyUi, Operation::Read);
        let request = ToolCallRequest::new("comfyui_status", json!({}));
        let outcome = ToolOutcome::failed(
            &request,
            ToolFailure::new(ToolFailureKind::Transport, "ComfyUI connection reset"),
            Duration::from_millis(10),
        );

        assert!(RetryPolicy::default().should_retry(&spec, &outcome, 1));
    }

    #[test]
    fn mutating_tools_are_not_retried_by_default() {
        let mut spec = ToolSpec::new("generate_video", "", ToolKind::ComfyUi, Operation::Write);
        spec.idempotent = false;
        let request = ToolCallRequest::new("generate_video", json!({ "shot": 1 }));
        let outcome = ToolOutcome::failed(
            &request,
            ToolFailure::new(ToolFailureKind::Timeout, "timeout"),
            Duration::from_secs(60),
        );

        assert!(!RetryPolicy::default().should_retry(&spec, &outcome, 1));
    }

    #[test]
    fn policy_failures_are_never_retried() {
        let spec = ToolSpec::new("run_command", "", ToolKind::Shell, Operation::Execute);
        let request = ToolCallRequest::new("run_command", json!({ "command": "sudo rm -rf /" }));
        let outcome = ToolOutcome::failed(
            &request,
            ToolFailure::new(ToolFailureKind::SandboxDenied, "blocked"),
            Duration::from_millis(2),
        );

        assert!(!RetryPolicy::default().should_retry(&spec, &outcome, 1));
    }
}
