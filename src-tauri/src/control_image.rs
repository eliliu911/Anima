//! **程序化控制图** —— 不用任何模型，直接按分镜信息画出 ControlNet 的输入。
//!
//! # 为什么是程序化画的，不是从图里抽的
//!
//! 常见做法是拿一张图跑深度估计 / 骨架识别。出图阶段用不了这个办法：
//! **那时候还没有图**。先出草图再抽深度再重出，等于每个镜头跑两遍。
//!
//! 而我们手上有更便宜的东西：分镜表里本来就写着景别和人物站位。
//! 从这些信息直接画一张深度图或骨架图，**零推理成本**，几毫秒就好。
//! 精度当然不如真的深度估计，但 ControlNet 要的本来就是「大致的空间关系」，
//! 不是精确的毫米级深度。
//!
//! # 深度约定：越白越近
//!
//! 和 `depth_capture.rs` 那条链（Video Depth Anything）保持一致。写反了不报错，
//! 只会得到「前景虚化、背景清晰」这种整个反过来的画面。
//!
//! # 两件在别处踩过、这里必须避开的事
//!
//! - 控制图要写进 **ComfyUI 的 input 目录**，而且不能写死绝对路径 ——
//!   理由见 `cinema::comfy_input_dir` 的注释（写不进去时锁脸会静默失效）。
//! - Z-Image 的 ControlNet 挂在 **model** 上，SDXL 的挂在 **conditioning** 上，
//!   两边是反的。接线在 `image_engine::attach_zimage_control`。
//!
//! # ⚠️ 控制图的人数必须和提示词一致
//!
//! 2026-09-16 实跑踩到：拿一张**两人**的深度图配「a lone swordsman」的提示词，
//! 出来是两个撑满画面的半裸躯干 —— ControlNet 按结构画了两个人，
//! 提示词只说了一个，两边打架，结果谁也不对。
//!
//! 结构约束会**压过**提示词。所以 `subjects.len()` 要和镜头里实际几个人对上，
//! 调用方有责任保证这一点。不一致时不会报错，只会出废图。

use crate::error::Result;
use image::{Rgb, RgbImage};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 景别。决定人物在画面里占多高 —— 这是深度图里最关键的一个量，
/// 因为它同时决定了「人有多近」和「背景该虚化到什么程度」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShotSize {
    /// 特写：脸占大半个画面
    CloseUp,
    /// 近景：胸部以上
    MediumCloseUp,
    /// 中景：腰部以上
    Medium,
    /// 全景：全身，脚在画面内
    Full,
    /// 远景：人很小，环境为主 —— **写实风格的人脸扭曲就发生在这一档**
    Wide,
}

impl ShotSize {
    /// 人物高度占画面高度的比例。数字是按常规景别定义定的：
    /// 特写 ≈ 一张脸撑满，远景 ≈ 人只有画面的四分之一高。
    pub fn subject_height_ratio(self) -> f32 {
        match self {
            ShotSize::CloseUp => 1.35,
            ShotSize::MediumCloseUp => 1.05,
            ShotSize::Medium => 0.85,
            ShotSize::Full => 0.70,
            ShotSize::Wide => 0.26,
        }
    }

    /// 前景人物的深度值（0–255，越大越近）。
    /// 远景人物本来就离得远，给太亮会让 ControlNet 把他推到镜头前。
    pub fn subject_depth(self) -> u8 {
        match self {
            ShotSize::CloseUp => 255,
            ShotSize::MediumCloseUp => 246,
            ShotSize::Medium => 236,
            ShotSize::Full => 222,
            ShotSize::Wide => 170,
        }
    }

    /// 这一档该不该压景深（背景虚化）。
    ///
    /// 依据是用户 2026-09-15 的观察：**写实风格远距离人脸扭曲严重，
    /// 解决办法是控制景深**。远景和全景要压，特写本来就浅景深不用额外压。
    pub fn wants_shallow_depth_of_field(self) -> bool {
        matches!(self, ShotSize::Full | ShotSize::Wide)
    }
}

/// 画面里一个人的位置。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Subject {
    /// 横向位置，0.0 = 最左，1.0 = 最右
    pub x: f32,
    /// 相对这个景别的缩放。1.0 = 标准，>1 更近，<1 更远
    #[serde(default = "one")]
    pub scale: f32,
}

fn one() -> f32 {
    1.0
}

impl Subject {
    pub fn at(x: f32) -> Self {
        Subject { x, scale: 1.0 }
    }
}

