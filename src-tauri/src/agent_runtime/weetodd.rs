//! WeeTodd / MLX 这条 LTX 2.5 通路。
//!
//! 和 `video_tool` 里那条原生 33 节点的图是**两条并行的路**，不是替换关系：
//!
//! | | 原生（`ComfyUI-LTXVideo` + KJNodes） | WeeTodd（MLX） |
//! |---|---|---|
//! | 节点数 | 33 | **4** |
//! | 权重 | `...-bf16.safetensors` 单文件 | `weetodd-q8/...-q8-paged/` 目录 |
//! | 5 秒 768×512 | 116s / **96.82 GB** | 121s / **22.84 GB** |
//! | 15 秒接续 | 做不到 | 一个节点换名字的事 |
//! | 负向提示词 | 有（KJNodes 的 NAG） | **没有入口** |
//!
//! 最后一行是选它要付的代价，调用方必须知道：`WeeToddLTX25Generate` 根本没有
//! negative 输入（蒸馏模型 cfg=1，负向本来就是结构性无效的），所以
//! 「镜头别乱动 / 别切镜」这类约束只能写进正向提示词。
//!
//! 实测数据来自 2026-09-03，全部是打真实 ComfyUI、看过画面、Whisper 验过中文台词的。

use crate::agent_runtime::video_tool::DEFAULT_COMFY_URL;
use serde_json::{json, Map, Value};

/// q8 分页权重放在 `models/weetodd-q8/` 下，**不放 `diffusion_models/`**。
///
/// 分页是「一个目录 + 49 个 `.safetensors`」，而 ComfyUI 的模型扫描是递归的：
/// 放在 `diffusion_models/` 下会往原生 `UNETLoader` 的下拉框里塞 49 条噪声，
/// 而且那些分页**是可选的** —— 选中 `layer-017` 等于把第 17 层当整个模型加载。
pub const TRANSFORMER_Q8: &str = "weetodd-q8/ltx-2.5-22b-distilled-transformer-q8-paged";
pub const TEXT_ENCODER_Q8: &str = "weetodd-q8/gemma4-12b-with-proj-ltx-2.5-q8-paged";
/// 视频 VAE 用 **conv** 那份，不是 `ltx-2.5-video-vae-bf16`。
pub const VIDEO_VAE: &str = "ltx-2.5-video-vae-conv-bf16.safetensors";
pub const AUDIO_VAE: &str = "ltx-2.5-audio-vae-bf16.safetensors";
pub const SPATIAL_UPSCALER: &str = "ltx-2.5-latent-spatial-upscaler-x2-bf16-1.0.safetensors";

/// 判断 WeeTodd 装没装，看这个节点在不在 `/object_info` 里。
pub const PROBE_NODE: &str = "WeeToddLTX25Generate";
/// 长片接续用的节点。
pub const CHAINED_NODE: &str = "WeeToddLTX25GenerateChained";

/// 单段最长多少秒。超过就得走接续（多窗口），这是模型的上下文长度决定的。
pub const MAX_SINGLE_WINDOW_SECS: f64 = 5.0;
/// 接续时相邻窗口重叠多少帧。25 是官方示例的值，也是我们实测跑通的值。
pub const OVERLAP_FRAMES: u32 = 25;

/// 一次 WeeTodd LTX 2.5 生成。
pub struct WeeToddParams<'a> {
    /// 首帧图（ComfyUI `input/` 下的相对文件名）。`None` = 纯文生。
    ///
    /// LTX 这边简单 —— `Generate` 直接有个 `first_frame: IMAGE` 输入，
    /// 接个 `LoadImage` 就行。**H3 那边不是这样**：它要按分区切
    /// （`fl2va`）并走 `FirstFrame → KeyframeEncode`，而且需要带视觉权重的
    /// 编码器（我们那份 q8-paged 是纯文本的，见 `H3Mode` 的注释）。
    pub first_frame: Option<&'a str>,
    /// 末帧图（同样是 `input/` 下的相对文件名）。走 `WeeToddLTX25Keyframe`
    /// 把它钉在最后一帧上 —— **不是** `first_frame` 的对称参数，那个入口不存在。
    pub last_frame: Option<&'a str>,
    pub prompt: &'a str,
    pub width: u32,
    pub height: u32,
    pub duration_secs: f64,
    pub fps: f64,
    pub seed: u64,
    pub filename_prefix: &'a str,
    /// 挂哪些 LoRA（已解析成 ComfyUI 枚举出的真实名字）。
    pub loras: &'a [(String, f64)],
    /// 分成几段接续。1 = 单段。
    pub window_count: u32,
    /// 每段各自的提示词；为空时所有段共用 `prompt`。
    pub window_prompts: &'a [String],
}

fn node(class_type: &str, inputs: Value) -> Value {
    json!({ "class_type": class_type, "inputs": inputs })
}

/// **`preset` 必须是 `Custom`。**
///
/// 选了别的 preset，逐项控件会被它覆盖 —— 我们设的 `low_ram_streaming=true`
/// 会被压回 false，而 q8 分页权重在那种情况下直接报
/// 「Paged LTX 2.5 transformer checkpoints require low_ram_streaming=true」。
/// 2026-09-03 第一次提交就是这么失败的。
const PRESET_CUSTOM: &str = "Custom";

pub fn build_graph(p: &WeeToddParams) -> Value {
    let mut m = Map::new();
    m.insert(
        "1".into(),
        node(
            "WeeToddLTX25ComponentLoader",
            json!({
                "transformer": TRANSFORMER_Q8,
                "text_encoder": TEXT_ENCODER_Q8,
                "video_vae": VIDEO_VAE,
                "audio_vae": AUDIO_VAE,
                "spatial_upscaler": SPATIAL_UPSCALER,
                "duration_head": ""
            }),
        ),
    );
    m.insert(
        "2".into(),
        node(
            "WeeToddLTX25GenerationConfig",
            json!({
                "preset": PRESET_CUSTOM,
                "width": p.width,
                "height": p.height,
                "duration_seconds": p.duration_secs,
                "frame_rate": p.fps,
                "seed": p.seed,
                "low_memory": true,
                // q8 分页权重强制要求这个为 true，见 PRESET_CUSTOM 的注释
                "low_ram_streaming": true,
                "prompt_context": "official_1024",
                "feed_forward_backend": "reference_fp32"
            }),
        ),
    );
    m.insert(
        "3".into(),
        node(
            "WeeToddLTX25Preflight",
            json!({"model": ["1", 0], "config": ["2", 0]}),
        ),
    );

    // LoRA 链接在 Preflight 之后、Generate 之前。节点号从 900 起，和别处约定一致。
    let mut model_ref = json!(["3", 0]);
    for (i, (name, strength)) in p.loras.iter().enumerate() {
        let id = (900 + i).to_string();
        m.insert(
            id.clone(),
            node(
                "WeeToddLTX25LoRALoader",
                json!({"model": model_ref, "lora": name, "strength": strength}),
            ),
        );
        model_ref = json!([id, 0]);
    }

    let mut gen = Map::new();
    gen.insert("model".into(), model_ref);

    // 首帧和末帧**不能一个走 `first_frame` 一个走关键帧** —— 上游
    // `ltx_pipelines_mlx/distilled.py` 里是这么写的：
    //
    //     if image is not None and not resolved_images:
    //         resolved_images = [ImageConditioningInput(path=image, frame_idx=0, ...)]
    //
    // 也就是说**只要有任何关键帧，`first_frame` 就被整个丢掉**，不报错。
    // 实测过：给了首帧+末帧，出来的视频末帧对得上、首帧完全是另一个画面。
    // 节点自己那句 "Use either first_frame or a frame-zero keyframe, not both"
    // 拦的是反过来的情况，帮不上忙。
    //
    // 所以：**只有首帧**走 `first_frame`（简单、已验证）；**一旦有末帧**，
    // 两头都改走关键帧链，首帧钉在 0 帧。
    match (p.first_frame, p.last_frame) {
        (Some(file), None) => {
            m.insert("10".into(), node("LoadImage", json!({"image": file, "upload": "image"})));
            gen.insert("first_frame".into(), json!(["10", 0]));
        }
        (first, Some(last_file)) => {
            // frame_index 取总帧数减一。给大了节点会拒；给小了末帧提前出现、
            // 后面继续动 —— 后者不报错，就是"末帧没生效"的样子。
            let last_idx = ((p.duration_secs * p.fps).round() as i64 - 1).max(0);
            let mut chain: Option<Value> = None;
            if let Some(file) = first {
                m.insert("10".into(), node("LoadImage", json!({"image": file, "upload": "image"})));
                m.insert(
                    "13".into(),
                    node(
                        "WeeToddLTX25Keyframe",
                        json!({"image": ["10", 0], "frame_index": 0, "strength": 1.0}),
                    ),
                );
                chain = Some(json!(["13", 0]));
            }
            m.insert("11".into(), node("LoadImage", json!({"image": last_file, "upload": "image"})));
            let mut kf = json!({"image": ["11", 0], "frame_index": last_idx, "strength": 1.0});
            if let Some(prev) = chain {
                kf["previous_keyframes"] = prev;
            }
            m.insert("12".into(), node("WeeToddLTX25Keyframe", kf));
            gen.insert("keyframes".into(), json!(["12", 0]));
        }
        (None, None) => {}
    }
    gen.insert("config".into(), json!(["2", 0]));
    gen.insert("filename_prefix".into(), json!(p.filename_prefix));
    gen.insert("unload_after_generate".into(), json!(true));

    if p.window_count > 1 {
        gen.insert("window_count".into(), json!(p.window_count));
        gen.insert(
            "overlap_frames".into(),
            json!(std::env::var("ANIMA_OVERLAP")
                .ok()
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(OVERLAP_FRAMES)),
        );
        // 段提示词逐条填。给不满时后面的段沿用总提示词 ——
        // 空字符串会让那一段没有条件输入，不是「继续上一段」。
        for i in 0..4 {
            let key = format!("prompt_{}", i + 1);
            let v = if (i as u32) < p.window_count {
                p.window_prompts.get(i).map(String::as_str).unwrap_or(p.prompt)
            } else {
                ""
            };
            gen.insert(key, json!(v));
        }
        m.insert("4".into(), node(CHAINED_NODE, Value::Object(gen)));
    } else {
        gen.insert("prompt".into(), json!(p.prompt));
        m.insert("4".into(), node(PROBE_NODE, Value::Object(gen)));
    }
    Value::Object(m)
}

/// 要几个窗口才装得下这么长。
///
/// 单段上限是 5 秒；再长就得接续，而**接续不是免费的** ——
/// 实测跨窗口时人物身份稳得住，但走位和布景会飘。
/// 长片每一段的提示词。
///
/// **每段必须完整重述该保持的一切**（人物、服装、机位、构图、背景、道具），
/// 只有动作不同 —— `base` 就是那个不变部分，`actions` 是各段的动作。
///
/// 2026-09-04 同题三跑实测（15 秒 / 三窗口 / 768×512，取各段中点比构图）：
///
/// | 写法 | w1~w2 | w1~w3 | w2~w3 | 平均 |
/// |---|---|---|---|---|
/// | 三段同一句（原来的做法） | 0.906 | 0.815 | 0.819 | 0.846 |
/// | 分段但只写动作 | 0.819 | **0.578** | 0.666 | 0.688 |
/// | **每段完整重述** | 0.841 | **0.872** | 0.918 | **0.877** |
///
/// 关键不是平均值，是**随窗口距离的变化**：原做法相似度随距离衰减
/// （0.906 → 0.815），这就是漂移累积的形状；每段完整重述后不再衰减。
///
/// **只写动作反而最差** —— 第二段没提「年轻人」，模型就把他从画面里删了。
/// 分段提示词不是增量指令，是那一段的完整描述。
pub fn window_prompts(base: &str, actions: &[String], windows: u32) -> Vec<String> {
    let n = windows.max(1) as usize;
    if actions.is_empty() {
        return vec![base.to_string(); n];
    }
    (0..n)
        .map(|i| {
            // 段数给少了就沿用最后一个动作（「继续这么做」），
            // 而不是留空 —— 空提示词那一段等于没有条件输入。
            let act = actions.get(i).or_else(|| actions.last()).map(String::as_str).unwrap_or("");
            match (base.trim().is_empty(), act.trim().is_empty()) {
                (true, _) => act.trim().to_string(),
                (_, true) => base.trim().to_string(),
                _ => format!("{} {}", base.trim(), act.trim()),
            }
        })
        .collect()
}

pub fn window_count_for(duration_secs: f64) -> u32 {
    if duration_secs <= MAX_SINGLE_WINDOW_SECS {
        1
    } else {
        (duration_secs / MAX_SINGLE_WINDOW_SECS).ceil() as u32
    }
}

/// 这条路没有负向提示词入口 —— 调用方要把这句原样告诉用户。
pub const NO_NEGATIVE_NOTE: &str =
    "WeeTodd/MLX 这条路没有负向提示词入口（蒸馏模型 cfg=1，负向结构性无效）。\
     「固定机位 / 不切镜 / 别推近」这类约束请写进正向提示词。";

/// 某个节点在跑着的 ComfyUI 里注册了没有。
///
/// **查 `/object_info/<节点>`，不查文件系统。** 节点包可以装在任意位置、
/// 也可能因为依赖缺失而加载失败 —— 目录在不代表节点能用，而 `/object_info`
/// 报的就是它真正认得的那一份。
pub async fn node_available(client: &reqwest::Client, node: &str) -> bool {
    let url = format!("{DEFAULT_COMFY_URL}/object_info/{node}");
    match client
        .get(&url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => match r.json::<Value>().await {
            // ComfyUI 对未知节点返回 200 + 空对象，所以不能只看状态码
            Ok(v) => probe_installed(&v, node),
            Err(_) => false,
        },
        _ => false,
    }
}

/// 这次请求该走哪条路。
///
/// **两条 MLX 路是首选，原生那条只在 WeeTodd 没装时兜底。** 依据是本机同题实测
/// （见 `docs/APPLE_SILICON_VIDEO_STACK.md`）：LTX 5 秒片内存峰值
/// 96.82 GB → 22.84 GB，H3 49.06 GB → 15.87 GB。
///
/// H3 的判据是「用户点名要音画联合」—— 它是唯一能一次出画面+对白+环境声的。
/// 其余一律 LTX：更快（121s vs 384s）、控制力更强（pose/depth/canny/多主体参考）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// WeeTodd/MLX 的 LTX 2.5。
    Ltx,
    /// WeeTodd/MLX 的 MiniMax H3（音画联合）。
    H3,
    /// 原生 ComfyUI 节点。WeeTodd 没装时才走这条。
    Native,
}

/// 选路。`engine_hint` 是用户/模型点名的引擎，空表示没点名。
///
/// **没装 WeeTodd 时一律回落原生，而且要说明** —— 静默回落等于骗人。
pub fn pick_route(engine_hint: &str, ltx_ok: bool, h3_ok: bool) -> Route {
    let h = engine_hint.to_lowercase();
    let wants_h3 = h.contains("h3") || h.contains("minimax");
    let wants_ltx = h.contains("ltx");
    // 点名了一个**不归这两条路管**的引擎（svd……）就交回原生。
    // 否则「我要 svd」会静默出一段 LTX —— 用户没有任何线索知道换过。
    // （Wan 已下架，在调用方先被 `video_tool::retired_video_engine` 拦下。）
    // 空 / "auto" 不算点名，那是"你看着办"。
    let named_other = !h.is_empty() && h != "auto" && !wants_h3 && !wants_ltx;
    if named_other {
        return Route::Native;
    }
    if wants_h3 && h3_ok {
        return Route::H3;
    }
    if ltx_ok && !wants_h3 {
        return Route::Ltx;
    }
    // 点名 H3 但没装 → 宁可用 LTX 也别用原生（原生的 H3 权重已删）
    if wants_h3 && ltx_ok {
        return Route::Ltx;
    }
    Route::Native
}

/// 回落到原生时给用户的说明。
pub fn route_note(route: Route, engine_hint: &str) -> Option<String> {
    let wants_h3 = {
        let h = engine_hint.to_lowercase();
        h.contains("h3") || h.contains("minimax")
    };
    let named_other = {
        let h = engine_hint.to_lowercase();
        !h.is_empty() && h != "auto" && !wants_h3 && !h.contains("ltx")
    };
    match route {
        Route::Native if named_other => Some(format!(
            "点名了 `{engine_hint}`，这个引擎不在 WeeTodd/MLX 那两条路上，走原生 ComfyUI 链路 —— \
             内存占用会高得多。想要低占用请用 `ltx` 或 `h3`。"
        )),
        Route::Ltx if wants_h3 => Some(format!(
            "点名了 MiniMax H3，但本机没装 WeeTodd 的 H3 节点，改用 LTX 2.5。\n{H3_MIGRATED_NOTE}"
        )),
        Route::Native => Some(
            "没装 WeeTodd 节点，回落到原生 ComfyUI 链路 —— 内存占用会高得多\
             （LTX 5 秒片实测 96.82 GB vs 22.84 GB），而且没有长片接续。"
                .to_string(),
        ),
        _ => None,
    }
}

/// 从 ComfyUI history 的 `outputs` 里找出视频文件。
///
/// **扫全部输出节点，不用固定 id 列表。** 原来两处各写了一份硬编码
/// （t2v 是 `["10","9","outputs"]`，i2v 是 `["15","14","11","10","outputs"]`），
/// 每接一条新链路就要去补一次，漏了的症状是「任务 success 但找不到文件」——
/// 又一个静默失败。WeeTodd 的 LTX 出在节点 `4`、H3 出在 `7`，正好都不在那两个列表里。
///
/// 三个键都认：`videos` / `gifs` / `images`。H3 和 VHS 都用 `gifs` 装 mp4。
pub fn find_video_output(outputs: &Value) -> Option<(String, String, String)> {
    let obj = outputs.as_object()?;
    // 节点顺序按 id 排，保证同一批输出每次挑中的是同一个。
    // **按数字排，不能按字符串** —— 字符串序里 "11" < "9"，
    // 而人读节点号是按数字读的，排错了会挑中不该挑的那个。
    let mut ids: Vec<&String> = obj.keys().collect();
    ids.sort_by_key(|k| (k.parse::<u64>().unwrap_or(u64::MAX), (*k).clone()));
    for id in ids {
        for key in ["videos", "gifs", "images"] {
            let Some(arr) = obj[id].get(key).and_then(Value::as_array) else {
                continue;
            };
            let Some(first) = arr.first() else { continue };
            let Some(name) = first.get("filename").and_then(Value::as_str) else {
                continue;
            };
            if name.is_empty() {
                continue;
            }
            // **`type: input` 是输入，不是产出。** ComfyUI 2026-09 起 `LoadVideo` 会把它载入的那条视频
            // 也报在 outputs 里（`{"images": [{"type": "input", ...}], "animated": [true]}`），
            // 而它的节点号最小 —— 于是深度捕捉拿回来的是原视频本身，动作迁移拿回来的是参考视频，
            // 任务状态都是成功。2026-09-23 用户截图：「深度动作捕捉为什么不是白模？变成了原视频」。
            if first.get("type").and_then(Value::as_str) == Some("input") {
                continue;
            }
            return Some((
                name.to_string(),
                first
                    .get("subfolder")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                first
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("output")
                    .to_string(),
            ));
        }
    }
    None
}

/// WeeTodd 装没装 —— 判据是 `/object_info` 里有没有那个节点。
///
/// **不查文件、不查目录**：ComfyUI 的模型可以放在 `extra_model_paths.yaml` 指的任意位置，
/// 而 `/object_info` 报的就是它真正认得的那一份。这和 `commands.rs` 里查 checkpoint
/// 的做法一致。
pub fn probe_installed(object_info: &Value, node: &str) -> bool {
    object_info.get(node).is_some()
}

/// H3 的资产从 2026-09-04 起只有 WeeTodd/MLX 这一套。
///
/// 原来那套（`MiniMax_H3_FL2VA_pruned_int8_convrot` + 两个 VAE + turbo LoRA，
/// 共 72.35 GB）已删除。删除依据见 `docs/STAGE_LOG.md`：内存峰值 49 GB → 15.8 GB，
/// 而且多出一个 222 秒的预览档。
///
/// 调用方拿不到 H3 模型时要原样报这句 —— 否则用户只会看到「没有匹配的模型」，
/// 不知道是被有意删掉的还是装漏了。
/// 带**视觉权重**的 H3 文本编码器可能在的地方（相对 ComfyUI 的 `models/`）。
///
/// 常驻的 `text_encoders/q8-paged` 是**纯文本**的 —— 它跑 t2va 完全没问题，
/// 但 fl2va / ref2va 要把首帧或参考图喂进编码器，节点会直接拒绝：
/// `The selected paged text_encoder is text-only.`
///
/// 这个判断**必须落在提交之前**。否则用户等到模型加载才拿到这句话，
/// 而 H3 光加载就要几十秒 —— 和"卡住了"长得一模一样。
/// 图生视频在这条路上有哪些**做不到**的事 —— 提交前就说，别生成完再让用户发现。
///
/// **末帧不在这个列表里。** 我一度以为 H3 不吃末帧并写进了这里，
/// 根据只是 `WeeToddH3Generate` 没有末帧入口 —— 但 H3 的关键帧是走
/// 条件节点的，`WeeToddH3LastFrame` / `WeeToddH3FirstLastFrame` 一直都在。
/// 两条路现在都支持首尾帧。
///
/// 静默丢参数是这个项目最常见的失败形状：视频出来了、任务是 success，
/// 只是末帧/参考图根本没起作用，而用户唯一能做的判断是"感觉不太对"。
pub fn animate_blocker(route: Route, wants_reference: bool) -> Option<String> {
    match route {
        Route::Ltx if wants_reference => Some(
            "LTX 2.5 这条路没有参考图入口（`mode: \"reference\"`）—— 它只认首帧和关键帧。\n             要用参考图请指定 `engine: \"h3\"`。"
                .into(),
        ),
        // H3 的 fl2va / ref2va 都要把图喂进文本编码器，缺视觉权重就跑不了
        Route::H3 if h3_vision_encoder().is_none() => Some(h3_vision_missing_note()),
        _ => None,
    }
}

