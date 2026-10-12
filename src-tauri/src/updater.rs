//! 应用自更新 —— **本机构建那条路**。
//!
//! # 为什么是「重新构建」而不是「下载新包」
//!
//! Tauri 官方的 updater 要一对 minisign 密钥和一个放 `latest.json` 的地方，
//! 两样都得先定；而自用 + 开源自 build 的场景下，装的人本来就有完整仓库。
//! 对外分发那条路另做（`bundle.createUpdaterArtifacts`），两者不冲突。
//!
//! # 要比三个点，不是两个
//!
//! 2026-09-18 用户当场指出来的：面板说「已经是最新的」，而他跑的二进制
//! 比磁盘上的代码旧两个提交。**因为它只比了 git HEAD ↔ origin** ——
//! 那两边确实一致，但真正要紧的那一段没人看：
//!
//! ```text
//! 正在跑的构建  ──①──  本地仓库 HEAD  ──②──  origin
//!      a857fd3            a3e98aa           a3e98aa
//!      └─ 差 2 个提交 ─┘   └─ 一致，于是「已经是最新的」
//! ```
//!
//! ① 才是用户真正会踩的那段：拉过代码、或者自己改过，但**没重新构建**。
//! 和早上 `anima install` 装了旧包却一声不吭是同一个盲区。
//!
//! 所以现在两段都报：① 说「重新构建」，② 说「下载并构建」。
//!
//! # 还有第四个点：磁盘上构建好、还没换上的那个包
//!
//! 2026-09-28 用户：「构建完了，然后点检查更新，版本号没变，但是只有重新构建，
//! 就会又做一遍相同版本的构建，耽误时间也不合理。」
//!
//! 构建完、还没换包时再点一次「检查更新」，界面就忘了「已经构建好了」——
//! 换包按钮只在「刚构建完」那一刻出现，而正在跑的仍是旧构建，① 照样报落后，
//! 于是只剩「重新构建当前代码」，一点就把同一份代码再编十几分钟。
//! 已经是最新时那个按钮也照样亮着。
//!
//! 所以查的时候也看一眼 `bundle/macos/Anima.app`：**它的二进制里有没有当前 HEAD 的构建戳**
//! （[`BUILD_STAMP`]）。有 = 构建好了只差换包，不用再编。
//! 不看标记文件、不看时间戳 —— 谁构建的、怎么构建的都一样认得出。
//!
//! **不能直接搜 sha。** 第一版就是搜 `929e9d83`，在真的包里一次都没搜到：
//! 不到 16 字节的短字符串，编译器会拆成几条指令的立即数（arm64 是 16 位一段的 movz/movk），
//! 二进制里根本没有连续的那 8 个字节 —— 版本号 `0.42.90` 也搜不到，只有够长的构建日期留在了常量区。
//! 所以另编一段带前缀的戳，用 `#[used]` 静态量钉在常量区里。
//!
//! # 三步，中间那步十几分钟
//!
//! 1. **查**：`git fetch` 之后看落后几个提交，把提交标题列出来
//! 2. **构建**：`git pull --ff-only` → `npm run tauri:build -- --bundles app`
//! 3. **换**：应用**不能覆盖正在运行的自己**，所以写一个小脚本、脱离进程启动，
//!    由它等本进程退出后再换包并重新打开
//!
//! # 不做假进度条，但要让人看得出它活着
//!
//! cargo 不告诉你总共要编多少个 crate，凑一个百分比只会在 87% 停十分钟。
//!
//! 但第一版只报「阶段 + 已编译个数」是不够的，2026-09-18 实测暴露了：
//! **增量构建只有 `anima` 一个 crate 要编**，于是界面上就是一行
//! 「已编译 1 个」，然后**静止六分钟** —— 那六分钟全在链接，
//! 而 cargo 在链接期一个字都不输出。看起来和卡死一模一样。
//!
//! 所以现在三样一起给：
//!
//! - **已用秒数**，每 2 秒一次心跳。链接期完全静默，这是唯一能证明没死的东西
//! - **阶段**（拉代码 / 前端 / 编译 / **链接** / 打包）—— 离散且真实。
//!   链接单独列出来，因为它最长也最安静
//! - **原始输出行**，前端按终端那样滚。噪音行（`|`、`^^^`、`= note:`）滤掉
//!
//! # 几条拒绝更新的理由，都是故意的
//!
//! - **工作区有未提交的改动**（只看已跟踪文件）：拉代码会撞车
//! - **不是快进**：本地和远端分叉了，自动合并只会把事情搞复杂
//! - **找不到仓库**：打包版靠 `app_root` 指针找，没装过就没有

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::Serialize;
use tauri::{Emitter, Manager};

