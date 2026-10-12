//! 画布的「深度动作捕捉」—— 一段视频进，一段黑白深度视频出（越近越白）。
//!
//! # 它在整条链里的位置（照 libtv 做的，用户 2026-09-14 描述）
//!
//! ```text
//! [视频] ──→ [深度动作捕捉] ──→ [参考生视频] ──→ 新视频
//!                                  ↑
//!                    [人物图] ─────┘
//! ```
//!
//! 这一步只抽深度，不生成任何东西。下游拿深度视频当动作 / 运镜控制。
//! **只做画布节点，不接生产工作区**（用户定的，先看效果）。
//!
//! # 用的是什么
//!
//! WeeTodd 的 `Video Depth Preprocessor (MLX)`，模型是 Video Depth Anything Small
//! （Apache-2.0，转成 MLX 放在 `models/annotators/video_depth_anything/`）。
//! 按 32 帧一窗、带重叠对齐地处理，所以前后帧的深度不跳。
//! 2026-09-14 实测：一段 121 帧 768×512 的舞蹈视频几秒就抽完，人物轮廓和墙角结构都清楚。
//!
//! # 两个写错不报错的地方
//!
//! - 输出节点用 `VHS_VideoCombine`，它的产物挂在 history 的 `gifs` 键下，不是 `videos`。
//!   `weetodd::find_video_output` 三个键都认，所以取得到。
//! - 帧率接的是源视频自己的（`GetVideoComponents` 第 3 个输出）。写死 24 的话，
//!   30fps 的素材抽出来的深度视频会变慢，和下游对不上帧。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// 深度模型的文件名。**和 ComfyUI 下拉里的名字逐字一致**，写错了节点直接报找不到。
pub const DEPTH_CHECKPOINT: &str = "video_depth_anything/video_depth_anything_vits_mlx.safetensors";

/// 上传视频的大小上限。走 base64 过 IPC，太大会把界面卡住。
const MAX_VIDEO_BYTES: usize = 300 * 1024 * 1024;

/// 抽深度的 ComfyUI 图。`video_name` 是已经放进 ComfyUI `input/` 的文件名。
pub fn build_depth_graph(video_name: &str, filename_prefix: &str) -> Value {
    json!({
        "1": {"class_type": "LoadVideo", "inputs": {"file": video_name}},
        "2": {"class_type": "GetVideoComponents", "inputs": {"video": ["1", 0]}},
        "3": {"class_type": "WeeToddMLXVideoDepthLoader", "inputs": {"checkpoint": DEPTH_CHECKPOINT}},
        "4": {"class_type": "WeeToddMLXVideoDepthPreprocessor", "inputs": {
            "images": ["2", 0],
            "depth_model": ["3", 0],
            "input_size": 518,
            "precision": "float32 quality",
            "encoder_chunk_size": 4,
            "decoder_chunk_size": 4,
            // 越近越白 —— LTX Union 深度控制训练时就是这个方向
            "depth_polarity": "near white (recommended)",
            // 抽完就卸，下游出片要内存
            "keep_model_warm": false
        }},
        "5": {"class_type": "VHS_VideoCombine", "inputs": {
            "images": ["4", 0],
            "frame_rate": ["2", 2],
            "loop_count": 0,
            "filename_prefix": filename_prefix,
            "format": "video/h264-mp4",
            "pingpong": false,
            "save_output": true
        }}
    })
}

fn video_ext_from_mime(mime: &str) -> Option<&'static str> {
    match mime.trim().to_ascii_lowercase().as_str() {
        "video/mp4" => Some("mp4"),
        "video/quicktime" => Some("mov"),
        "video/webm" => Some("webm"),
        "video/x-matroska" => Some("mkv"),
        _ => None,
    }
}

fn canvas_media_dir(config_dir: &Path, kind: &str) -> PathBuf {
    config_dir.join("anima").join("canvas").join(kind)
}

#[derive(Debug, Deserialize)]
pub struct SaveVideoArgs {
    pub data_url: String,
    pub filename: String,
}

#[derive(Debug, Serialize)]
pub struct SavedVideo {
    pub path: String,
}

/// 保存画布上传的视频，返回本机绝对路径。
#[tauri::command]
pub async fn canvas_save_video(state: tauri::State<'_, crate::AppState>, args: SaveVideoArgs) -> Result<SavedVideo> {
    save_video(&state.config_dir, &args).map(|p| SavedVideo { path: p.to_string_lossy().to_string() })
}

pub fn save_video(config_dir: &Path, args: &SaveVideoArgs) -> Result<PathBuf> {
    use base64::Engine;
    let rest = args.data_url.strip_prefix("data:").ok_or_else(|| Error::config("视频必须是 data:video/*;base64,... 格式"))?;
    let (mime, b64) = rest.split_once(";base64,").ok_or_else(|| Error::config("视频缺少 base64 数据"))?;
    let ext = video_ext_from_mime(mime).ok_or_else(|| Error::config(format!("不支持的视频格式：{mime}（支持 mp4 / mov / webm / mkv）")))?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).map_err(|e| Error::config(format!("解析视频失败：{e}")))?;
    if bytes.is_empty() {
        return Err(Error::config("视频是空的"));
    }
    if bytes.len() > MAX_VIDEO_BYTES {
        return Err(Error::config(format!("视频太大（{} MB），上限 300 MB —— 先剪短一点", bytes.len() / 1024 / 1024)));
    }
    let dir = canvas_media_dir(config_dir, "uploads");
    std::fs::create_dir_all(&dir).map_err(|e| Error::config(format!("建上传目录失败：{e}")))?;
    let stem: String = Path::new(&args.filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("video")
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .take(40)
        .collect();
    let path = dir.join(format!("{stem}_{}.{ext}", &uuid::Uuid::new_v4().simple().to_string()[..8]));
    std::fs::write(&path, bytes).map_err(|e| Error::config(format!("保存视频失败：{e}")))?;
    Ok(path)
}

fn audio_ext_from_mime(mime: &str) -> Option<&'static str> {
    match mime.trim().to_ascii_lowercase().as_str() {
        "audio/mpeg" | "audio/mp3" => Some("mp3"),
        "audio/wav" | "audio/x-wav" | "audio/wave" | "audio/vnd.wave" => Some("wav"),
        "audio/mp4" | "audio/x-m4a" | "audio/m4a" | "audio/aac" => Some("m4a"),
        "audio/flac" | "audio/x-flac" => Some("flac"),
        "audio/ogg" | "audio/opus" => Some("ogg"),
        _ => None,
    }
}

/// 画布上传的音频多大算太大。一首歌几 MB 到几十 MB；参考音频只用前十几秒，没必要更大
const MAX_AUDIO_BYTES: usize = 200 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct SavedAudio {
    pub path: String,
    /// 时长（秒）。读不出来是 `None` —— 界面就不显示，不编一个
    pub secs: Option<f64>,
}

/// 保存画布上传的音频（「音频素材」节点，2026-10-08），返回本机绝对路径和时长。
/// 参数和视频那条同形（`data_url` + `filename`）。
#[tauri::command]
pub async fn canvas_save_audio(state: tauri::State<'_, crate::AppState>, args: SaveVideoArgs) -> Result<SavedAudio> {
    let p = save_audio(&state.config_dir, &args)?;
    Ok(SavedAudio { secs: probe_secs(&p), path: p.to_string_lossy().to_string() })
}

pub fn save_audio(config_dir: &Path, args: &SaveVideoArgs) -> Result<PathBuf> {
    use base64::Engine;
    let rest = args.data_url.strip_prefix("data:").ok_or_else(|| Error::config("音频必须是 data:audio/*;base64,... 格式"))?;
    let (mime, b64) = rest.split_once(";base64,").ok_or_else(|| Error::config("音频缺少 base64 数据"))?;
    // 有的系统给 mp3 报空 mime —— 退回看扩展名
    let by_name = Path::new(&args.filename).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    let ext = audio_ext_from_mime(mime)
        .or_else(|| by_name.as_deref().and_then(|e| ["mp3", "wav", "m4a", "flac", "ogg", "aac"].into_iter().find(|x| *x == e)))
        .ok_or_else(|| Error::config(format!("不支持的音频格式：{mime}（支持 mp3 / wav / m4a / flac / ogg）")))?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).map_err(|e| Error::config(format!("解析音频失败：{e}")))?;
    if bytes.is_empty() {
        return Err(Error::config("音频是空的"));
    }
    if bytes.len() > MAX_AUDIO_BYTES {
        return Err(Error::config(format!("音频太大（{} MB），上限 200 MB", bytes.len() / 1024 / 1024)));
    }
    let dir = canvas_media_dir(config_dir, "uploads");
    std::fs::create_dir_all(&dir).map_err(|e| Error::config(format!("建上传目录失败：{e}")))?;
    let stem: String = Path::new(&args.filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("audio")
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .take(40)
        .collect();
    let path = dir.join(format!("{stem}_{}.{ext}", &uuid::Uuid::new_v4().simple().to_string()[..8]));
    std::fs::write(&path, bytes).map_err(|e| Error::config(format!("保存音频失败：{e}")))?;
    Ok(path)
}

/// 媒体时长（秒），ffprobe 读。读不出来回 `None`
fn probe_secs(path: &Path) -> Option<f64> {
    let out = std::process::Command::new(crate::agent_runtime::video_tool::ffprobe_binary())
        .args(["-v", "error", "-show_entries", "format=duration", "-of", "default=nw=1:nk=1"])
        .arg(path)
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse::<f64>().ok().filter(|s| *s > 0.0)
}

#[derive(Debug, Deserialize)]
pub struct DepthCaptureArgs {
    /// 源视频的本机绝对路径（上传的，或画布上已经出好的）
    pub video_path: String,
}

#[derive(Debug, Serialize)]
pub struct DepthCaptureResult {
    /// 深度视频的本机绝对路径
    pub path: String,
}

pub async fn depth_capture_with(config_dir: &Path, args: DepthCaptureArgs) -> Result<DepthCaptureResult> {
    let src = PathBuf::from(args.video_path.trim());
    if !src.is_file() {
        return Err(Error::config(format!("找不到源视频：{}", src.display())));
    }
    let id = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("mp4").to_ascii_lowercase();
    let name = format!("anima_depth_src_{id}.{ext}");
    let mut copied = false;
    for dir in crate::agent_runtime::video_tool::find_comfyui_input_dirs() {
        copied |= std::fs::copy(&src, dir.join(&name)).is_ok();
    }
    if !copied {
        return Err(Error::config("源视频复制进 ComfyUI 的 input 失败 —— ComfyUI 装在哪？"));
    }

    let client = reqwest::Client::builder().no_proxy().timeout(std::time::Duration::from_secs(120)).build().unwrap_or_default();
    if !crate::agent_runtime::weetodd::node_available(&client, "WeeToddMLXVideoDepthPreprocessor").await {
        return Err(Error::config("ComfyUI 里没有 WeeTodd 的深度节点（WeeToddMLXVideoDepthPreprocessor）—— ComfyUI 没开，或者 WeeTodd-Nodes 版本太旧"));
    }
    let out = canvas_media_dir(config_dir, "depth").join(format!("depth_{id}.mp4"));
    let graph = build_depth_graph(&name, "Anima/canvas_depth");
    let bytes = crate::agent_runtime::video_tool::submit_and_fetch_video(&client, &graph, "画布 · 深度动作捕捉", Some(&out))
        .await
        .map_err(Error::runtime)?;
    if let Some(parent) = out.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&out, &bytes).map_err(|e| Error::runtime(format!("写深度视频失败：{e}")))?;
    Ok(DepthCaptureResult { path: out.to_string_lossy().to_string() })
}

