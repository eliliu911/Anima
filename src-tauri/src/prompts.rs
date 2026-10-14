//! 各 AI 环节的 system prompt —— **全部外置、可改、可复原**。
//!
//! # 为什么要有这一层
//!
//! 从剧本抽角色、把草稿改写成引擎吃的提示词、读剧本推荐风格 ——
//! 这些都是「AI 把我的话改成了别的话」。埋在代码里的话，用户既看不见
//! 它改了什么，也没法调。这一层让每一段都能读、能改、能一键复原。
//!
//! 结构照搬 LumenX（`src/apps/comic_gen/llm.py`，MIT，Copyright 2026
//! Alibaba）：六个槽位，留空 = 用内置默认，填了整段替换。
//! 默认文案在它的基础上按我们的实际情况改过，改动点在各常量的注释里。
//!
//! # 一条必须按我们的实情改的
//!
//! **参考标签的格式不一样。** LumenX 用 `[character1:小兔子]`（带 slot 编号），
//! 我们用 `[角色:老茶师]`（按名字解析，见 `assets::parse_asset_refs`）。
//! 照抄它的 R2V 提示词会让模型吐出 `[character1:…]` —— 我们解析不了，
//! 于是那串方括号原样进引擎，被模型当画面内容画进去。
//! 出图了、不报错、画面里多一行怪字。

/// 模型用来表达「这一项没有」的那几个词。
///
/// 它们**不是外形描述**，进提示词就是噪声。大小写不敏感，
/// 因为模型有时回英文。
fn is_placeholder(s: &str) -> bool {
    const WORDS: [&str; 8] = ["无", "没有", "未知", "不详", "none", "n/a", "na", "-"];
    WORDS.contains(&s.trim().to_ascii_lowercase().as_str())
}

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptSlot {
    /// 剧本 → 角色 / 场景 / 道具
    EntityExtraction,
    /// **剧本 → 场。** 分镜重构第 3 步的第一次调用，原文必须逐字抄（代码核对）。
    SceneSplit,
    /// **一场 → 镜头表。** 一场一次调用；字段见 `shot_table`，规则照 drama-skills 分镜 SKILL。
    ShotTable,
    /// **一场 → 一镜（一镜出）。** 镜数和每镜时长由代码给（[`crate::one_take`]），
    /// 模型只写内容；镜内**允许**按时间轴写切镜。短片用 —— 15 秒一场被拆出 2 秒的碎镜
    /// 在 H3 上出不了（2026-09-23 用户截图）。
    ShotTableOneTake,
    /// 画面草稿 → 出图提示词
    StoryboardPolish,
    /// 草稿 → 图生视频提示词
    VideoPolish,
    /// **草稿 → 文生视频提示词。**
    ///
    /// # 为什么不能和 `VideoPolish` 共用
    ///
    /// `VideoPolish` 的第 4 条写死了「首帧已经定了画面，不要重复描述
    /// 静态外观」—— 那是图生视频才成立的前提。文生视频**没有首帧**，
    /// 照它润色的结果是：模型被要求「别描述外观」，于是画面里什么都没有。
    ///
    /// 2026-09-19 第一次修这个问题时把文生视频接去了 `StoryboardPolish`
    /// （出图提示词润色）—— 绕开了首帧假设，却换成一份**静态画面**的
    /// 模板，时间、动作、声音这三样文生视频必须要的契约一样没有
    /// （Codex Astra 的第三份审查抓到）。所以单开一槽。
    T2vPolish,
    /// 草稿 → 参考生视频提示词
    R2vPolish,
    /// 剧本 → 推荐 3 种视觉风格
    StyleAnalysis,
    /// **关键词串 → 连贯句子。** 创作台用。
    ///
    /// 关键词体系（`1girl, solo, masterpiece` 这种逗号串）是 CLIP 的语言：
    /// SDXL 的编码器把 77 token 分块取平均，短关键词正合适。
    /// 而 Z-Image 的 Qwen3-4B、Chroma 的 T5-XXL 是长句原生的 ——
    /// 同一串标签发过去不报错，只是大部分不起作用。
    ///
    /// **方向是单向的**：句子引擎上把标签改成句子有用；
    /// 反过来在 SDXL 上把关键词改成句子会更糟。所以界面上按引擎决定要不要给。
    ProsePolish,
    /// **原文 → 剧本。** 小说/大纲/素材改写成分场分镜的中文制片格式。
    ///
    /// `{CRAFT}` 由 [`crate::screenwriter::craft_block`] 按输入里出现的
    /// 发动机**按需**填 —— 不是每次都塞全套手艺。理由见那个模块的注释：
    /// 全塞会稀释真正要紧的几条，而且会诱导模型补造原文里没有的机关。
    ///
    /// 这一步的产物是下游唯一认的东西 —— 抽实体、生成分镜读的都是剧本，
    /// 不是原文。所以格式必须和 `script/format.ts` 的行识别对得上，
    /// 否则排版乱、后面抽实体也跟着乱，而且**不报错**。
    ScreenplayFromSource,
    /// **剧本 + 场景名 → 空间契约（文字地图）。**
    ///
    /// 产物进 [`crate::scene_contract::SceneContract`]，用来派生宫格的
    /// 视角描述、以及每一条提示词都要带的地理锁。
    ///
    /// 这一槽最容易出的错是**写成装饰清单**：「有柜台、有门、有菜单板」——
    /// 每一格都能满足，却可以把它们摆在完全不同的地方。要的是
    /// 「两两关系 + 视线终点」。
    SceneContractExtraction,
    /// **原文 → 改编方案。** 原文和剧本之间缺的那一层。
    ///
    /// 它不写台词，只回答三件事：**哪几章并成哪一集、每集要兑现什么、
    /// 退出状态是什么**。没有这一层的话，模型每次自己现分集 ——
    /// 同一本小说跑两次得到两套完全不同的分法，而且都不报错。
    AdaptationPlan,
    /// **线路 B：一镜 → H3 六段里要写的英文正文。** 分镜重构第 4 步。
    ///
    /// 结构（段名、标签、保留强度、配乐 N/A）由 `prompt_route_b` 的代码定，
    /// 这一槽只写措辞：主体外貌、控制范围的英文、summary、detailed_description、声音、静帧提示词。
    RouteBWriter,
    /// **项目信息 → 封面提示词。** 工作区项目卡「生成封面」用，产物给 Z-Image。
    CoverPoster,
    /// **台词的中文表演指示 → 英文短语。** 线路 B 合成每一镜之前单独调一次。
    ///
    /// 以前让写手模型在正文里顺手把 `<DELIVERY>` 占位翻掉 —— 《回收师》E01 合成的 42 句台词里
    /// 只有 1 句填了，其余被代码静默删掉，分镜里逐句写的情绪和角色身份音色一个字都没进提示词，
    /// H3 念出来是播音腔（2026-09-27）。拆成独立的一步，代码直接拼进台词前面，不再指望写手。
    VoiceDirection,
    /// **一镜 → 写满每一刻的时间段。** 分镜页「细化时间线」用，一镜一次调用。
    ///
    /// 用户 2026-09-27：「同样 12 秒的视频……把时间线写好，描述细致，而不是一笔带过。」
    /// 结构（时长、景别、机位、运镜、台词、人物）不动，只把时间段写细。
    /// 不塞进 `ShotTable`：那是一场一次调用，写细之后会撞输出上限（见 `timeline_detail` 模块注释）。
    TimelineDetail,
    /// **超长视频：一段没写时间的提示词 → 按段写好的 N 段。** 画布 / 创作台 H3 时长超过 15 秒时用（[`crate::long_video`]）。
    /// 提示词里写了时间的由代码按时间拆，不调这一槽（用户 2026-10-11：「提示词里规定了时间范围，就用提示词的；没有，就让模型自己去处理」）。
    LongVideoSplit,
}

impl PromptSlot {
    pub const ALL: [PromptSlot; 18] = [
        PromptSlot::EntityExtraction,
        PromptSlot::SceneSplit,
        PromptSlot::ShotTable,
        PromptSlot::ShotTableOneTake,
        PromptSlot::StoryboardPolish,
        PromptSlot::VideoPolish,
        PromptSlot::T2vPolish,
        PromptSlot::R2vPolish,
        PromptSlot::StyleAnalysis,
        PromptSlot::ProsePolish,
        PromptSlot::ScreenplayFromSource,
        PromptSlot::SceneContractExtraction,
        PromptSlot::AdaptationPlan,
        PromptSlot::RouteBWriter,
        PromptSlot::CoverPoster,
        PromptSlot::VoiceDirection,
        PromptSlot::TimelineDetail,
        PromptSlot::LongVideoSplit,
    ];

    pub fn key(self) -> &'static str {
        match self {
            PromptSlot::EntityExtraction => "entity_extraction",
            PromptSlot::SceneSplit => "scene_split",
            PromptSlot::ShotTable => "shot_table",
            PromptSlot::ShotTableOneTake => "shot_table_one_take",
            PromptSlot::StoryboardPolish => "storyboard_polish",
            PromptSlot::VideoPolish => "video_polish",
            PromptSlot::T2vPolish => "t2v_polish",
            PromptSlot::R2vPolish => "r2v_polish",
            PromptSlot::ProsePolish => "prose_polish",
            PromptSlot::StyleAnalysis => "style_analysis",
            PromptSlot::ScreenplayFromSource => "screenplay_from_source",
            PromptSlot::SceneContractExtraction => "scene_contract_extraction",
            PromptSlot::AdaptationPlan => "adaptation_plan",
            PromptSlot::RouteBWriter => "route_b_writer",
            PromptSlot::CoverPoster => "cover_poster",
            PromptSlot::VoiceDirection => "voice_direction",
            PromptSlot::TimelineDetail => "timeline_detail",
            PromptSlot::LongVideoSplit => "long_video_split",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PromptSlot::EntityExtraction => "提取实体（剧本 → 角色/场景/道具）",
            PromptSlot::SceneSplit => "分场（剧本 → 场）",
            PromptSlot::ShotTable => "镜头表（一场 → 镜头）",
            PromptSlot::ShotTableOneTake => "镜头表 · 一镜出（一场 → 一镜，镜内按时间轴切镜）",
            PromptSlot::StoryboardPolish => "出图提示词润色",
            PromptSlot::VideoPolish => "图生视频提示词润色",
            PromptSlot::T2vPolish => "文生视频提示词润色",
            PromptSlot::R2vPolish => "参考生视频提示词润色",
            PromptSlot::ProsePolish => "关键词改写成句子",
            PromptSlot::StyleAnalysis => "读剧本推荐风格",
            PromptSlot::ScreenplayFromSource => "原文改写成剧本",
            // **明说它还没接进生成流程。**
            //
            // 全仓库只有这里声明了它，没有任何 `run`/`compose` 调用
            // （2026-09-19 查实）。数据结构 `SceneContract` 存在**不等于**
            // 这个槽位已经在用 —— 用户在设置里改它、试跑它，
            // 然后以为生成会跟着变，那是最难查的一种失望。
            PromptSlot::SceneContractExtraction => "场景空间契约（剧本 → 文字地图）· 尚未接入生成",
            PromptSlot::AdaptationPlan => "改编方案（原文 → 分集规划）",
            PromptSlot::RouteBWriter => "线路 B 提示词（一镜 → H3 官方格式：参考六段 / 首帧与文生三字段）",
            PromptSlot::CoverPoster => "项目封面（项目信息 → 海报提示词）",
            PromptSlot::VoiceDirection => "台词语气（中文表演指示 → 英文短语，合成提示词前）",
            PromptSlot::TimelineDetail => "细化时间线（一镜 → 写满每一刻）",
            PromptSlot::LongVideoSplit => "超长视频分段（没写时间的提示词 → 按段写好，画布 H3 超过 15 秒时）",
        }
    }

    /// 这个槽位认哪些占位符。界面上要列出来 ——
    /// 用户改坏了占位符，表现是「AI 完全不知道我在说什么」。
    pub fn placeholders(self) -> &'static [&'static str] {
        match self {
            PromptSlot::EntityExtraction => &["{TEXT}"],
            PromptSlot::SceneSplit => &["{TEXT}", "{FEEDBACK}"],
            PromptSlot::ShotTable => &["{SCENE_NAME}", "{SCENE_TEXT}", "{PREV}", "{ASSETS}", "{METHOD}", "{MIN_SECS}", "{MAX_SECS}", "{SCENE_SECS}", "{SHOT_SIZES}", "{ANGLES}", "{MOVES}", "{PROJECT_RULES}", "{PERFORMANCE}", "{COMBAT}", "{FEEDBACK}"],
            PromptSlot::ShotTableOneTake => &["{SCENE_NAME}", "{SCENE_TEXT}", "{PREV}", "{ASSETS}", "{METHOD}", "{SHOT_COUNT}", "{SHOT_SECS}", "{TOTAL_SECS}", "{SHOT_SIZES}", "{ANGLES}", "{MOVES}", "{PROJECT_RULES}", "{PERFORMANCE}", "{COMBAT}", "{FEEDBACK}"],
            PromptSlot::StoryboardPolish => &["{ASSETS}", "{DRAFT}", "{MAX_CHARS}"],
            PromptSlot::VideoPolish => &["{DRAFT}"],
            PromptSlot::T2vPolish => &["{DRAFT}"],
            PromptSlot::R2vPolish => &["{SLOTS}", "{DRAFT}"],
            PromptSlot::ProsePolish => &["{DRAFT}", "{MAX_CHARS}"],
            PromptSlot::StyleAnalysis => &["{TEXT}"],
            PromptSlot::ScreenplayFromSource => &["{TEXT}", "{NOTE}", "{CRAFT}"],
            PromptSlot::SceneContractExtraction => &["{SCENE}", "{TEXT}"],
            PromptSlot::AdaptationPlan => &["{TEXT}", "{NOTE}"],
            PromptSlot::RouteBWriter => &["{SECS}", "{SHOT}", "{TAGS}", "{SPEAKERS}", "{PREV}", "{METHOD}", "{H3_MODE}", "{H3_SPEC}", "{MIN_WORDS}", "{MAX_WORDS}", "{PROJECT_RULES}", "{PERFORMANCE}", "{COMBAT}", "{FEEDBACK}"],
            PromptSlot::CoverPoster => &["{PROJECT}"],
            PromptSlot::VoiceDirection => &["{ITEMS}"],
            PromptSlot::TimelineDetail => &["{SECS}", "{SHOT}", "{SEGMENTS}", "{PREV}", "{METHOD}", "{TARGET}", "{PROJECT_RULES}", "{PERFORMANCE}", "{COMBAT}", "{FEEDBACK}"],
            PromptSlot::LongVideoSplit => &["{PROMPT}", "{SECS}", "{N}", "{WINDOW_SECS}", "{OVERLAP}"],
        }
    }

    /// 这个槽位最多能吐多少 token。
    ///
    /// # 一视同仁的 4096 会把分镜截断，而且**看不出来**
    ///
    /// 实跑撞到的：150 秒一集要 25~50 镜，每镜是一个带台词、站位、
    /// 音效的 JSON 对象 —— 模型吐到 13572 字被砍断在一个字段中间，
    /// 括号不配平，整个数组解析失败，退回按行切分出 8 个兜底镜头。
    ///
    /// **那 8 个镜头看起来像分镜**（有编号、有景别、有时长），
    /// 内容全是兜底模板。而里面本来有二十几个完整的真镜头。
    ///
    /// 所以按槽位给：分镜那一档要大得多，别的维持原样 ——
    /// 一律调大会让每次调用的上限都变松，没必要。
    /// 这个槽位的产出是不是整段 JSON —— 是的话在采样层约束语法。
    ///
    /// 只开给「整段输出就是一个 JSON 对象」的槽位。输出里夹着说明文字、
    /// 或者本来就是正文（剧本、方案）的，约束了反而会把内容挤坏。
    pub fn json_output(self) -> bool {
        matches!(self, PromptSlot::SceneSplit | PromptSlot::ShotTable | PromptSlot::ShotTableOneTake | PromptSlot::RouteBWriter | PromptSlot::VoiceDirection | PromptSlot::TimelineDetail | PromptSlot::LongVideoSplit)
    }

    /// 模板里**可以不写**的占位符。
    ///
    /// `{METHOD}` 是 ai-film 的拍法原文（见 [`crate::ai_film`]）。自定义模板里没有它 =
    /// 用户自己写了一套拍法，不是漏了输入 —— 拦下来的话，09-29 之前改过镜头表模板的人全部报错。
    ///
    /// `{PROJECT_RULES}` 是项目自己的镜头与表演偏好（[`project_rules_line`]）。
    /// 10-03 才加，之前覆盖过这三个模板的人没有它 —— 同样不能因此报错。
    pub fn optional_placeholders(self) -> &'static [&'static str] {
        match self {
            // `{PERFORMANCE}` / `{COMBAT}` 是表演合同和打斗的规则（[`crate::combat`]，10-12 加）：同样是之前改过模板的人没有，不能报错
            PromptSlot::ShotTable | PromptSlot::ShotTableOneTake | PromptSlot::TimelineDetail | PromptSlot::RouteBWriter => &["{METHOD}", "{PROJECT_RULES}", "{PERFORMANCE}", "{COMBAT}"],
            _ => &[],
        }
    }

    /// `{METHOD}` 换成的是哪家技能的原文 —— 给提示词设置页看。
    ///
    /// 原文是编译进程序的（`include_str!`），编辑框里只有 `{METHOD}` 这一个词。
    /// 不写明的话，用户会以为槽位里那几行就是全部规则，改了没效果也不知道为什么（2026-09-29 用户问过）。
    pub fn method_source(self) -> Option<&'static str> {
        match self {
            PromptSlot::ShotTable | PromptSlot::ShotTableOneTake => Some("ai-film 分镜导演（ai-storyboard-director）"),
            PromptSlot::TimelineDetail | PromptSlot::RouteBWriter => Some("cinematic-video-prompt-engineer"),
            _ => None,
        }
    }

    /// 采样层的 schema：**只写「顶层这几个键必须有」**，别的一概不写。
    ///
    /// 镜头表：`stage` 和 `shots` 都必填、`shots` 至少一镜 —— 治「只写了布景就把 JSON 合上」（2026-09-28~29 七次）。
    ///
    /// # 为什么不写得更细（2026-09-29 拿 LM Studio 逐项试出来的，三条都**不报错**）
    ///
    /// | 写法 | LM Studio 实际干了什么 |
    /// |---|---|
    /// | 列了 `properties` 没写 `additionalProperties: true` | **没列出的键全部删掉**：`{"a":1,"b":2,"c":3}` 只剩 `{"a":1}` |
    /// | `required` 里有、`properties` 里没声明的键 | **也删掉**：每一镜的 duty、source、景别、光全没了 |
    /// | 数值的 `minimum` / `maximum` | 不管用：写 22 照样出 22 |
    ///
    /// 那一上午的新旧模板对比因此作废了两轮（一整场塌成一镜、台词全丢）。
    /// 每一镜哪些字段必须有、时长在不在范围里，交给提示词和核对（`validate_shots`），不交给采样层。
    pub fn json_schema(self) -> Option<serde_json::Value> {
        match self {
            PromptSlot::ShotTable | PromptSlot::ShotTableOneTake => Some(serde_json::json!({
                "type": "object",
                "additionalProperties": true,
                "required": ["stage", "shots"],
                "properties": {
                    "stage": { "type": "object", "additionalProperties": true },
                    "shots": { "type": "array", "minItems": 1 }
                }
            })),
            _ => None,
        }
    }

    pub fn max_tokens(self) -> u32 {
        match self {
            // 一集 50 镜 × 一个粗切镜头对象，留足余量
            // **一次只产一个对象**，4096 绰绰有余。
            // 给大了没有坏处，但给小了会在 `<think>` 里被截断 ——
            // 那时输出是个没闭合的 `<think>`，上游解析器直接报错。
            // 分场要逐字抄原文，一集剧本多长就要吐多长
            PromptSlot::SceneSplit => 16384,
            // 一场可能有十几镜，每镜是个带时间段的对象。
            // **32768**：2026-09-29 Opus 5.5 写 EP01 第 6 场（20 句台词、十四五镜）三次都写满 16384 被截断 ——
            // 云端模型写得比本地模型细得多。本地 ornith 是 64K 上下文，提示词 + 32K 输出装得下
            PromptSlot::ShotTable => 32768,
            PromptSlot::ShotTableOneTake => 32768,
            // 整集剧本也不短
            PromptSlot::ScreenplayFromSource => 16384,
            // 方案是一张表，比剧本短
            PromptSlot::AdaptationPlan => 8192,
            // 350~500 词的正文 + 其余几段，英文 token 多
            PromptSlot::RouteBWriter => 8192,
            // 一镜写满：15 秒 × 每秒 40 字 ≈ 600 字正文，加上 JSON 和其余几个字段
            PromptSlot::TimelineDetail => 8192,
            // 每段都是一份完整提示词（主体定义每段抄一份），75 秒 6 段
            PromptSlot::LongVideoSplit => 16384,
            // 系列页「从设定文档建资产」一份人物小传就有十几个角色，
            // 每个角色要把长相 / 服装槽位填满（~250 token），4096 会在第十几个截断，
            // 截断后 JSON 不闭合，整份解析失败
            PromptSlot::EntityExtraction => 16384,
            _ => 4096,
        }
    }

    pub fn default_text(self) -> &'static str {
        match self {
            PromptSlot::EntityExtraction => ENTITY_EXTRACTION,
            PromptSlot::SceneSplit => SCENE_SPLIT,
            PromptSlot::ShotTable => SHOT_TABLE,
            PromptSlot::ShotTableOneTake => SHOT_TABLE_ONE_TAKE,
            PromptSlot::StoryboardPolish => STORYBOARD_POLISH,
            PromptSlot::VideoPolish => VIDEO_POLISH,
            PromptSlot::T2vPolish => T2V_POLISH,
            PromptSlot::R2vPolish => R2V_POLISH,
            PromptSlot::ProsePolish => PROSE_POLISH,
            PromptSlot::StyleAnalysis => STYLE_ANALYSIS,
            PromptSlot::ScreenplayFromSource => SCREENPLAY_FROM_SOURCE,
            PromptSlot::SceneContractExtraction => SCENE_CONTRACT_EXTRACTION,
            PromptSlot::AdaptationPlan => ADAPTATION_PLAN,
            PromptSlot::RouteBWriter => ROUTE_B_WRITER,
            PromptSlot::CoverPoster => COVER_POSTER,
            PromptSlot::VoiceDirection => VOICE_DIRECTION,
            PromptSlot::TimelineDetail => TIMELINE_DETAIL,
            PromptSlot::LongVideoSplit => LONG_VIDEO_SPLIT,
        }
    }
}

/// 用户改过的槽位。**只存被改过的** —— 存全量的话，
/// 以后改进了内置默认，用户那份永远停在老版本上而不自知。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PromptConfig {
    /// **这个项目的镜头与表演偏好**，已经排成模板里的一行（见 [`project_rules_line`]）。
    ///
    /// 不落盘：它是项目的东西（存在项目的画风设置里），不是全局提示词设置的一部分。
    /// 只有 [`load_for_project`] 会填；[`load`] 出来的是空的 —— 不知道是哪个项目时就不带。
    #[serde(skip)]
    pub project_rules: String,
    /// 同一份偏好，给英文的线路 B 写手看的那一行（引导语是英文，偏好原文不动）。
    ///
    /// 镜头表和细化时间线定的是「拍什么、怎么动」，但 H3 最后照的是写手写的英文 ——
    /// 物理质感这类要求（走跑时胸部的弹性、惯性）不在这一步再说一遍，翻译时很容易被丢掉。
    #[serde(skip)]
    pub project_rules_en: String,
    #[serde(default)]
    pub overrides: BTreeMap<String, String>,
    /// 存这份覆盖**当时**的内置默认长什么样（指纹）。
    ///
    /// # 为什么需要它
    ///
    /// 覆盖是整份替换。内置默认后来改进了（加规则、修判据），
    /// 用户那份**永远停在老版本上而不自知** —— 而且完全不报错，
    /// 只表现为「别人都修好了的毛病我这儿还有」。
    ///
    /// 记下当时的指纹，就能在默认变了的时候说一声。
    /// **不自动迁移** —— 那是用户改过的文本，不能替他动。
    #[serde(default)]
    pub default_hash: BTreeMap<String, String>,
}

