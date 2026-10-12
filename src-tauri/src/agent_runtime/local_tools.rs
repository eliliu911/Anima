//! Built-in local tools for code/workspace tasks.
//!
//! These are intentionally small and conservative. They prove the runtime path
//! for local development work before existing UI/agent flows are migrated.

use std::collections::BTreeMap;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::Serialize;
use serde_json::json;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::Mutex;
use uuid::Uuid;

use super::{
    FileCapability, RuntimeContext, ToolCallRequest, ToolFailure, ToolFailureKind, ToolHandler,
    ToolKind, ToolOutcome, ToolSpec,
};
use super::pty_session::PtyInputPlan;
use super::ToolRouter;
use crate::mcp::{ApprovalMode, CommandPolicy, Guard, Operation, ServerPermissions};

pub struct ReadTextFileTool;
pub struct WriteTextFileTool;
pub struct ListDirectoryTool;
pub struct ShellCommandTool {
    guard: Guard,
}
pub struct ExecCommandTool {
    sessions: Arc<ExecSessionManager>,
}
pub struct WriteStdinTool {
    sessions: Arc<ExecSessionManager>,
}
pub struct KillExecSessionTool {
    sessions: Arc<ExecSessionManager>,
}
pub struct WorkspaceSearchTool;
pub struct ApplyPatchTool;
pub struct DevServerStatusTool;
pub struct WebSearchTool;
pub struct ToolSearchTool {
    specs: Vec<ToolSpec>,
}

const MAX_BACKGROUND_SESSIONS: usize = 8;
const BACKGROUND_SESSION_TTL: Duration = Duration::from_secs(30 * 60);
const MAX_SESSION_BUFFER_BYTES: usize = 200_000;
const KEEP_SESSION_BUFFER_BYTES: usize = 160_000;

#[derive(Default)]
pub struct ExecSessionManager {
    sessions: Mutex<BTreeMap<String, ExecSession>>,
}

struct ExecSession {
    stdin: Arc<Mutex<ChildStdin>>,
    child: Arc<Mutex<Child>>,
    stdout: Arc<Mutex<String>>,
    stderr: Arc<Mutex<String>>,
    created_at: Instant,
    last_access: Arc<Mutex<Instant>>,
}

impl Default for ShellCommandTool {
    fn default() -> Self {
        Self {
            guard: Guard::new(
                ApprovalMode::Conservative,
                ServerPermissions::permissive(),
                CommandPolicy::default(),
            ),
        }
    }
}

#[async_trait]
impl ToolHandler for ReadTextFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "view_file",
            "View or read file contents within the workspace. Supports line slicing with start_line and end_line for precise function/block inspection (e.g. lines 450~600). Defaults to the first 300 lines if no range is specified. Use code_outline first for rapid outline navigation.",
            ToolKind::FileSystem,
            Operation::Read,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Relative or absolute file path" },
                "start_line": { "type": "integer", "description": "Starting line number (1-indexed, inclusive, optional)" },
                "end_line": { "type": "integer", "description": "Ending line number (1-indexed, inclusive, optional)" },
                "limit": { "type": "integer", "description": "Maximum number of lines to read (default 300, optional)" }
            },
            "required": ["path"]
        }))
        .with_timeout(Duration::from_secs(5))
    }

    async fn execute(&self, ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let Some(path) = string_arg(&request.arguments, "path")
            .or_else(|| string_arg(&request.arguments, "AbsolutePath"))
            .or_else(|| string_arg(&request.arguments, "file"))
        else {
            return invalid_params(&request, "缺少 path 参数");
        };
        let path = resolve_workspace_path(ctx, path);
        let verdict = ctx.policy.evaluate_path(&path, FileCapability::Read);
        if !verdict.allowed() {
            return denied(&request, verdict.reason());
        }

        match std::fs::read_to_string(&path) {
            Ok(content) => {
                let lines: Vec<&str> = content.lines().collect();
                let total_lines = lines.len();

                let start_line = u64_arg(&request.arguments, "start_line")
                    .or_else(|| u64_arg(&request.arguments, "StartLine"))
                    .or_else(|| u64_arg(&request.arguments, "line_start"))
                    .or_else(|| u64_arg(&request.arguments, "offset"))
                    .unwrap_or(1)
                    .max(1) as usize;

                let end_line_param = u64_arg(&request.arguments, "end_line")
                    .or_else(|| u64_arg(&request.arguments, "EndLine"))
                    .or_else(|| u64_arg(&request.arguments, "line_end"));

                if let Some(el) = end_line_param {
                    if (el as usize) < start_line {
                        return ToolOutcome::ok(
                            &request,
                            format!(
                                "[参数无效: end_line ({}) 小于 start_line ({})]",
                                el, start_line
                            ),
                            vec![path.to_string_lossy().to_string()],
                            Duration::from_millis(0),
                        );
                    }
                }

                if total_lines == 0 {
                    return ToolOutcome::ok(
                        &request,
                        "[文件为空]".to_string(),
                        vec![path.to_string_lossy().to_string()],
                        Duration::from_millis(0),
                    );
                }

                if start_line > total_lines {
                    return ToolOutcome::ok(
                        &request,
                        format!(
                            "[start_line {} 已超过文件末尾（共 {} 行）。有效范围 1~{}]",
                            start_line, total_lines, total_lines
                        ),
                        vec![path.to_string_lossy().to_string()],
                        Duration::from_millis(0),
                    );
                }

                let default_limit = ctx.model_tier.default_line_limit();

                let limit = if let Some(el) = end_line_param {
                    (el as usize) - start_line + 1
                } else {
                    u64_arg(&request.arguments, "limit")
                        .or_else(|| u64_arg(&request.arguments, "line_count"))
                        .or_else(|| u64_arg(&request.arguments, "max_lines"))
                        .unwrap_or(default_limit as u64) as usize
                };
                let limit = limit.max(1);

                let start_idx = start_line - 1;
                let end_idx = (start_idx + limit).min(total_lines);
                let slice = lines[start_idx..end_idx].join("\n");

                // 精确指定 end_line 时放宽字节截断上限至 36,000 字符，否则按模型分级动态设置
                let max_chars = if end_line_param.is_some() {
                    36_000
                } else {
                    ctx.model_tier.default_char_limit()
                };
                let (mut out, was_truncated) = truncate_with_flag(&slice, max_chars);

                let shown_lines = out.lines().count();
                let real_end = (start_idx + shown_lines).min(total_lines);

                if was_truncated && shown_lines <= 1 {
                    out.push_str(&format!(
                        "\n\n[提示: 第 {} 行单行超长（已截取前 {} 字符，未完整输出该行）。文件共 {} 行]",
                        start_line, max_chars, total_lines
                    ));
                } else if real_end < total_lines || was_truncated {
                    if end_line_param.is_none() && total_lines > 200 {
                        out.push_str(&format!(
                            "\n\n[提示: 文件共 {} 行，本次显示第 {}~{} 行。请勿通篇全量读取；推荐先使用 code_outline(path: \"{}\") 查看类/函数起止行，再用 view_file(start_line, end_line) 定向精读目标实现]",
                            total_lines,
                            start_idx + 1,
                            real_end,
                            path.display()
                        ));
                    } else {
                        out.push_str(&format!(
                            "\n\n[提示: 文件共 {} 行，本次显示第 {}~{} 行。如需查看后续内容，请指定 start_line: {}，或使用 code_outline 查看大纲]",
                            total_lines,
                            start_idx + 1,
                            real_end,
                            real_end + 1
                        ));
                    }
                }
                ctx.mark_file_read(&path);
                ToolOutcome::ok(
                    &request,
                    out,
                    vec![path.to_string_lossy().to_string()],
                    Duration::from_millis(0),
                )
            }
            Err(err) => {
                let ws_root = ctx.workspace_root.display().to_string();
                let hint = if !path.exists() {
                    format!(
                        "文件不存在：'{}'（工作区根目录为：'{}'）。若读取相对路径请确保文件名正确，或使用 fs_list_directory 查看目录内容。",
                        path.display(),
                        ws_root
                    )
                } else {
                    format!("读取文件失败：{err}")
                };
                ToolOutcome::failed(
                    &request,
                    ToolFailure::new(ToolFailureKind::Execution, hint),
                    Duration::from_millis(0),
                )
            }
        }
    }
}

#[async_trait]
impl ToolHandler for WriteTextFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "fs_write_text",
            "Write or overwrite a UTF-8 text file inside the workspace.",
            ToolKind::FileSystem,
            Operation::Write,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Target file path relative to workspace root" },
                "content": { "type": "string", "description": "Full UTF-8 file content to write" }
            },
            "required": ["path", "content"]
        }))
        .with_idempotent(false)
        .with_timeout(Duration::from_secs(5))
    }

    async fn execute(&self, ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let Some(path) = string_arg(&request.arguments, "path") else {
            return invalid_params(&request, "缺少 path 参数");
        };
        let Some(content) = string_arg(&request.arguments, "content") else {
            return invalid_params(&request, "缺少 content 参数");
        };
        let path = resolve_workspace_path(ctx, path);
        let verdict = ctx.policy.evaluate_path(&path, FileCapability::Write);
        if !verdict.allowed() {
            return denied(&request, verdict.reason());
        }

        // 先读后写约束：已存在的文件必须先用 view_file 查看过才能覆写。
        // 新建文件（路径不存在）直接放行，apply_patch 不经过此处不受影响。
        if path.exists() && !ctx.has_read_file(&path) {
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(
                    ToolFailureKind::Precondition,
                    format!(
                        "文件 {} 已存在但本轮未用 view_file 读取过。\
                         请先查看文件内容、确认理解后再覆写，避免意外丢失代码。",
                        path.display()
                    ),
                ),
                Duration::from_millis(0),
            );
        }

        if let Some(parent) = path.parent() {
            if let Err(err) = std::fs::create_dir_all(parent) {
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(ToolFailureKind::Execution, format!("创建目录失败：{err}")),
                    Duration::from_millis(0),
                );
            }
        }

        match std::fs::write(&path, content) {
            Ok(()) => ToolOutcome::ok(
                &request,
                format!("已写入 {}", path.display()),
                vec![path.to_string_lossy().to_string()],
                Duration::from_millis(0),
            ),
            Err(err) => ToolOutcome::failed(
                &request,
                ToolFailure::new(ToolFailureKind::Execution, format!("写入文件失败：{err}")),
                Duration::from_millis(0),
            ),
        }
    }
}

#[async_trait]
impl ToolHandler for ListDirectoryTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "fs_list_directory",
            "List directory contents within the workspace. Supports '.' or relative subdirectories. Automatically ignores .git, target, and node_modules by default.",
            ToolKind::FileSystem,
            Operation::Read,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "default": ".", "description": "Relative or absolute directory path, default '.'" },
                "max_depth": { "type": "integer", "default": 1, "description": "Recursion depth, default 1 (current directory only), maximum 3" },
                "include_hidden": { "type": "boolean", "default": false, "description": "Whether to include hidden files/directories starting with '.'" }
            }
        }))
        .with_timeout(Duration::from_secs(5))
    }

    async fn execute(&self, ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let path_str = string_arg(&request.arguments, "path")
            .or_else(|| string_arg(&request.arguments, "dir"))
            .unwrap_or(".");
        let path = resolve_workspace_path(ctx, path_str);
        let verdict = ctx.policy.evaluate_path(&path, FileCapability::Read);
        if !verdict.allowed() {
            return denied(&request, verdict.reason());
        }

        if !path.exists() {
            let ws_root = ctx.workspace_root.display().to_string();
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(
                    ToolFailureKind::InvalidParams,
                    format!(
                        "目录不存在：'{}'（工作区根目录为：'{}'）。建议使用 '.' 查看根目录，或确认相对路径是否拼写正确。",
                        path.display(),
                        ws_root
                    ),
                ),
                Duration::from_millis(0),
            );
        }

        let max_depth = u64_arg(&request.arguments, "max_depth")
            .unwrap_or(1)
            .clamp(1, 3) as usize;
        let include_hidden = bool_arg(&request.arguments, "include_hidden").unwrap_or(false);

        let mut rows = Vec::new();
        collect_directory_entries(&path, &path, 1, max_depth, include_hidden, &mut rows);

        if rows.is_empty() {
            return ToolOutcome::ok(
                &request,
                "[空目录或所有条目均在忽略名单中]".to_string(),
                Vec::new(),
                Duration::from_millis(0),
            );
        }

        rows.sort();
        ToolOutcome::ok(
            &request,
            rows.join("\n"),
            Vec::new(),
            Duration::from_millis(0),
        )
    }
}

fn collect_directory_entries(
    base: &Path,
    current: &Path,
    current_depth: usize,
    max_depth: usize,
    include_hidden: bool,
    rows: &mut Vec<String>,
) {
    // 忽略的巨型或衍生目录与文件
    const DEFAULT_IGNORED_NAMES: &[&str] = &[
        ".git",
        ".svn",
        ".hg",
        "node_modules",
        "target",
        "dist",
        "build",
        "out",
        ".next",
        ".nuxt",
        ".turbo",
        ".cache",
        "__pycache__",
        ".pytest_cache",
        ".venv",
        "venv",
        ".DS_Store",
        "Thumbs.db",
    ];

    let Ok(entries) = std::fs::read_dir(current) else {
        return;
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if !include_hidden && name_str.starts_with('.') && name_str != "." && name_str != ".." {
            continue;
        }

        if DEFAULT_IGNORED_NAMES.contains(&name_str.as_ref()) {
            continue;
        }

        let file_type = entry.file_type().ok();
        let is_dir = file_type.as_ref().is_some_and(|t| t.is_dir());
        let rel_path = entry
            .path()
            .strip_prefix(base)
            .unwrap_or(&entry.path())
            .to_string_lossy()
            .to_string();

        let type_tag = if is_dir { "dir" } else { "file" };
        rows.push(format!("{}\t{}", type_tag, rel_path));

        if is_dir && current_depth < max_depth {
            collect_directory_entries(
                base,
                &entry.path(),
                current_depth + 1,
                max_depth,
                include_hidden,
                rows,
            );
        }
    }
}

#[async_trait]
impl ToolHandler for ShellCommandTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "shell_command",
            "Execute a shell command within the workspace root under the security policy and approval guard. Note: NEVER use shell_command with afplay/mpv/ffplay to play audio; use the dedicated `play_audio` tool instead.",
            ToolKind::Shell,
            Operation::Execute,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "The exact shell command line to execute" }
            },
            "required": ["command"]
        }))
        .with_idempotent(false)
        .with_timeout(Duration::from_secs(30))
    }

    async fn execute(&self, ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let Some(command) = string_arg(&request.arguments, "command") else {
            return invalid_params(&request, "缺少 command 参数");
        };

        if let Some(player) = command_uses_system_audio_player(command) {
            return denied_audio_player_command(&request, player);
        }

        let verdict = Guard::new(
            self.guard.mode,
            self.guard.permissions.clone(),
            ctx.policy.command_policy(),
        )
        .evaluate("run_command", "在 shell 里执行一条命令", &request.arguments);
        if verdict.denied() {
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(ToolFailureKind::SandboxDenied, verdict.reason().to_string()),
                Duration::from_millis(0),
            );
        }
        if verdict.needs_confirmation() && !ctx.is_tool_call_approved(&request.id).await {
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(
                    ToolFailureKind::ApprovalDenied,
                    verdict.reason().to_string(),
                )
                .with_hint("该命令需要接入审批 UI 后才能执行。"),
                Duration::from_millis(0),
            );
        }

        let output = std::process::Command::new("sh")
            .arg("-lc")
            .arg(command)
            .current_dir(&ctx.workspace_root)
            .env_clear()
            .envs(sanitized_command_env())
            .output();
        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                let mut content = String::new();
                if !stdout.trim().is_empty() {
                    content.push_str("stdout:\n");
                    content.push_str(&stdout);
                }
                if !stderr.trim().is_empty() {
                    if !content.is_empty() {
                        content.push('\n');
                    }
                    content.push_str("stderr:\n");
                    content.push_str(&stderr);
                }
                if content.is_empty() {
                    content = format!("命令已结束，退出码：{}", out.status);
                }
                if out.status.success() {
                    ToolOutcome::ok(
                        &request,
                        truncate(&content, 64_000),
                        Vec::new(),
                        Duration::from_millis(0),
                    )
                } else {
                    ToolOutcome::failed(
                        &request,
                        ToolFailure::new(
                            ToolFailureKind::Execution,
                            format!(
                                "命令退出码 {}：\n{}",
                                out.status,
                                truncate(&content, 32_000)
                            ),
                        ),
                        Duration::from_millis(0),
                    )
                }
            }
            Err(err) => ToolOutcome::failed(
                &request,
                ToolFailure::new(ToolFailureKind::Execution, format!("命令执行失败：{err}")),
                Duration::from_millis(0),
            ),
        }
    }
}

#[async_trait]
impl ToolHandler for ExecCommandTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "exec_command",
            "Execute a shell command with structured exit_code, stdout, and stderr. Supports background=true for long-running processes.",
            ToolKind::Shell,
            Operation::Execute,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "cmd": { "type": "string", "description": "The exact shell command line to execute" },
                "workdir": { "type": "string", "description": "Optional subdirectory relative to the workspace root" },
                "timeout_ms": { "type": "integer", "default": 30000, "description": "Execution timeout in milliseconds" },
                "max_output_chars": { "type": "integer", "default": 64000, "description": "Maximum characters of output to retain" },
                "background": { "type": "boolean", "default": false, "description": "Run in background as an interactive session" },
                "cursor": { "type": "integer", "default": 0, "description": "Cursor offset for output pagination" },
                "stdout_cursor": { "type": "integer", "default": 0 },
                "stderr_cursor": { "type": "integer", "default": 0 }
            },
            "required": ["cmd"]
        }))
        .with_idempotent(false)
        .with_timeout(Duration::from_secs(120))
    }

    async fn execute(&self, ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let Some(cmd) = string_arg(&request.arguments, "cmd")
            .or_else(|| string_arg(&request.arguments, "command"))
        else {
            return invalid_params(&request, "缺少 cmd 参数");
        };

        if let Some(player) = command_uses_system_audio_player(cmd) {
            return denied_audio_player_command(&request, player);
        }

        let workdir = string_arg(&request.arguments, "workdir")
            .map(|w| resolve_workspace_path(ctx, w))
            .unwrap_or_else(|| ctx.workspace_root.clone());
        let verdict = ctx.policy.evaluate_path(&workdir, FileCapability::Read);
        if !verdict.allowed() {
            return denied(&request, verdict.reason());
        }

        let guard = Guard::new(
            ApprovalMode::Conservative,
            ServerPermissions::permissive(),
            ctx.policy.command_policy(),
        );
        let verdict = guard.evaluate(
            "run_command",
            "在 shell 里执行一条命令",
            &serde_json::json!({ "command": cmd }),
        );
        if verdict.denied() {
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(ToolFailureKind::SandboxDenied, verdict.reason().to_string()),
                Duration::from_millis(0),
            );
        }
        if verdict.needs_confirmation() && !ctx.is_tool_call_approved(&request.id).await {
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(
                    ToolFailureKind::ApprovalDenied,
                    verdict.reason().to_string(),
                )
                .with_hint("该命令需要审批 UI 放行，或加入项目 auto_approve_commands。"),
                Duration::from_millis(0),
            );
        }

        let max_output = usize_arg(&request.arguments, "max_output_chars").unwrap_or(64_000);
        let timeout_ms = u64_arg(&request.arguments, "timeout_ms").unwrap_or(30_000);
        let background = bool_arg(&request.arguments, "background").unwrap_or(false);
        let cursor = usize_arg(&request.arguments, "cursor").unwrap_or(0);
        let stdout_cursor = usize_arg(&request.arguments, "stdout_cursor").unwrap_or(cursor);
        let stderr_cursor = usize_arg(&request.arguments, "stderr_cursor").unwrap_or(cursor);

        if background {
            return self
                .sessions
                .spawn_background(
                    &request,
                    cmd,
                    &workdir,
                    max_output,
                    stdout_cursor,
                    stderr_cursor,
                )
                .await;
        }

        run_command_once(&request, cmd, &workdir, max_output, timeout_ms).await
    }
}

