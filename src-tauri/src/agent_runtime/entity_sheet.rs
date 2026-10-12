//! 完整设定表（entity sheet）—— Krea 2 + `DynamicCharacterSheet` LoRA 的图生图链路。
//!
//! 和 `layout: character_sheet` 那条**不是一回事**：
//!
//! | | `character_sheet`（旧） | `entity_sheet`（这里） |
//! |---|---|---|
//! | 引擎 | SDXL | Krea 2 Turbo |
//! | 输入 | 只有文字 | **必须有源图** |
//! | 产出 | 一排三视图 | 七个区（元数据/hero/转身/动作/剪影/表情/细节） |
//! | 脸的像素 | 42 px | hero 186 / 表情 178~189 / 细节六格 |
//!
//! 2026-09-08 实跑看过两个题目（小雨、大橘），图都确认过。
//!
//! # 这条路上四件写错也不报错的事
//!
//! 1. **`fit_mode: "fit"` 必须同时接 `vae` + `source_image`。** 只接
//!    `source_latent` 的话节点会打印告警然后退回 legacy 的 crop 几何 ——
//!    任务照样 `success`，只是参考图的对齐方式和训练时不一致。
//! 2. **`target_latent` 要接采样器用的同一个 latent。** 不接的话 VAE 在
//!    **采样中途**才被拉进显存，会把扩散模型挤出去，之后每一步都从 CPU 流权重。
//! 3. **负向也要走 `Krea2EditGroundedEncode` 并接同一张源图**（对应训练时的
//!    无条件分支），不是 `CLIPTextEncode`。
//! 4. **两个节点来自 `comfyui-krea2edit`，不是 ComfyUI 核心。** 缺了必须
//!    明确报错 —— 静默退回普通文生图会出一张没有版式的图，而它看起来是「成功」。
//!
//! # 已知能力边界（实测，别指望）
//!
//! - **元数据列的正文是涂鸦。** 短标题（`EXPRESSION STUDY` / `DETAIL STUDY`）
//!   渲染正确，正文和专有名词不行（写 `Xiaoyu` 出 `Xiayu`）。
//! - **表情研究三张脸几乎一样** —— 尺寸够（178~189 px）但变化没做出来。
//!   2026-09-15 起出完之后自动补表情（`crate::sheet_expressions`），每张多 5 分钟左右。
//! - **转身那排只有 104~115 px**，低于可用下限（150~200）。要转身参考用
//!   `QuadView`，不要用这个。
//! - 源图上有中文标签时会渗进输出（提示词里写了 English only 也拦不住）。

use serde_json::{json, Map, Value};

use super::image_engine::{KREA2_CLIP, KREA2_CLIP_TYPE, KREA2_UNET, KREA2_VAE};

/// 这条链路能出的两种版式。**同一套节点，换 LoRA + 换触发句 + 换采样器。**
///
/// 分成两种是因为它们的用途正交，不是「一个更好」：
///
/// | | `Quad`（四视图） | `Dynamic`（七分区） |
/// |---|---|---|
/// | 出什么 | 大脸特写 + 正/侧/背三全身 | 元数据/hero/转身/动作/剪影/表情/六格细节 |
/// | 脸的像素 | **特写 486 / 转身 135~199** | hero 186、**转身只有 110** |
/// | 耗时 | **307s** | 450s |
/// | 拿来干嘛 | **身份参考**（R2V / 分镜首帧 / 封面） | 表情和细节的取材 |
///
/// **转身那排的像素差是关键**：可用下限 150~200 px，`Dynamic` 的转身在下限
/// 以下，所以它不能当身份参考 —— 那正是 `Quad` 补的位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SheetKind {
    /// 四视图：一格大脸特写 + 正 / 侧 / 背三个全身。
    Quad,
    /// 七分区完整参考表。
    Dynamic,
}

impl SheetKind {
    pub fn id(self) -> &'static str {
        match self {
            Self::Quad => "quad_view",
            Self::Dynamic => "entity_sheet",
        }
    }

    pub fn from_id(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "quad_view" | "quadview" | "quad" => Some(Self::Quad),
            "entity_sheet" | "dynamic" => Some(Self::Dynamic),
            _ => None,
        }
    }

    /// **两种版式各有自己的 LoRA，都只有 Krea 2 版。**
    pub fn lora(self) -> &'static str {
        match self {
            Self::Quad => "krea/QuadView_krea2_v1.safetensors",
            Self::Dynamic => "krea/DynamicCharacterSheet_krea2_v1.safetensors",
        }
    }

    /// **采样器不一样，别抄错。**
    ///
    /// 作者实测：`Dynamic` 用 LCM 时图上文字最好；`Quad` 官方参数是 euler。
    /// 抄错不报错，只是版面质量掉一档。
    pub fn sampler(self) -> &'static str {
        match self {
            Self::Quad => "euler",
            Self::Dynamic => "lcm",
        }
    }

    /// 大概要等多久。**实测值**（Quad 306.6s / Dynamic 450~457s）。
    pub fn secs(self) -> u64 {
        match self {
            Self::Quad => 307,
            Self::Dynamic => 455,
        }
    }
}

