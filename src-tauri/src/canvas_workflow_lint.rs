//! 画布工作流**保存之前**的检查。
//!
//! 方案：`docs/plans/2026-09-23-Chat写画布工作流方案.md`。
//!
//! # 为什么要在保存时拦
//!
//! 画布模板出错的样子几乎全是**静默**的：连线指向一个不存在的节点，载入后那根线直接没了；
//! 引擎名写错，一跑才报「模型不可用」；H3 声音层里写了 `「砰！」`，老师就把「砰」念出来
//! （EP07 实跑）；人声栏写一句解释，解释被当台词念掉、真台词被挤到前面（EP06 实跑）。
//! 每一条都是任务显示成功、片子是错的，跑一次一个多小时。
//!
//! 这些规矩以前只在 `docs/` 和写模板的那个人脑子里。代理写模板时没法指望它全记得，
//! 所以**会静默出错的一律拒绝保存**（`errors`），只是让人困惑的给警告（`warnings`）。
//!
//! H3 那几条的出处：`skills/film-director/reference/h3-prompt-format.md` 1.1 / 1.2 / 4.5。

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value};

use crate::h3_prompt::{speakable_chars, NATURAL_CPS, TRUNCATE_CPS};

/// 检查结果。
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Lint {
    /// 会静默出错的 —— 有一条就不该保存
    pub errors: Vec<String>,
    /// 不影响出片，但会让人困惑
    pub warnings: Vec<String>,
}

impl Lint {
    fn err(&mut self, s: impl Into<String>) {
        self.errors.push(s.into());
    }
    fn warn(&mut self, s: impl Into<String>) {
        self.warnings.push(s.into());
    }
}

/// 跑出来的东西。**和前端 `workflowTemplate.ts` 的 `PRODUCT_KEYS` 是同一份**，有测试比对。
///
/// `notes` 在里面：它是**运行提示**（执行器每次开跑先清空再写），不是模板说明。
/// 模板说明放 `guide`。
pub const PRODUCT_KEYS: &[&str] = &[
    "outputImages", "outputImageUrl", "outputVideos", "outputVideoUrl",
    "longTakeUrl", "longTakeRunning", "outputUrl", "depthVideoUrl",
    "motionFirstFrame", "sourceVideoPath", "sourcePath", "requestedSourcePath",
    "lastRun", "lastExecutedSignature", "isGenerating", "progress",
    "error", "notes", "referenceError", "referenceLoading", "loading",
    "warnings", "unresolved", "dialogueWarning",
];

/// 和「这一集这一镜」绑死的字段。同上，和前端 `BINDING_KEYS` 一份。
pub const BINDING_KEYS: &[&str] = &["projectId", "episodeId", "shotId", "context", "sceneAssetId", "avatarUrl"];

/// 会生成东西的节点。只有它们需要步骤说明、引擎名。
const GENERATING_TYPES: &[&str] = &["imageGen", "videoGen", "upscale", "depthCapture"];

/// 把一份工作流收拾成模板的样子：剥产物和绑定字段、补齐 `version` / `name` / `slots` / `savedAt`。
///
/// 返回剥掉了哪些字段（`节点 id.字段`），给调用方告诉模型 —— 不说的话它会以为存进去的就是它写的。
pub fn normalize(v: &mut Value, name: &str, now_ms: u64) -> Vec<String> {
    let mut stripped = Vec::new();
    let Some(obj) = v.as_object_mut() else { return stripped };
    obj.insert("version".into(), Value::from(1));
    obj.insert("name".into(), Value::from(name));
    obj.insert("savedAt".into(), Value::from(now_ms));
    if !obj.get("slots").is_some_and(Value::is_array) {
        obj.insert("slots".into(), Value::Array(Vec::new()));
    }
    if !obj.get("edges").is_some_and(Value::is_array) {
        obj.insert("edges".into(), Value::Array(Vec::new()));
    }
    if let Some(nodes) = obj.get_mut("nodes").and_then(Value::as_array_mut) {
        for n in nodes.iter_mut() {
            let id = n.get("id").and_then(Value::as_str).unwrap_or("?").to_string();
            if let Some(data) = n.get_mut("data").and_then(Value::as_object_mut) {
                for k in PRODUCT_KEYS.iter().chain(BINDING_KEYS) {
                    if data.remove(*k).is_some() {
                        stripped.push(format!("{id}.{k}"));
                    }
                }
            }
            if let Some(o) = n.as_object_mut() {
                o.remove("selected");
                o.remove("dragging");
            }
        }
    }
    stripped
}

