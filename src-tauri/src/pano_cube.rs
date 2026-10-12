//! 将等距柱状 360 全景拆为六个 90° 立方体面，供多视角重建器使用。

use image::RgbImage;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct PanoCubeArgs { pub image_path: String, #[serde(default = "default_size")] pub face_size: u32 }
#[derive(Debug, Serialize)]
pub struct PanoCubeResult { pub path: String, pub faces: Vec<String>, pub input_path: String, pub log_path: String }
fn default_size() -> u32 { 1536 }
fn local_path(s: &str) -> PathBuf { PathBuf::from(s.strip_prefix("file://").unwrap_or(s).replace("%20", " ")) }

fn direction(face: usize, u: f64, v: f64) -> (f64, f64, f64) {
    match face { 0 => (1., -v, -u), 1 => (-1., -v, u), 2 => (u, 1., v), 3 => (u, -1., -v), 4 => (u, -v, 1.), _ => (-u, -v, -1.) }
}

fn sample_bilinear(img: &RgbImage, x: f64, y: f64) -> image::Rgb<u8> {
    let (w, h) = img.dimensions();
    let x0 = x.floor().rem_euclid(w as f64) as u32;
    let x1 = (x0 + 1) % w;
    let y0 = y.floor().clamp(0., (h - 1) as f64) as u32;
    let y1 = (y0 + 1).min(h - 1);
    let tx = x - x.floor();
    let ty = y - y.floor();
    let mut out = [0u8; 3];
    for (c, value) in out.iter_mut().enumerate() {
        let a = img.get_pixel(x0, y0)[c] as f64 * (1. - tx) + img.get_pixel(x1, y0)[c] as f64 * tx;
        let b = img.get_pixel(x0, y1)[c] as f64 * (1. - tx) + img.get_pixel(x1, y1)[c] as f64 * tx;
        *value = (a * (1. - ty) + b * ty).round().clamp(0., 255.) as u8;
    }
    image::Rgb(out)
}

/// 从等距柱状全景投影一张普通透视图，供只接受单视图的 SHARP 使用。
pub fn project_perspective(input: &Path, output: &Path, width: u32, height: u32, yaw_deg: f64, pitch_deg: f64, fov_deg: f64) -> Result<(), String> {
    let img = image::open(input).map_err(|e| format!("读取全景图失败：{e}"))?.to_rgb8();
    let (sw, sh) = img.dimensions();
    if sw < sh.saturating_mul(3) / 2 { return Err(format!("图片不像 2:1 等距柱状全景：{sw}×{sh}")); }
    let mut out = RgbImage::new(width, height);
    let yaw = yaw_deg.to_radians();
    let pitch = pitch_deg.to_radians();
    let tan = (fov_deg.to_radians() / 2.).tan();
    let aspect = width as f64 / height as f64;
    for y in 0..height { for x in 0..width {
        let rx = (2. * (x as f64 + 0.5) / width as f64 - 1.) * tan;
        let ry = (1. - 2. * (y as f64 + 0.5) / height as f64) * tan / aspect;
        let rz = 1.;
        let py = ry * pitch.cos() - rz * pitch.sin();
        let pz = ry * pitch.sin() + rz * pitch.cos();
        let dx = rx * yaw.cos() + pz * yaw.sin();
        let dz = -rx * yaw.sin() + pz * yaw.cos();
        let len = (dx*dx + py*py + dz*dz).sqrt();
        let lon = dx.atan2(dz);
        let lat = (py / len).asin();
        let sx = (lon / (2. * std::f64::consts::PI) + 0.5) * sw as f64 - 0.5;
        let sy = (0.5 - lat / std::f64::consts::PI) * sh as f64 - 0.5;
        out.put_pixel(x, y, sample_bilinear(&img, sx, sy));
    }}
    out.save(output).map_err(|e| format!("保存透视图失败：{e}"))
}

pub async fn split(config_dir: &Path, args: PanoCubeArgs) -> Result<PanoCubeResult, String> {
    let input = local_path(args.image_path.trim());
    if !input.is_file() { return Err(format!("全景图不存在：{}", input.display())); }
    let size = args.face_size.clamp(512, 4096);
    let img = image::open(&input).map_err(|e| format!("读取全景图失败：{e}"))?.to_rgb8();
    let (w, h) = img.dimensions();
    if w < h.saturating_mul(3) / 2 { return Err(format!("图片不像 2:1 等距柱状全景：{w}×{h}")); }
    let dir = config_dir.join("canvas/worlds").join(format!("cubemap-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let names = ["right", "left", "up", "down", "front", "back"];
    let mut faces = Vec::new();
    for (face, name) in names.iter().enumerate() {
        let mut out = RgbImage::new(size, size);
        for y in 0..size { for x in 0..size {
            let u = 2. * (x as f64 + 0.5) / size as f64 - 1.;
            let v = 2. * (y as f64 + 0.5) / size as f64 - 1.;
            let (dx, dy, dz) = direction(face, u, v);
            let len = (dx*dx + dy*dy + dz*dz).sqrt();
            let lon = dx.atan2(dz);
            let lat = (dy / len).asin();
            let sx = (lon / (2. * std::f64::consts::PI) + 0.5) * w as f64 - 0.5;
            let sy = (0.5 - lat / std::f64::consts::PI) * h as f64 - 0.5;
            out.put_pixel(x, y, sample_bilinear(&img, sx, sy));
        }}
        let p = dir.join(format!("{name}.png"));
        out.save(&p).map_err(|e| format!("保存{name}失败：{e}"))?;
        faces.push(p.to_string_lossy().into_owned());
    }
    let manifest = dir.join("cubemap.json");
    std::fs::write(&manifest, serde_json::to_vec_pretty(&serde_json::json!({"source":input,"projection":"cubemap","face_size":size,"faces":faces})).unwrap()).map_err(|e| e.to_string())?;
    let log = dir.join("cubemap.generation.md");
    std::fs::write(&log, format!("# Anima 全景六面预处理日志\n\n- 状态：成功\n- 模式：等距柱状 360 全景 → Cubemap 六面\n- 输入文件：{}\n- 输出清单：{}\n- 单面尺寸：{}×{}\n- 面顺序：right, left, up, down, front, back\n\n## 提示词 / 生成依据\n\n纯几何投影，不使用文本提示词；六面均直接派生自输入全景图。\n\n## 关联关系\n\n`cubemap.json` 保存输入图、投影类型、面尺寸和全部输出路径。六面尚未融合为同一 3DGS。\n", input.display(), manifest.display(), size, size)).map_err(|e| e.to_string())?;
    Ok(PanoCubeResult { path: manifest.to_string_lossy().into_owned(), faces, input_path: input.to_string_lossy().into_owned(), log_path: log.to_string_lossy().into_owned() })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cube_directions_are_distinct() { assert_ne!(direction(0,0.,0.), direction(1,0.,0.)); }

    /// 全景每个像素的颜色就是它自己的坐标（R = 列，G = 行），投影出来的透视图就能直接读出「看的是哪」。
    /// 正前方 = 全景中心；左边缘 = 经度 −半视场；上边缘 = 纬度 +竖直半视场。
    #[test]
    fn perspective_looks_where_it_should() {
        let dir = tempfile::tempdir().unwrap();
        let pano = dir.path().join("pano.png");
        let img = RgbImage::from_fn(256, 128, |x, y| image::Rgb([x as u8, (y * 2) as u8, 0]));
        img.save(&pano).unwrap();
        let out = dir.path().join("flat.png");
        let (w, h, fov) = (640u32, 360u32, 64.0f64);
        project_perspective(&pano, &out, w, h, 0., 0., fov).unwrap();
        let flat = image::open(&out).unwrap().to_rgb8();
        let col = |px: &image::Rgb<u8>| px[0] as f64;
        let row = |px: &image::Rgb<u8>| px[1] as f64 / 2.0;
        let center = flat.get_pixel(w / 2, h / 2);
        assert!((col(center) - 128.0).abs() < 1.5 && (row(center) - 64.0).abs() < 1.5, "正前方不是全景中心：{center:?}");
        let left = flat.get_pixel(0, h / 2);
        let expect_left = (-(fov / 2.0) / 360.0 + 0.5) * 256.0;
        assert!((col(left) - expect_left).abs() < 1.5, "左边缘 {} ≠ {expect_left}", col(left));
        let top = flat.get_pixel(w / 2, 0);
        let v_half = ((fov.to_radians() / 2.0).tan() * h as f64 / w as f64).atan().to_degrees();
        let expect_top = (0.5 - v_half / 180.0) * 128.0;
        assert!((row(top) - expect_top).abs() < 1.5, "上边缘 {} ≠ {expect_top}（上下是不是反了）", row(top));
    }
}
