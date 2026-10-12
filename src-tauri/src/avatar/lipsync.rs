//! 音频驱动的口型：从**正在播的声音**里实时算出该张什么嘴。
//!
//! ## 为什么要有这一条，文本驱动那条还不够
//!
//! 现有的口型是**文本驱动**的：文字 → 拼音 → viseme 时间线（`viseme.rs`），
//! 精度高（15 个 viseme，含辅音），但它有个硬前提 —— **得有文本**。
//!
//! 于是这些场景嘴一动不动：
//!
//! - `play_audio` / 电台放出来的任何声音
//! - 外部送进来的音频
//!
//! 这条链路补的就是这一段：没有文本时，直接听声音。
//!
//! ## 做法：MFCC + 模板比对
//!
//! 和 AIRI 用的 wLipSync 是同一类做法 —— 取一帧音频算 MFCC，和事先标定好的
//! 几个元音模板比余弦相似度，最像哪个就张哪个嘴。
//!
//! **精度天然低于文本驱动**：只分得出元音（张口的形状），分不出辅音。
//! 所以它是**补位**，不是替代 —— 有文本时仍然走 `viseme.rs`。
//!
//! ## 没有标定模板时怎么办
//!
//! 退回**只按响度张嘴**。这一档很粗糙，但比"嘴完全不动"强得多，
//! 而且不需要任何标定就能用 —— 用户不做任何事也能看到嘴在动。

use crate::avatar::viseme::Viseme;

/// 一帧多少个采样。1024 @ 16kHz = 64ms，和 wLipSync 的取值一致：
/// 再短了频谱分辨率不够（分不开共振峰），再长了口型跟不上语速。
///
/// **必须和播放层送来的帧长一致**（`audio::playback::ANALYSIS_FRAME`）。
/// 播放层不知道我们要做 FFT，所以它自己定长度；两边对不上的话
/// FFT 会补零到 1024，频谱被拉平 —— 口型仍然会动，只是全都分不出元音，
/// 而且**不报任何错**。下面有测试盯着。
pub const FRAME: usize = 1024;
/// 统一重采样到这个频率再算。语音的共振峰都在 4kHz 以下，16k 足够，
/// **而且模板必须和输入同一个采样率**，否则 mel 滤波器组对不上
pub const TARGET_RATE: u32 = 16_000;
/// mel 滤波器组的通道数
const MEL_CHANNELS: usize = 30;
/// 取几阶 MFCC。低阶反映的是频谱包络（也就是口型），高阶是音高和细节
const MFCC_NUM: usize = 12;

/// 低于这个响度就算没在说话，直接闭嘴。
///
/// **不设这条的话，静音段的底噪也会被算出一个"最像"的元音** ——
/// 嘴会在没声音的时候自己嘟囔
const SILENCE_RMS: f32 = 0.008;

/// 一个标定好的元音模板
#[derive(Debug, Clone, PartialEq)]
pub struct Template {
    pub viseme: Viseme,
    /// 归一化过的 MFCC。比对用余弦相似度，所以只关心方向
    pub mfcc: Vec<f32>,
}

/// 一套模板。空的话退回按响度张嘴
#[derive(Debug, Clone, Default)]
pub struct Profile {
    pub templates: Vec<Template>,
}

/// 一帧的分析结果
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub viseme: Viseme,
    /// 张口幅度 0..1。由响度决定 —— 模板只决定**形状**，不决定大小
    pub weight: f32,
}

/// 就地做 radix-2 FFT。`re`/`im` 长度必须是 2 的幂。
///
/// 自己写而不是拉一个 FFT 库：这里只需要一个定长实部输入的正变换，
/// 而多一个依赖就多一处版本与构建问题。三十行、有测试
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    debug_assert!(n.is_power_of_two() && im.len() == n);

    // 位反转置换
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }

    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f32::consts::PI / len as f32;
        let (wr, wi) = (ang.cos(), ang.sin());
        let mut i = 0;
        while i < n {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let (ur, ui) = (re[i + k], im[i + k]);
                let (vr, vi) = (
                    re[i + k + len / 2] * cr - im[i + k + len / 2] * ci,
                    re[i + k + len / 2] * ci + im[i + k + len / 2] * cr,
                );
                re[i + k] = ur + vr;
                im[i + k] = ui + vi;
                re[i + k + len / 2] = ur - vr;
                im[i + k + len / 2] = ui - vi;
                let nr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = nr;
            }
            i += len;
        }
        len <<= 1;
    }
}