/// 检查一份工作流。`engines` = 能力表里认得的全部引擎 id。
pub fn lint(v: &Value, engines: &HashSet<String>) -> Lint {
    let mut out = Lint::default();
    let Some(nodes) = v.get("nodes").and_then(Value::as_array).filter(|a| !a.is_empty()) else {
        out.err("工作流里一个节点都没有（要有 nodes 数组）");
        return out;
    };

    // 接着深度动作捕捉的视频节点是**动作迁移**：H3 那档由后端包成「参考图 + 参考视频」的写法（`h3_motion_prompt`），
    // 不走三字段格式 —— 这些节点不做 H3 格式检查（2026-09-23 Kotte 路线 C 的模板被误拦）
    let types_by_id: HashMap<&str, &str> = nodes.iter()
        .filter_map(|n| Some((n.get("id")?.as_str()?, n.get("type")?.as_str()?)))
        .collect();
    let mut motion_nodes: HashSet<String> = v.get("edges").and_then(Value::as_array).into_iter().flatten()
        .filter(|e| e.get("source").and_then(Value::as_str).and_then(|s| types_by_id.get(s)) == Some(&"depthCapture"))
        .filter_map(|e| e.get("target").and_then(Value::as_str).map(str::to_string))
        .collect();
    // 2026-09-24 起原视频可以直接连出片节点（原片复刻，不经过深度节点）—— 和前端 `inferVideoMode` 同一个口径：
    // **节点上选了原片复刻**才算，光是视频连视频不算（旧画布里长镜头窗口链那种线还在，什么都不传）
    motion_nodes.extend(nodes.iter()
        .filter(|n| matches!(n.pointer("/data/motionEngine").and_then(Value::as_str), Some("h3_ref") | Some("h3_native")))
        .filter_map(|n| n.get("id")?.as_str().map(str::to_string)));

    // ── 节点 ──────────────────────────────────────────────
    let mut ids: HashSet<String> = HashSet::new();
    let mut types: HashMap<String, String> = HashMap::new();
    for (i, n) in nodes.iter().enumerate() {
        let Some(id) = n.get("id").and_then(Value::as_str).filter(|s| !s.trim().is_empty()) else {
            out.err(format!("第 {} 个节点没有 id", i + 1));
            continue;
        };
        if !ids.insert(id.to_string()) {
            out.err(format!("节点 id「{id}」重复了 —— 连线会接到错的节点上"));
        }
        let ty = n.get("type").and_then(Value::as_str).unwrap_or("");
        if ty.is_empty() {
            out.err(format!("节点「{id}」没有 type"));
        }
        types.insert(id.to_string(), ty.to_string());
        let pos_ok = n.get("position").is_some_and(|p| p.get("x").is_some_and(Value::is_number) && p.get("y").is_some_and(Value::is_number));
        if !pos_ok {
            out.err(format!("节点「{id}」没有 position {{x, y}} —— 画布摆不出来"));
        }
        let Some(data) = n.get("data").and_then(Value::as_object) else {
            out.err(format!("节点「{id}」没有 data"));
            continue;
        };
        lint_node(&mut out, id, ty, data, engines, motion_nodes.contains(id));
    }

    // ── 连线 ──────────────────────────────────────────────
    let edges = v.get("edges").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut linked: HashSet<String> = HashSet::new();
    let mut seen_pairs: HashSet<(String, String)> = HashSet::new();
    let mut incoming: HashMap<String, Vec<String>> = HashMap::new();
    for e in &edges {
        let (Some(s), Some(t)) = (e.get("source").and_then(Value::as_str), e.get("target").and_then(Value::as_str)) else {
            out.err("有一根连线缺 source 或 target");
            continue;
        };
        for end in [s, t] {
            if !ids.contains(end) {
                out.err(format!("连线 {s} → {t} 指向不存在的节点「{end}」—— 载入后这根线会直接消失"));
            }
        }
        if s == t {
            out.err(format!("节点「{s}」连到了自己"));
        }
        if !seen_pairs.insert((s.to_string(), t.to_string())) {
            out.warn(format!("{s} → {t} 连了两次"));
        }
        linked.insert(s.to_string());
        linked.insert(t.to_string());
        incoming.entry(t.to_string()).or_default().push(s.to_string());
    }
    if let Some(cycle_at) = find_cycle(&ids, &incoming) {
        out.err(format!("连线有环（经过「{cycle_at}」）—— 执行器会拒绝运行"));
    }
    for (id, ty) in &types {
        if ty != "group" && !linked.contains(id) && ids.len() > 1 {
            out.warn(format!("节点「{id}」一根线都没连 —— 它不会进任何提示词，也不会被当成参考图"));
        }
    }
    out
}