/// 出图尺寸：按源图长宽比，对齐到 32 的倍数，并把长边压在 1344 以内。
///
/// **对齐不是美观问题。** LTX 的 VAE 下采样倍数是 32，给个 1343 会在
/// patchify 阶段报维度不匹配 —— 而那条错误信息里不会出现"1343"。
/// 帧率：**没给就用 LTX 2.5 的原生 24**，给了就夹到节点允许的 1~60。
///
/// 别默默换掉用户给的值 —— 帧率错了时长和口型会一起错位，而且不报错。
/// 上游 `WeeToddLTX25GenerationConfig` 的范围是 min 1 / max 60。
pub fn clamp_fps(requested: Option<f64>) -> f64 {
    const NATIVE: f64 = 24.0;
    match requested {
        Some(v) if v.is_finite() && v >= 1.0 => v.min(60.0),
        _ => NATIVE,
    }
}

pub fn fit_dimensions((w, h): (u32, u32)) -> (u32, u32) {
    const MAX_LONG: f64 = 1344.0;
    const ALIGN: u32 = 32;
    let (w, h) = (w.max(1) as f64, h.max(1) as f64);
    let scale = (MAX_LONG / w.max(h)).min(1.0);
    let round = |v: f64| ((v * scale / ALIGN as f64).round() as u32).max(1) * ALIGN;
    (round(w), round(h))
}

/// 找带视觉权重的 H3 文本编码器（在 ComfyUI 的 `models/` 下）。
pub fn h3_vision_encoder() -> Option<String> {
    crate::agent_runtime::video_tool::find_comfyui_input_dirs()
        .iter()
        .filter_map(|input| input.parent().map(|root| root.join("models")))
        .find_map(|models| h3_vision_encoder_in(&models))
}

/// 这次该用哪个文本编码器。
///
/// 文生用常驻的 q8-paged（小、够用）；首帧和参考图那两条必须用带视觉权重的
/// 常驻编码器 —— 传错不会静默出错，`preflight` 会明确拒绝，但那要等到模型
/// 加载才发生，所以选路也在这里一次做对。
pub fn h3_text_encoder(mode: &H3Mode<'_>) -> String {
    match mode {
        H3Mode::Text => H3_TEXT_ENCODER.to_string(),
        _ => h3_vision_encoder().unwrap_or_else(|| H3_VISION_ENCODER_CANDIDATES[0].to_string()),
    }
}

pub const H3_VISION_ENCODER_CANDIDATES: &[&str] = &[
    "MiniMax-H3/FL2VA/text_encoder",
    "MiniMax-H3/text_encoders/fl2va",
    "MiniMax-H3/text_encoders/bf16",
];

/// 在 `models_dir` 下找一份带视觉权重的 H3 文本编码器，返回它的 ComfyUI 相对名。
///
/// **空目录不算数。** 下载中断、或者只落了个 `config.json` 的目录看着都"存在"，
/// 但挂上去照样失败 —— 判据必须是"里面真有权重文件"。
pub fn h3_vision_encoder_in(models_dir: &std::path::Path) -> Option<String> {
    H3_VISION_ENCODER_CANDIDATES.iter().find_map(|rel| {
        let dir = models_dir.join(rel);
        let has_weights = std::fs::read_dir(&dir).ok()?.flatten().any(|e| {
            e.path()
                .extension()
                .is_some_and(|x| x == "safetensors" || x == "bin")
        });
        has_weights.then(|| (*rel).to_string())
    })
}

/// 缺视觉编码器时给用户看的话。**说清缺什么、多大、放哪**，而不是只说失败。
pub fn h3_vision_missing_note() -> String {
    format!(
        "MiniMax H3 的图生视频 / 参考图生视频需要一份**带视觉权重**的 Qwen3-VL 文本编码器，\n         本机现在只有纯文本的 `{TEXT}`（它只够跑文生视频）。\n\n         需要下载 `MiniMaxAI/MiniMax-H3` 仓库里的 `FL2VA/text_encoder/`（未量化，约 66.73 GB），\n         放到 ComfyUI 的 `models/{DEST}`。\n\n         在那之前：文生视频（LTX 2.5 / H3）和 LTX 2.5 的图生视频都正常可用。",
        TEXT = H3_TEXT_ENCODER,
        DEST = H3_VISION_ENCODER_CANDIDATES[0],
    )
}

pub const H3_MIGRATED_NOTE: &str =
    "MiniMax H3 已迁到 WeeTodd/MLX（models/MiniMax-H3/），\
     原来那套 ComfyUI 单文件权重已于 2026-09-04 删除。\
     H3 现在只能走 WeeTodd 节点，`diffusion_models/` 下不会再有 minimax 模型。";

// ─────────────────────────── MiniMax H3 ───────────────────────────

/// H3 的组件在 `models/MiniMax-H3/` 下，**不在 `diffusion_models/`**。
///
/// 和 LTX 2.5 那次不同：H3 走 WeeTodd 要一整套 MLX q8-paged 权重（61 GB），
/// 盘上原来那份 ComfyUI 单文件用不了，读代码确认是双重原因：
///
/// 1. 单文件形态只接受剪枝版，且 adaLN 曲线秩要 ≥ 32；我们那份是 8。
/// 2. 量化结构只能从**目录**里的 `quant_config.json` 重建，单文件走 `/__missing__`。
///
/// 所以 C 站上任何 ComfyUI 单文件微调版都用不了，和微调质量无关。
pub const H3_CHECKPOINT: &str = "MiniMax-H3/FL2VA";
pub const H3_TRANSFORMER: &str = "MiniMax-H3/transformers/q8_extended_paged";
pub const H3_TEXT_ENCODER: &str = "MiniMax-H3/text_encoders/q8-paged";
pub const H3_PROCESSOR: &str = "MiniMax-H3/FL2VA/processor";
pub const H3_TOKENIZER: &str = "MiniMax-H3/FL2VA/tokenizer";
pub const H3_VIDEO_VAE: &str = "MiniMax-H3/vae/q8/video_vae_affine_q8.safetensors";
/// **要 `FL2VA/audio_vae`，不是顶层的 `audio_vae/`。**
///
/// 官方仓库里有两个同名目录，都是 605 MB，只有布局不同：
/// 顶层那份是 diffusers 布局（`config.json` + `diffusion_pytorch_model.safetensors`），
/// 而这里要的是 `metadata.json` + `model.safetensors` —— 正是加载器读的两个文件名。
/// 下错了报的是「audio_vae metadata file not found」，看不出是目录选错了。
pub const H3_AUDIO_VAE: &str = "MiniMax-H3/FL2VA/audio_vae";

/// **原生 Ref2VA 权重**（2026-10-06）：WT 下载器（`model_downloads.prepare_download`）装在 `models/MiniMax-H3/downloads/` 下。
///
/// 没装的时候参考生视频 / 长镜头只能用 FL2VA 权重跑 Ref2VA 版式（`allow_fl2va_weights_for_ref2va`），
/// WT 自己对这个开关的说明：「Experimental … The official partitions share an architecture but not weights.」
/// 官方工作流：「Use a genuine Ref2VA partition for strict Ref2VA workflows.」核查见 `docs/reports/2026-10-06-H3提示词规范核查.md`。
pub const H3_REF2VA_TRANSFORMER: &str = "MiniMax-H3/downloads/h3-ref2va-q8-preconverted";
/// 原生 Ref2VA 的任务清单、音频 VAE、分词器、处理器（官方 `Ref2VA/` 里除权重以外的部分）
pub const H3_REF2VA_SUPPORT: &str = "MiniMax-H3/downloads/h3-ref2va-support";

/// 原生 Ref2VA 两份都装齐了吗。**判据是真有权重文件**（和视觉编码器同一条规矩：下载中断的空目录不算）。
pub fn h3_ref2va_native_in(models_dir: &std::path::Path) -> bool {
    let t = models_dir.join(H3_REF2VA_TRANSFORMER);
    let s = models_dir.join(H3_REF2VA_SUPPORT);
    let pages = std::fs::read_dir(t.join("pages")).ok().is_some_and(|mut d| d.any(|e| e.ok().is_some_and(|e| e.path().extension().is_some_and(|x| x == "safetensors"))));
    pages && t.join("paged_manifest.json").is_file() && s.join("model_index.json").is_file() && s.join("audio_vae/model.safetensors").is_file()
}

/// 本机装了原生 Ref2VA 吗（在 ComfyUI 的 `models/` 下找）。**装了不等于用** —— 用不用看 [`h3_ref2va_native`]。
pub fn h3_ref2va_native_installed() -> bool {
    crate::agent_runtime::video_tool::find_comfyui_input_dirs()
        .iter()
        .filter_map(|input| input.parent().map(|root| root.join("models")))
        .any(|models| h3_ref2va_native_in(&models))
}

/// 设置里打开了「原生 Ref2VA」吗（`config.json` 的 `h3_native_ref2va`，默认关）。
///
/// 直接读文件，和 `image_tool` 读出图底模同一个做法：出片的调用链上没有 config。读不到一律当关 ——
/// 关 = 兼容模式，是验证过能出对人的那一条。
fn h3_ref2va_native_enabled() -> bool {
    if let Ok(v) = std::env::var("ANIMA_H3_NATIVE_REF2VA") {
        return v == "1";
    }
    let Some(dir) = dirs::config_dir() else { return false };
    std::fs::read_to_string(dir.join("com.anima.desktop").join("config.json"))
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|v| v.get("h3_native_ref2va").and_then(Value::as_bool))
        .unwrap_or(false)
}

/// 这一次参考模式出片用原生 Ref2VA 吗：**装了，而且设置里打开了**。
///
/// 2026-10-06 之前是「装了就自动用」。EP03 第 2 镜同种子对照，原生三条都把第二女角色（苏晴）换成了星澜，
/// 兼容模式那条正常 —— 用户定：做成开关、默认兼容模式。
///
/// 测试里一律当没装 —— 否则图的形状跟着这台机器下没下完权重变（切换逻辑单测 [`h3_loader`]）。
/// 实跑对照（`video_tool` 的 `参考生视频实测`）设 `ANIMA_H3_REAL_MODELS=1` 才按本机真实情况判，
/// `ANIMA_H3_NATIVE_REF2VA=1/0` 可以绕过设置直接指定。
pub fn h3_ref2va_native() -> bool {
    if cfg!(test) && std::env::var_os("ANIMA_H3_REAL_MODELS").is_none() {
        return false;
    }
    use_native_ref2va(h3_ref2va_native_installed(), h3_ref2va_native_enabled())
}

/// 两个条件缺一不可。单独拎出来是为了测得到：最容易写错成「装了就用」（10-06 之前的行为）
pub fn use_native_ref2va(installed: bool, enabled: bool) -> bool {
    installed && enabled
}

/// **这张图实际走的链路**，一句话给人看：「FL2VA 首帧 · LightX2V 4 步」「Ref2VA 兼容（FL2VA 权重）· Larry v4 6 次 · 接上一镜」。
///
/// **从要提交给 ComfyUI 的那张图里读，不从设置推** —— 用户 2026-10-07：右下角进度前面显示当前链路，「不是硬编码出来的，而是返回的」。
/// 设置写的是「想要什么」，图才是「真发了什么」：原生 Ref2VA 没装退回兼容、加速 LoRA 对不上退回稠密，这些都只在图里看得到。
/// 读不出来（不是 WeeTodd 的图）回 `None`，界面就不显示，不编一个。
pub fn graph_pipeline(graph: &Value) -> Option<String> {
    let nodes = graph.as_object()?;
    let of = |class: &'static str| nodes.values().filter(move |n| n["class_type"] == class);
    if of("WeeToddLTX25ComponentLoader").next().is_some() {
        return Some(if of("WeeToddLTX25Keyframe").next().is_some() { "LTX 2.5 首帧" } else { "LTX 2.5 文生" }.to_string());
    }
    let loader = &of("WeeToddH3ComponentLoader").next()?["inputs"];
    let mode = match loader["task"].as_str()? {
        "t2va" => "T2VA 文生".to_string(),
        "fl2va" => "FL2VA 首帧".to_string(),
        "ref2va" if loader["allow_fl2va_weights_for_ref2va"] == true => "Ref2VA 兼容（FL2VA 权重）".to_string(),
        "ref2va" => "Ref2VA 原生".to_string(),
        other => other.to_string(),
    };
    let mut parts = vec![mode];
    let steps = of("WeeToddH3GenerationConfig").next().and_then(|n| n["inputs"]["steps"].as_u64());
    if let Some(lora) = of("WeeToddH3LoRALoader").find(|n| n["inputs"]["profile"] == "turbo") {
        let name = lora["inputs"]["lora_name"].as_str().unwrap_or("");
        let who = match name {
            H3_LARRY_TURBO_LORA => "Larry v4".to_string(),
            H3_LIGHTX2V_TURBO_LORA => "LightX2V".to_string(),
            H3_REF2VA_TURBO_LORA => "LightX2V Ref2VA 专用".to_string(),
            H3_GENERIC_TURBO_LORA => "drbaph 精简版".to_string(),
            n => n.rsplit('/').next().unwrap_or(n).trim_end_matches(".safetensors").to_string(),
        };
        // 自己挂的加速 LoRA：采样点数 = GenerationConfig 的 steps，求值次数少一
        parts.push(match steps { Some(p) if p > 1 => format!("{who} {} 次", p - 1), _ => who });
    } else if let Some(preset) = of("WeeToddH3ValidatedSamplingPreset").next().and_then(|n| n["inputs"]["preset"].as_str()) {
        let evals = preset.split_whitespace().collect::<Vec<_>>();
        let n = evals.windows(2).filter(|w| w[1] == "evaluations" || w[1] == "Turbo").filter_map(|w| w[0].parse::<u32>().ok()).sum::<u32>()
            + evals.windows(2).find(|w| w[1] == "base").and_then(|w| w[0].parse::<u32>().ok()).unwrap_or(0);
        let who = if preset.contains("drbaph") { "drbaph 精简版" } else if preset.starts_with("Dense") { "稠密不加速" } else { preset };
        parts.push(if n > 0 { format!("{who} {n} 次") } else { who.to_string() });
    }
    let windows = of("WeeToddH3Sample").count();
    if windows > 1 {
        parts.push(format!("{windows} 段窗口链"));
    }
    if of("WeeToddH3ReferenceVideo").next().is_some() {
        parts.push("接上一镜".into());
    }
    let audios = of("WeeToddH3ReferenceAudio").count();
    if audios > 0 {
        parts.push(format!("参考音频 {audios} 段"));
    }
    Some(parts.join(" · "))
}

/// H3 组件加载节点的输入。
///
/// **三种模式对应三个分区，写错会在编码那一步才炸。** 首帧那条实测报的是「H3 keyframe encoding requires an FL2VA
/// component set」—— 我一开始按「首帧也是文生的一种」写成了 t2va，单元测试照样绿（它只是复述了我的错误假设），真跑才抓到。
///
/// **文本编码器按模式换。** q8-paged 那份是纯文本子集（上游转换脚本第一行就写着 "text subset"），fl2va / ref2va 要把图喂进
/// 编码器，preflight 会拒一切分页编码器。文生仍用 q8-paged —— 它小得多，而且文生根本用不到视觉塔。
///
/// **参考模式有原生 Ref2VA 就用它**（`native`），任务清单、音频 VAE、分词器、处理器都换成 Ref2VA 那套、关掉兼容开关；
/// 文本编码器和视频 VAE 两个分区共用（官方 Ref2VA 配套里本来就不带）。没装才退回 FL2VA 权重 + 兼容开关。
fn h3_loader(task: &str, ref2va: bool, text_encoder: &str, native: bool) -> Value {
    if native {
        return json!({
            "checkpoint": H3_REF2VA_SUPPORT, "task": task,
            "transformer": H3_REF2VA_TRANSFORMER, "text_encoder": text_encoder,
            "processor": format!("{H3_REF2VA_SUPPORT}/processor"), "tokenizer": format!("{H3_REF2VA_SUPPORT}/tokenizer"),
            "video_vae": H3_VIDEO_VAE, "audio_vae": format!("{H3_REF2VA_SUPPORT}/audio_vae"),
            "allow_fl2va_weights_for_ref2va": false
        });
    }
    json!({
        "checkpoint": H3_CHECKPOINT, "task": task,
        "transformer": H3_TRANSFORMER, "text_encoder": text_encoder,
        "processor": H3_PROCESSOR, "tokenizer": H3_TOKENIZER,
        "video_vae": H3_VIDEO_VAE, "audio_vae": H3_AUDIO_VAE,
        "allow_fl2va_weights_for_ref2va": ref2va
    })
}

/// Ref2VA 一次最多挂几段参考音频（WeeTodd `minimax_h3_mlx/ref2va.py` 的 `MAX_REFERENCE_AUDIOS`）
pub const H3_MAX_REFERENCE_AUDIOS: usize = 3;

/// 判断 WeeTodd 的 H3 装没装。
pub const H3_PROBE_NODE: &str = "WeeToddH3Generate";

/// 三档采样预设 —— 本机同题实测（768×512 / 5 秒，384p 档是 640×384）。
///
/// | 档 | 耗时 | 内存峰值 |
/// |---|---|---|
/// | `Preview` | 222s | 15.77 GB |
/// | `Standard` | 384s | 15.87 GB |
/// | `Staged` | 528s | 15.83 GB |
///
/// **`Staged`（6 次评估）看不出比 `Standard`（4 次）好** —— 多花 144 秒买不到
/// 可见的画质。留着这一档只是为了以后能复测，日常别用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum H3Tier {
    /// 预览 / 批量试镜。640×384，实测 222 秒。
    Preview,
    /// 正式镜头。768×512，实测 384 秒。
    Standard,
    /// 2 base + 4 turbo（双采法，上游官方「Balance」档）。
    /// 2026-09-14 第 11 镜 6 秒同种子对照：896×512 16.0 分钟，1280×736 43.0 分钟（Turbo 1280×736 为 31.9 分钟）。
    Staged,
    /// 20 点 / 19 次真实求值（上游「Performance」档）。质量优先，大约是 Turbo 的四五倍时间，没有实测。
    Dense,
    /// LightX2V 首尾帧 4 步 v0.1（[`H3_LIGHTX2V_TURBO_LORA`]），4 次求值、强度 0.0625、不经过预设。
    /// 2026-10-07 EP03 第 10 镜 480p 同种子对照：20.2 分钟（双采 28.9），礼服一直是正红缎面（双采 2.5 秒漂成发光洋红）。用户定：加进链路做一个选项，双采保留。
    LightX2V,
    /// drbaph 精简版 4 次求值、经过预设 —— 10-07 之前「标准」档的原样。**只给实跑验证过、要原样保留的链路用**
    /// （动作迁移「原片复刻 · 快」，Kotte 实跑那张图一字不差地钉在测试里）。出片设置里没有这一档。
    LegacyTurbo,
}

impl H3Tier {
    pub fn preset(self) -> &'static str {
        match self {
            Self::Preview => {
                "Turbo — drbaph v4 step-600 — 384p low-memory — 5 points / 4 evaluations"
            }
            Self::Standard => "Turbo — drbaph v4 step-600 — 5 points / 4 evaluations",
            Self::Staged => {
                "Staged Turbo — drbaph v4 step-600 — 2 base + 4 Turbo evaluations"
            }
            Self::Dense => "Dense baseline — 20 points / 19 evaluations",
            // 不经过预设（见 `H3SamplingPlan::new`）；万一被拿去当预设，至少是同样 5 点 / 4 次的那档
            Self::LightX2V | Self::LegacyTurbo => "Turbo — drbaph v4 step-600 — 5 points / 4 evaluations",
        }
    }
    /// 这一档预设自带的加速 LoRA（WeeTodd 预设表里点名的文件）。`None` = 不挂。
    pub fn turbo_lora(self) -> Option<&'static str> {
        match self {
            Self::Preview | Self::Standard | Self::Staged | Self::LegacyTurbo => Some(H3_GENERIC_TURBO_LORA),
            Self::Dense => None,
            Self::LightX2V => Some(H3_LIGHTX2V_TURBO_LORA),
        }
    }
    /// 一次出片里主干真正跑几次
    pub fn evaluations(self) -> u32 {
        match self {
            Self::Preview | Self::Standard | Self::LegacyTurbo => 4,
            Self::Staged => 6,
            Self::Dense => 19,
            Self::LightX2V => 4,
        }
    }
    /// 实测耗时（秒），用于超时预算和给用户的等待提示。
    pub fn secs(self) -> u64 {
        match self {
            Self::Preview => 222,
            Self::Standard | Self::LegacyTurbo => 384,
            Self::Staged => 528,
            Self::Dense => 1800,
            // 双采的七成（10-07 对照 20.2 / 28.9 分钟）
            Self::LightX2V => 370,
        }
    }
}

