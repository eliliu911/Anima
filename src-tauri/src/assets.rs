//! 资产库 —— 角色 / 场景 / 道具的统一存储。
//!
//! 这个模块解决的是「资产库不可能参与生成」这个结构问题。旧的
//! `studio_assets` 是对 `.anima/generated/` 的一次 `read_dir` —— 它列的是
//! **文件**，而分镜要引用的是**资产**。文件没有 id，于是没有任何东西
//! 能指向它，于是资产库只能是只读画廊。
//!
//! # 三条硬规则
//!
//! 1. **JSON 里永远不存 base64。** 只存相对 `media/` 的路径。
//!    现存的 `characters.json` 是 86 MB，里面是 71 张 base64 图 ——
//!    读一次角色列表要解析 86 MB，改一个字段要重写 86 MB。
//!    [`assert_no_data_urls`] 有测试钉住这条。
//! 2. **媒体按资产 id 分目录。** 删资产 = 删一个目录，
//!    不用扫全盘找孤儿文件。
//! 3. **不自动囤。** 创作台产物留在 `.anima/generated/`，
//!    只有用户显式「录入」才拷进资产库并获得 id。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// 一个资产最多留多少个**非收藏**变体。超出按创建时间删最旧的。
///
/// 抽卡会让变体无限增长 —— 一次出 4 张、试十轮就是 40 张。
/// 收藏过的永不自动删，那是用户明确说过「这张要留」的。
pub const MAX_VARIANTS: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Character,
    Scene,
    Prop,
}

impl AssetKind {
    pub fn id_prefix(self) -> &'static str {
        match self {
            AssetKind::Character => "char",
            AssetKind::Scene => "scene",
            AssetKind::Prop => "prop",
        }
    }

    /// 提示词内联标签里用的中文名：`[角色:老茶师]`。
    pub fn tag_label(self) -> &'static str {
        match self {
            AssetKind::Character => "角色",
            AssetKind::Scene => "场景",
            AssetKind::Prop => "道具",
        }
    }

    pub fn from_tag_label(s: &str) -> Option<Self> {
        match s {
            "角色" | "character" => Some(AssetKind::Character),
            "场景" | "scene" => Some(AssetKind::Scene),
            "道具" | "prop" => Some(AssetKind::Prop),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaVariant {
    pub id: String,
    /// **相对 store 的 `media/` 目录**，形如 `char_abc/v_123.png`。
    /// 绝对路径会在换机器/改工作区后全部失效，base64 会把 JSON 撑爆。
    pub rel_path: String,
    pub created_at: u64,
    #[serde(default)]
    pub prompt_used: Option<String>,
    /// 收藏的变体不参与 [`MAX_VARIANTS`] 自动清理。
    #[serde(default)]
    pub favorited: bool,
    #[serde(default)]
    pub from_upload: bool,
    /// **这张图在流水线里是什么。**
    ///
    /// 以前变体只有「有没有图」，没有「是哪种图」—— 于是角色的定妆照、
    /// 三视图、场景的主镜和反打全都挤在同一个池子里，谁也说不清哪张能
    /// 拿去干什么。空间契约要的是**主镜 + 反打这一对**，参考生视频要的是
    /// 定妆或三视图，拿错了不报错，只是出来的东西不对。
    ///
    /// 空 = 没标过（所有存量变体都是这个状态，也是「通用参考图」的意思）。
    #[serde(default)]
    pub role: Option<VariantRole>,
}

/// 一个变体在流水线里的用途。
///
/// **这是「带类型的提交」的落点。** 画布出了一张图，它是「随便一张图」
/// 还是「这个场景的反打」，决定了下游能不能用它 —— 而这件事只有提交的
/// 那一刻知道，事后没人认得出来。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VariantRole {
    /// 角色定妆：正面半身，身份的主参考。
    Portrait,
    /// 角色三视图 / 设定卡。
    Turnaround,
    /// 场景主镜（正打）。**和 `SceneReverse` 成对**才喂得了空间契约。
    SceneMaster,
    /// 场景反打。
    SceneReverse,
    /// 场景 2:1 全景。
    ScenePano,
    /// 通用参考图 —— 说不清是哪种的就是这个。
    Reference,
}

impl VariantRole {
    /// 这个角色能落在哪类资产上。**错配要拦住** ——
    /// 把「场景反打」挂到角色身上不报错，只是以后取的时候取不到。
    pub fn allows(self, kind: AssetKind) -> bool {
        match self {
            VariantRole::Portrait | VariantRole::Turnaround => kind == AssetKind::Character,
            VariantRole::SceneMaster | VariantRole::SceneReverse | VariantRole::ScenePano => {
                kind == AssetKind::Scene
            }
            VariantRole::Reference => true,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            VariantRole::Portrait => "角色定妆",
            VariantRole::Turnaround => "角色三视图",
            VariantRole::SceneMaster => "场景主镜",
            VariantRole::SceneReverse => "场景反打",
            VariantRole::ScenePano => "场景全景",
            VariantRole::Reference => "参考图",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssetUnit {
    #[serde(default)]
    pub selected_image_id: Option<String>,
    #[serde(default)]
    pub image_variants: Vec<MediaVariant>,
    #[serde(default)]
    pub selected_video_id: Option<String>,
    #[serde(default)]
    pub video_variants: Vec<MediaVariant>,
    #[serde(default)]
    pub image_prompt: Option<String>,
    #[serde(default)]
    pub video_prompt: Option<String>,
}

impl AssetUnit {
    pub fn selected_image(&self) -> Option<&MediaVariant> {
        let id = self.selected_image_id.as_deref();
        match id {
            Some(id) => self.image_variants.iter().find(|v| v.id == id),
            // 没显式选过就用最新的一张 —— 比返回 None 好，
            // 「刚生成完还没点选」是最常见的状态。
            None => self.image_variants.last(),
        }
    }
}

/// 主形象的 id。主形象就是资产自己的 `description` + `unit`，不在 `looks` 里。
pub const MAIN_LOOK: &str = "main";

/// 角色的一个**形象**：同一个人的不同发型、服装、造型。
///
/// 用户 2026-09-15：一个角色 = 人物本身 + 多个形象，每个形象有自己的描述和图，可以标 NSFW，
/// 项目里选默认形象、单镜可以换。方案见 `docs/plans/2026-09-15-角色形象方案.md`。
///
/// # 为什么主形象不在这个列表里
///
/// 资产的 `description` 和 `unit` 被出图、分镜、提示词、画布、封面几十处直接读。
/// 搬进 `looks[0]` 要么全改（漏一处就静默拿到空描述），要么留一份镜像（两份真相）。
/// 所以主形象就是资产自己，老数据零迁移、老代码读到的就是主形象。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Look {
    pub id: String,
    pub name: String,
    /// 这个形象的发型、服装、妆容、配饰。发给模型时前面拼上人物本身（[`Asset::identity`]）
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub nsfw: bool,
    /// 候选图。和资产的图片池同一个结构、同一套清理规则
    #[serde(default)]
    pub unit: AssetUnit,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub updated_at: u64,
}

/// 角色专属字段。场景/道具为 `None`。
///
/// 这里的东西 LumenX 一样都没有 —— 设定卡槽位、视觉基因、固定种子、
/// 逐角色 LoRA。**不要为了「对齐参考项目」削平。**
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CharacterTraits {
    /// 同一个人的不同视觉版本共享一个 persona（少年张三 / 成年张三）。
    #[serde(default)]
    pub persona: String,
    #[serde(default)]
    pub trigger_words: String,
    #[serde(default)]
    pub seed: Option<i64>,
    #[serde(default)]
    pub lora_model: Option<String>,
    #[serde(default)]
    pub lora_weight: Option<f32>,
    #[serde(default)]
    pub consistency_mode: Option<String>,
    #[serde(default)]
    pub clothing_prompt: Option<String>,
    #[serde(default)]
    pub voice_id: Option<String>,
    /// **给 H3 看的中文声音描述**：音高、音色、语速、口音。
    ///
    /// 和 `voice_id` 不是一回事（那个是点名一份参考音频给 TTS 引擎用）——
    /// 两条音频链路各一个，混用两边都静默失效。
    ///
    /// # 为什么绑在角色上
    ///
    /// 镜头表是**一场一次调用**，模型只看得到本场。要求它「整集 voice 一致」
    /// 等于要求它遵守一个它看不见的约束。实测 E04：李白第 2 镜是
    /// 「嗓音略带沙哑的**中年**男声」，第 8 镜变成「清亮略带醉意的**青年**男声」
    /// —— 不是醉意的差别，是换了个人（2026-09-19 Codex Astra 的审查指出）。
    ///
    /// 所以身份音色放这里（整集一份），逐句的 `DialogueLine::voice`
    /// 只写**这一句的变化**（醉意、压低、气音）。
    #[serde(default)]
    pub h3_voice: Option<String>,
    #[serde(default)]
    pub asset_sheet: Option<crate::cinema::CharacterAssetSheet>,
    #[serde(default)]
    pub visual_dna: Option<crate::cinema_dna::CharacterVisualDna>,
    /// **这个角色没有能出镜的身体。** 旁白、电话那头的人、只有声音的 AI。
    ///
    /// 分镜会读它。不标出来的后果实测过一次：某一集里 AI 角色「零」是
    /// 一团悬浮光点，它每说一句台词，模型就得给它一个镜头，而它没有
    /// 表情、没有姿态、没有景别可换 —— 22 镜里有 9 镜是同一句
    /// 「淡蓝色的光点悬浮着，微微波动」。那不是模型在注水，是我们
    /// 逼它给一个拍不出来的东西找画面。
    ///
    /// 导演的做法是把这种台词做成**画外音**，压在对手戏角色的脸上，
    /// 或者压在它所在的那台设备上。`disembodied` 就是让分镜知道该这么干。
    #[serde(default)]
    pub disembodied: bool,
    /// 槽位化的长相和服装。见 [`crate::character_spec`]。
    ///
    /// **资产的 `description` 是从它渲染出来的**，不是另写的一段。
    /// 两份各写各的话，改了槽位而出图还用旧那段，且不报错。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec: Option<crate::character_spec::CharacterSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    pub id: String,
    pub kind: AssetKind,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub starred: bool,
    /// 锁定的资产不被批量重生成覆盖。
    #[serde(default)]
    pub locked: bool,
    pub created_at: u64,
    pub updated_at: u64,
    #[serde(default)]
    pub unit: AssetUnit,
    #[serde(default)]
    pub character: Option<CharacterTraits>,
    /// 场景资产的**空间契约**：文字地图。和 `character` 是对称的两件事 ——
    /// 一个锁人长什么样，一个锁地方长什么样。
    ///
    /// 只对 [`AssetKind::Scene`] 有意义，但**不在类型上强制**：资产类型是
    /// 可以改的，绑死在 kind 上会让改类型时悄悄丢数据。
    #[serde(default)]
    pub scene: Option<crate::scene_contract::SceneContract>,
    /// 这个资产**出图时**的配置：引擎、画幅、挂哪些 LoRA。
    ///
    /// 空 = 跟项目的生成设置走。三类资产通用 —— 场景和道具一样可能要
    /// 专属 LoRA（比如某个画风的建筑）。
    ///
    /// **不复用 `CharacterTraits.lora_model`。** 那个字段是旧的设定卡路径
    /// 在 `consistency_mode == "lora"` 时才读的，语义是「用 LoRA 锁身份」，
    /// 和这里的「出这张图挂什么」不是一回事。塞在一起就是两个所有者。
    #[serde(default)]
    pub gen: Option<AssetGen>,
    /// **人物本身**：性别、年龄、身高体型、脸、五官、肤色 —— 所有形象共用，只写一次。
    /// 空 = 老数据，`description` 当作主形象的整段完整描述（行为和以前一样）。
    #[serde(default)]
    pub identity: String,
    /// 主形象的名字。空 = 显示「主形象」
    #[serde(default)]
    pub main_look_name: String,
    /// 主形象是不是 NSFW（缩略图默认模糊）
    #[serde(default)]
    pub nsfw: bool,
    /// 主形象之外的形象
    #[serde(default)]
    pub looks: Vec<Look>,
    /// **用户明确说了「只留本集」。** 只对分集级资产有意义。
    ///
    /// 分集级资产默认是「待归属」—— 系列页顶上会一直提醒它还没进系列池。
    /// 标了这个就不再提醒：一次性的路人、只出现一次的场景，
    /// 进系列池反而会让下一集的提取把它当成已有角色。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub episode_only: bool,
}

/// 单个资产的出图配置。每一项都是**覆盖**，`None` = 用项目设置。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AssetGen {
    /// 引擎 id（`zimage` / `sdxl` / …）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    /// 画幅 id（`portrait` / `square` / `landscape`）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect: Option<String>,
    /// 挂的 LoRA。**空数组和 `None` 意思不一样**：
    /// 空数组 = 明确不挂，`None` = 没设过、跟项目走。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loras: Option<Vec<LoraPick>>,
}

/// 一个挂上的 LoRA。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LoraPick {
    /// ComfyUI 枚举出来的名字，**含子目录前缀**（`zimage/xxx.safetensors`）
    pub name: String,
    pub strength: f32,
}

impl Asset {
    pub fn new(kind: AssetKind, name: impl Into<String>) -> Self {
        let now = now_ms();
        Asset {
            id: format!("{}_{}", kind.id_prefix(), short_uuid()),
            kind,
            name: name.into(),
            description: String::new(),
            starred: false,
            locked: false,
            created_at: now,
            updated_at: now,
            unit: AssetUnit::default(),
            character: (kind == AssetKind::Character).then(CharacterTraits::default),
            // **场景不预建空契约**（角色那边预建 traits 是因为里面有
            // voice_id 之类马上要用的字段）。空契约 `is_empty()` 为真，
            // 界面上却会显示「已有空间契约」—— 比没有更糟。
            scene: None,
            gen: None,
            identity: String::new(),
            main_look_name: String::new(),
            nsfw: false,
            looks: Vec::new(),
            episode_only: false,
        }
    }

    /// 这个形象存在吗。`main` 永远存在
    pub fn has_look(&self, look_id: &str) -> bool {
        look_id == MAIN_LOOK || self.looks.iter().any(|l| l.id == look_id)
    }

    /// 形象的图片池。`None` / `main` = 主形象
    pub fn look_unit(&self, look_id: Option<&str>) -> Option<&AssetUnit> {
        match look_id.filter(|l| *l != MAIN_LOOK) {
            None => Some(&self.unit),
            Some(id) => self.looks.iter().find(|l| l.id == id).map(|l| &l.unit),
        }
    }

    pub fn look_unit_mut(&mut self, look_id: Option<&str>) -> Option<&mut AssetUnit> {
        match look_id.filter(|l| *l != MAIN_LOOK) {
            None => Some(&mut self.unit),
            Some(id) => self.looks.iter_mut().find(|l| l.id == id).map(|l| &mut l.unit),
        }
    }

    /// 所有图片池：主形象在前
    pub fn all_units(&self) -> impl Iterator<Item = &AssetUnit> {
        std::iter::once(&self.unit).chain(self.looks.iter().map(|l| &l.unit))
    }

    pub fn all_units_mut(&mut self) -> impl Iterator<Item = &mut AssetUnit> {
        std::iter::once(&mut self.unit).chain(self.looks.iter_mut().map(|l| &mut l.unit))
    }

    /// 发给模型的完整描述：人物本身 + 这个形象的描述。
    ///
    /// | 形象 | 人物本身填了 | 没填（老数据） |
    /// |---|---|---|
    /// | 主形象 | 人物本身 + 主形象描述 | `description` 原样 |
    /// | 其他 | 人物本身 + 形象描述 | 形象名 + 形象描述 |
    ///
    /// 找不到这个形象时按主形象（形象被删了，落回主形象）。
    pub fn look_description(&self, look_id: Option<&str>) -> String {
        let join = |a: &str, b: &str| {
            [a.trim(), b.trim()].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join("\n")
        };
        let look = look_id.filter(|l| *l != MAIN_LOOK).and_then(|id| self.looks.iter().find(|l| l.id == id));
        match look {
            None => join(&self.identity, &self.description),
            Some(l) if self.identity.trim().is_empty() => join(&format!("{}：", l.name.trim()), &l.description),
            Some(l) => join(&self.identity, &l.description),
        }
    }
}

/// 资产存在哪一级。分镜引用资产时**从近到远**解析：分集 → 剧集 → 全局。
///
/// 层级和 LumenX 对得上，只是命名不同：它的 Series 是我们的剧集
/// （`CinemaProject`），它的 Project 是我们的分集（`EpisodeData`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Scope {
    Global,
    Project {
        project_id: String,
    },
    Episode {
        project_id: String,
        episode_id: String,
    },
}

impl Scope {
    /// 这一级的资产索引文件。
    pub fn store_file(&self, config_dir: &Path) -> PathBuf {
        self.store_dir(config_dir).join(match self {
            Scope::Global => "library.json",
            _ => "assets.json",
        })
    }

    pub fn store_dir(&self, config_dir: &Path) -> PathBuf {
        match self {
            Scope::Global => config_dir.join("library"),
            Scope::Project { project_id } => config_dir.join("projects").join(project_id),
            Scope::Episode {
                project_id,
                episode_id,
            } => config_dir
                .join("projects")
                .join(project_id)
                .join("episodes")
                .join(episode_id),
        }
    }

    pub fn media_dir(&self, config_dir: &Path) -> PathBuf {
        self.store_dir(config_dir).join("media")
    }