fn lint_node(out: &mut Lint, id: &str, ty: &str, data: &Map<String, Value>, engines: &HashSet<String>, motion: bool) {
    let generating = GENERATING_TYPES.contains(&ty);
    if generating || ty == "imageGen" || ty == "videoGen" {
        if let Some(engine) = data.get("engine").and_then(Value::as_str).filter(|s| !s.is_empty()) {
            if !engines.contains(engine) {
                let mut known: Vec<&String> = engines.iter().collect();
                known.sort();
                out.err(format!(
                    "节点「{id}」的引擎「{engine}」不认识 —— 载入后一跑就报模型不可用。认得的：{}",
                    known.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("、")
                ));
            }
        }
    }

    // 本机绝对路径：模板要能换一集、换一台机器用。带着路径不报错，只是指着别的集的文件
    let mut paths = Vec::new();
    collect_abs_paths(&Value::Object(data.clone()), "", &mut paths);
    for (key, p) in paths {
        out.err(format!("节点「{id}」的 {key} 是本机路径「{}」—— 模板不带文件路径，参考图让用户在节点上传", clip(&p, 60)));
    }

    let prompt = data.get("prompt").and_then(Value::as_str).unwrap_or("");
    if let Some(hit) = size_mention(prompt) {
        out.err(format!("节点「{id}」的提示词里写了画幅 / 尺寸 / 帧率「{hit}」—— 这些在节点上设，不写进提示词"));
    }

    // 说明文字
    let guide: Vec<&str> = data.get("guide").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let has_upload_role = prompt.trim().is_empty() && ty == "imageGen";
    if (generating || has_upload_role) && guide.is_empty() {
        out.warn(format!("节点「{id}」没写 guide（步骤说明）—— 用户看着画布不知道先点哪个"));
    }
    if guide.iter().any(|g| g.contains("**")) {
        out.warn(format!("节点「{id}」的 guide 里有 `**` —— 界面不渲染 markdown，会原样显示星号"));
    }

    if ty == "videoGen" && !motion && data.get("engine").and_then(Value::as_str) == Some("h3") && !prompt.trim().is_empty() {
        let windows = data.get("longTakeWindows").and_then(Value::as_array).map_or(0, Vec::len);
        if windows >= 2 {
            out.warn(format!("节点「{id}」是长镜头（{windows} 个窗口），H3 格式检查只查单段视频，窗口提示词没查"));
        } else {
            let secs = data.get("durationSecs").and_then(Value::as_f64).unwrap_or(5.0);
            lint_h3(out, id, prompt, secs);
        }
    }
}

/// H3 单段视频的提示词。规则出处见模块注释。
pub fn lint_h3(out: &mut Lint, id: &str, prompt: &str, secs: f64) {
    let has_desc = prompt.contains("integrated_multimodal_description:") || prompt.contains("detailed_description:");
    let mut missing = Vec::new();
    if !has_desc {
        missing.push("integrated_multimodal_description:");
    }
    if !prompt.contains("overall_soundscape:") {
        missing.push("overall_soundscape:");
    }
    if !prompt.contains("non_diegetic_music:") {
        missing.push("non_diegetic_music:（没有配乐也要写 N/A，不写 H3 会自己配）");
    }
    if !missing.is_empty() {
        out.err(format!("节点「{id}」的 H3 提示词缺字段：{} —— 写成散文也出片，但声画对不上", missing.join("、")));
        return;
    }
    let sound_at = prompt.find("overall_soundscape:").unwrap_or(prompt.len());
    let music_at = prompt.find("non_diegetic_music:").unwrap_or(prompt.len());
    let desc = &prompt[..sound_at];
    let sound = if music_at > sound_at { &prompt[sound_at..music_at] } else { &prompt[sound_at..] };

    for q in ['「', '」', '“', '”', '『', '』'] {
        if sound.contains(q) {
            out.err(format!(
                "节点「{id}」的声音层里有「{q}」—— 引号包的拟声词会被 H3 当成台词念出来（EP07「砰！」）。写成描述：一记沉闷的巨响"
            ));
            break;
        }
    }
    if sound.contains("<d>") {
        out.err(format!("节点「{id}」的声音层里有 <d> 台词 —— 台词只写在画面描述里，写两遍会打架"));
    }
    for voice in field_values(sound, "人声：") {
        if voice == "无" {
            continue;
        }
        let bad_word = ["台词", "对白", "秒"].into_iter().find(|w| voice.contains(w));
        if voice.chars().count() > 30 || bad_word.is_some() {
            out.err(format!(
                "节点「{id}」的人声栏「{}」不是短名词短语 —— 这一栏的解释会被当台词念出来（EP06）。只写「无」或「短促的惊叫与抽气，无词」这种",
                clip(&voice, 30)
            ));
        }
    }

    let segs_desc = segments(desc, '：');
    let segs_sound = segments(sound, '｜');
    if !segs_desc.is_empty() && !segs_sound.is_empty() && segs_desc != segs_sound {
        out.err(format!(
            "节点「{id}」画面的时间段 {} 和声音的时间段 {} 对不上 —— 声画会错位",
            fmt_segs(&segs_desc),
            fmt_segs(&segs_sound)
        ));
    }
    if let (Some(first), Some(last)) = (segs_desc.first(), segs_desc.last()) {
        if first.0.abs() > 0.01 {
            out.warn(format!("节点「{id}」的时间段从 {} 秒开始，不是 0", first.0));
        }
        if (last.1 - secs).abs() > 0.05 {
            out.warn(format!("节点「{id}」的时间段写到 {} 秒，节点时长是 {secs} 秒", last.1));
        }
        for w in segs_desc.windows(2) {
            if (w[0].1 - w[1].0).abs() > 0.01 {
                out.warn(format!("节点「{id}」的时间段在 {}~{} 秒之间断开了", w[0].1, w[1].0));
            }
        }
    }

    // 台词容量（h3-prompt-format.md 4.5）：≤ 4.1 字/秒自然；4.1~9 说得完但明显赶；> 9 **静默截断**。
    // 可用时间 = 到下一句开口为止（最后一句到片尾）—— 两句挤在一起时，前一句会被后一句截掉。
    let mut lines = dialogue_lines(desc);
    lines.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (n, (at, text)) in lines.iter().enumerate() {
        let until = lines.get(n + 1).map_or(secs, |next| next.0);
        let room = until - at;
        let units = spoken_units(text);
        let who = if n + 1 < lines.len() { format!("到下一句 {until} 秒开口") } else { format!("到片尾 {secs} 秒") };
        if room <= 0.0 {
            out.err(format!("节点「{id}」第 {} 句台词 {at} 秒开口，但{who}，一点时间都没有", n + 1));
            continue;
        }
        let cps = units / room;
        if cps > TRUNCATE_CPS {
            out.err(format!(
                "节点「{id}」第 {} 句台词约 {units:.0} 字，{at} 秒开口、{who}，只有 {room:.1} 秒 —— {cps:.1} 字/秒，超过 {TRUNCATE_CPS} 会被静默截掉。提前开口或删字",
                n + 1
            ));
        } else if cps > NATURAL_CPS {
            out.warn(format!(
                "节点「{id}」第 {} 句台词约 {units:.0} 字，{at} 秒开口、{who}，要 {cps:.1} 字/秒 —— 说得完，但会明显赶（自然是 {NATURAL_CPS}）。赶是故意的（发飙、喊口令）就不用管",
                n + 1
            ));
        } else if n + 1 == lines.len() && at + units / NATURAL_CPS + 0.3 > secs {
            out.warn(format!("节点「{id}」最后一句台词说完离结尾不到 0.3 秒（H3 常晚 0.3 秒出声）"));
        }
    }
}

