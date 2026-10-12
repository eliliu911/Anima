//! 超长镜：单镜超过 15 秒，后台等分成几段，一次交给 H3 的窗口链出完。
//!
//! # 用户要的
//!
//! 2026-10-05 EP03 第 20 镜：台词 51 字，14 秒放不下，提示「要么加长这一镜」——
//! 可时长上限是 15 秒，加长不了，红字一直在、也没有修复按钮。用户：
//! 「单镜时长超过 15 秒后，自动做倍数拆分提交给 H3，让 H3 自动拆分时间并组合，跟前面的一镜出片一样」
//! 「前台不需要展示出来」「要有个上限，比如单镜不能超过 45 秒」。
//!
//! # 和长镜头、一镜出的关系
//!
//! 出片用的是同一条窗口链（[`crate::agent_runtime::video_tool::render_long_take`]，
//! `WeeToddH3ChainedTimeline`）：窗口必须等长、窗口之间重叠 [`crate::long_take::CONTEXT_FRAMES`] 帧，
//! 一次提交跑完，H3 自己接成一整条。区别只在**拆分不落到镜头表上**：
//!
//! - 长镜头 / 一镜出：一镜一个窗口，镜头表里就是几镜；
//! - 超长镜：镜头表里还是一镜，合成提示词时临时拆成几个「窗口镜头」（[`windows`]），
//!   各合成一份，存回这一镜时用 [`WINDOW_MARK`] 隔开（[`join_prompts`]）；出片时再按它拆开（[`split_prompt`]）。
//!
//! 存成一份带分隔的正文而不是另开字段：界面上看、手改都是这一份，出片拆的也是这一份，不会出现
//! 「改了显示的那份、发出去的是另一份」。
//!
//! # 怎么拆
//!
//! 1. 段数：每段不超过 15 秒（[`plan`]）。20 秒 → 2 段 × 10.46 秒；45 秒 → 4 段 × 11.94 秒。
//! 2. 台词跟着它所在的那段画面走：画面描述里「谁开口 / 说 / 问」按顺序认出说话时刻，台词按顺序对上去（[`line_segments`]）。
//! 3. 在分段边界上切成几组，每组的台词要放得下（最后一段以外，每段最后 2 秒不许说话 —— 那里是和下一段的重叠区），
//!    放得下的切法里挑各组时长最均匀的。
//! 4. 每组的分段按比例伸缩到窗口时长，描写里的「约 5.1 秒」「第 8 秒」一起换算。
//!
//! 一段一个窗口镜头，各自合成：台词只写进它所在的那一段，画面只写那一段的，H3 每个窗口只看自己那份。

use crate::cinema::{Keyframes, StoryboardShot};
use crate::shot_table::{ShotSegment, MAX_SHOT_SECS};

/// 单镜最长多少秒（用户定的）。45 秒是 4 段，每段约 12 秒；一次跑完约是普通一镜的 4 倍时间。
pub const MAX_LONG_SHOT_SECS: f32 = 45.0;

/// 段与段之间重叠多少秒（和长镜头同一档）。
fn overlap_secs() -> f32 {
    crate::long_take::CONTEXT_FRAMES as f32 / 24.0
}

/// 这一镜是不是超长镜（要拆段出）。
pub fn is_long(duration: f32) -> bool {
    duration > MAX_SHOT_SECS + 0.01
}

/// 拆几段、每段多少秒。窗口等长：`n × 每段 − (n − 1) × 重叠 = 总时长`。不超过 15 秒的是 `(1, 时长)`。
pub fn plan(duration: f32) -> (usize, f32) {
    if !is_long(duration) {
        return (1, duration);
    }
    let ov = overlap_secs();
    let n = ((duration - ov) / (MAX_SHOT_SECS - ov)).ceil().max(2.0) as usize;
    let w = (duration + (n as f32 - 1.0) * ov) / n as f32;
    (n, (w * 100.0).round() / 100.0)
}

