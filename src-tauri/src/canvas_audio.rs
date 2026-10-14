//! 画布上的音频节点：**同一段词，几把嗓子并排听。**
//!
//! # 它补的是什么
//!
//! 配音能力早就有了（[`crate::cinema_dub`]），但它绑死在「某一集的某一镜」上。
//! 想换个音色只能走「改角色资产 → 整镜重新配音 → 听」——
//! 要比三把嗓子就得来回三趟，而且每趟都会**覆盖掉**上一趟的结果。
//!
//! 画布上的做法是把「试音色」当成一个**动作**而不是一个库：
//! 三个音频节点挂同一段词、各选一把嗓子，一次出三条，并排听。
//!
//! # 口型这件事必须先说清
//!
//! H3 原声的口型是跟着它自己生成的那段语音做的。**一旦换成 TTS 配音，
//! 口型就对不上** —— 这是个真取舍，不是能两全的事
//! （见 `cinema_dub` 的模块注释）。
//!
//! 画布上的音频节点**不改变这个取舍**，它只是让取舍看得见：
//! 并排听完再决定这一镜要原声还是要配音。界面上写着这句话。
//!
//! # 不另造一条合成路
//!
//! 引擎和音色的取法、`TtsConfig` 的来源，和 `cinema_dub` 完全一样 ——
//! 另写一份必然出现「分镜页配出来是这个声，画布上试听是另一个声」。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

#[derive(Debug, Clone, Deserialize)]
pub struct CanvasSpeakArgs {
    pub text: String,
    /// 空 = 用当前引擎的默认音色
    #[serde(default)]
    pub voice: String,
    /// 空 = 用设置里当前选的引擎
    #[serde(default)]
    pub engine_id: String,
    /// **声音复制**（2026-10-13，照 LibTV：上游连进来的音频当参考）。本机路径；空 = 不复制，按 `voice` 念
    #[serde(default)]
    pub ref_audio: String,
    /// 参考音频说的那句话。空 = 自动（裁 8 秒、Whisper 转写，见 `tts::clone_ref`）；用户改过就用改过的
    #[serde(default)]
    pub ref_text: String,
    /// 语速倍数（1 = 原速，0.5~2）
    #[serde(default = "one")]
    pub speed: f32,
    /// 声调（半音，-12~12，0 = 不变）
    #[serde(default)]
    pub pitch: f32,
    /// 音量倍数（1 = 不变，0~2）
    #[serde(default = "one")]
    pub volume: f32,
    /// 输出采样率：8000 / 16000 / 24000 / 48000
    #[serde(default = "default_rate")]
    pub sample_rate: u32,
    /// 输出格式：wav / mp3 / pcm / ogg_opus
    #[serde(default = "default_format")]
    pub format: String,
}

fn one() -> f32 { 1.0 }
fn default_rate() -> u32 { 24_000 }
fn default_format() -> String { "wav".into() }

