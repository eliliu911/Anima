//! 角色设定图的三条管线：三视图 / 表情 / 动作。
//!
//! ## 为什么是三条，不是一条
//!
//! 早先是一条纯文生图，靠一段提示词让 SDXL 排出「正面/侧面/背面」。
//! **做不到** —— 实测出了 4 个人而且全是正面；把「背面、看不见脸」的权重
//! 拉到 1.5，身体转过去了、脸还是回头看镜头。这不是提示词写得不够好，
//! SDXL 的训练数据压倒性是正面人像。
//!
//! 三档的难点根本不同，所以各用各的办法：
//!
//! | 档 | 难在哪 | 办法 |
//! |---|---|---|
//! | 三视图 | 画不出背面 | turnaround LoRA 出整排，再切 |
//! | 表情 | 不难，都是正面特写 | 分格生成，IPAdapter 锁脸 |
//! | 动作 | 具体动作控不住 | 分格 + ControlNet 骨架（没有就退回提示词） |
//!
//! 三视图仍走「整排 + 切片」是因为 LoRA 教的就是那个版式，分格反而用不上它。
//! 表情和动作分格生成，每格分辨率比切片高好几倍。
//!
//! ## LoRA 权重是量出来的
//!
//! `CharTurnXL` 在 3D 角色设定图上训练，权重高了会把画风整个带成 3D 卡通。
//! 实测同一 seed 下：0.80 是塑料人偶，0.70 开始发假，**0.55 是真实照片质感
//! 且版式依然稳**。所以默认 0.55，不是 LoRA 页面推荐的 0.7。

use serde_json::{json, Map, Value};

/// 一次出图的完整参数。
#[derive(Debug, Clone)]
pub struct Shot {
    pub checkpoint: String,
    pub positive: String,
    pub negative: String,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    pub cfg: f32,
    pub seed: i64,
    /// (文件名, 权重)。没有就不挂 LoRA
    pub lora: Option<(String, f32)>,
    /// 第二个 LoRA，用来补底模缺的东西（比如东亚面孔）。
    ///
    /// **权重要低于主 LoRA。** 两个 LoRA 叠加会互相削弱 ——
    /// 面孔 LoRA 给高了会把 turnaround 的版式冲散（那是主 LoRA 教的）。
    pub lora2: Option<(String, f32)>,
    /// ComfyUI input 目录下的参考图文件名，用于 IPAdapter 锁脸
    pub reference: Option<String>,
    /// 参考图进入模型的方式。None = 纯提示词，不挂身份节点。
    pub identity_mode: IdentityMode,
    /// IPAdapter 权重。**不同档位不一样**，见上面的常量
    pub reference_weight: f32,
    /// IPAdapter 结束步数百分比 (0.0 - 1.0)。
    /// 表情格设为 0.75 释放后 25% 步数生成丰富微表情形变
    pub reference_end_at: f32,
    /// 姿势约束：(ComfyUI input 下的姿势参考图文件名, 强度)。
    ///
    /// 有它才控得住具体动作。**没有就只靠提示词** —— 实测「奔跑」出来是
    /// 站着被风吹，因为 SDXL 对动作词的遵循度很低。
    /// (姿势参考图, 强度, ControlNet 模型名)
    pub pose_control: Option<(String, f32, String)>,
    /// 两段式放大。`None` = 一段出图。
    ///
    /// SD1.5 必须用它 —— 见 [`Hires`]。
    pub hires: Option<Hires>,
    pub filename_prefix: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum IdentityMode {
    IpAdapter,
    Pulid { file: String },
    None,
}

/// 先在底模的原生分辨率定构图，再放大 latent 低强度精修。
///
/// **SD1.5 走这条路不是为了更清楚，是为了构图不散。**
/// 实测 `three_sided_view_sd15` + majicmix：
///
/// | 分辨率 | 结果 |
/// |---|---|
/// | 832×512（LoRA 元数据里写的训练分辨率） | 四格干净，正/侧/侧/背齐全 ✓ |
/// | 1536×1024（SDXL 那档的尺寸） | 平铺出五个偏正面的人，**衣服还各不相同** ✗ |
///
/// 直接出大图时 SD1.5 是在「铺满画布」而不是在排三视图。
/// 所以先按 832×512 出，再 latent 放大到 1664×1024、denoise 0.45 精修 ——
/// 构图在第一段就定死了，第二段只补细节。实测 90 秒。
#[derive(Debug, Clone, Copy)]
pub struct Hires {
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    /// **别超过 0.5。** 高了第二段会重新构图，等于白做第一段
    pub denoise: f32,
}

/// 三视图 LoRA 的默认权重。**别调高** —— 见模块注释
pub const TURNAROUND_LORA_WEIGHT: f32 = 0.55;

/// IPAdapter 权重：**分档，不是一个数**。
///
/// 它迁移的不只是身份 —— 参考图的**表情**和**衣服颜色**会一起过来。
/// 实测对照（同一参考图，一张面无表情的白上衣半身照）：
///
/// - 三视图要黑裙子，权重 0.85 → 出来银色，正是参考图上衣的颜色。
/// - 表情格更明显。同一提示词（愤怒，怒眉张口）扫三档：
///
/// ```text
/// 0.85   几乎没有表情，嘴微张
/// 0.55   中等，眉头皱起
/// 0.35   完全到位，怒眉 + 张口吼叫
/// ```
///
///   **三档的身份都保住了**（短发栗色、同一张脸的结构），
///   所以为了表情把权重压下来是划算的，不会换成另一个人。
///   0.45 取在「中等到完全」之间。
///
/// 所以按「这一格要固定什么」分档：脸要像但表情要自由的，权重必须低。
pub const IPADAPTER_WEIGHT_IDENTITY: f32 = 0.80;
/// 亚洲人像 LoRA 的权重。**明显低于主 LoRA** ——
/// 它只负责把骨相往东亚拉，版式仍归 CharTurnXL 管；给高了会把整排冲散。
pub const ASIAN_LORA_WEIGHT: f32 = 0.35;
/// 面部机位 LoRA 的权重。和三视图同理，高了会把画风带成 3D
pub const FACE_LORA_WEIGHT: f32 = 0.55;
/// 在 ComfyUI 的 controlnet 目录里找 union 模型。
pub fn find_controlnet() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let dirs = [
        std::env::var_os("ANIMA_COMFY_CONTROLNET").map(std::path::PathBuf::from),
        Some(std::path::PathBuf::from(format!(
            "{home}/AI/ComfyUI/models/controlnet"
        ))),
        Some(std::path::PathBuf::from(format!(
            "{home}/ComfyUI/models/controlnet"
        ))),
    ];
    for dir in dirs.into_iter().flatten() {
        if !dir.is_dir() {
            continue;
        }
        // **union 优先。** 目录里可能同时有专用 openpose 和 union 两个模型
        // （实测这台机器上就是：4.7GB 的 openpose 专用版 + 2.3GB 的 union）。
        // union 一个文件覆盖 openpose/depth/canny，后面接景深、线稿时不用再换模型；
        // 而按 read_dir 的顺序取第一个是**不确定的** —— 同一份代码在两台机器上
        // 可能挂上不同的模型，出图差异却找不到原因
        let mut found: Vec<String> = std::fs::read_dir(&dir)
            .ok()?
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| {
                let l = n.to_lowercase();
                l.ends_with(".safetensors") && (l.contains("union") || l.contains("openpose"))
            })
            .collect();
        found.sort_by_key(|n| {
            let l = n.to_lowercase();
            (!l.contains("union"), n.clone()) // union 排前面，其余按名字稳定排序
        });
        if let Some(first) = found.into_iter().next() {
            return Some(first);
        }
    }
    None
}

/// ControlNet 姿势约束强度。**0.8 而不是 1.0** ——
/// 拉满会让人物僵硬地贴着骨架、失去自然的重心和衣褶
pub const POSE_CONTROL_STRENGTH: f32 = 0.8;
/// 表情格：面部锁脸与表情提示词平衡（<=0.5 允许嘴型和眼角自然做出喜怒哀乐）
pub const IPADAPTER_WEIGHT_EXPRESSION: f32 = 0.48;
/// 全身格：强锁面部骨相与五官，服装由提示词决定
pub const IPADAPTER_WEIGHT_BODY: f32 = 0.60;
/// 三视图整排的尺寸。LoRA 页面建议 1536×1024
pub const SHEET_SIZE: (u32, u32) = (1536, 1024);
/// 分格图的尺寸，SDXL 竖构图
pub const SLOT_SIZE: (u32, u32) = (832, 1216);

const NEG_COMMON: &str =
    "(worst quality, low quality:1.4), 3d render, cgi, cartoon, anime, illustration, doll, \
     plastic skin, cropped, watermark, text, signature, \
     bad anatomy, deformed, mutated, bad hands, deformed hands, extra fingers, missing fingers";

/// 写实向的正向词。**必须显式写** —— turnaround LoRA 会把画风往 3D 拉，
/// 不压回来的话出来是塑料人偶
/// 每一档都带上的画质词。
///
/// **不要往这里写族裔词。** 写死在这儿就没法覆盖 —— 用户在触发词里写
/// 「caucasian woman」时，正负两边会打架。族裔归 [`ensure_ethnicity`] 管：
/// 它只在用户**没写**的时候补，写了就一个字不动。
///
/// **也不要写肤质词。** 「smooth porcelain skin」会和年长角色打架 ——
/// 内置人设里有个 65 岁的角色，全局注入会把她的皱纹磨平，
/// 而那正是这个角色的特征。肤质属于角色描述。
const POS_REAL: &str = "(masterpiece, realistic photo:1.2), raw photo, photorealistic, \
                        real person, photograph, detailed skin texture, detailed eyes";

/// 三视图整排的两条路。
///
/// ## 为什么要分档，而不是「都试一遍」
///
/// 这两条路上**每一样东西都不通用**，而且混搭起来**不报错**：
///
/// | | SDXL | SD1.5 |
/// |---|---|---|
/// | 底模 | RealVisXL | majicmixRealistic |
/// | 版式 LoRA | CharTurnXLRetag（435M） | three_sided_view_sd15（144M） |
/// | 整排尺寸 | 1536×1024 一段出 | 832×512 + 放大到 1664×1024 |
/// | 触发词 | `charturnersheet` | `three sided view, multiple views` |
///
/// 把 SDXL 的 LoRA 挂到 SD1.5 底模上，ComfyUI **照常跑完、不报错**，
/// 只是那个 LoRA 完全不起作用 —— 出来是一张普通单人像。
/// 尺寸搞反同样不报错：SD1.5 出 1536×1024 会平铺出五个偏正面、
/// 衣服还各不相同的人。所以四样东西必须由同一个枚举决定。
///
/// ## 两条路各自强在哪（实测同一角色、同一种子）
///
/// - **SD1.5**：四格干净、正/侧/侧/背齐全、东亚面孔天生就对、
///   **黑色天鹅绒礼服保住了**（SDXL 那几版都跑成了银色）、
///   格与格之间有明显背景间隔所以切图可靠。90 秒。
/// - **SDXL**：单格分辨率高一档；但需要靠 `ensure_ethnicity` 补族裔词，
///   服装还原也弱一些。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum SheetEngine {
    #[default]
    Sdxl,
    Sd15,
}


