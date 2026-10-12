//! Tool registry and single dispatch gate.
//!
//! Local commands, MCP, search, ComfyUI, and cinema jobs should share approval,
//! timeout, retry, cancellation, and event logging instead of each building its
//! own loop.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::Value;

use super::{
    RetryPolicy, RuntimeContext, RuntimeEvent, ToolCallRequest, ToolFailure, ToolFailureKind,
    ToolKind, ToolOutcome, ToolSpec, ToolStatus,
};
use crate::mcp::{Operation, Risk};

#[async_trait]
pub trait ToolHandler: Send + Sync {
    fn spec(&self) -> ToolSpec;

    async fn execute(&self, ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome;
}

#[derive(Default, Clone)]
pub struct ToolRegistry {
    handlers: BTreeMap<String, Arc<dyn ToolHandler>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<H>(&mut self, handler: H) -> Option<Arc<dyn ToolHandler>>
    where
        H: ToolHandler + 'static,
    {
        let spec = handler.spec();
        self.handlers.insert(spec.name.clone(), Arc::new(handler))
    }

    pub fn contains(&self, name: &str) -> bool {
        self.handlers.contains_key(name)
    }

    pub fn spec(&self, name: &str) -> Option<ToolSpec> {
        self.handlers.get(name).map(|handler| handler.spec())
    }

    pub fn specs(&self) -> Vec<ToolSpec> {
        self.handlers
            .values()
            .map(|handler| handler.spec())
            .collect()
    }

    fn get(&self, name: &str) -> Option<Arc<dyn ToolHandler>> {
        self.handlers.get(name).cloned()
    }
}

#[derive(Clone)]
pub struct ToolRouter {
    registry: ToolRegistry,
    retry: RetryPolicy,
}

impl ToolRouter {
    pub fn new(registry: ToolRegistry) -> Self {
        Self {
            registry,
            retry: RetryPolicy::default(),
        }
    }

    pub fn with_retry_policy(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    pub fn specs(&self) -> Vec<ToolSpec> {
        self.registry.specs()
    }

    pub async fn dispatch(
        &self,
        ctx: &RuntimeContext,
        mut request: ToolCallRequest,
    ) -> ToolOutcome {
        let Some(handler) = self.registry.get(&request.tool_name) else {
            return unknown_tool(&request);
        };
        let spec = handler.spec();

        if let Some(failure) = preflight(ctx, &spec, &request.arguments) {
            return ToolOutcome::failed(&request, failure, Duration::from_millis(0));
        }

        let mut last = None;
        for attempt in 1..=self.retry.max_attempts {
            request.attempt = attempt;
            ctx.emit(RuntimeEvent::ToolStarted {
                turn_id: ctx.ids.turn_id.clone(),
                call_id: request.id.clone(),
                tool_name: request.tool_name.clone(),
            })
            .await;

            let started = Instant::now();
            let timeout = Duration::from_millis(spec.timeout_ms.max(1));
            let outcome = tokio::select! {
                _ = ctx.cancellation.cancelled() => {
                    ToolOutcome::failed(
                        &request,
                        ToolFailure::new(ToolFailureKind::Cancelled, "工具调用已被用户取消"),
                        started.elapsed(),
                    )
                }
                result = tokio::time::timeout(timeout, handler.execute(ctx, request.clone())) => {
                    match result {
                        Ok(outcome) => outcome,
                        Err(_) => ToolOutcome::failed(
                            &request,
                            ToolFailure::new(
                                ToolFailureKind::Timeout,
                                format!("工具 {} 超时（{}ms）", spec.name, spec.timeout_ms),
                            ),
                            started.elapsed(),
                        ),
                    }
                }
            };

            ctx.emit(RuntimeEvent::ToolFinished {
                turn_id: ctx.ids.turn_id.clone(),
                outcome: outcome.clone(),
            })
            .await;

            if outcome.status == ToolStatus::Ok
                || !self.retry.should_retry(&spec, &outcome, attempt)
            {
                return outcome;
            }

            last = Some(outcome);
            tokio::time::sleep(self.retry.delay_for_attempt(attempt)).await;
        }

        last.unwrap_or_else(|| {
            ToolOutcome::failed(
                &request,
                ToolFailure::new(ToolFailureKind::Execution, "工具执行失败且没有返回结果"),
                Duration::from_millis(0),
            )
        })
    }
}

fn preflight(ctx: &RuntimeContext, spec: &ToolSpec, arguments: &Value) -> Option<ToolFailure> {
    if spec.requires_network() && !ctx.policy.allows_network() {
        return Some(
            ToolFailure::new(
                ToolFailureKind::SandboxDenied,
                "当前 runtime 禁止联网搜索；需要启用 network policy 后才能执行",
            )
            .with_hint("把该任务的网络策略切到 enabled，或改用本地知识库。"),
        );
    }

    if spec.kind == ToolKind::FileSystem {
        let path = arguments
            .get("path")
            .and_then(|v| v.as_str())
            .or_else(|| arguments.get("file").and_then(|v| v.as_str()));
        if let Some(path) = path {
            let capability = if spec.operation == Operation::Read {
                super::FileCapability::Read
            } else {
                super::FileCapability::Write
            };
            let resolved = if Path::new(path).is_absolute() {
                Path::new(path).to_path_buf()
            } else {
                ctx.workspace_root.join(path)
            };
            let verdict = ctx.policy.evaluate_path(resolved, capability);
            if !verdict.allowed() {
                return Some(ToolFailure::new(
                    ToolFailureKind::SandboxDenied,
                    verdict.reason().to_string(),
                ));
            }
        }
    }

    None
}

fn unknown_tool(request: &ToolCallRequest) -> ToolOutcome {
    ToolOutcome::failed(
        request,
        ToolFailure::new(
            ToolFailureKind::InvalidParams,
            format!("未知工具：{}", request.tool_name),
        ),
        Duration::from_millis(0),
    )
}

pub struct StaticToolHandler<F>
where
    F: Send + Sync,
{
    spec: ToolSpec,
    f: F,
}

impl<F> StaticToolHandler<F>
where
    F: Send + Sync,
{
    pub fn new(spec: ToolSpec, f: F) -> Self {
        Self { spec, f }
    }
}

#[async_trait]
impl<F, Fut> ToolHandler for StaticToolHandler<F>
where
    F: Fn(RuntimeContext, ToolCallRequest) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = ToolOutcome> + Send,
{
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    async fn execute(&self, ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        (self.f)(ctx.clone(), request).await
    }
}

pub fn web_search_spec(name: impl Into<String>) -> ToolSpec {
    ToolSpec {
        name: name.into(),
        description: "联网搜索资源并返回可追溯证据".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" }
            },
            "required": ["query"]
        }),
        kind: ToolKind::WebSearch,
        operation: Operation::Read,
        risk: Risk::Low,
        idempotent: true,
        timeout_ms: 20_000,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde_json::json;

