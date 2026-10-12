//! 风格定调 —— 整部剧的视觉基调。
//!
//! 内置预设的**文字**改自 LumenX（`src/apps/comic_gen/style_presets.json`，
//! MIT，Copyright 2026 Alibaba）：15 个预设、4 个分类。
//! 另加了第 16 个 `bjd_3d_guoman`（BJD 人偶国漫）和「3D 国漫」分类，是我们自己写的（2026-09-17）。
//!
//! # 缩略图必须我们自己出
//!
//! 他们那 16 张样例图是 wanx / gpt-image 出的。我们是
//! Z-Image / Chroma / SDXL / FLUX，编码器完全不同 ——
//! 同一段词出来不是同一张画。拿他们的图当选择器，等于让用户
//! **按一张我们复现不出来的画面做选择**。
//!
//! 所以缩略图由 `scripts/render-style-thumbs.sh` 用本机引擎逐个出一遍，
//! 落在项目根的 `resources/style_thumbs/<id>.png`。没出的显示占位，
//! 不显示别人的图。
//!
//! # 负向提示词只在一半引擎上有效
//!
//! 每个预设都带 `negative_prompt`，但它在 **Z-Image 和 FLUX 上结构性无效**
//! （cfg=1，负向被 `ConditioningZeroOut` 清零）。只有 SDXL 和 Chroma 真吃。
//! 界面上要按当前引擎标出来 —— 显示一个不生效的字段比不显示更糟。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

const PRESETS_JSON: &str = include_str!("../resources/style_presets.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StyleCategory {
    pub id: String,
    pub name: String,
    pub name_zh: String,
    #[serde(default)]
    pub sort_order: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StylePreset {
    pub id: String,
    pub category: String,
    pub name: String,
    pub name_zh: String,
    #[serde(default)]
    pub subtitle_zh: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub best_for: Vec<String>,
    #[serde(default)]
    pub avoid_for: Vec<String>,
    pub positive_prompt: String,
    #[serde(default)]
    pub negative_prompt: String,
    /// 出缩略图用的样例画面 —— **每个风格一句自己的**，题材配合风格。
    #[serde(default)]
    pub sample_prompt: String,
}

#[derive(Debug, Clone, Deserialize)]
struct PresetFile {
    categories: Vec<StyleCategory>,
    presets: Vec<StylePreset>,
}

fn presets() -> &'static PresetFile {
    use std::sync::OnceLock;
    static CELL: OnceLock<PresetFile> = OnceLock::new();
    CELL.get_or_init(|| {
        // 内置数据解析不了是编译期就该发现的错误，不是运行时的可恢复情况。
        serde_json::from_str(PRESETS_JSON).expect("内置风格预设 JSON 坏了")
    })
}

pub fn all_presets() -> &'static [StylePreset] {
    &presets().presets
}

pub fn find_preset(id: &str) -> Option<&'static StylePreset> {
    presets().presets.iter().find(|p| p.id == id)
}

/// 缩略图目录。**没有就没有** —— 显示占位，不显示别人的图。
pub fn thumbs_dir() -> PathBuf {
    if let Some(root) = crate::cinema::workspace_root() {
        let dir = root.join("resources").join("style_thumbs");
        if dir.is_dir() {
            return dir;
        }
    }
    PathBuf::from("resources").join("style_thumbs")
}

/// 构图模板的示例图在哪。和风格缩略图同一套路数。
///
/// **这些图是我们自己的引擎出的，不是抄别人的。** 示例图的用处是
/// 「点下去大概会得到什么」—— 放一张别的模型出的图等于骗人，
/// 尤其是我们已经量到 FLUX 画不出这种版面。
pub fn template_thumbs_dir() -> PathBuf {
    if let Some(root) = crate::cinema::workspace_root() {
        let dir = root.join("resources").join("template_thumbs");
        if dir.is_dir() {
            return dir;
        }
    }
    PathBuf::from("resources").join("template_thumbs")
}

