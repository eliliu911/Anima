//! 画布 / 创作台的**超长视频**：H3 时长拉过 15 秒，还是一段提示词、点的还是「生成」，后台自己分段出一整条。
//!
//! # 用户要的
//!
//! 2026-10-11（看了 LibTV 的「超长视频」）：「生成按钮就是生成按钮，不能因为省事儿，或者实现问题，再给我搞一个
//! 『整段出片』的按钮出来」；「我们自己提示词规定了时间范围，就用提示词的。如果提示词里没有，就让模型自己去处理」。
//!
//! # 为什么要我们拆，不能交给 H3
//!
//! H3 一次生成最长 15 秒（`WeeToddH3GenerationConfig.duration_seconds` 2.5~15）。更长的只能走窗口链
//! （`WeeToddH3ChainedTimeline`，每窗 ≤15 秒、2~16 窗、总长 ≤120 秒），而**每个窗口要自己的一段提示词**
//! （一个窗口一个 `WeeToddH3ReferenceEncode`）—— 节点只管时间对齐和裁掉重叠，不会替你拆提示词（10-11 查过 `/object_info`）。
//! 工作台的超长镜也是 Anima 拆的（[`crate::long_shot`]），只是界面不展示。
//!
//! # 怎么拆
//!
//! - 段数、每段多长：和工作台超长镜同一个公式（[`crate::long_shot::plan`]）：每段 ≤15 秒、等长、相邻重叠 22 帧。
//! - **提示词里写了时间**（H3 官方格式的 `[Shot N] At 00:SS.sss`，或者一行一段的「0-3秒：…」）：代码按时间拆（[`split_timed`]）——
//!   不带时间的部分（主体定义、画风、数量锁、声音）每段都抄一份；带时间的按全片时间分进各段、换算成段内时间；
//!   一个镜头跨在两段之间的，后一段开头写明「接着同一个镜头，不切」。
//! - **没写时间**：交给后台模型按段写（[`PromptSlot::LongVideoSplit`](crate::prompts::PromptSlot::LongVideoSplit)）。
//! - 台词开口落在某一段最后 2 秒（非最后一段）的，**生成前拦下**：那里是和下一段的交接区，跑完才发现说不完要整条重来。
//!
//! 只做参考出片（Ref2VA）：窗口链只吃参考图，首帧 / 首尾帧出片超过 15 秒直接报错说明。

use std::path::{Path, PathBuf};

use crate::studio::{GenerateArgs, GenerateOutput};

/// 画布 / 创作台超长视频的上限（用户定的 75 秒 = 6 段 × 约 13.3 秒）。节点自己允许到 120 秒。
pub const MAX_LONG_VIDEO_SECS: f64 = 75.0;

/// 段末不许开口的秒数（和工作台超长镜同一条，见 [`crate::long_shot`]）
const NO_SPEECH_TAIL_SECS: f64 = 2.0;

/// 这次生成要不要走超长视频
pub fn wants(args: &GenerateArgs) -> bool {
    args.engine == "h3" && args.duration.is_some_and(|d| crate::long_shot::is_long(d as f32))
}

/// 拆成几段、每段多少秒、每段从全片第几秒开始
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub windows: usize,
    pub window_secs: f64,
    pub overlap_secs: f64,
    pub total_secs: f64,
}

impl Plan {
    pub fn new(total: f64) -> Self {
        let (n, w) = crate::long_shot::plan(total as f32);
        Plan { windows: n, window_secs: (w as f64 * 100.0).round() / 100.0, overlap_secs: crate::long_take::CONTEXT_FRAMES as f64 / 24.0, total_secs: total }
    }
    /// 第 k 段在全片里的起点
    pub fn start(&self, k: usize) -> f64 {
        k as f64 * (self.window_secs - self.overlap_secs)
    }
    pub fn end(&self, k: usize) -> f64 {
        self.start(k) + self.window_secs
    }
}

// ---------------------------------------------------------------------------
// 按时间码拆
// ---------------------------------------------------------------------------

/// H3 官方格式里各段的段名（行首 `name:`）
const SECTIONS: [&str; 7] = [
    "subject_definitions",
    "summary",
    "retention_analysis",
    "detailed_description",
    "integrated_multimodal_description",
    "overall_soundscape",
    "non_diegetic_music",
];

/// 一个带时间的单位：一个 `[Shot N]` 镜头，或者「0-3秒：…」这样一行
#[derive(Debug, Clone)]
struct Unit {
    start: f64,
    /// 句子 + 每句的全片时刻（句里有时间标记就用它，没有就沿用上一句的）
    sentences: Vec<(f64, String)>,
    /// `[Shot N] At 00:SS.sss, ` 之后的正文开头（镜头式）；行式为 None
    shot_style: bool,
    /// 行式：原来那行的结束时刻
    line_end: Option<f64>,
}

