//! 长镜头：一场整段出，不分镜出。
//!
//! # 用户要的
//!
//! 2026-09-20：「我有一个分集……就是要一个 1 分钟的长镜头，就 1 场，
//! 室内打斗，平均 3-5 秒换一次运镜，但是不分镜，就可以勾选这个选项。」
//!
//! # 它和现有结构比想象中贴合
//!
//! 我们的镜头表**本来就是按时间段写画面的**（`segments`）。所以：
//!
//! > **一个长镜头 = N 个窗口，每个窗口一段现成的画面描述。**
//! > 「3-5 秒换一次运镜但不分镜」正好是一个窗口内部的时间段描述。
//!
//! # 硬约束：窗口必须等长
//!
//! `WeeToddH3ChainedTimeline` 的说明是
//! 「Map global timestamps onto **equal-length** H3 windows」——
//! `window_duration_seconds` 只有一个值，所有窗口共用。
//!
//! 所以这一场里的镜头**必须时长相同**，一镜就是一个窗口。
//! 不同长的话得把镜头边界和窗口边界重新对齐，那是另一套东西，
//! 而且会让「这一镜的提示词」和「这个窗口的提示词」不再是一回事。
//!
//! **宁可要求用户把时长拉齐，也不要在底下悄悄重切。**
//!
//! # 另一条硬约束：不能逐窗重做
//!
//! 查遍所有碰 `WEETODD_H3_LATENTS` 的节点，**没有一个能把 latents 写盘或
//! 读回**（2026-09-20 查实）。所以整条链只能在一次提交里跑完：
//! 某一个窗口不满意，只能整条重来。这件事必须在界面上说，不能只写在注释里。

use crate::cinema::StoryboardShot;

/// 重叠多少帧的**默认值**。节点给的是四选一：5 / 22 / 39 / 56
/// （就是 H3 的 17n+5 潜空间栅格）。
///
/// 22 帧 ≈ 0.92 秒 @24fps，是上游推荐值，也是 Codex 查到的那个开源实现用的数。
pub const CONTEXT_FRAMES: u32 = 22;

/// 节点认的全部档位。**不在这张表里的值节点会拒**。
pub const CONTEXT_FRAME_CHOICES: [u32; 4] = [5, 22, 39, 56];

/// 这一档重叠该配什么说明。**短窗口配大重叠是在白烧算力** ——
/// 3 秒窗口配 22 帧，31% 的采样发布时会被裁掉。
pub fn context_frames_hint(frames: u32, window_secs: f32) -> String {
    let secs = frames as f32 / 24.0;
    let share = secs / window_secs.max(0.1);
    let base = match frames {
        5 => "最小重叠：落在词中间的概率最小，但运动衔接最弱",
        22 => "推荐：运动和声音的上下文足够，上游默认",
        39 | 56 => "大重叠：衔接最稳，但被裁掉的采样也最多",
        _ => "非标准档位，节点可能会拒",
    };
    format!("{base}。{secs:.2} 秒，占窗口的 {:.0}%", share * 100.0)
}

/// H3 一个窗口最长多少秒。和 `studio::catalog()` 里 h3 的 `duration.max` 是同一个数。
pub const MAX_WINDOW_SECS: f32 = 15.0;

/// 最短多少秒。同样取自能力表（h3 的 `duration.min`）。
pub const MIN_WINDOW_SECS: f32 = 2.5;

/// 低于这个秒数是**实验性**档位。
///
/// 上游的说明：低于 5 秒会**向上吸附到 H3 的 17n+5 帧栅格**，
/// 实际时长可能比填的长一点（3.0 秒 → 73 帧 → 3.04 秒）。
/// 不说的话用户会以为是我们算错了时长。
pub const EXPERIMENTAL_BELOW_SECS: f32 = 5.0;

