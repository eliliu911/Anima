//! 本地 ComfyUI 文生图/设计工具 (Generate Image Tool)
//!
//! 为 Agent 提供开箱即用的图像生成能力（Logo、UI 资产、概念草图、插画）：
//! 1. 自动探查本地 ComfyUI (127.0.0.1:8188) 是否就绪与可用 Checkpoint（优先选用 SDXL/RealVisXL）；
//! 2. 构造标准 SDXL 扩散工作流（采样、正负向提示词、分辨率裁剪）；
//! 3. 产出图片安全落盘至工作区的 `.anima/generated/`，返回极简 Markdown 图片引用。

use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{json, Value};

use super::tool::{ToolCallRequest, ToolFailure, ToolFailureKind, ToolOutcome, ToolSpec, ToolStatus};
use super::{RuntimeContext, ToolHandler, ToolKind};
use crate::agent_runtime::lora;
use crate::agent_runtime::RuntimeEvent;
use crate::mcp::Operation;

const DEFAULT_COMFY_URL: &str = "http://127.0.0.1:8188";

/// 估算队列等待时用的单张耗时（秒）。本机 SDXL 1024 / 20 步实测约 30s。
/// 估算比不估好，但措辞上要让人看得出这是估的。
/// 队列预估和轮询上限的每张耗时。
///
/// **必须按引擎取**，不能写死。三个引擎实测 45s / 331s / 39s，差 8.5 倍 ——
/// 用固定值给 Chroma 估时会告诉用户「预计还要等约 60 秒」，
/// 而真实等待是 6 分钟以上，用户会以为卡死了。
///
/// 这里只能用**自己这一张**的引擎：排在前面的任务是谁提交的、用什么引擎，
/// ComfyUI 的 `/queue` 不告诉我们。所以这是个下界，不是精确值 ——
/// 宁可估短了让用户多等一会儿，也别估长了吓退他。
fn secs_per_image_job(engine: crate::agent_runtime::image_engine::ImageEngine) -> u64 {
    engine.secs_per_job()
}

/// **生成本身不设时长上限，看 ComfyUI 的实际进度**（见 [`super::comfy_watch`]）。
///
/// 这里先后是 40 分钟、2 小时，两次都在任务还在跑时报了超时：2026-09-29 Qwen 2.1 出首帧挂 8 张参考图，
/// 一步 85~96 秒、25 步约 37 分钟（不挂图约 5 分钟）—— 参考图越多越慢、不是线性的，定多少都会有一次撞上。
/// 出图的节点每一步都报进度，只要还在报就一直等。
///
/// **排队也不算。** 同一天按场出首帧，排在后面的镜头光排队就等满 40 分钟被判失败 ——
/// 而它们在 ComfyUI 里好好地排着、随后就出图了。排队只有一个很宽的上限（每轮 2 秒 → 12 小时），只防死循环。
pub const IMAGE_PENDING_CEILING_POLLS: u32 = 12 * 3600 / 2;

/// **registry 给整个工具的超时 —— 只管对话里调的那条**（分镜出首帧直接调 `execute`，不经过 registry，没有这一层）。
///
/// 这里出过一次事，而且很难查：`with_timeout` 原来是 **120 秒**，
/// 于是模型一次要三张图时**每次都是第三张失败**——
///
/// | | ComfyUI 完成时刻 | 该调用已耗时 |
/// |---|---|---|
/// | 第 1 张 | ~46s | 46s ✅ |
/// | 第 2 张 | ~91s | 91s ✅ |
/// | 第 3 张 | ~136s | **136s > 120s ❌ 被 registry 掐掉** |
///
/// 三个工具调用**并发启动、各自的 120 秒同时开始跑**，而 ComfyUI 是
/// 单实例串行执行的 —— 排在第三位的那个必然等超时。而且它在 ComfyUI 那边
/// 是 `success` 的，图就在磁盘上，用户看到的是「这张没生成出来」。
///
/// 更隐蔽的是：轮询循环自己的上限是 40 分钟，**那个数根本到不了** ——
/// registry 在 120 秒就把整个 future 丢掉了。下面那条测试就是为此加的。
///
/// 对话里等一张图不该等上几个小时，所以这里仍是 2 小时（和视频工具的 `VIDEO_TOOL_TIMEOUT_SECS` 同一量级）。
pub const IMAGE_TOOL_TIMEOUT_SECS: u64 = 2 * 3600 + 60;

/// 把 LoRA 的告警**顶到返回的最前面**。
///
/// 顶到前面是因为前端会按预算截断工具返回 —— 缀在结尾的话，模型下一轮根本
/// 看不到自己的 LoRA 名字写错了，只会一遍遍猜。这和 Seed 要前置是同一个道理。
pub fn with_lora_warning(summary: String, warning: Option<String>) -> String {
    match warning {
        Some(w) => format!("⚠️ {w}\n\n{summary}"),
        None => summary,
    }
}

/// CLIP 的文本窗口是 **77 token 一块**，超出的部分 ComfyUI 会切成多块再合并。
/// 块数越多，每一块（包括写着"学院风校服"的那一块）的权重就被摊得越薄 ——
/// 提示词不是越长越准，过了某个长度是**越长越糊**。
///
/// 实测（seed 228334280 / 同 LoRA / 同设定卡参数，只改提示词长短）：
///
/// | 提示词 | 结果 |
/// |---|---|
/// | 1075 字的 Markdown 长文 | 服装变成白背心白长裤，金色刺绣 / 玉带 / 珍珠耳钉全丢 |
/// | 同样内容压到约 120 字关键词 | 学院风校服、及腰长发、金边领口都在 |
///
/// 中文在 CLIP 的 BPE 里走字节回退，**一个汉字约 2~3 个 token**，
/// 所以同样字数的中文比英文更容易超窗。
const CLIP_CHUNK_TOKENS: usize = 77;

/// 估算提示词占多少个 CLIP token。不求精确，只要能把"明显超窗"和"正常"分开。
pub fn estimate_clip_tokens(prompt: &str) -> usize {
    prompt
        .chars()
        .map(|c| if c.is_ascii() { 1 } else { 5 }) // 非 ASCII 按 2~3 token/字，除以约 2.5 个字符宽度
        .sum::<usize>()
        / 5
        * 2
}

/// 步数明显超出引擎设计点时给个告警。`None` 表示正常。
///
/// 出过事：模型按 SDXL 的习惯给 Z-Image 传了 `steps: 20`，而它是**蒸馏 turbo
/// 模型，设计点就是 8 步**。实测 1024x1024 从 45 秒变成 121 秒 —— 多花的
/// 2.5 倍时间**完全是白花的**，画质不会更好，超过设计点还可能更差。
///
/// 只告警不改数：用户可能真的想要那个步数。但必须让他知道代价。
pub fn steps_warning(
    engine: crate::agent_runtime::image_engine::ImageEngine,
    steps: u32,
) -> Option<String> {
    use crate::agent_runtime::image_engine::{default_steps, ImageEngine};
    let d = default_steps(engine);
    if steps <= d {
        return None;
    }
    let ratio = steps as f32 / d as f32;
    if engine == ImageEngine::ZImage {
        // 蒸馏模型：超过设计点是纯亏，任何超出都值得说
        Some(format!(
            "步数 {steps} 超过 Z-Image 的设计点 {d}。它是蒸馏 turbo 模型，\
             加步数不会更好、还可能更差，但耗时按倍数涨（实测 8→20 步：45 秒变 121 秒）。\
             下次别传 steps，让工具按引擎给。"
        ))
    } else if ratio >= 1.5 {
        Some(format!(
            "步数 {steps} 是 {} 默认值 {d} 的 {ratio:.1} 倍，耗时也会按这个倍数涨。",
            engine.label()
        ))
    } else {
        None
    }
}

/// 提示词长到会被 CLIP 摊薄时给个告警。`None` 表示长度正常。
pub fn prompt_dilution_warning(prompt: &str) -> Option<String> {
    let est = estimate_clip_tokens(prompt);
    let chunks = est.div_ceil(CLIP_CHUNK_TOKENS);
    (chunks > 3).then(|| {
        format!(
            "提示词偏长（约 {est} 个 CLIP token，会被切成 {chunks} 段合并），\
             细节会被摊薄：服装、配饰这类具体描述最容易先丢。\
             建议压成 150 字以内的**逗号分隔关键词**，别写 Markdown 长文 —— \
             把最重要的（人种 / 发型 / 服装 / 画风）放前面。"
        )
    })
}

/// 角色设定卡的版式方案：一次扩散出整排多视图，而不是分别出图再拼。
///
/// 三件事必须一起给，缺一样都出不来整排（而且**不报错**，只是出一张普通单人图）：
///
/// 1. **版式触发词**当前缀 —— LoRA 的训练标签，不写它 LoRA 不激活
/// 2. **版式 LoRA** —— `CharTurnXLRetag` 那一类
/// 3. **宽画幅** —— 1024x1024 塞不下三个人
///
/// 触发词和 LoRA 选择都复用 `cinema_sheet`，不在这里另写一份 ——
/// 两处各写一份的话，改了一处另一处会静默漂掉。
pub struct SheetLayout {
    pub prompt_prefix: &'static str,
    /// 追加到正向末尾。**只在用户没点名画风时**才含写实画风词 ——
    /// 服装锚点则任何情况下都有，见 [`sheet_layout`]。
    pub positive_suffix: String,
    /// **必须点名的负向词。**
    ///
    /// 版式 LoRA（CharTurnXLRetag）是 3D 角色设定图训练的，会把画风往
    /// 塑料人偶拽。通用负向（blurry / deformed / ugly…）拦不住它。
    ///
    /// 实测：同一个 seed、同一个 checkpoint、**同样 0.55 权重**，
    /// 只加下面这串负向词，结果就从塑料人偶变成写实照片。
    /// 一度以为要靠调低权重（试过 0.55 / 0.40 / 0.28），
    /// **三档都写实** —— 说明根本不是权重的问题。
    pub negative_extra: String,
    pub lora: Option<String>,
    pub lora_weight: f32,
    pub width: u32,
    pub height: u32,
}

/// 请求里要的是不是设定卡版式。
/// 这次调用要不要出场景宫格。
///
/// 返回视角表 —— 版式和切分都由它决定。
/// 逐格精修：把布局稿放大补细节。
///
/// # 为什么是**再调一次这个工具**，不是另写一条提交链路
///
/// 工具本身已经支持 `image_path` + `denoise` 的 i2i，而且带着引擎选择、
/// LoRA、超时、进度上报这一整套。另写一条等于把那些全复制一遍 ——
/// 两份真相，而且新的那份没人测。cinema 出首帧也是这么调的。
///
/// # 成本
///
/// 每格一次出图。九格在本机是 45s × 9 ≈ 7 分钟 ——
/// **所以默认关**，而且关着的时候也要在结果里说它存在。
async fn refine_cells(
    ctx: &RuntimeContext,
    cells: &[std::path::PathBuf],
    views: &[crate::agent_runtime::scene_sheet::Viewpoint],
    style: &str,
    engine: Option<&str>,
) -> (Vec<String>, Vec<String>) {
    use crate::agent_runtime::scene_sheet::refine_plan;
    let mut ok = Vec::new();
    let mut failed = Vec::new();
    for (i, cell) in cells.iter().enumerate() {
        let Ok(img) = image::open(cell) else {
            failed.push(format!("第 {} 格读不出来", i + 1));
            continue;
        };
        let plan = refine_plan(img.width(), img.height(), 1024, 0.35);
        let desc = views.get(i).map(|v| v.description.as_str()).unwrap_or("");
        let name = views.get(i).map(|v| v.name.as_str()).unwrap_or("");
        let mut args = serde_json::json!({
            "prompt": crate::agent_runtime::scene_sheet::refine_prompt(style, desc),
            "image_path": cell.to_string_lossy(),
            "denoise": plan.denoise,
            "width": plan.width,
            "height": plan.height,
            "output_filename": format!(
                "{}_refined.png",
                cell.file_stem().and_then(|s| s.to_str()).unwrap_or("cell")
            ),
        });
        if let Some(e) = engine {
            args["engine"] = serde_json::json!(e);
        }
        let out = GenerateImageTool
            .execute(ctx, ToolCallRequest::new("generate_image", args))
            .await;
        // **一格失败不拖垮其余的。** 九格里坏一格还剩八格能用，
        // 整批失败等于把一个部分可用的结果变成完全没有。
        if out.status == ToolStatus::Ok {
            match out.artifacts.first() {
                Some(p) => ok.push(p.clone()),
                None => failed.push(format!("第 {} 格没有返回文件", i + 1)),
            }
        } else {
            failed.push(format!(
                "第 {} 格{}：{}",
                i + 1,
                if name.is_empty() { String::new() } else { format!("（{name}）") },
                out.failure.map(|f| f.message).unwrap_or(out.content)
            ));
        }
    }
    (ok, failed)
}

/// 宫格标题里用的场景名。
///
/// 优先取调用方给的 `scene_name`；没给就从提示词开头截一段 ——
/// **不编一个名字**，截出来的至少是用户自己写的字。
fn scene_name_from(args: &Value, prompt: &str) -> String {
    if let Some(n) = args.get("scene_name").and_then(|v| v.as_str()) {
        let n = n.trim();
        if !n.is_empty() {
            return n.to_string();
        }
    }
    prompt
        .split(['，', ',', '。', '\n'])
        .next()
        .unwrap_or(prompt)
        .trim()
        .chars()
        .take(20)
        .collect()
}

pub fn wants_scene_sheet(args: &Value) -> Option<Vec<crate::agent_runtime::scene_sheet::Viewpoint>> {
    let is = args
        .get("layout")
        .and_then(|v| v.as_str())
        .is_some_and(|m| m.eq_ignore_ascii_case("scene_sheet"));
    is.then(|| crate::agent_runtime::scene_sheet::viewpoints_from_args(args.get("viewpoints")))
}

/// 这次要出的场景在资产库里有没有空间契约。
///
/// 读不到配置目录、场景名对不上、场景没契约 —— 任何一条不成立都返回
/// `None`，调用方回落到它自己的默认。**这条路不能挡住出图。**
fn scene_contract_of(args: &Value, prompt: &str) -> Option<crate::scene_contract::SceneContract> {
    let dir = dirs::config_dir()?.join("com.anima.desktop");
    crate::assets::scene_contract_by_name(&dir, &scene_name_from(args, prompt))
}

/// 请求里要的是不是 **360° 全景 → 切四方位**。
///
/// 见 [`crate::agent_runtime::panorama`]。和宫格是两条路：宫格**求**模型
/// 画四个不同机位，这条是从一张球面投影里**算**出来的。
pub fn wants_pano_views(args: &Value) -> bool {
    args.get("layout")
        .and_then(|v| v.as_str())
        .map(|m| m.eq_ignore_ascii_case("pano_views"))
        .unwrap_or(false)
}

pub fn wants_character_sheet(args: &Value) -> bool {
    args.get("layout")
        .and_then(|v| v.as_str())
        .map(|m| m.eq_ignore_ascii_case("character_sheet"))
        .unwrap_or(false)
}

/// 请求里要的是不是**完整设定表**（七区 entity sheet）。
///
/// 和 `character_sheet` 是两条完全不同的路 —— 见
/// [`crate::agent_runtime::entity_sheet`] 的模块注释。这条**必须有源图**。
pub fn wants_entity_sheet(args: &Value) -> Option<crate::agent_runtime::entity_sheet::SheetKind> {
    args.get("layout")
        .and_then(|v| v.as_str())
        .and_then(crate::agent_runtime::entity_sheet::SheetKind::from_id)
}

/// 组装版式方案。`lora` 为 `None` 表示本机没装版式 LoRA ——
/// 那时仍然给触发词和宽画幅，效果会差一些但不至于完全失败。
/// 转身卡训练集的默认长相是**裸体人体参考**（人体扫描素材）。
/// 提示词里写了衣服也压不住，服装锚点必须在版式这一层无条件顶回去。
const SHEET_CLOTHED_ANCHOR: &str = "fully clothed, wearing complete outfit";

/// 同上：版式 LoRA 自带裸体先验，nsfw 兜底不能只靠用户自己写负向词。
const SHEET_NSFW_GUARD: &str = "nsfw, nude, naked, topless, undressed";

/// 提示词里有没有**明确的画风意图**（3D / 人偶 / 动漫 / 插画……）。
///
/// 有的话设定卡就不再硬塞写实画风词。否则工具会把用户点名要的画风
/// 写进自己的负向词里，用户拿到一张「生成成功、画风正好相反」的图。
pub fn has_style_intent(prompt: &str) -> bool {
    let lower = prompt.to_lowercase();
    [
        // 英文
        "3d", "cgi", "render", "bjd", "doll", "figurine", "anime", "cartoon",
        "toon", "cel shad", "illustration", "stylized", "painterly",
        "unreal engine", "octane", "sculpt",
        // 中文（`to_lowercase` 不改变中文，放同一张表里查一遍就够）
        "人偶", "手办", "国漫", "二次元", "动漫", "卡通", "插画", "渲染", "三维",
    ]
    .iter()
    .any(|k| lower.contains(k))
}

pub fn sheet_layout(prompt: &str) -> SheetLayout {
    use crate::cinema_sheet::{SheetEngine, SHEET_SIZE, TURNAROUND_LORA_WEIGHT};
    // 用户点名了画风 → 一个画风词都不加，把画风交还给用户
    let styled = has_style_intent(prompt);
    SheetLayout {
        prompt_prefix: SheetEngine::Sdxl.layout_trigger(),
        positive_suffix: if styled {
            SHEET_CLOTHED_ANCHOR.to_string()
        } else {
            format!(
                "{SHEET_CLOTHED_ANCHOR}, photorealistic, raw photo, \
                 natural skin texture, studio lighting"
            )
        },
        negative_extra: if styled {
            SHEET_NSFW_GUARD.to_string()
        } else {
            format!("{SHEET_NSFW_GUARD}, 3d render, cgi, plastic, doll, cartoon, anime, toy figure")
        },
        lora: crate::cinema_sheet::find_turnaround_lora(),
        lora_weight: TURNAROUND_LORA_WEIGHT,
        width: SHEET_SIZE.0,
        height: SHEET_SIZE.1,
    }
}