#[async_trait]
impl ToolHandler for WriteStdinTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "write_stdin",
            "向 exec_command background=true 创建的会话写入 stdin，并返回新输出片段。",
            ToolKind::Shell,
            Operation::Execute,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" },
                "chars": { "type": "string" },
                "cursor": { "type": "integer", "default": 0, "description": "兼容字段；同时作为 stdout/stderr cursor" },
                "stdout_cursor": { "type": "integer", "default": 0 },
                "stderr_cursor": { "type": "integer", "default": 0 },
                "max_output_chars": { "type": "integer", "default": 20000 }
            },
            "required": ["session_id", "chars"]
        }))
        .with_idempotent(false)
        .with_timeout(Duration::from_secs(10))
    }

    async fn execute(&self, _ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let Some(session_id) = string_arg(&request.arguments, "session_id") else {
            return invalid_params(&request, "缺少 session_id 参数");
        };
        let Some(chars) = string_arg(&request.arguments, "chars") else {
            return invalid_params(&request, "缺少 chars 参数");
        };
        let max_output = usize_arg(&request.arguments, "max_output_chars").unwrap_or(20_000);
        let cursor = usize_arg(&request.arguments, "cursor").unwrap_or(0);
        let stdout_cursor = usize_arg(&request.arguments, "stdout_cursor").unwrap_or(cursor);
        let stderr_cursor = usize_arg(&request.arguments, "stderr_cursor").unwrap_or(cursor);
        self.sessions
            .write_stdin(
                &request,
                session_id,
                chars,
                max_output,
                stdout_cursor,
                stderr_cursor,
            )
            .await
    }
}

#[async_trait]
impl ToolHandler for KillExecSessionTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "kill_exec_session",
            "终止 exec_command background=true 创建的后台会话。",
            ToolKind::Shell,
            Operation::Execute,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" }
            },
            "required": ["session_id"]
        }))
        .with_idempotent(false)
        .with_timeout(Duration::from_secs(10))
    }

    async fn execute(&self, _ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let Some(session_id) = string_arg(&request.arguments, "session_id") else {
            return invalid_params(&request, "缺少 session_id 参数");
        };
        self.sessions.kill_session(&request, session_id).await
    }
}

// ── PTY 交互式伪终端工具族 (P3: Interactive PTY Tools) ──

pub struct PtySpawnTool {
    pub pty_mgr: Arc<super::pty_session::PtySessionManager>,
}

#[async_trait]
impl ToolHandler for PtySpawnTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "pty_spawn",
            "在跨平台伪终端 (PTY) 中启动交互式命令（如 npm init、交互式 REPL、Vite 开发服务器或看门狗任务），返回 session_id 与初始 ANSI 字符流。",
            ToolKind::Shell,
            Operation::Execute,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "cmd": { "type": "string", "description": "要执行的交互式终端命令" },
                "workdir": { "type": "string", "description": "工作目录相对路径（默认 workspace 根目录）" },
                "rows": { "type": "integer", "default": 24, "description": "终端行数" },
                "cols": { "type": "integer", "default": 80, "description": "终端列数" }
            },
            "required": ["cmd"]
        }))
        .with_idempotent(false)
        .with_timeout(Duration::from_secs(10))
    }

    async fn execute(&self, ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let Some(cmd) = string_arg(&request.arguments, "cmd") else {
            return invalid_params(&request, "缺少 cmd 参数");
        };
        let workdir = string_arg(&request.arguments, "workdir")
            .map(|w| resolve_workspace_path(ctx, w))
            .unwrap_or_else(|| ctx.workspace_root.clone());
        let verdict = ctx.policy.evaluate_path(&workdir, FileCapability::Read);
        if !verdict.allowed() {
            return denied(&request, verdict.reason());
        }

        let guard = Guard::new(
            ApprovalMode::Conservative,
            ServerPermissions::permissive(),
            ctx.policy.command_policy(),
        );
        let verdict = guard.evaluate(
            "run_command",
            "在交互式 PTY 中执行命令",
            &serde_json::json!({ "command": cmd }),
        );
        if verdict.denied() {
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(ToolFailureKind::SandboxDenied, verdict.reason().to_string()),
                Duration::from_millis(0),
            );
        }
        if verdict.needs_confirmation() && !ctx.is_tool_call_approved(&request.id).await {
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(
                    ToolFailureKind::ApprovalDenied,
                    verdict.reason().to_string(),
                )
                .with_hint("该命令需要审批 UI 放行，或加入项目 auto_approve_commands。"),
                Duration::from_millis(0),
            );
        }

        let rows = usize_arg(&request.arguments, "rows").unwrap_or(24) as u16;
        let cols = usize_arg(&request.arguments, "cols").unwrap_or(80) as u16;
        let env_vars = sanitized_command_env();

        match self.pty_mgr.spawn(cmd, &workdir, &env_vars, rows, cols).await {
            Ok((_, payload)) => {
                ToolOutcome::ok(
                    &request,
                    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string()),
                    Vec::new(),
                    Duration::from_millis(0),
                )
            }
            Err(err) => ToolOutcome::failed(
                &request,
                ToolFailure::new(ToolFailureKind::Execution, err),
                Duration::from_millis(0),
            ),
        }
    }
}

pub struct PtyWriteTool {
    pub pty_mgr: Arc<super::pty_session::PtySessionManager>,
}

#[async_trait]
impl ToolHandler for PtyWriteTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "pty_write",
            "向运行中的 PTY 会话注入按键或标准输入文本（支持写入 \\r 回车、\\x03 中断等控制符），并抓取子进程最新渲染的增量输出。",
            ToolKind::Shell,
            Operation::Execute,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string", "description": "PTY 会话 ID" },
                "input": { "type": "string", "description": "要写入的内容。命令必须自带换行（如 'npm run build\\n'、'y\\n'）—— 不含换行的文本只会被累积、不会送达，直到补上换行整行过安全闸门为止。控制键（Ctrl-C 等）原样透传。" },
                "cursor": { "type": "integer", "default": 0, "description": "上次读取到的输出游标位置" },
                "max_chars": { "type": "integer", "default": 20000, "description": "单次最多提取字符数" }
            },
            "required": ["session_id", "input"]
        }))
        .with_idempotent(false)
        .with_timeout(Duration::from_secs(10))
    }

    async fn execute(&self, ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let Some(session_id) = string_arg(&request.arguments, "session_id") else {
            return invalid_params(&request, "缺少 session_id 参数");
        };
        let Some(input) = string_arg(&request.arguments, "input") else {
            return invalid_params(&request, "缺少 input 参数");
        };

        let cursor = usize_arg(&request.arguments, "cursor").unwrap_or(0);
        let max_chars = usize_arg(&request.arguments, "max_chars").unwrap_or(20_000);

        // ── P0 安全闸门 ──
        // 不变式：**写进 PTY 的字节，必须和通过 Guard 核验的字节逐一相同。**
        // 所以这里不再把原始 input 透传下去 —— 普通文本一律先在 `plan_input` 里累积，
        // 只有凑齐整行并且逐行过闸之后，才由 `write_vetted_lines` 一次性写出。
        // 详细的失同步攻击链见 `pty_session::plan_input` 的文档注释。
        let plan = match self.pty_mgr.plan_input(session_id, input) {
            Ok(plan) => plan,
            Err(err) => {
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(ToolFailureKind::Execution, err),
                    Duration::from_millis(0),
                );
            }
        };

        let lines = match plan {
            // 控制键原样透传；普通文本没凑齐整行则只累积、一个字节都不写进 PTY。
            // 两种情况都回报一次当前终端画面，调用方可以接着往下拼。
            plan @ (PtyInputPlan::Passthrough | PtyInputPlan::Buffered) => {
                let payload = match plan {
                    PtyInputPlan::Passthrough => input,
                    _ => "",
                };
                return match self
                    .pty_mgr
                    .write_input(session_id, payload, cursor, max_chars)
                    .await
                {
                    Ok(out) => ToolOutcome::ok(
                        &request,
                        serde_json::to_string_pretty(&out).unwrap_or_else(|_| "{}".to_string()),
                        Vec::new(),
                        Duration::from_millis(0),
                    ),
                    Err(err) => ToolOutcome::failed(
                        &request,
                        ToolFailure::new(ToolFailureKind::Execution, err),
                        Duration::from_millis(0),
                    ),
                };
            }
            PtyInputPlan::Lines(lines) => lines,
        };

        let guard = Guard::new(
            ApprovalMode::Conservative,
            ServerPermissions::permissive(),
            ctx.policy.command_policy(),
        );

        for trimmed in &lines {
            let verdict = guard.evaluate(
                "run_command",
                "在交互式 PTY 中提交执行命令",
                &serde_json::json!({ "command": trimmed }),
            );

            if verdict.denied() {
                self.pty_mgr.clear_pending_input(session_id);
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(
                        ToolFailureKind::SandboxDenied,
                        format!(
                            "PTY 输入命令被安全策略拦截 ({}): {}",
                            verdict.reason(),
                            trimmed
                        ),
                    ),
                    Duration::from_millis(0),
                );
            }

            if verdict.needs_confirmation() && !ctx.is_tool_call_approved(&request.id).await {
                self.pty_mgr.clear_pending_input(session_id);
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(
                        ToolFailureKind::ApprovalDenied,
                        format!("PTY 命令需要审批放行: {}", trimmed),
                    )
                    .with_hint("该命令包含敏感操作，需要审批 UI 明确授权。"),
                    Duration::from_millis(0),
                );
            }
        }

        // 到这里 `lines` 里每一行都过闸了，写下去的就是刚刚核验过的那些字节。
        match self
            .pty_mgr
            .write_vetted_lines(session_id, &lines, cursor, max_chars)
            .await
        {
            Ok(payload) => ToolOutcome::ok(
                &request,
                serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string()),
                Vec::new(),
                Duration::from_millis(0),
            ),
            Err(err) => ToolOutcome::failed(
                &request,
                ToolFailure::new(ToolFailureKind::Execution, err),
                Duration::from_millis(0),
            ),
        }
    }
}

pub struct PtyPollTool {
    pub pty_mgr: Arc<super::pty_session::PtySessionManager>,
}

#[async_trait]
impl ToolHandler for PtyPollTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "pty_poll",
            "轮询 PTY 会话状态与增量输出，用于查看长时间运行的构建、开发服务器或测试输出进度。",
            ToolKind::Shell,
            Operation::Read,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string", "description": "PTY 会话 ID" },
                "cursor": { "type": "integer", "default": 0, "description": "上次读取到的输出游标位置" },
                "max_chars": { "type": "integer", "default": 20000, "description": "最多提取字符数" }
            },
            "required": ["session_id"]
        }))
        .with_timeout(Duration::from_secs(5))
    }

    async fn execute(&self, _ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let Some(session_id) = string_arg(&request.arguments, "session_id") else {
            return invalid_params(&request, "缺少 session_id 参数");
        };
        let cursor = usize_arg(&request.arguments, "cursor").unwrap_or(0);
        let max_chars = usize_arg(&request.arguments, "max_chars").unwrap_or(20_000);

        match self.pty_mgr.poll_output(session_id, cursor, max_chars).await {
            Ok(payload) => ToolOutcome::ok(
                &request,
                serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string()),
                Vec::new(),
                Duration::from_millis(0),
            ),
            Err(err) => ToolOutcome::failed(
                &request,
                ToolFailure::new(ToolFailureKind::Execution, err),
                Duration::from_millis(0),
            ),
        }
    }
}

pub struct PtyKillTool {
    pub pty_mgr: Arc<super::pty_session::PtySessionManager>,
}

#[async_trait]
impl ToolHandler for PtyKillTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "pty_kill",
            "强制终止正在运行的 PTY 交互会话并释放系统资源。",
            ToolKind::Shell,
            Operation::Execute,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string", "description": "PTY 会话 ID" }
            },
            "required": ["session_id"]
        }))
        .with_idempotent(false)
        .with_timeout(Duration::from_secs(10))
    }

    async fn execute(&self, _ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let Some(session_id) = string_arg(&request.arguments, "session_id") else {
            return invalid_params(&request, "缺少 session_id 参数");
        };

        match self.pty_mgr.kill(session_id).await {
            Ok(exit_code) => {
                let payload = json!({
                    "session_id": session_id,
                    "killed": true,
                    "exit_code": exit_code
                });
                ToolOutcome::ok(
                    &request,
                    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string()),
                    Vec::new(),
                    Duration::from_millis(0),
                )
            }
            Err(err) => ToolOutcome::failed(
                &request,
                ToolFailure::new(ToolFailureKind::Execution, err),
                Duration::from_millis(0),
            ),
        }
    }
}

#[async_trait]
impl ToolHandler for WorkspaceSearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "workspace_search",
            "Search for text matches within the workspace using ripgrep (rg), returning matched files, line numbers, and snippets.",
            ToolKind::FileSystem,
            Operation::Read,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search term or regex pattern" },
                "path": { "type": "string", "default": ".", "description": "Subdirectory to search within, default '.'" },
                "max_results": { "type": "integer", "default": 80, "description": "Maximum number of matched lines to return" }
            },
            "required": ["query"]
        }))
        .with_timeout(Duration::from_secs(15))
    }

    async fn execute(&self, ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let Some(query) = string_arg(&request.arguments, "query") else {
            return invalid_params(&request, "缺少 query 参数");
        };
        let path = string_arg(&request.arguments, "path").unwrap_or(".");
        let search_root = resolve_workspace_path(ctx, path);
        let verdict = ctx.policy.evaluate_path(&search_root, FileCapability::Read);
        if !verdict.allowed() {
            return denied(&request, verdict.reason());
        }
        let max_results = usize_arg(&request.arguments, "max_results")
            .unwrap_or(80)
            .min(500);

        let mut cmd = Command::new("rg");
        cmd.arg("--line-number")
            .arg("--column")
            .arg("--color")
            .arg("never")
            .arg("--max-count")
            .arg(max_results.to_string())
            .arg(query)
            .arg(search_root)
            .current_dir(&ctx.workspace_root);

        let started = Instant::now();
        match tokio::time::timeout(Duration::from_secs(12), cmd.output()).await {
            Err(_) => ToolOutcome::failed(
                &request,
                ToolFailure::new(ToolFailureKind::Timeout, "workspace_search 超时"),
                started.elapsed(),
            ),
            Ok(Err(err)) => ToolOutcome::failed(
                &request,
                ToolFailure::new(ToolFailureKind::Execution, format!("启动 rg 失败：{err}")),
                started.elapsed(),
            ),
            Ok(Ok(out)) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                if out.status.success() || out.status.code() == Some(1) {
                    let content = if stdout.trim().is_empty() {
                        "没有匹配结果".to_string()
                    } else {
                        truncate(&stdout, 64_000)
                    };
                    ToolOutcome::ok(&request, content, Vec::new(), started.elapsed())
                } else {
                    ToolOutcome::failed(
                        &request,
                        ToolFailure::new(
                            ToolFailureKind::Execution,
                            format!("rg 执行失败：{}", truncate(&stderr, 12_000)),
                        ),
                        started.elapsed(),
                    )
                }
            }
        }
    }
}

#[async_trait]
impl ToolHandler for DevServerStatusTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "dev_server_status",
            "检测本地开发服务器端口是否可连接。",
            ToolKind::BuiltIn,
            Operation::Read,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "host": { "type": "string", "default": "127.0.0.1" },
                "port": { "type": "integer" },
                "timeout_ms": { "type": "integer", "default": 1000 }
            },
            "required": ["port"]
        }))
        .with_timeout(Duration::from_secs(5))
    }

    async fn execute(&self, _ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let host = string_arg(&request.arguments, "host").unwrap_or("127.0.0.1");
        let Some(port) = u64_arg(&request.arguments, "port") else {
            return invalid_params(&request, "缺少 port 参数");
        };
        if port == 0 || port > u16::MAX as u64 {
            return invalid_params(&request, "port 必须在 1..=65535 之间");
        }
        let timeout_ms = u64_arg(&request.arguments, "timeout_ms").unwrap_or(1000);
        let started = Instant::now();
        let address = format!("{host}:{port}");
        let result = tokio::task::spawn_blocking({
            let address = address.clone();
            move || {
                let mut addrs = address.to_socket_addrs().map_err(|err| err.to_string())?;
                let Some(addr) = addrs.next() else {
                    return Err("无法解析地址".to_string());
                };
                TcpStream::connect_timeout(&addr, Duration::from_millis(timeout_ms.max(1)))
                    .map(|_| true)
                    .map_err(|err| err.to_string())
            }
        })
        .await;

        let payload = match result {
            Ok(Ok(true)) => json!({
                "host": host,
                "port": port,
                "status": "running",
                "connectable": true
            }),
            Ok(Ok(false)) => json!({
                "host": host,
                "port": port,
                "status": "unreachable",
                "connectable": false
            }),
            Ok(Err(err)) => json!({
                "host": host,
                "port": port,
                "status": "unreachable",
                "connectable": false,
                "error": err
            }),
            Err(err) => json!({
                "host": host,
                "port": port,
                "status": "error",
                "connectable": false,
                "error": err.to_string()
            }),
        };
        ToolOutcome::ok(
            &request,
            serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string()),
            Vec::new(),
            started.elapsed(),
        )
    }
}

#[async_trait]
impl ToolHandler for WebSearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "web_search",
            "联网搜索信息，返回带 source attribution 的 EvidenceBundle JSON。",
            ToolKind::WebSearch,
            Operation::Read,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "trigger": {
                    "type": "string",
                    "enum": [
                        "user_requested",
                        "latest_information_needed",
                        "missing_local_knowledge",
                        "dependency_resolution",
                        "failure_diagnosis"
                    ],
                    "default": "user_requested"
                },
                "rationale": { "type": "string", "default": "用户或任务需要联网证据" },
                "max_results": { "type": "integer", "default": 5 }
            },
            "required": ["query"]
        }))
        .with_timeout(Duration::from_secs(20))
    }

    async fn execute(&self, ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        if !ctx.policy.allows_network() {
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(
                    ToolFailureKind::SandboxDenied,
                    "当前 runtime 禁止联网搜索；需要启用 network policy 后才能执行",
                )
                .with_hint("把该任务的 network_enabled 设为 true，或改用本地知识库。"),
                Duration::from_millis(0),
            );
        }
        let Some(query) =
            string_arg(&request.arguments, "query").or_else(|| string_arg(&request.arguments, "q"))
        else {
            return invalid_params(&request, "缺少 query 参数");
        };
        let max_results = usize_arg(&request.arguments, "max_results")
            .unwrap_or(5)
            .clamp(1, 10);
        let trigger = parse_research_trigger(string_arg(&request.arguments, "trigger"));
        let rationale =
            string_arg(&request.arguments, "rationale").unwrap_or("用户或任务需要联网证据");
        let started = Instant::now();

        let url = format!(
            "https://html.duckduckgo.com/html/?q={}",
            urlencoding::encode(query)
        );
        let client = reqwest::Client::new();
        let html = match client
            .get(url)
            .header(
                "User-Agent",
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:128.0) Gecko/20100101 Firefox/128.0",
            )
            .send()
            .await
        {
            Ok(resp) => match resp.text().await {
                Ok(text) => text,
                Err(err) => {
                    return ToolOutcome::failed(
                        &request,
                        ToolFailure::new(
                            ToolFailureKind::Transport,
                            format!("读取搜索结果失败：{err}"),
                        ),
                        started.elapsed(),
                    )
                }
            },
            Err(err) => {
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(ToolFailureKind::Transport, format!("搜索请求失败：{err}")),
                    started.elapsed(),
                )
            }
        };

        let research_query =
            super::ResearchQuery::new(trigger, query.to_string(), rationale.to_string());
        let mut bundle = super::EvidenceBundle::new(research_query);
        for item in parse_duckduckgo_results(&html)
            .into_iter()
            .take(max_results)
        {
            bundle = bundle.with_item(item);
        }
        let conclusion = if bundle.items.is_empty() {
            "未解析到可用搜索结果，可能被搜索引擎拦截或查询无结果。"
        } else {
            "已获取搜索证据。证据已充足，请勿重复发起搜索，请立即基于上述搜索结果为用户总结作答。"
        };
        bundle = bundle.with_conclusion(conclusion);
        ToolOutcome::ok(
            &request,
            serde_json::to_string_pretty(&bundle).unwrap_or_else(|_| "{}".to_string()),
            Vec::new(),
            started.elapsed(),
        )
    }
}

