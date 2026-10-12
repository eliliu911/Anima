//! Code Outline and AST Symbol Extractor Tool
//!
//! Provides a compact overview of code structures (functions, structs, classes,
//! interfaces, types, and markdown headings) with line numbers.
//!
//! Designed specifically for agentic coding: allows local & cloud models to
//! understand the architecture of large files (1,000 ~ 5,000+ lines) using only
//! ~50 lines (200 tokens) before selectively reading specific methods.

use std::path::Path;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::json;

use super::{
    FileCapability, RuntimeContext, ToolCallRequest, ToolFailure, ToolFailureKind, ToolHandler,
    ToolKind, ToolOutcome, ToolSpec,
};
use crate::agent_runtime::local_tools::{invalid_params, resolve_workspace_path, string_arg};
use crate::mcp::Operation;

pub struct CodeOutlineTool;

#[async_trait]
impl ToolHandler for CodeOutlineTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "code_outline",
            "Extract structural outline and symbol definitions (functions, classes, structs, interfaces, types, and markdown headers) with start/end line numbers. Crucial for understanding large files before targeted slicing.",
            ToolKind::FileSystem,
            Operation::Read,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Relative or absolute file path" },
                "filter": { "type": "string", "description": "Optional symbol name filter keyword" },
                "max_symbols": { "type": "integer", "description": "Maximum number of symbols to return (default 80)" }
            },
            "required": ["path"]
        }))
        .with_timeout(Duration::from_secs(5))
    }

    async fn execute(&self, ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let started = Instant::now();
        let Some(path_str) = string_arg(&request.arguments, "path") else {
            return invalid_params(&request, "缺少 path 参数");
        };

        let filter = string_arg(&request.arguments, "filter");
        let max_symbols = request
            .arguments
            .get("max_symbols")
            .and_then(|v| v.as_u64())
            .unwrap_or(80) as usize;

        let path = resolve_workspace_path(ctx, path_str);
        let verdict = ctx.policy.evaluate_path(&path, FileCapability::Read);
        if !verdict.allowed() {
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(ToolFailureKind::SandboxDenied, verdict.reason().to_string()),
                started.elapsed(),
            );
        }

        match std::fs::read_to_string(&path) {
            Ok(content) => {
                let outline = extract_outline_with_opts(&path, &content, filter, max_symbols);
                ToolOutcome::ok(
                    &request,
                    outline,
                    vec![path.to_string_lossy().to_string()],
                    started.elapsed(),
                )
            }
            Err(err) => ToolOutcome::failed(
                &request,
                ToolFailure::new(
                    ToolFailureKind::Execution,
                    format!("读取文件大纲失败：{err}"),
                ),
                started.elapsed(),
            ),
        }
    }
}

/// 针对不同语言提取轻量符号大纲（便捷入口）
pub fn extract_outline(path: &Path, content: &str) -> String {
    extract_outline_with_opts(path, content, None, 80)
}

/// 针对不同语言提取轻量符号大纲（带过滤与上限参数）
pub fn extract_outline_with_opts(
    path: &Path,
    content: &str,
    filter: Option<&str>,
    max_symbols: usize,
) -> String {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();

    let mut symbols: Vec<String> = match ext.as_str() {
        "rs" => extract_rust_symbols(&lines),
        "ts" | "tsx" | "js" | "jsx" => extract_ts_js_symbols(&lines),
        "py" => extract_python_symbols(&lines),
        "go" => extract_go_symbols(&lines),
        "md" | "markdown" => extract_markdown_headings(&lines),
        "json" | "toml" | "yaml" | "yml" => extract_config_keys(&lines, &ext),
        _ => extract_generic_outline(&lines),
    };

    if let Some(kw) = filter {
        let kw_lower = kw.to_lowercase();
        symbols.retain(|s| s.to_lowercase().contains(&kw_lower));
    }

    let file_name = path.file_name().and_then(|f| f.to_str()).unwrap_or("file");

    if symbols.is_empty() {
        return format!(
            "文件: {} (共 {} 行)\n未检测到顶层函数或类定义。可使用 view_file(start_line, end_line) 读取代码。",
            file_name, total_lines
        );
    }

    let total_matched = symbols.len();
    let truncated_note = if total_matched > max_symbols {
        let omitted = total_matched - max_symbols;
        symbols.truncate(max_symbols);
        format!(
            "\n  ... (另有 {} 个符号未列出，可使用 filter 参数按名称过滤)",
            omitted
        )
    } else {
        String::new()
    };

    format!(
        "文件大纲: {} (共 {} 行，显示 {}/{} 个符号)\n{}{}\n\n提示: 请使用 view_file(path: \"{}\", start_line: N, end_line: M) 精确定向读取所需代码块。",
        file_name,
        total_lines,
        symbols.len(),
        total_matched,
        symbols.join("\n"),
        truncated_note,
        path.to_string_lossy()
    )
}