/// SDXL 的默认负向提示词。
///
/// **手脚和多肢畸形必须点名，笼统的 `bad anatomy` 拦不住。**
///
/// 实测（同 seed / 同提示词 / 同 20 步，一个瑜伽单腿站姿）：
///
/// | 负向词 | 结果 |
/// |---|---|
/// | 只有 `bad anatomy` | 人悬空、支撑腿够不到地面、姿态不成立 |
/// | 加上下面这串具体的词 | 站姿正常、脚踩实 |
///
/// 这和「设定卡的塑料人偶靠 anti-3D 负向词而不是调 LoRA 权重」是同一回事：
/// SDXL 的负向词是**逐词**起作用的，抽象形容词覆盖不到具体的失败模式。
///
/// 手指仍然是最难的一环 —— 1024 尺寸下一只手只有几十个像素。负向词能减少
/// 多指/融指，根治要靠更高分辨率或局部重绘（见 docs/MEDIA_PIPELINES.md）。
pub fn default_negative_prompt() -> &'static str {
    "blurry, low quality, deformed, distorted, text errors, ugly, \
     bad anatomy, watermark, pixelated, \
     extra limbs, extra legs, extra arms, extra fingers, extra digits, \
     fused fingers, missing fingers, malformed hands, mutated hands, \
     poorly drawn hands, poorly drawn feet, malformed feet, \
     bad proportions, disfigured, mutation, cropped"
}

/// 把用户给的负向词**追加**在默认负向词之后，而不是替换掉它。
///
/// 之前这里是 `.unwrap_or(default_negative_prompt())`：模型只要自己传了
/// `negative_prompt`，上面那串调过的解剖负向词就整串消失，「三条腿」当场回来 ——
/// 而且不报任何错，图照出。用户传的负向词表达的是"再多躲开这些"，
/// 从来不是"把你调好的那套扔掉"。
/// 从 ComfyUI 报的列表里挑一个 SDXL checkpoint。
///
/// 优先用用户在设置里点名的那个，**但必须真的在列表里**。换了机器、删了文件、
/// 或者改了名之后还硬用那个字符串，ComfyUI 会直接拒掉整个任务，
/// 而用户看到的只是"生成失败"，完全不知道是配置指向了一个不存在的文件。
///
/// 没点名时按内置优先级。**这个顺序本身有倾向**：`RealVisXL` 排第一，
/// 那是欧美写实底模；做国风角色设定卡时 `AWPortrait_XL` 明显更合适
/// （交领、纱质层次、玉簪更清楚，多视图之间的脸更一致）。
/// 所以可配置那条才是主路，这里只是兜底。
pub fn pick_sdxl_checkpoint(names: &[String], configured: Option<String>) -> String {
    if let Some(picked) = configured {
        if let Some(found) = names.iter().find(|n| **n == picked) {
            return found.clone();
        }
    }
    for preferred in ["RealVisXL", "AWPortrait_XL", "XL", "xl", "majicmix"] {
        if let Some(found) = names.iter().find(|n| n.contains(preferred)) {
            return found.clone();
        }
    }
    names
        .first()
        .cloned()
        .unwrap_or_else(|| "RealVisXL_V5.0_fp16.safetensors".to_string())
}

pub fn merge_negative_prompt(user: Option<&str>) -> String {
    let default_neg = default_negative_prompt();
    match user.map(str::trim).filter(|s| !s.is_empty()) {
        Some(u) => format!("{default_neg}, {u}"),
        None => default_neg.to_string(),
    }
}

/// 姿势控制（ControlNet OpenPose）。
///
/// # 为什么需要它 —— 提示词治不了姿势
///
/// 实测：把提示词写到极致（点名 vrksasana、说清哪条腿站立、脚放哪、
/// 甚至写 "exactly two legs and two arms"），4 个种子里只有 1 张是要求的
/// 树式，其余是**手倒立、前屈举腿、弓步** —— 完全另外的动作。
/// 换采样器、加步数都不改变这一点。
///
/// SDXL 对长姿势描述的遵从度本来就很低，再好的措辞也只是在掷骰子。
/// 给一根骨架才是硬约束：肢体数量和位置由 ControlNet 定死，模型没有发挥余地。
/// 实测同一根骨架生成出来的图**逐关节对齐**，而且腿数正确。
///
/// # 顺带治多肢
///
/// 「三条腿」这类畸形也归它管 —— 骨架里只有两条腿，模型画不出第三条。
/// （注意：多肢本身是**偶发**的，实测 4 个种子里未必复现，
/// 所以别把「换了配置就好了」当成修复 —— 见 `docs/MEDIA_PIPELINES.md`。）
pub struct PoseControl {
    /// 已放进 ComfyUI input 目录的姿势参考图文件名
    pub pose_filename: String,
    /// ControlNet 权重文件名
    pub controlnet: String,
    pub strength: f64,
    pub end_percent: f64,
}

/// 在可用的 ControlNet 里挑一个能做姿势的。
///
/// 只信 ComfyUI 报的枚举，不猜文件名 —— 和 `lora.rs`、`pick_video_unet`
/// 同一条规矩。
pub fn pick_pose_controlnet(available: &[String]) -> Option<String> {
    for kw in ["openpose", "union", "promax"] {
        if let Some(hit) = available.iter().find(|n| n.to_lowercase().contains(kw)) {
            return Some(hit.clone());
        }
    }
    None
}

/// 拉取 ComfyUI 认得的 ControlNet 文件名。
/// 参考生图要用的图。`reference_images` 是主来源；没给时退回 `image_path` ——
/// 调用方（画布、对话）常把唯一一张图放在 `image_path` 里，
/// 参考生图引擎拿它当第一张参考，而不是报「没有参考图」。
///
/// 只留**真实存在的文件**，顺序保持调用方给的顺序（场景在前、人物在后）。
pub fn reference_image_paths(args: &Value, workspace_root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let resolve = |raw: &str| {
        let clean = raw.trim().trim_start_matches("file://");
        let p = std::path::Path::new(clean);
        if p.is_absolute() { p.to_path_buf() } else { workspace_root.join(clean) }
    };
    let mut raw: Vec<String> = args
        .get("reference_images")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str()).map(str::to_string).collect())
        .unwrap_or_default();
    if raw.is_empty() {
        if let Some(p) = args.get("image_path").and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty()) {
            raw.push(p.to_string());
        }
    }
    raw.iter().map(|r| resolve(r)).filter(|p| p.is_file()).collect()
}

/// 参考图比槽位多时的那句话。**挑剩下的要说出来** —— 不说的话用户以为那个人也锁住了。
pub fn reference_overflow_note(engine: crate::agent_runtime::image_engine::ImageEngine, given: usize) -> Option<String> {
    let slots = engine.reference_slots();
    (slots > 0 && given > slots).then(|| {
        format!(
            "给了 {given} 张参考图，{} 只用前 {slots} 张（场景在前、人物在后），后面 {} 张没用上。",
            engine.label(),
            given - slots
        )
    })
}

/// 这个引擎要的节点里，ComfyUI 没加载的那几个；以及它要的模型文件里缺的那几个。
///
/// **缺什么都不能照跑**：缺编辑节点出一张普通图、缺 GELU 修复出纯黑图，
/// 任务状态都是 success。
async fn missing_nodes(client: &reqwest::Client, engine: crate::agent_runtime::image_engine::ImageEngine) -> Vec<String> {
    use crate::agent_runtime::image_engine::{self as eng, ImageEngine};
    let (nodes, files): (Vec<&str>, Vec<(&str, &str, &str)>) = match engine {
        ImageEngine::Krea2Edit => (
            vec!["Krea2EditModelPatch", "Krea2EditGroundedEncode"],
            vec![
                ("UNETLoader", "unet_name", eng::KREA2_UNET),
                ("LoraLoaderModelOnly", "lora_name", eng::KREA2_EDIT_LORA),
            ],
        ),
        ImageEngine::Qwen21 | ImageEngine::Qwen21Edit => (
            // `TextEncodeQwenImage21` 要 ComfyUI ≥ v0.37；`UnetLoaderGGUF` 要 ComfyUI-GGUF 插件。
            // 两个都缺就直接停 —— 缺了照跑是「图被 ComfyUI 拒掉」，而任务那头只看到一句
            // 看不懂的校验错。
            // `AnimaGgufArch` 是**我们自己的补丁节点**（`comfyui-nodes/anima_gguf_arch`）。
            // 下的这份 GGUF 一个 KV 元数据都没有，ComfyUI-GGUF 只好按张量名猜架构，
            // 而它那张表里没有 Qwen —— 缺了这个补丁，`UnetLoaderGGUF` 每次都抛
            // `Unknown model architecture!`。查节点在不在，比看盘上有没有文件可靠：
            // 它能出现在 `/object_info` 里，说明**正在跑的这个进程**真的补上了。
            vec!["TextEncodeQwenImage21", "UnetLoaderGGUF", GGUF_ARCH_NODE],
            vec![
                ("UnetLoaderGGUF", "unet_name", eng::QWEN21_UNET),
                ("CLIPLoader", "clip_name", eng::QWEN21_CLIP),
                ("VAELoader", "vae_name", eng::QWEN21_VAE),
            ],
        ),
        _ => (vec![], vec![]),
    };
    let mut out = Vec::new();
    for n in nodes {
        if !crate::agent_runtime::weetodd::node_available(client, n).await {
            out.push(format!("节点 {n}"));
        }
    }
    for (class, field, want) in files {
        // 列表拿不到（ComfyUI 没起来）时不在这里下结论 —— 提交那一步会报连不上。
        if let Some(list) = comfy_option_list(client, class, field).await {
            if !list.iter().any(|x| x == want) {
                out.push(format!("模型文件 {want}"));
            }
        }
    }
    out
}

/// 修 MPS 半精度 tanh-GELU 吐 NaN 的那个节点注册的类名。
/// 它能出现在 `/object_info` 里，就说明**跑着的这个 ComfyUI 进程**真的加载了补丁 ——
/// 比读日志可靠：日志会随重启改名，文件在不代表进程加载过。
pub const GELU_FIX_NODE: &str = "AnimaMpsGeluFix";
/// 让 ComfyUI-GGUF 认得出 Qwen-Image 2.1 的补丁节点（`comfyui-nodes/anima_gguf_arch`）。
pub const GGUF_ARCH_NODE: &str = "AnimaGgufArch";

fn missing_engine_nodes_message(engine: crate::agent_runtime::image_engine::ImageEngine, missing: &[String]) -> String {
    let mut msg = format!("{} 跑不了，ComfyUI 里缺：{}。", engine.label(), missing.join("、"));
    if missing.iter().any(|m| m.contains(GELU_FIX_NODE)) {
        msg.push_str(
            "\n缺 GELU 修复时在这台 Mac 上会出**纯黑图而且显示成功**。\
             确认 ComfyUI/custom_nodes/anima_mps_gelu_fix 这个链接在（指向仓库 comfyui-nodes/），然后重启 ComfyUI。",
        );
    }
    if missing.iter().any(|m| m.contains(GGUF_ARCH_NODE)) {
        msg.push_str(
            "\n缺它时 `UnetLoaderGGUF` 会直接报 `Unknown model architecture!` ——              那份 GGUF 没带架构元数据，ComfyUI-GGUF 自己的猜测表里没有 Qwen。             确认 ComfyUI/custom_nodes/anima_gguf_arch 这个链接在（指向仓库 comfyui-nodes/），然后重启 ComfyUI。",
        );
    }
    if missing.iter().any(|m| m.contains("Krea2Edit")) {
        msg.push_str("\n编辑节点来自 comfyui-krea2edit 节点包。");
    }
    if missing.iter().any(|m| m.starts_with("模型文件")) {
        msg.push_str("\n模型文件的下载地址和核对方法记在 docs/ASSET_LEDGER.md 2026-09-13 那一节。");
    }
    msg
}

