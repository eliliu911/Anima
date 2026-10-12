//! SD1.5 那条路端到端跑一次真图，把结果存下来给人看。
//!
//! 单元测试只证明了「参数按引擎分开了」，证明不了**出来的图是对的** ——
//! 而这套东西所有的失败都不报错：LoRA 挂错族只是不起作用，
//! 分辨率超出训练范围只是把人平铺开，两种都能正常跑完全部采样步。
//!
//! ```bash
//! cargo test --test sheet_engine_live -- --ignored --nocapture
//! ```

use anima_lib::cinema_qc::{classify_views, view_bounds, View};
use anima_lib::cinema_sheet::{self as sheet, SheetEngine};

#[tokio::test]
#[ignore = "需要本机 ComfyUI + majicmix + three_sided_view_sd15"]
async fn sd15整排出得来正侧背() {
    let ckpt = SheetEngine::Sd15
        .pick_checkpoint(&list_checkpoints())
        .expect("没找到 SD1.5 底模（majicmixRealistic）");
    let lora = sheet::find_layout_lora(SheetEngine::Sd15).expect("没找到 SD1.5 版式 LoRA");
    eprintln!("  底模 = {ckpt}\n  版式 LoRA = {lora}");

    let shot = sheet::turnaround(
        SheetEngine::Sd15,
        &ckpt,
        "1girl, east asian woman, chinese, long black silky hair, sharp confident gaze",
        "black silk velvet evening gown, diamond necklace",
        Some(lora),
        None,
        167_776_081,
        "Anima_EngineLive",
    );
    eprintln!(
        "  第一段 {}×{}  第二段 {:?}",
        shot.width, shot.height, shot.hires
    );

    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let out = sheet::render(&http, shot, 0).await.expect("出图失败");
    for n in &out.notes {
        eprintln!("  note: {n}");
    }

    let path = std::env::temp_dir().join("anima_sd15_turnaround.png");
    std::fs::write(&path, &out.bytes).unwrap();
    eprintln!("  存到 {}", path.display());

    let img = image::load_from_memory(&out.bytes).unwrap();
    // 第二段放大之后才是最终尺寸
    assert!(
        img.width() >= 1600,
        "第二段没生效？宽度只有 {}",
        img.width()
    );

    let bounds = view_bounds(&img);
    let views = classify_views(&img);
    let picked = anima_lib::cinema_qc::pick_views(&img);
    eprintln!("  {} 格 {views:?}  挑 {picked:?}", bounds.len());

    assert!(bounds.len() >= 3, "整排至少要有三格，实际 {}", bounds.len());
    assert_eq!(
        views.first(),
        Some(&View::Front),
        "第一格该是正面：{views:?}"
    );

    // **这里不断言「分类结果里有 Back」。**
    // 实测这条路常出「身体背对、头转回来」的背视图 —— 脸是看得见的，
    // 肤色判据必然把它判成正面。那不是 bug，是这个判据分不了的情况。
    //
    // 真正的契约是**挑出来的三格互不相同、且第三格是最靠后的那个视角**：
    // `pick_views` 找不到 Back 时回退到最后一格，而 three_sided_view 这个
    // LoRA 的训练版式就是正→侧→背，最后一格正是背面。
    let [f, s, b] = picked.expect("三格以上应该挑得出来");
    assert_eq!(f, 0, "正面该是第一格");
    assert_eq!(b, views.len() - 1, "背面该是最后一格");
    assert!(s != f && s != b, "三格不能重复：{:?}", [f, s, b]);
}

fn list_checkpoints() -> Vec<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    std::fs::read_dir(format!("{home}/AI/ComfyUI/models/checkpoints"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".safetensors"))
        .collect()
}