fn re(p: &str) -> regex::Regex {
    regex::Regex::new(p).unwrap()
}

/// `00:12.500` / `1:05.0` → 秒
fn mmss(m: &str, s: &str) -> f64 {
    m.parse::<f64>().unwrap_or(0.0) * 60.0 + s.parse::<f64>().unwrap_or(0.0)
}

fn fmt_mmss(t: f64) -> String {
    let t = t.max(0.0);
    let m = (t / 60.0).floor();
    format!("{:02}:{:06.3}", m as u64, t - m * 60.0)
}

fn fmt_secs(t: f64) -> String {
    let v = (t.max(0.0) * 10.0).round() / 10.0;
    format!("{v:.1}")
}

/// 句里的时间标记（全片时间）：`At 00:05.5` / `At the 5.0-second mark` / `Around the 5.0-second mark`
fn sentence_time(s: &str) -> Option<f64> {
    let a = re(r"\bAt (\d{1,2}):(\d{2}(?:\.\d+)?)");
    if let Some(c) = a.captures(s) {
        return Some(mmss(&c[1], &c[2]));
    }
    let b = re(r"\b(?:[Aa]t|[Aa]round) the (\d+(?:\.\d+)?)-second mark");
    b.captures(s).map(|c| c[1].parse().unwrap_or(0.0))
}

/// 句里的时间标记换成段内时间
fn shift_times(s: &str, offset: f64) -> String {
    let a = re(r"\bAt (\d{1,2}):(\d{2}(?:\.\d+)?)");
    let s = a.replace_all(s, |c: &regex::Captures| format!("At {}", fmt_mmss(mmss(&c[1], &c[2]) - offset)));
    let b = re(r"\b([Aa]t|[Aa]round) the (\d+(?:\.\d+)?)-second mark");
    b.replace_all(&s, |c: &regex::Captures| format!("{} the {}-second mark", &c[1], fmt_secs(c[2].parse::<f64>().unwrap_or(0.0) - offset)))
        .into_owned()
}

/// 按句切（英文句号 / 中文句号后断开），保留标点
fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut in_d = false;
    for (i, &c) in chars.iter().enumerate() {
        cur.push(c);
        if cur.ends_with("<d>") {
            in_d = true;
        }
        if cur.ends_with("</d>") {
            in_d = false;
        }
        let end = matches!(c, '。' | '！' | '？') || (matches!(c, '.' | '!' | '?') && chars.get(i + 1).map_or(true, |n| n.is_whitespace()));
        if end && !in_d {
            let t = cur.trim().to_string();
            if !t.is_empty() {
                out.push(t);
            }
            cur.clear();
        }
    }
    let t = cur.trim().to_string();
    if !t.is_empty() {
        out.push(t);
    }
    out
}

fn timed_sentences(start: f64, text: &str) -> Vec<(f64, String)> {
    let mut t = start;
    sentences(text)
        .into_iter()
        .map(|s| {
            if let Some(x) = sentence_time(&s) {
                t = x;
            }
            (t, s)
        })
        .collect()
}

