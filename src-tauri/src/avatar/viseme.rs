//! 把文字 + 词边界时间戳变成一条**嘴型时间线**。
//!
//! ## 为什么放在 Rust 侧而不是前端
//!
//! 两个理由。一是汉字→拼音是数据问题：一张两万字的表放前端等于往 bundle 里
//! 塞几百 KB，而 Rust 这边一个 crate 就解决。二是这套映射写错了**不会报错**，
//! 只会表现成「嘴在动但口型不对」—— 那是最难发现的一类问题，需要测试守着，
//! 而测试基建在 Rust 这边。
//!
//! ## 为什么不用音频分析
//!
//! 更常见的做法是分析播放中的波形推口型。这里不行：音频在 Rust 侧解码播放，
//! 而且真正的口型要靠**共振峰**（哪个元音）而不是音量 —— 只看音量只能得到
//! 「嘴一张一合」，所有元音长一个样。
//!
//! 而我们手上有更好的东西：文字本身。文字直接给出音素序列，比从波形反推准得多。
//! 音量仍然有用，但只用来调**幅度**（说得响嘴张得大），不决定形状。
//!
//! ## 口型集
//!
//! 用 Oculus Viseme 那 15 个（`sil PP FF TH DD kk CH SS nn RR aa E I O U`）。
//! 选它是因为 Ready Player Me 导出的模型自带这套 morph target，
//! 拿到模型就能直接驱动，不用自己做混合形状。
//!
//! ## 一个音节内部怎么分配时间
//!
//! 这一段是「像在说话」和「嘴在乱动」的分界线：
//!
//! - 声母短，元音长。声母只占音节前段（约三成，且不超过 80ms），
//!   剩下全给韵母 —— 现实里辅音就是一掠而过的。
//! - 复韵母（ai / ao / ou / ie / uo …）要**两个**目标口型依次到位。
//!   只给一个的话「好」和「喝」的嘴型没区别。
//! - 鼻韵尾（an / ang / eng / ong …）末尾要收成 nn，否则嘴一直张着不闭。

use crate::tts::WordBoundary;
use serde::Serialize;

/// Oculus Viseme 集。序列化成 morph target 的名字，前端拿到就能直接用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Viseme {
    #[serde(rename = "sil")]
    Sil,
    #[serde(rename = "PP")]
    Pp,
    #[serde(rename = "FF")]
    Ff,
    #[serde(rename = "TH")]
    Th,
    #[serde(rename = "DD")]
    Dd,
    #[serde(rename = "kk")]
    Kk,
    #[serde(rename = "CH")]
    Ch,
    #[serde(rename = "SS")]
    Ss,
    #[serde(rename = "nn")]
    Nn,
    #[serde(rename = "RR")]
    Rr,
    #[serde(rename = "aa")]
    Aa,
    #[serde(rename = "E")]
    E,
    #[serde(rename = "I")]
    I,
    #[serde(rename = "O")]
    O,
    #[serde(rename = "U")]
    U,
}

/// 时间线上的一个关键帧：`at_ms` 时刻该到 `viseme` 这个口型，强度 `weight`。
///
/// 前端在两个关键帧之间插值。给的是关键帧而不是逐帧采样：一句话几十个关键帧
/// 就够，逐帧（60fps）会把事件通道塞满 —— 而高频数据不该走事件广播
/// （ARCHITECTURE.md 坑点 1）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct VisemeKey {
    /// 相对这一句开始出声的偏移（毫秒）
    pub at_ms: u64,
    pub viseme: Viseme,
    /// 0..1。辅音给得比元音弱 —— 辅音的嘴型本来就不夸张
    pub weight: f32,
}

/// 声母最多占一个音节的多长（毫秒）。
///
/// 现实里辅音是一掠而过的。不设上限的话，慢速朗读时「妈」会变成
/// 嘴闭着大半拍再张开 —— 看着像卡住了。
const MAX_ONSET_MS: u64 = 80;
/// 声母最多占音节的比例
const ONSET_RATIO: f64 = 0.3;
/// 元音口型的强度
const VOWEL_WEIGHT: f32 = 1.0;
/// 辅音口型的强度。压低是因为辅音的嘴型幅度本来就小，
/// 给满会让整句看起来在用力过猛
const CONSONANT_WEIGHT: f32 = 0.75;
/// 韵尾（鼻音收口）的强度
const CODA_WEIGHT: f32 = 0.6;

