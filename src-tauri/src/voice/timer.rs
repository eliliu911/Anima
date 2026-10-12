//! 定时提醒。
//!
//! ## 为什么它到现在才能做
//!
//! 定时器的本质是「**在未来某个时刻主动说话**」。而在 `initiative` 出现之前，
//! 这个系统 100% 反应式 —— 没有任何一条路能让它在用户不说话时开口。
//! 所以"帮我定 15 分钟"以前不是"工具没写"，是**说出来的通道不存在**。
//!
//! 现在通道有了，定时器只是它的一个使用者。
//!
//! ## 为什么在管线里拦截，不做成普通工具
//!
//! `system_tools::execute` 拿不到 actor 的状态，而定时器必须存在 actor 里
//! （要有人每个 tick 去查有没有到点）。所以和"切换角色"一样，
//! 在管线里拦截这个工具名 —— 那里已经有先例。

use std::time::Instant;

use crate::llm::Tool;

/// 一个待触发的提醒
#[derive(Debug, Clone)]
pub struct Timer {
    pub id: u64,
    pub due: Instant,
    /// 提醒什么。空字符串表示只是"时间到了"
    pub label: String,
}

/// 最长能定多久。超过一天的提醒应该进日历，不是进内存 ——
/// 而且这个进程重启就没了，定太长是骗人
const MAX_MINUTES: f64 = 24.0 * 60.0;
/// 最短。低于这个数没有意义，而且容易是模型把秒当成分钟传错了
const MIN_MINUTES: f64 = 0.1;

pub fn declarations() -> Vec<Tool> {
    vec![
        Tool {
            name: "set_timer".to_string(),
            description: "定一个提醒，到点会主动开口告诉用户。\
                          用户说「定个 15 分钟」「20 分钟后提醒我喝水」时用这个。"
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "minutes": { "type": "number", "description": "多少分钟后提醒" },
                    "label": { "type": "string", "description": "提醒的事，比如「喝水」。没有就留空" }
                },
                "required": ["minutes"]
            }),
        },
        Tool {
            name: "list_timers".to_string(),
            description: "看看现在有哪些还没到点的提醒。".to_string(),
            parameters: serde_json::json!({ "type": "object", "properties": {} }),
        },
        Tool {
            name: "cancel_timer".to_string(),
            description: "取消提醒。不给 id 就取消全部。".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": { "id": { "type": "number" } }
            }),
        },
    ]
}

pub fn is_timer_call(name: &str) -> bool {
    matches!(name, "set_timer" | "list_timers" | "cancel_timer")
}

