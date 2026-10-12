//! 共享音色库：零样本克隆的参考音频 + 那段音频的文字。
//!
//! ## 为什么要有这一层
//!
//! F5-TTS 和 GPT-SoVITS 都靠"参考音频 + 转写稿"做零样本克隆，但它们各自的
//! 配置方式完全不同：F5-TTS 是服务端按名字去目录里找，GPT-SoVITS 是请求里带
//! 一个服务端可读的**绝对路径**加那段话的文字。
//!
//! 结果是同一个音色要在两处各配一遍，而且 GPT-SoVITS 的两个字段（路径与文字）
//! 必须手工保持一致 —— 写错的表现是"音色像但吐字乱"，极难归因。
//!
//! 这一层把"音色名 → (音频路径, 转写稿)"的解析集中起来，两个引擎都从同一个库读。
//! 用户在设置里选一个音色名，两边都能用。
//!
//! ## 目录约定
//!
//! ```text
//! <应用数据目录>/voices/
//!   <人名>/
//!     default.wav + default.txt     ← 参考（少数几条，命名过）
//!     b.wav       + b.txt
//!     library/                       ← 原始素材库，**不参与解析**
//! ```
//!
//! 音色名可以是 `yaemiko`（用 default）或 `yaemiko/b`（指定变体）。
//!
//! **`library/` 必须排除**：素材库动辄上千条，把它们都当音色会让下拉框没法用。
//! 那个目录是 `tools/voices/organize.py` 的输入。

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// 参考音频认的扩展名
const AUDIO_SUFFIXES: &[&str] = &["wav", "mp3", "m4a", "flac"];

/// 转写稿认的扩展名。
///
/// **`.lab` 必须支持**：语音素材包普遍用它存转写稿，每个音频配一个同名 lab。
/// 只认 `.txt` 的话每个音色都要手工改扩展名。
/// 转写稿认的扩展名。
///
/// **`.rtf` 也要认。** macOS 的 Finder / TextEdit 默认产出的是 RTF（就是那个
/// "MacOS 文稿"），普通用户很难顺手建一个纯 `.txt` —— 用户实际卡在这里过。
/// 读进来剥掉 RTF 控制字即可，比让人去 TextEdit 里点"制成纯文本"友好。
const TEXT_SUFFIXES: &[&str] = &["txt", "lab", "rtf"];

/// RTF 文件的开头。用它判断要不要剥标记，而不是只看扩展名 ——
/// 有人会把 RTF 内容存成 .txt
const RTF_MAGIC: &str = "{\\rtf";

/// 一个可用的参考音色。
#[derive(Debug, Clone, PartialEq)]
pub struct VoiceRef {
    /// 音色名，形如 `yaemiko` 或 `yaemiko/b`
    pub name: String,
    /// 参考音频的绝对路径。GPT-SoVITS 要的就是这个
    pub audio: PathBuf,
    /// 那段音频的文字。**模型靠它把参考对齐到音素**，写错会明显掉相似度
    pub text: String,
    /// 同一个人、**同一前缀组**里的其它参考音频（不含 `audio` 自己）。
    ///
    /// GPT-SoVITS 的 `aux_ref_audio_paths` 拿它们做音色融合。零样本克隆只看一条
    /// 参考时，音色稳定性受这一条的录音条件支配；多给几条同一个人的，
    /// 模型能把共性挑出来。
    ///
    /// 分组规则见 [`group_of`]。上限见 [`MAX_FUSED`]。
    pub aux: Vec<PathBuf>,
}

/// 一个前缀组最多融合几条参考（含主参考）。
///
/// 不是越多越好：每条辅助参考都要过一遍 HuBERT，首包时间和显存都跟着涨，
/// 而收益在第三条之后就很小了。
pub const MAX_FUSED: usize = 3;

/// 变体名 → 前缀组。
///
/// 规则：按**最后一个 `-`** 切开，前半段是组名；没有 `-` 就属于空组。
///
///   soft-a / soft-b       → 组 "soft"
///   excited-1 / excited-2 → 组 "excited"
///   default / b / c / d   → 组 ""（沿用既有命名的素材自然成一组）
///
/// 为什么要分组：融合的前提是这些片段**音色一致**。同一个人的"温柔"和"激动"
/// 混在一起会互相抵消，平均出一个没性格的音色 —— 那比只用一条更糟。
/// 想让某一条不参与融合，给它一个独有前缀即可（例如 `warm-greeting`）。
pub fn group_of(variant: &str) -> &str {
    match variant.rfind('-') {
        Some(i) if i > 0 && i + 1 < variant.len() => &variant[..i],
        _ => "",
    }
}