#[derive(Debug, Clone, Serialize)]
pub struct CanvasSpeakOut {
    pub audio_path: String,
    /// 实际用的音色和引擎 —— **回显，不是回声**。
    /// 传了个不存在的音色时引擎可能悄悄退回默认，那时候这两个字段是
    /// 唯一能看出来的地方。
    pub voice: String,
    pub engine_id: String,
    pub secs: f32,
    /// 声音复制时：实际用的那段参考（原音频里第几秒到第几秒）和它的文字。界面上给用户核对 —— 文字错了会「音色像但吐字乱」
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clone: Option<CloneUsed>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CloneUsed {
    pub ref_text: String,
    /// 文字是用户改过的（不是 Whisper 转的）
    pub ref_text_edited: bool,
    pub from_secs: f32,
    pub to_secs: f32,
    pub source_secs: f32,
    /// 参考不到 3 秒、末尾补了静音（GPT-SoVITS 要 3~10 秒）。太短克隆不稳，界面上提醒换长一点的
    pub padded: bool,
}

/// 哪些引擎会**拿一段任意音频**克隆。Edge / Kokoro 只有自带音色；F5-TTS 走 OpenAI 兼容接口，只认音色库里的名字
pub fn clone_capable(kind: crate::tts::TtsEngineKind) -> bool {
    use crate::tts::TtsEngineKind::*;
    matches!(kind, GptSoVits | CosyVoice2 | IndexTts2)
}

/// 合成完的统一后处理（FFmpeg 滤镜）。`engine_speed` = 引擎自己做不了语速（CosyVoice2），在这里变速
pub fn post_filters(speed: f32, pitch: f32, volume: f32, engine_speed: bool, in_rate: u32) -> Vec<String> {
    let mut f = Vec::new();
    let pitch = pitch.clamp(-12.0, 12.0);
    if pitch.abs() > 0.01 {
        // 变调不变速：采样率乘 2^(n/12) 抬调，再 atempo 把时长拉回来（atempo 单次 0.5~2，±12 半音刚好在范围内）
        let k = 2f32.powf(pitch / 12.0);
        f.push(format!("asetrate={}", (in_rate as f32 * k).round() as u32));
        f.push(format!("aresample={in_rate}"));
        f.push(format!("atempo={:.4}", 1.0 / k));
    }
    if !engine_speed {
        let s = speed.clamp(0.5, 2.0);
        if (s - 1.0).abs() > 0.001 {
            f.push(format!("atempo={s:.3}"));
        }
    }
    let v = volume.clamp(0.0, 2.0);
    if (v - 1.0).abs() > 0.001 {
        f.push(format!("volume={v:.3}"));
    }
    f
}

/// 输出格式 → （扩展名, FFmpeg 编码参数）。认不出的按 wav
pub fn output_codec(format: &str) -> (&'static str, Vec<&'static str>) {
    match format {
        "mp3" => ("mp3", vec!["-c:a", "libmp3lame", "-b:a", "192k"]),
        "pcm" => ("pcm", vec!["-f", "s16le", "-c:a", "pcm_s16le"]),
        "ogg_opus" => ("ogg", vec!["-c:a", "libopus", "-b:a", "64k"]),
        _ => ("wav", vec!["-c:a", "pcm_s16le"]),
    }
}

/// 采样率只给这四档（Opus 只认 8/12/16/24/48k，四档都在里面）
pub fn output_rate(rate: u32) -> u32 {
    match rate {
        8000 | 16_000 | 24_000 | 48_000 => rate,
        _ => 24_000,
    }
}

fn out_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("anima").join("canvas").join("audio")
}

/// 文件名去掉不安全字符；同一个节点重跑要换文件，否则播放器拿着旧的缓存不刷新。
///
/// **时间戳用毫秒，不是 `jobs::now()` 的秒。** 试音色正是「连点几下比着听」
/// 的场景，一秒内出两条太正常了 —— 秒级会撞名，表现是「点了没反应，
/// 还是刚才那把嗓子」，而且不报错。
fn out_path(config_dir: &Path, id: &str) -> PathBuf {
    let safe: String = id.chars().filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_').collect();
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    out_dir(config_dir).join(format!("{}-{}.wav", if safe.is_empty() { "audio" } else { &safe }, ms))
}

#[tauri::command]
pub async fn canvas_speak(
    state: tauri::State<'_, crate::AppState>,
    node_id: String,
    args: CanvasSpeakArgs,
) -> Result<CanvasSpeakOut> {
    let config = state.gateway.config().read().await.clone();
    speak(&state.config_dir, &config, state.http.clone(), &node_id, args).await
}