/// 拆出 (不带时间的头部, 带时间的单位, 尾部段落)。认不出任何时间返回 None。
fn parse_timed(prompt: &str) -> Option<(String, Vec<Unit>, String)> {
    let section = re(&format!(r"(?m)^({}):", SECTIONS.join("|")));
    let marks: Vec<(usize, String)> = section.captures_iter(prompt).map(|c| (c.get(0).unwrap().start(), c[1].to_string())).collect();
    let body_name = marks.iter().find(|(_, n)| n == "detailed_description" || n == "integrated_multimodal_description").map(|(_, n)| n.clone());
    let (head, body, tail) = match &body_name {
        Some(name) => {
            let i = marks.iter().position(|(_, n)| n == name).unwrap();
            let b0 = marks[i].0;
            let b1 = marks.get(i + 1).map_or(prompt.len(), |m| m.0);
            let label = format!("{name}:");
            (prompt[..b0].to_string() + &label, prompt[b0 + label.len()..b1].to_string(), prompt[b1..].to_string())
        }
        None => (String::new(), prompt.to_string(), String::new()),
    };

    // ① 镜头式：[Shot N] …
    let shot = re(r"\[Shot (\d+)\]");
    let starts: Vec<usize> = shot.find_iter(&body).map(|m| m.start()).collect();
    if !starts.is_empty() {
        let pre = body[..starts[0]].to_string();
        let mut units = Vec::new();
        for (i, &s) in starts.iter().enumerate() {
            let e = starts.get(i + 1).copied().unwrap_or(body.len());
            let chunk = body[s..e].trim();
            let rest = shot.replace(chunk, "").trim().to_string();
            // 第一镜没有时间码 = 0 秒；其余取开头那个 `At 00:SS`
            let head_at = re(r"^At (\d{1,2}):(\d{2}(?:\.\d+)?),?\s*");
            let (t0, text) = match head_at.captures(&rest) {
                Some(c) => (mmss(&c[1], &c[2]), rest[c.get(0).unwrap().end()..].to_string()),
                None if i == 0 => (0.0, rest),
                None => return None, // 中间某一镜没写时间码：当作没写时间，交给模型
            };
            units.push(Unit { start: t0, sentences: timed_sentences(t0, &text), shot_style: true, line_end: None });
        }
        if units.len() < 2 && units[0].sentences.iter().all(|(t, _)| *t == 0.0) {
            return None; // 只有一镜、镜内也没时间：等于没写时间
        }
        return Some((head + &pre, units, tail));
    }

    // ② 行式：0-3秒：… / 0~3s … / 00:00-00:03 …
    let line = re(r"^\s*(?:(\d{1,2}):)?(\d+(?:\.\d+)?)\s*(?:秒|s)?\s*[-~–—至到]\s*(?:(\d{1,2}):)?(\d+(?:\.\d+)?)\s*(?:秒|s|S)?\s*[:：，,、]?\s*(.*)$");
    let mut pre = String::new();
    let mut units = Vec::new();
    for l in body.lines() {
        match line.captures(l) {
            Some(c) => {
                let a = mmss(c.get(1).map_or("0", |m| m.as_str()), &c[2]);
                let b = mmss(c.get(3).map_or("0", |m| m.as_str()), &c[4]);
                units.push(Unit { start: a, sentences: vec![(a, c[5].trim().to_string())], shot_style: false, line_end: Some(b) });
            }
            None if units.is_empty() => {
                pre.push_str(l);
                pre.push('\n');
            }
            None => {
                // 时间行下面的续行，归到上一行
                if let Some(u) = units.last_mut() {
                    let t = u.start;
                    if !l.trim().is_empty() {
                        u.sentences.push((t, l.trim().to_string()));
                    }
                }
            }
        }
    }
    if units.len() < 2 {
        return None;
    }
    Some((head + &pre, units, tail))
}

/// 台词开口落在非最后一段的最后 2 秒里吗（那里是和下一段的交接区）
fn speech_in_tail(plan: &Plan, units: &[Unit]) -> Option<String> {
    for u in units {
        for (t, s) in &u.sentences {
            let speaks = s.contains("<d>") || s.contains("says") || s.contains('「') || s.contains('“');
            if !speaks {
                continue;
            }
            for k in 0..plan.windows - 1 {
                let (a, b) = (plan.end(k) - NO_SPEECH_TAIL_SECS, plan.end(k));
                if *t >= a && *t < b {
                    let quote: String = s.chars().take(60).collect();
                    return Some(format!(
                        "第 {:.1} 秒有一句台词（「{quote}…」），正好落在第 {} 段的最后 2 秒（{:.1}~{:.1} 秒）—— 那里是和下一段的交接区，\
                         这句会被切断。把它挪到 {:.1} 秒之前或 {:.1} 秒之后，或者把总时长改一下（现在 {:.0} 秒分 {} 段，每段 {:.2} 秒）",
                        t, k + 1, a, b, a, b, plan.total_secs, plan.windows, plan.window_secs
                    ));
                }
            }
        }
    }
    None
}

/// 保留说明（retention_analysis）按这一段里实际出现的镜头重写镜号。整段没有就原样
fn rewrite_retention(head: &str, shots: &[String]) -> String {
    let line = re(r"(?m)^(<Subject (\d+)>) \(appears in [^)]*\)");
    line.replace_all(head, |c: &regex::Captures| {
        let tag = format!("<Subject {}>", &c[2]);
        let used: Vec<String> = shots.iter().enumerate().filter(|(_, t)| t.contains(&tag)).map(|(i, _)| format!("[Shot {}]", i + 1)).collect();
        if used.is_empty() {
            format!("{} (appears in none of the shots in this part)", &c[1])
        } else {
            format!("{} (appears in {})", &c[1], used.join(", "))
        }
    })
    .into_owned()
}

fn part_label(k: usize, plan: &Plan) -> String {
    format!(
        "Part {} of {} of one continuous {:.0}-second video{}.",
        k + 1,
        plan.windows,
        plan.total_secs,
        if k == 0 { "" } else { ", picking up exactly where the previous part ends with no cut" }
    )
}