/// 台词要念多少「字」。中文按字；英文和数字按词，一个词算 2 字（「Listen」「200」都差不多两个音节的时长）。
/// `speakable_chars` 把每个字母都算一个字，英文会被高估好几倍。
fn spoken_units(text: &str) -> f64 {
    let mut units = 0.0;
    let mut in_word = false;
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            if !in_word {
                units += 2.0;
                in_word = true;
            }
            continue;
        }
        in_word = false;
        if !c.is_ascii() && speakable_chars(&c.to_string()) == 1 {
            units += 1.0;
        }
    }
    units
}

/// `人声：xxx｜` / `人声：xxx\n` 里的 xxx。
fn field_values(text: &str, label: &str) -> Vec<String> {
    text.match_indices(label)
        .map(|(i, _)| {
            let rest = &text[i + label.len()..];
            let end = rest.find(['｜', '\n']).unwrap_or(rest.len());
            rest[..end].trim().trim_end_matches(['。', '.']).to_string()
        })
        .collect()
}

/// 行首的 `A~B 秒` + 分隔符。画面那边是 `：`，声音那边是 `｜`。
fn segments(text: &str, sep: char) -> Vec<(f64, f64)> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim_start();
            let (range, rest) = l.split_once(" 秒")?;
            if !rest.starts_with(sep) {
                return None;
            }
            let (a, b) = range.split_once('~')?;
            Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
        })
        .collect()
}

fn fmt_segs(s: &[(f64, f64)]) -> String {
    s.iter().map(|(a, b)| format!("{a}~{b}")).collect::<Vec<_>>().join(" / ")
}

/// `At 00:SS.S, …<d>[Chinese]台词</d>` → (开口秒数, 台词)。
fn dialogue_lines(desc: &str) -> Vec<(f64, String)> {
    let mut out = Vec::new();
    for (i, _) in desc.match_indices("At ") {
        let rest = &desc[i + 3..];
        let stamp: String = rest.chars().take_while(|c| c.is_ascii_digit() || *c == ':' || *c == '.').collect();
        let Some((m, s)) = stamp.split_once(':') else { continue };
        let (Ok(m), Ok(s)) = (m.parse::<f64>(), s.parse::<f64>()) else { continue };
        let Some(open) = rest.find("<d>") else { continue };
        let Some(close) = rest[open..].find("</d>") else { continue };
        let mut body = &rest[open + 3..open + close];
        if body.starts_with('[') {
            if let Some(e) = body.find(']') {
                body = &body[e + 1..];
            }
        }
        out.push((m * 60.0 + s, body.to_string()));
    }
    out
}

/// 提示词里的画幅 / 尺寸 / 帧率写法。返回命中的那一截。
///
/// 比例只在两边都不是数字时才算（`00:14:30` 里的 `4:3` 不算）。
pub fn size_mention(prompt: &str) -> Option<String> {
    for w in ["横构图", "竖构图", "超宽画幅", "fps", "FPS"] {
        if prompt.contains(w) {
            return Some(w.to_string());
        }
    }
    let chars: Vec<char> = prompt.chars().collect();
    let digit_run = |from: usize, dir: isize| -> usize {
        let mut n = 0;
        let mut i = from as isize;
        while i >= 0 && (i as usize) < chars.len() && chars[i as usize].is_ascii_digit() {
            n += 1;
            i += dir;
        }
        n
    };
    for (i, c) in chars.iter().enumerate() {
        let left = if i == 0 { 0 } else { digit_run(i - 1, -1) };
        let right = digit_run(i + 1, 1);
        if left == 0 || right == 0 {
            continue;
        }
        let token: String = chars[i - left..=i + right].iter().collect();
        let hit = match c {
            ':' => ["16:9", "9:16", "21:9", "4:3", "3:4", "2:3", "3:2"].contains(&token.as_str()),
            '×' | 'x' | '*' => (3..=4).contains(&left) && (3..=4).contains(&right),
            _ => false,
        };
        if hit {
            return Some(token);
        }
    }
    None
}

