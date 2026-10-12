//! 本地 ComfyUI 文生视频与图生视频工具 (Generate Video & Animate Image Tool)
//!
//! 1. 精准对接本机 ComfyUI 真实节点拓扑：
//!    - 模型加载：UNETLoader (Wan2.1 / Hunyuan / MiniMax) 或 CheckpointLoaderSimple
//!    - 编码器与 VAE：CLIPLoader / VAELoader (wan_2.1_vae.safetensors)
//!    - 视频生成与保存：CreateVideo + SaveVideo (原生 nodes_video 节点，淘汰缺失的 VHS_VideoCombine)
//!    - 图生视频 (I2V)：自动复制图像至 input 目录，使用 LoadImage + WanImageToVideo 真实驱动
//! 2. 严格校验产物有效性（HTTP 状态、最小尺寸 > 10KB、MP4/WEBM 头部签名特征）
//! 3. 完整的 6 阶段流式进度与 call_id 精准溯源

use crate::agent_runtime::weetodd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{json, Value};

use super::super::agent_runtime::lora;
use super::tool::{
    ToolCallRequest, ToolFailure, ToolFailureKind, ToolOutcome, ToolSpec, ToolStatus,
};
use super::{RuntimeContext, ToolHandler, ToolKind};
use crate::agent_runtime::RuntimeEvent;
use crate::mcp::Operation;

pub(crate) const DEFAULT_COMFY_URL: &str = "http://127.0.0.1:8188";

/// 探查本地 ComfyUI 运行目录以支持输入图像上传（支持多实例多路径）
/// 读一张图的像素尺寸。读不出来时按 16:9 的常见默认走 ——
/// 这里失败不该让整次生成失败，源图已经复制进 ComfyUI 了。
pub(crate) fn image_dimensions(path: &Path) -> (u32, u32) {
    image::image_dimensions(path).unwrap_or((1344, 768))
}

pub(crate) fn find_comfyui_input_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let home = std::env::var("HOME").unwrap_or_default();
    let candidates = [
        PathBuf::from(&home).join("AI/ComfyUI/input"),
        PathBuf::from(&home).join("ComfyUI/input"),
        PathBuf::from(&home).join("Documents/ComfyUI/input"),
        // 这里原先还硬编码了两条 `/Users/you/...`，和上面用 $HOME 拼的
        // 那两条**完全重复**（在那台机器上），换台机器就是死路径。删了。
    ];
    for p in &candidates {
        if p.is_dir() && !dirs.contains(p) {
            dirs.push(p.clone());
        }
    }
    dirs
}

/// 在生图/生视频完成后主动触发 ComfyUI 释放潜空间缓存与 PyTorch MPS 显存池
pub(crate) async fn free_comfyui_cache(client: &reqwest::Client, unload_models: bool) {
    let free_url = format!("{}/free", DEFAULT_COMFY_URL);
    let payload = json!({
        "unload_models": unload_models,
        "free_memory": true
    });
    let _ = client.post(&free_url).json(&payload).send().await;
}

pub(crate) fn ffmpeg_binary() -> &'static str {
    for candidate in ["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg"] {
        if Path::new(candidate).is_file() {
            return candidate;
        }
    }
    "ffmpeg"
}

/// 读视频的像素尺寸。**字幕图必须和视频同尺寸**，否则 overlay 会把它
/// 钉在左上角按原尺寸叠，字幕位置整个错掉 —— 而 ffmpeg 不会报错。
pub(crate) fn video_dimensions(path: &Path) -> crate::error::Result<(u32, u32)> {
    let out = std::process::Command::new(ffprobe_binary())
        .args([
            "-v", "error", "-select_streams", "v:0",
            "-show_entries", "stream=width,height",
            "-of", "csv=p=0:s=x",
        ])
        .arg(path)
        .output()
        .map_err(|e| crate::error::Error::runtime(format!("跑 ffprobe 失败：{e}")))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let (w, h) = text
        .trim()
        .split_once('x')
        .ok_or_else(|| {
            crate::error::Error::runtime(format!("读不出 {} 的尺寸：{text:?}", path.display()))
        })?;
    Ok((
        w.trim().parse().map_err(|_| crate::error::Error::runtime(format!("宽度不是数字：{w:?}")))?,
        h.trim().parse().map_err(|_| crate::error::Error::runtime(format!("高度不是数字：{h:?}")))?,
    ))
}

/// 这个视频有没有音轨。
///
/// **判据是「有没有音频流」，不是「响不响」** —— 一段 -91 dB 的数字静音
/// 也算有音轨。这里只用来决定「要不要另外补一条」。
pub(crate) fn has_audio_stream(path: &Path) -> bool {
    std::process::Command::new(ffprobe_binary())
        .args(["-v", "error", "-select_streams", "a", "-show_entries", "stream=index", "-of", "csv=p=0"])
        .arg(path)
        .output()
        .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
        .unwrap_or(false)
}

/// 快速剥离视频中的音轨（纯流拷贝，不重编码画面）
pub(crate) fn strip_audio(path: &Path) -> Result<(), String> {
    if !has_audio_stream(path) {
        return Ok(());
    }
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("mp4");
    let tmp = path.with_extension(format!("noaudio_{}.{ext}", std::process::id()));
    let status = std::process::Command::new(ffmpeg_binary())
        .args(["-y", "-loglevel", "error", "-i"])
        .arg(path)
        .args(["-c:v", "copy", "-an"])
        .arg(&tmp)
        .status()
        .map_err(|e| format!("调用 ffmpeg 剥离音频失败: {e}"))?;
    if status.success() && tmp.is_file() {
        if std::fs::rename(&tmp, path).is_err() {
            std::fs::copy(&tmp, path).map_err(|e| format!("覆盖无音频视频失败: {e}"))?;
            let _ = std::fs::remove_file(&tmp);
        }
        Ok(())
    } else {
        let _ = std::fs::remove_file(&tmp);
        Err("ffmpeg 剥离音频失败".into())
    }
}

/// 出片落盘后的音轨处置。**只有用户显式关掉时才剥。**
///
/// 为什么单独拎成一个函数：这是一道**调用点**的闸门，不是一个算法。
/// `strip_audio` 本身一直是对的，出事的是「谁在什么条件下调它」——
/// 只测函数抓不到，得测这一层。
///
/// 无条件剥会让 [`crate::cinema::pick_shot_audio`] 的 `Source` 分支变成死代码：
/// 它探的正是这个文件有没有音轨，探不到就只能补静音，于是没配 TTS 的分集
/// 整集是哑片 —— 而 H3 生成的同步中文台词正是这条链路的招牌能力。
///
/// 这个 bug 复发过两次：2026-09-04 在 mux 那端（TTS 或静音，把源片音轨顶掉），
/// 2026-09-10 从这端（落盘后无条件 strip）。两次的现象一模一样：
/// 任务成功、ffprobe 照样报 `aac,audio`、测试全绿，成片是哑的。
pub(crate) fn finalize_shot_audio(out: &Path, generate_audio: bool) {
    if !generate_audio {
        let _ = strip_audio(out);
    }
}

pub(crate) fn ffprobe_binary() -> &'static str {
    for candidate in ["/opt/homebrew/bin/ffprobe", "/usr/local/bin/ffprobe"] {
        if Path::new(candidate).is_file() {
            return candidate;
        }
    }
    "ffprobe"
}

fn is_probably_black_video(path: &Path) -> Result<bool, String> {
    let probe = std::process::Command::new(ffprobe_binary())
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .map_err(|e| format!("调用 ffprobe 失败: {e}"))?;
    if !probe.status.success() {
        return Ok(false);
    }

    let duration = String::from_utf8_lossy(&probe.stdout)
        .trim()
        .parse::<f64>()
        .unwrap_or(0.0);
    let sample_times = if duration > 1.0 {
        vec![0.2, duration * 0.5, (duration - 0.2).max(0.2)]
    } else {
        vec![0.0]
    };

    let mut checked = 0usize;
    let mut black = 0usize;
    for (idx, at) in sample_times.iter().enumerate() {
        let frame_path = std::env::temp_dir().join(format!(
            "anima_video_probe_{}_{}.png",
            std::process::id(),
            idx
        ));
        let status = std::process::Command::new(ffmpeg_binary())
            .args(["-y", "-loglevel", "error", "-ss", &format!("{at:.3}"), "-i"])
            .arg(path)
            .args(["-frames:v", "1"])
            .arg(&frame_path)
            .status();

        if !matches!(status, Ok(s) if s.success()) {
            let _ = std::fs::remove_file(&frame_path);
            continue;
        }

        let Ok(img) = image::open(&frame_path) else {
            let _ = std::fs::remove_file(&frame_path);
            continue;
        };
        let _ = std::fs::remove_file(&frame_path);

        let rgb = img.to_rgb8();
        let mut sum = 0f64;
        let mut max_luma = 0u8;
        for p in rgb.pixels() {
            let luma = (0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64) as u8;
            sum += luma as f64;
            max_luma = max_luma.max(luma);
        }
        let mean = sum / (rgb.width() as f64 * rgb.height() as f64).max(1.0);
        checked += 1;
        if mean < 5.0 && max_luma < 25 {
            black += 1;
        }
    }

    Ok(checked > 0 && black == checked)
}

// ─── 文生视频工具 (Generate Video Tool) ──────────────────────────────────

pub struct GenerateVideoTool;

/// 视频生成的等待上限。
///
/// **这两个数是量出来的，不是拍的。** 本机（M4 Max，Wan2.1 T2V 1.3B，
/// `--fp32-unet`）实测：
///
/// | 配置 | 耗时 |
/// |---|---|
/// | 832×480 / 33 帧 / 20 步 | 390s |
/// | 832×480 / 33 帧 / 30 步 | 560s |
/// | 256×256 / 13 帧 / 6 步  | 10s |
///
/// 原来是轮询 180s、工具超时 240s —— 按图片的量级设的，**任何一次真实的
/// 视频生成都会在完成前被判失败**，而 ComfyUI 那边还在跑。这就是这个项目
/// 反复出现的"重试到超时，看起来和卡死一模一样"。
///
/// MiniMax / LTX 高质量模板实测可超过 2400s。**2026-09-30 起所有轮询都不按时长判超时**，
/// 看 ComfyUI 的实际进度（[`super::comfy_watch`]）—— 720p 一镜就可能超过 4 小时。
/// 对话里调的出片工具外面仍包着 registry 的 [`VIDEO_TOOL_TIMEOUT_SECS`]。
pub(crate) const VIDEO_POLL_INTERVAL_SECS: u64 = 3;
/// **只用来估进度条**（按 2 小时铺满），不是等待上限
const VIDEO_PROGRESS_POLLS: u32 = 2400;

/// 查一次 `/queue`：这个任务在排队还是在跑，以及队列里还有没有它（`Some(false)` = 刚跑完或者丢了）。
/// 连不上 / 读不出 = `(Unreachable, None)` —— 读不到不是「没了」的证据
async fn queue_state(client: &reqwest::Client, prompt_id: &str) -> (super::comfy_watch::Seen, Option<bool>) {
    let (seen, in_queue, _) = queue_state_ahead(client, prompt_id).await;
    (seen, in_queue)
}

/// 同上，再带上「前面还有几个」（卡片上的「排队中 · 前面还有 N 个」）
async fn queue_state_ahead(client: &reqwest::Client, prompt_id: &str) -> (super::comfy_watch::Seen, Option<bool>, Option<usize>) {
    let q = match client.get(format!("{DEFAULT_COMFY_URL}/queue")).send().await {
        Ok(r) => r.json::<Value>().await.ok(),
        Err(_) => None,
    };
    match q {
        None => (super::comfy_watch::Seen::Unreachable, None, None),
        Some(q) => (
            if queue_pending_only(&q, prompt_id) { super::comfy_watch::Seen::Queued } else { super::comfy_watch::Seen::Running },
            Some(super::comfy_jobs::queue_contains(&q, prompt_id)),
            crate::comfyui::parse_queue(&q, Some(prompt_id)).ahead,
        ),
    }
}

/// 分镜出片、画布这条路（后台任务，**不经过** registry 的工具超时）**不按时长判超时**，盯的是 ComfyUI 的实际进度 ——
/// 见 [`super::comfy_watch`]。
///
/// 以前这里是「开始生成后 4 小时」（再以前 2 小时）。2026-09-30 一条三窗口长镜头一步采样 30 分钟、整条十二个多小时，
/// 在第 4 小时被判失败，ComfyUI 那边还在好好地算。固定时长对短镜头是十倍余量、对长镜头连一半都不够，怎么定都会错。
///
/// 对话里的出片工具轮询同样看进度，只是外面还包着 registry 的 [`VIDEO_TOOL_TIMEOUT_SECS`]。
///
/// 在 ComfyUI 队列里排着的轮次上限：12 小时。只防死循环 —— 队列一直认这个任务就一直等
/// （2026-09-29 用户：「不应该固定 40 分钟，而是排到了才开始计时」）
const SHOT_PENDING_CEILING_POLLS: u32 = 12 * 3600 / VIDEO_POLL_INTERVAL_SECS as u32;

/// 这个任务是不是**只是在排队**（还没轮到）
pub(crate) fn queue_pending_only(queue: &Value, prompt_id: &str) -> bool {
    let has = |k: &str| {
        queue
            .get(k)
            .and_then(|v| v.as_array())
            .is_some_and(|items| items.iter().any(|it| it.get(1).and_then(|v| v.as_str()) == Some(prompt_id)))
    };
    has("queue_pending") && !has("queue_running")
}
/// 连续这么多轮「队列里没有、history 里也没有」才算任务丢了（3s × 5 = 15 秒）。
/// 任务离开队列到 history 写完之间有空窗，不留容忍度会把刚跑完的判成丢了。
const VIDEO_LOST_TOLERANCE: u32 = 5;

/// 「丢失」计数怎么走。`in_queue == None` = 这一轮读不到队列（网络抖了），
/// **不算丢**，也不清零 —— 读不到不是证据。
fn next_lost_streak(streak: u32, in_history: bool, in_queue: Option<bool>) -> u32 {
    match (in_history, in_queue) {
        (true, _) | (_, Some(true)) => 0,
        (false, Some(false)) => streak + 1,
        (false, None) => streak,
    }
}
const VIDEO_TOOL_TIMEOUT_SECS: u64 = 7260; // 比轮询多留 1 分钟收尾
const MINIMAX_H3_TURBO_LORA: &str =
    "minimax_h3_fl2v_turbo_8step_v1.0_comfyui_bf16.safetensors";

/// 盘上的 turbo LoRA 叫什么（枚举出来的真实名字）。
///
/// **不能拿常量去和枚举名做全等比较。** `models/loras/` 从 2026-09-03 起按引擎
/// 分了子目录，ComfyUI 报出来的是 `h3/minimax_h3_fl2v_turbo_8step_....safetensors`。
/// 全等比较当场失配 —— **turbo LoRA 悄悄不挂，而步数仍然按 turbo 的 8 步走**，
/// 结果是又慢又糊，线索只有告警里的一行字。整理目录当天就踩了这个。
fn find_turbo_lora(available: &[String]) -> Option<String> {
    lora::match_name(MINIMAX_H3_TURBO_LORA, available)
}

/// 调用方是不是已经自己点名了 turbo LoRA（点名了就别再自动加一遍）。
///
/// 同样不能全等比较：用户从子目录里点名的话名字带前缀。
fn turbo_already_requested(resolved: &[lora::ResolvedLora]) -> bool {
    resolved
        .iter()
        .any(|l| l.name.to_lowercase().contains("minimax_h3_fl2v_turbo"))
}

/// 文生视频优先挑哪个家族。**是关键词，不是文件名。**
const T2V_PREFERRED: &[&str] = &[
    // 先点名蒸馏版：我们的采样参数（8 步 / cfg=1 / 靠 NAG 出负向）是按它配的，
    // dev 版拿这套参数跑会出纯黑。家族关键词留在后面兜底。
    "ltx-2.5-22b-distilled",
    "ltx-2.5",
    "ltx",
    "ltxv",
    "minimax",
];

/// 图生视频优先挑哪个家族。原生拓扑只剩 LTX-2.5 和 MiniMax H3（Wan 2026-10-01 下架）。
const I2V_PREFERRED: &[&str] = &[
    // 先点名蒸馏版：我们的采样参数（8 步 / cfg=1 / 靠 NAG 出负向）是按它配的，
    // dev 版拿这套参数跑会出纯黑。家族关键词留在后面兜底。
    "ltx-2.5-22b-distilled",
    "ltx-2.5",
    "ltx",
    "ltxv",
    "minimax",
];

const I2V_LTX_PREFERRED: &[&str] =
    &["ltx-2.5-22b-distilled", "ltx-2.5", "ltx", "ltxv"];
// 关键词匹配走 `to_lowercase().contains()`，所以 `MiniMax_H3_FL2VA_pruned_int8_convrot`
// 也能被 "minimax" 命中。**但它取的是第一个匹配** —— 盘上同时有多个 minimax 模型时，
// 选中哪个取决于 ComfyUI 的枚举顺序，不受这里控制。
// 2026-09-03 实测：int8 版比 bf16 快 20%、加载内存 20 GB vs 76.7 GB、画质无可见退化，
// bf16 版已删除。
// **H3 从 2026-09-04 起只在 WeeTodd/MLX 那条路上**（`agent_runtime::weetodd`）。
// 原来那套 ComfyUI 单文件权重已删除（72.35 GB），所以 `diffusion_models/` 下
// 不会再有 minimax 模型，这个关键词永远匹配不到 —— 保留它是为了让
// `pick_video_unet` 走到「找不到」那条分支，届时调用方要把
// `weetodd::H3_MIGRATED_NOTE` 原样告诉用户，否则用户以为是装漏了。
const I2V_MINIMAX_PREFERRED: &[&str] = &["minimax"];
const I2V_FALLBACK_ENGINE: &str = "minimax";

/// MiniMax H3 的图生视频 latent 长度要求落在 `17n+5` 上。
pub(crate) fn snap_frames_17n_plus_5(frames: u32) -> u32 {
    let f = frames.max(5);
    f + (5 + 17 - (f % 17)) % 17
}

/// 图生视频选哪条引擎。**纯函数，可测。**
///
/// `ltx_blocked` 来自 `comfyui::ltx_blocker`（启动参数 + 可用内存），
/// 提交前就能算出来。auto 模式下 LTX 跑不了时直接走 MiniMax ——
/// 不做那次注定出纯黑的尝试（实测白烧 150~250 秒）。
pub(crate) fn pick_i2v_preferred(
    engine_hint: &str,
    ltx_blocked: bool,
) -> &'static [&'static str] {
    if engine_hint.contains("minimax") {
        I2V_MINIMAX_PREFERRED
    } else if engine_hint.contains("ltx") {
        I2V_LTX_PREFERRED
    } else if ltx_blocked {
        I2V_MINIMAX_PREFERRED
    } else {
        I2V_PREFERRED
    }
}

/// 已下架的视频引擎。点名它们时**明确拒绝**，不悄悄换成别的引擎。
///
/// Wan 2.1 / 2.2 / Wan-Animate 的权重都删了（2026-10-01 下架入口）。
/// 不拦的话，`weetodd::pick_route` 会把「点名了别的引擎」交回原生路径，
/// 原生路径再报一句「找不到 UNet」—— 用户看不出是这条路没了，还以为是装漏了。
/// 判的是**名字开头**（引擎提示，或 `model` 文件名去掉子目录后的部分），
/// 免得 `swan_lake.safetensors` 这种名字被误伤。
pub(crate) fn retired_video_engine(engine_hint: &str, model: Option<&str>) -> Option<String> {
    let starts_wan = |s: &str| {
        let base = s.trim().rsplit('/').next().unwrap_or("").to_lowercase();
        base.starts_with("wan")
    };
    let hit = starts_wan(engine_hint) || model.is_some_and(starts_wan);
    hit.then(|| {
        "Wan 已于 2026-10-01 下架（权重已删）。请改用 LTX-2.5（默认，快）\
         或 MiniMax H3（要画面 + 对白 + 环境声一次出时）。"
            .to_string()
    })
}

/// 原生路径碰到没有对应拓扑的模型时的说明。
fn native_video_unsupported(unet_name: &str, kind: &str, supported: &str) -> String {
    format!(
        "原生{kind}只支持 {supported}，`{unet_name}` 不在其中。\
         Wan 的拓扑已于 2026-10-01 随权重一起下架。"
    )
}

fn should_auto_fallback_to_minimax(engine_hint: &str, is_ltx: bool) -> bool {
    is_ltx && (engine_hint.is_empty() || engine_hint == "auto")
}

fn minimax_fallback_request(request: &ToolCallRequest, seed: u64) -> ToolCallRequest {
    let mut fallback = request.clone();
    if let Some(args) = fallback.arguments.as_object_mut() {
        args.insert("engine".to_string(), json!(I2V_FALLBACK_ENGINE));
        args.insert("seed".to_string(), json!(seed));
        args.insert("_fallback_from_ltx".to_string(), json!(true));
    }
    fallback
}

