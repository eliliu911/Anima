//! Anima Studio 的能力目录。
//!
//! **前端不该知道任何引擎的脾气。** 哪个引擎吃负向提示词、哪个不该传步数、
//! 哪个提示词写长了会被摊薄 —— 这些都是本机实测出来的结论，写在
//! `image_engine.rs` / `weetodd.rs` 的注释里，并且有测试盯着。
//!
//! 这里把它们**序列化**给 UI，让界面上根本选不出无效组合。
//!
//! # 为什么不学 LumenX 写一份 YAML 目录
//!
//! 参考项目 LumenX 用 `config/model_catalog/*.yaml` 描述模型能力。对它合适，
//! 因为它的模型是云端 API、约束写在服务商文档里。对我们不合适：
//! 我们的约束是**测出来的**，而且已经有代码和测试承载
//! （`ImageEngine::negative_works()` 等）。再写一份 YAML 就是第二份真相，
//! 必然和代码漂移 —— 而漂移的表现是「UI 显示了一个不起作用的输入框」，
//! 又是一个静默失败。
//!
//! 所以：**Rust 是唯一真相，这里只是它的投影。**

use serde::{Deserialize, Serialize};

use crate::agent_runtime::image_engine::{self, ImageEngine};
use crate::agent_runtime::weetodd;

/// 一种生成模式。UI 的模式选择器直接渲染这个列表。
///
/// **`label` 必须短。** 它渲染在等宽的分段控件里，四个一行 ——
/// 「参考图生视频」六个字会换行，一格变两行高，整排跟着长高。
/// 两三个字为准（和 LumenX 的「文生 / 图生 / 参考生 / 编辑」同一节奏），
/// 说明放悬浮提示里，别放按钮上。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ModeSpec {
    pub id: &'static str,
    /// 完整名称。用在句子里（「『文生图』没有可用引擎」）和悬浮说明里。
    pub label: &'static str,
    /// **按钮上的短名字。** 模式选择器是等宽分段，长名字会换行把整排撑高。
    /// 和 `label` 分开存：只改 `label` 的话，提示句会变成「『文生』没有可用引擎」。
    pub short: &'static str,
    /// "image" | "video" —— 决定放在模式选择器的哪一组
    pub kind: &'static str,
    /// 需要几张输入图（0 = 纯文生）。UI 据此决定显示不显示媒体输入区。
    pub images: u8,
    /// 媒体输入区的标题。**每个模式不一样** —— 「参考图」和「首帧图」
    /// 是两件事，标签写错会让人塞错图而看不出问题。
    pub media_label: Option<&'static str>,
    /// 哪些引擎支持这个模式。
    pub engines: Vec<&'static str>,
}

/// 一个可选的出图/出片尺寸。
///
/// `verified` = **这台机器上真跑过**。没跑过的不是不能选，但要标出来 ——
/// 这个项目里「没验过」和「验过」是两种东西，混在一起列会让人以为都一样稳。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SizeOption {
    pub label: &'static str,
    pub width: u32,
    pub height: u32,
    pub verified: bool,
    /// 实测耗时（秒）。`None` = 没测过这个点。
    ///
    /// **代价要在点下去之前就看得见。** 1080p 出片 5 秒要 14 分钟、吃 104 GB，
    /// 让人点完等着才发现是最糟的形式。
    pub secs: Option<u64>,
    /// 实测峰值内存（GB）。
    pub peak_gb: Option<f64>,
}

/// 时长的取值范围。**每个引擎不一样，而且各有各的脾气。**
///
/// 固定几档（5/10/15/20）不合适 —— H3 的合法范围是 2.5~15，
/// 5 的整数倍在它身上没有意义。所以给范围让 UI 画步进控件。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DurationSpec {
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub default: f64,
    /// 超过这个值会被切成多段接续。`None` = 不分段。
    ///
    /// LTX 是 5 秒一段：给 7 秒会补齐到 10，**而那时每段都要单独写提示词，
    /// 否则场面会飘**（见 `window_prompts` 的实测）。UI 要提示这件事。
    pub window_secs: Option<f64>,
    /// 直接显示给用户的约束说明。
    pub note: Option<String>,
}

/// 一个引擎的能力。**每一条都对应一个实测结论或一处代码约束。**
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct EngineSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub kind: &'static str,
    /// 负向提示词**真的起作用**吗。false 时 UI 不显示那个输入框 ——
    /// 显示一个不起作用的框比不显示更糟。
    pub negative_prompt: bool,
    /// 提示词写长了会不会被摊薄（SDXL 的 77 token 分块取平均）。
    pub long_prompt_ok: bool,
    /// 该用多少步。`None` = **不要暴露这个参数**（视频那两条由预设内部决定）。
    pub steps: Option<u32>,
    /// 支持挂 LoRA。
    pub loras: bool,
    /// 单次大约多久（秒）。用来在按钮上写「预计 …」，别让人干等。
    pub secs: u64,
    /// 给 UI 直接显示的实测提示，**一句一条**。
    pub notes: Vec<String>,
    /// 可选尺寸。第一个是默认。
    pub sizes: Vec<SizeOption>,
    /// 一次最多排几个。
    ///
    /// **这不是图里的 `batch_size`。** 我们的建图里 `batch_size` 恒为 1，
    /// 一次多张是**多次提交**、由客户端队列串行发出去 —— ComfyUI 本身串行
    /// 执行，并发发出去只会让后面的计时器空转成「超时」（踩过：一次三张
    /// 固定第三张失败）。而把帧数塞进 `batch_size` 会直接把机器搞崩。
    pub max_batch: u8,
    /// 参考生图引擎吃几张参考图（0 = 不是参考生图引擎）。界面据此决定放几个上传槽、
    /// 槽上写「场景 / 人物」。**顺序就是含义**：两个引擎都是场景在前训练的。
    pub reference_slots: u8,
    /// **工作区（项目模型设置）里显不显示。** 用户明确不要 Pony 进工作区。
    /// 参考生图引擎只进「分镜首帧」那一栏（资产图是文生的）。
    pub project_settings: bool,
    /// 时长范围。`None` = 出图，没有时长这个维度。
    pub duration: Option<DurationSpec>,
}

/// 整份目录。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CapabilityCatalog {
    pub modes: Vec<ModeSpec>,
    pub engines: Vec<EngineSpec>,
    /// 出图默认引擎（2026-09-01 同题实测定的）。
    pub default_image_engine: &'static str,
    /// 视频默认引擎。
    pub default_video_engine: &'static str,
    /// 资产出图能选的画幅。**从实测尺寸推出来** —— 前端不该另写一份，
    /// 另写就会出现「界面上能选、送到引擎报错」。
    pub aspects: Vec<AspectOption>,
}

fn image_engine_spec(e: ImageEngine) -> EngineSpec {
    let mut notes: Vec<String> = Vec::new();
    if !e.negative_works() {
        notes.push(
            "这个引擎的负向提示词结构性无效（cfg=1，负向通路被清零），所以不显示那个输入框。"
                .into(),
        );
    }
    if e.dilutes_long_prompts() {
        notes.push(
            "提示词超过 77 token 会被分块取平均而摊薄 —— 压到 150 字以内的关键词最稳。".into(),
        );
    }
    if e.reference_slots() > 0 {
        notes.push(format!(
            "照着参考图出新构图：场景图放第一张、人物图放后面，最多 {} 张。不吃 LoRA。",
            e.reference_slots()
        ));
    }
    if e == ImageEngine::Krea2Edit {
        notes.push("对照实测里脸最像资产（多 seed 平均）。参考槽只有两个。".into());
    }
    if e == ImageEngine::Qwen21Edit {
        notes.push("和「Qwen-Image 2.1」是同一份权重，只是接了参考图。最多 10 张，比另外两个参考引擎多。".into());
        notes.push("出图的**长宽比跟第一张参考图**走（节点自己开 latent），这里选的画幅只决定面积。".into());
    }
    if e == ImageEngine::Qwen21 {
        notes.push("Qwen3-VL-8B 编码器，中文原生、长提示词不摊薄。".into());
        // **不是因为权重被改过** —— 下的就是上游原版（`abenzerps/Qwen-Image-2.1-GGUF`，
        // 那个叫 Uncensored 的仓库 README 自己写着权重一个字节没动）。
        // 是这个基础模型本来就不带独立的内容过滤。2026-09-22 实测：全裸提示词直接出图，
        // 不拒绝、不打码、不自动加衣服。
        notes.push("**这一个没有内容过滤**（实测全裸提示词直接出图）。别的引擎拒掉的题材找它。".into());
        notes.push("211s 一张，比 Krea 2（108s）慢一倍 —— 不是试稿那一档。".into());
        notes.push("只做文生图。它的 VAE 是四通道 RGBA，通用的图生图/局部重绘编码在它身上出坏图，所以那两条路会回落别的引擎。".into());
    }
    EngineSpec {
        id: e.id(),
        label: e.label(),
        kind: "image",
        negative_prompt: e.negative_works(),
        long_prompt_ok: !e.dilutes_long_prompts(),
        steps: Some(image_engine::default_steps(e)),
        loras: e.reference_slots() == 0,
        secs: e.secs_per_job(),
        notes,
        sizes: image_sizes(),
        max_batch: 4,
        reference_slots: e.reference_slots() as u8,
        // 参考生图引擎只给分镜首帧用（资产图是文生的，那一栏由前端按
        // `reference_slots` 滤掉、后端存设置时再拦一次）。
        project_settings: true,
        duration: None,
    }
}

/// 出图的可选画幅。
///
/// **全身人物一定要竖幅** —— 1:1 下人物被压缩，脸和手的像素预算都不够，
/// 这也是手指畸形最常见的间接原因（CLAUDE.md 记着）。所以竖幅排在横幅前面。
/// 出图的可选画幅。数字都是 2026-09-04 实测（见 7.20）。
///
/// **4K（3840×2160）不在列表里** —— 实测峰值 210.65 GB，而 MPS 的上限是
/// 107.52 GiB，857 秒后失败。吃内存的不是像素是注意力：像素涨 8 倍，
/// 注意力涨 63 倍。要 4K 就出 2K 再用 RealESRGAN 放大，几秒的事。
pub fn image_sizes() -> Vec<SizeOption> {
    vec![
        SizeOption { label: "1024×1024 方", width: 1024, height: 1024,
                     verified: true, secs: Some(45), peak_gb: None },
        SizeOption { label: "832×1216 竖（人物全身）", width: 832, height: 1216,
                     verified: true, secs: Some(45), peak_gb: None },
        SizeOption { label: "1216×832 横", width: 1216, height: 832,
                     verified: true, secs: Some(45), peak_gb: None },
        SizeOption { label: "2048×2048 2K 方", width: 2048, height: 2048,
                     verified: true, secs: Some(287), peak_gb: Some(66.66) },
        SizeOption { label: "2560×1440 2K 宽", width: 2560, height: 1440,
                     verified: true, secs: Some(242), peak_gb: Some(57.40) },
        // ---- 2026-09-13 新增的主流尺寸。**全部 `verified: false`、不填秒数** ——
        // 这台机器上没跑过，不借别的尺寸的数字。实测之后再翻成 true。
        //
        // 用户点名缺 1024×576：做视频参考图/首帧时，1216×832 是 3:2，要裁一截。
        //
        // **720p 是 1280×736 不是 1280×720**：每个尺寸必须是 32 的倍数
        // （LTX 的 VAE 下采样是 32，见测试 `每个引擎都有尺寸和批量上限`），
        // 720÷32=22.5。镜头出片 `shot_render_size` 用的也是 736。
        SizeOption { label: "1024×576 16:9", width: 1024, height: 576,
                     verified: false, secs: None, peak_gb: None },
        SizeOption { label: "1280×736 16:9（≈720p）", width: 1280, height: 736,
                     verified: false, secs: None, peak_gb: None },
        SizeOption { label: "1344×768 16:9（SDXL 桶）", width: 1344, height: 768,
                     verified: false, secs: None, peak_gb: None },
        SizeOption { label: "1920×1088 16:9（≈1080p）", width: 1920, height: 1088,
                     verified: false, secs: None, peak_gb: None },
        SizeOption { label: "576×1024 9:16", width: 576, height: 1024,
                     verified: false, secs: None, peak_gb: None },
        SizeOption { label: "736×1280 9:16（≈720p）", width: 736, height: 1280,
                     verified: false, secs: None, peak_gb: None },
        SizeOption { label: "768×1344 9:16（SDXL 桶）", width: 768, height: 1344,
                     verified: false, secs: None, peak_gb: None },
        SizeOption { label: "1088×1920 9:16（≈1080p）", width: 1088, height: 1920,
                     verified: false, secs: None, peak_gb: None },
        SizeOption { label: "1152×864 4:3", width: 1152, height: 864,
                     verified: false, secs: None, peak_gb: None },
        SizeOption { label: "864×1152 3:4", width: 864, height: 1152,
                     verified: false, secs: None, peak_gb: None },
        SizeOption { label: "1536×640 21:9", width: 1536, height: 640,
                     verified: false, secs: None, peak_gb: None },
        // 和 H3 的 21:9 出片尺寸一模一样（长边上限 1344 内正好 21:9），
        // 做 21:9 视频的首帧用这一档就不用裁。1536×640 其实是 2.4:1。
        SizeOption { label: "1344×576 21:9（和 H3 出片同尺寸）", width: 1344, height: 576,
                     verified: false, secs: None, peak_gb: None },
    ]
}

/// 画幅 id。**原来那三个一个字都不能改** —— `portrait` / `landscape` / `square`
/// 存在项目设置里（`character_aspect: "portrait"` 这种），改了旧项目就读不回来。
///
/// 以前 id 是按横竖方向推出来的，一个方向只有一个尺寸时没问题；
/// 加了 1024×576 之后就有两个「横」，`size_for_aspect("landscape")` 会
/// **静默返回先找到的那一个**。所以老三个钉死，新的用尺寸本身当 id。
fn aspect_id(width: u32, height: u32) -> String {
    match (width, height) {
        (1024, 1024) => "square".into(),
        (832, 1216) => "portrait".into(),
        (1216, 832) => "landscape".into(),
        _ => format!("{width}x{height}"),
    }
}

/// 一个画幅选项。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AspectOption {
    /// `"portrait" | "square" | "landscape"`
    pub id: String,
    pub label: String,
    pub width: u32,
    pub height: u32,
}

/// 资产出图的画幅选项。
///
/// **从 `image_sizes()` 里推出来，不另写一张表。**
///
/// 另写一张的话，界面上就会出现引擎根本产不出的尺寸 —— 提交时才报错，
/// 而那时用户已经填完提示词、等了几十秒。LumenX 就有这个毛病：
/// 它给 9:16 / 3:4 / 1:1 / 4:3 / 16:9 五档（云端模型的习惯），
/// 接到本机之后出片直接报 `WT resolution must be 384p or 512p`。
///
/// 只取**实测过**（`verified`）且不是 2K 那两档的 —— 2K 出一张要 4~5 分钟，
/// 不该是设定卡的默认选项。
pub fn aspect_options() -> Vec<AspectOption> {
    image_sizes()
        .into_iter()
        .filter(|s| s.verified && s.width <= 1536 && s.height <= 1536)
        .map(|s| {
            let label = match s.width.cmp(&s.height) {
                std::cmp::Ordering::Less => "竖",
                std::cmp::Ordering::Equal => "方",
                std::cmp::Ordering::Greater => "横",
            };
            AspectOption {
                id: aspect_id(s.width, s.height),
                label: format!("{label} {}×{}", s.width, s.height),
                width: s.width,
                height: s.height,
            }
        })
        .collect()
}

/// 画幅 id → 实际尺寸。认不出的回 `None`，**不要瞎给一个** ——
/// 给错了就是出一张比例不对的图，而且不报错。
pub fn size_for_aspect(id: &str) -> Option<(u32, u32)> {
    aspect_options().into_iter().find(|a| a.id == id).map(|a| (a.width, a.height))
}

/// 组装目录。
/// LTX 一段的秒数。和 `weetodd::MAX_SINGLE_WINDOW_SECS` 是同一个数 ——
/// 这里引过来，别让两处各写一个。
const MAX_WINDOW: f64 = weetodd::MAX_SINGLE_WINDOW_SECS;

/// 有示例图的构图模板 id。
///
/// **和前端 `assetPrompt.ts` 的 `CHARACTER_TEMPLATES` 是同一份清单**，
/// 有测试直接读那个文件比对 —— 两边分叉的表现是「卡片上写着『还没出示例图』，
/// 而图就在盘上」，不报错、只是空着。
pub const TEMPLATE_THUMB_IDS: &[&str] =
    &["avatar", "portrait", "simple", "detailed", "panel5", "quadzh", "bible", "quad", "panel7", "propcard"];