/// 合成好的各段正文之间的分隔行。出片按它拆开；**手改的时候别删**（[`split_prompt`] 数不对就不发）。
pub const WINDOW_MARK: &str = "=== H3 WINDOW";

/// 各段正文拼成存回镜头的那一份。
pub fn join_prompts(parts: &[String]) -> String {
    let n = parts.len();
    parts
        .iter()
        .enumerate()
        .map(|(i, p)| format!("{WINDOW_MARK} {}/{n} ===\n{}", i + 1, p.trim()))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// 存着的正文拆回各段。没有分隔行的（普通一镜）返回一段。
pub fn split_prompt(video: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut cur: Option<String> = None;
    for line in video.lines() {
        if line.trim_start().starts_with(WINDOW_MARK) {
            if let Some(c) = cur.take() {
                parts.push(c.trim().to_string());
            }
            cur = Some(String::new());
            continue;
        }
        let c = cur.get_or_insert_with(String::new);
        c.push_str(line);
        c.push('\n');
    }
    if let Some(c) = cur {
        parts.push(c.trim().to_string());
    }
    parts.retain(|p| !p.is_empty());
    parts
}

/// 换一个人说话要留的停顿、每段末尾不许说话的秒数、留给说话的比例 —— 和镜头表那层的容量检查同一套数。
const TURN_GAP: f32 = 0.35;
const SPEECH_SHARE: f32 = 0.8;
const CPS: f32 = 4.1;

/// 第 `k` 段（0 起，共 `n` 段、每段 `w` 秒）留给说话的秒数。最后一段以外要扣掉末尾不说话的那 2 秒。
pub fn window_speech_secs(k: usize, n: usize, w: f32) -> f32 {
    let tail = if k + 1 < n { crate::prompt_route_b::LONG_TAKE_TAIL_SECS } else { 0.0 };
    (w * SPEECH_SHARE - tail).max(0.0)
}

/// 一组台词说完要多少秒（含换人停顿）。
fn needs(lines: &[&str]) -> f32 {
    let said: Vec<&&str> = lines.iter().filter(|t| !t.trim().is_empty()).collect();
    let chars: usize = said.iter().map(|t| crate::h3_prompt::speakable_chars(t)).sum();
    chars as f32 / CPS + said.len().saturating_sub(1) as f32 * TURN_GAP
}

/// 按顺序把台词装进 `n` 段，每段装不下就换下一段。装得下返回 `true`。**只算容量，不看画面**，
/// 给镜头表那层「这一镜要多长」用；真正拆的时候按画面切（[`windows`]）。
pub fn lines_fit(lines: &[&str], duration: f32) -> bool {
    let (n, w) = plan(duration);
    let mut k = 0;
    let mut cur: Vec<&str> = Vec::new();
    for l in lines.iter().copied().filter(|t| !t.trim().is_empty()) {
        cur.push(l);
        if needs(&cur) <= window_speech_secs(k, n, w) + 1e-3 {
            continue;
        }
        // 这一段装不下了，从下一段重新装；一句话单独也装不下就再往后（最后一段没有末尾那 2 秒，可能更宽）
        cur = vec![l];
        loop {
            k += 1;
            if k >= n {
                return false;
            }
            if needs(&cur) <= window_speech_secs(k, n, w) + 1e-3 {
                break;
            }
        }
    }
    true
}

/// 画面描述里表示「开口说话」的词。
const SPEECH_CUES: &[&str] = &["开口", "说", "问", "答", "喊", "叫", "道", "回应", "回嘴", "接话", "念", "吼", "嘀咕", "低语", "宣布", "插嘴"];

/// 每句台词落在哪一段画面里（分段下标）。
///
/// 画面描述是这么写的：「约5.1秒星澜平稳开口……」「约8.5秒他直视星澜追问」。按句（。！？）切开 ——
/// 不按分号切：「陆川剥橘子……；约0.5秒带着笑意开口，随后看星澜」分号后面那半句没有主语，切开就认成星澜了。
/// 带「开口 / 说 / 问」的那一句里**最先出现的人名**是说话的人 —— 「陆川……开口……随后抬眼看星澜」说话的是陆川。
/// 台词按顺序对到同一个人的下一次开口上；对不上的跟上一句同一段（第一句对不上就是第 0 段）。顺序不会倒退。
pub fn line_segments(shot: &StoryboardShot) -> Vec<usize> {
    let names: Vec<String> = shot.dialogue_lines.iter().map(|l| base(&l.speaker).to_string()).filter(|n| !n.is_empty()).collect();
    let mut events: Vec<(usize, String)> = Vec::new();
    for (i, seg) in shot.segments.iter().enumerate() {
        for sentence in seg.visual.split(['。', '！', '？', '\n']) {
            if !SPEECH_CUES.iter().any(|c| sentence.contains(c)) {
                continue;
            }
            let first = names.iter().filter_map(|n| sentence.find(n.as_str()).map(|at| (at, n))).min_by_key(|(at, _)| *at);
            if let Some((_, n)) = first {
                events.push((i, n.clone()));
            }
        }
    }
    let mut out = Vec::new();
    let mut next_event = 0;
    let mut last = 0;
    for l in shot.dialogue_lines.iter().filter(|l| !l.line.trim().is_empty()) {
        let who = base(&l.speaker);
        if let Some(j) = (next_event..events.len()).find(|&j| events[j].1 == who) {
            last = last.max(events[j].0);
            next_event = j + 1;
        }
        out.push(last);
    }
    out
}

fn base(name: &str) -> &str {
    let n = name.trim();
    for open in ['（', '('] {
        if let Some(i) = n.find(open) {
            if n.ends_with('）') || n.ends_with(')') {
                return n[..i].trim_end();
            }
        }
    }
    n
}

/// 段太少（一段 20 秒）就从最长的那段对半分，直到至少 `n` 段 —— 每段都得有画面可写。
fn enough_segments(segments: &[ShotSegment], n: usize) -> Vec<ShotSegment> {
    let mut segs: Vec<ShotSegment> = segments.to_vec();
    while segs.len() < n {
        let Some((i, _)) = segs.iter().enumerate().max_by(|a, b| (a.1.end - a.1.start).total_cmp(&(b.1.end - b.1.start))) else { break };
        let s = segs[i].clone();
        let mid = (s.start + s.end) / 2.0;
        let mut a = s.clone();
        a.end = mid;
        let mut b = s;
        b.start = mid;
        b.title = format!("{}（续）", b.title.trim());
        segs.splice(i..=i, [a, b]);
    }
    segs
}

/// 拆成几个「窗口镜头」：每个 `w` 秒、只带自己那几段画面和那几句台词，合成提示词时一个一个按长镜头窗口合成。
///
/// 不是超长镜的原样返回一个。拆不进去（台词怎么切都放不下）也照样拆 —— 按超出最少的切法，
/// 合成那一步的逐句核对会报出具体哪一句放不下。
pub fn windows(shot: &StoryboardShot) -> Vec<StoryboardShot> {
    let (n, w) = plan(shot.duration);
    if n <= 1 {
        return vec![shot.clone()];
    }
    let segs = enough_segments(&shot.segments, n);
    // 分段被对半分过的话，台词要按新的分段重新认
    let mut probe = shot.clone();
    probe.segments = segs.clone();
    let line_seg = line_segments(&probe);
    let lines: Vec<&crate::cinema::DialogueLine> = shot.dialogue_lines.iter().filter(|l| !l.line.trim().is_empty()).collect();

    // 在 segs.len() - 1 个分段边界里挑 n - 1 个切点
    let cuts = best_cuts(&segs, &line_seg, &lines, n, w, shot.duration);
    let bounds: Vec<usize> = std::iter::once(0).chain(cuts).chain(std::iter::once(segs.len())).collect();
    (0..n)
        .map(|k| {
            let group = &segs[bounds[k]..bounds[k + 1]];
            let (from, to) = (group[0].start, group[group.len() - 1].end);
            let scale = w / (to - from).max(0.1);
            let mut v = shot.clone();
            v.duration = w;
            v.segments = group
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let mut s = s.clone();
                    s.start = if i == 0 { 0.0 } else { round2((s.start - from) * scale) };
                    s.end = if i + 1 == group.len() { w } else { round2((s.end - from) * scale) };
                    for t in [&mut s.visual, &mut s.blocking, &mut s.camera, &mut s.sound, &mut s.constraints, &mut s.forbid] {
                        *t = retime_mentions(t, from, scale, w);
                    }
                    s
                })
                .collect();
            v.dialogue_lines = shot
                .dialogue_lines
                .iter()
                .filter(|l| !l.line.trim().is_empty())
                .zip(line_seg.iter())
                .filter(|(_, &seg)| seg >= bounds[k] && seg < bounds[k + 1])
                .map(|(l, _)| l.clone())
                .collect();
            v.dialogue = v.dialogue_lines.iter().map(|l| format!("{}：{}", l.speaker.trim(), l.line.trim())).collect::<Vec<_>>().join("\n");
            // 窗口链不吃首帧、不接参考视频（衔接靠窗口之间的重叠帧）
            v.keyframes = Keyframes::NONE;
            v.continuity = crate::cinema::Continuity::None;
            v.continues_previous = false;
            v
        })
        .collect()
}

