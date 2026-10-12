//! 版本号散在三个文件里，必须一致。
//!
//! `Cargo.toml`（Rust 侧）、`package.json`（前端）、`tauri.conf.json`（打包时写进
//! .app 的 Info.plist）。漂了不会有任何报错 —— 只会让打出来的包和源码对不上，
//! 而那种问题在"用户报的 bug 在我这儿复现不了"的时候最难查。

/// 三处的版本号必须完全相同。
#[test]
fn the_three_version_numbers_agree() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));

    let cargo = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let cargo_version = cargo
        .lines()
        // 只看 [package] 段开头那个 version，不要撞上依赖的版本
        .find(|l| l.starts_with("version = "))
        .and_then(|l| l.split('"').nth(1))
        .expect("Cargo.toml 里找不到 version");

    let tauri = std::fs::read_to_string(root.join("tauri.conf.json")).unwrap();
    let tauri_version =
        json_string_field(&tauri, "version").expect("tauri.conf.json 里找不到 version");

    let pkg = std::fs::read_to_string(root.parent().unwrap().join("package.json")).unwrap();
    let pkg_version = json_string_field(&pkg, "version").expect("package.json 里找不到 version");

    assert_eq!(
        cargo_version, tauri_version,
        "Cargo.toml 与 tauri.conf.json 的版本号不一致"
    );
    assert_eq!(
        cargo_version, pkg_version,
        "Cargo.toml 与 package.json 的版本号不一致"
    );
    // 顺手挡住"忘了填"的情况
    assert!(
        cargo_version.split('.').count() == 3,
        "版本号应当是 x.y.z：{cargo_version}"
    );
}

/// 取 JSON 里顶层某个字符串字段。
///
/// 不引 serde_json 到集成测试里：这两个文件的形状固定，而少一个依赖
/// 就少一处会因为版本变化而失效的地方。
fn json_string_field(text: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(&needle) {
            let rest = rest.trim_start().strip_prefix(':')?.trim();
            return rest.split('"').nth(1).map(str::to_string);
        }
    }
    None
}