/// 把一句话的词边界变成嘴型时间线。
///
/// 输入的 `words` 就是 TTS 给的那份（Edge 是真实时间戳，本地引擎是按字均分的
/// 估算值，见 `tts::estimate_word_boundaries`）。两种都能用 —— 均分的估算值
/// 对中文其实相当接近，因为汉字基本等时长。
pub fn schedule(words: &[WordBoundary]) -> Vec<VisemeKey> {
    let mut keys: Vec<VisemeKey> = Vec::new();

    for w in words {
        // 一个"词"可能是多个字（Edge 会按词给），按字均分它的时长。
        //
        // **均分的分母是全部字符，包括标点。** 只按发音的字来分会把标点占的
        // 时间并给前一个字 —— 那个字的元音就一直挂到停顿结束，
        // 表现是「话停了嘴还张着」。标点自己那一段要闭嘴，那才是停顿的样子。
        let chars: Vec<char> = w.text.chars().collect();
        if chars.is_empty() {
            continue;
        }
        let per = (w.duration_ms / chars.len() as u64).max(1);

        for (i, ch) in chars.iter().enumerate() {
            let start = w.offset_ms + per * i as u64;
            if is_silent(*ch) {
                keys.push(VisemeKey {
                    at_ms: start,
                    viseme: Viseme::Sil,
                    weight: 0.0,
                });
            } else {
                emit_char(*ch, start, per, &mut keys);
            }
        }
    }

    // 词与词之间的空档要闭嘴。
    //
    // 上一段的标点处理只管「标点在词内部」的情况，但引擎也可能把标点单独摘掉，
    // 只在词边界之间留一段空白（Edge 就是这样）。那种情况下前一个字的元音会
    // 一直挂到下一个词开始 —— 实测「你好，我是 Anima」在逗号处嘴保持张开
    // 近半秒。补一帧闭嘴才是停顿的样子。
    //
    // 阈值取 120ms：比这更短的空档是音节之间的正常过渡，闭一下反而会抖。
    const GAP_MS: u64 = 120;
    let mut gap_keys: Vec<VisemeKey> = Vec::new();
    for pair in words.windows(2) {
        let end = pair[0].offset_ms + pair[0].duration_ms;
        if pair[1].offset_ms.saturating_sub(end) >= GAP_MS {
            gap_keys.push(VisemeKey {
                at_ms: end,
                viseme: Viseme::Sil,
                weight: 0.0,
            });
        }
    }
    keys.extend(gap_keys);

    // 结尾闭嘴。没有这一帧的话最后一个元音会一直挂在脸上 ——
    // 说完之后嘴半张着不动，比嘴型不准更瘆人
    if let Some(last) = words.last() {
        keys.push(VisemeKey {
            at_ms: last.offset_ms + last.duration_ms,
            viseme: Viseme::Sil,
            weight: 0.0,
        });
    }

    // 时间戳可能因为多音字/估算而轻微乱序，前端的插值假设是有序的
    keys.sort_by_key(|k| k.at_ms);
    keys
}

/// 不出声的字符：标点、空白。它们占时间但没有口型
fn is_silent(c: char) -> bool {
    c.is_whitespace()
        || c.is_ascii_punctuation()
        || matches!(
            c,
            '。' | '，'
                | '、'
                | '；'
                | '：'
                | '？'
                | '！'
                | '「'
                | '」'
                | '（'
                | '）'
                | '《'
                | '》'
                | '…'
                | '—'
                | '·'
                | '\u{201c}'
                | '\u{201d}'
                | '\u{2018}'
                | '\u{2019}'
        )
}

