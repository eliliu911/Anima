//! 七分区参考表（`entity_sheet`）的「表情研究」补表情。
//!
//! # 为什么要有
//!
//! `DynamicCharacterSheet` LoRA 出的表情区**三张脸几乎一样**（见 `entity_sheet` 模块注释），尺寸够、变化出不来。
//! 2026-09-15 照 ComfyUI_VNCCS 的做法试了两轮（小雨那张）：把三张脸各自裁出来 → 放大 → Z-Image 图生图按表情重画 →
//! 缩回 → 只贴脸部椭圆。第二轮定稿：开心 0.55、生气 0.7、惊讶 0.7，遮罩要**罩住眉毛**（第一轮压在眉毛下面，生气改不出来），
//! 表情描述要写成动漫式（八字眉、怒气符号、冷汗）。发型、衣服、整张表都不动，贴回看不出接缝。
//!
//! # 位置要找，不能写死
//!
//! 同一个 LoRA 出的四张表，表情区上下左右差几十像素（猫那两张更靠下）。所以在表情区大致范围里
//! **按背景色把三个头分开**：上半截（头发那一段）按列统计前景，连成三段就是三个头；认不出正好三个就跳过、原图不动。
//! 脸部裁剪框和遮罩都按「头」的外框比例算。

use std::path::{Path, PathBuf};

use image::{imageops, GrayImage, Luma, Rgb, RgbImage};

use crate::error::{Error, Result};

/// 表情区里的一个头（原图像素坐标）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaceCell {
    /// 头的外框：左、上、右、下
    pub head: (u32, u32, u32, u32),
}

impl FaceCell {
    fn w(&self) -> u32 {
        self.head.2 - self.head.0
    }
    fn h(&self) -> u32 {
        self.head.3 - self.head.1
    }
    pub fn cx(&self) -> u32 {
        (self.head.0 + self.head.2) / 2
    }
    /// 送去重画的方形裁剪框（左、上、边长），比头略小一点，居中
    pub fn crop(&self, img_w: u32, img_h: u32) -> (u32, u32, u32) {
        let side = ((self.w().max(self.h()) as f32) * 0.97).round() as u32;
        let side = side.min(img_w).min(img_h);
        let cy = self.head.1 + (self.h() as f32 * 0.48) as u32;
        let x = self.cx().saturating_sub(side / 2).min(img_w - side);
        let y = cy.saturating_sub(side / 2).min(img_h - side);
        (x, y, side)
    }
    /// 贴回的脸部椭圆：横向头宽的 37%，纵向从头顶往下 17% 到 78%（刘海下沿罩住眉毛，到下巴）
    pub fn face_ellipse(&self) -> (f32, f32, f32, f32) {
        let h = self.h() as f32;
        let top = self.head.1 as f32 + h * 0.17;
        let bottom = self.head.1 as f32 + h * 0.78;
        (self.cx() as f32, (top + bottom) / 2.0, self.w() as f32 * 0.37, (bottom - top) / 2.0)
    }
}

/// 表情区在 1536×1024 版面里的大致范围（按实际尺寸等比换算）。标题行以下、标签行以上，
/// 左边避开剪影、右边避开细节区的分隔线
const REGION: (f32, f32, f32, f32) = (400.0, 716.0, 975.0, 960.0);

fn dist(a: &Rgb<u8>, b: &Rgb<u8>) -> u32 {
    (0..3).map(|i| (a[i] as i32 - b[i] as i32).unsigned_abs()).sum()
}

/// 区域里最常见的颜色（量化到 16 级）当背景
fn background(img: &RgbImage, x0: u32, y0: u32, x1: u32, y1: u32) -> Rgb<u8> {
    let mut counts = std::collections::HashMap::<(u8, u8, u8), (u32, [u64; 3])>::new();
    for y in (y0..y1).step_by(2) {
        for x in (x0..x1).step_by(2) {
            let p = img.get_pixel(x, y);
            let e = counts.entry((p[0] >> 4, p[1] >> 4, p[2] >> 4)).or_insert((0, [0; 3]));
            e.0 += 1;
            for i in 0..3 {
                e.1[i] += p[i] as u64;
            }
        }
    }
    let (n, sum) = counts.into_values().max_by_key(|v| v.0).unwrap_or((1, [255; 3]));
    Rgb([(sum[0] / n as u64) as u8, (sum[1] / n as u64) as u8, (sum[2] / n as u64) as u8])
}