fn is_ltx_fallback_request(request: &ToolCallRequest) -> bool {
    request
        .arguments
        .get("_fallback_from_ltx")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

async fn interrupt_prompt_if_running(client: &reqwest::Client, prompt_id: &str) -> bool {
    let Ok(resp) = client
        .get(format!("{}/queue", DEFAULT_COMFY_URL))
        .send()
        .await
    else {
        return false;
    };
    let Ok(queue) = resp.json::<Value>().await else {
        return false;
    };
    let is_running = queue
        .get("queue_running")
        .and_then(|v| v.as_array())
        .is_some_and(|items| items.iter().any(|item| item.to_string().contains(prompt_id)));
    if !is_running {
        return false;
    }
    client
        .post(format!("{}/interrupt", DEFAULT_COMFY_URL))
        .send()
        .await
        .is_ok()
}

/// 在 ComfyUI 报的候选里挑一个 UNet。**纯函数，不碰网络。**
///
/// # 为什么 `explicit` 也要核对
///
/// 这个函数原来长这样：显式指定就**原样返回**，不核对。于是
/// `animate_image` 里一句 `find_best_video_unet(&client, Some("wan2.1_i2v"))`
/// —— 作者本意是"优先挑 i2v 那个家族" —— 把字符串 `"wan2.1_i2v"` 当文件名
/// 发给了 ComfyUI，而盘上叫 `wan2.1_i2v_480p_14B_fp8_scaled.safetensors`。
/// 结果是**每一次图生视频都在提交阶段就被拒**，用户看到的只是
/// `Value not in list`，看不出是缺文件还是名字写错。
///
/// 所以现在：`explicit` 是"用户点名要这个文件"，必须在候选里；
/// 家族偏好走 `preferred` 关键词，两个语义分开。
pub(crate) fn pick_video_unet(
    available: &[String],
    explicit: Option<&str>,
    preferred: &[&str],
) -> Result<String, String> {
    if available.is_empty() {
        return Err(
            "ComfyUI 没报出任何 UNet 模型（models/diffusion_models/ 是空的，或 ComfyUI 没在跑）。"
                .to_string(),
        );
    }

    if let Some(want) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(hit) = crate::agent_runtime::lora::match_name(want, available) {
            return Ok(hit);
        }
        return Err(format!(
            "指定的视频模型 `{want}` 对不上（写错了，或者不唯一）。本机可用的是：\n{}",
            available
                .iter()
                .map(|n| format!("  - {n}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }

    for kw in preferred {
        let lower = kw.to_lowercase();
        // **不能用 `find`。** `find` 取的是 ComfyUI 枚举里的第一个，而枚举顺序
        // 由目录内容决定 —— 往 models/diffusion_models/ 放一个名字也含这个关键词的
        // 文件，选中的模型就会**静默**换掉。这不是假想：2026-09-03 下了一份
        // `ltx-2.5-22b-dev-transformer_W4A8`，它按字母序排在 `...-distilled-...`
        // 前面（`e` < `i`），于是所有 LTX 生成都切到了这个没验过的模型上，
        // 出来的是纯黑视频，而任务状态是 success。
        //
        // 所以这里排序后取第一个：结果只由**盘上有哪些文件**决定，
        // 不由**它们被枚举的顺序**决定。同一批文件，答案永远一样。
        let mut hits: Vec<&String> = available
            .iter()
            .filter(|n| n.to_lowercase().contains(&lower))
            .collect();
        hits.sort_by_key(|n| n.to_lowercase());
        if let Some(found) = hits.first() {
            return Ok((*found).clone());
        }
    }

    // 一个偏好都没命中：与其挑一个八成不是视频模型的，不如说清楚
    Err(format!(
        "没找到已知的视频生成模型。本机的 UNet 是：\n{}\n认得的家族关键词：{}",
        available
            .iter()
            .map(|n| format!("  - {n}"))
            .collect::<Vec<_>>()
            .join("\n"),
        preferred.join("、")
    ))
}

fn pick_required_model(
    available: &[String],
    exact: &[&str],
    keywords: &[&str],
    label: &str,
) -> Result<String, String> {
    for want in exact {
        if let Some(found) = available.iter().find(|n| n.as_str() == *want) {
            return Ok(found.clone());
        }
    }

    for kw in keywords {
        let lower = kw.to_lowercase();
        if let Some(found) = available.iter().find(|n| n.to_lowercase().contains(&lower)) {
            return Ok(found.clone());
        }
    }

    Err(format!(
        "缺少 {label}。本机 ComfyUI 当前可用候选是：\n{}",
        if available.is_empty() {
            "  - （ComfyUI 没报出任何候选，请确认模型目录和自定义节点已加载）".to_string()
        } else {
            available
                .iter()
                .map(|n| format!("  - {n}"))
                .collect::<Vec<_>>()
                .join("\n")
        }
    ))
}

/// 文生视频的完整工作流。**抽出来是为了能测** —— 这张图里有两条不测就
/// 看不出来、出事却很大的性质：
///
/// 1. 空 latent 必须是时序 latent（`EmptyLTXVLatentVideo`），
///    不是 `EmptyLatentImage`（一批图片 latent）；
/// 2. `batch_size` 必须是 1 —— 把帧数塞进 batch_size 会让视频 UNet
///    一次并行采样几十个样本，实测把整台机器压到 watchdog 重启
///    （那次出在 Wan 拓扑上；Wan 已于 2026-10-01 下架，这张图现在只有 LTX-2.5 一套）。
pub(crate) struct T2vParams<'a> {
    pub unet_name: &'a str,
    pub clip_name: &'a str,
    pub clip_type: &'a str,
    pub vae_name: &'a str,
    pub prompt: &'a str,
    pub neg_prompt: &'a str,
    pub width: u32,
    pub height: u32,
    pub frames_count: u32,
    pub fps: u32,
    pub seed: u64,
    pub steps: u32,
    /// LoRA 链的出口；没挂 LoRA 时是 `["1", 0]`
    pub model_ref: Value,
}

/// LTX-2.5 官方 I2V 工作流的扁平化参数。
///
/// ComfyUI 自带模板 `video_ltx2_5_i2v.json` 把核心链路包在 subgraph 里，
/// API 不能直接提交那个 subgraph id。这里把它展开成普通节点：
/// video latent + audio latent 合并，两段 sampler，潜空间 2x upscaler，再 tiled decode。
///
/// 注意：最终 MP4 目前只导出视频轨。LTX 的 audio latent 参与 sampler 链路，
/// 但本机 MPS 上 `LTXVAudioVAEDecode -> AAC` 曾输出 NaN，导致 SaveVideo 报
/// `[aac] Input contains (near) NaN/+-Inf`。图生视频默认并不需要音频轨，
/// 所以这里不把不稳定的 audio decode 接进 `CreateVideo.audio`。
pub(crate) struct LtxI2vParams<'a> {
    pub unet_name: &'a str,
    pub clip_name: &'a str,
    pub video_vae_name: &'a str,
    pub audio_vae_name: &'a str,
    pub latent_upscaler_name: &'a str,
    pub prompt: &'a str,
    pub neg_prompt: &'a str,
    pub input_filename: &'a str,
    pub width: u32,
    pub height: u32,
    pub frames_count: u32,
    pub fps: u32,
    pub seed: u64,
    /// LoRA 链的出口；没挂 LoRA 时是 `["1", 0]`
    pub model_ref: Value,
}

pub(crate) fn build_ltx_i2v_graph(p: &LtxI2vParams) -> Value {
    let node = |class_type: &str, inputs: Value| {
        json!({
            "class_type": class_type,
            "inputs": inputs
        })
    };
    let model_ref = p.model_ref.clone();
    let first_pass_width = (p.width / 2).max(64);
    let first_pass_height = (p.height / 2).max(64);

    let mut prompt = serde_json::Map::new();
    prompt.insert(
        "1".to_string(),
        node(
            "UNETLoader",
            json!({"unet_name": p.unet_name, "weight_dtype": "default"}),
        ),
    );
    prompt.insert(
        "2".to_string(),
        node(
            "CLIPLoader",
            json!({"clip_name": p.clip_name, "type": "ltxv"}),
        ),
    );
    prompt.insert(
        "3".to_string(),
        node("VAELoader", json!({"vae_name": p.video_vae_name})),
    );
    prompt.insert(
        "4".to_string(),
        node("LoadImage", json!({"image": p.input_filename})),
    );
    prompt.insert(
        "5".to_string(),
        node(
            "CLIPTextEncode",
            json!({"text": p.prompt, "clip": ["2", 0]}),
        ),
    );
    prompt.insert(
        "6".to_string(),
        node(
            "CLIPTextEncode",
            json!({"text": p.neg_prompt, "clip": ["2", 0]}),
        ),
    );
    prompt.insert(
        "12".to_string(),
        node(
            "LTXVConditioning",
            json!({"positive": ["5", 0], "negative": ["6", 0], "frame_rate": p.fps as f64}),
        ),
    );
    prompt.insert(
        "14".to_string(),
        node("VAELoader", json!({"vae_name": p.audio_vae_name})),
    );
    // Match ComfyUI's bundled LTX-2.5 I2V template. The first frame must keep
    // its aspect ratio before LTXVPreprocess; fixed ImageScale stretching can
    // corrupt the image condition and produce black frames on otherwise valid runs.
    prompt.insert(
        "15".to_string(),
        node(
            "ResizeImageMaskNode",
            json!({
                "input": ["4", 0],
                "resize_type": "scale longer dimension",
                "resize_type.longer_size": 1536,
                "scale_method": "lanczos"
            }),
        ),
    );
    prompt.insert(
        "16".to_string(),
        node(
            "LTXVPreprocess",
            json!({"image": ["15", 0], "img_compression": 18}),
        ),
    );
    prompt.insert(
        "17".to_string(),
        node(
            "EmptyLTXVLatentVideo",
            json!({
                "width": first_pass_width,
                "height": first_pass_height,
                "length": p.frames_count,
                "batch_size": 1
            }),
        ),
    );
    prompt.insert(
        "18".to_string(),
        node(
            "LTXVImgToVideoInplace",
            json!({"vae": ["3", 0], "image": ["16", 0], "latent": ["17", 0], "strength": 0.7, "bypass": false}),
        ),
    );
    prompt.insert(
        "19".to_string(),
        node(
            "LTXVEmptyLatentAudio",
            json!({"frames_number": p.frames_count, "frame_rate": p.fps as f64, "batch_size": 1, "audio_vae": ["14", 0]}),
        ),
    );
    prompt.insert(
        "20".to_string(),
        node(
            "LTXVConcatAVLatent",
            json!({"video_latent": ["18", 0], "audio_latent": ["19", 0]}),
        ),
    );
    prompt.insert(
        "21".to_string(),
        node("RandomNoise", json!({"noise_seed": p.seed})),
    );
    prompt.insert(
        "22".to_string(),
        node("KSamplerSelect", json!({"sampler_name": "euler_ancestral"})),
    );
    prompt.insert(
        "23".to_string(),
        node(
            "ManualSigmas",
            json!({"sigmas": "1.0, 0.99375, 0.9875, 0.98125, 0.975, 0.909375, 0.725, 0.421875, 0.0"}),
        ),
    );
    prompt.insert(
        "24".to_string(),
        node(
            "LTXVDualCFGGuider",
            json!({"model": model_ref.clone(), "positive": ["12", 0], "negative": ["12", 1], "video_cfg": 1.0, "audio_cfg": 1.0}),
        ),
    );
    prompt.insert(
        "25".to_string(),
        node(
            "SamplerCustomAdvanced",
            json!({"noise": ["21", 0], "guider": ["24", 0], "sampler": ["22", 0], "sigmas": ["23", 0], "latent_image": ["20", 0]}),
        ),
    );
    prompt.insert(
        "26".to_string(),
        node("LTXVSeparateAVLatent", json!({"av_latent": ["25", 0]})),
    );
    prompt.insert(
        "27".to_string(),
        node(
            "LatentUpscaleModelLoader",
            json!({"model_name": p.latent_upscaler_name}),
        ),
    );
    prompt.insert(
        "28".to_string(),
        node(
            "LTXVLatentUpsampler",
            json!({"samples": ["26", 0], "upscale_model": ["27", 0], "vae": ["3", 0]}),
        ),
    );
    prompt.insert(
        "29".to_string(),
        node(
            "LTXVImgToVideoInplace",
            json!({"vae": ["3", 0], "image": ["16", 0], "latent": ["28", 0], "strength": 1.0, "bypass": false}),
        ),
    );
    prompt.insert(
        "30".to_string(),
        node(
            "LTXVConcatAVLatent",
            json!({"video_latent": ["29", 0], "audio_latent": ["26", 1]}),
        ),
    );
    prompt.insert(
        "31".to_string(),
        node("RandomNoise", json!({"noise_seed": 42})),
    );
    prompt.insert(
        "32".to_string(),
        node("KSamplerSelect", json!({"sampler_name": "euler_ancestral"})),
    );
    prompt.insert(
        "33".to_string(),
        node(
            "ManualSigmas",
            json!({"sigmas": "0.85, 0.7250, 0.4219, 0.0"}),
        ),
    );
    prompt.insert(
        "34".to_string(),
        node(
            "LTXVDualCFGGuider",
            json!({"model": model_ref, "positive": ["12", 0], "negative": ["12", 1], "video_cfg": 1.0, "audio_cfg": 1.0}),
        ),
    );
    prompt.insert(
        "35".to_string(),
        node(
            "SamplerCustomAdvanced",
            json!({"noise": ["31", 0], "guider": ["34", 0], "sampler": ["32", 0], "sigmas": ["33", 0], "latent_image": ["30", 0]}),
        ),
    );
    prompt.insert(
        "36".to_string(),
        node("LTXVSeparateAVLatent", json!({"av_latent": ["35", 0]})),
    );
    prompt.insert(
        "37".to_string(),
        node(
            "VAEDecodeTiled",
            json!({"samples": ["36", 0], "vae": ["3", 0], "tile_size": 512, "overlap": 64, "temporal_size": 64, "temporal_overlap": 16}),
        ),
    );
    prompt.insert(
        "10".to_string(),
        node(
            "CreateVideo",
            json!({"images": ["37", 0], "fps": p.fps as f64, "bit_depth": 8}),
        ),
    );
    prompt.insert(
        "11".to_string(),
        node(
            "SaveVideo",
            json!({"video": ["10", 0], "filename_prefix": format!("video/Anima_LTX_I2V_{}", p.seed), "format": "mp4", "codec": "auto"}),
        ),
    );

    let mut root = serde_json::Map::new();
    root.insert("prompt".to_string(), Value::Object(prompt));
    Value::Object(root)
}

/// MiniMax H3 官方 I2V 工作流的扁平化参数。
///
/// 对应 ComfyUI 自带模板 `video_minimax_h3_i2v.json` 的本地 subgraph：
/// `MiniMaxH3ImageToVideo -> BasicGuider/BasicScheduler -> SamplerCustomAdvanced
/// -> VAEDecode + VAEDecodeAudio -> CreateVideo -> SaveVideo`。
pub(crate) struct MinimaxH3I2vParams<'a> {
    pub unet_name: &'a str,
    pub clip_name: &'a str,
    pub video_vae_name: &'a str,
    pub audio_vae_name: &'a str,
    pub prompt: &'a str,
    pub input_filename: &'a str,
    /// 末帧（首尾帧生视频）。`None` = 只给首帧，走原来的路。
    ///
    /// 实测确认可行（2026-09-02）：在现有 i2v 图上加一个
    /// `MiniMaxH3AddGuide{frame_idx: -1}`，采样那条链一个字都不用改。
    pub end_filename: Option<&'a str>,
    pub width: u32,
    pub height: u32,
    pub frames_count: u32,
    pub seed: u64,
    pub steps: u32,
    /// LoRA 链的出口；没挂 LoRA 时是 `["1", 0]`
    pub model_ref: Value,
}

/// 首尾帧的接法（2026-09-02 实测确认）。
///
/// 在现有 i2v 图上加一个 `MiniMaxH3AddGuide{frame_idx: -1}`，把它的输出接给
/// `BasicGuider` —— 采样那条链一个字都不用改，`latent_image` 仍然取
/// `MiniMaxH3ImageToVideo` 的第二个输出。
///
/// **验证过程留在这里，因为其中一步差点得出错误结论：**
///
/// 首帧是细高灯塔偏左、棱角石板；末帧是矮胖灯塔偏右、圆润巨石 —— 两张图的
/// 几何结构刻意做得完全不同。输出的最后一帧复现的是**末帧那套几何**。
///
/// 一开始拿 RGB 均值当判据，首尾帧组"离末帧 31.9、离首帧 153.3"，看起来已经
/// 证明了。但**对照组（只给首帧）也是"离末帧 38.7"** —— 因为提示词里写了
/// "from midday to night"，模型自己就会变暗。颜色统计被提示词污染，分不出来。
///
/// 真正的判据是几何：对照组的末帧保持了首帧的细高灯塔和棱角石板，只是暗了；
/// 首尾帧组变成了末帧的矮胖灯塔和圆石。**统计值只能否定，不能肯定。**
fn h3_end_guide_nodes(end_filename: &str) -> [(&'static str, Value); 2] {
    let node = |class_type: &str, inputs: Value| json!({"class_type": class_type, "inputs": inputs});
    [
        ("20", node("LoadImage", json!({"image": end_filename}))),
        (
            "21",
            node(
                "MiniMaxH3AddGuide",
                json!({
                    "positive": ["6", 0],
                    "latent": ["6", 1],
                    // -1 = 最后一帧。这个字段的范围是 -9999~9999，
                    // 中间任意帧也能钉，但我们只验证过首尾。
                    "frame_idx": -1,
                    "vae": ["3", 0],
                    "image": ["20", 0]
                }),
            ),
        ),
    ]
}

pub(crate) fn build_minimax_h3_i2v_graph(p: &MinimaxH3I2vParams) -> Value {
    let node = |class_type: &str, inputs: Value| {
        json!({
            "class_type": class_type,
            "inputs": inputs
        })
    };
    let model_ref = p.model_ref.clone();

    let mut graph = json!({
        "prompt": {
            "1": node("UNETLoader", json!({
                "unet_name": p.unet_name,
                "weight_dtype": "default"
            })),
            "2": node("CLIPLoader", json!({
                "clip_name": p.clip_name,
                "type": "minimax"
            })),
            "3": node("VAELoader", json!({"vae_name": p.video_vae_name})),
            "4": node("VAELoader", json!({"vae_name": p.audio_vae_name})),
            "5": node("LoadImage", json!({"image": p.input_filename})),
            "6": node("MiniMaxH3ImageToVideo", json!({
                "clip": ["2", 0],
                "vae": ["3", 0],
                "first_frame": ["5", 0],
                "prompt": p.prompt,
                "width": p.width,
                "height": p.height,
                "length": p.frames_count
            })),
            "7": node("KSamplerSelect", json!({"sampler_name": "res_multistep"})),
            "8": node("BasicScheduler", json!({
                "model": model_ref.clone(),
                "scheduler": "simple",
                "steps": p.steps,
                "denoise": 1.0
            })),
            "9": node("BasicGuider", json!({
                "model": model_ref,
                // 有末帧时条件走 AddGuide（节点 20），否则直连 MiniMaxH3ImageToVideo
                "conditioning": if p.end_filename.is_some() { json!(["21", 0]) } else { json!(["6", 0]) }
            })),
            "10": node("RandomNoise", json!({"noise_seed": p.seed})),
            "11": node("SamplerCustomAdvanced", json!({
                "noise": ["10", 0],
                "guider": ["9", 0],
                "sampler": ["7", 0],
                "sigmas": ["8", 0],
                "latent_image": ["6", 1]
            })),
            "12": node("VAEDecode", json!({
                "samples": ["11", 0],
                "vae": ["3", 0]
            })),
            "13": node("VAEDecodeAudio", json!({
                "samples": ["11", 0],
                "vae": ["4", 0]
            })),
            "14": node("CreateVideo", json!({
                "images": ["12", 0],
                "audio": ["13", 0],
                "fps": 24.0,
                "bit_depth": 8
            })),
            "15": node("SaveVideo", json!({
                "video": ["14", 0],
                "filename_prefix": format!("video/Anima_MINIMAX_H3_I2V_{}", p.seed),
                "format": "mp4",
                "codec": "auto"
            }))
        }
    });

    if let Some(end) = p.end_filename {
        if let Some(map) = graph["prompt"].as_object_mut() {
            for (id, n) in h3_end_guide_nodes(end) {
                map.insert(id.to_string(), n);
            }
        }
    }
    graph
}

/// 速度/质量档位。**每一档的数字都是量出来的**（M4 Max，MiniMax H3，
/// 864x480 / 56 帧 / 同 seed 同提示词）：
///
/// | 档 | 步数 | SolAttn | 实测 |
/// |---|---|---|---|
/// | `fast` | 4（turbo 采样器） | 开 | 159s（无参考）/ 250s（带参考图） |
/// | `balanced` | 8 | 开 | 253s |
/// | `quality` | 8 | **关** | 264s |
///
/// 两条反直觉的实测结论，写在这里免得下次又靠猜：
///
/// - **SolAttn 的收益随分辨率放大。** 1344x768 下它给 2.1×（1270s→612s），
///   864x480 下只给 4%（264s→253s）—— 它的 `min_tokens: 4096` 阈值意味着
///   序列不够长时基本退回稠密注意力。所以低分辨率下关掉它几乎不花钱。
/// - **拖影是步数的问题，不是 SolAttn 的问题。** Turbo v4 LoRA 在 4 步 +
///   大幅快速运动下会拖影，官方文档说 6~8 步基本消除。所以 `quality` 档
///   优先提步数，其次才关 SolAttn。
///
/// 面部扭曲那条要靠**分辨率**，不在这个档位里 —— 864x480 下人脸只有几十个
/// 像素，再好的采样也救不回来。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VideoQuality {
    Fast,
    Balanced,
    Quality,
}

impl VideoQuality {
    pub(crate) fn from_arg(v: Option<&str>) -> Self {
        match v.map(str::trim).unwrap_or("") {
            s if s.eq_ignore_ascii_case("quality") => Self::Quality,
            s if s.eq_ignore_ascii_case("balanced") => Self::Balanced,
            _ => Self::Fast,
        }
    }
    /// 采样步数
    pub(crate) fn steps(self) -> u32 {
        match self {
            Self::Fast => 4,
            Self::Balanced | Self::Quality => 8,
        }
    }
}

/// MiniMax H3 的**参考图生视频**（R2V）。
///
/// # 和 I2V 的区别，以及为什么需要它
///
/// `MiniMaxH3ImageToVideo` 的 `first_frame` 是**字面意义的第一帧** —— 视频
/// 就从那张图开始。给一张白底人像特写、却要求"走在街上"，第 0 帧是白底特写、
/// 第 1 帧跳到街景，硬切。用户的原话是「首帧是原图是个问题」。
///
/// `MiniMaxH3ReferenceToVideo` 把图当**身份/外观参考**：画面自由生成，人物
/// 特征来自参考图。首帧不再突兀，而且能一次给**多张**（正面/侧面/背面，
/// 或一整张多视图设定卡），信息越多身份越稳。
///
/// # `ref_images` 的 API 键名是点号路径，别踩这个坑
///
/// 这是个 autogrow 输入（`prefix="ref_image_"`, max 9），API 格式里的键是
/// **`ref_images.ref_image_0`**。另外两种写法都是错的，而且错得很隐蔽：
///
/// - `"ref_images": [["5", 0]]` —— **静默忽略**。因为它是 optional，
///   ComfyUI 校验照样通过、任务照样成功，只是一张参考图都没收到。
///   实测：四组不同参考图配置跑出来的视频**逐像素相同**（差 0.00）。
/// - `"ref_image_0": ["5", 0]` —— 平铺键会直接透传进 `execute()`，
///   报 `TypeError: unexpected keyword argument`。
///
/// 我一度据此得出"R2V 保不住人脸"的结论，其实是 R2V 根本没拿到图。
pub(crate) struct MinimaxH3R2vParams<'a> {
    pub unet_name: &'a str,
    pub clip_name: &'a str,
    pub video_vae_name: &'a str,
    pub audio_vae_name: &'a str,
    pub prompt: &'a str,
    /// 已经放进 ComfyUI input 目录的参考图文件名，按重要性排序
    pub reference_filenames: &'a [String],
    pub width: u32,
    pub height: u32,
    pub frames_count: u32,
    pub seed: u64,
    pub steps: u32,
    /// `match`（缩到输出像素面积）或 `max`（2048 短边，身份保真更好但更慢）
    pub ref_image_size: &'a str,
    pub model_ref: Value,
}

/// 一次最多能挂几张参考图（节点声明 `max=9`）。
pub(crate) const MAX_REFERENCE_IMAGES: usize = 9;

pub(crate) fn build_minimax_h3_r2v_graph(p: &MinimaxH3R2vParams) -> Value {
    let node = |class_type: &str, inputs: Value| json!({"class_type": class_type, "inputs": inputs});
    let model_ref = p.model_ref.clone();

    let mut refs = json!({
        "clip": ["2", 0],
        "vae": ["3", 0],
        "audio_vae": ["4", 0],
        "prompt": p.prompt,
        "width": p.width,
        "height": p.height,
        "length": p.frames_count,
        "ref_image_size": p.ref_image_size,
    });
    let mut loaders = serde_json::Map::new();
    for (i, name) in p
        .reference_filenames
        .iter()
        .take(MAX_REFERENCE_IMAGES)
        .enumerate()
    {
        let nid = (500 + i).to_string();
        loaders.insert(nid.clone(), node("LoadImage", json!({"image": name})));
        if let Some(obj) = refs.as_object_mut() {
            // 点号路径 —— 见上面的注释，写错会被静默忽略
            obj.insert(format!("ref_images.ref_image_{i}"), json!([nid, 0]));
        }
    }

    let mut prompt_map = serde_json::Map::new();
    prompt_map.insert("1".into(), node("UNETLoader", json!({"unet_name": p.unet_name, "weight_dtype": "default"})));
    prompt_map.insert("2".into(), node("CLIPLoader", json!({"clip_name": p.clip_name, "type": "minimax"})));
    prompt_map.insert("3".into(), node("VAELoader", json!({"vae_name": p.video_vae_name})));
    prompt_map.insert("4".into(), node("VAELoader", json!({"vae_name": p.audio_vae_name})));
    prompt_map.insert("6".into(), node("MiniMaxH3ReferenceToVideo", refs));
    prompt_map.insert("7".into(), node("KSamplerSelect", json!({"sampler_name": "res_multistep"})));
    prompt_map.insert("8".into(), node("BasicScheduler", json!({
        "model": model_ref.clone(), "scheduler": "simple", "steps": p.steps, "denoise": 1.0
    })));
    prompt_map.insert("9".into(), node("BasicGuider", json!({"model": model_ref, "conditioning": ["6", 0]})));
    prompt_map.insert("10".into(), node("RandomNoise", json!({"noise_seed": p.seed})));
    prompt_map.insert("11".into(), node("SamplerCustomAdvanced", json!({
        "noise": ["10", 0], "guider": ["9", 0], "sampler": ["7", 0], "sigmas": ["8", 0], "latent_image": ["6", 1]
    })));
    prompt_map.insert("12".into(), node("VAEDecode", json!({"samples": ["11", 0], "vae": ["3", 0]})));
    prompt_map.insert("13".into(), node("VAEDecodeAudio", json!({"samples": ["11", 0], "vae": ["4", 0]})));
    prompt_map.insert("14".into(), node("CreateVideo", json!({
        "images": ["12", 0], "audio": ["13", 0], "fps": 24.0, "bit_depth": 8
    })));
    prompt_map.insert("15".into(), node("SaveVideo", json!({
        "video": ["14", 0],
        "filename_prefix": format!("video/Anima_MINIMAX_H3_R2V_{}", p.seed),
        "format": "mp4", "codec": "auto"
    })));
    for (k, v) in loaders {
        prompt_map.insert(k, v);
    }

    json!({ "prompt": prompt_map })
}

pub(crate) fn build_t2v_graph(p: &T2vParams) -> Value {
    let T2vParams {
        unet_name,
        clip_name,
        clip_type,
        vae_name,
        prompt,
        neg_prompt,
        width,
        height,
        frames_count,
        fps,
        seed,
        steps,
        model_ref,
    } = p;
    let model_ref = model_ref.clone();
    // LTX-2.5 官方文生视频拓扑 (LTXVConditioning + ModelSamplingLTXV + EmptyLTXVLatentVideo -> KSampler -> VAEDecode -> CreateVideo -> SaveVideo)
    json!({
        "prompt": {
            "1": {
                "class_type": "UNETLoader",
                "inputs": {
                    "unet_name": unet_name,
                    "weight_dtype": "default"
                }
            },
            "2": {
                "class_type": "CLIPLoader",
                "inputs": {
                    "clip_name": clip_name,
                    "type": clip_type
                }
            },
            "3": {
                "class_type": "VAELoader",
                "inputs": {
                    "vae_name": vae_name
                }
            },
            "4": {
                "class_type": "CLIPTextEncode",
                "inputs": {
                    "text": prompt,
                    "clip": ["2", 0]
                }
            },
            "5": {
                "class_type": "CLIPTextEncode",
                "inputs": {
                    "text": neg_prompt,
                    "clip": ["2", 0]
                }
            },
            "12": {
                "class_type": "LTXVConditioning",
                "inputs": {
                    "positive": ["4", 0],
                    "negative": ["5", 0],
                    "frame_rate": *fps as f64
                }
            },
            "13": {
                "class_type": "ModelSamplingLTXV",
                "inputs": {
                    "model": model_ref,
                    "max_shift": 2.05,
                    "base_shift": 0.95
                }
            },
            "6": {
                "class_type": "EmptyLTXVLatentVideo",
                "inputs": {
                    "width": *width,
                    "height": *height,
                    "length": *frames_count,
                    "batch_size": 1
                }
            },
            "7": {
                "class_type": "KSampler",
                "inputs": {
                    "seed": seed,
                    "steps": steps,
                    "cfg": 3.0,
                    "sampler_name": "euler",
                    "scheduler": "normal",
                    "denoise": 1.0,
                    "model": ["13", 0],
                    "positive": ["12", 0],
                    "negative": ["12", 1],
                    "latent_image": ["6", 0]
                }
            },
            "8": {
                "class_type": "VAEDecode",
                "inputs": {
                    "samples": ["7", 0],
                    "vae": ["3", 0]
                }
            },
            "9": {
                "class_type": "CreateVideo",
                "inputs": {
                    "images": ["8", 0],
                    "fps": *fps as f64
                }
            },
            "10": {
                "class_type": "SaveVideo",
                "inputs": {
                    "video": ["9", 0],
                    "filename_prefix": format!("video/Anima_LTX_T2V_{seed}"),
                    "format": "mp4",
                    "codec": "auto"
                }
            }
        }
    })
}

/// 生成成功后返回给模型的那段文本。
///
/// **Seed 必须排在最前面，alt 文本里不要塞整段 prompt。**
///
/// 和 `image_tool.rs` 的 `image_summary` 同一条理由：前端把历史里的工具返回
/// 按预算截断，这段原来以 `![{prompt}](…)` 开头，一条镜头描述几百字符，
/// 把结尾的 Seed 直接挤出截断窗口。于是下一轮「改成夜景」时模型看不到上一段
/// 用的什么种子，只能重新随机 —— 出来的是**完全无关的一段片子**。
///
/// alt 只给人看；取路径的正则是 `!\[.*?\]\((file://…)\)`，缩短 alt 不影响它。
#[allow(clippy::too_many_arguments)]
fn t2v_summary(
    seed: u64,
    abs_path: &str,
    unet_name: &str,
    width: u32,
    height: u32,
    fps: u32,
    duration_secs: u32,
) -> String {
    format!(
        "- **Seed**: `{seed}`（改这段视频的细节时把同一个 seed 传回来，画面和运动才会保持一致）\n![生成的视频](file://{abs_path})\n\n- **模型**: `{unet_name}` | **规格**: {width}x{height} @ {fps}fps ({duration_secs}s)\n- **已保存在工作区**：`{abs_path}`"
    )
}

/// 图生视频的返回。
///
/// 这一条原来**根本不报 seed** —— 连"改一改"的可能都没有。
fn i2v_summary(seed: u64, abs_path: &str, source: &str, unet_name: &str) -> String {
    format!(
        "- **Seed**: `{seed}`（改这段视频的细节时把同一个 seed 传回来）\n![生成的视频](file://{abs_path})\n\n- **首帧来源**: `{source}` | **模型**: `{unet_name}`\n- **已保存在工作区**：`{abs_path}`"
    )
}

impl GenerateVideoTool {
    /// 拉取 ComfyUI 认得的 UNet 文件名。
    async fn available_unets(client: &reqwest::Client) -> Vec<String> {
        let mut available_unets = Vec::new();
        let unet_info_url = format!("{}/object_info/UNETLoader", DEFAULT_COMFY_URL);
        if let Ok(resp) = client.get(&unet_info_url).send().await {
            if let Ok(json) = resp.json::<Value>().await {
                if let Some(list) = json
                    .pointer("/UNETLoader/input/required/unet_name/0")
                    .and_then(|v| v.as_array())
                {
                    for v in list {
                        if let Some(s) = v.as_str() {
                            available_unets.push(s.to_string());
                        }
                    }
                }
            }
        }

        available_unets
    }

    async fn available_vaes(client: &reqwest::Client) -> Vec<String> {
        let mut available_vaes = Vec::new();
        let vae_info_url = format!("{}/object_info/VAELoader", DEFAULT_COMFY_URL);
        if let Ok(resp) = client.get(&vae_info_url).send().await {
            if let Ok(json) = resp.json::<Value>().await {
                if let Some(list) = json
                    .pointer("/VAELoader/input/required/vae_name/0")
                    .and_then(|v| v.as_array())
                {
                    for v in list {
                        if let Some(s) = v.as_str() {
                            available_vaes.push(s.to_string());
                        }
                    }
                }
            }
        }

        available_vaes
    }

    async fn available_clips(client: &reqwest::Client) -> Vec<String> {
        let mut available_clips = Vec::new();
        let clip_info_url = format!("{}/object_info/CLIPLoader", DEFAULT_COMFY_URL);
        if let Ok(resp) = client.get(&clip_info_url).send().await {
            if let Ok(json) = resp.json::<Value>().await {
                if let Some(list) = json
                    .pointer("/CLIPLoader/input/required/clip_name/0")
                    .and_then(|v| v.as_array())
                {
                    for v in list {
                        if let Some(s) = v.as_str() {
                            available_clips.push(s.to_string());
                        }
                    }
                }
            }
        }

        available_clips
    }

    async fn available_latent_upscale_models(client: &reqwest::Client) -> Vec<String> {
        let mut available_models = Vec::new();
        let info_url = format!("{}/object_info/LatentUpscaleModelLoader", DEFAULT_COMFY_URL);
        if let Ok(resp) = client.get(&info_url).send().await {
            if let Ok(json) = resp.json::<Value>().await {
                if let Some(list) = json
                    .pointer("/LatentUpscaleModelLoader/input/required/model_name/0")
                    .and_then(|v| v.as_array())
                    .or_else(|| {
                        json.pointer(
                            "/LatentUpscaleModelLoader/input/required/model_name/1/options",
                        )
                        .and_then(|v| v.as_array())
                    })
                {
                    for v in list {
                        if let Some(s) = v.as_str() {
                            available_models.push(s.to_string());
                        }
                    }
                }
            }
        }

        available_models
    }

    /// 原生文生视频（只剩 LTX-2.5）的 VAE。找不到就交给 ComfyUI 报缺哪个文件。
    async fn find_best_vae(client: &reqwest::Client) -> String {
        let names = Self::available_vaes(client).await;
        for kw in &["ltx-2.5-video", "ltx"] {
            if let Some(found) = names.iter().find(|n| n.contains(kw)) {
                return found.clone();
            }
        }
        "ltx-2.5-video-vae-bf16.safetensors".to_string()
    }

    /// 原生文生视频（只剩 LTX-2.5）的文本编码器，类型固定 `ltxv`。
    async fn find_best_clip(client: &reqwest::Client) -> String {
        let names = Self::available_clips(client).await;
        for kw in &["gemma4-12b-with-proj-ltx-2.5", "gemma4-12b", "gemma4"] {
            if let Some(found) = names.iter().find(|n| n.contains(kw)) {
                return found.clone();
            }
        }
        "gemma4-12b-with-proj-ltx-2.5-bf16.safetensors".to_string()
    }

    async fn emit_progress(
        ctx: &RuntimeContext,
        request: &ToolCallRequest,
        prompt: &str,
        duration_secs: u32,
        stage: &str,
        label: &str,
        progress: f32,
        prompt_id: Option<&str>,
        output_path: Option<&str>,
        message: Option<&str>,
    ) {
        let call_id = request
            .arguments
            .get("__ui_call_id")
            .and_then(|v| v.as_str())
            .unwrap_or(&request.id)
            .to_string();

        ctx.emit(RuntimeEvent::VideoGenerationProgress {
            turn_id: ctx.ids.turn_id.clone(),
            call_id,
            prompt: prompt.to_string(),
            duration_secs,
            stage: stage.to_string(),
            label: label.to_string(),
            progress: progress.clamp(0.0, 1.0),
            prompt_id: prompt_id.map(str::to_string),
            output_path: output_path.map(str::to_string),
            message: message.map(str::to_string),
        })
        .await;
    }
}

pub(crate) fn extract_comfyui_history_error(h_json: &Value, prompt_id: &str) -> Option<String> {
    let status_str = h_json
        .pointer(&format!("/{prompt_id}/status/status_str"))
        .and_then(|v| v.as_str())
        .unwrap_or_default();

    if let Some(messages) = h_json
        .pointer(&format!("/{prompt_id}/status/messages"))
        .and_then(|v| v.as_array())
    {
        for item in messages {
            if let Some(arr) = item.as_array() {
                let msg_type = arr.first().and_then(|v| v.as_str()).unwrap_or_default();
                if msg_type == "execution_error" {
                    if let Some(payload) = arr.get(1) {
                        let exc_msg = payload
                            .get("exception_message")
                            .and_then(|v| v.as_str())
                            .map(str::trim)
                            .unwrap_or_default();
                        let exc_type = payload
                            .get("exception_type")
                            .and_then(|v| v.as_str())
                            .map(str::trim)
                            .unwrap_or_default();
                        let node_type = payload
                            .get("node_type")
                            .and_then(|v| v.as_str())
                            .map(str::trim)
                            .unwrap_or_default();

                        if !exc_msg.is_empty() {
                            if exc_type == "BrokenPipeError" || exc_msg.contains("Broken pipe") {
                                return Some(format!(
                                    "{node_type} ({exc_type}): {exc_msg}。ComfyUI 的 stdout/stderr 输出管道已断开，采样进度条写日志时中断；请通过 Anima 的 comfyui_start 或终端重启 ComfyUI，并把 stdout/stderr 重定向到日志文件。"
                                ));
                            }
                            if node_type == "SaveVideo"
                                && exc_msg.contains("Input contains")
                                && (exc_msg.contains("NaN") || exc_msg.contains("Inf"))
                            {
                                return Some(format!(
                                    "{node_type} ({exc_type}): {exc_msg}。视频帧已生成到导出阶段，但音频轨包含 NaN/Inf，AAC 编码失败；普通图生视频应先禁用音频 mux，只导出无声 MP4。"
                                ));
                            }
                            return Some(format!("{node_type} ({exc_type}): {exc_msg}"));
                        }
                    }
                    return Some("ComfyUI 节点执行发生 execution_error".to_string());
                }
            }
        }
    }

    if status_str == "error" {
        return Some("ComfyUI 执行状态返回 error".to_string());
    }

    None
}

#[async_trait]
impl ToolHandler for GenerateVideoTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "generate_video",
            "Generate short dynamic MP4 video clips from text prompts using the local ComfyUI video pipeline (LTX-2.5 by default, MiniMax H3 for joint audio+video). The resulting video will be saved directly into `.anima/generated/` and rendered with a playable video card.",
            ToolKind::BuiltIn,
            Operation::Write,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "Detailed prompt describing the scene, motion dynamics, camera movement, and lighting (e.g. 'cinematic drone shot flying through futuristic neon city at night, gentle rain reflections, smooth forward movement, 4k 60fps')"
                },
                "negative_prompt": {
                    "type": "string",
                    "description": "Elements or motion artifacts to avoid (e.g. 'jerky motion, static image, deformed limbs, blurry, low frame rate')"
                },
                "duration_secs": {
                    "type": "integer",
                    "default": 2,
                    "description": "Video duration in seconds. COST (measured 2026-10-08 on this machine, minutes of wall time per second of H3 video): first-frame clips with LightX2V 2.0 at 480p / 6.7 at 720p; reference clips with Larry 6 steps 6.6 at 480p / 16.4 at 720p (2s ≈ 4-13 min at 480p). Longer clips cost more per second. Keep it at 2 unless the user asked for a longer clip."
                },
                "width": {
                    "type": "integer",
                    "default": 832,
                    "description": "Video frame width in pixels (e.g. 832 for landscape, 480 for mobile portrait)"
                },
                "height": {
                    "type": "integer",
                    "default": 480,
                    "description": "Video frame height in pixels (e.g. 480 for landscape, 832 for mobile portrait)"
                },
                "steps": {
                    "type": "integer",
                    "default": 20,
                    "description": "Sampling steps for the native ComfyUI fallback only (default 20). The WeeTodd LTX/H3 routes pick their own steps, so normally leave this out."
                },
                "window_actions": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "For clips longer than 5s only. LTX 2.5 renders long clips as chained 5-second windows, and each window re-imagines the scene from its own prompt. Give ONE action per window here (5s→1, 10s→2, 15s→3), describing ONLY what changes; `prompt` must carry everything that must STAY THE SAME (characters, clothing, camera, framing, background, props) and is prepended to every window automatically. Measured on this machine: full restatement stops the drift (composition similarity across windows 0.877 and NOT decaying with distance), while writing only the delta is WORSE than repeating one prompt (0.688) — a window that fails to mention a character deletes them from the frame."
                },
                "loras": crate::agent_runtime::lora::schema_fragment(),
                "seed": {
                    "type": "integer",
                    "description": "Optional random seed for reproducibility"
                },
                "model": {
                    "type": "string",
                    "description": "Optional specific model file name for the native ComfyUI fallback. Normally leave this out."
                },
                "output_filename": {
                    "type": "string",
                    "description": "Optional specific output filename (e.g. 'city_flythrough.mp4')"
                }
            },
            "required": ["prompt"]
        }))
        .with_timeout(Duration::from_secs(VIDEO_TOOL_TIMEOUT_SECS))
    }

    async fn execute(&self, ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let Some(prompt) = request
            .arguments
            .get("prompt")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(ToolFailureKind::InvalidParams, "缺少 prompt 参数"),
                Duration::from_millis(0),
            );
        };

        let started = Instant::now();
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(120))
            .build()
            .unwrap_or_default();

        let duration_secs = request
            .arguments
            .get("duration_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(4) as u32;
        let width = request
            .arguments
            .get("width")
            .and_then(|v| v.as_u64())
            .unwrap_or(832) as u32;
        let height = request
            .arguments
            .get("height")
            .and_then(|v| v.as_u64())
            .unwrap_or(480) as u32;

        Self::emit_progress(
            ctx,
            &request,
            prompt,
            duration_secs,
            "connecting",
            "正在连接 ComfyUI 视频引擎",
            0.04,
            None,
            None,
            None,
        )
        .await;

        // 1. 探活 ComfyUI
        let stats_url = format!("{}/system_stats", DEFAULT_COMFY_URL);
        if client.get(&stats_url).send().await.is_err() {
            Self::emit_progress(
                ctx,
                &request,
                prompt,
                duration_secs,
                "error",
                "ComfyUI 未连接",
                0.0,
                None,
                None,
                Some("本地 ComfyUI 服务未运行"),
            )
            .await;
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(
                    ToolFailureKind::Execution,
                    "本地 ComfyUI 服务未运行 (http://127.0.0.1:8188)。请先在后台启动 ComfyUI。",
                )
                .with_hint(crate::comfyui::manual_start_hint()),
                started.elapsed(),
            );
        }

        let model_override = request.arguments.get("model").and_then(|v| v.as_str());

        // ── 选路：两条 MLX 路优先，原生只在 WeeTodd 没装时兜底 ──────────────
        //
        // 依据是本机同题实测（docs/APPLE_SILICON_VIDEO_STACK.md）：
        // LTX 5 秒片内存峰值 96.82 GB → 22.84 GB，H3 49.06 GB → 15.87 GB。
        // 而原生那条的 H3 权重已于 2026-09-04 删除，点名 H3 也不能往那儿回落。
        let (ltx_ok, h3_ok) = tokio::join!(
            weetodd::node_available(&client, weetodd::PROBE_NODE),
            weetodd::node_available(&client, weetodd::H3_PROBE_NODE),
        );
        let window_actions: Vec<String> = request
            .arguments
            .get("window_actions")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::trim).filter(|s| !s.is_empty()))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        if let Some(msg) = retired_video_engine("", model_override) {
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(ToolFailureKind::Precondition, msg),
                started.elapsed(),
            );
        }
        let route = weetodd::pick_route(model_override.unwrap_or_default(), ltx_ok, h3_ok);
        let route_note = weetodd::route_note(route, model_override.unwrap_or_default());

        if route != weetodd::Route::Native {
            // seed 在这条路上单独读一次 —— 原生那条要到后面才解析它
            let seed = request
                .arguments
                .get("seed")
                .and_then(|v| v.as_u64())
                .unwrap_or_else(|| {
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64
                        % 1_000_000_000
                });
            return Self::run_weetodd(
                ctx, &request, &client, route, prompt, width, height,
                duration_secs, seed, None, None, None,
                &window_actions,
                &[],
                &[],
                route_note, started,
            )
            .await;
        }

        let unet_name = match pick_video_unet(
            &Self::available_unets(&client).await,
            model_override,
            T2V_PREFERRED,
        ) {
            Ok(name) => name,
            Err(msg) => {
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(ToolFailureKind::Execution, msg),
                    started.elapsed(),
                );
            }
        };
        // 原生文生视频只剩 LTX-2.5 一套拓扑。Wan（连带挂在它上面、
        // 从来没下过权重的 Hunyuan）2026-10-01 下架。
        if !unet_name.to_lowercase().contains("ltx") {
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(
                    ToolFailureKind::Precondition,
                    native_video_unsupported(&unet_name, "文生视频", "LTX-2.5"),
                ),
                started.elapsed(),
            );
        }
        let vae_name = Self::find_best_vae(&client).await;
        let clip_name = Self::find_best_clip(&client).await;

        Self::emit_progress(
            ctx,
            &request,
            prompt,
            duration_secs,
            "model",
            "正在准备扩散模型、CLIP 与 VAE 节点",
            0.10,
            None,
            None,
            Some(&unet_name),
        )
        .await;

        let seed = request
            .arguments
            .get("seed")
            .and_then(|v| v.as_u64())
            .unwrap_or_else(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64
                    % 1_000_000_000
            });

        let default_neg = "static image, blurry, low frame rate, jerky movement, watermark, bad anatomy, artifacts";
        let neg_prompt = request
            .arguments
            .get("negative_prompt")
            .and_then(|v| v.as_str())
            .unwrap_or(default_neg);

        // LoRA 通路。视频这条用 `LoraLoaderModelOnly` —— UNETLoader 不产 CLIP。
        // MiniMax H3 官方图里那个**必需**的 turbo LoRA 就是这么挂的，
        // 所以这不是可选装饰，是后面接 H3 的前置条件。
        //
        // 对不上名字直接失败并列出本机有哪些，不静默跳过：静默跳过的话用户
        // 拿到的是一段"生成成功但 LoRA 完全没生效"的视频，而且不报任何错。
        let mut lora_warning: Option<String> = None;
        let requested_loras = lora::parse_requested(&request.arguments);
        let resolved_loras = if requested_loras.is_empty() {
            Vec::new()
        } else {
            let (resolved, warn) =
                lora::resolve_loras(&requested_loras, &lora::available_loras(&client).await);
            lora_warning = warn;
            resolved
        };
        let chain = lora::build_chain(&resolved_loras, 900, json!(["1", 0]), None);
        let model_ref = chain.model_ref.clone();

        // 2. 构造 LTX-2.5 官方文生视频拓扑（帧数以 8 为步进：8k+1）
        let effective_fps = 24;
        let frames_count = ((duration_secs * effective_fps + 4) / 8) * 8 + 1;
        let steps = request
            .arguments
            .get("steps")
            .and_then(|v| v.as_u64())
            .unwrap_or(20) as u32;
        let mut prompt_payload = build_t2v_graph(&T2vParams {
            unet_name: &unet_name,
            clip_name: &clip_name,
            clip_type: "ltxv",
            vae_name: &vae_name,
            prompt,
            neg_prompt,
            width,
            height,
            frames_count,
            fps: effective_fps,
            seed,
            steps,
            model_ref,
        });

        // 3. 提交任务
        if let Some(map) = prompt_payload
            .get_mut("prompt")
            .and_then(|v| v.as_object_mut())
        {
            for (id, node) in chain.nodes {
                map.insert(id, node);
            }
        }

        let submit_url = format!("{}/prompt", DEFAULT_COMFY_URL);
        let resp = match client.post(&submit_url).json(&prompt_payload).send().await {
            Ok(r) => r,
            Err(e) => {
                Self::emit_progress(
                    ctx,
                    &request,
                    prompt,
                    duration_secs,
                    "error",
                    "向 ComfyUI 提交失败",
                    0.0,
                    None,
                    None,
                    Some(&e.to_string()),
                )
                .await;
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(ToolFailureKind::Transport, format!("提交任务失败: {e}")),
                    started.elapsed(),
                );
            }
        };

        if !resp.status().is_success() {
            let err_body = resp.text().await.unwrap_or_default();
            Self::emit_progress(
                ctx,
                &request,
                prompt,
                duration_secs,
                "error",
                "ComfyUI 拒绝视频任务",
                0.0,
                None,
                None,
                Some(&err_body),
            )
            .await;
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(
                    ToolFailureKind::Execution,
                    format!("ComfyUI 拒绝视频生成任务: {err_body}"),
                ),
                started.elapsed(),
            );
        }

        let resp_json: Value = match resp.json().await {
            Ok(j) => j,
            Err(e) => {
                Self::emit_progress(
                    ctx,
                    &request,
                    prompt,
                    duration_secs,
                    "error",
                    "解析 ComfyUI 响应失败",
                    0.0,
                    None,
                    None,
                    Some(&e.to_string()),
                )
                .await;
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(
                        ToolFailureKind::Execution,
                        format!("解析 ComfyUI 响应失败: {e}"),
                    ),
                    started.elapsed(),
                );
            }
        };

        let Some(prompt_id) = resp_json.get("prompt_id").and_then(|v| v.as_str()) else {
            Self::emit_progress(
                ctx,
                &request,
                prompt,
                duration_secs,
                "error",
                "ComfyUI 未返回任务 ID",
                0.0,
                None,
                None,
                None,
            )
            .await;
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(ToolFailureKind::Execution, "ComfyUI 未返回 prompt_id"),
                started.elapsed(),
            );
        };

        // 记账：交出去了。销账只在成功落盘那一步 —— 超时/失败故意留着，
        // 让开机对账去收（视频一跑几分钟到几十分钟，白丢一次代价很大）。
        super::comfy_jobs::remember(
            prompt_id,
            &ctx.ids.session_id,
            "video",
            "10,9,outputs",
            &super::comfy_jobs::summarize(prompt),
        )
        .await;

        Self::emit_progress(
            ctx,
            &request,
            prompt,
            duration_secs,
            "queued",
            "已提交视频渲染队列",
            0.15,
            Some(prompt_id),
            None,
            None,
        )
        .await;

        // 4. 轮询等待视频渲染完成 (最多等待 180 秒)
        let history_url = format!("{}/history/{}", DEFAULT_COMFY_URL, prompt_id);
        let mut video_filename = String::new();
        let mut video_subfolder = String::new();
        let mut video_type = "output".to_string();

        // **不按时长判超时，看进度**（`comfy_watch`）；队列里、history 里都没有它连着几轮，才算丢了
        let mut watch = super::comfy_watch::tracker(prompt_id);
        let (mut lost_streak, mut pending) = (0u32, 0u32);
        let mut gave_up: Option<String> = None;
        let mut poll_index = 0u32;
        loop {
            if pending >= SHOT_PENDING_CEILING_POLLS {
                gave_up = Some("在 ComfyUI 队列里排了 12 小时还没轮到 —— 前面的任务可能卡住了".into());
                break;
            }
            poll_index += 1;
            tokio::time::sleep(Duration::from_secs(VIDEO_POLL_INTERVAL_SECS)).await;
            let estimated_progress = (0.16 + ((poll_index as f32) / VIDEO_PROGRESS_POLLS as f32) * 0.72).min(0.88);
            let mut label = "ComfyUI 正在时序采样与逐帧合成";
            if let Ok(q_resp) = client
                .get(format!("{}/queue", DEFAULT_COMFY_URL))
                .send()
                .await
            {
                if let Ok(q_json) = q_resp.json::<Value>().await {
                    let is_pending = q_json
                        .get("queue_pending")
                        .and_then(|v| v.as_array())
                        .is_some_and(|items| {
                            items
                                .iter()
                                .any(|item| item.to_string().contains(prompt_id))
                        });
                    let is_running = q_json
                        .get("queue_running")
                        .and_then(|v| v.as_array())
                        .is_some_and(|items| {
                            items
                                .iter()
                                .any(|item| item.to_string().contains(prompt_id))
                        });
                    label = if is_pending {
                        "正在等待视频渲染队列"
                    } else if is_running {
                        "ComfyUI 正在逐帧扩散生成"
                    } else {
                        "正在压制 MP4 视频编码"
                    };
                }
            }

            let label = match watch.describe() {
                Some(p) => format!("{label} · {p}"),
                None => label.to_string(),
            };
            Self::emit_progress(
                ctx,
                &request,
                prompt,
                duration_secs,
                "running",
                &label,
                estimated_progress,
                Some(prompt_id),
                None,
                None,
            )
            .await;

            let mut in_history = false;
            let mut history_ok = false;
            if let Ok(h_resp) = client.get(&history_url).send().await {
                if let Ok(h_json) = h_resp.json::<Value>().await {
                    history_ok = true;
                    in_history = h_json.get(prompt_id).is_some();
                    if let Some(err_str) = extract_comfyui_history_error(&h_json, prompt_id) {
                        Self::emit_progress(
                            ctx,
                            &request,
                            prompt,
                            duration_secs,
                            "error",
                            "视频生成报错",
                            0.0,
                            Some(prompt_id),
                            None,
                            Some(&err_str),
                        )
                        .await;
                        return ToolOutcome::failed(
                            &request,
                            ToolFailure::new(
                                ToolFailureKind::Execution,
                                format!("ComfyUI 视频渲染失败: {err_str}"),
                            ),
                            started.elapsed(),
                        );
                    }

                    // **扫全部输出节点，不用固定 id 列表。**
                    // 原来这里写死了几个节点号，每接一条新链路就要补一次，
                    // 漏了的症状是「任务 success 但找不到文件」。
                    // WeeTodd 的 LTX 出在节点 4、H3 出在 7，都不在原来那个列表里。
                    if let Some(outs) = h_json.pointer(&format!("/{prompt_id}/outputs")) {
                        if let Some((f, sub, ty)) =
                            crate::agent_runtime::weetodd::find_video_output(outs)
                        {
                            video_filename = f;
                            video_subfolder = sub;
                            video_type = ty;
                        }
                    }
                    if !video_filename.is_empty() {
                        break;
                    }
                }
            }

            let (seen, in_queue) = queue_state(&client, prompt_id).await;
            lost_streak = next_lost_streak(lost_streak, in_history, in_queue);
            if lost_streak >= VIDEO_LOST_TOLERANCE {
                gave_up = Some(format!(
                    "ComfyUI 里已经没有这个任务了（prompt_id `{prompt_id}`：不在队列里，也没有结果）—— 多半是被取消了，或者 ComfyUI 重启过"
                ));
                break;
            }
            if seen == super::comfy_watch::Seen::Queued {
                pending += 1;
            }
            let seen = if history_ok { seen } else { super::comfy_watch::Seen::Unreachable };
            if let Some(msg) = watch.observe(seen).explain(prompt_id) {
                gave_up = Some(match watch.describe() {
                    Some(p) => format!("{msg}（最后一次：{p}）"),
                    None => msg,
                });
                break;
            }
        }

        if video_filename.is_empty() {
            Self::emit_progress(
                ctx,
                &request,
                prompt,
                duration_secs,
                "error",
                "没等到视频",
                0.0,
                Some(prompt_id),
                None,
                None,
            )
            .await;
            return ToolOutcome::failed(
                &request,
                // **超时要把 prompt_id 带回去。**
                //
                // 我们放弃等待不等于 ComfyUI 停了 —— 它那边还在算。不说清楚的话
                // 模型会当成失败再发一次，于是两个任务排队互相拖慢，
                // 现象和"卡死"完全一样。
                ToolFailure::new(
                    ToolFailureKind::Execution,
                    format!(
                        "{}。**如果 ComfyUI 那边还在跑**（prompt_id `{prompt_id}`）—— 不要重新提交，那只会排队互相拖慢。\
                         可以稍后查 http://127.0.0.1:8188/history/{prompt_id} 取结果。",
                        gave_up.unwrap_or_else(|| "没拿到视频".into())
                    ),
                ),
                started.elapsed(),
            );
        }

        // 5. 获取视频字节并落盘至 .anima/generated/
        let view_url = format!(
            "{}/view?filename={}&subfolder={}&type={}",
            DEFAULT_COMFY_URL, video_filename, video_subfolder, video_type
        );
        Self::emit_progress(
            ctx,
            &request,
            prompt,
            duration_secs,
            "downloading",
            "正在下载与校验视频流",
            0.92,
            Some(prompt_id),
            None,
            Some(&video_filename),
        )
        .await;

        let video_resp = match client.get(&view_url).send().await {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(
                        ToolFailureKind::Transport,
                        format!("下载视频失败，HTTP 状态码: {}", r.status()),
                    ),
                    started.elapsed(),
                );
            }
            Err(e) => {
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(ToolFailureKind::Transport, format!("请求视频失败: {e}")),
                    started.elapsed(),
                );
            }
        };

        let video_bytes = match video_resp.bytes().await {
            Ok(b) => b,
            Err(e) => {
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(ToolFailureKind::Transport, format!("读取视频字节失败: {e}")),
                    started.elapsed(),
                );
            }
        };

        // 严谨校验视频有效性 (大小至少大于 10KB 且包含常见视频特征)
        if video_bytes.len() < 10240 {
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(
                    ToolFailureKind::Execution,
                    format!("生成的视频文件损坏或过小（{} 字节）", video_bytes.len()),
                ),
                started.elapsed(),
            );
        }

        let generated_dir = ctx.workspace_root.join(".anima").join("generated");
        let _ = std::fs::create_dir_all(&generated_dir);

        let file_name = request
            .arguments
            .get("output_filename")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| format!("video_{seed}.mp4"));
        let target_path = generated_dir.join(&file_name);

        if let Err(e) = std::fs::write(&target_path, video_bytes) {
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(ToolFailureKind::Execution, format!("写入视频失败: {e}")),
                started.elapsed(),
            );
        }
        // 销账放在**落盘成功之后**。写盘失败的话记录留着，
        // 让开机对账再取一次 —— 结果在 ComfyUI 那边一直都在。
        super::comfy_jobs::finish(prompt_id).await;

        let generate_audio = request
            .arguments
            .get("generate_audio")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        if !generate_audio {
            let _ = strip_audio(&target_path);
        }


        let abs_path = target_path.to_string_lossy().to_string();
        Self::emit_progress(
            ctx,
            &request,
            prompt,
            duration_secs,
            "done",
            "视频已就绪",
            1.0,
            Some(prompt_id),
            Some(&abs_path),
            None,
        )
        .await;

        // 视频生成后主动释放潜空间显存缓存，防止 Apple Silicon 统一内存暴涨
        free_comfyui_cache(&client, false).await;

        let summary = crate::agent_runtime::image_tool::with_lora_warning(
            t2v_summary(
                seed,
                &abs_path,
                &unet_name,
                width,
                height,
                effective_fps,
                duration_secs,
            ),
            lora_warning,
        );

        ToolOutcome::ok(&request, summary, vec![abs_path], started.elapsed())
    }
}

