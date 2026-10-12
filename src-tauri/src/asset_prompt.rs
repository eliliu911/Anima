//! 资产出图的提示词拼装 —— `src/components/studio/assets/assetPrompt.ts` 和 `sceneViews.ts` 的 Rust 版。
//!
//! # 为什么有两份
//!
//! 工作台里改提示词、换模板是逐字交互的，留在前端；Chat 的工具和后台批量出图在后端，调不到前端。
//! Chat 接 Studio 方案 §4.3 选的是「搬到后端」。**两份靠对拍样例绑死**：
//! `shared/asset-prompt-fixtures.json` 由 TS 实现生成，这里的测试读同一份，一字不差才算过。
//! 改拼装逻辑或文案时两边一起改，然后重新生成样例（见那个 TS 测试文件的说明）。
//!
//! 每一条规矩的来历（风格插在构图段之前、场景道具去掉描述人的风格词、屏幕类道具写清屏幕上是什么、
//! 每一格点名不同锚点……）写在 TS 那两个文件里，这里不重复。

use crate::scene_contract::{AnchorSize, SceneContract};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Character,
    Scene,
    Prop,
}

impl Kind {
    pub fn from_asset(k: crate::assets::AssetKind) -> Self {
        match k {
            crate::assets::AssetKind::Character => Kind::Character,
            crate::assets::AssetKind::Scene => Kind::Scene,
            crate::assets::AssetKind::Prop => Kind::Prop,
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "character" => Some(Kind::Character),
            "scene" => Some(Kind::Scene),
            "prop" => Some(Kind::Prop),
            _ => None,
        }
    }
}

pub struct Template {
    pub id: &'static str,
    pub label: &'static str,
    pub composition: &'static str,
    pub negative: &'static str,
    pub engines: &'static [&'static str],
    pub prefers_aspect: Option<&'static str>,
}

pub struct ViewTerm {
    pub id: &'static str,
    pub name: &'static str,
    pub shot_size: &'static str,
    pub angle: &'static str,
    pub lens: Option<&'static str>,
    pub depth: Option<&'static str>,
    pub placement: &'static str,
}