/// 找表情区的三个头。认不出正好三个、或者大小差太多，回 `None`
pub fn find_faces(img: &RgbImage) -> Option<[FaceCell; 3]> {
    let (w, h) = img.dimensions();
    let sx = w as f32 / 1536.0;
    let sy = h as f32 / 1024.0;
    let (x0, y0, x1, y1) = ((REGION.0 * sx) as u32, (REGION.1 * sy) as u32, ((REGION.2 * sx) as u32).min(w), ((REGION.3 * sy) as u32).min(h));
    if x1 <= x0 + 30 || y1 <= y0 + 30 {
        return None;
    }
    let bg = background(img, x0, y0, x1, y1);
    let fg = |x: u32, y: u32| dist(img.get_pixel(x, y), &bg) > 60;

    // 上面 40% 按列统计：头顶那一段相邻的头是分开的（再往下波波头的发梢、肩膀常常挨在一起）。
    // 跳过最上面一小条：标题「EXPRESSION STUDY」有时压进这个范围（猫那张）。
    // 实测真表上相邻两个头发之间只空 7 像素左右，所以断开的门槛只能是 3
    let band_top = y0 + (14.0 * sy) as u32;
    let band = y0 + (((y1 - y0) as f32) * 0.4) as u32;
    let on: Vec<bool> = (x0..x1).map(|x| (band_top..band).filter(|&y| fg(x, y)).count() >= 4).collect();
    let mut runs: Vec<(u32, u32)> = Vec::new();
    let gap = ((3.0 * sx) as u32).max(1);
    for (i, &v) in on.iter().enumerate() {
        if !v {
            continue;
        }
        let x = x0 + i as u32;
        match runs.last_mut() {
            Some(r) if x <= r.1 + gap + 1 => r.1 = x,
            _ => runs.push((x, x)),
        }
    }
    let min_w = (60.0 * sx) as u32;
    runs.retain(|r| r.1 - r.0 >= min_w);
    if runs.len() != 3 {
        return None;
    }
    let widths: Vec<u32> = runs.iter().map(|r| r.1 - r.0).collect();
    if *widths.iter().max()? as f32 > *widths.iter().min()? as f32 * 1.6 {
        return None;
    }
    let cells: Vec<FaceCell> = runs
        .iter()
        .map(|&(a, b)| {
            // 头顶：从头的中段往上走，走到连着 3 行都几乎没有前景为止（标题和头之间有一段空白）
            let row = |y: u32| (a..=b).filter(|&x| fg(x, y)).count();
            let mut top = band;
            let mut empty = 0;
            let mut y = band;
            while y > y0.saturating_sub((20.0 * sy) as u32) {
                y -= 1;
                if row(y) >= 2 {
                    top = y;
                    empty = 0;
                } else {
                    empty += 1;
                    if empty >= 3 {
                        break;
                    }
                }
            }
            FaceCell { head: (a, top, b + 1, y1) }
        })
        .collect();
    if cells.iter().any(|c| c.h() < (80.0 * sy) as u32) {
        return None;
    }
    Some([cells[0], cells[1], cells[2]])
}

/// 三格各出什么表情、多大强度。**顺序就是表情区从左到右**
pub const EXPRESSIONS: [(&str, f64, &str); 3] = [
    ("开心", 0.55, "开心地笑着，眼睛笑成弯弯的月牙，嘴巴张开露出笑容。"),
    ("生气", 0.7, "非常生气：眉毛向眉心倒竖成八字形、紧紧皱起，眼睛眯起、眼神凶狠，嘴角向下撇、咬着牙，额角有动漫式的愤怒井字符号，脸颊微红。"),
    ("惊讶", 0.7, "大吃一惊：眼睛睁得又大又圆、瞳孔缩小，眉毛高高扬起，嘴巴张大成O形，额头挂着一滴冷汗。"),
];