pub fn catalog() -> CapabilityCatalog {
    use ImageEngine::*;
    // **Qwen 2.1 不进 `image_ids`** —— 那份列表还喂场景宫格，而宫格版式在它上面没验过
    // （和 Pony 同一条理由）。它只进文生 / 图生。
    let image_ids = vec!["zimage", "sdxl", "krea2"];
    let with_qwen21 = |mut v: Vec<&'static str>| {
        v.push("qwen21");
        v
    };
    // **Pony 只进文生 / 图生，不进 `image_ids`** —— 那份列表还喂场景宫格，
    // 而 Pony 在宫格版式上没验过。

    let engines = vec![
        image_engine_spec(ZImage),
        image_engine_spec(Sdxl),
        image_engine_spec(Krea2),
        image_engine_spec(Krea2Edit),
        image_engine_spec(Qwen21),
        image_engine_spec(Qwen21Edit),
        EngineSpec {
            id: "ltx",
            label: "LTX 2.5 (WeeTodd/MLX)",
            kind: "video",
            // WeeTodd 这条路**根本没有负向提示词入口**，不是无效而是不存在
            negative_prompt: false,
            long_prompt_ok: true,
            // 蒸馏模型，步数由预设决定，暴露出来只会被人按 SDXL 的习惯改坏
            steps: None,
            loras: true,
            secs: 121,
            sizes: vec![
                SizeOption { label: "768×512", width: 768, height: 512,
                             verified: true, secs: Some(121), peak_gb: Some(22.88) },
                SizeOption { label: "1344×768", width: 1344, height: 768,
                             verified: true, secs: Some(473), peak_gb: Some(123.45) },
                SizeOption { label: "1920×1088 1080p", width: 1920, height: 1088,
                             verified: true, secs: Some(839), peak_gb: Some(103.78) },
            ],
            max_batch: 2,
            reference_slots: 0,
            project_settings: true,
            // 节点允许 0.25~30，但我们最多 4 段 × 5 秒 = 20 秒；
            // 低于 1 秒（24 帧）没实用价值
            duration: Some(DurationSpec {
                min: 1.0, max: 20.0, step: 0.5, default: 5.0,
                window_secs: Some(MAX_WINDOW),
                note: Some(
                    "超过 5 秒会切成多段接续。**每段都要单独写提示词并完整重述不变的部分**，\
                     否则机位和布景会一段一个样（实测：只写动作比不改还差）。"
                        .into(),
                ),
            }),
            notes: vec![
                weetodd::NO_NEGATIVE_NOTE.to_string(),
                format!(
                    "超过 {:.0} 秒会自动切成多段接续 —— 那时每段都要完整重述不变的部分，否则场面会飘。",
                    weetodd::MAX_SINGLE_WINDOW_SECS
                ),
            ],
        },
        EngineSpec {
            id: "h3",
            label: "MiniMax H3 (WeeTodd/MLX)",
            kind: "video",
            negative_prompt: false,
            long_prompt_ok: true,
            steps: None,
            loras: true,
            secs: weetodd::H3Tier::Standard.secs(),
            // **这两档就是原来那个「画质档位」。** `H3Tier` 是按长边 ≤640
            // 推出来的（`video_tool.rs`），所以选画幅就是选档 —— 以前界面上
            // 还另有一个「画质档位」选择器，是这一栏的重复品，而且它选的值
            // 一个字都没发下去。拆掉的理由见 DECISIONS.md 2026-09-12。
            sizes: vec![
                SizeOption { label: "640×384 预览", width: 640, height: 384,
                             verified: true, secs: Some(222), peak_gb: Some(15.81) },
                // 384 秒是 `H3Tier::Standard` 的文档里记的实测值。
                // **`verified` 仍然是 false** —— 那个字段说的是「这台机器上
                // 真跑过这个尺寸」，我没跑过，不替它签字。
                SizeOption { label: "768×512", width: 768, height: 512,
                             verified: false, secs: Some(weetodd::H3Tier::Standard.secs()),
                             peak_gb: None },
            ],
            max_batch: 2,
            reference_slots: 0,
            project_settings: true,
            // 节点自己的范围是 2.5~15，并注明低于 5 秒是实验性的
            duration: Some(DurationSpec {
                min: 2.5, max: 15.0, step: 0.5, default: 5.0,
                window_secs: None,
                note: Some(
                    "低于 5 秒是实验性的 —— 会向上吸附到 H3 的 17n+5 帧栅格，\
                     实际时长可能比你填的长一点。"
                        .into(),
                ),
            }),
            notes: vec![
                "H3 读的是结构化剧本，不是散文 —— 台词写进 `<d>[Chinese] …</d>` 会变成同步中文语音。"
                    .into(),
                "图生 / 首尾帧 / 参考图这三种要加载 66.73 GB 的视觉编码器，内存峰值约 54 GB。"
                    .into(),
            ],
        },
    ];

    let modes = vec![
        ModeSpec {
            id: "t2i", label: "文生图", short: "文生", kind: "image", images: 0,
            media_label: None, engines: with_qwen21(image_ids.clone()),
        },
        ModeSpec {
            // **四个引擎都真的有图生图**（`image_engine::attach_i2i`）。
            // 以前只有 SDXL 那一支接了源图，别的会回落 SDXL —— 那不是
            // 模型不支持，是我们的图没接。现在接上了，这里也就不用限制。
            id: "i2i", label: "图生图", short: "图生", kind: "image", images: 1,
            // **Qwen 2.1 不在图生图里** —— 它的 VAE 吃四通道 RGBA，通用的
            // 「空 latent 换成源图 latent」改写喂三通道，实测出坏图且任务 success
            // （见 `image_engine::supports_i2i` 的注释）。
            media_label: Some("源图片"), engines: image_ids.clone(),
        },
        ModeSpec {
            // **参考生图**：资产图（场景 + 人物）→ 新构图。和「图生」不是一回事 ——
            // 图生是改这张图，参考生是照着这几张图画一张新的。
            // `images` 是槽位上限；实际用几张看引擎（Krea 2 两张、Qwen 三张）。
            // `images` 是界面上的槽位上限，取**最大的那个引擎**；实际用几张看引擎
            // （Krea 2 两张、Qwen Edit 三张、Qwen 2.1 十张）。
            id: "ref2i", label: "参考生图", short: "参生", kind: "image", images: 10,
            media_label: Some("参考图（场景在前、人物在后）"), engines: vec!["krea2_edit", "qwen21_edit"],
        },
        ModeSpec {
            // 设定卡只有 SDXL 出得了 —— 多视图靠的是 SDXL 的 LoRA
            id: "sheet", label: "设定卡", short: "设定", kind: "image", images: 0,
            media_label: None, engines: vec!["sdxl"],
        },
        ModeSpec {
            // **场景多方位宫格：一次采样出 N 个机位，然后切开。**
            //
            // 不挂任何版式 LoRA，所以**四个引擎都行** —— 这一点和设定卡
            // 那几档正相反。纯文字，不要源图。
            //
            // 它解决的是场景一致性：同一次去噪过程里画出来的几个视角，
            // 光线、材质、家具款式必然同源，这比任何提示词都硬。
            id: "scene_sheet", label: "场景宫格", short: "场景", kind: "image", images: 0,
            media_label: None, engines: image_ids.clone(),
        },
        ModeSpec {
            // **四视图：大脸特写 + 正/侧/背，必须有源图。**
            // 这是「身份参考」那一档 —— 转身的脸 135~199 px，在可用下限之上；
            // 而下面那个七分区的转身只有 110 px，当不了身份参考。
            id: "quad_view", label: "四视图", short: "四图", kind: "image", images: 1,
            media_label: Some("角色源图"), engines: vec!["krea2"],
        },
        ModeSpec {
            // **完整设定表：七个区，必须有源图。** 只有 Krea 2 —— 版式 LoRA
            // 只有它这一版，挂到别的引擎上加载「成功」但一点作用都没有。
            id: "entity_sheet", label: "参考表", short: "参考", kind: "image", images: 1,
            media_label: Some("角色源图"), engines: vec!["krea2"],
        },
        ModeSpec {
            // 姿势控制走 ControlNet，同样只在 SDXL 上
            id: "pose", label: "姿势", short: "姿势", kind: "image", images: 1,
            media_label: Some("姿势参考图"), engines: vec!["sdxl"],
        },
        ModeSpec {
            id: "t2v", label: "文生", short: "文生", kind: "video", images: 0,
            media_label: None, engines: vec!["ltx", "h3"],
        },
        ModeSpec {
            id: "i2v", label: "图生", short: "图生", kind: "video", images: 1,
            media_label: Some("首帧图片"), engines: vec!["ltx", "h3"],
        },
        ModeSpec {
            id: "fl2v", label: "首尾帧", short: "首尾帧", kind: "video", images: 2,
            media_label: Some("首帧与末帧"), engines: vec!["ltx", "h3"],
        },
        ModeSpec {
            // 参考图生视频只有 H3 有（ref2va 分区）
            id: "r2v", label: "参考生", short: "参考生", kind: "video", images: 1,
            media_label: Some("参考图片"), engines: vec!["h3"],
        },
    ];

    CapabilityCatalog {
        modes,
        engines,
        default_image_engine: crate::agent_runtime::image_engine::DEFAULT_ENGINE.id(),
        default_video_engine: "ltx",
        aspects: aspect_options(),
    }
}