// 由 TS 的 `assetPrompt.ts` / `sceneViews.ts` 导出生成，改文案时两边一起改；对拍样例见 shared/asset-prompt-fixtures.json
pub const CHARACTER_TEMPLATES: &[Template] = &[
    // **排在第一个**，和 `assetPrompt.ts` 的顺序一致（有测试钉着）。
    // `template_by_id` 的回落方向因此改成了 `simple` —— 见那个函数。
    Template {
        id: "avatar",
        label: "头像 / 标识（没有身体）",
        composition: "Composition: a single centered avatar icon on a plain neutral background, front-facing, symmetrical, filling most of the frame. This subject has NO body: do not draw a torso, arms, legs, clothing, shoes or a standing pose. Do not draw a turnaround sheet, multiple views, panels or borders. Flat even lighting, clean silhouette, crisp edges, suitable for display on a screen.",
        negative: "full body, standing pose, turnaround sheet, multiple views, multiple separate images, human figure, torso, arms, legs, clothing, shoes, photorealistic human, panel borders, frames, text, watermark",
        engines: &[],
        prefers_aspect: Some("square"),
    },
    // 构图卡一张都不选时用的版式（2026-09-26），和 `assetPrompt.ts` 同序
    Template { id: "portrait", label: "居中头像", composition: "Composition: a single centered head-and-shoulders portrait of this one character, front-facing, eye-level, on a plain clean neutral background, filling most of the frame. Sharp facial details, soft even studio lighting. One image only, no other views, panels, borders or text.", negative: "text, labels, watermark, UI overlay, panel borders, frames, multiple separate images", engines: &[], prefers_aspect: Some("portrait") },
    Template { id: "simple", label: "简洁三视图", composition: "Composition: character reference sheet, single unified image, seamless layout without borders or frames, neutral gray background. Left half: large head close-up portrait (shoulders up, sharp facial details, front-facing, detailed skin texture). Right half: three equally-sized full-body standing poses arranged side by side (front view, side view, back view), head-to-toe fully visible, relaxed neutral pose. Consistent soft studio lighting across all views, no harsh shadows, even illumination.", negative: "text, labels, watermark, UI overlay, panel borders, frames, multiple separate images", engines: &[], prefers_aspect: None },
    Template { id: "detailed", label: "详细参考表", composition: "Composition: detailed character reference sheet, single unified image, seamless layout without borders or frames, neutral gray background. Left section: three full-body standing views side by side (front / side / back), head-to-toe visible, neutral relaxed pose. Upper right: large face close-up portrait (shoulders up, detailed skin texture, sharp eyes, pores visible). Lower right: three smaller head shots showing different angles (front, three-quarter, profile). Consistent soft studio lighting, no harsh shadows, even illumination across all panels.", negative: "text, labels, watermark, UI overlay, panel borders, frames, multiple separate images", engines: &[], prefers_aspect: None },
    Template { id: "panel5", label: "五分区", composition: "Keep the same character identity across every panel. Composition: professional character design sheet, single unified image with dark neutral background (deep charcoal with a subtle gradient). Layout divided into labeled panels with thin border frames: - Top left: large dramatic character portrait (bust shot, three-quarter angle, moody rim lighting), unlabeled - Center: exactly three full-body standing views and no other figures in this row (front / side / back), labeled \"正面\" \"侧面\" \"背面\" - Top right: 4 expression close-ups in a row (calm, smiling, angry, surprised), labeled \"表情特写\" - Bottom left: 3-4 detail close-up panels showing the hands, the footwear, a collar and fastening detail, a belt or pocket detail, labeled \"细节特写\". Cinematic lighting, high detail, concept art quality, game character sheet aesthetic. Do not create an information table, stats card, UI panel, watermark, or extra character body.", negative: "text paragraphs, info panel, stat block, watermark, UI overlay, multiple separate images", engines: &["zimage"], prefers_aspect: Some("landscape") },
    Template { id: "quadzh", label: "四视图（中文）", composition: "高质量专业角色设定图，横向构图，纯白色纯净背景，中性摄影棚灯光，平光布光。布局结构（必须是角色四视图）：正面面部头部特写（占图片水平 1/3 的空间，**竖向放大，从头顶到胸口占满这一栏的完整高度**）＋ 全身正面视图、全身左侧面视图、全身背面视图（三个并列排列，占图片水平剩余的 2/3 的空间），四个视图之间用淡灰色(#E2E2E2)的 2px 细线分割。光影：柔和的前侧光，清晰的轮廓定义，自然的肤色，面部清晰、服装可辨识，平视镜头，完整全身，无裁剪。确保所有视图中的面部特征、发型、体型和服装保持一致。不得出现任何道具、武器、食物、饮料、手持物（角色必须空手）；不得出现复杂动作、情绪表情、面部遮挡；不得出现环境背景（仅纯白）；不得出现其他角色；不得出现文字、水印、标签、UI 元素；不得过度风格化。不得出现第二个头像或第二个身体。", negative: "text, labels, watermark, UI overlay, panel borders, frames, multiple separate images", engines: &["zimage"], prefers_aspect: Some("landscape") },
    // 改自用户的陆川设定卡提示词（2026-09-26）。`{name}` 由 `build_prompt` 换成资产名
    Template { id: "bible", label: "设定卡（Qwen Only）", composition: "Composition: a wide, premium, art-directed character-design sheet on a clean white and cool light-gray technical background with thin pale-cyan grid lines and restrained interface graphics. The same person described above appears consistently in every panel, with the same face, age, hairstyle, body proportions, clothing and accessories. The upper-left contains no emblem and no logo, only the exact title “{name}” and beneath it “CHARACTER SHEET”; the narrow profile column below the title is clean empty ruled lines reserved for later typesetting. The central area contains exactly three complete, equally sized, head-to-toe orthographic full-body views in a relaxed neutral A-pose, labeled “FRONT”, “SIDE” and “BACK”; the legs are straight and naturally proportioned and the hands are anatomically correct and relaxed. The right side is an orderly eight-panel section headed “DETAILS”: “FACE / EVERYDAY” shows the usual expression; “EYES / FOCUSED” shows the gaze sharp and concentrated; “HAND DETAIL” shows the hands doing something characteristic of this person; “GEAR” shows the most distinctive accessory up close; “FOOTWEAR” shows the shoes; “SIGNATURE ITEM” neatly displays the object most associated with this person; “FABRIC / MATERIAL” shows the main garment's texture up close; “ALTERNATE LOOK” shows the same person in a second outfit if one is described above, otherwise a three-quarter close-up. Along the bottom are three cinematic state portraits of the same face labeled “EVERYDAY”, “FOCUSED” and “UNDER PRESSURE”, beside organized sections labeled “COLOR PALETTE” and “MATERIAL BREAKDOWN” with swatches taken from the outfit. Clean editorial hierarchy like a meticulously art-directed AAA game character bible, accurate front-side-back turnaround, realistic materials, soft neutral studio light and extremely high material clarity; the three figures occupy most of the central height while every section stays spacious and readable. The sheet contains only this one identity; every full-body figure has two arms, two legs, natural five-fingered hands and complete visible head and feet; the central figures wear only the main outfit; uncertain secondary copy areas are left blank rather than filled with invented writing.", negative: "watermark, extra character body, multiple separate images", engines: &["qwen21"], prefers_aspect: Some("landscape") },
    Template { id: "quad", label: "四视图（Krea 2 版式）", composition: "", negative: "", engines: &["krea2"], prefers_aspect: Some("landscape") },
    Template { id: "panel7", label: "七分区", composition: "", negative: "", engines: &["krea2"], prefers_aspect: Some("landscape") },
    // **没有身体的角色**：手机屏幕上的卡通 AI 头像、一团光、一个图标、纯语音助理的形象。
    //
    // 别的模板都是「全身正面/侧面/背面 + 面部特写」的人物设定图 ——
    // 那段构图本身就会画出一个人，描述里写什么都盖不过它。
    // 2026-09-19 用户在 E05 的小青（「蓝色卡通 AI 助理头像」）身上撞到：
    // 出来的是个真人。
    //
    // 这一份**只要形象本身**：居中、正面、没有身体、没有服装、没有立绘姿势。
];

