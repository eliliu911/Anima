//! 思考档位：策略、模型能力、以及各后端的线格式。
//!
//! **这个模块是「档位 → 线格式」的唯一入口。** 三个后端（OpenAI 系 / Ollama 原生 /
//! Anthropic）的表达方式完全不同，散着写必然分叉 —— 这个项目已经在
//! 「两处描述同一件事」上栽过好几次。
//!
//! # 为什么不是一个 bool
//!
//! 本机实测（每个模型发同一道压缩题，读 `usage.completion_tokens_details.reasoning_tokens`）：
//!
//! | 模型 | arch | 不发 | `none` | `low` | `medium` | `high` |
//! |---|---|---|---|---|---|---|
//! | `openai/gpt-oss-120b` | gpt-oss | 8 | **8** | 9 | 92 | 184 |
//! | `qwen3.6-35b-a3b-uncensored` | qwen35moe | 900 | **0** | 900 | 900 | 900 |
//! | `ornith-1.5-35b-a3b` | qwen35moe | 243 | **0** | 195 | 137 | 230 |
//! | `gemma-4-e4b-uncensored` | gemma4 | 483 | **0** | 500 | 507 | 455 |
//! | `google/gemma-4-31b` | gemma4 | 458 | **0** | 468 | 333 | 429 |
//! | `qwen3-coder-next@4bit` | qwen3_next | 0 | 0 | 0 | 0 | 0 |
//!
//! 两条互补的事实：
//!
//! 1. **`gpt-oss` 认 low/medium/high，但不认 `none`** —— LM Studio 打一行 WARN
//!    然后忽略它，**不报错**。发 `none` 等于什么都没发。
//! 2. **其余所有族只认 `none`**，三个档位被忽略。表里那些 195/137/230、
//!    500/507/455 看着"有变化"，但**不单调** —— 那是采样噪声不是梯度。
//!    真正生效的长成 gpt-oss 那样：9 → 92 → 184，干净单调。
//!
//! 判据是**单调性**而不是「和基线不一样」。后者会把噪声判成生效。
//!
//! # 为什么「关」在 gpt-oss 上要发 `low`
//!
//! 它没有「关」这一档，而 `low` 只有 9 个思考 token（基线 8），实质等于不思考。
//! 发 `none` 反而什么也做不到 —— 这就是本模块存在之前的那个静默 bug。

use serde::{Deserialize, Serialize};

/// 思考档位。数值顺序即「从省到费」，供 UI 的滑杆直接用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingEffort {
    /// 尽力关闭思考。后端不支持时降级到它支持的最低档，不会让请求失败。
    Off,
    Low,
    Medium,
    High,
    /// 不干预，用后端/模型自己的默认行为。
    #[default]
    Auto,
}

impl ThinkingEffort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Auto => "auto",
        }
    }
}

/// 某个模型实际支持哪些档位。
///
/// UI 据此渲染：空表示这个模型不支持任何干预，**不要显示控件** ——
/// 给一个无效的控件比没有控件更糟。
pub fn supported_levels(model: &str) -> &'static [ThinkingEffort] {
    use ThinkingEffort::*;
    match family_of(model) {
        // 有真实梯度，但没有「关」。见模块注释的表。
        //
        // **不含 Auto。** 实测这个族「不发参数」和 `low` 是同一件事
        // （8 tok vs 9 tok），把它当成滑杆上的第四格是三重错误：
        // 位置错（它不在"更聪明"那一端）、语义错（它是"不干预"不是"最费"）、
        // 而且多余（和最左那格实测等价）。
        Family::GptOss => &[Low, Medium, High],
        // 只有开/关两态，档位会被忽略。
        Family::BinaryToggle => &[Off, Auto],
        // 压根不思考，发什么都一样。
        Family::NoThinking => &[],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    GptOss,
    BinaryToggle,
    NoThinking,
}

/// 这个模型收不收 `temperature`。
///
/// GPT-5 系列和 o1 / o3 / o4 这类推理模型**只认默认值 1**，发 0 直接 400：
/// 「'temperature' does not support 0 with this model. Only the default (1) value is supported」
/// （2026-09-29 Azure 上的 gpt-5.6-sol 实测）。Anima 的提示词槽位一律发 0，
/// 不去掉的话把它设成后台模型，生成镜头、合成提示词全部失败。
///
/// Claude 5 代起（`claude-opus-5-5` 这类）**整个不收 temperature**，发什么值都 400：
/// 「`temperature` is deprecated for this model」（2026-09-29 Azure 上的 Opus 5.5 实测）。
///
/// 按名字判断：Azure 的部署名是用户自己起的，名字里没带 `gpt-5` / `o3` / `claude-…-5` 这类字样的认不出来 —— 那种会照旧 400，报错原文说得清楚。
pub fn accepts_temperature(model: &str) -> bool {
    let m = model.to_ascii_lowercase();
    let m = m.rsplit('/').next().unwrap_or(&m);
    let reasoning = m.starts_with("gpt-5") || ["o1", "o3", "o4"].iter().any(|p| m == *p || m.starts_with(&format!("{p}-")));
    !reasoning && !is_claude_5_or_later(m)
}