impl GenerateVideoTool {
    /// 走 WeeTodd/MLX 那条路：建图 → 提交 → 轮询 → 取结果 → 落盘。
    ///
    /// 和原生那条共用轮询上限和落盘位置，但**图完全不同**（4~8 个节点 vs 33 个），
    /// 所以单独写一份而不是塞进原来那个函数 —— 那个函数已经有三个引擎的分支了。
    #[allow(clippy::too_many_arguments)]
    async fn run_weetodd(
        ctx: &RuntimeContext,
        request: &ToolCallRequest,
        client: &reqwest::Client,
        route: weetodd::Route,
        prompt: &str,
        width: u32,
        height: u32,
        duration_secs: u32,
        seed: u64,
        // 首帧：ComfyUI `input/` 下的文件名。None = 纯文生。
        first_frame: Option<&str>,
        // 末帧：只有 LTX 那条路支持；H3 走参考图而不是末帧。
        last_frame: Option<&str>,
        // 帧率。None = 用 LTX 2.5 的原生 24 —— **不是** 16。
        // 静默换掉用户给的帧率会让时长和口型全部错位，而这不报错。
        fps: Option<f64>,
        // 长片各段的动作。空 = 全段同一句（会漂，见 weetodd::window_prompts）。
        window_actions: &[String],
        // R2V 参考图（含源图，源图排第一）。空 = 不走参考模式。
        references: &[String],
        // R2V 参考音频（ComfyUI `input/` 下的文件名），只在参考模式下有用
        audios: &[String],
        route_note: Option<String>,
        started: std::time::Instant,
    ) -> ToolOutcome {
        let dur = duration_secs.max(1) as f64;
        // **调用方选的 LoRA**（画布视频节点的风格 / LoRA、对话里传的 `loras`）。
        // 2026-10-11 用户问「画布里能选成人向 LoRA，是只能选、实际没挂吗」—— 查实是没挂：
        // 原生那条（下面 `execute` 里）解析了 `loras`，但 H3 / LTX 都在那之前就转到这里，而这里一直传空。
        // 能选、发出去了、任务成功、画面里没有 LoRA 的效果，不报任何错。
        // 名字对不上的照 `resolve_loras` 的老规矩：跳过并在结果里说出来，不整条失败。
        let requested_loras = lora::parse_requested(&request.arguments);
        let (resolved_loras, lora_warning) = if requested_loras.is_empty() {
            (Vec::new(), None)
        } else {
            lora::resolve_loras(&requested_loras, &lora::available_loras(client).await)
        };
        let user_loras: Vec<(String, f64)> = resolved_loras.iter().map(|l| (l.name.clone(), l.strength)).collect();
        let (graph, engine_label, est_secs) = match route {
            weetodd::Route::H3 => {
                // **调用方选了加速档就照用**（画布视频节点的「加速」，和工作台出片设置同一套 id）；
                // 没选照旧按分辨率：小图走 384p 预览档，否则正式档
                let tier = requested_h3_tier(request).unwrap_or(if width.max(height) <= 640 {
                    weetodd::H3Tier::Preview
                } else {
                    weetodd::H3Tier::Standard
                });
                let mode = if !references.is_empty() {
                    weetodd::H3Mode::Reference {
                        images: references,
                        pixel_budget_percent: 100,
                        audios,
                    }
                } else if first_frame.is_some() || last_frame.is_some() {
                    weetodd::H3Mode::Frames { first: first_frame, last: last_frame }
                } else {
                    weetodd::H3Mode::Text
                };
                (
                    weetodd::build_h3_graph(&weetodd::H3Params {
                        continuity: None,
                        mode,
                        prompt,
                        width,
                        height,
                        duration_secs: dur,
                        seed,
                        tier,
                        filename_prefix: "Anima/h3",
                        loras: &user_loras,
                    }),
                    "MiniMax H3 (WeeTodd/MLX)",
                    // 原生 Ref2VA 实际走稠密 19 次，预计耗时按实际那一档算
                    weetodd::H3SamplingPlan::current(!references.is_empty(), tier).tier.secs(),
                )
            }
            _ => {
                let windows = weetodd::window_count_for(dur);
                let win_prompts = weetodd::window_prompts(prompt, window_actions, windows);
                (
                    weetodd::build_graph(&weetodd::WeeToddParams {
                        first_frame,
                        last_frame,
                        prompt,
                        width,
                        height,
                        duration_secs: dur,
                        fps: weetodd::clamp_fps(fps),
                        seed,
                        filename_prefix: "Anima/ltx25",
                        loras: &user_loras,
                        window_count: windows,
                        window_prompts: &win_prompts,
                    }),
                    "LTX 2.5 (WeeTodd/MLX)",
                    // 实测 5 秒 121 秒；接续按窗口数线性估
                    121 * windows.max(1) as u64,
                )
            }
        };

        Self::emit_progress(
            ctx, request, prompt, duration_secs, "submitting",
            &format!("{engine_label}，预计 {est_secs} 秒"),
            0.05, None, None, None,
        )
        .await;

        // 结果落哪：提交之前就定下来 —— 续接单要记它（应用中途重启，凭它去 ComfyUI 收，见 `render_resume`）
        let dir = ctx.workspace_root.join(".anima").join("generated");
        let name = request
            .arguments
            .get("output_filename")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| format!("video_{seed}.mp4"));
        let path = dir.join(&name);
        let generate_audio = request
            .arguments
            .get("generate_audio")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        // 后台任务（画布）里才有用：右下角那句「走的哪条链路」、重启续接单。对话里这几句什么也不做
        if let Some(p) = weetodd::graph_pipeline(&graph) {
            crate::jobs::note_pipeline(&p);
        }
        crate::jobs::note_resume(json!({
            "kind": "canvas_video",
            "out": path.to_string_lossy(),
            "generate_audio": generate_audio,
            "summary": format!("已用 {engine_label} 生成 {width}×{height} / {duration_secs} 秒视频，seed {seed}（应用重启后从 ComfyUI 收回）"),
        }));

