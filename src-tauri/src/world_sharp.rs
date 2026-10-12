//! SHARP 单图场景重建：把一张场景图变成导演世界可直接载入的 3DGS `.ply`。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Deserialize)]
pub struct WorldGenerateArgs {
    pub image_path: String,
    #[serde(default = "default_device")]
    pub device: String,
}

#[derive(Debug, Serialize)]
pub struct WorldGenerateResult {
    pub path: String,
    pub format: String,
    pub input_path: String,
    pub prepared_input_path: String,
    pub model_path: String,
    pub log_path: String,
}

fn default_device() -> String { "mps".into() }

/// 全景投影给 SHARP 的单视图尺寸。
const SHARP_INPUT_W: u32 = 1280;
const SHARP_INPUT_H: u32 = 720;

/// **SHARP 读不到 EXIF 焦距时按 30mm（35mm 等效，按对角线换算）算**
/// （`ml-sharp/src/sharp/utils/io.py`：`f_px = 30 · √(w²+h²) / √(36²+24²)`）。
/// 1280×720 下这等于水平视场 ≈ 64.3°。
///
/// 投影出来的透视图**必须就是这个视场**：投成 90°（上一版）的话 SHARP 以为镜头长了 1.6 倍，
/// 重建出的深度和墙面角度全错，相机一动墙就弯 —— 而且不报错。
fn sharp_default_hfov_deg(w: u32, h: u32) -> f64 {
    let f_px = 30.0 * ((w as f64).powi(2) + (h as f64).powi(2)).sqrt() / (36.0f64.powi(2) + 24.0f64.powi(2)).sqrt();
    2.0 * ((w as f64 / 2.0) / f_px).atan().to_degrees()
}

fn local_path(raw: &str) -> PathBuf {
    let value = raw.strip_prefix("file://").unwrap_or(raw);
    PathBuf::from(value.replace("%20", " "))
}

fn find_workspace(config_dir: &Path) -> Result<PathBuf, String> {
    if let Ok(path) = std::env::var("ANIMA_WORKSPACE") {
        let p = PathBuf::from(path);
        if p.join("tools/world/ml-sharp").is_dir() { return Ok(p); }
    }
    let mut candidates = crate::app_root::roots(Some(config_dir));
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf());
    candidates.into_iter().find(|p| p.join("tools/world/ml-sharp").is_dir())
        .ok_or_else(|| "找不到 Anima 内置 SHARP 运行器（tools/world/ml-sharp）".into())
}

fn epoch_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis()
}

fn write_log(path: &Path, status: &str, input: &Path, model: &Path, command: &str, output: &Path, detail: &str) {
    let body = format!("# Anima 3DGS 生成日志\n\n- 状态：{status}\n- 时间（Unix ms）：{}\n- 模式：SHARP 单图参考生成 3D Gaussian Splatting\n- 输入文件：{}\n- 模型文件：{}\n- 输出文件：{}\n- 输出格式：PLY（3DGS）\n- 设备：MPS\n- 运行命令：`{command}`\n\n## 提示词 / 生成依据\n\n此模式不使用文本提示词；完整视觉依据为上述输入场景图。输入图与输出 3DGS 为一对一衍生关系。\n\n## 运行信息\n\n{detail}\n", epoch_ms(), input.display(), model.display(), output.display());
    let _ = std::fs::write(path, body);
}

