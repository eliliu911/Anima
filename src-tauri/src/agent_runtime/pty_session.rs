//! Portable PTY Session Manager for Interactive Terminal Execution
//!
//! Provides true Master/Slave PTY allocation for interactive CLI processes
//! (such as REPLs, interactive setup scripts, dev servers, and live watch tasks).
//! Handles raw ANSI output streaming, non-blocking reading, window resizing, and SIGKILL teardown.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, CommandBuilder, PtyPair, PtySize};
use serde::{Deserialize, Serialize};

const MAX_PTY_SESSIONS: usize = 6;
const MAX_PTY_BUFFER_BYTES: usize = 1_000_000;
const KEEP_PTY_BUFFER_BYTES: usize = 400_000;
const PTY_SESSION_TTL: Duration = Duration::from_secs(60 * 60); // 1 hour idle TTL

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyExecOutput {
    pub session_id: String,
    pub output: String,
    pub cursor: usize,
    pub is_running: bool,
    pub exit_code: Option<u32>,
    pub truncated: bool,
}

/// `plan_input` 的判定结果：这次 `pty_write` 该往 PTY 里写什么。
pub enum PtyInputPlan {
    /// 控制键：原样透传（拼不出命令文本）。
    Passthrough,
    /// 不含行终止符的普通文本：只累积，**不写 PTY**。
    Buffered,
    /// 凑齐了完整行：交调用方逐行核验，全部通过后才写下去。
    Lines(Vec<String>),
}

pub struct PtySession {
    pub id: String,
    pub pair: Arc<StdMutex<Option<PtyPair>>>,
    pub writer: Arc<StdMutex<Box<dyn Write + Send>>>,
    pub output: Arc<StdMutex<String>>,
    pub child: Arc<StdMutex<Box<dyn portable_pty::Child + Send>>>,
    pub created_at: Instant,
    pub last_access: Arc<StdMutex<Instant>>,
    pub pending_input: Arc<StdMutex<String>>,
    /// shell 的行缓冲内容是否已不可知（透传过控制键之后置位）。
    pub buffer_dirty: Arc<StdMutex<bool>>,
}

type SessionMap = Arc<StdMutex<HashMap<String, PtySession>>>;

/// 进程级的活跃会话表登记处。
///
/// `Drop` 只在析构函数真的跑到时才生效，而 macOS 上应用退出常走 `process::exit`，
/// 那条路径上析构不跑 —— 「用户关掉 Anima，沙箱里还挂着一个 `npm run dev`」
/// 恰恰是 `Drop` 覆盖不到的场景。所以额外登记一份弱引用，
/// 供 Tauri 的 `RunEvent::Exit` 钩子显式回收（`stop_all_pty_sessions`）。
static LIVE_SESSION_MAPS: std::sync::OnceLock<StdMutex<Vec<std::sync::Weak<StdMutex<HashMap<String, PtySession>>>>>> =
    std::sync::OnceLock::new();

fn live_session_maps() -> &'static StdMutex<Vec<std::sync::Weak<StdMutex<HashMap<String, PtySession>>>>> {
    LIVE_SESSION_MAPS.get_or_init(|| StdMutex::new(Vec::new()))
}

/// 终止所有活跃 PTY 会话的子进程。供 Tauri 的 `RunEvent::Exit` / `ExitRequested` 钩子调用，
/// 与 `music::stop_radio()` / `sidecar::stop_all_sidecars()` 并列。
pub fn stop_all_pty_sessions() {
    let Ok(mut registry) = live_session_maps().lock() else {
        return;
    };
    registry.retain(|weak| match weak.upgrade() {
        Some(map) => {
            kill_sessions_in(&map);
            true
        }
        None => false, // 已经被 Drop 回收，从登记处摘掉
    });
}

/// 终止**一张**会话表里的所有子进程。
///
/// 单独抽出来是为了让回归测试能只作用于自己那个 manager ——
/// `stop_all_pty_sessions` 会遍历进程内所有会话表，在并行测试里调它
/// 会把别的测试正在用的 PTY 会话一起杀掉。
fn kill_sessions_in(map: &SessionMap) {
    let Ok(mut sessions) = map.lock() else {
        return;
    };
    for (id, session) in sessions.drain() {
        if let Ok(mut child) = session.child.lock() {
            if child.try_wait().ok().flatten().is_none() {
                tracing::info!(session_id = %id, "应用退出：正在终止 PTY 子进程...");
                let _ = child.kill();
            }
        }
    }
}