/// 一次生成请求。字段名和前端 store 里的一一对应。
#[derive(Debug, Clone, serde::Deserialize)]
pub struct GenerateArgs {
    /// **画布 H3 出片按官方格式整理提示词**（2026-10-06，[`crate::h3_spec::canvas_official`]）。
    /// 只动格式不动正文；默认关 —— 会改变已经验证过的画布模板实际发给 H3 的内容，先对照再开。
    #[serde(default)]
    pub h3_official_format: Option<bool>,
    pub mode: String,
    pub engine: String,
    pub prompt: String,
    /// **H3 加速档**（和工作台「出片设置」同一套 id：`lightx2v` / `standard` / `staged` / `dense`）。
    /// 画布视频节点的「加速」选项（用户 2026-10-08：「昨天的加速节点在画布里没有」）。不传 = 照旧按尺寸自动（Larry 4 步）
    #[serde(default)]
    pub h3_quality: Option<String>,
    /// **参考音频**（本机路径，`r2v` + H3 才用，最多 3 段）。画布「音频素材」节点接进视频节点时带上（2026-10-08）
    #[serde(default)]
    pub audios: Vec<String>,
    #[serde(default)]
    pub negative_prompt: String,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub seed: Option<u64>,
    /// 本机绝对路径。按模式需要 0~2 张
    #[serde(default)]
    pub images: Vec<String>,
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub generate_audio: Option<bool>,
    /// **别挂设置里那两个 Z-Image 常驻 LoRA。** `None` = 照旧。
    ///
    /// # 为什么资产出图必须关掉它
    ///
    /// 设置面板里配的常挂 LoRA，生效条件是「纯文生图 + 这次没自己传」
    /// （`should_apply_configured_zimage_loras`）。而**传一个空数组等于
    /// 没传** —— 资产出图正好是这种情况，于是它们被静默挂上。
    ///
    /// 2026-09-12 实拍：一批 11 个资产全废。配着
    /// `Pintrest_ZiT`(0.6) + `Straight_hair_zimgt`(0.8) 两个，
    /// 结果是**道具「键盘」那张图里有三格是人像和头发** ——
    /// 直发人像 LoRA 往任何画面里塞人。角色四视图的版式也稳不住：
    /// 两个 LoRA 和那段四百字的构图规范抢控制权，四张里只有一张版式对。
    ///
    /// 它们在资产工作台上**完全不显示**，却在起作用 —— 又一个「看着没有，
    /// 实际有」。资产图的画风该由项目的风格定调决定，不该由一个
    /// 为日常出图配的 LoRA 决定。
    #[serde(default)]
    pub auto_loras: Option<bool>,
    /// 场景宫格要出的方位。**空 = 用工具的兜底那套**（全景/正打/反打/入口），
    /// 而那套笼统说法实测会让几格出成同一个机位 —— 所以界面上要让人填。
    ///
    /// 一个方位就是空间契约里的一个 `SceneView`，描述写成
    /// 「两两关系 + 视线终点」才区分得开。
    #[serde(default)]
    pub viewpoints: Vec<ViewpointArg>,
    /// 宫格切完之后逐格 i2i 精修。
    ///
    /// **默认不开**：每格多一次出图。本机 3×3 每格只有 512×341，
    /// 不精修的话那是布局稿不是成品 —— 工具的返回里会说这句话。
    #[serde(default)]
    pub refine: Option<bool>,
    /// 图生图的重绘幅度。`None` = 用工具的默认值（0.65）。
    ///
    /// **只对带源图的那条有意义** —— 文生图传了也不会生效（那边 denoise 恒为 1）。
    #[serde(default)]
    pub denoise: Option<f64>,
    /// 完整设定表的参考图保真度旋钮。`None` = 用实测的默认值（4.0）。
    ///
    /// **只对 `entity_sheet` 有意义。** 实测它改善的是构图/指令服从度
    /// （动作三姿真的不同了），不是人脸相似度 —— 见 `entity_sheet` 的模块注释。
    #[serde(default)]
    pub ref_boost: Option<f64>,
    /// 参考表的实体类型（`HUMAN` / `ANIMAL` / …）。认不出的会回落。
    #[serde(default)]
    pub entity_type: Option<String>,
    /// 参考表上写的名字。
    ///
    /// **以前这里没有这个字段**：界面一直在送 `entity_name`，反序列化时被静默丢掉，
    /// 模板里的名字永远是兜底的 `Subject`（2026-09-15 接补表情时发现）。
    #[serde(default)]
    pub entity_name: Option<String>,
    /// 参考表的身份锚点，一条一行。
    ///
    /// **提示词模板只有一个所有者，在 `entity_sheet::sheet_prompt`。**
    /// 界面只送零件（名字/类型/锚点/风格句），不自己拼那段带方括号的模板 ——
    /// 拼两份的表现是「改了界面那份，模型收到的还是旧的」。
    #[serde(default)]
    pub identity_locks: Vec<String>,
    /// 参考表用的风格句。空 = 用模板自带的默认。
    #[serde(default)]
    pub style_line: Option<String>,
    /// 工作区根目录。出的文件落在它的 `generated/` 下。
    #[serde(default)]
    pub workspace_path: Option<String>,
    /// 选中的 LoRA 及强度。
    #[serde(default)]
    pub loras: Vec<LoraPick>,
    /// 显式文件名。不给就自动生成一个唯一的。
    #[serde(default)]
    pub output_filename: Option<String>,
    /// 由提示词里的 `[角色:x]` 解析出来的参考图（本机绝对路径）。
    ///
    /// **由 `studio_generate` 填，前端不用管。** 放在这里而不是在
    /// [`to_tool_call`] 里现解析，是为了让那个函数保持无副作用、可单测。
    #[serde(default)]
    pub reference_images: Vec<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct LoraPick {
    pub name: String,
    pub strength: f64,
}

/// 宫格里的一个方位。
///
/// **描述比名字重要得多。** 只给名字（「反打」）的话模型会画出一个
/// 几乎一样的角度；要写成「从柜台内侧朝卷帘门看，尽头是门外的街」
/// 才真的转得过去。这一条是实测出来的，见 `scene_sheet::grid_prompt`。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ViewpointArg {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

/// 一次生成的结果。
#[derive(Debug, Clone, Serialize)]
pub struct GenerateOutput {
    /// 出的文件的本机绝对路径
    pub path: Option<String>,
    /// **所有**落盘的文件。宫格这条路一次出 1 张联合图 + N 张切好的格子，
    /// 只返回 `path` 的话后面 N 张就丢了 —— 而它们才是要入库的东西。
    #[serde(default)]
    pub paths: Vec<String>,
    /// 工具原样返回的说明（含 seed、耗时、模型名）
    pub message: String,
    /// **跑成功了、但有话要说**。见 [`notices`]。
    pub notes: Vec<String>,
    pub ok: bool,
}

/// 从工具返回里摘出「成功但有话要说」的那几条。
///
/// # 为什么必须摘出来
///
/// 工具那边把回落说明、LoRA 名字写错、提示词摊薄、步数不对这几类告警
/// 用 `⚠️ ` 开头拼在 summary 最前面，注释里写着「回落说明必须顶到最前：
/// 用户以为在用 Z-Image，实际拿到 SDXL 的图，**不说等于骗人**」。
///
/// 而创作台那边是 `error: out.ok ? undefined : out.message` ——
/// **成功时整段 message 被丢掉**。工具尽了责，界面在最后一行扔了。
/// 结果就是：选 Z-Image + 图生图，实际出的是 SDXL 的图，而界面一声不吭。
///
/// # 切法
///
/// 按 `- **Seed**` 那一行切，而不是逐行认 `⚠️` 开头 ——
/// 告警本身可能是多行的（LoRA 那条会把本机有哪些列出来）。
pub fn notices(summary: &str) -> Vec<String> {
    let head = match summary.find("- **Seed**") {
        Some(i) => &summary[..i],
        // 没有 Seed 那一行说明不是出图的返回（视频那条）—— 整段里只挑告警。
        None => summary,
    };
    head.split('⚠')
        .skip(1)
        .map(|part| part.trim_start_matches('️').trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// 把 Studio 的一次请求翻译成 agent 工具的参数。
///
/// **这里只做翻译，不做决策。** 选引擎、挑模型、回落、建图全部在工具里 ——
/// Studio 和对话必须走同一条路径，否则会出现「在对话里能跑、在界面上不行」
/// 这种最难查的分歧。
///
/// 返回 `(工具名, 参数)`。
pub fn to_tool_call(a: &GenerateArgs) -> Result<(&'static str, serde_json::Value), String> {
    use serde_json::json;
    let mut args = json!({
        "prompt": a.prompt,
        "width": a.width,
        "height": a.height,
    });
    // **运动提示词只发给出片的模式。** 和下面负向提示词那条同一个道理：
    // `generate_image` 不认这个键，传了不报错、也不生效，但日志里会留下
    // 一条「运动提示词已下发」的记录，排查时会被它带偏一整轮。
    if a.mode.ends_with("2v") || a.mode == "fl2v" {
        args["motion_prompt"] = json!(a.prompt);
        // 加速档也只发给出片的模式（同上：出图工具不认它）
        if let Some(q) = a.h3_quality.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
            args["h3_quality"] = json!(q);
        }
    }
    // **每次生成一个唯一文件名。**
    //
    // 工具默认用 `img_{seed}.png` —— 同一个 seed 换个模式或改了提示词再跑，
    // 会**覆盖上一张**，而画廊里那张旧卡片还指着同一个路径，
    // 于是它显示的是新图。任务成功、文件也在，只是内容对不上，
    // 又一个静默失败。带上模式和毫秒时间戳就不会撞。
    if let Some(name) = &a.output_filename {
        args["output_filename"] = json!(name);
    } else {
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let ext = if a.mode.ends_with("2v") || a.mode == "fl2v" { "mp4" } else { "png" };
        args["output_filename"] = json!(format!("studio_{}_{ms}.{ext}", a.mode));
    }
    if let Some(seed) = a.seed {
        args["seed"] = json!(seed);
    }
    if let Some(ga) = a.generate_audio {
        args["generate_audio"] = json!(ga);
    }
    // **只在明说「别挂」时才发。** 不发 = 照旧，老行为一个字没变。
    if a.auto_loras == Some(false) {
        args["auto_loras"] = json!(false);
    }
    // **负向提示词只在真的起作用时才传。** 传给不认它的引擎不会报错，
    // 只是白传 —— 但那样日志里会留下一条看起来生效了的记录，误导排查。
    let honors_negative = catalog()
        .engines
        .iter()
        .find(|e| e.id == a.engine)
        .map(|e| e.negative_prompt)
        .unwrap_or(false);
    if honors_negative && !a.negative_prompt.trim().is_empty() {
        args["negative_prompt"] = json!(a.negative_prompt);
    }

    // LoRA 用工具已有的那套参数格式。**架构过滤在 UI 上做过了**，
    // 但工具那边还有一层 `lora_arch` 兜底 —— 两层都在，因为挂错了不报错。
    if !a.loras.is_empty() {
        args["loras"] = serde_json::Value::Array(
            a.loras
                .iter()
                .map(|l| json!({"name": l.name, "strength": l.strength}))
                .collect(),
        );
    }

    let need = |n: usize| -> Result<(), String> {
        if a.images.len() < n {
            Err(format!("「{}」需要 {n} 张图，只给了 {}", a.mode, a.images.len()))
        } else {
            Ok(())
        }
    };

    match a.mode.as_str() {
        "t2i" => {
            args["engine"] = json!(a.engine);
            Ok(("generate_image", args))
        }
        "ref2i" => {
            need(1)?;
            args["engine"] = json!(a.engine);
            // **全部原样发过去、顺序不动**。挑哪几张、挑剩的怎么说，是工具那层的事 ——
            // 在这里截掉的话，「后面几张没用上」那句提示就发不出来了。
            args["reference_images"] = json!(a.images);
            Ok(("generate_image", args))
        }
        "i2i" => {
            need(1)?;
            args["engine"] = json!(a.engine);
            args["image_path"] = json!(a.images[0]);
            // **不给就别塞。** 塞一个 null 进去会盖掉工具那边的默认值判断
            // （`unwrap_or(有源图 ? 0.65 : 1.0)`），而那种偏差没人看得出来。
            if let Some(d) = a.denoise {
                args["denoise"] = json!(d.clamp(0.05, 1.0));
            }
            Ok(("generate_image", args))
        }
        "sheet" => {
            // 设定卡强制 SDXL —— 多视图靠的是 SDXL 的 LoRA，
            // 挂到别的引擎上不生效**且不报错**
            args["engine"] = json!("sdxl");
            args["layout"] = json!("character_sheet");
            Ok(("generate_image", args))
        }
        "quad_view" | "entity_sheet" => {
            // **完整设定表是图生图** —— 身份全部来自源图，纯文字出不了这个版式。
            need(1)?;
            // 引擎不写这里：`generate_image` 见到 `layout: entity_sheet` 就
            // **强制 Krea 2**（版式 LoRA 只有它这一版）。在这儿再写一遍就成了
            // 两个所有者，改一处另一处不跟着变。
            // **layout 用模式名本身** —— 两种版式共用这一段，差别全在 layout 上
            args["layout"] = json!(a.mode);
            args["image_path"] = json!(a.images[0]);
            if let Some(b) = a.ref_boost {
                args["ref_boost"] = json!(b.clamp(0.0, crate::agent_runtime::entity_sheet::REF_BOOST_MAX));
            }
            if let Some(t) = a.entity_type.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                args["entity_type"] = json!(t);
            }
            if let Some(n) = a.entity_name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                args["entity_name"] = json!(n);
            }
            let locks: Vec<&str> = a
                .identity_locks
                .iter()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect();
            if !locks.is_empty() {
                args["identity_locks"] = json!(locks);
            }
            if let Some(sl) = a.style_line.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                args["style_line"] = json!(sl);
            }
            Ok(("generate_image", args))
        }
        // **360° 全景 → 切四方位。**
        //
        // 和 `scene_sheet` 是两条路，解决的是同一个问题（一个场景要四个
        // 同源机位），但方式根本不同：
        //
        // | | 宫格 | 全景 |
        // |---|---|---|
        // | 四个机位怎么来 | **求**模型画四个不同的 | **算**出来的 |
        // | 一致性 | 提示词约束（实拍出过四格一样的） | 几何保证 |
        // | 依赖 | 无 | 360Redmond LoRA |
        // | 已知缺陷 | 机位可能拉不开 | 左右接缝不闭合 |
        "pano_views" => {
            // 引擎锁死 —— 装的是 Z-Image Turbo 版的 LoRA，别的底模挂不上
            args["engine"] = json!(crate::agent_runtime::panorama::PANO_ENGINE);
            args["layout"] = json!("pano_views");
            // **触发词拼在最前。** LoRA 的触发词是它被训练时的 caption 开头，
            // 排在一长段场景描述后面会被稀释。
            args["prompt"] = json!(format!(
                "{}, {}",
                crate::agent_runtime::panorama::PANO_TRIGGER,
                a.prompt.trim()
            ));
            // **等距圆柱必须 2:1。** 用户选的画幅在这条路上不生效 ——
            // 比例不对的话切出来的四个角度全错，而图看着是正常的。
            args["width"] = json!(1536);
            args["height"] = json!(768);
            Ok(("generate_image", args))
        }
        "scene_sheet" => {
            // **不锁引擎。** 宫格不挂任何版式 LoRA，哪个底模都出得来；
            // 默认 Z-Image 是因为它最快而且第一段本来就是布局稿。
            args["engine"] = json!(a.engine);
            args["layout"] = json!("scene_sheet");
            let views: Vec<serde_json::Value> = a
                .viewpoints
                .iter()
                .filter(|v| !v.name.trim().is_empty())
                .map(|v| json!({"name": v.name.trim(), "description": v.description.trim()}))
                .collect();
            // **一个方位都没有就别传这个键。** 传一个空数组进去，
            // 工具那边 `viewpoints_from_args` 会当成「给了但都不合法」，
            // 而它的兜底判据是「数组里一条都没解析出来」—— 行为一样，
            // 但日志里留下一条「视角已下发」的假记录。
            if !views.is_empty() {
                args["viewpoints"] = serde_json::Value::Array(views);
            }
            if a.refine.unwrap_or(false) {
                args["refine"] = json!(true);
            }
            Ok(("generate_image", args))
        }
        "pose" => {
            need(1)?;
            args["engine"] = json!("sdxl");
            args["pose_image"] = json!(a.images[0]);
            Ok(("generate_image", args))
        }
        "t2v" => {
            args["model"] = json!(a.engine);
            if let Some(d) = a.duration {
                args["duration_secs"] = json!(d.round() as u64);
            }
            Ok(("generate_video", args))
        }
        "i2v" => {
            need(1)?;
            args["engine"] = json!(a.engine);
            args["image_path"] = json!(a.images[0]);
            if let Some(d) = a.duration {
                args["duration_secs"] = json!(d.round() as u64);
            }
            Ok(("animate_image", args))
        }
        "fl2v" => {
            need(2)?;
            args["engine"] = json!(a.engine);
            args["image_path"] = json!(a.images[0]);
            args["end_image"] = json!(a.images[1]);
            if let Some(d) = a.duration {
                args["duration_secs"] = json!(d.round() as u64);
            }
            Ok(("animate_image", args))
        }
        "r2v" => {
            // 参考图有两个来源：媒体槽位里手动放的，和提示词里 `@` 出来的。
            // **两个都要用上** —— 只认一个的话，另一边就成了「点了没反应」。
            let mut refs = a.images.clone();
            for p in &a.reference_images {
                if !refs.contains(p) {
                    refs.push(p.clone());
                }
            }
            if refs.is_empty() {
                return Err(format!(
                    "「{}」需要 1 张参考图：拖一张进来，或者在提示词里 @ 一个资产",
                    a.mode
                ));
            }
            args["engine"] = json!("h3");
            args["image_path"] = json!(refs[0]);
            args["mode"] = json!("reference");
            // **上限取自单一来源。** 这里原来写死 `take(4)` —— 和
            // `cinema::shot_reference_images` 那个 `truncate(4)` 是同一个错数字。
            // 节点声明 `max=9`，MiniMax 官方规格 reference_image 也是 9 张。
            // 截在 4 的后果是第 5 张往后静默丢掉，而图和提示词两头都不报错。
            args["reference_images"] = json!(refs
                .iter()
                .take(crate::agent_runtime::video_tool::MAX_REFERENCE_IMAGES)
                .collect::<Vec<_>>());
            if !a.audios.is_empty() {
                args["reference_audios"] = json!(a.audios);
            }
            if let Some(d) = a.duration {
                args["duration_secs"] = json!(d.round() as u64);
            }
            Ok(("animate_image", args))
        }
        other => Err(format!("不认识的模式「{other}」")),
    }
}

/// 磁盘上的一个 LoRA。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LoraOption {
    /// ComfyUI 枚举出来的名字（**含子目录前缀**，如 `zimage/xxx.safetensors`）
    pub name: String,
    /// 去掉目录和扩展名后的显示名
    pub label: String,
    /// 认出来的架构：`zimage` / `sdxl` / `krea2` / `flux2` / `ltx` / `h3` / `unknown`
    pub arch: String,
    /// 猜出来的激活词。**是猜的** —— UI 必须显示出来并允许去掉。
    pub triggers: Vec<String>,
    /// 哪些引擎能用它
    pub engines: Vec<String>,
    /// 训练时的高频标签。
    ///
    /// **这不是作者写的说明**，是 kohya 记下的训练集标签，只能当线索。
    /// 13 个 LoRA 里只有 5 个有；`name` 字段更没用（`example_SDXL`、
    /// `tangStyle_sdxl_autodl_v36` 这种输出名）。剩下的靠用户自己写备注。
    pub concepts: Vec<String>,
    /// 用户自己写的备注。比训练标签优先显示。
    pub note: Option<String>,
    /// **作者标的激活词**（从 Civitai 之类抄下来存的），和 `triggers` 的区别是
    /// 它不是猜的。有它就该显示它、并且别再说「可能不准」。
    ///
    /// 为什么单开一个字段而不是覆盖 `triggers`：两者的可信度不一样，
    /// 界面上要能分开说。混成一个就没法告诉用户「这个是猜的、那个是确认的」。
    pub confirmed_triggers: Vec<String>,
    /// 成人向。按发布页作者标的记 —— **只标出来，不拦截**。
    pub nsfw: bool,
    /// 风格窗口的分类（见 `LoraNote::category`）
    pub category: String,
    /// 出处页面地址
    pub source_url: String,
    /// 示例图的本地路径（作者样例，`_anima_thumbs/`）。没有 = 界面显示占位
    pub thumb: Option<String>,
}

/// 从 `ss_tag_frequency` 里挑**概念词**（训练集里的高频标签）。
///
/// 和 `trigger_candidates` 正好相反：那边挑低频的造词，这里挑高频的普通词。
/// 实例：CharTurnXLRetag 出 `back view / side view / front view` ——
/// 一看就知道是转身多视图用的。
pub fn concept_words(tag_frequency: &serde_json::Value) -> Vec<String> {
    let mut totals: std::collections::BTreeMap<String, u64> = Default::default();
    if let Some(sets) = tag_frequency.as_object() {
        for sub in sets.values() {
            if let Some(tags) = sub.as_object() {
                for (tag, n) in tags {
                    *totals.entry(tag.trim().to_string()).or_default() +=
                        n.as_u64().unwrap_or(0);
                }
            }
        }
    }
    let mut v: Vec<(String, u64)> = totals
        .into_iter()
        // 太长的是整句训练描述，不是概念；`misc`/`all` 是 kohya 的占位垃圾
        .filter(|(t, n)| *n > 3 && t.len() <= 28 && !matches!(t.as_str(), "misc" | "all"))
        .collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v.into_iter().map(|(t, _)| t).take(5).collect()
}

/// 用户备注存哪儿。和模型放一起，跟着 LoRA 走。
fn notes_path() -> Option<std::path::PathBuf> {
    Some(crate::cinema_sheet::loras_dir()?.join("_anima_notes.json"))
}

/// 一条 LoRA 的人工标注。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LoraNote {
    #[serde(default)]
    pub note: String,
    /// 作者标的激活词。空 = 没标过，回落到猜的那些。
    #[serde(default)]
    pub triggers: Vec<String>,
    /// 成人向。**按发布页作者自己标的记**，不是我们判的。
    ///
    /// 只影响界面上标不标出来 —— **不做拦截**。这是个人原型，
    /// 拦截等于给自己加一道要绕的门；而不标出来的问题是：
    /// 挂了一个成人向 LoRA 去出角色资产，出来的东西不对劲而看不出为什么。
    ///
    /// `#[serde(default)]` 让老文件读得出来（没这个字段就是 false）。
    #[serde(default)]
    pub nsfw: bool,
    /// 风格窗口顶上的分类（画风 / 角色 / 服装 / 场景 / 功能 …）。空 = 没分过，界面归到「其它」。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub category: String,
    /// 出处（Civitai / HF 的页面地址）。示例图就是从这里下的作者样例。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source_url: String,
}

/// 示例图放在哪：LoRA 目录旁边的 `_anima_thumbs/<子目录>/<文件名>.jpg`，和备注一样跟着 LoRA 走。
pub fn lora_thumbs_dir() -> Option<std::path::PathBuf> {
    Some(crate::cinema_sheet::loras_dir()?.join("_anima_thumbs"))
}

/// 这个 LoRA 的示例（文件在才给）。`name` 是 ComfyUI 枚举出来的名字，含子目录。
///
/// 视频类 LoRA 的示例是一段 `.mp4`（作者样例视频），优先于图 —— 用户 2026-09-25：
/// 「没视频就找 gif，没 gif 就找静态图片」（gif 下来时就转成 mp4 了，体积小得多）。
fn lora_thumb(name: &str) -> Option<String> {
    lora_thumb_in(&lora_thumbs_dir()?, name)
}

