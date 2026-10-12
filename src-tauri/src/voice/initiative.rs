//! 主动开口：什么时候该说话、说哪一类。
//!
//! ## 缺的是什么
//!
//! 在这个模块出现之前，整个系统是 **100% 反应式**的：
//! `start_response()` 是唯一能让它说话的函数，而它只被 `handle_utterance`
//! 调用，后者只被 `CaptureEvent::Utterance` 触发。
//! 也就是说 —— **你不说话，它永远是一尊雕像**。
//!
//! 用户的原话：「你的朋友、爱人陪在你身边的时候，她会偶尔发出一些声音、
//! 做一些事情、主动给你一些信息」。那不是主动性不够，是这条路不存在。
//!
//! ## 为什么这块必须是纯函数
//!
//! **主动开口的失败模式比沉默严重得多。** 一个在你专心工作时不停搭话的
//! 助手，比一个不说话的助手糟糕一个数量级 —— 而且用户对它的容忍度极低，
//! 烦一次就会永久关掉这个功能。
//!
//! 所以"什么时候开口"必须能被测试钉死，不能散在 actor 的分支里。
//! 这里只做决策，不碰时钟、不碰模型、不发事件。
//!
//! ## 三道闸门
//!
//! 1. **绝不打断** —— 只在空闲态评估（调用方保证）
//! 2. **要有理由** —— 随机闲聊比沉默更差。每一种触发都对应一个真实状态
//! 3. **有冷却和配额** —— 说过就要等，而且每小时有上限

use std::time::Duration;

use crate::personality::EmotionState;

use super::InteractionMode;

/// 主动开口的理由。**每一种都对应一个真实的状态**，不是随机闲聊。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Initiative {
    /// 很久没说话了，问一句。最常见的一种
    Idle,
    /// 隔了很长时间（比如一整天）之后第一次见面
    Reunion,
    /// 心情持续低落 —— 这时候的主动搭话是有意义的
    Concern,
    /// 后台任务做完了，回报结果。**这一类不受闲聊配额限制**
    TaskDone { summary: String },
    /// 定时提醒到点了。和任务回报一样是承诺，同样不受限制
    Reminder { what: String },
}

/// 主动开口时手上有什么可说的。
///
/// **这是这个功能的成败所在。** 没有素材的话，模型只会说
/// 「在忙什么呀」这种废话 —— 那还不如不说。真正让人觉得"她记得我"的，
/// 是能提起一件**具体的**事。
///
/// 而主动开口时**没有查询词**（用户没说话），所以不能用语义召回那条路。
/// 改成直接从记忆里按时间倒序取，靠游标轮换避免每次都提同一件事。
#[derive(Debug, Clone, Default)]
pub struct Material {
    /// 刚才聊了什么（会话内的滚动摘要）
    pub recent: Option<String>,
    /// 长期记忆里的几条事实
    pub facts: Vec<String>,
    /// 距上次交互多少小时。用来决定说"刚才"还是"上次"
    pub hours_since: f32,
}

/// 从事实列表里挑几条。
///
/// 用**游标轮换**而不是随机：随机会重复（连着两次提同一件事很出戏），
/// 而且没法测。游标每次前进，自然轮遍所有事实。
pub fn pick_facts(all: &[String], cursor: usize, n: usize) -> Vec<String> {
    if all.is_empty() || n == 0 {
        return Vec::new();
    }
    (0..n.min(all.len()))
        .map(|i| all[(cursor + i) % all.len()].clone())
        .collect()
}