    /// 界面上显示的归属徽章。
    pub fn label(&self) -> String {
        match self {
            Scope::Global => "全局".into(),
            Scope::Project { project_id } => format!("剧集 {project_id}"),
            Scope::Episode { episode_id, .. } => format!("分集 {episode_id}"),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssetStore {
    #[serde(default)]
    pub assets: Vec<Asset>,
}

// ---------------------------------------------------------------------------
// 读写
// ---------------------------------------------------------------------------

pub fn load_store(config_dir: &Path, scope: &Scope) -> AssetStore {
    let file = scope.store_file(config_dir);
    let Ok(text) = std::fs::read_to_string(&file) else {
        return AssetStore::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

/// 落盘。**写之前先检查有没有 base64。**
///
/// 这一条是防回流的：任何一个新写的上传路径忘了先落文件、
/// 直接把 data URL 塞进字段，地基就白拆了。让它在写盘那一刻就失败，
/// 比几周后发现 JSON 又涨到几百 MB 要好。
pub fn save_store(config_dir: &Path, scope: &Scope, store: &AssetStore) -> Result<()> {
    let text = serde_json::to_string_pretty(store)
        .map_err(|e| Error::config(format!("序列化资产库失败：{e}")))?;
    assert_no_data_urls(&text)?;
    let file = scope.store_file(config_dir);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| Error::config(format!("建立资产库目录失败：{e}")))?;
    }
    std::fs::write(&file, text).map_err(|e| Error::config(format!("写入资产库失败：{e}")))
}

/// 序列化结果里不许出现 data URL。
pub fn assert_no_data_urls(serialized: &str) -> Result<()> {
    if serialized.contains("data:image/") || serialized.contains("data:video/") {
        return Err(Error::runtime(
            "资产库 JSON 里出现了 base64 —— 媒体必须先落盘再存路径（见 assets.rs 模块注释）",
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 变体
// ---------------------------------------------------------------------------

/// 往资产里加一个变体：先把字节落到 `media/<asset_id>/<variant_id>.<ext>`，
/// 再往 JSON 里写**相对路径**。
#[allow(clippy::too_many_arguments)] // 参数都是「写一个变体」这件事本身要的
pub fn add_variant(
    config_dir: &Path,
    scope: &Scope,
    asset: &mut Asset,
    bytes: &[u8],
    ext: &str,
    is_video: bool,
    prompt_used: Option<String>,
    from_upload: bool,
    role: Option<VariantRole>,
) -> Result<MediaVariant> {
    add_variant_to_look(config_dir, scope, asset, None, bytes, ext, is_video, prompt_used, from_upload, role)
}

/// 同 [`add_variant`]，加到指定形象里。`None` / `main` = 主形象。
#[allow(clippy::too_many_arguments)]
pub fn add_variant_to_look(
    config_dir: &Path,
    scope: &Scope,
    asset: &mut Asset,
    look_id: Option<&str>,
    bytes: &[u8],
    ext: &str,
    is_video: bool,
    prompt_used: Option<String>,
    from_upload: bool,
    role: Option<VariantRole>,
) -> Result<MediaVariant> {
    if asset.look_unit(look_id).is_none() {
        return Err(Error::runtime("这个形象不在了 —— 可能已经删掉了"));
    }
    let variant_id = format!("v_{}", short_uuid());
    let rel = format!("{}/{}.{}", asset.id, variant_id, sanitize_ext(ext));
    let abs = scope.media_dir(config_dir).join(&rel);
    if let Some(parent) = abs.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| Error::config(format!("建立媒体目录失败：{e}")))?;
    }
    std::fs::write(&abs, bytes).map_err(|e| Error::config(format!("写入媒体失败：{e}")))?;

    let variant = MediaVariant {
        role,
        id: variant_id.clone(),
        rel_path: rel,
        created_at: now_ms(),
        prompt_used,
        favorited: false,
        from_upload,
    };
    let unit = asset.look_unit_mut(look_id).expect("上面查过形象存在");
    if is_video {
        unit.video_variants.push(variant.clone());
        unit.selected_video_id = Some(variant_id);
    } else {
        unit.image_variants.push(variant.clone());
        unit.selected_image_id = Some(variant_id);
    }
    if let Some(l) = look_id.filter(|l| *l != MAIN_LOOK).and_then(|id| asset.looks.iter_mut().find(|l| l.id == id)) {
        l.updated_at = now_ms();
    }
    asset.updated_at = now_ms();
    prune_variants(config_dir, scope, asset);
    Ok(variant)
}

/// 砍掉超出 [`MAX_VARIANTS`] 的非收藏变体，**连磁盘文件一起删**。
///
/// 选中的那一个也不删 —— 否则「选了一张老图然后抽了十几轮」会把
/// 正在用的图删掉，界面上表现为封面突然空了。
pub fn prune_variants(config_dir: &Path, scope: &Scope, asset: &mut Asset) {
    let media_dir = scope.media_dir(config_dir);
    // 每个形象的图片池各自清理，互不挤占
    for unit in asset.all_units_mut() {
        prune_unit(&media_dir, unit);
    }
}

fn prune_unit(media_dir: &Path, unit: &mut AssetUnit) {
    for is_video in [false, true] {
        let selected = if is_video {
            unit.selected_video_id.clone()
        } else {
            unit.selected_image_id.clone()
        };
        let list = if is_video {
            &mut unit.video_variants
        } else {
            &mut unit.image_variants
        };
        let protected = |v: &MediaVariant| v.favorited || Some(&v.id) == selected.as_ref();
        let removable = list.iter().filter(|v| !protected(v)).count();
        if removable <= MAX_VARIANTS {
            continue;
        }
        let mut to_drop = removable - MAX_VARIANTS;
        let mut dropped: Vec<String> = Vec::new();
        // 已按插入顺序（= 时间顺序）排列，从头删最旧的。
        for v in list.iter() {
            if to_drop == 0 {
                break;
            }
            if protected(v) {
                continue;
            }
            dropped.push(v.rel_path.clone());
            to_drop -= 1;
        }
        list.retain(|v| !dropped.contains(&v.rel_path));
        for rel in dropped {
            let _ = std::fs::remove_file(media_dir.join(rel));
        }
    }
}

/// 变体的绝对路径。给界面（走 `anima-media://`）和生成链路（当参考图）用。
pub fn variant_abs_path(config_dir: &Path, scope: &Scope, variant: &MediaVariant) -> PathBuf {
    scope.media_dir(config_dir).join(&variant.rel_path)
}

// ---------------------------------------------------------------------------
// 媒体协议的路径解析
// ---------------------------------------------------------------------------

/// `anima-media://` 请求的路径解析 —— **这是一道安全边界，不是便利函数。**
///
/// Tauri 自带的 asset 协议在这个项目里配的是 `scope: ["**"]`，
/// 等于把整台机器的读权限交给了 webview，和「前端不持有文件权限」
/// 的既定原则正好相反。这个函数只放行 `roots` 底下的文件。
///
/// 用 `canonicalize` 而不是字符串前缀比较 —— `../../` 和符号链接
/// 都能绕过纯字符串判断，而且**不会报错**。
pub fn resolve_media_path(roots: &[PathBuf], requested: &Path) -> Result<PathBuf> {
    let real = requested
        .canonicalize()
        .map_err(|_| Error::runtime("找不到该媒体文件"))?;
    for root in roots {
        let Ok(root) = root.canonicalize() else {
            continue;
        };
        if real.starts_with(&root) {
            return Ok(real);
        }
    }
    Err(Error::runtime("该路径不在允许的媒体目录内"))
}

/// 自定义协议名。**不用 Tauri 自带的 asset 协议。**
///
/// 两个理由，各自都成立：
/// 1. 现有配置是 `assetProtocol.scope: ["**"]` —— 等于把整台机器的读权限
///    交给 webview，和「前端不持有文件权限」这条既定原则正好相反。
/// 2. 自己的 handler 能在 Rust 里测（见 [`resolve_media_path`] 的用例），
///    自带协议的行为只能靠跑起来看。
pub const MEDIA_SCHEME: &str = "anima-media";

/// 这个文件当前这一版的标识。**内容变了它必须变。**
///
/// 用 mtime + 长度，不算内容哈希 —— 哈希一个 6 MB 的 png 比直接把它
/// 发出去还慢，而这里每次显示都要算一遍。
///
/// 读不到元信息时回一个**每次都不同**的值：那种情况下宁可多传一次，
/// 也不能回一个固定值 —— 回固定值等于告诉浏览器"没变过"，
/// 而我们其实不知道。
pub fn media_etag(path: &Path) -> String {
    match std::fs::metadata(path) {
        Ok(m) => {
            let t = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis())
                .unwrap_or(0);
            format!("\"{}-{}\"", t, m.len())
        }
        Err(_) => format!("\"{}\"", now_ms()),
    }
}

/// 把绝对路径变成界面能直接塞进 `<img src>` 的 URL。
///
/// 路径走 **query 参数**而不是 URL path —— 空格、中文、`#`
/// 在 path 段里各有各的转义坑（`~/Library/Application Support/…`
/// 就带空格），query 只需要一次 percent 编码。
pub fn media_url(abs: &Path) -> String {
    format!(
        "{MEDIA_SCHEME}://localhost/?p={}",
        urlencoding::encode(&abs.to_string_lossy())
    )
}

/// 从请求 URI 里取回绝对路径。解不出就是 `None`（handler 回 400）。
pub fn parse_media_request(uri: &str) -> Option<PathBuf> {
    let query = uri.split('?').nth(1)?;
    for pair in query.split('&') {
        if let Some(raw) = pair.strip_prefix("p=") {
            let decoded = urlencoding::decode(raw).ok()?;
            if decoded.is_empty() {
                return None;
            }
            return Some(PathBuf::from(decoded.as_ref()));
        }
    }
    None
}

/// 解析 `Range: bytes=…`，回闭区间 `(start, end)`。
///
/// - `bytes=0-1` / `bytes=100-` / `bytes=-500`（最后 500 字节）
/// - 开放区间（`100-`）按 [`MAX_RANGE_CHUNK`] 截断 —— 播放器会接着要下一段
/// - 越界、写坏、多段（`0-1,5-9`）回 `None`，调用方回 416 / 整文件
pub fn parse_range(header: &str, len: u64) -> Option<(u64, u64)> {
    if len == 0 {
        return None;
    }
    let spec = header.trim().strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (a, b) = spec.split_once('-')?;
    let (a, b) = (a.trim(), b.trim());
    let (start, end) = if a.is_empty() {
        let n: u64 = b.parse().ok()?;
        if n == 0 {
            return None;
        }
        (len.saturating_sub(n), len - 1)
    } else {
        let start: u64 = a.parse().ok()?;
        let end = if b.is_empty() { start.saturating_add(MAX_RANGE_CHUNK - 1) } else { b.parse().ok()? };
        (start, end.min(len - 1))
    };
    (start <= end && start < len).then_some((start, end))
}

/// 一次最多回多少字节。开放区间不截的话，WebKit 放视频时每次都会把剩下的整个文件要走。
pub const MAX_RANGE_CHUNK: u64 = 8 * 1024 * 1024;

/// `anima-media://` 的响应。
///
/// # 为什么要支持 Range
///
/// 2026-09-15 用户：成片在别的播放器里不卡，在应用里卡得厉害。原来这里**每个请求都把整个文件读出来整个回**，
/// 不认 Range。WebKit 放视频是一段一段要的（先要头两个字节，再按播放进度要），
/// 123 MB 的 1080P 成片于是每次缓冲、每次拖动都整读整传一遍。
/// 现在按 Range 只读那一段、回 206；不带 Range 的（图片）照旧整个回。
pub fn media_response(real: &Path, range: Option<&str>, if_none_match: Option<&str>) -> tauri::http::Response<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let fail = |code: u16, msg: String| {
        tauri::http::Response::builder()
            .status(code)
            .header("Content-Type", "text/plain; charset=utf-8")
            .body(msg.into_bytes())
            .unwrap_or_else(|_| tauri::http::Response::new(Vec::new()))
    };
    let etag = media_etag(real);
    let len = match std::fs::metadata(real) {
        Ok(m) => m.len(),
        Err(e) => return fail(404, format!("读不到文件：{e}")),
    };
    let builder = || {
        tauri::http::Response::builder()
            .header("Content-Type", media_mime(real))
            .header("ETag", &etag)
            .header("Cache-Control", "no-cache")
            .header("Accept-Ranges", "bytes")
    };
    if let Some(range) = range {
        let Some((start, end)) = parse_range(range, len) else {
            return tauri::http::Response::builder()
                .status(416)
                .header("Content-Range", format!("bytes */{len}"))
                .body(Vec::new())
                .unwrap_or_else(|_| tauri::http::Response::new(Vec::new()));
        };
        let mut buf = vec![0u8; (end - start + 1) as usize];
        let read = std::fs::File::open(real)
            .and_then(|mut f| f.seek(SeekFrom::Start(start)).and_then(|_| f.read_exact(&mut buf)));
        if let Err(e) = read {
            return fail(404, format!("读不到文件：{e}"));
        }
        return builder()
            .status(206)
            .header("Content-Range", format!("bytes {start}-{end}/{len}"))
            .header("Content-Length", buf.len().to_string())
            .body(buf)
            .unwrap_or_else(|_| tauri::http::Response::new(Vec::new()));
    }
    if if_none_match.is_some_and(|v| v == etag) {
        return builder().status(304).body(Vec::new()).unwrap_or_else(|_| tauri::http::Response::new(Vec::new()));
    }
    match std::fs::read(real) {
        Ok(bytes) => builder().body(bytes).unwrap_or_else(|_| tauri::http::Response::new(Vec::new())),
        Err(e) => fail(404, format!("读不到文件：{e}")),
    }
}

/// 按扩展名给出 Content-Type。给错的话 WebKit 会拒绝渲染，
/// 而且**不报错** —— 表现就是一个碎图图标。
pub fn media_mime(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        _ => "application/octet-stream",
    }
}

/// 允许 `anima-media://` 读取的根目录。
///
/// `config_dir` 覆盖 `library/`、`projects/`、`.anima/generated/` 三处；
/// 额外再加 ComfyUI 的输出目录和用户工作区（生成产物落在那里）。
pub fn media_roots(config_dir: &Path, extra: &[PathBuf]) -> Vec<PathBuf> {
    let mut roots = vec![config_dir.to_path_buf()];
    roots.extend(extra.iter().cloned());
    roots
}

// ---------------------------------------------------------------------------
// 小工具
// ---------------------------------------------------------------------------

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn short_uuid() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..12].to_string()
}

/// 扩展名只允许字母数字，且限长 —— 它会拼进文件路径。
fn sanitize_ext(ext: &str) -> String {
    let cleaned: String = ext
        .trim_start_matches('.')
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(5)
        .collect::<String>()
        .to_ascii_lowercase();
    if cleaned.is_empty() {
        "png".into()
    } else {
        cleaned
    }
}

// ---------------------------------------------------------------------------
// 引用完整性
// ---------------------------------------------------------------------------

/// 「谁在用这个资产」。删除前必须先问这个问题。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AssetReference {
    pub project_id: String,
    pub project_title: String,
    pub episode_id: String,
    pub episode_title: String,
    pub shot_id: String,
    pub shot_num: u32,
}