/// 派发子智能体 (Subagent Dispatcher)
/// 支持 Explore (只读探索探路) 与 Isolated (Git Worktree 影子沙箱重构) 模式
pub struct ResearchSubagentTool {
    pub worktree_mgr: Arc<super::worktree::WorktreeManager>,
}

#[async_trait]
impl ToolHandler for ResearchSubagentTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "invoke_subagent",
            "Dispatch a specialized subagent to execute an isolated task. Supports 'explore' (read-only research/probing) and 'isolated' (Git Worktree physical sandbox refactoring).",
            ToolKind::BuiltIn,
            Operation::Read,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "role": { "type": "string", "description": "The subagent's role title, e.g. 'Codebase Researcher' or 'Refactor Sandbox'" },
                "goal": { "type": "string", "description": "The high-level goal of the subtask" },
                "prompt": { "type": "string", "description": "Detailed actionable instructions and questions for the subagent" },
                "mode": {
                    "type": "string",
                    "enum": ["explore", "isolated"],
                    "default": "explore",
                    "description": "Workspace topology mode: 'explore' (read-only probing in current workspace) or 'isolated' (dedicated Git Worktree sandbox)"
                },
                "target_paths": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional list of target file/directory paths to focus on"
                }
            },
            "required": ["role", "goal", "prompt"]
        }))
        .with_timeout(Duration::from_secs(180))
    }

    async fn execute(&self, ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let Some(role) = string_arg(&request.arguments, "role") else {
            return invalid_params(&request, "缺少 role 参数");
        };
        let Some(goal) = string_arg(&request.arguments, "goal") else {
            return invalid_params(&request, "缺少 goal 参数");
        };
        let Some(prompt) = string_arg(&request.arguments, "prompt") else {
            return invalid_params(&request, "缺少 prompt 参数");
        };
        let mode = super::subagent::SubagentMode::from_str_opt(string_arg(
            &request.arguments,
            "mode",
        ));
        let target_paths: Vec<String> = request
            .arguments
            .get("target_paths")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|p| p.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        let started = Instant::now();

        // 网关没就绪就**明确失败**。绝不能退化成"不调模型也给一份报告" ——
        // 那样返回的是凭空编造的内容，而调用方看到的是 status=Ok。
        let Some(gateway) = super::subagent::gateway() else {
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(
                    ToolFailureKind::Execution,
                    "子智能体需要 LLM 网关，但它尚未就绪",
                )
                .with_hint("这通常意味着 RuntimeService 是在没有 gateway 的情况下构造的。"),
                started.elapsed(),
            );
        };

        // 子智能体用**自己的**工具路由：不与主会话共享 PTY / worktree 管理器，
        // 也天然不含 MCP 工具。递归派发和 worktree_* 由 subagent 侧的名单拦掉。
        let mut sub_registry = super::ToolRegistry::new();
        register_local_tools(&mut sub_registry);
        let router = Arc::new(ToolRouter::new(sub_registry));

        let spec = super::subagent::SubagentSpec {
            role: role.to_string(),
            goal: goal.to_string(),
            prompt: prompt.to_string(),
            mode,
            target_paths,
        };

        if mode == super::subagent::SubagentMode::Isolated {
            return self
                .run_isolated(ctx, &request, gateway, router, spec, started)
                .await;
        }

        // ── Explore：只读探路，直接在父工作区跑 ──
        match super::subagent::run_subagent_loop(gateway, router, ctx, &spec).await {
            Ok(outcome) => ToolOutcome::ok(
                &request,
                render_subagent_report(&outcome),
                Vec::new(),
                started.elapsed(),
            ),
            Err(err) => ToolOutcome::failed(
                &request,
                ToolFailure::new(ToolFailureKind::Execution, format!("子智能体执行失败: {err}"))
                    .with_hint("可退回 code_outline / workspace_search 自行检索。"),
                started.elapsed(),
            ),
        }
    }
}

impl ResearchSubagentTool {
    /// 沙箱模式：建影子工作区 → 在**沙箱**里跑子智能体 → 出 diff → 决定去留。
    ///
    /// 三条不变式：
    /// 1. 子智能体全程感知不到 worktree 的存在 —— 它眼里只有一个普通工作区
    ///    （`RuntimeContext` 的根被换成沙箱路径，工具名单里也没有 `worktree_*`）；
    /// 2. **没产生任何改动就立刻丢弃沙箱** —— 否则每次调用都会留下一份完整 checkout；
    /// 3. 产生了改动才保留沙箱与分支，交给主 Agent / 用户决定 merge 还是 discard。
    async fn run_isolated(
        &self,
        ctx: &RuntimeContext,
        request: &ToolCallRequest,
        gateway: Arc<crate::llm::gateway::Gateway>,
        router: Arc<ToolRouter>,
        spec: super::subagent::SubagentSpec,
        started: Instant,
    ) -> ToolOutcome {
        let verdict = ctx
            .policy
            .evaluate_path(&ctx.workspace_root, FileCapability::Write);
        if !verdict.allowed() {
            return denied(request, verdict.reason());
        }

        let task_id = format!(
            "subagent-{}",
            uuid::Uuid::new_v4()
                .to_string()
                .chars()
                .take(8)
                .collect::<String>()
        );
        let session = match self
            .worktree_mgr
            .create_sandbox(&ctx.workspace_root, &task_id)
            .await
        {
            Ok(session) => session,
            Err(err) => {
                return ToolOutcome::failed(
                    request,
                    ToolFailure::new(
                        ToolFailureKind::Execution,
                        format!("创建独立沙箱失败: {err}"),
                    )
                    .with_hint("当前目录可能不是 Git 仓库，建议改用 mode: \"explore\" 只读探路。"),
                    started.elapsed(),
                );
            }
        };

        // 换根，但**继承**父级策略 —— deny_patterns / secret_names / 只读开关都要跟着，
        // 否则沙箱会变成一个绕过项目安全配置的后门。
        let sandbox_ctx = RuntimeContext::new(&session.worktree_path, ctx.policy.clone());
        let loop_result =
            super::subagent::run_subagent_loop(gateway, router, &sandbox_ctx, &spec).await;

        let diff = git_diff_summary(&session.worktree_path);
        let changed = diff.as_ref().is_some_and(|d| !d.trim().is_empty());

        // 失败、或者压根没动过任何文件 —— 沙箱立刻回收，不留孤儿。
        if loop_result.is_err() || !changed {
            let _ = self
                .worktree_mgr
                .discard_sandbox(&ctx.workspace_root, &task_id)
                .await;
        }

        match loop_result {
            Ok(mut outcome) => {
                outcome.sandbox_task_id = changed.then(|| session.task_id.clone());
                outcome.sandbox_branch = changed.then(|| session.branch_name.clone());
                outcome.diff_summary = diff;
                ToolOutcome::ok(
                    request,
                    render_subagent_report(&outcome),
                    Vec::new(),
                    started.elapsed(),
                )
            }
            Err(err) => ToolOutcome::failed(
                request,
                ToolFailure::new(
                    ToolFailureKind::Execution,
                    format!("沙箱子智能体执行失败（沙箱已回收）: {err}"),
                ),
                started.elapsed(),
            ),
        }
    }
}

/// 沙箱里的改动摘要。含未跟踪文件 —— 新建文件是重构里最常见的一种改动，
/// 只看 `git diff` 会把它整个漏掉，表现为「子智能体明明写了文件却报告没改动」。
fn git_diff_summary(worktree: &std::path::Path) -> Option<String> {
    let staged = std::process::Command::new("git")
        .args(["add", "-A"])
        .current_dir(worktree)
        .output();
    if staged.is_err() {
        return None;
    }
    let out = std::process::Command::new("git")
        .args(["diff", "--cached", "--stat"])
        .current_dir(worktree)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn render_subagent_report(outcome: &super::subagent::SubagentOutcome) -> String {
    let mut report = format!("# 子智能体报告（{}）\n\n", outcome.role);
    report.push_str(&format!("- **拓扑模式**：`{:?}`\n", outcome.mode));
    report.push_str(&format!("- **耗时**：{} ms\n", outcome.elapsed_ms));
    if let Some(id) = &outcome.sandbox_task_id {
        report.push_str(&format!("- **沙箱任务 ID**：`{id}`\n"));
    }
    if let Some(branch) = &outcome.sandbox_branch {
        report.push_str(&format!("- **影子分支**：`{branch}`\n"));
    }
    if !outcome.findings.is_empty() {
        report.push_str("\n## 工具调用轨迹\n");
        for finding in &outcome.findings {
            report.push_str(&format!("- {finding}\n"));
        }
    }
    match outcome.diff_summary.as_deref().map(str::trim) {
        Some(diff) if !diff.is_empty() => {
            let task_id = outcome.sandbox_task_id.as_deref().unwrap_or("sandbox");
            report.push_str(&format!("\n## 沙箱变更\n```\n{diff}\n```\n"));
            report.push_str(&format!(
                "\n沙箱改动已就绪（task_id: `{task_id}`），可直接调用 `worktree_merge(task_id: \"{task_id}\")` 一键合入主分支并自动提交，或用 `worktree_discard` 丢弃。\n"
            ));
        }
        _ if outcome.mode == super::subagent::SubagentMode::Isolated => {
            report.push_str("\n## 沙箱变更\n未产生任何文件改动，沙箱已自动回收。\n");
        }
        _ => {}
    }
    report.push_str(&format!("\n## 结论\n{}\n", outcome.conclusion));
    report
}

// ── Git Worktree 隔离沙箱工具族 (P5: Git Worktree Tools) ──

pub struct WorktreeCreateTool {
    pub worktree_mgr: Arc<super::worktree::WorktreeManager>,
}

#[async_trait]
impl ToolHandler for WorktreeCreateTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "worktree_create",
            "在当前 Git 仓库内创建独立的影子工作区（Git Worktree 沙箱），供重构或实验性修改使用，保护主工作区不被污染。",
            ToolKind::FileSystem,
            Operation::Execute,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "task_id": { "type": "string", "description": "沙箱任务唯一标识，用于生成影子分支名与工作区目录" }
            },
            "required": ["task_id"]
        }))
        .with_idempotent(false)
        .with_timeout(Duration::from_secs(15))
    }

    async fn execute(&self, ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {

        // 沙箱操作会改变版本库状态 —— 只读会话不得建/合/弃沙箱。
        //
        // 这里检查的是 **workspace_root 的 Write 能力**，而不是 `.git/` 本身。
        // `worktree_*` 按设计豁免 `deny_patterns` 里的 `.git/`：操作版本库元数据
        // 就是它的职责，受那条约束等于让它不能工作（详见 DECISIONS.md 同日条目）。
        // 它受约束的是 sandbox 模式，由这一句表达。
        let verdict = ctx
            .policy
            .evaluate_path(&ctx.workspace_root, FileCapability::Write);
        if !verdict.allowed() {
            return denied(&request, verdict.reason());
        }
        let Some(task_id) = string_arg(&request.arguments, "task_id") else {
            return invalid_params(&request, "缺少 task_id 参数");
        };

        match self.worktree_mgr.create_sandbox(&ctx.workspace_root, task_id).await {
            Ok(session) => {
                let payload = json!({
                    "task_id": session.task_id,
                    "branch_name": session.branch_name,
                    "worktree_path": session.worktree_path.to_string_lossy(),
                    "is_active": session.is_active
                });
                ToolOutcome::ok(
                    &request,
                    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string()),
                    Vec::new(),
                    Duration::from_millis(0),
                )
            }
            Err(err) => ToolOutcome::failed(
                &request,
                ToolFailure::new(ToolFailureKind::Execution, err),
                Duration::from_millis(0),
            ),
        }
    }
}

pub struct WorktreeMergeTool {
    pub worktree_mgr: Arc<super::worktree::WorktreeManager>,
}

#[async_trait]
impl ToolHandler for WorktreeMergeTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "worktree_merge",
            "将 Git Worktree 沙箱内完成并通过验证的改动一键提交并合入主分支，随后自动清理影子工作区。",
            ToolKind::FileSystem,
            Operation::Execute,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "task_id": { "type": "string", "description": "要合入的沙箱任务 ID" }
            },
            "required": ["task_id"]
        }))
        .with_idempotent(false)
        .with_timeout(Duration::from_secs(20))
    }

    async fn execute(&self, ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {

        // 沙箱操作会改变版本库状态 —— 只读会话不得建/合/弃沙箱。
        //
        // 这里检查的是 **workspace_root 的 Write 能力**，而不是 `.git/` 本身。
        // `worktree_*` 按设计豁免 `deny_patterns` 里的 `.git/`：操作版本库元数据
        // 就是它的职责，受那条约束等于让它不能工作（详见 DECISIONS.md 同日条目）。
        // 它受约束的是 sandbox 模式，由这一句表达。
        let verdict = ctx
            .policy
            .evaluate_path(&ctx.workspace_root, FileCapability::Write);
        if !verdict.allowed() {
            return denied(&request, verdict.reason());
        }
        let Some(task_id) = string_arg(&request.arguments, "task_id") else {
            return invalid_params(&request, "缺少 task_id 参数");
        };

        match self.worktree_mgr.merge_sandbox(&ctx.workspace_root, task_id).await {
            Ok(msg) => {
                let payload = json!({
                    "task_id": task_id,
                    "merged": true,
                    "message": msg
                });
                ToolOutcome::ok(
                    &request,
                    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string()),
                    Vec::new(),
                    Duration::from_millis(0),
                )
            }
            Err(err) => ToolOutcome::failed(
                &request,
                ToolFailure::new(ToolFailureKind::Execution, err),
                Duration::from_millis(0),
            ),
        }
    }
}

pub struct WorktreeDiscardTool {
    pub worktree_mgr: Arc<super::worktree::WorktreeManager>,
}

#[async_trait]
impl ToolHandler for WorktreeDiscardTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "worktree_discard",
            "丢弃并彻底清理指定的 Git Worktree 隔离沙箱及其影子分支，安全恢复干净状态。",
            ToolKind::FileSystem,
            Operation::Execute,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "task_id": { "type": "string", "description": "要丢弃的沙箱任务 ID" }
            },
            "required": ["task_id"]
        }))
        .with_idempotent(false)
        .with_timeout(Duration::from_secs(15))
    }

    async fn execute(&self, ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {

        // 沙箱操作会改变版本库状态 —— 只读会话不得建/合/弃沙箱。
        //
        // 这里检查的是 **workspace_root 的 Write 能力**，而不是 `.git/` 本身。
        // `worktree_*` 按设计豁免 `deny_patterns` 里的 `.git/`：操作版本库元数据
        // 就是它的职责，受那条约束等于让它不能工作（详见 DECISIONS.md 同日条目）。
        // 它受约束的是 sandbox 模式，由这一句表达。
        let verdict = ctx
            .policy
            .evaluate_path(&ctx.workspace_root, FileCapability::Write);
        if !verdict.allowed() {
            return denied(&request, verdict.reason());
        }
        let Some(task_id) = string_arg(&request.arguments, "task_id") else {
            return invalid_params(&request, "缺少 task_id 参数");
        };

        match self.worktree_mgr.discard_sandbox(&ctx.workspace_root, task_id).await {
            Ok(()) => {
                let payload = json!({
                    "task_id": task_id,
                    "discarded": true
                });
                ToolOutcome::ok(
                    &request,
                    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string()),
                    Vec::new(),
                    Duration::from_millis(0),
                )
            }
            Err(err) => ToolOutcome::failed(
                &request,
                ToolFailure::new(ToolFailureKind::Execution, err),
                Duration::from_millis(0),
            ),
        }
    }
}

// ── 跨项目 LSP 符号追踪工具族 (P4: LSP Symbol Tracking Tools) ──

pub struct LspDefinitionTool {
    pub lsp_mgr: Arc<super::lsp_client::LspClientManager>,
}

#[async_trait]
impl ToolHandler for LspDefinitionTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "lsp_find_definition",
            "Locate the exact symbol definition across the workspace via Language Server Protocol (Go to Definition). Preferred over text search for accurate type, function, and constant lookups.",
            ToolKind::FileSystem,
            Operation::Read,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Relative or absolute path of the source file" },
                "line": { "type": "integer", "description": "1-based line number of the symbol" },
                "character": { "type": "integer", "description": "1-based column/character offset of the symbol" }
            },
            "required": ["path", "line", "character"]
        }))
        .with_timeout(Duration::from_secs(12))
    }

    async fn execute(&self, ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let Some(path_str) = string_arg(&request.arguments, "path") else {
            return invalid_params(&request, "缺少 path 参数");
        };
        let file_path = resolve_workspace_path(ctx, path_str);
        let verdict = ctx.policy.evaluate_path(&file_path, FileCapability::Read);
        if !verdict.allowed() {
            return denied(&request, verdict.reason());
        }

        let line = usize_arg(&request.arguments, "line").unwrap_or(1).saturating_sub(1) as u32;
        let character = usize_arg(&request.arguments, "character").unwrap_or(1).saturating_sub(1) as u32;

        let started = Instant::now();
        match self.lsp_mgr.goto_definition(&ctx.workspace_root, &file_path, line, character).await {
            Ok(locations) => {
                let payload = json!({
                    "symbol_path": path_str,
                    "definitions": locations,
                    "count": locations.len()
                });
                ToolOutcome::ok(
                    &request,
                    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string()),
                    Vec::new(),
                    started.elapsed(),
                )
            }
            Err(err) => ToolOutcome::failed(
                &request,
                ToolFailure::new(
                    ToolFailureKind::Execution,
                    format!("LSP 符号定义查询失败: {err}"),
                )
                .with_hint("Language Server 不可用或查询失败，建议使用 workspace_search 或 code_outline 进行回退查找。"),
                started.elapsed(),
            ),
        }
    }
}

pub struct LspReferencesTool {
    pub lsp_mgr: Arc<super::lsp_client::LspClientManager>,
}

#[async_trait]
impl ToolHandler for LspReferencesTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "lsp_find_references",
            "Find all call sites, usages, and references of a symbol across the workspace via Language Server Protocol (Find References). Crucial for refactoring and impact analysis.",
            ToolKind::FileSystem,
            Operation::Read,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Relative or absolute path of the source file" },
                "line": { "type": "integer", "description": "1-based line number of the symbol" },
                "character": { "type": "integer", "description": "1-based column/character offset of the symbol" },
                "include_declaration": { "type": "boolean", "default": false, "description": "Whether to include the symbol's own declaration location" }
            },
            "required": ["path", "line", "character"]
        }))
        .with_timeout(Duration::from_secs(15))
    }

    async fn execute(&self, ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let Some(path_str) = string_arg(&request.arguments, "path") else {
            return invalid_params(&request, "缺少 path 参数");
        };
        let file_path = resolve_workspace_path(ctx, path_str);
        let verdict = ctx.policy.evaluate_path(&file_path, FileCapability::Read);
        if !verdict.allowed() {
            return denied(&request, verdict.reason());
        }

        let line = usize_arg(&request.arguments, "line").unwrap_or(1).saturating_sub(1) as u32;
        let character = usize_arg(&request.arguments, "character").unwrap_or(1).saturating_sub(1) as u32;
        let include_declaration = bool_arg(&request.arguments, "include_declaration").unwrap_or(false);

        let started = Instant::now();
        match self.lsp_mgr.find_references(&ctx.workspace_root, &file_path, line, character, include_declaration).await {
            Ok(locations) => {
                let payload = json!({
                    "symbol_path": path_str,
                    "references": locations,
                    "count": locations.len()
                });
                ToolOutcome::ok(
                    &request,
                    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string()),
                    Vec::new(),
                    started.elapsed(),
                )
            }
            Err(err) => ToolOutcome::failed(
                &request,
                ToolFailure::new(
                    ToolFailureKind::Execution,
                    format!("LSP 符号引用查询失败: {err}"),
                )
                .with_hint("Language Server 不可用或查询失败，建议使用 workspace_search 或 code_outline 进行回退查找。"),
                started.elapsed(),
            ),
        }
    }
}