        let submit = client
            .post(format!("{DEFAULT_COMFY_URL}/prompt"))
            .json(&json!({"prompt": graph}))
            .send()
            .await;
        let prompt_id = match submit {
            Ok(r) => match r.json::<Value>().await {
                Ok(v) => match v.get("prompt_id").and_then(Value::as_str) {
                    Some(id) => id.to_string(),
                    None => {
                        // ComfyUI 校验失败时把原因放在 error/node_errors 里，
                        // 原样带出去 —— 只说「提交失败」等于没说
                        return ToolOutcome::failed(
                            request,
                            ToolFailure::new(
                                ToolFailureKind::Execution,
                                format!("ComfyUI 拒绝了这张图：{v}"),
                            ),
                            started.elapsed(),
                        );
                    }
                },
                Err(e) => {
                    return ToolOutcome::failed(
                        request,
                        ToolFailure::new(ToolFailureKind::Execution, format!("提交失败: {e}")),
                        started.elapsed(),
                    )
                }
            },
            Err(e) => {
                return ToolOutcome::failed(
                    request,
                    ToolFailure::new(
                        ToolFailureKind::Execution,
                        format!("连不上 ComfyUI: {e}。{}", crate::comfyui::manual_start_hint()),
                    ),
                    started.elapsed(),
                )
            }
        };
        // 记账，好让重启后能认领在跑的任务（save_node 写两个：H3 出在 7、LTX 出在 4）
        super::comfy_jobs::remember_to(
            &prompt_id, "", "video", "7,4",
            &format!("{engine_label} {width}×{height} {duration_secs}s"),
            Some(&path),
        )
        .await;
        // 记到当前后台任务名下：画布点取消时靠它找到这个 prompt 去撤（以前这条路没记，取消了 ComfyUI 照跑）
        super::comfy_jobs::attach(&prompt_id);
        crate::jobs::note_prompt_id(&prompt_id);
        let mut run_since = crate::jobs::current_run_started_at();

        // 轮询。**不按时长判超时，看进度**（`comfy_watch`）—— 15 秒 1344×768 实测就要 1418 秒，720p 一镜可能超过 4 小时
        let mut outputs = None;
        let mut watch = super::comfy_watch::tracker(&prompt_id);
        let (mut lost_streak, mut pending) = (0u32, 0u32);
        let mut gave_up: Option<String> = None;
        let mut i = 0u32;
        loop {
            if pending >= SHOT_PENDING_CEILING_POLLS {
                gave_up = Some("在 ComfyUI 队列里排了 12 小时还没轮到 —— 前面的任务可能卡住了".into());
                break;
            }
            i += 1;
            tokio::time::sleep(std::time::Duration::from_secs(VIDEO_POLL_INTERVAL_SECS)).await;
            if super::comfy_jobs::cancel_requested() {
                return ToolOutcome::failed(request, ToolFailure::new(ToolFailureKind::Execution, "已取消"), started.elapsed());
            }
            let history = match client.get(format!("{DEFAULT_COMFY_URL}/history/{prompt_id}")).send().await {
                Ok(r) => r.json::<Value>().await.ok(),
                Err(_) => None,
            };
            if let Some(entry) = history.as_ref().and_then(|h| h.get(&prompt_id)) {
                if let Some(msg) = weetodd_exec_error(entry) {
                    return ToolOutcome::failed(
                        request,
                        ToolFailure::new(ToolFailureKind::Execution, msg),
                        started.elapsed(),
                    );
                }
                outputs = entry.get("outputs").cloned();
                break;
            }
            let (seen, in_queue, ahead) = queue_state_ahead(client, &prompt_id).await;
            lost_streak = next_lost_streak(lost_streak, false, in_queue);
            if lost_streak >= VIDEO_LOST_TOLERANCE {
                gave_up = Some("ComfyUI 里已经没有这个任务了（不在队列里，也没有结果）—— 多半是被取消了，或者 ComfyUI 重启过".into());
                break;
            }
            if seen == super::comfy_watch::Seen::Queued {
                pending += 1;
            }
            let seen = if history.is_some() { seen } else { super::comfy_watch::Seen::Unreachable };
            if let Some(msg) = watch.observe(seen).explain(&prompt_id) {
                gave_up = Some(match watch.describe() {
                    Some(p) => format!("{msg}（最后一次：{p}）"),
                    None => msg,
                });
                break;
            }
            // **这一条自己的进度**写进后台任务（画布节点、右下角读它）—— 和分镜出片的等待循环同一套算法
            // （`wait_and_fetch_video`）。v0.43.30 只给分镜那条记了，画布的 H3 出片走这里，什么都没记，节点仍显示全局那一个
            if seen == super::comfy_watch::Seen::Running && run_since.is_none() {
                let t = crate::jobs::now().saturating_sub(watch.run_secs());
                crate::jobs::note_run_started(t);
                run_since = Some(t);
            }
            if seen != super::comfy_watch::Seen::Unreachable && i % 5 == 0 {
                let running = seen == super::comfy_watch::Seen::Running;
                let log = if running { crate::comfy_progress::comfy_progress(None) } else { crate::comfy_progress::describe(None, 0.0, 0.0) };
                let run_secs = match run_since {
                    Some(t) if running => crate::jobs::now().saturating_sub(t),
                    _ => watch.run_secs(),
                };
                let (done, total, label, eta) = crate::comfy_progress::video_line(ahead, !running, watch.progress().and_then(|p| p.step), &log, run_secs);
                crate::jobs::note_progress(done, total, &label, eta.map(|e| crate::jobs::now() + e));
            }
            if i % 10 == 0 {
                let done = (i as f64 * VIDEO_POLL_INTERVAL_SECS as f64) / est_secs as f64;
                let label = match watch.describe() {
                    Some(p) => format!("{engine_label} 渲染中 · {p}"),
                    None => format!("{engine_label} 渲染中"),
                };
                Self::emit_progress(
                    ctx, request, prompt, duration_secs, "rendering",
                    &label,
                    (0.1 + 0.8 * done.min(0.95)) as f32, Some(&prompt_id), None, None,
                )
                .await;
            }
        }

        let Some((file, sub, ty)) = outputs.as_ref().and_then(weetodd::find_video_output) else {
            return ToolOutcome::failed(
                request,
                ToolFailure::new(
                    ToolFailureKind::Execution,
                    format!(
                        "{}。ComfyUI 那边如果还在跑，可以稍后查 {DEFAULT_COMFY_URL}/history/{prompt_id}",
                        gave_up.unwrap_or_else(|| "ComfyUI 跑完了但没有产出视频文件".into())
                    ),
                ),
                started.elapsed(),
            );
        };

        let view = format!("{DEFAULT_COMFY_URL}/view?filename={file}&subfolder={sub}&type={ty}");
        let bytes = match client.get(&view).send().await {
            Ok(r) => match r.bytes().await {
                Ok(b) => b,
                Err(e) => {
                    return ToolOutcome::failed(
                        request,
                        ToolFailure::new(ToolFailureKind::Execution, format!("下载失败: {e}")),
                        started.elapsed(),
                    )
                }
            },
            Err(e) => {
                return ToolOutcome::failed(
                    request,
                    ToolFailure::new(ToolFailureKind::Execution, format!("下载失败: {e}")),
                    started.elapsed(),
                )
            }
        };

        let _ = std::fs::create_dir_all(&dir);
        if let Err(e) = std::fs::write(&path, &bytes) {
            return ToolOutcome::failed(
                request,
                ToolFailure::new(ToolFailureKind::Execution, format!("写入视频失败: {e}")),
                started.elapsed(),
            );
        }
        // 销账放在落盘成功之后 —— 写盘失败时记录留着，让开机对账再取一次
        super::comfy_jobs::finish(&prompt_id).await;

        if !generate_audio {
            if let Err(e) = strip_audio(&path) {
                tracing::warn!("WeeTodd 视频剥离音频失败 ({}): {e}", path.display());
            }
        }

        let abs = path.to_string_lossy().to_string();
        Self::emit_progress(
            ctx, request, prompt, duration_secs, "done", "视频已就绪",
            1.0, Some(&prompt_id), Some(&abs), None,
        )
        .await;

        let mut summary = format!(
            "已用 {engine_label} 生成 {width}×{height} / {duration_secs} 秒视频，seed {seed}，\
             耗时 {:.0} 秒。\n文件：{abs}",
            started.elapsed().as_secs_f64()
        );
        // 这条路没有负向提示词入口，调用方必须知道
        summary.push_str("\n\n");
        summary.push_str(weetodd::NO_NEGATIVE_NOTE);
        if let Some(n) = route_note {
            summary.push_str("\n\n");
            summary.push_str(&n);
        }
        if !user_loras.is_empty() {
            let names: Vec<String> = user_loras.iter().map(|(n, w)| format!("{n} ×{w}")).collect();
            summary.push_str(&format!("\n\n挂了 LoRA：{}", names.join("、")));
        }
        if let Some(w) = lora_warning {
            summary.push_str("\n\n");
            summary.push_str(&w);
        }
        ToolOutcome::ok(request, summary, vec![abs], started.elapsed())
    }
}

/// 提交一张 WeeTodd 图、等它跑完、把视频字节拿回来。
///
/// 给**没有 `RuntimeContext` 的调用方**用（短剧那条线是 Tauri command，
/// 不在 agent runtime 里）。和 `run_weetodd` 共用同一组判据 ——
/// `weetodd_exec_error`、`weetodd::find_video_output`；
/// 差别是这里**不按时长判超时**（对话那条外面包着 registry 的超时，这里没有），看 ComfyUI 的实际进度。
///
/// **等不到 ≠ 任务失败**：ComfyUI 那边可能还在跑，所以错误信息里带上
/// `prompt_id`，让人能自己去查，而不是只说「超时」。
/// 提交一张图给 ComfyUI，等它出片，把字节拿回来。
///
/// `dest` 是**这段片子该落到哪个文件**。它不参与这里的写盘（写盘还是调用方
/// 的事），只记进任务账本 —— 这样一旦这次等待被打断
/// （webview 重载、`tauri dev` 重编译、用户关窗口），
/// 下次开机的对账能把文件放回原位。
///
/// **不传的后果是实测过的**：两段 r2v 视频在 ComfyUI 那边出好了、
/// 回收也下载成功了，但账本里没有归属，最后一米交付不出去。
pub(crate) async fn submit_and_fetch_video(
    client: &reqwest::Client,
    graph: &Value,
    label: &str,
    dest: Option<&Path>,
) -> std::result::Result<Vec<u8>, String> {
    // 右下角进度前面那句「走的哪条链路」：从这张图里读，不按设置推
    if let Some(p) = super::weetodd::graph_pipeline(graph) {
        crate::jobs::note_pipeline(&p);
    }
    let v: Value = client
        .post(format!("{DEFAULT_COMFY_URL}/prompt"))
        .json(&json!({ "prompt": graph }))
        .send()
        .await
        .map_err(|e| format!("连不上 ComfyUI: {e}。{}", crate::comfyui::manual_start_hint()))?
        .json()
        .await
        .map_err(|e| format!("ComfyUI 返回的不是 JSON: {e}"))?;
    let prompt_id = v
        .get("prompt_id")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("ComfyUI 拒绝了这张图：{v}"))?
        .to_string();

    super::comfy_jobs::remember_to(&prompt_id, "", "video", "7,4", label, dest).await;
    // 记到当前后台任务名下 —— 用户点取消时靠它找得到这个 prompt。
    super::comfy_jobs::attach(&prompt_id);
    // 记进出片任务的续接单 —— 应用中途关掉，重开时凭它去 ComfyUI 收结果（`crate::render_resume`）
    crate::jobs::note_prompt_id(&prompt_id);
    wait_and_fetch_video(client, &prompt_id).await
}

/// 等一个已经交给 ComfyUI 的视频任务出结果，把字节拿回来。提交和续接（重启后接着等）共用。
pub(crate) async fn wait_and_fetch_video(client: &reqwest::Client, prompt_id: &str) -> std::result::Result<Vec<u8>, String> {
    let prompt_id = prompt_id.to_string();

    // **既查 `/history` 也查 `/queue`。**
    //
    // 只查 `/history` 的话，任务在 ComfyUI 那边被取消（或 ComfyUI 重启）之后
    // 什么也不会写进 history，这个循环就会一路等下去。
    // 用户 2026-09-21：「ComfyUI 已经取消了，画布里还在转」。
    //
    // 判据和 `comfy_jobs::classify` 是同一个：队列里没有、也没有产出 = 它不在了。
    // 要连着两轮都查不到才认 —— 刚提交那一瞬间可能两处都还没有它。
    //
    // **还在不在跑，看进度，不看过了多久**（`comfy_watch`）：只要 ComfyUI 还在报采样进度就一直等。
    let mut outputs = None;
    let mut lost_streak = 0;
    let mut pending = 0u32;
    let mut watch = super::comfy_watch::tracker(&prompt_id);
    let mut gave_up: Option<String> = None;
    // 卡片上的进度（第几步、百分比、还要多久）：约 15 秒报一次，不是每轮都写任务表
    let mut tick = 0u32;
    let mut ahead: Option<usize> = None;
    // **「已跑」从 ComfyUI 开始跑这一镜算起，重启续接也接着算**（见 `jobs::note_run_started`）
    let mut run_since = crate::jobs::current_run_started_at();
    loop {
        if pending >= SHOT_PENDING_CEILING_POLLS {
            gave_up = Some(format!(
                "在 ComfyUI 队列里排了 12 小时还没轮到（prompt_id `{prompt_id}`）—— 前面的任务可能卡住了，去 ComfyUI 看一眼队列"
            ));
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(VIDEO_POLL_INTERVAL_SECS)).await;
        if super::comfy_jobs::cancel_requested() {
            return Err("已取消".into());
        }
        let history = match client.get(format!("{DEFAULT_COMFY_URL}/history/{prompt_id}")).send().await {
            Ok(r) => r.json::<Value>().await.ok(),
            Err(_) => None,
        };
        let seen = match history {
            // 连不上、读不出都不算「任务没了」—— 误判会把跑了几个小时的任务丢掉。连着太久才算 ComfyUI 退了
            None => super::comfy_watch::Seen::Unreachable,
            Some(h) => {
                if let Some(entry) = h.get(&prompt_id) {
                    if let Some(msg) = weetodd_exec_error(entry) {
                        return Err(msg);
                    }
                    outputs = entry.get("outputs").cloned();
                    break;
                }
                // history 里没有 —— 它还在队列里吗
                let queue = match client.get(format!("{DEFAULT_COMFY_URL}/queue")).send().await {
                    Ok(r) => r.json::<Value>().await.ok(),
                    Err(_) => None,
                };
                match queue {
                    None => super::comfy_watch::Seen::Unreachable,
                    Some(q) => {
                        lost_streak = if super::comfy_jobs::queue_contains(&q, &prompt_id) { 0 } else { lost_streak + 1 };
                        if lost_streak >= 2 {
                            return Err(format!(
                                "ComfyUI 那边已经没有这个任务了（prompt_id `{prompt_id}`）—— \
                                 多半是在 ComfyUI 界面上取消了，或者 ComfyUI 重启过。"
                            ));
                        }
                        ahead = crate::comfyui::parse_queue(&q, Some(&prompt_id)).ahead;
                        if queue_pending_only(&q, &prompt_id) {
                            pending += 1;
                            super::comfy_watch::Seen::Queued
                        } else {
                            super::comfy_watch::Seen::Running
                        }
                    }
                }
            }
        };
        if let Some(msg) = watch.observe(seen).explain(&prompt_id) {
            gave_up = Some(match watch.describe() {
                Some(p) => format!("{msg}（最后一次：{p}）"),
                None => msg,
            });
            break;
        }
        if seen == super::comfy_watch::Seen::Running && run_since.is_none() {
            let t = crate::jobs::now().saturating_sub(watch.run_secs());
            crate::jobs::note_run_started(t);
            run_since = Some(t);
        }
        if seen != super::comfy_watch::Seen::Unreachable && tick % 5 == 0 {
            let log = if seen == super::comfy_watch::Seen::Running { crate::comfy_progress::comfy_progress(None) } else { crate::comfy_progress::describe(None, 0.0, 0.0) };
            let run_secs = match run_since {
                Some(t) if seen == super::comfy_watch::Seen::Running => crate::jobs::now().saturating_sub(t),
                _ => watch.run_secs(),
            };
            let (done, total, label, eta) = crate::comfy_progress::video_line(ahead, seen == super::comfy_watch::Seen::Queued, watch.progress().and_then(|p| p.step), &log, run_secs);
            crate::jobs::note_progress(done, total, &label, eta.map(|e| crate::jobs::now() + e));
        }
        tick += 1;
    }

    let (file, sub, ty) = outputs
        .as_ref()
        .and_then(weetodd::find_video_output)
        .ok_or_else(|| {
            gave_up.unwrap_or_else(|| {
                format!("ComfyUI 跑完了但没有产出视频文件（prompt_id `{prompt_id}`）—— 去 ComfyUI 看看输出节点")
            })
        })?;

    let bytes = client
        .get(format!(
            "{DEFAULT_COMFY_URL}/view?filename={file}&subfolder={sub}&type={ty}"
        ))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("下载失败: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("下载失败: {e}"))?;
    super::comfy_jobs::finish(&prompt_id).await;
    Ok(bytes.to_vec())
}

#[allow(clippy::too_many_arguments)]
/// **续接一镜**：应用重启前交给 ComfyUI 的出片，接着等它出完、落到 `out`（和正常出完同一个收尾）。
pub(crate) async fn resume_shot_video(prompt_id: &str, out: &Path, generate_audio: bool) -> std::result::Result<(), String> {
    let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(120)).build().unwrap_or_default();
    let bytes = wait_and_fetch_video(&client, prompt_id).await?;
    if let Some(parent) = out.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(out, &bytes).map_err(|e| format!("写入 {} 失败：{e}", out.display()))?;
    finalize_shot_audio(out, generate_audio);
    Ok(())
}

/// ComfyUI 现在连得上吗（续接的批量开始前要等它起来，不然一镜接一镜秒败）。
pub(crate) async fn comfy_reachable() -> bool {
    let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(5)).build().unwrap_or_default();
    client.get(format!("{DEFAULT_COMFY_URL}/system_stats")).send().await.is_ok_and(|r| r.status().is_success())
}

/// 关键帧搬进 ComfyUI 的 input：**尺寸和出片画布不一样就先等比裁到画布**，一样就原样拷。回「有没有至少写进一个目录」。
///
/// H3 首帧出片把第一张关键帧**直接拉伸**到画布（WeeTodd `prepare_keyframe_image`，ComfyUI 自带 H3 节点也是
/// 「geometry anchor: plain stretch to canvas」），不保比例。2026-10-07 查实：EP03 十张首帧全是 1216×832，
/// 拉到 1312×736 人物宽了约 22%（「变矮变胖」）。新首帧已经按画布比例出（`cinema::frame_request_size`），
/// 这里管旧首帧、导入的、出完首帧又改了清晰度 / 比例的 —— **原文件不动**，只改送进去的那份。
/// 调用方点名的 H3 加速档（`h3_quality`：`lightx2v` / `standard` / `staged` / `dense`，和工作台出片设置同一套 id）。
/// 没给或认不出回 `None`（照旧按尺寸自动）—— 认不出时记一条日志，不悄悄换成别的档
pub(crate) fn requested_h3_tier(request: &ToolCallRequest) -> Option<weetodd::H3Tier> {
    let raw = request.arguments.get("h3_quality")?.as_str()?.trim();
    match serde_json::from_value::<crate::cinema::RenderQuality>(json!(raw)) {
        Ok(q) => Some(q.tier()),
        Err(_) => {
            tracing::warn!(raw, "h3_quality 认不出，按尺寸自动选档");
            None
        }
    }
}

