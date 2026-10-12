//! 构建垃圾回收：每次构建完顺手把旧产物清掉。
//!
//! # 为什么需要
//!
//! 2026-09-20 用户发现构建垃圾攒到了 **190 GB**，自己手动清掉的。
//! 两处在攒，而且都不会自己停：
//!
//! | 在哪 | 攒的是什么 | 为什么不会自己停 |
//! |---|---|---|
//! | `src-tauri/target/{debug,release}` | 旧版本依赖编出来的 `.rlib`、指纹、增量缓存 | **cargo 从不回收**：依赖升级一次就多一份，旧的永远留着 |
//! | `~/AnimaBackups/apps` | 每次换包挪走的旧 `.app` | 换包脚本「挪走不删」是对的（拷贝失败要挪回来），但没人删 |
//!
//! # 判据是「这次构建没碰过」，不是「看着像垃圾」
//!
//! 一次构建会把**它真正用到的**每个产物的 mtime 刷新。所以构建成功之后，
//! `target` 里 mtime 还停在很久以前的那些，就是旧版本依赖留下的残骸。
//!
//! 这条判据的好处是**猜错也不会坏事**：真删错了，cargo 下次重编一遍就是了，
//! 代价是时间不是正确性。所以这里可以自动做，不用问用户。
//!
//! # 副本的编译缓存（2026-09-28 补）
//!
//! 用户：「硬盘从 300G 左右掉到了 240G……我记得生产环境里有更新后自动清理，没生效？」
//! 查下来 55 GB 在这里原本不扫的两处：一个后台任务的 worktree（`.claude/worktrees/*`，26 GB）
//! 和变异测试副本（`/tmp/anima-mut`，29 GB）。它们各自有一整份 `src-tauri/target`，
//! 而且都是**当天**长出来的 —— 30 天的判据清不到。
//!
//! 这些副本是一次性的：任务做完、变异跑完就没人再编译它。所以按「整个 target 两天没编译过」
//! 整份删掉（[`SIDE_STALE_DAYS`]）；还在用的每次编译都会刷新 `deps/` 的时间，不会被误删。
//! 变异副本从 2026-09-28 起改用主工作区的 target（`scripts/mut.sh`），这里清的是以前攒下的。
//!
//! # 绝不碰的
//!
//! - `target/release/bundle` —— 换包脚本要从这里拷 `.app`，删了就换不了包；
//! - `target/*` 下的可执行文件本身；
//! - 最近几份 `.app` 备份 —— 换包出问题时要靠它们回退。

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// 多久没被碰过就算残骸。
///
/// 30 天：比任何一次正常的依赖升级周期都长，而一个还在用的产物
/// **每次构建都会被刷新 mtime**，不可能躺够 30 天。
pub const STALE_DAYS: u64 = 30;

/// 留几份旧 `.app`。换包出问题时要靠它们回退，留三份够回退到上上个版本。
pub const KEEP_APPS: usize = 3;

/// 副本（任务 worktree、变异副本）的 `target` 多久没编译过就整份删。
pub const SIDE_STALE_DAYS: u64 = 2;

/// 副本的 `target` 在哪：`<repo>/.claude/worktrees/*/src-tauri/target`，以及变异副本的。
pub fn side_targets(repo: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(repo.join(".claude/worktrees")) {
        for e in entries.flatten() {
            let t = e.path().join("src-tauri/target");
            if t.is_dir() {
                out.push(t);
            }
        }
    }
    let mutw = std::env::var("ANIMA_MUT_WORKTREE").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/tmp/anima-mut"));
    let t = mutw.join("src-tauri/target");
    if t.is_dir() {
        out.push(t);
    }
    out
}

/// 一份 `target` 最后一次编译是什么时候：`debug/deps`、`release/deps` 和它们上一层里最新的那个修改时间
/// （每次编译都会往 `deps/` 里写新文件，目录的修改时间跟着变）。一个都读不到就当它是新的。
fn last_build(target: &Path) -> Option<SystemTime> {
    ["", "debug", "debug/deps", "release", "release/deps"]
        .iter()
        .filter_map(|sub| modified(&target.join(sub)))
        .max()
}

/// 副本里该整份删的 `target`。只规划不动手（和 [`plan`] 一样，能先测、先写进日志）
pub fn plan_side(targets: &[PathBuf], now: SystemTime, stale_days: u64) -> Sweep {
    let mut out = Sweep::default();
    let cutoff = now.checked_sub(Duration::from_secs(stale_days * 86_400));
    for t in targets {
        if is_stale(last_build(t), cutoff) {
            out.bytes += dir_size(t);
            out.paths.push(t.clone());
        }
    }
    out
}