/// 进度事件名。前端 `listen` 它。
pub const EVENT: &str = "updater://progress";

/// 一条进度。
#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    /// `git` / `frontend` / `cargo` / `link` / `bundle` / `tick` / `done` / `failed`
    pub phase: &'static str,
    /// 给人看的一句话
    pub label: String,
    /// 已编译的 crate 数（只有 `cargo` 阶段有意义）
    pub crates: usize,
    /// **原始输出行**，不是我们编的话。空 = 这条是心跳，没有新输出
    pub line: String,
    /// 从开始构建到现在多少秒。**链接期完全静默，全靠它证明还活着**
    pub elapsed: u64,
}

/// 查更新的结果。
#[derive(Debug, Clone, Default, Serialize)]
pub struct CheckResult {
    /// 仓库路径。`None` = 没找到（打包版没跑过 `anima install`）
    pub repo: Option<String>,
    pub branch: String,
    /// 本地 HEAD 落后 origin 几个提交
    pub behind: usize,
    /// 远端新提交的标题，最多 20 条
    pub commits: Vec<String>,
    /// **正在跑的这个构建**是从哪个提交编出来的（可能带 `-dirty`）
    pub running_sha: String,
    /// **正在跑的构建落后本地代码几个提交。**
    ///
    /// 这一段才是最常踩的：拉过代码 / 自己改过，但没重新构建。
    /// 只比 HEAD↔origin 的话它永远是 0 个提示都没有。
    pub behind_built: usize,
    /// 已经写进本地仓库、但还没编进当前构建的提交
    pub built_commits: Vec<String>,
    /// 当前构建是带未提交改动编出来的
    pub running_dirty: bool,
    /// 本地仓库 HEAD（短 sha，和 `build.rs` 取的同一种格式）
    pub head_sha: String,
    /// **磁盘上构建好的包就是用当前 HEAD 编的，只差换包。**
    ///
    /// 为真时不该再让人「重新构建」—— 那是把同一份代码再编十几分钟。
    /// 正在跑的已经是 HEAD 时它是 `false`（没有要换的）。
    pub bundle_ready: bool,
    /// 不能更新的原因。**有值就别让按钮可点**
    pub blocked: Option<String>,
}

/// 找仓库根：要同时有 `.git` 和 `package.json`，光有一个都可能是别人的目录。
pub fn find_repo(config_dir: &Path) -> Option<PathBuf> {
    crate::app_root::roots(Some(config_dir))
        .into_iter()
        .find(|r| r.join(".git").exists() && r.join("package.json").is_file())
}

/// `git rev-list --count` 的输出 → 数字。读不出来当 0（= 没有更新），
/// **不是当成「有更新」** —— 宁可少提示一次，也不要让人白等十几分钟。
pub fn parse_behind(out: &str) -> usize {
    out.trim().lines().next().unwrap_or("").trim().parse().unwrap_or(0)
}

/// `git log --oneline` 的输出 → 提交标题列表。
pub fn parse_commits(out: &str) -> Vec<String> {
    out.lines().map(str::trim).filter(|l| !l.is_empty()).take(20).map(str::to_string).collect()
}

/// 为什么不能更新。`None` = 可以。
///
/// **只看已跟踪文件的改动**（调用方传 `git status --porcelain -uno` 的输出）：
/// 工作区里长期躺着未跟踪的设计稿，算上它们这条护栏永远是红的、等于没有。
pub fn blocked_reason(repo: Option<&Path>, dirty: &str, branch: &str) -> Option<String> {
    if repo.is_none() {
        return Some(
            "找不到 Anima 的源码仓库 —— 在仓库里跑一次 `anima install` 就会记下位置".into(),
        );
    }
    if !dirty.trim().is_empty() {
        let n = dirty.lines().filter(|l| !l.trim().is_empty()).count();
        return Some(format!("仓库里有 {n} 个文件被改过还没提交，先处理掉再更新"));
    }
    if branch != "main" && !branch.is_empty() {
        return Some(format!("当前在 {branch} 分支，不是 main —— 界面更新只走 main"));
    }
    None
}