impl SheetEngine {
    pub fn from_id(id: &str) -> Self {
        match id.trim().to_ascii_lowercase().as_str() {
            "sd15" | "sd1.5" | "majicmix" => Self::Sd15,
            _ => Self::Sdxl,
        }
    }

    /// 第一段（定构图）的尺寸。SD1.5 **必须**用它的训练分辨率
    pub fn sheet_size(self) -> (u32, u32) {
        match self {
            Self::Sdxl => SHEET_SIZE,
            Self::Sd15 => (832, 512),
        }
    }

    /// 第二段放大。SDXL 一段就够，SD1.5 必须两段
    pub fn hires(self) -> Option<Hires> {
        match self {
            Self::Sdxl => None,
            Self::Sd15 => Some(Hires {
                width: 1664,
                height: 1024,
                steps: 16,
                denoise: 0.35,
            }),
        }
    }

    /// 版式 LoRA 的文件名关键词。**两条路的 LoRA 不能互换**
    pub fn layout_lora_keywords(self) -> &'static [&'static str] {
        match self {
            Self::Sdxl => &["charturn", "char_turn", "turnaround"],
            Self::Sd15 => &["three_sided_view", "three_sided", "multiview"],
        }
    }

    /// 版式 LoRA 的权重。
    ///
    /// SDXL 那个是 3D 角色设定图训练的，高了会把画风带成塑料人偶（见模块注释）；
    /// SD1.5 这个是照片训练的，0.8 版式稳且不伤画质
    pub fn layout_lora_weight(self) -> f32 {
        match self {
            Self::Sdxl => TURNAROUND_LORA_WEIGHT,
            Self::Sd15 => 0.8,
        }
    }

    /// 版式前缀 —— **整段都归引擎管，调用方不要再往后拼版式词**。
    pub fn layout_trigger(self) -> &'static str {
        match self {
            Self::Sdxl => {
                "charturnersheet, character turnaround model sheet, \
                           multiple views of the same character in a row: \
                           front view, side view, back view. full body, standing,"
            }
            // SD1.5 three_sided_view 训练标签显式需要 front view, side view, back view 触发三视角
            Self::Sd15 => {
                "three sided view, three_sided_view, multiple views, multiple_views, \
                           front view, side view, back view, \
                           full body, full_body, standing, shoes, highres, simple background,"
            }
        }
    }

    /// 背景描述。**不能两条路共用** —— `white background` 在
    /// three_sided_view 的训练标签里是 100% 出现的，写成灰背景等于和 LoRA 打架。
    fn background(self) -> &'static str {
        match self {
            Self::Sdxl => "plain light grey studio background",
            Self::Sd15 => "white background, plain seamless backdrop",
        }
    }

    /// 底模是不是这条路能用的。
    ///
    /// **这是最要命的一条**：SD1.5 底模配 SDXL LoRA（或反过来）不报错、
    /// 照常出图，只是版式 LoRA 完全不起作用
    pub fn checkpoint_matches(self, ckpt: &str) -> bool {
        let low = ckpt.to_ascii_lowercase();
        let looks_xl = low.contains("xl") || low.contains("realvis") || low.contains("awportrait");
        match self {
            Self::Sdxl => looks_xl,
            Self::Sd15 => !looks_xl,
        }
    }

    /// 这条路的底模优先级
    pub fn pick_checkpoint(self, available: &[String]) -> Option<String> {
        match self {
            Self::Sdxl => pick_turnaround_checkpoint(available),
            Self::Sd15 => {
                let usable: Vec<&String> = available
                    .iter()
                    .filter(|c| self.checkpoint_matches(c))
                    .collect();
                for want in ["majicmix", "chilloutmix", "realistic"] {
                    if let Some(h) = usable.iter().find(|c| c.to_lowercase().contains(want)) {
                        return Some((*h).clone());
                    }
                }
                usable.first().map(|c| (*c).clone())
            }
        }
    }
}