/// ComfyUI 某个加载器节点的某个下拉选项列表（模型文件名）。拿不到返回 None。
async fn comfy_option_list(client: &reqwest::Client, class: &str, field: &str) -> Option<Vec<String>> {
    let url = format!("{DEFAULT_COMFY_URL}/object_info/{class}");
    let json = client.get(&url).timeout(std::time::Duration::from_secs(10)).send().await.ok()?.json::<Value>().await.ok()?;
    json.pointer(&format!("/{class}/input/required/{field}/0"))
        .and_then(|v| v.as_array())
        .map(|l| l.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
}

pub async fn available_controlnets(client: &reqwest::Client) -> Vec<String> {
    let url = format!("{DEFAULT_COMFY_URL}/object_info/ControlNetLoader");
    let Ok(resp) = client.get(&url).send().await else {
        return Vec::new();
    };
    let Ok(json) = resp.json::<Value>().await else {
        return Vec::new();
    };
    json.pointer("/ControlNetLoader/input/required/control_net_name/0")
        .and_then(|v| v.as_array())
        .map(|l| l.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

/// 把姿势控制的节点插进图里，并返回改写后的正/负条件出口。
///
/// 节点 id 从 800 起 —— 图本身用个位数、LoRA 链用 900 起，不会撞。
pub fn insert_pose_control(
    workflow: &mut Value,
    pose: &PoseControl,
    positive: Value,
    negative: Value,
) -> (Value, Value) {
    let Some(map) = workflow.as_object_mut() else {
        return (positive, negative);
    };
    let node = |t: &str, i: Value| json!({"class_type": t, "inputs": i});
    map.insert("800".into(), node("LoadImage", json!({"image": pose.pose_filename})));
    // DWPreprocessor 比 OpenposePreprocessor 更准，手和脸也一起认
    map.insert(
        "801".into(),
        node(
            "DWPreprocessor",
            json!({"image": ["800", 0], "detect_hand": "enable",
                   "detect_body": "enable", "detect_face": "enable", "resolution": 832}),
        ),
    );
    map.insert(
        "802".into(),
        node("ControlNetLoader", json!({"control_net_name": pose.controlnet})),
    );
    map.insert(
        "803".into(),
        node(
            "ControlNetApplyAdvanced",
            json!({"positive": positive, "negative": negative, "control_net": ["802", 0],
                   "image": ["801", 0], "strength": pose.strength,
                   "start_percent": 0.0, "end_percent": pose.end_percent}),
        ),
    );
    (json!(["803", 0]), json!(["803", 1]))
}

pub struct GenerateImageTool;

impl GenerateImageTool {
    async fn find_best_checkpoint(client: &reqwest::Client) -> String {
        let object_info_url = format!("{}/object_info/CheckpointLoaderSimple", DEFAULT_COMFY_URL);
        if let Ok(resp) = client.get(&object_info_url).send().await {
            if let Ok(json) = resp.json::<Value>().await {
                if let Some(list) = json
                    .pointer("/CheckpointLoaderSimple/input/required/ckpt_name/0")
                    .and_then(|v| v.as_array())
                {
                    let names: Vec<String> = list
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect();

                    return pick_sdxl_checkpoint(&names, Self::configured_sdxl_checkpoint());
                }
            }
        }
        "RealVisXL_V5.0_fp16.safetensors".to_string()
    }

    /// 设置面板里配的**设定卡专用** LoRA。没配返回空。
    ///
    /// 只在 `layout: character_sheet` 时读 —— 别的路径不受影响。
    /// 这是刻意的：LoRA 只在 SDXL 上有效，而默认引擎是 Z-Image，
    /// 全局常挂会在那条路上静默失效。设定卡那条路本来就强制 SDXL。
    fn configured_sheet_loras() -> Vec<lora::RequestedLora> {
        let Some(dir) = dirs::config_dir() else {
            return Vec::new();
        };
        let Ok(raw) = std::fs::read_to_string(dir.join("com.anima.desktop/config.json")) else {
            return Vec::new();
        };
        let Ok(v) = serde_json::from_str::<Value>(&raw) else {
            return Vec::new();
        };
        Self::configured_loras_field(&v, "sheet_loras")
    }

    /// 设置里配的 Z-Image 常挂 LoRA。
    fn configured_zimage_loras() -> Vec<lora::RequestedLora> {
        let Some(dir) = dirs::config_dir() else {
            return Vec::new();
        };
        let Ok(raw) = std::fs::read_to_string(dir.join("com.anima.desktop/config.json")) else {
            return Vec::new();
        };
        let Ok(v) = serde_json::from_str::<Value>(&raw) else {
            return Vec::new();
        };
        Self::configured_loras_field(&v, "zimage_loras")
    }

    fn configured_loras_field(v: &Value, field: &str) -> Vec<lora::RequestedLora> {
        v.get(field)
            .and_then(|x| x.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|it| {
                        let name = it.get("name")?.as_str()?.trim().to_string();
                        if name.is_empty() {
                            return None;
                        }
                        Some(lora::RequestedLora {
                            name,
                            strength: it.get("strength").and_then(|s| s.as_f64()).unwrap_or(0.6),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 设置面板里点名的 SDXL checkpoint。没点名返回 `None`。
    fn configured_sdxl_checkpoint() -> Option<String> {
        let dir = dirs::config_dir()?.join("com.anima.desktop");
        let raw = std::fs::read_to_string(dir.join("config.json")).ok()?;
        let v: Value = serde_json::from_str(&raw).ok()?;
        v.get("sdxl_checkpoint")
            .and_then(|x| x.as_str())
            .map(str::to_string)
            .filter(|s| !s.trim().is_empty())
    }

    /// 设置面板里选的默认底模。读不到就返回 `None`，交给 `pick_engine` 用它的默认。
    ///
    /// 直接读 `config.json` 而不是走 `RuntimeContext` —— 后者不带 config，
    /// 为一个字符串给它加通路要动所有构造点。配置损坏时**不能挡住出图**，
    /// 所以这里所有失败都吞掉返回 `None`。
    fn configured_engine() -> Option<String> {
        let dir = dirs::config_dir()?.join("com.anima.desktop");
        let raw = std::fs::read_to_string(dir.join("config.json")).ok()?;
        let v: Value = serde_json::from_str(&raw).ok()?;
        v.get("image_engine")
            .and_then(|x| x.as_str())
            .map(str::to_string)
            .filter(|s| !s.trim().is_empty())
    }

    async fn emit_progress(
        ctx: &RuntimeContext,
        request: &ToolCallRequest,
        prompt: &str,
        width: u32,
        height: u32,
        engine: &str,
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

        ctx.emit(RuntimeEvent::ImageGenerationProgress {
            turn_id: ctx.ids.turn_id.clone(),
            call_id,
            prompt: prompt.to_string(),
            width,
            height,
            engine: engine.to_string(),
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

#[async_trait]
impl ToolHandler for GenerateImageTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "generate_image",
            "Generate high-quality images, app logos, UI assets, concept sketches, or illustrations using the local ComfyUI SDXL diffusion pipeline. The generated image will be saved directly into the workspace's `.anima/generated/` directory and returned as a markdown image reference.",
            ToolKind::BuiltIn,
            Operation::Write,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "Detailed prompt describing the image. Write it in the user's language for the default engine (Qwen-Image 2.1 reads Chinese natively; so do Z-Image and Krea 2); write English only when the call will run on SDXL (pose_image, character_sheet, SDXL LoRAs). **Required on every call, including refinements of an earlier image** — the engine has no memory of the previous prompt, so always write the complete description of what the final image should be (e.g. 'minimalist modern app logo, sleek letter A with soft glowing gradients, clean vector style, premium 8k, centered on dark background')"
                },
                "negative_prompt": {
                    "type": "string",
                    "description": "Elements or styles to avoid (e.g. 'blurry, low quality, deformed, messy text, artifacts')"
                },
                "width": {
                    "type": "integer",
                    "default": 1024,
                    "description": "Image width in pixels (typically 1024 for square logos, 768 for portrait, 1152 for landscape)"
                },
                "height": {
                    "type": "integer",
                    "default": 1024,
                    "description": "Image height in pixels (typically 1024 for square logos, 1024 for portrait, 768 for landscape)"
                },
                "steps": {
                    "type": "integer",
                    "description": "Number of diffusion sampling steps. **Omit this.** Each engine has its own correct value and the tool fills it in: Z-Image Turbo uses 8, SDXL 20, Krea 2 Turbo 8, Qwen-Image 2.1 25. Z-Image is a *distilled turbo* model whose design point is 8 steps - raising it to an SDXL-style 20 makes the image take 2.5x longer (45s becomes 121s, measured) without improving it, and can make it worse. Only set this if the user explicitly asks for a specific step count."
                },
                "image_path": {
                    "type": "string",
                    "description": "Optional source image path to modify, refine, or transform (Image-to-Image). If omitted, standard text-to-image is performed. **`prompt` is still required** — it is the conditioning for the new render, not a delta. When refining, repeat the FULL description of the desired result with your changes folded in, not just the change ('now with cold pale skin' alone loses everything else)."
                },
                "keep_mask_path": {
                    "type": "string",
                    "description": "Path to a mask image marking what to PRESERVE (white = keep). Requires `image_path`. Everything outside the white area is regenerated from `prompt` while the masked pixels stay untouched. This is how you put a real product into a new advertising scene: the product must not change at all, which plain `image_path` + `denoise` can never guarantee (low denoise leaves the scene unchanged, high denoise repaints the product). Prefer `auto_mask` unless you already have a hand-made mask."
                },
                "auto_mask": {
                    "type": "boolean",
                    "default": false,
                    "description": "Cut the subject out automatically (BiRefNet) and preserve it, regenerating everything else from `prompt`. Requires `image_path`. Use this for product advertising shots: the product's pixels are preserved exactly (measured: 0.8% of pixels changed inside the silhouette, which is VAE round-trip noise) while background, lighting and any headline text are generated fresh. Beats a hand-drawn rectangle mask, which preserves the original background around the subject and leaves a visible box."
                },
                "denoise": {
                    "type": "number",
                    "default": 1.0,
                    "description": "Denoising strength (0.0 - 1.0). When image_path is provided, 0.5-0.75 modifies the image while keeping its overall structure/composition; 1.0 completely redraws it."
                },
                "pose_image": {
                    "type": "string",
                    "description": "Path to a reference image whose POSE should be copied (ControlNet OpenPose). Use this whenever the user asks for a specific body pose — SDXL largely ignores written pose descriptions (measured: a fully explicit tree-pose prompt produced handstands, forward folds and lunges across 4 seeds), and a skeleton is a hard constraint that also prevents extra limbs. Only the skeleton is copied; appearance, clothing and background still come from `prompt`."
                },
                "pose_strength": {
                    "type": "number",
                    "default": 0.85,
                    "description": "How strictly to follow the pose reference (0.0 - 1.0). Lower it toward 0.6 if the pose looks stiff or fights the prompt."
                },
                "engine": {
                    "type": "string",
                    "enum": ["zimage", "sdxl", "krea2", "krea2_edit", "qwen21", "qwen21_edit"],
                    "default": "qwen21",
                    "description": "Which base model renders the image. **Omit this parameter. Do not set it unless the user names an engine in their own words, or explicitly asks for maximum quality and says they will wait.** Picking a slower engine on your own initiative is a mistake: it multiplies the wait, and when you issue several images at once it makes them wait for the slowest one. The default is `qwen21` (see below) unless the user picked another engine in Settings. `zimage` (Z-Image Turbo) takes ~45s and is the fast choice - use it when the user asks for speed or many quick drafts. `sdxl` is chosen automatically and cannot be avoided when the request needs a LoRA, `pose_image`, or `layout: character_sheet`, because those weights only exist for the SDXL architecture. `krea2` (Krea 2 Turbo, int8) takes ~108s and is the strongest at prompt adherence and photo-realism - measured on the same prompt, it got an exact object count right that `zimage` got wrong - and its Qwen3-VL encoder reads Chinese natively; pick it for a final image, or when the user asks for maximum realism or exact adherence and accepts the wait. Note that `negative_prompt` has NO effect on `zimage` or `krea2` (both run at cfg=1, which cancels the negative branch out); they still produce correct hands without one. `krea2_edit` and `qwen21_edit` are REFERENCE engines: they take existing images via `reference_images` (scene first, then characters) and draw a NEW composition in which those people and that location look the same - use them only when the user wants the output to match existing asset images. `krea2_edit` (~200s, 2 references) keeps faces closest; `qwen21_edit` (~260s, up to 10 references) is the same weights as `qwen21` and takes by far the most references - use it when the shot needs more than three assets kept consistent. Note that with `qwen21_edit` the OUTPUT aspect ratio follows the FIRST reference image, not the requested size. `qwen21` (Qwen-Image 2.1, uncensored Q8_0 GGUF) is THE DEFAULT: a text-to-image base model with a Qwen3-VL-8B encoder that reads Chinese natively, takes long prompts, renders exact Chinese and English text on the image, and keeps one identity consistent across many panels (measured on the same character-sheet prompt: it wrote the Chinese name correctly where `zimage` and `krea2` both garbled it). It has no content filter. It takes ~211s at 1024x1024. It cannot edit an existing image: calls with `image_path` automatically fall back to `zimage`. `negative_prompt` has NO effect on it either (cfg=1)."
                },
                "reference_images": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Only for `engine: krea2_edit` or `qwen21_edit`. Paths of existing images to keep consistent. ORDER MATTERS: put the scene/location image first, then character images. Extra images beyond the engine's slots (2 for krea2_edit, 10 for qwen21_edit) are dropped and the result says which."
                },
                "allow_nsfw": {
                    "type": "boolean",
                    "description": "Only for `pony` / `pony_realistic`. Default false adds safety tags, because Pony checkpoints were measured producing nudity from an entirely ordinary prompt. Set true only when the user explicitly asks for adult content."
                },
                "refine": {
                    "type": "boolean",
                    "default": false,
                    "description": "Only for `layout: scene_sheet`. After splitting, run a second img2img pass on every cell to raise it from layout draft to usable image. Costs one more generation PER CELL (about 45s each on this machine with the default engine, so a 2x2 sheet is ~3 minutes and a 3x3 is ~7), which is why it is off by default. Turn it on when the cells are meant to be used as references or keyframes rather than just to check the layout."
                },
                "viewpoints": {
                    "type": "array",
                    "description": "Only for `layout: scene_sheet`. The views of this one location that this episode actually needs - derive them from the shots, do not produce a complete set of compass directions for its own sake. Each item is either a plain name (\"全景\") or {\"name\", \"description\"}. Give the name real meaning: it becomes the split file name and how the view is referenced later, so \"视角1\" or \"参考\" is unusable downstream. Omitted: a conservative default set is used. More than 9 views are paginated into several sheets.",
                    "items": {"type": ["string", "object"]}
                },
                "layout": {
                    "type": "string",
                    "enum": ["normal", "character_sheet", "entity_sheet", "quad_view", "scene_sheet"],
                    "default": "normal",
                    "description": "`character_sheet` produces ONE wide image containing multiple views of the same character in a row (front / side / back, full body). Use it when the user wants a 三视图 / 设定卡 / turnaround sheet, It is for reading and design review only: do NOT feed a whole sheet into `animate_image` / H3 as the identity reference — each face on it is only ~40 px and H3 cannot lock identity from that; use the single-person main image instead. It auto-applies the turnaround layout LoRA and its trigger words; just describe the character (appearance, clothing, hair), not the layout.\n\n`entity_sheet` is a different, richer thing and **requires `image_path`** — it converts an EXISTING image of the subject into one full reference sheet with seven zones: a metadata column, a large hero view, a front/side/back turnaround row, three action poses, a silhouette study, an expression study, and six identity-critical detail crops. Use it when the user wants a 完整设定表 / 角色设定图 with expressions and detail close-ups, and already has an image of the subject. It forces the Krea 2 engine and its own layout LoRA, takes about 7.5 minutes, and outputs 1536x1024. Two caveats measured on this machine: the turnaround row inside it is only ~110px per head (too small to use as an identity reference — use `character_sheet` for that), and the on-sheet body text renders as scribble (short English headers are fine, prose is not).\n\nWrite the prompt for `entity_sheet` as the bracketed template the layout LoRA was trained on. If you are unsure of the format, omit the parameter and let the caller build it.\n\n`scene_sheet` produces ONE grid image holding several views of the SAME location, then splits it into individual files. Use it for scene consistency: the cells come out of a single sampling pass, so lighting, materials and furniture are necessarily the same across views - which no amount of prompting can guarantee when you generate them one at a time. Pass the views you actually need via `viewpoints`; do not ask for a complete set of directions for its own sake. A new view is only worth a cell when it changes the entrances, the visible geography, the light source, the occlusions or the narrative function. IMPORTANT: on this machine a 3x3 grid gives about 512x341 per cell, which reads as a LAYOUT DRAFT (furniture placement and light direction are trustworthy, material detail is not) - the tool says so in its result. Refine each cell with img2img before treating it as final."
                },
                "loras": crate::agent_runtime::lora::schema_fragment(),
                "seed": {
                    "type": "integer",
                    "description": "Random seed. To REFINE a previously generated image (change a detail, adjust wording), pass back the exact seed reported by that image's result — same seed + edited prompt keeps the same subject and composition. Omit it only when you want a genuinely different image."
                },
                "output_filename": {
                    "type": "string",
                    "description": "Optional specific filename (without directory), e.g. 'app_logo.png'"
                }
            },
            "required": ["prompt"]
        }))
        .with_timeout(Duration::from_secs(IMAGE_TOOL_TIMEOUT_SECS))
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
                // **失败信息要能自我纠正。**
                //
                // 实测踩到过：模型在改图时只传了 `image_path` + `seed` + `denoise`，
                // 以为"微调上一张"不用再给提示词。原来的信息只有干巴巴一句
                // "缺少 prompt 参数"，模型不知道该补什么、也不知道要补完整而不是补增量。
                ToolFailure::new(
                    ToolFailureKind::InvalidParams,
                    "缺少 `prompt` 参数。**每一次调用都要有**，包括在已有图片上改细节 —— \
                     SDXL 不记得上一次的提示词，`prompt` 是这次渲染的全部条件而不是增量。\
                     改图时请把**完整的目标描述**重写一遍并把改动融进去（只写\"改成冷白皮\"\
                     会丢掉发型、服装、构图等其余全部要求）。",
                ),
                Duration::from_millis(0),
            );
        };

        let started = Instant::now();
        // **超时不能按"服务空闲"来设。**
        //
        // ComfyUI 采样时事件循环被占住，`/object_info` 和 `/prompt` 都会拖很久。
        // 10 秒的时候实测撞到过：用户的请求正好撞上另一批任务在跑，
        // 请求**根本没到 ComfyUI**（history 里没有任何记录）就报了失败。
        // 用户自己在 ComfyUI 界面里跑东西时也是同一个场景。
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(120))
            .build()
            .unwrap_or_default();

        // **只有真的挂得上的 LoRA 才有资格决定引擎。**
        //
        // 模型会把提示词里的触发词当成 LoRA 名字传进来（实测："触发词 j1v_w0m"
        // → `loras: ["j1v_w0m"]`）。按名字判架构的话认不出、按保守规则算成 SDXL，
        // 于是引擎被拽回 SDXL；而后面真去解析时盘上没这个文件，又被丢掉 ——
        // **引擎掉了、LoRA 一个没挂**，界面上只剩一个 SDXL 徽章。
        //
        // 所以这里先和 ComfyUI 的枚举对一次名。只有请求里带了 LoRA 才发这个请求。
        let engine_deciding_loras: Vec<String> = {
            let asked: Vec<String> = crate::agent_runtime::lora::parse_requested(&request.arguments)
                .into_iter()
                .map(|l| l.name)
                .collect();
            if asked.is_empty() {
                Vec::new()
            } else {
                let avail = crate::agent_runtime::lora::available_loras(&client).await;
                crate::agent_runtime::lora::attachable_names(&asked, &avail)
            }
        };
        // 选底模。**必须在第一次 emit_progress 之前定下来** —— 骨架屏要显示
        // 用的是哪个引擎，而三个引擎耗时差 8.5 倍，用户得知道自己在等什么。
        //
        // SDXL 专属能力只看**参数里有没有**，不等到后面真去解析 ——
        // 那时候第一条进度早就发出去了，改不了。
        // **完整设定表自成一路**：它有自己的引擎（Krea 2）、自己的 LoRA
        // （版式 LoRA 写死在图里）、自己的采样器。让它参与 SDXL 回落判定的话，
        // 「有源图」这一条会把它拽走，而那条路根本出不了七区版式。
        let sheet_kind = wants_entity_sheet(&request.arguments);
        let entity_sheet = sheet_kind.is_some();
        // **参考生图引擎也自成一路。** 它们本身就吃图 —— 要是让「有 image_path」这一条
        // 参与 SDXL 回落判定，会被当成「这个引擎不支持图生图」而静默换成 SDXL，
        // 参考图全部作废。
        let asked_reference_engine = request
            .arguments
            .get("engine")
            .and_then(|v| v.as_str())
            .and_then(crate::agent_runtime::image_engine::ImageEngine::from_id)
            .filter(|e| e.reference_slots() > 0);
        let sdxl_only = if entity_sheet || asked_reference_engine.is_some() {
            crate::agent_runtime::image_engine::SdxlOnly::default()
        } else {
        crate::agent_runtime::image_engine::SdxlOnly {
            character_sheet: wants_character_sheet(&request.arguments),
            pose_image: request.arguments.get("pose_image").and_then(|v| v.as_str())
                .is_some_and(|s| !s.trim().is_empty()),
            // **只有非 Z-Image 的 LoRA 才逼着回落 SDXL。**
            //
            // 以前是"一见 LoRA 就回落"—— 那条规则当初没错（手上全是 SDXL 权重），
            // 但代价是为了挂 LoRA 放弃 Z-Image 的长提示词和画质。现在社区有了
            // Z-Image 架构的 LoRA，两者可以兼得。认不出的一律当 SDXL，保持原行为。
            loras: crate::agent_runtime::image_engine::needs_sdxl_for_loras(&engine_deciding_loras),
            img2img: request.arguments.get("image_path").and_then(|v| v.as_str())
                .is_some_and(|s| !s.trim().is_empty()),
        }
        };
        // 优先级：本次调用的 `engine` 参数 > 设置面板里的默认值 > zimage。
        //
        // 配置在这里是**直接读文件**的，因为 `RuntimeContext` 不带 config ——
        // 为一个字符串给它加一条 config 通路，要动所有构造点，不划算。
        // 读失败就退回默认（`pick_engine` 的 None 分支），不让配置坏掉挡住出图。
        let configured = Self::configured_engine();
        let choice = crate::agent_runtime::image_engine::pick_engine(
            request
                .arguments
                .get("engine")
                .and_then(|v| v.as_str())
                .or(configured.as_deref()),
            sdxl_only,
        );
        // 强制 Krea 2 的两种情况，理由是同一个：**那些权重挂到别的引擎上
        // 加载「成功」、图照出、一点作用都没有。**
        //
        // 1. 设定表 —— 版式 LoRA 只有 Krea 2 这一版
        // 2. 这次要挂的 LoRA 里有 Krea 2 架构的
        let krea2_lora =
            crate::agent_runtime::image_engine::needs_krea2_for_loras(&engine_deciding_loras);
        // 参考生图引擎不吃用户 LoRA —— 为一条 Krea 2 LoRA 把用户选的参考生图换成
        // 普通 Krea 2，参考图就全作废了。
        let krea2_lora = krea2_lora && choice.engine.reference_slots() == 0;
        let engine = if entity_sheet || krea2_lora {
            crate::agent_runtime::image_engine::ImageEngine::Krea2
        } else {
            choice.engine
        };
        let engine_label = engine.label();
        let engine_note = if entity_sheet {
            None
        } else if krea2_lora && choice.engine != crate::agent_runtime::image_engine::ImageEngine::Krea2 {
            // **回落必须说出来** —— 静默换引擎就是骗人（`EngineChoice` 的注释）
            Some(format!(
                "已改用 Krea 2 出图（你要的是 {}）：这次挂的 LoRA 是 Krea 2 架构的，\
                 挂到别的引擎上会静默失效。",
                choice.engine.label()
            ))
        } else {
            choice.note
        };

        // **缺节点就在这里停住，不往下走。**
        //
        // `Krea2EditModelPatch` / `Krea2EditGroundedEncode` 来自
        // `comfyui-krea2edit`，不是 ComfyUI 核心。没有它们而我们照跑的话，
        // 出来一张**没有版式的普通图**，而任务状态是 success ——
        // 这个仓库最典型的失败形状。
        if entity_sheet {
            use crate::agent_runtime::entity_sheet as es;
            let mut missing: Vec<&'static str> = Vec::new();
            for n in es::REQUIRED_NODES {
                if !crate::agent_runtime::weetodd::node_available(&client, n).await {
                    missing.push(n);
                }
            }
            if !missing.is_empty() {
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(
                        ToolFailureKind::MissingDependency,
                        es::missing_nodes_message(&missing),
                    ),
                    started.elapsed(),
                );
            }
        }

        // ---- 参考生图 / Pony：跑之前把缺的东西查清楚，缺了就停下说清楚 ----
        //
        // 这台机器上这几条路的失败**全是静默的**：缺节点出一张普通图、
        // 缺 GELU 修复出纯黑图，任务状态都是 success。
        let reference_inputs: Vec<String> = if engine.reference_slots() > 0 {
            let paths = reference_image_paths(&request.arguments, &ctx.workspace_root);
            if paths.is_empty() {
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(
                        ToolFailureKind::InvalidParams,
                        format!(
                            "{} 至少要一张参考图（`reference_images`）—— 它是照着参考图出新构图的，\
                             没有图就没有东西可参考。场景图放第一张、人物图放后面。",
                            engine.label()
                        ),
                    ),
                    started.elapsed(),
                );
            }
            let missing = missing_nodes(&client, engine).await;
            if !missing.is_empty() {
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(ToolFailureKind::MissingDependency, missing_engine_nodes_message(engine, &missing)),
                    started.elapsed(),
                );
            }
            let mut names = Vec::new();
            for (i, src) in paths.iter().take(engine.reference_slots()).enumerate() {
                let fname = format!("ref_{}_{i}.png", uuid::Uuid::new_v4().simple());
                let mut copied = false;
                for input_dir in crate::agent_runtime::video_tool::find_comfyui_input_dirs() {
                    copied |= std::fs::copy(src, input_dir.join(&fname)).is_ok();
                }
                if !copied {
                    return ToolOutcome::failed(
                        &request,
                        ToolFailure::new(
                            ToolFailureKind::Execution,
                            format!("参考图没能放进 ComfyUI 的 input 目录：{}", src.display()),
                        ),
                        started.elapsed(),
                    );
                }
                names.push(fname);
            }
            names
        } else {
            Vec::new()
        };
        // **挑剩下的参考图要说出来** —— 不说的话用户以为锁住了，其实那个人没进图。
        let reference_note = reference_overflow_note(
            engine,
            reference_image_paths(&request.arguments, &ctx.workspace_root).len(),
        );
        // 文生图那一档也要查：Qwen 2.1 少了 GGUF 架构补丁节点，每一次都跑不起来。
        // （参考生图那一档在上面收参考图时已经查过了。）
        if engine.reference_slots() == 0 {
            let missing = missing_nodes(&client, engine).await;
            if !missing.is_empty() {
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(ToolFailureKind::MissingDependency, missing_engine_nodes_message(engine, &missing)),
                    started.elapsed(),
                );
            }
        }

        // 设定卡版式：触发词 + 版式 LoRA + 宽画幅，三件事一起生效
        let sheet = wants_character_sheet(&request.arguments).then(|| sheet_layout(prompt));
        // 场景宫格：视角表决定版式和切分。**和设定卡是两条路** ——
        // 它不挂版式 LoRA、不锁引擎，产物是一张图切成 N 张。
        // 这个场景在资产库里的空间契约（文字地图）。只有宫格这条路用得上。
        let scene_contract = wants_scene_sheet(&request.arguments)
            .is_some()
            .then(|| scene_contract_of(&request.arguments, prompt))
            .flatten();
        let scene_views = wants_scene_sheet(&request.arguments).map(|fallback| {
            // **调用方没点名视角时，优先用空间契约派生。**
            //
            // 手写视角表有两个毛病：换个人写就散了，而且没有任何东西保证
            // 几个视角说的是同一份地理 —— 实测撞过「全景」和「反打」
            // 出成几乎同一个机位，光向材质全对、柜台却只在其中一格里有。
            // 契约的 View 自带「两两关系 + 视线终点」，那正是区分开机位的东西。
            //
            // 调用方明确传了 `viewpoints` 就听它的：那是它这一次真正要的机位，
            // 契约是兜底不是覆盖。
            if request.arguments.get("viewpoints").and_then(|v| v.as_array()).is_some() {
                return fallback;
            }
            match scene_contract.as_ref().map(|c| c.viewpoints()) {
                Some(v) if !v.is_empty() => v,
                _ => fallback,
            }
        });
        let scene_grid = scene_views
            .as_ref()
            .map(|v| crate::agent_runtime::scene_sheet::grid_for(v.len()));
        let (default_w, default_h) = if entity_sheet {
            // 七个区是按 3:2 排的，方图/竖图会挤成一团（有测试钉着横幅）
            (
                crate::agent_runtime::entity_sheet::SHEET_WIDTH,
                crate::agent_runtime::entity_sheet::SHEET_HEIGHT,
            )
        } else {
            match (&sheet, scene_grid) {
                (Some(s), _) => (s.width, s.height),
                // 宫格默认出宽幅 —— 格子越接近正方越好读，
                // 而 1536×1024 是本机 Z-Image 的常用尺寸（3×3 每格 512×341）
                (None, Some(_)) => (1536, 1024),
                (None, None) => (1024, 1024),
            }
        };
        let width = request
            .arguments
            .get("width")
            .and_then(|v| v.as_u64())
            .unwrap_or(default_w as u64) as u32;
        let height = request
            .arguments
            .get("height")
            .and_then(|v| v.as_u64())
            .unwrap_or(default_h as u64) as u32;
        // 回显给模型的是**它自己传的那一段**，不是拼上版式前缀后的那一段 ——
        // 下一轮它要修改的正是自己的参数，工具会再把前缀拼回去。
        let original_prompt = prompt;
        let sheet_prompt;
        let grid_prompt_buf;
        let prompt: &str = match (&sheet, &scene_views) {
            (Some(s), _) => {
                sheet_prompt = format!("{} {prompt}, {}", s.prompt_prefix, s.positive_suffix);
                &sheet_prompt
            }
            // 宫格提示词把**版式写在最前** —— 出图模型对「画一张 N 行 M 列的
            // 网格」要先建立整体版式，写在后面会被场景描述带偏成一张普通图。
            // 用户写的那段当场景描述用。
            (None, Some(views)) => {
                let grid = scene_grid.unwrap_or(crate::agent_runtime::scene_sheet::Grid { rows: 1, cols: 1 });
                let name = scene_name_from(&request.arguments, prompt);
                // **文字地图排在用户那段描述前面。**
                //
                // 地理是这张图要锁的东西，气氛是可以浮动的东西；提示词越靠后
                // 越容易被稀释，所以先说地理。契约里已经写了空场政策和
                // 「不要多出来的门」，`grid_prompt` 自己那几句是兜底。
                let desc_buf;
                let desc: &str = match scene_contract.as_ref() {
                    Some(c) => {
                        // **名字传空。** 场景板上不需要知道这地方叫什么，
                        // 而名字是一大段中文描述里唯一被引号括起来的专名 ——
                        // Z-Image 认得中文也画得出中文，它会当招牌画上去。
                        // 实测四格里有两格把「安和面馆」写在了门楣上，
                        // 而提示词里当时有三道禁令。见 `scene_sheet::grid_prompt`。
                        desc_buf = format!("{}\n{prompt}", c.to_prompt_block(""));
                        &desc_buf
                    }
                    None => prompt,
                };
                grid_prompt_buf =
                    crate::agent_runtime::scene_sheet::grid_prompt(&name, desc, views, grid);
                &grid_prompt_buf
            }
            (None, None) => prompt,
        };
        // 默认步数按引擎给 —— Z-Image 是 Turbo 蒸馏模型，8 步是它的设计点，
        // 沿用 SDXL 的 20 步不会更好，只会白等 2.5 倍时间。
        let steps = request
            .arguments
            .get("steps")
            .and_then(|v| v.as_u64())
            .unwrap_or(crate::agent_runtime::image_engine::default_steps(engine) as u64)
            as u32;

        Self::emit_progress(
            ctx,
            &request,
            prompt,
            width,
            height,
            engine_label,
            "connecting",
            "正在连接 ComfyUI",
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
                width,
                height,
                engine_label,
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

        // checkpoint 探测只对 SDXL 有意义 —— 另外两个引擎是 UNETLoader + 独立
        // 文本编码器，不走 CheckpointLoaderSimple。给它们探测等于白打一次网络请求，
        // 而且返回的名字会让 summary 显示成一个根本没参与的模型。
        let ckpt_name = if engine == crate::agent_runtime::image_engine::ImageEngine::Sdxl {
            Self::find_best_checkpoint(&client).await
        } else {
            engine.label().to_string()
        };
        Self::emit_progress(
            ctx,
            &request,
            prompt,
            width,
            height,
            engine_label,
            "model",
            "正在准备模型与提示词",
            0.10,
            None,
            None,
            Some(&ckpt_name),
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
        // 负向提示词的写法**逐引擎不同**，混用会静默变弱或完全失效：
        //
        // - SDXL：逗号标签，逐词起作用（那串解剖负向词就是这么调出来的）
        // - Z-Image：官方 cfg=1.0、负向走 `ConditioningZeroOut`，**结构性无效** ——
        //   传什么都不起作用。它照样能出对手，但别以为是负向词在保护。
        let user_neg = request.arguments.get("negative_prompt").and_then(|v| v.as_str());
        let merged_neg = merge_negative_prompt(user_neg);
        let base_neg: &str = &merged_neg;
        // 设定卡的 anti-3D 负向词**总是追加**，即使模型自己传了 negative_prompt ——
        // 它不会知道版式 LoRA 有这个画风偏置
        let sheet_neg;
        let neg_prompt: &str = match &sheet {
            Some(l) => {
                sheet_neg = format!("{base_neg}, {}", l.negative_extra);
                &sheet_neg
            }
            None => base_neg,
        };

        // 2. LoRA 通路。
        //
        // 名字要和 ComfyUI 的枚举对齐 —— 不猜文件名（`lora.rs` 的模块注释里
        // 记着为什么）。对不上就直接失败并把本机有哪些列回去，让模型改对再来，
        // 而不是悄悄不挂 LoRA 出一张图：那样用户拿到的是一张"看起来生成成功、
        // 但风格完全没生效"的图，而且不报任何错。
        let mut lora_warning: Option<String> = None;
        let mut requested_loras = lora::parse_requested(&request.arguments);
        // 设置面板里配的设定卡专用 LoRA，**只在设定卡这条路上加**。
        //
        // 排在模型自己传的之后、版式 LoRA 之前：版式决定构图必须最靠前，
        // 而用户在设置里配的是"这个项目的固定画风"，比模型临时点的更稳定。
        if sheet.is_some() {
            let configured = Self::configured_sheet_loras();
            for l in configured.into_iter().rev() {
                requested_loras.insert(0, l);
            }
        }
        // 设置面板里配的 Z-Image 常挂 LoRA。范围由 `should_apply_configured_zimage_loras`
        // 卡死（不是 Z-Image / 这次自己传了 / 图生图 / 设定卡，四种都不挂）。
        // **调用方明说「别挂」时就不挂。** 资产出图走的是这条 ——
        // 它传的 `loras` 是空数组，而空数组等于「没传」，于是设置里那两个
        // 常驻 LoRA 会被静默挂上。见 `studio::GenerateArgs::auto_loras`。
        let auto_loras = request
            .arguments
            .get("auto_loras")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        if auto_loras
            && eng::should_apply_configured_zimage_loras(
            engine,
            // **同样只算「挂得上的」。** 模型传了个盘上没有的名字（比如把提示词里的
            // 触发词当成 LoRA），那一条会被丢掉 —— 这时候还把它当成"模型点过名了"
            // 而不挂用户配置的，结果是一个 LoRA 都没有。
            !engine_deciding_loras.is_empty(),
            request.arguments.get("image_path").and_then(|v| v.as_str())
                .is_some_and(|s| !s.trim().is_empty()),
            sheet.is_some(),
        ) {
            requested_loras = Self::configured_zimage_loras();
        }
                // **全景那条路必须挂 360 LoRA，缺了当场停。**
        //
        // 不挂它出来的是一张普通宽图 —— 比例对、看着像图，但根本不是
        // 等距圆柱投影，切出来的四个角度全错，而且不报错。
        if wants_pano_views(&request.arguments) {
            use crate::agent_runtime::panorama::PANO_LORA;
            let have = lora::available_loras(&client).await;
            if !have.iter().any(|x| x == PANO_LORA) {
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(
                        ToolFailureKind::Execution,
                        format!(
                            "出 360° 全景要 `{PANO_LORA}`，本机没装。\n\
                             不挂它照样能出图，但那是一张普通宽图不是等距圆柱投影 —— \
                             切出来的四个角度会全错，而图看着是正常的，所以这里直接停。\n\
                             下载：scripts/fetch-model.sh civitai '2702227?type=Model&format=SafeTensor' \
                             360viewer_Redmond_zit.safetensors <ComfyUI>/models/loras/zimage"
                        ),
                    ),
                    started.elapsed(),
                );
            }
            requested_loras.insert(
                0,
                lora::RequestedLora { name: PANO_LORA.to_string(), strength: 1.0 },
            );
        }
        // 版式 LoRA 排在最前 —— 它决定构图，其余都叠在它后面
        if let Some(s) = &sheet {
            if let Some(name) = &s.lora {
                requested_loras.insert(
                    0,
                    lora::RequestedLora { name: name.clone(), strength: s.lora_weight as f64 },
                );
            }
        }
        let resolved_loras = if requested_loras.is_empty() {
            Vec::new()
        } else {
            {
                let (resolved, warn) =
                    lora::resolve_loras(&requested_loras, &lora::available_loras(&client).await);
                lora_warning = warn;
                resolved
            }
        };
        // 900 起：图本身只用到个位数节点 id，不会撞
        let chain = lora::build_chain(&resolved_loras, 900, json!(["1", 0]), Some(json!(["1", 1])));
        let model_ref = chain.model_ref.clone();
        let clip_ref = chain.clip_ref.clone().unwrap_or_else(|| json!(["1", 1]));
        let clip_ref2 = clip_ref.clone();

        let image_path_opt = request
            .arguments
            .get("image_path")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let denoise = request
            .arguments
            .get("denoise")
            .and_then(|v| v.as_f64())
            .unwrap_or(if image_path_opt.is_some() { 0.65 } else { 1.0 })
            .clamp(0.05, 1.0);

        // 如果提供了源图，验证路径并复制至 ComfyUI input 目录
        let mut i2i_input_filename: Option<String> = None;
        if let Some(image_path_raw) = image_path_opt {
            let clean_path = image_path_raw.trim_start_matches("file://");
            let source_path = if std::path::Path::new(clean_path).is_absolute() {
                std::path::PathBuf::from(clean_path)
            } else {
                ctx.workspace_root.join(clean_path)
            };

            if source_path.is_file() {
                let fname = format!("i2i_source_{seed}.png");
                for input_dir in crate::agent_runtime::video_tool::find_comfyui_input_dirs() {
                    let _ = std::fs::copy(&source_path, input_dir.join(&fname));
                }
                i2i_input_filename = Some(fname);
            }
        }

        // 姿势控制（ControlNet OpenPose）。提示词治不了姿势，骨架才是硬约束 ——
        // 见 `insert_pose_control` 的注释。
        let mut pose: Option<PoseControl> = None;
        if let Some(raw) = request
            .arguments
            .get("pose_image")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let clean = raw.trim_start_matches("file://");
            let src = if std::path::Path::new(clean).is_absolute() {
                std::path::PathBuf::from(clean)
            } else {
                ctx.workspace_root.join(clean)
            };
            if src.is_file() {
                match pick_pose_controlnet(&available_controlnets(&client).await) {
                    Some(cn) => {
                        let fname = format!("pose_ref_{seed}.png");
                        let mut copied = false;
                        for input_dir in crate::agent_runtime::video_tool::find_comfyui_input_dirs() {
                            copied |= std::fs::copy(&src, input_dir.join(&fname)).is_ok();
                        }
                        if copied {
                            pose = Some(PoseControl {
                                pose_filename: fname,
                                controlnet: cn,
                                strength: request
                                    .arguments
                                    .get("pose_strength")
                                    .and_then(|v| v.as_f64())
                                    .unwrap_or(0.85)
                                    .clamp(0.0, 1.0),
                                end_percent: 0.9,
                            });
                        }
                    }
                    None => {
                        // 用户点名要控姿势而机器上没有 ControlNet —— 说清楚，
                        // 别默默出一张姿势不对的图
                        return ToolOutcome::failed(
                            &request,
                            ToolFailure::new(
                                ToolFailureKind::Execution,
                                "要按参考图控制姿势需要 ControlNet 权重，但 ComfyUI 里一个都没有。\
                                 把 openpose 或 union 类的 SDXL ControlNet 放进 models/controlnet/ 再试。",
                            ),
                            started.elapsed(),
                        );
                    }
                }
            }
        }
        // 没挂 ControlNet 时条件直连文本编码器
        let (pos_ref, neg_ref) = (json!(["2", 0]), json!(["3", 0]));

        // 3. 组装工作流。
        //
        // 非 SDXL 的两条走 `image_engine` 里按 ComfyUI 官方模板写死的图 ——
        // 它们的采样拓扑和 SDXL 完全不同（Chroma 是 CFGGuider + BasicScheduler +
        // SamplerCustomAdvanced，不是 KSampler 一把梭），不能共用下面这段。
        use crate::agent_runtime::image_engine as eng;
        // 下面的 SDXL 分支会 move 掉 i2i_input_filename，先留一份给局部重绘用
        let inpaint_src = i2i_input_filename.clone();

        // 局部重绘：遮罩内保留、遮罩外重画。广告图的核心 ——
        // 商品必须一个像素不变，而整图重绘做不到这件事。
        let inpaint: Option<eng::MaskSource> = {
            let auto = request
                .arguments
                .get("auto_mask")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let manual = request
                .arguments
                .get("keep_mask_path")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            if (auto || manual.is_some()) && i2i_input_filename.is_none() {
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(
                        ToolFailureKind::InvalidParams,
                        "局部重绘需要 `image_path` —— 没有源图就没有可保留的东西。",
                    ),
                    started.elapsed(),
                );
            }
            match (manual, auto) {
                (Some(raw), _) => {
                    let p = if std::path::Path::new(raw).is_absolute() {
                        std::path::PathBuf::from(raw)
                    } else {
                        ctx.workspace_root.join(raw)
                    };
                    if !p.is_file() {
                        return ToolOutcome::failed(
                            &request,
                            ToolFailure::new(
                                ToolFailureKind::InvalidParams,
                                format!("遮罩图不存在：{}", p.display()),
                            ),
                            started.elapsed(),
                        );
                    }
                    let name = format!("keepmask_{seed}.png");
                    for d in crate::agent_runtime::video_tool::find_comfyui_input_dirs() {
                        let _ = std::fs::copy(&p, d.join(&name));
                    }
                    Some(eng::MaskSource::File(name))
                }
                (None, true) => Some(eng::MaskSource::Auto),
                _ => None,
            }
        };

        // 取结果时要按各引擎自己的 SaveImage 节点 id 找。写死 "7" 的话，
        // 换引擎会变成「任务 success、outputs 里有图、我们却取不到」。
        let save_node = if entity_sheet {
            crate::agent_runtime::entity_sheet::SAVE_NODE_ID
        } else {
            eng::save_node_id(engine)
        };
        // 非 SDXL 那三条：先按官方模板建文生图，有源图再改写成图生图。
        //
        // **改写是通用的**（见 `eng::attach_i2i`）—— 各引擎的采样拓扑差别很大，
        // 但要动的两处一样：空 latent 换成 `VAEEncode`、把 denoise 设上。
        // 以前这几条根本没接源图，于是 `SdxlOnly.img2img` 把它们一律回落 SDXL，
        // 而那不是因为模型不支持：图生图是采样器层面的事。
        let mut workflow = if entity_sheet {
            // 设定表：源图必须有（`need(1)` 在 studio 那层已经拦过一道，
            // 但对话里也能直接调这个工具，所以这里也要兜住）。
            let Some(src) = i2i_input_filename.clone() else {
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(
                        ToolFailureKind::InvalidParams,
                        "完整设定表要一张源图（`image_path`）—— 它是图生图，\
                         身份全部来自那张图。纯文字出不了这个版式。",
                    ),
                    started.elapsed(),
                );
            };
            use crate::agent_runtime::entity_sheet as es;
            let kind = sheet_kind.expect("entity_sheet 分支里 kind 一定有");
            let boost = request
                .arguments
                .get("ref_boost")
                .and_then(|v| v.as_f64())
                .unwrap_or(es::REF_BOOST_DEFAULT);
            // **模板只有一个所有者，就是 `es::sheet_prompt`。**
            //
            // 调用方（界面 / Agent）送零件，这里拼。已经是完整模板的话
            // 原样放过 —— Agent 想自己写那段方括号结构时不该被覆盖。
            let _arg_str = |k: &str| {
                request.arguments.get(k).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty())
            };
            // **拼装规则整块在 `es::sheet_prompt_from_args`**，不在这里。
            //
            // 之前这段写在这个 1200 行函数里，而测试只测了那个切分纯函数 ——
            // 变异测试证明了那是假绿：摘掉兜底照样全过。提出去之后调用点只剩一行，
            // 规则本身有测试真的盯着。
            let sheet_prompt = es::sheet_prompt_from_args(kind, prompt, &request.arguments);
            es::build_entity_sheet_graph(kind, &src, &sheet_prompt, boost, width, height, seed, steps)
        } else if engine == eng::ImageEngine::Krea2Edit {
            eng::build_krea2_edit_graph(prompt, width, height, seed, &reference_inputs)
        } else if engine == eng::ImageEngine::ZImage {
            eng::build_zimage_graph_with_loras(prompt, width, height, seed, steps, &resolved_loras)
        } else if engine == eng::ImageEngine::Krea2 {
            eng::build_krea2_graph_with_loras(prompt, width, height, seed, steps, &resolved_loras)
        } else if engine == eng::ImageEngine::Qwen21Edit {
            eng::build_qwen21_graph(prompt, neg_prompt, width, height, seed, steps, &reference_inputs)
        } else if engine == eng::ImageEngine::Qwen21 {
            eng::build_qwen21_graph(prompt, neg_prompt, width, height, seed, steps, &[])
        } else if let Some(input_fname) = i2i_input_filename.clone() {
            // 图生图 (I2I) 拓扑：LoadImage -> VAEEncode -> KSampler
            json!({
                "1": {
                    "class_type": "CheckpointLoaderSimple",
                    "inputs": {
                        "ckpt_name": ckpt_name
                    }
                },
                "2": {
                    "class_type": "CLIPTextEncode",
                    "inputs": {
                        "text": prompt,
                        "clip": clip_ref
                    }
                },
                "3": {
                    "class_type": "CLIPTextEncode",
                    "inputs": {
                        "text": neg_prompt,
                        "clip": clip_ref2
                    }
                },
                "4": {
                    "class_type": "LoadImage",
                    "inputs": {
                        "image": input_fname
                    }
                },
                "10": {
                    "class_type": "VAEEncode",
                    "inputs": {
                        "pixels": ["4", 0],
                        "vae": ["1", 2]
                    }
                },
                "5": {
                    "class_type": "KSampler",
                    "inputs": {
                        "model": model_ref,
                        "positive": pos_ref,
                        "negative": neg_ref,
                        "latent_image": ["10", 0],
                        "seed": seed,
                        "steps": steps,
                        "cfg": 7.0,
                        "sampler_name": "euler",
                        "scheduler": "normal",
                        "denoise": denoise
                    }
                },
                "6": {
                    "class_type": "VAEDecode",
                    "inputs": {
                        "samples": ["5", 0],
                        "vae": ["1", 2]
                    }
                },
                "7": {
                    "class_type": "SaveImage",
                    "inputs": {
                        "filename_prefix": "Anima_Generate",
                        "images": ["6", 0]
                    }
                }
            })
        } else {
            // 纯文生图 (T2I) 拓扑：EmptyLatentImage -> KSampler
            json!({
                "1": {
                    "class_type": "CheckpointLoaderSimple",
                    "inputs": {
                        "ckpt_name": ckpt_name
                    }
                },
                "2": {
                    "class_type": "CLIPTextEncode",
                    "inputs": {
                        "text": prompt,
                        "clip": clip_ref
                    }
                },
                "3": {
                    "class_type": "CLIPTextEncode",
                    "inputs": {
                        "text": neg_prompt,
                        "clip": clip_ref2
                    }
                },
                "4": {
                    "class_type": "EmptyLatentImage",
                    "inputs": {
                        "width": width,
                        "height": height,
                        "batch_size": 1
                    }
                },
                "5": {
                    "class_type": "KSampler",
                    "inputs": {
                        "model": model_ref,
                        "positive": pos_ref,
                        "negative": neg_ref,
                        "latent_image": ["4", 0],
                        "seed": seed,
                        "steps": steps,
                        "cfg": 7.0,
                        "sampler_name": "euler",
                        "scheduler": "normal",
                        "denoise": denoise
                    }
                },
                "6": {
                    "class_type": "VAEDecode",
                    "inputs": {
                        "samples": ["5", 0],
                        "vae": ["1", 2]
                    }
                },
                "7": {
                    "class_type": "SaveImage",
                    "inputs": {
                        "filename_prefix": "Anima_Generate",
                        "images": ["6", 0]
                    }
                }
            })
        };
        // **Z-Image 的图自己接 LoRA，这里不能再插一遍。**
        //
        // 这条链是按 SDXL 的拓扑接的（`CheckpointLoaderSimple` 的 `["1",0]`/`["1",1]`
        // 出 model 和 clip）。Z-Image 的节点 1 是 `UNETLoader`，压根没有第 1 号输出 ——
        // 用同一批节点 id 再插一次，等于把 `build_zimage_graph_with_loras` 接好的
        // 那份**覆盖成接错的**，而且 id 相同、不会有任何冲突提示。
        if !eng::graph_builds_own_lora_chain(engine) {
            if let Some(map) = workflow.as_object_mut() {
                for (id, node) in chain.nodes {
                    map.insert(id, node);
                }
            }
        }

        // 图生图：非 SDXL 那三条是按文生图模板建的，这里改写成图生图。
        //
        // **SDXL 那一支不走这里** —— 它的图在上面就是按 i2i 拓扑直接建的
        // （`CheckpointLoaderSimple` 出 VAE，没有 `VAELoader` 节点，
        // 通用改写认不出来）。
        //
        // 改写失败要**说出来**：那说明这张图不是我们以为的形状，
        // 而失败的样子是「源图静默没生效」—— 图照出、任务 success。
        // **参考生图引擎不走这里**：它们的 `image_path` 已经当成第一张参考图用了，
        // 再套这个改写会把目标 latent 换成那张图 —— 出来是参考图的轻微变体，不是新构图。
        if engine != eng::ImageEngine::Sdxl && engine.reference_slots() == 0 {
            if let Some(src) = i2i_input_filename.as_deref() {
                if !eng::attach_i2i(&mut workflow, src, denoise) {
                    return ToolOutcome::failed(
                        &request,
                        ToolFailure::new(
                            ToolFailureKind::Execution,
                            format!(
                                "{} 的图改不成图生图 —— 建图的模板变了。\
                                 换 SDXL，或者把这条报给开发。",
                                engine.label()
                            ),
                        ),
                        started.elapsed(),
                    );
                }
            }
        }

        // 局部重绘：把采样器的 latent 换成 `VAEEncodeForInpaint` 的输出。
        //
        // 放在姿势控制之前 —— 两者互不相干（一个换 latent、一个换 conditioning），
        // 但顺序固定下来比"碰巧不冲突"可靠。
        // **闸门是 `supports_i2i`，不是 `reference_slots`。** 局部重绘和图生图是同一个
        // 机制（把空 latent 换成 `VAEEncode*` 的输出），能不能做取决于这个引擎的 VAE
        // 吃不吃通用的像素编码 —— 那正是 `supports_i2i` 回答的问题。
        // 按 `reference_slots` 判的话，Qwen-Image 2.1（槽位 0）会被放行，
        // 然后出一张坏图、任务 success（它的 VAE 要四通道 RGBA，通用编码只喂三通道）。
        if inpaint.is_some() && !eng::supports_i2i(engine) {
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(
                    ToolFailureKind::InvalidParams,
                    format!("{} 不做局部重绘 —— 要保留商品像素请换 Z-Image 或 SDXL。", engine.label()),
                ),
                started.elapsed(),
            );
        }
        if let (Some(mask), Some(src)) = (&inpaint, &inpaint_src) {
            eng::apply_inpaint(&mut workflow, engine, src.as_str(), mask);
        }

        // 姿势控制的节点插在最后，然后把 KSampler 的条件连线改到 ControlNet 出口。
        // **必须改连线** —— 只插节点不改连线的话 ControlNet 是死的：图跑得通、
        // 任务成功、姿势完全不受控，而且不报任何错。
        // pose 会强制回落 SDXL（见 pick_engine），这里再挡一道 ——
        // 把 openpose 节点插进 Z-Image/Chroma 的图里不会报错，只是彻底不生效。
        if let Some(p) = pose.as_ref().filter(|_| engine == eng::ImageEngine::Sdxl) {
            let (pos, neg) = insert_pose_control(&mut workflow, p, json!(["2", 0]), json!(["3", 0]));
            if let Some(k) = workflow.get_mut("5").and_then(|n| n.get_mut("inputs")) {
                if let Some(o) = k.as_object_mut() {
                    o.insert("positive".into(), pos);
                    o.insert("negative".into(), neg);
                }
            }
        }

        // 3. 提交任务
        let prompt_url = format!("{}/prompt", DEFAULT_COMFY_URL);
        let submit_payload = json!({
            "prompt": workflow,
            "client_id": "anima_agent_image"
        });

        let resp = match client.post(&prompt_url).json(&submit_payload).send().await {
            Ok(r) => r,
            Err(e) => {
                Self::emit_progress(
                    ctx,
                    &request,
                    prompt,
                    width,
                    height,
                    engine_label,
                    "error",
                    "提交 ComfyUI 失败",
                    0.0,
                    None,
                    None,
                    Some(&e.to_string()),
                )
                .await;
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(
                        ToolFailureKind::Transport,
                        format!("向 ComfyUI 提交生图任务失败: {e}"),
                    ),
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
                width,
                height,
                engine_label,
                "error",
                "ComfyUI 拒绝任务",
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
                    format!("ComfyUI 拒绝生图任务: {err_body}"),
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
                    width,
                    height,
                    engine_label,
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
                width,
                height,
                engine_label,
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
        // **记账：这个任务已经交出去了。**
        //
        // 不记的话 `prompt_id` 只活在这个循环里，Anima 一关就没了 —— 而 ComfyUI
        // 那边还在跑。用户回到对话看不到任何东西，只好再说一次，于是又排一个新的。
        //
        // 销账只在**成功落盘**那一步做。失败/超时/被打断都故意留着记录，
        // 让开机对账去收 —— "Anima 这边报超时、ComfyUI 那边其实出图了"
        // 正是要救的那一类。
        super::comfy_jobs::remember(
            prompt_id,
            &ctx.ids.session_id,
            "image",
            save_node,
            &crate::agent_runtime::comfy_jobs::summarize(prompt),
        )
        .await;
        // 出首帧任务的续接单：应用中途关掉，重开时凭它去 ComfyUI 收图（`crate::render_resume`）。不在出首帧任务里什么也不做
        crate::jobs::note_prompt_id(prompt_id);

        // **提交后立刻报队列深度。**
        //
        // ComfyUI 单实例串行执行，同时提交只会排队。用户看不到队列时，
        // 排在别人后面和"卡住了"长得一模一样 —— 实测撞到过。
        let q = crate::comfyui::queue_state(&client, Some(prompt_id)).await;
        let queued_label = q
            .describe(secs_per_image_job(engine))
            .unwrap_or_else(|| "已提交 ComfyUI 队列".to_string());
        Self::emit_progress(
            ctx,
            &request,
            prompt,
            width,
            height,
            engine_label,
            "queued",
            &queued_label,
            0.16,
            Some(prompt_id),
            None,
            None,
        )
        .await;

        // 4. 轮询等待产出 (最多 90 秒)
        let history_url = format!("{}/history/{}", DEFAULT_COMFY_URL, prompt_id);
        let mut image_filename = String::new();
        let mut image_subfolder = String::new();
        let mut image_type = "output".to_string();

        // **轮询不设死上限 —— 判据是「ComfyUI 还认不认这个任务」。**
        //
        // 出过事：ComfyUI 实实在在跑了 121 秒把图出出来了，而这里的固定预算
        // 是 84 秒，于是用户看到「这张没生成出来 / ComfyUI 生成超时」，
        // 而磁盘上那张图是好的。**轮询预算用完 ≠ 任务失败。**
        //
        // 固定预算必然误判，因为实测的 45s / 331s / 39s 是**稳态值**，不包含：
        //
        // - 首次切引擎要加载 12~17GB 权重（可以比出图本身还久）
        // - 换分辨率（1024x1024 比 832x1216 的稳态基准更慢）
        // - 模型自己把 steps 调高
        // - 别人往同一个 ComfyUI 塞任务
        //
        // 所以只要它还在 `/queue` 里（running 或 pending），就继续等。
        // 连续多轮**既不在队列、也没有产出**，才算真的丢了。
        // 绝对上限只用来防死循环，给得很宽。
        let per = secs_per_image_job(engine) as u32;
        // 只用来估进度条，不再当作截止线
        let expected_polls = ((per / 2 + 20) + q.ahead.unwrap_or(0) as u32 * (per / 2 + 5)).max(1);
        /// 连续这么多轮「队列里没有、history 里也没有」才算任务真的丢了。
        /// 任务离开队列到 history 写完之间有一个空窗，不留容忍度会误判。
        const LOST_TOLERANCE: u32 = 5;
        let mut lost_streak: u32 = 0;
        // 排队只数轮次防死循环；在跑的看进度（见 `IMAGE_PENDING_CEILING_POLLS`）
        let mut pending_polls = 0u32;
        let mut watch = super::comfy_watch::tracker(prompt_id);
        let mut gave_up: Option<String> = None;
        let mut poll_index: u32 = 0;
        loop {
            if pending_polls >= IMAGE_PENDING_CEILING_POLLS {
                gave_up = Some(format!(
                    "任务 `{prompt_id}` 在 ComfyUI 队列里排了 12 小时还没轮到 —— 前面的任务可能卡住了，去 ComfyUI 看一眼队列。"
                ));
                break;
            }
            poll_index += 1;
            tokio::time::sleep(Duration::from_secs(2)).await;
            // 进度条按预期耗时估，但封顶在 0.86 —— 超出预期时继续等，
            // 进度条不该假装已经完成
            let estimated_progress =
                (0.18 + ((poll_index as f32) / expected_polls as f32) * 0.68).min(0.86);
            let mut label: &str = "ComfyUI 正在生成图像";
            let dyn_label;
            // 读不出队列状态 = 这一轮连不上 ComfyUI（连着太久才算它退了）
            let mut seen = super::comfy_watch::Seen::Unreachable;
            if let Ok(q_resp) = client
                .get(format!("{}/queue", DEFAULT_COMFY_URL))
                .send()
                .await
            {
                if let Ok(q_json) = q_resp.json::<Value>().await {
                    let is_pending = q_json
                        .get("queue_pending")
                        .and_then(|v| v.as_array())
                        .is_some_and(|items| items.iter().any(|item| item.to_string().contains(prompt_id)));
                    let is_running = q_json
                        .get("queue_running")
                        .and_then(|v| v.as_array())
                        .is_some_and(|items| items.iter().any(|item| item.to_string().contains(prompt_id)));
                    let qs = crate::comfyui::parse_queue(&q_json, Some(prompt_id));
                    seen = if is_pending && !is_running {
                        super::comfy_watch::Seen::Queued
                    } else {
                        super::comfy_watch::Seen::Running
                    };
                    // 只要 ComfyUI 还认这个任务，就重置"丢失"计数
                    if is_pending || is_running {
                        lost_streak = 0;
                    } else {
                        lost_streak += 1;
                    }
                    dyn_label = if is_pending {
                        // 说清排在第几，不要只说"正在等待"
                        qs.describe(secs_per_image_job(engine))
                            .unwrap_or_else(|| "正在等待 ComfyUI 队列".to_string())
                    } else if is_running {
                        match watch.describe() {
                            Some(p) => format!("ComfyUI 正在采样 · {p}"),
                            None => "ComfyUI 正在采样".to_string(),
                        }
                    } else {
                        "正在读取 ComfyUI 结果".to_string()
                    };
                    label = &dyn_label;
                }
            }
            Self::emit_progress(
                ctx,
                &request,
                prompt,
                width,
                height,
                engine_label,
                "running",
                label,
                estimated_progress,
                Some(prompt_id),
                None,
                None,
            )
            .await;

            if let Ok(h_resp) = client.get(&history_url).send().await {
                if let Ok(h_json) = h_resp.json::<Value>().await {
                    if let Some(err_msg) = h_json.pointer(&format!("/{prompt_id}/status/messages"))
                    {
                        let err_str = err_msg.to_string();
                        if err_str.contains("execution_error") {
                            Self::emit_progress(
                                ctx,
                                &request,
                                prompt,
                                width,
                                height,
                                engine_label,
                                "error",
                                "ComfyUI 节点执行报错",
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
                                    format!("ComfyUI 节点执行报错: {err_str}"),
                                ),
                                started.elapsed(),
                            );
                        }
                    }

                    if let Some(outputs) = h_json
                        .pointer(&format!("/{prompt_id}/outputs/{save_node}/images"))
                        .and_then(|v| v.as_array())
                    {
                        if let Some(img_info) = outputs.first() {
                            image_filename = img_info
                                .get("filename")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string();
                            image_subfolder = img_info
                                .get("subfolder")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string();
                            image_type = img_info
                                .get("type")
                                .and_then(|v| v.as_str())
                                .unwrap_or("output")
                                .to_string();
                            break;
                        }
                    }
                }
            }

            // 既不在队列、也没有产出，连续若干轮 —— 任务是真的丢了，
            // 再等下去只是让用户干等。
            if lost_streak >= LOST_TOLERANCE {
                break;
            }
            if seen == super::comfy_watch::Seen::Queued {
                pending_polls += 1;
            }
            if let Some(msg) = watch.observe(seen).explain(prompt_id) {
                gave_up = Some(match watch.describe() {
                    Some(p) => format!("{msg}（最后一次：{p}）"),
                    None => msg,
                });
                break;
            }
        }

        if image_filename.is_empty() {
            Self::emit_progress(
                ctx,
                &request,
                prompt,
                width,
                height,
                engine_label,
                "error",
                if gave_up.is_some() { "不再等了，任务可能仍在 ComfyUI 里跑" } else { "ComfyUI 未产出图片" },
                0.0,
                Some(prompt_id),
                None,
                None,
            )
            .await;
            // **两种情况的说法必须不一样。**
            //
            // 撞到绝对上限时，任务很可能还在跑、图随后就会出来 ——
            // 说成"生成失败"是错的，用户会以为白等了，实际上去 ComfyUI
            // 的历史里就能拿到。带上 prompt_id 才捞得回来。
            let msg = if let Some(msg) = gave_up {
                msg
            } else {
                format!(
                    "ComfyUI 未产出图片：任务 `{prompt_id}` 已经不在队列里，history 里也没有输出。\
                     去 ComfyUI 控制台看有没有报错。"
                )
            };
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(ToolFailureKind::Execution, msg),
                started.elapsed(),
            );
        }

        // 5. 获取图片二进制并保存到工作区目录
        let view_url = format!(
            "{}/view?filename={}&subfolder={}&type={}",
            DEFAULT_COMFY_URL, image_filename, image_subfolder, image_type
        );
        Self::emit_progress(
            ctx,
            &request,
            prompt,
            width,
            height,
            engine_label,
            "downloading",
            "正在下载生成图片",
            0.90,
            Some(prompt_id),
            None,
            Some(&image_filename),
        )
        .await;
        let img_bytes = match client.get(&view_url).send().await {
            Ok(r) => match r.bytes().await {
                Ok(b) => b,
                Err(e) => {
                    Self::emit_progress(
                        ctx,
                        &request,
                        prompt,
                        width,
                        height,
                        engine_label,
                        "error",
                        "读取图片内容失败",
                        0.0,
                        Some(prompt_id),
                        None,
                        Some(&e.to_string()),
                    )
                    .await;
                    return ToolOutcome::failed(
                        &request,
                        ToolFailure::new(
                            ToolFailureKind::Transport,
                            format!("读取图片内容失败: {e}"),
                        ),
                        started.elapsed(),
                    )
                }
            },
            Err(e) => {
                Self::emit_progress(
                    ctx,
                    &request,
                    prompt,
                    width,
                    height,
                    engine_label,
                    "error",
                    "下载图片失败",
                    0.0,
                    Some(prompt_id),
                    None,
                    Some(&e.to_string()),
                )
                .await;
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(ToolFailureKind::Transport, format!("下载图片失败: {e}")),
                    started.elapsed(),
                )
            }
        };

        let generated_dir = ctx.workspace_root.join(".anima").join("generated");
        if let Err(e) = std::fs::create_dir_all(&generated_dir) {
            Self::emit_progress(
                ctx,
                &request,
                prompt,
                width,
                height,
                engine_label,
                "error",
                "创建图片目录失败",
                0.0,
                Some(prompt_id),
                None,
                Some(&e.to_string()),
            )
            .await;
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(
                    ToolFailureKind::Execution,
                    format!("创建目标目录失败: {e}"),
                ),
                started.elapsed(),
            );
        }

        let file_name = request
            .arguments
            .get("output_filename")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| format!("img_{seed}.png"));
        let target_path = generated_dir.join(&file_name);

        if let Err(e) = std::fs::write(&target_path, img_bytes) {
            Self::emit_progress(
                ctx,
                &request,
                prompt,
                width,
                height,
                engine_label,
                "error",
                "保存图片到磁盘失败",
                0.0,
                Some(prompt_id),
                Some(&target_path.to_string_lossy()),
                Some(&e.to_string()),
            )
            .await;
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(
                    ToolFailureKind::Execution,
                    format!("保存图片到磁盘失败: {e}"),
                ),
                started.elapsed(),
            );
        }

        // 销账放在**落盘成功之后**。写盘失败的话记录留着，
        // 让开机对账再取一次 —— 结果在 ComfyUI 那边一直都在。
        super::comfy_jobs::finish(prompt_id).await;

        let abs_path = target_path.to_string_lossy().to_string();
        Self::emit_progress(
            ctx,
            &request,
            prompt,
            width,
            height,
            engine_label,
            "done",
            "图片已写入本地文件",
            1.0,
            Some(prompt_id),
            Some(&abs_path),
            None,
        )
        .await;

        // 生图完成后主动释放 PyTorch MPS 显存缓存
        crate::agent_runtime::video_tool::free_comfyui_cache(&client, false).await;

        // 两个告警都顶到前面。LoRA 名字写错是硬错误，排在长度告警之前。
        // 回落说明必须顶到最前：用户以为在用 Z-Image，实际拿到 SDXL 的图，
        // 不说等于骗人。
        let summary = with_lora_warning(
            with_lora_warning(
            with_lora_warning(
                image_summary(
                    seed,
                    original_prompt,
                    &abs_path,
                    engine_label,
                    width,
                    height,
                    &resolved_loras,
                    engine,
                    neg_prompt,
                ),
                // 只有 SDXL 会被长提示词摊薄（CLIP 77 token 分块取平均）。
                // Chroma 用 T5-XXL、Z-Image 用 Qwen3-4B，都是长文本原生 ——
                // 对它们报这个告警是错的，会把用户往「把提示词写短」上误导，
                // 而那恰恰会浪费掉它们最大的优势。
                steps_warning(engine, steps).or_else(|| {
                    if engine.dilutes_long_prompts() { prompt_dilution_warning(prompt) } else { None }
                }),
            ),
            lora_warning,
            ),
            // 引擎回落说明 + 参考图没用上的那几张，都得原样告诉用户
            match (engine_note, reference_note) {
                (Some(a), Some(b)) => Some(format!("{a}\n{b}")),
                (a, b) => a.or(b),
            },
        );

        // **全景出完要切开。** 一张等距圆柱图对下游没有用 —— 参考图、
        // 首帧、资产变体要的都是正常透视的单张。不切的话这一步就是死路：
        // 资产库里多一张奇怪的宽图，后面一个环节都认不出来。
        if wants_pano_views(&request.arguments) {
            use crate::agent_runtime::panorama;
            let p = std::path::Path::new(&abs_path);
            let seam = image::open(p).ok().and_then(|i| panorama::seam_score(&i));
            match panorama::split_panorama(p, &panorama::quad_views(), 768) {
                Ok(cells) => {
                    let mut summary = format!(
                        "{summary}\n\n已按 90° 切成 {} 张正交视图（正面 / 右侧 / 背面 / 左侧）：\n{}",
                        cells.len(),
                        cells.iter().map(|p| p.to_string_lossy().to_string())
                            .collect::<Vec<_>>().join("\n"),
                    );
                    // **接缝要说出来。** 这个 LoRA 出的全景左右边缘并不闭合
                    // （实测 0.33~0.51），跨 ±180° 的那一格会带一条竖直接缝。
                    // 不说的话那张单看很容易被当成「构图问题」重出一遍。
                    if let Some(s) = seam {
                        if s > 0.15 {
                            summary.push_str(&format!(
                                "\n\n⚠ 全景左右边缘没有闭合（接缝分数 {s:.2}，闭合的接近 0）——\
                                 **「背面」那一张会有一条竖直接缝**，它正好切在接缝上。\
                                 要躲开的话把四个角度整体偏 45°，代价是主体不再正对镜头。"
                            ));
                        }
                    }
                    let mut files = vec![abs_path.clone()];
                    files.extend(cells.iter().map(|p| p.to_string_lossy().to_string()));
                    return ToolOutcome::ok(&request, summary, files, started.elapsed());
                }
                Err(e) => {
                    return ToolOutcome::failed(
                        &request,
                        ToolFailure::new(
                            ToolFailureKind::Execution,
                            format!("全景出来了但切不开：{e}\n图在 {abs_path}"),
                        ),
                        started.elapsed(),
                    );
                }
            }
        }

        // **宫格出完要切开。** 一张九宫格对下游没有用 ——
        // 参考图、首帧、资产变体都要的是单张。
        if let (Some(views), Some(grid)) = (&scene_views, scene_grid) {
            let budget = crate::agent_runtime::scene_sheet::PixelBudget::of(width, height, grid);
            match crate::agent_runtime::scene_sheet::split_sheet(
                std::path::Path::new(&abs_path),
                grid,
                views,
            ) {
                Ok(cells) => {
                    let names: Vec<&str> = views.iter().map(|v| v.name.as_str()).collect();
                    let mut summary = format!(
                        "{summary}\n\n已切成 {} 张单图（{}）：{}\n{}",
                        cells.len(),
                        names.join(" / "),
                        cells
                            .iter()
                            .map(|p| p.to_string_lossy().to_string())
                            .collect::<Vec<_>>()
                            .join("\n"),
                        // **像素预算必须说出来。**
                        // 不说的话这张布局稿会被当成品用下去 ——
                        // 设定卡那次就是没人算这笔账。
                        budget.note()
                    );
                    // **色温读数。是个数，不是判决。**
                    //
                    // 端到端实跑两次否掉了「超了就是错」这个用法：面馆四格
                    // 人眼看全对，离散 70% —— 因为两格里有大片门外天光、
                    // 两格是纯内景。所以这里用 ℹ️ 不用 ⚠️，措辞也不下结论。
                    // 详见 `cinema_qc::TEMP_SPREAD_REPORT`。
                    if let Some(note) = crate::cinema_qc::check_color_temp_paths(&cells).note {
                        summary.push_str(&format!("\n\nℹ️ {note}"));
                    }

                    let mut files = vec![abs_path];
                    files.extend(cells.iter().map(|p| p.to_string_lossy().to_string()));

                    let wants_refine = request
                        .arguments
                        .get("refine")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    if wants_refine {
                        let (refined, failed) =
                            refine_cells(ctx, &cells, views, original_prompt, Some(engine.id())).await;
                        summary.push_str(&format!(
                            "\n\n逐格精修：{} 张成功{}",
                            refined.len(),
                            if failed.is_empty() {
                                String::new()
                            } else {
                                // **失败的要点名。** 只说「9 张成功了 7 张」，
                                // 用户不知道哪两格还是布局稿。
                                format!("，{} 张失败：{}", failed.len(), failed.join("；"))
                            }
                        ));
                        files.extend(refined);
                    } else if !budget.usable_as_final() {
                        // **关着的时候也要说它存在。**
                        // 不说的话用户只会拿到一张糊的布局稿，不知道还有第二段。
                        summary.push_str(
                            "\n\n要把这些格子当成品用，加 `refine: true` 逐格放大精修\
                             （每格多一次出图）。",
                        );
                    }
                    return ToolOutcome::ok(&request, summary, files, started.elapsed());
                }
                Err(e) => {
                    // 切不开不等于白出 —— 联合图本身还在，报上去让用户自己切
                    let summary = format!(
                        "{summary}\n\n**切分失败**：{e}\n联合图本身已经出来了，可以自己裁。"
                    );
                    return ToolOutcome::ok(&request, summary, vec![abs_path], started.elapsed());
                }
            }
        }

        ToolOutcome::ok(&request, summary, vec![abs_path], started.elapsed())
    }
}