/// 道具的构图卡，和 `assetPrompt.ts` 的 `PROP_TEMPLATES` 同一份（照抄 Codex 出《回收师》道具卡的提示词，见那边的注释）。
/// 一张都不选时用 [`PROP_COMPOSITION`]
pub const PROP_TEMPLATES: &[Template] = &[
    Template { id: "propcard", label: "道具信息卡（Qwen）", composition: "Use case: stylized-concept\nAsset type: 道具三视图设定卡\nPrimary request: 生成“{name}”的高精度三视图道具卡，外观严格按上面的描述。\nScene/backdrop: 冷白略带浅蓝的无缝工业设计图背景，细灰蓝辅助线和分区线，无环境场景。\nStyle/medium: 高精度 3D 半写实工业产品渲染，美术资产生产图。\nComposition/framing: 16:9 横向设定卡。上排从左到右是 FRONT 正面、SIDE 严格90度侧面、BACK 背面，比例一致、无遮挡。下排两个大特写框：左右各放大展示这件道具最有辨识度的一处结构或细节（材质、机构、刻印或标签），每个框下方用简短的英文大写标注这处细节的名称。主体占比适中，边缘完整，不裁切。\nLighting/mood: 冷白柔光、轻微轮廓光，金属反射克制，严谨、洁净。\nText (verbatim): “FRONT”, “SIDE”, “BACK”；道具本身上的文字、编号照描述原样写。\nConstraints: 必须是同一个道具的严格三视图；正侧背结构相互对应；这是独立道具卡，画面里只有这一件道具本身；所有小字尽量准确、清晰、简洁；除三视图标注、两个特写框的标注和道具本身的文字外，画面里只有干净的背景和辅助线。", negative: "watermark, signature, people, hands, human body, multiple separate images, cropped object", engines: &["qwen21"], prefers_aspect: Some("landscape") },
];

const SCENE_COMPOSITION: &str = "Composition: wide establishing shot of the environment on neutral gray background, single unified image, no figures in foreground. Emphasize atmosphere, architecture and terrain structure. Lighting and color palette match the scene mood. Soft volumetric lighting, depth of field.";
const PROP_COMPOSITION: &str = "Composition: product photography style on neutral gray background, single unified image, seamless layout without borders. The object alone is the only thing in the frame. Main view: object centered at slight angle. Secondary views: detail close-ups of material and texture. Clean even studio lighting, subtle shadow beneath object.";
const PLAIN_NEGATIVE: &str = "text, labels, watermark, UI overlay, panel borders, frames";