/// 前三档预设自带的那个加速 LoRA。
///
/// 文件自己的元数据（2026-10-07 读的）：`partial_conversion: true`、`target_model: MiniMax-H3-pruned-curve`、
/// 删掉了全部 51 对 AdaLN 适配器，警告「Four-step distillation behaviour may be degraded or broken」，不写属于哪个分区。
/// 它是给**剪枝曲线版**主干转的 —— FL2VA 那份（`time_embed_dim` 64、`adaln_curve_grid` 1001）正是；
/// 原生 Ref2VA 那份是完整 AdaLN（`time_embed_dim` 2688），结构和分区都对不上。
pub const H3_GENERIC_TURBO_LORA: &str = "minimax_h3_turbo_v4_step600_ema_pruned_comfyui.safetensors";

/// LightX2V 首尾帧 4 步 v0.1（`lightx2v/Minimax-h3-Turbo`，2026-10-07 下）。diffusers PEFT 格式、**文件里没写 alpha**：
/// WeeTodd 认这种键名（`transformer_blocks.` → `blocks.`、`to_q/k/v` 合成 qkv），没 alpha 时把强度当缩放，
/// 所以强度要 [`H3_LIGHTX2V_STRENGTH`]（= 训练 alpha 8 / rank 128）。不碰 AdaLN，剪枝曲线版 FL2VA 主干能挂。
pub const H3_LIGHTX2V_TURBO_LORA: &str = "minimax_h3_fl2v_turbo_4step_v0.1.safetensors";
pub const H3_LIGHTX2V_STRENGTH: f64 = 0.0625;

/// **Larry v4 step-600 EMA 完整版**（`larryvrh/MiniMax-H3-Turbo-Lora`），快速 / 标准 / 双采那三档用它（2026-10-07 用户定，替换 drbaph 精简版）。
///
/// drbaph 那份（[`H3_GENERIC_TURBO_LORA`]）就是它删掉 51 对 AdaLN 适配器的残缺转换 —— 而 Larry 的蒸馏效果主要就在那 102 个 AdaLN 张量上。
/// 完整版挂在剪枝曲线版 FL2VA 主干上要 [`H3_LARRY_GRID`]（`silu(time_embedder(t))`，随 Larry 自己的 ComfyUI 节点发布），
/// WeeTodd 会在 LoRA 旁边找同名文件。**AdaLN 适配器不能「前几步不挂」**（WeeTodd：Staged LoRA activation does not support AdaLN adapter targets），
/// 所以双采那一档改成 Larry 官方推荐的 6 次求值（7 个采样点）全程挂。
/// 实测（10-07 EP03，480p）：参考出片第 2 镜 33.6 分钟、人和衣服都对；首帧出片第 10 镜多出一个人、铭牌冒乱字。
pub const H3_LARRY_TURBO_LORA: &str = "minimax_h3_turbo_v4_step600_ema.safetensors";
pub const H3_LARRY_GRID: &str = "h3_silu_temb_grid.safetensors";

/// Larry 完整版和它的 AdaLN 时间网格都在 `models/loras/` 下吗。缺一样就退回 drbaph（[`H3SamplingPlan::current`]）
pub fn h3_larry_installed() -> bool {
    crate::agent_runtime::video_tool::find_comfyui_input_dirs()
        .iter()
        .filter_map(|input| input.parent().map(|root| root.join("models").join("loras")))
        .any(|d| d.join(H3_LARRY_TURBO_LORA).is_file() && d.join(H3_LARRY_GRID).is_file())
}

/// **Ref2VA 专用**的 4 步加速 LoRA（LightX2V / ModelTC 官方蒸馏，`lightx2v/Minimax-h3-Turbo`，2026-10-07 下）。
///
/// 元数据：`source_format: Diffusers PEFT LoRA`、rank 128 / alpha 8、**不碰 AdaLN**（剪枝 / 完整主干都能挂）；
/// `base_model` 写的是 `Comfy-Org/MiniMax-H3 minimax_h3_fl2va_bf16.safetensors` —— 是 ComfyUI 打包时用的结构名还是真拿 FL2VA 训的，**没坐实**。
/// 官方推荐 4 步、video / audio shift 12 / 3；WeeTodd 的 Turbo 日程是 5 点 / 4 次评估，不暴露 shift。
/// 2026-10-07 Run D（EP03 第 2 镜）：苏晴回来了，但下半身串了星澜的纱裙、青年复制成两个 —— 原生仍是实验。
/// 原生 Ref2VA 装了它就默认挂（见 [`H3SamplingPlan::current`]）。
pub const H3_REF2VA_TURBO_LORA: &str = "minimax_h3_ref2v_turbo_4step_v0.1_comfyui_bf16.safetensors";

/// 专用加速 LoRA 在不在 ComfyUI 的 `models/loras/` 下、**是不是我们核过的那一份**（[`ref2va_turbo_check`]）。
/// 不是的话退回稠密，日志说明原因 —— 不静默挂一个不认识的文件（Codex 复查 §21.3）。
pub fn h3_ref2va_turbo_installed() -> bool {
    let found = crate::agent_runtime::video_tool::find_comfyui_input_dirs()
        .iter()
        .filter_map(|input| input.parent().map(|root| root.join("models").join("loras").join(H3_REF2VA_TURBO_LORA)))
        .find(|p| p.is_file());
    let Some(path) = found else { return false };
    match ref2va_turbo_check(&path) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(path = %path.display(), "Ref2VA 专用加速 LoRA 对不上，原生 Ref2VA 退回稠密：{e}");
            false
        }
    }
}

/// 核这份 LoRA 是不是 2026-10-07 下的那一份：大小、元数据（PEFT 转 ComfyUI、rank 128 / alpha 8、不是残缺转换）、
/// 张量键名（`diffusion_model.blocks.N.` 开头、不碰 AdaLN）。读 safetensors 头，不读权重（1.96 GB 每次哈希太慢）。
pub fn ref2va_turbo_check(path: &std::path::Path) -> Result<(), String> {
    use std::io::Read;
    const SIZE: u64 = 1_956_193_000;
    let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if len != SIZE {
        return Err(format!("大小 {len} 字节，核过的那份是 {SIZE}"));
    }
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut n = [0u8; 8];
    f.read_exact(&mut n).map_err(|e| e.to_string())?;
    let n = u64::from_le_bytes(n);
    if n > 64 << 20 {
        return Err("safetensors 头太大，不是正常文件".into());
    }
    let mut head = vec![0u8; n as usize];
    f.read_exact(&mut head).map_err(|e| e.to_string())?;
    let head: Value = serde_json::from_slice(&head).map_err(|e| format!("头不是 JSON：{e}"))?;
    ref2va_turbo_header_check(&head)
}

fn ref2va_turbo_header_check(head: &Value) -> Result<(), String> {
    let meta = head.get("__metadata__").cloned().unwrap_or_default();
    let m = |k: &str| meta.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    if m("partial_conversion") == "true" {
        return Err("元数据写着 partial_conversion（残缺转换）".into());
    }
    if !m("source_format").contains("PEFT") || m("training_rank") != "128" || m("training_alpha") != "8.0" {
        return Err(format!("元数据对不上：source_format={} rank={} alpha={}", m("source_format"), m("training_rank"), m("training_alpha")));
    }
    let keys: Vec<&String> = head.as_object().map(|o| o.keys().filter(|k| *k != "__metadata__").collect()).unwrap_or_default();
    // 主干块 `diffusion_model.blocks.N.*` + 文本细化块 `diffusion_model.token_refiner.blocks.N.*`（真文件里两样都有）
    if !keys.iter().all(|k| k.starts_with("diffusion_model.")) || !keys.iter().any(|k| k.starts_with("diffusion_model.blocks.")) {
        return Err("张量键名不是 diffusion_model.blocks.N.* 的布局".into());
    }
    if keys.iter().any(|k| k.contains("adaln")) {
        return Err("碰了 AdaLN（专用那份不碰）".into());
    }
    Ok(())
}

/// 这一次 H3 **实际**怎么采样。界面、日志、图都从这一份来，不各算各的。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct H3SamplingPlan {
    /// 用原生 Ref2VA 主干吗
    pub native_ref2va: bool,
    /// 调用方要的档
    pub requested: H3Tier,
    /// 实际用的档
    pub tier: H3Tier,
    /// 不用预设自带的 LoRA、改挂这个（原生 Ref2VA + Ref2VA 专用加速 LoRA 的对照）。`None` = 预设怎么定就怎么来
    pub own_turbo: Option<&'static str>,
    /// [`Self::own_turbo`] 的强度。专用 Ref2VA 那份自带 alpha，1.0；没写 alpha 的（LightX2V 首尾帧 4 步 v0.1）要 0.0625
    pub turbo_strength: f64,
    /// [`Self::own_turbo`] 时的采样点数（评估次数 + 1）。4 步 = 5，8 步 = 9
    pub turbo_points: u32,
}

impl H3SamplingPlan {
    /// **原生 Ref2VA 一律走稠密 19 次、不挂加速 LoRA**（2026-10-07 用户同意）。
    ///
    /// 原生三条对照都把苏晴换成星澜，而它和兼容模式唯一的差别是主干 —— 那个加速 LoRA 是给剪枝曲线版 FL2VA 转的
    /// （见 [`H3_GENERIC_TURBO_LORA`]），挂到完整 AdaLN 的原生 Ref2VA 上分区、结构双重不配。
    /// WeeTodd 自己对原生 Ref2VA Q8 唯一验过的也是「稠密 19 次、不挂 Turbo」。
    /// 等找到并验过 Ref2VA 专用的加速 LoRA 再放开。兼容模式照旧。
    pub fn new(ref2va: bool, requested: H3Tier, native_installed_and_on: bool) -> Self {
        let native_ref2va = ref2va && native_installed_and_on;
        let tier = if native_ref2va { H3Tier::Dense } else { requested };
        let plan = Self { native_ref2va, requested, tier, own_turbo: None, turbo_strength: 1.0, turbo_points: 5 };
        // 不经过预设、自己挂 LoRA 的几档（原生 Ref2VA 已经改稠密，`with_trial_turbo` 对它不动）：
        // LightX2V；快速 / 标准 / 双采换成 Larry 完整版（双采 = 6 次求值，见 [`H3_LARRY_TURBO_LORA`]）
        match tier {
            H3Tier::LightX2V => plan.with_trial_turbo(H3_LIGHTX2V_TURBO_LORA, H3_LIGHTX2V_STRENGTH, 5),
            H3Tier::Preview | H3Tier::Standard => plan.with_trial_turbo(H3_LARRY_TURBO_LORA, 1.0, 5),
            H3Tier::Staged => plan.with_trial_turbo(H3_LARRY_TURBO_LORA, 1.0, 7),
            H3Tier::Dense | H3Tier::LegacyTurbo => plan,
        }
    }
    /// 原生 Ref2VA 改挂 Ref2VA 专用加速 LoRA（4 次评估）。只有原生才换；其余原样返回
    pub fn with_ref2va_turbo(self) -> Self {
        if !self.native_ref2va {
            return self;
        }
        // 档位取「标准」只为拿它那份 5 点 / 4 次 / euler 的配置；它自带的通用 LoRA 不接进采样（见 `h3_prefix_with`）
        Self { tier: H3Tier::Standard, own_turbo: Some(H3_REF2VA_TURBO_LORA), ..self }
    }
    /// 参考图像素预算。**原生 Ref2VA + 专用加速 LoRA 照官方用 100（= 参考图面积不超过成片画布，LightX2V 的 `match`）**；
    /// 其余照旧按最大那张参考图算（`reference_pixel_budget`，09-29 为了设定卡上的脸放到 100 以上）。
    ///
    /// 2026-10-07 核了官方 LightX2V `configs/minimax_h3/dmd/minimax_h3_ref2av_4step.json`：4 步、video / audio shift 12 / 3、
    /// 关 CFG、LoRA 强度 1、`training_euler`、输出 768×1344 —— WeeTodd 这条已经一致（shift 从 `model_index.json` 读，默认 12 / 3；
    /// 5 点 / 4 次 = 在 1, .75, .5, .25 上加 shift）。只有参考图缩放不一样：我们放到了 163%。
    pub fn reference_budget(&self, computed: u32) -> u32 {
        if self.native_ref2va && self.own_turbo.is_some() { 100 } else { computed }
    }
    /// 这一次实际挂的加速 LoRA
    pub fn turbo(&self) -> Option<&'static str> {
        self.own_turbo.or_else(|| self.tier.turbo_lora())
    }
    /// 这一次的计划（读本机装没装、设置开没开）。
    ///
    /// **原生 Ref2VA 装了专用加速 LoRA 就挂它**（2026-10-07 用户：稠密 19 次「无实用性，没必要耗费时间」）。
    /// Run D 实测 42.9 分钟、苏晴回来了（通用 LoRA 那三条是被换成星澜）。没装这个 LoRA 才退回稠密 —— 日志和设置页都写明。
    /// `ANIMA_H3_REF2VA_TURBO=0` 强制稠密（只给对照用）。
    pub fn current(ref2va: bool, requested: H3Tier) -> Self {
        let plan = Self::new(ref2va, requested, ref2va && h3_ref2va_native());
        let plan = if plan.own_turbo == Some(H3_LARRY_TURBO_LORA) && !(cfg!(test) || h3_larry_installed()) {
            tracing::warn!("Larry 完整版或 {H3_LARRY_GRID} 不在 models/loras/ 下，这一档退回 drbaph 精简版（预设）");
            plan.without_own_turbo()
        } else {
            plan
        };
        let off = std::env::var("ANIMA_H3_REF2VA_TURBO").is_ok_and(|v| v == "0");
        let plan = plan.pick_turbo(plan.native_ref2va && h3_ref2va_turbo_installed(), off);
        // **加速 LoRA 对照**（2026-10-07，只给实跑用，界面没有入口）：`ANIMA_H3_TRIAL_LORA=h3-candidates/x.safetensors`
        // `ANIMA_H3_TRIAL_STRENGTH`（默认 1）`ANIMA_H3_TRIAL_POINTS`（默认 5 = 4 次评估）。只换非原生那条（首帧 / 首尾帧 / 兼容参考）
        match std::env::var("ANIMA_H3_TRIAL_LORA") {
            Ok(name) if !name.is_empty() && !plan.native_ref2va => {
                let strength = std::env::var("ANIMA_H3_TRIAL_STRENGTH").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
                let points = std::env::var("ANIMA_H3_TRIAL_POINTS").ok().and_then(|v| v.parse().ok()).unwrap_or(5);
                plan.with_trial_turbo(Box::leak(name.into_boxed_str()), strength, points)
            }
            _ => plan,
        }
    }
    /// 换一个加速 LoRA 试（不经过预设，配置取节点 2，采样点 `points`）。原生 Ref2VA 那条不动
    pub fn with_trial_turbo(self, name: &'static str, strength: f64, points: u32) -> Self {
        if self.native_ref2va {
            return self;
        }
        Self { own_turbo: Some(name), turbo_strength: strength, turbo_points: points.max(2), ..self }
    }
    /// 退回预设自带的那个 LoRA（Larry 没装时）
    pub fn without_own_turbo(self) -> Self {
        Self { own_turbo: None, turbo_strength: 1.0, turbo_points: 5, ..self }
    }
    /// 实际评估次数
    pub fn evaluations(&self) -> u32 {
        if self.own_turbo.is_some() { self.turbo_points - 1 } else { self.tier.evaluations() }
    }
    /// 原生 Ref2VA：装了专用加速 LoRA、又没强制稠密，就挂它；否则原样（稠密）
    pub fn pick_turbo(self, turbo_installed: bool, force_dense: bool) -> Self {
        if turbo_installed && !force_dense { self.with_ref2va_turbo() } else { self }
    }
    /// 一行话：实际用的主干、档位、评估次数、挂没挂加速 LoRA。写进日志和任务说明
    pub fn describe(&self) -> String {
        let dit = if self.native_ref2va { "原生 Ref2VA 主干" } else { "FL2VA 主干" };
        let lora = self.turbo().map_or("不挂加速 LoRA".to_string(), |l| format!("加速 LoRA {l}"));
        let changed = if self.own_turbo == Some(H3_LARRY_TURBO_LORA) && !self.native_ref2va {
            format!("（Larry v4 完整版：{} 次求值，不经过预设）", self.turbo_points - 1)
        } else if self.tier == H3Tier::LightX2V && self.own_turbo == Some(H3_LIGHTX2V_TURBO_LORA) {
            format!("（LightX2V 4 步：强度 {}，不经过预设）", self.turbo_strength)
        } else if self.own_turbo.is_some() && !self.native_ref2va {
            format!("（加速 LoRA 对照：强度 {}，{} 个采样点，不经过预设）", self.turbo_strength, self.turbo_points)
        } else if self.own_turbo.is_some() {
            "（原生 Ref2VA 挂专用加速 LoRA，不用预设自带的通用那个）".to_string()
        } else if self.requested != self.tier {
            format!("（要的是 {:?}；原生 Ref2VA 不能挂通用加速 LoRA、本机又没装专用的，改稠密）", self.requested)
        } else {
            String::new()
        };
        format!("{dit} · {:?} {} 次评估 · {lora}{changed}", self.tier, self.evaluations())
    }
}

/// H3 的三种输入模式。**三种共用同一条主干**，只换「条件」那一段：
///
/// ```text
/// t2v   TextEncode(prompt)                                            → Sample
/// i2v   FirstFrame/LastFrame/FirstLastFrame → KeyframeEncode(prompt)  → Sample
/// r2v   ReferenceImage(image) → ReferenceEncode(prompt) → Sample
/// ```
///
/// `LoadImage` 的文件名是 ComfyUI `input/` 下的相对名 —— 调用方负责先把图拷进去。
#[derive(Debug, Clone)]
pub enum H3Mode<'a> {
    /// 纯文生视频。
    Text,
    /// 关键帧驱动 —— 首帧、末帧、或两者。**分区就叫 FL2VA（First-Last）**，
    /// 三种情况都走它，只是条件节点不同。
    ///
    /// 我一开始只接了首帧，还在 `animate_blocker` 里写了「H3 不吃末帧」——
    /// 那是看 `WeeToddH3Generate` 没有末帧入口就下的结论，**没查条件节点**。
    /// 实际上 `WeeToddH3LastFrame` 和 `WeeToddH3FirstLastFrame` 都在。
    Frames {
        first: Option<&'a str>,
        last: Option<&'a str>,
    },
    /// 参考图（身份/服装/场景），`pixel_budget_percent` 控制编码密度。
    ///
    /// `audios`：参考音频（ComfyUI `input/` 下的文件名），串在图片 / 视频参考之后，编号 `<Audio 1..>`
    /// （和 `<Picture N>` / `<Video N>` 各自独立编号，官方 Ref2VA 指南 2.5）。WeeTodd 规定**至少还要一张图或一段视频**，
    /// 最多 3 段（`ref2va.MAX_REFERENCE_AUDIOS`）。画布「音频素材」节点用（2026-10-08 用户：画布里加音频参考）。
    Reference {
        images: &'a [String],
        pixel_budget_percent: u32,
        audios: &'a [String],
    },
}

/// 接上一镜：把它尾段的帧当作运镜 / 光线 / 布景的参考。
///
/// # 为什么是 `WeeToddH3ReferenceVideo` 而不是首帧
///
/// 它接在参考链的**最后**，在提示词里是 `<Video 1>`（图片是 `<Picture N>`，
/// 两套编号各数各的 —— 官方示例工作流 `h3_ref2va_balance.json` 就是这么写的）。
///
/// **只绑数据不写标签是无效的。** 提示词里没有 `<Video 1>`，这段视频等于没传；
/// 反过来只写标签不传数据也一样。两边必须同时对 —— 见
/// [`crate::prompt_route_b`] 里生成那句定义的地方。
///
/// `video_size` 用 `match output`：另一档 `native H3 reference canvas` 上游写明
/// 「dramatically slower」。2026-09-20 实测 match output 这一档**没有变慢**
/// （63.9 分钟 vs 不带参考的 68.0）。
pub struct H3Continuity<'a> {
    /// 上一镜成片的绝对路径。
    pub video_path: &'a str,
    /// 从第几秒开始取。
    pub start_secs: f64,
    /// 取多少帧。实测 24 帧（1 秒）够了。
    pub frames: u32,
    pub fps: f64,
}

pub struct H3Params<'a> {
    pub mode: H3Mode<'a>,
    /// 接上一镜。`None` = 不接。见 [`H3Continuity`]。
    pub continuity: Option<H3Continuity<'a>>,
    pub prompt: &'a str,
    pub width: u32,
    pub height: u32,
    pub duration_secs: f64,
    pub seed: u64,
    pub tier: H3Tier,
    pub filename_prefix: &'a str,
    /// 额外挂的 LoRA（预设自带的 turbo 不用写在这里）。
    pub loras: &'a [(String, f64)],
}