/// 生成成功后返回给模型的那段文本。
///
/// **Seed 必须排在最前面，alt 文本里不要塞整段 prompt。**
///
/// 前端把历史里的工具返回按预算截断（`truncateArgs`）。这段原来长这样：
/// `![{prompt}](file://…)` 开头 —— 一条人像 prompt 七百多字符，把结尾的
/// Seed 直接挤出了截断窗口。于是下一轮「把裙子改成白色」时模型根本看不到
/// 上一张用的什么种子，只能重新随机 —— 出来的是**另一个人**。用户看到的
/// 就是「改个细节，结果跟上一张完全无关」，而且不报任何错。
///
/// alt 文本只给人看；前端取路径的正则是 `!\[.*?\]\((file://…)\)`，
/// 缩短 alt 不影响它。
/// 提示词回显在返回里最多占多少字符。
///
/// 前端 medium 档给整个工具轮次的额度是 1200 字符，回显要能整段活过那一刀，
/// 同时给 Seed / 路径 / 模型这些留出位置。
const PROMPT_ECHO_MAX: usize = 700;

/// **返回里必须回显本次用的提示词。**
///
/// 出过事：用户连出三张图后说「衣服不好看，换成浴袍」，模型只回了一句
/// 「马上为您进行二次渲染，请稍候」，然后什么都没发生 —— 工具没被调用，
/// ComfyUI 没收到任何任务，也不报错。
///
/// 根因不是模型能力，也不是上下文超长（实测同模型在 22662 token 下照常发
/// tool_calls）。是**模型看不到自己上一轮用的提示词**：
///
/// - 工具返回里只有 Seed / 路径 / 模型 / 尺寸，**没有提示词**；
/// - 前端只给最近两个工具轮次保留协议原样（`assistant.tool_calls` 里带着完整
///   参数），再往前压成文本摘要，参数就没了。
///
/// 于是改图请求变成「基于一个我看不见的东西做修改」。重放那段真实对话验证过：
///
/// | 历史形态 | 结果 |
/// |---|---|
/// | 只有聊天式总结（线上就是这样） | **无 tool_calls，正文也是空的** |
/// | 同一句话、无历史 | 明说「请您提供一下原始的图片描述和 Seed 值」 |
/// | 带 `tool_calls` + tool 结果（提示词和 seed 都在） | ✅ 正确发出改图调用，seed 对得上 |
///
/// 所以提示词要回显在**工具返回**里 —— 返回是压缩后仍然留在历史里的那一份，
/// 而调用参数不是。这和 Seed 前置是同一个道理。
///
/// **顺序是被测试钉住的**：Seed（前 100 字符内）→ 图片链接（前 200 字符内，
/// 前端靠 `![…](file://…)` 取图）→ 提示词回显。回显放前面会把图片链接挤出
/// 窗口，图就不显示了 —— 改动这段前先看那两条测试。
/// 挂了哪几个 LoRA。**必须说出来。**
///
/// 设定卡的 LoRA 有三个来源叠在一起：版式 LoRA（决定构图）、设置面板里配的
/// 项目固定画风、模型这次自己点的。哪个生效了、权重多少，不写在返回里的话
/// 用户没有任何办法知道 —— 而 LoRA 挂错或没挂上都**不报错**，只是图不对。
fn lora_line(loras: &[crate::agent_runtime::lora::ResolvedLora]) -> String {
    if loras.is_empty() {
        return String::new();
    }
    let list = loras
        .iter()
        .map(|l| format!("`{}` @{:.2}", l.name, l.strength))
        .collect::<Vec<_>>()
        .join(" + ");
    format!("\n- **已挂 LoRA**: {list}")
}

