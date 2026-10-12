//! 画布图片节点的两个小工具：抠图、超分。
//!
//! 都是本机 ComfyUI 上几秒钟的事，2026-09-14 实跑看过：
//!
//! | 工具 | 模型 | 实测 |
//! |---|---|---|
//! | 抠图 | BiRefNet（`background_removal/birefnet.safetensors`） | 3 s，全身古装人像边缘干净 |
//! | 超分 ×2 | RealESRGAN x2plus | 2 s，575×832 → 1150×1664，睫毛发丝更锐 |
//! | 视频补帧 ×2 | RIFE 4.7 | 6 s，121 帧 24fps → 241 帧 48fps，快速动作的中间帧没有重影，音轨保留 |
//! | **视频超分 ×2（默认）** | RealESRGAN 逐帧 | **125 s**，12 秒 896×512 → 1792×1024；脸、横摇时的背景人物都锐利、没有重影；前后帧变化和原片一样，不闪 |
//! | 视频精修超分 ×2 | LTX 2.5 Pixel-Spatial IC-LoRA（生成式重绘） | **51 min、峰值 98 GB**；静止画面纹理最细，**镜头运动时重绘出重影** —— 只给个别静止特写用 |
//!
//! 同题四种超分对照（2026-09-14，第 10 镜）：LTX「只做潜空间放大」85 s 但发软、横摇时糊成一片，排除。
//!
//! # 抠图写错不报错的那个地方
//!
//! `RemoveBackground` 出的遮罩是「人 = 1」，而 `JoinImageWithAlpha` 会把遮罩**反过来**当透明度用。
//! 直接连的话出来的是**人被挖空、背景留着**，任务成功、没有任何报错 —— 实跑第一次就是这样。
//! 中间必须加一个 `InvertMask`。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;

use crate::error::{Error, Result};

pub const BG_MODEL: &str = "birefnet.safetensors";
pub const UPSCALE_MODEL: &str = "RealESRGAN_x2plus.pth";

pub fn build_op_graph(op: &str, image_name: &str, prefix: &str) -> Option<Value> {
    match op {
        "remove_bg" => Some(json!({
            "1": {"class_type": "LoadImage", "inputs": {"image": image_name}},
            "2": {"class_type": "LoadBackgroundRemovalModel", "inputs": {"bg_removal_name": BG_MODEL}},
            "3": {"class_type": "RemoveBackground", "inputs": {"bg_removal_model": ["2", 0], "image": ["1", 0]}},
            // 见模块注释：不反转的话人被挖空
            "4": {"class_type": "InvertMask", "inputs": {"mask": ["3", 0]}},
            "5": {"class_type": "JoinImageWithAlpha", "inputs": {"image": ["1", 0], "alpha": ["4", 0]}},
            "6": {"class_type": "SaveImage", "inputs": {"images": ["5", 0], "filename_prefix": prefix}}
        })),
        "upscale" => Some(json!({
            "1": {"class_type": "LoadImage", "inputs": {"image": image_name}},
            "2": {"class_type": "UpscaleModelLoader", "inputs": {"model_name": UPSCALE_MODEL}},
            "3": {"class_type": "ImageUpscaleWithModel", "inputs": {"upscale_model": ["2", 0], "image": ["1", 0]}},
            "6": {"class_type": "SaveImage", "inputs": {"images": ["3", 0], "filename_prefix": prefix}}
        })),
        _ => None,
    }
}

/// 视频补帧 ×2。`fps` 是源视频帧率，输出按两倍写 —— 写死 48 的话 30fps 素材会变慢。
pub fn build_interpolate_graph(video_name: &str, fps: f64, prefix: &str) -> Value {
    json!({
        "1": {"class_type": "LoadVideo", "inputs": {"file": video_name}},
        "2": {"class_type": "GetVideoComponents", "inputs": {"video": ["1", 0]}},
        "3": {"class_type": "RIFE VFI", "inputs": {
            "ckpt_name": "rife47.pth", "frames": ["2", 0], "clear_cache_after_n_frames": 10,
            "multiplier": 2, "fast_mode": true, "ensemble": true, "scale_factor": 1.0,
            "dtype": "float32", "torch_compile": false, "batch_size": 1
        }},
        // 音轨原样接回去，补帧不改时长
        "4": {"class_type": "VHS_VideoCombine", "inputs": {
            "images": ["3", 0], "audio": ["2", 1], "frame_rate": fps * 2.0, "loop_count": 0,
            "filename_prefix": prefix, "format": "video/h264-mp4", "pingpong": false, "save_output": true
        }}
    })
}