/// 一次 H3 生成的**公共前缀**：组件、预览守卫、配置、预设、preflight。
///
/// 单窗口（[`build_h3_graph`]）和窗口链（[`build_h3_chain_graph`]）共用这一段。
/// **不要各写一份** —— 组件加载那几个键写错会在编码那一步才炸，
/// 两份各写各的迟早只改一边。
fn h3_prefix(task: &str, ref2va: bool, text_encoder: &str, width: u32, height: u32,
             duration_secs: f64, seed: u64, tier: H3Tier) -> Map<String, Value> {
    h3_prefix_with(task, text_encoder, width, height, duration_secs, seed, H3SamplingPlan::current(ref2va, tier), ref2va)
}

#[allow(clippy::too_many_arguments)]
fn h3_prefix_with(task: &str, text_encoder: &str, width: u32, height: u32,
                  duration_secs: f64, seed: u64, plan: H3SamplingPlan, ref2va: bool) -> Map<String, Value> {
    // 原生 Ref2VA 上挂了通用加速 LoRA = 10-06 串人那条组合。计划里已经排除了，这里再钉一次：谁改了 `new` 先在这里炸
    assert!(
        !(plan.native_ref2va && plan.turbo() == Some(H3_GENERIC_TURBO_LORA)),
        "原生 Ref2VA 不能挂通用加速 LoRA：{}", plan.describe()
    );
    tracing::info!(task, plan = %plan.describe(), "H3 采样计划");
    let tier = plan.tier;
    let mut m = Map::new();
    // 专用加速 LoRA 自己起一条链（不接预设的输出），采样从 [`LORA_START`] 取
    if let Some(name) = plan.own_turbo {
        m.insert(
            "899".into(),
            node(
                "WeeToddH3LoRALoader",
                json!({
                    "lora_name": name, "strength": plan.turbo_strength, "profile": "turbo",
                    "adaln_input_grid": "", "qkv_layout": "auto", "start_after_evaluations": 0
                }),
            ),
        );
    }
    m.insert(
        "1".into(),
        node("WeeToddH3ComponentLoader", h3_loader(task, ref2va, text_encoder, plan.native_ref2va)),
    );
    // 预览覆盖：采样中途出真彩缩略图，并在 latent 变成非有限值时**立刻中止**。
    // 这条比它看起来重要 —— 这个项目最典型的失败就是「跑完了才发现是纯黑」。
    //
    // **守卫用 `abort non-finite`，不用 `conservative collapse guard`。**
    // 后者在采样后半程看到「连续几帧没有特征」就停，2026-09-13 试片 B（深夜办公室、
    // 画面本来就暗）第一次被它误杀，关掉之后同一条提示词正常出片。
    // 真正的崩坏（纯黑的根因）是 latent 变成 NaN / Inf，`abort non-finite` 正好只管这个。
    m.insert(
        "9".into(),
        node(
            "WeeToddH3PreviewOverride",
            json!({
                "components": ["1", 0], "tae_model": "taeh3.safetensors",
                "preview_backend": "mlx", "coreml_model": "",
                "preview_every": 1, "preview_frames": 6, "max_preview_edge": 256,
                "safety_guard": "abort non-finite"
            }),
        ),
    );
    m.insert(
        "2".into(),
        node(
            "WeeToddH3GenerationConfig",
            json!({
                "duration_seconds": duration_secs, "steps": if plan.own_turbo.is_some() { plan.turbo_points } else { 5 }, "seed": seed,
                "resolution_mode": "exact dimensions",
                "resolution_tier": "Use size slider — 32 px steps",
                "aspect_ratio": "21:9 — ultrawide landscape",
                "custom_width": width, "custom_height": height,
                "drop_adaln": true, "memory_mode": "normal",
                "attention_chunk_size": "automatic", "projection_backend": "auto",
                "short_edge": width.min(height), "sampling_method": "euler"
            }),
        ),
    );
    // 预设同时给出 config 和 turbo LoRA —— 第 1 个输出是 config，第 2 个是 loras
    // **原生 + 专用加速 LoRA 不经过预设**（Codex 复查 §21）：「标准」预设节点执行时会去加载它点名的通用 LoRA 文件
    // （WeeTodd `WeeToddH3ValidatedSamplingPreset.apply` → `WeeToddH3LoRALoader().load`），我们虽然不把它接进采样，
    // 那个文件缺了照样报错、还白读一遍。这档预设对配置只做两件事：steps = 5、sampling_method = euler ——
    // 节点 2（GenerationConfig）本来就是这两个值，所以直接用节点 2 的配置（[`finish_h3`] 把 `["3", 0]` 改指过去）。
    if plan.own_turbo.is_none() {
        m.insert(
            "3".into(),
            node(
                "WeeToddH3ValidatedSamplingPreset",
                json!({"config": ["2", 0], "preset": tier.preset()}),
            ),
        );
    }
    m.insert(
        "4".into(),
        node(
            "WeeToddH3Preflight",
            json!({
                "components": ["9", 0], "config": ["3", 0],
                "prompt_tokens": 512, "available_memory_gb": 0.0, "ffmpeg_path": ""
            }),
        ),
    );
    m
}

/// 图收尾：没有预设节点（原生 + 专用加速 LoRA）时，所有取 `["3", 0]` 配置的改取节点 2 的
fn finish_h3(mut m: Map<String, Value>) -> Value {
    if !m.contains_key("3") {
        for n in m.values_mut() {
            if let Some(inputs) = n.get_mut("inputs").and_then(Value::as_object_mut) {
                for v in inputs.values_mut() {
                    if *v == json!(["3", 0]) {
                        *v = json!(["2", 0]);
                    }
                }
            }
        }
    }
    Value::Object(m)
}

/// 采样的 LoRA 链从哪起：有专用加速 LoRA（节点 899）就从它起，否则接预设的第 2 个输出
fn lora_start(m: &Map<String, Value>) -> Value {
    if m.contains_key("899") { json!(["899", 0]) } else { json!(["3", 1]) }
}

/// 长镜头里的一个窗口。
pub struct H3Window<'a> {
    /// 这一窗口的正文（就是那一镜合成好的线路 B 提示词）
    pub prompt: &'a str,
    /// 参考图，**已经搬进 ComfyUI 的 input 目录**，这里放文件名
    pub images: &'a [String],
}

/// 一场整段出：N 个等长窗口串成一条链，**一次提交出一个 MP4**。
pub struct H3ChainParams<'a> {
    pub windows: &'a [H3Window<'a>],
    pub window_secs: f64,
    /// 重叠帧数。节点只收 5 / 22 / 39 / 56，推荐 22（≈0.92s @24fps）
    pub context_frames: u32,
    pub width: u32,
    pub height: u32,
    pub seed: u64,
    pub tier: H3Tier,
    pub filename_prefix: &'a str,
    pub loras: &'a [(String, f64)],
}

/// 把 N 个窗口串成一条 H3 latent 链。
///
/// ```text
/// ChainedTimeline ─────────────────────────────┐
///                                              ↓
/// 窗口 i:  ReferenceEncode ─→ Sample(continuation=上一窗口的尾部)
///                               ├─→ ContinuationContext ─→ 下一窗口
///                               └─→ ChainAppend ─→ chain
///                                                    ↓
///                                     DirectPublishChain ─→ 一个 MP4
/// ```
///
/// # 为什么必须在一次提交里跑完
///
/// 查遍所有碰 `WEETODD_H3_LATENTS` 的节点，**没有一个能把 latents 写盘或读回**
/// （2026-09-20 查实）。所以某一个窗口不满意只能整条重来 ——
/// 这件事要在界面上说清楚，见 [`crate::long_take`]。
///
/// # 节点编号
///
/// 窗口 i 占 `1000 + i*100` 起的一段。公共前缀是 1/2/3/4/9，
/// 单窗口那条用的 5/6/7、20+、40+ 这里都不碰 —— 免得哪天两条路合并时撞号。
pub fn build_h3_chain_graph(p: &H3ChainParams) -> Value {
    // 配置里的 `duration_seconds` 是**一个窗口**的长度，不是总长
    let mut m = h3_prefix(
        "ref2va",
        true,
        &h3_text_encoder(&H3Mode::Reference { images: &[], pixel_budget_percent: 100, audios: &[] }),
        p.width, p.height, p.window_secs, p.seed, p.tier,
    );

    // LoRA 链（和单窗口那条一样，串在预设后面）
    let mut lora_ref = lora_start(&m);
    for (i, (name, strength)) in p.loras.iter().enumerate() {
        let id = (900 + i).to_string();
        m.insert(
            id.clone(),
            node(
                "WeeToddH3LoRALoader",
                json!({
                    "lora_name": name, "strength": strength, "profile": "auto",
                    "adaln_input_grid": "", "qkv_layout": "auto",
                    "start_after_evaluations": 0, "previous_loras": lora_ref
                }),
            ),
        );
        lora_ref = json!([id, 0]);
    }

    // **时间轴。** `target_duration_seconds: 0` = 发布整条装配好的时间轴
    // （节点自己的说明：Use 0 to publish the complete assembled timeline）。
    m.insert(
        "8".into(),
        node(
            "WeeToddH3ChainedTimeline",
            json!({
                "window_duration_seconds": p.window_secs,
                "window_count": p.windows.len(),
                "context_frames": p.context_frames.to_string(),
                "target_duration_seconds": 0.0
            }),
        ),
    );

    let mut chain: Option<Value> = None;
    let mut carry: Option<Value> = None; // 上一窗口的尾部上下文
    for (i, w) in p.windows.iter().enumerate() {
        let b = 1000 + i * 100;
        let (enc, sample, ctx, app) =
            ((b + 90).to_string(), (b + 91).to_string(), (b + 92).to_string(), (b + 93).to_string());

        // 这一窗口自己的参考图链
        let mut prev_ref: Option<Value> = None;
        for (j, file) in w.images.iter().enumerate() {
            let (load, refn) = ((b + j * 2).to_string(), (b + j * 2 + 1).to_string());
            m.insert(load.clone(), node("LoadImage", json!({"image": file, "upload": "image"})));
            let mut inputs = json!({"image": [load, 0], "pixel_budget_percent": 100});
            if let Some(p0) = prev_ref {
                inputs["previous_references"] = p0;
            }
            m.insert(refn.clone(), node("WeeToddH3ReferenceImage", inputs));
            prev_ref = Some(json!([refn, 0]));
        }

        let mut enc_in = json!({
            "components": ["4", 0], "config": ["3", 0], "prompt": w.prompt
        });
        if let Some(r) = prev_ref {
            enc_in["references"] = r;
        }
        m.insert(enc.clone(), node("WeeToddH3ReferenceEncode", enc_in));

        let mut s_in = json!({
            "components": ["4", 0], "conditioning": [enc, 0], "config": ["3", 0],
            "unload_after_sample": true, "loras": lora_ref
        });
        // **第一个窗口没有上下文可继承** —— 它就是普通的一次生成
        if let Some(c) = &carry {
            s_in["continuation"] = c.clone();
        }
        m.insert(sample.clone(), node("WeeToddH3Sample", s_in));

        // 最后一个窗口不用再留尾巴
        if i + 1 < p.windows.len() {
            m.insert(
                ctx.clone(),
                node(
                    "WeeToddH3ContinuationContext",
                    json!({"latents": [sample, 0], "context_frames": p.context_frames.to_string()}),
                ),
            );
            carry = Some(json!([ctx, 0]));
        }

        let mut a_in = json!({"timeline": ["8", 0], "latents": [sample, 0]});
        if let Some(c) = &chain {
            a_in["previous_chain"] = c.clone();
        }
        m.insert(app.clone(), node("WeeToddH3ChainAppend", a_in));
        chain = Some(json!([app, 0]));
    }

    m.insert(
        "7".into(),
        node(
            "WeeToddH3DirectPublishChain",
            json!({
                "components": ["4", 0], "chain": chain.expect("窗口链是空的"),
                "filename_prefix": p.filename_prefix, "crf": 18,
                "max_av_drift_seconds": 0.025, "generation_metadata": "{}", "ffmpeg_path": ""
            }),
        ),
    );
    finish_h3(m)
}

/// 参考图的像素预算（`WeeToddH3ReferenceImage.pixel_budget_percent`，50~400）。
///
/// H3 把参考图缩到**成片面积 × 预算**，只缩不放。预算 100 时，一张 1672×941 的角色设定卡在 896×512 的成片里
/// 缩成 903×508 —— 三视图里的脸只剩 25~30 像素，细节格里最大的脸约 60 像素，服装还认得出、脸认不出
/// （2026-09-29 用户：「人物相貌跟我的角色卡不太一致」）。
///
/// 所以按**最大的那张参考图**算：够让它保持原尺寸就行（480P 下那张卡要 343%），不超过 400。
/// 小图不会被放大（只缩不放），多给的预算对它不起作用。代价是参考图的像素每一步采样都要算，出片会慢一些。
pub fn reference_pixel_budget(ref_sizes: &[(u32, u32)], width: u32, height: u32) -> u32 {
    let out = (width.max(1) as f64) * (height.max(1) as f64);
    let largest = ref_sizes.iter().map(|(w, h)| *w as f64 * *h as f64).fold(0.0, f64::max);
    ((largest / out * 100.0).ceil() as u32).clamp(100, 400)
}

pub fn build_h3_graph(p: &H3Params) -> Value {
    let task = match &p.mode {
        H3Mode::Text => "t2va",
        H3Mode::Frames { .. } => "fl2va",
        H3Mode::Reference { .. } => "ref2va",
    };
    let mut m = h3_prefix(
        task,
        matches!(p.mode, H3Mode::Reference { .. }),
        &h3_text_encoder(&p.mode),
        p.width, p.height, p.duration_secs, p.seed, p.tier,
    );
    // 条件那一段按模式换。节点 5 永远是「产 conditioning 的那个」，
    // 采样器只认 `["5", 0]`，所以三种模式对采样链是透明的。
    match &p.mode {
        H3Mode::Text => {
            m.insert(
                "5".into(),
                node(
                    "WeeToddH3TextEncode",
                    json!({
                        "components": ["4", 0], "prompt": p.prompt,
                        "unload_after_encode": true, "config": ["3", 0]
                    }),
                ),
            );
        }
        H3Mode::Frames { first, last } => {
            // 三个条件节点各管一种组合。**给了图却挑错节点不报错** ——
            // 那张图就是单纯不进条件，出来是一段和它无关的视频。
            let mut inputs = Map::new();
            if let Some(f) = first {
                m.insert("10".into(), node("LoadImage", json!({"image": f, "upload": "image"})));
                inputs.insert("first_frame".into(), json!(["10", 0]));
            }
            if let Some(l) = last {
                m.insert("12".into(), node("LoadImage", json!({"image": l, "upload": "image"})));
                inputs.insert("last_frame".into(), json!(["12", 0]));
            }
            let kind = match (first, last) {
                (Some(_), Some(_)) => "WeeToddH3FirstLastFrame",
                (Some(_), None) => "WeeToddH3FirstFrame",
                (None, Some(_)) => "WeeToddH3LastFrame",
                // 两个都没有不该走到这个分支 —— 调用方该给 Text
                (None, None) => "WeeToddH3FirstFrame",
            };
            m.insert("11".into(), node(kind, Value::Object(inputs)));
            m.insert(
                "5".into(),
                node(
                    "WeeToddH3KeyframeEncode",
                    json!({
                        "components": ["4", 0], "config": ["3", 0],
                        "keyframes": ["11", 0], "prompt": p.prompt
                    }),
                ),
            );
        }
        H3Mode::Reference {
            images,
            pixel_budget_percent,
            audios,
        } => {
            // 参考图逐张串起来：每个 ReferenceImage 吃上一个的 references。
            // 空列表时退回纯文本，否则 ReferenceEncode 会拿到空引用。
            if images.is_empty() {
                m.insert(
                    "5".into(),
                    node(
                        "WeeToddH3TextEncode",
                        json!({
                            "components": ["4", 0], "prompt": p.prompt,
                            "unload_after_encode": true, "config": ["3", 0]
                        }),
                    ),
                );
            } else {
                let mut prev: Option<Value> = None;
                for (i, file) in images.iter().enumerate() {
                    let load = (20 + i * 2).to_string();
                    let refn = (21 + i * 2).to_string();
                    m.insert(
                        load.clone(),
                        node("LoadImage", json!({"image": file, "upload": "image"})),
                    );
                    let mut inputs = json!({
                        "image": [load, 0],
                        "pixel_budget_percent": pixel_budget_percent
                    });
                    if let Some(p0) = prev {
                        inputs["previous_references"] = p0;
                    }
                    m.insert(refn.clone(), node("WeeToddH3ReferenceImage", inputs));
                    prev = Some(json!([refn, 0]));
                }
                // **接上一镜：视频参考串在图片参考之后。**
                // 顺序决定编号 —— 图片是 <Picture 1..N>，视频是 <Video 1>。
                let refs = match &p.continuity {
                    Some(c) => {
                        m.insert(
                            "40".into(),
                            node(
                                "VHS_LoadVideoFFmpegPath",
                                json!({
                                    "video": c.video_path,
                                    "force_rate": c.fps,
                                    "custom_width": 0, "custom_height": 0,
                                    "frame_load_cap": c.frames,
                                    "start_time": c.start_secs,
                                }),
                            ),
                        );
                        let mut vin = json!({
                            "video_frames": ["40", 0],
                            "fps": c.fps,
                            // 另一档 native canvas 上游写明「dramatically slower」
                            "video_size": "match output (recommended)",
                            "temporal_density": "all frames (recommended)",
                        });
                        if let Some(p0) = &prev {
                            vin["previous_references"] = p0.clone();
                        }
                        m.insert("41".into(), node("WeeToddH3ReferenceVideo", vin));
                        json!(["41", 0])
                    }
                    None => prev.clone().unwrap(),
                };
                // **音频参考串在最后**（`<Audio 1..>`）。`LoadAudio` 读 ComfyUI `input/` 下的文件
                let mut refs = refs;
                for (i, file) in audios.iter().take(H3_MAX_REFERENCE_AUDIOS).enumerate() {
                    let load = (60 + i * 2).to_string();
                    let refn = (61 + i * 2).to_string();
                    m.insert(load.clone(), node("LoadAudio", json!({"audio": file})));
                    m.insert(refn.clone(), node("WeeToddH3ReferenceAudio", json!({"audio": [load, 0], "previous_references": refs})));
                    refs = json!([refn, 0]);
                }
                m.insert(
                    "5".into(),
                    node(
                        "WeeToddH3ReferenceEncode",
                        json!({
                            "components": ["4", 0], "config": ["3", 0],
                            "references": refs, "prompt": p.prompt
                        }),
                    ),
                );
            }
        }
    }

    // 额外 LoRA 串在预设那条链后面。`previous_loras` 就是给串联用的。
    let mut lora_ref = lora_start(&m);
    for (i, (name, strength)) in p.loras.iter().enumerate() {
        let id = (900 + i).to_string();
        m.insert(
            id.clone(),
            node(
                "WeeToddH3LoRALoader",
                json!({
                    "lora_name": name, "strength": strength, "profile": "auto",
                    "adaln_input_grid": "", "qkv_layout": "auto",
                    "start_after_evaluations": 0, "previous_loras": lora_ref
                }),
            ),
        );
        lora_ref = json!([id, 0]);
    }

    m.insert(
        "6".into(),
        node(
            "WeeToddH3Sample",
            json!({
                "components": ["4", 0], "conditioning": ["5", 0], "config": ["3", 0],
                "unload_after_sample": true, "loras": lora_ref
            }),
        ),
    );
    m.insert(
        "7".into(),
        node(
            "WeeToddH3DirectPublishLatents",
            json!({
                "components": ["4", 0], "latents": ["6", 0],
                "filename_prefix": p.filename_prefix, "crf": 18,
                "max_av_drift_seconds": 0.025, "generation_metadata": "{}",
                "sampling_info": ["6", 1], "ffmpeg_path": ""
            }),
        ),
    );
    finish_h3(m)
}

/// **远景修脸**（FaceRefine 搬到 MLX，2026-10-08 用户：「做 B，把 FaceRefine 搬到 MLX」）。
///
/// ```text
/// H3FaceSelect(读视频、选人) ─▶ H3FaceTrackCrop(画布 768，脸占四成)
///   ─▶ AnimaH3EncodeVideoLatents(缩到 512 编码 + 原声) ─▶ WeeToddH3LatentHiresFix(1.5x → 768，低强度重绘)
///   ─▶ WeeToddH3VideoVAEDecode ─▶ H3FaceStitch(贴回原片) ─▶ VHS_VideoCombine(带原声)
/// 身份：identity 图（没给就用裁出来的第一帧）→ ReferenceImage → ReferenceEncode
/// ```
///
/// 选人 / 跟踪 / 裁切 / 贴回是 Carasibana 的 ComfyUI-H3-FaceRefine（纯图像处理，CPU）；中间重绘是 WeeTodd 的 MLX H3，
/// 只多一个我们自己的节点（`comfyui-nodes/anima_h3_face_refine`）把画面编码成 latent —— 原版那两个重绘节点要原生 PyTorch H3。
/// 主干 / 加速 LoRA 走和出片同一份前缀（[`h3_prefix`]），兼容模式 Ref2VA。为什么是 512 → 768、和原版差在哪，见那个节点的模块注释。
pub struct FaceRefineParams<'a> {
    /// 原片的绝对路径
    pub video_path: &'a str,
    /// 身份参考图（ComfyUI `input/` 下的文件名）。`None` = 用裁出来的第一帧
    pub identity_image: Option<&'a str>,
    /// 重绘提示词（Ref2VA 写法，`<Picture 1>` 是这个人）
    pub prompt: &'a str,
    /// 重绘强度（0.05~1）。原版推荐 0.40 起；H3 shift 12，小数值也比看上去狠
    pub strength: f64,
    /// 选哪张脸：`largest_face` / `smallest_face` / `left_most`
    pub select: &'a str,
    pub seed: u64,
    pub tier: H3Tier,
    pub filename_prefix: &'a str,
}

