//! 视角分类对着**真实出图**验，真值是我逐格看出来的。
//!
//! ## 为什么要存图进仓库
//!
//! 这套判据的每一个常量都是从这几张图上量出来的（见 `cinema_qc.rs` 里
//! 那两张表）。图不在，常量就变成了魔法数字，谁都不敢动 ——
//! 而它必须能动：换底模、换 LoRA、换引擎都可能让分布整体平移。
//!
//! 图缩到 768 宽是为了别把仓库撑大。判据全是**相对量**（相对本图最宽的
//! 视角、相对头部区域），所以缩放不影响结果 —— 这条本身也被下面测着。
//!
//! ## 这里守的失败长什么样
//!
//! 全都不报错：切图切在两个人中间、「背面」那格显示侧身、
//! 整排明明没有背面却报成有。用户看到的只是「图不对」。

use anima_lib::cinema_qc::{classify_views, pick_views, view_bounds, View};
use std::path::PathBuf;

fn fixture(name: &str) -> image::DynamicImage {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/sheets")
        .join(name);
    image::open(&p).unwrap_or_else(|e| panic!("读不到 {}: {e}", p.display()))
}

use View::{Back, Front, Side};

/// SDXL 那条路（真值：正 / 侧 侧 侧 / 背 背）。
///
/// **这里只断言契约，不断言逐格全对 —— 因为做不到。**
/// SDXL 的整排里人物是肩挨着肩的，中间常常没有背景缝隙，
/// 分割靠的是忙碌度曲线的周期性，实测会偶尔把一格切歪一点。
/// 切歪的那一格宽高比就不准，跟着判错。
///
/// 能保证的是下面这三条，而它们才是界面上真正要的东西。
/// 想要逐格都准，正路是换成版式固定的整排（见 SD1.5 那条），
/// 而不是继续拧这里的阈值。
#[test]
fn sdxl整排的三格挑得对() {
    let img = fixture("sdxl_front_side3_back2.jpg");
    let v = classify_views(&img);

    assert_eq!(v.first(), Some(&Front), "第一格是正面：{v:?}");
    assert!(v.contains(&Back), "这张整排里确实有背面，得找得出来：{v:?}");
    assert!(v.contains(&Side), "也得找得出侧面：{v:?}");

    let [f, s, b] = pick_views(&img).expect("六格应该挑得出三格");
    assert_eq!(v[f], Front);
    assert_eq!(v[b], Back);
    assert!(s != f && s != b, "三格不能重复：{:?}", [f, s, b]);

    // 挑出来的「背面」必须真的是背面 —— 这正是当初翻车的地方：
    // 那一格被判成 Back，实际显示的是侧身
    let m = anima_lib::cinema_qc::view_metrics(&img);
    assert!(
        m[b].face_skin < 0.12,
        "「背面」那格露着脸，肤色 {:.3}",
        m[b].face_skin
    );
}

/// SD1.5 引擎的产出：正 / 侧 侧 / 背。
///
/// 它和 SDXL 那张的**格宽差很多**（SD1.5 的人物之间有真背景间隔），
/// 这正是当初拿「占格宽的比例」当判据会翻车的地方。
#[test]
fn sd15整排逐格判对() {
    assert_eq!(
        classify_views(&fixture("sd15_front_side2_back.jpg")),
        vec![Front, Side, Side, Back]
    );
}

/// **反例**：这张整排里根本没有背面（是「正侧侧侧侧正」）。
///
/// 它同时是最难的一张 —— 中间有个半透明残影和后一个人叠在一起。
/// 这里只要求别把第一格判错，以及挑格不会返回重复下标；
/// 逐格判对做不到，如实写下来，不假装。
#[test]
fn 难切的整排至少不会挑出重复的三格() {
    let img = fixture("sdxl_no_back.jpg");
    let v = classify_views(&img);
    assert_eq!(
        v.first(),
        Some(&Front),
        "第一格是正面，这条底线要守住：{v:?}"
    );

    let picked = pick_views(&img).expect("四格以上应该挑得出三格");
    let uniq: std::collections::HashSet<_> = picked.iter().collect();
    assert_eq!(
        uniq.len(),
        3,
        "三格重复时界面上看着像少生成了一格：{picked:?}"
    );
}

/// 判据必须是相对量 —— 缩放一半结果不变。
///
/// 早先躯干宽度是「占**格宽**的比例」，而格宽由分割算法定，
/// 和人物转没转身毫无关系：同一个人在宽格子里显得窄、窄格子里显得宽。
#[test]
fn 缩放不改变判断() {
    let img = fixture("sd15_front_side2_back.jpg");
    let half = img.resize(
        img.width() / 2,
        img.height() / 2,
        image::imageops::FilterType::Lanczos3,
    );
    assert_eq!(classify_views(&img), classify_views(&half));
}

/// 切图和分类必须共用同一份边界，否则切出来的不是判断的那一格。
#[test]
fn 挑出的下标落在边界数组内() {
    for name in [
        "sdxl_front_side3_back2.jpg",
        "sd15_front_side2_back.jpg",
        "sd15_engine_3view.jpg",
    ] {
        let img = fixture(name);
        let n = view_bounds(&img).len();
        assert_eq!(
            n,
            classify_views(&img).len(),
            "{name}: 边界数和分类数必须一致"
        );
        if let Some(idx) = pick_views(&img) {
            for i in idx {
                assert!(i < n, "{name}: 挑到了越界下标 {i} / {n}");
            }
        }
    }
}
