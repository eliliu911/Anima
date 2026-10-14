//! 镜头表：分镜重构后每一镜的字段，以及对它的校验。
//!
//! 见 `docs/plans/2026-09-13-分镜重构方案.md`。这里只放**两条提示词线路共用**的东西：
//! 字段本身和校验。线路 A / B 各自怎么把字段写成提示词，在它们自己的模块里，
//! 不在这里 —— 共用拼装函数就不叫隔离了。
//!
//! # 字段从哪来
//!
//! 取线路 A（libtv 中文结构）和线路 B（H3 官方六段）需求的**并集**，
//! 写法照 `skills/film-director/reference/drama-skills/storyboard/SKILL.md`：
//! 每镜唯一职责、来源、起点 → 唯一动作 → 终点；参考图槽位写清用途 / 控制 / 不得控制。
//! 任何一条线路都不许自己加字段。
//!
//! # 校验只检查，不补内容
//!
//! 以前那一套会在出片时替模型「补」景别、锚点、契约，结果互相打架。
//! 这里发现问题只报出来，由生成那一步重试或者由用户改。

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// H3 单次生成的原生时长（秒）。drama-skills `minimax-h3.md` 的档案 + 本机 WeeTodd 节点上限 15。
pub const MIN_SHOT_SECS: f32 = 4.0;
pub const MAX_SHOT_SECS: f32 = 15.0;

/// 时间段首尾对齐的容差（秒）。模型写 3.9 / 4.0 这种不该算断档。
const SEGMENT_TOLERANCE: f32 = 0.05;

/// 一镜里的一个时间段。**各段描述的是同一空间里的连续变化，不是切镜。**
///
/// 照 libtv 的写法：「0~4 秒 打字与震动」「4~8 秒 晕倒与切黑」，每段有自己的
/// 画面 / 禁止 / 约束 / 站位 / 运镜 / 音效。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ShotSegment {
    /// 本镜内的起止秒数。第一段从 0 开始，最后一段结束于本镜时长。
    pub start: f32,
    pub end: f32,
    /// 这一段的一句话标题，如「打字与震动」。
    #[serde(default)]
    pub title: String,
    /// 画面：这一段里能看见的人、动作和状态变化。
    #[serde(default)]
    pub visual: String,
    /// 禁止：这一段里不能出现的东西。
    #[serde(default)]
    pub forbid: String,
    /// 约束：这一段里必须保持的状态。
    #[serde(default)]
    pub constraints: String,
    /// 站位与朝向。
    #[serde(default)]
    pub blocking: String,
    /// 这一段的运镜：只一个主运镜（类型 + 幅度 + 速度 + 落点）。
    #[serde(default)]
    pub camera: String,
    /// 这一段的声音。
    #[serde(default)]
    pub sound: String,
    /// **表演合同**：这一段里人物「说之前、说的时候、说完后」观众看得见什么（2026-10-12，见 [`Performance`]）。
    /// 没有就不写出来 —— 老镜头的提示词指纹不变，不会全库误报过期。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub performance: Option<Performance>,
    /// **攻防交换**：打斗镜里这一段的一次交换（七个字段，见 [`Exchange`]）。一段最多一个。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exchange: Option<Exchange>,
}

/// **表演合同**（2026-10-12，用户要出片「去 AI 感」；Codex 方案 P0 + P1）。
///
/// 表演以前散在 `visual` 和情绪词里，合成时写手要自己猜、经常整段丢掉。这里把它拆成可检查的几样：
/// 触发 → 视线 → 面部（≤2）/ 呼吸身体（≤2）→ 手在干什么 → 物理细节（汗 / 水 / 泪 / 灰要有来源、运动、可见光）→ 结果 → 怎么拍才看得见。
/// 规矩（`timeline_lint` 会查）：每段可见表演信号 2~4 个；远景不写面部；情绪必须落成看得见的行为。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Performance {
    /// 谁在表演（人物名，和资产名一致）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub performer: String,
    /// 触发：听见、看见、受了什么力
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub trigger: String,
    /// 视线：从哪到哪
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub gaze: String,
    /// 面部信号，最多 2 个：慢眨、下眼睑收紧、眉心短收、嘴角停住……
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub face: Vec<String>,
    /// 呼吸 / 身体，最多 2 个：吸气、吞咽、肩线起伏、重心后移……
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub body: Vec<String>,
    /// 手实际在做什么（不是「紧张」）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub hand: String,
    /// 物理细节：汗 / 水 / 泪 / 血 / 灰 / 蒸汽 / 衣料。必须有来源、运动、看得见它的光
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<MaterialEvent>,
    /// 动作 / 台词之后留下的状态（进下一段的开场）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub aftermath: String,
    /// 什么景别、焦点、停留能证明它
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub camera_proof: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct MaterialEvent {
    /// 来源：雨、蒸汽、尘土、伤口、汗……
    #[serde(default)]
    pub source: String,
    /// 看得见的运动：水从发梢甩出、沿太阳穴滑下……
    #[serde(default)]
    pub motion: String,
    /// 哪侧的光让它看得见
    #[serde(default)]
    pub light: String,
}

/// **一次攻防交换**（2026-10-12 打斗编排；字段顺序照 combat-choreography 的「开场空间 → 意图 → 攻击路径 → 防守 → 结果 → 新站位 → 镜头与声音」，
/// 用我们自己的话写）。和 [`Performance`] 是同一套因果：上一个交换的 `new_state` 就是下一个的 `opening`。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Exchange {
    /// 发起方（人物名）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub attacker: String,
    /// 开场：谁在画左 / 画右、距离、高度、架势、武器状态
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub opening: String,
    /// 意图与触发：读到了什么、佯攻、护住什么、想抢什么
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub intent: String,
    /// 攻击路径：哪只手 / 什么武器、从哪起、什么轨迹、打向哪里
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub attack: String,
    /// 防守：格 / 挡 / 闪 / 卸 / 盾 / 舍
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub defense: String,
    /// 结果：命中 / 擦过 / 落空 / 对撞 / 打断，力和环境的反应
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub result: String,
    /// 新站位：脚下、距离、朝向、武器状态、伤、露出的破绽
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub new_state: String,
    /// 证明这个动作的镜头 + 同步音效
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub camera_sound: String,
    /// 接触那一刻（本镜内的秒）。FCPX 导出按它打冲击帧标记
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact_at: Option<f32>,
}