/// 负向提示词那一行。
///
/// **必须写出来。** 用户看不到它就没法判断"负向词到底有没有生效" ——
/// 而这件事今天被问过：SDXL 出图不如 Z-Image，用户以为是没自动加负向词。
/// 实际是加了的（`merge_negative_prompt` 永远把调好的那套放最前），
/// 只是返回里一个字都没提。
///
/// Z-Image 例外：它的负向通路被 `ConditioningZeroOut` 清零，**传什么都不起作用**，
/// 所以那里要写清"无效"而不是列一串让人误以为在保护。
fn negative_line(engine: crate::agent_runtime::image_engine::ImageEngine, neg: &str) -> String {
    if !engine.negative_works() {
        return format!(
            "\n- **负向提示词**：{} 不吃负向词（官方配置把负向通路清零了），本次未生效",
            engine.label()
        );
    }
    if neg.trim().is_empty() {
        return String::new();
    }
    let shown: String = if neg.chars().count() > 220 {
        format!("{}…", neg.chars().take(220).collect::<String>())
    } else {
        neg.to_string()
    };
    format!("\n- **负向提示词**：`{shown}`")
}

fn image_summary(
    seed: u64,
    prompt: &str,
    abs_path: &str,
    ckpt_name: &str,
    width: u32,
    height: u32,
    loras: &[crate::agent_runtime::lora::ResolvedLora],
    engine: crate::agent_runtime::image_engine::ImageEngine,
    negative: &str,
) -> String {
    let echo: String = if prompt.chars().count() > PROMPT_ECHO_MAX {
        let head: String = prompt.chars().take(PROMPT_ECHO_MAX).collect();
        format!("{head}…（已截断）")
    } else {
        prompt.to_string()
    };
    let lora_note = lora_line(loras);
    let neg_note = negative_line(engine, negative);
    format!(
            "- **Seed**: `{seed}`（改这张图的细节时把同一个 seed 传回来，构图和人物才会保持一致）\n![生成的图像](file://{abs_path})\n\n- **本次提示词**（改图时在它基础上改，不要凭空重写；不记得就照抄这一段）：`{echo}`\n- **模型**: `{ckpt_name}` | **尺寸**: {width}x{height}{lora_note}{neg_note}\n- **已保存在工作区**：`{abs_path}`"
    )
}