/// `target` 下只在这几个子目录里清。**不是整个 target** ——
/// `bundle/` 是换包要读的，可执行文件本身也在上一层。
const SWEEPABLE: [&str; 5] = ["deps", "build", ".fingerprint", "incremental", "examples"];

#[derive(Debug, Default, PartialEq)]
pub struct Sweep {
    /// 要删的东西，按占地从大到小
    pub paths: Vec<PathBuf>,
    pub bytes: u64,
}

impl Sweep {
    /// 给人看的一句话。**零也要说** —— 什么都不显示的话，
    /// 用户不知道这一步到底跑没跑。
    pub fn note(&self) -> String {
        if self.paths.is_empty() {
            return "构建垃圾：没有可清的".into();
        }
        format!("清掉 {} 项构建垃圾，腾出 {}", self.paths.len(), human(self.bytes))
    }
}

pub fn human(bytes: u64) -> String {
    const G: u64 = 1024 * 1024 * 1024;
    const M: u64 = 1024 * 1024;
    if bytes >= G {
        format!("{:.1} GB", bytes as f64 / G as f64)
    } else {
        format!("{} MB", bytes / M)
    }
}

fn dir_size(p: &Path) -> u64 {
    let Ok(md) = std::fs::symlink_metadata(p) else { return 0 };
    if !md.is_dir() {
        return md.len();
    }
    std::fs::read_dir(p)
        .map(|it| it.flatten().map(|e| dir_size(&e.path())).sum())
        .unwrap_or(0)
}

fn modified(p: &Path) -> Option<SystemTime> {
    std::fs::symlink_metadata(p).ok()?.modified().ok()
}

/// 这一项算不算残骸。
///
/// **抽出来是为了测得到。** 「读不出修改时间就当它是新的」这条分支，
/// 靠摆文件试不出来：`SystemTime` 在 Unix 上能表示负值，
/// 拿 `UNIX_EPOCH` 当 now 也减得动 30 天，于是那个分支永远走不到 ——
/// 2026-09-20 变异测试当场抓出这是条假绿测试。
fn is_stale(modified: Option<SystemTime>, cutoff: Option<SystemTime>) -> bool {
    match (modified, cutoff) {
        (Some(m), Some(c)) => m < c,
        // **宁可少清，不可误删。** 判不出来就别动它
        _ => false,
    }
}

/// 这次该清什么。**只规划不动手** —— 分开是为了测得到，
/// 也为了「要删什么」能先写进日志给用户看。
pub fn plan(repo: &Path, keep_dir: &Path, now: SystemTime, stale_days: u64, keep_apps: usize) -> Sweep {
    let mut out = Sweep::default();
    let cutoff = now.checked_sub(Duration::from_secs(stale_days * 86_400));

    for profile in ["debug", "release"] {
        for sub in SWEEPABLE {
            let dir = repo.join("src-tauri/target").join(profile).join(sub);
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for e in entries.flatten() {
                let p = e.path();
                // **这次构建碰过的一律不动。** 判据只有 mtime，没有别的猜测
                if is_stale(modified(&p), cutoff) {
                    out.bytes += dir_size(&p);
                    out.paths.push(p);
                }
            }
        }
    }

    // 旧 .app 备份：留最近几份。
    //
    // **按文件名排，不按 mtime。** 换包脚本起的名是
    // `Anima-YYYYmmdd-HHMMSS.app`，名字顺序就是时间顺序；而 mtime 是
    // `cp -R` 拷出来的，不保证等于换包那一刻。
    //
    // 也**不看 30 天**：一天换三次包的话三份都是今天的，按天数算一份都清不掉，
    // 而它们恰恰是最占地方的（一份 .app 一百多 MB）。
    if let Ok(entries) = std::fs::read_dir(keep_dir) {
        let mut apps: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "app"))
            .collect();
        apps.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
        for p in apps.into_iter().skip(keep_apps) {
            out.bytes += dir_size(&p);
            out.paths.push(p);
        }
    }

    out
}

/// 动手。回**真正删掉**的数量和字节数 —— 删不掉的（权限、正被占用）跳过，
/// 不报错：清理失败不该让一次成功的构建看起来像失败了。
pub fn run(sweep: &Sweep) -> Sweep {
    let mut done = Sweep::default();
    for p in &sweep.paths {
        let size = dir_size(p);
        let ok = if p.is_dir() { std::fs::remove_dir_all(p).is_ok() } else { std::fs::remove_file(p).is_ok() };
        if ok {
            done.bytes += size;
            done.paths.push(p.clone());
        }
    }
    done
}

