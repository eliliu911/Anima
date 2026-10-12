//! Git Worktree Manager for Isolated Subagent Sandboxes (P5)
//!
//! Provides isolated physical workspace directories powered by git worktree.
//! Enables subagents to perform refactorings, test modifications, and build experiments
//! without polluting or dirtying the user's primary working tree.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct WorktreeSession {
    pub task_id: String,
    pub base_branch: String,
    pub branch_name: String,
    pub worktree_path: PathBuf,
    pub is_active: bool,
}

#[derive(Default)]
pub struct WorktreeManager {
    // 维护活跃的 worktree 沙箱会话
    sessions: Arc<Mutex<Vec<WorktreeSession>>>,
}

/// 同时存在的沙箱上限。每个沙箱是仓库的一份完整 checkout，必须有个天花板。
const MAX_ACTIVE_SANDBOXES: usize = 4;

/// 列出仓库里所有 Anima 建的沙箱 —— 无论是不是本进程建的。
///
/// 需要它是因为孤儿沙箱**会真的困住用户**：Demo 里就留下过一个
/// `cache-feature`，来自一次模型跟命令闸门较劲、用 git plumbing 搞出来的残局。
/// 之后用户每次问「列出当前挂起的更改」，模型都会跑去翻那个沙箱，
/// 报一堆和用户代码毫无关系的东西 —— 用户的原话是
/// 「沙箱挂不挂起跟我实际代码有什么关系」。
///
/// 光靠提示词说「别看 .anima/」不够：那依赖模型听话，而依从性因模型而异
/// （见 `commands.rs` 里那张对照表）。所以还要有一条不依赖模型的路：能列出来、能收掉。
pub fn list_sandboxes(repo_root: &Path) -> Vec<String> {
    let Ok(out) = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(repo_root)
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.strip_prefix("worktree "))
        .filter(|p| p.contains("/.anima/worktrees/"))
        .filter_map(|p| p.rsplit('/').next().map(str::to_string))
        .collect()
}

impl WorktreeManager {
    /// 检查指定路径是否属于合法的 git 仓库根目录
    pub fn is_git_repo(repo_root: &Path) -> bool {
        repo_root.join(".git").exists()
    }