/// 三视图整排。**版式触发词、尺寸、LoRA 权重、放大方式全部跟着 `engine` 走** ——
/// 见 [`SheetEngine`]，混搭起来不报错但版式 LoRA 会静默失效。
pub fn turnaround(
    engine: SheetEngine,
    checkpoint: &str,
    trigger: &str,
    clothing: &str,
    lora: Option<String>,
    reference: Option<String>,
    seed: i64,
    prefix: &str,
) -> Shot {
    let (w, h) = engine.sheet_size();
    Shot {
        checkpoint: checkpoint.into(),
        positive: format!(
            "{} {POS_REAL}, {trigger}, {clothing}, identical character across all views, same face, same outfit, consistent proportions, no text, {}",
            engine.layout_trigger(),
            engine.background()
        ),
        negative: format!("{NEG_COMMON}, multiple characters, different outfits"),
        width: w,
        height: h,
        steps: 28,
        cfg: 6.0,
        seed,
        lora: lora.map(|l| (l, engine.layout_lora_weight())),
        lora2: None,
        reference,
        identity_mode: IdentityMode::IpAdapter,
        // 三视图整排：兼顾面容一致性与三视角全身站姿排版
        reference_weight: match engine {
            SheetEngine::Sdxl => 0.40,
            SheetEngine::Sd15 => 0.45,
        },
        reference_end_at: match engine {
            SheetEngine::Sdxl => 0.55,
            SheetEngine::Sd15 => 0.65,
        },
        pose_control: None,
        hires: engine.hires(),
        filename_prefix: prefix.into(),
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TurnaroundSlot {
    pub key: &'static str,
    pub title: &'static str,
    pub prompt: &'static str,
    pub negative: &'static str,
}

pub const TURNAROUND_SLOTS: &[TurnaroundSlot] = &[
    TurnaroundSlot {
        key: "turnaround_front",
        title: "正面全身三视图",
        prompt: "front view, facing camera, looking directly at camera, straight upright standing pose, feet together, arms straight down at sides, legs straight, centered full-length body studio shot, head to toe, shoes on floor",
        negative: "sitting, chair, seat, bench, leaning, crouching, kneeling, side view, back view, turned body, arms up, dynamic action",
    },
    TurnaroundSlot {
        key: "turnaround_side",
        title: "侧面全身三视图",
        prompt: "side view, perfect 90 degree side profile view, turned 90 degrees to side, straight upright standing pose, arms straight down at sides, legs straight, centered full-length body studio shot, head to toe, looking straight ahead to side, sharp clean profile silhouette, shoes on floor",
        negative: "sitting, chair, seat, bench, leaning, crouching, kneeling, front view, back view, facing camera, turned body, arms up, dynamic action",
    },
    TurnaroundSlot {
        key: "turnaround_back",
        title: "背面全身三视图",
        prompt: "back view, full body shot from behind, straight upright standing pose, facing directly away from camera, arms straight down at sides, legs straight, showing back of head, short neat haircut, tailored jacket back silhouette, shoes on floor, centered full-length body studio shot",
        negative: "front view, face, turning head, looking back, looking at camera, turned body, sitting, chair, seat, bench, leaning, crouching, kneeling, dynamic action",
    },
];

/// 单格三视图视角（正面 / 侧面 / 背面）。逐格独立精准出图，彻底杜绝切片重影与视角混淆。
pub fn turnaround_view(
    engine: SheetEngine,
    checkpoint: &str,
    trigger: &str,
    clothing: &str,
    view_prompt: &str,
    view_negative: &str,
    reference: Option<String>,
    seed: i64,
    prefix: &str,
) -> Shot {
    let (w, h) = match engine {
        SheetEngine::Sdxl => (832, 1216),
        SheetEngine::Sd15 => (512, 768),
    };
    let hires = match engine {
        SheetEngine::Sdxl => None,
        SheetEngine::Sd15 => Some(Hires {
            width: 1024,
            height: 1536,
            steps: 14,
            denoise: 0.35,
        }),
    };

    let is_back = view_prompt.contains("back view");
    let ref_for_view = if is_back { None } else { reference };

    Shot {
        checkpoint: checkpoint.into(),
        positive: format!(
            "solo, 1person, standing, straight upright neutral standing pose, {view_prompt}, {POS_REAL}, {trigger}, {clothing}, plain light grey studio background, crisp clean studio lighting"
        ),
        negative: format!(
            "(sitting:1.5), (chair:1.5), (bench:1.5), (seated:1.5), (crouching:1.4), (leaning:1.4), (multiple people:1.5), (2persons:1.5), (2boys:1.5), group, crowd, {NEG_COMMON}, {view_negative}"
        ),
        width: w,
        height: h,
        steps: 28,
        cfg: 6.5,
        seed,
        lora: None,
        lora2: None,
        reference: ref_for_view,
        identity_mode: IdentityMode::IpAdapter,
        reference_weight: 0.65,
        reference_end_at: 0.75,
        pose_control: None,
        hires,
        filename_prefix: prefix.into(),
    }
}

/// 单格表情（头部特写）。
pub fn expression(
    checkpoint: &str,
    trigger: &str,
    emotion: &str,
    reference: Option<String>,
    seed: i64,
    prefix: &str,
) -> Shot {
    Shot {
        checkpoint: checkpoint.into(),
        positive: format!(
            "close-up portrait, head and shoulders only, facing camera, {emotion}, \
             {POS_REAL}, {trigger}, plain light grey studio background, soft even lighting"
        ),
        negative: format!("{NEG_COMMON}, full body, multiple people"),
        width: 1024,
        height: 1024,
        steps: 26,
        cfg: 6.0,
        seed,
        lora: None,
        lora2: None,
        reference,
        identity_mode: IdentityMode::IpAdapter,
        reference_weight: IPADAPTER_WEIGHT_EXPRESSION,
        reference_end_at: 0.85,
        pose_control: None,
        hires: None,
        filename_prefix: prefix.into(),
    }
}

/// 单格动作。`skeleton` 留给后面接 ControlNet —— 现在没有模型就只靠提示词，
/// 并且**在结果里说明**，不要让用户以为姿势被锁住了。
#[allow(clippy::too_many_arguments)]
pub fn pose(
    checkpoint: &str,
    trigger: &str,
    clothing: &str,
    action: &str,
    reference: Option<String>,
    pose_control: Option<String>,
    seed: i64,
    prefix: &str,
) -> Shot {
    Shot {
        checkpoint: checkpoint.into(),
        positive: format!(
            "full body shot, head to toe, {action}, {POS_REAL}, {trigger}, {clothing}, \
             plain light grey studio background"
        ),
        negative: format!("{NEG_COMMON}, close-up, multiple people, cropped feet"),
        width: SLOT_SIZE.0,
        height: SLOT_SIZE.1,
        steps: 26,
        cfg: 6.5,
        seed,
        lora: None,
        lora2: None,
        reference,
        identity_mode: IdentityMode::IpAdapter,
        reference_weight: IPADAPTER_WEIGHT_BODY,
        reference_end_at: 0.85,
        pose_control: pose_control
            .and_then(|p| find_controlnet().map(|m| (p, POSE_CONTROL_STRENGTH, m))),
        hires: None,
        filename_prefix: prefix.into(),
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FaceAngleSlot {
    pub key: &'static str,
    pub title: &'static str,
    pub prompt: &'static str,
}

pub const FACE_ANGLES: &[FaceAngleSlot] = &[
    FaceAngleSlot {
        key: "0_front",
        title: "0° 正面标准特写",
        prompt: "front view, facing forward, looking directly at camera, centered headshot",
    },
    FaceAngleSlot {
        key: "1_left_45",
        title: "左侧 45° 侧颜",
        prompt: "three-quarter view, head turned 45 degrees to the left, looking slightly upward, sharp jawline",
    },
    FaceAngleSlot {
        key: "2_right_45",
        title: "右侧 45° 侧颜",
        prompt: "three-quarter view, head turned 45 degrees to the right, looking down at camera, sharp jawline",
    },
    FaceAngleSlot {
        key: "3_side_90",
        title: "90° 正侧面轮廓",
        prompt: "perfect 90 degree side profile view, looking to the side, sharp facial silhouette, clean ear contour",
    },
    FaceAngleSlot {
        key: "4_cinematic_tilt",
        title: "电影级微仰角",
        prompt: "cinematic low angle shot, looking down at camera with confidence, dynamic cinematic lighting, sharp gaze",
    },
];

/// 单格面部机位（头部特写），逐格定向生成与 100% 锁脸。
pub fn face_angle(
    checkpoint: &str,
    trigger: &str,
    angle_prompt: &str,
    reference: Option<String>,
    seed: i64,
    prefix: &str,
) -> Shot {
    Shot {
        checkpoint: checkpoint.into(),
        positive: format!(
            "close-up portrait, head and shoulders only, {angle_prompt}, \
             {POS_REAL}, {trigger}, plain light grey studio background, soft even lighting"
        ),
        negative: format!("{NEG_COMMON}, full body, wide shot, multiple people, blurry"),
        width: 1024,
        height: 1024,
        steps: 26,
        cfg: 6.0,
        seed,
        lora: None,
        lora2: None,
        reference,
        identity_mode: IdentityMode::IpAdapter,
        reference_weight: IPADAPTER_WEIGHT_IDENTITY,
        reference_end_at: 0.90,
        pose_control: None,
        hires: None,
        filename_prefix: prefix.into(),
    }
}

/// 面部机位（5 维）。用 FaceTurn LoRA 出整排头像，再切。
pub fn face_angles(
    checkpoint: &str,
    trigger: &str,
    lora: Option<String>,
    reference: Option<String>,
    seed: i64,
    prefix: &str,
) -> Shot {
    Shot {
        checkpoint: checkpoint.into(),
        positive: format!(
            "faceturnsheet, character face turnaround, multiple head shots of the same face in a row: \
             front, three-quarter, profile, looking up, looking down. head and shoulders only, \
             {POS_REAL}, {trigger}, plain light grey studio background"
        ),
        negative: format!("{NEG_COMMON}, full body, different people, different hairstyles"),
        width: SHEET_SIZE.0,
        height: 768,
        steps: 28,
        cfg: 6.0,
        seed,
        lora: lora.map(|l| (l, FACE_LORA_WEIGHT)),
        lora2: None,
        reference,
        identity_mode: IdentityMode::IpAdapter,
        // 机位图要的是同一张脸从不同角度看，身份权重该高
        reference_weight: IPADAPTER_WEIGHT_IDENTITY,
        reference_end_at: 0.90,
        pose_control: None,
        hires: None,
        filename_prefix: prefix.into(),
    }
}

/// 把 [`Shot`] 编成 ComfyUI 的 prompt 图。
///
/// 节点编号刻意写死且连续，方便出错时对着 ComfyUI 的报错定位。
/// 可选部分（LoRA、IPAdapter）用「有就插、没有就把上游直连下游」的方式接，
/// 这样图永远是连通的 —— 少一根线 ComfyUI 会报一句很难懂的错。
pub fn to_workflow(shot: &Shot) -> Value {
    let mut g = Map::new();

    g.insert(
        "1".into(),
        json!({
            "class_type": "CheckpointLoaderSimple",
            "inputs": { "ckpt_name": shot.checkpoint }
        }),
    );

    // MODEL / CLIP 的当前出口，插入可选节点时往后挪
    let mut model_src = json!(["1", 0]);
    let mut clip_src = json!(["1", 1]);

    if let Some((name, weight)) = &shot.lora {
        g.insert(
            "2".into(),
            json!({
                "class_type": "LoraLoader",
                "inputs": {
                    "model": model_src, "clip": clip_src,
                    "lora_name": name,
                    "strength_model": weight, "strength_clip": weight
                }
            }),
        );
        model_src = json!(["2", 0]);
        clip_src = json!(["2", 1]);
    }

    // 第二个 LoRA **串在第一个之后** —— 两个都要作用到同一条 model/clip 上。
    // 各挂各的（都从 checkpoint 取输入）只会让后一个覆盖前一个
    if let Some((name, weight)) = &shot.lora2 {
        g.insert(
            "16".into(),
            json!({
                "class_type": "LoraLoader",
                "inputs": {
                    "model": model_src, "clip": clip_src,
                    "lora_name": name,
                    "strength_model": weight, "strength_clip": weight
                }
            }),
        );
        model_src = json!(["16", 0]);
        clip_src = json!(["16", 1]);
    }

    if let Some(file) = &shot.reference {
        g.insert(
            "3".into(),
            json!({
                "class_type": "LoadImage", "inputs": { "image": file }
            }),
        );
        match &shot.identity_mode {
            IdentityMode::IpAdapter => {
                g.insert(
                    "4".into(),
                    json!({
                        "class_type": "IPAdapterUnifiedLoader",
                        "inputs": { "model": model_src, "preset": "PLUS FACE (portraits)" }
                    }),
                );
                g.insert("5".into(), json!({
                    "class_type": "IPAdapter",
                    "inputs": {
                        "model": ["4", 0], "ipadapter": ["4", 1], "image": ["3", 0],
                        "weight": shot.reference_weight, "weight_type": "standard",
                        "combine_embeds": "concat", "start_at": 0.0, "end_at": shot.reference_end_at
                    }
                }));
                model_src = json!(["5", 0]);
            }
            IdentityMode::Pulid { file } => {
                g.insert(
                    "4".into(),
                    json!({
                        "class_type": "PulidModelLoader",
                        "inputs": { "pulid_file": file }
                    }),
                );
                g.insert(
                    "5".into(),
                    json!({
                        "class_type": "PulidEvaClipLoader",
                        "inputs": {}
                    }),
                );
                g.insert(
                    "21".into(),
                    json!({
                        "class_type": "PulidInsightFaceLoader",
                        "inputs": { "provider": "CPU" }
                    }),
                );
                g.insert(
                    "22".into(),
                    json!({
                        "class_type": "ApplyPulid",
                        "inputs": {
                            "model": model_src, "pulid": ["4", 0], "eva_clip": ["5", 0],
                            "face_analysis": ["21", 0], "image": ["3", 0],
                            "method": "fidelity", "weight": shot.reference_weight,
                            "start_at": 0.0, "end_at": shot.reference_end_at
                        }
                    }),
                );
                model_src = json!(["22", 0]);
            }
            IdentityMode::None => {}
        }
    }

    g.insert(
        "6".into(),
        json!({
            "class_type": "CLIPTextEncode",
            "inputs": { "clip": clip_src, "text": shot.positive }
        }),
    );
    g.insert(
        "7".into(),
        json!({
            "class_type": "CLIPTextEncode",
            "inputs": { "clip": clip_src, "text": shot.negative }
        }),
    );
    g.insert(
        "8".into(),
        json!({
            "class_type": "EmptyLatentImage",
            "inputs": { "width": shot.width, "height": shot.height, "batch_size": 1 }
        }),
    );

    // ControlNet 挂在 conditioning 上，不是挂在 model 上 ——
    // 这是最容易接错的地方：接到 model 上不报错，但完全不起作用
    let mut pos_src = json!(["6", 0]);
    let mut neg_src = json!(["7", 0]);
    if let Some((pose_file, strength, model)) = &shot.pose_control {
        g.insert(
            "12".into(),
            json!({
                "class_type": "LoadImage", "inputs": { "image": pose_file }
            }),
        );
        g.insert(
            "13".into(),
            json!({
                "class_type": "OpenposePreprocessor",
                "inputs": {
                    "image": ["12", 0],
                    "detect_hand": "enable", "detect_body": "enable", "detect_face": "enable",
                    "resolution": 512
                }
            }),
        );
        g.insert(
            "14".into(),
            json!({
                "class_type": "ControlNetLoader",
                "inputs": { "control_net_name": model }
            }),
        );
        g.insert(
            "15".into(),
            json!({
                "class_type": "ControlNetApplyAdvanced",
                "inputs": {
                    "positive": pos_src, "negative": neg_src,
                    "control_net": ["14", 0], "image": ["13", 0],
                    "strength": strength, "start_percent": 0.0, "end_percent": 0.85,
                    "vae": ["1", 2]
                }
            }),
        );
        pos_src = json!(["15", 0]);
        neg_src = json!(["15", 1]);
    }

    g.insert(
        "9".into(),
        json!({
            "class_type": "KSampler",
            "inputs": {
                "model": model_src, "positive": pos_src, "negative": neg_src,
                "latent_image": ["8", 0], "seed": shot.seed,
                "steps": shot.steps, "cfg": shot.cfg,
                "sampler_name": "dpmpp_2m", "scheduler": "karras", "denoise": 1.0
            }
        }),
    );
    // 两段式：latent 放大后再低 denoise 采一次。
    // **第二段必须复用同一条 model/positive/negative** —— 换掉任何一个
    // 都会在第二段重新构图，第一段定的版式就白费了
    let mut latent_src = json!(["9", 0]);
    if let Some(h) = &shot.hires {
        // **在图像空间放大，不在 latent 空间放大。**
        //
        // latent 放大（LatentUpscale）会留下重影：实测整排上每个人物旁边
        // 都有一层粉色的位移副本。latent 空间不是空间平滑的，2× 插值出来的
        // 中间值不对应任何真实图像，第二段采样就把它当成"那里本来有东西"。
        // nearest-exact 和 bislerp 都一样，换插值方式解决不了。
        //
        // 解码 → lanczos 放大像素 → 重新编码，重影完全消失，耗时还略低。
        g.insert(
            "17".into(),
            json!({
                "class_type": "VAEDecode", "inputs": { "samples": ["9", 0], "vae": ["1", 2] }
            }),
        );
        g.insert(
            "18".into(),
            json!({
                "class_type": "ImageScale",
                "inputs": {
                    "image": ["17", 0], "upscale_method": "lanczos",
                    "width": h.width, "height": h.height, "crop": "disabled"
                }
            }),
        );
        g.insert(
            "19".into(),
            json!({
                "class_type": "VAEEncode", "inputs": { "pixels": ["18", 0], "vae": ["1", 2] }
            }),
        );
        g.insert(
            "20".into(),
            json!({
                "class_type": "KSampler",
                "inputs": {
                    "model": model_src, "positive": pos_src, "negative": neg_src,
                    "latent_image": ["19", 0], "seed": shot.seed,
                    "steps": h.steps, "cfg": shot.cfg,
                    "sampler_name": "dpmpp_2m", "scheduler": "karras", "denoise": h.denoise
                }
            }),
        );
        latent_src = json!(["20", 0]);
    }

    g.insert(
        "10".into(),
        json!({
            "class_type": "VAEDecode", "inputs": { "samples": latent_src, "vae": ["1", 2] }
        }),
    );
    g.insert(
        "11".into(),
        json!({
            "class_type": "SaveImage",
            "inputs": { "filename_prefix": shot.filename_prefix, "images": ["10", 0] }
        }),
    );

    Value::Object(g)
}

/// 出图节点的编号 —— 取结果时要按它去 history 里找
pub const OUTPUT_NODE: &str = "11";

/// 从整排里挑出「正面 / 侧面 / 背面」三格的下标。
///
/// LoRA 出的是均匀排开的 N 个视角（实测 6 个），顺序大致是
/// 正面 → 逐步转身 → 背面。所以取首、中、尾。
///
/// **不要写死 6** —— 同一段提示词下次可能出 5 个或 7 个，
/// 写死的话会取到空位或漏掉背面。
pub fn pick_three(total: usize) -> Option<[usize; 3]> {
    match total {
        0..=2 => None,
        3 => Some([0, 1, 2]),
        n => Some([0, n / 2, n - 1]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 由美子的真实触发词 —— 一个族裔词都没有，这是最常见的用法
    const YUMIKO: &str = "1girl, long black silky hair, sharp confident gaze, \
                          haute couture evening dress, diamond earrings";

    #[test]
    fn 没写族裔就补东亚() {
        let out = ensure_ethnicity(YUMIKO);
        assert!(
            out.contains("east asian"),
            "留白会被底模的欧美偏置填掉：{out}"
        );
        assert!(out.contains("long black silky hair"), "原文得留着：{out}");
    }

    #[test]
    fn 写了族裔就一个字不动() {
        // 内置人设本来就写了
        for p in crate::cinema_persona::PERSONAS {
            assert_eq!(
                ensure_ethnicity(p.trigger),
                p.trigger,
                "{} 被重复注入了",
                p.id
            );
        }
    }

    /// **这条是重点**：用户明确要欧美角色时不许偷偷换人。
    #[test]
    fn 用户指定了别的族裔就不动() {
        for t in [
            "1girl, caucasian woman, blonde hair",
            "a european man in a suit",
            "black woman, natural afro",
            "south asian bride, red sari",
        ] {
            assert_eq!(ensure_ethnicity(t), t, "把用户指定的族裔改掉了：{t}");
        }
    }

    #[test]
    fn 空触发词不留下孤零零的逗号() {
        let out = ensure_ethnicity("");
        assert!(!out.trim().ends_with(','), "{out}");
        assert!(out.contains("east asian"));
    }

    fn refs_of(g: &Value) -> Vec<String> {
        let mut out = Vec::new();
        for (_, node) in g.as_object().unwrap() {
            for (_, v) in node["inputs"].as_object().unwrap() {
                if let Some(a) = v.as_array() {
                    if let Some(id) = a.first().and_then(|x| x.as_str()) {
                        out.push(id.to_string());
                    }
                }
            }
        }
        out
    }

    /// 图里每一条引用都要指向真实存在的节点。
    /// **少一根线 ComfyUI 报的错很难懂**（"clip input is invalid: None"），
    /// 与其等运行时才发现，不如在这里炸
    fn assert_connected(g: &Value) {
        let ids: Vec<String> = g.as_object().unwrap().keys().cloned().collect();
        for r in refs_of(g) {
            assert!(ids.contains(&r), "引用了不存在的节点 {r}；现有 {ids:?}");
        }
        assert!(
            g.get(OUTPUT_NODE).is_some(),
            "出图节点 {OUTPUT_NODE} 必须存在"
        );
    }

    #[test]
    fn 不挂任何可选节点时图是连通的() {
        let s = expression("ckpt.safetensors", "1girl", "smiling", None, 1, "p");
        assert_connected(&to_workflow(&s));
    }

    #[test]
    fn 挂了lora和参考图之后图仍然连通() {
        let s = turnaround(
            SheetEngine::Sdxl,
            "ckpt.safetensors",
            "1girl",
            "dress",
            Some("CharTurnXL.safetensors".into()),
            Some("ref.png".into()),
            1,
            "p",
        );
        let g = to_workflow(&s);
        assert_connected(&g);
        // KSampler 必须吃到 IPAdapter 的输出，而不是绕过它直连 checkpoint
        assert_eq!(
            g["9"]["inputs"]["model"],
            json!(["5", 0]),
            "IPAdapter 被跳过了"
        );
        // 文本编码必须吃 LoRA 之后的 CLIP，否则触发词对 LoRA 不生效
        assert_eq!(g["6"]["inputs"]["clip"], json!(["2", 1]), "CLIP 没走 LoRA");
    }

    #[test]
    fn 只有lora没有参考图时ksampler接lora() {
        let s = turnaround(
            SheetEngine::Sdxl,
            "c",
            "1girl",
            "dress",
            Some("L.safetensors".into()),
            None,
            1,
            "p",
        );
        let g = to_workflow(&s);
        assert_connected(&g);
        assert_eq!(g["9"]["inputs"]["model"], json!(["2", 0]));
    }

    #[test]
    fn 只有参考图没有lora时clip直连checkpoint() {
        let s = expression("c", "1girl", "crying", Some("ref.png".into()), 1, "p");
        let g = to_workflow(&s);
        assert_connected(&g);
        assert_eq!(g["6"]["inputs"]["clip"], json!(["1", 1]));
        assert_eq!(g["9"]["inputs"]["model"], json!(["5", 0]));
    }

    #[test]
    fn 表情格的参考图权重必须低于全身格() {
        // 权重高了参考图的**表情**会被一起搬过来 —— 实测 0.85 时
        // 「温暖微笑」出来是面无表情，而同样提示词不挂参考图时「愤怒」很到位。
        // 压住表情的不是提示词太弱，是参考图的中性表情。
        let e = expression("c", "1girl", "smiling", Some("r.png".into()), 1, "p");
        let b = pose(
            "c",
            "1girl",
            "dress",
            "running",
            Some("r.png".into()),
            None,
            1,
            "p",
        );
        assert!(
            e.reference_weight < b.reference_weight,
            "表情格 {} 应低于全身格 {}",
            e.reference_weight,
            b.reference_weight
        );
        assert!(e.reference_weight <= 0.5, "表情格权重高于 0.5 会压住表情");
    }

    #[test]
    fn 权重真的写进了工作流() {
        // 早先这里写死 0.85，分档之后忘了改的话，测试都过、图照样错
        let e = expression("c", "1girl", "smiling", Some("r.png".into()), 1, "p");
        let g = to_workflow(&e);
        assert_eq!(g["5"]["inputs"]["weight"], json!(e.reference_weight));
    }

    #[test]
    fn pulid模式真的写进工作流() {
        let mut e = expression("c", "1girl", "smiling", Some("r.png".into()), 1, "p");
        e.identity_mode = IdentityMode::Pulid {
            file: "pulid_v1.1.safetensors".into(),
        };
        let g = to_workflow(&e);
        assert_connected(&g);
        assert_eq!(g["4"]["class_type"], json!("PulidModelLoader"));
        assert_eq!(g["22"]["class_type"], json!("ApplyPulid"));
        assert_eq!(g["9"]["inputs"]["model"], json!(["22", 0]));
    }

    #[test]
    fn 两个lora串联而不是并联() {
        // **各挂各的（都从 checkpoint 取输入）只会让后一个覆盖前一个** ——
        // 不报错，只是第一个 LoRA 悄悄失效。这里要求第二个的输入
        // 必须来自第一个的输出
        let mut s = turnaround(
            SheetEngine::Sdxl,
            "c",
            "1girl",
            "dress",
            Some("CharTurn.safetensors".into()),
            None,
            1,
            "p",
        );
        s.lora2 = Some(("Asian.safetensors".into(), 0.35));
        let g = to_workflow(&s);
        assert_eq!(
            g["16"]["inputs"]["model"],
            json!(["2", 0]),
            "第二个 LoRA 没接在第一个之后"
        );
        assert_eq!(g["16"]["inputs"]["clip"], json!(["2", 1]));
        // 下游必须吃到第二个的输出
        assert_eq!(
            g["9"]["inputs"]["model"],
            json!(["16", 0]),
            "KSampler 绕过了第二个 LoRA"
        );
        assert_eq!(
            g["6"]["inputs"]["clip"],
            json!(["16", 1]),
            "文本编码绕过了第二个 LoRA"
        );
    }

    #[test]
    fn 只有第二个lora时也要接上() {
        let mut s = expression("c", "1girl", "smiling", None, 1, "p");
        s.lora = None;
        s.lora2 = Some(("Asian.safetensors".into(), 0.35));
        let g = to_workflow(&s);
        assert_eq!(
            g["16"]["inputs"]["model"],
            json!(["1", 0]),
            "应直接接 checkpoint"
        );
        assert_eq!(g["9"]["inputs"]["model"], json!(["16", 0]));
    }

    #[test]
    fn 面孔lora权重必须低于版式lora() {
        // 两个 LoRA 叠加会互相削弱。面孔给高了会把 turnaround 的版式冲散 ——
        // 而版式正是主 LoRA 的全部价值
        assert!(
            ASIAN_LORA_WEIGHT < TURNAROUND_LORA_WEIGHT,
            "面孔 {ASIAN_LORA_WEIGHT} 应低于版式 {TURNAROUND_LORA_WEIGHT}"
        );
    }

    #[test]
    fn 三视图权重不能被调高() {
        // 0.8 实测是塑料人偶，0.7 开始发假。改高了这条会提醒你先看图
        assert!(
            TURNAROUND_LORA_WEIGHT <= 0.6,
            "权重高于 0.6 会把画风带成 3D 卡通"
        );
    }

    #[test]
    fn 写实词和反卡通词必须同时在() {
        let s = turnaround(
            SheetEngine::Sdxl,
            "c",
            "1girl",
            "dress",
            Some("L".into()),
            None,
            1,
            "p",
        );
        assert!(
            s.positive.contains("photorealistic"),
            "正向缺写实词，LoRA 会把画风拉走"
        );
        assert!(s.negative.contains("3d render"), "负向缺反 3D 词");
    }

    #[test]
    fn 从整排里挑三格不写死数量() {
        assert_eq!(pick_three(6), Some([0, 3, 5]));
        assert_eq!(pick_three(3), Some([0, 1, 2]));
        assert_eq!(pick_three(5), Some([0, 2, 4]));
        assert_eq!(pick_three(7), Some([0, 3, 6]));
        // 不够三个就没法装配，交给调用方重生
        assert_eq!(pick_three(2), None);
        assert_eq!(pick_three(0), None);
    }

    #[test]
    fn controlnet挂在conditioning上而不是model上() {
        // **这是最容易接错的地方。** 接到 model 上 ComfyUI 不报错，
        // 但姿势完全不起作用 —— 出图正常、约束没了，没有任何线索
        let mut s = pose("c", "1girl", "dress", "running", None, None, 1, "p");
        s.pose_control = Some(("pose_running.png".into(), 0.8, "cn.safetensors".into()));
        let g = to_workflow(&s);
        assert_eq!(
            g["9"]["inputs"]["positive"],
            json!(["15", 0]),
            "positive 没走 ControlNet"
        );
        assert_eq!(
            g["9"]["inputs"]["negative"],
            json!(["15", 1]),
            "negative 没走 ControlNet"
        );
        // model 不该被 ControlNet 改动
        assert_ne!(g["9"]["inputs"]["model"], json!(["15", 0]));
        // 骨架必须经过预处理器，不能把原图直接喂给 ControlNet
        assert_eq!(
            g["15"]["inputs"]["image"],
            json!(["13", 0]),
            "少了 OpenPose 预处理"
        );
        assert_eq!(g["13"]["inputs"]["image"], json!(["12", 0]));
    }

    #[test]
    fn 没有姿势参考图时完全不出现controlnet节点() {
        let s = pose("c", "1girl", "dress", "running", None, None, 1, "p");
        let g = to_workflow(&s);
        for id in ["12", "13", "14", "15"] {
            assert!(g.get(id).is_none(), "没有姿势图却插了节点 {id}");
        }
        assert_eq!(g["9"]["inputs"]["positive"], json!(["6", 0]));
    }

    #[test]
    fn 姿势强度不拉满() {
        // 1.0 会让人物僵硬地贴着骨架、失去自然的重心和衣褶
        assert!(POSE_CONTROL_STRENGTH < 1.0 && POSE_CONTROL_STRENGTH >= 0.6);
    }

    #[test]
    fn 面部机位是横排头像不是全身() {
        let s = face_angles(
            "c",
            "1girl",
            Some("FaceTurnXL.safetensors".into()),
            None,
            1,
            "p",
        );
        assert!(
            s.positive.contains("head and shoulders"),
            "机位图应是头肩特写"
        );
        assert!(s.negative.contains("full body"), "要排除全身");
        assert!(s.width > s.height, "整排应该是横构图");
        assert_eq!(FACE_ANGLE_TITLES.len(), 5, "界面写着 5 维机位");
    }

    #[test]
    fn 三档的尺寸各不相同() {
        // 分格图必须比整排切出来的一格大，否则分格就没意义了
        let sheet_slice_w = SHEET_SIZE.0 / 6;
        assert!(
            SLOT_SIZE.0 > sheet_slice_w * 2,
            "分格 {} 应显著大于整排切片 {}",
            SLOT_SIZE.0,
            sheet_slice_w
        );
    }
}

/// 一次生成的重试策略。
///
/// **每次重试必须换种子。** 同一个种子重跑得到的是同一张废片 ——
/// 扩散是确定性的，随机性全在种子里。这一点很容易写错，
/// 写错了表现为「重试了 3 次，3 次一模一样」。
#[derive(Debug, Clone, PartialEq)]
pub struct Attempt {
    pub index: u32,
    pub seed: i64,
}

/// 最多试几次。三次还不行多半是提示词或模型的问题，再试只是烧电
pub const MAX_ATTEMPTS: u32 = 3;

pub fn attempts(base_seed: i64) -> Vec<Attempt> {
    attempts_with_limit(base_seed, MAX_ATTEMPTS)
}

pub fn attempts_with_limit(base_seed: i64, max_attempts: u32) -> Vec<Attempt> {
    (0..max_attempts.max(1))
        .map(|i| Attempt {
            index: i,
            // 乘一个大质数再取模，避免相邻种子出的图高度相似
            seed: base_seed
                .wrapping_add(i as i64 * 7_919)
                .rem_euclid(1_000_000_007),
        })
        .collect()
}

#[cfg(test)]
mod engine_tests {
    use super::*;

    fn turn(engine: SheetEngine) -> Shot {
        turnaround(
            engine,
            "ckpt.safetensors",
            "1girl",
            "dress",
            Some("l.safetensors".into()),
            None,
            1,
            "p",
        )
    }

    /// SD1.5 出 1536×1024 会平铺出五个衣服各不相同的人 ——
    /// 必须按训练分辨率出，再放大。
    #[test]
    fn sd15走原生分辨率加两段放大() {
        let sh = turn(SheetEngine::Sd15);
        assert_eq!(
            (sh.width, sh.height),
            (832, 512),
            "SD1.5 超出训练分辨率会散架"
        );
        let h = sh.hires.expect("SD1.5 必须有第二段，否则每格只有 208×512");
        assert!(h.width > sh.width && h.height > sh.height);
        assert!(h.denoise <= 0.5, "denoise 高了第二段会重新构图，第一段白做");
    }

    /// SD1.5 对重复词很敏感 —— 实测多一个重复的 `full body` 就从
    /// 「四格含背面」退化成「三格无背面」。
    #[test]
    fn 提示词里没有重复的版式词() {
        for engine in [SheetEngine::Sdxl, SheetEngine::Sd15] {
            let pos = turn(engine).positive.to_ascii_lowercase();
            for word in ["full body", "simple background", "standing"] {
                assert!(
                    pos.matches(word).count() <= 1,
                    "{engine:?} 的提示词里「{word}」出现了 {} 次：{pos}",
                    pos.matches(word).count()
                );
            }
        }
    }

    /// **放大必须在图像空间做，不能在 latent 空间做。**
    ///
    /// latent 放大会留下重影：实测整排上每个人物旁边都有一层粉色的位移副本。
    /// latent 空间不是空间平滑的，插值出来的中间值不对应任何真实图像，
    /// 第二段采样就把它当成"那里本来有东西"。换插值方式（nearest-exact /
    /// bislerp）都一样。解码→放大像素→重编码之后重影完全消失。
    #[test]
    fn 放大走图像空间而不是latent() {
        let wf = to_workflow(&turn(SheetEngine::Sd15));
        let g = wf.as_object().unwrap();
        let kinds: Vec<&str> = g
            .values()
            .filter_map(|n| n["class_type"].as_str())
            .collect();
        assert!(kinds.contains(&"ImageScale"), "放大要在图像空间：{kinds:?}");
        assert!(kinds.contains(&"VAEEncode"), "放大完要重新编码回 latent");
        assert!(
            !kinds.contains(&"LatentUpscale"),
            "LatentUpscale 会留下位移重影，别用它：{kinds:?}"
        );
        // 图是连通的：最终解码必须取自第二段采样
        let last_decode = g
            .values()
            .filter(|n| n["class_type"] == "VAEDecode")
            .filter_map(|n| n["inputs"]["samples"][0].as_str().map(String::from))
            .collect::<Vec<_>>();
        assert!(
            last_decode.len() >= 2,
            "两段式应该有两个 VAEDecode：{last_decode:?}"
        );
    }

    #[test]
    fn sdxl一段出图不放大() {
        let sh = turn(SheetEngine::Sdxl);
        assert_eq!((sh.width, sh.height), SHEET_SIZE);
        assert!(sh.hires.is_none());
    }

    /// **两个 LoRA 认的触发词完全不同，写错了等于没挂。**
    #[test]
    fn 版式触发词跟着引擎走() {
        assert!(turn(SheetEngine::Sdxl).positive.contains("charturnersheet"));
        assert!(turn(SheetEngine::Sd15)
            .positive
            .contains("three sided view"));
        // 不能串味
        assert!(!turn(SheetEngine::Sd15).positive.contains("charturnersheet"));
        assert!(!turn(SheetEngine::Sdxl)
            .positive
            .contains("three sided view"));
    }

    /// **这是这套东西最要命的失败**：底模和 LoRA 不同族时 ComfyUI
    /// 照常跑完、不报错，只是版式 LoRA 完全不起作用。
    #[test]
    fn 底模和引擎必须同族() {
        assert!(SheetEngine::Sdxl.checkpoint_matches("RealVisXL_V5.0_fp16.safetensors"));
        assert!(!SheetEngine::Sdxl.checkpoint_matches("majicmixRealistic_v7.safetensors"));
        assert!(SheetEngine::Sd15.checkpoint_matches("majicmixRealistic_v7.safetensors"));
        assert!(!SheetEngine::Sd15.checkpoint_matches("AWPortrait_XL_ver1.1.safetensors"));
    }

    #[test]
    fn 各自挑得出同族底模() {
        let all: Vec<String> = [
            "AWPortrait_XL_ver1.1.safetensors",
            "RealVisXL_V5.0_fp16.safetensors",
            "majicmixRealistic_v7.safetensors",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(
            SheetEngine::Sd15.pick_checkpoint(&all).as_deref(),
            Some("majicmixRealistic_v7.safetensors")
        );
        // SDXL 那档要避开会盖版式的 AWPortrait
        assert_eq!(
            SheetEngine::Sdxl.pick_checkpoint(&all).as_deref(),
            Some("RealVisXL_V5.0_fp16.safetensors")
        );
    }

    /// LoRA 关键词不能重叠 —— 重叠了两条路会互相认错。
    #[test]
    fn 两条路的lora关键词互不命中() {
        for k in SheetEngine::Sd15.layout_lora_keywords() {
            assert!(
                !"charturnxlretag_v4.safetensors".contains(k),
                "SD1.5 关键词「{k}」认到了 SDXL 的 LoRA"
            );
        }
        for k in SheetEngine::Sdxl.layout_lora_keywords() {
            assert!(
                !"three_sided_view_sd15.safetensors".contains(k),
                "SDXL 关键词「{k}」认到了 SD1.5 的 LoRA"
            );
        }
    }

    /// SD1.5 的 LoRA 只有 144M，卡 300M 下限会把它全滤掉。
    #[test]
    fn sd15的lora不卡大小下限() {
        let dir = std::env::temp_dir().join("anima_lora_size_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("three_sided_view_sd15.safetensors"),
            vec![0u8; 4096],
        )
        .unwrap();

        assert!(
            find_lora_in(&dir, SheetEngine::Sd15.layout_lora_keywords()).is_none(),
            "卡大小的那个版本本来就该滤掉小文件"
        );
        assert_eq!(
            find_lora_in_any_size(&dir, SheetEngine::Sd15.layout_lora_keywords()).as_deref(),
            Some("three_sided_view_sd15.safetensors"),
            "SD1.5 那档必须找得到自己的小 LoRA"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn 引擎id认得出常见写法() {
        for id in ["sd15", "SD1.5", "majicmix"] {
            assert_eq!(SheetEngine::from_id(id), SheetEngine::Sd15, "{id}");
        }
        // 认不出的一律回到默认那条，不要 panic
        assert_eq!(SheetEngine::from_id("什么鬼"), SheetEngine::Sdxl);
    }
}

#[cfg(test)]
mod checkpoint_tests {
    use super::*;

    fn v(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// **这条守的是一次实测结论。**
    /// AWPortrait 出的脸更好看，很容易被当成「最好的底模」排到最前 ——
    /// 但它会把三视图的版式整个盖掉。单格档用它没问题，整排档不行。
    #[test]
    fn 三视图不选会盖版式的底模() {
        let all = v(&[
            "AWPortrait_XL_ver1.1.safetensors",
            "RealVisXL_V5.0_fp16.safetensors",
        ]);
        assert_eq!(
            pick_turnaround_checkpoint(&all).as_deref(),
            Some("RealVisXL_V5.0_fp16.safetensors"),
            "整排档选了 AWPortrait 会出成两张半身摆拍"
        );
        // 单格档反过来，就该选它
        assert_eq!(
            pick_portrait_checkpoint(&all).as_deref(),
            Some("AWPortrait_XL_ver1.1.safetensors")
        );
    }

    /// 只装了会盖版式的那个底模时说「没有」，由调用方兜底 ——
    /// 不要硬返回它，那等于默认出废图。
    #[test]
    fn 只有会盖版式的底模时返回none() {
        assert_eq!(
            pick_turnaround_checkpoint(&v(&["AWPortrait_XL_ver1.1.safetensors"])),
            None
        );
    }

    /// CharTurnXL 是 SDXL LoRA，挂到 SD1.5 底模上**不报错也不起作用**。
    #[test]
    fn 整排档优先sdxl而不是sd15() {
        let mix = v(&[
            "majicmixRealistic_v7.safetensors",
            "RealVisXL_V5.0_fp16.safetensors",
        ]);
        assert_eq!(
            pick_turnaround_checkpoint(&mix).as_deref(),
            Some("RealVisXL_V5.0_fp16.safetensors"),
            "majicmix 是 SD1.5，CharTurnXL 挂上去静默失效"
        );
    }

    /// 手指/解剖崩坏是这套图最常见的废图原因。
    #[test]
    fn 负向词覆盖解剖崩坏() {
        for want in ["bad anatomy", "bad hands", "extra fingers", "deformed"] {
            assert!(NEG_COMMON.contains(want), "负向词里缺 {want}");
        }
    }

    /// 全局画质词不许写族裔，也不许写肤质。
    ///
    /// 族裔写死在这儿就没法覆盖（用户写 caucasian 时正负两边打架），
    /// 肤质写死会把 65 岁那个人设的皱纹磨平 —— 而那正是她的特征。
    #[test]
    fn 画质词不含族裔和肤质() {
        let low = POS_REAL.to_ascii_lowercase();
        for bad in [
            "east asian",
            "chinese",
            "caucasian",
            "porcelain",
            "smooth skin",
            "young",
        ] {
            assert!(
                !low.contains(bad),
                "画质词里不该有「{bad}」—— 那是角色描述，归触发词管"
            );
        }
        assert!(low.contains("masterpiece"), "画质词权重丢了");
    }

    #[test]
    fn 人像底模优先于通用写实底模() {
        // **底模的数据集决定骨相，提示词改不动。**
        // 目录里第一个（文件名排序）和适不适合人像没关系
        let all = v(&[
            "AWPortrait_XL_ver1.1.safetensors",
            "RealVisXL_V5.0_fp16.safetensors",
        ]);
        assert_eq!(
            pick_portrait_checkpoint(&all).as_deref(),
            Some("AWPortrait_XL_ver1.1.safetensors")
        );
        // 顺序反过来也该挑同一个 —— 不能受目录顺序影响
        let mut reversed = all.clone();
        reversed.reverse();
        assert_eq!(
            pick_portrait_checkpoint(&reversed).as_deref(),
            Some("AWPortrait_XL_ver1.1.safetensors")
        );
    }

    #[test]
    fn 名单里一个都没有时用第一个() {
        let all = v(&["SomeOtherModel.safetensors"]);
        assert_eq!(
            pick_portrait_checkpoint(&all).as_deref(),
            Some("SomeOtherModel.safetensors")
        );
    }

    #[test]
    fn 空列表返回none() {
        assert_eq!(pick_portrait_checkpoint(&[]), None);
    }

    #[test]
    fn realvis排在最后() {
        // 它能用，只是在有更合适的时候不该被选中
        let all = v(&[
            "RealVisXL_V5.0_fp16.safetensors",
            "majicmixRealistic_v7.safetensors",
        ]);
        assert_eq!(
            pick_portrait_checkpoint(&all).as_deref(),
            Some("majicmixRealistic_v7.safetensors")
        );
    }
}

#[cfg(test)]
mod slot_tests {
    use super::*;

    #[test]
    fn 槽位数量和界面上标的一致() {
        // 界面写着「6 大表情模组 + 4 动作姿态」，对不上就是有一边改漏了
        assert_eq!(EXPRESSIONS.len(), 6);
        assert_eq!(POSES.len(), 4);
        assert_eq!(TURNAROUND_TITLES.len(), 3);
    }

    #[test]
    fn key不能重复() {
        // key 是存盘文件名，重了会互相覆盖 —— 表现是某一格永远显示成另一格
        let mut keys: Vec<&str> = EXPRESSIONS.iter().chain(POSES).map(|s| s.key).collect();
        keys.extend(TURNAROUND_TITLES.iter().map(|t| t.0));
        let n = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), n, "有重复的 key");
    }

    #[test]
    fn 每个槽位都有非空提示词() {
        for slot in EXPRESSIONS.iter().chain(POSES) {
            assert!(!slot.prompt.trim().is_empty(), "{} 缺提示词", slot.key);
            assert!(!slot.title.trim().is_empty(), "{} 缺标题", slot.key);
        }
    }

    #[test]
    fn 表情提示词进得去工作流() {
        // 抽一个验：提示词片段要真的出现在正向词里，
        // 拼错了的话六张表情会长得一模一样
        let slot = &EXPRESSIONS[3]; // 愤怒
        let shot = expression("c", "1girl", slot.prompt, None, 1, "p");
        assert!(shot.positive.contains(slot.prompt), "提示词没拼进去");
    }
}

#[cfg(test)]
mod discovery_tests {
    use super::*;
    use std::io::Write;

    fn fake_loras(files: &[(&str, u64)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("建临时目录");
        for (name, size) in files {
            let mut f = std::fs::File::create(dir.path().join(name)).unwrap();
            // 真写出字节 —— 判据看的是文件大小
            f.write_all(&vec![0u8; *size as usize]).unwrap();
        }
        dir
    }

    /// **不碰环境变量** —— 直接把目录传进去，测试之间互不干扰
    fn find_in(dir: &std::path::Path) -> Option<String> {
        find_lora_in(dir, &["charturn", "char_turn", "turnaround"])
    }

    const XL: u64 = 400 * 1024 * 1024;
    const SD15: u64 = 70 * 1024 * 1024;

    #[test]
    fn 认得出常见命名() {
        for name in [
            "CharTurnXLRetag_v4.safetensors",
            "char_turn_xl.safetensors",
            "MyTurnaround.safetensors",
        ] {
            let d = fake_loras(&[(name, XL)]);
            assert_eq!(
                find_in(d.path()).as_deref(),
                Some(name),
                "{name} 应该被认出"
            );
        }
    }

    #[test]
    fn 跳过sd15版() {
        // **同名的 SD1.5 版只有 70MB。** 挂到 SDXL 底模上不报错、
        // 也不起作用 —— 那是最难查的一种失败，所以按大小挡掉
        let d = fake_loras(&[("CharTurn_v4.safetensors", SD15)]);
        assert_eq!(find_in(d.path()), None, "SD1.5 尺寸的不该被选中");
    }

    #[test]
    fn 无关的lora不会被误选() {
        let d = fake_loras(&[
            ("add-detail-xl.safetensors", XL),
            ("AWPortrait-FL-lora.safetensors", XL),
        ]);
        assert_eq!(find_in(d.path()), None);
    }

    #[test]
    fn 多个候选时选最大的() {
        let d = fake_loras(&[
            ("charturn_small.safetensors", XL),
            ("charturn_big.safetensors", XL + 1024),
        ]);
        assert_eq!(
            find_in(d.path()).as_deref(),
            Some("charturn_big.safetensors")
        );
    }

    #[test]
    fn 目录不存在时不panic() {
        assert_eq!(find_in(std::path::Path::new("/nonexistent/loras")), None);
    }
}

#[cfg(test)]
mod retry_tests {
    use super::*;

    #[test]
    fn 每次重试的种子都不同() {
        let a = attempts(12345);
        assert_eq!(a.len(), MAX_ATTEMPTS as usize);
        let seeds: Vec<i64> = a.iter().map(|x| x.seed).collect();
        let mut uniq = seeds.clone();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(
            uniq.len(),
            seeds.len(),
            "种子重复了，重试会得到同一张废片：{seeds:?}"
        );
    }

    #[test]
    fn 种子始终是合法的非负数() {
        for base in [-1_i64, 0, i64::MAX, 999_999_999_999] {
            for a in attempts(base) {
                assert!(a.seed >= 0, "种子 {} 非法（base={base}）", a.seed);
                assert!(a.seed < 1_000_000_007);
            }
        }
    }

    #[test]
    fn 同一个基准种子结果可复现() {
        assert_eq!(attempts(42), attempts(42), "同样输入要给同样的重试序列");
    }
}

/// 一个槽位：key（存盘用）、界面标题、提示词片段。
pub struct Slot {
    pub key: &'static str,
    pub title: &'static str,
    pub prompt: &'static str,
}

/// 六个表情。**这份定义是唯一真相** —— 生成走它，切现成大图也走它。
///
/// 早先它只写在切片函数里，于是「生成」和「导入」两条路各有一份 key 列表，
/// 改一处漏一处，表现是同一个角色在两条路下产出的资产 key 对不上、界面串位。
pub const EXPRESSIONS: &[Slot] = &[
    Slot {
        key: "expr_smile",
        title: "微笑 (Joy)",
        prompt: "smiling joyfully, warm bright smile",
    },
    Slot {
        key: "expr_crying",
        title: "悲伤哭泣 (Sad)",
        prompt: "crying with tears, sorrowful",
    },
    Slot {
        key: "expr_shocked",
        title: "震惊张口 (Shock)",
        prompt: "shocked surprised expression, mouth open wide",
    },
    Slot {
        key: "expr_angry",
        title: "愤怒冷厉 (Angry)",
        prompt: "furious angry expression, furrowed brows",
    },
    Slot {
        key: "expr_pout",
        title: "委屈撒娇 (Pout)",
        prompt: "pouting lips, cute sulking, teary eyes",
    },
    Slot {
        key: "expr_calm",
        title: "冷酷从容 (Calm)",
        prompt: "cool composed expression, calm confident",
    },
];

/// 四个动作。
///
/// **这四个里有三个靠提示词控不住**（实测「奔跑」出来是站着被风吹）——
/// 它们需要 ControlNet 骨架。现在照样生成，但调用方要把这一点告诉用户，
/// 不要让人以为姿势被锁住了。
pub const POSES: &[Slot] = &[
    Slot {
        key: "pose_standing",
        title: "站姿对话 (Standing)",
        prompt: "standing gracefully, talking posture",
    },
    Slot {
        key: "pose_running",
        title: "快步奔跑 (Running)",
        prompt: "running dynamically, mid-stride, motion",
    },
    Slot {
        key: "pose_jumping",
        title: "喜悦跃起 (Jumping)",
        prompt: "jumping in the air with joy, energetic",
    },
    Slot {
        key: "pose_weeping",
        title: "掩面崩溃 (Weeping)",
        prompt: "covering face with both hands, despair",
    },
];

/// 五维面部机位的标题，顺序与整排从左到右一致。
/// 五维面部机位的 key 与标题。**这是唯一一份真相。**
///
/// 早先有两套：这一档用 `angle_front / angle_q45 / …`，
/// 而更早的「5 维面部锁脸机位」那个按钮用 `0_front / 1_left_45 / …`。
/// 两条路写进同一个界面槽位，key 却对不上 —— 于是全套生成出来的机位图
/// 存下来了、界面上显示的还是旧那批（直接裁原图的那种），
/// 看起来就是「面部机位用的是原图」。而且**一行报错都没有**。
///
/// 沿用后者那套 key：它已经在界面上、在磁盘文件名里、在用户的旧档案里。
/// `tests/asset_keys.rs` 盯着这份名单和界面一致。
pub const FACE_ANGLE_TITLES: &[(&str, &str)] = &[
    ("0_front", "0° 正面标准特写"),
    ("1_left_45", "左侧 45° 侧颜"),
    ("2_right_45", "右侧 45° 侧颜"),
    ("3_side_90", "90° 正侧面轮廓"),
    ("4_cinematic_tilt", "电影级微仰角"),
];

/// 三视图的三格标题，顺序与 [`pick_three`] 的返回一致。
/// **key 必须和界面、和切片函数用的一套一致** ——
/// 我最初写成 view_front/side/back，而界面和 cinema_slice_character_sheet
/// 都用 turnaround_*。对不上的表现是：生成明明成功了，界面上却是空的，
/// 而且不报任何错（它按 key 找不到就当没有）。
pub const TURNAROUND_TITLES: &[(&str, &str)] = &[
    ("turnaround_front", "正面全身三视图"),
    ("turnaround_side", "侧面全身三视图"),
    ("turnaround_back", "背面全身三视图"),
];

/// 挑一个最适合出人像的底模。
///
/// ## 为什么不能随便取一个
///
/// **底模的数据集决定骨相，提示词改不动。** `RealVisXL` 主要在欧美写实摄影上
/// 训练，触发词里写 `east asian` 能把肤色发色五官带过去，但眼窝深度、
/// 颧骨结构这些仍然偏欧美 —— 短剧要的东方长相到不了位。
///
/// 前端原先取 `checkpoints[0]`，也就是**目录里的第一个**，
/// 那是文件名排序的结果，和适不适合人像毫无关系。
///
/// 优先级按「对东亚面孔的先天理解」排。名单是显式的 ——
/// 不在名单里的底模照用，只是不会被优先挑中。
const PORTRAIT_CHECKPOINTS: &[&str] = &[
    "awportrait", // SDXL 微调，东方人像向
    "majicmix",   // 同上（SD1.5，作为兜底）
    "realvis",    // 欧美写实向，最后才选
];

/// 已经写明族裔/人种的词。**任何一个命中就不注入** ——
/// 用户明确要欧美角色时，我们不该偷偷把人换成东亚人。
const ETHNICITY_MARKERS: &[&str] = &[
    "asian",
    "chinese",
    "japanese",
    "korean",
    "taiwanese",
    "hongkong",
    "caucasian",
    "european",
    "western",
    "american",
    "white woman",
    "white man",
    "african",
    "black woman",
    "black man",
    "latina",
    "latino",
    "hispanic",
    "indian",
    "south asian",
    "middle eastern",
    "arab",
    "persian",
];

/// 默认族裔。这个项目是做中文短剧的，没写就按东亚人出。
const DEFAULT_ETHNICITY: &str = "east asian, chinese";

/// 触发词里没写族裔时补一个。
///
/// ## 为什么必须补
///
/// SDXL 底模的数据集偏欧美，触发词里**留白的地方由底模的偏置填**。
/// 实测：由美子的触发词是「1girl, long black silky hair, sharp confident gaze,
/// haute couture evening dress, diamond earrings」，一个族裔词都没有，
/// 出来的整排是明显的南亚/中东骨相。同一颗种子、只加上 `east asian, chinese`
/// 之后就是东亚面孔 —— 底模、LoRA、权重一个没动。
///
/// 内置人设的触发词里本来就写了 `east asian`，所以这个坑只有**用户自己写
/// 触发词**时才会踩到，而那正是最常见的用法。
///
/// ## 为什么不是「换底模」
///
/// 试过。AWPortrait_XL 确实能出东亚脸，但它是强风格的时尚人像底模，
/// 会把三视图 LoRA 的版式整个盖掉 —— 出来是两张半身摆拍，
/// 把版式 LoRA 加到 0.75 也压不住。换底模在三视图这一档不成立。
pub fn ensure_ethnicity(trigger: &str) -> String {
    let low = trigger.to_ascii_lowercase();
    if ETHNICITY_MARKERS.iter().any(|m| low.contains(m)) {
        return trigger.to_string();
    }
    if trigger.trim().is_empty() {
        return DEFAULT_ETHNICITY.to_string();
    }
    format!("{DEFAULT_ETHNICITY}, {trigger}")
}

/// 从可用底模里挑一个。`available` 是 ComfyUI 报上来的列表。
/// **会盖掉版式 LoRA 的底模。** 三视图那一档必须避开。
///
/// 实测（同一颗种子、同一份触发词，只换底模）：
///
/// | 底模 | 版式 LoRA 权重 | 结果 |
/// |---|---|---|
/// | RealVisXL | 0.55 | 六视角整排，含背面 ✓ |
/// | AWPortrait_XL | 0.55 | 两张半身摆拍 ✗ |
/// | AWPortrait_XL | 0.75 | 两张半身摆拍 ✗ |
///
/// AWPortrait 是强风格的时尚人像微调，构图先验压得过 CharTurnXL，
/// 把版式 LoRA 权重拉到 0.75 也压不住（再高就出塑料人偶了）。
/// 它在**单人单格**的表情/姿势/机位档很好用，只是不能用来排整排。
const LAYOUT_HOSTILE_CHECKPOINTS: &[&str] = &["awportrait"];

/// 三视图整排专用的底模优先级。
///
/// 和 [`pick_portrait_checkpoint`] 分开，是因为两者要的东西**相反**：
/// 单格要的是脸好看（风格越强越好），整排要的是**听话**（风格必须让位给版式）。
/// 早先只有一套优先级，把 AWPortrait 排在最前，结果三视图整排全崩成半身摆拍。
pub fn pick_turnaround_checkpoint(available: &[String]) -> Option<String> {
    let usable: Vec<String> = available
        .iter()
        .filter(|c| {
            let low = c.to_lowercase();
            !LAYOUT_HOSTILE_CHECKPOINTS.iter().any(|b| low.contains(b))
        })
        .cloned()
        .collect();
    // 整排要 SDXL —— CharTurnXL 是 SDXL LoRA，挂到 SD1.5 底模上不报错也不起作用
    for want in ["realvis", "xl"] {
        if let Some(hit) = usable.iter().find(|c| c.to_lowercase().contains(want)) {
            return Some(hit.clone());
        }
    }
    usable.first().cloned()
}

pub fn pick_portrait_checkpoint(available: &[String]) -> Option<String> {
    for want in PORTRAIT_CHECKPOINTS {
        if let Some(hit) = available.iter().find(|c| c.to_lowercase().contains(want)) {
            return Some(hit.clone());
        }
    }
    available.first().cloned()
}

/// 在 ComfyUI 的 loras 目录里找三视图 LoRA。
///
/// **按文件名认，不写死具体版本** —— 用户换个版本（v4 → v5）或者换一个同类
/// LoRA，不该还要改代码。找不到就返回 None，调用方退回纯提示词并明确告知。
///
/// 只认 SDXL 尺寸（>300MB）：同名的 SD1.5 版只有 70MB 左右，
/// 挂到 SDXL 底模上不报错、但完全不起作用 —— 那是最难查的一种失败。
pub fn find_turnaround_lora() -> Option<String> {
    find_layout_lora(SheetEngine::Sdxl)
}

/// 按引擎找版式 LoRA。
///
/// **两条路的 LoRA 不能互换，而且换错了不报错。** SDXL 的 CharTurnXL 是
/// 435M，SD1.5 的 three_sided_view 是 144M —— 挂错了 ComfyUI 照常跑完，
/// 只是那个 LoRA 完全不起作用，出来是一张普通单人像。
///
/// SDXL 那档还额外卡大小（>300MB），因为同名的 SD1.5 版只有 70M 左右；
/// SD1.5 这档不能卡，它本来就小。
pub fn find_layout_lora(engine: SheetEngine) -> Option<String> {
    let dir = loras_dir()?;
    match engine {
        SheetEngine::Sdxl => find_lora_in(&dir, engine.layout_lora_keywords()),
        SheetEngine::Sd15 => find_lora_in_any_size(&dir, engine.layout_lora_keywords()),
    }
}

/// 找亚洲人像 LoRA。
///
/// **由调用方决定挂不挂，构造函数不碰文件系统。**
/// 早先在 turnaround()/expression() 里直接调它，结果是同一份代码在
/// 装了这个 LoRA 的机器上和没装的机器上产出**不同的工作流** ——
/// 而且单元测试的结果取决于本机装了什么（实测三个测试因此变红）。
///
/// **底模的数据集决定骨相，提示词改不动** —— RealVisXL 在欧美写实摄影上训练，
/// 触发词写 east asian 能带过去肤色发色，但眼窝深度和颧骨结构仍偏欧美。
/// 换底模是根治，挂这个 LoRA 是在任何底模上都能补一层。
/// 按引擎找面孔增强 LoRA。
///
/// **LoRA 和底模不同族时不报错，只是完全不起作用。**
/// 早先这一档固定找 `Asian_Beauty XL`（SDXL，870M）——
/// SD1.5 引擎照样把它挂上去，ComfyUI 照常跑完，出来的脸还是底模自己的。
/// SD1.5 那条路要用 SD1.5 版的（本地是 `asianGirlsFace_v1`，151M）。
pub fn find_face_boost_lora(engine: SheetEngine) -> Option<String> {
    let dir = loras_dir()?;
    match engine {
        SheetEngine::Sdxl => find_lora_in(
            &dir,
            &[
                "asian_beauty",
                "asian-beauty",
                "asianbeauty",
                "asian beauty",
            ],
        ),
        // SD1.5 的这类 LoRA 只有 100~150M，不能卡 300M 下限
        SheetEngine::Sd15 => {
            find_lora_in_any_size(&dir, &["asiangirlsface", "asian_girls_face", "asianface"])
        }
    }
}

pub fn find_asian_lora() -> Option<String> {
    // 关键词要宽一点：C 站下下来的文件名带空格（`Asian_Beauty XL.safetensors`），
    // HF 镜像叫 `asian-beauty-xl.safetensors` —— 匹配前先转小写，
    // 分隔符按 _ / - / 空格三种都认
    find_lora_by(&[
        "asian_beauty",
        "asian-beauty",
        "asianbeauty",
        "asian beauty",
    ])
}

/// 找面部机位 LoRA。和三视图同构 —— 只是关键词和它不同，
/// **必须分开找**：两个都叫 *TurnXL，用一套关键词会互相认错，
/// 拿 FaceTurn 去出全身三视图只会得到一排头
pub fn find_face_lora() -> Option<String> {
    find_lora_by(&["faceturn", "face_turn"])
}

fn find_lora_by(keywords: &[&str]) -> Option<String> {
    find_lora_in(&loras_dir()?, keywords)
}

/// 在指定目录里找。**把目录当参数传** —— 早先靠环境变量指路，
/// 测试之间会互相串：`set_var` 是进程级的，并行跑时一个测试设的值
/// 被另一个测试读到，表现为「单独跑过、一起跑失败」
pub fn find_lora_in(dir: &std::path::Path, keywords: &[&str]) -> Option<String> {
    find_lora_sized(dir, keywords, 300 * 1024 * 1024)
}

/// 不卡大小的版本 —— 给 SD1.5 那档用，它的 LoRA 本来就只有 100M 出头。
pub fn find_lora_in_any_size(dir: &std::path::Path, keywords: &[&str]) -> Option<String> {
    find_lora_sized(dir, keywords, 0)
}

fn find_lora_sized(dir: &std::path::Path, keywords: &[&str], min_size: u64) -> Option<String> {
    let mut best: Option<(u64, String)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let lower = name.to_lowercase();
        if !lower.ends_with(".safetensors") {
            continue;
        }
        if !keywords.iter().any(|k| lower.contains(k)) {
            continue;
        }
        // **这里不能用 `?`** —— 单个文件读不到元数据就整个函数返回 None，
        // 等于后面的候选全被跳过。跳过这一个继续找才对
        let Ok(size) = entry.metadata().map(|m| m.len()) else {
            continue;
        };
        if size < min_size {
            continue; // 尺寸对不上的那一版，挂上去不报错也不起作用
        }
        if best.as_ref().is_none_or(|(s, _)| size > *s) {
            best = Some((size, name));
        }
    }
    best.map(|(_, n)| n)
}

pub(crate) fn loras_dir() -> Option<std::path::PathBuf> {
    if let Some(d) = std::env::var_os("ANIMA_COMFY_LORAS") {
        return Some(std::path::PathBuf::from(d));
    }
    let home = std::env::var("HOME").ok()?;
    [
        format!("{home}/AI/ComfyUI/models/loras"),
        format!("{home}/ComfyUI/models/loras"),
        format!("{home}/Documents/ComfyUI/models/loras"),
    ]
    .into_iter()
    .map(std::path::PathBuf::from)
    .find(|p| p.is_dir())
}

// ── 执行：提交 → 轮询 → 取图 → 质检 → 不合格换种子重来 ──────────────

use crate::cinema_qc::{check, Verdict};
use crate::error::{Error, Result};
use std::time::Duration;

/// ComfyUI 的地址。它是本机服务，端口写死
pub const COMFY_URL: &str = "http://127.0.0.1:8188";

/// 一次生成的结果，**带上过程**。
///
/// `notes` 不是调试信息，是要给用户看的：「重生了 2 次，原因是数出 4 个主体」
/// 比一句「生成成功」有用得多 —— 尤其当结果仍然不理想时，
/// 用户需要知道系统已经试过什么。
pub struct Rendered {
    pub bytes: Vec<u8>,
    pub attempts_used: u32,
    pub notes: Vec<String>,
}

/// 跑一格（或一整排），不合格就换种子重来。
///
/// `expect_subjects`：这张图里应该有几个主体。整排传 0 表示「不限个数」——
/// turnaround LoRA 出几个视角是不固定的，实测同样提示词出过 5 个也出过 6 个，
/// 卡死个数只会白白重生。
pub async fn render(
    http: &reqwest::Client,
    shot: Shot,
    expect_subjects: usize,
) -> Result<Rendered> {
    render_with_attempts(http, shot, expect_subjects, MAX_ATTEMPTS).await
}

pub async fn render_with_attempts(
    http: &reqwest::Client,
    mut shot: Shot,
    expect_subjects: usize,
    max_attempts: u32,
) -> Result<Rendered> {
    let mut notes = Vec::new();
    // 最后一次的图。**QC 三次都不过时把它交出去，而不是报错。**
    //
    // 这三张图是花了三分钟真跑出来的。全都丢掉、整格报失败，是这套流程里
    // 最贵的一种失败 —— 用户等完十分钟，界面上那一档是空的。
    // 而 QC 挡的多半是「质量不够好」而不是「完全不能用」：
    // 实测报「只数出 2 个视角，不够切三格」的那批，图里其实是三个视角，
    // 只是分割少数了一个。宁可交一张有瑕疵的图并**说清楚哪儿有瑕疵**，
    // 也不要交空白。
    let mut fallback: Option<(Vec<u8>, u32)> = None;

    let max_attempts = max_attempts.max(1);
    for attempt in attempts_with_limit(shot.seed, max_attempts) {
        shot.seed = attempt.seed;
        let bytes = submit_and_wait(http, &shot).await?;
        // 空白图不能当兜底 —— 那是真的不能用
        if !image::load_from_memory(&bytes)
            .map(|i| matches!(crate::cinema_qc::check(&i, 0), Verdict::Retry(ref w) if w.contains("空白")))
            .unwrap_or(true)
        {
            fallback = Some((bytes.clone(), attempt.index + 1));
        }

        if expect_subjects == 0 {
            // 整排的视角数不固定，不能按个数判 —— 改用**尺度一致性**：
            // 正经 turnaround 里每个视角都顶天立地，跑偏时会混进半身特写。
            // 光挡空白图是不够的：一张「4 个人、全是正面、两个巨大半身」的
            // 废图能过空白检查，切出来三格全是碎片
            match image::load_from_memory(&bytes) {
                Ok(img) => match crate::cinema_qc::check_turnaround(&img) {
                    Verdict::Ok => {
                        return Ok(Rendered {
                            bytes,
                            attempts_used: attempt.index + 1,
                            notes,
                        });
                    }
                    Verdict::Retry(why) => {
                        notes.push(format!("第 {} 次不合格：{why}", attempt.index + 1))
                    }
                },
                Err(e) => notes.push(format!("第 {} 次：解码失败 {e}", attempt.index + 1)),
            }
            continue;
        }

        let img = image::load_from_memory(&bytes)
            .map_err(|e| Error::runtime(format!("ComfyUI 返回的图解不开：{e}")))?;
        match check(&img, expect_subjects) {
            Verdict::Ok => {
                return Ok(Rendered {
                    bytes,
                    attempts_used: attempt.index + 1,
                    notes,
                });
            }
            Verdict::Retry(why) => {
                notes.push(format!("第 {} 次不合格：{why}", attempt.index + 1));
            }
        }
    }

    // 三次都没过 QC。交出最后一张能用的，并把每次的原因带上去 ——
    // 调用方会把 notes 塞进 warnings，界面上看得到
    match fallback {
        Some((bytes, attempts_used)) => {
            notes.insert(
                0,
                format!("试了 {max_attempts} 次都没完全达标，用的是最后一张 —— 下面是每次的问题"),
            );
            Ok(Rendered {
                bytes,
                attempts_used,
                notes,
            })
        }
        None => Err(Error::runtime(format!(
            "试了 {max_attempts} 次，出的都是空白图：{}",
            notes.join("；")
        ))),
    }
}

async fn submit_and_wait(http: &reqwest::Client, shot: &Shot) -> Result<Vec<u8>> {
    let payload = serde_json::json!({ "prompt": to_workflow(shot), "client_id": "anima-cinema" });
    let resp = http
        .post(format!("{COMFY_URL}/prompt"))
        .json(&payload)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| Error::runtime(format!("连不上本地 ComfyUI (:8188)：{e}")))?;

    if !resp.status().is_success() {
        // **把 ComfyUI 的原文带出来。** 它的报错（哪个节点、缺什么输入）
        // 是排查的唯一线索，吞掉之后只剩「生成失败」
        let body = resp.text().await.unwrap_or_default();
        return Err(Error::runtime(format!("ComfyUI 拒绝了这个工作流：{body}")));
    }

    let submitted: Value = resp
        .json()
        .await
        .map_err(|e| Error::runtime(format!("解析响应失败：{e}")))?;
    let prompt_id = submitted["prompt_id"]
        .as_str()
        .ok_or_else(|| Error::runtime("ComfyUI 没返回 prompt_id"))?
        .to_string();

    // SDXL 1536×1024 在 MPS 上实测约 40~90 秒，留足余量
    for _ in 0..180 {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let Ok(r) = http
            .get(format!("{COMFY_URL}/history/{prompt_id}"))
            .send()
            .await
        else {
            continue;
        };
        let Ok(h) = r.json::<Value>().await else {
            continue;
        };
        let Some(rec) = h.get(&prompt_id) else {
            continue;
        };

        if rec.pointer("/status/status_str").and_then(|v| v.as_str()) == Some("error") {
            let msg = rec
                .pointer("/status/messages")
                .and_then(|m| serde_json::to_string(m).ok())
                .unwrap_or_default();
            return Err(Error::runtime(format!("ComfyUI 执行报错：{msg}")));
        }

        if let Some(images) = rec
            .pointer(&format!("/outputs/{OUTPUT_NODE}/images"))
            .and_then(|v| v.as_array())
        {
            if let Some(first) = images.first() {
                let name = first["filename"].as_str().unwrap_or_default();
                let sub = first["subfolder"].as_str().unwrap_or_default();
                let kind = first["type"].as_str().unwrap_or("output");
                let bytes = http
                    .get(format!(
                        "{COMFY_URL}/view?filename={name}&subfolder={sub}&type={kind}"
                    ))
                    .send()
                    .await
                    .map_err(|e| Error::runtime(format!("下载出图失败：{e}")))?
                    .bytes()
                    .await
                    .map_err(|e| Error::runtime(format!("读取出图失败：{e}")))?
                    .to_vec();
                return Ok(bytes);
            }
        }
    }

    Err(Error::runtime("ComfyUI 生成超时（6 分钟没出图）"))
}
