//! AI 影视短剧制片工厂与剧本分镜管理系统 (Cinema Studio Backend)
//!
//! 负责 AI 短剧/影视项目管理、分集资产管理、分镜解构、ComfyUI 批量渲染调度与剧本直接视频流无缝拼接。

use crate::cinema_sheet as sheet;
use crate::error::{Error, Result};
use crate::AppState;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path, PathBuf};
use tauri::State;

const SKELETON_STANDING: &[u8] = include_bytes!("../../public/poses/pose_standing.png");
const SKELETON_RUNNING: &[u8] = include_bytes!("../../public/poses/pose_running.png");
const SKELETON_JUMPING: &[u8] = include_bytes!("../../public/poses/pose_jumping.png");
const SKELETON_WEEPING: &[u8] = include_bytes!("../../public/poses/pose_weeping.png");

/// 项目根目录（优先查找工作区 projects/，其次为 config_dir/projects）
/// 项目目录：优先仓库里的 `projects/`，否则应用数据目录下的。
///
/// **走共用的候选列表**，不是只从 cwd 往上找 —— 后者会让开发版用仓库里的、
/// 打包版用数据目录里的，**同一台机器上两份项目**。
/// 现在仓库里恰好没有 `projects/`，所以两边落在同一处 —— 那是运气不是设计。
pub fn cinema_projects_dir(config_dir: &Path) -> PathBuf {
    crate::app_root::find_plain("projects", Some(config_dir))
        .unwrap_or_else(|| config_dir.join("projects"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CinemaProject {
    pub id: String,
    pub title: String,
    pub genre: String,
    pub aspect_ratio: String, // SERIES_ASPECTS 里的一种："16:9" / "9:16" / "21:9" …
    pub description: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub episode_count: usize,
    pub cover_image: Option<String>,
    /// `"series"`（多集，共用角色与风格）或 `"standalone"`（独立项目，单集）。
    ///
    /// **不新增存储概念** —— 独立项目就是只有一集的项目，
    /// 界面上不显示分集列表而已。两者共用同一条流水线，
    /// 分叉出两套代码是没必要的复杂度。
    ///
    /// 老数据没有这个字段，默认当系列。
    #[serde(default = "default_project_kind")]
    pub kind: String,
    /// 出图/出片用哪个引擎、各类资产用什么画幅。见 [`ModelSettings`]。
    #[serde(default)]
    pub model_settings: ModelSettings,
    /// `"scripted"`（我有剧本）或 `"freeform"`（我直接创作）。
    ///
    /// **决定第 1 步在不在流程里。** 选 `freeform` 时剧本那一步整个不出现，
    /// 后面几步重新编号 —— 不是灰掉、不是软锁，是不存在。
    ///
    /// 这个设计是从 LumenX 抄的（`Series.content_mode`）：
    /// 「第一步好复杂」的正解不是把第 1 步做简单，而是让人在建项目时就
    /// 说清楚要不要它。直接写分镜提示词的人根本不需要剧本编辑器。
    ///
    /// 老数据默认 `scripted` —— 行为完全不变。
    #[serde(default = "default_content_mode")]
    pub content_mode: String,
    /// 新建镜头默认走哪条路。
    ///
    /// `FirstFrame`（画面优先）先出首帧、人看过再出视频；
    /// `Reference`（节奏优先）用 `@` 引用的资产参考图直接出视频。
    ///
    /// **以前这个默认值写死在 `ShotMode` 的 `#[default]` 上**，
    /// 于是每个项目的每个镜头都是首帧流程，想用参考图得一个一个翻。
    /// 单个镜头仍然可以覆盖它。
    #[serde(default)]
    pub default_shot_mode: ShotMode,
    /// 项目里每个角色的默认形象：资产 id → 形象 id。没写的用主形象
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub default_looks: std::collections::BTreeMap<String, String>,
    /// **绑定的剧本编辑器项目**（`studio/scripts/projects.json` 里的 id）。
    ///
    /// 系列左栏的「故事原文」、角色与资产页的「从设定文档建资产」都要找到它。
    /// 以前只按名字对，而系列名常带书名号（「《回收师》」对不上「回收师」），一对不上就什么都找不到。
    /// 按名字认出来一次就存成绑定，之后按 id 找，两边谁改名都不断。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script_project_id: Option<String>,
}

fn default_content_mode() -> String {
    "scripted".to_string()
}

/// 项目级的「生成设置」。
///
/// 照 LumenX 的 `ModelSettings`（MIT，Copyright 2026 Alibaba）：出图分两条路
/// —— 资产是文生图、分镜首帧是图生图 —— 各自可以用不同引擎；
/// 三类资产各有各的画幅（角色竖、场景横、道具方）。
///
/// **不含分镜画幅。** 项目本身已经有 `aspect_ratio` 了，
/// 再存一份就是两个所有者，迟早对不上。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelSettings {
    /// 资产出图（文生图）用哪个引擎。
    #[serde(default = "default_t2i_engine")]
    pub t2i_engine: String,
    /// 分镜首帧（图生图 / 带参考图）用哪个引擎。
    #[serde(default = "default_i2i_engine")]
    pub i2i_engine: String,
    /// 出片用哪个引擎。
    #[serde(default = "default_video_engine")]
    pub video_engine: String,
    /// 三类资产各自的画幅 id（见 `studio::aspect_options`）。
    #[serde(default = "default_character_aspect")]
    pub character_aspect: String,
    #[serde(default = "default_scene_aspect")]
    pub scene_aspect: String,
    #[serde(default = "default_prop_aspect")]
    pub prop_aspect: String,
    /// 提示词线路（A 中文结构 / B H3 官方六段）。合成和出片都用它。
    #[serde(default)]
    pub prompt_route: crate::shot_table::PromptRoute,
    /// 出片设置（画质、清晰度、比例、每镜几次、声音、字幕）。出片前弹出的选项卡改的就是它，整个项目统一。
    #[serde(default)]
    pub render: RenderSettings,
}

/// 出片画质 = H3 的采样档。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RenderQuality {
    /// Turbo 384p 低内存预设：最快，只看构图动作
    Fast,
    /// Turbo 4 次求值
    Standard,
    /// 双采法：2 次基础模型 + 4 次 Turbo（上游官方 Balance 档）
    #[default]
    Staged,
    /// 20 点 / 19 次求值，质量优先，很慢
    Dense,
    /// LightX2V 首尾帧 4 步（2026-10-07 加，见 `weetodd::H3Tier::LightX2V`）
    #[serde(rename = "lightx2v")]
    LightX2V,
}

impl RenderQuality {
    pub fn tier(self) -> crate::agent_runtime::weetodd::H3Tier {
        use crate::agent_runtime::weetodd::H3Tier;
        match self {
            // 「快速」10-07 起和「标准」一样（都是 Larry 4 次）；界面不再给，老项目存的照旧能出
            RenderQuality::Fast => H3Tier::Preview,
            RenderQuality::Standard => H3Tier::Standard,
            RenderQuality::Staged => H3Tier::Staged,
            RenderQuality::Dense => H3Tier::Dense,
            RenderQuality::LightX2V => H3Tier::LightX2V,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            // 和前端 `renderOptions.QUALITIES` 的名字一致（10-07 换成 Larry / LightX2V 之后改的）
            RenderQuality::Fast => "Larry 4 步",
            RenderQuality::Standard => "Larry 4 步",
            RenderQuality::Staged => "Larry 6 步",
            RenderQuality::Dense => "精细 19 步",
            RenderQuality::LightX2V => "LightX2V 4 步",
        }
    }
}

/// 出片设置。默认值来自 2026-09-14 第 11 镜同种子对照：双采 896×512 16 分钟，比原来的 Turbo 1280×736（32 分钟）快一倍。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RenderSettings {
    /// **参考出片**（Ref2VA）的画质。字段名沿用老的 `quality`：10-07 之前只有这一份，老项目存的就是它
    #[serde(default)]
    pub quality: RenderQuality,
    /// **首帧出片**（FL2VA）的画质。2026-10-07 用户：「要区分首帧生和参考生的画质选项」。
    /// 默认 LightX2V：10-07 EP03 第 10 镜首帧出片四方对照里它最好（Larry 多出一个人、铭牌冒乱字）
    #[serde(default = "default_keyframe_quality")]
    pub keyframe_quality: RenderQuality,
    /// `480p` / `576p` / `720p`（短边 512 / 576 / 736）
    #[serde(default = "default_render_resolution")]
    pub resolution: String,
    /// 画面比例，如 `16:9`。空 = 跟项目的比例
    #[serde(default)]
    pub aspect: String,
    /// 每镜出几次（每次换种子，挑一条用）
    #[serde(default = "default_render_takes")]
    pub takes: u8,
    #[serde(default = "default_true")]
    pub generate_audio: bool,
    /// 台词字幕烧进画面
    #[serde(default = "default_true")]
    pub burn_subtitles: bool,
    /// H3 的采样方法（2026-10-12）。默认 euler；Combat 类 LoRA 作者推荐 res_multistep（见 `weetodd::H3Sampler`）
    #[serde(default, skip_serializing_if = "crate::combat::is_default_sampler")]
    pub sampling: crate::agent_runtime::weetodd::H3Sampler,
    /// **战斗 LoRA 预设**：打斗镜按它的动作类型（`StoryboardShot::combat_style`）挂这一组 LoRA + 触发词。
    /// 键是动作类型 id（unarmed / melee / gunfu / energy / xianxia）。空 = 打斗镜也不挂（具体配哪些等 A/B 定）
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub combat: std::collections::BTreeMap<String, crate::combat::CombatPreset>,
}

fn default_keyframe_quality() -> RenderQuality {
    RenderQuality::LightX2V
}
fn default_render_resolution() -> String {
    "480p".into()
}
fn default_render_takes() -> u8 {
    1
}
fn default_true() -> bool {
    true
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self { quality: RenderQuality::default(), keyframe_quality: default_keyframe_quality(), resolution: default_render_resolution(), aspect: String::new(), takes: 1, generate_audio: true, burn_subtitles: default_burn_subtitles(), sampling: Default::default(), combat: Default::default() }
    }
}

impl RenderSettings {
    /// 出片尺寸：按清晰度定短边、按比例算长边，32 对齐（引擎硬约束）。认不出的比例按 16:9。
    pub fn size(&self, project_aspect: &str) -> (u32, u32) {
        let short = match self.resolution.as_str() {
            "576p" => 576.0,
            "720p" => 736.0,
            _ => 512.0,
        };
        let aspect = if self.aspect.trim().is_empty() { project_aspect } else { self.aspect.as_str() };
        let (a, b) = aspect
            .split_once(':')
            .and_then(|(a, b)| Some((a.trim().parse::<f64>().ok()?, b.trim().parse::<f64>().ok()?)))
            .filter(|(a, b)| *a > 0.0 && *b > 0.0)
            .unwrap_or((16.0, 9.0));
        let (w, h) = if a >= b { (short * a / b, short) } else { (short, short * b / a) };
        crate::agent_runtime::weetodd::fit_dimensions((w.round() as u32, h.round() as u32))
    }
}

impl Default for ModelSettings {
    fn default() -> Self {
        Self {
            t2i_engine: default_t2i_engine(),
            i2i_engine: default_i2i_engine(),
            video_engine: default_video_engine(),
            character_aspect: default_character_aspect(),
            scene_aspect: default_scene_aspect(),
            prop_aspect: default_prop_aspect(),
            prompt_route: Default::default(),
            render: RenderSettings::default(),
        }
    }
}

impl ModelSettings {
    /// 这一类资产用什么画幅。
    ///
    /// **认不出的类型回角色那档**（竖）—— 人像是最常见的，
    /// 而且竖幅在别的类型上只是不好看，不会失败。
    pub fn aspect_for(&self, kind: &str) -> &str {
        match kind {
            "scene" => &self.scene_aspect,
            "prop" => &self.prop_aspect,
            _ => &self.character_aspect,
        }
    }
}

/// 新项目的文生图底模 = 全局默认（Qwen-Image 2.1，见 `image_engine::DEFAULT_ENGINE`）。
/// 已有项目存着自己的值，不受影响。
fn default_t2i_engine() -> String {
    crate::agent_runtime::image_engine::DEFAULT_ENGINE.id().to_string()
}

/// 图生图底模**不跟着换**：Qwen 2.1 没有图生图通路（`supports_i2i`），
/// 默认到它上面等于每次改图都要回落。
fn default_i2i_engine() -> String {
    crate::agent_runtime::image_engine::I2I_FALLBACK.id().to_string()
}
fn default_video_engine() -> String {
    "h3".to_string()
}
/// 角色竖（全身站姿 + 大头特写的版面），场景横（环境全景），道具方。
fn default_character_aspect() -> String {
    "portrait".to_string()
}
fn default_scene_aspect() -> String {
    "landscape".to_string()
}
fn default_prop_aspect() -> String {
    "square".to_string()
}

fn default_project_kind() -> String {
    "series".to_string()
}

/// 一镜走哪条链路。**这是逐镜的开关，不是项目级的。**
///
/// 一集里两种镜头必然混着用：需要精确控构图/服装/道具位置的镜头走首帧，
/// 人物要动起来的对话和动作镜头走参考。项目级只存一个新建镜头时的默认值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShotMode {
    /// 提示词 → 出首帧（可抽卡选一张）→ 首帧 + 提示词 → 视频。
    #[default]
    FirstFrame,
    /// 提示词 + @ 引用的资产参考图 → 直接出视频。
    Reference,
}

/// 这一镜的起点从上一镜那里继承什么。
///
/// # 为什么是三档，不是一个勾选框
///
/// 用户 2026-09-20 提的是「要不要以上一镜作为**首帧参考**」。但在 H3 这里
/// 这句话对应三件**不同**的事，界面上混成一个开关必然误导：
///
/// | | 上一镜那一帧的地位 | 代价 |
/// |---|---|---|
/// | [`Continuity::None`] | 无 | 各拍各的 |
/// | [`Continuity::RefVideo`] | 运镜、光线、布景的**延续依据** | 不锁画面内容，接点可能有跳变 |
/// | [`Continuity::FirstFrame`] | **就是第 0 帧**（FL2VA） | 姿势接不上时开头会跳/扭曲 |
///
/// # 实测（2026-09-20，E05 第 1 镜尾段 → 第 2 镜）
///
/// `RefVideo` 这一档：第 1 镜最后一帧 vs 第 2 镜第 0 帧，背景差 **4.10**、
/// 整幅差 **30.60**，而**第 1 镜自己隔 6 秒的正常波动是 51.05** ——
/// 跨镜比同镜头内部还稳。耗时 63.9 分钟，比不带参考的 68.0 分钟还略快。
///
/// 同一次实测也确认了它**管不了身份**：卡通头像的脸在两镜之间漂了。
/// 人物长相要靠 `<Picture N>` 的角色设定图，视频参考只管机位光线布景。
///
/// # 默认必须是 `None`
///
/// 已有的 E01~E05 都没有这个字段。默认成别的会让所有老镜头的指纹一起变、
/// 全部误报「要重新合成」—— 和 `looks` / `performance` 是同一个坑，
/// 见 [`crate::prompt_input::fields_json`]。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Continuity {
    /// 不接。跨场、跳时间的镜头接了反而串景。
    #[default]
    None,
    /// 上一镜尾段当运镜 / 光线 / 布景的参考（`WeeToddH3ReferenceVideo`）。
    RefVideo,
    /// 上一镜最后一帧当字面首帧（FL2VA）。
    FirstFrame,
}

impl StoryboardShot {
    /// 这一镜实际的衔接方式。**所有判断都走这里。**
    ///
    /// 把老字段 `continues_previous` 折进新的 [`Continuity`]：
    /// 它为 true 而新字段没设时，等价于 [`Continuity::FirstFrame`]
    /// —— 两者做的是同一件事（拿上一镜末帧当这一镜的起点）。
    pub fn effective_continuity(&self) -> Continuity {
        if self.continuity == Continuity::None && self.continues_previous {
            // 唯一允许读老字段的地方 —— 下面那条测试认这个记号 [折叠点]
            Continuity::FirstFrame
        } else {
            self.continuity
        }
    }
}

/// 接上一镜时用它的哪一条成片。
///
/// **指纹和出片必须读同一个函数。** 两边各挑各的，就会出现
/// 「指纹说没过期、出片拿的却是另一条片子」—— 而这种不一致不报错。
///
/// 判据是「最新的那一条」，不分线路：用户眼里「上一镜的成片」就是他最后
/// 看到的那个，不是「上一镜在这条线路上的那个」。
pub fn continuity_source(prev: &StoryboardShot) -> Option<&VideoTake> {
    prev.takes.last()
}

impl Continuity {
    /// 要不要去拿上一镜的成片。
    pub fn needs_prev_take(self) -> bool {
        !matches!(self, Continuity::None)
    }

    /// 给人看的名字。
    pub fn label(self) -> &'static str {
        match self {
            Continuity::None => "不接",
            Continuity::RefVideo => "运镜与光线",
            Continuity::FirstFrame => "接住最后一帧",
        }
    }
}

/// 这一「次」是怎么来的。
///
/// # 为什么非分不可
///
/// 画布回填的片**没走线路 A / B**，它是用户在画布上自己接出来的。
/// 可 `VideoTake.route` 是个只有 A|B 的枚举，随便给一个的话，
/// 两处判断会当场读错，而且**都不报错**：
///
/// - 「这一镜还没在这条线路出过片」（`shot_ids_for_route`）会以为出过了，
///   批量出片就把这一镜跳过去；
/// - 「最新那条 take 是不是按当前字段出的」（`stale_take_issues`）会读到
///   这条没有指纹的回填片，于是**闭嘴** —— 把真正该报的过期警告盖掉。
///
/// 反过来，「上一镜的成片」（`continuity_source`）**要认它** ——
/// 用户眼里那就是他最后看到的那条片子。所以不能一刀切，得逐处判。
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TakeOrigin {
    /// 分镜页出片出的
    #[default]
    Render,
    /// 画布回填的
    Canvas,
}

/// 一镜出过的一次（「次」）。**写明是哪条线路出的**（方案：生成记录写明线路），A / B 对比看片靠它。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct VideoTake {
    pub route: crate::shot_table::PromptRoute,
    /// 视频文件的绝对路径
    pub path: String,
    pub created_at: u64,
    pub seed: u64,
    /// 挂了几张参考图（0 = 文生）。**老数据只有这个数**。
    #[serde(default)]
    pub pictures: usize,
    /// **挂的是哪几张，按顺序。**
    ///
    /// # 为什么光有张数不够
    ///
    /// 「它到底用了哪张场景图」是回看历史 take 时**唯一**想知道的事，
    /// 而只存张数永远答不出来（2026-09-20 Codex 的画布评估指出）。
    /// 顺序也有意义 —— `<Picture N>` 的编号就是这个顺序，
    /// 场景在前、人物在后，换个顺序出来的是另一张图。
    ///
    /// 老数据是空的；显示时空就退回 `pictures` 那个数。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub picture_paths: Vec<String>,
    /// 出片档位，如「双采 · 896×512」。老数据没有
    #[serde(default)]
    pub quality: String,
    /// **出这一条时发出去的完整提示词。**
    ///
    /// # 为什么要存
    ///
    /// 原来只存路线、路径、时间、seed、图数、档位。重新合成提示词之后，
    /// **历史 take 就没法复现了** —— 你看着两条片子不一样，却说不出
    /// 当时发的是什么（2026-09-19 Codex Astra 的审查指出）。
    ///
    /// 存全文而不是指纹：指纹只能回答「变没变」，回答不了「变成了什么」。
    /// 而「当时到底发了什么」正是回看历史 take 时唯一想知道的事。
    #[serde(default)]
    pub prompt: String,
    /// 出这一条时镜头表字段的指纹（`prompt_input::fields_hash`）。
    ///
    /// 和当前的对不上 = 这条片子是按旧版镜头表出的。**「有 take」不等于
    /// 「按当前版本出过片」** —— 这两件事原来混在一起，没法分别标过期。
    #[serde(default)]
    pub fields_hash: String,
    /// 出这一条时用的画风正文（截断）。风格是项目级可变状态，
    /// 改了之后重出会是另一种画风 —— 存下来才说得清历史那条是按什么出的。
    #[serde(default)]
    pub style: String,
    /// 分镜页出的，还是画布回填的。见 [`TakeOrigin`]。
    #[serde(default)]
    pub origin: TakeOrigin,
}

/// 一场整段出的一次成片（长镜头）。见 [`EpisodeData::scene_renders`]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SceneRender {
    pub scene_no: u32,
    pub route: crate::shot_table::PromptRoute,
    /// 整条视频的绝对路径
    pub path: String,
    pub created_at: u64,
    pub seed: u64,
    #[serde(default)]
    pub quality: String,
    /// 按窗口顺序的镜头 id（一镜一个窗口）
    #[serde(default)]
    pub shot_ids: Vec<String>,
    /// 各窗口发出去的提示词全文（和逐镜 take 存 `prompt` 同一个理由：重新合成之后还说得清当时发的是什么）
    #[serde(default)]
    pub prompts: Vec<String>,
    /// 开跑时各镜的字段指纹
    #[serde(default)]
    pub fields_hashes: Vec<String>,
    /// 各窗口挂的参考图
    #[serde(default)]
    pub picture_paths: Vec<Vec<String>>,
}

/// 这一场最新的一次整段成片。`route` 为 `None` 时不挑线路
pub(crate) fn scene_render(ep: &EpisodeData, scene_no: u32, route: Option<crate::shot_table::PromptRoute>) -> Option<&SceneRender> {
    ep.scene_renders
        .iter()
        .filter(|r| r.scene_no == scene_no && route.map_or(true, |x| r.route == x) && Path::new(&r.path).is_file())
        .max_by_key(|r| r.created_at)
}

/// **这一场是不是按整条长镜头用。** 合成一集、FCP、剪映、组装页读的都是这一个判断 ——
/// 各判各的就会出现「组装页显示整段出过、合成出来却缺这三镜」。
/// 2026-10-01 用户截图：第 5 场整段出过，组装页第 22~24 镜一直显示「未出片」，
/// 「合成一集」也照样把这三镜当缺镜跳过（只有 FCP 认）。
///
/// 判据：这一场设成了整段出（`long_take_scenes`），并且出过整段片（当前线路优先，
/// 另一条线路出过也用 —— 总比空着强）。这时各镜自己**单独出**的片不用：
/// 长镜头不切开（用户 09-30 定的），没法把一镜换进整条里。要用单镜的，把这一场的出法改回分镜。
///
/// 前端 `storyboardView.ts` 的 `longTakeClip` 照抄这一条，改的时候两边一起改。
pub(crate) fn long_take_clip(ep: &EpisodeData, scene_no: u32, route: crate::shot_table::PromptRoute) -> Option<&SceneRender> {
    if !ep.long_take_scenes.contains(&scene_no) {
        return None;
    }
    scene_render(ep, scene_no, Some(route))
        .or_else(|| scene_render(ep, scene_no, None))
        .filter(|r| scene_render_matches(ep, r))
}

/// 整段片还是不是这一场现在这几镜出的。
///
/// **重新生成过这一场**（镜头 id、镜数都换了）的话，那条整段片拍的是旧镜头 —— 原来照样拼进成片、
/// 剪映 / FCP 照样导出、批量出片还当它出过了跳过（2026-10-03 审查）。按场号认片子认不出这件事，要对镜头 id。
/// 老数据没记 `shot_ids` 的照旧认（不知道是按什么出的，不乱拦）。
/// 只改了字段、镜头还是那几镜的不算换了 —— 那和逐镜的旧 take 一样报「要重出才对得上」（`stale_take_issues`），片子照用。
fn scene_render_matches(ep: &EpisodeData, r: &SceneRender) -> bool {
    if r.shot_ids.is_empty() {
        return true;
    }
    let mut now: Vec<&StoryboardShot> = ep.storyboard.iter().filter(|s| s.scene_no == r.scene_no).collect();
    now.sort_by_key(|s| s.shot_num);
    now.iter().map(|s| s.id.as_str()).eq(r.shot_ids.iter().map(String::as_str))
}

/// 一集按播放顺序排出来的一段：要么一镜，要么一整场长镜头。
pub(crate) enum EpisodeSegment<'a> {
    Shot(&'a StoryboardShot),
    /// 一整场长镜头。放在这一场**第一次出现**的位置（和 FCP 导出同一个规矩），
    /// 这一场后面的镜头不再各占一段；`shots` 按镜号排。
    LongTake { scene_no: u32, render: &'a SceneRender, shots: Vec<&'a StoryboardShot> },
}

/// 一集由哪些片段、按什么顺序组成。合成一集和剪映导出都按它走。
pub(crate) fn episode_segments(ep: &EpisodeData, route: crate::shot_table::PromptRoute) -> Vec<EpisodeSegment<'_>> {
    let mut out = Vec::new();
    let mut placed: Vec<u32> = Vec::new();
    for shot in &ep.storyboard {
        match long_take_clip(ep, shot.scene_no, route) {
            Some(render) => {
                if placed.contains(&shot.scene_no) {
                    continue;
                }
                placed.push(shot.scene_no);
                let mut shots: Vec<&StoryboardShot> = ep.storyboard.iter().filter(|s| s.scene_no == shot.scene_no).collect();
                shots.sort_by_key(|s| s.shot_num);
                out.push(EpisodeSegment::LongTake { scene_no: shot.scene_no, render, shots });
            }
            None => out.push(EpisodeSegment::Shot(shot)),
        }
    }
    out
}

/// 一句台词。
///
/// **说话人写资产名**（和 `prompt` 里 `[角色:名字]` 用的是同一个名字）——
/// H3 的说话人 ID 只有在这个称呼能在画面描写里指得到人时才发得出去，
/// 见 `h3_prompt::speakers_anchored`。名字对不上就退回不标记，不报错。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct DialogueLine {
    pub speaker: String,
    pub line: String,
    /// 音高 / 音色 / 语速 / 口音。**和画面描写同一种语言** ——
    /// 中文描写里塞英文音色词（或反过来）实测会让说话人绑定失效。
    #[serde(default)]
    pub voice: String,
    #[serde(default)]
    pub emotion: String,
    /// **只给这一句**用的 TTS 音色模板名，盖过角色资产上的 `voice_id`。
    ///
    /// 空 = 用角色的（绝大多数情况）。存在是因为确实有例外：
    /// 回忆里的少年版、电话那头的失真、旁白由某个角色兼任。
    ///
    /// **和上面的 `voice` 不是一回事**：那个是给 H3 看的中文描述
    /// （「沙哑，带川渝口音」），这个是点名一份参考音频（`catboss`）。
    /// 两条音频链路各用一个，混用的话两边都静默失效。
    #[serde(default)]
    pub voice_override: String,
    /// 画外音。为真时编译出 off-screen voiceover 并写明嘴唇不动 ——
    /// 不写那一句，画面里的人会跟着旁白动嘴。
    #[serde(default)]
    pub off_screen: bool,
}

/// 一个人在画面里站哪。**屏幕空间**，不是世界坐标。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct StageSubject {
    /// 角色名（同 `[角色:名字]`）
    pub actor: String,
    /// 左 / 中 / 右
    #[serde(default)]
    pub zone: String,
    /// 前景 / 中景 / 后景
    #[serde(default)]
    pub depth: String,
    /// 朝向：面向镜头 / 背对 / 面向左 / 面向右
    #[serde(default)]
    pub facing: String,
    /// 体态：站 / 坐 / 躺 / 蹲
    #[serde(default)]
    pub posture: String,
}

/// 一镜挂的关键帧。**出片模式按它自动定**（用户 2026-09-27：「根据导入情况自动识别」）：
///
/// | 首帧 | 尾帧 | H3 怎么出 |
/// |---|---|---|
/// | 无 | 无 | 参考生视频（资产图当身份参考）或文生 |
/// | 有 | 无 | 首帧图生视频（I2VA） |
/// | 有 | 有 | 首尾帧（FL2VA） |
/// | 无 | 有 | 不出，要先补首帧 |
///
/// # 为什么要有这条路
///
/// 《回收师》E01 第 1 镜只靠参考图出：场景、道具、机位全是模型自己编的，
/// 激光刀被装到了机械臂上。Codex 同一个镜头先用资产出**首帧 / 尾帧**锁住画面，
/// 再首尾帧生视频，一次就对。参考图管「长什么样」，管不了「画面是什么」。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keyframes {
    #[serde(default)]
    pub first: bool,
    #[serde(default)]
    pub last: bool,
}

impl Keyframes {
    pub const NONE: Keyframes = Keyframes { first: false, last: false };

    pub fn is_none(&self) -> bool {
        !self.first && !self.last
    }

    /// 按磁盘上的文件算。
    pub fn on_disk(shots_dir: &Path, shot_id: &str) -> Self {
        Keyframes {
            first: shots_dir.join(format!("{shot_id}.png")).is_file(),
            last: end_frame_path(shots_dir, shot_id).is_file(),
        }
    }

    /// 界面和报错里的叫法。
    pub fn label(self) -> &'static str {
        match (self.first, self.last) {
            (true, true) => "首尾帧",
            (true, false) => "首帧",
            (false, true) => "只有尾帧",
            (false, false) => "参考生视频",
        }
    }

    /// 这一镜出片要不要走关键帧（首帧必须在）。
    pub fn drives_video(self) -> bool {
        self.first
    }
}

/// 整集保存时，导演世界那两份状态以磁盘为准（界面拿着的多半是打开导演世界之前的那份）：
/// - `director_scene` **一律**用盘上的（只有 `cinema_set_director_scene` 写它）；
/// - `director_camera` 界面那份没有时用盘上的（导演世界里「绑定机位」写的；分镜详情改运镜备注时会带着它）。
fn keep_director_state(disk: &EpisodeData, episode: &mut EpisodeData) {
    for shot in &mut episode.storyboard {
        let Some(old) = disk.storyboard.iter().find(|s| s.id == shot.id) else { continue };
        shot.director_scene = old.director_scene.clone();
        if shot.director_camera.is_none() {
            shot.director_camera = old.director_camera.clone();
        }
    }
}

/// 按磁盘把每一镜的 [`Keyframes`] 重算一遍。
pub(crate) fn sync_keyframes(shots_dir: &Path, ep: &mut EpisodeData) {
    for shot in &mut ep.storyboard {
        shot.keyframes = Keyframes::on_disk(shots_dir, &shot.id);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectorCameraBinding {
    pub name: String,
    pub camera: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<serde_json::Value>,
    #[serde(default)]
    pub motion_note: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rgb_video: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth_video: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_frame: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_frame: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryboardShot {
    pub id: String,
    pub shot_num: u32,
    /// **这一镜是按「一场一镜出」生成的**（见 [`crate::one_take`]）：镜数和时长是代码给的，
    /// 镜内按时间轴写切镜和运镜。
    ///
    /// 场的**模式**存在集上（`EpisodeData::one_take` / `scene_takes`），这里记的是
    /// 「这一镜当初是怎么生成的」这个事实 —— 校验（允许镜内切镜）和线路 B（允许写硬切）
    /// 读它，不用把集的设置一路传下去。改了场的模式，这一场会重新生成，两边不会长期对不上。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub one_take: bool,
    /// 首帧文件当前这一版（mtime 毫秒）。**每次读盘重算，不是存下来的。**
    ///
    /// 界面拿它给媒体 URL 加一个 `v=` —— 首帧固定写 `shots/<id>.png`，
    /// 重出一次内容换了、路径没变，`<img src>` 那个字符串一模一样，
    /// **React 不会重设 src，浏览器也就不会去发请求**，
    /// 于是界面上一直是旧图（要切一次页面才会换）。
    /// 光靠 HTTP 那层的 ETag 不够 —— 请求压根不会发出去。
    #[serde(default)]
    pub image_rev: u64,
    /// 同上，出片的那个文件。
    #[serde(default)]
    pub video_rev: u64,
    pub scale: String, // "特写 (Close-Up)", "中景 (Medium)", "全景 (Wide)", "大特写 (ECU)", "远景 (ELS)"
    /// 运镜 —— `shot-language.json` 的 `move` 组 id（16 条）。
    pub camera_movement: String,
    /// 机位 —— `shot-language.json` 的 `angle` 组 id（平视/仰拍/俯拍/鸟瞰/
    /// 荷兰角/过肩/主观）。
    ///
    /// **和运镜是两件事。** 以前只有运镜一栏，而那一栏里混着「俯视」——
    /// 那是机位，不是运动。对比过 LumenX 和 libtv：两家都把景别/机位/运镜
    /// 分开（LumenX 三栏并列，libtv 把机位写进运镜那句话里），
    /// 我们把它们挤进了一个枚举。
    ///
    /// 旧数据没有这一栏，`default` 是空串 —— 空串表示「没指定」，
    /// 出片时不往提示词里加机位，而不是硬塞一个平视。
    #[serde(default)]
    pub camera_angle: String,
    /// 这一镜的运镜**为什么这么运**，一句话。
    ///
    /// 枚举给机器（ffmpeg 的运镜预览、词表稳定性），这一句给模型。
    /// 词表里 `push_in` 的提示词是通用句「镜头缓慢推近，逐步收紧到主体」，
    /// 而 LumenX 写的是「镜头由全景缓缓推近，逐渐聚焦于小猫与纸箱的互动，
    /// **悬念感随推进逐步增强**」—— 后半句是意图，通用句给不出来。
    /// libtv 同理：「缓缓向前推镜，**增加帝王的压迫感与不容置疑的威仪**」。
    ///
    /// 空着就退回词表那句通用的，不会更差。
    #[serde(default)]
    pub camera_note: String,
    /// 导演世界的结构化机位/轨迹。几何 JSON 只用于 UI 与控制链路。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub director_camera: Option<DirectorCameraBinding>,
    /// 工作区里打开导演世界时这一镜的场面：人偶、相机、命名机位、运镜路径，以及最近一次输出的
    /// 人偶稿（`outputs`）/ 运镜首尾帧（`trajectory`）。**只由 `cinema_set_director_scene` 写**，
    /// 整集保存时以磁盘为准 —— 界面拿着的那份多半是打开导演世界之前的。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub director_scene: Option<serde_json::Value>,
    pub dialogue: String,
    /// **已废弃，单数。** 双人对话镜头在这个字段上表达不了 ——
    /// 保留只为读旧数据，[`StoryboardShot::all_character_ids`] 会把它并进
    /// `character_ids`。新代码一律写 `character_ids`。
    #[serde(default)]
    pub character_id: String,
    /// 这一镜出场的角色（资产 id）。
    #[serde(default)]
    pub character_ids: Vec<String>,
    /// 这一镜的场景资产 id。
    #[serde(default)]
    pub scene_id: Option<String>,
    /// 这一镜的道具资产 id。
    #[serde(default)]
    pub prop_ids: Vec<String>,
    /// 这一镜走哪条链路。见 [`ShotMode`]。
    #[serde(default)]
    pub mode: ShotMode,
    /// **这一镜怎么接上一镜。** 见 [`Continuity`]。
    ///
    /// ⚠️ **不要直接读这个字段** —— 用 [`StoryboardShot::effective_continuity`]。
    /// 老数据的 `continues_previous` 要在那里折进来。
    #[serde(default)]
    pub continuity: Continuity,
    /// 「接上一镜」是用户手动选的 —— 自动设（`shot_route::apply_auto_continuity`）不再覆盖它。
    /// 2026-10-07 用户：「需要在分镜后自动处理接上一镜的选项，不能光写推荐」
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub continuity_manual: bool,
    /// **出片方式手动指定**（2026-10-11 用户：「三样都改」—— 规则判错时能直接改）。`None` = 跟着内容建议
    /// （`shot_route::content_route`）；`Keyframe` / `Reference` = 用户在镜头详情里选的，建议、按场出首帧、接上一镜都照它。
    /// 实际走哪条仍看有没有首帧（`shot_route::actual_route`）：选了参考的不出首帧，选了首帧的照常要出首帧。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_choice: Option<crate::shot_route::ShotRoute>,
    /// **打斗镜的动作类型**（2026-10-12 打斗编排）：`unarmed` 徒手 / `melee` 冷兵器 / `gunfu` 枪斗 / `energy` 能量武器 / `xianxia` 仙侠。
    /// 空 = 不是打斗镜。镜头表按剧本判断，镜头详情能改；出片时据此挂项目设置里的战斗 LoRA 预设。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub combat_style: String,
    /// 这一镜**动起来**的部分：谁在动、怎么动、事情怎么发展。
    ///
    /// # 为什么和 `prompt` 分开
    ///
    /// `prompt` 是**静态**的那一帧长什么样，喂首帧；`action` 是沿时间轴的
    /// 变化，喂出片。以前两边读的是同一个 `prompt` ——
    /// 于是首帧提示词里混着动作（对静态图没用，还挤占篇幅），
    /// 出片提示词里混着静态描写（而 H3 要的是变化）。
    /// **两边都在读一份不是为自己写的稿子。**
    ///
    /// 空着就退回老行为（两边都只有 `prompt`），旧数据不受影响。
    #[serde(default)]
    pub action: String,
    /// 这一镜**结束时**画面该是什么样。空 = 不约束结尾。
    ///
    /// # 两种用法，成本差很远
    ///
    /// - **文字约束**（总是生效）：编进出片提示词，告诉模型这段该落到哪儿。
    ///   零成本。
    /// - **真尾帧图**（只在 `<shot>_end.png` 确实存在时）：走首尾帧
    ///   （FL2VA），是硬约束。**我们不自动生成它** —— 每镜多出一张图
    ///   （本机 45 秒起），而且 H3 的首尾帧要加载 66.73 GB 的视觉编码器、
    ///   内存峰值约 54 GB。要用就自己放一张进去。
    ///
    /// 和 `continues_previous` 是一对：这一镜**落在哪**决定了下一镜
    /// **从哪开始**接得顺不顺。
    #[serde(default)]
    pub end_frame: String,
    /// 结构化台词。**非空时压过 `dialogue` 那个字符串。**
    ///
    /// `dialogue` 是人写、人读的那一份（剧本原文，界面上显示的也是它），
    /// 留着不动；这一份是给模型用的，多了说话人绑定、音色和画外音标记。
    /// 空着就按老办法从 `dialogue` 现拆（`shot_dialogue_lines`）。
    #[serde(default)]
    pub dialogue_lines: Vec<DialogueLine>,
    /// 情绪**轨迹**，不是状态。两个标签意味着一次转折，编译成 `mood: A → B`。
    #[serde(default)]
    pub emotion_tags: Vec<String>,
    /// 站位。**屏幕空间**（左/中/右 × 前/中/后），不是世界坐标 ——
    /// 我们没有几何来兑现世界坐标，写了也只是一段模型可以不理的文字。
    #[serde(default)]
    pub blocking: Vec<StageSubject>,
    /// 环境音：风雨、脚步、引擎、衣料摩擦。编进 H3 的 `overall_soundscape`。
    #[serde(default)]
    pub ambient_sound: String,
    /// 音效：具体的一响（关门、碎裂、通知音）。和环境音一起编进 soundscape。
    #[serde(default)]
    pub sound_effect: String,
    /// 配乐 —— **只有观众听得见的**那种。角色听得见的音乐（收音机、有人在唱）
    /// 是画面内事件，写进画面描述，不是这里。
    ///
    /// **空字符串会编译成 `N/A`，而不是省略这个字段。** 这是压住「无端配背景
    /// 音乐」的那一下：H3 的音轨是和画面一起无条件生成的，字段缺席它就自己编。
    #[serde(default)]
    pub score: String,
    pub prompt: String,
    /// 第几场（从 1 开始）和场名（「地点 · 时间」）。由生成镜头表时的分场结果写入。
    #[serde(default)]
    pub scene_no: u32,
    #[serde(default)]
    pub scene_name: String,
    /// 这一镜为什么存在（唯一职责）。照 drama-skills 分镜 SKILL。
    #[serde(default)]
    pub duty: String,
    /// 剧本来源：这一镜负责拍剧本里的哪几句。
    #[serde(default)]
    pub source: String,
    /// 光影氛围：光源依据、方向、光比、色温，只留一个主光。
    #[serde(default)]
    pub lighting: String,
    /// 画面里要交代的场景和道具，带尺寸和摆放。
    #[serde(default)]
    pub scene_items: Vec<crate::shot_table::SceneItem>,
    /// **这一场的固定布景**（整场共用，镜头表每场写一次）。见 [`crate::shot_table::SceneStage`]。
    /// 老镜头没有，空的不写盘。
    #[serde(default, skip_serializing_if = "crate::shot_table::SceneStage::is_empty")]
    pub stage: crate::shot_table::SceneStage,
    /// 按时间段写的画面描述。见 [`crate::shot_table::ShotSegment`]。
    #[serde(default)]
    pub segments: Vec<crate::shot_table::ShotSegment>,
    /// 参考图槽位：挂哪几张、按什么顺序、各自控制什么。
    #[serde(default)]
    pub ref_slots: Vec<crate::shot_table::RefSlot>,
    /// 这一镜里某个角色换用的形象：资产 id → 形象 id。**单镜覆盖**，没写的用项目默认形象。
    /// 见 `looks::effective_look`。
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub look_overrides: std::collections::BTreeMap<String, String>,
    /// 每条提示词线路合成出来的两条提示词（分镜 + 视频运动），**按线路分别存**。
    /// 出片 / 出首帧原样发送当前线路那一份，不再追加任何内容。
    #[serde(default)]
    pub prompts: crate::shot_table::ShotPromptSet,
    /// 镜头表里写了、资产库里没有的资产（合成时按文字兜底）
    #[serde(default)]
    pub unlinked_assets: Vec<crate::shot_table::UnlinkedAsset>,
    /// 这一镜出过的每一次（场记板上的「次」）。新的在后；`video_url` 指向最新一次。
    #[serde(default)]
    pub takes: Vec<VideoTake>,
    pub negative_prompt: Option<String>,
    pub duration: f32,
    pub image_url: Option<String>,
    pub video_url: Option<String>,
    #[serde(default)]
    pub audio_url: Option<String>,
    /// 宫格草图。**每次读盘重算，不落库** —— 它只是
    /// `shots/<id>_sketch.png` 在不在的投影，存下来会漂。
    ///
    /// 和 `image_url` 是两回事：草图是布局稿（本机 3×3 每格 512×341，
    /// 能定构图和光向，定不了材质和脸），关键帧才是成品。
    /// 界面上必须标出这个区别，否则草图会被当成品用下去。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sketch_url: Option<String>,
    /// 这一镜挂了哪几张关键帧（`shots/<id>.png` 首帧、`shots/<id>_end.png` 尾帧）。
    ///
    /// **真相在磁盘上**，这里只是投影：读盘（[`resolve_episode_media`]）、整集保存、
    /// 合成、出片之前都按文件重算一遍（[`sync_keyframes`]）。存下来是为了进提示词指纹 ——
    /// 挂上 / 摘掉关键帧会换出片模式（参考生视频 ↔ 首帧 / 首尾帧），旧提示词就不对了。
    #[serde(default, skip_serializing_if = "Keyframes::is_none")]
    pub keyframes: Keyframes,
    /// 尾帧图（`shots/<id>_end.png`）。**每次读盘重算**，和 `sketch_url` 一样只是投影。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_image_url: Option<String>,
    /// 这一镜是否**接着上一镜继续**：关键帧直接用上一镜视频的最后一帧。
    ///
    /// # ⚠️ 已被 [`Continuity::FirstFrame`] 取代，这里只为读老数据保留
    ///
    /// 2026-09-20 查实：现在的分镜生成器**一次都不写它**，真实数据里只有
    /// 一集的一镜打开过。而它做的事和 `Continuity::FirstFrame` 是同一件。
    ///
    /// **不要读这个字段** —— 读 [`StoryboardShot::effective_continuity`]，
    /// 它会把老数据折进来。两个字段各读各的就是两份真相，
    /// 而这个项目今天刚为同一类问题修了一整天。
    #[serde(default)]
    pub continues_previous: bool,
    pub status: String, // "pending" | "generating" | "ready" | "error"
}


/// 开场时不在画面里的两种人，见 [`StoryboardShot::absent_from_opening`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpeningAbsence {
    /// 整镜只出声（画外音），从头到尾看不见
    VoiceOnly,
    /// 镜头中途才进画面
    EntersLater,
}

/// 场景站位（`stage.people`）里说「这个人开场不在画面里」的说法。
const OPENING_ABSENT_WORDS: &[&str] = &[
    "尚未出现", "还没出现", "未出现", "尚未出场", "还没出场", "未出场", "尚未入画", "未入画", "不入画", "不出画", "画外", "画面外", "不在画面",
];

impl StoryboardShot {
    /// 给提示词的运镜文字。兼容旧数据，但永远剥掉历史 `[导演机位] JSON`。
    pub fn camera_prompt_note(&self) -> String {
        let mut lines: Vec<&str> = self.camera_note.lines()
            .filter(|line| !line.trim_start().starts_with("[导演机位:"))
            .collect();
        if let Some(binding) = &self.director_camera {
            if !binding.motion_note.trim().is_empty() { lines.push(binding.motion_note.trim()); }
        }
        lines.join("\n").trim().to_string()
    }
    /// 这一镜出场的角色 id —— 新字段并上废弃的单数字段。
    ///
    /// 旧数据只有 `character_id`，直接读 `character_ids` 会得到空列表，
    /// 于是引用扫描认为「没人用这个角色」，删除保护静默失效。
    pub fn all_character_ids(&self) -> Vec<String> {
        let mut out = self.character_ids.clone();
        if !self.character_id.is_empty() && !out.contains(&self.character_id) {
            out.push(self.character_id.clone());
        }
        out
    }

    /// **这个人开场时不在画面里**（首帧里没有他）：只出声的，或者镜头中途才进来的。在画面里返回 `None`。
    ///
    /// # 为什么（2026-10-05 EP02 第 11 镜）
    ///
    /// 首帧出片时，人物表里的每一个人原来都被当成「首帧里看得见的人」，长相只认首帧。
    /// 第 11 镜陆川 3.5 秒才从屋里冲出来，首帧里根本没有他；写手只能编一句
    /// 「<Subject 2> 是首帧里南边门口那个男人」，H3 就在首帧里找了个男人 ——
    /// 出来的是穿背心的邻居把星澜推进了屋，门也开错了一边。
    ///
    /// # 怎么判（宁可少判）
    ///
    /// 开场那一段（第一段的画面 / 站位，没分段就看分镜描述）或站位表写到了他 → 在画面里。没写到的：
    /// - 他的台词全是画外 → 只出声；
    /// - 后面的段写到了他，而且场景站位里明说他开场不在（「尚未出现」「不出画」……）→ 中途进场。
    ///
    /// 其余都当在画面里（原来的行为）：群演常常整镜不点名，开场没写到不等于不在。
    pub fn absent_from_opening(&self, name: &str) -> Option<OpeningAbsence> {
        // 资产名可能带形象后缀（「林婉清（盛唐）」），镜头表里写的是本名
        let name = base_character_name(name);
        if name.is_empty() {
            return None;
        }
        let same = |who: &str| base_character_name(who) == name;
        let opening = match self.segments.first() {
            Some(s) => format!("{}\n{}", s.visual, s.blocking),
            None => self.prompt.clone(),
        };
        if opening.contains(name) || self.blocking.iter().any(|b| same(&b.actor)) {
            return None;
        }
        let mut lines = self.dialogue_lines.iter().filter(|l| same(&l.speaker) && !l.line.trim().is_empty()).peekable();
        if lines.peek().is_some() && lines.all(|l| l.off_screen) {
            return Some(OpeningAbsence::VoiceOnly);
        }
        let later = self.segments.iter().skip(1).any(|s| s.visual.contains(name) || s.blocking.contains(name));
        let said_absent = self.stage.people.iter()
            .find(|p| p.trim_start().starts_with(name))
            .is_some_and(|p| OPENING_ABSENT_WORDS.iter().any(|w| p.contains(w)));
        (later && said_absent).then_some(OpeningAbsence::EntersLater)
    }

    /// 这一镜是否引用了某个资产（任意类型）。
    pub fn references_asset(&self, asset_id: &str) -> bool {
        self.all_character_ids().iter().any(|c| c == asset_id)
            || self.scene_id.as_deref() == Some(asset_id)
            || self.prop_ids.iter().any(|p| p == asset_id)
    }
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpisodeData {
    pub id: String,
    /// **这一集用的画风。空 = 跟项目走。**
    ///
    /// # 为什么要能固定到分集
    ///
    /// 风格是项目级可变状态。实测同一个项目里
    /// E01 国漫 CG / E02 真人写实 / E03 仙侠 CG / E04 水墨插画，
    /// 而当前 `art_direction` 是水墨 —— **重合成 E01 会按水墨出，
    /// 复现不了它自己的历史风格**（2026-09-19 Codex Astra 指出）。
    ///
    /// 空串保留「跟随项目」这个显式选择：大多数项目一集一集往下做，
    /// 改风格就是想让后面都跟着变。只有想锁住某一集时才填。
    #[serde(default)]
    pub style_lock: String,
    /// **哪几场整段出**（长镜头），按场号。
    ///
    /// # 为什么按场号存，不存在镜头上
    ///
    /// 一场里的镜头必须整体一致 —— 放镜头上就得维护「同一场的这个标记要
    /// 一样」这条不变式，而这种不变式一定会被破坏（用户插入一镜、复制一镜、
    /// 改场号）。存在集上只有一份真相。
    ///
    /// 硬约束和代价见 [`crate::long_take`]：窗口必须等长、不能逐窗重做。
    #[serde(default)]
    pub long_take_scenes: Vec<u32>,
    /// 长镜头窗口之间重叠多少帧。空 = 用推荐值 22。
    ///
    /// 只有 5 / 22 / 39 / 56 四档（H3 的 17n+5 潜空间栅格）。
    /// **短窗口配大重叠是在白烧算力** —— 3 秒窗口配 22 帧，
    /// 31% 的采样发布时会被裁掉。见 [`crate::long_take::context_frames_hint`]。
    #[serde(default)]
    pub long_take_context_frames: Option<u32>,
    /// **整段出的场出过的片**（长镜头），新的在后。**整条记在场上，不切开** ——
    /// H3 本来就是分窗口生成、拼好了交出来的，再按接缝切回每镜一段是白折腾（用户 2026-09-30）。
    /// 只由后端写（出片完成时），界面保存时沿用盘上的，见 `cinema_save_episode`
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scene_renders: Vec<SceneRender>,
    /// **这一集默认不分镜**（每场一镜出，见 [`crate::one_take`]）。给短片用 ——
    /// 15 秒的一场被拆成 4 / 2 / 3 / 4 秒，2 秒那镜 H3 根本出不了（2026-09-23 用户截图）。
    #[serde(default)]
    pub one_take: bool,
    /// 单场的设置，优先于 `one_take`：某一场单独一镜出 / 改回分镜、手填总时长。
    #[serde(default)]
    pub scene_takes: Vec<crate::one_take::SceneTake>,
    pub episode_num: u32,
    pub title: String,
    pub synopsis: String,
    /// 剧本的**纯文本投影**。
    ///
    /// **下游全部读它** —— 提取实体、生成分镜、前情提要都走这一份。
    /// 编辑器保存时和 `script_doc` 一起写。
    pub script_text: String,
    /// 剧本编辑器的结构化文档（ProseMirror JSON，序列化成字符串）。
    ///
    /// **可以为空**：老数据、或者从别处直接写进 `script_text` 的内容。
    /// 为空时编辑器用粘贴识别那套把纯文本解析成结构。
    ///
    /// **不能反过来只存它** —— 那样每个下游都要先渲染成文本才能用，
    /// 而且所有老数据都读不出来。
    #[serde(default)]
    pub script_doc: Option<String>,
    /// 这一集打算做多长（秒）。`0` = 没定，分镜不受约束。
    ///
    /// # 没有它分镜就是散的
    ///
    /// 改编方案那一步已经定了「这一集约 150 秒」，而分镜这一步**完全不知道** ——
    /// 它按剧本长度自由发挥，出来可能是 8 个镜头也可能是 60 个。
    /// 一集的长度是导演决定，不是模型即兴。
    ///
    /// 只当**预算**用，不卡死：镜头时长 3~6 秒，模型按这个估个数，
    /// 多一两镜少一两镜都正常。
    #[serde(default)]
    pub target_secs: u32,
    pub storyboard: Vec<StoryboardShot>,
    /// 分镜数据是哪一版结构生成的。不等于 [`STORYBOARD_VERSION`] 的一律当没有分镜。
    ///
    /// 2026-09-13 分镜重构：旧分镜靠出片时层层拼提示词，结构整个换掉，
    /// 用户原话「直接丢吧，不要了」—— 所以不迁移，读盘时清空。
    #[serde(default)]
    pub storyboard_version: u32,
    pub final_video_url: Option<String>,
    /// 本集颜色表：每个出场角色一个唯一颜色。**颜色 = 身份** ——
    /// 导演世界的人偶颜色、以后出图时认人都查它。见 [`crate::actor_colors`]。
    #[serde(default)]
    pub actor_colors: Vec<crate::actor_colors::ActorColor>,
}

/// 当前分镜结构的版本号。见 [`EpisodeData::storyboard_version`]。
pub const STORYBOARD_VERSION: u32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CinemaProjectDetail {
    pub project: CinemaProject,
    pub episodes: Vec<EpisodeData>,
}

#[derive(Debug, Deserialize)]
pub struct CreateProjectReq {
    pub title: String,
    #[serde(default)]
    pub genre: String,
    pub aspect_ratio: Option<String>,
    #[serde(default)]
    pub description: String,
    /// `"series"` | `"standalone"`，不给按系列。
    #[serde(default)]
    pub kind: Option<String>,
    /// `"scripted"` | `"freeform"`，不给按有剧本。
    #[serde(default)]
    pub content_mode: Option<String>,
    /// 新建镜头的默认模式，不给按首帧优先。
    #[serde(default)]
    pub default_shot_mode: Option<ShotMode>,
}

#[derive(Debug, Deserialize)]
pub struct CreateEpisodeReq {
    pub project_id: String,
    pub title: Option<String>,
    pub synopsis: Option<String>,
    pub script_text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GenerateAiSequelReq {
    pub project_id: String,
    pub prev_episode_id: String,
    pub direction: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AiSequelResult {
    pub title: String,
    pub synopsis: String,
    pub script: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterAngle {
    pub angle_name: String, // "0° 正面特写", "左侧 45°", "右侧 45°", "90° 正侧剪影", "电影仰角"
    pub image_url: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterViewItem {
    pub key: String,
    pub title: String,
    pub image_url: String,
    pub description: String,
    pub category: String, // "turnaround" | "expression" | "pose" | "angle"
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CharacterAssetSheet {
    pub turnarounds: Vec<CharacterViewItem>, // 正面全身、侧面全身、背面全身
    pub expressions: Vec<CharacterViewItem>, // 微笑、哭泣、震惊、愤怒、委屈、冷酷
    pub poses: Vec<CharacterViewItem>,       // 站姿、奔跑、跃起、崩溃
    pub angles: Vec<CharacterAngle>,         // 5 维面部机位
    /// 这一次生成里出的问题。**必须回到界面上。**
    ///
    /// 早先它们只进 `tracing::warn!` —— 于是「表情那一档六张全被 QC 拒掉」
    /// 表现成界面上一个空的「表情模组 (0/6)」，用户不知道是没做、
    /// 做失败了、还是做完了没显示。整档失败是这套流程里最贵的一种失败，
    /// 它绝对不能是安静的。
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CinemaCharacter {
    pub id: String,
    pub name: String,
    pub role: String,       // "主角 (真千金)", "男主 (商界巨擘)", "反派 (虚伪养女)"
    pub avatar_url: String, // path to portrait / face reference
    pub trigger_words: String, // "1girl, long black hair, royal evening gown, sapphire necklace"
    pub lora_model: Option<String>,
    pub lora_weight: f32,
    pub consistency_mode: String, // "pulid" | "ip_adapter_faceid" | "instant_id" | "lora" | "reference_only"
    pub clothing_prompt: Option<String>,
    #[serde(default)]
    pub multi_angles: Vec<CharacterAngle>,
    #[serde(default)]
    pub asset_sheet: Option<CharacterAssetSheet>,
    /// 8 维角色视觉基因锚点 (Character Visual DNA)
    #[serde(default)]
    pub visual_dna: Option<crate::cinema_dna::CharacterVisualDna>,
    /// 这个角色住在哪一层。`None` = 项目级（跨集共用），
    /// `Some(ep)` = 本集级（刚提取出来、还没决定归属）。
    ///
    /// **保存时必须按它回写原来那一层。** 一律写项目级的话，
    /// 编辑一个本集角色会在项目池里凭空复制一份 —— 而且不报错，
    /// 表现是「资产库里怎么多了个一样的人」。
    #[serde(default)]
    pub episode_id: Option<String>,
    /// 这个角色的固定种子。
    ///
    /// **首次生成时记下来，之后默认复用。** 不固定的话每次点生成都是一次
    /// 新的掷骰子 —— 「上次那张好」找不回来，而且改了提示词也分不清
    /// 差异是提示词造成的还是运气。
    ///
    /// 注意它管的是**可复现**，不是角色一致性 ——
    /// 跨镜头还是同一个人靠的是参考图（IPAdapter/PuLID），不是种子。
    #[serde(default)]
    pub seed: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CinemaEngineConfig {
    pub llm_provider: String, // "ollama" | "lmstudio" | "custom"
    pub llm_endpoint: String, // "http://127.0.0.1:11434"
    pub llm_model: String,    // "qwen2.5:7b", "deepseek-r1:14b", "llama3.2"
    pub checkpoint: String,   // "AnimagineXL_V3.1.safetensors", "RealVisXL_V5.0_fp16.safetensors"
    pub lora: Option<String>,
    pub lora_weight: f32,           // 0.8
    pub consistency_engine: String, // "pulid" | "ip_adapter_faceid" | "instant_id" | "reference_only" | "lora"
    pub sampler: String,            // "euler", "dpmpp_2m_karras"
    pub steps: u32,                 // 25
    pub cfg: f32,                   // 7.0
    pub video_engine: String, // "ltx" | "minimax_h3" | "liveportrait" | "camera_preview"；"wan2.1" 已下架，"hunyuan_i2v" | "svd" | "animatediff" 只是占位
    #[serde(default = "default_video_workflow_id")]
    pub video_workflow_id: String,
    pub custom_workflow_json: Option<String>, // imported Civitai / C-station workflow template
    /// 把台词烧进画面。**默认开** —— 模型自己画的中文字幕认不出来（实测），
    /// 而台词是我们自己写的，后期渲染是像素级正确的。
    #[serde(default = "default_burn_subtitles")]
    pub burn_subtitles: bool,
    /// 让模型生成音轨。**默认开，而且这个默认值是有代价的** ——
    /// H3 的编码器是 Qwen3-VL，提示词里的台词会变成**同步中文语音**，
    /// 那是这条链路的招牌能力。关掉它等于主动放弃配音。
    ///
    /// 只有用户显式关掉时才剥音轨；缺省（老的 `engine_config` 里没这个键）
    /// 一律当开。**不要改成 `false` 兜底** —— 见 [`ShotAudio`] 的注释，
    /// 「无条件静音」这个 bug 在 2026-09-04 和 2026-09-10 各复发过一次。
    #[serde(default = "default_generate_audio")]
    pub generate_audio: bool,
}

/// **默认不烧硬字幕。** 烧进画面的改不了（字体、时间、错别字都要重出片）；
/// 合成一集时另出 `final.srt`，播放器和剪辑软件都能用。要硬字幕在生成设置里开
fn default_burn_subtitles() -> bool {
    false
}

fn default_generate_audio() -> bool {
    true
}

/// 没有任何代码读这个字段（只是存着）。原来的默认值是 Wan 的工作流 id，
/// Wan 2026-10-01 下架后留空，免得看起来像还有一条 Wan 路。
fn default_video_workflow_id() -> String {
    String::new()
}

impl Default for CinemaEngineConfig {
    fn default() -> Self {
        Self {
            llm_provider: "ollama".to_string(),
            llm_endpoint: "http://127.0.0.1:11434".to_string(),
            llm_model: "qwen2.5:7b".to_string(),
            checkpoint: "AWPortrait_XL_ver1.1.safetensors".to_string(),
            lora: None,
            lora_weight: 0.8,
            consistency_engine: "pulid".to_string(),
            sampler: "euler".to_string(),
            steps: 25,
            cfg: 7.0,
            video_engine: "liveportrait".to_string(),
            video_workflow_id: default_video_workflow_id(),
            custom_workflow_json: None,
            burn_subtitles: true,
            generate_audio: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RenderShotReq {
    pub project_id: String,
    pub episode_id: String,
    pub shot_id: String,
    pub engine_config: Option<CinemaEngineConfig>,
}

#[derive(Debug, Serialize)]
pub struct RenderShotResult {
    pub shot_id: String,
    pub image_url: String,
    pub video_url: String,
    pub audio_url: String,
    pub status: String,
}

/// 给一个项目挑张封面。
///
/// # 为什么是**读盘时算**，不是存下来
///
/// `cover_image` 这个字段一直存在、界面也一直在读，但**全仓库没有任何一处
/// 给它赋过值** —— 于是每个项目卡都是一块纯色渐变。
/// 「机制写了、语义留了、执行漏了」，和 `tool_search` 那次一个形状。
///
/// 补的时候选了读盘算而不是存字段：存下来的会漂（重出了首帧、删了镜头，
/// 封面还指着不存在的文件），而漂了之后表现是「有的项目有封面、有的没有」，
/// 看不出是哪一步没跟上。`hydrate_episode` 那边是同样的理由。
///
/// # 挑哪一张
///
/// **第一集第一个有首帧的镜头** —— 那是这个故事的开场画面，
/// 比任意一张资产图更能代表「这个项目是什么」。找不到就退回资产封面。
fn pick_project_cover(proj_dir: &std::path::Path) -> Option<String> {
    let eps_dir = proj_dir.join("episodes");
    let mut eps: Vec<_> = std::fs::read_dir(&eps_dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    // 按目录名排序 —— `ep_01` 在 `ep_02` 前面
    eps.sort();
    for ep_dir in eps {
        let Ok(txt) = std::fs::read_to_string(ep_dir.join("episode.json")) else {
            continue;
        };
        let Ok(mut ep) = serde_json::from_str::<EpisodeData>(&txt) else {
            continue;
        };
        // **走同一条归一化** —— 否则拿到的是没解析过的相对路径，
        // 界面上又是一张读不出来的图（组装页那个成片刚踩过）。
        hydrate_episode(&ep_dir, &mut ep);
        if let Some(url) = ep
            .storyboard
            .iter()
            .find_map(|sh| sh.image_url.clone().filter(|u| !u.is_empty()))
        {
            return Some(url);
        }
    }
    None
}

/// 设项目里某个角色的默认形象。`main` = 主形象（从表里去掉）。
pub fn set_default_look(config_dir: &Path, project_id: &str, asset_id: &str, look_id: &str) -> Result<()> {
    let file = cinema_projects_dir(config_dir).join(project_id).join("project.json");
    let mut p: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file).map_err(|e| Error::config(format!("读不到项目：{e}")))?)
        .map_err(|e| Error::config(format!("项目文件坏了：{e}")))?;
    let mut map: std::collections::BTreeMap<String, String> = p.get("default_looks").cloned().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
    if look_id == crate::assets::MAIN_LOOK || look_id.is_empty() {
        map.remove(asset_id);
    } else {
        map.insert(asset_id.to_string(), look_id.to_string());
    }
    p["default_looks"] = serde_json::to_value(&map).unwrap_or_default();
    std::fs::write(&file, serde_json::to_string_pretty(&p).unwrap_or_default()).map_err(|e| Error::runtime(format!("写不回项目：{e}")))
}

#[tauri::command]
pub fn cinema_set_default_look(state: State<'_, AppState>, project_id: String, asset_id: String, look_id: String) -> Result<()> {
    set_default_look(&state.config_dir, &project_id, &asset_id, &look_id)
}

/// 列出所有影视短剧创作项目
#[tauri::command]
pub async fn cinema_list_projects(state: State<'_, AppState>) -> Result<Vec<CinemaProject>> {
    let root = cinema_projects_dir(&state.config_dir);
    if !root.is_dir() {
        let _ = std::fs::create_dir_all(&root);
        // 如果为空，自动初始化一个示例爆款短剧项目
        let sample = create_sample_project(&root);
        if let Ok(p) = sample {
            return Ok(vec![p]);
        }
        return Ok(Vec::new());
    }

    let mut projects = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            let p_dir = entry.path();
            if p_dir.is_dir() {
                let meta_file = p_dir.join("project.json");
                if meta_file.is_file() {
                    if let Ok(content) = std::fs::read_to_string(&meta_file) {
                        if let Ok(mut proj) = serde_json::from_str::<CinemaProject>(&content) {
                            // 自己没设封面就挑一张。**用户设过的不动。**
                            if proj.cover_image.as_deref().unwrap_or("").trim().is_empty() {
                                proj.cover_image = pick_project_cover(&p_dir);
                            }
                            projects.push(proj);
                        }
                    }
                }
            }
        }
    }

    projects.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    if projects.is_empty() {
        if let Ok(p) = create_sample_project(&root) {
            projects.push(p);
        }
    }
    Ok(projects)
}

/// 创建全新影视短剧项目
#[tauri::command]
pub async fn cinema_create_project(
    state: State<'_, AppState>,
    req: CreateProjectReq,
) -> Result<CinemaProject> {
    // 和 Chat 的建项目工具共用一份（studio_ops）
    crate::studio_ops::create_project(&state.config_dir, req)
}

/// 把镜头的媒体字段统一成**本机绝对路径**。
///
/// # 这个函数以前是反着来的
///
/// 它原来叫 `hydrate_episode_media`，做的是把老的 `/projects/...` 相对路径
/// **读成 base64 塞回字段里**。于是 `episode.json` 变成了一个装图和视频的
/// 容器：实测单集 1.82 MB，其中 **99.5% 是 base64**，而那还只是
/// 1 张首帧、0 段视频的时候。
///
/// 往下推：一张首帧 PNG 1.36 MB → base64 约 1.81 MB；10 镜全出首帧 ≈ 18 MB，
/// 再加 10 段 3 秒视频 ≈ 再 7~27 MB。而 `cinema_get_project_detail` 会把
/// **整个项目的所有分集**装进一条 Tauri IPC 消息，`shotPersistence` 又在
/// 每次失焦保存时整份读写一遍。
///
/// 这个仓库为同一件事付过一次代价 —— 86 MB 的 `characters.json`，
/// 读一次角色列表要解析整个文件（见 `MediaThumb` 的文件头）。
///
/// LumenX 存的是相对路径，它的 `projects.json` 0.05 MB、内嵌 base64 零处。
///
/// # 文件没了的时候不扔 base64
///
/// 老数据里可能有「字段是 base64、磁盘上那个文件已经不在」的镜头。
/// 那段 base64 是这份像素**唯一的副本**，扔掉就是真的丢图。
fn resolve_episode_media(ep_dir: &std::path::Path, ep: &mut EpisodeData) {
    let shots_dir = ep_dir.join("shots");
    let audio_dir = ep_dir.join("audio");
    for shot in &mut ep.storyboard {
        let img = shots_dir.join(format!("{}.png", shot.id));
        let vid = shots_dir.join(format!("{}.mp4", shot.id));
        shot.image_url = resolve_media_field(shot.image_url.take(), &img);
        shot.video_url = resolve_media_field(shot.video_url.take(), &vid);
        shot.audio_url = resolve_media_field(
            shot.audio_url.take(),
            &audio_dir.join(format!("{}.wav", shot.id)),
        );
        // **每次读盘重算**，不信任存下来的值 —— 存下来的会漂，
        // 而漂了之后表现是"有的镜头刷新、有的不刷新"，极难查。
        shot.image_rev = media_rev(&img);
        shot.video_rev = media_rev(&vid);
        shot.sketch_url = resolve_media_field(None, &sketch_path(&shots_dir, &shot.id));
        shot.keyframes = Keyframes::on_disk(&shots_dir, &shot.id);
        shot.end_image_url = resolve_media_field(None, &end_frame_path(&shots_dir, &shot.id));
    }
    // **成片也要过这一道。**
    //
    // 漏了三个月都没人发现，因为它不报错：`cinema_stitch_episode` 存的是
    // `/projects/<id>/episodes/<id>/final.mp4` 这种**相对路径**，
    // 而界面拿它当媒体 URL 用 —— 组装页那格就一直显示「读不出来」，
    // 看着像文件坏了，实际上文件好好地在盘上。
    //
    // 镜头没这个问题只是因为它们碰巧走了上面那个循环。
    ep.final_video_url = resolve_media_field(ep.final_video_url.take(), &ep_dir.join("final.mp4"));
}

/// **读盘之后对这一集做的全部归一化。**
///
/// 只有一个入口是故意的：以前是媒体路径一件事、镜头语言一件事，
/// 各自在调用点排队。那种写法漏一处的表现是「有的分集是对的、
/// 有的不对」—— 而两者读的是同一批文件，从现象上分不出漏了哪一步。
///
/// 加新的「读盘时要做的事」请加进这里，不要在调用点再排一行。
fn hydrate_episode(ep_dir: &std::path::Path, ep: &mut EpisodeData) {
    drop_old_storyboard(ep);
    resolve_episode_media(ep_dir, ep);
}

/// 旧结构的分镜不再读。只清内存里这一份，磁盘上的镜头文件不动。
fn drop_old_storyboard(ep: &mut EpisodeData) {
    if ep.storyboard_version != STORYBOARD_VERSION {
        ep.storyboard.clear();
        ep.storyboard_version = STORYBOARD_VERSION;
    }
}

/// 这个文件当前这一版：mtime 的毫秒数。文件不在就是 0。
///
/// 只用来给界面做缓存击穿，不参与任何判断 —— 所以取不到时间时
/// 回 0 就行（那种情况下文件多半也不在）。
fn media_rev(path: &std::path::Path) -> u64 {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 一个媒体字段该变成什么。
///
/// 优先级：**磁盘上有约定文件就用它的绝对路径** > 内联数据（唯一副本，留着）
/// > 已经是有效路径 > 都不是就置空。
///
/// **置空不是丢数据，是止损。** 老的 `/projects/xxx.png` 指不到任何东西，
/// 留着只会让界面显示一张碎图，而碎图看起来像「文件坏了」。
pub fn resolve_media_field(current: Option<String>, on_disk: &std::path::Path) -> Option<String> {
    let current = current.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    if on_disk.is_file() {
        return Some(on_disk.to_string_lossy().to_string());
    }
    let current = current?;
    if current.starts_with("data:") {
        return Some(current);
    }
    std::path::Path::new(&current)
        .is_file()
        .then_some(current)
}

/// 在系统文件管理器里**定位到这个文件**（选中它，不是打开所在目录）。
///
/// # 为什么要有这个
///
/// 组装那一步的产出是 `final.mp4`，而界面上只有一个 380px 的预览 ——
/// 「组装 / 导出」这一步却导不出去。真正要做的事（发给别人、拖进剪辑软件、
/// 存到别处）全都得先在访达里找到它。
///
/// # 为什么不是打开目录
///
/// `open <目录>` 之后用户还得在一堆 `shot_*.mp4` 里认哪个是成片。
/// macOS 的 `open -R` / Windows 的 `explorer /select,` 会**直接选中**那个文件。
/// Linux 没有通用的等价物，退回打开父目录。
#[tauri::command]
pub fn cinema_reveal_file(path: String) -> Result<()> {
    let p = std::path::Path::new(&path);
    if !p.is_file() {
        return Err(Error::config(format!("文件不在了：{path}")));
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg("-R").arg(p).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer")
            .arg(format!("/select,{}", p.display()))
            .spawn();
    }
    #[cfg(target_os = "linux")]
    {
        // 没有通用的「选中文件」，退回打开父目录
        if let Some(dir) = p.parent() {
            let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
        }
    }
    Ok(())
}

/// 获取影视项目详情与各分集列表
#[tauri::command]
pub async fn cinema_get_project_detail(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<CinemaProjectDetail> {
    let root = cinema_projects_dir(&state.config_dir);
    let proj_dir = root.join(&project_id);
    if !proj_dir.is_dir() {
        return Err(Error::config(format!("找不到项目「{project_id}」")));
    }

    let meta_file = proj_dir.join("project.json");
    let proj: CinemaProject = if meta_file.is_file() {
        let content = std::fs::read_to_string(&meta_file).unwrap_or_default();
        serde_json::from_str(&content).map_err(|e| Error::config(format!("解析项目失败：{e}")))?
    } else {
        return Err(Error::config("缺少 project.json 元文件"));
    };

    let mut episodes = Vec::new();
    let ep_dir = proj_dir.join("episodes");
    if let Ok(entries) = std::fs::read_dir(ep_dir) {
        for entry in entries.flatten() {
            let ep_json = entry.path().join("episode.json");
            if ep_json.is_file() {
                if let Ok(content) = std::fs::read_to_string(&ep_json) {
                    if let Ok(mut ep) = serde_json::from_str::<EpisodeData>(&content) {
                        hydrate_episode(&entry.path(), &mut ep);
                        episodes.push(ep);
                    }
                }
            }
        }
    }
    episodes.sort_by_key(|e| e.episode_num);

    Ok(CinemaProjectDetail {
        project: proj,
        episodes,
    })
}

/// 给项目加一集。**建出来是空的**（或带上调用方给的剧本），不塞样板。
///
/// 以前这里会塞一集示例剧本和三个示例镜头（都市商战、由美子和顾辰），
/// 用户在自己的系列里点「加集」就多出一集不相干的内容，又没有删分集的入口（2026-09-16）。
/// 现在和 Chat 的加集工具共用 `studio_ops::create_episode`。
#[tauri::command]
pub async fn cinema_create_episode(
    state: State<'_, AppState>,
    req: CreateEpisodeReq,
) -> Result<EpisodeData> {
    let mut ep = crate::studio_ops::create_episode(
        &state.config_dir,
        &req.project_id,
        req.title.as_deref().unwrap_or(""),
        req.synopsis.as_deref().unwrap_or(""),
    )?;
    if let Some(text) = req.script_text.filter(|t| !t.trim().is_empty()) {
        crate::studio_ops::import_script(&state.config_dir, &req.project_id, &ep.id, &text, false)?;
        ep.script_text = text;
    }
    Ok(ep)
}

/// 删掉系列里的一集。两步确认，规则见 `studio_ops::delete_episode`。
#[tauri::command]
pub fn cinema_delete_episode(
    state: State<'_, AppState>,
    project_id: String,
    episode_id: String,
    confirm: bool,
) -> Result<String> {
    crate::studio_ops::delete_episode(&state.config_dir, &project_id, &episode_id, confirm)
}

/// 承接上一集结尾悬念 (Cliffhanger) 自动续写下一集剧本
#[tauri::command]
pub async fn cinema_generate_ai_sequel(
    state: State<'_, AppState>,
    req: GenerateAiSequelReq,
) -> Result<AiSequelResult> {
    cinema_generate_ai_sequel_with_config_dir(&state.config_dir, req).await
}

pub async fn cinema_generate_ai_sequel_with_config_dir(
    config_dir: &Path,
    req: GenerateAiSequelReq,
) -> Result<AiSequelResult> {
    let root = cinema_projects_dir(config_dir);
    let ep_file = root
        .join(&req.project_id)
        .join("episodes")
        .join(&req.prev_episode_id)
        .join("episode.json");

    let mut _prev_script = String::new();
    let mut prev_num = 1;
    if ep_file.is_file() {
        if let Ok(content) = std::fs::read_to_string(&ep_file) {
            if let Ok(ep) = serde_json::from_str::<EpisodeData>(&content) {
                _prev_script = ep.script_text;
                prev_num = ep.episode_num;
            }
        }
    }

    let next_num = prev_num + 1;
    let direction = req.direction.as_deref().unwrap_or("reverse");

    let (title, synopsis, script) = match direction {
        "cliffhanger" => (
            format!("第 {} 集：暗夜迷局", next_num),
            "上集高潮处留下的惊天线索被逐一揭开，幕后黑手竟早已潜伏在主角身边！".to_string(),
            "【场景：废弃钟楼顶层】\n【时间：暴雨深夜】\n【人物：由美子、顾辰、神秘人】\n\n[镜头 1: 暴雨大远景] 电闪雷鸣，古老钟楼在雨幕中若隐若现。\n\n[镜头 2: 主角特写] 由美子浑身湿透，眼神中满是难以置信。\n由美子：“原来从一开始，你给我的名单就是假的？！”\n\n[镜头 3: 神秘人剪影] 阴影中的人缓缓扔出一枚银色徽章，冷笑不语。\n神秘人：“你以为你赢了？好戏才刚刚开始。”\n\n[镜头 4: 顾辰侧身拉镜头] 顾辰挡在由美子身前，手扣扳机。\n顾辰：“动她一下，整个江城为你陪葬！”\n".to_string()
        ),
        "sweet" => (
            format!("第 {} 集：星光之誓", next_num),
            "经历前集重重危机后，男女主心意相通，霸气公开定情，全网热议！".to_string(),
            "【场景：外滩顶奢旋转餐厅】\n【时间：星光夜景】\n【人物：由美子、顾辰】\n\n[镜头 1: 城市全景] 东方明珠璀璨倒映，浪漫乐声流淌。\n\n[镜头 2: 男主特写推镜头] 顾辰单膝落地，掌心捧着专属定制黑钻王冠。\n顾辰：“这场仗你已经打赢了，现在，该轮到我来守护你。”\n\n[镜头 3: 女主含泪仰角特写] 由美子眼眶微红，唇边泛起动人微笑。\n由美子：“顾辰，我答应你。”\n\n[镜头 4: 两人拥抱中景] 烟花在窗外夜空绚烂绽放。\n".to_string()
        ),
        _ => (
            format!("第 {} 集：全场跪拜", next_num),
            "主角直接亮出最终底牌，反派全线崩盘，千亿继承人身份震撼全城！".to_string(),
            "【场景：全球金融峰会主会场】\n【时间：正午日光】\n【人物：由美子、顾辰、全体董事】\n\n[镜头 1: 会场大俯拍] 数百位商界巨头肃立，闪光灯疯狂闪烁。\n\n[镜头 2: 女主特写] 由美子走上主发言台，将一份股权转让协议狠狠拍在桌上。\n由美子：“从今天起，顾氏集团最大表决权归我所有，谁赞成？谁反对？！”\n\n[镜头 3: 反派脸色惨白中景] 众人面面相觑，颤抖着纷纷低头鞠躬。\n全体董事：“参见董事长！”\n\n[镜头 4: 顾辰欣慰微笑特写] 顾辰在台下优雅举杯。\n顾辰：“恭喜你，我的女王。”\n".to_string()
        ),
    };

    Ok(AiSequelResult {
        title,
        synopsis,
        script,
    })
}

pub(crate) fn episode_file(config_dir: &Path, project_id: &str, episode_id: &str) -> PathBuf {
    cinema_projects_dir(config_dir).join(project_id).join("episodes").join(episode_id).join("episode.json")
}

/// 本集按首次出场顺序排好的有身体的角色 `(asset_id, name)`。
///
/// 读的是分镜里已经解析好的 `character_ids`。只有声音的角色（`disembodied`）不算 ——
/// 它不会出现在画面里，给它占一个颜色只会让调色盘更早用完。
pub fn episode_cast(config_dir: &Path, project_id: &str, episode: &EpisodeData) -> Vec<(String, String)> {
    let scope = crate::assets::RefScope::Project { project_id, episode_id: Some(&episode.id) };
    let mut by_id: std::collections::HashMap<String, crate::assets::Asset> = std::collections::HashMap::new();
    for sc in crate::assets::lookup_scopes(config_dir, &scope) {
        for a in crate::assets::load_store(config_dir, &sc).assets {
            by_id.entry(a.id.clone()).or_insert(a);
        }
    }
    let mut out: Vec<(String, String)> = Vec::new();
    for shot in &episode.storyboard {
        for id in shot.all_character_ids() {
            if out.iter().any(|(x, _)| *x == id) {
                continue;
            }
            let Some(a) = by_id.get(&id) else { continue };
            if a.kind != crate::assets::AssetKind::Character {
                continue;
            }
            if a.character.as_ref().is_some_and(|c| c.disembodied) {
                continue;
            }
            out.push((id, a.name.clone()));
        }
    }
    out
}

/// [`read_episode`] + 按磁盘补上关键帧（首帧锁画面的判断要它）。出片队列用（`render_queue`）
pub(crate) fn read_episode_synced(config_dir: &Path, project_id: &str, episode_id: &str) -> Result<EpisodeData> {
    let mut ep = read_episode(config_dir, project_id, episode_id)?;
    sync_keyframes(&episode_file(config_dir, project_id, episode_id).with_file_name("shots"), &mut ep);
    Ok(ep)
}

pub(crate) fn read_episode(config_dir: &Path, project_id: &str, episode_id: &str) -> Result<EpisodeData> {
    let f = episode_file(config_dir, project_id, episode_id);
    let txt = std::fs::read_to_string(&f).map_err(|_| Error::config("分集不存在"))?;
    serde_json::from_str(&txt).map_err(|e| Error::config(format!("分集文件读不出来：{e}")))
}

/// 给本集出场角色补齐颜色并存盘。**已有的颜色不动。**
pub fn assign_actor_colors(config_dir: &Path, project_id: &str, episode_id: &str) -> Result<crate::actor_colors::Assignment> {
    // 出场名单要读资产库，在锁外算；分配按锁里那份最新的颜色表来，已有的颜色照样不动
    let cast = episode_cast(config_dir, project_id, &read_episode(config_dir, project_id, episode_id)?);
    crate::episode_lock::update(config_dir, project_id, episode_id, |ep| {
        let a = crate::actor_colors::assign(&ep.actor_colors, &cast);
        ep.actor_colors = a.colors.clone();
        Ok(a)
    })
}

/// 要存的分集没带颜色表时，沿用盘上那份。见 `cinema_save_episode`。
fn keep_actor_colors(disk: &EpisodeData, episode: &mut EpisodeData) {
    if episode.actor_colors.is_empty() {
        episode.actor_colors = disk.actor_colors.clone();
    }
}

/// 分配本集颜色表（幂等）。返回整张表和没分到颜色的角色名。
#[tauri::command]
pub async fn cinema_assign_actor_colors(
    state: State<'_, AppState>,
    project_id: String,
    episode_id: String,
) -> Result<crate::actor_colors::Assignment> {
    assign_actor_colors(&state.config_dir, &project_id, &episode_id)
}

/// 手动改一个角色的颜色。撞色、调色盘外的颜色都拒绝。
#[tauri::command]
pub async fn cinema_set_actor_color(
    state: State<'_, AppState>,
    project_id: String,
    episode_id: String,
    asset_id: String,
    color: String,
) -> Result<Vec<crate::actor_colors::ActorColor>> {
    crate::episode_lock::update(&state.config_dir, &project_id, &episode_id, |ep| {
        ep.actor_colors = crate::actor_colors::set_color(&ep.actor_colors, &asset_id, &color).map_err(Error::config)?;
        Ok(ep.actor_colors.clone())
    })
}

/// 界面送来的分镜是不是**整批重新生成之前**的那份。
///
/// 判据是镜头 id：重新生成会把 id 全换掉，而界面上的删一镜、加一镜、改一镜
/// id 大部分还在。所以**两边都有镜头、却一个 id 都对不上**，就是界面拿着旧数据。
/// 任一边为空不算（新建分集、清空分镜都是正常操作）。
pub(crate) fn storyboard_is_stale(disk: &[StoryboardShot], incoming: &[StoryboardShot]) -> bool {
    if disk.is_empty() || incoming.is_empty() {
        return false;
    }
    let disk_ids: std::collections::HashSet<&str> = disk.iter().map(|s| s.id.as_str()).collect();
    !incoming.iter().any(|s| disk_ids.contains(s.id.as_str()))
}

/// **只存这一镜**（分镜详情用）。以前详情面板每改一处都整集保存，拿的是界面里每一镜的那一份 ——
/// 批量细化时间线 / 批量合成提示词在后台逐镜写回，界面还没刷新时改了**别的**镜头，
/// 刚写回的那几镜就被界面里的旧版本整个盖掉，而且不报错（2026-09-28 审查时查出来的）。
///
/// # 这一镜自己也要防
///
/// `base` 是详情面板的草稿**从哪一份开始改的**。后台会写的两样（时间段、提示词）做三方比较：
/// - 界面没动、盘上变了（后台写回了）→ 用盘上的；
/// - 界面动了、盘上也变了、两边不一样 → 不存，报出来（不能静默丢掉任何一边）；
/// - 其余用界面的。
///
/// 出片结果（takes / 视频 / 状态）、关键帧、导演世界的场面一律以盘上为准 —— 那些只归后端写。
#[tauri::command]
pub fn cinema_save_shot(state: State<'_, AppState>, project_id: String, episode_id: String, shot: StoryboardShot, base: StoryboardShot) -> Result<()> {
    save_shot(&state.config_dir, &project_id, &episode_id, shot, &base)
}

pub(crate) fn save_shot(config_dir: &Path, project_id: &str, episode_id: &str, shot: StoryboardShot, base: &StoryboardShot) -> Result<()> {
    let ep_file = episode_file(config_dir, project_id, episode_id);
    let shots_dir = ep_file.with_file_name("shots");
    crate::episode_lock::update_file(&ep_file, |ep| {
        let Some(i) = ep.storyboard.iter().position(|s| s.id == shot.id) else {
            return Err(Error::config("这一镜不在盘上了（分镜刚重新生成过）—— 切到别的步骤再切回来刷新，再改一次"));
        };
        let disk = &ep.storyboard[i];
        let mut next = shot;
        for (field, label) in [("segments", "时间段"), ("prompts", "提示词")] {
            let (ui, was, now) = (field_of(&next, field), field_of(base, field), field_of(disk, field));
            if ui == was && now != was {
                set_field(&mut next, disk, field);
            } else if ui != was && now != was && ui != now {
                return Err(Error::config(format!(
                    "这一镜的{label}刚在后台被改过（细化 / 合成写回了），你这次的改动没存 —— 切到别的镜头再切回来看最新的，再改一次"
                )));
            }
        }
        next.takes = disk.takes.clone();
        next.video_url = disk.video_url.clone();
        next.status = disk.status.clone();
        next.director_scene = disk.director_scene.clone();
        if next.director_camera.is_none() {
            next.director_camera = disk.director_camera.clone();
        }
        next.keyframes = Keyframes::on_disk(&shots_dir, &next.id);
        ep.storyboard[i] = next;
        // 资产引用跟着文字重算（和整集保存同一个函数；只读资产库，不碰分集文件，锁里调没问题）
        sync_shot_asset_ids(config_dir, ep, &crate::assets::RefScope::Project { project_id, episode_id: Some(episode_id) });
        Ok(())
    })
}

fn field_of(s: &StoryboardShot, field: &str) -> serde_json::Value {
    match field {
        "segments" => serde_json::to_value(&s.segments).unwrap_or_default(),
        _ => serde_json::to_value(&s.prompts).unwrap_or_default(),
    }
}

fn set_field(to: &mut StoryboardShot, from: &StoryboardShot, field: &str) {
    match field {
        "segments" => to.segments = from.segments.clone(),
        _ => to.prompts = from.prompts.clone(),
    }
}

/// 保存分集剧本与分镜表
#[tauri::command]
pub async fn cinema_save_episode(
    state: State<'_, AppState>,
    project_id: String,
    episode: EpisodeData,
) -> Result<()> {
    let root = cinema_projects_dir(&state.config_dir);
    let ep_dir = root.join(&project_id).join("episodes").join(&episode.id);
    let _ = std::fs::create_dir_all(&ep_dir);

    let mut episode = episode;
    episode.storyboard_version = STORYBOARD_VERSION;
    // 关键帧以磁盘为准 —— 界面那份可能是挂帧之前拿的
    sync_keyframes(&ep_dir.join("shots"), &mut episode);
    let episode_id = episode.id.clone();
    sync_shot_asset_ids(&state.config_dir, &mut episode, &crate::assets::RefScope::Project { project_id: &project_id, episode_id: Some(&episode_id) });

    // 下面三件事都要和磁盘上**最新**的那份比，所以和写在同一把锁里 ——
    // 锁外读、锁里写的话，读完到写之间别处写进来的（出片回写、导演世界存场面）照样被盖掉。
    crate::episode_lock::save_over(&ep_dir.join("episode.json"), |disk| {
        if let Some(disk) = disk {
            // **界面拿着旧分镜保存时不许盖掉新分镜。**
            //
            // 2026-09-26 出过事：分镜在应用外重新生成（实跑 / 后台任务写回）之后，用户在界面上
            // 改了一场的「出法」—— 那一下调的是整集保存，传的是界面里**还没刷新**的那份，
            // 新生成的 36 镜被整个换回了旧的 40 镜，而且不报错。
            if storyboard_is_stale(&disk.storyboard, &episode.storyboard) {
                return Err(Error::config(
                    "这一集的分镜刚在别处重新生成过，界面上这份是旧的 —— 没有保存，免得把新分镜盖掉。切到别的步骤再切回来刷新一下，再改一次。".to_string(),
                ));
            }
            // **颜色表只增不改。** 前端拿着一份没带颜色表的分集保存（老页面、自己拼的对象），
            // 不能把盘上的表冲掉 —— 换了颜色，已经导出的导演合成图就对不上人了。
            // 要改颜色走 `cinema_set_actor_color`。
            keep_actor_colors(&disk, &mut episode);
            // 导演世界的场面同理：只由 cinema_set_director_scene 写；机位绑定是导演世界里写的，界面那份没有就沿用盘上的
            keep_director_state(&disk, &mut episode);
            // 整段出的成片只由出片那条写，界面那份可能是出完之前拿的
            episode.scene_renders = disk.scene_renders.clone();
        }
        Ok(episode)
    })?;

    // 更新 project.json 的修改时间
    let meta_file = root.join(&project_id).join("project.json");
    if meta_file.is_file() {
        if let Ok(c) = std::fs::read_to_string(&meta_file) {
            if let Ok(mut p) = serde_json::from_str::<CinemaProject>(&c) {
                p.updated_at = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let _ = std::fs::write(
                    meta_file,
                    serde_json::to_string_pretty(&p).unwrap_or_default(),
                );
            }
        }
    }
    Ok(())
}

/// 读取影视项目专属的独立 AI 引擎/模型/LoRA 配置
#[tauri::command]
pub async fn cinema_get_engine_config(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<CinemaEngineConfig> {
    load_cinema_engine_config(&state.config_dir, &project_id)
}

pub fn load_cinema_engine_config(
    config_dir: &Path,
    project_id: &str,
) -> Result<CinemaEngineConfig> {
    let root = cinema_projects_dir(config_dir);
    let cfg_file = root.join(project_id).join("cinema_engine_config.json");
    if cfg_file.is_file() {
        if let Ok(c) = std::fs::read_to_string(&cfg_file) {
            if let Ok(cfg) = serde_json::from_str::<CinemaEngineConfig>(&c) {
                return Ok(cfg);
            }
        }
    }
    Ok(CinemaEngineConfig::default())
}

/// 保存影视项目专属的独立 AI 引擎/模型/LoRA 配置
#[tauri::command]
pub async fn cinema_save_engine_config(
    state: State<'_, AppState>,
    project_id: String,
    config: CinemaEngineConfig,
) -> Result<()> {
    let root = cinema_projects_dir(&state.config_dir);
    let proj_dir = root.join(&project_id);
    let _ = std::fs::create_dir_all(&proj_dir);
    let cfg_file = proj_dir.join("cinema_engine_config.json");
    let content = serde_json::to_string_pretty(&config)
        .map_err(|e| Error::config(format!("序列化引擎配置失败：{e}")))?;
    std::fs::write(cfg_file, content)
        .map_err(|e| Error::config(format!("写入引擎配置文件失败：{e}")))?;
    Ok(())
}

/// 工作区里的一条。**统计值是算出来的，不是存的。**
///
/// 存一个 `status` 字段就一定会和事实漂移 —— 用户删掉最后一个镜头，
/// 项目还显示「进行中」。这里每次都从磁盘上的分集数出来。
#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceItem {
    #[serde(flatten)]
    pub project: CinemaProject,
    pub episode_count: usize,
    pub shot_count: usize,
    /// 已经出过片的镜头数。
    pub rendered_count: usize,
    pub has_final: bool,
    /// `"draft"` | `"active"` | `"done"`
    pub status: &'static str,
}

/// 跨项目的剧本一览。侧栏「剧本编辑器」那一页用。
///
/// **返回结构文档本身，不在 Rust 里数场数和角色数。**
/// 数场景要认「什么是场景标题」，那套规则已经在前端
/// （`script/format.ts`、`script/derive.ts`）—— 在这儿再写一份，
/// 两边迟早对不上，而对不上时没有任何东西会报错。
///
/// 代价是把文档一起传过去。个人工具、几十集的量级，够用；
/// 真到了传不动的时候再加缓存的计数，不要现在就分叉出第二套规则。
#[derive(Debug, Clone, Serialize)]
pub struct ScriptSummary {
    pub project_id: String,
    pub project_title: String,
    pub project_kind: String,
    pub episode_id: String,
    pub episode_num: u32,
    pub episode_title: String,
    /// 纯文本字数
    pub chars: usize,
    /// 结构文档；`None` = 还没在编辑器里打开过
    pub script_doc: Option<String>,
    /// 分镜数，用来提示「这一集写完了没」
    pub shot_count: usize,
}

#[tauri::command]
pub fn cinema_scripts(state: State<'_, AppState>) -> Vec<ScriptSummary> {
    script_summaries(&state.config_dir)
}

pub fn script_summaries(config_dir: &Path) -> Vec<ScriptSummary> {
    let root = cinema_projects_dir(config_dir);
    let Ok(projects) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for p in projects.flatten() {
        let dir = p.path();
        if !dir.is_dir() {
            continue;
        }
        let project_id = p.file_name().to_string_lossy().to_string();
        let meta: Option<CinemaProject> = std::fs::read_to_string(dir.join("project.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok());
        let project_title = meta
            .as_ref()
            .map(|m| m.title.clone())
            .unwrap_or_else(|| project_id.clone());
        let project_kind = meta
            .as_ref()
            .map(|m| m.kind.clone())
            .unwrap_or_else(|| "series".into());

        let Ok(eps) = std::fs::read_dir(dir.join("episodes")) else {
            continue;
        };
        for e in eps.flatten() {
            let Ok(text) = std::fs::read_to_string(e.path().join("episode.json")) else {
                continue;
            };
            let Ok(ep) = serde_json::from_str::<EpisodeData>(&text) else {
                continue;
            };
            out.push(ScriptSummary {
                project_id: project_id.clone(),
                project_title: project_title.clone(),
                project_kind: project_kind.clone(),
                episode_id: ep.id.clone(),
                episode_num: ep.episode_num,
                episode_title: ep.title.clone(),
                chars: ep.script_text.chars().count(),
                script_doc: ep.script_doc.clone(),
                shot_count: ep.storyboard.len(),
            });
        }
    }
    // 有内容的排前面，其次按集号 —— 打开这一页多半是想接着写。
    out.sort_by(|a, b| {
        (b.chars > 0)
            .cmp(&(a.chars > 0))
            .then(a.project_title.cmp(&b.project_title))
            .then(a.episode_num.cmp(&b.episode_num))
    });
    out
}

/// 删掉一整个项目。**不可逆，会连媒体一起删。**
///
/// 两步：不带 `confirm` 时只回一句「里面有什么」，让界面先说清楚
/// 要删掉多少东西；用户看过之后再带 `confirm` 真删。
/// 一个按钮直接删掉几十分钟的生成结果，是这一页里唯一真正危险的操作。
#[tauri::command]
pub fn cinema_delete_project(
    state: State<'_, AppState>,
    project_id: String,
    confirm: bool,
) -> Result<String> {
    delete_project(&state.config_dir, &project_id, confirm)
}

pub fn delete_project(config_dir: &Path, project_id: &str, confirm: bool) -> Result<String> {
    let dir = cinema_projects_dir(config_dir).join(project_id);
    if !dir.is_dir() {
        return Err(Error::runtime("这个项目已经不在了"));
    }
    // **路径必须落在 projects/ 里面。** id 是前端传来的，
    // `../..` 之类的东西不能变成一次 `remove_dir_all`。
    let root = cinema_projects_dir(config_dir)
        .canonicalize()
        .map_err(|e| Error::runtime(format!("解析项目目录失败：{e}")))?;
    let real = dir
        .canonicalize()
        .map_err(|e| Error::runtime(format!("解析项目路径失败：{e}")))?;
    if !real.starts_with(&root) || real == root {
        return Err(Error::runtime("这个路径不在项目目录里，拒绝删除"));
    }

    let item = workspace_items(config_dir)
        .into_iter()
        .find(|i| i.project.id == project_id);
    let summary = match &item {
        Some(i) => format!(
            "「{}」：{} 集 · {} 镜 · {} 已出片{}",
            i.project.title,
            i.episode_count,
            i.shot_count,
            i.rendered_count,
            if i.has_final { " · 有成片" } else { "" }
        ),
        None => format!("「{project_id}」"),
    };
    if !confirm {
        return Ok(summary);
    }
    std::fs::remove_dir_all(&real)
        .map_err(|e| Error::runtime(format!("删除失败：{e}")))?;
    tracing::info!(project = %project_id, "删掉了项目：{summary}");
    Ok(summary)
}

/// 工作区列表。一次读完所有项目的统计，不用前端逐个拉详情。
#[tauri::command]
pub fn cinema_workspace(state: State<'_, AppState>) -> Vec<WorkspaceItem> {
    workspace_items(&state.config_dir)
}

pub fn workspace_items(config_dir: &Path) -> Vec<WorkspaceItem> {
    let root = cinema_projects_dir(config_dir);
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in entries.flatten() {
        let dir = e.path();
        if !dir.is_dir() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(dir.join("project.json")) else {
            continue;
        };
        let Ok(mut project) = serde_json::from_str::<CinemaProject>(&text) else {
            continue;
        };

        let mut episode_count = 0usize;
        let mut shot_count = 0usize;
        let mut rendered_count = 0usize;
        let mut has_final = false;
        if let Ok(eps) = std::fs::read_dir(dir.join("episodes")) {
            for ep in eps.flatten() {
                let Ok(t) = std::fs::read_to_string(ep.path().join("episode.json")) else {
                    continue;
                };
                let Ok(ep) = serde_json::from_str::<EpisodeData>(&t) else {
                    continue;
                };
                episode_count += 1;
                shot_count += ep.storyboard.len();
                rendered_count += ep
                    .storyboard
                    .iter()
                    .filter(|s| s.video_url.is_some())
                    .count();
                has_final |= ep.final_video_url.is_some();
            }
        }
        // project.json 里那个 episode_count 是写入时的快照，会过时。
        project.episode_count = episode_count;
        // **封面同理：字段一直在、界面一直读，但从来没人给它赋过值。**
        // 用户自己设过的不动，没设的挑第一集第一个有首帧的镜头。
        if project.cover_image.as_deref().unwrap_or("").trim().is_empty() {
            project.cover_image = pick_project_cover(&dir);
        }

        let status = if has_final {
            "done"
        } else if rendered_count > 0 || shot_count > 0 {
            "active"
        } else {
            "draft"
        };
        out.push(WorkspaceItem {
            project,
            episode_count,
            shot_count,
            rendered_count,
            has_final,
            status,
        });
    }
    out.sort_by(|a, b| b.project.updated_at.cmp(&a.project.updated_at));
    out
}

/// 获取短剧项目的角色池列表（支持用户自定义角色、上传肖像底图与绑定 PuLID / IP-Adapter / LoRA）
#[tauri::command]
pub async fn cinema_list_characters(
    state: State<'_, AppState>,
    project_id: String,
    episode_id: Option<String>,
) -> Result<Vec<CinemaCharacter>> {
    load_cinema_characters(&state.config_dir, &project_id, episode_id.as_deref())
}


// ---------------------------------------------------------------------------
// 角色：资产库是唯一的所有者
// ---------------------------------------------------------------------------
//
// 原来角色有**两个所有者**：`characters.json` 里的 `CinemaCharacter`，
// 和资产库里的 `Asset`。于是「这一镜是谁」有两个答案 ——
// `shot.character_id` 走旧的，`@` 引用走新的。**一个概念两个所有者
// 就是「流程乱」的根。**
//
// 现在 `CinemaCharacter` 是资产的一个**视图**：读的时候从资产库投影出来，
// 写的时候写回资产库。`characters.json` 只在迁移时读一次，之后不再是真相。

/// 资产 → 短剧角色。给现有的角色界面用。
pub fn asset_to_character(
    config_dir: &Path,
    scope: &crate::assets::Scope,
    a: &crate::assets::Asset,
) -> CinemaCharacter {
    let t = a.character.clone().unwrap_or_default();
    // 头像取当前选中的图变体，转成界面能直接显示的 URL。
    let avatar_url = a
        .unit
        .selected_image()
        .map(|v| crate::assets::media_url(&crate::assets::variant_abs_path(config_dir, scope, v)))
        .unwrap_or_default();
    let sheet = t.asset_sheet.clone();
    CinemaCharacter {
        id: a.id.clone(),
        name: a.name.clone(),
        role: a.description.clone(),
        avatar_url,
        trigger_words: t.trigger_words,
        lora_model: t.lora_model,
        lora_weight: t.lora_weight.unwrap_or(0.85),
        consistency_mode: t.consistency_mode.unwrap_or_else(|| "pulid".into()),
        clothing_prompt: t.clothing_prompt,
        multi_angles: sheet.as_ref().map(|s| s.angles.clone()).unwrap_or_default(),
        asset_sheet: sheet,
        visual_dna: t.visual_dna,
        seed: t.seed,
        episode_id: match scope {
            crate::assets::Scope::Episode { episode_id, .. } => Some(episode_id.clone()),
            _ => None,
        },
    }
}

/// 短剧角色 → 资产（就地更新，**不碰变体池**）。
///
/// 头像是投影出来的显示 URL，不写回去 —— 写回去就等于把显示层的东西
/// 存进存储，和当初把 base64 存进 JSON 是同一类错误。
/// 换头像走资产库的变体选择。
fn apply_character_to_asset(a: &mut crate::assets::Asset, c: &CinemaCharacter) {
    a.name = c.name.clone();
    a.description = c.role.clone();
    let mut t: crate::assets::CharacterTraits = a.character.clone().unwrap_or_default();
    t.trigger_words = c.trigger_words.clone();
    t.lora_model = c.lora_model.clone();
    t.lora_weight = Some(c.lora_weight);
    t.consistency_mode = Some(c.consistency_mode.clone());
    t.clothing_prompt = c.clothing_prompt.clone();
    t.seed = c.seed;
    if c.asset_sheet.is_some() {
        t.asset_sheet = c.asset_sheet.clone();
    }
    if c.visual_dna.is_some() {
        t.visual_dna = c.visual_dna.clone();
    }
    a.character = Some(t);
    a.updated_at = crate::assets::now_ms();
}

/// 相对路径 → 显示 URL。见 [`crate::assets::hydrate_media_urls`]。
fn hydrate_characters(
    config_dir: &Path,
    project_id: &str,
    chars: Vec<CinemaCharacter>,
) -> Vec<CinemaCharacter> {
    map_character_media(config_dir, project_id, chars)
}

fn map_character_media(
    config_dir: &Path,
    project_id: &str,
    chars: Vec<CinemaCharacter>,
) -> Vec<CinemaCharacter> {
    let scope = crate::assets::Scope::Project {
        project_id: project_id.to_string(),
    };
    let Ok(mut value) = serde_json::to_value(&chars) else {
        return chars;
    };
    crate::assets::hydrate_media_urls(config_dir, &scope, &mut value);
    serde_json::from_value(value).unwrap_or(chars)
}

/// 读这个项目的角色。
///
/// **两层都读**：项目级（跨集共用）+ 本集级（刚提取、还没归属）。
/// 只读项目级的话，刚提取出来的角色点进去会说「找不到」——
/// 这是把提取改成落本集之后我自己造出来的。
pub fn load_cinema_characters(
    config_dir: &Path,
    project_id: &str,
    episode_id: Option<&str>,
) -> Result<Vec<CinemaCharacter>> {
    let mut local: Vec<CinemaCharacter> = Vec::new();
    if let Some(ep) = episode_id {
        let ep_scope = crate::assets::Scope::Episode {
            project_id: project_id.to_string(),
            episode_id: ep.to_string(),
        };
        local = crate::assets::load_store(config_dir, &ep_scope)
            .assets
            .iter()
            .filter(|a| a.kind == crate::assets::AssetKind::Character)
            .map(|a| asset_to_character(config_dir, &ep_scope, a))
            .collect();
    }
    let mut out = load_project_characters(config_dir, project_id)?;
    // 本集的排前面 —— 它们是这一集正在处理的东西
    local.append(&mut out);
    Ok(local)
}

fn load_project_characters(config_dir: &Path, project_id: &str) -> Result<Vec<CinemaCharacter>> {
    // **资产库是唯一的所有者。** 里面有角色就用它，`characters.json`
    // 只在还没迁过的时候读一次（迁移会把它写进资产库）。
    let scope = crate::assets::Scope::Project {
        project_id: project_id.to_string(),
    };
    let store = crate::assets::load_store(config_dir, &scope);
    let from_assets: Vec<CinemaCharacter> = store
        .assets
        .iter()
        .filter(|a| a.kind == crate::assets::AssetKind::Character)
        .map(|a| asset_to_character(config_dir, &scope, a))
        .collect();
    if !from_assets.is_empty() {
        return Ok(from_assets);
    }
    load_cinema_characters_legacy(config_dir, project_id)
}

/// 迁移之前的读法。**只在资产库里一个角色都没有时才走这条。**
fn load_cinema_characters_legacy(
    config_dir: &Path,
    project_id: &str,
) -> Result<Vec<CinemaCharacter>> {
    let root = cinema_projects_dir(config_dir);
    let proj_dir = root.join(project_id);
    let char_file = proj_dir.join("characters.json");
    if char_file.is_file() {
        if let Ok(c) = std::fs::read_to_string(&char_file) {
            if let Ok(mut chars) = serde_json::from_str::<Vec<CinemaCharacter>>(&c) {
                let mut changed = false;
                for ch in &mut chars {
                    let is_empty = ch.asset_sheet.as_ref().map_or(true, |s| {
                        s.turnarounds.is_empty()
                            && s.angles.is_empty()
                            && s.expressions.is_empty()
                            && s.poses.is_empty()
                    });
                    if is_empty {
                        if let Some(discovered) =
                            discover_character_asset_sheet_from_disk(config_dir, project_id, &ch.id)
                        {
                            ch.multi_angles = discovered.angles.clone();
                            ch.asset_sheet = Some(discovered);
                            changed = true;
                        }
                    }
                }
                if changed {
                    let _ = std::fs::write(
                        &char_file,
                        serde_json::to_string_pretty(&chars).unwrap_or_default(),
                    );
                }
                // 磁盘上存的是相对 `media/` 的路径（迁移之后不再有 base64），
                // 界面要的是能塞进 `<img src>` 的 URL。转换只发生在这一层。
                return Ok(hydrate_characters(config_dir, project_id, chars));
            }
        }
    }

    // **文件不存在就是没有角色。**
    //
    // 这里原来会塞 3 个样板角色（由美子/苏沐雪/顾辰）**并写盘** ——
    // 于是任何一个新项目一读角色列表就凭空多出三个别人的人设，
    // 而且是持久化的。和「新项目预填一整集样板剧本」是同一个毛病。
    // 角色由第 1 步的「提取实体」从剧本里来，或者手动新建。
    Ok(Vec::new())
}

/// 写角色 —— **写进资产库**，不再写 `characters.json`。
pub fn save_cinema_character(
    config_dir: &Path,
    project_id: &str,
    character: CinemaCharacter,
) -> Result<Vec<CinemaCharacter>> {
    use crate::assets::{Asset, AssetKind};
    // **回写原来那一层。** 一律写项目级的话，编辑一个本集角色会在
    // 项目池里凭空复制一份 —— 不报错，表现是「怎么多了个一样的人」。
    let scope = match character.episode_id.as_deref() {
        Some(ep) => crate::assets::Scope::Episode {
            project_id: project_id.to_string(),
            episode_id: ep.to_string(),
        },
        None => crate::assets::Scope::Project {
            project_id: project_id.to_string(),
        },
    };
    // 资产库还是空的（老项目第一次动角色）：先把 characters.json 灌进去，
    // 免得这一次保存把其它角色弄丢。
    let mut store = crate::assets::load_store(config_dir, &scope);
    if !store
        .assets
        .iter()
        .any(|a| a.kind == AssetKind::Character)
    {
        for legacy in load_cinema_characters_legacy(config_dir, project_id).unwrap_or_default() {
            let mut a = Asset::new(AssetKind::Character, legacy.name.clone());
            a.id = legacy.id.clone();
            apply_character_to_asset(&mut a, &legacy);
            store.assets.push(a);
        }
    }

    match store.assets.iter_mut().find(|a| a.id == character.id) {
        Some(a) => apply_character_to_asset(a, &character),
        None => {
            let mut a = Asset::new(AssetKind::Character, character.name.clone());
            if !character.id.trim().is_empty() {
                a.id = character.id.clone();
            }
            apply_character_to_asset(&mut a, &character);
            store.assets.push(a);
        }
    }
    crate::assets::save_store(config_dir, &scope, &store)?;
    load_cinema_characters(config_dir, project_id, None)
}

/// 辅助解析肖像源图像二进制字节（支持 Base64、相对预设、绝对路径及多级资产目录）
fn resolve_portrait_bytes(url: &str, config_dir: &Path) -> Option<Vec<u8>> {
    use base64::Engine;
    // 拆完 base64 之后，角色的 `avatar_url` 读出来是 `anima-media://` 显示 URL
    // （磁盘上存的是 `media/` 下的相对路径）。**这里必须认得它** ——
    // 不认的话锁脸参考图会静默变成 None，出图不报错，只是人不像了。
    if url.starts_with(crate::assets::MEDIA_SCHEME) {
        if let Some(p) = crate::assets::parse_media_request(url) {
            if let Ok(bytes) = std::fs::read(&p) {
                return Some(bytes);
            }
        }
        return None;
    }
    if url.starts_with("data:image/") {
        if let Some(pos) = url.find(',') {
            let b64 = &url[pos + 1..];
            return base64::engine::general_purpose::STANDARD.decode(b64).ok();
        }
        return None;
    }

    let direct_path = PathBuf::from(url);
    if direct_path.is_file() {
        if let Ok(bytes) = std::fs::read(&direct_path) {
            return Some(bytes);
        }
    }

    let clean_name = url
        .trim_start_matches('/')
        .trim_start_matches("portraits/")
        .trim_start_matches("avatars/")
        .trim_start_matches("neural/");

    let candidate_paths = [
        PathBuf::from("public").join("portraits").join(clean_name),
        PathBuf::from("public").join(clean_name),
        PathBuf::from("avatars").join("neural").join(clean_name),
        PathBuf::from("avatars").join(clean_name),
        config_dir.join("models").join("neural").join(clean_name),
        config_dir.join("avatars").join(clean_name),
        config_dir.join("avatars").join("neural").join(clean_name),
        PathBuf::from("/Users/you/AI/workspace/Anima/public/portraits").join(clean_name),
        PathBuf::from("/Users/you/AI/workspace/Anima/avatars/neural").join(clean_name),
    ];

    for p in &candidate_paths {
        if p.is_file() {
            if let Ok(bytes) = std::fs::read(p) {
                return Some(bytes);
            }
        }
    }

    None
}

#[derive(Debug, Deserialize)]
pub struct SliceSheetReq {
    pub project_id: String,
    pub character_id: String,
    pub sheet_image_data: String,
}

/// 智能解析并切割导入 C 站 / Liblib 漫剧全套大图（三视图 + 表情模组 + 动作姿态）
#[tauri::command]
pub async fn cinema_slice_character_sheet(
    state: State<'_, AppState>,
    req: SliceSheetReq,
) -> Result<CharacterAssetSheet> {
    cinema_slice_character_sheet_with_config_dir(&state.config_dir, req).await
}

pub async fn cinema_slice_character_sheet_with_config_dir(
    config_dir: &Path,
    req: SliceSheetReq,
) -> Result<CharacterAssetSheet> {
    use base64::Engine;
    let root = cinema_projects_dir(config_dir);
    let proj_dir = root.join(&req.project_id);
    let angles_dir = proj_dir
        .join("characters")
        .join(&req.character_id)
        .join("angles");
    let _ = std::fs::create_dir_all(&angles_dir);

    let raw_bytes = resolve_portrait_bytes(&req.sheet_image_data, config_dir)
        .ok_or_else(|| Error::runtime("无法读取漫剧素材大图数据"))?;

    let dyn_img = image::load_from_memory(&raw_bytes)
        .map_err(|e| Error::runtime(format!("解析素材图失败: {e}")))?;

    let (w, h) = (dyn_img.width() as f32, dyn_img.height() as f32);

    let mut turnarounds = Vec::new();
    let mut expressions = Vec::new();
    let mut poses = Vec::new();

    // 1. 切割顶部全身三视图 (Top 42%: 正面全身、侧面全身、背面全身)
    let t_y = 0u32;
    let t_h = (h * 0.42) as u32;
    let t_w = (w / 3.0) as u32;

    let t_defs = [
        (
            "turnaround_front",
            "正面全身三视图",
            "full-body front view standing, showing full costume",
            0,
        ),
        (
            "turnaround_side",
            "侧面全身三视图",
            "full-body side profile view standing",
            1,
        ),
        (
            "turnaround_back",
            "背面全身三视图",
            "full-body back view standing, back details of costume",
            2,
        ),
    ];
    for (key, title, desc, idx) in t_defs {
        let x = (idx as f32 * t_w as f32) as u32;
        let cropped = dyn_img.crop_imm(
            x,
            t_y,
            t_w.min(dyn_img.width() - x),
            t_h.min(dyn_img.height() - t_y),
        );
        let out_path = angles_dir.join(format!("{}.png", key));
        let mut buf = std::io::Cursor::new(Vec::new());
        let _ = cropped.write_to(&mut buf, image::ImageFormat::Png);
        let png_bytes = buf.into_inner();
        let _ = std::fs::write(&out_path, &png_bytes);
        turnarounds.push(CharacterViewItem {
            key: key.to_string(),
            title: title.to_string(),
            image_url: format!(
                "data:image/png;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(&png_bytes)
            ),
            description: desc.to_string(),
            category: "turnaround".to_string(),
        });
    }

    // 2. 切割中部表情模组 (Middle 26%: 6 个大头表情)
    let e_y = (h * 0.42) as u32;
    let e_h = (h * 0.26) as u32;
    let e_w = (w / 6.0) as u32;
    let e_defs = [
        ("expr_smile", "微笑 (Joy)", "smiling joyfully"),
        ("expr_crying", "悲伤哭泣 (Sad)", "crying with tears"),
        (
            "expr_shocked",
            "震惊张口 (Shock)",
            "shocked surprised expression",
        ),
        ("expr_angry", "愤怒冷厉 (Angry)", "intense fierce glare"),
        ("expr_pout", "委屈撒娇 (Pout)", "pouting lips, cute sulking"),
        ("expr_calm", "冷酷从容 (Calm)", "cool composed expression"),
    ];
    for (idx, (key, title, desc)) in e_defs.iter().enumerate() {
        let x = (idx as f32 * e_w as f32) as u32;
        let cropped = dyn_img.crop_imm(
            x,
            e_y,
            e_w.min(dyn_img.width() - x),
            e_h.min(dyn_img.height() - e_y),
        );
        let out_path = angles_dir.join(format!("{}.png", key));
        let mut buf = std::io::Cursor::new(Vec::new());
        let _ = cropped.write_to(&mut buf, image::ImageFormat::Png);
        let png_bytes = buf.into_inner();
        let _ = std::fs::write(&out_path, &png_bytes);
        expressions.push(CharacterViewItem {
            key: key.to_string(),
            title: title.to_string(),
            image_url: format!(
                "data:image/png;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(&png_bytes)
            ),
            description: desc.to_string(),
            category: "expression".to_string(),
        });
    }

    // 3. 切割底部 4 大动态姿态 (Bottom 32%: 4 个全身姿态)
    let p_y = (h * 0.68) as u32;
    let p_h = (h * 0.32) as u32;
    let p_w = (w / 4.0) as u32;
    let p_defs = [
        (
            "pose_standing",
            "站姿对话 (Standing)",
            "standing gracefully, talking posture",
        ),
        ("pose_running", "快步奔跑 (Running)", "running dynamically"),
        (
            "pose_jumping",
            "喜悦跃起 (Jumping)",
            "jumping with joy, energetic action",
        ),
        (
            "pose_weeping",
            "掩面崩溃 (Weeping)",
            "covering face with hands despair",
        ),
    ];
    for (idx, (key, title, desc)) in p_defs.iter().enumerate() {
        let x = (idx as f32 * p_w as f32) as u32;
        let cropped = dyn_img.crop_imm(
            x,
            p_y,
            p_w.min(dyn_img.width() - x),
            p_h.min(dyn_img.height() - p_y),
        );
        let out_path = angles_dir.join(format!("{}.png", key));
        let mut buf = std::io::Cursor::new(Vec::new());
        let _ = cropped.write_to(&mut buf, image::ImageFormat::Png);
        let png_bytes = buf.into_inner();
        let _ = std::fs::write(&out_path, &png_bytes);
        poses.push(CharacterViewItem {
            key: key.to_string(),
            title: title.to_string(),
            image_url: format!(
                "data:image/png;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(&png_bytes)
            ),
            description: desc.to_string(),
            category: "pose".to_string(),
        });
    }

    // 获取现有 angles
    let mut chars = load_cinema_characters(config_dir, &req.project_id, None).unwrap_or_default();
    let existing_angles = chars
        .iter()
        .find(|c| c.id == req.character_id)
        .map(|c| c.multi_angles.clone())
        .unwrap_or_default();

    let asset_sheet = CharacterAssetSheet {
        turnarounds,
        expressions,
        poses,
        angles: existing_angles,
        warnings: Vec::new(),
    };

    if let Some(char_item) = chars.iter_mut().find(|c| c.id == req.character_id) {
        char_item.asset_sheet = Some(asset_sheet.clone());
        let char_file = proj_dir.join("characters.json");
        let content = serde_json::to_string_pretty(&chars).unwrap_or_default();
        let _ = std::fs::write(char_file, content);
    }

    Ok(asset_sheet)
}

#[derive(Debug, Deserialize)]
pub struct UpdateAssetSlotReq {
    pub project_id: String,
    pub character_id: String,
    pub category: String, // "turnaround" | "expression" | "pose" | "angle"
    pub key: String,
    pub image_data: String,
}

/// 单独上传或替换某一个具体的姿态、表情或三视图槽位
#[tauri::command]
pub async fn cinema_update_character_asset_slot(
    state: State<'_, AppState>,
    req: UpdateAssetSlotReq,
) -> Result<CharacterAssetSheet> {
    cinema_update_character_asset_slot_with_config_dir(&state.config_dir, req).await
}

pub async fn cinema_update_character_asset_slot_with_config_dir(
    config_dir: &Path,
    req: UpdateAssetSlotReq,
) -> Result<CharacterAssetSheet> {
    use base64::Engine;
    let root = cinema_projects_dir(config_dir);
    let proj_dir = root.join(&req.project_id);
    let angles_dir = proj_dir
        .join("characters")
        .join(&req.character_id)
        .join("angles");
    let _ = std::fs::create_dir_all(&angles_dir);

    let raw_bytes = resolve_portrait_bytes(&req.image_data, config_dir)
        .ok_or_else(|| Error::runtime("无法读取上传的图片数据"))?;

    let out_path = angles_dir.join(format!("{}.png", req.key));
    let _ = std::fs::write(&out_path, &raw_bytes);
    let data_url = format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&raw_bytes)
    );

    let mut chars = load_cinema_characters(config_dir, &req.project_id, None).unwrap_or_default();
    let char_item = chars
        .iter_mut()
        .find(|c| c.id == req.character_id)
        .ok_or_else(|| Error::runtime("未找到该角色"))?;

    let mut sheet = char_item.asset_sheet.clone().unwrap_or_default();

    match req.category.as_str() {
        "turnaround" => {
            if let Some(item) = sheet.turnarounds.iter_mut().find(|i| i.key == req.key) {
                item.image_url = data_url;
            } else {
                sheet.turnarounds.push(CharacterViewItem {
                    key: req.key.clone(),
                    title: req.key.clone(),
                    image_url: data_url,
                    description: String::new(),
                    category: "turnaround".to_string(),
                });
            }
        }
        "expression" => {
            if let Some(item) = sheet.expressions.iter_mut().find(|i| i.key == req.key) {
                item.image_url = data_url;
            } else {
                sheet.expressions.push(CharacterViewItem {
                    key: req.key.clone(),
                    title: req.key.clone(),
                    image_url: data_url,
                    description: String::new(),
                    category: "expression".to_string(),
                });
            }
        }
        "pose" => {
            if let Some(item) = sheet.poses.iter_mut().find(|i| i.key == req.key) {
                item.image_url = data_url;
            } else {
                sheet.poses.push(CharacterViewItem {
                    key: req.key.clone(),
                    title: req.key.clone(),
                    image_url: data_url,
                    description: String::new(),
                    category: "pose".to_string(),
                });
            }
        }
        "angle" => {
            let title = crate::cinema_sheet::FACE_ANGLES
                .iter()
                .find(|slot| slot.key == req.key)
                .map(|slot| slot.title.to_string())
                .unwrap_or_else(|| req.key.clone());
            if let Some(item) = sheet
                .angles
                .iter_mut()
                .find(|a| a.angle_name == title || a.angle_name.contains(&req.key))
            {
                item.image_url = data_url;
            } else {
                sheet.angles.push(CharacterAngle {
                    angle_name: title,
                    image_url: data_url,
                    description: String::new(),
                });
            }
        }
        _ => {}
    }

    char_item.asset_sheet = Some(sheet.clone());
    let char_file = proj_dir.join("characters.json");
    let content = serde_json::to_string_pretty(&chars).unwrap_or_default();
    let _ = std::fs::write(char_file, content);

    Ok(sheet)
}

#[derive(Debug, Deserialize)]
pub struct GenerateSheetComfyReq {
    pub project_id: String,
    pub character_id: String,
    pub name: String,
    pub trigger_words: String,
    pub clothing_prompt: Option<String>,
    pub front_image_url: Option<String>,
    pub checkpoint: Option<String>,
    pub lora: Option<String>,
    pub lora_weight: Option<f32>,
    pub consistency_mode: Option<String>,
    pub seed: Option<i64>,
    /// 8 维视觉基因结构化特征
    pub visual_dna: Option<crate::cinema_dna::CharacterVisualDna>,
    /// 三视图是否也用底图当参考。
    pub use_reference_for_turnaround: Option<bool>,
    /// 这次要生成哪几档："turnaround" / "expressions" / "poses" / "angles"。
    pub tiers: Option<Vec<String>>,
    /// 三视图整排走哪条路："sdxl"（默认）/ "sd15"。
    pub engine: Option<String>,
    /// 调试模式：每档只跑第一格，并用低步数/单次尝试。
    pub debug_one_slot: Option<bool>,
    /// 单格最多尝试次数。None = 默认 3 次。
    pub max_attempts: Option<u32>,
}

/// PuLID_ComfyUI 节点能读的那份权重。
///
/// **不是 `ip-adapter_pulid_sdxl.safetensors`。** 本机 `pulid/` 下原有的那份和
/// `pulid_v1.1.safetensors` 是同一个文件（SHA-256 相同），研究仓库的原始格式，
/// 键名是 `id_adapter.*`；节点要的是 `image_proj.*` / `ip_adapter.*`，读到就报错。
/// 2026-09-13 实跑锁脸有效的是节点 README 指定的 `_fp16` 这份（见 ASSET_LEDGER）。
///
/// 角色的一致性模式默认就是 `pulid`，所以写错这个名字等于默认路径必挂。
const PULID_SDXL_FILE: &str = "ip-adapter_pulid_sdxl_fp16.safetensors";

fn pulid_file_for_checkpoint(checkpoint: &str) -> String {
    let _ = checkpoint;
    PULID_SDXL_FILE.to_string()
}

fn checkpoint_is_sdxl(checkpoint: &str) -> bool {
    let ckpt = checkpoint.to_lowercase();
    ckpt.contains("xl")
        || ckpt.contains("sdxl")
        || ckpt.contains("realvis")
        || ckpt.contains("awportrait")
}

fn tune_character_sheet_shot(
    mut shot: crate::cinema_sheet::Shot,
    consistency_mode: &str,
    lora: Option<(String, f32)>,
    debug_one_slot: bool,
) -> crate::cinema_sheet::Shot {
    use crate::cinema_sheet::IdentityMode;

    if shot.reference.is_none() || consistency_mode == "prompt_only" {
        shot.identity_mode = IdentityMode::None;
    } else if consistency_mode == "pulid" && checkpoint_is_sdxl(&shot.checkpoint) {
        shot.identity_mode = IdentityMode::Pulid {
            file: pulid_file_for_checkpoint(&shot.checkpoint),
        };
    } else {
        shot.identity_mode = IdentityMode::IpAdapter;
        if consistency_mode == "reference_only" {
            shot.reference_weight = (shot.reference_weight + 0.15).min(0.90);
            shot.reference_end_at = shot.reference_end_at.max(0.90);
        }
    }

    if consistency_mode == "lora" {
        if let Some(lora) = lora {
            shot.lora = Some(lora);
        }
    }

    if debug_one_slot {
        shot.steps = shot.steps.min(10);
        shot.cfg = shot.cfg.min(5.5);
        shot.hires = None;
        let max_side = shot.width.max(shot.height);
        if max_side > 768 {
            let scale = 768.0 / max_side as f32;
            let snap = |v: u32| ((v.max(384) / 8) * 8).max(384);
            shot.width = snap((shot.width as f32 * scale).round() as u32);
            shot.height = snap((shot.height as f32 * scale).round() as u32);
        }
    }

    shot
}

/// 调度本地 ComfyUI 扩散模型生成标准漫剧全套大图并自动切片装配
#[tauri::command]
pub async fn cinema_generate_character_sheet_via_comfyui(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    req: GenerateSheetComfyReq,
) -> Result<CharacterAssetSheet> {
    cinema_generate_character_sheet_via_comfyui_with_deps(
        Some(&app),
        &state.config_dir,
        state.http.clone(),
        req,
    )
    .await
}

pub async fn cinema_generate_character_sheet_via_comfyui_with_deps(
    app: Option<&tauri::AppHandle>,
    config_dir: &Path,
    http: reqwest::Client,
    req: GenerateSheetComfyReq,
) -> Result<CharacterAssetSheet> {
    use crate::cinema_sheet as sheet;
    use tauri::Emitter;

    let client = &http;
    let engine = req
        .engine
        .as_deref()
        .map(sheet::SheetEngine::from_id)
        .unwrap_or_default();
    let project_cfg = load_cinema_engine_config(config_dir, &req.project_id).unwrap_or_default();
    let effective_checkpoint = req.checkpoint.clone().or_else(|| {
        (!project_cfg.checkpoint.trim().is_empty()).then_some(project_cfg.checkpoint.clone())
    });
    let consistency_mode = req
        .consistency_mode
        .clone()
        .unwrap_or_else(|| project_cfg.consistency_engine.clone());
    let debug_one_slot = req.debug_one_slot.unwrap_or(false);
    let max_attempts = if debug_one_slot {
        1
    } else {
        req.max_attempts.unwrap_or(sheet::MAX_ATTEMPTS)
    };

    let root = cinema_projects_dir(config_dir);
    let proj_dir = root.join(&req.project_id);
    let angles_dir = proj_dir
        .join("characters")
        .join(&req.character_id)
        .join("angles");
    let _ = std::fs::create_dir_all(&angles_dir);

    let home = std::env::var("HOME").unwrap_or_default();
    let ckpt_dirs = [
        PathBuf::from(format!("{home}/AI/ComfyUI/models/checkpoints")),
        PathBuf::from(format!("{home}/ComfyUI/models/checkpoints")),
    ];
    let available_ckpts: Vec<String> = ckpt_dirs
        .iter()
        .filter_map(|d| std::fs::read_dir(d).ok())
        .flat_map(|rd| rd.flatten())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".safetensors") || n.ends_with(".ckpt"))
        .collect();

    let auto_portrait = || {
        sheet::pick_portrait_checkpoint(&available_ckpts)
            .unwrap_or_else(|| "RealVisXL_V5.0_fp16.safetensors".to_string())
    };

    let ckpt_portrait = effective_checkpoint
        .clone()
        .unwrap_or_else(&auto_portrait);
    let ckpt_turnaround = effective_checkpoint
        .clone()
        .filter(|c| engine.checkpoint_matches(c))
        .or_else(|| engine.pick_checkpoint(&available_ckpts))
        .unwrap_or_else(auto_portrait);

    let has_user_avatar = req.front_image_url.is_some();
    let (reference, reference_face_only) = if let Some(url) = &req.front_image_url {
        if let Some(bytes) = resolve_portrait_bytes(url, config_dir) {
            let input_dir = comfy_input_dir();
            let _ = std::fs::create_dir_all(&input_dir);
            let full_name = format!("ref_{}.png", req.character_id);
            let _ = std::fs::write(input_dir.join(&full_name), &bytes);

            // 提取纯面部特写（去除参考底图的便服/姿势/背景），给全身立绘与三视图使用，彻底避免参考图便服覆盖角色设定服装
            let face_name = if let Ok(dyn_img) = image::load_from_memory(&bytes) {
                let (w, h) = (dyn_img.width(), dyn_img.height());
                // 裁切头顶至脖颈区域（宽 70% 居中，高 55% 顶部），剔除肩膀胸前衣服与坐姿
                let crop_x = (w as f32 * 0.15) as u32;
                let crop_w = (w as f32 * 0.70) as u32;
                let crop_h = (h as f32 * 0.55) as u32;
                let face_crop = dyn_img.crop_imm(crop_x, 0, crop_w.min(w), crop_h.min(h));
                let mut buf = std::io::Cursor::new(Vec::new());
                if face_crop
                    .write_to(&mut buf, image::ImageFormat::Png)
                    .is_ok()
                {
                    let fn_name = format!("ref_face_tight_{}.png", req.character_id);
                    let _ = std::fs::write(input_dir.join(&fn_name), buf.into_inner());
                    Some(fn_name)
                } else {
                    Some(full_name.clone())
                }
            } else {
                Some(full_name.clone())
            };
            (Some(full_name), face_name)
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };

    // 版式 LoRA 同理 —— 两条路的 LoRA 不能互换
    let lora = req
        .lora
        .clone()
        .or_else(|| project_cfg.lora.clone())
        .or_else(|| sheet::find_layout_lora(engine));
    let character_lora = lora.clone().map(|name| {
        (
            name,
            req.lora_weight
                .unwrap_or(project_cfg.lora_weight)
                .clamp(0.05, 1.5),
        )
    });

    // 当用户上传了具体真人底图时，底图自身就已经带有精准的五官与族裔特征。
    // 此时停用 Asian_Beauty LoRA，避免韩系网红脸覆盖用户底图的原生骨相与气质；
    // 仅当用户不传底图纯靠 Prompt 盲抽时，才启用辅助亚洲人像 LoRA。
    let asian = if has_user_avatar {
        None
    } else {
        sheet::find_face_boost_lora(sheet::SheetEngine::Sdxl).map(|l| (l, sheet::ASIAN_LORA_WEIGHT))
    };
    let asian_turnaround = if has_user_avatar {
        None
    } else {
        sheet::find_face_boost_lora(engine).map(|l| (l, sheet::ASIAN_LORA_WEIGHT))
    };
    if asian.is_some() {
        tracing::info!("挂上亚洲人像 LoRA");
    }
    // 种子的三级来源：本次请求 → 角色档案里存的 → 随机。
    // **档案优先于随机**，这样同一个角色重复生成结果一致；
    // 请求优先于档案，是为了让界面上的「换一个」能覆盖
    let stored_seed = load_character_seed(config_dir, &req.project_id, &req.character_id);
    let base_seed = req.seed.or(stored_seed).unwrap_or_else(|| {
        use std::time::{SystemTime, UNIX_EPOCH};
        (SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            % 1_000_000_000) as i64
    });
    // 首次生成把它落盘，之后就固定了
    if stored_seed != Some(base_seed) {
        save_character_seed(config_dir, &req.project_id, &req.character_id, base_seed);
    }
    // **族裔词与视觉 DNA 必须在这里补齐，四档共用同一份。**
    let mut trigger = sheet::ensure_ethnicity(&req.trigger_words);
    if let Some(ref dna) = req.visual_dna {
        let block = dna.to_prompt_block(&req.name);
        if !block.is_empty() {
            trigger = format!("{trigger}, {block}");
        }
    }

    let clothing = req
        .clothing_prompt
        .clone()
        .unwrap_or_else(|| "modern luxury elegant costume, high fashion styling".into());

    let tiers = req
        .tiers
        .clone()
        .unwrap_or_else(|| vec!["turnaround".into()]);
    let want = |t: &str| tiers.iter().any(|x| x == t);

    // 总数用来算进度。**要在开始前就算出来** ——
    // 用户需要知道「一共几张」，否则进度条没有意义。
    let count_for = |n: usize| if debug_one_slot { n.min(1) } else { n };
    let total = (if want("turnaround") {
        count_for(sheet::TURNAROUND_TITLES.len())
    } else {
        0
    }) + (if want("expressions") {
        count_for(sheet::EXPRESSIONS.len())
    } else {
        0
    }) + (if want("poses") {
        count_for(sheet::POSES.len())
    } else {
        0
    }) + (if want("angles") {
        count_for(sheet::FACE_ANGLE_TITLES.len())
    } else {
        0
    });
    let mut done = 0usize;
    let progress = |done: usize, total: usize, label: &str| {
        if let Some(app) = app {
            let _ = app.emit(
                "anima://cinema-sheet-progress",
                json!({ "done": done, "total": total, "label": label }),
            );
        }
    };

    let mut out = CharacterAssetSheet::default();
    let mut warnings: Vec<String> = Vec::new();
    if consistency_mode == "pulid" && !checkpoint_is_sdxl(&ckpt_turnaround) {
        warnings.push(format!(
            "当前三视图使用 {ckpt_turnaround}，PuLID_ComfyUI 本地节点按 SDXL PuLID 模型工作；已自动降级为 IPAdapter，避免 SD1.5 快速测试报 IDEncoder 权重不匹配。"
        ));
    }

    // ── 三视图：逐格精准定向出图（正面 / 侧面 / 背面 各 1 格 832x1216 独立立绘）────
    if want("turnaround") {
        let slots = sheet::TURNAROUND_SLOTS
            .iter()
            .take(count_for(sheet::TURNAROUND_SLOTS.len()));
        for (i, slot) in slots.enumerate() {
            progress(done, total, slot.title);
            let shot = sheet::turnaround_view(
                engine,
                &ckpt_turnaround,
                &trigger,
                &clothing,
                slot.prompt,
                slot.negative,
                if req.use_reference_for_turnaround.unwrap_or(false) {
                    reference_face_only.clone()
                } else {
                    None
                },
                base_seed.wrapping_add(1_000 + i as i64),
                &format!("Anima_Turn_{}_{}", req.character_id, slot.key),
            );
            let shot = sheet::Shot {
                lora2: asian_turnaround.clone(),
                ..shot
            };
            let shot = tune_character_sheet_shot(
                shot,
                &consistency_mode,
                character_lora.clone(),
                debug_one_slot,
            );
            match sheet::render_with_attempts(client, shot, 1, max_attempts).await {
                Ok(r) => {
                    warnings.extend(r.notes.clone());
                    if let Ok(img) = image::load_from_memory(&r.bytes) {
                        out.turnarounds.push(save_view(
                            &angles_dir,
                            slot.key,
                            slot.title,
                            "turnaround",
                            &img,
                        ));
                    }
                }
                Err(e) => warnings.push(format!("{} 生成失败：{e}", slot.title)),
            }
            done = out.turnarounds.len();
        }
        progress(done, total, "三视图完成");
    }

    // ── 面部机位：逐格定向生成（5 维精确机位 + IP-Adapter 100% 锁脸）────
    if want("angles") {
        let slots = sheet::FACE_ANGLES
            .iter()
            .take(count_for(sheet::FACE_ANGLES.len()));
        for (i, slot) in slots.enumerate() {
            progress(done, total, slot.title);
            let shot = sheet::face_angle(
                &ckpt_portrait,
                &trigger,
                slot.prompt,
                reference.clone(),
                base_seed.wrapping_add(3_000 + i as i64),
                &format!("Anima_Angle_{}_{}", req.character_id, slot.key),
            );
            let shot = sheet::Shot {
                lora2: asian.clone(),
                ..shot
            };
            let shot = tune_character_sheet_shot(
                shot,
                &consistency_mode,
                character_lora.clone(),
                debug_one_slot,
            );
            match sheet::render_with_attempts(client, shot, 1, max_attempts).await {
                Ok(r) => {
                    warnings.extend(r.notes.clone());
                    if let Ok(img) = image::load_from_memory(&r.bytes) {
                        let item = save_view(&angles_dir, slot.key, slot.title, "angle", &img);
                        out.angles.push(CharacterAngle {
                            angle_name: slot.title.to_string(),
                            image_url: item.image_url,
                            description: String::new(),
                        });
                    }
                }
                Err(e) => warnings.push(format!("{} 生成失败：{e}", slot.title)),
            }
            done = out.turnarounds.len() + out.angles.len();
        }
        progress(done, total, "面部机位完成");
    }

    // ── 表情：逐格生成 ────────────────────────────────────
    if want("expressions") {
        let slots = sheet::EXPRESSIONS
            .iter()
            .take(count_for(sheet::EXPRESSIONS.len()));
        for (i, slot) in slots.enumerate() {
            progress(done, total, slot.title);
            let shot = sheet::expression(
                &ckpt_portrait,
                &trigger,
                slot.prompt,
                reference.clone(),
                base_seed.wrapping_add(1_000 + i as i64),
                &format!("Anima_Expr_{}_{}", req.character_id, slot.key),
            );
            let shot = sheet::Shot {
                lora2: asian.clone(),
                ..shot
            };
            let shot = tune_character_sheet_shot(
                shot,
                &consistency_mode,
                character_lora.clone(),
                debug_one_slot,
            );
            match sheet::render_with_attempts(client, shot, 1, max_attempts).await {
                Ok(r) => {
                    warnings.extend(r.notes.clone());
                    if let Ok(img) = image::load_from_memory(&r.bytes) {
                        out.expressions.push(save_view(
                            &angles_dir,
                            slot.key,
                            slot.title,
                            "expression",
                            &img,
                        ));
                    }
                }
                // **一格失败不该让整批失败** —— 十张图里坏一张，
                // 其余九张仍然有用，把失败记下来继续
                Err(e) => warnings.push(format!("{} 生成失败：{e}", slot.title)),
            }
            // 失败那一格不加 —— 进度条要如实反映存下来了几张
            done = out.turnarounds.len() + out.angles.len() + out.expressions.len();
        }
    }

    // ── 动作：逐格生成 ────────────────────────────────────
    if want("poses") {
        let controlnet = sheet::find_controlnet();
        if controlnet.is_none() {
            warnings.push(
                "没找到 ControlNet（models/controlnet 下的 union/openpose），动作只靠提示词 —— \
                 奔跑/跃起这类大概率做不出来（实测出来是站着）"
                    .into(),
            );
        }
        let slots = sheet::POSES.iter().take(count_for(sheet::POSES.len()));
        for (i, slot) in slots.enumerate() {
            progress(done, total, slot.title);
            // 姿势参考图：自动部署内置标准 OpenPose 骨架模板
            let bare = slot.key.trim_start_matches("pose_");
            let pose_name = format!("pose_{bare}.png");
            let target_file = comfy_input_dir().join(&pose_name);
            if !target_file.is_file() {
                let bytes = match bare {
                    "standing" => Some(SKELETON_STANDING),
                    "running" => Some(SKELETON_RUNNING),
                    "jumping" => Some(SKELETON_JUMPING),
                    "weeping" | "collapse" => Some(SKELETON_WEEPING),
                    _ => None,
                };
                if let Some(b) = bytes {
                    let _ = std::fs::create_dir_all(comfy_input_dir());
                    let _ = std::fs::write(&target_file, b);
                }
            }
            let pose_ref = controlnet
                .as_ref()
                .and_then(|_| target_file.is_file().then_some(pose_name));
            let shot = sheet::pose(
                &ckpt_portrait,
                &trigger,
                &clothing,
                slot.prompt,
                reference_face_only.clone(),
                pose_ref,
                base_seed.wrapping_add(2_000 + i as i64),
                &format!("Anima_Pose_{}_{}", req.character_id, slot.key),
            );
            let shot = sheet::Shot {
                lora2: asian.clone(),
                ..shot
            };
            let shot = tune_character_sheet_shot(
                shot,
                &consistency_mode,
                character_lora.clone(),
                debug_one_slot,
            );
            match sheet::render_with_attempts(client, shot, 1, max_attempts).await {
                Ok(r) => {
                    warnings.extend(r.notes.clone());
                    if let Ok(img) = image::load_from_memory(&r.bytes) {
                        out.poses
                            .push(save_view(&angles_dir, slot.key, slot.title, "pose", &img));
                    }
                }
                Err(e) => warnings.push(format!("{} 生成失败：{e}", slot.title)),
            }
            done =
                out.turnarounds.len() + out.angles.len() + out.expressions.len() + out.poses.len();
        }
    }

    // **要了某一档却一张都没出来 —— 这必须说出来。**
    // 这是整个流程里最贵的失败：用户等了十分钟，界面上那一档是空的，
    // 而空白既可能是「没要求做」也可能是「做了全失败」。
    let empty: Vec<&str> = [
        ("turnaround", out.turnarounds.is_empty(), "全身三视图"),
        ("expressions", out.expressions.is_empty(), "表情模组"),
        ("poses", out.poses.is_empty(), "动态姿态"),
        ("angles", out.angles.is_empty(), "面部机位"),
    ]
    .into_iter()
    .filter(|(tier, is_empty, _)| want(tier) && *is_empty)
    .map(|(_, _, label)| label)
    .collect();
    for label in &empty {
        warnings.insert(
            0,
            format!("「{label}」这一档一张都没出来 —— 下面是每一格的原因"),
        );
    }

    progress(total, total, "全部完成");
    for w in &warnings {
        tracing::warn!(id = %req.character_id, "{w}");
    }
    out.warnings = warnings;

    // 自动保存生成的 asset_sheet 和 multi_angles 到角色档案中，防止刷新丢失
    save_character_sheet(
        config_dir,
        &req.project_id,
        &req.character_id,
        base_seed,
        &out,
    );

    Ok(out)
}

/// 存一格并组装成界面用的条目。
fn save_view(
    dir: &std::path::Path,
    key: &str,
    title: &str,
    category: &str,
    img: &image::DynamicImage,
) -> CharacterViewItem {
    use base64::Engine;
    let mut buf = std::io::Cursor::new(Vec::new());
    let _ = img.write_to(&mut buf, image::ImageFormat::Png);
    let png = buf.into_inner();
    let _ = std::fs::write(dir.join(format!("{key}.png")), &png);
    CharacterViewItem {
        key: key.to_string(),
        title: title.to_string(),
        image_url: format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&png)
        ),
        description: String::new(),
        category: category.to_string(),
    }
}

/// 从磁盘目录扫描并还原角色的全套资产（三视图、机位、表情、动作）。
pub fn discover_character_asset_sheet_from_disk(
    config_dir: &std::path::Path,
    project_id: &str,
    character_id: &str,
) -> Option<CharacterAssetSheet> {
    use base64::Engine;
    let angles_dir = cinema_projects_dir(config_dir)
        .join(project_id)
        .join("characters")
        .join(character_id)
        .join("angles");
    if !angles_dir.is_dir() {
        return None;
    }

    let load_view = |key: &str, title: &str, category: &str| -> Option<CharacterViewItem> {
        let p = angles_dir.join(format!("{key}.png"));
        if !p.is_file() {
            return None;
        }
        let bytes = std::fs::read(&p).ok()?;
        Some(CharacterViewItem {
            key: key.to_string(),
            title: title.to_string(),
            image_url: format!(
                "data:image/png;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(&bytes)
            ),
            description: String::new(),
            category: category.to_string(),
        })
    };

    let mut turnarounds = Vec::new();
    for &(key, title) in sheet::TURNAROUND_TITLES {
        if let Some(item) = load_view(key, title, "turnaround")
            .or_else(|| load_view(&format!("turnaround_{key}"), title, "turnaround"))
        {
            turnarounds.push(item);
        }
    }

    let mut angles = Vec::new();
    for &(key, title) in sheet::FACE_ANGLE_TITLES {
        if let Some(item) = load_view(key, title, "angle") {
            angles.push(CharacterAngle {
                angle_name: title.to_string(),
                image_url: item.image_url,
                description: String::new(),
            });
        }
    }

    let mut expressions = Vec::new();
    for slot in sheet::EXPRESSIONS {
        if let Some(item) = load_view(slot.key, slot.title, "expression") {
            expressions.push(item);
        }
    }

    let mut poses = Vec::new();
    for slot in sheet::POSES {
        if let Some(item) = load_view(slot.key, slot.title, "pose") {
            poses.push(item);
        }
    }

    if turnarounds.is_empty() && angles.is_empty() && expressions.is_empty() && poses.is_empty() {
        return None;
    }

    Some(CharacterAssetSheet {
        turnarounds,
        expressions,
        poses,
        angles,
        warnings: Vec::new(),
    })
}

/// 读角色档案里存的种子。
///
/// 读不到就返回 None（首次生成、或档案还没建）—— 调用方会随机一个并存下来。
fn load_character_seed(
    config_dir: &std::path::Path,
    project_id: &str,
    character_id: &str,
) -> Option<i64> {
    let file = cinema_projects_dir(config_dir)
        .join(project_id)
        .join("characters.json");
    let raw = std::fs::read_to_string(file).ok()?;
    let list: Vec<CinemaCharacter> = serde_json::from_str(&raw).ok()?;
    list.into_iter().find(|c| c.id == character_id)?.seed
}

/// 把种子写回角色档案。
fn save_character_seed(
    config_dir: &std::path::Path,
    project_id: &str,
    character_id: &str,
    seed: i64,
) {
    let file = cinema_projects_dir(config_dir)
        .join(project_id)
        .join("characters.json");
    let Ok(raw) = std::fs::read_to_string(&file) else {
        return;
    };
    let Ok(mut list) = serde_json::from_str::<Vec<CinemaCharacter>>(&raw) else {
        return;
    };
    let Some(c) = list.iter_mut().find(|c| c.id == character_id) else {
        return;
    };
    c.seed = Some(seed);
    if let Ok(text) = serde_json::to_string_pretty(&list) {
        let _ = std::fs::write(&file, text);
    }
}

/// 把生成的资产包和种子写回角色档案。
fn save_character_sheet(
    config_dir: &std::path::Path,
    project_id: &str,
    character_id: &str,
    seed: i64,
    sheet: &CharacterAssetSheet,
) {
    let file = cinema_projects_dir(config_dir)
        .join(project_id)
        .join("characters.json");
    let Ok(raw) = std::fs::read_to_string(&file) else {
        return;
    };
    let Ok(mut list) = serde_json::from_str::<Vec<CinemaCharacter>>(&raw) else {
        return;
    };
    let Some(c) = list.iter_mut().find(|c| c.id == character_id) else {
        return;
    };
    c.seed = Some(seed);
    c.asset_sheet = Some(sheet.clone());
    c.multi_angles = sheet.angles.clone();
    if let Ok(text) = serde_json::to_string_pretty(&list) {
        let _ = std::fs::write(&file, text);
    }
}

/// ComfyUI 的 input 目录。
///
/// **不写死绝对路径。** 原先硬编码成 `/Users/you/AI/ComfyUI/input`，
/// 换机器或换安装位置就静默失效 —— 参考图写不进去，锁脸悄悄不生效，
/// 而生成本身照样成功，没有任何报错。
pub(crate) fn comfy_input_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("ANIMA_COMFY_INPUT") {
        return PathBuf::from(dir);
    }
    let home = std::env::var("HOME").unwrap_or_default();
    for candidate in [
        format!("{home}/AI/ComfyUI/input"),
        format!("{home}/ComfyUI/input"),
        format!("{home}/Documents/ComfyUI/input"),
    ] {
        let p = PathBuf::from(&candidate);
        if p.is_dir() {
            return p;
        }
    }
    PathBuf::from(format!("{home}/AI/ComfyUI/input"))
}

/// 从分集里读出这一镜的编号和提示词，给任务登记用。
///
/// **读不到也要能登记。** 分集文件坏了、镜头刚被删了 —— 这些情况下
/// 渲染本来就会失败，而失败恰恰是最需要在队列里留一条的时候。
fn shot_brief(config_dir: &Path, req: &RenderShotReq) -> (u32, String) {
    let f = cinema_projects_dir(config_dir)
        .join(&req.project_id)
        .join("episodes")
        .join(&req.episode_id)
        .join("episode.json");
    let Ok(ep) = std::fs::read_to_string(&f)
        .ok()
        .ok_or(())
        .and_then(|t| serde_json::from_str::<EpisodeData>(&t).map_err(|_| ()))
    else {
        return (0, String::new());
    };
    ep.storyboard
        .iter()
        .find(|s| s.id == req.shot_id)
        .map(|s| (s.shot_num, s.prompt.clone()))
        .unwrap_or((0, String::new()))
}

/// 渲染单个影视分镜（通过 ComfyUI 发送专属 Prompt 与底模，生成独立关键帧及动态镜头视频）
#[tauri::command]
pub async fn cinema_render_shot(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    req: RenderShotReq,
) -> Result<RenderShotResult> {
    // **登记在命令层，不在 `_with_deps` 里。** 那个函数在冒烟测试和
    // agent 工具里也会被调，那些场景不该往用户的任务队列里塞东西。
    let (num, prompt) = shot_brief(&state.config_dir, &req);
    let ids = (req.project_id.clone(), req.episode_id.clone(), req.shot_id.clone());
    let job = crate::jobs::start(
        &state.config_dir,
        crate::jobs::JobKind::Frame,
        &req.project_id,
        &req.episode_id,
        &req.shot_id,
        num,
        &prompt,
    );
    crate::jobs::emit(Some(&app), crate::jobs::get(&state.config_dir, &job).as_ref());
    // 在这条任务名下跑：交给 ComfyUI 时记下 prompt_id（应用中途关掉，重开时凭它收图，见 `render_resume`）
    let out = crate::jobs::scoped(&state.config_dir, &job, cinema_render_shot_with_deps(&state.config_dir, state.http.clone(), req)).await;
    // 成功和失败都收尾 —— 只在成功路径上收的话，失败的任务会永远
    // 停在「正在跑」，队列上的数字再也不归零。
    let done = match &out {
        Ok(r) => crate::jobs::finish(&state.config_dir, &job, Some(&r.image_url), None),
        Err(e) => crate::jobs::finish(&state.config_dir, &job, None, Some(&e.to_string())),
    };
    crate::jobs::emit(Some(&app), done.as_ref());
    if out.is_ok() {
        // 有了首帧 = 链路换成首帧出片，提示词要按首帧的写法重合成（自动排进合成队列）
        resynth_after_route_change(&app, &state, &ids.0, &ids.1, &ids.2);
    }
    out
}

/// **首帧加上 / 摘掉之后，这一镜的提示词按新链路自动重合成**（排进合成队列）。
///
/// 首帧出片（三字段）和参考出片（六段）写法完全不同，换了链路旧提示词就过期、批量出片会跳过。
/// 用户 2026-10-07：「必须根据当前链路进行提示词处理」。只合**出片线路上合成过的**（从没合成过的等用户自己合，
/// 和「按场出首帧」自动补合的规则一致，见 [`frame_resynth_targets`]）。失败只记日志、不挡这一下操作。
fn resynth_after_route_change(app: &tauri::AppHandle, state: &State<'_, AppState>, project_id: &str, episode_id: &str, shot_id: &str) {
    let route = project_route(&state.config_dir, project_id);
    let synthesized = read_episode(&state.config_dir, project_id, episode_id)
        .ok()
        .and_then(|ep| ep.storyboard.into_iter().find(|s| s.id == shot_id))
        .is_some_and(|s| s.prompts.get(&route).is_some_and(|p| !p.video.trim().is_empty()));
    if !synthesized {
        return;
    }
    if let Err(e) = queue_synthesize(Some(app.clone()), state.config_dir.clone(), state.gateway.clone(), project_id.to_string(), episode_id.to_string(), vec![shot_id.to_string()], Some(route)) {
        tracing::warn!(shot_id, "换了出片链路，自动重合成没能开始：{e}");
    }
}

/// 批量出首帧（「按场出首帧」那个按钮）。
///
/// `redo = false` 时已经有首帧的镜头跳过 —— 导入的首帧、挑过的首帧不能被一键盖掉。
/// 每一镜走和单镜「出首帧」同一个函数（资产图怎么挑、画幅怎么裁都是同一份）。
#[tauri::command]
pub fn cinema_render_episode_frames(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    project_id: String,
    episode_id: String,
    shot_ids: Vec<String>,
    redo: bool,
) -> Result<crate::jobs::Job> {
    let config_dir = state.config_dir.clone();
    let ep = read_episode(&config_dir, &project_id, &episode_id)?;
    let shots_dir = episode_file(&config_dir, &project_id, &episode_id).with_file_name("shots");
    let mut ordered: Vec<&StoryboardShot> = ep.storyboard.iter().filter(|s| shot_ids.contains(&s.id)).collect();
    ordered.sort_by_key(|s| s.shot_num);
    // **按内容该走参考出片的不出首帧**（有人中途进场 / 人多到首帧挂不下）—— 出了也会被拦，或者把人画丢（`shot_route`）
    let assets = crate::storyboard_gen::load_assets(&config_dir, &project_id, &episode_id);
    let (ordered, by_ref): (Vec<&StoryboardShot>, Vec<&StoryboardShot>) = ordered.into_iter().partition(|s| {
        let long_take = ep.long_take_scenes.contains(&s.scene_no);
        !long_take && crate::shot_route::content_route(s, &assets).route == crate::shot_route::ShotRoute::Keyframe
    });
    let (targets, kept): (Vec<&StoryboardShot>, Vec<&StoryboardShot>) =
        ordered.into_iter().partition(|s| redo || !Keyframes::on_disk(&shots_dir, &s.id).first);
    let ref_note = (!by_ref.is_empty()).then(|| {
        let nums: Vec<String> = by_ref.iter().map(|s| s.shot_num.to_string()).collect();
        format!("第 {} 镜走参考出片（中途有人进场、人太多、首帧里拍不到脸，或者手动选了参考），没出首帧", nums.join("、"))
    });
    if targets.is_empty() {
        let why = if !kept.is_empty() {
            format!("选中的 {} 镜都已经有首帧了 —— 要重出就勾「已有的也重出」", kept.len())
        } else {
            ref_note.clone().unwrap_or_else(|| "没有要出首帧的镜头".to_string())
        };
        return Err(Error::config(why));
    }
    let ids: Vec<String> = targets.iter().map(|s| s.id.clone()).collect();
    let kept_note = [(!kept.is_empty()).then(|| format!("{} 镜已经有首帧，没动", kept.len())), ref_note].into_iter().flatten().collect::<Vec<_>>().join("；");
    let kept_note = (!kept_note.is_empty()).then_some(kept_note);
    start_frame_batch(app, config_dir, state.http.clone(), state.gateway.clone(), project_id, episode_id, ids, kept_note, Vec::new(), false)
}

/// 「按场出首帧」的本体。按钮和重启后续接（`render_resume`）共用。
///
/// - `also_framed`：这一批之外、但也刚出了首帧要一起补合成的镜头（重启前正在出、重启后续接收回来的那一张）；
/// - `wait_comfy`：开始前先等 ComfyUI 连得上（重启后续接时它可能还没起来，不等的话一镜接一镜秒败）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn start_frame_batch(
    app: tauri::AppHandle,
    config_dir: PathBuf,
    http: reqwest::Client,
    gateway: std::sync::Arc<crate::Gateway>,
    project_id: String,
    episode_id: String,
    ids: Vec<String>,
    kept_note: Option<String>,
    also_framed: Vec<String>,
    wait_comfy: bool,
) -> Result<crate::jobs::Job> {
    start_frame_batch_with(app, config_dir, http, gateway, project_id, episode_id, ids, kept_note, also_framed, wait_comfy, Vec::new())
}

/// `pending_resume`：这几镜重启前正在出、重启后另有任务在续接收图 —— 排在最后，轮到时**已经有首帧了就跳过**
/// （续接收回来了），续接的任务还在等就先等它，没收回来（ComfyUI 那边丢了）才重出。
#[allow(clippy::too_many_arguments)]
pub(crate) fn start_frame_batch_with(
    app: tauri::AppHandle,
    config_dir: PathBuf,
    http: reqwest::Client,
    gateway: std::sync::Arc<crate::Gateway>,
    project_id: String,
    episode_id: String,
    ids: Vec<String>,
    kept_note: Option<String>,
    also_framed: Vec<String>,
    wait_comfy: bool,
    pending_resume: Vec<String>,
) -> Result<crate::jobs::Job> {
    let title = format!("出首帧 · {} 镜", ids.len());
    let job_id = crate::jobs::start_exclusive(&config_dir, crate::jobs::JobKind::FrameBatch, &project_id, &episode_id, "", 0, &title)
        .map_err(|_| Error::config("这一集已经在批量出首帧了 —— 进度在右下角和 QUEUE 里"))?;
    let started = crate::jobs::set_targets(&config_dir, &job_id, &ids).ok_or_else(|| Error::runtime("任务登记后读不回来"))?;
    crate::jobs::emit(Some(&app), Some(&started));
    tauri::async_runtime::spawn(async move {
        let total = ids.len();
        if wait_comfy {
            let since = std::time::Instant::now();
            let mut said = false;
            while !crate::agent_runtime::video_tool::comfy_reachable().await {
                if crate::jobs::cancel_requested(&config_dir, &job_id) || since.elapsed() > std::time::Duration::from_secs(30 * 60) {
                    let done = crate::jobs::finish_with(&config_dir, &job_id, None, None, &[], Some(&format!(
                        "等了 30 分钟 ComfyUI 还连不上（或者停了），这 {total} 镜首帧没有出 —— 先把 ComfyUI 开起来（anima start），再点一次按场出首帧"
                    )));
                    crate::jobs::emit(Some(&app), done.as_ref());
                    return;
                }
                if !said {
                    let job = crate::jobs::set_progress(&config_dir, &job_id, 0, total, "排队中 · 等 ComfyUI 起来");
                    crate::jobs::emit(Some(&app), job.as_ref());
                    said = true;
                }
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
        }
        let (mut done, mut notes) = (0, Vec::new());
        // 这一轮真的出了首帧的镜头 —— 跑完按首帧补合成它们
        let mut framed: Vec<String> = also_framed;
        for (i, id) in ids.iter().enumerate() {
            if crate::jobs::cancel_requested(&config_dir, &job_id) {
                notes.push(format!("已停止，后面 {} 镜没有出", total - i));
                break;
            }
            let req = RenderShotReq { project_id: project_id.clone(), episode_id: episode_id.clone(), shot_id: id.clone(), engine_config: None };
            let (num, prompt) = shot_brief(&config_dir, &req);
            let job = crate::jobs::set_progress(&config_dir, &job_id, i, total, &format!("第 {num} 镜"));
            crate::jobs::emit(Some(&app), job.as_ref());
            if pending_resume.contains(id) {
                // 续接收图的那条还在等 ComfyUI：等它（最多 6 小时），别重复出
                let since = std::time::Instant::now();
                while crate::jobs::running(&config_dir, &project_id, &episode_id).iter().any(|j| j.kind == crate::jobs::JobKind::Frame && &j.shot_id == id)
                    && since.elapsed() < std::time::Duration::from_secs(6 * 3600)
                {
                    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                }
                let shots_dir = episode_file(&config_dir, &project_id, &episode_id).with_file_name("shots");
                if Keyframes::on_disk(&shots_dir, id).first {
                    done += 1;
                    framed.push(id.clone());
                    continue;
                }
            }
            let one = crate::jobs::start(&config_dir, crate::jobs::JobKind::Frame, &project_id, &episode_id, id, num, &prompt);
            crate::jobs::emit(Some(&app), crate::jobs::get(&config_dir, &one).as_ref());
            let out = crate::jobs::scoped(&config_dir, &one, cinema_render_shot_with_deps(&config_dir, http.clone(), req)).await;
            let fin = match &out {
                Ok(r) => { done += 1; framed.push(id.clone()); crate::jobs::finish(&config_dir, &one, Some(&r.image_url), None) }
                Err(e) => { notes.push(format!("第 {num} 镜：{e}")); crate::jobs::finish(&config_dir, &one, None, Some(&e.to_string())) }
            };
            crate::jobs::emit(Some(&app), fin.as_ref());
        }
        let summary = [Some(format!("出首帧 {done}/{total} 镜")), kept_note].into_iter().flatten().collect::<Vec<_>>().join("；");
        // **有了首帧，出片线路的提示词要按首帧重写** —— 接着自动补合成，不用人再去点。
        // 用户 2026-09-29：「生成了提示词，然后按场出首帧，然后提示词又还再生成」—— 这一步原来要人自己发现、自己点
        let mut remarks = Vec::new();
        let resynth = frame_resynth_targets(&config_dir, &project_id, &episode_id, &framed);
        if !resynth.is_empty() {
            let nums = shot_nums_label(&config_dir, &project_id, &episode_id, &resynth);
            match start_synthesize(Some(app.clone()), config_dir.clone(), gateway, project_id.clone(), episode_id.clone(), resynth.clone(), None) {
                Ok(_) => remarks.push(format!("有了首帧，已经接着按首帧重新合成{nums}（{} 镜）", resynth.len())),
                Err(e) => remarks.push(format!("有了首帧的{nums}要按首帧重新合成，没能自动开始（{e}）—— 在合成弹窗里点一下")),
            }
        }
        let finished = if done == 0 {
            crate::jobs::finish_with(&config_dir, &job_id, None, None, &notes, Some(&format!("{total} 镜一镜都没出成：{}", notes.join("；"))))
        } else {
            crate::jobs::finish_with_remarks(&config_dir, &job_id, None, Some(&summary), &notes, &remarks, None)
        };
        crate::jobs::emit(Some(&app), finished.as_ref());
    });
    Ok(started)
}

/// [`frame_resynth_targets`] 给续接收回首帧那条路用（`render_resume`）
pub(crate) fn frame_resynth_targets_pub(config_dir: &Path, project_id: &str, episode_id: &str, framed: &[String]) -> Vec<String> {
    frame_resynth_targets(config_dir, project_id, episode_id, framed)
}

/// 一镜的出片链路信息（界面上显示、批量分组用）。见 [`crate::shot_route`]
#[derive(Debug, Clone, Serialize)]
pub struct ShotRouteInfo {
    pub shot_id: String,
    pub shot_num: u32,
    /// 实际走哪条（有首帧 = 首帧出片）
    pub actual: crate::shot_route::ShotRoute,
    /// 按分镜内容建议走哪条
    pub advised: crate::shot_route::ShotRoute,
    pub reason: String,
    /// 首帧出片却有人中途进场：拦下的原因
    pub blocker: Option<String>,
    /// 「接上一镜」按规则该是什么（手动改过的界面上可以「恢复自动」到它）
    pub auto_continuity: Continuity,
}

pub(crate) fn shot_routes(config_dir: &Path, project_id: &str, episode_id: &str) -> Result<Vec<ShotRouteInfo>> {
    let mut ep = read_episode(config_dir, project_id, episode_id)?;
    sync_keyframes(&episode_file(config_dir, project_id, episode_id).with_file_name("shots"), &mut ep);
    let assets = crate::storyboard_gen::load_assets(config_dir, project_id, episode_id);
    let mut shots: Vec<&StoryboardShot> = ep.storyboard.iter().collect();
    shots.sort_by_key(|s| s.shot_num);
    Ok(shots
        .iter()
        .enumerate()
        .map(|(k, s)| {
            let long_take = ep.long_take_scenes.contains(&s.scene_no);
            let advice = crate::shot_route::content_route(s, &assets);
            let prev = (k > 0).then(|| shots[k - 1]);
            let planned = crate::shot_route::planned_route(s, &assets, long_take);
            ShotRouteInfo {
                shot_id: s.id.clone(),
                shot_num: s.shot_num,
                actual: crate::shot_route::actual_route(s, long_take),
                advised: if long_take { crate::shot_route::ShotRoute::Reference } else { advice.route },
                reason: if long_take { "这一场整段出（长镜头），窗口按参考图出".into() } else { advice.reason },
                blocker: crate::shot_route::keyframe_blocker(s, &assets, long_take),
                auto_continuity: crate::shot_route::auto_continuity(s, prev, planned, long_take),
            }
        })
        .collect())
}

#[tauri::command]
pub fn cinema_shot_routes(state: State<'_, AppState>, project_id: String, episode_id: String) -> Result<Vec<ShotRouteInfo>> {
    shot_routes(&state.config_dir, &project_id, &episode_id)
}

/// **按规则把这一集的「接上一镜」设好**（手动改过的不动，见 `shot_route::apply_auto_continuity`）。回改了哪几镜。
///
/// 在三个时机调：生成完镜头表、首帧加上 / 摘掉（链路变了）、合成提示词之前（合成按当前的接法写 `<Video 1>`）。
/// 用户 2026-10-07：「需要在分镜后自动处理接上一镜的选项，不能光写推荐」。
pub(crate) fn auto_continuity_now(config_dir: &Path, project_id: &str, episode_id: &str) -> Vec<(u32, Continuity)> {
    let ep_file = cinema_projects_dir(config_dir).join(project_id).join("episodes").join(episode_id).join("episode.json");
    let assets = crate::storyboard_gen::load_assets(config_dir, project_id, episode_id);
    let mut changed = Vec::new();
    let r = crate::episode_lock::update_file(&ep_file, |ep| {
        sync_keyframes(&ep_file.with_file_name("shots"), ep);
        changed = crate::shot_route::apply_auto_continuity(ep, &assets);
        Ok(())
    });
    if let Err(e) = r {
        tracing::warn!("自动设接上一镜失败：{e}");
    }
    if !changed.is_empty() {
        tracing::info!(?changed, "接上一镜按规则自动设了");
    }
    changed
}

/// 出完首帧之后要补合成哪几镜：**出片线路**上合成过、现在过期了的（有了首帧，写法要变），
/// 和**出了首帧、还从没合成过的**（2026-10-03 起出首帧不用先合成，这是它唯一的一次合成）。
/// 没出成首帧的不补；另一条线路也不补，出片不用它。
fn frame_resynth_targets(config_dir: &Path, project_id: &str, episode_id: &str, framed: &[String]) -> Vec<String> {
    let Ok(mut ep) = read_episode(config_dir, project_id, episode_id) else { return Vec::new() };
    let ep_file = cinema_projects_dir(config_dir).join(project_id).join("episodes").join(episode_id).join("episode.json");
    sync_keyframes(&ep_file.with_file_name("shots"), &mut ep);
    let route = project_route(config_dir, project_id);
    let assets = crate::storyboard_gen::load_assets(config_dir, project_id, episode_id);
    let style = episode_style(config_dir, project_id, &ep);
    ep.storyboard
        .iter()
        .filter(|s| framed.contains(&s.id))
        // 过期的要重合；**从没合成过的也要合**（2026-10-03 起出首帧不用先合成 —— 见 `first_frame_source`）。
        // `prompt_stale` 对「没合成过」回 false（不乱报），只靠它的话，先出首帧的镜头出完就停在没有提示词上
        .filter(|s| {
            let never = s.prompts.get(&route).map_or(true, |p| p.video.trim().is_empty());
            (never && s.keyframes.first) || prompt_stale(&ep.storyboard, s, &assets, &style, route, &ep.long_take_scenes)
        })
        .map(|s| s.id.clone())
        .collect()
}

/// 「第 3、8、16 镜」
fn shot_nums_label(config_dir: &Path, project_id: &str, episode_id: &str, ids: &[String]) -> String {
    let Ok(ep) = read_episode(config_dir, project_id, episode_id) else { return String::new() };
    let mut nums: Vec<u32> = ep.storyboard.iter().filter(|s| ids.contains(&s.id)).map(|s| s.shot_num).collect();
    nums.sort();
    format!("第 {} 镜", nums.iter().map(|n| n.to_string()).collect::<Vec<_>>().join("、"))
}

/// 首帧最多挂几张参考图。
///
/// 用户 2026-09-29 定的：「首帧可以最多 4 张。主要就是场景和角色」。Qwen 2.1 有 10 个槽，但参考图越多越慢、而且不是线性的 ——
/// 同一晚第 8 镜挂 8 张，一步 85~96 秒、一张约 37 分钟（不挂图约 5 分钟），按场出六张就是几个小时。
/// 顺序还是 [`frame_reference_pick`] 那套：场景 → 说话的 → 有站位的 → 其余人 → 道具，**道具只在场景和角色没占满时才进**。
/// 出片挂的参考图不受这个限制（H3 那边另算）。
pub const FIRST_FRAME_MAX_REFS: usize = 4;

/// 出首帧实际能挂几张：引擎的槽位数和 [`FIRST_FRAME_MAX_REFS`] 取小。出图、备注、界面提示都读它，三处说法一致
fn first_frame_slots(engine: &str) -> usize {
    reference_slots_of(engine).min(FIRST_FRAME_MAX_REFS)
}

/// 这个出图引擎有几个参考图槽。0 = 文生图那一类，首帧不用资产的图。
fn reference_slots_of(engine: &str) -> usize {
    crate::agent_runtime::image_engine::ImageEngine::from_id(engine)
        .map(|e| e.reference_slots() as usize)
        .unwrap_or(0)
}

/// 参考生图首帧挑了哪几张图。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct FrameRefPick {
    /// 按槽位顺序：场景在前、人物在后。
    pub images: Vec<String>,
    /// 这些图发出去之后，每一张是谁。给界面和提示词开头用。
    pub names: Vec<String>,
    /// 有图但槽位满了没用上的资产名。**必须说出来** —— 不说用户会以为锁住了。
    pub unused: Vec<String>,
}

/// 一镜的资产图怎么排进参考图槽。
///
/// # 顺序为什么是这样
///
/// 1. **场景占第一个槽。** Krea2Edit 和 Qwen-Image-Edit 实测都是「场景在前、
///    人物在后」跑出来的；Krea2Edit 的 `ref_boost` 只作用在最后一张，
///    场景放后面会被当成要锁身份的那个人。
/// 2. 人物：**有台词的 → 有站位的 → 其余**，同级按提示词里出现的顺序。
///    槽不够时，说话的人最不能丢 —— 出片时声音要绑到他脸上。
/// 3. **道具排在最后**，槽够就进。以前道具一律不进槽 —— E01 第 1 镜的激光刀
///    于是被模型画成了机械臂上的东西。Qwen 2.1 参考生图有 10 个槽，同题实测
///    （CHANGELOG 2026-09-22）第 4 张道具图挂上之后快递单和罐头才对。
///
/// 「参考图 N 是谁」要写进合成好的提示词里（实测跑通的工作流都明写了
/// 「the woman from image 2」）—— 那是合成那一步的事，这里只挑图、排顺序。
pub(crate) fn frame_reference_pick(
    refs: &[crate::assets::ResolvedRef],
    dialogue: &[DialogueLine],
    blocking: &[StageSubject],
    slots: usize,
) -> FrameRefPick {
    use crate::assets::AssetKind;
    if slots == 0 {
        return FrameRefPick::default();
    }
    let imaged = |k: AssetKind| refs.iter().filter(move |r| r.kind == k && r.asset_id.is_some() && r.image_path.is_some());
    let same = |a: &str, b: &str| a.trim() == b.trim() || base_character_name(a) == base_character_name(b);
    let rank = |name: &str| {
        if dialogue.iter().any(|d| !d.off_screen && !d.line.trim().is_empty() && same(&d.speaker, name)) {
            0
        } else if blocking.iter().any(|b| same(&b.actor, name)) {
            1
        } else {
            2
        }
    };
    let mut ordered: Vec<&crate::assets::ResolvedRef> = imaged(AssetKind::Scene).collect();
    let mut people: Vec<&crate::assets::ResolvedRef> = imaged(AssetKind::Character).collect();
    people.sort_by_key(|r| rank(&r.name)); // 稳定排序：同级保持出现顺序
    // 只要一个场景 —— 两张场景图同时进槽，模型不知道人该站在哪一张里。
    ordered.truncate(1);
    let extra_scenes: Vec<String> = imaged(AssetKind::Scene).skip(1).map(|r| r.name.clone()).collect();
    ordered.extend(people);
    ordered.extend(imaged(AssetKind::Prop));

    let mut pick = FrameRefPick::default();
    let mut seen: Vec<&str> = Vec::new();
    for r in ordered {
        let path = r.image_path.as_deref().unwrap_or_default();
        if seen.contains(&path) {
            continue;
        }
        seen.push(path);
        if pick.images.len() < slots {
            pick.images.push(path.to_string());
            pick.names.push(r.name.clone());
        } else {
            pick.unused.push(r.name.clone());
        }
    }
    pick.unused.extend(extra_scenes);
    pick
}

/// 出首帧挂哪些资产图：**按镜头表的字段取**（场景、出场人物、道具），带这一镜的形象覆盖。
///
/// 以前是从中文分镜描述里认 `[角色:xx]` 这种写法 —— 新的镜头表不写这种标记，
/// 于是一张都认不出来，首帧等于纯文生，场景和道具全靠模型编。
fn shot_frame_refs(config_dir: &Path, project_id: &str, episode_id: &str, shot: &StoryboardShot) -> Vec<crate::assets::ResolvedRef> {
    use crate::assets::AssetKind;
    let assets = crate::looks::for_shot(&crate::storyboard_gen::load_assets(config_dir, project_id, episode_id), shot);
    let mut ids: Vec<String> = shot.scene_id.clone().into_iter().collect();
    ids.extend(shot.all_character_ids());
    ids.extend(shot.prop_ids.iter().cloned());
    // 槽位里排了、字段里漏了的也算
    ids.extend(shot.ref_slots.iter().map(|s| s.asset_id.clone()));
    // 老数据的中文描述里还有 `[角色:xx]` 标记，照认
    let mut out: Vec<crate::assets::ResolvedRef> = crate::assets::resolve_refs_in(config_dir, &shot.prompt, &crate::assets::RefScope::Project { project_id, episode_id: None });
    for id in ids {
        let Some(a) = assets.iter().find(|a| a.id == id) else { continue };
        if out.iter().any(|r| r.asset_id.as_deref() == Some(a.id.as_str())) || !matches!(a.kind, AssetKind::Scene | AssetKind::Character | AssetKind::Prop) {
            continue;
        }
        // 开场不在画面里的人不挂图：挂上了模型就会把他画进首帧（EP02 第 11 镜陆川中途才冲出来）
        if a.kind == AssetKind::Character && shot.absent_from_opening(&a.name).is_some() {
            continue;
        }
        let path = asset_image_path(config_dir, project_id, episode_id, &a.id, &a.look_id).map(|p| p.to_string_lossy().to_string());
        out.push(crate::assets::ResolvedRef {
            kind: a.kind, name: a.name.clone(), asset_id: Some(a.id.clone()),
            description: String::new(), anchors: String::new(), avoid: String::new(), lock: String::new(),
            image_url: path.clone(), image_path: path,
        });
    }
    // 360 全景的场景图先转成正常透视（见 `flat_scene_ref`）
    for r in out.iter_mut().filter(|r| r.kind == AssetKind::Scene) {
        if let Some(p) = r.image_path.take() {
            let flat = flat_scene_ref(config_dir, Path::new(&p)).to_string_lossy().to_string();
            r.image_url = Some(flat.clone());
            r.image_path = Some(flat);
        }
    }
    out
}

/// 参考图说明：第 N 张是谁、管什么。
fn frame_ref_legend(refs: &[crate::assets::ResolvedRef], pick: &FrameRefPick) -> String {
    use crate::assets::AssetKind;
    let lines: Vec<String> = pick.names.iter().enumerate().map(|(i, name)| {
        let n = i + 1;
        match refs.iter().find(|r| &r.name == name).map(|r| r.kind) {
            Some(AssetKind::Scene) => format!("Image {n} is the location「{name}」: keep its layout, materials, props in place and lighting; it is only the setting, not a person."),
            Some(AssetKind::Prop) => format!("Image {n} is the object「{name}」: keep its exact shape, colours, wear and markings wherever it appears."),
            _ => format!("Image {n} is the person「{name}」: keep this exact face, hair, body and costume; ignore the pose, framing and background of that image."),
        }
    }).collect();
    format!("Reference images:\n{}\nDraw one single new film frame (not a collage, no panels, no text) combining them as described below.", lines.join("\n"))
}

/// 出首帧发给出图模型的画面提示词。
///
/// **当前线路合成过就原样用合成好的那份**（用户可能在详情里手改过）。
///
/// **没合成过，就按这一镜现在的字段拼一份线路 A 的画面提示词** —— 线路 A 的合成是纯代码拼接，
/// 不调模型、不花 token（[`crate::prompt_route_a::synthesize`]）。
///
/// # 为什么（2026-10-03 用户：「这不是犯傻吗」）
///
/// 原来出首帧必须先合成提示词，而挂上首帧之后提示词又判过期、要按首帧模式再合一遍 ——
/// 第一轮合成（EP02 线路 B 147 次 gpt-5.6-sol 调用里的一大块）出完首帧就作废了。
/// 出首帧要的只是「开场那一刻谁在哪、什么景别、什么光」，镜头表里都有；出首帧的 Qwen 2.1 读中文长提示词本来就好。
/// 现在的顺序：镜头表 → 细化时间线 → 按场出首帧（不用先合成）→ 合成提示词（只合一次，按场出首帧跑完自动补合）。
///
/// **不拿存着的线路 A 结果凑**：那份可能是改字段之前合的（过期了还拿来出图，画面和镜头对不上）。
/// 这里每次按当前字段现拼。
/// 出首帧用哪份画面提示词，以及（线路 B 那份）里面 `<Subject N>` 各是谁。
///
/// 1. **手写的**（`video` 空、`frame` 有字 —— 没合成过时在单镜里填的）：原样用。
/// 2. **线路 B 按「还没有首帧」写的、合成后字段没改过**：用它，标签按那时的编号换成名字。
/// 3. 其余一律按镜头表现拼线路 A（纯代码、不花 token）：
///    - **按首帧写的**（新顺序下的常态：先出首帧再合成）—— 正文是「和 <Picture 1> 完全一致」，
///      故意不写长相、只按位置认人。重出首帧时 Image 1 是场景（导演世界里是人偶稿），等于叫模型照着场景画人；
///    - **合成后改过字段的** —— 编号可能已经对不上，换出来的名字会张冠李戴，比不换还糟；
///    - 线路 A 存着的那份同理（可能过期），每次现拼。
///
/// 2026-10-03 审查查出 1、2 两类问题（EP01 第 1/3/8/16/25 镜、EP02 第 6/13 镜存着按首帧写的画面提示词）。
pub(crate) fn first_frame_source(config_dir: &Path, project: &CinemaProject, episode_id: &str, shot: &StoryboardShot) -> (String, Vec<(u32, String, String)>) {
    let route = project.model_settings.prompt_route;
    if let Some(p) = shot.prompts.get(&route) {
        if p.video.trim().is_empty() && !p.frame.trim().is_empty() {
            return (p.frame.trim().to_string(), Vec::new());
        }
        if route == crate::shot_table::PromptRoute::B && !p.frame.trim().is_empty() {
            if let Some(subjects) = frame_subjects(config_dir, &project.id, episode_id, shot, route) {
                return (p.frame.trim().to_string(), subjects);
            }
        }
    }
    (compose_route_a_frame(config_dir, project, episode_id, shot), Vec::new())
}

fn compose_route_a_frame(config_dir: &Path, project: &CinemaProject, episode_id: &str, shot: &StoryboardShot) -> String {
    let ep = read_episode(config_dir, &project.id, episode_id).ok();
    // 按这一镜的形象覆盖换过（换了衣服的镜头描述也跟着换），和合成时一样
    let assets = crate::looks::for_shot(&crate::storyboard_gen::load_assets(config_dir, &project.id, episode_id), shot);
    let style = ep.as_ref().map(|e| episode_style(config_dir, &project.id, e)).unwrap_or_default();
    let prev = ep.as_ref().and_then(|e| e.storyboard.iter().filter(|s| s.shot_num < shot.shot_num).max_by_key(|s| s.shot_num));
    let long_take = ep.as_ref().is_some_and(|e| e.long_take_scenes.contains(&shot.scene_no));
    let ctx = crate::prompt_input::ShotContext { shot, prev, assets: &assets, style: &style, long_take };
    crate::prompt_route_a::synthesize(&ctx, 0).frame.trim().to_string()
}

/// 线路 B 画面提示词能不能拿来出首帧；能的话，里面每个 `<Subject N>` 是谁（见 [`resolve_subject_tags`]）。
///
/// **只认「合成时还没有首帧」、而且合成后字段没改过的那份**：拿存着的指纹对「没有首帧」的两种关键帧组合，
/// 对上了就按那时的编号。对不上（按首帧写的、改过字段的、老数据没指纹的）回 `None`，调用方改为现拼线路 A ——
/// 猜一个编号去换，换错了是把打手乙的名字安到铁头头上，比不换还糟。
pub(crate) fn frame_subjects(config_dir: &Path, project_id: &str, episode_id: &str, shot: &StoryboardShot, route: crate::shot_table::PromptRoute) -> Option<Vec<(u32, String, String)>> {
    let ep = read_episode(config_dir, project_id, episode_id).ok()?;
    let raw = crate::storyboard_gen::load_assets(config_dir, project_id, episode_id);
    let style = episode_style(config_dir, project_id, &ep);
    let stored = shot.prompts.get(&route).map(|p| p.fields_hash.clone()).filter(|h| !h.is_empty())?;
    let then = [Keyframes { first: false, last: false }, Keyframes { first: false, last: true }]
        .into_iter()
        .find(|&kf| {
            let mut s = shot.clone();
            s.keyframes = kf;
            stored == current_hash(&ep.storyboard, &s, &raw, &style, &ep.long_take_scenes)
        })?;
    let mut s = shot.clone();
    s.keyframes = then;
    let assets = crate::looks::for_shot(&raw, &s);
    let ctx = crate::prompt_input::ShotContext {
        shot: &s, prev: prev_of(&ep.storyboard, &s), assets: &assets, style: &style,
        long_take: ep.long_take_scenes.contains(&s.scene_no),
    };
    Some(crate::prompt_route_b::subject_names(&ctx))
}

/// 把线路 B 正文里的 `<Subject N>` 换成「名字」，出图模型才知道说的是谁。
///
/// # 为什么（2026-10-03 EP02 首帧一半废片）
///
/// `<Subject N>` 是 H3 的写法，编号是合成时的槽位顺序；出首帧挂的参考图另有一套编号
/// （`Image 1` 是场景、人物从 2 起），而且正文一个名字都没有。Qwen 2.1 只能把 Subject N 往 Image N 上猜：
/// 第 17 镜「<Subject 1> 站在画面中间」，Image 1 是场景，画出来的是铁头站中间、陆川没了；
/// 第 12、15、18 镜同一个人画了两遍。EP02 28 镜里 27 镜的首帧提示词都带这种标签。
///
/// 第一次出现时补上对应关系：有参考图的写「（Image k）」，没挂上图的（群演多、槽位满了）写资产描述 ——
/// 只给一个名字，模型不知道「打手乙」长什么样。认不出的编号原样留着。
pub(crate) fn resolve_subject_tags(prompt: &str, subjects: &[(u32, String, String)], pictured: &[(String, usize)]) -> String {
    const OPEN: &str = "<Subject ";
    let mut out = String::with_capacity(prompt.len());
    let mut seen: Vec<u32> = Vec::new();
    let mut rest = prompt;
    while let Some(at) = rest.find(OPEN) {
        out.push_str(&rest[..at]);
        let after = &rest[at + OPEN.len()..];
        let digits = after.find(|c: char| !c.is_ascii_digit()).unwrap_or(after.len());
        let known = (digits > 0 && after[digits..].starts_with('>'))
            .then(|| after[..digits].parse::<u32>().ok())
            .flatten()
            .and_then(|n| subjects.iter().find(|(m, ..)| *m == n));
        let Some((n, name, description)) = known else {
            out.push_str(OPEN);
            rest = after;
            continue;
        };
        out.push_str(&format!("「{name}」"));
        if !seen.contains(n) {
            seen.push(*n);
            match pictured.iter().find(|(p, _)| p == name) {
                Some((_, k)) => out.push_str(&format!(" (Image {k})")),
                None if !description.is_empty() => out.push_str(&format!(" ({description})")),
                None => {}
            }
        }
        rest = &after[digits + 1..];
    }
    out.push_str(rest);
    out
}

/// 出首帧发给出图模型的正文（不含参考图图例）：选哪份（[`first_frame_source`]）→ 标签换名字 → 补没挂图的人。
/// `pictured` 是挂上的参考图（名字, 第几张）；画布这种不挂图的传空。
pub(crate) fn first_frame_body(config_dir: &Path, project: &CinemaProject, episode_id: &str, shot: &StoryboardShot, pictured: &[(String, usize)]) -> String {
    let (mut prompt, mut subjects) = first_frame_source(config_dir, project, episode_id, shot);
    // **兜底：换完还留着 H3 标签就不用这份。** 出图模型不认 `<Subject N>` / `<Picture N>`，
    // 留一个都是叫它瞎对编号（2026-10-03 EP02 首帧一半废片）。改为按镜头表现拼
    if has_h3_tags(&resolve_subject_tags(&prompt, &subjects, &[])) {
        prompt = compose_route_a_frame(config_dir, project, episode_id, shot);
        subjects.clear();
    }
    let body = resolve_subject_tags(&prompt, &subjects, pictured);
    let body = with_unpictured_people(config_dir, &project.id, episode_id, shot, &body, pictured);
    with_look_outfits(config_dir, &project.id, episode_id, shot, &body, pictured)
}

/// 挂了图、这一镜**选了别的造型**的人，末尾明说他这一镜穿什么。
///
/// 2026-10-07 EP03 重出首帧：星澜挂的是「嫁衣改晚礼服」那张参考图，出来的却是白纱短裙 / 粉色吊带 / 主形象的机甲胸衣 ——
/// 镜头表里还写着旧衣服（已改），**画风里也点名星澜「珍珠白和青蓝能量核心」**，而提示词里没有一句说她这一镜穿红色晚礼服。
/// 参考图说明只写了「keep … costume」，压不过两处文字。这里把造型描述里的「服装：」那一行原样写进去，并说明不按画风配色。
fn with_look_outfits(config_dir: &Path, project_id: &str, episode_id: &str, shot: &StoryboardShot, body: &str, pictured: &[(String, usize)]) -> String {
    let assets = crate::looks::for_shot(&crate::storyboard_gen::load_assets(config_dir, project_id, episode_id), shot);
    let lines = look_outfit_lines(&assets, pictured);
    if lines.is_empty() {
        return body.to_string();
    }
    format!("{body}\n这一镜的服装（以参考图为准，不按画风里的配色）：{}", lines.join("；"))
}

/// [`with_look_outfits`] 的纯函数部分：挂了图、选了非主形象造型的角色 → `「名字」（图 N）：服装那一行`
pub(crate) fn look_outfit_lines(assets: &[crate::storyboard_gen::AssetRef], pictured: &[(String, usize)]) -> Vec<String> {
    pictured
        .iter()
        .filter_map(|(name, n)| {
            let a = assets.iter().find(|a| &a.name == name && a.kind == crate::assets::AssetKind::Character)?;
            if a.look_id.is_empty() || a.look_id == crate::assets::MAIN_LOOK {
                return None;
            }
            let outfit = a.description.lines().find_map(|l| l.trim().strip_prefix("服装：")).map(str::trim).filter(|o| !o.is_empty())?;
            Some(format!("「{name}」（图 {n}）：{outfit}"))
        })
        .collect()
}

/// 正文里还有没有 H3 的标签（出图模型不认）。
pub(crate) fn has_h3_tags(text: &str) -> bool {
    ["<Subject ", "<Picture ", "<Video "].iter().any(|t| text.contains(t))
}

/// 画面里有、但没挂上参考图的人，在末尾补一句他长什么样。
///
/// 首帧最多挂 4 张（场景 + 3 人），打手、邻居一多就挂不上；正文里只有一个名字的话，
/// 模型不知道「打手乙」长什么样，只能编 —— 或者干脆照着挂了图的人再画一个（EP02 第 17 镜门口又一个铁头）。
/// 正文里已经带描述的（线路 B 换标签时补过）不重复。
fn with_unpictured_people(config_dir: &Path, project_id: &str, episode_id: &str, shot: &StoryboardShot, body: &str, pictured: &[(String, usize)]) -> String {
    let assets = crate::looks::for_shot(&crate::storyboard_gen::load_assets(config_dir, project_id, episode_id), shot);
    let lines: Vec<String> = shot.all_character_ids().iter()
        .filter_map(|id| assets.iter().find(|a| &a.id == id && a.kind == crate::assets::AssetKind::Character))
        .filter(|a| !pictured.iter().any(|(n, _)| n == &a.name) && !body.contains(&format!("「{}」 (", a.name)))
        // 开场不在画面里的（只出声的、中途才进来的）别画进首帧
        .filter(|a| shot.absent_from_opening(&a.name).is_none())
        .map(|a| {
            let d = crate::prompt_input::short_description(a, 60);
            if d.is_empty() { format!("「{}」", a.name) } else { format!("「{}」：{d}", a.name) }
        })
        .collect();
    if lines.is_empty() {
        return body.to_string();
    }
    format!("{body}\n画面里还有这几个人（没有参考图，按描述画，每人只画一个）：{}", lines.join("；"))
}

fn studio_frame_args(config_dir: &Path, project: &CinemaProject, episode_id: &str, shot: &StoryboardShot) -> Result<serde_json::Value> {
    let ratio: Vec<&str> = project.aspect_ratio.split(':').collect();
    // 项目画幅写坏了照旧报错（出片画布那边会静默回落 16:9，不能靠它）
    match ratio.as_slice() {
        [w, h] => w.parse::<f64>().ok().zip(h.parse::<f64>().ok()),
        _ => None,
    }.filter(|(w, h)| w.is_finite() && h.is_finite() && *w > 0.0 && *h > 0.0)
        .ok_or_else(|| Error::config("项目画幅格式无效"))?;
    // **首帧按出片画布的比例出**（2026-10-07）。原来是从尺寸表里挑「已实测」里最接近项目画幅的一档 ——
    // 精确 16:9 的几档都标着未实测，16:9 项目一直挑中 1216×832（约 3:2）。H3 首帧出片会把首帧**直接拉伸**到画布上
    // （WeeTodd `prepare_keyframe_image`：「The first keyframe … is *stretched* onto the canvas」），
    // 1216×832 → 1312×736 横向 ×1.079、纵向 ×0.885，人物宽了约 22%：EP03 十张首帧全是这个尺寸，「人物变矮变胖」。
    // `verified` 说的是耗时实测过没有，不是这个比例能不能用。出片前还有一道按画布裁切（`video_tool::stage_keyframe`）兜底旧首帧。
    let canvas = project.model_settings.render.size(&project.aspect_ratio);
    let (fw, fh) = frame_request_size(canvas);
    let mut args = json!({
        "engine": project.model_settings.i2i_engine,
        // 用哪份见 `first_frame_source`：手写的、按「还没首帧」合成且没过期的线路 B，其余按镜头表现拼
        "prompt": "",
        "width": fw, "height": fh,
        "output_filename": format!("frame_{}.png", uuid::Uuid::new_v4()),
    });
    let slots = first_frame_slots(&project.model_settings.i2i_engine);
    let refs = if slots > 0 { shot_frame_refs(config_dir, &project.id, episode_id, shot) } else { Vec::new() };
    let pick = if slots > 0 { Some(frame_reference_pick(&refs, &shot.dialogue_lines, &shot.blocking, slots)) } else { None };
    let pictured: Vec<(String, usize)> = pick.iter().flat_map(|p| p.names.iter().enumerate().map(|(i, n)| (n.clone(), i + 1))).collect();
    let body = first_frame_body(config_dir, project, episode_id, shot, &pictured);
    args["prompt"] = json!(body);
    if let Some(pick) = pick {
        if pick.images.is_empty() {
            // 工具那边也会拒，但它只会说「至少要一张参考图」——
            // 用户在工作区里根本没有「传参考图」这个动作，得告诉他图从哪来。
            return Err(Error::config(
                "参考生图需要这一镜里至少有一个带图的场景或角色 —— 先给资产出图，或者在项目模型设置里把首帧换回文生图引擎",
            ));
        }
        // 「参考图 N 是谁」写在最前面 —— 实测跑通的参考生图工作流都明写了
        // 「the woman from image 2」，不写的话模型不知道哪张管脸、哪张管地方
        args["prompt"] = json!(format!("{}\n\n{}", frame_ref_legend(&refs, &pick), body));
        args["reference_images"] = json!(pick.images);
    }
    let negative = shot.negative_prompt.as_deref().map(str::trim).unwrap_or_default();
    if !negative.is_empty()
        && crate::studio::catalog().engines.iter().any(|e| e.id == project.model_settings.i2i_engine && e.negative_prompt)
    {
        args["negative_prompt"] = json!(negative);
    }
    Ok(args)
}

/// 首帧请求出图的尺寸：和出片画布**同一个比例**，长边放大到 1536（出图引擎在这一档清楚又不慢），两边取 16 的倍数。
/// 取整带来的比例误差在 0.5% 以内，出片前 `stage_keyframe` 会再裁到画布本身。
pub(crate) fn frame_request_size((w, h): (u32, u32)) -> (u32, u32) {
    let (w, h) = (w.max(1) as f64, h.max(1) as f64);
    let k = 1536.0 / w.max(h);
    let r16 = |x: f64| ((x * k / 16.0).round() as u32).max(1) * 16;
    (r16(w), r16(h))
}

/// 居中裁到目标长宽比再缩放到目标尺寸。**裁，不拉伸** —— 拉伸会把脸压扁。
pub(crate) fn fit_to_frame(img: image::DynamicImage, (w, h): (u32, u32)) -> image::DynamicImage {
    if (img.width(), img.height()) == (w, h) || w == 0 || h == 0 {
        return img;
    }
    let (iw, ih) = (img.width() as f64, img.height() as f64);
    let target = w as f64 / h as f64;
    let (cw, ch) = if iw / ih > target { ((ih * target).round(), ih) } else { (iw, (iw / target).round()) };
    let (x, y) = (((iw - cw) / 2.0).round() as u32, ((ih - ch) / 2.0).round() as u32);
    img.crop_imm(x, y, (cw as u32).max(1), (ch as u32).max(1)).resize_exact(w, h, image::imageops::FilterType::Lanczos3)
}

/// 出尾帧的参数。**首帧当第 1 张参考图** —— 地方、光线、衣服、每个人的长相都照它，
/// 只改「这一镜结束时」的动作和构图。首帧不在就不出：没有首帧的尾帧和这一镜对不上。
fn end_frame_args(config_dir: &Path, project: &CinemaProject, episode_id: &str, shots_dir: &Path, shot: &StoryboardShot) -> Result<serde_json::Value> {
    let first = shots_dir.join(format!("{}.png", shot.id));
    if !first.is_file() {
        return Err(Error::config(format!("第 {} 镜还没有首帧 —— 尾帧照着首帧出，先出或导入首帧", shot.shot_num)));
    }
    let mut engine = project.model_settings.i2i_engine.clone();
    if reference_slots_of(&engine) < 2 {
        // 尾帧没有「文生」这条路：不挂首帧就是另画一张，和这一镜对不上
        engine = "qwen21_edit".into();
    }
    // 和首帧同一个上限：首帧 1 张 + 最多 3 个角色（见 `FIRST_FRAME_MAX_REFS`）
    let slots = reference_slots_of(&engine).min(FIRST_FRAME_MAX_REFS);
    let ending = if shot.end_frame.trim().is_empty() {
        shot.segments.last().map(|s| s.visual.trim().to_string()).unwrap_or_default()
    } else {
        shot.end_frame.trim().to_string()
    };
    if ending.is_empty() {
        return Err(Error::config(format!("第 {} 镜没写这一镜怎么结束（镜头表的「结尾画面」和最后一段画面都是空的）", shot.shot_num)));
    }
    let refs = shot_frame_refs(config_dir, &project.id, episode_id, shot);
    let people: Vec<&crate::assets::ResolvedRef> = refs.iter().filter(|r| r.kind == crate::assets::AssetKind::Character && r.image_path.is_some()).take(slots - 1).collect();
    let mut images = vec![first.to_string_lossy().to_string()];
    let mut legend = vec!["Image 1 is the OPENING frame of this very shot: keep the same location, camera side, lighting, colours, props and every person's face, hair and costume exactly as in it.".to_string()];
    for (i, r) in people.iter().enumerate() {
        images.push(r.image_path.clone().unwrap_or_default());
        legend.push(format!("Image {} is the person「{}」: use it only to keep this face and costume accurate.", i + 2, r.name));
    }
    let (w, h) = image::image_dimensions(&first).map_err(|e| Error::runtime(format!("首帧读不出来：{e}")))?;
    let prompt = format!(
        "Reference images:\n{}\nDraw the FINAL frame of the same continuous shot, the instant it ends — one single film frame, no panels, no text.\n\
         Camera: {} ({}); the shot starts as {}. The camera move has finished, so the framing is where that move lands — exactly as described below.\n\
         What the frame shows at the end:「{}」",
        legend.join("\n"),
        shot.camera_movement, if shot.camera_note.trim().is_empty() { "as planned" } else { shot.camera_note.trim() },
        shot.scale, ending
    );
    Ok(json!({
        "engine": engine,
        "prompt": prompt,
        "width": w, "height": h,
        "reference_images": images,
        "output_filename": format!("frame_end_{}.png", uuid::Uuid::new_v4()),
    }))
}

/// 出一镜的尾帧，存成 `shots/<id>_end.png`。返回更新后的这一镜。
pub(crate) async fn render_end_frame(config_dir: &Path, project_id: &str, episode_id: &str, shot_id: &str) -> Result<StoryboardShot> {
    use crate::agent_runtime::{RuntimeContext, RuntimePolicy, ToolCallRequest, ToolHandler, ToolStatus};
    let proj_dir = cinema_projects_dir(config_dir).join(project_id);
    let project: CinemaProject = serde_json::from_str(&std::fs::read_to_string(proj_dir.join("project.json")).map_err(|e| Error::config(format!("读不到项目：{e}")))?)
        .map_err(|e| Error::config(format!("项目文件坏了：{e}")))?;
    let ep = read_episode(config_dir, project_id, episode_id)?;
    let shot = ep.storyboard.iter().find(|s| s.id == shot_id).ok_or_else(|| Error::config(format!("找不到分镜「{shot_id}」")))?;
    let shots_dir = proj_dir.join("episodes").join(episode_id).join("shots");
    let args = end_frame_args(config_dir, &project, episode_id, &shots_dir, shot)?;
    let want = (args["width"].as_u64().unwrap_or(0) as u32, args["height"].as_u64().unwrap_or(0) as u32);
    let ctx = RuntimeContext::new(config_dir.to_path_buf(), RuntimePolicy::workspace_write(config_dir));
    let out = crate::agent_runtime::image_tool::GenerateImageTool.execute(&ctx, ToolCallRequest::new("generate_image", args)).await;
    if out.status != ToolStatus::Ok {
        return Err(Error::runtime(out.failure.map(|f| f.message).unwrap_or(out.content)));
    }
    let path = out.artifacts.first().ok_or_else(|| Error::runtime("尾帧生成没有返回文件"))?;
    let img = image::open(path).map_err(|e| Error::runtime(format!("尾帧图片无法解码：{e}")))?;
    let img = fit_to_frame(img, want);
    let temp = shots_dir.join(format!("{}.png", uuid::Uuid::new_v4()));
    img.save_with_format(&temp, image::ImageFormat::Png).map_err(|e| Error::runtime(e.to_string()))?;
    let shot = set_shot_keyframe(config_dir, project_id, episode_id, shot_id, KeyframeSlot::Last, Some(&crate::assets::MediaInput::Path { path: temp.to_string_lossy().to_string() }));
    let _ = std::fs::remove_file(&temp);
    shot
}

/// 出首帧 / 尾帧之前看一眼：**真正会发给出图模型的**提示词、引擎和挂的参考图。
///
/// 用户 2026-09-28：「没有首尾帧提示词吗？直接就出来了，不对都没法修改。」—— 首帧发的是合成好的
/// 分镜提示词（前面再拼一段参考图说明），尾帧是代码临时把「结尾画面」套进一段英文模板，两样界面上都看不到。
/// 这里和出图**同一个函数拼参数**（`studio_frame_args` / `end_frame_args`），只是不执行，看到的就是要发的。
#[derive(Debug, Clone, Serialize)]
pub struct KeyframePreview {
    pub engine: String,
    pub prompt: String,
    /// 挂的参考图（本机路径），顺序就是提示词里的「图 N / Image N」
    pub images: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub negative_prompt: Option<String>,
}

pub(crate) fn keyframe_preview(config_dir: &Path, project_id: &str, episode_id: &str, shot_id: &str, which: KeyframeSlot) -> Result<KeyframePreview> {
    let proj_dir = cinema_projects_dir(config_dir).join(project_id);
    let project = read_project(config_dir, project_id)?;
    let ep = read_episode(config_dir, project_id, episode_id)?;
    let shot = ep.storyboard.iter().find(|s| s.id == shot_id).ok_or_else(|| Error::config(format!("找不到分镜「{shot_id}」")))?;
    let args = match which {
        KeyframeSlot::First => studio_frame_args(config_dir, &project, episode_id, shot)?,
        KeyframeSlot::Last => end_frame_args(config_dir, &project, episode_id, &proj_dir.join("episodes").join(episode_id).join("shots"), shot)?,
    };
    Ok(KeyframePreview {
        engine: args["engine"].as_str().unwrap_or_default().to_string(),
        prompt: args["prompt"].as_str().unwrap_or_default().to_string(),
        images: serde_json::from_value(args["reference_images"].clone()).unwrap_or_default(),
        negative_prompt: args["negative_prompt"].as_str().filter(|n| !n.trim().is_empty()).map(str::to_string),
    })
}

#[tauri::command]
pub fn cinema_keyframe_preview(state: State<'_, AppState>, project_id: String, episode_id: String, shot_id: String, which: KeyframeSlot) -> Result<KeyframePreview> {
    keyframe_preview(&state.config_dir, &project_id, &episode_id, &shot_id, which)
}

/// 出一镜的尾帧（单镜按钮）。登记一条 `Frame` 任务。
#[tauri::command]
pub async fn cinema_render_end_frame(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    project_id: String,
    episode_id: String,
    shot_id: String,
) -> Result<StoryboardShot> {
    let req = RenderShotReq { project_id: project_id.clone(), episode_id: episode_id.clone(), shot_id: shot_id.clone(), engine_config: None };
    let (num, _) = shot_brief(&state.config_dir, &req);
    let job = crate::jobs::start(&state.config_dir, crate::jobs::JobKind::Frame, &project_id, &episode_id, &shot_id, num, "尾帧");
    crate::jobs::emit(Some(&app), crate::jobs::get(&state.config_dir, &job).as_ref());
    let out = render_end_frame(&state.config_dir, &project_id, &episode_id, &shot_id).await;
    let done = match &out {
        Ok(s) => crate::jobs::finish(&state.config_dir, &job, s.end_image_url.as_deref(), None),
        Err(e) => crate::jobs::finish(&state.config_dir, &job, None, Some(&e.to_string())),
    };
    crate::jobs::emit(Some(&app), done.as_ref());
    out
}

async fn render_studio_frame(
    config_dir: &Path,
    project: &CinemaProject,
    shot_id: &str,
    ep_file: &Path,
    shots_dir: &Path,
    shot: &StoryboardShot,
    continuity_note: Option<&str>,
) -> Result<RenderShotResult> {
    use crate::agent_runtime::{RuntimeContext, RuntimePolicy, ToolCallRequest, ToolHandler, ToolStatus};
    // 分集 id 就是分集目录名
    let episode_id = ep_file.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()).unwrap_or_default();
    let args = studio_frame_args(config_dir, project, episode_id, shot)?;
    let want = (args["width"].as_u64().unwrap_or(0) as u32, args["height"].as_u64().unwrap_or(0) as u32);
    // **续接单**：交给 ComfyUI 之前记下这一张是哪一镜的首帧、要裁成多大（应用中途关掉，重开时照它收图，见 `render_resume`）
    crate::jobs::note_resume(json!({"kind": "frame", "project_id": project.id, "episode_id": episode_id, "shot_id": shot_id, "width": want.0, "height": want.1}));
    let ctx = RuntimeContext::new(config_dir.to_path_buf(), RuntimePolicy::workspace_write(config_dir));
    let out = crate::agent_runtime::image_tool::GenerateImageTool
        .execute(&ctx, ToolCallRequest::new("generate_image", args.clone())).await;
    if out.status != ToolStatus::Ok {
        return Err(Error::runtime(out.failure.map(|f| f.message).unwrap_or(out.content)));
    }
    let path = out.artifacts.first().ok_or_else(|| Error::runtime("首帧生成没有返回文件"))?;
    let bytes = std::fs::read(path).map_err(|e| Error::runtime(format!("读取首帧失败：{e}")))?;
    let unused = frame_reference_pick(
        &shot_frame_refs(config_dir, &project.id, episode_id, shot), &shot.dialogue_lines, &shot.blocking,
        first_frame_slots(&project.model_settings.i2i_engine)).unused;
    let mut notes: Vec<String> = continuity_note.map(|n| format!("衔接未生效：{n}")).into_iter().collect();
    if !unused.is_empty() {
        notes.push(format!("首帧最多挂 {FIRST_FRAME_MAX_REFS} 张参考图（场景和角色优先），{} 的图没挂、按文字描述", unused.join("、")));
    }
    install_first_frame(ep_file, shots_dir, shot_id, &bytes, want, &notes)
}

/// 出好的首帧落盘、挂到这一镜上。正常出完和重启后续接收图（`render_resume`）共用。
///
/// `want` 是项目画幅：参考生图的画幅跟着**第一张参考图**走（Qwen 2.1 的节点就是这么定尺寸的），
/// 场景图不是 16:9 的话首帧也不是 —— 裁回项目画幅，出片才不变形。
pub(crate) fn install_first_frame(ep_file: &Path, shots_dir: &Path, shot_id: &str, bytes: &[u8], want: (u32, u32), notes: &[String]) -> Result<RenderShotResult> {
    let decoded = image::load_from_memory(bytes).map_err(|e| Error::runtime(format!("首帧图片无法解码：{e}")))?;
    let decoded = if want.0 > 0 && want.1 > 0 { fit_to_frame(decoded, want) } else { decoded };
    // 出图要几分钟，这期间镜头可能被删了 —— 删了就别往 shots 里留一张没人认的图
    let ep: EpisodeData = serde_json::from_str(&std::fs::read_to_string(ep_file)
        .map_err(|e| Error::runtime(e.to_string()))?).map_err(|e| Error::config(e.to_string()))?;
    if !ep.storyboard.iter().any(|s| s.id == shot_id) {
        return Err(Error::runtime("该镜头已删除，首帧未绑定"));
    }
    let dest = shots_dir.join(format!("{shot_id}.png"));
    let temp = shots_dir.join(format!("{}.png", uuid::Uuid::new_v4()));
    decoded.save_with_format(&temp, image::ImageFormat::Png).map_err(|e| Error::runtime(e.to_string()))?;
    std::fs::rename(temp, &dest).map_err(|e| Error::runtime(e.to_string()))?;
    // **存路径，不存 base64。**
    //
    // 老路径把整张图 base64 塞进 `episode.json`。这个仓库为这件事付过
    // 一次代价（86 MB 的 `characters.json`，读一次角色列表要解析整个文件），
    // 而现在每次失焦保存都会整份读写这个 json —— 十几镜就是几十 MB
    // 在 IPC 上来回搬。
    //
    // 界面侧不需要 base64：`anima-media://` 协议直接读本机路径，
    // 而 shots 目录在 `config_dir` 底下、在允许的根目录里。
    let url = dest.to_string_lossy().to_string();
    let status = if notes.is_empty() { "ready".to_string() } else { format!("ready({})", notes.join("；")) };
    // 在锁里按最新的那份只改这一镜，出图期间别处的改动不会被冲掉
    crate::episode_lock::update_file(ep_file, |ep| {
        let saved = ep.storyboard.iter_mut().find(|s| s.id == shot_id)
            .ok_or_else(|| Error::runtime("该镜头已删除，首帧未绑定"))?;
        saved.image_url = Some(url.clone());
        saved.keyframes.first = true;
        saved.status = status;
        Ok(RenderShotResult {
            shot_id: shot_id.into(), image_url: url,
            video_url: saved.video_url.clone().unwrap_or_default(),
            audio_url: saved.audio_url.clone().unwrap_or_default(), status: saved.status.clone(),
        })
    })
}

/// 这一镜存下的最终提示词。画布的镜头节点读它 —— 和出片发送的是同一份。
#[derive(Debug, Clone, serde::Serialize)]
pub struct StoredShotPrompts {
    /// 项目当前用的线路
    pub route: crate::shot_table::PromptRoute,
    /// 当前线路的分镜提示词
    pub frame: String,
    /// 当前线路的视频运动提示词
    pub video: String,
    /// 每条线路各自存的，给界面对比用
    pub routes: crate::shot_table::ShotPromptSet,
}

#[tauri::command]
pub fn cinema_shot_prompt_preview(
    state: State<'_, AppState>,
    project_id: String,
    episode_id: String,
    shot_id: String,
) -> Result<StoredShotPrompts> {
    let proj_dir = cinema_projects_dir(&state.config_dir).join(&project_id);
    let route = std::fs::read_to_string(proj_dir.join("project.json")).ok()
        .and_then(|t| serde_json::from_str::<CinemaProject>(&t).ok())
        .map(|p| p.model_settings.prompt_route)
        .unwrap_or_default();
    let f = proj_dir.join("episodes").join(&episode_id).join("episode.json");
    let ep: EpisodeData = serde_json::from_str(&std::fs::read_to_string(&f).unwrap_or_default())
        .map_err(|e| Error::config(format!("读不出 episode.json：{e}")))?;
    let shot = ep.storyboard.iter().find(|s| s.id == shot_id)
        .ok_or_else(|| Error::config(format!("找不到分镜「{shot_id}」")))?;
    let current = shot.prompts.get(&route).cloned().unwrap_or_default();
    // 画布的分镜节点把 `frame` 接给下游出图：给它出首帧时真正发的那份正文（标签换成名字、
    // 按首帧写的 / 过期的不用、没合成过按镜头表现拼），不给 H3 那套原文
    let frame = read_project(&state.config_dir, &project_id)
        .map(|p| first_frame_body(&state.config_dir, &p, &episode_id, shot, &[]))
        .unwrap_or(current.frame);
    Ok(StoredShotPrompts { route, frame, video: current.video, routes: shot.prompts.clone() })
}

/// 把画布上的一张图收成这一镜的首帧。
///
/// # 为什么是复制而不是记一个路径
///
/// 首帧的位置是**约定死的** `shots/<shot_id>.png` —— 出片、重出、
/// 打包全按这个路径找。记一个画布输出目录的路径的话，那边一清理，
/// 成片就少一帧，而且是到导出时才发现。
///
/// 复制完**不删原图** —— 画布上那张还在节点里显示着，删了就成了碎图。
#[tauri::command]
pub fn cinema_adopt_frame(
    state: State<'_, AppState>,
    project_id: String,
    episode_id: String,
    shot_id: String,
    source_path: String,
) -> Result<u64> {
    adopt_frame(&state.config_dir, &project_id, &episode_id, &shot_id, &source_path)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn cinema_adopt_video(
    state: State<'_, AppState>,
    project_id: String,
    episode_id: String,
    shot_id: String,
    source_path: String,
    prompt: String,
    engine: String,
    seed: Option<u64>,
    picture_paths: Option<Vec<String>>,
) -> Result<AdoptVideoOut> {
    adopt_video(&state.config_dir, &project_id, &episode_id, &shot_id, &source_path,
        &prompt, &engine, seed.unwrap_or(0), picture_paths.unwrap_or_default())
}

#[tauri::command]
pub fn cinema_pick_take(
    state: State<'_, AppState>,
    project_id: String,
    episode_id: String,
    shot_id: String,
    take_path: String,
) -> Result<String> {
    pick_take(&state.config_dir, &project_id, &episode_id, &shot_id, &take_path)
}

/// [`cinema_adopt_frame`] 的实现。**分开是为了测得到** ——
/// 包着 `State` 的那层起不了一个 tauri app 就跑不了。
pub(crate) fn adopt_frame(
    config_dir: &Path,
    project_id: &str,
    episode_id: &str,
    shot_id: &str,
    source_path: &str,
) -> Result<u64> {
    let src = std::path::Path::new(source_path);
    if !src.is_file() {
        return Err(Error::config(format!("这张图不在了：{source_path}")));
    }
    let ep_dir = cinema_projects_dir(config_dir)
        .join(project_id)
        .join("episodes")
        .join(episode_id);
    // **先确认这一镜真的存在**，否则会在分集目录里留下一个谁也找不到的
    // `shots/<不存在的 id>.png`，而界面上什么都不会变。
    let ep_file = ep_dir.join("episode.json");
    let ep: EpisodeData = serde_json::from_str(&std::fs::read_to_string(&ep_file).unwrap_or_default())
        .map_err(|e| Error::config(format!("读不出分集：{e}")))?;
    if !ep.storyboard.iter().any(|s| s.id == shot_id) {
        return Err(Error::config(format!("找不到分镜「{shot_id}」")));
    }
    let shots_dir = ep_dir.join("shots");
    std::fs::create_dir_all(&shots_dir)
        .map_err(|e| Error::config(format!("建不出 shots 目录：{e}")))?;
    let dst = shots_dir.join(format!("{shot_id}.png"));
    std::fs::copy(src, &dst).map_err(|e| Error::config(format!("收图失败：{e}")))?;
    // 返回新的 `image_rev` —— 路径没变，前端不换 `v=` 的话看到的还是旧图。
    Ok(std::fs::metadata(&dst)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0))
}

/// 一镜的首帧 / 尾帧：导入一张，或者摘掉（`media` 为 `None`）。
///
/// 出片模式按挂了哪几张**自动定**（见 [`Keyframes`]）—— 用户不用另选「首帧模式」。
/// 挂上 / 摘掉会让已经合成的提示词过期（正文写法不同），界面会提示重新合成。
///
/// 导入的图**裁成项目画幅**：H3 按出片尺寸吃首帧，比例不对就是拉伸变形，而且不报错。
#[tauri::command]
pub fn cinema_set_shot_keyframe(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    project_id: String,
    episode_id: String,
    shot_id: String,
    which: KeyframeSlot,
    media: Option<crate::assets::MediaInput>,
) -> Result<StoryboardShot> {
    let shot = set_shot_keyframe(&state.config_dir, &project_id, &episode_id, &shot_id, which, media.as_ref())?;
    // 首帧加上 / 摘掉 = 换了出片链路：提示词按新链路自动重合成（2026-10-07 用户：删掉首帧自动恢复参考出片）
    resynth_after_route_change(&app, &state, &project_id, &episode_id, &shot_id);
    Ok(shot)
}

/// 首帧还是尾帧。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyframeSlot {
    First,
    Last,
}

pub(crate) fn set_shot_keyframe(
    config_dir: &Path,
    project_id: &str,
    episode_id: &str,
    shot_id: &str,
    which: KeyframeSlot,
    media: Option<&crate::assets::MediaInput>,
) -> Result<StoryboardShot> {
    let ep_dir = cinema_projects_dir(config_dir).join(project_id).join("episodes").join(episode_id);
    let ep_file = ep_dir.join("episode.json");
    let ep: EpisodeData = serde_json::from_str(&std::fs::read_to_string(&ep_file).map_err(|e| Error::config(format!("读不到分集：{e}")))?)
        .map_err(|e| Error::config(format!("分集文件坏了：{e}")))?;
    if !ep.storyboard.iter().any(|s| s.id == shot_id) {
        return Err(Error::config(format!("找不到分镜「{shot_id}」")));
    }
    let shots_dir = ep_dir.join("shots");
    std::fs::create_dir_all(&shots_dir).map_err(|e| Error::config(format!("建不出 shots 目录：{e}")))?;
    let dest = match which {
        KeyframeSlot::First => shots_dir.join(format!("{shot_id}.png")),
        KeyframeSlot::Last => end_frame_path(&shots_dir, shot_id),
    };
    match media {
        None => {
            if dest.is_file() {
                std::fs::remove_file(&dest).map_err(|e| Error::config(format!("摘不掉：{e}")))?;
            }
        }
        Some(m) => {
            let (bytes, _) = m.read()?;
            let img = image::load_from_memory(&bytes).map_err(|e| Error::config(format!("这张图读不出来：{e}")))?;
            let aspect = std::fs::read_to_string(cinema_projects_dir(config_dir).join(project_id).join("project.json")).ok()
                .and_then(|t| serde_json::from_str::<CinemaProject>(&t).ok())
                .map(|p| p.aspect_ratio)
                .unwrap_or_default();
            let img = match aspect_parts(&aspect) {
                Some((a, b)) => {
                    // 保留原图的分辨率，只把多出来的边裁掉
                    let (w, h) = (img.width() as f64, img.height() as f64);
                    let size = if w / h > a / b { ((h * a / b).round() as u32, h as u32) } else { (w as u32, (w * b / a).round() as u32) };
                    fit_to_frame(img, size)
                }
                None => img,
            };
            // 先写临时文件再改名：出片那边可能正好在读这个路径
            let temp = shots_dir.join(format!("{}.png", uuid::Uuid::new_v4()));
            img.save_with_format(&temp, image::ImageFormat::Png).map_err(|e| Error::runtime(format!("存不下：{e}")))?;
            std::fs::rename(&temp, &dest).map_err(|e| Error::runtime(format!("存不下：{e}")))?;
        }
    }
    // 图在锁外裁好、换上；锁里按最新的那份只改这一镜
    let mut ep = crate::episode_lock::update_file(&ep_file, |ep| {
        sync_keyframes(&shots_dir, ep);
        let shot = ep.storyboard.iter_mut().find(|s| s.id == shot_id).ok_or_else(|| Error::config(format!("找不到分镜「{shot_id}」")))?;
        if which == KeyframeSlot::First {
            shot.image_url = dest.is_file().then(|| dest.to_string_lossy().to_string());
        }
        Ok(ep.clone())
    })?;
    // 首帧加上 / 摘掉 = 链路变了（首帧出片 ↔ 参考出片），「接上一镜」跟着按规则改
    if !auto_continuity_now(config_dir, project_id, episode_id).is_empty() {
        if let Ok(fresh) = read_episode(config_dir, project_id, episode_id) {
            ep = fresh;
        }
    }
    hydrate_episode(&ep_dir, &mut ep);
    Ok(ep.storyboard.into_iter().find(|s| s.id == shot_id).expect("上面查过"))
}

/// 画布回填：把画布上出的一段视频收进某一镜。
///
/// # 它补的断口
///
/// 图片早就能「收为首帧」，视频不能 —— 在画布上把一镜修好之后，
/// 只能去 Finder 里找文件再手动放进分集目录，而放错地方不报错。
///
/// # 为什么追加 take 而不是覆盖
///
/// `takes` 本来就是版本链（新的在后，`video_url` 指最新一条）。
/// 覆盖的话旧版本就没了，而画布出的片**不一定比原来那条好** ——
/// 用户得能换回去。这里不造第二套版本机制。
///
/// 回填的 take 标 [`TakeOrigin::Canvas`]，理由见那里。
#[allow(clippy::too_many_arguments)]
pub(crate) fn adopt_video(
    config_dir: &Path,
    project_id: &str,
    episode_id: &str,
    shot_id: &str,
    source_path: &str,
    prompt: &str,
    engine: &str,
    seed: u64,
    picture_paths: Vec<String>,
) -> Result<AdoptVideoOut> {
    let src = std::path::Path::new(source_path);
    if !src.is_file() {
        return Err(Error::config(format!("这段视频不在了：{source_path}")));
    }
    let ep_dir = cinema_projects_dir(config_dir)
        .join(project_id)
        .join("episodes")
        .join(episode_id);
    let ep_file = ep_dir.join("episode.json");
    let ep: EpisodeData = serde_json::from_str(&std::fs::read_to_string(&ep_file).unwrap_or_default())
        .map_err(|e| Error::config(format!("读不出分集：{e}")))?;
    if !ep.storyboard.iter().any(|s| s.id == shot_id) {
        return Err(Error::config(format!("找不到分镜「{shot_id}」")));
    }
    let shots_dir = ep_dir.join("shots");
    std::fs::create_dir_all(&shots_dir)
        .map_err(|e| Error::config(format!("建不出 shots 目录：{e}")))?;
    // **按时间各存一份，不覆盖任何已有的「次」。**
    let dst = shots_dir.join(format!("{shot_id}-canvas-{}.mp4", crate::jobs::now()));
    std::fs::copy(src, &dst).map_err(|e| Error::config(format!("收片失败：{e}")))?;
    let url = dst.to_string_lossy().to_string();

    // **在锁里按最新的那份只改这一镜。** 和出片那条同一个理由：
    // 这期间合成提示词、改别的镜头都在写同一个文件。
    let take_count = crate::episode_lock::update_file(&ep_file, |latest| {
        let Some(target) = latest.storyboard.iter_mut().find(|s| s.id == shot_id) else {
            return Err(Error::config(format!("收片完成，但这一镜已经不在了：{shot_id}")));
        };
        target.takes.push(VideoTake {
            // 回填没走线路 —— 给默认值，靠 `origin` 区分，别的地方按 origin 过滤
            route: crate::shot_table::PromptRoute::default(),
            path: url.clone(),
            created_at: crate::jobs::now(),
            seed,
            pictures: picture_paths.len(),
            picture_paths,
            quality: format!("画布 · {engine}"),
            prompt: prompt.to_string(),
            // **故意留空。** 回填的片不是按镜头表出的，写个指纹进去等于宣称
            // 「它和当前字段对得上」—— 那是句假话，而且会盖掉真正的过期警告。
            fields_hash: String::new(),
            style: String::new(),
            origin: TakeOrigin::Canvas,
        });
        target.video_url = Some(url.clone());
        target.status = "ready".to_string();
        Ok(target.takes.len())
    })?;
    Ok(AdoptVideoOut { video_url: url, take_count })
}

#[derive(Debug, Clone, Serialize)]
pub struct AdoptVideoOut {
    pub video_url: String,
    /// 这一镜现在总共几「次」。界面用它说「已收为第 N 次」。
    pub take_count: usize,
}

/// 换回某一「次」。**回填必须能撤销** —— 画布出的片不一定比原来那条好。
///
/// 只改 `video_url` 的指向，不动 `takes`：历史是历史，换来换去都不该丢。
pub(crate) fn pick_take(
    config_dir: &Path,
    project_id: &str,
    episode_id: &str,
    shot_id: &str,
    take_path: &str,
) -> Result<String> {
    let ep_file = cinema_projects_dir(config_dir)
        .join(project_id)
        .join("episodes")
        .join(episode_id)
        .join("episode.json");
    crate::episode_lock::update_file(&ep_file, |ep| {
        let Some(shot) = ep.storyboard.iter_mut().find(|s| s.id == shot_id) else {
            return Err(Error::config(format!("找不到分镜「{shot_id}」")));
        };
        // **只认这一镜自己出过的那几条。** 不然这个接口就成了「把任意路径写进分集」
        if !shot.takes.iter().any(|t| t.path == take_path) {
            return Err(Error::config("这一「次」不属于这一镜".to_string()));
        }
        if !std::path::Path::new(take_path).is_file() {
            return Err(Error::config(format!("这一「次」的文件不在了：{take_path}")));
        }
        shot.video_url = Some(take_path.to_string());
        Ok(take_path.to_string())
    })
}

/// 这一集出首帧 / 出片用的尺寸。
///
/// **两处必须是同一组数。** 首帧和视频尺寸不一致的话，i2v 那一步会缩放，
/// 而缩放会把首帧精心控好的构图裁掉一圈。
///
/// 尺寸交给 `weetodd::fit_dimensions` 定 —— 32 对齐 + 长边不超 1344 是
/// 引擎的硬约束（H3 的 `WeeToddH3GenerationConfig` 直接拒绝非 32 倍数：
/// `width and height must be divisible by 32`）。
///
/// 以前这里写死 720×1280 / 1280×720，而 **720 ÷ 32 = 22.5** ——
/// 两个画幅都会在出片时失败，报的还是一句看不懂的英文。
/// SDXL 只要求 8 的倍数，32 也满足，所以首帧跟着用同一组数。
pub fn shot_render_size(aspect_ratio: &str) -> (u32, u32) {
    crate::agent_runtime::weetodd::fit_dimensions(frame_for(aspect_ratio, 1280, 32, (9.0, 16.0)))
}

/// 整部剧能选的画幅。**和出片设置里的 `renderOptions.ts` 的 `ASPECTS` 是同一组**（有测试对拍）。
pub const SERIES_ASPECTS: [&str; 6] = ["16:9", "9:16", "1:1", "4:3", "3:4", "21:9"];

/// `"21:9"` → `(21.0, 9.0)`。读不出来 = `None`。
pub(crate) fn aspect_parts(aspect: &str) -> Option<(f64, f64)> {
    let (a, b) = aspect.split_once(':')?;
    let (a, b) = (a.trim().parse::<f64>().ok()?, b.trim().parse::<f64>().ok()?);
    (a > 0.0 && b > 0.0).then_some((a, b))
}

/// 按画幅算画面尺寸：**长边固定**，短边按比例算、对齐到 `align`。读不出画幅就按 `fallback`。
///
/// 以前这几处写死「16:9 就横，否则竖」—— 21:9 会被静默出成竖屏（2026-09-26 用户要加宽画幅时查出来）。
pub(crate) fn frame_for(aspect: &str, long: u32, align: u32, fallback: (f64, f64)) -> (u32, u32) {
    let (a, b) = aspect_parts(aspect).unwrap_or(fallback);
    let short = |x: f64| (((x / align as f64).round() as u32).max(1)) * align;
    if a >= b {
        (long, short(long as f64 * b / a))
    } else {
        (short(long as f64 * a / b), long)
    }
}

pub async fn cinema_render_shot_with_deps(
    config_dir: &Path,
    // 老的 SDXL 链路自己连 ComfyUI 用的；现在出图统一走出图工具，留着是为了不动调用方。
    _http: reqwest::Client,
    req: RenderShotReq,
) -> Result<RenderShotResult> {
    let root = cinema_projects_dir(config_dir);
    let proj_dir = root.join(&req.project_id);
    let ep_dir = proj_dir.join("episodes").join(&req.episode_id);
    let shots_dir = ep_dir.join("shots");
    let _ = std::fs::create_dir_all(&shots_dir);

    let ep_file = ep_dir.join("episode.json");
    if !ep_file.is_file() {
        return Err(Error::config("分集不存在"));
    }

    let ep: EpisodeData =
        serde_json::from_str(&std::fs::read_to_string(&ep_file).unwrap_or_default())
            .map_err(|e| Error::config(format!("读取分集失败：{e}")))?;

    let shot_opt = ep.storyboard.iter().find(|s| s.id == req.shot_id);
    if shot_opt.is_none() {
        return Err(Error::config(format!("找不到分镜「{}」", req.shot_id)));
    }
    // **只读取值，不在这里拿可变借用。**
    //
    // 下面的链式衔接要遍历 `ep.storyboard` 找上一镜，而可变借用会一直活到
    // 函数后半段 —— 提前拿会让那次只读遍历借用冲突。
    let shot = shot_opt.unwrap();
    let shot_num_of_current = shot.shot_num;
    let continues_previous = shot.effective_continuity() == Continuity::FirstFrame;
    let shot_id_str = shot.id.clone();

    // 首帧只走一条路：项目设置里选的出图引擎 + 这一镜存下的分镜提示词。
    // 以前还有一条 SDXL 专用的老链路，自己拼提示词，ComfyUI 没出图时
    // 拿一张写着镜头信息的 SVG 冒充首帧 —— 2026-09-13 分镜重构时删掉。
    let project: CinemaProject = serde_json::from_str(
        &std::fs::read_to_string(proj_dir.join("project.json"))
            .map_err(|e| Error::config(format!("读取首帧模型设置失败：{e}")))?,
    )
    .map_err(|e| Error::config(format!("解析首帧模型设置失败：{e}")))?;
    {
        let engine = project.model_settings.i2i_engine.as_str();
        if !crate::studio::catalog().engines.iter().any(|e| e.kind == "image" && e.id == engine) {
            return Err(Error::config(format!("不支持的首帧模型：{engine}")));
        }
    }

    let shot_img_path = shots_dir.join(format!("{shot_id_str}.png"));

    // 1.5 镜头链式衔接：这一镜接着上一镜时，关键帧直接用上一镜的**末帧**。
    //
    // 这是 AI 漫剧连贯性的主要手段 —— 重新生成一张关键帧必然和上一镜的收尾
    // 对不上（人物姿态、光线、构图都会漂），而末帧本身就是完美衔接点。
    //
    // **失败不传染。** 上一镜没渲、或者末帧抽取失败时，退回正常生成并把原因
    // 记进 status —— 让第 4 镜因为第 3 镜没渲就整个报错，是把一个可恢复的
    // 情况变成了硬失败。
    let mut continuity_note: Option<String> = None;
    if continues_previous {
        let prev = ep
            .storyboard
            .iter()
            .filter(|s| s.shot_num < shot_num_of_current)
            .max_by_key(|s| s.shot_num)
            .map(|s| s.id.clone());
        match prev {
            None => {
                continuity_note = Some("这是第一镜，没有上一镜可接，已改为正常生成".into());
            }
            Some(prev_id) => {
                let src = last_frame_path(&shots_dir, &prev_id);
                if src.is_file() && std::fs::copy(&src, &shot_img_path).is_ok() {
                    // 关键帧已经就位，不用再生成
                    // **这里以前不写 `image_url`。** 末帧文件复制过去了，
                    // 字段还是空的 —— 界面上这一镜看起来「还没出首帧」，
                    // 而磁盘上它已经有了。
                    let url = shot_img_path.to_string_lossy().to_string();
                    let _ = crate::episode_lock::update_file(&ep_file, |ep| {
                        if let Some(shot) = ep.storyboard.iter_mut().find(|s| s.id == req.shot_id) {
                            shot.status = "ready".to_string();
                            shot.keyframes.first = true;
                            shot.image_url = Some(url.clone());
                        }
                        Ok(())
                    });
                    return Ok(RenderShotResult {
                        shot_id: req.shot_id,
                        image_url: url,
                        video_url: String::new(),
                        audio_url: String::new(),
                        status: "ready".to_string(),
                    });
                }
                continuity_note = Some(format!(
                    "上一镜「{prev_id}」还没有渲染出视频（找不到末帧），已改为正常生成"
                ));
            }
        }
    }

    render_studio_frame(config_dir, &project, &req.shot_id, &ep_file, &shots_dir, shot,
        continuity_note.as_deref()).await
}

/// 为指定分镜生成基于真实运镜指令的动态视频（推拉摇移/I2V）
/// 把一段视频的**最后一帧**抽出来存成 PNG。
///
/// 这是镜头链式衔接的基础：第 N 镜的末帧就是第 N+1 镜的首帧，
/// 镜头之间不跳、人物不漂。
///
/// **`-sseof -0.1` 而不是 `select=eq(n,last)`** —— 后者要把整段解码一遍数帧，
/// 前者直接从文件尾部倒退定位，同样准确但快得多。
///
/// 失败返回 `false` 而不是报错：抽不到末帧只是下一镜没法接续，
/// 不该让**这一镜**的渲染结果作废 —— 它已经成功了。
fn extract_last_frame(ffmpeg_bin: &str, video: &Path, dest: &Path) -> bool {
    if !video.is_file() {
        return false;
    }
    let ok = std::process::Command::new(ffmpeg_bin)
        .args(["-v", "error", "-sseof", "-0.1", "-i"])
        .arg(video)
        .args(["-vframes", "1", "-y"])
        .arg(dest)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    ok && dest.is_file()
}

/// 上一镜末帧存在哪。和 `{shot_id}.png` 放同一个目录，名字加 `_last` 后缀。
fn last_frame_path(shots_dir: &Path, shot_id: &str) -> PathBuf {
    shots_dir.join(format!("{shot_id}_last.png"))
}

/// 出片时挂的参考图文件，**顺序和合成提示词用的一致**（[`crate::prompt_input::ShotContext::slots`]）。
///
/// - 资产槽位：取资产当前选中的那张图；
/// - 起始帧 / 结束帧：这一镜自己的 `shots/<id>.png` / `shots/<id>_end.png`，文件不在就跳过
///   （提示词那边同样只按实际挂上的算，见 `slots()` 的注释）。
fn render_reference_paths(
    config_dir: &Path,
    project_id: &str,
    episode_id: &str,
    shots_dir: &Path,
    shot: &StoryboardShot,
    assets: &[crate::storyboard_gen::AssetRef],
    long_take: bool,
) -> Vec<PathBuf> {
    use crate::shot_table::RefUsage;
    let assets = crate::looks::for_shot(assets, shot);
    // **`long_take` 必须和合成时一样。** 长镜头里挂了首帧也不走首帧出片（`ShotContext::keyframe_video`），
    // 提示词按资产槽位编号；这里写死 false 的话挂出去的只有首帧那一张，<Picture 2..N> 全指向空处（2026-10-03 审查查出）
    let ctx = crate::prompt_input::ShotContext { shot, prev: None, assets: &assets, style: "", long_take };
    ctx.slots()
        .into_iter()
        .filter_map(|(slot, asset)| match (slot.usage, asset) {
            (_, Some(a)) => asset_image_path(config_dir, project_id, episode_id, &a.id, &a.look_id)
                .map(|p| if a.kind == crate::assets::AssetKind::Scene { flat_scene_ref(config_dir, &p) } else { p }),
            (RefUsage::StartFrame, None) => Some(shots_dir.join(format!("{}.png", shot.id))).filter(|p| p.is_file()),
            (RefUsage::EndFrame, None) => Some(end_frame_path(shots_dir, &shot.id)).filter(|p| p.is_file()),
            _ => None,
        })
        .collect()
}

/// 提示词里引用到的最大图号：线路 B 的 `<Picture N>`、线路 A 的「参考图N」。
fn max_picture_ref(prompt: &str) -> usize {
    let mut max = 0;
    for (open, needs_close) in [("<Picture ", true), ("参考图", false)] {
        for (at, _) in prompt.match_indices(open) {
            let rest = &prompt[at + open.len()..];
            let n = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
            if n > 0 && (!needs_close || rest[n..].starts_with('>')) {
                max = max.max(rest[..n].parse().unwrap_or(0));
            }
        }
    }
    max
}

/// **出片前最后一道关：提示词里点名的图，这次真的挂上了没有。**
///
/// 合成时按槽位编好了号（`<Picture 3>` = 第 3 张），出片时挂图是另算一遍的
/// （资产图文件丢了、单镜换了形象没图、长镜头 / 首帧模式判断不一致……都会少挂）。少一张，
/// 后面的编号整体错位，H3 **不报错**，几个小时后出来的人是错的（2026-10-03 审查：长镜头挂了首帧就是这样）。
/// 这里在发给 ComfyUI 之前对一遍，对不上就停，几秒钟的事。
/// `<Video 1>`（接上一镜）同理：写了却没挂视频，标签指向空处。
pub(crate) fn render_refs_guard(shot_num: u32, prompt: &str, pictures: usize, ref_video: bool) -> Result<()> {
    let wanted = max_picture_ref(prompt);
    if wanted > pictures {
        return Err(Error::config(format!(
            "第 {shot_num} 镜的提示词点名了第 {wanted} 张参考图，这次只挂得上 {pictures} 张 —— 编号会整体错位，没有发出去。\
             先看资产图在不在、单镜换的形象有没有图，再重新合成这一镜"
        )));
    }
    if prompt.contains("<Video 1>") && !ref_video {
        return Err(Error::config(format!("第 {shot_num} 镜的提示词写了接上一镜（<Video 1>），这次没挂上一镜的片子 —— 没有发出去，重新合成这一镜")));
    }
    Ok(())
}

/// **360 全景场景图转成一张正常透视的 16:9 图**，再当参考图发出去。
///
/// 《回收师》的场景资产是 2:1 等距柱状全景。整张直接发给 H3，它当成一张奇怪的平面图 ——
/// E01 第 3 镜出来是一间编出来的机库，跟拆机间不沾边；Codex 做预告时留了一段同样原因的废稿
/// （`resources/回收师/…/H3-360参考重建/废稿-全景畸变-*.mp4`），然后用 ffmpeg `v360`
/// 投影出直线透视图再发，就对了。这里照做：正前方（全景中心）、水平视场 100°。
///
/// 不是 2:1 的图原样返回。投影失败也原样返回（发全景总比不发强），并记日志。
/// 缓存在 `<config>/cache/pano_flat/`，按源文件路径 + 修改时间命名，源图换了自动重算。
pub(crate) fn flat_scene_ref(config_dir: &Path, path: &Path) -> PathBuf {
    let Ok((w, h)) = image::image_dimensions(path) else { return path.to_path_buf() };
    if !is_equirect(w, h) {
        return path.to_path_buf();
    }
    let mtime = media_rev(path);
    let key = {
        use std::hash::{Hash, Hasher};
        let mut hs = std::collections::hash_map::DefaultHasher::new();
        (path, mtime).hash(&mut hs);
        hs.finish()
    };
    let dir = config_dir.join("cache").join("pano_flat");
    let out = dir.join(format!("{key:016x}.png"));
    if out.is_file() {
        return out;
    }
    let _ = std::fs::create_dir_all(&dir);
    let ok = std::process::Command::new(ffmpeg_binary())
        .args(["-v", "error", "-y", "-i"])
        .arg(path)
        .args(["-vf", PANO_FLAT_FILTER, "-frames:v", "1"])
        .arg(&out)
        .status()
        .is_ok_and(|s| s.success());
    if ok && out.is_file() {
        out
    } else {
        tracing::warn!("全景转透视失败，照原图发：{}", path.display());
        path.to_path_buf()
    }
}

/// 2:1（±2.5%）就当等距柱状全景。
fn is_equirect(w: u32, h: u32) -> bool {
    h > 0 && (1.95..=2.05).contains(&(w as f64 / h as f64))
}

/// 水平 100°、16:9（竖直视场 = 2·atan(tan 50° · 9/16) ≈ 67.7°），正前方。
const PANO_FLAT_FILTER: &str = "v360=input=e:output=flat:h_fov=100:v_fov=67.7:w=1344:h=756:interp=lanczos";

/// 资产某个形象当前选中那张图的绝对路径（按 全局 → 项目 → 本集 找）。没有或文件不在返回 `None`。
/// `look_id` 空 = 主形象；形象不在了按主形象。
pub(crate) fn asset_image_path(config_dir: &Path, project_id: &str, episode_id: &str, asset_id: &str, look_id: &str) -> Option<PathBuf> {
    use crate::assets::Scope;
    [
        Scope::Global,
        Scope::Project { project_id: project_id.into() },
        Scope::Episode { project_id: project_id.into(), episode_id: episode_id.into() },
    ]
    .into_iter()
    .find_map(|scope| {
        let store = crate::assets::load_store(config_dir, &scope);
        let a = store.assets.iter().find(|a| a.id == asset_id)?;
        let unit = a.look_unit(Some(look_id).filter(|l| !l.is_empty())).unwrap_or(&a.unit);
        let path = crate::assets::variant_abs_path(config_dir, &scope, unit.selected_image()?);
        path.is_file().then_some(path)
    })
}

/// 用户指定的**尾帧**（和 `_last.png` 不是一回事）。
///
/// - `_last.png` 是**渲完之后抽出来的**，给下一镜接续用（结果）。
/// - `_end.png` 是**渲之前放进去的**，告诉这一镜该落到哪（输入）。
///
/// 两个名字差一个字，含义相反 —— 混了就会拿上一次的结果当这一次的目标。
fn end_frame_path(shots_dir: &Path, shot_id: &str) -> PathBuf {
    shots_dir.join(format!("{shot_id}_end.png"))
}

#[tauri::command]
pub async fn cinema_render_shot_video(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    req: RenderShotReq,
) -> Result<RenderShotResult> {
    let root = cinema_projects_dir(&state.config_dir);
    let proj_dir = root.join(&req.project_id);
    let ep_dir = proj_dir.join("episodes").join(&req.episode_id);
    let shots_dir = ep_dir.join("shots");
    let ep_file = ep_dir.join("episode.json");
    if ep_file.is_file() {
        let ep: EpisodeData =
            serde_json::from_str(&std::fs::read_to_string(&ep_file).unwrap_or_default())
                .map_err(|e| Error::config(format!("读取分集失败：{e}")))?;
        if let Some(shot) = ep.storyboard.iter().find(|s| s.id == req.shot_id) {
            let shot_img_path = shots_dir.join(format!("{}.png", shot.id));
            // H3 新链路不需要关键帧（挂图或文生），只有旧的首帧引擎才先补出首帧
            let h3 = resolve_shot_video_engine(&state.config_dir, &req, shot.mode).is_ok_and(|e| e == "h3");
            if !h3 && shot.mode == ShotMode::FirstFrame && !shot_img_path.is_file() {
                let _ = cinema_render_shot(app.clone(), state.clone(), req.clone()).await?;
            }
        }
    }
    // 批量出片在跑时不许单镜插队：两个都会去抢 ComfyUI，而且批量那边轮到这一镜时会撞上「已经在出片」
    if crate::jobs::running(&state.config_dir, &req.project_id, &req.episode_id).iter().any(|j| j.kind == crate::jobs::JobKind::VideoBatch) {
        return Err(Error::config("这一集正在批量出片，等它跑完再单独出 —— 进度在右下角和 QUEUE 里"));
    }
    // 首帧那一步（如果补跑了）已经自己登记过一条，这里登记的是出片本身。
    render_video_registered(Some(&app), &state.config_dir, req).await
}

/// 台词烧进画面（出片设置里开了「烧字幕」时）。正常出完和续接收尾共用。
fn burn_dialogue(video: &Path, shot: &StoryboardShot) -> Result<()> {
    let line = shot.dialogue.trim();
    if line.is_empty() {
        return Ok(());
    }
    let tmp = video.with_extension("sub.mp4");
    let cues = crate::subtitle::cues_from_dialogue(line, shot.duration.max(1.0) as f64);
    crate::subtitle::burn_in(video, &cues, &tmp)?;
    std::fs::rename(&tmp, video).map_err(|e| Error::runtime(format!("字幕版替换原片失败：{e}")))
}

/// **续接收尾**：重启前交给 ComfyUI 的那一镜出完了、文件已经落到 `take.path` —— 记一条 take，和正常出完一样。
///
/// 按锁里最新的那份只改这一镜（续接的时候用户可能已经在改别的镜头了）。这一镜被删了回 `Ok(false)`。
pub(crate) fn record_resumed_take(config_dir: &Path, project_id: &str, episode_id: &str, shot_id: &str, mut take: VideoTake, burn_subtitles: bool) -> Result<bool> {
    let ep_file = cinema_projects_dir(config_dir).join(project_id).join("episodes").join(episode_id).join("episode.json");
    let video = PathBuf::from(&take.path);
    if !video.is_file() {
        return Err(Error::runtime(format!("续接收尾时视频文件不在：{}", take.path)));
    }
    if burn_subtitles {
        if let Ok(ep) = read_episode(config_dir, project_id, episode_id) {
            if let Some(shot) = ep.storyboard.iter().find(|s| s.id == shot_id) {
                burn_dialogue(&video, shot)?;
            }
        }
    }
    take.created_at = crate::jobs::now();
    let mut found = false;
    crate::episode_lock::update_file(&ep_file, |latest| {
        if let Some(target) = latest.storyboard.iter_mut().find(|s| s.id == shot_id) {
            found = true;
            if !target.takes.iter().any(|t| t.path == take.path) {
                target.takes.push(take.clone());
            }
            target.video_url = Some(take.path.clone());
            target.status = "ready".into();
        }
        Ok(())
    })
    .map_err(|e| Error::runtime(format!("续接出完了，但结果没存进分集：{e}")))?;
    Ok(found)
}

/// 出一镜，并在任务登记表里登记一条 `Video`（开始、结束都发事件）。单镜出片和批量出片共用。
async fn render_video_registered(app: Option<&tauri::AppHandle>, config_dir: &Path, req: RenderShotReq) -> Result<RenderShotResult> {
    // 出片前把 LM Studio 的后台模型卸掉，腾出二十几 GB（见 llm_memory 的注释）
    crate::llm_memory::free_for_video(config_dir);
    let (num, prompt) = shot_brief(config_dir, &req);
    let job = crate::jobs::start_exclusive(
        config_dir,
        crate::jobs::JobKind::Video,
        &req.project_id,
        &req.episode_id,
        &req.shot_id,
        num,
        &prompt,
    )
    .map_err(|running| Error::config(format!(
        "第 {} 镜已经在出片了（{} 秒前开始），等它跑完再出 —— 进度在右上角 QUEUE 里",
        running.shot_num, crate::jobs::now().saturating_sub(running.created_at))))?;
    crate::jobs::emit(app, crate::jobs::get(config_dir, &job).as_ref());
    // 每镜出几次：出片设置里的「每镜生成」。每次种子不同（同线路第 N 次种子 +N），挑一条用
    let takes = project_render_settings(config_dir, &req.project_id).takes.clamp(1, 4);
    // 在这条任务名下跑：交给 ComfyUI 时把 prompt_id 记进续接单（应用中途关掉，重开时凭它收结果，见 `render_resume`）
    let out = crate::jobs::scoped(config_dir, &job, async {
        let mut out = cinema_render_shot_video_with_config_dir(config_dir, req.clone()).await;
        for _ in 1..takes {
            if out.is_err() {
                break;
            }
            out = cinema_render_shot_video_with_config_dir(config_dir, req.clone()).await;
        }
        out
    })
    .await;
    let done = match &out {
        Ok(r) => crate::jobs::finish(config_dir, &job, Some(&r.video_url), None),
        Err(e) => crate::jobs::finish(config_dir, &job, None, Some(&e.to_string())),
    };
    crate::jobs::emit(app, done.as_ref());
    out
}

/// 批量出片的结果。
#[derive(Debug, Clone, Serialize)]
pub struct BatchRenderResult {
    pub done: usize,
    pub total: usize,
    /// 跳过的和失败的，每镜一条原因
    pub notes: Vec<String>,
}

/// 同一集里的上一镜（按镜号取，数组顺序不等于镜头顺序）。
fn prev_of<'a>(shots: &'a [StoryboardShot], shot: &StoryboardShot) -> Option<&'a StoryboardShot> {
    shots.iter().filter(|s| s.shot_num < shot.shot_num).max_by_key(|s| s.shot_num)
}

/// 这一镜现在的字段指纹（和合成时算的是同一份：上一镜、资产描述、风格都算进去）。
/// `long_takes` 是这一集设成长镜头的场号。**必须传** ——
/// 长镜头窗口的正文里多一条「台词要在重叠区之前说完」，
/// 不进指纹的话，勾上长镜头之后旧提示词不判过期，出片用的还是没那条约束的版本。
fn current_hash(shots: &[StoryboardShot], shot: &StoryboardShot, assets: &[crate::storyboard_gen::AssetRef], style: &str, long_takes: &[u32]) -> String {
    // 按这一镜的形象覆盖换过再算 —— 和合成时算指纹用的是同一份
    let assets = crate::looks::for_shot(assets, shot);
    crate::prompt_input::fields_hash(&crate::prompt_input::ShotContext {
        shot, prev: prev_of(shots, shot), assets: &assets, style,
        long_take: long_takes.contains(&shot.scene_no),
    })
}

/// 这一镜现在能不能出片（没合成、合成后改过就不能）。返回原因；`None` = 可以。
fn render_blocker(shots: &[StoryboardShot], shot: &StoryboardShot, route: crate::shot_table::PromptRoute, assets: &[crate::storyboard_gen::AssetRef], style: &str, long_takes: &[u32]) -> Option<String> {
    let p = shot.prompts.get(&route).filter(|p| !p.video.trim().is_empty());
    match p {
        None => Some(format!("还没合成{}的提示词，跳过", route.label())),
        Some(p) if !p.fields_hash.is_empty() && p.fields_hash != current_hash(shots, shot, assets, style, long_takes) => Some("合成之后改过字段、上一镜、资产或风格（或合成规则升级了），要先重新合成，跳过".into()),
        Some(_) => None,
    }
}

/// 批量出片出哪几镜（和前端 `storyboardView.batchTargets` 同一条规矩）：
/// 能出的里面还没用这条线路出过片的；都出过了就全部重出。回 `(镜头 id, 是不是重出, 不能出的镜数)`
///
/// **长镜头的场不在里面。** 勾了「这一场整段出」之后这一场的镜头仍然满足
/// 「提示词合成好了」—— 不摘的话会把它们一镜一镜再出一遍：GPU 白烧，
/// 而且出来的 take 和整段出的成片是两套东西。整段出走
/// `cinema_render_long_take`（对话里那条路暂时只出逐镜的那部分）。
pub fn batch_render_targets(config_dir: &Path, project_id: &str, episode_id: &str) -> Result<(Vec<String>, bool, usize)> {
    let ep_file = cinema_projects_dir(config_dir).join(project_id).join("episodes").join(episode_id).join("episode.json");
    let mut ep: EpisodeData = serde_json::from_str(&std::fs::read_to_string(&ep_file).map_err(|e| Error::config(format!("读不到分集：{e}")))?)
        .map_err(|e| Error::config(format!("分集文件坏了：{e}")))?;
    sync_keyframes(&ep_file.with_file_name("shots"), &mut ep);
    let route = project_route(config_dir, project_id);
    let assets = crate::storyboard_gen::load_assets(config_dir, project_id, episode_id);
    let style = episode_style(config_dir, project_id, &ep);
    let mut sorted: Vec<&StoryboardShot> = ep.storyboard.iter().collect();
    sorted.sort_by_key(|s| s.shot_num);
    let ok: Vec<&StoryboardShot> = sorted.into_iter().filter(|s| render_blocker(&ep.storyboard, s, route, &assets, &style, &ep.long_take_scenes).is_none()).collect();
    // **`blocked` 要在摘长镜头之前算。** 它的含义是「出不了的镜数」，
    // 而长镜头的镜头**不是出不了**，它们走整段出那条路 ——
    // 算进去的话界面会说「3 镜没合成」，人会去找根本不存在的问题。
    let blocked = ep.storyboard.len() - ok.len();
    // **长镜头的场整段出，不逐镜出** —— 和前端 `batchTargets` 同一条规矩
    let ok: Vec<&StoryboardShot> = ok.into_iter().filter(|s| !ep.long_take_scenes.contains(&s.scene_no)).collect();
    // **画布回填的不算「出过片」** —— 算进去的话批量出片会跳过这一镜（见 `TakeOrigin`）
    let fresh: Vec<&StoryboardShot> = ok.iter().copied()
        .filter(|s| !s.takes.iter().any(|t| t.route == route && t.origin == TakeOrigin::Render))
        .collect();
    let redo = fresh.is_empty() && !ok.is_empty();
    let ids = if fresh.is_empty() { &ok } else { &fresh }.iter().map(|s| s.id.clone()).collect();
    Ok((ids, redo, blocked))
}

/// 一镜一镜出，**某一镜失败只记下来，接着出下一镜**。`render` 注入，测试换假的。
/// 批量里的一项是**整场长镜头**时，id 写成 `scene_<场号>` —— 和长镜头任务登记的 `shot_id` 同一个写法
pub fn long_take_unit(id: &str) -> Option<u32> {
    id.strip_prefix("scene_")?.parse().ok()
}

/// `shot_ids` 里可以夹 `scene_<场号>`（见 [`long_take_unit`]）：轮到它就整场出一条长镜头（`render_scene`）。
///
/// **按给的顺序出，不把长镜头提前。** 以前前端先单独提交长镜头、再提交其余镜头的批量 ——
/// 2026-09-29 夜里批量那一下被「这一集有镜头正在出片」拒了，其余 32 镜一整夜都没排上，
/// 用户早上看到的是「为什么先出第五场」。现在一个任务从头到尾按场次出。
/// 停在暂停位上时多久看一次名单
const HOLD_POLL: std::time::Duration = if cfg!(test) { std::time::Duration::from_millis(5) } else { std::time::Duration::from_secs(5) };

#[allow(clippy::too_many_arguments)]
pub async fn render_batch<F, Fut, G, GFut>(
    config_dir: &Path,
    project_id: &str,
    episode_id: &str,
    shot_ids: &[String],
    render: F,
    render_scene: G,
    mut progress: impl FnMut(usize, usize, &str),
    // 每一镜开始前问一次：用户点了停止就不再往下出
    should_stop: impl Fn() -> bool,
    // 每一镜开始前问一次：任务现在的完整名单（「加入队列」追加在后面，见 `jobs::append_targets`）
    more: impl Fn() -> Vec<String>,
) -> Result<BatchRenderResult>
where
    F: Fn(RenderShotReq) -> Fut,
    Fut: std::future::Future<Output = Result<RenderShotResult>>,
    G: Fn(u32) -> GFut,
    GFut: std::future::Future<Output = Result<String>>,
{
    let route = project_route(config_dir, project_id);
    let mut ids: Vec<String> = shot_ids.to_vec();
    let mut done = 0;
    let mut notes = Vec::new();
    // 这一批里跳过或失败的镜头：接它们的下一镜也出不了
    let mut missed: Vec<String> = Vec::new();
    let mut next = 0;
    loop {
        // 每一镜开始前（包括跑到头时）看一眼名单：「加入队列」追加的接在后面出；
        // 没轮到的那段被重排过（`render_queue::reorder`，2026-10-10）也认 —— 只要已经出过的前缀没变。
        // 读不到任务（空名单）不认：不然一次读盘失败就把后面全清掉了
        let latest = more();
        if !latest.is_empty() && latest.len() >= next && latest[..next] == ids[..next] {
            ids = latest;
        }
        if next >= ids.len() {
            break;
        }
        let (i, total, id) = (next, ids.len(), ids[next].clone());
        let id = &id;
        next += 1;
        if should_stop() {
            notes.push(format!("已停止，后面 {} 镜没有出", total - i));
            break;
        }
        // **暂停位**（`render_queue`）：停在这里等它被撤掉，不交 ComfyUI 任何东西 —— 给名单外的测试让出 GPU
        if let Some(label) = id.strip_prefix(crate::render_queue::HOLD_PREFIX) {
            progress(i, total, &format!("暂停 · {label}"));
            loop {
                if should_stop() {
                    break;
                }
                tokio::time::sleep(HOLD_POLL).await;
                let l = more();
                if l.get(i) != Some(id) {
                    if !l.is_empty() && l.len() >= i && l[..i] == ids[..i] {
                        ids = l;
                    }
                    // 暂停位撤掉了：这个下标上现在是它后面那一项，重新轮到它
                    next = i;
                    break;
                }
            }
            continue;
        }
        if let Some(scene_no) = long_take_unit(id) {
            progress(i, total, &format!("第 {scene_no} 场 · 长镜头"));
            match render_scene(scene_no).await {
                Ok(_) => done += 1,
                Err(e) => notes.push(format!("第 {scene_no} 场长镜头出片失败：{e}")),
            }
            continue;
        }
        // 每一镜轮到时才读：前面几镜要跑几个小时，这期间用户可能改了、重新合成了后面的
        let ep_file = cinema_projects_dir(config_dir).join(project_id).join("episodes").join(episode_id).join("episode.json");
        let mut ep: Option<EpisodeData> = std::fs::read_to_string(&ep_file).ok().and_then(|t| serde_json::from_str(&t).ok());
        if let Some(e) = ep.as_mut() {
            sync_keyframes(&ep_file.with_file_name("shots"), e);
        }
        let Some(shot) = ep.as_ref().and_then(|e| e.storyboard.iter().find(|s| &s.id == id)) else {
            notes.push(format!("镜头 {id} 已经不在这一集里，跳过"));
            progress(i + 1, total, "");
            continue;
        };
        progress(i, total, &format!("第 {} 镜", shot.shot_num));
        let assets = crate::storyboard_gen::load_assets(config_dir, project_id, episode_id);
        let style = episode_style(config_dir, project_id, ep.as_ref().unwrap());
        if let Some(why) = render_blocker(&ep.as_ref().unwrap().storyboard, shot, route, &assets, &style, &ep.as_ref().unwrap().long_take_scenes) {
            notes.push(format!("第 {} 镜：{why}", shot.shot_num));
            missed.push(id.clone());
            continue;
        }
        // **接上一镜的，上一镜出不了就别再往下试。**
        //
        // 2026-09-27 按场出第 2 场：第 3 镜过期被跳过，4~7 镜都接上一镜，于是各登记了一条
        // 失败的出片任务，报的是「第 3 镜还没出片」—— 看不出根子在第 3 镜过期。
        // 这里一次说清楚是被谁卡住的，也不再往队列里塞注定失败的任务。
        if let Some(why) = chain_blocker(&ep.as_ref().unwrap().storyboard, shot, &missed) {
            notes.push(format!("第 {} 镜：{why}", shot.shot_num));
            missed.push(id.clone());
            continue;
        }
        let num = shot.shot_num;
        match render(RenderShotReq { project_id: project_id.into(), episode_id: episode_id.into(), shot_id: id.clone(), engine_config: None }).await {
            Ok(_) => done += 1,
            Err(e) => {
                notes.push(format!("第 {num} 镜出片失败：{e}"));
                missed.push(id.clone());
            }
        }
    }
    let total = ids.len();
    progress(total, total, "");
    Ok(BatchRenderResult { done, total, notes })
}

/// 这一镜要接上一镜、而上一镜没有片子可接时，说清楚卡在谁身上。`None` = 不用接或接得上。
///
/// 挂了首帧的镜头不接（画面由首帧锁住，见 [`Keyframes`]）。
/// `missed` 是这一批里已经跳过 / 失败的镜头 id —— 上一镜在里面就说「这一批没出成」，
/// 不在里面就是本来就没出过。
fn chain_blocker(shots: &[StoryboardShot], shot: &StoryboardShot, missed: &[String]) -> Option<String> {
    if shot.effective_continuity() == Continuity::None || shot.keyframes.drives_video() {
        return None;
    }
    let prev = prev_of(shots, shot)?;
    if continuity_source(prev).is_some() {
        return None;
    }
    Some(if missed.contains(&prev.id) {
        format!("接的是第 {} 镜，第 {} 镜这一批没出成，跟着跳过 —— 先解决第 {} 镜", prev.shot_num, prev.shot_num, prev.shot_num)
    } else {
        format!("要接第 {} 镜，第 {} 镜还没出片，跳过 —— 先出第 {} 镜，或者把这一镜改成「不接」", prev.shot_num, prev.shot_num, prev.shot_num)
    })
}

/// 批量出片（按项目当前线路）。**后端任务**，立即返回登记的任务。
///
/// 用户要求：点一下就能去睡觉，某一镜失败不阻断后面的。每一镜仍然各自登记一条出片任务。
#[tauri::command]
pub fn cinema_render_episode_videos(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    project_id: String,
    episode_id: String,
    shot_ids: Vec<String>,
) -> Result<crate::jobs::Job> {
    start_render_batch(Some(app), state.config_dir.clone(), project_id, episode_id, shot_ids)
}

/// 一场整段出（长镜头）。见 [`crate::long_take`]。
///
/// # 为什么不复用逐镜出片那条路
///
/// 那条路的单位是「一镜 = 一次提交」，而长镜头是「一场 = 一次提交」。
/// 硬掰到一起要么给每一镜塞一个「其实我是别人的一部分」的标记，
/// 要么在批量循环里特判 —— 两种都会让本来就长的那段更难读。
///
/// # 产物整条记在场上
///
/// 出来的是**整场**的片子，记成「第 N 镜的一次 take」是假数据（点开第 1 镜却是整场）。
/// 原来只放进任务的 `output` —— 分镜页看不到、导出 FCP 带不上、批量出片认不出这一场出过
/// （2026-09-30 第 5 场出完，再点批量会把它重跑 9 个小时）。
/// 现在记进 [`EpisodeData::scene_renders`]：分镜页那一场显示它，FCP 这一场放整条，批量跳过出过的场。
/// **不切开**（那天切过一次，用户：H3 自己分好了、生成完组装上，又给切开 —— 白折腾）。
#[tauri::command]
pub fn cinema_render_long_take(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    project_id: String,
    episode_id: String,
    scene_no: u32,
) -> Result<crate::jobs::Job> {
    let config_dir = state.config_dir.clone();
    if crate::jobs::running(&config_dir, &project_id, &episode_id).iter().any(|j| j.kind == crate::jobs::JobKind::Video || j.kind == crate::jobs::JobKind::VideoBatch) {
        return Err(Error::config("这一集有东西正在出片，等它跑完"));
    }
    let job = register_long_take(&config_dir, &project_id, &episode_id, scene_no)?;
    let started = crate::jobs::get(&config_dir, &job.job_id).ok_or_else(|| Error::runtime("任务登记后读不回来"))?;
    crate::jobs::emit(Some(&app), Some(&started));
    tauri::async_runtime::spawn(async move {
        let _ = run_registered_long_take(Some(&app), &config_dir, &project_id, &episode_id, scene_no, job).await;
    });
    Ok(started)
}

/// 登记好、还没开跑的一条长镜头任务
struct LongTakeJob {
    job_id: String,
    plan: crate::long_take::LongTakePlan,
    route: crate::shot_table::PromptRoute,
}

/// 登记一条长镜头的出片任务（`shot_id` = `scene_<场号>`）。**排不成窗口链的直接报错，不登记** ——
/// 注定失败的任务不进队列。单独的「整段出片」按钮和批量出片都走这里
fn register_long_take(config_dir: &Path, project_id: &str, episode_id: &str, scene_no: u32) -> Result<LongTakeJob> {
    let ep = read_episode(config_dir, project_id, episode_id)?;
    let shots: Vec<&StoryboardShot> = ep.storyboard.iter().filter(|s| s.scene_no == scene_no).collect();
    let frames = ep.long_take_context_frames.unwrap_or(crate::long_take::CONTEXT_FRAMES);
    let plan = crate::long_take::plan_with(&shots, frames).map_err(|e| Error::config(e.message()))?;
    let route = project_route(config_dir, project_id);
    let title = format!("长镜头 · 第 {scene_no} 场 · {} 个窗口 × {:.1} 秒", plan.shot_ids.len(), plan.window_secs);
    let job_id = crate::jobs::start_exclusive(config_dir, crate::jobs::JobKind::Video, project_id, episode_id, &format!("scene_{scene_no}"), 0, &title)
        .map_err(|_| Error::config("这一场已经在出了"))?;
    Ok(LongTakeJob { job_id, plan, route })
}

/// 跑一条登记好的长镜头，跑完把结果记回任务（产物路径在任务的 `output` 上，见上面「产物不记在任何一镜名下」）
async fn run_registered_long_take(app: Option<&tauri::AppHandle>, config_dir: &Path, project_id: &str, episode_id: &str, scene_no: u32, job: LongTakeJob) -> Result<String> {
    // 和逐镜出片一样，先把 LM Studio 的后台模型卸掉（见 llm_memory）
    crate::llm_memory::free_for_video(config_dir);
    let out = run_long_take(config_dir, project_id, episode_id, scene_no, job.route, &job.plan).await;
    let done = match &out {
        Ok(path) => crate::jobs::finish_with(config_dir, &job.job_id, Some(path), Some(&format!("长镜头 {:.1} 秒", job.plan.net_secs)), &[], None),
        Err(e) => crate::jobs::finish(config_dir, &job.job_id, None, Some(&e.to_string())),
    };
    crate::jobs::emit(app, done.as_ref());
    out
}

async fn run_long_take(
    config_dir: &Path,
    project_id: &str,
    episode_id: &str,
    scene_no: u32,
    route: crate::shot_table::PromptRoute,
    plan: &crate::long_take::LongTakePlan,
) -> Result<String> {
    let ep = read_episode(config_dir, project_id, episode_id)?;
    let assets = crate::storyboard_gen::load_assets(config_dir, project_id, episode_id);
    let shots_dir = cinema_projects_dir(config_dir).join(project_id).join("episodes").join(episode_id).join("shots");
    std::fs::create_dir_all(&shots_dir).ok();
    let render = project_render_settings(config_dir, project_id);
    let aspect = serde_json::from_str::<CinemaProject>(
        &std::fs::read_to_string(cinema_projects_dir(config_dir).join(project_id).join("project.json"))
            .map_err(|e| Error::config(format!("读不到项目：{e}")))?,
    )
    .map(|p| p.aspect_ratio)
    .unwrap_or_else(|_| "9:16".into());
    let (width, height) = render.size(&aspect);
    let style = episode_style(config_dir, project_id, &ep);
    let (windows, records) = long_take_windows(config_dir, project_id, episode_id, &ep, plan, route, &assets, &style, &shots_dir)?;

    let out = shots_dir.join(format!("longtake_s{scene_no}-{}.mp4", crate::jobs::now()));
    let seed = shot_seed(project_id, episode_id, &format!("scene_{scene_no}"));
    crate::agent_runtime::video_tool::render_long_take(
        &windows, &out, plan.window_secs as f64, plan.context_frames,
        width, height, seed, render.generate_audio, Some(render.quality.tier()), &[],
    )
    .await
    .map_err(Error::runtime)?;
    let quality = format!("长镜头 · {} · {width}×{height}", render.quality.label());
    record_scene_render(config_dir, project_id, episode_id, scene_no, &out, &records, route, seed, &quality)?;
    Ok(out.to_string_lossy().to_string())
}

/// 每个窗口一组（提示词，参考图）。**顺序就是镜号顺序**（plan 已排好）。
/// 记到各镜名下的东西也在这里量好 —— 跑几个小时期间用户可能改了字段，记的得是当时发出去的那份
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn long_take_windows(
    config_dir: &Path,
    project_id: &str,
    episode_id: &str,
    ep: &EpisodeData,
    plan: &crate::long_take::LongTakePlan,
    route: crate::shot_table::PromptRoute,
    assets: &[crate::storyboard_gen::AssetRef],
    style: &str,
    shots_dir: &Path,
) -> Result<(Vec<(String, Vec<PathBuf>)>, Vec<LongTakeWindow>)> {
    let mut windows = Vec::new();
    let mut records = Vec::new();
    for id in &plan.shot_ids {
        let shot = ep.storyboard.iter().find(|s| &s.id == id)
            .ok_or_else(|| Error::config(format!("镜头 {id} 不在这一集里")))?;
        let prompt = video_prompt_for_render(shot, route).map_err(|e| {
            Error::config(format!("第 {} 镜还没合成{}的提示词：{e}", shot.shot_num, route.label()))
        })?;
        // 和逐镜出片同一道关：合成之后改过字段（含勾上长镜头 —— 正文少一条「台词在重叠区之前说完」）就不出，
        // 不然一跑几个小时，出来的是旧提示词（2026-10-03 审查查出：整段出片原来只看提示词非空）
        if let Some(why) = render_blocker(&ep.storyboard, shot, route, assets, style, &ep.long_take_scenes) {
            return Err(Error::config(format!("第 {} 镜{}", shot.shot_num, why.trim_end_matches("，跳过"))));
        }
        let refs = render_reference_paths(config_dir, project_id, episode_id, shots_dir, shot, assets, ep.long_take_scenes.contains(&shot.scene_no));
        render_refs_guard(shot.shot_num, &prompt, refs.len(), false)?;
        records.push(LongTakeWindow {
            shot_id: id.clone(),
            prompt: prompt.clone(),
            fields_hash: current_hash(&ep.storyboard, shot, assets, style, &ep.long_take_scenes),
            picture_paths: refs.iter().map(|p| p.to_string_lossy().to_string()).collect(),
        });
        windows.push((prompt, refs));
    }
    Ok((windows, records))
}

/// 长镜头一个窗口要记到哪一镜名下
pub(crate) struct LongTakeWindow {
    pub shot_id: String,
    /// 这一窗口发出去的提示词全文
    pub prompt: String,
    /// 开跑时这一镜的字段指纹（和逐镜出片取 `storyboard_snapshot` 同一个道理）
    pub fields_hash: String,
    pub picture_paths: Vec<String>,
}

/// 出好的长镜头**整条记到这一场名下**（[`EpisodeData::scene_renders`]）。不切开 —— 见那里
#[allow(clippy::too_many_arguments)]
pub(crate) fn record_scene_render(
    config_dir: &Path,
    project_id: &str,
    episode_id: &str,
    scene_no: u32,
    full: &Path,
    windows: &[LongTakeWindow],
    route: crate::shot_table::PromptRoute,
    seed: u64,
    quality: &str,
) -> Result<()> {
    let render = SceneRender {
        scene_no,
        route,
        path: full.to_string_lossy().to_string(),
        created_at: crate::jobs::now(),
        seed,
        quality: quality.to_string(),
        shot_ids: windows.iter().map(|w| w.shot_id.clone()).collect(),
        prompts: windows.iter().map(|w| w.prompt.clone()).collect(),
        fields_hashes: windows.iter().map(|w| w.fields_hash.clone()).collect(),
        picture_paths: windows.iter().map(|w| w.picture_paths.clone()).collect(),
    };
    crate::episode_lock::update(config_dir, project_id, episode_id, |ep| {
        ep.scene_renders.push(render);
        Ok(())
    })
}

/// [`cinema_render_episode_videos`] 的本体（Chat 的 `studio_render_episode` 也走这里）
pub fn start_render_batch(app: Option<tauri::AppHandle>, config_dir: PathBuf, project_id: String, episode_id: String, shot_ids: Vec<String>) -> Result<crate::jobs::Job> {
    start_render_batch_with(app, config_dir, project_id, episode_id, shot_ids, false)
}

/// **加入出片队列**（2026-10-05 用户：「有镜头正在出片无法点击重新出片，但是我不可能一直等着」）。
///
/// - 这一集有批量在跑：追加到它后面（[`crate::jobs::append_targets`]），轮到了照常出；
/// - 只有单镜在出：开一个批量，**等那一镜出完再开始**；之后再加的都追加到这个批量后面。
///
/// ComfyUI 一次只跑一个，排队就是排在同一条任务后面，不另起一条去抢。
pub fn queue_render(app: Option<tauri::AppHandle>, config_dir: PathBuf, project_id: String, episode_id: String, shot_ids: Vec<String>) -> Result<crate::jobs::Job> {
    if shot_ids.is_empty() {
        return Err(Error::config("没有要出片的镜头"));
    }
    let append = |config_dir: &Path| {
        let batch = crate::jobs::running(config_dir, &project_id, &episode_id).into_iter().find(|j| j.kind == crate::jobs::JobKind::VideoBatch)?;
        crate::jobs::append_targets(config_dir, &batch.id, &shot_ids)
    };
    if let Some((job, _)) = append(&config_dir) {
        crate::jobs::emit(app.as_ref(), Some(&job));
        return Ok(job);
    }
    match start_render_batch_with(app.clone(), config_dir.clone(), project_id.clone(), episode_id.clone(), shot_ids.clone(), true) {
        Ok(job) => Ok(job),
        // 同一瞬间别人刚开了批量：追加到那一条
        Err(e) => match append(&config_dir) {
            Some((job, _)) => {
                crate::jobs::emit(app.as_ref(), Some(&job));
                Ok(job)
            }
            None => Err(e),
        },
    }
}

#[tauri::command]
pub fn cinema_queue_shot_videos(app: tauri::AppHandle, state: State<'_, AppState>, project_id: String, episode_id: String, shot_ids: Vec<String>) -> Result<crate::jobs::Job> {
    queue_render(Some(app), state.config_dir.clone(), project_id, episode_id, shot_ids)
}

/// `wait_for_video`：有单镜在出时不拒绝，登记好之后等它出完再开始（加入队列用）。
fn start_render_batch_with(app: Option<tauri::AppHandle>, config_dir: PathBuf, project_id: String, episode_id: String, shot_ids: Vec<String>, wait_for_video: bool) -> Result<crate::jobs::Job> {
    if shot_ids.is_empty() {
        return Err(Error::config("没有要出片的镜头"));
    }
    if !wait_for_video && crate::jobs::running(&config_dir, &project_id, &episode_id).iter().any(|j| j.kind == crate::jobs::JobKind::Video) {
        return Err(Error::config("这一集有镜头正在出片，等它跑完再批量出"));
    }
    let route = project_route(&config_dir, &project_id);
    let scenes = shot_ids.iter().filter(|id| long_take_unit(id).is_some()).count();
    let title = match (shot_ids.len() - scenes, scenes) {
        (n, 0) => format!("{} · {n} 镜", route.label()),
        (0, k) => format!("{} · {k} 场长镜头", route.label()),
        (n, k) => format!("{} · {n} 镜 + {k} 场长镜头", route.label()),
    };
    let job_id = crate::jobs::start_exclusive(&config_dir, crate::jobs::JobKind::VideoBatch, &project_id, &episode_id, "", 0, &title)
        .map_err(|_| Error::config("这一集已经在批量出片了 —— 进度在右下角和 QUEUE 里"))?;
    let started = crate::jobs::set_targets(&config_dir, &job_id, &shot_ids).ok_or_else(|| Error::runtime("任务登记后读不回来"))?;
    crate::jobs::emit(app.as_ref(), Some(&started));

    tauri::async_runtime::spawn(async move {
        // 加入队列开的批量：前面那一镜（单镜出片 / 重启后续接的）出完再开始，不和它抢 ComfyUI
        if wait_for_video {
            let mut first = true;
            while crate::jobs::running(&config_dir, &project_id, &episode_id).iter().any(|j| j.kind == crate::jobs::JobKind::Video) {
                if crate::jobs::cancel_requested(&config_dir, &job_id) {
                    break;
                }
                if first {
                    let job = crate::jobs::set_progress(&config_dir, &job_id, 0, shot_ids.len(), "排队中 · 等前一镜出完");
                    crate::jobs::emit(app.as_ref(), job.as_ref());
                    first = false;
                }
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
            // **再等 ComfyUI 连得上**：重启后续接时 ComfyUI 可能还没起来 —— 不等的话一镜接一镜「连不上」秒败，整批作废。
            // 等 30 分钟还连不上就停，说清楚
            let since = std::time::Instant::now();
            let mut said = false;
            while !crate::agent_runtime::video_tool::comfy_reachable().await {
                if crate::jobs::cancel_requested(&config_dir, &job_id) {
                    break;
                }
                if since.elapsed() > std::time::Duration::from_secs(30 * 60) {
                    let done = crate::jobs::finish_with(&config_dir, &job_id, None, None, &[], Some(&format!(
                        "等了 30 分钟 ComfyUI 还连不上，这 {} 镜没有出 —— 先把 ComfyUI 开起来（anima start），再点一次出片",
                        shot_ids.len()
                    )));
                    crate::jobs::emit(app.as_ref(), done.as_ref());
                    return;
                }
                if !said {
                    let job = crate::jobs::set_progress(&config_dir, &job_id, 0, shot_ids.len(), "排队中 · 等 ComfyUI 起来");
                    crate::jobs::emit(app.as_ref(), job.as_ref());
                    said = true;
                }
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
        }
        let render = |req: RenderShotReq| {
            let app = app.clone();
            let config_dir = config_dir.clone();
            async move { render_video_registered(app.as_ref(), &config_dir, req).await }
        };
        let render_scene = |scene_no: u32| {
            let app = app.clone();
            let (config_dir, project_id, episode_id) = (config_dir.clone(), project_id.clone(), episode_id.clone());
            async move {
                let job = register_long_take(&config_dir, &project_id, &episode_id, scene_no)?;
                crate::jobs::emit(app.as_ref(), crate::jobs::get(&config_dir, &job.job_id).as_ref());
                run_registered_long_take(app.as_ref(), &config_dir, &project_id, &episode_id, scene_no, job).await
            }
        };
        let out = render_batch(
            &config_dir,
            &project_id,
            &episode_id,
            &shot_ids,
            render,
            render_scene,
            |done, total, label| {
                let job = crate::jobs::set_progress(&config_dir, &job_id, done, total, label);
                crate::jobs::emit(app.as_ref(), job.as_ref());
            },
            || crate::jobs::cancel_requested(&config_dir, &job_id),
            || crate::jobs::get(&config_dir, &job_id).map(|j| j.targets).unwrap_or_default(),
        )
        .await;
        // 收尾那一瞬间才加进队列的（名单已经读完）：说出来，别默默丢掉
        let out = out.map(|mut r| {
            let late = crate::jobs::get(&config_dir, &job_id).map_or(0, |j| j.targets.len().saturating_sub(r.total));
            if late > 0 {
                r.notes.push(format!("有 {late} 镜在这一批收尾时才加入队列，没赶上 —— 重新点一次出片"));
            }
            r
        });
        let finished = match out {
            Ok(r) if r.done == 0 => crate::jobs::finish_with(&config_dir, &job_id, None, None, &r.notes, Some(&format!("{} 镜一镜都没出成：{}", r.total, r.notes.join("；")))),
            Ok(r) => crate::jobs::finish_with(&config_dir, &job_id, None, Some(&format!("出片 {}/{} 镜", r.done, r.total)), &r.notes, None),
            Err(e) => crate::jobs::finish_with(&config_dir, &job_id, None, None, &[], Some(&e.to_string())),
        };
        crate::jobs::emit(app.as_ref(), finished.as_ref());
    });
    Ok(started)
}

fn resolve_shot_video_engine(
    config_dir: &Path,
    req: &RenderShotReq,
    mode: ShotMode,
) -> Result<String> {
    let engine = if let Some(config) = &req.engine_config {
        config.video_engine.clone()
    } else if mode == ShotMode::Reference {
        // The reference renderer currently supports H3 only.
        "h3".into()
    } else {
        let file = cinema_projects_dir(config_dir).join(&req.project_id).join("project.json");
        let text = std::fs::read_to_string(file)
            .map_err(|e| Error::config(format!("读取项目生成设置失败：{e}")))?;
        let project: CinemaProject = serde_json::from_str(&text)
            .map_err(|e| Error::config(format!("解析项目生成设置失败：{e}")))?;
        project.model_settings.video_engine
    };
    let engine = match engine.as_str() {
        "minimax_h3" => "h3",
        "ltx25" | "ltx2.5" => "ltx",
        other => other,
    };
    if mode == ShotMode::Reference && engine != "h3" {
        return Err(Error::config("参考生视频当前只支持 MiniMax H3，请切换模型或使用首帧模式"));
    }
    if !matches!(engine, "h3" | "ltx" | "liveportrait" | "camera_preview") {
        return Err(Error::config(format!("不支持的视频引擎：{engine}")));
    }
    Ok(engine.to_string())
}

/// 超长镜的线路 B 建议：拆回各段（和合成时同一套 [`crate::long_shot::windows`]），一段一段查，前面标第几段。
/// 段数和正文对不上的不查 —— 那种出片前会拦下（见出片的超长镜分支），这里再报一遍只是重复
fn long_shot_advisories(
    shot: &StoryboardShot,
    prev: Option<&StoryboardShot>,
    assets: &[crate::storyboard_gen::AssetRef],
    style: &str,
    p: &crate::shot_table::RoutePrompts,
) -> Vec<String> {
    let windows = crate::long_shot::windows(shot);
    let parts = crate::long_shot::split_prompt(&p.video);
    if parts.len() != windows.len() {
        return Vec::new();
    }
    let n = windows.len();
    let mut out = Vec::new();
    for (k, (win, part)) in windows.iter().zip(parts).enumerate() {
        let ctx = crate::prompt_input::ShotContext {
            shot: win,
            prev: if k == 0 { prev } else { Some(&windows[k - 1]) },
            assets,
            style,
            long_take: k + 1 < n,
        };
        let one = crate::shot_table::RoutePrompts { video: part, ..p.clone() };
        out.extend(crate::prompt_route_b::advisories(&ctx, &one).into_iter().map(|m| format!("超长镜第 {}/{n} 段：{m}", k + 1)));
    }
    out
}

/// 校验一集的镜头表。只检查，不补内容 —— 规则见 [`crate::shot_table::validate`]，
/// 资产名换 id 在 [`crate::storyboard_gen::validate_shots`]，生成和界面共用这一份。
pub fn validate_storyboard(config_dir: &Path, project_id: &str, episode_id: &str, ep: &EpisodeData) -> Vec<crate::shot_table::ShotIssue> {
    let assets = crate::storyboard_gen::load_assets(config_dir, project_id, episode_id);
    let mut issues = crate::storyboard_gen::validate_shots(&ep.storyboard, &assets);
    issues.extend(scene_coverage_issues(ep));
    let style = episode_style(config_dir, project_id, &ep);
    let route = project_route(config_dir, project_id);
    issues.extend(stale_prompt_issues(&ep.storyboard, &assets, &style, route, &ep.long_take_scenes));
    // **三级过期分别标**：提示词过期 / 片子过期 / 成片过期。
    // 「有 take」不等于「按当前版本出过片」（2026-09-19 Codex Astra 指出）。
    issues.extend(stale_take_issues(&ep.storyboard, &assets, &style, route, &ep.long_take_scenes));
    issues.extend(unmounted_ref_issues(&ep.storyboard, &assets));
    issues.extend(timeline_issues(&ep.storyboard, &assets));
    // 首帧出片却有人中途进场：**挡出片**（字段 `route`，前端 `renderBlocker` 认它），出片入口另有一道同样的拦
    for shot in &ep.storyboard {
        if let Some(why) = crate::shot_route::keyframe_blocker(shot, &assets, ep.long_take_scenes.contains(&shot.scene_no)) {
            issues.push(crate::shot_table::ShotIssue { shot_num: shot.shot_num, field: "route", message: why });
        }
    }
    // **只提示不阻断的那些**（表演指示丢了、说话人可能串了、英文二选一）。
    //
    // 它们是启发式，会误报 —— 放进 `check` 的话误报就是白烧一次生成
    // （`check_wording` 里有 `out.extend(check(...))`，重试三次后整镜报错）。
    // 放这里：界面上看得见，合成照常成功。
    if route == crate::shot_table::PromptRoute::B {
        for shot in &ep.storyboard {
            let Some(p) = shot.prompts.get(&route).filter(|p| !p.video.trim().is_empty()) else { continue };
            let prev = ep.storyboard.iter().filter(|s| s.shot_num < shot.shot_num).max_by_key(|s| s.shot_num);
            // **套这一镜选的造型**（`looks::for_shot`），和合成、出片同一份 —— 10-07 用项目默认的主形象查，
            // EP03 第 1~22 镜明明都选了晚礼服，每个星澜镜头都报「没选造型」
            let shot_assets = crate::looks::for_shot(&assets, shot);
            let assets = &shot_assets;
            let ctx = crate::prompt_input::ShotContext { shot, prev, assets, style: &style, long_take: false };
            // **超长镜按段查。** 每段是单独合成的，(S1)(S2) 各段从头编 —— 第 2 段第一个开口的星澜在那一段里就是 (S1)。
            // 拼起来按一整镜对，三句全报「说话人串了」（2026-10-05 EP03 第 20 镜，误报）
            let msgs = if crate::long_shot::is_long(shot.duration) {
                long_shot_advisories(shot, prev, assets, &style, p)
            } else {
                crate::prompt_route_b::advisories(&ctx, p)
            };
            for m in msgs {
                // **字段是 `advice`，不是 `prompts`。**
                //
                // 2026-09-20 出过事，而且是同一个错误犯了第二遍：我把这三条
                // 启发式检查从 `check`（会让合成重试三次后报错）里挪出来，
                // 以为「放进 issues 就只是提示」—— 而前端把
                // **任何 `field == "prompts"` 的 issue 都当成「合成之后改过字段，
                // 要先重新合成」**，于是它不进批量出片、整集出不了片
                // （`storyboardView.renderBlocker` / `batchTargets`）。
                //
                // 一条已知会误报的启发式，挂在一个会挡住出片的字段上 ——
                // 比放在 `check` 里还糟，因为它连错误信息都是错的
                // （显示「改过字段或资产图」，而真正原因是「正文里有个 or」）。
                issues.push(crate::shot_table::ShotIssue { shot_num: shot.shot_num, field: "advice", message: m });
            }
        }
    }
    issues
}

/// **这一镜写了、资产库里有图、却没挂成参考图的资产**（人物 / 场景 / 道具）。
///
/// # 为什么（2026-10-06 EP03 第 3 镜）
///
/// 「白色悬浮车（吴峥）」在出场里、道具库里也有图，可这一镜挂的 6 张参考图里没有它 ——
/// 镜头表是 10-03 生成的，车的图 10-04 才出，生成时没图可挂，之后也没人补。
/// 提示词里它只剩一个词 `hovercar`，H3 出成了一辆四轮轿车。全剧扫下来 28 镜是这样。
///
/// 不算的：只出声的人物（台词全是画外、开场没写到他）；首帧出片时的人物（长相只认首帧，本来就不挂）。
pub(crate) fn unmounted_assets<'a>(shot: &StoryboardShot, assets: &'a [crate::storyboard_gen::AssetRef]) -> Vec<&'a crate::storyboard_gen::AssetRef> {
    let mut ids = shot.all_character_ids();
    ids.extend(shot.scene_id.clone());
    ids.extend(shot.prop_ids.iter().cloned());
    let mut out: Vec<&crate::storyboard_gen::AssetRef> = Vec::new();
    for id in &ids {
        let Some(a) = assets.iter().find(|a| &a.id == id && a.has_image) else { continue };
        if shot.ref_slots.iter().any(|r| r.asset_id == a.id) || out.iter().any(|o| o.id == a.id) {
            continue;
        }
        if a.kind == crate::assets::AssetKind::Character
            && (shot.keyframes.drives_video() || shot.absent_from_opening(&a.name) == Some(OpeningAbsence::VoiceOnly))
        {
            continue;
        }
        out.push(a);
    }
    out
}

/// 问题原文里认这一条用的那半句（[`issue_fixes`] 按它挂「挂上」按钮）
const UNMOUNTED_MARK: &str = "有参考图，但这一镜没挂上";

/// 「有参考图，但这一镜没挂上」那条问题的「挂上参考图」按钮。挂满了不给（先摘一张）。
fn mount_fix(shot: &StoryboardShot, assets: &[crate::storyboard_gen::AssetRef], message: &str) -> Option<IssueFix> {
    use crate::assets::AssetKind;
    use crate::shot_table::RefUsage;
    if shot.ref_slots.len() >= crate::agent_runtime::video_tool::MAX_REFERENCE_IMAGES {
        return None;
    }
    let a = unmounted_assets(shot, assets).into_iter().find(|a| message.starts_with(&format!("「{}」", a.name)))?;
    let usage = if a.kind == AssetKind::Scene { RefUsage::Geography } else { RefUsage::Identity };
    let control = match a.kind {
        AssetKind::Scene => "空间布局、陈设与材质".to_string(),
        AssetKind::Character => "脸型、发型、体型与服装".to_string(),
        _ => format!("{}的外形、颜色与材质", a.name),
    };
    Some(IssueFix::Mount { asset_id: a.id.clone(), usage, control })
}

/// 时间线里 H3 会照着画错的两类写法（[`crate::timeline_lint`]）：机位背后的人写了看不见的部位；剧本这一镜没有的人或进场。
/// 字段 `advice`：只提示、不挡出片 —— 两条都是按词判的启发式。
fn timeline_issues(shots: &[StoryboardShot], assets: &[crate::storyboard_gen::AssetRef]) -> Vec<crate::shot_table::ShotIssue> {
    use crate::assets::AssetKind;
    let library: Vec<&str> = assets.iter().filter(|a| a.kind == AssetKind::Character).map(|a| base_character_name(&a.name)).collect();
    let mut out = Vec::new();
    for shot in shots {
        let ids = shot.all_character_ids();
        let cast: Vec<&str> = assets.iter().filter(|a| a.kind == AssetKind::Character && ids.contains(&a.id)).map(|a| base_character_name(&a.name)).collect();
        let note = shot.camera_prompt_note();
        for (seg, name, word, clause) in crate::timeline_lint::hidden_side_actions(&note, &shot.segments, &cast) {
            out.push(crate::shot_table::ShotIssue {
                shot_num: shot.shot_num,
                field: "advice",
                message: format!(
                    "第 {seg} 段：机位在「{name}」背后，时间线却写了他的「{word}」（「{clause}」）—— 镜头看不到的部位，H3 会另画一个正脸的{name}来演（EP04 第 7 镜多出第二个老秦）。背对镜头只写重心、肩颈、呼吸，或者让运镜绕到他身前"
                ),
            });
        }
        for m in crate::timeline_lint::unsourced_people(&shot.source, &shot.segments, &cast, &library) {
            out.push(crate::shot_table::ShotIssue { shot_num: shot.shot_num, field: "advice", message: format!("{m} —— 多半是写时间线时加出来的，H3 会照着多画一个人。剧本里确实有就补进出场 / 剧本，没有就从时间线删掉") });
        }
        // 表演合同 / 打斗（10-12）：只查填了合同、交换的段，不挡出片
        for m in crate::timeline_lint::performance_advice(shot).into_iter().chain(crate::timeline_lint::combat_advice(shot)) {
            out.push(crate::shot_table::ShotIssue { shot_num: shot.shot_num, field: "advice", message: m });
        }
    }
    out
}

fn unmounted_ref_issues(shots: &[StoryboardShot], assets: &[crate::storyboard_gen::AssetRef]) -> Vec<crate::shot_table::ShotIssue> {
    let mut out = Vec::new();
    for shot in shots {
        let full = shot.ref_slots.len() >= crate::agent_runtime::video_tool::MAX_REFERENCE_IMAGES;
        for a in unmounted_assets(shot, assets) {
            let tail = if full {
                format!("这一镜已经挂满 {} 张，要挂就先摘一张", crate::agent_runtime::video_tool::MAX_REFERENCE_IMAGES)
            } else {
                "出片时只按文字写，样子由模型自己编（EP03 第 3 镜的悬浮车就出成了四轮轿车）".to_string()
            };
            // 字段是 `advice`：只提示、不挡出片（挡出片的字段见 `validate_storyboard` 末尾那段注释）
            out.push(crate::shot_table::ShotIssue { shot_num: shot.shot_num, field: "advice", message: format!("「{}」{UNMOUNTED_MARK} —— {tail}", a.name) });
        }
    }
    out
}

/// 整场检查：剧本每句都有镜头拍到、台词都进了台词栏、原文逐字出自剧本。
///
/// 这两项要对照整场剧本，以前只在生成镜头表时做，用户手动改了字段不会重查（2026-09-14 用户要求补上）。
/// **只在能按剧本标题重新切出同样几场时做**：场是模型切的（剧本没写标题）就对不上是哪一场的原文，不乱报。
fn scene_coverage_issues(ep: &EpisodeData) -> Vec<crate::shot_table::ShotIssue> {
    // **单场剧本也要查。**
    //
    // `split_by_headings` 在切不出两场时回 `None`（没有标题，或全篇一个地点），
    // 原来就整个跳过 —— 于是 E01~E03 这种单场剧本**编辑之后完全没被复查过**
    // （2026-09-19 Codex Astra 指出）。
    //
    // 单场时整篇就是那一场，直接拿全文比：不需要知道「是哪一场」。
    let Some(scenes) = crate::storyboard_gen::split_by_headings(&ep.script_text) else {
        let mut shots = ep.storyboard.clone();
        if shots.is_empty() || ep.script_text.trim().is_empty() {
            return Vec::new();
        }
        shots.sort_by_key(|s| s.shot_num);
        let mut out = crate::storyboard_gen::dialogue_coverage(&ep.script_text, &shots);
        out.extend(crate::storyboard_gen::source_coverage(&ep.script_text, &shots));
        out.extend(crate::storyboard_gen::shot_density(&shots));
        return out;
    };
    let max_scene = ep.storyboard.iter().map(|s| s.scene_no).max().unwrap_or(0) as usize;
    if max_scene != scenes.len() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (i, scene) in scenes.iter().enumerate() {
        let mut shots: Vec<StoryboardShot> = ep.storyboard.iter().filter(|s| s.scene_no as usize == i + 1).cloned().collect();
        if shots.is_empty() {
            continue;
        }
        shots.sort_by_key(|s| s.shot_num);
        out.extend(crate::storyboard_gen::dialogue_coverage(&scene.text, &shots));
        out.extend(crate::storyboard_gen::source_coverage(&scene.text, &shots));
    }
    out
}

/// 这一镜在 `route` 上的提示词是不是过期了（合成过、之后字段又改过）。
///
/// 没合成过、旧数据没有指纹（不知道改没改过）都**不算**过期 —— 不乱报。
fn prompt_stale(shots: &[StoryboardShot], s: &StoryboardShot, assets: &[crate::storyboard_gen::AssetRef], style: &str, route: crate::shot_table::PromptRoute, long_takes: &[u32]) -> bool {
    s.prompts
        .get(&route)
        .is_some_and(|p| !p.video.trim().is_empty() && !p.fields_hash.is_empty() && p.fields_hash != current_hash(shots, s, assets, style, long_takes))
}

/// 过期是不是**只因为**这一镜的首帧 / 尾帧变了（刚出了首帧、删了尾帧……）。
///
/// 出了首帧，线路 B 要按首帧重写（人物改成从首帧里认，不再按参考图定义），指纹里算了关键帧，判过期是对的。
/// 但提示原来只说「字段、上一镜、资产或风格改过」—— 用户刚出完首帧看到，以为出了什么问题（2026-09-29 截图）。
/// 做法：把关键帧换成别的几种组合重算指纹，和合成时的对上了，就说明别的都没动、只动了关键帧。
fn stale_only_by_keyframes(shots: &[StoryboardShot], s: &StoryboardShot, assets: &[crate::storyboard_gen::AssetRef], style: &str, route: crate::shot_table::PromptRoute, long_takes: &[u32]) -> bool {
    let Some(p) = s.prompts.get(&route) else { return false };
    [(false, false), (true, false), (false, true), (true, true)]
        .into_iter()
        .filter(|&(f, l)| (f, l) != (s.keyframes.first, s.keyframes.last))
        .any(|(first, last)| {
            let mut then = s.clone();
            then.keyframes = Keyframes { first, last };
            p.fields_hash == current_hash(shots, &then, assets, style, long_takes)
        })
}

/// 合成过、但之后镜头字段又改过的镜头：提示词还是按旧字段写的，出片会和表对不上。
fn stale_prompt_issues(shots: &[StoryboardShot], assets: &[crate::storyboard_gen::AssetRef], style: &str, route: crate::shot_table::PromptRoute, long_takes: &[u32]) -> Vec<crate::shot_table::ShotIssue> {
    shots
        .iter()
        .filter_map(|s| {
            if !prompt_stale(shots, s, assets, style, route, long_takes) {
                return None;
            }
            let message = if stale_only_by_keyframes(shots, s, assets, style, route, long_takes) {
                let what = match (s.keyframes.first, s.keyframes.last) {
                    (true, true) => "有了首帧和尾帧",
                    (true, false) => "有了首帧",
                    (false, true) => "有了尾帧",
                    (false, false) => "首帧 / 尾帧删掉了",
                };
                format!("这一镜{what}，{}要按关键帧重新合成一次（人物改从首帧里认）—— 点「重新合成这一镜」，其它没变", route.label())
            } else {
                format!("合成之后镜头字段、上一镜、资产或风格改过，{}的提示词要重新合成", route.label())
            };
            Some(crate::shot_table::ShotIssue { shot_num: s.shot_num, field: "prompts", message })
        })
        .collect()
}

/// 两条线路各自过期的镜头 id。
///
/// `validate_storyboard` 只报**当前线路**的过期 —— 出片只发当前线路的提示词，另一条过期不挡出片。
/// 但合成弹窗能选另一条线路，只靠那份问题清单的话，另一条线路过期了也显示「不需要合成」；
/// 而合成哪条就会把哪条存成当前线路，所以合成完 A 再点 B 必然这样
/// （2026-09-29 用户：「合成其中一个，点另一个就变成不需要了」）。
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct StalePrompts {
    pub a: Vec<String>,
    pub b: Vec<String>,
}

pub fn stale_prompts(config_dir: &Path, project_id: &str, episode_id: &str, ep: &EpisodeData) -> StalePrompts {
    use crate::shot_table::PromptRoute;
    let assets = crate::storyboard_gen::load_assets(config_dir, project_id, episode_id);
    let style = episode_style(config_dir, project_id, ep);
    let ids = |route: PromptRoute| -> Vec<String> {
        ep.storyboard
            .iter()
            .filter(|s| prompt_stale(&ep.storyboard, s, &assets, &style, route, &ep.long_take_scenes))
            .map(|s| s.id.clone())
            .collect()
    };
    StalePrompts { a: ids(PromptRoute::A), b: ids(PromptRoute::B) }
}

/// 合成弹窗用：两条线路各有哪几镜过期。见 [`StalePrompts`]
#[tauri::command]
pub fn cinema_stale_prompts(state: State<'_, AppState>, project_id: String, episode_id: String) -> Result<StalePrompts> {
    let f = cinema_projects_dir(&state.config_dir).join(&project_id).join("episodes").join(&episode_id).join("episode.json");
    let mut ep: EpisodeData = serde_json::from_str(&std::fs::read_to_string(&f).unwrap_or_default())
        .map_err(|e| Error::config(format!("读不出 episode.json：{e}")))?;
    // 和 `cinema_validate_storyboard` 同一套整理，两边判过期用的是同一份数据
    drop_old_storyboard(&mut ep);
    sync_keyframes(&f.with_file_name("shots"), &mut ep);
    Ok(stale_prompts(&state.config_dir, &project_id, &episode_id, &ep))
}

/// 这一集实际用的画风：**锁了就用锁的，没锁跟项目走**。
///
/// 风格是项目级可变状态 —— 改了之后重合成旧的分集会按新风格出，
/// 复现不了它自己的历史风格（2026-09-19 Codex Astra 指出）。
/// `EpisodeData::style_lock` 非空时它说了算。
pub(crate) fn episode_style(config_dir: &Path, project_id: &str, ep: &EpisodeData) -> String {
    if !ep.style_lock.trim().is_empty() {
        return ep.style_lock.clone();
    }
    crate::style::load_art(config_dir, project_id).effective_positive()
}

/// 已经出过片、但那条片子是按旧版镜头表出的。
///
/// # 为什么要和「提示词过期」分开
///
/// `stale_prompt_issues` 管的是**提示词**过期（合成之后字段又改了）。
/// 但还有一层：提示词是新的、片子是旧的 —— 用户重新合成了提示词却没重出片，
/// 界面上那一镜照样有视频、照样显示「已完成」。
///
/// **「有 take」不等于「按当前版本出过片」**（2026-09-19 Codex Astra 指出）。
/// 这两件事原来混在一起，没法分别标。
///
/// 只看**最新那条 take**：更早的本来就是历史版本，不该报。
fn stale_take_issues(shots: &[StoryboardShot], assets: &[crate::storyboard_gen::AssetRef], style: &str, route: crate::shot_table::PromptRoute, long_takes: &[u32]) -> Vec<crate::shot_table::ShotIssue> {
    shots
        .iter()
        .filter_map(|s| {
            // 只看分镜页出的那些 —— 画布回填的没有指纹，会把该报的过期警告盖掉
            let last = s.takes.iter().filter(|t| t.route == route && t.origin == TakeOrigin::Render).next_back()?;
            // 老 take 没记指纹 —— 不知道是按什么出的，不乱报
            if last.fields_hash.is_empty() {
                return None;
            }
            if last.fields_hash == current_hash(shots, s, assets, style, long_takes) {
                return None;
            }
            Some(crate::shot_table::ShotIssue {
                shot_num: s.shot_num,
                field: "takes",
                message: format!("第 {} 镜已有的片子是按旧版镜头表出的 —— 现在的字段改过了，要重出才对得上", s.shot_num),
            })
        })
        .collect()
}

/// 由剧本生成镜头表（分镜重构第 3 步「确认镜头」）。**会覆盖这一集现有的镜头表。**
///
/// **后端任务，立刻返回。** 登记进任务表（落盘、QUEUE 里可见），真正的生成在后台跑；
/// 进度、结果摘要和备注都记在任务上，并通过 [`crate::jobs::EVENT`] 推给界面。
/// 用户切到别的页面、切回来都不会丢；应用重启时没跑完的会被判失败。
/// 同一集同时只能有一个。
#[tauri::command]
pub fn cinema_generate_shot_table(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    project_id: String,
    episode_id: String,
) -> Result<crate::jobs::Job> {
    start_shot_table(Some(app), state.config_dir.clone(), state.gateway.clone(), project_id, episode_id)
}

/// [`cinema_generate_shot_table`] 的本体（Chat 的 `studio_generate_shot_table` 也走这里）
pub fn start_shot_table(app: Option<tauri::AppHandle>, config_dir: PathBuf, gateway: std::sync::Arc<crate::Gateway>, project_id: String, episode_id: String) -> Result<crate::jobs::Job> {
    let title = std::fs::read_to_string(cinema_projects_dir(&config_dir).join(&project_id).join("episodes").join(&episode_id).join("episode.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<EpisodeData>(&t).ok())
        .map(|e| e.title)
        .unwrap_or_default();
    let job_id = crate::jobs::start_exclusive(&config_dir, crate::jobs::JobKind::ShotTable, &project_id, &episode_id, "", 0, &title)
        .map_err(|_| Error::config("这一集的镜头正在生成，等它跑完 —— 进度在右下角和 QUEUE 里"))?;
    let started = crate::jobs::get(&config_dir, &job_id).ok_or_else(|| Error::runtime("任务登记后读不回来"))?;
    crate::jobs::emit(app.as_ref(), Some(&started));

    let usage_scope = crate::token_usage::Scope::episode(&project_id, &episode_id);
    tauri::async_runtime::spawn(usage_scope.run(async move {
        // 出片时会把后台模型卸掉；本机 LM Studio 不会按需自动加载，开工前确认它在内存里。
        // 放在后台任务里做：加载要十几秒，放在命令里会卡住界面
        let dir = config_dir.clone();
        let loaded = tauri::async_runtime::spawn_blocking(move || crate::llm_memory::ensure_background_loaded(&dir)).await.unwrap_or_else(|e| Err(e.to_string()));
        let out = match loaded {
            Ok(()) => generate_shot_table_job(app.as_ref(), &config_dir, gateway, &project_id, &episode_id, &job_id).await,
            Err(e) => Err(Error::config(e)),
        };
        let done = match out {
            Ok(r) => {
                let summary = format!("{} 场 {} 个镜头{}", r.scenes, r.shots,
                    if r.issues.is_empty() { String::new() } else { format!("，{} 个问题待改", r.issues.len()) });
                crate::jobs::finish_with(&config_dir, &job_id, None, Some(&summary), &r.notes, None)
            }
            Err(e) => crate::jobs::finish_with(&config_dir, &job_id, None, None, &[], Some(&e.to_string())),
        };
        crate::jobs::emit(app.as_ref(), done.as_ref());
    }));
    Ok(started)
}

async fn generate_shot_table_job(
    app: Option<&tauri::AppHandle>,
    config_dir: &Path,
    gateway: std::sync::Arc<crate::Gateway>,
    project_id: &str,
    episode_id: &str,
    job_id: &str,
) -> Result<crate::storyboard_gen::ShotTableResult> {
    let ep_file = cinema_projects_dir(config_dir).join(project_id).join("episodes").join(episode_id).join("episode.json");
    let ep: EpisodeData = serde_json::from_str(&std::fs::read_to_string(&ep_file).map_err(|e| Error::config(format!("读不到分集：{e}")))?)
        .map_err(|e| Error::config(format!("解析分集失败：{e}")))?;
    let assets = crate::storyboard_gen::load_assets(config_dir, project_id, episode_id);
    let cfg = crate::prompts::load_for_project(config_dir, project_id);
    let call = |slot: crate::prompts::PromptSlot, vars: Vec<(String, String)>| {
        let gateway = gateway.clone();
        let cfg = cfg.clone();
        async move {
            let vars: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            crate::prompts::run(&gateway, &cfg, slot, &vars).await
        }
    };
    let take = |no: u32, scene: &crate::storyboard_gen::SceneChunk, count: usize| {
        crate::one_take::plan(ep.one_take, &ep.scene_takes, no, &scene.text, count, ep.target_secs)
    };
    let (shots, result) = crate::storyboard_gen::generate(&ep.script_text, &assets, call, |done, total, scene| {
        let job = crate::jobs::set_progress(config_dir, job_id, done, total, scene);
        crate::jobs::emit(app, job.as_ref());
    }, &take, ep.target_secs)
    .await?;

    // 按锁里最新的那份写：生成要几分钟，这期间用户可能改了剧本标题之类的字段。
    crate::episode_lock::update_file(&ep_file, |latest| {
        let scene_nos: Vec<u32> = shots.iter().map(|s| s.scene_no).collect();
        for no in scene_nos {
            sync_one_take_long_take(&mut latest.long_take_scenes, &shots, no);
        }
        latest.storyboard = shots;
        latest.storyboard_version = STORYBOARD_VERSION;
        Ok(())
    })?;
    // 镜头表一出来就把「接上一镜」按链路设好（不再只写「推荐」）
    auto_continuity_now(config_dir, project_id, episode_id);
    Ok(result)
}

/// 合成提示词的结果。
#[derive(Debug, Clone, Serialize)]
pub struct SynthesizeResult {
    pub route: crate::shot_table::PromptRoute,
    pub done: usize,
    pub total: usize,
    /// 没合成成功的镜头和原因。**必须说出来**，不拿不合格的存。
    pub notes: Vec<String>,
    /// **合成成功、存下来了，但有话要说**（正文偏薄、漏了几个汉字）。
    ///
    /// 和 `notes` 严格分开 —— 2026-09-20 把这些塞进 `notes` 之后，
    /// 界面上那一段的标题写死是「没通过检查（没有保存）」，
    /// 四镜明明存好了却被报成四镜失败。见 [`crate::jobs::Job::remarks`]。
    #[serde(default)]
    pub remarks: Vec<String>,
}

/// 读项目的出片设置。只取这一个键（理由同 [`project_route`]）。
fn project_render_settings(config_dir: &Path, project_id: &str) -> RenderSettings {
    std::fs::read_to_string(cinema_projects_dir(config_dir).join(project_id).join("project.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| serde_json::from_value(v["model_settings"]["render"].clone()).ok())
        .unwrap_or_default()
}

/// 读项目当前的提示词线路。读不到用默认（B）。
///
/// 只取 `model_settings.prompt_route` 这一个键，不反序列化整个项目 ——
/// 项目文件里别的字段缺一个，整份就解析失败、线路悄悄变成默认值，合成出另一条线路的提示词。
pub(crate) fn read_project(config_dir: &Path, project_id: &str) -> Result<CinemaProject> {
    let text = std::fs::read_to_string(cinema_projects_dir(config_dir).join(project_id).join("project.json"))
        .map_err(|e| Error::config(format!("读不到项目：{e}")))?;
    serde_json::from_str(&text).map_err(|e| Error::config(format!("项目文件坏了：{e}")))
}

pub(crate) fn project_route(config_dir: &Path, project_id: &str) -> crate::shot_table::PromptRoute {
    std::fs::read_to_string(cinema_projects_dir(config_dir).join(project_id).join("project.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| serde_json::from_value(v["model_settings"]["prompt_route"].clone()).ok())
        .unwrap_or_default()
}

/// 给一集里的镜头按 `route` 合成提示词，**每合成一镜就落盘一次**（中途失败、应用重启，已合成的不丢）。
///
/// `shot_ids` 为空 = 整集。线路 A 纯填空；线路 B 每镜一次 LM 调用（`call` 注入，测试换假的）。
/// 某一镜没通过检查就跳过并写进 `notes`，不影响其它镜。
pub async fn synthesize_episode_prompts<F, Fut>(
    config_dir: &Path,
    project_id: &str,
    episode_id: &str,
    route: crate::shot_table::PromptRoute,
    shot_ids: &[String],
    call: F,
    progress: impl FnMut(usize, usize, &str),
) -> Result<SynthesizeResult>
where
    F: Fn(crate::prompts::PromptSlot, Vec<(String, String)>) -> Fut,
    Fut: std::future::Future<Output = Result<String>>,
{
    synthesize_episode_prompts_queued(config_dir, project_id, episode_id, route, shot_ids, call, progress, Vec::new).await
}

/// 同上，外加 `more`：每一镜开始前问一次任务现在的完整名单 ——「加入合成队列」追加在后面的接着合
/// （和出片的 [`render_batch`] 同一套，见 [`queue_synthesize`]）。追加进来的镜头轮到时**重新读一遍分集**：
/// 用户多半是改完这一镜才点的重新合成，按开工时读的那份合就把改动丢了。
#[allow(clippy::too_many_arguments)]
pub async fn synthesize_episode_prompts_queued<F, Fut>(
    config_dir: &Path,
    project_id: &str,
    episode_id: &str,
    route: crate::shot_table::PromptRoute,
    shot_ids: &[String],
    call: F,
    mut progress: impl FnMut(usize, usize, &str),
    more: impl Fn() -> Vec<String>,
) -> Result<SynthesizeResult>
where
    F: Fn(crate::prompts::PromptSlot, Vec<(String, String)>) -> Fut,
    Fut: std::future::Future<Output = Result<String>>,
{
    use crate::shot_table::PromptRoute;
    let ep_file = cinema_projects_dir(config_dir).join(project_id).join("episodes").join(episode_id).join("episode.json");
    let read = || -> Result<EpisodeData> {
        let mut ep: EpisodeData = serde_json::from_str(&std::fs::read_to_string(&ep_file).map_err(|e| Error::config(format!("读不到分集：{e}")))?)
            .map_err(|e| Error::config(format!("解析分集失败：{e}")))?;
        drop_old_storyboard(&mut ep);
        // 挂没挂关键帧决定正文怎么写（首帧 / 首尾帧 / 参考），按磁盘算
        sync_keyframes(&ep_file.with_file_name("shots"), &mut ep);
        Ok(ep)
    };
    let ep = read()?;
    if ep.storyboard.is_empty() {
        return Err(Error::config("这一集还没有镜头，先生成镜头表"));
    }
    let assets = crate::storyboard_gen::load_assets(config_dir, project_id, episode_id);
    // 风格那一步选的画面风格，合成时写进提示词。**锁了分集风格就用锁的**
    let style = episode_style(config_dir, project_id, &ep);
    let mut ids: Vec<String> = ep.storyboard.iter().filter(|s| shot_ids.is_empty() || shot_ids.contains(&s.id)).map(|s| s.id.clone()).collect();
    let initial = ids.len();
    let mut done = 0;
    let mut notes = Vec::new();
    let mut remarks = Vec::new();
    let mut i = 0;
    loop {
        // 每一镜开始前（包括跑到头时）看一眼名单：「加入合成队列」追加的接在后面合
        let latest = more();
        if latest.len() > ids.len() && latest.starts_with(&ids) {
            ids.extend(latest[ids.len()..].iter().cloned());
        }
        if i >= ids.len() {
            break;
        }
        let total = ids.len();
        let fresh;
        let ep = if i < initial {
            &ep
        } else {
            match read() {
                Ok(e) => {
                    fresh = e;
                    &fresh
                }
                Err(e) => {
                    notes.push(e.to_string());
                    i += 1;
                    continue;
                }
            }
        };
        let Some(shot) = ep.storyboard.iter().find(|s| s.id == ids[i]) else {
            notes.push(format!("镜头 {} 已经不在这一集里，跳过", ids[i]));
            i += 1;
            continue;
        };
        i += 1;
        progress(i - 1, total, &format!("第 {} 镜", shot.shot_num));
        // 上一镜按镜号取（数组顺序不等于镜头顺序，见「上一镜按镜号取而不是按数组顺序」）
        let prev = ep.storyboard.iter().filter(|s| s.shot_num < shot.shot_num).max_by_key(|s| s.shot_num);
        // 这一镜的形象覆盖换进去：描述、有没有图都按这一镜实际用的形象
        let shot_assets = crate::looks::for_shot(&assets, shot);
        // **长镜头窗口的正文里多一条约束**（台词要在重叠区之前说完）——
        // 和 `current_hash` 读的是同一份 `long_take_scenes`，两边各判各的
        // 会出现「指纹说没过期，而正文里那条约束是上一轮留下的」。
        let ctx = crate::prompt_input::ShotContext {
            shot, prev, assets: &shot_assets, style: &style,
            long_take: ep.long_take_scenes.contains(&shot.scene_no),
        };
        let now = crate::jobs::now();
        let result = if crate::long_shot::is_long(shot.duration) {
            // 超长镜：拆成几段各合成一份，拼回这一镜（见 `long_shot`）
            synthesize_long_shot(ep, shot, prev, &assets, &shot_assets, &style, route, &call, now).await
        } else {
            match route {
                PromptRoute::A => {
                    let p = crate::prompt_route_a::synthesize(&ctx, now);
                    let problems = crate::prompt_route_a::check(&ctx, &p);
                    if problems.is_empty() { Ok((p, Vec::new())) } else { Err(Error::runtime(format!("第 {} 镜线路 A 合成没通过检查：{}", shot.shot_num, problems.join("；")))) }
                }
                PromptRoute::B => crate::prompt_route_b::synthesize(&ctx, &call, now).await,
            }
        };
        match result {
            Ok((p, soft)) => {
                // **能出片但有话要说**的，走 `remarks` 不走 `notes` ——
                // `notes` 那一段在界面上的标题是「没通过检查（没有保存）」。
                // 见 `prompt_route_b::Problems` 和 `jobs::Job::remarks`。
                remarks.extend(soft);
                // 按锁里最新的那份写：B 线路一镜要几十秒，这期间用户可能改了别的镜头
                let saved = crate::episode_lock::update_file(&ep_file, |disk| {
                    // 和 `read` 同一套整理（旧分镜不认、关键帧按磁盘），镜头还在才落盘
                    let mut latest = disk.clone();
                    drop_old_storyboard(&mut latest);
                    sync_keyframes(&ep_file.with_file_name("shots"), &mut latest);
                    let Some(target) = latest.storyboard.iter_mut().find(|s| s.id == shot.id) else { return Ok(false) };
                    // **手写的首帧提示词不覆盖。** 没合成过时在单镜里填的首帧提示词（`video` 空、`frame` 有字）是用户自己写的，
                    // 按场出首帧跑完自动合成会把整份换掉 —— 合成出来的 `frame` 只用来出首帧，留用户那份（2026-10-03 审查查出）
                    let mut p = p;
                    if let Some(old) = target.prompts.get(&route).filter(|o| o.video.trim().is_empty() && !o.frame.trim().is_empty()) {
                        p.frame = old.frame.clone();
                    }
                    target.prompts.insert(route, p);
                    *disk = latest;
                    Ok(true)
                })?;
                if saved {
                    done += 1;
                } else {
                    notes.push(format!("第 {} 镜在合成期间被删掉了，没有保存", shot.shot_num));
                }
            }
            Err(e) => notes.push(e.to_string()),
        }
    }
    let total = ids.len();
    progress(total, total, "");
    Ok(SynthesizeResult { route, done, total, notes, remarks })
}

/// **超长镜（超过 15 秒）的合成**：拆成几个等长窗口镜头（[`crate::long_shot::windows`]），一段一段按长镜头窗口合成，
/// 正文用分隔行拼回这一镜（[`crate::long_shot::join_prompts`]）。指纹按这一镜本身算，和出片前的过期检查对得上。
///
/// 最后一段以外按长镜头窗口写（末尾 2 秒不说话，那里是和下一段的重叠区）；首帧只取第一段的。
#[allow(clippy::too_many_arguments)]
async fn synthesize_long_shot<F, Fut>(
    ep: &EpisodeData,
    shot: &StoryboardShot,
    prev: Option<&StoryboardShot>,
    raw_assets: &[crate::storyboard_gen::AssetRef],
    shot_assets: &[crate::storyboard_gen::AssetRef],
    style: &str,
    route: crate::shot_table::PromptRoute,
    call: &F,
    now: u64,
) -> Result<(crate::shot_table::RoutePrompts, Vec<String>)>
where
    F: Fn(crate::prompts::PromptSlot, Vec<(String, String)>) -> Fut,
    Fut: std::future::Future<Output = Result<String>>,
{
    use crate::shot_table::PromptRoute;
    if ep.long_take_scenes.contains(&shot.scene_no) {
        return Err(Error::config(format!(
            "第 {} 镜 {} 秒是超长镜，不能放在整段出的场里（整段出要求一镜一个 15 秒以内的窗口）—— 这一场取消整段出，或者把这一镜改回 15 秒以内",
            shot.shot_num, shot.duration
        )));
    }
    if shot.effective_continuity() != Continuity::None {
        return Err(Error::config(format!(
            "第 {} 镜是超长镜（{} 秒，分段出），不能「{}」—— 段与段之间靠重叠帧接，不用参考视频。把它改成「不接」再合成",
            shot.shot_num, shot.duration, shot.effective_continuity().label()
        )));
    }
    let windows = crate::long_shot::windows(shot);
    let n = windows.len();
    let mut videos = Vec::with_capacity(n);
    let mut frame = String::new();
    let mut remarks = Vec::new();
    for (k, win) in windows.iter().enumerate() {
        let ctx = crate::prompt_input::ShotContext {
            shot: win,
            prev: if k == 0 { prev } else { Some(&windows[k - 1]) },
            assets: shot_assets,
            style,
            long_take: k + 1 < n,
        };
        let label = format!("第 {} 镜（超长镜第 {}/{n} 段）", shot.shot_num, k + 1);
        let (p, soft) = match route {
            PromptRoute::A => {
                let p = crate::prompt_route_a::synthesize(&ctx, now);
                let problems = crate::prompt_route_a::check(&ctx, &p);
                if !problems.is_empty() {
                    return Err(Error::runtime(format!("{label}线路 A 合成没通过检查：{}", problems.join("；"))));
                }
                (p, Vec::new())
            }
            PromptRoute::B => crate::prompt_route_b::synthesize(&ctx, call, now).await.map_err(|e| Error::runtime(format!("{label}：{e}")))?,
        };
        remarks.extend(soft.into_iter().map(|r| format!("{label}：{r}")));
        if k == 0 {
            frame = p.frame;
        }
        videos.push(p.video);
    }
    Ok((
        crate::shot_table::RoutePrompts {
            frame,
            video: crate::long_shot::join_prompts(&videos),
            synthesized_at: now,
            fields_hash: current_hash(&ep.storyboard, shot, raw_assets, style, &ep.long_take_scenes),
        },
        remarks,
    ))
}

/// 合成提示词（按项目当前线路）。**后端任务**，立即返回登记的任务；进度、结果走任务事件。
///
/// `shot_ids` 为空 = 整集。同一集同时只能有一个合成任务。
#[tauri::command]
pub fn cinema_synthesize_prompts(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    project_id: String,
    episode_id: String,
    shot_ids: Vec<String>,
    route: Option<crate::shot_table::PromptRoute>,
) -> Result<crate::jobs::Job> {
    start_synthesize(Some(app), state.config_dir.clone(), state.gateway.clone(), project_id, episode_id, shot_ids, route)
}

/// [`cinema_synthesize_prompts`] 的本体（Chat 的 `studio_synthesize_prompts` 也走这里）
///
/// **合成哪条线路由调用方说**：合成弹窗左 A 右 B 各合各的，合成**不改**出片用哪条线路（那在出片设置里选）。
/// 以前合成哪条就把哪条存成当前线路，合成完 A 再点 B 就对不上了（2026-09-29 用户：「现在有 confuse」）。
/// 不说（对话里的工具）就按项目当前线路。
pub fn start_synthesize(app: Option<tauri::AppHandle>, config_dir: PathBuf, gateway: std::sync::Arc<crate::Gateway>, project_id: String, episode_id: String, shot_ids: Vec<String>, route: Option<crate::shot_table::PromptRoute>) -> Result<crate::jobs::Job> {
    let route = route.unwrap_or_else(|| project_route(&config_dir, &project_id));
    let title = format!("{} · {}", route.label(), if shot_ids.is_empty() { "整集".to_string() } else { format!("{} 镜", shot_ids.len()) });
    let job_id = crate::jobs::start_exclusive(&config_dir, crate::jobs::JobKind::Prompts, &project_id, &episode_id, "", 0, &title)
        .map_err(|_| Error::config("这一集正在合成提示词，等它跑完 —— 进度在右下角和 QUEUE 里"))?;
    // 合成按当前的接法写（参考出片同场的写 `<Video 1>`）：先把「接上一镜」按规则设好
    auto_continuity_now(&config_dir, &project_id, &episode_id);
    // **名单登记在任务上**：「加入合成队列」要往后面追加（见 [`queue_synthesize`]），界面按它显示排第几
    let resolved: Vec<String> = read_episode(&config_dir, &project_id, &episode_id)
        .map(|ep| ep.storyboard.iter().filter(|s| shot_ids.is_empty() || shot_ids.contains(&s.id)).map(|s| s.id.clone()).collect())
        .unwrap_or_default();
    let started = if resolved.is_empty() { crate::jobs::get(&config_dir, &job_id) } else { crate::jobs::set_targets(&config_dir, &job_id, &resolved) }
        .ok_or_else(|| Error::runtime("任务登记后读不回来"))?;
    crate::jobs::emit(app.as_ref(), Some(&started));

    let usage_scope = crate::token_usage::Scope::episode(&project_id, &episode_id);
    tauri::async_runtime::spawn(usage_scope.run(async move {
        // 线路 B 要用后台模型：出片时可能被卸掉了，开工前确认加载（在后台任务里做，不卡界面）
        if route == crate::shot_table::PromptRoute::B {
            let dir = config_dir.clone();
            let loaded = tauri::async_runtime::spawn_blocking(move || crate::llm_memory::ensure_background_loaded(&dir)).await.unwrap_or_else(|e| Err(e.to_string()));
            if let Err(e) = loaded {
                let done = crate::jobs::finish_with(&config_dir, &job_id, None, None, &[], Some(&e));
                crate::jobs::emit(app.as_ref(), done.as_ref());
                return;
            }
        }
        let cfg = crate::prompts::load_for_project(&config_dir, &project_id);
        let call = |slot: crate::prompts::PromptSlot, vars: Vec<(String, String)>| {
            let gateway = gateway.clone();
            let cfg = cfg.clone();
            async move {
                let vars: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                crate::prompts::run(&gateway, &cfg, slot, &vars).await
            }
        };
        let out = synthesize_episode_prompts_queued(
            &config_dir,
            &project_id,
            &episode_id,
            route,
            &shot_ids,
            call,
            |done, total, label| {
                let job = crate::jobs::set_progress(&config_dir, &job_id, done, total, label);
                crate::jobs::emit(app.as_ref(), job.as_ref());
            },
            || crate::jobs::get(&config_dir, &job_id).map(|j| j.targets).unwrap_or_default(),
        )
        .await;
        // 收尾那一瞬间才加进队列的（名单已经读完）：说出来，别默默丢掉
        let out = out.map(|mut r| {
            let late = crate::jobs::get(&config_dir, &job_id).map_or(0, |j| j.targets.len().saturating_sub(r.total));
            if late > 0 {
                r.notes.push(format!("有 {late} 镜在这一轮收尾时才加入合成队列，没赶上 —— 重新点一次合成"));
            }
            r
        });
        let done = match out {
            Ok(r) if r.done == 0 && r.total > 0 => crate::jobs::finish_with(&config_dir, &job_id, None, None, &r.notes, Some(&format!("{} 镜都没合成成功：{}", r.total, r.notes.join("；")))),
            Ok(r) => {
                let summary = format!("{}：合成 {}/{} 镜", r.route.label(), r.done, r.total);
                crate::jobs::finish_with_remarks(&config_dir, &job_id, None, Some(&summary), &r.notes, &r.remarks, None)
            }
            Err(e) => crate::jobs::finish_with(&config_dir, &job_id, None, None, &[], Some(&e.to_string())),
        };
        crate::jobs::emit(app.as_ref(), done.as_ref());
    }));
    Ok(started)
}

/// **合成这几镜；这一集已经在合成的话，排到那一轮的后面**（2026-10-06 用户：「重新合成这一镜……如果有正在合成的，
/// 则无法点击，这里也要做到可以添加到合成提示词列队里，方式方法跟出片一致」）。和出片的 [`queue_render`] 同一套：
/// 追加进在跑那一条的名单（`jobs::append_targets`），那一轮每合一镜前看一眼名单。
///
/// 在跑的那一轮合的是另一条线路就不追加 —— 一条任务只合一条线路，混进去会用错线路合。
pub fn queue_synthesize(app: Option<tauri::AppHandle>, config_dir: PathBuf, gateway: std::sync::Arc<crate::Gateway>, project_id: String, episode_id: String, shot_ids: Vec<String>, route: Option<crate::shot_table::PromptRoute>) -> Result<crate::jobs::Job> {
    if shot_ids.is_empty() {
        return Err(Error::config("没有要合成的镜头"));
    }
    let want = route.unwrap_or_else(|| project_route(&config_dir, &project_id));
    if let Some(running) = crate::jobs::running(&config_dir, &project_id, &episode_id).into_iter().find(|j| j.kind == crate::jobs::JobKind::Prompts) {
        if !running.prompt.starts_with(want.label()) {
            return Err(Error::config(format!("这一集正在合成另一条线路（{}），等它跑完再合{}", running.prompt, want.label())));
        }
        if let Some((job, _)) = crate::jobs::append_targets(&config_dir, &running.id, &shot_ids) {
            crate::jobs::emit(app.as_ref(), Some(&job));
            return Ok(job);
        }
    }
    start_synthesize(app, config_dir, gateway, project_id, episode_id, shot_ids, Some(want))
}

#[tauri::command]
pub fn cinema_queue_synthesize(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    project_id: String,
    episode_id: String,
    shot_ids: Vec<String>,
    route: Option<crate::shot_table::PromptRoute>,
) -> Result<crate::jobs::Job> {
    queue_synthesize(Some(app), state.config_dir.clone(), state.gateway.clone(), project_id, episode_id, shot_ids, route)
}

/// 单独重新生成一场的结果。
#[derive(Debug, Clone, Serialize)]
pub struct SceneRegenResult {
    pub scene_no: u32,
    pub shots: usize,
    /// 重新生成之后这一场还剩几条问题
    pub issues: usize,
    pub notes: Vec<String>,
}

/// 只重新生成一场，**其它场的镜头、提示词、出过的片都不动**，镜号顺延。
///
/// 用户场景（2026-09-14）：第 4 场漏拍了「殿外风起」两句 —— 这是少了镜头，代码修不了；
/// 为这两句重新生成整张表会把其余 13 镜合成好的提示词和出过的片全覆盖掉。
///
/// 把这一场现在的问题原文先告诉模型，最多重试 3 次，取问题最少的一份。
/// 只在剧本有标题、能切出和镜头表同样几场时可用（要知道这一场的原文是哪段）。
pub async fn regenerate_scene<F, Fut>(
    config_dir: &Path,
    project_id: &str,
    episode_id: &str,
    scene_no: u32,
    call: F,
) -> Result<SceneRegenResult>
where
    F: Fn(crate::prompts::PromptSlot, Vec<(String, String)>) -> Fut,
    Fut: std::future::Future<Output = Result<String>>,
{
    let ep_file = cinema_projects_dir(config_dir).join(project_id).join("episodes").join(episode_id).join("episode.json");
    let read = || -> Result<EpisodeData> {
        let mut ep: EpisodeData = serde_json::from_str(&std::fs::read_to_string(&ep_file).map_err(|e| Error::config(format!("读不到分集：{e}")))?)
            .map_err(|e| Error::config(format!("解析分集失败：{e}")))?;
        drop_old_storyboard(&mut ep);
        Ok(ep)
    };
    let ep = read()?;
    let scenes = crate::storyboard_gen::split_by_headings(&ep.script_text)
        .filter(|s| s.len() == ep.storyboard.iter().map(|x| x.scene_no).max().unwrap_or(0) as usize)
        .ok_or_else(|| Error::config("这一集的场是模型切的（剧本没有【】标题），对不上这一场的原文，没法单独重新生成 —— 请重新生成整张镜头表"))?;
    let scene = scenes.get((scene_no as usize).wrapping_sub(1)).ok_or_else(|| Error::config(format!("没有第 {scene_no} 场")))?;
    let assets = crate::storyboard_gen::load_assets(config_dir, project_id, episode_id);
    let mut sorted = ep.storyboard.clone();
    sorted.sort_by_key(|s| s.shot_num);
    let before: Vec<&StoryboardShot> = sorted.iter().filter(|s| s.scene_no < scene_no).collect();
    let first_num = before.len() as u32 + 1;
    // 和整集生成同一份「上一场的结尾」（地点、光线、固定布景、画面停在哪）
    let prev_end = before.last().map(|s| crate::storyboard_gen::scene_end(s)).unwrap_or_else(|| "这是本集第一场".into());
    // 这一场现在的问题，先告诉模型
    let old_shots: Vec<StoryboardShot> = sorted.iter().filter(|s| s.scene_no == scene_no).cloned().collect();
    let mut current = crate::storyboard_gen::validate_shots(&old_shots, &assets);
    current.extend(crate::storyboard_gen::dialogue_coverage(&scene.text, &old_shots));
    current.extend(crate::storyboard_gen::source_coverage(&scene.text, &old_shots));
    let feedback = current.iter().map(|i| i.message.clone()).collect::<Vec<_>>().join("；");
    let catalog = crate::storyboard_gen::asset_catalog(&assets);
    let take = crate::one_take::plan(ep.one_take, &ep.scene_takes, scene_no, &scene.text, scenes.len(), ep.target_secs);
    // 一镜出的场换了模式重新生成时，旧镜头的问题（「一镜内不切镜」「时长不在 4~15」）对新写法不适用，不带
    let feedback = if take.is_some() && !old_shots.iter().all(|s| s.one_take) { String::new() } else { feedback };
    let (new_shots, notes) = crate::storyboard_gen::generate_scene(scene, scene_no, first_num, &prev_end, &assets, &catalog, &call, &feedback, 3, take, crate::storyboard_gen::scene_budgets(&scenes, ep.target_secs)[(scene_no - 1) as usize])
        .await
        .map_err(|e| Error::runtime(format!("第 {scene_no} 场重新生成失败（重试过 3 次）：{e}")))?;
    let mut remaining = crate::storyboard_gen::validate_shots(&new_shots, &assets);
    remaining.extend(crate::storyboard_gen::dialogue_coverage(&scene.text, &new_shots));
    remaining.extend(crate::storyboard_gen::source_coverage(&scene.text, &new_shots));

    // 按锁里最新的那份写：只换这一场，其它场原样保留（生成要一两分钟，这期间别的镜头可能改过）
    let count = new_shots.len();
    crate::episode_lock::update_file(&ep_file, |latest| {
        drop_old_storyboard(latest);
        let delta = new_shots.len() as i64 - latest.storyboard.iter().filter(|s| s.scene_no == scene_no).count() as i64;
        latest.storyboard.retain(|s| s.scene_no != scene_no);
        for s in latest.storyboard.iter_mut().filter(|s| s.scene_no > scene_no) {
            s.shot_num = (s.shot_num as i64 + delta).max(1) as u32;
        }
        sync_one_take_long_take(&mut latest.long_take_scenes, &new_shots, scene_no);
        latest.storyboard.extend(new_shots);
        latest.storyboard.sort_by_key(|s| (s.scene_no, s.shot_num));
        Ok(())
    })?;
    Ok(SceneRegenResult { scene_no, shots: count, issues: remaining.len(), notes })
}

/// 一镜出的场生成完之后，**长镜头标记跟着镜数走**：2 镜以上（超过 15 秒、已经等分）自动整段出，
/// 1 镜就不是长镜头。不是一镜出的场不碰 —— 那是用户自己勾的。
fn sync_one_take_long_take(long_take_scenes: &mut Vec<u32>, shots: &[StoryboardShot], scene_no: u32) {
    let in_scene: Vec<&StoryboardShot> = shots.iter().filter(|s| s.scene_no == scene_no).collect();
    if in_scene.is_empty() || !in_scene.iter().all(|s| s.one_take) {
        return;
    }
    long_take_scenes.retain(|n| *n != scene_no);
    if in_scene.len() >= 2 {
        long_take_scenes.push(scene_no);
        long_take_scenes.sort_unstable();
    }
}

/// 单独重新生成一场。**后台任务**（和生成镜头表同一类，同一集同时只能有一个）。
#[tauri::command]
pub fn cinema_regenerate_scene(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    project_id: String,
    episode_id: String,
    scene_no: u32,
) -> Result<crate::jobs::Job> {
    let config_dir = state.config_dir.clone();
    let job_id = crate::jobs::start_exclusive(&config_dir, crate::jobs::JobKind::ShotTable, &project_id, &episode_id, "", 0, &format!("重新生成第 {scene_no} 场"))
        .map_err(|_| Error::config("这一集正在生成镜头，等它跑完 —— 进度在右下角和 QUEUE 里"))?;
    let started = crate::jobs::set_progress(&config_dir, &job_id, 0, 1, &format!("第 {scene_no} 场")).ok_or_else(|| Error::runtime("任务登记后读不回来"))?;
    crate::jobs::emit(Some(&app), Some(&started));
    let gateway = state.gateway.clone();
    let usage_scope = crate::token_usage::Scope::episode(&project_id, &episode_id);
    tauri::async_runtime::spawn(usage_scope.run(async move {
        let dir = config_dir.clone();
        let loaded = tauri::async_runtime::spawn_blocking(move || crate::llm_memory::ensure_background_loaded(&dir)).await.unwrap_or_else(|e| Err(e.to_string()));
        let cfg = crate::prompts::load_for_project(&config_dir, &project_id);
        let call = |slot: crate::prompts::PromptSlot, vars: Vec<(String, String)>| {
            let gateway = gateway.clone();
            let cfg = cfg.clone();
            async move {
                let vars: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                crate::prompts::run(&gateway, &cfg, slot, &vars).await
            }
        };
        let out = match loaded {
            Ok(()) => regenerate_scene(&config_dir, &project_id, &episode_id, scene_no, call).await,
            Err(e) => Err(Error::config(e)),
        };
        let done = match out {
            Ok(r) => {
                let summary = format!("第 {} 场重新生成：{} 个镜头{}", r.scene_no, r.shots, if r.issues == 0 { String::new() } else { format!("，还有 {} 个问题", r.issues) });
                crate::jobs::finish_with(&config_dir, &job_id, None, Some(&summary), &r.notes, None)
            }
            Err(e) => crate::jobs::finish_with(&config_dir, &job_id, None, None, &[], Some(&e.to_string())),
        };
        crate::jobs::emit(Some(&app), done.as_ref());
    }));
    Ok(started)
}

/// 自动修正镜头表里模型反复犯的固定写法（见 [`crate::storyboard_gen::repair_shots`]），返回改了几处。
///
/// 生成镜头表时已经自动修；这个命令给**已经生成好的表**用（修正规则是后加的）。
#[tauri::command]
pub fn cinema_repair_storyboard(state: State<'_, AppState>, project_id: String, episode_id: String) -> Result<usize> {
    let f = cinema_projects_dir(&state.config_dir).join(&project_id).join("episodes").join(&episode_id).join("episode.json");
    let assets = crate::storyboard_gen::load_assets(&state.config_dir, &project_id, &episode_id);
    // 修正是纯字符串活，整段放在锁里；修了几处为 0 就不落盘（清旧分镜只在内存里做）
    crate::episode_lock::update_file(&f, |disk| {
        let mut ep = disk.clone();
        let fixed = repair_episode(&mut ep, &assets);
        if fixed > 0 {
            *disk = ep;
        }
        Ok(fixed)
    })
}

/// 「自动修正」的本体：按场修写法（[`crate::storyboard_gen::repair_shots`]）和原文（`repair_sources`）。回改了几处
fn repair_episode(ep: &mut EpisodeData, assets: &[crate::storyboard_gen::AssetRef]) -> usize {
    drop_old_storyboard(ep);
    // 按场分开修：简称对全名时看的是本场的用法
    let mut fixed = 0;
    let mut scene_nos: Vec<u32> = ep.storyboard.iter().map(|s| s.scene_no).collect();
    scene_nos.sort_unstable();
    scene_nos.dedup();
    // 剧本有标题、场数对得上时，顺便把原文里不是剧本原句的部分删掉（第 7、8 镜那类）
    let scene_texts = crate::storyboard_gen::split_by_headings(&ep.script_text)
        .filter(|scenes| scenes.len() == ep.storyboard.iter().map(|s| s.scene_no).max().unwrap_or(0) as usize);
    for no in scene_nos {
        let mut scene: Vec<StoryboardShot> = ep.storyboard.iter().filter(|s| s.scene_no == no).cloned().collect();
        fixed += crate::storyboard_gen::repair_shots(&mut scene, assets);
        if let Some(text) = scene_texts.as_ref().and_then(|t| t.get((no as usize).wrapping_sub(1))) {
            fixed += crate::storyboard_gen::repair_sources(&mut scene, &text.name, &text.text);
        }
        for s in scene {
            if let Some(t) = ep.storyboard.iter_mut().find(|x| x.id == s.id) {
                *t = s;
            }
        }
    }
    fixed
}

/// 「自动修正」**真能改掉**的那几条问题：在内存里修一遍，修完不见了的就是。不写盘。
///
/// 按钮上的数字以这个为准。原来前端按问题文字猜（「不是这一镜的出场人物」一律算能修），
/// 而后端只认得资产库里有的角色 —— EP02 的邻居甲、打手甲没有资产，按钮写「自动修正 5 处」，
/// 点了一处没改、界面也没任何反应（2026-09-30）。
#[tauri::command]
pub fn cinema_repairable_issues(state: State<'_, AppState>, project_id: String, episode_id: String) -> Result<Vec<crate::shot_table::ShotIssue>> {
    let ep = read_episode(&state.config_dir, &project_id, &episode_id)?;
    Ok(repairable_issues(&state.config_dir, &project_id, &episode_id, &ep))
}

pub(crate) fn repairable_issues(config_dir: &Path, project_id: &str, episode_id: &str, ep: &EpisodeData) -> Vec<crate::shot_table::ShotIssue> {
    let assets = crate::storyboard_gen::load_assets(config_dir, project_id, episode_id);
    let before = validate_storyboard(config_dir, project_id, episode_id, ep);
    let mut fixed = ep.clone();
    if repair_episode(&mut fixed, &assets) == 0 {
        return Vec::new();
    }
    let after = validate_storyboard(config_dir, project_id, episode_id, &fixed);
    before.into_iter().filter(|i| !after.contains(i)).collect()
}

/// 红色问题框右边的「修复」按钮能做什么（用户 2026-09-30：「没有修复按钮了，建议在红色问题提示框右侧增加修复按钮」）。
///
/// **后端只给改法，改由界面在当前那一镜上改、走正常的保存** —— 和手动改时长 / 改说话人是同一条路，
/// 时长伸缩时间线用的也是界面那份 `retime.ts`，不在这边另写一份。
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IssueFix {
    /// 加长到这么多秒（台词装不下）
    Duration { secs: f32 },
    /// 第 `line` 句台词改成 `speaker` 说；`off_screen` = 改完这人既不在出场人物里、画面里也没写到，一起标成画外音
    Speaker { line: usize, speaker: String, off_screen: bool },
    /// 「自动修正」能改掉的那几类（夹英文、切至……），整集修一遍
    Repair,
    /// 重新合成这一镜（当前线路）。线路 B 正文里的英文二选一：合成时会当软问题喂回写手让它只写一个版本
    /// （`prompt_route_b::check_wording`），代码自己改英文句子改不准
    Resynth,
    /// 把这张有图的资产挂成参考图（见 [`unmounted_assets`]）。挂上之后提示词过期，要重新合成
    Mount { asset_id: String, usage: crate::shot_table::RefUsage, control: String },
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct IssueFixOffer {
    pub shot_num: u32,
    /// 挂在哪一条问题旁边（按原文对上）
    pub message: String,
    /// 按钮上写什么
    pub label: String,
    pub fix: IssueFix,
}

#[tauri::command]
pub fn cinema_issue_fixes(state: State<'_, AppState>, project_id: String, episode_id: String) -> Result<Vec<IssueFixOffer>> {
    let ep = read_episode(&state.config_dir, &project_id, &episode_id)?;
    Ok(issue_fixes(&state.config_dir, &project_id, &episode_id, &ep))
}

pub(crate) fn issue_fixes(config_dir: &Path, project_id: &str, episode_id: &str, ep: &EpisodeData) -> Vec<IssueFixOffer> {
    let issues = validate_storyboard(config_dir, project_id, episode_id, ep);
    let repairable = repairable_issues(config_dir, project_id, episode_id, ep);
    let assets = crate::storyboard_gen::load_assets(config_dir, project_id, episode_id);
    let mut out = Vec::new();
    for i in &issues {
        let offer = |label: String, fix: IssueFix| IssueFixOffer { shot_num: i.shot_num, message: i.message.clone(), label, fix };
        if repairable.contains(i) {
            out.push(offer("自动修正".into(), IssueFix::Repair));
            continue;
        }
        if i.field == "advice" && i.message.starts_with("正文里有二选一") {
            out.push(offer("重新合成这一镜".into(), IssueFix::Resynth));
            continue;
        }
        let Some(shot) = ep.storyboard.iter().find(|s| s.shot_num == i.shot_num) else { continue };
        if i.field == "advice" && i.message.contains(UNMOUNTED_MARK) {
            if let Some(fix) = mount_fix(shot, &assets, &i.message) {
                out.push(offer("挂上参考图".into(), fix));
            }
            continue;
        }
        if i.field == "dialogue" && (i.message.contains("要么加长这一镜") || i.message.contains("怎么都说不完")) {
            let lines: Vec<(&str, &str, bool)> = shot.dialogue_lines.iter().map(|l| (l.speaker.as_str(), l.line.as_str(), l.off_screen)).collect();
            // 超过 15 秒就是超长镜（分段出，见 `long_shot`）；45 秒也装不下的不给按钮 —— 只能拆镜或删台词
            if let Some(secs) = crate::shot_table::min_secs_for_lines(&lines).filter(|s| *s > shot.duration) {
                let label = if crate::long_shot::is_long(secs) { format!("加长到 {secs} 秒（超长镜）") } else { format!("加长到 {secs} 秒") };
                out.push(offer(label, IssueFix::Duration { secs }));
            }
            continue;
        }
        let names: Vec<&str> = assets.iter().filter(|a| a.kind == crate::assets::AssetKind::Character).map(|a| a.name.as_str()).collect();
        if let Some((line, speaker)) = crate::storyboard_gen::speaker_fix(shot, &i.message, &names) {
            let cast: Vec<&str> = shot.all_character_ids().iter().filter_map(|id| assets.iter().find(|a| &a.id == id)).map(|a| a.name.as_str()).collect();
            let on_screen = shot.segments.iter().any(|g| g.visual.contains(speaker.as_str()) || g.blocking.contains(speaker.as_str()));
            let off_screen = !cast.contains(&speaker.as_str()) && !on_screen;
            out.push(offer(format!("改成「{speaker}」说"), IssueFix::Speaker { line, speaker, off_screen }));
        }
    }
    out
}

/// 校验这一集的镜头表，给界面标问题。
#[tauri::command]
pub fn cinema_validate_storyboard(
    state: State<'_, AppState>,
    project_id: String,
    episode_id: String,
) -> Result<Vec<crate::shot_table::ShotIssue>> {
    let f = cinema_projects_dir(&state.config_dir).join(&project_id).join("episodes").join(&episode_id).join("episode.json");
    let mut ep: EpisodeData = serde_json::from_str(&std::fs::read_to_string(&f).unwrap_or_default())
        .map_err(|e| Error::config(format!("读不出 episode.json：{e}")))?;
    drop_old_storyboard(&mut ep);
    sync_keyframes(&f.with_file_name("shots"), &mut ep);
    Ok(validate_storyboard(&state.config_dir, &project_id, &episode_id, &ep))
}

/// 出片要发的提示词：这一镜在**当前线路**下存的视频运动提示词，一个字都不加。
///
/// 没合成过就直接拒绝，不拿别的字段凑、也不拿另一条线路的凑 —— 凑出来的东西
/// 就是之前那种「看着能出片、出来的东西和镜头对不上」，而且看片时分不清是哪条线路出的。
pub(crate) fn video_prompt_for_render(shot: &StoryboardShot, route: crate::shot_table::PromptRoute) -> Result<String> {
    let prompt = shot.prompts.get(&route).map(|p| p.video.trim()).unwrap_or_default();
    if prompt.is_empty() {
        return Err(Error::config(format!("第 {} 镜还没有用{}合成视频运动提示词，先合成再出片", shot.shot_num, route.label())));
    }
    Ok(prompt.to_string())
}


/// 出一镜。外层只做一件事：把这一镜的**出片额外设置**（项目的采样方法；打斗镜按动作类型挂战斗 LoRA 预设）
/// 放进作用域，H3 建图时自己取（`weetodd::with_extras`）。正文在 `render_shot_video_body`
pub async fn cinema_render_shot_video_with_config_dir(
    config_dir: &Path,
    req: RenderShotReq,
) -> Result<RenderShotResult> {
    let extras = shot_render_extras(config_dir, &req);
    if !extras.loras.is_empty() {
        tracing::info!(shot = %req.shot_id, loras = ?extras.loras, trigger = %extras.trigger, "打斗镜挂战斗 LoRA 预设");
    }
    crate::agent_runtime::weetodd::with_extras(extras, render_shot_video_body(config_dir, req)).await
}

/// 读项目出片设置 + 这一镜的打斗类型，算出额外设置。读不到就是默认（euler、不挂）
fn shot_render_extras(config_dir: &Path, req: &RenderShotReq) -> crate::agent_runtime::weetodd::H3Extras {
    let proj_dir = cinema_projects_dir(config_dir).join(&req.project_id);
    let render = std::fs::read_to_string(proj_dir.join("project.json")).ok()
        .and_then(|c| serde_json::from_str::<CinemaProject>(&c).ok())
        .map(|p| p.model_settings.render)
        .unwrap_or_default();
    let style = std::fs::read_to_string(proj_dir.join("episodes").join(&req.episode_id).join("episode.json")).ok()
        .and_then(|c| serde_json::from_str::<EpisodeData>(&c).ok())
        .and_then(|ep| ep.storyboard.into_iter().find(|s| s.id == req.shot_id))
        .map(|s| s.combat_style)
        .unwrap_or_default();
    crate::combat::extras_for(&render, &style)
}

async fn render_shot_video_body(
    config_dir: &Path,
    req: RenderShotReq,
) -> Result<RenderShotResult> {
    let root = cinema_projects_dir(config_dir);
    let proj_dir = root.join(&req.project_id);
    let ep_dir = proj_dir.join("episodes").join(&req.episode_id);
    let shots_dir = ep_dir.join("shots");
    let _ = std::fs::create_dir_all(&shots_dir);

    let ep_file = ep_dir.join("episode.json");
    if !ep_file.is_file() {
        return Err(Error::config("分集不存在"));
    }

    let mut ep: EpisodeData =
        serde_json::from_str(&std::fs::read_to_string(&ep_file).unwrap_or_default())
            .map_err(|e| Error::config(format!("读取分集失败：{e}")))?;
    // 关键帧按磁盘算：指纹和出片模式都读它
    sync_keyframes(&shots_dir, &mut ep);

    let shot_id_for_path = if let Some(shot) = ep.storyboard.iter().find(|s| s.id == req.shot_id) {
        shot.id.clone()
    } else {
        return Err(Error::config(format!("找不到分镜「{}」", req.shot_id)));
    };

    let shot_img_path = shots_dir.join(format!("{}.png", shot_id_for_path));
    let mut shot_vid_path = shots_dir.join(format!("{}.mp4", shot_id_for_path));

    // **参考模式不需要关键帧。** 它拿的是 @ 引用出来的资产参考图，
    // 直接出片。要求它先出首帧，等于把两条链路当成一条。
    let shot_mode = ep
        .storyboard
        .iter()
        .find(|s| s.id == req.shot_id)
        .map(|s| s.mode)
        .unwrap_or_default();
    let engine = resolve_shot_video_engine(config_dir, &req, shot_mode)?;
    // H3 走新链路：挂图还是文生由参考图槽位决定，不看旧的 mode 字段、不需要关键帧
    if engine != "h3" && shot_mode == ShotMode::FirstFrame && !shot_img_path.is_file() {
        return Err(Error::runtime(format!(
            "Shot {} 尚未生成关键帧，请先执行关键帧渲染步骤",
            req.shot_id
        )));
    }
    // 判断提示词过期要看上一镜，先留一份不可变的镜头表（下面要可变借用这一镜）
    let storyboard_snapshot = ep.storyboard.clone();
    // 下面要可变借用这一镜，长镜头场号也先取一份
    let long_takes_snapshot = ep.long_take_scenes.clone();
    // **画风也要在借可变引用之前算好。** 分集锁了就用锁的，没锁跟项目走
    let style_for_render = episode_style(config_dir, &req.project_id, &ep);
    let shot = ep
        .storyboard
        .iter_mut()
        .find(|s| s.id == req.shot_id)
        .unwrap();

    // 1. 读取画幅尺寸和提示词线路
    let mut aspect_ratio = "9:16".to_string();
    let mut prompt_route = crate::shot_table::PromptRoute::default();
    let mut render = RenderSettings::default();
    let proj_file = proj_dir.join("project.json");
    if proj_file.is_file() {
        if let Ok(c) = std::fs::read_to_string(&proj_file) {
            if let Ok(p) = serde_json::from_str::<CinemaProject>(&c) {
                aspect_ratio = p.aspect_ratio;
                prompt_route = p.model_settings.prompt_route;
                render = p.model_settings.render;
            }
        }
    }
    // H3 按出片设置定尺寸（出片选项卡里选的清晰度和比例）；别的引擎还是老规则
    let (width, height) = if engine == "h3" { render.size(&aspect_ratio) } else { shot_render_size(&aspect_ratio) };

    // 2. 查找 FFmpeg 可执行文件
    let ffmpeg_candidates = [
        "/opt/homebrew/bin/ffmpeg",
        "/usr/local/bin/ffmpeg",
        "ffmpeg",
    ];
    let mut ffmpeg_bin = "ffmpeg";
    for c in ffmpeg_candidates {
        if std::path::Path::new(c).is_file() {
            ffmpeg_bin = c;
            break;
        }
    }

    let duration = shot.duration.max(1.0);
    let fps = 30;
    let total_frames = ((duration * fps as f32) as u32).max(30);
    let _ = std::fs::remove_file(&shot_vid_path);

    // **台词装不装得下，出片前就算得出来，那就别等用户听。**
    //
    // 超容量时 H3 会静默截断：任务返回成功、画面正常、时长也对，只是台词
    // 后半段根本没发声（实测送 78 字和 108 字都只说了 46 字）。
    // 不说的话用户只有逐句听才会发现 —— 而那通常是成片之后。
    let dialogue_warning = crate::h3_prompt::check_dialogue_fit(
        &shot_dialogue_lines(config_dir, shot, &crate::assets::RefScope::Project { project_id: &req.project_id, episode_id: Some(&req.episode_id) }),
        shot.duration as f64,
    )
    .warning();
    if let Some(w) = &dialogue_warning {
        tracing::warn!(shot = %shot_id_for_path, "{w}");
    }

    if engine != "camera_preview" {
        match engine.as_str() {
            "liveportrait" => {
                render_liveportrait_shot(shot, &shot_img_path, &shot_vid_path, &ep_dir)?;
            }
            // 本机 WeeTodd/MLX 那两条。逐镜两条链路在这里分流：
            //   · 首帧模式：关键帧就是 i2v 的输入（上面已确认它存在）
            //   · 参考模式：@ 引用出来的资产参考图直接出片，没有关键帧
            "minimax_h3" | "h3" | "ltx" | "ltx25" | "ltx2.5" => {
                // **缺省是「要音轨」。** 老的 engine_config 里没有这个键，
                // 读不到时必须当开 —— 当关会静默掐掉 H3 的同步中文配音。
                // 老调用方传了 engine_config 就听它的；没传按项目出片设置
                let generate_audio = req
                    .engine_config
                    .as_ref()
                    .map(|c| c.generate_audio)
                    .unwrap_or(render.generate_audio);
                // **原样发送合成好的视频运动提示词，不再追加任何东西。**
                // 以前这里在出片那一刻由六处拼出一千多字，互相打架（见 2026-09-13 分镜重构方案）。
                let prompt = video_prompt_for_render(shot, prompt_route)?;
                let assets = crate::storyboard_gen::load_assets(config_dir, &req.project_id, &req.episode_id);
                // 合成之后改过字段或补过资产图：提示词里的参考图编号 / 描述已经和现在对不上，先拦下
                let style = style_for_render.clone();
                if shot.prompts.get(&prompt_route).is_some_and(|p| !p.fields_hash.is_empty() && p.fields_hash != current_hash(&storyboard_snapshot, shot, &assets, &style, &long_takes_snapshot)) {
                    return Err(Error::config(format!("第 {} 镜合成提示词之后，镜头字段、上一镜、资产或风格改过 —— 先重新合成{}的提示词再出片", shot.shot_num, prompt_route.label())));
                }
                // 这一次的文件：按线路、按时间各存一份，A / B 和每一「次」互不覆盖
                let route_key = match prompt_route { crate::shot_table::PromptRoute::A => "a", crate::shot_table::PromptRoute::B => "b" };
                // 回填的不参与抽卡计数，否则收一条画布的片就把下一次重出的种子挪走了
                let same_route = shot.takes.iter().filter(|t| t.route == prompt_route && t.origin == TakeOrigin::Render).count() as u64;
                // 同一镜同一线路第 N 次换一个种子（抽卡）；A / B 的第一次用同一个种子，对比才公平
                let seed = shot_seed(&req.project_id, &req.episode_id, &shot_id_for_path).wrapping_add(same_route);
                let take_path = shots_dir.join(format!("{}-{}-{}.mp4", shot_id_for_path, route_key, crate::jobs::now()));
                let mut pictures = 0;
                let mut picture_paths: Vec<String> = Vec::new();

                // **接上一镜：拿它的成片，拿不到就停，不静默降级。**
                //
                // 静默降级的后果是：提示词里写着 `<Video 1>`（合成那一步按
                // `continuity` 写进去的），而实际没传视频 —— H3 收到一个指向
                // 空处的标签，出来的东西说不清哪儿不对，**而且不报错**。
                // **挂了首帧就按关键帧出**，接上一镜那套不用了：画面已经被首帧锁住，
                // FL2VA 分区也不接参考视频。上一镜没出片不该拦住这一镜。
                let keyframes = shot.keyframes;
                // 长镜头里的镜头合成时不按首帧写（见 `ShotContext::keyframe_video`），单独出片也不能按首帧出
                let long_take = long_takes_snapshot.contains(&shot.scene_no);
                let keyframe_video = keyframes.drives_video() && !long_take;
                let long_shot = engine == "h3" && crate::long_shot::is_long(shot.duration);
                if engine == "h3" && !long_shot && keyframes.last && !keyframes.first {
                    return Err(Error::config(format!(
                        "第 {} 镜只挂了尾帧、没有首帧 —— 补一张首帧（出或导入都行），或者把尾帧摘掉",
                        shot.shot_num
                    )));
                }
                // **首帧出片却有人中途进场：拦下**（2026-10-07 用户定「A 加检查」，见 `shot_route`）
                if engine == "h3" && !long_shot {
                    if let Some(why) = crate::shot_route::keyframe_blocker(shot, &assets, long_take) {
                        return Err(Error::config(format!("第 {} 镜：{why}", shot.shot_num)));
                    }
                }
                // 「接住最后一帧」只在出首帧那一步起作用（上一镜末帧当这一镜首帧）：没首帧就出片 = 它被悄悄丢了，拦下说清楚
                if engine == "h3" && !keyframe_video && !long_shot && shot.effective_continuity() == Continuity::FirstFrame {
                    return Err(Error::config(format!(
                        "第 {} 镜设了「接住最后一帧」，但还没有首帧 —— 先点「出首帧」（会拿上一镜最后一帧当首帧），或者改成「运镜与光线」/「不接」",
                        shot.shot_num
                    )));
                }
                let continuity = match shot.effective_continuity() {
                    _ if engine == "h3" && keyframe_video => None,
                    Continuity::None => None,
                    c => {
                        let prev = storyboard_snapshot
                            .iter()
                            .filter(|s| s.shot_num < shot.shot_num)
                            .max_by_key(|s| s.shot_num)
                            .ok_or_else(|| Error::config(format!(
                                "第 {} 镜设了「{}」，但它前面没有镜头 —— 把它改成「不接」",
                                shot.shot_num, c.label()
                            )))?;
                        let take = continuity_source(prev).ok_or_else(|| Error::config(format!(
                            "第 {} 镜要接第 {} 镜（{}），而第 {} 镜还没出片 —— 先出上一镜，或者把这一镜改成「不接」",
                            shot.shot_num, prev.shot_num, c.label(), prev.shot_num
                        )))?;
                        if !std::path::Path::new(&take.path).is_file() {
                            return Err(Error::config(format!(
                                "第 {} 镜要接第 {} 镜，但那条片子的文件不在了：{}",
                                shot.shot_num, prev.shot_num, take.path
                            )));
                        }
                        Some((c, take.path.clone(), prev.duration.max(1.0)))
                    }
                };

                // **续接单**：交给 ComfyUI 之前记下结果落哪、这一次 take 怎么记（应用中途关掉，重开时照它收尾，见 `render_resume`）
                let burn_subtitles = req.engine_config.as_ref().map(|c| c.burn_subtitles).unwrap_or(render.burn_subtitles);
                let quality_label = if long_shot {
                    format!("超长镜 {} 段 · {} · {width}×{height}", crate::long_shot::plan(shot.duration).0, render.quality.label())
                } else if engine == "h3" {
                    // 首帧出片用首帧那一栏的画质 —— 记成参考那一栏的，出完看记录对不上（2026-10-07 复查）
                    if keyframe_video {
                        format!("首帧 · {} · {width}×{height}", render.keyframe_quality.label())
                    } else {
                        format!("参考 · {} · {width}×{height}", render.quality.label())
                    }
                } else {
                    format!("{engine} · {width}×{height}")
                };
                let note_resume = |pictures: usize, picture_paths: &[String]| {
                    let take = VideoTake {
                        route: prompt_route, path: take_path.to_string_lossy().to_string(), created_at: 0, seed, pictures,
                        picture_paths: picture_paths.to_vec(), quality: quality_label.clone(), prompt: prompt.clone(),
                        fields_hash: current_hash(&storyboard_snapshot, shot, &assets, &style, &long_takes_snapshot),
                        style: style.chars().take(200).collect(), origin: TakeOrigin::Render,
                    };
                    crate::jobs::note_resume(json!({
                        "project_id": req.project_id, "episode_id": req.episode_id, "shot_id": req.shot_id,
                        "out": take_path.to_string_lossy(), "generate_audio": generate_audio, "burn_subtitles": burn_subtitles, "take": take,
                    }));
                };
                if long_shot {
                    // **超长镜**：拆成几段等长窗口，一次交给 H3 的窗口链出完，H3 自己接成一条（见 `long_shot`）。
                    // 首帧、接上一镜都不用：窗口链只吃资产参考图，段与段靠重叠帧接
                    let (n, w) = crate::long_shot::plan(shot.duration);
                    let parts = crate::long_shot::split_prompt(&prompt);
                    if parts.len() != n {
                        return Err(Error::config(format!(
                            "第 {} 镜是超长镜（{} 秒，分 {n} 段出），提示词里却是 {} 段 —— 时长改过、或者手改时删了「{}」分隔行。重新合成这一镜再出片",
                            shot.shot_num, shot.duration, parts.len(), crate::long_shot::WINDOW_MARK
                        )));
                    }
                    if continuity.is_some() {
                        return Err(Error::config(format!("第 {} 镜是超长镜，不能接上一镜 —— 把它改成「不接」、重新合成再出片", shot.shot_num)));
                    }
                    let refs = render_reference_paths(config_dir, &req.project_id, &req.episode_id, &shots_dir, shot, &assets, true);
                    if refs.is_empty() {
                        return Err(Error::config(format!(
                            "第 {} 镜是超长镜，分段出要用人物 / 场景的资产图当参考，这一镜一张都挂不上 —— 先给出场的资产补图",
                            shot.shot_num
                        )));
                    }
                    for part in &parts {
                        render_refs_guard(shot.shot_num, part, refs.len(), false)?;
                    }
                    pictures = refs.len();
                    picture_paths = refs.iter().map(|p| p.to_string_lossy().to_string()).collect();
                    note_resume(pictures, &picture_paths);
                    let windows: Vec<(String, Vec<PathBuf>)> = parts.into_iter().map(|p| (p, refs.clone())).collect();
                    crate::agent_runtime::video_tool::render_long_take(
                        &windows, &take_path, w as f64, crate::long_take::CONTEXT_FRAMES,
                        width, height, seed, generate_audio, Some(render.quality.tier()), &[],
                    )
                    .await
                    .map_err(Error::runtime)?;
                } else if engine == "h3" && keyframe_video {
                    // 首帧 / 首尾帧（FL2VA）。人、场景、道具都已经画在首帧里了（出首帧那一步挂的资产图）
                    let end = Some(end_frame_path(&shots_dir, &shot_id_for_path)).filter(|p| keyframes.last && p.is_file());
                    pictures = 1 + usize::from(end.is_some());
                    render_refs_guard(shot.shot_num, &prompt, pictures, false)?;
                    picture_paths = std::iter::once(shot_img_path.to_string_lossy().to_string())
                        .chain(end.iter().map(|p| p.to_string_lossy().to_string()))
                        .collect();
                    note_resume(pictures, &picture_paths);
                    crate::agent_runtime::video_tool::render_shot_video(
                        &shot_img_path, &take_path, "h3", &prompt, shot.duration.max(1.0) as f64,
                        width, height, 24.0, seed, generate_audio, end.as_deref(), Some(render.keyframe_quality.tier()),
                    )
                    .await
                    .map_err(Error::runtime)?;
                } else if engine == "h3" {
                    // 挂哪几张、什么顺序，和合成提示词用的是**同一份**（prompt_input::ShotContext::slots）——
                    // 提示词里的 <Picture N> / 「参考图 N」和实际发出去的第 N 张才对得上
                    let refs = render_reference_paths(config_dir, &req.project_id, &req.episode_id, &shots_dir, shot, &assets, long_take);
                    pictures = refs.len();
                    picture_paths = refs.iter().map(|p| p.to_string_lossy().to_string()).collect();
                    render_refs_guard(shot.shot_num, &prompt, refs.len(), matches!(continuity, Some((Continuity::RefVideo, ..))))?;
                    note_resume(pictures, &picture_paths);
                    if refs.is_empty() {
                        crate::agent_runtime::video_tool::render_shot_video_text(&take_path, &prompt, shot.duration.max(1.0) as f64, width, height, seed, generate_audio, Some(render.quality.tier()))
                            .await
                            .map_err(Error::runtime)?;
                    } else {
                        // 尾段取最后 1 秒 / 24 帧 —— 2026-09-20 实测这一档就够
                        // （背景差 4.10，比同一镜头内部隔 6 秒的波动 51.05 还小）
                        let cont = continuity.as_ref().and_then(|(c, path, dur)| {
                            (*c == Continuity::RefVideo).then(|| crate::agent_runtime::weetodd::H3Continuity {
                                video_path: path.as_str(),
                                start_secs: (*dur as f64 - 1.0).max(0.0),
                                frames: 24,
                                fps: 24.0,
                            })
                        });
                        crate::agent_runtime::video_tool::render_shot_video_reference_with(&refs, &take_path, &prompt, shot.duration.max(1.0) as f64, width, height, seed, generate_audio, Some(render.quality.tier()), cont)
                            .await
                            .map_err(Error::runtime)?;
                    }
                } else {
                    // LTX 等：合成出来的两条线路都是按 H3 写的（`<Subject N>`、`<d>` 台词块），它不认 ——
                    // 原样发过去不报错，出来的东西和提示词对不上（2026-10-03 审查）
                    if has_h3_tags(&prompt) || prompt.contains("<d>") {
                        return Err(Error::config(format!(
                            "第 {} 镜的提示词是按 H3 写的（带 <Subject N> / <d> 台词标记），{engine} 不认 —— 出片引擎换回 H3，或者给这一镜手写一份提示词",
                            shot.shot_num
                        )));
                    }
                    note_resume(pictures, &picture_paths);
                    crate::agent_runtime::video_tool::render_shot_video(
                        &shot_img_path,
                        &take_path,
                        &engine,
                        &prompt,
                        shot.duration.max(1.0) as f64,
                        width,
                        height,
                        24.0,
                        seed,
                        generate_audio,
                        // **只在尾帧图确实存在时才走首尾帧。**
                        Some(end_frame_path(&shots_dir, &shot_id_for_path))
                            .filter(|p| p.is_file())
                            .as_deref(),
                        None,
                    )
                    .await
                    .map_err(Error::runtime)?;
                }
                shot_vid_path = take_path.clone();
                let quality = quality_label;
                // **把这一条是怎么出来的一起存下来。**
                // 只存路径和 seed 的话，重新合成提示词之后历史 take 就没法复现了 ——
                // 你看着两条片子不一样，却说不出当时发的是什么。
                shot.takes.push(VideoTake {
                    route: prompt_route,
                    path: take_path.to_string_lossy().to_string(),
                    created_at: crate::jobs::now(),
                    seed,
                    pictures,
                    picture_paths,
                    quality,
                    prompt: prompt.clone(),
                    fields_hash: current_hash(&storyboard_snapshot, shot, &assets, &style, &long_takes_snapshot),
                    style: style.chars().take(200).collect(),
                    origin: TakeOrigin::Render,
                });

                // 台词烧进画面。**失败要说出来**，不能默默交一段没字幕的片子。
                if burn_subtitles {
                    burn_dialogue(&shot_vid_path, shot)?;
                }
            }
            "wan2.1" => {
                return Err(Error::runtime(
                    "视频引擎「wan2.1」已于 2026-10-01 下架（权重已删）。请改用 ltx / minimax_h3（本机 WeeTodd/MLX，已闭环）。",
                ));
            }
            "hunyuan_i2v" | "svd" | "animatediff" => {
                return Err(Error::runtime(format!(
                    "视频引擎「{engine}」只在设置面板占位，后端还没有对应 ComfyUI I2V 工作流节点映射。请改用 ltx / minimax_h3（本机 WeeTodd/MLX，已闭环）或 liveportrait。"
                )));
            }
            other => {
                return Err(Error::runtime(format!(
                    "未知视频引擎「{other}」。当前可用：ltx（LTX 2.5）/ minimax_h3 / liveportrait（真实人物动作）/ camera_preview（静帧运镜预览）。"
                )));
            }
        }

        // 存路径，不存 base64 —— 一段 mp4 内联进 json 是最贵的一种。
        if !shot_vid_path.is_file() {
            return Err(Error::runtime("出片完成但没找到视频文件"));
        }
        let vid_url = shot_vid_path.to_string_lossy().to_string();
        let img_url = shot_img_path.to_string_lossy().to_string();
        shot.video_url = Some(vid_url.clone());
        let audio_url = shot.audio_url.clone().unwrap_or_default();
        // **把台词超容量写进状态。** 和「衔接未生效」同一个套路 ——
        // 出片成功但有个听不出来的问题时，状态是唯一会被看到的地方。
        shot.status = match &dialogue_warning {
            Some(w) => format!("ready(台词：{w})"),
            None => "ready".to_string(),
        };

        // **在锁里按最新的那份只改这一镜的出片结果。** 出片要几分钟到十几分钟，
        // 这期间合成提示词、改别的镜头、导演世界存场面都在写同一个文件 —— 拿开头读的整份写回去会把它们全冲掉。
        let (video_url, status, takes) = (shot.video_url.clone(), shot.status.clone(), shot.takes.clone());
        crate::episode_lock::update_file(&ep_file, |latest| {
            if let Some(target) = latest.storyboard.iter_mut().find(|s| s.id == req.shot_id) {
                target.video_url = video_url;
                target.status = status;
                target.takes = takes;
            }
            Ok(())
        })
        .map_err(|e| Error::runtime(format!("出片完成，但结果没存进分集：{e}")))?;

        return Ok(RenderShotResult {
            shot_id: req.shot_id,
            image_url: img_url,
            video_url: vid_url,
            audio_url,
            status: "ready".to_string(),
        });
    }

    // 3. 根据镜头运镜方式生成对应的电影级动态滤镜
    let vf_filter = ken_burns_filter(&shot.camera_movement, total_frames, width, height, fps);

    let dur_str = format!("{duration}");
    let ffmpeg_res = std::process::Command::new(ffmpeg_bin)
        .args([
            "-y",
            "-loop",
            "1",
            "-i",
            shot_img_path.to_str().unwrap_or_default(),
            "-vf",
            &vf_filter,
            "-c:v",
            "libx264",
            "-t",
            &dur_str,
            "-pix_fmt",
            "yuv420p",
            shot_vid_path.to_str().unwrap_or_default(),
        ])
        .output();

    let mut ffmpeg_error = None;
    match ffmpeg_res {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr)
                .chars()
                .take(1200)
                .collect::<String>();
            tracing::warn!(%stderr, "FFmpeg 运镜滤镜失败，降级为静态保底视频");
            ffmpeg_error = Some(stderr);
        }
        Err(e) => {
            tracing::warn!("FFmpeg 运镜生成失败，降级为静态保底视频：{e}");
            ffmpeg_error = Some(e.to_string());
        }
    }

    if ffmpeg_error.is_some() || !shot_vid_path.is_file() {
        let fallback_filter = format!(
            "scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2,format=yuv420p",
            width, height, width, height
        );
        let fallback = std::process::Command::new(ffmpeg_bin)
            .args([
                "-y",
                "-loop",
                "1",
                "-i",
                shot_img_path.to_str().unwrap_or_default(),
                "-vf",
                &fallback_filter,
                "-c:v",
                "libx264",
                "-t",
                &dur_str,
                "-pix_fmt",
                "yuv420p",
                shot_vid_path.to_str().unwrap_or_default(),
            ])
            .output();

        match fallback {
            Ok(out) if out.status.success() && shot_vid_path.is_file() => {}
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr)
                    .chars()
                    .take(1600)
                    .collect::<String>();
                return Err(Error::runtime(format!(
                    "FFmpeg 生成镜头视频失败。主运镜错误：{}；保底视频错误：{}",
                    ffmpeg_error.unwrap_or_else(|| "未知".to_string()),
                    stderr
                )));
            }
            Err(e) => {
                return Err(Error::runtime(format!(
                    "无法执行 FFmpeg 生成镜头视频：{e}。主运镜错误：{}",
                    ffmpeg_error.unwrap_or_else(|| "未知".to_string())
                )));
            }
        }
    }

    use base64::Engine;
    let vid_bytes = std::fs::read(&shot_vid_path).unwrap_or_default();
    let vid_url = if !vid_bytes.is_empty() {
        format!(
            "data:video/mp4;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&vid_bytes)
        )
    } else {
        String::new()
    };

    if vid_url.is_empty() {
        return Err(Error::runtime(
            "FFmpeg 执行完成但没有产出可用视频文件".to_string(),
        ));
    }

    // **存路径，不存 base64。** 见 `resolve_episode_media` 的注释：
    // 内联进 episode.json 的话单集能涨到几十 MB，而每次失焦保存都要
    // 整份读写它。界面走 `anima-media://` 直接读本机路径。
    let img_url = shot_img_path.to_string_lossy().to_string();

    shot.video_url = if vid_url.is_empty() {
        None
    } else {
        Some(vid_url.clone())
    };
    let audio_url = shot.audio_url.clone().unwrap_or_default();
    shot.status = "ready".to_string();
    let video_url_saved = shot.video_url.clone();

    // 渲完就把末帧抽出来，给下一镜接续用。
    //
    // **无条件抽，不看下一镜有没有打开 continues_previous** —— 用户可能渲完
    // 第 3 镜之后才决定第 4 镜要接上，那时再回头补抽就得重渲一遍。
    // 一张 PNG 的成本远低于此。
    if !vid_url.is_empty() {
        let _ = extract_last_frame(
            ffmpeg_bin,
            &shot_vid_path,
            &last_frame_path(&shots_dir, &shot_id_for_path),
        );
    }

    // 同上：ffmpeg 跑的这几秒里别处可能写过，只改这一镜
    let _ = crate::episode_lock::update_file(&ep_file, |latest| {
        if let Some(target) = latest.storyboard.iter_mut().find(|s| s.id == req.shot_id) {
            target.video_url = video_url_saved;
            target.status = "ready".to_string();
        }
        Ok(())
    });

    Ok(RenderShotResult {
        shot_id: req.shot_id,
        image_url: img_url,
        video_url: vid_url,
        audio_url,
        status: "ready".to_string(),
    })
}

fn cinema_spoken_dialogue(raw: &str) -> String {
    let mut text = raw.trim().to_string();
    if text.is_empty() {
        return String::new();
    }

    let mut quoted = Vec::new();
    let mut start = None;
    for (idx, ch) in text.char_indices() {
        if ch == '“' {
            start = Some(idx + ch.len_utf8());
        } else if ch == '”' {
            if let Some(s) = start.take() {
                let part = text[s..idx].trim();
                if !part.is_empty() {
                    quoted.push(part.to_string());
                }
            }
        }
    }
    if !quoted.is_empty() {
        return quoted.join(" ");
    }

    while let Some(open) = text
        .find('[')
        .or_else(|| text.find('【'))
        .or_else(|| text.find('（'))
    {
        let close_char = match text[open..].chars().next().unwrap_or('[') {
            '[' => ']',
            '【' => '】',
            '（' => '）',
            _ => ']',
        };
        if let Some(close_rel) = text[open..].find(close_char) {
            let close = open + close_rel + close_char.len_utf8();
            text.replace_range(open..close, "");
        } else {
            break;
        }
    }

    if let Some(pos) = text.find('：') {
        text = text[pos + '：'.len_utf8()..].to_string();
    } else if let Some(pos) = text.find(':') {
        text = text[pos + ':'.len_utf8()..].to_string();
    }

    text.trim()
        .trim_matches('“')
        .trim_matches('”')
        .trim_matches('"')
        .trim()
        .to_string()
}

fn mime_for_audio(bytes: &[u8]) -> &'static str {
    if is_wav_audio(bytes) {
        "audio/wav"
    } else if bytes.starts_with(b"fLaC") {
        "audio/flac"
    } else if bytes.starts_with(b"ID3") || bytes.first().copied() == Some(0xFF) {
        "audio/mpeg"
    } else {
        "application/octet-stream"
    }
}

fn is_wav_audio(bytes: &[u8]) -> bool {
    bytes.starts_with(b"RIFF") && matches!(bytes.get(8..12), Some(chunk) if chunk == b"WAVE")
}

pub(crate) fn ffmpeg_binary() -> &'static str {
    for candidate in ["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg"] {
        if Path::new(candidate).is_file() {
            return candidate;
        }
    }
    "ffmpeg"
}

fn python_binary() -> &'static str {
    for candidate in [
        "/Users/you/AI/ComfyUI/.venv/bin/python",
        "/Users/you/AI/ComfyUI/.venv/bin/python3",
        "python3",
        "python",
    ] {
        if candidate.starts_with('/') {
            if Path::new(candidate).is_file() {
                return candidate;
            }
        } else {
            return candidate;
        }
    }
    "python3"
}

/// Anima 的仓库根。**候选和其它项目目录共用同一份**（[`crate::app_root`]）——
/// 各写一份正是这个坑连修三轮的原因：只从 cwd 往上找的话，
/// 双击的 `.app`（cwd 是 `/`）永远找不到，而症状是「风格缩略图一张不显示」
/// 这种和路径毫无关联的样子。
pub fn workspace_root() -> Option<PathBuf> {
    crate::app_root::repo_root()
}

/// `camera_preview` 那条路的 Ken-Burns 滤镜 —— 一张静帧假装成一段运镜。
///
/// # 为什么按 id 分派，而不是按中文字分派
///
/// 原来写的是 `cam.contains("推")`。运镜换成词表 id（`push_in`）之后，
/// 那些 `contains` **一条都不会再命中**，于是每一个运镜都掉进最后的
/// 「固定」分支 —— 视频照出、不报错，只是全片没有任何运镜。
/// 这正是这个仓库最典型的失败样子。
///
/// 16 条运镜归到 5 种平面运动。一张静帧上能做的就这么多：
/// 环绕 / 跟拍 / 手持是真三维运动，静帧假装不了，落到「呼吸感」那一档。
fn ken_burns_filter(move_id: &str, total_frames: u32, width: u32, height: u32, fps: u32) -> String {
    // 推 / 拉：缩放
    let zoom_in = format!(
        "scale=-2:4*ih,zoompan=z='min(zoom+0.0015,1.25)':x='iw/2-(iw/zoom/2)':y='ih/2-(ih/zoom/2)':d={total_frames}:s={width}x{height}:fps={fps}"
    );
    let zoom_out = format!(
        "scale=-2:4*ih,zoompan=z='if(lte(zoom,1.0),1.25,max(1.001,zoom-0.0015))':x='iw/2-(iw/zoom/2)':y='ih/2-(ih/zoom/2)':d={total_frames}:s={width}x{height}:fps={fps}"
    );
    // 横向：从右往左 / 从左往右扫
    let go_left = format!(
        "scale=2*iw:-2,zoompan=z=1.12:x='if(lte(on,1),iw*0.2,max(0,x-1.5))':y='ih/2-(ih/zoom/2)':d={total_frames}:s={width}x{height}:fps={fps}"
    );
    let go_right = format!(
        "scale=2*iw:-2,zoompan=z=1.12:x='if(lte(on,1),0,min(iw*0.2,x+1.5))':y='ih/2-(ih/zoom/2)':d={total_frames}:s={width}x{height}:fps={fps}"
    );
    // 纵向。**上下要分开** —— 原来两个方向共用一条滤镜，
    // 选「上摇」和选「下摇」出来的是同一段。
    let go_up = format!(
        "scale=-2:2*ih,zoompan=z=1.12:x='iw/2-(iw/zoom/2)':y='if(lte(on,1),ih*0.2,max(0,y-1.5))':d={total_frames}:s={width}x{height}:fps={fps}"
    );
    let go_down = format!(
        "scale=-2:2*ih,zoompan=z=1.12:x='iw/2-(iw/zoom/2)':y='if(lte(on,1),0,min(ih*0.2,y+1.5))':d={total_frames}:s={width}x{height}:fps={fps}"
    );
    // 固定：微动态呼吸感
    let breathe = format!(
        "scale=-2:4*ih,zoompan=z='min(zoom+0.0004,1.05)':x='iw/2-(iw/zoom/2)':y='ih/2-(ih/zoom/2)':d={total_frames}:s={width}x{height}:fps={fps}"
    );

    match move_id.trim() {
        // 平移里的前后 + 纯镜头的变焦，在静帧预览上**表现相同**：
        // 都是画面放大/缩小。差别在透视，而一张静帧没有透视可变。
        // 这不是偷懒 —— 出片那条路（H3/LTX）拿到的是两句不同的提示词，
        // 只有这条 ffmpeg 预览合并。
        "push_in" | "zoom_in" => zoom_in,
        "pull_out" | "zoom_out" => zoom_out,
        "pan_left" | "truck_left" => go_left,
        "pan_right" | "truck_right" | "whip_pan" => go_right,
        "tilt_up" | "pedestal_up" => go_up,
        "tilt_down" | "pedestal_down" => go_down,
        // static / follow / orbit* / handheld / rack_focus / dolly_zoom，
        // 以及认不出的值（用户自己打的词）都走这一档。
        //
        // **新加运镜词时要回来看一眼这里。** 漏了的话它静默退到「呼吸感」——
        // 用户选了「变焦推近」而预览一动不动，看不出是哪儿错了。
        // 有测试钉着「词表里每一条都在这儿有着落或明确走兜底」。
        _ => breathe,
    }
}


/// 草图文件名。**和关键帧分开** —— 草图不该覆盖已经精修过的关键帧。
pub(crate) fn sketch_path(shots_dir: &Path, shot_id: &str) -> PathBuf {
    shots_dir.join(format!("{shot_id}_sketch.png"))
}

/// 这一镜的台词拆成 H3 要的「说话人 + 台词」。
///
/// `dialogue` 存的是剧本原文，一行一句，本来就写成 `名字：台词`。
/// 拆不出说话人时回落到这一镜的**第一个出场角色** —— 只有一个人在画面里时
/// 这个回落是对的；两个人时拆不出来说明剧本本身没写清楚，
/// 那时候宁可让它落在第一个人身上，也好过完全不给 ID（不给 ID 模型会乱指）。
pub(crate) fn shot_dialogue_lines(
    config_dir: &Path,
    shot: &StoryboardShot,
    scope: &crate::assets::RefScope,
) -> Vec<crate::h3_prompt::H3Line> {
    // 出场角色的显示名，用于回落
    let fallback = crate::assets::resolve_refs_in(config_dir, &shot.prompt, scope)
        .into_iter()
        .find(|r| r.kind == crate::assets::AssetKind::Character)
        .map(|r| r.name)
        .unwrap_or_default();

    // **结构化的那份优先。** 它带说话人绑定、音色和画外音标记；
    // `dialogue` 那个字符串是人写人读的剧本原文，拆出来的信息少一截。
    if !shot.dialogue_lines.is_empty() {
        return shot
            .dialogue_lines
            .iter()
            .filter(|d| !d.line.trim().is_empty())
            .map(|d| crate::h3_prompt::H3Line {
                speaker: if d.speaker.trim().is_empty() {
                    fallback.clone()
                } else {
                    d.speaker.trim().to_string()
                },
                text: d.line.trim().to_string(),
                voice: d.voice.trim().to_string(),
                off_screen: d.off_screen,
            })
            .collect();
    }

    shot.dialogue
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| match crate::h3_prompt::split_speaker(l) {
            Some((speaker, text, voice)) => crate::h3_prompt::H3Line {
                speaker,
                text,
                voice,
                off_screen: false,
            },
            None => crate::h3_prompt::H3Line {
                speaker: fallback.clone(),
                text: l.trim().to_string(),
                voice: String::new(),
                off_screen: false,
            },
        })
        .collect()
}

/// 这一镜要用哪些参考图。
///
/// 两个来源合并去重：提示词里的 `@` 引用，和 `character_ids` 里存的资产 id。
/// **不能只认提示词** —— 改了资产名字标签就失配了；
/// **也不能只认 id** —— 刚打完 `@` 还没保存的那一刻只有文本。
pub(crate) fn shot_reference_images(config_dir: &Path, shot: &StoryboardShot, scope: &crate::assets::RefScope) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut push = |p: PathBuf| {
        if !out.contains(&p) {
            out.push(p);
        }
    };
    for r in crate::assets::resolve_refs_in(config_dir, &shot.prompt, scope) {
        if let Some(p) = r.image_path {
            push(PathBuf::from(p));
        }
    }
    let mut wanted: Vec<&str> = Vec::new();
    let ids = shot.all_character_ids();
    wanted.extend(ids.iter().map(String::as_str));
    if let Some(s) = &shot.scene_id {
        wanted.push(s);
    }
    wanted.extend(shot.prop_ids.iter().map(String::as_str));
    if !wanted.is_empty() {
        for sc in crate::assets::all_scopes(config_dir) {
            for a in crate::assets::load_store(config_dir, &sc).assets {
                if !wanted.contains(&a.id.as_str()) {
                    continue;
                }
                if let Some(v) = a.unit.selected_image() {
                    push(crate::assets::variant_abs_path(config_dir, &sc, v));
                }
            }
        }
    }
    // **上限取自单一来源，不再写死。**
    //
    // 这里原来是 `truncate(4)`，注释还断言「参考图有 4 张上限」——
    // 那句话是错的。节点自己声明 `max=9`
    // （`video_tool::MAX_REFERENCE_IMAGES`），MiniMax 官方规格也是
    // reference_image 最多 9 张。截在 4 的后果是：一镜里引用 5 个以上
    // 资产时，第 5 张往后**静默丢掉** —— 图没发出去，而提示词那边
    // 按「这张图确实发出去了」去删描述，两头都没了。
    out.truncate(crate::agent_runtime::video_tool::MAX_REFERENCE_IMAGES);
    out
}

/// 保存分集时，把提示词里的 `@` 引用回写成资产 id。
///
/// **标签是书写形式，id 才是索引。** 引用完整性扫描（删资产前查
/// 「哪几个镜头在用它」）读的是 `character_ids` / `scene_id` / `prop_ids` ——
/// 只靠正则扫提示词的话，用户一改资产名字标签就失配，
/// 删除保护会**静默失效**。
///
/// 对不上库的引用不写 id（那就是「提示词里提了个不存在的名字」），
/// 把提示词里**没打标签的资产名**补成标签。
///
/// # 为什么必须有这道兜底
///
/// 提示词里明写着要用 `[角色:名字]`，而实测抽出来的一整集**一个标签
/// 都没有** —— 名字写成了普通文字（「林婉清伏案趴在桌上」）。
///
/// 后果不是排版难看：**没有标签就不挂参考图**（`resolve_refs` 认的是
/// 标签），那个角色的长相出图时全靠模型现编，每一镜一张不同的脸，
/// 而且**不报错**。
///
/// 改示例字面量之后模型会打了，但这条路太贵了不能只靠它 ——
/// 一次没打就是一集废片。
///
/// # 长名字优先，避免把长名字咬断
///
/// 「林婉清（盛唐）」和「林婉清」同时存在时，先匹配长的 ——
/// 反过来会把前者切成 `[角色:林婉清]（盛唐）`，那是个不存在的资产。
///
/// # 已经打过标签的不动
///
/// 扫描时跳过 `[...]` 里面的内容，否则会把标签里的名字再包一层。
pub fn backfill_asset_tags(config_dir: &Path, prompt: &str) -> String {
    if prompt.trim().is_empty() {
        return prompt.to_string();
    }
    // 名字 → 前缀。长的排前面。
    let mut names: Vec<(String, &'static str)> = Vec::new();
    for scope in crate::assets::all_scopes(config_dir) {
        for a in crate::assets::load_store(config_dir, &scope).assets {
            let n = a.name.trim().to_string();
            if n.is_empty() || names.iter().any(|(x, _)| *x == n) {
                continue;
            }
            names.push((
                n,
                match a.kind {
                    crate::assets::AssetKind::Character => "角色",
                    crate::assets::AssetKind::Scene => "场景",
                    crate::assets::AssetKind::Prop => "道具",
                },
            ));
        }
    }
    names.sort_by(|a, b| b.0.chars().count().cmp(&a.0.chars().count()));
    if names.is_empty() {
        return prompt.to_string();
    }

    let mut out = String::with_capacity(prompt.len());
    let mut rest = prompt;
    'outer: while !rest.is_empty() {
        // 已经在标签里的整段原样搬过去
        if rest.starts_with('[') {
            if let Some(end) = rest.find(']') {
                out.push_str(&rest[..=end]);
                rest = &rest[end + 1..];
                continue;
            }
        }
        for (name, prefix) in &names {
            if rest.starts_with(name.as_str()) {
                out.push_str(&format!("[{prefix}:{name}]"));
                rest = &rest[name.len()..];
                continue 'outer;
            }
        }
        let ch = rest.chars().next().expect("非空");
        out.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    out
}

/// 去掉造型分身的括号后缀：`林婉清（盛唐）` → `林婉清`。中英括号都认。
fn base_character_name(name: &str) -> &str {
    let n = name.trim();
    for open in ['（', '('] {
        if let Some(i) = n.find(open) {
            if n.ends_with('）') || n.ends_with(')') {
                return n[..i].trim_end();
            }
        }
    }
    n
}

/// 也不清空已有的 id —— 手动绑过的关系不该被一次改词冲掉。
pub fn sync_shot_asset_ids(config_dir: &Path, episode: &mut EpisodeData, scope: &crate::assets::RefScope) {
    for shot in episode.storyboard.iter_mut() {
        let resolved = crate::assets::resolve_refs_in(config_dir, &shot.prompt, scope);
        if resolved.is_empty() {
            continue;
        }
        for r in resolved {
            let Some(id) = r.asset_id else { continue };
            match r.kind {
                crate::assets::AssetKind::Character => {
                    if !shot.character_ids.contains(&id) {
                        shot.character_ids.push(id);
                    }
                }
                crate::assets::AssetKind::Scene => shot.scene_id = Some(id),
                crate::assets::AssetKind::Prop => {
                    if !shot.prop_ids.contains(&id) {
                        shot.prop_ids.push(id);
                    }
                }
            }
        }
    }
}

/// 这一镜的 seed。**同一镜重渲要出同一段** —— 随机 seed 会让人以为
/// 改了提示词有效果，其实只是换了个数。
pub(crate) fn shot_seed(project_id: &str, episode_id: &str, shot_id: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    project_id.hash(&mut h);
    episode_id.hash(&mut h);
    shot_id.hash(&mut h);
    h.finish() % 1_000_000_000
}

fn render_liveportrait_shot(
    shot: &StoryboardShot,
    shot_img_path: &Path,
    shot_vid_path: &Path,
    ep_dir: &Path,
) -> Result<()> {
    let img_bytes = std::fs::read(shot_img_path)
        .map_err(|e| Error::runtime(format!("读取镜头关键帧失败：{e}")))?;
    if img_bytes.starts_with(b"<svg") {
        return Err(Error::runtime(
            "当前关键帧是 SVG 占位图，不是真实人物图。请先用 ComfyUI 生成真实关键帧，再生成 LivePortrait 动态视频。"
        ));
    }

    let root =
        workspace_root().unwrap_or_else(|| PathBuf::from("/Users/you/AI/workspace/Anima"));
    let runner = root.join("scripts/liveportrait_generate_all.py");
    if !runner.is_file() {
        return Err(Error::runtime(format!(
            "找不到 LivePortrait 生成脚本：{}",
            runner.display()
        )));
    }

    let liveportrait_root =
        Path::new("/Users/you/AI/ComfyUI/custom_nodes/official_liveportrait");
    if !liveportrait_root.is_dir() {
        return Err(Error::runtime(format!(
            "未找到 official_liveportrait：{}。请先安装 ComfyUI LivePortrait 节点或切换到「运镜预览」。",
            liveportrait_root.display()
        )));
    }

    let out_dir = ep_dir.join("liveportrait").join(&shot.id);
    let _ = std::fs::remove_dir_all(&out_dir);
    std::fs::create_dir_all(&out_dir)
        .map_err(|e| Error::runtime(format!("创建 LivePortrait 输出目录失败：{e}")))?;

    let safe_id = format!("{}_{}", shot.character_id.replace('/', "_"), shot.id);
    let mode = if cinema_spoken_dialogue(&shot.dialogue).is_empty() {
        "idle"
    } else {
        "speaking"
    };
    let intensity = if shot.dialogue.contains('！')
        || shot.dialogue.contains('!')
        || shot.dialogue.contains('？')
        || shot.dialogue.contains('?')
    {
        "expressive"
    } else {
        "balanced"
    };
    let duration = format!("{:.2}", shot.duration.max(1.0));

    let output = std::process::Command::new(python_binary())
        .args([
            runner.to_str().unwrap_or_default(),
            "--source",
            shot_img_path.to_str().unwrap_or_default(),
            "--char_id",
            &safe_id,
            "--output_dir",
            out_dir.to_str().unwrap_or_default(),
            "--intensity",
            intensity,
            "--duration",
            &duration,
        ])
        .output()
        .map_err(|e| Error::runtime(format!("启动 LivePortrait 失败：{e}")))?;

    let candidate = out_dir.join(format!("{}_{}.mp4", safe_id, mode));
    if !output.status.success() || !candidate.is_file() {
        let stdout = String::from_utf8_lossy(&output.stdout)
            .chars()
            .take(1000)
            .collect::<String>();
        let stderr = String::from_utf8_lossy(&output.stderr)
            .chars()
            .take(1600)
            .collect::<String>();
        return Err(Error::runtime(format!(
            "LivePortrait 没有生成可用视频（模式：{mode}）。stdout: {stdout} stderr: {stderr}"
        )));
    }

    std::fs::copy(&candidate, shot_vid_path)
        .map_err(|e| Error::runtime(format!("保存 LivePortrait 镜头视频失败：{e}")))?;
    Ok(())
}

/// 为单个分镜对白生成可落盘的 TTS 音频。
#[tauri::command]
pub async fn cinema_generate_shot_audio(
    state: State<'_, AppState>,
    req: RenderShotReq,
) -> Result<RenderShotResult> {
    cinema_generate_shot_audio_with_deps(
        &state.config_dir,
        state.gateway.as_ref(),
        state.http.clone(),
        req,
    )
    .await
}

pub async fn cinema_generate_shot_audio_with_deps(
    config_dir: &Path,
    gateway: &crate::llm::gateway::Gateway,
    http: reqwest::Client,
    req: RenderShotReq,
) -> Result<RenderShotResult> {
    let root = cinema_projects_dir(config_dir);
    let proj_dir = root.join(&req.project_id);
    let ep_dir = proj_dir.join("episodes").join(&req.episode_id);
    let audio_dir = ep_dir.join("audio");
    let shots_dir = ep_dir.join("shots");
    let _ = std::fs::create_dir_all(&audio_dir);

    let ep_file = ep_dir.join("episode.json");
    if !ep_file.is_file() {
        return Err(Error::config("分集不存在"));
    }

    let ep: EpisodeData =
        serde_json::from_str(&std::fs::read_to_string(&ep_file).unwrap_or_default())
            .map_err(|e| Error::config(format!("读取分集失败：{e}")))?;

    let shot_snapshot = ep
        .storyboard
        .iter()
        .find(|s| s.id == req.shot_id)
        .cloned()
        .ok_or_else(|| Error::config(format!("找不到分镜「{}」", req.shot_id)))?;

    let spoken = cinema_spoken_dialogue(&shot_snapshot.dialogue);
    if spoken.is_empty() {
        return Err(Error::runtime(format!(
            "Shot #{} 没有可配音对白，请先在分镜对白里写入台词",
            shot_snapshot.shot_num
        )));
    }

    let (selected, tts_config) = {
        let config = gateway.config();
        let config = config.read().await;
        let selected = config
            .tts_engines
            .iter()
            .find(|e| e.id == config.tts_engine)
            .cloned()
            .ok_or_else(|| {
                Error::config(format!(
                    "找不到 TTS 引擎「{}」，请在设置里重新选择",
                    config.tts_engine
                ))
            })?;
        let mut tts_config = config.voice.tts.clone();
        tts_config.voice = crate::tts::effective_voice(&selected);
        (selected, tts_config)
    };

    let engine = crate::tts::engine(&selected, http, &crate::tts::voices::voices_dir(config_dir))?;
    let speech = engine.synthesize(&spoken, &tts_config).await?;
    if speech.audio.is_empty() {
        return Err(Error::runtime("TTS 返回了空音频".to_string()));
    }

    let mut audio_bytes = speech.audio;
    let audio_path = audio_dir.join(format!("{}.wav", shot_snapshot.id));
    if !is_wav_audio(&audio_bytes) {
        let source_path = audio_dir.join(format!("{}.source_audio", shot_snapshot.id));
        std::fs::write(&source_path, &audio_bytes)
            .map_err(|e| Error::runtime(format!("写入临时音频失败：{e}")))?;
        let out = std::process::Command::new(ffmpeg_binary())
            .args([
                "-y",
                "-i",
                source_path.to_str().unwrap_or_default(),
                "-ac",
                "1",
                "-ar",
                "24000",
                "-c:a",
                "pcm_s16le",
                audio_path.to_str().unwrap_or_default(),
            ])
            .output();
        let _ = std::fs::remove_file(&source_path);
        match out {
            Ok(o) if o.status.success() && audio_path.is_file() => {
                audio_bytes = std::fs::read(&audio_path)
                    .map_err(|e| Error::runtime(format!("读取转码 WAV 失败：{e}")))?;
            }
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr)
                    .chars()
                    .take(1200)
                    .collect::<String>();
                return Err(Error::runtime(format!("TTS 音频转 WAV 失败：{stderr}")));
            }
            Err(e) => return Err(Error::runtime(format!("执行 FFmpeg 音频转码失败：{e}"))),
        }
    } else {
        std::fs::write(&audio_path, &audio_bytes)
            .map_err(|e| Error::runtime(format!("写入音频文件失败：{e}")))?;
    }

    use base64::Engine;
    let audio_url = format!(
        "data:{};base64,{}",
        mime_for_audio(&audio_bytes),
        base64::engine::general_purpose::STANDARD.encode(&audio_bytes)
    );
    let image_url = {
        let img_path = shots_dir.join(format!("{}.png", shot_snapshot.id));
        match std::fs::read(&img_path) {
            Ok(bytes) if !bytes.is_empty() => {
                if img_path.extension().and_then(|e| e.to_str()) == Some("svg")
                    || bytes.starts_with(b"<svg")
                {
                    format!(
                        "data:image/svg+xml;base64,{}",
                        base64::engine::general_purpose::STANDARD.encode(&bytes)
                    )
                } else {
                    format!(
                        "data:image/png;base64,{}",
                        base64::engine::general_purpose::STANDARD.encode(&bytes)
                    )
                }
            }
            _ => shot_snapshot.image_url.clone().unwrap_or_default(),
        }
    };
    let video_url = shot_snapshot.video_url.clone().unwrap_or_default();

    // TTS 要几秒到几十秒，开头读的那份早就旧了 —— 在锁里按最新的那份只改这一镜
    crate::episode_lock::update_file(&ep_file, |latest| {
        if let Some(shot) = latest.storyboard.iter_mut().find(|s| s.id == req.shot_id) {
            shot.audio_url = Some(audio_url.clone());
            shot.status = "ready".to_string();
        }
        Ok(())
    })
    .map_err(|e| Error::runtime(format!("保存分集音频状态失败：{e}")))?;

    Ok(RenderShotResult {
        shot_id: req.shot_id,
        image_url,
        video_url,
        audio_url,
        status: "ready".to_string(),
    })
}

/// 把镜头提示词里的 `[角色:名字]` 标签解析成资产 id，回填进镜头。
///
/// # 为什么解析完还要把认不出的报出去
///
/// 模型再怎么约束也会编名字。编出来的标签**长得和真标签一模一样**，
/// 界面上是一颗一样的彩色圆点 —— 但它挂不上任何参考图，
/// 出片时那一镜的人物身份就是随机的。这个失败全程不报错，
/// 所以必须在生成完那一刻就说出来。
///
/// 回填 id 而不是只留文本：文本会被用户改，改了标签就失配；
/// id 是稳的。两个来源在 `shot_reference_images` 里合并去重。
pub fn bind_shot_refs(config_dir: &Path, shots: &mut [StoryboardShot], scope: &crate::assets::RefScope) -> Vec<String> {
    let mut unknown: Vec<String> = Vec::new();
    for shot in shots.iter_mut() {
        let mut chars: Vec<String> = Vec::new();
        let mut props: Vec<String> = Vec::new();
        let mut scene: Option<String> = None;
        for r in crate::assets::resolve_refs_in(config_dir, &shot.prompt, scope) {
            let Some(id) = r.asset_id else {
                let label = format!("{}：{}", r.kind.tag_label(), r.name);
                if !unknown.contains(&label) {
                    unknown.push(label);
                }
                continue;
            };
            match r.kind {
                crate::assets::AssetKind::Character => {
                    if !chars.contains(&id) {
                        chars.push(id);
                    }
                }
                crate::assets::AssetKind::Prop => {
                    if !props.contains(&id) {
                        props.push(id);
                    }
                }
                // 一镜只有一个场景 —— 多写了以第一个为准，
                // 后面的当没看见比随机挑一个强。
                crate::assets::AssetKind::Scene => {
                    if scene.is_none() {
                        scene = Some(id);
                    }
                }
            }
        }
        // 旧的单数字段跟着写，读旧数据的地方还在用它。
        shot.character_id = chars.first().cloned().unwrap_or_default();
        shot.character_ids = chars;
        shot.prop_ids = props;
        shot.scene_id = scene;
    }
    unknown
}

/// 这一镜的音频从哪来。
///
/// **有 TTS 就用 TTS；没有但源片自带音轨就保留源片的；都没有才补静音。**
///
/// 原来无条件走「TTS 或静音」，于是 LTX/H3 生成的**同步中文台词被静音顶掉** ——
/// 2026-09-04 复现：源片 mean_volume −35.5 dB，过一遍这步变 −91.0 dB
/// （数字静音），而 ffprobe 照样报 `aac,audio`、任务照样成功。
/// 整集拼出来是哑片，而同步中文台词正是这条链路的招牌能力。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ShotAudio {
    /// 用单独配的 TTS 音轨
    Tts,
    /// 用源片自带的（模型生成的台词）
    Source,
    /// 两者都没有，补静音占位（concat 要求每段都有音轨）
    Silence,
}

pub(crate) fn pick_shot_audio(has_tts: bool, source_has_audio: bool) -> ShotAudio {
    match (has_tts, source_has_audio) {
        (true, _) => ShotAudio::Tts,
        (false, true) => ShotAudio::Source,
        (false, false) => ShotAudio::Silence,
    }
}

/// 把任意尺寸的片段塞进目标画幅：等比缩放 + 黑边补齐，**不裁切也不拉伸**。
///
/// `-f concat` 要求各输入宽高一致。同一集里各镜走同一条渲染路径时本来就一致，
/// 但回落到角色 idle/speaking 视频那条（见下）几乎肯定是另一个分辨率 ——
/// 不归一的话 concat 直接出错或出花屏。
///
/// 宽高都强制偶数（`libx264` + `yuv420p` 要求），否则编码器会拒。
pub(crate) fn fit_filter(w: u32, h: u32) -> String {
    let (w, h) = (w.max(2) / 2 * 2, h.max(2) / 2 * 2);
    format!(
        "scale={w}:{h}:force_original_aspect_ratio=decrease,\
         pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color=black,setsar=1"
    )
}

/// 片段进整集画幅用哪种方式。
///
/// 比例只差一点（H3 要宽高是 32 的倍数，1280×736 比 16:9 窄 2%）就**铺满、裁掉那一丝**——
/// 加黑边的话左右各一条 20 px 的黑线，进剪辑软件还得手动去。
/// 差得多（竖屏片放进横屏的集）才加黑边，否则会把人裁掉。
pub(crate) fn frame_filter(src: Option<(u32, u32)>, w: u32, h: u32) -> String {
    let close = src.filter(|(sw, sh)| *sw > 0 && *sh > 0).is_some_and(|(sw, sh)| {
        let r = (sw as f64 / sh as f64) / (w.max(1) as f64 / h.max(1) as f64);
        (r - 1.0).abs() <= 0.04
    });
    if !close {
        return fit_filter(w, h);
    }
    let (w, h) = (w.max(2) / 2 * 2, h.max(2) / 2 * 2);
    format!("scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h},setsar=1")
}

/// 连贯拼接整集生成的所有分镜视频为剧本全集视频流
#[tauri::command]
pub async fn cinema_stitch_episode(
    state: State<'_, AppState>,
    project_id: String,
    episode_id: String,
) -> Result<String> {
    cinema_stitch_episode_with_config_dir(&state.config_dir, project_id, episode_id).await
}

pub async fn cinema_stitch_episode_with_config_dir(
    config_dir: &Path,
    project_id: String,
    episode_id: String,
) -> Result<String> {
    stitch_episode(config_dir, project_id, episode_id, false, false, |_, _, _| {}).await.map(|(url, _)| url)
}

/// 合成一集的后台任务版。超分时一镜要一两分钟，整集几十分钟 —— 前端等不住，
/// 切页、刷新都会断。登记进任务表，按镜报进度，完了刷新分集。
#[tauri::command]
pub fn cinema_stitch_episode_start(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    project_id: String,
    episode_id: String,
    upscale: bool,
    cover: bool,
) -> Result<crate::jobs::Job> {
    start_stitch(Some(app), state.config_dir.clone(), project_id, episode_id, upscale, cover)
}

/// [`cinema_stitch_episode_start`] 的本体（Chat 的 `studio_stitch_episode` 也走这里）
pub fn start_stitch(app: Option<tauri::AppHandle>, config_dir: PathBuf, project_id: String, episode_id: String, upscale: bool, cover: bool) -> Result<crate::jobs::Job> {
    let title = if upscale { "合成一集 · 超分 1080P" } else { "合成一集" };
    let job_id = crate::jobs::start_exclusive(&config_dir, crate::jobs::JobKind::Stitch, &project_id, &episode_id, "", 0, title)
        .map_err(|_| Error::config("这一集已经在合成了 —— 进度在右下角"))?;
    let started = crate::jobs::get(&config_dir, &job_id).ok_or_else(|| Error::runtime("任务登记后读不回来"))?;
    crate::jobs::emit(app.as_ref(), Some(&started));
    tauri::async_runtime::spawn(async move {
        let out = stitch_episode(&config_dir, project_id, episode_id, upscale, cover, |done, total, label| {
            let job = crate::jobs::set_progress(&config_dir, &job_id, done, total, label);
            crate::jobs::emit(app.as_ref(), job.as_ref());
        })
        .await;
        let job = match out {
            // **备注要跟着落到任务上** —— 缺镜的「部分预览」只有在这里说出来，
            // 否则它和完整成片在界面上长得一模一样
            Ok((url, notes)) => crate::jobs::finish_with(&config_dir, &job_id, Some(&url), None, &notes, None),
            Err(e) => crate::jobs::finish(&config_dir, &job_id, None, Some(&e.to_string())),
        };
        crate::jobs::emit(app.as_ref(), job.as_ref());
    });
    Ok(started)
}

/// 这一镜拼哪个文件。
///
/// **用分镜上记着的那一版（`video_url`）**。出片每次落一个新文件
/// `{镜头 id}-b-{时间戳}.mp4`，以前这里只认 `{镜头 id}.mp4` —— 新出的片一条都找不到，
/// 合成出来的是旧片或者干脆报「没有已生成的镜头」。老数据的 `video_url` 可能不带扩展名。
pub(crate) fn stitch_source(shot: &StoryboardShot, shots_dir: &Path) -> Option<PathBuf> {
    if let Some(url) = shot.video_url.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
        let p = PathBuf::from(url);
        if p.is_file() {
            return Some(p);
        }
        if p.extension().is_none() {
            let with_ext = p.with_extension("mp4");
            if with_ext.is_file() {
                return Some(with_ext);
            }
        }
    }
    let legacy = shots_dir.join(format!("{}.mp4", shot.id));
    legacy.is_file().then_some(legacy)
}

/// 整集画幅。超分后是 1080P，否则 720P。
pub(crate) fn episode_frame(aspect_ratio: &str, upscale: bool) -> (u32, u32) {
    // 偶数对齐：yuv420p 要求宽高都是偶数
    frame_for(aspect_ratio, if upscale { 1920 } else { 1280 }, 2, (9.0, 16.0))
}

/// 拼整集。回 `(成片地址, 备注)`。
///
/// **备注不是可选的装饰。** 缺镜时拼出来的是「部分预览」，
/// 和完整交付在界面上长得一模一样 —— 不说清楚的话用户会当成成片发出去
/// （2026-09-19 Codex Astra 指出）。
async fn stitch_episode(
    config_dir: &Path,
    project_id: String,
    episode_id: String,
    upscale: bool,
    cover: bool,
    progress: impl Fn(usize, usize, &str),
) -> Result<(String, Vec<String>)> {
    let root = cinema_projects_dir(config_dir);
    let ep_dir = root.join(&project_id).join("episodes").join(&episode_id);
    let ep_file = ep_dir.join("episode.json");
    if !ep_file.is_file() {
        return Err(Error::config("分集不存在"));
    }

    let ep: EpisodeData =
        serde_json::from_str(&std::fs::read_to_string(&ep_file).unwrap_or_default())
            .map_err(|e| Error::config(format!("读取分集失败：{e}")))?;

    let ffmpeg_bin = ffmpeg_binary();

    // 整集画幅：所有片段都归一到这个尺寸，否则 concat 会被尺寸不一致卡住。
    // 取项目的画幅设置，读不到就按竖屏（短剧默认）。
    let (ep_w, ep_h) = {
        let proj_file = root.join(&project_id).join("project.json");
        let ar = std::fs::read_to_string(&proj_file)
            .ok()
            .and_then(|c| serde_json::from_str::<CinemaProject>(&c).ok())
            .map(|p| p.aspect_ratio)
            .unwrap_or_else(|| "9:16".to_string());
        episode_frame(&ar, upscale)
    };

    // 寻找已经生成好视频的 shots，并标准化为带音轨的片段。
    let shots_dir = ep_dir.join("shots");
    let audio_dir = ep_dir.join("audio");
    let muxed_dir = ep_dir.join("muxed");
    let _ = std::fs::create_dir_all(&muxed_dir);
    let upscaled_dir = ep_dir.join("upscaled");
    let mut valid_vids = Vec::new();
    // 台词 + 这一段片子，收着给整集字幕用（时间轴按真实时长算，不是分镜上写的秒数）
    let mut cue_src: Vec<(String, PathBuf)> = Vec::new();
    let total = ep.storyboard.len();
    let route = project_route(config_dir, &project_id);
    // 片段实际盖住了几镜：一整条长镜头算它那一场的全部镜头
    let mut filled = 0usize;
    let segments = episode_segments(&ep, route);
    let seg_total = segments.len();

    for (i, seg) in segments.iter().enumerate() {
        // 这一段：源视频、输出片段、配音文件、台词（给字幕）、报错用的名字、静音补多长。
        // **长镜头不接逐镜配音**：那些 wav 是按单镜时长配的，铺不到一整条上 —— 用 H3 原声
        let (source_vid, muxed, tts, dialogue, label, secs): (PathBuf, PathBuf, Option<PathBuf>, String, String, f32) = match seg {
            EpisodeSegment::LongTake { scene_no, render, shots } => {
                let (first, last) = (shots[0].shot_num, shots[shots.len() - 1].shot_num);
                let label = format!("第 {scene_no} 场长镜头（第 {first}–{last} 镜）");
                progress(i, seg_total, &label);
                let src = PathBuf::from(&render.path);
                let take = if upscale {
                    upscaled_take(&src, &upscaled_dir).await.map_err(|e| Error::runtime(format!("{label}超分失败：{e}")))?
                } else {
                    src
                };
                filled += shots.len();
                let dialogue = shots.iter().map(|s| s.dialogue.trim()).filter(|d| !d.is_empty()).collect::<Vec<_>>().join("\n");
                let secs: f32 = shots.iter().map(|s| s.duration).sum();
                (take, muxed_dir.join(format!("longtake_s{scene_no}.mp4")), None, dialogue, label, secs)
            }
            EpisodeSegment::Shot(shot) => {
                progress(i, seg_total, &format!("第 {} 镜", shot.shot_num));
                let source_vid = if let Some(take) = stitch_source(shot, &shots_dir) {
                    if upscale {
                        Some(upscaled_take(&take, &upscaled_dir).await.map_err(|e| {
                            Error::runtime(format!("第 {} 镜超分失败：{e}", shot.shot_num))
                        })?)
                    } else {
                        Some(take)
                    }
                } else {
                    // 如果分镜没有独立视频，尝试回退使用该角色的 idle / speaking 视频作为合成底料
                    let neural_dir = crate::avatar::model::models_dir(config_dir).join("neural");
                    let fallback = neural_dir.join(format!("{}_speaking.mp4", shot.character_id));
                    if fallback.is_file() {
                        Some(fallback)
                    } else {
                        None
                    }
                };

                let Some(source_vid) = source_vid else {
                    continue;
                };
                filled += 1;
                (
                    source_vid,
                    muxed_dir.join(format!("{}.mp4", shot.id)),
                    Some(audio_dir.join(format!("{}.wav", shot.id))),
                    shot.dialogue.clone(),
                    format!("Shot #{}", shot.shot_num),
                    shot.duration,
                )
            }
        };

        let src_has_audio = crate::agent_runtime::video_tool::has_audio_stream(&source_vid);
        let pick = pick_shot_audio(tts.as_ref().is_some_and(|p| p.is_file()), src_has_audio);

        let mut cmd = std::process::Command::new(ffmpeg_bin);
        cmd.arg("-y")
            .arg("-i")
            .arg(source_vid.to_str().unwrap_or_default());
        match pick {
            ShotAudio::Tts => {
                cmd.arg("-i").arg(tts.as_ref().and_then(|p| p.to_str()).unwrap_or_default());
            }
            ShotAudio::Source => {}
            ShotAudio::Silence => {
                cmd.arg("-f")
                    .arg("lavfi")
                    .arg("-t")
                    .arg(format!("{}", secs.max(1.0)))
                    .arg("-i")
                    .arg("anullsrc=channel_layout=mono:sample_rate=24000");
            }
        }
        let audio_map = match pick {
            ShotAudio::Source => "0:a:0",
            _ => "1:a:0",
        };
        let output = cmd
            .args([
                "-map",
                "0:v:0",
                "-map",
                audio_map,
                // 归一到整集画幅，否则 concat 会被尺寸不一致卡住
                "-vf",
                &frame_filter(crate::agent_runtime::video_tool::video_dimensions(&source_vid).ok(), ep_w, ep_h),
                "-c:v",
                "libx264",
                "-crf",
                "18",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-shortest",
                "-movflags",
                "+faststart",
                muxed.to_str().unwrap_or_default(),
            ])
            .output();

        match output {
            Ok(o) if o.status.success() && muxed.is_file() => {
                cue_src.push((dialogue.clone(), muxed.clone()));
                valid_vids.push(muxed);
            }
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr)
                    .chars()
                    .take(1600)
                    .collect::<String>();
                return Err(Error::runtime(format!(
                    "标准化镜头音视频片段失败（{label}）：{stderr}"
                )));
            }
            Err(e) => {
                return Err(Error::runtime(format!(
                    "执行 FFmpeg 标准化片段失败（{label}）：{e}"
                )))
            }
        }
    }

    progress(seg_total, seg_total, "拼接");
    if valid_vids.is_empty() {
        return Err(Error::config(
            "当前分集尚无已生成的镜头视频，请先点击分镜渲染！",
        ));
    }
    // **缺镜的成片是「部分预览」，不是交付。**
    //
    // 原来只判「一镜都没有」，所以少拍了三镜照样能拼出一份「合成成功」的片子 ——
    // 而它和完整交付在界面上长得一模一样（2026-09-19 Codex Astra 指出）。
    //
    // 不拦（预览有用），但要在**备注里说清楚缺了哪几镜**。
    let mut notes: Vec<String> = Vec::new();
    if filled < total {
        let missing: Vec<String> = ep
            .storyboard
            .iter()
            .filter(|s| long_take_clip(&ep, s.scene_no, route).is_none() && stitch_source(s, &shots_dir).is_none())
            .map(|s| format!("第 {} 镜", s.shot_num))
            .collect();
        notes.push(format!(
            "**这是部分预览，不是完整成片**：{} 个镜头里有 {} 个还没出片（{}）—— 拼出来的片子少了这几段",
            total,
            total - filled,
            if missing.len() > 6 {
                format!("{}… 等 {} 镜", missing[..6].join("、"), missing.len())
            } else {
                missing.join("、")
            }
        ));
    }

    // 生成 concat.txt
    let concat_file = ep_dir.join("concat_list.txt");
    let mut concat_content = String::new();
    for v in &valid_vids {
        concat_content.push_str(&format!("file '{}'\n", v.display()));
    }
    let _ = std::fs::write(&concat_file, &concat_content);

    let out_final = ep_dir.join("final.mp4");
    let ep_bgm = ep_dir.join("bgm.wav");
    let ep_bgm_flac = ep_dir.join("bgm.flac");
    let ep_bgm_mp3 = ep_dir.join("bgm.mp3");
    let candidate_bgm = if ep_bgm_flac.is_file() {
        Some(ep_bgm_flac)
    } else if ep_bgm.is_file() {
        Some(ep_bgm)
    } else if ep_bgm_mp3.is_file() {
        Some(ep_bgm_mp3)
    } else {
        None
    };

    let temp_concat_video = if candidate_bgm.is_some() {
        ep_dir.join("concat_temp.mp4")
    } else {
        out_final.clone()
    };

    let status = std::process::Command::new(ffmpeg_bin)
        .args([
            "-y",
            "-f",
            "concat",
            "-safe",
            "0",
            "-i",
            concat_file.to_str().unwrap_or_default(),
            "-c:v",
            "libx264",
            "-crf",
            "18",
            "-c:a",
            "aac",
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            "+faststart",
            temp_concat_video.to_str().unwrap_or_default(),
        ])
        .output();

    let _ = std::fs::remove_file(&concat_file);

    match status {
        Ok(o) if o.status.success() && temp_concat_video.is_file() => {
            if let Some(bgm_path) = candidate_bgm {
                let mix_status = std::process::Command::new(ffmpeg_bin)
                    .args([
                        "-y",
                        "-i",
                        temp_concat_video.to_str().unwrap_or_default(),
                        "-stream_loop",
                        "-1",
                        "-i",
                        bgm_path.to_str().unwrap_or_default(),
                        "-filter_complex",
                        "[1:a]volume=0.25[bgm];[0:a][bgm]amix=inputs=2:duration=first:dropout_transition=2[aout]",
                        "-map",
                        "0:v:0",
                        "-map",
                        "[aout]",
                        "-c:v",
                        "copy",
                        "-c:a",
                        "aac",
                        "-shortest",
                        out_final.to_str().unwrap_or_default(),
                    ])
                    .output();
                let _ = std::fs::remove_file(&temp_concat_video);
                match mix_status {
                    Ok(mo) if mo.status.success() && out_final.is_file() => {},
                    Ok(mo) => {
                        let err = String::from_utf8_lossy(&mo.stderr);
                        return Err(Error::runtime(format!("BGM 背景音乐混音失败：{err}")));
                    }
                    Err(e) => return Err(Error::runtime(format!("执行 BGM 混音失败：{e}"))),
                }
            }

            if !out_final.is_file() {
                return Err(Error::runtime("成片拼接完成但未找到生成的 final.mp4"));
            }

            // 片头封面。**刚拼好的这份就是新母版**，所以先把旧母版清掉 ——
            // 留着的话 `has_cover_intro` 会说谎，字幕平白多偏移 2.5 秒。
            crate::episode_cover::clear_cover_intro(&ep_dir);
            if cover {
                crate::episode_cover::set_cover_intro(config_dir, &project_id, &episode_id, true).await?;
                // 字幕也要让开片头这几秒。空台词不产出字幕、只推进偏移
                if let Some(intro) = crate::episode_cover::intro_clip(&ep_dir) {
                    cue_src.insert(0, (String::new(), intro));
                }
            }

            // 软字幕：和成片同名的 final.srt。硬字幕烧不烧在出片设置里，这份**总是写**
            let srt = write_episode_srt(&ep_dir, &cue_src);
            if let Err(e) = &srt {
                tracing::warn!(%e, "整集字幕没写成");
            }

            let out_url = format!("/projects/{}/episodes/{}/final.mp4", project_id, episode_id);
            // 更新 episode.json。**只改成片地址** —— 开头读的那份是拼接之前的，
            // 整份写回去会把拼接这几分钟里别处的改动（出片回写、导演世界）全冲掉
            let _ = crate::episode_lock::update_file(&ep_file, |latest| {
                latest.final_video_url = Some(out_url.clone());
                Ok(())
            });
            Ok((out_url, notes))
        }
        Ok(o) => Err(Error::config(format!(
            "FFmpeg 拼接视频失败：{}",
            String::from_utf8_lossy(&o.stderr)
        ))),
        Err(e) => Err(Error::config(format!("执行 FFmpeg 失败：{e}"))),
    }
}

/// 片子的真实秒数（ffprobe）。读不出来回 `None` —— 调用方退回分镜上写的秒数
pub(crate) fn clip_seconds(path: &Path) -> Option<f64> {
    let out = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"])
        .arg(path)
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse::<f64>().ok().filter(|d| *d > 0.0)
}

/// 整集字幕写成 `final.srt`（和成片同名，剪映 / 达芬奇 / 播放器都按这个找）
fn write_episode_srt(ep_dir: &Path, clips: &[(String, PathBuf)]) -> Result<PathBuf> {
    let items: Vec<(String, f64)> = clips
        .iter()
        .map(|(d, p)| (d.clone(), clip_seconds(p).unwrap_or(0.0)))
        .filter(|(_, secs)| *secs > 0.0)
        .collect();
    let srt = crate::subtitle::to_srt(&crate::subtitle::episode_cues(&items));
    let path = ep_dir.join("final.srt");
    if srt.trim().is_empty() {
        // 整集一句台词都没有：别留一个空文件让人以为字幕丢了
        let _ = std::fs::remove_file(&path);
        return Ok(path);
    }
    std::fs::write(&path, srt).map_err(|e| Error::runtime(format!("写字幕失败：{e}")))?;
    Ok(path)
}

/// 重新生成整集字幕（不用重新合成）。回字幕文件路径和正文
#[tauri::command]
pub fn cinema_export_srt(state: State<'_, AppState>, project_id: String, episode_id: String) -> Result<SrtExport> {
    export_srt(&state.config_dir, &project_id, &episode_id)
}

/// [`cinema_export_srt`] 的本体，不依赖界面状态
pub fn export_srt(config_dir: &Path, project_id: &str, episode_id: &str) -> Result<SrtExport> {
    let ep_dir = cinema_projects_dir(config_dir).join(project_id).join("episodes").join(episode_id);
    let ep: EpisodeData = serde_json::from_str(&std::fs::read_to_string(ep_dir.join("episode.json")).map_err(|e| Error::config(format!("读不到分集：{e}")))?)
        .map_err(|e| Error::config(format!("分集文件坏了：{e}")))?;
    let muxed_dir = ep_dir.join("muxed");
    let shots_dir = ep_dir.join("shots");
    // 合成时用的是 muxed 里那一段；还没合成过就退回出片文件，秒数再退回分镜上写的
    let clips: Vec<(String, f64)> = ep
        .storyboard
        .iter()
        .filter_map(|shot| {
            let muxed = muxed_dir.join(format!("{}.mp4", shot.id));
            let secs = clip_seconds(&muxed)
                .or_else(|| stitch_source(shot, &shots_dir).and_then(|p| clip_seconds(&p)))
                .or_else(|| (shot.video_url.is_some()).then_some(shot.duration as f64))?;
            (secs > 0.0).then(|| (shot.dialogue.clone(), secs))
        })
        .collect();
    if clips.is_empty() {
        return Err(Error::config("这一集还没有出好片的镜头，先出片"));
    }
    // **片头封面要占掉开头几秒。** 字幕是按镜头时长重算的，看不见片头 ——
    // 不让开的话整集字幕早 2.5 秒，而且哪里都不报错。
    // 空台词那一段不产出字幕，但照样把偏移往后推（`episode_cues` 的语义）。
    let mut clips = clips;
    let intro = crate::episode_cover::cover_intro_offset(&ep_dir);
    if intro > 0.0 {
        clips.insert(0, (String::new(), intro));
    }
    let content = crate::subtitle::to_srt(&crate::subtitle::episode_cues(&clips));
    if content.trim().is_empty() {
        return Err(Error::config("这一集的镜头里没有台词，没有字幕可导"));
    }
    let path = ep_dir.join("final.srt");
    std::fs::write(&path, &content).map_err(|e| Error::runtime(format!("写字幕失败：{e}")))?;
    Ok(SrtExport { path: path.to_string_lossy().to_string(), content })
}

/// 导出的字幕：路径给「在访达中显示」，正文给播放器直接挂
#[derive(Debug, Clone, Serialize)]
pub struct SrtExport {
    pub path: String,
    pub content: String,
}

/// 一镜超分后的文件。**按出片文件名缓存**：重拼一次不用重跑；重出了这一镜，文件名换了，自然重做。
/// 已经够大的（长边 ≥ 1800）直接用原片。
///
/// 文件名相同但出片文件**比缓存新**（比如重烧过字幕）也要重做 —— 只认文件名的话会拼进旧画面。
pub(crate) async fn upscaled_take(take: &Path, dir: &Path) -> std::result::Result<PathBuf, String> {
    let (w, h) = crate::agent_runtime::video_tool::video_dimensions(take).map_err(|e| e.to_string())?;
    if w.max(h) >= 1800 {
        return Ok(take.to_path_buf());
    }
    let stem = take.file_stem().and_then(|s| s.to_str()).unwrap_or("shot");
    let out = dir.join(format!("{stem}_x2.mp4"));
    if cache_fresh(take, &out) {
        return Ok(out);
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("建目录失败：{e}"))?;
    crate::canvas_ops::esrgan_upscale_file(take, &out, "Anima/episode_upscale", "合成 · 超分").await?;
    Ok(out)
}

/// 超分缓存还能不能用：存在，且不比出片文件旧。
pub(crate) fn cache_fresh(source: &Path, cached: &Path) -> bool {
    let mtime = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    match (mtime(source), mtime(cached)) {
        (Some(src), Some(c)) => c >= src,
        _ => false,
    }
}

fn create_sample_project(root: &Path) -> Result<CinemaProject> {
    let id = "drama_sample_sweetheart";
    let p_dir = root.join(id);
    let _ = std::fs::create_dir_all(p_dir.join("episodes/ep_01/shots"));

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let proj = CinemaProject {
        default_looks: Default::default(),
        script_project_id: None,
        id: id.to_string(),
        title: "《千金归来：她带着千亿帝国杀疯了》".to_string(),
        genre: "都市逆袭 / 爆款反转".to_string(),
        aspect_ratio: "9:16".to_string(),
        description: "豪门真千金重返家族，面对虚伪养女与偏心养父母，以绝对实力逆风翻盘的爆款短剧！"
            .to_string(),
        created_at: now,
        updated_at: now,
        episode_count: 1,
        cover_image: None,
        kind: "series".into(),
        content_mode: default_content_mode(),
        default_shot_mode: ShotMode::default(),
        model_settings: ModelSettings::default(),
    };

    let _ = std::fs::write(
        p_dir.join("project.json"),
        serde_json::to_string_pretty(&proj).unwrap_or_default(),
    );

    let ep1 = EpisodeData {
        long_take_scenes: Vec::new(),
        one_take: false,
        scene_takes: Vec::new(),
            long_take_context_frames: None,
            scene_renders: Vec::new(),
        style_lock: String::new(),
        id: "ep_01".to_string(),
        episode_num: 1,
        title: "第 1 集：真假千金的鸿门宴".to_string(),
        synopsis: "由美子受邀参加顶奢晚宴，身份遭人轻视，全场却不知她是幕后真正的大股东。".to_string(),
        script_text: "【场景：顶级私人宴会厅】\n【人物：由美子（真千金）、顾辰（商界巨擘）】\n\n[镜头 1: 华丽大厅全景] 水晶吊灯倾泻而下，名流云集。\n\n[镜头 2: 主角眼神特写] 由美子手持香槟，神色从容优雅。\n由美子：“听说有人想用三百万打发我离开？”\n\n[镜头 3: 男主侧脸推镜头] 顾辰缓步上前，眼神灼热。\n顾辰：“整个顾氏集团都是你的，谁敢让你走。”\n".to_string(),
        script_doc: None,
        target_secs: 0,
        storyboard_version: STORYBOARD_VERSION, actor_colors: Vec::new(),
        storyboard: vec![
            StoryboardShot { combat_style: String::new(),
                look_overrides: Default::default(),
                one_take: false,
            end_frame: String::new(),
            scene_no: 0, scene_name: String::new(), duty: String::new(), source: String::new(), lighting: String::new(), scene_items: Vec::new(), stage: Default::default(), segments: Vec::new(), ref_slots: Vec::new(), prompts: Default::default(), unlinked_assets: Vec::new(), takes: Vec::new(),
            action: String::new(), dialogue_lines: Vec::new(), emotion_tags: Vec::new(), blocking: Vec::new(),
            ambient_sound: String::new(), sound_effect: String::new(), score: String::new(),
                id: "shot_1".to_string(),
                shot_num: 1,
                image_rev: 0,
                video_rev: 0,
                scale: "全景 (Wide)".to_string(),
                camera_movement: "俯视 (Tilt Down)".to_string(),
                camera_angle: String::new(),
                camera_note: String::new(),
                director_camera: None, director_scene: None,
                dialogue: "（环境氛围）水晶吊灯倾泻而下，名流云集。".to_string(),
                character_id: "environment".to_string(),
                character_ids: Vec::new(),
                scene_id: None,
                prop_ids: Vec::new(),
                mode: ShotMode::FirstFrame,
                continuity: Continuity::None,
                continuity_manual: false, route_choice: None,
                prompt: "masterpiece, grand luxury ballroom with crystal chandeliers, golden atmospheric lighting, cinematic film still, 8k".to_string(),
                negative_prompt: Some("low quality, blurry".to_string()),
                duration: 3.5,
                image_url: None,
                video_url: None,
                audio_url: None, sketch_url: None, keyframes: Keyframes::NONE, end_image_url: None,
                continues_previous: false,
                status: "ready".to_string(),
            },
            StoryboardShot { combat_style: String::new(),
                look_overrides: Default::default(),
                one_take: false,
            end_frame: String::new(),
            scene_no: 0, scene_name: String::new(), duty: String::new(), source: String::new(), lighting: String::new(), scene_items: Vec::new(), stage: Default::default(), segments: Vec::new(), ref_slots: Vec::new(), prompts: Default::default(), unlinked_assets: Vec::new(), takes: Vec::new(),
            action: String::new(), dialogue_lines: Vec::new(), emotion_tags: Vec::new(), blocking: Vec::new(),
            ambient_sound: String::new(), sound_effect: String::new(), score: String::new(),
                id: "shot_2".to_string(),
                shot_num: 2,
                image_rev: 0,
                video_rev: 0,
                scale: "特写 (Close-Up)".to_string(),
                camera_movement: "推镜头 (Zoom In)".to_string(),
                camera_angle: String::new(),
                camera_note: String::new(),
                director_camera: None, director_scene: None,
                dialogue: "由美子：“听说有人想用三百万打发我离开？”".to_string(),
                character_id: "yumiko".to_string(),
                character_ids: Vec::new(),
                scene_id: None,
                prop_ids: Vec::new(),
                mode: ShotMode::FirstFrame,
                continuity: Continuity::None,
                continuity_manual: false, route_choice: None,
                prompt: "masterpiece, 1girl, beautiful East Asian heiress holding champagne, elegant dress, confident smile, sparkling jewelry, 8k".to_string(),
                negative_prompt: Some("low quality, deformed".to_string()),
                duration: 4.0,
                image_url: None,
                video_url: Some("/avatars/neural/yumiko_speaking.mp4".to_string()),
                audio_url: None, sketch_url: None, keyframes: Keyframes::NONE, end_image_url: None,
                continues_previous: false,
                status: "ready".to_string(),
            },
            StoryboardShot { combat_style: String::new(),
                look_overrides: Default::default(),
                one_take: false,
            end_frame: String::new(),
            scene_no: 0, scene_name: String::new(), duty: String::new(), source: String::new(), lighting: String::new(), scene_items: Vec::new(), stage: Default::default(), segments: Vec::new(), ref_slots: Vec::new(), prompts: Default::default(), unlinked_assets: Vec::new(), takes: Vec::new(),
            action: String::new(), dialogue_lines: Vec::new(), emotion_tags: Vec::new(), blocking: Vec::new(),
            ambient_sound: String::new(), sound_effect: String::new(), score: String::new(),
                id: "shot_3".to_string(),
                shot_num: 3,
                image_rev: 0,
                video_rev: 0,
                scale: "中景 (Medium)".to_string(),
                camera_movement: "固定 (Static)".to_string(),
                camera_angle: String::new(),
                camera_note: String::new(),
                director_camera: None, director_scene: None,
                dialogue: "顾辰：“整个顾氏集团都是你的，谁敢让你走。”".to_string(),
                character_id: "xiaoya".to_string(),
                character_ids: Vec::new(),
                scene_id: None,
                prop_ids: Vec::new(),
                mode: ShotMode::FirstFrame,
                continuity: Continuity::None,
                continuity_manual: false, route_choice: None,
                prompt: "masterpiece, 1girl, elegant gentle expression, luxurious ballroom background, cinematic lighting, 8k".to_string(),
                negative_prompt: Some("low quality, deformed".to_string()),
                duration: 3.5,
                image_url: None,
                video_url: Some("/avatars/neural/xiaoya_speaking.mp4".to_string()),
                audio_url: None, sketch_url: None, keyframes: Keyframes::NONE, end_image_url: None,
                continues_previous: false,
                status: "ready".to_string(),
            },
        ],
        final_video_url: None,
    };

    let _ = crate::episode_lock::put(&p_dir.join("episodes/ep_01/episode.json"), &ep1);
    Ok(proj)
}

/// 导出当前短剧分集为剪映草稿工程 (CapCut / JianYing draft)
#[tauri::command]
pub async fn cinema_export_jianying_project(
    state: State<'_, AppState>,
    project_id: String,
    episode_id: String,
) -> Result<String> {
    cinema_export_jianying_project_with_config_dir(&state.config_dir, project_id, episode_id).await
}

pub async fn cinema_export_jianying_project_with_config_dir(
    config_dir: &Path,
    project_id: String,
    episode_id: String,
) -> Result<String> {
    export_jianying(config_dir, &project_id, &episode_id, None).await
}

/// 本体。`export_dir` 不给就写进剪映的草稿目录（`~/Movies/JianyingPro/…`）
pub async fn export_jianying(
    config_dir: &Path,
    project_id: &str,
    episode_id: &str,
    export_dir: Option<PathBuf>,
) -> Result<String> {
    let root = cinema_projects_dir(config_dir);
    let proj_dir = root.join(project_id);
    let ep_dir = proj_dir.join("episodes").join(episode_id);
    let ep_file = ep_dir.join("episode.json");
    if !ep_file.is_file() {
        return Err(Error::runtime(format!("分集档案不存在: {episode_id}")));
    }
    let ep_json = std::fs::read_to_string(&ep_file).map_err(|e| Error::runtime(e.to_string()))?;
    let ep_data: EpisodeData =
        serde_json::from_str(&ep_json).map_err(|e| Error::runtime(e.to_string()))?;

    // 画幅跟项目走（竖屏短剧 1080×1920，横屏 1920×1080），别写死
    let aspect = std::fs::read_to_string(proj_dir.join("project.json"))
        .ok()
        .and_then(|c| serde_json::from_str::<CinemaProject>(&c).ok())
        .map(|p| p.aspect_ratio)
        .unwrap_or_else(|| "9:16".to_string());
    let (ep_w, ep_h) = episode_frame(&aspect, true);

    let export_dir = export_dir.unwrap_or_else(|| {
        dirs::home_dir()
            .map(|h| h.join("Movies/JianyingPro/User Data/Projects/com.lveditor.draft"))
            .unwrap_or_else(|| proj_dir.join("jianying_drafts"))
    });
    let _ = std::fs::create_dir_all(&export_dir);

    // **用合成时那一套片段**（`muxed/` 里已经归一到整集画幅、带音轨），没有就退回出片文件。
    // 音轨在片段里，所以不另外排配音轨 —— 排了会和片段里的声音重叠成双声。
    let shots_dir = ep_dir.join("shots");
    let muxed_dir = ep_dir.join("muxed");
    let mut clips = Vec::new();
    let mut cue_src: Vec<(String, f64)> = Vec::new();
    // 成片上加过片头封面的话，**剪映时间轴也要有它** ——
    // 少了它导出的草稿比成片短一截、每句字幕还都对不上。
    let intro_clip = ep_dir.join("cover_intro.mp4");
    if crate::episode_cover::has_cover_intro(&ep_dir) && intro_clip.is_file() {
        let secs = clip_seconds(&intro_clip).unwrap_or(crate::episode_cover::COVER_INTRO_SECS);
        cue_src.push((String::new(), secs));
        clips.push(crate::cinema_export_jianying::JyClipInput {
            name: "片头封面".into(),
            path: intro_clip.to_string_lossy().to_string(),
            duration_sec: secs,
            width: Some(ep_w),
            height: Some(ep_h),
        });
    }
    // 整段出的场放一整条（和合成一集、FCP 同一个规矩，见 `episode_segments`）
    let route = project_route(config_dir, project_id);
    for seg in episode_segments(&ep_data, route) {
        let (path, name, dialogue, fallback_secs) = match &seg {
            EpisodeSegment::LongTake { scene_no, render, shots } => {
                let muxed = muxed_dir.join(format!("longtake_s{scene_no}.mp4"));
                let path = if muxed.is_file() { muxed } else { PathBuf::from(&render.path) };
                let (first, last) = (shots[0].shot_num, shots[shots.len() - 1].shot_num);
                let dialogue = shots.iter().map(|s| s.dialogue.trim()).filter(|d| !d.is_empty()).collect::<Vec<_>>().join("\n");
                (Some(path), format!("第 {scene_no} 场 · 长镜头（第 {first}–{last} 镜）"), dialogue, shots.iter().map(|s| s.duration as f64).sum())
            }
            EpisodeSegment::Shot(shot) => {
                let muxed = muxed_dir.join(format!("{}.mp4", shot.id));
                let path = if muxed.is_file() { Some(muxed) } else { stitch_source(shot, &shots_dir) };
                (path, format!("第 {} 镜", shot.shot_num), shot.dialogue.clone(), shot.duration as f64)
            }
        };
        let Some(path) = path else { continue };
        let secs = clip_seconds(&path).unwrap_or(fallback_secs);
        cue_src.push((dialogue, secs));
        clips.push(crate::cinema_export_jianying::JyClipInput {
            name,
            path: path.to_string_lossy().to_string(),
            duration_sec: secs,
            width: Some(ep_w),
            height: Some(ep_h),
        });
    }
    if clips.is_empty() {
        return Err(Error::config("这一集还没有出好片的镜头，先出片"));
    }
    let voiceovers: Vec<crate::cinema_export_jianying::JyAudioInput> = Vec::new();

    // 字幕和成片那份 SRT 同一个来源：真实时长接出来的时间轴，一行一条、不带角色名
    let subtitles: Vec<crate::cinema_export_jianying::JySubtitleInput> = crate::subtitle::episode_cues(&cue_src)
        .into_iter()
        .map(|c| crate::cinema_export_jianying::JySubtitleInput {
            text: c.text,
            start_sec: c.start,
            duration_sec: (c.end - c.start).max(0.3),
        })
        .collect();

    let ep_bgm = ep_dir.join("bgm.wav");
    let ep_bgm_flac = ep_dir.join("bgm.flac");
    let ep_bgm_mp3 = ep_dir.join("bgm.mp3");
    let bgm_path = if ep_bgm_flac.is_file() {
        Some(ep_bgm_flac.to_string_lossy().to_string())
    } else if ep_bgm.is_file() {
        Some(ep_bgm.to_string_lossy().to_string())
    } else if ep_bgm_mp3.is_file() {
        Some(ep_bgm_mp3.to_string_lossy().to_string())
    } else {
        None
    };

    let draft_input = crate::cinema_export_jianying::JianYingDraftInput {
        name: format!("Anima_{}_{}", ep_data.episode_num, ep_data.title),
        width: ep_w,
        height: ep_h,
        // 出片就是 24 帧，草稿写别的会让剪映按别的帧率解释时间轴
        fps: 24,
        clips,
        voiceovers,
        bgm_path,
        subtitles,
    };

    let exported_path =
        crate::cinema_export_jianying::export_episode_to_jianying(&export_dir, &draft_input)
            .map_err(|e| Error::runtime(format!("导出剪映草稿失败: {e}")))?;

    Ok(exported_path.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {

    use super::cinema_spoken_dialogue;

    #[test]
    fn extracts_chinese_quoted_dialogue() {
        assert_eq!(
            cinema_spoken_dialogue("由美子：“今天开始，所有规则由我来定。”"),
            "今天开始，所有规则由我来定。"
        );
    }

    #[test]
    fn strips_stage_direction_and_speaker_prefix() {
        assert_eq!(
            cinema_spoken_dialogue("[镜头 2: 特写] 顾辰：随时可以入场。"),
            "随时可以入场。"
        );
    }
}

// 漏了这一行的话，下面的测试辅助函数会被编进正式程序（2026-09-16 清理时发现，报 20 多条「从未使用」）
#[cfg(test)]
mod continuity_tests {
    use super::*;

    fn 镜(prompt: &str, dialogue: &str) -> StoryboardShot {
        StoryboardShot { combat_style: String::new(),
            one_take: false,
            continuity: Default::default(),
            continuity_manual: false, route_choice: None,
            look_overrides: Default::default(),
            end_frame: String::new(),
            scene_no: 0, scene_name: String::new(), duty: String::new(), source: String::new(), lighting: String::new(), scene_items: Vec::new(), stage: Default::default(), segments: Vec::new(), ref_slots: Vec::new(), prompts: Default::default(), unlinked_assets: Vec::new(), takes: Vec::new(),
            action: String::new(), dialogue_lines: Vec::new(), emotion_tags: Vec::new(), blocking: Vec::new(),
            ambient_sound: String::new(), sound_effect: String::new(), score: String::new(),
            id: "s1".into(), shot_num: 1, image_rev: 0, video_rev: 0,
            scale: "close_up".into(),
            camera_movement: "push_in".into(),
            camera_angle: String::new(),
            camera_note: String::new(),
            director_camera: None, director_scene: None,
            dialogue: dialogue.into(), character_id: "c1".into(), character_ids: Vec::new(), scene_id: None, prop_ids: Vec::new(), mode: ShotMode::FirstFrame,
            prompt: prompt.into(), negative_prompt: None, duration: 5.0,
            image_url: None, video_url: None, audio_url: None, sketch_url: None, keyframes: Keyframes::NONE, end_image_url: None,
            continues_previous: false, status: "pending".into(),
        }
    }





    /// **词表里每一条运镜，静帧预览都要有个说法。**
    ///
    /// 不是说每条都得有专属滤镜 —— 环绕、手持在一张静帧上本来就没法表现，
    /// 走「呼吸感」是对的。要钉住的是**别在加新词时忘了这里**：
    /// 漏了的话用户选了「变焦推近」而预览一动不动，看不出是哪儿错了。
    ///
    /// 判据是「能不能跑出一条非空滤镜」，外加**方向相反的两条必须不同** ——
    /// 上摇和下摇曾经共用一条滤镜，选哪个出来都一样。
    #[test]
    fn 每条运镜在静帧预览里都有着落() {
        let moves = crate::shot_language::group("move").unwrap();
        for t in &moves.items {
            let f = ken_burns_filter(&t.id, 100, 640, 360, 25);
            assert!(!f.trim().is_empty(), "{} 跑不出滤镜", t.id);
        }
        // **该动的不能退到「呼吸感」。**
        //
        // 第一版只查「跑得出非空滤镜」—— 而兜底的呼吸感也是非空的，
        // 于是漏掉一条映射照样绿（变异测试当场抓到）。
        // 环绕、手持、变焦点这些在一张静帧上本来就没法表现，走兜底是对的；
        // 下面这些不是。
        let 兜底 = ken_burns_filter("static", 100, 640, 360, 25);
        for id in [
            "push_in", "pull_out", "zoom_in", "zoom_out",
            "pan_left", "pan_right", "tilt_up", "tilt_down",
            "truck_left", "truck_right", "pedestal_up", "pedestal_down", "whip_pan",
        ] {
            assert_ne!(
                ken_burns_filter(id, 100, 640, 360, 25),
                兜底,
                "{id} 退到了兜底 —— 用户选了它而预览一动不动，看不出哪儿错了"
            );
        }
        // 方向相反的必须不同
        for (a, b) in [
            ("push_in", "pull_out"),
            ("zoom_in", "zoom_out"),
            ("pan_left", "pan_right"),
            ("tilt_up", "tilt_down"),
            ("truck_left", "truck_right"),
            ("pedestal_up", "pedestal_down"),
        ] {
            assert_ne!(
                ken_burns_filter(a, 100, 640, 360, 25),
                ken_burns_filter(b, 100, 640, 360, 25),
                "{a} 和 {b} 出来是同一段"
            );
        }
    }








    fn 台词(speaker: &str) -> DialogueLine {
        DialogueLine { speaker: speaker.into(), line: "说了句话".into(), ..Default::default() }
    }






    /// **台词必须进提示词** —— H3 的编码器把它变成同步语音，
    /// 丢了就只剩哑剧，而且不报错。
    /// **参考图上限是 9，不是 4。**
    ///
    /// 这里原来写死 `truncate(4)`，注释还断言「参考图有 4 张上限」——
    /// 而节点声明的是 `max=9`，MiniMax 官方规格也是 9。
    /// 后果：一镜引用 5 个以上资产时，第 5 张往后**静默丢掉**。
    ///
    /// **这条测的是真行为，不是源码文本。** 第一版断言「源码里要出现
    /// `MAX_REFERENCE_IMAGES`」—— 而那个字符串**自己就在断言里**，
    /// 于是把代码改回 `truncate(4)` 测试照样绿。变异测试当场抓出来的假绿。
    #[test]
    fn 一镜最多送九张参考图() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        let sc = crate::assets::Scope::Global;
        let media = sc.media_dir(cfg);
        std::fs::create_dir_all(&media).unwrap();

        // 建 12 个各自有图的角色，全部被这一镜引用
        let mut prompt = String::new();
        let mut store = crate::assets::load_store(cfg, &sc);
        for i in 0..12 {
            let name = format!("角色{i}");
            let mut a = crate::assets::Asset::new(crate::assets::AssetKind::Character, &name);
            let rel = format!("v{i}.png");
            std::fs::write(media.join(&rel), b"x").unwrap();
            a.unit.image_variants.push(crate::assets::MediaVariant {
                id: format!("v{i}"),
                rel_path: rel,
                created_at: 0,
                prompt_used: None,
                favorited: false,
                from_upload: false,
                role: None,
            });
            a.unit.selected_image_id = Some(format!("v{i}"));
            store.assets.push(a);
            prompt.push_str(&format!("[角色:{name}] "));
        }
        crate::assets::save_store(cfg, &sc, &store).unwrap();

        let shot = 镜(&prompt, "");
        let refs = shot_reference_images(cfg, &shot, &crate::assets::RefScope::Anywhere);
        assert_eq!(
            refs.len(),
            crate::agent_runtime::video_tool::MAX_REFERENCE_IMAGES,
            "十二个资产都有图，该送满上限"
        );
        assert_eq!(refs.len(), 9, "上限就是 9 —— 写死成 4 会静默丢掉五张");
    }




    /// 整集拼接的端到端：三镜 → final.mp4。**打真 ffmpeg，验音轨和时长。**
    ///
    /// `ANIMA_STITCH_DIR=<临时目录> ANIMA_STITCH_SHOTS=<a.mp4,b.mp4,c.mp4> \
    ///   cargo test --lib stitch_e2e -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn stitch_e2e() {
        let dir = std::path::PathBuf::from(
            std::env::var("ANIMA_STITCH_DIR").expect("要设 ANIMA_STITCH_DIR"),
        );
        let shots: Vec<String> = std::env::var("ANIMA_STITCH_SHOTS")
            .expect("要设 ANIMA_STITCH_SHOTS")
            .split(',')
            .map(|s| s.trim().to_string())
            .collect();

        let (pid, eid) = ("e2e_proj", "ep1");
        let ep_dir = dir.join("projects").join(pid).join("episodes").join(eid);
        std::fs::create_dir_all(ep_dir.join("shots")).unwrap();
        std::fs::write(
            dir.join("projects").join(pid).join("project.json"),
            serde_json::to_string(&CinemaProject {
                default_looks: Default::default(),
                script_project_id: None,
                model_settings: ModelSettings::default(),
                content_mode: default_content_mode(),
                default_shot_mode: ShotMode::default(),
                id: pid.into(), title: "端到端".into(), genre: "剧情".into(),
                aspect_ratio: "16:9".into(), description: String::new(),
                created_at: 0, updated_at: 0, episode_count: 1, cover_image: None,
                kind: "series".into(),
            })
            .unwrap(),
        )
        .unwrap();

        let mut board = Vec::new();
        for (i, src) in shots.iter().enumerate() {
            let id = format!("s{}", i + 1);
            std::fs::copy(src, ep_dir.join("shots").join(format!("{id}.mp4"))).unwrap();
            board.push(StoryboardShot { combat_style: String::new(),
                one_take: false,
                continuity: Default::default(),
                continuity_manual: false, route_choice: None,
                look_overrides: Default::default(),
            end_frame: String::new(),
            scene_no: 0, scene_name: String::new(), duty: String::new(), source: String::new(), lighting: String::new(), scene_items: Vec::new(), stage: Default::default(), segments: Vec::new(), ref_slots: Vec::new(), prompts: Default::default(), unlinked_assets: Vec::new(), takes: Vec::new(),
            action: String::new(), dialogue_lines: Vec::new(), emotion_tags: Vec::new(), blocking: Vec::new(),
            ambient_sound: String::new(), sound_effect: String::new(), score: String::new(),
                id: id.clone(), shot_num: i as u32 + 1, image_rev: 0, video_rev: 0,
                scale: "中景".into(), camera_movement: "固定".into(),
                camera_angle: String::new(), camera_note: String::new(),
                director_camera: None, director_scene: None,
                dialogue: String::new(), character_id: "c1".into(), character_ids: Vec::new(), scene_id: None, prop_ids: Vec::new(), mode: ShotMode::FirstFrame,
                prompt: String::new(), negative_prompt: None, duration: 5.0,
                image_url: None, video_url: None, audio_url: None, sketch_url: None, keyframes: Keyframes::NONE, end_image_url: None,
                continues_previous: i > 0, status: "ready".into(),
            });
        }
        std::fs::write(
            ep_dir.join("episode.json"),
            serde_json::to_string(&EpisodeData {
                long_take_scenes: Vec::new(),
                one_take: false,
                scene_takes: Vec::new(),
            long_take_context_frames: None,
            scene_renders: Vec::new(),
                style_lock: String::new(),
                id: eid.into(), episode_num: 1, title: "第一集".into(),
                synopsis: String::new(), script_text: String::new(), script_doc: None,
                target_secs: 0,
                storyboard_version: STORYBOARD_VERSION, actor_colors: Vec::new(),
                storyboard: board, final_video_url: None,
            })
            .unwrap(),
        )
        .unwrap();

        // cinema_projects_dir 会先往上找 `projects/`，所以直接把 cwd 挪进去
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(&dir).unwrap();
        let r = cinema_stitch_episode_with_config_dir(&dir, pid.into(), eid.into()).await;
        std::env::set_current_dir(prev).unwrap();
        let url = r.expect("拼接失败");

        let out = ep_dir.join("final.mp4");
        assert!(out.is_file(), "没生成 final.mp4（返回 {url}）");
        println!("  ✓ {}  {:.2} MB", out.display(),
                 std::fs::metadata(&out).unwrap().len() as f64 / 1e6);
    }

    /// **模型生成的台词音轨不能被静音顶掉。**
    ///
    /// 原来无条件走「TTS 或静音」。2026-09-04 复现：源片 mean_volume −35.5 dB，
    /// 过一遍那步变 −91.0 dB（数字静音），而 ffprobe 照样报 `aac,audio`、
    /// 任务照样成功 —— 整集拼出来是哑片。
    #[test]
    fn 没有tts时保留源片自带音轨() {
        use ShotAudio::*;
        assert_eq!(pick_shot_audio(true, true), Tts, "有 TTS 就用 TTS");
        assert_eq!(pick_shot_audio(true, false), Tts);
        assert_eq!(
            pick_shot_audio(false, true),
            Source,
            "没 TTS 但源片有声音 —— 必须保留，这是模型生成的台词"
        );
        assert_eq!(pick_shot_audio(false, false), Silence, "都没有才补静音");
    }

    /// concat 要求各段宽高一致；等比缩放 + 补黑边，不拉伸。比例只差一丝的铺满裁掉那一丝（见 `frame_filter`）。
    #[test]
    fn 归一画幅不裁切不拉伸且宽高为偶数() {
        let f = fit_filter(720, 1280);
        assert!(f.contains("force_original_aspect_ratio=decrease"), "会裁切或拉伸：{f}");
        assert!(f.contains("pad=720:1280"), "没补黑边：{f}");
        assert!(f.contains("setsar=1"), "没归一像素宽高比：{f}");
        // libx264 + yuv420p 要求偶数，奇数会被编码器拒
        let odd = fit_filter(721, 1281);
        assert!(odd.contains("scale=720:1280"), "奇数没对齐：{odd}");
        // 退化输入不能产出 0
        assert!(fit_filter(0, 0).contains("scale=2:2"));
        // H3 的 1280×736 / 超分后 2560×1472 进 16:9：铺满裁一丝，不加黑边
        assert!(frame_filter(Some((1280, 736)), 1920, 1080).contains("crop=1920:1080"));
        assert!(frame_filter(Some((2560, 1472)), 1920, 1080).contains("crop=1920:1080"));
        // 竖屏片进横屏集、读不出尺寸：加黑边，不能把人裁掉
        assert!(frame_filter(Some((720, 1280)), 1920, 1080).contains("pad="));
        assert!(frame_filter(None, 1920, 1080).contains("pad="));
        // LTX 的 3:2 差 16%，也加黑边
        assert!(frame_filter(Some((768, 512)), 1920, 1080).contains("pad="));
    }

    /// 同一镜重渲要出同一段。随机 seed 会让人以为改提示词有效果，
    /// 其实只是换了个数。
    #[test]
    fn 同一镜的seed稳定且各镜不同() {
        let a = shot_seed("p", "e", "s1");
        assert_eq!(a, shot_seed("p", "e", "s1"));
        assert_ne!(a, shot_seed("p", "e", "s2"));
        assert_ne!(a, shot_seed("p", "e2", "s1"));
        assert_ne!(a, shot_seed("p2", "e", "s1"));
    }

    fn shot(id: &str, num: u32, cont: bool) -> StoryboardShot {
        StoryboardShot { combat_style: String::new(),
            one_take: false,
            continuity: Default::default(),
            continuity_manual: false, route_choice: None,
            look_overrides: Default::default(),
            end_frame: String::new(),
            scene_no: 0, scene_name: String::new(), duty: String::new(), source: String::new(), lighting: String::new(), scene_items: Vec::new(), stage: Default::default(), segments: Vec::new(), ref_slots: Vec::new(), prompts: Default::default(), unlinked_assets: Vec::new(), takes: Vec::new(),
            action: String::new(), dialogue_lines: Vec::new(), emotion_tags: Vec::new(), blocking: Vec::new(),
            ambient_sound: String::new(), sound_effect: String::new(), score: String::new(),
            id: id.into(), shot_num: num, image_rev: 0, video_rev: 0,
            scale: "中景".into(), camera_movement: "固定".into(),
            camera_angle: String::new(), camera_note: String::new(),
            director_camera: None, director_scene: None,
            dialogue: String::new(), character_id: String::new(), character_ids: Vec::new(), scene_id: None, prop_ids: Vec::new(), mode: ShotMode::FirstFrame, prompt: String::new(),
            negative_prompt: None, duration: 2.0, image_url: None, video_url: None,
            audio_url: None, sketch_url: None, keyframes: Keyframes::NONE, end_image_url: None, continues_previous: cont, status: "pending".into(),
        }
    }

    fn 读分集(cfg: &std::path::Path, proj: &str, ep: &str) -> EpisodeData {
        serde_json::from_str(&std::fs::read_to_string(episode_file(cfg, proj, ep)).unwrap()).unwrap()
    }

    fn 写项目(cfg: &std::path::Path, id: &str, kind: &str, eps: Vec<EpisodeData>) {
        let dir = cinema_projects_dir(cfg).join(id);
        std::fs::create_dir_all(dir.join("episodes")).unwrap();
        std::fs::write(
            dir.join("project.json"),
            serde_json::to_string(&CinemaProject {
                default_looks: Default::default(),
                script_project_id: None,
                model_settings: ModelSettings::default(),
                content_mode: default_content_mode(),
                default_shot_mode: ShotMode::default(),
                id: id.into(), title: id.into(), genre: String::new(),
                aspect_ratio: "9:16".into(), description: String::new(),
                created_at: 0, updated_at: 0,
                // 故意写一个过时的快照值，看列表会不会照抄
                episode_count: 99, cover_image: None, kind: kind.into(),
            })
            .unwrap(),
        )
        .unwrap();
        for ep in eps {
            let d = dir.join("episodes").join(&ep.id);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("episode.json"), serde_json::to_string(&ep).unwrap()).unwrap();
        }
    }

    /// 颜色表：按首次出场分色、只有声音的角色不分、存盘、再分一次不变
    #[test]
    fn 分集颜色表_按出场顺序_跳过只有声音的角色_幂等() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        let scope = crate::assets::Scope::Project { project_id: "p1".into() };
        let mut store = crate::assets::load_store(cfg, &scope);
        let a = crate::assets::Asset::new(crate::assets::AssetKind::Character, "沈星晴");
        let b = crate::assets::Asset::new(crate::assets::AssetKind::Character, "陆临川");
        let mut voice = crate::assets::Asset::new(crate::assets::AssetKind::Character, "零");
        voice.character.as_mut().unwrap().disembodied = true;
        let scene = crate::assets::Asset::new(crate::assets::AssetKind::Scene, "面馆");
        let (ida, idb, idv, ids) = (a.id.clone(), b.id.clone(), voice.id.clone(), scene.id.clone());
        store.assets.extend([a, b, voice, scene]);
        crate::assets::save_store(cfg, &scope, &store).unwrap();

        let mut s1 = 镜("x", "");
        s1.character_ids = vec![idb.clone(), idv.clone()];
        let mut s2 = 镜("y", "");
        s2.character_ids = vec![ida.clone(), idb.clone(), ids.clone()];
        let mut ep = 带剧本的分集("ep_01", 1, "", None);
        ep.storyboard = vec![s1, s2];
        写项目(cfg, "p1", "series", vec![ep]);

        let first = assign_actor_colors(cfg, "p1", "ep_01").unwrap();
        let names: Vec<_> = first.colors.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["陆临川", "沈星晴"], "先出场的在前，旁白和场景不分色");
        assert!(first.unassigned.is_empty());

        let on_disk = read_episode(cfg, "p1", "ep_01").unwrap();
        assert_eq!(on_disk.actor_colors, first.colors, "要存盘");

        let again = assign_actor_colors(cfg, "p1", "ep_01").unwrap();
        assert_eq!(again.colors, first.colors);
    }

    /// 前端拿着没带颜色表的分集保存，不能把盘上的表冲掉
    #[test]
    fn 分集颜色表_保存分集时不被冲掉() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        let mut ep = 带剧本的分集("ep_01", 1, "", None);
        ep.actor_colors = vec![crate::actor_colors::ActorColor {
            asset_id: "char_a".into(), name: "甲".into(), color: "#FF00FF".into(), color_name: "FLUORESCENT MAGENTA".into(),
        }];
        写项目(cfg, "p1", "series", vec![ep.clone()]);
        let mut incoming = ep.clone();
        incoming.actor_colors.clear();
        keep_actor_colors(&read_episode(cfg, "p1", "ep_01").unwrap(), &mut incoming);
        assert_eq!(incoming.actor_colors, ep.actor_colors);
    }

    /// 老数据没有 `actor_colors` 字段也要读得出来
    #[test]
    fn 分集颜色表_老数据没有这个字段() {
        let v = serde_json::json!({
            "id": "ep_01", "episode_num": 1, "title": "t", "synopsis": "", "script_text": "",
            "storyboard": [], "final_video_url": null
        });
        let ep: EpisodeData = serde_json::from_value(v).unwrap();
        assert!(ep.actor_colors.is_empty());
    }

    fn 分集(id: &str, shots: Vec<StoryboardShot>, final_url: Option<&str>) -> EpisodeData {
        EpisodeData {
            long_take_scenes: Vec::new(),
            one_take: false,
            scene_takes: Vec::new(),
            long_take_context_frames: None,
            scene_renders: Vec::new(),
            style_lock: String::new(),
            id: id.into(), episode_num: 1, title: id.into(),
            synopsis: String::new(), script_text: String::new(), script_doc: None,
            target_secs: 0,
            storyboard_version: STORYBOARD_VERSION, actor_colors: Vec::new(),
            storyboard: shots, final_video_url: final_url.map(String::from),
        }
    }

    /// 软字幕：合成之后要有一份和成片同名的 SRT，时间轴按镜头依次接
    #[test]
    fn 导出整集字幕_时间轴连着接_没台词的镜头只占时间() {
        let d = tempfile::tempdir().unwrap();
        let mut s1 = 出过片的镜("a");
        s1.dialogue = "大臣：大逆不道！".into();
        s1.duration = 6.0;
        let mut s2 = 出过片的镜("b");
        s2.dialogue = String::new();
        s2.duration = 4.0;
        let mut s3 = 出过片的镜("c");
        s3.dialogue = "林婉清：这是……唐朝？".into();
        s3.duration = 8.0;
        写项目(d.path(), "p1", "standalone", vec![分集("ep_01", vec![s1, s2, s3], None)]);
        // 没有 muxed / 出片文件时退回分镜上写的秒数
        let out = export_srt(d.path(), "p1", "ep_01").unwrap();
        assert!(out.path.ends_with("final.srt"));
        assert_eq!(std::fs::read_to_string(&out.path).unwrap(), out.content);
        assert!(out.content.starts_with("1\n00:00:00,000 --> 00:00:06,000\n大逆不道！"), "{}", out.content);
        // 第二镜没台词，第三镜从 10 秒开始（6 + 4）
        assert!(out.content.contains("00:00:10,000 --> 00:00:18,000\n这是……唐朝？"), "{}", out.content);
        assert_eq!(out.content.matches("-->").count(), 2, "没台词的镜头不产生空字幕");

        // 一句台词都没有时明确报错，不留空文件
        let mut 哑 = 出过片的镜("z");
        哑.dialogue = String::new();
        写项目(d.path(), "p2", "standalone", vec![分集("ep_01", vec![哑], None)]);
        assert!(export_srt(d.path(), "p2", "ep_01").is_err());
        // 还没出片的分集也报错
        写项目(d.path(), "p3", "standalone", vec![分集("ep_01", vec![镜("x", "有台词：喂")], None)]);
        assert!(export_srt(d.path(), "p3", "ep_01").is_err());
    }

    /// 剪映草稿：用合成那套片段、画幅跟项目、字幕和 SRT 同源、不另排配音轨（音轨在片段里）
    #[tokio::test]
    async fn 剪映草稿用当前流程的片段和字幕() {
        let d = tempfile::tempdir().unwrap();
        let mut s1 = 出过片的镜("a");
        s1.dialogue = "大臣：大逆不道！".into();
        s1.duration = 6.0;
        let mut s2 = 出过片的镜("b");
        s2.dialogue = "林婉清：这是……唐朝？".into();
        s2.duration = 4.0;
        写项目(d.path(), "p1", "standalone", vec![分集("ep_01", vec![s1, s2], None)]);
        let ep_dir = cinema_projects_dir(d.path()).join("p1").join("episodes").join("ep_01");
        // 合成产物：muxed 里一镜一个
        std::fs::create_dir_all(ep_dir.join("muxed")).unwrap();
        for id in ["a", "b"] {
            std::fs::write(ep_dir.join("muxed").join(format!("{id}.mp4")), b"fake").unwrap();
        }
        let out = export_jianying(d.path(), "p1", "ep_01", Some(d.path().join("drafts"))).await.unwrap();
        let draft: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(std::path::Path::new(&out).join("draft_content.json")).unwrap()).unwrap();
        assert_eq!((draft["canvas_config"]["width"].as_u64(), draft["canvas_config"]["height"].as_u64(), draft["fps"].as_u64()), (Some(1080), Some(1920), Some(24)), "竖屏项目 1080×1920 / 24 帧");
        let videos = draft["materials"]["videos"].as_array().unwrap();
        assert_eq!(videos.len(), 2);
        assert!(videos[0]["path"].as_str().unwrap().ends_with("muxed/a.mp4"), "{}", videos[0]["path"]);
        let texts = draft["materials"]["texts"].as_array().unwrap();
        assert_eq!(texts.len(), 2, "两条字幕");
        assert!(serde_json::to_string(&texts[0]).unwrap().contains("大逆不道"));
        assert!(!serde_json::to_string(&texts[0]).unwrap().contains("大臣"), "字幕不带角色名");
        assert!(draft["materials"]["audios"].as_array().map(|a| a.is_empty()).unwrap_or(true), "音轨在片段里，不另排配音轨");
        // 没出片的分集不给导
        写项目(d.path(), "p9", "standalone", vec![分集("ep_01", vec![镜("x", "")], None)]);
        assert!(export_jianying(d.path(), "p9", "ep_01", Some(d.path().join("drafts"))).await.is_err());
    }

    /// **导到 FCP：用真视频（ffmpeg 现造）走一遍整条。**
    ///
    /// 用户的要求是「FCP 打开文件后直接读取到内容」—— 所以核的是：时间线里每一个 `file://` 解码回来都是真文件、
    /// 都在用户选的文件夹里（路径带空格和中文）、整份过 FCP 自带的 DTD；没出片的镜头留空、字幕跟着让开；
    /// 配音盖原声；重导不重拷；外置硬盘没挂上要说出来。
    #[tokio::test]
    async fn fcp_导出_每个文件都找得到_格式过校验() {
        use crate::cinema_export_fcp::{export_episode, fcp_dtd, ExportOptions};
        let ff = crate::agent_runtime::video_tool::ffmpeg_binary();
        if std::process::Command::new(ff).arg("-version").output().is_err() {
            eprintln!("没有 ffmpeg，跳过");
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let ep_dir = cinema_projects_dir(d.path()).join("p1").join("episodes").join("ep_01");
        std::fs::create_dir_all(ep_dir.join("shots")).unwrap();
        std::fs::create_dir_all(ep_dir.join("audio")).unwrap();
        let make = |out: &std::path::Path, secs: &str| {
            let o = std::process::Command::new(ff)
                .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=size=320x180:rate=24", "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=32000", "-t", secs, "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
                .arg(out)
                .output()
                .unwrap();
            assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        };
        let (a, c) = (ep_dir.join("shots/a-b-1.mp4"), ep_dir.join("shots/c-b-1.mp4"));
        make(&a, "2");
        make(&c, "1");
        // 第 3 镜配过音
        let o = std::process::Command::new(ff)
            .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", "sine=frequency=220:sample_rate=24000", "-t", "1"])
            .arg(ep_dir.join("audio/c.wav"))
            .output()
            .unwrap();
        assert!(o.status.success());

        let mut s1 = 出过片的镜("a");
        (s1.shot_num, s1.duration, s1.video_url) = (1, 2.0, Some(a.to_string_lossy().into()));
        s1.dialogue = "陆川：你叫我什么？".into();
        s1.scene_name = "场景：拆机间".into();
        let mut s2 = 镜("x", "");
        (s2.id, s2.shot_num, s2.duration) = ("b".into(), 2, 3.0);
        let mut s3 = 出过片的镜("c");
        (s3.shot_num, s3.duration, s3.video_url) = (3, 1.0, Some(c.to_string_lossy().into()));
        s3.dialogue = "星澜：陆川。".into();
        写项目(d.path(), "p1", "standalone", vec![分集("ep_01", vec![s1, s2, s3], None)]);

        let dest = d.path().join("外置 硬盘");
        std::fs::create_dir_all(&dest).unwrap();
        let opts = ExportOptions { dest_dir: dest.to_string_lossy().into(), event: "02_S01E01_第三次".into(), project: "S01E01 第三次".into(), upscale: false };
        let ex = export_episode(d.path(), "p1", "ep_01", &opts, |_, _, _| {}).await.unwrap();
        assert_eq!((ex.clips, ex.missing.clone()), (2, vec![2]));
        assert!(ex.folder.starts_with(&*dest.to_string_lossy()), "{}", ex.folder);

        let xml = std::fs::read_to_string(&ex.fcpxml).unwrap();
        let srcs: Vec<String> = xml
            .match_indices("src=\"file://")
            .map(|(i, m)| {
                let rest = &xml[i + m.len()..];
                urlencoding::decode(&rest[..rest.find('"').unwrap()]).unwrap().into_owned()
            })
            .collect();
        assert_eq!(srcs.len(), 3, "两条视频 + 一条配音");
        for p in &srcs {
            assert!(std::path::Path::new(p).is_file(), "时间线里写的文件不存在：{p}");
            assert!(p.starts_with(&ex.folder), "素材没拷进导出文件夹：{p}");
        }
        if fcp_dtd().is_some() {
            assert!(ex.validated, "装了 FCP 却没校验");
        }
        assert!(xml.contains(r#"<event name="02_S01E01_第三次">"#) && xml.contains(r#"<project name="S01E01 第三次">"#));
        assert!(xml.contains(r#"name="第 1 镜 · 拆机间""#));
        assert!(xml.contains(r#"<gap name="第 2 镜 · 未出片" offset="48/24s" start="0s" duration="72/24s">"#), "{xml}");
        assert!(xml.contains(r#"<adjust-volume amount="-96dB"/>"#), "配音要盖掉原声");

        let srt = std::fs::read_to_string(std::path::Path::new(&ex.folder).join("S01E01 第三次.srt")).unwrap();
        assert!(srt.contains("你叫我什么？") && !srt.contains("陆川："), "{srt}");
        // 星澜那句在第 3 镜：2 秒 + 空着的 3 秒 = 第 5 秒开始。空镜不占位的话这里是第 2 秒
        assert!(srt.contains("00:00:05,000 --> "), "{srt}");

        // 重导一次不重拷
        let media = std::path::Path::new(&ex.folder).join("媒体");
        let n = std::fs::read_dir(&media).unwrap().count();
        export_episode(d.path(), "p1", "ep_01", &opts, |_, _, _| {}).await.unwrap();
        assert_eq!(std::fs::read_dir(&media).unwrap().count(), n);

        // 外置硬盘没挂上：说出来，不是一句「失败」
        let gone = ExportOptions { dest_dir: "/Volumes/不存在的盘/回收师".into(), ..opts.clone() };
        let err = export_episode(d.path(), "p1", "ep_01", &gone, |_, _, _| {}).await.unwrap_err().to_string();
        assert!(err.contains("外置硬盘"), "{err}");
    }

    /// 整段出的场（长镜头）在 FCP 里放**一整条**，不按镜拆（2026-09-30 用户：H3 自己分好了、生成完组装上，不要再切）。
    /// 这一场各镜自己没片子也不算缺镜；台词按各镜时长铺在这一条上
    #[tokio::test]
    async fn fcp_导出_整段出的场放一整条() {
        use crate::cinema_export_fcp::{export_episode, ExportOptions};
        let ff = crate::agent_runtime::video_tool::ffmpeg_binary();
        if std::process::Command::new(ff).arg("-version").output().is_err() {
            eprintln!("没有 ffmpeg，跳过");
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let ep_dir = cinema_projects_dir(d.path()).join("p1").join("episodes").join("ep_01");
        std::fs::create_dir_all(ep_dir.join("shots")).unwrap();
        let make = |out: &std::path::Path, secs: &str| {
            let o = std::process::Command::new(ff)
                .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=size=320x180:rate=24", "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=32000", "-t", secs, "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
                .arg(out)
                .output()
                .unwrap();
            assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        };
        let (a, lt) = (ep_dir.join("shots/a-b-1.mp4"), ep_dir.join("shots/longtake_s2-1.mp4"));
        make(&a, "2");
        make(&lt, "4");
        let mut s1 = 出过片的镜("a");
        (s1.shot_num, s1.scene_no, s1.duration, s1.video_url) = (1, 1, 2.0, Some(a.to_string_lossy().into()));
        let mut s2 = 镜("x", "");
        (s2.id, s2.shot_num, s2.scene_no, s2.duration) = ("b".into(), 2, 2, 2.0);
        let mut s3 = 镜("x", "");
        (s3.id, s3.shot_num, s3.scene_no, s3.duration) = ("c".into(), 3, 2, 2.0);
        s3.dialogue = "星澜：陆川。".into();
        let mut ep = 分集("ep_01", vec![s1, s2, s3], None);
        ep.long_take_scenes = vec![2];
        ep.scene_renders = vec![SceneRender { scene_no: 2, route: crate::shot_table::PromptRoute::B, path: lt.to_string_lossy().into(), created_at: 1, ..Default::default() }];
        写项目(d.path(), "p1", "standalone", vec![ep]);

        let dest = d.path().join("导出");
        std::fs::create_dir_all(&dest).unwrap();
        let opts = ExportOptions { dest_dir: dest.to_string_lossy().into(), event: "E".into(), project: "P".into(), upscale: false };
        let ex = export_episode(d.path(), "p1", "ep_01", &opts, |_, _, _| {}).await.unwrap();
        assert_eq!((ex.clips, ex.missing.clone()), (2, vec![]), "第 2 场放一整条、不算缺镜");
        let xml = std::fs::read_to_string(&ex.fcpxml).unwrap();
        assert!(xml.contains(r#"name="第 2 场 · 长镜头（第 2–3 镜）""#), "{xml}");
        assert!(!xml.contains("第 3 镜 · 未出片"), "{xml}");
        // 星澜那句在第 3 镜：第 1 镜 2 秒 + 长镜头前一半 2 秒 = 第 4 秒开始
        let srt = std::fs::read_to_string(std::path::Path::new(&ex.folder).join("P.srt")).unwrap();
        assert!(srt.contains("00:00:04,000 --> ") && srt.contains("陆川。"), "{srt}");

        // 这一场改回分镜出了：不再用整条
        let mut ep = 读分集(d.path(), "p1", "ep_01");
        ep.long_take_scenes.clear();
        写项目(d.path(), "p1", "standalone", vec![ep]);
        let ex = export_episode(d.path(), "p1", "ep_01", &opts, |_, _, _| {}).await.unwrap();
        assert_eq!(ex.missing, vec![2, 3]);
    }

    /// **一集由哪些片段组成**：整段出的场放一整条，位置在这一场第一次出现的地方，
    /// 这一场其余镜头不再各占一段。组装页、合成一集、剪映、FCP 都按这个排。
    #[test]
    fn 片段顺序_整段出的场放一整条_其余镜头各占一段() {
        let d = tempfile::tempdir().unwrap();
        let lt = d.path().join("longtake_s2-1.mp4");
        std::fs::write(&lt, b"x").unwrap();
        let mk = |id: &str, n: u32, scene: u32| {
            let mut s = 镜("x", "");
            (s.id, s.shot_num, s.scene_no) = (id.into(), n, scene);
            s
        };
        // 第 3 镜被挪到了第 1 镜前面（组装页能调顺序）：长镜头放在第 2 场第一次出现的位置
        let mut ep = 分集("ep_01", vec![mk("c", 3, 2), mk("a", 1, 1), mk("b", 2, 2), mk("d", 4, 3)], None);
        ep.long_take_scenes = vec![2];
        ep.scene_renders = vec![SceneRender { scene_no: 2, route: crate::shot_table::PromptRoute::A, path: lt.to_string_lossy().into(), created_at: 1, ..Default::default() }];
        let segs = episode_segments(&ep, crate::shot_table::PromptRoute::B);
        let shape: Vec<String> = segs
            .iter()
            .map(|g| match g {
                EpisodeSegment::Shot(s) => s.id.clone(),
                EpisodeSegment::LongTake { scene_no, shots, .. } => format!("场{scene_no}[{}]", shots.iter().map(|s| s.id.as_str()).collect::<Vec<_>>().join(",")),
            })
            .collect();
        // 当前线路是 B、只有 A 出过：照样用（总比空着强）；成员按镜号排
        assert_eq!(shape, vec!["场2[b,c]", "a", "d"]);

        // 这一场改回分镜：整条不再用，各镜各占一段
        ep.long_take_scenes.clear();
        assert_eq!(episode_segments(&ep, crate::shot_table::PromptRoute::B).len(), 4);
        // 设成整段出、但片子文件已经不在了：也按各镜算，不放一条找不到的文件
        ep.long_take_scenes = vec![2];
        std::fs::remove_file(&lt).unwrap();
        assert!(long_take_clip(&ep, 2, crate::shot_table::PromptRoute::A).is_none());
        assert_eq!(episode_segments(&ep, crate::shot_table::PromptRoute::A).len(), 4);
    }

    /// **「合成一集」认整段出的场。** 2026-10-01 用户截图：第 5 场整段出过，组装页 22~24 镜
    /// 一直显示「未出片」，合成也把这三镜当缺镜跳过 —— 只有 FCP 导出认长镜头。
    #[tokio::test]
    async fn 合成一集_整段出的场放一整条_不算缺镜() {
        let ff = crate::agent_runtime::video_tool::ffmpeg_binary();
        if std::process::Command::new(ff).arg("-version").output().is_err() {
            eprintln!("没有 ffmpeg，跳过");
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let ep_dir = cinema_projects_dir(d.path()).join("p1").join("episodes").join("ep_01");
        std::fs::create_dir_all(ep_dir.join("shots")).unwrap();
        let make = |out: &std::path::Path, secs: &str| {
            let o = std::process::Command::new(ff)
                .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=size=320x180:rate=24", "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=32000", "-t", secs, "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
                .arg(out)
                .output()
                .unwrap();
            assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        };
        let (a, lt) = (ep_dir.join("shots/a-b-1.mp4"), ep_dir.join("shots/longtake_s2-1.mp4"));
        make(&a, "2");
        make(&lt, "4");
        let mut s1 = 出过片的镜("a");
        (s1.shot_num, s1.scene_no, s1.duration, s1.video_url) = (1, 1, 2.0, Some(a.to_string_lossy().into()));
        let mut s2 = 镜("x", "");
        (s2.id, s2.shot_num, s2.scene_no, s2.duration) = ("b".into(), 2, 2, 2.0);
        let mut s3 = 镜("x", "");
        (s3.id, s3.shot_num, s3.scene_no, s3.duration) = ("c".into(), 3, 2, 2.0);
        s3.dialogue = "星澜：陆川。".into();
        let mut ep = 分集("ep_01", vec![s1, s2, s3], None);
        ep.long_take_scenes = vec![2];
        ep.scene_renders = vec![SceneRender { scene_no: 2, route: crate::shot_table::PromptRoute::B, path: lt.to_string_lossy().into(), created_at: 1, ..Default::default() }];
        写项目(d.path(), "p1", "standalone", vec![ep.clone()]);

        // 先加一镜真缺片的（第 3 场第 4 镜）：缺镜名单里只能有它，**不能**有长镜头盖住的第 2、3 镜
        let mut s4 = 镜("x", "");
        (s4.id, s4.shot_num, s4.scene_no, s4.duration) = ("d".into(), 4, 3, 2.0);
        let mut with_gap = ep.clone();
        with_gap.storyboard.push(s4);
        写项目(d.path(), "p1", "standalone", vec![with_gap]);
        let (_, notes) = stitch_episode(d.path(), "p1".into(), "ep_01".into(), false, false, |_, _, _| {}).await.unwrap();
        let note = notes.iter().find(|n| n.contains("部分预览")).expect("缺了第 4 镜，该报部分预览");
        assert!(note.contains("4 个镜头里有 1 个") && note.contains("第 4 镜"), "{note}");
        assert!(!note.contains("第 2 镜") && !note.contains("第 3 镜"), "长镜头盖住的镜头被当成缺镜了：{note}");
        写项目(d.path(), "p1", "standalone", vec![ep]);

        let (out, notes) = stitch_episode(d.path(), "p1".into(), "ep_01".into(), false, false, |_, _, _| {}).await.unwrap();
        assert!(notes.iter().all(|n| !n.contains("部分预览")), "整段出的场不该算缺镜：{notes:?}");
        // 第 1 镜 2 秒 + 长镜头 4 秒 = 6 秒（容差给编码器）
        // 返回的是界面用的地址（`/projects/…`），文件在分集目录下
        assert!(out.ends_with("final.mp4"), "{out}");
        let secs = clip_seconds(&ep_dir.join("final.mp4")).unwrap();
        assert!((secs - 6.0).abs() < 0.3, "成片 {secs} 秒，应该约 6 秒");
        assert!(ep_dir.join("muxed/longtake_s2.mp4").is_file(), "长镜头的标准化片段没落在约定的位置（剪映导出要读它）");

        // 剪映草稿：同一份片段 —— 一条长镜头，不是两个「第 N 镜」
        let draft = export_jianying(d.path(), "p1", "ep_01", Some(d.path().join("剪映"))).await.unwrap();
        let mut json = String::new();
        for e in walkdir_files(std::path::Path::new(&draft)) {
            if e.extension().is_some_and(|x| x == "json") {
                json.push_str(&std::fs::read_to_string(&e).unwrap_or_default());
            }
        }
        assert!(json.contains("第 2 场 · 长镜头（第 2–3 镜）"), "剪映草稿里没有那条长镜头");
        assert!(!json.contains("第 3 镜\""), "第 3 镜不该再单独占一段");

        // 这一场改回分镜出：各镜各算，第 2、3 镜没有自己的片 —— 部分预览
        let mut ep = 读分集(d.path(), "p1", "ep_01");
        ep.long_take_scenes.clear();
        写项目(d.path(), "p1", "standalone", vec![ep]);
        let (_, notes) = stitch_episode(d.path(), "p1".into(), "ep_01".into(), false, false, |_, _, _| {}).await.unwrap();
        assert!(notes.iter().any(|n| n.contains("部分预览") && n.contains("第 2 镜") && n.contains("第 3 镜")), "{notes:?}");
    }

    fn walkdir_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() { out.extend(walkdir_files(&p)); } else { out.push(p); }
            }
        }
        out
    }

    fn 出过片的镜(id: &str) -> StoryboardShot {
        let mut s = 镜("x", "");
        s.id = id.into();
        s.video_url = Some(format!("/v/{id}.mp4"));
        s
    }

    /// **收图要落在约定死的那个路径上。** 出片、重出、打包都按
    /// `shots/<id>.png` 找 —— 落错地方的表现是画布上看着收好了，
    /// 导出的时候少一帧。
    #[test]
    fn 画布收图落在分镜的首帧位置() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        写项目(cfg, "p1", "series", vec![分集("ep_01", vec![镜("s1", "")], None)]);
        let src = dir.path().join("画布产物.png");
        std::fs::write(&src, b"PNG-BYTES").unwrap();

        let rev = adopt_frame(cfg, "p1", "ep_01", "s1", src.to_str().unwrap()).unwrap();
        let dst = cinema_projects_dir(cfg)
            .join("p1").join("episodes").join("ep_01").join("shots").join("s1.png");
        assert_eq!(std::fs::read(&dst).unwrap(), b"PNG-BYTES");
        // 路径不变，前端靠这个数换 `v=`；给 0 的话界面一直显示旧图。
        assert!(rev > 0, "没给出新的 image_rev");
        // **原图不删** —— 画布节点上那张还在显示着。
        assert!(src.is_file(), "把画布上那张图删了");
    }

    /// **镜头不存在就当场报错。** 不拦的话会在 shots/ 里留下一个
    /// 谁也找不到的文件，而界面上什么都不会变 —— 这种错最难查。
    #[test]
    fn 收图到不存在的镜头要报错而不是留下孤儿文件() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        写项目(cfg, "p1", "series", vec![分集("ep_01", vec![镜("s1", "")], None)]);
        let src = dir.path().join("画布产物.png");
        std::fs::write(&src, b"PNG-BYTES").unwrap();

        assert!(adopt_frame(cfg, "p1", "ep_01", "不存在", src.to_str().unwrap()).is_err());
        let shots = cinema_projects_dir(cfg)
            .join("p1").join("episodes").join("ep_01").join("shots");
        let leftovers: Vec<_> = std::fs::read_dir(&shots)
            .map(|d| d.flatten().map(|e| e.file_name()).collect())
            .unwrap_or_default();
        assert!(leftovers.is_empty(), "留下了孤儿文件：{leftovers:?}");
    }

    /// 回填要落成**新的一「次」**，不能覆盖已有的片子。
    #[test]
    fn 画布回填视频追加一次而不覆盖() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        let mut s1 = 镜("s1", "");
        s1.takes.push(VideoTake { path: "/旧/片.mp4".into(), created_at: 1, ..Default::default() });
        s1.video_url = Some("/旧/片.mp4".into());
        写项目(cfg, "p1", "series", vec![分集("ep_01", vec![s1], None)]);
        let src = dir.path().join("画布出的.mp4");
        std::fs::write(&src, b"MP4").unwrap();

        let out = adopt_video(cfg, "p1", "ep_01", "s1", src.to_str().unwrap(), "一段提示词", "h3", 42,
            vec!["/参考/客厅.png".into(), "/参考/筱筱.png".into()]).unwrap();
        assert_eq!(out.take_count, 2, "把旧的那一次覆盖掉了");

        let ep = 读分集(cfg, "p1", "ep_01");
        let shot = &ep.storyboard[0];
        assert_eq!(shot.video_url.as_deref(), Some(out.video_url.as_str()), "video_url 没指向新收的这条");
        assert_eq!(shot.takes[0].path, "/旧/片.mp4", "旧的那一次得留着，不然换不回去");
        let t = &shot.takes[1];
        assert_eq!(t.origin, TakeOrigin::Canvas);
        assert_eq!(t.prompt, "一段提示词");
        assert_eq!((t.seed, t.pictures), (42, 2));
        // **「它用了哪张场景图」要答得出来。** 只存张数的话永远答不出来，
        // 而那正是回看历史 take 时唯一想知道的事。顺序也有意义（<Picture N> 的编号）。
        assert_eq!(t.picture_paths, vec!["/参考/客厅.png", "/参考/筱筱.png"]);
        // **指纹必须留空。** 写个指纹进去等于宣称「它和当前字段对得上」，
        // 而回填的片根本不是按镜头表出的 —— 那句假话会盖掉真正的过期警告。
        assert!(t.fields_hash.is_empty(), "给回填的片编了个指纹");
        // 原片不删 —— 画布节点上还在显示着
        assert!(src.is_file());
    }

    /// **回填的那一「次」不算「这条线路出过片」。**
    ///
    /// 混进去的两个后果都不报错：批量出片跳过这一镜；过期警告被盖掉。
    #[test]
    fn 回填的次不算出过片也不盖掉过期警告() {
        use crate::shot_table::PromptRoute;
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        let mut s1 = 镜("s1", "");
        s1.prompts.insert(PromptRoute::B, crate::shot_table::RoutePrompts {
            video: "有正文".into(), fields_hash: "老指纹".into(), ..Default::default() });
        // 分镜页出过一次（带指纹），之后字段改了 —— 应该报过期
        s1.takes.push(VideoTake { route: PromptRoute::B, path: "/出片/b.mp4".into(),
            fields_hash: "老指纹".into(), origin: TakeOrigin::Render, ..Default::default() });
        写项目(cfg, "p1", "series", vec![分集("ep_01", vec![s1], None)]);

        let before = 读分集(cfg, "p1", "ep_01").storyboard;
        let 过期 = |sb: &[StoryboardShot]| stale_take_issues(sb, &[], "", PromptRoute::B, &[]).len();
        assert_eq!(过期(&before), 1, "前提不成立：本来就该报一条过期");

        let src = dir.path().join("画布出的.mp4");
        std::fs::write(&src, b"MP4").unwrap();
        adopt_video(cfg, "p1", "ep_01", "s1", src.to_str().unwrap(), "词", "h3", 0, vec![]).unwrap();

        let after = 读分集(cfg, "p1", "ep_01").storyboard;
        assert_eq!(过期(&after), 1, "回填之后过期警告没了 —— 它把真正该报的那条盖掉了");

        // 批量出片仍然认为这一镜「还没出过 B」吗？回填不该改变这个判断。
        // （这里直接验 fresh 的判据，和 shot_ids_for_route 用的是同一条）
        let 出过 = |sb: &[StoryboardShot]| sb[0].takes.iter()
            .any(|t| t.route == PromptRoute::B && t.origin == TakeOrigin::Render);
        assert!(出过(&after), "前提：分镜页确实出过一次 B");
        let mut 只有回填 = 镜("s2", "");
        只有回填.takes.push(VideoTake { route: PromptRoute::B, path: "/画布/x.mp4".into(),
            origin: TakeOrigin::Canvas, ..Default::default() });
        assert!(!只有回填.takes.iter().any(|t| t.route == PromptRoute::B && t.origin == TakeOrigin::Render),
            "只有回填片的镜头被当成出过片了 —— 批量出片会跳过它");
    }

    /// **「上一镜的成片」要认回填的那条。** 用户眼里那就是他最后看到的片子。
    /// 这一处和上面两处方向相反，所以不能一刀切按 origin 过滤。
    #[test]
    fn 接上一镜要认回填的那条() {
        let mut prev = 镜("s1", "");
        prev.takes.push(VideoTake { path: "/出片/b.mp4".into(), created_at: 1, origin: TakeOrigin::Render, ..Default::default() });
        prev.takes.push(VideoTake { path: "/画布/x.mp4".into(), created_at: 2, origin: TakeOrigin::Canvas, ..Default::default() });
        assert_eq!(continuity_source(&prev).unwrap().path, "/画布/x.mp4");
    }

    /// 回填必须能撤销 —— 画布出的片不一定比原来那条好。
    #[test]
    fn 换回某一次只改指向不动历史() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        let 旧 = dir.path().join("旧.mp4");
        std::fs::write(&旧, b"OLD").unwrap();
        let mut s1 = 镜("s1", "");
        s1.takes.push(VideoTake { path: 旧.to_string_lossy().to_string(), created_at: 1, ..Default::default() });
        写项目(cfg, "p1", "series", vec![分集("ep_01", vec![s1], None)]);
        let src = dir.path().join("新.mp4");
        std::fs::write(&src, b"MP4").unwrap();
        adopt_video(cfg, "p1", "ep_01", "s1", src.to_str().unwrap(), "词", "h3", 0, vec![]).unwrap();

        pick_take(cfg, "p1", "ep_01", "s1", &旧.to_string_lossy()).unwrap();
        let ep = 读分集(cfg, "p1", "ep_01");
        assert_eq!(ep.storyboard[0].video_url.as_deref(), Some(旧.to_string_lossy().as_ref()));
        assert_eq!(ep.storyboard[0].takes.len(), 2, "换回去把历史弄丢了");

        // **不能拿这个接口把任意路径写进分集。**
        assert!(pick_take(cfg, "p1", "ep_01", "s1", "/别人的/片.mp4").is_err());
    }

    /// 源图不在了要说清楚，而不是写出一个 0 字节的首帧。
    #[test]
    fn 源图不在了要报错() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        写项目(cfg, "p1", "series", vec![分集("ep_01", vec![镜("s1", "")], None)]);
        assert!(adopt_frame(cfg, "p1", "ep_01", "s1", "/没有这个/图.png").is_err());
    }



    /// 状态是**算出来的**。存一个字段就一定会和事实漂移。
    #[test]
    fn 工作区状态由磁盘上的分集算出来() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        写项目(cfg, "空的", "standalone", vec![分集("ep_01", vec![], None)]);
        写项目(cfg, "有镜头", "series", vec![分集("ep_01", vec![镜("x", "")], None)]);
        写项目(cfg, "出片了", "series", vec![分集("ep_01", vec![出过片的镜("s1")], Some("/f.mp4"))]);

        let items = workspace_items(cfg);
        let by = |id: &str| items.iter().find(|i| i.project.id == id).unwrap();
        assert_eq!(by("空的").status, "draft");
        assert_eq!(by("有镜头").status, "active");
        assert_eq!(by("出片了").status, "done");
        assert_eq!(by("出片了").rendered_count, 1);
    }

    /// **工作区卡片要有封面。**
    ///
    /// 2026-09-08：`cover_image` 字段一直在、界面一直读，但全仓库没有一处
    /// 给它赋过值 —— 每张卡都是纯色渐变。
    ///
    /// 这条测试钉的是 `workspace_items` 这条路，**因为第一次我修错了地方**：
    /// 补在了 `cinema_list_projects` 里，而工作区走的是这个函数，
    /// 界面上一点变化都没有。单测得钉在界面真正走的那条路上。
    #[test]
    fn 工作区卡片自动带封面() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        let mut sh = 镜("x", "");
        sh.id = "s1".into();
        写项目(cfg, "有图的", "series", vec![分集("ep_01", vec![sh], None)]);
        写项目(cfg, "没图的", "series", vec![分集("ep_01", vec![镜("y", "")], None)]);
        // 首帧文件真的落盘 —— `resolve_media_field` 判据是文件在不在
        let shots = cinema_projects_dir(cfg).join("有图的/episodes/ep_01/shots");
        std::fs::create_dir_all(&shots).unwrap();
        std::fs::write(shots.join("s1.png"), b"x").unwrap();

        let items = workspace_items(cfg);
        let by = |id: &str| items.iter().find(|i| i.project.id == id).unwrap();
        let cover = by("有图的").project.cover_image.clone().expect("没挑出封面");
        assert!(cover.ends_with("s1.png"), "挑错了：{cover}");
        assert!(std::path::Path::new(&cover).is_file(), "不是真路径：{cover}");
        // 一张首帧都没有的保持没有 —— 界面会退回渐变色，那是设计好的
        assert_eq!(by("没图的").project.cover_image, None);
    }

    #[test]
    fn 集数用磁盘上的真实数量_不用_json_里的快照() {
        // project.json 里写的是 99 —— 那是写入时的快照，会过时。
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        写项目(cfg, "p", "series", vec![分集("ep_01", vec![], None), 分集("ep_02", vec![], None)]);
        let items = workspace_items(cfg);
        assert_eq!(items[0].episode_count, 2);
        assert_eq!(items[0].project.episode_count, 2, "快照值没被纠正");
    }

    #[test]
    fn 老项目没有_kind_字段时当系列() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        let d = cinema_projects_dir(cfg).join("old");
        std::fs::create_dir_all(&d).unwrap();
        // 迁移前的 project.json 没有 kind
        std::fs::write(
            d.join("project.json"),
            r#"{"id":"old","title":"老的","genre":"","aspect_ratio":"9:16",
                "description":"","created_at":0,"updated_at":0,
                "episode_count":1,"cover_image":null}"#,
        )
        .unwrap();
        let items = workspace_items(cfg);
        assert_eq!(items.len(), 1, "老数据读不出来了");
        assert_eq!(items[0].project.kind, "series");
    }

    #[test]
    fn 读不动的项目被跳过_而不是让整个列表失败() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        写项目(cfg, "好的", "series", vec![分集("ep_01", vec![], None)]);
        let bad = cinema_projects_dir(cfg).join("坏的");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join("project.json"), "{ 这不是 json").unwrap();
        // 一个坏文件不该让工作区整个空掉。
        assert_eq!(workspace_items(cfg).len(), 1);
    }

    // ---- 剧本一览 --------------------------------------------------------

    fn 带剧本的分集(id: &str, num: u32, script: &str, doc: Option<&str>) -> EpisodeData {
        EpisodeData {
            long_take_scenes: Vec::new(),
            one_take: false,
            scene_takes: Vec::new(),
            long_take_context_frames: None,
            scene_renders: Vec::new(),
            style_lock: String::new(),
            id: id.into(), episode_num: num, title: format!("第 {num} 集"),
            synopsis: String::new(), script_text: script.into(),
            target_secs: 0,
            storyboard_version: STORYBOARD_VERSION, actor_colors: Vec::new(),
            script_doc: doc.map(String::from),
            storyboard: Vec::new(), final_video_url: None,
        }
    }

    #[test]
    fn 剧本一览跨项目列出所有分集() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        写项目(cfg, "p1", "series", vec![
            带剧本的分集("ep_01", 1, "第一集的剧本", None),
            带剧本的分集("ep_02", 2, "", None),
        ]);
        写项目(cfg, "p2", "standalone", vec![带剧本的分集("ep_01", 1, "另一个项目", None)]);

        let all = script_summaries(cfg);
        assert_eq!(all.len(), 3);
        assert_eq!(all.iter().filter(|s| s.project_id == "p1").count(), 2);
    }

    #[test]
    fn 有内容的排前面() {
        // 打开这一页多半是想接着写，空的没必要顶在最上面。
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        写项目(cfg, "p1", "series", vec![
            带剧本的分集("ep_01", 1, "", None),
            带剧本的分集("ep_02", 2, "有字", None),
        ]);
        let all = script_summaries(cfg);
        assert_eq!(all[0].episode_id, "ep_02");
    }

    #[test]
    fn 字数按字符数不是字节数() {
        // 中文按字节数会大三倍，界面上直接是错的。
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        写项目(cfg, "p1", "series", vec![带剧本的分集("ep_01", 1, "老茶师擦壶", None)]);
        assert_eq!(script_summaries(cfg)[0].chars, 5);
    }

    #[test]
    fn 结构文档原样带出去() {
        // **不在 Rust 里数场景。** 那套规则在前端，
        // 在这儿再写一份两边迟早对不上，而且对不上时不报错。
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        写项目(cfg, "p1", "series", vec![带剧本的分集("ep_01", 1, "x", Some(r#"{"type":"doc"}"#))]);
        assert_eq!(script_summaries(cfg)[0].script_doc.as_deref(), Some(r#"{"type":"doc"}"#));
    }

    #[test]
    fn 读不动的分集跳过_不让整页空掉() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        写项目(cfg, "p1", "series", vec![带剧本的分集("ep_01", 1, "好的", None)]);
        let bad = cinema_projects_dir(cfg).join("p1").join("episodes").join("坏的");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join("episode.json"), "{ 不是 json").unwrap();
        assert_eq!(script_summaries(cfg).len(), 1);
    }

    // ---- 删除项目 --------------------------------------------------------

    #[test]
    fn 不带确认时只回摘要_不删东西() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        写项目(cfg, "p1", "series", vec![分集("ep_01", vec![出过片的镜("s1")], None)]);
        let summary = delete_project(cfg, "p1", false).unwrap();
        assert!(summary.contains("1 镜"), "摘要里没说有多少东西：{summary}");
        assert!(
            cinema_projects_dir(cfg).join("p1").is_dir(),
            "没确认就把项目删了"
        );
    }

    #[test]
    fn 确认之后真的删掉整个目录() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        写项目(cfg, "p1", "series", vec![分集("ep_01", vec![], None)]);
        delete_project(cfg, "p1", true).unwrap();
        assert!(!cinema_projects_dir(cfg).join("p1").exists());
    }

    #[test]
    fn 不存在的项目报错_而不是当成删成功() {
        let dir = tempfile::tempdir().unwrap();
        assert!(delete_project(dir.path(), "查无此项", true).is_err());
    }

    /// id 是前端传来的字符串。`../..` 之类的东西**不能**变成一次
    /// `remove_dir_all` —— 这是这一页里唯一真正危险的操作。
    #[test]
    fn 路径穿越删不动_projects_外面的东西() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        写项目(cfg, "p1", "series", vec![分集("ep_01", vec![], None)]);
        let outside = cfg.join("别动我");
        std::fs::create_dir_all(&outside).unwrap();

        assert!(delete_project(cfg, "../别动我", true).is_err());
        assert!(delete_project(cfg, "..", true).is_err());
        assert!(outside.is_dir(), "把 projects 外面的目录删了");
        assert!(cinema_projects_dir(cfg).is_dir(), "把整个 projects 删了");
    }

    // ---- 分镜解析 --------------------------------------------------------












    // ---- 角色：资产库是唯一的所有者 --------------------------------------

    fn 角色(id: &str, name: &str) -> CinemaCharacter {
        CinemaCharacter {
            id: id.into(), name: name.into(), role: "主角".into(),
            avatar_url: String::new(), trigger_words: "1girl".into(),
            lora_model: None, lora_weight: 0.8, consistency_mode: "pulid".into(),
            clothing_prompt: Some("长衫".into()), multi_angles: Vec::new(),
            asset_sheet: None, visual_dna: None, seed: Some(42), episode_id: None,
        }
    }

    // ── 实体对齐 ──────────────────────────────────────────────────────

    #[test]
    fn 匹配度_完全相同一百_子串七十五_否则零() {
        assert_eq!(match_confidence("老茶师", "老茶师"), 100);
        assert_eq!(match_confidence("老茶馆（清晨）", "老茶馆"), 75);
        assert_eq!(match_confidence("老茶馆", "老茶馆（清晨）"), 75, "反过来也要认");
        assert_eq!(match_confidence("老茶师", "小徒弟"), 0);
    }

    #[test]
    fn 匹配度_空名字不匹配任何东西() {
        // 不挡的话，两个空名字会算成 100 —— 一堆没名字的实体互相合并
        assert_eq!(match_confidence("", ""), 0);
        assert_eq!(match_confidence("  ", "老茶师"), 0);
        assert_eq!(match_confidence("老茶师", ""), 0);
    }

    #[test]
    fn 匹配度_忽略大小写和首尾空白() {
        assert_eq!(match_confidence(" Tea Master ", "tea master"), 100);
    }

    /// 往指定作用域塞一个资产，回它的 id。（已有的 `建资产` 只管项目级。）
    fn 建到(cfg: &Path, scope: crate::assets::Scope, kind: crate::assets::AssetKind, name: &str) -> String {
        let mut store = crate::assets::load_store(cfg, &scope);
        let a = crate::assets::Asset::new(kind, name);
        let id = a.id.clone();
        store.assets.push(a);
        crate::assets::save_store(cfg, &scope, &store).unwrap();
        id
    }

    /// 第二集重抽时，同名的要认出来 —— 这是整条链路唯一的判据。
    #[test]
    fn 对齐建议_同名的认得出来() {
        use crate::assets::{AssetKind, Scope};
        let d = tempfile::tempdir().unwrap();
        let cfg = d.path();
        let series = Scope::Project { project_id: "p1".into() };
        let ep = Scope::Episode { project_id: "p1".into(), episode_id: "ep_02".into() };
        let sid = 建到(cfg, series.clone(), AssetKind::Character, "老茶师");
        建到(cfg, ep.clone(), AssetKind::Character, "老茶师");
        建到(cfg, ep.clone(), AssetKind::Character, "灰斗篷客人");

        let out = suggest_alignment(cfg, "p1", "ep_02");
        let 老 = out.iter().find(|s| s.local_name == "老茶师").unwrap();
        assert_eq!(老.confidence, 100);
        assert_eq!(老.target_id.as_deref(), Some(sid.as_str()));
        let 客 = out.iter().find(|s| s.local_name == "灰斗篷客人").unwrap();
        assert_eq!(客.confidence, 0, "新角色不该匹配到任何东西");
        assert!(客.target_id.is_none());
    }

    /// **只在同类之间比。** 不限类的话，道具「铜壶」会匹配到场景「铜壶」——
    /// 名字一样但根本是两个东西，合并之后镜头引用指错，而且不报错。
    #[test]
    fn 对齐建议_不跨类匹配() {
        use crate::assets::{AssetKind, Scope};
        let d = tempfile::tempdir().unwrap();
        let cfg = d.path();
        建到(cfg, Scope::Project { project_id: "p1".into() }, AssetKind::Scene, "铜壶");
        建到(
            cfg,
            Scope::Episode { project_id: "p1".into(), episode_id: "ep_01".into() },
            AssetKind::Prop,
            "铜壶",
        );
        let out = suggest_alignment(cfg, "p1", "ep_01");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].confidence, 0, "道具匹配到了场景");
        assert!(out[0].target_id.is_none());
    }

    /// 多个候选时取最像的那个，不是取第一个碰到的。
    #[test]
    fn 对齐建议_取最像的那个() {
        use crate::assets::{AssetKind, Scope};
        let d = tempfile::tempdir().unwrap();
        let cfg = d.path();
        let series = Scope::Project { project_id: "p1".into() };
        // 先建子串候选，再建完全相同的 —— 顺序刻意反着放
        建到(cfg, series.clone(), AssetKind::Scene, "老茶馆的后院");
        let 准 = 建到(cfg, series.clone(), AssetKind::Scene, "老茶馆");
        建到(
            cfg,
            Scope::Episode { project_id: "p1".into(), episode_id: "ep_01".into() },
            AssetKind::Scene,
            "老茶馆",
        );
        let out = suggest_alignment(cfg, "p1", "ep_01");
        assert_eq!(out[0].confidence, 100);
        assert_eq!(out[0].target_id.as_deref(), Some(准.as_str()), "取到了子串那个");
    }

    #[test]
    fn 存角色写进资产库_不再写_characters_json() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        save_cinema_character(cfg, "p1", 角色("c1", "老茶师")).unwrap();

        // 角色是**项目级**的（跨集共用），跟提取产物落本集是两回事。
        let store = crate::assets::load_store(
            cfg,
            &crate::assets::Scope::Project { project_id: "p1".into() },
        );
        assert_eq!(store.assets.len(), 1);
        assert_eq!(store.assets[0].id, "c1");
        assert!(
            !cinema_projects_dir(cfg).join("p1").join("characters.json").exists(),
            "还在写 characters.json —— 那就还是两个所有者"
        );
    }

    #[test]
    fn 角色专属字段来回一趟不丢() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        let mut c = 角色("c1", "老茶师");
        c.seed = Some(20260905);
        c.lora_model = Some("zimage/tea.safetensors".into());
        c.lora_weight = 0.66;
        c.consistency_mode = "ip_adapter_faceid".into();
        save_cinema_character(cfg, "p1", c).unwrap();

        let back = &load_cinema_characters(cfg, "p1", None).unwrap()[0];
        assert_eq!(back.seed, Some(20260905));
        assert_eq!(back.lora_model.as_deref(), Some("zimage/tea.safetensors"));
        assert!((back.lora_weight - 0.66).abs() < 1e-6);
        assert_eq!(back.consistency_mode, "ip_adapter_faceid");
        assert_eq!(back.trigger_words, "1girl");
        assert_eq!(back.role, "主角");
    }

    #[test]
    fn 改一个字段不会把别的清掉() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        save_cinema_character(cfg, "p1", 角色("c1", "老茶师")).unwrap();
        // 只改名字，别的照抄读出来的那份
        let mut c = load_cinema_characters(cfg, "p1", None).unwrap()[0].clone();
        c.name = "老茶师（雨夜）".into();
        save_cinema_character(cfg, "p1", c).unwrap();

        let back = &load_cinema_characters(cfg, "p1", None).unwrap()[0];
        assert_eq!(back.name, "老茶师（雨夜）");
        assert_eq!(back.seed, Some(42), "顺手把种子清了 —— 那张脸就再也复现不出来");
        assert_eq!(back.trigger_words, "1girl");
    }

    #[test]
    fn 存角色时不传设定卡_已有的不会被清掉() {
        // 角色界面上改个名字提交时，前端不一定把整张设定卡带回来。
        // 一并清掉的话，十分钟跑出来的 18 张图就没了 —— 而且不报错。
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        let mut c = 角色("c1", "老茶师");
        c.asset_sheet = Some(CharacterAssetSheet {
            turnarounds: vec![CharacterViewItem {
                key: "front".into(), title: "正面".into(),
                image_url: "c1/a.png".into(), description: String::new(),
                category: "turnaround".into(),
            }],
            ..Default::default()
        });
        save_cinema_character(cfg, "p1", c).unwrap();

        // 再存一次，这次不带设定卡
        let mut bare = 角色("c1", "老茶师（改名）");
        bare.asset_sheet = None;
        save_cinema_character(cfg, "p1", bare).unwrap();

        let back = &load_cinema_characters(cfg, "p1", None).unwrap()[0];
        assert_eq!(back.name, "老茶师（改名）");
        let sheet = back.asset_sheet.as_ref().expect("设定卡被清掉了");
        assert_eq!(sheet.turnarounds.len(), 1);
    }

    #[test]
    fn 老项目第一次存角色_不会把其它角色弄丢() {
        // 资产库还空着、characters.json 里有三个人的时候，
        // 存第四个人不能把前三个冲掉。
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        let legacy = vec![角色("a", "甲"), 角色("b", "乙")];
        let p = cinema_projects_dir(cfg).join("p1");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(
            p.join("characters.json"),
            serde_json::to_string(&legacy).unwrap(),
        )
        .unwrap();

        save_cinema_character(cfg, "p1", 角色("c", "丙")).unwrap();
        let all = load_cinema_characters(cfg, "p1", None).unwrap();
        let names: Vec<_> = all.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(all.len(), 3, "实际留下：{names:?}");
        for n in ["甲", "乙", "丙"] {
            assert!(names.contains(&n), "{n} 丢了");
        }
    }

    #[test]
    fn 资产库为空时回落到_characters_json() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        let p = cinema_projects_dir(cfg).join("p1");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(
            p.join("characters.json"),
            serde_json::to_string(&vec![角色("a", "甲")]).unwrap(),
        )
        .unwrap();
        assert_eq!(load_cinema_characters(cfg, "p1", None).unwrap()[0].name, "甲");
    }

    #[test]
    fn 头像是显示_url_不写回存储() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        let scope = crate::assets::Scope::Project { project_id: "p1".into() };
        let mut a = crate::assets::Asset::new(crate::assets::AssetKind::Character, "老茶师");
        a.id = "c1".into();
        crate::assets::add_variant(cfg, &scope, &mut a, b"IMG", "png", false, None, false, None).unwrap();
        let mut store = crate::assets::load_store(cfg, &scope);
        store.assets.push(a);
        crate::assets::save_store(cfg, &scope, &store).unwrap();

        let c = load_cinema_characters(cfg, "p1", None).unwrap()[0].clone();
        assert!(c.avatar_url.starts_with("anima-media:"), "头像不是显示 URL");
        // 原样存回去，存储里不该出现显示 URL
        save_cinema_character(cfg, "p1", c).unwrap();
        let raw = std::fs::read_to_string(scope.store_file(cfg)).unwrap();
        assert!(
            !raw.contains("anima-media:"),
            "显示层的 URL 被写进存储了 —— 和当初把 base64 存进 JSON 是同一类错误"
        );
    }

    fn 实体(kind: &str, name: &str) -> crate::prompts::ExtractedEntity {
        crate::prompts::ExtractedEntity {
            anchors: None,
            kind: kind.into(), name: name.into(),
            description: "描述".into(), persona: String::new(), exists: false,
            disembodied: false, incidental: false,
            spec: None,
        }
    }

    fn 系列(cfg: &std::path::Path) -> Vec<crate::assets::Asset> {
        crate::assets::load_store(cfg, &crate::assets::Scope::Project { project_id: "p1".into() }).assets
    }

    fn 本集(cfg: &std::path::Path, ep: &str) -> Vec<crate::assets::Asset> {
        crate::assets::load_store(
            cfg,
            &crate::assets::Scope::Episode { project_id: "p1".into(), episode_id: ep.into() },
        )
        .assets
    }

    #[test]
    fn 剧本编辑器项目能绑能解绑_别的字段不动() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        let f = cinema_projects_dir(cfg).join("p1");
        std::fs::create_dir_all(&f).unwrap();
        let v = serde_json::json!({
            "id": "p1", "title": "《回收师》", "genre": "", "aspect_ratio": "9:16", "description": "",
            "created_at": 0, "updated_at": 0, "episode_count": 30, "cover_image": null,
            "未建模的字段": "留着"
        });
        std::fs::write(f.join("project.json"), v.to_string()).unwrap();

        let p = link_script_project(cfg, "p1", Some("sp-9".into())).unwrap();
        assert_eq!(p.script_project_id.as_deref(), Some("sp-9"));
        let raw = std::fs::read_to_string(f.join("project.json")).unwrap();
        assert!(raw.contains("未建模的字段"), "改绑定把别的字段丢了");
        let p = link_script_project(cfg, "p1", None).unwrap();
        assert!(p.script_project_id.is_none());
        assert!(!std::fs::read_to_string(f.join("project.json")).unwrap().contains("script_project_id"));
    }

    #[test]
    fn 勾选的实体直接建进系列池() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        let n = import_entities(
            cfg, "p1", "ep_01",
            vec![实体("character", "老茶师"), 实体("scene", "老茶馆"), 实体("prop", "铜壶")],
            &[],
        )
        .unwrap();
        assert_eq!(n, ImportSummary { added: 3, merged: 0, episode_only: 0, looks: 0 });
        assert_eq!(系列(cfg).len(), 3);
        assert!(本集(cfg, "ep_01").is_empty(), "提取的东西又落回本集了 —— 第二集用不到");
        // 角色要带上 character 字段，否则角色工作台那一块整个没有
        let ch = 系列(cfg).into_iter().find(|a| a.name == "老茶师").unwrap();
        assert!(ch.character.is_some());
    }

    #[test]
    fn 第二集再提取同名角色_并进系列里那一个() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        import_entities(cfg, "p1", "ep_01", vec![实体("character", "老茶师")], &[]).unwrap();
        let mut again = 实体("character", "老茶师 ");
        again.persona = "茶馆老板".into();
        let n = import_entities(cfg, "p1", "ep_02", vec![again], &[]).unwrap();
        assert_eq!(n, ImportSummary { added: 0, merged: 1, episode_only: 0, looks: 0 });
        let s = 系列(cfg);
        assert_eq!(s.len(), 1, "第二集又长出一个同名角色");
        assert_eq!(s[0].character.as_ref().unwrap().persona, "茶馆老板", "空着的人设没补上");
    }

    #[test]
    fn 同名但不同类不算重复() {
        // 「铜壶」既可以是道具，也可以是某个场景的名字。
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        let n = import_entities(cfg, "p1", "ep_01", vec![实体("prop", "铜壶"), 实体("scene", "铜壶")], &[])
            .unwrap();
        assert_eq!(n.added, 2);
    }

    #[test]
    fn 点成只留本集的进本集并打标记() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        let n = import_entities(
            cfg, "p1", "ep_01",
            vec![实体("character", "路人甲"), 实体("character", "陆川")],
            &["character/路人甲".to_string()],
        )
        .unwrap();
        assert_eq!(n, ImportSummary { added: 1, merged: 0, episode_only: 1, looks: 0 });
        let local = 本集(cfg, "ep_01");
        assert_eq!(local.len(), 1);
        assert!(local[0].episode_only, "没打「只留本集」—— 系列页会一直提醒它待归属");
        // 再点一次不重复建
        import_entities(cfg, "p1", "ep_01", vec![实体("character", "路人甲")], &["character/路人甲".to_string()]).unwrap();
        assert_eq!(本集(cfg, "ep_01").len(), 1);
    }

    #[test]
    fn persona_跟着一起进去() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        let mut e = 实体("character", "张三");
        e.persona = "张三".into();
        import_entities(cfg, "p1", "ep_01", vec![e], &[]).unwrap();
        assert_eq!(系列(cfg)[0].character.as_ref().unwrap().persona, "张三");
    }

    #[test]
    fn 同一个人的几套造型建成一个角色加几个形象() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        let 造型 = |name: &str, desc: &str| {
            let mut e = 实体("character", name);
            e.persona = "星澜".into();
            e.description = desc.into();
            e
        };
        let n = import_entities(
            cfg, "p1", "ep_01",
            vec![造型("星澜（出厂陪伴款）", "粉白"), 造型("星澜（战斗款）", "冰蓝"), 造型("星澜（外壳破损款）", "银骨")],
            &[],
        )
        .unwrap();
        assert_eq!(n, ImportSummary { added: 1, merged: 0, episode_only: 0, looks: 2 });
        let s = 系列(cfg);
        assert_eq!(s.len(), 1, "三套造型建成了三个角色");
        assert_eq!(s[0].name, "星澜");
        assert_eq!(s[0].main_look_name, "出厂陪伴款");
        assert_eq!(s[0].description, "粉白");
        assert_eq!(s[0].looks.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["战斗款", "外壳破损款"]);
        // 再导一次：不重复加形象
        let again = import_entities(cfg, "p1", "ep_02", vec![造型("星澜（战斗款）", "冰蓝")], &[]).unwrap();
        assert_eq!(again, ImportSummary { added: 0, merged: 1, episode_only: 0, looks: 0 });
    }

    #[test]
    fn 不认识的类型直接报错_而不是默默当道具() {
        let dir = tempfile::tempdir().unwrap();
        assert!(import_entities(dir.path(), "p1", "ep_01", vec![实体("镜头", "特写")], &[]).is_err());
    }

    fn 建资产(cfg: &std::path::Path, kind: crate::assets::AssetKind, name: &str) -> String {
        let sc = crate::assets::Scope::Global;
        let mut store = crate::assets::load_store(cfg, &sc);
        let a = crate::assets::Asset::new(kind, name);
        let id = a.id.clone();
        store.assets.push(a);
        crate::assets::save_store(cfg, &sc, &store).unwrap();
        id
    }

    fn 一集(shots: Vec<StoryboardShot>) -> EpisodeData {
        EpisodeData {
            long_take_scenes: Vec::new(),
            one_take: false,
            scene_takes: Vec::new(),
            long_take_context_frames: None,
            scene_renders: Vec::new(),
            style_lock: String::new(),
            id: "ep_01".into(), episode_num: 1, title: "第一集".into(),
            synopsis: String::new(), script_text: String::new(), script_doc: None,
            target_secs: 0,
            storyboard_version: STORYBOARD_VERSION, actor_colors: Vec::new(),
            storyboard: shots, final_video_url: None,
        }
    }

    /// **没打标签的名字要补上。**
    ///
    /// 实测抽出来的一整集一个标签都没有，而**没有标签就不挂参考图** ——
    /// 长相全靠模型现编，每一镜一张不同的脸，且不报错。
    #[test]
    fn 补标签_把裸名字包起来() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        建资产(cfg, crate::assets::AssetKind::Character, "林婉清（盛唐）");
        建资产(cfg, crate::assets::AssetKind::Scene, "金銮殿");
        建资产(cfg, crate::assets::AssetKind::Prop, "龙椅");

        let got = backfill_asset_tags(cfg, "林婉清（盛唐）跪在金銮殿正中，龙椅在高台上");
        assert_eq!(got, "[角色:林婉清（盛唐）]跪在[场景:金銮殿]正中，[道具:龙椅]在高台上");
    }

    /// **已经打过的不再包一层。**
    #[test]
    fn 补标签_已有标签原样保留() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        建资产(cfg, crate::assets::AssetKind::Scene, "金銮殿");
        let src = "[场景:金銮殿] 里空无一人";
        assert_eq!(backfill_asset_tags(cfg, src), src);
    }

    /// **长名字优先。**
    ///
    /// 「林婉清（盛唐）」和「林婉清」同时在库里时，先匹配长的 ——
    /// 反过来会切成 `[角色:林婉清]（盛唐）`，那是个不存在的资产，
    /// 挂不上参考图而且不报错。
    #[test]
    fn 补标签_长名字优先不咬断() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        建资产(cfg, crate::assets::AssetKind::Character, "林婉清");
        建资产(cfg, crate::assets::AssetKind::Character, "林婉清（盛唐）");
        let got = backfill_asset_tags(cfg, "林婉清（盛唐）抬起头");
        assert_eq!(got, "[角色:林婉清（盛唐）]抬起头");
    }

    /// 库里没有的名字不动 —— 不能凭空造一个引用。
    #[test]
    fn 补标签_库里没有的不碰() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        建资产(cfg, crate::assets::AssetKind::Character, "皇帝");
        let got = backfill_asset_tags(cfg, "一个路人经过，皇帝没看他");
        assert_eq!(got, "一个路人经过，[角色:皇帝]没看他");
    }

    /// 一个资产都没有时原样返回，别把提示词搅坏。
    #[test]
    fn 补标签_空库不动提示词() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(backfill_asset_tags(dir.path(), "随便一句话"), "随便一句话");
    }

    /// 保存时把 `@` 引用回写成 id —— 否则删除保护会静默失效。
    #[test]
    fn 保存分集会把引用回写成资产_id() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        let cid = 建资产(cfg, crate::assets::AssetKind::Character, "老茶师");
        let sid = 建资产(cfg, crate::assets::AssetKind::Scene, "老茶馆");
        let pid = 建资产(cfg, crate::assets::AssetKind::Prop, "铜壶");

        let mut s = 镜("[角色:老茶师] 在 [场景:老茶馆] 擦 [道具:铜壶]", "");
        s.prompt = "[角色:老茶师] 在 [场景:老茶馆] 擦 [道具:铜壶]".into();
        let mut ep = 一集(vec![s]);
        sync_shot_asset_ids(cfg, &mut ep, &crate::assets::RefScope::Anywhere);

        let shot = &ep.storyboard[0];
        assert_eq!(shot.character_ids, vec![cid]);
        assert_eq!(shot.scene_id.as_deref(), Some(sid.as_str()));
        assert_eq!(shot.prop_ids, vec![pid]);
    }

    #[test]
    fn 对不上库的引用不写_id_也不清掉已有的() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        let mut s = 镜("x", "");
        s.prompt = "[角色:查无此人] 抬头".into();
        // 手动绑过的关系不该被一次改词冲掉。
        s.character_ids = vec!["手动绑的".into()];
        let mut ep = 一集(vec![s]);
        sync_shot_asset_ids(cfg, &mut ep, &crate::assets::RefScope::Anywhere);
        assert_eq!(ep.storyboard[0].character_ids, vec!["手动绑的".to_string()]);
    }

    #[test]
    fn 重复保存不会把同一个_id_写两遍() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        let cid = 建资产(cfg, crate::assets::AssetKind::Character, "老茶师");
        let mut s = 镜("x", "");
        s.prompt = "[角色:老茶师] 抬头".into();
        let mut ep = 一集(vec![s]);
        sync_shot_asset_ids(cfg, &mut ep, &crate::assets::RefScope::Anywhere);
        sync_shot_asset_ids(cfg, &mut ep, &crate::assets::RefScope::Anywhere);
        assert_eq!(ep.storyboard[0].character_ids, vec![cid]);
    }

    /// 迁移之后角色头像是 `anima-media://` URL，锁脸参考图必须还读得到。
    ///
    /// 不认这个 scheme 的话 `resolve_portrait_bytes` 返回 None，
    /// 关键帧就没有参考图了 —— **出图不报错，只是人不像了**。
    #[test]
    fn 锁脸参考图认得出媒体协议的_url() {
        let dir = tempfile::tempdir().unwrap();
        let img = dir.path().join("face.png");
        std::fs::write(&img, b"FACEBYTES").unwrap();
        let url = crate::assets::media_url(&img);
        assert_eq!(
            resolve_portrait_bytes(&url, dir.path()).as_deref(),
            Some(&b"FACEBYTES"[..]),
            "认不出 anima-media:// —— 参考图会静默变成 None"
        );
    }

    #[test]
    fn 参考图取不到时返回_none_而不是_panic() {
        let dir = tempfile::tempdir().unwrap();
        let url = crate::assets::media_url(&dir.path().join("不存在.png"));
        assert!(resolve_portrait_bytes(&url, dir.path()).is_none());
    }








    /// **默认必须是关闭的。**
    ///
    /// 转场、切场景的地方硬接上一镜末帧，会把本该切开的两个镜头糊在一起 ——
    /// 比不接更糟。所以链式衔接只能显式打开，不能默认生效。
    #[test]
    fn 衔接默认关闭() {
        let s = shot("s1", 1, false);
        assert!(!s.continues_previous);
        // 老的 episode.json 里没有这个字段，反序列化后也必须是 false
        let json = r#"{"id":"s1","shot_num":1,"scale":"中景","camera_movement":"固定",
            "dialogue":"","character_id":"","prompt":"","duration":2.0,"status":"pending"}"#;
        let parsed: StoryboardShot = serde_json::from_str(json).expect("老数据要能读");
        assert!(!parsed.continues_previous, "老 episode.json 不该突然打开衔接");
    }

    /// 找上一镜要按 `shot_num` 取**最大的、小于当前**的那个，
    /// 不能按数组顺序 —— 分镜可以被重排、删除，数组顺序不等于镜头顺序。
    #[test]
    fn 上一镜按镜号取而不是按数组顺序() {
        let sb = [shot("c", 3, true), shot("a", 1, false), shot("b", 2, false)];
        let cur = 3u32;
        let prev = sb.iter().filter(|s| s.shot_num < cur).max_by_key(|s| s.shot_num);
        assert_eq!(prev.map(|s| s.id.as_str()), Some("b"), "应该取第 2 镜，不是数组里的前一个");
    }

    // ── 合成提示词 ────────────────────────────────────────────────

    fn 合成用的集(dir: &Path, route: &str) {
        let proj = cinema_projects_dir(dir).join("p1");
        std::fs::create_dir_all(proj.join("episodes").join("e1")).unwrap();
        std::fs::write(proj.join("project.json"), json!({
            "id":"p1","title":"t","genre":"","aspect_ratio":"9:16","description":"",
            "model_settings": {"prompt_route": route}
        }).to_string()).unwrap();
        let shot = |id: &str, num: u32, line: &str| json!({
            "id": id, "shot_num": num, "scale": "medium", "camera_movement": "static", "dialogue": "",
            "prompt": "", "duration": 8.0, "status": "pending", "lighting": "冷光",
            "segments": [{"start": 0.0, "end": 8.0, "title": format!("第{num}镜"), "visual": format!("画面{num}")}],
            "dialogue_lines": if line.is_empty() { json!([]) } else { json!([{"speaker": "旁白", "line": line, "off_screen": true}]) }
        });
        std::fs::write(proj.join("episodes").join("e1").join("episode.json"), json!({
            "id":"e1","episode_num":1,"title":"t","synopsis":"","script_text":"x","storyboard_version":3,
            // 数组顺序故意和镜号相反
            "storyboard": [shot("s2", 2, ""), shot("s1", 1, "这茶，要等。")]
        }).to_string()).unwrap();
    }

    fn 读集(dir: &Path) -> EpisodeData {
        serde_json::from_str(&std::fs::read_to_string(cinema_projects_dir(dir).join("p1/episodes/e1/episode.json")).unwrap()).unwrap()
    }

    #[tokio::test]
    async fn 线路_a_整集合成_落盘_不碰线路_b() {
        use crate::shot_table::{PromptRoute, RoutePrompts};
        let d = tempfile::tempdir().unwrap();
        合成用的集(d.path(), "a");
        // 先放一份 B 的结果，合成 A 不能覆盖它
        let mut ep = 读集(d.path());
        ep.storyboard[0].prompts.insert(PromptRoute::B, RoutePrompts { video: "B 的正文".into(), ..Default::default() });
        std::fs::write(cinema_projects_dir(d.path()).join("p1/episodes/e1/episode.json"), serde_json::to_string(&ep).unwrap()).unwrap();

        let never = |_: crate::prompts::PromptSlot, _: Vec<(String, String)>| async { Err::<String, _>(Error::runtime("线路 A 不该调模型")) };
        let mut seen = Vec::new();
        let r = synthesize_episode_prompts(d.path(), "p1", "e1", project_route(d.path(), "p1"), &[], never, |done, total, label| seen.push((done, total, label.to_string()))).await.unwrap();
        assert_eq!((r.route, r.done, r.total), (PromptRoute::A, 2, 2), "{:?}", r.notes);
        assert_eq!(seen.last().map(|(d, t, _)| (*d, *t)), Some((2, 2)));
        let ep = 读集(d.path());
        let s1 = ep.storyboard.iter().find(|s| s.id == "s1").unwrap();
        let s2 = ep.storyboard.iter().find(|s| s.id == "s2").unwrap();
        assert!(s1.prompts[&PromptRoute::A].video.contains("旁白（画外音）：“这茶，要等。”"));
        assert!(s2.prompts[&PromptRoute::A].video.contains("前一个分镜描述：\n画面1"), "上一镜要按镜号取：{}", s2.prompts[&PromptRoute::A].video);
        assert_eq!(ep.storyboard[0].prompts[&PromptRoute::B].video, "B 的正文", "合成 A 覆盖了 B");
        // 合成完不算过期；改了字段就报
        assert!(stale_prompt_issues(&ep.storyboard, &[], "", PromptRoute::A, &[]).is_empty());
        let mut changed = ep.storyboard.clone();
        changed[1].lighting = "暖光".into();
        let stale = stale_prompt_issues(&changed, &[], "", PromptRoute::A, &[]);
        assert_eq!(stale.iter().map(|i| (i.shot_num, i.field)).collect::<Vec<_>>(), [(1, "prompts")]);
        assert!(stale_prompt_issues(&changed, &[], "", PromptRoute::B, &[]).is_empty(), "B 那份没有指纹，不乱报");
        // 只出了首帧：提示要说是首帧的事，不能是笼统的「字段改过」（2026-09-29 用户截图）
        let mut framed = ep.storyboard.clone();
        framed[0].keyframes = Keyframes { first: true, last: false };
        let msg = &stale_prompt_issues(&framed, &[], "", PromptRoute::A, &[])[0].message;
        assert!(msg.contains("有了首帧") && msg.contains("重新合成这一镜"), "{msg}");
        // 首帧 + 字段都改了：照旧笼统说，不能把字段的改动说成只是首帧
        framed[0].lighting = "测试里新写的一种光".into();
        let msg = &stale_prompt_issues(&framed, &[], "", PromptRoute::A, &[])[0].message;
        assert!(msg.contains("字段") && !msg.contains("有了首帧"), "{msg}");
    }

    /// **加入合成队列**（2026-10-06 用户：「重新合成这一镜……也要做到可以添加到合成提示词列队里，方式方法跟出片一致」）：
    /// 在合的那一轮每合一镜前看一眼名单，追加的接在后面合；追加的镜头**轮到时重新读分集**（用户多半是改完才点的）。
    #[tokio::test]
    async fn 合成中追加的镜头接着合_按轮到时的版本合() {
        use crate::shot_table::PromptRoute;
        let d = tempfile::tempdir().unwrap();
        合成用的集(d.path(), "a");
        let file = cinema_projects_dir(d.path()).join("p1/episodes/e1/episode.json");
        let never = |_: crate::prompts::PromptSlot, _: Vec<(String, String)>| async { Err::<String, _>(Error::runtime("线路 A 不该调模型")) };
        let calls = std::cell::Cell::new(0);
        let more = || {
            calls.set(calls.get() + 1);
            if calls.get() == 1 {
                return vec!["s1".to_string()];
            }
            // 合第 1 镜期间用户改了第 2 镜、点了「加入合成队列」
            if calls.get() == 2 {
                let mut ep = 读集(d.path());
                ep.storyboard.iter_mut().find(|s| s.id == "s2").unwrap().lighting = "冷蓝顶光".into();
                std::fs::write(&file, serde_json::to_string(&ep).unwrap()).unwrap();
            }
            vec!["s1".to_string(), "s2".to_string()]
        };
        let mut seen = Vec::new();
        let r = synthesize_episode_prompts_queued(d.path(), "p1", "e1", PromptRoute::A, &["s1".to_string()], never, |done, total, label| seen.push((done, total, label.to_string())), more)
            .await
            .unwrap();
        assert_eq!((r.done, r.total), (2, 2), "{:?}", r.notes);
        assert_eq!(seen[1], (1, 2, "第 2 镜".to_string()), "追加后总数跟着变：{seen:?}");
        let ep = 读集(d.path());
        let s2 = ep.storyboard.iter().find(|s| s.id == "s2").unwrap();
        assert!(s2.prompts[&PromptRoute::A].video.contains("冷蓝顶光"), "追加的镜头要按轮到时的版本合：{}", s2.prompts[&PromptRoute::A].video);
    }

    /// 加入合成队列：这一集在合同一条线路 → 追加进那一轮的名单；在合另一条线路 → 报清楚，不混进去
    #[test]
    fn 加入合成队列_同线路追加_别的线路不混() {
        use crate::shot_table::PromptRoute;
        let d = tempfile::tempdir().unwrap();
        合成用的集(d.path(), "b");
        let gateway = std::sync::Arc::new(crate::llm::gateway::Gateway::new(crate::config::AppConfig::default()).unwrap());
        let id = crate::jobs::start_exclusive(d.path(), crate::jobs::JobKind::Prompts, "p1", "e1", "", 0, &format!("{} · 整集", PromptRoute::B.label())).unwrap();
        crate::jobs::set_targets(d.path(), &id, &["s1".to_string(), "s2".to_string()]);
        crate::jobs::set_progress(d.path(), &id, 1, 2, "第 2 镜");
        let job = queue_synthesize(None, d.path().to_path_buf(), gateway.clone(), "p1".into(), "e1".into(), vec!["s1".into()], Some(PromptRoute::B)).unwrap();
        assert_eq!(job.id, id, "追加进在跑的那一轮，不另开");
        assert_eq!(job.targets, vec!["s1".to_string(), "s2".to_string(), "s1".to_string()], "合过的那一镜可以再排一次");
        let err = queue_synthesize(None, d.path().to_path_buf(), gateway, "p1".into(), "e1".into(), vec!["s2".into()], Some(PromptRoute::A)).unwrap_err();
        assert!(err.to_string().contains("另一条线路"), "{err}");
    }

    /// **超长镜（2026-10-05 EP03 第 20 镜）**：22 秒一镜，合成时拆成 2 段各合成一份，存回这一镜时用分隔行隔开；
    /// 指纹按这一镜本身算（不算过期）；接上一镜、整段出的场里都拦下。
    #[tokio::test]
    async fn 超长镜_拆段合成_拼回一镜_不过期() {
        use crate::shot_table::PromptRoute;
        let d = tempfile::tempdir().unwrap();
        合成用的集(d.path(), "a");
        let file = cinema_projects_dir(d.path()).join("p1/episodes/e1/episode.json");
        let mut ep = 读集(d.path());
        {
            let s1 = ep.storyboard.iter_mut().find(|s| s.id == "s1").unwrap();
            s1.duration = 22.0;
            s1.segments = vec![
                crate::shot_table::ShotSegment { start: 0.0, end: 8.0, title: "追问".into(), visual: "约0.5秒陆川带着笑意开口，随后抬眼看星澜。".into(), ..Default::default() },
                crate::shot_table::ShotSegment { start: 8.0, end: 22.0, title: "回答".into(), visual: "约11.1秒星澜抬眼开口，承认不知道。".into(), ..Default::default() },
            ];
            s1.dialogue_lines = vec![
                DialogueLine { speaker: "陆川".into(), line: "刚才那句也是出厂话术？".into(), off_screen: true, ..Default::default() },
                DialogueLine { speaker: "星澜".into(), line: "本机也不知道。本机只知道，她让主人不开心。".into(), off_screen: true, ..Default::default() },
            ];
        }
        std::fs::write(&file, serde_json::to_string(&ep).unwrap()).unwrap();
        let never = |_: crate::prompts::PromptSlot, _: Vec<(String, String)>| async { Err::<String, _>(Error::runtime("线路 A 不该调模型")) };
        let r = synthesize_episode_prompts(d.path(), "p1", "e1", PromptRoute::A, &["s1".to_string()], never, |_, _, _| {}).await.unwrap();
        assert_eq!(r.done, 1, "{:?}", r.notes);
        let ep = 读集(d.path());
        let s1 = ep.storyboard.iter().find(|s| s.id == "s1").unwrap();
        let video = &s1.prompts[&PromptRoute::A].video;
        let parts = crate::long_shot::split_prompt(video);
        assert_eq!(parts.len(), crate::long_shot::plan(22.0).0, "{video}");
        assert!(parts[0].contains("刚才那句也是出厂话术") && !parts[0].contains("她让主人不开心"), "第一段只带第一段的台词：{}", parts[0]);
        assert!(parts[1].contains("她让主人不开心") && !parts[1].contains("刚才那句也是出厂话术"), "第二段只带第二段的台词：{}", parts[1]);
        assert!(stale_prompt_issues(&ep.storyboard, &[], "", PromptRoute::A, &[]).is_empty(), "按这一镜本身算指纹，合成完不算过期");

        // 接上一镜：拦下，说清楚为什么
        let mut ep = 读集(d.path());
        ep.storyboard.iter_mut().find(|s| s.id == "s1").unwrap().continuity = Continuity::RefVideo;
        std::fs::write(&file, serde_json::to_string(&ep).unwrap()).unwrap();
        let r = synthesize_episode_prompts(d.path(), "p1", "e1", PromptRoute::A, &["s1".to_string()], never, |_, _, _| {}).await.unwrap();
        assert_eq!(r.done, 0);
        assert!(r.notes.iter().any(|n| n.contains("超长镜") && n.contains("不接")), "{:?}", r.notes);
    }

    /// **超长镜的「说话人串了」按段查**（2026-10-05 EP03 第 20 镜误报三条）：各段的 (S1)(S2) 从头编，
    /// 第 2 段第一个开口的星澜就是 (S1)。真串了照样报，并说清是第几段。
    #[test]
    fn 超长镜按段查说话人() {
        let mut s: StoryboardShot = serde_json::from_value(json!({
            "id": "s20", "shot_num": 20, "scale": "medium", "camera_movement": "static", "dialogue": "",
            "prompt": "", "duration": 20.5, "status": "pending"
        })).unwrap();
        s.segments = vec![
            crate::shot_table::ShotSegment { start: 0.0, end: 7.5, title: "问".into(), visual: "约0.5秒陆川带着笑意开口。".into(), ..Default::default() },
            crate::shot_table::ShotSegment { start: 7.5, end: 11.5, title: "答".into(), visual: "约7.6秒星澜平稳开口。".into(), ..Default::default() },
            crate::shot_table::ShotSegment { start: 11.5, end: 16.0, title: "追问".into(), visual: "约12.1秒陆川直视星澜追问。".into(), ..Default::default() },
            crate::shot_table::ShotSegment { start: 16.0, end: 20.5, title: "确认".into(), visual: "约16.1秒星澜抬眼开口。".into(), ..Default::default() },
        ];
        let line = |who: &str, t: &str| DialogueLine { speaker: who.into(), line: t.into(), ..Default::default() };
        s.dialogue_lines = vec![
            line("陆川", "刚才那句「主人不收二手的东西」，也是出厂话术？"),
            line("星澜", "数据库里没有这句。"),
            line("陆川", "那是哪儿来的？"),
            line("星澜", "本机也不知道。本机只知道，她让主人不开心。"),
        ];
        let wins = crate::long_shot::windows(&s);
        assert_eq!(wins.len(), 2);
        assert_eq!(wins[1].dialogue_lines.first().map(|l| l.speaker.as_str()), Some("星澜"), "这条测试要第 2 段由星澜开头");
        let says = |sn: u32, t: &str| format!("(S{sn}) says, <d>[Chinese] {t}</d> ");
        let part1 = says(1, "刚才那句「主人不收二手的东西」，也是出厂话术？");
        let ok2 = format!("{}{}{}", says(1, "数据库里没有这句。"), says(2, "那是哪儿来的？"), says(1, "本机也不知道。本机只知道，她让主人不开心。"));
        let p = crate::shot_table::RoutePrompts { video: crate::long_shot::join_prompts(&[part1.clone(), ok2]), ..Default::default() };
        let adv = long_shot_advisories(&s, None, &[], "", &p);
        assert!(!adv.iter().any(|m| m.contains("串了")), "{adv:?}");
        // 拼起来按一整镜查就会误报 —— 这正是改之前的样子
        let whole = crate::prompt_input::ShotContext { shot: &s, prev: None, assets: &[], style: "", long_take: false };
        assert!(crate::prompt_route_b::advisories(&whole, &p).iter().any(|m| m.contains("串了")), "前提不成立：整镜查本来就该误报");
        // 第 2 段里真串了：照样报，带段号
        let bad2 = format!("{}{}{}", says(2, "数据库里没有这句。"), says(1, "那是哪儿来的？"), says(1, "本机也不知道。本机只知道，她让主人不开心。"));
        let bad = crate::shot_table::RoutePrompts { video: crate::long_shot::join_prompts(&[part1, bad2]), ..Default::default() };
        let adv = long_shot_advisories(&s, None, &[], "", &bad);
        assert!(adv.iter().any(|m| m.starts_with("超长镜第 2/2 段：") && m.contains("串了")), "{adv:?}");
        // 镜头表校验真的走的是按段查（接线改坏了，界面上就又是三条误报）
        let src = include_str!("cinema.rs");
        let at = src.find("pub fn validate_storyboard(").unwrap();
        let body = &src[at..at + src[at..].find("\n}\n").unwrap()];
        assert!(body.contains("if crate::long_shot::is_long(shot.duration) {\n                long_shot_advisories(shot, prev, assets, &style, p)"), "镜头表校验没按段查超长镜");
    }

    /// 超长镜出片走窗口链（一次提交、H3 自己接），参考图按长镜头的槽位挂（不吃首帧），每段都过图号检查。
    /// 出片要连着 ComfyUI 跑不了单测，这里钉住接线（改坏了这几行，超长镜会按普通一镜发出去 —— H3 只出前 15 秒，不报错）
    #[test]
    fn 超长镜出片走窗口链() {
        let src = include_str!("cinema.rs");
        let at = src.find("async fn render_shot_video_body").unwrap();
        let body = &src[at..at + src[at..].find("shot_vid_path = take_path.clone();").unwrap()];
        let branch = &body[body.find("if long_shot {").expect("超长镜分支不见了")..body.find("} else if engine == \"h3\" && keyframe_video {").unwrap()];
        for needle in ["crate::long_shot::split_prompt(&prompt)", "render_reference_paths(config_dir, &req.project_id, &req.episode_id, &shots_dir, shot, &assets, true)", "render_refs_guard(shot.shot_num, part,", "render_long_take("] {
            assert!(branch.contains(needle), "超长镜分支少了 {needle}");
        }
        assert!(body.contains("let long_shot = engine == \"h3\" && crate::long_shot::is_long(shot.duration);"));
    }

    /// **两条线路各报各的过期，不管哪条是当前线路。** 用户 2026-09-29：「合成其中一个，点另一个就变成不需要了」——
    /// 问题清单只报当前线路，合成弹窗切到另一条线路时看不见它过期了
    #[tokio::test]
    async fn 两条线路各报各的过期_不看当前线路() {
        use crate::shot_table::{PromptRoute, RoutePrompts};
        let d = tempfile::tempdir().unwrap();
        合成用的集(d.path(), "a");
        let never = |_: crate::prompts::PromptSlot, _: Vec<(String, String)>| async { Err::<String, _>(Error::runtime("线路 A 不该调模型")) };
        synthesize_episode_prompts(d.path(), "p1", "e1", PromptRoute::A, &[], never, |_, _, _| {}).await.unwrap();
        let mut ep = 读集(d.path());
        // B 也合成过，但用的是改之前的字段（指纹对不上）；当前线路是 A
        let a1 = ep.storyboard[1].prompts[&PromptRoute::A].clone();
        ep.storyboard[1].prompts.insert(PromptRoute::B, RoutePrompts { video: "B 的正文".into(), fields_hash: "改之前的指纹".into(), ..a1 });
        let s = stale_prompts(d.path(), "p1", "e1", &ep);
        assert_eq!((s.a.len(), s.b), (0, vec![ep.storyboard[1].id.clone()]), "A 刚合成完不过期；B 过期要报出来");
        // 问题清单只看当前线路（A）：它报不出 B 过期 —— 这正是弹窗不能只靠它的原因
        assert!(validate_storyboard(d.path(), "p1", "e1", &ep).iter().all(|i| i.field != "prompts"));
    }

    /// 出完首帧自动补合成：出片线路上**因为有了首帧而过期**的，和**出了首帧、还没合成过**的。
    /// 用户 2026-09-29：「生成了提示词，然后按场出首帧，然后提示词又还再生成」；
    /// 2026-10-03 起出首帧不用先合成（`first_frame_source`），出完首帧这一次补合就是唯一一次合成。
    #[tokio::test]
    async fn 出完首帧补合成过期的和还没合成的() {
        use crate::shot_table::PromptRoute;
        let d = tempfile::tempdir().unwrap();
        合成用的集(d.path(), "a");
        let never = |_: crate::prompts::PromptSlot, _: Vec<(String, String)>| async { Err::<String, _>(Error::runtime("线路 A 不该调模型")) };
        // 只合成第一镜（线路 A 是出片线路）
        let ep = 读集(d.path());
        let first = ep.storyboard.iter().min_by_key(|s| s.shot_num).unwrap().id.clone();
        let second = ep.storyboard.iter().max_by_key(|s| s.shot_num).unwrap().id.clone();
        synthesize_episode_prompts(d.path(), "p1", "e1", PromptRoute::A, std::slice::from_ref(&first), never, |_, _, _| {}).await.unwrap();
        let shots = cinema_projects_dir(d.path()).join("p1/episodes/e1/shots");
        std::fs::create_dir_all(&shots).unwrap();
        let both = vec![first.clone(), second.clone()];
        // 还没有首帧：什么都不补
        assert!(frame_resynth_targets(d.path(), "p1", "e1", &both).is_empty());
        // 两镜都有了首帧：合成过的那一镜过期了要补；没合成过的那一镜也要合 —— 这是它唯一的一次合成
        for id in &both {
            image::RgbImage::from_pixel(16, 9, image::Rgb([1, 2, 3])).save(shots.join(format!("{id}.png"))).unwrap();
        }
        let mut got = frame_resynth_targets(d.path(), "p1", "e1", &both);
        got.sort();
        let mut want = both.clone();
        want.sort();
        assert_eq!(got, want, "出了首帧的两镜都要合");
        // 不在这一轮出首帧名单里的不补：名单里只有第二镜，第一镜就算过期也不碰
        assert_eq!(frame_resynth_targets(d.path(), "p1", "e1", std::slice::from_ref(&second)), vec![second.clone()]);
    }

    /// **「有 take」不等于「按当前版本出过片」。**
    ///
    /// 提示词是新的、片子是旧的 —— 用户重新合成了提示词却没重出片，
    /// 那一镜照样有视频、照样显示「已完成」。这两件事原来混在一起，
    /// 没法分别标（2026-09-19 Codex Astra 指出）。
    #[test]
    fn 片子过期和提示词过期分开报() {
        use crate::shot_table::PromptRoute;
        let 镜 = |h: &str| -> StoryboardShot {
            let mut s: StoryboardShot = serde_json::from_value(serde_json::json!({
                "id": "s1", "shot_num": 1, "scale": "", "camera_movement": "",
                "dialogue": "", "prompt": "", "duration": 8.0, "status": "ready"
            })).unwrap();
            s.takes = vec![VideoTake {
                route: PromptRoute::A,
                path: "/tmp/a.mp4".into(),
                created_at: 0,
                seed: 1,
                pictures: 0,
                picture_paths: Vec::new(),
                quality: String::new(),
                prompt: "当时发的正文".into(),
                fields_hash: h.into(),
                style: String::new(),
                origin: TakeOrigin::Render,
            }];
            s
        };
        let now = |shots: &[StoryboardShot]| current_hash(shots, &shots[0], &[], "", &[]);

        // 指纹和当前一致 —— 不报
        let mut one = vec![镜("x")];
        let h = now(&one);
        one[0].takes[0].fields_hash = h;
        assert!(stale_take_issues(&one, &[], "", PromptRoute::A, &[]).is_empty());

        // 字段改了 —— 要报
        one[0].lighting = "暖光".into();
        let out = stale_take_issues(&one, &[], "", PromptRoute::A, &[]);
        assert_eq!(out.iter().map(|i| i.field).collect::<Vec<_>>(), ["takes"], "{out:?}");
        assert!(out[0].message.contains("旧版"), "{:?}", out[0]);

        // **老 take 没记指纹 —— 不知道是按什么出的，不乱报。**
        // 一次性把所有历史 take 标成过期，用户看到的是一屏警告，结果是整片忽略。
        let old = vec![镜("")];
        assert!(stale_take_issues(&old, &[], "", PromptRoute::A, &[]).is_empty(), "老数据不该被报");

        // 另一条线路的 take 不算
        assert!(stale_take_issues(&one, &[], "", PromptRoute::B, &[]).is_empty());
    }

    #[tokio::test]
    async fn 线路_b_只合成指定镜头_失败的写进备注不存() {
        use crate::shot_table::PromptRoute;
        let d = tempfile::tempdir().unwrap();
        合成用的集(d.path(), "b");
        let bad = |_: crate::prompts::PromptSlot, _: Vec<(String, String)>| async { Ok::<String, Error>(r#"{"detailed_description":"too short"}"#.into()) };
        let r = synthesize_episode_prompts(d.path(), "p1", "e1", PromptRoute::B, &["s1".to_string()], bad, |_, _, _| {}).await.unwrap();
        assert_eq!((r.done, r.total), (0, 1));
        assert!(r.notes[0].contains("第 1 镜"), "{:?}", r.notes);
        let ep = 读集(d.path());
        assert!(ep.storyboard.iter().all(|s| !s.prompts.contains_key(&PromptRoute::B)), "不合格的被存下来了");
    }

    // ── 批量出片 ────────────────────────────────────────────────

    /// 重启后续接收尾：ComfyUI 那边出完的片落到原定位置后，记一条 take、指向它；重复收尾不重复记
    #[test]
    fn 续接收尾记一条_take() {
        use crate::shot_table::PromptRoute;
        let d = tempfile::tempdir().unwrap();
        合成用的集(d.path(), "b");
        let shots = cinema_projects_dir(d.path()).join("p1/episodes/e1/shots");
        std::fs::create_dir_all(&shots).unwrap();
        let out = shots.join("s1-b-123.mp4");
        let take = VideoTake { route: PromptRoute::B, path: out.to_string_lossy().to_string(), seed: 7, quality: "双采 · 1312×736".into(), prompt: "B 正文".into(), ..Default::default() };
        assert!(record_resumed_take(d.path(), "p1", "e1", "s1", take.clone(), false).is_err(), "文件不在不能记");
        std::fs::write(&out, b"mp4").unwrap();
        assert!(record_resumed_take(d.path(), "p1", "e1", "s1", take.clone(), false).unwrap());
        assert!(record_resumed_take(d.path(), "p1", "e1", "s1", take.clone(), false).unwrap());
        let s = 读集(d.path()).storyboard.into_iter().find(|s| s.id == "s1").unwrap();
        assert_eq!(s.takes.len(), 1, "重复收尾记了两条");
        assert_eq!((s.takes[0].seed, s.takes[0].origin, s.video_url.as_deref()), (7, TakeOrigin::Render, Some(out.to_str().unwrap())));
        assert!(s.takes[0].created_at > 0 && s.status == "ready");
        assert!(!record_resumed_take(d.path(), "p1", "e1", "不在了", take, false).unwrap(), "镜头被删了不报错，回 false");
    }

    /// 加入队列：批量跑着的时候追加进来的镜头，接在后面出（`more` 每一镜前读一次名单），总数跟着变
    #[tokio::test]
    async fn 批量出片_排队加进来的接着出() {
        use crate::shot_table::{PromptRoute, RoutePrompts};
        let d = tempfile::tempdir().unwrap();
        合成用的集(d.path(), "b");
        let mut ep = 读集(d.path());
        let snapshot = ep.storyboard.clone();
        for s in ep.storyboard.iter_mut() {
            let h = current_hash(&snapshot, s, &[], "", &[]);
            s.prompts.insert(PromptRoute::B, RoutePrompts { video: "B 正文".into(), frame: "f".into(), synthesized_at: 1, fields_hash: h });
        }
        std::fs::write(cinema_projects_dir(d.path()).join("p1/episodes/e1/episode.json"), serde_json::to_string(&ep).unwrap()).unwrap();
        let called = std::sync::Mutex::new(Vec::<String>::new());
        let render = |req: RenderShotReq| {
            called.lock().unwrap().push(req.shot_id.clone());
            async move { Ok(RenderShotResult { shot_id: req.shot_id, image_url: String::new(), video_url: "v.mp4".into(), audio_url: String::new(), status: "ready".into() }) }
        };
        // 出第一镜的时候，用户在别的镜头上点了「加入队列」
        let more = || if called.lock().unwrap().is_empty() { vec!["s1".to_string()] } else { vec!["s1".to_string(), "s2".into()] };
        let mut seen = Vec::new();
        let r = render_batch(d.path(), "p1", "e1", &["s1".to_string()], render, |_: u32| async { Err::<String, Error>(Error::runtime("不该调")) }, |done, total, label| seen.push((done, total, label.to_string())), || false, more).await.unwrap();
        assert_eq!(*called.lock().unwrap(), ["s1", "s2"]);
        assert_eq!((r.done, r.total), (2, 2));
        assert_eq!(seen.last().map(|x| (x.0, x.1)), Some((2, 2)), "{seen:?}");
        // 入口：单镜出片在批量跑时被拒，前端改走「加入队列」
        let src = include_str!("cinema.rs");
        let prod = &src[..src.find("#[cfg(test)]").unwrap()];
        assert!(prod.contains("|| crate::jobs::get(&config_dir, &job_id).map(|j| j.targets).unwrap_or_default(),"), "批量没把任务名单接给 more");
    }

    /// 队列调序 + 暂停位（2026-10-10 用户：「14/23 个任务的列队，但是完全没办法调整顺序」）：
    /// 没轮到的那段被重排了照新顺序出；走到暂停位停下等，撤掉之后接着出它后面那一项
    #[tokio::test]
    async fn 批量出片_重排和暂停位() {
        use crate::shot_table::{PromptRoute, RoutePrompts};
        let d = tempfile::tempdir().unwrap();
        合成用的集(d.path(), "b");
        let mut ep = 读集(d.path());
        let snapshot = ep.storyboard.clone();
        for s in ep.storyboard.iter_mut() {
            let h = current_hash(&snapshot, s, &[], "", &[]);
            s.prompts.insert(PromptRoute::B, RoutePrompts { video: "B 正文".into(), frame: "f".into(), synthesized_at: 1, fields_hash: h });
        }
        std::fs::write(cinema_projects_dir(d.path()).join("p1/episodes/e1/episode.json"), serde_json::to_string(&ep).unwrap()).unwrap();
        let called = std::sync::Mutex::new(Vec::<String>::new());
        let render = |req: RenderShotReq| {
            called.lock().unwrap().push(req.shot_id.clone());
            async move { Ok(RenderShotResult { shot_id: req.shot_id, image_url: String::new(), video_url: "v.mp4".into(), audio_url: String::new(), status: "ready".into() }) }
        };
        // 开始前用户把 s2 挪到最前、后面插了一个暂停位；暂停位等了几轮之后被撤掉
        let polls = std::sync::Mutex::new(0);
        let more = || {
            let mut n = polls.lock().unwrap();
            *n += 1;
            let v: &[&str] = if *n < 5 { &["s2", "hold:冒烟测试", "s1"] } else { &["s2", "s1"] };
            v.iter().map(|s| s.to_string()).collect::<Vec<String>>()
        };
        let mut seen = Vec::new();
        let r = render_batch(d.path(), "p1", "e1", &["s1".to_string(), "s2".into()], render, |_: u32| async { Err::<String, Error>(Error::runtime("不该调")) }, |done, total, label| seen.push((done, total, label.to_string())), || false, more).await.unwrap();
        assert_eq!(*called.lock().unwrap(), ["s2", "s1"], "没按新顺序出");
        assert!(seen.iter().any(|x| x.2 == "暂停 · 冒烟测试"), "{seen:?}");
        assert_eq!((r.done, r.total), (2, 2), "{:?}", r.notes);
        assert!(*polls.lock().unwrap() >= 5, "没在暂停位上等");
    }

    /// 用户要求：点一下去睡觉，某一镜失败不阻断。没合成的跳过并说明，失败的记下接着出。
    #[tokio::test]
    async fn 批量出片_失败不阻断_没合成的跳过_删掉的跳过() {
        use crate::shot_table::{PromptRoute, RoutePrompts};
        let d = tempfile::tempdir().unwrap();
        合成用的集(d.path(), "b");
        // s2 合成了 B，s1 没合成；再加一个会失败的 s3
        let mut ep = 读集(d.path());
        let mut s3 = ep.storyboard[0].clone();
        s3.id = "s3".into();
        s3.shot_num = 3;
        ep.storyboard.push(s3);
        // 指纹要在镜头表定下来之后算（上一镜也算进去）
        let snapshot = ep.storyboard.clone();
        let synth = |s: &StoryboardShot| RoutePrompts { video: "B 正文".into(), frame: "f".into(), synthesized_at: 1, fields_hash: current_hash(&snapshot, s, &[], "", &[]) };
        for i in [0, 2] {
            let p = synth(&snapshot[i]);
            ep.storyboard[i].prompts.insert(PromptRoute::B, p);
        }
        std::fs::write(cinema_projects_dir(d.path()).join("p1/episodes/e1/episode.json"), serde_json::to_string(&ep).unwrap()).unwrap();

        let called = std::sync::Mutex::new(Vec::<String>::new());
        let render = |req: RenderShotReq| {
            called.lock().unwrap().push(req.shot_id.clone());
            async move {
                if req.shot_id == "s3" {
                    Err(Error::runtime("ComfyUI 没有响应"))
                } else {
                    Ok(RenderShotResult { shot_id: req.shot_id, image_url: String::new(), video_url: "v.mp4".into(), audio_url: String::new(), status: "ready".into() })
                }
            }
        };
        let ids: Vec<String> = ["s3", "s1", "gone", "s2"].iter().map(|s| s.to_string()).collect();
        let mut seen = Vec::new();
        let r = render_batch(d.path(), "p1", "e1", &ids, render, |_: u32| async { Err::<String, Error>(Error::runtime("不该调")) }, |done, total, label| seen.push((done, total, label.to_string())), || false, Vec::new).await.unwrap();
        assert_eq!(*called.lock().unwrap(), ["s3", "s2"], "s3 失败之后 s2 照样要出；没合成的 s1 不该调出片");
        assert_eq!((r.done, r.total), (1, 4));
        assert!(r.notes.iter().any(|n| n.contains("第 3 镜出片失败") && n.contains("ComfyUI 没有响应")), "{:?}", r.notes);
        assert!(r.notes.iter().any(|n| n.contains("第 1 镜") && n.contains("还没合成")), "{:?}", r.notes);
        assert!(r.notes.iter().any(|n| n.contains("gone")), "{:?}", r.notes);
        assert_eq!(seen.last(), Some(&(4, 4, String::new())));
        assert!(seen.contains(&(3, 4, "第 2 镜".to_string())));

        // 合成之后改过字段：跳过，不出
        let mut ep = 读集(d.path());
        ep.storyboard.iter_mut().find(|s| s.id == "s2").unwrap().lighting = "改过".into();
        std::fs::write(cinema_projects_dir(d.path()).join("p1/episodes/e1/episode.json"), serde_json::to_string(&ep).unwrap()).unwrap();
        let called = std::sync::Mutex::new(Vec::<String>::new());
        let render = |req: RenderShotReq| {
            called.lock().unwrap().push(req.shot_id.clone());
            async move { Err::<RenderShotResult, _>(Error::runtime("不该调")) }
        };
        let r = render_batch(d.path(), "p1", "e1", &["s2".to_string()], render, |_: u32| async { Err::<String, Error>(Error::runtime("不该调")) }, |_, _, _| {}, || false, Vec::new).await.unwrap();
        assert!(called.lock().unwrap().is_empty());
        assert!(r.notes[0].contains("重新合成"), "{:?}", r.notes);

        // 用户点了停止：出完当前这一镜，后面的不再出
        let calls = std::sync::Mutex::new(0);
        let render = |req: RenderShotReq| {
            *calls.lock().unwrap() += 1;
            async move { Ok(RenderShotResult { shot_id: req.shot_id, image_url: String::new(), video_url: String::new(), audio_url: String::new(), status: "ready".into() }) }
        };
        let stop_after_first = || *calls.lock().unwrap() >= 1;
        let ids: Vec<String> = ["s3", "s3", "s3"].iter().map(|s| s.to_string()).collect();
        let r = render_batch(d.path(), "p1", "e1", &ids, render, |_: u32| async { Err::<String, Error>(Error::runtime("不该调")) }, |_, _, _| {}, stop_after_first, Vec::new).await.unwrap();
        assert_eq!(*calls.lock().unwrap(), 1, "停止之后又出了");
        assert!(r.notes.iter().any(|n| n.contains("已停止，后面 2 镜没有出")), "{:?}", r.notes);
    }

    /// **有图却没挂成参考图**（2026-10-06 EP03 第 3 镜：悬浮车的图在镜头表生成之后才出，出片时只剩 `hovercar` 一个词，
    /// H3 出成了四轮轿车）：提示（不挡出片）+「挂上参考图」按钮；只出声的人、首帧出片时的人物不算；挂满 9 张不给按钮
    #[test]
    fn 有图没挂的资产_提示并给挂上按钮() {
        use crate::assets::AssetKind;
        use crate::shot_table::{RefSlot, RefUsage};
        let a = |id: &str, kind: AssetKind, name: &str, img: bool| crate::storyboard_gen::AssetRef {
            id: id.into(), kind, name: name.into(), has_image: img, description: String::new(),
            look_id: String::new(), looks: Vec::new(), h3_voice: String::new(),
        };
        let assets = vec![
            a("car", AssetKind::Prop, "白色悬浮车", true),
            a("fruit", AssetKind::Prop, "水果箱", false),
            a("xu", AssetKind::Character, "苏晴", true),
            a("voice", AssetKind::Character, "吴峥", true),
            a("alley", AssetKind::Scene, "巷口", true),
        ];
        let mut s = shot("s3", 3, false);
        s.character_ids = vec!["xu".into(), "voice".into()];
        s.prop_ids = vec!["car".into(), "fruit".into()];
        s.scene_id = Some("alley".into());
        s.segments = vec![crate::shot_table::ShotSegment { start: 0.0, end: 8.0, visual: "苏晴下车".into(), ..Default::default() }];
        s.dialogue_lines = vec![DialogueLine { speaker: "吴峥".into(), line: "走吧。".into(), off_screen: true, ..Default::default() }];
        s.ref_slots = vec![RefSlot { order: 1, asset_id: "xu".into(), usage: RefUsage::Identity, control: "脸".into(), not_control: String::new() }];
        let names: Vec<&str> = unmounted_assets(&s, &assets).iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["巷口", "白色悬浮车"], "没图的水果箱、只出声的吴峥、已挂的苏晴都不算");
        let issues = unmounted_ref_issues(std::slice::from_ref(&s), &assets);
        assert!(issues.iter().all(|i| i.field == "advice"), "只提示，不挡出片：{issues:?}");
        let car = issues.iter().find(|i| i.message.starts_with("「白色悬浮车」")).expect("悬浮车该报");
        assert_eq!(mount_fix(&s, &assets, &car.message), Some(IssueFix::Mount { asset_id: "car".into(), usage: RefUsage::Identity, control: "白色悬浮车的外形、颜色与材质".into() }));
        let alley = issues.iter().find(|i| i.message.starts_with("「巷口」")).unwrap();
        assert!(matches!(mount_fix(&s, &assets, &alley.message), Some(IssueFix::Mount { usage: RefUsage::Geography, .. })));
        // 首帧出片：人物长相只认首帧，不报
        let mut kf = s.clone();
        kf.ref_slots.clear();
        kf.keyframes.first = true;
        assert!(kf.keyframes.drives_video(), "前提：挂了首帧就按首帧出");
        assert!(!unmounted_assets(&kf, &assets).iter().any(|a| a.name == "苏晴"));
        // 挂满 9 张：照样提示，不给按钮
        let mut full = s.clone();
        full.ref_slots = (1..=9u8).map(|i| RefSlot { order: i, asset_id: format!("x{i}"), usage: RefUsage::Identity, control: "c".into(), not_control: String::new() }).collect();
        let msg = unmounted_ref_issues(std::slice::from_ref(&full), &assets).into_iter().find(|i| i.message.starts_with("「白色悬浮车」")).unwrap().message;
        assert!(msg.contains("挂满"), "{msg}");
        assert_eq!(mount_fix(&full, &assets, &msg), None);
    }

    /// 线路 B 正文里的英文二选一：红框右边给「重新合成这一镜」（代码改英文句子改不准，交给写手；合成时会被要求只写一个版本）
    #[test]
    fn 正文二选一给重新合成的修复按钮() {
        use crate::shot_table::{PromptRoute, RoutePrompts};
        let d = tempfile::tempdir().unwrap();
        合成用的集(d.path(), "b");
        let mut ep = 读集(d.path());
        let snapshot = ep.storyboard.clone();
        for s in ep.storyboard.iter_mut() {
            let h = current_hash(&snapshot, s, &[], "", &[]);
            s.prompts.insert(PromptRoute::B, RoutePrompts { video: "[Shot 1] neighbors throw back their heads, or slap their thighs laughing.".into(), frame: "f".into(), synthesized_at: 1, fields_hash: h });
        }
        std::fs::write(cinema_projects_dir(d.path()).join("p1/episodes/e1/episode.json"), serde_json::to_string(&ep).unwrap()).unwrap();
        let ep = 读集(d.path());
        let fixes = issue_fixes(d.path(), "p1", "e1", &ep);
        let alt: Vec<_> = fixes.iter().filter(|f| f.message.starts_with("正文里有二选一")).collect();
        assert!(!alt.is_empty(), "{fixes:?}");
        assert!(alt.iter().all(|f| f.fix == IssueFix::Resynth && f.label == "重新合成这一镜"), "{alt:?}");
    }

    /// 「自动修正 N 处」的 N 要是**真能改掉的**（2026-09-30 EP02：按钮写 5 处，点了一处没改、也没反应）。
    /// 后端在内存里修一遍，只回修完不见了的那几条；不写盘
    #[test]
    fn 自动修正只算真能改掉的_预演不写盘() {
        let d = tempfile::tempdir().unwrap();
        合成用的集(d.path(), "b");
        let f = cinema_projects_dir(d.path()).join("p1/episodes/e1/episode.json");
        let mut ep = 读集(d.path());
        for s in ep.storyboard.iter_mut() {
            if s.id == "s1" {
                s.segments[0].visual = "画面1，切至近景".into(); // 切镜词：代码改得掉
            } else {
                // 画面里没写到、资产库也没有的说话人：代码改不了
                s.dialogue_lines = vec![DialogueLine { speaker: "路人".into(), line: "让一让。".into(), ..Default::default() }];
            }
        }
        std::fs::write(&f, serde_json::to_string(&ep).unwrap()).unwrap();
        let before = std::fs::read_to_string(&f).unwrap();
        let all = validate_storyboard(d.path(), "p1", "e1", &ep);
        assert!(all.iter().any(|i| i.message.contains("路人")), "{all:?}");
        let fixable = repairable_issues(d.path(), "p1", "e1", &ep);
        assert_eq!(fixable.len(), 1, "{fixable:?}");
        assert!(fixable[0].message.contains("切至"), "{fixable:?}");
        assert_eq!(std::fs::read_to_string(&f).unwrap(), before, "预演不能写盘");
    }

    /// 2026-09-30 第 5 场出完只记在任务上：分镜页看不到、FCP 带不上、再点批量又排进去跑 9 小时。
    /// 现在**整条**记在场上（不切开），界面拿旧的分集保存也冲不掉
    #[test]
    fn 长镜头出完整条记在场上_界面保存冲不掉() {
        use crate::shot_table::PromptRoute;
        let d = tempfile::tempdir().unwrap();
        合成用的集(d.path(), "b");
        let shots = cinema_projects_dir(d.path()).join("p1/episodes/e1/shots");
        std::fs::create_dir_all(&shots).unwrap();
        let full = shots.join("longtake_s1-1.mp4");
        std::fs::write(&full, b"mp4").unwrap();
        let stale_ui = 读集(d.path()); // 界面在出完之前拿的那份
        let win = |id: &str, n: usize| LongTakeWindow { shot_id: id.into(), prompt: format!("窗{n}"), fields_hash: format!("h{n}"), picture_paths: vec![format!("/图/{n}.png")] };
        record_scene_render(d.path(), "p1", "e1", 1, &full, &[win("s1", 1), win("s2", 2)], PromptRoute::B, 7, "长镜头 · 双采 · 64×64").unwrap();
        let ep = 读集(d.path());
        let r = scene_render(&ep, 1, Some(PromptRoute::B)).expect("没记上");
        assert_eq!((r.path.as_str(), r.seed, r.shot_ids.clone(), r.prompts.clone()), (full.to_str().unwrap(), 7, vec!["s1".to_string(), "s2".into()], vec!["窗1".to_string(), "窗2".into()]));
        assert!(scene_render(&ep, 1, Some(PromptRoute::A)).is_none(), "另一条线路的不算");
        // **不切开**：各镜名下什么都不多
        assert!(ep.storyboard.iter().all(|s| s.takes.is_empty() && s.video_url.is_none()));
        // 界面拿着出完之前的那份保存，整段成片不能被冲掉
        crate::episode_lock::save_over(&cinema_projects_dir(d.path()).join("p1/episodes/e1/episode.json"), |disk| {
            let mut e = stale_ui.clone();
            if let Some(disk) = disk {
                e.scene_renders = disk.scene_renders.clone();
            }
            Ok(e)
        })
        .unwrap();
        assert!(scene_render(&读集(d.path()), 1, None).is_some());
        let src = include_str!("cinema.rs");
        let save = &src[src.find("pub async fn cinema_save_episode(").unwrap()..];
        assert!(save[..save.find("\n}\n").unwrap()].contains("episode.scene_renders = disk.scene_renders.clone();"), "保存那条没沿用盘上的");
    }

    /// 重新生成过这一场（镜头 id 换了）的旧整段片不能再用：原来按场号认，合成一集照样拼进去（2026-10-03 审查）
    #[test]
    fn 长镜头整段片要是这一场现在这几镜出的() {
        use crate::shot_table::PromptRoute;
        let mut ep = EpisodeData { long_take_scenes: vec![1], ..serde_json::from_value(json!({"id":"e1","episode_num":1,"title":"t","synopsis":"","script_text":"","storyboard":[]})).unwrap() };
        let shot = |id: &str, n: u32| StoryboardShot { scene_no: 1, ..分镜(json!({"id": id, "shot_num": n})) };
        ep.storyboard = vec![shot("s2", 2), shot("s1", 1)];
        let clip = tempfile::NamedTempFile::new().unwrap();
        ep.scene_renders = vec![SceneRender { scene_no: 1, route: PromptRoute::B, path: clip.path().to_string_lossy().to_string(), created_at: 1, seed: 0, quality: String::new(),
            shot_ids: vec!["s1".into(), "s2".into()], prompts: vec![], fields_hashes: vec![], picture_paths: vec![] }];
        assert!(long_take_clip(&ep, 1, PromptRoute::B).is_some(), "同一批镜头（按镜号排）");
        ep.storyboard[0].id = "新的s2".into();
        assert!(long_take_clip(&ep, 1, PromptRoute::B).is_none(), "镜头换了还在用旧整段片");
        assert!(matches!(episode_segments(&ep, PromptRoute::B)[0], EpisodeSegment::Shot(_)), "合成一集该按各镜算");
        ep.scene_renders[0].shot_ids.clear();
        assert!(long_take_clip(&ep, 1, PromptRoute::B).is_some(), "老数据没记镜头的照旧认");
    }

    /// 2026-09-29 夜里：前端先单独提交第 5 场长镜头、再提交其余镜头的批量，批量被「这一集有镜头正在出片」拒了，
    /// 其余 32 镜一整夜没排上。现在长镜头的场以 `scene_<场号>` 夹在批量里，**按给的顺序**出，失败照样不阻断后面的
    #[tokio::test]
    async fn 批量出片_长镜头的场按顺序夹在中间出() {
        use crate::shot_table::{PromptRoute, RoutePrompts};
        let d = tempfile::tempdir().unwrap();
        合成用的集(d.path(), "b");
        let mut ep = 读集(d.path());
        let snapshot = ep.storyboard.clone();
        for s in ep.storyboard.iter_mut() {
            let h = current_hash(&snapshot, s, &[], "", &[]);
            s.prompts.insert(PromptRoute::B, RoutePrompts { video: "B 正文".into(), frame: "f".into(), synthesized_at: 1, fields_hash: h });
        }
        std::fs::write(cinema_projects_dir(d.path()).join("p1/episodes/e1/episode.json"), serde_json::to_string(&ep).unwrap()).unwrap();

        let order = std::sync::Mutex::new(Vec::<String>::new());
        let render = |req: RenderShotReq| {
            order.lock().unwrap().push(req.shot_id.clone());
            async move { Ok(RenderShotResult { shot_id: req.shot_id, image_url: String::new(), video_url: "v.mp4".into(), audio_url: String::new(), status: "ready".into() }) }
        };
        let render_scene = |n: u32| {
            order.lock().unwrap().push(format!("场{n}"));
            async move { if n == 3 { Err(Error::runtime("ComfyUI 没有响应")) } else { Ok(format!("longtake_s{n}.mp4")) } }
        };
        let ids: Vec<String> = ["s1", "scene_2", "scene_3", "s2"].iter().map(|s| s.to_string()).collect();
        let mut seen = Vec::new();
        let r = render_batch(d.path(), "p1", "e1", &ids, render, render_scene, |done, total, label| seen.push((done, total, label.to_string())), || false, Vec::new).await.unwrap();
        assert_eq!(*order.lock().unwrap(), ["s1", "场2", "场3", "s2"], "按给的顺序出，长镜头不提前也不挪后");
        assert_eq!((r.done, r.total), (3, 4));
        assert!(r.notes.iter().any(|n| n.contains("第 3 场长镜头出片失败") && n.contains("ComfyUI 没有响应")), "{:?}", r.notes);
        // 长镜头那一项出完就是出完了，不能再被当成一个「找不到的镜头」记一笔
        assert_eq!(r.notes.len(), 1, "{:?}", r.notes);
        assert!(seen.contains(&(1, 4, "第 2 场 · 长镜头".to_string())), "{seen:?}");
        assert_eq!(long_take_unit("scene_12"), Some(12));
        assert_eq!(long_take_unit("shot_abc"), None);
    }

    /// 2026-09-27 按场出第 2 场：第 3 镜过期被跳过，接着它的 4~7 镜各报一条「第 N 镜还没出片」的失败。
    /// 现在：卡住的那一镜说原因，接它的跟着跳过、说清楚卡在谁身上，不再调出片。
    #[tokio::test]
    async fn 批量出片_上一镜出不了_接它的跟着跳过_说清楚卡在谁() {
        use crate::shot_table::{PromptRoute, RoutePrompts};
        let d = tempfile::tempdir().unwrap();
        合成用的集(d.path(), "b");
        let mut ep = 读集(d.path());
        let mut s3 = ep.storyboard[1].clone();
        s3.id = "s3".into();
        s3.shot_num = 3;
        ep.storyboard.push(s3);
        for s in ep.storyboard.iter_mut().filter(|s| s.id != "s1") {
            s.continuity = Continuity::RefVideo;
        }
        let snapshot = ep.storyboard.clone();
        for s in ep.storyboard.iter_mut().filter(|s| s.id != "s1") {
            let h = current_hash(&snapshot, s, &[], "", &[]);
            s.prompts.insert(PromptRoute::B, RoutePrompts { video: "B 正文".into(), frame: "f".into(), synthesized_at: 1, fields_hash: h });
        }
        std::fs::write(cinema_projects_dir(d.path()).join("p1/episodes/e1/episode.json"), serde_json::to_string(&ep).unwrap()).unwrap();

        let called = std::sync::Mutex::new(Vec::<String>::new());
        let render = |req: RenderShotReq| {
            called.lock().unwrap().push(req.shot_id.clone());
            async move { Err::<RenderShotResult, _>(Error::runtime("不该调")) }
        };
        let ids: Vec<String> = ["s1", "s2", "s3"].iter().map(|s| s.to_string()).collect();
        let r = render_batch(d.path(), "p1", "e1", &ids, render, |_: u32| async { Err::<String, Error>(Error::runtime("不该调")) }, |_, _, _| {}, || false, Vec::new).await.unwrap();
        assert!(called.lock().unwrap().is_empty(), "注定失败的不该再调出片、登记失败任务");
        assert!(r.notes[0].contains("第 1 镜") && r.notes[0].contains("还没合成"), "{:?}", r.notes);
        assert!(r.notes[1].contains("第 2 镜：接的是第 1 镜，第 1 镜这一批没出成"), "{:?}", r.notes);
        assert!(r.notes[2].contains("第 3 镜：接的是第 2 镜"), "{:?}", r.notes);

        // 上一镜不在这一批、也从没出过：说「还没出片」
        let r = render_batch(d.path(), "p1", "e1", &["s3".to_string()], render, |_: u32| async { Err::<String, Error>(Error::runtime("不该调")) }, |_, _, _| {}, || false, Vec::new).await.unwrap();
        assert!(r.notes[0].contains("要接第 2 镜，第 2 镜还没出片"), "{:?}", r.notes);
    }

    #[test]
    fn 单镜出片和批量出片共用同一个登记函数_批量时不许单镜插队() {
        let src = include_str!("cinema.rs");
        let single = &src[src.find("pub async fn cinema_render_shot_video(").unwrap()..];
        let single = &single[..single.find("\n}\n").unwrap()];
        // **只比函数名，不比整段参数。** 2026-09-18 这条断言因为
        // `&app` 变成 `Some(&app)` 就挂了 —— 而那个改动和「共用同一个登记函数」
        // 这个不变量毫无关系。断言写得比不变量窄，就会在无关改动上误报。
        assert!(single.contains("render_video_registered("), "单镜没走统一的登记函数");
        assert!(single.contains("JobKind::VideoBatch"), "批量在跑时要拦单镜");
        // 批量的本体在 `start_render_batch_with`（`start_render_batch` 和加入队列都走它）
        let batch = &src[src.find("fn start_render_batch_with(").unwrap()..];
        let batch = &batch[..batch.find("\n}\n").unwrap()];
        assert!(batch.contains("render_video_registered("), "批量没走统一的登记函数");
        assert!(batch.contains("set_targets(&config_dir, &job_id, &shot_ids)"));
        // 长镜头的场也在同一个批量里出，和单独的「整段出片」共用登记
        assert!(batch.contains("register_long_take(") && batch.contains("run_registered_long_take("), "批量里的长镜头没走统一的登记");
    }

    // ── 手动改字段后重新检查 ────────────────────────────────────────────────

    #[test]
    fn 手动改了字段_整场检查也重新做() {
        let mut ep: EpisodeData = serde_json::from_value(json!({
            "id":"e1","episode_num":1,"title":"t","synopsis":"","storyboard_version":3,
            "script_text": "片名\n【办公室】\n键盘声急促。她抬头：“我还能撑。”\n【金銮殿】\n鼓声震天。",
            "storyboard": [
                {"id":"s1","shot_num":1,"scene_no":1,"scale":"","camera_movement":"","dialogue":"","prompt":"","duration":8.0,"status":"pending",
                 "source":"键盘声急促。她抬头：“我还能撑。”","dialogue_lines":[{"speaker":"她","line":"我还能撑。"}]},
                {"id":"s2","shot_num":2,"scene_no":2,"scale":"","camera_movement":"","dialogue":"","prompt":"","duration":8.0,"status":"pending","source":"鼓声震天。"}
            ]
        })).unwrap();
        assert_eq!(scene_coverage_issues(&ep), vec![], "表是好的");
        // 用户把台词删了、把原文改成自己写的
        ep.storyboard[0].dialogue_lines.clear();
        ep.storyboard[0].source = "她伏案工作".into();
        let issues = scene_coverage_issues(&ep);
        assert!(issues.iter().any(|i| i.message.contains("我还能撑") && i.field == "dialogue"), "{issues:?}");
        assert!(issues.iter().any(|i| i.message.contains("她伏案工作")), "{issues:?}");
        assert!(issues.iter().any(|i| i.message.contains("键盘声急促") && i.message.contains("没有任何一镜拍到")), "{issues:?}");
        // 场数对不上（模型切的场）就不做
        ep.storyboard[1].scene_no = 3;
        assert_eq!(scene_coverage_issues(&ep), vec![]);
    }

    // ── 单独重新生成一场 ────────────────────────────────────────────────

    #[tokio::test]
    async fn 只重新生成一场_其它场不动_镜号顺延_问题先告诉模型() {
        use crate::shot_table::{PromptRoute, RoutePrompts};
        let d = tempfile::tempdir().unwrap();
        let proj = cinema_projects_dir(d.path()).join("p1");
        std::fs::create_dir_all(proj.join("episodes/e1")).unwrap();
        let shot = |id: &str, num: u32, scene: u32, source: &str| json!({
            "id": id, "shot_num": num, "scene_no": scene, "scale": "", "camera_movement": "", "dialogue": "", "prompt": "",
            "duration": 8.0, "status": "pending", "source": source,
            "segments": [{"start": 0.0, "end": 8.0, "visual": format!("画面{num}")}]
        });
        std::fs::write(proj.join("episodes/e1/episode.json"), json!({
            "id":"e1","episode_num":1,"title":"t","synopsis":"","storyboard_version":3,
            "script_text": "【一】\n开场。\n【二】\n殿外风起。皇帝低声说话。\n【三】\n结尾。",
            "storyboard": [shot("a", 1, 1, "开场。"), shot("b", 2, 2, "皇帝低声说话。"), shot("c", 3, 3, "结尾。")]
        }).to_string()).unwrap();
        // 第 3 场那一镜有合成好的提示词，不能被动
        let mut ep: EpisodeData = serde_json::from_str(&std::fs::read_to_string(proj.join("episodes/e1/episode.json")).unwrap()).unwrap();
        ep.storyboard[2].prompts.insert(PromptRoute::B, RoutePrompts { video: "第 3 场的提示词".into(), ..Default::default() });
        std::fs::write(proj.join("episodes/e1/episode.json"), serde_json::to_string(&ep).unwrap()).unwrap();

        let good = |source: &str| json!({"duration": 6, "source": source, "segments": [{"start": 0, "end": 6, "visual": "风吹过殿外"}]});
        let reply = json!({"shots": [good("殿外风起。"), good("皇帝低声说话。")]}).to_string();
        let seen = std::sync::Mutex::new(Vec::<Vec<(String, String)>>::new());
        let call = |_slot: crate::prompts::PromptSlot, vars: Vec<(String, String)>| {
            seen.lock().unwrap().push(vars);
            let r = reply.clone();
            async move { Ok::<String, Error>(r) }
        };
        let r = regenerate_scene(d.path(), "p1", "e1", 2, call).await.unwrap();
        assert_eq!((r.scene_no, r.shots, r.issues), (2, 2, 0), "{:?}", r.notes);
        let fb = &seen.lock().unwrap()[0].iter().find(|(k, _)| k == "FEEDBACK").unwrap().1.clone();
        assert!(fb.contains("殿外风起"), "漏拍的句子要先告诉模型：{fb}");
        let ep: EpisodeData = serde_json::from_str(&std::fs::read_to_string(proj.join("episodes/e1/episode.json")).unwrap()).unwrap();
        let rows: Vec<(u32, u32, String)> = ep.storyboard.iter().map(|s| (s.scene_no, s.shot_num, s.id.clone())).collect();
        assert_eq!(rows[0], (1, 1, "a".to_string()));
        assert_eq!((rows[1].0, rows[1].1), (2, 2));
        assert_eq!((rows[2].0, rows[2].1), (2, 3));
        assert_eq!(rows[3], (3, 4, "c".to_string()), "后面的场镜号顺延、id 不变");
        assert_eq!(ep.storyboard[3].prompts[&PromptRoute::B].video, "第 3 场的提示词", "别的场的提示词被动了");
        assert!(!ep.storyboard.iter().any(|s| s.id == "b"), "这一场旧的镜头要换掉");
    }

    /// 把某一场改成一镜出、30 秒 —— 重新生成这一场：换槽位、两镜各 15 秒、自动进长镜头。
    #[tokio::test]
    async fn 一场改成一镜出_重新生成后是等长两镜_自动整段出() {
        let d = tempfile::tempdir().unwrap();
        let proj = cinema_projects_dir(d.path()).join("p1");
        std::fs::create_dir_all(proj.join("episodes/e1")).unwrap();
        let shot = |id: &str, num: u32, scene: u32, secs: f32| json!({
            "id": id, "shot_num": num, "scene_no": scene, "scale": "", "camera_movement": "", "dialogue": "", "prompt": "",
            "duration": secs, "status": "pending", "source": "开场。",
            "segments": [{"start": 0.0, "end": secs, "visual": "画面"}]
        });
        std::fs::write(proj.join("episodes/e1/episode.json"), json!({
            "id":"e1","episode_num":1,"title":"t","synopsis":"","storyboard_version":3,
            "script_text": "【一】\n开场。\n【二】\n殿外风起。皇帝低声说话。",
            "scene_takes": [{"scene_no": 2, "one_take": true, "secs": 30.0}],
            "storyboard": [shot("a", 1, 1, 8.0), shot("b", 2, 2, 4.0), shot("c", 3, 2, 2.0)]
        }).to_string()).unwrap();
        let reply = json!({"shots": [
            {"duration": 10, "source": "殿外风起。", "segments": [{"start": 0, "end": 10, "visual": "风吹过殿外，硬切到皇帝近景"}]},
            {"duration": 10, "source": "皇帝低声说话。", "segments": [{"start": 0, "end": 10, "visual": "皇帝低声说话"}]}
        ]}).to_string();
        let seen = std::sync::Mutex::new(Vec::<(crate::prompts::PromptSlot, Vec<(String, String)>)>::new());
        let call = |slot: crate::prompts::PromptSlot, vars: Vec<(String, String)>| {
            seen.lock().unwrap().push((slot, vars));
            let r = reply.clone();
            async move { Ok::<String, Error>(r) }
        };
        let r = regenerate_scene(d.path(), "p1", "e1", 2, call).await.unwrap();
        assert_eq!(r.shots, 2);
        let (slot, vars) = seen.lock().unwrap()[0].clone();
        assert_eq!(slot, crate::prompts::PromptSlot::ShotTableOneTake);
        let v = |k: &str| vars.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone()).unwrap_or_default();
        assert_eq!((v("SHOT_COUNT"), v("SHOT_SECS")), ("2".into(), "15".into()));
        assert!(!v("FEEDBACK").contains("不在"), "旧镜头的时长问题不该带给一镜出：{}", v("FEEDBACK"));
        let ep: EpisodeData = serde_json::from_str(&std::fs::read_to_string(proj.join("episodes/e1/episode.json")).unwrap()).unwrap();
        let scene2: Vec<&StoryboardShot> = ep.storyboard.iter().filter(|s| s.scene_no == 2).collect();
        assert_eq!(scene2.iter().map(|s| (s.duration, s.one_take)).collect::<Vec<_>>(), [(15.0, true), (15.0, true)]);
        assert_eq!(ep.long_take_scenes, [2], "两镜以上自动整段出");
        assert!(!ep.storyboard[0].one_take, "别的场不动");
    }

    #[test]
    fn 一镜出的长镜头标记跟着镜数走_用户自己勾的不碰() {
        let mk = |scene: u32, one_take: bool| {
            let mut s: StoryboardShot = serde_json::from_value(json!({"id": "x", "shot_num": 1, "scene_no": scene, "scale": "", "camera_movement": "", "dialogue": "", "prompt": "", "duration": 15.0, "status": "pending"})).unwrap();
            s.one_take = one_take;
            s
        };
        let mut lt = vec![1, 3];
        sync_one_take_long_take(&mut lt, &[mk(1, true)], 1);
        assert_eq!(lt, [3], "一镜出只剩 1 镜就不是长镜头");
        sync_one_take_long_take(&mut lt, &[mk(2, true), mk(2, true)], 2);
        assert_eq!(lt, [2, 3]);
        sync_one_take_long_take(&mut lt, &[mk(3, false), mk(3, false)], 3);
        assert_eq!(lt, [2, 3], "不是一镜出的场是用户自己勾的，不碰");
    }

    #[tokio::test]
    async fn 剧本没有标题时不能单独重新生成一场() {
        let d = tempfile::tempdir().unwrap();
        合成用的集(d.path(), "b");
        let call = |_: crate::prompts::PromptSlot, _: Vec<(String, String)>| async { Err::<String, _>(Error::runtime("不该调")) };
        let err = regenerate_scene(d.path(), "p1", "e1", 1, call).await.unwrap_err();
        assert!(err.to_string().contains("重新生成整张镜头表"), "{err}");
    }

    // ── 出片设置 ────────────────────────────────────────────────

    #[test]
    fn 出片设置_清晰度和比例定尺寸_默认双采480p() {
        let d = RenderSettings::default();
        // 默认**不烧**硬字幕：烧进画面的事后改不了，字幕走合成时另出的 final.srt
        assert_eq!((d.quality, d.resolution.as_str(), d.takes, d.generate_audio, d.burn_subtitles), (RenderQuality::Staged, "480p", 1, true, false));
        assert_eq!(d.size("16:9"), (896, 512), "对照实测用的草稿尺寸");
        assert_eq!(d.size("9:16"), (512, 896));
        let hd = RenderSettings { resolution: "720p".into(), ..d.clone() };
        assert_eq!(hd.size("16:9"), (1312, 736));
        assert_eq!(RenderSettings { resolution: "576p".into(), ..d.clone() }.size("16:9"), (1024, 576));
        // 设置里指定了比例就不跟项目
        assert_eq!(RenderSettings { aspect: "1:1".into(), ..d.clone() }.size("16:9"), (512, 512));
        // 认不出的比例按 16:9；超长边会被引擎上限压住并 32 对齐
        assert_eq!(RenderSettings { aspect: "abc".into(), ..d.clone() }.size(""), (896, 512));
        let (w, h) = RenderSettings { resolution: "720p".into(), aspect: "21:9".into(), ..d }.size("");
        assert!(w <= 1344 && w % 32 == 0 && h % 32 == 0, "{w}x{h}");
        // 老项目没有 render 键也能读，给默认
        let ms: ModelSettings = serde_json::from_value(json!({"prompt_route": "a"})).unwrap();
        assert_eq!(ms.render, RenderSettings::default());
        assert_eq!(RenderQuality::Dense.tier(), crate::agent_runtime::weetodd::H3Tier::Dense);
        assert_eq!(RenderQuality::LightX2V.tier(), crate::agent_runtime::weetodd::H3Tier::LightX2V);
        assert_eq!(serde_json::to_value(RenderQuality::LightX2V).unwrap(), "lightx2v", "和前端 renderOptions 的 id 一致");
    }

    // ── 出片挂哪几张图 ────────────────────────────────────────────────

    /// 出片挂的图和合成提示词用的是同一份：没图的跳过、顺序按槽位编号、起始帧文件在才挂。
    #[test]
    fn 出片挂图顺序和提示词一致_没图的不挂() {
        use crate::assets::{AssetKind, Scope};
        use crate::shot_table::{RefSlot, RefUsage};
        let d = tempfile::tempdir().unwrap();
        let cfg = d.path();
        let scope = Scope::Project { project_id: "p1".into() };
        let with_img = 建到(cfg, scope.clone(), AssetKind::Character, "皇帝");
        let no_img = 建到(cfg, scope.clone(), AssetKind::Character, "大臣");
        let scene = 建到(cfg, scope.clone(), AssetKind::Scene, "金銮殿");
        // 给皇帝和金銮殿加一版图
        let png = cfg.join("x.png");
        std::fs::write(&png, b"\x89PNG fake").unwrap();
        let mut store = crate::assets::load_store(cfg, &scope);
        for a in store.assets.iter_mut().filter(|a| a.id == with_img || a.id == scene) {
            crate::assets::add_variant(cfg, &scope, a, b"\x89PNG fake", "png", false, None, true, None).unwrap();
        }
        crate::assets::save_store(cfg, &scope, &store).unwrap();

        let shots_dir = cfg.join("shots");
        std::fs::create_dir_all(&shots_dir).unwrap();
        let mut shot = test_shot("s1");
        let slot = |order: u8, id: &str, usage: RefUsage| RefSlot { order, asset_id: id.into(), usage, control: "c".into(), not_control: String::new() };
        shot.ref_slots = vec![slot(3, &scene, RefUsage::Geography), slot(1, &no_img, RefUsage::Identity), slot(2, &with_img, RefUsage::Identity), slot(4, "frame", RefUsage::StartFrame)];
        let assets = crate::storyboard_gen::load_assets(cfg, "p1", "e1");
        let paths = render_reference_paths(cfg, "p1", "e1", &shots_dir, &shot, &assets, false);
        let ctx = crate::prompt_input::ShotContext { shot: &shot, prev: None, assets: &assets, style: "", long_take: false };
        let names: Vec<String> = ctx.slots().iter().filter_map(|(_, a)| a.map(|a| a.name.clone())).collect();
        assert_eq!(names, ["皇帝", "金銮殿"], "提示词那边的顺序");
        assert_eq!(paths.len(), 2, "没图的大臣不挂；起始帧文件不在不挂：{paths:?}");
        assert_eq!(paths[0], asset_image_path(cfg, "p1", "e1", &with_img, "").unwrap());
        assert_eq!(paths[1], asset_image_path(cfg, "p1", "e1", &scene, "").unwrap());
        // 起始帧文件放进去就挂上，排在最后
        std::fs::write(shots_dir.join("s1.png"), b"x").unwrap();
        let paths = render_reference_paths(cfg, "p1", "e1", &shots_dir, &shot, &assets, false);
        assert_eq!(paths.last().unwrap(), &shots_dir.join("s1.png"));
        assert!(asset_image_path(cfg, "p1", "e1", &no_img, "").is_none());
        // **长镜头里挂了首帧也按资产槽位挂**（合成时 `long_take: true` 不走首帧写法）。
        // 原来这里写死 false：只挂首帧那一张，<Picture 2..N> 全指向空处（2026-10-03 审查）
        shot.keyframes = Keyframes { first: true, last: false };
        assert_eq!(render_reference_paths(cfg, "p1", "e1", &shots_dir, &shot, &assets, false), [shots_dir.join("s1.png")], "普通镜头挂了首帧：只发首帧");
        let paths = render_reference_paths(cfg, "p1", "e1", &shots_dir, &shot, &assets, true);
        assert_eq!(paths[0], asset_image_path(cfg, "p1", "e1", &with_img, "").unwrap(), "长镜头：{paths:?}");
        assert_eq!(paths.len(), 3, "{paths:?}");
    }

    /// 出片前最后一道关：提示词点名的图号多过实际挂上的，或写了 <Video 1> 没挂视频，不发出去。
    #[test]
    fn 出片前对一遍提示词点名的图和实际挂的() {
        assert_eq!(max_picture_ref("<Subject 1> from <Picture 1>, <Picture 3> is the room; <Picture x> <Picture 12"), 3);
        assert_eq!(max_picture_ref("参考图1：陆川（身份）。参考图2：拆机间"), 2);
        assert!(render_refs_guard(7, "<Picture 1> <Picture 3>", 3, false).is_ok());
        let e = render_refs_guard(7, "<Picture 1> <Picture 3>", 2, false).unwrap_err().to_string();
        assert!(e.contains("第 7 镜") && e.contains("第 3 张") && e.contains("2 张"), "{e}");
        assert!(render_refs_guard(7, "参考图3：铁皮屋", 2, false).is_err(), "线路 A 的写法也要查");
        assert!(render_refs_guard(7, "<Video 1> is the end of the previous shot", 0, false).is_err());
        assert!(render_refs_guard(7, "<Video 1> is the end of the previous shot", 0, true).is_ok());
        assert!(render_refs_guard(7, "没有任何引用", 0, false).is_ok());
        // 三处出片都接上了（首帧模式、参考模式、长镜头窗口），LTX 不收按 H3 写的提示词
        let src = include_str!("cinema.rs");
        let prod = &src[..src.find("#[cfg(test)]").unwrap()];
        assert_eq!(prod.matches("render_refs_guard(shot.shot_num, &prompt,").count(), 3);
        // 单独出长镜头场里的一镜：和合成时一样，长镜头不走首帧出片
        assert!(prod.contains("let keyframe_video = keyframes.drives_video() && !long_take;"));
        assert_eq!(prod.matches("engine == \"h3\" && keyframe_video").count(), 2);
        assert!(prod.contains("if has_h3_tags(&prompt) || prompt.contains(\"<d>\") {"));
    }

    /// 10-07：校验提示用的资产要套这一镜选的造型（和合成、出片同一份），否则选了晚礼服的镜头照样报「没选造型」
    #[test]
    fn 校验提示套这一镜的造型() {
        let src = include_str!("cinema.rs");
        let at = src.find("pub fn validate_storyboard(").unwrap();
        let body = &src[at..at + src[at..].find("\n}\n").unwrap()];
        assert!(body.contains("let shot_assets = crate::looks::for_shot(&assets, shot);"));
        assert!(body.contains("ShotContext { shot, prev, assets, style: &style, long_take: false }"));
    }

    /// 2026-10-07 复查出片链路：首帧出片记首帧那一栏的画质；「接住最后一帧」没首帧时拦下（以前悄悄走参考出片）
    #[test]
    fn 出片按链路记画质_接住最后一帧没首帧要拦() {
        let src = include_str!("cinema.rs");
        let prod = &src[..src.find("#[cfg(test)]").unwrap()];
        assert!(prod.contains("format!(\"首帧 · {} · {width}×{height}\", render.keyframe_quality.label())"));
        assert!(prod.contains("Some(render.keyframe_quality.tier()),"), "首帧出片没用首帧那一栏的画质");
        assert!(prod.contains("shot.effective_continuity() == Continuity::FirstFrame {"));
        assert!(prod.contains("设了「接住最后一帧」，但还没有首帧"));
        assert_eq!(RenderQuality::Staged.label(), "Larry 6 步");
    }

    /// 整段出片和逐镜出片同一道关：合成之后改过字段就不出（原来只看提示词非空，2026-10-03 审查）。
    #[tokio::test]
    async fn 整段出片_提示词过期就不出() {
        use crate::shot_table::PromptRoute;
        let d = tempfile::tempdir().unwrap();
        合成用的集(d.path(), "a");
        let f = cinema_projects_dir(d.path()).join("p1/episodes/e1/episode.json");
        let mut ep = 读集(d.path());
        for s in ep.storyboard.iter_mut() { s.scene_no = 1; }
        ep.long_take_scenes = vec![1];
        std::fs::write(&f, serde_json::to_string(&ep).unwrap()).unwrap();
        let never = |_: crate::prompts::PromptSlot, _: Vec<(String, String)>| async { Err::<String, _>(Error::runtime("线路 A 不该调模型")) };
        synthesize_episode_prompts(d.path(), "p1", "e1", PromptRoute::A, &[], never, |_, _, _| {}).await.unwrap();
        let windows = |ep: &EpisodeData| {
            let mut shots: Vec<&StoryboardShot> = ep.storyboard.iter().collect();
            shots.sort_by_key(|s| s.shot_num);
            let plan = crate::long_take::plan_with(&shots, crate::long_take::CONTEXT_FRAMES).unwrap();
            let assets = crate::storyboard_gen::load_assets(d.path(), "p1", "e1");
            long_take_windows(d.path(), "p1", "e1", ep, &plan, PromptRoute::A, &assets, &episode_style(d.path(), "p1", ep), &d.path().join("shots")).map(|_| ())
        };
        let ep = 读集(d.path());
        assert!(windows(&ep).is_ok(), "{:?}", windows(&ep).err());
        let mut changed = ep.clone();
        changed.storyboard[1].lighting = "合成之后改的光".into();
        let e = windows(&changed).unwrap_err().to_string();
        assert!(e.contains("改过") && e.contains("镜"), "{e}");
    }

    /// 没合成过时手写的首帧提示词，按场出首帧跑完自动合成不能把它换掉（2026-10-03 审查）。
    #[tokio::test]
    async fn 自动合成不覆盖手写的首帧提示词() {
        use crate::shot_table::{PromptRoute, RoutePrompts};
        let d = tempfile::tempdir().unwrap();
        合成用的集(d.path(), "a");
        let mut ep = 读集(d.path());
        let id = ep.storyboard[0].id.clone();
        ep.storyboard[0].prompts.insert(PromptRoute::A, RoutePrompts { frame: "我手写的首帧".into(), ..Default::default() });
        let f = cinema_projects_dir(d.path()).join("p1/episodes/e1/episode.json");
        std::fs::write(&f, serde_json::to_string(&ep).unwrap()).unwrap();
        let never = |_: crate::prompts::PromptSlot, _: Vec<(String, String)>| async { Err::<String, _>(Error::runtime("线路 A 不该调模型")) };
        synthesize_episode_prompts(d.path(), "p1", "e1", PromptRoute::A, std::slice::from_ref(&id), never, |_, _, _| {}).await.unwrap();
        let p = 读集(d.path()).storyboard[0].prompts[&PromptRoute::A].clone();
        assert!(!p.video.trim().is_empty(), "该合成的照样合成");
        assert_eq!(p.frame, "我手写的首帧");
        // 合成过之后用户再点合成：两条一起换（和合成过的镜头一样，分不出哪份是手写的）
        synthesize_episode_prompts(d.path(), "p1", "e1", PromptRoute::A, std::slice::from_ref(&id), never, |_, _, _| {}).await.unwrap();
        assert_ne!(读集(d.path()).storyboard[0].prompts[&PromptRoute::A].frame, "我手写的首帧");
    }

    #[test]
    fn 设默认形象_主形象从表里去掉() {
        let d = tempfile::tempdir().unwrap();
        let proj = d.path().join("projects").join("p1");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(proj.join("project.json"), r#"{"title":"盛唐","default_looks":{"char_b":"look_1"}}"#).unwrap();
        set_default_look(d.path(), "p1", "char_a", "look_9").unwrap();
        set_default_look(d.path(), "p1", "char_b", "main").unwrap();
        let m = crate::looks::project_default_looks(d.path(), "p1");
        assert_eq!(m.len(), 1);
        assert_eq!(m["char_a"], "look_9");
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(proj.join("project.json")).unwrap()).unwrap();
        assert_eq!(v["title"], "盛唐", "别的字段不能丢");
    }

    /// 形象：项目默认形象、单镜覆盖都要换到参考图和提示词指纹上 —— 换了形象不重新合成就拦下。
    #[test]
    fn 参考图和指纹跟着形象走() {
        use crate::assets::{AssetKind, Scope};
        use crate::shot_table::{RefSlot, RefUsage};
        let d = tempfile::tempdir().unwrap();
        let cfg = d.path();
        let scope = Scope::Project { project_id: "p1".into() };
        let shen = 建到(cfg, scope.clone(), AssetKind::Character, "林婉清");
        let mut store = crate::assets::load_store(cfg, &scope);
        let a = store.assets.iter_mut().find(|a| a.id == shen).unwrap();
        a.identity = "二十八岁女子".into();
        a.description = "白衬衫".into();
        crate::assets::add_variant(cfg, &scope, a, b"main", "png", false, None, true, None).unwrap();
        let tang = crate::assets::add_look(a, "盛唐", "男装官服").unwrap();
        crate::assets::add_variant_to_look(cfg, &scope, a, Some(&tang.id), b"tang", "png", false, None, true, None).unwrap();
        let ball = crate::assets::add_look(a, "礼服", "晚礼服").unwrap();
        crate::assets::add_variant_to_look(cfg, &scope, a, Some(&ball.id), b"ball", "png", false, None, true, None).unwrap();
        crate::assets::save_store(cfg, &scope, &store).unwrap();
        let proj = cfg.join("projects").join("p1");
        std::fs::create_dir_all(&proj).unwrap();
        let read = |p: &Path| std::fs::read(p).unwrap();

        let shots_dir = cfg.join("shots");
        let mut shot = test_shot("s1");
        shot.character_ids = vec![shen.clone()];
        shot.ref_slots = vec![RefSlot { order: 1, asset_id: shen.clone(), usage: RefUsage::Identity, control: "c".into(), not_control: String::new() }];
        let shots = vec![shot.clone()];

        let assets = crate::storyboard_gen::load_assets(cfg, "p1", "e1");
        assert_eq!(read(&render_reference_paths(cfg, "p1", "e1", &shots_dir, &shot, &assets, false)[0]), b"main");
        let h_main = current_hash(&shots, &shot, &assets, "", &[]);

        // 项目默认换成盛唐
        std::fs::write(proj.join("project.json"), format!(r#"{{"default_looks":{{"{shen}":"{}"}}}}"#, tang.id)).unwrap();
        let assets = crate::storyboard_gen::load_assets(cfg, "p1", "e1");
        assert_eq!(read(&render_reference_paths(cfg, "p1", "e1", &shots_dir, &shot, &assets, false)[0]), b"tang");
        let h_tang = current_hash(&shots, &shot, &assets, "", &[]);
        assert_ne!(h_main, h_tang, "换了默认形象，提示词指纹要变");

        // 这一镜覆盖成礼服
        shot.look_overrides.insert(shen.clone(), ball.id.clone());
        assert_eq!(read(&render_reference_paths(cfg, "p1", "e1", &shots_dir, &shot, &assets, false)[0]), b"ball");
        assert_ne!(current_hash(&shots, &shot, &assets, "", &[]), h_tang, "单镜换了形象，提示词指纹要变");
    }

    /// 出片：A / B 和每一「次」各存一个文件、写明线路；过期的提示词拦下；落盘前重读只改这一镜。
    #[test]
    fn 出片按线路存每一次_过期提示词先拦下() {
        let src = include_str!("cinema.rs");
        let at = src.find("async fn render_shot_video_body").unwrap();
        let body = &src[at..at + src[at..].find("\n}\n").unwrap()];
        assert!(body.contains("shots_dir.join(format!(\"{}-{}-{}.mp4\", shot_id_for_path, route_key, crate::jobs::now()))"));
        assert!(body.contains("shot.takes.push(VideoTake {"));
        // **挂的是哪几张要一起存下来。** 只存张数的话，回看历史 take 时
        // 答不出「它用了哪张场景图」—— 而那是唯一想知道的事。
        // 存的必须是 `render_reference_paths` 那一份（和提示词里的
        // <Picture N> 编号同源），不是另外收集的一份。
        assert!(
            body.contains("picture_paths = refs.iter().map(|p| p.to_string_lossy().to_string()).collect();"),
            "出片没把实际挂的参考图路径存进 take",
        );
        assert!(body.contains("picture_paths,"), "take 里没带上 picture_paths");
        // **出这一条时的提示词、字段指纹、画风要一起存** ——
        // 不存的话重新合成之后历史 take 无法复现（2026-09-19 Codex Astra 指出）
        for k in ["prompt: prompt.clone()", "fields_hash: current_hash(", "style: style.chars()"] {
            assert!(body.contains(k), "take 没存「{k}」—— 历史版本就复现不了了");
        }
        assert!(body.contains("render_shot_video_reference_with(&refs, &take_path, &prompt, shot.duration.max(1.0) as f64, width, height, seed, generate_audio, Some(render.quality.tier()), cont)"), "出片设置的画质或衔接参数没传下去");
        assert!(body.contains("if engine == \"h3\" { render.size(&aspect_ratio) }"), "出片设置的尺寸没用上");
        assert!(body.contains("p.fields_hash != current_hash(&storyboard_snapshot, shot, &assets, &style, &long_takes_snapshot)"));
        assert!(body.contains("render_reference_paths(config_dir, &req.project_id, &req.episode_id, &shots_dir, shot, &assets, long_take)"));
        assert!(body.contains("render_shot_video_text("), "一张图都挂不上时要走文生");
        assert!(body.contains("target.takes = takes;"), "落盘前要重读只改这一镜");
        // 老数据没有 takes 也能读
        let old: StoryboardShot = serde_json::from_value(json!({"id":"s","shot_num":1,"scale":"","camera_movement":"","dialogue":"","prompt":"","duration":4.0,"status":"pending"})).unwrap();
        assert!(old.takes.is_empty());
    }

    // ── 角色住在哪一层 ────────────────────────────────────────────────

    /// **提取出来的角色点进去要打得开。**
    ///
    /// 把提取改成落本集之后，`load_cinema_characters` 还只读项目级 ——
    /// 于是刚抽出来的角色点一下就是「这个角色在角色视图里找不到」。
    /// 是我自己造出来的回归。
    #[test]
    fn 本集角色也读得出来() {
        use crate::assets::{AssetKind, Scope};
        let d = tempfile::tempdir().unwrap();
        let cfg = d.path();
        建到(cfg, Scope::Project { project_id: "p1".into() }, AssetKind::Character, "老茶师");
        建到(
            cfg,
            Scope::Episode { project_id: "p1".into(), episode_id: "ep_01".into() },
            AssetKind::Character,
            "小徒弟",
        );
        let names: Vec<String> = load_cinema_characters(cfg, "p1", Some("ep_01"))
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert!(names.contains(&"小徒弟".to_string()), "读不到本集角色：{names:?}");
        assert!(names.contains(&"老茶师".to_string()), "读不到项目角色：{names:?}");
    }

    #[test]
    fn 角色带着自己在哪一层() {
        use crate::assets::{AssetKind, Scope};
        let d = tempfile::tempdir().unwrap();
        let cfg = d.path();
        建到(cfg, Scope::Project { project_id: "p1".into() }, AssetKind::Character, "老茶师");
        建到(
            cfg,
            Scope::Episode { project_id: "p1".into(), episode_id: "ep_01".into() },
            AssetKind::Character,
            "小徒弟",
        );
        let all = load_cinema_characters(cfg, "p1", Some("ep_01")).unwrap();
        let 徒 = all.iter().find(|c| c.name == "小徒弟").unwrap();
        let 师 = all.iter().find(|c| c.name == "老茶师").unwrap();
        assert_eq!(徒.episode_id.as_deref(), Some("ep_01"));
        assert_eq!(师.episode_id, None, "项目级的不该带集号");
    }

    /// **保存要回写原来那一层。**
    ///
    /// 一律写项目级的话，编辑一个本集角色会在项目池里凭空复制一份 ——
    /// 不报错，表现是「资产库里怎么多了个一样的人」。
    #[test]
    fn 存本集角色不会在项目池里多出一份() {
        use crate::assets::{AssetKind, Scope};
        let d = tempfile::tempdir().unwrap();
        let cfg = d.path();
        let ep = Scope::Episode { project_id: "p1".into(), episode_id: "ep_01".into() };
        建到(cfg, ep.clone(), AssetKind::Character, "小徒弟");

        let mut c = load_cinema_characters(cfg, "p1", Some("ep_01")).unwrap()[0].clone();
        c.trigger_words = "1boy".into();
        save_cinema_character(cfg, "p1", c).unwrap();

        let proj = crate::assets::load_store(
            cfg,
            &Scope::Project { project_id: "p1".into() },
        );
        assert!(proj.assets.is_empty(), "本集角色被复制进项目池了：{:?}",
                proj.assets.iter().map(|a| &a.name).collect::<Vec<_>>());
        let local = crate::assets::load_store(cfg, &ep);
        assert_eq!(local.assets.len(), 1);
        assert_eq!(
            local.assets[0].character.as_ref().unwrap().trigger_words,
            "1boy",
            "改动没写进本集那一层"
        );
    }

    // ── 生成设置 ──────────────────────────────────────────────────────

    /// **画幅选项必须都能算出真实尺寸。**
    ///
    /// 另写一张画幅表的话，界面上会出现引擎产不出的比例 —— 提交时才报错，
    /// 而那时人已经填完提示词等了几十秒。LumenX 就是这样：它给 9:16 /
    /// 3:4 / 1:1 / 4:3 / 16:9 五档（云端模型的习惯），接到本机之后
    /// 出片直接报 `WT resolution must be 384p or 512p`。
    #[test]
    fn 每个画幅选项都对得上一个实测尺寸() {
        // **跟 `image_sizes()` 对，不是跟 `size_for_aspect` 对。**
        // 后者是从 `aspect_options()` 推出来的 —— 自己跟自己比永远一致，
        // 哪怕整张表都是编的。第一版就是这么写的，把「另写一张表」
        // 的变异放过去了。
        let real: Vec<(u32, u32)> = crate::studio::image_sizes()
            .into_iter()
            .map(|s| (s.width, s.height))
            .collect();
        let opts = crate::studio::aspect_options();
        assert!(!opts.is_empty(), "一个画幅选项都没有");
        for o in &opts {
            assert!(
                real.contains(&(o.width, o.height)),
                "画幅「{}」给的 {}×{} 不在实测尺寸表里：{real:?}",
                o.id,
                o.width,
                o.height
            );
            assert_eq!(crate::studio::size_for_aspect(&o.id), Some((o.width, o.height)));
        }
    }

    #[test]
    fn 三种画幅都在_竖方横() {
        let ids: Vec<String> =
            crate::studio::aspect_options().into_iter().map(|o| o.id).collect();
        for want in ["portrait", "square", "landscape"] {
            assert!(ids.contains(&want.to_string()), "少了 {want}：{ids:?}");
        }
    }

    /// 默认值必须**本身就是合法的**。不合法的话，新建项目之后
    /// 一次都没进过设置就直接出图，会在最后一刻报错。
    #[test]
    fn 默认生成设置本身是合法的() {
        let d = ModelSettings::default();
        for id in [&d.character_aspect, &d.scene_aspect, &d.prop_aspect] {
            assert!(crate::studio::size_for_aspect(id).is_some(), "默认画幅 {id} 不合法");
        }
        let cat = crate::studio::catalog();
        for e in [&d.t2i_engine, &d.i2i_engine, &d.video_engine] {
            assert!(cat.engines.iter().any(|x| x.id == e), "默认引擎 {e} 不在能力表里");
        }
    }

    /// 首帧按出片画布的比例请求（2026-10-07：原来 16:9 项目一直挑中 1216×832）
    #[test]
    fn 首帧请求尺寸跟出片画布同比例() {
        assert_eq!(frame_request_size((1312, 736)), (1536, 864));
        assert_eq!(frame_request_size((736, 1312)), (864, 1536));
        for canvas in [(1312, 736), (1024, 576), (736, 1312), (512, 512), (1344, 576)] {
            let (w, h) = frame_request_size(canvas);
            let err = (w as f64 / h as f64) / (canvas.0 as f64 / canvas.1 as f64) - 1.0;
            assert!(err.abs() < 0.005, "{canvas:?} → {w}×{h}，比例差 {err:.4}");
            assert!(w % 16 == 0 && h % 16 == 0 && w.max(h) <= 1536);
        }
        // 16:9 项目、720p：不再是 1216×832
        let project: CinemaProject = serde_json::from_value(json!({
            "id":"p1","title":"t","genre":"","aspect_ratio":"16:9",
            "description":"","created_at":0,"updated_at":0,"episode_count":1
        })).unwrap();
        let mut project = project;
        project.model_settings.render.resolution = "720p".into();
        assert_eq!(project.model_settings.render.size(&project.aspect_ratio), (1312, 736));
        let (w, h) = frame_request_size(project.model_settings.render.size(&project.aspect_ratio));
        assert_ne!((w, h), (1216, 832));
    }

    #[test]
    fn 首帧原样发送存下的分镜提示词_负向只在引擎认时发() {
        let dir = tempfile::tempdir().unwrap();
        let mut project: CinemaProject = serde_json::from_value(json!({
            "id":"p1","title":"t","genre":"","aspect_ratio":"1:1",
            "description":"","created_at":0,"updated_at":0,"episode_count":1
        })).unwrap();
        let mut shot: StoryboardShot = serde_json::from_value(json!({
            "id":"s1","shot_num":1,"scale":"特写","camera_movement":"固定",
            "dialogue":"不要画成字幕", "character_id":"", "prompt":"[角色:林婉清] 一个茶杯",
            "negative_prompt":"模糊", "duration":3.0,"status":"pending"
        })).unwrap();
        project.model_settings.i2i_engine = "zimage".into();
        // **没合成过也能出首帧**（2026-10-03）：按当前字段现拼一份线路 A 的画面提示词，不调模型
        let args = studio_frame_args(dir.path(), &project, "ep_01", &shot).expect("没合成过也该能出首帧");
        let composed = args["prompt"].as_str().unwrap();
        assert!(!composed.trim().is_empty(), "现拼的画面提示词是空的");
        assert_eq!(composed, crate::prompt_route_a::synthesize(&crate::prompt_input::ShotContext { shot: &shot, prev: None, assets: &[], style: "", long_take: false }, 0).frame.trim(), "不是按线路 A 现拼的");
        // 存着的线路 A 结果**不拿来凑**：它可能是改字段之前合的
        shot.prompts.insert(crate::shot_table::PromptRoute::A, crate::shot_table::RoutePrompts {
            frame: "线路 A 的分镜提示词（旧的）".into(), ..Default::default() });
        let args = studio_frame_args(dir.path(), &project, "ep_01", &shot).unwrap();
        assert_ne!(args["prompt"], "线路 A 的分镜提示词（旧的）", "拿了存着的线路 A 结果");

        shot.prompts.insert(crate::shot_table::PromptRoute::B, crate::shot_table::RoutePrompts {
            frame: "  特写，一只白瓷茶杯放在木桌上  ".into(), ..Default::default() });
        // SDXL 吃负向、Z-Image 不吃（原来用的是 Chroma，2026-10-01 下架）
        for engine in ["zimage", "sdxl"] {
            project.model_settings.i2i_engine = engine.into();
            let args = studio_frame_args(dir.path(), &project, "ep_01", &shot).unwrap();
            assert_eq!(args["engine"], engine);
            assert_eq!(args["width"], args["height"]);
            assert_eq!(args["prompt"], "特写，一只白瓷茶杯放在木桌上", "必须原样发送，只去掉首尾空白");
            assert_eq!(args.get("negative_prompt").is_some(), engine == "sdxl");
            assert!(args.get("image_path").is_none());
        }
        // 16:9 + 720p：首帧按出片画布 1312×736 的比例请求（以前一直是 1216×832，H3 拉伸后人物宽 22%）
        project.aspect_ratio = "16:9".into();
        project.model_settings.render.resolution = "720p".into();
        let args = studio_frame_args(dir.path(), &project, "ep_01", &shot).unwrap();
        assert_eq!((args["width"].as_u64(), args["height"].as_u64()), (Some(1536), Some(864)));
        project.aspect_ratio = "invalid".into();
        assert!(studio_frame_args(dir.path(), &project, "ep_01", &shot).is_err());
    }

    /// 旧结构的分镜读盘时清空（用户原话「直接丢吧，不要了」）；新结构的原样保留。
    #[test]
    fn 旧结构的分镜读盘时清空_新结构的保留() {
        let d = tempfile::tempdir().unwrap();
        let mk = |version: Option<u32>| -> EpisodeData {
            let mut v = json!({
                "id":"e1","episode_num":1,"title":"t","synopsis":"","script_text":"剧本",
                "storyboard":[{"id":"s1","shot_num":1,"scale":"","camera_movement":"",
                    "dialogue":"","prompt":"p","duration":3.0,"status":"pending"}]
            });
            if let Some(n) = version { v["storyboard_version"] = json!(n); }
            serde_json::from_value(v).unwrap()
        };
        let mut old = mk(None);
        hydrate_episode(d.path(), &mut old);
        assert!(old.storyboard.is_empty(), "旧分镜还在");
        assert_eq!(old.script_text, "剧本", "剧本不能跟着丢");
        let mut current = mk(Some(STORYBOARD_VERSION));
        hydrate_episode(d.path(), &mut current);
        assert_eq!(current.storyboard.len(), 1, "新结构的分镜被误清");
    }

    /// 说话人要按**资产名**对，镜头上存的是资产 id —— 这一层换名字不能出错。
    #[test]
    fn 校验按资产库里的名字对说话人_不认识的资产也报() {
        use crate::assets::{AssetKind, Scope};
        let d = tempfile::tempdir().unwrap();
        let ep_scope = Scope::Episode { project_id: "p1".into(), episode_id: "ep_01".into() };
        let shen = 建到(d.path(), ep_scope.clone(), AssetKind::Character, "林婉清");
        let phone = 建到(d.path(), ep_scope, AssetKind::Prop, "林婉清"); // 同名道具不算出场人物
        let mut shot: StoryboardShot = serde_json::from_value(json!({
            "id":"s1","shot_num":1,"scale":"","camera_movement":"","dialogue":"",
            "prompt":"","duration":8.0,"status":"pending",
            "character_ids":[shen], "prop_ids":[phone],
            "segments":[{"start":0,"end":8,"visual":"打字"}],
            "dialogue_lines":[{"speaker":"林婉清","line":"我还能撑。"}]
        })).unwrap();
        let ep = |shot: &StoryboardShot| EpisodeData {
            long_take_scenes: Vec::new(),
            one_take: false,
            scene_takes: Vec::new(),
            long_take_context_frames: None,
            scene_renders: Vec::new(),
            style_lock: String::new(),
            id: "ep_01".into(), episode_num: 1, title: String::new(), synopsis: String::new(),
            script_text: String::new(), script_doc: None, target_secs: 0,
            storyboard: vec![shot.clone()], storyboard_version: STORYBOARD_VERSION, actor_colors: Vec::new(), final_video_url: None,
        };
        assert_eq!(validate_storyboard(d.path(), "p1", "ep_01", &ep(&shot)), vec![]);

        shot.character_ids = vec![];
        let issues = validate_storyboard(d.path(), "p1", "ep_01", &ep(&shot));
        assert!(issues.iter().any(|i| i.field == "dialogue"), "人不在镜头里却能说话：{issues:?}");

        // 道具 id 被错放进人物列表时，同名道具也不能冒充说话人
        shot.character_ids = vec![phone.clone()];
        let issues = validate_storyboard(d.path(), "p1", "ep_01", &ep(&shot));
        assert!(issues.iter().any(|i| i.field == "dialogue"), "同名道具被当成了出场人物：{issues:?}");

        shot.character_ids = vec![shen.clone(), "char_ghost".into()];
        let issues = validate_storyboard(d.path(), "p1", "ep_01", &ep(&shot));
        assert_eq!(issues.iter().map(|i| i.field).collect::<Vec<_>>(), ["assets"]);
    }

    #[test]
    fn 出片原样发送当前线路存下的视频运动提示词_没合成就拒绝() {
        use crate::shot_table::{PromptRoute, RoutePrompts};
        let mut shot: StoryboardShot = serde_json::from_value(json!({
            "id":"s1","shot_num":7,"scale":"","camera_movement":"","dialogue":"台词",
            "prompt":"[角色:林婉清] 画面","duration":3.0,"status":"pending"
        })).unwrap();
        let err = video_prompt_for_render(&shot, PromptRoute::A).unwrap_err();
        assert!(err.to_string().contains("第 7 镜还没有用线路 A（中文结构）合成视频运动提示词"), "{err}");
        shot.prompts.insert(PromptRoute::B, RoutePrompts { video: "B 的正文".into(), ..Default::default() });
        assert!(video_prompt_for_render(&shot, PromptRoute::A).is_err(), "拿线路 B 的结果凑了线路 A");
        shot.prompts.insert(PromptRoute::A, RoutePrompts {
            video: "\n出场角色：\n现代林婉清\n---\n画面：打字\n".into(), ..Default::default() });
        assert_eq!(video_prompt_for_render(&shot, PromptRoute::A).unwrap(), "出场角色：\n现代林婉清\n---\n画面：打字");
        assert_eq!(video_prompt_for_render(&shot, PromptRoute::B).unwrap(), "B 的正文");
        // 出片、出首帧那两处必须用它，而且用的是项目当前线路
        let src = include_str!("cinema.rs");
        let prod = &src[..src.find("#[cfg(test)]").unwrap()];
        assert!(prod.contains("let prompt = video_prompt_for_render(shot, prompt_route)?;"));
        assert!(prod.contains("prompt_route = p.model_settings.prompt_route;"));
        // 出首帧：选哪份见 `first_frame_source`（手写的 / 没首帧时合成且没过期的线路 B / 其余按镜头表现拼），
        // 出图、预览、导演世界、画布都走 `first_frame_body`（2026-10-03）
        assert!(prod.contains("let body = first_frame_body(config_dir, project, episode_id, shot, &pictured);"));
        assert!(prod.contains("crate::cinema::first_frame_body(config_dir, &project, episode_id, shot, &pictured)") || include_str!("studio_director.rs").contains("crate::cinema::first_frame_body(config_dir, &project, episode_id, shot, &pictured)"));
    }

    fn 带图引用(kind: crate::assets::AssetKind, name: &str) -> crate::assets::ResolvedRef {
        crate::assets::ResolvedRef {
            kind, name: name.into(), asset_id: Some(format!("id_{name}")),
            description: String::new(), anchors: String::new(), avoid: String::new(), lock: String::new(),
            image_path: Some(format!("/img/{name}.png")), image_url: None,
        }
    }

    fn 分镜(extra: serde_json::Value) -> StoryboardShot {
        let mut v = json!({
            "id":"s1","shot_num":1,"scale":"","camera_movement":"","dialogue":"",
            "prompt":"","duration":3.0,"status":"pending"
        });
        v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        serde_json::from_value(v).unwrap()
    }



    /// 用户 2026-09-29：「首帧可以最多 4 张。主要就是场景和角色」—— Qwen 挂 8 张一张约 37 分钟
    #[test]
    fn 首帧最多挂四张_场景和角色优先_道具只补空位() {
        use crate::assets::AssetKind::*;
        assert!(reference_slots_of("qwen21_edit") > FIRST_FRAME_MAX_REFS, "前提：Qwen 本身的槽比 4 多");
        assert_eq!(first_frame_slots("qwen21_edit"), FIRST_FRAME_MAX_REFS);
        let slots = first_frame_slots("qwen21_edit");
        let refs = [带图引用(Scene, "拆机间"), 带图引用(Prop, "激光刀"), 带图引用(Character, "工人"), 带图引用(Character, "竹竿"),
            带图引用(Character, "陆川"), 带图引用(Character, "星澜"), 带图引用(Character, "老秦"), 带图引用(Prop, "平板")];
        let shot = 分镜(json!({ "dialogue_lines": [{"speaker":"竹竿","line":"迟哥"}], "blocking": [{"actor":"陆川"}] }));
        let p = frame_reference_pick(&refs, &shot.dialogue_lines, &shot.blocking, slots);
        assert_eq!(p.names, ["拆机间", "竹竿", "陆川", "工人"], "场景 → 说话的 → 有站位的 → 其余");
        assert_eq!(p.unused, ["星澜", "老秦", "激光刀", "平板"], "没挂上的要说出来");
        // 人少的镜头：场景 + 一个人，道具补进空位（E01 第 1 镜的激光刀靠它才画对）
        let few = [带图引用(Scene, "拆机间"), 带图引用(Character, "陆川"), 带图引用(Prop, "激光刀")];
        assert_eq!(frame_reference_pick(&few, &[], &[], slots).names, ["拆机间", "陆川", "激光刀"]);
    }

    #[test]
    fn 首帧参考图_场景永远占第一个槽() {
        use crate::assets::AssetKind::*;
        let refs = [带图引用(Character, "林婉清"), 带图引用(Scene, "大殿")];
        let p = frame_reference_pick(&refs, &[], &[], 2);
        assert_eq!(p.images, ["/img/大殿.png", "/img/林婉清.png"]);
        assert_eq!(p.names, ["大殿", "林婉清"]);
    }

    #[test]
    fn 首帧参考图_槽不够时说话的人先进_没进的说出来() {
        use crate::assets::AssetKind::*;
        let refs = [带图引用(Scene, "大殿"), 带图引用(Character, "太监"),
            带图引用(Character, "皇帝"), 带图引用(Character, "林婉清（盛唐）")];
        let shot = 分镜(json!({
            "dialogue_lines": [{"speaker":"林婉清","line":"臣女遵旨"}],
            "blocking": [{"actor":"皇帝"}]
        }));
        let p = frame_reference_pick(&refs, &shot.dialogue_lines, &shot.blocking, 3);
        assert_eq!(p.names, ["大殿", "林婉清（盛唐）", "皇帝"], "说话的（造型分身也认）→ 有站位的");
        assert_eq!(p.unused, ["太监"]);
    }

    #[test]
    fn 首帧参考图_画外音不算说话_道具排最后_文生引擎一张不挑() {
        use crate::assets::AssetKind::*;
        let refs = [带图引用(Prop, "玉佩"), 带图引用(Character, "甲"), 带图引用(Character, "乙")];
        let shot = 分镜(json!({"dialogue_lines": [{"speaker":"乙","line":"旁白","off_screen":true}]}));
        let p = frame_reference_pick(&refs, &shot.dialogue_lines, &shot.blocking, 1);
        assert_eq!(p.names, ["甲"]);
        assert_eq!(p.unused, ["乙", "玉佩"], "道具排在人后面，槽不够时说出来");
        let p = frame_reference_pick(&refs, &shot.dialogue_lines, &shot.blocking, 3);
        assert_eq!(p.names, ["甲", "乙", "玉佩"], "槽够时道具也进（E01 第 1 镜激光刀没挂上就被画成了机械臂上的）");
        assert_eq!(frame_reference_pick(&refs, &shot.dialogue_lines, &shot.blocking, 0), FrameRefPick::default());
    }

    #[test]
    fn 参考生图首帧没有带图的资产时说清楚图从哪来() {
        let dir = tempfile::tempdir().unwrap();
        let mut project: CinemaProject = serde_json::from_value(json!({
            "id":"p1","title":"t","genre":"","aspect_ratio":"16:9",
            "description":"","created_at":0,"updated_at":0,"episode_count":1
        })).unwrap();
        project.model_settings.i2i_engine = "krea2_edit".into();
        let err = studio_frame_args(dir.path(), &project, "ep_01", &分镜(json!({"prompt":"一个茶杯","prompts":{"b":{"frame":"一个茶杯"}}}))).unwrap_err();
        assert!(err.to_string().contains("先给资产出图"), "{err}");
        project.model_settings.i2i_engine = "zimage".into();
        let args = studio_frame_args(dir.path(), &project, "ep_01", &分镜(json!({"prompt":"一个茶杯","prompts":{"b":{"frame":"一个茶杯"}}}))).unwrap();
        assert!(args.get("reference_images").is_none());
    }

    /// 建一个 16:9 项目、一集、一镜（`s1`），返回配置目录。
    fn 关键帧夹具(extra: serde_json::Value) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let pdir = cinema_projects_dir(dir.path()).join("p1");
        std::fs::create_dir_all(pdir.join("episodes").join("ep_01")).unwrap();
        std::fs::write(pdir.join("project.json"), json!({"id":"p1","title":"t","genre":"","aspect_ratio":"16:9",
            "description":"","created_at":0,"updated_at":0,"episode_count":1}).to_string()).unwrap();
        let ep = json!({"id":"ep_01","episode_num":1,"title":"t","synopsis":"","script_text":"","storyboard_version":STORYBOARD_VERSION,
            "storyboard":[serde_json::to_value(分镜(extra)).unwrap()]});
        std::fs::write(pdir.join("episodes/ep_01/episode.json"), ep.to_string()).unwrap();
        dir
    }

    fn 存一张图(path: &Path, w: u32, h: u32) {
        image::RgbImage::from_pixel(w, h, image::Rgb([200, 30, 30])).save(path).unwrap();
    }

    /// 用户 2026-09-27：「可以是先生成，也可以是导入首尾帧，或者首帧，根据导入情况自动识别。」
    #[test]
    fn 导入首尾帧_裁成项目画幅_摘掉首帧就回到参考模式() {
        let dir = 关键帧夹具(json!({}));
        let src = dir.path().join("方图.png");
        存一张图(&src, 400, 400);
        let media = crate::assets::MediaInput::Path { path: src.to_string_lossy().to_string() };
        let shots = cinema_projects_dir(dir.path()).join("p1/episodes/ep_01/shots");

        let s = set_shot_keyframe(dir.path(), "p1", "ep_01", "s1", KeyframeSlot::First, Some(&media)).unwrap();
        assert_eq!(s.keyframes, Keyframes { first: true, last: false });
        assert_eq!(s.keyframes.label(), "首帧");
        assert_eq!(image::image_dimensions(shots.join("s1.png")).unwrap(), (400, 225), "方图要裁成 16:9，不能拉伸");
        assert!(s.image_url.is_some());

        let s = set_shot_keyframe(dir.path(), "p1", "ep_01", "s1", KeyframeSlot::Last, Some(&media)).unwrap();
        assert_eq!(s.keyframes.label(), "首尾帧");
        assert!(s.end_image_url.as_deref().is_some_and(|u| u.ends_with("s1_end.png")));
        // 存进 episode.json 的也是这个（提示词指纹读它）
        let disk = read_episode(dir.path(), "p1", "ep_01").unwrap();
        assert_eq!(disk.storyboard[0].keyframes, Keyframes { first: true, last: true });

        let s = set_shot_keyframe(dir.path(), "p1", "ep_01", "s1", KeyframeSlot::First, None).unwrap();
        assert_eq!(s.keyframes, Keyframes { first: false, last: true });
        assert!(!s.keyframes.drives_video(), "只剩尾帧不能按关键帧出");
        assert!(!shots.join("s1.png").exists() && s.image_url.is_none());
        assert!(set_shot_keyframe(dir.path(), "p1", "ep_01", "没有这镜", KeyframeSlot::First, Some(&media)).is_err());
    }

    /// 存下来的只是投影：文件没了，读盘 / 保存 / 出片前一算就跟着变。
    #[test]
    fn 关键帧以磁盘为准() {
        let dir = 关键帧夹具(json!({"keyframes": {"first": true, "last": true}}));
        let ep_dir = cinema_projects_dir(dir.path()).join("p1/episodes/ep_01");
        let mut ep = read_episode(dir.path(), "p1", "ep_01").unwrap();
        assert!(ep.storyboard[0].keyframes.first, "夹具没写进去");
        sync_keyframes(&ep_dir.join("shots"), &mut ep);
        assert_eq!(ep.storyboard[0].keyframes, Keyframes::NONE);
        let mut ep = read_episode(dir.path(), "p1", "ep_01").unwrap();
        hydrate_episode(&ep_dir, &mut ep);
        assert_eq!(ep.storyboard[0].keyframes, Keyframes::NONE);
        let src = include_str!("cinema.rs");
        let save = &src[src.find("pub async fn cinema_save_episode(").unwrap()..];
        assert!(save[..save.find("\n}\n").unwrap()].contains("sync_keyframes("), "整集保存要按磁盘重算，界面那份可能是挂帧之前拿的");
    }

    /// 2026-09-27 E01 第 3 镜：拆机间是 360 全景，整张发给 H3，场景出来是编的。
    #[test]
    fn 全景场景图转成_16比9_透视再发_普通图原样() {
        let dir = tempfile::tempdir().unwrap();
        let pano = dir.path().join("pano.png");
        存一张图(&pano, 400, 200);
        let flat = flat_scene_ref(dir.path(), &pano);
        assert_ne!(flat, pano, "2:1 全景没转");
        assert_eq!(image::image_dimensions(&flat).unwrap(), (1344, 756));
        assert!(flat.starts_with(dir.path().join("cache/pano_flat")), "缓存位置不对：{}", flat.display());
        assert_eq!(flat_scene_ref(dir.path(), &pano), flat, "第二次要命中缓存");
        let normal = dir.path().join("normal.png");
        存一张图(&normal, 320, 180);
        assert_eq!(flat_scene_ref(dir.path(), &normal), normal);
        assert!(is_equirect(1774, 887) && !is_equirect(1344, 756) && !is_equirect(1024, 1024));
    }

    #[test]
    fn 裁到画幅_居中裁不拉伸() {
        // 左右两条黑边、中间白：裁成方图后应该全白
        let mut img = image::RgbImage::from_pixel(300, 100, image::Rgb([255, 255, 255]));
        for x in (0..100).chain(200..300) {
            for y in 0..100 {
                img.put_pixel(x, y, image::Rgb([0, 0, 0]));
            }
        }
        let out = fit_to_frame(image::DynamicImage::ImageRgb8(img), (50, 50)).to_rgb8();
        assert_eq!(out.dimensions(), (50, 50));
        assert!(out.pixels().all(|p| p.0[0] > 200), "裁的不是中间");
        let tall = fit_to_frame(image::DynamicImage::ImageRgb8(image::RgbImage::new(90, 160)), (160, 90));
        assert_eq!((tall.width(), tall.height()), (160, 90));
    }

    /// 尾帧照着首帧出：首帧是第 1 张参考图、尺寸跟首帧；文生引擎换成参考生图。
    #[test]
    fn 尾帧参数_首帧当第一张参考_没有首帧不出() {
        let dir = 关键帧夹具(json!({"camera_movement":"push_in","scale":"close_up",
            "segments":[{"start":0.0,"end":3.0,"title":"a","visual":"她睁眼"},{"start":3.0,"end":5.0,"title":"b","visual":"她盯着镜头，红光刃停在颈侧"}]}));
        let mut project: CinemaProject = serde_json::from_str(&std::fs::read_to_string(cinema_projects_dir(dir.path()).join("p1/project.json")).unwrap()).unwrap();
        project.model_settings.i2i_engine = "zimage".into();
        let shots = cinema_projects_dir(dir.path()).join("p1/episodes/ep_01/shots");
        let ep = read_episode(dir.path(), "p1", "ep_01").unwrap();
        let err = end_frame_args(dir.path(), &project, "ep_01", &shots, &ep.storyboard[0]).unwrap_err();
        assert!(err.to_string().contains("还没有首帧"), "{err}");
        std::fs::create_dir_all(&shots).unwrap();
        存一张图(&shots.join("s1.png"), 320, 180);
        let args = end_frame_args(dir.path(), &project, "ep_01", &shots, &ep.storyboard[0]).unwrap();
        assert_eq!(args["engine"], "qwen21_edit", "文生引擎挂不了首帧");
        assert!(args["reference_images"][0].as_str().unwrap().ends_with("s1.png"));
        assert_eq!((args["width"].as_u64(), args["height"].as_u64()), (Some(320), Some(180)));
        let prompt = args["prompt"].as_str().unwrap();
        assert!(prompt.contains("Image 1 is the OPENING frame") && prompt.contains("「她盯着镜头，红光刃停在颈侧」") && prompt.contains("push_in"), "{prompt}");
        // 景别是这一镜**开始**时的：推镜推完不再是这个景别，不能写成「结束时是 close_up」和结尾画面打架
        // （2026-09-28 第 8 镜：写着 full_body at the end，结尾画面却是「推近停在半身近景」）
        assert!(prompt.contains("the shot starts as close_up") && !prompt.contains("at the end."), "{prompt}");
        // 看实际发送：和出图同一个函数拼，引擎、提示词、图一样
        let pv = keyframe_preview(dir.path(), "p1", "ep_01", &ep.storyboard[0].id, KeyframeSlot::Last);
        if let Ok(pv) = pv {
            assert_eq!(pv.images.len(), args["reference_images"].as_array().unwrap().len());
        }
    }

    #[test]
    fn 参考生图首帧_图按槽位发出去_提示词原样发送() {
        use crate::assets::{Asset, AssetKind, MediaVariant, Scope};
        let dir = tempfile::tempdir().unwrap();
        let scope = Scope::Global;
        let media = scope.media_dir(dir.path());
        std::fs::create_dir_all(&media).unwrap();
        let mut store = crate::assets::load_store(dir.path(), &scope);
        for (kind, name) in [(AssetKind::Character, "林婉清"), (AssetKind::Scene, "大殿")] {
            let mut a = Asset::new(kind, name);
            a.description = format!("{name}的外貌描述");
            std::fs::write(media.join(format!("{name}.png")), b"x").unwrap();
            a.unit.image_variants.push(MediaVariant {
                id: "v".into(), rel_path: format!("{name}.png"), created_at: 0,
                prompt_used: None, favorited: false, from_upload: false, role: None,
            });
            a.unit.selected_image_id = Some("v".into());
            store.assets.push(a);
        }
        crate::assets::save_store(dir.path(), &scope, &store).unwrap();

        let mut project: CinemaProject = serde_json::from_value(json!({
            "id":"p1","title":"t","genre":"","aspect_ratio":"16:9",
            "description":"","created_at":0,"updated_at":0,"episode_count":1
        })).unwrap();
        project.model_settings.i2i_engine = "qwen21_edit".into();
        let shot = 分镜(json!({"prompt":"[角色:林婉清]跪在[场景:大殿]中央","prompts":{"b":{"frame":"合成好的分镜提示词"}}}));
        let args = studio_frame_args(dir.path(), &project, "ep_01", &shot).unwrap();
        let imgs: Vec<String> = serde_json::from_value(args["reference_images"].clone()).unwrap();
        assert_eq!(imgs.len(), 2);
        assert!(imgs[0].ends_with("大殿.png") && imgs[1].ends_with("林婉清.png"), "{imgs:?}");
        // 合成好的分镜提示词原样在后面；前面只加一段「第 N 张是谁」，和发出去的图是同一份挑选结果
        let prompt = args["prompt"].as_str().unwrap();
        assert!(prompt.ends_with("\n\n合成好的分镜提示词"), "{prompt}");
        assert!(prompt.starts_with("Reference images:\nImage 1 is the location「大殿」") && prompt.contains("Image 2 is the person「林婉清」"), "{prompt}");
    }

    /// 线路 B 的 `<Subject N>` 出首帧时换成名字，编号对上合成那一刻的关键帧状态。
    ///
    /// 2026-10-03 EP02：正文原样发给 Qwen 2.1，Subject 编号和 Image 编号是两套，正文里没有名字 ——
    /// 第 17 镜陆川没了、铁头站到了中间，第 12、15、18 镜同一个人画了两遍。
    #[test]
    fn 首帧提示词里的主体标签换成名字_对上参考图编号() {
        use crate::assets::{Asset, AssetKind, MediaVariant, Scope};
        let dir = 关键帧夹具(json!({}));
        let scope = Scope::Global;
        let media = scope.media_dir(dir.path());
        std::fs::create_dir_all(&media).unwrap();
        let mut store = crate::assets::load_store(dir.path(), &scope);
        let mut id = std::collections::HashMap::new();
        for (kind, name) in [(AssetKind::Character, "星澜"), (AssetKind::Character, "陆川"), (AssetKind::Scene, "铁皮屋"),
                             (AssetKind::Character, "铁头"), (AssetKind::Character, "打手乙")] {
            let mut a = Asset::new(kind, name);
            a.description = format!("{name}的外貌描述");
            std::fs::write(media.join(format!("{name}.png")), b"x").unwrap();
            a.unit.image_variants.push(MediaVariant {
                id: "v".into(), rel_path: format!("{name}.png"), created_at: 0,
                prompt_used: None, favorited: false, from_upload: false, role: None,
            });
            a.unit.selected_image_id = Some("v".into());
            id.insert(name, a.id.clone());
            store.assets.push(a);
        }
        crate::assets::save_store(dir.path(), &scope, &store).unwrap();
        // 槽位顺序 = Subject 编号：1 星澜、2 陆川、3 铁皮屋、4 铁头、5 打手乙
        let slot = |order: u8, name: &str, usage: &str| json!({"order":order,"asset_id":id[name],"usage":usage,"control":"","not_control":""});
        let frame = "<Subject 2> grips <Subject 1>'s wrist inside <Subject 3>; <Subject 4> and <Subject 5> block the door. <Subject 2> leans in.";
        let write = |keyframes: Keyframes, frame: &str, video: &str, hash: &str| {
            let ep_file = cinema_projects_dir(dir.path()).join("p1/episodes/ep_01/episode.json");
            let mut ep: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&ep_file).unwrap()).unwrap();
            ep["storyboard"][0] = serde_json::to_value(分镜(json!({
                "character_ids":[id["星澜"],id["陆川"],id["铁头"],id["打手乙"]], "scene_id":id["铁皮屋"],
                "ref_slots":[slot(1,"星澜","identity"),slot(2,"陆川","identity"),slot(3,"铁皮屋","geography"),slot(4,"铁头","identity"),slot(5,"打手乙","identity")],
                "segments":[{"start":0.0,"end":3.0,"title":"a","visual":"镜头表里的开场画面"}],
                "keyframes":keyframes, "prompts":{"b":{"frame":frame,"video":video,"fields_hash":hash}}
            }))).unwrap();
            std::fs::write(&ep_file, ep.to_string()).unwrap();
            read_episode(dir.path(), "p1", "ep_01").unwrap()
        };
        let mut project: CinemaProject = serde_json::from_str(&std::fs::read_to_string(cinema_projects_dir(dir.path()).join("p1/project.json")).unwrap()).unwrap();
        project.model_settings.i2i_engine = "qwen21_edit".into();
        project.model_settings.prompt_route = crate::shot_table::PromptRoute::B;
        let hash_with = |ep: &EpisodeData, kf: Keyframes| {
            let mut s = ep.storyboard[0].clone();
            s.keyframes = kf;
            current_hash(&ep.storyboard, &s, &crate::storyboard_gen::load_assets(dir.path(), "p1", "ep_01"), &episode_style(dir.path(), "p1", ep), &[])
        };
        let body_of = |ep: &EpisodeData| {
            let args = studio_frame_args(dir.path(), &project, "ep_01", &ep.storyboard[0]).unwrap();
            args["prompt"].as_str().unwrap().split_once("\n\n").unwrap().1.to_string()
        };
        let first = Keyframes { first: true, last: false };

        // 1. 没首帧时合成、之后没改过：用它，标签换成名字
        let ep = write(Keyframes::NONE, frame, "v", "");
        let fresh = hash_with(&ep, Keyframes::NONE);
        let ep = write(Keyframes::NONE, frame, "v", &fresh);
        let args = studio_frame_args(dir.path(), &project, "ep_01", &ep.storyboard[0]).unwrap();
        let prompt = args["prompt"].as_str().unwrap();
        let (legend, body) = prompt.split_once("\n\n").unwrap();
        assert!(!has_h3_tags(body), "{body}");
        // 挂上图的：第一次出现写「（Image k）」，k 和图例里那一行是同一个号
        let pictured: Vec<(usize, &str)> = legend.lines().filter_map(|l| {
            let k = l.strip_prefix("Image ")?.split_once(' ')?.0.parse().ok()?;
            Some((k, l.split('「').nth(1)?.split('」').next()?))
        }).collect();
        assert_eq!(pictured.len(), 4, "{legend}");
        for (k, name) in &pictured {
            assert_eq!(body.matches(&format!("「{name}」 (Image {k})")).count(), 1, "「{name}」该是 Image {k}：{body}");
        }
        // 槽位满了没挂上图的那个人写描述，不然模型不知道他长什么样；第二次出现只写名字
        let dropped = ["星澜", "陆川", "铁头", "打手乙"].into_iter().find(|n| !pictured.iter().any(|(_, p)| p == n)).unwrap();
        assert!(body.contains(&format!("「{dropped}」 ({dropped}的外貌描述)")), "{body}");
        assert!(body.starts_with("「陆川」 (") && body.contains("「陆川」 leans in"), "{body}");
        assert!(body.contains("「星澜」 (Image") && body.contains("inside 「铁皮屋」 (Image 1)"), "Subject 1/3 编错了：{body}");

        // 2. 合成后才出的首帧：编号还是合成那一刻的（场景占 3 号），不能按「有首帧」重编
        let ep = write(first, frame, "v", &fresh);
        assert!(body_of(&ep).contains("inside 「铁皮屋」"), "{}", body_of(&ep));

        // 3. 按首帧写的（新顺序下的常态）：正文是「和 <Picture 1> 一致」、不写长相 —— 不用，按镜头表现拼
        let keyframe_mode = hash_with(&ep, first);
        // 不带 <Picture 1> 的也要认出来（不能只靠「换完还有标签」那道兜底）
        let ep = write(first, "<Subject 1> stands left, looking exactly as in the opening frame.", "v", &keyframe_mode);
        let body = body_of(&ep);
        assert!(body.contains("镜头表里的开场画面") && !has_h3_tags(&body) && !body.contains("stands left"), "{body}");

        // 4. 合成后改过字段（指纹对不上）：猜编号会张冠李戴，不用
        let ep = write(Keyframes::NONE, frame, "v", "0000000000000000");
        let body = body_of(&ep);
        assert!(body.contains("镜头表里的开场画面") && !body.contains("grips"), "{body}");
        // 现拼的那份也把没挂上图的人补上描述，只在画外说话的不算
        assert!(body.contains("画面里还有这几个人") && body.contains(&format!("「{dropped}」：{dropped}的外貌描述")), "{body}");

        // 只在画外说话的不在画面里，不补（不然会把内心独白的人画进首帧）
        let mut voice_only = ep.storyboard[0].clone();
        voice_only.dialogue_lines = serde_json::from_value(json!([{"speaker": dropped, "line": "画外的一句", "off_screen": true}])).unwrap();
        let args = studio_frame_args(dir.path(), &project, "ep_01", &voice_only).unwrap();
        assert!(!args["prompt"].as_str().unwrap().contains(&format!("「{dropped}」：")), "{}", args["prompt"]);

        // 5. 认不出的编号：换完还有标签就不用
        let ep = write(Keyframes::NONE, "<Subject 9> stands.", "v", &fresh);
        assert!(body_of(&ep).contains("镜头表里的开场画面"), "{}", body_of(&ep));

        // 6. 手写的（没合成过时在单镜里填的）：原样用
        let ep = write(Keyframes::NONE, "我手写的首帧提示词", "", "");
        assert!(body_of(&ep).starts_with("我手写的首帧提示词"), "{}", body_of(&ep));
    }

    /// **道具的形象也要生效。** 协议铭牌主图是量产款金属牌，星澜那一镜换成「星澜专属」款 ——
    /// 以前出片链路只给角色读形象，这一镜照样挂金属牌主图（2026-09-28 第 8 镜查出来的）。
    /// 专属款还没图时，一张都不挂（按文字画），**不能退回去挂主图**。
    #[test]
    fn 道具换了形象_首帧挂那个形象的图_没图就不挂主图() {
        use crate::assets::{Asset, AssetKind, MediaVariant, Scope};
        let dir = tempfile::tempdir().unwrap();
        let scope = Scope::Global;
        let media = scope.media_dir(dir.path());
        std::fs::create_dir_all(&media).unwrap();
        let variant = |file: &str| MediaVariant { id: file.into(), rel_path: file.into(), created_at: 0, prompt_used: None, favorited: false, from_upload: false, role: None };
        let mut store = crate::assets::load_store(dir.path(), &scope);
        let mut scene = Asset::new(AssetKind::Scene, "拆机间");
        std::fs::write(media.join("拆机间.png"), b"x").unwrap();
        scene.unit.image_variants.push(variant("拆机间.png"));
        scene.unit.selected_image_id = Some("拆机间.png".into());
        let mut plate = Asset::new(AssetKind::Prop, "协议铭牌");
        std::fs::write(media.join("金属牌.png"), b"x").unwrap();
        plate.unit.image_variants.push(variant("金属牌.png"));
        plate.unit.selected_image_id = Some("金属牌.png".into());
        let look = crate::assets::add_look(&mut plate, "星澜专属", "青色晶石").unwrap();
        let plate_id = plate.id.clone();
        store.assets.extend([scene, plate]);
        crate::assets::save_store(dir.path(), &scope, &store).unwrap();

        let mut project: CinemaProject = serde_json::from_value(json!({
            "id":"p1","title":"t","genre":"","aspect_ratio":"16:9",
            "description":"","created_at":0,"updated_at":0,"episode_count":1
        })).unwrap();
        project.model_settings.i2i_engine = "qwen21_edit".into();
        let mut shot = 分镜(json!({"prompt":"[场景:拆机间]","prompts":{"b":{"frame":"f"}}}));
        shot.prop_ids = vec![plate_id.clone()];
        let imgs = |shot: &StoryboardShot| -> Vec<String> {
            serde_json::from_value(studio_frame_args(dir.path(), &project, "ep_01", shot).unwrap()["reference_images"].clone()).unwrap()
        };
        assert!(imgs(&shot).iter().any(|p| p.ends_with("金属牌.png")), "没换形象：挂主图");
        shot.look_overrides.insert(plate_id.clone(), look.id.clone());
        assert!(!imgs(&shot).iter().any(|p| p.ends_with("金属牌.png")), "换成专属款、专属款没图：不能退回去挂金属牌");
        // 专属款有了图就挂它
        let mut store = crate::assets::load_store(dir.path(), &scope);
        std::fs::write(media.join("晶石.png"), b"x").unwrap();
        let p = store.assets.iter_mut().find(|a| a.id == plate_id).unwrap();
        p.looks[0].unit.image_variants.push(variant("晶石.png"));
        p.looks[0].unit.selected_image_id = Some("晶石.png".into());
        crate::assets::save_store(dir.path(), &scope, &store).unwrap();
        let got = imgs(&shot);
        assert!(got.iter().any(|p| p.ends_with("晶石.png")) && !got.iter().any(|p| p.ends_with("金属牌.png")), "{got:?}");
    }

    #[test]
    fn shot_video_model_uses_saved_project_settings() {
        let dir = tempfile::tempdir().unwrap();
        let pdir = cinema_projects_dir(dir.path()).join("p1");
        std::fs::create_dir_all(&pdir).unwrap();
        let project = json!({"id":"p1","title":"t","genre":"","aspect_ratio":"9:16",
            "description":"","created_at":0,"updated_at":0,"episode_count":1});
        std::fs::write(pdir.join("project.json"), project.to_string()).unwrap();
        let mut req = RenderShotReq {
            project_id: "p1".into(), episode_id: "e1".into(), shot_id: "s1".into(),
            engine_config: None,
        };
        assert_eq!(resolve_shot_video_engine(dir.path(), &req, ShotMode::FirstFrame).unwrap(), "h3");
        let mut settings = ModelSettings::default();
        settings.video_engine = "ltx".into();
        save_model_settings(dir.path(), "p1", settings).unwrap();
        assert_eq!(resolve_shot_video_engine(dir.path(), &req, ShotMode::FirstFrame).unwrap(), "ltx");
        assert_eq!(resolve_shot_video_engine(dir.path(), &req, ShotMode::Reference).unwrap(), "h3");

        req.engine_config = Some(CinemaEngineConfig { video_engine: "minimax_h3".into(), ..Default::default() });
        assert_eq!(resolve_shot_video_engine(dir.path(), &req, ShotMode::FirstFrame).unwrap(), "h3");
        req.engine_config.as_mut().unwrap().video_engine = "ltx".into();
        assert!(resolve_shot_video_engine(dir.path(), &req, ShotMode::Reference).is_err());
        std::fs::write(pdir.join("project.json"), "broken json").unwrap();
        req.engine_config = None;
        assert!(resolve_shot_video_engine(dir.path(), &req, ShotMode::FirstFrame).is_err());
    }


    /// **`_last.png` 和 `_end.png` 含义相反，别混。**
    ///
    /// - `_last.png`：渲完之后抽出来的，给**下一镜**接续用（结果）
    /// - `_end.png`：渲之前放进去的，告诉**这一镜**该落到哪（输入）
    ///
    /// 混了的后果是拿上一次的结果当这一次的目标 —— 画面会往回走，
    /// 而且不报错。
    #[test]
    fn 尾帧和末帧是两个文件() {
        let dir = std::path::Path::new("/tmp/shots");
        assert_eq!(last_frame_path(dir, "s1").file_name().unwrap(), "s1_last.png");
        assert_eq!(end_frame_path(dir, "s1").file_name().unwrap(), "s1_end.png");
        assert_ne!(last_frame_path(dir, "s1"), end_frame_path(dir, "s1"));
    }







    /// **结构化台词压过字符串那份。**
    #[test]
    fn 结构化台词优先且带音色和画外音() {
        let dir = tempfile::tempdir().unwrap();
        let mut shot: StoryboardShot = serde_json::from_value(json!({
            "id":"s1","shot_num":1,"scale":"中景","camera_movement":"固定",
            "dialogue":"这是老的那份字符串",
            "character_id":"","prompt":"x","duration":5.0,"status":"pending"
        })).unwrap();
        shot.dialogue_lines = vec![
            DialogueLine { speaker: "沈星晴".into(), line: "你哪来的？".into(),
                           voice: "年轻女声".into(), emotion: "试探".into(), ..Default::default() },
            DialogueLine { speaker: "旁白".into(), line: "那天之后，她再没回去。".into(),
                           off_screen: true, ..Default::default() },
        ];
        let lines = shot_dialogue_lines(dir.path(), &shot, &crate::assets::RefScope::Anywhere);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].speaker, "沈星晴");
        assert_eq!(lines[0].voice, "年轻女声");
        assert!(lines[1].off_screen, "画外音标记丢了");
        assert!(!lines.iter().any(|l| l.text.contains("老的那份")), "还在读字符串那份");
    }







    #[tokio::test]
    async fn shot_video_model_invalid_project_is_rejected_before_rendering() {
        let dir = tempfile::tempdir().unwrap();
        let pdir = cinema_projects_dir(dir.path()).join("p1");
        let edir = pdir.join("episodes/e1");
        std::fs::create_dir_all(&edir).unwrap();
        let project = json!({"id":"p1","title":"t","genre":"","aspect_ratio":"9:16",
            "description":"","created_at":0,"updated_at":0,"episode_count":1,
            "model_settings":{"video_engine":"unsupported"}});
        std::fs::write(pdir.join("project.json"), project.to_string()).unwrap();
        let episode = json!({
            "id":"e1", "episode_num":1, "title":"t", "synopsis":"", "script_text":"",
            "storyboard":[{"id":"s1","shot_num":1,"scale":"中景","camera_movement":"固定",
                "dialogue":"","character_id":"","prompt":"test","duration":3.0,"status":"pending"}]
        });
        // Deserialize against the actual storage schema before exercising the renderer.
        let ep: EpisodeData = serde_json::from_value(episode).unwrap();
        std::fs::write(edir.join("episode.json"), serde_json::to_string(&ep).unwrap()).unwrap();
        let error = cinema_render_shot_video_with_config_dir(dir.path(), RenderShotReq {
            project_id: "p1".into(), episode_id: "e1".into(), shot_id: "s1".into(), engine_config: None,
        }).await.unwrap_err();
        assert!(error.to_string().contains("unsupported"), "{error}");
        assert!(!edir.join("shots/s1.mp4").exists());
    }

    #[test]
    fn 角色竖_场景横_道具方() {
        let d = ModelSettings::default();
        assert_eq!(d.aspect_for("character"), "portrait");
        assert_eq!(d.aspect_for("scene"), "landscape");
        assert_eq!(d.aspect_for("prop"), "square");
    }

    /// 认不出的类型回角色那档 —— 竖幅在别的类型上只是不好看，不会失败。
    #[test]
    fn 认不出的类型用角色那档() {
        let d = ModelSettings::default();
        assert_eq!(d.aspect_for("怪东西"), d.character_aspect);
    }

    #[test]
    fn 存设置只改这一块_别的字段不动() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        let pdir = cinema_projects_dir(cfg).join("p1");
        std::fs::create_dir_all(&pdir).unwrap();
        let mut proj: CinemaProject = serde_json::from_str(
            r#"{"id":"p1","title":"t","genre":"g","aspect_ratio":"9:16","description":"d",
                "created_at":0,"updated_at":0,"episode_count":7,"cover_image":"cover.png"}"#,
        )
        .unwrap();
        proj.model_settings.t2i_engine = "sdxl".into();
        std::fs::write(pdir.join("project.json"), serde_json::to_string(&proj).unwrap()).unwrap();

        let mut next = ModelSettings::default();
        next.t2i_engine = "chroma".into();
        let out = save_model_settings(cfg, "p1", next).unwrap();

        assert_eq!(out.model_settings.t2i_engine, "chroma");
        assert_eq!(out.episode_count, 7, "别的字段被清掉了");
        assert_eq!(out.cover_image.as_deref(), Some("cover.png"));
        assert_eq!(out.title, "t");
    }

    /// 线路开关要真的存下来；老项目没有这个字段时按 B（和界面默认一致）。
    #[test]
    fn 提示词线路能存盘读回_老项目默认b() {
        let old: CinemaProject = serde_json::from_value(json!({
            "id":"p1","title":"t","genre":"","aspect_ratio":"9:16","description":"",
            "created_at":0,"updated_at":0,"episode_count":1
        })).unwrap();
        assert_eq!(old.model_settings.prompt_route, crate::shot_table::PromptRoute::B);

        let dir = tempfile::tempdir().unwrap();
        let pdir = cinema_projects_dir(dir.path()).join("p1");
        std::fs::create_dir_all(&pdir).unwrap();
        std::fs::write(pdir.join("project.json"), serde_json::to_string(&old).unwrap()).unwrap();
        let next = ModelSettings { prompt_route: crate::shot_table::PromptRoute::A, ..ModelSettings::default() };
        save_model_settings(dir.path(), "p1", next).unwrap();
        let back: CinemaProject = serde_json::from_str(&std::fs::read_to_string(pdir.join("project.json")).unwrap()).unwrap();
        assert_eq!(back.model_settings.prompt_route, crate::shot_table::PromptRoute::A);
    }

    #[test]
    fn 资产引擎不许选参考生图_首帧可以() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        let pdir = cinema_projects_dir(cfg).join("p1");
        std::fs::create_dir_all(&pdir).unwrap();
        let proj: CinemaProject = serde_json::from_str(
            r#"{"id":"p1","title":"t","genre":"","aspect_ratio":"9:16","description":"",
                "created_at":0,"updated_at":0,"episode_count":1}"#,
        ).unwrap();
        std::fs::write(pdir.join("project.json"), serde_json::to_string(&proj).unwrap()).unwrap();
        let mut next = ModelSettings::default();
        next.i2i_engine = "krea2_edit".into();
        assert_eq!(save_model_settings(cfg, "p1", next.clone()).unwrap().model_settings.i2i_engine, "krea2_edit");
        next.t2i_engine = "qwen21_edit".into();
        let err = save_model_settings(cfg, "p1", next).unwrap_err();
        assert!(err.to_string().contains("只能用在分镜首帧"), "{err}");
    }

    /// 前端传了个产不出的画幅时**当场拒绝**，而不是存下来等出图时炸。
    #[test]
    fn 存设置时拒绝不存在的画幅() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        let pdir = cinema_projects_dir(cfg).join("p1");
        std::fs::create_dir_all(&pdir).unwrap();
        let proj: CinemaProject = serde_json::from_str(
            r#"{"id":"p1","title":"t","genre":"","aspect_ratio":"9:16","description":"",
                "created_at":0,"updated_at":0,"episode_count":1,"cover_image":null}"#,
        )
        .unwrap();
        std::fs::write(pdir.join("project.json"), serde_json::to_string(&proj).unwrap()).unwrap();

        let mut bad = ModelSettings::default();
        bad.scene_aspect = "9:16".into(); // 这是 LumenX 的写法，我们没有这一档
        let err = save_model_settings(cfg, "p1", bad).unwrap_err().to_string();
        assert!(err.contains("场景"), "没说清是哪一类：{err}");
        assert!(err.contains("9:16"), "没把那个值报出来：{err}");
    }

    /// 老 project.json 没有这一块时读得出来、且是默认值。
    #[test]
    fn 老项目没有生成设置时用默认() {
        let p: CinemaProject = serde_json::from_str(
            r#"{"id":"p1","title":"t","genre":"","aspect_ratio":"9:16","description":"",
                "created_at":0,"updated_at":0,"episode_count":1,"cover_image":null}"#,
        )
        .unwrap();
        assert_eq!(p.model_settings, ModelSettings::default());
    }

    // ── 合成一集 ──────────────────────────────────────────

    /// 出片每次落 `{id}-b-{时间戳}.mp4`，拼接必须认分镜上记的那一版；
    /// 老数据不带扩展名；两样都没有才退回 `{id}.mp4`。
    #[test]
    fn 合成用分镜选中的那一版() {
        let d = tempfile::tempdir().unwrap();
        let shots = d.path();
        let take = shots.join("s1-b-100.mp4");
        std::fs::write(&take, b"x").unwrap();
        std::fs::write(shots.join("s1.mp4"), b"old").unwrap();
        let mut s = test_shot("s1");
        s.video_url = Some(take.to_string_lossy().into());
        assert_eq!(stitch_source(&s, shots), Some(take.clone()));
        s.video_url = Some(shots.join("s1-b-100").to_string_lossy().into());
        assert_eq!(stitch_source(&s, shots), Some(take), "不带扩展名的老数据");
        s.video_url = None;
        assert_eq!(stitch_source(&s, shots), Some(shots.join("s1.mp4")));
        assert_eq!(stitch_source(&test_shot("s2"), shots), None);
    }

    #[test]
    fn 出片文件比超分缓存新就重做() {
        let d = tempfile::tempdir().unwrap();
        let (src, cache) = (d.path().join("a.mp4"), d.path().join("a_x2.mp4"));
        assert!(!cache_fresh(&src, &cache));
        std::fs::write(&src, b"v1").unwrap();
        assert!(!cache_fresh(&src, &cache), "没有缓存");
        std::fs::write(&cache, b"x2").unwrap();
        let f = std::fs::File::options().write(true).open(&cache).unwrap();
        f.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5)).unwrap();
        assert!(cache_fresh(&src, &cache));
        let f = std::fs::File::options().write(true).open(&src).unwrap();
        f.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(60)).unwrap();
        assert!(!cache_fresh(&src, &cache), "出片文件更新过（比如重烧字幕）");
    }

    #[test]
    fn 超分后整集是_1080p() {
        assert_eq!(episode_frame("16:9", false), (1280, 720));
        assert_eq!(episode_frame("16:9", true), (1920, 1080));
        assert_eq!(episode_frame("9:16", true), (1080, 1920));
        assert_eq!(episode_frame("9:16", false), (720, 1280));
    }

    // ── 项目级的默认镜头模式 ──────────────────────────────────────────

    fn test_shot(id: &str) -> StoryboardShot {
        serde_json::from_str(&format!(
            r#"{{"id":"{id}","shot_num":1,"scale":"中景","camera_movement":"固定",
                 "dialogue":"","prompt":"","negative_prompt":null,"duration":3.0,
                 "image_url":null,"video_url":null,"status":"pending"}}"#
        ))
        .expect("测试镜头构造失败")
    }



    /// 老 project.json 没有这两个字段，读出来要有默认值、行为不变。
    #[test]
    fn 老项目缺字段时行为不变() {
        let p: CinemaProject = serde_json::from_str(
            r#"{"id":"p1","title":"t","genre":"","aspect_ratio":"9:16","description":"",
                "created_at":0,"updated_at":0,"episode_count":1,"cover_image":null}"#,
        )
        .expect("老格式读不出来了");
        assert_eq!(p.content_mode, "scripted", "老项目该按有剧本走");
        assert_eq!(p.default_shot_mode, ShotMode::FirstFrame);
        assert_eq!(p.kind, "series");
    }

    /// 末帧文件名的约定：和关键帧同目录，加 `_last` 后缀。
    /// 两处（抽取和读取）必须用同一个函数算，各写一份必然漂。
    #[test]
    fn 末帧路径两处共用一个来源() {
        let d = PathBuf::from("/tmp/shots");
        assert_eq!(last_frame_path(&d, "s2"), d.join("s2_last.png"));
    }

    // ── 分镜的资产绑定（照着 LumenX 的 analyze_text_to_frames 补的） ──

    fn seed_assets(dir: &std::path::Path) {
        use crate::assets::{Asset, AssetKind, Scope, load_store, save_store};
        let proj = Scope::Project { project_id: "p1".into() };
        let mut st = load_store(dir, &proj);
        let mut scene = Asset::new(AssetKind::Scene, "小雨家的房间");
        scene.description = "温暖的午后阳光，木地板，堆着纸箱".into();
        st.assets.push(scene);
        save_store(dir, &proj, &st).unwrap();

        let ep = Scope::Episode { project_id: "p1".into(), episode_id: "e1".into() };
        let mut st = load_store(dir, &ep);
        let mut cat = Asset::new(AssetKind::Character, "小花");
        cat.description = "三米小团，右眼上方有一撮黑毛".into();
        st.assets.push(cat);
        st.assets.push(Asset::new(AssetKind::Prop, "纸箱"));
        save_store(dir, &ep, &st).unwrap();
    }

    fn shot_with(prompt: &str) -> StoryboardShot {
        serde_json::from_value(json!({
            "id": "s1", "shot_num": 1, "scale": "", "camera_movement": "", "dialogue": "",
            "prompt": prompt, "duration": 3.0, "status": "pending"
        }))
        .unwrap()
    }




    #[test]
    fn 标签解析成资产_id_回填进镜头() {
        let dir = tempfile::tempdir().unwrap();
        seed_assets(dir.path());
        let mut shots =
            vec![shot_with("[场景:小雨家的房间]里，[角色:小花]扒着[道具:纸箱]的边缘")];
        let unknown = bind_shot_refs(dir.path(), &mut shots, &crate::assets::RefScope::Anywhere);
        assert!(unknown.is_empty(), "全都在库里，不该有认不出的：{unknown:?}");
        let s = &shots[0];
        assert_eq!(s.character_ids.len(), 1, "角色没回填");
        assert!(s.character_ids[0].starts_with("char_"));
        assert_eq!(s.character_id, s.character_ids[0], "旧的单数字段也要跟着写");
        assert!(s.scene_id.as_deref().unwrap_or("").starts_with("scene_"), "场景没回填");
        assert_eq!(s.prop_ids.len(), 1, "道具没回填");
    }

    /// **这条是整件事的重点。** 模型编的名字和真名字在界面上长得一模一样
    /// （都是一颗彩色圆点），但它挂不上任何参考图，那一镜的人物身份就是
    /// 随机的 —— 而且全程不报错。所以必须在生成完那一刻就报出来。
    #[test]
    fn 库里没有的引用要被报出来而不是静默丢掉() {
        let dir = tempfile::tempdir().unwrap();
        seed_assets(dir.path());
        let mut shots = vec![shot_with("[角色:男 1]走进[场景:小雨家的房间]")];
        let unknown = bind_shot_refs(dir.path(), &mut shots, &crate::assets::RefScope::Anywhere);
        assert_eq!(unknown, vec!["角色：男 1"], "编出来的名字没被报出来");
        assert!(shots[0].character_ids.is_empty(), "解析不到就不该回填 id");
        assert!(shots[0].scene_id.is_some(), "解析得到的那个还是要回填");
    }

    #[test]
    fn 同一个认不出的名字只报一次() {
        let dir = tempfile::tempdir().unwrap();
        seed_assets(dir.path());
        let mut shots = vec![shot_with("[角色:男 1]说话"), shot_with("[角色:男 1]转身")];
        assert_eq!(bind_shot_refs(dir.path(), &mut shots, &crate::assets::RefScope::Anywhere).len(), 1);
    }

    /// 一镜只能有一个场景。模型多写了以第一个为准 ——
    /// 随机挑一个的话，同一份数据每次生成挂的参考图都不一样。
    #[test]
    fn 一镜写了两个场景时取第一个() {
        let dir = tempfile::tempdir().unwrap();
        seed_assets(dir.path());
        use crate::assets::{Asset, AssetKind, Scope, load_store, save_store};
        let proj = Scope::Project { project_id: "p1".into() };
        let mut st = load_store(dir.path(), &proj);
        st.assets.push(Asset::new(AssetKind::Scene, "走廊"));
        save_store(dir.path(), &proj, &st).unwrap();

        let mut shots = vec![shot_with("[场景:小雨家的房间]和[场景:走廊]")];
        bind_shot_refs(dir.path(), &mut shots, &crate::assets::RefScope::Anywhere);
        let want = crate::assets::list_assets(dir.path(), Some(proj))
            .into_iter()
            .find(|v| v.asset.name == "小雨家的房间")
            .unwrap()
            .asset
            .id;
        assert_eq!(shots[0].scene_id.as_deref(), Some(want.as_str()));
    }

    /// 重新生成时旧的绑定必须被清掉，不能留着上一版的 id。
    #[test]
    fn 重新绑定会清掉上一次的结果() {
        let dir = tempfile::tempdir().unwrap();
        seed_assets(dir.path());
        let mut shots = vec![shot_with("空镜，没有任何标签")];
        shots[0].character_ids = vec!["char_stale".into()];
        shots[0].character_id = "char_stale".into();
        shots[0].scene_id = Some("scene_stale".into());
        shots[0].prop_ids = vec!["prop_stale".into()];
        bind_shot_refs(dir.path(), &mut shots, &crate::assets::RefScope::Anywhere);
        assert!(shots[0].character_ids.is_empty(), "旧的角色绑定没清掉");
        assert!(shots[0].character_id.is_empty(), "旧的单数字段没清掉");
        assert!(shots[0].scene_id.is_none(), "旧的场景绑定没清掉");
        assert!(shots[0].prop_ids.is_empty(), "旧的道具绑定没清掉");
    }

    // ── 媒体字段：路径 vs base64 ──



    /// **两个画幅都必须是 32 的倍数。**
    ///
    /// 以前写死 720×1280 / 1280×720，而 720 ÷ 32 = 22.5 ——
    /// 两个画幅在出片时都会被 H3 的 `WeeToddH3GenerationConfig` 拒掉
    /// （`width and height must be divisible by 32`），报的是一句
    /// 看不懂的英文。而首帧那条路用的是同一组数，所以两边一起错。
    #[test]
    fn 出片尺寸对齐到32() {
        for aspect in ["9:16", "16:9", "1:1", "乱写的"] {
            let (w, h) = shot_render_size(aspect);
            assert_eq!((w % 32, h % 32), (0, 0), "{aspect} 得到 {w}x{h}");
            assert!(w >= 32 && h >= 32, "{aspect} 得到 {w}x{h}");
        }
    }

    /// 画幅方向不能丢 —— 竖屏项目出成横的没人会觉得是"对齐"造成的。
    #[test]
    fn 对齐之后方向不变() {
        let (w, h) = shot_render_size("9:16");
        assert!(h > w, "竖屏出成了 {w}x{h}");
        let (w, h) = shot_render_size("16:9");
        assert!(w > h, "横屏出成了 {w}x{h}");
    }


    /// **界面要靠这个值把缓存打穿。**
    ///
    /// 首帧固定写 `shots/<镜头 id>.png` —— 重出一次内容换了、路径没变。
    /// `<img src>` 字符串一模一样，React 不会重设 src，浏览器也就不会
    /// 发请求，HTTP 那层的 ETag 根本轮不到。表现是要切一次页面才看到新图。
    #[test]
    fn 媒体版本号跟着文件走() {
        let d = tempfile::tempdir().unwrap();
        let shots = d.path().join("shots");
        std::fs::create_dir_all(&shots).unwrap();
        let img = shots.join("s1.png");
        std::fs::write(&img, b"first").unwrap();

        let mk = || -> EpisodeData {
            serde_json::from_value(json!({
                "id":"e1","episode_num":1,"title":"t","synopsis":"","script_text":"","storyboard_version":3,
                "storyboard":[{
                    "id":"s1","shot_num":1,"scale":"中景","camera_movement":"固定",
                    "dialogue":"","character_id":"","prompt":"p","duration":3.0,"status":"ready"
                }]
            })).unwrap()
        };

        let mut a = mk();
        hydrate_episode(d.path(), &mut a);
        let rev_a = a.storyboard[0].image_rev;
        assert!(rev_a > 0, "有文件却没算出版本号");

        // 重出一次
        std::thread::sleep(std::time::Duration::from_millis(5));
        std::fs::write(&img, b"second").unwrap();
        let mut b = mk();
        hydrate_episode(d.path(), &mut b);
        assert_ne!(rev_a, b.storyboard[0].image_rev, "重出之后版本号没变，界面不会刷新");
    }

    /// 文件不在就是 0 —— 它只用来打穿缓存，不参与任何判断。
    #[test]
    fn 没有文件时版本号是零() {
        let d = tempfile::tempdir().unwrap();
        let mut ep: EpisodeData = serde_json::from_value(json!({
            "id":"e1","episode_num":1,"title":"t","synopsis":"","script_text":"","storyboard_version":3,
            "storyboard":[{
                "id":"s1","shot_num":1,"scale":"中景","camera_movement":"固定",
                "dialogue":"","character_id":"","prompt":"p","duration":3.0,"status":"pending"
            }]
        })).unwrap();
        hydrate_episode(d.path(), &mut ep);
        assert_eq!(ep.storyboard[0].image_rev, 0);
        assert_eq!(ep.storyboard[0].video_rev, 0);
    }

    /// **每次读盘重算，不信任存下来的值。** 存下来的会漂，
    /// 而漂了之后表现是"有的镜头刷新、有的不刷新"，极难查。
    #[test]
    fn 存下来的旧版本号会被覆盖() {
        let d = tempfile::tempdir().unwrap();
        let mut ep: EpisodeData = serde_json::from_value(json!({
            "id":"e1","episode_num":1,"title":"t","synopsis":"","script_text":"","storyboard_version":3,
            "storyboard":[{
                "id":"s1","shot_num":1,"scale":"中景","camera_movement":"固定",
                "dialogue":"","character_id":"","prompt":"p","duration":3.0,"status":"ready",
                "image_rev": 999999
            }]
        })).unwrap();
        assert_eq!(ep.storyboard[0].image_rev, 999999);
        hydrate_episode(d.path(), &mut ep);
        assert_eq!(ep.storyboard[0].image_rev, 0, "没有重算，用了存下来的旧值");
    }

    /// **磁盘上有约定文件就用路径。** 这是把 base64 请出 `episode.json`
    /// 的那一步：实测单集 1.82 MB、99.5% 是 base64，而那还只是一张首帧。
    #[test]
    fn 有文件时统一成绝对路径() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("s1.png");
        std::fs::write(&f, b"png").unwrap();
        for cur in [
            Some("data:image/png;base64,AAAA".to_string()),
            Some("/projects/x/s1.png".to_string()),
            None,
        ] {
            assert_eq!(
                resolve_media_field(cur, &f),
                Some(f.to_string_lossy().to_string())
            );
        }
    }

    /// **文件没了的时候不能扔 base64** —— 那是这份像素唯一的副本。
    #[test]
    fn 文件不在时保留内联数据() {
        let d = tempfile::tempdir().unwrap();
        let gone = d.path().join("missing.png");
        let inline = Some("data:image/png;base64,AAAA".to_string());
        assert_eq!(resolve_media_field(inline.clone(), &gone), inline);
    }

    /// 老的 `/projects/xxx.png` 指不到任何东西，留着只会显示一张碎图 ——
    /// 而碎图看起来像「文件坏了」，不像「这镜还没生成」。
    #[test]
    fn 指不到东西的老路径置空() {
        let d = tempfile::tempdir().unwrap();
        let gone = d.path().join("missing.png");
        assert_eq!(resolve_media_field(Some("/projects/x/s1.png".into()), &gone), None);
        assert_eq!(resolve_media_field(Some("   ".into()), &gone), None);
        assert_eq!(resolve_media_field(None, &gone), None);
    }

    /// 字段里已经是另一个**存在的**绝对路径时别丢掉（约定文件还没生成的中间态）。
    #[test]
    fn 已有的有效路径不会被抹掉() {
        let d = tempfile::tempdir().unwrap();
        let other = d.path().join("other.png");
        std::fs::write(&other, b"png").unwrap();
        let gone = d.path().join("missing.png");
        assert_eq!(
            resolve_media_field(Some(other.to_string_lossy().to_string()), &gone),
            Some(other.to_string_lossy().to_string())
        );
    }

    /// 整集一起过一遍：三种媒体各按各的约定文件名找。
    #[test]
    fn 整集的媒体字段一起归一() {
        let d = tempfile::tempdir().unwrap();
        let shots = d.path().join("shots");
        let audio = d.path().join("audio");
        std::fs::create_dir_all(&shots).unwrap();
        std::fs::create_dir_all(&audio).unwrap();
        std::fs::write(shots.join("s1.png"), b"p").unwrap();
        std::fs::write(shots.join("s1.mp4"), b"v").unwrap();
        std::fs::write(audio.join("s1.wav"), b"a").unwrap();

        let mut ep: EpisodeData = serde_json::from_value(json!({
            "id":"e1","episode_num":1,"title":"t","synopsis":"","script_text":"","storyboard_version":3,
            "storyboard":[{
                "id":"s1","shot_num":1,"scale":"中景","camera_movement":"固定",
                "dialogue":"","character_id":"","prompt":"p","duration":3.0,"status":"ready",
                "image_url":"data:image/png;base64,AAAA",
                "video_url":"data:video/mp4;base64,BBBB",
                "audio_url":"data:audio/wav;base64,CCCC"
            }]
        })).unwrap();
        hydrate_episode(d.path(), &mut ep);
        let s = &ep.storyboard[0];
        for v in [&s.image_url, &s.video_url, &s.audio_url] {
            let v = v.as_deref().unwrap();
            assert!(!v.starts_with("data:"), "base64 没被换掉：{v}");
            assert!(std::path::Path::new(v).is_file(), "换出来的路径不存在：{v}");
        }
        assert!(s.image_url.as_deref().unwrap().ends_with("s1.png"));
        assert!(s.video_url.as_deref().unwrap().ends_with("s1.mp4"));
        assert!(s.audio_url.as_deref().unwrap().ends_with("s1.wav"));
    }

    /// 拿**本机真实的项目目录**跑一遍挑封面 —— 单测用的是造出来的目录，
    /// 证明不了「你那个项目为什么没封面」。
    ///
    /// `cargo test --lib 真实项目的封面 -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn 真实项目的封面() {
        // 先验界面真正走的那条路
        if let Some(cfg) = dirs::config_dir().map(|d| d.join("com.anima.desktop")) {
            for it in workspace_items(&cfg) {
                eprintln!(
                    "[workspace] {} → cover={:?}",
                    it.project.id, it.project.cover_image
                );
            }
        }
        let root = dirs::config_dir()
            .map(|d| d.join("com.anima.desktop/projects"))
            .expect("拿不到配置目录");
        let Ok(entries) = std::fs::read_dir(&root) else {
            eprintln!("没有 {root:?}");
            return;
        };
        for e in entries.flatten() {
            let d = e.path();
            if !d.is_dir() {
                continue;
            }
            eprintln!("--- {:?}", d.file_name().unwrap());
            let eps = d.join("episodes");
            eprintln!("    episodes 存在: {}", eps.is_dir());
            if let Ok(list) = std::fs::read_dir(&eps) {
                for ep in list.flatten() {
                    let p = ep.path();
                    if !p.is_dir() { continue; }
                    let has_json = p.join("episode.json").is_file();
                    let shots = std::fs::read_dir(p.join("shots"))
                        .map(|r| r.flatten().filter(|f| f.path().extension().is_some_and(|x| x=="png")).count())
                        .unwrap_or(0);
                    eprintln!("    {:?} episode.json={} shots的png={}", p.file_name().unwrap(), has_json, shots);
                }
            }
            eprintln!("    → 挑出来的封面: {:?}", pick_project_cover(&d));
        }
    }

    /// **项目卡的封面要自己挑一张。**
    ///
    /// `cover_image` 这个字段一直存在、界面一直在读，但从来没人给它赋值 ——
    /// 于是每个项目卡都是一块纯色渐变。不报错，只是看着像没做完。
    #[test]
    fn 项目封面自动挑第一集第一个有首帧的镜头() {
        let d = tempfile::tempdir().unwrap();
        let ep2 = d.path().join("episodes/ep_02");
        let ep1 = d.path().join("episodes/ep_01");
        std::fs::create_dir_all(ep1.join("shots")).unwrap();
        std::fs::create_dir_all(ep2.join("shots")).unwrap();
        // 第一集：头一镜没首帧，第二镜有 —— 该挑第二镜那张
        std::fs::write(ep1.join("shots/s2.png"), b"x").unwrap();
        std::fs::write(ep2.join("shots/s9.png"), b"x").unwrap();
        let mk = |shots: serde_json::Value| {
            serde_json::json!({
                "id":"e","episode_num":1,"title":"t","script_text":"","synopsis":"","storyboard_version":3,
                "storyboard":shots
            })
            .to_string()
        };
        std::fs::write(ep1.join("episode.json"), mk(json!([
            {"id":"s1","shot_num":1,"scale":"","camera_movement":"","dialogue":"",
             "character_id":"","prompt":"","duration":3.0,"status":"pending"},
            {"id":"s2","shot_num":2,"scale":"","camera_movement":"","dialogue":"",
             "character_id":"","prompt":"","duration":3.0,"status":"pending"}
        ]))).unwrap();
        std::fs::write(ep2.join("episode.json"), mk(json!([
            {"id":"s9","shot_num":1,"scale":"","camera_movement":"","dialogue":"",
             "character_id":"","prompt":"","duration":3.0,"status":"pending"}
        ]))).unwrap();

        let cover = pick_project_cover(d.path()).expect("没挑出封面");
        assert!(cover.ends_with("s2.png"), "挑错了：{cover}");
        // **必须是真路径** —— 拿相对路径当媒体 URL 用，界面上又是一张
        // 读不出来的图（组装页那个成片刚踩过）
        assert!(std::path::Path::new(&cover).is_file(), "不是真路径：{cover}");

        // 第一集一张首帧都没有时，才轮到第二集
        std::fs::remove_file(ep1.join("shots/s2.png")).unwrap();
        let cover = pick_project_cover(d.path()).expect("没挑出封面");
        assert!(cover.ends_with("s9.png"), "第一集没图时该用第二集：{cover}");

        // 一张都没有就是 None，不是空串 —— 空串会让界面渲染一个坏 img
        std::fs::remove_file(ep2.join("shots/s9.png")).unwrap();
        assert_eq!(pick_project_cover(d.path()), None);

        // 没有 episodes 目录也不能炸
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(pick_project_cover(empty.path()), None);
    }

    /// **成片也要走读盘归一化。**
    ///
    /// `cinema_stitch_episode` 存的是 `/projects/<id>/episodes/<id>/final.mp4`
    /// 这种**相对路径**，而界面拿它当媒体 URL 用 —— 组装页那格显示
    /// 「读不出来」，看着像文件坏了，实际文件好好在盘上。
    /// 不报错，所以漏了很久没人发现。
    #[test]
    fn 读盘时成片路径也要换成真路径() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("final.mp4"), b"fake").unwrap();
        let mut ep: EpisodeData = serde_json::from_value(json!({
            "id":"e1","episode_num":1,"title":"第一集","script_text":"","synopsis":"",
            "storyboard":[],
            // 拼接时存下来的那种相对路径
            "final_video_url":"/projects/p1/episodes/e1/final.mp4"
        })).unwrap();
        hydrate_episode(d.path(), &mut ep);
        let got = ep.final_video_url.as_deref().expect("成片路径被丢了");
        assert!(
            std::path::Path::new(got).is_file(),
            "换出来的不是真路径：{got} —— 界面上会显示「读不出来」"
        );
        assert!(got.ends_with("final.mp4"));

        // **文件不在时要给 None，不能把那个假路径留着** ——
        // 留着的话界面会以为有成片、然后渲染一个读不出来的播放器
        let d2 = tempfile::tempdir().unwrap();
        let mut ep2: EpisodeData = serde_json::from_value(json!({
            "id":"e1","episode_num":1,"title":"t","script_text":"","synopsis":"",
            "storyboard":[],
            "final_video_url":"/projects/p1/episodes/e1/final.mp4"
        })).unwrap();
        hydrate_episode(d2.path(), &mut ep2);
        assert_eq!(ep2.final_video_url, None, "文件不在却报告有成片");

        // 本来就没有的保持没有
        let mut ep3: EpisodeData = serde_json::from_value(json!({
            "id":"e1","episode_num":1,"title":"t","script_text":"","synopsis":"",
            "storyboard":[]
        })).unwrap();
        hydrate_episode(d.path(), &mut ep3);
        // 目录里有 final.mp4 时，即使 json 里没记也该认出来 —— 拼接成功但
        // 写 episode.json 失败过的话，成片不该就此消失
        assert!(ep3.final_video_url.is_some(), "盘上有成片却没认出来");
    }


    // ── 景别 / 运镜 到底进没进提示词 ──

    fn 镜头(prompt: &str, scale: &str, movement: &str) -> StoryboardShot {
        serde_json::from_value(json!({
            "id":"s1","shot_num":1,"scale":scale,"camera_movement":movement,
            "dialogue":"","character_id":"","prompt":prompt,"duration":3.0,"status":"pending"
        })).unwrap()
    }

    /// **每一条运镜都要真的换出一条不同的滤镜。**
    ///
    /// 原来是 `cam.contains("推")` 那套。换成 id 之后那些 contains
    /// 一条都不会命中，全部掉进「固定」分支 —— 视频照出、不报错，
    /// 只是全片没有运镜。这条测试就是钉这件事。
    #[test]
    fn 每条运镜都换出对应的滤镜() {
        let f = |id: &str| ken_burns_filter(id, 90, 720, 1280, 30);
        let 固定 = f("static");

        // 六个方向各不相同，而且都不等于固定
        let 推 = f("push_in");
        let 拉 = f("pull_out");
        let 左 = f("pan_left");
        let 右 = f("pan_right");
        let 上 = f("tilt_up");
        let 下 = f("tilt_down");
        let 方向 = [&推, &拉, &左, &右, &上, &下];
        for (i, a) in 方向.iter().enumerate() {
            assert_ne!(**a, 固定, "第 {i} 个方向掉回固定分支了");
            for b in &方向[i + 1..] {
                assert_ne!(*a, *b, "两个方向出了同一条滤镜");
            }
        }

        // 同轴的两条共用一条滤镜是设计（平移和摇在一张静帧上做不出区别）
        assert_eq!(f("truck_left"), 左);
        assert_eq!(f("truck_right"), 右);
        assert_eq!(f("pedestal_up"), 上);
        assert_eq!(f("pedestal_down"), 下);

        // 真三维的那几条静帧假装不了，落到呼吸感 —— 这是取舍，不是漏
        for id in ["follow", "orbit", "orbit_up", "orbit_down", "handheld"] {
            assert_eq!(f(id), 固定, "{id} 应该落到呼吸感那一档");
        }
        // 认不出的值也走这一档，不能崩
        assert_eq!(f("用户自己打的词"), 固定);
    }

    /// 词表里每一条运镜都得被 `ken_burns_filter` 认识 —— 哪怕是「归到呼吸感」。
    /// 加了新运镜却忘了在这儿分派，表现是那条运镜静默没效果。
    #[test]
    fn 词表里的每条运镜都在滤镜里有归宿() {
        let 固定 = ken_burns_filter("static", 90, 720, 1280, 30);
        let 有方向的 = [
            "push_in", "pull_out", "pan_left", "pan_right", "tilt_up", "tilt_down",
            "truck_left", "truck_right", "pedestal_up", "pedestal_down",
        ];
        for t in &crate::shot_language::group("move").unwrap().items {
            let got = ken_burns_filter(&t.id, 90, 720, 1280, 30);
            if 有方向的.contains(&t.id.as_str()) {
                assert_ne!(got, 固定, "{} 应该有自己的运动方向", t.id);
            }
            assert!(got.contains("zoompan"), "{} 出来的不是个滤镜", t.id);
        }
    }

    fn 词(gid: &str, id: &str) -> &'static crate::shot_language::Term {
        crate::shot_language::term_in(gid, id).unwrap()
    }









}

// ---------------------------------------------------------------------------
// 剧本 → 实体（Step 1 → Step 3 的那根线）
// ---------------------------------------------------------------------------

/// 读剧本抽角色/场景/道具，**只返回预览，不入库**。
///
/// 一次抽出来几十条直接灌进资产库，等于把资产库变成垃圾场。
/// 让人先看一眼、勾一勾，再入库 —— 和「创作台产物不自动囤」是同一条原则。
#[tauri::command]
pub async fn cinema_extract_entities(
    state: State<'_, AppState>,
    project_id: String,
    episode_id: String,
) -> Result<Vec<crate::prompts::ExtractedEntity>> {
    extract_entities(&state.config_dir, &state.gateway, &project_id, &episode_id).await
}

/// 读一集剧本，让后台模型认出角色 / 场景 / 道具。**不落盘**。和 Chat 的工具共用。
/// 这个项目（本集、项目、全局库）里已经有同名同类的标 `exists`。
pub async fn extract_entities(
    config_dir: &Path,
    gateway: &crate::Gateway,
    project_id: &str,
    episode_id: &str,
) -> Result<Vec<crate::prompts::ExtractedEntity>> {
    let ep_file = cinema_projects_dir(config_dir)
        .join(project_id)
        .join("episodes")
        .join(episode_id)
        .join("episode.json");
    let ep: EpisodeData = serde_json::from_str(
        &std::fs::read_to_string(&ep_file)
            .map_err(|e| Error::config(format!("读不到分集：{e}")))?,
    )
    .map_err(|e| Error::config(format!("解析分集失败：{e}")))?;

    let script = ep.script_text.trim();
    if script.is_empty() {
        return Err(Error::runtime("这一集还没有剧本 —— 先在第 1 步写点东西"));
    }

    let cfg = crate::prompts::load(config_dir);
    let raw = crate::prompts::run(
        gateway,
        &cfg,
        crate::prompts::PromptSlot::EntityExtraction,
        &[("TEXT", script)],
    )
    .await?;

    let mut entities = crate::prompts::parse_entities(&raw);
    if entities.is_empty() {
        // **把模型原样返回的东西带出去。** 只说「提取失败」的话，
        // 用户不知道是模型没听懂、还是返回了但我们没解析出来。
        let head: String = raw.chars().take(240).collect();
        return Err(Error::runtime(format!(
            "没从模型的回复里解析出实体。它返回的是：{head}"
        )));
    }

    // 标出已经有的，界面上默认不勾。按项目找（本集 → 项目 → 全局库）
    let scope = crate::assets::RefScope::Project { project_id, episode_id: Some(episode_id) };
    for e in entities.iter_mut() {
        let Ok(kind) = crate::studio_ops::parse_kind(&e.kind) else { continue };
        e.exists = crate::assets::find_asset_by_name(config_dir, kind, &e.name, &scope).is_some();
    }
    Ok(entities)
}


/// 存项目的生成设置。
///
/// **只改这一块，其余字段原样读回来再写回去** —— 整份覆盖的话，
/// 前端没带上的字段（比如 `episode_count`、`cover_image`）会被清空，
/// 而且不报错。
#[tauri::command]
pub fn cinema_save_model_settings(
    state: State<'_, AppState>,
    project_id: String,
    settings: ModelSettings,
) -> Result<CinemaProject> {
    save_model_settings(&state.config_dir, &project_id, settings)
}

/// 分镜首帧用参考生图引擎时，这一镜会挑哪几张资产图、哪几张挤不进去。
///
/// **界面不自己排。** 排序规则（场景 → 说话的 → 有站位的）只在
/// [`frame_reference_pick`] 里写一份，出图和提示都读它 —— 两边各写一份的话，
/// 界面说「皇帝的图用上了」而实际发出去的是太监的，又是一次看着有实际没有。
#[tauri::command]
pub fn cinema_frame_reference_pick(
    state: State<'_, AppState>,
    prompt: String,
    dialogue_lines: Vec<DialogueLine>,
    blocking: Vec<StageSubject>,
    engine: String,
) -> FrameRefPick {
    let refs = crate::assets::resolve_refs(&state.config_dir, &prompt);
    frame_reference_pick(&refs, &dialogue_lines, &blocking, first_frame_slots(&engine))
}

/// [`cinema_save_model_settings`] 的实现。
/// 绑定（或解绑，`None`）剧本编辑器里的项目。
#[tauri::command]
pub fn cinema_link_script_project(
    state: State<'_, AppState>,
    project_id: String,
    script_project_id: Option<String>,
) -> Result<CinemaProject> {
    link_script_project(&state.config_dir, &project_id, script_project_id)
}

pub fn link_script_project(config_dir: &Path, project_id: &str, script_project_id: Option<String>) -> Result<CinemaProject> {
    let f = cinema_projects_dir(config_dir).join(project_id).join("project.json");
    // 按裸 JSON 改这一个键 —— 类型没建模的字段原样留着
    let mut v: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&f).map_err(|e| Error::config(format!("读不到项目：{e}")))?,
    )
    .map_err(|e| Error::config(format!("解析项目失败：{e}")))?;
    match script_project_id.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        Some(id) => v["script_project_id"] = serde_json::Value::String(id),
        None => {
            v.as_object_mut().map(|m| m.remove("script_project_id"));
        }
    }
    std::fs::write(&f, serde_json::to_string_pretty(&v).unwrap_or_default())
        .map_err(|e| Error::runtime(format!("写项目失败：{e}")))?;
    serde_json::from_value(v).map_err(|e| Error::config(format!("解析项目失败：{e}")))
}

/// 改整部剧的画幅。只认 [`SERIES_ASPECTS`] 里的几种（含 21:9 宽银幕）。
///
/// 以前画幅只能在新建项目时选，之后界面上**没有任何地方能改**（只在项目页右上角显示一行灰字）——
/// 《回收师》建成了 9:16，出了一版竖屏预告才发现整部剧要 16:9（2026-09-26）。
/// 画幅不进合成提示词（线路 A / B 都不读），只决定首帧、出片、组装的尺寸，
/// 所以已经合成的提示词不用重来，但**已经出的首帧和视频尺寸会对不上**，界面在切换前要说出来。
#[tauri::command]
pub fn cinema_set_aspect_ratio(state: State<'_, AppState>, project_id: String, aspect_ratio: String) -> Result<CinemaProject> {
    set_aspect_ratio(&state.config_dir, &project_id, &aspect_ratio)
}

pub fn set_aspect_ratio(config_dir: &Path, project_id: &str, aspect_ratio: &str) -> Result<CinemaProject> {
    let aspect = aspect_ratio.trim();
    if !SERIES_ASPECTS.contains(&aspect) {
        return Err(Error::config(format!("画幅只能是 {}，收到的是「{aspect}」", SERIES_ASPECTS.join(" / "))));
    }
    let f = cinema_projects_dir(config_dir).join(project_id).join("project.json");
    // 按裸 JSON 改这一个键 —— 类型没建模的字段原样留着（同 link_script_project）
    let mut v: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&f).map_err(|e| Error::config(format!("读不到项目：{e}")))?,
    )
    .map_err(|e| Error::config(format!("解析项目失败：{e}")))?;
    v["aspect_ratio"] = serde_json::Value::String(aspect.to_string());
    std::fs::write(&f, serde_json::to_string_pretty(&v).unwrap_or_default())
        .map_err(|e| Error::runtime(format!("写项目失败：{e}")))?;
    serde_json::from_value(v).map_err(|e| Error::config(format!("解析项目失败：{e}")))
}

pub fn save_model_settings(
    config_dir: &Path,
    project_id: &str,
    settings: ModelSettings,
) -> Result<CinemaProject> {
    let f = cinema_projects_dir(config_dir).join(project_id).join("project.json");
    let mut proj: CinemaProject = serde_json::from_str(
        &std::fs::read_to_string(&f).map_err(|e| Error::config(format!("读不到项目：{e}")))?,
    )
    .map_err(|e| Error::config(format!("解析项目失败：{e}")))?;

    // 画幅必须是真实存在的那几档。前端传了个引擎产不出的值时**当场拒绝** ——
    // 放过去的话要等到出图那一刻才报错，那时人已经等了几十秒。
    for (label, id) in [
        ("角色", &settings.character_aspect),
        ("场景", &settings.scene_aspect),
        ("道具", &settings.prop_aspect),
    ] {
        if crate::studio::size_for_aspect(id).is_none() {
            return Err(Error::config(format!("{label}的画幅「{id}」没有对应的尺寸")));
        }
    }
    // 资产图是从文字出的，那时候还没有图可以参考 —— 选了参考生图引擎，
    // 每一次出资产都会在最后一刻报「至少要一张参考图」。
    if reference_slots_of(&settings.t2i_engine) > 0 {
        return Err(Error::config("资产形象图是从文字出的，不能用参考生图引擎 —— 它只能用在分镜首帧"));
    }

    proj.model_settings = settings;
    proj.updated_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    std::fs::write(&f, serde_json::to_string_pretty(&proj).unwrap_or_default())
        .map_err(|e| Error::runtime(format!("写项目失败：{e}")))?;
    Ok(proj)
}

/// 把勾选的实体建进**系列池**；系列里已有同类同名的并进去（只补空字段）。
/// `episode_only` 里列出的（`类型/名字`）建成本集资产、标「只留本集」。
#[tauri::command]
pub fn cinema_import_entities(
    state: State<'_, AppState>,
    project_id: String,
    episode_id: String,
    entities: Vec<crate::prompts::ExtractedEntity>,
    episode_only: Option<Vec<String>>,
) -> Result<ImportSummary> {
    import_entities(&state.config_dir, &project_id, &episode_id, entities, &episode_only.unwrap_or_default())
}

/// 一次导入的结果。界面上要分开说 —— 「新建 3 个、并入已有 4 个」
/// 和「导入了 7 个」是两件事：后者会让人以为系列里又多了 7 个。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ImportSummary {
    /// 系列里新建的
    pub added: usize,
    /// 系列里已有、并进去的
    pub merged: usize,
    /// 只留本集的
    pub episode_only: usize,
    /// 同一个人的其它造型，加成了形象（不算新角色）
    #[serde(default)]
    pub looks: usize,
}

// ---------------------------------------------------------------------------
// 实体对齐：本集名单 → 系列池
// ---------------------------------------------------------------------------

/// 名字有多像。`100` = 完全相同，`75` = 一方包含另一方，`0` = 不匹配。
///
/// # 为什么是这么粗的启发式，而不是让模型判断
///
/// **这一步的正确性由人兜底，不由模型兜底。** 模型判错了没法解释、
/// 也没法复现；子串匹配错了用户一眼看得出来，而且旁边就是下拉框。
/// 「老茶馆（清晨）」对上「老茶馆」是 75 —— 看到这个数字就知道为什么。
///
/// 照 LumenX 的 `_name_match_confidence`（它源码注释里也写着
/// 「Future: embedding-based」，说明现在这样是有意为之）。
pub fn match_confidence(a: &str, b: &str) -> u8 {
    let a = a.trim().to_lowercase();
    let b = b.trim().to_lowercase();
    if a.is_empty() || b.is_empty() {
        return 0;
    }
    if a == b {
        return 100;
    }
    if a.contains(&b) || b.contains(&a) {
        return 75;
    }
    0
}

/// 一条对齐建议。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AlignSuggestion {
    /// 本集那份的 id
    pub local_id: String,
    pub local_name: String,
    /// `"character"` | `"scene"` | `"prop"`
    pub kind: String,
    /// 系列池里最像的那个，没有就是 `None`
    pub target_id: Option<String>,
    pub target_name: Option<String>,
    pub confidence: u8,
}

/// 给本集每个实体找系列池里最像的那个。
///
/// **只在同类之间比。** 不限类的话，道具「铜壶」会去匹配场景「铜壶」——
/// 名字一样但根本是两个东西，而合并之后镜头引用会指错，不报错。
pub fn suggest_alignment(
    config_dir: &Path,
    project_id: &str,
    episode_id: &str,
) -> Vec<AlignSuggestion> {
    let local = crate::assets::load_store(
        config_dir,
        &crate::assets::Scope::Episode {
            project_id: project_id.to_string(),
            episode_id: episode_id.to_string(),
        },
    );
    let series = crate::assets::load_store(
        config_dir,
        &crate::assets::Scope::Project {
            project_id: project_id.to_string(),
        },
    );

    local
        .assets
        .iter()
        .map(|l| {
            let best = series
                .assets
                .iter()
                .filter(|s| s.kind == l.kind)
                .map(|s| (s, match_confidence(&l.name, &s.name)))
                .filter(|(_, c)| *c > 0)
                .max_by_key(|(_, c)| *c);
            AlignSuggestion {
                local_id: l.id.clone(),
                local_name: l.name.clone(),
                kind: l.kind.id_prefix().to_string(),
                target_id: best.map(|(s, _)| s.id.clone()),
                target_name: best.map(|(s, _)| s.name.clone()),
                confidence: best.map(|(_, c)| c).unwrap_or(0),
            }
        })
        .collect()
}

/// [`cinema_import_entities`] 的实现。命令层只是薄壳 ——
/// `tauri::State` 在单测里造不出来。
pub fn import_entities(
    config_dir: &Path,
    project_id: &str,
    episode_id: &str,
    entities: Vec<crate::prompts::ExtractedEntity>,
    episode_only: &[String],
) -> Result<ImportSummary> {
    use crate::assets::{Asset, AssetKind, CharacterTraits, Scope};
    // **默认进系列池，同名并入已有的。**（2026-09-25 用户定的）
    //
    // 历史：最早这里写 `Scope::Project`，但没有「已经有了吗」这一问，
    // 第二集提取又长出一个同名角色；后来改成先落本集、等「对齐」那一步
    // 决定 —— 对齐一直没做，结果每一集的角色都各是各的，设定卡出在哪一个上都不对。
    //
    // 现在两件事一起做：进系列池 + 同类同名（去掉首尾空白后相等）就并进已有的那一个，
    // 只补它空着的字段。只有用户在提取弹窗里点成「只留本集」的才进本集。
    let series_scope = Scope::Project { project_id: project_id.to_string() };
    let ep_scope = Scope::Episode { project_id: project_id.to_string(), episode_id: episode_id.to_string() };
    let mut series = crate::assets::load_store(config_dir, &series_scope);
    let mut local = crate::assets::load_store(config_dir, &ep_scope);
    let mut sum = ImportSummary::default();
    let mut local_dirty = false;

    let build = |e: &crate::prompts::ExtractedEntity, name: &str| -> Result<Asset> {
        let kind = match e.kind.as_str() {
            "character" => AssetKind::Character,
            "scene" => AssetKind::Scene,
            "prop" => AssetKind::Prop,
            other => return Err(Error::config(format!("不认识的资产类型「{other}」"))),
        };
        let mut asset = Asset::new(kind, name);
        asset.description = e.description.trim().to_string();
        if kind == AssetKind::Character {
            asset.character = Some(CharacterTraits {
                persona: e.persona.trim().to_string(),
                // 身份锚点跟着建进来 —— 它是长相不漂移的主要抓手，
                // 而且抽取时顺手就有了，之后再补要用户自己填。
                visual_dna: e.anchors.clone(),
                disembodied: e.disembodied,
                // 槽位跟着建进来 —— 它是 `description` 的来源，
                // 只存渲染结果的话，以后想改一个槽就只能重写整段。
                spec: e.spec.clone(),
                ..Default::default()
            });
        }
        Ok(asset)
    };

    // 先分出「只留本集」的
    let mut to_series = Vec::new();
    for e in entities {
        let name = e.name.trim().to_string();
        if name.is_empty() {
            continue;
        }
        let key = format!("{}/{}", e.kind, name);
        let key_raw = format!("{}/{}", e.kind, e.name);
        if !episode_only.iter().any(|k| *k == key || *k == key_raw) {
            to_series.push(e);
            continue;
        }
        let mut asset = build(&e, &name)?;
        // 幂等：同名同类只建一次。重复点「应用」不该攒出一堆同名的。
        if !local.assets.iter().any(|a| crate::series_assets::same_entity(a, &asset)) {
            asset.episode_only = true;
            local.assets.push(asset);
            local_dirty = true;
        }
        sum.episode_only += 1;
    }

    // **同一个人的几套造型合成一个角色 + 几个形象**（2026-09-26 用户定的）。
    // 模型从人物小传里会抽出「星澜（出厂陪伴款）」「星澜（战斗款）」「星澜（外壳破损款）」三条，
    // 各建一个角色的话，出设定卡、分镜引用都当成三个人。按人设分组见 `series_assets::group_characters`。
    let (groups, rest) = crate::series_assets::group_characters(to_series);
    for g in &groups {
        let asset = build(&g.main, &g.name)?;
        if let Some(t) = series.assets.iter_mut().find(|a| crate::series_assets::same_entity(a, &asset)) {
            crate::series_assets::fill_missing(t, &asset);
            sum.looks += crate::series_assets::apply_looks(t, g);
            sum.merged += 1;
        } else {
            let mut asset = asset;
            sum.looks += crate::series_assets::apply_looks(&mut asset, g);
            series.assets.push(asset);
            sum.added += 1;
        }
    }
    for e in rest {
        let asset = build(&e, e.name.trim())?;
        if let Some(t) = series.assets.iter_mut().find(|a| crate::series_assets::same_entity(a, &asset)) {
            crate::series_assets::fill_missing(t, &asset);
            sum.merged += 1;
        } else {
            series.assets.push(asset);
            sum.added += 1;
        }
    }
    crate::assets::save_store(config_dir, &series_scope, &series)?;
    if local_dirty {
        crate::assets::save_store(config_dir, &ep_scope, &local)?;
    }
    Ok(sum)
}

#[cfg(test)]
mod synth_live {
    use super::*;

    /// 验收样本：真实项目里抽 3 镜，两条线路各合成一次，写进临时文件给人读。**不写回项目**。
    #[tokio::test]
    #[ignore = "要本机后台模型（LM Studio）和真实项目数据"]
    async fn 合成提示词实跑() {
        // 合成没过检查时模型原文走 tracing::warn —— 测试里不装订阅者就看不见
        let _ = tracing_subscriber::fmt().with_env_filter("warn").with_writer(std::io::stdout).try_init();
        let config_dir = dirs::config_dir().unwrap().join("com.anima.desktop");
        let project = std::env::var("ANIMA_LIVE_PROJECT").unwrap_or_else(|_| "drama_1789250410".into());
        let episode = std::env::var("ANIMA_LIVE_EPISODE").unwrap_or_else(|_| "ep_01".into());
        let nums: Vec<u32> = std::env::var("ANIMA_LIVE_SHOTS").unwrap_or_else(|_| "1,5,10".into()).split(',').filter_map(|n| n.trim().parse().ok()).collect();
        let ep_file = cinema_projects_dir(&config_dir).join(&project).join("episodes").join(&episode).join("episode.json");
        let mut ep: EpisodeData = serde_json::from_str(&std::fs::read_to_string(&ep_file).unwrap()).unwrap();
        // 挂没挂首帧按磁盘算，和正式合成一样（首帧删了的镜头不能还按首帧出片写）
        sync_keyframes(&ep_file.with_file_name("shots"), &mut ep);
        // `SHOT_FILE`：镜头从一份镜头数组 JSON 里取；`SEGMENTS_FILE`：第一镜的时间段换成这份（接细化实跑的输出，整条链串起来看）
        if let Ok(f) = std::env::var("SHOT_FILE") {
            ep.storyboard = serde_json::from_str(&std::fs::read_to_string(f).unwrap()).unwrap();
        }
        if let (Ok(f), Some(first)) = (std::env::var("SEGMENTS_FILE"), nums.first()) {
            let segs: Vec<crate::shot_table::ShotSegment> = serde_json::from_str(&std::fs::read_to_string(f).unwrap()).unwrap();
            if let Some(s) = ep.storyboard.iter_mut().find(|s| s.shot_num == *first) {
                s.segments = segs;
            }
        }
        let assets = crate::storyboard_gen::load_assets(&config_dir, &project, &episode);
        let mut app_cfg = crate::config::AppConfig::load(&config_dir.join("config.json")).unwrap();
        if let Ok(p) = std::env::var("ANIMA_LIVE_PROVIDER") {
            app_cfg.background_provider = p;
        }
        if let Ok(m) = std::env::var("ANIMA_LIVE_MODEL") {
            app_cfg.background_model = Some(m);
        }
        let gateway = std::sync::Arc::new(crate::llm::gateway::Gateway::new(app_cfg).unwrap());
        let mut cfg = crate::prompts::load(&config_dir);
        // `ANIMA_LIVE_ROUTE_B_TEMPLATE`：线路 B 写手换一份模板跑（对照用，不动用户的提示词设置）
        if let Ok(path) = std::env::var("ANIMA_LIVE_ROUTE_B_TEMPLATE") {
            cfg.overrides.insert(crate::prompts::PromptSlot::RouteBWriter.key().to_string(), std::fs::read_to_string(path).unwrap());
        }
        let call = |slot: crate::prompts::PromptSlot, vars: Vec<(String, String)>| {
            let gateway = gateway.clone();
            let cfg = cfg.clone();
            async move {
                let t = std::time::Instant::now();
                let vars_ref: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                let out = crate::prompts::run(&gateway, &cfg, slot, &vars_ref).await;
                println!("  {slot:?} 用时 {:.1}s", t.elapsed().as_secs_f32());
                out
            }
        };
        // `ANIMA_LIVE_PROJECT_STYLE=1`：画风按正式合成那样取项目 / 分集的（对照出片时必须一致）
        let style = if std::env::var_os("ANIMA_LIVE_PROJECT_STYLE").is_some() { episode_style(&config_dir, &project, &ep) } else { String::new() };
        // `ANIMA_LIVE_B_DIR`：线路 B 正文每镜单独存一份（`shot_<镜号>.txt`），拿去原样出片对照
        let b_dir = std::env::var("ANIMA_LIVE_B_DIR").ok().map(std::path::PathBuf::from);
        let mut report = String::new();
        for num in nums {
            let shot = ep.storyboard.iter().find(|s| s.shot_num == num).expect("没有这一镜");
            let prev = ep.storyboard.iter().filter(|s| s.shot_num < num).max_by_key(|s| s.shot_num);
            // 套这一镜选的造型，和正式合成一样（`synthesize_episode_prompts`）
            let shot_assets = crate::looks::for_shot(&assets, shot);
            let ctx = crate::prompt_input::ShotContext { shot, prev, assets: &shot_assets, style: &style, long_take: ep.long_take_scenes.contains(&shot.scene_no) };
            let a = crate::prompt_route_a::synthesize(&ctx, 0);
            println!("第 {num} 镜 A 检查：{:?}", crate::prompt_route_a::check(&ctx, &a));
            let b = crate::prompt_route_b::synthesize(&ctx, &call, 0).await;
            report.push_str(&format!("\n\n==================== 第 {num} 镜 ====================\n\n----- 线路 A · 视频 -----\n{}\n\n----- 线路 A · 分镜 -----\n{}\n", a.video, a.frame));
            match b {
                Ok((b, soft)) => {
                    println!("第 {num} 镜 B 检查：{:?}", crate::prompt_route_b::check(&ctx, &b));
                    if !soft.is_empty() {
                        println!("第 {num} 镜 B 还差点意思：{soft:?}");
                    }
                    report.push_str(&format!("\n----- 线路 B · 视频 -----\n{}\n\n----- 线路 B · 分镜 -----\n{}\n", b.video, b.frame));
                    if let Some(dir) = &b_dir {
                        std::fs::create_dir_all(dir).unwrap();
                        std::fs::write(dir.join(format!("shot_{num}.txt")), &b.video).unwrap();
                    }
                }
                Err(e) => {
                    println!("第 {num} 镜 B 失败：{e}");
                    report.push_str(&format!("\n----- 线路 B 失败 -----\n{e}\n"));
                }
            }
        }
        let out = std::env::var("ANIMA_LIVE_OUT").map(std::path::PathBuf::from).unwrap_or_else(|_| std::env::temp_dir().join("anima-prompts-live.txt"));
        std::fs::write(&out, report).unwrap();
        println!("结果写在 {}", out.display());
    }
}

#[cfg(test)]
mod look_outfit_tests {
    use super::*;
    use crate::storyboard_gen::AssetRef;

    /// 2026-10-07 EP03 重出首帧：星澜挂了晚礼服那张图，出来却是白纱短裙 / 机甲胸衣 —— 提示词里要明说这一镜穿什么
    #[test]
    fn 选了造型的人明说这一镜穿什么() {
        let a = |name: &str, look: &str, desc: &str| AssetRef { id: name.into(), kind: crate::assets::AssetKind::Character, name: name.into(), has_image: true,
            description: desc.into(), look_id: look.into(), looks: Vec::new(), h3_voice: String::new() };
        let assets = vec![
            a("星澜", "look_gown", "描述：陪伴机器人，银白长发\n服装：正红色缎面绣金线团花晚礼服，裙摆及踝\n特征：协议铭牌"),
            a("陆川", "", "描述：男\n服装：工装外套"),
        ];
        let lines = look_outfit_lines(&assets, &[("星澜".into(), 3), ("陆川".into(), 4)]);
        assert_eq!(lines, vec!["「星澜」（图 3）：正红色缎面绣金线团花晚礼服，裙摆及踝".to_string()], "主形象的不写，免得每张首帧都多一句");
        assert!(look_outfit_lines(&assets, &[("陆川".into(), 1)]).is_empty());
        assert!(look_outfit_lines(&assets, &[]).is_empty(), "没挂图的人不写（另有「画面里还有这几个人」）");
    }
}

#[cfg(test)]
mod routes_live {
    use super::*;

    /// 真实数据：一集每镜走哪条链路、按内容建议、被拦没有、自动接法。**只读**。
    /// `ANIMA_LIVE_PROJECT=… ANIMA_LIVE_EPISODE=ep_03 cargo test --lib 看出片链路 -- --ignored --nocapture`
    #[test]
    #[ignore = "读真实项目"]
    fn 看出片链路() {
        let cfg = dirs::config_dir().unwrap().join("com.anima.desktop");
        let project = std::env::var("ANIMA_LIVE_PROJECT").unwrap_or_else(|_| "drama_1790397900".into());
        let episode = std::env::var("ANIMA_LIVE_EPISODE").unwrap_or_else(|_| "ep_03".into());
        let mut ep = read_episode(&cfg, &project, &episode).unwrap();
        sync_keyframes(&episode_file(&cfg, &project, &episode).with_file_name("shots"), &mut ep);
        let route = project_route(&cfg, &project);
        let assets = crate::storyboard_gen::load_assets(&cfg, &project, &episode);
        let style = episode_style(&cfg, &project, &ep);
        for r in shot_routes(&cfg, &project, &episode).unwrap() {
            let s = ep.storyboard.iter().find(|s| s.id == r.shot_id).unwrap();
            let takes = s.takes.len();
            let prompt = match s.prompts.get(&route).filter(|p| !p.video.is_empty()) {
                None => "无",
                Some(p) if p.fields_hash != current_hash(&ep.storyboard, s, &assets, &style, &ep.long_take_scenes) => "过期",
                Some(_) => "最新",
            };
            println!(
                "第{:>2}镜 {:>4}秒 实际={:?} 建议={:?} 接={:?}{} 片={} 提示词={} | {}{}",
                r.shot_num, s.duration, r.actual, r.advised, s.effective_continuity(), if s.continuity_manual { "(手动)" } else { "" }, takes,
                prompt,
                r.reason, r.blocker.map(|b| format!(" ‼ 被拦：{b}")).unwrap_or_default()
            );
        }
        // 合成弹窗「没合成或过期的 N 镜」也算这些提示（`synthTargets` 的 advice）—— 打出来对得上数
        for i in validate_storyboard(&cfg, &project, &episode, &ep).iter().filter(|i| i.field == "advice") {
            println!("  提示 第{:>2}镜：{}", i.shot_num, i.message.chars().take(90).collect::<String>());
        }
        // `ANIMA_LIVE_ALL_ISSUES=1`：其余字段的问题也打出来（镜头表核对、过期、路线……）
        if std::env::var_os("ANIMA_LIVE_ALL_ISSUES").is_some() {
            for i in validate_storyboard(&cfg, &project, &episode, &ep).iter().filter(|i| i.field != "advice") {
                println!("  问题[{}] 第{:>2}镜：{}", i.field, i.shot_num, i.message.chars().take(120).collect::<String>());
            }
        }
    }
}

#[cfg(test)]
mod timeline_lint_live {
    use super::*;

    /// 真实数据：每集的时间线检查（背后看不见的部位 / 剧本没有的人）报了哪些。**只读**，量误报用。
    /// `ANIMA_LIVE_PROJECT=… cargo test --lib 看时间线检查 -- --ignored --nocapture`
    #[test]
    #[ignore = "读真实项目"]
    fn 看时间线检查() {
        let cfg = dirs::config_dir().unwrap().join("com.anima.desktop");
        let project = std::env::var("ANIMA_LIVE_PROJECT").unwrap_or_else(|_| "drama_1790397900".into());
        let mut eps: Vec<String> = std::fs::read_dir(cinema_projects_dir(&cfg).join(&project).join("episodes")).unwrap()
            .filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).collect();
        eps.sort();
        for episode in eps {
            let Ok(ep) = read_episode(&cfg, &project, &episode) else { continue };
            let assets = crate::storyboard_gen::load_assets(&cfg, &project, &episode);
            let issues = timeline_issues(&ep.storyboard, &assets);
            println!("== {episode}（{} 镜）：{} 条", ep.storyboard.len(), issues.len());
            for i in issues {
                println!("  第{:>2}镜 {}", i.shot_num, i.message.chars().take(150).collect::<String>());
            }
        }
    }
}

#[cfg(test)]
mod openh3ir_export {
    use super::*;

    /// **只读**：把某几镜的原始输入（镜头表描述、台词、参考图 —— 和生产出片挂的同一组同一顺序）连同线路 B 现成的稿子导出成 JSON，
    /// 给 open-h3-ir 做对照（2026-10-09 用户：「先做第 1、2 步，但是不要影响当前生产环境的稿子内容」）。不写任何项目文件。
    /// `OPENH3_OUT=/abs/out.json OPENH3_SHOTS=4,2,9 cargo test --lib 导出对照输入 -- --ignored --nocapture`
    #[test]
    #[ignore = "读真实项目"]
    fn 导出对照输入() {
        let cfg = dirs::config_dir().unwrap().join("com.anima.desktop");
        let project = std::env::var("ANIMA_LIVE_PROJECT").unwrap_or_else(|_| "drama_1790397900".into());
        let episode = std::env::var("ANIMA_LIVE_EPISODE").unwrap_or_else(|_| "ep_03".into());
        let want: Vec<u32> = std::env::var("OPENH3_SHOTS").unwrap_or_else(|_| "4,2,9".into()).split(',').filter_map(|x| x.trim().parse().ok()).collect();
        let mut ep = read_episode(&cfg, &project, &episode).unwrap();
        let shots_dir = episode_file(&cfg, &project, &episode).with_file_name("shots");
        sync_keyframes(&shots_dir, &mut ep);
        let raw = crate::storyboard_gen::load_assets(&cfg, &project, &episode);
        let mut out = Vec::new();
        for n in want {
            let shot = ep.storyboard.iter().find(|s| s.shot_num == n).unwrap();
            let assets = crate::looks::for_shot(&raw, shot);
            let ctx = crate::prompt_input::ShotContext { shot, prev: None, assets: &assets, style: "", long_take: false };
            let keyframe = ctx.keyframe_video();
            let refs: Vec<serde_json::Value> = if keyframe {
                vec![json!({"path": shots_dir.join(format!("{}.png", shot.id)), "role": "frame_anchor_first", "name": "首帧"})]
            } else {
                ctx.slots().into_iter().filter_map(|(slot, a)| {
                    let a = a?;
                    let p = asset_image_path(&cfg, &project, &episode, &a.id, &a.look_id)?;
                    let p = if a.kind == crate::assets::AssetKind::Scene { flat_scene_ref(&cfg, &p) } else { p };
                    let role = if a.kind == crate::assets::AssetKind::Scene { "environment" } else { "subject" };
                    Some(json!({"path": p, "role": role, "name": a.name, "look": a.description.chars().take(160).collect::<String>(), "control": slot.control, "not_control": slot.not_control}))
                }).collect()
            };
            let segments: Vec<String> = shot.segments.iter().map(|g| {
                let extra = [("站位", &g.blocking), ("镜头", &g.camera), ("声音", &g.sound), ("禁止", &g.forbid)]
                    .iter().filter(|(_, v)| !v.trim().is_empty()).map(|(k, v)| format!("{k}：{v}")).collect::<Vec<_>>().join("；");
                format!("{:.1}-{:.1}秒「{}」{}{}", g.start, g.end, g.title, g.visual, if extra.is_empty() { String::new() } else { format!("（{extra}）") })
            }).collect();
            out.push(json!({
                "shot_num": n, "duration": shot.duration, "route": if keyframe { "keyframe" } else { "reference" },
                "scale": shot.scale, "camera": shot.camera_movement, "prompt": shot.prompt, "segments": segments,
                "dialogue": shot.dialogue_lines.iter().map(|l| json!({"speaker": l.speaker, "line": l.line, "off_screen": l.off_screen, "voice": l.voice, "emotion": l.emotion})).collect::<Vec<_>>(),
                "refs": refs,
                "route_b": shot.prompts.get(&crate::shot_table::PromptRoute::B).map(|p| p.video.clone()).unwrap_or_default(),
            }));
        }
        let path = std::env::var("OPENH3_OUT").unwrap_or_else(|_| "/tmp/openh3_inputs.json".into());
        std::fs::write(&path, serde_json::to_string_pretty(&out).unwrap()).unwrap();
        println!("写到 {path}（{} 镜）", out.len());
    }
}

#[cfg(test)]
mod ep03_look_live {
    use super::*;

    /// 真实数据：EP03 第 2 镜星澜要挂「嫁衣改晚礼服（E03）」那张，资产描述是正红色及踝晚礼服、没有银链纱裙。
    /// `cargo test --lib ep03星澜挂晚礼服 -- --ignored`
    #[test]
    #[ignore = "读真实项目"]
    fn ep03星澜挂晚礼服() {
        let cfg = dirs::config_dir().unwrap().join("com.anima.desktop");
        let (project, episode) = ("drama_1790397900", "ep_03");
        let ep_dir = cinema_projects_dir(&cfg).join(project).join("episodes").join(episode);
        let ep: EpisodeData = serde_json::from_str(&std::fs::read_to_string(ep_dir.join("episode.json")).unwrap()).unwrap();
        let shot = ep.storyboard.iter().find(|s| s.shot_num == 2).unwrap();
        let assets = crate::storyboard_gen::load_assets(&cfg, project, episode);
        let refs = render_reference_paths(&cfg, project, episode, &ep_dir.join("shots"), shot, &assets, false);
        assert!(refs[0].to_string_lossy().contains("red_wedding_evening_gown_e03"), "Picture 1 不是晚礼服那张：{:?}", refs[0]);
        let bai = crate::looks::for_shot(&assets, shot).into_iter().find(|a| a.name == "星澜").unwrap();
        assert!(bai.description.contains("正红色") && !bai.description.contains("纱裙") && !bai.description.contains("吊袜带"), "{}", bai.description);
        let ctx = crate::prompt_input::ShotContext { shot, prev: None, assets: &crate::looks::for_shot(&assets, shot), style: "", long_take: false };
        assert!(!ctx.slots().iter().any(|(s, a)| crate::prompt_input::outfit_mismatch(s, *a)), "还有换装没选造型的人");
    }
}

#[cfg(test)]
mod tier_live {
    use super::*;

    /// 出片档位对照：同一镜、同一个种子、同一条提示词，三种档位各出一次，记耗时。**不写回项目**。
    ///
    /// 环境变量：`ANIMA_LIVE_SHOT`（镜号，默认 11）、`ANIMA_TIER_OUT`（输出目录）。
    #[tokio::test]
    #[ignore = "要本机 ComfyUI + H3，跑一个多小时"]
    async fn 出片档位对照() {
        use crate::agent_runtime::weetodd::H3Tier;
        let config_dir = dirs::config_dir().unwrap().join("com.anima.desktop");
        let (project, episode) = ("drama_1789250410", "ep_01");
        let num: u32 = std::env::var("ANIMA_LIVE_SHOT").ok().and_then(|v| v.parse().ok()).unwrap_or(11);
        let out_dir = PathBuf::from(std::env::var("ANIMA_TIER_OUT").unwrap_or_else(|_| std::env::temp_dir().join("anima-tier-test").to_string_lossy().to_string()));
        std::fs::create_dir_all(&out_dir).unwrap();
        let ep_dir = cinema_projects_dir(&config_dir).join(project).join("episodes").join(episode);
        let ep: EpisodeData = serde_json::from_str(&std::fs::read_to_string(ep_dir.join("episode.json")).unwrap()).unwrap();
        let shot = ep.storyboard.iter().find(|s| s.shot_num == num).unwrap();
        let prompt = video_prompt_for_render(shot, crate::shot_table::PromptRoute::B).unwrap();
        let assets = crate::storyboard_gen::load_assets(&config_dir, project, episode);
        let refs = render_reference_paths(&config_dir, project, episode, &ep_dir.join("shots"), shot, &assets, false);
        let seed = shot_seed(project, episode, &shot.id);
        println!("第 {num} 镜 {} 秒，参考图 {} 张，种子 {seed}", shot.duration, refs.len());
        let only = std::env::var("ANIMA_TIER_ONLY").unwrap_or_default();
        for (name, w, h, tier) in [
            ("现状-turbo-1280x736", 1280u32, 736u32, H3Tier::Standard),
            ("双采-896x512", 896, 512, H3Tier::Staged),
            ("双采-1280x736", 1280, 736, H3Tier::Staged),
        ] {
            if !only.is_empty() && !name.contains(&only) {
                continue;
            }
            let out = out_dir.join(format!("shot{num}-{name}.mp4"));
            let t = std::time::Instant::now();
            let r = crate::agent_runtime::video_tool::render_shot_video_reference_with(&refs, &out, &prompt, shot.duration as f64, w, h, seed, true, Some(tier), None).await;
            println!("{name}：{:.1} 分钟 {:?} → {}", t.elapsed().as_secs_f32() / 60.0, r.err(), out.display());
        }
    }
}

#[cfg(test)]
mod route_ab_live {
    use super::*;

    /// 把 ComfyUI 已经出好、但 Anima 按旧的等待上限判了失败的首帧收进镜头 —— 和详情面板「导入首帧」同一个函数
    /// （`set_shot_keyframe`，按项目画幅裁）。**写真实项目**。已经有首帧的镜头跳过（应用自己收到了就不重复收）。
    ///
    /// 2026-09-29：排队出首帧，排在后面的镜头光排队就等满 40 分钟被判失败，而 ComfyUI 随后出了图（v0.42.110 修了计时）。
    /// `ANIMA_ADOPT_FRAMES`：`镜头id=图片路径;…`
    /// **参考出片对照**：一镜的参考图（按正式出片取，`render_reference_paths`，带这一镜选的造型）+ 存好的线路 B 提示词，
    /// 照正式出片（`render_shot_video_reference_with`、项目的档位和种子）出一条，只换加速 LoRA（`ANIMA_H3_TRIAL_*`）。
    /// **兼容模式**才吃对照 LoRA：要比原生的话另设 `ANIMA_H3_NATIVE_REF2VA`。**不写回项目**。
    /// `ANIMA_TRIAL_SHOT=2 ANIMA_TRIAL_OUT=/x.mp4 [ANIMA_TRIAL_RES=480p] [ANIMA_TRIAL_PROMPT_FILE=/abs/prompt.txt] [ANIMA_TRIAL_SEED=…] ANIMA_H3_NATIVE_REF2VA=0 ANIMA_H3_TRIAL_LORA=…`
    #[tokio::test]
    #[ignore = "占 ComfyUI，一条几十分钟"]
    async fn 参考出片对照() {
        let _ = tracing_subscriber::fmt().with_env_filter("info").with_writer(std::io::stdout).try_init();
        let cfg = dirs::config_dir().unwrap().join("com.anima.desktop");
        let project_id = std::env::var("ANIMA_LIVE_PROJECT").unwrap_or_else(|_| "drama_1790397900".into());
        let episode = std::env::var("ANIMA_LIVE_EPISODE").unwrap_or_else(|_| "ep_03".into());
        let num: u32 = std::env::var("ANIMA_TRIAL_SHOT").expect("要设 ANIMA_TRIAL_SHOT").parse().unwrap();
        let out = PathBuf::from(std::env::var("ANIMA_TRIAL_OUT").expect("要设 ANIMA_TRIAL_OUT"));
        let ep_dir = cinema_projects_dir(&cfg).join(&project_id).join("episodes").join(&episode);
        let ep: EpisodeData = serde_json::from_str(&std::fs::read_to_string(ep_dir.join("episode.json")).unwrap()).unwrap();
        let shot = ep.storyboard.iter().find(|s| s.shot_num == num).expect("没有这一镜");
        let assets = crate::storyboard_gen::load_assets(&cfg, &project_id, &episode);
        let refs = render_reference_paths(&cfg, &project_id, &episode, &ep_dir.join("shots"), shot, &assets, false);
        assert!(!refs.is_empty(), "这一镜没有参考图");
        // `ANIMA_TRIAL_PROMPT_FILE`：换一份提示词（同一组参考图、同一档位和种子），比如镜内切镜的冒烟测试（2026-10-10）
        let prompt = match std::env::var("ANIMA_TRIAL_PROMPT_FILE") {
            Ok(f) => std::fs::read_to_string(&f).expect("读不到 ANIMA_TRIAL_PROMPT_FILE"),
            Err(_) => video_prompt_for_render(shot, crate::shot_table::PromptRoute::B).expect("没有线路 B 提示词"),
        };
        let project = read_project(&cfg, &project_id).unwrap();
        let mut render = project.model_settings.render.clone();
        if let Ok(r) = std::env::var("ANIMA_TRIAL_RES") {
            render.resolution = r;
        }
        let (w, h) = render.size(&project.aspect_ratio);
        let tier = render.quality.tier();
        let seed = std::env::var("ANIMA_TRIAL_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or_else(|| shot_seed(&project_id, &episode, &shot.id));
        let plan = crate::agent_runtime::weetodd::H3SamplingPlan::current(true, tier);
        println!("第 {num} 镜 {} 秒 {w}×{h} 种子 {seed} · {}", shot.duration, plan.describe());
        for (i, r) in refs.iter().enumerate() {
            println!("  Picture {}: {}", i + 1, r.display());
        }
        let t = std::time::Instant::now();
        let r = crate::agent_runtime::video_tool::render_shot_video_reference_with(&refs, &out, &prompt, shot.duration as f64, w, h, seed, true, Some(tier), None).await;
        println!("出完：{:?}（{:.1} 分钟）→ {}", r.err(), t.elapsed().as_secs_f64() / 60.0, out.display());
    }

    /// **首帧出片对照**：拿一镜现成的首帧和存好的线路 B 提示词，照正式出片（`render_shot_video`、项目的档位和种子）出一条，
    /// 只换加速 LoRA（`ANIMA_H3_TRIAL_*`，见 `weetodd::H3SamplingPlan::current`）。**不写回项目**。
    /// `ANIMA_TRIAL_SHOT=10 ANIMA_TRIAL_OUT=/x.mp4 [ANIMA_TRIAL_RES=480p] [ANIMA_H3_TRIAL_LORA=… ANIMA_H3_TRIAL_STRENGTH=… ANIMA_H3_TRIAL_POINTS=…]`
    #[tokio::test]
    #[ignore = "占 ComfyUI，一条几十分钟"]
    async fn 首帧出片对照() {
        let _ = tracing_subscriber::fmt().with_env_filter("info").with_writer(std::io::stdout).try_init();
        let cfg = dirs::config_dir().unwrap().join("com.anima.desktop");
        let project_id = std::env::var("ANIMA_LIVE_PROJECT").unwrap_or_else(|_| "drama_1790397900".into());
        let episode = std::env::var("ANIMA_LIVE_EPISODE").unwrap_or_else(|_| "ep_03".into());
        let num: u32 = std::env::var("ANIMA_TRIAL_SHOT").expect("要设 ANIMA_TRIAL_SHOT").parse().unwrap();
        let out = PathBuf::from(std::env::var("ANIMA_TRIAL_OUT").expect("要设 ANIMA_TRIAL_OUT"));
        let ep_dir = cinema_projects_dir(&cfg).join(&project_id).join("episodes").join(&episode);
        let ep: EpisodeData = serde_json::from_str(&std::fs::read_to_string(ep_dir.join("episode.json")).unwrap()).unwrap();
        let shot = ep.storyboard.iter().find(|s| s.shot_num == num).expect("没有这一镜");
        let frame = ep_dir.join("shots").join(format!("{}.png", shot.id));
        assert!(frame.is_file(), "这一镜没有首帧");
        let prompt = video_prompt_for_render(shot, crate::shot_table::PromptRoute::B).expect("没有线路 B 提示词");
        let mut render = read_project(&cfg, &project_id).unwrap().model_settings.render;
        if let Ok(r) = std::env::var("ANIMA_TRIAL_RES") {
            render.resolution = r;
        }
        let project = read_project(&cfg, &project_id).unwrap();
        let (w, h) = render.size(&project.aspect_ratio);
        let tier = render.quality.tier();
        let seed = shot_seed(&project_id, &episode, &shot.id);
        let plan = crate::agent_runtime::weetodd::H3SamplingPlan::current(false, tier);
        println!("第 {num} 镜 {} 秒 {w}×{h} 种子 {seed} · {}", shot.duration, plan.describe());
        let t = std::time::Instant::now();
        let r = crate::agent_runtime::video_tool::render_shot_video(&frame, &out, "h3", &prompt, shot.duration as f64, w, h, 24.0, seed, true, None, Some(tier)).await;
        println!("出完：{:?}（{:.1} 分钟）→ {}", r.err(), t.elapsed().as_secs_f64() / 60.0, out.display());
    }

    /// **重出几镜的首帧**（和界面「出首帧」同一个函数 `render_studio_frame`），一张一张串行出。**写真实项目**。
    ///
    /// 2026-10-07 EP03：旧首帧是星澜主形象（银链纱裙）+ 1216×832，选了 E03 晚礼服造型、首帧改按画布比例之后要重出。
    /// 每张提交前写生成日志（AGENTS.md），旧首帧先拷进 `backups/frames-<时间>/`。
    /// `ANIMA_LIVE_PROJECT` / `ANIMA_LIVE_EPISODE` / `ANIMA_REGEN_FRAMES=4,8,…`
    #[tokio::test]
    #[ignore = "写真实项目、占 ComfyUI"]
    async fn 重出首帧() {
        let _ = tracing_subscriber::fmt().with_env_filter("info").with_writer(std::io::stdout).try_init();
        let cfg = dirs::config_dir().unwrap().join("com.anima.desktop");
        let project_id = std::env::var("ANIMA_LIVE_PROJECT").unwrap_or_else(|_| "drama_1790397900".into());
        let episode = std::env::var("ANIMA_LIVE_EPISODE").unwrap_or_else(|_| "ep_03".into());
        let nums: Vec<u32> = std::env::var("ANIMA_REGEN_FRAMES").expect("要设 ANIMA_REGEN_FRAMES").split(',').filter_map(|n| n.trim().parse().ok()).collect();
        let proj_dir = cinema_projects_dir(&cfg).join(&project_id);
        let ep_dir = proj_dir.join("episodes").join(&episode);
        let (ep_file, shots_dir) = (ep_dir.join("episode.json"), ep_dir.join("shots"));
        let date = |f: &str| String::from_utf8_lossy(&std::process::Command::new("date").arg(f).output().unwrap().stdout).trim().to_string();
        let backup = proj_dir.join("backups").join(format!("frames-{}", date("+%Y%m%d-%H%M%S")));
        std::fs::create_dir_all(&backup).unwrap();
        let project = read_project(&cfg, &project_id).unwrap();
        let now = || date("+%Y-%m-%dT%H:%M:%S%z");
        for num in nums {
            let ep: EpisodeData = serde_json::from_str(&std::fs::read_to_string(&ep_file).unwrap()).unwrap();
            let shot = ep.storyboard.iter().find(|s| s.shot_num == num).expect("没有这一镜").clone();
            let old = shots_dir.join(format!("{}.png", shot.id));
            if old.is_file() {
                std::fs::copy(&old, backup.join(format!("shot{num:02}-{}.png", shot.id))).unwrap();
            }
            let args = studio_frame_args(&cfg, &project, &episode, &shot).unwrap();
            let log = shots_dir.join(format!("{}.regen-{}.generation.md", shot.id, date("+%H%M%S")));
            let inputs: Vec<String> = args["reference_images"].as_array().map(|a| a.iter().enumerate().map(|(i, p)| format!("- Input {}:\n  - Role: reference（图 {}）\n  - Absolute path: {}", i + 1, i + 1, p.as_str().unwrap_or_default())).collect()).unwrap_or_default();
            let head = format!(
                "# Generation Log\n\n## Identity\n- Attempt: 1\n- Status: {{STATUS}}\n- Media type: image\n- Planned filename: {id}.png\n- Final absolute path: {path}\n\n## Time\n- Submitted at: {t}\n- Finished at: {{DONE}}\n\n## Generator\n- Engine/model: {engine}\n- Tool/API: `cinema::render_studio_frame`（和界面「出首帧」同一个函数），`cargo test --lib 重出首帧 -- --ignored`\n- Mode: reference-image（首帧）\n\n## Inputs and relationships\n{inputs}\n- Relationship: EP03 第 {num} 镜首帧重出：星澜换 E03 晚礼服造型、首帧改按 16:9 画布比例。旧首帧备份在 {bk}\n\n## Full prompt\n{prompt}\n\n## Parameters\n- Width: {w}\n- Height: {h}\n- Other: 出图后按 {w}×{h} 居中裁切落盘（`install_first_frame`）\n\n## Result\n{{RESULT}}\n",
                id = shot.id, path = old.display(), t = now(), engine = args["engine"], inputs = inputs.join("\n"), num = num, bk = backup.display(),
                prompt = args["prompt"].as_str().unwrap_or_default(), w = args["width"], h = args["height"],
            );
            std::fs::write(&log, head.replace("{STATUS}", "submitted").replace("{DONE}", "unknown / 未完成").replace("{RESULT}", "- 待看图")).unwrap();
            let t = std::time::Instant::now();
            let r = render_studio_frame(&cfg, &project, &shot.id, &ep_file, &shots_dir, &shot, None).await;
            let secs = t.elapsed().as_secs();
            let (status, result) = match &r {
                Ok(_) => ("succeeded", format!("- 耗时 {secs} 秒；尺寸 {:?}；待看图", image::image_dimensions(&old).ok())),
                Err(e) => ("failed", format!("- Error message: {e}")),
            };
            std::fs::write(&log, head.replace("{STATUS}", status).replace("{DONE}", &now()).replace("{RESULT}", &result)).unwrap();
            println!("第 {num} 镜：{status}（{secs} 秒）{}", r.err().map(|e| e.to_string()).unwrap_or_default());
        }
        println!("旧首帧备份：{}", backup.display());
    }

    #[test]
    #[ignore = "写真实项目"]
    fn 收回超时的首帧() {
        let cfg = dirs::config_dir().unwrap().join("com.anima.desktop");
        let project = std::env::var("ANIMA_LIVE_PROJECT").unwrap_or_else(|_| "drama_1790397900".into());
        let episode = std::env::var("ANIMA_LIVE_EPISODE").unwrap_or_else(|_| "ep_01".into());
        let shots_dir = cinema_projects_dir(&cfg).join(&project).join("episodes").join(&episode).join("shots");
        for pair in std::env::var("ANIMA_ADOPT_FRAMES").expect("要设 ANIMA_ADOPT_FRAMES").split(';').filter(|p| !p.trim().is_empty()) {
            let (id, path) = pair.split_once('=').expect("镜头id=图片路径");
            if Keyframes::on_disk(&shots_dir, id).first {
                println!("{id} 已经有首帧，跳过");
                continue;
            }
            let shot = set_shot_keyframe(&cfg, &project, &episode, id, KeyframeSlot::First, Some(&crate::assets::MediaInput::Path { path: path.into() })).unwrap();
            println!("第 {} 镜收好了：{:?}", shot.shot_num, shot.keyframes);
        }
    }

    /// 把一条出好、但 Anima 没收回来的长镜头**整条记到这一场名下**（`record_scene_render`，和出片那条同一个函数）。**写真实项目**。
    ///
    /// 2026-09-30：第 5 场跑了 8 小时 55 分，Anima 在第 4 小时就判了失败（v0.42.114 修了）。
    /// `ANIMA_ADOPT_LONG_TAKE`：`场号=mp4 路径`。不在 shots/ 里的先拷进去。
    /// 顺带清掉 v0.42.116 按接缝切出来、挂在各镜名下的那几段（`-lt-`，用户定了不切）。**提示词要和当时发出去的一致** —— 先对过再跑
    #[test]
    #[ignore = "写真实项目"]
    fn 收回长镜头() {
        let cfg = dirs::config_dir().unwrap().join("com.anima.desktop");
        let project = std::env::var("ANIMA_LIVE_PROJECT").unwrap_or_else(|_| "drama_1790397900".into());
        let episode = std::env::var("ANIMA_LIVE_EPISODE").unwrap_or_else(|_| "ep_01".into());
        let spec = std::env::var("ANIMA_ADOPT_LONG_TAKE").expect("要设 ANIMA_ADOPT_LONG_TAKE");
        let (scene, src) = spec.split_once('=').expect("场号=mp4 路径");
        let scene_no: u32 = scene.trim().parse().unwrap();
        let src = PathBuf::from(src.trim());

        let ep = read_episode(&cfg, &project, &episode).unwrap();
        let shots: Vec<&StoryboardShot> = ep.storyboard.iter().filter(|s| s.scene_no == scene_no).collect();
        let frames = ep.long_take_context_frames.unwrap_or(crate::long_take::CONTEXT_FRAMES);
        let plan = crate::long_take::plan_with(&shots, frames).unwrap();
        let route = project_route(&cfg, &project);
        let shots_dir = cinema_projects_dir(&cfg).join(&project).join("episodes").join(&episode).join("shots");

        // v0.42.116 切出来的那几段：从镜头上摘掉、文件删掉（都是那天从整条派生出来的，整条还在）
        let cut: Vec<String> = shots.iter().flat_map(|s| s.takes.iter()).filter(|t| t.path.contains("-lt-")).map(|t| t.path.clone()).collect();
        let ids: Vec<String> = shots.iter().map(|s| s.id.clone()).collect();
        crate::episode_lock::update(&cfg, &project, &episode, |ep| {
            for s in ep.storyboard.iter_mut().filter(|s| ids.contains(&s.id)) {
                s.takes.retain(|t| !t.path.contains("-lt-"));
                if s.video_url.as_deref().is_some_and(|u| u.contains("-lt-")) {
                    s.video_url = s.takes.last().map(|t| t.path.clone());
                    if s.video_url.is_none() {
                        s.status = "pending".into();
                    }
                }
            }
            Ok(())
        })
        .unwrap();
        for p in &cut {
            let _ = std::fs::remove_file(p);
            println!("摘掉切片：{p}");
        }

        if ep.scene_renders.iter().any(|r| r.scene_no == scene_no && r.route == route && Path::new(&r.path).is_file()) {
            println!("第 {scene_no} 场已经记过整段成片，不重复记");
            return;
        }
        let assets = crate::storyboard_gen::load_assets(&cfg, &project, &episode);
        let style = episode_style(&cfg, &project, &ep);
        let (_, records) = long_take_windows(&cfg, &project, &episode, &ep, &plan, route, &assets, &style, &shots_dir).unwrap();
        let full = if src.parent() == Some(shots_dir.as_path()) {
            src.clone()
        } else {
            let dst = shots_dir.join(format!("longtake_s{scene_no}-{}.mp4", crate::jobs::now()));
            std::fs::copy(&src, &dst).unwrap();
            dst
        };
        let out = std::process::Command::new(crate::agent_runtime::video_tool::ffprobe_binary())
            .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height", "-of", "csv=p=0"])
            .arg(&full).output().unwrap();
        let wh = String::from_utf8_lossy(&out.stdout).trim().replace(',', "×");
        let quality = format!("长镜头 · {} · {wh}", project_render_settings(&cfg, &project).quality.label());
        let seed = shot_seed(&project, &episode, &format!("scene_{scene_no}"));
        record_scene_render(&cfg, &project, &episode, scene_no, &full, &records, route, seed, &quality).unwrap();
        println!("第 {scene_no} 场整段成片：{}（{quality}）", full.display());
    }

    /// 出片前把整集过一遍：每镜两条线路的正式检查、提示词过没过期、挂了哪些参考图、线路 B 正文里混进多少中文。
    /// **只读**。`ANIMA_LIVE_PROJECT` / `ANIMA_LIVE_EPISODE` 选哪一集
    #[test]
    #[ignore = "读真实项目"]
    fn 出片前全集核对() {
        use crate::shot_table::PromptRoute;
        let cfg = dirs::config_dir().unwrap().join("com.anima.desktop");
        let project = std::env::var("ANIMA_LIVE_PROJECT").unwrap_or_else(|_| "drama_1790397900".into());
        let episode = std::env::var("ANIMA_LIVE_EPISODE").unwrap_or_else(|_| "ep_01".into());
        let ep_dir = cinema_projects_dir(&cfg).join(&project).join("episodes").join(&episode);
        let ep: EpisodeData = serde_json::from_str(&std::fs::read_to_string(ep_dir.join("episode.json")).unwrap()).unwrap();
        let assets = crate::storyboard_gen::load_assets(&cfg, &project, &episode);
        let style = episode_style(&cfg, &project, &ep);
        // 线路 B 正文里 <d>…</d> 和「」以外的汉字个数（写手漏段时代码按中文原文补进去的就在这里）
        let cjk_outside = |t: &str| -> usize {
            let (mut n, mut depth) = (0usize, 0i32);
            let mut rest = t;
            while let Some(c) = rest.chars().next() {
                if rest.starts_with("<d>") { depth += 1; rest = &rest[3..]; continue; }
                if rest.starts_with("</d>") { depth -= 1; rest = &rest[4..]; continue; }
                if c == '「' { depth += 1 } else if c == '」' { depth -= 1 }
                if depth <= 0 && ('\u{4e00}'..='\u{9fff}').contains(&c) { n += 1 }
                rest = &rest[c.len_utf8()..];
            }
            n
        };
        let mut shots: Vec<&StoryboardShot> = ep.storyboard.iter().collect();
        shots.sort_by_key(|s| s.shot_num);
        for shot in shots {
            let prev = prev_of(&ep.storyboard, shot);
            let shot_assets = crate::looks::for_shot(&assets, shot);
            let ctx = crate::prompt_input::ShotContext { shot, prev, assets: &shot_assets, style: &style, long_take: ep.long_take_scenes.contains(&shot.scene_no) };
            let refs = render_reference_paths(&cfg, &project, &episode, &ep_dir.join("shots"), shot, &assets, false);
            let mut lines = Vec::new();
            for route in [PromptRoute::A, PromptRoute::B] {
                if let Some(why) = render_blocker(&ep.storyboard, shot, route, &assets, &style, &ep.long_take_scenes) {
                    lines.push(format!("{route:?} 不能出片：{why}"));
                }
                if let Some(p) = shot.prompts.get(&route) {
                    let problems = match route {
                        PromptRoute::A => crate::prompt_route_a::check(&ctx, p),
                        PromptRoute::B => crate::prompt_route_b::check(&ctx, p),
                    };
                    for pr in problems {
                        lines.push(format!("{route:?} 检查：{pr}"));
                    }
                }
            }
            let b_cn = shot.prompts.get(&PromptRoute::B).map(|p| cjk_outside(&p.video)).unwrap_or(0);
            let b_words = shot.prompts.get(&PromptRoute::B).map(|p| p.video.split_whitespace().count()).unwrap_or(0);
            println!(
                "第 {:>2} 镜 {:>4.1}s {:<14} 参考图 {} 张 · B {} 词、标签外汉字 {}{}",
                shot.shot_num, shot.duration, shot.scale, refs.len(), b_words, b_cn,
                if lines.is_empty() { String::new() } else { format!("\n    {}", lines.join("\n    ")) }
            );
        }
        for i in validate_storyboard(&cfg, &project, &episode, &ep) {
            println!("分镜检查：第 {} 镜 {} {}", i.shot_num, i.field, i.message);
        }
        let num = |ids: &[String]| ids.iter().filter_map(|id| ep.storyboard.iter().find(|s| &s.id == id).map(|s| s.shot_num)).collect::<Vec<_>>();
        let st = stale_prompts(&cfg, &project, &episode, &ep);
        println!("合成弹窗看到的过期：线路 A {:?}，线路 B {:?}", num(&st.a), num(&st.b));
    }

    /// 线路 A / B 对照出片：同一镜、同一组参考图、同一个种子、同一个尺寸和档位，只换提示词。**不写回项目**。
    ///
    /// 2026-09-29 用户：「线路 A 从来没测过，正好这个机会测一下」。
    ///
    /// 环境变量：`SHOT_FILE`（镜头数组 JSON）、`SEGMENTS_FILE`（这一镜的时间段换成这份）、`ANIMA_LIVE_SHOT`（镜号）、
    /// `ANIMA_AB_OUT`（输出目录）、`ANIMA_LIVE_PROVIDER` / `ANIMA_LIVE_MODEL`（线路 B 写手用的后台模型）、
    /// `ANIMA_AB_ONLY`（`a` / `b`，只出一条）。
    #[tokio::test]
    #[ignore = "要本机 ComfyUI + H3 和后台模型，两条片一两个小时"]
    async fn 线路对照出片() {
        let config_dir = dirs::config_dir().unwrap().join("com.anima.desktop");
        let (project, episode) = ("drama_1790397900", "ep_01");
        let num: u32 = std::env::var("ANIMA_LIVE_SHOT").ok().and_then(|v| v.parse().ok()).unwrap_or(13);
        let out_dir = PathBuf::from(std::env::var("ANIMA_AB_OUT").expect("要设 ANIMA_AB_OUT"));
        std::fs::create_dir_all(&out_dir).unwrap();
        let mut storyboard: Vec<StoryboardShot> = serde_json::from_str(&std::fs::read_to_string(std::env::var("SHOT_FILE").expect("要设 SHOT_FILE")).unwrap()).unwrap();
        if let Ok(f) = std::env::var("SEGMENTS_FILE") {
            let segs: Vec<crate::shot_table::ShotSegment> = serde_json::from_str(&std::fs::read_to_string(f).unwrap()).unwrap();
            storyboard.iter_mut().find(|s| s.shot_num == num).unwrap().segments = segs;
        }
        let shot = storyboard.iter().find(|s| s.shot_num == num).unwrap().clone();
        let prev = storyboard.iter().filter(|s| s.shot_num < num).max_by_key(|s| s.shot_num).cloned();
        let assets = crate::storyboard_gen::load_assets(&config_dir, project, episode);
        let ctx = crate::prompt_input::ShotContext { shot: &shot, prev: prev.as_ref(), assets: &assets, style: "", long_take: false };

        // 线路 A：纯填空；线路 B：后台模型写措辞
        let a = crate::prompt_route_a::synthesize(&ctx, 0);
        println!("线路 A 检查：{:?}", crate::prompt_route_a::check(&ctx, &a));
        let mut app_cfg = crate::config::AppConfig::load(&config_dir.join("config.json")).unwrap();
        if let Ok(p) = std::env::var("ANIMA_LIVE_PROVIDER") {
            app_cfg.background_provider = p;
        }
        if let Ok(m) = std::env::var("ANIMA_LIVE_MODEL") {
            app_cfg.background_model = Some(m);
        }
        let gateway = crate::llm::gateway::Gateway::new(app_cfg).unwrap();
        let cfg = crate::prompts::load(&config_dir);
        let call = |slot: crate::prompts::PromptSlot, vars: Vec<(String, String)>| {
            let (gateway, cfg) = (&gateway, &cfg);
            async move {
                let v: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                crate::prompts::run(gateway, cfg, slot, &v).await
            }
        };
        // `ANIMA_AB_B_PROMPT`：线路 B 直接用这份提示词，不再让写手重写（只比别的变量时用，比如参考图像素预算）
        let b_video = match std::env::var("ANIMA_AB_B_PROMPT") {
            Ok(f) => std::fs::read_to_string(f).unwrap(),
            Err(_) => {
                let (b, soft) = crate::prompt_route_b::synthesize(&ctx, &call, 0).await.expect("线路 B 合成失败");
                println!("线路 B 检查：{:?} 软提示：{soft:?}", crate::prompt_route_b::check(&ctx, &b));
                b.video
            }
        };
        std::fs::write(out_dir.join(format!("shot{num}-A.txt")), &a.video).unwrap();
        std::fs::write(out_dir.join(format!("shot{num}-B.txt")), &b_video).unwrap();

        // 同一组参考图、同一个种子、480P、双采档（项目用的档位）
        let refs = render_reference_paths(&config_dir, project, episode, &cinema_projects_dir(&config_dir).join(project).join("episodes").join(episode).join("shots"), &shot, &assets, false);
        let seed = shot_seed(project, episode, &shot.id);
        let (w, h) = RenderSettings { resolution: "480p".into(), ..Default::default() }.size("16:9");
        println!("第 {num} 镜 {} 秒，参考图 {} 张 {:?}，种子 {seed}，{w}×{h}", shot.duration, refs.len(), refs.iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect::<Vec<_>>());
        let only = std::env::var("ANIMA_AB_ONLY").unwrap_or_default();
        for (name, prompt) in [("A", &a.video), ("B", &b_video)] {
            if !only.is_empty() && !only.eq_ignore_ascii_case(name) {
                continue;
            }
            let out = out_dir.join(format!("shot{num}-{name}.mp4"));
            let t = std::time::Instant::now();
            let r = crate::agent_runtime::video_tool::render_shot_video_reference_with(&refs, &out, prompt, shot.duration as f64, w, h, seed, true, Some(crate::agent_runtime::weetodd::H3Tier::Staged), None).await;
            println!("线路 {name}：{:.1} 分钟 {:?} → {}", t.elapsed().as_secs_f32() / 60.0, r.err(), out.display());
        }
    }
}

#[cfg(test)]
mod stitch_live {
    use super::*;

    /// 修 2026-09-14 以前烧进去的字幕（带角色名、多行同时出现）：从 ComfyUI 输出目录按种子找回
    /// 没烧字幕的原片，用新的切分重烧，覆盖出片文件。旧文件先挪到 `ANIMA_SUB_BACKUP`。
    #[test]
    #[ignore = "改真实项目的出片文件"]
    fn 重烧字幕() {
        let config_dir = dirs::config_dir().unwrap().join("com.anima.desktop");
        let (project, episode) = ("drama_1789250410", "ep_01");
        let backup = PathBuf::from(std::env::var("ANIMA_SUB_BACKUP").expect("要设 ANIMA_SUB_BACKUP"));
        std::fs::create_dir_all(&backup).unwrap();
        let comfy_out = dirs::home_dir().unwrap().join("AI/ComfyUI/output/Anima");
        let ep_dir = config_dir.join("projects").join(project).join("episodes").join(episode);
        let ep: EpisodeData = serde_json::from_str(&std::fs::read_to_string(ep_dir.join("episode.json")).unwrap()).unwrap();
        for shot in &ep.storyboard {
            let (Some(url), false) = (shot.video_url.as_deref(), shot.dialogue.trim().is_empty()) else { continue };
            let Some(take) = shot.takes.iter().find(|t| t.path == url) else { continue };
            let raw = comfy_out.join(format!("cinema_{}.mp4", take.seed));
            if !raw.is_file() {
                println!("第 {} 镜：找不到原片 {}", shot.shot_num, raw.display());
                continue;
            }
            let dst = PathBuf::from(url);
            let cues = crate::subtitle::cues_from_dialogue(&shot.dialogue, shot.duration.max(1.0) as f64);
            let tmp = dst.with_extension("resub.mp4");
            crate::subtitle::burn_in(&raw, &cues, &tmp).unwrap();
            std::fs::rename(&dst, backup.join(dst.file_name().unwrap())).unwrap();
            std::fs::rename(&tmp, &dst).unwrap();
            println!("第 {} 镜：{} 条字幕 → {}", shot.shot_num, cues.len(), dst.display());
        }
    }

    /// 合并形象后核对真实项目：每镜挂的参考图、分镜检查报的问题
    #[test]
    #[ignore = "读真实项目"]
    fn 合并后核对() {
        let cfg = dirs::config_dir().unwrap().join("com.anima.desktop");
        let project = std::env::var("ANIMA_LIVE_PROJECT").unwrap_or_else(|_| "drama_1789250410".into());
        let episode = std::env::var("ANIMA_LIVE_EPISODE").unwrap_or_else(|_| "ep_01".into());
        let (project, episode) = (project.as_str(), episode.as_str());
        let ep_dir = cfg.join("projects").join(project).join("episodes").join(episode);
        let ep: EpisodeData = serde_json::from_str(&std::fs::read_to_string(ep_dir.join("episode.json")).unwrap()).unwrap();
        let assets = crate::storyboard_gen::load_assets(&cfg, project, episode);
        for shot in &ep.storyboard {
            let refs = render_reference_paths(&cfg, project, episode, &ep_dir.join("shots"), shot, &assets, false);
            let names: Vec<String> = crate::looks::for_shot(&assets, shot).iter().filter(|a| shot.all_character_ids().contains(&a.id)).map(|a| format!("{}[{}]", a.name, if a.look_id.is_empty() { "主" } else { &a.look_id })).collect();
            println!("第 {} 镜 {:?} 参考图 {:?}", shot.shot_num, names, refs.iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect::<Vec<_>>());
        }
        for i in validate_storyboard(&cfg, project, episode, &ep) {
            println!("问题：第 {} 镜 {} {}", i.shot_num, i.field, i.message);
        }
    }

    /// 整集超分合成实跑。**不写回项目**：项目和分集文件复制进临时目录（片子按绝对路径引用，不用复制），
    /// 只把 `upscaled/` 链回真实分集目录，放大过的镜头之后在应用里合成时直接复用。
    ///
    /// 环境变量：`ANIMA_STITCH_OUT`（临时配置目录）。
    #[tokio::test]
    #[ignore = "要本机 ComfyUI，逐镜超分，整集几十分钟"]
    async fn 整集超分合成() {
        let config_dir = dirs::config_dir().unwrap().join("com.anima.desktop");
        let (project, episode) = ("drama_1789250410", "ep_01");
        let real = config_dir.join("projects").join(project);
        let tmp = PathBuf::from(std::env::var("ANIMA_STITCH_OUT").unwrap_or_else(|_| std::env::temp_dir().join("anima-stitch-live").to_string_lossy().to_string()));
        let ep_dir = tmp.join("projects").join(project).join("episodes").join(episode);
        std::fs::create_dir_all(&ep_dir).unwrap();
        std::fs::copy(real.join("project.json"), tmp.join("projects").join(project).join("project.json")).unwrap();
        std::fs::copy(real.join("episodes").join(episode).join("episode.json"), ep_dir.join("episode.json")).unwrap();
        let real_up = real.join("episodes").join(episode).join("upscaled");
        std::fs::create_dir_all(&real_up).unwrap();
        let link = ep_dir.join("upscaled");
        if !link.exists() {
            std::os::unix::fs::symlink(&real_up, &link).unwrap();
        }
        std::env::set_current_dir(&tmp).unwrap();
        let t = std::time::Instant::now();
        let r = stitch_episode(&tmp, project.into(), episode.into(), true, false, |done, total, label| {
            println!("[{:.0}s] {done}/{total} {label}", t.elapsed().as_secs_f32());
        })
        .await;
        println!("{:.1} 分钟 {:?} → {}", t.elapsed().as_secs_f32() / 60.0, r, ep_dir.join("final.mp4").display());
        r.unwrap();
    }
}

#[cfg(test)]
mod continuity_wiring_tests {
    /// **接上一镜拿不到片子时必须报错，不能静默降级。**
    ///
    /// 静默降级的后果：合成那一步已经按 `continuity` 把 `<Video 1>` 写进了
    /// 提示词，而出片时没传视频 —— H3 收到一个指向空处的标签，
    /// 出来的东西说不清哪儿不对，**而且不报错**。
    ///
    /// 这条钉的是出片路径里那几句 `ok_or_else` 还在，以及
    /// 「视频参考」那一档真的把 `cont` 传下去了。
    /// **老字段 `continues_previous` 要折进新的 `Continuity`。**
    ///
    /// 两个字段各读各的就是两份真相 —— 用户在界面上选「不接」，
    /// 而老数据里那个 true 还在，出片照样去拿上一镜的末帧。
    #[test]
    fn 老数据的接上一镜折进新字段() {
        use super::*;
        // 老 episode.json：只有 continues_previous，没有 continuity
        let mut s: StoryboardShot = serde_json::from_value(serde_json::json!({
            "id": "s1", "shot_num": 2, "scale": "", "camera_movement": "", "dialogue": "",
            "prompt": "", "duration": 5.0, "status": "pending", "continues_previous": true
        })).unwrap();
        assert_eq!(s.effective_continuity(), Continuity::FirstFrame, "老数据没折进来");

        // 新字段设过了就听新的 —— 用户在界面上改了，不能被老字段翻回去
        s.continuity = Continuity::RefVideo;
        assert_eq!(s.effective_continuity(), Continuity::RefVideo);

        // 干净的老数据（两个都没有）仍然是「不接」
        let clean: StoryboardShot = serde_json::from_value(serde_json::json!({
            "id": "s1", "shot_num": 1, "scale": "", "camera_movement": "", "dialogue": "",
            "prompt": "", "duration": 5.0, "status": "pending"
        })).unwrap();
        assert_eq!(clean.effective_continuity(), Continuity::None);
    }

    /// **不许直接读 `continues_previous`。** 读了就绕过了上面那条折叠。
    #[test]
    fn 没有人直接读老字段() {
        for (name, src) in [
            ("cinema.rs", include_str!("cinema.rs")),
            ("prompt_input.rs", include_str!("prompt_input.rs")),
            ("prompt_route_b.rs", include_str!("prompt_route_b.rs")),
        ] {
            // **运行时拼出来** —— 写成字面量的话这个测试自己的源码会命中自己
            let read = ".continues_".to_string() + "previous";
            let assign = "continues_".to_string() + "previous:";
            for (i, line) in src.lines().enumerate() {
                let l = line.trim();
                // 注释、字段声明、结构体字面量里的赋值、测试断言都不算「读」
                if !l.contains(&read) || l.starts_with("//") {
                    continue;
                }
                // 折叠点本身当然要读它；它下一行带着 [折叠点] 记号
                if src.lines().nth(i + 1).is_some_and(|n| n.contains("[折叠点]")) {
                    continue;
                }
                assert!(
                    l.contains("assert") || l.contains(&assign),
                    "{name}:{} 绕过了 effective_continuity() 直接读老字段：{l}",
                    i + 1
                );
            }
        }
    }

    #[test]
    fn 上一镜没出片时出片要停下来() {
        let src = include_str!("cinema.rs");
        let at = src.find("let continuity = match shot.effective_continuity() {").expect("接线不见了");
        let block = &src[at..at + 2200];
        for need in [
            "它前面没有镜头",          // 第一镜设了衔接
            "还没出片",                // 上一镜没有 take
            "那条片子的文件不在了",    // take 记着但文件没了
        ] {
            assert!(block.contains(need), "少了一种停下来的理由：{need}");
        }
        assert!(
            !block.contains("unwrap_or_default()") && !block.contains("unwrap_or(None)"),
            "出现了静默兜底 —— 拿不到片子就该停，不该当成不接"
        );
        // 真的传下去了（光有 continuity 变量没用）
        assert!(
            src.contains("Some(render.quality.tier()), cont)"),
            "算出来的衔接参数没传进出片调用"
        );
    }
}

#[cfg(test)]
mod advice_field_tests {
    /// **建议类的问题不许挂 `prompts` 字段。**
    ///
    /// 2026-09-20 出过事：三条启发式建议（二选一、表演指示丢了、说话人串了）
    /// 挂了 `field: "prompts"`，而前端把任何 `prompts` 问题都当成
    /// 「合成之后改过字段，要先重新合成」—— 正文里出现一个 `or`
    /// （哪怕是 `never move or fall` 这种否定并列），这一镜就进不了批量出片。
    /// 八镜全中时**整集一镜都出不了**，而界面给的理由
    /// （「改过字段或资产图」）和真实原因毫无关系。
    ///
    /// 这条钉的是**字段名**，不是措辞 —— 措辞会改，
    /// 「建议不能挡出片」这件事不能改。前端那半边由
    /// `storyboardView.test.ts` 的同名测试钉。
    #[test]
    fn 建议类问题不挂会挡出片的字段() {
        let src = include_str!("cinema.rs");
        let at = src
            .find("prompt_route_b::advisories(&ctx, p)")
            .expect("找不到 advisories 的调用点 —— 接线改了，这条测试要跟着改");
        // 按字符边界截：这一段里全是中文注释，按字节截会切在一个汉字中间
        let mut end = (at + 2000).min(src.len());
        while !src.is_char_boundary(end) {
            end += 1;
        }
        let block = &src[at..end];
        let push = block.find("issues.push").expect("advisories 的结果没有进 issues");
        let stmt = &block[push..];
        let end = stmt.find(");").unwrap_or(stmt.len());
        assert!(
            stmt[..end].contains("field: \"advice\""),
            "advisories 的 issue 换了字段 —— 只要不是 advice 就会挡住批量出片：{}",
            &stmt[..end]
        );
    }
}

#[cfg(test)]
mod pulid_file_tests {
    use super::*;

    /// 默认一致性模式是 pulid —— 这里指错文件，带参考图的 SDXL 设定卡就全部失败。
    #[test]
    fn pulid模式用节点读得了的那份权重() {
        let shot = crate::cinema_sheet::expression("AWPortrait_XL_ver1.1.safetensors", "1girl", "smiling", Some("r.png".into()), 1, "p");
        let tuned = tune_character_sheet_shot(shot, "pulid", None, false);
        match tuned.identity_mode {
            crate::cinema_sheet::IdentityMode::Pulid { file } => {
                assert_eq!(file, "ip-adapter_pulid_sdxl_fp16.safetensors");
            }
            other => panic!("SDXL + pulid 应该走 PuLID，实际是 {other:?}"),
        }
    }

    /// 装了 ComfyUI 的机器上，这份文件必须真的在。没装就跳过。
    #[test]
    fn pulid权重在磁盘上() {
        let Some(home) = dirs::home_dir() else { return };
        let dir = home.join("AI/ComfyUI/models/pulid");
        if !dir.is_dir() {
            return;
        }
        assert!(dir.join(PULID_SDXL_FILE).is_file(), "{} 下没有 {PULID_SDXL_FILE}", dir.display());
    }
}

#[cfg(test)]
mod aspect_tests {
    #[test]
    fn 画幅只改这一个键_只认清单里的() {
        let d = tempfile::tempdir().unwrap();
        let dir = super::cinema_projects_dir(d.path()).join("p1");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("project.json"), serde_json::json!({
            "id": "p1", "title": "t", "genre": "", "aspect_ratio": "9:16", "description": "", "created_at": 0, "updated_at": 0, "episode_count": 1, "cover_image": null, "unknown_key": 7
        }).to_string()).unwrap();
        let p = super::set_aspect_ratio(d.path(), "p1", "16:9").unwrap();
        assert_eq!(p.aspect_ratio, "16:9");
        let raw: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("project.json")).unwrap()).unwrap();
        assert_eq!(raw["unknown_key"], 7, "别的键要原样留着");
        assert!(super::set_aspect_ratio(d.path(), "p1", "21:9").is_ok(), "宽银幕要能设");
        assert!(super::set_aspect_ratio(d.path(), "p1", "5:2").is_err());
    }

    /// 21:9 以前被「不是 16:9 就当竖屏」静默出成竖的。
    #[test]
    fn 各种画幅的尺寸方向和比例都对() {
        for a in super::SERIES_ASPECTS {
            let (x, y) = super::aspect_parts(a).unwrap();
            for (w, h) in [super::shot_render_size(a), super::episode_frame(a, false), super::episode_frame(a, true), crate::cover::cover_size(a)] {
                assert_eq!(w >= h, x >= y, "{a} 方向错了：{w}x{h}");
                let want = x / y;
                let got = w as f64 / h as f64;
                assert!((got - want).abs() / want < 0.08, "{a} 比例偏了：{w}x{h}");
            }
        }
        assert_eq!(super::episode_frame("21:9", true), (1920, 822));
    }

    /// 和前端出片设置里的比例清单是同一组，别两边各改各的。
    #[test]
    fn 画幅清单和前端一致() {
        let ts = std::fs::read_to_string("../src/components/studio/storyboard/renderOptions.ts").unwrap();
        let line = ts.lines().find(|l| l.contains("export const ASPECTS")).expect("找不到 ASPECTS");
        for a in super::SERIES_ASPECTS {
            assert!(line.contains(&format!("\"{a}\"")), "前端比例清单里没有 {a}");
        }
    }
}

#[cfg(test)]
mod stale_save_tests {
    use super::*;

    fn shot(id: &str) -> StoryboardShot {
        serde_json::from_value(serde_json::json!({
            "id": id, "shot_num": 1, "scale": "medium", "camera_movement": "static", "dialogue": "",
            "prompt": "", "duration": 8.0, "status": "pending"
        })).unwrap()
    }

    /// 2026-09-26：分镜在应用外重新生成后，界面拿着旧的那份整集保存，新分镜被整个盖掉。
    #[test]
    fn 旧分镜不许盖新分镜() {
        let fresh = vec![shot("n1"), shot("n2")];
        let old = vec![shot("o1"), shot("o2"), shot("o3")];
        assert!(storyboard_is_stale(&fresh, &old), "一个 id 都对不上 = 界面拿的是重新生成之前的");
        // 正常编辑：删一镜、加一镜，id 还有重合
        assert!(!storyboard_is_stale(&fresh, &[shot("n1"), shot("x9")]));
        assert!(!storyboard_is_stale(&fresh, &[shot("n2")]));
        // 新建 / 清空都是正常操作
        assert!(!storyboard_is_stale(&[], &old));
        assert!(!storyboard_is_stale(&fresh, &[]));
    }
}

#[cfg(test)]
mod live_teaser {
    /// 实跑：用 H3 参考生视频出一条独立的片子（预告片这类不属于任何一集的东西），和正片出片走同一个函数。
    ///
    /// `ANIMA_TEASER_DIR` 里放 `prompt.txt`；`ANIMA_TEASER_REFS` 是逗号分隔的参考图路径（顺序 = `<Picture N>`）；
    /// `ANIMA_TEASER_SECS` 默认 15；`ANIMA_TEASER_ASPECT` 默认 9:16；`ANIMA_TEASER_RES` 默认 480p。输出 `teaser.mp4`。
    #[tokio::test]
    #[ignore = "要本机 ComfyUI + H3，出一条约半小时"]
    async fn 预告片实跑() {
        let dir = std::path::PathBuf::from(std::env::var("ANIMA_TEASER_DIR").expect("ANIMA_TEASER_DIR"));
        let prompt = std::fs::read_to_string(dir.join("prompt.txt")).unwrap();
        let refs: Vec<std::path::PathBuf> = std::env::var("ANIMA_TEASER_REFS").unwrap().split(',').map(|p| p.trim().into()).collect();
        for r in &refs {
            assert!(r.is_file(), "参考图不在：{}", r.display());
        }
        let secs: f64 = std::env::var("ANIMA_TEASER_SECS").ok().and_then(|x| x.parse().ok()).unwrap_or(15.0);
        let aspect = std::env::var("ANIMA_TEASER_ASPECT").unwrap_or_else(|_| "9:16".into());
        let render = super::RenderSettings { resolution: std::env::var("ANIMA_TEASER_RES").unwrap_or_else(|_| "480p".into()), ..Default::default() };
        let (w, h) = render.size(&aspect);
        println!("{w}×{h}，{secs} 秒，{} 张参考图，质量档 {:?}", refs.len(), render.quality);
        let out = dir.join("teaser.mp4");
        let t = std::time::Instant::now();
        crate::agent_runtime::video_tool::render_shot_video_reference_with(&refs, &out, &prompt, secs, w, h, 20260926, true, Some(render.quality.tier()), None)
            .await
            .unwrap();
        println!("用时 {:.0}s → {}", t.elapsed().as_secs_f32(), out.display());
    }
    /// 听一遍出出来的片子说了什么：`ANIMA_TEASER_DIR/audio.wav`（16k 单声道 16 位），用项目自己的 Whisper 转写。
    #[tokio::test]
    #[ignore = "要本机 Whisper 模型"]
    async fn 预告片转写() {
        use crate::stt::SpeechToText;
        let dir = std::path::PathBuf::from(std::env::var("ANIMA_TEASER_DIR").expect("ANIMA_TEASER_DIR"));
        let bytes = std::fs::read(dir.join("audio.wav")).unwrap();
        let pcm: Vec<f32> = bytes[44..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0).collect();
        let model = dirs::config_dir().unwrap().join("com.anima.desktop").join("models").join("ggml-small.bin");
        let stt = crate::stt::whisper::WhisperStt::load(crate::stt::SttConfig {
            model_path: model.to_string_lossy().into(), language: "zh".into(), threads: 0, translate: false,
        }).unwrap();
        let t = stt.transcribe(&pcm).await.unwrap();
        println!("转写：{}（平均概率 {:.2}）", t.text, t.confidence);
    }
}

#[cfg(test)]
mod live_shot_table {
    /// 实跑：用**当前代码**的镜头表提示词给某一集重新生成分镜，**写回项目**（和界面上点「生成镜头」同一个函数）。
    ///
    /// 用处：改了镜头表槽位之后，不必等用户构建新版就能出一版给他看。
    /// `ANIMA_LIVE_PROJECT`、`ANIMA_LIVE_EPISODE` 必填。会覆盖这一集现有的分镜 —— 跑之前先备份。
    #[tokio::test]
    #[ignore = "要本机后台模型，而且会改写真实项目"]
    async fn 生成镜头实跑() {
        let config_dir = dirs::config_dir().unwrap().join("com.anima.desktop");
        let pid = std::env::var("ANIMA_LIVE_PROJECT").expect("ANIMA_LIVE_PROJECT");
        let eid = std::env::var("ANIMA_LIVE_EPISODE").expect("ANIMA_LIVE_EPISODE");
        let app_cfg = crate::config::AppConfig::load(&config_dir.join("config.json")).unwrap();
        println!("后台模型：{:?}", app_cfg.background_model);
        crate::llm_memory::ensure_background_loaded(&config_dir).unwrap();
        let gateway = std::sync::Arc::new(crate::llm::gateway::Gateway::new(app_cfg).unwrap());
        // `ANIMA_LIVE_FIX=1`：不调模型，只对现有分镜跑代码修正（台词贴回原句、头衔名字认回资产），打印剩下的问题
        // `ANIMA_LIVE_SCENES=3,6`：只重新生成这几场（和界面上「重新生成这一场」同一个函数，带着现有问题重写）
        let ep_file = super::cinema_projects_dir(&config_dir).join(&pid).join("episodes").join(&eid).join("episode.json");
        if std::env::var("ANIMA_LIVE_FIX").is_ok() {
            let mut ep: super::EpisodeData = serde_json::from_str(&std::fs::read_to_string(&ep_file).unwrap()).unwrap();
            let assets = crate::storyboard_gen::load_assets(&config_dir, &pid, &eid);
            let scenes = crate::storyboard_gen::split_by_headings(&ep.script_text).unwrap();
            let (mut snapped, mut linked) = (0, 0);
            for (i, sc) in scenes.iter().enumerate() {
                let no = i as u32 + 1;
                let mut part: Vec<super::StoryboardShot> = ep.storyboard.iter().filter(|s| s.scene_no == no).cloned().collect();
                snapped += crate::storyboard_gen::snap_lines_to_script(&mut part, &sc.text);
                linked += crate::storyboard_gen::relink_titled(&mut part, &assets);
                crate::storyboard_gen::repair_shots(&mut part, &assets);
                let mut issues = crate::storyboard_gen::validate_shots(&part, &assets);
                issues.extend(crate::storyboard_gen::dialogue_coverage(&sc.text, &part));
                issues.extend(crate::storyboard_gen::source_coverage(&sc.text, &part));
                println!("第 {no} 场「{}」：{} 镜 {} 秒，剩 {} 个问题", sc.name, part.len(), part.iter().map(|s| s.duration).sum::<f32>(), issues.len());
                for i in &issues {
                    println!("    第 {} 镜：{}", i.shot_num, i.message);
                }
                for s in part {
                    if let Some(slot) = ep.storyboard.iter_mut().find(|x| x.id == s.id) {
                        *slot = s;
                    }
                }
            }
            println!("台词贴回原句 {snapped} 处，名字认回资产 {linked} 镜");
            std::fs::write(&ep_file, serde_json::to_string_pretty(&ep).unwrap()).unwrap();
            return;
        }
        // `ANIMA_LIVE_SYNTH=22,23`：只按线路 B 重新合成这几镜（镜号），和界面上「合成提示词」同一个函数；`all` = 整集
        if let Ok(list) = std::env::var("ANIMA_LIVE_SYNTH") {
            let ep: super::EpisodeData = serde_json::from_str(&std::fs::read_to_string(&ep_file).unwrap()).unwrap();
            let ids: Vec<String> = if list.trim() == "all" {
                Vec::new()
            } else {
                let nums: Vec<u32> = list.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                ep.storyboard.iter().filter(|s| nums.contains(&s.shot_num)).map(|s| s.id.clone()).collect()
            };
            let gw = gateway.clone();
            let cfg = crate::prompts::load(&config_dir);
            let call = |slot: crate::prompts::PromptSlot, vars: Vec<(String, String)>| {
                let gw = gw.clone();
                let cfg = cfg.clone();
                async move {
                    let vars: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                    crate::prompts::run(&gw, &cfg, slot, &vars).await
                }
            };
            let t = std::time::Instant::now();
            let r = super::synthesize_episode_prompts(&config_dir, &pid, &eid, crate::shot_table::PromptRoute::B, &ids, call, |d, n, l| println!("合成 {d}/{n} {l}"))
                .await
                .unwrap();
            println!("合成用时 {:.0}s：{}/{}", t.elapsed().as_secs_f32(), r.done, r.total);
            for n in &r.notes {
                println!("  没通过：{n}");
            }
            for n in &r.remarks {
                println!("  备注：{n}");
            }
            return;
        }
        if let Ok(list) = std::env::var("ANIMA_LIVE_SCENES") {
            let gw = gateway.clone();
            let cfg = crate::prompts::load(&config_dir);
            let call = |slot: crate::prompts::PromptSlot, vars: Vec<(String, String)>| {
                let gw = gw.clone();
                let cfg = cfg.clone();
                async move {
                    let vars: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                    crate::prompts::run(&gw, &cfg, slot, &vars).await
                }
            };
            for no in list.split(',').filter_map(|x| x.trim().parse::<u32>().ok()) {
                let t = std::time::Instant::now();
                match super::regenerate_scene(&config_dir, &pid, &eid, no, &call).await {
                    Ok(r) => println!("第 {no} 场重新生成：{:.0}s，{r:?}", t.elapsed().as_secs_f32()),
                    Err(e) => println!("第 {no} 场重新生成失败：{e}"),
                }
            }
            return;
        }
        let t = std::time::Instant::now();
        let r = super::generate_shot_table_job(None, &config_dir, gateway, &pid, &eid, "__live__").await.unwrap();
        println!("用时 {:.0}s：{} 场 {} 镜，{} 个问题", t.elapsed().as_secs_f32(), r.scenes, r.shots, r.issues.len());
        for i in &r.issues {
            println!("  问题：{i:?}");
        }
        for n in &r.notes {
            println!("  说明：{n}");
        }
    }
}

// 放在文件末尾：不少钉子测试按「第一个 #[cfg(test)] 之前」截生产代码，
// 测试模块插在中间会把后面的生产代码全截掉（2026-09-27 51e293c3 就这样截断过）。
#[cfg(test)]
mod director_camera_binding_tests {
    use super::StoryboardShot;

    #[test]
    fn source_pin_tests_see_all_production_code() {
        // 钉子测试用「第一个 #[cfg(test)] 之前」当生产代码。测试模块插在文件中间，
        // 后面的生产代码就被截掉，断言「不包含 X」的钉子会假绿（2026-09-27 51e293c3）。
        let src = include_str!("cinema.rs");
        let first_test = src.find("#[cfg(test)]").unwrap();
        for f in ["pub async fn cinema_render_shot_video_with_config_dir", "async fn render_shot_video_body", "pub async fn render_batch", "fn studio_frame_args"] {
            assert!(src.find(f).unwrap() < first_test, "{f} 在第一个 #[cfg(test)] 之后 —— 测试模块别插在生产代码中间");
        }
    }

    /// **只存这一镜**：后台刚细化 / 合成写回的别的镜头不被界面的旧版本盖掉；
    /// 这一镜的时间段、提示词做三方比较（2026-09-28 审查查出的整集保存覆盖）。
    #[test]
    fn save_shot_merges_background_writes() {
        use super::*;
        let dir = tempfile::tempdir().unwrap();
        let ep_dir = cinema_projects_dir(dir.path()).join("p1/episodes/e1");
        std::fs::create_dir_all(&ep_dir).unwrap();
        let shot = |id: &str, n: u32, visual: &str| -> StoryboardShot {
            let mut s: StoryboardShot = serde_json::from_value(serde_json::json!({"id":id,"shot_num":n,"scale":"","camera_movement":"","dialogue":"","prompt":"","duration":5.0,"status":"pending"})).unwrap();
            s.segments = vec![crate::shot_table::ShotSegment { start: 0.0, end: 5.0, visual: visual.into(), ..Default::default() }];
            s
        };
        let write = |shots: Vec<StoryboardShot>| {
            let ep: EpisodeData = serde_json::from_value(serde_json::json!({"id":"e1","episode_num":1,"title":"t","synopsis":"","script_text":"","storyboard_version":STORYBOARD_VERSION,"storyboard":shots})).unwrap();
            std::fs::write(ep_dir.join("episode.json"), serde_json::to_string(&ep).unwrap()).unwrap();
        };
        let disk = |id: &str| read_episode(dir.path(), "p1", "e1").unwrap().storyboard.into_iter().find(|s| s.id == id).unwrap();

        // 界面打开时两镜都是旧的；之后后台把 s2 细化了、s1 出了片
        let (s1, s2) = (shot("s1", 1, "旧"), shot("s2", 2, "旧"));
        let mut s1_disk = s1.clone();
        s1_disk.video_url = Some("/v/s1.mp4".into());
        write(vec![s1_disk, shot("s2", 2, "细化过的")]);

        // 用户改了 s1 的光：只存 s1；s2 的细化结果、s1 的片都在
        let mut edited = s1.clone();
        edited.lighting = "冷蓝顶光".into();
        save_shot(dir.path(), "p1", "e1", edited, &s1).unwrap();
        assert_eq!(disk("s1").lighting, "冷蓝顶光");
        assert_eq!(disk("s1").video_url.as_deref(), Some("/v/s1.mp4"), "出片结果以盘上为准");
        assert_eq!(disk("s2").segments[0].visual, "细化过的", "别的镜头不能被界面的旧版本盖掉");

        // s2：界面没动时间段、盘上被细化了 → 用盘上的
        let mut e2 = s2.clone();
        e2.duty = "改职责".into();
        save_shot(dir.path(), "p1", "e1", e2, &s2).unwrap();
        assert_eq!(disk("s2").segments[0].visual, "细化过的");
        assert_eq!(disk("s2").duty, "改职责");

        // s2：界面也改了时间段、和盘上不一样 → 不存，报出来，盘上不动
        let mut e3 = s2.clone();
        e3.segments[0].visual = "界面手改".into();
        let err = save_shot(dir.path(), "p1", "e1", e3, &s2).unwrap_err();
        assert!(err.to_string().contains("时间段"), "{err}");
        assert_eq!(disk("s2").segments[0].visual, "细化过的");

        // 从最新的开始改就能存
        let mut e4 = disk("s2");
        let base4 = e4.clone();
        e4.segments[0].visual = "在细化版上再改".into();
        save_shot(dir.path(), "p1", "e1", e4, &base4).unwrap();
        assert_eq!(disk("s2").segments[0].visual, "在细化版上再改");
    }

    /// 工作区拿着「打开导演世界之前」的那份整集保存：导演世界写的场面、机位绑定不能被冲掉。
    #[test]
    fn stale_episode_save_keeps_director_state_from_disk() {
        use super::*;
        let shot = |extra: serde_json::Value| {
            let mut v = serde_json::json!({"id":"s1","shot_num":1,"scale":"","camera_movement":"","dialogue":"","prompt":"","duration":5.0,"status":"pending"});
            v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            v
        };
        let ep = |s: serde_json::Value| -> EpisodeData { serde_json::from_value(serde_json::json!({"id":"e1","episode_num":1,"title":"t","synopsis":"","script_text":"","storyboard_version":STORYBOARD_VERSION,"storyboard":[s]})).unwrap() };
        let disk = ep(shot(serde_json::json!({"director_scene":{"scene":{"actors":[1]}},"director_camera":{"name":"低机位","camera":{},"motion_note":"推"}})));
        let mut stale = ep(shot(serde_json::json!({"lighting":"改了光"})));
        keep_director_state(&disk, &mut stale);
        assert_eq!(stale.storyboard[0].director_scene.as_ref().unwrap()["scene"]["actors"][0], 1);
        assert_eq!(stale.storyboard[0].director_camera.as_ref().unwrap().name, "低机位");
        assert_eq!(stale.storyboard[0].lighting, "改了光", "界面改的字段照常保存");
    }

    #[test]
    fn legacy_camera_json_never_enters_prompt_text() {
        let shot: StoryboardShot = serde_json::from_value(serde_json::json!({
            "id":"s1", "shot_num":1, "scale":"medium", "camera_movement":"push_in",
            "camera_angle":"", "camera_note":"缓慢推近\n[导演机位:旧] {\"camera\":{\"position\":[1,2,3]}}",
            "dialogue":"", "prompt":"", "duration":3.0, "status":"pending",
            "director_camera":{"name":"新机位","camera":{},"motion_note":"3 秒内向前平移 1 米。"}
        })).unwrap();
        let note = shot.camera_prompt_note();
        assert_eq!(note, "缓慢推近\n3 秒内向前平移 1 米。");
        assert!(!note.contains("position"));
        assert!(!note.contains("导演机位"));
    }
}


#[cfg(test)]
mod keyframe_preview_live {
    /// **实跑，只读**：某一镜的资产到底进没进工作流 —— 首帧挂哪几张、尾帧挂哪几张、出片挂哪几张。
    /// `SHOT=8 cargo test --lib keyframe_preview_live -- --ignored --nocapture`（`PROJECT` / `EPISODE` 默认《回收师》EP01）
    #[test]
    #[ignore = "读本机真实项目"]
    fn 这一镜的资产进没进工作流() {
        let config_dir = dirs::config_dir().unwrap().join("com.anima.desktop");
        let project = std::env::var("PROJECT").unwrap_or_else(|_| "drama_1790397900".into());
        let episode = std::env::var("EPISODE").unwrap_or_else(|_| "ep_01".into());
        let num: u32 = std::env::var("SHOT").ok().and_then(|v| v.parse().ok()).unwrap_or(8);
        let ep = super::read_episode(&config_dir, &project, &episode).unwrap();
        let shot = ep.storyboard.iter().find(|s| s.shot_num == num).expect("没有这一镜");
        let shots_dir = super::episode_file(&config_dir, &project, &episode).with_file_name("shots");
        let name = |p: &str| std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        println!("== 第 {num} 镜 · 出场人物 {:?} · 场景 {:?} · 道具 {:?}", shot.all_character_ids(), shot.scene_id, shot.prop_ids);
        println!("== 参考图槽位：{:?}", shot.ref_slots.iter().map(|s| (s.order, s.asset_id.clone(), s.usage)).collect::<Vec<_>>());
        println!("== 关键帧：首帧 {} · 尾帧 {}", shot.keyframes.first, shot.keyframes.last);
        for (label, which) in [("首帧", super::KeyframeSlot::First), ("尾帧", super::KeyframeSlot::Last)] {
            match super::keyframe_preview(&config_dir, &project, &episode, &shot.id, which) {
                Ok(p) => {
                    println!("\n== 出{label}：引擎 {} · 挂 {} 张：{:?}", p.engine, p.images.len(), p.images.iter().map(|i| name(i)).collect::<Vec<_>>());
                    println!("{}", p.prompt);
                }
                Err(e) => println!("\n== 出{label}：出不了 —— {e}"),
            }
        }
        let assets = crate::storyboard_gen::load_assets(&config_dir, &project, &episode);
        let refs = super::render_reference_paths(&config_dir, &project, &episode, &shots_dir, shot, &assets, false);
        if shot.keyframes.first {
            println!("\n== 出片：挂了首帧 → 首帧{}图生视频，**只发关键帧**，资产图不再单独发（要在首帧里画对）", if shot.keyframes.last { "+尾帧" } else { "" });
        } else {
            println!("\n== 出片：没挂首帧 → 参考生视频，挂 {} 张：{:?}", refs.len(), refs.iter().map(|p| name(&p.to_string_lossy())).collect::<Vec<_>>());
        }
        // 资产 id → 名字，方便对上
        for a in &assets {
            if shot.all_character_ids().contains(&a.id) || shot.scene_id.as_deref() == Some(a.id.as_str()) || shot.prop_ids.contains(&a.id) {
                println!("   资产 {} = 「{}」 有图：{}", a.id, a.name, a.has_image);
            }
        }
    }
}