fn round2(x: f32) -> f32 {
    (x * 100.0).round() / 100.0
}

/// 切点：放得下的切法里，各组时长离平均最近的；都放不下就挑超出最少的。
fn best_cuts(segs: &[ShotSegment], line_seg: &[usize], lines: &[&crate::cinema::DialogueLine], n: usize, w: f32, total: f32) -> Vec<usize> {
    let m = segs.len();
    let mut best: Option<(f32, Vec<usize>)> = None;
    let mut cuts: Vec<usize> = (1..n).collect();
    loop {
        let bounds: Vec<usize> = std::iter::once(0).chain(cuts.iter().copied()).chain(std::iter::once(m)).collect();
        let mut overflow = 0.0f32;
        let mut spread = 0.0f32;
        for k in 0..n {
            let group: Vec<&str> = lines.iter().zip(line_seg).filter(|(_, &s)| s >= bounds[k] && s < bounds[k + 1]).map(|(l, _)| l.line.as_str()).collect();
            overflow += (needs(&group) - window_speech_secs(k, n, w)).max(0.0);
            let dur = segs[bounds[k + 1] - 1].end - segs[bounds[k]].start;
            spread += (dur - total / n as f32).abs();
        }
        let score = overflow * 1000.0 + spread;
        if best.as_ref().map_or(true, |(b, _)| score < *b) {
            best = Some((score, cuts.clone()));
        }
        // 下一个组合（1 ≤ c1 < c2 < … < m）
        let mut i = n - 1;
        loop {
            if i == 0 {
                return best.map(|(_, c)| c).unwrap_or_default();
            }
            i -= 1;
            if cuts[i] < m - (n - 1 - i) {
                cuts[i] += 1;
                for j in i + 1..n - 1 {
                    cuts[j] = cuts[j - 1] + 1;
                }
                break;
            }
        }
    }
}