/// 共享音色库的位置。
///
/// 按顺序解析：
///
/// 1. `ANIMA_VOICES` 环境变量 —— `anima.sh` 会把它指向项目里的 `voice-library/`
/// 2. 从当前目录往上找 `voice-library/`（直接 `cargo run` / 跑测试时也能命中）
/// 3. 退回 `<应用数据目录>/voices`（打包成 .app 之后没有项目目录）
///
/// **放在项目里是用户的选择**：素材就在手边、能跟项目一起浏览和备份。
/// 代价是必须严格 gitignore（`/voice-library/*` 只放行 README）——
/// 语音包动辄 1~2GB，一旦被 commit 就很难从 git 历史里清掉。
pub fn voices_dir(config_dir: &Path) -> PathBuf {
    if let Some(explicit) = std::env::var_os("ANIMA_VOICES") {
        let path = PathBuf::from(explicit);
        if !path.as_os_str().is_empty() {
            return path;
        }
    }
    if let Some(found) = find_project_library(config_dir) {
        return found;
    }
    config_dir.join("voices")
}

/// 从当前工作目录往上找 `voice-library/`。
///
/// 只往上找几层：再深就说明根本不在项目里跑，硬找下去只会误命中别人的目录。
fn find_project_library(config_dir: &Path) -> Option<PathBuf> {
    crate::app_root::find_plain("voice-library", Some(config_dir))
}

/// 去掉游戏语音包转写稿里的模板标记。
///
/// 实测撞到过 `#早上好，{NICKNAME}。看你的气色…`：`#` 是"这句带占位符"的标记，
/// `{NICKNAME}` 在实际语音里是玩家名或干脆是停顿 —— **那些字并没有被念出来**。
/// 当成参考文本会让模型按错误的音素去对齐参考音频。
///
/// 原则是**只删能确定没被念出来的**（标记、占位符、富文本标签），其余原样保留。
pub fn strip_markup(text: &str) -> String {
    let mut out = text.trim().to_string();
    // 行首的 # 标记
    if let Some(rest) = out.strip_prefix('#') {
        out = rest.trim_start().to_string();
    }
    // 成对括号里的内容：{NICKNAME}、<color=...>、[提示]
    for (open, close) in [('{', '}'), ('<', '>'), ('[', ']')] {
        out = strip_pairs(&out, open, close);
    }
    // 删掉占位符后可能留下连续空白
    let collapsed: String = out.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.trim().to_string()
}

/// 删掉所有 `open…close` 包起来的片段（不嵌套，长度设上限防止吃掉正文）。
fn strip_pairs(text: &str, open: char, close: char) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(open) {
        let after = &rest[start + open.len_utf8()..];
        match after.find(close) {
            // 上限 40 个字符：正文里出现的孤立括号不该把后面一大段吃掉
            Some(end) if after[..end].chars().count() <= 40 => {
                out.push_str(&rest[..start]);
                rest = &after[end + close.len_utf8()..];
            }
            _ => break,
        }
    }
    out.push_str(rest);
    out
}

