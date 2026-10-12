//! 用**本机引擎**给每个内置风格预设各出一张缩略图。
//!
//! ```text
//! cargo run --release --bin render-style-thumbs [预设id ...]
//! ```
//!
//! # 为什么不用参考项目那 16 张
//!
//! LumenX 的样例图是 wanx / gpt-image 出的。我们是
//! Z-Image / Chroma / SDXL / FLUX，编码器完全不同 —— 同一段词出来
//! 不是同一张画。拿它们当选择器，等于让用户**按一张我们复现不出来的
//! 画面做选择**，而这正是这个项目反复栽的那类失败。
//!
//! 每个风格一句**自己的** `sample_prompt`，画的是适合这个风格的题材。
//! 以前 15 张用同一句（雨夜站台 + 罗盘），想比出风格差异，结果一排看下来
//! 全是同一个画面，用户觉得怪（2026-09-14 改）。
//! 走 Z-Image（45s/张，约 11 分钟跑完）。
//!
//! 输出目录默认 `resources/style_thumbs`，设 `STYLE_THUMBS_OUT` 可以先出到别处、
//! 全部成功后再整体替换 —— 跑到一半失败时界面不会变成新旧混搭。

use anima_lib::agent_runtime::{RuntimeContext, RuntimePolicy, ToolCallRequest, ToolHandler};
use anima_lib::style;
use std::path::PathBuf;

#[tokio::main]
async fn main() {
    let only: Vec<String> = std::env::args().skip(1).collect();
    let out_dir = std::env::var_os("STYLE_THUMBS_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("resources").join("style_thumbs"));
    if let Err(e) = std::fs::create_dir_all(&out_dir) {
        eprintln!("建不了输出目录 {}：{e}", out_dir.display());
        std::process::exit(1);
    }

    let work = std::env::temp_dir().join("anima-style-thumbs");
    let _ = std::fs::create_dir_all(&work);
    let ctx = RuntimeContext::new(work.clone(), RuntimePolicy::workspace_write(&work));

    let presets: Vec<_> = style::all_presets()
        .iter()
        .filter(|p| only.is_empty() || only.contains(&p.id))
        .collect();
    println!("要出 {} 张，落在 {}", presets.len(), out_dir.display());

    let mut ok = 0usize;
    let mut failed: Vec<String> = Vec::new();
    for (i, p) in presets.iter().enumerate() {
        let target = out_dir.join(format!("{}.png", p.id));
        let shrunk = out_dir.join(format!("{}.jpg", p.id));
        if target.is_file() || shrunk.is_file() {
            println!("[{}/{}] {} 已有，跳过", i + 1, presets.len(), p.id);
            ok += 1;
            continue;
        }
        // 画面在前、风格在后 —— 和 `style::apply_style` 同一条规矩。
        let prompt = format!("{}, {}", p.sample_prompt.trim(), p.positive_prompt.trim());
        let file = format!("style_{}.png", p.id);
        let args = serde_json::json!({
            "prompt": prompt,
            "engine": "zimage",
            "width": 768, "height": 768,
            // 固定 seed，重跑出同一张。
            "seed": 20_260_905u64,
            "output_filename": file,
        });
        print!("[{}/{}] {} … ", i + 1, presets.len(), p.id);
        use std::io::Write;
        let _ = std::io::stdout().flush();

        let started = std::time::Instant::now();
        let outcome = anima_lib::agent_runtime::image_tool::GenerateImageTool
            .execute(&ctx, ToolCallRequest::new("generate_image", args))
            .await;

        let produced = find_output(&work, &file);
        match (&outcome.status, produced) {
            (anima_lib::agent_runtime::ToolStatus::Ok, Some(src)) => {
                match std::fs::copy(&src, &target) {
                    Ok(n) => {
                        println!("{:.0}s · {} KB", started.elapsed().as_secs_f32(), n / 1024);
                        ok += 1;
                    }
                    Err(e) => {
                        println!("拷不过去：{e}");
                        failed.push(p.id.clone());
                    }
                }
            }
            (_, None) => {
                println!("没找到产物 —— {}", first_line(&outcome.content));
                failed.push(p.id.clone());
            }
            _ => {
                println!("失败 —— {}", first_line(&outcome.content));
                failed.push(p.id.clone());
            }
        }
    }

    // **判据是数文件，不是「跑完了」。**
    let on_disk = std::fs::read_dir(&out_dir)
        .map(|d| {
            d.flatten()
                .filter(|e| {
                    e.path()
                        .extension()
                        .is_some_and(|x| x == "png" || x == "jpg")
                })
                .count()
        })
        .unwrap_or(0);
    println!("\n成功 {ok} · 磁盘上 {on_disk} 张");
    println!("提示：进仓库前先缩一下 ——");
    println!("  for f in resources/style_thumbs/*.png; do \\");
    println!("    sips -Z 384 \"$f\" --out \"$f\" && \\");
    println!("    sips -s format jpeg -s formatOptions 82 \"$f\" --out \"${{f%.png}}.jpg\" && rm \"$f\"; done");
    println!("  （768px PNG 一套 12 MB，384px JPG 一套 850 KB；界面上只显示 224px）");
    if !failed.is_empty() {
        println!("失败的：{}", failed.join(" "));
        std::process::exit(1);
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").chars().take(160).collect()
}

/// 工具把文件落在工作区的 `generated/` 下，名字可能带后缀变化，
/// 所以按前缀找一遍。
fn find_output(work: &std::path::Path, wanted: &str) -> Option<PathBuf> {
    let stem = wanted.trim_end_matches(".png");
    for dir in [work.join("generated"), work.join(".anima").join("generated")] {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut hits: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(stem))
            })
            .collect();
        hits.sort();
        if let Some(p) = hits.pop() {
            return Some(p);
        }
    }
    None
}