/// 这一镜画面里要交代的场景和道具，带尺寸和摆放。
///
/// libtv 和 B 试片都写了「手机，长约 15 厘米，放在桌面上」—— 不写的话，
/// 模型不知道手机多大、放哪，会画成悬浮的或者和显示器一样大。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SceneItem {
    pub name: String,
    /// 尺寸和摆放，如「长约 15 厘米，屏幕朝上放在键盘右边」。
    #[serde(default)]
    pub detail: String,
}

/// **这一场的固定布景**：整场所有镜头共用的一份「谁在哪、灯在哪、朝哪看」。
///
/// # 为什么要有
///
/// 镜头表以前只有每段的「站位」，写的是画面左 / 中 / 右 —— 那是某一个机位下的投影，
/// 不是世界里的事实。换机位时谁该在左没有任何依据可推，每镜各编各的。
/// EP01「拆机间」第 4、5 镜陆川在左、竹竿在右，第 9 镜两人无交代地对调；
/// 工作灯一镜在右上、一镜在侧后（2026-09-28 对照报告第 0 节）。
///
/// 外部的做法（ai-short-drama-production 的场面调度图、cinematic-video-prompt-engineer 的
/// 场景连续性记录）都是**先固定世界，再按机位推画面左右**。这里是它的最小版本：
/// 镜头表每场写一次，存在这一场的每一镜上（和 `lighting` 一样跟着镜头走 ——
/// 时间线细化、线路 A/B、首帧拿到的都是镜头，不用再去集上查场号）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SceneStage {
    /// 这一场拿哪个方向当「正前方」—— 下面所有的左右都按它说。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub facing: String,
    /// 搬不走的东西在场里的位置。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anchors: Vec<String>,
    /// 场里的每一个光源：在哪、朝哪照、冷暖。**整场固定**，换机位只换从哪一侧看它。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lights: Vec<String>,
    /// 主要是谁和谁的关系、机位一般待在哪一侧（180° 轴）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub axis: String,
    /// 每个人一开始在哪、朝哪。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub people: Vec<String>,
    /// 时段和天气，整场一个说法。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub time: String,
}

impl SceneStage {
    pub fn is_empty(&self) -> bool {
        self.facing.trim().is_empty() && self.anchors.is_empty() && self.lights.is_empty() && self.axis.trim().is_empty() && self.people.is_empty() && self.time.trim().is_empty()
    }

    /// 从模型输出里宽松地读：每一项写成字符串或字符串数组都认（模型两种都写过）。
    pub fn from_value(v: &serde_json::Value) -> Self {
        use serde_json::Value;
        let text = |v: &Value| -> String {
            match v {
                Value::String(s) => s.trim().to_string(),
                Value::Array(a) => a.iter().filter_map(|x| x.as_str()).map(str::trim).filter(|x| !x.is_empty()).collect::<Vec<_>>().join("；"),
                Value::Null => String::new(),
                other => other.to_string(),
            }
        };
        let list = |v: &Value| -> Vec<String> {
            match v {
                Value::Array(a) => a
                    .iter()
                    .map(|x| match x {
                        Value::String(s) => s.trim().to_string(),
                        Value::Object(m) => m.values().filter_map(|y| y.as_str()).map(str::trim).filter(|y| !y.is_empty()).collect::<Vec<_>>().join("："),
                        other => other.to_string(),
                    })
                    .filter(|x| !x.is_empty())
                    .collect(),
                Value::String(s) => s.split(['；', '\n']).map(str::trim).filter(|x| !x.is_empty()).map(str::to_string).collect(),
                _ => Vec::new(),
            }
        };
        let get = |k: &str| v.get(k).unwrap_or(&Value::Null);
        Self {
            facing: text(get("facing")),
            anchors: list(get("anchors")),
            lights: list(get("lights")),
            axis: text(get("axis")),
            people: list(get("people")),
            time: text(get("time")),
        }
    }

    /// 写进下游提示词的样子（中文，一项一行）。空的项不写。
    pub fn lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.facing.trim().is_empty() {
            out.push(format!("方向基准：{}（下面的左右都按这个方向）", self.facing.trim()));
        }
        if !self.anchors.is_empty() {
            out.push(format!("固定物：{}", self.anchors.join("；")));
        }
        if !self.lights.is_empty() {
            out.push(format!("光源（整场固定，换机位只换从哪一侧看）：{}", self.lights.join("；")));
        }
        if !self.axis.trim().is_empty() {
            out.push(format!("主关系与机位侧：{}", self.axis.trim()));
        }
        if !self.people.is_empty() {
            out.push(format!("开场站位：{}", self.people.join("；")));
        }
        if !self.time.trim().is_empty() {
            out.push(format!("时段天气：{}", self.time.trim()));
        }
        out
    }
}

/// 一张参考图在这一镜里负责什么。取值照 drama-skills 分镜 SKILL 的「用途」。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum RefUsage {
    /// 人物跨镜不变的长相与体态
    Identity,
    /// 服装与损污
    Outfit,
    /// 空间关系
    Geography,
    /// 构图
    Composition,
    /// 尺度
    Scale,
    /// 特效
    Effect,
    /// 开场构图与姿态（首帧）
    StartFrame,
    /// 结尾构图（尾帧），必须同时有起始帧
    EndFrame,
    /// 画风
    Style,
}

impl RefUsage {
    pub fn label(self) -> &'static str {
        match self {
            RefUsage::Identity => "身份",
            RefUsage::Outfit => "造型状态",
            RefUsage::Geography => "地理",
            RefUsage::Composition => "构图",
            RefUsage::Scale => "尺度",
            RefUsage::Effect => "效果",
            RefUsage::StartFrame => "起始帧",
            RefUsage::EndFrame => "结束帧",
            RefUsage::Style => "风格",
        }
    }
}