    use super::*;
    use crate::agent_runtime::{NetworkPolicy, RuntimePolicy};

    #[tokio::test]
    async fn router_dispatches_registered_tool_and_logs_events() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = RuntimeContext::new(dir.path(), RuntimePolicy::workspace_write(dir.path()));
        let request = ToolCallRequest::new("echo", json!({ "text": "hello" }));

        let mut registry = ToolRegistry::new();
        registry.register(StaticToolHandler::new(
            ToolSpec::new("echo", "echo", ToolKind::BuiltIn, Operation::Read),
            |_, req| async move {
                ToolOutcome::ok(&req, "hello", Vec::new(), Duration::from_millis(1))
            },
        ));

        let outcome = ToolRouter::new(registry).dispatch(&ctx, request).await;
        assert_eq!(outcome.status, ToolStatus::Ok);
        assert_eq!(ctx.recent_events().await.len(), 2);
    }

    #[tokio::test]
    async fn router_retries_idempotent_transient_failures() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = RuntimeContext::new(dir.path(), RuntimePolicy::workspace_write(dir.path()));
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_clone = Arc::clone(&attempts);

        let mut registry = ToolRegistry::new();
        registry.register(StaticToolHandler::new(
            ToolSpec::new("flaky", "flaky", ToolKind::BuiltIn, Operation::Read),
            move |_, req| {
                let attempts = Arc::clone(&attempts_clone);
                async move {
                    let n = attempts.fetch_add(1, Ordering::SeqCst);
                    if n == 0 {
                        ToolOutcome::failed(
                            &req,
                            ToolFailure::new(ToolFailureKind::Transport, "reset"),
                            Duration::from_millis(1),
                        )
                    } else {
                        ToolOutcome::ok(&req, "ok", Vec::new(), Duration::from_millis(1))
                    }
                }
            },
        ));

        let router = ToolRouter::new(registry).with_retry_policy(RetryPolicy {
            max_attempts: 2,
            base_delay_ms: 1,
        });
        let outcome = router
            .dispatch(&ctx, ToolCallRequest::new("flaky", json!({})))
            .await;

        assert_eq!(outcome.status, ToolStatus::Ok);
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn web_search_requires_network_policy() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = RuntimeContext::new(dir.path(), RuntimePolicy::workspace_write(dir.path()));

        let mut registry = ToolRegistry::new();
        registry.register(StaticToolHandler::new(
            web_search_spec("search"),
            |_, req| async move {
                ToolOutcome::ok(&req, "unused", Vec::new(), Duration::from_millis(1))
            },
        ));

        let blocked = ToolRouter::new(registry.clone())
            .dispatch(&ctx, ToolCallRequest::new("search", json!({ "q": "x" })))
            .await;
        assert_eq!(blocked.status, ToolStatus::Blocked);

        let mut policy = RuntimePolicy::workspace_write(dir.path());
        policy.network = NetworkPolicy::Enabled;
        let allowed_ctx = RuntimeContext::new(dir.path(), policy);
        let allowed = ToolRouter::new(registry)
            .dispatch(
                &allowed_ctx,
                ToolCallRequest::new("search", json!({ "q": "x" })),
            )
            .await;
        assert_eq!(allowed.status, ToolStatus::Ok);
    }
}
