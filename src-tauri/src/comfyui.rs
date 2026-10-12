//! ComfyUI 创作者工坊与工作流直连调度器
//!
//! 负责与本地 ComfyUI (http://127.0.0.1:8188) 进行 API 交互：
//! 1. 探活检测与硬件状态获取 (/system_stats, /object_info)；
//! 2. 调度执行文生图/视频数字人工作流 (/prompt, /history, /view)；
//! 3. 自动将产出图像/视频与音色配置装配为 Anima 全新出战角色。

use crate::error::{Error, Result};
use crate::AppState;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::State;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComfyStatus {
    pub online: bool,
    pub server_url: String,
    pub device: String,
    pub vram_total_mb: u64,
    pub vram_free_mb: u64,
    pub queue_remaining: u32,
    pub checkpoints: Vec<String>,
    pub loras: Vec<String>,
    pub video_models: Vec<String>,
}

const DEFAULT_COMFY_URL: &str = "http://127.0.0.1:8188";

/// 查找本地 ComfyUI 根目录
pub fn find_comfyui_dir() -> Option<PathBuf> {
    comfyui_candidates().into_iter().find(|c| c.join("main.py").exists())
}

/// 会去找 ComfyUI 的那几个地方。**按 `$HOME` 算，不写死任何人的家目录。**
fn comfyui_candidates() -> Vec<PathBuf> {
    let Ok(home) = std::env::var("HOME") else { return Vec::new() };
    vec![
        Path::new(&home).join("AI").join("ComfyUI"),
        Path::new(&home).join("ComfyUI"),
        Path::new(&home).join("Documents").join("ComfyUI"),
    ]
}

/// 找不到时把**真正找过的地方**列给人看。
pub fn comfyui_search_hint() -> String {
    comfyui_candidates()
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("、")
}

/// 获取 ComfyUI 运行状态
#[tauri::command]
pub async fn comfyui_status(_state: State<'_, AppState>) -> Result<ComfyStatus> {
    comfyui_status_snapshot().await
}

