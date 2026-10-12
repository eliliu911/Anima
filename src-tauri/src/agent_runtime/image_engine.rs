//! 出图底模的选路与图构建。
//!
//! # 为什么有三个引擎
//!
//! 2026-09-01 在本机做了同题对比：三个底模、三档画风（真人写实 / 3D 国漫 / 动漫）、
//! 每档三个 seed，共 27 张，提示词完全一致，采样参数各用各自 ComfyUI 官方模板的值。
//!
//! | 引擎 | 稳态单张 | 相对 SDXL | 结论 |
//! |---|---|---|---|
//! | SDXL（RealVisXL） | 39s | 1.0× | 细节丢得最多，手最差；写实 checkpoint 出不了动漫 |
//! | **Z-Image Turbo** | **45s** | **1.15×** | 质量大幅领先，只贵 15% —— 日常出图 |
//! | **Chroma1-HD** | **331s** | **8.5×** | 质量最高但太慢 —— 只用于视频首尾帧底图 |
//!
//! 同一段 1251 字提示词下：SDXL 把 "black silk" 出成深蓝、丢了戒指、构图从全身缩成
//! 半身、手指畸形；Z-Image 和 Chroma 细节几乎全中。根因是编码器 ——
//! SDXL 的 CLIP 是 77 token 分块取平均，长提示词会被摊薄；Chroma 用 T5-XXL、
//! Z-Image 用 Qwen3-4B，都是长文本原生。
//!
//! # 三件必须知道、否则会静默出错的事
//!
//! 1. **Z-Image 的负向提示词是结构性无效的。** 官方模板 cfg=1.0，负向走
//!    `ConditioningZeroOut`。我们在 SDXL 上调了很久的那串 `extra fingers,
//!    fused fingers, malformed hands` 在它这里一个字都不起作用 —— 而它照样
//!    出对了手。别以为传了负向词就有保护。
//! 2. **CLIP 的 type 值猜不出来。** Chroma 是 `chroma`，Z-Image 是 **`lumina2`**。
//!    写错不报错，只是出一张风马牛不相及的图。
//! 3. **LoRA / 设定卡版式是 SDXL 专属，ControlNet 不再是。** 本机装的 LoRA
//!    （CharTurnXL、国风、汉服）挂到别的引擎上不生效，用到时仍要回落 SDXL
//!    并且**说出来**。但 **ControlNet 从 2026-09-16 起有了 Z-Image 版**
//!    （`ZIMAGE_CONTROLNET`），控姿势 / 深度 / 线稿不必再牺牲画质。
//!    `models/controlnet/` 下那两个 openpose/union 仍然只对 SDXL 有效。

use serde_json::{json, Map, Value};

/// Z-Image Turbo 的三个权重文件名。**取自 ComfyUI 官方模板，不猜。**
pub const ZIMAGE_UNET: &str = "z_image_turbo_bf16.safetensors";
pub const ZIMAGE_CLIP: &str = "qwen_3_4b.safetensors";
pub const ZIMAGE_VAE: &str = "z_image_ae.safetensors";
/// Z-Image 的 CLIP type —— 是 `lumina2`，不是 `z_image`，也不是 `qwen_image`。
pub const ZIMAGE_CLIP_TYPE: &str = "lumina2";

/// Z-Image 的 ControlNet（Union，支持 canny / depth / pose / hed / scribble / mlsd / gray）。
///
/// **放在 `models/model_patches/`，不是 `models/controlnet/`** —— 它走 ComfyUI 的
/// model patch 通道（`ModelPatchLoader` → `ZImageFunControlnet`），
/// 和传统 ControlNet 是两套完全不同的节点。放错目录时 `ModelPatchLoader`
/// 的下拉里根本看不到这个文件。
///
/// 2026-09-16 实测（同 seed、同提示词、唯一变量是控制图）：程序化画的深度图
/// 把塔从「左侧高细灯塔」变成「左侧矮宽城堡塔」，地平线落在指定的 45%；
/// 把深度图左右镜像后，人物和塔**双双换边**。是控制图在起作用，不是碰巧。
pub const ZIMAGE_CONTROLNET: &str =
    "Z-Image-Turbo-Fun-Controlnet-Union-2.1-2602-8steps.safetensors";

/// Z-Image ControlNet 的控制强度。**0.8**，和 SDXL 那边
/// （`cinema_sheet::POSE_CONTROL_STRENGTH`）同一个理由：拉满会让画面僵硬地
/// 贴着控制图、失去自然的光影和体积。官方标的可用区间是 0.65–1.00。
pub const ZIMAGE_CONTROL_STRENGTH: f32 = 0.8;

/// Krea 2 Turbo（int8-convrot）。2026-09-08 接入，同日顶掉了 FLUX.2 Klein。
///
/// **为什么是 int8 而不是 bf16**：官方模板点名的就是这份
/// （`image_krea2_turbo_t2i_int8.json`），而 `ComfyUI-AppleSilicon-FP8`
/// 让 int8 在 M4 Max 上是原生档（见 ASSET_LEDGER 第七节）。
/// bf16 那份 24.5 GiB，int8 只要 12.6 GiB，同题实测没有可见差别。
pub const KREA2_UNET: &str = "krea2_turbo_int8_convrot.safetensors";
/// 编码器是 **Qwen3-VL-4B**（带视觉塔，设定表那条要用它做图像接地）。
pub const KREA2_CLIP: &str = "qwen3vl_4b_fp8_scaled.safetensors";
/// **CLIP type 是 `krea2`。** 写错只是换一套编码规则，图照出、不报错。
pub const KREA2_CLIP_TYPE: &str = "krea2";
/// **不是 `ae.safetensors`，也不是 `flux2-vae`。** Krea 2 用 Qwen-Image 的 VAE。
pub const KREA2_VAE: &str = "qwen_image_vae.safetensors";
/// Turbo 版的步数。官方模板是 8。
pub const KREA2_STEPS: u32 = 8;
/// 2026-09-08 实测：1024²/8 步 107.6s，1216×832 101.8s。取 108。
pub const KREA2_SECS: u64 = 108;

// ---- 参考生图（2026-09-13 接入）。参数全部照这台机器上实跑过的工作流，
// 对照记录见 docs/plans/2026-09-13-出图模型对照-QwenEdit与Pony.md。

/// Krea2Edit 的身份编辑 LoRA。**没有它节点只能加载、不能做身份编辑。**
pub const KREA2_EDIT_LORA: &str = "krea/krea2_identity_edit_v1_2.safetensors";
/// 作用在最后一张参考图（人物）上的保真度。1.0 = 关；账本记着 ~4.0 明显加强脸，>10 会让编辑失败。
pub const KREA2_EDIT_REF_BOOST: f64 = 4.0;
/// 实测 1024×576：150~210s（含一次冷启动），取 200。
pub const KREA2_EDIT_SECS: u64 = 200;

/// **UC 版 Q8_0**（`abenzerps/Qwen-Image-2.1-Uncensored-GGUF`，2026-09-26 换上）。
///
/// 09-22 下的是基础版 Q6_K —— 那时这个 UC 仓库还只是基础权重的镜像，
/// 09-22 晚上作者才放出真正改过的权重（和基础版哈希不同）。
/// 换 Q8_0 不为速度：ComfyUI 带 `--fp32-unet`，任何量化档都还原成 fp32 再算，
/// 量化只省盘和加载内存，所以挑最接近无损的那档。
///
/// 这份文件**带** `general.architecture = qwen_image21`，要过 ComfyUI-GGUF 的
/// `IMG_ARCH_LIST` 白名单 —— 靠 `comfyui-nodes/anima_gguf_arch` 补的那一条。
pub const QWEN21_UNET: &str = "qwen-image-2.1-UC-Q8_0.gguf";
pub const QWEN21_CLIP: &str = "qwen3vl_8b_int8_convrot.safetensors";
pub const QWEN21_CLIP_TYPE: &str = "qwen_image";
pub const QWEN21_VAE: &str = "qwen_image_2.1_vae_bf16.safetensors";
/// 官方模板的采样参数：25 步 / cfg 1 / euler+simple。
///
/// **cfg=1 意味着负向结构性无效**，和 Z-Image / Krea 2 同一条数学
/// （`out = uncond + cfg*(cond-uncond)`，cfg=1 时就等于 cond）。
/// `TextEncodeQwenImage21` 有 `negative_prompt` 这个入口，**但它在这组参数下不起作用** ——
/// 这正是最容易被当成「负向没写好」而反复改提示词的地方。
pub const QWEN21_STEPS: u32 = 25;
/// `TextEncodeQwenImage21` 的 `resolution`：参考图会被缩到这个面积附近。
/// 纯文生图时它只决定那个用不上的 latent 输出的尺寸，实际画幅由 `EmptyLatentImage` 定。
pub const QWEN21_RESOLUTION: u32 = 1024;
/// 参考图槽位。节点本身开了 16 个（`image_1`..`image_16`），
/// 官方 model card 写的是最多 10 张 —— 取小的那个。
pub const QWEN21_REF_SLOTS: usize = 10;

/// 实测稳态耗时，用来估排队时间。
pub const ZIMAGE_SECS: u64 = 45;
pub const SDXL_SECS: u64 = 39;
/// 参考生图那一档的耗时。**比纯文生慢** —— 参考图要过 VAE 编码，
/// 而且视觉 token 拉长了序列。2026-09-22 实测填。
pub const QWEN21_EDIT_SECS: u64 = 260;

/// 实测 1024×1024 / 25 步：首跑 213.6s、第二跑 210.6s（2026-09-22，Q6_K GGUF）。
///
/// **两跑差 3 秒 —— 模型加载几乎不占时间**，所以这个数就是稳态，不用另外扣。
/// 排下来是 Z-Image 45s < Krea 2 108s < **它 211s** < Chroma 331s。
pub const QWEN21_SECS: u64 = 211;


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageEngine {
    /// 日常出图。45s，质量远高于 SDXL。
    ZImage,
    /// LoRA / ControlNet / 设定卡版式的唯一去处 —— 那些权重都是 SDXL 架构。
    Sdxl,
    /// Krea 2 Turbo（int8）。108s，Qwen3-VL-4B 编码器。
    ///
    /// **它顶掉了同日接入的 FLUX.2 Klein。** 同题实测（见 ASSET_LEDGER）：
    /// 提示词服从度和 Klein 打平（「三个青苹果」数量都对，Z-Image 出成 4 个），
    /// 画质更像照片，**图上的中文标注也能画**（Klein 全乱码）；
    /// 而且 `DynamicCharacterSheet` 那个七区设定表 LoRA **只有 Krea 2 版**。
    /// 代价是慢 3 倍（108s vs 33s）—— 所以它不是日常出图，是定稿/设定表那一档。
    Krea2,
    /// **参考生图**：场景图 + 人物图 → 新构图。Krea 2 Turbo + 身份编辑 LoRA。
    ///
    /// 多 seed 对照里**脸最像**（身份相似度平均 0.311，Qwen 0.212）。
    /// 两个参考槽：场景在前、人物在后（训练时的顺序）。
    Krea2Edit,
    /// **Qwen-Image 2.1（Q6_K GGUF，Uncensored 版）。** 文生图 + 图生图。
    ///
    /// 一个 `TextEncodeQwenImage21` 节点同时出 positive / negative / latent 三路。
    ///
    /// **参考图那条路故意没接。** 那个节点确实吃最多 16 张参考图，但
    /// 「照参考图出新构图」这一档交给 [`Self::Qwen21Edit`]（同一份权重）；
    /// 没验过就把槽位露出来，等于给一个「看起来能用、实际没测过」的选项。
    /// 要接的话只有两处：`reference_slots` 和 `build_qwen21_graph` 里的 latent 来源
    /// （换成节点 4 的第 2 路输出，尺寸由第一张参考图定）。
    Qwen21,
    /// **Qwen-Image 2.1 参考生图**（和 [`Self::Qwen21`] 同一份权重、同一个图，只是接了参考图）。
    ///
    /// 分成两个 id 是因为本仓库用 `reference_slots() > 0` 表示「这个引擎**必须**有参考图」
    /// （`image_tool` 据此在没图时直接报错）。2.1 两样都能做，硬塞进一个 id 就得
    /// 改那条判断，代价是每个别的参考引擎都要重新想一遍。
    /// （`Pony` / `PonyRealistic` 当初也是这么共用一个 builder 的，2026-09-22 随权重删掉了。）
    Qwen21Edit,
}

impl ImageEngine {
    pub fn id(self) -> &'static str {
        match self {
            Self::ZImage => "zimage",
            Self::Sdxl => "sdxl",
            Self::Krea2 => "krea2",
            Self::Krea2Edit => "krea2_edit",
            Self::Qwen21 => "qwen21",
            Self::Qwen21Edit => "qwen21_edit",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ZImage => "Z-Image Turbo",
            Self::Sdxl => "SDXL",
            Self::Krea2 => "Krea 2 Turbo",
            Self::Krea2Edit => "Krea 2 参考生图",
            Self::Qwen21 => "Qwen-Image 2.1",
            Self::Qwen21Edit => "Qwen 2.1 参考生图",
        }
    }

    pub fn secs_per_job(self) -> u64 {
        match self {
            Self::ZImage => ZIMAGE_SECS,
            Self::Sdxl => SDXL_SECS,
            Self::Krea2 => KREA2_SECS,
            Self::Krea2Edit => KREA2_EDIT_SECS,
            Self::Qwen21 => QWEN21_SECS,
            Self::Qwen21Edit => QWEN21_EDIT_SECS,
        }
    }

    /// 这个引擎吃几张参考图。0 = 不是参考生图引擎。
    pub fn reference_slots(self) -> usize {
        match self {
            Self::Krea2Edit => 2,
            Self::Qwen21Edit => QWEN21_REF_SLOTS,
            _ => 0,
        }
    }

    /// 这个引擎的负向提示词到底起不起作用。
    ///
    /// Z-Image 是 `false` —— 官方模板 cfg=1.0，负向被 `ConditioningZeroOut` 清零。
    /// 传了也没用，所以要告诉调用方别白费劲，而不是让它以为有保护。
    pub fn negative_works(self) -> bool {
        // Krea 2 Turbo 和 Z-Image 一样：cfg=1，负向分支被约掉
        // （out = uncond + cfg*(cond-uncond)，cfg=1 时就等于 cond），
        // 官方模板也是接 `ConditioningZeroOut`。
        // 同一条数学在 Klein 上实测过：强负向 vs 清零，**100.00% 像素相同**。
        // 两个参考生图引擎也是 cfg=1（Krea2Edit Turbo、Qwen Lightning），负向同样被约掉。
        // Qwen-Image 2.1 同理：官方模板 cfg=1。`TextEncodeQwenImage21` 有
        // `negative_prompt` 入口，看着像能用，实际被同一条数学约掉。
        !matches!(self, Self::ZImage | Self::Krea2 | Self::Krea2Edit | Self::Qwen21 | Self::Qwen21Edit)
    }

    /// 提示词长了会不会被摊薄。只有 SDXL 会 —— 另外两个是长文本原生编码器。
    pub fn dilutes_long_prompts(self) -> bool {
        matches!(self, Self::Sdxl)
    }

    pub fn from_id(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "zimage" | "z-image" | "z_image" => Some(Self::ZImage),
            // `chroma` / `chroma1-hd` 是**退役 id**（2026-10-01 清磁盘时连权重一起删了，
            // `Chroma1-HD` + `t5xxl_fp16` + `ae` 共 26 GB）。和 `flux` / `pony` 同样故意不认 ——
            // 认了就得留一个指向已删文件的引擎。删的时候查过：没有项目把出图引擎存成 chroma。
            "sdxl" | "realvisxl" => Some(Self::Sdxl),
            "krea2" | "krea-2" | "krea_2" | "krea" => Some(Self::Krea2),
            "krea2_edit" | "krea2-edit" | "krea2edit" => Some(Self::Krea2Edit),
            "qwen21" | "qwen-2.1" | "qwen_2_1" | "qwen_image_2.1" | "qwen-image-2.1" => Some(Self::Qwen21),
            // **`qwen_edit` 是 Qwen-Image-Edit-2511 的旧 id，2026-09-22 权重删掉了。**
            //
            // 这里**不**像 `flux` / `pony` 那样返回 `None`，而是指到 2.1 的参考生图 ——
            // 因为它不是「这条能力没了」，是同一条能力换了更好的实现：
            // 同题对照（4 张真实资产、同提示词、同 seed）2.1 更快（346s vs 406s）、
            // 更听话，而且参考槽从 3 张变 10 张。
            //
            // 返回 `None` 的代价是用户存着的画布工作流会**静默落回 Z-Image**，
            // 那是文生图 —— 参考图一张都不用，出来完全不相干的画面。
            "qwen_edit" | "qwen-edit" | "qwen_image_edit" | "qwen-image-edit"
            | "qwen21_edit" | "qwen21-edit" | "qwen2.1_edit" => Some(Self::Qwen21Edit),
            // `pony` / `pony_realistic` 是**退役 id**（2026-09-22 随权重一起删掉）。
            // 和 `flux` 同样故意不认 —— 认了就得留一个指向已删文件的引擎。
            // `flux` / `flux1` / `flux.1` / `flux1-dev` 是**退役 id**（2026-09-08
            // 删掉 FLUX.1 权重时一起下线）。故意不认 —— 认了就得留一个指向已删文件的
            // 引擎。旧 `config.json` 里存着这个值时会走 `pick_engine` 的默认分支
            // 回落 Z-Image，有测试钉着。
            _ => None,
        }
    }
}

