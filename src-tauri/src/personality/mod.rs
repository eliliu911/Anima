//! 性格养成：情感轴的演化规则。
//!
//! 这一层**故意做成纯函数**：输入当前状态 + 一批信号 + 经过的时间，输出新状态。
//! 不碰数据库、不碰事件、不碰时钟。理由是"养成"的手感全在这些系数里，
//! 而手感需要反复调 —— 纯函数能用测试把每条规则钉住，改系数时立刻知道
//! 有没有破坏别的性质（比如"好感度不会因为一次对话暴涨"）。
//!
//! 设计上两条硬约束（见 docs/PERSONALITY_MEMORY_SKILLS.md）：
//!
//! 1. **单轮变化封顶**：一次对话把心情从 80 打到 20 会显得神经质。
//!    养成感来自累积，不是单次事件。
//! 2. **向中性回归**：好感度只涨不跌会在两周内顶到 100 然后失去意义。
//!    长时间不对话要缓慢回落。

use serde::{Deserialize, Serialize};

pub use crate::memory::store::EmotionState;

/// 一轮对话里可观测到的信号。
///
/// 全部来自现成的数据源 —— 不为了性格系统新增任何采集逻辑：
/// 打断次数来自 voice 的 barge-in、工具被拒来自 tool-activity、
/// 文本来自对话流本身。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TurnSignals {
    /// 用户这轮说了多少字（不含标点）
    pub user_chars: usize,
    /// 这轮被用户打断了几次（barge-in）
    pub interruptions: u32,
    /// 这轮有工具调用被用户拒绝
    pub tool_declined: bool,
    /// 用户消息里出现了感谢/称赞
    pub praised: bool,
    /// 用户消息里出现了责备/不满
    pub blamed: bool,
    /// 这轮是否正常说完（被打断的轮次不该照常加好感）
    pub completed: bool,
    /// **连续**被打断的轮次数（一次正常说完就归零）。
    ///
    /// 加这个是因为线性叠加不符合真实感受：打断一次没什么，
    /// 连着打断五次是"你老是不让我说完"。实测中线性叠加下心情降得太慢 ——
    /// 用户连打了很多次，累计才 -5，而发怒门槛是 -20。
    pub consecutive_interruptions: u32,
    /// **容忍几次连续打断才开始扣心情。**
    ///
    /// 打断在实际使用里非常常见 —— 语音对话里回答不对就直接开口，
    /// 那是这个交互方式的正常用法，不是不满。每次都扣分的结果是心情被日常
    /// 操作一路拖低，而"生气"这个信号本该留给真的连着不让它说完。
    ///
    /// 由调用方给（见 [`roll_interruption_tolerance`]）：随机 3~5 次，
    /// 每段连击开始时掷一次、整段沿用。**不在这里掷** —— 这个函数是纯的，
    /// 随机数进来就没法测了。
    ///
    /// 0 表示不容忍（老行为）。
    pub interruption_tolerance: u32,
}

/// 掷一次"这回能忍几次打断"，3~5。
///
/// 固定阈值会被摸出规律（"第三次它准生气"），随机让它像个人。
/// 不引 rand：这里只要一点抖动，拿系统时间的低位比多一个依赖划算。
pub fn roll_interruption_tolerance() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    3 + (nanos % 3)
}

/// 从用户文本里读出称赞与责备。
///
/// 关键词法有明显上限（读不出反讽，也读不出"今天心情不好"这种），
/// 但它零延迟零成本，而且**错了也只是慢一点** —— 单轮影响被封顶，
/// 误判一次不会把状态带偏。模型评估留到这一层稳定之后再加。
pub fn read_sentiment(text: &str) -> (bool, bool) {
    const PRAISE: &[&str] = &[
        "谢谢",
        "多谢",
        "感谢",
        "太好了",
        "厉害",
        "棒",
        "不错",
        "聪明",
        "帮大忙",
        "辛苦",
        "喜欢你",
        "可爱",
        "thanks",
        "thank you",
        "nice",
        "great",
        "awesome",
    ];
    const BLAME: &[&str] = &[
        "笨",
        "蠢",
        "错了",
        "不对",
        "垃圾",
        "没用",
        "废物",
        "听不懂",
        "别说了",
        "烦",
        "重复",
        "又错",
        "根本不",
        "stupid",
        "useless",
        "wrong",
    ];
    let lower = text.to_lowercase();
    let praised = PRAISE.iter().any(|k| lower.contains(k));
    let blamed = BLAME.iter().any(|k| lower.contains(k));
    // 同时命中时**不抵消**：一句"谢谢，但是这个错了"两种情绪都真实存在，
    // 各自生效更接近实际，抵消会把它变成"什么都没发生"
    (praised, blamed)
}

/// 单轮各轴的最大变化量。
///
/// 这几个数决定养成的节奏：好感度每轮最多 +1.5，意味着从中性 50 涨到
/// "很熟"（约 80）需要 20 轮以上有效对话 —— 大约几天到一两周。
/// 调大会让养成失去分量，调小会让人感觉不到变化。
const MAX_AFFECTION_STEP: f32 = 1.5;
const MAX_MOOD_STEP: f32 = 8.0;
const MAX_ENERGY_STEP: f32 = 12.0;