/// 重画一格用的提示词。风格和身份用参考表那次的零件（没有就只说「和原图同样的画风」）
pub fn face_prompt(name: &str, style_line: Option<&str>, identity_locks: &[String], expression: &str) -> String {
    let style = style_line.map(str::trim).filter(|s| !s.is_empty()).unwrap_or("和原图同样的画风");
    let who = if name.trim().is_empty() { "这个角色".to_string() } else { name.trim().to_string() };
    let locks: Vec<&str> = identity_locks.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    let locks = if locks.is_empty() { String::new() } else { format!("{}。", locks.join("，")) };
    format!("{style}，{who}的头像特写。{locks}{who}{expression}保持原有发型、脸型、五官比例和配色不变。干净的纯色背景。")
}

/// 羽化椭圆遮罩（整张图大小）
fn ellipse_mask(w: u32, h: u32, (cx, cy, rx, ry): (f32, f32, f32, f32), feather: f32) -> GrayImage {
    let mut m = GrayImage::new(w, h);
    let (xa, xb) = ((cx - rx - 2.0).max(0.0) as u32, ((cx + rx + 2.0) as u32).min(w));
    let (ya, yb) = ((cy - ry - 2.0).max(0.0) as u32, ((cy + ry + 2.0) as u32).min(h));
    for y in ya..yb {
        for x in xa..xb {
            let dx = (x as f32 - cx) / rx;
            let dy = (y as f32 - cy) / ry;
            if dx * dx + dy * dy <= 1.0 {
                m.put_pixel(x, y, Luma([255]));
            }
        }
    }
    imageops::blur(&m, feather)
}

/// 把重画好的脸（任意尺寸）缩回裁剪框大小，按遮罩贴回
pub fn paste_face(sheet: &mut RgbImage, face: &RgbImage, cell: &FaceCell) {
    let (w, h) = sheet.dimensions();
    let (x, y, side) = cell.crop(w, h);
    let face = imageops::resize(face, side, side, imageops::FilterType::Lanczos3);
    let feather = (side as f32 / 32.0).max(3.0);
    let mask = ellipse_mask(w, h, cell.face_ellipse(), feather);
    for fy in 0..side {
        for fx in 0..side {
            let (px, py) = (x + fx, y + fy);
            let a = mask.get_pixel(px, py)[0] as u32;
            if a == 0 {
                continue;
            }
            let src = face.get_pixel(fx, fy);
            let dst = sheet.get_pixel_mut(px, py);
            for i in 0..3 {
                dst[i] = ((src[i] as u32 * a + dst[i] as u32 * (255 - a)) / 255) as u8;
            }
        }
    }
}

/// 补完表情的参考表另存为 `<原名>_表情.png`，原图不动
pub fn output_path(sheet: &Path) -> PathBuf {
    let stem = sheet.file_stem().and_then(|s| s.to_str()).unwrap_or("sheet");
    sheet.with_file_name(format!("{stem}_表情.png"))
}

/// 补表情的结果
pub struct Filled {
    pub path: PathBuf,
    /// 给人看的备注（哪几格没出来）
    pub notes: Vec<String>,
}

