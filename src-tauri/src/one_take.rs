//! 一场一镜出：不分镜，提示词里按时间轴写运镜。
//!
//! # 用户要的
//!
//! 2026-09-23：「剧本镜头生成有一些很离谱的情况：一共就 15 秒，被分成 2 秒的，
//! 根本到 Comfy 里就被 PASS 了，所以我需要一个直接通过在提示词里分时间轴运镜，
//! 这样就省得短视频再分镜了。」
//!
//! 那一场被拆成 4 / 2 / 3 / 4 秒 —— 2 秒低于 H3 的下限 2.5 秒；想勾长镜头又被
//! 「窗口必须等长」挡住。
//!
//! # 为什么这么做就够
//!
//! H3 **一次生成里本来就会切镜头**（Kotte 方案 D 两次打样，7 秒里切了 5~6 刀，
//! 人和房间都稳）。镜头表的每一镜本来就按时间段写（`segments`），
//! 所以「一镜 15 秒、里面按时间轴写切镜和运镜」不是新结构 ——
//! **只是镜数和时长改由代码定，不让模型自己拆。**
//!
//! 超过 15 秒的，按窗口上限**等分**成几镜，再整段出（窗口链）。
//! 等分是窗口链的硬约束（见 [`crate::long_take`]），这里按构造就满足。
//!
//! 设计见 `docs/plans/2026-09-23-一场一镜出方案.md`。

use serde::{Deserialize, Serialize};

/// 一个窗口最长多少秒。和 [`crate::long_take::MAX_WINDOW_SECS`]、镜头表的上限是同一个数。
pub const WINDOW_MAX_SECS: f32 = crate::long_take::MAX_WINDOW_SECS;
/// 一场一镜出最长多少秒。再长就是四个以上窗口 —— 整条只能重来，一次三个多小时，不给默认开
pub const MAX_TOTAL_SECS: f32 = 60.0;
/// 最短多少秒。镜头表的下限（[`crate::shot_table::MIN_SHOT_SECS`]），比 H3 的 2.5 秒下限宽一点
pub const MIN_TOTAL_SECS: f32 = crate::shot_table::MIN_SHOT_SECS;

/// 一场单独的设置。**按场号存在集上**（和 `long_take_scenes` 同一个理由：放镜头上就得维护
/// 「同一场的标记要一样」，而它一定会被破坏）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneTake {
    pub scene_no: u32,
    /// true = 这一场一镜出；false = 这一场照常分镜（用来在「整集不分镜」时把某一场改回去）
    pub one_take: bool,
    /// 手填的总时长。空 = 自动估
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secs: Option<f32>,
}

/// 这一场是不是一镜出；是的话手填的时长是多少（`Some(None)` = 一镜出、时长自动）。
///
/// 单场的设置优先；没设的场跟分集的默认走。
pub fn scene_mode(episode_default: bool, takes: &[SceneTake], scene_no: u32) -> Option<Option<f32>> {
    match takes.iter().find(|t| t.scene_no == scene_no) {
        Some(t) if t.one_take => Some(t.secs),
        Some(_) => None,
        None => episode_default.then_some(None),
    }
}

/// 剧本里写明的总时长：「全片 15 秒」「时长 30 秒」「共 12 秒」。
///
/// 《#上课不要玩手机》《反差示例》那种剧本开头都写了 —— **写了就照它**，
/// 比按台词估准得多（估算看不见「开炮那两秒没人说话」）。
pub fn stated_secs(text: &str) -> Option<f32> {
    for key in ["全片", "时长", "总长", "共"] {
        let mut rest = text;
        while let Some(at) = rest.find(key) {
            let after = &rest[at + key.len()..];
            let digits: String = after
                .trim_start_matches([' ', '约', '：', ':'])
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            let tail = &after.trim_start_matches([' ', '约', '：', ':'])[digits.len()..];
            if let Ok(n) = digits.parse::<f32>() {
                if tail.trim_start().starts_with('秒') && n > 0.0 {
                    return Some(n);
                }
            }
            rest = after;
        }
    }
    None
}