/// 一行构建输出 → 阶段。`None` = 这行没意义，不用往界面上推。
///
/// **只认真正有信息的那几种**：全推的话一秒钟几十条，界面只会闪。
pub fn phase_of(line: &str) -> Option<(&'static str, String)> {
    let l = line.trim();
    if l.is_empty() {
        return None;
    }
    if let Some(rest) = l.strip_prefix("Compiling ") {
        return Some(("cargo", format!("编译 {}", rest.split_whitespace().next().unwrap_or(rest))));
    }
    if l.starts_with("Finished") && l.contains("release") {
        return Some(("cargo", "Rust 编译完成".into()));
    }
    if l.contains("vite v") || l.contains("building for production") {
        return Some(("frontend", "构建前端".into()));
    }
    if l.contains("built in") {
        return Some(("frontend", "前端构建完成".into()));
    }
    if l.contains("Bundling") {
        return Some(("bundle", "打包 .app".into()));
    }
    if l.contains("Finished") && l.contains("bundle") {
        return Some(("bundle", "打包完成".into()));
    }
    if l.starts_with("error") || l.contains("error:") {
        return Some(("failed", l.chars().take(200).collect()));
    }
    None
}

/// 一行输出该不该进日志窗。`None` = 纯噪音。
///
/// **不是全推。** rustc 的一条 warning 会带出五六行画箭头的上下文
/// （`  |`、`  = note:`、`^^^^`），全推的话日志窗里全是竖线，
/// 真正有用的那一行一闪而过。
pub fn log_line(line: &str) -> Option<String> {
    let t = line.trim();
    if t.is_empty() {
        return None;
    }
    // rustc 诊断的续行：竖线、脱字号、note/help 缩进块
    if t.starts_with('|') || t.starts_with('^') || t.starts_with("= note:") || t.starts_with("= help:") {
        return None;
    }
    // `12 | some code` 这种带行号的代码片段
    if t.split_once(" | ").is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())) {
        return None;
    }
    // 纯箭头指位行 `--> src/x.rs:1:2` 留着，它说明是哪个文件
    Some(t.chars().take(300).collect())
}

/// 换包并重启用的小脚本。
///
/// # 为什么要一个脚本
///
/// **应用不能覆盖正在运行的自己** —— 覆盖之后当前进程的可执行文件被换掉，
/// 行为是未定义的，而且下次启动大概率起不来（`anima install` 因此直接拒绝）。
/// 所以只能：脱离进程启动一个脚本 → 本进程退出 → 脚本换包 → 重新打开。
///
/// # 三条必须有的保险
///
/// 1. **等进程真的退出再动手**，而且等不到就什么都不做
/// 2. **旧包先挪走不是删**，留条回头路
/// 3. **拷失败要把旧包挪回来** —— 否则 `/Applications` 里什么都没有了，
///    用户连退回上一版都做不到
///
/// # 顺带回收
///
/// 每换一次包就留一个 102 MB 的旧 `.app`，不清的话一天就上 G。
/// 它们**是可再生的**（随时能重新构建），唯一用途是「刚换的坏了，退回上一个」，
/// 所以只留最近 [`APP_BACKUP_KEEP`] 个。
///
/// **只清 `.app` 备份** —— 数据备份（`anima backup` 的 tar.gz）一个都不碰，
/// 那是唯一的回头路，删了就没了。
pub fn relaunch_script(pid: u32, bundle: &Path, installed: &Path, keep_dir: &Path) -> String {
    format!(
        r#"#!/bin/bash
# Anima 换包脚本 —— 由应用自己写出来、脱离进程启动。
# 应用不能覆盖正在运行的自己，所以这件事只能在它退出之后做。
set -u
SRC={src}
DST={dst}
KEEP={keep_dir}
PID={pid}

# 1. 等它退出。**等不到就什么都不做** —— 覆盖一个还在跑的 .app 会让它下次起不来
for _ in $(seq 1 150); do
  kill -0 "$PID" 2>/dev/null || break
  sleep 0.2
done
if kill -0 "$PID" 2>/dev/null; then
  exit 1
fi

[ -d "$SRC" ] || exit 1
mkdir -p "$KEEP"
STAMP=$(date +%Y%m%d-%H%M%S)
BACKUP="$KEEP/Anima-$STAMP.app"

# 2. 旧包先挪走，不是删
if [ -d "$DST" ]; then
  mv "$DST" "$BACKUP" || exit 1
fi

# 3. 拷失败要挪回来，否则 /Applications 里就什么都没有了
if ! cp -R "$SRC" "$DST"; then
  rm -rf "$DST"
  [ -d "$BACKUP" ] && mv "$BACKUP" "$DST"
  exit 1
fi

open "$DST"

# 4. 回收旧的 .app 备份，只留最近 {keep} 个。
#    **只删 Anima-*.app** —— 同目录下别的东西一律不碰
ls -dt "$KEEP"/Anima-*.app 2>/dev/null | tail -n +$(({keep} + 1)) | while IFS= read -r old; do
  [ -n "$old" ] && rm -rf "$old"
done
"#,
        src = shell_quote(bundle),
        dst = shell_quote(installed),
        keep_dir = shell_quote(keep_dir),
        pid = pid,
        keep = APP_BACKUP_KEEP,
    )
}