fn lora_thumb_in(dir: &std::path::Path, name: &str) -> Option<String> {
    let stem = name.strip_suffix(".safetensors").unwrap_or(name);
    ["mp4", "jpg"]
        .iter()
        .map(|ext| dir.join(format!("{stem}.{ext}")))
        .find(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
}

/// 读备注。
///
/// **老格式是 `{"名字": "一句话"}`，要能读出来。** 直接换成结构体的话，
/// 已经写过备注的人一升级全没了 —— 而且不报错，只是备注空了。
pub fn load_notes() -> std::collections::BTreeMap<String, LoraNote> {
    let raw: std::collections::BTreeMap<String, serde_json::Value> = notes_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or_default();
    raw.into_iter()
        .map(|(k, v)| {
            let n = match &v {
                serde_json::Value::String(s) => LoraNote { note: s.clone(), ..Default::default() },
                _ => serde_json::from_value(v).unwrap_or_default(),
            };
            (k, n)
        })
        .collect()
}

fn save_notes(all: &std::collections::BTreeMap<String, LoraNote>) -> crate::error::Result<()> {
    let path = notes_path().ok_or_else(|| crate::error::Error::config("找不到 loras 目录"))?;
    std::fs::write(&path, serde_json::to_string_pretty(all).unwrap_or_default())
        .map_err(|e| crate::error::Error::runtime(format!("写备注失败：{e}")))
}

/// 记一条 LoRA 备注。空字符串 = 删掉。
#[tauri::command]
pub fn studio_set_lora_note(name: String, note: String) -> crate::error::Result<()> {
    let mut all = load_notes();
    let e = all.entry(name.clone()).or_default();
    e.note = note.trim().to_string();
    // **只在两样都空的时候才删这一条** —— 只看备注的话，
    // 清掉备注会把作者标的激活词一起带走，而且不报错。
    // 分类、出处也算 —— 清掉备注不能把它们一起带走
    if e.note.is_empty() && e.triggers.is_empty() && e.category.is_empty() && e.source_url.is_empty() {
        all.remove(&name);
    }
    save_notes(&all)
}

/// 从 `ss_tag_frequency` 里挑激活词。
///
/// # 为什么是启发式，以及为什么不能静默注入
///
/// 2026-09-04 把本机 13 个 LoRA 的元数据全看了一遍：**没有一个**有
/// `trigger_words` / `activation_text` 字段。有线索的只有 kohya 训练时写的
/// `ss_tag_frequency`，而它混了两类东西：
///
/// - **稀有词触发**：训练时造出来的 token，出现次数极少。
///   实例：`j1v_w0m` ×1（Straight_hair_zimgt）、`sh0r7y_asian` ×1（shorty_asian_zimgt）
/// - **概念词**：训练集的普通标签，出现上百次。
///   实例：`back view` ×85、`asian girls` ×104、`misc` ×8208
///
/// 判据：**出现次数少 + 长得像造出来的 token**（含数字或下划线、无空格、不长）。
/// 这个判据会错 —— 所以调用方必须把结果显示出来并允许去掉，
/// **绝不能悄悄改用户的提示词**。那会变成我们自己造的静默失败：
/// 出图不对，而人根本不知道提示词被动过。
pub fn trigger_candidates(tag_frequency: &serde_json::Value) -> Vec<String> {
    let mut totals: std::collections::BTreeMap<String, u64> = Default::default();
    // 结构是 {数据集名: {标签: 次数}}
    if let Some(sets) = tag_frequency.as_object() {
        for sub in sets.values() {
            if let Some(tags) = sub.as_object() {
                for (tag, n) in tags {
                    *totals.entry(tag.trim().to_string()).or_default() +=
                        n.as_u64().unwrap_or(0);
                }
            }
        }
    }
    let rare_token = |t: &str| -> bool {
        !t.contains(' ')
            && t.len() <= 24
            && t.len() >= 3
            // 造出来的 token 基本都掺数字，或者是带下划线的自造词
            && (t.chars().any(|c| c.is_ascii_digit()) || t.contains('_'))
    };
    let mut out: Vec<String> = totals
        .iter()
        .filter(|(tag, &n)| n <= 3 && rare_token(tag))
        .map(|(tag, _)| tag.clone())
        .collect();
    out.sort();
    out.truncate(3);
    out
}

/// 架构 → 能用它的引擎。
///
/// **挂错架构不会报错。** LoRA 挂到不认识它的引擎上是「0 bypass adapters,
/// 0 inj」—— 图照出、任务照样成功，只是那个风格一点没生效。所以这层过滤
/// 必须在 UI 上就做掉，别让人有机会选错。
/// 一个 LoRA 该挂到哪条链路上。
///
/// **做成枚举而不是字符串，是为了让编译器盯着「加了架构忘了加映射」这件事。**
///
/// 2026-09-08 真出过：`LoraArch` 加了 `Krea2` 变体，`detect_arch` 那边编译器
/// 拦下来了（match 不全），但 `engines_for_arch` 是 `match arch: &str` 且带
/// `_ => sdxl` 兜底 —— **兜底把编译器的嘴堵住了**，于是刚下的 Krea 2 LoRA
/// 被算成只能挂 SDXL，选 Krea 2 时界面上直接不显示，而且不报任何错。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoraTarget {
    Image(crate::agent_runtime::image_engine::LoraArch),
    /// 视频那两种 `image_engine::LoraArch` 不覆盖 —— 它不管视频。
    Ltx,
    H3,
    /// Qwen-Image-Edit。**认得出，但没有引擎能挂它**（`image_ids` 里没有 qwen）。
    ///
    /// 2026-09-17 踩到：它的键名是 `transformer_blocks.N.attn.add_k_proj.*`，
    /// LTX 那条要 `.attn1.`、H3 那条要 `.attn.out_proj.`，都不中 ——
    /// 于是落到「认不出一律当 SDXL」的兜底，**在两个 Pony 和 SDXL 的 LoRA
    /// 列表里都冒了出来**，挂上去静默失效。和 FLUX.2 同一种处理：
    /// 认出来、给空引擎列表，界面说「没有适配这个引擎的」至少是句实话。
    Qwen,
    // 没有 `Unknown` 这一档：读不出键名和文件时，`lora_arch_in` 自己就
    // 回落到 `LoraArch::Sdxl` 了（保守方向和这里一致）。再留一个到不了的
    // 变体，只会让人以为还有一条没覆盖的分支。
}

impl LoraTarget {
    /// 序列化给界面看的名字。
    fn id(self) -> &'static str {
        use crate::agent_runtime::image_engine::LoraArch;
        match self {
            Self::Image(LoraArch::ZImage) => "zimage",
            Self::Image(LoraArch::Sdxl) => "sdxl",
            Self::Image(LoraArch::Krea2) => "krea2",
            Self::Image(LoraArch::Flux2) => "flux2",
            Self::Ltx => "ltx",
            Self::H3 => "h3",
            Self::Qwen => "qwen",
        }
    }

    /// 哪些引擎能用它。**空 = 一个都不能挂**（认出来了但没有引擎支持）。
    fn engines(self) -> Vec<String> {
        use crate::agent_runtime::image_engine::LoraArch;
        match self {
            Self::Image(LoraArch::ZImage) => vec!["zimage".into()],
            // Pony 两个底模都是 SDXL 架构，Pony 专用 LoRA 张量键名和 SDXL 的一样，
            // 分不出来 —— 都列上，引擎说明里写着「普通 SDXL LoRA 效果打折」。
            Self::Image(LoraArch::Sdxl) => vec!["sdxl".into()],
            Self::Image(LoraArch::Krea2) => vec!["krea2".into()],
            // FLUX.2 的引擎已于 2026-09-08 换成 Krea 2 —— 认得出但没地方挂。
            // **给空列表而不是回落 SDXL**：挂到 SDXL 上是静默失效，
            // 而界面上「没有适配这个引擎的」至少是句实话。
            Self::Image(LoraArch::Flux2) => vec![],
            Self::Ltx => vec!["ltx".into()],
            Self::H3 => vec!["h3".into()],
            // 同 FLUX.2：认得出但没地方挂，**给空列表而不是回落 SDXL**。
            Self::Qwen => vec![],
        }
    }
}

/// `…blocks.<数字>.mlp.fc1|fc2…` —— H3 的 MLP 键。`blocks` 必须是完整的一段。
fn is_h3_mlp_key(k: &str) -> bool {
    let seg: Vec<&str> = k.split('.').collect();
    seg.windows(4).any(|w| {
        w[0] == "blocks" && w[1].parse::<u32>().is_ok() && w[2] == "mlp" && matches!(w[3], "fc1" | "fc2")
    })
}

/// 从 safetensors 的张量键名认架构。视频那两种 `image_engine` 没覆盖，
/// 这里按注释里记的指纹补上。
fn detect_arch(dir: &std::path::Path, name: &str) -> LoraTarget {
    let path = dir.join(name);
    if let Some(keys) = read_safetensors_keys(&path) {
        // 视频那两种先判 —— 它们的键名更具体，别被通用规则先吃掉
        if keys.iter().any(|k| k.contains("transformer_blocks.") && k.contains(".attn1.")) {
            return LoraTarget::Ltx;
        }
        if keys.iter().any(|k| k.contains("blocks.") && k.contains(".attn.out_proj.")) {
            return LoraTarget::H3;
        }
        // 只训了 MLP 的 H3 LoRA：`diffusion_model.blocks.N.mlp.fc1 / fc2`，一个 attn 键都没有。
        // 2026-09-25 踩到：HMNSFW V2.5、HMSquirt 都是这种，只认 `attn.out_proj` 的话
        // 会落到兜底当 SDXL —— 出现在出图的 LoRA 列表里、H3 下面却没有，挂上去静默失效。
        // 按**整段**比 `blocks`：diffusers 的 `down_blocks.N` 和 SDXL 文本编码器的
        // `layers.N.mlp.fc1` 都不能中。
        if keys.iter().any(|k| is_h3_mlp_key(k)) {
            return LoraTarget::H3;
        }
        // Qwen-Image-Edit：`transformer_blocks.N.attn.add_k_proj / add_q_proj`。
        // `add_*_proj` 是 MMDiT 那种联合注意力的额外投影，SDXL / Z-Image /
        // Krea 2 都没有，所以够具体，不会误吃别人。
        // **必须排在下面那行兜底之前** —— 兜底会把它当成 SDXL。
        if keys
            .iter()
            .any(|k| k.contains("transformer_blocks.") && (k.contains(".add_k_proj") || k.contains(".add_q_proj")))
        {
            return LoraTarget::Qwen;
        }
    }
    LoraTarget::Image(crate::agent_runtime::image_engine::lora_arch_in(dir, name))
}

/// 读 safetensors 头里的 `__metadata__`。
fn read_safetensors_metadata(
    path: &std::path::Path,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut len = [0u8; 8];
    f.read_exact(&mut len).ok()?;
    let n = u64::from_le_bytes(len) as usize;
    if n == 0 || n > 32 * 1024 * 1024 {
        return None;
    }
    let mut buf = vec![0u8; n];
    f.read_exact(&mut buf).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&buf).ok()?;
    v.get("__metadata__")?.as_object().cloned()
}

/// 读 safetensors 头里的张量名。**只读头，不加载张量。**
fn read_safetensors_keys(path: &std::path::Path) -> Option<Vec<String>> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut len = [0u8; 8];
    f.read_exact(&mut len).ok()?;
    let n = u64::from_le_bytes(len) as usize;
    if n == 0 || n > 32 * 1024 * 1024 {
        return None;
    }
    let mut buf = vec![0u8; n];
    f.read_exact(&mut buf).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&buf).ok()?;
    Some(v.as_object()?.keys().cloned().collect())
}

/// 列出磁盘上的 LoRA 并标注架构。
#[tauri::command]
pub async fn studio_loras() -> Vec<LoraOption> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap_or_default();
    let names = crate::agent_runtime::lora::available_loras(&client).await;
    let notes = load_notes();
    let dir = crate::cinema_sheet::loras_dir().unwrap_or_default();
    names
        .into_iter()
        .map(|name| {
            let target = detect_arch(&dir, &name);
            let label = name
                .rsplit('/')
                .next()
                .unwrap_or(&name)
                .trim_end_matches(".safetensors")
                .to_string();
            let tf = read_safetensors_metadata(&dir.join(&name))
                .and_then(|md| md.get("ss_tag_frequency").cloned())
                .and_then(|v| serde_json::from_str::<serde_json::Value>(v.as_str()?).ok());
            let triggers = tf.as_ref().map(trigger_candidates).unwrap_or_default();
            let concepts = tf.as_ref().map(concept_words).unwrap_or_default();
            let saved = notes.get(&name).cloned().unwrap_or_default();
            let note = if saved.note.is_empty() { None } else { Some(saved.note) };
            let thumb = lora_thumb(&name);
            LoraOption {
                engines: target.engines(),
                confirmed_triggers: saved.triggers,
                nsfw: saved.nsfw,
                category: saved.category,
                source_url: saved.source_url,
                thumb,
                arch: target.id().to_string(),
                label, name, triggers, concepts, note,
            }
        })
        .collect()
}

/// 一条 Prompt 模板。
#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq)]
pub struct PromptTemplate {
    pub id: String,
    pub title: String,
    pub body: String,
    /// "image" | "video" | "general" —— 决定在哪个分类页出现
    pub kind: String,
}

/// 一条历史记录。
#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq)]
pub struct PromptHistoryItem {
    pub prompt: String,
    pub mode: String,
    pub engine: String,
    /// 毫秒时间戳
    pub at: u64,
}

fn studio_dir() -> Option<std::path::PathBuf> {
    let d = dirs::config_dir()?.join("com.anima.desktop").join("studio");
    std::fs::create_dir_all(&d).ok()?;
    Some(d)
}

fn read_json<T: serde::de::DeserializeOwned + Default>(name: &str) -> T {
    studio_dir()
        .and_then(|d| std::fs::read_to_string(d.join(name)).ok())
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or_default()
}

fn write_json<T: Serialize>(name: &str, v: &T) -> crate::error::Result<()> {
    let d = studio_dir().ok_or_else(|| crate::error::Error::config("找不到配置目录"))?;
    std::fs::write(d.join(name), serde_json::to_string_pretty(v).unwrap_or_default())
        .map_err(|e| crate::error::Error::runtime(format!("写 {name} 失败：{e}")))
}

/// 资产库里的一个条目。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Asset {
    pub path: String,
    pub name: String,
    /// "image" | "video"
    pub kind: String,
    pub bytes: u64,
    /// 修改时间（毫秒）
    pub at: u64,
}

/// 生成目录。所有产物都落在这儿。
fn generated_dir(workspace_path: Option<&str>) -> Option<std::path::PathBuf> {
    let root = workspace_path
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| dirs::config_dir().map(|d| d.join("com.anima.desktop")))?;
    Some(root.join(".anima").join("generated"))
}

/// 列出生成过的东西。
///
/// **这是「生成的内容没地方查看」的解法。** 之前出的图和片子落在工作区的
/// `.anima/generated/` 下，界面上没有任何入口 —— 只能去 Finder 翻。
/// 按修改时间倒序，最新的在最前。
#[tauri::command]
pub fn studio_assets(workspace_path: Option<String>) -> Vec<Asset> {
    let Some(dir) = generated_dir(workspace_path.as_deref()) else {
        return vec![];
    };
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return vec![];
    };
    let mut out: Vec<Asset> = rd
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let kind = match p.extension()?.to_str()?.to_ascii_lowercase().as_str() {
                "png" | "jpg" | "jpeg" | "webp" => "image",
                "mp4" | "mov" | "webm" => "video",
                _ => return None,
            };
            let md = e.metadata().ok()?;
            let at = md.modified().ok()?
                .duration_since(std::time::UNIX_EPOCH).ok()?
                .as_millis() as u64;
            Some(Asset {
                name: p.file_name()?.to_str()?.to_string(),
                path: p.to_str()?.to_string(),
                kind: kind.to_string(),
                bytes: md.len(),
                at,
            })
        })
        .collect();
    out.sort_by(|a, b| b.at.cmp(&a.at));
    out
}

/// 删掉一个产物。
///
/// **只允许删生成目录里的东西。** 判据是「这个路径出现在 `studio_assets`
/// 的结果里」—— 比自己拼路径比较可靠，路径穿越（`../../`）也一并挡住了。
/// 这是唯一一个会不可逆动文件的命令。
#[tauri::command]
pub fn studio_delete_asset(
    workspace_path: Option<String>,
    path: String,
) -> crate::error::Result<()> {
    if !studio_assets(workspace_path).iter().any(|a| a.path == path) {
        return Err(crate::error::Error::config("只能删除生成目录里的文件"));
    }
    std::fs::remove_file(&path)
        .map_err(|e| crate::error::Error::runtime(format!("删除失败：{e}")))
}

/// 结果画廊里的一条。
///
/// **文件落盘 ≠ 结果保住了。** 出的图一直在 `.anima/generated/` 下，
/// 但「这张图是用什么提示词、什么模式、什么引擎、什么 seed 出的」
/// 只活在前端内存里 —— 一重启就只剩一堆文件名，等于白出。
///
/// 字段跟前端 `StudioResult` 一一对应。多余的字段用 `#[serde(default)]`
/// 兜住，这样老的 history.json 加了字段之后还读得出来 ——
/// 读不出来的话 `read_json` 会静默回一个空 Vec，用户看到的是「历史又没了」。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct HistoryEntry {
    pub id: String,
    pub mode: String,
    #[serde(default, rename = "engineId")]
    pub engine_id: String,
    #[serde(default)]
    pub prompt: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, rename = "triggersAdded", skip_serializing_if = "Vec::is_empty")]
    pub triggers_added: Vec<String>,
    #[serde(default, rename = "startedAt")]
    pub started_at: u64,
    #[serde(default, rename = "endedAt", skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<u64>,
}

