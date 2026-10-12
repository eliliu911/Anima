//! 等距圆柱全景 → 四方位正交视图。
//!
//! # 为什么在 Rust 里做，不装 ComfyUI 节点
//!
//! 用户提的方案是「用 `Equirectangular To Cubemap` 节点自动裁切」。
//! 本机没装那个节点，而**不该为这件事去装**：
//!
//! 1. 切片是纯数学，装个自定义节点等于给一段十几行的三角函数引入一个
//!    外部依赖。而这个仓库对缺节点的态度已经写在 CLAUDE.md 里 ——
//!    「缺了直接停，照跑会出一张没版式的普通图而任务显示成功」。
//! 2. 宫格那条路**已经在 Rust 里切**（[`super::scene_sheet::split_sheet`]），
//!    切完每格入库。全景切片是同一形状的操作，插在同一个位置。
//! 3. 纯函数能在没有 ComfyUI 的情况下单测。
//!
//! # 和宫格的根本区别
//!
//! 宫格是**求**模型画四个不同机位 —— 提示词说得再细也可能出四张一样的
//! （2026-09-12 实拍过）。全景切片是**算**出来的：四个视角取自同一个
//! 球面投影，柱子位置、光线、材质、色温必然一致。
//! 一致性从「提示词约束」变成了「几何保证」。
//!
//! # 已知缺陷：接缝
//!
//! 360Redmond 出的全景左右边缘并不闭合。实测两个 seed：
//! 接缝差 / 随机两列平均差 ≈ 0.43 与 0.51（闭合的话该接近 0），
//! 提示词里加 "seamless / edges continuous" 没有改善 —— 是这个 LoRA
//! 在这个分辨率上的系统性特征。
//!
//! 后果是**只有跨越 ±180° 的那个视角会看见一条竖直接缝**。
//! [`seam_score`] 把这件事量化出来，调用方要把它说给用户听 ——
//! 不说的话，四张图里有一张带缝，而那张单看很容易被当成「构图问题」。

use image::{DynamicImage, GenericImageView, RgbImage};

/// 出全景必须挂的那个 LoRA。
///
/// **缺了要当场停，不能照跑。** 不挂它出来的是一张普通宽图 ——
/// 看着像图、比例也对，但根本不是等距圆柱投影，切出来的四个角度全错，
/// 而且**不报错**。这个仓库对缺依赖的态度写在 CLAUDE.md 里：
/// 「缺了直接停，照跑会出一张没版式的普通图而任务显示成功」。
pub const PANO_LORA: &str = "zimage/360viewer_Redmond_zit.safetensors";

/// 触发词。Civitai #118025 的 `trainedWords`，逐字照抄。
pub const PANO_TRIGGER: &str = "360 view, 360, equirectangular panorama";

/// 全景只能由 Z-Image 出 —— 我们装的是它的 Z-Image Turbo 版，
/// 张量键名确认过是 DiT 架构。挂到别的底模上不生效**且不报错**。
pub const PANO_ENGINE: &str = "zimage";

/// 一个要切出来的视角。
#[derive(Debug, Clone, PartialEq)]
pub struct ViewAngle {
    /// 给人看的名字，会进文件名。
    pub name: String,
    /// 水平转角，度。0 = 全景图正中，正数向右。
    pub yaw: f32,
    /// 俯仰角，度。正数抬头。**场景板一律 0** —— 平视是可导航空间的基准。
    pub pitch: f32,
    /// 水平视场角，度。
    pub fov: f32,
}

/// 默认四方位：正 / 右 / 后 / 左。
///
/// **fov 用 90°。** 四个 90° 正好拼满一圈，相邻视角边缘相接不重叠 ——
/// 这就是 cubemap 的四个侧面。再大会把等距圆柱两极的拉伸带进画面。
pub fn quad_views() -> Vec<ViewAngle> {
    [("正面", 0.0), ("右侧", 90.0), ("背面", 180.0), ("左侧", 270.0)]
        .into_iter()
        .map(|(name, yaw)| ViewAngle { name: name.into(), yaw, pitch: 0.0, fov: 90.0 })
        .collect()
}