/// 四视图的触发句。**照作者 README 的原文，一个字都别改** ——
/// 这是训练时的 caption，改了版式就不稳。
pub const QUAD_TRIGGER: &str = "Convert the character in the image to a Character Sheet \
showing a face close-up, front full body, side full body and back full body views";

/// 这条路必须有、而 ComfyUI 核心**不提供**的两个节点。
///
/// 来自注册表上的 `comfyui-krea2edit`（零 pip 依赖，Mac 上没有 CUDA 阻塞点）。
pub const REQUIRED_NODES: &[&str] = &["Krea2EditModelPatch", "Krea2EditGroundedEncode"];

pub const SHEET_SCHEDULER: &str = "simple";
pub const SHEET_STEPS: u32 = 10;
/// 默认画幅。**必须横幅** —— 七个区是按 3:2 排的。
pub const SHEET_WIDTH: u32 = 1536;
pub const SHEET_HEIGHT: u32 = 1024;
/// 源图先缩到这个方形再编码（作者 workflow 的做法）。
pub const SOURCE_SIDE: u32 = 1024;

/// 参考图保真度旋钮的默认值。
///
/// **2026-09-08 实测：1.0 → 4.0 改善的不是身份，是构图。** 两张大特写并排
/// 几乎一样；真正变好的是「动作三姿从近乎相同的俯视变成真正不同的姿态」，
/// 细节格还多出一格手，`FRONT` 标签也从 `TURNAROND` 变对了。
/// 所以它更像**指令服从度**旋钮，README 说的「人脸相似度」在我们的题目上没体现。
/// 默认取 4.0；作者提醒 >10 会让「删除/替换」类编辑开始失败。
pub const REF_BOOST_DEFAULT: f64 = 4.0;
pub const REF_BOOST_MAX: f64 = 10.0;

/// 实体类型。取自 LoRA 作者的模板，**不能自己造** —— 训练时就是这七个词。
pub const ENTITY_TYPES: &[&str] = &[
    "HUMAN",
    "STYLIZED_CHARACTER",
    "ANIMAL",
    "CREATURE",
    "ROBOT",
    "VEHICLE",
    "OBJECT",
];

/// 归一化实体类型：认不出的一律当 `STYLIZED_CHARACTER`。
///
/// **保守方向**：那是训练分布的正中间，比塞一个模型没见过的词安全。
pub fn normalize_entity_type(raw: &str) -> &'static str {
    let up = raw.trim().to_uppercase().replace(['-', ' '], "_");
    ENTITY_TYPES
        .iter()
        .find(|t| **t == up)
        .copied()
        .unwrap_or("STYLIZED_CHARACTER")
}

/// 把名字压成 `[ENTITY_ID: ...]` 能用的 slug。
///
/// 只留 ASCII 字母数字和下划线 —— 中文名会被整段丢掉，所以空了就用类型兜底。
pub fn entity_id(entity_type: &str, name: &str) -> String {
    let mut slug = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('_') && !slug.is_empty() {
            slug.push('_');
        }
    }
    let slug = slug.trim_matches('_').to_string();
    let kind = entity_type.to_lowercase();
    if slug.is_empty() {
        format!("{kind}_subject")
    } else {
        format!("{kind}_{slug}")
    }
}

/// 缺哪些节点。空 = 齐了。
///
/// `available` 是 ComfyUI `/object_info` 报的节点名。
pub fn missing_nodes(available: &[String]) -> Vec<&'static str> {
    REQUIRED_NODES
        .iter()
        .filter(|n| !available.iter().any(|a| a == *n))
        .copied()
        .collect()
}

/// 缺节点时给用户看的话。**要说清装什么、以及为什么不能凑合跑。**
pub fn missing_nodes_message(missing: &[&str]) -> String {
    format!(
        "完整设定表需要 ComfyUI 节点包 `comfyui-krea2edit`，当前缺 {}。\
         没有它这条路会退化成普通文生图 —— 出来一张没有版式的图，而任务显示「成功」，\
         所以这里直接停下。装法：ComfyUI Manager 搜 `Krea 2 Identity Edit`，\
         或 `git clone https://github.com/lbouaraba/comfyui-krea2edit` 到 \
         `custom_nodes/` 后重启 ComfyUI（它没有 pip 依赖）。",
        missing.join(" / ")
    )
}