fn collect_abs_paths(v: &Value, key: &str, out: &mut Vec<(String, String)>) {
    match v {
        Value::String(s) => {
            let t = s.trim();
            let looks_local = t.starts_with("file://")
                || t.starts_with("/Users/")
                || t.starts_with("/private/")
                || t.starts_with("/tmp/")
                || t.starts_with("/var/")
                || t.starts_with("/Volumes/")
                || t.starts_with("~/");
            if looks_local {
                out.push((key.to_string(), t.to_string()));
            }
        }
        Value::Array(a) => a.iter().for_each(|x| collect_abs_paths(x, key, out)),
        Value::Object(o) => o.iter().for_each(|(k, x)| collect_abs_paths(x, if key.is_empty() { k } else { key }, out)),
        _ => {}
    }
}

fn find_cycle(ids: &HashSet<String>, incoming: &HashMap<String, Vec<String>>) -> Option<String> {
    // 0 = 没访问，1 = 在栈上，2 = 做完
    let mut state: HashMap<&str, u8> = HashMap::new();
    fn dfs<'a>(id: &'a str, incoming: &'a HashMap<String, Vec<String>>, state: &mut HashMap<&'a str, u8>) -> Option<String> {
        match state.get(id) {
            Some(1) => return Some(id.to_string()),
            Some(2) => return None,
            _ => {}
        }
        state.insert(id, 1);
        for src in incoming.get(id).map(Vec::as_slice).unwrap_or(&[]) {
            if let Some(c) = dfs(src, incoming, state) {
                return Some(c);
            }
        }
        state.insert(id, 2);
        None
    }
    let mut sorted: Vec<&String> = ids.iter().collect();
    sorted.sort();
    sorted.into_iter().find_map(|id| dfs(id, incoming, &mut state))
}

fn clip(s: &str, n: usize) -> String {
    let t: String = s.chars().take(n).collect();
    if s.chars().count() > n { format!("{t}…") } else { t }
}

/// 能力表里认得的全部引擎 id（引擎列表 + 各模式的引擎列表 —— `krea2_edit` 只在后者里）。
pub fn known_engines() -> HashSet<String> {
    let cat = crate::studio::studio_capabilities();
    let mut s: HashSet<String> = cat.engines.iter().map(|e| e.id.to_string()).collect();
    for m in &cat.modes {
        s.extend(m.engines.iter().map(|e| e.to_string()));
    }
    s
}