fn hz_to_mel(hz: f32) -> f32 {
    2595.0 * (1.0 + hz / 700.0).log10()
}
fn mel_to_hz(mel: f32) -> f32 {
    700.0 * (10f32.powf(mel / 2595.0) - 1.0)
}

/// 算一帧的 MFCC。返回**归一化**过的向量 —— 比对只看方向，不看能量大小
pub fn mfcc(samples: &[f32], sample_rate: u32) -> Vec<f32> {
    let n = samples.len().min(FRAME);
    if n < 64 {
        return Vec::new();
    }
    let size = FRAME;

    let mut re = vec![0f32; size];
    let mut im = vec![0f32; size];
    // 预加重 + 汉明窗。预加重抬高高频，让共振峰更清楚
    let mut prev = 0.0f32;
    for i in 0..n {
        let s = samples[i] - 0.97 * prev;
        prev = samples[i];
        let w = 0.54 - 0.46 * (2.0 * std::f32::consts::PI * i as f32 / (n - 1) as f32).cos();
        re[i] = s * w;
    }
    fft(&mut re, &mut im);

    // 功率谱（只要前一半）
    let half = size / 2;
    let power: Vec<f32> = (0..half).map(|k| re[k] * re[k] + im[k] * im[k]).collect();

    // mel 滤波器组
    let nyquist = sample_rate as f32 / 2.0;
    let mel_max = hz_to_mel(nyquist);
    let mut energies = [0f32; MEL_CHANNELS];
    for (c, e) in energies.iter_mut().enumerate() {
        let lo = mel_to_hz(mel_max * c as f32 / (MEL_CHANNELS + 1) as f32);
        let mid = mel_to_hz(mel_max * (c + 1) as f32 / (MEL_CHANNELS + 1) as f32);
        let hi = mel_to_hz(mel_max * (c + 2) as f32 / (MEL_CHANNELS + 1) as f32);
        let bin = |hz: f32| ((hz / nyquist) * half as f32) as usize;
        let (b0, b1, b2) = (bin(lo), bin(mid), bin(hi).min(half - 1));
        let mut sum = 0.0;
        for (k, p) in power.iter().enumerate().take(b2 + 1).skip(b0) {
            let w = if k <= b1 {
                if b1 == b0 {
                    1.0
                } else {
                    (k - b0) as f32 / (b1 - b0) as f32
                }
            } else if b2 == b1 {
                1.0
            } else {
                (b2 - k) as f32 / (b2 - b1) as f32
            };
            sum += p * w;
        }
        // log 之前加一个极小量，静音时不会变成 -inf
        *e = (sum + 1e-10).ln();
    }

    // DCT-II，取低 MFCC_NUM 阶（跳过第 0 阶：那是整体能量，不是口型）
    let mut out = Vec::with_capacity(MFCC_NUM);
    for i in 1..=MFCC_NUM {
        let mut sum = 0.0;
        for (c, e) in energies.iter().enumerate() {
            sum += e
                * (std::f32::consts::PI * i as f32 * (c as f32 + 0.5) / MEL_CHANNELS as f32).cos();
        }
        out.push(sum);
    }
    normalize(&mut out);
    out
}

fn normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-6 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

/// 张口幅度。响度到幅度不是线性的 —— 用平方根压一下，
/// 小声时也能看出嘴在动
fn openness(level: f32) -> f32 {
    (level / 0.15).sqrt().clamp(0.0, 1.0)
}

/// 分析一帧，给出该张什么嘴、张多大。
///
/// 没有模板时退回**只按响度张嘴**（用 `aa`）—— 粗糙，但比嘴完全不动强，
/// 而且不需要任何标定
pub fn analyze(samples: &[f32], sample_rate: u32, profile: &Profile) -> Frame {
    let level = rms(samples);
    if level < SILENCE_RMS {
        return Frame {
            viseme: Viseme::Sil,
            weight: 0.0,
        };
    }
    let weight = openness(level);

    if profile.templates.is_empty() {
        return Frame {
            viseme: Viseme::Aa,
            weight,
        };
    }
    let coeffs = mfcc(samples, sample_rate);
    if coeffs.is_empty() {
        return Frame {
            viseme: Viseme::Aa,
            weight,
        };
    }

    let mut best = (&profile.templates[0], f32::MIN);
    for t in &profile.templates {
        if t.mfcc.len() != coeffs.len() {
            continue;
        }
        let sim: f32 = t.mfcc.iter().zip(&coeffs).map(|(a, b)| a * b).sum();
        if sim > best.1 {
            best = (t, sim);
        }
    }
    Frame {
        viseme: best.0.viseme,
        weight,
    }
}

