//! ComfyUI 正在跑到哪一步。
//!
//! # 为什么不能问 ComfyUI
//!
//! 我们轮询的 `/queue` 和 `/history/{id}` **都不含进度**。步数进度在
//! ComfyUI 的 websocket 上（`progress` 事件），但 **H3 / WeeTodd 的视频节点
//! 不走那套** —— 它自己往 stdout 打：
//!
//! ```text
//! step 1/4  734.8s  eta 36.7 min
//! step 2/4  706.8s  eta 24.0 min
//! ```
//!
//! 2026-09-21 挂 websocket 上听了 12 秒，只收到 `status`，一条 `progress` 都没有。
//! 所以视频那条只能读日志。出图那条走 tqdm（`0%|…| 0/8`），也在同一份日志里。
//!
//! **2026-09-30 更正：H3 的进度在 ws 上是有的**，只是一步十几到三十分钟，12 秒听不到。
//! 那天挂着听了一步：`{"type":"progress","data":{"value":5,"max":6,"prompt_id":…}}`，和预览图同一秒到。
//! 判「任务还在不在动」用的就是它（`agent_runtime::comfy_watch`）；这里的界面进度仍读日志。
//!
//! # 要害：两次 `step` 之间有十几分钟的完全静默
//!
//! 那行 `eta` **只在每步跑完的那一刻打一次**。15 秒的片子是 4 步、每步约 12 分钟
//! —— 一小时里只有 4 个时间点有信息。照日志原样显示的话界面 12 分钟不动一次，
//! 和转圈没差多少（用户原话：「Comfy 上也没有准确完成时间」）。
//!
//! 所以要**在步之间插值**，而预估来自**这一次任务自己前几步的实测耗时**
//! （734.8 / 706.8，差 4%，非常稳），每打出一行 `step` 就重新校准。
//!
//! # 和 DramaClaw 那条曲线的区别
//!
//! DC 的 `NodeGenerationOverlay`（Elastic-2.0，参考其做法）是按一个**写死的**
//! 预估时长走指数饱和曲线，注释原话「用于模拟进度」：跑偏了不会修正，
//! 一路爬到 99% 封顶。
//!
//! 我们的预估每步校准一次；而且**插值到头就停住**，显示「已超出预估 N 分钟」
//! —— 超时本身是要被看见的信息，爬到 99% 反而把它藏起来了。
//!
//! # 拿不到就说拿不到
//!
//! 日志读不到、格式变了（那行是 WeeTodd 自己的，它换版本就可能变）——
//! 一律退回「已跑 N 分钟」，**不显示百分比**。编一个数字出来，
//! 和这个项目反复踩的「任务显示成功、结果是错的」是同一类病。

use std::path::{Path, PathBuf};

/// 日志末尾读多少字节。一步一行，几十行足够；整份日志可能有几十 MB。
const TAIL_BYTES: u64 = 64 * 1024;

/// 从日志里认出来的一步。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Step {
    /// 已经跑完第几步
    pub done: u32,
    pub total: u32,
    /// 这一步花了多少秒。**下一步的预估就用它**
    pub secs: f32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Progress {
    /// 给人看的一句话。**永远有值** —— 拿不到步数时是「已跑 N 分钟」
    pub text: String,
    /// 已经跑完第几步。拿不到时 `None`
    pub step: Option<u32>,
    pub total: Option<u32>,
    /// 估计还要多少秒。**只在拿到步数时给** —— 拿不到就别编
    pub eta_secs: Option<u32>,
}

/// `step 2/4  706.8s  eta 24.0 min` → `Step`。
///
/// **只认它现在这个格式，认不出就回 `None`。** 宽松匹配（比如「随便找两个数字」）
/// 在格式变了之后会解析出垃圾，而垃圾会被当成真进度显示出去 ——
/// 那比没有进度糟得多。
pub fn parse_step(line: &str) -> Option<Step> {
    let rest = line.trim().strip_prefix("step ")?;
    let (frac, tail) = rest.split_once(char::is_whitespace)?;
    let (done, total) = frac.split_once('/')?;
    // `706.8s  eta 24.0 min` —— 取第一个 token，去掉结尾的 `s`
    let secs = tail.split_whitespace().next()?.strip_suffix('s')?;
    Some(Step {
        done: done.parse().ok()?,
        total: total.parse().ok()?,
        secs: secs.parse().ok()?,
    })
}

/// 日志末尾**最后一行** `step`。前面的步已经被它取代了。
pub fn last_step(tail: &str) -> Option<Step> {
    // WeeTodd 用 `\r` 刷同一行，tqdm 也是 —— 得按两种换行都切
    tail.split(['\n', '\r']).filter_map(parse_step).next_back()
}