/// 把 RTF 剥成纯文本。
///
/// 只做到"够用"：RTF 是个复杂格式，但参考文本只是一句话，用不着完整解析器。
/// 处理三件事 —— `\'e4` 这种十六进制转义、`\uNNNN?` 这种 Unicode 转义、
/// 以及其余控制字与花括号。
///
/// macOS 的 TextEdit 存中文时用 `\'xx\'xx` 的字节对（按文档编码），
/// 而更常见的是 `\uNNNN` —— 后者能无损还原，前者只能尽力而为。
pub fn strip_rtf(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() / 2);
    let mut chars = raw.chars().peekable();
    // 当前花括号深度，以及"从哪个深度起整组丢掉"。
    //
    // RTF 有一类**目标组**（`{\fonttbl…}`、`{\colortbl…}`、`{\*\…}`），里面是
    // 元数据不是正文。只去掉花括号的话字体名会被当成句子的一部分 ——
    // 实测漏过一次："PingFangSC-Regular;早上好，旅行者。"
    let mut depth: i32 = 0;
    let mut skip_from: Option<i32> = None;
    while let Some(c) = chars.next() {
        // 处在被丢弃的组里：只跟踪深度，内容一律不要
        if let Some(from) = skip_from {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth < from {
                        skip_from = None;
                    }
                }
                // 组里的转义要跳过下一个字符，否则 \} 会被当成组结束
                '\\' => {
                    chars.next();
                }
                _ => {}
            }
            continue;
        }
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            '\\' => {
                // 控制字：\word 后面可能跟数字参数，再可能跟一个空格（被吃掉）
                let mut word = String::new();
                while let Some(&n) = chars.peek() {
                    if n.is_ascii_alphabetic() {
                        word.push(n);
                        chars.next();
                    } else {
                        break;
                    }
                }
                if word.is_empty() {
                    // \'e4 这种十六进制字节，或者 \\ \{ \} 这种转义
                    match chars.peek() {
                        Some('\'') => {
                            chars.next();
                            let hex: String = chars.by_ref().take(2).collect();
                            if let Ok(b) = u8::from_str_radix(&hex, 16) {
                                // 单字节转义没有编码信息，只能按 latin1 放进去 ——
                                // 中文会变成乱码，所以下面的 \u 才是首选路径
                                out.push(b as char);
                            }
                        }
                        Some(&esc @ ('\\' | '{' | '}')) => {
                            chars.next();
                            out.push(esc);
                        }
                        // `\*` 是 RTF 的约定：这个组不认识就整组忽略
                        Some('*') => {
                            chars.next();
                            skip_from = Some(depth);
                        }
                        _ => {}
                    }
                    continue;
                }
                // 数字参数
                let mut num = String::new();
                if chars.peek() == Some(&'-') {
                    num.push('-');
                    chars.next();
                }
                while let Some(&n) = chars.peek() {
                    if n.is_ascii_digit() {
                        num.push(n);
                        chars.next();
                    } else {
                        break;
                    }
                }
                // 控制字后紧跟的一个空格是分隔符，不是内容
                if chars.peek() == Some(&' ') {
                    chars.next();
                }
                match word.as_str() {
                    // 目标组：从当前深度起整组丢掉
                    "fonttbl" | "colortbl" | "stylesheet" | "info" | "pict" | "listtable"
                    | "generator" | "expandedcolortbl" => {
                        skip_from = Some(depth);
                    }
                    // \uNNNN 是真正的 Unicode 码点，后面跟一个"替代字符"要跳过
                    "u" => {
                        if let Ok(n) = num.parse::<i32>() {
                            // RTF 里大于 32767 的码点写成负数
                            let cp = if n < 0 { (n + 65536) as u32 } else { n as u32 };
                            if let Some(ch) = char::from_u32(cp) {
                                out.push(ch);
                            }
                        }
                        // 紧跟的替代字符（通常是 '?'）跳过
                        if chars.peek() == Some(&'?') {
                            chars.next();
                        }
                    }
                    "par" | "line" | "tab" => out.push(' '),
                    _ => {}
                }
            }
            '\n' | '\r' => {}
            other => out.push(other),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 读一条参考的转写稿。挡住 HTK 风格的音素对齐标注。
///
/// HTK 标注逐行是"起时间 止时间 音素"，那不是句子 —— 拿它当参考文本会让克隆
/// 完全跑偏，而症状（音色像但吐字乱）极难归因。宁可拒收。
fn read_text(path: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    let raw = if raw.trim_start().starts_with(RTF_MAGIC) {
        strip_rtf(&raw)
    } else {
        raw
    };
    let lines: Vec<&str> = raw
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return None;
    }
    if lines.len() > 2 {
        let aligned = lines
            .iter()
            .filter(|l| {
                let mut parts = l.split_whitespace();
                matches!((parts.next(), parts.next(), parts.next()),
                    (Some(a), Some(b), Some(_))
                        if a.chars().all(|c| c.is_ascii_digit())
                            && b.chars().all(|c| c.is_ascii_digit()))
            })
            .count();
        if aligned * 5 > lines.len() * 4 {
            tracing::warn!(
                path = %path.display(),
                "这是 HTK 音素对齐标注而不是转写稿，跳过"
            );
            return None;
        }
    }
    let joined = lines.join(" ");
    let cleaned = strip_markup(&joined);
    if cleaned != joined.trim() {
        tracing::debug!(before = %joined, after = %cleaned, "清掉了转写稿里的模板标记");
    }
    (!cleaned.is_empty()).then_some(cleaned)
}

fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_SUFFIXES.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// 给一个音频文件找它的转写稿。
fn text_for(audio: &Path) -> Option<String> {
    TEXT_SUFFIXES
        .iter()
        .find_map(|suffix| read_text(&audio.with_extension(suffix)))
}

/// 列出所有可用的参考音色（按解析出来的库位置）。
pub fn list(config_dir: &Path) -> Vec<VoiceRef> {
    list_in(&voices_dir(config_dir))
}

/// 列出**指定目录**下的参考音色。
///
/// 和 [`list`] 分开是因为 [`voices_dir`] 会被环境变量和项目目录影响 ——
/// 让这个函数吃一个显式的根目录，签名才不会说谎，测试也才隔离得开
/// （否则测试用的临时目录会被真实的 `voice-library/` 顶掉）。
///
/// 顶层的单个音频算一个音色；子目录算一个"人"，里面顶层的每个音频是一个语气变体，
/// 并且额外注册一个不带变体的名字指向 `default`（没有 default 就用排第一个的）。
///
/// **`library/` 天然被排除**：这里只看每个人目录的顶层文件，不递归。
pub fn list_in(root: &Path) -> Vec<VoiceRef> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return out;
    };
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut files: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        } else if is_audio(&path) {
            files.push(path);
        }
    }
    files.sort();
    dirs.sort();

    for audio in files {
        if let (Some(stem), Some(text)) =
            (audio.file_stem().and_then(|s| s.to_str()), text_for(&audio))
        {
            // 顶层散装文件不属于任何人，没有可融合的同伴
            out.push(VoiceRef {
                name: stem.to_string(),
                audio,
                text,
                aux: Vec::new(),
            });
        }
    }

    for person in dirs {
        let Some(who) = person.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let Ok(inner) = std::fs::read_dir(&person) else {
            continue;
        };
        // **只看顶层**，library/ 里的素材不算音色
        let mut variants: Vec<PathBuf> = inner
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && is_audio(p))
            .collect();
        variants.sort();

        let mut added: Vec<VoiceRef> = Vec::new();
        for audio in variants {
            let Some(stem) = audio.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if let Some(text) = text_for(&audio) {
                added.push(VoiceRef {
                    name: format!("{who}/{stem}"),
                    audio,
                    text,
                    // 同组同伴要等这个人的变体全收完才知道，下面统一填
                    aux: Vec::new(),
                });
            }
        }
        if added.is_empty() {
            continue;
        }

        // 填同组同伴。`variants` 已排序，所以这里的顺序是确定的 ——
        // 同一份素材每次给出同样的融合组合，音色才可复现
        let members: Vec<(String, PathBuf)> = added
            .iter()
            .map(|v| {
                let stem = v.name.rsplit('/').next().unwrap_or("").to_string();
                (group_of(&stem).to_string(), v.audio.clone())
            })
            .collect();
        for (i, v) in added.iter_mut().enumerate() {
            let my_group = &members[i].0;
            v.aux = members
                .iter()
                .enumerate()
                .filter(|(j, (g, _))| *j != i && g == my_group)
                .map(|(_, (_, p))| p.clone())
                .take(MAX_FUSED - 1)
                .collect();
        }
        // 不带变体的名字 → default，没有就用第一个
        let default = added
            .iter()
            .find(|v| v.name.ends_with("/default"))
            .unwrap_or(&added[0])
            .clone();
        out.push(VoiceRef {
            name: who.to_string(),
            audio: default.audio,
            text: default.text,
            aux: default.aux,
        });
        out.extend(added);
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// 按名字解析一个音色。
pub fn resolve(config_dir: &Path, name: &str) -> Result<VoiceRef> {
    resolve_in(&voices_dir(config_dir), name)
}

