//! 打包版的路径解析：**模拟 cwd 是 `/`**，验证靠指针文件还能不能找到。
//!
//! 这是「风格缩略图一张不显示」那个 bug 的回归测试。它的价值在于
//! **不用真的打包就能验** —— 前三轮都是打包、装上、用户发现、才知道漏了。

use std::path::PathBuf;

/// 双击的 `.app` 启动时 cwd 是 `/`。把当前进程的 cwd 换成 `/`，
/// 看那几个目录还找不找得到。
#[test]
fn cwd是根目录时靠指针文件仍然找得到() {
    let d = tempfile::tempdir().unwrap();
    let cfg = d.path().join("cfg");
    let repo = d.path().join("repo");
    std::fs::create_dir_all(&cfg).unwrap();
    for sub in ["characters/某角色", "skills/某技能", "avatars", "voice-library", "tools", "resources/style_thumbs", "scripts"] {
        std::fs::create_dir_all(repo.join(sub)).unwrap();
    }
    std::fs::write(repo.join("characters/某角色/SKILL.md"), b"x").unwrap();
    std::fs::write(repo.join("skills/某技能/SKILL.md"), b"x").unwrap();
    std::fs::write(repo.join("package.json"), b"{}").unwrap();
    std::fs::write(cfg.join("app_root"), format!("{}\n", repo.display())).unwrap();

    let prev = std::env::current_dir().unwrap();
    std::env::set_current_dir("/").unwrap();

    // **每一个都要找得到。** 少一个就是一处「打包版里这个功能静默消失」
    for name in ["characters", "skills", "avatars", "voice-library", "tools", "resources"] {
        let got = anima_lib::app_root::find_plain(name, Some(&cfg));
        assert_eq!(got, Some(repo.join(name)), "cwd 是 / 时找不到 {name}/");
    }
    // 仓库根（风格缩略图、LivePortrait 脚本靠它）
    anima_lib::app_root::init(&cfg);
    assert_eq!(anima_lib::app_root::repo_root(), Some(repo.clone()), "cwd 是 / 时找不到仓库根");

    std::env::set_current_dir(prev).unwrap();
}

/// 没有指针文件、cwd 又是 `/` 时，**要回 `None` 而不是一个错路径**。
#[test]
fn 什么都没有时回空不回错路径() {
    let cfg = tempfile::tempdir().unwrap();
    let prev = std::env::current_dir().unwrap();
    std::env::set_current_dir("/").unwrap();
    let got = anima_lib::app_root::find_plain("characters", Some(cfg.path()));
    std::env::set_current_dir(prev).unwrap();
    assert_eq!(got, None, "应该回 None —— 回一个错路径会让后面每一步都静默失败");
}
