//! ComfyUI runtime provider.
//!
//! This makes ComfyUI visible to the same runtime router as local tools and MCP.
//! The first layer is intentionally operational: status, start, and running
//! arguments. Generation workflows can move here once durable job steps own
//! provenance and retries.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::json;

use super::{
    RuntimeContext, ToolCallRequest, ToolFailure, ToolFailureKind, ToolHandler, ToolKind,
    ToolOutcome, ToolRegistry, ToolSpec,
};
use crate::mcp::Operation;

#[derive(Clone)]
pub struct ComfyUiStatusTool;

#[derive(Clone)]
pub struct ComfyUiStartTool {
    config_dir: PathBuf,
}

#[derive(Clone)]
pub struct ComfyUiRunningArgsTool;

pub fn register_comfyui_tools(
    registry: &mut ToolRegistry,
    config_dir: impl Into<PathBuf>,
) {
    let dir = config_dir.into();
    registry.register(ComfyUiStatusTool);
    registry.register(ComfyUiStartTool {
        config_dir: dir.clone(),
    });
    registry.register(ComfyUiRunningArgsTool);
    // 原来这里还注册了 `comfyui_generate_song`（ACE-Step 1.5 配乐）。
    // 权重 09-08 删了，入口 2026-10-01 下架 —— 留着只会让模型调一个注定报缺文件的工具。
}

#[async_trait]
impl ToolHandler for ComfyUiStatusTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "comfyui_status",
            "Read local ComfyUI runtime status, VRAM usage, checkpoints, LoRAs, and video models inventory.",
            ToolKind::ComfyUi,
            Operation::Read,
        )
        .with_parameters(json!({"type":"object","properties":{}}))
        .with_timeout(Duration::from_secs(8))
    }

    async fn execute(&self, _ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let started = Instant::now();
        match crate::comfyui::comfyui_status_snapshot().await {
            Ok(status) => {
                let content = serde_json::to_string_pretty(&status)
                    .unwrap_or_else(|_| "Failed to serialize ComfyUI status".to_string());
                ToolOutcome::ok(&request, content, Vec::new(), started.elapsed())
            }
            Err(err) => ToolOutcome::failed(
                &request,
                ToolFailure::new(ToolFailureKind::Transport, err.to_string()),
                started.elapsed(),
            ),
        }
    }
}

#[async_trait]
impl ToolHandler for ComfyUiStartTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "comfyui_start",
            "Start local ComfyUI server in the background and pipe logs to application config directory.",
            ToolKind::ComfyUi,
            Operation::Execute,
        )
        .with_parameters(json!({"type":"object","properties":{}}))
        .with_idempotent(false)
        .with_timeout(Duration::from_secs(10))
    }

    async fn execute(&self, _ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let started = Instant::now();
        match crate::comfyui::comfyui_start_with_config_dir(&self.config_dir).await {
            Ok(true) => ToolOutcome::ok(
                &request,
                "Requested background launch of ComfyUI.".to_string(),
                vec![self
                    .config_dir
                    .join("comfyui.log")
                    .to_string_lossy()
                    .to_string()],
                started.elapsed(),
            ),
            Ok(false) => ToolOutcome::failed(
                &request,
                ToolFailure::new(ToolFailureKind::Execution, "ComfyUI launch command did not succeed"),
                started.elapsed(),
            ),
            Err(err) => ToolOutcome::failed(
                &request,
                ToolFailure::new(ToolFailureKind::Execution, err.to_string())
                .with_hint("Verify that ComfyUI/main.py exists locally and python3/venv is accessible."),
                started.elapsed(),
            ),
        }
    }
}

#[async_trait]
impl ToolHandler for ComfyUiRunningArgsTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "comfyui_running_args",
            "Read currently running ComfyUI CLI arguments to inspect launch flags (fp32-unet, listen, port, etc.).",
            ToolKind::ComfyUi,
            Operation::Read,
        )
        .with_parameters(json!({"type":"object","properties":{}}))
        .with_timeout(Duration::from_secs(5))
    }

    async fn execute(&self, _ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let started = Instant::now();
        match crate::comfyui::running_argv().await {
            Some(argv) => ToolOutcome::ok(
                &request,
                serde_json::to_string_pretty(&argv).unwrap_or_else(|_| "[]".to_string()),
                Vec::new(),
                started.elapsed(),
            ),
            None => ToolOutcome::failed(
                &request,
                ToolFailure::new(
                    ToolFailureKind::Transport,
                    "ComfyUI 未运行或 /system_stats 不可用",
                ),
                started.elapsed(),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comfyui_specs_cover_operational_tools() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = ToolRegistry::new();
        register_comfyui_tools(&mut registry, dir.path());
        let names = registry
            .specs()
            .into_iter()
            .map(|spec| (spec.name, spec.kind, spec.operation))
            .collect::<Vec<_>>();

        assert!(names.contains(&(
            "comfyui_status".to_string(),
            ToolKind::ComfyUi,
            Operation::Read
        )));
        assert!(names.contains(&(
            "comfyui_start".to_string(),
            ToolKind::ComfyUi,
            Operation::Execute
        )));
        assert!(names.contains(&(
            "comfyui_running_args".to_string(),
            ToolKind::ComfyUi,
            Operation::Read
        )));
    }
}