/// 旧 `.app` 备份保留几个。**和 `anima.sh` 的 `APP_BACKUP_KEEP` 是同一个数**，
/// 两条换包路径（CLI 和界面）回收行为要一致。
pub const APP_BACKUP_KEEP: usize = 3;

/// 路径塞进 shell 脚本要加引号 —— 家目录里有空格或中文是常事。
fn shell_quote(p: &Path) -> String {
    format!("'{}'", p.to_string_lossy().replace('\'', r"'\''"))
}

// ---------------------------------------------------------------------------
// 命令
// ---------------------------------------------------------------------------

fn git(repo: &Path, args: &[&str]) -> String {
    std::process::Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

#[tauri::command]
pub async fn updater_check(state: tauri::State<'_, crate::AppState>) -> crate::error::Result<CheckResult> {
    let repo = find_repo(&state.config_dir);
    let Some(repo) = repo else {
        return Ok(CheckResult {
            blocked: blocked_reason(None, "", ""),
            ..Default::default()
        });
    };
    let branch = git(&repo, &["rev-parse", "--abbrev-ref", "HEAD"]).trim().to_string();
    let dirty = git(&repo, &["status", "--porcelain", "-uno"]);

    // **先算「跑的构建 vs 本地代码」那一段** —— 它和能不能更新无关，
    // 就算 blocked（工作区脏之类）也照样要告诉用户「你在跑旧构建」
    let running_sha = env!("ANIMA_GIT_SHA").to_string();
    let running_dirty = running_sha.ends_with("-dirty");
    let bare = running_sha.trim_end_matches("-dirty");
    let (behind_built, built_commits) = built_gap(&repo, bare);
    let head_sha = git(&repo, &["rev-parse", "--short", "HEAD"]).trim().to_string();
    // 只在「跑的不是 HEAD」时才去翻包 —— 跑的就是 HEAD 的话没有要换的，也省得读 150 MB
    let bundle_ready = (behind_built > 0 || running_dirty) && bundle_built_from(&bundle_binary(&repo), &head_sha);

    let blocked = blocked_reason(Some(&repo), &dirty, &branch);
    if blocked.is_some() {
        return Ok(CheckResult {
            repo: Some(repo.to_string_lossy().to_string()),
            branch,
            blocked,
            running_sha,
            running_dirty,
            behind_built,
            built_commits,
            head_sha,
            bundle_ready,
            ..Default::default()
        });
    }
    // 真的去问远端。没网时它会失败，behind 就是 0 —— 不会误报「有更新」
    let _ = git(&repo, &["fetch", "--quiet", "origin", &branch]);
    let range = format!("HEAD..origin/{branch}");
    let behind = parse_behind(&git(&repo, &["rev-list", "--count", &range]));
    let commits = if behind > 0 {
        parse_commits(&git(&repo, &["log", "--oneline", "--no-decorate", "-20", &range]))
    } else {
        Vec::new()
    };
    Ok(CheckResult {
        repo: Some(repo.to_string_lossy().to_string()),
        branch,
        behind,
        commits,
        running_sha,
        running_dirty,
        behind_built,
        built_commits,
        head_sha,
        bundle_ready,
        blocked: None,
    })
}

/// **构建戳**：`ANIMA-BUILD-STAMP:<sha>;`（脏的是 `<sha>-dirty;`）。
///
/// 为什么要单独一段、为什么是 `#[used]` 静态量，见模块注释「第四个点」：
/// 短字符串会被拆进指令里，搜不到；静态量引用的字面量必须在常量区里有完整一份，
/// `#[used]` 保证链接时不被当成没人用而丢掉。
#[used]
pub static BUILD_STAMP: &str = concat!("ANIMA-BUILD-STAMP:", env!("ANIMA_GIT_SHA"), ";");

/// 某个提交的构建戳（不含结尾，脏不脏都认）。
fn stamp_of(sha: &str) -> String {
    format!("ANIMA-BUILD-STAMP:{sha}")
}

/// 构建产物里的主程序。换包（[`updater_apply`]）换的就是它所在的 `.app`。
fn bundle_binary(repo: &Path) -> PathBuf {
    repo.join("src-tauri/target/release/bundle/macos/Anima.app/Contents/MacOS/anima")
}

/// 这个二进制是不是用 `sha` 那个提交编出来的：**看里面有没有那个提交的构建戳**（[`BUILD_STAMP`]）。
///
/// 读不到文件、sha 认不出、包是加构建戳之前编的（搜不到）都当「不是」——
/// 宁可多让人构建一次，也不要把一个旧包当成新的换上去。
pub fn bundle_built_from(binary: &Path, sha: &str) -> bool {
    if sha.len() < 7 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return false;
    }
    let Ok(bytes) = std::fs::read(binary) else { return false };
    let needle = stamp_of(sha);
    let needle = needle.as_bytes();
    bytes.windows(needle.len()).any(|w| w == needle)
}