/// 扫所有剧集/分集的分镜，找出引用了这个 id 的镜头。
///
/// **不能只靠正则扫提示词。** 提示词里的 `[角色:老茶师]` 是书写形式，
/// 改了资产名字标签就失配了；`character_ids` / `scene_id` / `prop_ids`
/// 才是解析后的索引，保存分镜时从提示词回写。
///
/// 没有这个扫描，删资产不报错、分镜静默失去参考图，
/// 表现是「生成的人突然不像了」—— 这种失败要挖很久才能挖到根因。
pub fn scan_references(config_dir: &Path, asset_id: &str) -> Vec<AssetReference> {
    let mut out = Vec::new();
    let projects_root = config_dir.join("projects");
    let Ok(projects) = std::fs::read_dir(&projects_root) else {
        return out;
    };
    for p in projects.flatten() {
        let proj_dir = p.path();
        if !proj_dir.is_dir() {
            continue;
        }
        let project_id = p.file_name().to_string_lossy().to_string();
        let project_title = std::fs::read_to_string(proj_dir.join("project.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v.get("title").and_then(|t| t.as_str()).map(String::from))
            .unwrap_or_else(|| project_id.clone());

        let Ok(eps) = std::fs::read_dir(proj_dir.join("episodes")) else {
            continue;
        };
        for e in eps.flatten() {
            let ep_json = e.path().join("episode.json");
            let Ok(text) = std::fs::read_to_string(&ep_json) else {
                continue;
            };
            let Ok(ep) = serde_json::from_str::<crate::cinema::EpisodeData>(&text) else {
                continue;
            };
            for shot in &ep.storyboard {
                if shot.references_asset(asset_id) {
                    out.push(AssetReference {
                        project_id: project_id.clone(),
                        project_title: project_title.clone(),
                        episode_id: ep.id.clone(),
                        episode_title: ep.title.clone(),
                        shot_id: shot.id.clone(),
                        shot_num: shot.shot_num,
                    });
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 提示词内联引用：`[角色:老茶师]`
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetRef {
    pub kind: AssetKind,
    pub name: String,
    /// 原始标签文本，用来剥除。
    pub raw: String,
}

/// 从提示词里解析出 `[角色:名字]` / `[场景:名字]` / `[道具:名字]`。
///
/// 抄的是 LumenX 最聪明的一处设计：**提示词文本本身就是绑定关系**，
/// 用户不用「先在下拉框选角色再写提示词」。一次解析喂三个地方 ——
/// 参考图、提示词扩写的上下文、界面上的头像行。
pub fn parse_asset_refs(prompt: &str) -> Vec<AssetRef> {
    let mut out: Vec<AssetRef> = Vec::new();
    let bytes: Vec<char> = prompt.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != '[' {
            i += 1;
            continue;
        }
        let Some(close) = (i + 1..bytes.len()).find(|&j| bytes[j] == ']') else {
            break;
        };
        let inner: String = bytes[i + 1..close].iter().collect();
        // 冒号两种写法都收 —— 中文输入法下打出来的是全角。
        let split = inner.find(':').or_else(|| inner.find('：'));
        if let Some(pos) = split {
            let (label, rest) = inner.split_at(pos);
            let name = rest
                .trim_start_matches(':')
                .trim_start_matches('：')
                .trim()
                .to_string();
            if let Some(kind) = AssetKind::from_tag_label(label.trim()) {
                if !name.is_empty() {
                    let raw: String = bytes[i..=close].iter().collect();
                    // 同一个资产在一句里出现多次只算一次。
                    if !out.iter().any(|r| r.kind == kind && r.name == name) {
                        out.push(AssetRef { kind, name, raw });
                    }
                }
            }
        }
        i = close + 1;
    }
    out
}

/// 送进引擎之前把标签剥掉，只留名字。
///
/// **忘了剥就是一次典型的静默失败**：`[角色:老茶师]` 原样进提示词，
/// 模型把方括号和「角色」两个字当画面内容画进去 ——
/// 出图了、不报错、画面里多一行奇怪的字。
pub fn strip_ref_tags(prompt: &str) -> String {
    let mut out = prompt.to_string();
    for r in parse_asset_refs(prompt) {
        out = out.replace(&r.raw, &r.name);
    }
    out
}

// ---------------------------------------------------------------------------
// 存的是相对路径，显示要的是 URL —— 两头转换
// ---------------------------------------------------------------------------

/// 读出来给界面看：把 `media/` 下的相对路径换成 `anima-media://` URL。
///
/// **只换真的指向媒体目录里存在文件的那些字符串。** 老的预设路径
/// （`/portraits/xiaoya.jpg` 之类，前端 public 目录里的）必须原样放过 ——
/// 一律替换的话那些图会全变成 404。
pub fn hydrate_media_urls(config_dir: &Path, scope: &Scope, value: &mut serde_json::Value) {
    let media_dir = scope.media_dir(config_dir);
    walk_strings(value, &mut |s| {
        if s.starts_with("anima-media:") || s.starts_with("data:") || s.starts_with('/') {
            return;
        }
        let abs = media_dir.join(&*s);
        if abs.is_file() {
            *s = media_url(&abs);
        }
    });
}

/// 写回磁盘之前：把 `anima-media://` URL 换回相对路径。
///
/// 没有这一步，界面把读到的 URL 原样提交回来就会被持久化 ——
/// 存储里混进显示层的东西，和当初把 base64 存进 JSON 是同一类错误。
pub fn dehydrate_media_urls(config_dir: &Path, scope: &Scope, value: &mut serde_json::Value) {
    let media_dir = scope.media_dir(config_dir);
    walk_strings(value, &mut |s| {
        if !s.starts_with(MEDIA_SCHEME) {
            return;
        }
        let Some(abs) = parse_media_request(s) else {
            return;
        };
        if let Ok(rel) = abs.strip_prefix(&media_dir) {
            *s = rel.to_string_lossy().to_string();
        }
    });
}

fn walk_strings(value: &mut serde_json::Value, f: &mut impl FnMut(&mut String)) {
    match value {
        serde_json::Value::String(s) => f(s),
        serde_json::Value::Array(items) => {
            for it in items {
                walk_strings(it, f);
            }
        }
        serde_json::Value::Object(map) => {
            for (_, v) in map.iter_mut() {
                walk_strings(v, f);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// 迁移：把 characters.json 里的 base64 拆出来
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct MigrationReport {
    pub project_id: String,
    /// 落盘的图片数。**判据是这个数，不是「迁移成功」。**
    pub images_written: usize,
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub assets_created: usize,
    /// 解不开的 data URL。空着才算干净。
    pub failures: Vec<String>,
}

/// 把一个短剧项目的 `characters.json` 从 base64 改造成「文件 + 路径」，
/// 并顺带写出剧集级 `assets.json`。
///
/// 现场是 86,433,931 字节的 JSON，里面 71 处 `data:image/`。根因在
/// `cinema_update_character_asset_slot`：PNG 已经写盘了，又把 data URL
/// 塞回 `image_url` 字段。于是读一次角色列表要解析 86 MB。
///
/// **幂等**：已经是路径的字段原样跳过，重复跑不会重复落盘。
pub fn migrate_project_characters(config_dir: &Path, project_id: &str) -> Result<MigrationReport> {
    let proj_dir = config_dir.join("projects").join(project_id);
    let char_file = proj_dir.join("characters.json");
    let bytes_before = std::fs::metadata(&char_file).map(|m| m.len()).unwrap_or(0);

    let text = std::fs::read_to_string(&char_file)
        .map_err(|e| Error::config(format!("读不到 characters.json：{e}")))?;
    let mut chars: Vec<serde_json::Value> = serde_json::from_str(&text)
        .map_err(|e| Error::config(format!("解析 characters.json 失败：{e}")))?;

    let scope = Scope::Project {
        project_id: project_id.to_string(),
    };
    let media_dir = scope.media_dir(config_dir);
    let mut report = MigrationReport {
        project_id: project_id.to_string(),
        images_written: 0,
        bytes_before,
        bytes_after: 0,
        assets_created: 0,
        failures: Vec::new(),
    };

    let mut store = load_store(config_dir, &scope);
    for ch in chars.iter_mut() {
        let char_id = ch
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let mut seq = 0usize;
        rewrite_data_urls(
            ch,
            &media_dir,
            &char_id,
            &mut seq,
            &mut report.images_written,
            &mut report.failures,
        );

        // 同步建一条剧集级资产（幂等：同 id 覆盖）。
        let name = ch
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(&char_id)
            .to_string();
        let mut asset = Asset::new(AssetKind::Character, name);
        asset.id = char_id.clone();
        asset.description = ch
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let mut traits = CharacterTraits {
            trigger_words: ch
                .get("trigger_words")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            seed: ch.get("seed").and_then(|v| v.as_i64()),
            lora_model: ch
                .get("lora_model")
                .and_then(|v| v.as_str())
                .map(String::from),
            lora_weight: ch.get("lora_weight").and_then(|v| v.as_f64()).map(|f| f as f32),
            consistency_mode: ch
                .get("consistency_mode")
                .and_then(|v| v.as_str())
                .map(String::from),
            clothing_prompt: ch
                .get("clothing_prompt")
                .and_then(|v| v.as_str())
                .map(String::from),
            ..Default::default()
        };
        // **解析失败不能静默变成 None。** 设定卡是我们比参考项目强的地方，
        // 它整档消失而界面只显示「(0/6)」，是这个项目最典型的失败样子。
        // `null` 是「没设过」，不是解析失败 —— 对 null 报警会训练人忽略警告。
        let non_null = |v: &serde_json::Value| (!v.is_null()).then(|| v.clone());
        if let Some(v) = ch.get("asset_sheet").and_then(non_null) {
            match serde_json::from_value(v) {
                Ok(sheet) => traits.asset_sheet = Some(sheet),
                Err(e) => report
                    .failures
                    .push(format!("{char_id}: 设定卡解析失败，已丢弃 —— {e}")),
            }
        }
        if let Some(v) = ch.get("visual_dna").and_then(non_null) {
            match serde_json::from_value(v) {
                Ok(dna) => traits.visual_dna = Some(dna),
                Err(e) => report
                    .failures
                    .push(format!("{char_id}: 视觉基因解析失败，已丢弃 —— {e}")),
            }
        }
        asset.character = Some(traits);

        // 封面：先用头像；头像还是老的预设路径（`/portraits/x.jpg`，
        // 不在 media/ 下）时退回设定卡的第一张三视图 ——
        // **不能就这么留空**，资产库卡片是空白封面时用户根本认不出这是谁。
        let cover = ch
            .get("avatar_url")
            .and_then(|v| v.as_str())
            .filter(|rel| media_dir.join(rel).is_file())
            .or_else(|| {
                ch.get("asset_sheet")
                    .and_then(|s| s.get("turnarounds"))
                    .and_then(|t| t.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|it| it.get("image_url"))
                    .and_then(|v| v.as_str())
                    .filter(|rel| media_dir.join(rel).is_file())
            });
        if let Some(rel) = cover {
            let vid = format!("v_{}", short_uuid());
            asset.unit.image_variants.push(MediaVariant {
                role: None,
                id: vid.clone(),
                rel_path: rel.to_string(),
                created_at: now_ms(),
                prompt_used: None,
                favorited: true,
                from_upload: true,
            });
            asset.unit.selected_image_id = Some(vid);
        }

        match store.assets.iter_mut().find(|a| a.id == asset.id) {
            Some(existing) => *existing = asset,
            None => store.assets.push(asset),
        }
        report.assets_created += 1;
    }

    let compacted = serde_json::to_string_pretty(&chars)
        .map_err(|e| Error::config(format!("序列化失败：{e}")))?;
    assert_no_data_urls(&compacted)?;
    std::fs::write(&char_file, &compacted)
        .map_err(|e| Error::config(format!("回写 characters.json 失败：{e}")))?;
    save_store(config_dir, &scope, &store)?;
    report.bytes_after = compacted.len() as u64;
    Ok(report)
}

/// 递归把 JSON 里所有 `data:image/...;base64,...` 字符串换成相对路径。
///
/// **文件名带序号。** 早先按槽位 key 命名，`turnarounds` 和 `poses` 里
/// 同名的 key 会互相覆盖 —— 落盘 71 张只剩 40 张，而且不报错。
fn rewrite_data_urls(
    node: &mut serde_json::Value,
    media_dir: &Path,
    char_id: &str,
    seq: &mut usize,
    written: &mut usize,
    failures: &mut Vec<String>,
) {
    use base64::Engine;
    match node {
        serde_json::Value::String(s) => {
            if !s.starts_with("data:image/") && !s.starts_with("data:video/") {
                return;
            }
            let Some(comma) = s.find(',') else {
                failures.push(format!("{char_id}#{seq}: data URL 没有逗号分隔"));
                return;
            };
            let ext = s[5..comma]
                .split(['/', ';'])
                .nth(1)
                .unwrap_or("png")
                .to_string();
            let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&s[comma + 1..]) else {
                failures.push(format!("{char_id}#{seq}: base64 解码失败"));
                return;
            };
            *seq += 1;
            let rel = format!("{}/legacy_{:03}.{}", char_id, seq, sanitize_ext(&ext));
            let abs = media_dir.join(&rel);
            if let Some(parent) = abs.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if std::fs::write(&abs, &bytes).is_err() {
                failures.push(format!("{char_id}#{seq}: 写盘失败"));
                return;
            }
            *written += 1;
            *s = rel;
        }
        serde_json::Value::Array(items) => {
            for it in items {
                rewrite_data_urls(it, media_dir, char_id, seq, written, failures);
            }
        }
        serde_json::Value::Object(map) => {
            for (_, v) in map.iter_mut() {
                rewrite_data_urls(v, media_dir, char_id, seq, written, failures);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Tauri 命令层
// ---------------------------------------------------------------------------

/// 发给界面的资产：原始字段 + 已解析好的显示 URL + 归属。
///
/// **URL 在后端拼。** 前端只有相对路径的话，就得知道 `media/` 在哪、
/// 作用域怎么映射成目录 —— 那等于把存储布局复制一份到前端，
/// 改布局时两边必然走散。
#[derive(Debug, Clone, Serialize)]
pub struct AssetView {
    #[serde(flatten)]
    pub asset: Asset,
    pub scope: Scope,
    /// 归属的显示名：「全局」/ 项目名 / 「项目名 · 第 1 集」
    pub scope_label: String,
    /// 属于哪个项目（项目名）。全局资产是 `None`。资产库「按项目」按它分组
    pub project_title: Option<String>,
    /// 当前选中图的显示 URL；没有图就是 `None`（界面显示占位，不是碎图）。
    pub cover_url: Option<String>,
    /// 每个图变体的显示 URL，顺序与 `unit.image_variants` 一致。
    pub image_urls: Vec<String>,
    /// 每个图变体的**本机绝对路径**，顺序同上。
    ///
    /// **和 `image_urls` 不是一回事**：URL 是给 `<img src>` 用的（走媒体协议），
    /// 而生成链路要的是真路径（工具会把文件复制进 ComfyUI 的 `input/`）。
    /// 拿 URL 去当路径的表现是「找不到源图」，而那时候看不出是哪一层错的。
    pub image_paths: Vec<String>,
    pub video_urls: Vec<String>,
    /// 有多少个镜头在用它。删除按钮据此变成「查看引用」。
    pub reference_count: usize,
    /// 角色的形象，**主形象在第一个**。场景 / 道具只有主形象这一个
    pub look_views: Vec<LookView>,
}

/// 界面上的一个形象。
#[derive(Debug, Clone, Serialize)]
pub struct LookView {
    pub id: String,
    /// 显示名（主形象没起名时是「主形象」）
    pub name: String,
    pub is_main: bool,
    /// 这个形象自己的描述（主形象就是资产的 description）
    pub description: String,
    /// 发给模型的完整描述（人物本身 + 形象描述）
    pub prompt: String,
    pub nsfw: bool,
    pub unit: AssetUnit,
    pub cover_url: Option<String>,
    pub image_urls: Vec<String>,
    pub image_paths: Vec<String>,
}

/// 归属的显示名。
///
/// 以前是 `Scope::label()`：「剧集 drama_1789250410」「分集 ep_01」—— 只有 id。
/// 2026-09-15 用户：资产库「按项目」无效、项目名不对。两个项目的分集都叫 `ep_01`，
/// 按这个标签分组就把两部剧的资产合进了同一组，标题还是一串 id。
/// 现在读项目名和集数；读不到项目文件时退回 id。
pub fn scope_display(config_dir: &Path, scope: &Scope) -> (String, Option<String>) {
    let project_title = |project_id: &str| {
        std::fs::read_to_string(config_dir.join("projects").join(project_id).join("project.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| v.get("title").and_then(|t| t.as_str()).map(|t| t.trim().to_string()))
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| project_id.to_string())
    };
    match scope {
        Scope::Global => ("全局".into(), None),
        Scope::Project { project_id } => {
            let t = project_title(project_id);
            (t.clone(), Some(t))
        }
        Scope::Episode { project_id, episode_id } => {
            let t = project_title(project_id);
            let ep = std::fs::read_to_string(scope.store_dir(config_dir).join("episode.json"))
                .ok()
                .and_then(|x| serde_json::from_str::<serde_json::Value>(&x).ok())
                .and_then(|v| v.get("episode_num").and_then(|n| n.as_u64()))
                .map(|n| format!("第 {n} 集"))
                .unwrap_or_else(|| episode_id.clone());
            (format!("{t} · {ep}"), Some(t))
        }
    }
}

pub(crate) fn to_view(config_dir: &Path, scope: &Scope, asset: Asset, reference_count: usize) -> AssetView {
    let url_of = |v: &MediaVariant| media_url(&variant_abs_path(config_dir, scope, v));
    let cover_url = asset.unit.selected_image().map(url_of);
    let (scope_label, project_title) = scope_display(config_dir, scope);
    let path_of = |v: &MediaVariant| variant_abs_path(config_dir, scope, v).to_string_lossy().into_owned();
    let look_view = |id: &str, name: String, description: String, nsfw: bool, unit: &AssetUnit| LookView {
        id: id.to_string(),
        is_main: id == MAIN_LOOK,
        prompt: asset.look_description(Some(id)),
        name,
        description,
        nsfw,
        cover_url: unit.selected_image().map(url_of),
        image_urls: unit.image_variants.iter().map(url_of).collect(),
        image_paths: unit.image_variants.iter().map(path_of).collect(),
        unit: unit.clone(),
    };
    let main_name = if asset.main_look_name.trim().is_empty() { "主形象".to_string() } else { asset.main_look_name.clone() };
    let mut look_views = vec![look_view(MAIN_LOOK, main_name, asset.description.clone(), asset.nsfw, &asset.unit)];
    look_views.extend(asset.looks.iter().map(|l| look_view(&l.id, l.name.clone(), l.description.clone(), l.nsfw, &l.unit)));
    AssetView {
        look_views,
        scope_label,
        project_title,
        cover_url,
        image_urls: asset.unit.image_variants.iter().map(url_of).collect(),
        image_paths: asset
            .unit
            .image_variants
            .iter()
            .map(|v| variant_abs_path(config_dir, scope, v).to_string_lossy().into_owned())
            .collect(),
        video_urls: asset.unit.video_variants.iter().map(url_of).collect(),
        scope: scope.clone(),
        reference_count,
        asset,
    }
}

/// 枚举磁盘上所有存在资产的作用域。
pub fn all_scopes(config_dir: &Path) -> Vec<Scope> {
    let mut out = vec![Scope::Global];
    let Ok(projects) = std::fs::read_dir(config_dir.join("projects")) else {
        return out;
    };
    for p in projects.flatten() {
        if !p.path().is_dir() {
            continue;
        }
        let project_id = p.file_name().to_string_lossy().to_string();
        out.push(Scope::Project {
            project_id: project_id.clone(),
        });
        if let Ok(eps) = std::fs::read_dir(p.path().join("episodes")) {
            for e in eps.flatten() {
                if e.path().is_dir() {
                    out.push(Scope::Episode {
                        project_id: project_id.clone(),
                        episode_id: e.file_name().to_string_lossy().to_string(),
                    });
                }
            }
        }
    }
    out
}

/// 列资产。`scope` 不给就列全部作用域 —— 资产库页面默认就是这个视图，
/// 「按类型」和「按项目」两个视图轴用的是同一批数据。
#[tauri::command]
pub fn assets_list(
    state: tauri::State<'_, crate::AppState>,
    scope: Option<Scope>,
) -> Vec<AssetView> {
    list_assets(&state.config_dir, scope)
}

/// [`assets_list`] 的实现。**命令层只是薄壳** —— 逻辑放在这里才测得到，
/// `tauri::State` 在单测里造不出来。
pub fn list_assets(config_dir: &Path, scope: Option<Scope>) -> Vec<AssetView> {
    let scopes = match scope {
        Some(s) => vec![s],
        None => all_scopes(config_dir),
    };
    let mut out = Vec::new();
    for sc in scopes {
        for a in load_store(config_dir, &sc).assets {
            let n = scan_references(config_dir, &a.id).len();
            out.push(to_view(config_dir, &sc, a, n));
        }
    }
    out.sort_by_key(|v| std::cmp::Reverse(v.asset.updated_at));
    out
}

/// 一份待入库的媒体：本地路径，或前端刚上传的 data URL。
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MediaInput {
    /// 磁盘上已有的文件（创作台产物、用户从 Finder 选的图）。
    Path { path: String },
    /// 前端 FileReader 读出来的 data URL。**只在这里出现**，
    /// 落盘之后 JSON 里存的是路径。
    DataUrl { data_url: String },
}

impl MediaInput {
    pub(crate) fn read(&self) -> Result<(Vec<u8>, String)> {
        match self {
            MediaInput::Path { path } => {
                let p = Path::new(path);
                let ext = p
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("png")
                    .to_string();
                let bytes = std::fs::read(p)
                    .map_err(|e| Error::runtime(format!("读不到 {path}：{e}")))?;
                Ok((bytes, ext))
            }
            MediaInput::DataUrl { data_url } => {
                use base64::Engine;
                let comma = data_url
                    .find(',')
                    .ok_or_else(|| Error::runtime("data URL 格式不对"))?;
                let ext = data_url[5..comma]
                    .split(['/', ';'])
                    .nth(1)
                    .unwrap_or("png")
                    .to_string();
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(&data_url[comma + 1..])
                    .map_err(|e| Error::runtime(format!("base64 解码失败：{e}")))?;
                Ok((bytes, ext))
            }
        }
    }

    fn is_video(&self) -> bool {
        let probe = match self {
            MediaInput::Path { path } => path.to_ascii_lowercase(),
            MediaInput::DataUrl { data_url } => data_url[..data_url.len().min(30)].to_lowercase(),
        };
        probe.contains("video/")
            || probe.ends_with(".mp4")
            || probe.ends_with(".mov")
            || probe.ends_with(".webm")
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateAssetReq {
    pub scope: Scope,
    pub kind: AssetKind,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub media: Option<MediaInput>,
}

/// 新建资产。**图可以留空** —— 先有身份，再补图。
#[tauri::command]
pub fn assets_create(
    state: tauri::State<'_, crate::AppState>,
    req: CreateAssetReq,
) -> Result<AssetView> {
    let cfg = &state.config_dir;
    let name = req.name.trim();
    if name.is_empty() {
        return Err(Error::runtime("资产得有个名字"));
    }
    let mut asset = Asset::new(req.kind, name);
    asset.description = req.description;
    if let Some(media) = req.media {
        let (bytes, ext) = media.read()?;
        add_variant(
            cfg,
            &req.scope,
            &mut asset,
            &bytes,
            &ext,
            media.is_video(),
            None,
            true,
            None,
        )?;
    }
    let mut store = load_store(cfg, &req.scope);
    store.assets.push(asset.clone());
    save_store(cfg, &req.scope, &store)?;
    Ok(to_view(cfg, &req.scope, asset, 0))
}

#[derive(Debug, Deserialize)]
pub struct UpdateAssetReq {
    pub scope: Scope,
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub starred: Option<bool>,
    #[serde(default)]
    pub locked: Option<bool>,
    #[serde(default)]
    pub character: Option<CharacterTraits>,
    #[serde(default)]
    pub scene: Option<crate::scene_contract::SceneContract>,
    /// 出图配置。**`Some(空 AssetGen)` = 清空覆盖，回到项目设置**；
    /// `None` = 这次不动它。两者不是一回事。
    #[serde(default)]
    pub gen: Option<AssetGen>,
    /// 人物本身（所有形象共用）
    #[serde(default)]
    pub identity: Option<String>,
}

#[tauri::command]
pub fn assets_update(
    state: tauri::State<'_, crate::AppState>,
    req: UpdateAssetReq,
) -> Result<AssetView> {
    let cfg = &state.config_dir;
    let mut store = load_store(cfg, &req.scope);
    let a = store
        .assets
        .iter_mut()
        .find(|a| a.id == req.id)
        .ok_or_else(|| Error::runtime("找不到这个资产"))?;
    if let Some(v) = req.name {
        a.name = v;
    }
    if let Some(v) = req.description {
        a.description = v;
    }
    if let Some(v) = req.identity {
        a.identity = v;
    }
    if let Some(v) = req.starred {
        a.starred = v;
    }
    if let Some(v) = req.locked {
        a.locked = v;
    }
    if let Some(v) = req.gen {
        // **全空的 AssetGen 存成 `None`** —— 存一个所有字段都是 None 的
        // 结构进去，读的时候「设过但没设任何东西」和「没设过」就分不开了。
        a.gen = if v == AssetGen::default() { None } else { Some(v) };
    }
    if let Some(v) = req.character {
        a.character = Some(v);
    }
    if let Some(mut v) = req.scene {
        // 3D 片场只由生成命令写。界面整份保存契约时多半不带它（老页面、自己拼的对象），不能冲掉
        if v.world.is_none() {
            v.world = a.scene.as_ref().and_then(|s| s.world.clone());
        }
        // 空契约存成 `None`。留一个空壳子会让界面显示「已有空间契约」，
        // 而它一条地理都锁不住 —— 比没有更糟。
        // **但挂着校准 / 3D 片场的不算空**：以前没有锚点的场景存眼高，校准直接被当空契约丢掉。
        a.scene = if v.is_empty() && !v.has_world_data() { None } else { Some(v) };
    }
    a.updated_at = now_ms();
    let updated = a.clone();
    save_store(cfg, &req.scope, &store)?;
    let n = scan_references(cfg, &updated.id).len();
    Ok(to_view(cfg, &req.scope, updated, n))
}

/// 谁在用这个资产。
#[tauri::command]
pub fn assets_references(
    state: tauri::State<'_, crate::AppState>,
    asset_id: String,
) -> Vec<AssetReference> {
    scan_references(&state.config_dir, &asset_id)
}

/// 删资产。**默认先查引用。**
///
/// 不查的话，删掉的角色留下一堆悬空引用，分镜静默失去参考图 ——
/// 表现是「生成的人突然不像了」，根因要挖很久。
/// `force` 是用户看过引用列表之后的明确选择。
#[tauri::command]
pub fn assets_delete(
    state: tauri::State<'_, crate::AppState>,
    scope: Scope,
    id: String,
    force: bool,
) -> Result<Vec<AssetReference>> {
    let cfg = &state.config_dir;
    let refs = scan_references(cfg, &id);
    if !refs.is_empty() && !force {
        let where_ = refs
            .iter()
            .take(3)
            .map(|r| format!("{}·{}·镜{}", r.project_title, r.episode_title, r.shot_num))
            .collect::<Vec<_>>()
            .join("，");
        let more = if refs.len() > 3 {
            format!("等 {} 处", refs.len())
        } else {
            String::new()
        };
        return Err(Error::runtime(format!(
            "还有镜头在用它：{where_}{more}。确认要删就再点一次「强制删除」。"
        )));
    }
    let mut store = load_store(cfg, &scope);
    let before = store.assets.len();
    // 合并来的形象可能引用别的文件夹里的图（同一作用域合并时不复制文件），整目录删不到，逐个删
    let stray: Vec<String> = store.assets.iter().filter(|a| a.id == id)
        .flat_map(|a| a.all_units().flat_map(|u| u.image_variants.iter().chain(u.video_variants.iter())).map(|v| v.rel_path.clone()).collect::<Vec<_>>())
        .filter(|rel| !rel.starts_with(&format!("{id}/")))
        .collect();
    store.assets.retain(|a| a.id != id);
    if store.assets.len() == before {
        return Err(Error::runtime("找不到这个资产"));
    }
    save_store(cfg, &scope, &store)?;
    // 媒体整目录删掉 —— 按资产 id 分目录就是为了这一步不用扫全盘。
    let _ = std::fs::remove_dir_all(scope.media_dir(cfg).join(&id));
    for rel in stray {
        let _ = std::fs::remove_file(scope.media_dir(cfg).join(rel));
    }
    Ok(refs)
}

#[derive(Debug, Deserialize)]
pub struct AddVariantReq {
    pub scope: Scope,
    pub id: String,
    pub media: MediaInput,
    #[serde(default)]
    pub prompt_used: Option<String>,
    /// 这张图在流水线里是什么。见 [`VariantRole`]。空 = 通用参考图。
    #[serde(default)]
    pub role: Option<VariantRole>,
    /// 加到哪个形象。空 / `main` = 主形象
    #[serde(default)]
    pub look_id: Option<String>,
}

#[tauri::command]
pub fn assets_add_variant(
    state: tauri::State<'_, crate::AppState>,
    req: AddVariantReq,
) -> Result<AssetView> {
    let cfg = &state.config_dir;
    let mut store = load_store(cfg, &req.scope);
    let a = store
        .assets
        .iter_mut()
        .find(|a| a.id == req.id)
        .ok_or_else(|| Error::runtime("找不到这个资产"))?;
    // **错配的角色要拦住。**
    //
    // 把「场景反打」挂到角色身上不会报错，只是以后按角色取的时候取不到 ——
    // 而那时候没人会想到是这一步标错了。宁可在提交的当下拒绝。
    if let Some(role) = req.role {
        if !role.allows(a.kind) {
            return Err(Error::runtime(format!(
                "「{}」只能标在{}上，这个资产是{}",
                role.label(),
                match role {
                    VariantRole::Portrait | VariantRole::Turnaround => "角色",
                    _ => "场景",
                },
                a.kind.tag_label()
            )));
        }
    }
    let (bytes, ext) = req.media.read()?;
    let is_video = req.media.is_video();
    add_variant_to_look(
        cfg,
        &req.scope,
        a,
        req.look_id.as_deref(),
        &bytes,
        &ext,
        is_video,
        req.prompt_used,
        matches!(req.media, MediaInput::DataUrl { .. }),
        req.role,
    )?;
    let updated = a.clone();
    save_store(cfg, &req.scope, &store)?;
    let n = scan_references(cfg, &updated.id).len();
    Ok(to_view(cfg, &req.scope, updated, n))
}

#[tauri::command]
pub fn assets_select_variant(
    state: tauri::State<'_, crate::AppState>,
    scope: Scope,
    id: String,
    variant_id: String,
    is_video: bool,
) -> Result<AssetView> {
    let cfg = &state.config_dir;
    let mut store = load_store(cfg, &scope);
    let a = store
        .assets
        .iter_mut()
        .find(|a| a.id == id)
        .ok_or_else(|| Error::runtime("找不到这个资产"))?;
    // 候选图 id 全局唯一：在哪个形象的图片池里就选哪个池子的，不用前端传形象
    let unit = a
        .all_units_mut()
        .find(|u| if is_video { u.video_variants.iter().any(|v| v.id == variant_id) } else { u.image_variants.iter().any(|v| v.id == variant_id) })
        .ok_or_else(|| Error::runtime("这张候选图不在了"))?;
    if is_video {
        unit.selected_video_id = Some(variant_id);
    } else {
        unit.selected_image_id = Some(variant_id);
    }
    a.updated_at = now_ms();
    let updated = a.clone();
    save_store(cfg, &scope, &store)?;
    let n = scan_references(cfg, &updated.id).len();
    Ok(to_view(cfg, &scope, updated, n))
}

#[tauri::command]
pub fn assets_favorite_variant(
    state: tauri::State<'_, crate::AppState>,
    scope: Scope,
    id: String,
    variant_id: String,
    on: bool,
) -> Result<AssetView> {
    let cfg = &state.config_dir;
    let mut store = load_store(cfg, &scope);
    let a = store
        .assets
        .iter_mut()
        .find(|a| a.id == id)
        .ok_or_else(|| Error::runtime("找不到这个资产"))?;
    for unit in a.all_units_mut() {
        for v in unit.image_variants.iter_mut().chain(unit.video_variants.iter_mut()) {
            if v.id == variant_id {
                v.favorited = on;
            }
        }
    }
    a.updated_at = now_ms();
    let updated = a.clone();
    save_store(cfg, &scope, &store)?;
    let n = scan_references(cfg, &updated.id).len();
    Ok(to_view(cfg, &scope, updated, n))
}

/// 深拷贝到另一个作用域，媒体文件一起拷。
///
/// 两个方向共用这一个实现：`promote`（本集 → 全局库）
/// 和 `fork`（全局库 → 本集）。**媒体必须真的拷贝** ——
/// 只改 JSON 的话两份资产共享同一批文件，删任何一份都会把另一份掏空。
fn copy_asset_between(config_dir: &Path, from: &Scope, to: &Scope, id: &str) -> Result<Asset> {
    let src_store = load_store(config_dir, from);
    let src = src_store
        .assets
        .iter()
        .find(|a| a.id == id)
        .ok_or_else(|| Error::runtime("源作用域里找不到这个资产"))?;

    let mut copy = src.clone();
    copy.id = format!("{}_{}", copy.kind.id_prefix(), short_uuid());
    copy.created_at = now_ms();
    copy.updated_at = now_ms();

    let from_media = from.media_dir(config_dir);
    let to_media = to.media_dir(config_dir);
    let copy_id = copy.id.clone();
    for v in copy
        .all_units_mut()
        .flat_map(|u| u.image_variants.iter_mut().chain(u.video_variants.iter_mut()))
    {
        let old_abs = from_media.join(&v.rel_path);
        let file_name = Path::new(&v.rel_path)
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| format!("{}.png", v.id));
        let new_rel = format!("{}/{}", copy_id, file_name);
        let new_abs = to_media.join(&new_rel);
        if let Some(parent) = new_abs.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::copy(&old_abs, &new_abs)
            .map_err(|e| Error::runtime(format!("拷贝媒体失败（{}）：{e}", v.rel_path)))?;
        v.rel_path = new_rel;
    }

    let mut dst_store = load_store(config_dir, to);
    dst_store.assets.push(copy.clone());
    save_store(config_dir, to, &dst_store)?;
    Ok(copy)
}

/// [`assets_remove_variant`] 的实现。**分开是为了测得到** ——
/// 包着 `State` 的那层起不了一个 tauri app 就跑不了。
///
/// 返回被删那张的相对路径（调用方负责删文件：**先落盘再删文件**，
/// 反过来的话写盘失败时文件已经没了而记录还在，界面上是一张碎图）。
pub(crate) fn remove_variant_in_place(
    _config_dir: &Path,
    _scope: &Scope,
    a: &mut Asset,
    variant_id: &str,
) -> Result<String> {
    for unit in a.all_units_mut() {
        for is_video in [false, true] {
            let list = if is_video { &mut unit.video_variants } else { &mut unit.image_variants };
            if let Some(i) = list.iter().position(|v| v.id == variant_id) {
                let rel = list.remove(i).rel_path;
                let next = list.last().map(|v| v.id.clone());
                // **封面被删了就改选最新的那张。**
                let selected = if is_video { &mut unit.selected_video_id } else { &mut unit.selected_image_id };
                if selected.as_deref() == Some(variant_id) {
                    *selected = next;
                }
                return Ok(rel);
            }
        }
    }
    Err(Error::runtime("这张候选图不在了 —— 可能已经在另一个窗口里删掉了"))
}

/// 删掉一张候选图（或候选片）。
///
/// # 为什么要有手动删
///
/// 自动裁剪（[`prune_variants`]）只在超过 12 张时从最旧的开始删。
/// 于是「出了十张废图」这种常态下，一张都去不掉 —— 用户 2026-09-12
/// 的原话是「一堆废图还去不掉」。
///
/// # 删到封面头上时必须改选一张
///
/// 封面就是 `@` 引用时真正发出去的那一张。删了不改选的话，
/// `selected_image_id` 指向一个不存在的变体 —— 界面上封面位空了，
/// 而分镜里那个 `[场景:金銮殿]` 还在引用它，出图时**只剩名字**，
/// 长相一点都带不上，**而且不报错**。
///
/// 改选的规则是「剩下的最后一张」= 最新的那张。
#[tauri::command]
pub fn assets_remove_variant(
    state: tauri::State<'_, crate::AppState>,
    scope: Scope,
    id: String,
    variant_id: String,
) -> Result<AssetView> {
    let cfg = &state.config_dir;
    let mut store = load_store(cfg, &scope);
    let a = store
        .assets
        .iter_mut()
        .find(|a| a.id == id)
        .ok_or_else(|| Error::runtime("找不到这个资产"))?;

    let media_dir = scope.media_dir(cfg);
    let rel = remove_variant_in_place(cfg, &scope, a, &variant_id)?;

    a.updated_at = now_ms();
    let updated = a.clone();
    save_store(cfg, &scope, &store)?;
    // **先落盘再删文件。** 反过来的话，写盘失败时文件已经没了，
    // 而记录还在 —— 界面上是一张碎图，比两者都在更糟。
    let _ = std::fs::remove_file(media_dir.join(rel));
    let n = scan_references(cfg, &updated.id).len();
    Ok(to_view(cfg, &scope, updated, n))
}

// ---------------------------------------------------------------------------
// 形象
// ---------------------------------------------------------------------------

/// 加一个形象。名字不能空；新形象没有图，候选图用「加变体」带 `look_id` 加。
pub fn add_look(a: &mut Asset, name: &str, description: &str) -> Result<Look> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::runtime("给形象起个名字，比如「盛唐男装」"));
    }
    // **道具也有形象**（2026-09-28）：协议铭牌量产款是金属牌，星澜那块是零号机专属款的青色晶石 ——
    // 同一个东西、剧本里同一个名字，长得不一样。拆成两个道具的话剧本里的「铭牌」对不上号。
    // 场景不开：场景换时段、换天气走空间契约和场景图，不是形象。
    if !matches!(a.kind, AssetKind::Character | AssetKind::Prop) {
        return Err(Error::runtime("只有角色和道具有形象"));
    }
    let now = now_ms();
    let look = Look {
        id: format!("look_{}", short_uuid()),
        name: name.to_string(),
        description: description.trim().to_string(),
        nsfw: false,
        unit: AssetUnit::default(),
        created_at: now,
        updated_at: now,
    };
    a.looks.push(look.clone());
    a.updated_at = now;
    Ok(look)
}

#[derive(Debug, Deserialize)]
pub struct UpdateLookReq {
    pub scope: Scope,
    pub id: String,
    /// `main` = 主形象
    pub look_id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub nsfw: Option<bool>,
}

/// 改形象的名字、描述、NSFW。主形象的描述就是资产的 description
pub fn update_look(a: &mut Asset, req: &UpdateLookReq) -> Result<()> {
    if req.look_id == MAIN_LOOK {
        if let Some(v) = &req.name {
            a.main_look_name = v.trim().to_string();
        }
        if let Some(v) = &req.description {
            a.description = v.clone();
        }
        if let Some(v) = req.nsfw {
            a.nsfw = v;
        }
    } else {
        let l = a.looks.iter_mut().find(|l| l.id == req.look_id).ok_or_else(|| Error::runtime("这个形象不在了"))?;
        if let Some(v) = &req.name {
            if v.trim().is_empty() {
                return Err(Error::runtime("形象名字不能空"));
            }
            l.name = v.trim().to_string();
        }
        if let Some(v) = &req.description {
            l.description = v.clone();
        }
        if let Some(v) = req.nsfw {
            l.nsfw = v;
        }
        l.updated_at = now_ms();
    }
    a.updated_at = now_ms();
    Ok(())
}

/// 删形象，回它的候选图相对路径（调用方落盘后删文件）。主形象不能删
pub fn remove_look(a: &mut Asset, look_id: &str) -> Result<Vec<String>> {
    if look_id == MAIN_LOOK {
        return Err(Error::runtime("主形象不能删"));
    }
    let i = a.looks.iter().position(|l| l.id == look_id).ok_or_else(|| Error::runtime("这个形象不在了"))?;
    let look = a.looks.remove(i);
    a.updated_at = now_ms();
    Ok(look.unit.image_variants.iter().chain(look.unit.video_variants.iter()).map(|v| v.rel_path.clone()).collect())
}

fn with_asset<T>(cfg: &Path, scope: &Scope, id: &str, f: impl FnOnce(&mut Asset) -> Result<T>) -> Result<(T, AssetView)> {
    let mut store = load_store(cfg, scope);
    let a = store.assets.iter_mut().find(|a| a.id == id).ok_or_else(|| Error::runtime("找不到这个资产"))?;
    let out = f(a)?;
    let updated = a.clone();
    save_store(cfg, scope, &store)?;
    let n = scan_references(cfg, &updated.id).len();
    Ok((out, to_view(cfg, scope, updated, n)))
}

#[tauri::command]
pub fn assets_add_look(state: tauri::State<'_, crate::AppState>, scope: Scope, id: String, name: String, description: String) -> Result<AssetView> {
    with_asset(&state.config_dir, &scope, &id, |a| add_look(a, &name, &description)).map(|(_, v)| v)
}

#[tauri::command]
pub fn assets_update_look(state: tauri::State<'_, crate::AppState>, req: UpdateLookReq) -> Result<AssetView> {
    let (scope, id) = (req.scope.clone(), req.id.clone());
    with_asset(&state.config_dir, &scope, &id, |a| update_look(a, &req)).map(|(_, v)| v)
}

/// 删形象。**先落盘再删文件**，理由同 [`assets_remove_variant`]。
/// 项目默认形象、单镜覆盖指着它的，出片时落回主形象（见 `looks::effective_look`）。
#[tauri::command]
pub fn assets_remove_look(state: tauri::State<'_, crate::AppState>, scope: Scope, id: String, look_id: String) -> Result<AssetView> {
    let cfg = &state.config_dir;
    let (rels, view) = with_asset(cfg, &scope, &id, |a| remove_look(a, &look_id))?;
    let media = scope.media_dir(cfg);
    for rel in rels {
        let _ = std::fs::remove_file(media.join(rel));
    }
    Ok(view)
}

#[tauri::command]
pub fn assets_promote(
    state: tauri::State<'_, crate::AppState>,
    from: Scope,
    id: String,
) -> Result<AssetView> {
    let cfg = &state.config_dir;
    let copy = copy_asset_between(cfg, &from, &Scope::Global, &id)?;
    Ok(to_view(cfg, &Scope::Global, copy, 0))
}

/// 老数据体积阈值。拆完 base64 的 `characters.json` 只有十几 KB，
/// 带图的是几十 MB —— 用体积当判据，比每次启动都去读整个文件便宜得多。
const LEGACY_SIZE_GATE: u64 = 1024 * 1024;

/// 启动时自动迁移还带着 base64 的老项目。
///
/// **只对超过 [`LEGACY_SIZE_GATE`] 的文件动手**，所以已经迁过的项目
/// 连读都不会读。迁移本身是幂等的，这道闸只是为了不在每次启动时
/// 白解析一遍 JSON。
pub fn migrate_legacy_projects_on_start(config_dir: &Path) -> Vec<MigrationReport> {
    let Ok(entries) = std::fs::read_dir(config_dir.join("projects")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in entries.flatten() {
        let file = e.path().join("characters.json");
        let Ok(meta) = std::fs::metadata(&file) else {
            continue;
        };
        if meta.len() < LEGACY_SIZE_GATE {
            continue;
        }
        let id = e.file_name().to_string_lossy().to_string();
        match migrate_project_characters(config_dir, &id) {
            Ok(r) => {
                tracing::info!(
                    project = %id, images = r.images_written,
                    before = r.bytes_before, after = r.bytes_after,
                    "把老角色库里的 base64 拆成了文件"
                );
                for f in &r.failures {
                    tracing::warn!(project = %id, "{f}");
                }
                out.push(r);
            }
            Err(err) => tracing::warn!(project = %id, %err, "角色库迁移失败"),
        }
    }
    out
}

/// 一条 `@` 引用的解析结果。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ResolvedRef {
    pub kind: AssetKind,
    pub name: String,
    /// `None` = 提示词里写了这个名字，但库里没有。
    ///
    /// **必须让它在界面上可见。** 悄悄当普通文字处理的话，
    /// 用户以为挂了参考图，实际什么都没挂 —— 又一次静默失败。
    pub asset_id: Option<String>,
    pub description: String,
    /// 身份锚点编译出来的那一段（辨识标记 + Hex 色值 + 五官）。
    ///
    /// **和 `description` 分开存**，因为两者的去留规则不一样：
    /// 描述在参考图发出去时要删掉（图已经说清楚长相了，再写一遍会打架），
    /// 而锚点是**给纯文生图那条路兜底**的 —— 那条路一张图都不传，
    /// 长相全靠文字，这时候「左眼下方两厘米的痣」比六十字的形容词有用得多。
    pub anchors: String,
    /// 这个角色专属的负向词。**只对认负向的引擎有意义** ——
    /// Z-Image 和 Krea 2 跑在 cfg=1 上，负向分支被抵消掉，传了也不生效。
    pub avoid: String,
    /// **一致性锁面 —— 发了参考图也要发。**
    ///
    /// 和 `anchors` 分工：`anchors` 是长描述，发了图就不发（会和图打架）；
    /// 锁面是极短的判别性事实（Hex 色值、辨识标记），**参考图补不上这一层** ——
    /// 一张角色板只能约束它自己画到的部分。同一件衣服第一镜蓝第二镜红，
    /// 通常不是资产写错了，是没有任何一条提示词把颜色带进去。
    pub lock: String,
    /// 当前选中变体的本机绝对路径，喂给引擎。
    pub image_path: Option<String>,
    /// 显示用。
    pub image_url: Option<String>,
}

/// 把提示词里的 `[角色:老茶师]` 对到库里的资产上。
///
/// 同名同类型取**第一个**命中的（作用域顺序：全局 → 剧集 → 分集）。
pub fn resolve_refs(config_dir: &Path, prompt: &str) -> Vec<ResolvedRef> {
    resolve_refs_in(config_dir, prompt, &RefScope::Anywhere)
}

/// 按名字找资产时在哪儿找。
///
/// # 为什么要分
///
/// 以前一律在**所有项目**里按名字找、取第一个（目录顺序）。两个项目都有「皇帝」时，
/// 这一集的镜头可能挂上另一个项目的皇帝的图，而且不报错（2026-09-15 查出来的）。
/// Chat 接 Studio 之后模型天然用名字说话，这个口子必须先堵上。
#[derive(Debug, Clone, Copy)]
pub enum RefScope<'a> {
    /// 知道是哪个项目：本集 → 项目 → 全局库，**从近到远**，近的赢。
    /// 不知道哪一集时查这个项目的所有分集（按分集目录名排）
    Project { project_id: &'a str, episode_id: Option<&'a str> },
    /// 不属于任何项目（创作台、画布）：全部作用域里找，**只有一个匹配才算**；
    /// 多个匹配时全局库里有就用全局库的，否则当没找到（不猜是哪个项目的）
    Anywhere,
}

/// 从近到远要查的作用域
pub fn lookup_scopes(config_dir: &Path, scope: &RefScope) -> Vec<Scope> {
    match scope {
        RefScope::Anywhere => all_scopes(config_dir),
        RefScope::Project { project_id, episode_id } => {
            let mut out = Vec::new();
            match episode_id {
                Some(ep) => out.push(Scope::Episode { project_id: project_id.to_string(), episode_id: ep.to_string() }),
                None => {
                    let mut eps: Vec<String> = std::fs::read_dir(config_dir.join("projects").join(project_id).join("episodes"))
                        .map(|r| r.flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().to_string()).collect())
                        .unwrap_or_default();
                    eps.sort();
                    out.extend(eps.into_iter().map(|ep| Scope::Episode { project_id: project_id.to_string(), episode_id: ep }));
                }
            }
            out.push(Scope::Project { project_id: project_id.to_string() });
            out.push(Scope::Global);
            out
        }
    }
}

/// 按种类和名字找一个资产。见 [`RefScope`]
pub fn find_asset_by_name(config_dir: &Path, kind: AssetKind, name: &str, scope: &RefScope) -> Option<(Scope, Asset)> {
    let name = name.trim();
    let mut hits = Vec::new();
    for sc in lookup_scopes(config_dir, scope) {
        for a in load_store(config_dir, &sc).assets {
            if a.kind == kind && a.name.trim() == name {
                if matches!(scope, RefScope::Project { .. }) {
                    return Some((sc, a));
                }
                hits.push((sc.clone(), a));
            }
        }
    }
    match hits.len() {
        0 => None,
        1 => hits.pop(),
        _ => hits.into_iter().find(|(sc, _)| *sc == Scope::Global),
    }
}

pub fn resolve_refs_in(config_dir: &Path, prompt: &str, scope: &RefScope) -> Vec<ResolvedRef> {
    let refs = parse_asset_refs(prompt);
    if refs.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for r in refs {
        let mut hit = ResolvedRef {
            kind: r.kind,
            name: r.name.clone(),
            asset_id: None,
            description: String::new(),
            anchors: String::new(),
            avoid: String::new(),
            lock: String::new(),
            image_path: None,
            image_url: None,
        };
        if let Some((scope_hit, a)) = find_asset_by_name(config_dir, r.kind, &r.name, scope) {
            let sc = &scope_hit;
            {
                hit.asset_id = Some(a.id.clone());
                hit.description = a.description.clone();
                if let Some(dna) = a.character.as_ref().and_then(|c| c.visual_dna.as_ref()) {
                    hit.anchors = dna.to_prompt_block(&a.name);
                    hit.avoid = dna.avoid_prompt();
                    hit.lock = dna.lock_line();
                }
                // **`visual_dna` 给不出东西时退到 `spec`。**
                //
                // 机制一直都在，数据不在：实测用户项目里四个角色的
                // `visual_dna` 每个描述字段都是 null，只剩一组
                // **四个角色完全相同**的默认色值（iris #3D2314、hair
                // #1A1A1A…）。那不是这个角色的特征，是模板 ——
                // 拿它当身份锚点，等于告诉模型「这四个人长一样」。
                //
                // 而真正填好的东西在 `spec`（槽位化的长相和服装，
                // 用户在界面上一格一格填的），这条路一直没读它。
                // 典型的「看着有实际没有」：功能在、测试绿、发出去的是模板。
                //
                // 判据是**有没有描述性内容**，不是 `visual_dna` 存不存在。
                if let Some(spec) = a
                    .character
                    .as_ref()
                    .and_then(|c| c.spec.as_ref())
                    .filter(|s| !s.is_empty())
                {
                    if !spec_has_real_anchor(&hit.anchors) {
                        hit.anchors = spec.description_block(&[]);
                    }
                    // **锁面同样要按「有没有描述性内容」判，不是空不空。**
                    //
                    // 实测：`林婉清（盛唐）` 和 `林婉清（现代）` 的锁面
                    // 完全相同（都是那组默认色值）—— 而锁面正是**发了参考图
                    // 也会发**的那一段，它的活就是分开这两个造型分身。
                    // 一组四个角色共用的色值分不开任何人。
                    //
                    // 换成规格那一行（「盛唐女官，28岁，体型偏瘦，黑发，
                    // 男装官服」），短，而且真的能分开。色值不另外保留：
                    // 它对身份没有贡献，而锁面每一镜都发，字数很贵。
                    if !spec_has_real_anchor(&hit.lock) {
                        hit.lock = spec.anchor_line(crate::character_spec::LOCK_CHARS);
                    }
                }
                // 场景走同一套分工：文字地图当 `anchors`（发了参考图就不发），
                // 地理锁当 `lock`（**发了参考图也要发**）。
                //
                // 后者是这条路的要害：一张场景板只画了**一个方向**，
                // 换个方向的镜头里「柜台在哪、有几扇门」图上根本没有。
                if let Some(sc) = a.scene.as_ref().filter(|s| !s.is_empty()) {
                    hit.anchors = sc.to_prompt_block(&a.name);
                    hit.lock = sc.lock_line();
                }
                if let Some(v) = a.unit.selected_image() {
                    let abs = variant_abs_path(config_dir, sc, v);
                    hit.image_url = Some(media_url(&abs));
                    hit.image_path = Some(abs.to_string_lossy().to_string());
                }
            }
        }
        out.push(hit);
    }
    out
}

/// 按名字找场景资产的空间契约。
///
/// 存在的理由：宫格的视角表以前是调用方每次手写的。手写有两个毛病 ——
/// 换个人写就散了，而且没有任何东西保证几个视角说的是同一份地理。
/// 有契约时就从契约派生，见
/// [`crate::scene_contract::SceneContract::viewpoints`]。
///
/// **按名字查，不按 `[场景:x]` 标签查。** 标签在送进出图工具之前就已经被
/// [`expand_ref_tags`] 换成「名字（描述）」了（`studio.rs` 里那一步），
/// 工具这一层根本看不到标签；而对话里模型直接调工具时本来也不会写标签。
///
/// 找不到返回 `None`，调用方回落到自己的默认 —— 这条路**不能挡住出图**。
pub fn scene_contract_by_name(
    config_dir: &Path,
    name: &str,
) -> Option<crate::scene_contract::SceneContract> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    // 不知道是哪个项目：只有一个同名场景才用（见 `RefScope::Anywhere`）
    if let Some((_, a)) = find_asset_by_name(config_dir, AssetKind::Scene, name, &RefScope::Anywhere) {
        // 空契约当没有 —— 它一条地理都锁不住，
        // 却会把调用方的默认视角表顶掉，出一张更差的图。
        return a.scene.filter(|c| !c.is_empty());
    }
    None
}

/// 送进引擎之前把标签换成「名字（描述）」。
///
/// 描述是资产库里写的那句「白须，青灰长衫」—— 它进提示词，
/// 生成出来的人才会真的像。没对上库的引用退化成 [`strip_ref_tags`]
/// 的行为（只留名字），**不能原样留着方括号**：
/// 模型会把括号和「角色」两个字当画面内容画进去。
/// 展开引用时带不带那一长串描述。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefDetail<'a> {
    /// 带上描述。**没有参考图的路子只能靠文字** —— 首帧走的是纯文生图，
    /// 一张参考图都不传，描述是这时候唯一说得清「谁长什么样」的东西。
    Full,
    /// **图确实发出去了的资产只留名字。**
    ///
    /// 参考图已经把长相说清楚了，再写一遍六十字的外貌描述有两个害处：
    /// 图和字互相打架（实测过一镜里 5 个资产的描述占了 699 字里的 300 字，
    /// 而用户自己写的画面只有 200 字，改画面等于只改了三成），
    /// 以及把提示词挤爆。
    ///
    /// # 判据是「图发出去了」，不是「它有图」
    ///
    /// 参考图有 4 张的上限（H3 的硬约束）。一镜里第 5 个资产**有图但送不出去** ——
    /// 这时候要是因为「它有图」就把描述也删掉，那一个资产**图和字两头都没了**，
    /// 而且不报错。所以这里带的是**实际送出去的那几张图的路径**。
    ///
    /// 没有参考图、或者图没排进这几张的资产，在这个模式下照样带描述。
    NameWhenSent(&'a [String]),
    /// **一律只留名字。**
    ///
    /// 给动作段用：长相在画面段已经说过一遍了，动作里再来一遍六十字外貌
    /// 描述会把真正的动作挤到角落。这里不看有没有图 —— 动作段本来就不该
    /// 承担「说清楚谁长什么样」这件事。
    NameOnly,
}

/// 送进引擎之前把标签换成「名字（描述）」。见 [`expand_ref_tags_detail`]。
pub fn expand_ref_tags(prompt: &str, resolved: &[ResolvedRef]) -> String {
    expand_ref_tags_detail(prompt, resolved, RefDetail::Full)
}

/// 这段锚点里有没有**描述性**内容，还是只剩一组模板色值。
///
/// `CharacterVisualDna` 在每个字段都是 null 时仍然会渲染出
/// `iris #3D2314, hair #1A1A1A, skin #E8C4A0, lips #C4727E` ——
/// 那是默认模板，实测四个不同角色一模一样。非空，但一点身份信息都没有。
///
/// **判据是「除了 Hex 色值还有没有别的」**，不是「字符串空不空」。
fn spec_has_real_anchor(anchors: &str) -> bool {
    let stripped: String = anchors
        .split(|c| c == ',' || c == '；' || c == ';')
        .map(str::trim)
        .filter(|p| !p.is_empty() && !p.contains('#'))
        .collect::<Vec<_>>()
        .join("");
    !stripped.is_empty()
}

pub fn expand_ref_tags_detail(
    prompt: &str,
    resolved: &[ResolvedRef],
    detail: RefDetail<'_>,
) -> String {
    let refs = parse_asset_refs(prompt);
    // **挨着的两个标签之间补一个逗号。**
    //
    // 用户是连着打的：`[场景:窗台][角色:小花]`。展开之后成了
    // `窗台（…）小花（…）` —— 两段描述之间一个标点都没有，模型读到的是
    // 一个连着的长词组。只在**两个都确实是标签**的时候补，
    // 所以不会动到正文里碰巧出现的 `][`。
    let mut out = prompt.to_string();
    for pair in refs.windows(2) {
        let joined = format!("{}{}", pair[0].raw, pair[1].raw);
        if out.contains(&joined) {
            out = out.replace(&joined, &format!("{}，{}", pair[0].raw, pair[1].raw));
        }
    }
    for r in &refs {
        let hit = resolved
            .iter()
            .find(|x| x.kind == r.kind && x.name == r.name);
        let sent = |h: &ResolvedRef| match (detail, &h.image_path) {
            (RefDetail::NameOnly, _) => true,
            (RefDetail::NameWhenSent(paths), Some(p)) => paths.iter().any(|x| x == p),
            _ => false,
        };
        let desc = match hit {
            Some(h) if sent(h) => "",
            Some(h) => h.description.trim(),
            None => "",
        };
        let desc = clean_description(desc);

        // **身份锚点只在没发参考图时补。**
        //
        // 发了图那条路，长相已经由像素说清楚了 —— 再写一段「左眼下方两厘米
        // 一颗痣 / 虹膜 #3D2314」只会和图打架，还挤占用户自己写的画面。
        // 没发图那条（纯文生图的首帧）正相反：文字是唯一的载体，
        // 而判别性强的锚点正是在这时候救场的东西。
        let anchors = match hit {
            Some(h) if !sent(h) => h.anchors.trim(),
            _ => "",
        };

        // **锁面发了图也要发。**
        //
        // 参考图只能约束它自己画到的部分 —— 被手挡住的道具、每镜重新构图的
        // 服装细节，都落在身份参考「默认控不住」的一侧。它足够短
        // （Hex 色值 + 辨识标记），不会挤爆提示词。
        //
        // 动作段（`NameOnly`）例外：那一段只负责说谁在动，长相的事在画面段说过了。
        let lock = match (hit, detail) {
            (_, RefDetail::NameOnly) => "",
            (Some(h), _) => h.lock.trim(),
            _ => "",
        };

        let mut bits: Vec<&str> = Vec::new();
        if !desc.is_empty() {
            bits.push(desc.as_str());
        }
        if !anchors.is_empty() {
            bits.push(anchors);
        }
        // 锚点里已经含了锁面时不要重复 —— 没发图那条路两者都在
        if !lock.is_empty() && !anchors.contains(lock) {
            bits.push(lock);
        }
        let inside = bits.join("；");
        let replacement = if inside.is_empty() {
            r.name.clone()
        } else {
            format!("{}（{inside}）", r.name)
        };
        out = out.replace(&r.raw, &replacement);
    }
    out
}

/// 描述里那些**「不适用」的占位词**，进提示词就是纯噪声。
///
/// 真实数据：`小花（三花猫）` 的描述是
/// 「…两只尖耳朵，约3个月，雌，**无**」—— 最后那个「无」是
/// 抽取时把 `clothing` 字段拼进来的，而猫没有服装，模型填的就是「无」。
/// 判据只有「非空」，于是「无」这个字被当成外貌特征发给了出图模型。
///
/// 源头已经在 `prompts.rs` 堵住了，这里是第二道 —— **老数据里已经写死了**，
/// 而那些描述是用户资产库里的内容，不能替他重写。
fn clean_description(desc: &str) -> String {
    const PLACEHOLDERS: [&str; 8] = ["无", "没有", "未知", "不详", "none", "n/a", "na", "-"];
    desc.split(['，', ','])
        .map(str::trim)
        .filter(|seg| {
            !seg.is_empty() && !PLACEHOLDERS.contains(&seg.to_ascii_lowercase().as_str())
        })
        .collect::<Vec<_>>()
        .join("，")
}

/// 参考图路径，去重、上限 4 张。
///
/// 上限是 H3 那边的实际约束，也是让 payload 别失控。
pub fn reference_images(resolved: &[ResolvedRef]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for r in resolved {
        if let Some(p) = &r.image_path {
            if !out.contains(p) {
                out.push(p.clone());
            }
        }
        if out.len() == 4 {
            break;
        }
    }
    out
}

/// 界面用：解析当前提示词里的 `@` 引用，用来画头像行和"没对上"的告警。
#[tauri::command]
pub fn assets_resolve_refs(
    state: tauri::State<'_, crate::AppState>,
    prompt: String,
) -> Vec<ResolvedRef> {
    resolve_refs(&state.config_dir, &prompt)
}

/// 读一个媒体文件，回 data URL。**这是自定义协议的兜底通道。**
///
/// 界面优先用 `anima-media://`（浏览器缓存、不走 IPC、内存友好）。
/// 但自定义协议能不能在这个 webview 里工作，只有跑起来才知道 ——
/// 而「图显示不出来」这件事已经造成过一次真实伤害：
/// 有人因为路径显示不了，改成把 base64 存进 JSON，于是有了 86 MB 的
/// `characters.json`。所以留一条一定能走通的路。
///
/// 走的是同一道 [`resolve_media_path`] 边界，不因为是兜底就放宽。
#[tauri::command]
pub fn assets_read_media(
    state: tauri::State<'_, crate::AppState>,
    path: String,
) -> Result<String> {
    use base64::Engine;
    let roots = media_roots(&state.config_dir, &crate::media_extra_roots());
    let real = resolve_media_path(&roots, Path::new(&path))?;
    let bytes =
        std::fs::read(&real).map_err(|e| Error::runtime(format!("读不到媒体文件：{e}")))?;
    // 超过 24 MB 就别走 IPC 了 —— 编码成 base64 还要再涨三分之一，
    // 一张图把整条 IPC 堵住比显示不出来更糟。
    const MAX_INLINE: usize = 24 * 1024 * 1024;
    if bytes.len() > MAX_INLINE {
        return Err(Error::runtime(format!(
            "文件太大（{} MB），走不了兜底通道",
            bytes.len() / 1024 / 1024
        )));
    }
    Ok(format!(
        "data:{};base64,{}",
        media_mime(&real),
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    ))
}


#[cfg(test)]
mod tests {
    use super::*;

    /// 两个项目都有「皇帝」、全局库也有一个：知道项目时取这个项目的（本集优先）；
    /// 不知道项目时多个匹配用全局库的，没有全局的就当没找到，不猜
    #[test]
    fn 按名字找资产_限定项目内_从近到远() {
        let dir = tmp();
        let cfg = dir.path();
        let put = |sc: Scope, name: &str| {
            let mut store = load_store(cfg, &sc);
            let a = Asset::new(AssetKind::Character, name);
            let id = a.id.clone();
            store.assets.push(a);
            save_store(cfg, &sc, &store).unwrap();
            id
        };
        let ep = |p: &str| Scope::Episode { project_id: p.into(), episode_id: "ep_01".into() };
        std::fs::create_dir_all(cfg.join("projects/p1/episodes/ep_01")).unwrap();
        std::fs::create_dir_all(cfg.join("projects/p2/episodes/ep_01")).unwrap();
        let p1_ep = put(ep("p1"), "皇帝");
        let p2_proj = put(Scope::Project { project_id: "p2".into() }, "皇帝");
        let find = |scope: RefScope| find_asset_by_name(cfg, AssetKind::Character, "皇帝", &scope).map(|(_, a)| a.id);

        assert_eq!(find(RefScope::Project { project_id: "p1", episode_id: Some("ep_01") }), Some(p1_ep.clone()));
        assert_eq!(find(RefScope::Project { project_id: "p1", episode_id: None }), Some(p1_ep.clone()), "不知道哪一集时查项目的所有分集");
        assert_eq!(find(RefScope::Project { project_id: "p2", episode_id: Some("ep_01") }), Some(p2_proj.clone()), "本集没有落到项目");
        assert_eq!(find(RefScope::Anywhere), None, "两个项目都有、全局没有：不猜");
        let global = put(Scope::Global, "皇帝");
        assert_eq!(find(RefScope::Anywhere), Some(global.clone()), "多个匹配时用全局库的");
        assert_eq!(find(RefScope::Project { project_id: "p3", episode_id: None }), Some(global), "项目里没有落到全局");
        // 提示词标签走同一套
        let r = resolve_refs_in(cfg, "[角色:皇帝]上朝", &RefScope::Project { project_id: "p2", episode_id: None });
        assert_eq!(r[0].asset_id.as_deref(), Some(p2_proj.as_str()));
    }


    #[test]
    fn 老数据没有形象字段也读得出来_主形象就是资产自己() {
        let a: Asset = serde_json::from_str(r#"{"id":"char_1","kind":"character","name":"林婉清","description":"二十八岁女子，男装官服","created_at":0,"updated_at":0}"#).unwrap();
        assert!(a.looks.is_empty() && a.identity.is_empty() && !a.nsfw);
        assert_eq!(a.look_description(None), "二十八岁女子，男装官服");
        assert_eq!(a.look_description(Some(MAIN_LOOK)), "二十八岁女子，男装官服");
        assert!(a.has_look(MAIN_LOOK) && !a.has_look("look_x"));
    }

    #[test]
    fn 形象描述_人物本身加形象描述_找不到落回主形象() {
        let mut a = Asset::new(AssetKind::Character, "林婉清");
        a.description = "白衬衫黑西裤".into();
        let l = add_look(&mut a, "盛唐男装", "高马尾束冠，深青色圆领官服").unwrap();
        // 人物本身没填：形象名 + 形象描述
        assert_eq!(a.look_description(Some(&l.id)), "盛唐男装：\n高马尾束冠，深青色圆领官服");
        a.identity = "二十八岁女子，鹅蛋脸，杏眼".into();
        assert_eq!(a.look_description(None), "二十八岁女子，鹅蛋脸，杏眼\n白衬衫黑西裤");
        assert_eq!(a.look_description(Some(&l.id)), "二十八岁女子，鹅蛋脸，杏眼\n高马尾束冠，深青色圆领官服");
        assert_eq!(a.look_description(Some("look_gone")), a.look_description(None), "形象被删了落回主形象");
        assert!(add_look(&mut a, "  ", "").is_err());
        assert!(add_look(&mut Asset::new(AssetKind::Scene, "金銮殿"), "夜景", "").is_err(), "场景没有形象");
        // 道具有：同一块协议铭牌，量产款和星澜专属款长得不一样
        let mut plate = Asset::new(AssetKind::Prop, "协议铭牌");
        assert_eq!(add_look(&mut plate, "星澜专属", "青色晶石").unwrap().name, "星澜专属");
        assert_eq!(plate.look_description(Some(&plate.looks[0].id.clone())), "星澜专属：\n青色晶石", "道具没有人物本身，发给模型的描述前面带上形象名");
    }

    #[test]
    fn 每个形象的候选图各自独立_选中删除收藏都认得出在哪个形象() {
        let dir = tmp();
        let cfg = dir.path();
        let sc = Scope::Global;
        let mut a = Asset::new(AssetKind::Character, "林婉清");
        let look = add_look(&mut a, "盛唐", "官服").unwrap();
        let m1 = add_variant(cfg, &sc, &mut a, b"main", "png", false, None, false, None).unwrap();
        let l1 = add_variant_to_look(cfg, &sc, &mut a, Some(&look.id), b"l1", "png", false, None, false, None).unwrap();
        let l2 = add_variant_to_look(cfg, &sc, &mut a, Some(&look.id), b"l2", "png", false, None, false, None).unwrap();
        assert!(add_variant_to_look(cfg, &sc, &mut a, Some("look_gone"), b"x", "png", false, None, false, None).is_err());
        assert_eq!(a.unit.image_variants.len(), 1, "形象的图不能进主形象");
        assert_eq!(a.unit.selected_image_id.as_deref(), Some(m1.id.as_str()));
        let lu = a.look_unit(Some(&look.id)).unwrap();
        assert_eq!((lu.image_variants.len(), lu.selected_image_id.as_deref()), (2, Some(l2.id.as_str())));
        // 删形象里被选中的那张：改选剩下的，主形象不受影响
        let rel = remove_variant_in_place(cfg, &sc, &mut a, &l2.id).unwrap();
        assert!(rel.contains(&l2.id));
        assert_eq!(a.look_unit(Some(&look.id)).unwrap().selected_image_id.as_deref(), Some(l1.id.as_str()));
        assert_eq!(a.unit.selected_image_id.as_deref(), Some(m1.id.as_str()));
        // 删形象：回它的文件
        let rels = remove_look(&mut a, &look.id).unwrap();
        assert_eq!(rels.len(), 1);
        assert!(remove_look(&mut a, MAIN_LOOK).is_err());
    }

    #[test]
    fn 改形象_主形象改的是资产自己的字段() {
        let mut a = Asset::new(AssetKind::Character, "林婉清");
        let l = add_look(&mut a, "盛唐", "").unwrap();
        let aid = a.id.clone();
        let req = |look_id: &str, name: Option<&str>, nsfw: Option<bool>| UpdateLookReq {
            scope: Scope::Global, id: aid.clone(), look_id: look_id.into(),
            name: name.map(String::from), description: Some("新描述".into()), nsfw,
        };
        let r = req(MAIN_LOOK, Some("现代"), Some(true));
        update_look(&mut a, &r).unwrap();
        assert_eq!((a.main_look_name.as_str(), a.description.as_str(), a.nsfw), ("现代", "新描述", true));
        let r = req(&l.id, Some("盛唐官服"), Some(true));
        update_look(&mut a, &r).unwrap();
        assert_eq!((a.looks[0].name.as_str(), a.looks[0].description.as_str(), a.looks[0].nsfw), ("盛唐官服", "新描述", true));
        assert!(update_look(&mut a, &req(&l.id, Some(" "), None)).is_err());
    }

    #[test]
    fn 列表里带形象_主形象在第一个() {
        let dir = tmp();
        let cfg = dir.path();
        let sc = Scope::Global;
        let mut a = Asset::new(AssetKind::Character, "林婉清");
        a.identity = "女".into();
        let l = add_look(&mut a, "盛唐", "官服").unwrap();
        add_variant_to_look(cfg, &sc, &mut a, Some(&l.id), b"x", "png", false, None, false, None).unwrap();
        a.looks[0].nsfw = true;
        save_store(cfg, &sc, &AssetStore { assets: vec![a] }).unwrap();
        let v = &list_assets(cfg, Some(sc))[0];
        assert_eq!(v.look_views.len(), 2);
        assert!(v.look_views[0].is_main && v.look_views[0].name == "主形象" && v.look_views[0].cover_url.is_none());
        let lv = &v.look_views[1];
        assert_eq!((lv.name.as_str(), lv.nsfw, lv.prompt.as_str(), lv.image_urls.len()), ("盛唐", true, "女\n官服", 1));
        assert!(lv.cover_url.is_some());
    }


    #[test]
    fn range_头的几种写法() {
        assert_eq!(parse_range("bytes=0-1", 100), Some((0, 1)));
        assert_eq!(parse_range("bytes=90-", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=-10", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=50-500", 100), Some((50, 99)), "尾巴越界截到文件末");
        let big = 1u64 << 30;
        assert_eq!(parse_range("bytes=0-", big), Some((0, MAX_RANGE_CHUNK - 1)), "开放区间要截断");
        for bad in ["bytes=100-", "bytes=5-1", "bytes=0-1,5-9", "items=0-1", "bytes=x-1", "bytes=-0"] {
            assert_eq!(parse_range(bad, 100), None, "{bad}");
        }
        assert_eq!(parse_range("bytes=0-1", 0), None);
    }

    /// 放视频靠 Range。整个回的话 123 MB 的成片每次缓冲都整读整传，播放卡顿。
    #[test]
    fn 媒体协议按_range_回一段() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("v.mp4");
        std::fs::write(&f, (0u8..100).collect::<Vec<_>>()).unwrap();
        let r = media_response(&f, Some("bytes=10-19"), None);
        assert_eq!(r.status(), 206);
        assert_eq!(r.body(), &(10u8..20).collect::<Vec<_>>());
        assert_eq!(r.headers()["Content-Range"], "bytes 10-19/100");
        assert_eq!(r.headers()["Content-Type"], "video/mp4");
        assert_eq!(r.headers()["Accept-Ranges"], "bytes");
        let r = media_response(&f, Some("bytes=200-"), None);
        assert_eq!(r.status(), 416);
        assert_eq!(r.headers()["Content-Range"], "bytes */100");
        // 不带 Range：整个回，并告诉播放器可以按段要
        let r = media_response(&f, None, None);
        assert_eq!((r.status().as_u16(), r.body().len()), (200, 100));
        assert_eq!(r.headers()["Accept-Ranges"], "bytes");
        let etag = r.headers()["ETag"].to_str().unwrap().to_string();
        assert_eq!(media_response(&f, None, Some(&etag)).status(), 304);
        assert_eq!(media_response(&d.path().join("none.mp4"), None, None).status(), 404);
    }

    use std::fs;

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    // ---- 地基：JSON 里不许有 base64 -------------------------------------

    /// **错配的角色要在提交的当下拒绝。**
    ///
    /// 把「场景反打」挂到角色身上不会报错，只是以后按角色取的时候取不到 ——
    /// 而那时候没人会想到是这一步标错了。
    /// **删到封面头上时必须改选一张。**
    ///
    /// 封面就是 `@` 引用时真正发出去的那张。删了不改选的话，
    /// `selected_image_id` 指向一个不存在的变体 —— 界面上封面位空了，
    /// 而分镜里那个 `[场景:X]` 还在引用它，出图时只剩名字、长相带不上，
    /// **而且不报错**。
    #[test]
    fn 删掉封面之后要改选一张() {
        let dir = tempfile::tempdir().unwrap();
        let scope = Scope::Global;
        let media = scope.media_dir(dir.path());
        std::fs::create_dir_all(&media).unwrap();
        let mut a = Asset::new(AssetKind::Scene, "金銮殿");
        for n in ["a", "b", "c"] {
            std::fs::write(media.join(format!("{n}.png")), b"x").unwrap();
            a.unit.image_variants.push(MediaVariant {
                id: n.into(),
                rel_path: format!("{n}.png"),
                created_at: 0,
                prompt_used: None,
                favorited: false,
                from_upload: false,
                role: None,
            });
        }
        a.unit.selected_image_id = Some("b".into());

        // 删掉封面 b
        let removed = remove_variant_in_place(dir.path(), &scope, &mut a, "b").unwrap();
        assert_eq!(removed, "b.png");
        assert_eq!(
            a.unit.selected_image_id.as_deref(),
            Some("c"),
            "封面没了却没改选 —— @ 引用会指向不存在的变体"
        );
        assert_eq!(a.unit.image_variants.len(), 2);
    }

    /// 删的不是封面时，封面一动不动。
    #[test]
    fn 删别的不动封面() {
        let dir = tempfile::tempdir().unwrap();
        let scope = Scope::Global;
        std::fs::create_dir_all(scope.media_dir(dir.path())).unwrap();
        let mut a = Asset::new(AssetKind::Scene, "x");
        for n in ["a", "b"] {
            a.unit.image_variants.push(MediaVariant {
                id: n.into(),
                rel_path: format!("{n}.png"),
                created_at: 0,
                prompt_used: None,
                favorited: false,
                from_upload: false,
                role: None,
            });
        }
        a.unit.selected_image_id = Some("b".into());
        remove_variant_in_place(dir.path(), &scope, &mut a, "a").unwrap();
        assert_eq!(a.unit.selected_image_id.as_deref(), Some("b"));
    }

    /// 删掉最后一张时封面清空，而不是留一个指向空气的 id。
    #[test]
    fn 删光之后封面清空() {
        let dir = tempfile::tempdir().unwrap();
        let scope = Scope::Global;
        std::fs::create_dir_all(scope.media_dir(dir.path())).unwrap();
        let mut a = Asset::new(AssetKind::Scene, "x");
        a.unit.image_variants.push(MediaVariant {
            id: "only".into(),
            rel_path: "only.png".into(),
            created_at: 0,
            prompt_used: None,
            favorited: false,
            from_upload: false,
            role: None,
        });
        a.unit.selected_image_id = Some("only".into());
        remove_variant_in_place(dir.path(), &scope, &mut a, "only").unwrap();
        assert!(a.unit.selected_image_id.is_none(), "指向空气的封面 id");
        assert!(a.unit.image_variants.is_empty());
    }

    /// 不存在的变体要报错，不能静静地什么都不做。
    #[test]
    fn 删不存在的变体要报错() {
        let dir = tempfile::tempdir().unwrap();
        let scope = Scope::Global;
        let mut a = Asset::new(AssetKind::Scene, "x");
        assert!(remove_variant_in_place(dir.path(), &scope, &mut a, "没有这个").is_err());
    }

    #[test]
    fn 变体角色和资产类型必须匹配() {
        use VariantRole::*;
        assert!(Portrait.allows(AssetKind::Character));
        assert!(Turnaround.allows(AssetKind::Character));
        assert!(!Portrait.allows(AssetKind::Scene));
        assert!(!Turnaround.allows(AssetKind::Prop));

        assert!(SceneMaster.allows(AssetKind::Scene));
        assert!(SceneReverse.allows(AssetKind::Scene));
        assert!(ScenePano.allows(AssetKind::Scene));
        assert!(!SceneMaster.allows(AssetKind::Character));

        // 通用参考图哪儿都能放 —— 说不清是哪种的就是它
        for k in [AssetKind::Character, AssetKind::Scene, AssetKind::Prop] {
            assert!(Reference.allows(k));
        }
    }

    // ---- 场景空间契约 ---------------------------------------------------

    fn 存场景契约(dir: &Path, name: &str, c: crate::scene_contract::SceneContract) -> String {
        let mut store = load_store(dir, &Scope::Global);
        let mut a = Asset::new(AssetKind::Scene, name);
        a.scene = Some(c);
        let id = a.id.clone();
        store.assets.push(a);
        save_store(dir, &Scope::Global, &store).unwrap();
        id
    }

    fn 面馆契约() -> crate::scene_contract::SceneContract {
        use crate::scene_contract::*;
        SceneContract {
            shape: "东西进深的狭长一层店铺".into(),
            entrances: vec![Entrance { name: "东侧卷帘门".into(), connects_to: "街道".into() }],
            anchors: vec![Anchor {
                name: "北侧旧木柜台".into(),
                zone: "北墙".into(),
                size: AnchorSize::WallDominant,
                must_not_become: vec!["现代吧台".into()],
            }],
            views: vec![SceneView {
                name: "入口正打".into(),
                from: "东侧卷帘门内侧".into(),
                toward: "店堂深处".into(),
                far: "靠北墙的旧木柜台".into(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn 场景契约按名字查得到() {
        let d = tmp();
        存场景契约(d.path(), "安和面馆", 面馆契约());
        let got = scene_contract_by_name(d.path(), "安和面馆").unwrap();
        assert_eq!(got.anchors[0].name, "北侧旧木柜台");
        // 带空白的名字也要对上 —— `scene_name` 是人手填的
        assert!(scene_contract_by_name(d.path(), "  安和面馆  ").is_some());
        assert!(scene_contract_by_name(d.path(), "别的店").is_none());
        assert!(scene_contract_by_name(d.path(), "").is_none());
    }

    /// **空契约要当没有。**
    ///
    /// 它一条地理都锁不住，却会把调用方的默认视角表顶掉 —— 出一张更差的图，
    /// 而且不报错。
    #[test]
    fn 空的场景契约当没有() {
        let d = tmp();
        存场景契约(d.path(), "空壳", crate::scene_contract::SceneContract::default());
        assert!(scene_contract_by_name(d.path(), "空壳").is_none());
    }

    #[test]
    fn 同名的角色资产不会被当成场景() {
        let d = tmp();
        let mut store = load_store(d.path(), &Scope::Global);
        store.assets.push(Asset::new(AssetKind::Character, "安和面馆"));
        save_store(d.path(), &Scope::Global, &store).unwrap();
        assert!(scene_contract_by_name(d.path(), "安和面馆").is_none());
    }

    /// **地理锁发了参考图也要发。**
    ///
    /// 一张场景板只画了一个方向，换个方向的镜头里「柜台在哪、有几扇门」
    /// 图上根本没有 —— 这时候只有文字能带过去。
    #[test]
    fn 场景引用带出文字地图和地理锁() {
        let d = tmp();
        存场景契约(d.path(), "安和面馆", 面馆契约());
        let r = resolve_refs(d.path(), "[场景:安和面馆] 傍晚");
        assert_eq!(r.len(), 1);
        assert!(r[0].anchors.contains("东西进深"), "文字地图没带上：{}", r[0].anchors);
        assert!(r[0].lock.contains("北侧旧木柜台"), "地理锁没带上：{}", r[0].lock);
        assert!(r[0].lock.contains("不要画成现代吧台"), "负向锁没带上：{}", r[0].lock);
        assert!(r[0].lock.contains("入口只有 1 处"), "{}", r[0].lock);
    }

    #[test]
    fn 没契约的场景照旧只有描述() {
        let d = tmp();
        let mut store = load_store(d.path(), &Scope::Global);
        let mut a = Asset::new(AssetKind::Scene, "走廊");
        a.description = "昏暗的楼道".into();
        store.assets.push(a);
        save_store(d.path(), &Scope::Global, &store).unwrap();
        let r = resolve_refs(d.path(), "[场景:走廊]");
        assert_eq!(r[0].description, "昏暗的楼道");
        assert!(r[0].lock.is_empty());
        assert!(r[0].anchors.is_empty());
    }

    /// **老存档里没有 `scene` 字段。** 加一个字段就让所有已存资产读不出来，
    /// 是这一类改动最容易出的事故。
    #[test]
    fn 老资产没有场景字段也能读出来() {
        let old = r#"{"id":"scene_1","kind":"scene","name":"走廊","created_at":1,"updated_at":1}"#;
        let a: Asset = serde_json::from_str(old).unwrap();
        assert_eq!(a.name, "走廊");
        assert!(a.scene.is_none());
    }

    /// **老变体没有 role 字段，读出来是 `None`。**
    /// `None` 的语义就是「通用参考图」，正好是存量数据本来的意思。
    #[test]
    fn 老变体没有角色字段也能读出来() {
        let old = r#"{"id":"v1","rel_path":"a/b.png","created_at":1,"favorited":false}"#;
        let v: MediaVariant = serde_json::from_str(old).unwrap();
        assert_eq!(v.role, None);
        assert_eq!(v.id, "v1");
    }

    #[test]
    fn 存盘时拒绝_base64() {
        let dir = tmp();
        let scope = Scope::Global;
        let mut store = AssetStore::default();
        let mut a = Asset::new(AssetKind::Character, "老茶师");
        a.unit.image_variants.push(MediaVariant {
            id: "v1".into(),
            rel_path: "data:image/png;base64,iVBORw0KGgo=".into(),
            created_at: 0,
            prompt_used: None,
            role: None,
            favorited: false,
            from_upload: true,
        });
        store.assets.push(a);
        let err = save_store(dir.path(), &scope, &store).unwrap_err();
        assert!(
            err.to_string().contains("base64"),
            "报错要说清是 base64 回流了，实际：{err}"
        );
        assert!(
            !scope.store_file(dir.path()).exists(),
            "拒绝之后不能留下半个文件"
        );
    }

    #[test]
    fn 存路径的资产可以正常落盘并读回() {
        let dir = tmp();
        let scope = Scope::Global;
        let mut store = AssetStore::default();
        let mut a = Asset::new(AssetKind::Scene, "老茶馆");
        a.description = "雨天午后".into();
        a.starred = true;
        let id = a.id.clone();
        store.assets.push(a);
        save_store(dir.path(), &scope, &store).unwrap();

        let back = load_store(dir.path(), &scope);
        assert_eq!(back.assets.len(), 1);
        assert_eq!(back.assets[0].id, id);
        assert!(back.assets[0].starred);
        assert!(
            back.assets[0].character.is_none(),
            "场景不该带角色专属字段"
        );
    }

    #[test]
    fn 角色建出来自带角色字段_场景不带() {
        assert!(Asset::new(AssetKind::Character, "x").character.is_some());
        assert!(Asset::new(AssetKind::Prop, "x").character.is_none());
    }

    // ---- 变体池 ----------------------------------------------------------

    #[test]
    fn 加变体会落盘并存相对路径() {
        let dir = tmp();
        let scope = Scope::Global;
        let mut a = Asset::new(AssetKind::Character, "老茶师");
        let v = add_variant(dir.path(), &scope, &mut a, b"PNGDATA", "png", false, None, false, None)
            .unwrap();
        assert!(
            !v.rel_path.contains(dir.path().to_str().unwrap()),
            "存的必须是相对路径，实际：{}",
            v.rel_path
        );
        assert!(v.rel_path.starts_with(&a.id));
        let abs = variant_abs_path(dir.path(), &scope, &v);
        assert_eq!(fs::read(abs).unwrap(), b"PNGDATA");
        assert_eq!(a.unit.selected_image_id.as_deref(), Some(v.id.as_str()));
    }

    #[test]
    fn 超出上限删最旧的_收藏和选中的留下() {
        let dir = tmp();
        let scope = Scope::Global;
        let mut a = Asset::new(AssetKind::Character, "老茶师");
        let mut ids = Vec::new();
        for i in 0..(MAX_VARIANTS + 5) {
            let v = add_variant(
                dir.path(),
                &scope,
                &mut a,
                format!("img{i}").as_bytes(),
                "png",
                false,
                None,
                false,
                None,
            )
            .unwrap();
            ids.push(v.id);
            if i == 0 {
                // 第一张收藏 —— 它是最旧的，正常会被第一个删掉。
                a.unit.image_variants[0].favorited = true;
            }
        }
        prune_variants(dir.path(), &scope, &mut a);

        assert!(
            a.unit.image_variants.iter().any(|v| v.id == ids[0]),
            "收藏过的最旧变体被删了 —— 收藏就是白收藏"
        );
        let selected = a.unit.selected_image_id.clone().unwrap();
        assert!(
            a.unit.image_variants.iter().any(|v| v.id == selected),
            "正在选中的变体被删了 —— 界面上封面会突然变空"
        );
        // 非收藏、非选中的不超过上限。
        let removable = a
            .unit
            .image_variants
            .iter()
            .filter(|v| !v.favorited && v.id != selected)
            .count();
        assert!(removable <= MAX_VARIANTS, "实际留了 {removable} 个");

        // 磁盘文件也要跟着删，不能只从 JSON 里摘掉。
        let on_disk = fs::read_dir(scope.media_dir(dir.path()).join(&a.id))
            .unwrap()
            .count();
        assert_eq!(
            on_disk,
            a.unit.image_variants.len(),
            "JSON 里删了但文件还在 —— 会攒出一堆孤儿文件"
        );
    }

    // ---- 媒体协议的安全边界 ---------------------------------------------

    #[test]
    fn 媒体路径只放行允许目录内的文件() {
        let root = tmp();
        let outside = tmp();
        let inside_file = root.path().join("ok.png");
        fs::write(&inside_file, b"x").unwrap();
        let outside_file = outside.path().join("secret.png");
        fs::write(&outside_file, b"x").unwrap();

        let roots = vec![root.path().to_path_buf()];
        assert!(resolve_media_path(&roots, &inside_file).is_ok());
        assert!(
            resolve_media_path(&roots, &outside_file).is_err(),
            "根目录外的文件必须拒绝"
        );
    }

    #[test]
    fn 媒体路径挡住_上级目录穿越() {
        let dir = tmp();
        // **根目录必须先 canonicalize。** macOS 上 `/tmp` 是指向
        // `/private/tmp` 的符号链接 —— 不规范化的话，即便把
        // `resolve_media_path` 里的 canonicalize 拆掉，前缀比较也会因为
        // `/tmp` != `/private/tmp` 而「碰巧」拒绝，测试变成绿的却什么都没钉住。
        // （这条是变异测试逼出来的：改坏实现后它依然通过。）
        let root = dir.path().canonicalize().unwrap();
        let sub = root.join("library");
        fs::create_dir_all(&sub).unwrap();
        fs::write(root.join("secret.png"), b"x").unwrap();

        let roots = vec![sub.clone()];
        let escaped = sub.join("..").join("secret.png");
        assert!(
            resolve_media_path(&roots, &escaped).is_err(),
            "../ 穿越必须被挡住 —— 纯字符串前缀比较挡不住它，而且不报错"
        );
        // 同一个根下的正常文件仍然放行，否则上面那条可能只是「全拒」。
        fs::write(sub.join("ok.png"), b"x").unwrap();
        assert!(resolve_media_path(&roots, &sub.join("ok.png")).is_ok());
    }

    #[test]
    fn 媒体_url_能原样还原带空格的路径() {
        // 真实路径就是这个样子：`~/Library/Application Support/…`。
        // 空格没编码好的话，界面上是一个碎图图标，而且不报错。
        let p = Path::new("/Users/x/Library/Application Support/com.anima.desktop/a b.png");
        let url = media_url(p);
        assert!(!url.contains(' '), "URL 里不能留裸空格：{url}");
        assert_eq!(parse_media_request(&url).as_deref(), Some(p));
    }

    #[test]
    fn 媒体_url_能原样还原中文路径() {
        let p = Path::new("/tmp/资产/老茶师 01.png");
        assert_eq!(parse_media_request(&media_url(p)).as_deref(), Some(p));
    }

    #[test]
    fn 缺少参数的媒体请求解不出路径() {
        assert!(parse_media_request("anima-media://localhost/").is_none());
        assert!(parse_media_request("anima-media://localhost/?q=1").is_none());
        assert!(parse_media_request("anima-media://localhost/?p=").is_none());
    }

    #[test]
    fn 常见媒体类型都有对应的_mime() {
        assert_eq!(media_mime(Path::new("a.PNG")), "image/png");
        assert_eq!(media_mime(Path::new("a.webp")), "image/webp");
        assert_eq!(media_mime(Path::new("a.mp4")), "video/mp4");
        assert_eq!(media_mime(Path::new("a.xyz")), "application/octet-stream");
    }

    // ---- 提示词内联引用 --------------------------------------------------


    /// **内容变了 ETag 必须变。**
    ///
    /// 分镜首帧固定写 `shots/<镜头 id>.png` —— 重出一次内容换了、URL 没变。
    /// 协议以前回的是 `max-age=31536000, immutable`，webview 一年之内
    /// 都不会再去读，界面上永远是旧图，而且不报任何错。
    #[test]
    fn 文件内容变了_etag_跟着变() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("shot_1.png");
        std::fs::write(&f, b"first").unwrap();
        let a = media_etag(&f);
        // mtime 的分辨率有限，改一下长度就够拉开
        std::fs::write(&f, b"second and longer").unwrap();
        let b = media_etag(&f);
        assert_ne!(a, b, "内容变了 ETag 却没变 —— 界面会一直显示旧图");
    }

    #[test]
    fn 同一个文件没动时_etag_稳定() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.png");
        std::fs::write(&f, b"x").unwrap();
        assert_eq!(media_etag(&f), media_etag(&f));
    }

    /// 读不到元信息时**不能回固定值** —— 那等于告诉浏览器"没变过"，
    /// 而我们其实不知道。宁可多传一次。
    #[test]
    fn etag_读不到文件时每次都不同() {
        let d = tempfile::tempdir().unwrap();
        let gone = d.path().join("不存在.png");
        let a = media_etag(&gone);
        std::thread::sleep(std::time::Duration::from_millis(2));
        assert_ne!(a, media_etag(&gone));
    }

    /// ETag 要带引号 —— HTTP 规范要求，不带的话有些实现会整个忽略它。
    #[test]
    fn etag_带引号() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.png");
        std::fs::write(&f, b"x").unwrap();
        let e = media_etag(&f);
        assert!(e.starts_with('"') && e.ends_with('"'), "{e}");
    }

    #[test]
    fn 解析提示词里的资产标签() {
        let refs = parse_asset_refs("[角色:老茶师] 在窗边擦茶盏，[场景:老茶馆] 雨天午后");
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].kind, AssetKind::Character);
        assert_eq!(refs[0].name, "老茶师");
        assert_eq!(refs[1].kind, AssetKind::Scene);
        assert_eq!(refs[1].name, "老茶馆");
    }

    #[test]
    fn 全角冒号也要认() {
        // 中文输入法下打出来的就是全角，用户不会注意到区别。
        let refs = parse_asset_refs("[角色：老茶师] 抬头");
        assert_eq!(refs.len(), 1, "全角冒号没认出来");
        assert_eq!(refs[0].name, "老茶师");
    }

    #[test]
    fn 同一个资产出现多次只算一次() {
        let refs = parse_asset_refs("[角色:老茶师] 看向 [角色:老茶师] 的影子");
        assert_eq!(refs.len(), 1);
    }

    #[test]
    fn 不认识的标签不当引用() {
        let refs = parse_asset_refs("[镜头:特写] 一只手 [note] [::]");
        assert!(refs.is_empty(), "实际解析出：{refs:?}");
    }

    #[test]
    fn 送进引擎的提示词里不能有方括号() {
        let prompt = "[角色:老茶师] 在 [场景:老茶馆] 擦茶盏";
        let stripped = strip_ref_tags(prompt);
        assert!(
            !stripped.contains('['),
            "标签没剥干净：{stripped} —— 模型会把方括号和「角色」两个字画进去"
        );
        assert_eq!(stripped, "老茶师 在 老茶馆 擦茶盏");
    }

    // ---- 相对路径 ↔ 显示 URL ---------------------------------------------

    #[test]
    fn 读出来换成_url_写回去换回相对路径() {
        let dir = tmp();
        let scope = Scope::Project { project_id: "p1".into() };
        let media = scope.media_dir(dir.path()).join("yumiko");
        fs::create_dir_all(&media).unwrap();
        fs::write(media.join("a.png"), b"x").unwrap();

        let original = serde_json::json!({
            "avatar_url": "yumiko/a.png",
            "sheet": [{"image_url": "yumiko/a.png"}],
        });
        let mut v = original.clone();
        hydrate_media_urls(dir.path(), &scope, &mut v);
        assert!(
            v["avatar_url"].as_str().unwrap().starts_with("anima-media:"),
            "没换成显示 URL：{}",
            v["avatar_url"]
        );
        assert!(v["sheet"][0]["image_url"]
            .as_str()
            .unwrap()
            .starts_with("anima-media:"));

        dehydrate_media_urls(dir.path(), &scope, &mut v);
        assert_eq!(
            v, original,
            "来回一趟没还原 —— 界面把读到的 URL 提交回来就会被持久化"
        );
    }

    #[test]
    fn 绝对路径一律不动() {
        // 两种绝对路径都要放过：
        //  · `/portraits/xiaoya.jpg` —— 前端 public 下的预设图，不在 media/ 里；
        //  · 一个**真实存在**的绝对路径 —— `media_dir.join("/abs")` 在
        //    Rust 里等于 `/abs`，光靠 `is_file()` 判断会把它换成指向
        //    media/ 之外的 URL，而 dehydrate 又还原不回去 →
        //    存储里就永久留下一条显示层 URL。
        let dir = tmp();
        let outside = tmp();
        let real_abs = outside.path().join("elsewhere.png");
        fs::write(&real_abs, b"x").unwrap();

        let scope = Scope::Project { project_id: "p1".into() };
        let original = serde_json::json!({
            "avatar_url": "/portraits/xiaoya.jpg",
            "other": real_abs.to_string_lossy(),
        });
        let mut v = original.clone();
        hydrate_media_urls(dir.path(), &scope, &mut v);
        assert_eq!(v, original, "绝对路径被改写了：{v}");
    }

    #[test]
    fn 指向不存在文件的相对路径不换() {
        let dir = tmp();
        let scope = Scope::Global;
        let mut v = serde_json::json!({"x": "char_a/gone.png"});
        hydrate_media_urls(dir.path(), &scope, &mut v);
        assert_eq!(v["x"], "char_a/gone.png");
    }

    // ---- @ 引用的解析与展开 ----------------------------------------------

    fn 建一个带图的资产(cfg: &Path, sc: &Scope, kind: AssetKind, name: &str, desc: &str) {
        let mut store = load_store(cfg, sc);
        let mut a = Asset::new(kind, name);
        a.description = desc.into();
        add_variant(cfg, sc, &mut a, b"IMG", "png", false, None, false, None).unwrap();
        store.assets.push(a);
        save_store(cfg, sc, &store).unwrap();
    }

    /// 没有参考图的资产 —— 只在库里有一条记录。
    fn 建一个资产(cfg: &Path, sc: &Scope, kind: AssetKind, name: &str, desc: &str) {
        let mut store = load_store(cfg, sc);
        let mut a = Asset::new(kind, name);
        a.description = desc.into();
        store.assets.push(a);
        save_store(cfg, sc, &store).unwrap();
    }

    #[test]
    fn 引用对上资产时带出描述和参考图() {
        let dir = tmp();
        let cfg = dir.path();
        建一个带图的资产(cfg, &Scope::Global, AssetKind::Character, "老茶师", "白须，青灰长衫");

        let r = resolve_refs(cfg, "[角色:老茶师] 在窗边擦茶盏");
        assert_eq!(r.len(), 1);
        assert!(r[0].asset_id.is_some());
        assert_eq!(r[0].description, "白须，青灰长衫");
        let p = r[0].image_path.as_ref().expect("对上了就该有参考图路径");
        assert!(Path::new(p).is_file(), "参考图路径指向的文件不存在：{p}");

        let expanded = expand_ref_tags("[角色:老茶师] 在窗边擦茶盏", &r);
        assert_eq!(expanded, "老茶师（白须，青灰长衫） 在窗边擦茶盏");
        assert!(!expanded.contains('['), "标签没剥干净");
    }

    #[test]
    fn 库里没有的引用不能静默当成普通文字() {
        let dir = tmp();
        let r = resolve_refs(dir.path(), "[角色:查无此人] 抬头");
        assert_eq!(r.len(), 1, "没对上也要作为一条结果回去，界面才能提示");
        assert!(r[0].asset_id.is_none());
        // 但提示词里的方括号还是必须剥掉。
        assert_eq!(expand_ref_tags("[角色:查无此人] 抬头", &r), "查无此人 抬头");
    }

    #[test]
    fn 没有描述的资产只留名字_不留空括号() {
        let dir = tmp();
        let cfg = dir.path();
        建一个带图的资产(cfg, &Scope::Global, AssetKind::Scene, "老茶馆", "");
        let r = resolve_refs(cfg, "雨天午后的 [场景:老茶馆]");
        assert_eq!(expand_ref_tags("雨天午后的 [场景:老茶馆]", &r), "雨天午后的 老茶馆");
    }

    /// **挨着的两个标签之间要有标点。**
    ///
    /// 用户是连着打的：`[场景:窗台][角色:小花]`。展开之后成了
    /// `窗台（…）小花（…）` —— 两段描述之间一个标点都没有，
    /// 模型读到的是一个连着的长词组。
    #[test]
    fn 相邻的两个标签之间补逗号() {
        let dir = tmp();
        let cfg = dir.path();
        建一个带图的资产(cfg, &Scope::Global, AssetKind::Scene, "窗台", "朝南，光线充足");
        建一个带图的资产(cfg, &Scope::Global, AssetKind::Character, "小花", "三米小团");
        let prompt = "[场景:窗台][角色:小花]";
        let r = resolve_refs(cfg, prompt);
        let out = expand_ref_tags(prompt, &r);
        assert_eq!(out, "窗台（朝南，光线充足），小花（三米小团）");
        assert!(!out.contains("）小"), "两段描述黏在一起了：{out}");
    }

    /// 正文里碰巧出现的 `][` 不该被动 —— 只在两边都确实是标签时才补。
    #[test]
    fn 不是标签的方括号不补逗号() {
        let dir = tmp();
        let out = expand_ref_tags("他翻到 [第三章][附录] 那一页", &resolve_refs(dir.path(), ""));
        assert_eq!(out, "他翻到 [第三章][附录] 那一页");
    }

    /// **「无」不是外形。**
    ///
    /// 抽取时把 `clothing` 拼进描述，判据只有「非空」——
    /// 而猫没有服装，模型填的就是「无」。实测抓到过：
    /// `小花（三花猫）` 的描述末尾是「，雌，无」，那个「无」
    /// 被当成一条外貌特征发给了出图模型。
    #[test]
    fn 描述里的占位词不进提示词() {
        let dir = tmp();
        let cfg = dir.path();
        建一个带图的资产(
            cfg,
            &Scope::Global,
            AssetKind::Character,
            "小花",
            "三个月大的小三花猫，圆溜溜的大眼睛，约3个月，雌，无",
        );
        let r = resolve_refs(cfg, "[角色:小花] 在箱子里");
        let out = expand_ref_tags("[角色:小花] 在箱子里", &r);
        assert!(!out.contains("，无"), "占位词进了提示词：{out}");
        assert!(out.contains("雌"), "把有用的也滤掉了：{out}");
        assert!(out.contains("圆溜溜的大眼睛"), "{out}");
        assert!(!out.ends_with("，）"), "滤完留了个空尾巴：{out}");
    }

    /// 描述里的**空段**也要滤掉：`白须，，青衫` 或者末尾多一个逗号。
    /// 不滤的话拼出来是「（白须，，青衫）」，或者一个空尾巴。
    #[test]
    fn 描述里的空段和多余逗号被清掉() {
        let dir = tmp();
        let cfg = dir.path();
        建一个资产(cfg, &Scope::Global, AssetKind::Character, "老茶师", "白须，，青灰长衫，");
        let r = resolve_refs(cfg, "[角色:老茶师] 擦茶盏");
        let out = expand_ref_tags("[角色:老茶师] 擦茶盏", &r);
        assert_eq!(out, "老茶师（白须，青灰长衫） 擦茶盏");
        assert!(!out.contains("，，"), "留了双逗号：{out}");
        assert!(!out.contains("，）"), "留了空尾巴：{out}");
    }

    #[test]
    fn 整条描述全是占位词时只留名字() {
        let dir = tmp();
        let cfg = dir.path();
        建一个带图的资产(cfg, &Scope::Global, AssetKind::Prop, "道具X", "无，未知，none");
        let r = resolve_refs(cfg, "[道具:道具X] 放在桌上");
        assert_eq!(expand_ref_tags("[道具:道具X] 放在桌上", &r), "道具X 放在桌上");
    }

    /// **有参考图的资产在参考模式下只留名字。**
    ///
    /// 图已经把长相说清楚了。再写一遍六十字外貌描述，是让图和字打架，
    /// 还把用户自己写的画面挤到角落。
    #[test]
    fn 参考模式下有图的资产只留名字() {
        let dir = tmp();
        let cfg = dir.path();
        建一个带图的资产(cfg, &Scope::Global, AssetKind::Character, "小花", "三米小团，圆眼睛");
        let prompt = "[角色:小花] 在箱子里";
        let r = resolve_refs(cfg, prompt);
        assert!(r[0].image_path.is_some(), "前提：这个资产有参考图");

        let full = expand_ref_tags_detail(prompt, &r, RefDetail::Full);
        assert_eq!(full, "小花（三米小团，圆眼睛） 在箱子里");

        let sent = vec![r[0].image_path.clone().unwrap()];
        let lean = expand_ref_tags_detail(prompt, &r, RefDetail::NameWhenSent(&sent));
        assert_eq!(lean, "小花 在箱子里", "图发出去了还带描述");
    }

    /// **有图但没排进那 4 张的资产，描述必须留着。**
    ///
    /// 参考图上限是 4（H3 的硬约束）。要是因为「它有图」就删描述，
    /// 第 5 个资产就图和字两头都没了 —— 而且不报错。
    #[test]
    fn 图没发出去的资产照样带描述() {
        let dir = tmp();
        let cfg = dir.path();
        建一个带图的资产(cfg, &Scope::Global, AssetKind::Character, "小花", "三米小团，圆眼睛");
        let prompt = "[角色:小花] 在箱子里";
        let r = resolve_refs(cfg, prompt);
        assert!(r[0].image_path.is_some(), "前提：它有图");
        // 实际送出去的是别人的图 —— 名单里没有它
        let sent = vec!["/别人的/图.png".to_string()];
        assert_eq!(
            expand_ref_tags_detail(prompt, &r, RefDetail::NameWhenSent(&sent)),
            "小花（三米小团，圆眼睛） 在箱子里"
        );
        // 一张都没发的时候同理
        assert_eq!(
            expand_ref_tags_detail(prompt, &r, RefDetail::NameWhenSent(&[])),
            "小花（三米小团，圆眼睛） 在箱子里"
        );
    }

    /// **没有参考图的资产照样带描述** —— 它没有别的载体，
    /// 一刀切掉等于让引擎随便画。
    #[test]
    fn 没有参考图的资产在参考模式下仍带描述() {
        let dir = tmp();
        let cfg = dir.path();
        建一个资产(cfg, &Scope::Global, AssetKind::Scene, "陌生的房间", "室内新居，光线柔和");
        let prompt = "[场景:陌生的房间] 的角落";
        let r = resolve_refs(cfg, prompt);
        assert!(r[0].image_path.is_none(), "前提：这个资产没有参考图");
        assert_eq!(
            expand_ref_tags_detail(prompt, &r, RefDetail::NameWhenSent(&["/任何/图.png".to_string()])),
            "陌生的房间（室内新居，光线柔和） 的角落"
        );
    }

    #[test]
    fn 参考图去重且最多四张() {
        let dir = tmp();
        let cfg = dir.path();
        for i in 0..6 {
            建一个带图的资产(cfg, &Scope::Global, AssetKind::Character, &format!("角色{i}"), "");
        }
        let prompt = (0..6).map(|i| format!("[角色:角色{i}]")).collect::<Vec<_>>().join(" ");
        let r = resolve_refs(cfg, &prompt);
        assert_eq!(r.len(), 6);
        let imgs = reference_images(&r);
        assert_eq!(imgs.len(), 4, "上限没生效：{}", imgs.len());
        assert_eq!(
            imgs.iter().collect::<std::collections::HashSet<_>>().len(),
            4,
            "参考图有重复"
        );
    }

    #[test]
    fn 同一个资产在提示词里出现两次只算一张参考图() {
        let dir = tmp();
        let cfg = dir.path();
        建一个带图的资产(cfg, &Scope::Global, AssetKind::Character, "老茶师", "");
        let r = resolve_refs(cfg, "[角色:老茶师] 看向 [角色:老茶师] 的影子");
        assert_eq!(r.len(), 1, "解析层就该去重");
        assert_eq!(reference_images(&r).len(), 1);
    }

    #[test]
    fn 参考图列表自己也去重() {
        // 上面那条其实是解析层去的重 —— 把 `reference_images` 里的去重逻辑
        // 拆掉它照样绿。**这条直接喂重复的输入**，钉的才是这个函数本身。
        // （变异测试逼出来的：不然那段去重代码等于没有测试覆盖。）
        let same = |p: &str| ResolvedRef {
            kind: AssetKind::Character,
            name: p.into(),
            asset_id: Some("x".into()),
            description: String::new(),
            anchors: String::new(),
            avoid: String::new(),
            lock: String::new(),
            image_path: Some("/lib/a.png".into()),
            image_url: None,
        };
        let imgs = reference_images(&[same("甲"), same("乙")]);
        assert_eq!(imgs, vec!["/lib/a.png".to_string()]);
    }

    #[test]
    fn 没有引用时不做任何事() {
        let dir = tmp();
        assert!(resolve_refs(dir.path(), "一只猫在窗台上").is_empty());
        assert_eq!(expand_ref_tags("一只猫在窗台上", &[]), "一只猫在窗台上");
    }

    // ---- 引用完整性 ------------------------------------------------------

    fn 写一个分集(config_dir: &Path, project: &str, shot: crate::cinema::StoryboardShot) {
        let ep_dir = config_dir
            .join("projects")
            .join(project)
            .join("episodes")
            .join("ep_01");
        fs::create_dir_all(&ep_dir).unwrap();
        fs::write(
            config_dir.join("projects").join(project).join("project.json"),
            serde_json::json!({"title": "测试剧"}).to_string(),
        )
        .unwrap();
        let ep = crate::cinema::EpisodeData {
            long_take_scenes: Vec::new(),
            one_take: false,
            scene_takes: Vec::new(),
        long_take_context_frames: None,
        scene_renders: Vec::new(),
            style_lock: String::new(),
            id: "ep_01".into(),
            episode_num: 1,
            title: "第一集".into(),
            synopsis: String::new(),
            script_text: String::new(),
            target_secs: 0,
            script_doc: None,
            storyboard: vec![shot],
            storyboard_version: crate::cinema::STORYBOARD_VERSION,
            actor_colors: Vec::new(),
            final_video_url: None,
        };
        fs::write(
            ep_dir.join("episode.json"),
            serde_json::to_string(&ep).unwrap(),
        )
        .unwrap();
    }

    fn 空镜头(id: &str) -> crate::cinema::StoryboardShot {
        crate::cinema::StoryboardShot {
            one_take: false,
            look_overrides: Default::default(),
            end_frame: String::new(),
            scene_no: 0, scene_name: String::new(), duty: String::new(), source: String::new(), lighting: String::new(), scene_items: Vec::new(), stage: Default::default(), segments: Vec::new(), ref_slots: Vec::new(), prompts: Default::default(), unlinked_assets: Vec::new(), takes: Vec::new(),
            action: String::new(), dialogue_lines: Vec::new(), emotion_tags: Vec::new(), blocking: Vec::new(),
            ambient_sound: String::new(), sound_effect: String::new(), score: String::new(),
            sketch_url: None, keyframes: crate::cinema::Keyframes::NONE, end_image_url: None,
            id: id.into(),
            shot_num: 1,
            image_rev: 0,
            video_rev: 0,
            scale: "中景".into(),
            camera_movement: "固定".into(),
            camera_angle: String::new(),
            camera_note: String::new(),
            director_camera: None, director_scene: None,
            dialogue: String::new(),
            character_id: String::new(),
            character_ids: Vec::new(),
            scene_id: None,
            prop_ids: Vec::new(),
            mode: crate::cinema::ShotMode::FirstFrame,
            continuity: Default::default(),
            continuity_manual: false,
            route_choice: None,
            prompt: String::new(),
            negative_prompt: None,
            duration: 3.0,
            image_url: None,
            video_url: None,
            audio_url: None,
            continues_previous: false,
            status: "pending".into(),
        }
    }

    #[test]
    fn 扫得到引用了资产的镜头() {
        let dir = tmp();
        let mut shot = 空镜头("shot_1");
        shot.character_ids = vec!["char_abc".into()];
        写一个分集(dir.path(), "proj1", shot);

        let refs = scan_references(dir.path(), "char_abc");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].project_title, "测试剧");
        assert_eq!(refs[0].episode_title, "第一集");
        assert_eq!(refs[0].shot_id, "shot_1");
        assert!(scan_references(dir.path(), "char_不存在").is_empty());
    }

    #[test]
    fn 旧数据的单数字段也要扫到() {
        // 旧分镜只有 character_id。直接读 character_ids 会得到空列表，
        // 于是「没人用这个角色」，删除保护静默失效。
        let dir = tmp();
        let mut shot = 空镜头("shot_1");
        shot.character_id = "yumiko".into();
        写一个分集(dir.path(), "proj1", shot);
        assert_eq!(
            scan_references(dir.path(), "yumiko").len(),
            1,
            "旧的单数 character_id 没被扫到"
        );
    }

    #[test]
    fn 场景和道具的引用也算数() {
        let dir = tmp();
        let mut shot = 空镜头("shot_1");
        shot.scene_id = Some("scene_x".into());
        shot.prop_ids = vec!["prop_y".into()];
        写一个分集(dir.path(), "proj1", shot);
        assert_eq!(scan_references(dir.path(), "scene_x").len(), 1);
        assert_eq!(scan_references(dir.path(), "prop_y").len(), 1);
    }

    // ---- 迁移 ------------------------------------------------------------

    /// 够大的一坨 base64，让「体积有没有降下来」这个判据有意义 ——
    /// 用 1×1 的图做夹具，pretty-print 的缩进就能把差异淹掉。
    fn 大块_base64() -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(vec![0xABu8; 20_000])
    }

    fn 写一个带_base64_的角色库(config_dir: &Path, project: &str, n_images: usize) {
        let proj = config_dir.join("projects").join(project);
        fs::create_dir_all(&proj).unwrap();
        let data_url = format!("data:image/png;base64,{}", 大块_base64());
        // 故意让两档用**同名的 key**，复现「按 key 命名会互相覆盖」那个坑。
        let item = |i: usize, cat: &str| {
            serde_json::json!({
                "key": format!("slot_{i}"), "title": format!("槽位 {i}"),
                "image_url": data_url, "description": "", "category": cat
            })
        };
        let turnarounds: Vec<_> = (0..n_images / 2).map(|i| item(i, "turnaround")).collect();
        let poses: Vec<_> = (0..n_images - n_images / 2).map(|i| item(i, "pose")).collect();
        let chars = serde_json::json!([{
            "id": "yumiko",
            "name": "由美子",
            "role": "主角",
            "avatar_url": data_url,
            "trigger_words": "1girl",
            "seed": 42,
            "asset_sheet": {"turnarounds": turnarounds, "poses": poses,
                            "expressions": [], "angles": [], "warnings": []},
        }]);
        fs::write(
            proj.join("characters.json"),
            serde_json::to_string_pretty(&chars).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn 迁移把_base64_全部落成文件() {
        let dir = tmp();
        写一个带_base64_的角色库(dir.path(), "p1", 10);
        let report = migrate_project_characters(dir.path(), "p1").unwrap();

        // 10 张切片 + 1 张头像 = 11。**数文件，不看「成功」。**
        assert_eq!(report.images_written, 11, "落盘数不对：{report:?}");
        assert!(report.failures.is_empty(), "有失败：{:?}", report.failures);

        let files = fs::read_dir(
            Scope::Project { project_id: "p1".into() }
                .media_dir(dir.path())
                .join("yumiko"),
        )
        .unwrap()
        .count();
        assert_eq!(
            files, 11,
            "同名 key 互相覆盖了 —— 磁盘上只剩 {files} 个文件"
        );

        let text = fs::read_to_string(
            dir.path().join("projects/p1/characters.json"),
        )
        .unwrap();
        assert!(!text.contains("data:image"), "还有 base64 残留");
        assert!(
            (report.bytes_after as f64) < (report.bytes_before as f64) * 0.2,
            "体积没降下来：{} → {}",
            report.bytes_before,
            report.bytes_after
        );
    }

    #[test]
    fn 迁移会建出剧集级资产并保留角色专属字段() {
        let dir = tmp();
        写一个带_base64_的角色库(dir.path(), "p1", 2);
        migrate_project_characters(dir.path(), "p1").unwrap();

        let store = load_store(
            dir.path(),
            &Scope::Project { project_id: "p1".into() },
        );
        assert_eq!(store.assets.len(), 1);
        let a = &store.assets[0];
        assert_eq!(a.id, "yumiko");
        assert_eq!(a.name, "由美子");
        let t = a.character.as_ref().expect("角色资产必须带 character 字段");
        assert_eq!(t.trigger_words, "1girl");
        assert_eq!(t.seed, Some(42));
        assert!(t.asset_sheet.is_some(), "设定卡丢了 —— 那是我们比参考项目强的地方");
        assert!(
            a.unit.selected_image_id.is_some(),
            "头像没变成第一个变体，资产库里会是空封面"
        );
    }

    #[test]
    fn 启动迁移只碰超过阈值的文件() {
        let dir = tmp();
        // 小文件（已经迁过的样子）：一个字节都不该动。
        let small = dir.path().join("projects/done");
        fs::create_dir_all(&small).unwrap();
        fs::write(small.join("characters.json"), "[]").unwrap();

        // 要真的超过 1 MB 闸门：每张 base64 约 26.7 KB。
        写一个带_base64_的角色库(dir.path(), "legacy", 48);
        let before_small = fs::read_to_string(small.join("characters.json")).unwrap();

        let reports = migrate_legacy_projects_on_start(dir.path());
        assert_eq!(reports.len(), 1, "只该迁那个大的");
        assert_eq!(reports[0].project_id, "legacy");
        assert_eq!(
            fs::read_to_string(small.join("characters.json")).unwrap(),
            before_small,
            "小文件被改写了 —— 每次启动都重写一遍所有项目是没必要的写放大"
        );
    }

    #[test]
    fn 迁移是幂等的() {
        let dir = tmp();
        写一个带_base64_的角色库(dir.path(), "p1", 4);
        let first = migrate_project_characters(dir.path(), "p1").unwrap();
        let second = migrate_project_characters(dir.path(), "p1").unwrap();
        assert_eq!(first.images_written, 5);
        assert_eq!(
            second.images_written, 0,
            "第二次又落了 {} 张 —— 重复跑会不断堆文件",
            second.images_written
        );
        let store = load_store(dir.path(), &Scope::Project { project_id: "p1".into() });
        assert_eq!(store.assets.len(), 1, "重复跑不能建出重复资产");
    }

    // ---- 列表 ------------------------------------------------------------

    /// 两个项目的分集都叫 ep_01 —— 标签必须带项目名，不能只有集的 id
    #[test]
    fn 归属标签用项目名和集数() {
        let dir = tmp();
        let cfg = dir.path();
        for (id, title) in [("p1", "《我在长安写诗词》"), ("p2", "米小团")] {
            let d = cfg.join("projects").join(id).join("episodes").join("ep_01");
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(cfg.join("projects").join(id).join("project.json"), format!(r#"{{"title":"{title}"}}"#)).unwrap();
            std::fs::write(d.join("episode.json"), r#"{"episode_num":1}"#).unwrap();
        }
        let ep = |p: &str| Scope::Episode { project_id: p.into(), episode_id: "ep_01".into() };
        assert_eq!(scope_display(cfg, &ep("p1")), ("《我在长安写诗词》 · 第 1 集".to_string(), Some("《我在长安写诗词》".to_string())));
        assert_eq!(scope_display(cfg, &ep("p2")).0, "米小团 · 第 1 集");
        assert_eq!(scope_display(cfg, &Scope::Project { project_id: "p2".into() }), ("米小团".to_string(), Some("米小团".to_string())));
        assert_eq!(scope_display(cfg, &Scope::Global), ("全局".to_string(), None));
    }

    #[test]
    fn 不给作用域时三级资产一起列出来() {
        let dir = tmp();
        let cfg = dir.path();
        let scopes = [
            Scope::Global,
            Scope::Project { project_id: "p1".into() },
            Scope::Episode { project_id: "p1".into(), episode_id: "ep_01".into() },
        ];
        for (i, sc) in scopes.iter().enumerate() {
            let mut store = AssetStore::default();
            let mut a = Asset::new(AssetKind::Character, format!("角色{i}"));
            add_variant(cfg, sc, &mut a, b"x", "png", false, None, false, None).unwrap();
            // add_variant 自己会盖 updated_at（加了一张图当然算更新过），
            // 所以要排的那个值必须在它之后写。
            a.updated_at = 100 - i as u64;
            store.assets.push(a);
            save_store(cfg, sc, &store).unwrap();
        }

        let list = list_assets(cfg, None);
        assert_eq!(list.len(), 3, "少列了作用域：{:?}", list.len());
        assert_eq!(
            list.iter().map(|v| v.scope_label.as_str()).collect::<Vec<_>>(),
            vec!["全局", "p1", "p1 · ep_01"],
            "归属标签不对，或者排序不是按 updated_at 降序"
        );
        for v in &list {
            let url = v.cover_url.as_ref().expect("有图的资产必须有封面 URL");
            assert!(url.starts_with("anima-media:"), "封面不是显示 URL：{url}");
            // URL 要真的指回那个作用域自己的媒体目录，不能全指到同一处。
            let abs = parse_media_request(url).unwrap();
            assert!(abs.starts_with(v.scope.media_dir(cfg)), "封面指错了目录：{abs:?}");
        }
    }

    #[test]
    fn 没有图的资产封面是空而不是坏链接() {
        let dir = tmp();
        let mut store = AssetStore::default();
        store.assets.push(Asset::new(AssetKind::Scene, "老茶馆"));
        save_store(dir.path(), &Scope::Global, &store).unwrap();
        let list = list_assets(dir.path(), Some(Scope::Global));
        assert_eq!(list.len(), 1);
        assert!(
            list[0].cover_url.is_none(),
            "没有图就该是 None —— 给个指向不存在文件的 URL，界面上是碎图，会被当成加载失败"
        );
    }

    #[test]
    fn 列表带上被引用次数() {
        let dir = tmp();
        let cfg = dir.path();
        let mut store = AssetStore::default();
        let mut a = Asset::new(AssetKind::Character, "老茶师");
        a.id = "char_tea".into();
        store.assets.push(a);
        save_store(cfg, &Scope::Global, &store).unwrap();

        let mut shot = 空镜头("shot_1");
        shot.character_ids = vec!["char_tea".into()];
        写一个分集(cfg, "p1", shot);

        let list = list_assets(cfg, Some(Scope::Global));
        assert_eq!(list[0].reference_count, 1, "引用数没带出来，删除保护在界面上就看不见");
    }

    // ---- 作用域路径 ------------------------------------------------------

    #[test]
    fn 三级作用域各自落在自己的目录() {
        let root = Path::new("/cfg");
        assert_eq!(
            Scope::Global.store_file(root),
            Path::new("/cfg/library/library.json")
        );
        assert_eq!(
            Scope::Project { project_id: "p1".into() }.store_file(root),
            Path::new("/cfg/projects/p1/assets.json")
        );
        assert_eq!(
            Scope::Episode {
                project_id: "p1".into(),
                episode_id: "ep_01".into()
            }
            .store_file(root),
            Path::new("/cfg/projects/p1/episodes/ep_01/assets.json")
        );
    }
    /// **一组模板色值不算身份信息。**
    ///
    /// `CharacterVisualDna` 在每个描述字段都是 null 时仍然渲染出
    /// `iris #3D2314, hair #1A1A1A, …` —— 实测用户项目里四个不同角色
    /// 这一段完全相同。非空，但一点身份信息都没有；拿它当锚点等于
    /// 告诉模型「这四个人长一样」。
    #[test]
    fn 只剩模板色值时不算有锚点() {
        assert!(!spec_has_real_anchor("iris #3D2314, hair #1A1A1A, skin #E8C4A0"));
        assert!(!spec_has_real_anchor(""));
        assert!(!spec_has_real_anchor("   ,  ; "));
        // 有一条真的辨识标记就算
        assert!(spec_has_real_anchor("左眼下方一颗痣, iris #3D2314"));
        assert!(spec_has_real_anchor("eyes: 细长丹凤眼"));
    }

    /// **`visual_dna` 是模板时要退到 `spec`。**
    ///
    /// 机制一直都在、测试一直绿，而发出去的是四个角色共用的一组色值 ——
    /// 用户在界面上一格一格填的长相和服装（`spec`）从来没进过提示词。
    #[test]
    fn 视觉dna是模板时退到外观规格() {
        use crate::character_spec::{CharacterLook, CharacterOutfit, CharacterSpec};
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        let sc = Scope::Global;
        let mut store = load_store(cfg, &sc);
        let mut a = Asset::new(AssetKind::Character, "林婉清（盛唐）");
        a.character = Some(CharacterTraits {
            // 每个描述字段都是 null，只剩默认色值 —— 真实数据就是这样
            visual_dna: Some(serde_json::from_str(
                r##"{"color_anchors":{"iris":"#3D2314","hair":"#1A1A1A"}}"##,
            ).unwrap()),
            spec: Some(CharacterSpec {
                look: CharacterLook {
                    role: "盛唐女官".into(),
                    age: "二十岁".into(),
                    hair: "齐肩发".into(),
                    hair_color: "黑色".into(),
                    ..Default::default()
                },
                outfit: CharacterOutfit { top: "男装官服".into(), ..Default::default() },
            }),
            ..Default::default()
        });
        store.assets.push(a);
        save_store(cfg, &sc, &store).unwrap();

        let r = resolve_refs(cfg, "[角色:林婉清（盛唐）] 跪在殿中");
        let hit = r.first().expect("没解析出引用");
        assert!(hit.anchors.contains("盛唐女官"), "外观规格没顶上来：{}", hit.anchors);
        assert!(hit.anchors.contains("男装官服"), "{}", hit.anchors);
        // **锁面也要换掉，不能留那组模板色值。** 它是发了参考图也会发的
        // 那一段，实测两个造型分身的锁面完全相同 —— 分不开任何人。
        assert!(hit.lock.contains("盛唐女官"), "锁面还是模板色值：{}", hit.lock);
        assert!(!hit.lock.contains('#'), "锁面里还留着色值，白占字数：{}", hit.lock);
        assert!(hit.lock.chars().count() <= crate::character_spec::LOCK_CHARS,
                "锁面太长（{} 字），会挤掉画面描述", hit.lock.chars().count());
    }

    /// **两个造型分身的锁面必须不一样。**
    ///
    /// 锁面是发了参考图也会发的那一段，它的活就是把「林婉清（盛唐）」
    /// 和「林婉清（现代）」分开。实测改之前两条完全相同
    /// （`iris #3D2314, hair #1A1A1A, …`）—— 一组四个角色共用的色值
    /// 分不开任何人。
    #[test]
    fn 造型分身的锁面互不相同() {
        use crate::character_spec::{CharacterLook, CharacterOutfit, CharacterSpec};
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        let sc = Scope::Global;
        let mut store = load_store(cfg, &sc);
        let 同一组色值 = r##"{"color_anchors":{"iris":"#3D2314","hair":"#1A1A1A"}}"##;
        for (name, role, top) in [
            ("林婉清（盛唐）", "盛唐女官", "男装官服"),
            ("林婉清（现代）", "现代办公室职员", "白色衬衫"),
        ] {
            let mut a = Asset::new(AssetKind::Character, name);
            a.character = Some(CharacterTraits {
                visual_dna: Some(serde_json::from_str(同一组色值).unwrap()),
                spec: Some(CharacterSpec {
                    look: CharacterLook { role: role.into(), age: "28岁".into(), ..Default::default() },
                    outfit: CharacterOutfit { top: top.into(), ..Default::default() },
                }),
                ..Default::default()
            });
            store.assets.push(a);
        }
        save_store(cfg, &sc, &store).unwrap();

        let a = resolve_refs(cfg, "[角色:林婉清（盛唐）] 跪着").remove(0);
        let b = resolve_refs(cfg, "[角色:林婉清（现代）] 站着").remove(0);
        assert_ne!(a.lock, b.lock, "两个造型分身的锁面一样，分不开");
        assert!(a.lock.contains("男装官服"), "{}", a.lock);
        assert!(b.lock.contains("白色衬衫"), "{}", b.lock);
    }

    /// **`visual_dna` 真的填过时不要被 `spec` 盖掉。**
    /// 它更精确（Hex 色值 + 辨识标记），是专门为身份一致做的。
    #[test]
    fn 视觉dna有真内容时不被外观规格覆盖() {
        use crate::character_spec::{CharacterLook, CharacterSpec};
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        let sc = Scope::Global;
        let mut store = load_store(cfg, &sc);
        let mut a = Asset::new(AssetKind::Character, "沈星晴");
        a.character = Some(CharacterTraits {
            visual_dna: Some(serde_json::from_str(
                r##"{"marks":["左眼下方两厘米一颗痣"],"color_anchors":{"iris":"#3D2314"}}"##,
            ).unwrap()),
            spec: Some(CharacterSpec {
                look: CharacterLook { role: "现代白领".into(), ..Default::default() },
                ..Default::default()
            }),
            ..Default::default()
        });
        store.assets.push(a);
        save_store(cfg, &sc, &store).unwrap();

        let r = resolve_refs(cfg, "[角色:沈星晴] 站在路边");
        let hit = r.first().unwrap();
        assert!(hit.anchors.contains("一颗痣"), "把真的 DNA 盖掉了：{}", hit.anchors);
        assert!(!hit.anchors.contains("现代白领"), "不该被规格覆盖：{}", hit.anchors);
    }

    /// **发了参考图就不该再灌一遍身份锚点。**
    ///
    /// 那条路上长相已经由像素说清楚了，再写「左眼下方两厘米一颗痣 /
    /// 虹膜 #3D2314」只会和图打架，还挤占用户自己写的画面
    /// （实测过一镜里 5 个资产的描述占了 699 字里的 300 字）。
    /// 没发图那条正相反 —— 文字是唯一的载体，锚点正是这时候救场的。
    #[test]
    fn 身份锚点只在没发参考图时进提示词() {
        let r = ResolvedRef {
            kind: AssetKind::Character,
            name: "沈星晴".into(),
            asset_id: Some("c1".into()),
            description: "牛仔外套，短发".into(),
            anchors: "沈星晴 visual DNA: a small mole 2cm below the left eye, iris #3D2314".into(),
            avoid: "金发".into(),
            lock: "a small mole 2cm below the left eye, iris #3D2314".into(),
            image_path: Some("/lib/x.png".into()),
            image_url: None,
        };
        let prompt = "[角色:沈星晴] 站在路边";

        // 没发图：描述和锚点都进
        let full = expand_ref_tags_detail(prompt, std::slice::from_ref(&r), RefDetail::Full);
        assert!(full.contains("牛仔外套"), "{full}");
        assert!(full.contains("mole 2cm below"), "没发图时锚点该进来：{full}");
        assert!(full.contains("#3D2314"), "{full}");

        // 发了图：**长描述不进，锁面要进。**
        //
        // 参考图只能约束它自己画到的部分 —— 被手挡住的道具、每镜重新构图的
        // 服装细节，都落在身份参考「默认控不住」的一侧。同一件衣服第一镜蓝
        // 第二镜红，通常不是资产写错了，是没有任何一条提示词把颜色带进去。
        let sent = [r.image_path.clone().unwrap()];
        let named = expand_ref_tags_detail(prompt, std::slice::from_ref(&r), RefDetail::NameWhenSent(&sent));
        assert!(!named.contains("牛仔外套"), "发了图不该再写长描述：{named}");
        assert!(!named.contains("visual DNA"), "发了图不该再写整块锚点：{named}");
        assert!(named.contains("#3D2314"), "锁面发了图也要发：{named}");
        assert!(named.contains("mole 2cm below"), "锁面发了图也要发：{named}");
        assert!(named.contains("沈星晴"), "名字要留着：{named}");

        // 动作段：一律只留名字 —— 那一段只负责说谁在动，长相的事画面段说过了
        let only = expand_ref_tags_detail(prompt, std::slice::from_ref(&r), RefDetail::NameOnly);
        assert!(!only.contains("mole"), "动作段不该带锚点：{only}");
        assert!(!only.contains("#3D2314"), "动作段也不该带锁面：{only}");
        assert!(only.contains("沈星晴"));
    }

    /// 没发图时锚点里已经含了锁面，不要重复写两遍。
    #[test]
    fn 锚点已含锁面时不重复() {
        let r = ResolvedRef {
            kind: AssetKind::Character,
            name: "沈星晴".into(),
            asset_id: Some("c1".into()),
            description: String::new(),
            anchors: "沈星晴 visual DNA: a small mole 2cm below the left eye, iris #3D2314".into(),
            avoid: String::new(),
            lock: "a small mole 2cm below the left eye, iris #3D2314".into(),
            image_path: None,
            image_url: None,
        };
        let out = expand_ref_tags_detail("[角色:沈星晴] 站着", std::slice::from_ref(&r), RefDetail::Full);
        assert_eq!(out.matches("#3D2314").count(), 1, "锁面写了两遍：{out}");
    }

}
