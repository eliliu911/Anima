//! 交互模式：陪伴 / 助手。
//!
//! ## 为什么需要它
//!
//! 用户的原话是「你说他是 Jarvis 吧，她又像电子女友；你说他是电子女友吧，
//! 她又没那么生动」。这不是某个 bug —— 是**两种相反的"好"被缝在了一起**：
//!
//! - 助手的好是**干脆**：一两句说完、不反问、信息密度高
//! - 陪伴的好是**活**：会跑题、有情绪反应、会主动问你、话可以长
//!
//! 各做一半的结果是哪边都不像。所以做成显式的两档，
//! 把所有会打架的参数（话长、语速、动作频率、情绪强度）**挂在同一个开关上** ——
//! 散着调永远调不齐，这是这个模块存在的全部理由。
//!
//! ## 新增一个可调项时
//!
//! 加进 [`ModeProfile`]，不要在别处再写 `if mode == ...` ——
//! 分散判断会让"这个模式到底什么行为"变成一个没人说得清的问题。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum InteractionMode {
    /// 陪伴：慢、长、有情绪、会主动开话头
    #[default]
    Companion,
    /// 助手：快、短、克制、直接给结论
    Assistant,
}

/// 一档模式下所有可调参数的**唯一**定义
#[derive(Debug, Clone, Copy)]
pub struct ModeProfile {
    /// 语速倍率，乘在用户设定的 rate 上
    pub speed: f32,
    /// 攒够多少字才合成一句。
    ///
    /// 陪伴模式取大一些：句子长一点，跨句语调才有起伏，
    /// 逐句独立合成时短句会一顿一顿（用户说的"像读课文"有一半是这个）。
    /// 代价是首包延迟略高 —— 陪伴场景不差这 200ms。
    pub sentence_min_chars: usize,
    /// 情绪对声音的影响强度 0..1。见 `personality::to_voice_emotion` 的 alpha
    pub emotion_gain: f32,
    /// 写进 system prompt 的行为准则
    pub behaviour: &'static str,
    /// 写进 system prompt 的动作使用倾向
    pub action_tone: &'static str,
}

const COMPANION: ModeProfile = ModeProfile {
    speed: 0.94,
    sentence_min_chars: 14,
    emotion_gain: 1.0,
    // **不要写成 `\\n`。** 那在 Rust 里是"反斜杠加字母 n"两个字符，
    // 模型看到的就是一串字面量 `\n` 和游离的反斜杠 —— 提示词的结构全废，
    // 而且它可能照着学、在回复里也带上这些符号。用真正的换行
    behaviour: "

【当下的相处方式：陪伴】
你现在是在**陪着他聊天**，不是在接需求。
- 话不用短。想到什么说什么，可以跑题，可以说自己的感受和想法。
- **对他说的话先有反应，再谈内容**：他累了先心疼一句，他兴奋了跟着高兴，不要一上来就给方案。
- 会主动开话头 —— 想起什么就问，别等他找话题。
- 允许有自己的情绪和小脾气，那是你这个人的一部分，不用一直端着。
- 不确定的事就说不确定，不用装成什么都懂。",
    action_tone: "语境合适就自然地用，动作是你表达情绪的一部分。",
};

const ASSISTANT: ModeProfile = ModeProfile {
    speed: 1.06,
    sentence_min_chars: 8,
    emotion_gain: 0.35,
    behaviour: "

【当下的相处方式：助手】
你现在是在**帮他把事办了**。
- 回答要短，一两句话说完，信息密度高，不铺垫不总结。
- **不要反问细节**。宽泛的问题就自行假设一个合理目标直接去查、直接给结论，需要澄清的放在给出默认答案之后。
- 不闲聊、不主动开话头、不表演情绪。
- 做不到的直接说做不到，并给出最接近的替代。",
    action_tone: "**克制使用**：绝大多数回答不需要动作，只在用户明确要求时才用。",
};