/// 一个窗口实际会出多少帧。
///
/// **H3 的 latent 时间维要求帧数落在 `17n+5` 上**，不落在栅格上的会被
/// 向上吸附 —— 和出片那条走的是同一个
/// [`crate::agent_runtime::video_tool::snap_frames_17n_plus_5`]。
pub fn window_frames(window_secs: f32) -> u32 {
    crate::agent_runtime::video_tool::snap_frames_17n_plus_5((window_secs * 24.0).round() as u32)
}

/// 去掉重叠之后的成片时长。
///
/// # 为什么不是 `n × 窗口秒数 − (n−1) × 重叠`
///
/// 那是最直觉的写法，也是这里原本写着的那份 —— **它漏了帧栅格吸附**，
/// 而实测对不上：4 个 4 秒窗口按那个公式是 13.25 秒，真跑出来的文件是
/// **15.08 秒**（362 帧）；3 个 3 秒窗口公式给 7.17 秒，实际 7.29 秒（175 帧）。
///
/// 少算一两秒本身不致命，致命的是**它会让人以为窗口配少了**：
/// 想要 15 秒的人看到 13.25 会去加窗口，而他本来就已经到 15 秒了。
///
/// 前端 `components/studio/storyboard/longTake.ts` 的 `netSecs` 是同一份公式。
pub fn net_secs(window_secs: f32, windows: usize, frames: u32) -> f32 {
    let f = window_frames(window_secs) as f32;
    (f + (windows as f32 - 1.0) * (f - frames as f32)) / 24.0
}

/// 一场整段出的排布。
#[derive(Debug, Clone, PartialEq)]
pub struct LongTakePlan {
    /// 每个窗口多少秒（所有窗口相同）
    pub window_secs: f32,
    /// 按镜号排好的窗口（一镜一个窗口）
    pub shot_ids: Vec<String>,
    pub context_frames: u32,
    /// 去掉重叠之后的成片时长
    pub net_secs: f32,
    /// 重叠占一个窗口的比例。**短窗口会很难看** ——
    /// 3 秒窗口配 22 帧重叠是 30%，那三成算力发布时会被裁掉。
    pub overlap_share: f32,
    /// 非空 = 能跑，但有话要说（实验性档位）。
    pub warning: Option<String>,
}

/// 为什么这一场整段出不了。**每一条都要能直接告诉用户怎么改。**
#[derive(Debug, Clone, PartialEq)]
pub enum LongTakeBlocker {
    /// 一场只有一镜 —— 那就是普通出片，不需要长镜头
    OnlyOneShot,
    /// 镜头时长不一样。带上实际有哪几种，用户才知道要改成什么
    UnequalDurations(Vec<f32>),
    /// 单个窗口超过 H3 的上限
    WindowTooLong { secs: f32 },
    /// 单个窗口短于 H3 的下限
    WindowTooShort { secs: f32 },
}

impl LongTakeBlocker {
    pub fn message(&self) -> String {
        match self {
            LongTakeBlocker::OnlyOneShot => {
                "这一场只有一镜 —— 直接出片就行，不需要长镜头".into()
            }
            LongTakeBlocker::UnequalDurations(secs) => {
                let mut s: Vec<String> = secs.iter().map(|v| format!("{v:.0}")).collect();
                s.dedup();
                format!(
                    "长镜头的窗口必须等长，而这一场的镜头是 {} 秒 —— 把它们改成同一个时长再试（H3 的窗口时长只有一个值，所有窗口共用）",
                    s.join(" / ")
                )
            }
            LongTakeBlocker::WindowTooLong { secs } => format!(
                "每一镜 {secs:.0} 秒，超过 H3 单窗口的 {MAX_WINDOW_SECS:.0} 秒上限 —— 把这一场拆成更多、更短的镜头"
            ),
            LongTakeBlocker::WindowTooShort { secs } => format!(
                "每一镜只有 {secs:.1} 秒，短于 H3 单窗口的 {MIN_WINDOW_SECS} 秒下限 —— 把镜头拉长一点"
            ),
        }
    }
}