/// 在**指定目录**里按名字解析。理由同 [`list_in`]。
pub fn resolve_in(root: &Path, name: &str) -> Result<VoiceRef> {
    let all = list_in(root);
    if let Some(found) = all.iter().find(|v| v.name == name) {
        return Ok(found.clone());
    }
    if all.is_empty() {
        return Err(Error::config(format!(
            "共享音色库是空的（{}）。往 <人名>/ 里丢音频和同名 .lab/.txt，\
             再跑 tools/voices/organize.py",
            root.display()
        )));
    }
    Err(Error::config(format!(
        "音色库里没有「{name}」。现有：{}",
        all.iter()
            .map(|v| v.name.as_str())
            .collect::<Vec<_>>()
            .join("、")
    )))
}

/// 忽略 `library/` 这一点必须成立，否则素材库会把音色列表淹掉。
#[cfg(test)]
mod fusion_tests {
    use super::*;

    #[test]
    fn prefix_group_splits_on_the_last_dash() {
        assert_eq!(group_of("soft-a"), "soft");
        assert_eq!(group_of("soft-b"), "soft");
        assert_eq!(group_of("excited-1"), "excited");
        // 多个连字符：按最后一个切，所以 "taiwan-soft" 这一层还能再分
        assert_eq!(group_of("taiwan-soft-a"), "taiwan-soft");
        // 沿用既有命名的素材自然成一组
        assert_eq!(group_of("default"), "");
        assert_eq!(group_of("b"), "");
        // 退化形状不该 panic，也不该切出空前缀组以外的东西
        assert_eq!(group_of("-a"), "");
        assert_eq!(group_of("a-"), "");
        assert_eq!(group_of(""), "");
    }

