//! 按情绪挑参考音色。
//!
//! ## 为什么需要它
//!
//! 情感轴一直在算好感度/心情/活跃度，但**一个字节都没进 TTS** ——
//! 心情 -57 和 +80 说出来的声音完全一样。用户的原话是「没那么生动有感情」，
//! 这是其中最大的一块。
//!
//! GPT-SoVITS 不吃情绪向量（那是 IndexTTS2 的接口，而它在 Apple Silicon 上
//! 慢到不可用）。但零样本克隆有个天然的入口：**参考音频的语气会被一起克隆**。
//! 所以换一条语气不同的参考，出来的语气就跟着变 —— 不需要模型支持情绪。
//!
//! ## 三档，不是八维
//!
//! 只分三档是有意的：素材是有限的，而且档位越多越难保证每一档的音色一致
//! （同一个人不同录音的音色本来就有漂移）。三档已经能听出区别。
//!
//! 档位的边界取自情感轴自己的量纲，不是拍脑袋：
//! 心情的中性是 0、范围 ±100，所以 ±25 大约是"明显偏离中性"的位置。

use serde::{Deserialize, Serialize};

use crate::personality::EmotionState;

/// 情绪档位。对应角色 `voice.json` 里 `emotion_voices` 的键
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceTone {
    /// 上扬：高兴、兴奋。基频高、抑扬幅度大
    Up,
    /// 平常：中性偏暖，日常说话
    Soft,
    /// 低沉：心情差或冷淡。基频低、抑扬幅度小
    Calm,
}

impl VoiceTone {
    pub fn key(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Soft => "soft",
            Self::Calm => "calm",
        }
    }
}

/// 心情明显好/差的分界。情感轴的心情是 -100..100，中性 0
const MOOD_HIGH: f32 = 25.0;
const MOOD_LOW: f32 = -25.0;
/// 活跃度的中性是 50。低于这个值时即使心情不差也不该用上扬的语气
const ENERGY_NEUTRAL: f32 = 50.0;

/// 情感轴 → 语气档。
///
/// **心情为主、活跃度为辅**：心情决定往哪边偏，活跃度决定"有没有力气表现出来"。
/// 心情好但没精神（深夜聊天）时用平常档而不是上扬档 —— 那种时候的兴高采烈
/// 是很假的。
pub fn tone_for(emotion: EmotionState) -> VoiceTone {
    if emotion.mood <= MOOD_LOW {
        return VoiceTone::Calm;
    }
    if emotion.mood >= MOOD_HIGH && emotion.energy >= ENERGY_NEUTRAL {
        return VoiceTone::Up;
    }
    VoiceTone::Soft
}

/// 从角色的 `voice.json` 里取出情绪音色表。
///
/// 形状：
/// ```json
/// { "emotion_voices": { "up": "yaemiko/greeting", "soft": "yaemiko/d", "calm": "yaemiko/default" } }
/// ```
///
/// 缺档位就退回下一档（up → soft → calm → 不换），所以只配一两档也能用。
pub fn resolve(voice_config: Option<&serde_json::Value>, tone: VoiceTone) -> Option<String> {
    let table = voice_config?.get("emotion_voices")?.as_object()?;
    let pick = |k: &str| {
        table
            .get(k)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    // 退化顺序：想要的那档没有就往"平常"靠，平常也没有才用别的 ——
    // 宁可语气不到位，也不要因为缺一档就整个不换音色
    match tone {
        VoiceTone::Up => pick("up").or_else(|| pick("soft")).or_else(|| pick("calm")),
        VoiceTone::Soft => pick("soft").or_else(|| pick("calm")).or_else(|| pick("up")),
        VoiceTone::Calm => pick("calm").or_else(|| pick("soft")).or_else(|| pick("up")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emo(mood: f32, energy: f32) -> EmotionState {
        EmotionState {
            affection: 50.0,
            mood,
            energy,
        }
    }

    #[test]
    fn mood_decides_the_direction() {
        assert_eq!(tone_for(emo(60.0, 70.0)), VoiceTone::Up);
        assert_eq!(tone_for(emo(0.0, 50.0)), VoiceTone::Soft);
        assert_eq!(tone_for(emo(-60.0, 50.0)), VoiceTone::Calm);
    }

    /// 心情好但没精神时不该"兴高采烈" —— 那种时候的上扬语气很假
    #[test]
    fn energy_gates_the_upbeat_tone() {
        assert_eq!(tone_for(emo(80.0, 20.0)), VoiceTone::Soft);
        assert_eq!(tone_for(emo(80.0, 80.0)), VoiceTone::Up);
    }

    /// 心情差就是差，没精神也好、很亢奋也好，都不该是上扬的
    #[test]
    fn low_mood_wins_regardless_of_energy() {
        assert_eq!(tone_for(emo(-40.0, 95.0)), VoiceTone::Calm);
        assert_eq!(tone_for(emo(-40.0, 5.0)), VoiceTone::Calm);
    }

    #[test]
    fn reads_the_table_from_voice_json() {
        let cfg = serde_json::json!({
            "emotion_voices": { "up": "yaemiko/greeting", "soft": "yaemiko/d", "calm": "yaemiko/default" }
        });
        assert_eq!(
            resolve(Some(&cfg), VoiceTone::Up).as_deref(),
            Some("yaemiko/greeting")
        );
        assert_eq!(
            resolve(Some(&cfg), VoiceTone::Calm).as_deref(),
            Some("yaemiko/default")
        );
    }

    /// 只配了一档也要能用 —— 素材是慢慢攒的，不能要求一次配齐
    #[test]
    fn a_partial_table_degrades_instead_of_giving_up() {
        let cfg = serde_json::json!({ "emotion_voices": { "soft": "yaemiko/d" } });
        for t in [VoiceTone::Up, VoiceTone::Soft, VoiceTone::Calm] {
            assert_eq!(
                resolve(Some(&cfg), t).as_deref(),
                Some("yaemiko/d"),
                "{t:?}"
            );
        }
    }

    #[test]
    fn no_table_means_do_not_switch() {
        assert!(resolve(None, VoiceTone::Up).is_none());
        assert!(resolve(Some(&serde_json::json!({"voice": "x"})), VoiceTone::Up).is_none());
        // 空字符串等于没配
        let blank = serde_json::json!({ "emotion_voices": { "up": "  " } });
        assert!(resolve(Some(&blank), VoiceTone::Up).is_none());
    }
}