/// 视频超分 ×2。`refine = false` 走 RealESRGAN（默认），`true` 走 LTX 生成式重绘（慢、运动画面有重影）。
pub fn build_video_upscale_graph(video_name: &str, fps: f64, refine: bool, prompt: &str, prefix: &str) -> Value {
    if !refine {
        return json!({
            "1": {"class_type": "LoadVideo", "inputs": {"file": video_name}},
            "2": {"class_type": "GetVideoComponents", "inputs": {"video": ["1", 0]}},
            "3": {"class_type": "UpscaleModelLoader", "inputs": {"model_name": UPSCALE_MODEL}},
            "4": {"class_type": "ImageUpscaleWithModel", "inputs": {"upscale_model": ["3", 0], "image": ["2", 0]}},
            "5": {"class_type": "VHS_VideoCombine", "inputs": {
                "images": ["4", 0], "audio": ["2", 1], "frame_rate": fps, "loop_count": 0,
                "filename_prefix": prefix, "format": "video/h264-mp4", "pingpong": false, "save_output": true
            }}
        });
    }
    let prompt = format!(
        "Describe the source movie accurately, preserving its people, setting, camera movement, action, lighting, and visual style while recovering clean fine detail. Keep motion and composition faithful to the source. {}",
        prompt.trim()
    );
    json!({
        "1": {"class_type": "LoadVideo", "inputs": {"file": video_name}},
        "2": {"class_type": "GetVideoComponents", "inputs": {"video": ["1", 0]}},
        "3": {"class_type": "WeeToddLTX25ComponentLoader", "inputs": {
            "transformer": crate::agent_runtime::weetodd::TRANSFORMER_Q8,
            "text_encoder": crate::agent_runtime::weetodd::TEXT_ENCODER_Q8,
            "video_vae": "ltx-2.5-video-vae-conv-bf16.safetensors",
            "audio_vae": "ltx-2.5-audio-vae-bf16.safetensors",
            "spatial_upscaler": crate::agent_runtime::weetodd::SPATIAL_UPSCALER,
            "duration_head": ""
        }},
        "4": {"class_type": "WeeToddLTX25VideoUpscale", "inputs": {
            "model": ["3", 0], "images": ["2", 0], "fps": fps, "audio": ["2", 1],
            "mode": "pixel spatial IC-LoRA 2x (recommended)", "prompt": prompt, "seed": 584293325,
            "input_size_policy": "fit nearest 32px grid (preserve aspect)", "refinement_strength": 0.35,
            "source_frame_anchors": "first frame", "reference_strength": 0.7, "filename_prefix": prefix,
            "max_av_drift_seconds": 0.1, "low_ram_streaming": true, "sol_attention_profile": "disabled",
            "prompt_context": "official_1024", "generation_metadata": "{}",
            "pixel_spatial_lora": "LTX-2.5/ltx-2.5-22b-ic-lora-pixel-spatial-upscaler-x2-1.0.safetensors",
            "pixel_spatial_lora_strength": 1.0, "ffmpeg_path": "", "reuse_prompt_conditioning": true,
            "max_output_frame_megapixels": 0.0, "temporal_chunking": "disabled", "chunk_frame_megapixels": 260.0,
            "keep_chunks": false
        }}
    })
}

fn video_fps(path: &std::path::Path) -> Option<f64> {
    let out = std::process::Command::new(crate::agent_runtime::video_tool::ffprobe_binary())
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=r_frame_rate", "-of", "csv=p=0"])
        .arg(path)
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let (n, d) = s.trim().split_once('/')?;
    let fps = n.parse::<f64>().ok()? / d.parse::<f64>().ok()?;
    (fps.is_finite() && fps > 0.0).then_some(fps)
}

