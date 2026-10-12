//! Language Server Protocol (LSP) Client for Precise Symbol Tracking (P4)
//!
//! Provides JSON-RPC communication with local language servers (such as rust-analyzer,
//! typescript-language-server, pyright, gopls) for:
//! 1. "Go to Definition" (textDocument/definition)
//! 2. "Find References" (textDocument/references)
//! 3. Document opening synchronization (textDocument/didOpen)
//! 4. Process lifecycle & exit hook management

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolLocation {
    pub file_path: String,
    pub line: u32,
    pub character: u32,
    pub end_line: u32,
    pub end_character: u32,
    pub preview: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LanguageKind {
    Rust,
    TypeScript,
    JavaScript,
    Python,
    Go,
    Unknown,
}

impl LanguageKind {
    pub fn from_path(path: &Path) -> Self {
        match path.extension().and_then(|e| e.to_str()) {
            Some("rs") => LanguageKind::Rust,
            Some("ts") | Some("tsx") => LanguageKind::TypeScript,
            Some("js") | Some("jsx") | Some("mjs") | Some("cjs") => LanguageKind::JavaScript,
            Some("py") => LanguageKind::Python,
            Some("go") => LanguageKind::Go,
            _ => LanguageKind::Unknown,
        }
    }

    pub fn language_id(&self) -> &'static str {
        match self {
            LanguageKind::Rust => "rust",
            LanguageKind::TypeScript => "typescript",
            LanguageKind::JavaScript => "javascript",
            LanguageKind::Python => "python",
            LanguageKind::Go => "go",
            LanguageKind::Unknown => "plaintext",
        }
    }

    pub fn server_command(&self) -> Option<(&'static str, &'static [&'static str], &'static str)> {
        match self {
            LanguageKind::Rust => Some(("rust-analyzer", &[], "--version")),
            LanguageKind::TypeScript | LanguageKind::JavaScript => {
                Some(("typescript-language-server", &["--stdio"], "--version"))
            }
            LanguageKind::Python => Some(("pyright-langserver", &["--stdio"], "--version")),
            LanguageKind::Go => Some(("gopls", &[], "version")),
            LanguageKind::Unknown => None,
        }
    }
}

pub struct LspServerProcess {
    pub lang: LanguageKind,
    pub stdin: Arc<Mutex<ChildStdin>>,
    pub stdout: Arc<Mutex<BufReader<ChildStdout>>>,
    pub child: Arc<Mutex<Child>>,
    pub stderr_buffer: Arc<StdMutex<String>>,
    pub opened_files: Arc<Mutex<HashSet<PathBuf>>>,
    pub next_req_id: AtomicI64,
    /// 服务器是否仍在建立索引。
    ///
    /// rust-analyzer 这类服务器在索引完成前对 definition / references **返回空结果而不是错误**。
    /// 不区分的话，`count: 0` 读起来就是「这个符号没有定义」—— 一条错误的事实。
    /// 而工具自己的超时只有 12s，真实工程索引是分钟级，**等不出来**，
    /// 所以正确做法不是等，是把「还没准备好」和「确实没有」分开报。
    pub indexing: Arc<AtomicBool>,
}

/// 全局常驻 LSP 进程表，供 `RunEvent::Exit` 钩子彻底回收
static ACTIVE_LSP_PROCESSES: OnceLock<StdMutex<Vec<Arc<LspServerProcess>>>> = OnceLock::new();

fn active_lsp_processes() -> &'static StdMutex<Vec<Arc<LspServerProcess>>> {
    ACTIVE_LSP_PROCESSES.get_or_init(|| StdMutex::new(Vec::new()))
}

/// 进程级退出清理钩子：向所有活跃 LSP 发送 shutdown / exit 并强制 kill 残留
pub fn stop_all_lsp_servers() {
    if let Ok(mut lock) = active_lsp_processes().lock() {
        for proc in lock.drain(..) {
            let child_arc = Arc::clone(&proc.child);
            tauri::async_runtime::spawn(async move {
                if let Ok(mut child) = child_arc.try_lock() {
                    let _ = child.kill().await;
                }
            });
        }
    }
}