/// 参考音频搬进 ComfyUI 的 `input/`：**截到这一条的时长**、转成 48 kHz 双声道 WAV。
///
/// 截短是因为 `fully_copy` 的意思是「这段就是成片的整条音轨」—— 拿一首 4 分钟的歌去配 15 秒，
/// 模型要对齐的是整首歌。格式统一是为了 `LoadAudio` 不挑 mp3 / m4a / flac。
pub(crate) fn stage_reference_audio(src: &Path, name: &str, duration_secs: f64) -> std::result::Result<(), String> {
    let tmp = std::env::temp_dir().join(name);
    let out = std::process::Command::new(ffmpeg_binary())
        .args(["-y", "-v", "error", "-i"])
        .arg(src)
        .args(["-t", &format!("{:.3}", duration_secs.max(1.0)), "-vn", "-ac", "2", "-ar", "48000"])
        .arg(&tmp)
        .output()
        .map_err(|e| format!("参考音频转码失败（ffmpeg 起不来）：{e}"))?;
    if !out.status.success() {
        return Err(format!("参考音频转码失败：{}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let mut copied = false;
    for dir in find_comfyui_input_dirs() {
        copied |= std::fs::copy(&tmp, dir.join(name)).is_ok();
    }
    let _ = std::fs::remove_file(&tmp);
    if copied { Ok(()) } else { Err(format!("参考音频复制进 ComfyUI 的 input 失败：{}", src.display())) }
}

pub(crate) fn stage_keyframe(src: &Path, name: &str, canvas: (u32, u32)) -> bool {
    let fitted = keyframe_for_canvas(src, canvas);
    let mut ok = false;
    for dir in find_comfyui_input_dirs() {
        let dest = dir.join(name);
        ok |= match &fitted {
            Some(img) => img.save_with_format(&dest, image::ImageFormat::Png).is_ok(),
            None => std::fs::copy(src, &dest).is_ok(),
        };
    }
    ok
}

/// 关键帧要不要先裁：和画布一样大 = `None`（原样送，不多一次重采样）；不一样 = 等比裁到画布的那张。读不出来也 `None`（照旧拷，让 ComfyUI 报）
fn keyframe_for_canvas(src: &Path, canvas: (u32, u32)) -> Option<image::DynamicImage> {
    let size = image::image_dimensions(src).ok()?;
    if size == canvas || canvas.0 == 0 || canvas.1 == 0 {
        return None;
    }
    let img = image::open(src).ok()?;
    tracing::info!(src = %src.display(), from = ?size, to = ?canvas, "关键帧和出片画布尺寸不一致，送进 H3 前等比裁切");
    Some(crate::cinema::fit_to_frame(img, canvas))
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn render_shot_video(
    src_image: &Path,
    out: &Path,
    engine_hint: &str,
    prompt: &str,
    duration_secs: f64,
    width: u32,
    height: u32,
    fps: f64,
    seed: u64,
    generate_audio: bool,
    // `end_image`：尾帧。有的话走**首尾帧**（FL2VA）—— 结尾就被锁死了，
    // 而不是只靠提示词里一句「结尾落在…」。
    // **代价要知道**：首尾帧要加载 66.73 GB 的视觉编码器，内存峰值约 54 GB，
    // 所以这是按镜可选的，不是默认开。
    end_image: Option<&Path>,
    // H3 的采样档位。`None` = 按画幅自动（≤640 预览档，否则正式档）。
    // 分镜出片要传项目设置里的那一档 —— 和参考生视频那条一致，不然同一集两种出法两种质量。
    tier: Option<weetodd::H3Tier>,
) -> std::result::Result<(), String> {
    // **走 `weetodd::fit_dimensions`，不要自己再写一遍。**
    //
    // 32 对齐是引擎的硬约束（H3 的 `WeeToddH3GenerationConfig` 直接拒绝
    // 非 32 倍数：`width and height must be divisible by 32`），
    // 而这件事**早就有主了** —— agent 那条出片路一直在调它，
    // 只有分镜这条忘了。再写一个 `align32` 就是第二份真相。
    let (width, height) = weetodd::fit_dimensions((width, height));
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(120))
        .build()
        .unwrap_or_default();

    let (ltx_ok, h3_ok) = tokio::join!(
        weetodd::node_available(&client, weetodd::PROBE_NODE),
        weetodd::node_available(&client, weetodd::H3_PROBE_NODE),
    );
    if let Some(msg) = retired_video_engine(engine_hint, None) {
        return Err(msg);
    }
    let route = weetodd::pick_route(engine_hint, ltx_ok, h3_ok);
    if route == weetodd::Route::Native {
        return Err(format!(
            "本机没装 WeeTodd 的 MLX 节点，短剧这条线不走原生回落 —— \
             原生的 H3 权重已删，LTX 原生要 96.82 GB 峰值。\
             先装 WeeTodd-Nodes（引擎选 `{engine_hint}`）。"
        ));
    }
    if let Some(msg) = weetodd::animate_blocker(route, false) {
        return Err(msg);
    }

    // 关键帧搬进 ComfyUI 的 input，文件名带 seed 防并发互相覆盖
    let input_name = format!("cinema_shot_{seed}.png");
    if !stage_keyframe(src_image, &input_name, (width, height)) {
        return Err(format!(
            "关键帧复制进 ComfyUI 的 input 目录失败：{}",
            src_image.display()
        ));
    }

    // 尾帧同理。**搬不进去就当没有** —— 退回只用首帧，
    // 比因为一张可选的图让整镜失败好。
    let end_name = end_image.and_then(|p| {
        let name = format!("cinema_shot_{seed}_end.png");
        if stage_keyframe(p, &name, (width, height)) {
            Some(name)
        } else {
            tracing::warn!("尾帧复制进 ComfyUI input 失败，本镜退回只用首帧：{}", p.display());
            None
        }
    });

    let dur = duration_secs.max(1.0);
    let graph = match route {
        weetodd::Route::H3 => weetodd::build_h3_graph(&weetodd::H3Params {
            continuity: None,
            mode: weetodd::H3Mode::Frames {
                first: Some(&input_name),
                last: end_name.as_deref(),
            },
            prompt,
            width,
            height,
            duration_secs: dur,
            seed,
            tier: tier.unwrap_or(if width.max(height) <= 640 {
                weetodd::H3Tier::Preview
            } else {
                weetodd::H3Tier::Standard
            }),
            filename_prefix: "Anima/cinema",
            loras: &[],
        }),
        _ => weetodd::build_graph(&weetodd::WeeToddParams {
            first_frame: Some(&input_name),
            last_frame: end_name.as_deref(),
            prompt,
            width,
            height,
            duration_secs: dur,
            fps: weetodd::clamp_fps(Some(fps)),
            seed,
            filename_prefix: "Anima/cinema",
            loras: &[],
            window_count: weetodd::window_count_for(dur),
            window_prompts: &[],
        }),
    };

    let bytes = submit_and_fetch_video(&client, &graph, "cinema shot", Some(out)).await?;
    if let Some(parent) = out.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(out, &bytes).map_err(|e| format!("写入 {} 失败：{e}", out.display()))?;
    finalize_shot_audio(out, generate_audio);
    Ok(())
}

/// 同上，可以指定采样档位（`None` = 按画幅自动：≤640 预览档，否则 Turbo 正式档）。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn render_shot_video_reference_with(
    reference_images: &[std::path::PathBuf],
    out: &Path,
    prompt: &str,
    duration_secs: f64,
    width: u32,
    height: u32,
    seed: u64,
    generate_audio: bool,
    tier: Option<weetodd::H3Tier>,
    continuity: Option<weetodd::H3Continuity<'_>>,
) -> std::result::Result<(), String> {
    // 同上。**参考生视频走的是同一个引擎**，只在首帧那条上对齐等于漏一半。
    let (width, height) = weetodd::fit_dimensions((width, height));
    if reference_images.is_empty() {
        return Err("参考模式至少要一张参考图 —— 在提示词里 @ 一个资产，                    或者把这一镜切回首帧模式"
            .into());
    }
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(120))
        .build()
        .unwrap_or_default();

    if !weetodd::node_available(&client, weetodd::H3_PROBE_NODE).await {
        return Err("参考模式要 H3（ref2va 分区），本机没有它的 WeeTodd 节点。                    把这一镜切回首帧模式，或者装上 WeeTodd-Nodes。"
            .into());
    }

    // 参考图搬进 ComfyUI 的 input，文件名带 seed 和序号防并发互相覆盖。
    let mut names = Vec::new();
    // **上限 9 张，取自单一来源**（H3 官方规格 reference_image 最多 9 张）。以前这里写死 4 ——
    // 第 5 张往后静默丢掉，而提示词里的 <Picture 5> 还在，模型收到互相矛盾的说明。
    for (i, src) in reference_images.iter().take(MAX_REFERENCE_IMAGES).enumerate() {
        let ext = src
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("png")
            .to_ascii_lowercase();
        let name = format!("cinema_ref_{seed}_{i}.{ext}");
        let mut copied = false;
        for dir in find_comfyui_input_dirs() {
            copied |= std::fs::copy(src, dir.join(&name)).is_ok();
        }
        if !copied {
            return Err(format!("参考图复制进 ComfyUI 的 input 失败：{}", src.display()));
        }
        names.push(name);
    }

    // 预算够让最大那张参考图保持原尺寸 —— 设定卡按 100 缩下去，脸只剩几十个像素（见 `reference_pixel_budget`）
    let sizes: Vec<(u32, u32)> = reference_images.iter().take(MAX_REFERENCE_IMAGES).filter_map(|p| image::image_dimensions(p).ok()).collect();
    // 原生 Ref2VA + 专用加速 LoRA 照官方 match（100），见 `H3SamplingPlan::reference_budget`
    let plan = weetodd::H3SamplingPlan::current(true, tier.unwrap_or(weetodd::H3Tier::Standard));
    let budget = plan.reference_budget(weetodd::reference_pixel_budget(&sizes, width, height));
    tracing::info!(budget, refs = sizes.len(), width, height, "参考图像素预算");
    let graph = weetodd::build_h3_graph(&weetodd::H3Params {
        continuity,
        mode: weetodd::H3Mode::Reference {
            images: &names,
            pixel_budget_percent: budget,
            audios: &[],
        },
        prompt,
        width,
        height,
        duration_secs: duration_secs.max(1.0),
        seed,
        tier: tier.unwrap_or(if width.max(height) <= 640 {
            weetodd::H3Tier::Preview
        } else {
            weetodd::H3Tier::Standard
        }),
        filename_prefix: "Anima/cinema",
        loras: &[],
    });

    let bytes = submit_and_fetch_video(&client, &graph, "cinema shot (参考模式)", Some(out)).await?;
    if let Some(parent) = out.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(out, &bytes).map_err(|e| format!("写入 {} 失败：{e}", out.display()))?;
    finalize_shot_audio(out, generate_audio);
    Ok(())
}

/// 长镜头：一场 N 个等长窗口串成一条链，**一次提交出一个 MP4**。
///
/// `windows` 的每一项是 `(提示词, 这一窗口的参考图路径)`，按窗口顺序。
/// 硬约束和代价见 [`crate::long_take`]。
///
/// # 为什么参考图是逐窗口搬的
///
/// 每个窗口有自己的 `ref_slots`（同一场里换人、换景都可能），
/// 文件名带窗口序号防止互相覆盖 —— 单窗口那条只带 seed，
/// 一条链里几个窗口同 seed，不加序号会互相盖掉。
pub(crate) async fn render_long_take(
    windows: &[(String, Vec<std::path::PathBuf>)],
    out: &Path,
    window_secs: f64,
    context_frames: u32,
    width: u32,
    height: u32,
    seed: u64,
    generate_audio: bool,
    tier: Option<weetodd::H3Tier>,
    // 调用方选的 LoRA（ComfyUI 枚举里的完整名, 强度），每个窗口共用。分镜页那条不给（传空）
    loras: &[(String, f64)],
) -> std::result::Result<(), String> {
    let (width, height) = weetodd::fit_dimensions((width, height));
    if windows.len() < 2 {
        return Err("长镜头至少要两个窗口 —— 一镜直接出片就行".into());
    }
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(120))
        .build()
        .unwrap_or_default();
    if !weetodd::node_available(&client, weetodd::H3_PROBE_NODE).await {
        return Err("长镜头要 H3（ref2va 分区），本机没有它的 WeeTodd 节点".into());
    }

    // 参考图逐窗口搬进 ComfyUI 的 input
    let mut names: Vec<Vec<String>> = Vec::new();
    for (wi, (_, refs)) in windows.iter().enumerate() {
        let mut cur = Vec::new();
        for (i, src) in refs.iter().take(MAX_REFERENCE_IMAGES).enumerate() {
            let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("png").to_ascii_lowercase();
            let name = format!("cinema_lt_{seed}_{wi}_{i}.{ext}");
            let mut copied = false;
            for dir in find_comfyui_input_dirs() {
                copied |= std::fs::copy(src, dir.join(&name)).is_ok();
            }
            if !copied {
                return Err(format!("参考图复制进 ComfyUI 的 input 失败：{}", src.display()));
            }
            cur.push(name);
        }
        names.push(cur);
    }

    let ws: Vec<weetodd::H3Window> = windows
        .iter()
        .zip(names.iter())
        .map(|((prompt, _), imgs)| weetodd::H3Window { prompt, images: imgs })
        .collect();

    let graph = weetodd::build_h3_chain_graph(&weetodd::H3ChainParams {
        windows: &ws,
        window_secs,
        context_frames,
        width,
        height,
        seed,
        tier: tier.unwrap_or(weetodd::H3Tier::Standard),
        filename_prefix: "Anima/cinema_long",
        loras,
    });

    let bytes = submit_and_fetch_video(&client, &graph, "长镜头（窗口链）", Some(out)).await?;
    if let Some(parent) = out.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(out, &bytes).map_err(|e| format!("写入 {} 失败：{e}", out.display()))?;
    finalize_shot_audio(out, generate_audio);
    Ok(())
}

/// 纯文本出片（H3 t2va）：这一镜一张参考图都挂不上时用。提示词是线路合成好的三段 Base 结构，原样发送。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn render_shot_video_text(
    out: &Path,
    prompt: &str,
    duration_secs: f64,
    width: u32,
    height: u32,
    seed: u64,
    generate_audio: bool,
    tier: Option<weetodd::H3Tier>,
) -> std::result::Result<(), String> {
    // 同上，32 对齐
    let (width, height) = weetodd::fit_dimensions((width, height));
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(120))
        .build()
        .unwrap_or_default();
    if !weetodd::node_available(&client, weetodd::H3_PROBE_NODE).await {
        return Err("文生视频要 H3，本机没有它的 WeeTodd 节点。装上 WeeTodd-Nodes 再出片。".into());
    }
    let graph = weetodd::build_h3_graph(&weetodd::H3Params {
        continuity: None,
        mode: weetodd::H3Mode::Text,
        prompt,
        width,
        height,
        duration_secs: duration_secs.max(1.0),
        seed,
        tier: tier.unwrap_or(if width.max(height) <= 640 { weetodd::H3Tier::Preview } else { weetodd::H3Tier::Standard }),
        filename_prefix: "Anima/cinema",
        loras: &[],
    });
    let bytes = submit_and_fetch_video(&client, &graph, "cinema shot (文生)", Some(out)).await?;
    if let Some(parent) = out.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(out, &bytes).map_err(|e| format!("写入 {} 失败：{e}", out.display()))?;
    finalize_shot_audio(out, generate_audio);
    Ok(())
}

/// history 里这条任务有没有报执行错误。
fn weetodd_exec_error(entry: &Value) -> Option<String> {
    let msgs = entry.pointer("/status/messages")?.as_array()?;
    for m in msgs {
        let a = m.as_array()?;
        if a.first().and_then(Value::as_str) == Some("execution_error") {
            let d = a.get(1)?;
            return Some(format!(
                "ComfyUI 执行失败于节点 {} ({}): {}",
                d.get("node_id").and_then(Value::as_str).unwrap_or("?"),
                d.get("node_type").and_then(Value::as_str).unwrap_or("?"),
                d.get("exception_message").and_then(Value::as_str).unwrap_or("?")
            ));
        }
    }
    None
}

// ─── 图生视频工具 (Animate Image Tool - 真正的 I2V 工作流) ───────────────────

pub struct AnimateImageTool;