/// 辅助函数：从起始行开始，通过简单花括号计数估算代码块的结束行号
fn find_block_end_line(lines: &[&str], start_idx: usize) -> usize {
    let mut brace_count = 0;
    let mut started = false;

    for (offset, line) in lines[start_idx..].iter().enumerate() {
        let trimmed = line.trim();
        // 忽略单行注释
        if trimmed.starts_with("//") {
            continue;
        }

        for ch in line.chars() {
            if ch == '{' {
                brace_count += 1;
                started = true;
            } else if ch == '}' {
                brace_count -= 1;
            }
        }

        if started && brace_count <= 0 {
            return start_idx + offset + 1;
        }
    }

    start_idx + 1
}

fn extract_rust_symbols(lines: &[&str]) -> Vec<String> {
    let mut symbols = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let line_no = idx + 1;

        if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
            continue;
        }

        if trimmed.starts_with("pub fn ")
            || trimmed.starts_with("pub async fn ")
            || trimmed.starts_with("fn ")
            || trimmed.starts_with("async fn ")
            || trimmed.starts_with("pub(crate) fn ")
            || trimmed.starts_with("pub(crate) async fn ")
        {
            let end_line = find_block_end_line(lines, idx);
            let sig = trimmed.split('{').next().unwrap_or(trimmed).trim();
            let line_range = if end_line > line_no {
                format!("L{}-L{}", line_no, end_line)
            } else {
                format!("L{}", line_no)
            };
            symbols.push(format!("  {:<12} [fn] {}", line_range, sig));
        } else if trimmed.starts_with("pub struct ") || trimmed.starts_with("struct ") {
            let end_line = find_block_end_line(lines, idx);
            let sig = trimmed.split('{').next().unwrap_or(trimmed).trim();
            let line_range = if end_line > line_no {
                format!("L{}-L{}", line_no, end_line)
            } else {
                format!("L{}", line_no)
            };
            symbols.push(format!("  {:<12} [struct] {}", line_range, sig));
        } else if trimmed.starts_with("pub enum ") || trimmed.starts_with("enum ") {
            let end_line = find_block_end_line(lines, idx);
            let sig = trimmed.split('{').next().unwrap_or(trimmed).trim();
            let line_range = if end_line > line_no {
                format!("L{}-L{}", line_no, end_line)
            } else {
                format!("L{}", line_no)
            };
            symbols.push(format!("  {:<12} [enum] {}", line_range, sig));
        } else if trimmed.starts_with("pub trait ") || trimmed.starts_with("trait ") {
            let end_line = find_block_end_line(lines, idx);
            let sig = trimmed.split('{').next().unwrap_or(trimmed).trim();
            let line_range = if end_line > line_no {
                format!("L{}-L{}", line_no, end_line)
            } else {
                format!("L{}", line_no)
            };
            symbols.push(format!("  {:<12} [trait] {}", line_range, sig));
        } else if trimmed.starts_with("impl ") || trimmed.starts_with("impl<") {
            let end_line = find_block_end_line(lines, idx);
            let sig = trimmed.split('{').next().unwrap_or(trimmed).trim();
            let line_range = if end_line > line_no {
                format!("L{}-L{}", line_no, end_line)
            } else {
                format!("L{}", line_no)
            };
            symbols.push(format!("  {:<12} [impl] {}", line_range, sig));
        } else if trimmed.starts_with("pub type ") || trimmed.starts_with("type ") {
            let sig = trimmed.split('=').next().unwrap_or(trimmed).trim();
            symbols.push(format!("  L{:<11} [type] {}", line_no, sig));
        } else if trimmed.starts_with("pub mod ") || trimmed.starts_with("mod ") {
            symbols.push(format!(
                "  L{:<11} [mod] {}",
                line_no,
                trimmed.trim_end_matches(';')
            ));
        }
    }
    symbols
}

