//! 把短剧项目的 `characters.json` 从 base64 拆成「文件 + 路径」。
//!
//! ```text
//! cargo run --bin migrate-assets              # 全部项目
//! cargo run --bin migrate-assets -- <项目 id>
//! ```
//!
//! **验收判据是落盘文件数，不是「迁移成功」。** 早先按槽位 key 命名，
//! 不同档里的同名 key 会互相覆盖 —— 71 张只落下 40 张，而且不报错。

use anima_lib::assets::migrate_project_characters;
use std::path::PathBuf;

fn config_dir() -> PathBuf {
    dirs::config_dir()
        .expect("找不到配置目录")
        .join("com.anima.desktop")
}

fn main() {
    let cfg = config_dir();
    let only: Option<String> = std::env::args().nth(1);

    let projects_root = cfg.join("projects");
    let Ok(entries) = std::fs::read_dir(&projects_root) else {
        eprintln!("没有 {} —— 无事可做", projects_root.display());
        return;
    };

    let mut any = false;
    for e in entries.flatten() {
        let id = e.file_name().to_string_lossy().to_string();
        if !e.path().is_dir() || !e.path().join("characters.json").is_file() {
            continue;
        }
        if only.as_deref().is_some_and(|o| o != id) {
            continue;
        }
        any = true;
        match migrate_project_characters(&cfg, &id) {
            Ok(r) => {
                println!(
                    "{id}: 落盘 {} 张 · {} → {} 字节 · 资产 {} 条",
                    r.images_written, r.bytes_before, r.bytes_after, r.assets_created
                );
                for f in &r.failures {
                    println!("  ⚠ {f}");
                }
            }
            Err(err) => println!("{id}: 失败 —— {err}"),
        }
    }
    if !any {
        println!("没有找到需要迁移的项目");
    }
}
