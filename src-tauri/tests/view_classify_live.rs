//! 视角分类拿真实整排验。
//!
//! **不能按位置猜。** 原先假设整排是「正面 → 逐步转身 → 背面」取首/中/尾，
//! 而 LoRA 不保证这个顺序 —— 实测一张 5 视角整排是
//! 正面/侧面/侧面/背面/正面，取尾拿到正面，于是「背面」那格显示的是正脸。

use anima_lib::cinema_qc::{classify_views, pick_views, View};

#[test]
#[ignore = "需要本机实拍整排"]
fn 分得出正侧背() {
    let home = std::env::var("HOME").unwrap();
    let p = format!(
        "{home}/Library/Application Support/com.anima.desktop/projects/\
         drama_sample_sweetheart/characters/yumiko/angles/full_sheet.png"
    );
    let path = std::path::Path::new(&p);
    if !path.exists() {
        eprintln!("跳过：没有整排图");
        return;
    }
    let img = image::open(path).unwrap();
    let n = anima_lib::cinema_qc::count_subjects(&img);
    let views = classify_views(&img);
    eprintln!("  {n} 个视角：{views:?}");
    eprintln!("  挑格 -> {:?}", pick_views(&img));

    assert_eq!(views.first(), Some(&View::Front), "第一格应是正面");
    assert!(views.contains(&View::Back), "整排里应该能找出背面");
    assert!(views.contains(&View::Side), "整排里应该能找出侧面");

    let picked = pick_views(&img).expect("应该挑得出三格");
    assert_eq!(views[picked[0]], View::Front);
    assert_eq!(views[picked[1]], View::Side);
    assert_eq!(
        views[picked[2]],
        View::Back,
        "背面那格挑错了 —— 这正是原来的 bug"
    );
}