/// 把一段普通描述切成身份锚点。
///
/// **存在的理由是「不许静默丢掉调用方写的东西」。** `layout: entity_sheet` 会
/// 用结构化模板**替换**整段提示词 —— 如果调用方（尤其是对话里的 Agent）
/// 在 `prompt` 里写了角色描述却没单独给 `identity_locks`，那段描述就凭空消失了，
/// 而图照出、不报错，只是七个区之间没有锚点可依、开始互相漂。
///
/// 切法和界面侧 `entitySheet.ts` 的 `locksFromDescription` 一致（有测试比对）：
/// 中英标点一起切、丢掉「无/未知」这类占位、上限 8 条
///（作者模板写的是 4~7 条，多了会开始互相矛盾）。
pub fn locks_from_text(text: &str) -> Vec<String> {
    text.split(['，', ',', '、', '；', ';', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter(|s| {
            let low = s.to_lowercase();
            !matches!(
                low.as_str(),
                "无" | "没有" | "未知" | "不详" | "none" | "n/a" | "na" | "-"
            )
        })
        .take(8)
        .map(str::to_string)
        .collect()
}

/// 这段提示词已经是结构化模板了吗。
///
/// 是的话原样用 —— Agent 想自己写那段方括号结构时不该被覆盖。
pub fn is_structured(prompt: &str) -> bool {
    prompt.contains("[TASK: ENTITY_SHEET_GENERATION]")
}

/// 结构化提示词。**格式是训练时定的，不能自由发挥。**
///
/// `identity_locks` 是这段里唯一需要按角色写的部分，而且**必须具体可验**
/// （「额头 M 纹加三道竖纹」「垂颊」「尾巴有深色环」）。写成「有特色的外形」
/// 这种笼统说法的话，各视角之间会漂 —— 和 `assetPrompt` 里
/// 「表情写成『不同的表情』就四格一样」是同一个坑。
pub fn sheet_prompt(
    name: &str,
    entity_type: &str,
    identity_locks: &[String],
    style_line: &str,
) -> String {
    let ty = normalize_entity_type(entity_type);
    let id = entity_id(ty, name);
    let display = if name.trim().is_empty() { "Subject" } else { name.trim() };

    let mut locks = String::new();
    for l in identity_locks {
        let l = l.trim();
        if !l.is_empty() {
            locks.push_str("- ");
            locks.push_str(l);
            locks.push('\n');
        }
    }
    locks.push_str("- Keep the subject from Image 1 recognizable in every view.\n");
    locks.push_str(
        "- Keep handedness, asymmetry, markings, costume, hardware, and color placement consistent.\n",
    );
    let style = style_line.trim();
    if !style.is_empty() {
        locks.push_str("- ");
        locks.push_str(style);
        locks.push('\n');
    } else {
        locks.push_str("- ");
        locks.push_str(DEFAULT_STYLE_LINE);
        locks.push('\n');
    }

    format!(
        "[TASK: ENTITY_SHEET_GENERATION]\n\
         [TEMPLATE: MULTI_ANGLE_ENTITY_SHEET_V1]\n\
         [ENTITY_TYPE: {ty}]\n\
         [ENTITY_ID: {id}]\n\
         \n\
         Convert the subject in Image 1 into one standardized advanced Character Sheet. \
         Image 1 is the sole identity and design reference for {display}. Preserve the exact \
         same design, proportions, colors, materials, markings, surface wear, and every \
         signature detail. Do not redesign, beautify, age-shift, or simplify it.\n\
         \n\
         FIXED LANDSCAPE SHEET FORMAT\n\
         Use a clean 3:2 landscape canvas with a warm off-white paper background and generous \
         white space. Organize one coherent sheet with these fixed zones:\n\
         1. LEFT METADATA COLUMN: the exact name \"{display}\", ENTITY TYPE, CORE MOOD, and \
         VISUAL SIGNATURE in compact readable English.\n\
         2. LARGE CENTER HERO VIEW: one dominant full-body or complete-object three-quarter view.\n\
         3. TOP-RIGHT TURNAROUND ROW: neutral FRONT FULL BODY, SIDE FULL BODY, and BACK FULL \
         BODY views at matching scale, with all extremities visible.\n\
         4. MID-RIGHT ACTION POSES: three readable views including a low angle, an overhead or \
         high angle, and one characteristic action or operating state.\n\
         5. BOTTOM-LEFT SILHOUETTE STUDY: three solid black silhouettes that preserve the \
         design shape.\n\
         6. BOTTOM-CENTER EXPRESSION STUDY: one clear neutral face close-up plus three compact \
         expression, state, or functional studies showing meaningful variation without changing \
         identity or construction.\n\
         7. BOTTOM-RIGHT DETAIL STUDY: six close-up crops of the most identity-critical \
         features, materials, joints, face, markings, controls, or accessories.\n\
         \n\
         IDENTITY LOCKS\n\
         {locks}\
         \n\
         VISIBLE TEXT\n\
         All labels must be English only. Keep labels short and legible. Do not invent lore \
         paragraphs.\n\
         \n\
         NEGATIVE CONSTRAINTS\n\
         No extra character or object, identity drift, species drift, wardrobe drift, material \
         drift, inconsistent markings, changed proportions, missing extremities, duplicated \
         limbs, merged views, overlapping panels, logo, watermark, or unrelated decoration."
    )
}

/// 没给风格句时用这个。
///
/// **这个项目是 AI 漫剧，默认就该是动漫赛璐璐** —— 而且「sharp linework」
/// 那半句实测有用：不写的话七个区之间的线条粗细会飘。
pub const DEFAULT_STYLE_LINE: &str =
    "Use clean anime cel-shaded illustration style with sharp linework throughout the sheet.";

/// 从工具参数拼出这次要发的结构化提示词。
///
/// **整块提出来是为了能真的测到。** 之前这段逻辑写在 `image_tool` 那个
/// 1200 行的 `execute` 里，而测试只测了 `locks_from_text` 这个纯函数 ——
/// 变异测试当场证明了那是假绿：把调用点的兜底摘掉，测试照样全过。
/// 钉符号不等于钉调用点。
///
/// 规则（顺序有意义）：
/// 1. `prompt` 已经是结构化模板 → 原样用（Agent 想自己写方括号结构时不该被覆盖）
/// 2. 否则按零件拼；`identity_locks` 没给就**从 `prompt` 里切**
///    —— 模板会替换整段提示词，不切的话调用方写的描述会凭空消失
pub fn sheet_prompt_from_args(kind: SheetKind, prompt: &str, args: &Value) -> String {
    // **四视图不吃结构化模板。** 它的训练 caption 就是那一句固定触发句，
    // 塞一大段方括号结构进去反而让版式不稳 —— 作者 README 写得很明确：
    // TripleView / QuadView 用短触发句，只有 DynamicCharacterSheet 用模板。
    if kind == SheetKind::Quad {
        return QUAD_TRIGGER.to_string();
    }
    if is_structured(prompt) {
        return prompt.to_string();
    }
    let field = |k: &str| {
        args.get(k)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };
    let mut locks: Vec<String> = args
        .get("identity_locks")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    if locks.is_empty() {
        locks = locks_from_text(prompt);
    }
    sheet_prompt(
        field("entity_name").unwrap_or("Subject"),
        field("entity_type").unwrap_or("STYLIZED_CHARACTER"),
        &locks,
        field("style_line").unwrap_or(DEFAULT_STYLE_LINE),
    )
}

/// 负向那一句。**照作者 workflow 的原文** —— 它也要过 `GroundedEncode`。
pub const SHEET_NEGATIVE: &str = "imperfect text, bad text, blur";

fn node(class: &str, inputs: Value) -> Value {
    json!({"class_type": class, "inputs": inputs})
}

/// 建图。`source` 是 ComfyUI `input/` 下的文件名（`LoadImage` 吃的就是文件名）。
///
/// 四处不能改的接线见模块注释。节点 id 固定，`save_node_id` 那边靠它取图。
#[allow(clippy::too_many_arguments)]
pub fn build_entity_sheet_graph(
    kind: SheetKind,
    source: &str,
    prompt: &str,
    ref_boost: f64,
    width: u32,
    height: u32,
    seed: u64,
    steps: u32,
) -> Value {
    let boost = ref_boost.clamp(0.0, REF_BOOST_MAX);
    let mut m = Map::new();
    m.insert("1".into(), node("UNETLoader",
        json!({"unet_name": KREA2_UNET, "weight_dtype": "default"})));
    m.insert("2".into(), node("LoraLoaderModelOnly", json!({
        "model": ["1", 0], "lora_name": kind.lora(), "strength_model": 1.0
    })));
    m.insert("3".into(), node("CLIPLoader", json!({
        "clip_name": KREA2_CLIP, "type": KREA2_CLIP_TYPE, "device": "default"
    })));
    m.insert("4".into(), node("VAELoader", json!({"vae_name": KREA2_VAE})));
    m.insert("5".into(), node("LoadImage", json!({"image": source})));
    // 源图先缩到方形再编码 —— 作者 workflow 的做法
    m.insert("6".into(), node("ImageResizeKJv2", json!({
        "image": ["5", 0], "width": SOURCE_SIDE, "height": SOURCE_SIDE,
        "upscale_method": "lanczos", "keep_proportion": "resize", "pad_color": "0, 0, 0",
        "crop_position": "center", "divisible_by": 2, "device": "cpu"
    })));
    m.insert("7".into(), node("VAEEncode", json!({"pixels": ["6", 0], "vae": ["4", 0]})));
    m.insert("8".into(), node("EmptySD3LatentImage",
        json!({"width": width, "height": height, "batch_size": 1})));
    // 正负都过 GroundedEncode，都接源图（见模块注释第 3 条）
    m.insert("9".into(), node("Krea2EditGroundedEncode", json!({
        "clip": ["3", 0], "prompt": prompt, "image": ["6", 0],
        "grounding_px": 0, "system_prompt": ""
    })));
    m.insert("10".into(), node("Krea2EditGroundedEncode", json!({
        "clip": ["3", 0], "prompt": SHEET_NEGATIVE, "image": ["6", 0],
        "grounding_px": 0, "system_prompt": ""
    })));
    // `fit_mode: fit` 要 vae + source_image 都在；`target_latent` 要和采样器同一个
    m.insert("11".into(), node("Krea2EditModelPatch", json!({
        "model": ["2", 0], "source_latent": ["7", 0],
        "ref_boost": boost, "ref_boost_a": 1.0, "fit_mode": "fit",
        "vae": ["4", 0], "source_image": ["6", 0], "target_latent": ["8", 0]
    })));
    m.insert("12".into(), node("KSampler", json!({
        "seed": seed, "steps": steps, "cfg": 1.0,
        "sampler_name": kind.sampler(), "scheduler": SHEET_SCHEDULER, "denoise": 1.0,
        "model": ["11", 0], "positive": ["9", 0], "negative": ["10", 0],
        "latent_image": ["8", 0]
    })));
    m.insert("13".into(), node("VAEDecode", json!({"samples": ["12", 0], "vae": ["4", 0]})));
    m.insert("14".into(), node("SaveImage",
        json!({"images": ["13", 0], "filename_prefix": "anima_img"})));
    Value::Object(m)
}

/// 取图时按这个 id 去 outputs 里找。
pub const SAVE_NODE_ID: &str = "14";

#[cfg(test)]
mod tests {
    use super::*;

    /// **四件写错也不报错的事**，一条一条钉。
    ///
    /// 这四条全是「任务 success、图也出来了、只是不对」的形状 ——
    /// 没有测试盯着的话，改坏了要靠肉眼比图才发现。
    #[test]
    fn 设定表图里四件写错也不报错的事() {
        let g = build_entity_sheet_graph(SheetKind::Dynamic, "src.png", "P", 4.0, 1536, 1024, 7, 10);
        let patch = &g["11"]["inputs"];

        // 1. fit_mode=fit 必须同时有 vae 和 source_image，否则退回 legacy 几何
        assert_eq!(patch["fit_mode"], "fit");
        assert!(patch["vae"].is_array(), "fit_mode=fit 没接 vae —— 会退回 legacy crop");
        assert!(
            patch["source_image"].is_array(),
            "fit_mode=fit 没接 source_image —— 会退回 legacy crop"
        );

        // 2. target_latent 必须和采样器吃的是同一个 latent 节点
        assert_eq!(
            patch["target_latent"], g["12"]["inputs"]["latent_image"],
            "target_latent 和采样器的 latent 不是同一个 —— VAE 会在采样中途抢显存"
        );

        // 3. 负向走 GroundedEncode 且接了源图，不是 CLIPTextEncode
        let neg = g["12"]["inputs"]["negative"][0].as_str().expect("负向要是节点引用");
        assert_eq!(
            g[neg]["class_type"], "Krea2EditGroundedEncode",
            "负向没走 GroundedEncode —— 和训练时的无条件分支不一致"
        );
        assert!(g[neg]["inputs"]["image"].is_array(), "负向没接源图");

        // 4. LoRA 挂在 UNet 之后、进 patch 之前，强度 1.0
        assert_eq!(g["2"]["class_type"], "LoraLoaderModelOnly");
        assert_eq!(g["2"]["inputs"]["lora_name"], SheetKind::Dynamic.lora());
        assert_eq!(g["2"]["inputs"]["strength_model"], 1.0);
        assert_eq!(patch["model"], json!(["2", 0]), "patch 没接在 LoRA 之后");
    }

    /// 采样参数：LCM + simple + 10 步 + cfg 1。**作者实测这一组文字最好。**
    #[test]
    fn 采样参数照作者的实测值() {
        let g = build_entity_sheet_graph(SheetKind::Dynamic, "s.png", "P", 4.0, 1536, 1024, 1, SHEET_STEPS);
        let k = &g["12"]["inputs"];
        assert_eq!(k["sampler_name"], "lcm");
        assert_eq!(k["scheduler"], "simple");
        assert_eq!(k["steps"], 10);
        assert_eq!(k["cfg"], 1.0);
        assert_eq!(k["denoise"], 1.0);
    }

    /// `save_node_id` 指错 = 「任务成功但找不到图」。
    #[test]
    fn save_node_指向真的saveimage() {
        let g = build_entity_sheet_graph(SheetKind::Dynamic, "s.png", "P", 4.0, 1536, 1024, 1, 10);
        assert_eq!(g[SAVE_NODE_ID]["class_type"], "SaveImage");
    }

    /// `ref_boost` 要被夹住。作者提醒 >10 会让编辑类操作开始失败。
    #[test]
    fn ref_boost_被夹在上限内() {
        let g = build_entity_sheet_graph(SheetKind::Dynamic, "s.png", "P", 999.0, 1536, 1024, 1, 10);
        assert_eq!(g["11"]["inputs"]["ref_boost"], REF_BOOST_MAX);
        // 负数也不该穿过去
        let g = build_entity_sheet_graph(SheetKind::Dynamic, "s.png", "P", -3.0, 1536, 1024, 1, 10);
        assert_eq!(g["11"]["inputs"]["ref_boost"], 0.0);
        // 默认值是实测选出来的那个
        assert_eq!(REF_BOOST_DEFAULT, 4.0);
    }

    /// **缺节点必须报出来，不能静默跑。**
    ///
    /// 这是这条路最危险的失败形状：`comfyui-krea2edit` 没装时，如果我们
    /// 退回普通文生图，会出一张**没有版式的普通图**，而任务是 `success`。
    #[test]
    fn 缺节点时报得出来且说清怎么装() {
        let none: Vec<String> = vec![];
        let miss = missing_nodes(&none);
        assert_eq!(miss.len(), 2, "两个节点都缺时要都报出来");

        let msg = missing_nodes_message(&miss);
        assert!(msg.contains("comfyui-krea2edit"), "没说要装哪个包");
        assert!(msg.contains("Krea2EditModelPatch"), "没说缺哪个节点");
        // **必须说清为什么不能凑合跑** —— 否则用户会觉得我们多事
        assert!(msg.contains("成功"), "没说清静默退化的后果");

        // 齐了就是空
        let all: Vec<String> = REQUIRED_NODES.iter().map(|s| s.to_string()).collect();
        assert!(missing_nodes(&all).is_empty());
        // 只有一个也要报出缺的那个
        let one = vec!["Krea2EditModelPatch".to_string()];
        assert_eq!(missing_nodes(&one), vec!["Krea2EditGroundedEncode"]);
    }

    /// 提示词的骨架不能少 —— 少一段模型就不按那个版式排。
    #[test]
    fn 提示词带齐模板要求的每一段() {
        let p = sheet_prompt(
            "小雨",
            "animal",
            &["short black bob".into(), "amber eyes".into()],
            "clean anime cel-shaded illustration style",
        );
        for must in [
            "[TASK: ENTITY_SHEET_GENERATION]",
            "[TEMPLATE: MULTI_ANGLE_ENTITY_SHEET_V1]",
            "[ENTITY_TYPE: ANIMAL]",
            "[ENTITY_ID: animal_",
            "FIXED LANDSCAPE SHEET FORMAT",
            "IDENTITY LOCKS",
            "VISIBLE TEXT",
            "NEGATIVE CONSTRAINTS",
        ] {
            assert!(p.contains(must), "提示词缺 {must}");
        }
        // 七个区一个都不能漏
        for zone in [
            "LEFT METADATA COLUMN",
            "LARGE CENTER HERO VIEW",
            "TOP-RIGHT TURNAROUND ROW",
            "MID-RIGHT ACTION POSES",
            "BOTTOM-LEFT SILHOUETTE STUDY",
            "BOTTOM-CENTER EXPRESSION STUDY",
            "BOTTOM-RIGHT DETAIL STUDY",
        ] {
            assert!(p.contains(zone), "少了分区 {zone}");
        }
        // 传进来的 identity locks 要真的进去
        assert!(p.contains("- short black bob"));
        assert!(p.contains("- amber eyes"));
        // 风格那句也进 locks
        assert!(p.contains("clean anime cel-shaded"));
        // 英文标签那条规则不能丢 —— 中文标注是这个模型的短板
        assert!(p.contains("English only"));
    }

    /// 实体类型认不出时**不能把原词塞进去**。
    ///
    /// 模型只见过那七个词。塞一个没见过的进去不报错，只是版式开始飘。
    #[test]
    fn 实体类型只认那七个别的回落到通用() {
        assert_eq!(normalize_entity_type("HUMAN"), "HUMAN");
        assert_eq!(normalize_entity_type("  animal "), "ANIMAL");
        assert_eq!(normalize_entity_type("stylized-character"), "STYLIZED_CHARACTER");
        assert_eq!(normalize_entity_type("stylized character"), "STYLIZED_CHARACTER");
        // 认不出的一律回落，而且**不是**原样塞进去
        for bad in ["猫", "monster", "", "CAT_PERSON"] {
            let t = normalize_entity_type(bad);
            assert!(ENTITY_TYPES.contains(&t), "{bad} 回落到了名单外的 {t}");
        }
        assert_eq!(normalize_entity_type("猫"), "STYLIZED_CHARACTER");
    }

    /// `[ENTITY_ID: ...]` 得是个 slug —— 中文名会被整段滤掉，不能出空的。
    #[test]
    fn 中文名也能生成非空的entity_id() {
        assert_eq!(entity_id("ANIMAL", "大橘（橘猫）"), "animal_subject");
        assert_eq!(entity_id("HUMAN", "Lena Ward"), "human_lena_ward");
        assert_eq!(entity_id("HUMAN", "  "), "human_subject");
        // 混合名字保住 ASCII 那部分
        assert_eq!(entity_id("ANIMAL", "小雨 Xiaoyu"), "animal_xiaoyu");
        for id in ["animal_subject", "human_lena_ward", "animal_xiaoyu"] {
            assert!(!id.ends_with('_'), "{id} 尾巴带下划线");
            assert!(!id.contains("__"), "{id} 有连续下划线");
        }
    }

    /// 把 Rust 真正会提交的参考表图导出来，打真实 ComfyUI 做端到端验证。
    ///
    /// **单测只能证明图的形状。** 提示词模板少一段、`fit_mode` 的两个依赖
    /// 没接上，这些在单测里能查，但「这张图 ComfyUI 收不收、出来的是不是
    /// 七个区」只能真跑。
    ///
    /// `ANIMA_GRAPH_DUMP=/tmp/g.json cargo test --lib dump_entity_sheet -- --ignored`
    #[test]
    #[ignore]
    fn dump_entity_sheet() {
        let name = std::env::var("ANIMA_NAME").unwrap_or_else(|_| "Daju".into());
        let ty = std::env::var("ANIMA_ENTITY_TYPE").unwrap_or_else(|_| "ANIMAL".into());
        let locks: Vec<String> = std::env::var("ANIMA_LOCKS")
            .unwrap_or_else(|_| "very rotund overweight body, standing upright on the hind legs;\
orange and amber tabby fur with darker stripes, cream-white chest and paws;\
forehead marked with an M plus three vertical stripes;\
heavy drooping cheeks, small pink nose, short muzzle;\
narrow angry-looking eyes with dark angular brows;\
long striped tail with distinct darker rings".into())
            .split(';')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let src = std::env::var("ANIMA_SOURCE").unwrap_or_else(|_| "cs_cat.png".into());
        let p = sheet_prompt(&name, &ty, &locks, "");
        eprintln!("--- prompt ---\n{p}\n--- end ---");
        let kind = std::env::var("ANIMA_SHEET_KIND")
            .ok()
            .and_then(|k| SheetKind::from_id(&k))
            .unwrap_or(SheetKind::Dynamic);
        let p = sheet_prompt_from_args(kind, &p, &json!({}));
        let g = build_entity_sheet_graph(kind, &src, &p, REF_BOOST_DEFAULT, SHEET_WIDTH, SHEET_HEIGHT, 43, SHEET_STEPS);
        let out = std::env::var("ANIMA_GRAPH_DUMP").expect("要设 ANIMA_GRAPH_DUMP");
        std::fs::write(&out, serde_json::to_string_pretty(&g).unwrap()).unwrap();
        eprintln!("写到 {out}；save node = {SAVE_NODE_ID}");
    }

    /// **调用方写在 `prompt` 里的描述不许被静默丢掉。**
    ///
    /// 模板会**替换**整段提示词。对话里的 Agent 习惯把一切写进 `prompt`，
    /// 如果它没单独给 `identity_locks`，那段描述就凭空消失 ——
    /// 图照出、不报错，只是七个区之间没有锚点可依、开始互相漂。
    #[test]
    fn 普通描述能被切成锚点而不是丢掉() {
        let locks = locks_from_text("橘色短发带黄褐条纹，眼神凶狠，无，穿深色制服");
        assert_eq!(locks, ["橘色短发带黄褐条纹", "眼神凶狠", "穿深色制服"]);
        // 「无」是资产表单没填时的占位，进了提示词模型会去画一个「无」
        assert!(!locks.iter().any(|l| l == "无"));

        // 切出来的东西必须真的进模板
        let p = sheet_prompt("Daju", "ANIMAL", &locks, "");
        for l in &locks {
            assert!(p.contains(l.as_str()), "锚点「{l}」没进提示词");
        }
    }

    /// 中英标点都要能切，且上限 8 条（作者模板写 4~7 条，多了会互相矛盾）。
    #[test]
    fn 锚点切分和界面侧同口径() {
        assert_eq!(locks_from_text("a, b、c；d\ne"), ["a", "b", "c", "d", "e"]);
        assert!(locks_from_text("").is_empty());
        assert!(locks_from_text("，，、；").is_empty());
        let many: Vec<String> = (0..20).map(|i| format!("锚点{i}")).collect();
        assert_eq!(locks_from_text(&many.join("，")).len(), 8);
    }

    /// 已经是结构化模板的原样放过 —— Agent 想自己写那段方括号结构时不该被覆盖。
    #[test]
    fn 结构化提示词原样放过() {
        let mine = sheet_prompt("X", "HUMAN", &["a".into()], "");
        assert!(is_structured(&mine));
        assert!(!is_structured("一只橘猫，戴红帽"));
        assert!(!is_structured(""));
    }

    /// **整条拼装路径**，不只是那个切分函数。
    ///
    /// 变异测试抓到过：只测 `locks_from_text` 的话，把调用点的兜底摘掉照样绿。
    #[test]
    fn 从参数拼提示词时不丢掉调用方写的东西() {
        // 只给了 prompt，没给 identity_locks —— Agent 最常这么调
        let args = json!({"entity_name": "Daju", "entity_type": "ANIMAL"});
        let p = sheet_prompt_from_args(SheetKind::Dynamic, "橘色短发带黄褐条纹，眼神凶狠，穿深色制服", &args);
        assert!(p.contains("[ENTITY_TYPE: ANIMAL]"));
        assert!(p.contains("- 橘色短发带黄褐条纹"), "描述被丢掉了");
        assert!(p.contains("- 眼神凶狠"), "描述被丢掉了");
        assert!(p.contains("- 穿深色制服"), "描述被丢掉了");

        // 显式给了锚点时以它为准，不再从 prompt 切
        let args = json!({"identity_locks": ["A 锚点", "B 锚点"], "entity_name": "X"});
        let p = sheet_prompt_from_args(SheetKind::Dynamic, "这段描述该被忽略", &args);
        assert!(p.contains("- A 锚点") && p.contains("- B 锚点"));
        assert!(!p.contains("这段描述该被忽略"));

        // 已经是结构化模板的原样放过
        let mine = sheet_prompt("Y", "ROBOT", &["c".into()], "");
        assert_eq!(sheet_prompt_from_args(SheetKind::Dynamic, &mine, &json!({})), mine);

        // 什么都没给也不能崩，而且仍是合法模板
        let p = sheet_prompt_from_args(SheetKind::Dynamic, "", &json!({}));
        assert!(is_structured(&p));
        assert!(p.contains("[ENTITY_TYPE: STYLIZED_CHARACTER]"));
    }

    /// **两种版式的三处差别，抄错都不报错。**
    ///
    /// LoRA 挂错 → 出的是另一个版式；采样器抄错 → 版面质量掉一档；
    /// 提示词给错 → 四视图塞进结构化模板会让版式不稳。全都是「图照出、不报错」。
    #[test]
    fn 四视图和七分区的三处差别都钉住() {
        let q = build_entity_sheet_graph(SheetKind::Quad, "s.png", "忽略", 4.0, 1536, 1024, 1, 10);
        let d = build_entity_sheet_graph(SheetKind::Dynamic, "s.png", "P", 4.0, 1536, 1024, 1, 10);

        // 1. LoRA 各是各的，且都在 krea/ 下（版式 LoRA 只有 Krea 2 版）
        assert_eq!(q["2"]["inputs"]["lora_name"], "krea/QuadView_krea2_v1.safetensors");
        assert_eq!(d["2"]["inputs"]["lora_name"], "krea/DynamicCharacterSheet_krea2_v1.safetensors");
        assert_ne!(q["2"]["inputs"]["lora_name"], d["2"]["inputs"]["lora_name"]);

        // 2. 采样器不一样：Quad 官方是 euler，Dynamic 作者实测 LCM 文字最好
        assert_eq!(q["12"]["inputs"]["sampler_name"], "euler");
        assert_eq!(d["12"]["inputs"]["sampler_name"], "lcm");

        // 3. 提示词：Quad 只发那句固定触发句，**不发结构化模板**
        let qp = sheet_prompt_from_args(SheetKind::Quad, "随便写点什么", &json!({}));
        assert_eq!(qp, QUAD_TRIGGER);
        assert!(!is_structured(&qp), "四视图不该带方括号模板");
        assert!(qp.contains("face close-up"), "触发句被改过 —— 那是训练 caption");
        // 就算调用方塞了一整段模板进来，Quad 也只发触发句
        let mine = sheet_prompt(SheetKind::Dynamic.id(), "HUMAN", &["a".into()], "");
        assert_eq!(sheet_prompt_from_args(SheetKind::Quad, &mine, &json!({})), QUAD_TRIGGER);

        // 其余接线两种版式必须一致 —— 那四条静默失效的坑对谁都成立
        for g in [&q, &d] {
            assert_eq!(g["11"]["inputs"]["fit_mode"], "fit");
            assert!(g["11"]["inputs"]["vae"].is_array());
            assert!(g["11"]["inputs"]["source_image"].is_array());
            assert_eq!(g["11"]["inputs"]["target_latent"], g["12"]["inputs"]["latent_image"]);
            let neg = g["12"]["inputs"]["negative"][0].as_str().unwrap();
            assert_eq!(g[neg]["class_type"], "Krea2EditGroundedEncode");
        }

        // id 往返
        for k in [SheetKind::Quad, SheetKind::Dynamic] {
            assert_eq!(SheetKind::from_id(k.id()), Some(k));
        }
        assert_eq!(SheetKind::from_id("character_sheet"), None, "别把旧的设定卡认进来");
    }

    /// 画幅必须横的 —— 七个区是按 3:2 排的，方图或竖图会挤成一团。
    #[test]
    fn 默认画幅是横幅() {
        assert!(SHEET_WIDTH > SHEET_HEIGHT, "设定表必须横幅");
        assert_eq!((SHEET_WIDTH, SHEET_HEIGHT), (1536, 1024));
    }
}