/// 历史最多留这么多条。再多就只是拖慢启动 —— 真要翻旧的去资产库。
pub const HISTORY_MAX: usize = 400;

/// 读回来的历史要**对着磁盘核一遍**。
///
/// 三件事，每一件不做都会让画廊显示假象：
///
/// 1. `pending` / `running` 直接丢掉。应用重启了，那次生成没人在等它 ——
///    留着就是一张永远转圈的卡片，而且没有任何东西会把它变成别的状态。
/// 2. `done` 但**文件已经不在了**的丢掉（用户在访达里删了 / 换了工作区）。
///    留着是一张碎图，而 `MediaThumb` 只会说「读不出来」，
///    看着像我们的 bug。
/// 3. 超出上限的截掉，**从旧的那头截**。
///
/// 拆成纯函数是因为这三条都是「不做也不报错」的那种。
pub fn reconcile_history(saved: Vec<HistoryEntry>, exists: impl Fn(&str) -> bool) -> Vec<HistoryEntry> {
    let mut out: Vec<HistoryEntry> = saved
        .into_iter()
        .filter(|e| match e.status.as_str() {
            "pending" | "running" => false,
            "done" => e.path.as_deref().is_some_and(&exists),
            _ => true,
        })
        .collect();
    out.truncate(HISTORY_MAX);
    out
}

/// 结果历史存在哪。
///
/// **和产物同一个根**（`<工作区>/.anima/`，`generated/` 的旁边）。
///
/// 这不是随手放的。历史里每一条都指向 `generated/` 下的一个文件，而
/// `reconcile_history` 会把「文件不在了」的条目丢掉 —— 要是历史存在
/// 应用配置目录里、产物跟着工作区走，那么**一换工作区，整份历史就会被
/// 判定为全都不存在而清空**，而且是静默的、不可逆的。
///
/// LumenX 的 `PlaygroundStorage.HISTORY_PATH` 是相对路径
/// `output/playground_history.json` —— 跟它的产物目录 `output/` 是同一个，
/// 同一个道理。
fn results_path(workspace_path: Option<&str>) -> Option<std::path::PathBuf> {
    Some(generated_dir(workspace_path)?.with_file_name("studio-results.json"))
}

/// 读回上次的结果画廊。
///
/// 名字叫 `results` 不叫 `history` —— 这个模块里 `history` 已经是
/// **提示词历史**了，再占一次这个词，两边迟早有人调错。
#[tauri::command]
pub fn studio_results(workspace_path: Option<String>) -> Vec<HistoryEntry> {
    let saved: Vec<HistoryEntry> = results_path(workspace_path.as_deref())
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or_default();
    reconcile_history(saved, |p| std::path::Path::new(p).is_file())
}

/// 存结果画廊。**整份覆盖** —— 前端那份是唯一真相，增量合并只会两边打架。
#[tauri::command]
pub fn studio_save_results(
    workspace_path: Option<String>,
    items: Vec<HistoryEntry>,
) -> crate::error::Result<()> {
    let p = results_path(workspace_path.as_deref())
        .ok_or_else(|| crate::error::Error::config("找不到工作区目录"))?;
    let mut items = items;
    items.truncate(HISTORY_MAX);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)
            .map_err(|e| crate::error::Error::runtime(format!("建目录失败：{e}")))?;
    }
    std::fs::write(&p, serde_json::to_string_pretty(&items).unwrap_or_default())
        .map_err(|e| crate::error::Error::runtime(format!("写结果历史失败：{e}")))
}

#[tauri::command]
pub fn studio_templates() -> Vec<PromptTemplate> {
    read_json("templates.json")
}

/// 存一条模板。`id` 已存在就覆盖，不存在就新增。
#[tauri::command]
pub fn studio_save_template(tpl: PromptTemplate) -> crate::error::Result<Vec<PromptTemplate>> {
    let mut all: Vec<PromptTemplate> = read_json("templates.json");
    match all.iter_mut().find(|t| t.id == tpl.id) {
        Some(existing) => *existing = tpl,
        None => all.push(tpl),
    }
    write_json("templates.json", &all)?;
    Ok(all)
}

#[tauri::command]
pub fn studio_delete_template(id: String) -> crate::error::Result<Vec<PromptTemplate>> {
    let mut all: Vec<PromptTemplate> = read_json("templates.json");
    all.retain(|t| t.id != id);
    write_json("templates.json", &all)?;
    Ok(all)
}

/// 历史最多留多少条。
///
/// **不是越多越好** —— 这个列表要能一眼扫完，而且每条都是整段提示词。
/// 留太多会让搜索变成唯一的入口，那还不如不留。
pub const HISTORY_LIMIT: usize = 200;

#[tauri::command]
pub fn studio_history() -> Vec<PromptHistoryItem> {
    read_json("history.json")
}

/// 记一条历史。**同样的提示词只留最新那条**，否则连点五次生成
/// 就把列表刷满了五条一模一样的。
#[tauri::command]
pub fn studio_add_history(
    item: PromptHistoryItem,
) -> crate::error::Result<Vec<PromptHistoryItem>> {
    let mut all: Vec<PromptHistoryItem> = read_json("history.json");
    all.retain(|h| h.prompt.trim() != item.prompt.trim());
    all.insert(0, item);
    all.truncate(HISTORY_LIMIT);
    write_json("history.json", &all)?;
    Ok(all)
}

#[tauri::command]
pub fn studio_capabilities() -> CapabilityCatalog {
    catalog()
}

/// 跑一次生成。
///
/// **直接调 agent 工具本身**，不重新实现建图 —— Studio 和对话必须走同一条
/// 路径，否则会出现「在对话里能跑、在界面上不行」这种最难查的分歧。
/// 选引擎、挑模型、回落、LoRA、进度、错误措辞全部沿用工具里那套（有测试的）。
#[tauri::command]
pub async fn studio_generate(
    state: tauri::State<'_, crate::AppState>,
    args: GenerateArgs,
) -> crate::error::Result<GenerateOutput> {
    generate_with_gateway(&state.config_dir, Some(&state.gateway), args).await
}

/// 画布节点勾了「H3 官方格式」就按出片方式整理提示词（只动格式，见 [`crate::h3_spec::canvas_official`]）。别的引擎、没勾的不动。
fn apply_h3_official_format(args: &mut GenerateArgs) {
    if args.engine == "h3" && args.h3_official_format == Some(true) {
        args.prompt = crate::h3_spec::canvas_official(&args.prompt, &args.mode, args.duration.unwrap_or(5.0) as f32);
    }
}

/// [`studio_generate`] 的本体，不依赖界面状态。出封面、资产批量这些不会超过 15 秒的调用方用它。
pub async fn generate_with(config_dir: &std::path::Path, args: GenerateArgs) -> crate::error::Result<GenerateOutput> {
    generate_with_gateway(config_dir, None, args).await
}