/// 按剧本估一场要多少秒：**台词字数 ÷ 4 + 每一句动作 1.5 秒**，向上取到 0.5 秒。
///
/// 和镜头表规则 3「时长 = 台词字数 ÷ 4 + 动作和反应占的秒数」同一个算法；
/// 「动作占的秒数」这里拿非台词的句子数近似 —— 代码看不懂动作要多久，
/// 所以界面上总是让人能手填。
pub fn estimate_secs(scene_text: &str) -> f32 {
    let lines = crate::script_parse::dialogue_of(scene_text);
    let spoken: usize = lines.iter().map(|(_, l)| l.chars().filter(|c| c.is_alphanumeric()).count()).sum();
    let dialogue_lines: Vec<&str> = lines.iter().map(|(_, l)| l.as_str()).collect();
    let actions = scene_text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('【') && !dialogue_lines.iter().any(|d| l.contains(d)))
        .flat_map(|l| l.split(['。', '！', '？', '；']))
        .filter(|s| s.chars().filter(|c| c.is_alphanumeric()).count() >= 4)
        .count();
    round_half_up(spoken as f32 / 4.0 + actions as f32 * 1.5)
}

/// 这一场一镜出最终多少秒：手填的 > 剧本写明的 > `fallback`（整集只有一场时给分集的目标时长）> 估算。
/// 夹在上下限之间。
pub fn resolve_secs(manual: Option<f32>, scene_text: &str, fallback: Option<f32>) -> f32 {
    let raw = manual
        .or_else(|| stated_secs(scene_text))
        .or(fallback.filter(|s| *s > 0.0))
        .unwrap_or_else(|| estimate_secs(scene_text));
    round_half_up(raw.clamp(MIN_TOTAL_SECS, MAX_TOTAL_SECS))
}

/// 总时长等分成几镜、每镜几秒。每镜不超过一个窗口，**每镜一样长**。
///
/// 15 → 1 × 15；30 → 2 × 15；20 → 2 × 10；31 → 3 × 10.5（向上取 0.5 秒，总长会多一点，不会少）。
pub fn windows(total: f32) -> (u32, f32) {
    let total = total.clamp(MIN_TOTAL_SECS, MAX_TOTAL_SECS);
    let n = (total / WINDOW_MAX_SECS).ceil().max(1.0) as u32;
    (n, round_half_up(total / n as f32).min(WINDOW_MAX_SECS))
}

/// 一场一镜出的计划：几镜、每镜几秒。整集生成和单场重新生成都走这里，两边算的一定一样。
///
/// `scene_count` 用来决定分集的目标时长能不能当这一场的时长 —— 只有一场时才能。
pub fn plan(episode_default: bool, takes: &[SceneTake], scene_no: u32, scene_text: &str, scene_count: usize, target_secs: u32) -> Option<(u32, f32)> {
    let manual = scene_mode(episode_default, takes, scene_no)?;
    let fallback = (scene_count == 1 && target_secs > 0).then_some(target_secs as f32);
    Some(windows(resolve_secs(manual, scene_text, fallback)))
}

/// 模型写回来的时长不对时，**按比例把每一段拉到给定时长**，不重新问模型 ——
/// 段与段的相对节奏是它写的，只是总长对不上。
pub fn fit_duration(shot: &mut crate::cinema::StoryboardShot, secs: f32) {
    let old = shot.duration;
    if old > 0.0 && (old - secs).abs() > 0.01 {
        let k = secs / old;
        for seg in &mut shot.segments {
            seg.start = (seg.start * k * 10.0).round() / 10.0;
            seg.end = (seg.end * k * 10.0).round() / 10.0;
        }
    }
    if let Some(last) = shot.segments.last_mut() {
        last.end = secs;
    }
    shot.duration = secs;
}