/// 这一轮任务是不是刚开始（上一次任务的 `step` 行还留在日志里）。
///
/// **判据是 `got prompt` 之后还有没有 `step`。** 不看这个的话，
/// 新任务的头十几分钟会显示上一个任务的「第 4/4 步」—— 一个看着很正常、
/// 其实完全错的进度。
pub fn steps_since_last_prompt(tail: &str) -> Option<Step> {
    let at = tail.rfind("got prompt")?;
    last_step(&tail[at..])
}

/// 给人看的一句话。
///
/// - `step`：日志里最后一行 `step`（同一轮任务里的）
/// - `since_step`：距那一行打出来过了多少秒
/// - `elapsed`：这个任务总共跑了多少秒
pub fn describe(step: Option<Step>, since_step: f32, elapsed: f32) -> Progress {
    let mins = |s: f32| (s / 60.0).round().max(0.0) as u32;
    match step {
        // 拿到步数：按上一步的实测耗时插值
        Some(s) if s.done < s.total && s.secs > 0.0 => {
            let left_in_this = s.secs - since_step;
            let remaining_steps = (s.total - s.done - 1) as f32;
            if left_in_this <= 0.0 {
                // **插值到头了就停住，不往前爬。** 超时是要被看见的信息
                return Progress {
                    text: format!(
                        "第 {}/{} 步 · 已超出预估 {} 分钟",
                        s.done + 1, s.total, mins(-left_in_this).max(1)
                    ),
                    step: Some(s.done),
                    total: Some(s.total),
                    eta_secs: None,
                };
            }
            let eta = left_in_this + remaining_steps * s.secs;
            Progress {
                text: format!("第 {}/{} 步 · 约还要 {} 分钟", s.done + 1, s.total, mins(eta).max(1)),
                step: Some(s.done),
                total: Some(s.total),
                eta_secs: Some(eta.round().max(0.0) as u32),
            }
        }
        // 最后一步也跑完了：在解码 / 存盘
        Some(s) => Progress {
            text: format!("{}/{} 步跑完 · 正在解码存盘", s.total, s.total),
            step: Some(s.done),
            total: Some(s.total),
            eta_secs: None,
        },
        // **拿不到就说拿不到，不编百分比。**
        None => Progress {
            text: format!("已跑 {} 分钟", mins(elapsed)),
            step: None,
            total: None,
            eta_secs: None,
        },
    }
}

/// **出片任务卡片上的一句进度**：排队还是在跑、第几步、百分比、还要多久、已经跑了多久。
/// 回 `(已完成步数, 总步数, 这句话, 还要多少秒)`。
///
/// 2026-10-05 用户：「能不能把 ComfyUI 里的进度列出来，比如 1/4、83%、预计完成于」。
///
/// - 步数优先用日志（`log`，带每步实测耗时、ETA 准）；读不到日志（自己起的 ComfyUI）用 ws 的 `value/max`（`ws`），
///   ETA 按「已跑时长 ÷ 已完成步数 × 剩下步数」估；
/// - 都没有就只说「已跑 N 分钟」—— **不编百分比**（和本模块开头那条规矩一样）。
pub fn video_line(ahead: Option<usize>, queued: bool, ws: Option<(u64, u64)>, log: &Progress, run_secs: u64) -> (usize, usize, String, Option<u64>) {
    let mins = |s: u64| (s as f64 / 60.0).round().max(1.0) as u64;
    if queued {
        let label = match ahead {
            Some(n) if n > 0 => format!("ComfyUI 排队中 · 前面还有 {n} 个"),
            _ => "ComfyUI 排队中".to_string(),
        };
        return (0, 0, label, None);
    }
    let (done, total, eta) = match (log.step, log.total, ws) {
        (Some(d), Some(t), _) if t > 0 => (d as u64, t as u64, log.eta_secs.map(u64::from)),
        (_, _, Some((v, m))) if m > 0 => (v.min(m), m, (v > 0 && v < m).then(|| run_secs * (m - v) / v)),
        _ => return (0, 0, format!("ComfyUI 正在跑 · 已跑 {} 分钟", mins(run_secs)), None),
    };
    let pct = (done * 100 / total).min(100);
    let mut parts = vec![if done >= total { format!("{total}/{total} 步跑完 · 正在解码存盘") } else { format!("第 {}/{total} 步 · {pct}%", done + 1) }];
    if let Some(e) = eta.filter(|_| done < total) {
        parts.push(format!("约还要 {} 分钟", mins(e)));
    }
    if run_secs > 0 {
        parts.push(format!("已跑 {} 分钟", mins(run_secs)));
    }
    (done as usize, total as usize, parts.join(" · "), eta)
}