/// `canvas_speak` 的正文（不依赖 Tauri，实跑测试从这里进）
pub async fn speak(
    config_dir: &Path,
    config: &crate::config::AppConfig,
    http: reqwest::Client,
    node_id: &str,
    args: CanvasSpeakArgs,
) -> Result<CanvasSpeakOut> {
    let text = args.text.trim();
    if text.is_empty() {
        return Err(Error::config("没有词可念"));
    }

    // **引擎和音色的取法和 `cinema_dub` 一字不差。** 另写一份必然出现
    // 「分镜页配出来是这个声，画布上试听是另一个声」。
    let want = if args.engine_id.is_empty() { &config.tts_engine } else { &args.engine_id };
    let mut selected = config
        .tts_engines
        .iter()
        .find(|e| &e.id == want)
        .cloned()
        .ok_or_else(|| Error::config(format!("找不到 TTS 引擎「{want}」，请在设置里重新选")))?;
    let mut cfg = config.voice.tts.clone();
    cfg.voice = crate::tts::effective_voice(&selected);
    if !args.voice.trim().is_empty() {
        cfg.voice = args.voice.trim().to_string();
    }

    // ── 声音复制：参考音频 → 裁好的参考 + 文字，塞进这次用的引擎配置 ──
    let mut clone = None;
    if !args.ref_audio.trim().is_empty() {
        if !clone_capable(selected.kind) {
            return Err(Error::config(format!(
                "引擎「{}」不会复制声音 —— 换成 GPT-SoVITS、CosyVoice2 或 IndexTTS2（节点上的引擎下拉里选）",
                selected.label
            )));
        }
        let r = crate::tts::clone_ref::prepare(config_dir, Path::new(args.ref_audio.trim())).await?;
        let edited = !args.ref_text.trim().is_empty();
        let ref_text = if edited { args.ref_text.trim().to_string() } else { r.text.clone() };
        selected.ref_audio_path = Some(r.audio.to_string_lossy().to_string());
        selected.ref_text = Some(ref_text.clone());
        // GPT-SoVITS 有 voice 时优先走音色库 —— 复制时必须清掉，不然念出来的是库里那把嗓子、不报错
        cfg.voice = String::new();
        clone = Some(CloneUsed { ref_text, ref_text_edited: edited, from_secs: r.from_secs, to_secs: r.to_secs, source_secs: r.source_secs, padded: r.padded });
    }

    // 语速：引擎自己会的交给引擎（更自然），CosyVoice2 不会 → 后处理变速
    let engine_speed = !matches!(selected.kind, crate::tts::TtsEngineKind::CosyVoice2);
    let speed = args.speed.clamp(0.5, 2.0);
    if engine_speed && (speed - 1.0).abs() > 0.001 {
        cfg.rate = format!("{:+.0}%", (speed - 1.0) * 100.0);
    }

    let tts = crate::tts::engine(&selected, http, &crate::tts::voices::voices_dir(config_dir))?;
    let speech = tts.synthesize(text, &cfg).await?;
    if speech.audio.is_empty() {
        // **空音频要当场报。** 不报的话画布上会出现一个「成功了」的节点，
        // 点播放没声音 —— 而根因多半是这个音色在这个引擎里不存在。
        return Err(Error::runtime(format!(
            "合成回了空音频 —— 多半是音色「{}」在引擎「{}」里不存在",
            cfg.voice, selected.id
        )));
    }

    let dir = out_dir(config_dir);
    std::fs::create_dir_all(&dir).map_err(|e| Error::runtime(format!("建不出音频目录：{e}")))?;
    // 引擎回的可能是 MP3 / PCM / WAV，统一过一遍 FFmpeg：后处理（声调 / 音量 / 变速）+ 选的采样率和格式
    let (ext, codec) = output_codec(&args.format);
    let rate = output_rate(args.sample_rate);
    let path = out_path(config_dir, node_id).with_extension(ext);
    let raw = path.with_file_name(format!(".raw-{}", path.file_name().unwrap_or_default().to_string_lossy()));
    std::fs::write(&raw, &speech.audio).map_err(|e| Error::runtime(format!("写临时音频失败：{e}")))?;
    // 先统一到 24k 再做滤镜，asetrate 的基准才对
    let mut filters = vec!["aresample=24000".to_string()];
    filters.extend(post_filters(speed, args.pitch, args.volume, engine_speed, 24_000));
    let out = std::process::Command::new(crate::cinema::ffmpeg_binary())
        .arg("-y").arg("-i").arg(&raw)
        .arg("-af").arg(filters.join(","))
        .args(["-ac", "1", "-ar"]).arg(rate.to_string())
        .args(&codec)
        .arg(&path)
        .output()
        .map_err(|e| Error::runtime(format!("执行 FFmpeg 失败：{e}")))?;
    let _ = std::fs::remove_file(&raw);
    if !out.status.success() || !path.is_file() {
        let err: String = String::from_utf8_lossy(&out.stderr).chars().rev().take(800).collect::<Vec<_>>().into_iter().rev().collect();
        return Err(Error::runtime(format!("转成 {ext} 失败：{err}")));
    }
    // 裸 PCM 没有文件头，ffprobe 读不出时长 —— 按字节数算
    let secs = if ext == "pcm" {
        std::fs::metadata(&path).map(|m| m.len() as f32 / (rate as f32 * 2.0)).unwrap_or(0.0)
    } else {
        crate::cinema::clip_seconds(&path).map(|d| d as f32).unwrap_or(0.0)
    };
    Ok(CanvasSpeakOut {
        audio_path: path.to_string_lossy().to_string(),
        voice: cfg.voice,
        engine_id: selected.id,
        secs,
        clone,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 文件名去掉不安全字符_同一节点每次不同() {
        let d = std::path::Path::new("/tmp/x");
        let a = out_path(d, "audio-1758/../../etc");
        assert!(!a.to_string_lossy().contains(".."), "路径能跳出去：{a:?}");
        assert!(a.to_string_lossy().contains("audio-1758etc"));
        // 同一个节点重跑要换文件，否则播放器拿着旧的缓存不刷新。
        // **试音色就是「连点几下比着听」**，一秒内出两条太正常 ——
        // 所以时间戳必须是毫秒级的。
        std::thread::sleep(std::time::Duration::from_millis(2));
        assert_ne!(a, out_path(d, "audio-1758/../../etc"));
    }

    #[test]
    fn 后处理滤镜_声调音量变速() {
        assert!(post_filters(1.0, 0.0, 1.0, true, 24_000).is_empty(), "全默认什么都不加");
        let up = post_filters(1.0, 12.0, 1.0, true, 24_000);
        assert_eq!(up, vec!["asetrate=48000", "aresample=24000", "atempo=0.5000"], "升 12 半音 = 采样率翻倍再把时长拉回来");
        assert_eq!(post_filters(1.5, 0.0, 1.0, true, 24_000), Vec::<String>::new(), "引擎会变速就不重复变速");
        assert_eq!(post_filters(1.5, 0.0, 1.0, false, 24_000), vec!["atempo=1.500"], "CosyVoice2 这种在后处理变速");
        assert_eq!(post_filters(1.0, 0.0, 0.5, true, 24_000), vec!["volume=0.500"]);
        assert_eq!(post_filters(9.0, 99.0, 9.0, false, 24_000).len(), 5, "越界夹住，不报错");
    }

    #[test]
    fn 输出格式和采样率() {
        assert_eq!(output_codec("mp3").0, "mp3");
        assert_eq!(output_codec("ogg_opus").0, "ogg");
        assert_eq!(output_codec("pcm").0, "pcm");
        assert_eq!(output_codec("乱写").0, "wav");
        assert_eq!(output_rate(48_000), 48_000);
        assert_eq!(output_rate(44_100), 24_000, "不在四档里的按 24k");
    }

    #[test]
    fn 只有能克隆的引擎放行声音复制() {
        use crate::tts::TtsEngineKind::*;
        assert!(clone_capable(GptSoVits) && clone_capable(CosyVoice2) && clone_capable(IndexTts2));
        assert!(!clone_capable(Edge) && !clone_capable(OpenAiCompatible) && !clone_capable(F5Tts));
    }

    #[test]
    fn 空节点名也给得出文件名() {
        let p = out_path(std::path::Path::new("/tmp/x"), "///");
        assert!(p.file_name().unwrap().to_string_lossy().starts_with("audio-"));
    }

    /// **声音复制实跑**：拿一段音频当参考，用设置里当前的引擎念一句新的（要本机 TTS 服务在跑）。
    /// `ANIMA_CLONE_REF=/a.wav ANIMA_CLONE_TEXT='…' cargo test --lib 声音复制实跑 -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "要本机 TTS 服务 + Whisper 模型"]
    async fn 声音复制实跑() {
        let cfg_dir = dirs::config_dir().unwrap().join("com.anima.desktop");
        let config = crate::config::AppConfig::load(&cfg_dir.join("config.json")).unwrap();
        let args = CanvasSpeakArgs {
            text: std::env::var("ANIMA_CLONE_TEXT").unwrap_or_else(|_| "零号已经不在这里了，你们回去吧。".into()),
            voice: String::new(), engine_id: std::env::var("ANIMA_CLONE_ENGINE").unwrap_or_default(),
            ref_audio: std::env::var("ANIMA_CLONE_REF").unwrap_or_default(), ref_text: String::new(),
            speed: 1.0, pitch: 0.0, volume: 1.0, sample_rate: 24_000, format: "wav".into(),
        };
        let t0 = std::time::Instant::now();
        let out = speak(&cfg_dir, &config, reqwest::Client::new(), "clone-live", args).await.expect("复制失败");
        println!("用时 {:.1}s → {} ({:.1}s)，引擎 {}", t0.elapsed().as_secs_f32(), out.audio_path, out.secs, out.engine_id);
        println!("参考：{:?}", out.clone);
    }
}