/// 描写里写死的「约 5.1 秒」「第 8 秒」换算到窗口里的时刻：减去这一段的起点再按比例缩放，落在 0 ~ 窗口时长之间。
fn retime_mentions(text: &str, from: f32, scale: f32, w: f32) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = ["约", "第"].iter().filter_map(|p| rest.find(p)).min() {
        let prefix = if rest[at..].starts_with('约') { "约" } else { "第" };
        out.push_str(&rest[..at + prefix.len()]);
        let after = &rest[at + prefix.len()..];
        let sp = after.len() - after.trim_start().len();
        let num = &after[sp..];
        let num_len = num.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(num.len());
        let tail = &num[num_len..];
        match num[..num_len].parse::<f32>() {
            Ok(v) if num_len > 0 && tail.trim_start().starts_with('秒') => {
                let local = ((v - from) * scale).clamp(0.0, w);
                let local = (local * 10.0).round() / 10.0;
                out.push_str(&after[..sp]);
                out.push_str(&if local.fract() == 0.0 { format!("{local:.0}") } else { format!("{local:.1}") });
                rest = tail;
            }
            _ => rest = after,
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cinema::DialogueLine;

    fn seg(start: f32, end: f32, visual: &str) -> ShotSegment {
        ShotSegment { start, end, title: format!("{start}-{end}"), visual: visual.into(), ..Default::default() }
    }

    /// EP03 第 20 镜（2026-10-05）的原样数据，加长到 20 秒（按比例伸缩过的分段）。
    fn ep03_20(duration: f32) -> StoryboardShot {
        let mut s: StoryboardShot = serde_json::from_value(serde_json::json!({
            "id": "s20", "shot_num": 20, "scale": "medium_close", "camera_movement": "push_in", "dialogue": "",
            "prompt": "", "duration": duration, "status": "pending"
        }))
        .unwrap();
        let k = duration / 14.0;
        s.segments = vec![
            seg(0.0, 5.0 * k, "陆川低头继续剥橘子，拇指沿裂口逐片揭开果皮，皮屑垂在掌边；约0.5秒带着笑意开口，提到被引用的话时压低声音，随后抬眼看星澜。星澜侧身面对他，静静接住他的目光。"),
            seg(5.0 * k, 8.0 * k, "约5.1秒星澜平稳开口，粉色眼睛始终落在陆川眼睛上，语气明确；陆川听见检索结论时，拇指停在果皮边缘半拍。"),
            seg(8.0 * k, 11.0 * k, "陆川双手向两侧施力，橘子沿果肉中线分成完整两半；约8.5秒他直视星澜追问，笑意已经淡下。星澜听完眨一次粉色眼睛。"),
            seg(11.0 * k, duration, "约11.1秒星澜先短暂看向陆川胸前的橘子与呼吸起伏，再抬眼回到他脸上开口；前半句如实承认未知。陆川没有接话。"),
        ];
        s.dialogue_lines = vec![
            DialogueLine { speaker: "陆川".into(), line: "刚才那句「主人不收二手的东西」，也是出厂话术？".into(), ..Default::default() },
            DialogueLine { speaker: "星澜".into(), line: "数据库里没有这句。".into(), ..Default::default() },
            DialogueLine { speaker: "陆川".into(), line: "那是哪儿来的？".into(), ..Default::default() },
            DialogueLine { speaker: "星澜".into(), line: "本机也不知道。本机只知道，她让主人不开心。".into(), ..Default::default() },
        ];
        s
    }

    #[test]
    fn 段数和每段时长_等长窗口拼回总时长() {
        assert_eq!(plan(14.0), (1, 14.0));
        assert_eq!(plan(15.0), (1, 15.0));
        let ov = overlap_secs();
        for d in [15.5f32, 20.0, 29.0, 30.0, 44.0, 45.0] {
            let (n, w) = plan(d);
            assert!(n >= 2 && w <= MAX_SHOT_SECS + 1e-3, "{d}: {n} × {w}");
            assert!((n as f32 * w - (n as f32 - 1.0) * ov - d).abs() < 0.02, "{d}: {n} × {w} 拼不回去");
            // 少一段就装不下（段数是最少的）
            assert!((d + (n as f32 - 2.0) * ov) / (n as f32 - 1.0) > MAX_SHOT_SECS, "{d} 段数多了");
        }
        assert_eq!(plan(20.0).0, 2);
        assert_eq!(plan(45.0).0, 4);
    }

    #[test]
    fn 正文拼起来再拆开_一段不差() {
        let parts = vec!["first window\nline two".to_string(), "second".to_string(), "third".to_string()];
        let joined = join_prompts(&parts);
        assert!(joined.contains("=== H3 WINDOW 2/3 ==="));
        assert_eq!(split_prompt(&joined), parts);
        assert_eq!(split_prompt("普通一镜的正文\n两行"), vec!["普通一镜的正文\n两行".to_string()]);
    }

    #[test]
    fn 台词对到开口的那段画面() {
        let s = ep03_20(14.0);
        // 第一段的「开口」那句里先出现的是陆川（星澜在后面）；第二句星澜在第二段开口
        assert_eq!(line_segments(&s), vec![0, 1, 2, 3]);
    }

    /// 第 20 镜加长到 22 秒：拆两段，台词跟着画面走，第一段最后 2 秒不说话也放得下。
    /// （20 秒不够：第一段末尾 2 秒不许说话，后三句挤在第二段里要 8.5 秒）
    #[test]
    fn 第20镜加长到22秒_拆两段_台词跟着画面() {
        let s = ep03_20(22.0);
        let ws = windows(&s);
        assert_eq!(ws.len(), 2);
        let (n, w) = plan(22.0);
        for (k, v) in ws.iter().enumerate() {
            assert_eq!(v.duration, w);
            assert_eq!(v.segments.first().unwrap().start, 0.0);
            assert_eq!(v.segments.last().unwrap().end, w);
            assert!(v.segments.windows(2).all(|p| (p[0].end - p[1].start).abs() < 1e-3), "{:?}", v.segments);
            assert_eq!(v.keyframes, Keyframes::NONE);
            let said: Vec<&str> = v.dialogue_lines.iter().map(|l| l.line.as_str()).collect();
            assert!(needs(&said) <= window_speech_secs(k, n, w) + 1e-3, "第 {k} 段放不下：{said:?}");
        }
        let all: Vec<String> = ws.iter().flat_map(|v| v.dialogue_lines.iter().map(|l| l.line.clone())).collect();
        assert_eq!(all, s.dialogue_lines.iter().map(|l| l.line.clone()).collect::<Vec<_>>(), "台词一句不少、顺序不变");
        // 每句台词所在的那段画面也在同一个窗口里
        for v in &ws {
            for l in &v.dialogue_lines {
                let at = s.dialogue_lines.iter().position(|x| x.line == l.line).unwrap();
                let title = &s.segments[line_segments(&s)[at]].title;
                assert!(v.segments.iter().any(|g| &g.title == title), "「{}」和它的画面被拆开了", l.line);
            }
        }
        // 第二段里写死的「约11.1秒」换算到了窗口里
        assert!(!ws[1].segments.iter().any(|g| g.visual.contains("约11.1秒")), "{:?}", ws[1].segments);
    }

    #[test]
    fn 一段画面太长_对半分出够用的段() {
        let mut s = ep03_20(20.0);
        s.segments = vec![seg(0.0, 20.0, "陆川开口追问，星澜开口回答")];
        let ws = windows(&s);
        assert_eq!(ws.len(), 2);
        assert!(ws.iter().all(|v| !v.segments.is_empty() && v.segments[0].visual.contains("陆川")));
    }

    #[test]
    fn 时刻换算() {
        assert_eq!(retime_mentions("约11.1秒星澜开口", 10.0, 0.5, 10.0), "约0.6秒星澜开口");
        assert_eq!(retime_mentions("第8秒停下，约 3 秒", 4.0, 1.0, 10.0), "第4秒停下，约 0 秒");
        assert_eq!(retime_mentions("大约一会儿，第一次", 0.0, 1.0, 10.0), "大约一会儿，第一次");
    }

    #[test]
    fn 容量_超长镜按段装台词() {
        let l: Vec<&str> = vec!["刚才那句「主人不收二手的东西」，也是出厂话术？", "数据库里没有这句。", "那是哪儿来的？", "本机也不知道。本机只知道，她让主人不开心。"];
        assert!(!lines_fit(&l, 14.0));
        assert!(!lines_fit(&l, 20.0));
        assert!(lines_fit(&l, 22.0));
        // 一句话比哪一段都长：怎么拆都装不下
        let long = "我".repeat(60);
        assert!(!lines_fit(&[long.as_str()], 45.0));
    }
}