/// 构图模板的示例图 URL。没出过的回 `None` —— 界面显示占位，
/// **不显示别人的图**。
#[tauri::command]
pub fn style_template_thumbs() -> std::collections::BTreeMap<String, String> {
    let dir = template_thumbs_dir();
    // **id 清单从模板表读，不在这儿手抄一份。**
    //
    // 2026-09-08 出过：模板 id 从 `design_sheet` 改成 `panel5` / `panel7`，
    // 这里的硬编码数组没跟着改 —— 那两张卡在界面上显示「还没出示例图」，
    // 而示例图文件明明在盘上。**不报错、只是空着。**
    crate::studio::TEMPLATE_THUMB_IDS
        .iter()
        .copied()
        .filter_map(|id| {
            let f = [dir.join(format!("{id}.jpg")), dir.join(format!("{id}.png"))]
                .into_iter()
                .find(|f| f.is_file())?;
            Some((id.to_string(), crate::assets::media_url(&f)))
        })
        .collect()
}

/// 这个引擎吃不吃负向提示词。
///
/// Z-Image 的 cfg=1，负向被 `ConditioningZeroOut` 清零 ——
/// **传了不报错，只是完全不起作用**。界面上必须标出来。
///
/// 白名单而不是黑名单：不认识的 id（包括 2026-09-08 退役的 `flux`）一律
/// 判成「不吃负向」。方向是保守的 —— 少显示一个框，好过显示一个不起作用的框。
pub fn honors_negative(engine: &str) -> bool {
    // Chroma 原来也在这里，2026-10-01 下架了
    matches!(engine, "sdxl")
}

// ---------------------------------------------------------------------------
// 项目的风格设定
// ---------------------------------------------------------------------------

/// 一个项目选定的风格。自定义的正/负向词优先于预设。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ArtDirection {
    /// 选中的内置预设 id。空 = 纯自定义。
    #[serde(default)]
    pub preset_id: String,
    /// 覆盖或追加的正向词。**留空表示用预设的**，不是「不要正向词」。
    #[serde(default)]
    pub positive_prompt: String,
    #[serde(default)]
    pub negative_prompt: String,
    /// **镜头与表演偏好（本项目）**。生成镜头表、细化时间线时当成「用户定下的」规则带进去
    /// （见 `prompts::load_for_project`）。
    ///
    /// 为什么是项目级：2026-10-03 用户要《回收师》女性角色多给性感机位和姿态（低机位仰拍、
    /// 翘腿斜坐、猫步）—— 这是这一部剧的口味，写进全局模板会把《每日小离谱》这种喜剧也带歪。
    /// 空 = 没有额外偏好，模板里那一行整个不出现。
    #[serde(default)]
    pub shot_notes: String,
    #[serde(default)]
    pub updated_at: u64,
}

impl ArtDirection {
    /// 实际生效的正向风格词。
    pub fn effective_positive(&self) -> String {
        let custom = self.positive_prompt.trim();
        if !custom.is_empty() {
            return custom.to_string();
        }
        find_preset(&self.preset_id)
            .map(|p| p.positive_prompt.clone())
            .unwrap_or_default()
    }

    pub fn effective_negative(&self) -> String {
        let custom = self.negative_prompt.trim();
        if !custom.is_empty() {
            return custom.to_string();
        }
        find_preset(&self.preset_id)
            .map(|p| p.negative_prompt.clone())
            .unwrap_or_default()
    }

    pub fn is_set(&self) -> bool {
        !self.preset_id.trim().is_empty() || !self.positive_prompt.trim().is_empty()
    }
}

fn art_file(config_dir: &Path, project_id: &str) -> PathBuf {
    crate::cinema::cinema_projects_dir(config_dir)
        .join(project_id)
        .join("art_direction.json")
}