#[derive(Debug, Deserialize)]
pub struct VideoOpArgs {
    pub video_path: String,
}

#[derive(Debug, Deserialize)]
pub struct VideoUpscaleArgs {
    pub video_path: String,
    /// 生成式精修（LTX）。默认 false = RealESRGAN
    #[serde(default)]
    pub refine: bool,
    /// 精修时给 LTX 的画面描述（可空）
    #[serde(default)]
    pub prompt: String,
}

/// 视频超分 ×2。
pub async fn video_upscale_with(config_dir: &std::path::Path, args: VideoUpscaleArgs) -> Result<ImageOpResult> {
    let src = PathBuf::from(args.video_path.trim());
    if !src.is_file() {
        return Err(Error::config(format!("找不到视频：{}", src.display())));
    }
    video_fps(&src).ok_or_else(|| Error::config("读不出这段视频的帧率"))?;
    let (w, h) = crate::agent_runtime::video_tool::video_dimensions(&src)?;
    if w.max(h) >= 1800 {
        return Err(Error::config(format!("这段视频已经是 {w}×{h}，再放大一倍太大了")));
    }
    let id = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
    if args.refine {
        crate::llm_memory::free_for_video(config_dir);
    }
    let dir = config_dir.join("anima").join("canvas").join("ops");
    std::fs::create_dir_all(&dir).map_err(|e| Error::runtime(format!("建目录失败：{e}")))?;
    let out = dir.join(format!("upscale_{id}.mp4"));
    upscale_file(&src, &out, args.refine, &args.prompt, "Anima/canvas_upscale", "视频超分").await.map_err(Error::runtime)?;
    Ok(ImageOpResult { path: out.to_string_lossy().to_string() })
}

/// RealESRGAN ×2 放大一个视频文件到 `out`。组装「整片超分」逐镜调它。
pub async fn esrgan_upscale_file(src: &std::path::Path, out: &std::path::Path, prefix: &str, label: &str) -> std::result::Result<(), String> {
    upscale_file(src, out, false, "", prefix, label).await
}

async fn upscale_file(src: &std::path::Path, out: &std::path::Path, refine: bool, prompt: &str, prefix: &str, label: &str) -> std::result::Result<(), String> {
    let fps = video_fps(src).ok_or_else(|| "读不出这段视频的帧率".to_string())?;
    let id = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("mp4").to_ascii_lowercase();
    let name = format!("anima_upscale_{id}.{ext}");
    let mut copied = false;
    for dir in crate::agent_runtime::video_tool::find_comfyui_input_dirs() {
        copied |= std::fs::copy(src, dir.join(&name)).is_ok();
    }
    if !copied {
        return Err("视频复制进 ComfyUI 的 input 失败".into());
    }
    let client = reqwest::Client::builder().no_proxy().timeout(std::time::Duration::from_secs(60)).build().unwrap_or_default();
    let graph = build_video_upscale_graph(&name, fps, refine, prompt, prefix);
    let bytes = crate::agent_runtime::video_tool::submit_and_fetch_video(&client, &graph, label, Some(out)).await?;
    std::fs::write(out, &bytes).map_err(|e| format!("写视频失败：{e}"))
}