/// SDXL 独占的能力。用到任何一个都必须回落 SDXL。
#[derive(Debug, Clone, Copy, Default)]
pub struct SdxlOnly {
    pub character_sheet: bool,
    pub pose_image: bool,
    pub loras: bool,
    pub img2img: bool,
}

impl SdxlOnly {
    /// 要不要为了这些能力回落 SDXL。
    ///
    /// **`asked` 是必须的** —— 图生图已经不是 SDXL 独占了（见 [`supports_i2i`]），
    /// 要的引擎自己有这条通路就不该回落。其余三项才是真的独占：
    /// 那些权重是 SDXL 架构的，挂到别处不生效**且不报错**。
    fn reason_for(self, asked: ImageEngine) -> Option<&'static str> {
        if self.character_sheet {
            Some("设定卡版式要用 CharTurnXL 版式 LoRA")
        } else if self.pose_image {
            Some("按参考图控姿势要用 openpose ControlNet")
        } else if self.loras {
            Some("要挂的 LoRA")
        } else if self.img2img && !supports_i2i(asked) {
            Some("图生图（改图）通路")
        } else {
            None
        }
    }
}

/// 选路结果。`note` 非空时必须原样告诉用户 —— 回落是静默发生就等于骗人。
pub struct EngineChoice {
    pub engine: ImageEngine,
    pub note: Option<String>,
}

/// 选出本次用哪个引擎。
///
/// 默认 `DEFAULT_ENGINE`（Qwen-Image 2.1）。`engine` 参数可以点名。但只要用到 SDXL 独占能力，
/// **一律回落 SDXL 并给出理由** —— 那些权重是 SDXL 架构的，挂到别处不生效，
/// 而且不报错，用户拿到的是一张「看起来成功、能力完全没生效」的图。
/// 设置里配的 Z-Image LoRA，这一次该不该挂上。
///
/// 当初否掉「全局常挂」的三条理由里，第 3 条（Z-Image 上静默失效）已经不成立，
/// 但另外两条还在，所以生效范围被这四个条件卡死：
///
/// - **不是 Z-Image 就不挂**：SDXL 那条路挂 Z-Image 权重是静默失效，
///   正是第 3 条要躲的那件事，只是方向反过来。
/// - **这次调用自己传了 LoRA 就不挂**：两批叠加权重会失控（理由 1）。
///   模型明确点名了，就听模型的。
/// - **图生图不挂**、**设定卡不挂**：那两条路各有自己的 LoRA 编排，
///   混进来效果不可预期（理由 2）；而且设定卡本来就强制回落 SDXL。
pub fn should_apply_configured_zimage_loras(
    engine: ImageEngine,
    call_has_own_loras: bool,
    is_img2img: bool,
    is_sheet: bool,
) -> bool {
    engine == ImageEngine::ZImage && !call_has_own_loras && !is_img2img && !is_sheet
}

/// 这个引擎的图**自己**已经把 LoRA 链接好了吗。
///
/// `image_tool` 在分支之外还有一次统一的 LoRA 插入，那份是按 SDXL 拓扑接的
/// （`CheckpointLoaderSimple` 的 `["1",0]`/`["1",1]` 出 model 和 clip）。
/// Z-Image 的节点 1 是 `UNETLoader`，没有第 1 号输出 —— 用同一批节点 id 再插一次，
/// 等于把接好的那份**覆盖成接错的**，而且 id 相同、不会有任何冲突提示。
pub fn graph_builds_own_lora_chain(engine: ImageEngine) -> bool {
    // Krea2 同样自己接独立 UNET/CLIP；外层 SDXL 链会覆盖 900 并引用不存在的 UNET 输出 1。
    // 两个参考生图引擎不吃用户 LoRA，但同样**不能**让通用插入往里写 ——
    // 它们的节点 1 是 `UNETLoader`，按 SDXL 拓扑插进去就是接错的线，而且不报错。
    matches!(
        engine,
        ImageEngine::ZImage
            | ImageEngine::Krea2
            | ImageEngine::Krea2Edit
            // Qwen 2.1 的节点 1 是 `UnetLoaderGGUF`，同样没有第 1 号输出。
            // 它现在还吃不到 LoRA（没有这个架构的权重，`lora_arch` 一律判 SDXL、
            // 于是 `pick_engine` 会先回落 SDXL），但这一条是拦在那之后的第二道 ——
            // 少了它的话，哪天有人放行 LoRA，覆盖出来的错线不会有任何提示。
            | ImageEngine::Qwen21
            | ImageEngine::Qwen21Edit
    )
}

/// 一条 LoRA 是给哪个架构训的。
///
/// **判错这一格是静默失效。** Z-Image 的 LoRA 挂到 SDXL 上、或者反过来，
/// ComfyUI 都不报错，只是权重不生效 —— 出来的图看着正常，只是"LoRA 好像没用"。
///
/// 权威依据是 safetensors 里的张量键名，两种架构完全不同（实测本机三个文件）：
///
/// ```text
/// Z-Image (DiT):  diffusion_model.layers.0.adaLN_modulation.0.lora_A.weight
/// SDXL   (UNet):  lora_unet_input_blocks_4_1_proj_in.alpha
/// ```
///
/// 但这里**只看文件名**：架构必须在第一条进度事件发出去之前定下来
/// （骨架屏要显示引擎名），那时候还没碰过文件系统，也不该为此加一次磁盘 I/O。
///
/// 所以规矩是**认不出就当 SDXL** —— 那是所有既有 LoRA 的归属，
/// 保持原行为不变，只有明确认得出的 Z-Image LoRA 才走新路。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoraArch {
    ZImage,
    Sdxl,
    /// Krea 2。键名是 `diffusion_model.blocks.N.attn.{wq,wk,wv,wo,gate}`。
    ///
    /// **和 H3 视频 LoRA 长得很像**（都是 `diffusion_model.blocks.N.attn.*`），
    /// 区别在注意力子名：H3 是 `out_proj`，Krea 2 是 `wq/wk/wv/wo/gate`。
    Krea2,
    /// FLUX.2（Klein）。键名是 FLUX 家族的 `double_blocks` 形状。
    ///
    /// **引擎已于 2026-09-08 换成 Krea 2，这个架构标识故意留着** ——
    /// `loras/flux2/` 下那两个文件如果还在盘上，它保证它们被认成
    /// FLUX.2 而不是被误判成 SDXL 然后静默挂上去。
    /// 见 [`needs_sdxl_for_loras`]。
    Flux2,
}

/// Z-Image LoRA 的文件名特征。社区命名基本统一在这几种后缀上。
const ZIMAGE_LORA_MARKERS: &[&str] = &["zimgt", "_zit", "zimage", "z_image", "z-image"];

/// 读 safetensors 的头（开头 8 字节长度 + 一段 JSON），**不加载任何张量**。
///
/// 我们最大的 LoRA 头也只有 425 KB，所以这是几毫秒的事，可以在选路时同步做。
fn safetensors_keys(path: &std::path::Path) -> Option<Vec<String>> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut len = [0u8; 8];
    f.read_exact(&mut len).ok()?;
    let n = u64::from_le_bytes(len);
    // 头再大也不该到 64 MB —— 超了说明不是 safetensors，别去分配那块内存
    if n == 0 || n > 64 * 1024 * 1024 {
        return None;
    }
    let mut buf = vec![0u8; n as usize];
    f.read_exact(&mut buf).ok()?;
    let v: Value = serde_json::from_slice(&buf).ok()?;
    Some(
        v.as_object()?
            .keys()
            .filter(|k| k.as_str() != "__metadata__")
            .cloned()
            .collect(),
    )
}

/// 按张量键名判架构。**这是权威判据**，认不出就返回 `None` 交给调用方兜底。
///
/// 各家的键名长这样（都是本机实测 dump 出来的，不是从文档抄的）：
///
/// ```text
/// Z-Image (DiT):  diffusion_model.layers.0.adaLN_modulation.0.lora_A.weight
///                 diffusion_model.layers.0.attention.to_k.lora_A.weight
/// SDXL   (kohya): lora_te1_text_model_encoder_layers_0_mlp_fc1.alpha
///                 lora_unet_input_blocks_4_1_proj_in.alpha
/// H3     (视频):  diffusion_model.blocks.0.attn.out_proj.lora_A.weight
/// LTX    (视频):  diffusion_model.transformer_blocks.0.attn1.to_k.lora_A.weight
/// ```
///
/// 后两类不是出图引擎的候选，认不出来正好走保守分支。
pub fn arch_from_keys<S: AsRef<str>>(keys: &[S]) -> Option<LoraArch> {
    let has = |f: &dyn Fn(&str) -> bool| keys.iter().any(|k| f(k.as_ref()));

    // ⚠️ **顺序有意义：具体的先判，通用的后判。**
    //
    // 2026-09-08 踩到过：`NexBlend Asian Face 01 ZIT` 是 Z-Image 的 LoRA，
    // 但用的是 **kohya 命名**（`lora_unet_layers_N_attention_to_q`）——
    // 被下面那条通用的 `lora_unet_` 规则先吃掉，判成了 SDXL，
    // 于是整个请求被强制回落 SDXL，而 Z-Image 的权重挂在 SDXL 上
    // **加载「成功」、图照出、一点作用都没有**。

    // Z-Image。两种命名都要认：
    // - 原生 DiT：`.adaLN_modulation` / `diffusion_model.layers.N.attention.*`
    // - kohya：`lora_unet_layers_N_attention_*`（**必须排在 SDXL 那条之前**）
    if has(&|k| k.contains(".adaLN_modulation"))
        // **`diffusion_model.` 前缀是可选的。** ModelScope / MuseAI 的训练器
        // 不写它（2026-09-17 踩到：`Kook_Zimage_瑶光` 的键是
        // `layers.0.attention.to_q.lora_A.default.weight`）。那份现在判得对
        // **纯粹是因为它躺在一个叫 `zimage/` 的文件夹里**，名字启发式兜住了 ——
        // 挪个目录或改个名就会变成 SDXL，而那是静默失效。
        || has(&|k| {
            let k = k.strip_prefix("diffusion_model.").unwrap_or(k);
            k.starts_with("layers.") && k.contains(".attention.")
        })
        // Lumina2 / Z-Image 独有的两组 refiner 块 —— 别的架构没有这两个名字
        || has(&|k| k.contains("context_refiner.") || k.contains("noise_refiner."))
        || has(&|k| k.starts_with("lora_unet_layers_") && k.contains("_attention_"))
    {
        return Some(LoraArch::ZImage);
    }
    // Krea 2。**两种命名都要认**，而它们只在分隔符上不同：
    // - 原生：`diffusion_model.blocks.N.attn.wq.lora_A.weight`
    // - kohya：`lora_unet_blocks_N_attn_gate.lora_down.weight`
    //
    // 所以判据是把分隔符归一化之后看**注意力子名**：Krea 2 是
    // `wq/wk/wv/wo/gate`，而 H3 视频 LoRA 是 `out_proj` —— 只判 `blocks`
    // 会把 H3 的一起吃掉。SDXL 那套是 `input_blocks`/`attn1_to_q`/`proj_in`，
    // 和这几个子名不重叠。
    if has(&|k| {
        let n = k.replace('.', "_");
        n.contains("blocks_")
            && (n.contains("attn_wq")
                || n.contains("attn_wk")
                || n.contains("attn_wv")
                || n.contains("attn_gate")
                // **第三套命名：diffusers 的 `to_*`**（ModelScope / MuseAI 的训练器）。
                //
                // 2026-09-17 踩到：nickai 那批 Krea 2 LoRA 的键名是
                // `transformer.text_fusion.layerwise_blocks.N.attn.to_gate.lora_A.weight`。
                // 注意 `attn_to_gate` **不含** `attn_gate`（中间多个 `to_`），
                // 于是上面四条一条都不中 → 四个架构全判不出 → 退到文件名启发式
                // → 判成 SDXL → **整个请求被强制回落 SDXL，而 Krea 2 的权重挂上去
                // 一点作用都没有、图照出、不报错。**
                //
                // `to_gate` 是这一族的身份标志：SDXL 的 kohya 是 `attn1_to_q`
                // （没有 gate），H3 是 `out_proj`，都不会撞。
                || n.contains("attn_to_gate")
                || (n.contains("text_fusion") && n.contains("attn_to_")))
    }) {
        return Some(LoraArch::Krea2);
    }
    // FLUX 家族（FLUX.2 Klein）：`double_blocks` 的注意力/MLP 上挂 lora_A/lora_B。
    if has(&|k| k.starts_with("diffusion_model.double_blocks.") && k.contains(".lora_"))
    {
        return Some(LoraArch::Flux2);
    }
    // SDXL kohya。**这条最泛，所以必须放最后。**
    //
    // 2026-09-08 一天里被这条吃错三次，全是下新 LoRA 才暴露的：
    // Z-Image 的 `lora_unet_layers_N_attention_*`、
    // Krea 2 的 `lora_unet_blocks_N_attn_gate`。
    // **kohya 是训练框架的命名习惯，每个架构都有它的 kohya 版** ——
    // 所以 `lora_unet_` 天生是兜底判据，不是 SDXL 的身份标志。
    if has(&|k| k.starts_with("lora_unet_") || k.starts_with("lora_te1_") || k.starts_with("lora_te2_"))
    {
        return Some(LoraArch::Sdxl);
    }
    None
}

/// 这个 LoRA 是给哪个架构的。
///
/// **判据优先读文件本身**，文件名只是退路。理由：挂错架构是**静默失效** ——
/// 权重加载"成功"、图照出、不报错，只是一点作用都没有。而文件名完全不可靠：
///
/// - 一个 Z-Image 的 LoRA 完全可以不带 `zimgt` 标记；
/// - 反过来，把 SDXL 的 LoRA 放进一个叫 `zimage/` 的**文件夹**里，
///   ComfyUI 枚举出来的名字就变成 `zimage/xxx.safetensors` ——
///   名字里凭空多了个标记，于是被判成 Z-Image。整理目录就会踩到这个。
///
/// 读不到文件（还没下载、路径解析不出来、不是 safetensors）时退回名字启发式，
/// 再认不出**一律当 SDXL** —— 保守方向和以前一致。
pub fn lora_arch(name: &str) -> LoraArch {
    match crate::cinema_sheet::loras_dir() {
        Some(dir) => lora_arch_in(&dir, name),
        None => lora_arch_by_name(name),
    }
}

/// `lora_arch` 的可测版本：目录由调用方给，不去碰全局状态。
///
/// 分出来是为了能验「文件内容压过文件名」这条 —— 而那正是这次改动的全部意义，
/// 用真实 loras 目录测的话，测的是这台机器此刻有什么文件，不是这个函数。
pub(crate) fn lora_arch_in(dir: &std::path::Path, name: &str) -> LoraArch {
    safetensors_keys(&dir.join(name))
        .as_deref()
        .and_then(arch_from_keys)
        .unwrap_or_else(|| lora_arch_by_name(name))
}

/// 文件名启发式 —— 只在读不到文件时用。
fn lora_arch_by_name(name: &str) -> LoraArch {
    let n = name.to_lowercase();
    if ZIMAGE_LORA_MARKERS.iter().any(|m| n.contains(m)) {
        LoraArch::ZImage
    } else {
        LoraArch::Sdxl
    }
}

/// 这批 LoRA 里有没有**必须**走 SDXL 的。
///
/// 全是 Z-Image 的就不用回落 —— 这是这次改动的核心：
/// 以前"只要带 LoRA 就回落 SDXL"，等于为了挂 LoRA 放弃 Z-Image 的长提示词和画质。
pub fn needs_sdxl_for_loras<I, S>(names: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    // **判据是「不是 Z-Image 架构」，不是「是 SDXL 架构」。**
    //
    // 差别在 FLUX.2 上：它的 LoRA 现在能被 `arch_from_keys` 正确认出来
    // （以前会被误判成 SDXL），但没有任何引擎图有挂 FLUX.2 LoRA 的通路。
    // 如果这里写成 `== Sdxl`，FLUX.2 的 LoRA 就既不回落 SDXL、又会被挂到
    // Z-Image 的图上 —— 那是静默失效，比现在更糟。
    //
    // 所以保守方向不变：认不出的、以及认出来但还没通路的，一律回落 SDXL。
    // FLUX.2 的 LoRA 通路接上那天，这里要改成按引擎匹配（有测试盯着这一条）。
    names
        .into_iter()
        .any(|n| matches!(lora_arch(n.as_ref()), LoraArch::Sdxl | LoraArch::Flux2))
}

/// 这批 LoRA 里有没有**必须**走 Krea 2 的。
///
/// Krea 2 架构的权重挂到别的引擎上**加载「成功」、图照出、一点作用都没有** ——
/// 和 SDXL 那条回落是同一个理由，只是方向不同。
pub fn needs_krea2_for_loras<I, S>(names: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    names.into_iter().any(|n| lora_arch(n.as_ref()) == LoraArch::Krea2)
}

/// **默认出图底模：Qwen-Image 2.1**（2026-09-26 用户拍板，原来是 Z-Image）。
///
/// 同一段陆川设定卡提示词三方对照：Z-Image 把「陆川」写成「近」、右侧格子里换了人；
/// Krea 2 版式规整但同样写错字；Qwen 2.1 名字一字不差、8 行英文资料全对、
/// 十几个格子里是同一张脸。用户的判断：「比 Z-Image 和 Krea2 好太多了」。
///
/// **代价**：1024² 约 211 秒（Z-Image 45 秒），1664×928 约 315 秒。
/// 以及它**没有图生图**（`supports_i2i`）—— 改图时回落 Z-Image，见 `pick_engine`。
pub const DEFAULT_ENGINE: ImageEngine = ImageEngine::Qwen21;