pub async fn comfyui_status_snapshot() -> Result<ComfyStatus> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(1500))
        .build()
        .map_err(|e| Error::runtime(format!("Failed to create HTTP client: {}", e)))?;

    let stats_url = format!("{}/system_stats", DEFAULT_COMFY_URL);
    let default_video_models = vec![
        "LivePortrait (神经面部与体态拓扑引擎 · 60fps 实时锁脸)".to_string(),
        "AnimateDiff v3 (SD 时序动作扩散模块 · mm_v3)".to_string(),
        "SVD-XT (Stable Video Diffusion · 25帧高动态)".to_string(),
    ];

    let resp = match client.get(&stats_url).send().await {
        Ok(r) if r.status().is_success() => r,
        _ => {
            let mut loras = Vec::new();
            if let Some(comfy_dir) = find_comfyui_dir() {
                let lora_dir = comfy_dir.join("models").join("loras");
                if let Ok(entries) = std::fs::read_dir(lora_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_file() {
                            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                                if ext == "safetensors" || ext == "ckpt" || ext == "pt" {
                                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                                        loras.push(name.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
            return Ok(ComfyStatus {
                online: false,
                server_url: DEFAULT_COMFY_URL.to_string(),
                device: "未知 (ComfyUI 未运行)".to_string(),
                vram_total_mb: 0,
                vram_free_mb: 0,
                queue_remaining: 0,
                checkpoints: vec![
                    "majicmixRealistic_v7.safetensors".to_string(),
                    "RealVisXL_V5.0_fp16.safetensors".to_string(),
                    "flux1-dev-fp8.safetensors".to_string(),
                ],
                loras,
                video_models: default_video_models,
            });
        }
    };

    let body: Value = resp.json().await.unwrap_or(Value::Null);
    let mut device_name = "Apple Silicon Metal / MPS".to_string();
    let mut vram_total = 0u64;
    let mut vram_free = 0u64;

    if let Some(devices) = body.get("devices").and_then(|d| d.as_array()) {
        if let Some(dev) = devices.first() {
            if let Some(name) = dev.get("name").and_then(|n| n.as_str()) {
                device_name = name.to_string();
            }
            vram_total = dev.get("vram_total").and_then(|v| v.as_u64()).unwrap_or(0) / 1024 / 1024;
            vram_free = dev.get("vram_free").and_then(|v| v.as_u64()).unwrap_or(0) / 1024 / 1024;
        }
    }

    // 获取 Checkpoint 模型列表
    let mut checkpoints = Vec::new();
    let object_info_url = format!("{}/object_info/CheckpointLoaderSimple", DEFAULT_COMFY_URL);
    if let Ok(info_resp) = client.get(&object_info_url).send().await {
        if let Ok(info_json) = info_resp.json::<Value>().await {
            if let Some(ckpt_list) = info_json
                .pointer("/CheckpointLoaderSimple/input/required/ckpt_name/0")
                .and_then(|v| v.as_array())
            {
                for ckpt in ckpt_list {
                    if let Some(s) = ckpt.as_str() {
                        checkpoints.push(s.to_string());
                    }
                }
            }
        }
    }

    if checkpoints.is_empty() {
        if let Some(comfy_dir) = find_comfyui_dir() {
            let ckpt_dir = comfy_dir.join("models").join("checkpoints");
            if let Ok(entries) = std::fs::read_dir(ckpt_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                            if ext == "safetensors" || ext == "ckpt" || ext == "pt" {
                                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                                    checkpoints.push(name.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // 获取 LoRA 模型列表
    let mut loras = Vec::new();
    let lora_info_url = format!("{}/object_info/LoraLoader", DEFAULT_COMFY_URL);
    if let Ok(info_resp) = client.get(&lora_info_url).send().await {
        if let Ok(info_json) = info_resp.json::<Value>().await {
            if let Some(lora_list) = info_json
                .pointer("/LoraLoader/input/required/lora_name/0")
                .and_then(|v| v.as_array())
            {
                for l in lora_list {
                    if let Some(s) = l.as_str() {
                        loras.push(s.to_string());
                    }
                }
            }
        }
    }

    if loras.is_empty() {
        if let Some(comfy_dir) = find_comfyui_dir() {
            let lora_dir = comfy_dir.join("models").join("loras");
            if let Ok(entries) = std::fs::read_dir(lora_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                            if ext == "safetensors" || ext == "ckpt" || ext == "pt" {
                                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                                    loras.push(name.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(ComfyStatus {
        online: true,
        server_url: DEFAULT_COMFY_URL.to_string(),
        device: device_name,
        vram_total_mb: vram_total,
        vram_free_mb: vram_free,
        queue_remaining: 0,
        checkpoints,
        loras,
        video_models: default_video_models,
    })
}

/// 启动 ComfyUI 必须带上的命令行参数。
///
/// `--fp32-unet` 不是性能调优，是**正确性前提**。
///
/// ComfyUI 在 MPS 上会给扩散模型默认选 bf16（`should_use_bf16(mps)` 为真），
/// 而 FLUX 的前向在 MPS + bf16 下会从 `double_blocks.0` 的 MLP 开始吐 NaN，
/// 一路传到底，latent 全是 NaN，VAE 解出来是**纯黑图**——
/// 采样正常跑完 20 步，日志一行错误都没有。fp16 也不行：FLUX 的激活值
/// 会超过 fp16 上限，出来是没收敛的彩色噪声（像素范围 0~255，
/// 所以「不是全黑」并不能证明它对了，必须真的看图）。只有 fp32 是对的。
///
/// 代价实测为零：SDXL 1024×1024 / 20 步在 fp16 和 fp32 下都是 30.1s ——
/// 统一内存下这段是带宽瓶颈，不是算力瓶颈。所以全局开着就行，
/// 不需要为 FLUX 单起一个实例。
///
/// `--fp32-vae` 同理，防止 VAE 解码阶段的精度问题。
/// 启动 ComfyUI 的参数。
///
/// **`scripts/anima.sh` 的 `COMFY_PRECISION_ARGS` 必须和这里一致** —— 两条
/// 启动路径给的参数不一样的话，从哪边起的决定了链路能不能用，而且不报错。
///
/// - `--fp32-unet` / `--fp32-vae`：FLUX / Wan2.1 / LTX-2.5 在 MPS 上必须 fp32。
///   不带的话 ComfyUI 按可用内存自己挑 bf16，而 bf16 会让 latent 全变 NaN ——
///   出来是**纯黑视频，且任务状态是 success**。
/// - `--use-pytorch-cross-attention`：MiniMax H3 的 SolAttn 加速节点要求它；
///   实测对 Wan / LTX / SDXL 的输出没有影响。
///
/// 细节见 `docs/MEDIA_PIPELINES.md`。
const COMFY_ARGS: &[&str] = &[
    "main.py",
    "--listen",
    "127.0.0.1",
    "--port",
    "8188",
    "--fp32-unet",
    "--fp32-vae",
    "--use-pytorch-cross-attention",
];

/// 启动 ComfyUI 必须带的环境变量。**和 `COMFY_ARGS` 同等重要，同样要两处一致。**
///
/// # 现在是空的 —— 而它曾经有一条，是错的，值得把过程留下来
///
/// 2026-09-03 我往这里加过 `ASFP8_INT8_EXT=1`，理由是日志里出现了
/// `[int8_kernel] INT8 convrot Linear seam installed on MPS`，我据此认为
/// 「强制构建在 M4 Max 上安全且确实装上了」。
///
/// **「缝装上了」不等于「内核算得对」。** 那个包默认把这个内核 gate 在 M5 的
/// 协作 TensorOps 上，M4 过不了探测；`=1` 只是跳过探测强行构建。构建确实成功，
/// 但它自带的**逐位自检**在第一次真正调用时失败：
///
/// ```text
/// 32×128 @ 128×64 的 int8 矩阵乘，和 CPU 参考值比：
///   2048 个元素全错，最大偏差 329125
///   参考 [-15793, 36582, -92708, 60363]
///   内核 [ 65328, 75961, -44890, -57363]
/// [AppleSilicon-FP8/int8_kernel] self-check mismatch; using comfy's int8 path.
/// ```
///
/// 所以在这台机器上强制它**一点好处都没有**，代价是每个新 ComfyUI 进程
/// 白白编译一次 Metal 内核（日志原话「seconds to a few minutes」）。
///
/// 唯一的好消息是自检拦住了它 —— 回落到 comfy 自己的 int8 通路，结果是对的，
/// 只是慢。**没有产生过错误的输出。**
///
/// # 注意别把两层搞混（我就搞混了）
///
/// ComfyUI 自己的混合精度系统报的
/// `Native ops: asym_w4a8_int8, int8_tensorwise, convrot_w4a4` 是**另一回事** ——
/// 那是 comfy_kitchen 的反量化通路，和这里说的 W8A8 Metal 内核不是同一层。
/// int8 权重在这台机器上照样比 nvfp4 待遇好（nvfp4 在那条消息里属于 emulated），
/// 失效的只是额外那层矩阵乘加速。
///
/// **留空但保留这个机制**：两处一致的测试还在，下次真要加环境变量时不用重搭。
pub const COMFY_ENV: &[(&str, &str)] = &[];

/// 日志里出现这句，只说明 W8A8 那层的**接缝**装上了 —— 不代表内核算得对。
///
/// 本机实测：接缝装上了，第一次真正调用时逐位自检全错，随后回落。
/// 所以别拿这行当「int8 加速生效」的判据。
pub const INT8_SEAM_LOG_MARKER: &str = "INT8 convrot Linear seam installed";

/// 给用户看的手工启动命令。**别在各处各写一份** —— 之前 `video_tool.rs` 的
/// 提示里少了精度参数，照着做出来的就是纯黑视频。
pub fn manual_start_hint() -> String {
    hint_with_env(COMFY_ENV)
}

/// `manual_start_hint` 的可测版本。
///
/// 分出来是因为 `COMFY_ENV` 现在是空的 —— 直接测真实提示会得到一个
/// **恒真的空断言**，等于没测。这里用合成的环境变量验「变量必须排在解释器前面」
/// 这条不变量：写在后面就成了传给 `main.py` 的参数，而那**不报错**。
pub(crate) fn hint_with_env(env_vars: &[(&str, &str)]) -> String {
    let env: String = env_vars
        .iter()
        .map(|(k, v)| format!("{k}={v} "))
        .collect();
    // **路径要按这台机器算出来**，不能写死开发者的家目录 ——
    // 换台机器那句提示就是错的，而人会照着它敲
    let dir = find_comfyui_dir()
        .map(|d| d.to_string_lossy().to_string())
        .unwrap_or_else(|| format!("{}/AI/ComfyUI", std::env::var("HOME").unwrap_or_default()));
    format!("在终端运行: cd {dir} && {env}.venv/bin/python {}", COMFY_ARGS.join(" "))
}

/// ComfyUI 队列的当前状态。
///
/// # 为什么要把它显式报出来
///
/// ComfyUI 是**单实例串行执行**的：同时提交多个任务只会排队，不会并行。
/// 而排队期间用户看到的只是「正在生成」，既不知道前面有几个、也不知道要等多久 ——
/// 实测撞到过：用户的请求排在三个任务后面，等到工具超时判失败，
/// 而他完全不知道发生了什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QueueState {
    /// 正在执行的任务数（ComfyUI 实际上恒为 0 或 1）
    pub running: usize,
    /// 排队中的任务数
    pub pending: usize,
    /// 我们这一单前面还有几个。`None` = 队列里找不到它
    /// （还没提交、或已经跑完进了 history）
    pub ahead: Option<usize>,
}

impl QueueState {
    pub fn busy(self) -> bool {
        self.running > 0 || self.pending > 0
    }

    /// 给用户看的一句话。**要说清等多久，不要只说"排队中"。**
    ///
    /// `secs_per_job` 由调用方按链路给（图约 30s，视频按分钟算）——
    /// 估算比不估好，但要让人看得出这是估的。
    pub fn describe(self, secs_per_job: u64) -> Option<String> {
        let ahead = self.ahead?;
        if ahead == 0 {
            return None;
        }
        let est = ahead as u64 * secs_per_job;
        Some(if est >= 120 {
            format!("前面还有 {ahead} 个任务，预计还要等约 {} 分钟", est / 60)
        } else {
            format!("前面还有 {ahead} 个任务，预计还要等约 {est} 秒")
        })
    }
}

/// 从 `/queue` 的响应里解析状态。**纯函数，可测。**
///
/// ComfyUI 的队列项形如 `[优先级, "prompt_id", {图}, ...]`，所以位置就是
/// `queue_pending` 里的下标；正在跑的那个也要算进"前面还有几个"。
pub fn parse_queue(json: &Value, prompt_id: Option<&str>) -> QueueState {
    let arr = |k: &str| {
        json.get(k)
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
    };
    let running = arr("queue_running");
    let pending = arr("queue_pending");

    let id_of = |item: &Value| -> Option<String> {
        item.as_array()?
            .iter()
            .find_map(|v| v.as_str())
            .map(|s| s.to_string())
    };

    let ahead = prompt_id.and_then(|want| {
        if running.iter().any(|i| id_of(i).as_deref() == Some(want)) {
            return Some(0); // 已经在跑了
        }
        let idx = pending
            .iter()
            .position(|i| id_of(i).as_deref() == Some(want))?;
        // 前面排队的 + 正在跑的那个
        Some(idx + running.len())
    });

    QueueState {
        running: running.len(),
        pending: pending.len(),
        ahead,
    }
}

/// 问一次 ComfyUI 的队列。连不上返回默认值（当作空闲）。
pub async fn queue_state(client: &reqwest::Client, prompt_id: Option<&str>) -> QueueState {
    let Ok(resp) = client
        .get(format!("{DEFAULT_COMFY_URL}/queue"))
        .send()
        .await
    else {
        return QueueState::default();
    };
    let Ok(json) = resp.json::<Value>().await else {
        return QueueState::default();
    };
    parse_queue(&json, prompt_id)
}

/// 正在跑的这个 ComfyUI 实例支持哪条视频链路。
///
/// # 为什么要按启动参数选，而不是跑完再看结果
///
/// 两个引擎对精度的要求是**相反**的，而 `--fp32-unet` 是全局标志：
///
/// | 启动方式 | LTX-2.5 | MiniMax H3 |
/// |---|---|---|
/// | 带 `--fp32-unet` | 正常（实测 264s） | 能跑，但吃 fp32、也用不上 SolAttn |
/// | 不带 | **必出纯黑** | 正常（bf16 38GB），SolAttn 可加速 |
///
/// LTX 出黑的机制已经查到底了：不强制精度时 ComfyUI 按可用内存自己挑 dtype，
/// 内存紧张就挑 bf16，而 bf16 在 MPS 上让 LTX 的 latent **全是 NaN**
/// （实测 stage-1 latent 86016/86016 NaN），VAE 把 NaN 解成 0 → 纯黑。
/// 因为 LTX 是音视频**拼在同一条 latent** 里联合采样的，NaN 会顺着注意力
/// 从音频半边污染到视频半边；官方模板解码音频时会直接撞出
/// `[aac] Input contains (near) NaN/+-Inf`，而我们不解码音频，所以**静默变黑**。
///
/// 这件事**只看启动参数就能提前知道**。原来的做法是跑完 LTX（150~250 秒）、
/// 检测输出是不是黑、再回退 MiniMax —— 每次白烧那么久，而且"检测黑屏"
/// 本身是在猜。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoEngineCapability {
    /// 强制了 fp32：LTX 可用
    Fp32Forced,
    /// 没强制：LTX 会出纯黑，只能走 MiniMax
    AutoPrecision,
    /// ComfyUI 没在跑 / 问不到 argv —— 不猜，交给调用方处理
    Unknown,
}

/// 从 ComfyUI 的 argv 判断能力。**纯函数，可测。**
pub fn capability_from_argv(argv: &[String]) -> VideoEngineCapability {
    if argv.is_empty() {
        return VideoEngineCapability::Unknown;
    }
    // `--fp32-unet` 和 `--fp32_unet` 两种写法都认；ComfyUI 的 argparse 两者等价
    let forced = argv
        .iter()
        .any(|a| matches!(a.replace('_', "-").as_str(), "--fp32-unet" | "--fp64-unet"));
    if forced {
        VideoEngineCapability::Fp32Forced
    } else {
        VideoEngineCapability::AutoPrecision
    }
}

impl VideoEngineCapability {
    /// 这条链路下 LTX 能不能用。
    ///
    /// `Unknown` 时返回 false —— **问不到就不要赌**。赌错的代价是用户等
    /// 四分钟拿到一段纯黑，而且没有任何报错。
    pub fn ltx_usable(self) -> bool {
        matches!(self, VideoEngineCapability::Fp32Forced)
    }

    /// 给模型/用户看的一句话解释，说清为什么没走 LTX。
    pub fn why_not_ltx(self) -> Option<&'static str> {
        match self {
            VideoEngineCapability::Fp32Forced => None,
            VideoEngineCapability::AutoPrecision => Some(
                "正在跑的 ComfyUI 没带 `--fp32-unet`，LTX-2.5 在 MPS 上会因 bf16 出 NaN、\
                 生成纯黑视频，所以直接改用 MiniMax H3。想用 LTX 的话，用 `anima start` \
                 重启 ComfyUI（或手工加 `--fp32-unet --fp32-vae`）。",
            ),
            VideoEngineCapability::Unknown => Some(
                "问不到 ComfyUI 的启动参数（没在跑？），无法确认 LTX 的精度是否安全，\
                 改用 MiniMax H3。",
            ),
        }
    }
}

/// LTX-2.5 在这台机器上跑起来需要的**峰值**内存（GB）。
///
/// **量出来的，不是拍的。** 2026-08-29 实测，832×480 之外的同一张图：
///
/// | 配置 | 加载 | 峰值 | 结果 |
/// |---|---|---|---|
/// | bf16 + `--fp32-unet` | 80 GB | **108 GB** | ✅ 264s 出图 |
/// | bf16，无标志 | 40 GB | 98 GB | ❌ latent 全 NaN → 纯黑 |
/// | bf16 + `--use-pytorch-cross-attention` | 40 GB | 98 GB | ❌ 仍纯黑（SDPA 只对 H3 有效）|
/// | int8-convrot + `--fp32-unet` | 20 GB | — | ❌ MPS 缺 `aten::_int_mm` |
///
/// 留 4GB 余量取整到 112 —— 峰值 108 那次已经很贴边了。
pub const LTX_REQUIRED_FREE_GB: f64 = 112.0;

/// H3 的内存门槛：**低于这个数直接拦，因为实测确认会崩**。
///
/// 只有两个实测点（2026-09-02）：
///
/// | 空闲内存 | 结果 |
/// |---|---|
/// | 41 GB | **两次都崩** —— ComfyUI 进程被系统杀掉 |
/// | 108 GB | 正常跑完 |
///
/// **中间没测过。** 理论峰值约 105 GB（UNet 权重 40 GB 按 fp32 加载翻倍，
/// 加文本编码器 15 GB、视频 VAE 10 GB），但 ComfyUI 有分块加载和 offload，
/// 真实门槛很可能低于这个数 —— 所以**不能拿 105 当拦截线**，那会误伤本来
/// 跑得动的情况。这里取 60：明显高于已知会崩的 41，又给中间地带留出余地。
///
/// 崩的样子很特别，值得记住：ComfyUI 进程直接消失，日志停在
/// `Requested to load MiniMaxH3`，**没有任何 Python traceback** —— 是被系统
/// OOM 杀的。在界面上这和"超时"长得一模一样，用户完全查不出原因。
pub const H3_BLOCK_BELOW_GB: f64 = 60.0;

/// 理论峰值，只用在提示文案里，**不作为拦截判据**。
pub const H3_ESTIMATED_PEAK_GB: f64 = 105.0;

/// 内存明显不够时返回 `Some(free_gb)`。`None` = 放行（可能仍然不够，见常量注释）。
pub fn h3_memory_blocker(free_gb: f64) -> Option<f64> {
    (free_gb < H3_BLOCK_BELOW_GB).then_some(free_gb)
}

/// 拦下来时给用户看的话。点名 LM Studio —— 实测那是最大的一块。
pub fn h3_memory_hint(free_gb: f64) -> String {
    format!(
        "内存不够跑 MiniMax H3：当前只有 {free_gb:.0} GB 空闲，而实测 41 GB 时\
         ComfyUI 会被系统直接杀掉、不留任何报错（峰值估计约 {H3_ESTIMATED_PEAK_GB:.0} GB —— \
         UNet 权重 40 GB 按 fp32 加载要翻倍，再加文本编码器 15 GB 和 VAE 10 GB）。\
         先腾内存再试，LM Studio 里卸载模型是最大的一块。"
    )
}

/// 为什么这次不能走 LTX。`None` 表示可以走。
#[derive(Debug, Clone, PartialEq)]
pub enum LtxBlocker {
    /// ComfyUI 没带 `--fp32-unet`
    PrecisionNotForced,
    /// 带了，但内存不够
    NotEnoughMemory { free_gb: f64 },
    /// 问不到 ComfyUI
    ComfyUnavailable,
}

impl LtxBlocker {
    /// 给用户看的一句话：**差什么、怎么补**。
    ///
    /// 这条信息比"生成失败"值钱得多 —— 之前的做法是跑完 150~250 秒、
    /// 拿到一段纯黑、再回退，用户完全不知道差在哪。
    pub fn explain(&self) -> String {
        match self {
            LtxBlocker::PrecisionNotForced => "LTX-2.5 需要 ComfyUI 带 `--fp32-unet` 启动。当前实例没带 —— \
                 不带的话 ComfyUI 会按内存自己挑 bf16，而 bf16 在 MPS 上让 LTX 的 latent \
                 全变成 NaN，出来是一段**纯黑视频，且不报任何错**。\
                 用 `anima start` 重启 ComfyUI 即可（COMFY_ARGS 里已经锁了这个参数）。".to_string(),
            LtxBlocker::NotEnoughMemory { free_gb } => format!(
                "LTX-2.5 实测峰值要 {LTX_REQUIRED_FREE_GB:.0} GB，当前只有 {free_gb:.0} GB 可用。\
                 腾出内存（LM Studio 里卸载模型是最快的一块）后再试，或者改用 MiniMax H3。"
            ),
            LtxBlocker::ComfyUnavailable => {
                "连不上 ComfyUI（/system_stats 没响应），无法确认精度和内存。".to_string()
            }
        }
    }
}

/// 这台机器现在有多少内存可用（GB）。
///
/// 用 `free + inactive + speculative + purgeable` —— macOS 的 inactive、speculative 和 purgeable
/// 都是可回收的文件缓存与空闲页，只看 free 会严重低估，把本来跑得动的任务误判成跑不动。
pub fn free_memory_gb() -> f64 {
    let Ok(out) = std::process::Command::new("vm_stat").output() else {
        return 0.0;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut page = 16384f64;
    let mut pages = 0f64;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("Mach Virtual Memory Statistics: (page size of ") {
            if let Some(n) = rest.split(' ').next().and_then(|s| s.parse::<f64>().ok()) {
                page = n;
            }
        }
        if line.starts_with("Pages free:")
            || line.starts_with("Pages inactive:")
            || line.starts_with("Pages speculative:")
            || line.starts_with("Pages purgeable:")
        {
            if let Some(n) = line.rsplit(' ').next().and_then(|s| s.trim_end_matches('.').parse::<f64>().ok()) {
                pages += n;
            }
        }
    }
    pages * page / (1024.0 * 1024.0 * 1024.0)
}

/// 判定 LTX 这次能不能走。纯函数部分抽出来可测。
pub fn ltx_blocker(argv: Option<&[String]>, free_gb: f64) -> Option<LtxBlocker> {
    let Some(argv) = argv else {
        return Some(LtxBlocker::ComfyUnavailable);
    };
    match capability_from_argv(argv) {
        VideoEngineCapability::Unknown => Some(LtxBlocker::ComfyUnavailable),
        VideoEngineCapability::AutoPrecision => Some(LtxBlocker::PrecisionNotForced),
        VideoEngineCapability::Fp32Forced => {
            if free_gb < LTX_REQUIRED_FREE_GB {
                Some(LtxBlocker::NotEnoughMemory { free_gb })
            } else {
                None
            }
        }
    }
}

/// 内存够但参数不对时，能不能**自动重启** ComfyUI 换成正确参数。
///
/// 三个条件缺一不可，因为重启是**破坏性**的 —— 它会杀掉正在跑的任务，
/// 包括用户自己在 ComfyUI 界面里排的队：
///
/// 1. 内存够（否则换了参数也跑不动，白重启一次）
/// 2. 队列是空的（不能把别人的活干掉）
/// 3. 这个实例是 Anima 自己起的（用户手工起的实例有他自己的意图，不要动）
pub fn may_restart_comfy_for_ltx(free_gb: f64, queue_busy: bool, started_by_anima: bool) -> bool {
    free_gb >= LTX_REQUIRED_FREE_GB && !queue_busy && started_by_anima
}

/// 读跑着的 ComfyUI 的命令行（`/system_stats` 的 `argv`）。
///
/// 用户经常是自己先开好 ComfyUI 的（桌面版、或手敲命令），那时
/// [`COMFY_ARGS`] 根本没生效。所以精度对不对必须**问正在跑的那个实例**，
/// 不能看我们打算怎么启动。连不上就返回 `None` —— 没开图像功能是正常状态。
pub async fn running_argv() -> Option<Vec<String>> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .ok()?;
    let stats: serde_json::Value = client
        .get(format!("{DEFAULT_COMFY_URL}/system_stats"))
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    let argv = stats.get("system")?.get("argv")?.as_array()?;
    Some(
        argv.iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
    )
}

/// 尝试在后台启动本地 ComfyUI
#[tauri::command]
pub async fn comfyui_start(state: State<'_, AppState>) -> Result<bool> {
    comfyui_start_with_config_dir(&state.config_dir).await
}

pub async fn comfyui_start_with_config_dir(config_dir: &Path) -> Result<bool> {
    let comfy_dir = find_comfyui_dir().ok_or_else(|| {
        // 把**真正找过的那几个地方**列出来 —— 写死一个路径的话，
        // 换台机器这句提示是错的，而人会照着去那儿找
        Error::runtime(format!("这几个地方都没有 ComfyUI 的 main.py：{}", comfyui_search_hint()))
    })?;

    let venv_python = comfy_dir.join(".venv").join("bin").join("python");
    let python_bin = if venv_python.exists() {
        venv_python
    } else {
        PathBuf::from("python3")
    };

    let log_path = config_dir.join("comfyui.log");
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| Error::Io(format!("创建 ComfyUI 日志失败: {}", e)))?;
    let log_err = log_file
        .try_clone()
        .map_err(|e| Error::Io(format!("复制日志句柄失败: {}", e)))?;

    tracing::info!(dir = ?comfy_dir, python = ?python_bin, log = ?log_path, "正在后台启动 ComfyUI...");
    let _ = std::process::Command::new(python_bin)
        .args(COMFY_ARGS)
        .stdout(std::process::Stdio::from(log_file))
        .stderr(std::process::Stdio::from(log_err))
        .stdin(std::process::Stdio::null())
        .current_dir(&comfy_dir)
        .spawn()
        .map_err(|e| Error::runtime(format!("启动 ComfyUI 进程失败: {}", e)))?;

    Ok(true)
}

#[cfg(test)]
mod tests {

    fn queue_json(running: &[&str], pending: &[&str]) -> serde_json::Value {
        // ComfyUI 的队列项形如 [优先级, "prompt_id", {图}, ...]
        let mk = |ids: &[&str]| -> Vec<serde_json::Value> {
            ids.iter()
                .enumerate()
                .map(|(i, id)| serde_json::json!([i, id, {}]))
                .collect()
        };
        serde_json::json!({ "queue_running": mk(running), "queue_pending": mk(pending) })
    }

    /// 排在第几 = 前面排队的 + 正在跑的那个。
    #[test]
    fn 算得出前面还有几个() {
        let j = queue_json(&["a"], &["b", "c", "mine"]);
        let q = parse_queue(&j, Some("mine"));
        assert_eq!(q.running, 1);
        assert_eq!(q.pending, 3);
        // b、c 在前，外加正在跑的 a
        assert_eq!(q.ahead, Some(3));
    }

    #[test]
    fn 已经在跑时前面是零() {
        let q = parse_queue(&queue_json(&["mine"], &["x"]), Some("mine"));
        assert_eq!(q.ahead, Some(0));
        assert!(q.describe(30).is_none(), "已经在跑就不该再说排队");
    }

    /// 队列里找不到 = 还没提交，或已经跑完进了 history。**不要瞎猜位置。**
    #[test]
    fn 队列里没有时不编位置() {
        let q = parse_queue(&queue_json(&["a"], &["b"]), Some("mine"));
        assert_eq!(q.ahead, None);
        assert!(q.describe(30).is_none());
        // 但队列忙这件事仍然成立
        assert!(q.busy());
    }

    #[test]
    fn 空队列不忙() {
        let q = parse_queue(&queue_json(&[], &[]), None);
        assert!(!q.busy());
        assert_eq!(q.ahead, None);
    }

    /// **要说清等多久，不能只说"排队中"。**
    ///
    /// 排队和卡住在界面上长得一样；说不出预计时间的提示等于没提示。
    #[test]
    fn 提示里必须带数量和预计时间() {
        let q = parse_queue(&queue_json(&["a"], &["b", "c", "mine"]), Some("mine"));
        let msg = q.describe(30).expect("排在后面就该给提示");
        assert!(msg.contains('3'), "要说前面有几个：{msg}");
        assert!(msg.contains("分钟") || msg.contains('秒'), "要给预计时间：{msg}");
        // 3 个 × 30s = 90s，走秒的分支
        assert!(msg.contains("90"), "{msg}");
        // 超过两分钟改用分钟表述，别让人去数秒
        let long = QueueState { running: 1, pending: 9, ahead: Some(9) };
        assert!(long.describe(30).unwrap().contains("分钟"));
    }

    /// 格式错乱的响应不能 panic，当作空闲。
    #[test]
    fn 响应格式不对时当空闲() {
        for bad in [serde_json::json!({}), serde_json::json!({"queue_pending": "nope"}), serde_json::json!([])] {
            let q = parse_queue(&bad, Some("mine"));
            assert!(!q.busy());
            assert_eq!(q.ahead, None);
        }
    }


    fn argv_with_fp32() -> Vec<String> {
        ["main.py", "--listen", "127.0.0.1", "--port", "8188", "--fp32-unet", "--fp32-vae"]
            .iter().map(|s| s.to_string()).collect()
    }
    fn argv_plain() -> Vec<String> {
        ["main.py", "--listen", "127.0.0.1", "--port", "8188"]
            .iter().map(|s| s.to_string()).collect()
    }

    /// 参数不对时**不要跑** —— 跑出来是纯黑，且不报错。
    #[test]
    fn 参数不对直接拦下并说清怎么补() {
        let b = ltx_blocker(Some(&argv_plain()), 120.0).expect("该被拦");
        assert_eq!(b, LtxBlocker::PrecisionNotForced);
        let msg = b.explain();
        assert!(msg.contains("--fp32-unet"), "{msg}");
        assert!(msg.contains("anima start"), "要给出怎么补：{msg}");
        assert!(msg.contains("纯黑"), "要说清不补会怎样：{msg}");
    }

    /// 参数对但内存不够 —— 也别跑，实测峰值 108GB。
    #[test]
    fn 内存不够时拦下并报出差多少() {
        let b = ltx_blocker(Some(&argv_with_fp32()), 40.0).expect("该被拦");
        assert_eq!(b, LtxBlocker::NotEnoughMemory { free_gb: 40.0 });
        let msg = b.explain();
        assert!(msg.contains("40"), "要报出当前有多少：{msg}");
        assert!(msg.contains("112"), "要报出需要多少：{msg}");
        assert!(msg.contains("MiniMax"), "要给出退路：{msg}");
    }

    #[test]
    fn 参数对且内存够才放行() {
        assert!(ltx_blocker(Some(&argv_with_fp32()), 120.0).is_none());
        // 刚好卡在阈值下方要拦
        assert!(ltx_blocker(Some(&argv_with_fp32()), LTX_REQUIRED_FREE_GB - 0.1).is_some());
    }

    #[test]
    fn 连不上comfyui时不赌() {
        assert_eq!(ltx_blocker(None, 120.0), Some(LtxBlocker::ComfyUnavailable));
    }

    /// **自动重启是破坏性动作，三个条件缺一不可。**
    #[test]
    fn 自动重启的三个条件缺一不可() {
        assert!(may_restart_comfy_for_ltx(120.0, false, true), "三条都满足才该重启");
        assert!(!may_restart_comfy_for_ltx(40.0, false, true), "内存不够，换了参数也跑不动");
        assert!(!may_restart_comfy_for_ltx(120.0, true, true), "队列有活，不能干掉别人的任务");
        assert!(
            !may_restart_comfy_for_ltx(120.0, false, false),
            "用户手工起的实例有他自己的意图，不要动"
        );
    }

    /// 阈值必须来自实测，不能是随手写的整数。
    #[test]
    fn 内存阈值覆盖实测峰值() {
        // 实测 bf16 + --fp32-unet 峰值 108GB
        assert!(LTX_REQUIRED_FREE_GB >= 108.0, "阈值低于实测峰值就会放进跑不动的任务");
        assert!(LTX_REQUIRED_FREE_GB <= 128.0, "不能超过整机内存，否则永远进不去");
    }

    #[test]
    fn 可用内存读得出正数() {
        let gb = free_memory_gb();
        assert!(gb > 0.0 && gb < 1024.0, "vm_stat 解析异常：{gb}");
    }


    /// **两条启动路径必须给同一组参数。**
    ///
    /// `anima start` 走 scripts/anima.sh，应用内启动走 COMFY_ARGS。
    /// 两边漂了的话，从哪边起的决定了 LTX 能不能用 —— 而且不报错，
    /// 表现是"有时候好有时候黑"。今天就是这么丢了大半天。
    /// **环境变量也会漂，而且漂了不报错。**
    ///
    /// `--fp32-unet` 漂了会出纯黑图（响得很大声），`ASFP8_INT8_EXT` 漂了只是慢 ——
    /// 后者更危险，因为没有任何症状。2026-09-03 发现 `anima.sh` 从来没设过它：
    /// 从 `anima start` 起来的 ComfyUI 一直只有 int8 的「能跑」层，没有「跑得快」层。
    #[test]
    fn 启动脚本和_comfy_env_保持一致() {
        let sh = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join("scripts/anima.sh"),
        )
        .expect("读不到 anima.sh");

        // 同样只看那一行真正的数组，不全文搜 —— 上面那个测试就是这么被注释骗过的
        let line = sh
            .lines()
            .find(|l| l.trim_start().starts_with("COMFY_ENV_VARS=("))
            .expect("anima.sh 里找不到 COMFY_ENV_VARS=(...)");

        for (k, v) in COMFY_ENV {
            assert!(
                line.contains(&format!("{k}={v}")),
                "anima.sh 的 COMFY_ENV_VARS 里缺 `{k}={v}` —— 从 anima start 起的 \
                 ComfyUI 会少一层 int8 加速，而且**只是慢，不报错**。实际那行是：{line}"
            );
        }

        // 光有数组没用，启动那句得真的把它注入进去
        assert!(
            sh.contains("nohup env \"${COMFY_ENV_VARS[@]}\""),
            "anima.sh 定义了 COMFY_ENV_VARS 却没在启动 ComfyUI 时注入 —— \
             定义了不用比没定义更糟，因为看上去像做了"
        );
    }

    /// 手工启动提示必须把环境变量带上，**且排在解释器前面**。
    ///
    /// 用合成变量测，不用 `COMFY_ENV` —— 它现在是空的，拿它测会得到一个恒真断言。
    /// 而这条不变量本身没过期：写在 `python` 后面就成了传给 `main.py` 的参数，
    /// 而 ComfyUI 会忽略不认识的参数，**不报错**。
    #[test]
    fn 手工启动提示把环境变量放在解释器前面() {
        let h = hint_with_env(&[("FOO_BAR", "1"), ("BAZ", "2")]);
        for (k, v) in [("FOO_BAR", "1"), ("BAZ", "2")] {
            assert!(h.contains(&format!("{k}={v}")), "提示里缺 {k}={v}：{h}");
        }
        let env_pos = h.find("FOO_BAR").expect("没有环境变量");
        let py_pos = h.find(".venv/bin/python").expect("没有解释器");
        assert!(env_pos < py_pos, "环境变量必须在解释器前面：{h}");

        // COMFY_ENV 里真有东西时，真实提示也得带上
        let real = manual_start_hint();
        for (k, v) in COMFY_ENV {
            assert!(real.contains(&format!("{k}={v}")), "真实提示里缺 {k}={v}：{real}");
        }
    }

    #[test]
    fn 启动脚本和_comfy_args_保持一致() {
        let sh = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join("scripts/anima.sh"),
        )
        .expect("读不到 anima.sh");

        // **只看那一行真正的参数数组，不要全文搜。**
        //
        // 第一版写的是 `sh.contains(flag)` —— 结果匹配到了我自己写在上面的
        // 注释里的 `--fp32-unet`，把参数删掉测试照样绿。变异测试抓到的。
        let args_line = sh
            .lines()
            .find(|l| l.trim_start().starts_with("COMFY_PRECISION_ARGS=("))
            .expect("anima.sh 里找不到 COMFY_PRECISION_ARGS=(...)");

        for flag in COMFY_ARGS
            .iter()
            .filter(|a| a.starts_with("--") && **a != "--listen" && **a != "--port")
        {
            assert!(
                args_line.contains(flag),
                "anima.sh 的 COMFY_PRECISION_ARGS 里缺 `{flag}` —— 两条启动路径漂了，\
                 从 anima start 起的 ComfyUI 会和应用内起的行为不同，而且不报错。\
                 实际那行是：{args_line}"
            );
        }
    }

    /// 给用户看的手工启动命令必须**包含所有精度参数**。
    ///
    /// 之前 video_tool 的提示里少了 --fp32-unet，用户照着做起出来的实例
    /// 每次生成 LTX 都是纯黑。提示语和真实启动参数必须同源。
    #[test]
    fn 手工启动提示不能漏掉精度参数() {
        let hint = manual_start_hint();
        for flag in ["--fp32-unet", "--fp32-vae", "--use-pytorch-cross-attention"] {
            assert!(hint.contains(flag), "提示里缺 `{flag}`：{hint}");
        }
    }

    /// 参数里带了 fp32 才认为 LTX 可用。
    #[test]
    fn 带fp32时ltx可用() {
        let argv: Vec<String> = ["main.py", "--listen", "127.0.0.1", "--port", "8188", "--fp32-unet", "--fp32-vae"]
            .iter().map(|s| s.to_string()).collect();
        assert_eq!(capability_from_argv(&argv), VideoEngineCapability::Fp32Forced);
        assert!(capability_from_argv(&argv).ltx_usable());
        assert!(capability_from_argv(&argv).why_not_ltx().is_none());
    }

    /// **这条是重点。**
    ///
    /// 用户经常自己 `python main.py --listen 127.0.0.1 --port 8188` 起 ComfyUI，
    /// 那时 COMFY_ARGS 完全没生效。实测这种情况下 LTX 的 latent 全是 NaN、
    /// 出纯黑，而且不报任何错 —— 提前判掉，别跑完 250 秒再猜。
    #[test]
    fn 没带fp32时ltx不可用且说得出原因() {
        let argv: Vec<String> = ["main.py", "--listen", "127.0.0.1", "--port", "8188"]
            .iter().map(|s| s.to_string()).collect();
        let cap = capability_from_argv(&argv);
        assert_eq!(cap, VideoEngineCapability::AutoPrecision);
        assert!(!cap.ltx_usable());
        let why = cap.why_not_ltx().expect("必须说清为什么不走 LTX");
        assert!(why.contains("--fp32-unet"), "{why}");
        assert!(why.contains("MiniMax"), "要告诉用户改用了什么：{why}");
    }

    /// argparse 里 `--fp32_unet` 和 `--fp32-unet` 等价，两种写法都要认。
    #[test]
    fn 下划线写法也认() {
        let argv = vec!["main.py".to_string(), "--fp32_unet".to_string()];
        assert!(capability_from_argv(&argv).ltx_usable());
    }

    /// **问不到就不要赌。**
    ///
    /// 赌错的代价是用户等四分钟拿到一段纯黑，且没有任何报错。
    #[test]
    fn 问不到参数时不赌ltx() {
        let cap = capability_from_argv(&[]);
        assert_eq!(cap, VideoEngineCapability::Unknown);
        assert!(!cap.ltx_usable());
        assert!(cap.why_not_ltx().is_some());
    }

    /// 别的精度标志不能被误当成 fp32 —— fp16 对 LTX 一样是坏的。
    #[test]
    fn fp16标志不算数() {
        let argv = vec!["main.py".to_string(), "--fp16-unet".to_string()];
        assert!(!capability_from_argv(&argv).ltx_usable());
        let argv = vec!["main.py".to_string(), "--fp32-vae".to_string()];
        assert!(!capability_from_argv(&argv).ltx_usable(), "只有 vae 是 fp32 救不了 unet");
    }

    use super::*;

    /// 这条测试守的不是「参数拼对了」，而是**别人把它删掉时会有人喊一声**。
    ///
    /// 少了 `--fp32-unet`，FLUX 会安静地出纯黑图：采样跑完全部步数、
    /// 日志零报错。这种失败没有任何自然的发现途径，只能靠这里钉住。
    #[test]
    fn 启动参数必须锁死_fp32_精度() {
        assert!(
            COMFY_ARGS.contains(&"--fp32-unet"),
            "去掉它 FLUX 会静默出黑图"
        );
        assert!(COMFY_ARGS.contains(&"--fp32-vae"));
        // 精度开关是互斥的，混进来一个会覆盖掉 fp32
        for bad in ["--bf16-unet", "--fp16-unet", "--force-fp16"] {
            assert!(!COMFY_ARGS.contains(&bad), "{bad} 会盖掉 --fp32-unet");
        }
    }

    #[test]
    fn 启动参数指向诊断项检查的那个端口() {
        assert!(COMFY_ARGS.contains(&"8188"));
        assert!(DEFAULT_COMFY_URL.ends_with("8188"));
    }
}

#[cfg(test)]
mod queue_snapshot_probe {
    use super::*;

    /// 用**真实的** `/queue` 响应验证解析。取证用，默认 ignore。
    ///
    /// ```bash
    /// curl -s http://127.0.0.1:8188/queue > /tmp/qsnap.json
    /// ANIMA_QUEUE_SNAPSHOT=/tmp/qsnap.json ANIMA_QUEUE_ID=<prompt_id> \
    ///   cargo test --lib queue_snapshot_probe -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore]
    fn parse_real_queue() {
        let path = std::env::var("ANIMA_QUEUE_SNAPSHOT").expect("需要 ANIMA_QUEUE_SNAPSHOT");
        let want = std::env::var("ANIMA_QUEUE_ID").ok();
        let json: Value =
            serde_json::from_str(&std::fs::read_to_string(path).expect("读不到快照")).unwrap();
        let q = parse_queue(&json, want.as_deref());
        println!("running={} pending={} ahead={:?}", q.running, q.pending, q.ahead);
        println!("busy={}", q.busy());
        match q.describe(30) {
            Some(m) => println!("提示：{m}"),
            None => println!("提示：（不排队）"),
        }
    }
}
