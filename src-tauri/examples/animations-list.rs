//! 列出动作库里**实际被认出来**的动作，以及被忽略的文件。
//!
//! 加这个是因为"放进去了但用不上"没有任何反馈：不认识的文件被静默跳过，
//! 目录里躺着七个 `.fbx` 和八个 `.vrma`，从外面看不出区别。
//!
//!   cargo run --example animations-list

use std::collections::HashSet;

fn main() {
    let config_dir = dirs::config_dir().unwrap().join("com.anima.desktop");
    let root = anima_lib::avatar::model::animations_dir(&config_dir);
    println!("动作库：{}\n", root.display());

    let builtin: Vec<&str> = anima_lib::voice::BUILTIN_ACTIONS
        .iter()
        .map(|(n, _)| *n)
        .collect();

    println!("内置手势（代码生成，没有文件也能用）：");
    for (name, desc) in anima_lib::voice::BUILTIN_ACTIONS {
        println!("  <action:{name}>  {desc}");
    }

    let listed = anima_lib::avatar::model::list_animations_in(&root);
    let names = anima_lib::avatar::model::action_names_in(&root, &builtin);
    println!("\n动作库里认出来的（{} 个）：", names.len());
    for (name, file) in &names {
        println!("  <action:{name}>  ← {file}");
    }

    // 目录里还有什么没被认出来 —— 这才是这个工具的重点
    let known: HashSet<String> = listed.iter().map(|a| a.id.clone()).collect();
    let mut ignored: Vec<String> = Vec::new();
    let mut scan = |dir: &std::path::Path, prefix: &str| {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_file() {
                    let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
                        continue;
                    };
                    let id = format!("{prefix}{name}");
                    if !known.contains(&id) && !name.starts_with('.') && !name.ends_with(".md") {
                        ignored.push(id);
                    }
                }
            }
        }
    };
    scan(&root, "");
    if let Ok(entries) = std::fs::read_dir(&root) {
        for e in entries.flatten() {
            if e.path().is_dir() {
                let sub = e.file_name().to_string_lossy().to_string();
                scan(&e.path(), &format!("{sub}/"));
            }
        }
    }

    if ignored.is_empty() {
        println!("\n没有被忽略的文件。");
    } else {
        println!("\n被忽略的文件（{} 个）：", ignored.len());
        ignored.sort();
        for f in &ignored {
            println!("  {f}");
        }
        println!("\n只认 .vrma。Mixamo 的 .fbx 用的是它自己的骨骼命名，");
        println!("要重定向到 VRM humanoid 才能用 —— 见 animations/README.md。");
    }
}