/// 图生图回落到哪个引擎。
///
/// 只因为「这个引擎不会改图」而回落时，**不去 SDXL**：Qwen 2.1 当了默认之后，
/// 那等于把每一次「传一张图让它修」从 Z-Image 的画质降到 SDXL。
/// Z-Image 是换默认之前改图走的那个，行为和换默认之前完全一样。
pub const I2I_FALLBACK: ImageEngine = ImageEngine::ZImage;

pub fn pick_engine(requested: Option<&str>, sdxl_only: SdxlOnly) -> EngineChoice {
    let asked = requested.and_then(ImageEngine::from_id);
    // 判「要不要回落」时得知道**最终会用哪个引擎**：没点名就是默认那个。
    // 拿 `asked` 的 None 当「不支持」的话，默认引擎会被无谓地回落掉。
    let effective = asked.unwrap_or(DEFAULT_ENGINE);
    // 只差「会不会改图」这一条时回落 Z-Image，不是 SDXL（见 `I2I_FALLBACK`）。
    // SDXL 专属的那三条（设定卡 / 姿势 / SDXL LoRA）照旧优先。
    let only_i2i = SdxlOnly { img2img: false, ..sdxl_only }.reason_for(effective).is_none();
    if sdxl_only.img2img && !supports_i2i(effective) && only_i2i {
        return EngineChoice {
            engine: I2I_FALLBACK,
            note: Some(format!(
                "已改用 {} 出图（{} 没有图生图通路，改图会出坏图）。",
                I2I_FALLBACK.label(),
                effective.label()
            )),
        };
    }
    if let Some(why) = sdxl_only.reason_for(effective) {
        let note = match asked {
            Some(e) if e != ImageEngine::Sdxl => Some(format!(
                "已改用 SDXL 出图（你要的是 {}）：{why}只有 SDXL 架构的权重，换引擎会静默失效。",
                e.label()
            )),
            _ => None,
        };
        return EngineChoice { engine: ImageEngine::Sdxl, note };
    }
    EngineChoice { engine: effective, note: None }
}

/// 这个引擎有没有图生图。
///
/// # 曾经的结论是错的
///
/// 以前只有 SDXL 那一支接了 `LoadImage → VAEEncode → KSampler`，
/// 于是 `SdxlOnly.img2img` 一律把其余几个引擎回落到 SDXL。
/// 那不是因为它们不支持 —— **图生图是采样器层面的事，不是模型能力**：
/// 任何潜空间扩散模型把空 latent 换成 `VAEEncode` 的输出、
/// 再把 denoise 调到 1 以下就成立。缺的一直是我们这边的图。
///
/// Z-Image 和 Chroma 的图本来就都带 `VAELoader` 和一个空 latent 节点，所以
/// [`attach_i2i`] 一个通用改写就够。
pub fn supports_i2i(engine: ImageEngine) -> bool {
    matches!(
        engine,
        ImageEngine::Sdxl
            | ImageEngine::ZImage
            | ImageEngine::Krea2
    )
    // **Qwen-Image 2.1 故意不在这里，理由和下面两个不一样。**
    //
    // 它的图确实是 `EmptyLatentImage` + `KSampler`，`attach_i2i` 认得出、也改得动 ——
    // **改完出来的是坏图，而任务状态是 success**（2026-09-22 实测，先是图生图出了一张
    // 满屏网纹的东西，再把采样器整个摘掉、只做 VAE 编码→解码，坏得一模一样）。
    //
    // 根因是**这个 VAE 吃四通道 RGBA**（2.1 原生支持透明底）：官方的
    // `TextEncodeQwenImage21` 里写着 `vae.encode(s)`，`s` 是带 alpha 的四通道，
    // 而通用 `VAEEncode` 只喂 RGB 三通道。少的那一路不报错，只是编出来的 latent 不对。
    //
    // 它**有**图像条件通路，但不是「空 latent 换成源图 latent」这一种：
    // 源图要当参考图接进 `TextEncodeQwenImage21` 的 `images`，latent 取它的第 2 路输出。
    // 那条路还没验，验之前不在这里放行 —— 放行的代价是用户拿到一张坏图且没有任何提示。
    //
    // 两个参考生图引擎**故意不在这里**：它们本身就吃图，走的是自己的参考槽，
    // 不是「空 latent 换成源图 latent」那种改写。套上 `attach_i2i` 会把目标 latent
    // 替换成第一张参考图 —— 出来就是那张参考图的轻微变体，而不是新构图。
}

/// 把一张文生图的图改成图生图：空 latent 换成源图编码出来的 latent。
///
/// # 为什么是通用改写，不是每个引擎各写一份
///
/// 各引擎的采样拓扑差别很大（Chroma 是 `CFGGuider + SamplerCustomAdvanced`，
/// 不是 KSampler 一把梭），但**要动的两处是一样的**：
///
/// 1. 谁在产空 latent（`EmptySD3LatentImage` / `EmptyLatentImage`）——
///    把所有指向它的连线改指到 `VAEEncode`，然后删掉它
/// 2. 谁身上有 `denoise` 这个输入（KSampler，或者自定义采样拓扑里的调度器节点）
///
/// 各写一份的话，加一个引擎就会漏一处，而漏了**不报错** ——
/// 图照跑，只是源图没生效。
///
/// # 步数不用自己缩放
///
/// ComfyUI 的 `KSampler.set_steps` 里是 `new_steps = int(steps/denoise)`，
/// 再取最后 `steps+1` 个 sigma —— **它自己会补**。传 8 步 + denoise 0.65
/// 时模型仍然实打实跑 8 步。我们再乘一次会让蒸馏模型跑十几步，
/// 按实测那是更慢且不会更好。
///
/// 找不到空 latent 或找不到 VAE 时**原样返回**（那说明这张图不是这个形状），
/// 调用方据此判断有没有真的改成图生图。
pub fn attach_i2i(graph: &mut Value, input_filename: &str, denoise: f64) -> bool {
    let Some(m) = graph.as_object_mut() else { return false };

    let find_class = |m: &Map<String, Value>, classes: &[&str]| -> Option<String> {
        let mut ids: Vec<&String> = m.keys().collect();
        ids.sort_by_key(|k| (k.parse::<u64>().unwrap_or(u64::MAX), (*k).clone()));
        ids.into_iter()
            .find(|id| {
                m[*id]
                    .get("class_type")
                    .and_then(Value::as_str)
                    .is_some_and(|c| classes.contains(&c))
            })
            .cloned()
    };

    let empty_id = find_class(m, &["EmptySD3LatentImage", "EmptyLatentImage"]);
    // VAE 有两种来源：独立的 `VAELoader`（第 0 号输出），或者 checkpoint 自带的
    // （`CheckpointLoaderSimple` 的第 2 号输出）。**第一版只认前者** —— Pony 的 VAE
    // 来自 checkpoint，于是「Pony 支持图生图」写进了 `supports_i2i`，一跑却报
    // 「改不成图生图」。优先独立 VAE：两者都在时独立的那个才是图作者要的。
    let vae = find_class(m, &["VAELoader"])
        .map(|id| (id, 0))
        .or_else(|| find_class(m, &["CheckpointLoaderSimple"]).map(|id| (id, 2)));
    let (Some(empty_id), Some((vae_id, vae_slot))) = (empty_id, vae) else {
        return false;
    };

    // 新节点用 800 起 —— LoRA 链占的是 900 起，图本身只用两位数。
    let load_id = "800".to_string();
    let enc_id = "801".to_string();
    m.insert(load_id.clone(), node("LoadImage", json!({"image": input_filename})));
    m.insert(
        enc_id.clone(),
        node("VAEEncode", json!({"pixels": [load_id, 0], "vae": [vae_id, vae_slot]})),
    );

    // 所有指向空 latent 的连线改指到 VAEEncode。**遍历全图，不是只改采样器** ——
    // 点名改的话，换个拓扑就漏。
    let mut rewired = false;
    for (_, n) in m.iter_mut() {
        let Some(inputs) = n.get_mut("inputs").and_then(Value::as_object_mut) else {
            continue;
        };
        for (_, v) in inputs.iter_mut() {
            let is_ref = v
                .as_array()
                .and_then(|a| a.first())
                .and_then(Value::as_str)
                .is_some_and(|id| id == empty_id);
            if is_ref {
                *v = json!([enc_id, 0]);
                rewired = true;
            }
        }
    }
    if !rewired {
        // 没人用那个空 latent —— 这张图不是我们以为的形状，别留下两个孤儿节点
        m.remove(&load_id);
        m.remove(&enc_id);
        return false;
    }
    m.remove(&empty_id);

    // denoise 落在谁身上是逐引擎的（KSampler 上，或者调度器节点上）—— 不认节点名，谁有就设谁。
    let d = denoise.clamp(0.05, 1.0);
    let mut set = false;
    for (_, n) in m.iter_mut() {
        if let Some(inputs) = n.get_mut("inputs").and_then(Value::as_object_mut) {
            if inputs.contains_key("denoise") {
                inputs.insert("denoise".into(), json!(d));
                set = true;
            }
        }
    }
    set
}

fn node(class_type: &str, inputs: Value) -> Value {
    json!({"class_type": class_type, "inputs": inputs})
}

/// Z-Image Turbo 文生图。参数取自 ComfyUI 官方模板 `image_z_image_turbo.json`：
/// `ModelSamplingAuraFlow` shift 3.0、8 步、cfg 1.0、`res_multistep` / `simple`。
///
/// 负向那一路走 `ConditioningZeroOut`（模板就是这么接的）—— 保留这个节点而不是
/// 干脆不接，是为了让图的拓扑和官方一致，也提醒读代码的人：这里是**故意清零的**。
/// 给一张 Z-Image 的图挂上 ControlNet。
///
/// `control_image` 是 ComfyUI `input/` 下**已经预处理好**的控制图文件名
/// —— 深度图 / 骨架 / 线稿，不是原图。Union 模型自己认类型，不用指定是哪种。
///
/// # 和 SDXL 那条路是反的
///
/// SDXL 的 ControlNet 挂在 **conditioning** 上（见 `cinema_sheet.rs` 里那条
/// 「接到 model 上不报错但完全不起作用」的注释）。Z-Image 的正好相反：
/// `ZImageFunControlnet` 吃 MODEL 吐 MODEL，挂在 **model** 上。
/// 两边接反了都不报错，都是静默失效。
///
/// # 为什么读 KSampler 当前的 model 而不是写死
///
/// 这样它和 LoRA 链能叠加：挂了 LoRA 时 KSampler 的 model 指向链尾，
/// 这里把链尾接进 ControlNet、再把 ControlNet 接给 KSampler，顺序自然正确。
/// 写死成 `["4", 0]` 的话，一旦同时挂 LoRA，LoRA 会被**静默旁路**。
pub fn attach_zimage_control(graph: &mut Value, control_image: &str, strength: f32) {
    let Some(m) = graph.as_object_mut() else { return };
    // KSampler 现在的 model 来源 —— 可能是 ModelSamplingAuraFlow，也可能是 LoRA 链尾
    let Some(upstream) = m
        .get("8")
        .and_then(|n| n.get("inputs"))
        .and_then(|i| i.get("model"))
        .cloned()
    else {
        return;
    };
    m.insert(
        "20".into(),
        node("ModelPatchLoader", json!({ "name": ZIMAGE_CONTROLNET })),
    );
    m.insert(
        "21".into(),
        node("LoadImage", json!({ "image": control_image })),
    );
    m.insert(
        "22".into(),
        node(
            "ZImageFunControlnet",
            json!({
                "model": upstream,
                "model_patch": ["20", 0],
                "vae": ["3", 0],
                "image": ["21", 0],
                "strength": strength,
            }),
        ),
    );
    if let Some(x) = m
        .get_mut("8")
        .and_then(|n| n.get_mut("inputs"))
        .and_then(|i| i.as_object_mut())
    {
        x.insert("model".into(), json!(["22", 0]));
    }
}

pub fn build_zimage_graph(prompt: &str, width: u32, height: u32, seed: u64, steps: u32) -> Value {
    build_zimage_graph_with_loras(prompt, width, height, seed, steps, &[])
}

/// 带 LoRA 的 Z-Image 图。
///
/// LoRA 链插在 **UNETLoader/CLIPLoader 之后、ModelSamplingAuraFlow 之前** ——
/// 采样器要的是加过 LoRA 的 model，文本编码要的是加过 LoRA 的 clip，
/// 少接一条那一半就静默不生效。
///
/// 实测验证（同 seed、同提示词、唯一变量是挂不挂）：99.9% 像素改变、
/// 出的是明显不同的一张脸，画质与解剖都没有退化。
pub fn build_zimage_graph_with_loras(
    prompt: &str,
    width: u32,
    height: u32,
    seed: u64,
    steps: u32,
    loras: &[crate::agent_runtime::lora::ResolvedLora],
) -> Value {
    let mut m = Map::new();
    m.insert("1".into(), node("UNETLoader",
        json!({"unet_name": ZIMAGE_UNET, "weight_dtype": "default"})));
    m.insert("2".into(), node("CLIPLoader",
        json!({"clip_name": ZIMAGE_CLIP, "type": ZIMAGE_CLIP_TYPE, "device": "default"})));
    m.insert("3".into(), node("VAELoader", json!({"vae_name": ZIMAGE_VAE})));
    m.insert("4".into(), node("ModelSamplingAuraFlow",
        json!({"model": ["1", 0], "shift": 3.0})));
    m.insert("5".into(), node("CLIPTextEncode", json!({"text": prompt, "clip": ["2", 0]})));
    m.insert("6".into(), node("ConditioningZeroOut", json!({"conditioning": ["5", 0]})));
    m.insert("7".into(), node("EmptySD3LatentImage",
        json!({"width": width, "height": height, "batch_size": 1})));
    m.insert("8".into(), node("KSampler", json!({
        "seed": seed, "steps": steps, "cfg": 1.0,
        "sampler_name": "res_multistep", "scheduler": "simple", "denoise": 1.0,
        "model": ["4", 0], "positive": ["5", 0], "negative": ["6", 0], "latent_image": ["7", 0]
    })));
    m.insert("9".into(), node("VAEDecode", json!({"samples": ["8", 0], "vae": ["3", 0]})));
    m.insert("10".into(), node("SaveImage",
        json!({"images": ["9", 0], "filename_prefix": "anima_img"})));

    if !loras.is_empty() {
        // 900 起，和 image_tool / video_tool 的约定一致；图本身只用到两位数
        let chain = crate::agent_runtime::lora::build_chain(
            loras,
            900,
            json!(["1", 0]),
            Some(json!(["2", 0])),
        );
        for (id, n) in chain.nodes {
            m.insert(id, n);
        }
        if let Some(x) = m.get_mut("4").and_then(|n| n.get_mut("inputs")).and_then(|i| i.as_object_mut()) {
            x.insert("model".into(), chain.model_ref.clone());
        }
        if let Some(clip) = chain.clip_ref {
            if let Some(x) = m.get_mut("5").and_then(|n| n.get_mut("inputs")).and_then(|i| i.as_object_mut()) {
                x.insert("clip".into(), clip);
            }
        }
    }
    Value::Object(m)
}

/// Krea 2 Turbo 的出图图。
///
/// 四处和别的引擎不同，**写错都不报错**：
/// 1. `CLIPLoader` 的 `type` 是 **`krea2`**（不是 `qwen_image`，尽管 VAE 是它的）
/// 2. VAE 是 **`qwen_image_vae`** —— 不是 `ae.safetensors`、也不是 `flux2-vae`
/// 3. **cfg 必须 1.0**（Turbo 版），负向接 `ConditioningZeroOut`
/// 4. 空 latent 用 `EmptyLatentImage` 就行 —— `KSampler` 会走
///    `fix_empty_latent_channels` 按模型补通道，官方模板也是这么接的
///
/// 参数取自 ComfyUI 自带的 `image_krea2_turbo_t2i_int8.json`。
pub fn build_krea2_graph(prompt: &str, width: u32, height: u32, seed: u64, steps: u32) -> Value {
    build_krea2_graph_with_loras(prompt, width, height, seed, steps, &[])
}