pub async fn interpolate_with(config_dir: &std::path::Path, args: VideoOpArgs) -> Result<ImageOpResult> {
    let src = PathBuf::from(args.video_path.trim());
    if !src.is_file() {
        return Err(Error::config(format!("找不到视频：{}", src.display())));
    }
    let fps = video_fps(&src).ok_or_else(|| Error::config("读不出这段视频的帧率"))?;
    if fps >= 50.0 {
        return Err(Error::config(format!("这段视频已经是 {fps:.0}fps，不用再补")));
    }
    let id = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("mp4").to_ascii_lowercase();
    let name = format!("anima_interp_{id}.{ext}");
    let mut copied = false;
    for dir in crate::agent_runtime::video_tool::find_comfyui_input_dirs() {
        copied |= std::fs::copy(&src, dir.join(&name)).is_ok();
    }
    if !copied {
        return Err(Error::config("视频复制进 ComfyUI 的 input 失败"));
    }
    let client = reqwest::Client::builder().no_proxy().timeout(std::time::Duration::from_secs(60)).build().unwrap_or_default();
    let graph = build_interpolate_graph(&name, fps, "Anima/canvas_interp");
    let dir = config_dir.join("anima").join("canvas").join("ops");
    let out = dir.join(format!("interp_{id}.mp4"));
    let bytes = crate::agent_runtime::video_tool::submit_and_fetch_video(&client, &graph, "画布 · 视频补帧", Some(&out))
        .await
        .map_err(Error::runtime)?;
    std::fs::create_dir_all(&dir).map_err(|e| Error::runtime(format!("建目录失败：{e}")))?;
    std::fs::write(&out, &bytes).map_err(|e| Error::runtime(format!("写视频失败：{e}")))?;
    Ok(ImageOpResult { path: out.to_string_lossy().to_string() })
}

#[derive(Debug, Deserialize)]
pub struct ImageOpArgs {
    pub image_path: String,
    /// `remove_bg` | `upscale`
    pub op: String,
}

#[derive(Debug, Serialize)]
pub struct ImageOpResult {
    pub path: String,
}

