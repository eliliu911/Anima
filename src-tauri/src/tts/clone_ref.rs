//! **声音复制的参考音频**：一段随手拿来的音频 → 能喂给零样本克隆的「参考音频 + 那段音频的文字」。
//!
//! 原先只在命令行工具 `prep-voice-ref` 里（给音色库做参考用），2026-10-13 画布「声音复制」要用，挪进库里两边共用。
//!
//! # 顺序必须是先裁、再转写裁过的那一段
//!
//! 零样本克隆（GPT-SoVITS / CosyVoice2）要的是参考音频**和它说的那句话**。
//! 文字和音频对不上的症状是「音色像但吐字乱」，极难归因 —— 所以裁完再转写，文字天然匹配。
//!
//! # 为什么要裁
//!
//! 用户手上的素材常常太长（一首歌、一整段对白）。参考太长会被引擎截断，
//! 而且参考长度加进**每一次**合成的耗时。8 秒是实测的甜点（音色库那批就是这么做的）。

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::stt::{self, whisper::WhisperStt, SpeechToText, SttConfig};

/// Whisper 只吃 16kHz
pub const WHISPER_HZ: u32 = 16_000;
/// 参考音频写出的采样率，和音色库里其它参考一致
pub const OUT_HZ: u32 = 24_000;
/// 默认裁多长
pub const DEFAULT_SECONDS: f32 = 8.0;
/// 参考短于这个就在末尾补静音补到 [`PAD_TO_SECONDS`]。GPT-SoVITS 要求参考在 3~10 秒，
/// 短了直接 400「Reference audio is outside the 3-10 second range」—— 而用户随手拿来的常常就是一句台词（2026-10-13 实跑撞到 2.8 秒）
pub const MIN_SECONDS: f32 = 3.0;
pub const PAD_TO_SECONDS: f32 = 3.2;

/// 线性插值重采样。参考音频只是给模型听音色、给 ASR 认字，不进最终输出
pub fn resample(x: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || x.is_empty() {
        return x.to_vec();
    }
    let n = ((x.len() as f64) * to as f64 / from as f64).round() as usize;
    (0..n)
        .map(|i| {
            let pos = i as f64 * from as f64 / to as f64;
            let i0 = pos.floor() as usize;
            let frac = (pos - i0 as f64) as f32;
            let a = x.get(i0).copied().unwrap_or(0.0);
            let b = x.get(i0 + 1).copied().unwrap_or(a);
            a + (b - a) * frac
        })
        .collect()
}

/// 在整段里找最适合当参考的那个窗口（样本下标 `[from, to)`）。
///
/// 判据是**有声比例最高**，而不是能量最大：能量最大的窗口往往是一句喊叫或大笑，
/// 那个语气会渗进之后所有的合成。窗口起点往前挪到静音处 —— 从一个字中间切进去，第一个音素是残缺的。
pub fn best_window(x: &[f32], sr: u32, want: usize) -> (usize, usize) {
    if x.len() <= want {
        return (0, x.len());
    }
    let hop = (sr / 50).max(1) as usize; // 20ms
    let frames: Vec<f32> = x
        .chunks(hop)
        .map(|c| (c.iter().map(|v| v * v).sum::<f32>() / c.len() as f32).sqrt())
        .collect();
    let peak = frames.iter().copied().fold(0.0f32, f32::max);
    if peak <= 0.0 {
        return (0, want.min(x.len()));
    }
    let voiced: Vec<bool> = frames.iter().map(|&v| v > peak * 0.06).collect();
    let win_frames = (want / hop).max(1);
    let mut count: i64 = voiced[..win_frames.min(voiced.len())].iter().filter(|&&v| v).count() as i64;
    let mut best = (0usize, count);
    for start in 1..voiced.len().saturating_sub(win_frames) {
        if voiced[start - 1] {
            count -= 1;
        }
        if voiced[start + win_frames - 1] {
            count += 1;
        }
        if count > best.1 {
            best = (start, count);
        }
    }
    let mut start_frame = best.0;
    for back in 0..win_frames.min(start_frame) {
        if !voiced[start_frame - back] {
            start_frame -= back;
            break;
        }
    }
    let from = (start_frame * hop).min(x.len());
    let to = (from + want).min(x.len());
    (from, to)
}

/// 处理好的参考
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CloneRef {
    /// 裁好的参考（24kHz 单声道 wav）
    pub audio: PathBuf,
    /// Whisper 转写的**裁过那一段**的文字
    pub text: String,
    /// 在原音频里的起止秒数（界面上告诉用户用的是哪一段）
    pub from_secs: f32,
    pub to_secs: f32,
    pub source_secs: f32,
    /// 太短、末尾补过静音
    #[serde(default)]
    pub padded: bool,
}