#[cfg(test)]
mod tests {
    /// 手写（脚本生成）的模板照 `studio_workflow_save` 同一套检查存进画布 —— 用于打包版的 anima-mcp 还没有 studio_workflow_* 工具时。
    /// 有 errors 就不存；同名已存在就拒（要覆盖先在画布里删掉，或者换名字）。
    ///
    /// ```text
    /// WF_FILE=wf.json WF_PROJECT=drama_xxx WF_NAME=名字 WF_CFG="$HOME/Library/Application Support/com.anima.desktop" \
    ///   cargo test --lib 按studio_workflow_save的检查存模板 -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore]
    fn 按studio_workflow_save的检查存模板() {
        let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("缺 {k}"));
        let cfg = std::path::PathBuf::from(env("WF_CFG"));
        let (project, name) = (env("WF_PROJECT"), env("WF_NAME"));
        assert!(crate::canvas_workflows::load(&cfg, &project, &name).is_err(), "「{name}」已经有了，不覆盖");
        let mut v: Value = serde_json::from_str(&std::fs::read_to_string(env("WF_FILE")).unwrap()).unwrap();
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
        let stripped = super::normalize(&mut v, &name, now);
        let l = super::lint(&v, &super::known_engines());
        println!("stripped: {stripped:?}\nerrors: {:#?}\nwarnings: {:#?}", l.errors, l.warnings);
        assert!(l.errors.is_empty(), "有 errors，没存");
        crate::canvas_workflows::save(&cfg, &project, &name, &serde_json::to_string_pretty(&v).unwrap()).unwrap();
        println!("存好了：{project} / {name}（{} 个节点）", v["nodes"].as_array().map_or(0, Vec::len));
    }

    use super::*;
    use serde_json::json;

    /// 原视频直连出片节点（原片复刻）的提示词是逐毫秒母版，不是 H3 三字段 —— 不能被当成普通 H3 视频拦下
    #[test]
    fn 原视频直连出片_选了原片复刻才不套h3三字段检查() {
        let wf = json!({"nodes": [
            {"id": "clip", "type": "videoGen", "position": {"x": 0, "y": 0}, "data": {"prompt": "", "guide": ["上传"]}},
            {"id": "out", "type": "videoGen", "position": {"x": 500, "y": 0}, "data": {"engine": "h3", "durationSecs": 8.5, "motionEngine": "h3_ref",
                "prompt": "把参考视频1中的角色完全替换成图片1中的角色", "guide": ["生成"]}},
            // 视频连视频但没选原片复刻：前端不当动作迁移，这里也照普通 H3 查
            {"id": "tool", "type": "videoGen", "position": {"x": 500, "y": 600}, "data": {"engine": "h3",
                "prompt": "把参考视频1中的角色完全替换成图片1中的角色", "guide": ["出片"]}}],
            "edges": [{"id": "e1", "source": "clip", "target": "out"}, {"id": "e2", "source": "clip", "target": "tool"}]});
        let l = lint(&wf, &known_engines());
        assert!(!l.errors.iter().chain(&l.warnings).any(|m| m.contains("「out」")), "直连的出片节点不该被 H3 格式检查：{l:?}");
        assert!(l.errors.iter().chain(&l.warnings).any(|m| m.contains("「tool」")), "没选原片复刻的视频连视频照旧检查：{l:?}");
    }

    fn engines() -> HashSet<String> {
        ["krea2", "krea2_edit", "h3", "zimage"].iter().map(|s| s.to_string()).collect()
    }

    const GOOD_H3: &str = "integrated_multimodal_description: 教室里很安静。\n\
        0~2 秒：她转头。\n\
        2~5 秒：老师走来。\n\
        At 00:02.5, 老师说：<d>[Chinese]上课不要玩手机</d>\n\
        \n\
        overall_soundscape: 分四层。\n\
        0~2 秒｜环境层：安静｜动作层：翻书声｜冲击层：无｜人声：无\n\
        2~5 秒｜环境层：安静｜动作层：脚步声，一声比一声近｜冲击层：无｜人声：几名学生短促的无词惊叫\n\
        \n\
        non_diegetic_music: N/A";

    fn wf(video_prompt: &str) -> Value {
        json!({
            "nodes": [
                {"id": "img", "type": "imageGen", "position": {"x": 0, "y": 0},
                 "data": {"engine": "krea2", "prompt": "一张教室的照片", "guide": ["【第 1 步 / 共 2 步】出首帧"]}},
                {"id": "vid", "type": "videoGen", "position": {"x": 400, "y": 0},
                 "data": {"engine": "h3", "durationSecs": 5, "prompt": video_prompt, "guide": ["【第 2 步 / 共 2 步】出视频"]}},
            ],
            "edges": [{"id": "e1", "source": "img", "target": "vid"}],
        })
    }

    fn errors_of(v: &Value) -> Vec<String> {
        lint(v, &engines()).errors
    }

    #[test]
    fn 规矩都守住的工作流没有错误也没有警告() {
        let l = lint(&wf(GOOD_H3), &engines());
        assert!(l.errors.is_empty(), "{:?}", l.errors);
        assert!(l.warnings.is_empty(), "{:?}", l.warnings);
    }

    #[test]
    fn 连线指向不存在的节点() {
        let mut v = wf(GOOD_H3);
        v["edges"] = json!([{"id": "e1", "source": "img", "target": "ghost"}]);
        assert!(errors_of(&v).iter().any(|e| e.contains("ghost")));
    }

    #[test]
    fn 节点id重复() {
        let mut v = wf(GOOD_H3);
        v["nodes"][1]["id"] = json!("img");
        assert!(errors_of(&v).iter().any(|e| e.contains("重复")));
    }

    #[test]
    fn 连线有环() {
        let mut v = wf(GOOD_H3);
        v["edges"] = json!([{"id": "a", "source": "img", "target": "vid"}, {"id": "b", "source": "vid", "target": "img"}]);
        assert!(errors_of(&v).iter().any(|e| e.contains("有环")));
    }

    #[test]
    fn 引擎名写错() {
        let mut v = wf(GOOD_H3);
        v["nodes"][0]["data"]["engine"] = json!("flux");
        assert!(errors_of(&v).iter().any(|e| e.contains("flux")));
    }

    #[test]
    fn 带本机路径() {
        let mut v = wf(GOOD_H3);
        v["nodes"][0]["data"]["references"] = json!([{"path": "/Users/me/ep05/a.png", "name": "旧图"}]);
        assert!(errors_of(&v).iter().any(|e| e.contains("本机路径")));
    }

    #[test]
    fn 提示词里写尺寸() {
        for bad in ["横构图 16:9，写实", "21:9 超宽画幅", "出 1344×576 的图", "24fps。"] {
            let mut v = wf(GOOD_H3);
            v["nodes"][0]["data"]["prompt"] = json!(bad);
            assert!(errors_of(&v).iter().any(|e| e.contains("尺寸")), "{bad}");
        }
        // 时间码里的 4:3 不算
        assert_eq!(size_mention("At 00:14:30 她说"), None);
        assert_eq!(size_mention("0~2 秒：她转头"), None);
    }

    #[test]
    fn h3缺字段() {
        let v = wf("她走进教室，老师说话。");
        assert!(errors_of(&v).iter().any(|e| e.contains("缺字段") && e.contains("non_diegetic_music")));
    }

    #[test]
    fn 声音层有引号拟声词() {
        let v = wf(&GOOD_H3.replace("翻书声", "拍桌的巨响「砰！」"));
        assert!(errors_of(&v).iter().any(|e| e.contains("声音层里有")));
    }

    #[test]
    fn 声音层有台词标签() {
        let v = wf(&GOOD_H3.replace("｜动作层：翻书声", "｜动作层：<d>[Chinese]喂</d>"));
        assert!(errors_of(&v).iter().any(|e| e.contains("<d>")));
    }

    #[test]
    fn 人声栏写了解释() {
        let v = wf(&GOOD_H3.replacen("人声：无", "人声：无 —— 这段静默是唯一一句台词之前的蓄力", 1));
        assert!(errors_of(&v).iter().any(|e| e.contains("人声栏")));
        let v = wf(&GOOD_H3.replacen("人声：无", "人声：3 秒后老师开口", 1));
        assert!(errors_of(&v).iter().any(|e| e.contains("人声栏")));
    }

    #[test]
    fn 画面和声音的时间段对不上() {
        let v = wf(&GOOD_H3.replace("2~5 秒｜", "2~4.5 秒｜"));
        assert!(errors_of(&v).iter().any(|e| e.contains("对不上")));
    }

    #[test]
    fn 台词说不完() {
        // 7 个字只剩 0.4 秒：17.5 字/秒，会被截
        let v = wf(&GOOD_H3.replace("At 00:02.5", "At 00:04.6"));
        assert!(errors_of(&v).iter().any(|e| e.contains("截掉")));
    }

    #[test]
    fn 台词赶但说得完是提醒不是错误() {
        // 7 个字 1.2 秒：5.8 字/秒，在 4.1~9 之间
        let v = wf(&GOOD_H3.replace("At 00:02.5", "At 00:03.8"));
        let l = lint(&v, &engines());
        assert!(l.errors.is_empty(), "{:?}", l.errors);
        assert!(l.warnings.iter().any(|w| w.contains("明显赶")), "{:?}", l.warnings);
    }

    #[test]
    fn 两句台词挤在一起_前一句被截() {
        let two = GOOD_H3.replace(
            "At 00:02.5, 老师说：<d>[Chinese]上课不要玩手机</d>",
            "At 00:01.0, 老师说：<d>[Chinese]上课不要玩手机拿出来交给我</d>\nAt 00:02.0, 学生说：<d>[Chinese]好</d>",
        );
        assert!(errors_of(&wf(&two)).iter().any(|e| e.contains("第 1 句") && e.contains("截掉")), "{:?}", errors_of(&wf(&two)));
    }

    #[test]
    fn 英文按词算不按字母算() {
        assert_eq!(spoken_units("Listen to me!"), 6.0);
        assert_eq!(spoken_units("200焦耳"), 4.0);
        assert_eq!(spoken_units("上课不要玩手机"), 7.0);
    }

    #[test]
    fn 时间段没写到结尾是警告() {
        let mut v = wf(GOOD_H3);
        v["nodes"][1]["data"]["durationSecs"] = json!(8);
        let l = lint(&v, &engines());
        assert!(l.errors.is_empty(), "{:?}", l.errors);
        assert!(l.warnings.iter().any(|w| w.contains("写到 5 秒")));
    }

    #[test]
    fn 没写步骤说明和星号是警告() {
        let mut v = wf(GOOD_H3);
        v["nodes"][0]["data"].as_object_mut().unwrap().remove("guide");
        v["nodes"][1]["data"]["guide"] = json!(["**第 2 步**"]);
        let l = lint(&v, &engines());
        assert!(l.errors.is_empty());
        assert!(l.warnings.iter().any(|w| w.contains("img") && w.contains("guide")));
        assert!(l.warnings.iter().any(|w| w.contains("星号")));
    }

    #[test]
    fn 接着深度动作捕捉的_h3_节点是动作迁移_不查三字段() {
        let v = json!({
            "nodes": [
                {"id": "d", "type": "depthCapture", "position": {"x": 0, "y": 0}, "data": {"guide": ["抽深度"]}},
                {"id": "m", "type": "videoGen", "position": {"x": 1, "y": 0}, "data": {"engine": "h3", "prompt": "the gambler fires", "guide": ["出片"]}},
                {"id": "p", "type": "videoGen", "position": {"x": 2, "y": 0}, "data": {"engine": "h3", "prompt": "the gambler fires", "guide": ["出片"]}},
            ],
            "edges": [{"id": "e", "source": "d", "target": "m"}, {"id": "f", "source": "m", "target": "p"}],
        });
        let errs = errors_of(&v);
        assert!(!errs.iter().any(|e| e.contains("「m」") && e.contains("缺字段")), "{errs:?}");
        assert!(errs.iter().any(|e| e.contains("「p」") && e.contains("缺字段")), "普通 H3 出片照样要查：{errs:?}");
    }

    #[test]
    fn 孤立节点是警告() {
        let mut v = wf(GOOD_H3);
        v["nodes"].as_array_mut().unwrap().push(json!({"id": "lonely", "type": "prompt", "position": {"x": 0, "y": 0}, "data": {"prompt": "火箭筒"}}));
        assert!(lint(&v, &engines()).warnings.iter().any(|w| w.contains("lonely")));
    }

    #[test]
    fn 收拾成模板_剥产物和绑定_补齐字段() {
        let mut v = wf(GOOD_H3);
        v["nodes"][0]["data"]["outputImageUrl"] = json!("/Users/me/a.png");
        v["nodes"][0]["data"]["notes"] = json!(["上次运行的提示"]);
        v["nodes"][1]["data"]["projectId"] = json!("drama_1");
        let stripped = normalize(&mut v, "EP09", 42);
        assert_eq!(stripped, vec!["img.outputImageUrl", "img.notes", "vid.projectId"]);
        assert_eq!((v["version"].as_i64(), v["name"].as_str(), v["savedAt"].as_u64()), (Some(1), Some("EP09"), Some(42)));
        assert!(v["slots"].is_array());
        assert!(v["nodes"][0]["data"]["guide"].is_array(), "guide 是模板说明，不能被剥");
        assert!(lint(&v, &engines()).errors.is_empty());
    }

    /// 两边清单分叉的样子：前端存模板剥了某个字段、代理存的时候没剥（或反过来），
    /// 模板里就带着上一次的产物 —— 看着像这次出的。
    #[test]
    fn 剥字段的清单和前端一致() {
        let ts = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../src/canvas/utils/workflowTemplate.ts")).unwrap();
        let list_of = |name: &str| -> Vec<String> {
            let start = ts.find(&format!("const {name} = [")).unwrap_or_else(|| panic!("前端找不到 {name}"));
            let body = &ts[start..start + ts[start..].find("] as const").unwrap()];
            body.split('"').skip(1).step_by(2).map(str::to_string).collect()
        };
        assert_eq!(list_of("PRODUCT_KEYS"), PRODUCT_KEYS.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(list_of("BINDING_KEYS"), BINDING_KEYS.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    }

    #[test]
    fn 能力表里认得画布用到的引擎() {
        let e = known_engines();
        for id in ["krea2", "krea2_edit", "zimage", "h3", "ltx"] {
            assert!(e.contains(id), "{id}");
        }
    }

    /// 这两套是手写的、实跑过的模板 —— 检查工具要放行它们。放不过去的话，要么是模板真有问题，
    /// 要么是规则写严了；两种都得在这里看见，而不是等用户存模板时被拒。
    #[test]
    #[ignore = "公开版不含 docs/reports 里的画布模板"]
    fn 现有的画布模板都过得了检查() {
        for f in ["ep01-2026-09-23/workflow.json", "ep07-prompt-update-2026-09-22/workflow-updated.json", "ep08-2026-09-22/workflow.json",
                  "ep09-11-2026-09-23/ep09-workflow.json", "ep09-11-2026-09-23/ep10-workflow.json", "ep09-11-2026-09-23/ep11-workflow.json", "kotte-2026-09-23/workflow.json", "kotte-2026-09-23/workflow_c.json", "kotte-2026-09-23/plan-d/workflow_d.json", "ep06-2026-09-23/workflow.json"] {
            let path = format!("{}/../docs/reports/{f}", env!("CARGO_MANIFEST_DIR"));
            let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            let l = lint(&v, &known_engines());
            assert!(l.errors.is_empty(), "{f}：{:#?}", l.errors);
            eprintln!("{f} 的提醒：{:#?}", l.warnings);
        }
    }
}

#[cfg(test)]
mod save_live {
    /// **手动存一份画布工作流**：和 `studio_workflow_save` 同一套 —— 归一化、检查、有 errors 不存。
    /// 给没有应用内对话可用的时候（Claude Code 这边 MCP 不暴露 studio_workflow_*）。同名已存在时拒绝，除非 `ANIMA_WF_OVERWRITE=1`。
    /// `ANIMA_WF_PROJECT=drama_… ANIMA_WF_NAME=… ANIMA_WF_FILE=/abs/wf.json cargo test --lib 存画布工作流 -- --ignored --nocapture`
    #[test]
    #[ignore = "写真实的画布工作流目录"]
    fn 存画布工作流() {
        let cfg = dirs::config_dir().unwrap().join("com.anima.desktop");
        let project = std::env::var("ANIMA_WF_PROJECT").expect("要 ANIMA_WF_PROJECT");
        let name = std::env::var("ANIMA_WF_NAME").expect("要 ANIMA_WF_NAME");
        let text = std::fs::read_to_string(std::env::var("ANIMA_WF_FILE").expect("要 ANIMA_WF_FILE")).unwrap();
        if crate::canvas_workflows::load(&cfg, &project, &name).is_ok() && std::env::var_os("ANIMA_WF_OVERWRITE").is_none() {
            panic!("「{name}」已经有了：要覆盖带 ANIMA_WF_OVERWRITE=1（先确认用户没在画布上改过）");
        }
        let mut v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
        let stripped = super::normalize(&mut v, &name, now);
        let l = super::lint(&v, &super::known_engines());
        println!("去掉的：{stripped:?}");
        println!("提醒（{}）：", l.warnings.len());
        for w in &l.warnings { println!("  - {w}"); }
        assert!(l.errors.is_empty(), "没存 —— 有 {} 处会静默出错：\n- {}", l.errors.len(), l.errors.join("\n- "));
        crate::canvas_workflows::save(&cfg, &project, &name, &serde_json::to_string_pretty(&v).unwrap()).unwrap();
        println!("存好了：{project} / {name}（{} 个节点）", v["nodes"].as_array().map_or(0, Vec::len));
    }
}