/// 修脸的裁切画布。1.5 倍要正好对上（512 × 1.5 = 768），见 `anima_h3_face_refine.source_size_for`
pub const FACE_REFINE_CANVAS: u32 = 768;
/// 原版推荐的基础重绘强度
pub const FACE_REFINE_DEFAULT_STRENGTH: f64 = 0.40;
/// 默认的修脸提示词（Ref2VA 写法）。**只说「照原样、把脸画清楚」**，不描述长相 —— 长相由身份图和源画面定，
/// 写多了低强度重绘反而会往文字上拽
pub const FACE_REFINE_DEFAULT_PROMPT: &str = "subject_definitions:\n<Subject 1> is the person whose face fills this close-up, as shown in <Picture 1>.\n\n\
summary: A close-up of <Subject 1>'s face, refined for facial detail.\n\n\
retention_analysis:\n<Picture 1>: reference - keep <Subject 1>'s identity, face shape, hairstyle and skin tone exactly.\n\n\
detailed_description: The same close-up of <Subject 1> with exactly the same head motion, expression, mouth movements, gaze, lighting and framing as the source; \
only the facial features are rendered sharper and anatomically correct: clear eyes with natural catchlights, well-defined nose and lips, natural skin texture.\n\n\
overall_soundscape: Unchanged from the source.\n\nnon_diegetic_music: None.";

pub fn build_face_refine_graph(p: &FaceRefineParams) -> Value {
    let canvas = FACE_REFINE_CANVAS;
    // 配置里的尺寸 / 时长是占位：编码节点按真实帧数和 512 改写（WeeToddH3LatentHiresFix 只认源 latent 带的那份）
    let plan = H3SamplingPlan::current(true, p.tier);
    let mut m = h3_prefix_with("ref2va", &h3_text_encoder(&H3Mode::Reference { images: &[], pixel_budget_percent: 100, audios: &[] }),
        canvas * 2 / 3, canvas * 2 / 3, 5.0, p.seed, plan, true);
    let detector = "face_yolov8m.pt";
    m.insert("30".into(), node("H3FaceSelect", json!({
        "video": p.video_path, "detector": detector, "confidence": 0.35,
        "select": p.select, "select_index": 0, "confirmed_pick": ""
    })));
    m.insert("31".into(), node("H3FaceTrackCrop", json!({
        "images": ["30", 0], "detector": detector, "confidence": 0.35, "crop_factor": 2.5,
        "canvas_width": canvas, "canvas_height": canvas, "canvas_mode": "manual",
        "smooth_window": 21, "size_smooth_window": 51, "smooth_method": "gaussian", "size_mode": "max_of_clip",
        "face_pick": ["30", 2]
    })));
    m.insert("32".into(), node("AnimaH3EncodeVideoLatents", json!({
        "components": ["4", 0], "config": ["3", 0], "images": ["31", 0],
        "canvas_width": canvas, "canvas_height": canvas, "audio": ["30", 1]
    })));
    // 身份参考：给了图用图，没给用裁出来的第一帧（这个人自己的脸）
    let identity = match p.identity_image {
        Some(f) => {
            m.insert("20".into(), node("LoadImage", json!({"image": f, "upload": "image"})));
            json!(["20", 0])
        }
        None => {
            m.insert("20".into(), node("ImageFromBatch", json!({"image": ["31", 0], "batch_index": 0, "length": 1})));
            json!(["20", 0])
        }
    };
    m.insert("21".into(), node("WeeToddH3ReferenceImage", json!({"image": identity, "pixel_budget_percent": 100})));
    m.insert("5".into(), node("WeeToddH3ReferenceEncode", json!({
        "components": ["4", 0], "config": ["3", 0], "references": ["21", 0], "prompt": p.prompt
    })));
    // 采样点：挂了自己的加速 LoRA 就按它的点数（Larry 6 次 = 7 点），否则预设的 5 点
    let points = if plan.own_turbo.is_some() { plan.turbo_points } else { 5 };
    m.insert("33".into(), node("WeeToddH3LatentHiresFix", json!({
        "components": ["4", 0], "conditioning": ["5", 0], "source_latents": ["32", 0],
        "scale": "1.5x — balanced", "refinement_schedule_points": points,
        "refinement_strength": p.strength.clamp(0.05, 1.0), "unload_after_refine": true,
        "loras": lora_start(&m)
    })));
    m.insert("34".into(), node("WeeToddH3VideoVAEDecode", json!({"components": ["4", 0], "latents": ["33", 0], "unload_after_decode": true})));
    m.insert("35".into(), node("H3FaceStitch", json!({
        "base_images": ["30", 0], "refined_crops": ["34", 0], "transform": ["31", 1],
        "paste_region": "face_only", "mask_dilation": 16, "feather": 6, "colour_match": 1.0, "blend": 1.0,
        "undetected_frames": "fade_out"
    })));
    m.insert("36".into(), node("VHS_VideoCombine", json!({
        "images": ["35", 0], "audio": ["30", 1], "frame_rate": ["30", 6], "loop_count": 0,
        "filename_prefix": p.filename_prefix, "format": "video/h264-mp4", "pingpong": false, "save_output": true
    })));
    finish_h3(m)
}

/// 远景修脸实跑：导出图（`FACE_OUT`），给了 `FACE_SUBMIT=1` 就直接交给 ComfyUI。
/// **要先把 `comfyui-nodes/anima_h3_face_refine` 软链进 ComfyUI 的 custom_nodes 并重启 ComfyUI**（新节点重启才加载）。
///
/// ```bash
/// FACE_VIDEO=/abs/shot.mp4 FACE_IDENTITY=bai.png FACE_STRENGTH=0.4 cargo test --lib 导出修脸图供实跑 -- --ignored --nocapture
/// ```
#[cfg(test)]
#[test]
#[ignore = "要本机 ComfyUI；用 cargo test -- --ignored 手动跑"]
fn 导出修脸图供实跑() {
    let video = std::env::var("FACE_VIDEO").expect("FACE_VIDEO=原片绝对路径");
    let identity = std::env::var("FACE_IDENTITY").ok();
    let prompt = std::env::var("FACE_PROMPT").unwrap_or_else(|_| FACE_REFINE_DEFAULT_PROMPT.into());
    let g = build_face_refine_graph(&FaceRefineParams {
        video_path: &video,
        identity_image: identity.as_deref(),
        prompt: &prompt,
        strength: std::env::var("FACE_STRENGTH").ok().and_then(|v| v.parse().ok()).unwrap_or(FACE_REFINE_DEFAULT_STRENGTH),
        select: &std::env::var("FACE_SELECT").unwrap_or_else(|_| "largest_face".into()),
        seed: 20261008,
        tier: H3Tier::Staged,
        filename_prefix: "Anima/face_refine",
    });
    let out = std::env::var("FACE_OUT").unwrap_or_else(|_| "/tmp/face_refine.json".into());
    std::fs::write(&out, serde_json::to_string_pretty(&g).unwrap()).unwrap();
    println!("图写到 {out}");
    if std::env::var("FACE_SUBMIT").as_deref() == Ok("1") {
        let v: Value = tokio::runtime::Runtime::new().unwrap().block_on(async {
            reqwest::Client::new().post("http://127.0.0.1:8188/prompt").json(&json!({"prompt": g})).send().await.unwrap().json().await.unwrap()
        });
        println!("ComfyUI 回：{v}");
    }
}