/// 同上，带上后台模型 —— **H3 超过 15 秒走超长视频**（[`crate::long_video`]），提示词没写时间时要它来拆段。
/// 画布的后台任务和创作台走这一份。
pub async fn generate_with_gateway(config_dir: &std::path::Path, gateway: Option<&crate::Gateway>, args: GenerateArgs) -> crate::error::Result<GenerateOutput> {
    use crate::agent_runtime::{ToolCallRequest, ToolHandler, ToolStatus};

    // `@` 引用：先对到资产上，再把标签换成「名字（描述）」。
    //
    // **标签不能原样进引擎** —— 模型会把方括号和「角色」两个字
    // 当画面内容画进去。出图了、不报错、画面里多一行怪字。
    let resolved = crate::assets::resolve_refs(config_dir, &args.prompt);
    let mut args = args;
    if !resolved.is_empty() {
        args.prompt = crate::assets::expand_ref_tags(&args.prompt, &resolved);
        args.reference_images = crate::assets::reference_images(&resolved);
    }
    // 超长视频：同一个「生成」，后台分段串成一整条（官方格式按段各自规整，在里面做）
    if crate::long_video::wants(&args) {
        return crate::long_video::render(config_dir, gateway, &args).await;
    }
    apply_h3_official_format(&mut args);
    let args = args;

    let (tool_name, tool_args) =
        to_tool_call(&args).map_err(crate::error::Error::config)?;

    // 工作区：前端给了就用它，没给退到配置目录。
    // 出的图和片子落在工作区的 generated/ 下，和对话里那条完全一致。
    let workspace = args
        .workspace_path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| config_dir.to_path_buf());
    let ctx = crate::agent_runtime::RuntimeContext::new(
        workspace.clone(),
        crate::agent_runtime::RuntimePolicy::workspace_write(&workspace),
    );
    let request = ToolCallRequest::new(tool_name, tool_args);

    let outcome = match tool_name {
        "generate_image" => {
            crate::agent_runtime::image_tool::GenerateImageTool
                .execute(&ctx, request)
                .await
        }
        "generate_video" => {
            crate::agent_runtime::video_tool::GenerateVideoTool
                .execute(&ctx, request)
                .await
        }
        "animate_image" => {
            crate::agent_runtime::video_tool::AnimateImageTool
                .execute(&ctx, request)
                .await
        }
        other => {
            return Err(crate::error::Error::config(format!(
                "没有叫「{other}」的工具"
            )))
        }
    };

    let ok = outcome.status == ToolStatus::Ok;
    let message = outcome
        .failure
        .as_ref()
        .map(|f| f.message.clone())
        .unwrap_or(outcome.content);
    let mut out = GenerateOutput {
        // artifacts 里第一个就是落盘的文件
        path: outcome.artifacts.first().cloned(),
        paths: outcome.artifacts.clone(),
        // 失败时整段 message 已经显示出来了，不用再重复摘一遍
        notes: if ok { notices(&message) } else { Vec::new() },
        message,
        ok,
    };
    // 七分区参考表：表情区三张脸补成开心 / 生气 / 惊讶（见 `sheet_expressions`）。补不成就用原图，说一声
    if ok && args.mode == "entity_sheet" {
        if let Some(sheet) = out.path.clone() {
            let locks = args.identity_locks.clone();
            let filled = crate::sheet_expressions::fill(
                config_dir,
                std::path::Path::new(&sheet),
                args.entity_name.as_deref().unwrap_or(""),
                args.style_line.as_deref(),
                &locks,
                args.workspace_path.clone(),
            )
            .await;
            match filled {
                Ok(Some(f)) if f.path.to_string_lossy() != sheet => {
                    let p = f.path.to_string_lossy().to_string();
                    out.notes.push("表情区补成了开心 / 生气 / 惊讶".into());
                    out.notes.extend(f.notes);
                    out.path = Some(p.clone());
                    out.paths = vec![p];
                }
                Ok(Some(f)) => out.notes.extend(f.notes),
                Ok(None) => out.notes.push("没认出表情区（三张脸的位置），表情没有补".into()),
                Err(e) => out.notes.push(format!("补表情失败，用的是原图：{e}")),
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine(c: &CapabilityCatalog, id: &str) -> EngineSpec {
        c.engines.iter().find(|e| e.id == id).expect(id).clone()
    }

    // ── 结果画廊的持久化 ──────────────────────────────────────────────

    fn entry(id: &str, status: &str, path: Option<&str>) -> HistoryEntry {
        HistoryEntry {
            id: id.into(),
            mode: "t2i".into(),
            engine_id: "zimage".into(),
            prompt: "一只猫".into(),
            status: status.into(),
            path: path.map(String::from),
            ..Default::default()
        }
    }

    // ── 历史存在哪：必须和产物同一个根 ──────────────────────────────

    /// **这是「换个工作区历史就全没了」的防线。**
    ///
    /// 历史里每一条都指向 `generated/` 下的一个文件，而 `reconcile_history`
    /// 会把「文件不在了」的丢掉。历史存在应用配置目录、产物跟着工作区走的话，
    /// 一换工作区就等于整份历史全被判定为不存在 —— 静默清空，不可逆。
    /// **资产出图明说「别挂全局 LoRA」时，参数里要带上这一位。**
    ///
    /// 2026-09-12 实拍：一批 11 个资产全废。设置里配着
    /// `Pintrest_ZiT`(0.6) + `Straight_hair_zimgt`(0.8)，而资产出图传的
    /// `loras` 是空数组 —— 空数组等于「没传」，于是两个常驻 LoRA 被静默
    /// 挂上：道具「键盘」那张图里三格是人像和头发。
    /// **全景那条路的参数要锁死。**
    ///
    /// 等距圆柱必须 2:1（比例不对切出来的四个角度全错，而图看着正常）；
    /// 引擎锁 Z-Image（我们装的是它的 Turbo 版 LoRA，别的底模挂不上
    /// 且不报错）；触发词拼在最前（排在一长段场景描述后面会被稀释）。
    /// **r2v 的参考图上限也是 9，不是 4。**
    ///
    /// 这里原来写死 `take(4)` —— 和 `cinema::shot_reference_images` 那个
    /// `truncate(4)` 是同一个错数字，第 5 张往后静默丢掉。
    #[test]
    fn r2v最多送九张参考图() {
        let mut a = ga("r2v", "h3", &[]);
        a.images = (0..12).map(|i| format!("/tmp/ref{i}.png")).collect();
        let (tool, args) = to_tool_call(&a).unwrap();
        assert_eq!(tool, "animate_image", "r2v 走的是 animate_image");
        assert_eq!(args["engine"], "h3");
        assert_eq!(args["mode"], "reference");
        assert_eq!(
            args["reference_images"].as_array().unwrap().len(),
            crate::agent_runtime::video_tool::MAX_REFERENCE_IMAGES,
            "十二张参考图该送满 9 张"
        );
    }

    /// 画布加速档（2026-10-08）：只发给出片模式；没选就不发（照旧按尺寸自动）
    #[test]
    fn 加速档只发给出片() {
        let mut a = ga("i2v", "h3", &["/tmp/a.png"]);
        assert!(to_tool_call(&a).unwrap().1.get("h3_quality").is_none());
        a.h3_quality = Some("lightx2v".into());
        assert_eq!(to_tool_call(&a).unwrap().1["h3_quality"], "lightx2v");
        let mut t = ga("t2i", "zimage", &[]);
        t.h3_quality = Some("staged".into());
        assert!(to_tool_call(&t).unwrap().1.get("h3_quality").is_none(), "出图不发");
    }

    /// 画布音频素材（2026-10-08）：r2v 带上参考音频；没有就不发这个键
    #[test]
    fn r2v带参考音频() {
        let mut a = ga("r2v", "h3", &["/tmp/a.png"]);
        let (_, args) = to_tool_call(&a).unwrap();
        assert!(args.get("reference_audios").is_none());
        a.audios = vec!["/tmp/song.mp3".into()];
        let (tool, args) = to_tool_call(&a).unwrap();
        assert_eq!(tool, "animate_image");
        assert_eq!(args["reference_audios"], serde_json::json!(["/tmp/song.mp3"]));
    }

    #[test]
    fn 全景模式锁死画幅引擎和触发词() {
        let mut a = ga("pano_views", "sdxl", &[]);
        a.prompt = "盛唐金銮殿内景，高约15米".into();
        let (tool, args) = to_tool_call(&a).unwrap();
        assert_eq!(tool, "generate_image");
        assert_eq!(args["width"], 1536);
        assert_eq!(args["height"], 768);
        assert_eq!(
            args["engine"],
            crate::agent_runtime::panorama::PANO_ENGINE,
            "引擎该被锁成 Z-Image，用户选的 sdxl 不作数"
        );
        assert_eq!(args["layout"], "pano_views");
        let p = args["prompt"].as_str().unwrap();
        assert!(p.starts_with("360 view"), "触发词要在最前：{p}");
        assert!(p.contains("盛唐金銮殿"), "场景描述不能丢：{p}");
    }

    /// 别的模式不该被这条影响。
    #[test]
    fn 别的模式不带全景版式() {
        for m in ["t2i", "scene_sheet"] {
            let (_, args) = to_tool_call(&ga(m, "zimage", &[])).unwrap();
            assert_ne!(args.get("layout").and_then(|v| v.as_str()), Some("pano_views"));
        }
    }

    #[test]
    fn 明说不挂全局lora时参数里要带上() {
        let mut a = ga("t2i", "zimage", &[]);
        a.auto_loras = Some(false);
        let (_, args) = to_tool_call(&a).unwrap();
        assert_eq!(args.get("auto_loras").and_then(|v| v.as_bool()), Some(false));
    }

    /// **不发这一位时行为一个字不变。** 日常出图那条路还指着常驻 LoRA。
    #[test]
    fn 没明说时不带这个参数() {
        for v in [None, Some(true)] {
            let mut a = ga("t2i", "zimage", &[]);
            a.auto_loras = v;
            let (_, args) = to_tool_call(&a).unwrap();
            assert!(args.get("auto_loras").is_none(), "{v:?} 不该发出这个参数");
        }
    }

    #[test]
    fn 历史和产物在同一个目录下() {
        let ws = "/tmp/我的工作区";
        let gen = generated_dir(Some(ws)).unwrap();
        let res = results_path(Some(ws)).unwrap();
        assert_eq!(
            res.parent(),
            gen.parent(),
            "历史({res:?})和产物({gen:?})不在同一个根下 —— 换工作区会把历史清空"
        );
    }

    #[test]
    fn 两个工作区各有各的历史() {
        let a = results_path(Some("/tmp/甲")).unwrap();
        let b = results_path(Some("/tmp/乙")).unwrap();
        assert_ne!(a, b, "两个工作区共用了一份历史");
        assert!(a.starts_with("/tmp/甲"), "{a:?} 没落在甲下面");
    }

    #[test]
    fn 同一个工作区每次算出同一个路径() {
        assert_eq!(results_path(Some("/tmp/甲")), results_path(Some("/tmp/甲")));
    }

    /// 没给工作区时退回应用配置目录 —— 但**仍然要和产物同根**。
    #[test]
    fn 没给工作区时也和产物同根() {
        let gen = generated_dir(None).unwrap();
        let res = results_path(None).unwrap();
        assert_eq!(res.parent(), gen.parent());
    }

    /// 应用重启了，那次生成没人在等它。留着就是一张**永远转圈**的卡片 ——
    /// 没有任何东西会把它变成别的状态。
    #[test]
    fn 重启后不留正在跑的卡片() {
        let saved = vec![
            entry("a", "done", Some("/存在")),
            entry("b", "running", None),
            entry("c", "pending", None),
        ];
        let out = reconcile_history(saved, |_| true);
        assert_eq!(out.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), vec!["a"]);
    }

    /// 用户在访达里删了图 / 换了工作区。留着是一张碎图，
    /// 而 `MediaThumb` 只会说「读不出来」，看着像我们的 bug。
    #[test]
    fn 文件没了的条目要丢掉() {
        let saved = vec![entry("有", "done", Some("/在")), entry("没", "done", Some("/不在"))];
        let out = reconcile_history(saved, |p| p == "/在");
        assert_eq!(out.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), vec!["有"]);
    }

    /// 失败的那些**留着** —— 报错信息是这条链路上最有用的东西
    /// （ComfyUI 没起、显存不够、模型缺文件），而且它们本来就没有文件。
    #[test]
    fn 失败的条目留着_不因为没文件被误删() {
        let mut f = entry("f", "failed", None);
        f.error = Some("ComfyUI 没起".into());
        let out = reconcile_history(vec![f], |_| false);
        assert_eq!(out.len(), 1, "失败的被当成「文件没了」删掉了");
        assert_eq!(out[0].error.as_deref(), Some("ComfyUI 没起"));
    }

    /// 从**旧的**那头截。前端是新的在前，所以截尾巴。
    /// 截错头的话，每次重启都只剩最老的 400 条，新出的图反而没了。
    #[test]
    fn 超出上限从旧的那头截() {
        let saved: Vec<HistoryEntry> = (0..HISTORY_MAX + 5)
            .map(|i| entry(&format!("e{i}"), "done", Some("/在")))
            .collect();
        let out = reconcile_history(saved, |_| true);
        assert_eq!(out.len(), HISTORY_MAX);
        assert_eq!(out[0].id, "e0", "把最新的截掉了");
    }

    /// 加字段之后老的 history.json 还要读得出来。
    /// 读不出来时 `read_json` 会**静默回空 Vec** —— 用户看到的是「历史又没了」，
    /// 而且没有任何报错能指向真正的原因。
    #[test]
    fn 缺字段的老记录仍然读得出来() {
        let old = r#"[{"id":"a","mode":"t2i","status":"done","path":"/x"}]"#;
        let parsed: Vec<HistoryEntry> = serde_json::from_str(old).expect("老格式解析失败");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].engine_id, "", "缺的字段该给默认值");
    }

    /// 字段名要和前端 `StudioResult` 对得上，否则存进去读回来全是默认值 ——
    /// 卡片显示得出图，但引擎名、seed、用时全空，而且不报错。
    #[test]
    fn 序列化用的是前端那套字段名() {
        let mut e = entry("a", "done", Some("/x"));
        e.started_at = 1;
        e.ended_at = Some(2);
        e.triggers_added = vec!["sh0r7y".into()];
        let j = serde_json::to_string(&e).unwrap();
        for key in ["engineId", "startedAt", "endedAt", "triggersAdded"] {
            assert!(j.contains(&format!("\"{key}\"")), "缺字段 {key}：{j}");
        }
    }

    /// **负向提示词无效的引擎不能在 UI 上显示那个框。**
    ///
    /// 显示一个不起作用的输入框比不显示更糟 —— 用户会以为自己压住了什么，
    /// 而实际上那段文字被 `ConditioningZeroOut` 清零了。
    #[test]
    fn 负向提示词的开关跟着引擎的真实行为() {
        let c = catalog();
        assert!(!engine(&c, "zimage").negative_prompt, "Z-Image 的负向是无效的");
        assert!(engine(&c, "sdxl").negative_prompt, "SDXL 的负向是有效的");
        // Chroma 原来也在这里，2026-10-01 下架
        assert!(!c.engines.iter().any(|e| e.id == "chroma"), "已下架的 chroma 还在目录里");
        assert!(!engine(&c, "ltx").negative_prompt, "WeeTodd 这条路没有负向入口");
        assert!(!engine(&c, "h3").negative_prompt);

        // 无效时必须给出理由，否则用户只会觉得功能少了
        for id in ["zimage", "ltx", "h3"] {
            assert!(!engine(&c, id).notes.is_empty(), "{id} 关掉了负向却没说为什么");
        }
    }

    /// 视频那两条**不该暴露步数** —— 由预设内部决定，
    /// 暴露出来只会被按 SDXL 的习惯改坏（Z-Image 传 20 步慢 2.5 倍还不更好）。
    #[test]
    fn 只有出图暴露步数且值与建图一致() {
        let c = catalog();
        assert_eq!(engine(&c, "zimage").steps, Some(8));
        assert_eq!(engine(&c, "krea2").steps, Some(8));
        assert_eq!(engine(&c, "sdxl").steps, Some(20));
        assert_eq!(engine(&c, "ltx").steps, None, "视频不该暴露步数");
        assert_eq!(engine(&c, "h3").steps, None);
    }

    /// 模式和引擎必须**互相指得通**：模式点名的引擎要存在，
    /// 且种类要对得上（视频模式不能挂出图引擎）。
    #[test]
    fn 模式与引擎双向自洽() {
        let c = catalog();
        for m in &c.modes {
            assert!(!m.engines.is_empty(), "模式 {} 没有可用引擎", m.id);
            for id in &m.engines {
                let e = c.engines.iter().find(|e| &e.id == id);
                let e = e.unwrap_or_else(|| panic!("模式 {} 指向不存在的引擎 {id}", m.id));
                assert_eq!(e.kind, m.kind, "模式 {} 挂了种类不符的引擎 {id}", m.id);
            }
            // 需要图的模式必须给标签，否则用户不知道该塞哪张图
            if m.images > 0 {
                assert!(m.media_label.is_some(), "模式 {} 要图却没有标签", m.id);
            } else {
                assert!(m.media_label.is_none(), "模式 {} 不要图却有标签", m.id);
            }
        }
        // 默认引擎得真的存在
        assert!(c.engines.iter().any(|e| e.id == c.default_image_engine));
        assert!(c.engines.iter().any(|e| e.id == c.default_video_engine));
    }

    /// 只有 SDXL 能做设定卡和姿势控制 —— 那些权重挂到别的引擎上
    /// **不生效且不报错**（CLAUDE.md 记着的强制回落规则）。
    #[test]
    fn 设定卡与姿势控制只给sdxl() {
        let c = catalog();
        for id in ["sheet", "pose"] {
            let m = c.modes.iter().find(|m| m.id == id).expect(id);
            assert_eq!(m.engines, vec!["sdxl"], "{id} 不该给 SDXL 之外的引擎");
        }
        // 参考图生视频只有 H3 有 ref2va 分区
        let r2v = c.modes.iter().find(|m| m.id == "r2v").unwrap();
        assert_eq!(r2v.engines, vec!["h3"]);
    }

    fn ga(mode: &str, engine: &str, imgs: &[&str]) -> GenerateArgs {
        GenerateArgs {
            h3_official_format: None,
            h3_quality: None,
            audios: vec![],
            mode: mode.into(), engine: engine.into(),
            prompt: "老茶师注水".into(), negative_prompt: "模糊, 多手".into(),
            width: 768, height: 512, seed: Some(7),
            images: imgs.iter().map(|s| s.to_string()).collect(),
            duration: Some(5.0), generate_audio: None, auto_loras: None, denoise: None, ref_boost: None,
            entity_type: None, entity_name: None, identity_locks: vec![], style_line: None,
            workspace_path: None, loras: vec![],
            output_filename: None, reference_images: vec![],
            viewpoints: vec![], refine: None,
        }
    }

    /// 画布 H3 官方格式开关：勾了才整理、只管 H3（2026-10-06）
    #[test]
    fn 画布h3官方格式_勾了才整理_只管h3() {
        let mut a = ga("i2v", "h3", &["/x.png"]);
        apply_h3_official_format(&mut a);
        assert_eq!(a.prompt, "老茶师注水", "没勾不动");
        a.h3_official_format = Some(true);
        apply_h3_official_format(&mut a);
        assert!(a.prompt.starts_with(crate::h3_spec::I2VA_INSTRUCTION) && a.prompt.contains("integrated_multimodal_description: [Shot 1] 老茶师注水"), "{}", a.prompt);
        let mut l = ga("i2v", "ltx", &["/x.png"]);
        l.h3_official_format = Some(true);
        apply_h3_official_format(&mut l);
        assert_eq!(l.prompt, "老茶师注水", "LTX 不认这套格式");
    }

    // ── 场景宫格 ──────────────────────────────────────────────────

    fn 宫格(views: Vec<(&str, &str)>, refine: bool) -> serde_json::Value {
        let mut a = ga("scene_sheet", "zimage", &[]);
        a.viewpoints = views
            .into_iter()
            .map(|(n, d)| ViewpointArg { name: n.into(), description: d.into() })
            .collect();
        a.refine = refine.then_some(true);
        to_tool_call(&a).unwrap().1
    }

    #[test]
    fn 场景宫格走_scene_sheet_版式() {
        let v = 宫格(vec![("入口正打", "从卷帘门内侧朝店堂深处看，尽头是柜台")], false);
        assert_eq!(v["layout"], "scene_sheet");
        // **不锁引擎** —— 宫格不挂版式 LoRA，四个底模都出得来
        assert_eq!(v["engine"], "zimage");
        assert_eq!(v["viewpoints"][0]["name"], "入口正打");
        assert!(v["viewpoints"][0]["description"]
            .as_str()
            .unwrap()
            .contains("尽头是柜台"));
    }

    /// **一个方位都没有就别传这个键。**
    ///
    /// 传空数组进去行为和不传一样（工具的兜底判据是「一条都没解析出来」），
    /// 但日志里会留下一条「视角已下发」的假记录，把排查带偏。
    #[test]
    fn 没填方位时不传空数组() {
        let v = 宫格(vec![], false);
        assert!(v.get("viewpoints").is_none(), "{v}");
        let v2 = 宫格(vec![("  ", "只有空白")], false);
        assert!(v2.get("viewpoints").is_none(), "空白名字要被滤掉：{v2}");
    }

    /// 精修**默认不开**：每格多一次出图。
    #[test]
    fn 精修要显式打开() {
        assert!(宫格(vec![("正打", "")], false).get("refine").is_none());
        assert_eq!(宫格(vec![("正打", "")], true)["refine"], true);
    }

    /// **负向提示词只传给真的认它的引擎。**
    ///
    /// 传给不认的不会报错，只是白传 —— 但日志里会留下一条看起来生效了的
    /// 记录，把后面的排查带偏。
    #[test]
    fn 负向提示词不传给不认它的引擎() {
        let (_, sdxl) = to_tool_call(&ga("t2i", "sdxl", &[])).unwrap();
        assert_eq!(sdxl["negative_prompt"], "模糊, 多手");

        {
            let e = "zimage";
            let (_, v) = to_tool_call(&ga("t2i", e, &[])).unwrap();
            assert!(v.get("negative_prompt").is_none(), "{e} 不认负向却传了");
        }
        for m in ["t2v", "i2v"] {
            let (_, v) = to_tool_call(&ga(m, "ltx", &["/a.png"])).unwrap();
            assert!(v.get("negative_prompt").is_none(), "{m} 走 WeeTodd，没有负向入口");
        }
    }

    /// 八个模式各自映射到对的工具和参数。**映射错了不会报错** ——
    /// 比如 i2v 忘了传 image_path，出来的是一段纯文生视频，
    /// 而那看起来只是"没听话"。
    #[test]
    fn 八个模式映射到对的工具() {
        let cases: Vec<(&str, &str, &str, Vec<&str>)> = vec![
            ("t2i", "zimage", "generate_image", vec![]),
            ("i2i", "zimage", "generate_image", vec!["/a.png"]),
            ("sheet", "zimage", "generate_image", vec![]),
            ("pose", "zimage", "generate_image", vec!["/a.png"]),
            ("t2v", "ltx", "generate_video", vec![]),
            ("i2v", "ltx", "animate_image", vec!["/a.png"]),
            ("fl2v", "ltx", "animate_image", vec!["/a.png", "/z.png"]),
            ("r2v", "h3", "animate_image", vec!["/a.png"]),
        ];
        for (mode, engine, tool, imgs) in cases {
            let (t, v) = to_tool_call(&ga(mode, engine, &imgs)).unwrap();
            assert_eq!(t, tool, "模式 {mode} 走错工具");
            assert_eq!(v["prompt"], "老茶师注水");
            match mode {
                "i2i" | "pose" | "i2v" | "fl2v" | "r2v" => {
                    let key = if mode == "pose" { "pose_image" } else { "image_path" };
                    assert_eq!(v[key], "/a.png", "模式 {mode} 没把图接进去");
                }
                _ => assert!(v.get("image_path").is_none() && v.get("pose_image").is_none()),
            }
        }
        // 首尾帧必须两张都接进去，只接首帧会出一段没有末帧的片子
        let (_, fl) = to_tool_call(&ga("fl2v", "ltx", &["/a.png", "/z.png"])).unwrap();
        assert_eq!(fl["end_image"], "/z.png");
        // 设定卡和姿势强制 SDXL —— 那些权重挂到别的引擎上不生效且不报错
        for m in ["sheet", "pose"] {
            let (_, v) = to_tool_call(&ga(m, "zimage", &["/a.png"])).unwrap();
            assert_eq!(v["engine"], "sdxl", "{m} 没强制回落 SDXL");
        }
        // 参考图生视频只有 H3
        let (_, r) = to_tool_call(&ga("r2v", "ltx", &["/a.png"])).unwrap();
        assert_eq!(r["engine"], "h3");
        assert_eq!(r["mode"], "reference");
    }

    /// 删除只能落在生成目录里。**前端传什么路径都不该能删到别处去** ——
    /// 这是唯一一个会不可逆地动文件的命令。
    #[test]
    fn 删除产物不能越出生成目录() {
        let tmp = std::env::temp_dir().join(format!("anima_assets_{}", std::process::id()));
        let gen = tmp.join(".anima").join("generated");
        std::fs::create_dir_all(&gen).unwrap();
        let inside = gen.join("a.png");
        std::fs::write(&inside, b"x").unwrap();
        let outside = tmp.join("别动我.txt");
        std::fs::write(&outside, b"x").unwrap();

        let ws = Some(tmp.to_str().unwrap().to_string());
        assert!(studio_delete_asset(ws.clone(), outside.to_str().unwrap().into()).is_err());
        assert!(outside.exists(), "目录外的文件被删了");
        // 路径穿越也要拒绝
        let escape = gen.join("../../别动我.txt");
        assert!(studio_delete_asset(ws.clone(), escape.to_str().unwrap().into()).is_err());
        assert!(outside.exists());
        // 目录内的正常删
        assert!(studio_delete_asset(ws, inside.to_str().unwrap().into()).is_ok());
        assert!(!inside.exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// 只列图和视频，别的扩展名不该混进来。
    #[test]
    fn 资产库按时间倒序且只收图和视频() {
        let tmp = std::env::temp_dir().join(format!("anima_assets2_{}", std::process::id()));
        let gen = tmp.join(".anima").join("generated");
        std::fs::create_dir_all(&gen).unwrap();
        for n in ["a.png", "b.mp4", "c.json", "d.txt"] {
            std::fs::write(gen.join(n), b"xx").unwrap();
        }
        let v = studio_assets(Some(tmp.to_str().unwrap().to_string()));
        let names: std::collections::HashSet<_> = v.iter().map(|a| a.name.as_str()).collect();
        let want: std::collections::HashSet<&str> = ["a.png", "b.mp4"].into_iter().collect();
        assert_eq!(names, want);
        assert_eq!(v.iter().find(|a| a.name == "a.png").unwrap().kind, "image");
        assert_eq!(v.iter().find(|a| a.name == "b.mp4").unwrap().kind, "video");
        assert!(v.windows(2).all(|w| w[0].at >= w[1].at), "没按时间倒序");
        // 目录不存在时给空，不要 panic
        assert!(studio_assets(Some("/不存在的路径".into())).is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// 历史去重：连点五次生成不该刷出五条一模一样的。
    #[test]
    fn 历史同提示词只留最新一条() {
        // 纯函数化 `studio_add_history` 的核心，避免测试写到真实配置目录
        fn merge(
            mut all: Vec<PromptHistoryItem>,
            item: PromptHistoryItem,
        ) -> Vec<PromptHistoryItem> {
            all.retain(|h| h.prompt.trim() != item.prompt.trim());
            all.insert(0, item);
            all.truncate(HISTORY_LIMIT);
            all
        }
        let mk = |p: &str, at: u64| PromptHistoryItem {
            prompt: p.into(), mode: "t2i".into(), engine: "zimage".into(), at,
        };
        let mut v = vec![];
        v = merge(v, mk("老茶师", 1));
        v = merge(v, mk("窗外下雨", 2));
        v = merge(v, mk("老茶师", 3));
        assert_eq!(v.len(), 2, "重复的没去掉");
        assert_eq!(v[0].at, 3, "最新的没排在最前");
        // 首尾空白不该被当成不同的提示词
        v = merge(v, mk("  老茶师  ", 4));
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].at, 4);
        // 上限要真的生效，否则列表会长到没法扫
        let mut big = vec![];
        for i in 0..(HISTORY_LIMIT + 20) {
            big = merge(big, mk(&format!("p{i}"), i as u64));
        }
        assert_eq!(big.len(), HISTORY_LIMIT);
        assert_eq!(big[0].prompt, format!("p{}", HISTORY_LIMIT + 19), "最新的该在最前");
    }

    /// **同 seed 不同模式不能撞文件名。**
    ///
    /// 工具默认 `img_{seed}.png`，同 seed 换模式会覆盖上一张，
    /// 而画廊里旧卡片还指着同一路径 —— 显示的是新图，任务却是成功的。
    #[test]
    fn 每次生成的文件名都不一样() {
        let mut names = std::collections::HashSet::new();
        for m in ["t2i", "i2i", "t2v", "i2v"] {
            let imgs: Vec<&str> = if m.starts_with('t') { vec![] } else { vec!["/a.png"] };
            let (_, v) = to_tool_call(&ga(m, "zimage", &imgs)).unwrap();
            let n = v["output_filename"].as_str().expect("没有文件名").to_string();
            assert!(n.contains(m), "文件名里没有模式：{n}");
            // 视频出 mp4，图出 png —— 扩展名错了播放器打不开
            let want = if m.ends_with("2v") { ".mp4" } else { ".png" };
            assert!(n.ends_with(want), "{m} 的扩展名不对：{n}");
            assert!(names.insert(n.clone()), "文件名撞了：{n}");
        }
        // 显式给了就用给的
        let mut a = ga("t2i", "zimage", &[]);
        a.output_filename = Some("我的图.png".into());
        let (_, v) = to_tool_call(&a).unwrap();
        assert_eq!(v["output_filename"], "我的图.png");
    }

    /// 提示词里 `@` 出来的参考图必须真的进到调用里。
    ///
    /// **解析出来不用等于没做。** 界面上会显示一排头像，
    /// 让人以为挂上了，而生成的时候一张都没传 —— 典型的静默失败。
    #[test]
    fn 提示词里的引用会变成参考图() {
        let mut a = ga("r2v", "h3", &[]);
        a.reference_images = vec!["/lib/laochashi.png".into()];
        let (tool, v) = to_tool_call(&a).expect("只有 @ 引用、没有槽位图也该能跑");
        assert_eq!(tool, "animate_image");
        assert_eq!(v["mode"], "reference");
        assert_eq!(v["image_path"], "/lib/laochashi.png");
        assert_eq!(v["reference_images"][0], "/lib/laochashi.png");
    }

    #[test]
    fn 槽位图和引用图两个来源都要用上() {
        // 只认一个来源的话，另一边就成了「点了没反应」。
        let mut a = ga("r2v", "h3", &["/slot.png"]);
        a.reference_images = vec!["/slot.png".into(), "/ref.png".into()];
        let (_, v) = to_tool_call(&a).unwrap();
        let list = v["reference_images"].as_array().unwrap();
        assert_eq!(list.len(), 2, "重复的没去掉，或者少了一个来源：{list:?}");
        assert_eq!(list[0], "/slot.png");
        assert_eq!(list[1], "/ref.png");
    }

    /// 图不够时**提交前就失败**，别让人等完才发现少了一张。
    #[test]
    fn 图不够时提前失败并说清缺几张() {
        for (m, need) in [("i2v", 1), ("fl2v", 2), ("r2v", 1), ("i2i", 1), ("pose", 1)] {
            let e = to_tool_call(&ga(m, "ltx", &[])).unwrap_err();
            assert!(e.contains(&need.to_string()) && e.contains(m), "{m}: {e}");
        }
        // 首尾帧只给一张也要拦住
        assert!(to_tool_call(&ga("fl2v", "ltx", &["/a.png"])).is_err());
        assert!(to_tool_call(&ga("不存在", "ltx", &[])).is_err());
    }

    /// Studio 的端到端：**打真 ComfyUI，验到文件落盘。**
    ///
    /// 走的是 `studio_generate` 里那一条完全相同的路径（`to_tool_call` →
    /// agent 工具），只是不经过 Tauri 的 State。UI 那一层（点击 → invoke →
    /// store）测不了，只能人点。
    ///
    /// `ANIMA_STUDIO_MODE=t2i ANIMA_STUDIO_ENGINE=zimage \
    ///   cargo test --lib studio_generate_e2e -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn studio_generate_e2e() {
        use crate::agent_runtime::{ToolCallRequest, ToolHandler, ToolStatus};

        let mode = std::env::var("ANIMA_STUDIO_MODE").unwrap_or_else(|_| "t2i".into());
        let engine = std::env::var("ANIMA_STUDIO_ENGINE").unwrap_or_else(|_| "zimage".into());
        let images: Vec<String> = std::env::var("ANIMA_STUDIO_IMAGES")
            .map(|v| v.split(',').map(|s| s.trim().to_string()).collect())
            .unwrap_or_default();
        let (w, h) = if mode.starts_with('t') && mode.ends_with("2i") {
            (1024, 1024)
        } else {
            (768, 512)
        };

        let args = GenerateArgs {
            h3_official_format: None,
            h3_quality: None,
            audios: vec![],
            mode: mode.clone(),
            engine: engine.clone(),
            prompt: std::env::var("ANIMA_PROMPT")
                .unwrap_or_else(|_| "一位老茶师提起黄铜壶向白瓷茶盏注水，热气升腾，电影质感".into()),
            negative_prompt: "模糊, 多余的手".into(),
            width: w,
            height: h,
            seed: Some(20_260_904),
            images,
            duration: Some(5.0),
            generate_audio: None,
            auto_loras: None, viewpoints: Vec::new(), refine: None,
            // 联调脚本里可以用 ANIMA_DENOISE 试不同重绘幅度
            denoise: std::env::var("ANIMA_DENOISE").ok().and_then(|v| v.parse().ok()),
            // 联调脚本里可以用 ANIMA_REF_BOOST 试设定表的保真度旋钮
            ref_boost: std::env::var("ANIMA_REF_BOOST").ok().and_then(|v| v.parse().ok()),
            entity_type: None,
            entity_name: None,
            identity_locks: vec![],
            style_line: None,
            workspace_path: std::env::var("ANIMA_WS").ok(),
            loras: vec![],
            output_filename: None,
            reference_images: vec![],
        };

        let (tool, targs) = to_tool_call(&args).expect("参数映射失败");
        println!("  模式 {mode} / 引擎 {engine} → 工具 {tool}");
        println!("  参数 {}", serde_json::to_string(&targs).unwrap());

        let ws = args
            .workspace_path
            .clone()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let ctx = crate::agent_runtime::RuntimeContext::new(
            ws.clone(),
            crate::agent_runtime::RuntimePolicy::workspace_write(&ws),
        );
        let req = ToolCallRequest::new(tool, targs);
        let out = match tool {
            "generate_image" => {
                crate::agent_runtime::image_tool::GenerateImageTool.execute(&ctx, req).await
            }
            "generate_video" => {
                crate::agent_runtime::video_tool::GenerateVideoTool.execute(&ctx, req).await
            }
            _ => crate::agent_runtime::video_tool::AnimateImageTool.execute(&ctx, req).await,
        };

        println!("  状态 {:?}", out.status);
        println!("  产物 {:?}", out.artifacts);
        if out.status != ToolStatus::Ok {
            panic!("失败：{}", out.failure.map(|f| f.message).unwrap_or(out.content));
        }
        // **只看 status 不够** —— 这个项目最典型的失败就是「成功了但没有文件」
        let path = out.artifacts.first().expect("成功了却没有产物路径");
        let n = std::fs::metadata(path).expect("产物路径指向的文件不存在").len();
        assert!(n > 10_000, "文件太小，可能是空的：{n} 字节");
        println!("  ✓ {path}  {:.2} MB", n as f64 / 1e6);
    }

    /// 激活词的判据用**本机 13 个 LoRA 里真实观察到的值**钉住。
    ///
    /// 2026-09-04 全看了一遍：没有一个有 `trigger_words` 字段，
    /// 只有 kohya 写的 `ss_tag_frequency`，里面混着两类东西。
    #[test]
    fn 激活词只挑稀有token不挑概念词() {
        use serde_json::json;

        // Straight_hair_zimgt 的真实值 —— 造出来的 token，出现一次
        let z1 = json!({"ds": {"j1v_w0m": 1}});
        assert_eq!(trigger_candidates(&z1), vec!["j1v_w0m"]);

        // shorty_asian_zimgt 的真实值
        let z2 = json!({"ds": {"sh0r7y_asian": 1}});
        assert_eq!(trigger_candidates(&z2), vec!["sh0r7y_asian"]);

        // CharTurnXLRetag_v4 的真实值 —— 全是概念词，一个都不该挑
        let sdxl = json!({"ds": {
            "back view": 85, "side view": 84, "front view": 83,
            "full-body view": 69, "3d model": 69, "character design": 59
        }});
        assert!(trigger_candidates(&sdxl).is_empty(), "概念词被当成激活词了");

        // add-detail-xl 的真实值 —— 垃圾标签，更不该挑
        let junk = json!({"ds": {"misc": 8208, "all": 387, "portrait": 40, "anime": 20}});
        assert!(trigger_candidates(&junk).is_empty());

        // Asian_Beauty XL 的真实值。**这一组是判据的分水岭**：
        // 后两句出现次数只有 2 和 1（够稀有），但它们是自然语言长描述，
        // 不是造出来的 token。只看次数就会把整句话塞进提示词。
        // 变异测试抓出来的 —— 我第一版测试数据里正好缺了这一类。
        let ab = json!({"ds": {
            "asian girls": 104, "beautiful face": 104, "beauty": 104,
            "a woman in a bikini standing on a beach next to the ocean": 6,
            "a woman in a bikini standing on a beach with footprints in the sand": 2,
            "a woman with very large breast standing in the street": 1,
            // 短的带空格标签：`len <= 24` 挡不住它，只有空格检查能。
            // 少了这一条，「不检查空格」那个变异就测不出来
            "back view 3d": 1
        }});
        assert!(
            trigger_candidates(&ab).is_empty(),
            "低频的长描述被当成激活词了：{:?}",
            trigger_candidates(&ab)
        );

        // 多数据集要合并计数：同一个词分散在两个子集里，合起来就不算稀有
        let split = json!({"a": {"back_view": 2}, "b": {"back_view": 3}});
        assert!(trigger_candidates(&split).is_empty(), "跨数据集没合并计数");

        // 没有元数据时给空，不要瞎猜
        assert!(trigger_candidates(&json!({})).is_empty());
        assert!(trigger_candidates(&json!(null)).is_empty());

        // 最多给三个 —— 猜出一串塞进提示词比不猜更糟
        let many = json!({"ds": {"a1_x": 1, "b2_y": 1, "c3_z": 1, "d4_w": 1, "e5_v": 1}});
        assert!(trigger_candidates(&many).len() <= 3);
    }

    /// 每个引擎都得给尺寸和批量上限 —— 否则 UI 上那两个控件是空的，
    /// 而「控件在但没有选项」比没有控件更让人困惑。
    /// **按钮上的名字要短，不许换行。** 模式选择器是等宽分段，一排七个按钮，
    /// 三四个字的名字（「场景宫格」「参考表」）实测会折成两行把整排撑高。
    /// 完整名字留在 `label` 里，给提示句和悬浮说明用。
    #[test]
    fn 模式按钮名字不超过三个字且有完整名() {
        for m in catalog().modes {
            assert!(m.short.chars().count() <= 3, "「{}」按钮名太长：{}", m.label, m.short);
            assert!(m.label.chars().count() >= m.short.chars().count(), "{} 的完整名比短名还短", m.id);
        }
        let img: Vec<&str> = catalog().modes.iter().filter(|m| m.kind == "image").map(|m| m.short).collect();
        assert!(img.iter().all(|s| s.chars().count() <= 2), "图像那一排七个按钮，超过两个字会换行：{img:?}");
    }

    /// 每个 LoRA 架构都要映射到真的存在的引擎。
    #[test]
    fn 工作区列出的引擎都在目录里() {
        let c = catalog();
        for id in ["zimage", "sdxl", "krea2", "krea2_edit", "qwen21", "qwen21_edit"] {
            assert!(c.engines.iter().any(|e| e.id == id), "目录里没有 {id}");
        }
    }

    ///
    /// 2026-09-17 用户在创作台的两个 Pony 引擎下看到了
    /// `Qwen-Image-Edit-2511-Lightning-8steps`。根因：它的键名是
    /// `transformer_blocks.N.attn.add_k_proj.*`，而 LTX 那条要 `.attn1.`、
    /// H3 那条要 `.attn.out_proj.`，都不中 —— 于是落到「认不出一律当 SDXL」
    /// 的兜底，`engines()` 给出 `[sdxl, pony, pony_realistic]`，
    /// **在三个引擎下面都冒出来，挂上去静默失效。**
    #[test]
    fn qwen的lora不会被当成sdxl() {
        let dir = tempfile::tempdir().unwrap();
        // 造一份只有头的 safetensors：键名照真实文件写
        let keys = serde_json::json!({
            "transformer_blocks.0.attn.add_k_proj.lora_down.weight":
                {"dtype":"F16","shape":[8,64],"data_offsets":[0,1024]},
            "transformer_blocks.0.attn.add_q_proj.lora_up.weight":
                {"dtype":"F16","shape":[64,8],"data_offsets":[1024,2048]},
        });
        let head = serde_json::to_vec(&keys).unwrap();
        let mut buf = (head.len() as u64).to_le_bytes().to_vec();
        buf.extend_from_slice(&head);
        buf.extend_from_slice(&vec![0u8; 2048]);
        let name = "qwen_edit_lightning.safetensors";
        std::fs::write(dir.path().join(name), &buf).unwrap();

        let t = detect_arch(dir.path(), name);
        assert_eq!(t.id(), "qwen", "Qwen 的 LoRA 判成了 {}", t.id());
        assert!(
            t.engines().is_empty(),
            "Qwen 的 LoRA 不该出现在任何引擎下，实际：{:?}",
            t.engines()
        );
    }

    /// 2026-09-25：HMNSFW V2.5 / HMSquirt 只训了 MLP（`blocks.N.mlp.fc1`），
    /// 没有 `attn.out_proj`，原来会落到兜底当 SDXL。键名照真实文件写。
    /// 反面两条：diffusers 的 `down_blocks` 和 SDXL 文本编码器的 `layers.N.mlp.fc1` 不能被当成 H3。
    #[test]
    fn 只训mlp的h3_lora也认成h3() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, keys: &[&str]| {
            let mut m = serde_json::Map::new();
            for (i, k) in keys.iter().enumerate() {
                m.insert((*k).into(), serde_json::json!({"dtype":"F16","shape":[8,8],"data_offsets":[i * 128, (i + 1) * 128]}));
            }
            let head = serde_json::to_vec(&m).unwrap();
            let mut buf = (head.len() as u64).to_le_bytes().to_vec();
            buf.extend_from_slice(&head);
            buf.extend_from_slice(&vec![0u8; keys.len() * 128]);
            std::fs::write(dir.path().join(name), &buf).unwrap();
        };
        write("mlp_only.safetensors", &[
            "diffusion_model.blocks.0.mlp.fc1.lora_A.weight",
            "diffusion_model.blocks.0.mlp.fc2.lora_B.weight",
        ]);
        assert_eq!(detect_arch(dir.path(), "mlp_only.safetensors").id(), "h3");

        write("sdxl_te.safetensors", &[
            "text_encoder.text_model.encoder.layers.0.mlp.fc1.lora_A.weight",
            "unet.down_blocks.1.attentions.0.proj_in.lora_A.weight",
            // 不存在于真实文件，专钉「blocks 要整段」：`ends_with("blocks")` 会把它吃掉
            "unet.down_blocks.1.mlp.fc1.lora_A.weight",
        ]);
        assert_ne!(detect_arch(dir.path(), "sdxl_te.safetensors").id(), "h3");
    }

    /// 2026-09-08 出过：`LoraArch` 加了 `Krea2`，`engines_for_arch` 当时是
    /// `match arch: &str` 带 `_ => sdxl` 兜底，于是刚下的 Krea 2 LoRA
    /// 被算成只能挂 SDXL —— 选 Krea 2 时界面上根本不显示，而且不报错。
    /// **兜底把编译器的嘴堵住了**，所以现在这层是枚举，加变体会编译不过。
    #[test]
    fn 每个架构都映射到能挂它的引擎() {
        use crate::agent_runtime::image_engine::LoraArch;
        for (t, want) in [
            (LoraTarget::Image(LoraArch::ZImage), vec!["zimage"]),
            (LoraTarget::Image(LoraArch::Sdxl), vec!["sdxl"]),
            (LoraTarget::Image(LoraArch::Krea2), vec!["krea2"]),
            // FLUX.2 引擎已经没了 —— 认得出但没地方挂，给空列表
            (LoraTarget::Image(LoraArch::Flux2), vec![]),
            (LoraTarget::Ltx, vec!["ltx"]),
            (LoraTarget::H3, vec!["h3"]),
            // Qwen-Image-Edit：同 FLUX.2，认得出但没引擎能挂
            (LoraTarget::Qwen, vec![]),
        ] {
            assert_eq!(t.engines(), want, "{} 的引擎映射不对", t.id());
        }

        // **映射出来的引擎必须真的在能力目录里**，否则界面永远过滤不出它
        let c = catalog();
        let known: Vec<&str> = c.engines.iter().map(|e| e.id).collect();
        for t in [
            LoraTarget::Image(LoraArch::ZImage),
            LoraTarget::Image(LoraArch::Sdxl),
            LoraTarget::Image(LoraArch::Krea2),
            LoraTarget::Ltx,
            LoraTarget::H3,
        ] {
            for e in t.engines() {
                assert!(known.contains(&e.as_str()), "{} 映射到了不存在的引擎 {e}", t.id());
            }
        }

        // id 不能重名 —— 重名的话界面上两种架构显示成一个
        let ids: Vec<&str> = [
            LoraTarget::Image(LoraArch::ZImage),
            LoraTarget::Image(LoraArch::Sdxl),
            LoraTarget::Image(LoraArch::Krea2),
            LoraTarget::Image(LoraArch::Flux2),
            LoraTarget::Ltx,
            LoraTarget::H3,
        ]
        .iter()
        .map(|t| t.id())
        .collect();
        let mut uniq = ids.clone();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(uniq.len(), ids.len(), "架构 id 有重名");
    }

    /// **盘上真实的 LoRA 会不会出现在对应引擎的列表里。**
    ///
    /// 上面那条钉的是映射表；这条钉的是「下载好的这个文件，界面上选那个引擎
    /// 时能不能看见它」—— 而用户报的正是「刚下的 LoRA 这里咋没有看到」。
    ///
    /// `cargo test --lib 真实lora能出现在 -- --ignored`
    #[test]
    #[ignore]
    fn 真实lora能出现在对应引擎的列表里() {
        let Some(dir) = crate::cinema_sheet::loras_dir() else {
            return;
        };
        for (name, engine) in [
            ("zimage/NexBlend_Asian_Face_01_ZIT.safetensors", "zimage"),
            ("krea/Krea2NexBlendSemiRealistic.safetensors", "krea2"),
            ("krea/krea2_kgirl_v3.safetensors", "krea2"),
            ("krea/hina_krea2Turbo_asianmix_v4.safetensors", "krea2"),
        ] {
            if !dir.join(name).is_file() {
                eprintln!("跳过（盘上没有）：{name}");
                continue;
            }
            let t = detect_arch(&dir, name);
            assert!(
                t.engines().iter().any(|e| e == engine),
                "{name} 判成了 {}，挂不到 {engine} 上 —— 界面里会看不见它",
                t.id()
            );
        }
    }

    /// **重绘幅度要真的传到工具。** 界面上加了滑杆而参数没跟着走，
    /// 表现是「拉了没反应」—— 这个仓库最常见的失败形状。
    #[test]
    fn 图生图把重绘幅度传给工具() {
        let mut a = ga("i2i", "zimage", &["/src.png"]);
        a.denoise = Some(0.35);
        let (tool, args) = to_tool_call(&a).unwrap();
        assert_eq!(tool, "generate_image");
        assert_eq!(args["image_path"], "/src.png");
        assert_eq!(args["denoise"], 0.35);
    }

    /// **不给就别塞。** 塞一个 null 进去会盖掉工具那边
    /// 「有源图就默认 0.65」的判断，而那种偏差没人看得出来。
    #[test]
    fn 没设重绘幅度时不塞进参数() {
        let (_, args) = to_tool_call(&ga("i2i", "zimage", &["/src.png"])).unwrap();
        assert!(args.get("denoise").is_none(), "凭空塞了一个 denoise：{args}");
    }

    /// 越界的值要夹住 —— 0 会让采样器一步都不跑，出来是原图且不报错。
    #[test]
    fn 越界的重绘幅度会被夹住() {
        for (给, 至少) in [(0.0_f64, 0.05_f64), (-1.0, 0.05)] {
            let mut a = ga("i2i", "zimage", &["/src.png"]);
            a.denoise = Some(给);
            let (_, args) = to_tool_call(&a).unwrap();
            assert!(args["denoise"].as_f64().unwrap() >= 至少, "{给} 没夹住");
        }
        let mut a = ga("i2i", "zimage", &["/src.png"]);
        a.denoise = Some(5.0);
        let (_, args) = to_tool_call(&a).unwrap();
        assert!(args["denoise"].as_f64().unwrap() <= 1.0);
    }

    /// 文生图那边传了也不该带上 —— 那条路 denoise 恒为 1，
    /// 带上去只会让人以为它管用。
    #[test]
    fn 文生图不带重绘幅度() {
        let mut a = ga("t2i", "zimage", &[]);
        a.denoise = Some(0.3);
        let (_, args) = to_tool_call(&a).unwrap();
        assert!(args.get("denoise").is_none(), "文生图带上了 denoise");
    }
}