/// 一个字 → 若干关键帧
fn emit_char(ch: char, start: u64, dur: u64, out: &mut Vec<VisemeKey>) {
    if let Some(syl) = hanzi_pinyin(ch) {
        emit_syllable(&syl, start, dur, out);
    } else if ch.is_ascii_alphabetic() {
        // 拉丁字母：没有音节结构可依，按字母的典型发音给一个口型。
        // 精度不如中文，但混在中文里的英文词（"OK"、"iPhone"）总比不动好
        out.push(VisemeKey {
            at_ms: start,
            viseme: latin_viseme(ch.to_ascii_lowercase()),
            weight: if "aeiou".contains(ch.to_ascii_lowercase()) {
                VOWEL_WEIGHT
            } else {
                CONSONANT_WEIGHT
            },
        });
    } else if ch.is_numeric() {
        // 数字读出来是汉字，但读法依赖上下文（"一"在"一个"里变调）。
        // 不去猜，给一个中性张口 —— 至少时长是对的
        out.push(VisemeKey {
            at_ms: start,
            viseme: Viseme::Aa,
            weight: 0.7,
        });
    }
}

/// 取一个汉字的拼音（不带声调）。多音字取第一个读音。
///
/// 多音字**故意不做上下文消歧**。口型只关心声母和韵母的形状，
/// 而多音字的不同读音里绝大多数韵母形状接近（「行」xing/hang 都是张口+鼻音收尾）。
/// 为这点差别引入分词和词典，收益远小于复杂度。
fn hanzi_pinyin(ch: char) -> Option<String> {
    use pinyin::ToPinyin;
    ch.to_pinyin().map(|p| p.plain().to_string())
}

/// 把一个拼音音节铺成关键帧
fn emit_syllable(syl: &str, start: u64, dur: u64, out: &mut Vec<VisemeKey>) {
    let (onset, rime) = split_syllable(syl);

    // 声母占前段，但有硬上限
    let onset_ms = if onset.is_empty() {
        0
    } else {
        ((dur as f64 * ONSET_RATIO) as u64).clamp(1, MAX_ONSET_MS)
    };

    if let Some(v) = onset_viseme(onset) {
        out.push(VisemeKey {
            at_ms: start,
            viseme: v,
            weight: CONSONANT_WEIGHT,
        });
    }

    let rime_start = start + onset_ms;
    let rime_ms = dur.saturating_sub(onset_ms).max(1);
    let (vowels, coda) = split_rime(rime);

    // 鼻韵尾留出末段收口
    let coda_ms = if coda.is_some() {
        (rime_ms / 3).max(1)
    } else {
        0
    };
    let vowel_span = rime_ms.saturating_sub(coda_ms).max(1);

    // 复韵母：两个（偶尔三个）口型依次到位，把元音段等分
    if vowels.is_empty() {
        // 只有辅音的音节（拼音里不存在，但防御一下）
        out.push(VisemeKey {
            at_ms: rime_start,
            viseme: Viseme::Aa,
            weight: 0.5,
        });
    } else {
        let step = vowel_span / vowels.len() as u64;
        for (i, v) in vowels.iter().enumerate() {
            out.push(VisemeKey {
                at_ms: rime_start + step * i as u64,
                viseme: *v,
                // 后续的过渡元音略弱：主元音才是这个字"看起来"的样子
                weight: if i == 0 { VOWEL_WEIGHT } else { 0.8 },
            });
        }
    }

    if let Some(c) = coda {
        out.push(VisemeKey {
            at_ms: rime_start + vowel_span,
            viseme: c,
            weight: CODA_WEIGHT,
        });
    }
}

/// 切出声母和韵母。
///
/// 按拼音的实际规则，不是"第一个字母就是声母"：zh/ch/sh 是三个双字母声母，
/// 而 a/o/e 开头的音节（啊、哦、恩）根本没有声母。
fn split_syllable(syl: &str) -> (&str, &str) {
    for two in ["zh", "ch", "sh"] {
        if let Some(rest) = syl.strip_prefix(two) {
            return (two, rest);
        }
    }
    let first = syl.chars().next().unwrap_or('a');
    if "aoe".contains(first) {
        return ("", syl); // 零声母
    }
    let idx = first.len_utf8();
    (&syl[..idx], &syl[idx..])
}

