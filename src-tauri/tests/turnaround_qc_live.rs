//! 整排三视图的质检，拿真实出图验。
//!
//! 判据是**尺度一致性**：正经 turnaround 里每个视角都顶天立地，
//! 跑偏时会混进半身特写。光数个数不行 —— 视角数本来就不固定（5~6 个）。
//!
//! ```bash
//! cargo test --test turnaround_qc_live -- --ignored --nocapture
//! ```

use anima_lib::cinema_qc::{check_turnaround, Verdict};

#[test]
#[ignore = "需要本机 ComfyUI 的实拍产物"]
fn 废图被挡下好图放行() {
    let home = std::env::var("HOME").unwrap();
    let cases: [(&str, bool, &str); 4] = [
        (
            &*Box::leak(format!("{home}/Library/Application Support/com.anima.desktop/projects/drama_sample_sweetheart/characters/yumiko/angles/full_sheet.png").into_boxed_str()),
            false,
            "应用生成的废图（4 个人、两个巨大半身）",
        ),
        (&*Box::leak(format!("{home}/AI/ComfyUI/output/ip_none_00001_.png").into_boxed_str()), true, "好图 无参考"),
        (&*Box::leak(format!("{home}/AI/ComfyUI/output/ip_030_00001_.png").into_boxed_str()), true, "好图 IP 0.30"),
        (&*Box::leak(format!("{home}/AI/ComfyUI/output/ip_060_00001_.png").into_boxed_str()), true, "好图 IP 0.60"),
    ];

    for (path, should_pass, label) in cases {
        let p = std::path::Path::new(path);
        if !p.exists() {
            eprintln!("  跳过 {label}（文件不在）");
            continue;
        }
        let img = image::open(p).expect("读图失败");
        let v = check_turnaround(&img);
        let passed = matches!(v, Verdict::Ok);
        eprintln!(
            "  {label}: {}  {v:?}",
            if passed == should_pass { "✓" } else { "✗" }
        );
        assert_eq!(passed, should_pass, "{label} 判错了：{v:?}");
    }
}
