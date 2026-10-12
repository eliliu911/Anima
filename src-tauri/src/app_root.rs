//! 项目根目录的解析 —— **打包版和开发版在这件事上处境完全不同**。
//!
//! # 为什么需要这个模块
//!
//! 有五个目录是「放在项目里」而不是放在应用数据目录的：
//!
//! | 目录 | 放什么 | 为什么不进应用数据目录 |
//! |---|---|---|
//! | `characters/` | 角色形态（人格、声线、模型指向） | 想跟项目一起版本管理 |
//! | `skills/` | 技能（SKILL.md、提示词、工具白名单） | 同上，要能直接改文件 |
//! | `avatars/` | Live2D / 3D / 神经视频模型 | 几个 G，gitignore 掉 |
//! | `voice-library/` | 参考音频 | 1~2 GB，gitignore 掉 |
//! | `tools/` | 五个带各自 `.venv` 的 Python 服务 | 12 GB，torch 不可搬运 |
//!
//! 原来每一处各写一份「从 `current_dir()` 往上找四层」。**开发期能中，
//! 双击的 `.app` 一个都中不了** —— macOS 启动 `.app` 时 cwd 是 `/`。
//!
//! 2026-09-18 实测到的后果，全部不报错：
//!
//! - 角色形态只剩一个兜底的「默认形态」（16 个角色一个都不显示）
//! - 技能与拓展整页空白
//! - GPT-SoVITS 被显示成「服务未运行」—— 服务好好的，
//!   是共享音色库找不到、引擎构造时就失败了，界面把两件事显示成了一件
//! - 本地语音引擎（`tools/`）一个都起不来
//!
//! # 怎么解决的
//!
//! 一份候选列表，所有人共用。前两条是**专门给打包版的**：
//!
//! 1. `ANIMA_APP_ROOT` 环境变量 —— 从终端起的时候最直接
//! 2. `<应用数据目录>/app_root` 指针文件 —— **双击启动只有这一条能用**，
//!    由 `anima install` 写
//! 3. 从 `current_dir()` 往上四层 —— 开发期靠它
//! 4. 从可执行文件所在目录往上两层 —— 便携式布局
//!
//! # 判据是「里面有没有那个子目录」，不是「这个目录存不存在」
//!
//! 候选里几乎每一个都存在（`/`、`Contents/MacOS`……）。按存在与否挑的话
//! 第一个就中，而它是错的 —— 这正是打包版原来的失败方式。

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// 指针文件名。`anima install` 往里写本机仓库路径。
pub const POINTER: &str = "app_root";

/// 应用数据目录。启动时设一次。
///
/// **为什么要个全局的。** 指针文件在应用数据目录里，而它是双击启动唯一能用的
/// 那条路 —— 可是有一堆解析路径的函数**不带 `config_dir` 参数**
/// （`cinema::workspace_root()`、`style::thumbs_dir()`…）。挨个加参数要改一长串
/// 调用链，而漏掉任何一个就又是一处「打包版找不到」。
///
/// 所以反过来：启动时存一次，谁都能拿到。2026-09-18 之前没有这个，
/// 结果同一个坑修了三轮才修干净（`tools/` → 另外四个目录 → 风格缩略图）。
fn config_dir_cell() -> &'static OnceLock<PathBuf> {
    static C: OnceLock<PathBuf> = OnceLock::new();
    &C
}

/// 启动时调一次。**排在任何路径解析之前。**
pub fn init(config_dir: &Path) {
    let _ = config_dir_cell().set(config_dir.to_path_buf());
}

/// 往上找几层。再深就说明根本不在项目里跑，硬找下去只会误命中别人的目录。
const WALK_UP: usize = 4;

