fn main() {
    // 构建信息烤进二进制 —— 界面上「我在跑哪一版」必须能答得出来。
    //
    // **版本号不在这里**：它来自 `CARGO_PKG_VERSION`（Cargo.toml），
    // 而三处版本一致由 `src/version.test.ts` 守着。这里只补 Cargo 给不了的两样。
    //
    // 取不到时给明确的占位而不是空串 —— 空串在界面上看起来像「功能坏了」，
    // 而 `unknown` 至少说明「这份构建没带这个信息」。
    let sha = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into());

    // 工作区有没有未提交改动。**开发期构建和正式构建要能一眼分清** ——
    // 用户报 bug 时说的「0.9.0」如果是带脏工作区编的，那和发布的 0.9.0 不是一个东西。
    //
    // **`-uno`：只看已跟踪文件。** 不加的话未跟踪文件也算脏，而这个工作区里
    // 长期躺着一堆未跟踪的设计稿 —— 于是**每一次构建都是 `-dirty`**，
    // 这个标记就永远亮着、等于没有。2026-09-18 实测撞上：更新面板据此报
    // 「当前构建带未提交改动」，成了一条永远消不掉的假警报。
    //
    // 和 `anima update` 那条护栏是同一个坑（那边当天早些时候已经改成 -uno），
    // 这里漏了一处。
    let dirty = std::process::Command::new("git")
        .args(["status", "--porcelain", "-uno"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
        .unwrap_or(false);

    let date = std::process::Command::new("date")
        .args(["-u", "+%Y-%m-%d %H:%M UTC"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".into());

    println!("cargo:rustc-env=ANIMA_GIT_SHA={}{}", sha, if dirty { "-dirty" } else { "" });
    println!("cargo:rustc-env=ANIMA_BUILD_DATE={date}");
    // git HEAD 变了要重新烤 —— 不加这个，换分支/提交之后显示的还是旧 sha
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/index");

    tauri_build::build()
}
