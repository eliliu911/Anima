//! **MiniMax H3 官方提示词规范**：官方原文编译时嵌入，按生产方式取用（2026-10-06）。
//!
//! 用户：「重新去获取官方最新版本提示词、生产模式以及对应的规范，并在 Anima 中对应不同生产方式使用不同的规范进行生成。」
//!
//! 原文：`skills/film-director/reference/h3-official/`，取自 HF `MiniMaxAI/MiniMax-H3` 的 `docs/`
//! （[`SOURCE_REVISION`]）。官方 GitHub 的 `skills/h3-prompt-writing` 引用的是同两份，逐字相同。
//! 官方的改写层 Context-IR 没开源，所以这两份就是全部的官方规范。**改规则先更新原文文件，不在代码里改写官方的话。**
//!
//! | Anima 怎么出片 | WT 任务 | 官方模式 |
//! |---|---|---|
//! | 没挂任何图 | t2va | [`Mode::T2va`] |
//! | 挂了首帧 | fl2va（1 张 keyframe，和官方复现脚本一样） | [`Mode::I2va`] |
//! | 挂了首帧 + 尾帧 | fl2va（2 张） | [`Mode::Fl2va`] |
//! | 资产参考图 / 接上一镜 / 长镜头窗口 / 超长镜各段 | ref2va | [`Mode::Ref2va`] |
//!
//! L2VA（只有尾帧）Anima 不会产生，没做。

/// 官方基础模式指南（T2VA / I2VA / FL2VA / L2VA）原文。
pub const BASE_GUIDE: &str = include_str!("../../skills/film-director/reference/h3-official/VIDEO_PROMPT_WRITING_GUIDE_base_en.md");
/// 官方全参考模式指南（Ref2VA）原文。
pub const REF_GUIDE: &str = include_str!("../../skills/film-director/reference/h3-official/VIDEO_PROMPT_WRITING_GUIDE_ref_en.md");
/// 原文取自 HF 的哪个版本。更新原文时一起改，`SOURCE.md` 里有校验和。
pub const SOURCE_REVISION: &str = "42ed227ee7df40d41602854ae760620d6eb651fe";

/// 这一镜按官方哪种模式写。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// 纯文生：三字段，没有对齐指令
    T2va,
    /// 首帧：对齐指令 + 三字段
    I2va,
    /// 首尾帧：对齐指令 + 三字段
    Fl2va,
    /// 全参考：六段
    Ref2va,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::T2va => "T2VA",
            Mode::I2va => "I2VA",
            Mode::Fl2va => "FL2VA",
            Mode::Ref2va => "Ref2VA",
        }
    }

    /// 三字段的基础模式（不用 `<Subject N>`，没有 summary / retention）
    pub fn is_base(self) -> bool {
        self != Mode::Ref2va
    }
}

/// I2VA 的对齐指令，**逐字照官方**（基础指南 2.1）。必须是第一行，后面空一行。
pub const I2VA_INSTRUCTION: &str = "For the target video, at 0.00 seconds into the target video, <Picture 1> (from [Shot 1]) is fully referenced.";

/// FL2VA 的对齐指令（基础指南 2.1）。**官方原文这一行的 Picture / Shot 不带尖括号、方括号**，照抄，不「统一」成别的写法。
/// `last_shot` 是最后一个镜头的编号，`secs` 是成片时长（两位小数）。
pub fn fl2va_instruction(last_shot: u32, secs: f32) -> String {
    format!(
        "How the reference pictures align with the target video — Picture 1 (from Shot 1) aligns with the 0.00-second mark of the target video; Picture 2 (from Shot {last_shot}) aligns with the {secs:.2}-second mark of the target video."
    )
}

/// 切镜时间码 `MM:SS.mmm`（官方：`[Shot 2] At 00:03.500, the camera cuts to …`，严格递增、落在时长以内）。
pub fn cut_timecode(secs: f32) -> String {
    let ms = (secs.max(0.0) * 1000.0).round() as u64;
    format!("{:02}:{:02}.{:03}", ms / 60_000, ms / 1000 % 60, ms % 1000)
}

/// 一个镜头**之内**的时刻。官方没有镜内时间码（`At MM:SS` 只用来标切镜），所以写成自然英文，
/// 不长得像切镜：`Around the 3.5-second mark,`。代码靠它逐段核对、核对台词说不说得完（见 `prompt_route_b::timecodes`）。
pub fn beat_marker(secs: f32) -> String {
    format!("Around the {secs:.1}-second mark,")
}