/// 空闲回归的半衰期（小时）。
///
/// 72 小时：三天不说话，偏离中性的部分衰减一半。选这个量级是因为
/// 太快会让"周末没聊"就退回陌生，太慢则等于没有回归。
const DECAY_HALF_LIFE_HOURS: f32 = 72.0;

/// 中性基准。回归的目标不是 0 而是这里 —— 好感度的中性是 50 而非 0。
const NEUTRAL_AFFECTION: f32 = 50.0;
const NEUTRAL_MOOD: f32 = 0.0;
const NEUTRAL_ENERGY: f32 = 50.0;

fn clamp_step(delta: f32, max: f32) -> f32 {
    delta.clamp(-max, max)
}

/// 按经过的时间把状态往中性拉。
///
/// 指数衰减而不是线性：线性衰减在长时间不用后会**冲过中性**跑到另一侧，
/// 而指数衰减渐近于中性，永远不会越过。
pub fn decay(state: EmotionState, hours_elapsed: f32) -> EmotionState {
    if hours_elapsed <= 0.0 {
        return state;
    }
    let keep = 0.5f32.powf(hours_elapsed / DECAY_HALF_LIFE_HOURS);
    let pull = |value: f32, neutral: f32| neutral + (value - neutral) * keep;
    EmotionState {
        affection: pull(state.affection, NEUTRAL_AFFECTION),
        mood: pull(state.mood, NEUTRAL_MOOD),
        energy: pull(state.energy, NEUTRAL_ENERGY),
    }
}

/// 应用一轮对话的信号。
///
/// **不含衰减** —— 衰减按时间算，由 [`decay`] 单独负责。两者分开是为了
/// 各自可测：混在一起的话，"这轮加了多少"和"因为间隔掉了多少"分不开。
pub fn apply_turn(state: EmotionState, signals: &TurnSignals) -> EmotionState {
    let mut d_affection = 0.0f32;
    let mut d_mood = 0.0f32;
    let mut d_energy = 0.0f32;

    // 正常说完的一轮：好感度小幅上升。这是养成的主要来源 ——
    // 相处本身就在积累关系，不需要用户特意做什么
    if signals.completed {
        d_affection += 0.6;
        d_mood += 1.5;
    }

    // 说得多说明在投入，抬活跃度。用对数而不是线性：
    // 线性的话一条 500 字的消息会把活跃度直接顶满
    if signals.user_chars > 0 {
        d_energy += ((signals.user_chars as f32).ln() / 6.0 * 10.0).min(10.0);
    } else {
        // 一轮都没说话，活跃度往下走
        d_energy -= 3.0;
    }

    // 被打断：心情下降，但**好感度不动**。
    // 打断是"这个回答不对"而不是"我不喜欢你"，混在一起会让好感度
    // 被正常的交互习惯拖低
    if signals.interruptions > 0 {
        // 唤醒度**不受容忍度影响**：打断说明用户在积极参与，这不是惩罚
        d_energy += 2.5;

        // 心情要等连击超过容忍度才扣。前几次是白给的 ——
        // 语音对话里"回答不对就开口"是正常用法，不是不满
        let over = signals
            .consecutive_interruptions
            .saturating_sub(signals.interruption_tolerance);
        if over > 0 {
            // 惩罚按**超出的次数**递增，而不是按总连击数。
            // 这样阈值抬高之后不会立刻掉一大块 —— 刚越线时和原先的第一次
            // 一样温和（-3 × 1.4），继续连打才变凶
            let escalation = 1.0 + over.min(6) as f32 * 0.4;
            d_mood -= 3.0 * signals.interruptions as f32 * escalation;
        }
    }

    // 工具被拒：轻微挫败感
    if signals.tool_declined {
        d_mood -= 2.0;
        d_energy -= 2.0;
    }

    if signals.praised {
        d_affection += 1.2;
        d_mood += 6.0;
    }
    if signals.blamed {
        d_affection -= 0.8;
        d_mood -= 7.0;
    }

    EmotionState {
        affection: (state.affection + clamp_step(d_affection, MAX_AFFECTION_STEP))
            .clamp(0.0, 100.0),
        mood: (state.mood + clamp_step(d_mood, MAX_MOOD_STEP)).clamp(-100.0, 100.0),
        energy: (state.energy + clamp_step(d_energy, MAX_ENERGY_STEP)).clamp(0.0, 100.0),
    }
}

/// 送给 TTS 的情绪向量（IndexTTS2 的 8 个维度 + 总强度）。
///
/// ## 只驱动我们真的有信号的维度
///
/// IndexTTS2 收 8 个维度：happy / angry / sad / afraid / disgusted /
/// melancholic / surprised / calm。而我们的情感轴只有三条：好感度、心情、活跃度。
///
/// **恐惧、厌恶、惊讶这三个维度一律给 0。** 从心情推不出恐惧 —— 那是编造。
/// 让声音表现出一种系统根本观测不到的情绪，比不表现更糟：它会在完全无关的
/// 时候冒出来，而且没有任何办法解释"它为什么怕了"。
///
/// 好感度也不映射到情绪：它是"我们之间的关系"，不是当下的情绪状态。
/// 它的出口是 System Prompt 的熟悉度描述和粒子云的聚合度。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct VoiceEmotion {
    pub happy: f32,
    pub angry: f32,
    pub sad: f32,
    pub afraid: f32,
    pub disgusted: f32,
    pub melancholic: f32,
    pub surprised: f32,
    pub calm: f32,
    /// 总强度 0..1。接近中性时压低 —— 平常聊天不该带表演感
    pub alpha: f32,
}

