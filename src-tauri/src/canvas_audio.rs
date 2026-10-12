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
}

#[derive(Debug, Clone, Serialize)]
pub struct CanvasSpeakOut {
    pub audio_path: String,
    /// 实际用的音色和引擎 —— **回显，不是回声**。
    /// 传了个不存在的音色时引擎可能悄悄退回默认，那时候这两个字段是
    /// 唯一能看出来的地方。
    pub voice: String,
    pub engine_id: String,
    pub secs: f32,
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
    let text = args.text.trim();
    if text.is_empty() {
        return Err(Error::config("没有词可念"));
    }
    let config_dir = state.config_dir.clone();

    // **引擎和音色的取法和 `cinema_dub` 一字不差。** 另写一份必然出现
    // 「分镜页配出来是这个声，画布上试听是另一个声」。
    let (selected, mut cfg) = {
        let config = state.gateway.config();
        let config = config.read().await;
        let want = if args.engine_id.is_empty() { &config.tts_engine } else { &args.engine_id };
        let selected = config
            .tts_engines
            .iter()
            .find(|e| &e.id == want)
            .cloned()
            .ok_or_else(|| Error::config(format!("找不到 TTS 引擎「{want}」，请在设置里重新选")))?;
        let mut c = config.voice.tts.clone();
        c.voice = crate::tts::effective_voice(&selected);
        (selected, c)
    };
    if !args.voice.trim().is_empty() {
        cfg.voice = args.voice.trim().to_string();
    }

    let tts = crate::tts::engine(&selected, state.http.clone(), &crate::tts::voices::voices_dir(&config_dir))?;
    let speech = tts.synthesize(text, &cfg).await?;
    if speech.audio.is_empty() {
        // **空音频要当场报。** 不报的话画布上会出现一个「成功了」的节点，
        // 点播放没声音 —— 而根因多半是这个音色在这个引擎里不存在。
        return Err(Error::runtime(format!(
            "合成回了空音频 —— 多半是音色「{}」在引擎「{}」里不存在",
            cfg.voice, selected.id
        )));
    }

    let dir = out_dir(&config_dir);
    std::fs::create_dir_all(&dir).map_err(|e| Error::runtime(format!("建不出音频目录：{e}")))?;
    // 引擎回的可能是 MP3 / PCM / WAV，统一转成 wav —— 和配音那条一致，
    // 后面「收进某一镜」直接就是 `audio/<镜头id>.wav` 要的格式。
    let path = out_path(&config_dir, &node_id);
    let raw = path.with_file_name(format!(".raw-{}", path.file_name().unwrap_or_default().to_string_lossy()));
    std::fs::write(&raw, &speech.audio).map_err(|e| Error::runtime(format!("写临时音频失败：{e}")))?;
    let out = std::process::Command::new(crate::cinema::ffmpeg_binary())
        .arg("-y").arg("-i").arg(&raw)
        .args(["-ac", "1", "-ar", "24000", "-c:a", "pcm_s16le"])
        .arg(&path)
        .output()
        .map_err(|e| Error::runtime(format!("执行 FFmpeg 失败：{e}")))?;
    let _ = std::fs::remove_file(&raw);
    if !out.status.success() || !path.is_file() {
        let err: String = String::from_utf8_lossy(&out.stderr).chars().rev().take(800).collect::<Vec<_>>().into_iter().rev().collect();
        return Err(Error::runtime(format!("转成 wav 失败：{err}")));
    }
    Ok(CanvasSpeakOut {
        audio_path: path.to_string_lossy().to_string(),
        voice: cfg.voice,
        engine_id: selected.id,
        secs: crate::cinema::clip_seconds(&path).map(|d| d as f32).unwrap_or(0.0),
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
    fn 空节点名也给得出文件名() {
        let p = out_path(std::path::Path::new("/tmp/x"), "///");
        assert!(p.file_name().unwrap().to_string_lossy().starts_with("audio-"));
    }
}