/// `claude-<名字>-<主版本>-<小版本>`：主版本 ≥ 5。老式名字（`claude-3-5-sonnet`）主版本在前，照样认得出 3
fn is_claude_5_or_later(m: &str) -> bool {
    let Some(rest) = m.strip_prefix("claude-") else { return false };
    rest.split('-').find_map(|seg| seg.parse::<u32>().ok()).is_some_and(|major| major >= 5)
}

/// 按模型 id 归族。
///
/// 用 id 子串而不是 LM Studio 的 `arch` 字段，是因为线格式要在连接器里定，
/// 那一层只有模型名，去查 `/api/v0/models` 会给每次请求加一次网络往返。
///
/// 未知模型归到 `BinaryToggle` —— 实测六个模型里五个是这一类，
/// 而且它的「关」用 `none` 表达，被忽略时也只是退回默认，不会出错。
fn family_of(model: &str) -> Family {
    let m = model.to_ascii_lowercase();
    if m.contains("gpt-oss") || m.contains("gpt_oss") {
        return Family::GptOss;
    }
    // Coder 系列不产生思考内容。`qwen3_next` 是它的 arch，
    // 名字里则是 `coder`。两个都认，避免换个发布者就漏掉。
    if m.contains("coder") || m.contains("qwen3_next") || m.contains("qwen3-next") {
        return Family::NoThinking;
    }
    Family::BinaryToggle
}

/// 把用户/策略要求的档位，落到这个模型真正支持的那一档上。
///
/// 这是「Off 在 gpt-oss 上要变成 Low」那条规则的唯一实现处。
pub fn resolve(effort: ThinkingEffort, model: &str) -> ThinkingEffort {
    let levels = supported_levels(model);
    if levels.is_empty() {
        return ThinkingEffort::Auto;
    }
    if levels.contains(&effort) {
        return effort;
    }
    match effort {
        // 想省却没有 Off：取支持列表里最省的实档（Auto 不算，它意味着不干预）
        ThinkingEffort::Off => levels
            .iter()
            .copied()
            .find(|l| *l != ThinkingEffort::Auto)
            .unwrap_or(ThinkingEffort::Auto),
        // 想多想却没有档位：退回 Auto，让模型按自己的默认来
        _ => ThinkingEffort::Auto,
    }
}

/// OpenAI 兼容线格式：`reasoning_effort` 的值。`None` 表示这个参数不该出现。
pub fn openai_reasoning_effort(effort: ThinkingEffort, model: &str) -> Option<&'static str> {
    match resolve(effort, model) {
        ThinkingEffort::Auto => None,
        ThinkingEffort::Off => Some("none"),
        ThinkingEffort::Low => Some("low"),
        ThinkingEffort::Medium => Some("medium"),
        ThinkingEffort::High => Some("high"),
    }
}

/// Ollama 原生线格式：`think` 字段。`None` 表示不该出现。
///
/// 原生协议只有布尔，没有档位 —— 所以 Low 及以上都算「开着思考」。
pub fn ollama_think(effort: ThinkingEffort, model: &str) -> Option<bool> {
    match resolve(effort, model) {
        ThinkingEffort::Auto => None,
        ThinkingEffort::Off => Some(false),
        _ => Some(true),
    }
}