fn round_half_up(x: f32) -> f32 {
    ((x * 2.0) - 1e-4).ceil().max(0.0) / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 等分窗口() {
        assert_eq!(windows(15.0), (1, 15.0));
        assert_eq!(windows(12.0), (1, 12.0));
        assert_eq!(windows(30.0), (2, 15.0));
        assert_eq!(windows(20.0), (2, 10.0));
        assert_eq!(windows(31.0), (3, 10.5), "向上取到 0.5 秒，总长只会多不会少");
        assert_eq!(windows(60.0), (4, 15.0));
        assert_eq!(windows(90.0), (4, 15.0), "超过上限按上限");
        assert_eq!(windows(1.0), (1, 4.0), "低于下限按下限");
    }

    #[test]
    fn 每镜都不超过一个窗口() {
        for tenths in 40..=600 {
            let (n, each) = windows(tenths as f32 / 10.0);
            assert!(each <= WINDOW_MAX_SECS && each >= MIN_TOTAL_SECS / n as f32, "{tenths}: {n} × {each}");
            assert!(n as f32 * each + 1e-3 >= (tenths as f32 / 10.0).min(MAX_TOTAL_SECS), "{tenths}: 等分后总长不能比要的短");
        }
    }

    #[test]
    fn 剧本写明的时长() {
        assert_eq!(stated_secs("（类型：反差 / 荒诞打脸；全片 15 秒，一个连续手持长镜头）"), Some(15.0));
        assert_eq!(stated_secs("时长：30 秒"), Some(30.0));
        assert_eq!(stated_secs("全片约 12.5 秒"), Some(12.5));
        assert_eq!(stated_secs("共 3 人"), None, "后面不是「秒」不算");
        assert_eq!(stated_secs("老师在第 12 秒说话"), None, "没有关键词不算");
    }

    #[test]
    fn 时长优先级() {
        let text = "【场景：教室】\n（全片 15 秒）\n老师：你还玩手机！";
        assert_eq!(resolve_secs(Some(30.0), text, Some(20.0)), 30.0, "手填的最优先");
        assert_eq!(resolve_secs(None, text, Some(20.0)), 15.0, "剧本写明的其次");
        assert_eq!(resolve_secs(None, "老师：你还玩手机！", Some(20.0)), 20.0, "再其次是分集的目标时长");
        assert_eq!(resolve_secs(None, "老师：你还玩手机！", None), MIN_TOTAL_SECS, "都没有才估（估出来太短按下限）");
        assert_eq!(resolve_secs(Some(100.0), text, None), MAX_TOTAL_SECS);
        assert_eq!(resolve_secs(Some(1.0), text, None), MIN_TOTAL_SECS);
    }

    #[test]
    fn 估算按台词和动作() {
        // 「谁让你玩手机的交上来」10 个字 → 2.5 秒；两句动作 → 3 秒
        let text = "【场景：教室】\n她把武器架上窗台。老师猛地转过身来。\n老师：谁让你玩手机的交上来";
        assert_eq!(estimate_secs(text), 5.5);
    }

    #[test]
    fn 计划只有一场时才用分集目标时长() {
        assert_eq!(plan(true, &[], 1, "老师：你还玩手机！", 1, 30), Some((2, 15.0)));
        assert_eq!(plan(true, &[], 1, "老师：你还玩手机！", 2, 30), Some((1, 4.0)), "两场时不能把整集时长给一场");
        assert_eq!(plan(false, &[], 1, "（全片 15 秒）", 1, 30), None, "不是一镜出就没有计划");
    }

    #[test]
    fn 时长对不上按比例拉() {
        let mut shot: crate::cinema::StoryboardShot = serde_json::from_value(serde_json::json!({
            "id": "s", "shot_num": 1, "scale": "", "camera_movement": "", "dialogue": "", "prompt": "", "duration": 10.0, "status": "pending",
            "segments": [{"start": 0.0, "end": 4.0}, {"start": 4.0, "end": 10.0}]
        }))
        .unwrap();
        fit_duration(&mut shot, 15.0);
        assert_eq!(shot.duration, 15.0);
        assert_eq!((shot.segments[0].end, shot.segments[1].start, shot.segments[1].end), (6.0, 6.0, 15.0));
    }

    #[test]
    fn 单场设置优先于分集默认() {
        let takes = vec![
            SceneTake { scene_no: 1, one_take: true, secs: Some(30.0) },
            SceneTake { scene_no: 2, one_take: false, secs: None },
        ];
        assert_eq!(scene_mode(false, &takes, 1), Some(Some(30.0)));
        assert_eq!(scene_mode(true, &takes, 2), None, "整集不分镜，但这一场改回了分镜");
        assert_eq!(scene_mode(true, &takes, 3), Some(None), "没单独设的跟分集走，时长自动");
        assert_eq!(scene_mode(false, &takes, 3), None);
    }
}