#[cfg(test)]
mod tests {
    /// **宫格这条路要真的被工具认出来、真的会切。**
    ///
    /// 这条测的是「工具接没接」，不是版式数学对不对（那个在 `scene_sheet`
    /// 里测）。出事的形态是：模块写好了、它自己的测试全绿，
    /// 但 `generate_image` 根本没调它。
    #[test]
    fn 工具认得出场景宫格版式() {
        use serde_json::json;
        assert!(wants_scene_sheet(&json!({"layout": "scene_sheet"})).is_some());
        assert!(wants_scene_sheet(&json!({"layout": "SCENE_SHEET"})).is_some());
        assert!(wants_scene_sheet(&json!({"layout": "normal"})).is_none());
        assert!(wants_scene_sheet(&json!({})).is_none());
        // 没给视角就用默认那套，不是切零格
        let v = wants_scene_sheet(&json!({"layout": "scene_sheet"})).unwrap();
        assert!(!v.is_empty());
        // 给了就用给的
        let v = wants_scene_sheet(&json!({
            "layout": "scene_sheet",
            "viewpoints": ["全景", {"name": "反打", "description": "看见门"}]
        }))
        .unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[1].description, "看见门");
    }

    /// 宫格和设定卡**是两条路**，不能互相拽走。
    #[test]
    fn 宫格不触发设定卡那条路() {
        use serde_json::json;
        let args = json!({"layout": "scene_sheet"});
        assert!(!wants_character_sheet(&args), "宫格不该被当成设定卡");
        assert!(wants_entity_sheet(&args).is_none(), "宫格不该被当成完整设定表");
    }

    #[test]
    fn 场景名优先用传进来的不编一个() {
        use serde_json::json;
        assert_eq!(
            scene_name_from(&json!({"scene_name": "老茶馆"}), "随便什么"),
            "老茶馆"
        );
        // 没给就从提示词开头截，截出来的是用户自己写的字
        assert_eq!(
            scene_name_from(&json!({}), "老茶馆，午后的光线斜斜照进来"),
            "老茶馆"
        );
        // 空字符串当没给
        assert_eq!(scene_name_from(&json!({"scene_name": "  "}), "客厅"), "客厅");
    }

    use super::*;
    use crate::agent_runtime::image_engine::ImageEngine;

    /// 前端 recent 档给每条工具返回的额度下限（`MIN_TOOL_RESULT_CHARS`）。
    /// 只要 Seed 落在这个窗口内，无论前端怎么压缩它都活得下来。
    const FRONTEND_MIN_RESULT_CHARS: usize = 100;

    #[test]
    fn seed_survives_frontend_truncation() {
        let s = image_summary(
            424_242,
            "a girl in a red dress",
            "/Users/me/work/a_very_long_output_file_name_for_a_portrait.png",
            "sdxl",
            1024,
            1024,
            &[],
            ImageEngine::Sdxl,
            "",
        );
        let window: String = s.chars().take(FRONTEND_MIN_RESULT_CHARS).collect();
        assert!(
            window.contains("424242"),
            "Seed 必须落在前 {FRONTEND_MIN_RESULT_CHARS} 字符内，否则改图那一轮\
             模型看不到它，只能重新随机 —— 用户看到的就是「改个细节换了个人」。\
             实际开头是：{window}"
        );
    }

    /// 见 video_tool 里同名测试的说明：前导空格会让 markdown 把嵌入当成代码块，
    /// 图就不显示了。这份 summary 曾经真的坏过一整轮。
    #[test]
    fn 返回文本里没有会变成代码块的缩进() {
        let text = image_summary(1, "a girl", "/tmp/a.png", "sdxl", 1024, 1024, &[], ImageEngine::Sdxl, "");
        for line in text.lines() {
            assert!(!line.starts_with(' '), "这一行会被当成代码块：{line:?}");
        }
    }

    /// 设定卡版式的三件事必须一起生效。
    ///
    /// 少任何一样都不会报错，只是出一张普通单人图 —— 又是"看起来成功了"。
    #[test]
    fn 设定卡版式三件事齐全() {
        let l = sheet_layout("a girl");
        // 1. 触发词：LoRA 的训练标签，不写它 LoRA 不激活
        assert!(l.prompt_prefix.contains("turnaround"), "缺版式触发词：{}", l.prompt_prefix);
        assert!(l.prompt_prefix.contains("front view"), "{}", l.prompt_prefix);
        assert!(l.prompt_prefix.contains("side view"), "{}", l.prompt_prefix);
        assert!(l.prompt_prefix.contains("back view"), "{}", l.prompt_prefix);
        // 2. 宽画幅：1:1 塞不下三个人
        assert!(l.width > l.height, "设定卡必须是宽画幅，得到 {}x{}", l.width, l.height);
        // 3. 权重取自 cinema_sheet，不在这里另写一个数
        assert_eq!(l.lora_weight, crate::cinema_sheet::TURNAROUND_LORA_WEIGHT);
    }

    /// 用户**没说**画风时，仍然把画风拉向写实 —— 版式 LoRA 会往塑料人偶拽。
    ///
    /// 实测：同 seed、同 checkpoint、同 0.55 权重，只加这串词就从塑料变写实；
    /// 而调低权重（0.55 / 0.40 / 0.28）三档都写实 —— 不是权重的问题。
    #[test]
    fn 用户没点画风时才拉向写实() {
        let l = sheet_layout("a girl in a school uniform");
        for w in ["3d render", "plastic", "doll", "cartoon"] {
            assert!(l.negative_extra.contains(w), "负向词里缺 `{w}`：{}", l.negative_extra);
        }
        for w in ["photorealistic", "skin texture"] {
            assert!(l.positive_suffix.contains(w), "正向补充里缺 `{w}`：{}", l.positive_suffix);
        }
    }

    /// **用户点名的画风绝不能出现在工具自己加的负向词里。**
    ///
    /// 出过事：用户要「BJD 人偶 + 3D 写实国漫的学院风校服四视图」，
    /// 设定卡无条件追加 `photorealistic, raw photo, natural skin texture` 和
    /// 负向 `3d render, cgi, plastic, doll, cartoon, anime` ——
    /// 出来的是**欧美裸体人体扫描转身图**。
    ///
    /// 复现（seed 228334280 / LoRA 0.55 / 1536x1024 / 35 步）：同一句中文提示词，
    /// 加这些画风词 → 裸体欧美写实；不加 → 穿校服、东亚脸、BJD 质感。
    #[test]
    fn 用户点名画风时不加相反的画风词() {
        for p in [
            "BJD 人偶质感的 3D 写实国漫少女",
            "anime style schoolgirl, cel shaded",
            "高精度三维渲染，手办风格",
            "octane render, stylized character",
        ] {
            let l = sheet_layout(p);
            for w in ["3d render", "cgi", "plastic", "doll", "cartoon", "anime"] {
                assert!(
                    !l.negative_extra.contains(w),
                    "提示词 `{p}` 点名了画风，负向词却仍含 `{w}`：{}",
                    l.negative_extra
                );
            }
            for w in ["photorealistic", "raw photo", "skin texture"] {
                assert!(
                    !l.positive_suffix.contains(w),
                    "提示词 `{p}` 点名了画风，正向却仍硬塞 `{w}`：{}",
                    l.positive_suffix
                );
            }
        }
    }

    // ---- 参考生图 ---------------------------------------------------------------

    /// **顺序就是含义**：场景在前、人物在后。不许排序、不许去重打乱。
    #[test]
    fn 参考图保持调用方给的顺序() {
        let dir = tempfile::tempdir().unwrap();
        for n in ["scene.png", "a.png", "b.png"] {
            std::fs::write(dir.path().join(n), b"x").unwrap();
        }
        let args = json!({"reference_images": ["scene.png", "a.png", "b.png"]});
        let got: Vec<String> = reference_image_paths(&args, dir.path())
            .iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect();
        assert_eq!(got, vec!["scene.png", "a.png", "b.png"]);
    }

    /// 画布、对话常把唯一一张图放在 `image_path` —— 参考生图引擎拿它当第一张参考，
    /// 而不是报「没有参考图」。
    #[test]
    fn 没给参考图数组时退回_image_path() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("only.png"), b"x").unwrap();
        let args = json!({"image_path": "only.png"});
        assert_eq!(reference_image_paths(&args, dir.path()).len(), 1);
    }

    /// 不存在的文件不进参考图 —— 进了的话 ComfyUI 那边 LoadImage 找不到文件，
    /// 整次任务在排队几分钟之后才失败。
    #[test]
    fn 不存在的参考图被滤掉() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("real.png"), b"x").unwrap();
        let args = json!({"reference_images": ["real.png", "missing.png"]});
        assert_eq!(reference_image_paths(&args, dir.path()).len(), 1);
    }

    /// **挑剩下的要说出来**，不说的话用户以为那个人也锁住了。
    #[test]
    fn 参考图比槽位多时说出没用上几张() {
        use crate::agent_runtime::image_engine::ImageEngine;
        let n = reference_overflow_note(ImageEngine::Krea2Edit, 3).expect("三张给两个槽却没说");
        assert!(n.contains("1 张没用上"), "{n}");
        assert!(reference_overflow_note(ImageEngine::Krea2Edit, 2).is_none(), "没溢出却报了");
        assert!(reference_overflow_note(ImageEngine::Qwen21Edit, 10).is_none());
        assert!(reference_overflow_note(ImageEngine::ZImage, 5).is_none(), "不是参考引擎也报了");
    }

    /// 缺 GELU 修复的报错要**说清后果和修法** —— 后果是纯黑图还显示成功，
    /// 不说的话用户会以为是 Qwen 在这台机器上不行。
    #[test]
    fn 缺_gelu_修复时报错里说清后果和修法() {
        use crate::agent_runtime::image_engine::ImageEngine;
        let q = missing_engine_nodes_message(ImageEngine::Qwen21, &[format!("节点 {GGUF_ARCH_NODE}")]);
        assert!(q.contains("anima_gguf_arch") && q.contains("重启"), "{q}");
        assert!(q.contains("Unknown model architecture"), "要把 ComfyUI 那句原文写出来，否则搜不到：{q}");
        let m = missing_engine_nodes_message(ImageEngine::Krea2Edit, &[format!("节点 {GELU_FIX_NODE}")]);
        assert!(m.contains("纯黑"), "{m}");
        assert!(m.contains("anima_mps_gelu_fix") && m.contains("重启"), "{m}");
    }

    /// **服装锚点和 nsfw 兜底在两条分支上都必须有。**
    ///
    /// 转身卡训练集的默认是裸体人体参考；提示词里写了校服也压不住，
    /// 得在版式这一层顶回去。这是上面那次事故里「裸体」的直接来源。
    #[test]
    fn 设定卡两条分支都带服装锚点和nsfw兜底() {
        for p in ["BJD 人偶 3D 国漫少女", "a girl in a school uniform"] {
            let l = sheet_layout(p);
            assert!(
                l.positive_suffix.contains("fully clothed"),
                "`{p}` 缺服装锚点：{}",
                l.positive_suffix
            );
            for w in ["nsfw", "nude", "naked"] {
                assert!(l.negative_extra.contains(w), "`{p}` 缺 nsfw 兜底 `{w}`：{}", l.negative_extra);
            }
        }
    }

    /// **用户传负向词不能把默认解剖负向词冲掉。**
    ///
    /// 事故现场：用户传了一串中文负向词，`extra legs` / `malformed hands` /
    /// `bad proportions` 全部消失，出来的人物三条腿。
    /// **提示词过长要出声，因为它的失败是静默的。**
    ///
    /// 实测：同 seed / 同参数，1075 字 Markdown 长文出来的服装是白背心白长裤，
    /// 压到 ~120 字关键词后学院风校服就回来了。图两次都"生成成功"，不报任何错。
    #[test]
    fn 提示词过长时告警() {
        // 正常长度：不打扰
        assert!(prompt_dilution_warning("a girl in a school uniform, black hair").is_none());
        assert!(prompt_dilution_warning("米白色学院风校服的东方少女，乌黑及腰长发，3D 国漫风格").is_none());

        // 用户那段 1075 字的 Markdown 长文必须报
        let long_cn = "全套角色设定卡，四视图的超高精度三维写实国漫风格女性人物。".repeat(12);
        let w = prompt_dilution_warning(&long_cn).expect("这么长还不报，等于没做");
        assert!(w.contains("关键词"), "告警得说清楚怎么改：{w}");

        // 中文比同字数英文更容易超窗 —— 字节回退，一个汉字约 2~3 token
        let cn = "学院风校服的东方少女".repeat(20);
        let en = "a girl in a school uniform".repeat(20);
        assert!(
            estimate_clip_tokens(&cn) > estimate_clip_tokens(&en) / 2,
            "中文的 token 估算太低了：中文 {} vs 英文 {}",
            estimate_clip_tokens(&cn),
            estimate_clip_tokens(&en)
        );
    }

    /// **工具返回里必须回显提示词，否则改图那一轮模型是瞎的。**
    ///
    /// 线上事故：连出三张图后用户说「衣服不好看，换成浴袍」，模型回了一句
    /// 「马上为您进行二次渲染」就没了下文 —— 无 tool_calls、无任务、无报错。
    /// 重放验证过：把提示词和 seed 放进历史，同一个模型立刻发出正确的改图调用。
    ///
    /// 关键是回显在**返回**里而不是只在调用参数里：前端只给最近两个工具轮次
    /// 保留协议原样，再往前只剩返回的文本摘要。
    #[test]
    fn 返回里回显提示词供改图使用() {
        let p = "hyper realistic mirror selfie, black off-shoulder dress, marble bathroom";
        let s = image_summary(1, p, "/tmp/a.png", "sdxl", 1024, 768, &[], ImageEngine::Sdxl, "");
        assert!(s.contains(p), "返回里没有提示词，改图那一轮就无从下手：{s}");

        // 超长提示词要截断，否则会把 medium 档 1200 字符的额度整个吃掉
        let long: String = "a very detailed cinematic portrait, ".repeat(80);
        let s2 = image_summary(1, &long, "/tmp/a.png", "sdxl", 1024, 768, &[], ImageEngine::Sdxl, "");
        assert!(s2.contains("已截断"), "超长提示词没截断：{}", &s2[..200.min(s2.len())]);
        assert!(
            s2.chars().count() < 1_200,
            "回显后整段 {} 字符，超过前端 medium 档 1200 的额度",
            s2.chars().count()
        );

        // Seed 仍然要在最前面 —— 回显不能把它挤出截断窗口
        let window: String = s2.chars().take(FRONTEND_MIN_RESULT_CHARS).collect();
        assert!(window.contains('1'), "Seed 被回显挤出了窗口：{window}");
    }

    /// **工具必须能选引擎，而且默认是 Z-Image。**
    ///
    /// 三个引擎实测耗时 45s / 331s / 39s，差 8.5 倍。默认值选错的代价是
    /// 用户每张图多等 5 分钟，而且不知道为什么。
    /// **队列预估必须按引擎取，不能写死。**
    ///
    /// 线上撞到过：模型一次发了三张，其中一张走 Chroma。骨架屏说
    /// 「前面还有 2 个任务，预计还要等约 60 秒」—— 用的是写死的 30s/张，
    /// 而那两个里有一个是 Chroma（实测 331s），真实等待 6 分钟以上。
    /// 估时错到这个量级，和不显示队列一样糟：用户会以为卡死了。
    /// **模型会按 SDXL 的习惯给 Z-Image 传 steps=20，必须出声。**
    ///
    /// 线上撞到过：1024x1024 从 45 秒变成 121 秒，而 Z-Image 是蒸馏 turbo
    /// 模型、设计点就是 8 步 —— 那 2.5 倍时间完全白花，画质不会更好。
    /// 用户看到的是「怎么这么慢」，而工具当时一声不吭。
    #[test]
    fn 步数超出引擎设计点时告警() {
        use crate::agent_runtime::image_engine::ImageEngine;
        // 就是线上那次
        let w = steps_warning(ImageEngine::ZImage, 20).expect("20 步不告警等于没做");
        assert!(w.contains("8"), "没说清设计点是 8：{w}");
        assert!(w.contains("121") || w.contains("45"), "没给出实测代价：{w}");

        // 按默认值来的不打扰
        assert!(steps_warning(ImageEngine::ZImage, 8).is_none());
        assert!(steps_warning(ImageEngine::Krea2, 8).is_none());
        assert!(steps_warning(ImageEngine::Sdxl, 20).is_none());
        // 非蒸馏模型只在明显偏高时说
        assert!(steps_warning(ImageEngine::Sdxl, 24).is_none());
        assert!(steps_warning(ImageEngine::Sdxl, 40).is_some());
    }

    /// schema 里不能再写「20 是推荐值」—— 那句话就是模型传 20 的来源。
    #[test]
    fn steps描述不再推荐sdxl的20步() {
        let spec = GenerateImageTool.spec();
        let d = spec.parameters.pointer("/properties/steps/description")
            .and_then(|v| v.as_str()).expect("缺 steps 描述");
        assert!(d.contains("Omit this"), "没让模型别传 steps：{d}");
        assert!(d.contains('8'), "没说 Z-Image 用 8 步：{d}");
        assert!(
            spec.parameters.pointer("/properties/steps/default").is_none(),
            "steps 不该有 schema 默认值 —— 正确的默认值是逐引擎的，写在 schema 里就固定了一个"
        );
    }

    /// **排队的时间不算、生成本身不设时长上限。** 2026-09-29 用户按场出首帧：Qwen 一张五分钟，排在后面的镜头
    /// 光排队就等满 40 分钟被判失败，而它们在 ComfyUI 里好好排着、随后出了图；挂 8 张参考图的一张生成就要 37 分钟。
    /// 还在不在跑看进度，判定的测试在 `comfy_watch`；这里钉接线
    #[test]
    fn 出图等待_排队不计时_生成看进度() {
        let src = include_str!("image_tool.rs");
        let body = &src[src.find("let mut watch = super::comfy_watch::tracker(prompt_id);").expect("出图轮询没接进度判定")..];
        let body = &body[..body.find("if image_filename.is_empty()").unwrap()];
        assert!(body.contains("watch.observe(seen)"));
        assert!(body.contains("pending_polls >= IMAGE_PENDING_CEILING_POLLS"));
        assert_eq!(IMAGE_PENDING_CEILING_POLLS as u64 * 2, 12 * 3600);
    }

    /// **registry 的工具超时（只管对话那条）要装得下正常的一次多张。**
    ///
    /// 出过事：`with_timeout` 是 120 秒 —— 模型一次要三张图时**每次都是第三张失败**：三个调用
    /// 并发启动、各自的 120 秒同时跑，而 ComfyUI 串行执行，排第三的那个
    /// 等到 ~136 秒才轮到，早被 registry 掐了。图在 ComfyUI 那边是 success，
    /// 就在磁盘上，用户看到的却是「这张没生成出来」。
    #[test]
    fn 工具超时装得下一次多张() {
        // 而且要装得下最坏情况：一次多张、全排在最慢的那个引擎后面。
        // 最慢的原来是 Chroma（331s），2026-10-01 下架后是 Qwen 2.1 参考生图（260s）
        use crate::agent_runtime::image_engine::ImageEngine;
        let worst = secs_per_image_job(ImageEngine::Qwen21Edit) * 5;
        assert!(
            IMAGE_TOOL_TIMEOUT_SECS > worst,
            "一次五张 Qwen 2.1 参考生图要 {worst}s，超时只有 {IMAGE_TOOL_TIMEOUT_SECS}s"
        );

        // spec 里真的用上了这个值 —— 光定义常量不接线等于没改
        let spec = GenerateImageTool.spec();
        assert_eq!(spec.timeout_ms, IMAGE_TOOL_TIMEOUT_SECS * 1000);
    }

    #[test]
    fn 队列预估按引擎取而不是写死() {
        use crate::agent_runtime::image_engine::ImageEngine;
        assert_eq!(secs_per_image_job(ImageEngine::ZImage), 45);
        assert_eq!(secs_per_image_job(ImageEngine::Qwen21), 211);
        assert_eq!(secs_per_image_job(ImageEngine::Sdxl), 39);
        assert!(
            secs_per_image_job(ImageEngine::Qwen21)
                > secs_per_image_job(ImageEngine::ZImage) * 4,
            "Qwen 2.1 和 Z-Image 的差距必须留在预估里，否则等于没估"
        );
    }

    /// 轮询上限也得跟着引擎走 —— 用 SDXL 的数去算慢引擎，每次都会超时，
    /// 而超时和失败在界面上没有区别。
    ///
    /// **对着实测值验，不要拿同一个函数算出来的数自己验自己** ——
    /// 第一版写成了 `(per/2+20)*2 > per`，任何 per 都成立，变异测试照样绿。
    #[test]
    fn 轮询上限够跑完一张() {
        use crate::agent_runtime::image_engine::ImageEngine;
        // 每轮 sleep 2 秒，所以覆盖时长 = max_polls * 2
        let budget = |e| (secs_per_image_job(e) as u32 / 2 + 20) * 2;
        // 实测单张耗时写死在这里，不从被测函数里取
        for (e, measured) in [
            (ImageEngine::ZImage, 45u32),
            (ImageEngine::Qwen21, 211),
            (ImageEngine::Qwen21Edit, 260),
            (ImageEngine::Sdxl, 39),
        ] {
            assert!(
                budget(e) > measured,
                "{} 实测单张 {measured}s，轮询上限只覆盖 {}s —— 必然超时，\
                 而超时和失败在界面上没有区别",
                e.label(),
                budget(e)
            );
        }
    }

    #[test]
    fn 工具能选引擎且默认是默认底模() {
        let spec = GenerateImageTool.spec();
        let e = spec.parameters.pointer("/properties/engine").expect("缺 engine 参数");
        // 工具说明里的默认值必须就是真正的默认 —— 写着 zimage、实际跑 qwen21 的话，
        // 模型会按 45 秒去估等待时间
        assert_eq!(e["default"], crate::agent_runtime::image_engine::DEFAULT_ENGINE.id());
        let en: Vec<String> = e["enum"].as_array().unwrap().iter()
            .map(|v| v.as_str().unwrap().to_string()).collect();
        // **工具参数里的引擎表必须和能力目录里的出图引擎一模一样。**
        // 两处各写一份的话，加引擎时漏改一处：界面上能选、对话里调不动（或反过来），
        // 而且不报错。2026-09-13 加四个引擎时这条测试就是这么响的。
        let catalog_ids: Vec<String> = crate::studio::catalog().engines.iter()
            .filter(|e| e.kind == "image").map(|e| e.id.to_string()).collect();
        assert_eq!(en, catalog_ids, "工具参数的引擎表和能力目录对不上");
        // 默认引擎必须在表里。**不再要求排第一**：这张表和能力目录（设置里的下拉）
        // 是同一份顺序，为了换默认去重排界面不值得。
        assert!(en.contains(&crate::agent_runtime::image_engine::DEFAULT_ENGINE.id().to_string()), "默认引擎不在表里");
        let d = e["description"].as_str().unwrap();
        // 描述里必须写清代价，否则模型会为了「质量更好」无脑选慢的那个。
        // （原来是 Chroma 331s，2026-10-01 下架；现在慢的是 Qwen 2.1 211s、Krea 2 108s。）
        // 光写代价还不够 —— 实测它照选了，而且正文里说的是 zimage。
        // 所以还要有一句明确的「别自己设这个参数」。
        assert!(d.contains("211"), "没告诉模型 qwen21 有多慢：{d}");
        assert!(d.contains("108"), "没告诉模型 krea2 有多慢：{d}");
        assert!(!d.contains("chroma"), "说明里还在推荐已下架的 chroma：{d}");
        assert!(d.contains("45"), "没告诉模型 zimage 的耗时：{d}");
        assert!(
            d.contains("Omit this parameter"),
            "没有明确禁止模型自作主张选引擎 —— 实测它会选 chroma 而且不说：{d}"
        );
        // 也必须写清 zimage 的负向词无效，否则模型会以为传了就有保护
        assert!(d.contains("NO effect"), "没说明 zimage 的负向词无效：{d}");
    }

    /// **挂了哪几个 LoRA 必须写在返回里。**
    ///
    /// 设定卡的 LoRA 有三个来源叠在一起：版式（决定构图）、设置面板里配的
    /// 项目固定画风、模型这次自己点的。而 LoRA 挂错或没挂上都**不报错** ——
    /// 不写出来的话，用户没有任何办法知道设置到底生效没有。
    /// **负向提示词要写在返回里。**
    ///
    /// 用户问过"SDXL 是不是没自动加负向词" —— 实际是加了的
    /// （`merge_negative_prompt` 永远把调好的那套放最前），但返回里一个字没提，
    /// 他没有任何办法确认。看不见的保护等于没有保护。
    ///
    /// Z-Image 相反：它的负向通路被清零，**传什么都不起作用**，
    /// 那里必须写"未生效"，而不是列一串让人误以为在保护。
    #[test]
    fn 返回里报告负向提示词() {
        let neg = default_negative_prompt();

        let sdxl = image_summary(1, "p", "/tmp/a.png", "sdxl", 1024, 1024, &[], ImageEngine::Sdxl, neg);
        assert!(sdxl.contains("负向提示词"), "SDXL 没报告负向词：{sdxl}");
        assert!(sdxl.contains("extra limbs"), "负向词内容没写出来：{sdxl}");

        // Z-Image 必须说"未生效"，不能列一串
        let zi = image_summary(1, "p", "/tmp/a.png", "z", 1024, 1024, &[], ImageEngine::ZImage, neg);
        assert!(zi.contains("未生效"), "Z-Image 该说明负向词无效：{zi}");
        assert!(!zi.contains("extra limbs"), "Z-Image 不该列负向词让人误以为在保护：{zi}");

        // 空负向词时不多这一行
        let empty = image_summary(1, "p", "/tmp/a.png", "sdxl", 1024, 1024, &[], ImageEngine::Sdxl, "");
        assert!(!empty.contains("负向提示词"));
    }

    #[test]
    fn 返回里报告挂了哪些lora() {
        use crate::agent_runtime::lora::ResolvedLora;
        let none = image_summary(1, "p", "/tmp/a.png", "sdxl", 1024, 1024, &[], ImageEngine::Sdxl, "");
        assert!(!none.contains("已挂 LoRA"), "没挂 LoRA 时不该多这一行：{none}");

        let two = [
            ResolvedLora { name: "CharTurnXLRetag_v4.safetensors".into(), strength: 0.55 },
            ResolvedLora { name: "guofeng_illust_sdxl.safetensors".into(), strength: 0.6 },
        ];
        let s = image_summary(1, "p", "/tmp/a.png", "sdxl", 1024, 1024, &two, ImageEngine::Sdxl, "");
        assert!(s.contains("CharTurnXLRetag_v4"), "缺版式 LoRA：{s}");
        assert!(s.contains("guofeng_illust_sdxl"), "缺配置的风格 LoRA：{s}");
        assert!(s.contains("0.55") && s.contains("0.60"), "权重要写出来：{s}");
    }

    #[test]
    fn 用户负向词是追加而不是替换() {
        let merged = merge_negative_prompt(Some("低精度, 油画质感"));
        for w in ["extra legs", "extra limbs", "malformed hands", "bad proportions"] {
            assert!(merged.contains(w), "用户负向词把默认的 `{w}` 冲掉了：{merged}");
        }
        assert!(merged.contains("低精度"), "用户自己的负向词丢了：{merged}");

        // 没传 / 空串时就是默认那套，不要多出一个逗号
        assert_eq!(merge_negative_prompt(None), default_negative_prompt());
        assert_eq!(merge_negative_prompt(Some("   ")), default_negative_prompt());
    }

    #[test]
    fn 画风意图识别中英文都认() {
        for p in ["BJD 人偶", "3D 国漫", "手办风", "二次元插画", "anime", "octane render", "stylized"] {
            assert!(has_style_intent(p), "没认出画风意图：{p}");
        }
        for p in ["a girl in a school uniform", "一位穿着校服的少女", "portrait of a woman"] {
            assert!(!has_style_intent(p), "误判成有画风意图：{p}");
        }
    }

    #[test]
    fn 只有明确要设定卡时才切版式() {
        assert!(wants_character_sheet(&json!({"layout": "character_sheet"})));
        assert!(wants_character_sheet(&json!({"layout": "CHARACTER_SHEET"})));
        assert!(!wants_character_sheet(&json!({"layout": "normal"})));
        assert!(!wants_character_sheet(&json!({})));
    }

    /// 触发词必须和 cinema_sheet 用的是**同一份** —— 各写一份会静默漂。
    #[test]
    fn 触发词复用cinema_sheet而不是抄一份() {
        use crate::cinema_sheet::SheetEngine;
        assert_eq!(sheet_layout("a girl").prompt_prefix, SheetEngine::Sdxl.layout_trigger());
    }

    /// **手脚和多肢畸形必须点名。**
    ///
    /// 实测（同 seed / 同提示词 / 同 20 步，瑜伽单腿站姿）：只有笼统的
    /// `bad anatomy` 时人是悬空的、支撑腿够不到地面；加上具体词之后站姿正常。
    /// 和「设定卡塑料感靠 anti-3D 词而不是调权重」是同一回事 ——
    /// SDXL 的负向词逐词起作用，抽象形容词覆盖不到具体失败模式。
    #[test]
    fn 默认负向词覆盖具体的肢体失败模式() {
        let spec = GenerateImageTool.spec();
        let desc = serde_json::to_string(&spec.parameters).unwrap();
        // schema 里不该写死负向词，它是代码里的默认值
        assert!(desc.contains("negative_prompt"));

        // 直接检查默认值本身
        let neg = default_negative_prompt();
        for w in [
            "extra limbs", "extra legs", "extra fingers", "fused fingers",
            "malformed hands", "poorly drawn feet", "bad proportions",
        ] {
            assert!(neg.contains(w), "默认负向词里缺 `{w}` —— 笼统的 bad anatomy 拦不住：{neg}");
        }
    }

    fn pose() -> PoseControl {
        PoseControl {
            pose_filename: "pose_ref_1.png".into(),
            controlnet: "controlnet-openpose-sdxl.safetensors".into(),
            strength: 0.85,
            end_percent: 0.9,
        }
    }

    /// **插了节点还必须改连线。**
    ///
    /// 只插不改的话 ControlNet 是死的：图跑得通、任务 success、姿势完全不受控，
    /// 而且不报任何错 —— 又一个静默失败。
    #[test]
    fn 姿势控制要改写条件出口而不只是插节点() {
        let mut wf = json!({
            "5": {"class_type": "KSampler", "inputs": {"positive": ["2",0], "negative": ["3",0]}}
        });
        let (pos, neg) = insert_pose_control(&mut wf, &pose(), json!(["2", 0]), json!(["3", 0]));
        // 出口必须指向 ControlNetApplyAdvanced，不能还是文本编码器
        assert_eq!(pos, json!(["803", 0]));
        assert_eq!(neg, json!(["803", 1]));
        assert_ne!(pos, json!(["2", 0]), "没改写的话 ControlNet 形同虚设");

        // 四个节点都要在
        for (id, class) in [
            ("800", "LoadImage"),
            ("801", "DWPreprocessor"),
            ("802", "ControlNetLoader"),
            ("803", "ControlNetApplyAdvanced"),
        ] {
            assert_eq!(wf[id]["class_type"], class, "缺节点 {id}");
        }
        // ControlNet 吃的是**骨架**（预处理器输出），不是原图
        assert_eq!(wf["803"]["inputs"]["image"], json!(["801", 0]));
    }

    /// 节点 id 不能和图本身（个位数）或 LoRA 链（900 起）撞。
    #[test]
    fn 姿势节点的id不和别的段撞() {
        let mut wf = json!({});
        insert_pose_control(&mut wf, &pose(), json!(["2", 0]), json!(["3", 0]));
        for id in wf.as_object().unwrap().keys() {
            let n: u32 = id.parse().unwrap();
            assert!((800..900).contains(&n), "姿势节点 id {n} 落在了别的段");
        }
    }

    /// 挑 ControlNet 只信 ComfyUI 报的枚举，不猜文件名。
    #[test]
    fn 按枚举挑controlnet() {
        let avail: Vec<String> = ["controlnet-openpose-sdxl.safetensors", "controlnet-union-sdxl-promax.safetensors"]
            .iter().map(|s| s.to_string()).collect();
        assert_eq!(pick_pose_controlnet(&avail).unwrap(), "controlnet-openpose-sdxl.safetensors");
        // 一个都没有时返回 None —— 调用方据此明确报错，而不是默默出一张姿势不对的图
        assert!(pick_pose_controlnet(&[]).is_none());
        // 只有 union 时也能用
        let only_union = vec!["controlnet-union-sdxl-promax.safetensors".to_string()];
        assert!(pick_pose_controlnet(&only_union).is_some());
    }

    /// 路径也不能被挤掉：`getTurnImageCalls` 靠 `![…](file://…)` 取图。
    #[test]
    fn image_path_stays_near_the_front() {
        let s = image_summary(1, "a girl", "/tmp/a.png", "sdxl", 512, 512, &[], ImageEngine::Sdxl, "");
        let at = s.find("file:///tmp/a.png").expect("必须有 file:// 链接");
        assert!(at < 200, "图片链接落在第 {at} 字符，太靠后了");
    }

    #[test]
    fn 文生图支持图生图与去噪强度参数() {
        let spec = GenerateImageTool.spec();
        assert!(
            spec.parameters.pointer("/properties/image_path").is_some(),
            "GenerateImageTool 必须包含 image_path 参数以支持垫图修改"
        );
        assert!(
            spec.parameters.pointer("/properties/denoise").is_some(),
            "GenerateImageTool 必须包含 denoise 参数以控制重绘幅度"
        );
    }
}