/// Anthropic 线格式：`thinking.budget_tokens`。`None` 表示不发 `thinking` 块。
///
/// 预算值参照官方文档的最小可用值（1024）往上取整数档。
pub fn anthropic_budget_tokens(effort: ThinkingEffort) -> Option<u32> {
    match effort {
        ThinkingEffort::Auto | ThinkingEffort::Off => None,
        ThinkingEffort::Low => Some(1024),
        ThinkingEffort::Medium => Some(4096),
        ThinkingEffort::High => Some(16384),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ThinkingEffort::*;

    #[test]
    fn 归族覆盖本机实测过的每一个模型() {
        // 名单和模块注释里的表一一对应。加新模型时两处一起更新。
        assert_eq!(family_of("openai/gpt-oss-120b"), Family::GptOss);
        assert_eq!(family_of("openai/gpt-oss-20b"), Family::GptOss);
        assert_eq!(family_of("qwen/qwen3-coder-next@4bit"), Family::NoThinking);
        for m in [
            "qwen3.6-35b-a3b-uncensored-hauhaucs-aggressive",
            "ornith-1.5-35b-a3b",
            "gemma-4-e4b-uncensored-hauhaucs-aggressive",
            "google/gemma-4-31b",
            "qwen/qwen3.8-27b",
        ] {
            assert_eq!(family_of(m), Family::BinaryToggle, "{m}");
        }
    }

    #[test]
    fn 推理模型不发温度_别的照发() {
        for m in ["gpt-5.6-sol", "gpt-5", "GPT-5-mini", "openai/gpt-5.2", "o3", "o4-mini", "o1-preview"] {
            assert!(!accepts_temperature(m), "{m} 只认默认温度");
        }
        for m in ["claude-opus-5-5", "claude-sonnet-5-5", "claude-fable-5-1", "anthropic/claude-opus-5-5"] {
            assert!(!accepts_temperature(m), "{m}：Claude 5 代起不收 temperature");
        }
        for m in ["gpt-4o", "gpt-4.1", "gpt-oss-120b", "openai/gpt-oss-120b", "claude-sonnet-4-5", "claude-haiku-4-5-20251001", "claude-3-5-sonnet-20241022", "ornith-1.5-35b-a3b-uncensored", "o3de-model", "qwen3.8-27b"] {
            assert!(accepts_temperature(m), "{m} 照常发温度");
        }
    }

    #[test]
    fn gpt_oss_的关要降级成low而不是none() {
        // 这就是修掉的那个静默 bug：gpt-oss 收到 none 会打 WARN 然后忽略，
        // 实测思考量和不发时一模一样（都是 8 tok）。而 low 只有 9 tok。
        let m = "openai/gpt-oss-120b";
        assert_eq!(resolve(Off, m), Low);
        assert_eq!(openai_reasoning_effort(Off, m), Some("low"));
    }

    #[test]
    fn gpt_oss的档位表不含auto因为它和low实测等价() {
        // 不发参数 8 tok、low 9 tok —— 同一件事。放进滑杆只会让用户以为
        // "自动"比"高"更聪明。
        assert_eq!(
            supported_levels("openai/gpt-oss-120b"),
            &[Low, Medium, High]
        );
        // 但存量配置里可能就是 Auto（迁移的默认值），这时不发参数是正确行为
        assert_eq!(openai_reasoning_effort(Auto, "openai/gpt-oss-120b"), None);
    }

    #[test]
    fn 二态模型的关用none档位请求退回auto() {
        let m = "qwen3.6-35b-a3b-uncensored-hauhaucs-aggressive";
        assert_eq!(openai_reasoning_effort(Off, m), Some("none"));
        // 实测 low/medium/high 在这个族上全被忽略（900/900/900，和基线一样），
        // 发出去只是噪声，不如不发让模型走默认。
        assert_eq!(openai_reasoning_effort(Medium, m), None);
        assert_eq!(openai_reasoning_effort(High, m), None);
    }

    #[test]
    fn 不思考的模型什么都不发() {
        let m = "qwen/qwen3-coder-next@4bit";
        assert!(supported_levels(m).is_empty());
        for e in [Off, Low, Medium, High, Auto] {
            assert_eq!(openai_reasoning_effort(e, m), None, "{e:?}");
        }
    }

    #[test]
    fn auto永远不发参数() {
        for m in ["openai/gpt-oss-120b", "ornith-1.5-35b-a3b", "whatever"] {
            assert_eq!(openai_reasoning_effort(Auto, m), None, "{m}");
            assert_eq!(ollama_think(Auto, m), None, "{m}");
        }
    }

    #[test]
    fn 未知模型按二态兜底() {
        // 兜底必须是安全的那一侧：none 被忽略时只是退回默认，不会报错。
        let m = "some-brand-new-model-v9";
        assert_eq!(supported_levels(m), &[Off, Auto]);
        assert_eq!(openai_reasoning_effort(Off, m), Some("none"));
    }

    #[test]
    fn ollama只有布尔所以任何实档都算开() {
        let m = "qwen3:32b";
        assert_eq!(ollama_think(Off, m), Some(false));
        assert_eq!(ollama_think(Auto, m), None);
        // 二态族里 Medium 会被 resolve 成 Auto → 不发
        assert_eq!(ollama_think(Medium, m), None);
    }

    #[test]
    fn anthropic的预算随档位单调递增() {
        assert_eq!(anthropic_budget_tokens(Off), None);
        assert_eq!(anthropic_budget_tokens(Auto), None);
        let (l, m, h) = (
            anthropic_budget_tokens(Low).unwrap(),
            anthropic_budget_tokens(Medium).unwrap(),
            anthropic_budget_tokens(High).unwrap(),
        );
        assert!(l < m && m < h, "{l} {m} {h}");
        assert!(l >= 1024, "Anthropic 的最小可用预算是 1024");
    }

    #[test]
    fn 档位顺序就是从省到费供ui直接用() {
        assert!(Off < Low && Low < Medium && Medium < High);
        // Auto 排在最后：它不是"最费"，而是"不参与排序"，
        // UI 应当把它单独渲染成一个开关而不是滑杆上的一格。
        assert!(High < Auto);
    }
}