#[derive(Clone)]
pub struct PtySessionManager {
    sessions: SessionMap,
}

impl Default for PtySessionManager {
    fn default() -> Self {
        let sessions: SessionMap = Arc::new(StdMutex::new(HashMap::new()));
        if let Ok(mut registry) = live_session_maps().lock() {
            registry.push(Arc::downgrade(&sessions));
        }
        Self { sessions }
    }
}

impl Drop for PtySessionManager {
    fn drop(&mut self) {
        // 当 PtySessionManager 最后一个引用被 drop 时，强制终止所有子进程
        if Arc::strong_count(&self.sessions) == 1 {
            self.kill_all();
        }
    }
}

impl PtySessionManager {
    /// 显式终止所有活跃 PTY 进程（供 Tauri ExitRequested 钩子或 Drop 调用）
    pub fn kill_all(&self) {
        if let Ok(mut lock) = self.sessions.lock() {
            for (_, session) in lock.drain() {
                if let Ok(mut child) = session.child.lock() {
                    if child.try_wait().ok().flatten().is_none() {
                        let _ = child.kill();
                    }
                }
            }
        }
    }

    pub async fn spawn(
        &self,
        cmd: &str,
        workdir: &Path,
        env_vars: &[(String, String)],
        rows: u16,
        cols: u16,
    ) -> Result<(String, PtyExecOutput), String> {
        self.cleanup_stale();

        let pty_system = native_pty_system();
        let size = PtySize {
            rows: rows.max(10),
            cols: cols.max(20),
            pixel_width: 0,
            pixel_height: 0,
        };

        let pair = pty_system
            .openpty(size)
            .map_err(|e| format!("打开 PTY 失败: {e}"))?;

        let mut builder = CommandBuilder::new("sh");
        builder.arg("-lc");
        builder.arg(cmd);
        builder.cwd(workdir);

        for (k, v) in env_vars {
            builder.env(k, v);
        }
        builder.env("TERM", "xterm-256color");

        let child = pair
            .slave
            .spawn_command(builder)
            .map_err(|e| format!("在 PTY 中启动命令失败: {e}"))?;

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("获取 PTY reader 失败: {e}"))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("获取 PTY writer 失败: {e}"))?;

        let session_id = uuid::Uuid::new_v4().to_string();
        let output = Arc::new(StdMutex::new(String::new()));
        let output_collector = Arc::clone(&output);

        // 启动独立线程同步采集 PTY 原始输出并写入环形缓冲区
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let text = String::from_utf8_lossy(&buf[..n]);
                        if let Ok(mut out) = output_collector.lock() {
                            out.push_str(&text);
                            if out.len() > MAX_PTY_BUFFER_BYTES {
                                let keep_from = out.len().saturating_sub(KEEP_PTY_BUFFER_BYTES);
                                *out = out[keep_from..].to_string();
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        // 初始等待 150ms，抓取启动问候或交互提示
        tokio::time::sleep(Duration::from_millis(150)).await;

        let session = PtySession {
            id: session_id.clone(),
            pair: Arc::new(StdMutex::new(Some(pair))),
            writer: Arc::new(StdMutex::new(writer)),
            output: Arc::clone(&output),
            child: Arc::new(StdMutex::new(child)),
            created_at: Instant::now(),
            last_access: Arc::new(StdMutex::new(Instant::now())),
            pending_input: Arc::new(StdMutex::new(String::new())),
            buffer_dirty: Arc::new(StdMutex::new(false)),
        };

        {
            let mut lock = self.sessions.lock().map_err(|e| e.to_string())?;
            if lock.len() >= MAX_PTY_SESSIONS {
                return Err(format!(
                    "活跃 PTY 会话数已达上限 {MAX_PTY_SESSIONS}，请先终止已有会话"
                ));
            }
            lock.insert(session_id.clone(), session);
        }

        let initial_output = output.lock().map(|g| g.clone()).unwrap_or_default();
        let cursor = initial_output.len();

        Ok((
            session_id.clone(),
            PtyExecOutput {
                session_id,
                output: initial_output,
                cursor,
                is_running: true,
                exit_code: None,
                truncated: false,
            },
        ))
    }

    /// `pty_write` 这一次调用**到底该往 PTY 里写什么**。
    ///
    /// 存在的唯一理由是维持一条不变式：
    /// **写进 PTY 的字节，必须和通过 Guard 核验的字节逐一相同。**
    ///
    /// 早先的实现把原始 input 直接透传，只在 Anima 侧另存一份副本用于核验 —— 于是
    /// shell 的行缓冲和 `pending_input` 成了两份独立状态。某一行被拒时只清了 Anima 那份，
    /// 之后每次核验依据的都是错的文本。实测可以这样绕过：先写一段不含换行的片段
    /// （不过闸，但已进 shell 缓冲，结尾放个 `#` 把后续输入吃成注释），再故意触发一次拒绝
    /// 制造失同步，最后让用户批准一条无害命令 —— 用户批准的是 `ls`，执行的是那段片段。
    ///
    /// 现在的规则保证 shell 的行缓冲在每次提交前后都是空的：普通文本只在 Anima 侧累积，
    /// 只有整行过闸之后才连同换行一次性写下去。
    pub fn plan_input(&self, session_id: &str, input: &str) -> Result<PtyInputPlan, String> {
        let lock = self.sessions.lock().map_err(|e| e.to_string())?;
        let session = lock
            .get(session_id)
            .ok_or_else(|| format!("未找到 session_id: {session_id}"))?;

        // 控制键（Ctrl-C、Ctrl-D、退格、方向键的 ESC 序列……）原样透传：
        // 它们拼不出命令文本，但能改变 shell 的行缓冲（上方向键就能调出历史命令），
        // 所以置位 buffer_dirty。注意 `\n` / `\r` 不算控制键 —— 它们是行终止符，
        // 必须走下面的核验路径，否则等于给绕过留了一道门。
        if input
            .chars()
            .any(|c| c != '\n' && c != '\r' && (c.is_control() || c == '\u{7f}'))
        {
            if let Ok(mut pending) = session.pending_input.lock() {
                pending.clear();
            }
            if let Ok(mut dirty) = session.buffer_dirty.lock() {
                *dirty = true;
            }
            return Ok(PtyInputPlan::Passthrough);
        }

        let mut pending = session.pending_input.lock().map_err(|e| e.to_string())?;
        pending.push_str(input);

        if !pending.contains('\n') && !pending.contains('\r') {
            // 还没凑齐一行 —— 只累积，一个字节都不写进 PTY。
            return Ok(PtyInputPlan::Buffered);
        }

        let mut ready_lines = Vec::new();
        let mut current_segment = String::new();
        let chars: Vec<char> = pending.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '\n' || c == '\r' {
                let trimmed = current_segment.trim().to_string();
                if !trimmed.is_empty() {
                    ready_lines.push(trimmed);
                }
                current_segment.clear();
                // 兼容 \r\n
                if c == '\r' && i + 1 < chars.len() && chars[i + 1] == '\n' {
                    i += 1;
                }
            } else {
                current_segment.push(c);
            }
            i += 1;
        }

        // 行终止符之后剩下的不完整尾巴继续留在缓冲里，本次不写。
        *pending = current_segment;

        Ok(PtyInputPlan::Lines(ready_lines))
    }

    /// 取出并复位「shell 行缓冲内容未知」标记。
    fn take_buffer_dirty(&self, session_id: &str) -> bool {
        let Ok(lock) = self.sessions.lock() else {
            return false;
        };
        let Some(session) = lock.get(session_id) else {
            return false;
        };
        let Ok(mut dirty) = session.buffer_dirty.lock() else {
            return false;
        };
        std::mem::replace(&mut *dirty, false)
    }

    /// 把**已经过闸**的整行写进 PTY —— 普通文本进入 shell 的唯一出口。
    ///
    /// 若此前透传过控制键（行缓冲里可能被塞进了历史命令等我们看不见的东西），
    /// 先发一个 `\x15`（kill-line）清空，再写这些行 —— 否则 shell 拼出来的
    /// 就不是刚刚核验过的那条命令了。
    pub async fn write_vetted_lines(
        &self,
        session_id: &str,
        lines: &[String],
        cursor: usize,
        max_chars: usize,
    ) -> Result<PtyExecOutput, String> {
        let mut payload = String::new();
        if self.take_buffer_dirty(session_id) {
            payload.push('\u{15}');
        }
        for line in lines {
            payload.push_str(line);
            payload.push('\n');
        }
        self.write_input(session_id, &payload, cursor, max_chars)
            .await
    }

    /// 清空会话的 pending_input（例如命令审核失败时丢弃该行）
    pub fn clear_pending_input(&self, session_id: &str) {
        if let Ok(lock) = self.sessions.lock() {
            if let Some(session) = lock.get(session_id) {
                if let Ok(mut pending) = session.pending_input.lock() {
                    pending.clear();
                }
            }
        }
    }

    pub async fn write_input(
        &self,
        session_id: &str,
        input: &str,
        cursor: usize,
        max_chars: usize,
    ) -> Result<PtyExecOutput, String> {
        self.cleanup_stale();
        let (writer, child_arc, output_arc, last_access_arc) = {
            let lock = self.sessions.lock().map_err(|e| e.to_string())?;
            let session = lock
                .get(session_id)
                .ok_or_else(|| format!("未找到 session_id: {session_id}"))?;
            (
                Arc::clone(&session.writer),
                Arc::clone(&session.child),
                Arc::clone(&session.output),
                Arc::clone(&session.last_access),
            )
        };

        {
            let mut writer = writer.lock().map_err(|e| e.to_string())?;
            writer
                .write_all(input.as_bytes())
                .map_err(|e| format!("写入 PTY stdin 失败: {e}"))?;
            writer.flush().map_err(|e| format!("刷新 PTY stdin 失败: {e}"))?;
        }

        if let Ok(mut access) = last_access_arc.lock() {
            *access = Instant::now();
        }

        // 给予子进程 150ms 响应输入并渲染
        tokio::time::sleep(Duration::from_millis(150)).await;

        let (is_running, exit_code) = {
            let mut child = child_arc.lock().map_err(|e| e.to_string())?;
            let maybe_exit = child.try_wait().ok().flatten().map(|s| s.exit_code());
            (maybe_exit.is_none(), maybe_exit)
        };

        let full_output = output_arc.lock().map(|g| g.clone()).unwrap_or_default();
        let next_cursor = full_output.len();
        let start = cursor.min(full_output.len());
        let slice = &full_output[start..];

        let (output, truncated) = if slice.chars().count() > max_chars {
            (slice.chars().take(max_chars).collect::<String>() + "...", true)
        } else {
            (slice.to_string(), false)
        };

        Ok(PtyExecOutput {
            session_id: session_id.to_string(),
            output,
            cursor: next_cursor,
            is_running,
            exit_code,
            truncated,
        })
    }

    pub async fn poll_output(
        &self,
        session_id: &str,
        cursor: usize,
        max_chars: usize,
    ) -> Result<PtyExecOutput, String> {
        self.cleanup_stale();
        let (child_arc, output_arc, last_access_arc) = {
            let lock = self.sessions.lock().map_err(|e| e.to_string())?;
            let session = lock
                .get(session_id)
                .ok_or_else(|| format!("未找到 session_id: {session_id}"))?;
            (
                Arc::clone(&session.child),
                Arc::clone(&session.output),
                Arc::clone(&session.last_access),
            )
        };

        if let Ok(mut access) = last_access_arc.lock() {
            *access = Instant::now();
        }

        let (is_running, exit_code) = {
            let mut child = child_arc.lock().map_err(|e| e.to_string())?;
            let maybe_exit = child.try_wait().ok().flatten().map(|s| s.exit_code());
            (maybe_exit.is_none(), maybe_exit)
        };

        let full_output = output_arc.lock().map(|g| g.clone()).unwrap_or_default();
        let next_cursor = full_output.len();
        let start = cursor.min(full_output.len());
        let slice = &full_output[start..];

        let (output, truncated) = if slice.chars().count() > max_chars {
            (slice.chars().take(max_chars).collect::<String>() + "...", true)
        } else {
            (slice.to_string(), false)
        };

        Ok(PtyExecOutput {
            session_id: session_id.to_string(),
            output,
            cursor: next_cursor,
            is_running,
            exit_code,
            truncated,
        })
    }

    pub async fn resize(&self, session_id: &str, rows: u16, cols: u16) -> Result<(), String> {
        let pair_arc = {
            let lock = self.sessions.lock().map_err(|e| e.to_string())?;
            let session = lock
                .get(session_id)
                .ok_or_else(|| format!("未找到 session_id: {session_id}"))?;
            Arc::clone(&session.pair)
        };

        let pair_lock = pair_arc.lock().map_err(|e| e.to_string())?;
        if let Some(pair) = pair_lock.as_ref() {
            pair.master
                .resize(PtySize {
                    rows: rows.max(10),
                    cols: cols.max(20),
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|e| format!("调整 PTY 窗口尺寸失败: {e}"))?;
        }
        Ok(())
    }

    pub async fn kill(&self, session_id: &str) -> Result<Option<u32>, String> {
        let session = {
            let mut lock = self.sessions.lock().map_err(|e| e.to_string())?;
            lock.remove(session_id)
                .ok_or_else(|| format!("未找到 session_id: {session_id}"))?
        };

        let mut child = session.child.lock().map_err(|e| e.to_string())?;
        if child.try_wait().ok().flatten().is_none() {
            let _ = child.kill();
        }
        let status = child.wait().ok().map(|s| s.exit_code());
        Ok(status)
    }

    fn cleanup_stale(&self) {
        let Ok(mut lock) = self.sessions.lock() else {
            return;
        };
        let now = Instant::now();
        let mut stale_ids = Vec::new();

        for (id, session) in lock.iter() {
            let is_dead = session
                .child
                .lock()
                .ok()
                .and_then(|mut c| c.try_wait().ok().flatten())
                .is_some();
            let is_idle = session
                .last_access
                .lock()
                .ok()
                .map(|t| now.duration_since(*t) > PTY_SESSION_TTL)
                .unwrap_or(false);

            if is_dead || is_idle {
                stale_ids.push(id.clone());
            }
        }

        for id in stale_ids {
            if let Some(session) = lock.remove(&id) {
                if let Ok(mut child) = session.child.lock() {
                    if child.try_wait().ok().flatten().is_none() {
                        let _ = child.kill();
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod exit_hook_tests {
    use super::*;

    /// 回归：退出钩子（`RunEvent::Exit` → `stop_all_pty_sessions`）必须真的杀掉子进程。
    ///
    /// `Drop` 不够 —— macOS 上应用退出常走 `process::exit`，那条路径上析构函数不跑，
    /// 而「用户关掉 Anima、沙箱里还挂着一个 `npm run dev`」恰恰是 Drop 覆盖不到的场景。
    ///
    /// 这里调的是 `kill_sessions_in` 而不是 `stop_all_pty_sessions`：后者会遍历进程内
    /// 所有会话表，在并行测试里会把别的测试正在用的会话一起杀掉。
    #[tokio::test]
    async fn exit_hook_kills_child_processes() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = PtySessionManager::default();
        let (sid, _) = mgr.spawn("sleep 120", dir.path(), &[], 24, 80).await.unwrap();

        let child_arc = {
            let lock = mgr.sessions.lock().unwrap();
            Arc::clone(&lock.get(&sid).unwrap().child)
        };
        let pid = child_arc.lock().unwrap().process_id().expect("子进程该在跑");

        kill_sessions_in(&mgr.sessions);
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        let alive = std::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        assert!(!alive, "退出钩子跑过之后 pid {pid} 还活着");
    }

    /// 回归：manager 必须把自己登记到进程级表里，否则 `stop_all_pty_sessions` 找不到它，
    /// 退出钩子就成了空转 —— 而这正是「加了 kill_all 却没人调用」那次的失败形状。
    #[test]
    fn manager_registers_itself_for_the_exit_hook() {
        let before = live_session_maps().lock().unwrap().len();
        let mgr = PtySessionManager::default();
        let after = live_session_maps().lock().unwrap().len();
        assert!(after > before, "新建的 manager 没有登记到进程级会话表");
        assert!(
            live_session_maps()
                .lock()
                .unwrap()
                .iter()
                .any(|w| w.upgrade().map(|m| Arc::ptr_eq(&m, &mgr.sessions)).unwrap_or(false)),
            "登记处里找不到这个 manager 的会话表"
        );
    }
}