/// 一张深度图要画成什么样。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepthSpec {
    pub width: u32,
    pub height: u32,
    pub shot_size: ShotSize,
    /// 画面里的人。空数组就只画环境（空镜）。
    pub subjects: Vec<Subject>,
    /// 地平线高度，0.0 = 画面顶，1.0 = 画面底。默认 0.45。
    #[serde(default = "default_horizon")]
    pub horizon: f32,
}

fn default_horizon() -> f32 {
    0.45
}

impl DepthSpec {
    pub fn single(width: u32, height: u32, shot_size: ShotSize) -> Self {
        DepthSpec {
            width,
            height,
            shot_size,
            subjects: vec![Subject::at(0.5)],
            horizon: default_horizon(),
        }
    }
}

/// 常用站位模板。
///
/// # 为什么要模板库而不是每次找参考图
///
/// 找参考照片这件事在分镜环节是**卡点**：一集几十个镜头，每个都去翻图库，
/// 比写提示词还慢。而短剧的站位其实高度收敛 —— 对峙、一主二从、群像这几种
/// 覆盖绝大多数镜头。模板直接从景别 + 人数算出 x 坐标，零成本。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Blocking {
    /// 单人居中
    Solo,
    /// 单人偏置（三分法）—— 留出空间给对话或环境
    SoloOffset,
    /// 两人对峙，左右对称
    Faceoff,
    /// 两人并肩，靠得近
    SideBySide,
    /// 一主二从：主角居中偏前，两个配角分列两侧稍远
    LeadAndTwo,
    /// 三人品字：一前二后
    Triangle,
    /// 群像：均匀铺开
    Crowd(u8),
}

impl Blocking {
    /// 算出每个人的横向位置和相对缩放。
    ///
    /// **`scale` 不是随便给的**：站得靠后的人要同时更小、更暗，
    /// 否则 ControlNet 会把他们拉到和主角同一个平面上，画面失去纵深。
    pub fn subjects(self) -> Vec<Subject> {
        match self {
            Blocking::Solo => vec![Subject::at(0.5)],
            Blocking::SoloOffset => vec![Subject::at(0.36)],
            Blocking::Faceoff => vec![Subject::at(0.27), Subject::at(0.73)],
            Blocking::SideBySide => vec![Subject::at(0.43), Subject::at(0.60)],
            Blocking::LeadAndTwo => vec![
                Subject { x: 0.50, scale: 1.0 },
                Subject { x: 0.22, scale: 0.82 },
                Subject { x: 0.78, scale: 0.82 },
            ],
            Blocking::Triangle => vec![
                Subject { x: 0.50, scale: 1.0 },
                Subject { x: 0.30, scale: 0.78 },
                Subject { x: 0.70, scale: 0.78 },
            ],
            Blocking::Crowd(n) => {
                let n = n.clamp(2, 8) as usize;
                (0..n)
                    .map(|i| {
                        let t = (i as f32 + 0.5) / n as f32;
                        // 两侧的人稍远一点，中间的稍近 —— 平铺会显得像一排纸片
                        let depth = 1.0 - (t - 0.5).abs() * 0.5;
                        Subject { x: 0.12 + t * 0.76, scale: depth }
                    })
                    .collect()
            }
        }
    }

    /// 中文名，给界面和日志用。
    pub fn name_zh(self) -> &'static str {
        match self {
            Blocking::Solo => "单人居中",
            Blocking::SoloOffset => "单人三分",
            Blocking::Faceoff => "两人对峙",
            Blocking::SideBySide => "两人并肩",
            Blocking::LeadAndTwo => "一主二从",
            Blocking::Triangle => "三人品字",
            Blocking::Crowd(_) => "群像",
        }
    }
}

impl DepthSpec {
    /// 按站位模板建一张深度图规格。
    pub fn blocked(width: u32, height: u32, shot_size: ShotSize, blocking: Blocking) -> Self {
        DepthSpec {
            width,
            height,
            shot_size,
            subjects: blocking.subjects(),
            horizon: default_horizon(),
        }
    }
}