/// 声母 → 口型
fn onset_viseme(onset: &str) -> Option<Viseme> {
    Some(match onset {
        "" => return None,
        // 双唇音：嘴必须闭上。这三个是**最显眼**的口型，错了一眼就看出来
        "b" | "p" | "m" => Viseme::Pp,
        "f" => Viseme::Ff,
        "d" | "t" => Viseme::Dd,
        "n" | "l" => Viseme::Nn,
        "g" | "k" | "h" => Viseme::Kk,
        // j/q/x 和 zh/ch/sh 都是"扁嘴往前送"，归到 CH
        "j" | "q" | "x" | "zh" | "ch" | "sh" => Viseme::Ch,
        "z" | "c" | "s" => Viseme::Ss,
        "r" => Viseme::Rr,
        // y/w 是介音，本身就是元音口型
        "y" => Viseme::I,
        "w" => Viseme::U,
        _ => Viseme::Dd,
    })
}

/// 韵母 → （元音口型序列，鼻韵尾）
fn split_rime(rime: &str) -> (Vec<Viseme>, Option<Viseme>) {
    // 先摘鼻韵尾。顺序重要：ng 要在 n 之前试，否则 "ang" 会被切成 "a"+"n"+"g"
    let (core, coda) = if let Some(c) = rime.strip_suffix("ng") {
        (c, Some(Viseme::Nn))
    } else if let Some(c) = rime.strip_suffix('n') {
        (c, Some(Viseme::Nn))
    } else {
        (rime, None)
    };

    // 特例先查表：这些的口型不是逐字母拼出来的
    let vowels: Vec<Viseme> = match core {
        // ü（pinyin crate 的 plain 输出成 v）：撮口
        "v" | "u" => vec![Viseme::U],
        "ve" => vec![Viseme::U, Viseme::E], // üe：略→撮口到扁嘴
        "i" => vec![Viseme::I],
        "e" => vec![Viseme::E],
        "a" => vec![Viseme::Aa],
        "o" => vec![Viseme::O],
        // er / -i（zi ci si 的那个 i）实际是含混的中央元音，不是"衣"
        "er" => vec![Viseme::Rr],
        "" => vec![], // 纯鼻音音节，比如 "n"、"ng"
        _ => core.chars().filter_map(vowel_viseme).collect(),
    };

    // 相邻重复的口型合并 —— "iu" 两个都是圆唇的话不需要两帧
    let mut dedup: Vec<Viseme> = Vec::with_capacity(vowels.len());
    for v in vowels {
        if dedup.last() != Some(&v) {
            dedup.push(v);
        }
    }
    (dedup, coda)
}

/// 单个元音字母 → 口型
fn vowel_viseme(c: char) -> Option<Viseme> {
    Some(match c {
        'a' => Viseme::Aa,
        'o' => Viseme::O,
        'e' => Viseme::E,
        'i' => Viseme::I,
        'u' => Viseme::U,
        'v' => Viseme::U, // ü
        _ => return None,
    })
}