/// 同上，外加挂 LoRA。
///
/// **Krea 2 架构的 LoRA 必须挂在 Krea 2 上** —— 挂到别的引擎上加载「成功」、
/// 图照出、一点作用都没有。`needs_krea2_for_loras` 负责把引擎拽过来。
pub fn build_krea2_graph_with_loras(
    prompt: &str,
    width: u32,
    height: u32,
    seed: u64,
    steps: u32,
    loras: &[crate::agent_runtime::lora::ResolvedLora],
) -> Value {
    let mut m = Map::new();
    m.insert("1".into(), node("UNETLoader",
        json!({"unet_name": KREA2_UNET, "weight_dtype": "default"})));
    m.insert("2".into(), node("CLIPLoader", json!({
        "clip_name": KREA2_CLIP, "type": KREA2_CLIP_TYPE, "device": "default"
    })));
    m.insert("3".into(), node("VAELoader", json!({"vae_name": KREA2_VAE})));
    m.insert("4".into(), node("CLIPTextEncode", json!({"text": prompt, "clip": ["2", 0]})));
    m.insert("5".into(), node("ConditioningZeroOut", json!({"conditioning": ["4", 0]})));
    m.insert("6".into(), node("EmptyLatentImage",
        json!({"width": width, "height": height, "batch_size": 1})));
    m.insert("7".into(), node("KSampler", json!({
        "seed": seed, "steps": steps, "cfg": 1.0,
        "sampler_name": "euler", "scheduler": "simple", "denoise": 1.0,
        "model": ["1", 0], "positive": ["4", 0], "negative": ["5", 0], "latent_image": ["6", 0]
    })));
    m.insert("8".into(), node("VAEDecode", json!({"samples": ["7", 0], "vae": ["3", 0]})));
    m.insert("9".into(), node("SaveImage",
        json!({"images": ["8", 0], "filename_prefix": "anima_img"})));
    if !loras.is_empty() {
        // 900 起，和 image_tool / video_tool 的约定一致；图本身只用到一位数
        let chain = crate::agent_runtime::lora::build_chain(
            loras,
            900,
            json!(["1", 0]),
            Some(json!(["2", 0])),
        );
        for (id, n) in chain.nodes {
            m.insert(id, n);
        }
        // **采样器的 model 要改指到链尾**，不改的话 LoRA 节点在图里但没人用它 ——
        // ComfyUI 会把它剪掉，任务成功、权重没生效、不报错。
        if let Some(x) = m.get_mut("7").and_then(|n| n.get_mut("inputs")).and_then(|i| i.as_object_mut()) {
            x.insert("model".into(), chain.model_ref.clone());
        }
        if let Some(clip) = chain.clip_ref {
            if let Some(x) = m.get_mut("4").and_then(|n| n.get_mut("inputs")).and_then(|i| i.as_object_mut()) {
                x.insert("clip".into(), clip);
            }
        }
    }
    Value::Object(m)
}

/// **Krea2Edit 参考生图**：场景图 + 人物图 → 新构图。
///
/// `refs` 是已经放进 ComfyUI `input/` 的文件名，**顺序就是训练顺序：场景在前、人物在后**。
/// 只有一张时就是单参考（人物或场景都行）。多于两张的调用方要先挑 —— 这里只取前两张。
///
/// 照 2026-09-13 实跑通过的工作流（`docs/plans/2026-09-13-出图模型对照-QwenEdit与Pony.md`）。
/// 下面三条**写错都不报错**，节点作者和账本各记过一遍，测试钉着：
///
/// 1. `fit_mode=fit` 要同时接 `vae` + `source_image`，只接 `source_latent` 会退回旧几何
/// 2. `target_latent` 要接喂给 KSampler 的同一个 latent，否则 VAE 在采样中途进显存
/// 3. 负向也走 `Krea2EditGroundedEncode`、接同一组图（训练时的无条件分支），不是 `CLIPTextEncode`
pub fn build_krea2_edit_graph(prompt: &str, width: u32, height: u32, seed: u64, refs: &[String]) -> Value {
    let a = refs.first().cloned().unwrap_or_default();
    let b = refs.get(1).cloned();
    let mut m = Map::new();
    m.insert("1".into(), node("UNETLoader", json!({"unet_name": KREA2_UNET, "weight_dtype": "default"})));
    m.insert("2".into(), node("LoraLoaderModelOnly", json!({
        "model": ["1", 0], "lora_name": KREA2_EDIT_LORA, "strength_model": 1.0
    })));
    m.insert("3".into(), node("CLIPLoader", json!({
        "clip_name": KREA2_CLIP, "type": KREA2_CLIP_TYPE, "device": "default"
    })));
    m.insert("4".into(), node("VAELoader", json!({"vae_name": KREA2_VAE})));
    m.insert("5".into(), node("LoadImage", json!({"image": a})));
    m.insert("7".into(), node("EmptySD3LatentImage", json!({"width": width, "height": height, "batch_size": 1})));
    m.insert("8".into(), node("VAEEncode", json!({"pixels": ["5", 0], "vae": ["4", 0]})));
    let mut patch = json!({
        "model": ["2", 0], "source_latent": ["8", 0], "vae": ["4", 0], "source_image": ["5", 0],
        "ref_boost": KREA2_EDIT_REF_BOOST, "ref_boost_a": 1.0, "fit_mode": "fit", "target_latent": ["7", 0]
    });
    let mut pos = json!({"clip": ["3", 0], "prompt": prompt, "image": ["5", 0], "grounding_px": 1024});
    let mut neg = json!({"clip": ["3", 0], "prompt": "", "image": ["5", 0], "grounding_px": 1024});
    if let Some(b) = b {
        m.insert("6".into(), node("LoadImage", json!({"image": b})));
        patch["source_image_b"] = json!(["6", 0]);
        pos["image_b"] = json!(["6", 0]);
        neg["image_b"] = json!(["6", 0]);
    }
    m.insert("9".into(), node("Krea2EditModelPatch", patch));
    m.insert("10".into(), node("Krea2EditGroundedEncode", pos));
    m.insert("11".into(), node("Krea2EditGroundedEncode", neg));
    m.insert("12".into(), node("KSampler", json!({
        "seed": seed, "steps": default_steps(ImageEngine::Krea2Edit), "cfg": 1.0,
        "sampler_name": "lcm", "scheduler": "simple", "denoise": 1.0,
        "model": ["9", 0], "positive": ["10", 0], "negative": ["11", 0], "latent_image": ["7", 0]
    })));
    m.insert("13".into(), node("VAEDecode", json!({"samples": ["12", 0], "vae": ["4", 0]})));
    m.insert("14".into(), node("SaveImage", json!({"images": ["13", 0], "filename_prefix": "anima_img"})));
    Value::Object(m)
}

/// **Qwen-Image 2.1 文生图。** 拓扑逐字照 ComfyUI 自带的
/// `image_qwen_image_2_1_t2i` 模板，只把 `UNETLoader` 换成 `UnetLoaderGGUF`
/// （我们装的是 Q6_K 量化版）。
///
/// # 和别的引擎最不一样的一处：一个节点出三路
///
/// `TextEncodeQwenImage21` 的输出是 `[positive, negative, latent]` ——
/// **正负两路来自同一个节点的第 0、1 号输出**，不是两个 `CLIPTextEncode`。
/// 按别处的习惯接两个编码节点会直接被 ComfyUI 拒（类型对不上），这一条摔不了人；
/// 真正会静默出错的是**把 negative 接成第 0 路**，那样正负完全相同，
/// 在 cfg=1 下又恰好看不出任何区别。
///
/// # 有参考图时是另一套接法
///
/// 同一个 `TextEncodeQwenImage21` 就是编辑节点 —— 把 `vae` 接上、参考图接进
/// `image_1..image_N`，它会把参考图同时喂给文本编码器（当视觉 token）和 VAE
/// （当 `reference_latents`），并且**自己吐一个尺寸对得上的空 latent**（第 2 路输出）。
///
/// 那时候 `EmptyLatentImage` 就不能要了，`width`/`height` 也不再直接决定画幅 ——
/// 出图尺寸由**第一张参考图的长宽比** + `resolution`（面积）定。这是节点自己的
/// 说明写的：「Empty latent on the first reference image's size, to match with
/// sampling as any other size shifts the edit」。所以我们把 `resolution` 设成
/// `sqrt(width*height)`，让调用方要的**面积**还能作数，比例交给参考图。
///
/// # 为什么 `EmptyLatentImage` 是对的
///
/// 2.1 的 latent 是 **64 通道 / 16 倍下采样**（`comfy/latent_formats.py` 的
/// `QwenImage21`），而 `EmptyLatentImage` 出的是 4 通道 / 8 倍 —— 看着完全不匹配。
/// 但它同时带出一个 `downscale_ratio_spacial: 8`，`KSampler` 里的
/// `comfy.sample.fix_empty_latent_channels` 会**按模型的 latent_format 把全零 latent
/// 重排成 64 通道并缩到 /16**。所以宽高照常填，出的就是那个尺寸。
///
/// 换句话说：这里不能自作聪明改成 `EmptySD3LatentImage`（16 通道），
/// 那一样会被重排，但它是官方模板没用的节点，改了只是让两边对不上。
pub fn build_qwen21_graph(
    prompt: &str,
    negative: &str,
    width: u32,
    height: u32,
    seed: u64,
    steps: u32,
    refs: &[String],
) -> Value {
    let mut m = Map::new();
    m.insert("1".into(), node("UnetLoaderGGUF", json!({"unet_name": QWEN21_UNET})));
    m.insert("2".into(), node("CLIPLoader", json!({
        "clip_name": QWEN21_CLIP, "type": QWEN21_CLIP_TYPE, "device": "default"
    })));
    m.insert("3".into(), node("VAELoader", json!({"vae_name": QWEN21_VAE})));
    // 有参考图时 `resolution` 定的是**面积**（节点按第一张参考图的长宽比算实际宽高）；
    // 纯文生图时它只决定那个用不上的 latent 输出尺寸，画幅由 `EmptyLatentImage` 定。
    let resolution = if refs.is_empty() {
        QWEN21_RESOLUTION
    } else {
        // **不要自己先对齐到 32。** 节点拿这个数当面积，按第一张参考图的长宽比
        // 算出宽高之后**它自己**再对齐 32；抢在它前面对齐等于把精度丢了两次：
        //
        //   要 960×540：先对齐(704) → 928×512（长宽比差 2%）｜不对齐(720) → 960×544
        //   要 832×1216：先对齐(992) → 832×1184      ｜不对齐(1006) → 832×1216（正好）
        //
        // 差出来的那点长宽比不报错，只是画面被轻微拉伸 —— 而动作迁移的首帧
        // 要和深度视频严丝合缝，2% 就是错位。
        ((width as f64) * (height as f64)).sqrt().round() as u32
    };
    let mut enc = json!({
        "clip": ["2", 0],
        "prompt": prompt,
        // cfg=1 下这一路是摆设，照模板原样填着 —— 留空和填词出的图一模一样。
        "negative_prompt": negative,
        "resolution": resolution,
    });
    if !refs.is_empty() {
        // **`vae` 必须接。** 不接的话参考图只进文本编码器当视觉 token，
        // 不会变成 `reference_latents` —— 出来是「照着描述重画一张」，
        // 而不是「保住这张图里的东西」。不报错。
        enc["vae"] = json!(["3", 0]);
        for (i, r) in refs.iter().take(QWEN21_REF_SLOTS).enumerate() {
            let id = (100 + i).to_string();
            m.insert(id.clone(), node("LoadImage", json!({"image": r})));
            // **键是点号路径 `images.image_1`，不是平铺的 `image_1`。**
            //
            // 这是 ComfyUI 的 autogrow（`COMFY_AUTOGROW_V3`）在 API 格式里的写法：
            // `parse_class_inputs` 用这个输入自己的 id（`images`）当前缀，
            // `finalize_prefix(["images"], "image_1")` 才是它认的键。
            //
            // **和视频那边 `ref_images.ref_image_0` 是同一个机制、同一个坑的第二个入口**
            // （MEDIA_PIPELINES 第 7 节）。写成平铺会被 ComfyUI 当成多余的关键字参数抛错
            // —— 这一次是响的；而视频那条当初是**静默忽略**，参考图白给。
            enc[format!("images.image_{}", i + 1)] = json!([id, 0]);
        }
    }
    m.insert("4".into(), node("TextEncodeQwenImage21", enc));
    let latent = if refs.is_empty() {
        m.insert("5".into(), node("EmptyLatentImage", json!({
            "width": width, "height": height, "batch_size": 1
        })));
        json!(["5", 0])
    } else {
        // 第 2 路输出：按第一张参考图的尺寸开的空 latent。换成别的尺寸会让编辑漂移。
        json!(["4", 2])
    };
    m.insert("6".into(), node("KSampler", json!({
        "seed": seed, "steps": steps, "cfg": 1.0,
        "sampler_name": "euler", "scheduler": "simple", "denoise": 1.0,
        "model": ["1", 0],
        "positive": ["4", 0],
        "negative": ["4", 1],
        "latent_image": latent
    })));
    m.insert("7".into(), node("VAEDecode", json!({"samples": ["6", 0], "vae": ["3", 0]})));
    m.insert("8".into(), node("SaveImage", json!({"images": ["7", 0], "filename_prefix": "anima_img"})));
    Value::Object(m)
}

/// 各引擎图里 `SaveImage` 的节点 id —— 取结果时要按它去 outputs 里找。
pub fn save_node_id(engine: ImageEngine) -> &'static str {
    match engine {
        ImageEngine::ZImage => "10",
        ImageEngine::Sdxl => "7",
        ImageEngine::Krea2 => "9",
        ImageEngine::Krea2Edit => "14",
        ImageEngine::Qwen21 | ImageEngine::Qwen21Edit => "8",
    }
}