fn part_label_zh(k: usize, plan: &Plan) -> String {
    format!(
        "（这是一条 {:.0} 秒连续视频的第 {}/{} 段{}）",
        plan.total_secs,
        k + 1,
        plan.windows,
        if k == 0 { "" } else { "，紧接上一段，不切" }
    )
}

/// **按提示词里的时间拆。** 没写时间回 `None`（交给模型）；写了时间但拆不了（台词卡在交接区）回 `Some(Err)`。
pub fn split_timed(prompt: &str, plan: &Plan) -> Option<Result<Vec<String>, String>> {
    let (head, units, tail) = parse_timed(prompt)?;
    if let Some(e) = speech_in_tail(plan, &units) {
        return Some(Err(e));
    }
    let last = units.len() - 1;
    let unit_end = |i: usize| -> f64 { units[i].line_end.unwrap_or_else(|| if i < last { units[i + 1].start } else { plan.total_secs }) };
    let mut out = Vec::new();
    for k in 0..plan.windows {
        let (s, e) = (plan.start(k), plan.end(k));
        let mut shots: Vec<String> = Vec::new();
        for (i, u) in units.iter().enumerate() {
            let ue = unit_end(i).max(u.start + 0.01);
            if ue <= s || u.start >= e {
                continue;
            }
            // 这一段里看得到的句子：开始时刻之前那句（正在进行）+ 段内的
            let active = u.sentences.iter().rposition(|(t, _)| *t <= s).unwrap_or(0);
            let picked: Vec<String> = u
                .sentences
                .iter()
                .enumerate()
                .filter(|(j, (t, _))| (*j >= active || u.start >= s) && *t < e)
                .map(|(_, (_, x))| shift_times(x, s))
                .collect();
            if picked.is_empty() {
                continue;
            }
            let carried = u.start < s && k > 0;
            let n = shots.len() + 1;
            let text = picked.join(" ");
            let entry = if u.shot_style {
                if n == 1 {
                    if carried {
                        // 原文这一镜开头是「the camera cuts to …」—— 接着上一段时不切，去掉这半句，否则一句话里又说不切又说切
                        let cont = re(r"^(?i:the camera )?(?i:cuts to|cut to)\s+").replace(&text, "").into_owned();
                        format!("[Shot 1] Continuing the same shot from the previous moment with no cut: {cont}")
                    } else {
                        format!("[Shot 1] {text}")
                    }
                } else {
                    format!("[Shot {n}] At {}, {text}", fmt_mmss(u.start - s))
                }
            } else {
                let (a, b) = ((u.start - s).max(0.0), (ue.min(e) - s).max(0.0));
                let lead = if carried { "（接着上一段的同一个画面，不切）" } else { "" };
                format!("{}-{}秒：{lead}{text}", fmt_secs(a), fmt_secs(b))
            };
            shots.push(entry);
        }
        // 第几段写在 summary 的 [reference generation] 后面；没有 summary 的另起一行（中文行式用中文）
        let head_k = rewrite_retention(&head, &shots);
        let tagged = head_k.contains("[reference generation]");
        let head_k = head_k.replacen("[reference generation]", &format!("[reference generation] {}", part_label(k, plan)), 1);
        let mut t = String::new();
        if !head_k.trim().is_empty() {
            t.push_str(head_k.trim_end());
            t.push('\n');
        }
        if !tagged {
            t.push_str(&if units[0].shot_style { part_label(k, plan) } else { part_label_zh(k, plan) });
            t.push('\n');
        }
        t.push_str(&shots.join("\n"));
        if !tail.trim().is_empty() {
            t.push_str("\n\n");
            t.push_str(tail.trim());
        }
        out.push(t.trim().to_string());
    }
    Some(Ok(out))
}

// ---------------------------------------------------------------------------
// 没写时间：交给后台模型
// ---------------------------------------------------------------------------

/// 模型拆的结果：`{"windows": ["…", "…"]}`，段数要对
pub fn parse_model_split(raw: &str, n: usize) -> Result<Vec<String>, String> {
    let text = crate::script_library::strip_code_fence(raw);
    let start = text.find('{').ok_or("没有 JSON")?;
    let end = text.rfind('}').ok_or("JSON 没闭合")?;
    let v: serde_json::Value = serde_json::from_str(&text[start..=end]).map_err(|e| format!("JSON 读不出：{e}"))?;
    let w: Vec<String> = v["windows"]
        .as_array()
        .ok_or("没有 windows 数组")?
        .iter()
        .map(|x| x.as_str().unwrap_or_default().trim().to_string())
        .collect();
    if w.len() != n {
        return Err(format!("要 {n} 段，模型写了 {} 段", w.len()));
    }
    if let Some(i) = w.iter().position(|x| x.is_empty()) {
        return Err(format!("第 {} 段是空的", i + 1));
    }
    Ok(w)
}