/// 给一场排布长镜头。`shots` 是这一场的镜头，**不要求已排序**。
///
/// 成片时长 = `n × window - (n-1) × 重叠`，因为每个窗口的前 22 帧是上一个
/// 窗口的尾巴，发布时会被裁掉（`WeeToddH3DirectPublishChain` 干这件事）。
pub fn plan(shots: &[&StoryboardShot]) -> std::result::Result<LongTakePlan, LongTakeBlocker> {
    plan_with(shots, CONTEXT_FRAMES)
}

/// 同上，指定重叠帧数。见 [`CONTEXT_FRAME_CHOICES`]。
pub fn plan_with(shots: &[&StoryboardShot], frames: u32) -> std::result::Result<LongTakePlan, LongTakeBlocker> {
    if shots.len() < 2 {
        return Err(LongTakeBlocker::OnlyOneShot);
    }
    let mut ordered: Vec<&&StoryboardShot> = shots.iter().collect();
    ordered.sort_by_key(|s| s.shot_num);

    let window = ordered[0].duration;
    // 用半帧当容差：界面上是 0.5 秒一档，浮点相等靠不住
    if ordered.iter().any(|s| (s.duration - window).abs() > 1.0 / 48.0) {
        return Err(LongTakeBlocker::UnequalDurations(
            ordered.iter().map(|s| s.duration).collect(),
        ));
    }
    if window > MAX_WINDOW_SECS {
        return Err(LongTakeBlocker::WindowTooLong { secs: window });
    }
    if window < MIN_WINDOW_SECS {
        return Err(LongTakeBlocker::WindowTooShort { secs: window });
    }
    let overlap = frames as f32 / 24.0;
    Ok(LongTakePlan {
        window_secs: window,
        shot_ids: ordered.iter().map(|s| s.id.clone()).collect(),
        context_frames: frames,
        net_secs: net_secs(window, ordered.len(), frames),
        overlap_share: overlap / window,
        // **能跑，但要说出来。** 静默吸附之后时长对不上，用户查不出原因。
        warning: (window < EXPERIMENTAL_BELOW_SECS).then(|| format!(
            "每一镜只有 {window:.1} 秒，低于 {EXPERIMENTAL_BELOW_SECS:.0} 秒是 H3 的实验性档位：\
             实际时长会向上吸附到 17n+5 帧栅格（比填的长一点），\
             而且 {:.0} 秒的重叠占了窗口的 {:.0}%，这部分算力发布时会被裁掉。\
             适合验管道，不适合评内容。",
            overlap, overlap / window * 100.0
        )),
    })
}