impl Initiative {
    /// 写进提示词的那段：你为什么开口 + 手上有什么可说的。
    ///
    /// 三条硬约束写进提示词本身：**挑一件**（不要罗列）、
    /// **像想起来**（不要复述）、**没有就别编**（模型拿到事实会顺手编邻近细节）。
    pub fn nudge(&self, m: &Material) -> String {
        let mut s = String::new();
        match self {
            Self::Idle => s.push_str(
                "（你们已经安静了一会儿。你想起点什么，于是自然地开口 —— \
                 一句话就好，别像在完成任务。）",
            ),
            Self::Reunion => {
                s.push_str("（隔了挺久没见了。打个招呼，可以顺口提一句以前聊过的事。）")
            }
            Self::Concern => s.push_str(
                "（他刚才情绪不太好，而且已经安静一会儿了。轻轻问一句，\
                 别追问、别给建议，让他知道你在就行。）",
            ),
            Self::Reminder { what } => {
                // 提醒的措辞和任务回报不一样：这是"我说好要叫你的"，
                // 不是"我帮你做完了一件事"
                s.push_str(&format!(
                    "（{what} —— 你之前答应过要提醒他的，现在叫他一声。\
                     一句话，别啰嗦，也别解释你是怎么记住的。）"
                ));
                return s;
            }
            Self::TaskDone { summary } => {
                s.push_str(&format!(
                    "（你刚才在后台做的事已经完成了：{summary}。\
                     主动告诉他结果 —— 一两句说清楚，不要复述过程。）"
                ));
                // 任务回报不需要别的素材，说清楚结果就行
                return s;
            }
        }

        let mut has_material = false;
        if let Some(recent) = &m.recent {
            if !recent.trim().is_empty() {
                s.push_str("\n\n刚才聊过的：\n");
                s.push_str(recent.trim());
                has_material = true;
            }
        }
        if !m.facts.is_empty() {
            s.push_str("\n\n你记得关于他的一些事：\n");
            for f in &m.facts {
                s.push_str("- ");
                s.push_str(f);
                s.push('\n');
            }
            has_material = true;
        }

        if has_material {
            s.push_str(
                "\n**从上面挑一件**来开口，只挑一件。要像是自然想起来的，\
                 不要复述、不要罗列、不要说\"我记得你说过\"这种句式。\
                 上面没写到的细节**一个字都不要编** —— 编错了比不提更伤。",
            );
        } else {
            s.push_str(
                "\n手上没有具体可说的事，那就别硬找话题 —— \
                 简单问一句他在忙什么就够了。**不要编**任何你们没聊过的事。",
            );
        }
        s
    }

    /// 闲聊类的要占配额，任务回报不占 —— 事情办完了必须说，这是承诺
    pub fn counts_toward_quota(&self) -> bool {
        !matches!(self, Self::TaskDone { .. } | Self::Reminder { .. })
    }
}

/// 决策所需的全部输入。**显式传进来，不在内部读时钟** —— 否则没法测
#[derive(Debug, Clone)]
pub struct Context {
    /// 距离上一次交互（用户说话或它说话）过了多久
    pub since_last_turn: Duration,
    /// 距离上一次**主动开口**过了多久。从没主动过就给一个很大的值
    pub since_last_initiative: Duration,
    /// 最近一小时已经主动开口几次（不含任务回报）
    pub initiatives_this_hour: u32,
    /// 当前情感轴
    pub emotion: EmotionState,
    /// 陪伴 / 助手
    pub mode: InteractionMode,
    /// 用户是否可能不在（比如应用在后台）。不确定时给 false
    pub user_likely_away: bool,
    /// 有没有做完待回报的后台任务
    pub pending_report: Option<String>,
    /// 有没有到点待说的提醒。**排在任务回报之前** ——
    /// 提醒是有时效的，晚说就没意义了
    pub pending_reminder: Option<String>,
}

/// 安静多久之后可以搭话
const IDLE_AFTER: Duration = Duration::from_secs(8 * 60);
/// 多久算"隔了很久没见"
const REUNION_AFTER: Duration = Duration::from_secs(6 * 60 * 60);
/// 两次主动开口之间至少隔多久
const COOLDOWN: Duration = Duration::from_secs(12 * 60);
/// 每小时最多主动几次。**这个数字宁小勿大** ——
/// 被烦到一次，用户就会永久关掉这个功能
const MAX_PER_HOUR: u32 = 3;
/// 心情低到什么程度值得关心一下
const CONCERN_MOOD: f32 = -30.0;