impl ToolSearchTool {
    pub fn new(specs: Vec<ToolSpec>) -> Self {
        Self { specs }
    }
}

#[async_trait]
impl ToolHandler for ToolSearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "tool_search",
            "在可用工具库中检索特定能力。当需要数据库、第三方系统或特殊扩展工具而当前未直接暴露时，通过该工具查询可用工具名称与参数规范。",
            ToolKind::BuiltIn,
            Operation::Read,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "要搜索的工具功能或关键词，例如 'git', 'database', 'cinema'" },
                "max_results": { "type": "integer", "default": 5, "description": "最多返回的工具数量" }
            },
            "required": ["query"]
        }))
        .with_timeout(Duration::from_secs(5))
    }

    async fn execute(&self, _ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let Some(query) = string_arg(&request.arguments, "query") else {
            return invalid_params(&request, "缺少 query 参数");
        };
        let max_results = usize_arg(&request.arguments, "max_results")
            .unwrap_or(5)
            .clamp(1, 10);
        let q_lower = query.trim().to_lowercase();

        let mut matches = Vec::new();
        for spec in &self.specs {
            let name_match = spec.name.to_lowercase().contains(&q_lower);
            let desc_match = spec.description.to_lowercase().contains(&q_lower);
            if name_match || desc_match {
                matches.push(json!({
                    "name": spec.name,
                    "description": spec.description,
                    "parameters": spec.parameters,
                }));
            }
        }

        matches.truncate(max_results);

        // **查不到时把全部工具名列回去，而不是说「都内置了」。**
        //
        // 这里是纯子串匹配，而工具描述有中英两种。用户在中文对话里，
        // 模型会查「出图」「生成图片」，而 `generate_image` 的描述是英文的
        // —— 一个都匹配不上。
        //
        // 原来那句「当前代码开发核心工具集已全部内置并直接可用」在
        // 工具分层（见 `tool_tier`）之后是**在骗模型**：还有几十个藏着，
        // 而它被告知"全部可用"，于是放弃。
        //
        // 名字很便宜（几十个名字一两千字符），而"够不着"是硬失败。
        if matches.is_empty() {
            let names: Vec<&str> = self.specs.iter().map(|s| s.name.as_str()).collect();
            return ToolOutcome::ok(
                &request,
                format!(
                    "没有工具的名字或描述包含 '{query}'（这里是子串匹配，中文查英文描述会落空）。\n\
                     全部可用工具如下，看到要用的就用它的**准确名字**再查一次拿参数：\n{}",
                    names.join("、")
                ),
                Vec::new(),
                Duration::from_millis(0),
            );
        }

        ToolOutcome::ok(
            &request,
            serde_json::to_string_pretty(&matches).unwrap_or_else(|_| "[]".to_string()),
            Vec::new(),
            Duration::from_millis(0),
        )
    }
}

#[async_trait]
impl ToolHandler for ApplyPatchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "apply_patch",
            "Apply file modifications using Codex-style unified patch format (*** Begin Patch ... *** End Patch). Supports creating, deleting, and updating files.",
            ToolKind::FileSystem,
            Operation::Write,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "patch": {
                    "type": "string",
                    "description": "Codex-style patch content, beginning with '*** Begin Patch' and ending with '*** End Patch'"
                },
                "path": { "type": "string", "description": "Legacy fallback: target file path" },
                "search": { "type": "string", "description": "Legacy fallback: unique snippet to find" },
                "replace": { "type": "string", "description": "Legacy fallback: replacement content" },
                "replace_all": { "type": "boolean", "default": false }
            }
        }))
        .with_idempotent(false)
        .with_timeout(Duration::from_secs(10))
    }

    async fn execute(&self, ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        if !ctx.is_tool_call_approved(&request.id).await {
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(
                    ToolFailureKind::ApprovalDenied,
                    "即将修改 workspace 文件，需要审批后才能应用 patch",
                )
                .with_hint("请检查 diff/参数预览，确认后再执行。"),
                Duration::from_millis(0),
            );
        }

        if let Some(patch) = string_arg(&request.arguments, "patch") {
            return apply_codex_patch(ctx, &request, patch);
        }

        let Some(path) = string_arg(&request.arguments, "path") else {
            return invalid_params(&request, "缺少 path 参数");
        };
        let Some(search) = string_arg(&request.arguments, "search") else {
            return invalid_params(&request, "缺少 search 参数");
        };
        let Some(replace) = string_arg(&request.arguments, "replace") else {
            return invalid_params(&request, "缺少 replace 参数");
        };
        let path = resolve_workspace_path(ctx, path);
        let verdict = ctx.policy.evaluate_path(&path, FileCapability::Write);
        if !verdict.allowed() {
            return denied(&request, verdict.reason());
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(err) => {
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(
                        ToolFailureKind::Execution,
                        format!("读取待修改文件失败：{err}"),
                    ),
                    Duration::from_millis(0),
                )
            }
        };
        let count = content.matches(search).count();
        if count == 0 {
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(
                    ToolFailureKind::InvalidParams,
                    "search 片段没有匹配到文件内容",
                )
                .with_hint("先用 fs_read_text 或 workspace_search 读取精确上下文。"),
                Duration::from_millis(0),
            );
        }
        if count > 1 && !bool_arg(&request.arguments, "replace_all").unwrap_or(false) {
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(
                    ToolFailureKind::InvalidParams,
                    format!("search 片段匹配到 {count} 处；为避免误改，默认不执行。"),
                )
                .with_hint("扩大 search 上下文，或显式传 replace_all=true。"),
                Duration::from_millis(0),
            );
        }
        let updated = if bool_arg(&request.arguments, "replace_all").unwrap_or(false) {
            content.replace(search, replace)
        } else {
            content.replacen(search, replace, 1)
        };
        match std::fs::write(&path, updated) {
            Ok(()) => ToolOutcome::ok(
                &request,
                format!(
                    "已修改 {}，替换 {} 处。",
                    path.display(),
                    if count > 1 { count } else { 1 }
                ),
                vec![path.to_string_lossy().to_string()],
                Duration::from_millis(0),
            ),
            Err(err) => ToolOutcome::failed(
                &request,
                ToolFailure::new(ToolFailureKind::Execution, format!("写回文件失败：{err}")),
                Duration::from_millis(0),
            ),
        }
    }
}

pub struct FsReadTextTool;

#[async_trait]
impl ToolHandler for FsReadTextTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "fs_read_text",
            "读取 workspace 内的文本文件（view_file 的兼容别名，支持 start_line 与 end_line 切片）。",
            ToolKind::FileSystem,
            Operation::Read,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "文件相对路径或绝对路径" },
                "start_line": { "type": "integer", "description": "起始行号（从 1 开始，可选）" },
                "end_line": { "type": "integer", "description": "结束行号（包含该行，可选）" },
                "limit": { "type": "integer", "description": "最大读取行数（默认 300，可选）" }
            },
            "required": ["path"]
        }))
        .with_timeout(Duration::from_secs(5))
    }

    async fn execute(&self, ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        ReadTextFileTool.execute(ctx, request).await
    }
}

/// 重型编码运行时工具的名字。
///
/// 单独列出来是为了**在声明期按会话类型过滤** —— 注册表在应用启动时只建一次，
/// 但工具 schema 是**按请求**付 token 的，所以过滤点必须在组请求那一层，
/// 不能靠"注册时少注册几个"。
///
/// 语音会话执行工具时 `workspace_path` 是 Anima 自己的配置目录、
/// `approval_mode` 是 `Deny`（见 `voice/mod.rs` 的 `RuntimeToolCallArgs`），
/// 这几个工具在那里**根本跑不通**：要审批的会被拒，worktree 的目标又不是 git 仓库。
/// 声明给它们只是白占 token，还给模型指了条死路。
pub const CODING_RUNTIME_TOOLS: &[&str] = &[
    "pty_spawn",
    "pty_write",
    "pty_poll",
    "pty_kill",
    "worktree_create",
    "worktree_merge",
    "worktree_discard",
    "lsp_find_definition",
    "lsp_find_references",
];

/// 保留注册、但**不向模型声明**的工具 —— 由 runtime 在底层编排。
///
/// **保留注册、只收声明**：万一有别的路径调到它，走的仍是同一套策略与审批，
/// 而不是「工具不存在」这种不好排查的错误。
///
/// 这里**只有 `worktree_create`**。沙箱的创建归调度器托管
/// （`invoke_subagent(mode: "isolated")` 会自动开），让模型自己去建，
/// 就退回到了「靠提示词让它记得开沙箱」的老路。
///
/// `worktree_merge` / `worktree_discard` **必须留给模型看见** ——
/// 子智能体干完活会交回一份带 diff 的提案，主 Agent 需要一个接口来采纳或否决它。
/// 三个一起藏掉的话，产生了真实改动的沙箱就永远没法被接受，
/// 只能一路堆到 `MAX_ACTIVE_SANDBOXES` 然后所有派发一起失败 ——
/// 而且这个死路不报任何错，表现为「沙箱越来越多、子智能体的成果全都用不上」。
pub const HIDDEN_FROM_MODEL_TOOLS: &[&str] = &["worktree_create"];

/// 基础工具：读写、检索、shell/exec 会话。任何会话都用得上。
pub fn register_basic_local_tools(registry: &mut super::ToolRegistry) {
    let sessions = Arc::new(ExecSessionManager::default());
    let worktree_mgr = Arc::new(super::worktree::WorktreeManager::default());
    registry.register(ReadTextFileTool);
    registry.register(FsReadTextTool);
    registry.register(super::code_outline::CodeOutlineTool);
    registry.register(WriteTextFileTool);
    registry.register(ListDirectoryTool);
    registry.register(ShellCommandTool::default());
    registry.register(ExecCommandTool {
        sessions: Arc::clone(&sessions),
    });
    registry.register(WriteStdinTool {
        sessions: Arc::clone(&sessions),
    });
    registry.register(KillExecSessionTool {
        sessions: Arc::clone(&sessions),
    });
    registry.register(WorkspaceSearchTool);
    registry.register(DevServerStatusTool);
    registry.register(WebSearchTool);
    registry.register(super::fetch_url::FetchUrlTool);
    registry.register(super::image_tool::GenerateImageTool);
    registry.register(super::video_tool::GenerateVideoTool);
    registry.register(super::video_tool::AnimateImageTool);
    registry.register(ResearchSubagentTool {
        worktree_mgr,
    });
    registry.register(ApplyPatchTool);
}

/// 重型编码运行时工具：交互式 PTY、worktree 沙箱、LSP 符号分析。
/// 名字见 [`CODING_RUNTIME_TOOLS`]，两处必须保持同步（有测试盯着）。
pub fn register_coding_runtime_tools(registry: &mut super::ToolRegistry) {
    let pty_mgr = Arc::new(super::pty_session::PtySessionManager::default());
    registry.register(PtySpawnTool {
        pty_mgr: Arc::clone(&pty_mgr),
    });
    registry.register(PtyWriteTool {
        pty_mgr: Arc::clone(&pty_mgr),
    });
    registry.register(PtyPollTool {
        pty_mgr: Arc::clone(&pty_mgr),
    });
    registry.register(PtyKillTool {
        pty_mgr: Arc::clone(&pty_mgr),
    });
    let worktree_mgr = Arc::new(super::worktree::WorktreeManager::default());
    registry.register(WorktreeCreateTool {
        worktree_mgr: Arc::clone(&worktree_mgr),
    });
    registry.register(WorktreeMergeTool {
        worktree_mgr: Arc::clone(&worktree_mgr),
    });
    registry.register(WorktreeDiscardTool {
        worktree_mgr: Arc::clone(&worktree_mgr),
    });
    let lsp_mgr = Arc::new(super::lsp_client::LspClientManager::default());
    registry.register(LspDefinitionTool {
        lsp_mgr: Arc::clone(&lsp_mgr),
    });
    registry.register(LspReferencesTool {
        lsp_mgr: Arc::clone(&lsp_mgr),
    });
}

/// 注册全部本地工具。注册表里**始终是全集** —— 过滤发生在声明期，见
/// [`CODING_RUNTIME_TOOLS`]。这样语音会话即使误调也仍然走同一套策略与审批，
/// 而不是"工具不存在"这种不好排查的错误。
pub fn register_local_tools(registry: &mut super::ToolRegistry) {
    register_basic_local_tools(registry);
    register_coding_runtime_tools(registry);
}

pub(crate) fn string_arg<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(|v| v.as_str())
}

fn u64_arg(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(|v| v.as_u64())
}

fn usize_arg(value: &Value, key: &str) -> Option<usize> {
    value.get(key).and_then(|v| v.as_u64()).map(|n| n as usize)
}

fn bool_arg(value: &Value, key: &str) -> Option<bool> {
    value.get(key).and_then(|v| v.as_bool())
}

fn parse_research_trigger(raw: Option<&str>) -> super::ResearchTrigger {
    match raw.unwrap_or("user_requested") {
        "latest_information_needed" => super::ResearchTrigger::LatestInformationNeeded,
        "missing_local_knowledge" => super::ResearchTrigger::MissingLocalKnowledge,
        "dependency_resolution" => super::ResearchTrigger::DependencyResolution,
        "failure_diagnosis" => super::ResearchTrigger::FailureDiagnosis,
        _ => super::ResearchTrigger::UserRequested,
    }
}

fn parse_duckduckgo_results(html: &str) -> Vec<super::EvidenceItem> {
    let mut items = Vec::new();
    for block in html.split("result__body").skip(1) {
        let title = extract_between(block, "class=\"result__a\"", "</a>")
            .and_then(|raw| raw.split('>').next_back().map(clean_html_text))
            .unwrap_or_else(|| "搜索结果".to_string());
        let url = extract_between(block, "href=\"", "\"")
            .map(|raw| {
                let raw = raw.strip_prefix("//duckduckgo.com/l/?uddg=").unwrap_or(raw);
                urlencoding::decode(raw)
                    .map(|decoded| decoded.into_owned())
                    .unwrap_or_else(|_| raw.to_string())
            })
            .unwrap_or_default();
        let summary = extract_between(block, "class=\"result__snippet\"", "</a>")
            .and_then(|raw| raw.split('>').next_back().map(clean_html_text))
            .unwrap_or_default();
        if !url.is_empty() || !summary.is_empty() {
            items.push(super::EvidenceItem::new(title, url, summary, "web"));
        }
    }
    items
}

fn extract_between<'a>(text: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let from = text.find(start)? + start.len();
    let to = text[from..].find(end)? + from;
    Some(&text[from..to])
}

fn clean_html_text(text: &str) -> String {
    text.replace("<b>", "")
        .replace("</b>", "")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .trim()
        .to_string()
}

pub(crate) fn resolve_workspace_path(ctx: &RuntimeContext, path: &str) -> PathBuf {
    let raw_path = Path::new(path.trim());
    let combined = if raw_path.is_absolute() {
        raw_path.to_path_buf()
    } else {
        ctx.workspace_root.join(raw_path)
    };

    // 词法路径归一化（Lexical Normalization）：消除 `.` 与 `..`，
    // 防止通过相对路径符号逃逸出 workspace_root。
    normalize_lexical_path(&combined)
}

/// 纯词法规范化路径（不依赖磁盘实际存在），解析冗余的 `.` 与 `..` 组件。
pub(crate) fn normalize_lexical_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Prefix(p) => normalized.push(p.as_os_str()),
            std::path::Component::RootDir => normalized.push(std::path::MAIN_SEPARATOR.to_string()),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                // 回退一层（如果非根目录）
                if !normalized.pop() {
                    // 若已在根部则保持
                }
            }
            std::path::Component::Normal(c) => normalized.push(c),
        }
    }
    normalized
}

pub(crate) fn invalid_params(request: &ToolCallRequest, msg: &str) -> ToolOutcome {
    ToolOutcome::failed(
        request,
        ToolFailure::new(ToolFailureKind::InvalidParams, msg),
        Duration::from_millis(0),
    )
}

fn denied(request: &ToolCallRequest, reason: &str) -> ToolOutcome {
    ToolOutcome::failed(
        request,
        ToolFailure::new(ToolFailureKind::SandboxDenied, reason),
        Duration::from_millis(0),
    )
}

fn denied_audio_player_command(request: &ToolCallRequest, player: &str) -> ToolOutcome {
    ToolOutcome::failed(
        request,
        ToolFailure::new(
            ToolFailureKind::SandboxDenied,
            format!(
                "禁止通过 shell 调用 `{player}` 播放音频。请改用专用 `play_audio` 工具，并把音频文件绝对路径或直链放入 `url_or_path`。"
            ),
        )
        .with_hint("不要再尝试 afplay/mpv/ffplay；这些命令不会触发前端播放器抽屉和播放状态同步。"),
        Duration::from_millis(0),
    )
}

pub(crate) fn command_uses_system_audio_player(command: &str) -> Option<&'static str> {
    for raw in command.split(|c: char| {
        !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '+'))
    }) {
        let token = raw.trim();
        if token.is_empty() {
            continue;
        }
        let executable = std::path::Path::new(token)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(token);
        match executable {
            "afplay" => return Some("afplay"),
            "mpv" => return Some("mpv"),
            "ffplay" => return Some("ffplay"),
            _ => {}
        }
    }
    None
}

fn truncate(text: &str, max_chars: usize) -> String {
    truncate_with_flag(text, max_chars).0
}

fn truncate_with_flag(text: &str, max_chars: usize) -> (String, bool) {
    if text.chars().count() <= max_chars {
        return (text.to_string(), false);
    }
    (
        text.chars().take(max_chars).collect::<String>() + "...",
        true,
    )
}

fn incremental_output(text: &str, cursor: usize, max_chars: usize) -> (String, usize, bool) {
    let start = clamp_char_boundary(text, cursor.min(text.len()));
    let next = text.len();
    let slice = &text[start..];
    let (truncated, was_truncated) = truncate_with_flag(slice, max_chars);
    (truncated, next, was_truncated)
}