/// 把四个角度整体转一下。
///
/// **用来躲接缝。** 接缝固定在全景图的左右边缘，也就是 yaw=±180 ——
/// 默认切法正好把它切在「背面」那一格的正中。整体偏 45° 之后接缝落到
/// 两格交界处，四格都不带缝。
///
/// 代价是**锚点不再正对**：龙椅在 yaw≈0 时，偏 45° 会把它切在画面边缘。
/// 所以这是调用方的取舍，不是默认行为。
pub fn rotated(views: &[ViewAngle], by_deg: f32) -> Vec<ViewAngle> {
    views
        .iter()
        .map(|v| ViewAngle { yaw: v.yaw + by_deg, ..v.clone() })
        .collect()
}

/// 左右边缘闭不闭合。**越小越好，闭合的全景接近 0。**
///
/// 判据是「最左一列和最右一列的平均通道差」除以「随机若干对列的平均差」——
/// 除一下是为了对画面本身的对比度归一化：一张高对比的图，绝对差天然更大。
///
/// 返回 `None`：图太窄（少于 4 列），比不了。
pub fn seam_score(img: &DynamicImage) -> Option<f32> {
    let (w, h) = img.dimensions();
    if w < 4 || h == 0 {
        return None;
    }
    let rgb = img.to_rgb8();
    let col_diff = |a: u32, b: u32| -> f32 {
        let mut sum = 0f32;
        for y in 0..h {
            let pa = rgb.get_pixel(a, y).0;
            let pb = rgb.get_pixel(b, y).0;
            for c in 0..3 {
                sum += (pa[c] as f32 - pb[c] as f32).abs();
            }
        }
        sum / (h as f32 * 3.0)
    };
    let seam = col_diff(0, w - 1);
    // 参照：等距抽若干对列。**不用随机** —— 同一张图两次跑出不同的分数
    // 会让人怀疑是不是图变了。
    let mut base = 0f32;
    let n = 16u32;
    for k in 0..n {
        let a = (w * k) / n;
        let b = (w * (k + n / 2)) / n % w;
        base += col_diff(a, b);
    }
    base /= n as f32;
    if base <= f32::EPSILON {
        // 纯色图 —— 接缝当然是 0，但比值没有意义
        return Some(0.0);
    }
    Some(seam / base)
}

/// 从等距圆柱全景里渲一个透视视图。
///
/// # 这套数学和 `panoMath.ts` / `panoStage.ts` 是同一套
///
/// 画布上那个 360° 查看器用 three.js 的 BackSide 球面做同一件事。
/// 两边分叉的表现是「画布里转到的角度和切出来的那一格不一样」，
/// 而那不报错。约定：**yaw=0 对应全景图水平正中**，向右为正。
pub fn render_view(pano: &DynamicImage, view: &ViewAngle, size: u32) -> RgbImage {
    let (pw, ph) = pano.dimensions();
    let src = pano.to_rgb8();
    let yaw = view.yaw.to_radians();
    let pitch = view.pitch.to_radians();
    let f = (size as f32 / 2.0) / (view.fov.to_radians() / 2.0).tan();
    let mut out = RgbImage::new(size, size);

    for j in 0..size {
        for i in 0..size {
            let x = (j as f32 - size as f32 / 2.0) / f;
            let y = (i as f32 - size as f32 / 2.0) / f;
            let z = 1.0f32;
            // 先俯仰（绕 X），再水平（绕 Y）—— 顺序反了会在抬头时带上滚转
            let (yp, zp) = (y * pitch.cos() - z * pitch.sin(), y * pitch.sin() + z * pitch.cos());
            let (xr, zr) = (x * yaw.cos() + zp * yaw.sin(), -x * yaw.sin() + zp * yaw.cos());
            let lon = xr.atan2(zr);
            let lat = yp.atan2((xr * xr + zr * zr).sqrt());

            let u = (lon / std::f32::consts::TAU + 0.5) * pw as f32;
            let v = (lat / std::f32::consts::PI + 0.5) * ph as f32;
            // **横向取模不是夹紧。** 全景是环状的，u 越界要绕回去 ——
            // 夹紧的话画面边缘会糊成一条竖线。
            let ui = ((u.floor() as i64).rem_euclid(pw as i64)) as u32;
            let vi = (v.floor().clamp(0.0, (ph - 1) as f32)) as u32;
            out.put_pixel(j, i, *src.get_pixel(ui, vi));
        }
    }
    out
}