pub async fn generate(config_dir: &Path, args: WorldGenerateArgs) -> Result<WorldGenerateResult, String> {
    let input = local_path(args.image_path.trim());
    if !input.is_file() { return Err(format!("场景图不存在：{}", input.display())); }
    let ext = input.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
    if !matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "heic" | "heif") {
        return Err("SHARP 输入必须是 PNG、JPEG、WebP 或 HEIF 场景图".into());
    }
    let workspace = find_workspace(config_dir)?;
    let cli = workspace.join("tools/world/ml-sharp/.venv/bin/sharp");
    let model = std::env::var("ANIMA_SHARP_MODEL").ok().map(PathBuf::from).or_else(|| {
        workspace.ancestors().map(|p| p.join("ComfyUI/models/world/sharp/sharp_2572gikvuh.pt")).find(|p| p.is_file())
    }).unwrap_or_else(|| workspace.join("../ComfyUI/models/world/sharp/sharp_2572gikvuh.pt"));
    if !cli.is_file() { return Err(format!("SHARP 运行器不存在：{}", cli.display())); }
    if !model.is_file() { return Err(format!("SHARP 权重不存在：{}", model.display())); }

    let output_dir = config_dir.join("canvas/worlds").join(format!("sharp-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&output_dir).map_err(|e| format!("创建3D片场目录失败：{e}"))?;
    let (iw, ih) = image::image_dimensions(&input).map_err(|e| format!("读取场景图尺寸失败：{e}"))?;
    let prepared_input = if iw >= ih.saturating_mul(3) / 2 {
        let perspective = output_dir.join("sharp-input-perspective.png");
        crate::pano_cube::project_perspective(&input, &perspective, SHARP_INPUT_W, SHARP_INPUT_H, 0., 0., sharp_default_hfov_deg(SHARP_INPUT_W, SHARP_INPUT_H))?;
        perspective
    } else { input.clone() };
    let stem = prepared_input.file_stem().and_then(|x| x.to_str()).unwrap_or("scene");
    let output = output_dir.join(format!("{stem}.ply"));
    let log = output.with_extension("ply.generation.md");
    let device = if matches!(args.device.as_str(), "cpu" | "mps" | "cuda") { args.device } else { "mps".into() };
    let command_text = format!("{} predict -i {} -o {} -c {} --device {}", cli.display(), prepared_input.display(), output_dir.display(), model.display(), device);
    let prep = if prepared_input != input {
        format!("检测到全景图；已先投影为水平 {:.1}°、{SHARP_INPUT_W}×{SHARP_INPUT_H} 单视图（和 SHARP 无 EXIF 时假设的 30mm 焦距一致）：{}\n", sharp_default_hfov_deg(SHARP_INPUT_W, SHARP_INPUT_H), prepared_input.display())
    } else {
        "输入为普通透视图，无需预处理。\n".into()
    };
    write_log(&log, "已提交", &input, &model, &command_text, &output, &format!("后台任务已登记，等待 SHARP 推理。\n{prep}"));

    let result = tokio::process::Command::new(&cli)
        .args(["predict", "-i"]).arg(&prepared_input)
        .args(["-o"]).arg(&output_dir)
        .args(["-c"]).arg(&model)
        .args(["--device", &device])
        .stdout(Stdio::piped()).stderr(Stdio::piped()).output().await;
    match result {
        Ok(done) if done.status.success() && output.is_file() => {
            let detail = format!("退出码：{}\n\n### stdout\n```text\n{}\n```\n\n### stderr\n```text\n{}\n```", done.status, String::from_utf8_lossy(&done.stdout), String::from_utf8_lossy(&done.stderr));
            write_log(&log, "成功", &input, &model, &command_text, &output, &detail);
            Ok(WorldGenerateResult { path: output.to_string_lossy().into_owned(), format: "ply".into(), input_path: input.to_string_lossy().into_owned(), prepared_input_path: prepared_input.to_string_lossy().into_owned(), model_path: model.to_string_lossy().into_owned(), log_path: log.to_string_lossy().into_owned() })
        }
        Ok(done) => {
            let detail = format!("退出码：{}\nstdout：\n{}\nstderr：\n{}", done.status, String::from_utf8_lossy(&done.stdout), String::from_utf8_lossy(&done.stderr));
            write_log(&log, "失败", &input, &model, &command_text, &output, &detail);
            Err(format!("SHARP 生成失败：{}", String::from_utf8_lossy(&done.stderr).trim()))
        }
        Err(e) => {
            write_log(&log, "失败", &input, &model, &command_text, &output, &e.to_string());
            Err(format!("无法启动 SHARP：{e}"))
        }
    }
}

/// 3DGS 片场文件**按二进制原样**回给前端，编辑器转成 `blob:` 地址给 splat 加载器。
///
/// # 为什么不走 `assets_read_media`
///
/// 那条是 base64 data URL，上限 24 MB（`assets.rs` 的 `MAX_INLINE`）。SHARP 每个像素一个高斯，
/// 1280×720 的输入就是约 52 MB 的 `.ply`（09-16 冒烟 1920×1080 是 63 MB）——
/// 生成完的片场在打包版里**一个都载不进导演世界**，报「文件太大，走不了兜底通道」。
/// `anima-media://` 协议也不行：响应不带 CORS 头，加载器 `fetch` 会被拦。
///
/// 路径规则和媒体读取同一份（`media_roots` + `resolve_media_path`），只认 splat 的三种扩展名。
#[tauri::command]
pub fn world_read_splat(state: tauri::State<'_, crate::AppState>, path: String) -> crate::error::Result<tauri::ipc::Response> {
    let roots = crate::assets::media_roots(&state.config_dir, &crate::media_extra_roots());
    let real = crate::assets::resolve_media_path(&roots, &local_path(path.trim()))?;
    read_splat_bytes(&real).map(tauri::ipc::Response::new).map_err(crate::error::Error::config)
}

/// 上限 1 GB：再大 WebKit 也放不下，读进来只会把应用拖死。
const MAX_SPLAT_BYTES: u64 = 1024 * 1024 * 1024;

fn read_splat_bytes(real: &Path) -> Result<Vec<u8>, String> {
    let ext = real.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
    if !matches!(ext.as_str(), "ply" | "ksplat" | "spz") {
        return Err("只读 .ply / .ksplat / .spz 片场文件".into());
    }
    let len = std::fs::metadata(real).map_err(|e| format!("读不到片场文件：{e}"))?.len();
    if len > MAX_SPLAT_BYTES {
        return Err(format!("片场文件 {} MB，太大了，先转成 .ksplat / .spz", len / 1024 / 1024));
    }
    std::fs::read(real).map_err(|e| format!("读不到片场文件：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_url_is_converted_to_local_path() {
        assert_eq!(local_path("file:///tmp/a%20b.png"), PathBuf::from("/tmp/a b.png"));
    }

    #[test]
    fn pano_is_projected_at_the_fov_sharp_assumes() {
        let fov = sharp_default_hfov_deg(1280, 720);
        assert!((fov - 64.3).abs() < 0.1, "{fov}");
        // 反算回 SHARP 的焦距公式：同一个 f_px
        let f_from_fov = 640.0 / (fov.to_radians() / 2.0).tan();
        let f_sharp = 30.0 * (1280f64.powi(2) + 720f64.powi(2)).sqrt() / (36f64.powi(2) + 24f64.powi(2)).sqrt();
        assert!((f_from_fov - f_sharp).abs() < 0.01);
    }

    /// 超过 24 MB 的 `.ply` 也要读得出来（base64 那条通道读不了）；别的扩展名不读。
    #[test]
    fn splat_bytes_read_whole_and_only_splat_extensions() {
        let dir = tempfile::tempdir().unwrap();
        let big = dir.path().join("scene.ply");
        std::fs::write(&big, vec![7u8; 30 * 1024 * 1024]).unwrap();
        assert_eq!(read_splat_bytes(&big).unwrap().len(), 30 * 1024 * 1024);
        let other = dir.path().join("secret.txt");
        std::fs::write(&other, b"x").unwrap();
        assert!(read_splat_bytes(&other).is_err());
    }
}