pub const VIEW_TERMS: &[ViewTerm] = &[
    ViewTerm { id: "center", name: "正面中心线", shot_size: "extreme_wide", angle: "eye_level", lens: Some("f24"), depth: Some("deep"), placement: "镜头正对场景中心轴，构图严格居中对称，画面同时包含顶面与底面；视线尽头是{锚点}" },
    ViewTerm { id: "left45", name: "左前 45°", shot_size: "extreme_wide", angle: "eye_level", lens: Some("f24"), depth: Some("deep"), placement: "**以第 1 格的中心线为参考**，摄像机移动到场景**左前方 45 度**，画面主体换成**{锚点}**那一侧的墙面和陈设，空间连贯、比例一致；**画面里看不到{主锚点}**" },
    ViewTerm { id: "right45", name: "右前 45°", shot_size: "extreme_wide", angle: "eye_level", lens: Some("f24"), depth: Some("deep"), placement: "**以第 1 格的中心线为参考**，摄像机移动到场景**右前方 45 度**，画面主体换成**{锚点}**那一侧的墙面和陈设，空间连贯、比例一致；**画面里看不到{主锚点}**" },
    ViewTerm { id: "deep_out", name: "深处往外", shot_size: "extreme_wide", angle: "eye_level", lens: Some("f24"), depth: Some("deep"), placement: "**镜头站在室内最深处向外拍摄**，画面主体是**{入口}和门外透进来的天光**，以及入口两侧的墙；**画面里看不到{主锚点}**（镜头就站在它那儿）。这一格的进光方向和前几格相反 —— 前几格朝暗处看，这一格朝亮处看" },
    ViewTerm { id: "entry_in", name: "入口往里", shot_size: "wide", angle: "eye_level", lens: Some("f35"), depth: Some("deep"), placement: "镜头站在{入口}内侧朝空间深处拍，视线尽头是{锚点}，前景是脚下的地面" },
    ViewTerm { id: "high", name: "高机位", shot_size: "wide", angle: "high_angle", lens: Some("f24"), depth: Some("deep"), placement: "镜头升到接近天花板的高度俯看（**不是极端俯视**），地面分区和{锚点}的位置一览无余" },
    ViewTerm { id: "low", name: "低机位", shot_size: "wide", angle: "low_angle", lens: Some("f24"), depth: Some("deep"), placement: "镜头贴近地面仰看（**不是极端仰视**），{锚点}显得高大，天花板进入画面" },
    ViewTerm { id: "work_area", name: "活动区中景", shot_size: "medium", angle: "eye_level", lens: Some("f50"), depth: Some("deep"), placement: "镜头站在{入口}和{锚点}之间，中景拍{锚点}周围那一块真正发生事情的区域，看得出人在这儿会站在哪、怎么走动" },
    ViewTerm { id: "detail", name: "材质近景", shot_size: "close_up", angle: "eye_level", lens: Some("f85"), depth: Some("shallow"), placement: "镜头贴近{锚点}拍它的表面，看得清材质、磨损和它周围一小圈" },
];


pub fn template_by_id(id: &str) -> &'static Template {
    // **回落到 `simple`，不是数组第一个。** 第一个现在是 `avatar`（没有身体），
    // 把一个认不出的 id 回落成「不要画身体」比回落成全身图坏得多：
    // 真人角色会变成一个图标，而且不报错。
    CHARACTER_TEMPLATES
        .iter()
        .find(|t| t.id == id)
        .or_else(|| CHARACTER_TEMPLATES.iter().find(|t| t.id == "simple"))
        .expect("simple 版式不见了")
}

/// 去掉「描述：」「服装：」这类标签和换行，拼成一句。值一个字不删
pub fn flatten_description(desc: &str) -> String {
    const LABELS: [&str; 5] = ["描述", "服装", "特征", "外观", "说明"];
    let joined: String = desc
        .split('\n')
        .map(|line| {
            let t = line.trim_start();
            for l in LABELS {
                if let Some(rest) = t.strip_prefix(l) {
                    let rest = rest.trim_start();
                    if let Some(after) = rest.strip_prefix(':').or_else(|| rest.strip_prefix('：')) {
                        return after.trim().to_string();
                    }
                }
            }
            line.trim().to_string()
        })
        .filter(|l| !l.is_empty())
        .collect();
    joined.replace("。，", "。")
}