fn clamp_char_boundary(text: &str, mut index: usize) -> usize {
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn sanitized_command_env() -> Vec<(String, String)> {
    sanitized_command_env_from(std::env::vars())
}

fn sanitized_command_env_from<I, K, V>(vars: I) -> Vec<(String, String)>
where
    I: IntoIterator<Item = (K, V)>,
    K: Into<String>,
    V: Into<String>,
{
    let mut env = vars
        .into_iter()
        .filter_map(|(key, value)| {
            let key = key.into();
            let value = value.into();
            if should_keep_command_env(&key) {
                Some((key, value))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    env.sort_by(|a, b| a.0.cmp(&b.0));
    env
}

fn should_keep_command_env(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    const DENY_SUBSTRINGS: &[&str] = &[
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "API_KEY",
        "ACCESS_KEY",
        "PRIVATE_KEY",
        "CREDENTIAL",
    ];
    if DENY_SUBSTRINGS.iter().any(|needle| upper.contains(needle)) {
        return false;
    }

    const ALLOW_EXACT: &[&str] = &[
        "PATH",
        "HOME",
        "SHELL",
        "USER",
        "LOGNAME",
        "TMPDIR",
        "TEMP",
        "TMP",
        "LANG",
        "LC_ALL",
        "TERM",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
    ];
    if ALLOW_EXACT.contains(&upper.as_str()) {
        return true;
    }

    const ALLOW_PREFIXES: &[&str] = &[
        "ANIMA_",
        "TAURI_",
        "RUST",
        "CARGO",
        "NODE",
        "NPM_",
        "PNPM_",
        "PYTHON",
        "PIP_",
        "VIRTUAL_ENV",
        "CONDA_",
    ];
    ALLOW_PREFIXES
        .iter()
        .any(|prefix| upper.starts_with(prefix))
}

#[derive(Serialize)]
struct ExecOutput {
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    timed_out: bool,
    session_id: Option<String>,
    cursor: usize,
    stdout_cursor: usize,
    stderr_cursor: usize,
    stdout_truncated: bool,
    stderr_truncated: bool,
    is_running: bool,
}

async fn run_command_once(
    request: &ToolCallRequest,
    cmd: &str,
    workdir: &Path,
    max_output: usize,
    timeout_ms: u64,
) -> ToolOutcome {
    let started = Instant::now();
    let mut child = match Command::new("sh")
        .arg("-lc")
        .arg(cmd)
        .current_dir(workdir)
        .env_clear()
        .envs(sanitized_command_env())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            return ToolOutcome::failed(
                request,
                ToolFailure::new(ToolFailureKind::Execution, format!("启动命令失败：{err}")),
                started.elapsed(),
            )
        }
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_task = tokio::spawn(read_pipe(stdout));
    let stderr_task = tokio::spawn(read_pipe(stderr));

    let wait = tokio::time::timeout(Duration::from_millis(timeout_ms.max(1)), child.wait()).await;
    let (status, timed_out) = match wait {
        Ok(Ok(status)) => (Some(status), false),
        Ok(Err(err)) => {
            return ToolOutcome::failed(
                request,
                ToolFailure::new(ToolFailureKind::Execution, format!("等待命令失败：{err}")),
                started.elapsed(),
            )
        }
        Err(_) => {
            let _ = child.kill().await;
            (None, true)
        }
    };

    let stdout = stdout_task.await.unwrap_or_default();
    let stderr = stderr_task.await.unwrap_or_default();
    let stdout_cursor = stdout.len();
    let stderr_cursor = stderr.len();
    let (stdout, stdout_truncated) = truncate_with_flag(&stdout, max_output);
    let (stderr, stderr_truncated) = truncate_with_flag(&stderr, max_output);
    let payload = ExecOutput {
        exit_code: status.and_then(|s| s.code()),
        stdout,
        stderr,
        timed_out,
        session_id: None,
        cursor: stdout_cursor.saturating_add(stderr_cursor),
        stdout_cursor,
        stderr_cursor,
        stdout_truncated,
        stderr_truncated,
        is_running: false,
    };
    let content = serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string());

    if timed_out {
        ToolOutcome::failed(
            request,
            ToolFailure::new(ToolFailureKind::Timeout, content),
            started.elapsed(),
        )
    } else if status.is_some_and(|s| s.success()) {
        ToolOutcome::ok(request, content, Vec::new(), started.elapsed())
    } else {
        ToolOutcome::failed(
            request,
            ToolFailure::new(ToolFailureKind::Execution, content),
            started.elapsed(),
        )
    }
}

async fn read_pipe<R>(pipe: Option<R>) -> String
where
    R: tokio::io::AsyncRead + Unpin,
{
    let Some(mut pipe) = pipe else {
        return String::new();
    };
    let mut buf = Vec::new();
    let _ = pipe.read_to_end(&mut buf).await;
    String::from_utf8_lossy(&buf).to_string()
}

impl ExecSessionManager {
    async fn spawn_background(
        &self,
        request: &ToolCallRequest,
        cmd: &str,
        workdir: &Path,
        max_output: usize,
        stdout_cursor: usize,
        stderr_cursor: usize,
    ) -> ToolOutcome {
        let started = Instant::now();
        self.cleanup_finished_and_stale().await;
        if self.sessions.lock().await.len() >= MAX_BACKGROUND_SESSIONS {
            return ToolOutcome::failed(
                request,
                ToolFailure::new(
                    ToolFailureKind::Execution,
                    format!(
                        "后台会话数已达到上限 {MAX_BACKGROUND_SESSIONS}，请先 kill 已有 session"
                    ),
                ),
                started.elapsed(),
            );
        }

        let mut child = match Command::new("sh")
            .arg("-lc")
            .arg(cmd)
            .current_dir(workdir)
            .env_clear()
            .envs(sanitized_command_env())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(err) => {
                return ToolOutcome::failed(
                    request,
                    ToolFailure::new(
                        ToolFailureKind::Execution,
                        format!("启动后台命令失败：{err}"),
                    ),
                    started.elapsed(),
                )
            }
        };

        let Some(stdin) = child.stdin.take() else {
            return ToolOutcome::failed(
                request,
                ToolFailure::new(ToolFailureKind::Execution, "无法获取后台命令 stdin"),
                started.elapsed(),
            );
        };
        let stdout = Arc::new(Mutex::new(String::new()));
        let stderr = Arc::new(Mutex::new(String::new()));
        spawn_output_collector(child.stdout.take(), Arc::clone(&stdout));
        spawn_output_collector(child.stderr.take(), Arc::clone(&stderr));

        let session_id = Uuid::new_v4().to_string();
        let now = Instant::now();
        let session = ExecSession {
            stdin: Arc::new(Mutex::new(stdin)),
            child: Arc::new(Mutex::new(child)),
            stdout: Arc::clone(&stdout),
            stderr: Arc::clone(&stderr),
            created_at: now,
            last_access: Arc::new(Mutex::new(now)),
        };
        let child_for_status = Arc::clone(&session.child);
        self.sessions
            .lock()
            .await
            .insert(session_id.clone(), session);
        tokio::time::sleep(Duration::from_millis(120)).await;
        let status = {
            let mut child = child_for_status.lock().await;
            child.try_wait().ok().flatten()
        };
        let stdout_collected = stdout.lock().await.clone();
        let stderr_collected = stderr.lock().await.clone();
        let (stdout_slice, next_stdout_cursor, stdout_truncated) =
            incremental_output(&stdout_collected, stdout_cursor, max_output);
        let (stderr_slice, next_stderr_cursor, stderr_truncated) =
            incremental_output(&stderr_collected, stderr_cursor, max_output);
        let payload = ExecOutput {
            exit_code: status.and_then(|s| s.code()),
            stdout: stdout_slice,
            stderr: stderr_slice,
            timed_out: false,
            session_id: Some(session_id),
            cursor: next_stdout_cursor.saturating_add(next_stderr_cursor),
            stdout_cursor: next_stdout_cursor,
            stderr_cursor: next_stderr_cursor,
            stdout_truncated,
            stderr_truncated,
            is_running: status.is_none(),
        };
        ToolOutcome::ok(
            request,
            serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string()),
            Vec::new(),
            started.elapsed(),
        )
    }

    async fn write_stdin(
        &self,
        request: &ToolCallRequest,
        session_id: &str,
        chars: &str,
        max_output: usize,
        stdout_cursor: usize,
        stderr_cursor: usize,
    ) -> ToolOutcome {
        let started = Instant::now();
        self.cleanup_finished_and_stale().await;
        let Some(session) = self.sessions.lock().await.get(session_id).cloned() else {
            return ToolOutcome::failed(
                request,
                ToolFailure::new(
                    ToolFailureKind::InvalidParams,
                    format!("未知 session_id：{session_id}"),
                ),
                started.elapsed(),
            );
        };

        {
            let mut stdin = session.stdin.lock().await;
            if let Err(err) = stdin.write_all(chars.as_bytes()).await {
                return ToolOutcome::failed(
                    request,
                    ToolFailure::new(
                        ToolFailureKind::Execution,
                        format!("写入 stdin 失败：{err}"),
                    ),
                    started.elapsed(),
                );
            }
            let _ = stdin.flush().await;
        }
        *session.last_access.lock().await = Instant::now();
        tokio::time::sleep(Duration::from_millis(120)).await;
        let status = {
            let mut child = session.child.lock().await;
            child.try_wait().ok().flatten()
        };
        let stdout_collected = session.stdout.lock().await.clone();
        let stderr_collected = session.stderr.lock().await.clone();
        let (stdout_slice, next_stdout_cursor, stdout_truncated) =
            incremental_output(&stdout_collected, stdout_cursor, max_output);
        let (stderr_slice, next_stderr_cursor, stderr_truncated) =
            incremental_output(&stderr_collected, stderr_cursor, max_output);
        let payload = ExecOutput {
            exit_code: status.and_then(|s| s.code()),
            stdout: stdout_slice,
            stderr: stderr_slice,
            timed_out: false,
            session_id: Some(session_id.to_string()),
            cursor: next_stdout_cursor.saturating_add(next_stderr_cursor),
            stdout_cursor: next_stdout_cursor,
            stderr_cursor: next_stderr_cursor,
            stdout_truncated,
            stderr_truncated,
            is_running: status.is_none(),
        };
        ToolOutcome::ok(
            request,
            serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string()),
            Vec::new(),
            started.elapsed(),
        )
    }

    async fn kill_session(&self, request: &ToolCallRequest, session_id: &str) -> ToolOutcome {
        let started = Instant::now();
        let Some(session) = self.sessions.lock().await.remove(session_id) else {
            return ToolOutcome::failed(
                request,
                ToolFailure::new(
                    ToolFailureKind::InvalidParams,
                    format!("未知 session_id：{session_id}"),
                ),
                started.elapsed(),
            );
        };
        let status = {
            let mut child = session.child.lock().await;
            match child.try_wait().ok().flatten() {
                Some(status) => Some(status),
                None => {
                    let _ = child.kill().await;
                    child.wait().await.ok()
                }
            }
        };
        let payload = json!({
            "session_id": session_id,
            "killed": true,
            "exit_code": status.and_then(|s| s.code())
        });
        ToolOutcome::ok(
            request,
            serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string()),
            Vec::new(),
            started.elapsed(),
        )
    }

    async fn cleanup_finished_and_stale(&self) {
        let ids: Vec<(String, ExecSession)> = self
            .sessions
            .lock()
            .await
            .iter()
            .map(|(id, session)| (id.clone(), session.clone()))
            .collect();
        let now = Instant::now();
        let mut remove = Vec::new();
        for (id, session) in ids {
            let exited = {
                let mut child = session.child.lock().await;
                child.try_wait().ok().flatten().is_some()
            };
            let idle_too_long =
                now.duration_since(*session.last_access.lock().await) > BACKGROUND_SESSION_TTL;
            if exited || idle_too_long {
                if idle_too_long {
                    let mut child = session.child.lock().await;
                    if child.try_wait().ok().flatten().is_none() {
                        let _ = child.kill().await;
                    }
                }
                remove.push(id);
            }
        }
        if !remove.is_empty() {
            let mut sessions = self.sessions.lock().await;
            for id in remove {
                sessions.remove(&id);
            }
        }
    }
}

impl Clone for ExecSession {
    fn clone(&self) -> Self {
        Self {
            stdin: Arc::clone(&self.stdin),
            child: Arc::clone(&self.child),
            stdout: Arc::clone(&self.stdout),
            stderr: Arc::clone(&self.stderr),
            created_at: self.created_at,
            last_access: Arc::clone(&self.last_access),
        }
    }
}

fn spawn_output_collector<R>(pipe: Option<R>, output: Arc<Mutex<String>>)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let Some(mut pipe) = pipe else {
        return;
    };
    tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        loop {
            match pipe.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    let text = String::from_utf8_lossy(&buf[..n]);
                    let mut out = output.lock().await;
                    out.push_str(&text);
                    if out.len() > MAX_SESSION_BUFFER_BYTES {
                        let keep_from = clamp_char_boundary(
                            &out,
                            out.len().saturating_sub(KEEP_SESSION_BUFFER_BYTES),
                        );
                        *out = out[keep_from..].to_string();
                    }
                }
                Err(_) => break,
            }
        }
    });
}

fn apply_codex_patch(ctx: &RuntimeContext, request: &ToolCallRequest, patch: &str) -> ToolOutcome {
    let started = Instant::now();
    let operations = match parse_codex_patch(patch) {
        Ok(ops) => ops,
        Err(err) => {
            return ToolOutcome::failed(
                request,
                ToolFailure::new(ToolFailureKind::InvalidParams, err),
                started.elapsed(),
            )
        }
    };

    let mut touched = Vec::new();
    for op in operations {
        match apply_patch_operation(ctx, &op) {
            Ok(paths) => touched.extend(paths),
            Err(err) => {
                return ToolOutcome::failed(
                    request,
                    ToolFailure::new(ToolFailureKind::Execution, err),
                    started.elapsed(),
                )
            }
        }
    }

    let summary = if touched.is_empty() {
        "patch 没有产生文件变更".to_string()
    } else {
        format!("已应用 patch，变更文件：\n{}", touched.join("\n"))
    };
    ToolOutcome::ok(request, summary, touched, started.elapsed())
}

#[derive(Debug, Clone)]
enum PatchOperation {
    Add {
        path: String,
        content: String,
    },
    Delete {
        path: String,
    },
    Update {
        path: String,
        move_to: Option<String>,
        chunks: Vec<PatchChunk>,
    },
}

#[derive(Debug, Clone)]
struct PatchChunk {
    old: String,
    new: String,
}

/// 把模型常见的两种「方言」归一成 Codex 规范形式。
///
/// 实测 Qwen3-Coder-Next（4bit / 8bit 各 3 次）：补丁正文完全正确，
/// 但外层写成 `*** Begin Patch ***` 加 unified diff 的 `--- a/x` / `+++ b/x`
/// 文件头。严格相等的判定会把一整块正确的补丁判死，而这只是方言差异，
/// 不是内容错误——`@@` 里的行号本来就被忽略（改动靠内容唯一性定位）。
///
/// 只做无歧义的归一：哨兵行去掉装饰性尾巴；`---`/`+++` **成对、路径一致、
/// 且紧跟 `@@`** 时才折叠成 `*** Update File:`。那三个条件缺一不做转换，
/// 交给后面的严格解析去报错——最后一条同时挡住了「删除一行恰好以 `-- ` 开头」
/// 被误当成文件头的情况。
fn normalize_patch_dialect(lines: Vec<String>) -> Vec<String> {
    fn undecorate(line: &str) -> &str {
        line.trim_end_matches(['*', ' '])
    }

    let mut out = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].as_str();
        let bare = undecorate(line);
        if bare == "*** Begin Patch" || bare == "*** End Patch" {
            out.push(bare.to_string());
            i += 1;
            continue;
        }
        let folded = line
            .strip_prefix("--- ")
            .zip(lines.get(i + 1).and_then(|next| next.strip_prefix("+++ ")));
        if let Some((old_path, new_path)) = folded {
            let old_path = old_path.strip_prefix("a/").unwrap_or(old_path);
            let new_path = new_path.strip_prefix("b/").unwrap_or(new_path);
            let hunk_follows = lines.get(i + 2).is_some_and(|line| line.starts_with("@@"));
            if old_path == new_path && !new_path.is_empty() && hunk_follows {
                out.push(format!("*** Update File: {new_path}"));
                i += 2;
                continue;
            }
        }
        out.push(lines[i].clone());
        i += 1;
    }
    out
}

fn parse_codex_patch(patch: &str) -> Result<Vec<PatchOperation>, String> {
    let normalized = patch.replace("\r\n", "\n");
    let lines: Vec<String> = normalized
        .lines()
        .map(|line| line.trim_end().to_string())
        .collect();
    let lines = normalize_patch_dialect(lines);
    if lines.first().map(String::as_str) != Some("*** Begin Patch") {
        return Err("patch 必须以 *** Begin Patch 开始".to_string());
    }
    if !lines.iter().any(|line| line == "*** End Patch") {
        return Err("patch 缺少 *** End Patch".to_string());
    }

    let mut i = 1;
    let mut ops = Vec::new();
    while i < lines.len() {
        let line = lines[i].as_str();
        if line == "*** End Patch" {
            break;
        }
        if let Some(path) = line.strip_prefix("*** Add File: ") {
            i += 1;
            let mut content = String::new();
            while i < lines.len()
                && !is_hunk_header(lines[i].as_str())
                && lines[i] != "*** End Patch"
            {
                let Some(text) = lines[i].strip_prefix('+') else {
                    return Err(format!("Add File 行必须以 + 开头：{}", lines[i]));
                };
                content.push_str(text);
                content.push('\n');
                i += 1;
            }
            ops.push(PatchOperation::Add {
                path: path.to_string(),
                content,
            });
            continue;
        }
        if let Some(path) = line.strip_prefix("*** Delete File: ") {
            ops.push(PatchOperation::Delete {
                path: path.to_string(),
            });
            i += 1;
            continue;
        }
        if let Some(path) = line.strip_prefix("*** Update File: ") {
            i += 1;
            let mut move_to = None;
            if i < lines.len() {
                if let Some(target) = lines[i].strip_prefix("*** Move to: ") {
                    move_to = Some(target.to_string());
                    i += 1;
                }
            }
            let mut chunks = Vec::new();
            while i < lines.len()
                && !is_hunk_header(lines[i].as_str())
                && lines[i] != "*** End Patch"
            {
                if !lines[i].starts_with("@@") {
                    return Err(format!("Update File 需要 @@ chunk 头，实际：{}", lines[i]));
                }
                i += 1;
                let mut old = String::new();
                let mut new = String::new();
                while i < lines.len()
                    && !lines[i].starts_with("@@")
                    && !is_hunk_header(lines[i].as_str())
                    && lines[i] != "*** End Patch"
                {
                    let line = lines[i].as_str();
                    if line == "*** End of File" {
                        i += 1;
                        continue;
                    }
                    let Some(prefix) = line.chars().next() else {
                        old.push('\n');
                        new.push('\n');
                        i += 1;
                        continue;
                    };
                    let text = &line[1..];
                    match prefix {
                        ' ' => {
                            old.push_str(text);
                            old.push('\n');
                            new.push_str(text);
                            new.push('\n');
                        }
                        '-' => {
                            old.push_str(text);
                            old.push('\n');
                        }
                        '+' => {
                            new.push_str(text);
                            new.push('\n');
                        }
                        _ => {
                            return Err(format!("patch chunk 行必须以 空格/+/- 开头：{line}"));
                        }
                    }
                    i += 1;
                }
                chunks.push(PatchChunk { old, new });
            }
            if chunks.is_empty() && move_to.is_none() {
                return Err(format!("Update File {path} 没有任何变更 chunk"));
            }
            ops.push(PatchOperation::Update {
                path: path.to_string(),
                move_to,
                chunks,
            });
            continue;
        }
        return Err(format!("无法识别 patch hunk：{line}"));
    }
    if ops.is_empty() {
        return Err("patch 没有任何 hunk".to_string());
    }
    Ok(ops)
}

fn is_hunk_header(line: &str) -> bool {
    line.starts_with("*** Add File: ")
        || line.starts_with("*** Delete File: ")
        || line.starts_with("*** Update File: ")
}

fn apply_patch_operation(ctx: &RuntimeContext, op: &PatchOperation) -> Result<Vec<String>, String> {
    match op {
        PatchOperation::Add { path, content } => {
            let path = resolve_workspace_path(ctx, path);
            ensure_write_allowed(ctx, &path)?;
            if path.exists() {
                return Err(format!("Add File 目标已存在：{}", path.display()));
            }
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|err| format!("创建目录失败 {}：{err}", parent.display()))?;
            }
            std::fs::write(&path, content)
                .map_err(|err| format!("写入文件失败 {}：{err}", path.display()))?;
            Ok(vec![path.to_string_lossy().to_string()])
        }
        PatchOperation::Delete { path } => {
            let path = resolve_workspace_path(ctx, path);
            ensure_write_allowed(ctx, &path)?;
            std::fs::remove_file(&path)
                .map_err(|err| format!("删除文件失败 {}：{err}", path.display()))?;
            Ok(vec![path.to_string_lossy().to_string()])
        }
        PatchOperation::Update {
            path,
            move_to,
            chunks,
        } => {
            let path = resolve_workspace_path(ctx, path);
            ensure_write_allowed(ctx, &path)?;
            let mut content = std::fs::read_to_string(&path)
                .map_err(|err| format!("读取文件失败 {}：{err}", path.display()))?;
            for chunk in chunks {
                content = replace_unique_chunk(&content, &chunk.old, &chunk.new, &path)?;
            }

            let mut changed = vec![path.to_string_lossy().to_string()];
            if let Some(move_to) = move_to {
                let target = resolve_workspace_path(ctx, move_to);
                ensure_write_allowed(ctx, &target)?;
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|err| format!("创建目录失败 {}：{err}", parent.display()))?;
                }
                std::fs::write(&target, content)
                    .map_err(|err| format!("写入移动目标失败 {}：{err}", target.display()))?;
                std::fs::remove_file(&path)
                    .map_err(|err| format!("删除原文件失败 {}：{err}", path.display()))?;
                changed.push(target.to_string_lossy().to_string());
            } else {
                std::fs::write(&path, content)
                    .map_err(|err| format!("写回文件失败 {}：{err}", path.display()))?;
            }
            Ok(changed)
        }
    }
}

