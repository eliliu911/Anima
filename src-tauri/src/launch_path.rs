//! 双击启动的 `.app` **没有你 shell 里的 `PATH`**。
//!
//! # 症状
//!
//! 2026-09-18 实测：打包版里所有 MCP server 都报
//! `启动 MCP server「本地文件」失败（命令：npx）：No such file or directory (os error 2)`。
//!
//! 而 `npx` 明明装着 —— 在 `/opt/homebrew/bin/npx`。
//!
//! # 原因
//!
//! 从终端起的进程继承的是 shell 的 `PATH`（`.zshrc` / `.zprofile` 里加过
//! homebrew、nvm、cargo 那些）。**双击 `.app` 走的是 launchd，不经过 shell** ——
//! `launchctl getenv PATH` 在这台机器上是空的，于是进程只拿到内核默认的
//! `/usr/bin:/bin:/usr/sbin:/sbin`。
//!
//! 这一条影响的**不只是 MCP**：任何 `Command::new("node")` /
//! `Command::new("uv")` / `Command::new("ffmpeg")` 都一样找不到。
//!
//! # 怎么补
//!
//! 启动时问一次登录 shell 真正的 `PATH`，合并进当前进程。
//! 这是 VS Code / 各类 Electron 应用的通行做法。
//!
//! 两个必须的保险：
//!
//! - **要超时。** `$SHELL -l` 会跑用户的 profile，里面可能有 nvm、conda、
//!   甚至交互式的东西。卡住的话整个应用起不来 —— 那比少几个命令严重得多。
//! - **要有兜底。** shell 问不出来时补上几个常见位置，总比只有
//!   `/usr/bin:/bin` 强。

use std::path::Path;
use std::time::Duration;

/// 问登录 shell 最多等多久。超了就用兜底 —— 卡住比少几个命令严重得多。
const ASK_SHELL_TIMEOUT: Duration = Duration::from_secs(4);

/// shell 问不出来时补这几个。**顺序有意义**：homebrew 在前，
/// 因为 macOS 自带的 `/usr/bin` 里也有同名但太老的工具（python3、git）。
const FALLBACK: &[&str] = &[
    "/opt/homebrew/bin",
    "/opt/homebrew/sbin",
    "/usr/local/bin",
    "/usr/local/sbin",
];

/// 相对 `$HOME` 的兜底目录。
const FALLBACK_HOME: &[&str] = &[".cargo/bin", ".local/bin", ".bun/bin"];

/// 合并两个 `PATH`，**保序去重**。
///
/// `extra` 在前：登录 shell 的 `PATH` 才是用户真正想要的那一份，
/// 反过来的话 `/usr/bin/python3` 会压过 homebrew 的。
///
/// 纯函数，可测 —— 这块的 bug 表现是「某个命令用到了错误的版本」，
/// 从现象追回来很费劲。
pub fn merge(current: &str, extra: &str) -> String {
    let mut seen: Vec<&str> = Vec::new();
    for part in extra.split(':').chain(current.split(':')) {
        let part = part.trim();
        if part.is_empty() || seen.contains(&part) {
            continue;
        }
        seen.push(part);
    }
    seen.join(":")
}

/// 现在这个进程的 `PATH` 是不是「只有内核默认那几个」。
///
/// 用它判断要不要去问 shell：从终端起的时候完全没必要多跑一个子进程。
pub fn looks_bare(path: &str) -> bool {
    !path.split(':').any(|p| {
        let p = p.trim();
        !p.is_empty()
            && p != "/usr/bin"
            && p != "/bin"
            && p != "/usr/sbin"
            && p != "/sbin"
            && p != "/usr/local/bin"
    })
}

/// 兜底的那几条，拼成 `PATH` 片段。
pub fn fallback_path(home: Option<&Path>) -> String {
    let mut v: Vec<String> = FALLBACK.iter().map(|s| s.to_string()).collect();
    if let Some(h) = home {
        for rel in FALLBACK_HOME {
            v.push(h.join(rel).to_string_lossy().to_string());
        }
    }
    v.join(":")
}

/// 问登录 shell 要 `PATH`。超时或失败回 `None`。
fn ask_login_shell() -> Option<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        // `-l` 跑 login profile（homebrew / nvm / cargo 都在那儿加的）。
        // 用 `printf` 而不是 `echo`：profile 里 `echo` 可能被改过
        let out = std::process::Command::new(&shell)
            .args(["-l", "-c", "printf %s \"$PATH\""])
            .output();
        let _ = tx.send(out.ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()));
    });
    rx.recv_timeout(ASK_SHELL_TIMEOUT).ok().flatten().filter(|s| !s.is_empty())
}

/// 启动时调一次。**必须排在任何 `Command::new` 之前** ——
/// MCP server、sidecar、ffmpeg 全都要靠它。
///
/// 回 `(补没补, 最终的 PATH)`，方便诊断页显示。
pub fn ensure() -> (bool, String) {
    let current = std::env::var("PATH").unwrap_or_default();
    if !looks_bare(&current) {
        return (false, current);
    }
    let extra = ask_login_shell().unwrap_or_else(|| {
        tracing::warn!("登录 shell 问不出 PATH，用兜底列表");
        fallback_path(dirs::home_dir().as_deref())
    });
    let merged = merge(&current, &extra);
    // SAFETY: 启动早期、单线程阶段调用一次；之后只读
    unsafe { std::env::set_var("PATH", &merged) };
    tracing::info!(before = %current, after = %merged, "PATH 太窄，已补全（双击启动不经过 shell）");
    (true, merged)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **`extra` 必须在前。** 反过来的话 `/usr/bin/python3` 会压过
    /// homebrew 的那个，表现是「某个命令用到了错误的版本」——
    /// 从现象追回来很费劲。
    #[test]
    fn 合并时登录shell的在前() {
        let m = merge("/usr/bin:/bin", "/opt/homebrew/bin:/usr/bin");
        assert_eq!(m, "/opt/homebrew/bin:/usr/bin:/bin");
    }

    #[test]
    fn 合并去重且保序() {
        assert_eq!(merge("/a:/b:/a", "/b:/c"), "/b:/c:/a");
        assert_eq!(merge("", "/x"), "/x");
        assert_eq!(merge("/x", ""), "/x");
        // 空段和空白不能变成一条空路径
        assert_eq!(merge("/a::/b", "  :/c"), "/c:/a:/b");
    }

    /// 只有内核默认那几条 = 需要补。多一条就说明经过 shell 了。
    #[test]
    fn 认得出裸PATH() {
        assert!(looks_bare("/usr/bin:/bin:/usr/sbin:/sbin"));
        assert!(looks_bare(""));
        assert!(looks_bare("/usr/local/bin:/usr/bin:/bin"), "只有 /usr/local/bin 仍然算裸");
        assert!(!looks_bare("/opt/homebrew/bin:/usr/bin:/bin"));
        assert!(!looks_bare("/Users/me/.cargo/bin:/usr/bin"));
    }

    /// 兜底至少要有 homebrew —— 这台机器上 node/npx/ffmpeg 都在那儿。
    #[test]
    fn 兜底带homebrew和家目录() {
        let f = fallback_path(Some(Path::new("/Users/me")));
        assert!(f.starts_with("/opt/homebrew/bin"), "homebrew 要排最前：{f}");
        assert!(f.contains("/Users/me/.cargo/bin"));
        // 没有 home 也不能崩，也不能拼出 "/.cargo/bin"
        let f2 = fallback_path(None);
        assert!(!f2.contains(".cargo"), "没有 home 时不该硬拼：{f2}");
    }
}