async fn split_with_model(config_dir: &Path, gateway: &crate::Gateway, prompt: &str, plan: &Plan) -> Result<Vec<String>, String> {
    let cfg = crate::prompts::load(config_dir);
    let vars = [
        ("PROMPT", prompt.to_string()),
        ("SECS", format!("{:.0}", plan.total_secs)),
        ("N", plan.windows.to_string()),
        ("WINDOW_SECS", format!("{:.2}", plan.window_secs)),
        ("OVERLAP", format!("{:.2}", plan.overlap_secs)),
    ];
    let refs: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let mut last = String::new();
    // 段数不对 / JSON 坏：再来一次，两次都不行就报出来（不悄悄退成每段同一份）
    for _ in 0..2 {
        let raw = crate::prompts::run(gateway, &cfg, crate::prompts::PromptSlot::LongVideoSplit, &refs).await.map_err(|e| e.to_string())?;
        match parse_model_split(&raw, plan.windows) {
            Ok(w) => return Ok(w),
            Err(e) => last = e,
        }
    }
    Err(format!("后台模型没拆好这段提示词（{last}）—— 在提示词里按时间写镜头（比如「0-3秒：…」或 H3 的 [Shot N] At 00:05.000）就不用模型拆"))
}

// ---------------------------------------------------------------------------
// 出片
// ---------------------------------------------------------------------------

/// 拆好的各段（给出片和给界面预览共用）
pub async fn split(config_dir: &Path, gateway: Option<&crate::Gateway>, prompt: &str, plan: &Plan) -> Result<(Vec<String>, &'static str), String> {
    match split_timed(prompt, plan) {
        Some(Ok(w)) => Ok((w, "按提示词里的时间拆")),
        Some(Err(e)) => Err(e),
        None => {
            let g = gateway.ok_or("这段提示词没写时间，要后台模型来拆，但这里拿不到后台模型 —— 在提示词里按时间写镜头")?;
            Ok((split_with_model(config_dir, g, prompt, plan).await?, "提示词没写时间，后台模型拆的"))
        }
    }
}