/// 预计要跑多久（秒）。**给界面显示代价用，不是精确值。**
///
/// 4.5 分钟算力 / 每秒成片，来自 2026-09-20 实测（15 秒 / 576×1024 / 5 步 /
/// q8 用了 68 分钟，见 `MEDIA_PIPELINES` 4.1）。按**总窗口时长**算而不是
/// 净时长 —— 重叠那部分也要真的采样出来。
pub fn eta_secs(plan: &LongTakePlan) -> f32 {
    plan.window_secs * plan.shot_ids.len() as f32 * 4.5 * 60.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn 镜(num: u32, secs: f32) -> StoryboardShot {
        let mut s: StoryboardShot = serde_json::from_value(serde_json::json!({
            "id": format!("s{num}"), "shot_num": num, "scale": "", "camera_movement": "",
            "dialogue": "", "prompt": "", "duration": secs, "status": "pending"
        }))
        .unwrap();
        s.scene_no = 1;
        s
    }

    #[test]
    fn 等长的多镜排得出窗口链() {
        let a = 镜(1, 15.0);
        let b = 镜(2, 15.0);
        let c = 镜(3, 15.0);
        let p = plan(&[&c, &a, &b]).unwrap(); // **故意乱序**，要按镜号排
        assert_eq!(p.shot_ids, ["s1", "s2", "s3"]);
        assert_eq!(p.window_secs, 15.0);
        assert_eq!(p.context_frames, 22);
        // 3 个 15 秒窗口：每窗吸附到 362 帧，总 1042 帧
        assert!((p.net_secs - 1042.0 / 24.0).abs() < 0.05, "{}", p.net_secs);
    }

    /// **窗口必须等长** —— 上游节点只有一个 `window_duration_seconds`。
    /// 报错里要写出实际有哪几种，不然用户不知道改成什么。
    #[test]
    fn 时长不一样要拦下来并说清是哪几种() {
        let a = 镜(1, 15.0);
        let b = 镜(2, 10.0);
        let e = plan(&[&a, &b]).unwrap_err();
        assert_eq!(e, LongTakeBlocker::UnequalDurations(vec![15.0, 10.0]));
        assert!(e.message().contains("15 / 10"), "{}", e.message());
    }

    /// 0.5 秒一档的界面上，浮点相等靠不住 —— 半帧以内当相同。
    #[test]
    fn 半帧以内的差当成等长() {
        let a = 镜(1, 15.0);
        let mut b = 镜(2, 15.0);
        b.duration = 15.0 + 1.0 / 60.0;
        assert!(plan(&[&a, &b]).is_ok());
    }

    #[test]
    fn 一镜不算长镜头() {
        let a = 镜(1, 15.0);
        assert_eq!(plan(&[&a]).unwrap_err(), LongTakeBlocker::OnlyOneShot);
        assert!(plan(&[]).is_err());
    }

    #[test]
    fn 单窗口超过上限要拦() {
        let a = 镜(1, 20.0);
        let b = 镜(2, 20.0);
        let e = plan(&[&a, &b]).unwrap_err();
        assert_eq!(e, LongTakeBlocker::WindowTooLong { secs: 20.0 });
        assert!(e.message().contains("15 秒上限"), "{}", e.message());
    }

    /// **前后端的常数必须一样。**
    ///
    /// 两边各写各的会出现「界面说能跑，点下去后端拒了」—— 而用户不知道
    /// 该信哪个。这条扫 TS 那份的源码，只对常数（逻辑各写各的没办法，
    /// 但常数漂了是最容易发生、也最容易查的那一半）。
    /// **重叠档位可选，而且每一档要说清代价。**
    ///
    /// 用户 2026-09-20 问「3+3+3 行不行」时带出来的：短窗口配大重叠是在
    /// 白烧算力 —— 3 秒窗口配 22 帧，31% 的采样发布时会被裁掉。
    #[test]
    fn 重叠档位可选_每一档都说得出占多少() {
        let shots: Vec<StoryboardShot> = (1..=3).map(|i| 镜(i, 3.0)).collect();
        let refs: Vec<&StoryboardShot> = shots.iter().collect();

        let big = plan_with(&refs, 22).unwrap();
        let small = plan_with(&refs, 5).unwrap();
        assert_eq!(big.context_frames, 22);
        assert_eq!(small.context_frames, 5);
        // 小重叠 → 净时长更长（裁掉的少）
        assert!(small.net_secs > big.net_secs, "{} vs {}", small.net_secs, big.net_secs);
        assert!(small.overlap_share < 0.08 && big.overlap_share > 0.3);

        for f in CONTEXT_FRAME_CHOICES {
            let h = context_frames_hint(f, 3.0);
            assert!(h.contains('%'), "{f} 帧的说明里没说占比：{h}");
        }
        assert!(context_frames_hint(5, 3.0).contains("运动衔接最弱"), "没说小重叠的代价");
        assert!(context_frames_hint(22, 3.0).contains("推荐"), "没标出推荐档");
    }

    #[test]
    fn 前端的常数和这里一致() {
        let ts = std::fs::read_to_string("../src/components/studio/storyboard/longTake.ts")
            .expect("读不到 longTake.ts —— 文件挪了就把这条测试一起改");
        for (name, want) in [
            ("CONTEXT_FRAMES", CONTEXT_FRAMES.to_string()),
            ("MAX_WINDOW_SECS", format!("{MAX_WINDOW_SECS:.0}")),
            ("MIN_WINDOW_SECS", format!("{MIN_WINDOW_SECS}")),
            ("EXPERIMENTAL_BELOW_SECS", format!("{EXPERIMENTAL_BELOW_SECS:.0}")),
        ] {
            let needle = format!("export const {name} = {want}");
            assert!(ts.contains(&needle), "前端的 {name} 和后端对不上，应为「{needle}」");
        }
        // 耗时那个系数也要一致，否则界面显示 40 分钟、实际跑 4 小时
        assert!(ts.contains("* 4.5) / 60"), "前端的耗时系数（4.5 分钟/秒成片）漂了");
    }

    /// **3+3+3 冒烟档**：能跑，但要把两件事说出来 ——
    /// 实际时长会吸附、重叠占比高到三成。
    ///
    /// 用户 2026-09-20 问的就是这个：「可以先测短一点的，比如 3+3+3」。
    #[test]
    fn 三秒窗口能跑_但要报实验性和重叠占比() {
        let shots: Vec<StoryboardShot> = (1..=3).map(|i| 镜(i, 3.0)).collect();
        let refs: Vec<&StoryboardShot> = shots.iter().collect();
        let p = plan(&refs).unwrap();
        // **实测锚点**：09-20 的 3×3×3 冒烟测试出的是 175 帧 = 7.292 秒
        assert!((p.net_secs - 175.0 / 24.0).abs() < 0.01, "{}", p.net_secs);
        assert!((p.overlap_share - 0.306).abs() < 0.01, "重叠占比 {}", p.overlap_share);
        let w = p.warning.expect("低于 5 秒必须给告警");
        assert!(w.contains("17n+5"), "没说会吸附：{w}");
        assert!(w.contains("31%") || w.contains("30%"), "没说重叠占比：{w}");
        assert!(w.contains("验管道"), "没说清它适合干什么：{w}");
    }

    /// 5 秒及以上不该有告警 —— 每份正文都挂一条假警告，真的那些就没人看了。
    #[test]
    fn 正常档位没有告警() {
        let shots: Vec<StoryboardShot> = (1..=3).map(|i| 镜(i, 15.0)).collect();
        let refs: Vec<&StoryboardShot> = shots.iter().collect();
        let p = plan(&refs).unwrap();
        assert!(p.warning.is_none());
        assert!(p.overlap_share < 0.07, "15 秒窗口的重叠只占 6%：{}", p.overlap_share);
    }

    #[test]
    fn 短于下限要拦() {
        let a = 镜(1, 2.0);
        let b = 镜(2, 2.0);
        let e = plan(&[&a, &b]).unwrap_err();
        assert_eq!(e, LongTakeBlocker::WindowTooShort { secs: 2.0 });
        assert!(e.message().contains("2.5 秒下限"), "{}", e.message());
    }

    /// **用户要的那个例子**：1 分钟室内打斗。
    ///
    /// 4 个 15 秒窗口 ≈ 57.6 秒成片，约 4.5 小时算力 —— 这两个数要能直接
    /// 显示在勾选框旁边，它们就是「勾了要付什么代价」。
    #[test]
    fn 一分钟长镜头的两个数能算出来() {
        let shots: Vec<StoryboardShot> = (1..=4).map(|i| 镜(i, 15.0)).collect();
        let refs: Vec<&StoryboardShot> = shots.iter().collect();
        let p = plan(&refs).unwrap();
        assert!((p.net_secs - 1382.0 / 24.0).abs() < 0.05, "净时长 {}", p.net_secs);
        let hours = eta_secs(&p) / 3600.0;
        assert!((hours - 4.5).abs() < 0.1, "预计 {hours} 小时");
    }
}