// ---------------------------------------------------------------------------
// 深度 + 人物图 → 新视频（LTX 2.5 Union 深度控制）
// ---------------------------------------------------------------------------
//
// # 为什么人物图当首帧，不当关键帧
//
// 2026-09-14 同题实测（舞蹈视频 121 帧 + 校服三视图裁出的正面全身）：
//
// | 做法 | 耗时 | 结果 |
// |---|---|---|
// | 人物图当 `first_frame` | 150 s | 服装、发型全程保持，动作逐帧对得上原视频；前 8 帧左右停在站姿，然后顺滑接上 |
// | 第 0 帧关键帧、强度 0.5 | 300 s | 和上面几乎一样 |
//
// 效果没差别、慢一倍，所以用首帧。
//
// # 首帧为什么要自动对位
//
// LTX 把这张图当成视频第一帧，而深度视频规定了人第一帧站在哪、有多大。
// 两边对不上时模型要先把人「挪过去」。实测用的首帧是手工摆的：把人物缩到深度第一帧里
// 人的高度、站到同一个位置。这里把这一步自动化 —— 普通人不会自己去对位。

/// 出片的规格。Union 控制两段式要求宽高能被 128 整除；官方工作流是 768×512 / 121 帧。
pub const LANDSCAPE: (u32, u32) = (768, 512);
pub const PORTRAIT: (u32, u32) = (512, 768);
/// 高清档。2026-09-14 同题实测：1152×768 快速甩头时五官还看得清、也更像本人（768×512 糊成一团），
/// 耗时约 320 s（标准档 150 s）。宽高仍能被 128 整除、比例不变。
pub const LANDSCAPE_HD: (u32, u32) = (1152, 768);
pub const PORTRAIT_HD: (u32, u32) = (768, 1152);
pub const FPS: u32 = 24;
/// 以前固定取 5 秒。现在跟节点设置里的时长走，默认仍是 5 秒
pub const DEFAULT_SECS: f64 = 5.0;
/// 每条路一次最长取几秒。超过的按上限出、在节点上写明
pub const MAX_SECS_LTX: f64 = 10.0;
pub const MAX_SECS_H3: f64 = 10.0;

/// 横竖跟源视频走；高清只给 LTX（H3 这条路没测过高清，而且本来就要 17 分钟）。
pub fn pick_size(portrait: bool, hd: bool) -> (u32, u32) {
    match (portrait, hd) {
        (false, false) => LANDSCAPE,
        (true, false) => PORTRAIT,
        (false, true) => LANDSCAPE_HD,
        (true, true) => PORTRAIT_HD,
    }
}

/// 节点设置里的清晰度 → 出片尺寸。回 (尺寸, 实际用的档位)。
///
/// - LTX Union 两段式要求宽高能被 128 整除：480P 768×512、720P 1152×768、1080P 1536×1024。
/// - H3：480P 768×512、720P 1280×736（和分镜出片同一个尺寸）；**没有 1080P**，按 720P 出。
pub fn motion_size(portrait: bool, h3: bool, resolution: &str) -> ((u32, u32), &'static str) {
    let (land, tier) = match (h3, resolution.trim().to_ascii_uppercase().as_str()) {
        (false, "1080P") => ((1536, 1024), "1080P"),
        (false, "720P") => (LANDSCAPE_HD, "720P"),
        (true, "720P" | "1080P") => ((1280, 736), "720P"),
        _ => (LANDSCAPE, "480P"),
    };
    (if portrait { (land.1, land.0) } else { land }, tier)
}

/// 要取几秒：没设置按 5 秒，超过这条路的上限按上限。回 (秒数, 是否被截)。
pub fn motion_secs(requested: Option<f64>, h3: bool) -> (f64, bool) {
    let max = if h3 { MAX_SECS_H3 } else { MAX_SECS_LTX };
    let want = requested.filter(|v| v.is_finite() && *v > 0.0).unwrap_or(DEFAULT_SECS);
    (want.min(max).max(1.0), want > max)
}

/// LTX 的帧数必须是 8k+1。取 `max_frames` 帧，素材不够时往下取；少于 25 帧（约 1 秒）就不做了。
pub fn ltx_frames(available: u32, max_frames: u32) -> Option<u32> {
    let n = available.min(max_frames);
    if n < 25 {
        return None;
    }
    Some((n - 1) / 8 * 8 + 1)
}