/// 给一张七分区参考表补表情。认不出表情区时回 `Ok(None)`（原图照用）
pub async fn fill(config_dir: &Path, sheet_path: &Path, name: &str, style_line: Option<&str>, identity_locks: &[String], workspace: Option<String>) -> Result<Option<Filled>> {
    let mut sheet = image::open(sheet_path).map_err(|e| Error::runtime(format!("读不到参考表：{e}")))?.to_rgb8();
    let Some(cells) = find_faces(&sheet) else { return Ok(None) };
    let (w, h) = sheet.dimensions();
    let tmp_dir = std::env::temp_dir().join("anima-sheet-expressions");
    std::fs::create_dir_all(&tmp_dir).map_err(|e| Error::runtime(e.to_string()))?;
    let original = sheet.clone();
    let mut notes = Vec::new();
    let mut done = 0;
    for (cell, (label, denoise, expression)) in cells.iter().zip(EXPRESSIONS) {
        let (x, y, side) = cell.crop(w, h);
        let crop = imageops::resize(&imageops::crop_imm(&original, x, y, side, side).to_image(), 1024, 1024, imageops::FilterType::Lanczos3);
        let crop_path = tmp_dir.join(format!("{}_{label}.png", uuid::Uuid::new_v4().simple()));
        crop.save(&crop_path).map_err(|e| Error::runtime(format!("存裁剪图失败：{e}")))?;
        let args: crate::studio::GenerateArgs = serde_json::from_value(serde_json::json!({
            "mode": "i2i", "engine": "zimage",
            "prompt": face_prompt(name, style_line, identity_locks, expression),
            "width": 1024, "height": 1024,
            "images": [crop_path.to_string_lossy()],
            "denoise": denoise, "loras": [], "auto_loras": false,
            "workspace_path": workspace,
        }))
        .map_err(|e| Error::runtime(e.to_string()))?;
        let out = Box::pin(crate::studio::generate_with(config_dir, args)).await;
        let _ = std::fs::remove_file(&crop_path);
        match out {
            Ok(o) if o.ok && o.path.is_some() => {
                let p = o.path.unwrap_or_default();
                match image::open(&p) {
                    Ok(face) => {
                        paste_face(&mut sheet, &face.to_rgb8(), cell);
                        done += 1;
                        // 中间产物（单独一张脸）不留在生成目录里占地方
                        let _ = std::fs::remove_file(&p);
                    }
                    Err(e) => notes.push(format!("表情「{label}」读不回来：{e}")),
                }
            }
            Ok(o) => notes.push(format!("表情「{label}」没出来：{}", o.message.lines().next().unwrap_or(""))),
            Err(e) => notes.push(format!("表情「{label}」没出来：{e}")),
        }
    }
    if done == 0 {
        return Ok(Some(Filled { path: sheet_path.to_path_buf(), notes }));
    }
    let path = output_path(sheet_path);
    sheet.save(&path).map_err(|e| Error::runtime(format!("存补完表情的参考表失败：{e}")))?;
    Ok(Some(Filled { path, notes }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 画一张假的参考表：米白底，表情区三个「头」（深色头发 + 肤色脸 + 挨在一起的肩膀）
    fn fake_sheet(offset_y: i32, touching_shoulders: bool) -> RgbImage {
        let mut img = RgbImage::from_pixel(1536, 1024, Rgb([244, 238, 230]));
        for (i, cx) in [560i32, 730, 895].iter().enumerate() {
            let top = 730 + offset_y + i as i32 * 3;
            for y in top..960 {
                // 头发宽 ±70（相邻两个头之间留 20 多像素，和真表一样），肩膀挨上时宽 ±85
                for x in cx - 85..cx + 85 {
                    if y < top + 175 && (x - cx).abs() >= 70 {
                        continue;
                    }
                    let hair = y < top + 120;
                    let face = (x - cx).abs() < 50 && y > top + 60 && y < top + 170;
                    let shoulders = y > top + 175 && (touching_shoulders || (x - cx).abs() < 75);
                    let c = if face { Rgb([250, 215, 190]) } else if hair { Rgb([30, 28, 30]) } else if shoulders { Rgb([225, 230, 240]) } else { continue };
                    img.put_pixel(x as u32, y as u32, c);
                }
            }
        }
        // 右边细节区的分隔线
        for y in 690..990 {
            img.put_pixel(1000, y, Rgb([60, 60, 60]));
        }
        img
    }

    #[test]
    fn 表情区三个头按位置找出来_上下偏移也认得() {
        for dy in [0, 25] {
            let cells = find_faces(&fake_sheet(dy, false)).expect("应该认出三个头");
            let cx: Vec<u32> = cells.iter().map(|c| c.cx()).collect();
            for (got, want) in cx.iter().zip([560u32, 730, 895]) {
                assert!(got.abs_diff(want) <= 4, "{cx:?}");
            }
            assert!(cells[0].head.1.abs_diff((730 + dy) as u32) <= 2, "{:?}", cells[0]);
            // 裁剪框包住脸、遮罩在头里面
            let (x, y, side) = cells[1].crop(1536, 1024);
            assert!(x < 680 && x + side > 780 && y < 800 && y + side > 900, "{x} {y} {side}");
            let (ex, ey, rx, ry) = cells[1].face_ellipse();
            assert!((ex - 730.0).abs() < 4.0 && rx > 40.0 && ry > 50.0 && ey > (cells[1].head.1 as f32));
        }
    }

    #[test]
    fn 肩膀挨在一起也分得开_认不出三个就不动() {
        assert!(find_faces(&fake_sheet(0, true)).is_some(), "上半截头发是分开的");
        assert!(find_faces(&RgbImage::from_pixel(1536, 1024, Rgb([244, 238, 230]))).is_none());
        let mut two = fake_sheet(0, false);
        for y in 700..1000 {
            for x in 800..980 {
                two.put_pixel(x, y, Rgb([244, 238, 230]));
            }
        }
        assert!(find_faces(&two).is_none(), "只有两个头时不能硬凑");
    }

    #[test]
    fn 贴回只改脸部椭圆_外面一个像素都不动() {
        let mut sheet = fake_sheet(0, false);
        let before = sheet.clone();
        let cells = find_faces(&sheet).unwrap();
        let red = RgbImage::from_pixel(1024, 1024, Rgb([255, 0, 0]));
        paste_face(&mut sheet, &red, &cells[0]);
        let (cx, cy, _, _) = cells[0].face_ellipse();
        assert_eq!(sheet.get_pixel(cx as u32, cy as u32), &Rgb([255, 0, 0]), "脸中间换掉了");
        assert_eq!(sheet.get_pixel(cx as u32, cells[0].head.1 + 2), before.get_pixel(cx as u32, cells[0].head.1 + 2), "头顶头发不动");
        assert_eq!(sheet.get_pixel(730, 850), before.get_pixel(730, 850), "隔壁那格不动");
    }

    #[test]
    fn 提示词带风格身份和表情_没给风格时说照原图() {
        let p = face_prompt("小雨", Some("anime cel shading"), &["black bob hair".into()], EXPRESSIONS[1].2);
        assert!(p.starts_with("anime cel shading，小雨的头像特写。black bob hair。小雨非常生气"), "{p}");
        assert!(face_prompt("", None, &[], "x").contains("和原图同样的画风"));
        assert_eq!(EXPRESSIONS.map(|e| e.1), [0.55, 0.7, 0.7]);
        assert_eq!(output_path(Path::new("/g/sheet_01.png")), PathBuf::from("/g/sheet_01_表情.png"));
    }

    /// 真跑一遍补表情（要本机 ComfyUI，约 5 分钟）：`ANIMA_SHEET=<参考表路径> cargo test --lib 真跑补表情 -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn 真跑补表情() {
        let src = PathBuf::from(std::env::var("ANIMA_SHEET").expect("ANIMA_SHEET"));
        let dir = std::env::temp_dir().join("anima-sheet-expr-live");
        std::fs::create_dir_all(&dir).unwrap();
        let sheet = dir.join("sheet.png");
        std::fs::copy(&src, &sheet).unwrap();
        let cfg = dirs::home_dir().unwrap().join("Library/Application Support/com.anima.desktop");
        let t0 = std::time::Instant::now();
        let out = fill(&cfg, &sheet, std::env::var("ANIMA_SHEET_NAME").as_deref().unwrap_or(""), Some("日系动画风格，赛璐璐上色，干净线条"), &[], Some(dir.to_string_lossy().into())).await.unwrap();
        let f = out.expect("应该认出表情区");
        println!("{} 秒 → {} {:?}", t0.elapsed().as_secs(), f.path.display(), f.notes);
    }

    /// 本机那几张真参考表：认出的头画框存下来看（`-- --ignored --nocapture`）
    #[test]
    #[ignore]
    fn 真参考表的表情区() {
        let dir = dirs::home_dir().unwrap().join("AI/ComfyUI/output/CS_test");
        for f in ["dyn_sheet_00001_.png", "dyn_sheet_rb4_00001_.png", "dyn_daju_00001_.png", "e2e_sheet_00001_.png"] {
            let img = image::open(dir.join(f)).unwrap().to_rgb8();
            let cells = find_faces(&img);
            println!("{f}: {cells:?}");
            if let Some(cells) = cells {
                let mut dbg = img.clone();
                for c in &cells {
                    let (x, y, s) = c.crop(img.width(), img.height());
                    for i in 0..s {
                        for (px, py) in [(x + i, y), (x + i, y + s - 1), (x, y + i), (x + s - 1, y + i)] {
                            dbg.put_pixel(px, py, Rgb([255, 0, 0]));
                        }
                    }
                    let m = ellipse_mask(img.width(), img.height(), c.face_ellipse(), 1.0);
                    for (px, py, v) in m.enumerate_pixels() {
                        if v[0] > 0 && v[0] < 200 {
                            dbg.put_pixel(px, py, Rgb([0, 160, 255]));
                        }
                    }
                }
                dbg.save(std::env::temp_dir().join(format!("faces_{f}"))).unwrap();
            }
        }
    }
}