/// 一镜的一个参考图槽位。
///
/// **编号就是提示词里的标签**：`order` 为 2 的这张在线路 B 里叫 `<Picture 2>`，
/// 在线路 A 里叫「参考图 2」。两套编号对不上时模型收到互相矛盾的说明，接口不报错。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RefSlot {
    /// 从 1 开始，连续。
    pub order: u8,
    /// 资产 id。起始帧 / 结束帧这类不是资产的图，写本镜镜头 id。
    pub asset_id: String,
    pub usage: RefUsage,
    /// 这张图控制什么。**必须正面写** —— drama-skills 实测：写「保留起始构图」
    /// 构图才会带过来，只写「不得控制」没用。
    pub control: String,
    /// 不得控制什么。给人看的责任边界，不是对模型的开关。
    #[serde(default)]
    pub not_control: String,
}

/// 提示词线路。**两条物理隔离，可以切换。** 见方案 v3。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PromptRoute {
    /// libtv 中文结构，纯填空
    A,
    /// H3 官方六段结构，代码生成四段 + LM 写描述
    B,
}

impl Default for PromptRoute {
    /// 默认 B：两份试片用户评价 B 略好。
    fn default() -> Self {
        PromptRoute::B
    }
}

impl PromptRoute {
    pub fn label(self) -> &'static str {
        match self {
            PromptRoute::A => "线路 A（中文结构）",
            PromptRoute::B => "线路 B（H3 官方六段）",
        }
    }
}

/// 镜头表里写了、但资产库里没有的资产。**不丢掉**：合成提示词时按文字写进去（兜底）。
///
/// 2026-09-13 用户要求：「资产里没有，或者有但没有图片，要有提示词兜底」。
/// 以前名字对不上资产库就只写一条备注然后扔掉，提示词里这个东西就没了。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct UnlinkedAsset {
    /// `character` / `scene` / `prop`
    pub kind: String,
    pub name: String,
}

/// 某一条线路给这一镜合成出来的两条提示词。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RoutePrompts {
    /// 分镜提示词：出首帧 / 分镜图时原样发送。
    #[serde(default)]
    pub frame: String,
    /// 视频运动提示词：出片时原样发送。
    #[serde(default)]
    pub video: String,
    /// 合成时间（秒）。0 = 没合成过。
    #[serde(default)]
    pub synthesized_at: u64,
    /// 合成时镜头表字段的指纹（`prompt_input::fields_hash`）。和现在的对不上 = 字段改过，该重新合成。
    #[serde(default)]
    pub fields_hash: String,
}

/// 每条线路各存一份。切线路不覆盖另一条。
pub type ShotPromptSet = BTreeMap<PromptRoute, RoutePrompts>;

/// 校验需要的最小镜头视图。和 `StoryboardShot` 解耦，测试不用造一整个镜头。
pub struct ShotView<'a> {
    pub shot_num: u32,
    pub duration: f32,
    pub segments: &'a [ShotSegment],
    /// (说话人, 台词, 是否画外音)
    pub lines: Vec<(&'a str, &'a str, bool)>,
    /// 这一镜出场的资产 id（人物、场景、道具）
    pub asset_ids: Vec<&'a str>,
    /// 这一镜出场人物的名字（说话人只能是他们之一；画外音不限）
    pub character_names: Vec<&'a str>,
    pub ref_slots: &'a [RefSlot],
    /// 光影
    pub lighting: &'a str,
    /// 一镜出的镜头（[`crate::one_take`]）：镜内**允许**写切镜 —— H3 一次生成里就会切
    pub one_take: bool,
}

/// 这几句台词至少要多长的镜头才说得下（和下面「台词装不下」那条检查同一套算法，半秒一档）。
/// 15 秒以内装不下就往超长镜里找（[`crate::long_shot`]，最长 45 秒，分段出、每段末尾 2 秒不说话）；
/// 45 秒也装不下回 `None` —— 那只能拆镜或者删台词。`lines` 是 (说话人, 台词, 是否画外音)
pub fn min_secs_for_lines(lines: &[(&str, &str, bool)]) -> Option<f32> {
    let said: Vec<&str> = lines.iter().map(|(_, t, _)| *t).filter(|t| !t.trim().is_empty()).collect();
    let mut d = MIN_SHOT_SECS;
    while d <= crate::long_shot::MAX_LONG_SHOT_SECS {
        if lines_fit_secs(&said, d) {
            return Some(d);
        }
        d += 0.5;
    }
    None
}

/// 这几句台词在 `d` 秒的一镜里放不放得下。15 秒以内按整镜算；超长镜按段装（一句话不跨段）。
fn lines_fit_secs(said: &[&str], d: f32) -> bool {
    if crate::long_shot::is_long(d) {
        return crate::long_shot::lines_fit(said, d);
    }
    let spoken: usize = said.iter().map(|t| crate::h3_prompt::speakable_chars(t)).sum();
    let turns = said.len().saturating_sub(1) as f64;
    let usable = (d as f64 * SPEECH_SHARE - turns * TURN_GAP_SECS).max(0.0);
    (usable * MAX_SPOKEN_CPS).floor() as usize >= spoken
}

/// 一条校验问题。`field` 给界面定位用。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ShotIssue {
    pub shot_num: u32,
    pub field: &'static str,
    pub message: String,
}

/// 镜内切镜的写法。一镜 = 一次生成、镜内不切；需要切就拆成下一镜。
/// 「切黑」是结尾黑屏，不算切镜，所以不在表里。
const CUT_WORDS: [&str; 6] = ["切至", "切回", "切到", "镜头切", "画面切", "切换到"];

/// 背景音乐（观众才听得见的配乐）。视频模型的音轨和画面一起生成，写进声音就会真的配上一段乐。
const MUSIC_WORDS: [&str; 5] = ["背景音乐", "配乐", "音乐起", "BGM", "bgm"];

/// 二选一的写法。视频模型没法「或者」—— 它会挑一个，或者两个都糊进同一个画面。
/// 2026-09-13 实跑：「焦点转到眼睛，或者从眼前一只手掠过」「（或定格在震惊）」。
const ALT_WORDS: [&str; 7] = ["或者", "或是", "抑或", "（或", "(或", "还是说", "，或"];

/// 模型写给自己看的话，不是画面。实跑：光影写了「（根据前文'灯火连绵'调整为夜景或晨曦）」。
const META_WORDS: [&str; 4] = ["根据前文", "根据剧情", "视情况", "可根据"];