pub fn load_art(config_dir: &Path, project_id: &str) -> ArtDirection {
    std::fs::read_to_string(art_file(config_dir, project_id))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save_art(config_dir: &Path, project_id: &str, art: &ArtDirection) -> Result<()> {
    let mut art = art.clone();
    art.updated_at = crate::assets::now_ms();
    let f = art_file(config_dir, project_id);
    if let Some(p) = f.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    std::fs::write(
        &f,
        serde_json::to_string_pretty(&art)
            .map_err(|e| Error::config(format!("序列化风格设定失败：{e}")))?,
    )
    .map_err(|e| Error::config(format!("写入风格设定失败：{e}")))
}

/// 把风格词拼进一次生成的提示词。
///
/// **风格在后，画面在前。** 反过来的话，SDXL 那种分块取平均的编码器
/// 会让风格词把画面主体挤掉 —— 出来是一张「很有风格但不是这个镜头」的图。
pub fn apply_style(prompt: &str, art: &ArtDirection) -> String {
    let style = art.effective_positive();
    let style = style.trim();
    if style.is_empty() {
        return prompt.to_string();
    }
    let base = prompt.trim();
    if base.is_empty() {
        return style.to_string();
    }
    // 已经带过了就不重复叠 —— 重渲同一镜时最容易叠两遍。
    if base.contains(style) {
        return base.to_string();
    }
    // **长提示词把风格放前面。**
    //
    // 短提示词（一句画面描述）风格放后面是对的，这一点没变。
    // 但分镜的提示词是润色出来的两百字完整描述，里面全是摄影语言 ——
    // 「景深较浅」「背景虚化」「高清晰度」，末尾还常自带一句
    // 「写实摄影风格」。风格词排在那一大段后面会被彻底淹没：
    // 用户选了赛璐璐动画，出来是写实照片。
    //
    // 这条规矩是资产出图那边**同 seed 对照实测**出来的
    // （见 `assetPrompt.ts` 的表：风格在构图段之后 → 写实照片猫；
    // 之前 → 手绘插画猫）。这里是把同一条结论用在同一种形状的输入上
    // —— 长描述 + 摄影词。**但这一处本身还没在分镜上 A/B 过**，
    // 真出图核过之后再把结论钉在这里。
    if base.chars().count() > LONG_PROMPT {
        return format!("{style}, {base}");
    }
    format!("{base}, {style}")
}

/// 超过这个长度就算"长提示词"，风格改放前面。
///
/// 手打的一句画面描述通常在 40 字以内；润色出来的分镜提示词是 200 字起。
/// 80 落在中间，两边都留了余量。
const LONG_PROMPT: usize = 80;

// ---------------------------------------------------------------------------
// 命令层
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct PresetView {
    #[serde(flatten)]
    pub preset: &'static StylePreset,
    /// 我们自己出的缩略图。`None` = 还没出，界面显示占位。
    pub thumb_url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StyleCatalog {
    pub categories: &'static [StyleCategory],
    pub presets: Vec<PresetView>,
    /// 一共几张缩略图已经出好了。全 0 时界面要说清楚为什么都是占位。
    pub thumbs_ready: usize,
}

#[tauri::command]
pub fn style_catalog() -> StyleCatalog {
    let dir = thumbs_dir();
    let mut ready = 0usize;
    let presets = all_presets()
        .iter()
        .map(|p| {
            // 缩略图存 jpg（384px，全套 850 KB）—— 原始 768px PNG 一套 12 MB，
            // 而界面上只显示 224px 宽的卡片。旧的 png 也认，方便本地重跑。
            let f = [
                dir.join(format!("{}.jpg", p.id)),
                dir.join(format!("{}.png", p.id)),
            ]
            .into_iter()
            .find(|f| f.is_file())
            .unwrap_or_else(|| dir.join(format!("{}.jpg", p.id)));
            let thumb_url = if f.is_file() {
                ready += 1;
                Some(crate::assets::media_url(&f))
            } else {
                None
            };
            PresetView { preset: p, thumb_url }
        })
        .collect();
    StyleCatalog {
        categories: &presets_categories()[..],
        presets,
        thumbs_ready: ready,
    }
}

fn presets_categories() -> &'static Vec<StyleCategory> {
    &presets().categories
}

#[tauri::command]
pub fn style_get(state: tauri::State<'_, crate::AppState>, project_id: String) -> ArtDirection {
    load_art(&state.config_dir, &project_id)
}

#[tauri::command]
pub fn style_save(
    state: tauri::State<'_, crate::AppState>,
    project_id: String,
    art: ArtDirection,
) -> Result<ArtDirection> {
    save_art(&state.config_dir, &project_id, &art)?;
    Ok(load_art(&state.config_dir, &project_id))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StyleSuggestion {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub reason: String,
    pub positive_prompt: String,
    #[serde(default)]
    pub negative_prompt: String,
}

/// 读剧本推荐 3 种风格。
#[tauri::command]
pub async fn style_analyze(
    state: tauri::State<'_, crate::AppState>,
    script: String,
) -> Result<Vec<StyleSuggestion>> {
    if script.trim().is_empty() {
        return Err(Error::runtime("这一集还没有剧本 —— 先在第 1 步写点东西"));
    }
    let cfg = crate::prompts::load(&state.config_dir);
    let raw = crate::prompts::run(
        &state.gateway,
        &cfg,
        crate::prompts::PromptSlot::StyleAnalysis,
        &[("TEXT", script.as_str())],
    )
    .await?;
    parse_suggestions(&raw).ok_or_else(|| {
        // 把模型原样返回的带出去 —— 只说「失败」的话，分不清是模型没听懂
        // 还是我们没解析出来。
        let head: String = raw.chars().take(240).collect();
        Error::runtime(format!("没解析出风格推荐。模型返回的是：{head}"))
    })
}

pub fn parse_suggestions(raw: &str) -> Option<Vec<StyleSuggestion>> {
    let json = crate::prompts::extract_json(raw)?;
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    // 模型可能直接给数组，也可能包在 {"styles": [...]} 里 —— 两种都收。
    let arr = v
        .get("styles")
        .and_then(|x| x.as_array())
        .or_else(|| v.as_array())?;
    let out: Vec<StyleSuggestion> = arr
        .iter()
        .filter_map(|x| serde_json::from_value(x.clone()).ok())
        .collect();
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 内置预设读得出来_二十个五类() {
        assert_eq!(all_presets().len(), 20);
        assert_eq!(presets_categories().len(), 5);
        for p in all_presets() {
            assert!(!p.positive_prompt.trim().is_empty(), "{} 没有正向词", p.id);
            assert!(!p.name_zh.trim().is_empty(), "{} 没有中文名", p.id);
            assert!(
                presets_categories().iter().any(|c| c.id == p.category),
                "{} 的分类 {} 不存在",
                p.id,
                p.category
            );
        }
    }

    #[test]
    fn 预设_id_不重复() {
        let mut ids: Vec<_> = all_presets().iter().map(|p| p.id.as_str()).collect();
        ids.sort_unstable();
        let n = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), n);
    }

    #[test]
    fn 每个预设都有出缩略图用的样例提示词() {
        // 缺一句就少一张能选的图。
        for p in all_presets() {
            assert!(!p.sample_prompt.trim().is_empty(), "{} 没有 sample_prompt", p.id);
        }
        // 每个风格画自己的题材 —— 以前 15 句全是「站台上拿罗盘的年轻人」，一排缩略图看着是同一张画。
        let mut firsts: Vec<String> = all_presets()
            .iter()
            .map(|p| p.sample_prompt.split_whitespace().take(4).collect::<Vec<_>>().join(" ").to_lowercase())
            .collect();
        firsts.sort();
        firsts.dedup();
        assert_eq!(firsts.len(), all_presets().len(), "有风格的样例画面开头雷同");
        let compass = all_presets().iter().filter(|p| p.sample_prompt.contains("compass")).count();
        assert!(compass <= 1, "{compass} 个样例还在画罗盘");
    }

    #[test]
    fn 负向提示词只在_sdxl_上有效() {
        // Z-Image 的 cfg=1，负向被 ConditioningZeroOut 清零 ——
        // 传了不报错，只是完全不起作用。
        assert!(honors_negative("sdxl"));
        // Chroma 原来也吃，2026-10-01 下架：退役 id 判成不吃（和下面 flux 同一个方向）
        assert!(!honors_negative("chroma"));
        assert!(!honors_negative("zimage"));
        // 白名单的保守方向：退役 id（`flux`，2026-09-08 随权重删除）和任何
        // 拼错的 id 都判成「不吃负向」。反过来（默认吃）会让界面凭空多出一个
        // 不起作用的输入框 —— 那比少一个框糟得多。
        assert!(!honors_negative("flux"));
        assert!(!honors_negative("zimg"));
    }

    /// 缩略图不但要**在磁盘上**，还要**能被媒体协议放行**。
    ///
    /// 这条原来只验了 `path.is_file()` —— 于是它是绿的，而界面上整页
    /// 全是「读不出来」：缩略图在项目根的 `resources/style_thumbs/`，
    /// 不在 `app_config_dir` 里，`resolve_media_path` 直接 403，
    /// 兜底通道走同一套白名单所以也失败。
    ///
    /// **测试覆盖的是被测代码的邻居，不是它本身** —— 这是同一种假绿的
    /// 第四次（前三次：`/tmp` 符号链接、前导斜杠早退、只测解析不测调用）。
    #[test]
    fn 每张缩略图都要能被媒体协议放行() {
        // **必须用生产代码那份 roots**（`crate::media_extra_roots`），
        // 不能在测试里自己拼一份等价的 —— 拼的那份不会因为生产代码
        // 少放行一个目录而变红。变异测试第一次跑就抓到了这个。
        let roots = crate::assets::media_roots(
            &std::path::PathBuf::from("/不存在的配置目录"),
            &crate::media_extra_roots(),
        );
        let c = style_catalog();
        for p in &c.presets {
            let Some(u) = &p.thumb_url else { continue };
            let path = crate::assets::parse_media_request(u).expect("URL 解不出路径");
            assert!(path.is_file(), "{} 的缩略图指向不存在的文件", p.preset.id);
            crate::assets::resolve_media_path(&roots, &path).unwrap_or_else(|e| {
                panic!("{} 的缩略图过不了媒体协议这道门：{e}", p.preset.id)
            });
        }
        assert_eq!(
            c.thumbs_ready,
            c.presets.iter().filter(|p| p.thumb_url.is_some()).count()
        );
    }

    // ---- 生效的风格词 ----------------------------------------------------

    fn art(preset: &str, pos: &str) -> ArtDirection {
        ArtDirection {
            preset_id: preset.into(),
            positive_prompt: pos.into(),
            negative_prompt: String::new(),
            shot_notes: String::new(),
            updated_at: 0,
        }
    }

    #[test]
    fn 没自定义时用预设的词() {
        let p = &all_presets()[0];
        let a = art(&p.id, "");
        assert_eq!(a.effective_positive(), p.positive_prompt);
    }

    #[test]
    fn 自定义覆盖预设() {
        let p = &all_presets()[0];
        let a = art(&p.id, "我自己的风格词");
        assert_eq!(a.effective_positive(), "我自己的风格词");
    }

    #[test]
    fn 留空是用预设_不是不要风格() {
        // 「留空 = 清掉风格」的话，用户清一下自定义框就把预设也弄没了。
        let p = &all_presets()[0];
        assert!(!art(&p.id, "   ").effective_positive().is_empty());
    }

    #[test]
    fn 预设_id_不存在时不炸_回空() {
        assert_eq!(art("不存在的预设", "").effective_positive(), "");
    }

    #[test]
    fn 没定调时_is_set_为假() {
        assert!(!ArtDirection::default().is_set());
        assert!(art("cinematic_realism", "").is_set());
        assert!(art("", "自定义").is_set());
    }

    // ---- 拼进提示词 ------------------------------------------------------

    #[test]
    fn 短提示词的风格拼在画面之后() {
        // 反过来的话 SDXL 会让风格词把画面主体挤掉。
        let a = art("", "film grain");
        assert_eq!(apply_style("老茶师擦茶盏", &a), "老茶师擦茶盏, film grain");
    }

    /// **长提示词把风格放前面。**
    ///
    /// 分镜的提示词是润色出来的两百字完整描述，里面全是摄影语言，
    /// 末尾还常自带一句「写实摄影风格」。风格词排在那一大段后面会被
    /// 彻底淹没 —— 用户选了赛璐璐动画，出来是写实照片（实测遇到过）。
    #[test]
    fn 长提示词的风格排在前面() {
        let a = art("", "cel shading, flat colors");
        let long = "一个普通棕色瓦楞纸搬家箱放置在室内木地板上，箱体尺寸足以容纳一只小猫蜷缩其中，\
                    边缘略有磨损与折痕。柔和的自然光从侧方洒入。低角度平视构图，景深较浅，\
                    背景虚化。写实摄影风格，高清晰度。";
        let out = apply_style(long, &a);
        assert!(out.starts_with("cel shading"), "风格没排在前面");
        assert!(out.contains("瓦楞纸"));
    }

    /// 两条路都不能重复叠 —— 重渲同一镜时最容易叠两遍。
    #[test]
    fn 长短两条路都不重复叠风格() {
        let a = art("", "cel shading");
        let long = "很长的画面描述，".repeat(20);
        for p in ["老茶师擦茶盏", long.as_str()] {
            let once = apply_style(p, &a);
            assert_eq!(apply_style(&once, &a), once, "叠了两遍");
        }
    }

    /// 阈值两边各验一次 —— 改了常量却没人发现行为翻转的话，
    /// 表现是"某些镜头风格生效、某些不生效"，极难查。
    #[test]
    fn 阈值两边的行为相反() {
        let a = art("", "S");
        let short: String = "字".repeat(LONG_PROMPT);
        let long: String = "字".repeat(LONG_PROMPT + 1);
        assert!(apply_style(&short, &a).ends_with(", S"), "刚好等于阈值时应算短");
        assert!(apply_style(&long, &a).starts_with("S, "), "超过阈值时应算长");
    }

    #[test]
    fn 已经带过就不重复叠() {
        // 重渲同一镜时最容易叠两遍。
        let a = art("", "film grain");
        let once = apply_style("老茶师擦茶盏", &a);
        assert_eq!(apply_style(&once, &a), once);
    }

    #[test]
    fn 没定调时提示词原样不动() {
        assert_eq!(apply_style("老茶师擦茶盏", &ArtDirection::default()), "老茶师擦茶盏");
    }

    #[test]
    fn 画面为空时只留风格_不留一个逗号开头() {
        let a = art("", "film grain");
        assert_eq!(apply_style("   ", &a), "film grain");
    }

    // ---- 存取 ------------------------------------------------------------

    #[test]
    fn 风格设定能存回读出() {
        let dir = tempfile::tempdir().unwrap();
        let a = art("cinematic_realism", "");
        save_art(dir.path(), "p1", &a).unwrap();
        let back = load_art(dir.path(), "p1");
        assert_eq!(back.preset_id, "cinematic_realism");
        assert!(back.updated_at > 0, "没盖时间戳");
    }

    #[test]
    fn 没存过时回默认_不报错() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!load_art(dir.path(), "从没有过").is_set());
    }

    // ---- 解析风格推荐 ----------------------------------------------------

    #[test]
    fn 解析包在_styles_里的推荐() {
        let raw = r#"{"styles":[{"name":"水墨","positive_prompt":"ink wash"}]}"#;
        let s = parse_suggestions(raw).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].name, "水墨");
    }

    #[test]
    fn 裸数组也收() {
        // 模型经常直接给数组 —— 只认一种格式的话，一半的回复会被判成失败。
        let raw = r#"[{"name":"水墨","positive_prompt":"ink wash"}]"#;
        assert_eq!(parse_suggestions(raw).unwrap().len(), 1);
    }

    #[test]
    fn 少字段的条目跳过_但不拖垮整批() {
        let raw = r#"{"styles":[{"name":"缺正向词"},{"name":"好的","positive_prompt":"x"}]}"#;
        let s = parse_suggestions(raw).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].name, "好的");
    }

    #[test]
    fn 一条都没有时回_none_而不是空数组() {
        // 空数组会被界面显示成「推荐了 0 种风格」，看不出是失败了。
        assert!(parse_suggestions(r#"{"styles":[]}"#).is_none());
        assert!(parse_suggestions("模型今天不想说话").is_none());
    }
}
