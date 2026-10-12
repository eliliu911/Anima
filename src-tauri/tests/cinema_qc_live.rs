//! 质检的联调测试：**拿真实出图验，不是合成图**。
//!
//! 单元测试里的合成图太干净 —— 平底、无阴影、无暗角。第一版算法在合成图上
//! 7 个测试全绿，一碰真实照片就把单人侧面数成 3 个（多出来的两块贴着画面
//! 左右边缘，那是背景渐变和暗角）。这组测试就是为了不让那种事再发生。
//!
//! 默认 ignore：它依赖 ComfyUI output 目录里的实拍产物。
//!
//! ```bash
//! cargo test --test cinema_qc_live -- --ignored --nocapture
//! ```

#[test]
#[ignore = "需要本机 ComfyUI 的实拍产物"]
fn 三种画面都数对() {
    for (f, expect, label) in [
        ("probe_lora_00001_.png", 6usize, "LoRA 六视图（密排）"),
        ("probe_sheet_00001_.png", 4, "四个人（稀疏）"),
        ("probe_side_00001_.png", 1, "单人侧面"),
        ("probe_back_00001_.png", 1, "单人背面"),
    ] {
        let p = std::path::Path::new("/Users/you/AI/ComfyUI/output").join(f);
        if !p.exists() {
            continue;
        }
        let img = image::open(&p).unwrap();
        let n = anima_lib::cinema_qc::count_subjects(&img);
        eprintln!(
            "  {label}: 数出 {n}，期望 {expect}  {}",
            if n == expect { "✓" } else { "✗" }
        );
        assert_eq!(n, expect, "{label} 数错了");
    }
}