/// 连着 3 个以上英文字母。实跑混进了「Pan」「Rim Light」—— 中文提示词线路里夹英文术语，
/// 模型读成画面里的文字或者直接忽略。词表 id 不在这些自由文字段里，不受影响。
fn english_run(text: &str) -> Option<String> {
    let mut run = String::new();
    for c in text.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_alphabetic() {
            run.push(c);
        } else {
            if run.len() >= 3 {
                return Some(run);
            }
            run.clear();
        }
    }
    None
}

/// 只留可发声的字（去掉空白和标点）。
fn spoken_key(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).collect()
}

/// H3 中文自然语速（可发声字 / 秒），和 `h3_prompt::check_dialogue_fit` 同一个口径。
const MAX_SPOKEN_CPS: f64 = 4.1;

/// 换一个人说话要留的停顿（秒）。
///
/// **算容量时必须减掉。** 只按「总字数 ÷ 语速」算的话，
/// 一镜四句话的容量和一句话一样 —— 而四句之间要换三次人，
/// 每次都要留一点气口，否则出来是抢话。
const TURN_GAP_SECS: f64 = 0.35;

/// 一镜里留给动作和反应的最小比例。
///
/// # 为什么容量不能按整镜时长算
///
/// 2026-09-19 Codex Astra 的审查指出 E03 第 4 镜：总长 13 秒，
/// 阎王的长句 28 个可发声字，而最终提示词指定**约第 7 秒才开口**，
/// 还要求 quiet and slow。真正留给这句话的只有 6 秒，
/// 需要 4.67 字/秒 —— 超过 4.1 的自然语速。
///
/// 而按整镜 13 秒算容量是 53 字，**轻松通过**。
///
/// 一镜不可能从第 0 秒说到最后一秒：总要有进场、反应、收尾。
/// 按八成算，剩下两成给动作 —— 这是估计，不是精确值，
/// 所以它只用来**报提醒**，不拦。
const SPEECH_SHARE: f64 = 0.8;