/// 缓存键：同一个文件（路径 + 大小 + 修改时间）只裁、只转写一次。
/// 用户改了文件（重新上传同名文件）会换键
fn cache_key(src: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    // 处理规则的版本：改了裁法 / 补静音这类规则就进位，旧缓存自动作废（v2 = 短于 3 秒补静音）
    "clone-ref-v2".hash(&mut h);
    src.to_string_lossy().hash(&mut h);
    if let Ok(m) = std::fs::metadata(src) {
        m.len().hash(&mut h);
        if let Ok(t) = m.modified() {
            t.hash(&mut h);
        }
    }
    format!("{:016x}", h.finish())
}

pub fn cache_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("anima").join("canvas").join("audio").join("clone_refs")
}

/// 把 `src` 处理成克隆参考。有缓存直接用；没有就裁 8 秒、Whisper 转写，写进缓存。
pub async fn prepare(config_dir: &Path, src: &Path) -> Result<CloneRef> {
    if !src.is_file() {
        return Err(Error::config(format!("参考音频不在：{}", src.display())));
    }
    let dir = cache_dir(config_dir);
    let key = cache_key(src);
    let meta = dir.join(format!("{key}.json"));
    if let Some(r) = std::fs::read_to_string(&meta).ok().and_then(|t| serde_json::from_str::<CloneRef>(&t).ok()) {
        if r.audio.is_file() && !r.text.trim().is_empty() {
            return Ok(r);
        }
    }

    let bytes = std::fs::read(src).map_err(|e| Error::Io(format!("读不了参考音频 {}：{e}", src.display())))?;
    let (samples, sr) = crate::audio::playback::decode_mono(bytes)
        .map_err(|e| Error::runtime(format!("参考音频解码失败（{}）：{e}", src.display())))?;
    if samples.is_empty() || sr == 0 {
        return Err(Error::runtime("参考音频是空的"));
    }
    let want = (DEFAULT_SECONDS * sr as f32) as usize;
    let (from, to) = best_window(&samples, sr, want);
    let clip = &samples[from..to];

    let model_path = stt::discover_model(config_dir).ok_or_else(|| {
        Error::config(format!("找不到 Whisper 模型（转写参考音频要用）。把 ggml-*.bin 放进 {}", stt::default_model_dir(config_dir).display()))
    })?;
    // whisper.cpp 里「不设语种」等于英文，auto 必须真的传下去
    let stt_config = SttConfig { model_path, language: "auto".into(), ..SttConfig::default() };
    let for_whisper = resample(clip, sr, WHISPER_HZ);
    let stt = tokio::task::spawn_blocking(move || WhisperStt::load(stt_config))
        .await
        .map_err(|e| Error::runtime(format!("加载 Whisper 任务异常：{e}")))??;
    let text = stt.transcribe(&for_whisper).await?.text.trim().to_string();
    if text.is_empty() {
        return Err(Error::runtime("参考音频里没认出人声 —— 换一段有清楚说话声的"));
    }

    std::fs::create_dir_all(&dir).map_err(|e| Error::Io(format!("建不出缓存目录：{e}")))?;
    let audio = dir.join(format!("{key}.wav"));
    let mut out = resample(clip, sr, OUT_HZ);
    let padded = (out.len() as f32) < MIN_SECONDS * OUT_HZ as f32;
    if padded {
        out.resize((PAD_TO_SECONDS * OUT_HZ as f32) as usize, 0.0);
    }
    std::fs::write(&audio, crate::audio::playback::encode_mono_wav(&out, OUT_HZ))
        .map_err(|e| Error::Io(format!("写参考音频失败：{e}")))?;
    let r = CloneRef {
        audio,
        text,
        from_secs: from as f32 / sr as f32,
        to_secs: to as f32 / sr as f32,
        source_secs: samples.len() as f32 / sr as f32,
        padded,
    };
    let _ = std::fs::write(&meta, serde_json::to_string_pretty(&r).unwrap_or_default());
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 短于窗口整段用_长的挑有声最多的那段() {
        let sr = 1000;
        assert_eq!(best_window(&vec![0.5; 500], sr, 800), (0, 500));
        // 前 2 秒静音、中间 3 秒说话、后 2 秒静音 → 挑中间
        let mut x = vec![0.0f32; 2000];
        x.extend((0..3000).map(|i| ((i as f32) * 0.3).sin() * 0.5));
        x.extend(vec![0.0f32; 2000]);
        let (from, to) = best_window(&x, sr, 2000);
        assert!(from >= 1800 && to <= 5200, "{from}..{to}");
    }

    #[test]
    fn 同一个文件同一个键_换了内容换键() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.wav");
        std::fs::write(&p, b"1234").unwrap();
        let k1 = cache_key(&p);
        assert_eq!(k1, cache_key(&p));
        std::fs::write(&p, b"12345678").unwrap();
        assert_ne!(k1, cache_key(&p));
    }
}