fn extract_ts_js_symbols(lines: &[&str]) -> Vec<String> {
    let mut symbols = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let line_no = idx + 1;

        if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
            continue;
        }

        // Functions & React Components
        if trimmed.starts_with("export function ")
            || trimmed.starts_with("export async function ")
            || trimmed.starts_with("function ")
            || trimmed.starts_with("async function ")
            || (trimmed.starts_with("export const ")
                && (trimmed.contains(" = (")
                    || trimmed.contains(" = async (")
                    || trimmed.contains(" = function")))
            || (trimmed.starts_with("const ")
                && (trimmed.contains(" = (")
                    || trimmed.contains(" = async (")
                    || trimmed.contains(" = function")))
        {
            let end_line = find_block_end_line(lines, idx);
            let sig = trimmed.split('{').next().unwrap_or(trimmed).trim();
            let line_range = if end_line > line_no {
                format!("L{}-L{}", line_no, end_line)
            } else {
                format!("L{}", line_no)
            };
            symbols.push(format!("  {:<12} [fn] {}", line_range, sig));
        } else if trimmed.starts_with("export class ") || trimmed.starts_with("class ") {
            let end_line = find_block_end_line(lines, idx);
            let sig = trimmed.split('{').next().unwrap_or(trimmed).trim();
            let line_range = if end_line > line_no {
                format!("L{}-L{}", line_no, end_line)
            } else {
                format!("L{}", line_no)
            };
            symbols.push(format!("  {:<12} [class] {}", line_range, sig));
        } else if trimmed.starts_with("export interface ") || trimmed.starts_with("interface ") {
            let end_line = find_block_end_line(lines, idx);
            let sig = trimmed.split('{').next().unwrap_or(trimmed).trim();
            let line_range = if end_line > line_no {
                format!("L{}-L{}", line_no, end_line)
            } else {
                format!("L{}", line_no)
            };
            symbols.push(format!("  {:<12} [interface] {}", line_range, sig));
        } else if trimmed.starts_with("export type ") || trimmed.starts_with("type ") {
            let sig = trimmed.split('=').next().unwrap_or(trimmed).trim();
            symbols.push(format!("  L{:<11} [type] {}", line_no, sig));
        } else if trimmed.starts_with("export default function")
            || trimmed.starts_with("export default class")
        {
            let end_line = find_block_end_line(lines, idx);
            let sig = trimmed.split('{').next().unwrap_or(trimmed).trim();
            let line_range = if end_line > line_no {
                format!("L{}-L{}", line_no, end_line)
            } else {
                format!("L{}", line_no)
            };
            symbols.push(format!("  {:<12} [default] {}", line_range, sig));
        }
    }
    symbols
}

fn extract_python_symbols(lines: &[&str]) -> Vec<String> {
    let mut symbols = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        let line_no = idx + 1;
        let trimmed = line.trim();
        if trimmed.starts_with("def ") || trimmed.starts_with("async def ") {
            let sig = trimmed.trim_end_matches(':').trim();
            if line.starts_with(' ') || line.starts_with('\t') {
                symbols.push(format!("  L{:<4}    [method] {}", line_no, sig));
            } else {
                symbols.push(format!("  L{:<4}  [def] {}", line_no, sig));
            }
        } else if trimmed.starts_with("class ") {
            let sig = trimmed.trim_end_matches(':').trim();
            symbols.push(format!("  L{:<4}  [class] {}", line_no, sig));
        }
    }
    symbols
}

fn extract_go_symbols(lines: &[&str]) -> Vec<String> {
    let mut symbols = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let line_no = idx + 1;

        if trimmed.starts_with("func ") {
            let sig = trimmed.split('{').next().unwrap_or(trimmed).trim();
            symbols.push(format!("  L{:<4}  [func] {}", line_no, sig));
        } else if trimmed.starts_with("type ")
            && (trimmed.contains("struct") || trimmed.contains("interface"))
        {
            let sig = trimmed.split('{').next().unwrap_or(trimmed).trim();
            symbols.push(format!("  L{:<4}  [type] {}", line_no, sig));
        }
    }
    symbols
}

fn extract_markdown_headings(lines: &[&str]) -> Vec<String> {
    let mut headings = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let line_no = idx + 1;
        if trimmed.starts_with('#') {
            headings.push(format!("  L{:<4}  {}", line_no, trimmed));
        }
    }
    headings
}