/// 解析出「几分钟」。返回 Err 时里面是给模型看的说明 ——
/// **要让它知道怎么改**，只说"参数错误"它会原样重试
pub fn minutes_from_arguments(raw: &str) -> Result<(f64, String), String> {
    let v: serde_json::Value =
        serde_json::from_str(raw).map_err(|_| "参数不是合法 JSON".to_string())?;
    let minutes = v
        .get("minutes")
        .and_then(|m| {
            m.as_f64()
                .or_else(|| m.as_str().and_then(|s| s.parse().ok()))
        })
        .ok_or_else(|| "缺少 minutes（数字，单位分钟）".to_string())?;
    if !minutes.is_finite() || minutes < MIN_MINUTES {
        return Err(format!(
            "时间太短了（{minutes} 分钟）。最少 {MIN_MINUTES} 分钟"
        ));
    }
    if minutes > MAX_MINUTES {
        return Err(format!(
            "最长只能定 {MAX_MINUTES} 分钟（一天）。更久的提醒请用日历 —— \
             这个提醒存在内存里，应用重启就没了"
        ));
    }
    let label = v
        .get("label")
        .and_then(|l| l.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    Ok((minutes, label))
}

/// 人能听懂的时长。`90` → 「1 小时 30 分钟」
pub fn humanize(minutes: f64) -> String {
    // **判断要在取整之前**：0.5 分钟 round 成 1，会被说成"1 分钟"
    if minutes < 1.0 {
        return format!("{} 秒", (minutes * 60.0).round() as i64);
    }
    let total = minutes.round() as i64;
    match (total / 60, total % 60) {
        (0, m) => format!("{m} 分钟"),
        (h, 0) => format!("{h} 小时"),
        (h, m) => format!("{h} 小时 {m} 分钟"),
    }
}

/// 取出所有到点的提醒，剩下的留在原地。
///
/// 一次可能到点好几个（应用挂起后恢复），全部取出来一起说 ——
/// 一个一个报会连着说好几轮，很烦
pub fn take_due(timers: &mut Vec<Timer>, now: Instant) -> Vec<Timer> {
    let mut due = Vec::new();
    timers.retain(|t| {
        if t.due <= now {
            due.push(t.clone());
            false
        } else {
            true
        }
    });
    due
}

/// 到点时说什么。多个一起到点就并列说
pub fn announcement(due: &[Timer]) -> String {
    match due {
        [] => String::new(),
        [one] if one.label.is_empty() => "时间到了".to_string(),
        [one] => format!("「{}」的时间到了", one.label),
        many => {
            let labels: Vec<&str> = many
                .iter()
                .map(|t| {
                    if t.label.is_empty() {
                        "定时"
                    } else {
                        t.label.as_str()
                    }
                })
                .collect();
            format!("这几个提醒同时到点了：{}", labels.join("、"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn parses_minutes_and_label() {
        let (m, l) = minutes_from_arguments(r#"{"minutes": 15, "label": "喝水"}"#).unwrap();
        assert_eq!(m, 15.0);
        assert_eq!(l, "喝水");
    }

    /// 小模型经常把数字写成字符串
    #[test]
    fn accepts_a_stringly_typed_number() {
        assert_eq!(
            minutes_from_arguments(r#"{"minutes": "15"}"#).unwrap().0,
            15.0
        );
    }

    /// 报错要**告诉模型怎么改**。只说"参数错误"它会原样重试
    #[test]
    fn errors_explain_how_to_fix() {
        let e = minutes_from_arguments(r#"{"label": "喝水"}"#).unwrap_err();
        assert!(e.contains("minutes"), "实际：{e}");

        let too_long = minutes_from_arguments(r#"{"minutes": 100000}"#).unwrap_err();
        assert!(too_long.contains("日历"), "要给替代方案：{too_long}");

        // 应用重启就没了，这件事必须说出来而不是假装能定一年
        assert!(too_long.contains("重启"));
    }

    #[test]
    fn rejects_nonsense_durations() {
        assert!(minutes_from_arguments(r#"{"minutes": 0}"#).is_err());
        assert!(minutes_from_arguments(r#"{"minutes": -5}"#).is_err());
        assert!(minutes_from_arguments("不是 json").is_err());
    }

    #[test]
    fn humanizes_durations() {
        assert_eq!(humanize(15.0), "15 分钟");
        assert_eq!(humanize(60.0), "1 小时");
        assert_eq!(humanize(90.0), "1 小时 30 分钟");
        assert_eq!(humanize(0.5), "30 秒");
    }

    #[test]
    fn only_due_timers_are_taken() {
        let now = Instant::now();
        let mut timers = vec![
            Timer {
                id: 1,
                due: now - Duration::from_secs(1),
                label: "到了".into(),
            },
            Timer {
                id: 2,
                due: now + Duration::from_secs(600),
                label: "还早".into(),
            },
        ];
        let due = take_due(&mut timers, now);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, 1);
        assert_eq!(timers.len(), 1, "没到点的必须留着");
        assert_eq!(timers[0].id, 2);
    }

    /// 应用挂起后恢复，可能一次到点好几个。**一起说** ——
    /// 一个一个报会连着说好几轮，很烦
    #[test]
    fn several_due_at_once_are_announced_together() {
        let now = Instant::now();
        let due = vec![
            Timer {
                id: 1,
                due: now,
                label: "喝水".into(),
            },
            Timer {
                id: 2,
                due: now,
                label: "站起来".into(),
            },
        ];
        let text = announcement(&due);
        assert!(text.contains("喝水") && text.contains("站起来"));
    }

    #[test]
    fn a_timer_without_a_label_still_says_something() {
        let now = Instant::now();
        let due = vec![Timer {
            id: 1,
            due: now,
            label: String::new(),
        }];
        assert_eq!(announcement(&due), "时间到了");
    }

    #[test]
    fn nothing_due_says_nothing() {
        assert!(announcement(&[]).is_empty());
    }
}