/// 拉丁字母 → 口型。粗略，只为了让夹在中文里的英文词有嘴型。
///
/// `TH` 这个口型这里永远不会产出：中文没有 /θ/，而英文是逐字母处理的、
/// 看不到 "th" 这个组合。保留在枚举里是因为它属于 Oculus 那一套标准集，
/// 少一个会让前端的映射表对不上。
fn latin_viseme(c: char) -> Viseme {
    match c {
        'a' => Viseme::Aa,
        'e' | 'i' => Viseme::I,
        'o' => Viseme::O,
        'u' => Viseme::U,
        'b' | 'p' | 'm' => Viseme::Pp,
        'f' | 'v' => Viseme::Ff,
        'd' | 't' => Viseme::Dd,
        'n' => Viseme::Nn,
        'l' => Viseme::Nn,
        'g' | 'k' | 'c' | 'q' => Viseme::Kk,
        'j' | 'y' => Viseme::Ch,
        's' | 'z' | 'x' => Viseme::Ss,
        'r' => Viseme::Rr,
        'h' => Viseme::Kk,
        'w' => Viseme::U,
        _ => Viseme::Dd,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str, offset_ms: u64, duration_ms: u64) -> WordBoundary {
        WordBoundary {
            offset_ms,
            duration_ms,
            text: text.to_string(),
        }
    }

    fn visemes(keys: &[VisemeKey]) -> Vec<Viseme> {
        keys.iter().map(|k| k.viseme).collect()
    }

    /// 双唇音必须闭嘴。
    ///
    /// 这条是所有口型里**最显眼**的：说「妈」的时候嘴不闭，一眼就是假的。
    #[test]
    fn bilabials_close_the_mouth_first() {
        let keys = schedule(&[word("妈", 0, 300)]);
        assert_eq!(
            keys[0].viseme,
            Viseme::Pp,
            "ma 的声母 m 必须先闭嘴：{keys:?}"
        );
        assert_eq!(keys[1].viseme, Viseme::Aa, "然后张开成 a");
    }

    /// 声母只占前一小段，不能占掉半个音节。
    #[test]
    fn onset_is_short_even_for_long_syllables() {
        // 一个字拖到 1 秒（慢速朗读）
        let keys = schedule(&[word("妈", 0, 1000)]);
        let vowel_at = keys.iter().find(|k| k.viseme == Viseme::Aa).unwrap().at_ms;
        assert!(
            vowel_at <= MAX_ONSET_MS,
            "拖长音时元音必须仍然很快到位，否则看起来像卡住了：{vowel_at}ms"
        );
    }

    /// 复韵母要两个口型，否则「好」和「喝」的嘴型一样。
    #[test]
    fn diphthongs_move_through_two_shapes() {
        let hao = visemes(&schedule(&[word("好", 0, 300)]));
        let he = visemes(&schedule(&[word("喝", 0, 300)]));
        assert!(
            hao.contains(&Viseme::Aa) && hao.contains(&Viseme::O),
            "hao 应当从 a 滑到 o：{hao:?}"
        );
        assert_ne!(hao, he, "hao 和 he 的口型序列不能一样");
    }

    /// 鼻韵尾要收口。不收的话说完「安」嘴还张着。
    #[test]
    fn nasal_codas_close_the_mouth() {
        let keys = schedule(&[word("安", 0, 300)]);
        // 末尾那帧是收尾的 sil，鼻音在它前面
        let nasal = keys.iter().rev().find(|k| k.viseme == Viseme::Nn);
        assert!(nasal.is_some(), "an 的韵尾 n 要有一帧：{keys:?}");
    }

    /// ang 的 ng 不能被切成 n + g。
    #[test]
    fn ng_is_not_split_into_n_and_g() {
        let keys = schedule(&[word("昂", 0, 300)]);
        assert!(
            !visemes(&keys).contains(&Viseme::Kk),
            "ang 里不该出现 g 的口型（kk）：{keys:?}"
        );
    }

    /// 零声母音节没有辅音帧，第一帧就是元音。
    #[test]
    fn zero_onset_starts_on_the_vowel() {
        let keys = schedule(&[word("啊", 0, 200)]);
        assert_eq!(keys[0].viseme, Viseme::Aa);
        assert_eq!(keys[0].at_ms, 0, "零声母不该有前置的辅音时间");
    }

    /// 撮口呼（ü）不能当成 i。嘴形一个是圆一个是扁，差别很明显。
    #[test]
    fn rounded_front_vowel_is_not_flat() {
        let keys = schedule(&[word("鱼", 0, 300)]);
        let vs = visemes(&keys);
        assert!(vs.contains(&Viseme::U), "yu 应当是撮口：{vs:?}");
    }

    /// 说完必须闭嘴。
    ///
    /// 少了这一帧的表现是「说完之后嘴半张着不动」—— 比口型不准更瘆人。
    #[test]
    fn always_closes_at_the_end() {
        let keys = schedule(&[word("你好", 0, 600)]);
        let last = keys.last().unwrap();
        assert_eq!(last.viseme, Viseme::Sil);
        assert_eq!(last.weight, 0.0);
        assert_eq!(last.at_ms, 600, "闭嘴的时刻就是这一句结束的时刻");
    }

    /// 标点不发音，但它占的时间要留着 —— 那就是句中的停顿。
    #[test]
    fn punctuation_produces_no_mouth_shape() {
        let keys = schedule(&[word("好，", 0, 400)]);
        // "好，" 两个字符，只有"好"发音
        let non_sil: Vec<_> = keys.iter().filter(|k| k.viseme != Viseme::Sil).collect();
        assert!(
            non_sil.iter().all(|k| k.at_ms < 200),
            "标点占的后半段不该有口型：{keys:?}"
        );
    }

    /// 词之间的空档要闭嘴。
    ///
    /// 实测出来的问题：Edge 把标点从词里摘掉，只在词边界之间留一段空白，
    /// 于是「你好，我是」在逗号处嘴保持张开近半秒。
    #[test]
    fn gaps_between_words_close_the_mouth() {
        let keys = schedule(&[word("好", 0, 300), word("我", 800, 300)]);
        let in_gap: Vec<_> = keys
            .iter()
            .filter(|k| (300..800).contains(&k.at_ms))
            .collect();
        assert!(
            in_gap.iter().any(|k| k.viseme == Viseme::Sil),
            "停顿里必须有一帧闭嘴：{keys:?}"
        );
    }

    /// 音节之间的正常过渡不闭嘴 —— 那会让整句抖。
    #[test]
    fn tiny_gaps_do_not_close_the_mouth() {
        let keys = schedule(&[word("好", 0, 300), word("我", 340, 300)]);
        let in_gap: Vec<_> = keys
            .iter()
            .filter(|k| (300..340).contains(&k.at_ms) && k.viseme == Viseme::Sil)
            .collect();
        assert!(in_gap.is_empty(), "40ms 的空档不该闭嘴：{keys:?}");
    }

    /// 纯标点的词整个跳过，不能崩、也不能产出无意义的帧。
    #[test]
    fn all_punctuation_word_is_skipped() {
        let keys = schedule(&[word("。", 0, 100)]);
        assert!(
            keys.iter().all(|k| k.viseme == Viseme::Sil),
            "只有标点时不该有口型：{keys:?}"
        );
    }

    /// 时间线必须有序 —— 前端的插值假设了这一点。
    #[test]
    fn keys_are_sorted_by_time() {
        let keys = schedule(&[
            word("今天", 0, 400),
            word("天气", 400, 400),
            word("不错", 800, 500),
        ]);
        assert!(
            keys.windows(2).all(|w| w[0].at_ms <= w[1].at_ms),
            "时间线乱序：{keys:?}"
        );
        assert!(keys.len() > 10, "六个字应当产出十几帧：{}", keys.len());
    }

    /// 空输入不崩。
    #[test]
    fn empty_input_yields_empty_timeline() {
        assert!(schedule(&[]).is_empty());
    }

    /// 时长为 0 的词不该导致除零或 panic。
    #[test]
    fn zero_duration_does_not_panic() {
        let keys = schedule(&[word("好", 0, 0)]);
        assert!(!keys.is_empty());
    }

    /// 一个词包含多个字时按字均分。
    #[test]
    fn multi_character_words_are_divided_evenly() {
        let keys = schedule(&[word("你好世界", 0, 800)]);
        // 四个字，每字 200ms —— 第二个字的第一帧应当落在 200ms 附近
        let starts: Vec<u64> = keys
            .iter()
            .filter(|k| k.weight >= CONSONANT_WEIGHT)
            .map(|k| k.at_ms)
            .collect();
        assert!(
            starts.iter().any(|&t| (190..=210).contains(&t)),
            "第二个字应当在 200ms 起：{starts:?}"
        );
    }

    /// 英文字母也给口型 —— 中文里夹英文词很常见。
    #[test]
    fn latin_letters_get_shapes() {
        let keys = schedule(&[word("OK", 0, 400)]);
        let vs = visemes(&keys);
        assert!(
            vs.contains(&Viseme::O) && vs.contains(&Viseme::Kk),
            "{vs:?}"
        );
    }

    /// 序列化成的名字必须正好是 Oculus morph target 的名字。
    ///
    /// 写错一个大小写，前端查 morphTargetDictionary 会查不到 —— 表现是
    /// 「模型加载了、嘴不动」，而且不报错。
    #[test]
    fn serializes_to_oculus_morph_target_names() {
        let json = serde_json::to_string(&[
            Viseme::Sil,
            Viseme::Pp,
            Viseme::Aa,
            Viseme::E,
            Viseme::Kk,
            Viseme::Nn,
        ])
        .unwrap();
        assert_eq!(json, r#"["sil","PP","aa","E","kk","nn"]"#);
    }
}