pub async fn image_op_with(config_dir: &std::path::Path, args: ImageOpArgs) -> Result<ImageOpResult> {
    let src = PathBuf::from(args.image_path.trim());
    if !src.is_file() {
        return Err(Error::config(format!("找不到图片：{}", src.display())));
    }
    let id = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("png").to_ascii_lowercase();
    let name = format!("anima_op_{id}.{ext}");
    let graph = build_op_graph(args.op.as_str(), &name, "Anima/canvas_op").ok_or_else(|| Error::config(format!("不认识的图片工具：{}", args.op)))?;
    let mut copied = false;
    for dir in crate::agent_runtime::video_tool::find_comfyui_input_dirs() {
        copied |= std::fs::copy(&src, dir.join(&name)).is_ok();
    }
    if !copied {
        return Err(Error::config("图片复制进 ComfyUI 的 input 失败"));
    }
    let client = reqwest::Client::builder().no_proxy().timeout(std::time::Duration::from_secs(60)).build().unwrap_or_default();
    // 名字叫 video，实际按 videos / gifs / images 三个键找产物，图片一样取得到
    let bytes = crate::agent_runtime::video_tool::submit_and_fetch_video(&client, &graph, "画布 · 图片工具", None)
        .await
        .map_err(Error::runtime)?;
    let dir = config_dir.join("anima").join("canvas").join("ops");
    std::fs::create_dir_all(&dir).map_err(|e| Error::runtime(format!("建目录失败：{e}")))?;
    let out = dir.join(format!("{}_{id}.png", args.op));
    std::fs::write(&out, &bytes).map_err(|e| Error::runtime(format!("写图片失败：{e}")))?;
    Ok(ImageOpResult { path: out.to_string_lossy().to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 实跑第一次就踩了：不反转遮罩，人被挖空、背景留着，任务照样成功。
    #[test]
    fn 抠图要先反转遮罩再当透明度() {
        let g = build_op_graph("remove_bg", "a.png", "p").unwrap();
        assert_eq!(g["4"]["class_type"], "InvertMask");
        assert_eq!(g["4"]["inputs"]["mask"], json!(["3", 0]));
        assert_eq!(g["5"]["inputs"]["alpha"], json!(["4", 0]), "透明度必须接反转后的遮罩");
        assert_eq!(g["2"]["inputs"]["bg_removal_name"], BG_MODEL);
    }

    #[test]
    fn 超分连线() {
        let g = build_op_graph("upscale", "a.png", "p").unwrap();
        assert_eq!(g["3"]["inputs"]["upscale_model"], json!(["2", 0]));
        assert_eq!(g["6"]["inputs"]["images"], json!(["3", 0]));
        assert!(build_op_graph("sharpen", "a.png", "p").is_none());
    }

    /// 输出帧率跟源走：写死 48 的话 30fps 素材补完会变慢。
    #[test]
    fn 补帧按源帧率翻倍_音轨接回去() {
        let g = build_interpolate_graph("a.mp4", 30.0, "p");
        assert_eq!(g["4"]["inputs"]["frame_rate"], 60.0);
        assert_eq!(g["3"]["inputs"]["multiplier"], 2);
        assert_eq!(g["4"]["inputs"]["audio"], json!(["2", 1]));
        assert_eq!(g["3"]["inputs"]["frames"], json!(["2", 0]));
    }

    #[test]
    fn 视频超分默认走_realesrgan_帧率和音轨跟源() {
        let g = build_video_upscale_graph("a.mp4", 24.0, false, "", "p");
        assert_eq!(g["3"]["inputs"]["model_name"], UPSCALE_MODEL);
        assert_eq!(g["4"]["inputs"]["image"], json!(["2", 0]));
        assert_eq!(g["5"]["inputs"]["frame_rate"], 24.0);
        assert_eq!(g["5"]["inputs"]["audio"], json!(["2", 1]));
        assert!(g.as_object().unwrap().values().all(|n| n["class_type"] != "WeeToddLTX25VideoUpscale"));
    }

    #[test]
    fn 精修超分走_ltx_像素空间模式() {
        let g = build_video_upscale_graph("a.mp4", 24.0, true, "宫殿里念诗", "p");
        assert_eq!(g["4"]["class_type"], "WeeToddLTX25VideoUpscale");
        assert_eq!(g["4"]["inputs"]["mode"], "pixel spatial IC-LoRA 2x (recommended)");
        assert!(g["4"]["inputs"]["prompt"].as_str().unwrap().ends_with("宫殿里念诗"));
        assert_eq!(g["4"]["inputs"]["audio"], json!(["2", 1]));
    }

    #[test]
    fn 图片产物取得到() {
        let outputs = json!({"6": {"images": [{"filename": "x.png", "subfolder": "Anima", "type": "output"}]}});
        assert!(crate::agent_runtime::weetodd::find_video_output(&outputs).is_some());
    }
}


/// 画布上的长镜头：**视频节点连视频节点 = 窗口链**，一次提交出一个 MP4。
///
/// 和分镜页那条（`cinema::cinema_render_long_take`）**底层是同一个
/// `build_h3_chain_graph`**，区别只在窗口从哪来：那边从镜头表，这边从画布上
/// 连起来的节点，每个窗口的提示词直接在节点里写。
///
/// 硬约束（等长、2.5~15 秒、至少两个）由前端的 `longTakeChain.planChain`
/// 和这里各判一次 —— 前端判是为了**在按钮上当场说清为什么不能点**，
/// 这里判是因为不能信前端。
#[derive(Debug, serde::Deserialize)]
pub struct LongTakeWindow {
    pub prompt: String,
    /// 这一窗口的参考图绝对路径
    #[serde(default)]
    pub images: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct CanvasLongTakeArgs {
    pub windows: Vec<LongTakeWindow>,
    pub window_secs: f64,
    #[serde(default)]
    pub context_frames: Option<u32>,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub seed: Option<u64>,
    #[serde(default = "crate::canvas_ops::default_true")]
    pub generate_audio: bool,
    /// 节点上选的 H3 加速档（`lightx2v` / `standard` / …，和工作台出片设置同一套 id）。
    /// 没给照旧 Standard。以前这里不收，画布长镜头一律走 Standard —— 节点上选了 LightX2V 也不生效
    #[serde(default)]
    pub h3_quality: Option<crate::cinema::RenderQuality>,
    /// 节点上选的 LoRA（`[{name, strength}]`，前端已按引擎核过架构、激活词已拼进各窗口提示词）。
    /// 以前不收 —— 画布上能选、激活词进了提示词，权重没挂
    #[serde(default)]
    pub loras: Vec<serde_json::Value>,
}

pub fn default_true() -> bool {
    true
}

#[derive(Debug, serde::Serialize)]
pub struct CanvasLongTakeOut {
    pub video_path: String,
    pub windows: usize,
    pub net_secs: f64,
}

pub async fn run_canvas_long_take(
    config_dir: &std::path::Path,
    a: CanvasLongTakeArgs,
) -> std::result::Result<CanvasLongTakeOut, String> {
    let frames = a.context_frames.unwrap_or(crate::long_take::CONTEXT_FRAMES);
    if a.windows.len() < 2 {
        return Err("长镜头至少要两个窗口 —— 把两个视频节点首尾连起来".into());
    }
    if a.window_secs < crate::long_take::MIN_WINDOW_SECS as f64
        || a.window_secs > crate::long_take::MAX_WINDOW_SECS as f64
    {
        return Err(format!(
            "窗口 {:.1} 秒超出 H3 的 {}~{} 秒范围",
            a.window_secs,
            crate::long_take::MIN_WINDOW_SECS,
            crate::long_take::MAX_WINDOW_SECS
        ));
    }
    if !crate::long_take::CONTEXT_FRAME_CHOICES.contains(&frames) {
        return Err(format!("重叠帧数只能是 {:?}", crate::long_take::CONTEXT_FRAME_CHOICES));
    }
    let windows: Vec<(String, Vec<std::path::PathBuf>)> = a
        .windows
        .iter()
        .map(|w| (w.prompt.clone(), w.images.iter().map(std::path::PathBuf::from).collect()))
        .collect();
    // 和其它画布产物同一个池子
    let dir = config_dir.join("anima").join("canvas").join("ops");
    std::fs::create_dir_all(&dir).map_err(|e| format!("建目录失败：{e}"))?;
    let id = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
    let out = dir.join(format!("longtake_{id}.mp4"));
    // 没给种子就按产物 id 派生 —— 同一条链重跑要能复现
    let seed = a.seed.unwrap_or_else(|| {
        id.bytes().fold(0u64, |h, b| h.wrapping_mul(31).wrapping_add(b as u64))
    });
    // 名字对到 ComfyUI 的枚举上；对不上的跳过（和单条出片同一个规矩），说明写进日志
    let requested = crate::agent_runtime::lora::parse_requested(&serde_json::json!({ "loras": a.loras }));
    let loras: Vec<(String, f64)> = if requested.is_empty() {
        Vec::new()
    } else {
        let client = reqwest::Client::builder().no_proxy().timeout(std::time::Duration::from_secs(20)).build().unwrap_or_default();
        let (resolved, warn) = crate::agent_runtime::lora::resolve_loras(&requested, &crate::agent_runtime::lora::available_loras(&client).await);
        if let Some(w) = warn {
            tracing::warn!("画布长镜头的 LoRA：{w}");
        }
        resolved.into_iter().map(|l| (l.name, l.strength)).collect()
    };
    // **重启续接**（`render_resume::resume_canvas`）：和单条出片、超长视频同一份续接单。
    // 2026-10-11 漏了这条 —— 星澜 30 秒（窗口表长镜头）跑到一半要重构，才发现它重启后接不回来
    crate::jobs::note_resume(serde_json::json!({
        "kind": "canvas_video", "out": out.to_string_lossy(), "generate_audio": a.generate_audio,
        "summary": format!("长镜头 {} 个窗口 × {:.1} 秒（应用重启后从 ComfyUI 收回）", a.windows.len(), a.window_secs),
    }));
    crate::agent_runtime::video_tool::render_long_take(
        &windows, &out, a.window_secs, frames, a.width, a.height, seed, a.generate_audio, a.h3_quality.map(|q| q.tier()), &loras,
    )
    .await?;
    Ok(CanvasLongTakeOut {
        video_path: out.to_string_lossy().to_string(),
        windows: a.windows.len(),
        // **和 `long_take::plan` 用同一份公式** —— 两边各算各的必然漂移，
        // 而这个数是用户拿来判断「窗口配够了没有」的。
        net_secs: crate::long_take::net_secs(a.window_secs as f32, a.windows.len(), frames) as f64,
    })
}