/// 构建成功之后调这个。回一句给日志窗的话。
pub fn sweep_after_build(repo: &Path, keep_dir: &Path) -> String {
    let now = SystemTime::now();
    let mut plan = plan(repo, keep_dir, now, STALE_DAYS, KEEP_APPS);
    let side = plan_side(&side_targets(repo), now, SIDE_STALE_DAYS);
    plan.bytes += side.bytes;
    plan.paths.extend(side.paths);
    run(&plan).note()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// 放一个文件，并把它的修改时间推到 `days_ago` 天前。
    /// 用标准库的 `File::set_modified`（1.75 起），**不为测试加依赖**。
    fn 放(p: &Path, bytes: usize, days_ago: u64) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, vec![b'x'; bytes]).unwrap();
        let f = fs::OpenOptions::new().write(true).open(p).unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(days_ago * 86_400)).unwrap();
    }

    /// 副本的编译缓存：两天没编译过的整份删；刚编译过的（还在用的任务）不动；读不到时间的不动
    #[test]
    fn 副本的编译缓存两天没编译就整份删() {
        let d = tempfile::tempdir().unwrap();
        let old = d.path().join(".claude/worktrees/old-task/src-tauri/target");
        let live = d.path().join(".claude/worktrees/live-task/src-tauri/target");
        放(&old.join("debug/deps/libx.rlib"), 1000, 5);
        放(&live.join("debug/deps/liby.rlib"), 1000, 0);
        // 目录本身的修改时间也推到 5 天前（放文件时目录被刷新成了现在）
        for sub in ["", "debug", "debug/deps"] {
            let f = std::fs::File::open(old.join(sub)).unwrap();
            f.set_modified(SystemTime::now() - Duration::from_secs(5 * 86_400)).unwrap();
        }
        let targets = side_targets(d.path());
        assert!(targets.contains(&old) && targets.contains(&live), "{targets:?}");
        let s = plan_side(&targets, SystemTime::now(), SIDE_STALE_DAYS);
        assert_eq!(s.paths, vec![old.clone()], "只删两天没编译过的那份");
        assert_eq!(s.bytes, 1000);
        // 读不到时间：不动
        assert!(plan_side(&[d.path().join("不存在")], SystemTime::now(), SIDE_STALE_DAYS).paths.is_empty());
    }

    fn target(d: &Path, profile: &str, sub: &str, name: &str) -> PathBuf {
        d.join("src-tauri/target").join(profile).join(sub).join(name)
    }

    /// **判据是「这次构建没碰过」。** 一次构建会刷新它真正用到的每个产物的
    /// mtime，所以躺满 30 天的就是旧版本依赖留下的残骸。
    #[test]
    fn 只清很久没被碰过的() {
        let d = tempfile::tempdir().unwrap();
        放(&target(d.path(), "release", "deps", "libold-1.rlib"), 100, 60);
        放(&target(d.path(), "release", "deps", "libnew-2.rlib"), 100, 1);
        放(&target(d.path(), "debug", "incremental", "old.bin"), 100, 90);

        let s = plan(d.path(), &d.path().join("没有这个目录"), SystemTime::now(), STALE_DAYS, KEEP_APPS);
        let names: Vec<String> = s.paths.iter().map(|p| p.file_name().unwrap().to_string_lossy().into()).collect();
        assert!(names.contains(&"libold-1.rlib".to_string()));
        assert!(names.contains(&"old.bin".to_string()));
        assert!(!names.contains(&"libnew-2.rlib".to_string()), "把这次构建用到的删了");
        assert_eq!(s.bytes, 200);
    }

    /// **`bundle/` 是换包要读的 `.app`，删了就换不了包。**
    /// 可执行文件本身也在 `target/<profile>/` 那一层，同样不能碰。
    #[test]
    fn 绝不碰bundle和可执行文件() {
        let d = tempfile::tempdir().unwrap();
        放(&target(d.path(), "release", "bundle", "macos/Anima.app/x"), 100, 365);
        放(&d.path().join("src-tauri/target/release/anima"), 100, 365);
        放(&d.path().join("src-tauri/target/release/bundle/Anima.dmg"), 100, 365);

        let s = plan(d.path(), Path::new("/没有"), SystemTime::now(), STALE_DAYS, KEEP_APPS);
        assert_eq!(s, Sweep::default(), "动了不该动的：{:?}", s.paths);
    }

    /// 换包出问题时要靠旧包回退 —— 留最近几份，删更早的。
    #[test]
    fn 旧包留最近几份() {
        let d = tempfile::tempdir().unwrap();
        let keep = d.path().join("apps");
        // 名字就是换包脚本起的那种：`Anima-YYYYmmdd-HHMMSS.app`
        for stamp in ["20260918-101500", "20260919-093000", "20260920-081500", "20260920-233000", "20260917-120000"] {
            放(&keep.join(format!("Anima-{stamp}.app")).join("x"), 100, 0);
        }
        let s = plan(d.path(), &keep, SystemTime::now(), STALE_DAYS, 3);
        let mut names: Vec<String> = s.paths.iter().map(|p| p.file_name().unwrap().to_string_lossy().into()).collect();
        names.sort();
        // 最近三次留着（0920-2330 / 0920-0815 / 0919），更早的两份清掉
        assert_eq!(names, vec!["Anima-20260917-120000.app", "Anima-20260918-101500.app"], "留错了份数");

        // **备份不看 30 天** —— 一天换三次包的话三份都是今天的，
        // 按天数算一份都清不掉，而它们恰恰是最占地方的（一份一百多 MB）
        let s = plan(d.path(), &keep, SystemTime::now(), 9999, 3);
        assert_eq!(s.paths.len(), 2, "备份的清理被 stale_days 影响了");
    }

    /// 读不出修改时间时**当它是新的**。宁可少清，不可误删。
    ///
    /// **直接测 `is_stale`。** 原来这条是摆两个文件、拿 `UNIX_EPOCH` 当 now，
    /// 想让 cutoff 算不出来 —— 但 `SystemTime` 在 Unix 上能表示负值，
    /// 减得动，那个分支根本没走到。变异测试（把兜底从「新」改成「旧」）
    /// 当场把它认了出来：**一条假绿测试**。
    #[test]
    fn 判不出时间就不动它() {
        let old = SystemTime::now() - Duration::from_secs(9999 * 86_400);
        let cutoff = SystemTime::now() - Duration::from_secs(30 * 86_400);
        assert!(is_stale(Some(old), Some(cutoff)), "前提不成立：很旧的应该算残骸");
        assert!(!is_stale(Some(SystemTime::now()), Some(cutoff)));
        // 两种「判不出来」都不动
        assert!(!is_stale(None, Some(cutoff)), "读不出 mtime 却把它删了");
        assert!(!is_stale(Some(old), None), "算不出截止时间却把它删了");
        assert!(!is_stale(None, None));
    }

    /// 清理失败不该让一次成功的构建看起来像失败了。
    #[test]
    fn 删不掉的跳过_不报错() {
        let d = tempfile::tempdir().unwrap();
        let gone = d.path().join("早就没了.rlib");
        let real = target(d.path(), "release", "deps", "有的.rlib");
        放(&real, 100, 60);
        let done = run(&Sweep { paths: vec![gone, real.clone()], bytes: 0 });
        assert_eq!(done.paths, vec![real]);
        assert_eq!(done.bytes, 100);
    }

    /// **接线。** 上面那些纯函数全绿，也证明不了构建真的调了它 ——
    /// 这一类「算得对但没接上」在这个仓库抓到过好几次。
    #[test]
    fn 构建成功之后真的清一遍() {
        let src = include_str!("updater.rs");
        assert!(src.contains("crate::build_gc::sweep_after_build(&repo2, &backups_dir())"), "构建完没调清理");
        // **必须在成功那一支里。** 失败了还清，等于把排查现场毁掉
        // **按下一个分支切，不按字节数。** 中间那段中文注释一个字三字节，
        // 固定长度的窗口既可能不够、又可能切在字符中间
        let at = src.find("Ok(s) if s.success()").expect("找不到成功分支");
        let end = src[at..].find("Ok(s) =>").expect("找不到失败分支");
        let arm = &src[at..at + end];
        assert!(arm.contains("sweep_after_build"), "清理没放在构建成功那一支里");
        // 清理结果要写进日志窗 —— 不显示的话用户不知道它跑过
        assert!(arm.contains("emit_raw(\"done\"") && arm.contains("note"), "清理结果没推给界面");
    }

    /// 换包挪走旧包、清理扫旧包，**必须是同一个目录** ——
    /// 各写一份的话清理去扫一个空目录，而备份在另一处接着攒。
    #[test]
    fn 备份目录只有一个所有者() {
        let src = include_str!("updater.rs");
        assert_eq!(src.matches("AnimaBackups/apps").count(), 1, "备份目录被写了不止一处");
        assert!(src.contains("pub fn backups_dir()"));
        assert!(src.contains("let keep = backups_dir();"), "换包那边没走这个函数");
    }

    /// **零也要说。** 什么都不显示的话，用户不知道这一步跑没跑。
    #[test]
    fn 零也要说一句() {
        assert_eq!(Sweep::default().note(), "构建垃圾：没有可清的");
        let s = Sweep { paths: vec![PathBuf::from("a")], bytes: 3 * 1024 * 1024 * 1024 };
        assert_eq!(s.note(), "清掉 1 项构建垃圾，腾出 3.0 GB");
    }
}