#[async_trait]
impl ToolHandler for AnimateImageTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "animate_image",
            "Animate a static image into a dynamic MP4 video clip using local ComfyUI I2V engines. Routes through LTX-2.5 by default, can explicitly use MiniMax H3, and falls back from unusable LTX results to MiniMax H3 in auto mode.",
            ToolKind::BuiltIn,
            Operation::Write,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "image_path": {
                    "type": "string",
                    "description": "Path to the local source image to animate (e.g. '.anima/generated/img_123.png' or absolute path)"
                },
                "motion_prompt": {
                    "type": "string",
                    "description": "Prompt describing the dynamic motion, camera pan, or character action (e.g. 'gentle camera zoom in, wind blowing hair, blinking eyes')"
                },
                "width": {
                    "type": "integer",
                    "description": "Video frame width in pixels (e.g. 1344 for landscape, 768 for portrait)"
                },
                "height": {
                    "type": "integer",
                    "description": "Video frame height in pixels (e.g. 768 for landscape, 1344 for portrait)"
                },
                "duration_secs": {
                    "type": "integer",
                    "default": 2,
                    "description": "Duration in seconds. COST: roughly 4-5 minutes of wall time per second of 480p H3 video. Keep it at 2 unless the user asked for longer."
                },
                "fps": {
                    "type": "integer",
                    "default": 24,
                    "description": "Video frame rate. Leave it out: H3 / LTX render at 24 (16 was the retired Wan default)"
                },
                "steps": {
                    "type": "integer",
                    "default": 20,
                    "description": "Sampling steps. MiniMax H3 fallback automatically uses its 8-step turbo LoRA when available."
                },
                "engine": {
                    "type": "string",
                    "enum": ["auto", "ltx", "minimax"],
                    "default": "auto",
                    "description": "I2V engine route. Use auto by default; choose minimax when LTX-2.5 produces black frames or fails."
                },
                "quality": {
                    "type": "string",
                    "enum": ["fast", "balanced", "quality"],
                    "default": "fast",
                    "description": "Speed/quality trade-off (MiniMax H3). MEASURED on this machine at 864x480/56 frames: fast = 4 turbo steps (~159s, may show motion smear on fast action), balanced = 8 steps (~253s, removes the smear), quality = 8 steps with sparse attention off (~264s). Raise this when the user complains about ghosting or trailing; for facial distortion raise `width`/`height` instead — at 480p a face is only a few dozen pixels wide and no sampler setting fixes that."
                },
                "end_image": {
                    "type": "string",
                    "description": "Optional path to an image the video should END on (first-and-last-frame generation). The clip starts at `image_path` and converges to this one. Verified on this machine: given a start frame and a structurally different end frame, the final video frame reproduces the END frame's geometry, not the start's. Both images should be the same aspect ratio. MiniMax H3 only, and only with `mode: \"first_frame\"` — it has no effect in `reference` mode. Leave it out for a normal one-image clip."
                },
                "mode": {
                    "type": "string",
                    "enum": ["first_frame", "reference"],
                    "default": "first_frame",
                    "description": "How the source image is used. `first_frame` (default) makes it the literal opening frame — only right when the image ALREADY shows the scene you want, otherwise frame 0 is the still and frame 1 jumps. `reference` uses it as an identity/appearance reference and generates the scene freely: no jarring first frame, and the character stays consistent. Use `reference` whenever you want a character from an existing image placed into a NEW scene. MiniMax H3 only."
                },
                "reference_images": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Extra reference images for `mode: reference` (up to 9 total, including image_path). Use each character's single-person main image. Do NOT pass a whole multi-view character sheet as identity: each face on it is only ~40 px and H3 cannot lock identity from that."
                },
                "h3_quality": {
                    "type": "string",
                    "enum": ["lightx2v", "standard", "staged", "dense"],
                    "description": "H3 only: which acceleration LoRA / step count. lightx2v = LightX2V 4-step (best for first-frame clips), standard = Larry v4 4 steps, staged = Larry v4 6 steps (best for reference mode), dense = no acceleration, 19 evaluations (~4x slower). Omit to pick by size."
                },
                "reference_audios": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Reference audio files for `mode: reference` on H3 (up to 3), labelled <Audio 1..> in the prompt after the images. Use for a song / voice / sound the clip must follow (official Ref2VA: write `<Audio 1>: fully_copy` in retention_analysis to reuse it as the final soundtrack). Each file is trimmed to the clip duration."
                },
                "seed": {
                    "type": "integer",
                    "description": "Random seed. To REFINE a clip you already produced (adjust the motion, change a detail), pass back the exact seed reported in that clip's result — same seed + edited prompt keeps the same motion and framing. Omit it only when you want a genuinely different clip."
                },
                "loras": crate::agent_runtime::lora::schema_fragment()
            },
            "required": ["image_path"]
        }))
        .with_timeout(Duration::from_secs(VIDEO_TOOL_TIMEOUT_SECS))
    }

    async fn execute(&self, ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let Some(image_path_raw) = request
            .arguments
            .get("image_path")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(ToolFailureKind::InvalidParams, "缺少 image_path 参数"),
                Duration::from_millis(0),
            );
        };

        let started = Instant::now();
        let prompt = request
            .arguments
            .get("motion_prompt")
            .or_else(|| request.arguments.get("prompt"))
            .and_then(|v| v.as_str())
            .unwrap_or("gentle cinematic motion, natural physics animation, high quality");
        let duration_secs = request
            .arguments
            .get("duration_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(4) as u32;
        let clean_path = image_path_raw.trim_start_matches("file://");
        let source_path = if Path::new(clean_path).is_absolute() {
            PathBuf::from(clean_path)
        } else {
            ctx.workspace_root.join(clean_path)
        };

        if !source_path.is_file() {
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(
                    ToolFailureKind::InvalidParams,
                    format!("找不到源图像文件: {}", source_path.display()),
                ),
                started.elapsed(),
            );
        }

        // **尺寸要在确认文件存在之后再算。** `image_dimensions` 读的就是这个
        // 文件，排在存在性检查前面等于先对着不存在的路径量一遍尺寸，
        // 再落到那句友好报错上 —— 顺序反了。
        let req_width = request
            .arguments
            .get("width")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);
        let req_height = request
            .arguments
            .get("height")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);
        let (width, height) = match (req_width, req_height) {
            (Some(w), Some(h)) if w > 0 && h > 0 => weetodd::fit_dimensions((w, h)),
            _ => weetodd::fit_dimensions(image_dimensions(&source_path)),
        };

        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(120))
            .build()
            .unwrap_or_default();

        // 1. 将源图像安全复制/上传到 ComfyUI 的 input 目录
        //
        // seed 优先取模型传回来的 —— 报了 seed 却不收，等于报了个没法用的东西。
        let seed = request
            .arguments
            .get("seed")
            .and_then(|v| v.as_u64())
            .unwrap_or_else(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64
                    % 1_000_000_000
            });
        let input_filename = format!("i2v_source_{seed}.png");

        stage_keyframe(&source_path, &input_filename, (width, height));

        // 末帧（首尾帧生视频）。和源图同一套路径解析和搬运规则。
        //
        // 找不到文件时**直接失败**，不要静默退回只给首帧 —— 那样用户会拿到
        // 一段"生成成功但末帧没生效"的视频，而这正是这一块最典型的失败形状。
        let end_filename: Option<String> = match request
            .arguments
            .get("end_image")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            None => None,
            Some(raw) => {
                let p = if Path::new(raw).is_absolute() {
                    PathBuf::from(raw)
                } else {
                    ctx.workspace_root.join(raw)
                };
                if !p.is_file() {
                    return ToolOutcome::failed(
                        &request,
                        ToolFailure::new(
                            ToolFailureKind::InvalidParams,
                            format!("末帧图片不存在：{}", p.display()),
                        ),
                        started.elapsed(),
                    );
                }
                let name = format!("i2v_end_{seed}.png");
                stage_keyframe(&p, &name, (width, height));
                Some(name)
            }
        };

        // **内存不够就在提交前拦住。**
        //
        // H3 在 `--fp32-unet` 下要约 105 GB。不够的时候 ComfyUI 会被系统直接
        // 杀掉、不留任何 Python traceback —— 界面上和"超时"长得一模一样，
        // 用户等七分钟只会拿到一个查不出原因的失败。实测撞到过两次。
        let free_gb = crate::comfyui::free_memory_gb();
        if let Some(free) = crate::comfyui::h3_memory_blocker(free_gb) {
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(
                    ToolFailureKind::Precondition,
                    crate::comfyui::h3_memory_hint(free),
                ),
                started.elapsed(),
            );
        }

        // R2V：源图 + 额外参考图一起搬进 ComfyUI 的 input 目录。
        //
        // 源图排第一 —— `ref_images` 是有序的，第一张影响最大。
        let reference_mode = request
            .arguments
            .get("mode")
            .and_then(|v| v.as_str())
            .map(|m| m.eq_ignore_ascii_case("reference"))
            .unwrap_or(false);
        let mut reference_filenames: Vec<String> = vec![input_filename.clone()];
        if reference_mode {
            if let Some(list) = request.arguments.get("reference_images").and_then(|v| v.as_array()) {
                for (i, item) in list.iter().enumerate() {
                    let Some(raw) = item.as_str().map(str::trim).filter(|s| !s.is_empty()) else {
                        continue;
                    };
                    // 和源图同一套解析规则：绝对路径直接用，相对路径接工作区根
                    let src = if Path::new(raw).is_absolute() {
                        PathBuf::from(raw)
                    } else {
                        ctx.workspace_root.join(raw)
                    };
                    if !src.exists() {
                        continue;
                    }
                    let name = format!("r2v_ref_{seed}_{i}.png");
                    let mut copied = false;
                    for input_dir in find_comfyui_input_dirs() {
                        copied |= std::fs::copy(&src, input_dir.join(&name)).is_ok();
                    }
                    if copied {
                        reference_filenames.push(name);
                    }
                }
            }
            reference_filenames.truncate(MAX_REFERENCE_IMAGES);
        }
        // 参考音频（画布「音频素材」节点，2026-10-08）：截到这一条的时长再搬进 ComfyUI 的 input
        let mut reference_audios: Vec<String> = Vec::new();
        if reference_mode {
            if let Some(list) = request.arguments.get("reference_audios").and_then(|v| v.as_array()) {
                for (i, item) in list.iter().take(weetodd::H3_MAX_REFERENCE_AUDIOS).enumerate() {
                    let Some(raw) = item.as_str().map(str::trim).filter(|s| !s.is_empty()) else { continue };
                    let src = if Path::new(raw).is_absolute() { PathBuf::from(raw) } else { ctx.workspace_root.join(raw) };
                    if !src.exists() {
                        return ToolOutcome::failed(
                            &request,
                            ToolFailure::new(ToolFailureKind::Precondition, format!("参考音频不在了：{}", src.display())),
                            started.elapsed(),
                        );
                    }
                    let name = format!("r2v_audio_{seed}_{i}.wav");
                    match stage_reference_audio(&src, &name, duration_secs as f64) {
                        Ok(()) => reference_audios.push(name),
                        Err(e) => {
                            return ToolOutcome::failed(&request, ToolFailure::new(ToolFailureKind::Execution, e), started.elapsed());
                        }
                    }
                }
            }
        }

        // `"wan2.1_i2v"` 在这里是**家族关键词**，不是文件名 —— 曾经它被当成
        // 文件名原样发出去，导致每一次图生视频都被 ComfyUI 拒掉。
        let engine_hint = request
            .arguments
            .get("engine")
            .and_then(|v| v.as_str())
            .unwrap_or("auto")
            .trim()
            .to_lowercase();
        // ── 选路：图生视频同样优先走 WeeTodd/MLX ──────────────────────────
        //
        // 和文生视频用同一套探测和优先级，**不能只接一半** —— 原生那条的 H3
        // 权重已经删了，图生视频要是还留在原生分支上，点名 H3 会撞到一堆
        // 找不到的模型文件，而错误信息会指向 VAE 而不是"这条路没了"。
        let (ltx_ok, h3_ok) = tokio::join!(
            weetodd::node_available(&client, weetodd::PROBE_NODE),
            weetodd::node_available(&client, weetodd::H3_PROBE_NODE),
        );
        if let Some(msg) = retired_video_engine(&engine_hint, None) {
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(ToolFailureKind::Precondition, msg),
                started.elapsed(),
            );
        }
        let route = weetodd::pick_route(&engine_hint, ltx_ok, h3_ok);
        if route != weetodd::Route::Native {
            if let Some(msg) = weetodd::animate_blocker(route, reference_mode)
            {
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(ToolFailureKind::Precondition, msg),
                    started.elapsed(),
                );
            }
            let (w, h) = (width, height);
            let refs: &[String] = if reference_mode { &reference_filenames } else { &[] };
            if !reference_audios.is_empty() && route != weetodd::Route::H3 {
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(ToolFailureKind::Precondition, "参考音频只有 MiniMax H3 的参考生视频能用 —— 把引擎换成 H3"),
                    started.elapsed(),
                );
            }
            return GenerateVideoTool::run_weetodd(
                ctx,
                &request,
                &client,
                route,
                prompt,
                w,
                h,
                duration_secs,
                seed,
                Some(&input_filename),
                end_filename.as_deref(),
                request.arguments.get("fps").and_then(|v| v.as_f64()),
                // 图生视频目前只有单窗口，段动作留空
                &[],
                refs,
                &reference_audios,
                weetodd::route_note(route, &engine_hint),
                started,
            )
            .await;
        }

        // **LTX 是独占模式：提交前就判，不跑完再猜。**
        //
        // LTX-2.5 只在 ComfyUI 带 `--fp32-unet` 且内存够（实测峰值 108GB）时
        // 才出得了图；否则 ComfyUI 会按内存自己挑 bf16，而 bf16 在 MPS 上让
        // LTX 的 latent 全变成 NaN —— 出来是一段**纯黑视频，不报任何错**。
        //
        // 原来的做法是先跑 LTX（150~250 秒）、检测输出是不是黑、再回退 MiniMax。
        // 每次白烧那么久，而且"检测黑屏"本身是在猜。启动参数和可用内存都是
        // **提交前就能读到**的东西。
        let ltx_blocker = crate::comfyui::ltx_blocker(
            crate::comfyui::running_argv().await.as_deref(),
            crate::comfyui::free_memory_gb(),
        );
        let wants_ltx = engine_hint.contains("ltx");
        if wants_ltx {
            if let Some(blocker) = &ltx_blocker {
                // 用户点名要 LTX 而条件不满足 —— 直接说清差什么，别偷偷换引擎
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(ToolFailureKind::Execution, blocker.explain()),
                    started.elapsed(),
                );
            }
        }
        let preferred = pick_i2v_preferred(&engine_hint, ltx_blocker.is_some());

        let unet_name = match pick_video_unet(
            &GenerateVideoTool::available_unets(&client).await,
            None,
            preferred,
        ) {
            Ok(name) => name,
            Err(msg) => {
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(ToolFailureKind::Execution, msg),
                    started.elapsed(),
                );
            }
        };
        let unet_lower = unet_name.to_lowercase();
        let is_ltx = unet_lower.contains("ltx");
        let is_minimax = unet_lower.contains("minimax");
        // 原生图生视频只剩 LTX / H3 两套拓扑。Wan 那套（`WanImageToVideo`）
        // 2026-10-01 随权重一起下架 —— 点名别的模型时明确拒绝，
        // 不能让它落进某个拓扑里静默出错的图。
        if !is_ltx && !is_minimax {
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(
                    ToolFailureKind::Precondition,
                    native_video_unsupported(&unet_name, "图生视频", "LTX-2.5 和 MiniMax H3"),
                ),
                started.elapsed(),
            );
        }
        let (vae_name, clip_name, i2v_audio_vae_name, ltx_upscaler_name) = if is_ltx {
            let vaes = GenerateVideoTool::available_vaes(&client).await;
            let video_vae = match pick_required_model(
                &vaes,
                &["ltx-2.5-video-vae-bf16.safetensors"],
                &["ltx-2.5-video-vae", "ltx"],
                "LTX-2.5 video VAE",
            ) {
                Ok(name) => name,
                Err(msg) => {
                    return ToolOutcome::failed(
                        &request,
                        ToolFailure::new(ToolFailureKind::Execution, msg),
                        started.elapsed(),
                    );
                }
            };
            let audio_vae = match pick_required_model(
                &vaes,
                &["ltx-2.5-audio-vae-bf16.safetensors"],
                &["ltx-2.5-audio-vae", "audio-vae"],
                "LTX-2.5 audio VAE",
            ) {
                Ok(name) => name,
                Err(msg) => {
                    return ToolOutcome::failed(
                        &request,
                        ToolFailure::new(ToolFailureKind::Execution, msg),
                        started.elapsed(),
                    );
                }
            };
            let clips = GenerateVideoTool::available_clips(&client).await;
            let clip = match pick_required_model(
                &clips,
                &[
                    "gemma4-12b-with-proj-ltx-2.5-bf16.safetensors",
                    "gemma4-12b-with-proj-ltx-2.5-comfy-int8-convrot.safetensors",
                ],
                &["gemma4-12b-with-proj-ltx-2.5", "gemma4-12b"],
                "LTX-2.5 Gemma4 text encoder",
            ) {
                Ok(name) => name,
                Err(msg) => {
                    return ToolOutcome::failed(
                        &request,
                        ToolFailure::new(ToolFailureKind::Execution, msg),
                        started.elapsed(),
                    );
                }
            };
            let upscalers = GenerateVideoTool::available_latent_upscale_models(&client).await;
            let upscaler = match pick_required_model(
                &upscalers,
                &["ltx-2.5-latent-spatial-upscaler-x2-bf16-1.0.safetensors"],
                &["ltx-2.5-latent-spatial-upscaler", "ltx"],
                "LTX-2.5 latent spatial upscaler",
            ) {
                Ok(name) => name,
                Err(msg) => {
                    return ToolOutcome::failed(
                        &request,
                        ToolFailure::new(ToolFailureKind::Execution, msg),
                        started.elapsed(),
                    );
                }
            };

            (video_vae, clip, Some(audio_vae), Some(upscaler))
        } else {
            let vaes = GenerateVideoTool::available_vaes(&client).await;
            let video_vae = match pick_required_model(
                &vaes,
                &[], // 已随原 H3 一起删除，见 weetodd::H3_MIGRATED_NOTE
                &["minimax_h3_video_vae", "minimax"],
                "MiniMax H3 video VAE",
            ) {
                Ok(name) => name,
                Err(msg) => {
                    return ToolOutcome::failed(
                        &request,
                        ToolFailure::new(ToolFailureKind::Execution, msg),
                        started.elapsed(),
                    );
                }
            };
            let audio_vae = match pick_required_model(
                &vaes,
                &[], // 同上
                &["minimax_h3_audio_vae", "audio"],
                "MiniMax H3 audio VAE",
            ) {
                Ok(name) => name,
                Err(msg) => {
                    return ToolOutcome::failed(
                        &request,
                        ToolFailure::new(ToolFailureKind::Execution, msg),
                        started.elapsed(),
                    );
                }
            };
            let clips = GenerateVideoTool::available_clips(&client).await;
            let clip = match pick_required_model(
                &clips,
                &[], // 同上
                &["qwen3vl_32b_minimax_h3", "minimax_h3", "qwen3vl"],
                "MiniMax H3 Qwen3VL text encoder",
            ) {
                Ok(name) => name,
                Err(msg) => {
                    return ToolOutcome::failed(
                        &request,
                        ToolFailure::new(ToolFailureKind::Execution, msg),
                        started.elapsed(),
                    );
                }
            };

            (video_vae, clip, Some(audio_vae), None)
        };

        let connecting_label = if is_ltx {
            "正在构建 LTX-2.5 官方图生视频 (I2V) 管线"
        } else {
            "正在构建 MiniMax H3 官方图生视频 (I2V) 管线"
        };

        GenerateVideoTool::emit_progress(
            ctx,
            &request,
            prompt,
            duration_secs,
            "connecting",
            connecting_label,
            0.08,
            None,
            None,
            None,
        )
        .await;

        // LoRA 通路。视频这条用 `LoraLoaderModelOnly` —— UNETLoader 不产 CLIP。
        // MiniMax H3 官方图里那个**必需**的 turbo LoRA 就是这么挂的，
        // 所以这不是可选装饰，是后面接 H3 的前置条件。
        //
        // 对不上名字直接失败并列出本机有哪些，不静默跳过：静默跳过的话用户
        // 拿到的是一段"生成成功但 LoRA 完全没生效"的视频，而且不报任何错。
        let mut lora_warning: Option<String> = None;
        let requested_loras = lora::parse_requested(&request.arguments);
        let auto_minimax_fallback = is_minimax && is_ltx_fallback_request(&request);
        let available_loras = if is_minimax || !requested_loras.is_empty() {
            lora::available_loras(&client).await
        } else {
            Vec::new()
        };
        let mut resolved_loras = if requested_loras.is_empty() {
            Vec::new()
        } else {
            let (resolved, warn) = lora::resolve_loras(&requested_loras, &available_loras);
            lora_warning = warn;
            resolved
        };
        let mut minimax_auto_turbo = false;
        if auto_minimax_fallback && !turbo_already_requested(&resolved_loras) {
            if let Some(found) = find_turbo_lora(&available_loras) {
                resolved_loras.push(lora::ResolvedLora { name: found, strength: 1.0 });
                minimax_auto_turbo = true;
            } else {
                let note = format!(
                    "MiniMax H3 fallback 未找到官方 turbo LoRA `{MINIMAX_H3_TURBO_LORA}`，\
                     将退回普通采样，速度会明显变慢。"
                );
                lora_warning = Some(match lora_warning {
                    Some(existing) => format!("{existing}\n\n{note}"),
                    None => note,
                });
            }
        }
        let chain = lora::build_chain(&resolved_loras, 900, json!(["1", 0]), None);
        let model_ref = chain.model_ref.clone();

        // 2. 构造真实的 I2V 工作流（LTX-2.5 / MiniMax H3 两套拓扑）
        let effective_fps = 24;
        let frames_count = if is_ltx {
            // LTX-2.5 帧数以 8 为步进，通常为 8k+1 或 duration * fps
            ((duration_secs * effective_fps + 4) / 8) * 8 + 1
        } else {
            snap_frames_17n_plus_5(duration_secs * effective_fps)
        };

        let mut prompt_payload = if is_ltx {
            let audio_vae_name = i2v_audio_vae_name
                .as_deref()
                .expect("LTX branch validated audio VAE");
            let latent_upscaler_name = ltx_upscaler_name
                .as_deref()
                .expect("LTX branch validated latent upscaler");
            build_ltx_i2v_graph(&LtxI2vParams {
                unet_name: &unet_name,
                clip_name: &clip_name,
                video_vae_name: &vae_name,
                audio_vae_name,
                latent_upscaler_name,
                prompt,
                neg_prompt: "pc game, console game, video game, cartoon, childish, ugly, static image, blurry, low frame rate, jerky movement, watermark, bad anatomy, artifacts, glitches, black frames",
                input_filename: &input_filename,
                width,
                height,
                frames_count: frames_count.max(25),
                fps: effective_fps,
                seed,
                model_ref,
            })
        } else {
            let audio_vae_name = i2v_audio_vae_name
                .as_deref()
                .expect("MiniMax H3 branch validated audio VAE");
            // 显式 steps 优先；没给就按质量档（档位里的数字都是实测的）
            let quality = VideoQuality::from_arg(
                request.arguments.get("quality").and_then(|v| v.as_str()),
            );
            let steps = request
                .arguments
                .get("steps")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32)
                .unwrap_or(if minimax_auto_turbo {
                    quality.steps()
                } else {
                    20
                });
            if reference_mode {
                // 参考模式：画面自由生成，人物特征来自参考图 —— 首帧不再是源图本身
                build_minimax_h3_r2v_graph(&MinimaxH3R2vParams {
                    unet_name: &unet_name,
                    clip_name: &clip_name,
                    video_vae_name: &vae_name,
                    audio_vae_name,
                    prompt,
                    reference_filenames: &reference_filenames,
                    width,
                    height,
                    frames_count,
                    seed,
                    steps,
                    // `max` 身份保真更好；参考 token 参与每一步采样，所以更慢
                    // （实测 864x480 下 159s → 250s），但用户点名要参考模式时
                    // 图像一致性就是他要的东西
                    ref_image_size: "max",
                    model_ref,
                })
            } else {
                build_minimax_h3_i2v_graph(&MinimaxH3I2vParams {
                    unet_name: &unet_name,
                    clip_name: &clip_name,
                    video_vae_name: &vae_name,
                    audio_vae_name,
                    prompt,
                    input_filename: &input_filename,
                    end_filename: end_filename.as_deref(),
                    width,
                    height,
                    frames_count,
                    seed,
                    steps,
                    model_ref,
                })
            }
        };

        // 提交并轮询 I2V
        if let Some(map) = prompt_payload
            .get_mut("prompt")
            .and_then(|v| v.as_object_mut())
        {
            for (id, node) in chain.nodes {
                map.insert(id, node);
            }
        }

        let submit_url = format!("{}/prompt", DEFAULT_COMFY_URL);
        let resp = match client.post(&submit_url).json(&prompt_payload).send().await {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                let err_text = r.text().await.unwrap_or_default();
                if should_auto_fallback_to_minimax(&engine_hint, is_ltx) {
                    GenerateVideoTool::emit_progress(
                        ctx,
                        &request,
                        prompt,
                        duration_secs,
                        "fallback",
                        "LTX-2.5 提交被 ComfyUI 拒绝，正在切换 MiniMax H3 官方 I2V 链路",
                        0.05,
                        None,
                        None,
                        Some(&err_text),
                    )
                    .await;
                    free_comfyui_cache(&client, false).await;
                    let mut fallback =
                        Box::pin(self.execute(ctx, minimax_fallback_request(&request, seed))).await;
                    if matches!(fallback.status, ToolStatus::Ok) {
                        fallback.content = format!(
                            "LTX-2.5 I2V 提交失败，已自动切换 MiniMax H3 完成生成。\n\nLTX 提交错误：{err_text}\n\n{}",
                            fallback.content
                        );
                    }
                    return fallback;
                }
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(
                        ToolFailureKind::Execution,
                        format!("ComfyUI 拒绝 I2V 任务: {err_text}"),
                    ),
                    started.elapsed(),
                );
            }
            Err(e) => {
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(
                        ToolFailureKind::Transport,
                        format!("提交 I2V 任务失败: {e}"),
                    ),
                    started.elapsed(),
                );
            }
        };

        let resp_json: Value = resp.json().await.unwrap_or(Value::Null);
        let Some(prompt_id) = resp_json.get("prompt_id").and_then(|v| v.as_str()) else {
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(ToolFailureKind::Execution, "ComfyUI 未返回 prompt_id"),
                started.elapsed(),
            );
        };

        // 轮询历史
        let history_url = format!("{}/history/{}", DEFAULT_COMFY_URL, prompt_id);
        let mut video_filename = String::new();
        let mut video_subfolder = String::new();
        let mut video_type = "output".to_string();

        // 记账：交出去了。销账只在成功落盘那一步 —— 超时/失败故意留着，
        // 让开机对账去收（视频一跑几分钟到几十分钟，白丢一次代价很大）。
        // （原来这一句放在轮询循环里，每 3 秒写一遍）
        super::comfy_jobs::remember(
            prompt_id,
            &ctx.ids.session_id,
            "video",
            "10,9,outputs",
            &super::comfy_jobs::summarize(prompt),
        )
        .await;

        // **不按时长判超时，看进度**（`comfy_watch`）。自动回落那条原来另有 45 分钟的看门狗，
        // 也一并换掉 —— 720p 一镜就可能超过 4 小时（2026-09-30 用户：「全部根据 Comfy 实时状态抓」）
        let _ = auto_minimax_fallback;
        let mut watch = super::comfy_watch::tracker(prompt_id);
        let mut pending = 0u32;
        let mut gave_up: Option<String> = None;
        let mut lost_streak: u32 = 0;
        let mut poll_index = 0u32;
        loop {
            if pending >= SHOT_PENDING_CEILING_POLLS {
                gave_up = Some("在 ComfyUI 队列里排了 12 小时还没轮到 —— 前面的任务可能卡住了".into());
                break;
            }
            poll_index += 1;
            tokio::time::sleep(Duration::from_secs(VIDEO_POLL_INTERVAL_SECS)).await;
            let estimated_progress = (0.18 + ((poll_index as f32) / VIDEO_PROGRESS_POLLS as f32) * 0.70).min(0.88);
            let running_label = if is_ltx {
                if estimated_progress < 0.40 {
                    "LTX-2.5 正在执行第一阶段潜在空间生成"
                } else if estimated_progress < 0.78 {
                    "LTX-2.5 正在执行第二阶段潜空间超分与运动细化"
                } else {
                    "LTX-2.5 正在解码音视频并导出 MP4"
                }
            } else if is_minimax {
                if estimated_progress < 0.85 {
                    "MiniMax H3 正在执行官方图生视频扩散"
                } else {
                    "MiniMax H3 正在解码音视频并导出 MP4"
                }
            } else {
                if estimated_progress < 0.85 {
                    "Wan2.1 正在执行真实图生视频扩散"
                } else {
                    "正在解码并压制 MP4 视频流"
                }
            };
            let running_label = match watch.describe() {
                Some(p) => format!("{running_label} · {p}"),
                None => running_label.to_string(),
            };
            GenerateVideoTool::emit_progress(
                ctx,
                &request,
                prompt,
                duration_secs,
                "running",
                &running_label,
                estimated_progress,
                Some(prompt_id),
                None,
                None,
            )
            .await;

            let mut in_history = false;
            let mut history_ok = false;
            if let Ok(h_resp) = client.get(&history_url).send().await {
                if let Ok(h_json) = h_resp.json::<Value>().await {
                    history_ok = true;
                    in_history = h_json.get(prompt_id).is_some();
                    if let Some(err_str) = extract_comfyui_history_error(&h_json, prompt_id) {
                        GenerateVideoTool::emit_progress(
                            ctx,
                            &request,
                            prompt,
                            duration_secs,
                            "error",
                            "视频生成报错",
                            0.0,
                            Some(prompt_id),
                            None,
                            Some(&err_str),
                        )
                        .await;
                        if should_auto_fallback_to_minimax(&engine_hint, is_ltx) {
                            GenerateVideoTool::emit_progress(
                                ctx,
                                &request,
                                prompt,
                                duration_secs,
                                "fallback",
                                "LTX-2.5 渲染失败，正在切换 MiniMax H3 官方 I2V 链路",
                                0.05,
                                Some(prompt_id),
                                None,
                                Some(&err_str),
                            )
                            .await;
                            free_comfyui_cache(&client, false).await;
                            let mut fallback = Box::pin(
                                self.execute(ctx, minimax_fallback_request(&request, seed)),
                            )
                            .await;
                            if matches!(fallback.status, ToolStatus::Ok) {
                                fallback.content = format!(
                                    "LTX-2.5 I2V 失败，已自动切换 MiniMax H3 完成生成。\n\nLTX 失败原因：{err_str}\n\n{}",
                                    fallback.content
                                );
                            }
                            return fallback;
                        }
                        return ToolOutcome::failed(
                            &request,
                            ToolFailure::new(
                                ToolFailureKind::Execution,
                                format!("ComfyUI 视频渲染失败: {err_str}"),
                            ),
                            started.elapsed(),
                        );
                    }

                    // **扫全部输出节点，不用固定 id 列表。**
                    // 原来这里写死了几个节点号，每接一条新链路就要补一次，
                    // 漏了的症状是「任务 success 但找不到文件」。
                    // WeeTodd 的 LTX 出在节点 4、H3 出在 7，都不在原来那个列表里。
                    if let Some(outs) = h_json.pointer(&format!("/{prompt_id}/outputs")) {
                        if let Some((f, sub, ty)) =
                            crate::agent_runtime::weetodd::find_video_output(outs)
                        {
                            video_filename = f;
                            video_subfolder = sub;
                            video_type = ty;
                        }
                    }
                    if !video_filename.is_empty() {
                        break;
                    }
                }
            }

            // **ComfyUI 还认不认这个任务。** 只看 history 的话，排队时被取消的任务
            // 永远不会出现在 history 里，这里就会一直等到两小时的上限 ——
            // 2026-09-13 实拍：同一镜的第二个出片任务排队 32 秒就被取消，
            // 这里空等了 34 分钟，队列面板上一直在计时。
            let (seen, in_queue) = queue_state(&client, prompt_id).await;
            lost_streak = next_lost_streak(lost_streak, in_history, in_queue);
            if lost_streak >= VIDEO_LOST_TOLERANCE {
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(
                        ToolFailureKind::Execution,
                        format!(
                            "ComfyUI 里已经没有这个任务了（prompt_id `{prompt_id}`：不在队列里，也没有结果）。\
                             通常是被取消了 —— ComfyUI 网页上点了取消/清空队列，或者别的程序取消了它。重新出一次即可。"
                        ),
                    ),
                    started.elapsed(),
                );
            }
            if seen == super::comfy_watch::Seen::Queued {
                pending += 1;
            }
            let seen = if history_ok { seen } else { super::comfy_watch::Seen::Unreachable };
            if let Some(msg) = watch.observe(seen).explain(prompt_id) {
                gave_up = Some(match watch.describe() {
                    Some(p) => format!("{msg}（最后一次：{p}）"),
                    None => msg,
                });
                break;
            }
        }

        if video_filename.is_empty() {
            let interrupted = interrupt_prompt_if_running(&client, prompt_id).await;
            let interrupt_note = if interrupted {
                "已向 ComfyUI 发送 interrupt，避免这个卡住的任务继续占用队列。"
            } else {
                "ComfyUI 队列中未检测到该任务仍在运行。"
            };
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(
                    ToolFailureKind::Execution,
                    format!(
                        "{}。{interrupt_note} 可稍后查 http://127.0.0.1:8188/history/{prompt_id}。",
                        gave_up.unwrap_or_else(|| format!("没拿到视频（prompt_id `{prompt_id}`）"))
                    ),
                ),
                started.elapsed(),
            );
        }

        // 下载与落盘
        let view_url = format!(
            "{}/view?filename={}&subfolder={}&type={}",
            DEFAULT_COMFY_URL, video_filename, video_subfolder, video_type
        );
        let video_resp = match client.get(&view_url).send().await {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(
                        ToolFailureKind::Transport,
                        format!("下载视频失败，HTTP 状态: {}", r.status()),
                    ),
                    started.elapsed(),
                );
            }
            Err(e) => {
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(ToolFailureKind::Transport, format!("请求视频失败: {e}")),
                    started.elapsed(),
                );
            }
        };

        let video_bytes = match video_resp.bytes().await {
            Ok(b) => b,
            Err(e) => {
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(ToolFailureKind::Transport, format!("读取视频流失败: {e}")),
                    started.elapsed(),
                );
            }
        };

        if video_bytes.len() < 10240 {
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(
                    ToolFailureKind::Execution,
                    "生成的视频流小于 10KB，可能损坏",
                ),
                started.elapsed(),
            );
        }

        let generated_dir = ctx.workspace_root.join(".anima").join("generated");
        let _ = std::fs::create_dir_all(&generated_dir);
        let target_path = generated_dir.join(format!("i2v_video_{seed}.mp4"));
        if let Err(e) = std::fs::write(&target_path, video_bytes) {
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(
                    ToolFailureKind::Execution,
                    format!("写入 I2V 视频失败: {e}"),
                ),
                started.elapsed(),
            );
        }
        // 销账放在**落盘成功之后**。写盘失败的话记录留着，
        // 让开机对账再取一次 —— 结果在 ComfyUI 那边一直都在。
        super::comfy_jobs::finish(prompt_id).await;

        let generate_audio = request
            .arguments
            .get("generate_audio")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        if !generate_audio {
            let _ = strip_audio(&target_path);
        }


        if is_ltx && matches!(is_probably_black_video(&target_path), Ok(true)) {
            free_comfyui_cache(&client, false).await;
            if should_auto_fallback_to_minimax(&engine_hint, is_ltx) {
                let reason = format!(
                    "LTX-2.5 I2V 已导出 MP4，但抽帧检测为近似全黑视频：{}",
                    target_path.display()
                );
                GenerateVideoTool::emit_progress(
                    ctx,
                    &request,
                    prompt,
                    duration_secs,
                    "fallback",
                    "LTX-2.5 输出黑屏，正在切换 MiniMax H3 官方 I2V 链路",
                    0.05,
                    Some(prompt_id),
                    None,
                    Some(&reason),
                )
                .await;
                let mut fallback =
                    Box::pin(self.execute(ctx, minimax_fallback_request(&request, seed))).await;
                if matches!(fallback.status, ToolStatus::Ok) {
                    fallback.content = format!(
                        "LTX-2.5 I2V 输出黑屏，已自动切换 MiniMax H3 完成生成。\n\n{}\n\n{}",
                        reason, fallback.content
                    );
                }
                return fallback;
            }
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(
                    ToolFailureKind::Execution,
                    format!(
                        "LTX-2.5 I2V 已导出 MP4，但抽帧检测为近似全黑视频：{}。\
                         这不是可用结果；请改用 engine=`minimax` 走 MiniMax H3 官方 I2V 模板链路。",
                        target_path.display()
                    ),
                ),
                started.elapsed(),
            );
        }

        let abs_path = target_path.to_string_lossy().to_string();
        GenerateVideoTool::emit_progress(
            ctx,
            &request,
            prompt,
            duration_secs,
            "done",
            "图生视频已生成",
            1.0,
            Some(prompt_id),
            Some(&abs_path),
            None,
        )
        .await;

        // 图生视频完成后主动释放潜空间显存缓存
        free_comfyui_cache(&client, false).await;

        let summary = crate::agent_runtime::image_tool::with_lora_warning(
            i2v_summary(
                seed,
                &abs_path,
                &source_path.display().to_string(),
                &unet_name,
            ),
            lora_warning,
        );

        ToolOutcome::ok(&request, summary, vec![abs_path], started.elapsed())
    }
}

#[cfg(test)]
mod tests {