    /// 获取仓库当前 HEAD 所在的分支名
    pub fn current_branch(repo_root: &Path) -> Result<String, String> {
        let output = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(repo_root)
            .output()
            .map_err(|e| format!("获取当前分支失败: {e}"))?;

        if !output.status.success() {
            return Err("获取当前 Git HEAD 分支名失败".to_string());
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// 确保目标仓库的 `.git/info/exclude` 中忽略 `.anima/worktrees/`，避免污染主仓库与用户跟踪的 `.gitignore`
    fn ensure_local_exclude(repo_root: &Path) {
        // `info/exclude` 永远位于 **common dir**：普通仓库里就是 `.git/`，
        // 而当 repo_root 自身是一个 linked worktree 时，它的 `.git` 是个**文件**
        // （内容形如 `gitdir: /path/to/main/.git/worktrees/<name>`），
        // 真正生效的 exclude 仍在主仓库的 `.git/info/exclude` 里。
        // 所以不要手工拼路径 —— 直接问 git 要答案。
        let common_dir = match Command::new("git")
            .args(["rev-parse", "--git-common-dir"])
            .current_dir(repo_root)
            .output()
        {
            Ok(out) if out.status.success() => {
                let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if raw.is_empty() {
                    return;
                }
                let path = PathBuf::from(&raw);
                // git 在仓库根目录下会回一个相对路径（`.git`），需要拼回绝对路径
                if path.is_absolute() {
                    path
                } else {
                    repo_root.join(path)
                }
            }
            _ => return,
        };
        let info_dir = common_dir.join("info");
        let exclude_file = info_dir.join("exclude");
        let _ = std::fs::create_dir_all(&info_dir);

        let entry = ".anima/worktrees/";
        let current_content = std::fs::read_to_string(&exclude_file).unwrap_or_default();
        if !current_content.contains(entry) {
            let mut updated = current_content;
            if !updated.is_empty() && !updated.ends_with('\n') {
                updated.push('\n');
            }
            updated.push_str(entry);
            updated.push('\n');
            let _ = std::fs::write(&exclude_file, updated);
        }
    }

    /// 创建一个独立的 Git Worktree 沙箱环境
    pub async fn create_sandbox(
        &self,
        repo_root: &Path,
        task_id: &str,
    ) -> Result<WorktreeSession, String> {
        if !Self::is_git_repo(repo_root) {
            return Err("当前工作区不是一个有效的 Git 仓库，无法创建 Worktree 沙箱".to_string());
        }

        // 每个沙箱都是仓库的一份完整 checkout。没有上限的话，模型多派几次子智能体
        // 就会把 `.anima/worktrees/` 堆满 —— 而且不报任何错，只是磁盘悄悄见底。
        // （`pty_session` 的 `MAX_PTY_SESSIONS` 是同一个道理。）
        {
            let live = self.sessions.lock().await.len();
            if live >= MAX_ACTIVE_SANDBOXES {
                return Err(format!(
                    "活跃沙箱已达上限 {MAX_ACTIVE_SANDBOXES} 个。\
                     请先用 worktree_merge 合入或 worktree_discard 丢弃已完成的沙箱。"
                ));
            }
        }

        // P1: 确保本地 exclude 配置，避免 .anima/worktrees/ 污染主仓库状态
        Self::ensure_local_exclude(repo_root);

        // P2: 记录沙箱创建时的基线分支
        let base_branch = Self::current_branch(repo_root)?;

        let clean_id = task_id.replace(|c: char| !c.is_alphanumeric() && c != '-', "_");
        let branch_name = format!("anima/sandbox-{}", clean_id);
        let worktrees_dir = repo_root.join(".anima").join("worktrees");

        if let Err(e) = std::fs::create_dir_all(&worktrees_dir) {
            return Err(format!("创建 worktrees 根目录失败: {e}"));
        }

        let worktree_path = worktrees_dir.join(&clean_id);

        // 如果历史残留存在，先做一次安全清理
        if worktree_path.exists() {
            let _ = Self::remove_worktree_internal(repo_root, &worktree_path, &branch_name);
        }

        // git worktree add .anima/worktrees/<task_id> -b anima/sandbox-<task_id>
        let output = Command::new("git")
            .arg("worktree")
            .arg("add")
            .arg(&worktree_path)
            .arg("-b")
            .arg(&branch_name)
            .current_dir(repo_root)
            .output()
            .map_err(|e| format!("执行 git worktree add 失败: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // 若分支已存在，尝试不带 -b 直接 checkout
            let retry = Command::new("git")
                .arg("worktree")
                .arg("add")
                .arg(&worktree_path)
                .arg(&branch_name)
                .current_dir(repo_root)
                .output()
                .map_err(|e| format!("重试 git worktree add 失败: {e}"))?;

            if !retry.status.success() {
                return Err(format!(
                    "创建 Git Worktree 失败: {}",
                    stderr.trim()
                ));
            }
        }

        let session = WorktreeSession {
            task_id: task_id.to_string(),
            base_branch,
            branch_name,
            worktree_path,
            is_active: true,
        };

        let mut lock = self.sessions.lock().await;
        lock.push(session.clone());

        Ok(session)
    }

    /// 检查仓库当前是否处于未解决的 MERGING 状态
    pub fn is_merging(repo_root: &Path) -> bool {
        repo_root.join(".git").join("MERGE_HEAD").exists()
    }

    /// 将影子沙箱内的改动合并到主分支
    pub async fn merge_sandbox(
        &self,
        repo_root: &Path,
        task_id: &str,
    ) -> Result<String, String> {
        let session = {
            let lock = self.sessions.lock().await;
            lock.iter()
                .find(|s| s.task_id == task_id)
                .cloned()
                .ok_or_else(|| format!("未找到 task_id 为 {task_id} 的 Worktree 沙箱"))?
        };

        // P2: 基线分支校验 —— 确保当前主分支与创建沙箱时的分支一致
        let current_branch = Self::current_branch(repo_root)?;
        if current_branch != session.base_branch {
            return Err(format!(
                "基线分支不匹配：沙箱创建于分支 `{}`，但当前主仓库处于分支 `{}`，拒绝静默合并",
                session.base_branch, current_branch
            ));
        }

        // 1. 在 worktree 目录内提交所有变更
        let _ = Command::new("git")
            .arg("add")
            .arg(".")
            .current_dir(&session.worktree_path)
            .output();

        let _ = Command::new("git")
            .arg("commit")
            .arg("-m")
            .arg(format!("feat(agent): auto commit sandbox changes for {}", task_id))
            .current_dir(&session.worktree_path)
            .output();

        // 2. 回到主仓库执行 merge
        // ── 先在**沙箱里** rebase 到主分支，把冲突挡在沙箱内 ──
        //
        // 这一步是并行子智能体能用的前提。`isolated` 模式存在的理由就是同时铺开多个
        // 会改文件的子任务，它们最后都要合回同一条 main —— 而这些子任务本来就是从
        // 同一个大任务拆的，摸同一批文件是常态，第二个往后的每次 merge 都可能冲突。
        //
        // 不 rebase 直接 merge，冲突就落在**主工作区**：带着冲突标记卡在 MERGING，
        // 而沙箱的全部卖点恰恰是主分支干净。
        //
        // rebase 之后沙箱分支就在 main 的正上方，下面那次 merge 必然无冲突。
        // rebase 自己失败的话，冲突留在沙箱里，主工作区一个字节都没动过 ——
        // 而且影子分支保留，可以人工处理或让子智能体再跑一轮。
        let rebase_out = Command::new("git")
            .args(["rebase", &session.base_branch])
            .current_dir(&session.worktree_path)
            .output()
            .map_err(|e| format!("沙箱 rebase 失败: {e}"))?;

        if !rebase_out.status.success() {
            // 收拾干净，别让沙箱卡在 REBASING —— 否则下一轮连 git status 都读不准
            let _ = Command::new("git")
                .args(["rebase", "--abort"])
                .current_dir(&session.worktree_path)
                .output();
            let detail = String::from_utf8_lossy(&rebase_out.stderr);
            let short = detail.lines().take(6).collect::<Vec<_>>().join("\n");
            return Err(format!(
                "沙箱分支 {} 与 {} 有冲突，已挡在沙箱内 —— 主工作区未受任何影响。\n\
                 影子分支已保留，可人工处理后重试，或直接 worktree_discard 丢弃。\n{}",
                session.branch_name, session.base_branch, short
            ));
        }

        let merge_out = Command::new("git")
            .arg("merge")
            .arg("--no-ff")
            .arg(&session.branch_name)
            .arg("-m")
            .arg(format!("chore(agent): merge subagent sandbox {}", task_id))
            .current_dir(repo_root)
            .output()
            .map_err(|e| format!("合并沙箱分支失败: {e}"))?;

        if !merge_out.status.success() {
            let stderr = String::from_utf8_lossy(&merge_out.stderr);
            let stdout = String::from_utf8_lossy(&merge_out.stdout);

            // P0 防御：合并冲突时，立刻通过 git merge --abort 彻底回滚主仓库状态，确保零污染！
            let abort_out = Command::new("git")
                .args(["merge", "--abort"])
                .current_dir(repo_root)
                .output();

            // 若 merge --abort 失败，退回 reset --merge
            if abort_out.as_ref().map(|o| !o.status.success()).unwrap_or(true) {
                let _ = Command::new("git")
                    .args(["reset", "--merge"])
                    .current_dir(repo_root)
                    .output();
            }

            // 严禁在此处删除影子分支，保留分支供人工排查与重试
            return Err(format!(
                "合并沙箱分支发生冲突或失败：主工作区已安全自动回退至干净状态（未受污染），影子分支 `{}` 保持保留。\n详情：{}{}",
                session.branch_name, stdout, stderr
            ));
        }

        // 3. 合并成功后自动清理该 worktree
        let _ = self.discard_sandbox(repo_root, task_id).await;

        Ok(format!("成功将沙箱分支 {} 合入主分支", session.branch_name))
    }

    /// 丢弃并清理 Worktree 沙箱及影子分支
    pub async fn discard_sandbox(
        &self,
        repo_root: &Path,
        task_id: &str,
    ) -> Result<(), String> {
        // 如果主仓库当前处于 MERGING 状态，拒绝执行 discard，避免掩盖或破坏未完成的合并
        if Self::is_merging(repo_root) {
            return Err("主仓库当前处于冲突或 MERGING 状态，请先解决或中止合并再丢弃沙箱".to_string());
        }

        let maybe_session = {
            let mut lock = self.sessions.lock().await;
            lock.iter().position(|s| s.task_id == task_id).map(|pos| lock.remove(pos))
        };

        let clean_id = task_id.replace(|c: char| !c.is_alphanumeric() && c != '-', "_");
        let branch_name = format!("anima/sandbox-{}", clean_id);
        let worktree_path = repo_root.join(".anima").join("worktrees").join(&clean_id);

        if let Some(session) = maybe_session {
            Self::remove_worktree_internal(repo_root, &session.worktree_path, &session.branch_name)?;
        } else {
            Self::remove_worktree_internal(repo_root, &worktree_path, &branch_name)?;
        }

        Ok(())
    }

    fn remove_worktree_internal(
        repo_root: &Path,
        worktree_path: &Path,
        branch_name: &str,
    ) -> Result<(), String> {
        if worktree_path.exists() {
            let _ = Command::new("git")
                .arg("worktree")
                .arg("remove")
                .arg("--force")
                .arg(worktree_path)
                .current_dir(repo_root)
                .output();

            // 若 git worktree remove 未完全清理目录，做一次兜底删除
            if worktree_path.exists() {
                let _ = std::fs::remove_dir_all(worktree_path);
            }
        }

        let _ = Command::new("git")
            .arg("worktree")
            .arg("prune")
            .current_dir(repo_root)
            .output();

        let _ = Command::new("git")
            .arg("branch")
            .arg("-D")
            .arg(branch_name)
            .current_dir(repo_root)
            .output();

        Ok(())
    }
}

#[cfg(test)]
mod exclude_tests {
    use super::*;

    fn git(args: &[&str], cwd: &std::path::Path) {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("git 该能跑");
        assert!(out.status.success(), "git {args:?} 失败: {}", String::from_utf8_lossy(&out.stderr));
    }

    /// 回归：`repo_root` 自己就是一个 worktree 时，它的 `.git` 是**文件**而不是目录，
    /// 内容形如 `gitdir: /path/to/main/.git/worktrees/<name>`。
    /// 早先直接拼 `repo_root/.git/info`，`create_dir_all` 会失败而返回值被 `let _ =` 吞掉，
    /// 于是 exclude 静默不生效 —— 影子目录照样污染 `git status`，且没有任何报错。
    #[test]
    fn exclude_works_when_repo_root_is_itself_a_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let main = tmp.path().join("main");
        std::fs::create_dir_all(&main).unwrap();
        git(&["init", "-q", "."], &main);
        git(&["config", "user.email", "a@b.c"], &main);
        git(&["config", "user.name", "t"], &main);
        std::fs::write(main.join("f.txt"), "x").unwrap();
        git(&["add", "."], &main);
        git(&["commit", "-qm", "init"], &main);

        // 把「项目根」本身做成一个 worktree —— 此时它的 .git 是文件
        let nested = tmp.path().join("nested");
        git(&["worktree", "add", nested.to_str().unwrap(), "-b", "side"], &main);
        assert!(nested.join(".git").is_file(), "worktree 的 .git 应当是文件");

        WorktreeManager::ensure_local_exclude(&nested);

        // exclude 必须落在**真实 gitdir** 里，并且对 nested 生效
        let out = std::process::Command::new("git")
            .args(["check-ignore", "-q", ".anima/worktrees/x"])
            .current_dir(&nested)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "`.anima/worktrees/` 没有被本地 exclude 生效（repo_root 是 worktree 的情况）"
        );
    }
}


#[cfg(test)]
mod 孤儿沙箱 {
    use super::*;

    fn git(a: &[&str], cwd: &Path) {
        let _ = Command::new("git").args(a).current_dir(cwd).output();
    }

    /// 回归：能列出仓库里所有 Anima 沙箱，**包括不是本进程建的**。
    ///
    /// 孤儿沙箱是真出过事的：Demo 里留下一个 `cache-feature`（模型跟命令闸门较劲
    /// 时用 git plumbing 搞出来的残局），之后用户每问一次「列出当前挂起的更改」，
    /// 模型就跑去翻那个沙箱，报一堆和用户代码无关的东西。
    ///
    /// `WorktreeManager` 自己的 `sessions` 只记本进程建的，所以必须直接问 git。
    #[test]
    fn 能列出不是本进程建的沙箱() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git(&["init", "-q", "."], root);
        git(&["config", "user.email", "a@b.c"], root);
        git(&["config", "user.name", "t"], root);
        std::fs::write(root.join("f.txt"), "x").unwrap();
        git(&["add", "."], root);
        git(&["commit", "-qm", "init"], root);

        assert!(list_sandboxes(root).is_empty(), "干净仓库不该有沙箱");

        // 绕过 WorktreeManager 直接建 —— 模拟"上一次会话留下的孤儿"
        git(
            &["worktree", "add", ".anima/worktrees/orphan", "-b", "tmp-branch", "-q"],
            root,
        );
        assert_eq!(
            list_sandboxes(root),
            vec!["orphan".to_string()],
            "没能列出不是本进程建的沙箱"
        );

        // 正名：普通 worktree（不在 .anima/worktrees 下）不该被算成沙箱
        git(&["worktree", "add", "../normal-wt", "-b", "other", "-q"], root);
        assert_eq!(
            list_sandboxes(root),
            vec!["orphan".to_string()],
            "把普通 worktree 误认成沙箱了"
        );
    }
}