#[cfg(test)]
mod lora_note_tests {
    use super::*;

    /// **老格式（纯字符串）要能读出来。**
    ///
    /// 直接换成结构体的话，已经写过备注的人一升级全没了 ——
    /// 而且不报错，只是备注空了。
    #[test]
    fn 老格式的纯字符串备注读得出来() {
        let raw: std::collections::BTreeMap<String, serde_json::Value> =
            serde_json::from_str(r#"{"a.safetensors":"一句话"}"#).unwrap();
        let parsed: std::collections::BTreeMap<String, LoraNote> = raw
            .into_iter()
            .map(|(k, v)| {
                let n = match &v {
                    serde_json::Value::String(s) => LoraNote { note: s.clone(), ..Default::default() },
                    _ => serde_json::from_value(v).unwrap_or_default(),
                };
                (k, n)
            })
            .collect();
        assert_eq!(parsed["a.safetensors"].note, "一句话");
        assert!(parsed["a.safetensors"].triggers.is_empty());
    }

    /// 2026-09-25 风格窗口：分类和出处要存得进、读得出；没写的老条目读出来是空（不报错），
    /// 空的不写回文件（不给每条都塞两个空字段）
    #[test]
    fn 分类和出处读写一圈不丢() {
        let n: LoraNote = serde_json::from_str(
            r#"{"note":"赛博","triggers":[],"nsfw":false,"category":"画风","source_url":"https://civitai.com/models/1"}"#,
        ).unwrap();
        assert_eq!((n.category.as_str(), n.source_url.as_str()), ("画风", "https://civitai.com/models/1"));
        let back = serde_json::to_string(&n).unwrap();
        assert!(back.contains("\"category\":\"画风\"") && back.contains("source_url"));
        let old: LoraNote = serde_json::from_str(r#"{"note":"旧的"}"#).unwrap();
        assert!(old.category.is_empty() && old.source_url.is_empty());
        assert!(!serde_json::to_string(&old).unwrap().contains("category"));
    }

    /// 视频类 LoRA 的示例是 mp4，有它就用它；没有再找 jpg；都没有给 None
    #[test]
    fn 示例先找视频再找图() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("h3")).unwrap();
        std::fs::write(dir.path().join("h3/a.jpg"), b"x").unwrap();
        assert!(lora_thumb_in(dir.path(), "h3/a.safetensors").unwrap().ends_with("h3/a.jpg"));
        std::fs::write(dir.path().join("h3/a.mp4"), b"x").unwrap();
        assert!(lora_thumb_in(dir.path(), "h3/a.safetensors").unwrap().ends_with("h3/a.mp4"));
        assert_eq!(lora_thumb_in(dir.path(), "h3/none.safetensors"), None);
    }

