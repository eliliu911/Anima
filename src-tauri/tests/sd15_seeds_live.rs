//! SD1.5 整排跨种子的稳定性。
//!
//! 用户实测报「试了 3 次都没出合格的图：只数出 2 个视角 / 1 个视角」——
//! 三视图那一档整档失败。这里跑多个种子，看分割到底稳不稳。
//!
//! ```bash
//! cargo test --test sd15_seeds_live -- --ignored --nocapture
//! ```

use anima_lib::cinema_qc::{check_turnaround, classify_views, view_bounds, Verdict};
use anima_lib::cinema_sheet::{self as sheet, SheetEngine};

#[tokio::test]
#[ignore = "需要本机 ComfyUI + majicmix + three_sided_view_sd15"]
async fn 跨种子都分得出三格() {
    let home = std::env::var("HOME").unwrap_or_default();
    let ckpts: Vec<String> = std::fs::read_dir(format!("{home}/AI/ComfyUI/models/checkpoints"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".safetensors"))
        .collect();
    let ckpt = SheetEngine::Sd15
        .pick_checkpoint(&ckpts)
        .expect("没有 SD1.5 底模");
    let lora = sheet::find_layout_lora(SheetEngine::Sd15).expect("没有 SD1.5 版式 LoRA");
    let http = reqwest::Client::builder().no_proxy().build().unwrap();

    let mut ok = 0;
    let seeds = [11_111_i64, 22_222, 33_333, 44_444];
    for seed in seeds {
        let shot = sheet::turnaround(
            SheetEngine::Sd15,
            &ckpt,
            "1girl, east asian woman, chinese, short auburn bob hair, gentle gaze",
            "white ribbed tank top, simple silver necklace",
            Some(lora.clone()),
            None,
            seed,
            "Anima_Sd15Seeds",
        );
        match sheet::render(&http, shot, 0).await {
            Ok(r) => {
                let img = image::load_from_memory(&r.bytes).unwrap();
                let b = view_bounds(&img);
                let v = classify_views(&img);
                let verdict = check_turnaround(&img);
                if b.len() >= 3 {
                    ok += 1
                }
                eprintln!(
                    "  seed {seed}: {}格 {v:?}  QC={verdict:?}  尝试{}次",
                    b.len(),
                    r.attempts_used
                );
                for n in &r.notes {
                    eprintln!("      {n}");
                }
                std::fs::write(
                    std::env::temp_dir().join(format!("sd15_seed_{seed}.png")),
                    &r.bytes,
                )
                .ok();
            }
            Err(e) => eprintln!("  seed {seed}: 整格失败 {e}"),
        }
    }
    eprintln!("\n  {ok}/{} 个种子分出了三格以上", seeds.len());
    assert!(
        ok >= 3,
        "三视图那一档不能这么不稳 —— 只有 {ok}/{} 个种子成功",
        seeds.len()
    );
    let _ = Verdict::Ok;
}