/// 画一张深度图。越白越近。
pub fn depth_map(spec: &DepthSpec) -> RgbImage {
    let (w, h) = (spec.width.max(8), spec.height.max(8));
    let mut img = RgbImage::new(w, h);
    let horizon_y = (spec.horizon.clamp(0.05, 0.95) * h as f32) as u32;

    // 天空：最远，恒定的暗。地面：从地平线（远）到画面底（近）渐亮。
    for y in 0..h {
        let v = if y <= horizon_y {
            8u8
        } else {
            let t = (y - horizon_y) as f32 / (h - horizon_y).max(1) as f32;
            (18.0 + 148.0 * t) as u8
        };
        for x in 0..w {
            img.put_pixel(x, y, Rgb([v, v, v]));
        }
    }

    // 人物：一个椭圆当身体，上面一个小椭圆当头
    let base = spec.shot_size.subject_height_ratio();
    let depth = spec.shot_size.subject_depth();
    for s in &spec.subjects {
        let body_h = base * s.scale.max(0.05) * h as f32;
        // **半轴，不是全宽。** 站立人形的全宽约是身高的 0.25，所以半轴取 0.125。
        // 第一版写成 0.30（= 全宽 0.60），出来是两个撑满画面的巨大躯干 ——
        // ControlNet 忠实地照着那个形状画了。2026-09-16 实跑看出来的。
        let body_rx = body_h * 0.125;
        let cx = s.x.clamp(0.0, 1.0) * w as f32;
        // 脚落在地平线下方一点，越近的景别越靠下
        let feet_y = h as f32 * (spec.horizon + 0.55).min(1.08);
        let head_r = body_h * 0.075;
        // 躯干从「脚」往上占 body_h 的八成，剩下两成留给头 —— 头和躯干**必须相交**。
        // 分开画（中间留缝）会被读成「躯干 + 悬空的一个球」：水墨风里那两个球
        // 直接变成了天上的月亮。
        let torso_h = body_h * 0.82;
        let cy = feet_y - torso_h * 0.5;
        ellipse(&mut img, cx, cy, body_rx, torso_h * 0.5, depth);
        ellipse(
            &mut img,
            cx,
            cy - torso_h * 0.5 + head_r * 0.35,
            head_r,
            head_r * 1.2,
            depth.saturating_add(8),
        );
    }

    blur(&mut img, 5);
    img
}

/// 实心椭圆。`image` crate 不带绘图，自己写 —— 只需要这一个图元。
fn ellipse(img: &mut RgbImage, cx: f32, cy: f32, rx: f32, ry: f32, v: u8) {
    let (w, h) = (img.width() as i64, img.height() as i64);
    let (rx, ry) = (rx.max(1.0), ry.max(1.0));
    let x0 = ((cx - rx).floor() as i64).max(0);
    let x1 = ((cx + rx).ceil() as i64).min(w - 1);
    let y0 = ((cy - ry).floor() as i64).max(0);
    let y1 = ((cy + ry).ceil() as i64).min(h - 1);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = (x as f32 - cx) / rx;
            let dy = (y as f32 - cy) / ry;
            if dx * dx + dy * dy <= 1.0 {
                img.put_pixel(x as u32, y as u32, Rgb([v, v, v]));
            }
        }
    }
}

/// 盒式模糊，跑两遍近似高斯。
///
/// **必须模糊**：硬边的深度图会让 ControlNet 在边界上出硬切，
/// 人物像是被剪贴上去的。
fn blur(img: &mut RgbImage, radius: u32) {
    if radius == 0 {
        return;
    }
    for _ in 0..2 {
        let src = img.clone();
        let (w, h) = (img.width(), img.height());
        for y in 0..h {
            for x in 0..w {
                let mut sum = 0u32;
                let mut n = 0u32;
                let x0 = x.saturating_sub(radius);
                let x1 = (x + radius).min(w - 1);
                let y0 = y.saturating_sub(radius);
                let y1 = (y + radius).min(h - 1);
                for yy in y0..=y1 {
                    for xx in x0..=x1 {
                        sum += src.get_pixel(xx, yy)[0] as u32;
                        n += 1;
                    }
                }
                let v = (sum / n.max(1)) as u8;
                img.put_pixel(x, y, Rgb([v, v, v]));
            }
        }
    }
}