/// 把**窗口链的真实图**导出来，给手动实跑用。
///
/// 结构测试只能证明「我们按自己的假设接对了线」。这条链一次都没真提交过，
/// 而这个项目最典型的失败正是「结构测试全绿、真跑才炸」
/// （`build_h3_graph` 的 task 分区就是这么发现写错的）。
///
/// ```bash
/// H3_CHAIN_OUT=/tmp/chain.json cargo test --lib 导出窗口链的图供实跑 -- --ignored
/// ```
#[cfg(test)]
#[test]
#[ignore = "要本机 ComfyUI；用 cargo test -- --ignored 手动跑"]
fn 导出窗口链的图供实跑() {
    let prompts: Vec<String> = std::env::var("H3_CHAIN_PROMPTS")
        .map(|v| v.split("\u{1}").map(str::to_string).collect())
        .unwrap_or_else(|_| vec!["w1".into(), "w2".into(), "w3".into()]);
    let imgs: Vec<String> = std::env::var("H3_CHAIN_IMAGE").map(|v| vec![v]).unwrap_or_default();
    let ws: Vec<H3Window> = prompts.iter().map(|p| H3Window { prompt: p, images: &imgs }).collect();
    let g = build_h3_chain_graph(&H3ChainParams {
        windows: &ws,
        window_secs: std::env::var("H3_CHAIN_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(3.0),
        context_frames: std::env::var("H3_CHAIN_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(22),
        width: 576,
        height: 1024,
        seed: 880033,
        tier: H3Tier::Standard,
        filename_prefix: "Anima/chain_smoke",
        loras: &[],
    });
    std::fs::write(
        std::env::var("H3_CHAIN_OUT").unwrap_or_else(|_| "/tmp/chain.json".into()),
        serde_json::to_string_pretty(&g).unwrap(),
    )
    .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 设定卡在 480P 下要 343% 才不被缩（09-29：按 100 缩下去脸只剩几十像素，相貌对不上）；
    /// 720P 下 168%；小图不需要加预算；封顶 400
    /// **原生 Ref2VA 装齐了就用，没装退回 FL2VA 兼容模式**（2026-10-06）。空目录、下载一半不算装了
    /// 2026-10-07：原生 Ref2VA 一律稠密 19 次、不挂通用加速 LoRA；兼容模式和非参考模式照旧
    #[test]
    fn 原生ref2va不挂通用加速lora() {
        for t in [H3Tier::Preview, H3Tier::Standard, H3Tier::Staged, H3Tier::Dense] {
            let native = H3SamplingPlan::new(true, t, true);
            assert!(native.native_ref2va);
            assert_eq!(native.tier, H3Tier::Dense, "原生要 {t:?} 也得改稠密");
            assert_eq!(native.tier.turbo_lora(), None);
            assert_eq!(native.tier.evaluations(), 19);
            let compat = H3SamplingPlan::new(true, t, false);
            assert_eq!(compat.tier, t, "兼容模式不动");
            let frames = H3SamplingPlan::new(false, t, true);
            assert!(!frames.native_ref2va, "首帧 / 文生不走 Ref2VA");
            assert_eq!(frames.tier, t);
        }
        assert!(H3SamplingPlan::new(true, H3Tier::Standard, true).describe().contains("改稠密"), "界面 / 日志要说清楚换了档");
        assert!(H3SamplingPlan::new(true, H3Tier::Standard, false).describe().contains(H3_LARRY_TURBO_LORA), "兼容模式标准档挂 Larry 完整版");
    }

    /// 2026-10-07 用户：稠密「无实用性」—— 原生装了专用 LoRA 就默认挂它；没装才稠密
    #[test]
    fn 原生ref2va默认挂专用加速lora() {
        let native = H3SamplingPlan::new(true, H3Tier::Standard, true);
        assert_eq!(native.pick_turbo(true, false).turbo(), Some(H3_REF2VA_TURBO_LORA));
        assert_eq!(native.pick_turbo(false, false).tier, H3Tier::Dense, "没装专用 LoRA：稠密");
        assert_eq!(native.pick_turbo(true, true).tier, H3Tier::Dense, "强制稠密");
        let compat = H3SamplingPlan::new(true, H3Tier::Standard, false);
        assert_eq!(compat.pick_turbo(true, false).turbo(), Some(H3_LARRY_TURBO_LORA), "兼容模式不换成 Ref2VA 专用的");
    }

    /// 真文件过得了检查（`cargo test --lib 专用加速lora真文件 -- --ignored`）
    #[test]
    #[ignore = "读本机 ComfyUI 的 LoRA"]
    fn 专用加速lora真文件() {
        let p = std::path::Path::new(&std::env::var("HOME").unwrap()).join("AI/ComfyUI/models/loras").join(H3_REF2VA_TURBO_LORA);
        assert_eq!(ref2va_turbo_check(&p), Ok(()));
        let generic = p.with_file_name(H3_GENERIC_TURBO_LORA);
        assert!(ref2va_turbo_check(&generic).is_err(), "通用那个不该过");
    }

    /// 2026-10-07 用户定：LightX2V 4 步加进链路当一档，双采保留
    #[test]
    fn lightx2v档不经过预设_强度0_0625() {
        let plan = H3SamplingPlan::new(false, H3Tier::LightX2V, false);
        assert_eq!((plan.own_turbo, plan.turbo_strength, plan.turbo_points), (Some(H3_LIGHTX2V_TURBO_LORA), H3_LIGHTX2V_STRENGTH, 5));
        assert_eq!(plan.evaluations(), 4);
        let enc = h3_text_encoder(&H3Mode::Frames { first: Some("a.png"), last: None });
        let m = h3_prefix_with("fl2va", &enc, 1312, 736, 10.0, 1, plan, false);
        assert!(!m.contains_key("3"), "不经过预设");
        assert_eq!(m["899"]["inputs"]["lora_name"], H3_LIGHTX2V_TURBO_LORA);
        assert_eq!(m["899"]["inputs"]["strength"], H3_LIGHTX2V_STRENGTH);
        assert_eq!(m["2"]["inputs"]["steps"], 5);
        // 双采那档换成 Larry 完整版 6 次求值（7 点），同样不经过预设
        let staged = H3SamplingPlan::new(false, H3Tier::Staged, false);
        assert_eq!((staged.own_turbo, staged.turbo_points), (Some(H3_LARRY_TURBO_LORA), 7));
        assert!(!h3_prefix_with("fl2va", &enc, 1312, 736, 10.0, 1, staged, false).contains_key("3"));
        // 原生 Ref2VA 选了 LightX2V 档：照旧原生那套（专用 Ref2VA LoRA / 稠密），不挂首尾帧的 LoRA
        let native = H3SamplingPlan::new(true, H3Tier::LightX2V, true);
        assert_eq!((native.tier, native.own_turbo), (H3Tier::Dense, None));
        assert_eq!(native.pick_turbo(true, false).own_turbo, Some(H3_REF2VA_TURBO_LORA));
    }

    /// 加速 LoRA 对照：首帧那条换 LoRA、强度、采样点；原生 Ref2VA 那条不受影响
    #[test]
    fn 加速lora对照的图() {
        let plan = H3SamplingPlan::new(false, H3Tier::Staged, false).with_trial_turbo("h3-candidates/x8.safetensors", 1.0, 9);
        assert_eq!(plan.evaluations(), 8);
        let enc = h3_text_encoder(&H3Mode::Frames { first: Some("a.png"), last: None });
        let m = h3_prefix_with("fl2va", &enc, 1312, 736, 5.0, 1, plan, false);
        assert!(!m.contains_key("3"), "对照不经过预设");
        assert_eq!(m["2"]["inputs"]["steps"], 9);
        assert_eq!(m["899"]["inputs"]["lora_name"], "h3-candidates/x8.safetensors");
        let v01 = H3SamplingPlan::new(false, H3Tier::Staged, false).with_trial_turbo("v01", 0.0625, 5);
        assert_eq!(h3_prefix_with("fl2va", &enc, 1312, 736, 5.0, 1, v01, false)["899"]["inputs"]["strength"], 0.0625);
        let native = H3SamplingPlan::new(true, H3Tier::Standard, true).pick_turbo(true, false);
        assert_eq!(native.with_trial_turbo("v01", 0.0625, 5).own_turbo, Some(H3_REF2VA_TURBO_LORA), "原生那条不换");
        // 正式链路：双采那档 = Larry 7 点；稠密照旧走预设、节点 2 的 steps 不动
        let m = h3_prefix_with("fl2va", &enc, 1312, 736, 5.0, 1, H3SamplingPlan::new(false, H3Tier::Staged, false), false);
        assert_eq!(m["2"]["inputs"]["steps"], 7);
        let m = h3_prefix_with("fl2va", &enc, 1312, 736, 5.0, 1, H3SamplingPlan::new(false, H3Tier::Dense, false), false);
        assert_eq!(m["2"]["inputs"]["steps"], 5);
        assert!(m.contains_key("3"));
    }

    /// Codex 复查 §21：原生 + 专用加速 LoRA 的图里**没有预设节点**，配置取节点 2（steps 5 / euler）
    #[test]
    fn 原生专用加速不经过通用预设() {
        let plan = H3SamplingPlan::new(true, H3Tier::Standard, true).pick_turbo(true, false);
        let enc = h3_text_encoder(&H3Mode::Reference { images: &[], pixel_budget_percent: 100, audios: &[] });
        let m = h3_prefix_with("ref2va", &enc, 1312, 736, 5.0, 1, plan, true);
        assert!(!m.contains_key("3"), "还有预设节点：它会去加载通用 LoRA 文件");
        assert_eq!(m["2"]["inputs"]["steps"], 5);
        assert_eq!(m["2"]["inputs"]["sampling_method"], "euler");
        let g = finish_h3(m);
        assert!(!g.to_string().contains(r#"["3",0]"#), "还有节点取预设的配置");
        assert_eq!(g["4"]["inputs"]["config"], json!(["2", 0]));
        // 兼容模式标准档：Larry 完整版，也不经过预设；稠密照旧有预设
        let compat = H3SamplingPlan::new(true, H3Tier::Standard, false);
        let m = h3_prefix_with("ref2va", &enc, 1312, 736, 5.0, 1, compat, true);
        assert_eq!(m["899"]["inputs"]["lora_name"], H3_LARRY_TURBO_LORA);
        let dense = H3SamplingPlan::new(true, H3Tier::Dense, false);
        let m = h3_prefix_with("ref2va", &enc, 1312, 736, 5.0, 1, dense, true);
        assert!(m.contains_key("3"));
        assert_eq!(finish_h3(m)["4"]["inputs"]["config"], json!(["3", 0]));
    }

    #[test]
    fn 专用加速lora的头要对得上() {
        let ok = json!({"__metadata__": {"source_format": "Diffusers PEFT LoRA", "training_rank": "128", "training_alpha": "8.0"},
                        "diffusion_model.blocks.0.attn.qkv_proj.lora_A.weight": {}});
        assert!(ref2va_turbo_header_check(&ok).is_ok());
        let mut partial = ok.clone();
        partial["__metadata__"]["partial_conversion"] = json!("true");
        assert!(ref2va_turbo_header_check(&partial).is_err());
        let mut adaln = ok.clone();
        adaln["diffusion_model.blocks.0.adaln_proj.linear.lora_A.weight"] = json!({});
        assert!(ref2va_turbo_header_check(&adaln).is_err());
        let mut refiner = ok.clone();
        refiner["diffusion_model.token_refiner.blocks.0.attn.out_proj.alpha"] = json!({});
        assert!(ref2va_turbo_header_check(&refiner).is_ok(), "真文件里有 token_refiner 的张量");
        let mut other = ok.clone();
        other["lora_unet_input_blocks_4_1.alpha"] = json!({});
        assert!(ref2va_turbo_header_check(&other).is_err(), "SDXL 命名混进来");
        let generic = json!({"__metadata__": {"base_model": "MiniMax-H3", "partial_conversion": "true"}, "diffusion_model.blocks.0.x": {}});
        assert!(ref2va_turbo_header_check(&generic).is_err(), "通用那个（drbaph v4）不能当专用的");
    }

    /// 官方 Ref2VA 4 步：参考图按成片画布面积（match = 100）；兼容模式照旧按设定卡算
    #[test]
    fn 原生专用加速时参考图按官方match() {
        let native = H3SamplingPlan::new(true, H3Tier::Standard, true).pick_turbo(true, false);
        assert_eq!(native.reference_budget(163), 100);
        let compat = H3SamplingPlan::new(true, H3Tier::Standard, false);
        assert_eq!(compat.reference_budget(163), 163);
        let dense = H3SamplingPlan::new(true, H3Tier::Standard, true).pick_turbo(false, false);
        assert_eq!(dense.reference_budget(163), 163);
    }

    /// 对照用：原生 Ref2VA + Ref2VA 专用加速 LoRA —— 采样只挂专用那个，不接预设里的通用 LoRA
    #[test]
    fn 原生ref2va改挂专用加速lora() {
        let plan = H3SamplingPlan::new(true, H3Tier::Dense, true).with_ref2va_turbo();
        assert_eq!(plan.turbo(), Some(H3_REF2VA_TURBO_LORA));
        assert_eq!(plan.tier.evaluations(), 4);
        let enc = h3_text_encoder(&H3Mode::Reference { images: &[], pixel_budget_percent: 100, audios: &[] });
        let m = h3_prefix_with("ref2va", &enc, 1312, 736, 5.0, 1, plan, true);
        assert_eq!(m["899"]["inputs"]["lora_name"], H3_REF2VA_TURBO_LORA);
        assert!(m["899"]["inputs"].get("previous_loras").is_none(), "不能串在预设的通用 LoRA 后面");
        assert_eq!(lora_start(&m), json!(["899", 0]));
        // 兼容模式不受影响
        let compat = H3SamplingPlan::new(true, H3Tier::Standard, false).with_ref2va_turbo();
        assert_eq!(compat.turbo(), Some(H3_LARRY_TURBO_LORA), "兼容模式不换成 Ref2VA 专用的");
        let m = h3_prefix_with("ref2va", &enc, 1312, 736, 5.0, 1, compat, true);
        assert_eq!(m["899"]["inputs"]["lora_name"], H3_LARRY_TURBO_LORA, "兼容模式挂的是 Larry，不是 Ref2VA 专用的");
        // 整张图：采样器取的是 899（测试里当没装原生，标准档 = Larry）
        let g = build_h3_graph(&h3(H3Tier::Standard, &[]));
        assert_eq!(g["6"]["inputs"]["loras"], json!(["899", 0]));
        assert_eq!(g["899"]["inputs"]["lora_name"], H3_LARRY_TURBO_LORA);
    }

    /// 图里真的没挂：原生 Ref2VA 的预设是稠密那一档（WeeTodd 的稠密预设不带 LoRA）
    #[test]
    fn 原生ref2va的图用稠密预设() {
        let enc = h3_text_encoder(&H3Mode::Reference { images: &[], pixel_budget_percent: 100, audios: &[] });
        let m = h3_prefix_with("ref2va", &enc, 1312, 736, 5.0, 1, H3SamplingPlan::new(true, H3Tier::Standard, true), true);
        assert_eq!(m["3"]["inputs"]["preset"], H3Tier::Dense.preset());
        assert_eq!(m["1"]["inputs"]["transformer"], H3_REF2VA_TRANSFORMER);
        let m = h3_prefix_with("ref2va", &enc, 1312, 736, 5.0, 1, H3SamplingPlan::new(true, H3Tier::Standard, false), true);
        assert_eq!((m["899"]["inputs"]["lora_name"].clone(), m["2"]["inputs"]["steps"].clone()), (json!(H3_LARRY_TURBO_LORA), json!(5)), "兼容模式标准档：Larry 4 次");
        assert!(H3Tier::Standard.preset().contains("drbaph v4 step-600") && !H3Tier::Dense.preset().contains("Turbo"));
    }

    /// 2026-10-06 用户定：原生 Ref2VA 做成开关、默认兼容模式。装了但没打开 = 兼容模式
    #[test]
    fn 原生ref2va要装了而且打开才用() {
        assert!(!use_native_ref2va(true, false), "装了但设置没开：兼容模式（10-06 之前这里是装了就用）");
        assert!(!use_native_ref2va(false, true), "开了但没装：只能兼容模式");
        assert!(use_native_ref2va(true, true));
        assert!(!crate::config::AppConfig::default().h3_native_ref2va, "默认关");
        let old: crate::config::AppConfig = serde_json::from_value(serde_json::to_value(crate::config::AppConfig::default()).unwrap()
            .as_object().unwrap().iter().filter(|(k, _)| *k != "h3_native_ref2va").map(|(k, v)| (k.clone(), v.clone())).collect::<serde_json::Map<_, _>>().into()).unwrap();
        assert!(!old.h3_native_ref2va, "老配置里没有这一项：当关");
    }

    #[test]
    fn 原生ref2va和兼容模式的加载输入() {
        let native = h3_loader("ref2va", true, "MiniMax-H3/FL2VA/text_encoder", true);
        assert_eq!(native["checkpoint"], H3_REF2VA_SUPPORT);
        assert_eq!(native["transformer"], H3_REF2VA_TRANSFORMER);
        assert_eq!(native["allow_fl2va_weights_for_ref2va"], false);
        assert_eq!(native["audio_vae"], format!("{H3_REF2VA_SUPPORT}/audio_vae"));
        assert_eq!(native["text_encoder"], "MiniMax-H3/FL2VA/text_encoder", "文本编码器两个分区共用");
        let fallback = h3_loader("ref2va", true, "MiniMax-H3/FL2VA/text_encoder", false);
        assert_eq!(fallback["checkpoint"], H3_CHECKPOINT);
        assert_eq!(fallback["allow_fl2va_weights_for_ref2va"], true);
        // 首帧 / 文生不受影响
        assert_eq!(h3_loader("fl2va", false, "x", false)["allow_fl2va_weights_for_ref2va"], false);

        let d = tempfile::tempdir().unwrap();
        assert!(!h3_ref2va_native_in(d.path()));
        let t = d.path().join(H3_REF2VA_TRANSFORMER);
        let sup = d.path().join(H3_REF2VA_SUPPORT);
        std::fs::create_dir_all(t.join("pages")).unwrap();
        std::fs::create_dir_all(sup.join("audio_vae")).unwrap();
        std::fs::write(t.join("paged_manifest.json"), "{}").unwrap();
        std::fs::write(sup.join("model_index.json"), "{}").unwrap();
        std::fs::write(sup.join("audio_vae/model.safetensors"), "x").unwrap();
        assert!(!h3_ref2va_native_in(d.path()), "pages 里没有权重（下载中断）不算装了");
        std::fs::write(t.join("pages/block-000.safetensors"), "x").unwrap();
        assert!(h3_ref2va_native_in(d.path()));
    }

    #[test]
    fn 参考图预算够让最大那张保持原尺寸() {
        assert_eq!(reference_pixel_budget(&[(1672, 941), (1344, 756)], 896, 512), 343);
        assert_eq!(reference_pixel_budget(&[(1672, 941)], 1280, 736), 168);
        assert_eq!(reference_pixel_budget(&[(512, 512)], 896, 512), 100, "小图不加预算");
        assert_eq!(reference_pixel_budget(&[(4096, 4096)], 896, 512), 400, "封顶 400");
        assert_eq!(reference_pixel_budget(&[], 896, 512), 100);
        let src = include_str!("video_tool.rs");
        assert!(src.contains("pixel_budget_percent: budget"), "出片没按参考图尺寸给预算");
    }

    fn p<'a>(dur: f64, loras: &'a [(String, f64)], wins: u32) -> WeeToddParams<'a> {
        WeeToddParams {
            first_frame: None,
            last_frame: None,
            prompt: "一位女理发师停下剪刀",
            width: 768,
            height: 512,
            duration_secs: dur,
            fps: 24.0,
            seed: 584_293_325,
            filename_prefix: "Anima/WT",
            loras,
            window_count: wins,
            window_prompts: &[],
        }
    }

    /// **`run_weetodd` 会怎么建图，就导出什么** —— 分辨率决定 H3 档次那条规则
    /// 只写在 `video_tool.rs` 里，这里复刻一遍会测成两份。所以直接调它用的两个
    /// 构造函数，参数按 `run_weetodd` 的算法给。
    ///
    /// `ANIMA_GRAPH_DUMP=/x.json ANIMA_ROUTE=h3|ltx ANIMA_W=640 ANIMA_H=384 \
    ///   cargo test --lib dump_route_graph -- --ignored`
    /// 短剧那条线的端到端：关键帧 → 视频文件。**打真 ComfyUI。**
    ///
    /// `ANIMA_CINEMA_SRC=<图> ANIMA_CINEMA_OUT=<mp4> ANIMA_CINEMA_ENGINE=ltx|h3 \
    ///   cargo test --lib cinema_render_e2e -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn cinema_render_e2e() {
        let src = std::env::var("ANIMA_CINEMA_SRC").expect("要设 ANIMA_CINEMA_SRC");
        let out = std::env::var("ANIMA_CINEMA_OUT").expect("要设 ANIMA_CINEMA_OUT");
        let engine = std::env::var("ANIMA_CINEMA_ENGINE").unwrap_or_else(|_| "ltx".into());
        let prompt = std::env::var("ANIMA_PROMPT")
            .unwrap_or_else(|_| "老茶师放下铜壶，抬眼看向对座的年轻人".into());
        let r = crate::agent_runtime::video_tool::render_shot_video(
            std::path::Path::new(&src),
            std::path::Path::new(&out),
            &engine,
            &prompt,
            5.0, 768, 512, 24.0, 20_260_904, true, None, None,
        )
        .await;
        assert!(r.is_ok(), "{r:?}");
        let n = std::fs::metadata(&out).unwrap().len();
        assert!(n > 100_000, "文件太小，可能是空的：{n} 字节");
        println!("  ✓ {out}  {:.2} MB", n as f64 / 1e6);
    }

    #[test]
    #[ignore]
    fn dump_route_graph() {
        let out = std::env::var("ANIMA_GRAPH_DUMP").expect("要设 ANIMA_GRAPH_DUMP");
        let w: u32 = std::env::var("ANIMA_W").ok().and_then(|v| v.parse().ok()).unwrap_or(768);
        let h: u32 = std::env::var("ANIMA_H").ok().and_then(|v| v.parse().ok()).unwrap_or(512);
        let secs: f64 = std::env::var("ANIMA_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(5.0);
        let prompt = std::env::var("ANIMA_PROMPT").unwrap_or_else(|_| {
            "电影感实拍，固定机位中近景。清晨的中式茶馆，木格窗透进冷白晨光，\
             一位六十岁上下的亚洲老茶师提起铜壶注水，抬眼看向对座的年轻人，\
             低声说：「这一泡，你自己来。」蒸汽在光里升起，全程不切镜。"
                .into()
        });
        let g = if std::env::var("ANIMA_ROUTE").as_deref() == Ok("h3") {
            // 和 run_weetodd 同一条规则：短边 ≤ 640 走预览档
            let tier = if w.max(h) <= 640 { H3Tier::Preview } else { H3Tier::Standard };
            let img = std::env::var("ANIMA_H3_IMAGE").ok();
            let h3_end = std::env::var("ANIMA_H3_END").ok();
            let refs: Vec<String> = std::env::var("ANIMA_H3_REFS").ok()
                .map(|v| v.split(',').map(str::to_string).collect()).unwrap_or_default();
            let mode = if !refs.is_empty() {
                H3Mode::Reference { images: &refs, pixel_budget_percent: 100, audios: &[] }
            } else if img.is_some() || h3_end.is_some() {
                let f = img.as_deref();
                H3Mode::Frames { first: f, last: h3_end.as_deref() }
            } else { H3Mode::Text };
            build_h3_graph(&H3Params {
                continuity: None,
                mode,
                prompt: &prompt, width: w, height: h, duration_secs: secs,
                seed: 20_260_904, tier, filename_prefix: "Anima/h3", loras: &[],
            })
        } else {
            let wc: u32 = std::env::var("ANIMA_WINDOWS").ok().and_then(|v| v.parse().ok())
                .unwrap_or_else(|| window_count_for(secs));
            // 每段提示词用 `||` 分隔；不给就退回全段同一句（现状）
            let win_prompts: Vec<String> = std::env::var("ANIMA_WIN_PROMPTS")
                .map(|v| v.split("||").map(|x| x.trim().to_string()).collect())
                .unwrap_or_default();
            build_graph(&WeeToddParams {
                first_frame: std::env::var("ANIMA_LTX_IMAGE").ok().as_deref().map(|s| Box::leak(s.to_string().into_boxed_str()) as &str),
                last_frame: std::env::var("ANIMA_LTX_END").ok().as_deref().map(|s| Box::leak(s.to_string().into_boxed_str()) as &str),
                prompt: &prompt, width: w, height: h, duration_secs: secs,
                fps: 24.0, seed: 20_260_904, filename_prefix: "Anima/ltx25",
                loras: &[], window_count: wc,
                window_prompts: &win_prompts,
            })
        };
        std::fs::write(&out, serde_json::to_string_pretty(&g).unwrap()).unwrap();
    }

    /// 把 Rust 真正会提交的 **H3** 图导出来打真实 ComfyUI。
    ///
    /// `ANIMA_GRAPH_DUMP=/x.json ANIMA_H3_TIER=preview \
    ///   cargo test --lib dump_weetodd_h3_graph -- --ignored`
    #[test]
    #[ignore]
    fn dump_weetodd_h3_graph() {
        let out = std::env::var("ANIMA_GRAPH_DUMP").expect("要设 ANIMA_GRAPH_DUMP");
        let tier = match std::env::var("ANIMA_H3_TIER").unwrap_or_default().as_str() {
            "preview" => H3Tier::Preview,
            "staged" => H3Tier::Staged,
            _ => H3Tier::Standard,
        };
        let (w, h) = std::env::var("ANIMA_H3_SIZE").ok()
            .and_then(|v| v.split_once('x').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?))))
            .unwrap_or(if tier == H3Tier::Preview { (640, 384) } else { (768, 512) });
        // 可选首帧（ComfyUI input/ 下的文件名）—— 运镜示例拿同一张首帧试 H3（2026-09-25）
        let first = std::env::var("ANIMA_H3_FIRST").ok().filter(|v| !v.is_empty());
        let secs: f64 = std::env::var("ANIMA_H3_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(5.0);
        let seed: u64 = std::env::var("ANIMA_H3_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(771_231);
        let g = build_h3_graph(&H3Params {
            continuity: None,
            mode: match first.as_deref() { Some(f) => H3Mode::Frames { first: Some(f), last: None }, None => H3Mode::Text },
            prompt: &std::env::var("ANIMA_H3_PROMPT").unwrap_or_else(|_| {
                "电影感实拍，固定机位中近景。傍晚的老式理发店里，暖黄的吊灯和窗外冷蓝的天光交错。\
                 一位四十岁上下的亚洲女理发师停下手中的剪刀，看着镜子里的顾客，轻声说：\
                 「你确定要剪掉吗？留了十年了。」镜头缓慢向前推进一点，全程不切镜。"
                    .into()
            }),
            width: w, height: h, duration_secs: secs, seed,
            tier, filename_prefix: "Anima/H3_rust", loras: &[],
        });
        std::fs::write(&out, serde_json::to_string_pretty(&g).unwrap()).unwrap();
    }

    /// 把 Rust 真正会提交的图导出来，用于打真实 ComfyUI。
    /// 手抄一份副本去测等于测了个副本。
    ///
    /// `ANIMA_GRAPH_DUMP=/x.json cargo test --lib dump_weetodd_graph -- --ignored`
    #[test]
    #[ignore]
    fn dump_weetodd_graph() {
        let out = std::env::var("ANIMA_GRAPH_DUMP").expect("要设 ANIMA_GRAPH_DUMP");
        let dur: f64 = std::env::var("ANIMA_WT_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(5.0);
        let loras: Vec<(String, f64)> = std::env::var("ANIMA_WT_LORA")
            .ok()
            .filter(|v| !v.is_empty())
            .map(|v| vec![(v, 1.0)])
            .unwrap_or_default();
        let prompt = std::env::var("ANIMA_WT_PROMPT").unwrap_or_else(|_| {
            "电影感实拍，固定机位中近景。清晨的中式茶馆，木格窗透进冷白晨光，             一位六十岁上下的亚洲老茶师提起铜壶注水，抬眼看向对座的年轻人，             低声说：「这一泡，你自己来。」蒸汽在光里升起，全程不切镜。"
                .into()
        });
        // 可选：首帧（ComfyUI input/ 下的文件名）和尺寸 —— 画布运镜示例用同一张首帧出 21 段（2026-09-25）
        let first = std::env::var("ANIMA_WT_FIRST").ok().filter(|v| !v.is_empty());
        let (w, h) = std::env::var("ANIMA_WT_SIZE").ok()
            .and_then(|v| v.split_once('x').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?))))
            .unwrap_or((768, 512));
        let g = build_graph(&WeeToddParams {
            first_frame: first.as_deref(),
            last_frame: None,
            prompt: &prompt,
            width: w,
            height: h,
            duration_secs: dur,
            fps: 24.0,
            seed: 20_260_903,
            filename_prefix: "Anima/WT_rust",
            loras: &loras,
            window_count: window_count_for(dur),
            window_prompts: &[],
        });
        std::fs::write(&out, serde_json::to_string_pretty(&g).unwrap()).unwrap();
    }

    fn h3(tier: H3Tier, loras: &[(String, f64)]) -> H3Params<'_> {
        H3Params {
            continuity: None,
            mode: H3Mode::Text,
            prompt: "一位女理发师停下剪刀",
            width: 768, height: 512, duration_secs: 5.0, seed: 771_231,
            tier, filename_prefix: "Anima/H3", loras,
        }
    }

    /// **选路的每一条分支都要有明确理由，而且回落必须说出来。**
    #[test]
    fn 选路优先mlx且回落要说明() {
        // 都装了：默认 LTX（更快、控制力强），点名 H3 才走 H3
        assert_eq!(pick_route("", true, true), Route::Ltx);
        assert_eq!(pick_route("auto", true, true), Route::Ltx);
        assert_eq!(pick_route("minimax_h3", true, true), Route::H3);
        assert_eq!(pick_route("H3", true, true), Route::H3);

        // 点名 H3 但 H3 没装 → 用 LTX，**不能**回落原生（原生 H3 权重已删）
        let r = pick_route("h3", true, false);
        assert_eq!(r, Route::Ltx);
        let n = route_note(r, "h3").expect("换了引擎必须说明");
        assert!(n.contains("H3"), "没说清换掉了什么：{n}");

        // WeeTodd 完全没装 → 原生，且必须说明代价
        let r = pick_route("", false, false);
        assert_eq!(r, Route::Native);
        let n = route_note(r, "").expect("回落原生必须说明");
        assert!(n.contains("96.82") || n.contains("内存"), "没说代价：{n}");

        // 正常走 MLX 时不该有多余的告警
        assert!(route_note(Route::Ltx, "").is_none());
        assert!(route_note(Route::H3, "h3").is_none());
    }

    /// **取结果要扫全部节点，不能用固定 id 列表。**
    ///
    /// 原来两处硬编码分别是 `["10","9"]` 和 `["15","14","11","10"]`，
    /// 而 WeeTodd 的 LTX 出在 `4`、H3 出在 `7` —— 两个都不在列表里，
    /// 症状是「任务 success 但找不到文件」。
    #[test]
    fn 取结果扫全部输出节点() {
        // H3：节点 7 / gifs（本机真实结构）
        let h3 = json!({"7": {"gifs": [
            {"filename": "H3_probe_771231.mp4", "subfolder": "Anima",
             "type": "output", "format": "video/mp4"}]}});
        assert_eq!(
            find_video_output(&h3),
            Some(("H3_probe_771231.mp4".into(), "Anima".into(), "output".into()))
        );
        // LTX：节点 4
        let ltx = json!({"4": {"gifs": [{"filename": "a.mp4", "subfolder": "", "type": "output"}]}});
        assert_eq!(find_video_output(&ltx), Some(("a.mp4".into(), "".into(), "output".into())));
        // 原生：节点 10 / videos
        let native = json!({"10": {"videos": [{"filename": "b.mp4", "subfolder": "video", "type": "output"}]}});
        assert_eq!(find_video_output(&native), Some(("b.mp4".into(), "video".into(), "output".into())));
        // 多节点时按**数字**排序取第一个。字符串序里 "11" < "9"，会挑错。
        let multi = json!({
            "11": {"gifs": [{"filename": "node11.mp4", "subfolder": "", "type": "output"}]},
            "9":  {"gifs": [{"filename": "node9.mp4",  "subfolder": "", "type": "output"}]},
        });
        assert_eq!(find_video_output(&multi).unwrap().0, "node9.mp4", "9 该排在 11 前面");
        // WeeTodd 挂 LoRA 时节点号到 900+，真正的输出节点必须仍排在前面
        let with_lora = json!({
            "900": {"gifs": [{"filename": "lora_node.mp4", "subfolder": "", "type": "output"}]},
            "7":   {"gifs": [{"filename": "real.mp4", "subfolder": "", "type": "output"}]},
        });
        assert_eq!(find_video_output(&with_lora).unwrap().0, "real.mp4");
        // 只有文本输出时不该误判
        assert_eq!(find_video_output(&json!({"3": {"text": ["hello"]}})), None);
        assert_eq!(find_video_output(&json!({})), None);
        // 空文件名不算
        assert_eq!(find_video_output(&json!({"4": {"gifs": [{"filename": ""}]}})), None);
        // LoadVideo 报出来的输入视频不算产出 —— 原样照 2026-09-23 那次深度捕捉的 history
        let depth = json!({
            "1": {"images": [{"filename": "anima_depth_src_47852500299d.mp4", "subfolder": "", "type": "input"}], "animated": [true]},
            "5": {"gifs": [{"filename": "canvas_depth_00004.mp4", "subfolder": "Anima", "type": "output", "format": "video/h264-mp4"}]},
        });
        assert_eq!(find_video_output(&depth), Some(("canvas_depth_00004.mp4".into(), "Anima".into(), "output".into())));
        assert_eq!(find_video_output(&json!({"1": {"images": [{"filename": "src.mp4", "type": "input"}]}})), None, "只有输入就是没产出");
    }

    /// 探测只看 `/object_info`，不看文件系统。
    #[test]
    fn 装没装只看object_info() {
        let oi = json!({"WeeToddH3Generate": {}, "WeeToddLTX25Generate": {}});
        assert!(probe_installed(&oi, H3_PROBE_NODE));
        assert!(probe_installed(&oi, PROBE_NODE));
        assert!(!probe_installed(&json!({}), H3_PROBE_NODE));
    }

    /// 迁移说明必须说清三件事：去哪了、原来那套没了、以后别在哪儿找。
    #[test]
    fn 迁移说明要能自解释() {
        let n = H3_MIGRATED_NOTE;
        assert!(n.contains("WeeTodd"), "没说去哪了：{n}");
        assert!(n.contains("MiniMax-H3/"), "没给新路径：{n}");
        assert!(n.contains("删除"), "没说原来那套已删 —— 用户会以为是装漏了：{n}");
        assert!(n.contains("diffusion_models"), "没说以后别在哪儿找：{n}");
    }

    /// **H3 的组件路径写错了不会「找不到文件」，会报别的错。**
    ///
    /// audio_vae 尤其阴：官方仓库有两个同名目录，都是 605 MB，只有布局不同。
    /// 下错那份报的是「metadata file not found」，看不出是目录选错。
    #[test]
    fn h3组件路径钉死() {
        let g = build_h3_graph(&h3(H3Tier::Standard, &[]));
        let i = &g["1"]["inputs"];
        assert_eq!(i["audio_vae"], "MiniMax-H3/FL2VA/audio_vae",
            "必须是 FL2VA/ 下那份（metadata.json + model.safetensors），不是顶层 audio_vae/");
        assert_eq!(i["transformer"], "MiniMax-H3/transformers/q8_extended_paged");
        assert_eq!(i["text_encoder"], "MiniMax-H3/text_encoders/q8-paged");
        // 这些都不在 diffusion_models/ 下 —— 放进去会污染原生下拉框
        for k in ["transformer", "text_encoder", "video_vae", "audio_vae"] {
            let v = i[k].as_str().unwrap();
            assert!(v.starts_with("MiniMax-H3/"), "{k} 该在 MiniMax-H3/ 下：{v}");
        }
    }

    /// LTX 的首帧只是给 `Generate` 加一个输入，不换分区也不换编码器。
    /// **和 H3 完全不同** —— 那边要切 `fl2va` 分区并走 KeyframeEncode。
    #[test]
    fn ltx首帧只是多一个loadimage() {
        let p = |f: Option<&'static str>| WeeToddParams {
            first_frame: f, last_frame: None, prompt: "x", width: 768, height: 512,
            duration_secs: 5.0, fps: 24.0, seed: 1, filename_prefix: "p",
            loras: &[], window_count: 1, window_prompts: &[],
        };
        let t2v = build_graph(&p(None));
        assert!(t2v.get("10").is_none(), "纯文生不该有 LoadImage");
        assert!(t2v["4"]["inputs"].get("first_frame").is_none());

        let i2v = build_graph(&p(Some("src.png")));
        assert_eq!(i2v["10"]["class_type"], "LoadImage");
        assert_eq!(i2v["10"]["inputs"]["image"], "src.png");
        assert_eq!(i2v["4"]["inputs"]["first_frame"], json!(["10", 0]));
        // 其余一切不变 —— 同一个 Generate 节点、同一条 LoRA 链
        assert_eq!(i2v["4"]["class_type"], PROBE_NODE);
        assert_eq!(i2v["4"]["inputs"]["model"], t2v["4"]["inputs"]["model"]);
    }

    #[test]
    fn 末帧钉在最后一帧上() {
        let p = |l: Option<&'static str>| WeeToddParams {
            first_frame: Some("a.png"), last_frame: l, prompt: "x",
            width: 768, height: 512, duration_secs: 5.0, fps: 24.0,
            seed: 1, filename_prefix: "p", loras: &[],
            window_count: 1, window_prompts: &[],
        };
        assert!(build_graph(&p(None)).get("12").is_none());

        let g = build_graph(&p(Some("z.png")));
        assert_eq!(g["11"]["inputs"]["image"], "z.png");
        assert_eq!(g["12"]["class_type"], "WeeToddLTX25Keyframe");
        // 5 秒 × 24fps = 120 帧，末帧下标 119。给 120 会越界。
        assert_eq!(g["12"]["inputs"]["frame_index"], 119);
        assert_eq!(g["4"]["inputs"]["keyframes"], json!(["12", 0]));

        // **有末帧时首帧必须改走关键帧链，且不能再挂 `first_frame`。**
        // 上游只要看到任何关键帧就把 `first_frame` 整个丢掉、不报错 ——
        // 实测出过"末帧对上了、首帧是另一个画面"的片子。
        assert!(
            g["4"]["inputs"].get("first_frame").is_none(),
            "有末帧时还挂 first_frame，上游会静默丢掉它"
        );
        assert_eq!(g["13"]["class_type"], "WeeToddLTX25Keyframe");
        assert_eq!(g["13"]["inputs"]["frame_index"], 0);
        assert_eq!(g["13"]["inputs"]["image"], json!(["10", 0]));
        // 两个关键帧要串起来，否则只有末帧进得去
        assert_eq!(g["12"]["inputs"]["previous_keyframes"], json!(["13", 0]));
    }

    /// 点名一个别的引擎不能被 LTX 顶掉 —— 那是没有任何线索的替换。
    #[test]
    fn 点名别的引擎就交回原生并说明() {
        assert_eq!(pick_route("svd", true, true), Route::Native);
        assert_eq!(pick_route("hunyuan", true, true), Route::Native);
        assert!(route_note(Route::Native, "svd").is_some_and(|m| m.contains("svd")));
        // 空 / auto 不算点名，照旧优先 MLX
        assert_eq!(pick_route("", true, true), Route::Ltx);
        assert_eq!(pick_route("auto", true, true), Route::Ltx);
        assert_eq!(pick_route("h3", true, true), Route::H3);
        assert_eq!(pick_route("ltx-2.5", true, true), Route::Ltx);
    }

    #[test]
    fn 帧率没给才用原生24() {
        assert_eq!(clamp_fps(None), 24.0);
        // 给了就得用给的 —— 静默换成 24 会让时长和口型错位
        assert_eq!(clamp_fps(Some(16.0)), 16.0);
        assert_eq!(clamp_fps(Some(30.0)), 30.0);
        // 越界和垃圾值夹回去，别把节点参数写坏
        assert_eq!(clamp_fps(Some(120.0)), 60.0);
        assert_eq!(clamp_fps(Some(0.0)), 24.0);
        assert_eq!(clamp_fps(Some(f64::NAN)), 24.0);
    }

    #[test]
    fn 尺寸对齐到32且长边不超1344() {
        // 竖图：长边压到 1344，短边按比例
        assert_eq!(fit_dimensions((1080, 1920)), (768, 1344));
        // 小图不放大
        assert_eq!(fit_dimensions((640, 480)), (640, 480));
        // 非 32 倍数被对齐 —— 这条一旦失效，报错里不会出现原始数字
        let (w, h) = fit_dimensions((1000, 700));
        assert_eq!((w % 32, h % 32), (0, 0), "得到 {w}x{h}");
        // 退化输入不能产出 0
        assert_eq!(fit_dimensions((0, 0)), (32, 32));
    }

    /// 文生和图生用的**不是同一个编码器** —— 这条错了要等模型加载才炸。
    /// 三种关键帧组合各挑各的节点。**挑错不报错** —— 图不进条件，
    /// 出来是一段和它无关的视频，任务照样 success。
    #[test]
    fn 首末帧各挑各的条件节点() {
        let kind = |first, last| {
            let g = build_h3_graph(&H3Params {
                continuity: None,
                mode: H3Mode::Frames { first, last },
                prompt: "x", width: 640, height: 384, duration_secs: 3.0,
                seed: 1, tier: H3Tier::Preview, filename_prefix: "p", loras: &[],
            });
            (
                g["11"]["class_type"].as_str().unwrap().to_string(),
                g["11"]["inputs"].clone(),
                g.get("10").is_some(),
                g.get("12").is_some(),
            )
        };

        let (k, inp, has10, has12) = kind(Some("a.png"), None);
        assert_eq!(k, "WeeToddH3FirstFrame");
        assert_eq!(inp["first_frame"], json!(["10", 0]));
        assert!(inp.get("last_frame").is_none());
        assert!(has10 && !has12);

        let (k, inp, has10, has12) = kind(None, Some("z.png"));
        assert_eq!(k, "WeeToddH3LastFrame");
        assert_eq!(inp["last_frame"], json!(["12", 0]));
        assert!(inp.get("first_frame").is_none(), "只给末帧却接了首帧");
        assert!(!has10 && has12);

        let (k, inp, has10, has12) = kind(Some("a.png"), Some("z.png"));
        assert_eq!(k, "WeeToddH3FirstLastFrame");
        assert_eq!(inp["first_frame"], json!(["10", 0]));
        assert_eq!(inp["last_frame"], json!(["12", 0]));
        assert!(has10 && has12, "两张图都得有各自的 LoadImage");

        // 三种都用 fl2va 分区，且都从节点 5 出条件
        for m in [
            H3Mode::Frames { first: Some("a.png"), last: None },
            H3Mode::Frames { first: None, last: Some("z.png") },
            H3Mode::Frames { first: Some("a.png"), last: Some("z.png") },
        ] {
            let g = build_h3_graph(&H3Params {
                continuity: None,
                mode: m, prompt: "x", width: 640, height: 384, duration_secs: 3.0,
                seed: 1, tier: H3Tier::Preview, filename_prefix: "p", loras: &[],
            });
            assert_eq!(g["1"]["inputs"]["task"], "fl2va");
            assert_eq!(g["5"]["inputs"]["keyframes"], json!(["11", 0]));
        }
    }

    #[test]
    fn 文生用分页图生用视觉编码器() {
        assert_eq!(h3_text_encoder(&H3Mode::Text), H3_TEXT_ENCODER);
        assert!(H3_TEXT_ENCODER.contains("paged"), "文生这份就该是分页的");

        for mode in [
            H3Mode::Frames { first: Some("a.png"), last: None },
            H3Mode::Reference { images: &[], pixel_budget_percent: 100, audios: &[] },
        ] {
            let e = h3_text_encoder(&mode);
            assert_ne!(e, H3_TEXT_ENCODER, "图生不能用纯文本那份");
            assert!(!e.contains("paged"), "preflight 拒一切分页编码器，得到 {e}");
            assert!(H3_VISION_ENCODER_CANDIDATES.contains(&e.as_str()), "得到 {e}");
        }
    }

    /// **上面那条只测了函数，测不到建图有没有真的调它。**
    /// 之前 LoRA 那次就是这样：函数对了、调用点没接上，测试全绿。
    #[test]
    fn 建图真的按模式填了编码器() {
        let g = |mode| {
            build_h3_graph(&H3Params {
                continuity: None,
                mode, prompt: "x", width: 640, height: 384, duration_secs: 3.0,
                seed: 1, tier: H3Tier::Preview, filename_prefix: "p", loras: &[],
            })["1"]["inputs"]["text_encoder"]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert_eq!(g(H3Mode::Text), H3_TEXT_ENCODER);
        for mode in [
            H3Mode::Frames { first: Some("a.png"), last: None },
            H3Mode::Reference { images: &[], pixel_budget_percent: 100, audios: &[] },
        ] {
            let e = g(mode);
            assert!(!e.contains("paged"), "图上填的还是分页那份: {e}");
            assert!(H3_VISION_ENCODER_CANDIDATES.contains(&e.as_str()), "图上填了 {e}");
        }
    }

    #[test]
    fn 空目录不算装了视觉编码器() {
        let tmp = std::env::temp_dir().join(format!("anima_vis_{}", std::process::id()));
        let dir = tmp.join(H3_VISION_ENCODER_CANDIDATES[0]);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(h3_vision_encoder_in(&tmp), None, "空目录不该算数");

        std::fs::write(dir.join("config.json"), "{}").unwrap();
        assert_eq!(h3_vision_encoder_in(&tmp), None, "只有 config 也不算");

        std::fs::write(dir.join("model-00001.safetensors"), "x").unwrap();
        assert_eq!(
            h3_vision_encoder_in(&tmp).as_deref(),
            Some(H3_VISION_ENCODER_CANDIDATES[0])
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// 做不到的事必须**提交前**说，而不是生成完让用户自己发现参数被吞了。
    #[test]
    fn 做不到的组合会被提前拦住() {
        // LTX 不吃参考图
        assert!(animate_blocker(Route::Ltx, true)
            .is_some_and(|m| m.contains("参考图")));
        // **末帧两条路都支持** —— 曾经这里断言 H3 拦末帧，那个断言把我
        // 一个没查证的结论钉住了，直到发现 WeeToddH3FirstLastFrame 才推翻。
        assert_eq!(animate_blocker(Route::H3, false), None);
        assert_eq!(animate_blocker(Route::Ltx, false), None);
        assert_eq!(animate_blocker(Route::Native, true), None);
    }

    /// **三种模式必须都产出节点 5，采样链才对它们透明。**
    ///
    /// 接错了不会报错 —— Sample 拿到的 conditioning 只是不含图像信息，
    /// 出来的片子看着像纯文生，看不出参考图被忽略了。
    #[test]
    fn 三种模式都从节点5出条件() {
        let refs = vec!["a.png".to_string(), "b.png".to_string()];
        for (label, mode) in [
            ("t2v", H3Mode::Text),
            ("i2v", H3Mode::Frames { first: Some("src.png"), last: None }),
            ("r2v", H3Mode::Reference { images: &refs, pixel_budget_percent: 100, audios: &[] }),
        ] {
            let g = build_h3_graph(&H3Params {
                continuity: None,
                mode, prompt: "x", width: 768, height: 512, duration_secs: 5.0,
                seed: 1, tier: H3Tier::Standard, filename_prefix: "p", loras: &[],
            });
            assert_eq!(g["6"]["inputs"]["conditioning"], json!(["5", 0]), "{label} 的采样没取节点 5");
            let ct = g["5"]["class_type"].as_str().unwrap();
            assert!(ct.ends_with("Encode"), "{label} 的节点 5 不是编码器：{ct}");
            // 提示词必须真的送进去 —— 三个编码器的字段名都叫 prompt
            assert_eq!(g["5"]["inputs"]["prompt"], "x", "{label} 丢了提示词");
            // 分区必须和模式对上 —— 写错要到编码那一步才炸
            let want = match label { "t2v" => "t2va", "i2v" => "fl2va", _ => "ref2va" };
            assert_eq!(g["1"]["inputs"]["task"], want, "{label} 的分区不对");
        }
    }

    /// 画布音频参考（2026-10-08）：音频串在图片参考之后，`LoadAudio` → `ReferenceAudio`，最后进编码器；最多 3 段
    #[test]
    fn 参考音频串在图片之后() {
        let imgs = vec!["a.png".to_string(), "b.png".to_string()];
        let auds: Vec<String> = (0..5).map(|i| format!("song{i}.wav")).collect();
        let g = build_h3_graph(&H3Params {
            continuity: None,
            mode: H3Mode::Reference { images: &imgs, pixel_budget_percent: 100, audios: &auds },
            prompt: "x", width: 768, height: 512, duration_secs: 15.0, seed: 1,
            tier: H3Tier::Standard, filename_prefix: "p", loras: &[],
        });
        assert_eq!(g["60"]["class_type"], "LoadAudio");
        assert_eq!(g["60"]["inputs"]["audio"], "song0.wav");
        assert_eq!(g["61"]["class_type"], "WeeToddH3ReferenceAudio");
        assert_eq!(g["61"]["inputs"]["previous_references"], json!(["23", 0]), "第一段音频没接在最后一张图后面");
        assert_eq!(g["63"]["inputs"]["previous_references"], json!(["61", 0]));
        assert!(g.get("66").is_none(), "超过 3 段要截掉");
        assert_eq!(g["5"]["inputs"]["references"], json!(["65", 0]), "编码器没吃到音频");
        assert!(graph_pipeline(&g).unwrap().ends_with("参考音频 3 段"));
        // 没音频时和以前一模一样
        let g0 = build_h3_graph(&H3Params {
            continuity: None,
            mode: H3Mode::Reference { images: &imgs, pixel_budget_percent: 100, audios: &[] },
            prompt: "x", width: 768, height: 512, duration_secs: 5.0, seed: 1,
            tier: H3Tier::Standard, filename_prefix: "p", loras: &[],
        });
        assert_eq!(g0["5"]["inputs"]["references"], json!(["23", 0]));
    }

    /// 远景修脸（FaceRefine 搬到 MLX）：选人 → 裁 768 → 编码 512 → 1.5x 精修 → 解码 → 贴回 → 带原声输出，接线和出片同一份前缀
    #[test]
    fn 修脸图接线() {
        let g = build_face_refine_graph(&FaceRefineParams {
            video_path: "/x/shot.mp4", identity_image: None, prompt: "<Picture 1> ...", strength: 0.4,
            select: "largest_face", seed: 7, tier: H3Tier::Staged, filename_prefix: "Anima/face",
        });
        assert_eq!(g["1"]["inputs"]["task"], "ref2va");
        assert_eq!(g["30"]["class_type"], "H3FaceSelect");
        assert_eq!(g["31"]["inputs"]["canvas_mode"], "manual");
        assert_eq!(g["31"]["inputs"]["canvas_width"], FACE_REFINE_CANVAS);
        assert_eq!(g["31"]["inputs"]["face_pick"], json!(["30", 2]));
        assert_eq!(g["32"]["class_type"], "AnimaH3EncodeVideoLatents");
        assert_eq!(g["32"]["inputs"]["images"], json!(["31", 0]));
        assert_eq!(g["32"]["inputs"]["audio"], json!(["30", 1]), "原声要进 latent");
        assert_eq!(g["33"]["inputs"]["source_latents"], json!(["32", 0]));
        assert_eq!(g["33"]["inputs"]["scale"], "1.5x — balanced", "512 × 1.5 = 768，对上画布");
        assert_eq!(g["33"]["inputs"]["refinement_strength"], 0.4);
        assert_eq!(g["35"]["inputs"]["base_images"], json!(["30", 0]), "贴回原片，不是贴回裁切");
        assert_eq!(g["35"]["inputs"]["refined_crops"], json!(["34", 0]));
        assert_eq!(g["35"]["inputs"]["transform"], json!(["31", 1]));
        assert_eq!(g["36"]["inputs"]["audio"], json!(["30", 1]));
        assert_eq!(g["36"]["inputs"]["frame_rate"], json!(["30", 6]));
        // 没给身份图：用裁出来的第一帧；给了：用那张
        assert_eq!(g["20"]["class_type"], "ImageFromBatch");
        let g2 = build_face_refine_graph(&FaceRefineParams {
            video_path: "/x/shot.mp4", identity_image: Some("bai.png"), prompt: "x", strength: 9.0,
            select: "largest_face", seed: 7, tier: H3Tier::Staged, filename_prefix: "Anima/face",
        });
        assert_eq!(g2["20"]["inputs"]["image"], "bai.png");
        assert_eq!(g2["33"]["inputs"]["refinement_strength"], 1.0, "强度夹在 0.05~1");
        // 加速 LoRA 和出片同一套：Larry 6 次 = 7 个采样点，LoRA 从自己那条链取
        if g["899"].is_object() {
            assert_eq!(g["33"]["inputs"]["loras"], json!(["899", 0]));
            assert_eq!(g["33"]["inputs"]["refinement_schedule_points"], 7);
        }
        // 每个引用都指向图里存在的节点
        for (id, n) in g.as_object().unwrap() {
            for v in n["inputs"].as_object().unwrap().values() {
                if let Some(r) = v.as_array().filter(|a| a.len() == 2 && a[0].is_string() && a[1].is_number()) {
                    assert!(g.get(r[0].as_str().unwrap()).is_some(), "节点 {id} 引用了不存在的 {}", r[0]);
                }
            }
        }
    }

    /// 右下角进度前面那句链路是从图里读的（用户 2026-10-07：「不是硬编码出来的，而是返回的」）
    #[test]
    fn 链路说明从图里读() {
        let refs = vec!["a.png".to_string()];
        let g = |mode, tier| build_h3_graph(&H3Params {
            continuity: None, mode, prompt: "x", width: 768, height: 512, duration_secs: 5.0,
            seed: 1, tier, filename_prefix: "p", loras: &[],
        });
        let first = graph_pipeline(&g(H3Mode::Frames { first: Some("f.png"), last: None }, H3Tier::LightX2V)).unwrap();
        assert_eq!(first, "FL2VA 首帧 · LightX2V 4 次");
        let refd = graph_pipeline(&g(H3Mode::Reference { images: &refs, pixel_budget_percent: 80, audios: &[] }, H3Tier::Staged)).unwrap();
        assert!(refd.starts_with("Ref2VA "), "{refd}");
        assert!(refd.contains("Larry v4 6 次") || refd.contains("稠密") || refd.contains("Ref2VA 专用"), "{refd}");
        // 兼容 / 原生由图上的开关决定，不由设置推
        let mut compat = g(H3Mode::Reference { images: &refs, pixel_budget_percent: 80, audios: &[] }, H3Tier::Staged);
        compat["1"]["inputs"]["allow_fl2va_weights_for_ref2va"] = json!(true);
        assert!(graph_pipeline(&compat).unwrap().starts_with("Ref2VA 兼容（FL2VA 权重）"));
        compat["1"]["inputs"]["allow_fl2va_weights_for_ref2va"] = json!(false);
        assert!(graph_pipeline(&compat).unwrap().starts_with("Ref2VA 原生"));
        assert_eq!(graph_pipeline(&g(H3Mode::Text, H3Tier::Dense)).unwrap(), "T2VA 文生 · 稠密不加速 19 次");
        assert_eq!(graph_pipeline(&g(H3Mode::Text, H3Tier::LegacyTurbo)).unwrap(), "T2VA 文生 · drbaph 精简版 4 次");
        assert_eq!(graph_pipeline(&json!({"1": {"class_type": "KSampler", "inputs": {}}})), None, "认不出的图不编");
    }

    /// i2v：首帧要经过 `FirstFrame` 转成 keyframes，不能直接喂给编码器。
    #[test]
    fn 首帧走keyframe那条链() {
        let g = build_h3_graph(&H3Params {
            continuity: None,
            mode: H3Mode::Frames { first: Some("src.png"), last: None }, prompt: "x",
            width: 768, height: 512, duration_secs: 5.0, seed: 1,
            tier: H3Tier::Standard, filename_prefix: "p", loras: &[],
        });
        assert_eq!(g["10"]["class_type"], "LoadImage");
        assert_eq!(g["10"]["inputs"]["image"], "src.png");
        assert_eq!(g["11"]["class_type"], "WeeToddH3FirstFrame");
        assert_eq!(g["11"]["inputs"]["first_frame"], json!(["10", 0]));
        assert_eq!(g["5"]["class_type"], "WeeToddH3KeyframeEncode");
        assert_eq!(g["5"]["inputs"]["keyframes"], json!(["11", 0]));
        // **首帧走 fl2va 分区**（First/Last frame to Video+Audio）。
        // 写成 t2va 会在 KeyframeEncode 那一步炸：
        // 「H3 keyframe encoding requires an FL2VA component set」
        assert_eq!(g["1"]["inputs"]["task"], "fl2va");
    }

    /// r2v：多张参考图要**串**起来（每个吃上一个的 references），不是并联。
    ///
    /// 并联的话只有最后一张生效，而且不报错。
    #[test]
    fn 参考图串成链且切到ref2va分区() {
        let refs = vec!["a.png".to_string(), "b.png".to_string(), "c.png".to_string()];
        let g = build_h3_graph(&H3Params {
            continuity: None,
            mode: H3Mode::Reference { images: &refs, pixel_budget_percent: 80, audios: &[] },
            prompt: "x", width: 768, height: 512, duration_secs: 5.0, seed: 1,
            tier: H3Tier::Standard, filename_prefix: "p", loras: &[],
        });
        assert_eq!(g["1"]["inputs"]["task"], "ref2va");
        // 复用 FL2VA 权重，否则要另下一整套 Ref2VA
        assert_eq!(g["1"]["inputs"]["allow_fl2va_weights_for_ref2va"], true);
        assert!(g["21"]["inputs"].get("previous_references").is_none(), "第一张不该有前驱");
        assert_eq!(g["23"]["inputs"]["previous_references"], json!(["21", 0]));
        assert_eq!(g["25"]["inputs"]["previous_references"], json!(["23", 0]));
        assert_eq!(g["5"]["inputs"]["references"], json!(["25", 0]), "编码器要取链尾");
        assert_eq!(g["21"]["inputs"]["pixel_budget_percent"], 80);
    }

    /// **接上一镜：视频参考要串在图片参考之后，并且接进 ReferenceEncode。**
    ///
    /// 顺序决定编号 —— 图片是 `<Picture 1..N>`、视频是 `<Video 1>`。
    /// 串错位置的话提示词里的编号就对不上，**而且不报错**。
    #[test]
    fn 接上一镜时视频参考串在图片之后() {
        let imgs = vec!["a.png".to_string(), "b.png".to_string()];
        let g = build_h3_graph(&H3Params {
            mode: H3Mode::Reference { images: &imgs, pixel_budget_percent: 100, audios: &[] },
            continuity: Some(H3Continuity {
                video_path: "/tmp/prev.mp4", start_secs: 14.0, frames: 24, fps: 24.0,
            }),
            prompt: "x", width: 576, height: 1024, duration_secs: 5.0, seed: 1,
            tier: H3Tier::Standard, filename_prefix: "t", loras: &[],
        });
        assert_eq!(g["40"]["class_type"], "VHS_LoadVideoFFmpegPath");
        assert_eq!(g["40"]["inputs"]["video"], "/tmp/prev.mp4");
        assert_eq!(g["40"]["inputs"]["start_time"], 14.0);
        assert_eq!(g["40"]["inputs"]["frame_load_cap"], 24);
        assert_eq!(g["41"]["class_type"], "WeeToddH3ReferenceVideo");
        // **串在最后一张图片之后** —— 两张图是 21 / 23
        assert_eq!(g["41"]["inputs"]["previous_references"], json!(["23", 0]));
        // **另一档 native canvas 上游写明「dramatically slower」，别改**
        assert_eq!(g["41"]["inputs"]["video_size"], "match output (recommended)");
        // 编码那一步吃的是视频参考，不是图片参考
        assert_eq!(g["5"]["inputs"]["references"], json!(["41", 0]), "视频参考没接进 ReferenceEncode");
    }

    // ---- 长镜头：窗口链（2026-09-20）----

    fn 窗口<'a>(prompt: &'a str, imgs: &'a [String]) -> H3Window<'a> {
        H3Window { prompt, images: imgs }
    }

    /// **链要真的串起来。** 每一条边写错都不报错，只是某一段和别的段无关。
    #[test]
    fn 三窗口链的每一条边都接对了() {
        static A: [String; 1] = [String::new()];
        let ws = [窗口("w1", &A), 窗口("w2", &A), 窗口("w3", &A)];
        let g = build_h3_chain_graph(&H3ChainParams {
            windows: &ws, window_secs: 15.0, context_frames: 22,
            width: 576, height: 1024, seed: 7, tier: H3Tier::Standard,
            filename_prefix: "t", loras: &[],
        });

        // 时间轴：等长窗口 + 发布整条
        assert_eq!(g["8"]["class_type"], "WeeToddH3ChainedTimeline");
        assert_eq!(g["8"]["inputs"]["window_count"], 3);
        assert_eq!(g["8"]["inputs"]["window_duration_seconds"], 15.0);
        assert_eq!(g["8"]["inputs"]["target_duration_seconds"], 0.0, "0 才是「发布整条」");
        // **枚举是字符串**，写成数字节点会拒
        assert_eq!(g["8"]["inputs"]["context_frames"], "22");

        // 第一个窗口没有可继承的上下文
        assert!(g["1091"]["inputs"].get("continuation").is_none(), "第一窗口不该有 continuation");
        // 第二、三个窗口继承前一个的尾部
        assert_eq!(g["1191"]["inputs"]["continuation"], json!(["1092", 0]));
        assert_eq!(g["1291"]["inputs"]["continuation"], json!(["1192", 0]));
        // **最后一个窗口不再留尾巴** —— 留了就是白算一次
        assert!(g.get("1292").is_none(), "最后一个窗口多留了 ContinuationContext");

        // 链：一个接一个
        assert!(g["1093"]["inputs"].get("previous_chain").is_none(), "第一段不该有前驱");
        assert_eq!(g["1193"]["inputs"]["previous_chain"], json!(["1093", 0]));
        assert_eq!(g["1293"]["inputs"]["previous_chain"], json!(["1193", 0]));

        // 发布吃的是最后一段链，不是某个窗口的 latents
        assert_eq!(g["7"]["class_type"], "WeeToddH3DirectPublishChain");
        assert_eq!(g["7"]["inputs"]["chain"], json!(["1293", 0]));

        // 每个窗口有自己的提示词
        assert_eq!(g["1090"]["inputs"]["prompt"], "w1");
        assert_eq!(g["1290"]["inputs"]["prompt"], "w3");
    }

    /// **`duration_seconds` 是一个窗口的长度，不是总长。**
    /// 写成总长的话每个窗口都会去生成整条片子那么久。
    #[test]
    fn 配置里的时长是单个窗口的() {
        static A: [String; 1] = [String::new()];
        let ws = [窗口("a", &A), 窗口("b", &A), 窗口("c", &A), 窗口("d", &A)];
        let g = build_h3_chain_graph(&H3ChainParams {
            windows: &ws, window_secs: 15.0, context_frames: 22,
            width: 576, height: 1024, seed: 1, tier: H3Tier::Standard,
            filename_prefix: "t", loras: &[],
        });
        assert_eq!(g["2"]["inputs"]["duration_seconds"], 15.0, "写成 60 的话每个窗口都跑一分钟");
        // 分区必须是 ref2va，写错会在编码那一步才炸
        assert_eq!(g["1"]["inputs"]["task"], "ref2va");
        assert_eq!(g["1"]["inputs"]["allow_fl2va_weights_for_ref2va"], true);
    }

    /// 不接上一镜时**一个视频节点都不许有** —— 多出来会白占参考槽。
    #[test]
    fn 不接上一镜时没有视频参考节点() {
        let imgs = vec!["a.png".to_string()];
        let g = build_h3_graph(&H3Params {
            mode: H3Mode::Reference { images: &imgs, pixel_budget_percent: 100, audios: &[] },
            continuity: None,
            prompt: "x", width: 576, height: 1024, duration_secs: 5.0, seed: 1,
            tier: H3Tier::Standard, filename_prefix: "t", loras: &[],
        });
        let kinds: Vec<&str> = g.as_object().unwrap().values()
            .filter_map(|n| n["class_type"].as_str()).collect();
        assert!(!kinds.iter().any(|k| k.contains("ReferenceVideo")), "{kinds:?}");
        assert!(!kinds.iter().any(|k| k.contains("LoadVideo")), "{kinds:?}");
        assert_eq!(g["5"]["inputs"]["references"], json!(["21", 0]));
    }

    /// 参考图列表为空时退回纯文本 —— 否则编码器拿到空引用。
    #[test]
    fn 参考图为空时退回文本() {
        let empty: Vec<String> = vec![];
        let g = build_h3_graph(&H3Params {
            continuity: None,
            mode: H3Mode::Reference { images: &empty, pixel_budget_percent: 100, audios: &[] },
            prompt: "x", width: 768, height: 512, duration_secs: 5.0, seed: 1,
            tier: H3Tier::Standard, filename_prefix: "p", loras: &[],
        });
        assert_eq!(g["5"]["class_type"], "WeeToddH3TextEncode");
    }

    /// **采样预设同时供 config 和 turbo LoRA** —— 接错了 turbo 就静默不生效。
    ///
    /// 预设节点第 0 个输出是 config、第 1 个是 loras。把 Sample 的 loras
    /// 接到第 0 个上不会报错，只是 turbo 没挂。
    #[test]
    fn 预设的第二个输出才是lora() {
        // 走预设的只剩稠密那档（其余几档自己挂 LoRA、不经过预设）
        let g = build_h3_graph(&h3(H3Tier::Dense, &[]));
        assert_eq!(g["6"]["inputs"]["loras"], json!(["3", 1]), "loras 必须取预设的第 1 个输出");
        assert_eq!(g["6"]["inputs"]["config"], json!(["3", 0]), "config 取第 0 个");
        // 采样和文本编码都要吃预设后的 config，不能吃原始 config（步数会不对）
        assert_eq!(g["5"]["inputs"]["config"], json!(["3", 0]));
        assert_eq!(g["4"]["inputs"]["config"], json!(["3", 0]));
    }

    /// 额外 LoRA 串在预设那条链后面，且 Sample 取链尾。
    #[test]
    fn 额外lora串在预设之后() {
        let l = vec![("a.safetensors".to_string(), 0.8), ("h3/b.safetensors".to_string(), 1.0)];
        let g = build_h3_graph(&h3(H3Tier::Dense, &l));
        assert_eq!(g["900"]["inputs"]["previous_loras"], json!(["3", 1]), "第一个要接在预设的 loras 上");
        // 自己挂加速 LoRA 的档：额外的串在 899 后面
        let g = build_h3_graph(&h3(H3Tier::Standard, &l));
        assert_eq!(g["900"]["inputs"]["previous_loras"], json!(["899", 0]));
        assert_eq!(g["901"]["inputs"]["previous_loras"], json!(["900", 0]), "第二个串在第一个后面");
        assert_eq!(g["901"]["inputs"]["strength"], 1.0);
        assert_eq!(g["6"]["inputs"]["loras"], json!(["901", 0]), "Sample 取链尾");
        // 没有额外 LoRA 时直接取预设
        let g0 = build_h3_graph(&h3(H3Tier::Standard, &[]));
        assert!(g0.get("900").is_none());
    }

    /// **预览覆盖必须在链上，而且要开崩溃守卫 —— 只拦非有限值。**
    ///
    /// 它在 latent 变成非有限值时立刻中止 —— 这个项目最典型的失败正是
    /// 「跑满全部步数才发现是纯黑」。少了它就要白等一整轮。
    /// 不用 `conservative collapse guard`：它把暗场景误判成崩坏（试片 B 被误杀过一次）。
    #[test]
    fn 预览覆盖接在组件和体检之间且开守卫() {
        let g = build_h3_graph(&h3(H3Tier::Preview, &[]));
        assert_eq!(g["9"]["class_type"], "WeeToddH3PreviewOverride");
        assert_eq!(g["9"]["inputs"]["components"], json!(["1", 0]));
        assert_eq!(g["4"]["inputs"]["components"], json!(["9", 0]),
            "体检必须吃预览覆盖后的组件，否则守卫不生效");
        assert_eq!(g["9"]["inputs"]["safety_guard"], "abort non-finite");
    }

    /// 三档的预设字符串必须和 ComfyUI 里注册的一字不差 —— 写错会被当成未知预设。
    #[test]
    fn 三档预设与实测耗时() {
        assert!(H3Tier::Preview.preset().contains("384p low-memory"));
        assert!(H3Tier::Standard.preset().contains("5 points / 4 evaluations"));
        assert!(H3Tier::Staged.preset().starts_with("Staged Turbo"));
        // 实测值，用于超时预算；顺序不能乱（Preview 最快、Staged 最慢）
        assert!(H3Tier::Preview.secs() < H3Tier::Standard.secs());
        assert!(H3Tier::Standard.secs() < H3Tier::Staged.secs());
        // 快速 / 标准 / 双采：Larry 完整版 4 / 4 / 6 次求值，不经过预设（10-07 起）
        for (t, points) in [(H3Tier::Preview, 5), (H3Tier::Standard, 5), (H3Tier::Staged, 7)] {
            let g = build_h3_graph(&h3(t, &[]));
            assert!(g.get("3").is_none(), "{t:?} 还经过预设");
            assert_eq!((g["899"]["inputs"]["lora_name"].clone(), g["2"]["inputs"]["steps"].clone()), (json!(H3_LARRY_TURBO_LORA), json!(points)), "{t:?}");
        }
        assert_eq!(build_h3_graph(&h3(H3Tier::Dense, &[]))["3"]["inputs"]["preset"], H3Tier::Dense.preset());
    }

    /// **`preset` 不是 `Custom` 就会把逐项控件覆盖掉。**
    ///
    /// 2026-09-03 第一次提交就死在这：preset 用了「Official parity」，
    /// 它把 `low_ram_streaming` 压回 false，而 q8 分页权重当场报
    /// 「Paged LTX 2.5 transformer checkpoints require low_ram_streaming=true」。
    #[test]
    fn preset_必须是custom且开低内存流式() {
        let g = build_graph(&p(5.0, &[], 1));
        assert_eq!(g["2"]["inputs"]["preset"], "Custom");
        assert_eq!(g["2"]["inputs"]["low_ram_streaming"], true);
    }

    /// q8 分页权重必须走 `weetodd-q8/`，**不能放回 `diffusion_models/`**。
    ///
    /// 放回去会往原生 `UNETLoader` 的下拉框里塞 49 条分页噪声，
    /// 而且那些分页是可选的 —— 选中 `layer-017` 等于把第 17 层当整个模型加载。
    #[test]
    fn q8权重不落在原生扫描目录里() {
        let g = build_graph(&p(5.0, &[], 1));
        for k in ["transformer", "text_encoder"] {
            let v = g["1"]["inputs"][k].as_str().unwrap();
            assert!(v.starts_with("weetodd-q8/"), "{k} 没放在 weetodd-q8/ 下：{v}");
            assert!(v.ends_with("-q8-paged"), "{k} 不是分页目录：{v}");
        }
    }

    /// 视频 VAE 用的是 **conv** 那份，不是同名的 `-video-vae-bf16`。
    /// 两个文件都在盘上、名字只差一个词，写错不报错。
    #[test]
    fn 视频vae用的是conv那份() {
        let g = build_graph(&p(5.0, &[], 1));
        assert_eq!(g["1"]["inputs"]["video_vae"], "ltx-2.5-video-vae-conv-bf16.safetensors");
    }

    /// LoRA 串成链，且 `Generate` 取的是链的**出口**。
    ///
    /// 接错了 LoRA 就静默不生效 —— 图照出、任务照样 success。
    #[test]
    fn lora串成链且generate取链尾() {
        let loras = vec![("ltx25/a.safetensors".to_string(), 1.0), ("ltx25/b.safetensors".to_string(), 0.6)];
        let g = build_graph(&p(5.0, &loras, 1));
        assert_eq!(g["900"]["inputs"]["model"], json!(["3", 0]), "第一个 LoRA 该接 Preflight");
        assert_eq!(g["901"]["inputs"]["model"], json!(["900", 0]), "第二个该串在第一个后面，不是并联");
        assert_eq!(g["901"]["inputs"]["strength"], 0.6);
        assert_eq!(g["4"]["inputs"]["model"], json!(["901", 0]), "Generate 必须取链尾");

        // 没有 LoRA 时直连 Preflight
        let g0 = build_graph(&p(5.0, &[], 1));
        assert_eq!(g0["4"]["inputs"]["model"], json!(["3", 0]));
        assert!(g0.get("900").is_none());
    }

    /// 超过 5 秒要分窗口，而窗口数算错要么截短要么白等。
    /// 分段提示词的三条铁律，每条都有实测撑着（见 `window_prompts` 的注释）。
    #[test]
    fn 每段提示词都完整重述不变部分() {
        let base = "固定机位中景，老茶师坐在茶桌后，年轻人背对镜头在右前景。";
        let acts: Vec<String> = ["他提壶注水。", "他放回铜壶。", "他抬眼点头。"]
            .iter().map(|s| s.to_string()).collect();
        let p = window_prompts(base, &acts, 3);
        assert_eq!(p.len(), 3);
        for (i, one) in p.iter().enumerate() {
            assert!(one.starts_with(base), "第 {} 段没重述不变部分：{one}", i + 1);
            assert!(one.contains(acts[i].as_str()), "第 {} 段动作丢了", i + 1);
        }
        // 三段各不相同 —— 全一样就退回原来那个会漂的做法了
        assert_eq!(p.iter().collect::<std::collections::HashSet<_>>().len(), 3);

        // 段数给少了沿用最后一个动作，**不能留空**（空提示词那段没有条件输入）
        let p4 = window_prompts(base, &acts[..2], 4);
        assert!(p4[2].contains("他放回铜壶。") && p4[3].contains("他放回铜壶。"));
        assert!(p4.iter().all(|x| !x.trim().is_empty()));

        // 没给动作就退回全段同一句（现状），不硬造
        let none = window_prompts(base, &[], 3);
        assert_eq!(none, vec![base.to_string(); 3]);
    }

    #[test]
    fn 窗口数按五秒一段算() {
        assert_eq!(window_count_for(3.0), 1);
        assert_eq!(window_count_for(5.0), 1);
        assert_eq!(window_count_for(5.1), 2);
        assert_eq!(window_count_for(10.0), 2);
        assert_eq!(window_count_for(15.0), 3);
    }

    /// 接续走的是另一个节点，而且每段都得有提示词。
    ///
    /// 空字符串**不等于**「继续上一段」—— 那一段会没有条件输入。
    #[test]
    fn 接续用另一个节点且每段都有提示词() {
        let g = build_graph(&p(15.0, &[], 3));
        assert_eq!(g["4"]["class_type"], CHAINED_NODE);
        assert_eq!(g["4"]["inputs"]["window_count"], 3);
        assert_eq!(g["4"]["inputs"]["overlap_frames"], OVERLAP_FRAMES);
        for i in 1..=3 {
            let v = g["4"]["inputs"][format!("prompt_{i}")].as_str().unwrap();
            assert!(!v.is_empty(), "第 {i} 段没有提示词");
        }
        assert_eq!(g["4"]["inputs"]["prompt_4"], "", "只有三段时第四段该留空");

        // 单段走普通节点
        let s = build_graph(&p(5.0, &[], 1));
        assert_eq!(s["4"]["class_type"], PROBE_NODE);
        assert!(s["4"]["inputs"].get("window_count").is_none());
    }

    /// 两种形态的输出节点都是 `"4"` —— 取结果那段按这个找文件。
    #[test]
    fn 输出节点固定是4() {
        for (d, w) in [(5.0, 1), (15.0, 3)] {
            let g = build_graph(&p(d, &[], w));
            assert!(g["4"]["inputs"].get("filename_prefix").is_some());
        }
    }
}