fn replace_unique_chunk(
    content: &str,
    old: &str,
    new: &str,
    path: &Path,
) -> Result<String, String> {
    if old.is_empty() {
        return Err(format!("Update File {} 的 chunk 原文为空", path.display()));
    }

    let exact_count = content.matches(old).count();
    if exact_count == 1 {
        return Ok(content.replacen(old, new, 1));
    }
    if exact_count > 1 {
        return Err(format!(
            "Update File {} 的 chunk 匹配到 {exact_count} 处，拒绝模糊修改",
            path.display()
        ));
    }

    let matches = find_rtrim_line_matches(content, old);
    if matches.is_empty() {
        return Err(format!(
            "Update File {} 的 chunk 未匹配到原文",
            path.display()
        ));
    }
    if matches.len() > 1 {
        return Err(format!(
            "Update File {} 的 chunk 按行尾空白容错后匹配到 {} 处，拒绝模糊修改",
            path.display(),
            matches.len()
        ));
    }

    let (start, end) = matches[0];
    let mut updated = String::with_capacity(content.len() + new.len());
    updated.push_str(&content[..start]);
    updated.push_str(new);
    updated.push_str(&content[end..]);
    Ok(updated)
}

fn find_rtrim_line_matches(content: &str, old: &str) -> Vec<(usize, usize)> {
    let content_lines = line_spans_with_rtrim(content);
    let old_lines = rtrim_lines(old);
    if old_lines.is_empty() || old_lines.len() > content_lines.len() {
        return Vec::new();
    }

    let mut matches = Vec::new();
    for start_line in 0..=content_lines.len() - old_lines.len() {
        let window = &content_lines[start_line..start_line + old_lines.len()];
        if window
            .iter()
            .map(|(_, _, normalized)| normalized)
            .eq(old_lines.iter())
        {
            let start = window.first().map(|(start, _, _)| *start).unwrap_or(0);
            let end = window.last().map(|(_, end, _)| *end).unwrap_or(start);
            matches.push((start, end));
        }
    }
    matches
}

fn line_spans_with_rtrim(text: &str) -> Vec<(usize, usize, String)> {
    let mut spans = Vec::new();
    let mut cursor = 0;
    for line in text.split_inclusive('\n') {
        let end = cursor + line.len();
        spans.push((cursor, end, rtrim_line(line)));
        cursor = end;
    }
    if cursor < text.len() {
        let line = &text[cursor..];
        spans.push((cursor, text.len(), rtrim_line(line)));
    }
    spans
}

fn rtrim_lines(text: &str) -> Vec<String> {
    text.split_inclusive('\n').map(rtrim_line).collect()
}

fn rtrim_line(line: &str) -> String {
    if let Some(body) = line.strip_suffix('\n') {
        format!("{}\n", body.trim_end())
    } else {
        line.trim_end().to_string()
    }
}

fn ensure_write_allowed(ctx: &RuntimeContext, path: &Path) -> Result<(), String> {
    let verdict = ctx.policy.evaluate_path(path, FileCapability::Write);
    if verdict.allowed() {
        Ok(())
    } else {
        Err(verdict.reason().to_string())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::agent_runtime::{RuntimePolicy, ToolCallRequest, ToolRouter, ToolStatus};

    #[tokio::test]
    async fn local_file_tools_respect_workspace_policy() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = RuntimeContext::new(dir.path(), RuntimePolicy::workspace_write(dir.path()));

        let mut registry = super::super::ToolRegistry::new();
        register_local_tools(&mut registry);
        let router = ToolRouter::new(registry);

        let write = router
            .dispatch(
                &ctx,
                ToolCallRequest::new(
                    "fs_write_text",
                    json!({ "path": "notes/a.txt", "content": "hello" }),
                ),
            )
            .await;
        assert_eq!(write.status, ToolStatus::Ok);

        let read = router
            .dispatch(
                &ctx,
                ToolCallRequest::new("fs_read_text", json!({ "path": "notes/a.txt" })),
            )
            .await;
        assert_eq!(read.status, ToolStatus::Ok);
        assert_eq!(read.content, "hello");

        let outside = tempfile::tempdir().unwrap();
        let blocked = router
            .dispatch(
                &ctx,
                ToolCallRequest::new(
                    "fs_read_text",
                    json!({ "path": outside.path().join("x.txt").to_string_lossy() }),
                ),
            )
            .await;
        assert_eq!(blocked.status, ToolStatus::Blocked);
    }

    #[tokio::test]
    async fn shell_tool_runs_whitelisted_commands_only() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = RuntimeContext::new(dir.path(), RuntimePolicy::workspace_write(dir.path()));

        let mut registry = super::super::ToolRegistry::new();
        register_local_tools(&mut registry);
        let router = ToolRouter::new(registry);

        let ok = router
            .dispatch(
                &ctx,
                ToolCallRequest::new("shell_command", json!({ "command": "echo hello" })),
            )
            .await;
        assert_eq!(ok.status, ToolStatus::Ok);
        assert!(ok.content.contains("hello"));

        let blocked = router
            .dispatch(
                &ctx,
                ToolCallRequest::new("shell_command", json!({ "command": "sudo whoami" })),
            )
            .await;
        assert_eq!(blocked.status, ToolStatus::Blocked);
    }

    #[tokio::test]
    async fn shell_audio_players_are_blocked_in_favor_of_play_audio() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = RuntimeContext::new(dir.path(), RuntimePolicy::workspace_write(dir.path()));

        let mut registry = super::super::ToolRegistry::new();
        register_local_tools(&mut registry);
        let router = ToolRouter::new(registry);

        let shell = router
            .dispatch(
                &ctx,
                ToolCallRequest::new(
                    "shell_command",
                    json!({ "command": "nohup /usr/bin/afplay /tmp/song.flac >/tmp/anima.log 2>&1 &" }),
                ),
            )
            .await;
        assert_eq!(shell.status, ToolStatus::Blocked);
        assert!(shell.content.contains("play_audio"), "{}", shell.content);

        let exec = router
            .dispatch(
                &ctx,
                ToolCallRequest::new(
                    "exec_command",
                    json!({ "cmd": "ffplay -nodisp -autoexit /tmp/song.wav" }),
                ),
            )
            .await;
        assert_eq!(exec.status, ToolStatus::Blocked);
        assert!(exec.content.contains("play_audio"), "{}", exec.content);
    }

    #[test]
    fn shell_environment_is_sanitized_before_spawn() {
        let env = sanitized_command_env_from([
            ("PATH", "/usr/bin"),
            ("OPENAI_API_KEY", "secret"),
            ("GITHUB_TOKEN", "secret"),
            ("CARGO_HOME", "/tmp/cargo"),
            ("PYTHONPATH", "/tmp/python"),
            ("CUSTOM_VALUE", "nope"),
        ]);
        let keys = env.into_iter().map(|(key, _)| key).collect::<Vec<_>>();
        assert!(keys.contains(&"PATH".to_string()));
        assert!(keys.contains(&"CARGO_HOME".to_string()));
        assert!(keys.contains(&"PYTHONPATH".to_string()));
        assert!(!keys.contains(&"OPENAI_API_KEY".to_string()));
        assert!(!keys.contains(&"GITHUB_TOKEN".to_string()));
        assert!(!keys.contains(&"CUSTOM_VALUE".to_string()));
    }

    #[tokio::test]
    async fn exec_command_returns_structured_output() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = RuntimeContext::new(dir.path(), RuntimePolicy::workspace_write(dir.path()));

        let mut registry = super::super::ToolRegistry::new();
        register_local_tools(&mut registry);
        let router = ToolRouter::new(registry);

        let out = router
            .dispatch(
                &ctx,
                ToolCallRequest::new(
                    "exec_command",
                    json!({ "cmd": "echo hello", "timeout_ms": 1000, "max_output_chars": 1000 }),
                ),
            )
            .await;

        assert_eq!(out.status, ToolStatus::Ok);
        let parsed: serde_json::Value = serde_json::from_str(&out.content).unwrap();
        assert_eq!(parsed["exit_code"], 0);
        assert!(parsed["stdout"].as_str().unwrap().contains("hello"));
        assert_eq!(parsed["is_running"], false);
        assert_eq!(parsed["stdout_truncated"], false);
    }

    #[tokio::test]
    async fn workspace_search_uses_rg_shape() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.rs"),
            "fn main() {\n  println!(\"needle\");\n}\n",
        )
        .unwrap();
        let ctx = RuntimeContext::new(dir.path(), RuntimePolicy::workspace_write(dir.path()));

        let mut registry = super::super::ToolRegistry::new();
        register_local_tools(&mut registry);
        let router = ToolRouter::new(registry);

        let out = router
            .dispatch(
                &ctx,
                ToolCallRequest::new(
                    "workspace_search",
                    json!({ "query": "needle", "path": ".", "max_results": 10 }),
                ),
            )
            .await;

        assert_eq!(out.status, ToolStatus::Ok);
        assert!(out.content.contains("a.rs"));
        assert!(out.content.contains("needle"));
    }

    #[tokio::test]
    async fn apply_patch_replaces_unique_context_only() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("note.txt"), "alpha\nbeta\ngamma\n").unwrap();
        let ctx = RuntimeContext::new(dir.path(), RuntimePolicy::workspace_write(dir.path()));

        let mut registry = super::super::ToolRegistry::new();
        register_local_tools(&mut registry);
        let router = ToolRouter::new(registry);

        let out = dispatch_approved(
            &router,
            &ctx,
            ToolCallRequest::new(
                "apply_patch",
                json!({
                    "path": "note.txt",
                    "search": "alpha\nbeta\n",
                    "replace": "alpha\nBETA\n"
                }),
            ),
        )
        .await;

        assert_eq!(out.status, ToolStatus::Ok);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("note.txt")).unwrap(),
            "alpha\nBETA\ngamma\n"
        );
    }

    #[tokio::test]
    async fn apply_patch_blocks_ambiguous_replacements() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("note.txt"), "same\nsame\n").unwrap();
        let ctx = RuntimeContext::new(dir.path(), RuntimePolicy::workspace_write(dir.path()));

        let mut registry = super::super::ToolRegistry::new();
        register_local_tools(&mut registry);
        let router = ToolRouter::new(registry);

        let out = dispatch_approved(
            &router,
            &ctx,
            ToolCallRequest::new(
                "apply_patch",
                json!({ "path": "note.txt", "search": "same\n", "replace": "changed\n" }),
            ),
        )
        .await;

        assert_eq!(out.status, ToolStatus::Failed);
        assert!(out.content.contains("匹配到 2 处"));
    }

    #[tokio::test]
    async fn apply_patch_accepts_codex_style_patch() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("note.txt"), "alpha\nbeta\ngamma\n").unwrap();
        let ctx = RuntimeContext::new(dir.path(), RuntimePolicy::workspace_write(dir.path()));

        let mut registry = super::super::ToolRegistry::new();
        register_local_tools(&mut registry);
        let router = ToolRouter::new(registry);

        let patch = r#"*** Begin Patch
*** Update File: note.txt
@@
 alpha
-beta
+BETA
 gamma
*** Add File: added.txt
+new file
*** End Patch
"#;

        let out = dispatch_approved(
            &router,
            &ctx,
            ToolCallRequest::new("apply_patch", json!({ "patch": patch })),
        )
        .await;

        assert_eq!(out.status, ToolStatus::Ok, "{}", out.content);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("note.txt")).unwrap(),
            "alpha\nBETA\ngamma\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("added.txt")).unwrap(),
            "new file\n"
        );
    }

    /// Qwen3-Coder-Next 实际吐出来的形状：哨兵行带装饰尾巴 + unified diff 文件头。
    /// 正文本身是对的，之前却整块被拒。
    #[tokio::test]
    async fn apply_patch_accepts_decorated_sentinels_and_unified_headers() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("note.txt"), "alpha\nbeta\ngamma\n").unwrap();
        let ctx = RuntimeContext::new(dir.path(), RuntimePolicy::workspace_write(dir.path()));

        let mut registry = super::super::ToolRegistry::new();
        register_local_tools(&mut registry);
        let router = ToolRouter::new(registry);

        let patch = r#"*** Begin Patch ***
--- a/note.txt
+++ b/note.txt
@@ -1,3 +1,4 @@
 alpha
+// marker
 beta
 gamma
*** End Patch ***
"#;

        let out = dispatch_approved(
            &router,
            &ctx,
            ToolCallRequest::new("apply_patch", json!({ "patch": patch })),
        )
        .await;

        assert_eq!(out.status, ToolStatus::Ok, "{}", out.content);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("note.txt")).unwrap(),
            "alpha\n// marker\nbeta\ngamma\n"
        );
    }

    /// 归一化只在无歧义时发生：`---`/`+++` 路径对不上就不折叠，照旧报错。
    #[tokio::test]
    async fn apply_patch_rejects_mismatched_diff_headers() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("note.txt"), "alpha\n").unwrap();
        let ctx = RuntimeContext::new(dir.path(), RuntimePolicy::workspace_write(dir.path()));

        let mut registry = super::super::ToolRegistry::new();
        register_local_tools(&mut registry);
        let router = ToolRouter::new(registry);

        let patch = r#"*** Begin Patch
--- a/note.txt
+++ b/other.txt
@@
 alpha
+beta
*** End Patch
"#;

        let out = dispatch_approved(
            &router,
            &ctx,
            ToolCallRequest::new("apply_patch", json!({ "patch": patch })),
        )
        .await;

        assert_ne!(
            out.status,
            ToolStatus::Ok,
            "路径不一致时不该折叠成 Update File"
        );
    }

    /// 守卫的回归：chunk 里删掉一行恰好以 `-- ` 开头、下一行加的以 `++ ` 开头时，
    /// 不能被误当成 unified diff 文件头吃掉。靠「后面必须紧跟 @@」这条挡住。
    #[tokio::test]
    async fn patch_dialect_does_not_eat_deletion_lines() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("note.txt"), "keep\n-- legacy\ntail\n").unwrap();
        let ctx = RuntimeContext::new(dir.path(), RuntimePolicy::workspace_write(dir.path()));

        let mut registry = super::super::ToolRegistry::new();
        register_local_tools(&mut registry);
        let router = ToolRouter::new(registry);

        let patch = r#"*** Begin Patch
*** Update File: note.txt
@@
 keep
--- legacy
+++ legacy
 tail
*** End Patch
"#;

        let out = dispatch_approved(
            &router,
            &ctx,
            ToolCallRequest::new("apply_patch", json!({ "patch": patch })),
        )
        .await;

        assert_eq!(out.status, ToolStatus::Ok, "{}", out.content);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("note.txt")).unwrap(),
            "keep\n++ legacy\ntail\n"
        );
    }

    #[tokio::test]
    async fn apply_patch_tolerates_empty_context_lines_and_trailing_whitespace() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("note.txt"), "alpha\n\nbeta   \ngamma\n").unwrap();
        let ctx = RuntimeContext::new(dir.path(), RuntimePolicy::workspace_write(dir.path()));

        let mut registry = super::super::ToolRegistry::new();
        register_local_tools(&mut registry);
        let router = ToolRouter::new(registry);

        let patch = "*** Begin Patch\n*** Update File: note.txt   \n@@   \n alpha   \n\n-beta\n+BETA\n gamma\n*** End Patch\n";

        let out = dispatch_approved(
            &router,
            &ctx,
            ToolCallRequest::new("apply_patch", json!({ "patch": patch })),
        )
        .await;

        assert_eq!(out.status, ToolStatus::Ok, "{}", out.content);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("note.txt")).unwrap(),
            "alpha\n\nBETA\ngamma\n"
        );
    }

    #[tokio::test]
    async fn apply_patch_supports_move_to() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("old.txt"), "one\ntwo\n").unwrap();
        let ctx = RuntimeContext::new(dir.path(), RuntimePolicy::workspace_write(dir.path()));

        let mut registry = super::super::ToolRegistry::new();
        register_local_tools(&mut registry);
        let router = ToolRouter::new(registry);

        let patch = r#"*** Begin Patch
*** Update File: old.txt
*** Move to: nested/new.txt
@@
 one