/// 正在跑的构建和本地 HEAD 差多少。
///
/// **认不出那个 sha 时回 `(0, 空)`** —— 可能是没有 git 信息的构建、
/// 或者那个提交已经不在这个仓库里（改过历史）。这时候报一个瞎猜的数字
/// 比不报更糟。
fn built_gap(repo: &Path, sha: &str) -> (usize, Vec<String>) {
    if sha.is_empty() || sha == "unknown" {
        return (0, Vec::new());
    }
    // 先确认这个提交在不在本仓库里，不在就别往下算
    if git(repo, &["cat-file", "-t", sha]).trim() != "commit" {
        return (0, Vec::new());
    }
    let range = format!("{sha}..HEAD");
    let n = parse_behind(&git(repo, &["rev-list", "--count", &range]));
    let commits = if n > 0 {
        parse_commits(&git(repo, &["log", "--oneline", "--no-decorate", "-20", &range]))
    } else {
        Vec::new()
    };
    (n, commits)
}

/// 拉代码 + 构建。**不换包** —— 换包是 [`updater_apply`]，要先退出应用。
#[tauri::command]
pub async fn updater_build(app: tauri::AppHandle, state: tauri::State<'_, crate::AppState>) -> crate::error::Result<()> {
    let Some(repo) = find_repo(&state.config_dir) else {
        return Err(crate::error::Error::config("找不到源码仓库"));
    };
    let branch = git(&repo, &["rev-parse", "--abbrev-ref", "HEAD"]).trim().to_string();
    let dirty = git(&repo, &["status", "--porcelain", "-uno"]);
    if let Some(why) = blocked_reason(Some(&repo), &dirty, &branch) {
        return Err(crate::error::Error::config(why));
    }

    let started = std::time::Instant::now();
    let emit = |phase: &'static str, label: String, crates: usize| {
        let _ = app.emit(
            EVENT,
            Progress { phase, label, crates, line: String::new(), elapsed: started.elapsed().as_secs() },
        );
    };

    emit("git", "拉取新代码".into(), 0);
    // **--ff-only**：不产生合并提交。真分叉了停下来让人看
    let pull = std::process::Command::new("git")
        .current_dir(&repo)
        .args(["pull", "--ff-only", "origin", &branch])
        .output()
        .map_err(|e| crate::error::Error::runtime(format!("跑不了 git：{e}")))?;
    if !pull.status.success() {
        let err = String::from_utf8_lossy(&pull.stderr).chars().take(400).collect::<String>();
        emit("failed", format!("拉代码失败：{err}"), 0);
        return Err(crate::error::Error::runtime(format!("拉代码失败：{err}")));
    }

    emit("frontend", "开始构建（十几分钟）".into(), 0);
    let repo2 = repo.clone();
    let app2 = app.clone();

    // **心跳。** 链接那几分钟 cargo 一个字都不输出，没有它界面就是死的。
    // 2 秒一次 —— 再密没有信息量，再疏就看不出「秒数在走」
    let beat_app = app.clone();
    let beat_stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let beat_flag = std::sync::Arc::clone(&beat_stop);
    std::thread::spawn(move || {
        while !beat_flag.load(std::sync::atomic::Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_secs(2));
            if beat_flag.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            let _ = beat_app.emit(
                EVENT,
                Progress {
                    phase: "tick",
                    label: String::new(),
                    crates: 0,
                    line: String::new(),
                    elapsed: started.elapsed().as_secs(),
                },
            );
        }
    });

    tauri::async_runtime::spawn_blocking(move || {
        let emit_raw = |phase: &'static str, label: String, crates: usize, line: String| {
            let _ = app2.emit(
                EVENT,
                Progress { phase, label, crates, line, elapsed: started.elapsed().as_secs() },
            );
        };
        let emit = |phase: &'static str, label: String, crates: usize| emit_raw(phase, label, crates, String::new());
        let mut child = match std::process::Command::new("npm")
            .current_dir(&repo2)
            .args(["run", "tauri:build", "--", "--bundles", "app"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                emit("failed", format!("起不了 npm：{e}（PATH 里没有 node？）"), 0);
                return;
            }
        };
        // **cargo 的进度全在 stderr**，只读 stdout 的话界面十分钟一动不动
        let mut crates = 0usize;
        if let Some(err) = child.stderr.take() {
            use std::io::BufRead;
            let mut last_phase = "cargo";
            for line in std::io::BufReader::new(err).lines().map_while(Result::ok) {
                if let Some((phase, label)) = phase_of(&line) {
                    if phase == "cargo" && label.starts_with("编译 ") {
                        crates += 1;
                    }
                    last_phase = phase;
                    emit_raw(phase, label, crates, line.trim().to_string());
                } else if let Some(raw) = log_line(&line) {
                    // 有输出但不构成阶段变化 —— 只推给日志窗，阶段保持不变
                    emit_raw(last_phase, String::new(), crates, raw);
                }
            }
            // stderr 读完 = 编译输出结束，接下来是链接和打包，那段完全静默
            emit("link", "链接（这一段最长，而且没有任何输出）".into(), crates);
        }
        let r = child.wait();
        beat_stop.store(true, std::sync::atomic::Ordering::Relaxed);
        match r {
            Ok(s) if s.success() => {
                // **清理放在构建成功之后。** 一次构建会刷新它真正用到的每个
                // 产物的 mtime，所以这时候「很久没被碰过」这条判据才成立；
                // 放在构建之前的话，判不出这一轮到底用了哪些。
                //
                // 用户 2026-09-20：构建垃圾攒到 190 GB，自己手动清的。
                // cargo 从不回收旧版本依赖的产物，换包脚本也只挪不删。
                let note = crate::build_gc::sweep_after_build(&repo2, &backups_dir());
                emit_raw("done", "构建完成，可以换包了".into(), crates, note);
            }
            Ok(s) => emit("failed", format!("构建失败（退出码 {:?}）", s.code()), crates),
            Err(e) => emit("failed", format!("构建没跑完：{e}"), crates),
        }
    });
    Ok(())
}