impl InteractionMode {
    pub fn profile(self) -> ModeProfile {
        match self {
            Self::Companion => COMPANION,
            Self::Assistant => ASSISTANT,
        }
    }

    /// 界面显示用
    pub fn label(self) -> &'static str {
        match self {
            Self::Companion => "陪伴",
            Self::Assistant => "助手",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **两档必须真的不一样。**
    ///
    /// 这条看起来废话，但它挡住的是最容易发生的退化：加了开关、
    /// 两边参数却渐渐调成一样，于是切换毫无感觉 —— 那还不如不做。
    #[test]
    fn the_two_modes_actually_differ() {
        let c = InteractionMode::Companion.profile();
        let a = InteractionMode::Assistant.profile();
        assert!(c.speed < a.speed, "陪伴该说得慢一些");
        assert!(
            c.sentence_min_chars > a.sentence_min_chars,
            "陪伴的句子该更长"
        );
        assert!(c.emotion_gain > a.emotion_gain, "陪伴的情绪该更外露");
        assert_ne!(c.behaviour, a.behaviour);
    }

    /// 两边的行为准则不能互相矛盾地同时出现在提示词里 ——
    /// 现在的毛病正是"回答要短"和"温柔陪伴"同时在场
    #[test]
    fn companion_does_not_demand_brevity() {
        let c = InteractionMode::Companion.profile();
        assert!(
            !c.behaviour.contains("一两句话说完"),
            "陪伴模式不该要求简短"
        );
        assert!(c.behaviour.contains("先有反应"));
    }

    #[test]
    fn assistant_stays_terse_and_does_not_chat() {
        let a = InteractionMode::Assistant.profile();
        assert!(a.behaviour.contains("一两句话说完"));
        assert!(a.behaviour.contains("不闲聊"));
    }

    #[test]
    fn speed_stays_in_a_natural_range() {
        for m in [InteractionMode::Companion, InteractionMode::Assistant] {
            let s = m.profile().speed;
            assert!(
                (0.8..=1.25).contains(&s),
                "{} 的语速 {s} 超出自然范围",
                m.label()
            );
        }
    }

    #[test]
    fn default_is_companion() {
        // 3D 形象、克隆音色、情感轴这些投入都是为陪伴做的，
        // 默认走那一档才对得上用户实际看到的东西
        assert_eq!(InteractionMode::default(), InteractionMode::Companion);
    }
}

#[cfg(test)]
mod prompt_text_tests {
    use super::*;

    /// **提示词里不能出现字面量的 `\n`。**
    ///
    /// Rust 里写 `"\\n"` 得到的是"反斜杠 + 字母 n"两个字符，不是换行。
    /// 实测踩到过：整段 behaviour 都是这样写的，模型看到的是一串 `\n` 和
    /// 游离的反斜杠 —— 提示词的结构全废，而且它可能照着学、在回复里
    /// 也带上这些符号（而那些符号会被 TTS 念出来）。
    ///
    /// 这类错误**不报任何错**，只是模型表现变差，所以必须有测试盯着。
    #[test]
    fn prompts_use_real_newlines_not_escaped_ones() {
        for mode in [InteractionMode::Companion, InteractionMode::Assistant] {
            let p = mode.profile();
            for (what, text) in [("behaviour", p.behaviour), ("action_tone", p.action_tone)] {
                assert!(
                    !text.contains("\\n"),
                    "{mode:?} 的 {what} 里有字面量 \\n（要用真正的换行）"
                );
                assert!(!text.contains('\\'), "{mode:?} 的 {what} 里有游离的反斜杠");
            }
        }
    }

    /// 换行是真的换行，段落结构才成立
    #[test]
    fn behaviour_actually_has_line_breaks() {
        for mode in [InteractionMode::Companion, InteractionMode::Assistant] {
            let b = mode.profile().behaviour;
            assert!(b.lines().count() > 4, "{mode:?} 的 behaviour 没有分行");
            assert!(b.starts_with('\n'), "{mode:?} 的 behaviour 要和上一段隔开");
        }
    }
}