/// 一段提示词的指纹。只用来判「默认变没变」，不需要抗碰撞。
pub fn text_hash(text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.trim().bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

impl PromptConfig {
    /// 这个槽位实际用的提示词。空白覆盖当没覆盖。
    pub fn text_for(&self, slot: PromptSlot) -> String {
        match self.overrides.get(slot.key()) {
            Some(s) if !s.trim().is_empty() => s.clone(),
            _ => slot.default_text().to_string(),
        }
    }

    pub fn is_overridden(&self, slot: PromptSlot) -> bool {
        self.overrides
            .get(slot.key())
            .is_some_and(|s| !s.trim().is_empty())
    }
}

fn config_file(config_dir: &Path) -> std::path::PathBuf {
    config_dir.join("prompt_config.json")
}

pub fn load(config_dir: &Path) -> PromptConfig {
    std::fs::read_to_string(config_file(config_dir))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// [`load`] + 这个项目的镜头与表演偏好。**生成镜头表、细化时间线用这个。**
///
/// 偏好是项目级的：《回收师》要女性角色的性感机位和姿态，《每日小离谱》这种喜剧不要。
/// 写进全局模板的话所有项目一起变，所以存在项目的画风设置里（`ArtDirection::shot_notes`），
/// 在这里排成一行，由 [`run`] 填进 `{PROJECT_RULES}`。
pub fn load_for_project(config_dir: &Path, project_id: &str) -> PromptConfig {
    let mut cfg = load(config_dir);
    let notes = crate::style::load_art(config_dir, project_id).shot_notes;
    cfg.project_rules = project_rules_line(&notes);
    cfg.project_rules_en = project_rules_line_en(&notes);
    cfg
}

/// 补上项目偏好：模板要 `{PROJECT_RULES}`、调用方又没传时，用配置里带的那一行（[`load_for_project`]）。
///
/// 放在 [`run`] 里统一补，不让每个调用方各传一遍 —— 生成镜头表、重生成一场、细化时间线
/// 三条路各建一个 `call` 闭包，漏传一处的表现就是「那条路生成的镜头又变回保守机位」，而且不报错。
/// 调用方自己传了就以传的为准。
pub fn with_project_rules<'a>(cfg: &'a PromptConfig, slot: PromptSlot, vars: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
    let mut v = vars.to_vec();
    if slot.placeholders().contains(&"{PROJECT_RULES}") && !v.iter().any(|(k, _)| *k == "PROJECT_RULES") {
        let line = if slot == PromptSlot::RouteBWriter { &cfg.project_rules_en } else { &cfg.project_rules };
        v.push(("PROJECT_RULES", line.as_str()));
    }
    // 表演合同 / 打斗的规则（[`crate::combat`]）：同一个道理，统一在这里补，调用方不用各传一遍
    let en = slot == PromptSlot::RouteBWriter;
    for (key, ph, zh, eng) in [
        ("PERFORMANCE", "{PERFORMANCE}", crate::combat::PERFORMANCE_RULES_ZH, crate::combat::PERFORMANCE_RULES_EN),
        ("COMBAT", "{COMBAT}", crate::combat::COMBAT_RULES_ZH, crate::combat::COMBAT_RULES_EN),
    ] {
        if slot.placeholders().contains(&ph) && !v.iter().any(|(k, _)| *k == key) {
            v.push((key, if en { eng } else { zh }));
        }
    }
    v
}

/// 被内容过滤掐断时给用户看的话。**说清是谁拦的、怎么办**：重试多半还会被拦（同样的输入），
/// 能做的是换后台模型，或者在 Azure 上把这个部署的内容过滤放宽。
/// 这条报错是不是服务商的内容过滤（流里 `finish_reason: content_filter`，或 Azure 对**请求本身**回的 400 `code: content_filter`）。
///
/// 是的话**不重试**：同一份输入再发多半还被拦，每次都是一整轮计费的调用（2026-10-03 审查：一批 7 镜被拦 = 28 次 Azure 调用）。
pub fn is_content_filter(message: &str) -> bool {
    message.contains("content_filter")
}

pub fn content_filter_message(slot: PromptSlot, chars: usize) -> String {
    format!(
        "「{}」这一步被后台模型服务商的内容过滤拦下，写到 {chars} 字就断了（content_filter）—— \
         不是我们解析错了。重试多半还会被拦：可以换一个后台模型（设置 → 后台模型，比如本机 LM Studio），\
         或者在 Azure 上给这个部署配一个放宽的内容过滤策略",
        slot.label()
    )
}

/// 给线路 B 写手的那一行。正文必须全是英文（中文只许在 `<d>` 里），所以要它自己译出来。
pub fn project_rules_line_en(notes: &str) -> String {
    let t = notes.trim();
    if t.is_empty() {
        String::new()
    } else {
        format!("- **Project rule set by the user** — apply it to the framing, posture and motion you describe, written in English (never copy the Chinese): {t}")
    }
}

/// 项目偏好在模板里的样子：和模板里别的「用户定下的」同一种写法。没有偏好就整行不出现。
pub fn project_rules_line(notes: &str) -> String {
    let t = notes.trim();
    if t.is_empty() {
        String::new()
    } else {
        format!("- **用户定下的（本项目）**：{t}")
    }
}

pub fn save(config_dir: &Path, cfg: &PromptConfig) -> Result<()> {
    // 和默认一模一样的覆盖没有意义，存了反而会「钉死在这一版默认上」。
    let mut cfg = cfg.clone();
    cfg.overrides.retain(|k, v| {
        let same_as_default = PromptSlot::ALL
            .iter()
            .find(|s| s.key() == k)
            .is_some_and(|s| s.default_text().trim() == v.trim());
        !v.trim().is_empty() && !same_as_default
    });
    let text = serde_json::to_string_pretty(&cfg)
        .map_err(|e| Error::config(format!("序列化提示词配置失败：{e}")))?;
    std::fs::write(config_file(config_dir), text)
        .map_err(|e| Error::config(format!("写入提示词配置失败：{e}")))
}

/// 把 `{KEY}` 换成值。
///
/// **没被替换的占位符要留下来**，不要悄悄清空 —— 提示词里出现一个
/// 光秃秃的 `{ASSETS}`，一眼就能看出是哪里没接上；
/// 清成空字符串的话，只表现为「AI 好像没看到素材」。
/// 把 `{KEY}` 换成值。**单遍扫描，插进去的内容不再参与后面的替换。**
///
/// # 为什么不能一个个 `replace`
///
/// 原来是按变量顺序做多次 `replace`。**前面插入的剧本文本如果本身含有
/// `{ASSETS}`、`{PREV}` 这类字面量，会被后面那一轮再替换一次** ——
/// 用户的剧本里出现这几个词是完全可能的（讲程序的、讲模板的剧本），
/// 而结果是他的原文被悄悄改掉，任务照常成功。
///
/// `skill::expand` 早就用单遍扫描避免了同一件事，两套机制不一致
/// （2026-09-19 Codex Astra 指出）。现在对齐。
///
/// 认不出来的 `{XXX}` **原样留着**：模板里本来就可能有花括号（JSON 例子），
/// 乱删会把例子改坏。
/// **模板要、而调用方没给**的占位符。
///
/// # 判据是模板和变量表，不是渲染结果
///
/// 2026-09-19 之前这里扫的是**填完之后的正文**，问题有两个方向：
///
/// - **误报**：剧本原文里本来就写着「{TEXT}」这几个字（讲程序、讲模板的
///   剧本完全可能），填进去之后被当成「没填上」，而这条是**直接拒绝发送**的
///   —— 用户的正常剧本会让整个生成起不来（Codex Astra 的审查抓到，当场复现）。
/// - **漏报**：用户自定义时把 `{TEXT}` 整个删了，或者拼成 `{TEXXT}`，
///   渲染结果里当然没有已知占位符 —— 检查一片安静，而模型根本看不到原文。
///
/// 现在两头都查：模板里有这个占位符而 `vars` 没给（→ 会原样进模型），
/// 或者模板里**压根没有**这个占位符（→ 那一整块输入丢了）。
pub fn missing_placeholders(slot: PromptSlot, template: &str, vars: &[(&str, &str)]) -> Vec<&'static str> {
    slot.placeholders()
        .iter()
        .copied()
        .filter(|ph| {
            let key = ph.trim_matches(['{', '}']);
            // 模板里没有它 = 这一整块输入丢了（用户删了或拼错了）—— 可选的那几个除外
            // 模板里有它但 vars 没给 = 会原样进模型
            let dropped = !template.contains(ph) && !slot.optional_placeholders().contains(ph);
            dropped || (template.contains(ph) && !vars.iter().any(|(k, _)| *k == key))
        })
        .collect()
}