/// 画面里「人」的范围：深度图里最亮（最近）的那一片。
///
/// 阈值取亮度第 97 百分位 —— 固定阈值不行，不同素材整体亮度差很多
/// （实测那段舞蹈第一帧最亮才 175）。
pub fn subject_box(depth: &image::GrayImage) -> Option<(u32, u32, u32, u32)> {
    let mut hist = [0u32; 256];
    for p in depth.pixels() {
        hist[p.0[0] as usize] += 1;
    }
    let total: u32 = hist.iter().sum();
    let mut acc = 0;
    let mut thr = 255u8;
    for v in (0..256).rev() {
        acc += hist[v];
        if acc as f32 >= total as f32 * 0.03 {
            thr = v as u8;
            break;
        }
    }
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for (x, y, p) in depth.enumerate_pixels() {
        if p.0[0] >= thr && thr > 0 {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    if !(x1 > x0 && y1 > y0) {
        return None;
    }
    // **上下再扩。** 百分位只抓得到离镜头最近的那一截：脚和地面离镜头差不多远，
    // 头顶的头发边缘也暗 —— 2026-09-14 那段舞蹈只抓到 165~442，人实际在 111~511，
    // 按这个对位出来的人偏小、而且悬在半空。
    // 判据：这一行在人所在的列里，比整行的中位数亮出一截，就还是人。
    let row_contrast = |y: u32| -> i32 {
        let mut row: Vec<u8> = (0..depth.width()).map(|x| depth.get_pixel(x, y).0[0]).collect();
        let seg_max = row[x0 as usize..=x1 as usize].iter().copied().max().unwrap_or(0) as i32;
        row.sort_unstable();
        seg_max - row[row.len() / 2] as i32
    };
    const CONTRAST: i32 = 12;
    while y0 > 0 && row_contrast(y0 - 1) > CONTRAST {
        y0 -= 1;
    }
    while y1 + 1 < depth.height() && row_contrast(y1 + 1) > CONTRAST {
        y1 += 1;
    }
    Some((x0, y0, x1, y1))
}

/// 人物图里「人」的范围。背景色取**整圈边框像素的中位数**。
///
/// 不能取四个角的平均：2026-09-14 用户那张校服三视图，左下角正好压着半身特写的黑西装，
/// 平均出来是灰色，整张图都成了「前景」，挑出来的是整张图。中位数不怕一两个角被人压住。
///
/// **三视图 / 设定卡里有好几个人**：按列切成几段，挑露出完整全身的那一个 ——
/// 下边离图底有空隙（脚没被裁掉），里面最高的；都被裁了就挑最高的。
/// 用户给的校服三视图左边是半身特写（贴着图底），这样挑中的是正面全身。
pub fn figure_box(img: &image::RgbImage) -> (u32, u32, u32, u32, [u8; 3]) {
    let (x0, y0, x1, y1, bg, _) = figure_pick(img);
    (x0, y0, x1, y1, bg)
}

/// 人物图里有没有一个**完整露出的人**：头顶、脚底、左右都没碰到图片边缘。
///
/// 动作精准只靠首帧记住人。2026-09-14 用户那张坐姿半身图（腿被裁、深色背景铺满）：
/// 首帧贴出来是角落里一个带方框的半身人，和深度第一帧站着的人对不上，
/// 5 秒、10 秒两条都在第 1 帧就把人扔掉、换成模型现编的人；换 Qwen 画首帧也没换成人。
/// 所以这种图直接拦下、让它走人物清晰（H3 半身也行），不白跑一条换了人的片子。
pub fn figure_is_whole(img: &image::RgbImage) -> bool {
    figure_pick(img).5
}

fn figure_pick(img: &image::RgbImage) -> (u32, u32, u32, u32, [u8; 3], bool) {
    let (w, h) = img.dimensions();
    let border: Vec<[u8; 3]> = (0..w)
        .flat_map(|x| [img.get_pixel(x, 0).0, img.get_pixel(x, h - 1).0])
        .chain((0..h).flat_map(|y| [img.get_pixel(0, y).0, img.get_pixel(w - 1, y).0]))
        .collect();
    let bg: [u8; 3] = std::array::from_fn(|c| {
        let mut v: Vec<u8> = border.iter().map(|p| p[c]).collect();
        v.sort_unstable();
        v[v.len() / 2]
    });
    let is_fg = |x: u32, y: u32| {
        let p = img.get_pixel(x, y).0;
        (0..3).map(|c| (p[c] as i32 - bg[c] as i32).abs()).max().unwrap_or(0) > 24
    };
    let col_has: Vec<bool> = (0..w).map(|x| (0..h).step_by(2).any(|y| is_fg(x, y))).collect();
    let mut runs = vec![];
    let mut x = 0;
    while x < w {
        if col_has[x as usize] {
            let s = x;
            while x < w && col_has[x as usize] {
                x += 1;
            }
            runs.push((s, x - 1));
        } else {
            x += 1;
        }
    }
    let mut best: Option<(u32, u32, u32, u32, bool)> = None;
    for (s, e) in runs {
        if e - s < w / 40 {
            continue; // 细碎的杂点
        }
        let (mut top, mut bottom) = (u32::MAX, 0);
        for xx in s..=e {
            for y in 0..h {
                if is_fg(xx, y) {
                    top = top.min(y);
                    bottom = bottom.max(y);
                }
            }
        }
        if top == u32::MAX {
            continue;
        }
        let whole = bottom + 2 < h && top > 1 && s > 1 && e + 2 < w;
        let better = match best {
            None => true,
            Some((_, bt, _, bb, bw)) => (whole, bottom - top) > (bw, bb - bt),
        };
        if better {
            best = Some((s, top, e, bottom, whole));
        }
    }
    let (x0, y0, x1, y1, whole) = best.unwrap_or((0, 0, w - 1, h - 1, false));
    (x0, y0, x1, y1, bg, whole)
}

/// 把人物放到深度第一帧里人的位置：高度一样、底边一样、水平中心一样。
pub fn align_first_frame(depth0: &image::GrayImage, figure: &image::RgbImage, size: (u32, u32)) -> image::RgbImage {
    let (w, h) = size;
    let (fx0, fy0, fx1, fy1, bg) = figure_box(figure);
    let mut canvas = image::RgbImage::from_pixel(w, h, image::Rgb(bg));
    let crop = image::imageops::crop_imm(figure, fx0, fy0, fx1 - fx0 + 1, fy1 - fy0 + 1).to_image();
    let (tx, ty, th) = match subject_box(depth0) {
        // 深度图和出片同尺寸（调用方先规整过）
        Some((x0, y0, x1, y1)) => ((x0 + x1) / 2, y1, (y1 - y0).max(h / 4)),
        None => (w / 2, h * 95 / 100, h * 8 / 10),
    };
    let scale = th as f32 / crop.height() as f32;
    let nw = ((crop.width() as f32 * scale).round() as u32).max(1);
    let nh = ((crop.height() as f32 * scale).round() as u32).max(1);
    let resized = image::imageops::resize(&crop, nw, nh, image::imageops::FilterType::Lanczos3);
    let left = tx as i64 - nw as i64 / 2;
    let top = ty as i64 - nh as i64;
    image::imageops::overlay(&mut canvas, &resized, left, top);
    canvas
}

/// 把拿到的 PNG 落成**正好 `size`** 的文件。
///
/// 尺寸已经对的时候原样写，不重编码。差的时候按 Lanczos 缩 ——
/// 长宽比本来就一致（第一张参考图就是按 `size` 裁的那一帧），所以这是纯缩放，不会变形。
fn exact_size_png(bytes: &[u8], size: (u32, u32), out: &std::path::Path) -> Result<()> {
    let img = image::load_from_memory(bytes)
        .map_err(|e| Error::runtime(format!("首帧解不开：{e}")))?;
    if (img.width(), img.height()) == size {
        return std::fs::write(out, bytes).map_err(|e| Error::runtime(format!("写首帧失败：{e}")));
    }
    img.resize_exact(size.0, size.1, image::imageops::FilterType::Lanczos3)
        .save(out)
        .map_err(|e| Error::runtime(format!("写首帧失败：{e}")))
}

/// 出片用哪种控制信号。
///
/// 2026-09-22 同题实测（MEDIA_PIPELINES 5.8）：canny 帧间运动相关 0.160，深度 0.069，pose 等于没接。
/// **canny 要从原视频抽** —— 在深度视频上抽只剩一个剪影的外轮廓。
/// 大脸特写上 canny 也弱（画面暗、边缘少），那几段回落深度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionControl<'a> {
    Depth,
    /// 原视频（已经和深度视频同一起点、同一裁切、同一帧率规整过）在 ComfyUI input 里的名字
    Canny { source_name: &'a str },
}

/// LTX 2.5 Union 控制出片的图。深度 / 原视频都已经规整成 `size` / 24fps / `frames` 帧。
#[allow(clippy::too_many_arguments)]
pub fn build_depth_video_graph(depth_name: &str, first_frame_name: &str, prompt: &str, seed: u64, size: (u32, u32), frames: u32, prefix: &str, control: MotionControl) -> Value {
    let duration = (frames - 1) as f64 / FPS as f64;
    let mut g = json!({
        "1": {"class_type": "WeeToddLTX25ComponentLoader", "inputs": {
            // **要带 weetodd-q8/ 前缀**，2026-09-14 漏了一次：「split component files are missing: transformer_path」
            "transformer": "weetodd-q8/ltx-2.5-22b-distilled-union1p0-q8-paged",
            "text_encoder": crate::agent_runtime::weetodd::TEXT_ENCODER_Q8,
            "video_vae": "ltx-2.5-video-vae-conv-bf16.safetensors",
            "audio_vae": "ltx-2.5-audio-vae-bf16.safetensors",
            "spatial_upscaler": crate::agent_runtime::weetodd::SPATIAL_UPSCALER,
            "duration_head": ""
        }},
        "2": {"class_type": "WeeToddLTX25GenerationConfig", "inputs": {
            "preset": "Custom", "width": size.0, "height": size.1,
            "duration_seconds": duration, "frame_rate": FPS as f64, "seed": seed,
            "low_memory": true, "low_ram_streaming": true,
            "prompt_context": "official_1024", "feed_forward_backend": "reference_fp32"
        }},
        "3": {"class_type": "WeeToddLTX25ICLoRAPipelineMode", "inputs": {
            "config": ["2", 0], "mode": "Two stage — 8 low-resolution + 3 clean refinement",
            "cfg_pp_execution": "automatic", "cfg_pp_schedule": "full"
        }},
        "4": {"class_type": "LoadVideo", "inputs": {"file": depth_name}},
        "5": {"class_type": "GetVideoComponents", "inputs": {"video": ["4", 0]}},
        "6": {"class_type": "WeeToddLTX25ICLoRAControlGuide", "inputs": {
            "control_video": ["5", 0], "control_type": "depth_map",
            "start_frame": 0, "end_frame": frames - 1, "strength": 1.0, "attention_strength": 1.0
        }},
        "7": {"class_type": "WeeToddLTX25Preflight", "inputs": {"model": ["1", 0], "config": ["3", 0]}},
        "8": {"class_type": "LoadImage", "inputs": {"image": first_frame_name}},
        "9": {"class_type": "WeeToddLTX25Generate", "inputs": {
            "model": ["7", 0], "config": ["3", 0], "prompt": prompt,
            "filename_prefix": prefix, "unload_after_generate": true,
            "first_frame": ["8", 0], "media_conditioning": ["6", 0]
        }}
    });
    if let MotionControl::Canny { source_name } = control {
        // 照 probe3_canny 那张实测过的图接：预处理器吐的尺寸不是出片尺寸（resolution=512 出 1152×768），
        // **后面必须 ImageScale 回 `size`**，差几个像素控制就等于没接、任务照样 success
        g["4"]["inputs"]["file"] = json!(source_name);
        g["20"] = json!({"class_type": "CannyEdgePreprocessor", "inputs": {
            "image": ["5", 0], "low_threshold": 100, "high_threshold": 200, "resolution": 512}});
        g["25"] = json!({"class_type": "ImageScale", "inputs": {
            "image": ["20", 0], "upscale_method": "nearest-exact", "width": size.0, "height": size.1, "crop": "disabled"}});
        g["6"]["inputs"]["control_video"] = json!(["25", 0]);
        g["6"]["inputs"]["control_type"] = json!("canny_edges");
    }
    g
}

/// H3 参考生视频 + 深度视频当动作参考（「人物清晰」那一档）。
///
/// # 为什么还要这一条
///
/// 2026-09-14 同题实测（同一段舞蹈、同一张校服三视图）：
///
/// | | LTX 深度控制 | H3 + 原视频当参考 | **H3 + 深度视频当参考** |
/// |---|---|---|---|
/// | 动作 | 逐帧对得上 | 节奏像，姿势不对齐 | **大部分关键姿势对得上** |
/// | 脸 | 快速动作时糊、变形 | 清楚 | **清楚** |
/// | 耗时 | 150 s | 17 min | 17 min |
///
/// LTX 只靠首帧那一张记住人，一转头就忘；H3 把整张人物图当身份参考全程看着。
/// 拿**深度**而不是原视频当参考：原视频的衣服、墙面会被一起学进去，深度只剩动作。
///
/// 人物图原样当 `<Picture 1>`（三视图整张给，H3 自己看得懂），不做首帧对位。
///
/// **多人（2026-09-23 起）**：`image_names` 按顺序就是 `<Picture 1..N>`，默认对应深度视频首帧里从左到右的人。
/// 这条路身份**全程看参考图**（不像 LTX 只从首帧认），所以不用按剪辑点切段 —— 这正是用户选它的理由。
pub fn build_h3_motion_graph(image_names: &[String], depth_name: &str, prompt: &str, seed: u64, size: (u32, u32), frames: u32, prefix: &str) -> Value {
    use crate::agent_runtime::weetodd::{build_h3_graph, H3Mode, H3Params, H3Tier};
    let mut g = build_h3_graph(&H3Params {
        continuity: None,
        mode: H3Mode::Reference { images: image_names, pixel_budget_percent: 100, audios: &[] },
        prompt: &h3_motion_prompt_multi(prompt, image_names.len()),
        width: size.0,
        height: size.1,
        duration_secs: (frames - 1) as f64 / FPS as f64,
        seed,
        // **原样保留 Kotte 实跑验证过的那套**（drbaph 精简版 4 次、经过预设）—— 10-07「标准」档换成了 Larry 完整版，这条不跟着换
        tier: H3Tier::LegacyTurbo,
        filename_prefix: prefix,
        loras: &[],
    });
    // 参考视频要接在**参考图链的末尾**。以前写死 `["21", 0]`（第一张图），多张图时第二张起会被挤掉、不报错
    let tail = g["5"]["inputs"]["references"].clone();
    let m = g.as_object_mut().expect("图是对象");
    m.insert("60".into(), json!({"class_type": "LoadVideo", "inputs": {"file": depth_name}}));
    m.insert("61".into(), json!({"class_type": "GetVideoComponents", "inputs": {"video": ["60", 0]}}));
    // 参考视频串在参考图后面 —— 顺序决定标签：图是 <Picture 1>，视频是 <Video 1>
    m.insert("62".into(), json!({"class_type": "WeeToddH3ReferenceVideo", "inputs": {
        "video_frames": ["61", 0], "fps": FPS as f64,
        "video_size": "match output (recommended)", "temporal_density": "all frames (recommended)",
        "previous_references": tail
    }}));
    if let Some(enc) = m.get_mut("5") {
        enc["inputs"]["references"] = json!(["62", 0]);
    }
    g
}

// ---------------------------------------------------------------------------
// 原片复刻的共用部分：帧数栅格、尺寸、提示词模板
// ---------------------------------------------------------------------------
//
// 这几样最早是给原生 H3 那条（「H3 动作迁移 2.0」，吃猪侠233 的工作流、混合底模）写的。
// 2026-09-24 那条连同它的模型（混合底模、Qwen3-VL int8 编码器、int8 视频 VAE、Ref2V LoRA，约 56 GB）一起删了：
// WeeTodd 走同一个做法换得一样干净，还快 2 倍多（下面的「原片复刻 · 快」）。当初以为差在底模，其实两边失败都是提示词。
// 老画布里存着 `h3_native` 的节点照样能跑 —— 分派时按「原片复刻 · 快」走。

/// 原作的 0.5 MP 16:9（ResolutionSelector 0.5 / 32 对齐）。竖屏对调
pub const H3N_LANDSCAPE: (u32, u32) = (960, 544);

/// H3 的帧数栅格：24 fps、17n+5。和原作工作流里那个数学节点同一个式子。
pub fn h3_native_frames(secs: f64) -> u32 {
    let n = ((secs * FPS as f64).round() as u32).max(5);
    n + (5 + 17 - n % 17) % 17
}

/// 原作的提示词结构：主体定义（每张图一个主体、只管外观、不作首帧）→ 背景沿用视频 → 视频只管动作运镜 → 画面文字规则。
///
/// 用户自己写了 `< 图片 1 >` / `<Picture 1>` 这类标签就原样用 —— 那是按原作模板自己写全了。
pub fn h3_native_prompt(user: &str, n: usize) -> String {
    let user = user.trim();
    if user.contains("< 图片") || user.contains("<图片") || user.contains("<Picture") {
        return user.to_string();
    }
    let mut out = String::from("## 主体定义\n\n");
    for k in 1..=n.max(1) {
        out.push_str(&format!("<主体 {k}>：参考图 < 图片 {k} > 里的角色，脸、发型、服装只照 < 图片 {k} >，面部五官固定不崩坏。< 图片 {k} > 仅作为角色外观身份参考，不作为目标视频首帧。\n\n"));
    }
    let bg = n.max(1) + 1;
    out.push_str(&format!("<主体 {bg}>：背景环境。直接沿用 < 视频 1 > 的空间、环境光、色调与景深氛围，只继承背景与光照，不继承 < 视频 1 > 中的角色主体。\n\n"));
    out.push_str("<视频 1>：原始视频，**作为动作、姿态、手脚运动、身体转向、动作节奏、停顿、镜头运动、切镜、变焦、构图透视的唯一主参考，禁止改动动作**。\n\n");
    out.push_str(&format!("谁换成谁、长什么样、画风：{user}\n\n"));
    let who = (1..=n.max(1)).map(|k| format!("<主体 {k}>")).collect::<Vec<_>>().join("、");
    out.push_str(&format!("摘要：[动作迁移 + 参考生成] 目标视频基于 < 视频 1 > 做动作迁移，把原视频人物替换成 {who}。"));
    if n > 1 {
        out.push_str("**按角色替换，不按左右位置**；每个镜头里的人数和 < 视频 1 > 那一刻一致，特写保持一个人，不凭空加人，几个人的长相和衣服不能互换、不能混合。");
    }
    out.push_str("**100% 复刻原视频全部动作、运镜、切镜，禁止新增、删减、提前、延迟任何肢体动作。**\n\n");
    out.push_str("## 画面文字规则\n\n画面中不出现任何字幕、标题、花字、标识、水印、界面元素、可读文字或伪文字。**参考视频自带的水印、平台 logo、播放器界面全部视为画面噪声，必须擦除并重绘背景，绝对不能复刻。**");
    out
}

// ---------------------------------------------------------------------------
// 原片复刻 · 快：WeeTodd / MLX H3 + 原视频当参考（2026-09-24 Kotte 8 段跑通的那一套）
// ---------------------------------------------------------------------------
//
// 做法照「H3 动作迁移 2.0」（原视频当 `<Video 1>`、三视图当 `<Picture N>`、提示词照「逐毫秒母版」），
// 走 WeeTodd 的图 —— 就是「人物清晰」那张 [`build_h3_motion_graph`]，参考视频从深度换成原片。
// 8.5 秒实测 65 分钟（原生要 2.5 小时，已删），第 1 段人全换了、没漏三视图、没带水印
// （`docs/reports/kotte-2026-09-23/h3-vram-opt/REVIEW.md`）。
//
// **参考视频原样给，不缩放、不改帧率**：跑通的那一次喂的是用户自己切好的 1920×1080 / 30 fps 分段，
// 缩放交给 WeeTodd 节点（`match output`）。只有起点不是 0、或者只取其中一截时才截一刀，截的时候也不动尺寸和帧率。

/// 一次最长几秒 —— 和「人物清晰」同一个上限
pub const MAX_SECS_H3_REF: f64 = 10.0;
/// 一次最短几秒：WeeTodd 的 `WeeToddH3GenerationConfig.duration_seconds` 下限就是 2.5（5 秒以下它标成实验性）。
/// 以前这里兜底 1 秒，设短了 ComfyUI 直接拒收整张图（2026-09-25 切 Kotte Part01 时发现）
pub const MIN_SECS_H3_REF: f64 = 2.5;

/// 母版写的是中文的「图片1」「参考视频1」（Kotte V3 写「视频1」），WeeTodd 认 `<Picture 1>` / `<Video 1>`。**只换标签，一个字都不改**。
///
/// 标签写全了 [`h3_motion_prompt_multi`] 才不在前面补那段「Video 1 是深度参考」的英文说明 —— 这条路的参考视频是原片，那段说明是错的。
/// 用户没按母版写（没有「图片1」这类字样）就套原生那条的中文模板，再换标签。
pub fn h3_ref_prompt(user: &str, n: usize) -> String {
    let n = n.max(1);
    let tag = |s: &str| {
        let s = s.replace("< 视频 1 >", "<Video 1>").replace("<视频 1>", "<Video 1>").replace("参考视频1", "<Video 1>");
        // Kotte V3（2026-09-24，Codex 写的 14 段）写的是「视频1」—— 不换的话标签不全，会被套上一层模板把开头打乱。
        // 只换后面不跟数字的：「视频10」这种不是标签。泛指的「视频」（「以视频可见关系为准」）不动
        let mut s = replace_tag_word(&s, "视频1", "<Video 1>");
        for k in 1..=n {
            s = s.replace(&format!("< 图片 {k} >"), &format!("<Picture {k}>")).replace(&format!("<图片 {k}>"), &format!("<Picture {k}>"))
                .replace(&format!("图片{k}"), &format!("<Picture {k}>"));
        }
        s
    };
    let own = tag(user.trim());
    if (1..=n).all(|k| own.contains(&format!("<Picture {k}>"))) && own.contains("<Video 1>") {
        return own;
    }
    tag(&h3_native_prompt(user, n))
}

/// 把 `word` 换成 `to`，但只换后面**不紧跟数字**的那些（「视频1」换，「视频12」不换）
fn replace_tag_word(s: &str, word: &str, to: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(word) {
        let after = &rest[i + word.len()..];
        out.push_str(&rest[..i]);
        if after.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            out.push_str(word);
        } else {
            out.push_str(to);
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

async fn h3_ref_motion(config_dir: &Path, args: &DepthVideoArgs, image_paths: &[PathBuf]) -> Result<DepthVideoResult> {
    let src = args.source_video_path.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(PathBuf::from)
        .filter(|p| p.is_file())
        .ok_or_else(|| Error::config("原片复刻要用原视频本身 —— 在上游连一个有视频的节点"))?;
    let want = args.duration_secs.filter(|v| v.is_finite() && *v > 0.0).unwrap_or(DEFAULT_SECS);
    let secs = want.clamp(MIN_SECS_H3_REF, MAX_SECS_H3_REF);
    let frames = h3_native_frames(secs);
    let (sw, sh) = crate::agent_runtime::video_tool::video_dimensions(&src)?;
    let size = if sh > sw { (H3N_LANDSCAPE.1, H3N_LANDSCAPE.0) } else { H3N_LANDSCAPE };
    let id = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
    let work = canvas_media_dir(config_dir, "depth_video").join(&id);
    std::fs::create_dir_all(&work).map_err(|e| Error::config(format!("建工作目录失败：{e}")))?;
    let _ = std::fs::write(work.join("args.json"), serde_json::to_string_pretty(&json!({
        "start_secs": args.start_secs, "duration_secs": args.duration_secs, "engine": "h3_ref",
        "source_video_path": src.to_string_lossy(),
        "image_paths": image_paths.iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<_>>(),
    })).unwrap_or_default());
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("mp4").to_string();
    let clip = work.join(format!("reference.{ext}"));
    let whole = duration_secs(&src);
    if args.start_secs <= 0.0 && secs + 0.1 >= whole {
        // 起点 0、要的比整段还长：原样拷，和跑通那次喂的是同一个文件
        std::fs::copy(&src, &clip).map_err(|e| Error::runtime(format!("拷原视频失败：{e}")))?;
    } else {
        run_ffmpeg(&["-v", "error", "-y", "-ss", &format!("{:.3}", args.start_secs.max(0.0)), "-i", &src.to_string_lossy(), "-t", &format!("{secs:.3}"),
            "-an", "-c:v", "libx264", "-crf", "12", "-pix_fmt", "yuv420p", &clip.to_string_lossy()])
            .map_err(|e| Error::runtime(format!("截原视频失败：{e}")))?;
    }
    if count_frames(&clip) < FPS {
        return Err(Error::config("原视频从这个起点往后不到 1 秒 —— 起始秒往前挪"));
    }
    let video_name = format!("anima_h3r_{id}.{ext}");
    let image_names: Vec<String> = (0..image_paths.len()).map(|k| format!("anima_h3r_{id}_p{}.{}", k + 1,
        image_paths[k].extension().and_then(|e| e.to_str()).unwrap_or("png"))).collect();
    let mut copied = false;
    for dir in crate::agent_runtime::video_tool::find_comfyui_input_dirs() {
        copied |= std::fs::copy(&clip, dir.join(&video_name)).is_ok()
            && image_paths.iter().zip(&image_names).all(|(p, n)| std::fs::copy(p, dir.join(n)).is_ok());
    }
    if !copied {
        return Err(Error::config("复制进 ComfyUI 的 input 失败"));
    }
    let client = reqwest::Client::builder().no_proxy().timeout(std::time::Duration::from_secs(120)).build().unwrap_or_default();
    if !crate::agent_runtime::weetodd::node_available(&client, "WeeToddH3ReferenceVideo").await {
        return Err(Error::config("ComfyUI 里没有 WeeTodd 的 WeeToddH3ReferenceVideo 节点 —— ComfyUI 没开，或者 WeeTodd-Nodes 版本太旧"));
    }
    crate::llm_memory::free_for_video(config_dir);
    let seed = args.seed.unwrap_or_else(|| crate::assets::now_ms() % 1_000_000_007);
    let graph = build_h3_motion_graph(&image_names, &video_name, &h3_ref_prompt(&args.prompt, image_paths.len()), seed, size, frames, "Anima/canvas_h3_ref");
    let out = work.join("video.mp4");
    // 等多久不设上限，看 ComfyUI 的进度（8.5 秒实测 65 分钟）—— 见 `comfy_watch`
    let bytes = crate::agent_runtime::video_tool::submit_and_fetch_video(&client, &graph, "画布 · 原片复刻 · 快（WeeTodd）", Some(&out))
        .await
        .map_err(Error::runtime)?;
    std::fs::write(&out, &bytes).map_err(|e| Error::runtime(format!("写视频失败：{e}")))?;
    crate::agent_runtime::video_tool::finalize_shot_audio(&out, false);
    let mut notes = vec![format!("原片复刻 · 快（WeeTodd）· {}×{} · {} 帧（{:.1} 秒）· seed {seed}", size.0, size.1, frames, (frames - 1) as f64 / FPS as f64)];
    if want > MAX_SECS_H3_REF {
        notes.push(format!("这条路一次最长 {MAX_SECS_H3_REF:.0} 秒，按 {MAX_SECS_H3_REF:.0} 秒出；更长的分段从后面的起始秒再出一条"));
    }
    if image_paths.len() > 1 {
        notes.push(format!("连了 {} 张人物图：图片1…图片{} 按连线顺序；多人时在提示词里按角色写清谁换成谁", image_paths.len(), image_paths.len()));
    }
    Ok(DepthVideoResult { path: out.to_string_lossy().to_string(), first_frame: String::new(), notes })
}

/// H3 要靠 `<Picture 1>` / `<Video 1>` 知道哪张图管身份、哪段视频管动作。用户没写就补上。
pub fn h3_motion_prompt(user: &str) -> String {
    let user = user.trim();
    if user.contains("<Picture 1>") && user.contains("<Video 1>") {
        return user.to_string();
    }
    format!(
        "<Picture 1> defines the person's identity, face, hairstyle, hair length and clothing. \
         <Video 1> is a depth guide that defines only the target motion, rhythm and camera behavior; \
         do not copy its grey look, and ignore the outline of hair and clothing in <Video 1>: \
         keep exactly the hair length and the outfit from <Picture 1>, do not add jackets, shirts or extra layers. \
         The person from <Picture 1> performs the exact motion from <Video 1>. {user}"
    )
}

/// 多人版。一个人时和 [`h3_motion_prompt`] 一字不差。
///
/// 写法照 2026-09-23 用户找的那篇 LibTV 教程（`docs/reports/2026-09-23-多人动作迁移调研.md`）：
/// 每人绑一张图、不许互换不许混特征、不许凭空加人。**不写死「全程 N 个人」** —— 快切里一个人的特写会被硬塞进第二个人；
/// 改成「每个镜头画参考视频里那一刻有几个人」。默认按首帧从左到右，
/// 用户在描述里按空间特征（前景 / 背景、背对 / 面向镜头、体型大小）另写了绑定就以描述为准。
pub fn h3_motion_prompt_multi(user: &str, n: usize) -> String {
    if n <= 1 {
        return h3_motion_prompt(user);
    }
    let user = user.trim();
    if (1..=n).all(|k| user.contains(&format!("<Picture {k}>"))) && user.contains("<Video 1>") {
        return user.to_string();
    }
    let pics: Vec<String> = (1..=n).map(|k| format!("<Picture {k}>")).collect();
    let order: Vec<String> = (1..=n).map(|k| format!("the {} person from the left is <Picture {k}>", ordinal(k))).collect();
    format!(
        "There are {n} characters. {} each define one character's identity, face, hairstyle, hair length and clothing. \
         <Video 1> is a depth guide that defines only the timeline, the cuts, every person's position, motion and rhythm, and the camera; \
         do not copy its grey look, and ignore the outline of hair and clothing in <Video 1>. \
         If <Video 1> contains cuts, follow them exactly; in every shot show exactly as many people as <Video 1> shows at that moment \
         (a close-up of one person stays one person). \
         Character binding: in the first frame of <Video 1>, {}. If the description below binds the people differently \
         (by role or action, foreground or background, facing or with the back to the camera, bigger or smaller), follow the description. \
         Every character keeps their own picture's face and outfit in every shot; never swap identities, never mix features between people, \
         never add a person who is not in <Video 1>. {user}",
        pics.join(", "),
        order.join(", ")
    )
}

fn ordinal(k: usize) -> &'static str {
    ["first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth", "ninth"].get(k - 1).copied().unwrap_or("next")
}

/// 保留原视频背景时，给 Qwen 编辑的那句话。图 1 是原视频那一帧、图 2 是人物。
///
/// 2026-09-14 实测：Qwen 编辑墙面、窗影、地板全保留，女孩做出了舞姿、很像本人，
/// 但人被挪到了画面中间、放大了；拿它当首帧跑 LTX 深度，第 20 帧左右人就走到了原舞者的位置、
/// 之后动作全程跟住，背景一直在。Krea2Edit 背景也保留，但人只是站着、偏小，没用它。
pub fn background_swap_prompt(user: &str) -> String {
    let user = user.trim();
    let mut p = String::from(
        "把图1里的人换成图2里的人物。保持图1的背景、墙面、地面、光线和机位完全不变；\
         人物的姿势、位置、身体大小和图1里的那个人一模一样。人物的脸、发型、服装、鞋子都和图2一致。画面里只有一个人。",
    );
    if !user.is_empty() {
        p.push_str("人物描述：");
        p.push_str(user);
    }
    p
}

/// 多人换人首帧的话。图 1 是原片那一帧，图 2 起是要换上去的人。
///
/// **默认按从左到右对应**：原片里从左数第 k 个人换成图 k+1。提示词里另外写了谁换谁，以提示词为准。
/// 人数写死成 n —— 网上的博主踩过「参考图比人多一张，凭空多出一个人」
/// （`docs/reports/2026-09-23-多人动作迁移调研.md`）。
pub fn background_swap_prompt_multi(user: &str, n: usize) -> String {
    if n <= 1 {
        return background_swap_prompt(user);
    }
    let mut p = format!("图1是一张原始画面，里面有 {n} 个人。把这 {n} 个人分别换成后面 {n} 张图里的人物：");
    for k in 1..=n {
        p.push_str(&format!("图1里从左数第 {k} 个人换成图{} 里的人物；", k + 1));
    }
    p.push_str(
        "保持图1的背景、墙面、地面、光线和机位完全不变；每个人的姿势、位置、身体大小和图1里对应的那个人一模一样。\
         每个人的脸、发型、服装、鞋子都和各自的参考图一致，不要互相混。",
    );
    p.push_str(&format!("画面里一共只有 {n} 个人，不多也不少。"));
    let user = user.trim();
    if !user.is_empty() {
        p.push_str("如果下面的描述另外写了谁换成谁，以描述为准。人物描述：");
        p.push_str(user);
    }
    p
}

/// Qwen 2.1 参考生图最多吃 10 张图：原片那一帧占一张，人最多 9 个。
pub const MAX_MOTION_PEOPLE: usize = 9;

fn run_ffmpeg(args: &[&str]) -> std::result::Result<(), String> {
    let out = std::process::Command::new(crate::agent_runtime::video_tool::ffmpeg_binary())
        .args(args)
        .output()
        .map_err(|e| format!("调不起 ffmpeg：{e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("ffmpeg 失败").to_string())
    }
}

fn count_frames(path: &Path) -> u32 {
    std::process::Command::new(crate::agent_runtime::video_tool::ffprobe_binary())
        .args(["-v", "error", "-count_frames", "-select_streams", "v:0", "-show_entries", "stream=nb_read_frames", "-of", "csv=p=0"])
        .arg(path)
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        .unwrap_or(0)
}

fn duration_secs(path: &Path) -> f64 {
    std::process::Command::new(crate::agent_runtime::video_tool::ffprobe_binary())
        .args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"])
        .arg(path)
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        .unwrap_or(0.0)
}

#[derive(Debug, Deserialize)]
pub struct DepthVideoArgs {
    /// 深度动作捕捉节点出的深度视频
    pub depth_path: String,
    /// 人物图。三视图也行，会自动挑正面全身
    pub image_path: String,
    /// **多个人物图**，按从左到右对应原片里的人。空 = 只用 `image_path` 一张。
    /// 多于一张时强制走「保留背景」那条路：首帧用 Qwen 一次把所有人都换掉（贴图对位一次只能贴一个人）
    #[serde(default)]
    pub image_paths: Vec<String>,
    /// 控制信号：`canny`（默认，实测最强）或 `depth`（大脸特写回落用）。H3 那条不看这个
    #[serde(default)]
    pub control: Option<String>,
    pub prompt: String,
    /// 从深度视频第几秒开始取，默认 0
    #[serde(default)]
    pub start_secs: f64,
    /// 取几秒，跟节点设置里的时长走。空 = 5 秒；超过这条路的上限按上限
    #[serde(default)]
    pub duration_secs: Option<f64>,
    /// 清晰度，跟节点设置走：`480P` / `720P` / `1080P`。空的话看老字段 `hd`
    #[serde(default)]
    pub resolution: Option<String>,
    #[serde(default)]
    pub seed: Option<u64>,
    #[serde(default)]
    pub generate_audio: bool,
    /// `ltx`（动作精准，约 2~3 分钟，默认）、`h3`（人物清晰，约 17 分钟）或 `h3_ref`
    /// （原片复刻 · 快：原视频直接当参考、走 WeeTodd；8.5 秒约 65 分钟，不用深度）。
    /// 老的 `h3_native`（原生 H3，2026-09-24 删了）按 `h3_ref` 走
    #[serde(default)]
    pub engine: Option<String>,
    /// 老字段：高清 = 720P。新节点传 `resolution`
    #[serde(default)]
    pub hd: bool,
    /// 保留原视频背景（只对 LTX 生效）：先用 Qwen 编辑把原视频这一帧里的人换成人物图，再当首帧
    #[serde(default)]
    pub keep_background: bool,
    /// 深度是从哪段视频抽的。保留背景时要从它截帧
    #[serde(default)]
    pub source_video_path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DepthVideoResult {
    pub path: String,
    /// 自动对位后的首帧，节点上给人看「模型收到的第一帧是什么」
    pub first_frame: String,
    pub notes: Vec<String>,
}

#[tauri::command]
pub async fn canvas_depth_video(state: tauri::State<'_, crate::AppState>, args: DepthVideoArgs) -> Result<DepthVideoResult> {
    depth_video_with(&state.config_dir, args).await
}

pub async fn depth_video_with(config_dir: &Path, args: DepthVideoArgs) -> Result<DepthVideoResult> {
    let depth = PathBuf::from(args.depth_path.trim());
    let engine = args.engine.as_deref().map(str::trim);
    // 原片复刻两条（原生 / 快）拿原视频当参考，不看深度 —— 上游的深度节点不用跑
    let by_source = matches!(engine, Some("h3_native") | Some("h3_ref"));
    if !by_source && !depth.is_file() {
        return Err(Error::config("深度视频不存在 —— 先在上游「深度动作捕捉」节点跑一次"));
    }
    let image_paths: Vec<PathBuf> = {
        let many: Vec<PathBuf> = args.image_paths.iter().map(|p| p.trim()).filter(|p| !p.is_empty()).map(PathBuf::from).collect();
        if many.is_empty() { vec![PathBuf::from(args.image_path.trim())] } else { many }
    };
    if image_paths.len() > MAX_MOTION_PEOPLE {
        return Err(Error::config(format!("连了 {} 张人物图，一次最多换 {MAX_MOTION_PEOPLE} 个人", image_paths.len())));
    }
    if by_source {
        if args.prompt.trim().is_empty() {
            return Err(Error::config("写一句提示词：谁换成谁、长什么样、什么画风"));
        }
        if let Some(p) = image_paths.iter().find(|p| !p.is_file()) {
            return Err(Error::config(format!("人物图不存在：{}", p.display())));
        }
        // `h3_native`（原生那条）2026-09-24 删了，老画布里存着的也走这里
        return h3_ref_motion(config_dir, &args, &image_paths).await;
    }
    let mut figures = Vec::with_capacity(image_paths.len());
    for p in &image_paths {
        figures.push(image::open(p).map_err(|e| Error::config(format!("读不了人物图 {}：{e}", p.display())))?.to_rgb8());
    }
    let figure = figures[0].clone();
    let multi = figures.len() > 1;
    if args.prompt.trim().is_empty() {
        return Err(Error::config("写一句提示词：人物长什么样、在什么地方做什么"));
    }
    if args.engine.as_deref().map(str::trim) != Some("h3") {
        for (i, f) in figures.iter().enumerate() {
            if !figure_is_whole(f) {
                return Err(Error::config(format!(
                    "「动作精准」要完整的全身人像：头到脚都在图里，四周留有背景。{}这张图里的人被裁到了图片边缘，\
                     模型锁不住人，出来会换成别人。换一张全身图，或者改用「人物清晰」—— 半身图也行",
                    if multi { format!("第 {} 张（{}）", i + 1, image_paths[i].file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()) } else { String::new() }
                )));
            }
        }
    }
    let id = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
    let work = canvas_media_dir(config_dir, "depth_video").join(&id);
    std::fs::create_dir_all(&work).map_err(|e| Error::config(format!("建工作目录失败：{e}")))?;
    // 记下这一段是从原片第几秒开始的 —— 整片分段跑完之后，拼接脚本靠它认段
    // （H3 那条不换首帧、没有 source_frame.png，按画面认不出来）
    let _ = std::fs::write(work.join("args.json"), serde_json::to_string_pretty(&json!({
        "start_secs": args.start_secs, "duration_secs": args.duration_secs,
        "engine": args.engine, "control": args.control, "source_video_path": args.source_video_path,
        "image_paths": image_paths.iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<_>>(),
    })).unwrap_or_default());

    // 1. 深度视频规整：横竖跟源走，裁满画面，24fps，按设置的时长取
    let (sw, sh) = crate::agent_runtime::video_tool::video_dimensions(&depth)?;
    let h3 = args.engine.as_deref().map(str::trim) == Some("h3");
    let resolution = args.resolution.clone().unwrap_or_else(|| if args.hd { "720P".into() } else { "480P".into() });
    let (size, tier) = motion_size(sh > sw, h3, &resolution);
    let (secs, clipped) = motion_secs(args.duration_secs, h3);
    let max_frames = (secs * FPS as f64).round() as u32 + 1;
    let norm = work.join("depth.mp4");
    let vf = format!("fps={FPS},scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h}", w = size.0, h = size.1);
    let start = format!("{:.3}", args.start_secs.max(0.0));
    let max = max_frames.to_string();
    run_ffmpeg(&["-v", "error", "-y", "-ss", &start, "-i", &depth.to_string_lossy(), "-vf", &vf, "-frames:v", &max, "-an", "-pix_fmt", "yuv420p", &norm.to_string_lossy()])
        .map_err(|e| Error::runtime(format!("整理深度视频失败：{e}")))?;
    let frames = ltx_frames(count_frames(&norm), max_frames).ok_or_else(|| Error::config("深度视频从这个起点往后不到 1 秒 —— 起始秒往前挪"))?;

    let keep_bg = (args.keep_background || multi) && !h3;  // H3 不用首帧：身份全程看参考图
    let source_video = args.source_video_path.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(PathBuf::from);
    if keep_bg && !source_video.as_ref().is_some_and(|p| p.is_file()) {
        return Err(Error::config(if multi {
            "多人换人要从原视频截首帧 —— 让上游的深度动作捕捉重新跑一次（它会记下原视频在哪）"
        } else {
            "保留原视频背景要知道原视频在哪 —— 让上游的深度动作捕捉重新跑一次"
        }));
    }

    // 2. 首帧对位（H3 不用首帧，人物图整张当身份参考）
    let first0 = work.join("depth0.png");
    run_ffmpeg(&["-v", "error", "-y", "-i", &norm.to_string_lossy(), "-frames:v", "1", &first0.to_string_lossy()]).map_err(Error::runtime)?;
    let depth0 = image::open(&first0).map_err(|e| Error::runtime(format!("读深度第一帧失败：{e}")))?.to_luma8();
    let first_frame = work.join("first_frame.png");
    let client = reqwest::Client::builder().no_proxy().timeout(std::time::Duration::from_secs(120)).build().unwrap_or_default();
    if h3 {
        figure.save(&first_frame).map_err(|e| Error::runtime(format!("写人物图失败：{e}")))?;
    } else if keep_bg {
        // 从原视频截同一起点、同一裁切的那一帧 —— 裁法和深度视频一模一样，否则首帧和深度对不上位
        let src_frame = work.join("source_frame.png");
        run_ffmpeg(&["-v", "error", "-y", "-ss", &start, "-i", &source_video.as_ref().unwrap().to_string_lossy(), "-vf", &vf, "-frames:v", "1", &src_frame.to_string_lossy()])
            .map_err(|e| Error::runtime(format!("从原视频截帧失败：{e}")))?;
        // 每张人物图各裁出人来；图的顺序就是「从左数第几个人」的顺序
        let scene_name = format!("anima_bgswap_scene_{id}.png");
        let mut refs = vec![scene_name.clone()];
        let mut crops = Vec::with_capacity(figures.len());
        for (i, f) in figures.iter().enumerate() {
            let (fx0, fy0, fx1, fy1, _) = figure_box(f);
            let crop = work.join(format!("figure_{i}.png"));
            image::imageops::crop_imm(f, fx0, fy0, fx1 - fx0 + 1, fy1 - fy0 + 1)
                .to_image()
                .save(&crop)
                .map_err(|e| Error::runtime(format!("写人物裁图失败：{e}")))?;
            let name = if i == 0 { format!("anima_bgswap_person_{id}.png") } else { format!("anima_bgswap_person{}_{id}.png", i + 1) };
            refs.push(name.clone());
            crops.push((crop, name));
        }
        let mut ok = false;
        for dir in crate::agent_runtime::video_tool::find_comfyui_input_dirs() {
            ok |= std::fs::copy(&src_frame, dir.join(&scene_name)).is_ok()
                && crops.iter().all(|(crop, name)| std::fs::copy(crop, dir.join(name)).is_ok());
        }
        if !ok {
            return Err(Error::config("复制进 ComfyUI 的 input 失败"));
        }
        let seed0 = args.seed.unwrap_or(20_260_914);
        // **这里换过引擎（2026-09-22）**：Qwen-Image-Edit-2511 → Qwen-Image 2.1 参考生图。
        // 同题对照下 2.1 更快、更听话、参考槽更多，2511 的权重已经删掉了。
        let mut g = crate::agent_runtime::image_engine::build_qwen21_graph(
            &background_swap_prompt_multi(&args.prompt, figures.len()), "", size.0, size.1, seed0,
            crate::agent_runtime::image_engine::QWEN21_STEPS,
            &refs,
        );
        if let Some(save) = g.get_mut(crate::agent_runtime::image_engine::save_node_id(
            crate::agent_runtime::image_engine::ImageEngine::Qwen21Edit,
        )) {
            save["inputs"]["filename_prefix"] = json!("Anima/canvas_bgswap");
        }
        crate::llm_memory::free_for_video(config_dir);
        let bytes = crate::agent_runtime::video_tool::submit_and_fetch_video(&client, &g, "画布 · 动作迁移换人首帧", None)
            .await
            .map_err(|e| Error::runtime(format!("保留背景的首帧没出来：{e}")))?;
        // **必须自己收到精确尺寸。**
        //
        // 2511 那张图用的是 `EmptySD3LatentImage`，宽高照传、出来就是 `size`。
        // 2.1 不是：有参考图时 latent 由 `TextEncodeQwenImage21` 自己开，
        // 尺寸按**第一张参考图的长宽比** + `resolution` 折算再对齐到 32 的倍数
        // （960×540 会出成 928×528）。首帧和深度视频差几个像素，
        // 下游不报错，只是**动作和画面错位**。
        exact_size_png(&bytes, size, &first_frame)?;
    } else {
        let aligned = align_first_frame(&depth0, &figure, size);
        aligned.save(&first_frame).map_err(|e| Error::runtime(format!("写首帧失败：{e}")))?;
    }

    // 3. 控制信号。canny 从**原视频**抽，和深度视频同一起点、同一裁切、同一帧率规整 —— 不然和首帧对不上位
    let want_canny = !h3 && args.control.as_deref().map(str::trim) != Some("depth");
    let canny_src = if want_canny { source_video.as_ref().filter(|p| p.is_file()) } else { None };
    let src_norm = work.join("source.mp4");
    if let Some(src) = canny_src {
        run_ffmpeg(&["-v", "error", "-y", "-ss", &start, "-i", &src.to_string_lossy(), "-vf", &vf, "-frames:v", &max, "-an", "-pix_fmt", "yuv420p", &src_norm.to_string_lossy()])
            .map_err(|e| Error::runtime(format!("整理原视频失败：{e}")))?;
    }

    // 4. 进 ComfyUI
    let depth_name = format!("anima_depthctl_{id}.mp4");
    let frame_name = format!("anima_depthctl_{id}.png");
    let source_name = format!("anima_srcctl_{id}.mp4");
    let mut copied = false;
    // H3：每张人物图原样当 `<Picture k>`；第一张沿用 frame_name（就是 figures[0]）
    let mut h3_images = vec![frame_name.clone()];
    let mut h3_files = Vec::new();
    if h3 {
        for (k, f) in figures.iter().enumerate().skip(1) {
            let file = work.join(format!("figure_{k}.png"));
            f.save(&file).map_err(|e| Error::runtime(format!("写人物图失败：{e}")))?;
            let name = format!("anima_depthctl_{id}_p{}.png", k + 1);
            h3_images.push(name.clone());
            h3_files.push((file, name));
        }
    }
    for dir in crate::agent_runtime::video_tool::find_comfyui_input_dirs() {
        copied |= std::fs::copy(&norm, dir.join(&depth_name)).is_ok()
            && std::fs::copy(&first_frame, dir.join(&frame_name)).is_ok()
            && (canny_src.is_none() || std::fs::copy(&src_norm, dir.join(&source_name)).is_ok())
            && h3_files.iter().all(|(file, name)| std::fs::copy(file, dir.join(name)).is_ok());
    }
    if !copied {
        return Err(Error::config("复制进 ComfyUI 的 input 失败"));
    }
    let probe = if h3 { "WeeToddH3ReferenceVideo" } else { "WeeToddLTX25ICLoRAControlGuide" };
    if !crate::agent_runtime::weetodd::node_available(&client, probe).await {
        return Err(Error::config(format!("ComfyUI 里没有 WeeTodd 的 {probe} 节点 —— ComfyUI 没开，或者 WeeTodd-Nodes 版本太旧")));
    }
    crate::llm_memory::free_for_video(config_dir);
    let seed = args.seed.unwrap_or_else(|| crate::assets::now_ms() % 1_000_000_007);
    let graph = if h3 {
        build_h3_motion_graph(&h3_images, &depth_name, args.prompt.trim(), seed, size, frames, "Anima/canvas_depth_video_h3")
    } else {
        let control = if canny_src.is_some() { MotionControl::Canny { source_name: &source_name } } else { MotionControl::Depth };
        build_depth_video_graph(&depth_name, &frame_name, args.prompt.trim(), seed, size, frames, "Anima/canvas_depth_video", control)
    };
    let out = work.join("video.mp4");
    let bytes = crate::agent_runtime::video_tool::submit_and_fetch_video(&client, &graph, "画布 · 深度动作迁移", Some(&out))
        .await
        .map_err(Error::runtime)?;
    std::fs::write(&out, &bytes).map_err(|e| Error::runtime(format!("写视频失败：{e}")))?;
    crate::agent_runtime::video_tool::finalize_shot_audio(&out, args.generate_audio);

    let mut notes = vec![format!(
        "{} · {tier} {}×{} · {} 帧（{:.1} 秒）· seed {seed}",
        if h3 { "人物清晰（H3）" } else if keep_bg { "动作精准（LTX）· 保留原视频背景" } else { "动作精准（LTX）" },
        size.0, size.1, frames, (frames - 1) as f64 / FPS as f64
    )];
    if !h3 {
        notes.push(if canny_src.is_some() {
            "控制：轮廓线（canny，从原视频抽）".to_string()
        } else if want_canny {
            "控制：深度 —— 想用轮廓线但找不到原视频，让上游的深度动作捕捉重新跑一次".to_string()
        } else {
            "控制：深度".to_string()
        });
    }
    if multi {
        notes.push(if h3 {
            format!("连了 {} 张人物图：{} 个人全程各看各的参考图，默认按首帧从左到右对应（提示词里另写了绑定，以提示词为准）", figures.len(), figures.len())
        } else {
            format!("连了 {} 张人物图：首帧用 Qwen 一次把 {} 个人都换掉，默认按从左到右对应（提示词里另写了谁换谁，以提示词为准）", figures.len(), figures.len())
        });
    }
    if h3 && !tier.eq_ignore_ascii_case(resolution.trim()) {
        notes.push(format!("人物清晰这条路最高 {tier}，按 {tier} 出"));
    }
    if clipped {
        notes.push(format!("这条路一次最长 {secs:.0} 秒，按 {secs:.0} 秒出；更长的分段从后面的起始秒再出一条"));
    }
    if frames + 8 < max_frames {
        notes.push(format!("深度视频从起点往后不足 {secs:.0} 秒，按实际长度出"));
    }
    // H3 没有首帧，别把人物图当「对位后的首帧」显示出去
    let first_frame = if h3 { String::new() } else { first_frame.to_string_lossy().to_string() };
    Ok(DepthVideoResult { path: out.to_string_lossy().to_string(), first_frame, notes })
}

#[cfg(test)]
mod tests {
    /// 把 H3 动作迁移的图导出来，拿去直接提交 ComfyUI（打样、对比用）。**默认不跑。**
    ///
    /// ```text
    /// H3M_IMAGES=a.png,b.png H3M_VIDEO=seg.mp4 H3M_PROMPT_FILE=p.txt H3M_SEED=1 H3M_SIZE=960x544 \
    ///   H3M_FRAMES=209 H3M_OUT=/tmp/g.json cargo test --lib 导出h3动作迁移的图 -- --ignored
    /// ```
    /// 2026-09-24 Kotte 8 段走 WeeTodd 跑通的那一次（`docs/reports/kotte-2026-09-23/h3-vram-opt/wt_run.py`）：
    /// 画布「原片复刻 · 快」发出去的提示词和图必须和那次**一字不差** —— 用户要的是「和现在跑的这一套完全一致」。
    #[test]
    #[ignore = "公开版不含 docs/reports 里的实测提示词"]
    fn 原片复刻快_提示词和图和kotte实跑一字不差() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/reports/kotte-2026-09-23/h3-vram-opt");
        let cn = std::fs::read_to_string(dir.join("parts/part01_prompt.txt")).unwrap();
        let sent = std::fs::read_to_string(dir.join("parts_wt/part01_prompt.txt")).unwrap();
        let ours = super::h3_ref_prompt(&cn, 2);
        assert_eq!(ours, sent, "只换标签，别的一个字不动");
        let imgs = vec!["kotte_h3m2_charA.png".to_string(), "kotte_h3m2_charB.png".to_string()];
        let frames = super::h3_native_frames(8.542);
        assert_eq!(frames, 209);
        let g = super::build_h3_motion_graph(&imgs, "kotte_kotte_part_01.mp4", &ours, 62001, (960, 544), frames, "Anima/h3m_export");
        let ran: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("parts_wt/part01_graph.json")).unwrap()).unwrap();
        assert_eq!(g, ran, "整张图和实跑那张一样");
        // 8 段都得换全标签 —— 漏一个，后端就会在前面补那段「深度参考」的英文说明
        for k in 1..=8 {
            let cn = std::fs::read_to_string(dir.join(format!("parts/part{k:02}_prompt.txt"))).unwrap();
            let t = super::h3_ref_prompt(&cn, 2);
            assert!(t.contains("<Picture 1>") && t.contains("<Picture 2>") && t.contains("<Video 1>"), "第 {k} 段");
            assert!(!t.contains("图片1") && !t.contains("参考视频1"), "第 {k} 段还有没换的中文标签");
            assert_eq!(super::h3_motion_prompt_multi(&t, 2), t, "第 {k} 段：标签齐了就原样发，不补英文说明");
        }
    }

    /// Kotte V3（Codex 写的 14 段，2026-09-24）：参考视频写的是「视频1」。14 段都得换全标签、原样发出去 ——
    /// 不认「视频1」的话标签不全，会被套一层中文模板，把人家写好的开头打乱
    #[test]
    #[ignore = "公开版不含 docs/reports 里的实测提示词"]
    fn 原片复刻快_kotte_v3的视频1也认() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/reports/kotte-v3/prompts");
        for k in 1..=14 {
            let raw = std::fs::read_to_string(dir.join(format!("Kotte-V3-Part{k:02}.txt"))).unwrap();
            let t = super::h3_ref_prompt(&raw, 2);
            // 09-25 Codex 把 Part01 单独重写了（开头是「将视频1第一段中的原角色……」），只要求以「将<Video 1>」开头、没被套模板
            assert!(t.starts_with("将<Video 1>"), "第 {k} 段被套了模板：{}", &t[..80]);
            assert!(!t.contains("视频1") && !t.contains("图片1") && !t.contains("图片2"), "第 {k} 段还有没换的标签");
            assert_eq!(super::h3_motion_prompt_multi(&t, 2), t, "第 {k} 段：标签齐了就原样发");
            // 除了标签，一个字都不动
            assert_eq!(t.replace("<Video 1>", "视频1").replace("<Picture 1>", "图片1").replace("<Picture 2>", "图片2"), raw.trim(), "第 {k} 段");
        }
        assert_eq!(super::replace_tag_word("视频1和视频12", "视频1", "<Video 1>"), "<Video 1>和视频12");
    }

    /// WeeTodd 节点的下限是 2.5 秒 —— 帧数照着 2.5 秒吸附出来的时长也得不短于它，否则 ComfyUI 拒收
    #[test]
    fn 原片复刻快_最短按2点5秒算() {
        assert_eq!(super::MIN_SECS_H3_REF, 2.5);
        let frames = super::h3_native_frames(super::MIN_SECS_H3_REF);
        assert!((frames - 1) as f64 / super::FPS as f64 >= 2.5, "{frames} 帧不够 2.5 秒");
    }

    #[test]
    fn 原片复刻快_没按母版写就套中文模板再换标签() {
        let t = super::h3_ref_prompt("开枪的换成眼镜男，跳舞的换成光头", 2);
        assert!(t.contains("<Picture 1>") && t.contains("<Picture 2>") && t.contains("<Video 1>"));
        assert!(!t.contains("< 图片") && !t.contains("<视频"), "原生模板里的中文标签也换掉：{t}");
        assert!(t.contains("开枪的换成眼镜男"));
        assert!(!t.contains("depth guide"), "参考视频是原片，不能说成深度");
    }

    #[test]
    fn 原片复刻_帧数吸附到17n加5_和实跑对得上() {
        use super::h3_native_frames as f;
        assert_eq!(f(5.0), 124, "09-24 样例 5 秒出了 124 帧");
        assert_eq!(f(8.68), 209, "Kotte 第 7 段出了 209 帧");
        assert_eq!(f(0.1), 5, "最少 5 帧");
        for tenths in 10..=100 {
            let n = f(tenths as f64 / 10.0);
            assert_eq!((n - 5) % 17, 0, "{tenths}: {n} 不在 17n+5 上");
            assert!(n as f64 >= (tenths as f64 / 10.0 * 24.0).round(), "只往上吸附");
        }
    }

    #[test]
    fn 原片复刻的提示词照原作模板_写全了标签就原样用() {
        let p = super::h3_native_prompt("开枪的换成眼镜男，跳舞的换成光头", 2);
        for must in ["<主体 1>", "< 图片 1 >", "<主体 2>", "< 图片 2 >", "<视频 1>", "不作为目标视频首帧", "只继承背景与光照", "按角色替换", "100% 复刻", "水印", "开枪的换成眼镜男"] {
            assert!(p.contains(must), "少了「{must}」：{p}");
        }
        assert!(!super::h3_native_prompt("一个人跳舞", 1).contains("按角色替换"), "一个人不用说按角色");
        let own = "<主体 1>：参考图 < 图片 1 > 目标角色……";
        assert_eq!(super::h3_native_prompt(own, 1), own, "照原作模板自己写全了就不再套");
    }

    #[test]
    #[ignore]
    fn 导出h3动作迁移的图() {
        let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("缺 {k}"));
        let images: Vec<String> = env("H3M_IMAGES").split(',').map(str::to_string).collect();
        let (w, h) = env("H3M_SIZE").split_once('x').map(|(a, b)| (a.parse().unwrap(), b.parse().unwrap())).unwrap();
        let prompt = std::fs::read_to_string(env("H3M_PROMPT_FILE")).unwrap();
        let g = super::build_h3_motion_graph(&images, &env("H3M_VIDEO"), &prompt, env("H3M_SEED").parse().unwrap(), (w, h),
            env("H3M_FRAMES").parse().unwrap(), "Anima/h3m_export");
        std::fs::write(env("H3M_OUT"), serde_json::to_string_pretty(&g).unwrap()).unwrap();
    }

    use super::*;

    /// 连线要成链：读视频 → 拆帧 → 抽深度 → 合成视频。断一处 ComfyUI 会拒收整张图。
    #[test]
    fn 深度图连线成链() {
        let g = build_depth_graph("a.mp4", "p");
        assert_eq!(g["1"]["inputs"]["file"], "a.mp4");
        assert_eq!(g["2"]["inputs"]["video"], json!(["1", 0]));
        assert_eq!(g["4"]["inputs"]["images"], json!(["2", 0]));
        assert_eq!(g["4"]["inputs"]["depth_model"], json!(["3", 0]));
        assert_eq!(g["5"]["inputs"]["images"], json!(["4", 0]));
        assert_eq!(g["3"]["inputs"]["checkpoint"], DEPTH_CHECKPOINT);
    }

    /// 帧率跟源视频走。写死 24 的话 30fps 素材的深度视频会变慢，和下游对不上帧。
    #[test]
    fn 帧率接源视频的() {
        let g = build_depth_graph("a.mp4", "p");
        assert_eq!(g["5"]["inputs"]["frame_rate"], json!(["2", 2]));
    }

    /// LTX Union 深度控制认的是「越近越白」。反了的话人物是黑的、背景是白的，控制照样生效但动作全乱。
    #[test]
    fn 深度方向是近白() {
        let g = build_depth_graph("a.mp4", "p");
        assert_eq!(g["4"]["inputs"]["depth_polarity"], "near white (recommended)");
        assert_eq!(g["4"]["inputs"]["keep_model_warm"], false);
    }

    #[test]
    fn 深度视频的输出取得到() {
        // VHS 的产物挂在 gifs 下
        let outputs = json!({"5": {"gifs": [{"filename": "d_00001.mp4", "subfolder": "Anima", "type": "output"}]}});
        let (f, sub, ty) = crate::agent_runtime::weetodd::find_video_output(&outputs).unwrap();
        assert_eq!((f.as_str(), sub.as_str(), ty.as_str()), ("d_00001.mp4", "Anima", "output"));
    }

    #[test]
    fn 保存上传视频() {
        use base64::Engine;
        let dir = std::env::temp_dir().join(format!("anima-depth-{}", uuid::Uuid::new_v4()));
        let data = base64::engine::general_purpose::STANDARD.encode(b"fake mp4 bytes");
        let p = save_video(&dir, &SaveVideoArgs { data_url: format!("data:video/mp4;base64,{data}"), filename: "舞蹈 demo.mp4".into() }).unwrap();
        assert!(p.starts_with(dir.join("anima/canvas/uploads")));
        assert_eq!(p.extension().unwrap(), "mp4");
        assert_eq!(std::fs::read(&p).unwrap(), b"fake mp4 bytes");
        // 图片、空文件都拒收
        assert!(save_video(&dir, &SaveVideoArgs { data_url: format!("data:image/png;base64,{data}"), filename: "a.png".into() }).is_err());
        assert!(save_video(&dir, &SaveVideoArgs { data_url: "data:video/mp4;base64,".into(), filename: "a.mp4".into() }).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 画布音频素材（2026-10-08）：mime 认不出时看扩展名；图片、空文件拒收
    #[test]
    fn 保存上传音频() {
        use base64::Engine;
        let dir = std::env::temp_dir().join(format!("anima-audio-{}", uuid::Uuid::new_v4()));
        let data = base64::engine::general_purpose::STANDARD.encode(b"fake mp3 bytes");
        let p = save_audio(&dir, &SaveVideoArgs { data_url: format!("data:audio/mpeg;base64,{data}"), filename: "青花瓷 15s.mp3".into() }).unwrap();
        assert!(p.starts_with(dir.join("anima/canvas/uploads")));
        assert_eq!(p.extension().unwrap(), "mp3");
        let p2 = save_audio(&dir, &SaveVideoArgs { data_url: format!("data:;base64,{data}"), filename: "a.flac".into() }).unwrap();
        assert_eq!(p2.extension().unwrap(), "flac");
        assert!(save_audio(&dir, &SaveVideoArgs { data_url: format!("data:image/png;base64,{data}"), filename: "a.png".into() }).is_err());
        assert!(save_audio(&dir, &SaveVideoArgs { data_url: "data:audio/wav;base64,".into(), filename: "a.wav".into() }).is_err());
        assert_eq!(probe_secs(&p), None, "假文件读不出时长，不编");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 腿比身体暗（离地面近）—— 光看最亮那一截会把腿切掉，人对位出来偏小、悬空。
    #[test]
    fn 深度里人的范围包括较暗的腿() {
        let mut depth = image::GrayImage::from_pixel(768, 512, image::Luma([40]));
        for x in 450..550 {
            for y in 150..350 { depth.put_pixel(x, y, image::Luma([200])); } // 躯干，最亮
            for y in 350..500 { depth.put_pixel(x, y, image::Luma([90])); }  // 腿，暗一些但比背景亮
        }
        let (_, y0, _, y1) = subject_box(&depth).unwrap();
        assert_eq!((y0, y1), (150, 499));
    }

    #[test]
    fn h3_动作图_人物图是图1_深度是视频1() {
        let g = build_h3_motion_graph(&["girl.png".to_string()], "d.mp4", "跳舞", 1, (768, 512), 121, "x");
        assert_eq!(g["20"]["inputs"]["image"], "girl.png");
        assert_eq!(g["60"]["inputs"]["file"], "d.mp4");
        assert_eq!(g["62"]["class_type"], "WeeToddH3ReferenceVideo");
        assert_eq!(g["62"]["inputs"]["previous_references"], json!(["21", 0]));
        assert_eq!(g["5"]["inputs"]["references"], json!(["62", 0]), "参考视频没接进编码");
        let p = g["5"]["inputs"]["prompt"].as_str().unwrap();
        assert!(p.contains("<Picture 1>") && p.contains("<Video 1>") && p.ends_with("跳舞"));
    }

    /// 多人：参考视频接在参考图链**末尾**，不是第一张后面（写死 21 会把后面的图挤掉）
    #[test]
    fn h3_多人_每张图都进链_参考视频接链尾() {
        let imgs: Vec<String> = ["a.png", "b.png", "c.png"].iter().map(|s| s.to_string()).collect();
        let g = build_h3_motion_graph(&imgs, "d.mp4", "三个人", 1, (768, 512), 121, "x");
        assert_eq!((g["20"]["inputs"]["image"].as_str(), g["22"]["inputs"]["image"].as_str(), g["24"]["inputs"]["image"].as_str()), (Some("a.png"), Some("b.png"), Some("c.png")));
        assert_eq!(g["62"]["inputs"]["previous_references"], json!(["25", 0]), "要接第三张的后面");
        assert_eq!(g["5"]["inputs"]["references"], json!(["62", 0]));
        let p = g["5"]["inputs"]["prompt"].as_str().unwrap();
        assert!(p.contains("There are 3 characters") && p.contains("<Picture 3>") && p.contains("never swap"), "{p}");
        assert!(p.contains("follow them exactly") && p.contains("stays one person"), "快切里的特写不能被硬塞人：{p}");
        assert!(p.contains("the first person from the left is <Picture 1>") && p.contains("the third person from the left is <Picture 3>"));
        assert!(p.ends_with("三个人"));
    }

    #[test]
    fn h3_多人提示词_一个人时和原来一样_自己写全了标签就不补() {
        assert_eq!(h3_motion_prompt_multi("跳舞", 1), h3_motion_prompt("跳舞"));
        let own = "<Picture 1> 在左 <Picture 2> 在右，照 <Video 1> 打";
        assert_eq!(h3_motion_prompt_multi(own, 2), own);
        assert!(h3_motion_prompt_multi("<Picture 1> 在左，照 <Video 1>", 2).contains("There are 2 characters"), "少写了一张图的标签就要补");
    }

    #[test]
    fn 用户自己写了标签就不重复补() {
        let p = "<Picture 1> 的女孩跳 <Video 1> 的舞";
        assert_eq!(h3_motion_prompt(p), p);
    }

    #[test]
    fn 换人首帧的话_背景不动_姿势位置跟原图() {
        let p = background_swap_prompt("校服女孩");
        assert!(p.contains("背景") && p.contains("姿势、位置、身体大小") && p.contains("只有一个人"));
        assert!(p.ends_with("校服女孩"));
        assert!(!background_swap_prompt("  ").contains("人物描述"));
    }

    #[test]
    fn 帧数取_8k加1() {
        assert_eq!(ltx_frames(121, 121), Some(121));
        assert_eq!(ltx_frames(959, 121), Some(121));
        assert_eq!(ltx_frames(959, 241), Some(241));
        assert_eq!(ltx_frames(100, 121), Some(97));
        assert_eq!(ltx_frames(25, 121), Some(25));
        assert_eq!(ltx_frames(24, 121), None);
    }

    /// 三视图：左边半身特写贴着图底，右边一个完整全身 —— 要挑全身那个。
    #[test]
    fn 三视图挑露出全身的那个() {
        let mut img = image::RgbImage::from_pixel(300, 200, image::Rgb([255, 255, 255]));
        for x in 10..90 { for y in 60..200 { img.put_pixel(x, y, image::Rgb([20, 20, 20])); } } // 半身，贴底
        for x in 150..180 { for y in 20..180 { img.put_pixel(x, y, image::Rgb([30, 30, 30])); } } // 全身，脚离底
        let (x0, y0, x1, y1, bg) = figure_box(&img);
        assert_eq!((x0, x1), (150, 179));
        assert_eq!((y0, y1), (20, 179));
        assert_eq!(bg, [255, 255, 255]);
    }

    /// 动作精准拦不完整的人：碰到上下左右任何一边都不算
    #[test]
    fn 人物图要完整露出才算全身() {
        let fig = |x0: u32, y0: u32, x1: u32, y1: u32| {
            let mut img = image::RgbImage::from_pixel(300, 200, image::Rgb([255, 255, 255]));
            for x in x0..x1 { for y in y0..y1 { img.put_pixel(x, y, image::Rgb([30, 30, 30])); } }
            img
        };
        assert!(figure_is_whole(&fig(150, 20, 180, 180)), "四周都有背景");
        assert!(!figure_is_whole(&fig(150, 20, 180, 200)), "脚被裁");
        assert!(!figure_is_whole(&fig(150, 0, 180, 180)), "头被裁");
        assert!(!figure_is_whole(&fig(0, 20, 60, 180)), "贴左边");
        assert!(!figure_is_whole(&fig(250, 20, 300, 180)), "贴右边");
        // 三视图：半身特写贴底，但旁边有一个完整全身 —— 算
        let mut sheet = fig(150, 20, 180, 180);
        for x in 10..90 { for y in 60..200 { sheet.put_pixel(x, y, image::Rgb([20, 20, 20])); } }
        assert!(figure_is_whole(&sheet));
    }

    #[test]
    fn 人物清晰的提示词要求发型衣服照人物图() {
        let p = h3_motion_prompt("跳舞");
        assert!(p.contains("hair length") && p.contains("do not add jackets"), "{p}");
    }

    /// 半身特写压住了一个角（用户那张真实三视图就是左下角）—— 背景色不能被带成灰的。
    #[test]
    fn 角被人物压住时背景色照样对() {
        let mut img = image::RgbImage::from_pixel(300, 200, image::Rgb([255, 255, 255]));
        for x in 0..90 { for y in 40..200 { img.put_pixel(x, y, image::Rgb([10, 10, 10])); } } // 贴左下角
        for x in 150..180 { for y in 20..180 { img.put_pixel(x, y, image::Rgb([30, 30, 30])); } }
        let (x0, _, x1, _, bg) = figure_box(&img);
        assert_eq!(bg, [255, 255, 255]);
        assert_eq!((x0, x1), (150, 179));
    }

    /// 深度第一帧里人在右边、占一半高 —— 人物要缩到那个高度、站到那个位置。
    #[test]
    fn 首帧对到深度里人的位置() {
        let mut depth = image::GrayImage::from_pixel(768, 512, image::Luma([40]));
        for x in 450..550 { for y in 200..500 { depth.put_pixel(x, y, image::Luma([170])); } }
        assert_eq!(subject_box(&depth), Some((450, 200, 549, 499)));
        let mut fig = image::RgbImage::from_pixel(100, 400, image::Rgb([255, 255, 255]));
        for x in 40..60 { for y in 10..390 { fig.put_pixel(x, y, image::Rgb([0, 0, 0])); } }
        let out = align_first_frame(&depth, &fig, (768, 512));
        assert_eq!(out.dimensions(), (768, 512));
        // 人的底边在 499 附近、中心在 499 附近；左上角还是背景
        assert!(out.get_pixel(499, 480).0[0] < 60, "人没站到深度里的位置");
        assert!(out.get_pixel(499, 150).0[0] > 200, "人比深度里的高");
        assert_eq!(out.get_pixel(5, 5).0, [255, 255, 255]);
        assert!(out.get_pixel(300, 400).0[0] > 200, "人站偏了");
    }

    /// canny 那张图要和 2026-09-22 实测的 `probe3_canny` 一模一样：原视频 → 拆帧 → Canny(100/200, 512) → ImageScale 回出片尺寸 → 控制
    #[test]
    fn canny_控制从原视频抽_缩回出片尺寸() {
        let g = build_depth_video_graph("d.mp4", "f.png", "p", 7, (768, 512), 73, "x", MotionControl::Canny { source_name: "src.mp4" });
        assert_eq!(g["4"]["inputs"]["file"], "src.mp4", "canny 要从原视频抽，不是深度视频");
        assert_eq!(g["20"]["class_type"], "CannyEdgePreprocessor");
        assert_eq!((g["20"]["inputs"]["low_threshold"].as_i64(), g["20"]["inputs"]["high_threshold"].as_i64(), g["20"]["inputs"]["resolution"].as_i64()), (Some(100), Some(200), Some(512)));
        assert_eq!(g["20"]["inputs"]["image"], json!(["5", 0]));
        assert_eq!(g["25"]["class_type"], "ImageScale");
        assert_eq!((g["25"]["inputs"]["width"].as_u64(), g["25"]["inputs"]["height"].as_u64()), (Some(768), Some(512)), "预处理器吐的尺寸不是出片尺寸，必须缩回去");
        assert_eq!(g["6"]["inputs"]["control_video"], json!(["25", 0]));
        assert_eq!(g["6"]["inputs"]["control_type"], "canny_edges");
        // 深度那张不带预处理节点
        let d = build_depth_video_graph("d.mp4", "f.png", "p", 7, (768, 512), 73, "x", MotionControl::Depth);
        assert!(d.get("20").is_none() && d.get("25").is_none());
        assert_eq!(d["4"]["inputs"]["file"], "d.mp4");
    }

    #[test]
    fn 多人换人的话_写明人数和从左到右的对应() {
        let p = background_swap_prompt_multi("图2换拿枪的那个", 2);
        assert!(p.contains("里面有 2 个人"));
        assert!(p.contains("从左数第 1 个人换成图2") && p.contains("从左数第 2 个人换成图3"));
        assert!(p.contains("一共只有 2 个人"), "人数写死，防止凭空多出一个");
        assert!(p.contains("以描述为准") && p.ends_with("图2换拿枪的那个"));
        // 一个人时和原来一字不差
        assert_eq!(background_swap_prompt_multi("校服女孩", 1), background_swap_prompt("校服女孩"));
    }

    #[test]
    fn 老前端不传新字段_行为不变() {
        let a: DepthVideoArgs = serde_json::from_value(json!({"depth_path": "d", "image_path": "a.png", "prompt": "p"})).unwrap();
        assert!(a.image_paths.is_empty());
        assert_eq!(a.control, None);
    }

    #[test]
    fn 出片图连线和规格() {
        let g = build_depth_video_graph("d.mp4", "f.png", "p", 7, (768, 512), 121, "x", MotionControl::Depth);
        assert_eq!(g["1"]["inputs"]["transformer"], "weetodd-q8/ltx-2.5-22b-distilled-union1p0-q8-paged");
        assert_eq!(g["2"]["inputs"]["duration_seconds"], 5.0);
        assert_eq!(g["6"]["inputs"]["control_type"], "depth_map");
        assert_eq!(g["6"]["inputs"]["end_frame"], 120);
        assert_eq!(g["9"]["inputs"]["first_frame"], json!(["8", 0]));
        assert_eq!(g["9"]["inputs"]["media_conditioning"], json!(["6", 0]));
        // 首帧直接接 first_frame，不走关键帧（同效果慢一倍）
        assert!(g.as_object().unwrap().values().all(|n| n["class_type"] != "WeeToddLTX25Keyframe"));
        // 宽高能被 128 整除
        for s in [LANDSCAPE, PORTRAIT, LANDSCAPE_HD, PORTRAIT_HD] { assert_eq!((s.0 % 128, s.1 % 128), (0, 0)); }
        assert_eq!(pick_size(false, true), (1152, 768));
        assert_eq!(pick_size(true, true), (768, 1152));
        assert_eq!(pick_size(true, false), (512, 768));
    }

    /// 清晰度、时长跟节点设置走（以前固定 768×512 / 5 秒，下面的设置不起作用）。
    #[test]
    fn 动作迁移跟设置的清晰度和时长() {
        assert_eq!(motion_size(false, false, "480P"), ((768, 512), "480P"));
        assert_eq!(motion_size(false, false, "720P"), ((1152, 768), "720P"));
        assert_eq!(motion_size(true, false, "1080P"), ((1024, 1536), "1080P"));
        assert_eq!(motion_size(false, true, "1080P"), ((1280, 736), "720P"), "H3 没有 1080P");
        assert_eq!(motion_size(true, true, "480P"), ((512, 768), "480P"));
        assert_eq!(motion_size(false, false, ""), ((768, 512), "480P"));
        for res in ["480P", "720P", "1080P"] {
            let ((w, h), _) = motion_size(false, false, res);
            assert_eq!((w % 128, h % 128), (0, 0), "LTX Union 两段式要求能被 128 整除：{res}");
        }
        assert_eq!(motion_secs(None, false), (5.0, false));
        assert_eq!(motion_secs(Some(8.0), false), (8.0, false));
        assert_eq!(motion_secs(Some(15.0), false), (MAX_SECS_LTX, true));
        assert_eq!(motion_secs(Some(15.0), true), (MAX_SECS_H3, true));
    }
}

#[cfg(test)]
mod motion_live {
    use super::*;

    /// `ANIMA_FIGURES=a.png,b.jpg` 逐张打印算不算完整全身
    #[test]
    #[ignore = "读本机图片"]
    fn 全身判断实图() {
        for p in std::env::var("ANIMA_FIGURES").unwrap().split(',') {
            let img = image::open(p).unwrap().to_rgb8();
            println!("{} → {}", figure_is_whole(&img), p);
        }
    }

    /// 动作精准同题对照：同一张人物图、同一段深度、同一个种子，只改时长。
    ///
    /// `ANIMA_MOTION_DEPTH` `ANIMA_MOTION_IMAGE` `ANIMA_MOTION_SECS=5,10` `ANIMA_MOTION_OUT`
    #[tokio::test]
    #[ignore = "要本机 ComfyUI，每条几分钟"]
    async fn 动作精准时长对照() {
        let config_dir = dirs::config_dir().unwrap().join("com.anima.desktop");
        let out = PathBuf::from(std::env::var("ANIMA_MOTION_OUT").unwrap());
        std::fs::create_dir_all(&out).unwrap();
        for secs in std::env::var("ANIMA_MOTION_SECS").unwrap().split(',') {
            let secs: f64 = secs.trim().parse().unwrap();
            let t = std::time::Instant::now();
            let r = depth_video_with(&config_dir, DepthVideoArgs {
                depth_path: std::env::var("ANIMA_MOTION_DEPTH").unwrap(),
                image_path: std::env::var("ANIMA_MOTION_IMAGE").unwrap(),
                prompt: "The person from the reference image performs exactly the same motion as the guide video, keeping their face, hairstyle and outfit unchanged. Fixed camera, natural soft lighting, smooth realistic movement.".into(),
                start_secs: 0.0, duration_secs: Some(secs), resolution: Some("480P".into()),
                seed: Some(450_524_784), generate_audio: false, engine: Some("ltx".into()), hd: false,
                keep_background: std::env::var("ANIMA_MOTION_SOURCE").is_ok(),
                source_video_path: std::env::var("ANIMA_MOTION_SOURCE").ok(),
                image_paths: vec![], control: Some("depth".into()),
            }).await;
            match r {
                Ok(v) => {
                    let tag = if std::env::var("ANIMA_MOTION_SOURCE").is_ok() { "_keepbg" } else { "" };
                    let dst = out.join(format!("ltx_{secs}s{tag}.mp4"));
                    std::fs::copy(&v.path, &dst).unwrap();
                    std::fs::copy(&v.first_frame, out.join(format!("ltx_{secs}s{tag}_first.png"))).unwrap();
                    println!("{secs} 秒：{:.1} 分钟 → {} {:?}", t.elapsed().as_secs_f32() / 60.0, dst.display(), v.notes);
                }
                Err(e) => println!("{secs} 秒失败：{e}"),
            }
        }
    }
}