/// 把控制图写进 ComfyUI 的 input 目录，返回**文件名**（graph 里要的就是文件名）。
pub fn write_to_comfy_input(img: &RgbImage, stem: &str) -> Result<String> {
    let dir: PathBuf = crate::cinema::comfy_input_dir();
    std::fs::create_dir_all(&dir)?;
    let name = format!("{stem}.png");
    img.save(dir.join(&name))
        .map_err(|e| crate::error::Error::config(format!("控制图写不进 ComfyUI input：{e}")))?;
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lum(img: &RgbImage, xr: f32, yr: f32) -> u8 {
        img.get_pixel(
            (xr * (img.width() - 1) as f32) as u32,
            (yr * (img.height() - 1) as f32) as u32,
        )[0]
    }

    /// 深度约定：**越白越近**。天空必须比画面底部的地面暗。
    /// 写反了不报错，只会得到前后颠倒的虚化。
    #[test]
    fn 越白越近_天空比近处地面暗() {
        let img = depth_map(&DepthSpec::single(256, 256, ShotSize::Full));
        let sky = lum(&img, 0.05, 0.05);
        let near_ground = lum(&img, 0.05, 0.97);
        assert!(sky < near_ground, "天空 {sky} 应该比近处地面 {near_ground} 暗");
    }

    /// 人物位置要真的跟着 `x` 走 —— 这是整张图的意义所在。
    #[test]
    fn 人物位置跟着x走() {
        let mut left = DepthSpec::single(256, 256, ShotSize::Full);
        left.subjects = vec![Subject::at(0.2)];
        let mut right = left.clone();
        right.subjects = vec![Subject::at(0.8)];

        let l = depth_map(&left);
        let r = depth_map(&right);
        // 人物所在一侧应该更亮（更近）
        assert!(lum(&l, 0.2, 0.6) > lum(&l, 0.8, 0.6), "人在左边时左侧该更亮");
        assert!(lum(&r, 0.8, 0.6) > lum(&r, 0.2, 0.6), "人在右边时右侧该更亮");
    }

    /// 远景的人比全景的人小、也更暗（更远）。
    /// 这一条直接对应「写实风格远距离人脸扭曲」那个问题。
    #[test]
    fn 远景的人更小更远() {
        assert!(
            ShotSize::Wide.subject_height_ratio() < ShotSize::Full.subject_height_ratio()
        );
        assert!(ShotSize::Wide.subject_depth() < ShotSize::Full.subject_depth());
        assert!(ShotSize::Wide.wants_shallow_depth_of_field());
        assert!(!ShotSize::CloseUp.wants_shallow_depth_of_field());
    }

    /// 多人：每个人都要在图上留下痕迹，不能只画第一个。
    #[test]
    fn 多人各自成像() {
        let mut spec = DepthSpec::single(256, 256, ShotSize::Full);
        spec.subjects = vec![Subject::at(0.25), Subject::at(0.75)];
        let img = depth_map(&spec);
        let mid = lum(&img, 0.5, 0.6);
        assert!(lum(&img, 0.25, 0.6) > mid, "左边那个人没画出来");
        assert!(lum(&img, 0.75, 0.6) > mid, "右边那个人没画出来");
    }

    /// 深度图必须是模糊的 —— 硬边会让 ControlNet 在人物边界出硬切。
    #[test]
    fn 边界是软的() {
        let img = depth_map(&DepthSpec::single(256, 256, ShotSize::Full));
        // 沿人物边缘横扫，相邻像素的最大跳变不该像硬边那样接近满量程
        let y = (0.6 * 255.0) as u32;
        let mut max_jump = 0i32;
        for x in 1..img.width() {
            let a = img.get_pixel(x - 1, y)[0] as i32;
            let b = img.get_pixel(x, y)[0] as i32;
            max_jump = max_jump.max((a - b).abs());
        }
        assert!(max_jump < 60, "边界跳变 {max_jump} 太硬，模糊没生效");
    }

    /// 人眼验收用：把几档景别的深度图写出来，拿去 ComfyUI 实跑看效果。
    /// 默认 `#[ignore]` —— 它写文件，不是断言。
    #[test]
    #[ignore]
    fn 导出深度图供人眼验收() {
        let cases: &[(&str, ShotSize, Vec<Subject>)] = &[
            ("wide_single", ShotSize::Wide, vec![Subject::at(0.5)]),
            ("full_duo", ShotSize::Full, vec![Subject::at(0.3), Subject::at(0.7)]),
            ("closeup", ShotSize::CloseUp, vec![Subject::at(0.5)]),
        ];
        for (name, size, subs) in cases {
            let spec = DepthSpec {
                width: 1024,
                height: 1024,
                shot_size: *size,
                subjects: subs.clone(),
                horizon: 0.45,
            };
            let img = depth_map(&spec);
            let f = write_to_comfy_input(&img, &format!("anima_depth_{name}")).unwrap();
            println!("{f}");
        }
    }

    /// 人形不能太宽。站立人形全宽约为身高的 0.25 —— 第一版写成 0.60，
    /// ControlNet 忠实照画，出来是撑满画面的巨大躯干。
    #[test]
    fn 人形宽高比像个人() {
        let img = depth_map(&DepthSpec::single(512, 512, ShotSize::Full));
        let y = (0.75 * 511.0) as u32;
        let bg = img.get_pixel(4, y)[0] as i32;
        let wide: u32 = (0..img.width())
            .filter(|&x| img.get_pixel(x, y)[0] as i32 > bg + 25)
            .count() as u32;
        let ratio = wide as f32 / img.width() as f32;
        assert!(
            (0.05..0.30).contains(&ratio),
            "躯干占画面宽度 {ratio:.2}，不像一个站着的人"
        );
    }

    /// 头和躯干必须相交 —— 中间留缝会被读成「躯干 + 悬空的球」。
    /// 实跑见过：水墨风里那两个球变成了天上的月亮。
    #[test]
    fn 头和躯干相连没有缝() {
        let img = depth_map(&DepthSpec::single(512, 512, ShotSize::CloseUp));
        let cx = img.width() / 2;
        let bg = img.get_pixel(4, img.height() / 2)[0] as i32;
        // 从头顶往下扫中轴，人物区域必须是**一整段连续**的亮区，不能断开
        let col: Vec<bool> = (0..img.height())
            .map(|y| img.get_pixel(cx, y)[0] as i32 > bg + 25)
            .collect();
        let runs = col
            .windows(2)
            .filter(|w| w[0] != w[1])
            .count();
        assert!(runs <= 2, "中轴上人物断成了 {} 段，头和躯干没连上", runs / 2 + 1);
    }

    /// 站位模板：人数对、顺序对、不重叠。
    #[test]
    fn 站位模板人数与位置() {
        assert_eq!(Blocking::Solo.subjects().len(), 1);
        assert_eq!(Blocking::Faceoff.subjects().len(), 2);
        assert_eq!(Blocking::LeadAndTwo.subjects().len(), 3);
        assert_eq!(Blocking::Crowd(5).subjects().len(), 5);

        for b in [
            Blocking::Solo,
            Blocking::SoloOffset,
            Blocking::Faceoff,
            Blocking::SideBySide,
            Blocking::LeadAndTwo,
            Blocking::Triangle,
            Blocking::Crowd(4),
        ] {
            let subs = b.subjects();
            for s in &subs {
                assert!((0.0..=1.0).contains(&s.x), "{} 的 x 越界: {}", b.name_zh(), s.x);
                assert!(s.scale > 0.0, "{} 的 scale 非正", b.name_zh());
            }
            // 不能有两个人挤在同一个位置
            for i in 0..subs.len() {
                for j in (i + 1)..subs.len() {
                    assert!(
                        (subs[i].x - subs[j].x).abs() > 0.05,
                        "{} 里第 {i} 和第 {j} 个人重叠了",
                        b.name_zh()
                    );
                }
            }
        }
    }

    /// 一主二从：主角必须比配角**更大**（更近），否则画面没有主次。
    #[test]
    fn 一主二从的主角更近() {
        let s = Blocking::LeadAndTwo.subjects();
        assert!(s[0].scale > s[1].scale && s[0].scale > s[2].scale, "主角没有更近");
        assert!((s[0].x - 0.5).abs() < 0.05, "主角该在中间");
    }

    /// 群像不能是一排等大的纸片 —— 两侧要比中间远。
    #[test]
    fn 群像有纵深() {
        let s = Blocking::Crowd(6).subjects();
        let mid = s.len() / 2;
        assert!(s[mid].scale > s[0].scale, "中间的人该比最左边的近");
        assert!(s[mid].scale > s[s.len() - 1].scale, "中间的人该比最右边的近");
    }

    /// 模板画出来的深度图，每个人都要在图上真的出现。
    #[test]
    fn 站位模板真的画进图里() {
        let spec = DepthSpec::blocked(256, 256, ShotSize::Full, Blocking::Faceoff);
        let img = depth_map(&spec);
        let mid = lum(&img, 0.5, 0.6);
        assert!(lum(&img, 0.27, 0.6) > mid, "左边的人没画出来");
        assert!(lum(&img, 0.73, 0.6) > mid, "右边的人没画出来");
    }

    /// 空镜：没有人物时不该凭空画出人来。
    #[test]
    fn 空镜没有人物() {
        let mut spec = DepthSpec::single(256, 256, ShotSize::Wide);
        spec.subjects = vec![];
        let img = depth_map(&spec);
        // 同一水平线上应该处处相等（只有地面渐变，没有椭圆）
        let y = (0.6 * 255.0) as u32;
        let first = img.get_pixel(0, y)[0] as i32;
        for x in 0..img.width() {
            assert!(
                (img.get_pixel(x, y)[0] as i32 - first).abs() <= 2,
                "空镜里不该有横向起伏"
            );
        }
    }
}