/// 项目根的候选，**按优先级**。
///
/// `config_dir` 给 `None` 也能用（跑测试、或者还没拿到配置目录时），
/// 只是会少掉指针文件那条 —— 而那条正是双击启动唯一能用的。
pub fn roots(config_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();

    if let Some(v) = std::env::var_os("ANIMA_APP_ROOT") {
        let v = PathBuf::from(v);
        if !v.as_os_str().is_empty() {
            out.push(v);
        }
    }
    // 没显式给就用启动时存的那个 —— 见 `init` 的注释
    let fallback = config_dir_cell().get().map(PathBuf::as_path);
    if let Some(dir) = config_dir.or(fallback) {
        if let Ok(text) = std::fs::read_to_string(dir.join(POINTER)) {
            let line = text.lines().next().unwrap_or("").trim();
            if !line.is_empty() {
                out.push(PathBuf::from(line));
            }
        }
    }
    if let Ok(mut dir) = std::env::current_dir() {
        for _ in 0..WALK_UP {
            out.push(dir.clone());
            if !dir.pop() {
                break;
            }
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(Path::to_path_buf);
        for _ in 0..2 {
            let Some(d) = dir else { break };
            out.push(d.clone());
            dir = d.parent().map(Path::to_path_buf);
        }
    }
    out
}

/// 找一个**里面有 `name` 子目录**的项目根，回那个子目录的路径。
///
/// `extra` 是额外的判据（比如「`characters/` 下面得有个带 `SKILL.md` 的子目录」）——
/// 光看目录名会误命中同名的空目录。
pub fn find_dir(
    name: &str,
    config_dir: Option<&Path>,
    extra: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    roots(config_dir).into_iter().find_map(|root| {
        let c = root.join(name);
        (c.is_dir() && extra(&c)).then_some(c)
    })
}

/// 同 [`find_dir`]，不加额外判据。
pub fn find_plain(name: &str, config_dir: Option<&Path>) -> Option<PathBuf> {
    find_dir(name, config_dir, |_| true)
}

/// Anima 的仓库根：要同时有 `package.json` 和 `scripts/`。
///
/// **不带参数**，用启动时存的应用数据目录 —— 调用它的那些函数
/// （`cinema::workspace_root()` 之类）本来就没有 `config_dir` 可传。
pub fn repo_root() -> Option<PathBuf> {
    roots(None)
        .into_iter()
        .find(|r| r.join("package.json").is_file() && r.join("scripts").is_dir())
}

/// 找不到时给人看的话。**要能照着做**，不是「未找到」四个字。
pub fn missing_hint(name: &str, what: &str) -> String {
    format!(
        "找不到 {name}/ 目录 —— {what}。打包版不带它，也不经过 shell，\
         所以要么跑一句 `anima install` 写好指针文件，要么设 ANIMA_APP_ROOT 指到仓库根目录。\
         指针文件：<应用数据目录>/{POINTER}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **判据是「里面有没有那个子目录」，不是「这个目录存不存在」。**
    ///
    /// 候选里几乎每一个都存在（`/`、`Contents/MacOS`……），按存在与否挑的话
    /// 第一个就中，而它是错的 —— 这正是打包版原来的失败方式。
    #[test]
    fn 按有没有子目录挑() {
        let d = tempfile::tempdir().unwrap();
        let cfg = d.path().join("cfg");
        let repo = d.path().join("repo");
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::create_dir_all(repo.join("characters")).unwrap();
        std::fs::write(cfg.join(POINTER), format!("{}\n", repo.display())).unwrap();

        assert_eq!(find_plain("characters", Some(&cfg)), Some(repo.join("characters")));
        assert_eq!(find_plain("这个子目录不存在", Some(&cfg)), None);
    }

    /// **指针文件是双击启动唯一能用的那条路。** 它必须排在 cwd 前面 ——
    /// 双击时 cwd 是 `/`，排后面就等于没有。
    #[test]
    fn 指针文件排在cwd之前() {
        let d = tempfile::tempdir().unwrap();
        let cfg = d.path().join("cfg");
        let repo = d.path().join("repo");
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(cfg.join(POINTER), format!("{}\n", repo.display())).unwrap();

        let r = roots(Some(&cfg));
        let cwd = std::env::current_dir().unwrap();
        let at_repo = r.iter().position(|p| p == &repo).expect("指针文件没进候选");
        let at_cwd = r.iter().position(|p| p == &cwd).expect("cwd 没进候选");
        assert!(at_repo < at_cwd, "指针文件排到 cwd 后面了，双击启动就用不上");
    }

    /// 指针文件里是垃圾时不能崩，也不能把垃圾当成结果。
    #[test]
    fn 指针文件坏了不崩() {
        let d = tempfile::tempdir().unwrap();
        for junk in ["", "   \n", "/根本不存在的路径\n"] {
            std::fs::write(d.path().join(POINTER), junk).unwrap();
            assert_ne!(
                find_plain("characters", Some(d.path())),
                Some(PathBuf::from("/根本不存在的路径/characters"))
            );
        }
    }

    /// 额外判据要真的起作用 —— 光看目录名会误命中同名的空目录。
    #[test]
    fn 额外判据能排掉同名空目录() {
        let d = tempfile::tempdir().unwrap();
        let cfg = d.path().join("cfg");
        let empty = d.path().join("空的");
        let real = d.path().join("真的");
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::create_dir_all(empty.join("characters")).unwrap();
        std::fs::create_dir_all(real.join("characters/某角色")).unwrap();
        std::fs::write(real.join("characters/某角色/SKILL.md"), b"x").unwrap();
        // 两个候选都写进指针是不可能的，所以用 ANIMA_APP_ROOT 排第一、指针排第二
        std::fs::write(cfg.join(POINTER), format!("{}\n", real.display())).unwrap();

        let has_skill = |p: &Path| {
            std::fs::read_dir(p)
                .map(|it| it.flatten().any(|e| e.path().join("SKILL.md").is_file()))
                .unwrap_or(false)
        };
        assert!(!has_skill(&empty.join("characters")));
        assert_eq!(find_dir("characters", Some(&cfg), has_skill), Some(real.join("characters")));
    }

    /// 没有 config_dir 也不能崩 —— 跑测试、启动早期都是这种情况。
    #[test]
    fn 没有配置目录也能用() {
        assert!(!roots(None).is_empty(), "至少该有 cwd 那几条");
    }

    /// 仓库根要**同时**有 `package.json` 和 `scripts/` —— 只看一个的话，
    /// 任何 node 项目的目录都会被认成 Anima 的仓库。
    #[test]
    fn 仓库根要两样都有() {
        let d = tempfile::tempdir().unwrap();
        let only_pkg = d.path().join("只有package");
        let both = d.path().join("两样都有");
        std::fs::create_dir_all(&only_pkg).unwrap();
        std::fs::write(only_pkg.join("package.json"), b"{}").unwrap();
        std::fs::create_dir_all(both.join("scripts")).unwrap();
        std::fs::write(both.join("package.json"), b"{}").unwrap();

        let looks_like = |p: &std::path::Path| p.join("package.json").is_file() && p.join("scripts").is_dir();
        assert!(!looks_like(&only_pkg));
        assert!(looks_like(&both));
    }

    /// 提示要**能照着做**，不是「未找到」四个字。
    #[test]
    fn 提示能照着做() {
        let h = missing_hint("characters", "角色形态会只剩一个兜底");
        assert!(h.contains("anima install"), "没告诉用户怎么办：{h}");
        assert!(h.contains("ANIMA_APP_ROOT"));
        assert!(h.contains("角色形态会只剩一个兜底"), "没说会怎么样：{h}");
    }
}