/// ASCII 单词边界（和 JS 正则的 `\b` 一样）
fn has_word(hay: &str, word: &str) -> bool {
    let is_w = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut from = 0;
    while let Some(i) = hay[from..].find(word) {
        let at = from + i;
        let before = hay[..at].chars().next_back();
        let after = hay[at + word.len()..].chars().next();
        if !before.is_some_and(is_w) && !after.is_some_and(is_w) {
            return true;
        }
        from = at + word.len();
    }
    false
}

/// 带屏幕的道具写清屏幕上是什么。不是屏幕类道具回空
pub fn screen_hint(name: &str, description: &str) -> String {
    let hay = format!("{name} {description}").to_lowercase();
    let device = ["手机", "电脑", "显示器", "屏幕", "平板", "电视", "笔记本", "监控", "phone", "laptop", "monitor", "tablet", "screen"].iter().any(|k| hay.contains(k)) || has_word(&hay, "tv");
    if !device {
        return String::new();
    }
    let on = ["亮", "显示", "提示", "通知", "消息", "界面", "画面", "播放"].iter().any(|k| description.contains(k));
    if on {
        "屏幕画面是纯界面：几行文字通知，每行前面一个蓝色小圆点，界面平铺在屏幕上".into()
    } else {
        "屏幕熄灭，黑色玻璃面反光".into()
    }
}

fn composition_for(kind: Kind, template: &str) -> &'static str {
    match kind {
        Kind::Character => template_by_id(template).composition,
        Kind::Scene => SCENE_COMPOSITION,
        // 道具只有选了 `PROP_TEMPLATES` 里的卡才换，其余一律产品摄影那段（同 TS 的 `compositionFor`）
        Kind::Prop => PROP_TEMPLATES.iter().find(|t| t.id == template).map_or(PROP_COMPOSITION, |t| t.composition),
    }
}

/// `名字，描述`（道具再加屏幕说明）+ 空行 + 构图段
pub fn build_prompt(kind: Kind, name: &str, description: &str, template: &str) -> String {
    let name = name.trim();
    let desc = flatten_description(description.trim());
    let screen = if kind == Kind::Prop { screen_hint(name, &desc) } else { String::new() };
    let first = if desc.is_empty() { name.to_string() } else { format!("{name}，{desc}") };
    let head = [first, screen].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("。");
    // 设定卡要把名字一字不差写在图上：构图段里的 `{name}` 换成资产名（同 TS 的 `withName`）
    format!("{head}\n\n{}", composition_for(kind, template).replace("{name}", name))
}

pub fn template_negative(kind: Kind, template: &str) -> &'static str {
    match kind {
        Kind::Character => template_by_id(template).negative,
        Kind::Prop => PROP_TEMPLATES.iter().find(|t| t.id == template).map_or(PLAIN_NEGATIVE, |t| t.negative),
        Kind::Scene => PLAIN_NEGATIVE,
    }
}

fn is_human_style_word(t: &str) -> bool {
    let l = t.to_lowercase();
    const SUBSTR: &[&str] = &["human", "skin", "face", "facial", "actor", "actress", "performance", "portrait", "body", "proportion", "finger", "hair", "emotion", "people", "person", "character", "pose", "人物", "人体", "皮肤", "脸", "面部", "表情", "演员", "表演", "身材", "五官", "发型", "姿势"];
    SUBSTR.iter().any(|k| l.contains(k)) || ["hand", "hands", "eye", "eyes"].iter().any(|w| has_word(&l, w))
}