/// 现在该不该开口，开口说哪一类。
///
/// 返回 `None` 表示"继续安静"。**默认就是安静** —— 每一条判断都是在
/// 给"说话"找理由，找不到就不说。
pub fn decide(ctx: &Context) -> Option<Initiative> {
    // 提醒排在最前：它有时效，晚说就没意义了
    if let Some(what) = &ctx.pending_reminder {
        return Some(Initiative::Reminder { what: what.clone() });
    }

    // 任务回报次之，而且不受任何闲聊限制 ——
    // 事情办完了不告诉用户，那是失信，不是"安静"
    if let Some(summary) = &ctx.pending_report {
        return Some(Initiative::TaskDone {
            summary: summary.clone(),
        });
    }

    // 助手模式不闲聊。它的主动性只体现在把事办完之后回报
    if ctx.mode == InteractionMode::Assistant {
        return None;
    }

    // 人可能不在，对着空房间说话很傻
    if ctx.user_likely_away {
        return None;
    }

    if ctx.since_last_initiative < COOLDOWN {
        return None;
    }
    if ctx.initiatives_this_hour >= MAX_PER_HOUR {
        return None;
    }

    // 久别重逢优先于普通搭话
    if ctx.since_last_turn >= REUNION_AFTER {
        return Some(Initiative::Reunion);
    }
    if ctx.since_last_turn < IDLE_AFTER {
        return None;
    }
    if ctx.emotion.mood <= CONCERN_MOOD {
        return Some(Initiative::Concern);
    }
    Some(Initiative::Idle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Context {
        Context {
            since_last_turn: Duration::from_secs(10 * 60),
            since_last_initiative: Duration::from_secs(60 * 60),
            initiatives_this_hour: 0,
            emotion: EmotionState {
                affection: 50.0,
                mood: 0.0,
                energy: 50.0,
            },
            mode: InteractionMode::Companion,
            user_likely_away: false,
            pending_report: None,
            pending_reminder: None,
        }
    }

    #[test]
    fn stays_quiet_by_default() {
        // 刚说完话就想再说 —— 不行
        let c = Context {
            since_last_turn: Duration::from_secs(30),
            ..ctx()
        };
        assert_eq!(decide(&c), None);
    }

    #[test]
    fn speaks_up_after_a_long_silence() {
        assert_eq!(decide(&ctx()), Some(Initiative::Idle));
    }

    #[test]
    fn a_very_long_gap_is_a_reunion() {
        let c = Context {
            since_last_turn: Duration::from_secs(8 * 60 * 60),
            ..ctx()
        };
        assert_eq!(decide(&c), Some(Initiative::Reunion));
    }

    #[test]
    fn low_mood_turns_it_into_concern() {
        let c = Context {
            emotion: EmotionState {
                affection: 50.0,
                mood: -50.0,
                energy: 40.0,
            },
            ..ctx()
        };
        assert_eq!(decide(&c), Some(Initiative::Concern));
    }

    /// **这几条比"会开口"重要得多。**
    /// 一个在你专心工作时不停搭话的助手，比不说话的糟糕一个数量级
    #[test]
    fn cooldown_and_quota_hold_it_back() {
        let recent = Context {
            since_last_initiative: Duration::from_secs(60),
            ..ctx()
        };
        assert_eq!(decide(&recent), None, "刚说过就又说 —— 这是骚扰");

        let maxed = Context {
            initiatives_this_hour: MAX_PER_HOUR,
            ..ctx()
        };
        assert_eq!(decide(&maxed), None, "一小时的配额用完了");
    }

    #[test]
    fn never_talks_to_an_empty_room() {
        let away = Context {
            user_likely_away: true,
            ..ctx()
        };
        assert_eq!(decide(&away), None);
    }

    #[test]
    fn assistant_mode_does_not_chat() {
        let a = Context {
            mode: InteractionMode::Assistant,
            since_last_turn: Duration::from_secs(60 * 60),
            ..ctx()
        };
        assert_eq!(decide(&a), None);
    }

    /// **事情办完必须回报** —— 这不是闲聊，是承诺。
    /// 所以它绕过冷却、配额、模式，甚至绕过"人可能不在"
    #[test]
    fn a_finished_task_always_gets_reported() {
        let blocked = Context {
            pending_report: Some("那三个文件已经改好了".into()),
            mode: InteractionMode::Assistant,
            since_last_initiative: Duration::from_secs(1),
            initiatives_this_hour: 99,
            user_likely_away: true,
            since_last_turn: Duration::from_secs(1),
            ..ctx()
        };
        assert_eq!(
            decide(&blocked),
            Some(Initiative::TaskDone {
                summary: "那三个文件已经改好了".into()
            })
        );
    }

    #[test]
    fn task_reports_do_not_eat_the_chat_quota() {
        assert!(!Initiative::TaskDone {
            summary: "x".into()
        }
        .counts_toward_quota());
        assert!(Initiative::Idle.counts_toward_quota());
    }

    fn material() -> Material {
        Material {
            recent: Some("聊了他在改一个语音项目".into()),
            facts: vec!["用户养了一只叫小黑的猫".into()],
            hours_since: 0.3,
        }
    }

    /// 提醒排在任务回报**之前** —— 提醒有时效，晚说就没意义了
    #[test]
    fn a_due_reminder_outranks_a_task_report() {
        let c = Context {
            pending_reminder: Some("「喝水」的时间到了".into()),
            pending_report: Some("文件改好了".into()),
            ..ctx()
        };
        assert_eq!(
            decide(&c),
            Some(Initiative::Reminder {
                what: "「喝水」的时间到了".into()
            })
        );
    }

    /// 提醒同样绕过冷却、配额和模式 —— 说好了要叫他就必须叫
    #[test]
    fn a_reminder_ignores_every_gate() {
        let c = Context {
            pending_reminder: Some("时间到了".into()),
            mode: InteractionMode::Assistant,
            initiatives_this_hour: 99,
            since_last_initiative: Duration::from_secs(1),
            user_likely_away: true,
            ..ctx()
        };
        assert!(matches!(decide(&c), Some(Initiative::Reminder { .. })));
    }

    /// 提醒的措辞不能是"我帮你做完了一件事" —— 那是任务回报的说法
    #[test]
    fn a_reminder_sounds_like_a_reminder() {
        let n = Initiative::Reminder {
            what: "「喝水」的时间到了".into(),
        }
        .nudge(&material());
        assert!(n.contains("喝水"));
        assert!(n.contains("提醒"));
        assert!(!n.contains("后台"), "别把提醒说成后台任务");
        assert!(!n.contains("小黑"), "提醒时扯别的话题是跑题");
    }

    /// 每一类都要有自己的"为什么开口"，否则模型只会说一样的车轱辘话
    #[test]
    fn every_reason_has_its_own_nudge() {
        let m = material();
        let all = [
            Initiative::Idle,
            Initiative::Reunion,
            Initiative::Concern,
            Initiative::TaskDone {
                summary: "做完了".into(),
            },
        ];
        let mut seen = std::collections::HashSet::new();
        for i in &all {
            let n = i.nudge(&m);
            assert!(!n.is_empty());
            assert!(seen.insert(n), "{i:?} 的提示词和别的重复了");
        }
        assert!(all[3].nudge(&m).contains("做完了"));
    }

    /// **这条是第二层的核心。**
    ///
    /// 没有具体素材的话，模型只会说"在忙什么呀"这种废话 ——
    /// 那还不如不说。素材必须真的进到提示词里。
    #[test]
    fn material_actually_reaches_the_prompt() {
        let n = Initiative::Idle.nudge(&material());
        assert!(n.contains("小黑"), "长期记忆没进提示词");
        assert!(n.contains("语音项目"), "会话摘要没进提示词");
    }

    /// 三条硬约束必须写在提示词里，否则模型会罗列 + 编造
    #[test]
    fn the_prompt_forbids_reciting_and_inventing() {
        let n = Initiative::Idle.nudge(&material());
        assert!(n.contains("挑一件"), "不说清楚的话它会把素材全念一遍");
        assert!(n.contains("不要编") || n.contains("不要复述"));
    }

    /// **没素材时更要禁止编造。** 模型宁可说句普通的，也不能凭空造记忆
    #[test]
    fn with_no_material_it_is_told_not_to_invent() {
        let n = Initiative::Idle.nudge(&Material::default());
        assert!(n.contains("不要编"));
        assert!(
            !n.contains("你记得关于他的一些事"),
            "没有事实就不该有这一段"
        );
    }

    /// 任务回报只说结果，不需要也不该带上闲聊素材
    #[test]
    fn a_task_report_does_not_drag_in_chat_material() {
        let n = Initiative::TaskDone {
            summary: "文件改好了".into(),
        }
        .nudge(&material());
        assert!(n.contains("文件改好了"));
        assert!(!n.contains("小黑"), "回报结果时扯猫是跑题");
    }

    /// 游标轮换：连着几次主动开口，提到的事实要换 ——
    /// 每次都提同一件事很出戏
    #[test]
    fn facts_rotate_instead_of_repeating() {
        let all: Vec<String> = (1..=5).map(|i| format!("事实{i}")).collect();
        let first = pick_facts(&all, 0, 2);
        let second = pick_facts(&all, 2, 2);
        assert_eq!(first, vec!["事实1", "事实2"]);
        assert_eq!(second, vec!["事实3", "事实4"]);
        // 转完一圈会回头，但不会越界
        assert_eq!(pick_facts(&all, 4, 2), vec!["事实5", "事实1"]);
    }

    #[test]
    fn picking_from_an_empty_or_short_list_is_safe() {
        assert!(pick_facts(&[], 0, 3).is_empty());
        let one = vec!["只有一条".to_string()];
        assert_eq!(pick_facts(&one, 7, 3), vec!["只有一条"]);
    }
}