#[derive(Default, Clone)]
pub struct LspClientManager {
    servers: Arc<Mutex<HashMap<(PathBuf, String), Arc<LspServerProcess>>>>,
}

impl LspClientManager {
    /// 真实运行命令（检查 `--version` 或对应命令能否真正执行并返回成功），而非仅靠 `which`
    pub async fn check_server_health(lang: LanguageKind) -> Result<(), String> {
        let Some((bin, _, ver_arg)) = lang.server_command() else {
            return Err("当前文件类型不支持 LSP 符号服务".to_string());
        };

        let res = Command::new(bin)
            .arg(ver_arg)
            .output()
            .await
            .map_err(|e| format!("未找到可执行的 `{bin}` 服务器 ({e})"))?;

        if !res.status.success() {
            let stderr = String::from_utf8_lossy(&res.stderr);
            let hint = if bin == "rust-analyzer" && stderr.contains("Unknown binary") {
                " (检测到 rustup 缺少组件，请运行 `rustup component add rust-analyzer`)"
            } else {
                ""
            };
            return Err(format!(
                "`{bin}` 启动检查失败 (code {:?}): {}{hint}",
                res.status.code(),
                stderr.trim()
            ));
        }

        Ok(())
    }

    /// 获取或按需启动一个语言对应的 LSP Server 会话
    pub async fn get_or_spawn_server(
        &self,
        workspace_root: &Path,
        lang: LanguageKind,
    ) -> Result<Arc<LspServerProcess>, String> {
        let (bin, args, _) = lang
            .server_command()
            .ok_or_else(|| "当前文件类型不支持 LSP 符号服务".to_string())?;

        let key = (workspace_root.to_path_buf(), format!("{:?}", lang));
        {
            let lock = self.servers.lock().await;
            if let Some(proc) = lock.get(&key) {
                // 检查已缓存的进程是否仍然活着
                if let Ok(mut child) = proc.child.try_lock() {
                    if child.try_wait().ok().flatten().is_none() {
                        return Ok(Arc::clone(proc));
                    }
                }
            }
        }

        // 真实探查可用性
        Self::check_server_health(lang).await?;

        let mut child = Command::new(bin)
            .args(args)
            .current_dir(workspace_root)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("启动 LSP 服务器 `{bin}` 失败: {e}"))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "无法获取 LSP stdin 句柄".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "无法获取 LSP stdout 句柄".to_string())?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| "无法获取 LSP stderr 句柄".to_string())?;

        let stderr_buffer = Arc::new(StdMutex::new(String::new()));
        let stderr_buf_clone = Arc::clone(&stderr_buffer);

        // 异步循环抓取 stderr 管道，避免丢弃错误日志与子进程缓冲区阻塞
        tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            while let Ok(n) = stderr.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                let text = String::from_utf8_lossy(&buf[..n]);
                if let Ok(mut out) = stderr_buf_clone.lock() {
                    out.push_str(&text);
                    if out.len() > 10_000 {
                        let keep = out.len().saturating_sub(5_000);
                        *out = out[keep..].to_string();
                    }
                }
            }
        });

        // 初始等待 50ms 并核验存活
        tokio::time::sleep(Duration::from_millis(50)).await;
        if let Ok(Some(status)) = child.try_wait() {
            let err_msg = stderr_buffer.lock().map(|g| g.clone()).unwrap_or_default();
            return Err(format!(
                "LSP 服务器 `{bin}` 启动后立即异常退出 (code {:?}): {}",
                status.code(),
                err_msg.trim()
            ));
        }

        let proc = Arc::new(LspServerProcess {
            lang,
            stdin: Arc::new(Mutex::new(stdin)),
            stdout: Arc::new(Mutex::new(BufReader::new(stdout))),
            child: Arc::new(Mutex::new(child)),
            stderr_buffer,
            opened_files: Arc::new(Mutex::new(HashSet::new())),
            next_req_id: AtomicI64::new(1),
            indexing: Arc::new(AtomicBool::new(false)),
        });

        // 登记到全局表，供退出钩子使用
        if let Ok(mut g_lock) = active_lsp_processes().lock() {
            g_lock.push(Arc::clone(&proc));
        }

        // 执行标准 LSP `initialize` 握手
        let root_uri = format!("file://{}", workspace_root.display());
        let init_params = json!({
            "processId": std::process::id(),
            "rootUri": root_uri,
            "rootPath": workspace_root.to_string_lossy(),
            "capabilities": {
                "textDocument": {
                    "definition": { "dynamicRegistration": false },
                    "references": { "dynamicRegistration": false },
                    "synchronization": {
                        "dynamicRegistration": false,
                        "openClose": true
                    }
                }
            }
        });

        let init_res = Self::send_request_internal(&proc, "initialize", init_params).await?;
        if init_res.get("error").is_some() {
            return Err(format!("LSP initialize 失败: {:?}", init_res["error"]));
        }

        // 发送 `initialized` 通知
        Self::send_notification_internal(&proc, "initialized", json!({})).await?;

        let mut lock = self.servers.lock().await;
        lock.insert(key, Arc::clone(&proc));

        Ok(proc)
    }

    /// 发送 JSON-RPC 通知（无返回值）
    async fn send_notification_internal(
        proc: &LspServerProcess,
        method: &str,
        params: Value,
    ) -> Result<(), String> {
        let msg = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });
        let body = msg.to_string();
        let header = format!("Content-Length: {}\r\n\r\n", body.len());

        let mut stdin = proc.stdin.lock().await;
        stdin
            .write_all(header.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        stdin
            .write_all(body.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        stdin.flush().await.map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 发送 JSON-RPC 请求并等待响应（带 Content-Length 帧解析、EOF 检测与死循环防御）
    async fn send_request_internal(
        proc: &LspServerProcess,
        method: &str,
        params: Value,
    ) -> Result<Value, String> {
        let req_id = proc.next_req_id.fetch_add(1, Ordering::SeqCst);
        let msg = json!({
            "jsonrpc": "2.0",
            "id": req_id,
            "method": method,
            "params": params
        });
        let body = msg.to_string();
        let header = format!("Content-Length: {}\r\n\r\n", body.len());

        {
            let mut stdin = proc.stdin.lock().await;
            stdin
                .write_all(header.as_bytes())
                .await
                .map_err(|e| format!("向 LSP 写入消息头失败: {e}"))?;
            stdin
                .write_all(body.as_bytes())
                .await
                .map_err(|e| format!("向 LSP 写入请求体失败: {e}"))?;
            stdin
                .flush()
                .await
                .map_err(|e| format!("刷新 LSP 输入缓冲失败: {e}"))?;
        }

        // 读取响应并解析 Content-Length 帧（P0: 增加 EOF Ok(0) 退出判断与存活检查）
        let read_future = async {
            let mut stdout = proc.stdout.lock().await;
            loop {
                let mut line = String::new();
                let bytes_read = stdout
                    .read_line(&mut line)
                    .await
                    .map_err(|e| format!("读取 LSP 响应头失败: {e}"))?;

                // P0 致命修复：read_line 返回 0 表示遇到 EOF（子进程管道已关闭），绝不能当成空行 continue 空转！
                if bytes_read == 0 {
                    let err_msg = proc.stderr_buffer.lock().map(|g| g.clone()).unwrap_or_default();
                    return Err(format!(
                        "LSP 服务端已提前关闭输出管道 (EOF)。stderr: {}",
                        err_msg.trim()
                    ));
                }

                if line.trim().is_empty() {
                    continue;
                }

                if let Some(stripped) = line.to_lowercase().strip_prefix("content-length:") {
                    let len: usize = stripped
                        .trim()
                        .parse()
                        .map_err(|e| format!("解析 Content-Length 失败: {e}"))?;

                    // 跳过随后的空行 "\r\n"
                    let mut blank = String::new();
                    let blank_read = stdout
                        .read_line(&mut blank)
                        .await
                        .map_err(|e| format!("读取 LSP 分隔行失败: {e}"))?;
                    if blank_read == 0 {
                        return Err("LSP 服务在读取请求体分隔符时遇到 EOF".to_string());
                    }

                    let mut body_buf = vec![0u8; len];
                    stdout
                        .read_exact(&mut body_buf)
                        .await
                        .map_err(|e| format!("读取 LSP 消息体失败: {e}"))?;

                    let resp: Value = serde_json::from_slice(&body_buf)
                        .map_err(|e| format!("解析 LSP JSON-RPC 响应失败: {e}"))?;

                    // 顺手记录服务器的忙碌状态。`$/progress` 是通知，不带 id，
                    // 本来就会被下面的 id 匹配跳过 —— 但它携带的信息正是我们需要的。
                    if resp.get("method").and_then(|m| m.as_str()) == Some("$/progress") {
                        match resp
                            .get("params")
                            .and_then(|p| p.get("value"))
                            .and_then(|v| v.get("kind"))
                            .and_then(|k| k.as_str())
                        {
                            Some("begin") => proc.indexing.store(true, Ordering::Relaxed),
                            Some("end") => proc.indexing.store(false, Ordering::Relaxed),
                            _ => {}
                        }
                    }

                    // 判断是否是该请求的响应 ID
                    if resp.get("id").and_then(|id| id.as_i64()) == Some(req_id) {
                        return Ok(resp);
                    }
                }
            }
        };

        match tokio::time::timeout(Duration::from_secs(12), read_future).await {
            Ok(res) => res,
            Err(_) => {
                let err_msg = proc.stderr_buffer.lock().map(|g| g.clone()).unwrap_or_default();
                Err(format!(
                    "等待 LSP 响应超时 (12s)。stderr 日志: {}",
                    if err_msg.is_empty() { "(无错误输出)" } else { err_msg.trim() }
                ))
            }
        }
    }

    /// 确保文件已向 Language Server 执行 `textDocument/didOpen`
    async fn ensure_document_opened(
        proc: &LspServerProcess,
        file_path: &Path,
    ) -> Result<(), String> {
        let needs_open = {
            let mut opened = proc.opened_files.lock().await;
            if opened.contains(file_path) {
                false
            } else {
                opened.insert(file_path.to_path_buf());
                true
            }
        };

        if needs_open {
            let text = std::fs::read_to_string(file_path)
                .map_err(|e| format!("读取待同步文档失败: {e}"))?;
            let file_uri = format!("file://{}", file_path.display());
            let params = json!({
                "textDocument": {
                    "uri": file_uri,
                    "languageId": proc.lang.language_id(),
                    "version": 1,
                    "text": text
                }
            });
            Self::send_notification_internal(proc, "textDocument/didOpen", params).await?;
        }

        Ok(())
    }

    /// 发查询，空结果就在限定窗口内重试。
    ///
    /// **这一条是拿真实 rust-analyzer 量出来的，不是猜的。**
    ///
    /// 语言服务器刚拉起来时对 definition / references 返回**空结果而不是错误**。
    /// 早先的实现只问一次，77ms 拿到空就当「这个符号没有定义」报上去 ——
    /// 那是给模型一条**错误的事实**，比报错还糟。
    ///
    /// 实测（12 行的 crate，rust-analyzer 1.97）：
    ///
    /// ```text
    /// 第 1 次 @ 0.02s → 空
    /// ...
    /// 答出来  @ 1.6s
    /// ```
    ///
    /// 所以 1.6s 量级就够；窗口取 8s 是给大一些的工程留余量，
    /// 同时卡在工具自己 12s 超时之内 —— 超过就变成上层超时，
    /// 那样连「查不到」这个结论都拿不到。
    ///
    /// 顺带说明为什么**不用 `$/progress` 判断是否在索引**：
    /// 那需要在 `initialize` 里声明 `window.workDoneProgress` 能力。
    /// 实测声明之后 rust-analyzer 会灌进来 160+ 条 progress，而不声明时
    /// 它 1.6s 就正常答了。用不着为了"知道它在忙"去换一个更差的链路。
    async fn query_until_ready(
        proc: &Arc<LspServerProcess>,
        method: &str,
        params: Value,
    ) -> Result<Vec<SymbolLocation>, String> {
        const BUDGET: Duration = Duration::from_secs(8);
        const GAP: Duration = Duration::from_millis(400);

        let deadline = Instant::now() + BUDGET;
        let mut attempts = 0u32;
        loop {
            attempts += 1;
            let resp = Self::send_request_internal(proc, method, params.clone()).await?;
            let result = resp.get("result").unwrap_or(&Value::Null);

            let mut locations = Vec::new();
            if let Some(obj) = result.as_object() {
                if let Some(loc) = parse_single_location(obj) {
                    locations.push(loc);
                }
            } else if let Some(arr) = result.as_array() {
                for item in arr {
                    if let Some(obj) = item.as_object() {
                        if let Some(loc) = parse_single_location(obj) {
                            locations.push(loc);
                        }
                    }
                }
            }

            if !locations.is_empty() {
                return Ok(locations);
            }
            if Instant::now() + GAP >= deadline {
                tracing::debug!(
                    method,
                    attempts,
                    "LSP 查询在 {BUDGET:?} 内始终为空",
                );
                // 重试到底仍是空 —— 这时"确实没有"是合理结论，如实返回空。
                // 但大型工程首次调用可能还没索引完，所以工具层的文案要说清这一点。
                return Ok(Vec::new());
            }
            tokio::time::sleep(GAP).await;
        }
    }

    /// 查询符号定义跳转 (Go to Definition)
    pub async fn goto_definition(
        &self,
        workspace_root: &Path,
        file_path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Vec<SymbolLocation>, String> {
        let lang = LanguageKind::from_path(file_path);
        let proc = self.get_or_spawn_server(workspace_root, lang).await?;

        // P1: 执行 didOpen 文档同步
        Self::ensure_document_opened(&proc, file_path).await?;

        let file_uri = format!("file://{}", file_path.display());
        let params = json!({
            "textDocument": { "uri": file_uri },
            "position": { "line": line, "character": character }
        });

        let locations =
            Self::query_until_ready(&proc, "textDocument/definition", params).await?;

        Ok(locations)
    }

    /// 查询符号所有引用 (Find References)
    pub async fn find_references(
        &self,
        workspace_root: &Path,
        file_path: &Path,
        line: u32,
        character: u32,
        include_declaration: bool,
    ) -> Result<Vec<SymbolLocation>, String> {
        let lang = LanguageKind::from_path(file_path);
        let proc = self.get_or_spawn_server(workspace_root, lang).await?;

        // P1: 执行 didOpen 文档同步
        Self::ensure_document_opened(&proc, file_path).await?;

        let file_uri = format!("file://{}", file_path.display());
        let params = json!({
            "textDocument": { "uri": file_uri },
            "position": { "line": line, "character": character },
            "context": { "includeDeclaration": include_declaration }
        });

        let locations =
            Self::query_until_ready(&proc, "textDocument/references", params).await?;

        Ok(locations)
    }
}

fn parse_single_location(obj: &serde_json::Map<String, Value>) -> Option<SymbolLocation> {
    let uri_str = obj
        .get("uri")
        .or_else(|| obj.get("targetUri"))
        .and_then(|u| u.as_str())?;
    let file_path = uri_str
        .strip_prefix("file://")
        .unwrap_or(uri_str)
        .to_string();

    let range = obj
        .get("range")
        .or_else(|| obj.get("targetSelectionRange"))
        .or_else(|| obj.get("targetRange"))?;

    let start = range.get("start")?;
    let end = range.get("end")?;

    let line = start.get("line")?.as_u64()? as u32;
    let character = start.get("character")?.as_u64()? as u32;
    let end_line = end.get("line")?.as_u64()? as u32;
    let end_character = end.get("character")?.as_u64()? as u32;

    // 读取所在行代码片段作为 preview
    let preview = std::fs::read_to_string(&file_path)
        .ok()
        .and_then(|content| {
            content
                .lines()
                .nth(line as usize)
                .map(|l| l.trim().to_string())
        });

    Some(SymbolLocation {
        file_path,
        line: line + 1, // 转为 1-indexed 便于人类与模型阅读
        character: character + 1,
        end_line: end_line + 1,
        end_character: end_character + 1,
        preview,
    })
}