/// 旧 `.app` 挪到哪。换包和清理**必须读同一个** —— 各写一份的话
/// 清理会去扫一个空目录，而备份在另一处接着攒。
pub fn backups_dir() -> std::path::PathBuf {
    dirs::home_dir().unwrap_or_default().join("AnimaBackups/apps")
}

/// 换包并重启。**会退出当前应用** —— 调用方要先跟用户说清楚。
#[tauri::command]
pub async fn updater_apply(app: tauri::AppHandle, state: tauri::State<'_, crate::AppState>) -> crate::error::Result<()> {
    let Some(repo) = find_repo(&state.config_dir) else {
        return Err(crate::error::Error::config("找不到源码仓库"));
    };
    let bundle = repo.join("src-tauri/target/release/bundle/macos/Anima.app");
    if !bundle.is_dir() {
        return Err(crate::error::Error::config("还没有构建产物 —— 先点「下载并构建」"));
    }
    // 装在哪就换哪。**不写死 /Applications** —— 用户可能放在别处
    let installed = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")).map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("/Applications/Anima.app"));
    let keep = backups_dir();

    let script = relaunch_script(std::process::id(), &bundle, &installed, &keep);
    let path = state.config_dir.join("apply-update.sh");
    std::fs::write(&path, script).map_err(|e| crate::error::Error::runtime(format!("写不了换包脚本：{e}")))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
    }
    // **脱离进程启动** —— 它要在本进程死掉之后继续跑
    std::process::Command::new("/bin/bash")
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| crate::error::Error::runtime(format!("起不了换包脚本：{e}")))?;

    app.exit(0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 读不出来当 0 = **没有更新**，不是「有更新」——
    /// 宁可少提示一次，也不要让人白等十几分钟。
    /// 构建好的包认得出是不是当前 HEAD 编的（2026-09-28：构建完再点检查更新，只剩「重新构建」）
    #[test]
    fn 构建好的包认得出是不是当前代码编的() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("anima");
        std::fs::write(&bin, b"\x00\x01junkANIMA-BUILD-STAMP:929e9d83-dirty;UTC 85221d96 more junk").unwrap();
        assert!(bundle_built_from(&bin, "929e9d83"), "构建戳在里面：是这个提交编的（脏的也认）");
        assert!(!bundle_built_from(&bin, "85221d96"), "光有 sha、没有构建戳的不算 —— 可能只是别的数据碰巧长这样");
        assert!(!bundle_built_from(&bin, "929e"), "太短认不出，当不是");
        assert!(!bundle_built_from(&bin, "unknown"), "不是 sha");
        assert!(!bundle_built_from(&dir.path().join("没有"), "929e9d83"), "没构建过");
        // **这份测试二进制自己**里就要搜得到自己的构建戳 —— 证明 `#[used]` 静态量真的留在了产物里
        let me = std::env::current_exe().unwrap();
        let sha = env!("ANIMA_GIT_SHA").trim_end_matches("-dirty");
        if sha != "unknown" {
            assert!(bundle_built_from(&me, sha), "构建戳没进二进制：{}", BUILD_STAMP);
        }
    }

    #[test]
    fn 落后数读不出来时当没有更新() {
        assert_eq!(parse_behind("7\n"), 7);
        assert_eq!(parse_behind("  0 "), 0);
        assert_eq!(parse_behind(""), 0);
        assert_eq!(parse_behind("fatal: 不是仓库"), 0);
    }

    #[test]
    fn 提交列表最多二十条且去空行() {
        let many = (0..30).map(|i| format!("abc{i} 第 {i} 个提交")).collect::<Vec<_>>().join("\n");
        let c = parse_commits(&format!("\n{many}\n\n"));
        assert_eq!(c.len(), 20);
        assert!(c[0].contains("第 0 个提交"));
    }

    /// **认不出那个 sha 时回 0**，不是瞎猜一个数字。
    ///
    /// 可能是没有 git 信息的构建，也可能那个提交已经不在仓库里（改过历史）。
    /// 这时候报一个编出来的数字比不报更糟 —— 用户会照着它去重新构建，
    /// 然后发现什么都没变。
    #[test]
    fn 认不出构建的sha时不瞎猜() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(built_gap(d.path(), ""), (0, Vec::new()));
        assert_eq!(built_gap(d.path(), "unknown"), (0, Vec::new()));
        // 不是 git 仓库 → cat-file 失败 → 回 0
        assert_eq!(built_gap(d.path(), "deadbeef"), (0, Vec::new()));
    }

    /// 四条拒绝理由都要**说清楚为什么**，不能只说「不能更新」。
    #[test]
    fn 拒绝更新时说得出原因() {
        let repo = PathBuf::from("/repo");
        assert!(blocked_reason(None, "", "main").unwrap().contains("anima install"));
        let dirty = blocked_reason(Some(&repo), " M a.rs\n M b.rs\n", "main").unwrap();
        assert!(dirty.contains("2 个文件"), "没说有几个：{dirty}");
        assert!(blocked_reason(Some(&repo), "", "agent/anti").unwrap().contains("agent/anti"));
        assert!(blocked_reason(Some(&repo), "", "main").is_none());
        // 空分支名（读不出来）不该被当成「不是 main」而拦下
        assert!(blocked_reason(Some(&repo), "", "").is_none());
    }

    /// 只有真正有信息的行才往界面推 —— 全推的话一秒几十条，界面只会闪。
    #[test]
    fn 只推有意义的构建输出() {
        assert_eq!(phase_of("   Compiling serde v1.0.2").unwrap().0, "cargo");
        assert_eq!(phase_of("   Compiling serde v1.0.2").unwrap().1, "编译 serde");
        assert_eq!(phase_of("    Finished `release` profile [optimized]").unwrap().0, "cargo");
        assert_eq!(phase_of("    Bundling Anima.app (/x/y)").unwrap().0, "bundle");
        assert_eq!(phase_of("vite v7.0.0 building for production...").unwrap().0, "frontend");
        assert_eq!(phase_of("✓ built in 3.7s").unwrap().0, "frontend");
        assert_eq!(phase_of("error: could not compile `anima`").unwrap().0, "failed");
        // 噪音
        assert!(phase_of("").is_none());
        assert!(phase_of("    Downloaded quote v1.0").is_none());
        assert!(phase_of("warning: unused import").is_none());
    }

    /// **不是全推。** rustc 一条 warning 会带出五六行画箭头的上下文，
    /// 全推的话日志窗里全是竖线，真正有用的那一行一闪而过。
    #[test]
    fn 日志窗滤掉诊断的续行() {
        // 留着
        for keep in [
            "   Compiling anima v0.9.0",
            "warning: unused import: `Manager`",
            "  --> src/updater.rs:49:22",
            "    Finished `release` profile",
        ] {
            assert!(log_line(keep).is_some(), "不该滤掉：{keep}");
        }
        // 滤掉
        for drop in ["", "   ", "   |", "   ^^^^^^^", "   = note: `#[warn(unused)]` on by default", "49 | use tauri::{Emitter, Manager};"] {
            assert!(log_line(drop).is_none(), "该滤掉：{drop:?}");
        }
    }

    /// 超长行要截断 —— 一行几千字符会把日志窗撑坏。
    #[test]
    fn 超长行截断() {
        let long = "x".repeat(5000);
        assert_eq!(log_line(&long).unwrap().chars().count(), 300);
    }

    /// **链接那一段要单独报。** 它最长也最安静，不单独说的话
    /// 界面停在「已编译 N 个」不动，看起来和卡死一模一样。
    #[test]
    fn 打包完成也认得出() {
        assert_eq!(phase_of("    Finished 1 bundle at:").unwrap().0, "bundle");
    }

    /// **等不到退出就什么都不做。** 覆盖一个还在跑的 .app 会让它下次起不来。
    #[test]
    fn 换包脚本要先等进程退出() {
        let s = relaunch_script(1234, Path::new("/b/Anima.app"), Path::new("/Applications/Anima.app"), Path::new("/keep"));
        assert!(s.contains("PID=1234"));
        assert!(s.contains("kill -0 \"$PID\""), "没等进程退出");
        let wait = s.find("kill -0").unwrap();
        let copy = s.find("cp -R").unwrap();
        assert!(wait < copy, "等待要排在拷贝之前");
        assert!(s.contains("exit 1"), "等不到要放弃");
    }

    /// **拷失败要把旧包挪回来** —— 否则 /Applications 里什么都没有了，
    /// 用户连退回上一版都做不到。
    #[test]
    fn 换包失败要能回滚() {
        let s = relaunch_script(1, Path::new("/b/Anima.app"), Path::new("/Applications/Anima.app"), Path::new("/keep"));
        let fail = s.find("if ! cp -R").expect("没有失败分支");
        let restore = s[fail..].find(r#"mv "$BACKUP" "$DST""#).expect("失败了没挪回来");
        assert!(restore > 0);
        // 旧包是挪走不是删
        assert!(s.contains(r#"mv "$DST" "$BACKUP""#), "旧包被直接删了");
    }

    /// 家目录里有空格或中文是常事，路径必须带引号。
    #[test]
    fn 路径带空格也不会拆开() {
        let s = relaunch_script(1, Path::new("/有 空格/Anima.app"), Path::new("/A/Anima.app"), Path::new("/k"));
        assert!(s.contains("SRC='/有 空格/Anima.app'"), "路径没加引号：{s}");
        // 单引号也要转义掉
        let q = shell_quote(Path::new("/it's/here"));
        assert_eq!(q, r"'/it'\''s/here'");
    }
}