/// 从一段音频标定一个模板：整段切帧、算 MFCC、取平均。
///
/// **要跳过静音帧**。不跳的话，一段"啊——"里前后的静音会把模板拉向底噪，
/// 标出来的模板谁都不像
pub fn calibrate(viseme: Viseme, samples: &[f32], sample_rate: u32) -> Option<Template> {
    let mut acc = vec![0f32; MFCC_NUM];
    let mut used = 0usize;
    for chunk in samples.chunks(FRAME) {
        if chunk.len() < FRAME / 2 || rms(chunk) < SILENCE_RMS {
            continue;
        }
        let c = mfcc(chunk, sample_rate);
        if c.len() == MFCC_NUM {
            for (a, x) in acc.iter_mut().zip(&c) {
                *a += x;
            }
            used += 1;
        }
    }
    if used == 0 {
        return None;
    }
    for a in acc.iter_mut() {
        *a /= used as f32;
    }
    normalize(&mut acc);
    Some(Template { viseme, mfcc: acc })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(freq: f32, secs: f32, rate: u32, amp: f32) -> Vec<f32> {
        let n = (secs * rate as f32) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / rate as f32;
                amp * (2.0 * std::f32::consts::PI * freq * t).sin()
            })
            .collect()
    }

    /// 两个共振峰叠加，粗略模拟一个元音
    fn vowel(f1: f32, f2: f32, secs: f32, rate: u32) -> Vec<f32> {
        let a = tone(f1, secs, rate, 0.35);
        let b = tone(f2, secs, rate, 0.25);
        a.iter().zip(&b).map(|(x, y)| x + y).collect()
    }

    /// FFT 要先自己对：后面所有东西都建在它上面
    #[test]
    fn fft_finds_the_right_bin() {
        let rate = 16_000u32;
        let freq = 1000.0f32;
        let mut re: Vec<f32> = (0..FRAME)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin())
            .collect();
        let mut im = vec![0f32; FRAME];
        fft(&mut re, &mut im);

        let half = FRAME / 2;
        let peak = (1..half)
            .max_by(|&a, &b| {
                let pa = re[a] * re[a] + im[a] * im[a];
                let pb = re[b] * re[b] + im[b] * im[b];
                pa.partial_cmp(&pb).unwrap()
            })
            .unwrap();
        let expected = (freq / rate as f32 * FRAME as f32).round() as usize;
        assert!(
            (peak as i32 - expected as i32).abs() <= 1,
            "峰值在 {peak}，应该在 {expected} 附近"
        );
    }

    #[test]
    fn mfcc_has_the_right_shape_and_is_normalized() {
        let s = vowel(700.0, 1200.0, 0.1, TARGET_RATE);
        let c = mfcc(&s, TARGET_RATE);
        assert_eq!(c.len(), MFCC_NUM);
        let norm: f32 = c.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-3, "没归一化：{norm}");
    }

    /// **同一个元音的两段音频要算出相近的 MFCC，不同元音要拉开** ——
    /// 这是整套比对成立的前提
    #[test]
    fn the_same_vowel_matches_itself_better_than_a_different_one() {
        let a1 = mfcc(&vowel(700.0, 1200.0, 0.1, TARGET_RATE), TARGET_RATE);
        let a2 = mfcc(&vowel(710.0, 1190.0, 0.1, TARGET_RATE), TARGET_RATE);
        let i1 = mfcc(&vowel(300.0, 2400.0, 0.1, TARGET_RATE), TARGET_RATE);

        let dot = |x: &[f32], y: &[f32]| x.iter().zip(y).map(|(a, b)| a * b).sum::<f32>();
        assert!(
            dot(&a1, &a2) > dot(&a1, &i1),
            "同元音相似度 {:.3} 没高过异元音 {:.3}",
            dot(&a1, &a2),
            dot(&a1, &i1)
        );
    }

    #[test]
    fn calibration_then_matching_picks_the_right_vowel() {
        let profile = Profile {
            templates: vec![
                calibrate(
                    Viseme::Aa,
                    &vowel(700.0, 1200.0, 0.3, TARGET_RATE),
                    TARGET_RATE,
                )
                .unwrap(),
                calibrate(
                    Viseme::I,
                    &vowel(300.0, 2400.0, 0.3, TARGET_RATE),
                    TARGET_RATE,
                )
                .unwrap(),
                calibrate(
                    Viseme::U,
                    &vowel(320.0, 800.0, 0.3, TARGET_RATE),
                    TARGET_RATE,
                )
                .unwrap(),
            ],
        };
        for (f1, f2, want) in [
            (700.0, 1200.0, Viseme::Aa),
            (300.0, 2400.0, Viseme::I),
            (320.0, 800.0, Viseme::U),
        ] {
            let s = vowel(f1, f2, 0.08, TARGET_RATE);
            assert_eq!(analyze(&s, TARGET_RATE, &profile).viseme, want, "{f1}/{f2}");
        }
    }

    /// **静音必须闭嘴。** 不判的话底噪也会被算出一个"最像"的元音，
    /// 嘴会在没声音的时候自己嘟囔
    #[test]
    fn silence_closes_the_mouth() {
        let quiet = vec![0.0001f32; FRAME];
        let f = analyze(&quiet, TARGET_RATE, &Profile::default());
        assert_eq!(f.viseme, Viseme::Sil);
        assert_eq!(f.weight, 0.0);
    }

    /// 没有模板也要能用 —— 退回按响度张嘴，比嘴完全不动强
    #[test]
    fn it_works_without_any_templates() {
        let loud = vowel(700.0, 1200.0, 0.08, TARGET_RATE);
        let f = analyze(&loud, TARGET_RATE, &Profile::default());
        assert_eq!(f.viseme, Viseme::Aa);
        assert!(f.weight > 0.3, "响的时候要张嘴：{}", f.weight);
    }

    /// 张口幅度由响度决定，模板只决定形状
    #[test]
    fn louder_means_a_wider_mouth() {
        let p = Profile::default();
        let soft = analyze(
            &vowel(700.0, 1200.0, 0.08, TARGET_RATE)
                .iter()
                .map(|x| x * 0.15)
                .collect::<Vec<_>>(),
            TARGET_RATE,
            &p,
        );
        let loud = analyze(&vowel(700.0, 1200.0, 0.08, TARGET_RATE), TARGET_RATE, &p);
        assert!(
            loud.weight > soft.weight,
            "{} vs {}",
            loud.weight,
            soft.weight
        );
        assert!(loud.weight <= 1.0);
    }

    /// 标定时要跳过静音帧 —— 不跳的话模板会被底噪拉偏
    #[test]
    fn calibration_skips_silent_frames() {
        let mut s = vec![0.0f32; FRAME * 3];
        s.extend(vowel(700.0, 1200.0, 0.3, TARGET_RATE));
        s.extend(vec![0.0f32; FRAME * 3]);
        let with_silence = calibrate(Viseme::Aa, &s, TARGET_RATE).unwrap();
        let clean = calibrate(
            Viseme::Aa,
            &vowel(700.0, 1200.0, 0.3, TARGET_RATE),
            TARGET_RATE,
        )
        .unwrap();
        let sim: f32 = with_silence
            .mfcc
            .iter()
            .zip(&clean.mfcc)
            .map(|(a, b)| a * b)
            .sum();
        assert!(sim > 0.95, "静音把模板拉偏了，相似度只有 {sim:.3}");
    }

    /// 全是静音时标不出模板，要返回 None 而不是一个垃圾模板
    #[test]
    fn calibrating_pure_silence_returns_nothing() {
        assert!(calibrate(Viseme::Aa, &vec![0.0; FRAME * 4], TARGET_RATE).is_none());
    }

    /// **和播放层的帧长必须一致。**
    ///
    /// 对不上的话 FFT 会补零，频谱被拉平，口型分不出元音 ——
    /// 而且不报任何错，只是"她的嘴一直在做同一个形状"
    #[test]
    fn the_frame_length_matches_what_playback_sends() {
        assert_eq!(FRAME, crate::audio::playback::ANALYSIS_FRAME);
        assert!(FRAME.is_power_of_two(), "FFT 要求 2 的幂");
    }

    /// 太短的输入不能 panic
    #[test]
    fn tiny_input_is_handled() {
        assert!(mfcc(&[0.1, 0.2], TARGET_RATE).is_empty());
        let f = analyze(&[0.5; 32], TARGET_RATE, &Profile::default());
        assert!(f.weight >= 0.0);
    }
}