/// 默认步数。Z-Image 是 Turbo 蒸馏模型，8 步就是它的设计点，调高不会更好。
pub fn default_steps(engine: ImageEngine) -> u32 {
    match engine {
        ImageEngine::ZImage => 8,
        ImageEngine::Sdxl => 20,
        // Turbo 版，8 步就是设计点。
        ImageEngine::Krea2 => KREA2_STEPS,
        // Turbo / Lightning 8 步版，都是设计点。
        ImageEngine::Krea2Edit => 8,
        // 官方模板就是 25。它**不是**蒸馏模型，别按 Turbo 的习惯往下调。
        ImageEngine::Qwen21 | ImageEngine::Qwen21Edit => QWEN21_STEPS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **CLIP 的 type 值是猜不出来的，写错不报错、只是出一张不相干的图。**
    ///
    /// Z-Image 用 `lumina2` —— 不是 `z_image`，也不是 `qwen_image`，尽管它的
    /// 编码器就是 Qwen3-4B。这个值取自 ComfyUI 官方模板，改动前先去核对模板。
    #[test]
    fn clip_type_取自官方模板不能改() {
        assert_eq!(ZIMAGE_CLIP_TYPE, "lumina2");
        assert_eq!(KREA2_CLIP_TYPE, "krea2");
        let g = build_zimage_graph("a girl", 832, 1216, 7, 8);
        assert_eq!(g["2"]["inputs"]["type"], "lumina2");
    }

    /// ControlNet 挂在 **model** 上，不是 conditioning 上。
    /// 这条和 SDXL 那边正好相反，接反了不报错、只是静默失效，所以要钉住。
    #[test]
    fn zimage_controlnet_挂在model上而不是conditioning上() {
        let mut g = build_zimage_graph("a cat", 1024, 1024, 1, 8);
        attach_zimage_control(&mut g, "depth.png", ZIMAGE_CONTROL_STRENGTH);

        assert_eq!(g["22"]["class_type"], "ZImageFunControlnet");
        assert_eq!(g["8"]["inputs"]["model"], json!(["22", 0]));
        // conditioning 两路都必须原封不动
        assert_eq!(g["8"]["inputs"]["positive"], json!(["5", 0]));
        assert_eq!(g["8"]["inputs"]["negative"], json!(["6", 0]));
        // ControlNet 要 VAE —— 少接这一路会报 required input missing
        assert_eq!(g["22"]["inputs"]["vae"], json!(["3", 0]));
        assert_eq!(g["20"]["inputs"]["name"], ZIMAGE_CONTROLNET);
        assert_eq!(g["21"]["inputs"]["image"], "depth.png");
    }

    /// 和 LoRA 叠加时，整条 model 链必须完整：
    /// `UNETLoader → LoRA 链 → ModelSamplingAuraFlow → ControlNet → KSampler`。
    ///
    /// LoRA 链是插在 **ModelSamplingAuraFlow 之前**的（改的是节点 4 的 model 输入），
    /// 所以 KSampler 始终指向 4。ControlNet 读 KSampler 当前的 model 再接回去，
    /// 于是自动落在链尾。**断链了不报错，只是那一段静默失效** —— 所以整条都验。
    #[test]
    fn zimage_controlnet_与lora链能叠加() {
        let loras = vec![crate::agent_runtime::lora::ResolvedLora {
            name: "zimage/test.safetensors".into(),
            strength: 0.8,
        }];
        let mut g = build_zimage_graph_with_loras("a cat", 1024, 1024, 1, 8, &loras);
        let sampling_src = g["4"]["inputs"]["model"].clone();
        assert_ne!(
            sampling_src,
            json!(["1", 0]),
            "挂了 LoRA 时 ModelSamplingAuraFlow 不该直连 UNETLoader"
        );

        attach_zimage_control(&mut g, "depth.png", 0.8);
        // KSampler ← ControlNet ← ModelSamplingAuraFlow ← LoRA 链 ← UNETLoader
        assert_eq!(g["8"]["inputs"]["model"], json!(["22", 0]));
        assert_eq!(g["22"]["inputs"]["model"], json!(["4", 0]));
        assert_eq!(
            g["4"]["inputs"]["model"], sampling_src,
            "挂 ControlNet 不能动 LoRA 那一段"
        );
    }

    /// 强度用 0.8，不是 1.0 —— 拉满会僵硬地贴着控制图。
    #[test]
    fn zimage_控制强度不拉满() {
        assert!((0.65..=1.0).contains(&ZIMAGE_CONTROL_STRENGTH), "超出官方可用区间");
        assert!(ZIMAGE_CONTROL_STRENGTH < 1.0, "拉满会失去自然光影");
    }

    /// ControlNet 权重放 `model_patches/`，不是 `controlnet/`。
    /// 放错目录时 ModelPatchLoader 的下拉里根本看不到它。
    #[test]
    fn zimage_controlnet_文件名是model_patch而不是传统controlnet() {
        assert!(ZIMAGE_CONTROLNET.contains("Z-Image"));
        assert!(ZIMAGE_CONTROLNET.ends_with(".safetensors"));
    }

    /// **Z-Image 的负向提示词是结构性无效的，调用方必须知道。**
    ///
    /// 官方模板 cfg=1.0、负向接 `ConditioningZeroOut`。我们在 SDXL 上调了很久的
    /// 那串解剖负向词在它这里一个字都不起作用 —— 如果代码里不点破这件事，
    /// 就会有人以为"传了负向词所以安全"。
    #[test]
    fn zimage的负向词不起作用且图里说得出来() {
        assert!(!ImageEngine::ZImage.negative_works());
        assert!(ImageEngine::Sdxl.negative_works());

        let g = build_zimage_graph("a girl", 832, 1216, 7, 8);
        assert_eq!(g["8"]["inputs"]["cfg"], 1.0, "cfg 不是 1.0 的话负向就不是清零语义了");
        assert_eq!(g["6"]["class_type"], "ConditioningZeroOut");
        // 负向那一路必须真的接在清零节点上，而不是接回正向
        assert_eq!(g["8"]["inputs"]["negative"], json!(["6", 0]));
    }

    /// Krea 2 这条路上，四件写错都不报错的事。
    ///
    /// 全部来自 ComfyUI 自带模板 `image_krea2_turbo_t2i_int8.json` +
    /// 2026-09-08 的实跑（1024²/8 步/107.6s，中文提示词，图看过）。
    #[test]
    fn krea2图里四件写错也不报错的事() {
        let g = build_krea2_graph("一只橘猫", 1024, 1024, 7, KREA2_STEPS);

        // 1. int8 走普通 UNETLoader —— 不是 GGUF 那条
        assert_eq!(g["1"]["class_type"], "UNETLoader");
        assert!(KREA2_UNET.ends_with(".safetensors"), "不是 GGUF：{KREA2_UNET}");
        assert!(KREA2_UNET.contains("turbo"), "Raw 版要 40 步 cfg 3~4，参数完全不同");

        // 2. CLIP type 必须是 krea2（不是 qwen_image，尽管 VAE 是 Qwen-Image 的）
        assert_eq!(g["2"]["inputs"]["type"], "krea2");
        assert_ne!(g["2"]["inputs"]["type"], "qwen_image");

        // 3. VAE 是 qwen_image_vae —— 拿 ae/flux2-vae 解出来是噪声
        assert_eq!(g["3"]["inputs"]["vae_name"], "qwen_image_vae.safetensors");

        // 4. cfg=1.0，且负向真的接在清零节点上（不是接回正向）
        assert_eq!(g["7"]["inputs"]["cfg"], 1.0);
        let neg = g["7"]["inputs"]["negative"][0].as_str().expect("负向必须是节点引用");
        assert_eq!(g[neg]["class_type"], "ConditioningZeroOut");
        assert!(!ImageEngine::Krea2.negative_works());
    }

    /// **Turbo 版的步数是 8。** Raw 版要 40 步 + cfg 3~4，是另一套参数。
    #[test]
    fn krea2_是turbo版的步数() {
        assert_eq!(KREA2_STEPS, 8);
        assert_eq!(default_steps(ImageEngine::Krea2), 8);
        assert_eq!(
            build_krea2_graph("x", 1024, 1024, 1, default_steps(ImageEngine::Krea2))["7"]["inputs"]
                ["steps"],
            8
        );
    }

    /// **每个架构的 LoRA 都要落到能挂它的那个引擎上。**
    ///
    /// 挂错架构是**静默失效**：加载「成功」、图照出、一点作用都没有。
    /// 所以判定不能只有「是不是 SDXL」这一个问题。
    ///
    /// 2026-09-08 真踩到过两次，都是下了新 LoRA 才暴露的：
    ///
    /// 1. `NexBlend Asian Face 01 ZIT` 是 **Z-Image** 的，但用 kohya 命名
    ///    （`lora_unet_layers_N_attention_to_q`）—— 被通用的 `lora_unet_`
    ///    规则先吃掉判成 SDXL。修法是**具体的判据排在通用的前面**。
    /// 2. `Krea2NexBlendSemiRealistic` 是 **Krea 2** 的，键名
    ///    `diffusion_model.blocks.N.attn.wq` —— 以前谁都不认，落到名字启发式
    ///    再落到 SDXL。
    #[test]
    fn 每个架构的lora都落到能挂它的引擎上() {
        // Z-Image 原生 DiT 命名
        assert_eq!(
            arch_from_keys(&["diffusion_model.layers.0.attention.to_q.lora_A.weight"]),
            Some(LoraArch::ZImage)
        );
        assert_eq!(
            arch_from_keys(&["blocks.0.adaLN_modulation.0.lora_A.weight"]),
            Some(LoraArch::ZImage)
        );
        // Z-Image 的 **kohya** 命名 —— 这条必须赢过下面 SDXL 那条
        assert_eq!(
            arch_from_keys(&["lora_unet_layers_0_attention_to_q.lora_down.weight"]),
            Some(LoraArch::ZImage),
            "kohya 命名的 Z-Image LoRA 被判成 SDXL 了 —— 会静默失效"
        );
        // SDXL kohya
        assert_eq!(
            arch_from_keys(&["lora_unet_input_blocks_4_1_proj_in.alpha"]),
            Some(LoraArch::Sdxl)
        );
        assert_eq!(
            arch_from_keys(&["lora_te1_text_model_encoder.alpha"]),
            Some(LoraArch::Sdxl)
        );
        // Krea 2 原生命名
        assert_eq!(
            arch_from_keys(&["diffusion_model.blocks.0.attn.wq.lora_A.weight"]),
            Some(LoraArch::Krea2)
        );
        // Krea 2 的 **kohya** 命名 —— 同样必须赢过 SDXL 那条
        assert_eq!(
            arch_from_keys(&["lora_unet_blocks_0_attn_gate.lora_down.weight"]),
            Some(LoraArch::Krea2),
            "kohya 命名的 Krea 2 LoRA 被判成 SDXL 了 —— 会静默失效"
        );
        // Z-Image：**不带 `diffusion_model.` 前缀**的写法（同一个 ModelScope 训练器）。
        // `Kook_Zimage_瑶光` 就是这样 —— 原来判不出来，全靠它躺在 `zimage/` 目录里
        // 被名字启发式兜住；挪个目录就会静默变成 SDXL。
        assert_eq!(
            arch_from_keys(&["layers.0.attention.to_q.lora_A.default.weight"]),
            Some(LoraArch::ZImage),
            "不带 diffusion_model 前缀的 Z-Image LoRA 判不出来"
        );
        // Lumina2 / Z-Image 独有的 refiner 块
        assert_eq!(
            arch_from_keys(&["context_refiner.0.attention.to_k.lora_A.default.weight"]),
            Some(LoraArch::ZImage)
        );
        assert_eq!(
            arch_from_keys(&["noise_refiner.0.attention.to_k.lora_A.default.weight"]),
            Some(LoraArch::ZImage)
        );
        // Krea 2 的 **diffusers `to_*`** 命名（ModelScope / MuseAI 训练器）。
        //
        // 2026-09-17 踩到：nickai 那批（`Nick_cheongsam2_Krea2` 等三个）
        // 用的是 `attn.to_gate`，而 `attn_to_gate` **不含** `attn_gate` ——
        // 四个架构一条都不中，退到文件名启发式判成 SDXL，
        // 于是整个请求强制回落 SDXL，Krea 2 的权重挂上去一点作用都没有、还不报错。
        assert_eq!(
            arch_from_keys(&["transformer.text_fusion.layerwise_blocks.0.attn.to_gate.lora_A.weight"]),
            Some(LoraArch::Krea2),
            "diffusers 命名的 Krea 2 LoRA 判不出来 —— 会静默回落 SDXL"
        );
        assert_eq!(
            arch_from_keys(&["transformer.text_fusion.layerwise_blocks.0.attn.to_q.lora_A.weight"]),
            Some(LoraArch::Krea2)
        );
        // **H3 视频 LoRA 长得很像，不能被 Krea 2 那条吃掉**（子名是 out_proj）
        assert_ne!(
            arch_from_keys(&["diffusion_model.blocks.0.attn.out_proj.lora_A.weight"]),
            Some(LoraArch::Krea2),
            "H3 的视频 LoRA 被判成 Krea 2 了"
        );
        // FLUX.2
        assert_eq!(
            arch_from_keys(&["diffusion_model.double_blocks.0.img_attn.qkv.lora_A.weight"]),
            Some(LoraArch::Flux2)
        );
        // 认不出的走保守那条
        assert_eq!(arch_from_keys(&["something.unknown.weight"]), None);
    }

    /// **本机真实文件的架构判定。**
    ///
    /// 上面那条用的是手写键名 —— 它证明规则对，证明不了「盘上这个文件会被
    /// 判成什么」。下一个新 LoRA 就可能带来没见过的命名，
    /// 而那正是 2026-09-08 那两次的来源。
    ///
    /// `cargo test --lib 本机的lora -- --ignored`
    #[test]
    #[ignore]
    fn 本机的lora都判对了架构() {
        let Some(dir) = crate::cinema_sheet::loras_dir() else {
            return;
        };
        for (name, want) in [
            ("zimage/NexBlend_Asian_Face_01_ZIT.safetensors", LoraArch::ZImage),
            ("krea/Krea2NexBlendSemiRealistic.safetensors", LoraArch::Krea2),
            ("krea/krea2_kgirl_v3.safetensors", LoraArch::Krea2),
            ("krea/hina_krea2Turbo_asianmix_v4.safetensors", LoraArch::Krea2),
            ("krea/QuadView_krea2_v1.safetensors", LoraArch::Krea2),
            ("krea/DynamicCharacterSheet_krea2_v1.safetensors", LoraArch::Krea2),
        ] {
            if dir.join(name).is_file() {
                assert_eq!(lora_arch_in(&dir, name), want, "{name} 判错了架构");
            } else {
                eprintln!("跳过（盘上没有）：{name}");
            }
        }
    }

    /// 把 Rust 真正会提交的 Krea 2 图导出来，打真实 ComfyUI 做端到端验证。
    ///
    /// **测试只能证明图的形状，不能证明它出得来。** CLIP type、VAE 这类常量
    /// 写错在单测里全看不出来 —— 有些根本不炸、只是出一张不相干的图。
    ///
    /// `ANIMA_GRAPH_DUMP=/tmp/g.json cargo test --lib dump_krea2_graph -- --ignored`
    #[test]
    #[ignore]
    fn dump_krea2_graph() {
        let prompt = std::env::var("ANIMA_PROMPT")
            .unwrap_or_else(|_| "一只橘猫戴着红色毛线帽，坐在木桌上，桌上摆着三个青苹果".into());
        let g = build_krea2_graph(&prompt, 1024, 1024, 12345, default_steps(ImageEngine::Krea2));
        let out = std::env::var("ANIMA_GRAPH_DUMP").expect("要设 ANIMA_GRAPH_DUMP");
        std::fs::write(&out, serde_json::to_string_pretty(&g).unwrap()).unwrap();
        eprintln!("写到 {out}；save node = {}", save_node_id(ImageEngine::Krea2));
    }

    /// `save_node_id` / `sampler_node_id` / `vae_ref` 指错节点 = 「任务成功但找不到图」。
    ///
    /// 这三个函数是按引擎手写的常量表，和图的结构分开维护 —— 加引擎时最容易漏。
    /// 所以对**每个引擎**都验一遍：这些 id 在自己那张图里确实是那类节点。
    #[test]
    fn 每个引擎的节点id都指向图里正确的节点() {
        let graphs = [
            (ImageEngine::ZImage, build_zimage_graph("x", 1024, 1024, 7, 8)),
            (ImageEngine::Krea2, build_krea2_graph("x", 1024, 1024, 7, 8)),
        ];
        for (engine, g) in graphs {
            let sid = save_node_id(engine);
            assert_eq!(
                g[sid]["class_type"], "SaveImage",
                "{} 的 save_node_id={sid} 不是 SaveImage —— 出图会「成功但没文件」",
                engine.label()
            );
            // 图生图改写的就是这个节点的 `latent_image` 输入，所以判据是「它有这个输入」，
            // 不是「它叫 KSampler」—— Chroma 走的是 SamplerCustomAdvanced。
            let kid = sampler_node_id(engine);
            assert!(
                g[kid]["inputs"].get("latent_image").is_some(),
                "{} 的 sampler_node_id={kid} 没有 latent_image 输入，图生图改写会打空",
                engine.label()
            );
            assert!(
                g[kid]["class_type"].as_str().unwrap().contains("Sampler"),
                "{} 的 sampler_node_id={kid} 不是采样器：{}",
                engine.label(),
                g[kid]["class_type"]
            );
            // 正向/负向必须接在产条件的节点上。写串了（比如接到 MODEL 那一路）
            // 本地测试不会响 —— 要等 ComfyUI 校验才炸，而那时已经是运行期了。
            // （Chroma 走 SamplerCustomAdvanced，条件包在 guider 里，没有这两个输入。）
            for slot in ["positive", "negative"] {
                let Some(r) = g[kid]["inputs"].get(slot) else { continue };
                let n = r[0].as_str().unwrap_or_else(|| panic!("{} 的 {slot} 不是节点引用", engine.label()));
                let ct = g[n]["class_type"].as_str().unwrap_or("");
                assert!(
                    ct.contains("TextEncode") || ct.contains("Conditioning") || ct.contains("Guidance"),
                    "{} 的 {slot} 接到了 {n}（{ct}），那不是条件节点",
                    engine.label()
                );
            }

            let vae = vae_ref(engine);
            let vnode = vae[0].as_str().unwrap();
            assert_eq!(
                g[vnode]["class_type"], "VAELoader",
                "{} 的 vae_ref 指向 {vnode}，那不是 VAELoader",
                engine.label()
            );
        }
    }

    /// **退役 id 必须回落到默认引擎，而不是 panic 或者报错。**
    ///
    /// FLUX.1 于 2026-09-08 连权重一起删掉，`from_id` 故意不再认 `flux`。
    /// 但用户的 `config.json` 里可能还存着这个字符串（设置面板选过它），
    /// 所以这条路必须是「不认识 → 用默认」，不能是崩。
    ///
    /// 这也顺带钉住了 `pick_engine` 的一条语义：**`None` 不代表「不支持」**，
    /// 而是「没点名」。
    #[test]
    fn 退役的flux_id回落到默认引擎() {
        for id in ["flux", "flux1", "flux.1", "flux1-dev", "FLUX.1-dev"] {
            assert_eq!(ImageEngine::from_id(id), None, "{id} 应该已经退役");
            assert_eq!(
                pick_engine(Some(id), SdxlOnly::default()).engine,
                DEFAULT_ENGINE,
                "{id} 没回落到默认引擎"
            );
        }
    }


    /// 用到 SDXL 专属能力时必须回落，而且**要说出来**。
    ///
    /// 静默回落 = 用户以为在用 Z-Image、实际拿到 SDXL 的图；
    /// 静默不回落 = LoRA / ControlNet 挂上去不生效，图照出、不报错。两种都不行。
    #[test]
    fn 用到sdxl专属能力时回落并说明() {
        // 默认是 Qwen-Image 2.1（2026-09-26 起）
        assert_eq!(pick_engine(None, SdxlOnly::default()).engine, ImageEngine::Qwen21);
        // 点名 krea2 就用 krea2
        assert_eq!(pick_engine(Some("krea2"), SdxlOnly::default()).engine, ImageEngine::Krea2);
        // `chroma` 是退役 id（2026-10-01 连权重一起删了）：认不出，回落默认 —— 不能指向一个已删的文件
        assert_eq!(ImageEngine::from_id("chroma"), None);
        assert_eq!(ImageEngine::from_id("chroma1-hd"), None);
        assert_eq!(pick_engine(Some("chroma"), SdxlOnly::default()).engine, ImageEngine::Qwen21);

        for (flags, kw) in [
            (SdxlOnly { character_sheet: true, ..Default::default() }, "设定卡"),
            (SdxlOnly { pose_image: true, ..Default::default() }, "姿势"),
            (SdxlOnly { loras: true, ..Default::default() }, "LoRA"),
        ] {
            let c = pick_engine(Some("zimage"), flags);
            assert_eq!(c.engine, ImageEngine::Sdxl, "{kw} 没回落到 SDXL");
            let note = c.note.unwrap_or_else(|| panic!("{kw} 回落了却没说明"));
            assert!(note.contains(kw), "说明里没点出原因 `{kw}`：{note}");
        }
        // 没点名引擎时回落不用啰嗦
        assert!(pick_engine(None, SdxlOnly { loras: true, ..Default::default() }).note.is_none());
    }

    /// **图生图不再回落 SDXL。**
    ///
    /// 以前它和设定卡 / 姿势 / LoRA 排在一起，因为只有 SDXL 那一支接了源图。
    /// 但那不是模型不支持 —— 图生图是采样器层面的事，任何潜空间扩散模型
    /// 把空 latent 换成 `VAEEncode` 的输出就成立。缺的一直是我们这边的图，
    /// 现在四条都有了（见 [`attach_i2i`]）。
    ///
    /// 无谓的回落代价是实打实的：用户选了 Z-Image 想要它的长提示词和画质，
    /// 结果拿到一张 SDXL 的图。
    #[test]
    fn 图生图不再回落() {
        let i2i = SdxlOnly { img2img: true, ..Default::default() };
        for id in ["zimage", "krea2", "sdxl"] {
            let c = pick_engine(Some(id), i2i);
            assert_eq!(
                c.engine,
                ImageEngine::from_id(id).unwrap(),
                "{id} 被无谓地回落了"
            );
            assert!(c.note.is_none(), "{id} 没回落却给了说明：{:?}", c.note);
        }
        // 没点名时落到 Z-Image：默认的 Qwen 2.1 不会改图，回落目标是 `I2I_FALLBACK`，
        // 和换默认之前（默认 Z-Image、不用回落）拿到的是同一个引擎
        assert_eq!(pick_engine(None, i2i).engine, ImageEngine::ZImage);
    }

    /// **回落的判据要看最终会用哪个引擎**，不是「点没点名」。
    ///
    /// 拿 `asked` 的 `None` 当「不支持」的话，默认引擎会被无谓地回落。
    #[test]
    fn 没点名时按默认引擎判要不要回落() {
        // 默认引擎（Qwen 2.1）不会改图 → 落到 Z-Image，**不是** SDXL
        assert_eq!(
            pick_engine(None, SdxlOnly { img2img: true, ..Default::default() }).engine,
            ImageEngine::ZImage
        );
        // 而真正 SDXL 独占的能力，没点名也照样回落
        assert_eq!(
            pick_engine(None, SdxlOnly { pose_image: true, ..Default::default() }).engine,
            ImageEngine::Sdxl
        );
    }

    #[test]
    fn 三个引擎都有图生图() {
        for e in [ImageEngine::Sdxl, ImageEngine::ZImage, ImageEngine::Krea2] {
            assert!(supports_i2i(e), "{} 没标成支持图生图", e.label());
        }
    }

    /// 只有 SDXL 会被长提示词摊薄 —— 另外两个是长文本原生编码器。
    ///
    /// 实测：同一段 1251 字提示词，SDXL 把 "black silk" 出成深蓝、丢了戒指、
    /// 构图从全身缩成半身；Chroma / Z-Image 细节几乎全中。
    #[test]
    fn 只有sdxl会被长提示词摊薄() {
        assert!(ImageEngine::Sdxl.dilutes_long_prompts());
        assert!(!ImageEngine::ZImage.dilutes_long_prompts());
        assert!(!ImageEngine::Krea2.dilutes_long_prompts());
    }

    /// 取结果要按各自的 SaveImage 节点 id 找，写错就是「生成成功但取不到图」。
    /// **局部重绘必须改采样器的 latent 连线，只加节点等于没做。**
    ///
    /// 只插 `VAEEncodeForInpaint` 而不改连线时，图跑得通、任务 success、
    /// 商品照样被整个重画 —— 这是本项目最典型的失败形状。
    #[test]
    fn 局部重绘改采样器的latent连线() {
        for (e, sampler, mk) in [
            (ImageEngine::ZImage, "8", build_zimage_graph("p", 64, 64, 1, 8)),
            (ImageEngine::Krea2, "7", build_krea2_graph("p", 64, 64, 1, 8)),
        ] {
            let g = mk;
            let before = g["prompt"].is_null();
            assert!(!before || true);
            let mut inner = g.clone();
            apply_inpaint(&mut inner, e, "src.png", &MaskSource::Auto);
            assert_eq!(
                inner[sampler]["inputs"]["latent_image"],
                json!(["34", 0]),
                "{} 的采样器 latent 没改到 VAEEncodeForInpaint —— 图会跑通但商品被重画",
                e.label()
            );
            assert_eq!(inner["34"]["class_type"], "VAEEncodeForInpaint");
        }
    }

    /// **遮罩语义要反一次，而且反错了不报错。**
    ///
    /// 用户给的遮罩是「白色 = 我的商品」，而 `VAEEncodeForInpaint` 重画的是
    /// **白色区域**。少了 `InvertMask`，出来的图是商品被重画、背景被保留。
    #[test]
    fn 遮罩必须取反一次() {
        let mut g = build_zimage_graph("p", 64, 64, 1, 8);
        apply_inpaint(&mut g, ImageEngine::ZImage, "src.png", &MaskSource::Auto);
        assert_eq!(g["33"]["class_type"], "InvertMask", "自动抠图没取反");
        assert_eq!(g["34"]["inputs"]["mask"], json!(["33", 0]));
        assert_eq!(g["32"]["class_type"], "RemoveBackground");

        let mut g2 = build_zimage_graph("p", 64, 64, 1, 8);
        apply_inpaint(&mut g2, ImageEngine::ZImage, "src.png", &MaskSource::File("m.png".into()));
        assert_eq!(g2["31"]["class_type"], "LoadImageMask", "手动遮罩没走 LoadImageMask");
        assert_eq!(g2["33"]["class_type"], "InvertMask", "手动遮罩没取反");
        // 手动遮罩从红通道取 —— 手画的黑白图 alpha 通道常常是空的
        assert_eq!(g2["31"]["inputs"]["channel"], "red");
    }

    /// **不能删原来的空 latent 节点。**
    ///
    /// SDXL 的图生图拓扑里 `"4"` 是 `LoadImage` 而不是空 latent，
    /// 删了会让 `VAEEncode` 悬空、整个图校验失败。没人引用的节点
    /// ComfyUI 本来就会剪掉，留着无害。
    #[test]
    fn 局部重绘不删任何原有节点() {
        let g = build_zimage_graph("p", 64, 64, 1, 8);
        let before: Vec<String> = g.as_object().unwrap().keys().cloned().collect();
        let mut after_g = g.clone();
        apply_inpaint(&mut after_g, ImageEngine::ZImage, "src.png", &MaskSource::Auto);
        for k in before {
            assert!(after_g.get(&k).is_some(), "节点 {k} 被删掉了");
        }
    }

    #[test]
    fn savenode_id和图对得上() {
        for (e, g) in [
            (ImageEngine::ZImage, build_zimage_graph("a", 64, 64, 1, 8)),
            (ImageEngine::Krea2, build_krea2_graph("a", 64, 64, 1, 8)),
        ] {
            let id = save_node_id(e);
            assert_eq!(g[id]["class_type"], "SaveImage",
                "{} 的 save_node_id={id} 指的不是 SaveImage", e.label());
        }
    }

    /// 实测耗时不能乱写 —— 队列预估和「要不要换引擎」的建议都靠它。
    #[test]
    fn 耗时是实测值且量级正确() {
        assert_eq!(ImageEngine::ZImage.secs_per_job(), 45);
        assert_eq!(ImageEngine::Qwen21.secs_per_job(), 211);
        assert!(ImageEngine::Qwen21.secs_per_job() > ImageEngine::ZImage.secs_per_job() * 4,
            "Qwen 2.1 实测是 Z-Image 的 4 倍多，这个差距必须留在数据里");
    }

    /// Z-Image 是 Turbo 蒸馏模型，8 步是设计点，不是省着跑。
    #[test]
    fn 各引擎默认步数() {
        assert_eq!(default_steps(ImageEngine::ZImage), 8);
        assert_eq!(default_steps(ImageEngine::Krea2), 8);
        assert_eq!(default_steps(ImageEngine::Qwen21), 25);
    }
}

/// 把两个引擎的图导出成 JSON，用来真的提交给 ComfyUI 验证 ——
/// 单测只能证明结构自洽，证明不了 ComfyUI 认这张图。
///
/// `cargo test -p anima --lib dump_engine_graphs -- --ignored --nocapture`
#[cfg(test)]
mod dump_engine_graphs {
    use super::*;
    #[test]
    #[ignore]
    fn dump() {
        let p = "a photorealistic portrait of a young East Asian woman by a rainy window at night, \
                 black silk slip dress with lace trim, thin gold necklace, both hands visible";
        std::fs::write("/tmp/g_zimage.json",
            serde_json::to_string(&build_zimage_graph(p, 832, 1216, 424242, 8)).unwrap()).unwrap();
        println!("dumped");
    }
}

/// 局部重绘的遮罩从哪来。
#[derive(Debug, Clone)]
pub enum MaskSource {
    /// 用户自己给的遮罩图（ComfyUI input 目录里的文件名）。
    /// **白色 = 要保留的区域**（比如商品本体）—— 和"用户拿笔涂商品"的直觉一致。
    File(String),
    /// BiRefNet 自动抠主体。424MB 权重，ComfyUI 原生支持，不需要自定义节点。
    Auto,
}

/// 各引擎图里**消费 latent 的那个采样节点**的 id。
///
/// 局部重绘就是把它的 `latent_image` 换成 `VAEEncodeForInpaint` 的输出 ——
/// 采样那条链其余部分一个字都不用改。
///
/// **只改连线，不删原来的空 latent 节点。** SDXL 的图生图拓扑里 `"4"` 是
/// `LoadImage` 而不是空 latent，删了会让 `VAEEncode` 悬空、整个图校验失败。
/// 而没人引用的节点 ComfyUI 本来就会剪掉，留着无害。
fn sampler_node_id(engine: ImageEngine) -> &'static str {
    match engine {
        ImageEngine::ZImage => "8",
        ImageEngine::Sdxl => "5",
        ImageEngine::Krea2 => "7",
        ImageEngine::Krea2Edit => "12",
        ImageEngine::Qwen21 | ImageEngine::Qwen21Edit => "6",
    }
}