/// 校验一镜。`known_assets` 是资产库里所有资产 id。
pub fn validate(shot: &ShotView<'_>, known_assets: &BTreeSet<String>) -> Vec<ShotIssue> {
    let mut out = Vec::new();
    let mut issue = |field: &'static str, message: String| {
        out.push(ShotIssue { shot_num: shot.shot_num, field, message })
    };

    // 超过 15 秒是超长镜：出片时分成几段一次出完（`long_shot`），镜头表里还是一镜
    let max_secs = crate::long_shot::MAX_LONG_SHOT_SECS;
    if !(MIN_SHOT_SECS..=max_secs).contains(&shot.duration) {
        issue("duration", format!("时长 {} 秒，不在 {MIN_SHOT_SECS}~{max_secs} 秒之间（超过 {MAX_SHOT_SECS} 秒的按超长镜分段出）", shot.duration));
    }

    if shot.segments.is_empty() {
        issue("segments", "没有画面描述（至少要有一个时间段）".into());
    } else {
        let first = &shot.segments[0];
        if first.start.abs() > SEGMENT_TOLERANCE {
            issue("segments", format!("第一段从 {} 秒开始，应该从 0 秒开始", first.start));
        }
        for (i, seg) in shot.segments.iter().enumerate() {
            if seg.end <= seg.start {
                issue("segments", format!("第 {} 段 {}~{} 秒，结束不晚于开始", i + 1, seg.start, seg.end));
            }
            if seg.visual.trim().is_empty() {
                issue("segments", format!("第 {} 段没有写画面", i + 1));
            }
            if let Some(w) = CUT_WORDS.iter().find(|w| !shot.one_take && (seg.visual.contains(**w) || seg.camera.contains(**w))) {
                issue("segments", format!("第 {} 段写了「{w}」—— 一镜内不切镜，要切就拆成下一镜", i + 1));
            }
            if let Some(w) = MUSIC_WORDS.iter().find(|w| seg.sound.contains(**w)) {
                issue("sound", format!("第 {} 段的声音写了「{w}」—— 不要背景音乐，声音只写环境声、动作声", i + 1));
            }
            let texts = [&seg.visual, &seg.constraints, &seg.blocking, &seg.camera];
            if let Some(w) = ALT_WORDS.iter().find(|w| texts.iter().any(|t| t.contains(**w))) {
                issue("segments", format!("第 {} 段写了「{w}」—— 画面只能写一种，视频模型没法二选一", i + 1));
            }
            if let Some(w) = META_WORDS.iter().find(|w| texts.iter().any(|t| t.contains(**w))) {
                issue("segments", format!("第 {} 段写了「{w}」—— 这是写给自己的说明，不是画面，直接写定下来的结果", i + 1));
            }
            if let Some(word) = texts.iter().copied().chain([&seg.sound]).find_map(|t| english_run(t)) {
                issue("segments", format!("第 {} 段夹了英文「{word}」—— 全部写中文", i + 1));
            }
            // 台词只能在台词栏：写进声音或画面，出片时会念两遍或者叠音
            let heard = spoken_key(&format!("{}{}", seg.sound, seg.visual));
            if let Some((_, line, _)) = shot.lines.iter().find(|(_, line, _)| {
                let key = spoken_key(line);
                key.chars().count() >= 3 && heard.contains(&key)
            }) {
                issue("sound", format!("第 {} 段的声音或画面里又写了一遍台词「{}」—— 台词只写在台词栏，这里删掉", i + 1, line.trim()));
            }
            if let Some(next) = shot.segments.get(i + 1) {
                if (next.start - seg.end).abs() > SEGMENT_TOLERANCE {
                    issue("segments", format!("第 {} 段到 {} 秒，第 {} 段从 {} 秒开始，中间断开或重叠", i + 1, seg.end, i + 2, next.start));
                }
            }
        }
        let last = shot.segments.last().unwrap();
        if (last.end - shot.duration).abs() > SEGMENT_TOLERANCE {
            issue("segments", format!("最后一段到 {} 秒，镜头时长是 {} 秒，对不上", last.end, shot.duration));
        }
    }

    if let Some(w) = ALT_WORDS.iter().chain(META_WORDS.iter()).find(|w| shot.lighting.contains(**w)) {
        issue("lighting", format!("光影写了「{w}」—— 只写一种定下来的光，不要二选一或者写说明"));
    }
    if let Some(word) = english_run(shot.lighting) {
        issue("lighting", format!("光影夹了英文「{word}」—— 全部写中文"));
    }

    let mut spoken = 0usize;
    for (speaker, text, off_screen) in &shot.lines {
        if text.trim().is_empty() {
            continue;
        }
        spoken += crate::h3_prompt::speakable_chars(text);
        let speaker = speaker.trim();
        if speaker.is_empty() {
            issue("dialogue", format!("台词「{}」没有写说话人", text.trim()));
        } else if !off_screen && !shot.character_names.iter().any(|n| n.trim() == speaker) {
            // **画面里写着这个人，就不是问题** —— 邻居甲、打手甲这类龙套按文字描述出，不建资产
            // （用户 2026-09-30：全局资产是按完整设定建的，分集里剧本顺带出现的人和东西不需要单独建资产）。
            // H3 认说话人靠的也是画面描写里指得到这个人（`h3_prompt::speakers_anchored`）。
            // 原来一律报「画外音要标出来」：照着标了口型就没了，「自动修正」也修不了（没有资产可加），
            // 按钮上却算进去，点了没反应（2026-09-30 EP02）。
            let on_screen = shot.segments.iter().any(|seg| [&seg.visual, &seg.blocking].iter().any(|t| t.contains(speaker)));
            if !on_screen {
                issue("dialogue", format!("说话人「{speaker}」不是这一镜的出场人物，画面里也没写到他（画外音要标出来）"));
            }
        }
    }
    // **容量不能按整镜时长算。** 一镜里总要有进场、反应、收尾，
    // 换人说话之间还要留气口。详见 `SPEECH_SHARE` / `TURN_GAP_SECS` 的注释。
    if crate::long_shot::is_long(shot.duration) {
        // 超长镜：按段装，每段末尾 2 秒不说话、一句话不跨段（`long_shot::lines_fit`）
        let said: Vec<&str> = shot.lines.iter().map(|(_, t, _)| *t).filter(|t| !t.trim().is_empty()).collect();
        if !lines_fit_secs(&said, shot.duration) {
            let (n, w) = crate::long_shot::plan(shot.duration);
            issue(
                "dialogue",
                format!(
                    "台词 {spoken} 字，这一镜 {} 秒是超长镜，出片时分 {n} 段、每段 {w:.1} 秒（除最后一段，每段最后 2 秒不说话，一句话不跨段）—— 放不下。要么加长这一镜，要么把台词改短",
                    shot.duration
                ),
            );
        }
    } else {
        let turns = shot.lines.iter().filter(|(_, t, _)| !t.trim().is_empty()).count().saturating_sub(1) as f64;
        let usable = (shot.duration as f64 * SPEECH_SHARE - turns * TURN_GAP_SECS).max(0.0);
        let capacity = (usable * MAX_SPOKEN_CPS).floor() as usize;
        let hard = (shot.duration as f64 * MAX_SPOKEN_CPS).floor() as usize;
        if spoken > hard {
            issue("dialogue", format!("台词 {spoken} 字，{} 秒怎么都说不完（自然语速约 {hard} 字）—— 出片时超出的部分会被静默截掉", shot.duration));
        } else if spoken > capacity {
            issue(
                "dialogue",
                format!(
                    "台词 {spoken} 字，这一镜 {} 秒里真正留给说话的大约 {:.1} 秒（约 {capacity} 字）—— 要么加长这一镜，要么把台词改短，否则只能一开场就抢着说完",
                    shot.duration, usable
                ),
            );
        }
    }

    for id in &shot.asset_ids {
        if !known_assets.contains(*id) {
            issue("assets", format!("资产「{id}」在资产库里找不到"));
        }
    }

    let mut orders: Vec<u8> = shot.ref_slots.iter().map(|s| s.order).collect();
    orders.sort_unstable();
    if orders.iter().enumerate().any(|(i, o)| *o as usize != i + 1) {
        issue("ref_slots", format!("参考图编号 {orders:?} 必须从 1 开始连续且不重复（编号就是提示词里的标签）"));
    }
    let count = |u: RefUsage| shot.ref_slots.iter().filter(|s| s.usage == u).count();
    if count(RefUsage::StartFrame) > 1 || count(RefUsage::EndFrame) > 1 {
        issue("ref_slots", "起始帧、结束帧各最多一张".into());
    }
    if count(RefUsage::EndFrame) == 1 && count(RefUsage::StartFrame) == 0 {
        issue("ref_slots", "有结束帧就必须有起始帧".into());
    }
    for slot in shot.ref_slots {
        let frame_slot = matches!(slot.usage, RefUsage::StartFrame | RefUsage::EndFrame);
        // **只当外形参考挂的可以不在出场资产里**（2026-09-27：传送链上一排同型号仿生人，
        // 挂女主的出厂款当外形参考，她本人不出场）。用途是「身份」却不出场的仍然报 ——
        // 那多半是镜头表写错了人。
        if !frame_slot && !shot.asset_ids.contains(&slot.asset_id.as_str()) {
            if !known_assets.contains(&slot.asset_id) {
                issue("ref_slots", format!("参考图 {} 用的资产「{}」在资产库里找不到", slot.order, slot.asset_id));
            } else if slot.usage == RefUsage::Identity {
                issue("ref_slots", format!("参考图 {} 用的资产「{}」不在这一镜的出场资产里 —— 只当外形参考的话，用途改成「造型状态」", slot.order, slot.asset_id));
            }
        }
        if slot.control.trim().is_empty() {
            issue("ref_slots", format!("参考图 {}（{}）没写控制什么 —— 不正面写，模型不会保留", slot.order, slot.usage.label()));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start: f32, end: f32, visual: &str) -> ShotSegment {
        ShotSegment { start, end, visual: visual.into(), ..Default::default() }
    }

    fn slot(order: u8, asset: &str, usage: RefUsage, control: &str) -> RefSlot {
        RefSlot { order, asset_id: asset.into(), usage, control: control.into(), not_control: String::new() }
    }

    fn known() -> BTreeSet<String> {
        ["char_shen", "scene_office", "prop_phone"].iter().map(|s| s.to_string()).collect()
    }

    fn fields(issues: &[ShotIssue]) -> Vec<&'static str> {
        issues.iter().map(|i| i.field).collect()
    }

    /// libtv 第 1 镜的样子：8 秒两段，一个人，一张身份图。应当一条问题都没有。
    fn good<'a>(segments: &'a [ShotSegment], slots: &'a [RefSlot]) -> ShotView<'a> {
        ShotView {
            shot_num: 1,
            duration: 8.0,
            segments,
            lines: vec![("林婉清", "我还能撑。", false)],
            asset_ids: vec!["char_shen", "scene_office", "prop_phone"],
            character_names: vec!["林婉清"],
            ref_slots: slots,
            lighting: "显示器冷蓝光从正面打脸，背景全暗",
            one_take: false,
        }
    }

    #[test]
    fn 合格的镜头没有问题() {
        let segs = [seg(0.0, 4.0, "打字"), seg(4.0, 8.0, "晕倒切黑")];
        let slots = [slot(1, "char_shen", RefUsage::Identity, "脸型、发型、白衬衫")];
        assert_eq!(validate(&good(&segs, &slots), &known()), vec![]);
    }

    #[test]
    fn 时长超出视频模型一次生成的范围() {
        let segs = [seg(0.0, 3.0, "打字")];
        let mut v = good(&segs, &[]);
        v.duration = 3.0;
        assert_eq!(fields(&validate(&v, &known())), ["duration"]);
        // 16 秒是超长镜（分段出），不再报；超过 45 秒的上限才报
        let segs = [seg(0.0, 16.0, "打字")];
        let mut v = good(&segs, &[]);
        v.duration = 16.0;
        assert!(!fields(&validate(&v, &known())).contains(&"duration"));
        let segs = [seg(0.0, 46.0, "打字")];
        let mut v = good(&segs, &[]);
        v.duration = 46.0;
        assert!(fields(&validate(&v, &known())).contains(&"duration"));
    }

    #[test]
    fn 时间段必须从零开始_首尾相接_落在时长上() {
        let cases: [&[ShotSegment]; 5] = [
            &[seg(1.0, 8.0, "a")],
            &[seg(0.0, 3.0, "a"), seg(4.0, 8.0, "b")],
            &[seg(0.0, 5.0, "a"), seg(4.0, 8.0, "b")],
            &[seg(0.0, 4.0, "a"), seg(4.0, 7.0, "b")],
            &[seg(0.0, 4.0, "a"), seg(4.0, 4.0, "b"), seg(4.0, 8.0, "c")],
        ];
        for segs in cases {
            let issues = validate(&good(segs, &[]), &known());
            assert!(fields(&issues).contains(&"segments"), "没抓到：{segs:?}");
        }
        // 模型写 3.98 / 4.0 这种不算断档
        let ok = [seg(0.0, 3.98, "a"), seg(4.0, 8.0, "b")];
        assert!(!fields(&validate(&good(&ok, &[]), &known())).contains(&"segments"));
    }

    #[test]
    fn 镜内切镜和背景音乐都要报_切黑不算() {
        let cut = [ShotSegment { start: 0.0, end: 8.0, visual: "大臣指控，镜头切至林婉清震惊的脸".into(), ..Default::default() }];
        assert_eq!(fields(&validate(&good(&cut, &[]), &known())), ["segments"]);
        let cut_in_camera = [ShotSegment { start: 0.0, end: 8.0, visual: "a".into(), camera: "过肩，然后切回正面".into(), ..Default::default() }];
        assert_eq!(fields(&validate(&good(&cut_in_camera, &[]), &known())), ["segments"]);
        // 一镜出的镜头（一场一镜、镜内按时间轴切）：切镜是写法本身，不报
        let one_take = ShotView { one_take: true, ..good(&cut, &[]) };
        assert_eq!(validate(&one_take, &known()), vec![], "一镜出的镜头允许镜内切镜");
        let one_take_music = [ShotSegment { start: 0.0, end: 8.0, visual: "切至特写".into(), sound: "背景音乐渐起".into(), ..Default::default() }];
        assert_eq!(fields(&validate(&ShotView { one_take: true, ..good(&one_take_music, &[]) }, &known())), ["sound"], "别的规矩照旧");
        let black = [ShotSegment { start: 0.0, end: 8.0, visual: "倒下，最后 0.5 秒切黑".into(), camera: "固定，最后硬切黑屏".into(), ..Default::default() }];
        assert_eq!(validate(&good(&black, &[]), &known()), vec![]);
        for sound in ["风声，宏大的背景音乐起", ">>风声呼啸，音乐起（若有）"] {
            let music = [ShotSegment { start: 0.0, end: 8.0, visual: "a".into(), sound: sound.into(), ..Default::default() }];
            assert_eq!(fields(&validate(&good(&music, &[]), &known())), ["sound"], "{sound}");
        }
        // 画面里真有人弹琵琶，那是画面内的声音，不算配乐
        let diegetic = [ShotSegment { start: 0.0, end: 8.0, visual: "a".into(), sound: "隔壁传来琵琶声".into(), ..Default::default() }];
        assert_eq!(validate(&good(&diegetic, &[]), &known()), vec![]);
    }

    /// 2026-09-13 实跑挑出来的四种写法，原句照搬。
    #[test]
    fn 二选一_写给自己的说明_夹英文_台词写进声音都要报() {
        let slots = [slot(1, "char_shen", RefUsage::Identity, "脸型")];
        let check = |mut s: ShotSegment, lighting: &str| {
            s.start = 0.0;
            s.end = 8.0;
            let segs = [s];
            let mut v = good(&segs, &slots);
            v.lighting = lighting;
            validate(&v, &known())
        };
        let alt = check(seg(0.0, 8.0, "镜头焦点转移到眼睛，或者直接从她眼前的一只手前掠过"), "冷光");
        assert!(alt.iter().any(|i| i.message.contains("或者")), "{alt:?}");
        let paren = check(seg(0.0, 8.0, "眼神坚定（或定格在震惊）"), "冷光");
        assert!(paren.iter().any(|i| i.message.contains("（或")), "{paren:?}");
        let meta = check(seg(0.0, 8.0, "俯瞰长安"), "白昼阳光（根据前文调整为夜景）");
        assert_eq!(fields(&meta), ["lighting"], "{meta:?}");
        let pan = check(ShotSegment { camera: "左右轻微摇摄（Pan）".into(), ..seg(0.0, 8.0, "扫视") }, "冷光");
        assert!(pan.iter().any(|i| i.message.contains("Pan")), "{pan:?}");
        let rim = check(seg(0.0, 8.0, "侧脸"), "侧逆光（Rim Light）");
        assert!(rim.iter().any(|i| i.field == "lighting" && i.message.contains("Rim")), "{rim:?}");
        let dup = check(ShotSegment { sound: "林婉清低语：'我还能撑。'".into(), ..seg(0.0, 8.0, "抬头") }, "冷光");
        assert!(dup.iter().any(|i| i.field == "sound" && i.message.contains("我还能撑")), "{dup:?}");
        // 正常的中文、词表里的数字不误报；「或」字单独出现（「或许」以外的正文）不报
        let fine = check(ShotSegment { sound: "键盘声，4K 屏幕嗡嗡响".into(), ..seg(0.0, 8.0, "她或站或坐都不自在，最后伏在桌上") }, "显示器冷蓝光");
        assert_eq!(fine, vec![], "{fine:?}");
    }

    #[test]
    fn 没有画面描述或某段画面为空() {
        assert!(fields(&validate(&good(&[], &[]), &known())).contains(&"segments"));
        let segs = [seg(0.0, 4.0, "打字"), seg(4.0, 8.0, "  ")];
        assert!(fields(&validate(&good(&segs, &[]), &known())).contains(&"segments"));
    }

    #[test]
    fn 说话人必须是出场人物_画外音不限() {
        let segs = [seg(0.0, 8.0, "a")];
        let mut v = good(&segs, &[]);
        v.lines = vec![("皇帝", "放肆！", false)];
        assert_eq!(fields(&validate(&v, &known())), ["dialogue"]);
        v.lines = vec![("皇帝", "放肆！", true)];
        assert_eq!(validate(&v, &known()), vec![]);
        v.lines = vec![("", "放肆！", false)];
        assert_eq!(fields(&validate(&v, &known())), ["dialogue"]);
    }

    /// 2026-09-30 EP02：邻居甲、打手甲在画面里说话，资产库没有他们 —— 龙套按文字描述出、不建资产（用户定的），不该报。
    /// 原来报「画外音要标出来」，照着标了口型就没了；「自动修正」也修不了，按钮上却算进去
    #[test]
    fn 龙套在画面里说话不报_画面里没写到的才要标画外音() {
        let segs = [seg(0.0, 8.0, "邻居甲从门框探出头，大笑着起哄")];
        let mut v = good(&segs, &[]);
        v.lines = vec![("邻居甲", "终于有老婆了！", false)];
        assert_eq!(validate(&v, &known()), vec![]);
        v.lines = vec![("邻居乙", "多少钱啊？", false)];
        let out = validate(&v, &known());
        assert_eq!(fields(&out), ["dialogue"]);
        assert!(out[0].message.contains("画面里也没写到他"), "{out:?}");
    }

    /// 「加长到 N 秒」按钮的 N：和「台词装不下」那条检查同一套算法，半秒一档；超长镜（≤ 45 秒）也装不下给 None
    #[test]
    fn 台词至少要多长的镜头() {
        let one = |n: usize| "说".repeat(n);
        // 一句、不换人：可用 = 0.8 × 秒数，语速 4.1 字/秒 → 31 字要 9.5 秒（9 秒只够 29 字）
        let l31 = one(31);
        assert_eq!(min_secs_for_lines(&[("甲", l31.as_str(), false)]), Some(9.5));
        // 两人对话多扣一个换气
        let (a, b) = (one(20), one(11));
        assert_eq!(min_secs_for_lines(&[("甲", a.as_str(), false), ("乙", b.as_str(), false)]), Some(10.0));
        // 短台词不低于镜头下限
        assert_eq!(min_secs_for_lines(&[("甲", "好", false)]), Some(MIN_SHOT_SECS));
        let l50 = one(50);
        // 一句 50 字要 12.2 秒，比超长镜里最长的一段（15 秒 × 0.8）还长 —— 一句话不跨段，怎么加长都不行，只能拆镜
        assert_eq!(min_secs_for_lines(&[("甲", l50.as_str(), false)]), None, "一句话比一段还长，只能拆镜");
        // EP03 第 20 镜（2026-10-05）：四句 51 字，15 秒放不下 → 给出超长镜的时长，而不是没按钮
        let ep03 = [
            ("陆川", "刚才那句「主人不收二手的东西」，也是出厂话术？", false),
            ("星澜", "数据库里没有这句。", false),
            ("陆川", "那是哪儿来的？", false),
            ("星澜", "本机也不知道。本机只知道，她让主人不开心。", false),
        ];
        let secs = min_secs_for_lines(&ep03).expect("超长镜放得下");
        assert!(secs > MAX_SHOT_SECS && secs <= crate::long_shot::MAX_LONG_SHOT_SECS, "{secs}");
        let segs20 = [seg(0.0, secs, "陆川开口，星澜回答")];
        let mut v20 = good(&segs20, &[]);
        v20.duration = secs;
        v20.lines = ep03.iter().map(|(a, b, c)| (*a, *b, *c)).collect();
        v20.character_names = vec!["陆川", "星澜"];
        assert!(!fields(&validate(&v20, &known())).contains(&"dialogue"), "{:?}", validate(&v20, &known()));
        assert!(!fields(&validate(&v20, &known())).contains(&"duration"), "超长镜的时长不报");
        v20.duration = secs - 0.5;
        v20.segments = &[];
        let short: Vec<ShotIssue> = validate(&v20, &known()).into_iter().filter(|i| i.field == "dialogue").collect();
        assert!(short.iter().any(|i| i.message.contains("要么加长这一镜")), "短半秒要报，并且给修复按钮认得出的说法：{short:?}");
        // 和检查本身对得上：给出来的秒数下检查不再报
        let segs = [seg(0.0, 9.5, "a")];
        let mut v = good(&segs, &[]);
        v.duration = 9.5;
        v.lines = vec![("林婉清", l31.as_str(), false)];
        assert!(!fields(&validate(&v, &known())).contains(&"dialogue"));
    }

    /// 字数超过「整镜时长 × 自然语速」= **怎么都说不完**，出片会被静默截掉。
    #[test]
    fn 台词彻底装不下就报硬上限() {
        let segs = [seg(0.0, 4.0, "a")];
        let mut v = good(&segs, &[]);
        v.duration = 4.0;
        // 4 秒 × 4.1 ≈ 16 字；17 个可发声字怎么都说不完
        v.lines = vec![("林婉清", "一二三四五六七八九十一二三四五六七。", false)];
        let out = validate(&v, &known());
        assert_eq!(fields(&out), ["dialogue"]);
        assert!(out[0].message.contains("怎么都说不完"), "{:?}", out[0]);
    }

    /// **容量不能按整镜时长算。**
    ///
    /// 2026-09-19 Codex Astra 指出 E03 第 4 镜：总长 13 秒、28 个可发声字，
    /// 而提示词指定约第 7 秒才开口 —— 真正留给这句的只有 6 秒，
    /// 需要 4.67 字/秒。按整镜算容量是 53 字，**轻松通过**。
    ///
    /// 一镜里总要有进场、反应、收尾，按八成算。
    #[test]
    fn 台词挤满整镜要提醒_按八成算容量() {
        let segs = [seg(0.0, 13.0, "a")];
        let mut v = good(&segs, &[]);
        v.duration = 13.0;
        // 28 字：整镜容量 53 字（过），八成容量 42 字（也过）—— 不该报
        v.lines = vec![("林婉清", "一二三四五六七八九十一二三四五六七八九十一二三四五六七八。", false)];
        assert_eq!(validate(&v, &known()), vec![], "28 字放得下，不该报");

        // 45 字：整镜 53 字放得下，但八成只有 42 字 —— 要提醒，而且不能说成「说不完」
        let long: String = "一二三四五六七八九十".repeat(4) + "一二三四五。";
        v.lines = vec![("林婉清", Box::leak(long.into_boxed_str()), false)];
        let out = validate(&v, &known());
        assert_eq!(fields(&out), ["dialogue"], "{out:?}");
        assert!(out[0].message.contains("真正留给说话的"), "{:?}", out[0]);
        assert!(!out[0].message.contains("怎么都说不完"), "这是提醒不是硬超：{:?}", out[0]);
    }

    /// **换人说话要留气口。** 只按总字数算的话，一镜四句的容量和一句一样 ——
    /// 而四句之间要换三次人，不留气口出来是抢话。
    #[test]
    fn 多人对话的容量要减掉换人的停顿() {
        let segs = [seg(0.0, 10.0, "a")];
        let mut v = good(&segs, &[]);
        v.duration = 10.0;
        let one = vec![("林婉清", "一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十一。", false)];
        let four = vec![
            ("林婉清", "一二三四五六七八九十一二三四五。", false),
            ("皇帝", "一二三四五六七八九十一二三四五。", false),
            ("林婉清", "一二三四五六七八九十一二三四五。", false),
            ("皇帝", "一二三四五六七八九十一二三四五。", false),
        ];
        v.lines = one;
        let a = validate(&v, &known());
        v.lines = four;
        let b = validate(&v, &known());
        assert!(b.len() >= a.len(), "同样字数分成四句，容量应当更紧：一句 {a:?} / 四句 {b:?}");
    }

    #[test]
    fn 资产库里没有的资产要报() {
        let segs = [seg(0.0, 8.0, "a")];
        let mut v = good(&segs, &[]);
        v.asset_ids.push("char_ghost");
        assert_eq!(fields(&validate(&v, &known())), ["assets"]);
    }

    #[test]
    fn 参考图编号必须从一连续() {
        let segs = [seg(0.0, 8.0, "a")];
        let slots = [slot(1, "char_shen", RefUsage::Identity, "脸"), slot(3, "scene_office", RefUsage::Geography, "工位")];
        assert_eq!(fields(&validate(&good(&segs, &slots), &known())), ["ref_slots"]);
        let dup = [slot(1, "char_shen", RefUsage::Identity, "脸"), slot(1, "scene_office", RefUsage::Geography, "工位")];
        assert_eq!(fields(&validate(&good(&segs, &dup), &known())), ["ref_slots"]);
    }

    #[test]
    fn 参考图控制要正面写_资产要在本镜里() {
        let segs = [seg(0.0, 8.0, "a")];
        let empty_control = [slot(1, "char_shen", RefUsage::Identity, " ")];
        assert_eq!(fields(&validate(&good(&segs, &empty_control), &known())), ["ref_slots"]);
        let not_in_shot = [slot(1, "char_emperor", RefUsage::Identity, "脸")];
        assert_eq!(fields(&validate(&good(&segs, &not_in_shot), &known())), ["ref_slots"]);
        // 只当外形参考（不出场）：可以
        let mut lib = known();
        lib.insert("char_emperor".into());
        let look_only = [slot(1, "char_emperor", RefUsage::Outfit, "背景里同款制服的外形")];
        assert_eq!(validate(&good(&segs, &look_only), &lib), vec![]);
        // 库里有、但用途是身份又不出场：报，并提示改用途
        assert!(validate(&good(&segs, &not_in_shot), &lib)[0].message.contains("造型状态"));
        // 库里没有的：报
        let ghost = [slot(1, "char_ghost", RefUsage::Outfit, "外形")];
        assert!(validate(&good(&segs, &ghost), &known())[0].message.contains("资产库里找不到"));
        // 起始帧不是资产，不查出场资产
        let start = [slot(1, "shot_1", RefUsage::StartFrame, "起始构图、站位")];
        assert_eq!(validate(&good(&segs, &start), &known()), vec![]);
    }

    #[test]
    fn 结束帧必须配起始帧_各最多一张() {
        let segs = [seg(0.0, 8.0, "a")];
        let end_only = [slot(1, "shot_1", RefUsage::EndFrame, "结尾构图")];
        assert_eq!(fields(&validate(&good(&segs, &end_only), &known())), ["ref_slots"]);
        let two_starts = [slot(1, "shot_1", RefUsage::StartFrame, "构图"), slot(2, "shot_1", RefUsage::StartFrame, "构图")];
        assert_eq!(fields(&validate(&good(&segs, &two_starts), &known())), ["ref_slots"]);
    }

    #[test]
    fn 线路默认是_b_序列化成小写() {
        assert_eq!(PromptRoute::default(), PromptRoute::B);
        assert_eq!(serde_json::to_string(&PromptRoute::A).unwrap(), "\"a\"");
        let set: ShotPromptSet = serde_json::from_str(r#"{"b":{"video":"v"}}"#).unwrap();
        assert_eq!(set[&PromptRoute::B].video, "v");
        assert!(!set.contains_key(&PromptRoute::A));
    }
}
