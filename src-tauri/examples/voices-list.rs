//! 列出音色库里每个音色，以及**它会跟谁融合**。
//!
//! 融合是隐式的：你在设置里选一个音色名，实际送进 GPT-SoVITS 的可能是三条参考。
//! 选错了组（比如"温柔"和"激动"混在一起）表现是音色发飘，而界面上看不出来 ——
//! 所以需要一个地方把它显式打出来。
//!
//!   cargo run --example voices-list

fn main() {
    let config_dir = dirs::config_dir().unwrap().join("com.anima.desktop");
    let root = anima_lib::tts::voices::voices_dir(&config_dir);
    println!("音色库：{}\n", root.display());

    let list = anima_lib::tts::voices::list_in(&root);
    if list.is_empty() {
        println!("（空）");
        return;
    }

    for v in &list {
        let group = v
            .name
            .rsplit('/')
            .next()
            .map(anima_lib::tts::voices::group_of)
            .unwrap_or("");
        let group = if group.is_empty() {
            "（默认组）".to_string()
        } else {
            format!("组 {group}")
        };
        println!("{:<22} {:<14} 融合 {} 条", v.name, group, v.aux.len() + 1);
        for p in &v.aux {
            println!(
                "{:<22}   + {}",
                "",
                p.file_name().unwrap().to_string_lossy()
            );
        }
    }
    println!(
        "\n上限 {} 条。想让某一条不参与融合，给它一个独有前缀（例如 warm-greeting）。",
        anima_lib::tts::voices::MAX_FUSED
    );
}