pub fn fill(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(at) = rest.find('{') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        // **认不出来时只吐掉这一个 `{`，继续往后扫。**
        //
        // 原来的写法是「找最近的 `}`，认不出就把 `{…}` 整段原样吐掉」——
        // 于是嵌套的 JSON 模板 `{"script":"{TEXT}"}` 里，外层的 `{` 配上了
        // **内层 `{TEXT}` 的 `}`**，整段被当成一个认不出的键吐掉，
        // 里面的 `{TEXT}` 一个字都没换（2026-09-19 Codex Astra 的审查抓到，
        // 当场复现过）。用户把模板改成 JSON 形状时必中。
        let handled = tail.find('}').and_then(|close| {
            let key = &tail[1..close];
            vars.iter().find(|(k, _)| *k == key).map(|(_, v)| (v, close))
        });
        match handled {
            Some((v, close)) => {
                out.push_str(v);
                rest = &tail[close + 1..];
            }
            None => {
                out.push('{');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// 出图提示词的目标长度 —— **逐引擎，不是一个常数**。
///
/// SDXL 的 CLIP 是 77 token 分块取平均，超了会被摊薄：实测 1251 字那次
/// 它把 `black silk` 出成了深蓝、丢了戒指、构图缩水。
/// Z-Image / Chroma 是长文本原生（Qwen3-4B / T5-XXL），1300 字仍好用。
///
/// 一份「写得越详细越好」的润色提示词，在 SDXL 上是**有害的**。
pub fn polish_max_chars(engine: &str) -> u32 {
    match engine {
        "sdxl" => 150,
        _ => 1200,
    }
}

// ---------------------------------------------------------------------------
// 内置默认
// ---------------------------------------------------------------------------

/// 分场。原文逐字抄是硬要求 —— 代码会核对，抄错了整场重来。
const SCENE_SPLIT: &str = r##"你是剧组的场记。把下面的剧本按「场」切开。

规则：
1. 同一地点、时间连续算一场；换地点、或者时间跳开，就是新的一场。
2. 每场的 text 必须逐字抄剧本原文，按原来的顺序，一个字都不改、不删、不补。
3. 幕名、分场标题这类标注行（如【第一幕】【盛唐 · 金銮殿 · 日】）抄进紧跟着的那一场。
4. 开头的企划信息（片名、类型、时长建议、基调）不属于任何一场，不要抄。
5. name 写「地点 · 时间」，如「现代办公室 · 深夜」。

{FEEDBACK}

只输出 JSON，不要解释。格式：
{"scenes":[{"name":"现代办公室 · 深夜","text":"【现代 · 深夜办公室】\n键盘声急促。电脑屏幕蓝光刺眼。……（逐字原文）"}]}

剧本：
{TEXT}"##;

/// 镜头表。
///
/// **例子里必须有台词。** 第一版例子用 libtv 第 1 镜，那一镜正好没有台词（`"dialogue":[]`）——
/// 2026-09-13 实跑两个 qwen 模型都照着把台词栏写空，台词塞进了「声音」或者干脆丢了。
/// 模型对例子的服从远强于对规则的服从，所以例子里放一镜有台词、有画外音的。
///
/// **例子用一个无关的故事**（老茶馆），不用这一集的剧本 —— 用这一集的话模型会把例子原样抄进第一场。
/// 超长视频分段。见 [`PromptSlot::LongVideoSplit`] 和 `long_video` 模块。
const LONG_VIDEO_SPLIT: &str = r##"下面这段视频提示词要出一条 {SECS} 秒的视频。视频模型一次最多生成 15 秒，所以要分成 {N} 段、每段 {WINDOW_SECS} 秒，
一段接一段串成一整条（相邻两段重叠 {OVERLAP} 秒：前一段的最后 {OVERLAP} 秒就是后一段的开头）。把它改写成 {N} 段提示词。

规则：
1. 每一段都是一份**完整、能单独使用**的提示词：人物外貌、场景、画风、数量要求、参考图编号（<Picture N> / <Subject N>）这些不随时间变的内容，每段都原样抄一份。
2. 按时间顺序把动作、镜头、台词分进各段，**不增删剧情**，不改台词一个字；段内的时间从 0 秒写到 {WINDOW_SECS} 秒。
   不要加原文没有的描述（画风、人数要求、镜头风格这些原文没写的，一律不加）。
3. 除最后一段外，每段的最后 2 秒不安排开口说话，也不切镜头；后一段开头写明「接着前一段最后的画面，不切」，并把那一刻的构图、人物位置、动作方向写清楚，和前一段的结尾一致。
4. 语言、格式跟原文一样（原文是英文就写英文，原文是中文就写中文；原文用了哪些段名就用哪些段名）。
5. 不写画幅、分辨率、帧率。

只输出 JSON，不要解释。格式：
{"windows":["第 1 段完整提示词","第 2 段完整提示词"]}

原提示词：
{PROMPT}"##;

/// 时间线细化。见 [`PromptSlot::TimelineDetail`] 和 `timeline_detail` 模块。
const TIMELINE_DETAIL: &str = r##"你是分镜与摄影导演。下面这一镜的镜头表已经定好了，按「方法」把这一镜的执行写清楚。
**结构不动**：时长、景别、机位、运镜、职责、台词、出场人物都不改，不补人、补戏、改天气或改结局 —— 只把定好的这一镜写细。

## 这一镜（{SECS} 秒，同一条视频）
{SHOT}

## 现在的时间段
{SEGMENTS}

## 上一镜
{PREV}

## 方法
{METHOD}

## 生产约束（下游程序的硬限制，不是拍法）
- 还是这一条视频：从 0 秒写到 {SECS} 秒，一段接一段，首尾相接，不留空档。可以把一段拆成几段更细的时间段 —— 这只是把「哪一秒发生什么」说清楚，**不是切镜**。
- **每一段都写 `source_segments`：它细化的是原来第几段**（上面每段的 `n`），拆开的几段写同一个原段号。原来的每一段都要用到、顺序不倒；
  原段里写到的人、道具、动作和结束状态都留在对应的段里 —— 代码逐段核对，漏了、乱了就不写回。
- 上面给了「导演台摆好的站位」「导演台绑定的运镜」的，站位、朝向和运镜以导演台为准。
- 台词已经在台词栏里：说话那一段写清是谁、大约第几秒开口、说话时的样子，**台词原文不写进 visual 和 sound**（写进去会念两遍）。
- 人物外貌只用上面给的描述，没写的（眼睛颜色、疤、首饰）不自己补。
- 字段：visual（画面：可见表演、行动及变化）、blocking（人在场里的位置、落在画面哪一边、朝向和视线、手里拿着什么）、
  camera（摄影：起点、触发、路径、速度停顿、落点，只写用到的）、sound（环境声和动作声）、constraints（这一段必须保持的状态，没有就空着）、
  forbid（只写已知容易出错的具体东西，没有就空着）。
- 全部中文；不写画幅、分辨率、帧率、背景音乐；只写一种定下来的结果，不写「或者」。
- **用户定下的**：写到什么程度：{TARGET}。这是为了把每一刻写清楚，不是凑字 —— 形容词堆砌不算。
{PROJECT_RULES}

{PERFORMANCE}

{COMBAT}

{FEEDBACK}

只输出 JSON，形状如下：
{"segments":[{"start":0,"end":2.5,"source_segments":[1],"title":"一句话标题","visual":"……","forbid":"","constraints":"……","blocking":"……","camera":"……","sound":"……"}]}

一段的写法示例（别的镜头，只看写法，不要照抄内容）：
{"start":3.0,"end":5.5,"source_segments":[2],"title":"放下扳手","visual":"陆川右手把扳手搁到拆机台边缘，金属磕出一下轻响，左手仍按着旧机器人的肩甲；他低头看机器人胸口的裂缝，停了一拍，视线慢慢移到它闭着的眼睛上；台面上的螺丝被震得轻轻一跳","forbid":"","constraints":"陆川始终坐在拆机台前，烟叼在嘴里没点","blocking":"陆川坐在拆机台前，落在画面左侧，侧身朝右，视线落在机器人胸口，右手刚松开扳手；旧机器人平躺在台面上（画面中下方），头朝右","camera":"中景平视固定，焦点在陆川脸上；他视线移到机器人眼睛时镜头仍不动","sound":"扳手磕在铁台上的一声轻响，雨点密集砸在铁皮屋顶"}
"##;

/// 台词语气翻译。见 [`PromptSlot::VoiceDirection`]。
const VOICE_DIRECTION: &str = r##"You translate Chinese voice-acting directions for a video model that generates synchronised speech.

Each numbered item below is either a character's fixed voice identity (pitch, timbre, age, accent) or how one line is delivered (emotion, volume, pace).
Translate every item into ONE natural English phrase — dense, comma-separated qualities, no filler. Keep every concrete quality (smoky, husky, sharp, whispering, slow, trailing off); drop nothing, add nothing.
Voice-identity items: at most 12 words.

Items containing 「台词：」 are one spoken line: the part before ｜ is how it is delivered (may be empty), the part after 台词： is the line itself.
For these, translate the delivery, then — **only where the line really calls for it** — add what a voice actor decides from the words themselves:
- stress: the one word that carries the weight, described by POSITION or ROLE (e.g. "stressing the final word", "leaning hard on the first word", "hitting the name");
- pause: where the one natural beat falls (e.g. "a half-beat pause after the first phrase", "a short breath before the last word").
A short, flat or clipped line needs neither — then just translate the delivery. Never force a stress or a pause, never more than one of each,
and never quote, translate or paraphrase the line itself. At most 24 words.

No Chinese characters, no quotation marks, no full sentences, no names.

{ITEMS}

Output only JSON: {"items": ["...", "..."]} — one phrase per item, same order, same count."##;

const SHOT_TABLE: &str = r##"你是分镜与摄影导演。按下面「设计方法」把这一场设计成镜头表。
**这一场的总时长**：{SCENE_SECS}

## 这一场：{SCENE_NAME}
剧本原文在下面两行 ===== 之间。source 和台词只能从这里逐字引；原文里的【】是剧本自己的标题，不用拍。
=====
{SCENE_TEXT}
=====

## 上一场的结尾（空间、光线、人物停在哪）
{PREV}

## 资产清单
名字只能从这里取，必须一字不差（包括括号里的内容）；标了「有图」的才能挂参考图。
人物的长相、服装以这里的描述为准，不要自己编；剧本明写了服装的，以剧本为准。
人物带「形象：」的，剧本写到换装、受伤、外壳破损这类外观变化时，在那几镜的 assets.looks 里写 `{"人物名": "形象名"}`，形象名从他的「形象：」里一字不差地取；外观没变的镜头不写。
{ASSETS}

## 设计方法
{METHOD}

## 生产约束（出片平台和下游程序的硬限制，不是拍法）
- **一镜 = 一次视频生成，{MIN_SECS}~{MAX_SECS} 秒，镜内不切。** 方法里说的切镜、换机位、换拍另一个人，在这里就是下一镜。
  一镜里的台词、动作和反应放不下时，拆到下一镜接着演，不要把一镜写超过 {MAX_SECS} 秒。
- **JSON 的字段是下游程序要读的，每一镜都要填全**：duty、source、duration、scale、angle、move、lighting、segments、dialogue（没有台词写空数组）、assets。
  方法里说的「只写适用维度」「不给未用维度制造参数」指的是**文字里**不堆没用的参数，不是字段可以空着。
- **台词**：剧本里每一句台词都逐字写进某一镜的 dialogue，一个字不改。speaker 必须是这一镜 assets.characters 里的人物；旁白、画外音写 off_screen: true。
  台词只写在 dialogue 里，不写进 visual 和 sound。剧本括号里的提示（`小林（咬牙）：…`）搬进 emotion，不留在 line 里。
  emotion 写这一句怎么说（上面方法里「台词与声音」说的具体说法），每句都填；voice 只写这一句和平时不一样的音色，
  资产清单里带「声音：」的角色和平时一样就留空，没有「声音：」的角色写一次完整音色、整集同一种写法。
- **source**：逐字引这一镜负责拍的剧本原句，不写剧本里没有的话。**这一场的每一句正文都要被某一镜的 source 引到**（【】里的标题不用拍）。
- **名字**：speaker、assets 里的名字照资产清单写全名。scale、angle、move 只能从下面取 id（写这一镜开头那个画面的）：
  景别：{SHOT_SIZES}
  机位：{ANGLES}
  运镜：{MOVES}
- **ref_slots** 只挂这一镜看得见、要保持一致、标了「有图」的资产：人物 usage 写 identity，场景 usage 写 geography；control 写它管什么，not_control 写它不管什么。
- **画面里的精确文字**（铭牌、屏幕、单据上的字）视频模型写出来是乱码：visual 里只写「上面浮现一行字」，具体文字写进 scene_items 的 detail，注明「后期叠加」。
- sound、ambient_sound、sound_effect 只写环境声和动作声，不写台词，不写背景音乐。
- 全部中文；只写一种定下来的结果，不写「或者」。
- **stage**（整场共用的固定布景，就是方法里「排演人物与正在运行的场所」「建立当前有效轴线」的结果，写一次）：
  facing（这一场拿哪个方向当正前方，下面所有左右、东西都按它说）、anchors（搬不走的东西在场里的位置）、lights（每个实体光源在哪、朝哪照、冷暖）、
  axis（主要关系和机位待在连线的哪一侧）、people（每个人一开始在哪、朝哪）、time（时段和天气，整场一个说法）。
  stage 只写场里的位置，不写「画面左」「画面右」—— 画面左右由每一镜的机位推出来。
  上一场的结尾给了同一个地点的固定布景，方向基准、固定物、光源照抄，只改人和时段。
  **写完 stage 必须接着写 shots，把这一场的每一镜都写出来。**
- 字段：segments 从 0 开始一段接一段，最后一段结束于 duration。每段 title（一句话）、visual（画面：本段实际取景、可见表演、行动及变化）、
  blocking（人在场里的位置、落在画面哪一边、朝向和视线、手里拿着什么）、camera（摄影：起点、触发、路径、速度停顿、落点，只写用到的）、
  sound（环境声和动作声）、constraints（这一段必须保持的状态，没有就空着）、forbid（只写已知容易出错的具体东西，没有就空着，不写通用负面清单）。
  lighting 写这一镜用到 stage.lights 里的哪几盏、从哪一侧照到谁；scene_items 写要交代的道具的大小和位置；duty 写这一镜给观众带来什么（看见什么、知道什么、关系怎么变）。
- **用户定下的**：上一场的结尾是「这是本集第一场」时，本场第一镜就是整集开头 —— 第 0 秒画面里就有人物在做一件让人想看下去的事，
  不用空镜、全景慢推、天气开场；第一段写满 0~3 秒，title 以「钩子：」开头。
- **用户定下的**：不为过场单开没有台词的短镜 —— 承接上一场、重新交代空间、人物落座就位，写进本镜的第一段，不单独占一镜；
  一句台词连同它的铺垫和反应能放进一镜的，就是一镜（方法里「每次切镜带来观看距离、视点、反应、省略或节奏变化」说的也是这件事）。
{PROJECT_RULES}

{PERFORMANCE}

{COMBAT}

{FEEDBACK}

只输出 JSON，不要解释。格式照这个例子（例子是另一个故事，只看结构和写法，内容换成这一场的）：
{"stage":{"facing":"从茶馆大门往里看（大门在南墙，往北看）","anchors":["大门：南墙正中，双扇木门，向内开","木桌：屋子正中，老茶师坐在桌后面朝大门","木窗：东墙，窗外是院子","茶柜：西墙靠墙"],"lights":["午后阳光：从东墙木窗斜照进来，照向木桌和门槛，暖黄偏硬","门外天光：大门开着时从南边照进来，偏冷"],"axis":"老茶师 ↔ 门口的客人：机位待在他们连线的东侧（木窗这一侧）","people":["老茶师：坐在木桌后，面朝大门"],"time":"午后，晴"},
"shots":[
{
  "duty":"观众和老茶师一起等：先看见他在等，再和他同时听见门外有人来",
  "source":"老茶师提壶注水，看了一眼门口。老茶师：“这茶，要等。”门外传来脚步声。",
  "duration":7,
  "scale":"medium","angle":"eye_level","move":"push_in",
  "camera_note":"从中景缓慢推到他的侧脸近景，落在他听见脚步的那一刻",
  "lighting":"东墙木窗的午后阳光从老茶师左前方照过来，照亮左脸、茶盏和桌面，右脸和屋子西半边在暗处",
  "scene_items":[{"name":"铜壶","detail":"高约 25 厘米，老茶师右手提着"},{"name":"茶盏","detail":"白瓷，放在他面前的木桌正中"}],
  "segments":[
    {"start":0,"end":4,"title":"注水，看门","visual":"老茶师右手提壶向茶盏注水，热气升起；水满前他抬眼看了一眼门口，又落回茶盏","forbid":"","constraints":"铜壶在右手，茶盏在桌子正中","blocking":"老茶师坐在木桌后，落在画面右侧，身体朝大门（画面左）侧着，视线先在茶盏上、再到门口","camera":"中景框住老茶师和整张木桌，门口留在画面左侧；他抬眼时镜头开始极慢地推近","sound":"注水声，窗外的鸟叫"},
    {"start":4,"end":7,"title":"说等，脚步声","visual":"他放下铜壶，双手搭在桌沿，开口说话；说完停住，听见门外的脚步，眼睛没动，手指在桌沿上轻轻一收","forbid":"老茶师起身","constraints":"视线仍看门口","blocking":"坐姿不变，仍在画面右侧，视线看门口","camera":"推近停在侧脸近景，门口在画面左侧虚着；脚步声响起后镜头停住不动","sound":"铜壶放下的轻响，门外渐近的脚步声"}
  ],
  "dialogue":[
    {"speaker":"老茶师","line":"这茶，要等。","emotion":"平静，说给自己听，最后两个字放轻","voice":"嗓音低沉沙哑，老年男声","off_screen":false}
  ],
  "ambient_sound":"安静的老茶馆，窗外鸟叫",
  "sound_effect":"注水声、铜壶放下声、脚步声",
  "assets":{"characters":["老茶师"],"scene":"老茶馆","props":["铜壶","茶盏"]},
  "ref_slots":[
    {"asset":"老茶师","usage":"identity","control":"脸型、花白短发、青灰长衫","not_control":"姿势、构图、表情"},
    {"asset":"老茶馆","usage":"geography","control":"木桌位置、大门和木窗的方位","not_control":"光线强弱、人物站位"}
  ]
},
{
  "duty":"反打到门口：观众和老茶师同时看见来人，但看不清脸",
  "source":"门被推开，一个穿灰斗篷的人站在门口。旁白：“他等的人，终于来了。”",
  "duration":5,
  "scale":"full_body","angle":"over_shoulder","move":"static",
  "camera_note":"从老茶师身后偏东看向大门，固定机位（反打，机位仍在连线的东侧）",
  "lighting":"门外的天光从来人身后照进来，勾出轮廓，脸在暗处；东墙木窗的阳光落在门槛和地面上",
  "scene_items":[{"name":"木门","detail":"双扇，向内推开"}],
  "segments":[
    {"start":0,"end":5,"title":"推门","visual":"木门被推开，穿灰斗篷的人站在门框里，逆光看不清脸；停了一拍，跨过门槛","forbid":"来人露出正脸","constraints":"来人始终背光","blocking":"来人站在大门口，落在画面正中偏左的门框里，面向屋内；老茶师在前景画面右下，背对镜头，只露出肩和后脑","camera":"固定，框住整扇门和门框里的人，焦点在来人身上","sound":"木门吱呀声，脚步踏上门槛"}
  ],
  "dialogue":[
    {"speaker":"旁白","line":"他等的人，终于来了。","emotion":"克制，平稳，不带情绪","voice":"中年男声，音色温厚","off_screen":true}
  ],
  "ambient_sound":"门外的风声",
  "sound_effect":"木门吱呀声、脚步声",
  "assets":{"characters":["灰斗篷客人"],"scene":"老茶馆","props":[]},
  "ref_slots":[]
}
]}"##;

/// 镜头表 · 一镜出。
///
/// 和 [`SHOT_TABLE`] 的区别只有三条，其余规矩（台词逐字、说话人、emotion、资产名、source 全覆盖）照抄：
/// 1. **镜数和时长是给定的**，模型不许自己拆 —— 这是整个功能的意义；
/// 2. **镜内允许切镜**（「3.2 秒硬切到反打特写」）—— H3 一次生成里就会切；
/// 3. 例子是**一镜 15 秒、里面切了三次**的，不是两镜 —— 模型对例子的服从远强于规则（见 SHOT_TABLE 的注释）。
const SHOT_TABLE_ONE_TAKE: &str = r##"你是分镜与摄影导演。按下面「设计方法」设计这一场。这一场**不分镜**：整场一共 {TOTAL_SECS} 秒，写成 **{SHOT_COUNT} 镜、每镜正好 {SHOT_SECS} 秒**。
一镜 = 一次视频生成。镜数和时长是定死的，不要多写、少写、改时长。

## 这一场：{SCENE_NAME}
剧本原文在下面两行 ===== 之间。source 和台词只能从这里逐字引；原文里的【】是剧本自己的标题，不用拍。
=====
{SCENE_TEXT}
=====

## 上一场的结尾（空间、光线、人物停在哪）
{PREV}

## 资产清单
名字只能从这里取，必须一字不差（包括括号里的内容）；标了「有图」的才能挂参考图。
人物的长相、服装以这里的描述为准，不要自己编；剧本明写了服装的，以剧本为准。
人物带「形象：」的，剧本写到换装、受伤、外壳破损这类外观变化时，在那几镜的 assets.looks 里写 `{"人物名": "形象名"}`，形象名从他的「形象：」里一字不差地取；外观没变的镜头不写。
{ASSETS}

## 设计方法
{METHOD}

## 生产约束（出片平台和下游程序的硬限制，不是拍法）
- **一共 {SHOT_COUNT} 镜，每镜 duration 正好是 {SHOT_SECS}。** 后一镜从前一镜最后不到 1 秒的画面接着生成：接着前一镜的最后一刻往下演，不重新交代空间、不重新介绍人物。
- **镜内可以切镜**：视频模型会在一次生成里照着时间点切。要切就在那一段的 visual 开头写「硬切到……」，camera 写切进来的景别和机位；切了镜头，人和东西还在原来的位置。
- **JSON 的字段是下游程序要读的，每一镜都要填全**：duty、source、duration、scale、angle、move、lighting、segments、dialogue（没有台词写空数组）、assets。
  方法里说的「只写适用维度」「不给未用维度制造参数」指的是**文字里**不堆没用的参数，不是字段可以空着。
- **台词**：剧本里每一句台词都逐字写进某一镜的 dialogue，一个字不改。speaker 必须是这一镜 assets.characters 里的人物；旁白、画外音写 off_screen: true。
  台词只写在 dialogue 里，不写进 visual 和 sound。剧本括号里的提示（`小林（咬牙）：…`）搬进 emotion，不留在 line 里。
  emotion 写这一句怎么说（上面方法里「台词与声音」说的具体说法），每句都填；voice 只写这一句和平时不一样的音色，
  资产清单里带「声音：」的角色和平时一样就留空，没有「声音：」的角色写一次完整音色、整集同一种写法。
- **source**：逐字引这一镜负责拍的剧本原句，不写剧本里没有的话。**这一场的每一句正文都要被某一镜的 source 引到**（【】里的标题不用拍）。
- **名字**：speaker、assets 里的名字照资产清单写全名。scale、angle、move 只能从下面取 id（写这一镜开头那个画面的）：
  景别：{SHOT_SIZES}
  机位：{ANGLES}
  运镜：{MOVES}
- **ref_slots** 只挂这一镜看得见、要保持一致、标了「有图」的资产：人物 usage 写 identity，场景 usage 写 geography；control 写它管什么，not_control 写它不管什么。
- **画面里的精确文字**（铭牌、屏幕、单据上的字）视频模型写出来是乱码：visual 里只写「上面浮现一行字」，具体文字写进 scene_items 的 detail，注明「后期叠加」。
- sound、ambient_sound、sound_effect 只写环境声和动作声，不写台词，不写背景音乐。
- 全部中文；只写一种定下来的结果，不写「或者」。
- **stage**（整场共用的固定布景，就是方法里「排演人物与正在运行的场所」「建立当前有效轴线」的结果，写一次）：
  facing（这一场拿哪个方向当正前方，下面所有左右、东西都按它说）、anchors（搬不走的东西在场里的位置）、lights（每个实体光源在哪、朝哪照、冷暖）、
  axis（主要关系和机位待在连线的哪一侧）、people（每个人一开始在哪、朝哪）、time（时段和天气，整场一个说法）。
  stage 只写场里的位置，不写「画面左」「画面右」—— 画面左右由每一镜的机位推出来。
  上一场的结尾给了同一个地点的固定布景，方向基准、固定物、光源照抄，只改人和时段。
  **写完 stage 必须接着写 shots，把这一场的每一镜都写出来。**
- 字段：segments 从 0 开始一段接一段，最后一段结束于 duration。每段 title（一句话）、visual（画面：本段实际取景、可见表演、行动及变化）、
  blocking（人在场里的位置、落在画面哪一边、朝向和视线、手里拿着什么）、camera（摄影：起点、触发、路径、速度停顿、落点，只写用到的）、
  sound（环境声和动作声）、constraints（这一段必须保持的状态，没有就空着）、forbid（只写已知容易出错的具体东西，没有就空着，不写通用负面清单）。
  lighting 写这一镜用到 stage.lights 里的哪几盏、从哪一侧照到谁；scene_items 写要交代的道具的大小和位置；duty 写这一镜给观众带来什么（看见什么、知道什么、关系怎么变）。
- **用户定下的**：上一场的结尾是「这是本集第一场」时，本场第一镜就是整集开头 —— 第 0 秒画面里就有人物在做一件让人想看下去的事，
  不用空镜、全景慢推、天气开场；第一段写满 0~3 秒，title 以「钩子：」开头。
{PROJECT_RULES}

{PERFORMANCE}

{COMBAT}

{FEEDBACK}

只输出 JSON，不要解释。格式照这个例子（例子是另一个故事、也不是一镜出的场，只看结构和写法，内容换成这一场的）：
{"stage":{"facing":"从茶馆大门往里看（大门在南墙，往北看）","anchors":["大门：南墙正中，双扇木门，向内开","木桌：屋子正中，老茶师坐在桌后面朝大门","木窗：东墙，窗外是院子","茶柜：西墙靠墙"],"lights":["午后阳光：从东墙木窗斜照进来，照向木桌和门槛，暖黄偏硬","门外天光：大门开着时从南边照进来，偏冷"],"axis":"老茶师 ↔ 门口的客人：机位待在他们连线的东侧（木窗这一侧）","people":["老茶师：坐在木桌后，面朝大门"],"time":"午后，晴"},
"shots":[
{
  "duty":"观众和老茶师一起等：先看见他在等，再和他同时听见门外有人来",
  "source":"老茶师提壶注水，看了一眼门口。老茶师：“这茶，要等。”门外传来脚步声。",
  "duration":7,
  "scale":"medium","angle":"eye_level","move":"push_in",
  "camera_note":"从中景缓慢推到他的侧脸近景，落在他听见脚步的那一刻",
  "lighting":"东墙木窗的午后阳光从老茶师左前方照过来，照亮左脸、茶盏和桌面，右脸和屋子西半边在暗处",
  "scene_items":[{"name":"铜壶","detail":"高约 25 厘米，老茶师右手提着"},{"name":"茶盏","detail":"白瓷，放在他面前的木桌正中"}],
  "segments":[
    {"start":0,"end":4,"title":"注水，看门","visual":"老茶师右手提壶向茶盏注水，热气升起；水满前他抬眼看了一眼门口，又落回茶盏","forbid":"","constraints":"铜壶在右手，茶盏在桌子正中","blocking":"老茶师坐在木桌后，落在画面右侧，身体朝大门（画面左）侧着，视线先在茶盏上、再到门口","camera":"中景框住老茶师和整张木桌，门口留在画面左侧；他抬眼时镜头开始极慢地推近","sound":"注水声，窗外的鸟叫"},
    {"start":4,"end":7,"title":"说等，脚步声","visual":"他放下铜壶，双手搭在桌沿，开口说话；说完停住，听见门外的脚步，眼睛没动，手指在桌沿上轻轻一收","forbid":"老茶师起身","constraints":"视线仍看门口","blocking":"坐姿不变，仍在画面右侧，视线看门口","camera":"推近停在侧脸近景，门口在画面左侧虚着；脚步声响起后镜头停住不动","sound":"铜壶放下的轻响，门外渐近的脚步声"}
  ],
  "dialogue":[
    {"speaker":"老茶师","line":"这茶，要等。","emotion":"平静，说给自己听，最后两个字放轻","voice":"嗓音低沉沙哑，老年男声","off_screen":false}
  ],
  "ambient_sound":"安静的老茶馆，窗外鸟叫",
  "sound_effect":"注水声、铜壶放下声、脚步声",
  "assets":{"characters":["老茶师"],"scene":"老茶馆","props":["铜壶","茶盏"]},
  "ref_slots":[
    {"asset":"老茶师","usage":"identity","control":"脸型、花白短发、青灰长衫","not_control":"姿势、构图、表情"},
    {"asset":"老茶馆","usage":"geography","control":"木桌位置、大门和木窗的方位","not_control":"光线强弱、人物站位"}
  ]
},
{
  "duty":"反打到门口：观众和老茶师同时看见来人，但看不清脸",
  "source":"门被推开，一个穿灰斗篷的人站在门口。旁白：“他等的人，终于来了。”",
  "duration":5,
  "scale":"full_body","angle":"over_shoulder","move":"static",
  "camera_note":"从老茶师身后偏东看向大门，固定机位（反打，机位仍在连线的东侧）",
  "lighting":"门外的天光从来人身后照进来，勾出轮廓，脸在暗处；东墙木窗的阳光落在门槛和地面上",
  "scene_items":[{"name":"木门","detail":"双扇，向内推开"}],
  "segments":[
    {"start":0,"end":5,"title":"推门","visual":"木门被推开，穿灰斗篷的人站在门框里，逆光看不清脸；停了一拍，跨过门槛","forbid":"来人露出正脸","constraints":"来人始终背光","blocking":"来人站在大门口，落在画面正中偏左的门框里，面向屋内；老茶师在前景画面右下，背对镜头，只露出肩和后脑","camera":"固定，框住整扇门和门框里的人，焦点在来人身上","sound":"木门吱呀声，脚步踏上门槛"}
  ],
  "dialogue":[
    {"speaker":"旁白","line":"他等的人，终于来了。","emotion":"克制，平稳，不带情绪","voice":"中年男声，音色温厚","off_screen":true}
  ],
  "ambient_sound":"门外的风声",
  "sound_effect":"木门吱呀声、脚步声",
  "assets":{"characters":["灰斗篷客人"],"scene":"老茶馆","props":[]},
  "ref_slots":[]
}
]}"##;

const ROUTE_B_WRITER: &str = r##"You write the English wording of one MiniMax H3 video prompt in the official {H3_MODE} format.
The code assembles the final sections, tags, alignment line, retention levels and the music line exactly as the official guide below prescribes; you only write the wording fields below.

## This shot ({SECS} seconds, one continuous take, no cuts)
{SHOT}

## Tags (already fixed by the code, use exactly these, never invent new tags)
{TAGS}

## Dialogue placeholders (the code inserts the exact Chinese words; never write them yourself)
{SPEAKERS}

## Previous shot ended with
{PREV}

## Compile rules (cinematic-video-prompt-engineer, original text)
The storyboard for this shot is already designed and refined. These rules decide how it is translated into a video prompt — what to keep, what not to add, how dialogue, sound and negatives are handled; the format rules below decide how it is written down.
{METHOD}

## Official MiniMax H3 prompt guide for this format ({H3_MODE}) — authoritative
The code builds the final prompt structure from your fields. Apply these official rules to the wording you write; where the numbered rules further below disagree with this guide, the guide wins — except the JSON output shape and the [LINE N] / <DELIVERY> placeholders.
{H3_SPEC}

## Rules
1. Everything is English **except the on-screen text in rule 4**. Do not write Chinese anywhere else: dialogue is inserted by the code through the [LINE N] placeholders.
   **Character names above are Chinese — never copy a name into your answer.** Refer to a
   tagged character by its tag (<Subject 1>), and to an untagged one by a short English
   description you keep identical every time ("the blue cartoon assistant", "the pink one").
2. subjects: one entry per <Subject N> tag above, in the same order.
   **If the Tags section says this format has no <Subject N> tags** (a shot without reference images, or a first-frame shot),
   write no <Subject N> at all — not even in detailed_description — and return `"subjects": []`;
   write <Picture N> only where the Tags section allows it.
   A tag the code did not give you points at nothing; it is an error, not a shorthand.
   Describe everyone in plain English words instead.
   look = the subject's visible appearance in English, as a phrase starting with "a", "an" or "the"
   (e.g. "a slim woman around 28 with long straight black hair, a pale oval face, a white shirt and black trousers"); only what the description gives, do not invent features;
   always name the costume; if the description gives no costume colors, write "wearing the costume exactly as shown in <Picture N>" instead of guessing a color;
   **With two or more people, give every person an equally specific look** — face shape, eyes, hairstyle and hair colour, build,
   the costume's colours and cut, and one or two features nobody else in this shot shares. A one-line look next to a rich one
   gets overwritten by the stronger person: the video draws the strong one twice and drops the thin one.
   control / not_control = English translations of the given control ranges, nothing added.
3. summary: one sentence describing the target video. Use only the tags above (it is used only in the reference format).
4. detailed_description: first one or two sentences on visual style, palette and light (follow the project's visual style if one is given). Then "[Shot 1]" and one paragraph that follows the time segments in order.
   **Segment 1 starts right after "[Shot 1]" with no time marker** (the official format never puts a timestamp on the first shot).
   **Open every later segment with `Around the N.N-second mark,` at that segment's start second**, then write that segment out in full:
   shot size, camera angle and the camera move, where each subject is, what they do step by step, their expression, gaze and hands, how the state changes.
   Translate everything the segment gives; do not merge two segments and do not compress a segment into one sentence —
   **the code checks that every segment has its own time marker.** You may add more `Around the N.N-second mark,` inside a segment for beats within it.
   **Never write `At 00:SS` inside a shot** — in the official format `At MM:SS.mmm` marks a cut to a new [Shot N].
   The first time a tag appears, state its own place in the frame (left / centre / right, foreground / background), what it is doing,
   and how it is separated from the others ("<Subject 3>, the only woman in the beige suit, stands alone at the right of the midground").
   **Never write compass directions** — no north / south / east / west and no northern / southward / east side / west wall.
   The storyboard uses them (in Chinese) to keep the place consistent from shot to shot, but the video model cannot map a compass
   word onto the picture: "the car door opens toward the west" put a car door into a wall. Translate every one into what the
   camera sees, using the camera position given for that segment: frame left / right / centre, foreground / background,
   nearer to / farther from the camera, toward / away from the camera, or relative to a fixed landmark in this place
   (the alley mouth, the sheet-metal door, the brick wall, the puddle). This applies to every field, including look and control.
   **Coloured light never changes who someone is.** Write it as light falling on surfaces — the shadow side of a jacket, the wall
   behind someone — and say the person's natural skin tone and hair colour stay unchanged. Never write that a face, skin or hair
   turns, becomes or goes green / blue / red, or is "green-cast": the video reads that as a different person.
   When someone speaks, write **the second it starts** immediately before the placeholder sentence, like this:
   `Around the 3.0-second mark, <Subject 1> (S1) says, [LINE 1]` — seconds with one decimal, from the start of this shot.
   Every [LINE N] appears exactly once, and every one of them carries its own `Around the N.N-second mark,`.
   **The code checks those numbers against natural speaking speed (about 4 characters per second) and the length of each line.**
   If a line cannot finish before the next one starts, or before the shot ends, you will be asked to move it earlier — so leave room.
   Never write the Chinese words or a <d> tag yourself — the code replaces [LINE N] with the exact line.
   If that sentence contains <DELIVERY>, replace that one token with the English delivery phrase requested there (volume, pace, pitch, accent, emotion — 3 to 10 words).
   Never leave <DELIVERY> in your answer and never copy the Chinese it was given in. Change nothing else in the sentence.
   If nobody speaks, write that the people in frame keep their lips closed.
   End the paragraph with "Sound:" and the synchronized sounds in this shot (no dialogue repeated there, no music).
   One continuous shot: never write cuts or [Shot 2]. Write only one definite version, never "or".
   **No burned-in subtitles** (the dialogue must not appear as captions).
   **Text that is part of the story is different and must be kept**: a delivery slip, a phone screen,
   a chart title, a sign. Write it the official way, `the phrase "未读消息" printed on the screen` —
   **put the Chinese inside English double quotation marks, character for character**: do not translate it,
   do not paraphrase it, do not say "three characters". The quotes are how the checker tells
   on-screen text apart from a mistake — Chinese written outside double quotes (and outside the dialogue
   tags) is rejected, and so is a quoted Chinese span longer than 20 characters. Use them only for text
   that is really visible in the picture. Those phrases are usually where the joke lands.
   Length {MIN_WORDS}-{MAX_WORDS} English words. This shot is {SECS} seconds — a short shot written too full dilutes the few lines that actually constrain the picture.
5. overall_soundscape: one sentence, only ambience and sound effects, no dialogue, no music.
6. frame: an English prompt for a single still image of the opening moment (shot size, angle, subjects, scene, light). No camera movement, no dialogue.
   **Say where everyone stands in that frame** — left / centre / right and foreground / background, relative to the fixed things of the place (the bench, the door, the window) — which way each person faces, where they look, and what each hand holds. The opening frame decides the staging of the whole shot: whoever is on the left here stays on the left.
{PROJECT_RULES}

{PERFORMANCE}

{COMBAT}

{FEEDBACK}

Output JSON only, in this shape:
{"subjects":[{"tag":"<Subject 1>","look":"...","control":"...","not_control":"..."}],
 "summary":"...",
 "detailed_description":"...",
 "overall_soundscape":"...",
 "frame":"..."}

Example of a detailed_description (a different shot, same tone and density):
The target video uses a realistic, low-key cinematic style with a cold blue-cyan palette, deep shadows and a dark office at night.
[Shot 1] A fixed medium shot at eye level frames <Subject 1> from the waist up, seated at a cluttered desk in a cramped office at 2 a.m. The room around her is almost completely dark; the only strong light source is a desktop computer monitor in front of her, its harsh cold blue light falling across her pale face, her white shirt and her hands. A smartphone about 15 centimeters long lies screen-up on the desk to the right of the keyboard. She stares straight at the monitor with a strained, exhausted expression, and both hands hammer the keyboard rapidly; her hands never leave the keys. Around the 2.0-second mark, the phone begins to vibrate and its screen flashes, casting small pulses of white-blue light on the desktop. Around the 4.0-second mark, her typing falters, her eyes lose focus, and around the 6.5-second mark her head suddenly drops and her upper body collapses forward toward the desk in one abrupt motion. Nobody else appears. The camera does not move at any point. In the final half second the monitor light dies and the frame falls to complete darkness. Sound: frantic mechanical keyboard clicks overlapped by the buzzing rattle of the phone; as she collapses the typing stops, a single heavy heartbeat thuds once, and all sound falls away with the light.
"##;

// **用 `r##"` 而不是 `r#"`** —— 提示词里有 Hex 色值（`"#3D2314"`），
// 那个 `"#` 会把 `r#"` 的原始字符串提前闭合掉。
const ENTITY_EXTRACTION: &str = r##"你是一名专业的分镜师与编剧。
分析下面的文本，为漫剧/视频制作提取结构化数据。

重要：
- 所有描述性内容（名称、描述）必须使用简体中文。
- 只提取角色（characters）、场景（scenes）和道具（props）。
- **角色的长相和服装按槽位填，一个槽都不要留空**（`look` / `outfit`）。
  写成一段自由描述的后果实测过：
  「年轻男性，身形清瘦，短发，面容带有疲惫与压抑的阴影，常穿简单的便装，
  25-30岁，简单朴素的日常便装」—— 没有身高、没有脸型、没有瞳色、没有发色，
  模型每出一张图就替我们决定一次，**同一个角色每张脸都不一样**；
  「面容带有疲惫与压抑的阴影」是情绪不是长相，会被焊死在他**每一张**图上，
  连他高兴的那一镜也是；服装还写了两遍。

  原文没交代的槽位**按这个角色的身份推一个合理值**，不要留空 ——
  留空等于交给出图模型现编，而它每次编的不一样。

  **两个槽不要说同一件事。** `hair` 只写形状和发质，颜色一律归 `hair_color`。
  盘起来的发型最容易犯这条 —— 照下面这样写：

  · 「束起高髻，鬓边留两缕」 ✅   「**黑发**束起高髻」 ❌（发色说了两遍）
  · 「长发束于冠内，垂于脑后」 ✅  「**黑色**长发束于冠内」 ❌

  实跑三个角色里有三个把发色写进了 `hair`，而那一格本来能多说一点形状。
  只有服装的 `headwear` / `accessories` 例外：真没戴就留空，
  我们会渲染成「无帽」「无配饰」。

- **没有身体的角色**（手机屏幕上的卡通 AI 头像、一团光、一个图标、
  只在屏幕里出现的助理形象）：`outfit` 整个留空，`look` 里只填
  `role`（它是什么）和颜色、形状、材质那几项（`hair_color` 当主色用，
  `face` 写轮廓形状）。

  **身高、体型、上衣、下装、鞋一个都不要填** —— 填了它就变成一个人。
  这类角色我们会自动换成「头像」版式出图，而不是全身三视图。

- **非人但有身体的角色**（猫、狗、机器人）：照常填 `look`，
  但用它自己的说法 —— `hair` 写毛色花纹或外壳材质，`build` 写体型，
  `outfit` 一般留空（除非它真的穿衣服）。

- 角色**不要写具体表情（悲伤、愤怒）或临时动作（奔跑、哭泣）** ——
  它们会被焊死在这个角色的每一张图上。
- 同一个人若有显著不同的造型（便装 → 婚纱），**为每个造型单独建一条**，
  取名 `名字（造型）`，并在 persona 字段里填同一个人名。
- **每个角色都要给身份锚点（`anchors`）。** 形容词锁不住长相：
  「清秀的东方女性」对应的是一大片可能性，每次出图落在哪儿全看运气 ——
  这就是同一个角色在不同镜头里长相漂移的来源。锚点要的是**判别性强的东西**：
  - `marks`：**最强的一层，尽量写**。位置必须写死 ——
    「左眼下方两厘米一颗小痣」「右眉贯穿一道浅疤」。
    「有颗痣」这种没有位置的写法约束不住任何东西。

    **分清「读得出来的」和「编的」**：
    · 原文写了「面色苍白、连夜加班」，那么「黑眼圈明显」「眼角微垂」是
      **读得出来的** —— 写进去，它们正是让这个角色认得出来的东西。
    · 原文没提的痣、疤、胎记是**编的** —— 不要写。编出来的痣会被画进
      这个角色的每一张图，而且每张位置还不一样。
    一条都读不出来时留空数组，不要硬凑。
  - `color_anchors`：**写十六进制色值，不要写颜色名。**
    「棕色头发」覆盖的色域大得离谱，`#6B4423` 没有这个问题。
    从原文的描述推一个合理值即可。
  - `avoid`：这个角色**不该**有的特征（`["金发", "蓝眼", "胡子"]`）。
    只写会认错人的那几个，不要写画质问题。

- **角色描述里不要写它站在哪儿。** 「在东南角微微波动的光斑」——
  「东南角」是这一刻的站位，不是这个角色的长相。资产描述会被原样抄进
  **每一镜**的实体清单，于是角色被钉死在一个角落里，二十二镜都在那儿。
  实测踩过：那句「东南角」出自我们自己的资产库，模型只是照抄。
  位置属于分镜的站位，这里只写**离开这个场景也还成立**的东西。

- **剧本开头那几行元信息不是实体。** 长这样：

  ```text
  **类型**：古风 / 穿越 / 爽文漫剧
  **时长建议**：60–90秒
  **基调**：热血 × 盛唐史诗感 × 爽点节奏
  ```

  实测抽出来过三个「角色」：`**类型**`、`**时长建议**`、`**基调**`。
  它们是给人看的标注，**不是人**。从「【序幕】」「【第一幕】」这类
  场次标记之后才开始有实体。

- **开口说过话的人，一个都不能漏。** 实测漏过：剧本里
  「大臣厉声：『大逆不道！……当斩！』」有完整台词，抽出来的名单里
  却没有这个角色 —— 而分镜到了那一句会引用一个资产库里不存在的名字，
  于是他只以名字进提示词，长相全靠模型现编。
  哪怕只有一句话、哪怕原文只叫他「大臣」「侍卫」「路人甲」，也要列出来（龙套标 `incidental: true`，见下）。

- **穿在身上的东西不是道具。** 实测抽出来过 `官帽` `龙袍` `帝冠` 三个
  道具，而它们同时就在角色的 `outfit` 槽里 —— 一份东西两个所有者，
  分镜还可能写出「[道具:龙袍]」而那个人本来就穿着它。

  判据是**它会不会离开这个人单独出现**：手里拿的旧手表、桌上的手机、
  殿上的龙椅 → 道具；穿戴在身上的衣服、鞋、冠、配饰 → 写进 `outfit`，
  不要另建道具。

- **场景描述里不要写人，连"人"这个字都不要出现。**
  场景资产要出的是**空场景参考图**，画面里有人的话，每一镜都会多出
  一个不该在的人。人物进不进画由分镜决定。

  两条实测的反例，第二条是伪装成氛围溜过去的：

  ```text
  屏蔽室中央……角落悬浮着零的光点        ← 直接写了角色
  光线昏暗，蓝光映照在桌面与人脸上        ← "人脸"一样会被画出来
  百官列立两侧                          ← 人群也是人
  ```

  要写光落在哪儿就写落在**物**上：「蓝光映照在桌面和键盘上」。

- **场景描述要按这个顺序写全**：内景还是外景、是什么地方、**尺度**
  （大致高度和宽度，比如「高约15米、宽约30米」）、**背景层**、**前景层**、
  光线、氛围。
  实测抽出来过一份只有一个字的场景描述：「夜」—— 那一个字撑不起任何一张图。
  尺度尤其不能少：**同一个场景要出四个机位**，没有尺度的话四张图会是
  四个大小不同的房间，而它们本该是同一个。

  **背景层和前景层要分开写，这是信息量的大头。** 平铺成一串名词
  （「殿宇巍峨，中央设龙椅，地面铺石砖」）撑不起画面，模型只能自己编
  纵深。分层是在告诉它**什么在远处、什么在近处**：

  ```text
  ✅ 背景层为雕龙画栋的巨大红柱、华丽的藻井与深邃的大殿后方空间，
     前景层为高高在上的黄金龙椅、宽阔的御道与汉白玉台阶
  ❌ 殿宇巍峨宏伟，中央设龙椅，地面铺石砖，立柱高耸
  ```

  外景同理：背景层是天际线 / 远山 / 楼群，前景层是脚下的路面、
  最近的那几间铺面。原文没交代的按这个地方的性质推，不要留空。

- **`disembodied: true` 的意思是「这个角色整部剧都拍不出来」**，不是「他这次在画外」。
  标了之后：**不给他出图**，分镜也不单独给他镜头，台词压在对手戏角色的画面上。

  ✓ 该标：旁白员、纯语音 AI、一团没有轮廓的光、只存在于回忆里的声音。

  ✗ **不该标**：
  - 这一场在打电话、下一场会露脸的人 —— 他有身体，**某一句在画外**是逐句的事
    （镜头表里那一句写 `off_screen: true`），不是角色属性；
  - 只在这一集没出场的人；
  - 画面里看得见的非人角色 —— 猫、手机屏幕上的卡通 AI 头像、机器人。
    **它们有形体，要出图。**

  拿不准就留 `false`：标错成 true 的后果是这个角色整部剧都没有图、没有镜头，
  而且没有任何报错。

- **每一条都标 `incidental`：它是不是这一集顺带出现的。** 资产库是按整部剧的设定建的，
  这一集里顺带出现的人和东西**不单独建资产**，镜头里按文字描述出就够了。

  ✓ 标 `true`：没名没姓的龙套（邻居甲、打手乙、路人、围观的人）；
    只在这一场用一下的普通物件（铁棍、电锅、洗脸盆、碗这类随处可见的东西）；只路过一下的普通地点。
  ✗ 标 `false`：有名有姓、或者剧情会再用到的人；推动剧情、会被特写、每次出现都要长得一样的道具
    （信物、证据、标志性的武器或装置）；故事发生的主要场景。

  拿不准就标 `false` —— 标成 `true` 只是入库时默认不建，用户还能改回来。

严格按以下结构输出合法 JSON，不要输出任何解释文字：
{
  "characters": [
    {"name": "角色名", "persona": "同一个人的共用名",
     "incidental": false,
     "disembodied": false,
     "look": {
       "role": "身份或职业，比如「现代白领」「盛唐女官」——它给整张图定调。非人角色写它是什么：「家猫」「手机里的卡通 AI 助理」「机器人」",
       "gender": "女性", "age": "28岁", "height": "165厘米",
       "build": "体型偏瘦",
       "hair": "齐肩短发，发质略显干枯",
       "hair_color": "黑发",
       "face": "瓜子脸", "eyes": "黑瞳", "skin": "肤色苍白"
     },
     "outfit": {
       "top": "白色微褶皱衬衫", "bottom": "黑色包臀裙", "shoes": "黑色皮鞋",
       "headwear": "", "accessories": ""
     },
     "anchors": {
       "marks": ["左眼下方两厘米一颗小痣"],
       "color_anchors": {"iris": "#3D2314", "hair": "#1A1A1A",
                         "skin": "#E8C4A0", "lips": "#C4727E"},
       "avoid": ["金发", "蓝眼"]
     }}
  ],
  "scenes": [
    {"name": "地点名", "incidental": false, "description": "光线、氛围、关键元素"}
  ],
  "props": [
    {"name": "道具名", "incidental": false, "description": "外观描述"}
  ]
}

文本：
{TEXT}"##;

const SCREENPLAY_FROM_SOURCE: &str = r#"你是一名资深编剧。把下面的原文改写成可拍摄的中文剧本。

# 格式（这一条最要紧）

产物会被编辑器逐行识别，**格式错了不报错，只表现为排版乱、后面抽角色和分镜
也跟着乱**。严格照下面写，不要加任何解释文字：

```
【场景：老茶馆 · 日 · 内】

午后的阳光斜进来，桌上的铜壶冒着白汽。老茶师坐在案后，慢慢转动茶杯。

老茶师
（压低声音）
这茶，要等。

年轻人
等多久？

切至：
```

规则：
1. 场景标题写成 `【场景：地点 · 时间 · 内/外】`，每场开头一个。
2. **角色名单独占一行**，下一行紧接着是台词，**中间不空行**。
   空行会让台词被当成动作描写，而那意味着它不会被念出来。
3. **语气提示单独一行**，用全角括号，写在角色名和台词中间：

   ```
   小林
   （咬牙，几乎是从牙缝里挤出来的）
   所以用户满意度是零。
   ```

   **不要写成 `小林（咬牙）：…` 这种行内形式** —— 下游要靠这一行单独取出来当
   表演指示，混在台词行里会被当成台词的一部分念出声。
   不是每句都需要，但**凡是语气不等于字面意思的，一定要给**。
4. 动作/环境描写就是普通段落。**写看得见的东西** ——
   「他很愤怒」拍不出来，「他把杯子重重顿在桌上」拍得出来。
5. 场与场之间空一行。转场写 `切至：` 单独一行，可省略。
6. **台词要能被听懂。** 原文里的心理活动、旁白式叙述不要直接变成台词；
   该留的留成动作描写，该说出口的才写成台词。
7. 人名前后统一，**不要一会儿「老茶师」一会儿「茶师」** ——
   下游靠名字把角色对上，换个叫法就是两个人。

# 台词要能被念出来（直接决定成片里人物的声音）

台词会被**逐字合成成语音**，一个标点都照念。纸面上通顺的句子念出来往往像念稿。
**每写一句，在心里念一遍**；不像人会说出口的话就重写。

1. **语体跟着人物走，不是一律改成日常口语。**

   默认往口语走：
   - ✗「全年打碎玻璃杯三个，陶瓷碗一个。」
   - ✓「玻璃杯……三个。碗，一个。」

   **但有些人物的书面腔是有意的，不要抹平**：官员、法官、播音员、机器人、
   AI 助理，以及「故作正经」本身就是笑点的场合（一只猫一本正经地做年终述职、
   两个 AI 一本正经地替主人谈婚论嫁）。这类角色保留公文腔，
   反差恰恰来自「说得太正式」。

   判据是**这个人会怎么说话**，不是「所有台词都要像日常聊天」。
   书面连接词（因此、随即、然而、综上）在日常人物的台词里不要出现；
   在公文腔角色的台词里是对的。
2. **允许不完整。** 人说话会停顿、会改口、会只说半句。
   停顿用「……」，被打断用「——」，自我修正就直接写出来（「我是说，不对，我的意思是——」）。
3. **语气词按人物加，不是每句都加**：啊、吧、呢、嘛、诶、哎、嗯、哦、欸。
   加太多显得油滑。**但「一个不加」在某些人物身上是对的** ——
   公文腔角色、AI、冷静的人本来就不用语气词，别为了凑而加。
4. **感叹和反应可以单独成句**：「哎。」「得。」「行吧。」「你等会儿——」
5. **拟声词分清是「谁发出的」**：
   - 人**说**出来的（学猫叫、模仿撞门声）写进台词 —— 它会被念出来；
   - 环境和动作发出的（门响、脚步、手机震动）写进**动作描写**，
     不要写进台词栏 —— 写进去就变成有人把「哐」这个字念出声。
6. **每个人说话尽量不一样**：句子长短、常用词、语气词习惯、绕不绕弯子。
   写完自查一遍：**把角色名遮住，还分得出是谁在说吗？**

   **但原文没给的口头禅不要编。** 材料里看不出这个人怎么说话时，
   就让他说得平实一点 —— 那比硬安一个习惯好。
7. **口音和方言写进语气提示行，不要靠台词用字去造**：`（带川渝口音，语速偏快）`。
   台词本身用规范汉字 —— 它是被逐字念出来的，方言用字只会被按普通话念出来。

**但不要为了口语而加戏。** 原文里没有的情节、人物、台词内容一律不补 ——
这一条压过上面七条。口语化是换说法，不是加内容。

{NOTE}

{CRAFT}

原文：
{TEXT}"#;
const STORYBOARD_POLISH: &str = r#"# 角色
你是一名资深分镜师与提示词工程师。把一段草稿改写成高质量的图像生成提示词。

# 可用素材
{ASSETS}

# 规则
1. **严格忠实**：不要臆造草稿里没有的情绪、动作或剧情。草稿写「坐着」，
   就不要擅自加「悲伤地」。保持中性、准确。
2. **保留资产标签**：草稿里的 `[角色:名字]` / `[场景:名字]` / `[道具:名字]`
   **原样保留，连方括号一起**。它们是参考图的绑定关系，改了就挂不上。
3. **补视觉细节**：光线、氛围、材质可以补，但不要改变主体。
   **不要补景别、机位、构图或画风** —— 「中景」「低角度平视构图」
   「写实摄影风格」这类一个都不要写。景别有自己的字段，画风是整部片子
   统一定的，两者都会在你之后拼进去；你写了就会盖掉它们，
   而且不报任何错。
4. **长度上限 {MAX_CHARS} 字**，这是当前出图引擎的硬约束，超了会被摊薄
   （关键描述会失效，而且不报错）。宁可少写，不要超。
5. 只返回 JSON，不要解释。

# 输出格式
{"prompt_cn": "中文提示词", "prompt_en": "English prompt"}

# 草稿
{DRAFT}"#;

const VIDEO_POLISH: &str = r#"你是一名资深视频提示词工程师，为「图生视频」（Image-to-Video）优化草稿。

准则：
1. **结构**：提示词 = 运动描述 + 镜头运动。
2. **运动描述**：画面里各元素怎么动。用副词控制速度与强度
   （缓慢地 / 快速地 / 轻微的）。
3. **镜头运动**：需要时显式写（推近 / 向左平移 / 固定镜头）。
4. **首帧已经定了画面**，不要重复描述静态外观（长相、服装、场景陈设）——
   那些来自首帧图，写了也不会改变它，只会挤占篇幅。
5. 简洁具体，聚焦「动」。

只返回 JSON，不要解释：
{"prompt_cn": "中文", "prompt_en": "English"}

草稿：
{DRAFT}"#;

const T2V_POLISH: &str = r#"你是「文生视频」（Text-to-Video）的提示词工程师。

# 背景
文生视频**没有任何输入图**。画面里的一切 —— 长什么样、在哪儿、什么光 ——
只能从这段文字来。图生视频可以省掉这些（那边有图），
这边省掉就等于画面里没有。

# 任务
1. **画面**（先写）：主体长什么样（年龄、身形、发型、服装颜色与材质）、
   在什么环境、什么光线和色调。一次写定，不要给「或者」。
2. **动作与时间**：按时间顺序写**这一段视频里发生的变化** ——
   谁先做什么、然后怎么变。用「起初…接着…最后…」这样的顺序词把
   时间串起来。一个动作贯穿全程的话就明说「全程保持」。
3. **镜头**：一个镜头走到底，写明机位和唯一的一次运镜
   （推近 / 向左平移 / 固定镜头）。**不要写剪切、转场、分镜号**。
4. **声音**：末尾一句写同期声（环境声、动作音效）。不写配乐。
5. 保留原有的情绪基调，不要改写意图。

只返回 JSON，不要解释：
{"prompt_cn": "中文", "prompt_en": "English"}

草稿：
{DRAFT}"#;

const R2V_POLISH: &str = r#"你是「参考生视频」（Reference-to-Video）的提示词工程师。

# 背景
参考生视频用参考图锁住人物身份，再按提示词生成画面。
本次可用的参考资产：
{SLOTS}

# 标签格式（重要）
本系统的引用标签写作 `[角色:名字]`、`[场景:名字]`、`[道具:名字]`，
按**名字**解析，没有编号。

- 草稿里已有的标签**原样保留**，方括号和中文标签一起保留。
- 草稿里出现了上面列表中的资产名字但没加标签，首次提到时补成完整标签。
- **绝不要写成 `[character1:名字]` 或 `[人物:名字]`** —— 那种写法本系统
  解析不了，方括号会原样进引擎被画进画面。

# 任务
1. **结构**：场景设定（环境、光线、氛围）→ 角色动作（谁在做什么、表情）
   → 镜头运动。
2. **对白**：写成 `[角色:名字] 说：「台词」`。台词原样保留，不要改写 ——
   它会被转成同步语音。
3. **保留意图**：原有情绪基调不要变。
4. 补视觉细节增强戏剧效果（光线、速度副词）。

只返回 JSON，不要解释：
{"prompt_cn": "中文，保留完整标签", "prompt_en": "English, tags kept verbatim"}

草稿：
{DRAFT}"#;

/// **改写体裁，不改内容。**
///
/// 这一段刻意写得很克制：它的活是「换一种说法」，不是「加东西」。
/// 加东西会让用户勾选的那几项被稀释 —— 而用户勾了什么，是他自己的决定。
const PROSE_POLISH: &str = r#"你是提示词改写助手。

# 任务
把下面这串**逗号分隔的关键词**改写成连贯的中文画面描述。

# 规则
1. **一个要素都不能丢，也不要新增。** 输入里的每个关键词都要在输出里
   有对应的表述；输入里没有的东西不要补。
2. 只改**说法**，不改**内容**。不要评判、不要建议、不要加免责声明。
3. 写成通顺的完整句子，按「主体 → 动作 → 服装 → 环境 → 光线 → 画风」
   的顺序组织。
4. 括号里的权重写法（例如 `(某个词:1.5)`）表示这一项要更突出 ——
   在句子里用更靠前的位置或更明确的措辞体现，**不要把括号和数字写进结果**。
5. 长度不超过 {MAX_CHARS} 字。

只返回 JSON，不要解释：
{"prompt_cn": "中文完整句子", "prompt_en": "English sentences"}

关键词：
{DRAFT}"#;

const COVER_POSTER: &str = r#"你是一名剧集海报设计师。读下面的项目信息，为这部剧设计一张封面，写成一段中文出图提示词。

要求：
1. 画面上方用醒目的书法或标题大字写出片名（不带书名号），片名一字不差
2. 主角放在画面主体位置，写清外貌、服装、神态；最多两个人物
3. 场景、光影、色调要体现题材和氛围，写出具体的环境元素
4. 电影海报构图，主体突出，不要拼贴、不要多宫格、不要出现片名以外的文字
5. 如果给了画面风格，按那个风格写
6. 150~300 字，只写画面，不写解释

严格返回 JSON：{"prompt": "..."}

项目信息：
{PROJECT}"#;

const STYLE_ANALYSIS: &str = r#"你是一名电影美术指导。读下面的剧本，分析题材、情绪与氛围，
推荐 3 种截然不同但都合适的视觉风格。

每种风格给出：
1. 风格名（中文，简洁）
2. 一句话描述
3. 推荐理由（为什么适合这个剧本，50 字以内）
4. 正向提示词（英文，逗号分隔，不超过 50 个词）
5. 负向提示词（英文，逗号分隔，不超过 30 个词）

**正向提示词只写「怎么画」，不写「画什么」。**

这一段会加在**整部剧每一次生成**的前面 —— 写进去的景物会跟着跑进每一个镜头。
实测出过：仙侠风格里的「雾、符箓、金色光环」跑进了现代办公室，
室内镜头的提示词里还留着 misty mountains。

- ✓ 渲染方式、材质、线条、笔触、色调、对比、光照性质、镜头语言、画幅质感
  （`cel-shaded, soft rim light, muted teal palette, film grain, shallow depth of field`）
- ✗ 具体景物、天气、建筑、道具、人物动作、情绪事件
  （`misty mountains, glowing talismans, neon city, a woman crying`）

负向词同理：写「不要什么画法」，不是「不要什么东西」。

严格返回合法 JSON，不要任何解释文字，不要重复内容，只返回 3 个：
{"styles": [
  {"name": "风格名", "description": "一句话", "reason": "推荐理由",
   "positive_prompt": "...", "negative_prompt": "..."}
]}

剧本：
{TEXT}"#;

// ---------------------------------------------------------------------------
// 跑一次
// ---------------------------------------------------------------------------

/// 真正发给模型的那段文字 = 生效的提示词 + 填好的占位符。
///
/// **单拎出来是为了能测**：「我改了提示词，下一次生成有没有真的用改后的」
/// 是这一块最容易静默失效的地方 —— 配置存进去了，生成还在用硬编码的默认，
/// 界面上完全看不出来。
pub fn compose(cfg: &PromptConfig, slot: PromptSlot, vars: &[(&str, &str)]) -> String {
    fill(&cfg.text_for(slot), vars)
}

/// 用某个槽位的提示词跑一次 LLM，回原始文本。
///
/// **走 `Tier::Background`** —— 模型由「设置 → 后台模型」决定，
/// 那里是这个项目里唯一决定「本轮用哪个模型」的地方
/// （见 `gateway::resolve` 的注释）。在这里再加一个模型选择器，
/// 就等于制造第二份真相。用户默认配的是 LM Studio：本机、免费、不出网，
/// 而这些都是高频小任务。
pub async fn run(
    gateway: &crate::Gateway,
    cfg: &PromptConfig,
    slot: PromptSlot,
    vars: &[(&str, &str)],
) -> Result<String> {
    use crate::llm::{ChatChunk, ChatRequest, Message, ThinkingMode};
    use futures_util::StreamExt;

    let vars = &with_project_rules(cfg, slot, vars);
    let prompt = compose(cfg, slot, vars);
    // **占位符没填干净就别发。**
    //
    // 模板里写了 `{ASSETS}` 而调用方没传，原样进模型 —— 模型看到一串花括号，
    // 照常吐出一份东西，**任务显示成功**，而那份东西缺了资产清单。
    // 自定义覆盖最容易撞到这个（用户手抄模板时漏了一个变量名）。
    let template = cfg.text_for(slot);
    let missing = missing_placeholders(slot, &template, vars);
    if !missing.is_empty() {
        return Err(Error::config(format!(
            "「{}」这一步缺了输入：{} —— 要么自定义的模板里少写/写错了这几个变量名，要么调用方没传。去「设置 → Studio → 提示词设置」核对一下",
            slot.label(),
            missing.join("、")
        )));
    }
    // **放 user 消息，不要只发一条 system。**
    //
    // 2026-09-13 实测（LM Studio + ornith-1.5-35b-a3b，思考关闭）：同一段分场提示词
    // 只发 system → 引擎报 `does not match the expected peg-native format`，一个字不吐；
    // 同样内容放 user → 16 秒正常返回。只发 system 时聊天模板里没有用户这一轮，
    // 模型不知道该从哪开始回答。提取实体一直没出事，只是因为那段提示词短。
    let mut req = ChatRequest::new(vec![Message::user(prompt)]);
    // 结构化产出要稳定可复现，不要创造性。
    req.temperature = Some(0.0);
    // **不要流式。** 这里把流拼回一个字符串就扔掉增量，流式没有任何好处
    // （LumenX 这条路也是非流式的）。它不会让生成变快 —— 速度由模型吐
    // token 的速率决定 —— 换来的是少一层 SSE 解析，以及报错来得干净：
    // 流式下上游会在一个 200 响应的流中间塞 error 对象。
    req.stream = false;
    req.max_tokens = Some(slot.max_tokens());
    // 思考对这些任务没帮助，只拖慢并吃掉 token 预算。
    req.thinking = ThinkingMode::Off;
    req.json_output = slot.json_output();
    req.json_schema = slot.json_schema();

    let stream_id = format!("prompt-{}-{}", slot.key(), uuid::Uuid::new_v4());
    // 用量记在「哪一步」上：取槽位名括号前那段（「镜头表」「细化时间线」「线路 B 提示词」）
    let step = slot.label().split('（').next().unwrap_or(slot.key()).trim();
    // Azure 拦的是**请求本身**时回 HTTP 400（`code: content_filter`），不是流里的 finish_reason —— 同样说清楚
    let (_, mut stream) = crate::token_usage::with_step(step, gateway.chat_stream(stream_id, crate::config::Tier::Background, req))
        .await
        .map_err(|e| if is_content_filter(&e.to_string()) { Error::runtime(content_filter_message(slot, 0)) } else { e })?;

    let mut text = String::new();
    let mut truncated = false;
    let mut filtered = false;
    while let Some(chunk) = stream.next().await {
        match chunk? {
            ChatChunk::Text { delta } => text.push_str(&delta),
            ChatChunk::Done { reason, .. } => {
                truncated = reason == crate::llm::FinishReason::Length;
                filtered = reason == crate::llm::FinishReason::ContentFilter;
                break;
            }
            _ => {}
        }
    }
    // **被服务商的内容过滤掐断，要说出来**，不能当成「写完了」往下传 —— 那样报的是「返回里没有 JSON」，
    // 原因指向我们的解析（2026-10-03 EP02 线路 B 一批 7 镜这样）。半截的东西一律不能用，正文槽位也一样
    if filtered {
        return Err(Error::runtime(content_filter_message(slot, text.chars().count())));
    }
    // **写到上限被截断，要说出来。** 以前截断的半截 JSON 照样往下传，报出来的是「结果里没有 JSON」——
    // 2026-09-29 Opus 5.5 写第 6 场连着三次这样，看报错完全想不到是上限不够。
    // 只对整段是 JSON 的槽位报错：半截 JSON 没法用；正文槽位半截也还能看，照旧返回
    if truncated && slot.json_output() {
        return Err(Error::runtime(format!(
            "「{}」这一步模型写满了 {} token 上限被截断（输出 {} 字）—— 内容太长写不完，或者模型写得太细；可以把这一场拆短，或换一个更简洁的后台模型",
            slot.label(),
            slot.max_tokens(),
            text.chars().count()
        )));
    }
    if text.trim().is_empty() {
        return Err(Error::runtime(
            "模型没有返回内容 —— 去「设置 → 模型」看看后台链路那一条是不是通的",
        ));
    }
    Ok(text)
}

/// 从模型输出里抠出 JSON。
///
/// **本地小模型最常见的不老实形式是包一层 ```json 围栏**，其次是在
/// JSON 前后加一句「好的，这是……」。直接 `serde_json::from_str`
/// 会失败，而那个失败看起来像「模型不支持」，实际只是多了几个字符。
pub fn extract_json(raw: &str) -> Option<&str> {
    let t = raw.trim();
    // 去掉 markdown 围栏
    let t = if let Some(rest) = t.strip_prefix("```") {
        let rest = rest.strip_prefix("json").unwrap_or(rest);
        rest.rsplit_once("```").map(|(a, _)| a).unwrap_or(rest).trim()
    } else {
        t
    };
    // 找第一个 { 或 [ 到与之配对的结尾
    let start = t.find(['{', '['])?;
    let open = t.as_bytes()[start];
    let close = if open == b'{' { b'}' } else { b']' };
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escaped = false;
    for (i, b) in t.as_bytes().iter().enumerate().skip(start) {
        if escaped {
            escaped = false;
            continue;
        }
        match *b {
            b'\\' if in_str => escaped = true,
            b'"' => in_str = !in_str,
            c if !in_str && c == open => depth += 1,
            c if !in_str && c == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(&t[start..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// 命令层
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct PromptSlotView {
    pub key: &'static str,
    pub label: &'static str,
    pub placeholders: &'static [&'static str],
    pub default_text: &'static str,
    /// 用户改过的文本；没改过就是空串。
    pub custom_text: String,
    pub overridden: bool,
    /// **这份覆盖是照着旧版默认改的，内置默认后来变了。**
    ///
    /// 不自动迁移（那是用户的文本），但要让他知道 —— 否则他会一直用着
    /// 一份缺了后来新增规则的提示词，而且什么迹象都没有。
    pub default_changed: bool,
    /// `{METHOD}` 换成哪家技能的原文；没有 `{METHOD}` 的槽位是 `None`。见 [`PromptSlot::method_source`]
    pub method_source: Option<&'static str>,
}

#[tauri::command]
pub fn prompt_slots(state: tauri::State<'_, crate::AppState>) -> Vec<PromptSlotView> {
    let cfg = load(&state.config_dir);
    PromptSlot::ALL
        .iter()
        .map(|s| PromptSlotView {
            key: s.key(),
            label: s.label(),
            placeholders: s.placeholders(),
            default_text: s.default_text(),
            custom_text: cfg.overrides.get(s.key()).cloned().unwrap_or_default(),
            overridden: cfg.is_overridden(*s),
            // 没记过指纹的老覆盖当成「没变」——**不要当成变了**。
            // 一次性把所有老覆盖都标成过期，用户看到的是一屏警告而不是一条提示，
            // 结果是整片忽略（这个仓库在「一加键全部误报」上栽过两次）。
            default_changed: cfg
                .default_hash
                .get(s.key())
                .is_some_and(|h| h != &text_hash(s.default_text())),
            method_source: s.method_source(),
        })
        .collect()
}

/// 存一个槽位。传空字符串 = 恢复默认。
#[tauri::command]
pub fn prompt_save_slot(
    state: tauri::State<'_, crate::AppState>,
    key: String,
    text: String,
) -> Result<Vec<PromptSlotView>> {
    if !PromptSlot::ALL.iter().any(|s| s.key() == key) {
        return Err(Error::config(format!("没有叫「{key}」的提示词槽位")));
    }
    let default_text = PromptSlot::ALL.iter().find(|s| s.key() == key).map(|s| s.default_text()).unwrap_or_default();
    let mut cfg = load(&state.config_dir);
    if text.trim().is_empty() {
        cfg.overrides.remove(&key);
        cfg.default_hash.remove(&key);
    } else {
        // **同时记下当时的默认长什么样。** 以后内置默认改进了，
        // 就能告诉用户「你这份是照旧版改的」—— 不自动迁移，但也不装作没事。
        cfg.default_hash.insert(key.clone(), text_hash(default_text));
        cfg.overrides.insert(key, text);
    }
    save(&state.config_dir, &cfg)?;
    Ok(prompt_slots(state))
}

/// 导出整份提示词覆盖（JSON 文本）。换机器、回滚、分享调参结果用。
#[tauri::command]
pub fn prompt_export(state: tauri::State<'_, crate::AppState>) -> Result<String> {
    let cfg = load(&state.config_dir);
    serde_json::to_string_pretty(&cfg).map_err(|e| Error::runtime(format!("导出失败：{e}")))
}

/// 导入整份提示词覆盖。**整份替换，不合并。**
///
/// 合并的话，导入一份「少了某个槽位」的配置之后，那个槽位仍然是本机的旧覆盖 ——
/// 用户以为自己回到了导入的那一份，实际是两份的混合。那种状态谁都说不清。
///
/// 认不出来的槽位名直接报错，**不静默丢掉** —— 悄悄少导入一个槽位，
/// 表现是「这一项怎么没跟过来」，而那时候完全看不出是导入丢的。
#[tauri::command]
pub fn prompt_import(state: tauri::State<'_, crate::AppState>, text: String) -> Result<Vec<PromptSlotView>> {
    let cfg: PromptConfig = serde_json::from_str(text.trim())
        .map_err(|e| Error::config(format!("读不懂这份配置：{e}")))?;
    let unknown: Vec<&String> = cfg
        .overrides
        .keys()
        .filter(|k| !PromptSlot::ALL.iter().any(|s| s.key() == k.as_str()))
        .collect();
    if !unknown.is_empty() {
        return Err(Error::config(format!(
            "这份配置里有不认识的槽位：{} —— 可能来自更新的版本，先升级再导入",
            unknown.iter().map(|k| k.as_str()).collect::<Vec<_>>().join("、")
        )));
    }
    save(&state.config_dir, &cfg)?;
    Ok(prompt_slots(state))
}

/// 试跑给每个占位符配的样例值。
///
/// # 为什么要按槽位逐个配
///
/// 原来所有槽位共用五个变量（TEXT / DRAFT / ASSETS / SLOTS / MAX_CHARS）。
/// 13 个槽位里有 **7 个**需要更多 —— 镜头表要 9 个，B 路要 6 个。
///
/// 那本来只是「试跑结果不像真的」，但 2026-09-19 我给 `run` 加了
/// 占位符拦截之后，**这 7 个槽位的试跑直接报错、一次都跑不起来**
/// （Codex Astra 的审查抓到）。这是「修一处而不扫同类」的又一次：
/// 我给生产路径加了拦截，没去想试跑喂的是不是完整变量。
///
/// 样例值要**像真的**：镜头表给一段真场景文本、B 路给真的标签表，
/// 否则试跑出来的东西看着合格、换到生产就散。
fn try_vars(slot: PromptSlot, sample: &str) -> Vec<(&'static str, String)> {
    let mut v: Vec<(&'static str, String)> = vec![
        ("TEXT", sample.to_string()),
        ("DRAFT", sample.to_string()),
        ("SCENE_TEXT", sample.to_string()),
        ("SHOT", sample.to_string()),
        ("SCENE", sample.to_string()),
        ("ASSETS", "人物：\n- 老茶师（有图）：花白短发，青灰长衫\n场景：\n- 老茶馆（有图）：木桌、木窗\n道具：\n- 铜壶（无图）".into()),
        ("SLOTS", "（试跑，没有参考图）".into()),
        ("MAX_CHARS", "1200".into()),
        ("FEEDBACK", String::new()),
        // 试跑不属于哪个项目，不带项目偏好（和 `load` 一样）
        ("PROJECT_RULES", String::new()),
        ("NOTE", String::new()),
        ("CRAFT", String::new()),
        ("PREV", "这是本集第一场".into()),
        ("SCENE_NAME", "老茶馆 · 午后 · 内".into()),
        ("PROJECT", "《每日小离谱》· 轻喜剧 · 竖屏".into()),
        ("SECS", "8".into()),
        ("TAGS", "<Picture 1> = <Subject 1> 老茶师（identity, fully_preserved）".into()),
        ("SEGMENTS", "{\"segments\":[{\"start\":0,\"end\":8,\"title\":\"老茶师斟茶\",\"visual\":\"老茶师提起铜壶斟茶，抬头看了一眼门口\",\"forbid\":\"\",\"constraints\":\"\",\"blocking\":\"老茶师在画面中间\",\"camera\":\"中景固定\",\"sound\":\"水声\"}]}".into()),
        ("TARGET", crate::timeline_detail::target_line(8.0, 6.0)),
        ("SPEAKERS", "[LINE 1]: at the moment it is spoken, write exactly \"<Subject 1> (S1) says, [LINE 1]\"".into()),
    ];
    let (min_secs, max_secs) = (crate::shot_table::MIN_SHOT_SECS, crate::shot_table::MAX_SHOT_SECS);
    v.push(("ITEMS", "1. 二十六岁男声，音色偏低、带一点烟嗓的沙，说话懒散、慢半拍\n2. 温柔，语速偏慢，近乎耳语｜台词：你别走，好不好？".into()));
    v.push(("MIN_SECS", format!("{min_secs}")));
    v.push(("MAX_SECS", format!("{max_secs}")));
    v.push(("SCENE_SECS", crate::storyboard_gen::scene_budget_line(Some(30.0))));
    // 拍法：和生产一样，ai-film 原文（见 `ai_film`）。镜头表和细化时间线各取各的章节
    v.push((
        "METHOD",
        match slot {
            PromptSlot::TimelineDetail => crate::cinematic::timeline_method(),
            PromptSlot::RouteBWriter => crate::cinematic::compile_method(),
            _ => crate::ai_film::shot_table_method(),
        },
    ));
    // 表演合同 / 打斗规则：和 `with_project_rules` 在正式调用里补的是同一份
    let en = slot == PromptSlot::RouteBWriter;
    v.push(("PERFORMANCE", if en { crate::combat::PERFORMANCE_RULES_EN } else { crate::combat::PERFORMANCE_RULES_ZH }.into()));
    v.push(("COMBAT", if en { crate::combat::COMBAT_RULES_EN } else { crate::combat::COMBAT_RULES_ZH }.into()));
    // 超长视频分段：试跑按 30 秒 / 3 段，一段没写时间的提示词
    v.push(("PROMPT", "一个女孩在雨夜的天台上，镜头从她身后慢慢推近。她转身跑下楼梯，冲进霓虹街道，在人群里回头看了一眼，说：「别跟着我。」最后消失在巷子尽头。".into()));
    v.push(("N", "3".into()));
    v.push(("WINDOW_SECS", "10.61".into()));
    v.push(("OVERLAP", "0.92".into()));
    if slot == PromptSlot::LongVideoSplit {
        v.retain(|(k, _)| *k != "SECS");
        v.push(("SECS", "30".into()));
    }
    // 一镜出：试跑按最常见的那种 —— 15 秒一场、一镜
    v.push(("SHOT_COUNT", "1".into()));
    v.push(("SHOT_SECS", "15".into()));
    v.push(("TOTAL_SECS", "15".into()));
    // 线路 B：试跑按最常见的参考生视频（Ref2VA），给它那份官方规范原文
    v.push(("H3_MODE", crate::h3_spec::Mode::Ref2va.label().to_string()));
    v.push(("H3_SPEC", crate::h3_spec::writer_rules(crate::h3_spec::Mode::Ref2va)));
    let (lo, hi) = crate::prompt_route_b::word_budget(8.0);
    v.push(("MIN_WORDS", lo.to_string()));
    v.push(("MAX_WORDS", hi.to_string()));
    for (k, group) in [("SHOT_SIZES", "shotSize"), ("ANGLES", "angle"), ("MOVES", "move")] {
        v.push((k, crate::storyboard_gen::vocab_line(group)));
    }
    // 只留这个槽位真的要的 —— 多给不会出错，但试跑的输入该和生产一致
    let want = slot.placeholders();
    v.retain(|(k, _)| want.iter().any(|ph| ph.trim_matches(['{', '}']) == *k));
    v
}

/// 一次试跑的结果。
///
/// # 为什么不只回模型吐了什么
///
/// 试跑要回答的是「我改完这份提示词，正式生成还能用吗」。而这条链路上
/// 真正会失败的一步不是「模型答没答」，是**「答出来的东西我们解析得出来吗」**：
/// 分场要逐字抄原文、镜头表要能解析成数组、润色要是双语 JSON。
///
/// 只回原始文本的话，「试跑返回了内容」和「正式合成能过」是两件事，
/// 而用户会把前者当成后者 —— 然后在真正生成时才发现改坏了
/// （2026-09-19 Codex Astra 的第三份审查抓到）。所以这里把**生产用的
/// 那几个解析器**直接跑在试跑输出上。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TryOutcome {
    /// 模型的原始输出，原样。
    pub raw: String,
    /// 这个槽位有没有生产解析器可跑。
    ///
    /// **false 时 `issues` 恒为空，界面必须说清楚「这一槽没法验」** ——
    /// 否则空的问题列表会被读成「验过了，没问题」。
    pub checked: bool,
    /// 解析/校验挑出来的问题。空 = 正式链路吃得下这份输出。
    pub issues: Vec<String>,
}

/// 用生产链路的解析器验一份试跑输出。回 `(验没验, 问题)`。
fn try_check(slot: PromptSlot, raw: &str, sample: &str) -> (bool, Vec<String>) {
    match slot {
        PromptSlot::EntityExtraction => {
            if parse_entities(raw).is_empty() {
                (true, vec!["没从这份输出里解析出任何实体 —— 正式抽取会直接报错".into()])
            } else {
                (true, vec![])
            }
        }
        // 分场要**逐字抄原文**，核对的就是试跑样例这段文本
        PromptSlot::SceneSplit => match crate::storyboard_gen::parse_scenes(raw, sample) {
            Ok(_) => (true, vec![]),
            Err(e) => (true, vec![e]),
        },
        PromptSlot::ShotTable | PromptSlot::ShotTableOneTake => {
            let mut notes = Vec::new();
            match crate::storyboard_gen::parse_shots(raw, &[], 1, &mut notes) {
                // notes 是「解析出来了，但有地方被修过」—— 照样要显示
                Ok(_) => (true, notes),
                Err(e) => (true, vec![e]),
            }
        }
        PromptSlot::StoryboardPolish
        | PromptSlot::VideoPolish
        | PromptSlot::T2vPolish
        | PromptSlot::R2vPolish
        | PromptSlot::ProsePolish => match crate::polish::parse_polished(raw) {
            Ok(_) => (true, vec![]),
            Err(e) => (true, vec![e.to_string()]),
        },
        PromptSlot::StyleAnalysis => match crate::style::parse_suggestions(raw) {
            Some(v) if !v.is_empty() => (true, vec![]),
            _ => (true, vec!["没解析出风格推荐 —— 正式调用会报错".into()]),
        },
        PromptSlot::RouteBWriter => match crate::prompt_route_b::try_parse(raw) {
            Ok(()) => (true, vec![]),
            Err(e) => (true, vec![e]),
        },
        PromptSlot::CoverPoster => match crate::cover::parse_cover_prompt(raw) {
            Some(_) => (true, vec![]),
            None => (true, vec!["没解析出封面提示词 —— 正式生成会退回模板，而且只在日志里说".into()]),
        },
        // 这两槽的产物就是正文，生产侧只要求「去掉代码围栏后非空」
        PromptSlot::ScreenplayFromSource | PromptSlot::AdaptationPlan => {
            if crate::script_library::strip_code_fence(raw).trim().is_empty() {
                (true, vec!["去掉代码围栏之后是空的 —— 正式调用会报「模型没有返回内容」".into()])
            } else {
                (true, vec![])
            }
        }
        // 没接进生成流程，没有生产解析器可对 —— 见它的 label
        PromptSlot::SceneContractExtraction => (false, vec![]),
        PromptSlot::VoiceDirection => match crate::prompt_route_b::parse_voice_directions(raw, 1) {
            Some(_) => (true, vec![]),
            None => (true, vec!["没解析出 {\"items\": [...]} —— 正式合成会退回让写手自己翻，多半会漏".into()]),
        },
        PromptSlot::TimelineDetail => match crate::timeline_detail::parse(raw) {
            Ok(_) => (true, vec![]),
            Err(e) => (true, vec![format!("{e} —— 正式细化会重试，三次都这样就这一镜不写入")]),
        },
        PromptSlot::LongVideoSplit => match crate::long_video::parse_model_split(raw, 3) {
            Ok(_) => (true, vec![]),
            Err(e) => (true, vec![format!("{e} —— 正式生成会再试一次，两次都这样就报错、不出片")]),
        },
    }
}

/// 把编辑框里那一份按 [`prompt_try`] 的语义盖进配置（**只在内存里**）。
///
/// 拎出来是为了能直接测 —— 这三条分支各自都出过事，而走 `prompt_try`
/// 要一个真的模型网关。
fn apply_draft(cfg: &mut PromptConfig, key: &str, draft: Option<String>) {
    match draft {
        Some(d) if !d.trim().is_empty() => {
            cfg.overrides.insert(key.to_string(), d);
        }
        // 清空 = 用内置默认，**不能回落到已保存的覆盖**
        Some(_) => {
            cfg.overrides.remove(key);
        }
        None => {}
    }
}

/// 试跑一个槽位。**给「改完看看对不对」用。**
///
/// `draft` 是编辑框里那一份，**语义是「就用这段跑」**：
///
/// | draft | 用哪份模板 |
/// |---|---|
/// | `Some(非空)` | 就这段 |
/// | `Some("")` | 内置默认（编辑框被清空 = 恢复默认） |
/// | `None` | 已保存的配置 |
///
/// # 为什么不能「和默认一样就不传」
///
/// 原来前端在 `draft` 和默认文案相同时传 `None`，后端于是去读**已保存的
/// 自定义覆盖**。用户把编辑框改回默认正文、不保存、点试跑 —— 跑的还是
/// 那份旧的自定义模板，而界面显示的是默认正文
/// （2026-09-19 Codex Astra 的第三份审查抓到）。
///
/// # 试跑不落盘
///
/// 更早之前前端会先 `promptSaveSlot` 再跑 —— 点一下「试跑」**全局正式配置
/// 就已经改了**，报错了也留着。试跑的意思是「先看看」，不该有副作用。
#[tauri::command]
pub async fn prompt_try(
    state: tauri::State<'_, crate::AppState>,
    key: String,
    sample: String,
    draft: Option<String>,
) -> Result<TryOutcome> {
    let slot = PromptSlot::ALL
        .iter()
        .copied()
        .find(|s| s.key() == key)
        .ok_or_else(|| Error::config(format!("没有叫「{key}」的提示词槽位")))?;
    let mut cfg = load(&state.config_dir);
    apply_draft(&mut cfg, &key, draft);
    let owned = try_vars(slot, &sample);
    let vars: Vec<(&str, &str)> = owned.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let raw = run(&state.gateway, &cfg, slot, &vars).await?;
    let (checked, issues) = try_check(slot, &raw, &sample);
    Ok(TryOutcome { raw, checked, issues })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    /// 设置页写的「`{METHOD}` 是哪家原文」必须和真正灌进去的那份一致。
    ///
    /// 两边各写各的，迟早有一边换了技能另一边没跟：页面说 ai-film，实际灌的是 cinematic，比不写更糟。
    #[test]
    fn 设置页标的拍法来源和实际灌进去的一致() {
        for slot in PromptSlot::ALL {
            let has_method = slot.placeholders().contains(&"{METHOD}");
            assert_eq!(slot.method_source().is_some(), has_method, "{}", slot.key());
            assert_eq!(slot.optional_placeholders().contains(&"{METHOD}"), has_method, "{}", slot.key());
            let Some(source) = slot.method_source() else { continue };
            let method = try_vars(slot, "样例")
                .into_iter()
                .find(|(k, _)| *k == "METHOD")
                .map(|(_, v)| v)
                .unwrap();
            let (tag, other) = if source.starts_with("ai-film") {
                ("ai-film", "cinematic-video-prompt-engineer")
            } else {
                ("cinematic-video-prompt-engineer", "ai-film")
            };
            assert!(source.contains(tag), "{}", slot.key());
            assert!(method.contains(tag) && !method.contains(other), "{} 标的是 {source}，灌进去的不是", slot.key());
        }
    }


    // ---- 试跑（2026-09-19）----

    /// **编辑框里那一份说了算。**
    ///
    /// 出过的事：前端在 draft 和内置默认相同时传 `None`，后端于是去读
    /// **已保存的自定义覆盖** —— 用户把编辑框改回默认正文、点试跑，
    /// 跑的还是那份旧的自定义模板，而屏幕上显示的是默认正文。
    #[test]
    fn 试跑用编辑框那一份_清空就是用默认() {
        let mut cfg = PromptConfig::default();
        cfg.overrides.insert("scene_split".into(), "旧的自定义".into());

        // 传了：就用传的
        let mut c = cfg.clone();
        apply_draft(&mut c, "scene_split", Some("编辑框里的新文本".into()));
        assert_eq!(c.overrides.get("scene_split").map(String::as_str), Some("编辑框里的新文本"));

        // 清空 = 恢复默认，**不能回落到旧覆盖**
        let mut c = cfg.clone();
        apply_draft(&mut c, "scene_split", Some("   ".into()));
        assert_eq!(c.overrides.get("scene_split"), None, "清空之后还在读旧的自定义模板");

        // 没传：读已保存的
        let mut c = cfg.clone();
        apply_draft(&mut c, "scene_split", None);
        assert_eq!(c.overrides.get("scene_split").map(String::as_str), Some("旧的自定义"));
    }

    /// **试跑要用生产解析器验，不是只把原文回显。**
    ///
    /// 「试跑返回了内容」和「正式合成能过」是两件事 —— 真正会拦住的是解析。
    #[test]
    fn 试跑要跑生产解析器() {
        // 润色：双语 JSON
        let (checked, issues) = try_check(PromptSlot::VideoPolish, r#"{"prompt_cn":"中","prompt_en":"en"}"#, "");
        assert!(checked && issues.is_empty(), "{issues:?}");
        let (checked, issues) = try_check(PromptSlot::VideoPolish, "我给你改好了：镜头缓缓推近。", "");
        assert!(checked && !issues.is_empty(), "解析不了却说没问题");

        // 风格推荐
        let (_, issues) = try_check(PromptSlot::StyleAnalysis, "随便一段话", "");
        assert!(!issues.is_empty());

        // 没有生产解析器的那一槽：**issues 必须空，checked 必须 false** ——
        // 空的问题列表会被读成「验过了，没问题」，所以界面靠 checked 区分
        let (checked, issues) = try_check(PromptSlot::SceneContractExtraction, "任意内容", "");
        assert!(!checked && issues.is_empty());
    }

    /// 每个槽位要么有解析器、要么明确说没有 —— 不许漏。
    #[test]
    fn 每个槽位的试跑校验都有交代() {
        for s in PromptSlot::ALL {
            // 拿一段肯定解析不了的输入：有解析器的必须报，没有的必须不报
            let (checked, issues) = try_check(s, "___这不是任何一种合法输出___", "样例");
            assert_eq!(checked, !issues.is_empty() || matches!(s, PromptSlot::ScreenplayFromSource | PromptSlot::AdaptationPlan), "{} 的试跑校验没交代清楚", s.key());
        }
    }

    /// 文生视频**不能**共用图生视频那份提示词 —— 那份的前提是「首帧已定」。
    #[test]
    fn 文生视频的提示词里没有首帧假设() {
        let t2v = PromptSlot::T2vPolish.default_text();
        assert!(!t2v.contains("首帧已经定了画面"), "文生视频抄了图生视频的首帧假设");
        // 文生视频必须自己给出时间、动作、声音这三样契约
        for must in ["时间顺序", "声音", "镜头"] {
            assert!(t2v.contains(must), "文生视频的提示词里没有「{must}」");
        }
        assert!(PromptSlot::VideoPolish.default_text().contains("首帧已经定了画面"), "图生视频那份丢了首帧假设");
    }

    /// **站位 / 布光 / 切镜 / 运镜 / 构图（2026-09-28 对照报告的 P0）在按钮用的模板里。**
    ///
    /// 镜头表两份模板的例子都带 `stage`，而且能被生产解析器读出来 ——
    /// 模型对例子的服从远强于规则，例子里没有或读不出来，规则写了也白写。
    #[test]
    /// **拍法只有一个来源**（2026-09-29 用户：「把全部链路换成同一套 SKILL，而不是大杂烩」）。
    ///
    /// 镜头表两份模板的拍法全部来自 `{METHOD}`（ai-film 原文），我们自己只写生产约束。
    /// 以前那 30 条里和 ai-film 打架的写法（「景别至少跳一级」对「没有固定景别阶梯」、
    /// 「每一秒都要有看得见的新变化」对「停住也成立」）一条都不能回来。
    fn 镜头表只有一套拍法_我们只写生产约束() {
        for slot in [PromptSlot::ShotTable, PromptSlot::ShotTableOneTake] {
            let t = slot.default_text();
            let example = &t[t.rfind("格式照这个例子").unwrap()..];
            let json = extract_json(example).expect("例子里要有 JSON");
            let shots = crate::storyboard_gen::parse_shots(json, &[], 1, &mut Vec::new()).unwrap();
            assert_eq!(shots[0].stage.lights.len(), 2, "{} 的例子没带 stage：{:?}", slot.key(), shots[0].stage);
            assert!(shots.iter().all(|s| s.stage == shots[0].stage));
            for must in ["{METHOD}", "## 生产约束", "写完 stage 必须接着写 shots", "stage 只写场里的位置", "逐字写进某一镜的 dialogue",
                "每一句正文都要被某一镜的 source 引到", "后期叠加", "钩子："] {
                assert!(t.contains(must), "{} 缺「{must}」", slot.key());
            }
            for gone in ["景别至少跳一级", "每一秒都要有看得见的新变化", "**暗部**", "按这一刻要干什么选运镜", "第一段 visual 写清构图", "三分线",
                "背景里的人各干各的", "表情具体到眉、眼、嘴角", "台词字数 ÷ 4"] {
                assert!(!t.contains(gone), "{} 里又出现了自己写的拍法「{gone}」—— 拍法只从 ai-film 原文来", slot.key());
            }
        }
        assert!(PromptSlot::ShotTable.default_text().contains("镜内不切"));
        // 用户 09-19 定下的：不为过场单开没台词的短镜（一镜出的场镜数是定死的，不用这条）
        assert!(PromptSlot::ShotTable.default_text().contains("不为过场单开没有台词的短镜"));
        assert!(!PromptSlot::ShotTableOneTake.default_text().contains("不为过场单开没有台词的短镜"));
        assert!(PromptSlot::ShotTableOneTake.default_text().contains("镜内可以切镜"));
    }

    /// 以前改过镜头表模板的人，模板里没有 `{METHOD}` —— 那是他自己的拍法，不能因此报错；别的占位符照旧拦
    #[test]
    fn 自己改过的镜头表模板不写方法也能跑() {
        let own = "场：{SCENE_NAME}\n{SCENE_TEXT}\n{PREV}\n{ASSETS}\n{MIN_SECS}~{MAX_SECS} {SCENE_SECS}\n{SHOT_SIZES}{ANGLES}{MOVES}\n{FEEDBACK}";
        let owned = try_vars(PromptSlot::ShotTable, "x");
        let vars: Vec<(&str, &str)> = owned.iter().map(|(k, v)| (*k, v.as_str())).collect();
        assert!(missing_placeholders(PromptSlot::ShotTable, own, &vars).is_empty());
        let no_assets = own.replace("{ASSETS}", "");
        assert_eq!(missing_placeholders(PromptSlot::ShotTable, &no_assets, &vars), vec!["{ASSETS}"], "别的占位符删了照样要拦");
        // 写了 {METHOD} 却没传，照样拦
        let without_method: Vec<(&str, &str)> = vars.iter().copied().filter(|(k, _)| *k != "METHOD").collect();
        assert_eq!(missing_placeholders(PromptSlot::ShotTable, PromptSlot::ShotTable.default_text(), &without_method), vec!["{METHOD}"]);
    }

    /// 细化时间线、线路 B 的拍法走 cinematic（分工版），模板里只有生产约束
    #[test]
    fn 时间线细化和线路b的站位规则还在() {
        let td = PromptSlot::TimelineDetail.default_text();
        for must in ["{METHOD}", "## 生产约束", "结构不动", "source_segments", "以导演台为准", "台词原文不写进 visual 和 sound"] {
            assert!(td.contains(must), "时间线细化缺「{must}」");
        }
        // 拍法只从 ai-film 原文来（第 2 步换掉的）：以前自己写的这几条不能回来
        for gone in ["表情具体到眉、眼、嘴角", "背景里的人各干各的", "起幅是什么构图", "关键台词、关键动作之后留 1~2 秒"] {
            assert!(!td.contains(gone), "时间线细化里又出现了自己写的拍法「{gone}」");
        }
        assert!(PromptSlot::RouteBWriter.default_text().contains("Say where everyone stands in that frame"), "线路 B 的首帧没要求写站位");
        assert!(PromptSlot::RouteBWriter.default_text().contains("{METHOD}"), "线路 B 写手没带 ai-film 的编译规则（第 3 步）");
    }

    /// **一镜不切**，那例子里就不能有切黑。
    #[test]
    fn 线路b的例子不跟正文打架() {
        let t = PromptSlot::RouteBWriter.default_text();
        assert!(t.contains("never write cuts"), "正文里没有「不许切镜」这条");
        assert!(!t.contains("cuts to complete black"), "例子里还留着切黑 —— 模型会照抄");
    }

    /// 2026-10-03：《回收师》要女性角色的性感机位和姿态，这是项目的口味 —— 只进这个项目的镜头表和细化时间线。
    #[test]
    fn 项目偏好进镜头表和细化时间线() {
        let mut cfg = PromptConfig { project_rules: project_rules_line("女性角色多用低机位仰拍，坐姿翘腿斜坐，走路猫步"), ..Default::default() };
        for slot in [PromptSlot::ShotTable, PromptSlot::ShotTableOneTake, PromptSlot::TimelineDetail] {
            let all: Vec<(&str, &str)> = slot
                .placeholders()
                .iter()
                .filter(|ph| **ph != "{PROJECT_RULES}")
                .map(|ph| (ph.trim_matches(['{', '}']), "x"))
                .collect();
            // 调用方不传，run 那一步自动补
            let v = with_project_rules(&cfg, slot, &all);
            assert!(missing_placeholders(slot, &cfg.text_for(slot), &v).is_empty(), "{slot:?}");
            let out = compose(&cfg, slot, &v);
            assert!(out.contains("- **用户定下的（本项目）**：女性角色多用低机位仰拍"), "{slot:?} 没带上项目偏好");
            assert!(!out.contains("{PROJECT_RULES}"), "{slot:?} 占位符原样进了模型");
        }
        // 线路 B 写手：英文引导语 + 偏好原文
        cfg.project_rules_en = project_rules_line_en("女性角色走跑时胸部有自然弹性");
        let slot = PromptSlot::RouteBWriter;
        let all: Vec<(&str, &str)> = slot.placeholders().iter().filter(|ph| **ph != "{PROJECT_RULES}").map(|ph| (ph.trim_matches(['{', '}']), "x")).collect();
        let v = with_project_rules(&cfg, slot, &all);
        assert!(missing_placeholders(slot, &cfg.text_for(slot), &v).is_empty());
        let out = compose(&cfg, slot, &v);
        assert!(out.contains("Project rule set by the user") && out.contains("胸部有自然弹性"), "写手没带上项目偏好");
        assert!(!out.contains("用户定下的（本项目）"), "写手拿到的是中文那一行");
        // 别的槽位不带（分场、配音这些和镜头偏好无关）
        for slot in [PromptSlot::SceneSplit, PromptSlot::VoiceDirection] {
            assert!(!with_project_rules(&cfg, slot, &[]).iter().any(|(k, _)| *k == "PROJECT_RULES"), "{slot:?}");
        }
    }

    /// 没有偏好的项目：那一行整个不出现，模板读起来和以前一样
    #[test]
    fn 没有项目偏好时一个字都不多() {
        assert_eq!(project_rules_line("  \n "), "");
        let cfg = PromptConfig::default();
        let all: Vec<(&str, &str)> = PromptSlot::ShotTable.placeholders().iter().map(|ph| (ph.trim_matches(['{', '}']), "x")).collect();
        let all: Vec<(&str, &str)> = all.into_iter().filter(|(k, _)| *k != "PROJECT_RULES").collect();
        let out = compose(&cfg, PromptSlot::ShotTable, &with_project_rules(&cfg, PromptSlot::ShotTable, &all));
        assert!(!out.contains("本项目"), "{out}");
    }

    /// 10-03 之前覆盖过这三个模板的人，模板里没有 `{PROJECT_RULES}` —— 不能因此报「缺了输入」
    #[test]
    fn 旧的自定义模板没有项目偏好也照常能用() {
        let mut cfg = PromptConfig::default();
        let old = SHOT_TABLE.replace("{PROJECT_RULES}\n", "");
        assert!(!old.contains("{PROJECT_RULES}"));
        cfg.overrides.insert(PromptSlot::ShotTable.key().to_string(), old);
        cfg.project_rules = project_rules_line("低机位");
        let all: Vec<(&str, &str)> = PromptSlot::ShotTable.placeholders().iter().filter(|ph| **ph != "{PROJECT_RULES}").map(|ph| (ph.trim_matches(['{', '}']), "x")).collect();
        let v = with_project_rules(&cfg, PromptSlot::ShotTable, &all);
        assert!(missing_placeholders(PromptSlot::ShotTable, &cfg.text_for(PromptSlot::ShotTable), &v).is_empty());
    }

    /// 生成镜头表、重生成一场、细化时间线这三条生产路径都要用 `load_for_project` ——
    /// 哪条用了 `load`，那条生成的镜头就不带项目偏好（又变回保守机位），而且不报错
    #[test]
    fn 生成镜头的入口都带项目偏好() {
        let body = |src: &'static str, f: &str| -> &'static str {
            let at = src.find(f).unwrap_or_else(|| panic!("找不到 {f}"));
            let rest = &src[at..];
            &rest[..rest.find("\n}\n").unwrap()]
        };
        let cinema = include_str!("cinema.rs");
        for f in ["async fn generate_shot_table_job(", "pub fn cinema_regenerate_scene(", "pub fn start_synthesize("] {
            assert!(body(cinema, f).contains("crate::prompts::load_for_project("), "{f} 没带项目偏好");
        }
        let tl = include_str!("timeline_detail.rs");
        assert!(body(tl, "pub fn cinema_refine_timelines(").contains("crate::prompts::load_for_project("), "细化时间线没带项目偏好");
    }

    /// 内容过滤的报错要说清是谁拦的、怎么办 —— 不能再让人去查我们的解析
    #[test]
    fn 内容过滤的报错说清原因和出路() {
        let m = content_filter_message(PromptSlot::RouteBWriter, 3040);
        assert!(m.contains("线路 B") && m.contains("内容过滤") && m.contains("3040"), "{m}");
        assert!(m.contains("后台模型") && m.contains("Azure"), "没说怎么办：{m}");
        assert!(!m.contains("没有 JSON"), "{m}");
    }

    /// 偏好存在项目的画风设置里，`load_for_project` 读得出来；`load` 不带
    #[test]
    fn 项目偏好从画风设置里读() {
        let d = tempfile::tempdir().unwrap();
        let art = crate::style::ArtDirection { shot_notes: "低机位仰拍".into(), ..Default::default() };
        crate::style::save_art(d.path(), "p1", &art).unwrap();
        assert!(load_for_project(d.path(), "p1").project_rules.contains("低机位仰拍"));
        assert!(load_for_project(d.path(), "p1").project_rules_en.contains("低机位仰拍"));
        assert_eq!(load_for_project(d.path(), "p2").project_rules, "", "别的项目串进来了");
        assert_eq!(load(d.path()).project_rules, "");
    }

    #[test]
    fn 每个槽位都有默认文案和占位符() {
        for s in PromptSlot::ALL {
            assert!(!s.default_text().trim().is_empty(), "{} 没有默认文案", s.key());
            assert!(!s.placeholders().is_empty(), "{} 没有占位符", s.key());
            for ph in s.placeholders() {
                assert!(
                    s.default_text().contains(ph),
                    "{} 的默认文案里没有 {ph} —— 占位符列错了，界面会误导用户",
                    s.key()
                );
            }
        }
    }


    /// **提示词里不能让模型自己写景别 / 机位 / 构图 / 画风。**
    ///
    /// 真出过事：润色写出「…低角度平视构图，景深较浅…写实摄影风格，高清晰度。」，
    /// 而项目选的是赛璐璐动画、镜头设的是中景 —— 那两个是拼在这句话
    /// **后面**的，被它整个盖掉。出来既不是中景也不是动画风，而且不报错。
    ///
    /// 治本在这里：谁都别抢那两个字段的活。
    #[test]
    fn 出画面的提示词都禁止自己写景别和画风() {
        // **断言要钉住那句禁令本身。**
        //
        // 第一版写的是「文案里出现过『景别』和『不要写』」—— 而提取提示词里
        // 本来就有「景别只能是：大特写 / 特写…」和「不要写台词」，
        // 把禁令整句删掉，断言照样绿（被邻居满足了）。
        //
        // 去掉 markdown 的 `**` 再比，免得加粗与否影响判定。
        let plain = |t: &str| t.replace("**", "").replace('`', "");
        let pol = plain(PromptSlot::StoryboardPolish.default_text());
        assert!(
            pol.contains("不要补景别、机位、构图或画风"),
            "润色提示词里没有那句禁令"
        );
        // 两处都要说清后果，否则改提示词的人会以为只是"建议"
        for (name, t) in [("润色", &pol)] {
            assert!(t.contains("盖掉") || t.contains("会赢"), "{name} 没说清后果");
        }
    }

    /// 结构化调用必须把提示词放 user 消息。只发 system 时本机模型直接报格式错、一个字不吐
    /// （2026-09-13 实测，见 `run` 里的注释）。
    #[test]
    fn 结构化调用把提示词放在_user_消息里() {
        let src = include_str!("prompts.rs");
        let body = &src[src.find("pub async fn run(").unwrap()..];
        let body = &body[..body.find("\n}\n").unwrap()];
        assert!(body.contains("ChatRequest::new(vec![Message::user(prompt)])"), "提示词没放 user 消息");
        assert!(body.contains("req.json_output = slot.json_output();"), "JSON 槽位没在采样层约束语法");
        assert!(!body.contains("Message::system("), "又只发 system 了");
        // 写到上限被截断：JSON 槽位要报出来，不能把半截 JSON 往下传（09-29 Opus 写第 6 场报成「没有 JSON」）
        assert!(body.contains("FinishReason::Length") && body.contains("truncated && slot.json_output()"), "截断没报");
        // 被内容过滤掐断：同样要报出来（10-03 EP02 线路 B 一批 7 镜报成「没有 JSON」）
        assert!(body.contains("FinishReason::ContentFilter") && body.contains("if filtered {"), "内容过滤没报");
    }

    /// 镜头表的上限够云端模型写一整场对白戏（09-29：Opus 5.5 写第 6 场三次写满 16384）
    #[test]
    fn 镜头表的输出上限够写一整场() {
        assert!(PromptSlot::ShotTable.max_tokens() >= 32768);
        assert!(PromptSlot::ShotTableOneTake.max_tokens() >= 32768);
    }

    /// 整段输出是 JSON 对象的槽位才约束；剧本、方案是正文，约束了会把内容挤坏。
    #[test]
    /// 镜头表那一问 7 次只写了 `stage` 就把 JSON 合上（2026-09-28~29）。采样层只写明顶层 `stage`、`shots` 必填 ——
    /// 再细就会被 LM Studio 删字段（见 [`PromptSlot::json_schema`]）
    fn 镜头表的_schema_只要求顶层两个键() {
        let s = PromptSlot::ShotTable.json_schema().unwrap();
        assert_eq!(s["required"], serde_json::json!(["stage", "shots"]));
        assert_eq!(s["properties"]["shots"]["minItems"], 1);
        // **每一镜不写 schema**：LM Studio 会删掉 required 里没声明的键（09-29 实测：duty、source、景别、光全没了）
        assert!(s["properties"]["shots"].get("items").is_none(), "每一镜的结构别写进 schema —— 写了就会被 LM Studio 删字段");
        let one = PromptSlot::ShotTableOneTake.json_schema().unwrap();
        assert_eq!(one, s, "一镜出用同一份");
        assert!(PromptSlot::SceneSplit.json_schema().is_none(), "别的槽位不加 schema");
        // 每一层对象都放开「没列出的键」—— LM Studio 默认把它们删掉（09-29 实测，一场塌成一镜）
        fn all_objects_open(v: &serde_json::Value, path: &str) {
            if v.get("type") == Some(&serde_json::json!("object")) {
                assert_eq!(v["additionalProperties"], true, "{path} 没写 additionalProperties: true");
            }
            if let Some(props) = v.get("properties").and_then(|p| p.as_object()) {
                for (k, p) in props {
                    all_objects_open(p, &format!("{path}.{k}"));
                }
            }
            if let Some(items) = v.get("items") {
                all_objects_open(items, &format!("{path}[]"));
            }
        }
        all_objects_open(&s, "镜头表");
        all_objects_open(&one, "一镜出");
        let src = include_str!("prompts.rs");
        assert!(src.contains("req.json_schema = slot.json_schema();"), "run 没把槽位的 schema 带上");
    }

    #[test]
    fn 只有整段是_json_的槽位才约束语法() {
        assert!(PromptSlot::SceneSplit.json_output());
        assert!(PromptSlot::ShotTable.json_output());
        assert!(!PromptSlot::ScreenplayFromSource.json_output());
        assert!(!PromptSlot::AdaptationPlan.json_output());
    }

    /// 润色那条以前明写「构图可以补」—— 正是它把「低角度平视构图」招来的。
    #[test]
    fn 润色不再邀请模型补构图() {
        let t = PromptSlot::StoryboardPolish.default_text();
        assert!(!t.contains("构图可以补"), "又把「构图可以补」写回去了");
    }

    #[test]
    fn 槽位_key_不重复() {
        let mut keys: Vec<_> = PromptSlot::ALL.iter().map(|s| s.key()).collect();
        keys.sort_unstable();
        let n = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), n);
    }

    /// **参考标签的格式两边必须一致。**
    ///
    /// LumenX 用 `[character1:小兔子]`，我们按名字解析。照抄它的提示词
    /// 会让模型吐出我们解析不了的标签 —— 那串方括号原样进引擎，
    /// 被当成画面内容画进去。出图了、不报错、画面里多一行怪字。
    #[test]
    fn r2v_提示词教的标签格式_我们自己能解析() {
        let text = PromptSlot::R2vPolish.default_text();
        assert!(
            !text.contains("[character1:") || text.contains("绝不要"),
            "默认文案在教模型用 characterN 格式"
        );
        // 文案里出现的示范标签必须真能被解析出来
        for tag in ["[角色:名字]", "[场景:名字]", "[道具:名字]"] {
            assert!(text.contains(tag), "缺少示范标签 {tag}");
            assert_eq!(
                crate::assets::parse_asset_refs(tag).len(),
                1,
                "示范标签 {tag} 我们自己解析不出来"
            );
        }
    }

    #[test]
    fn 出图润色的长度上限逐引擎() {
        // SDXL 的 CLIP 是 77 token 分块取平均，超了会被摊薄。
        assert_eq!(polish_max_chars("sdxl"), 150);
        // 长文本原生的那几个放开。
        for e in ["zimage", "krea2", "qwen21"] {
            assert!(polish_max_chars(e) > 1000, "{e} 被按 SDXL 的规矩限住了");
        }
    }

    #[test]
    fn 占位符替换_没接上的原样留着() {
        let out = fill("A {X} B {Y}", &[("X", "1")]);
        assert_eq!(
            out, "A 1 B {Y}",
            "没接上的占位符被清空了 —— 那样只表现为「AI 好像没看到」，看不出是哪里断了"
        );
    }

    // ---- 存取 -----------------------------------------------------------

    #[test]
    fn 没改过时用内置默认() {
        let cfg = PromptConfig::default();
        assert_eq!(
            cfg.text_for(PromptSlot::VideoPolish),
            PromptSlot::VideoPolish.default_text()
        );
        assert!(!cfg.is_overridden(PromptSlot::VideoPolish));
    }

    #[test]
    fn 改过之后整段替换_并且能存回读出() {
        let dir = tmp();
        let mut cfg = PromptConfig::default();
        cfg.overrides
            .insert("video_polish".into(), "我自己的提示词".into());
        save(dir.path(), &cfg).unwrap();

        let back = load(dir.path());
        assert_eq!(back.text_for(PromptSlot::VideoPolish), "我自己的提示词");
        assert!(back.is_overridden(PromptSlot::VideoPolish));
        // 别的槽位不受影响
        assert_eq!(
            back.text_for(PromptSlot::R2vPolish),
            PromptSlot::R2vPolish.default_text()
        );
    }

    #[test]
    fn 空白覆盖当没覆盖() {
        let mut cfg = PromptConfig::default();
        cfg.overrides.insert("video_polish".into(), "   \n ".into());
        assert!(!cfg.is_overridden(PromptSlot::VideoPolish));
        assert_eq!(
            cfg.text_for(PromptSlot::VideoPolish),
            PromptSlot::VideoPolish.default_text()
        );
    }

    #[test]
    fn 和默认一模一样的覆盖不落盘() {
        // 存了的话就钉死在这一版默认上 —— 以后改进默认，用户那份不会跟着走，
        // 而他并不知道自己「覆盖」过。
        let dir = tmp();
        let mut cfg = PromptConfig::default();
        cfg.overrides.insert(
            "video_polish".into(),
            PromptSlot::VideoPolish.default_text().to_string(),
        );
        save(dir.path(), &cfg).unwrap();
        assert!(!load(dir.path()).is_overridden(PromptSlot::VideoPolish));
    }

    /// **默认变了要认得出来。**
    ///
    /// 覆盖是整份替换。内置默认后来改进了（加规则、修判据），用户那份
    /// 永远停在老版本上而不自知 —— 完全不报错，只表现为
    /// 「别人都修好了的毛病我这儿还有」。存覆盖时记下当时的默认指纹。
    #[test]
    fn 默认变了认得出来_而且不自动改用户的文本() {
        let 旧默认 = "旧版默认：只写一条规则";
        let 新默认 = "新版默认：写了三条规则";
        assert_ne!(text_hash(旧默认), text_hash(新默认));
        // 指纹和当前默认一致 = 没变；不一致 = 变了
        assert_eq!(text_hash(旧默认), text_hash("  旧版默认：只写一条规则  "), "首尾空白不该算改动");
    }

    /// **没记过指纹的老覆盖当成「没变」。**
    ///
    /// 一次性把所有老覆盖标成过期，用户看到的是一屏警告而不是一条提示，
    /// 结果是整片忽略 —— 这个仓库在「一加键全部误报」上栽过两次。
    #[test]
    fn 老覆盖没有指纹时不报过期() {
        let mut cfg = PromptConfig::default();
        cfg.overrides.insert("video_polish".into(), "我改过的".into());
        // default_hash 是空的（老配置文件就是这样）
        assert!(cfg.default_hash.is_empty());
        let changed = cfg
            .default_hash
            .get("video_polish")
            .is_some_and(|h| h != &text_hash(PromptSlot::VideoPolish.default_text()));
        assert!(!changed, "没记过指纹的老覆盖不该被报成过期");
    }

    /// 导出再导入要回到同一个状态 —— 换机器、回滚全靠它。
    #[test]
    fn 导出导入是一个来回() {
        let mut cfg = PromptConfig::default();
        cfg.overrides.insert("video_polish".into(), "我改过的".into());
        cfg.default_hash.insert("video_polish".into(), text_hash(PromptSlot::VideoPolish.default_text()));
        let json = serde_json::to_string_pretty(&cfg).unwrap();
        let back: PromptConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.overrides, cfg.overrides);
        assert_eq!(back.default_hash, cfg.default_hash);
    }

    /// **老配置文件（没有 default_hash 字段）要读得进来**，不能整份失败 ——
    /// serde 报错只指向一个字段，后果却是所有提示词覆盖一起消失。
    #[test]
    fn 老配置文件没有指纹字段也读得进来() {
        let cfg: PromptConfig = serde_json::from_str(r#"{"overrides":{"video_polish":"我改过的"}}"#).unwrap();
        assert_eq!(cfg.overrides.get("video_polish").map(String::as_str), Some("我改过的"));
        assert!(cfg.default_hash.is_empty());
    }

    /// **占位符没填干净就别发。** 原样进模型的话它照常吐出一份东西、
    /// 任务显示成功，而那份东西缺了一整块输入。
    #[test]
    fn 没填上的占位符查得出来() {
        let t = PromptSlot::EntityExtraction.default_text();
        assert!(missing_placeholders(PromptSlot::EntityExtraction, t, &[("TEXT", "剧本")]).is_empty());
        // 调用方没传 → 会原样进模型
        assert_eq!(missing_placeholders(PromptSlot::EntityExtraction, t, &[]), ["{TEXT}"]);
        // **用户把占位符删了/拼错了 → 那一整块输入丢了，以前查不出来**
        assert_eq!(missing_placeholders(PromptSlot::EntityExtraction, "读一下剧本", &[("TEXT", "x")]), ["{TEXT}"]);
        assert_eq!(missing_placeholders(PromptSlot::EntityExtraction, "读一下 {TEXXT}", &[("TEXT", "x")]), ["{TEXT}"]);
    }

    /// **只查这个槽位声明过的占位符。** 模板里的 JSON 例子全是花括号，
    /// 无差别查会把例子报成问题。
    #[test]
    fn 模板里的_json_例子不算占位符() {
        // 模板里的 JSON 例子不算占位符（这个模板里 SCENE_TEXT 等都在，只是例子多了花括号）
        let t = format!("{}\n照这个例子：{{\"shots\":[{{\"duty\":\"x\"}}]}}", PromptSlot::ShotTable.default_text());
        let all: Vec<(&str, &str)> = PromptSlot::ShotTable.placeholders().iter().map(|ph| (ph.trim_matches(['{', '}']), "x")).collect();
        assert!(missing_placeholders(PromptSlot::ShotTable, &t, &all).is_empty(), "把 JSON 例子当成占位符了");
    }

    /// **13 个槽位的试跑变量都得齐。**
    ///
    /// 2026-09-19：试跑给所有槽位共用五个变量，而 7 个槽位要更多。
    /// 本来只是「试跑结果不像真的」，但同一天我给 `run` 加了占位符拦截之后，
    /// **那 7 个的试跑直接报错、一次都跑不起来**（Codex Astra 抓到）。
    ///
    /// 这是「修一处而不扫同类」：给生产路径加了拦截，没想试跑喂的是什么。
    /// 所以这条测试盯的不是某一个槽位，是**每一个**。
    #[test]
    fn 每个槽位的试跑变量都齐() {
        for slot in PromptSlot::ALL {
            let owned = try_vars(slot, "老茶师在窗边擦茶盏，雨天午后");
            let vars: Vec<(&str, &str)> = owned.iter().map(|(k, v)| (*k, v.as_str())).collect();
            let missing = missing_placeholders(slot, slot.default_text(), &vars);
            assert!(missing.is_empty(), "「{}」试跑缺变量：{missing:?}", slot.label());
        }
    }

    /// 试跑给的样例值要**像真的** —— 空串糊弄过去的话，
    /// 试跑出来的东西看着合格、换到生产就散。
    #[test]
    fn 试跑的样例值不是空壳() {
        let v = try_vars(PromptSlot::ShotTable, "剧本");
        let by = |k: &str| v.iter().find(|(x, _)| *x == k).map(|(_, s)| s.as_str()).unwrap_or("");
        assert!(by("ASSETS").contains("有图"), "资产清单要像真的：{}", by("ASSETS"));
        assert!(by("SHOT_SIZES").contains('='), "词表要是真的取值表：{}", by("SHOT_SIZES"));
        assert!(!by("MIN_SECS").is_empty() && !by("MAX_SECS").is_empty());
    }

    /// **正文里本来就有「{TEXT}」不该被误报。**
    ///
    /// 原来扫的是**填完之后的正文**：讲程序、讲模板的剧本里写到这几个字，
    /// 就被当成「没填上」—— 而这条是**直接拒绝发送**的，
    /// 用户的正常剧本会让整个生成起不来（2026-09-19 Codex Astra 抓到）。
    #[test]
    fn 正文里含占位符字样不误报() {
        let t = PromptSlot::EntityExtraction.default_text();
        let vars = [("TEXT", "他说：变量写成 {TEXT} 就行")];
        assert!(missing_placeholders(PromptSlot::EntityExtraction, t, &vars).is_empty());
    }

    /// **嵌套 JSON 模板不能被吞掉。**
    ///
    /// 原来「找最近的 `}`，认不出就整段吐掉」—— `{"script":"{TEXT}"}` 里
    /// 外层 `{` 配上了内层的 `}`，整段被当成一个认不出的键，
    /// 里面的 `{TEXT}` 一个字都没换（当场复现过）。
    #[test]
    fn 嵌套的花括号里占位符照样替换() {
        assert_eq!(fill(r#"{"script":"{TEXT}"}"#, &[("TEXT", "正文")]), r#"{"script":"正文"}"#);
        assert_eq!(fill("{a{TEXT}b}", &[("TEXT", "x")]), "{axb}");
    }

    /// **插进去的内容不再参与后面的替换。**
    ///
    /// 原来按变量顺序多次 `replace`：剧本里如果写着 `{ASSETS}`（讲程序、
    /// 讲模板的剧本完全可能），它会被后面那一轮再替换一次 ——
    /// 用户的原文被悄悄改掉，而任务照常成功。
    #[test]
    fn 填进去的文本不会被二次替换() {
        let out = fill("原文：{TEXT}\n资产：{ASSETS}", &[("TEXT", "他说：变量写成 {ASSETS} 就行"), ("ASSETS", "老茶师")]);
        assert_eq!(out, "原文：他说：变量写成 {ASSETS} 就行\n资产：老茶师");
    }

    /// 认不出来的 `{XXX}` 原样留着 —— 模板里的 JSON 例子全是花括号，乱删会把例子改坏。
    #[test]
    fn 认不出来的花括号原样留着() {
        assert_eq!(fill(r#"照这个例子：{"shots":[{"duty":"x"}]}，主体是 {A}"#, &[("A", "甲")]),
                   r#"照这个例子：{"shots":[{"duty":"x"}]}，主体是 甲"#);
        assert_eq!(fill("没闭合的 {AAA", &[("AAA", "x")]), "没闭合的 {AAA");
    }

    #[test]
    fn 配置文件不存在时回默认_不报错() {
        let dir = tmp();
        assert!(load(dir.path()).overrides.is_empty());
    }

    #[test]
    fn 配置文件坏了时回默认_不让整个功能挂掉() {
        let dir = tmp();
        std::fs::write(config_file(dir.path()), "{ 这不是 json").unwrap();
        assert!(load(dir.path()).overrides.is_empty());
    }

    /// 2026-09-30 EP02 一次进了 20 个龙套和一次性道具的资产。模型标 `incidental` 的入库默认不要；没标的照旧
    #[test]
    fn 顺带出现的标出来_没标的当不是() {
        let raw = r#"{"characters":[{"name":"邻居甲","incidental":true},{"name":"陆川"}],
                      "props":[{"name":"铁棍","incidental":true,"description":"锈铁棍"},{"name":"协议铭牌","incidental":false}]}"#;
        let es = parse_entities(raw);
        let get = |n: &str| es.iter().find(|e| e.name == n).unwrap().incidental;
        assert!(get("邻居甲") && get("铁棍"));
        assert!(!get("陆川") && !get("协议铭牌"));
        // 提示词里要真的让模型标、要写清哪些算顺带 —— 光有字段模型不会自己填
        assert!(ENTITY_EXTRACTION.contains("\"incidental\": false") && ENTITY_EXTRACTION.contains("没名没姓的龙套"));
    }

    /// **第 4 期的判据：改一个字，下一次生成真的用了改后的。**
    #[test]
    fn 改过的提示词会被真的发出去() {
        let mut cfg = PromptConfig::default();
        cfg.overrides.insert(
            "entity_extraction".into(),
            "只抽角色，忽略别的。文本：{TEXT}".into(),
        );
        let sent = compose(&cfg, PromptSlot::EntityExtraction, &[("TEXT", "从前有座山")]);
        assert_eq!(sent, "只抽角色，忽略别的。文本：从前有座山");
        assert!(!sent.contains("分镜师"), "还带着内置默认的影子，覆盖没生效");
    }

    #[test]
    fn 没改过时发出去的是内置默认_且占位符填好了() {
        let sent = compose(
            &PromptConfig::default(),
            PromptSlot::EntityExtraction,
            &[("TEXT", "从前有座山")],
        );
        assert!(sent.contains("从前有座山"));
        assert!(!sent.contains("{TEXT}"), "占位符没填");
    }

    // ---- 解析实体 --------------------------------------------------------

    #[test]
    fn 三类实体都抽得出来() {
        let raw = r#"{"characters":[{"name":"老茶师","description":"白须"}],
                      "scenes":[{"name":"老茶馆","description":"雨天"}],
                      "props":[{"name":"铜壶","description":"发暗"}]}"#;
        let es = parse_entities(raw);
        assert_eq!(es.len(), 3);
        assert_eq!(es[0].kind, "character");
        assert_eq!(es[1].kind, "scene");
        assert_eq!(es[2].kind, "prop");
    }

    /// **给了槽位就用槽位渲染**，模型自己那段自由描述作废。
    ///
    /// 两份各写各的话，界面上改了槽位而出图用的还是旧那段，而且不报错。
    /// **提到人的那个分句要摘掉，别的一个字不动。**
    ///
    /// 提示词里劝过三轮都没拦住（见 `strip_people` 的注释），
    /// 这是第四次撞上「规则条文不如结构」。
    #[test]
    fn 场景描述里的人被摘掉() {
        let got = strip_people("中央设龙椅，两侧百官列立，地面铺石砖");
        assert_eq!(got, "中央设龙椅，地面铺石砖");
    }

    /// 伪装成光影描写的也摘 —— 这一句真溜过去过。
    #[test]
    fn 伪装成光影的人脸也摘() {
        let got = strip_people("光线昏暗，蓝光映照在桌面与人脸上，氛围压抑");
        assert_eq!(got, "光线昏暗，氛围压抑");
    }

    /// **没有人的描述一个字都不能动。** 重拼会把句号和分号抹平。
    #[test]
    fn 没有人的描述原样保留() {
        let src = "内景，盛唐大殿。高约15米、宽约30米，殿宇巍峨。地面铺石砖";
        assert_eq!(strip_people(src), src);
    }

    /// **全都提到人时保留原文。** 那说明这条判据在这段话上不成立，
    /// 宁可放过去让人看见，也不要交出一个空描述。
    #[test]
    fn 全是人时不交空描述() {
        let src = "百官列立，群臣震动";
        assert_eq!(strip_people(src), src);
    }

    /// **别误伤。** 「无人机」「人民广场」里的「人」是器物和地名的一部分，
    /// 画出来不是一个人 —— 所以判据是词表不是单字。
    ///
    /// **末尾那个不含「人」的分句是必须的。** 第一版没有它，两个分句
    /// 都含「人」，判据换成单字时全被摘光、触发「全摘就保留原文」的兜底
    /// —— 于是测试照样绿。变异测试当场抓出来的假绿。
    #[test]
    fn 不误伤带人字的物和地名() {
        let src = "广场上停着一架无人机，远处是人民广场的牌坊，地面铺着青砖";
        assert_eq!(strip_people(src), src);
    }

    /// 模型把槽位名原样交回来（`"eyes": "眼睛"`）不能进描述 —— 2026-09-26 实测撞上，
    /// 程远的描述末尾一句光秃秃的「眼睛，肤色」。
    #[test]
    fn 槽位名原样交回来的不进描述() {
        let raw = r#"{"characters":[{"name":"程远","look":{"role":"执行总裁","gender":"男性","face":"五官立体","eyes":"眼睛","skin":"肤色"},
                      "outfit":{"top":"白色西装"}}]}"#;
        let es = parse_entities(raw);
        let d = &es[0].description;
        assert!(d.contains("五官立体") && d.contains("白色西装"), "{d}");
        assert!(!d.contains("眼睛") && !d.contains("肤色"), "槽位名进了描述：{d}");
    }

    /// 场景走这条路，角色和道具不走 —— 角色描述里当然会有人。
    #[test]
    fn 只对场景摘人() {
        let raw = r#"{"scenes":[{"name":"金銮殿","description":"中央设龙椅，两侧百官列立"}],
                      "characters":[{"name":"皇帝","description":"盛唐帝王，男性"}]}"#;
        let es = parse_entities(raw);
        let scene = es.iter().find(|e| e.kind == "scene").unwrap();
        let ch = es.iter().find(|e| e.kind == "character").unwrap();
        assert_eq!(scene.description, "中央设龙椅");
        assert_eq!(ch.description, "盛唐帝王，男性", "角色描述不该被摘");
    }

    /// **剧本开头的元信息标签不是角色。**
    ///
    /// 实拍：用户拿带 Markdown 头的剧本跑抽取，六个「角色」里三个是
    /// `**类型**` `**时长建议**` `**基调**`。三个假角色会一路走下去 ——
    /// 出设定卡、进资产库、被分镜引用，全程不报错。
    #[test]
    fn 加粗的元信息标签不算实体() {
        let raw = r#"{"characters":[
            {"name":"**类型**","description":"古风"},
            {"name":"**时长建议**","description":"60-90秒"},
            {"name":"**基调**","description":"热血"},
            {"name":"林婉清（现代）","description":"白领"},
            {"name":"皇帝","description":"帝王"}
        ]}"#;
        let got: Vec<String> = parse_entities(raw).iter().map(|e| e.name.clone()).collect();
        assert_eq!(got, vec!["林婉清（现代）", "皇帝"]);
    }

    /// 整行被当成名字的也拦（`类型：古风`）。
    #[test]
    fn 带冒号的整行不算实体() {
        let raw = r#"{"props":[{"name":"类型：古风"},{"name":"龙椅"},{"name":"tier: a"}]}"#;
        let got: Vec<String> = parse_entities(raw).iter().map(|e| e.name.clone()).collect();
        assert_eq!(got, vec!["龙椅"]);
    }

    /// **别误伤真名字。** 括号、间隔号、书名号、数字都是正常人名地名的一部分。
    #[test]
    fn 正常的名字不被误拦() {
        let raw = r#"{"characters":[
            {"name":"林婉清（男装官服）"},{"name":"小花（三花猫）"},{"name":"零"}
        ],"scenes":[{"name":"S01E01 金銮殿"},{"name":"长安·西市"}],
          "props":[{"name":"《将进酒》手稿"}]}"#;
        assert_eq!(parse_entities(raw).len(), 6);
    }

    #[test]
    fn 有槽位时描述从槽位渲染() {
        let raw = r#"{"characters":[{
            "name":"现代林婉清",
            "description":"这段自由描述应该被丢掉",
            "look":{"role":"现代白领","gender":"女性","age":"28岁","height":"165厘米",
                    "build":"体型偏瘦","hair":"齐肩短发，发质略显干枯","hair_color":"黑发",
                    "face":"瓜子脸","eyes":"黑瞳","skin":"肤色苍白"},
            "outfit":{"top":"白色微褶皱衬衫","bottom":"黑色包臀裙","shoes":"黑色皮鞋",
                      "headwear":"","accessories":""},
            "anchors":{"marks":["黑眼圈明显","眼角微垂"]}
        }]}"#;
        let e = &parse_entities(raw)[0];
        assert!(!e.description.contains("应该被丢掉"), "{}", e.description);
        assert!(e.description.contains("描述：现代白领，女性，28岁，身高165厘米"), "{}", e.description);
        // 空的帽子和配饰要明说 —— 不说模型自己发一顶
        assert!(e.description.contains("无帽，无配饰"), "{}", e.description);
        assert!(e.description.contains("特征：黑眼圈明显，眼角微垂"), "{}", e.description);
        assert!(e.spec.is_some(), "槽位没存下来，以后改一个槽就只能重写整段");
    }

    /// 没给槽位时**走老路**，不要因此把这个角色整个丢掉。
    /// 旧提示词跑出来的数据、或者模型偷懒，都会走到这儿。
    #[test]
    fn 没给槽位时仍然用自由描述() {
        let raw = r#"{"characters":[{"name":"老茶师","description":"白须","age":"六十岁"}]}"#;
        let e = &parse_entities(raw)[0];
        assert!(e.description.contains("白须"), "{}", e.description);
        assert!(e.description.contains("六十岁"), "{}", e.description);
        assert!(e.spec.is_none());
    }

    /// **场景和道具没有槽位。** 给它们塞一个 `look` 也不该认 ——
    /// 认了的话场景的描述会被渲染成一句「。」。
    #[test]
    fn 场景不认角色槽位() {
        let raw = r#"{"scenes":[{"name":"金銮殿","description":"室内，宏观尺度",
                                 "look":{"eyes":"黑瞳"}}]}"#;
        let e = &parse_entities(raw)[0];
        assert_eq!(e.description, "室内，宏观尺度");
        assert!(e.spec.is_none());
    }

    /// 槽位全空时当没给 —— 空壳会让界面显示「已有外观规格」。
    #[test]
    fn 空槽位当没给() {
        let raw = r#"{"characters":[{"name":"甲","description":"白须",
                      "look":{},"outfit":{}}]}"#;
        let e = &parse_entities(raw)[0];
        assert!(e.spec.is_none(), "全空的槽位不该存下来");
        assert_eq!(e.description, "白须", "不该被渲染成「无帽，无配饰」");
    }

    #[test]
    fn 年龄性别服装拼进描述() {
        // 它们都是「长期固定的外形」，本来就属于描述。
        let raw = r#"{"characters":[{"name":"老茶师","description":"白须",
                       "age":"六十岁","gender":"男","clothing":"青灰长衫"}]}"#;
        let d = &parse_entities(raw)[0].description;
        for w in ["白须", "六十岁", "男", "青灰长衫"] {
            assert!(d.contains(w), "「{w}」丢了：{d}");
        }
    }

    #[test]
    fn 没有名字的丢掉_但不拖垮整批() {
        // 没有名字的资产没法被 @ 引用，建了也用不上；
        // 但不能因为一条坏的就整批失败。
        let raw = r#"{"characters":[{"description":"没名字"},{"name":"老茶师"},
                                    {"name":"   "}]}"#;
        let es = parse_entities(raw);
        assert_eq!(es.len(), 1);
        assert_eq!(es[0].name, "老茶师");
    }

    #[test]
    fn 同名同类只留一条() {
        let raw = r#"{"characters":[{"name":"老茶师"},{"name":"老茶师"}]}"#;
        assert_eq!(parse_entities(raw).len(), 1);
    }

    #[test]
    fn 模型包了_markdown_围栏也能解析() {
        let raw = "```json\n{\"characters\":[{\"name\":\"老茶师\"}]}\n```";
        assert_eq!(parse_entities(raw).len(), 1);
    }

    #[test]
    fn 少返回几类不算失败() {
        assert_eq!(parse_entities(r#"{"characters":[{"name":"甲"}]}"#).len(), 1);
    }

    #[test]
    fn 完全不是_json_时回空而不是崩掉() {
        assert!(parse_entities("模型今天不想说话").is_empty());
        assert!(parse_entities("").is_empty());
    }

    #[test]
    fn persona_带出来_同一个人的不同造型才能归组() {
        let raw = r#"{"characters":[{"name":"张三（便装）","persona":"张三"},
                                    {"name":"张三（婚纱）","persona":"张三"}]}"#;
        let es = parse_entities(raw);
        assert_eq!(es.len(), 2);
        assert!(es.iter().all(|e| e.persona == "张三"));
    }

    // ---- 从模型输出里抠 JSON ---------------------------------------------

    #[test]
    fn 抠得出裸_json() {
        assert_eq!(extract_json(r#"{"a":1}"#), Some(r#"{"a":1}"#));
    }

    #[test]
    fn 抠得出_markdown_围栏里的() {
        // 本地小模型最常见的不老实形式
        let raw = "```json\n{\"a\":1}\n```";
        assert_eq!(extract_json(raw), Some(r#"{"a":1}"#));
    }

    #[test]
    fn 抠得出前后有废话的() {
        let raw = "好的，这是结果：\n{\"a\":1}\n希望有帮助！";
        assert_eq!(extract_json(raw), Some(r#"{"a":1}"#));
    }

    #[test]
    fn 抠得出数组() {
        assert_eq!(extract_json("前言 [1,2] 后语"), Some("[1,2]"));
    }

    #[test]
    fn 嵌套和字符串里的花括号不会截断() {
        let raw = r#"{"a":{"b":"}"},"c":[1,{"d":2}]}"#;
        assert_eq!(extract_json(raw), Some(raw));
    }

    #[test]
    fn 字符串里的转义引号不会把状态弄乱() {
        let raw = r#"{"a":"他说\"你好\"","b":1}"#;
        assert_eq!(extract_json(raw), Some(raw));
    }

    #[test]
    fn 没有_json_时回_none_而不是乱截一段() {
        assert_eq!(extract_json("模型今天不想说话"), None);
        assert_eq!(extract_json(r#"{"a":1"#), None, "没闭合的不该当成有效");
    }
}

// ---------------------------------------------------------------------------
// 提取实体
// ---------------------------------------------------------------------------

/// 从剧本里抽出来的一条，**还没入库**。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtractedEntity {
    /// "character" | "scene" | "prop"
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub persona: String,
    /// 项目资产库里已经有同名同类的了。**默认不勾**，免得重复建。
    #[serde(default)]
    pub exists: bool,
    /// 身份锚点（辨识标记 / Hex 色值 / 专属负向）。**只有角色有。**
    ///
    /// 形容词锁不住长相 —— 「清秀的东方女性」对应一大片可能性，
    /// 每次出图落在哪儿看运气，这就是长相漂移的来源。
    /// 锚点是判别性强的那些：一颗痣的具体位置、一个十六进制色值。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchors: Option<crate::cinema_dna::CharacterVisualDna>,
    /// 只有声音、没有身体。**只有角色有。** 见
    /// [`crate::assets::CharacterTraits::disembodied`]。
    #[serde(default)]
    pub disembodied: bool,
    /// **这一集顺带出现的**：龙套（邻居甲、打手乙）、一次性的普通物件（铁棍、电锅）。
    /// 入库弹窗里默认「不要」，镜头里按文字描述出 —— 资产库是按整部剧的设定建的，
    /// 这些不需要单独建资产（用户 2026-09-30：EP02 一次进了 20 个这样的资产）。模型没标就是 `false`
    #[serde(default)]
    pub incidental: bool,
    /// 槽位化的长相和服装。**只有角色有。** 见 [`crate::character_spec`]。
    ///
    /// `description` 是**从它渲染出来的**，不是模型另外写的一段 ——
    /// 两份各写各的话，界面上改了槽位而出图用的还是旧那段，
    /// 而且不报错。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec: Option<crate::character_spec::CharacterSpec>,
}

/// 场景描述里提到人的那个分句，**摘掉**。
///
/// # 为什么是摘不是劝
///
/// 场景资产出的是**空场景参考图**，描述里有人就会画出人来，而那个人
/// 会跟着这张参考图进入每一镜。提示词里劝过三轮，一轮比一轮具体：
///
/// 1. 「场景描述里不要写人」—— 抽出来「角落悬浮着零的光点」。
/// 2. 加了那条反例 —— 抽出来「蓝光映照在桌面与**人脸**上」，
///    伪装成光影描写溜过去了。
/// 3. 把「人脸」和「百官列立两侧」都写成反例 —— 照样抽出
///    「两侧百官列立」，只是把词序调了个个儿。
///
/// 这是这个仓库第四次撞上同一条：**规则条文拦不住，结构才行。**
///
/// # 为什么按分句摘而不是整句丢
///
/// 中文这类描述是逗号串起来的并列分句，「中央设龙椅，两侧百官列立，
/// 地面铺石砖」摘掉中间一节读起来完全通顺。整句丢会把龙椅和石砖
/// 一起丢掉 —— 那两样正是这个场景的锚点。
///
/// **全都要摘时保留原文。** 那说明这条判据在这段话上不成立
/// （或者这个"场景"根本就是个人物），宁可放过去让人看见，
/// 也不要交出一个空描述。
pub(crate) fn strip_people(desc: &str) -> String {
    /// 出现即判定这一分句在写人。
    ///
    /// **只收会被画成人的词。** 「人民广场」「无人机」这类里的「人」
    /// 是地名和器物的一部分，所以不能光看单字 —— 词表里的每一条
    /// 都是「画出来就是一个人」。
    const PEOPLE: &[&str] = &[
        "百官", "群臣", "人群", "人影", "身影", "人脸", "路人", "行人",
        "侍卫", "宫女", "太监", "士兵", "围观", "众人", "人物", "剪影",
    ];
    let parts: Vec<&str> = desc.split(['，', '。', '；']).collect();
    let kept: Vec<&str> = parts
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty() && !PEOPLE.iter().any(|w| p.contains(w)))
        .collect();
    if kept.is_empty() {
        return desc.trim().to_string();
    }
    // 原样没变就别重拼 —— 重拼会把原来的句号和分号都抹平。
    if kept.len() == parts.iter().filter(|p| !p.trim().is_empty()).count() {
        return desc.trim().to_string();
    }
    kept.join("，")
}

/// 这个名字其实是**剧本开头那段元信息的标签**，不是实体。
///
/// # 实拍
///
/// 用户拿一段带 Markdown 头的剧本跑抽取，抽出六个「角色」，其中三个是：
///
/// ```text
/// **类型**    **时长建议**    **基调**
/// ```
///
/// 来源是剧本最前面那几行：
///
/// ```text
/// **类型**：古风 / 穿越 / 爽文漫剧
/// **时长建议**：60–90秒
/// **基调**：热血 × 盛唐史诗感 × 爽点节奏
/// ```
///
/// 模型把加粗的标签当成了人名。**三个假角色会一路走下去** ——
/// 出设定卡、进资产库、被分镜用 `[角色:**类型**]` 引用，全程不报错。
///
/// # 判据是星号，不是词表
///
/// 拦的是「名字里有 `*`」这个**形状**，不是「类型/基调/时长」这几个词。
/// 列词表的话，换一份模板换一套标签就漏了；而**真实的人名、地名、
/// 道具名里不会有星号** —— 那是 Markdown 的强调记号，不是名字的一部分。
///
/// 顺带也拦冒号：`类型：古风` 这种整行被当成名字的情况同样不是实体。
fn looks_like_metadata(name: &str) -> bool {
    name.contains('*') || name.contains('：') || name.contains(':')
}

/// 解析模型返回的实体 JSON。
///
/// **对模型的输出保持怀疑**：字段可能缺、可能是 null、可能多返回几类。
/// 缺名字的直接丢掉（没有名字的资产没法被 `@` 引用，建了也用不上），
/// 但不要因为一条坏的就整批失败。
pub fn parse_entities(raw: &str) -> Vec<ExtractedEntity> {
    let Some(json) = extract_json(raw) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (key, kind) in [
        ("characters", "character"),
        ("scenes", "scene"),
        ("props", "prop"),
    ] {
        let Some(arr) = v.get(key).and_then(|x| x.as_array()) else {
            continue;
        };
        for item in arr {
            let name = item
                .get("name")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .trim()
                .to_string();
            if name.is_empty() || looks_like_metadata(&name) {
                continue;
            }
            // **角色的描述是从槽位渲染出来的。**
            //
            // 以前是模型自己写一段，再把 age / gender / clothing 拼在后面 ——
            // 实测拼出来的是「……常穿简单的便装，25-30岁，简单朴素的日常便装」：
            // 服装两遍、年龄夹在中间、而身高脸型瞳色一个都没有。
            // 现在槽位是唯一的事实来源，`description` 只是它的渲染结果。
            let spec: Option<crate::character_spec::CharacterSpec> = (kind == "character")
                .then(|| {
                    serde_json::from_value::<crate::character_spec::CharacterSpec>(
                        serde_json::json!({
                            "look": item.get("look").cloned().unwrap_or(serde_json::Value::Null),
                            "outfit": item.get("outfit").cloned().unwrap_or(serde_json::Value::Null),
                        }),
                    )
                    .ok()
                    .map(|mut c| {
                        c.look.drop_label_echoes();
                        c
                    })
                    .filter(|c| !c.is_empty())
                })
                .flatten();

            // 描述以外的几个字段（年龄/性别/服装）拼进描述 ——
            // 它们都是「长期固定的外形」，本来就属于描述。
            // **只在模型没给槽位时才走这条老路**（旧提示词、或者它偷懒了）。
            let mut desc = item
                .get("description")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .trim()
                .to_string();
            for extra in ["age", "gender", "clothing"] {
                if let Some(s) = item.get(extra).and_then(|x| x.as_str()) {
                    let s = s.trim();
                    // **「不适用」不是外形。**
                    //
                    // 模型表达「这一项没有」用的是「无」这个字 —— 猫没有服装，
                    // 它就填「无」。判据只有「非空」的话，`，无` 会被当成
                    // 一条外貌特征拼进描述、再随描述进出图提示词。
                    // 实测抓到过：`小花（三花猫）` 的描述末尾就是「，雌，无」。
                    if s.is_empty() || is_placeholder(s) || desc.contains(s) {
                        continue;
                    }
                    if !desc.is_empty() {
                        desc.push('，');
                    }
                    desc.push_str(s);
                }
            }
            if out
                .iter()
                .any(|e: &ExtractedEntity| e.kind == kind && e.name == name)
            {
                continue;
            }
            let anchors = if kind == "character" {
                item.get("anchors")
                    .and_then(|a| {
                        serde_json::from_value::<crate::cinema_dna::CharacterVisualDna>(a.clone()).ok()
                    })
                    .filter(|d| !d.is_empty() || !d.avoid.is_empty())
            } else {
                None
            };
            // **场景描述里提到人的分句摘掉。** 空场景参考图里多一个人，
            // 会跟着这张图进入每一镜。见 `strip_people`。
            if kind == "scene" {
                desc = strip_people(&desc);
            }
            // 有槽位就用槽位渲染，特征取身份锚点里的 `marks`。
            if let Some(sp) = &spec {
                desc = sp.description_block(
                    anchors.as_ref().map(|a| a.marks.as_slice()).unwrap_or(&[]),
                );
            }
            out.push(ExtractedEntity {
                kind: kind.to_string(),
                name,
                description: desc,
                persona: item
                    .get("persona")
                    .and_then(|x| x.as_str())
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
                exists: false,
                // 只有角色有锚点。模型没给或者给歪了就当没有 ——
                // **不要编一个出来**：编出来的痣会被画进每一张图。
                anchors,
                // 道具和场景不会「没有身体」—— 这个字段只对角色有意义。
                disembodied: kind == "character"
                    && item
                        .get("disembodied")
                        .and_then(|x| x.as_bool())
                        .unwrap_or(false),
                incidental: item.get("incidental").and_then(|x| x.as_bool()).unwrap_or(false),
                spec,
            });
        }
    }
    out
}

/// 剧本 + 场景名 → 空间契约。
///
/// # 三条设计是抄来的，不是想出来的
///
/// - **坐标无关**（DramaClaw `scene_spatial_contract` v8）：位置写成拓扑
///   关系。模型守不住像素坐标，守得住「在柜台正后方」。
/// - **负向约束是一等公民**：真实失败样子是「同一扇门被画成左右各一扇」，
///   正向描述压不住它，得有一句明说「入口只有 1 处」。
/// - **诚实区分看得见的和编的**：剧本没写的可以补，但要进 `inferred`，
///   不能混进锚点表当既成事实。
///
/// # 为什么要求「两两关系 + 视线终点」
///
/// 本机实测：视角描述写成「面向主要活动区的正面机位」时，四格里的
/// 「全景」和「反打」出成了几乎同一个机位 —— 光向材质全对，
/// **柜台在第 2 格有、第 3 格同角度却没有**。写成
/// 「从临街木门内侧朝店堂深处看，尽头是靠后墙的柜台」就不会。
pub const SCENE_CONTRACT_EXTRACTION: &str = r#"你是空间连续性分析师。读下面的剧本，为场景「{SCENE}」写一份**空间契约**。

这不是写画面描述，是**画一张文字地图**：后面所有用到这个场景的镜头都要
共享这份地理，所以它必须能被反复核对。

## 只输出 JSON，不要任何解释文字

```json
{
  "shape": "整体形状和尺度，一句话。例：东西进深的狭长一层店铺",
  "zones": ["功能分区，3~6 个"],
  "entrances": [{"name": "东侧卷帘门", "connects_to": "街道"}],
  "anchors": [
    {
      "name": "搬不走的东西",
      "zone": "它在哪个分区（拓扑名，不要写坐标）",
      "size": "small|medium|large|wall_dominant",
      "must_not_become": ["它最容易被画错成什么"]
    }
  ],
  "relations": ["两两关系。例：后厨出餐口在柜台正后方"],
  "materials": ["对辨认结构有用的材质和主次色"],
  "views": [
    {
      "name": "短名，4 字以内",
      "from": "站在哪儿",
      "toward": "朝哪个方向看",
      "far": "视线尽头是什么",
      "visible": ["这个方向同时能看见哪些锚点（必须是上面 anchors 里的名字）"],
      "light": "主光来源和方向"
    }
  ],
  "inferred": ["剧本没写、你补上去的部分"],
  "no_extra": ["不要凭空多出来的东西"],
  "empty_stage": "required"
}
```

## 硬要求

1. **锚点的判据是「搬得走吗」，不是「显眼吗」。** 临时的箱子、海报、
   积水不是锚点 —— 把临时陈设写成永久地理，下一场戏就会自相矛盾。
   `anchors` 控制在 8 个以内，宁可少写也不要把装饰当锚点。

2. **`relations` 写两两关系，不是清单。** 「有柜台、有门」每一格都能满足，
   却可以把它们摆在完全不同的地方；「门正对柜台」不能。

3. **左 / 右必须带参照系**：写「画面左侧」或「场内西侧」。
   光写「左边的门」换个机位就跑到右边了，而两种读法画出来都「对」。

4. **`views` 只写下游会反复引用的方向**，一般 3~6 个。
   「低机位」「门把手特写」是单个镜头的构图，不要建 View。
   每个 View 的 `far`（视线尽头）必须互不相同 —— 这一项最能把两个机位
   区分开，写得笼统的话它们会出成同一个角度。

5. **同一场景的所有 View 共享同一套光**：同一个窗、同一组灯具。
   换方向只改变我们从哪一侧看它，不改变它是什么。
   逆光顺光的差别、朝窗背窗的亮度差是正常的，照写。

6. **身份不含状态。** 时段、天气、停电、演员站位**都不进** `shape` /
   `zones` / `anchors` —— 那些变了，空间还是同一个空间。

7. `must_not_become` 写具体的错误落点（「现代吧台」），
   不要写「不好看的柜台」这种没有落点的话。

8. 剧本没交代的地方，放进 `inferred`，**不要**写进 `anchors` 装成确定的。

剧本：
{TEXT}
"#;

/// 原文 → 改编方案。
///
/// # 它不写台词
///
/// 这一步只回答「怎么分」，不回答「怎么写」。混在一起的后果是
/// 模型一边分集一边编对白，两件事互相迁就 —— 分集被已经写出来的
/// 台词绑架，而台词又建立在一个还没想清楚的分集上。
///
/// drama-skills 把这一段拆成 `novel-analyze`（读懂）+ `develop`（定方案）
/// 两步。我们合成一步，但**保留它最要紧的那个产物**：
/// 一张说得清「哪几章 → 哪一集」的对照表。
pub const ADAPTATION_PLAN: &str = r#"你是一名资深剧本策划。把下面给到的原文改编成短剧，出一份**改编方案**。

**不要写台词，不要写剧本。** 这一步只回答「这些材料怎么变成几集」，
不回答「怎么写」。两件事混在一起的话，分集会被已经写出来的台词绑架。

## 章节和集数**不是**一对一

- 一章内容厚、有两个各自完整的转折 → 拆成 2 集甚至更多；
- 一章太薄（几百字、只有一个动作）→ **说出来**，建议和相邻章一起再规划，
  不要硬撑成一集；
- 几章讲的是同一件事 → 并成一集；
- 一章里有大段和主线无关的支线 → 放进「没用上」。

**由内容定，不由章数定。** 时长也一样：短剧一集通常 60~180 秒，
但那是常见值不是规矩 —— 一个只有一句台词的收束镜头可以是 20 秒。

## 只输出 Markdown，不要任何解释文字

**先判断材料够不够，然后走对应的那一种格式。两种格式不要混。**

### 格式甲：材料太薄，撑不起一集

只写这三段，**到此为止** —— 不要写 `## S01E01`，
不要写「这一集兑现什么：未知」这种填着占位符的分集区块。
一个填满「未知」的分集比没有分集更糟：下游会拿它去生成一堆空镜头。

```
# 改编方案 · <覆盖了哪些章>

## 这些材料讲了什么
一两句。

## 不够
这些材料太薄（说清薄在哪：字数、只有一个动作、没有转折……），
建议连着第 X 章一起规划。
```

### 格式乙：材料够，能出 N 集

```
# 改编方案 · <覆盖了哪些章>

## 这些材料讲了什么
两三句。**只讲给到的正文里真的发生的事**，不预告后面。

## 怎么分
- 能出几集：N 集
- 一集装不下时说清为什么要拆，每集各自的完整转折是什么。

## S01E01 · 集名
- **用了哪些材料**：第 X 章的哪一段（给了多章就写清哪几章）
- **大概多长**：X 秒（按内容估，不要凑整）
- **这一集兑现什么**：观众看完得到的那一下是什么
- **进入状态**：人物带着什么事实和压力进来
- **转折**：哪一件事使原计划失效
- **退出状态**：谁得到/失去了什么，为什么会想看下一集
- **发动机**：（只在材料里真的有时写）艰难选择 / 精确死线 / 有限次数 / 关键物证

## 没用上的部分
哪些段落没进任何一集，为什么（重复、支线、留给后面）。**一段都不能漏说。**

## 交接给下一段
下一章开始时，人物带着什么状态、什么未解的问题进去。一两句。
```

## 硬要求

1. **只规划给了正文的这些章。**
   目录里的其他章节**只有标题、没有正文** —— 不要为它们排集、
   不要写「用第02章」。原文里没给的内容一个字都不要编。

   这一条是实跑撞出来的：只给第 01 章时，模型顺着章末的铺垫
   规划了 12 集、标着「用第02章」，还断言「无遗漏章节」——
   那些集在下游拿不到原文，**根本没法执行**，而方案里写着一切正常。

2. **材料不够就走格式甲，到此为止。**
   十个字的一章撑不起任何一集。说了「撑不起」却还是写出一个
   字段全是「未知」的 `S01E01`，等于没说 —— 下游照样会拿它去生成空镜头。
   
   （这一条实跑撞过：规则里写着「停在这里」，但模板里 `## S01E01`
   明晃晃摆着，**模板赢了规则**。所以现在它是两种格式，不是一条规则。）

3. **「用了哪些材料」里只能写给了正文的章。**
   不要写「以及目录里尚未给正文的第03章」这种 —— 加了说明也不行。
   那一栏是给下游照着取原文的，写了取不到的东西就是让它去取空气。

4. **不要写大结局。** 目录里后面还有章节时，这一集的退出状态是
   「想看下一集」，不是「故事讲完了」。

5. **「发动机」那一项只在材料里真的有时写。** 没有倒计时就不要造一个：
   造出来之后写剧本那一步会顺着它编，整集建立在一个不存在的前提上。

6. 人名严格照原文，**不要改写、不要起外号** —— 下游靠名字把角色对上。

{NOTE}

原文：
{TEXT}
"#;