/// ComfyUI 的日志在哪。`anima.sh` 把它写在这儿；用户自己起的实例没有这份日志。
pub fn log_path() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join("Library/Logs/Anima/comfyui.log")
}

/// 读日志末尾。读不到（没这个文件、权限不够）回空串 —— 调用方会退回「已跑 N 分钟」。
pub fn read_tail(p: &Path, bytes: u64) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(p) else { return String::new() };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let from = len.saturating_sub(bytes);
    if f.seek(SeekFrom::Start(from)).is_err() {
        return String::new();
    }
    let mut buf = Vec::new();
    let _ = f.take(bytes + 1).read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}

/// 当前这轮的进度。`started_at` 是任务开始的 unix 秒。
#[tauri::command]
pub fn comfy_progress(started_at: Option<u64>) -> Progress {
    let tail = read_tail(&log_path(), TAIL_BYTES);
    let elapsed = started_at
        .map(|t| (crate::jobs::now().saturating_sub(t)) as f32)
        .unwrap_or(0.0);
    let step = steps_since_last_prompt(&tail);
    // **距最后那行 `step` 过了多久，按文件的修改时间算。**
    //
    // 日志里那行没有时间戳，而 ComfyUI 每打一行都会写文件 ——
    // 所以「文件最后被写的时刻」就是最后那行出现的时刻。
    // 拿不到就当刚打出来（保守：宁可 ETA 偏长，也不要显示一个已经超时的数）。
    let since = std::fs::metadata(log_path())
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .map(|d| d.as_secs_f32())
        .unwrap_or(0.0);
    describe(step, since, elapsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 出片卡片上的进度() {
        let none = Progress { text: String::new(), step: None, total: None, eta_secs: None };
        assert_eq!(video_line(Some(2), true, None, &none, 0).2, "ComfyUI 排队中 · 前面还有 2 个");
        assert_eq!(video_line(Some(0), true, None, &none, 0).2, "ComfyUI 排队中");
        // 日志拿得到：步数和 ETA 用日志的
        let log = Progress { text: String::new(), step: Some(4), total: Some(6), eta_secs: Some(1500) };
        let (d, t, l, eta) = video_line(None, false, Some((5, 6)), &log, 75 * 60);
        assert_eq!((d, t, eta), (4, 6, Some(1500)));
        assert_eq!(l, "第 5/6 步 · 66% · 约还要 25 分钟 · 已跑 75 分钟");
        // 读不到日志：用 ws 的步数，ETA 按已跑时长估
        let (d, t, l, eta) = video_line(None, false, Some((2, 6)), &none, 40 * 60);
        assert_eq!((d, t, eta), (2, 6, Some(80 * 60)));
        assert!(l.starts_with("第 3/6 步 · 33% · 约还要 80 分钟"), "{l}");
        // 最后一步也跑完了
        assert_eq!(video_line(None, false, Some((6, 6)), &none, 90 * 60).2, "6/6 步跑完 · 正在解码存盘 · 已跑 90 分钟");
        // 什么都没有：不编百分比
        let (_, t, l, eta) = video_line(None, false, None, &none, 5 * 60);
        assert_eq!((t, eta), (0, None));
        assert_eq!(l, "ComfyUI 正在跑 · 已跑 5 分钟");
    }

    const REAL: &str = "\
[INFO] got prompt
  adaln cache: 8 timesteps, 77 MB in 6.8s
  dropped adaln projections, freeing 0.0 GB
  step 1/4  734.8s  eta 36.7 min
  step 2/4  706.8s  eta 24.0 min
";

    #[test]
    fn 认得出真实日志里那一行() {
        assert_eq!(
            parse_step("  step 2/4  706.8s  eta 24.0 min"),
            Some(Step { done: 2, total: 4, secs: 706.8 })
        );
        // 取最后一行，前面的已经被它取代
        assert_eq!(last_step(REAL), Some(Step { done: 2, total: 4, secs: 706.8 }));
    }

    /// **格式变了就回 None，绝不猜。**
    ///
    /// 那行是 WeeTodd 自己 print 的，它换版本就可能变。宽松匹配
    /// （「随便找两个数字」）在那时候会解析出垃圾，而垃圾会被当成真进度显示 ——
    /// 比没有进度糟得多。
    #[test]
    fn 认不出的一概不猜() {
        for bad in [
            "step 2 of 4 took 706.8s",      // 换了写法
            "  step 2/4",                    // 没有耗时
            "  step x/4  706.8s",            // 步数不是数字
            "  step 2/4  706.8 s",           // 单位和数字分开了
            "100%|██████████| 8/8 [01:01<00:00,  7.73s/it]",  // 这是出图的 tqdm，不是它
            "",
        ] {
            assert_eq!(parse_step(bad), None, "把「{bad}」猜成了进度");
        }
    }

    /// **新任务的头十几分钟不能显示上一个任务的步数。**
    ///
    /// 上一轮的 `step 4/4` 还留在日志里，不按 `got prompt` 切的话，
    /// 新任务一开始就显示「第 5/4 步」或者「4/4 跑完」—— 看着很正常，完全是错的。
    #[test]
    fn 只认这一轮任务的步数() {
        let two_rounds = "\
[INFO] got prompt
  step 1/4  700.0s  eta 35.0 min
  step 4/4  700.0s  eta 0.0 min
[INFO] Prompt executed in 2800.00 seconds
[INFO] got prompt
  adaln cache: 8 timesteps
";
        // 新任务还没打出第一行 step —— 就该是 None，而不是上一轮的 4/4
        assert_eq!(steps_since_last_prompt(two_rounds), None);
        // 上一轮的最后一行还是认得出来的（`last_step` 不切轮次）
        assert_eq!(last_step(two_rounds).map(|s| s.done), Some(4));
        // 新任务打出第一行之后就认它
        let mut s = two_rounds.to_string();
        s.push_str("  step 1/4  710.0s  eta 35.5 min\n");
        assert_eq!(steps_since_last_prompt(&s), Some(Step { done: 1, total: 4, secs: 710.0 }));
    }

    /// 在步之间插值：预估来自**上一步的实测耗时**，每步校准。
    #[test]
    fn 步之间按上一步的实测耗时插值() {
        let s = Step { done: 2, total: 4, secs: 706.8 };
        // 刚打出这一行：还剩这一步 + 最后一步 ≈ 1413 秒 ≈ 24 分钟
        let p = describe(Some(s), 0.0, 1450.0);
        assert_eq!(p.text, "第 3/4 步 · 约还要 24 分钟");
        assert_eq!((p.step, p.total), (Some(2), Some(4)));
        // 过了 9 分钟：这一步还剩约 3 分钟，加最后一步 ≈ 15 分钟
        let p = describe(Some(s), 540.0, 1990.0);
        assert_eq!(p.text, "第 3/4 步 · 约还要 15 分钟");
    }

    /// **插值到头就停住，不往前爬。** 超时本身是要被看见的信息 ——
    /// 爬到 99% 反而把它藏起来（DC 那条曲线就是这么做的）。
    #[test]
    fn 超出预估就说超出_不继续爬() {
        let s = Step { done: 2, total: 4, secs: 706.8 };
        let p = describe(Some(s), 706.8 + 240.0, 3000.0);
        assert_eq!(p.text, "第 3/4 步 · 已超出预估 4 分钟");
        // **超时了就不给 ETA** —— 给了就是编的
        assert_eq!(p.eta_secs, None);
    }

    #[test]
    fn 最后一步跑完是在解码存盘() {
        let p = describe(Some(Step { done: 4, total: 4, secs: 700.0 }), 30.0, 2900.0);
        assert_eq!(p.text, "4/4 步跑完 · 正在解码存盘");
    }

    /// **拿不到步数时不显示百分比。**
    /// 编一个数字出来，和「任务显示成功、结果是错的」是同一类病。
    #[test]
    fn 拿不到步数就只说已跑多久_不编百分比() {
        let p = describe(None, 0.0, 754.0);
        assert_eq!(p.text, "已跑 13 分钟");
        assert_eq!((p.step, p.total, p.eta_secs), (None, None, None));
        assert!(!p.text.contains('%'), "编了个百分比出来");
    }

    #[test]
    fn 日志读不到不报错() {
        assert_eq!(read_tail(Path::new("/没有这个/comfyui.log"), 1024), "");
        // 只读末尾：给的字节数小于文件长度时，不该把整份读进来
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("x.log");
        // 换行不能省 —— 少一个换行，那 5000 个 a 就和 step 在同一行上，
        // 解析（正确地）认不出来，而测试会看起来像 read_tail 坏了
        std::fs::write(&f, "a".repeat(5000) + "\n  step 1/2  10.0s  eta 0.2 min\n").unwrap();
        let tail = read_tail(&f, 200);
        assert!(tail.len() <= 201, "读多了：{}", tail.len());
        assert_eq!(last_step(&tail), Some(Step { done: 1, total: 2, secs: 10.0 }));
    }
}