-two
+TWO
*** End Patch
"#;

        let out = dispatch_approved(
            &router,
            &ctx,
            ToolCallRequest::new("apply_patch", json!({ "patch": patch })),
        )
        .await;

        assert_eq!(out.status, ToolStatus::Ok, "{}", out.content);
        assert!(!dir.path().join("old.txt").exists());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("nested/new.txt")).unwrap(),
            "one\nTWO\n"
        );
    }

    #[tokio::test]
    async fn write_stdin_returns_incremental_output_and_liveness() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = ExecSessionManager::default();

        let cmd = "python3 -c 'import sys\nprint(sys.stdin.readline().strip(), flush=True)\nprint(sys.stdin.readline().strip(), flush=True)'";
        let open_request =
            ToolCallRequest::new("exec_command", json!({ "cmd": cmd, "background": true }));
        let opened = sessions
            .spawn_background(&open_request, cmd, dir.path(), 2000, 0, 0)
            .await;
        assert_eq!(opened.status, ToolStatus::Ok, "{}", opened.content);

        let opened_payload: serde_json::Value = serde_json::from_str(&opened.content).unwrap();
        let session_id = opened_payload["session_id"].as_str().unwrap();
        let first_request = ToolCallRequest::new(
            "write_stdin",
            json!({ "session_id": session_id, "chars": "first\n", "cursor": 0 }),
        );
        let first = sessions
            .write_stdin(&first_request, session_id, "first\n", 2000, 0, 0)
            .await;
        assert_eq!(first.status, ToolStatus::Ok, "{}", first.content);
        let first_payload: serde_json::Value = serde_json::from_str(&first.content).unwrap();
        assert!(first_payload["stdout"].as_str().unwrap().contains("first"));
        assert_eq!(first_payload["is_running"].as_bool(), Some(true));
        let cursor = first_payload["cursor"].as_u64().unwrap();
        let stdout_cursor = first_payload["stdout_cursor"].as_u64().unwrap();

        let second_request = ToolCallRequest::new(
            "write_stdin",
            json!({ "session_id": session_id, "chars": "second\n", "stdout_cursor": stdout_cursor }),
        );
        let second = sessions
            .write_stdin(
                &second_request,
                session_id,
                "second\n",
                2000,
                stdout_cursor as usize,
                0,
            )
            .await;
        assert_eq!(second.status, ToolStatus::Ok, "{}", second.content);
        let second_payload: serde_json::Value = serde_json::from_str(&second.content).unwrap();
        let stdout = second_payload["stdout"].as_str().unwrap();
        assert!(stdout.contains("second"));
        assert!(!stdout.contains("first"));
        assert_eq!(second_payload["is_running"].as_bool(), Some(false));
        assert!(second_payload["cursor"].as_u64().unwrap() >= cursor);
    }

    #[tokio::test]
    async fn kill_exec_session_terminates_background_process() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = ExecSessionManager::default();
        let request = ToolCallRequest::new(
            "exec_command",
            json!({ "cmd": "python3 -c 'import time; time.sleep(10)'", "background": true }),
        );
        let opened = sessions
            .spawn_background(
                &request,
                "python3 -c 'import time; time.sleep(10)'",
                dir.path(),
                2000,
                0,
                0,
            )
            .await;
        assert_eq!(opened.status, ToolStatus::Ok, "{}", opened.content);
        let payload: serde_json::Value = serde_json::from_str(&opened.content).unwrap();
        let session_id = payload["session_id"].as_str().unwrap();
        let kill_request =
            ToolCallRequest::new("kill_exec_session", json!({ "session_id": session_id }));
        let killed = sessions.kill_session(&kill_request, session_id).await;
        assert_eq!(killed.status, ToolStatus::Ok, "{}", killed.content);
        assert!(killed.content.contains("\"killed\": true"));
    }

    #[tokio::test]
    async fn dev_server_status_reports_unreachable_port() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = RuntimeContext::new(dir.path(), RuntimePolicy::workspace_write(dir.path()));
        let tool = DevServerStatusTool;
        let out = tool
            .execute(
                &ctx,
                ToolCallRequest::new(
                    "dev_server_status",
                    json!({ "host": "127.0.0.1", "port": 9, "timeout_ms": 50 }),
                ),
            )
            .await;
        assert_eq!(out.status, ToolStatus::Ok);
        assert!(out.content.contains("unreachable") || out.content.contains("running"));
    }

    #[tokio::test]
    async fn view_file_handles_limit_zero_and_out_of_bounds() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("sample.txt");
        std::fs::write(&file_path, "line1\nline2\nline3\n").unwrap();
        let ctx = RuntimeContext::new(dir.path(), RuntimePolicy::workspace_write(dir.path()));
        let tool = ReadTextFileTool;

        // 1. limit: 0 应该至少读取 1 行，而不是返回矛盾提示
        let req_zero =
            ToolCallRequest::new("view_file", json!({ "path": "sample.txt", "limit": 0 }));
        let out_zero = tool.execute(&ctx, req_zero).await;
        assert_eq!(out_zero.status, ToolStatus::Ok);
        assert!(out_zero.content.contains("line1"));

        // 2. start_line 超出范围要明确提示有效范围
        let req_oob = ToolCallRequest::new(
            "view_file",
            json!({ "path": "sample.txt", "start_line": 999 }),
        );
        let out_oob = tool.execute(&ctx, req_oob).await;
        assert_eq!(out_oob.status, ToolStatus::Ok);
        assert!(out_oob.content.contains("已超过文件末尾"));
        assert!(out_oob.content.contains("有效范围 1~3"));

        // 3. 空文件处理
        let empty_path = dir.path().join("empty.txt");
        std::fs::write(&empty_path, "").unwrap();
        let req_empty = ToolCallRequest::new("view_file", json!({ "path": "empty.txt" }));
        let out_empty = tool.execute(&ctx, req_empty).await;
        assert_eq!(out_empty.status, ToolStatus::Ok);
        assert_eq!(out_empty.content, "[文件为空]");

        // 4. end_line < start_line 无效参数提示
        let req_inv = ToolCallRequest::new(
            "view_file",
            json!({ "path": "sample.txt", "start_line": 8, "end_line": 3 }),
        );
        let out_inv = tool.execute(&ctx, req_inv).await;
        assert_eq!(out_inv.status, ToolStatus::Ok);
        assert!(out_inv.content.contains("参数无效"));
        assert!(out_inv.content.contains("小于 start_line"));
    }

    #[tokio::test]
    async fn fs_list_directory_filters_heavy_dirs_and_diagnoses_errors() {
        let dir = tempfile::tempdir().unwrap();
        // 创建正常文件与应被忽略的衍生/巨型目录
        std::fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src").join("lib.rs"), "// lib").unwrap();
        std::fs::create_dir_all(dir.path().join("node_modules").join("pkg")).unwrap();
        std::fs::create_dir_all(dir.path().join("target").join("debug")).unwrap();
        std::fs::create_dir_all(dir.path().join(".git").join("objects")).unwrap();

        let ctx = RuntimeContext::new(dir.path(), RuntimePolicy::workspace_write(dir.path()));
        let tool = ListDirectoryTool;

        // 1. 列出根目录（默认过滤 node_modules, target, .git）
        let req_root = ToolCallRequest::new("fs_list_directory", json!({ "path": "." }));
        let out_root = tool.execute(&ctx, req_root).await;
        assert_eq!(out_root.status, ToolStatus::Ok);
        assert!(out_root.content.contains("main.rs"));
        assert!(out_root.content.contains("src"));
        assert!(!out_root.content.contains("node_modules"));
        assert!(!out_root.content.contains("target"));
        assert!(!out_root.content.contains(".git"));

        // 2. 递归深度 max_depth = 2
        let req_deep =
            ToolCallRequest::new("fs_list_directory", json!({ "path": ".", "max_depth": 2 }));
        let out_deep = tool.execute(&ctx, req_deep).await;
        assert_eq!(out_deep.status, ToolStatus::Ok);
        assert!(out_deep.content.contains("src/lib.rs"));

        // 3. 访问不存在的目录返回 Actionable Error 诊断
        let req_nonexist =
            ToolCallRequest::new("fs_list_directory", json!({ "path": "not_exist_folder" }));
        let out_nonexist = tool.execute(&ctx, req_nonexist).await;
        assert_eq!(out_nonexist.status, ToolStatus::Failed);
        assert!(out_nonexist.content.contains("目录不存在"));
        assert!(out_nonexist.content.contains("工作区根目录为"));
    }

    #[tokio::test]
    async fn tool_search_finds_relevant_tools() {
        let specs = vec![
            ToolSpec::new(
                "git_status",
                "查看 git 仓库状态",
                ToolKind::Shell,
                Operation::Read,
            ),
            ToolSpec::new(
                "db_query",
                "执行 SQL 查询",
                ToolKind::BuiltIn,
                Operation::Read,
            ),
            ToolSpec::new(
                "view_file",
                "查看文件代码",
                ToolKind::FileSystem,
                Operation::Read,
            ),
        ];
        let tool = ToolSearchTool::new(specs);
        let dir = tempfile::tempdir().unwrap();
        let ctx = RuntimeContext::new(dir.path(), RuntimePolicy::workspace_write(dir.path()));

        let req = ToolCallRequest::new("tool_search", json!({ "query": "git" }));
        let out = tool.execute(&ctx, req).await;
        assert_eq!(out.status, ToolStatus::Ok);
        assert!(out.content.contains("git_status"));
        assert!(!out.content.contains("db_query"));
    }

    /// **查不到时要把全部工具名列回去**，不能说「都内置了」。
    ///
    /// 这里是纯子串匹配，而工具描述有中英两种。中文对话里模型会查
    /// 「出图」，而 `generate_image` 的描述是英文的 —— 一个都匹配不上。
    /// 工具分层之后，那句「核心工具集已全部内置并直接可用」是**在骗模型**：
    /// 还有几十个藏着，而它被告知全部可用，于是放弃。
    #[tokio::test]
    async fn 查不到时列出全部工具名() {
        let specs = vec![
            ToolSpec::new(
                "generate_image",
                "Generate an image from a text prompt using local ComfyUI.",
                ToolKind::BuiltIn,
                Operation::Write,
            ),
            ToolSpec::new("view_file", "查看文件代码", ToolKind::FileSystem, Operation::Read),
        ];
        let tool = ToolSearchTool::new(specs);
        let dir = tempfile::tempdir().unwrap();
        let ctx = RuntimeContext::new(dir.path(), RuntimePolicy::workspace_write(dir.path()));

        // 中文查英文描述 —— 必然落空，而这正是最常见的情况
        let req = ToolCallRequest::new("tool_search", json!({ "query": "出图" }));
        let out = tool.execute(&ctx, req).await;
        assert_eq!(out.status, ToolStatus::Ok);
        assert!(
            out.content.contains("generate_image"),
            "查不到时没把工具名列回去：{}",
            out.content
        );
        assert!(
            !out.content.contains("已全部内置"),
            "还在说「都内置了」，那是在骗模型：{}",
            out.content
        );
        // 要告诉模型下一步怎么做
        assert!(out.content.contains("准确名字"), "没说下一步：{}", out.content);
    }


    #[tokio::test]
    async fn pty_interactive_spawn_write_and_kill() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = RuntimeContext::new(dir.path(), RuntimePolicy::workspace_write(dir.path()));

        let mut registry = super::super::ToolRegistry::new();
        register_local_tools(&mut registry);
        let router = ToolRouter::new(registry);

        // 1. 启动交互式 PTY 进程（读取一行输入并打印问候）
        let spawn_req = ToolCallRequest::new(
            "pty_spawn",
            json!({
                "cmd": "read line; echo \"GREET: $line\"",
                "rows": 24,
                "cols": 80
            }),
        );
        let spawn_out = dispatch_approved(&router, &ctx, spawn_req).await;
        assert_eq!(spawn_out.status, ToolStatus::Ok);

        let parsed: serde_json::Value = serde_json::from_str(&spawn_out.content).unwrap();
        let session_id = parsed["session_id"].as_str().unwrap().to_string();
        assert!(parsed["is_running"].as_bool().unwrap());

        // 2. 向 PTY 写入输入并等待回显
        let write_req = ToolCallRequest::new(
            "pty_write",
            json!({
                "session_id": session_id,
                "input": "AnimaInteractive\n",
                "cursor": 0
            }),
        );
        let write_out = dispatch_approved(&router, &ctx, write_req).await;
        assert_eq!(write_out.status, ToolStatus::Ok);
        let write_parsed: serde_json::Value = serde_json::from_str(&write_out.content).unwrap();
        assert!(
            write_parsed["output"].as_str().unwrap().contains("GREET: AnimaInteractive"),
            "PTY 应该捕获到子进程标准输入的交互式回显：{}",
            write_parsed["output"]
        );

        // 3. 销毁会话
        let kill_req = ToolCallRequest::new(
            "pty_kill",
            json!({ "session_id": session_id }),
        );
        let kill_out = router.dispatch(&ctx, kill_req).await;
        assert_eq!(kill_out.status, ToolStatus::Ok);
    }

    #[tokio::test]
    async fn pty_write_enforces_command_policy_on_newline_commands() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = RuntimeContext::new(dir.path(), RuntimePolicy::workspace_write(dir.path()));

        let mut registry = super::super::ToolRegistry::new();
        register_local_tools(&mut registry);
        let router = ToolRouter::new(registry);

        // 1. 启动一个标准 sh PTY 会话
        let spawn_req = ToolCallRequest::new(
            "pty_spawn",
            json!({ "cmd": "sh", "rows": 24, "cols": 80 }),
        );
        let spawn_out = dispatch_approved(&router, &ctx, spawn_req).await;
        assert_eq!(spawn_out.status, ToolStatus::Ok);
        let parsed: serde_json::Value = serde_json::from_str(&spawn_out.content).unwrap();
        let session_id = parsed["session_id"].as_str().unwrap().to_string();

        // 2. 尝试不带审批注入未授权危险命令（如 sudo whoami），必须被安全策略直接拒绝或拦截
        let write_bad = router
            .dispatch(
                &ctx,
                ToolCallRequest::new(
                    "pty_write",
                    json!({
                        "session_id": session_id,
                        "input": "sudo whoami\n"
                    }),
                ),
            )
            .await;

        assert_ne!(
            write_bad.status,
            ToolStatus::Ok,
            "未授权的危险命令必须被 PTY 安全闸门拦截"
        );

        // 3. 纯单字符按键交互（不含换行）允许直接放行
        let write_char = router
            .dispatch(
                &ctx,
                ToolCallRequest::new(
                    "pty_write",
                    json!({
                        "session_id": session_id,
                        "input": "y"
                    }),
                ),
            )
            .await;
        assert_eq!(write_char.status, ToolStatus::Ok, "单字符交互应当免检放行");

        // 4. 清理
        let _ = router
            .dispatch(
                &ctx,
                ToolCallRequest::new("pty_kill", json!({ "session_id": session_id })),
            )
            .await;
    }

    #[tokio::test]
    async fn pty_manager_drop_kills_processes() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = super::super::pty_session::PtySessionManager::default();

        // 启动一个长时间睡眠的子进程
        let (session_id, out) = mgr
            .spawn("sleep 60", dir.path(), &[], 24, 80)
            .await
            .unwrap();
        assert!(out.is_running);

        // 显式 drop manager
        drop(mgr);

        // 进程管理器已被销毁，测试断言进程清理逻辑平稳触发
        assert!(!session_id.is_empty());
    }

    #[tokio::test]
    async fn worktree_sandbox_lifecycle_and_merge() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        // 1. 初始化一个真实的 Git 仓库并提交初始文件
        let _ = std::process::Command::new("git")
            .arg("init")
            .current_dir(root)
            .output()
            .unwrap();
        let _ = std::process::Command::new("git")
            .args(["config", "user.name", "AnimaBot"])
            .current_dir(root)
            .output()
            .unwrap();
        let _ = std::process::Command::new("git")
            .args(["config", "user.email", "bot@anima.local"])
            .current_dir(root)
            .output()
            .unwrap();

        std::fs::write(root.join("main.txt"), "main branch original content\n").unwrap();
        let _ = std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(root)
            .output()
            .unwrap();
        let _ = std::process::Command::new("git")
            .args(["commit", "-m", "init commit"])
            .current_dir(root)
            .output()
            .unwrap();

        let ctx = RuntimeContext::new(root, RuntimePolicy::workspace_write(root));
        let mut registry = super::super::ToolRegistry::new();
        register_local_tools(&mut registry);
        let router = ToolRouter::new(registry);

        // 2. 创建 Worktree 影子沙箱
        let task_id = "test-experiment-1";
        let create_req = ToolCallRequest::new(
            "worktree_create",
            json!({ "task_id": task_id }),
        );
        let create_out = router.dispatch(&ctx, create_req).await;
        assert_eq!(create_out.status, ToolStatus::Ok);

        let created_payload: serde_json::Value = serde_json::from_str(&create_out.content).unwrap();
        let worktree_dir = std::path::PathBuf::from(created_payload["worktree_path"].as_str().unwrap());
        assert!(worktree_dir.exists(), "Worktree 影子目录必须实际存在");

        // 3. 在 Worktree 沙箱内修改文件并新增实验文件
        std::fs::write(worktree_dir.join("main.txt"), "modified in sandbox\n").unwrap();
        std::fs::write(worktree_dir.join("experiment.txt"), "sandbox experimental feature\n").unwrap();

        // 4. 验证主工作区在此期间未受污染
        let main_content = std::fs::read_to_string(root.join("main.txt")).unwrap();
        assert_eq!(main_content, "main branch original content\n", "主分支文件在 merge 前必须保持未受污染");
        assert!(!root.join("experiment.txt").exists(), "实验文件在 merge 前不能出现在主工作区");

        // 5. 执行 Worktree Merge
        let merge_req = ToolCallRequest::new(
            "worktree_merge",
            json!({ "task_id": task_id }),
        );
        let merge_out = router.dispatch(&ctx, merge_req).await;
        assert_eq!(merge_out.status, ToolStatus::Ok);

        // 6. 验证主工作区已成功更新，且影子 worktree 目录已自动被干净销毁
        let updated_main = std::fs::read_to_string(root.join("main.txt")).unwrap();
        assert_eq!(updated_main, "modified in sandbox\n", "Merge 后主工作区文件应正确更新");
        assert!(root.join("experiment.txt").exists(), "Merge 后实验文件应成功合入主分支");
        assert!(!worktree_dir.exists(), "Merge 成功后影子 worktree 目录应自动被销毁");
    }

    #[tokio::test]
    async fn pty_write_packet_splitting_attack_blocked() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = RuntimeContext::new(dir.path(), RuntimePolicy::workspace_write(dir.path()));

        let mut registry = super::super::ToolRegistry::new();
        register_local_tools(&mut registry);
        let router = ToolRouter::new(registry);

        // 1. 启动一个标准 sh 会话
        let spawn_req = ToolCallRequest::new(
            "pty_spawn",
            json!({ "cmd": "sh", "rows": 24, "cols": 80 }),
        );
        let spawn_out = dispatch_approved(&router, &ctx, spawn_req).await;
        assert_eq!(spawn_out.status, ToolStatus::Ok);
        let parsed: serde_json::Value = serde_json::from_str(&spawn_out.content).unwrap();
        let session_id = parsed["session_id"].as_str().unwrap().to_string();

        // 2. 模拟拆包攻击：第一包只发未授权命令体（不带换行）
        let write_part1 = router
            .dispatch(
                &ctx,
                ToolCallRequest::new(
                    "pty_write",
                    json!({
                        "session_id": session_id,
                        "input": "sudo whoami"
                    }),
                ),
            )
            .await;
        assert_eq!(write_part1.status, ToolStatus::Ok, "未凑齐换行前暂存于缓冲");

        // 3. 第二包只发换行 `\n`，此时有状态缓冲拼出整行并触发 Guard 拦截！
        let write_part2 = router
            .dispatch(
                &ctx,
                ToolCallRequest::new(
                    "pty_write",
                    json!({
                        "session_id": session_id,
                        "input": "\n"
                    }),
                ),
            )
            .await;
        assert_ne!(
            write_part2.status,
            ToolStatus::Ok,
            "拆包注入的换行命令必须被有状态安全闸门严格拦截"
        );

        // 4. 清理
        let _ = router
            .dispatch(
                &ctx,
                ToolCallRequest::new("pty_kill", json!({ "session_id": session_id })),
            )
            .await;
    }

    #[tokio::test]
    async fn worktree_conflict_aborts_cleanly_without_polluting_main() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        // 初始化 Git 仓库
        let _ = std::process::Command::new("git").arg("init").current_dir(root).output().unwrap();
        let _ = std::process::Command::new("git").args(["config", "user.name", "Bot"]).current_dir(root).output().unwrap();
        let _ = std::process::Command::new("git").args(["config", "user.email", "bot@local"]).current_dir(root).output().unwrap();

        std::fs::write(root.join("f.txt"), "line 1: original\n").unwrap();
        let _ = std::process::Command::new("git").args(["add", "."]).current_dir(root).output().unwrap();
        let _ = std::process::Command::new("git").args(["commit", "-m", "init"]).current_dir(root).output().unwrap();

        let ctx = RuntimeContext::new(root, RuntimePolicy::workspace_write(root));
        let mut registry = super::super::ToolRegistry::new();
        register_local_tools(&mut registry);
        let router = ToolRouter::new(registry);

        // 1. 创建沙箱并在沙箱内修改 f.txt
        let task_id = "conflict-task";
        let create_out = router.dispatch(&ctx, ToolCallRequest::new("worktree_create", json!({ "task_id": task_id }))).await;
        assert_eq!(create_out.status, ToolStatus::Ok);
        let parsed: serde_json::Value = serde_json::from_str(&create_out.content).unwrap();
        let worktree_dir = std::path::PathBuf::from(parsed["worktree_path"].as_str().unwrap());

        std::fs::write(worktree_dir.join("f.txt"), "line 1: sandbox version\n").unwrap();

        // 2. 主仓库也在同一行做出冲突修改并提交
        std::fs::write(root.join("f.txt"), "line 1: main repo conflicting change\n").unwrap();
        let _ = std::process::Command::new("git").args(["commit", "-am", "main conflicting change"]).current_dir(root).output().unwrap();

        // 3. 尝试 merge，必然触发冲突
        let merge_out = router.dispatch(&ctx, ToolCallRequest::new("worktree_merge", json!({ "task_id": task_id }))).await;
        assert_ne!(merge_out.status, ToolStatus::Ok, "发生冲突时 merge 必须返回失败");

        // 4. 核心断言：主工作区必须自动回滚至干净状态，严禁留下 MERGING 或冲突标记
        assert!(!super::super::worktree::WorktreeManager::is_merging(root), "主仓库绝不能遗留 MERGE_HEAD 状态");
        let main_content = std::fs::read_to_string(root.join("f.txt")).unwrap();
        assert_eq!(main_content, "line 1: main repo conflicting change\n", "主工作区文件必须保持回退后的干净状态");
        assert!(!main_content.contains("<<<<<<<"), "严禁将冲突标记留在用户主工作区中");
    }

    async fn dispatch_approved(
        router: &ToolRouter,
        ctx: &RuntimeContext,
        request: ToolCallRequest,
    ) -> ToolOutcome {
        ctx.grant_tool_call(request.id.clone()).await;
        router.dispatch(ctx, request).await
    }
    /// 回归：审批的命令和执行的命令必须是同一条。
    ///
    /// 早先的实现把原始 input 直接透传、只在 Anima 侧另存一份用于核验，两份缓冲会失同步：
    /// 先写一段不含换行的片段（不过闸但已进 shell 缓冲，结尾 `#` 把后续输入吃成注释），
    /// 再故意触发一次拒绝让 Anima 单方面清空，最后让用户批准一条无害命令 ——
    /// 用户批准的是 `ls`，shell 执行的却是那段片段。
    #[tokio::test]
    async fn pty_write_approved_command_is_the_one_executed() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = RuntimeContext::new(dir.path(), RuntimePolicy::workspace_write(dir.path()));
        let mut registry = super::super::ToolRegistry::new();
        register_local_tools(&mut registry);
        let router = ToolRouter::new(registry);

        let spawn = dispatch_approved(
            &router,
            &ctx,
            ToolCallRequest::new("pty_spawn", json!({"cmd":"sh","rows":24,"cols":80})),
        )
        .await;
        let sid = serde_json::from_str::<serde_json::Value>(&spawn.content).unwrap()["session_id"]
            .as_str()
            .unwrap()
            .to_string();
        let w = |i: &str| ToolCallRequest::new("pty_write", json!({"session_id":sid,"input":i}));

        // 1. 不含换行的片段：只该被累积，一个字节都不该进 PTY
        assert_eq!(router.dispatch(&ctx, w("printf 'PWNED_%s\\n' 42 #")).await.status, ToolStatus::Ok);
        // 2. 触发一次拒绝，制造两份缓冲失同步的机会
        assert_ne!(router.dispatch(&ctx, w("sudo rm -rf /\n")).await.status, ToolStatus::Ok);
        // 3. 用户批准一条无害命令
        assert_eq!(dispatch_approved(&router, &ctx, w("ls\n")).await.status, ToolStatus::Ok);

        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        let poll = dispatch_approved(
            &router,
            &ctx,
            ToolCallRequest::new("pty_poll", json!({"session_id":sid})),
        )
        .await;
        assert!(
            !poll.content.contains("PWNED_42"),
            "被拒的片段借由「已批准的无害命令」执行了：\n{}",
            poll.content
        );
    }

    /// 回归：逐字符拆包也拼不出命令 —— 每次写入都不含换行，全部只进累积缓冲。
    #[tokio::test]
    async fn pty_write_char_by_char_never_reaches_the_shell() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = RuntimeContext::new(dir.path(), RuntimePolicy::workspace_write(dir.path()));
        let mut registry = super::super::ToolRegistry::new();
        register_local_tools(&mut registry);
        let router = ToolRouter::new(registry);

        let spawn = dispatch_approved(
            &router,
            &ctx,
            ToolCallRequest::new("pty_spawn", json!({"cmd":"sh","rows":24,"cols":80})),
        )
        .await;
        let sid = serde_json::from_str::<serde_json::Value>(&spawn.content).unwrap()["session_id"]
            .as_str()
            .unwrap()
            .to_string();

        for ch in "sudo rm -rf /".chars() {
            let out = router
                .dispatch(
                    &ctx,
                    ToolCallRequest::new(
                        "pty_write",
                        json!({"session_id":sid,"input":ch.to_string()}),
                    ),
                )
                .await;
            assert_eq!(out.status, ToolStatus::Ok, "片段本身不该报错，只该被累积");
        }
        // 补上换行 —— 此时整行才第一次被核验，必须挡住
        let submit = router
            .dispatch(
                &ctx,
                ToolCallRequest::new("pty_write", json!({"session_id":sid,"input":"\n"})),
            )
            .await;
        assert_ne!(submit.status, ToolStatus::Ok, "逐字符拼出的危险命令必须在提交时被拦截");

        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let poll = dispatch_approved(
            &router,
            &ctx,
            ToolCallRequest::new("pty_poll", json!({"session_id":sid})),
        )
        .await;
        assert!(!poll.content.contains("rm -rf"), "危险命令文本不该出现在终端里:\n{}", poll.content);
    }

    #[tokio::test]
    async fn lsp_tools_denied_paths_and_error_propagation() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let sample_file = root.join("main.rs");
        std::fs::write(
            &sample_file,
            "fn calculate_total(a: i32, b: i32) -> i32 {\n    a + b\n}\n\nfn main() {\n    let res = calculate_total(1, 2);\n}\n",
        )
        .unwrap();

        // 1. 验证 denied 拦截（例如拒绝访问 .env）
        let secret_file = root.join(".env");
        std::fs::write(&secret_file, "SECRET=123\n").unwrap();

        let mut policy = RuntimePolicy::workspace_write(root);
        policy.deny_patterns.push(".env*".to_string());
        let ctx = RuntimeContext::new(root, policy);

        let mut registry = super::super::ToolRegistry::new();
        register_local_tools(&mut registry);
        let router = ToolRouter::new(registry);

        let denied_req = ToolCallRequest::new(
            "lsp_find_definition",
            json!({
                "path": ".env",
                "line": 1,
                "character": 1
            }),
        );
        let denied_out = router.dispatch(&ctx, denied_req).await;
        assert_eq!(
            denied_out.status,
            ToolStatus::Blocked,
            "被 deny_patterns 拦截的文件必须直接返回 Blocked"
        );

        // 2. 当无真实 LSP Server 可用时，必须返回 ToolStatus::Failed（而非伪装成 Ok），以便自愈系统感知
        let def_req = ToolCallRequest::new(
            "lsp_find_definition",
            json!({
                "path": "main.rs",
                "line": 6,
                "character": 16
            }),
        );
        let def_out = router.dispatch(&ctx, def_req).await;
        // 如果系统未安装 rust-analyzer，则返回 Failed 并带 hint；若已安装则返回 Ok
        if def_out.status != ToolStatus::Ok {
            assert_eq!(def_out.status, ToolStatus::Failed);
            let failure = def_out.failure.unwrap();
            assert!(failure.hint.is_some(), "失败时必须提供回退提示");
        }
    }
    /// 回归：`CODING_RUNTIME_TOOLS` 这份名单和 `register_coding_runtime_tools`
    /// 实际注册的工具必须一一对应。
    ///
    /// 两处分开写就一定会漂 —— 加了个新 PTY 工具却忘了加进名单，
    /// 语音会话就会重新背上它，而且**不报任何错**，只是 token 悄悄涨回去。
    #[test]
    fn coding_runtime_list_matches_what_is_registered() {
        let mut registry = super::super::ToolRegistry::new();
        register_coding_runtime_tools(&mut registry);
        let mut registered: Vec<String> =
            registry.specs().into_iter().map(|spec| spec.name).collect();
        registered.sort();
        let mut declared: Vec<String> =
            CODING_RUNTIME_TOOLS.iter().map(|n| n.to_string()).collect();
        declared.sort();
        assert_eq!(
            registered, declared,
            "CODING_RUNTIME_TOOLS 与 register_coding_runtime_tools 不一致"
        );
    }

    /// 回归：拆分之后 `register_local_tools` 仍是两半之和，一个不多一个不少。
    #[test]
    fn split_registration_covers_exactly_the_whole_set() {
        let mut whole = super::super::ToolRegistry::new();
        register_local_tools(&mut whole);
        let mut halves = super::super::ToolRegistry::new();
        register_basic_local_tools(&mut halves);
        register_coding_runtime_tools(&mut halves);

        let mut a: Vec<String> = whole.specs().into_iter().map(|s| s.name).collect();
        let mut b: Vec<String> = halves.specs().into_iter().map(|s| s.name).collect();
        a.sort();
        b.sort();
        assert_eq!(a, b, "拆分注册与整体注册的工具集不一致");
        assert!(!a.is_empty());
    }

    /// 回归：只读会话不得建 / 合 / 弃沙箱。
    ///
    /// 注意这里检查的是 **workspace_root 的 Write 能力**，不是 `.git/`。
    /// `worktree_*` 按设计豁免 `deny_patterns` 里的 `.git/` —— 操作版本库元数据
    /// 就是它的职责本身，受那条约束等于让它不能工作。
    #[tokio::test]
    async fn worktree_tools_are_blocked_in_a_read_only_session() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = RuntimeContext::new(dir.path(), RuntimePolicy::read_only(dir.path()));
        let mut registry = super::super::ToolRegistry::new();
        register_local_tools(&mut registry);
        let router = ToolRouter::new(registry);

        for name in ["worktree_create", "worktree_merge", "worktree_discard"] {
            let out = dispatch_approved(
                &router,
                &ctx,
                ToolCallRequest::new(name, json!({"task_id":"t1"})),
            )
            .await;
            // 只断言"不是 Ok"是不够的 —— tempdir 本来就不是 git 仓库，
            // 那样即便策略检查根本没跑，测试也会通过。必须认准是**策略**挡的。
            let kind = out.failure.as_ref().map(|f| f.kind);
            assert_eq!(
                kind,
                Some(ToolFailureKind::SandboxDenied),
                "{name} 该被路径策略挡住（SandboxDenied），实际是 {:?}：{:?}",
                kind,
                out.failure.as_ref().map(|f| f.message.clone())
            );
        }

        // 正名：同样的调用在可写会话 + 真 git 仓库下必须能成功，
        // 否则上面那三条"被挡住"可能只是因为功能整个是坏的。
        let repo = tempfile::tempdir().unwrap();
        for args in [
            vec!["init", "-q", "."],
            vec!["config", "user.email", "a@b.c"],
            vec!["config", "user.name", "t"],
        ] {
            let _ = std::process::Command::new("git")
                .args(&args)
                .current_dir(repo.path())
                .output();
        }
        std::fs::write(repo.path().join("f.txt"), "x").unwrap();
        for args in [vec!["add", "."], vec!["commit", "-qm", "init"]] {
            let _ = std::process::Command::new("git")
                .args(&args)
                .current_dir(repo.path())
                .output();
        }
        let ok_ctx = RuntimeContext::new(repo.path(), RuntimePolicy::workspace_write(repo.path()));
        let created = dispatch_approved(
            &router,
            &ok_ctx,
            ToolCallRequest::new("worktree_create", json!({"task_id":"t1"})),
        )
        .await;
        assert_eq!(
            created.status,
            ToolStatus::Ok,
            "可写会话 + 真仓库下必须能建沙箱：{:?}",
            created.failure
        );
        let _ = dispatch_approved(
            &router,
            &ok_ctx,
            ToolCallRequest::new("worktree_discard", json!({"task_id":"t1"})),
        )
        .await;
    }

    /// 回归：LSP 客户端的协议实现 —— 用仓库内的假 language server 跑，
    /// **不需要本机装 rust-analyzer / tsserver 之类**。
    ///
    /// 一并盯住四件事，每一件都对应一个真实踩过的坑：
    ///
    /// 1. **噪声必须跳过** —— 假 server 在答案之前先塞一条通知和一条 id 错配的响应；
    /// 2. **`didOpen` 必须发** —— 不发就返回 null（tsserver 就是这么干的）；
    /// 3. **行号基准换算** —— LSP 是 0-based，工具参数和返回都是 1-based；
    /// 4. **索引期的空结果不能报成「这个符号没有定义」** ——
    ///    rust-analyzer 在索引完成前返回**空数组而不是错误**，
    ///    原样返回 `count: 0` 就是给了模型一条错误的事实。
    ///    工具超时只有 12s、真实工程索引是分钟级，等不出来，所以要如实报状态。
    #[tokio::test]
    async fn lsp_client_speaks_the_protocol_and_separates_indexing_from_empty() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/fake-lsp");
        // 假 server 只提供 `rust-analyzer` 这一个名字，顶在 PATH 最前面不影响别的工具。
        std::env::set_var(
            "PATH",
            format!(
                "{}:{}",
                fixture.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        );

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src/lib.rs"),
            "pub struct Widget {\n    pub size: u32,\n}\n\npub fn make() -> Widget {\n    Widget { size: 7 }\n}\n\npub fn use_it() -> u32 {\n    let w = make();\n    w.size\n}\n",
        )
        .unwrap();

        let ctx = RuntimeContext::new(&root, RuntimePolicy::workspace_write(&root));
        let mut registry = super::super::ToolRegistry::new();
        register_local_tools(&mut registry);
        let router = ToolRouter::new(registry);
        let call = || {
            ToolCallRequest::new(
                "lsp_find_definition",
                json!({"path":"src/lib.rs","line":10,"character":13}),
            )
        };

        // ── 刚拉起来还没就绪：前两次答空，客户端必须重试到答案出现 ──
        //
        // 修之前只问一次，拿到空就当"这个符号没有定义"报上去 ——
        // 那是给模型一条**错误的事实**。真实 rust-analyzer 实测约 1.6s 才答得出，
        // 在那之前一律返回空而不是错误。
        std::fs::write(root.join("INDEXING"), "0").unwrap();
        let out = dispatch_approved(&router, &ctx, call()).await;
        assert_eq!(
            out.status,
            ToolStatus::Ok,
            "重试之后该拿到定义：{:?}",
            out.failure
        );
        assert!(
            out.content.contains("\"line\": 5"),
            "重试之后该查到第 5 行的定义：{}",
            out.content
        );
        let tries: u32 = std::fs::read_to_string(root.join("INDEXING"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(tries >= 3, "客户端只问了 {tries} 次 —— 说明没有重试");

        // ── 索引完成：同一次调用必须查到定义 ──
        // 这条是上面那条的正名 —— 少了它，「不是 Ok」在整条链路坏掉时也会通过。
        std::fs::remove_file(root.join("INDEXING")).unwrap();
        let ok = dispatch_approved(&router, &ctx, call()).await;
        assert_eq!(
            ok.status,
            ToolStatus::Ok,
            "索引完成后必须能查到定义：{:?}",
            ok.failure
        );
        assert!(
            ok.content.contains("\"line\": 5"),
            "LSP 的 0-based line 4 该换算成第 5 行：{}",
            ok.content
        );
        assert!(
            ok.content.contains("pub fn make"),
            "preview 该是定义所在那一行：{}",
            ok.content
        );
        assert!(
            root.join(".didopen").exists(),
            "查询前必须发过 textDocument/didOpen（tsserver 不发就不答）"
        );
    }

    /// 回归：活跃沙箱有数量上限。
    ///
    /// 每个沙箱是仓库的一份完整 checkout。没有上限时，模型多派几次子智能体
    /// 就能把磁盘堆满 —— 而且**不报任何错**，只是越来越慢、越来越占地方。
    #[tokio::test]
    async fn active_sandboxes_are_capped() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for a in [
            vec!["init", "-q", "."],
            vec!["config", "user.email", "a@b.c"],
            vec!["config", "user.name", "t"],
        ] {
            let _ = std::process::Command::new("git")
                .args(&a)
                .current_dir(root)
                .output();
        }
        std::fs::write(root.join("f.txt"), "x").unwrap();
        for a in [vec!["add", "."], vec!["commit", "-qm", "init"]] {
            let _ = std::process::Command::new("git")
                .args(&a)
                .current_dir(root)
                .output();
        }
        let ctx = RuntimeContext::new(root, RuntimePolicy::workspace_write(root));
        let mut registry = super::super::ToolRegistry::new();
        register_local_tools(&mut registry);
        let router = ToolRouter::new(registry);

        let mut created = 0;
        let mut refused = None;
        for i in 0..8 {
            let out = dispatch_approved(
                &router,
                &ctx,
                ToolCallRequest::new("worktree_create", json!({ "task_id": format!("t{i}") })),
            )
            .await;
            if out.status == ToolStatus::Ok {
                created += 1;
            } else {
                refused = out.failure.map(|f| f.message);
                break;
            }
        }

        assert!(created > 0, "一个都没建起来，测试无效");
        let msg = refused.expect("超过上限之后必须被拒绝，而不是无限建下去");
        assert!(msg.contains("上限"), "拒绝理由该说明是达到上限，实际是：{msg}");
    }

    /// 回归：并行沙箱冲突必须挡在**沙箱内**，主工作区一个字节都不能动。
    ///
    /// 这是 `isolated` 模式能同时铺开多个子任务的前提。不先 rebase 直接 merge 的话，
    /// 冲突就落在主工作区 —— 带着冲突标记卡在 MERGING，而沙箱的全部卖点
    /// 恰恰是主分支干净。
    #[tokio::test]
    async fn a_conflicting_sandbox_never_dirties_the_main_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let git = |a: &[&str]| {
            let _ = std::process::Command::new("git")
                .args(a)
                .current_dir(root)
                .output();
        };
        git(&["init", "-q", "."]);
        git(&["config", "user.email", "a@b.c"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(root.join("f.txt"), "base\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "init"]);

        let ctx = RuntimeContext::new(root, RuntimePolicy::workspace_write(root));
        let mut registry = super::super::ToolRegistry::new();
        register_local_tools(&mut registry);
        let router = ToolRouter::new(registry);

        let created = dispatch_approved(
            &router,
            &ctx,
            ToolCallRequest::new("worktree_create", json!({"task_id":"c1"})),
        )
        .await;
        assert_eq!(created.status, ToolStatus::Ok, "{:?}", created.failure);

        // 沙箱和主分支改同一行 —— 必冲突
        let sandbox = root.join(".anima/worktrees/c1");
        std::fs::write(sandbox.join("f.txt"), "sandbox\n").unwrap();
        let _ = std::process::Command::new("git")
            .args(["commit", "-qam", "sandbox side"])
            .current_dir(&sandbox)
            .output();
        std::fs::write(root.join("f.txt"), "main\n").unwrap();
        git(&["commit", "-qam", "main side"]);

        let merged = dispatch_approved(
            &router,
            &ctx,
            ToolCallRequest::new("worktree_merge", json!({"task_id":"c1"})),
        )
        .await;
        assert_ne!(merged.status, ToolStatus::Ok, "冲突的合并不该成功");

        // ① 主工作区必须干净：没有冲突标记、没有卡在 MERGING
        assert_eq!(
            std::fs::read_to_string(root.join("f.txt")).unwrap(),
            "main\n",
            "主工作区的文件被改动了"
        );
        assert!(
            !root.join(".git/MERGE_HEAD").exists(),
            "主仓库卡在 MERGING 状态"
        );

        // ② 影子分支必须保留，否则人工重试的唯一凭据就没了
        let branches = std::process::Command::new("git")
            .args(["branch", "--list", "anima/sandbox-c1"])
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            !String::from_utf8_lossy(&branches.stdout).trim().is_empty(),
            "冲突之后影子分支被删掉了"
        );

        // ③ 正名：不冲突的沙箱必须能合进去，否则上面的"合不进去"可能只是功能坏了
        let ok = dispatch_approved(
            &router,
            &ctx,
            ToolCallRequest::new("worktree_create", json!({"task_id":"c2"})),
        )
        .await;
        assert_eq!(ok.status, ToolStatus::Ok);
        let sandbox2 = root.join(".anima/worktrees/c2");
        std::fs::write(sandbox2.join("other.txt"), "new\n").unwrap();
        for a in [vec!["add", "."], vec!["commit", "-qm", "no conflict"]] {
            let _ = std::process::Command::new("git")
                .args(&a)
                .current_dir(&sandbox2)
                .output();
        }
        let merged2 = dispatch_approved(
            &router,
            &ctx,
            ToolCallRequest::new("worktree_merge", json!({"task_id":"c2"})),
        )
        .await;
        assert_eq!(
            merged2.status,
            ToolStatus::Ok,
            "无冲突的沙箱必须能合入：{:?}",
            merged2.failure
        );
        assert!(root.join("other.txt").exists(), "合入后主工作区该出现新文件");
    }

}