/// 给写手看的官方规则：这种模式用得上的官方原文章节（原样，不改写）。
///
/// 基础模式去掉别的模式的关键帧说明（3.x）和案例；全参考给整份。
pub fn writer_rules(mode: Mode) -> String {
    match mode {
        Mode::Ref2va => REF_GUIDE.trim().to_string(),
        _ => {
            let keep = |heading: &str| -> bool {
                match mode {
                    Mode::T2va => !heading.starts_with("### 3.") && !heading.starts_with("### Case") || heading.contains("T2VA"),
                    Mode::I2va => (!heading.starts_with("### 3.") || heading.contains("I2VA")) && (!heading.starts_with("### Case") || heading.contains("I2VA")),
                    Mode::Fl2va => (!heading.starts_with("### 3.") || heading.contains("FL2VA")) && (!heading.starts_with("### Case") || heading.contains("FL2VA")),
                    Mode::Ref2va => true,
                }
            };
            let mut out = String::new();
            let mut on = true;
            for line in BASE_GUIDE.lines() {
                if line.starts_with("## ") || line.starts_with("### ") {
                    on = keep(line);
                }
                if on {
                    out.push_str(line);
                    out.push('\n');
                }
            }
            out.trim().to_string()
        }
    }
}

/// 正文里最大的 `[Shot N]` 编号（FL2VA 对齐指令要写尾帧属于哪一镜）。
pub fn last_shot_number(desc: &str) -> u32 {
    desc.match_indices("[Shot ")
        .filter(|(i, _)| !desc[..*i].ends_with("(from "))
        .filter_map(|(i, _)| desc[i + 6..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse::<u32>().ok())
        .max()
        .unwrap_or(1)
}


/// 写手的时间写法规整成官方的（基础指南 4.2 / ref 指南 5.1）：
///
/// - `[Shot 1]` **不带时间码**：开头的 `At 00:00.0,` 去掉；
/// - 镜头**之内**的 `At 00:SS.S,` → `Around the SS.S-second mark,`（官方的 `At MM:SS.mmm` 只标切镜，镜内这么写可能被读成切点）；
/// - 一镜出允许的切镜 `At 00:SS.S, hard cut to …` → `[Shot N] At 00:SS.mmm, the camera cuts to …`；
/// - 写手已经按官方写的 `[Shot N] At 00:03.500,` 原样留着。
///
/// 规整在拼装时做，写手写成哪种都出官方格式；老提示词的校验照样认两种写法（[`timecodes`]）。
pub fn official_timing(desc: &str, cuts_allowed: bool) -> String {
    let mut out = String::new();
    let mut rest = desc;
    let mut next_shot = last_shot_number(desc) + 1;
    while let Some(i) = rest.find("At ") {
        let Some((secs, len)) = parse_clock(&rest[i + 3..]) else {
            out.push_str(&rest[..i + 3]);
            rest = &rest[i + 3..];
            continue;
        };
        let before = &rest[..i];
        let after = rest[i + 3 + len..].trim_start_matches(',').trim_start();
        let shot_label = before.trim_end().ends_with(']') && before.trim_end().rsplit('[').next().is_some_and(|b| b.starts_with("Shot "));
        if shot_label {
            // `[Shot N] At …`：官方切镜写法，只把时间规整成三位小数；`[Shot 1]` 后面的去掉
            if before.trim_end().ends_with("[Shot 1]") {
                out.push_str(before.trim_end());
                out.push(' ');
            } else {
                out.push_str(&format!("{before}At {}, ", cut_timecode(secs)));
            }
        } else if cuts_allowed && is_cut_phrase(after) {
            let what = strip_cut_phrase(after);
            out.push_str(&format!("{before}[Shot {next_shot}] At {}, the camera cuts to ", cut_timecode(secs)));
            next_shot += 1;
            rest = what;
            continue;
        } else {
            out.push_str(before);
            out.push_str(&beat_marker(secs));
            out.push(' ');
        }
        rest = after;
    }
    out.push_str(rest);
    renumber_shots(&out)
}

/// `[Shot N]` 按出现顺序重新编号（官方：「Use sequential shot numbers」）。
fn renumber_shots(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    let mut n = 0;
    while let Some(i) = rest.find("[Shot ") {
        let tail = &rest[i + 6..];
        let digits = tail.chars().take_while(|c| c.is_ascii_digit()).count();
        // 对齐指令里的 `(from [Shot 1])` 是在说「这张图属于第几镜」，不是一个镜头 —— 不算、不改
        if digits == 0 || !tail[digits..].starts_with(']') || rest[..i].ends_with("(from ") {
            out.push_str(&rest[..i + 6]);
            rest = tail;
            continue;
        }
        n += 1;
        out.push_str(&rest[..i]);
        out.push_str(&format!("[Shot {n}]"));
        rest = &tail[digits + 1..];
    }
    out.push_str(rest);
    out
}

/// `MM:SS` / `MM:SS.s…` 开头的时钟时间，回 `(秒, 吃掉的字节数)`。
///
/// 秒数一位也认（老提示词里有 `At 00:0.5`），和以前的解析器一样宽。
pub fn parse_clock(s: &str) -> Option<(f32, usize)> {
    let b = s.as_bytes();
    if b.len() < 4 || !b[0].is_ascii_digit() || !b[1].is_ascii_digit() || b[2] != b':' || !b[3].is_ascii_digit() {
        return None;
    }
    let mut len = 3 + b[3..].iter().take_while(|c| c.is_ascii_digit()).count();
    if b.get(len) == Some(&b'.') && b.get(len + 1).is_some_and(|c| c.is_ascii_digit()) {
        len = len + 1 + b[len + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
    }
    let min: f32 = s[..2].parse().ok()?;
    let sec: f32 = s[3..len].parse().ok()?;
    Some((min * 60.0 + sec, len))
}

fn is_cut_phrase(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    ["hard cut", "cut to", "the camera cuts", "the shot cuts", "smash cut"].iter().any(|p| l.starts_with(p))
}

/// 去掉 `hard cut to` 之类的开头（后面接的是新镜头的内容）
fn strip_cut_phrase(s: &str) -> &str {
    let l = s.to_ascii_lowercase();
    for p in ["hard cut to ", "smash cut to ", "the camera cuts to ", "the shot cuts to ", "cut to "] {
        if l.starts_with(p) {
            return &s[p.len()..];
        }
    }
    s
}

/// 画面文字的「…」换成英文双引号（官方 4.5：`A red neon sign reading "营业中"`）。写手照旧写哪种都行。
/// **`<d>` 里的不动**：台词逐字照原文（官方 4.4），「玩具」这种台词里的引号是台词的一部分。
pub fn screen_quotes(text: &str) -> String {
    let swap = |t: &str| t.replace(['\u{300c}', '\u{300d}'], "\"");
    let mut out = String::new();
    let mut rest = text;
    while let Some(open) = rest.find("<d>") {
        let Some(close) = rest[open..].find("</d>") else { break };
        out.push_str(&swap(&rest[..open]));
        out.push_str(&rest[open..open + close + 4]);
        rest = &rest[open + close + 4..];
    }
    out.push_str(&swap(rest));
    out
}

/// **画布节点的 H3 提示词只整理格式**（2026-10-06，用户：「只做格式部分就行，正文……是用户自行调整的行为」）。
///
/// 画布的提示词是用户在节点里写的自由文本，以前原样发给 H3（WT 不加系统提示词、不查格式）。这里按出片方式套上官方结构：
///
/// - `t2v`：三字段（T2VA）；`i2v`：I2VA 对齐指令 + 三字段；`fl2v`：FL2VA 对齐指令 + 三字段；
/// - 镜内 `At 00:SS.S` → `Around the N.N-second mark`，`[Shot 1]` 去掉时间码，「」→ 英文双引号（和工作台同一套函数）；
/// - **正文一个字不改**（中文就还是中文）；`overall_soundscape` 写一句通用的「声音跟画面走」——
///   官方说只有要求全程无声才写 `N/A`；正文里提到配乐 / 音乐的，配乐那段指回正文，否则 `N/A`；
/// - `r2v`（参考生视频）要六段，主体定义代码编不出来，只做时间和引号的规整；
/// - 已经写成官方结构的（有 `integrated_multimodal_description:` / `subject_definitions:`）不再套一层，只补缺的对齐指令。
pub fn canvas_official(prompt: &str, mode: &str, secs: f32) -> String {
    let raw = prompt.trim();
    let structured = raw.contains("integrated_multimodal_description:") || raw.contains("subject_definitions:");
    let base = matches!(mode, "t2v" | "i2v" | "fl2v");
    // 套结构前先把 [Shot 1] 补上，`official_timing` 才认得出开头那个 `At 00:00.0` 是第一镜的时间码
    let seeded = if base && !structured && !raw.contains("[Shot 1]") { format!("[Shot 1] {raw}") } else { raw.to_string() };
    let body = screen_quotes(&official_timing(&seeded, true));
    if !base {
        return body;
    }
    let core = if structured {
        body
    } else {
        let lower = body.to_lowercase();
        let music = ["配乐", "背景音乐", "bgm", "music", "score"].iter().any(|k| lower.contains(k));
        format!(
            "integrated_multimodal_description: {body}\n\noverall_soundscape: Natural ambient and action sounds that match what happens on screen.\n\nnon_diegetic_music: {}",
            if music { "As described in the description above." } else { "N/A" }
        )
    };
    let head = match mode {
        "i2v" => Some(I2VA_INSTRUCTION.to_string()),
        "fl2v" => Some(fl2va_instruction(last_shot_number(&core), secs.max(1.0))),
        _ => None,
    };
    match head {
        Some(h) if !core.starts_with(&h) && !core.starts_with("How the reference pictures align") && !core.starts_with("For the target video") => format!("{h}\n\n{core}"),
        _ => core,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 原文真的嵌进来了，而且是官方那两份（不是我们的中文摘要）
    #[test]
    fn 官方原文嵌入且各模式取对章节() {
        assert!(BASE_GUIDE.starts_with("# Video Prompt Writing Guide (T2VA / I2VA / FL2VA / L2VA)"));
        assert!(REF_GUIDE.starts_with("# Full-Reference Mode Rewrite Output Format Guide"));
        assert!(BASE_GUIDE.contains(I2VA_INSTRUCTION), "I2VA 对齐指令要和官方原文逐字一致");
        assert!(BASE_GUIDE.contains("How the reference pictures align with the target video — Picture 1 (from Shot 1) aligns with the 0.00-second mark of the target video; Picture 2 (from Shot N)"));
        assert!(fl2va_instruction(1, 8.0).ends_with("Picture 2 (from Shot 1) aligns with the 8.00-second mark of the target video."));

        let i2va = writer_rules(Mode::I2va);
        assert!(i2va.contains("### 3.1 I2VA") && !i2va.contains("### 3.2 FL2VA") && !i2va.contains("### 3.3 L2VA"));
        assert!(i2va.contains("### Case 2: I2VA") && !i2va.contains("### Case 1: T2VA") && !i2va.contains("### Case 3"));
        assert!(i2va.contains("### 4.4 Speakers, Dialogue, and Singing"), "共用的写法章节要在");
        let fl = writer_rules(Mode::Fl2va);
        assert!(fl.contains("### 3.2 FL2VA") && !fl.contains("### 3.1 I2VA") && fl.contains("### Case 3: FL2VA"));
        let t2 = writer_rules(Mode::T2va);
        assert!(!t2.contains("### 3.1") && t2.contains("### Case 1: T2VA") && !t2.contains("### Case 2"));
        assert!(writer_rules(Mode::Ref2va).contains("## 4. `retention_analysis`"));
    }

    /// 画布：只整理格式、正文不动（中文照旧）；按出片方式套官方结构
    #[test]
    fn 画布提示词按出片方式套官方格式() {
        let p = "At 00:00.0, 老师走进教室。At 00:03.5, 他敲黑板 <d>[Chinese] 安静！</d> 黑板上写着「期中考试」。";
        let t = canvas_official(p, "t2v", 6.0);
        assert!(t.starts_with("integrated_multimodal_description: [Shot 1] 老师走进教室。Around the 3.5-second mark, 他敲黑板 <d>[Chinese] 安静！</d> 黑板上写着\"期中考试\"。"), "{t}");
        assert!(t.ends_with("\n\nnon_diegetic_music: N/A"), "{t}");
        assert!(t.contains("\n\noverall_soundscape: Natural ambient"), "不能写 N/A —— 官方说那是要求全程无声：{t}");
        let i = canvas_official(p, "i2v", 6.0);
        assert!(i.starts_with(&format!("{I2VA_INSTRUCTION}\n\nintegrated_multimodal_description: [Shot 1] ")), "{i}");
        assert!(canvas_official(p, "fl2v", 6.0).starts_with(&fl2va_instruction(1, 6.0)));
        // 提到配乐的，配乐那段不能写 N/A（会把它压掉）
        assert!(canvas_official("一段带背景音乐的蒙太奇", "t2v", 5.0).ends_with("non_diegetic_music: As described in the description above."));
        // 已经是官方结构的不再套一层，只补缺的对齐指令
        let done = "integrated_multimodal_description: [Shot 1] x.\n\noverall_soundscape: rain.\n\nnon_diegetic_music: N/A";
        assert_eq!(canvas_official(done, "t2v", 5.0), done);
        assert_eq!(canvas_official(done, "i2v", 5.0), format!("{I2VA_INSTRUCTION}\n\n{done}"));
        assert_eq!(canvas_official(&format!("{I2VA_INSTRUCTION}\n\n{done}"), "i2v", 5.0), format!("{I2VA_INSTRUCTION}\n\n{done}"), "不重复加");
        // 参考生视频：六段编不出来，只规整时间和引号
        let r = canvas_official("<Picture 1> 里的女孩转身。At 00:02.0, 她笑了。", "r2v", 5.0);
        assert_eq!(r, "<Picture 1> 里的女孩转身。Around the 2.0-second mark, 她笑了。");
    }

    #[test]
    fn 时间写法() {
        assert_eq!(cut_timecode(3.5), "00:03.500");
        assert_eq!(cut_timecode(65.25), "01:05.250");
        assert_eq!(beat_marker(4.0), "Around the 4.0-second mark,");
        assert!(!beat_marker(4.0).contains("At "), "镜内时刻不能长得像切镜");
    }
}