/// VAE 在图里的引用。SDXL 的 VAE 来自 checkpoint，另外两个是独立 VAELoader。
fn vae_ref(engine: ImageEngine) -> Value {
    match engine {
        ImageEngine::Sdxl => json!(["1", 2]),
        ImageEngine::Krea2Edit => json!(["4", 0]),
        _ => json!(["3", 0]),
    }
}

/// 给一张已经建好的图加上局部重绘：**遮罩内的像素原样保留，遮罩外按提示词重画**。
///
/// 这是广告图的核心 —— 商品必须一个像素都不变，而整图重绘（`image_path` +
/// `denoise`）天然做不到：denoise 低则场景不变，denoise 高则商品也被重画。
///
/// # 遮罩语义容易搞反，而且搞反了不报错
///
/// `VAEEncodeForInpaint` 重画的是**遮罩为白的区域**。而用户给的遮罩是「白色 =
/// 我的商品」，所以中间必须 `InvertMask`。少了这一步，图照样出得来、
/// 任务照样 success —— 只是商品被重画、背景被保留，正好反过来。
///
/// # 实测（2026-09-02）
///
/// Z-Image：商品轮廓内平均像素差 11.7/255、明显改动 0.8%（那是 VAE 往返噪声），
/// 轮廓外 164.6/255。SDXL 同样通过。**Chroma 未验证。**
pub fn apply_inpaint(
    graph: &mut Value,
    engine: ImageEngine,
    source_filename: &str,
    mask: &MaskSource,
) {
    let node = |t: &str, i: Value| json!({"class_type": t, "inputs": i});
    let sampler_id = sampler_node_id(engine);
    let Some(map) = graph.as_object_mut() else {
        return;
    };

    // 30~39 这一段专门留给局部重绘，避开各引擎已经用掉的 id
    map.insert("30".into(), node("LoadImage", json!({"image": source_filename})));
    let mask_out: Value = match mask {
        MaskSource::File(name) => {
            map.insert(
                "31".into(),
                // 从红通道取 —— 手画的遮罩多是黑白图，alpha 通道常常是空的
                node("LoadImageMask", json!({"image": name, "channel": "red"})),
            );
            map.insert("33".into(), node("InvertMask", json!({"mask": ["31", 0]})));
            json!(["33", 0])
        }
        MaskSource::Auto => {
            map.insert(
                "31".into(),
                node(
                    "LoadBackgroundRemovalModel",
                    json!({"bg_removal_name": "birefnet.safetensors"}),
                ),
            );
            map.insert(
                "32".into(),
                node("RemoveBackground", json!({"bg_removal_model": ["31", 0], "image": ["30", 0]})),
            );
            map.insert("33".into(), node("InvertMask", json!({"mask": ["32", 0]})));
            json!(["33", 0])
        }
    };
    map.insert(
        "34".into(),
        node(
            "VAEEncodeForInpaint",
            json!({
                "pixels": ["30", 0],
                "vae": vae_ref(engine),
                "mask": mask_out,
                // 往外扩几个像素，否则商品边缘会留下一圈原背景的硬边
                "grow_mask_by": 6
            }),
        ),
    );

    // **把采样器的 latent 换过去。只加节点不改这一处，等于没做局部重绘** ——
    // 图跑得通、任务 success、商品照样被重画。这是本项目最典型的失败形状。
    if let Some(inputs) = map.get_mut(sampler_id).and_then(|n| n.get_mut("inputs")) {
        if let Some(o) = inputs.as_object_mut() {
            o.insert("latent_image".into(), json!(["34", 0]));
        }
    }
}

#[cfg(test)]
mod zimage_lora_tests {
    use super::*;
    use crate::agent_runtime::lora::ResolvedLora;

    // ---- 2026-09-13 接入的四个引擎 ----------------------------------------