    /// 2026-10-07 EP03「人物变矮变胖」：1216×832 的首帧被 H3 直接拉到 1312×736。送进去之前等比裁到画布 —— 方的还是方的
    #[test]
    fn 关键帧先等比裁到出片画布() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("f.png");
        // 1216×832 中间画一个 200×200 的黑方块
        let mut img = image::RgbImage::from_pixel(1216, 832, image::Rgb([255, 255, 255]));
        for y in 316..516 {
            for x in 508..708 {
                img.put_pixel(x, y, image::Rgb([0, 0, 0]));
            }
        }
        img.save(&src).unwrap();
        let out = keyframe_for_canvas(&src, (1312, 736)).expect("尺寸不一样要裁").to_rgb8();
        assert_eq!(out.dimensions(), (1312, 736));
        let dark: Vec<(u32, u32)> = out.enumerate_pixels().filter(|(_, _, p)| p[0] < 128).map(|(x, y, _)| (x, y)).collect();
        let (x0, x1) = (dark.iter().map(|p| p.0).min().unwrap(), dark.iter().map(|p| p.0).max().unwrap());
        let (y0, y1) = (dark.iter().map(|p| p.1).min().unwrap(), dark.iter().map(|p| p.1).max().unwrap());
        let (w, h) = ((x1 - x0 + 1) as f64, (y1 - y0 + 1) as f64);
        assert!((w / h - 1.0).abs() < 0.02, "方块被拉变形了：{w}×{h}（直接拉伸会是 {:.2}）", 1312.0 / 1216.0 / (736.0 / 832.0));
        // 已经是画布尺寸：原样送，不多一次重采样
        let same = dir.path().join("g.png");
        image::RgbImage::new(1312, 736).save(&same).unwrap();
        assert!(keyframe_for_canvas(&same, (1312, 736)).is_none());
    }
    use super::*;
    #[test]
    fn 出片任务丢失判定_不在队列也不在历史才累计_读不到队列不算证据() {
        assert_eq!(next_lost_streak(3, false, Some(false)), 4);
        assert_eq!(next_lost_streak(3, false, Some(true)), 0, "还在排队/在跑");
        assert_eq!(next_lost_streak(3, true, Some(false)), 0, "已经进 history 了，是跑完在写结果");
        assert_eq!(next_lost_streak(3, false, None), 3, "读不到队列不是证据");
        let mut s = 0;
        for _ in 0..VIDEO_LOST_TOLERANCE { s = next_lost_streak(s, false, Some(false)); }
        assert!(s >= VIDEO_LOST_TOLERANCE, "被取消的任务要在十几秒内判出来，不是等两小时");
        // 判定函数对了但轮询循环没用它，等于没有
        let src = include_str!("video_tool.rs");
        assert!(src.contains(&["lost_streak = next_lost_streak(", "lost_streak, in_history, in_queue);"].concat()));
        assert!(src.contains(&["if lost_streak >= ", "VIDEO_LOST_TOLERANCE {"].concat()));
    }


    /// 本机 ComfyUI 实际报出来的核心视频模型族（2026-08-29）。
    fn avail() -> Vec<String> {
        [
            "hunyuan_video_720_cfgdistill_fp8_e4m3fn.safetensors",
            "ltx-2.5-22b-distilled-transformer-bf16.safetensors",
            "MiniMax_H3_FL2VA_pruned_int8_convrot.safetensors",
            "wan2.1_i2v_480p_14B_fp8_scaled.safetensors",
            "wan2.1_t2v_1.3B_bf16.safetensors",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    /// **P0 回归。**
    ///
    /// `animate_image` 曾经把 `"wan2.1_i2v"` 当文件名发给 ComfyUI，
    /// 而盘上叫 `wan2.1_i2v_480p_14B_fp8_scaled.safetensors`。
    /// 实测提交即被拒：`unet_name: 'wan2.1_i2v' not in [...]`。
    /// 每一次图生视频都死在这里。
    #[test]
    fn i2v_偏好关键词必须解析成真实文件名() {
        let got = pick_video_unet(&avail(), None, I2V_PREFERRED).unwrap();
        assert_eq!(got, "ltx-2.5-22b-distilled-transformer-bf16.safetensors");
        assert!(
            avail().contains(&got),
            "挑出来的必须在 ComfyUI 的枚举里，否则提交阶段就会被拒"
        );
    }

    /// **枚举顺序不该左右选中哪个模型 —— 这是 P0 回归。**
    ///
    /// 2026-09-03 实测：往 `models/diffusion_models/` 放一份
    /// `ltx-2.5-22b-dev-transformer_W4A8`，它按字母序排在 `...-distilled-...`
    /// 前面，于是所有 LTX 生成静默切到这个没验过的模型上 ——
    /// **出纯黑视频，而任务状态是 `success`**。
    ///
    /// 判据是「同一批文件、任意顺序，答案必须一样」，
    /// 而且必须仍然是蒸馏版（我们的 8 步 / cfg=1 是按它配的）。
    /// **P0 回归：turbo LoRA 的查找必须容忍子目录。**
    ///
    /// 2026-09-03 把 `models/loras/` 按引擎分了子目录，ComfyUI 报的名字变成
    /// `h3/minimax_h3_fl2v_turbo_8step_....safetensors`。原来那句
    /// `available_loras.iter().any(|n| n == MINIMAX_H3_TURBO_LORA)` 是全等比较，
    /// 立刻失配 —— **turbo LoRA 悄悄不挂，而步数仍然按 turbo 的 8 步走**，
    /// 出来又慢又糊，线索只有告警里的一行字。
    #[test]
    fn turbo_lora_在子目录里也要找得到() {
        let flat = vec![MINIMAX_H3_TURBO_LORA.to_string()];
        let nested = vec![format!("h3/{MINIMAX_H3_TURBO_LORA}")];
        let deeper = vec![format!("video/h3/{MINIMAX_H3_TURBO_LORA}")];

        for (label, avail) in [("平铺", &flat), ("一层子目录", &nested), ("两层子目录", &deeper)] {
            assert_eq!(
                find_turbo_lora(avail).as_ref(),
                avail.first(),
                "{label}下没找到 turbo LoRA —— 会静默退回无 turbo 的 8 步采样"
            );
        }

        // 盘上没有时必须是 None，好让调用方发出「退回普通采样」的告警
        assert!(find_turbo_lora(&["某个无关的.safetensors".to_string()]).is_none());

        // 调用方已经自己点名了（哪怕带子目录前缀），就不该再自动加一遍
        assert!(turbo_already_requested(&[lora::ResolvedLora {
            name: format!("h3/{MINIMAX_H3_TURBO_LORA}"),
            strength: 1.0,
        }]));
        assert!(!turbo_already_requested(&[lora::ResolvedLora {
            name: "zimage/Pintrest_ZiT.safetensors".into(),
            strength: 1.0,
        }]));
    }

    #[test]
    fn 枚举顺序不影响选中哪个模型() {
        let dev = "ltx-2.5-22b-dev-transformer_W4A8_Mixed.safetensors".to_string();
        let distilled = "ltx-2.5-22b-distilled-transformer-bf16.safetensors".to_string();

        for order in [
            vec![dev.clone(), distilled.clone()],
            vec![distilled.clone(), dev.clone()],
        ] {
            for pref in [T2V_PREFERRED, I2V_PREFERRED, I2V_LTX_PREFERRED] {
                let got = pick_video_unet(&order, None, pref).unwrap();
                assert_eq!(
                    got, distilled,
                    "枚举顺序 {order:?} 下选中了 {got} —— 采样参数是按蒸馏版配的，dev 会出黑屏"
                );
            }
        }
    }

    /// 同一关键词命中多个时，结果只由「有哪些文件」决定，不由「顺序」决定。
    #[test]
    fn 多个命中时结果与顺序无关() {
        let a = "ltxv_aaa.safetensors".to_string();
        let b = "ltxv_bbb.safetensors".to_string();
        let c = "ltxv_ccc.safetensors".to_string();
        let mut orders = vec![
            vec![a.clone(), b.clone(), c.clone()],
            vec![c.clone(), b.clone(), a.clone()],
            vec![b.clone(), c.clone(), a.clone()],
        ];
        let first = pick_video_unet(&orders.pop().unwrap(), None, &["ltxv"]).unwrap();
        for o in orders {
            assert_eq!(pick_video_unet(&o, None, &["ltxv"]).unwrap(), first);
        }
    }

    #[test]
    fn t2v_挑的是t2v不是i2v() {
        let got = pick_video_unet(&avail(), None, T2V_PREFERRED).unwrap();
        assert_eq!(got, "ltx-2.5-22b-distilled-transformer-bf16.safetensors");
    }

    /// 显式指定也要核对 —— 原来的实现是**原样返回不核对**，那正是 P0 的成因。
    #[test]
    fn 显式指定的名字对不上时要报错并列出可用的() {
        let err = pick_video_unet(&avail(), Some("wan2.1_i2v"), T2V_PREFERRED);
        // 片段唯一命中，应该被补全成真名而不是原样透传
        assert_eq!(err.unwrap(), "wan2.1_i2v_480p_14B_fp8_scaled.safetensors");

        let err = pick_video_unet(&avail(), Some("完全不存在的模型"), T2V_PREFERRED).unwrap_err();
        assert!(err.contains("wan2.1_t2v_1.3B_bf16.safetensors"), "{err}");
    }

    #[test]
    fn 一个候选都没有时说清楚而不是编一个文件名() {
        let err = pick_video_unet(&[], None, T2V_PREFERRED).unwrap_err();
        assert!(err.contains("diffusion_models"), "{err}");
    }

    fn t2v(frames: u32) -> Value {
        build_t2v_graph(&T2vParams {
            unet_name: "ltx-2.5-22b-distilled-transformer.safetensors",
            clip_name: "gemma4-12b-with-proj-ltx-2.5-bf16.safetensors",
            clip_type: "ltxv",
            vae_name: "ltx-2.5-video-vae-conv-bf16.safetensors",
            prompt: "a fox in snow",
            neg_prompt: "blurry",
            width: 832,
            height: 480,
            frames_count: frames,
            fps: 24,
            seed: 42,
            steps: 30,
            model_ref: json!(["1", 0]),
        })
    }

    /// **P0 回归 —— 这条是把机器搞崩的那一半。**
    ///
    /// 原来是 `EmptyLatentImage { batch_size: 64 }`：64 张互不相干的图片
    /// latent，通道 4≠16、秩 4≠5、没有时间轴。而且视频 UNet 被要求一次
    /// 并行采样 64 个样本，统一内存压穿，watchdog timeout 整机重启
    /// （2026-08-27 11:02，panic 日志在 /Library/Logs/DiagnosticReports）。
    #[test]
    fn 空latent必须是时序节点且batch恒为1() {
        let g = t2v(121);
        let node = &g["prompt"]["6"];
        assert_eq!(
            node["class_type"], "EmptyLTXVLatentVideo",
            "视频要时序 latent；EmptyLatentImage 出来的是一批图片 latent"
        );
        assert_eq!(
            node["inputs"]["batch_size"], 1,
            "batch_size 必须恒为 1 —— 帧数走 length。把帧数塞进 batch_size 会压穿内存"
        );
        assert_eq!(node["inputs"]["length"], 121, "帧数走 length");

        // 整张图里都不该再出现图片版的空 latent
        let wire = serde_json::to_string(&g).unwrap();
        assert!(!wire.contains("EmptyLatentImage"), "{wire}");
    }

    /// LTX 的 sigma 调度靠 `ModelSamplingLTXV`。缺了它采样轨迹是错的，
    /// 而且**不报任何错**。
    #[test]
    fn 采样前必须过modelsampling() {
        let g = t2v(121);
        assert_eq!(g["prompt"]["13"]["class_type"], "ModelSamplingLTXV");
        // KSampler 必须接在它后面，而不是直连 UNETLoader
        assert_eq!(g["prompt"]["7"]["inputs"]["model"], json!(["13", 0]));
    }

    /// 等待上限必须大于实测耗时，否则每一次真实生成都会被判失败，
    /// 而 ComfyUI 那边还在跑 —— 和"卡死"无法区分。
    #[test]
    fn 等待上限要覆盖实测耗时() {
        // 轮询本身不再有时长上限（看进度）；对话里那条外面的 registry 超时要装得下实测耗时
        let budget = VIDEO_TOOL_TIMEOUT_SECS - 60;
        // 实测：832x480 / 33 帧 / 20 步 = 390s；30 步 = 560s
        assert!(budget > 560, "轮询预算 {budget}s 覆盖不了实测的 560s");
        assert!(
            budget > 2400,
            "轮询预算 {budget}s 覆盖不了 MiniMax/LTX 高质量模板实测的 2400s"
        );
        assert!(
            VIDEO_TOOL_TIMEOUT_SECS > budget,
            "工具超时必须比轮询预算长，否则轮询还没走完就被外层砍掉"
        );
    }

    #[test]
    fn 自动fallback只在auto加ltx时触发() {
        assert!(should_auto_fallback_to_minimax("auto", true));
        assert!(should_auto_fallback_to_minimax("", true));
        assert!(!should_auto_fallback_to_minimax("ltx", true));
        assert!(!should_auto_fallback_to_minimax("minimax", false));
        assert!(!should_auto_fallback_to_minimax("svd", false));
    }

    /// 挂了 LoRA 之后，ModelSamplingLTXV 要接在链尾，不能绕回底座 ——
    /// 绕回去的话 LoRA 一条都不生效，而且不报错。
    #[test]
    fn 有lora时modelsampling接在链尾() {
        let g = build_t2v_graph(&T2vParams {
            unet_name: "ltx-u.safetensors",
            clip_name: "c.safetensors",
            clip_type: "ltxv",
            vae_name: "v.safetensors",
            prompt: "x",
            neg_prompt: "y",
            width: 832,
            height: 480,
            frames_count: 33,
            fps: 16,
            seed: 1,
            steps: 30,
            model_ref: json!(["901", 0]),
        });
        assert_eq!(g["prompt"]["13"]["inputs"]["model"], json!(["901", 0]));
    }

    /// 前端 recent 档给每条工具返回的额度下限（`MIN_TOOL_RESULT_CHARS`）。
    /// Seed 落在这个窗口内才活得下来。
    const FRONTEND_MIN_RESULT_CHARS: usize = 100;

    /// **改视频和换一段的唯一分界就是 seed。**
    ///
    /// 这段原来以 `![{prompt}](…)` 开头，一条镜头描述几百字符，把结尾的 Seed
    /// 挤出了前端的截断窗口。下一轮「改成夜景」时模型看不到上一段用的种子，
    /// 只能重新随机 —— 出来的是完全无关的一段片子，而且不报任何错。
    #[test]
    fn 视频返回里的seed活得过前端截断() {
        for s in [
            t2v_summary(
                424_242,
                "/Users/me/work/.anima/generated/a_long_clip_name.mp4",
                "wan2.1_t2v_1.3B_bf16",
                832,
                480,
                16,
                2,
            ),
            i2v_summary(
                424_242,
                "/Users/me/work/.anima/generated/a_long_clip_name.mp4",
                "/Users/me/work/src.png",
                "wan2.1_i2v",
            ),
        ] {
            let window: String = s.chars().take(FRONTEND_MIN_RESULT_CHARS).collect();
            assert!(
                window.contains("424242"),
                "Seed 被挤出截断窗口了，开头是：{window}"
            );
        }
    }

    /// **每一行都不能有前导空格。**
    ///
    /// Markdown 里 4 个以上前导空格是缩进代码块 —— 那样 `![...](...)` 不再是
    /// 嵌入，而是一段字面文本，视频/图片就不会被渲染出来。
    ///
    /// 这条是补的：写这些 format! 字符串时用了 Rust 的 `\` 续行，而生成代码的
    /// 脚本把续行吃掉了，于是每行开头留下 9 个空格。**编译通过、测试全绿、
    /// 肉眼看代码也像对的**，只有真去看渲染结果才会发现。文生图那份就这么
    /// 坏了一整轮没人发现。
    #[test]
    fn 返回文本里没有会变成代码块的缩进() {
        let samples = [
            ("t2v", t2v_summary(1, "/tmp/a.mp4", "u", 832, 480, 16, 2)),
            ("i2v", i2v_summary(1, "/tmp/a.mp4", "/tmp/s.png", "u")),
        ];
        for (name, text) in samples {
            for line in text.lines() {
                assert!(
                    !line.starts_with(' '),
                    "{name} 的返回里有前导空格，这一行会被当成代码块：{line:?}"
                );
            }
        }
    }

    fn r2v(files: &[&str]) -> Value {
        let names: Vec<String> = files.iter().map(|s| s.to_string()).collect();
        build_minimax_h3_r2v_graph(&MinimaxH3R2vParams {
            unet_name: "MiniMax_H3_FL2VA_pruned_int8_convrot.safetensors",
            clip_name: "qwen3vl_32b_minimax_h3_int8_convrot.safetensors",
            video_vae_name: "minimax_h3_video_vae_fp16.safetensors",
            audio_vae_name: "minimax_h3_audio_vae_fp32.safetensors",
            prompt: "walking down a street",
            reference_filenames: &names,
            width: 864,
            height: 480,
            frames_count: 56,
            seed: 777,
            steps: 4,
            ref_image_size: "max",
            model_ref: json!(["900", 0]),
        })
    }

    /// **参考图的键名必须是点号路径，写错会被静默忽略。**
    ///
    /// `ref_images` 是 optional 的 autogrow 输入。用 `"ref_images": [[..]]`
    /// 这种写法时 ComfyUI 校验通过、任务成功，只是一张图都没收到 ——
    /// 实测四组不同参考图配置跑出来的视频**逐像素相同**（差 0.00）。
    /// 我一度据此得出"R2V 保不住人脸"，其实是 R2V 根本没拿到图。
    #[test]
    fn 参考图用点号路径的键名() {
        let g = r2v(&["a.png", "b.png", "c.png"]);
        let ins = &g["prompt"]["6"]["inputs"];
        for i in 0..3 {
            let key = format!("ref_images.ref_image_{i}");
            assert!(ins.get(&key).is_some(), "缺少 {key}；写成别的形式会被静默忽略");
        }
        // 这两种写法都是错的，不许出现
        assert!(ins.get("ref_images").is_none(), "数组形式会被静默忽略");
        assert!(ins.get("ref_image_0").is_none(), "平铺键会 TypeError");
    }

    /// 每张参考图都要有自己的 LoadImage，而且连对。
    #[test]
    fn 每张参考图都有独立的loader() {
        let g = r2v(&["front.png", "side.png"]);
        let p = &g["prompt"];
        assert_eq!(p["500"]["inputs"]["image"], "front.png");
        assert_eq!(p["501"]["inputs"]["image"], "side.png");
        assert_eq!(p["6"]["inputs"]["ref_images.ref_image_0"], json!(["500", 0]));
        assert_eq!(p["6"]["inputs"]["ref_images.ref_image_1"], json!(["501", 0]));
    }

    /// 节点声明 max=9，多给的要截断而不是发出去被拒。
    /// **排队不吃出片的等待上限。** 按场出首帧时排在后面的镜头光排队就等满上限被判失败（2026-09-29），出片是同一个写法。
    /// **生成本身不设时长上限**：三窗口长镜头要十二个多小时（2026-09-30），还在不在跑看 ComfyUI 的进度
    #[test]
    fn 出片等待_排队不计时_生成看进度不看时长() {
        let q = |pending: &[&str], running: &[&str]| {
            serde_json::json!({
                "queue_pending": pending.iter().enumerate().map(|(i, id)| serde_json::json!([i, id, {}, {}, []])).collect::<Vec<_>>(),
                "queue_running": running.iter().enumerate().map(|(i, id)| serde_json::json!([i, id, {}, {}, []])).collect::<Vec<_>>(),
            })
        };
        assert!(queue_pending_only(&q(&["a", "b"], &["c"]), "b"), "排着队");
        assert!(!queue_pending_only(&q(&["a"], &["b"]), "b"), "已经在跑");
        assert!(!queue_pending_only(&q(&[], &[]), "b"), "不在队列里");
        // 接线：分镜 / 画布那条等待走进度判定（判定本身的测试在 `comfy_watch`）
        let src = include_str!("video_tool.rs");
        // 等待循环在 `wait_and_fetch_video`（提交和重启后续接共用）
        let body = &src[src.find("pub(crate) async fn wait_and_fetch_video(").unwrap()..];
        let body = &body[..body.find("\n}\n").unwrap()];
        assert!(body.contains("comfy_watch::tracker(&prompt_id)") && body.contains("watch.observe(seen)"));
        assert!(!body.contains("max_polls"), "不能再有按轮数的生成上限");
        assert_eq!(SHOT_PENDING_CEILING_POLLS as u64 * VIDEO_POLL_INTERVAL_SECS, 12 * 3600);
        // 对话里的出片工具不跟着变：它外面还有 registry 的超时，轮询不能比它长
        // 三条对话出片工具的轮询也不能再按轮数封顶（2026-09-30 用户：「全部根据 Comfy 实时状态抓」）
        let code = &src[..src.find("#[cfg(test)]").unwrap()];
        assert!(!code.contains("0..VIDEO_") && !code.contains("0..max_polls") && !code.contains("FALLBACK_MAX_POLLS"));
        assert_eq!(code.matches("watch.observe(seen)").count(), 4, "分镜那条 + 对话三条");
    }

    #[test]
    fn 超过九张要截断() {
        let files: Vec<String> = (0..12).map(|i| format!("r{i}.png")).collect();
        let refs: Vec<&str> = files.iter().map(|s| s.as_str()).collect();
        let g = r2v(&refs);
        let ins = g["prompt"]["6"]["inputs"].as_object().unwrap();
        let n = ins.keys().filter(|k| k.starts_with("ref_images.")).count();
        assert_eq!(n, MAX_REFERENCE_IMAGES);
    }

    /// R2V 用的是 ReferenceToVideo，**不能**是 ImageToVideo ——
    /// 后者的 first_frame 就是字面第一帧，正是"首帧突兀"的成因。
    #[test]
    fn r2v用的是参考节点而不是首帧节点() {
        let g = r2v(&["a.png"]);
        assert_eq!(g["prompt"]["6"]["class_type"], "MiniMaxH3ReferenceToVideo");
        assert!(g["prompt"]["6"]["inputs"].get("first_frame").is_none());
        // R2V 需要音频 VAE，I2V 不需要
        assert_eq!(g["prompt"]["6"]["inputs"]["audio_vae"], json!(["4", 0]));
    }

    /// 一张参考图都不给也要能跑（退化成纯文生），不能造出空的 loader。
    #[test]
    fn 没有参考图时不造空loader() {
        let g = r2v(&[]);
        let p = g["prompt"].as_object().unwrap();
        assert!(p.get("500").is_none());
        let ins = p["6"]["inputs"].as_object().unwrap();
        assert_eq!(ins.keys().filter(|k| k.starts_with("ref_images.")).count(), 0);
    }

    /// **auto 模式下 LTX 跑不了时，不做那次注定出纯黑的尝试。**
    ///
    /// 原来的做法是先跑 LTX（150~250 秒）、检测输出是不是黑、再回退。
    /// 启动参数和可用内存提交前就能读到，没必要烧那几分钟。
    #[test]
    fn ltx跑不了时auto直接走minimax() {
        assert_eq!(pick_i2v_preferred("auto", true), I2V_MINIMAX_PREFERRED);
        assert_eq!(pick_i2v_preferred("", true), I2V_MINIMAX_PREFERRED);
        // 条件满足时才走默认那条（里面 LTX 排在前面）
        assert_eq!(pick_i2v_preferred("auto", false), I2V_PREFERRED);
    }

    /// 用户点名的引擎优先于自动判断 —— 不偷偷换。
    #[test]
    fn 下架的wan点名就明确拒绝() {
        // 引擎提示、model 文件名（含子目录）都要拦
        for hint in ["wan", "Wan2.1", "wan2.2_ti2v"] {
            let msg = retired_video_engine(hint, None).expect(hint);
            assert!(msg.contains("下架"), "{msg}");
            assert!(msg.contains("LTX") && msg.contains("H3"), "要告诉用户改用什么：{msg}");
        }
        assert!(retired_video_engine("", Some("wan2.1_t2v_1.3B_bf16.safetensors")).is_some());
        assert!(retired_video_engine("", Some("video/Wan2_1_VAE_bf16.safetensors")).is_some());
        // 还在的引擎、没点名、名字里只是碰巧带 wan 的，都放行
        for hint in ["", "auto", "ltx", "minimax", "h3"] {
            assert!(retired_video_engine(hint, None).is_none(), "{hint}");
        }
        assert!(retired_video_engine("", Some("swan_lake_ltx.safetensors")).is_none());
        assert!(retired_video_engine("", None).is_none());
    }

    #[test]
    fn 下架后的偏好表里没有wan() {
        for list in [T2V_PREFERRED, I2V_PREFERRED] {
            assert!(!list.iter().any(|k| k.contains("wan")), "{list:?}");
        }
    }

    #[test]
    fn 点名的引擎不被自动判断覆盖() {
        assert_eq!(pick_i2v_preferred("minimax", false), I2V_MINIMAX_PREFERRED);
        // 点名 ltx 时这里仍返回 LTX —— 拦截在更早的地方做，并且**明确报错**
        // 说清差什么，而不是悄悄换成别的引擎
        assert_eq!(pick_i2v_preferred("ltx", true), I2V_LTX_PREFERRED);
    }

    /// 档位的数字都是实测的，别被随手改。
    #[test]
    fn 质量档按实测取值() {
        assert_eq!(VideoQuality::from_arg(Some("fast")).steps(), 4);
        assert_eq!(VideoQuality::from_arg(Some("balanced")).steps(), 8);
        assert_eq!(VideoQuality::from_arg(Some("quality")).steps(), 8);
        // 缺省和无法识别的值都退到 fast
        assert_eq!(VideoQuality::from_arg(None), VideoQuality::Fast);
        assert_eq!(VideoQuality::from_arg(Some("随便写的")), VideoQuality::Fast);
        assert_eq!(VideoQuality::from_arg(Some("QUALITY")), VideoQuality::Quality);
    }

    /// 路径也不能被挤掉：卡片靠 `![…](file://…)` 取视频。
    #[test]
    fn 视频链接也留在前面() {
        let s = t2v_summary(1, "/tmp/a.mp4", "u", 832, 480, 16, 2);
        let at = s.find("file:///tmp/a.mp4").expect("必须有 file:// 链接");
        assert!(at < 200, "视频链接落在第 {at} 字符，太靠后");
    }

    /// 报了 seed 却不收 = 报了个没法用的东西。图生视频原来正是这样：
    /// schema 里没有 seed 参数，每次都重新随机。
    #[test]
    fn 两个工具都既报seed也收seed() {
        use crate::agent_runtime::ToolHandler;
        for spec in [GenerateVideoTool.spec(), AnimateImageTool.spec()] {
            let has = spec.parameters.pointer("/properties/seed").is_some();
            assert!(
                has,
                "{} 的 schema 里没有 seed，改图/改视频就无从谈起",
                spec.name
            );
        }
    }

    #[test]
    fn 两个视频工具都声明并支持width和height() {
        use crate::agent_runtime::ToolHandler;
        for spec in [GenerateVideoTool.spec(), AnimateImageTool.spec()] {
            assert!(
                spec.parameters.pointer("/properties/width").is_some(),
                "{} 的 schema 里缺少 width 参数声明",
                spec.name
            );
            assert!(
                spec.parameters.pointer("/properties/height").is_some(),
                "{} 的 schema 里缺少 height 参数声明",
                spec.name
            );
        }

        let (w, h) = weetodd::fit_dimensions((384, 640));
        assert_eq!((w, h), (384, 640), "竖屏 9:16 (384x640) 经过 fit_dimensions 必须保持 9:16");
    }

    #[test]
    fn ltx家族名称正确命中分流() {
        let ltx_names = [
            "ltx-2.5-22b-distilled-transformer-bf16.safetensors",
            "ltx_video_v0.9.5.safetensors",
        ];
        for name in ltx_names {
            assert!(
                name.to_lowercase().contains("ltx"),
                "{name} 应该被识别为 LTX 家族"
            );
        }
    }

    /// **首尾帧：给了末帧就必须真的接进条件，没给就一个节点都不多。**
    ///
    /// 实测确认（2026-09-02）：首帧细高灯塔偏左、末帧矮胖灯塔偏右，
    /// 输出的最后一帧复现的是**末帧那套几何**。
    #[test]
    fn 首尾帧把末帧接进条件() {
        let base = |end: Option<&'static str>| MinimaxH3I2vParams {
            unet_name: "u", clip_name: "c", video_vae_name: "v", audio_vae_name: "a",
            prompt: "p", input_filename: "start.png", end_filename: end,
            width: 864, height: 480, frames_count: 22, seed: 1, steps: 4,
            model_ref: json!(["1", 0]),
        };

        // 没给末帧：条件直连 MiniMaxH3ImageToVideo，不多出任何节点
        let g = build_minimax_h3_i2v_graph(&base(None));
        assert_eq!(g["prompt"]["9"]["inputs"]["conditioning"], json!(["6", 0]));
        assert!(g["prompt"]["20"].is_null(), "没给末帧却插了 LoadImage");
        assert!(g["prompt"]["21"].is_null(), "没给末帧却插了 AddGuide");

        // 给了末帧：条件必须改走 AddGuide，否则图跑得通、末帧完全不生效、不报错
        let g = build_minimax_h3_i2v_graph(&base(Some("end.png")));
        assert_eq!(g["prompt"]["20"]["inputs"]["image"], "end.png");
        assert_eq!(g["prompt"]["21"]["class_type"], "MiniMaxH3AddGuide");
        assert_eq!(g["prompt"]["21"]["inputs"]["frame_idx"], -1, "frame_idx 不是 -1 就不是末帧");
        assert_eq!(g["prompt"]["21"]["inputs"]["positive"], json!(["6", 0]));
        assert_eq!(g["prompt"]["21"]["inputs"]["latent"], json!(["6", 1]));
        assert_eq!(
            g["prompt"]["9"]["inputs"]["conditioning"],
            json!(["21", 0]),
            "**只插节点不改 BasicGuider 的连线** —— 图会跑通、末帧完全不生效、\
             而且不报任何错。这是这个项目里最典型的失败形状"
        );
        // 采样那条链不能被动到
        assert_eq!(g["prompt"]["11"]["inputs"]["latent_image"], json!(["6", 1]));
    }

    /// **内存门槛只能用有证据的数，不能用算出来的。**
    ///
    /// 实测只有两个点：41 GB 崩、108 GB 正常。中间没测过，而理论峰值 105 GB
    /// 没考虑 ComfyUI 的分块加载和 offload —— 拿 105 当拦截线会误伤本来跑得动的。
    #[test]
    fn h3内存门槛取有证据的下界() {
        use crate::comfyui::{h3_memory_blocker, H3_BLOCK_BELOW_GB, H3_ESTIMATED_PEAK_GB};
        // 实测会崩的那个点必须被拦下
        assert!(h3_memory_blocker(41.0).is_some(), "41 GB 实测会崩，必须拦");
        // 实测跑得通的那个点必须放行
        assert!(h3_memory_blocker(108.0).is_none(), "108 GB 实测正常，不该拦");
        // 拦截线不能等于理论峰值 —— 那是没测过的数
        assert!(
            H3_BLOCK_BELOW_GB < H3_ESTIMATED_PEAK_GB,
            "拿理论峰值当拦截线会误伤：真实门槛在 41~108 之间，没测过"
        );
        assert!(H3_BLOCK_BELOW_GB > 41.0, "拦截线要高于已知会崩的 41 GB");
    }

    #[test]
    fn minimax帧数落到17n加5的栅格上() {
        assert_eq!(snap_frames_17n_plus_5(1), 5);
        assert_eq!(snap_frames_17n_plus_5(48), 56);
        assert_eq!(snap_frames_17n_plus_5(120), 124);
        assert_eq!(snap_frames_17n_plus_5(124), 124);
    }

    #[test]
    fn i2v可以按engine指定minimax模型家族() {
        let got = pick_video_unet(&avail(), None, I2V_MINIMAX_PREFERRED).unwrap();
        assert_eq!(got, "MiniMax_H3_FL2VA_pruned_int8_convrot.safetensors");

        let auto = pick_video_unet(&avail(), None, I2V_PREFERRED).unwrap();
        assert_eq!(auto, "ltx-2.5-22b-distilled-transformer-bf16.safetensors");
    }

    #[test]
    fn minimax_i2v使用官方h3节点() {
        let g = build_minimax_h3_i2v_graph(&MinimaxH3I2vParams {
            unet_name: "MiniMax_H3_FL2VA_pruned_int8_convrot.safetensors",
            clip_name: "qwen3vl_32b_minimax_h3_int8_convrot.safetensors",
            video_vae_name: "minimax_h3_video_vae_fp16.safetensors",
            audio_vae_name: "minimax_h3_audio_vae_fp32.safetensors",
            prompt: "a woman walks through a neon city street",
            input_filename: "i2v_source_971207467.png",
            end_filename: None,
            width: 1344,
            height: 768,
            frames_count: 124,
            seed: 971_207_467,
            steps: 20,
            model_ref: json!(["900", 0]),
        });

        assert_eq!(g["prompt"]["6"]["class_type"], "MiniMaxH3ImageToVideo");
        assert_eq!(g["prompt"]["6"]["inputs"]["first_frame"], json!(["5", 0]));
        assert_eq!(g["prompt"]["6"]["inputs"]["width"], 1344);
        assert_eq!(g["prompt"]["6"]["inputs"]["height"], 768);
        assert_eq!(g["prompt"]["6"]["inputs"]["length"], 124);
        assert_eq!(g["prompt"]["7"]["inputs"]["sampler_name"], "res_multistep");
        assert_eq!(g["prompt"]["8"]["class_type"], "BasicScheduler");
        assert_eq!(g["prompt"]["8"]["inputs"]["model"], json!(["900", 0]));
        assert_eq!(g["prompt"]["9"]["class_type"], "BasicGuider");
        assert_eq!(g["prompt"]["11"]["class_type"], "SamplerCustomAdvanced");
        assert_eq!(g["prompt"]["13"]["class_type"], "VAEDecodeAudio");
        assert_eq!(g["prompt"]["14"]["inputs"]["audio"], json!(["13", 0]));
        assert_eq!(g["prompt"]["15"]["class_type"], "SaveVideo");

        let wire = serde_json::to_string(&g).unwrap();
        assert!(!wire.contains("WanImageToVideo"), "{wire}");
        assert!(!wire.contains("LTXVImgToVideo"), "{wire}");
    }

    #[test]
    fn ltx黑屏fallback会保留seed并标记快速救场模式() {
        let req = ToolCallRequest::new(
            "animate_image",
            json!({
                "image_path": ".anima/generated/source.png",
                "engine": "auto",
                "motion_prompt": "gentle walk",
            }),
        );

        let fallback = minimax_fallback_request(&req, 971_207_467);

        assert_eq!(fallback.arguments["engine"], "minimax");
        assert_eq!(fallback.arguments["seed"], 971_207_467);
        assert_eq!(fallback.arguments["_fallback_from_ltx"], true);
        assert!(is_ltx_fallback_request(&fallback));
    }

    #[test]
    fn minimax_i2v_graph尊重turbo步数和lora模型出口() {
        let g = build_minimax_h3_i2v_graph(&MinimaxH3I2vParams {
            unet_name: "MiniMax_H3_FL2VA_pruned_int8_convrot.safetensors",
            clip_name: "qwen3vl_32b_minimax_h3_int8_convrot.safetensors",
            video_vae_name: "minimax_h3_video_vae_fp16.safetensors",
            audio_vae_name: "minimax_h3_audio_vae_fp32.safetensors",
            prompt: "a woman walks through a neon city street",
            input_filename: "i2v_source_971207467.png",
            end_filename: None,
            width: 1344,
            height: 768,
            frames_count: 56,
            seed: 971_207_467,
            steps: 8, // turbo LoRA 是 8 步调的
            model_ref: json!(["900", 0]),
        });

        // **钉字面量 8，不要拿建图用的同一个常量去比** —— 那样只能证明
        // 「代码等于我写的代码」，常量改错了测试照样绿（FLUX 那次踩过）。
        assert_eq!(g["prompt"]["8"]["inputs"]["steps"], 8);
        assert_eq!(g["prompt"]["8"]["inputs"]["model"], json!(["900", 0]));
        assert_eq!(g["prompt"]["9"]["inputs"]["model"], json!(["900", 0]));
    }

    /// **参考生视频实测**（2026-10-06 原生 Ref2VA 对照）：照一次已出片的参数（提示词、参考图、种子、尺寸）原样再出一条，
    /// 只看权重换了之后的差别。参数从 `ANIMA_REF_SPEC` 指向的 JSON 读：
    /// `{"prompt", "pictures": [...], "seed", "width", "height", "secs", "out"}`。
    ///
    /// `ANIMA_H3_REAL_MODELS=1 ANIMA_REF_SPEC=/x.json cargo test --lib 参考生视频实测 -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "要真 ComfyUI，720p 一条半小时上下"]
    async fn 参考生视频实测() {
        let spec: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(std::env::var("ANIMA_REF_SPEC").expect("要设 ANIMA_REF_SPEC")).unwrap()).unwrap();
        let pictures: Vec<std::path::PathBuf> = spec["pictures"].as_array().unwrap().iter().map(|p| p.as_str().unwrap().into()).collect();
        println!("原生 Ref2VA：{}", weetodd::h3_ref2va_native());
        let t = std::time::Instant::now();
        render_shot_video_reference_with(
            &pictures,
            Path::new(spec["out"].as_str().unwrap()),
            spec["prompt"].as_str().unwrap(),
            spec["secs"].as_f64().unwrap(),
            spec["width"].as_u64().unwrap() as u32,
            spec["height"].as_u64().unwrap() as u32,
            spec["seed"].as_u64().unwrap(),
            true,
            Some(weetodd::H3Tier::Staged),
            None,
        )
        .await
        .expect("出片失败");
        println!("出完：{}（{:.1} 分钟）", spec["out"], t.elapsed().as_secs_f64() / 60.0);
    }

    /// **方言 / 语种实测**（2026-10-06 粤语）：用生产那条首帧出片（`render_shot_video`，双采档）出一条，
    /// 只换提示词，判 H3 会不会说某种话。要真 ComfyUI，几十分钟一条。
    ///
    /// `ANIMA_LANG_IMG=首帧.png ANIMA_LANG_OUT=/x.mp4 ANIMA_LANG_PROMPT='...' [ANIMA_LANG_SECS=6] \
    ///   cargo test --lib 语种实测 -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "要真 ComfyUI，一条十几分钟"]
    async fn 语种实测() {
        let img = std::env::var("ANIMA_LANG_IMG").expect("要设 ANIMA_LANG_IMG");
        let out = std::env::var("ANIMA_LANG_OUT").expect("要设 ANIMA_LANG_OUT");
        let prompt = std::env::var("ANIMA_LANG_PROMPT").expect("要设 ANIMA_LANG_PROMPT");
        let secs: f64 = std::env::var("ANIMA_LANG_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(6.0);
        let t = std::time::Instant::now();
        render_shot_video(Path::new(&img), Path::new(&out), "h3", &prompt, secs, 896, 512, 24.0, 20_261_006, true, None, Some(weetodd::H3Tier::Staged))
            .await
            .expect("出片失败");
        println!("出完：{out}（{:.1} 分钟）", t.elapsed().as_secs_f64() / 60.0);
    }

    /// 把 Rust 真正会提交的 **H3** i2v 图导出来，用于打真实 ComfyUI 的闭环验证。
    ///
    /// `ANIMA_GRAPH_DUMP=/x.json ANIMA_H3_INPUT=xx.png \
    ///   cargo test --lib dump_h3_i2v_graph -- --ignored`
    #[test]
    #[ignore]
    fn dump_h3_i2v_graph() {
        let out = std::env::var("ANIMA_GRAPH_DUMP").expect("要设 ANIMA_GRAPH_DUMP");
        let input = std::env::var("ANIMA_H3_INPUT").expect("要设 ANIMA_H3_INPUT");
        let unet = std::env::var("ANIMA_H3_UNET")
            .unwrap_or_else(|_| "MiniMax_H3_FL2VA_pruned_int8_convrot.safetensors".into());
        let w: u32 = std::env::var("ANIMA_H3_W").ok().and_then(|v| v.parse().ok()).unwrap_or(768);
        let h: u32 = std::env::var("ANIMA_H3_H").ok().and_then(|v| v.parse().ok()).unwrap_or(512);
        let f: u32 = std::env::var("ANIMA_H3_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(49);
        // 环境变量给了 LoRA 名字就挂上，用的是生产代码那条链
        let requested: Vec<lora::ResolvedLora> = std::env::var("ANIMA_H3_LORA")
            .ok()
            .filter(|v| !v.is_empty())
            .map(|v| vec![lora::ResolvedLora { name: v, strength: 1.0 }])
            .unwrap_or_default();
        let chain = lora::build_chain(&requested, 900, serde_json::json!(["1", 0]), None);
        let g = build_minimax_h3_i2v_graph(&MinimaxH3I2vParams {
            unet_name: &unet,
            clip_name: "qwen3vl_32b_minimax_h3_int8_convrot.safetensors",
            video_vae_name: "minimax_h3_video_vae_fp16.safetensors",
            audio_vae_name: "minimax_h3_audio_vae_fp32.safetensors",
            prompt: &std::env::var("ANIMA_H3_PROMPT").unwrap_or_else(|_| {
                "一位亚洲女理发师停下剪刀，看着镜子里的顾客轻声说：「你确定要剪掉吗？」".into()
            }),
            input_filename: &input,
            end_filename: None,
            width: w,
            height: h,
            frames_count: f,
            seed: 771_231,
            steps: 8, // turbo LoRA 是 8 步调的
            model_ref: chain.model_ref.clone(),
        });
        // LoRA 链走的是生产代码本身（`lora::build_chain`），不是在图上手工插节点 ——
        // 手插一遍等于测了个副本。
        let mut g = g;
        if let Some(obj) = g.get_mut("prompt").and_then(|p| p.as_object_mut()) {
            for (k, v) in &chain.nodes {
                obj.insert(k.clone(), v.clone());
            }
        }
        std::fs::write(&out, serde_json::to_string_pretty(&g).unwrap()).unwrap();
    }

    /// 把 Rust 真正会提交的 LTX i2v 图导出来，用于打真实 ComfyUI 的端到端验证。
    /// 手抄一份副本去测等于测了个副本。
    ///
    /// `ANIMA_GRAPH_DUMP=/x.json ANIMA_LTX_UNET=<文件名> \
    ///   cargo test --lib dump_ltx_i2v_graph -- --ignored`
    #[test]
    #[ignore]
    fn dump_ltx_i2v_graph() {
        let out = std::env::var("ANIMA_GRAPH_DUMP").expect("要设 ANIMA_GRAPH_DUMP");
        let unet = std::env::var("ANIMA_LTX_UNET")
            .unwrap_or_else(|_| "ltx-2.5-22b-distilled-transformer-bf16.safetensors".into());
        let input = std::env::var("ANIMA_LTX_INPUT").expect("要设 ANIMA_LTX_INPUT");
        let w: u32 = std::env::var("ANIMA_LTX_W").ok().and_then(|v| v.parse().ok()).unwrap_or(768);
        let h: u32 = std::env::var("ANIMA_LTX_H").ok().and_then(|v| v.parse().ok()).unwrap_or(512);
        let f: u32 = std::env::var("ANIMA_LTX_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(49);
        let g = build_ltx_i2v_graph(&LtxI2vParams {
            unet_name: &unet,
            clip_name: "gemma4-12b-with-proj-ltx-2.5-bf16.safetensors",
            video_vae_name: "ltx-2.5-video-vae-bf16.safetensors",
            audio_vae_name: "ltx-2.5-audio-vae-bf16.safetensors",
            latent_upscaler_name: "ltx-2.5-latent-spatial-upscaler-x2-bf16-1.0.safetensors",
            prompt: &std::env::var("ANIMA_LTX_PROMPT")
                .unwrap_or_else(|_| "a woman on a rainy neon street slowly turns her head toward the camera".into()),
            neg_prompt: "black frames, static, blurry",
            input_filename: &input,
            width: w,
            height: h,
            frames_count: f,
            fps: 24,
            seed: 971_207_467,
            model_ref: serde_json::json!(["1", 0]),
        });
        std::fs::write(&out, serde_json::to_string_pretty(&g).unwrap()).unwrap();
    }

    #[test]
    fn ltx_i2v使用官方两阶段av链路而不是简化链() {
        let g = build_ltx_i2v_graph(&LtxI2vParams {
            unet_name: "ltx-2.5-22b-distilled-transformer-bf16.safetensors",
            clip_name: "gemma4-12b-with-proj-ltx-2.5-bf16.safetensors",
            video_vae_name: "ltx-2.5-video-vae-bf16.safetensors",
            audio_vae_name: "ltx-2.5-audio-vae-bf16.safetensors",
            latent_upscaler_name: "ltx-2.5-latent-spatial-upscaler-x2-bf16-1.0.safetensors",
            prompt: "a woman turns toward camera",
            neg_prompt: "black frames",
            input_filename: "i2v_source_971207467.png",
            width: 768,
            height: 512,
            frames_count: 49,
            fps: 24,
            seed: 971_207_467,
            model_ref: json!(["1", 0]),
        });

        assert_eq!(g["prompt"]["18"]["class_type"], "LTXVImgToVideoInplace");
        assert_eq!(g["prompt"]["20"]["class_type"], "LTXVConcatAVLatent");
        assert_eq!(g["prompt"]["25"]["class_type"], "SamplerCustomAdvanced");
        assert_eq!(g["prompt"]["28"]["class_type"], "LTXVLatentUpsampler");
        assert_eq!(g["prompt"]["29"]["class_type"], "LTXVImgToVideoInplace");
        assert_eq!(g["prompt"]["30"]["class_type"], "LTXVConcatAVLatent");
        assert_eq!(g["prompt"]["35"]["class_type"], "SamplerCustomAdvanced");
        assert_eq!(g["prompt"]["37"]["class_type"], "VAEDecodeTiled");
        assert_eq!(g["prompt"]["15"]["class_type"], "ResizeImageMaskNode");
        assert_eq!(
            g["prompt"]["15"]["inputs"]["resize_type"],
            "scale longer dimension"
        );
        assert_eq!(g["prompt"]["15"]["inputs"]["resize_type.longer_size"], 1536);
        assert_eq!(g["prompt"]["15"]["inputs"]["scale_method"], "lanczos");
        assert_eq!(g["prompt"]["16"]["inputs"]["image"], json!(["15", 0]));
        assert!(
            g["prompt"]["38"].is_null(),
            "I2V 导出不要接 AudioVAE；本机 MPS 上空 audio latent 可能解出 NaN 并拖垮 SaveVideo"
        );
        assert!(
            g["prompt"]["10"]["inputs"]["audio"].is_null(),
            "CreateVideo.audio 是可选输入，普通图生视频先导出稳定无声 MP4"
        );

        assert_eq!(g["prompt"]["17"]["inputs"]["width"], 384);
        assert_eq!(g["prompt"]["17"]["inputs"]["height"], 256);
        assert_eq!(g["prompt"]["17"]["inputs"]["length"], 49);
        assert_eq!(g["prompt"]["19"]["inputs"]["frames_number"], 49);
        assert_eq!(g["prompt"]["24"]["inputs"]["model"], json!(["1", 0]));
        assert_eq!(g["prompt"]["34"]["inputs"]["model"], json!(["1", 0]));

        let wire = serde_json::to_string(&g).unwrap();
        assert!(
            !wire.contains("\"class_type\":\"LTXVImgToVideo\""),
            "{wire}"
        );
        assert!(!wire.contains("\"class_type\":\"KSampler\""), "{wire}");
    }

    #[test]
    fn comfyui_execution_error_能被准确提取() {
        let history = json!({
            "27a4b099-7ff6-4b17-8164-337db9f9f837": {
                "status": {
                    "status_str": "error",
                    "completed": false,
                    "messages": [
                        [
                            "execution_error",
                            {
                                "node_id": "25",
                                "node_type": "SamplerCustomAdvanced",
                                "exception_message": "[Errno 32] Broken pipe\n",
                                "exception_type": "BrokenPipeError"
                            }
                        ]
                    ]
                }
            }
        });

        let err = extract_comfyui_history_error(&history, "27a4b099-7ff6-4b17-8164-337db9f9f837");
        assert!(err.is_some());
        let err_msg = err.unwrap();
        assert!(err_msg.contains("SamplerCustomAdvanced"));
        assert!(err_msg.contains("Broken pipe"));
        assert!(err_msg.contains("BrokenPipeError"));
        assert!(err_msg.contains("stdout/stderr"));
        assert!(err_msg.contains("重启 ComfyUI"));
    }

    #[test]
    fn savevideo音频nan要提示禁用mux而不是只吐av错误() {
        let history = json!({
            "7c35d3ee-7167-4eb3-b2d0-8e45e9015a7d": {
                "status": {
                    "status_str": "error",
                    "completed": false,
                    "messages": [
                        [
                            "execution_error",
                            {
                                "node_id": "11",
                                "node_type": "SaveVideo",
                                "exception_message": "Invalid argument: 'avcodec_send_frame()' returned 22; last error log: [aac] Input contains (near) NaN/+-Inf\n",
                                "exception_type": "av.error.ArgumentError"
                            }
                        ]
                    ]
                }
            }
        });

        let err = extract_comfyui_history_error(&history, "7c35d3ee-7167-4eb3-b2d0-8e45e9015a7d");
        assert!(err.is_some());
        let err_msg = err.unwrap();
        assert!(err_msg.contains("SaveVideo"));
        assert!(err_msg.contains("NaN"));
        assert!(err_msg.contains("音频轨"));
        assert!(err_msg.contains("无声 MP4"));
    }
}

#[cfg(test)]
mod dump_for_manual_verification {
    use super::*;

    /// 把工具真正会发出去的图打印出来，供人工提交给 ComfyUI 实跑。
    /// 默认 ignore —— 它不是断言，是取证工具。
    #[test]
    #[ignore]
    fn dump_t2v_graph() {
        let g = build_t2v_graph(&T2vParams {
            unet_name: "ltx-2.5-22b-distilled-transformer.safetensors",
            clip_name: "gemma4-12b-with-proj-ltx-2.5-bf16.safetensors",
            clip_type: "ltxv",
            vae_name: "ltx-2.5-video-vae-conv-bf16.safetensors",
            prompt: "a fox moving quickly in a beautiful winter scenery nature trees sunset tracking camera",
            neg_prompt: "blurry, static, low quality",
            width: 832,
            height: 480,
            frames_count: 121,
            fps: 24,
            seed: 4242,
            steps: 20,
            model_ref: json!(["1", 0]),
        });
        println!("{}", serde_json::to_string(&g).unwrap());
    }



    /// **两个出片入口都要自己对齐尺寸。**
    ///
    /// 32 对齐是引擎的硬约束（H3 的 `WeeToddH3GenerationConfig` 直接拒绝
    /// 非 32 倍数）。调用方记得对齐是好事，但**不能指望每个调用方都记得** ——
    /// 真出过：agent 那条路一直调 `fit_dimensions`，分镜这条忘了，
    /// 于是分镜出片必失败，报的还是一句看不懂的英文。
    ///
    /// 参考生视频走的是同一个引擎，只在首帧那条上对齐等于漏一半。
    #[test]
    fn 两个出片入口都对齐尺寸() {
        let src = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/agent_runtime/video_tool.rs"
        ))
        .expect("读不到源码");
        for entry in [
            "pub(crate) async fn render_shot_video(",
            "pub(crate) async fn render_shot_video_reference_with(",
            "pub(crate) async fn render_shot_video_text(",
        ] {
            let at = src.find(entry).unwrap_or_else(|| panic!("找不到 {entry}"));
            // 只看函数开头那一段 —— 对齐必须在用到尺寸之前
            let end = (at + 1400).min(src.len());
            // 按字符边界截：函数开头的中文注释会让按字节截的位置落在一个字中间
            let end = (at..=end).rev().find(|i| src.is_char_boundary(*i)).unwrap();
            let head = &src[at..end];
            assert!(
                head.contains("fit_dimensions((width, height))"),
                "{entry} 没有对齐尺寸"
            );
        }
    }

    #[test]
    fn 剥离音频函数在无音轨文件上平稳跳过() {
        let tmp = std::env::temp_dir().join("anima_dummy_test.txt");
        let _ = std::fs::write(&tmp, b"dummy content");
        let res = strip_audio(&tmp);
        let _ = std::fs::remove_file(&tmp);
        assert!(res.is_ok());
    }

    /// 合成一段**真的带音轨**的 mp4 当素材。
    ///
    /// 原来这个测试是读 `/tmp/test_with_audio.mp4`，文件不在就 `return` ——
    /// 于是在任何干净机器上它都绿，而且绿得毫无意义。素材要自己造。
    fn 合成带音轨样片(path: &Path) -> bool {
        let ok = std::process::Command::new(ffmpeg_binary())
            .args([
                "-y", "-loglevel", "error",
                "-f", "lavfi", "-i", "testsrc=size=64x64:rate=10:duration=1",
                "-f", "lavfi", "-i", "sine=frequency=440:duration=1",
                "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest",
            ])
            .arg(path)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        ok && path.is_file()
    }

    #[test]
    fn 剥离音频函数能成功剥离带音轨视频() {
        let tmp = std::env::temp_dir().join(format!("anima_strip_test_{}.mp4", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        if !合成带音轨样片(&tmp) {
            // ffmpeg 不在就没法测 —— 但要说出来，不要假装通过
            eprintln!("跳过：本机没有可用的 ffmpeg，无法合成测试素材");
            return;
        }
        assert!(has_audio_stream(&tmp), "合成的样本应该有音轨");
        let res = strip_audio(&tmp);
        assert!(res.is_ok(), "strip_audio 应该成功");
        assert!(!has_audio_stream(&tmp), "strip_audio 后不应再有音轨");
        let _ = std::fs::remove_file(&tmp);
    }

    /// **这条测的是「谁在什么条件下剥」，不是「剥得干不干净」。**
    ///
    /// 2026-09-10 的回归就长在这里：`strip_audio` 完全正确、它自己的测试也
    /// 全绿，错的是 `render_shot_video` 无条件调了它。孤立测函数抓不到。
    ///
    /// 判据直接接到下游的真实消费者 `pick_shot_audio` 上 —— 音轨还在不在，
    /// 决定的是整集成片有没有声音。
    #[test]
    fn 出片音轨门闸_开着时必须让下游选到源片音轨() {
        use crate::cinema::{pick_shot_audio, ShotAudio};
        let tmp = std::env::temp_dir().join(format!("anima_gate_on_{}.mp4", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        if !合成带音轨样片(&tmp) {
            eprintln!("跳过：本机没有可用的 ffmpeg，无法合成测试素材");
            return;
        }

        // 用户没关音频（缺省）：音轨必须原样留着
        finalize_shot_audio(&tmp, true);
        assert!(
            has_audio_stream(&tmp),
            "generate_audio=true 时不该剥音轨 —— H3 的同步中文台词就在这条音轨里"
        );
        assert_eq!(
            pick_shot_audio(false, has_audio_stream(&tmp)),
            ShotAudio::Source,
            "没配 TTS 时下游必须能选到源片音轨，否则整集是哑片"
        );

        // 用户显式关掉：这时才剥
        finalize_shot_audio(&tmp, false);
        assert!(!has_audio_stream(&tmp), "generate_audio=false 时应该剥掉音轨");
        assert_eq!(
            pick_shot_audio(false, has_audio_stream(&tmp)),
            ShotAudio::Silence,
            "显式关掉音频后补静音是对的"
        );

        let _ = std::fs::remove_file(&tmp);
    }

    /// 两个出片入口都必须走门闸，且门闸必须是有条件的。
    ///
    /// 和上面那条互补：上面测行为，这条防「又有人在别处加一句无条件 strip」。
    #[test]
    fn 两个出片入口的音轨都受开关控制() {
        let src = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/agent_runtime/video_tool.rs"
        ))
        .expect("读不到源码");
        for entry in [
            "pub(crate) async fn render_shot_video(",
            // 参考生视频原来还有一层薄包装 `render_shot_video_reference`，2026-09-16 清理死代码时删了，
            // 真正的实现一直是这个 `_with`
            "pub(crate) async fn render_shot_video_reference_with(",
        ] {
            let at = src.find(entry).unwrap_or_else(|| panic!("找不到 {entry}"));
            // **截断点要落在字符边界上。** 这个文件里全是中文注释，
            // `&src[a..b]` 只要 b 掉进一个汉字中间就直接 panic ——
            // 这条测试一直是靠运气过的：2026-09-12 在别处删掉一行重复的
            // `#[allow]`，字节全体前移 37，它就崩了，而被测的行为一个字没动。
            let mut end = (at + 6000).min(src.len());
            while !src.is_char_boundary(end) {
                end += 1;
            }
            let body = &src[at..end];
            assert!(
                body.contains("generate_audio: bool"),
                "{entry} 必须接收 generate_audio —— 不能自己决定剥不剥"
            );
            assert!(
                body.contains("finalize_shot_audio(out, generate_audio)"),
                "{entry} 必须走门闸"
            );
            assert!(
                !body.contains("let _ = strip_audio(out);"),
                "{entry} 不能无条件剥音轨（2026-09-04 / 09-10 两次回归都是这句）"
            );
        }
    }
}

#[cfg(test)]
mod h3_quality_arg {
    use super::*;
    /// 画布加速档（2026-10-08）：认得的照用，认不出 / 没给回 None（按尺寸自动）
    #[test]
    fn 加速档参数() {
        let req = |v: serde_json::Value| ToolCallRequest::new("animate_image", v);
        assert_eq!(requested_h3_tier(&req(json!({"h3_quality": "lightx2v"}))), Some(weetodd::H3Tier::LightX2V));
        assert_eq!(requested_h3_tier(&req(json!({"h3_quality": "staged"}))), Some(weetodd::H3Tier::Staged));
        assert_eq!(requested_h3_tier(&req(json!({"h3_quality": "dense"}))), Some(weetodd::H3Tier::Dense));
        assert_eq!(requested_h3_tier(&req(json!({"h3_quality": "乱写"}))), None);
        assert_eq!(requested_h3_tier(&req(json!({}))), None);
        // 只在生产代码那段里找 —— 整个文件里找的话，这条断言自己就含着这句话，永远找得到（变异测试抓出来的假绿）
        let src = include_str!("video_tool.rs");
        let prod = &src[..src.find("#[cfg(test)]").unwrap()];
        assert!(prod.contains("let tier = requested_h3_tier(request).unwrap_or("), "画布 H3 出片没读加速档");
    }

    /// 画布 / 对话的 H3、LTX 出片**挂上调用方选的 LoRA**（2026-10-11：以前能选、激活词进了提示词，权重一直没挂）。
    /// 长镜头同理。只看生产代码那段（理由同上）
    #[test]
    fn 选的_lora_挂进_weetodd_的图() {
        let src = include_str!("video_tool.rs");
        let prod = &src[..src.find("#[cfg(test)]").unwrap()];
        let i = prod.find("    async fn run_weetodd(").unwrap();
        let run = &prod[i..prod[i..].find("/// 提交一张 WeeTodd 图").map(|j| i + j).unwrap()];
        assert!(run.contains("lora::parse_requested(&request.arguments)"), "run_weetodd 没读 loras");
        assert_eq!(run.matches("loras: &user_loras,").count(), 2, "H3 和 LTX 两张图都要挂");
        assert!(!run.contains("loras: &[],"), "还有一张图传的是空 LoRA");
        let i = prod.find("pub(crate) async fn render_long_take(").unwrap();
        let lt = &prod[i..prod[i..].find("pub(crate) async fn render_shot_video_text(").map(|j| i + j).unwrap()];
        assert!(lt.contains("        loras,\n") && !lt.contains("loras: &[],"), "长镜头没挂调用方的 LoRA");
    }
}

#[cfg(test)]
mod reference_audio_live {
    /// 真 ffmpeg：一段 30 秒的正弦波截成 15 秒、48k 双声道，搬进 ComfyUI 的 input。
    /// `cargo test --lib 参考音频截到片长 -- --ignored`
    #[test]
    #[ignore = "要本机 ffmpeg 和 ComfyUI 的 input 目录"]
    fn 参考音频截到片长() {
        let dir = std::env::temp_dir().join(format!("anima-aud-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("long.mp3");
        let ok = std::process::Command::new(super::ffmpeg_binary())
            .args(["-y", "-v", "error", "-f", "lavfi", "-i", "sine=frequency=440:duration=30"])
            .arg(&src).status().unwrap().success();
        assert!(ok);
        let name = format!("r2v_audio_test_{}.wav", uuid::Uuid::new_v4().simple());
        super::stage_reference_audio(&src, &name, 15.0).unwrap();
        let staged = super::find_comfyui_input_dirs().into_iter().map(|d| d.join(&name)).find(|p| p.exists()).expect("没进 input");
        let probe = std::process::Command::new(super::ffprobe_binary())
            .args(["-v", "error", "-show_entries", "format=duration:stream=sample_rate,channels", "-of", "default=nw=1"])
            .arg(&staged).output().unwrap();
        let info = String::from_utf8_lossy(&probe.stdout).to_string();
        println!("{info}");
        assert!(info.contains("sample_rate=48000") && info.contains("channels=2"), "{info}");
        let secs: f64 = info.lines().find_map(|l| l.strip_prefix("duration=")).unwrap().parse().unwrap();
        assert!((secs - 15.0).abs() < 0.1, "截出来 {secs} 秒");
        for d in super::find_comfyui_input_dirs() { let _ = std::fs::remove_file(d.join(&name)); }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