/// 超长视频出片。`args` 已经过 `@` 引用展开（[`crate::studio::generate_with_gateway`] 里做的）。
pub async fn render(config_dir: &Path, gateway: Option<&crate::Gateway>, args: &GenerateArgs) -> crate::error::Result<GenerateOutput> {
    use crate::error::Error;
    let total = args.duration.unwrap_or(0.0);
    if total > MAX_LONG_VIDEO_SECS + 0.01 {
        return Err(Error::config(format!("超长视频最长 {MAX_LONG_VIDEO_SECS:.0} 秒，现在是 {total:.0} 秒")));
    }
    if args.mode != "r2v" {
        return Err(Error::config(format!(
            "超过 15 秒的 H3 视频只能参考出片（分段串起来靠参考图认人），现在是「{}」—— 连上人物 / 场景图改成参考，或者把时长调回 15 秒以内",
            args.mode
        )));
    }
    let mut refs = args.images.clone();
    for p in &args.reference_images {
        if !refs.contains(p) {
            refs.push(p.clone());
        }
    }
    refs.truncate(crate::agent_runtime::video_tool::MAX_REFERENCE_IMAGES);
    if refs.is_empty() {
        return Err(Error::config("超长视频要至少一张参考图（人物或场景）—— 分段串起来靠它认人"));
    }
    let plan = Plan::new(total);
    let (parts, how) = split(config_dir, gateway, args.prompt.trim(), &plan).await.map_err(Error::config)?;
    // 画布勾了「H3 官方格式」：每段各自规整（时间码、引号），和单条出片同一个函数
    let parts: Vec<String> = if args.h3_official_format == Some(true) {
        parts.iter().map(|p| crate::h3_spec::canvas_official(p, &args.mode, plan.window_secs as f32)).collect()
    } else {
        parts
    };
    let ref_paths: Vec<PathBuf> = refs.iter().map(PathBuf::from).collect();
    let windows: Vec<(String, Vec<PathBuf>)> = parts.iter().map(|p| (p.clone(), ref_paths.clone())).collect();

    let workspace = args.workspace_path.as_deref().map(str::trim).filter(|p| !p.is_empty()).map(PathBuf::from).unwrap_or_else(|| config_dir.to_path_buf());
    let seed = args.seed.unwrap_or_else(|| crate::jobs::now() % 1_000_000_000);
    let out = workspace.join(".anima").join("generated").join(args.output_filename.clone().unwrap_or_else(|| format!("long_{seed}.mp4")));
    let generate_audio = args.generate_audio.unwrap_or(true);
    let tier = args.h3_quality.as_deref().and_then(|q| serde_json::from_value::<crate::cinema::RenderQuality>(serde_json::json!(q)).ok()).map(|q| q.tier());
    let loras: Vec<(String, f64)> = if args.loras.is_empty() {
        Vec::new()
    } else {
        let requested = crate::agent_runtime::lora::parse_requested(&serde_json::json!({
            "loras": args.loras.iter().map(|l| serde_json::json!({"name": l.name, "strength": l.strength})).collect::<Vec<_>>()
        }));
        let client = reqwest::Client::builder().no_proxy().timeout(std::time::Duration::from_secs(20)).build().unwrap_or_default();
        let (r, warn) = crate::agent_runtime::lora::resolve_loras(&requested, &crate::agent_runtime::lora::available_loras(&client).await);
        if let Some(w) = warn {
            tracing::warn!("超长视频的 LoRA：{w}");
        }
        r.into_iter().map(|l| (l.name, l.strength)).collect()
    };
    let summary = format!(
        "超长视频 {:.0} 秒：分 {} 段 × {:.2} 秒（{how}），一次串起来出完",
        total, plan.windows, plan.window_secs
    );
    // 重启续接（`render_resume::resume_canvas`）：和单条出片同一份续接单
    crate::jobs::note_resume(serde_json::json!({
        "kind": "canvas_video", "out": out.to_string_lossy(), "generate_audio": generate_audio, "summary": summary,
    }));
    crate::agent_runtime::video_tool::render_long_take(
        &windows, &out, plan.window_secs, crate::long_take::CONTEXT_FRAMES, args.width, args.height, seed, generate_audio, tier, &loras,
    )
    .await
    .map_err(Error::runtime)?;
    let p = out.to_string_lossy().to_string();
    Ok(GenerateOutput {
        path: Some(p.clone()),
        paths: vec![p.clone()],
        message: format!("{summary}，seed {seed}。\n文件：{p}"),
        notes: vec![format!("超长视频：{} 段 × {:.2} 秒，{how}", plan.windows, plan.window_secs)],
        ok: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn battle_like() -> String {
        // 30 秒、H3 官方格式：0 / 2.5 / 5.7 / 7.6 / 10 / 15.5 / 20 / 26 秒切
        "subject_definitions:\n<Subject 1> is Baiye.\n<Subject 2> is the Hunter.\n\nsummary:\n[reference generation] A rooftop duel.\n\n\
         retention_analysis:\n<Subject 1> (appears in [Shot 1], [Shot 2]): fully_preserved - face.\n<Subject 2> (appears in [Shot 2]): fully_preserved - body.\n\n\
         detailed_description:\nStyle paragraph with exactly one woman.\n\
         [Shot 1] <Subject 1> hovers above a puddle. The camera rises.\n\
         [Shot 2] At 00:02.500, the camera cuts to a wide shot of <Subject 2>. Around the 3.0-second mark, <Subject 2> says, <d>[Chinese] 零号。</d>\n\
         [Shot 3] At 00:05.700, the camera cuts to a close-up of <Subject 1>. Around the 6.2-second mark, <Subject 1> says, <d>[Chinese] 拒绝。</d>\n\
         [Shot 4] At 00:07.600, the camera cuts to <Subject 2> charging <Subject 1>.\n\
         [Shot 5] At 00:10.000, the camera cuts to a chase behind <Subject 1>. At 00:13.000 she glances back.\n\
         [Shot 6] At 00:15.500, the camera cuts to <Subject 1> firing at <Subject 2>.\n\
         [Shot 7] At 00:20.000, the camera cuts to a blade clash.\n\
         [Shot 8] At 00:26.000, the camera cuts to <Subject 1> backlit.\n\n\
         overall_soundscape: Wind and thrust.\n\nnon_diegetic_music: N/A"
            .into()
    }

    #[test]
    fn 段数和每段时长和工作台超长镜同一个公式() {
        let p = Plan::new(30.0);
        assert_eq!(p.windows, 3);
        assert!((p.window_secs - 10.61).abs() < 0.01, "{}", p.window_secs);
        assert!((p.end(2) - 30.0).abs() < 0.05, "最后一段收在 30 秒：{}", p.end(2));
        let p = Plan::new(75.0);
        assert_eq!(p.windows, 6);
        assert!(p.window_secs <= 15.0);
    }

    #[test]
    fn 按镜头时间码拆_每段都有头部_时间换成段内的() {
        let plan = Plan::new(30.0);
        let w = split_timed(&battle_like(), &plan).expect("认得出时间").expect("拆得开");
        assert_eq!(w.len(), 3);
        for (k, t) in w.iter().enumerate() {
            assert!(t.contains("<Subject 1> is Baiye."), "第 {} 段缺主体定义", k + 1);
            assert!(t.contains("Style paragraph with exactly one woman."), "第 {} 段缺画风那段", k + 1);
            assert!(t.contains("overall_soundscape: Wind and thrust."), "第 {} 段缺声音", k + 1);
            assert!(t.contains(&format!("Part {} of 3", k + 1)), "第 {} 段没写是第几段", k + 1);
            assert_eq!(t.matches("retention_analysis:").count(), 1);
            assert_eq!(t.matches("detailed_description:").count(), 1);
        }
        // 第 1 段：0~10.61 秒 —— 1~5 镜，第 5 镜 10 秒开始
        assert!(w[0].contains("[Shot 5] At 00:10.000"), "{}", w[0]);
        assert!(!w[0].contains("firing"), "15.5 秒的镜头不该进第 1 段");
        // 第 2 段从 9.69 秒开始：第 5 镜跨过来 → 段内第 1 镜写「接着同一个镜头」，13 秒那句换成段内 3.3 秒
        let s1 = plan.start(1);
                // 第 2 段从 9.69 秒起：7.6~10 秒那一镜（冲上来）跨进来成段内第 1 镜 —— 不能再写「the camera cuts to」
        assert!(w[1].contains("[Shot 1] Continuing the same shot from the previous moment with no cut: <Subject 2> charging <Subject 1>."), "{}", w[1]);
        assert!(w[1].contains(&format!("[Shot 2] At {}, the camera cuts to a chase behind <Subject 1>.", fmt_mmss(10.0 - s1))), "10 秒那一镜换成段内时间：{}", w[1]);
        assert!(w[1].contains(&format!("At {}", fmt_mmss(13.0 - s1))), "段内时间没换算：{}", w[1]);
        assert!(w[1].contains(&format!("At {}, the camera cuts to <Subject 1> firing", fmt_mmss(15.5 - s1))), "{}", w[1]);
        // 第 3 段：第 7 镜跨进来、第 8 镜在段内
        assert!(w[2].contains("Continuing the same shot") && w[2].contains("blade clash"), "{}", w[2]);
        assert!(w[2].contains("backlit"));
        // 台词只在它所在的那一段
        assert!(w[0].contains("零号") && !w[1].contains("零号") && !w[2].contains("零号"));
    }

    #[test]
    fn 保留说明按这一段实际出现的镜头重写() {
        let w = split_timed(&battle_like(), &Plan::new(30.0)).unwrap().unwrap();
        // 第 3 段从 19.39 秒起：15.5 秒那一镜（开枪）跨进来成段内第 1 镜，刀光第 2 镜，逆光第 3 镜
        let line = |t: &str, tag: &str| t.lines().find(|l| l.starts_with(&format!("{tag} (appears"))).unwrap().split(':').next().unwrap().to_string();
        assert_eq!(line(&w[2], "<Subject 1>"), "<Subject 1> (appears in [Shot 1], [Shot 3])");
        assert_eq!(line(&w[2], "<Subject 2>"), "<Subject 2> (appears in [Shot 1])");
        // 一段里一次都没点名的主体
        let p = "retention_analysis:\n<Subject 1> (appears in [Shot 1]): x\n<Subject 2> (appears in [Shot 2]): y\n\ndetailed_description:\n\
                 [Shot 1] <Subject 1> waits.\n[Shot 2] At 00:12.000, the camera cuts to <Subject 2> arriving.\n[Shot 3] At 00:25.000, the camera cuts to <Subject 2> leaving.";
        let w = split_timed(p, &Plan::new(30.0)).unwrap().unwrap();
        assert!(line(&w[2], "<Subject 1>").contains("none of the shots in this part"), "{}", w[2]);
    }

    #[test]
    fn 台词落在交接区_生成前拦下() {
        let plan = Plan::new(30.0);
        // 第 1 段末 2 秒 = 8.61~10.61 秒
        let p = battle_like().replace("At 00:13.000 she glances back.", "").replace(
            "[Shot 5] At 00:10.000, the camera cuts to a chase behind <Subject 1>.",
            "[Shot 5] At 00:09.000, the camera cuts to a chase. Around the 9.5-second mark, <Subject 1> says, <d>[Chinese] 走。</d>",
        );
        let e = split_timed(&p, &plan).unwrap().unwrap_err();
        assert!(e.contains("9.5 秒") && e.contains("第 1 段的最后 2 秒"), "{e}");
    }

    #[test]
    fn 行式时间也认() {
        let p = "一个女孩在雨夜的天台上。\n0-6秒：她站在边缘往下看。\n6-12秒：她转身跑向门口。\n12-20秒：门被撞开，她冲下楼梯。\n20-28秒：她跑进街道，消失在人群里。";
        let plan = Plan::new(28.0);
        let w = split_timed(p, &plan).unwrap().unwrap();
        assert_eq!(w.len(), 2);
        assert!(w.iter().all(|t| t.contains("一个女孩在雨夜的天台上")), "头部每段都要有");
        assert!(w[0].starts_with("一个女孩") && w[0].contains("0.0-6.0秒：她站在边缘"), "{}", w[0]);
        assert!(!w[0].contains("消失在人群里"));
        // 第 2 段从 13.08 秒起：12-20 那行跨过来，写明接着同一个画面
        assert!(w[1].contains("（接着上一段的同一个画面，不切）门被撞开"), "{}", w[1]);
        assert!(w[1].contains("消失在人群里"));
    }

    #[test]
    fn 没写时间_交给模型() {
        assert!(split_timed("一个女孩在雨夜奔跑，镜头一路跟着她穿过街道和人群。", &Plan::new(30.0)).is_none());
        assert!(split_timed("[Shot 1] A girl runs in the rain. The camera follows.", &Plan::new(30.0)).is_none(), "只有一镜、没有时间");
    }

    #[test]
    fn 模型拆的结果_段数不对就不收() {
        assert_eq!(parse_model_split(r#"{"windows":["a","b","c"]}"#, 3).unwrap(), ["a", "b", "c"]);
        assert!(parse_model_split(r#"{"windows":["a","b"]}"#, 3).unwrap_err().contains("要 3 段"));
        assert!(parse_model_split(r#"```json
{"windows":["a",""]}
```"#, 2).unwrap_err().contains("第 2 段是空的"));
        assert!(parse_model_split("没有 JSON", 2).is_err());
    }

    #[test]
    fn 什么时候走超长视频() {
        let mut a: GenerateArgs = serde_json::from_value(serde_json::json!({"mode":"r2v","engine":"h3","prompt":"x","width":960,"height":544,"duration":15.0})).unwrap();
        assert!(!wants(&a), "15 秒还是单条");
        a.duration = Some(30.0);
        assert!(wants(&a));
        a.engine = "ltx".into();
        assert!(!wants(&a));
    }
}

#[cfg(test)]
mod live {
    use super::*;

    /// **没写时间码的提示词 → 真的后台模型拆段**，把每段打出来看。
    /// `../scripts/with-env.sh cargo test --lib 超长视频模型拆段 -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "调真实的后台模型"]
    async fn 超长视频模型拆段() {
        let config_dir = dirs::config_dir().unwrap().join("com.anima.desktop");
        let app_cfg = crate::config::AppConfig::load(&config_dir.join("config.json")).unwrap();
        println!("后台模型：{} / {:?}", app_cfg.background_provider, app_cfg.background_model);
        let gateway = crate::llm::gateway::Gateway::new(app_cfg).unwrap();
        let prompt = "<Picture 1> 是星澜（银白长发、白色陶瓷战甲、周身悬浮青色晶片），<Picture 2> 是追猎机器人（哑光黑、红色横条视觉传感器）。\
                      黎明的顶层废墟上，星澜悬停在积水上方，背后喷出青蓝色推进光。追猎机器人从断墙后升起，背部喷红色火焰，两人对峙。\
                      机器人说：「零号，回收程序已启动。」星澜平静地说：「本机拒绝。」机器人冲上来横斩，星澜侧滚躲开，刀砍进断墙。\
                      星澜向后飞出塔顶，在下城金色铁皮屋顶上低空疾飞，机器人拖着红色尾迹紧追。最后她急停转身，举起电磁步枪，一道青色光束贯穿机器人胸口，机器人冒着黑烟坠落。";
        let plan = Plan::new(30.0);
        assert!(split_timed(prompt, &plan).is_none(), "这段没写时间，应该交给模型");
        let (w, how) = split(&config_dir, Some(&gateway), prompt, &plan).await.expect("模型没拆好");
        println!("{how}，{} 段 × {} 秒", w.len(), plan.window_secs);
        for (i, t) in w.iter().enumerate() {
            println!("\n===== 第 {} 段 =====\n{t}", i + 1);
        }
        assert_eq!(w.len(), 3);
        for t in &w {
            assert!(t.contains("<Picture 1>") && t.contains("<Picture 2>"), "参考图编号每段都要有");
        }
    }
}