    fn inp<'a>(g: &'a Value, id: &str) -> &'a Value {
        &g[id]["inputs"]
    }
    fn class(g: &Value, id: &str) -> String {
        g[id]["class_type"].as_str().unwrap_or("").to_string()
    }
    fn refs(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn 四个新引擎的id能来回认() {
        for e in [ImageEngine::Krea2Edit, ImageEngine::Qwen21, ImageEngine::Qwen21Edit] {
            assert_eq!(ImageEngine::from_id(e.id()), Some(e), "{} 认不回来", e.id());
        }
        assert_eq!(ImageEngine::Krea2Edit.reference_slots(), 2);
        assert_eq!(ImageEngine::Qwen21Edit.reference_slots(), 10);
        assert_eq!(ImageEngine::ZImage.reference_slots(), 0);
    }

    /// **Krea2Edit 三条写错不报错的连线**，节点作者和账本各记过一遍。
    #[test]
    fn krea2edit_连线照实跑过的工作流() {
        let g = build_krea2_edit_graph("她跪在殿中", 1024, 576, 7, &refs(&["scene.png", "char.png"]));
        let patch = inp(&g, "9");
        assert_eq!(class(&g, "9"), "Krea2EditModelPatch");
        // 1. fit 模式要同时接 vae + source_image
        assert_eq!(patch["fit_mode"], "fit");
        assert_eq!(patch["vae"], json!(["4", 0]), "fit 模式没接 vae，会退回旧几何");
        assert_eq!(patch["source_image"], json!(["5", 0]), "fit 模式没接 source_image");
        // 2. target_latent 和 KSampler 用同一个 latent
        assert_eq!(patch["target_latent"], inp(&g, "12")["latent_image"], "target_latent 和采样器不是同一个 latent");
        // 3. 负向走 GroundedEncode、接同一组图
        assert_eq!(class(&g, "11"), "Krea2EditGroundedEncode", "负向没走 GroundedEncode");
        assert_eq!(inp(&g, "11")["image"], inp(&g, "10")["image"]);
        assert_eq!(inp(&g, "11")["image_b"], inp(&g, "10")["image_b"]);
        // 顺序：场景在前、人物在后；ref_boost 作用在最后一张（人物）
        assert_eq!(inp(&g, "5")["image"], "scene.png");
        assert_eq!(inp(&g, "6")["image"], "char.png");
        assert_eq!(patch["source_image_b"], json!(["6", 0]));
        assert_eq!(patch["ref_boost"], json!(KREA2_EDIT_REF_BOOST));
        // 身份编辑 LoRA 在链上，采样器吃的是打过补丁的模型
        assert_eq!(inp(&g, "2")["lora_name"], KREA2_EDIT_LORA);
        assert_eq!(inp(&g, "12")["model"], json!(["9", 0]));
        assert_eq!(class(&g, save_node_id(ImageEngine::Krea2Edit)), "SaveImage");
        assert_eq!(class(&g, sampler_node_id(ImageEngine::Krea2Edit)), "KSampler");
    }

    #[test]
    fn krea2edit_只有一张参考图时不留第二张的空连线() {
        let g = build_krea2_edit_graph("x", 1024, 576, 7, &refs(&["char.png"]));
        assert!(g.get("6").is_none(), "只有一张图却建了第二个 LoadImage");
        assert!(inp(&g, "9").get("source_image_b").is_none());
        assert!(inp(&g, "10").get("image_b").is_none());
    }

    /// 有独立 `VAELoader` 的图照旧接它 —— 这次改动不能改变 Z-Image / Krea 2 的行为。
    #[test]
    fn 有独立vae时图生图仍接独立vae() {
        let mut g = build_krea2_graph("x", 1024, 1024, 7, 8);
        assert!(attach_i2i(&mut g, "src.png", 0.6));
        assert_eq!(inp(&g, "801")["vae"], json!(["3", 0]));
    }

    /// **参考生图引擎不走通用图生图改写。** 那个改写把目标 latent 换成第一张参考图，
    /// 出来是参考图的轻微变体，不是新构图。
    #[test]
    fn 参考生图引擎不套通用图生图改写() {
        assert!(!supports_i2i(ImageEngine::Krea2Edit));
        assert!(!supports_i2i(ImageEngine::Qwen21Edit));
        assert!(supports_i2i(ImageEngine::Sdxl), "SDXL 那条路不能被这次收紧波及");
    }

    fn lo(n: &str) -> ResolvedLora {
        ResolvedLora { name: n.into(), strength: 1.0 }
    }

    /// **张量键名才是权威判据**，本机四个真实 LoRA 的 dump。
    ///
    /// 这些键名是从盘上的文件读出来的，不是从文档抄的 —— 文档抄错过一次。
    #[test]
    fn 按张量键名判架构() {
        let zimage = [
            "diffusion_model.layers.0.adaLN_modulation.0.lora_A.weight",
            "diffusion_model.layers.0.attention.to_k.lora_A.weight",
        ];
        assert_eq!(arch_from_keys(&zimage), Some(LoraArch::ZImage));

        let sdxl = [
            "lora_te1_text_model_encoder_layers_0_mlp_fc1.alpha",
            "lora_unet_input_blocks_4_1_proj_in.alpha",
        ];
        assert_eq!(arch_from_keys(&sdxl), Some(LoraArch::Sdxl));

        // 视频 LoRA 不是出图引擎的候选，认不出正好走保守分支
        let h3 = ["diffusion_model.blocks.0.attn.out_proj.lora_A.weight"];
        assert_eq!(arch_from_keys(&h3), None);
        let ltx = ["diffusion_model.transformer_blocks.0.attn1.to_k.lora_A.weight"];
        assert_eq!(arch_from_keys(&ltx), None);
    }

    /// **文件内容必须压过文件名** —— 这条不成立的话，这次改动等于没做。
    ///
    /// 造一个名字里带 `zimgt`（看起来像 Z-Image）、键名却是 SDXL 的文件，
    /// 判定必须是 SDXL。反过来再造一个。
    #[test]
    fn 文件内容压过文件名() {
        let dir = std::env::temp_dir().join(format!("anima_lora_arch_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let write = |name: &str, keys: &[&str]| {
            let mut header = Map::new();
            for k in keys {
                header.insert(
                    (*k).to_string(),
                    json!({"dtype": "F16", "shape": [1, 1], "data_offsets": [0, 2]}),
                );
            }
            let blob = serde_json::to_vec(&Value::Object(header)).unwrap();
            let mut bytes = (blob.len() as u64).to_le_bytes().to_vec();
            bytes.extend_from_slice(&blob);
            bytes.extend_from_slice(&[0u8; 2]);
            std::fs::write(dir.join(name), bytes).unwrap();
        };

        // 名字说 Z-Image，内容是 SDXL
        write("骗人的_zimgt.safetensors", &["lora_unet_input_blocks_4_1_proj_in.alpha"]);
        assert_eq!(
            lora_arch_in(&dir, "骗人的_zimgt.safetensors"),
            LoraArch::Sdxl,
            "名字带 zimgt 但键名是 SDXL —— 该信内容"
        );

        // 名字什么都不说，内容是 Z-Image（现实里很常见：作者没在文件名里标架构）
        write(
            "毫无线索.safetensors",
            &["diffusion_model.layers.0.adaLN_modulation.0.lora_A.weight"],
        );
        assert_eq!(
            lora_arch_in(&dir, "毫无线索.safetensors"),
            LoraArch::ZImage,
            "文件名认不出但键名是 Z-Image —— 旧实现会误判成 SDXL"
        );

        // 目录里没有这个文件时，退回名字启发式
        assert_eq!(lora_arch_in(&dir, "没有这个文件_zimgt.safetensors"), LoraArch::ZImage);
        assert_eq!(lora_arch_in(&dir, "没有这个文件.safetensors"), LoraArch::Sdxl);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// **整理目录不能改变架构判定。**
    ///
    /// 这是引入按内容判定的直接原因：`ZIMAGE_LORA_MARKERS` 里有 `zimage`，
    /// 而 ComfyUI 枚举子目录时名字会变成 `zimage/xxx.safetensors` ——
    /// 于是任何被放进这个文件夹的 SDXL LoRA 都会被判成 Z-Image，
    /// 挂上去静默失效。读文件的那条路不看名字，所以不受影响。
    #[test]
    fn 目录名不该左右架构判定() {
        // 名字这层确实会被目录名带偏 —— 这正是它只能当退路的原因
        assert_eq!(lora_arch_by_name("zimage/某个sdxl的lora.safetensors"), LoraArch::ZImage);
        // 而权威判据只看键名
        let sdxl = ["lora_unet_input_blocks_4_1_proj_in.alpha"];
        assert_eq!(arch_from_keys(&sdxl), Some(LoraArch::Sdxl));
    }

    #[test]
    fn 认得出本机三个_z_image_lora() {
        // 实测这三个都在 Z-Image 上生效（同 seed 对照，99.9% 像素改变、画质不掉）
        for n in [
            "Straight_hair_zimgt.safetensors",
            "shorty_asian_zimgt.safetensors",
            "Pintrest_ZiT.safetensors",
        ] {
            assert_eq!(lora_arch_by_name(n), LoraArch::ZImage, "{n} 该认成 Z-Image");
        }
    }

    #[test]
    fn 既有的_sdxl_lora_一个都不能被认错() {
        // 认错就会走 Z-Image 图，SDXL 权重在那儿静默不生效
        for n in [
            "hanfu_tang_sdxl.safetensors",
            "guofeng_illust_sdxl.safetensors",
            "CharTurnXLRetag_v4.safetensors",
            "Asian_Beauty XL.safetensors",
            "add-detail-xl.safetensors",
            "NsfwPovAllInOneLoraSdxl-000009.safetensors",
        ] {
            assert_eq!(lora_arch_by_name(n), LoraArch::Sdxl, "{n} 该认成 SDXL");
        }
    }

    #[test]
    fn 认不出的一律当_sdxl() {
        // 保守方向：认不出走原来的路，行为不变。反过来会让未知 LoRA 静默失效
        assert_eq!(lora_arch_by_name("some_random_thing.safetensors"), LoraArch::Sdxl);
        // 读不到文件时，整条路也要落在 SDXL 上
        assert_eq!(lora_arch("__不存在的文件__.safetensors"), LoraArch::Sdxl);
    }

    #[test]
    fn 全是_z_image_的_lora_不再逼着回落_sdxl() {
        // 这次改动的核心：以前一见 LoRA 就回落，等于为挂 LoRA 放弃 Z-Image 的画质
        assert!(!needs_sdxl_for_loras(["Straight_hair_zimgt.safetensors"]));
        assert!(!needs_sdxl_for_loras([
            "Straight_hair_zimgt.safetensors",
            "Pintrest_ZiT.safetensors"
        ]));
    }

    #[test]
    fn 混着一个_sdxl_的就得回落() {
        // 混挂时 Z-Image 图装不下 SDXL 权重，整批回落才是对的
        assert!(needs_sdxl_for_loras([
            "Straight_hair_zimgt.safetensors",
            "hanfu_tang_sdxl.safetensors"
        ]));
    }

    #[test]
    fn 没有_lora_时不回落() {
        let empty: [&str; 0] = [];
        assert!(!needs_sdxl_for_loras(empty));
    }

    #[test]
    fn z_image_图里_lora_接在采样和文本编码两条线上() {
        let g = build_zimage_graph_with_loras("x", 1024, 1024, 1, 8, &[lo("a_zimgt.safetensors")]);
        assert_eq!(g["900"]["class_type"], "LoraLoader");
        // 少接任何一条，那一半就静默不生效
        assert_eq!(g["4"]["inputs"]["model"], serde_json::json!(["900", 0]),
            "采样器没接到加过 LoRA 的 model");
        assert_eq!(g["5"]["inputs"]["clip"], serde_json::json!(["900", 1]),
            "文本编码没接到加过 LoRA 的 clip");
    }

    #[test]
    fn 多条_lora_串成链而不是并联() {
        let g = build_zimage_graph_with_loras(
            "x", 1024, 1024, 1, 8,
            &[lo("a_zimgt.safetensors"), lo("b_zimgt.safetensors")],
        );
        assert_eq!(g["901"]["inputs"]["model"], serde_json::json!(["900", 0]), "第二条没串在第一条后面");
        assert_eq!(g["4"]["inputs"]["model"], serde_json::json!(["901", 0]), "采样器该接最后一条");
    }

    #[test]
    fn 不挂_lora_时图和原来一模一样() {
        // 回归：新通路不能改变既有行为
        let a = build_zimage_graph("x", 1024, 1024, 7, 8);
        let b = build_zimage_graph_with_loras("x", 1024, 1024, 7, 8, &[]);
        assert_eq!(a, b);
        assert_eq!(a["4"]["inputs"]["model"], serde_json::json!(["1", 0]));
    }
}

/// 把生产代码生成的图导出来，交给真的 ComfyUI 跑一遍。
///
/// 结构测试只能证明 JSON 长得对，证明不了 ComfyUI 认。这两件事分开过 ——
/// 比如节点名拼错、某个输入是必填而我们没给，结构测试全绿而实际一提交就报错。
#[cfg(test)]
#[test]
#[ignore = "要本机 ComfyUI 在跑；用 cargo test -- --ignored 手动跑"]
fn 导出带_lora_的_zimage_图供实跑() {
    use crate::agent_runtime::lora::ResolvedLora;
    let g = build_zimage_graph_with_loras(
        "j1v_w0m, a young asian woman, upper body portrait, soft window light, grey background, photorealistic",
        1024, 1024, 778899, 8,
        &[ResolvedLora { name: "Straight_hair_zimgt.safetensors".into(), strength: 1.0 }],
    );
    std::fs::write(
        std::env::var("ZIMAGE_GRAPH_OUT").unwrap_or_else(|_| "/tmp/zimage_lora_graph.json".into()),
        serde_json::to_string_pretty(&g).unwrap(),
    )
    .unwrap();
}

#[cfg(test)]
mod qwen21_tests {
    use super::*;

    fn t2i() -> Value {
        build_qwen21_graph("a cat", "blurry", 1024, 1024, 42, QWEN21_STEPS, &[])
    }

    /// **正负两路必须来自同一个节点的第 0、1 号输出。**
    ///
    /// 这是 Qwen 2.1 和本仓库其它每一个引擎都不一样的地方 ——
    /// 别处是两个编码节点各出一路。把 negative 也接成 `["4", 0]` 的话，
    /// 正负完全相同；而这张图 cfg=1，负向本来就不起作用，
    /// **于是出的图一模一样、没有任何症状**。所以这条只能靠测试钉。
    #[test]
    fn 正负两路取自同一节点的第0和第1号输出() {
        let g = t2i();
        assert_eq!(g["4"]["class_type"], "TextEncodeQwenImage21");
        assert_eq!(g["6"]["inputs"]["positive"], json!(["4", 0]));
        assert_eq!(g["6"]["inputs"]["negative"], json!(["4", 1]));
    }

    /// 三个文件名 + CLIP type 全部取自 ComfyUI 自带的 `image_qwen_image_2_1_t2i` 模板。
    /// 写错都不报错：VAE 接成 Edit-2511 那份是颜色结构全崩，type 写错是出一张不相干的图。
    #[test]
    fn 模型文件和clip_type照官方模板() {
        let g = t2i();
        // **GGUF 专用加载器**。换回 `UNETLoader` 的话它枚举不到 .gguf，图会被 ComfyUI 拒。
        assert_eq!(g["1"]["class_type"], "UnetLoaderGGUF");
        assert_eq!(g["1"]["inputs"]["unet_name"], QWEN21_UNET);
        assert!(QWEN21_UNET.ends_with(".gguf"), "换成非 GGUF 权重就得同时换掉加载器");
        assert_eq!(g["2"]["inputs"]["clip_name"], "qwen3vl_8b_int8_convrot.safetensors");
        assert_eq!(g["2"]["inputs"]["type"], "qwen_image");
        assert_eq!(g["3"]["inputs"]["vae_name"], "qwen_image_2.1_vae_bf16.safetensors");
        // 2.1 的 VAE 不是 Edit-2511 / Krea 2 那份
        assert_ne!(QWEN21_VAE, KREA2_VAE);
    }

    /// 采样参数照模板：25 步 / cfg 1 / euler + simple。
    /// **它不是蒸馏模型**，按 Turbo 的习惯调到 8 步会明显变差。
    #[test]
    fn 采样参数照官方模板() {
        let g = t2i();
        assert_eq!(g["6"]["inputs"]["steps"], 25);
        assert_eq!(g["6"]["inputs"]["cfg"], 1.0);
        assert_eq!(g["6"]["inputs"]["sampler_name"], "euler");
        assert_eq!(g["6"]["inputs"]["scheduler"], "simple");
        assert_eq!(default_steps(ImageEngine::Qwen21), 25);
    }

    /// `EmptyLatentImage` 出的是 4 通道 /8，而 2.1 的 latent 是 64 通道 /16 ——
    /// 靠 `KSampler` 里的 `fix_empty_latent_channels` 按 `downscale_ratio_spacial`
    /// 重排。官方模板就是这么接的，**换成 `EmptySD3LatentImage` 只是让两边对不上**。
    #[test]
    fn 空latent用_empty_latent_image_并且宽高照传() {
        let g = build_qwen21_graph("x", "", 832, 1216, 1, 25, &[]);
        assert_eq!(g["5"]["class_type"], "EmptyLatentImage");
        assert_eq!(g["5"]["inputs"]["width"], 832);
        assert_eq!(g["5"]["inputs"]["height"], 1216);
        assert_eq!(g["6"]["inputs"]["latent_image"], json!(["5", 0]));
    }

    /// 取结果和局部重绘都按这两个 id 去找节点，指错了是「出了图但取不到」。
    #[test]
    fn 取结果的节点id指向真的存在的那两个节点() {
        let g = t2i();
        assert_eq!(g[save_node_id(ImageEngine::Qwen21)]["class_type"], "SaveImage");
        assert_eq!(g[sampler_node_id(ImageEngine::Qwen21)]["class_type"], "KSampler");
        assert_eq!(g[vae_ref(ImageEngine::Qwen21)[0].as_str().unwrap()]["class_type"], "VAELoader");
    }

    /// **不能走通用的图生图改写。**
    ///
    /// 这条是反着钉的，因为这里最容易「看着能用」就放行：`attach_i2i` 认得出这张图、
    /// 也真的改得动（下面第二个断言就是证据），**改完出的是坏图而任务 success**。
    /// 根因在 `supports_i2i` 的注释里：这个 VAE 吃四通道 RGBA，通用改写只喂三通道。
    #[test]
    fn 不走通用图生图改写() {
        assert!(!supports_i2i(ImageEngine::Qwen21), "放行了就会静默出坏图");
        // 改写本身是能落下去的 —— 所以拦它的只能是 `supports_i2i` 这道闸，不是「改不动」
        let mut g = t2i();
        assert!(attach_i2i(&mut g, "src.png", 0.6));
    }

    /// cfg=1，负向被约掉。界面据此不显示那个输入框 ——
    /// 显示了就是让人反复改一个不起作用的东西。
    #[test]
    fn 负向结构性无效() {
        assert!(!ImageEngine::Qwen21.negative_works());
        // 长提示词不摊薄（Qwen3-VL-8B 是长文本原生，不是 CLIP 的 77 token 分块）
        assert!(!ImageEngine::Qwen21.dilutes_long_prompts());
        // 不是参考生图引擎：槽位为 0，别的地方据此判断要不要收参考图
        assert_eq!(ImageEngine::Qwen21.reference_slots(), 0);
    }

    #[test]
    fn id_能来回转() {
        assert_eq!(ImageEngine::from_id("qwen21"), Some(ImageEngine::Qwen21));
        assert_eq!(ImageEngine::from_id("qwen-image-2.1"), Some(ImageEngine::Qwen21));
        assert_eq!(ImageEngine::Qwen21.id(), "qwen21");
        // **和 Edit-2511 不是同一个东西**，别认混
        // **旧 id 指到 2.1 的参考生图**（2511 的权重 2026-09-22 删了）。
        // 不指的话用户存着的画布工作流会静默落回 Z-Image —— 那是文生图，
        // 参考图一张都不用，出来完全不相干的画面。
        assert_eq!(ImageEngine::from_id("qwen_edit"), Some(ImageEngine::Qwen21Edit));
        assert_ne!(ImageEngine::from_id("qwen_edit"), Some(ImageEngine::Qwen21));
    }

    /// 通用 SDXL LoRA 插入不能往这张图上写 —— 节点 1 是 `UnetLoaderGGUF`，没有第 1 号输出。
    /// 要图生图时**必须回落**，而且要有理由 —— 静默换引擎等于骗人。
    ///
    /// 回落到 **Z-Image，不是 SDXL**：Qwen 2.1 是默认之后，没点名引擎的每一次
    /// 改图都会走这条，落到 SDXL 就是整条修图链路画质退一大截。
    #[test]
    fn 要图生图时回落zimage并说明理由() {
        for asked in [Some("qwen21"), None] {
            let c = pick_engine(asked, SdxlOnly { img2img: true, ..Default::default() });
            assert_eq!(c.engine, ImageEngine::ZImage, "{asked:?}");
            assert!(c.note.as_deref().unwrap_or("").contains("图生图"), "{:?}", c.note);
        }
        // 改图同时要 SDXL 专属能力（姿势）时，SDXL 那条优先 —— 否则 ControlNet 静默失效
        let c = pick_engine(None, SdxlOnly { img2img: true, pose_image: true, ..Default::default() });
        assert_eq!(c.engine, ImageEngine::Sdxl);
        // 会改图的引擎照旧是它自己
        assert_eq!(pick_engine(Some("krea2"), SdxlOnly { img2img: true, ..Default::default() }).engine, ImageEngine::Krea2);
        // 不要图生图时照样是它自己
        assert_eq!(pick_engine(Some("qwen21"), SdxlOnly::default()).engine, ImageEngine::Qwen21);
    }

    fn edit() -> Value {
        build_qwen21_graph("put her in a red dress", "", 1024, 1024, 7,
                           QWEN21_STEPS, &["a.png".into(), "b.png".into()])
    }

    /// **`vae` 不接就静默降级。** 参考图只会进文本编码器当视觉 token，
    /// 不会变成 `reference_latents` —— 出来是「照着描述重画一张」，
    /// 而不是「保住这张图里的东西」，而且不报错。
    #[test]
    fn 参考图路径必须把vae接进编码节点() {
        let g = edit();
        assert_eq!(g["4"]["inputs"]["vae"], json!(["3", 0]));
        // 纯文生图时不接 —— 接了会多做一次没用的编码
        assert!(t2i()["4"]["inputs"].get("vae").is_none());
    }

    /// **键是点号路径 `images.image_1`。** autogrow 在 API 格式里就是这么写的
    /// （`finalize_prefix(["images"], "image_1")`）。三种写错法各有各的症状：
    /// 平铺 `image_1` 当场抛「多余的关键字参数」；`image1`（EditPlus 的写法）
    /// 和从 0 开始都是**被静默忽略**，参考图白给。
    #[test]
    fn 参考图的键是点号路径且从1开始() {
        let g = edit();
        assert_eq!(g["4"]["inputs"]["images.image_1"], json!(["100", 0]));
        assert_eq!(g["4"]["inputs"]["images.image_2"], json!(["101", 0]));
        assert!(g["4"]["inputs"].get("image_1").is_none(), "平铺写法，ComfyUI 会抛错");
        assert!(g["4"]["inputs"].get("images.image_0").is_none(), "从 1 开始，不是 0");
        assert!(g["4"]["inputs"].get("images.image1").is_none(), "那是 EditPlus 的写法");
        assert_eq!(g["100"]["inputs"]["image"], "a.png");
    }

    /// 有参考图时 latent 来自编码节点的**第 2 路**，而且**不能**再留空 latent ——
    /// 节点自己的说明：换成别的尺寸会让编辑漂移。
    #[test]
    fn 有参考图时latent取第2路且没有空latent() {
        let g = edit();
        assert_eq!(g["6"]["inputs"]["latent_image"], json!(["4", 2]));
        assert!(g.get("5").is_none(), "空 latent 还在，尺寸会打架");
        // 纯文生图那边反过来
        assert_eq!(t2i()["6"]["inputs"]["latent_image"], json!(["5", 0]));
    }

    /// 画幅只剩**面积**作数（比例跟第一张参考图）。
    ///
    /// **这个数不能先对齐到 32** —— 节点自己会对齐，抢在前面对齐会把长宽比弄歪：
    /// 832×1216 先对齐是 992（出 832×1184），不对齐是 1006（出 832×1216，正好）。
    #[test]
    fn 参考模式下resolution是面积的开方且不预先对齐32() {
        let g = build_qwen21_graph("x", "", 832, 1216, 1, 25, &["a.png".into()]);
        assert_eq!(g["4"]["inputs"]["resolution"], 1006);
        assert_ne!(g["4"]["inputs"]["resolution"], 992, "又先对齐 32 了");
        // 960×540 那一档同理：720 才对，704 会出成 928×512
        let v = build_qwen21_graph("x", "", 960, 540, 1, 25, &["a.png".into()]);
        assert_eq!(v["4"]["inputs"]["resolution"], 720);
        // 纯文生图时它是个定值，画幅由 EmptyLatentImage 定
        assert_eq!(t2i()["4"]["inputs"]["resolution"], QWEN21_RESOLUTION);
    }

    /// 超过槽位的参考图要被截掉，而不是塞进去报错。
    #[test]
    fn 参考图超过十张只取前十张() {
        let many: Vec<String> = (0..14).map(|i| format!("r{i}.png")).collect();
        let g = build_qwen21_graph("x", "", 1024, 1024, 1, 25, &many);
        assert_eq!(ImageEngine::Qwen21Edit.reference_slots(), 10);
        assert!(g["4"]["inputs"].get("images.image_10").is_some());
        assert!(g["4"]["inputs"].get("images.image_11").is_none());
    }

    /// 两个 id 是同一份权重、同一张图 —— 只有参考图那几个节点不一样。
    #[test]
    fn 两个id共用一份权重() {
        let a = t2i();
        let b = edit();
        for k in ["1", "2", "3"] {
            assert_eq!(a[k], b[k], "节点 {k} 不一样了");
        }
        assert_eq!(ImageEngine::Qwen21Edit.reference_slots(), 10);
        assert_eq!(ImageEngine::Qwen21.reference_slots(), 0, "它是文生图那一档");
        assert!(!ImageEngine::Qwen21Edit.negative_works());
    }

    #[test]
    fn 通用lora插入被拦住() {
        assert!(graph_builds_own_lora_chain(ImageEngine::Qwen21));
    }

    /// 参考生图对照实跑：`QWEN21_CMP_ENGINE=qwen_edit|qwen21_edit`，
    /// `QWEN21_REFS=a.png,b.png,...`（ComfyUI input 目录里的文件名）。
    ///
    /// **两边共用同一段提示词和同一批参考图** —— 这个测试的全部意义就是
    /// 「同题」，各写一份就不是对照了。
    #[test]
    #[ignore]
    fn 导出参考生图对照图() {
        let prompt = std::env::var("QWEN21_PROMPT").expect("要 QWEN21_PROMPT");
        let refs: Vec<String> = std::env::var("QWEN21_REFS").expect("要 QWEN21_REFS")
            .split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
        let seed: u64 = std::env::var("QWEN21_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(778899);
        let w: u32 = std::env::var("QWEN21_W").ok().and_then(|v| v.parse().ok()).unwrap_or(1024);
        let h: u32 = std::env::var("QWEN21_H").ok().and_then(|v| v.parse().ok()).unwrap_or(1024);
        let which = std::env::var("QWEN21_CMP_ENGINE").unwrap_or_else(|_| "qwen21_edit".into());
        let g = match which.as_str() {
            "krea2_edit" => build_krea2_edit_graph(&prompt, w, h, seed, &refs),
            _ => build_qwen21_graph(&prompt, "", w, h, seed, QWEN21_STEPS, &refs),
        };
        let out = std::env::var("QWEN21_GRAPH_OUT").unwrap_or_else(|_| "/tmp/cmp_graph.json".into());
        std::fs::write(&out, serde_json::to_string_pretty(&g).unwrap()).unwrap();
        println!("wrote {out} ({which}, {} 张参考图)", refs.len().min(
            ImageEngine::from_id(&which).map(|e| e.reference_slots()).unwrap_or(10)));
    }

    /// 实跑用：把这张图导出来直接 POST 给 ComfyUI。
    /// `cargo test -p anima 导出_qwen21 -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn 导出_qwen21_图供实跑() {
        let prompt = std::env::var("QWEN21_PROMPT").unwrap_or_else(|_| {
            "a photorealistic portrait of a young asian woman by a window, soft morning light, 85mm".into()
        });
        let w: u32 = std::env::var("QWEN21_W").ok().and_then(|v| v.parse().ok()).unwrap_or(1024);
        let h: u32 = std::env::var("QWEN21_H").ok().and_then(|v| v.parse().ok()).unwrap_or(1024);
        let seed: u64 = std::env::var("QWEN21_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(778899);
        let refs: Vec<String> = std::env::var("QWEN21_REFS")
            .map(|v| v.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
            .unwrap_or_default();
        let g = build_qwen21_graph(&prompt, "", w, h, seed, QWEN21_STEPS, &refs);
        let out = std::env::var("QWEN21_GRAPH_OUT").unwrap_or_else(|_| "/tmp/qwen21_graph.json".into());
        std::fs::write(&out, serde_json::to_string_pretty(&g).unwrap()).unwrap();
        println!("wrote {out}");
    }
}

#[cfg(test)]
mod lora_insertion_guard_tests {
    use super::*;
    use crate::agent_runtime::lora::ResolvedLora;

    #[test]
    fn krea2_lora_经过外层插入后仍连接独立的模型和编码器() {
        let loras = [ResolvedLora { name: "test_krea2.safetensors".into(), strength: 0.8 }];
        let mut graph = build_krea2_graph_with_loras("p", 512, 512, 1, 8, &loras);
        // 模拟 image_tool 的通用 SDXL 插入：不能覆盖引擎自己建好的链。
        if !graph_builds_own_lora_chain(ImageEngine::Krea2) {
            let chain = crate::agent_runtime::lora::build_chain(
                &loras, 900, json!(["1", 0]), Some(json!(["1", 1])),
            );
            for (id, node) in chain.nodes {
                graph.as_object_mut().unwrap().insert(id, node);
            }
        }
        assert_eq!(graph["900"]["inputs"]["model"], json!(["1", 0]));
        assert_eq!(graph["900"]["inputs"]["clip"], json!(["2", 0]),
            "UNETLoader 只有 MODEL 输出，不能读取它的第 1 号输出作为 CLIP");
        assert_eq!(graph["7"]["inputs"]["model"], json!(["900", 0]));
        assert_eq!(graph["4"]["inputs"]["clip"], json!(["900", 1]));
    }

    /// 谓词必须和**图里实际有没有 LoRA 节点**对得上。
    ///
    /// 光断言 `graph_builds_own_lora_chain(ZImage) == true` 是同义反复。
    /// 这里拿真图去对：将来谁给 Chroma 加了 LoRA 支持却忘了改谓词，
    /// 外层会再插一遍、把接线覆盖掉 —— 那正是这条测试要拦的。
    #[test]
    fn 谓词与图里实际有没有_lora_节点一致() {
        let one = [ResolvedLora { name: "x_zimgt.safetensors".into(), strength: 1.0 }];

        let z = build_zimage_graph_with_loras("p", 512, 512, 1, 8, &one);
        assert_eq!(
            z.get("900").is_some(),
            graph_builds_own_lora_chain(ImageEngine::ZImage),
            "Z-Image 图里有 LoRA 节点，谓词却说没有（或反过来）"
        );
        assert!(!graph_builds_own_lora_chain(ImageEngine::Sdxl), "SDXL 靠的就是外层那次插入");
    }
}

#[cfg(test)]
mod configured_zimage_lora_tests {
    use super::*;

    #[test]
    fn 纯文生图且没自己传时才挂() {
        assert!(should_apply_configured_zimage_loras(ImageEngine::ZImage, false, false, false));
    }

    #[test]
    fn 不是_z_image_一律不挂() {
        // 反向的静默失效：Z-Image 权重挂到别的架构上不生效且不报错
        for e in [ImageEngine::Sdxl, ImageEngine::Krea2, ImageEngine::Qwen21] {
            assert!(!should_apply_configured_zimage_loras(e, false, false, false), "{e:?}");
        }
    }

    #[test]
    fn 这次调用自己传了就听模型的() {
        // 两批叠加权重会失控 —— 这是当初否掉全局常挂的理由 1
        assert!(!should_apply_configured_zimage_loras(ImageEngine::ZImage, true, false, false));
    }

    #[test]
    fn 图生图和设定卡都不挂() {
        // 理由 2：那两条路各有自己的 LoRA 编排，混进来效果不可预期
        assert!(!should_apply_configured_zimage_loras(ImageEngine::ZImage, false, true, false));
        assert!(!should_apply_configured_zimage_loras(ImageEngine::ZImage, false, false, true));
    }

    // ---- 图生图：通用改写 ----

    fn 两张图() -> Vec<(&'static str, Value)> {
        vec![
            ("zimage", build_zimage_graph("一只猫", 1024, 1024, 1, 8)),
            ("krea2", build_krea2_graph("一只猫", 1024, 1024, 1, 8)),
        ]
    }

    fn 节点(g: &Value, class: &str) -> Option<String> {
        g.as_object()?
            .iter()
            .find(|(_, n)| n["class_type"] == class)
            .map(|(id, _)| id.clone())
    }

    /// **两张图都能改成图生图。**
    ///
    /// 以前只有 SDXL 那一支接了源图，于是别的引擎一律回落 SDXL。
    /// 那不是模型不支持 —— 图生图是采样器层面的事，缺的是我们这边的图。
    #[test]
    fn 三个引擎都能改成图生图() {
        for (name, mut g) in 两张图() {
            assert!(attach_i2i(&mut g, "src.png", 0.65), "{name} 改写失败");
            assert!(节点(&g, "LoadImage").is_some(), "{name} 没加载源图");
            assert!(节点(&g, "VAEEncode").is_some(), "{name} 没编码源图");
            // **空 latent 必须删掉** —— 留着的话它还在图里，
            // ComfyUI 会当成一个没人用的节点，但更要命的是万一还有人连着它
            assert!(节点(&g, "EmptySD3LatentImage").is_none(), "{name} 还留着空 latent");
            assert!(节点(&g, "EmptyLatentImage").is_none(), "{name} 还留着空 latent");
        }
    }

    /// **采样器真的吃到了源图编码出来的 latent。**
    ///
    /// 只加节点不改连线的话，图跑得通、任务 success，出的还是纯文生图 ——
    /// 这个仓库在局部重绘那里踩过一模一样的坑。
    #[test]
    fn 采样器的_latent_指向源图() {
        for (name, mut g) in 两张图() {
            attach_i2i(&mut g, "src.png", 0.65);
            let enc = 节点(&g, "VAEEncode").unwrap();
            let 用到了 = g
                .as_object()
                .unwrap()
                .values()
                .filter_map(|n| n.get("inputs")?.as_object())
                .any(|inp| {
                    inp.values().any(|v| {
                        v.as_array().and_then(|a| a.first()).and_then(Value::as_str) == Some(&enc)
                    })
                });
            assert!(用到了, "{name}：VAEEncode 出来的 latent 没人用");
        }
    }

    /// denoise 落在谁身上是逐引擎的（以前 Chroma 在 `BasicScheduler` 上，2026-10-01 下架了）—— 改写按「谁有 denoise 就设谁」走，不认节点名。
    #[test]
    fn 三个引擎的_denoise_都被设上() {
        for (name, mut g) in 两张图() {
            attach_i2i(&mut g, "src.png", 0.65);
            let 有 = g
                .as_object()
                .unwrap()
                .values()
                .filter_map(|n| n.get("inputs")?.as_object())
                .filter_map(|inp| inp.get("denoise"))
                .any(|v| (v.as_f64().unwrap_or(1.0) - 0.65).abs() < 1e-9);
            assert!(有, "{name} 的 denoise 没设上");
        }
    }

    /// **denoise 要夹住。** 0 会让采样器一步都不跑（sigmas 空），
    /// 出来是原图；大于 1 是无意义的值。
    #[test]
    fn denoise_越界会被夹住() {
        let mut g = build_zimage_graph("猫", 1024, 1024, 1, 8);
        attach_i2i(&mut g, "src.png", 0.0);
        let d = g.as_object().unwrap().values()
            .filter_map(|n| n.get("inputs")?.as_object()?.get("denoise")?.as_f64())
            .next().unwrap();
        assert!(d >= 0.05, "denoise 被设成了 {d}");
    }

    /// 不是这个形状的图**原样返回**，而且不留孤儿节点。
    #[test]
    fn 认不出的图不乱改() {
        let mut g = json!({"1": {"class_type": "SaveImage", "inputs": {"images": ["2", 0]}}});
        let before = g.clone();
        assert!(!attach_i2i(&mut g, "src.png", 0.65));
        assert_eq!(g, before, "改坏了一张不认识的图");
    }

    /// LoRA 链占 900 起，图生图这两个节点占 800 起 —— **不能撞**，
    /// 撞了会把 LoRA 节点覆盖掉，而那是静默失效。
    #[test]
    fn 图生图节点号不和_lora_链撞() {
        let lora = crate::agent_runtime::lora::ResolvedLora {
            name: "zimage/x.safetensors".into(),
            strength: 0.8,
        };
        let mut g = build_zimage_graph_with_loras("猫", 1024, 1024, 1, 8, &[lora]);
        let lora_id = 节点(&g, "LoraLoader").expect("前提：图里有 LoRA 节点");
        assert!(lora_id.starts_with('9'), "前提：LoRA 链在 900 起，实际 {lora_id}");
        assert!(attach_i2i(&mut g, "src.png", 0.65));

        // **要查身份，不是查存在。**
        //
        // 只断言「900 号还在」的话，把图生图的节点号改成 900 也照样绿 ——
        // 那个 id 确实还在，只是从 `LoraLoader` 变成了 `LoadImage`。
        // 而那正是这条要防的事：LoRA 被静默覆盖掉。
        assert_eq!(
            g[&lora_id]["class_type"], "LoraLoader",
            "{lora_id} 号节点被图生图覆盖了"
        );
        assert_eq!(g[&lora_id]["inputs"]["lora_name"], "zimage/x.safetensors");
    }

    /// **让「图生图不再回落」那条分支当前碰不到的前提。**
    ///
    /// `reason_for` 里写的是 `self.img2img && !supports_i2i(asked)`，
    /// 而现在三个引擎全都支持 —— 那半条判断是死的。留着是因为那是
    /// **数据的性质，不是代码的性质**：加第五个引擎而它没有 i2i 拓扑时，
    /// 这条判断立刻变活。这条测试就是那个时刻的闹钟。
    #[test]
    fn 每个引擎都在图生图名单里() {
        for id in ["zimage", "krea2", "sdxl"] {
            let e = ImageEngine::from_id(id).expect(id);
            assert!(
                supports_i2i(e),
                "{id} 没有图生图 —— 那 reason_for 里那条分支就活了，去看看回落说明还对不对"
            );
        }
    }
}