fn extract_config_keys(lines: &[&str], ext: &str) -> Vec<String> {
    let mut keys = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let line_no = idx + 1;
        if ext == "toml" && trimmed.starts_with('[') && trimmed.ends_with(']') {
            keys.push(format!("  L{:<4}  {}", line_no, trimmed));
        } else if (ext == "yaml" || ext == "yml")
            && !line.starts_with(' ')
            && trimmed.ends_with(':')
        {
            keys.push(format!("  L{:<4}  {}", line_no, trimmed));
        } else if ext == "json"
            && !line.starts_with("  ")
            && trimmed.starts_with('"')
            && trimmed.contains(':')
        {
            let key = trimmed.split(':').next().unwrap_or(trimmed);
            keys.push(format!("  L{:<4}  {}", line_no, key));
        }
    }
    keys
}

fn extract_generic_outline(lines: &[&str]) -> Vec<String> {
    let mut outlines = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let line_no = idx + 1;
        if !line.starts_with(' ')
            && !line.starts_with('\t')
            && !trimmed.is_empty()
            && trimmed.len() < 120
        {
            outlines.push(format!("  L{:<4}  {}", line_no, trimmed));
        }
    }
    outlines.truncate(40);
    outlines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_rust_outline() {
        let code = r#"
pub struct Engine {
    id: String,
}

impl Engine {
    pub fn start(&self) {}
}

pub async fn run_server() {}
"#;
        let outline = extract_outline(Path::new("src/main.rs"), code);
        assert!(outline.contains("[struct] pub struct Engine"));
        assert!(outline.contains("[impl] impl Engine"));
        assert!(outline.contains("[fn] pub async fn run_server()"));
    }

    #[test]
    fn test_extract_typescript_outline() {
        let code = r#"
export interface User {
    id: string;
}

export function getUser(): User {
    return { id: "1" };
}

export const fetchProfile = async () => {};
"#;
        let outline = extract_outline(Path::new("src/api.ts"), code);
        assert!(outline.contains("[interface] export interface User"));
        assert!(outline.contains("[fn] export function getUser()"));
        assert!(outline.contains("[fn] export const fetchProfile = async () =>"));
    }

    #[test]
    fn test_extract_python_outline() {
        let code = r#"
class VoiceModel:
    def __init__(self):
        pass

    async def generate_speech(self, text: str):
        pass

def standalone_helper():
    pass
"#;
        let outline = extract_outline(Path::new("scripts/model.py"), code);
        assert!(outline.contains("[class] class VoiceModel"));
        assert!(outline.contains("[method] def __init__(self)"));
        assert!(outline.contains("[method] async def generate_speech(self, text: str)"));
        assert!(outline.contains("[def] def standalone_helper()"));
    }

    #[test]
    fn test_extract_go_outline() {
        let code = r#"
type Server struct {
    port int
}

func NewServer() *Server {
    return &Server{}
}
"#;
        let outline = extract_outline(Path::new("main.go"), code);
        assert!(outline.contains("[type] type Server struct"));
        assert!(outline.contains("[func] func NewServer() *Server"));
    }

    #[test]
    fn test_extract_markdown_outline() {
        let doc = r#"
# Architecture

## Gateway

### Connectors
"#;
        let outline = extract_outline(Path::new("docs/ARCH.md"), doc);
        assert!(outline.contains("# Architecture"));
        assert!(outline.contains("## Gateway"));
        assert!(outline.contains("### Connectors"));
    }

    #[test]
    fn test_extract_config_outline() {
        let toml = r#"
[package]
name = "anima"

[dependencies]
serde = "1.0"
"#;
        let outline = extract_outline(Path::new("Cargo.toml"), toml);
        assert!(outline.contains("[package]"));
        assert!(outline.contains("[dependencies]"));
    }

    #[test]
    fn test_filter_and_max_symbols() {
        let code = r#"
pub fn fn_alpha() {}
pub fn fn_beta() {}
pub fn fn_gamma() {}
"#;
        // Filter test
        let filtered = extract_outline_with_opts(Path::new("lib.rs"), code, Some("beta"), 80);
        assert!(filtered.contains("fn_beta"));
        assert!(!filtered.contains("fn_alpha"));

        // Max symbols test
        let capped = extract_outline_with_opts(Path::new("lib.rs"), code, None, 2);
        assert!(capped.contains("显示 2/3 个符号"));
        assert!(capped.contains("另有 1 个符号未列出"));
    }
}