#[cfg(test)]
mod dump_pose_graph {
    use super::*;

    /// 打印带姿势控制的完整图，供人工提交 ComfyUI 实跑。取证用，默认 ignore。
    #[test]
    #[ignore]
    fn dump() {
        let mut wf = json!({
            "1": {"class_type": "CheckpointLoaderSimple", "inputs": {"ckpt_name": "RealVisXL_V5.0_fp16.safetensors"}},
            "2": {"class_type": "CLIPTextEncode", "inputs": {"text": "full body photo of a young woman in tree pose on a yoga mat, both arms raised overhead palms together, bright studio, photorealistic", "clip": ["1", 1]}},
            "3": {"class_type": "CLIPTextEncode", "inputs": {"text": default_negative_prompt(), "clip": ["1", 1]}},
            "4": {"class_type": "EmptyLatentImage", "inputs": {"width": 832, "height": 1216, "batch_size": 1}},
            "5": {"class_type": "KSampler", "inputs": {"model": ["1", 0], "positive": ["2", 0], "negative": ["3", 0],
                  "latent_image": ["4", 0], "seed": 999, "steps": 20, "cfg": 7.0,
                  "sampler_name": "euler", "scheduler": "normal", "denoise": 1.0}},
            "6": {"class_type": "VAEDecode", "inputs": {"samples": ["5", 0], "vae": ["1", 2]}},
            "7": {"class_type": "SaveImage", "inputs": {"filename_prefix": "POSE_E2E", "images": ["6", 0]}},
        });
        let p = PoseControl {
            pose_filename: "pose_ref.png".into(),
            controlnet: "controlnet-openpose-sdxl.safetensors".into(),
            strength: 0.85,
            end_percent: 0.9,
        };
        let (pos, neg) = insert_pose_control(&mut wf, &p, json!(["2", 0]), json!(["3", 0]));
        let k = wf.get_mut("5").unwrap().get_mut("inputs").unwrap().as_object_mut().unwrap();
        k.insert("positive".into(), pos);
        k.insert("negative".into(), neg);
        println!("{}", serde_json::to_string(&wf).unwrap());
    }
}