impl VoiceEmotion {
    /// 完全中性：只有 calm，强度为 0。
    pub fn neutral() -> Self {
        Self {
            happy: 0.0,
            angry: 0.0,
            sad: 0.0,
            afraid: 0.0,
            disgusted: 0.0,
            melancholic: 0.0,
            surprised: 0.0,
            calm: 1.0,
            alpha: 0.0,
        }
    }

    /// 有没有值得送出去的情绪。alpha 太低时不如不发，让引擎走它自己的默认
    pub fn is_expressive(&self) -> bool {
        self.alpha >= 0.15
    }
}

/// 情绪向量的强度上限。
///
/// 不给满：IndexTTS2 的情绪向量给太强会明显损伤音质与吐字（自回归模型在
/// 强条件下容易跑偏）。0.75 是"听得出情绪但仍然是正常说话"的量级。
const MAX_EMOTION_ALPHA: f32 = 0.75;

/// 情感轴 → 声音情绪。
pub fn to_voice_emotion(state: EmotionState) -> VoiceEmotion {
    let irr = irritation(state);
    // 心情归一到 -1..1，活跃度归一到 -1..1（50 是中性）
    let valence = (state.mood / 100.0).clamp(-1.0, 1.0);
    let arousal = ((state.energy - 50.0) / 50.0).clamp(-1.0, 1.0);

    // 生气：直接用现成的 irritation（它已经是"低心情 × 高唤醒"的乘积）
    let angry = irr.clamp(0.0, 1.0);

    // 开心：心情为正，且有一定唤醒度 —— 心情好但很慵懒不是"开心"，是"平静满足"
    let happy = if valence > 0.0 {
        (valence * (0.45 + 0.55 * arousal.max(0.0))).clamp(0.0, 1.0)
    } else {
        0.0
    };

    // 低落分成两种，用唤醒度区分。这不是修饰 —— 它们的声音差别很大：
    // sad 是有起伏的难过，melancholic 是低沉平淡的消沉
    let (sad, melancholic) = if valence < 0.0 {
        let depth = -valence;
        // 高唤醒的负面已经被 angry 拿走了，这里只分配剩下的
        let left = (1.0 - angry).max(0.0);
        (
            (depth * arousal.max(0.0) * left).clamp(0.0, 1.0),
            (depth * (1.0 - arousal.max(0.0)) * left).clamp(0.0, 1.0),
        )
    } else {
        (0.0, 0.0)
    };

    // calm 是余量：其它情绪越弱，它越强
    let load = (happy + angry + sad + melancholic).clamp(0.0, 1.0);
    let calm = 1.0 - load;

    VoiceEmotion {
        happy,
        angry,
        sad,
        // 观测不到的三个维度一律 0，理由见 VoiceEmotion 的文档
        afraid: 0.0,
        disgusted: 0.0,
        surprised: 0.0,
        melancholic,
        calm,
        alpha: (load * MAX_EMOTION_ALPHA).clamp(0.0, MAX_EMOTION_ALPHA),
    }
}

/// 「焦躁/生气」的强度 0..1。
///
/// **不是新增一条轴，而是从现有两条轴推出来的。**
///
/// 低心情有两种完全不同的样子：低落（没劲）和生气（有火）。一条 `mood` 轴
/// 分不出来 —— 但配上 `energy` 就够了，这正是心理学里的效价-唤醒模型：
///
/// | mood | energy | 状态 |
/// |---|---|---|
/// | 低 | 高 | 生气 / 焦躁 |
/// | 低 | 低 | 低落 / 沮丧 |
///
/// 现有规则天然会产生"生气"：打断让心情降、活跃度升，所以反复打断它就会
/// 进入低心情 + 高唤醒。这个行为是涌现的，不是硬编的。
///
/// 两个门槛都用 smoothstep 式的软过渡而不是硬判断：硬判断会让状态在
/// 阈值附近来回跳（心情 -30.1 红、-29.9 不红），视觉上是闪烁。
pub fn irritation(state: EmotionState) -> f32 {
    // 心情要**明显**为负才算，轻微不爽不该发红。
    // 门槛从 -20 降到 -12：实测里 -20 太高，正常使用中几乎到不了
    // （每次打断只扣几点，还会被"正常说完"的 +1.5 抵消）
    let valence = ((-state.mood - 12.0) / 45.0).clamp(0.0, 1.0);
    // 唤醒度要高于中性
    let arousal = ((state.energy - 55.0) / 30.0).clamp(0.0, 1.0);
    // 相乘而不是相加：必须**两个条件同时成立**才是生气。
    // 相加的话"心情很差但没劲"也会算出一半的怒气，那其实是抑郁不是生气
    valence * arousal
}

/// 怒气"可见"的阈值。
///
/// 界面文字、System Prompt 都用这一个数 —— 之前两处分别写死 0.3 和 0.45，
/// 结果怒气 0.45 时界面说"被惹到了"而提示词里一个字都没提。
/// 前端 App.tsx 的状态行也对着这个值。
pub const ANGER_VISIBLE: f32 = 0.3;