    /// 建一个临时音色库：两组情绪各三条，外加一条独有前缀的
    fn library() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let who = dir.path().join("someone");
        std::fs::create_dir_all(&who).unwrap();
        for stem in [
            "soft-a",
            "soft-b",
            "soft-c",
            "soft-d",
            "excited-a",
            "excited-b",
            "warm-greeting",
        ] {
            std::fs::write(who.join(format!("{stem}.wav")), b"RIFF").unwrap();
            std::fs::write(who.join(format!("{stem}.txt")), "一句话").unwrap();
        }
        dir
    }

    #[test]
    fn only_same_prefix_is_fused() {
        let dir = library();
        let list = list_in(dir.path());
        let soft = list.iter().find(|v| v.name == "someone/soft-a").unwrap();
        for p in &soft.aux {
            let stem = p.file_stem().unwrap().to_str().unwrap();
            assert!(
                stem.starts_with("soft-"),
                "融合进来了别组的片段：{stem} —— 温柔和激动混在一起会互相抵消"
            );
        }
    }

    #[test]
    fn fusion_is_capped() {
        let dir = library();
        let list = list_in(dir.path());
        // soft 组有四条，主参考 + 辅助合起来不能超过 MAX_FUSED
        let soft = list.iter().find(|v| v.name == "someone/soft-a").unwrap();
        assert_eq!(soft.aux.len(), MAX_FUSED - 1, "上限没生效：{:?}", soft.aux);
        assert!(
            !soft.aux.contains(&soft.audio),
            "主参考不该出现在辅助列表里，那等于把同一条喂两遍"
        );
    }

    #[test]
    fn a_lone_prefix_fuses_with_nobody() {
        let dir = library();
        let list = list_in(dir.path());
        let lone = list
            .iter()
            .find(|v| v.name == "someone/warm-greeting")
            .unwrap();
        assert!(
            lone.aux.is_empty(),
            "独有前缀是「不要融合我」的表达方式，必须尊重：{:?}",
            lone.aux
        );
    }

    /// 裸名字（`someone`）拿的是 default/第一条，它的融合列表也要跟着带上 ——
    /// 否则"设置里选 someone"和"选 someone/soft-a"的音色会不一样
    #[test]
    fn bare_name_carries_the_same_fusion_set() {
        let dir = library();
        let list = list_in(dir.path());
        let bare = list.iter().find(|v| v.name == "someone").unwrap();
        let first = list
            .iter()
            .find(|v| v.name.starts_with("someone/") && v.audio == bare.audio)
            .unwrap();
        assert_eq!(bare.aux, first.aux);
    }

    /// 顺序必须确定 —— 同一份素材每次给出同样的融合组合，音色才可复现
    #[test]
    fn fusion_order_is_deterministic() {
        let dir = library();
        let a = list_in(dir.path());
        let b = list_in(dir.path());
        let pick = |l: &Vec<VoiceRef>| {
            l.iter()
                .find(|v| v.name == "someone/soft-a")
                .unwrap()
                .aux
                .clone()
        };
        assert_eq!(pick(&a), pick(&b));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// 造一个音色库。音频内容随便，解析只看文件是否存在
    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("voices");
        // 顶层单条
        write(&root.join("solo.wav"), "x");
        write(&root.join("solo.txt"), "我是单独的一条");
        // 一个人 + 变体
        write(&root.join("yaemiko/default.wav"), "x");
        write(&root.join("yaemiko/default.txt"), "默认那一条");
        write(&root.join("yaemiko/teasing.wav"), "x");
        write(&root.join("yaemiko/teasing.lab"), "调笑那一条");
        // 素材库：一千条的效果用几条代表
        for i in 0..5 {
            write(&root.join(format!("yaemiko/library/vo_{i}.wav")), "x");
            write(&root.join(format!("yaemiko/library/vo_{i}.lab")), "素材");
        }
        dir
    }

    #[test]
    fn library_directory_is_never_listed_as_a_voice() {
        // 素材库动辄上千条。把它们当音色会让设置面板的下拉框彻底没法用 ——
        // 实测撞到过：用户拷进 1291 条，列表里就冒出 1291 项
        let dir = fixture();
        let names: Vec<String> = list_in(&dir.path().join("voices"))
            .into_iter()
            .map(|v| v.name)
            .collect();
        assert!(
            !names
                .iter()
                .any(|n| n.contains("library") || n.contains("vo_")),
            "素材库不该出现在音色列表里：{names:?}"
        );
    }

    #[test]
    fn a_person_gets_both_a_bare_name_and_named_variants() {
        let dir = fixture();
        let names: Vec<String> = list_in(&dir.path().join("voices"))
            .into_iter()
            .map(|v| v.name)
            .collect();
        assert!(names.contains(&"yaemiko".to_string()), "{names:?}");
        assert!(names.contains(&"yaemiko/default".to_string()), "{names:?}");
        assert!(names.contains(&"yaemiko/teasing".to_string()), "{names:?}");
        assert!(names.contains(&"solo".to_string()), "{names:?}");
    }

    #[test]
    fn the_bare_name_points_at_the_default_variant() {
        let dir = fixture();
        let all = list_in(&dir.path().join("voices"));
        let bare = all.iter().find(|v| v.name == "yaemiko").unwrap();
        let default = all.iter().find(|v| v.name == "yaemiko/default").unwrap();
        assert_eq!(bare.audio, default.audio);
        assert_eq!(bare.text, "默认那一条");
    }

    #[test]
    fn lab_files_work_as_transcripts() {
        // 语音素材包普遍用 .lab 存转写稿。只认 .txt 的话每个音色都要手工改扩展名
        let dir = fixture();
        let all = list_in(&dir.path().join("voices"));
        let teasing = all.iter().find(|v| v.name == "yaemiko/teasing").unwrap();
        assert_eq!(teasing.text, "调笑那一条");
    }

    #[test]
    fn htk_alignment_labels_are_rejected_instead_of_used() {
        // 同名的另一种 .lab 是 HTK 风格的"起 止 音素"对齐标注。
        // 拿它当参考文本会让克隆跑偏，而症状（音色像但吐字乱）极难归因
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("voices");
        write(&root.join("bad.wav"), "x");
        write(
            &root.join("bad.lab"),
            "0 1250000 sil\n1250000 2100000 zh\n2100000 3000000 ai\n3000000 4000000 sil",
        );
        assert!(list_in(&root).is_empty(), "HTK 对齐标注不该被当成转写稿");
    }

    #[test]
    fn game_template_markup_is_stripped() {
        // `#` 前缀和 {NICKNAME} 占位符**并没有被念出来**，留着会让对齐偏掉
        assert_eq!(
            strip_markup("#早上好，{NICKNAME}。看你的气色，昨晚应该做了个好梦吧？"),
            "早上好，。看你的气色，昨晚应该做了个好梦吧？"
        );
        assert_eq!(
            strip_markup("<color=#FFD780>稻妻</color>的樱花"),
            "稻妻的樱花"
        );
        assert_eq!(strip_markup("[提示]往左走"), "往左走");
        // 正文里的孤立括号不该把后面一大段吃掉
        assert_eq!(
            strip_markup("这是 { 一个没闭合的括号"),
            "这是 { 一个没闭合的括号"
        );
        // 没有标记时原样保留
        assert_eq!(strip_markup("普通的一句话。"), "普通的一句话。");
    }

    #[test]
    fn resolving_a_missing_voice_says_what_is_available() {
        // 报错要能自己解决问题：只说"找不到"的话用户不知道该填什么
        let dir = fixture();
        let err = resolve_in(&dir.path().join("voices"), "不存在的音色")
            .unwrap_err()
            .to_string();
        assert!(err.contains("yaemiko"), "报错里应当列出现有音色：{err}");
    }

    #[test]
    fn an_empty_library_explains_how_to_fill_it() {
        let dir = tempfile::tempdir().unwrap();
        let err = resolve_in(&dir.path().join("voices"), "whatever")
            .unwrap_err()
            .to_string();
        assert!(err.contains("organize"), "空库的报错要给出下一步：{err}");
    }

    #[test]
    fn rtf_files_are_stripped_to_plain_text() {
        // macOS 的 Finder / TextEdit 默认产出 RTF（"MacOS 文稿"），普通用户很难
        // 顺手建一个纯 .txt —— 用户实际卡在这里过。与其让人去点"制成纯文本"，
        // 不如读进来剥掉标记。
        //
        // 夹具照 TextEdit 的真实产出写：有 fonttbl 目标组，中文走 \uNNNN。
        // 「早上好，旅行者。」的码点：26089 19978 22909 65292 26053 34892 32773 12290
        let rtf = concat!(
            r"{\rtf1\ansi\ansicpg936\cocoartf2761",
            "\n",
            r"{\fonttbl\f0\fnil\fcharset134 PingFangSC-Regular;}",
            "\n",
            r"{\colortbl;\red255\green255\blue255;}",
            "\n",
            r"\f0\fs24 \cf0 ",
            r"\u26089?\u19978?\u22909?\u65292?\u26053?\u34892?\u32773?\u12290?",
            "}"
        );
        assert_eq!(strip_rtf(rtf), "早上好，旅行者。");
    }

    #[test]
    fn rtf_metadata_groups_do_not_leak_into_the_text() {
        // 只去花括号不够：字体名会被当成句子的一部分。
        // 实测漏过一次 —— "PingFangSC-Regular;早上好，旅行者。"
        let rtf = concat!(
            r"{\rtf1\ansi",
            r"{\fonttbl\f0\fnil\fcharset134 PingFangSC-Regular;}",
            r"{\*\expandedcolortbl;;}",
            r"\f0 \u20320?\u22909?",
            "}"
        );
        let out = strip_rtf(rtf);
        assert_eq!(out, "你好", "元数据泄漏了：{out}");
        assert!(!out.contains("PingFang"), "字体名不该出现：{out}");
    }

    #[test]
    fn an_rtf_reference_is_usable_as_a_voice() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("voices");
        write(&root.join("rtfvoice.wav"), "x");
        write(
            &root.join("rtfvoice.rtf"),
            r"{\rtf1\ansi \u20320?\u22909?\u65292?\u26053?\u34892?\u32773?}",
        );
        let all = list_in(&root);
        assert_eq!(all.len(), 1, "RTF 转写稿应当能用：{all:?}");
        assert!(
            !all[0].text.contains('\\'),
            "剥完不该还剩控制字：{}",
            all[0].text
        );
        assert!(
            !all[0].text.contains('{'),
            "剥完不该还剩花括号：{}",
            all[0].text
        );
    }

    #[test]
    fn rtf_content_saved_as_txt_is_also_handled() {
        // 有人会把 RTF 内容另存成 .txt。判据看内容而不是扩展名
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("voices");
        write(&root.join("mislabeled.wav"), "x");
        write(
            &root.join("mislabeled.txt"),
            r"{\rtf1\ansi \u20320?\u22909?\u65292?\u26053?\u34892?\u32773?}",
        );
        let all = list_in(&root);
        assert_eq!(all.len(), 1);
        assert!(!all[0].text.contains("rtf"), "实际：{}", all[0].text);
    }
}