/// 切出所有视角，各存一张 PNG。返回写成功的那些。
///
/// 命名和宫格切格一致：`<全景图名>_<序号>_<视角名>.png` ——
/// 序号在前排序才稳，名字在后人才认得出。
///
/// **某一格写盘失败时跳过它继续**，和 `split_sheet` 同一条：
/// 让四格里坏一格就整个失败，是把部分可用的结果变成完全没有。
pub fn split_panorama(
    pano_path: &std::path::Path,
    views: &[ViewAngle],
    size: u32,
) -> Result<Vec<std::path::PathBuf>, String> {
    let img = image::open(pano_path)
        .map_err(|e| format!("读不出全景图 {}：{e}", pano_path.display()))?;
    let (w, h) = img.dimensions();
    // **不是 2:1 就不是等距圆柱。** 硬切出来的东西看着像图、角度全错，
    // 而且不报错 —— 这正是这个仓库最常见的失败形状。
    let ratio = w as f32 / h.max(1) as f32;
    if !(1.9..=2.1).contains(&ratio) {
        return Err(format!(
            "{}×{} 不是等距圆柱全景（比例 {ratio:.2}，必须接近 2.00）—— \
             切出来的四个角度会全错，而图看着是正常的",
            w, h
        ));
    }
    let stem = pano_path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let dir = pano_path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let mut out = Vec::new();
    for (i, v) in views.iter().enumerate() {
        let target = dir.join(format!("{stem}_{}_{}.png", i + 1, v.name.trim()));
        match render_view(&img, v, size).save(&target) {
            Ok(()) => out.push(target),
            Err(e) => tracing::warn!("第 {} 格写盘失败，跳过：{e}", i + 1),
        }
    }
    if out.is_empty() {
        return Err("一格都没切出来".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    /// 造一张「每 90° 一个纯色」的等距圆柱图：正=红 右=绿 后=蓝 左=黄。
    ///
    /// 这样每个视角切出来应该是**一个已知颜色**，角度错了立刻看得出来。
    fn 四色全景(w: u32, h: u32) -> DynamicImage {
        let mut img = RgbImage::new(w, h);
        for x in 0..w {
            // yaw=0 在图的正中；四个色块各占 90°
            let lon_deg = ((x as f32 + 0.5) / w as f32 - 0.5) * 360.0;
            let c = match lon_deg {
                d if (-45.0..45.0).contains(&d) => Rgb([255, 0, 0]),     // 正面 红
                d if (45.0..135.0).contains(&d) => Rgb([0, 255, 0]),     // 右侧 绿
                d if (-135.0..-45.0).contains(&d) => Rgb([255, 255, 0]), // 左侧 黄
                _ => Rgb([0, 0, 255]),                                   // 背面 蓝
            };
            for y in 0..h {
                img.put_pixel(x, y, c);
            }
        }
        DynamicImage::ImageRgb8(img)
    }

    fn 正中像素(img: &RgbImage) -> [u8; 3] {
        img.get_pixel(img.width() / 2, img.height() / 2).0
    }

    /// **四个方位要真的落在四个方向上。**
    ///
    /// 角度算错的表现是四张图看着都像全景的一部分、但对应的方向全不对 ——
    /// 而那不报错。四色全景让这件事变成一个可断言的事实。
    #[test]
    fn 四方位各自取到正确的方向() {
        let pano = 四色全景(720, 360);
        let got: Vec<[u8; 3]> = quad_views()
            .iter()
            .map(|v| 正中像素(&render_view(&pano, v, 64)))
            .collect();
        assert_eq!(
            got,
            vec![[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]],
            "正/右/后/左 取到的方向不对"
        );
    }

    /// **yaw 向右为正**，和 `panoMath.ts` 约定一致。反了的话画布里
    /// 转到的角度和切出来的那一格对不上，而两边都不报错。
    #[test]
    fn yaw_正数是向右转() {
        let pano = 四色全景(720, 360);
        let right = ViewAngle { name: "x".into(), yaw: 90.0, pitch: 0.0, fov: 60.0 };
        assert_eq!(正中像素(&render_view(&pano, &right, 32)), [0, 255, 0], "yaw=+90 该是右侧那块绿");
    }

    /// 转一圈回到原地 —— 横向必须取模，不能夹紧。
    #[test]
    fn 转满一圈等于没转() {
        let pano = 四色全景(720, 360);
        let a = ViewAngle { name: "x".into(), yaw: 0.0, pitch: 0.0, fov: 60.0 };
        let b = ViewAngle { yaw: 360.0, ..a.clone() };
        assert_eq!(render_view(&pano, &a, 48).into_raw(), render_view(&pano, &b, 48).into_raw());
    }

    /// 负角度也要绕回去（`rem_euclid`，不是 `%`）。
    #[test]
    fn 负角度绕得回去() {
        let pano = 四色全景(720, 360);
        let neg = ViewAngle { name: "x".into(), yaw: -90.0, pitch: 0.0, fov: 60.0 };
        assert_eq!(正中像素(&render_view(&pano, &neg, 32)), [255, 255, 0], "yaw=-90 该是左侧那块黄");
    }

    /// **偏 45° 是用来躲接缝的**，四个角度整体转，相对关系不变。
    #[test]
    fn 整体偏转不改变相对关系() {
        let v = rotated(&quad_views(), 45.0);
        let yaws: Vec<f32> = v.iter().map(|x| x.yaw).collect();
        assert_eq!(yaws, vec![45.0, 135.0, 225.0, 315.0]);
        assert_eq!(v[0].name, "正面", "名字不该跟着转");
    }

    /// **闭合的全景接缝分数接近 0，不闭合的明显更高。**
    #[test]
    fn 接缝分数分得出闭合与否() {
        // 四色全景左右都是蓝（背面那块跨越边缘）—— 天然闭合
        let seamless = seam_score(&四色全景(720, 360)).unwrap();
        assert!(seamless < 0.05, "闭合的全景却报了 {seamless}");

        // 左半红右半绿 —— 边缘不连
        let mut img = RgbImage::new(720, 360);
        for x in 0..720 {
            let c = if x < 360 { Rgb([255, 0, 0]) } else { Rgb([0, 255, 0]) };
            for y in 0..360 {
                img.put_pixel(x, y, c);
            }
        }
        let broken = seam_score(&DynamicImage::ImageRgb8(img)).unwrap();
        assert!(broken > 0.5, "明显不闭合却只报了 {broken}");
    }

    /// **不是 2:1 就当场报错。**
    ///
    /// 硬切出来的东西看着像图、角度全错，而且不报错 ——
    /// 这正是这个仓库最常见的失败形状。
    #[test]
    fn 非2比1的图要拒绝() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("方图.png");
        RgbImage::new(512, 512).save(&p).unwrap();
        let err = split_panorama(&p, &quad_views(), 64).unwrap_err();
        assert!(err.contains("等距圆柱"), "{err}");
        assert!(err.contains("2.00"), "错误里要说清楚该是多少：{err}");
    }

    /// 切完四张，文件名带序号和方位名。
    #[test]
    fn 切出四张并且名字认得出() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("金銮殿.png");
        四色全景(720, 360).save(&p).unwrap();
        let out = split_panorama(&p, &quad_views(), 64).unwrap();
        assert_eq!(out.len(), 4);
        let names: Vec<String> =
            out.iter().map(|x| x.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names[0], "金銮殿_1_正面.png");
        assert_eq!(names[3], "金銮殿_4_左侧.png");
    }
}