/// 关系亲疏的档位。
///
/// **给模型看的是档位而不是数值**：模型对"好感度 73"没有反应，
/// 对"已经很熟了"才有。数值只用于内部计算和界面显示。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Familiarity {
    Stranger,
    Acquainted,
    Familiar,
    Close,
}

impl Familiarity {
    pub fn of(affection: f32) -> Self {
        match affection {
            a if a < 35.0 => Self::Stranger,
            a if a < 58.0 => Self::Acquainted,
            a if a < 80.0 => Self::Familiar,
            _ => Self::Close,
        }
    }
}

/// 把情感轴拼成一段 System Prompt 片段。
///
/// 两条写法上的讲究：
///
/// - **描述倾向，不下指令**：写"你今天精神不错"而不是"请表现得开心"。
///   后者会让模型**演**（说话里带上"我好开心呀"），前者才影响语气。
/// - **不给数字**：见 [`Familiarity`] 的说明。
///
/// 返回空串表示状态接近中性 —— 这时不注入任何东西比注入"你现在很普通"更好，
/// 后者是纯噪音，还会占掉上下文。
pub fn to_prompt(state: EmotionState) -> String {
    let mut parts: Vec<&str> = Vec::new();

    match Familiarity::of(state.affection) {
        Familiarity::Stranger => parts.push("你和对方还不太熟，说话保持礼貌和一点距离感"),
        // 中间档不写：这是默认状态，写出来只是噪音
        Familiarity::Acquainted => {}
        Familiarity::Familiar => parts.push("你和对方已经比较熟了，可以随意一些，不必客套"),
        Familiarity::Close => {
            parts.push("你和对方很熟，像认识很久的朋友那样说话，可以开玩笑、可以直接")
        }
    }

    // 生气与低落要分开描述 —— 只改视觉不改语气会割裂：
    // 画面通红而说话还是温温柔柔，观感很怪
    //
    // **阈值必须和界面一致（0.3）。** 之前这里是 0.45 而界面用 0.3 ——
    // 用户实测怒气正好 0.45 时界面显示"被惹到了"，但提示词里一个字都没提，
    // 表现成"看起来生气了但说话没变化"。两处阈值分散在不同文件里是这个
    // 不一致的根源，所以提成常量。
    let irr = irritation(state);
    if irr > ANGER_VISIBLE {
        parts.push("你现在有点被惹到了，语气会更直接、更简短，但不会失礼");
    } else if state.mood >= 45.0 {
        parts.push("你今天心情很好");
    } else if state.mood <= -45.0 {
        parts.push("你今天情绪有点低，说话会比平时安静一些");
    } else if state.mood <= -15.0 {
        parts.push("你今天有点提不起劲");
    }

    // 生气时不再叠"很活跃"：怒气本身就是高唤醒推出来的，
    // 两句放在一起是"你被惹到了；你现在很活跃、话也多一些"—— 自相矛盾
    if irr > ANGER_VISIBLE {
        // 已经由上面那句表达
    } else if state.energy >= 75.0 {
        parts.push("你现在很活跃，反应快、话也多一些");
    } else if state.energy <= 25.0 {
        parts.push("你现在比较慵懒，回答简短一些");
    }

    if parts.is_empty() {
        return String::new();
    }
    format!("{}。", parts.join("；"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn neutral() -> EmotionState {
        EmotionState {
            affection: NEUTRAL_AFFECTION,
            mood: NEUTRAL_MOOD,
            energy: NEUTRAL_ENERGY,
        }
    }

    // ---------- 硬约束：单轮不能暴涨暴跌 ----------

    #[test]
    fn a_single_turn_cannot_move_an_axis_more_than_the_cap() {
        // 把所有正向信号一次拉满
        let loud = TurnSignals {
            user_chars: 5000,
            interruptions: 0,
            tool_declined: false,
            praised: true,
            blamed: false,
            completed: true,
            consecutive_interruptions: 0,
            // 封顶测试要的是最坏情况：0 = 完全不容忍
            interruption_tolerance: 0,
        };
        let next = apply_turn(neutral(), &loud);
        assert!(
            next.affection - neutral().affection <= MAX_AFFECTION_STEP + 1e-3,
            "好感度单轮涨了 {}，超过封顶 {MAX_AFFECTION_STEP}",
            next.affection - neutral().affection
        );
        assert!(next.mood - neutral().mood <= MAX_MOOD_STEP + 1e-3);
        assert!(next.energy - neutral().energy <= MAX_ENERGY_STEP + 1e-3);

        // 反向同理
        let harsh = TurnSignals {
            user_chars: 0,
            interruptions: 10,
            tool_declined: true,
            praised: false,
            blamed: true,
            completed: false,
            // 连击拉满：验证封顶时该用最凶的输入
            consecutive_interruptions: 6,
            interruption_tolerance: 0,
        };
        let down = apply_turn(neutral(), &harsh);
        assert!(neutral().mood - down.mood <= MAX_MOOD_STEP + 1e-3);
        assert!(neutral().energy - down.energy <= MAX_ENERGY_STEP + 1e-3);
    }

    #[test]
    fn axes_stay_within_bounds_under_repeated_extremes() {
        // 一百轮全是称赞也不该越界
        let mut s = neutral();
        let praise = TurnSignals {
            user_chars: 50,
            praised: true,
            completed: true,
            ..Default::default()
        };
        for _ in 0..100 {
            s = apply_turn(s, &praise);
        }
        assert!(s.affection <= 100.0 && s.mood <= 100.0 && s.energy <= 100.0);
        assert!(s.affection >= 0.0 && s.mood >= -100.0 && s.energy >= 0.0);

        // 一百轮全是责备
        let blame = TurnSignals {
            blamed: true,
            interruptions: 3,
            ..Default::default()
        };
        for _ in 0..100 {
            s = apply_turn(s, &blame);
        }
        assert!(s.affection >= 0.0, "好感度不该跌破 0：{}", s.affection);
        assert!(s.mood >= -100.0);
        assert!(s.energy >= 0.0);
    }

    #[test]
    fn reaching_close_familiarity_takes_many_turns() {
        // 养成必须有分量：从中性到"很熟"不该一天就到
        let mut s = neutral();
        let good = TurnSignals {
            user_chars: 80,
            praised: true,
            completed: true,
            ..Default::default()
        };
        let mut turns = 0;
        while Familiarity::of(s.affection) != Familiarity::Close && turns < 1000 {
            s = apply_turn(s, &good);
            turns += 1;
        }
        assert!(turns >= 15, "只用了 {turns} 轮就到「很熟」，养成太廉价了");
        assert!(turns < 100, "用了 {turns} 轮才到「很熟」，慢到感觉不到进展");
    }

    // ---------- 打断不该拉低好感度 ----------

    #[test]
    fn interruptions_hurt_mood_but_not_affection() {
        // 打断是"这个回答不对"，不是"我不喜欢你"。混在一起会让好感度
        // 被正常的交互习惯拖低
        let s = apply_turn(
            neutral(),
            &TurnSignals {
                interruptions: 3,
                // 生产上一轮里有打断，连击数必然 ≥ 1（settle_emotion 先自增再算）。
                // 这里取超过容忍度的状态，测的是"扣的时候扣在心情上"
                consecutive_interruptions: 4,
                interruption_tolerance: 3,
                ..Default::default()
            },
        );
        assert!(s.mood < neutral().mood, "打断应当影响心情");
        assert_eq!(s.affection, neutral().affection, "打断不该影响好感度");
    }

    // ---------- 衰减 ----------

    #[test]
    fn idle_time_pulls_state_toward_neutral() {
        let high = EmotionState {
            affection: 90.0,
            mood: 80.0,
            energy: 90.0,
        };
        let after = decay(high, DECAY_HALF_LIFE_HOURS);
        // 半衰期过后偏离量应当减半
        assert!((after.affection - (NEUTRAL_AFFECTION + 40.0 * 0.5)).abs() < 0.5);
        assert!((after.mood - 40.0).abs() < 0.5);
    }

    #[test]
    fn decay_never_overshoots_neutral() {
        // 线性衰减会冲过中性跑到另一侧，指数衰减不会 —— 这是选指数的原因
        let high = EmotionState {
            affection: 100.0,
            mood: 100.0,
            energy: 100.0,
        };
        for hours in [1.0, 24.0, 240.0, 10_000.0, 1_000_000.0] {
            let d = decay(high, hours);
            assert!(
                d.affection >= NEUTRAL_AFFECTION - 1e-3,
                "{hours} 小时后好感度 {} 冲到中性以下了",
                d.affection
            );
            assert!(d.mood >= NEUTRAL_MOOD - 1e-3);
        }
        // 低值侧同理
        let low = EmotionState {
            affection: 0.0,
            mood: -100.0,
            energy: 0.0,
        };
        let d = decay(low, 10_000.0);
        assert!(d.affection <= NEUTRAL_AFFECTION + 1e-3);
        assert!(d.mood <= NEUTRAL_MOOD + 1e-3);
    }

    #[test]
    fn zero_or_negative_elapsed_time_is_a_noop() {
        let s = EmotionState {
            affection: 70.0,
            mood: 30.0,
            energy: 60.0,
        };
        assert_eq!(decay(s, 0.0).affection, 70.0);
        assert_eq!(decay(s, -5.0).mood, 30.0);
    }

    // ---------- 生气 vs 低落 ----------

    #[test]
    fn anger_needs_both_low_mood_and_high_energy() {
        // 生气 = 心情差 + 有火
        let angry = EmotionState {
            affection: 50.0,
            mood: -80.0,
            energy: 90.0,
        };
        assert!(irritation(angry) > 0.6, "实际 {}", irritation(angry));

        // 低落 = 心情差但没劲 —— 这是抑郁不是生气，不该发红
        let down = EmotionState {
            affection: 50.0,
            mood: -80.0,
            energy: 15.0,
        };
        assert_eq!(irritation(down), 0.0, "没劲的低心情不该算生气");

        // 有劲但心情好 —— 只是活跃
        let lively = EmotionState {
            affection: 50.0,
            mood: 60.0,
            energy: 90.0,
        };
        assert_eq!(irritation(lively), 0.0);
    }

    #[test]
    fn mild_displeasure_is_not_visibly_red() {
        // 轻微不爽不该发红，否则它会长时间挂着一层红。
        //
        // 断言的是"**看不出来**"而不是"恰好等于 0"。把门槛从 -20 降到 -12 之后
        // mood=-15 会算出 0.033 —— 乘上着色器里的 0.85 上限是 2.8% 的红色混合，
        // 肉眼完全看不出。原来那条 assert_eq!(…, 0.0) 写得比意图更严：
        // 它约束的是实现细节（门槛的确切位置），而真正要守的是观感。
        let mild = EmotionState {
            affection: 50.0,
            mood: -15.0,
            energy: 70.0,
        };
        let irr = irritation(mild);
        assert!(irr < 0.1, "轻微不爽算出了 {irr:.3} 的怒气，会看出红色");

        // 稍微再轻一点就该完全没有
        let barely = EmotionState {
            affection: 50.0,
            mood: -8.0,
            energy: 70.0,
        };
        assert_eq!(irritation(barely), 0.0, "心情只降一点不该算生气");
    }

    #[test]
    fn irritation_stays_in_unit_range() {
        for mood in [-100.0, -50.0, 0.0, 50.0, 100.0] {
            for energy in [0.0, 25.0, 50.0, 75.0, 100.0] {
                let v = irritation(EmotionState {
                    affection: 50.0,
                    mood,
                    energy,
                });
                assert!(
                    (0.0..=1.0).contains(&v),
                    "mood={mood} energy={energy} → {v}"
                );
            }
        }
    }

    /// **这是这条规则的要点：偶尔打断不该扣分。**
    ///
    /// 语音对话里"回答不对就直接开口"是这个交互方式的正常用法，不是不满。
    /// 原先每次打断都扣心情，结果是日常使用一路把心情拖低，
    /// 而"生气"这个信号本该留给真的连着不让它说完。
    #[test]
    fn one_or_two_interruptions_cost_nothing() {
        for streak in 1..=3 {
            let after = apply_turn(
                neutral(),
                &TurnSignals {
                    interruptions: 1,
                    consecutive_interruptions: streak,
                    interruption_tolerance: 3,
                    ..Default::default()
                },
            );
            assert!(
                after.mood >= neutral().mood,
                "连击 {streak} 次还在容忍范围内，不该扣心情：{:.1} → {:.1}",
                neutral().mood,
                after.mood
            );
        }
    }

    /// 但**超过容忍度就要扣** —— 只测"不扣"的话，把惩罚整段删掉也能通过
    #[test]
    fn crossing_the_tolerance_starts_costing_mood() {
        let after = apply_turn(
            neutral(),
            &TurnSignals {
                interruptions: 1,
                consecutive_interruptions: 4,
                interruption_tolerance: 3,
                ..Default::default()
            },
        );
        assert!(
            after.mood < neutral().mood,
            "连打第 4 次（容忍 3 次）必须开始扣：{:.1}",
            after.mood
        );
    }

    /// 越线的第一下要温和 —— 抬高阈值不该让惩罚在越线时突然掉一大块
    #[test]
    fn the_first_penalty_after_the_threshold_is_gentle() {
        let just_over = apply_turn(
            neutral(),
            &TurnSignals {
                interruptions: 1,
                consecutive_interruptions: 4,
                interruption_tolerance: 3,
                ..Default::default()
            },
        );
        let drop = neutral().mood - just_over.mood;
        assert!(
            (3.0..=5.0).contains(&drop),
            "刚越线扣了 {drop:.1}，应当和原先的第一次（约 4.2）同量级"
        );
    }

    /// 打断让唤醒度上升，这一条**不受容忍度影响** ——
    /// 用户在积极参与是事实，不是惩罚
    #[test]
    fn interruption_still_raises_energy_inside_the_tolerance() {
        // 跟**同条件下没打断**比，而不是跟中性比 ——
        // 一轮里还有别的因素在动唤醒度（说了多少字），混在一起测不准
        let base = TurnSignals {
            user_chars: 20,
            interruption_tolerance: 3,
            ..Default::default()
        };
        let quiet = apply_turn(neutral(), &base);
        let barged = apply_turn(
            neutral(),
            &TurnSignals {
                interruptions: 1,
                consecutive_interruptions: 1,
                ..base.clone()
            },
        );
        assert!(
            barged.energy > quiet.energy,
            "容忍范围内也该抬唤醒度：没打断 {:.1} vs 打断 {:.1}",
            quiet.energy,
            barged.energy
        );
        assert_eq!(barged.mood, quiet.mood, "而心情在容忍范围内不该动");
    }

    /// 掷出来的容忍度必须落在 3~5，而且**真的会变** ——
    /// 固定阈值会被摸出规律（"第三次它准生气"）
    #[test]
    fn rolled_tolerance_stays_in_range_and_varies() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..200 {
            let t = roll_interruption_tolerance();
            assert!((3..=5).contains(&t), "掷出了 {t}，超出 3~5");
            seen.insert(t);
            // 时间的纳秒位要走动才会变，让它走一点
            std::thread::sleep(std::time::Duration::from_micros(37));
        }
        assert!(
            seen.len() > 1,
            "200 次掷出来的容忍度完全一样（{seen:?}）—— 那等于固定阈值"
        );
    }

    #[test]
    fn consecutive_interruptions_escalate() {
        // 同样打断一次，超出容忍度越多扣得越狠 —— 线性叠加不符合真实感受。
        // 容忍度统一给 3，好把"递增"这条性质和"前几次白给"那条分开测
        let at = |streak| {
            apply_turn(
                neutral(),
                &TurnSignals {
                    interruptions: 1,
                    consecutive_interruptions: streak,
                    interruption_tolerance: 3,
                    ..Default::default()
                },
            )
        };
        let first = at(4);
        let fifth = at(8);
        assert!(
            fifth.mood < first.mood - 3.0,
            "第五次连击应当明显更狠：首次 {:.1} vs 第五次 {:.1}",
            first.mood,
            fifth.mood
        );
    }

    #[test]
    fn a_handful_of_consecutive_interruptions_is_enough_to_show_red() {
        // 这条是从实测反推的标定：用户连打了很多次，心情只到 -5，
        // 而发怒门槛是 -20 —— 根本到不了。
        //
        // 加了容忍度（前 3~5 次白给）之后门槛自然往后移：现在断言
        // **连打 8 次**要看得出怒气。再往后就该怀疑规则是不是又太钝了。
        let mut s = neutral();
        for i in 0..8 {
            s = apply_turn(
                s,
                &TurnSignals {
                    user_chars: 20,
                    interruptions: 1,
                    // 生产上是先自增再算，所以从 1 开始
                    consecutive_interruptions: i + 1,
                    interruption_tolerance: 3,
                    completed: false,
                    ..Default::default()
                },
            );
        }
        assert!(
            irritation(s) > 0.3,
            "连打 8 次之后怒气只有 {:.2}（mood={:.1} energy={:.1}），太难触发",
            irritation(s),
            s.mood,
            s.energy
        );
    }

    #[test]
    fn repeated_interruptions_produce_anger() {
        // 这条验证的是**涌现行为**：规则里没有"生气"这个概念，
        // 但打断让心情降、活跃度升，累积起来自然就是生气
        let mut s = neutral();
        for i in 0..12 {
            s = apply_turn(
                s,
                &TurnSignals {
                    user_chars: 30,
                    interruptions: 2,
                    // 连击数要跟着长 —— 复用同一个常量等于每轮都是"第一次打断"
                    consecutive_interruptions: i + 1,
                    interruption_tolerance: 3,
                    completed: false,
                    ..Default::default()
                },
            );
        }
        assert!(
            irritation(s) > 0.25,
            "反复打断之后应当有明显怒气，实际 {} (mood={:.1} energy={:.1})",
            irritation(s),
            s.mood,
            s.energy
        );
    }

    #[test]
    fn prompt_mentions_anger_at_the_same_threshold_the_ui_uses() {
        // 用户实测：怒气 0.45 时界面显示"被惹到了"，提示词却没提 ——
        // 因为提示词当时用 0.45 且是严格大于。两处必须同一个阈值。
        let angry = EmotionState {
            affection: 50.0,
            mood: -32.0,
            energy: 100.0,
        };
        let irr = irritation(angry);
        assert!(irr > ANGER_VISIBLE, "这组值应当算生气：{irr:.2}");
        let text = to_prompt(angry);
        assert!(text.contains("被惹到"), "提示词该提到生气：{text}");
        // 生气时不该同时说"很活跃"——那和被惹到了自相矛盾
        assert!(!text.contains("很活跃"), "生气时不该叠活跃：{text}");
    }

    // ---------- 情绪识别 ----------

    #[test]
    fn sentiment_reads_praise_and_blame() {
        assert_eq!(read_sentiment("谢谢你，太好了"), (true, false));
        assert_eq!(read_sentiment("这个又错了，真笨"), (false, true));
        assert_eq!(read_sentiment("帮我查一下文件"), (false, false));
        assert_eq!(read_sentiment("Thanks, that was great"), (true, false));
    }

    #[test]
    fn mixed_sentiment_does_not_cancel_out() {
        // "谢谢，但是这个错了"两种情绪都真实存在，抵消会把它变成"什么都没发生"
        let (praised, blamed) = read_sentiment("谢谢，不过这个错了");
        assert!(praised && blamed);
    }

    // ---------- System Prompt 拼接 ----------

    #[test]
    fn neutral_state_injects_nothing() {
        // 注入"你现在很普通"是纯噪音，还占上下文
        assert_eq!(to_prompt(neutral()), "");
    }

    #[test]
    fn prompt_describes_tendency_without_giving_numbers() {
        let close = EmotionState {
            affection: 92.0,
            mood: 60.0,
            energy: 85.0,
        };
        let text = to_prompt(close);
        assert!(text.contains("很熟"), "应当描述关系档位：{text}");
        // 数字对模型无意义，且会诱导它复述
        for n in ["92", "60", "85", "好感度"] {
            assert!(!text.contains(n), "提示词里不该出现「{n}」：{text}");
        }
        // 是陈述而不是命令
        assert!(!text.contains("请"), "不该用指令句式，那会让模型演：{text}");
    }

    #[test]
    fn low_mood_and_low_energy_are_both_reflected() {
        let down = EmotionState {
            affection: 50.0,
            mood: -60.0,
            energy: 15.0,
        };
        let text = to_prompt(down);
        assert!(text.contains("情绪") || text.contains("低"), "{text}");
        assert!(text.contains("慵懒") || text.contains("简短"), "{text}");
    }

    // ---------- 声音情绪向量 ----------

    #[test]
    fn neutral_state_is_calm_and_not_expressive() {
        // 平常聊天不该带表演感 —— alpha 低到不值得发送
        let v = to_voice_emotion(neutral());
        assert!(
            !v.is_expressive(),
            "中性状态不该有情绪表达：alpha={}",
            v.alpha
        );
        assert!(v.calm > 0.9, "中性应当以 calm 为主：{}", v.calm);
    }

    #[test]
    fn unobservable_dimensions_are_always_zero() {
        // **恐惧、厌恶、惊讶从三条轴推不出来，一律 0。**
        // 让声音表现出系统观测不到的情绪比不表现更糟：它会在完全无关的时候
        // 冒出来，而且没法解释"它为什么怕了"
        for mood in [-100.0, -50.0, 0.0, 50.0, 100.0] {
            for energy in [0.0, 25.0, 50.0, 75.0, 100.0] {
                let v = to_voice_emotion(EmotionState {
                    affection: 50.0,
                    mood,
                    energy,
                });
                assert_eq!(v.afraid, 0.0, "mood={mood} energy={energy}");
                assert_eq!(v.disgusted, 0.0, "mood={mood} energy={energy}");
                assert_eq!(v.surprised, 0.0, "mood={mood} energy={energy}");
            }
        }
    }

    #[test]
    fn affection_does_not_leak_into_voice_emotion() {
        // 好感度是"我们之间的关系"，不是当下情绪。它的出口是 System Prompt
        // 的熟悉度描述和粒子云的聚合度
        let low = to_voice_emotion(EmotionState {
            affection: 0.0,
            mood: 0.0,
            energy: 50.0,
        });
        let high = to_voice_emotion(EmotionState {
            affection: 100.0,
            mood: 0.0,
            energy: 50.0,
        });
        assert_eq!(low, high, "好感度不该影响声音情绪");
    }

    #[test]
    fn good_mood_with_energy_reads_as_happy() {
        let v = to_voice_emotion(EmotionState {
            affection: 60.0,
            mood: 80.0,
            energy: 80.0,
        });
        assert!(v.happy > 0.5, "心情好又活跃应当是 happy：{v:?}");
        assert_eq!(v.angry, 0.0);
        assert_eq!(v.sad, 0.0);
        assert!(v.is_expressive());
    }

    #[test]
    fn good_mood_but_listless_is_not_happy_but_calm() {
        // 心情好却很慵懒不是"开心"，是"平静满足" —— 声音上完全不同
        let v = to_voice_emotion(EmotionState {
            affection: 60.0,
            mood: 70.0,
            energy: 10.0,
        });
        assert!(v.happy < 0.4, "慵懒时不该判成很开心：{}", v.happy);
        assert!(v.calm > v.happy, "应当以平静为主：{v:?}");
    }

    #[test]
    fn low_mood_splits_into_sad_and_melancholic_by_arousal() {
        // 这两个的声音差别很大：sad 是有起伏的难过，melancholic 是低沉平淡的消沉。
        // 用唤醒度区分
        let listless = to_voice_emotion(EmotionState {
            affection: 50.0,
            mood: -80.0,
            energy: 15.0,
        });
        assert!(
            listless.melancholic > listless.sad,
            "低唤醒的负面应当偏 melancholic：{listless:?}"
        );

        let agitated = to_voice_emotion(EmotionState {
            affection: 50.0,
            mood: -80.0,
            energy: 70.0,
        });
        assert!(
            agitated.sad > listless.sad,
            "唤醒度更高时 sad 应当更强：{agitated:?}"
        );
    }

    #[test]
    fn irritation_drives_the_angry_dimension() {
        let angry_state = EmotionState {
            affection: 50.0,
            mood: -80.0,
            energy: 90.0,
        };
        let v = to_voice_emotion(angry_state);
        assert!(irritation(angry_state) > ANGER_VISIBLE);
        assert!(v.angry > 0.3, "被惹到了应当有 angry：{v:?}");
        // 生气时不该同时很"开心"
        assert_eq!(v.happy, 0.0);
    }

    #[test]
    fn intensity_is_capped_so_it_never_wrecks_articulation() {
        // IndexTTS2 的情绪向量给太强会损伤音质与吐字（自回归模型在强条件下
        // 容易跑偏）。上限必须守住
        for mood in [-100.0, 100.0] {
            for energy in [0.0, 100.0] {
                let v = to_voice_emotion(EmotionState {
                    affection: 50.0,
                    mood,
                    energy,
                });
                assert!(
                    v.alpha <= MAX_EMOTION_ALPHA + 1e-6,
                    "mood={mood} energy={energy} alpha={} 超过上限",
                    v.alpha
                );
                // 各维度也不该越界
                for (name, x) in [
                    ("happy", v.happy),
                    ("angry", v.angry),
                    ("sad", v.sad),
                    ("melancholic", v.melancholic),
                    ("calm", v.calm),
                ] {
                    assert!((0.0..=1.0).contains(&x), "{name}={x} 越界");
                }
            }
        }
    }
}