    /// 新格式里两样都在。
    #[test]
    fn 新格式带激活词() {
        let n: LoraNote =
            serde_json::from_str(r#"{"note":"说明","triggers":["Asian woman"]}"#).unwrap();
        assert_eq!(n.triggers, vec!["Asian woman"]);
    }

    /// 缺字段时给默认值，不是解析失败 —— 失败的话整份备注会静默变空。
    #[test]
    fn 缺字段不会让整份备注读不出来() {
        let n: LoraNote = serde_json::from_str(r#"{"note":"只有说明"}"#).unwrap();
        assert!(n.triggers.is_empty());
        let n2: LoraNote = serde_json::from_str(r#"{"triggers":["x"]}"#).unwrap();
        assert_eq!(n2.note, "");
    }

    // ---- 成功但有话要说 ----

    fn 出图返回(warnings: &[&str]) -> String {
        let mut out = String::new();
        for w in warnings {
            out.push_str(&format!("⚠️ {w}\n\n"));
        }
        out.push_str("- **Seed**: `123`（改这张图的细节时把同一个 seed 传回来）\n");
        out.push_str("![生成的图像](file:///x/a.png)\n\n- **模型**: `sdxl`\n");
        out
    }

    /// **回落说明必须能被摘出来。**
    ///
    /// 用户选 Z-Image + 图生图，实际拿到的是 SDXL 的图 —— 工具那边说了，
    /// 而创作台以前把整段 message 在成功时丢掉，等于没说。
    #[test]
    fn 摘得出回落说明() {
        let n = notices(&出图返回(&[
            "已改用 SDXL 出图（你要的是 Z-Image Turbo）：图生图（改图）通路只有 SDXL 架构的权重。",
        ]));
        assert_eq!(n.len(), 1, "{n:?}");
        assert!(n[0].contains("已改用 SDXL"), "{}", n[0]);
        assert!(n[0].contains("图生图"), "{}", n[0]);
        assert!(!n[0].starts_with('⚠'), "警告符号没剥掉：{}", n[0]);
    }

    /// **告警可能是多行的**（LoRA 那条会把本机有哪些列出来）。
    /// 所以切法是按 Seed 那一行切，不是逐行认 ⚠️ 开头。
    #[test]
    fn 多行的告警不会被切断() {
        let n = notices(&出图返回(&[
            "这些 LoRA 名字对不上，已跳过：foo.safetensors\n本机有：\n- sdxl/a.safetensors\n- zimage/b.safetensors",
        ]));
        assert_eq!(n.len(), 1);
        assert!(n[0].contains("zimage/b.safetensors"), "多行被切断了：{}", n[0]);
    }

    #[test]
    fn 多条告警各算一条() {
        let n = notices(&出图返回(&["第一条告警", "第二条告警"]));
        assert_eq!(n, vec!["第一条告警".to_string(), "第二条告警".to_string()]);
    }

    /// **告警前面的正文不算告警。**
    ///
    /// 视频那条返回没有 `- **Seed**` 这一行，走的是 `None => summary` 分支，
    /// 整段都会被扫 —— 这时候第一段是正文，不是告警。
    /// 少一个 `skip(1)` 就会把正文当成一条提示挂到卡片上。
    #[test]
    fn 告警之前的正文不算告警() {
        let 视频返回 = "已生成 3 秒视频，保存在工作区。\n\n⚠️ LTX 在这台机器上需要 112GB 显存\n";
        let n = notices(视频返回);
        assert_eq!(n.len(), 1, "把正文也当成告警了：{n:?}");
        assert!(n[0].starts_with("LTX"), "{}", n[0]);
    }

    /// 空的 / 只有空白的告警不算一条 —— 界面上会渲染成一个空行，
    /// 看着像「有话要说但没说出来」。
    #[test]
    fn 空的告警不算一条() {
        assert!(notices("⚠️ ").is_empty());
        assert!(notices("⚠️\n\n⚠️ 真的告警\n\n- **Seed**: `1`") == vec!["真的告警".to_string()]);
    }

    /// **摘出来还得真的用上。**
    ///
    /// `notices()` 自己测得再全，只要 `studio_generate` 不调它，
    /// 界面上照样什么都没有 —— 而这正是这次要修的那个 bug 的形状：
    /// 工具拼好了告警，最后一环把它扔了。
    #[test]
    fn 生成结果里真的带上了告警() {
        // **先把测试模块切掉。** 这个字符串在下面这条测试里也出现，
        // 而 `find` 拿的是第一个 —— 现在恰好是对的（真函数在前），
        // 但换个顺序就会去匹配测试自己的字面量。这个坑在这个仓库里
        // 已经踩过好几次了。
        let src = include_str!("studio.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("源码是空的");
        let i = src
            .find("pub async fn studio_generate")
            .expect("找不到生成入口");
        let body = &src[i..];
        assert!(
            body.contains("notices(&message)"),
            "生成入口没调 notices —— 告警摘出来了但没人用"
        );
        assert!(
            body.contains("notes:"),
            "GenerateOutput 没填 notes"
        );
    }

    /// **没有告警时不能凭空造一条。** 每张图都挂个提示，用户很快就不看了。
    #[test]
    fn 没有告警就是空的() {
        assert!(notices(&出图返回(&[])).is_empty());
        assert!(notices("").is_empty());
    }

    /// Seed 那一行**之后**的内容不算告警 —— 提示词回显里可能带 ⚠️ 这个字符。
    #[test]
    fn 只看_seed_之前的部分() {
        let mut s = 出图返回(&["真的告警"]);
        s.push_str("- **本次提示词**：`一个 ⚠️ 标志牌`\n");
        let n = notices(&s);
        assert_eq!(n.len(), 1, "把提示词里的符号也当成告警了：{n:?}");
        assert_eq!(n[0], "真的告警");
    }

    /// **模式表里说能用的引擎，必须真的有那条通路。**
    ///
    /// 图生图这条以前就是错的：模式表对四个引擎全开，而只有 SDXL 接了源图，
    /// 其余三个靠 `pick_engine` 回落 SDXL 兜着。用户选了 Z-Image，
    /// 拿到的是 SDXL 的图。
    #[test]
    fn 图生图列出的引擎都真的支持() {
        use crate::agent_runtime::image_engine::{ImageEngine, supports_i2i};
        let c = catalog();
        let m = c.modes.iter().find(|m| m.id == "i2i").expect("没有图生图这个模式");
        assert!(!m.engines.is_empty());
        for id in &m.engines {
            let e = ImageEngine::from_id(id).unwrap_or_else(|| panic!("认不出的引擎 {id}"));
            assert!(supports_i2i(e), "模式表说 {id} 能图生图，而它没有那条通路");
        }
    }

    /// 反过来：有通路的引擎不该被模式表挡在外面 —— 那是白丢一条能力。
    #[test]
    fn 有图生图通路的引擎都列了出来() {
        use crate::agent_runtime::image_engine::{ImageEngine, supports_i2i};
        let c = catalog();
        let m = c.modes.iter().find(|m| m.id == "i2i").unwrap();
        for id in ["zimage", "krea2", "sdxl"] {
            if supports_i2i(ImageEngine::from_id(id).unwrap()) {
                assert!(m.engines.contains(&id), "{id} 有图生图却没列出来");
            }
        }
    }

}