/// 场景、道具只取风格里的质感、色调、镜头感，去掉描述人的词
pub fn style_for_kind(kind: Kind, style: &str) -> String {
    let style = style.trim();
    if kind == Kind::Character || style.is_empty() {
        return style.to_string();
    }
    style
        .split([',', '，'])
        .map(str::trim)
        .filter(|t| !t.is_empty() && !is_human_style_word(t) && !t.to_lowercase().contains("live-action"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// 最终正向：风格插在主体之后、构图段之前（第一个空行之前）
pub fn final_positive(prompt: &str, style: &str, apply: bool, kind: Kind) -> String {
    let base = prompt.trim();
    let style = if apply { style_for_kind(kind, style) } else { String::new() };
    if style.is_empty() {
        return base.to_string();
    }
    if base.is_empty() {
        return style;
    }
    if base.contains(&style) {
        return base.to_string();
    }
    match base.find("\n\n") {
        None => format!("{base}, {style}"),
        Some(at) => format!("{}，{}{}", &base[..at], style, &base[at..]),
    }
}

pub fn final_negative(style_negative: &str, composition_negative: &str, apply: bool) -> String {
    [if apply { style_negative.trim() } else { "" }, composition_negative.trim()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

/// 角色批量用哪个版式：中文四视图只有 Z-Image 撑得住，别的引擎退回简洁三视图
pub fn character_template_for(engine: &str) -> &'static str {
    if template_by_id("quadzh").engines.contains(&engine) { "quadzh" } else { "simple" }
}

/// **没有身体的角色**用头像版式，其余照旧。
///
/// # 判据是描述里有没有身体和衣物，不是「是不是人」
///
/// 「是不是人」从字段里判不出来（猫有身体、机器人有身体、AI 头像没有）。
/// 而「有没有身体」正好是同一件事的可观测形式：
/// 描述里一件衣物、一处身形都没有，就不是能出全身三视图的东西。
///
/// **默认当它有身体，只在正面认出「头像」时才换。**
///
/// 第一版反过来写（列身体关键词，一条都没有就判头像），当场误报：
/// 夹具里的「唐装」是衣服，却不在那张表里。**列举「有什么」必然漏**，
/// 而漏的代价是一个正经角色只出一张脸、分镜里没有全身参考 ——
/// 那比多画一个身体难发现得多。
///
/// 正面认「没有身体」的词少而且确定：头像、图标、一团光、界面里的形象。
/// 认不出就照旧 —— 最坏是多画一个身体，用户一眼看得出来、换个版式重出。
pub fn character_template_by_body(engine: &str, description: &str) -> &'static str {
    const BODILESS: &[&str] = &[
        "头像", "图标", "标识", "logo", "icon", "avatar",
        "一团光", "光团", "光点", "光球", "没有轮廓", "无实体", "纯语音",
    ];
    let d = description.to_lowercase();
    if BODILESS.iter().any(|w| d.contains(&w.to_lowercase())) {
        "avatar"
    } else {
        character_template_for(engine)
    }
}

pub fn template_aspect(template: &str) -> &'static str {
    template_by_id(template).prefers_aspect.unwrap_or("portrait")
}

// ── 场景宫格的方位 ────────────────────────────────────────────────

/// 锚点按「适不适合当机位目标」排：离散大件在前，整面墙其次
pub fn ranked_anchors(c: Option<&SceneContract>) -> Vec<String> {
    let weight = |s: AnchorSize| match s {
        AnchorSize::Large => 3,
        AnchorSize::WallDominant => 2,
        AnchorSize::Medium => 1,
        AnchorSize::Small => 0,
    };
    let mut v: Vec<_> = c.map(|c| c.anchors.iter().filter(|a| !a.name.trim().is_empty()).collect()).unwrap_or_default();
    v.sort_by(|a, b| weight(b.size).cmp(&weight(a.size)));
    v.into_iter().map(|a| a.name.trim().to_string()).collect()
}

/// 一格的方位描述：镜头语言 + 相机站位。`slot` 决定这一格点名哪个锚点
pub fn describe_term(term: &ViewTerm, c: Option<&SceneContract>, slot: usize) -> String {
    let ranked = ranked_anchors(c);
    let main = ranked.first().cloned().unwrap_or_else(|| "最显眼的那个不可移动物".into());
    let anchor = if ranked.is_empty() { main.clone() } else { ranked[slot % ranked.len()].clone() };
    let entrance = c.and_then(|c| c.entrances.iter().find(|e| !e.name.trim().is_empty())).map(|e| e.name.trim().to_string()).unwrap_or_else(|| "主入口".into());
    let placement = term.placement.replace("{锚点}", &anchor).replace("{主锚点}", &main).replace("{入口}", &entrance);
    let pt = |g: &str, v: &str| crate::shot_language::prompt_text(g, v, "zimage");
    let lang = [Some(pt("shotSize", term.shot_size)), Some(pt("angle", term.angle)), term.lens.map(|l| pt("lens", l)), term.depth.map(|d| pt("depth", d))]
        .into_iter()
        .flatten()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("，");
    if lang.is_empty() { placement } else { format!("{lang}。{placement}") }
}

/// 批量出场景时默认的 4 格（词表前四条）
pub const BATCH_GRID: usize = 4;

pub fn default_viewpoints(c: Option<&SceneContract>) -> Vec<(String, String)> {
    VIEW_TERMS.iter().take(BATCH_GRID).enumerate().map(|(i, t)| (t.name.to_string(), describe_term(t, c, i))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 设定卡要把名字一字不差写在图上：`{name}` 必须换成资产名，和 TS 的 `withName` 一样。
    #[test]
    fn 设定卡的标题换成资产名() {
        let p = build_prompt(Kind::Character, "陆川", "26 岁", "bible");
        assert!(p.contains("the exact title “陆川”"), "{p}");
        assert!(!p.contains("{name}"));
        assert_eq!(template_by_id("bible").engines, &["qwen21"]);
    }

    /// **没有身体的角色不能用人物设定图的构图。**
    ///
    /// 2026-09-19 用户在 E05 的小青身上撞到：描述写的是
    /// 「蓝色卡通 AI 助理头像」，出来的是个真人 ——
    /// 因为人物版式的构图本身就是「全身正面/侧面/背面 + 面部特写」，
    /// 描述里写什么都盖不过它。

    /// **Rust 和 TS 的版式表必须是同一份。**
    ///
    /// 2026-09-19 出过事，而且是本文件的作者干的：`avatar` 版式只加进了
    /// 这边，`assetPrompt.ts` 没加，工作台又写死 `"simple"` ——
    /// 用户在工作台点生成，拿到的还是「三个全身站姿」，头像照样出成一个人。
    ///
    /// 当时有三条相关的测试全是绿的：它们查的是
    /// 「TS 的 id ⇄ 示例图清单」和「TS 的构图 ⇄ 示例图渲染器」，
    /// **没有一条查 Rust ⇄ TS**。所以这条补上。
    #[test]
    fn 版式表和前端一字不差() {
        let ts = std::fs::read_to_string("../src/components/studio/assets/assetPrompt.ts")
            .expect("读不到 assetPrompt.ts");
        let start = ts.find("CHARACTER_TEMPLATES: Template[] = [").expect("找不到 CHARACTER_TEMPLATES");
        let src = &ts[start..ts[start..].find("\n];").map(|i| start + i).unwrap_or(ts.len())];
        // 只留字母数字和汉字：TS 那边是多行拼接的字符串，比字节没法比，
        // 而漂移从来不是漂在标点和换行上。
        let squeeze = |s: &str| -> String { s.chars().filter(|c| c.is_alphanumeric()).collect() };
        // **只取双引号里的内容。** `quadzh` 的构图段是十几个字符串用 `+`
        // 拼的，**中间还夹着 `//` 注释** —— 整段 squeeze 的话注释文字会插在
        // 两截构图中间，比对必然失败（而失败原因看起来像「文案漂了」）。
        let quoted_only = |src: &str| -> String {
            // **先把转义引号去掉。** `panel5` 里有 `\"正面\"` 这种嵌在字符串
            // 里的引号 —— 不去掉的话下面的「引号成对」扫描会把它们当成
            // 字符串的边界，正面/侧面/背面三个词整个被吃掉，
            // 表现是「构图段在前端找不到」，而真正的差异根本不存在。
            let src = src.replace("\\\"", "");
            let mut out = String::new();
            let mut rest = src.as_str();
            while let Some(q) = rest.find('"') {
                let body = &rest[q + 1..];
                match body.find('"') {
                    Some(e) => {
                        out.push_str(&body[..e]);
                        rest = &body[e + 1..];
                    }
                    None => break,
                }
            }
            out
        };
        let ts_squeezed = squeeze(&quoted_only(src));

        let mut ids: Vec<&str> = Vec::new();
        for part in src.split("id: \"").skip(1) {
            if let Some(end) = part.find('"') {
                let id = &part[..end];
                if id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) && !id.is_empty() {
                    ids.push(id);
                }
            }
        }
        let mine: Vec<&str> = CHARACTER_TEMPLATES.iter().map(|t| t.id).collect();
        assert_eq!(mine, ids, "两边的版式清单对不上 —— 改了一边没改另一边");

        for t in CHARACTER_TEMPLATES {
            // pipeline 版式（quad / panel7）的 composition 是空的，不比
            if t.composition.is_empty() {
                continue;
            }
            assert!(
                ts_squeezed.contains(&squeeze(t.composition)),
                "{} 的构图段在前端找不到 —— 两边会出不一样的图，而且不报错",
                t.id
            );
            // **负向词在整份文件里找。** TS 那边几个版式共用
            // `SHEET_NEGATIVE` 这个常量，文本不在 `CHARACTER_TEMPLATES`
            // 那一段里 —— 只扫那一段的话会误报「前端没有」。
            assert!(
                squeeze(&quoted_only(&ts)).contains(&squeeze(t.negative)),
                "{} 的负向词在前端找不到",
                t.id
            );
        }
    }
    #[test]
    fn 没有身体的角色走头像版式() {
        // E05 小青的真实描述
        assert_eq!(character_template_by_body("zimage", "描述：蓝色卡通 AI 助理头像。"), "avatar");
        assert_eq!(character_template_by_body("krea2", "一团没有轮廓的蓝色光"), "avatar");
    }

    /// **有身体的照旧。** 误判成头像的话，一个正经角色只出一张脸、
    /// 分镜里没有全身参考 —— 那比多画一个身体更难发现。
    /// **认不出就照旧。**
    ///
    /// 第一版是反着写的（列身体关键词，一条都没有就判头像），当场误报：
    /// 「唐装」是衣服却不在表里。**列举「有什么」必然漏**，
    /// 而漏的代价是一个正经角色只出一张脸 —— 比多画一个身体难发现得多。
    #[test]
    fn 认不出没有身体就照旧出三视图() {
        for d in [
            // E05 陈先生
            "描述：现代都市男性，身高175厘米，体型中等，短发。\n服装：上衣浅色衬衫，皮鞋。",
            "唐装",                              // ← 第一版在这里误报了
            "二十岁女性",
            "一只圆滚滚的橘猫，体型偏胖",
            "",                                   // 空描述多半是还没填
            "   ",
        ] {
            assert_ne!(character_template_by_body("zimage", d), "avatar", "误判成头像了：{d:?}");
        }
    }

    /// 头像版式要**明说没有身体** —— 只是不提的话，模型照样会画一个。
    #[test]
    fn 头像版式明确排除身体() {
        let t = template_by_id("avatar");
        for w in ["NO body", "do not draw a torso", "turnaround"] {
            assert!(t.composition.contains(w), "构图里少了「{w}」：{}", t.composition);
        }
        assert!(t.negative.contains("full body") && t.negative.contains("human figure"), "{}", t.negative);
    }
    use serde_json::Value;

    /// 和 TS 实现逐条对拍（shared/asset-prompt-fixtures.json）
    #[test]
    fn 和前端实现一字不差() {
        let fx: Value = serde_json::from_str(include_str!("../../shared/asset-prompt-fixtures.json")).unwrap();
        let contracts = &fx["contracts"];
        let kind = |v: &Value| Kind::parse(v.as_str().unwrap()).unwrap();
        let st = |v: &Value| v.as_str().unwrap_or("").to_string();
        let mut checked = 0;
        for case in fx["cases"].as_array().unwrap() {
            let a = &case["args"];
            let got: Value = match case["fn"].as_str().unwrap() {
                "flatten_description" => flatten_description(&st(&a["description"])).into(),
                "screen_hint" => screen_hint(&st(&a["name"]), &st(&a["description"])).into(),
                "build_prompt" => build_prompt(kind(&a["kind"]), &st(&a["name"]), &st(&a["description"]), &st(&a["template"])).into(),
                "template_negative" => template_negative(kind(&a["kind"]), &st(&a["template"])).into(),
                "final_positive" => final_positive(&st(&a["prompt"]), &st(&a["style"]), a["apply"].as_bool().unwrap(), kind(&a["kind"])).into(),
                "style_for_kind" => style_for_kind(kind(&a["kind"]), &st(&a["style"])).into(),
                "final_negative" => final_negative(&st(&a["style"]), &st(&a["composition"]), a["apply"].as_bool().unwrap()).into(),
                "character_template_for" => character_template_for(&st(&a["engine"])).into(),
                "template_aspect" => template_aspect(&st(&a["template"])).into(),
                "default_viewpoints" => {
                    let c: Option<SceneContract> = serde_json::from_value(contracts[a["contract"].as_str().unwrap()].clone()).unwrap();
                    Value::Array(default_viewpoints(c.as_ref()).into_iter().map(|(n, d)| serde_json::json!({"name": n, "description": d})).collect())
                }
                other => panic!("样例里有没对上的函数 {other}"),
            };
            assert_eq!(got, case["out"], "{} {}", case["fn"], a);
            checked += 1;
        }
        assert!(checked >= 60, "样例太少：{checked}");
    }
}
