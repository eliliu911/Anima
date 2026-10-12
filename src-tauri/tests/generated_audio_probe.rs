//! 把生成视频里的音轨转写出来 —— 用来验证「提示词里的台词是否真的驱动了配音」。
//!
//! 这不是断言，是**取证工具**：默认 `#[ignore]`，需要时手动跑。
//!
//! 为什么要有它：MiniMax H3 是音视频联合生成的，做 AI 漫剧时提示词里会带
//! 角色台词。音轨"有内容"（RMS 不为 0）只能说明不是静音，**说明不了它念的是
//! 什么** —— 那正是这个项目反复吃亏的地方：统计值只能否定，不能肯定。
//!
//! 用项目自己的 whisper 转写，把「听起来对不对」变成可读的文字。
//!
//! ```bash
//! # 先用 ffmpeg 把音轨抽成 16kHz 单声道 wav
//! ffmpeg -i out.mp4 -ac 1 -ar 16000 -f wav /tmp/probe.wav
//! ANIMA_AUDIO_PROBE=/tmp/probe.wav \
//!   cargo test --test generated_audio_probe -- --ignored --nocapture
//! # 判是不是粤语：换 large-v3 系列模型、指定语言
//! ANIMA_WHISPER_MODEL=.../ggml-large-v3-turbo.bin ANIMA_WHISPER_LANG=yue ANIMA_AUDIO_PROBE=... cargo test ...
//! ```

use anima_lib::stt::whisper::WhisperStt;
use anima_lib::stt::{SpeechToText, SttConfig};

fn read_wav_f32(path: &str) -> Vec<f32> {
    let bytes = std::fs::read(path).expect("读不到 wav");
    // 最简 PCM16 解析：跳过 44 字节头，其余按小端 i16 读
    assert!(bytes.len() > 44, "wav 太短");
    bytes[44..]
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
        .collect()
}

#[tokio::test]
#[ignore = "取证工具：需要 ANIMA_AUDIO_PROBE 指向一个 16kHz 单声道 wav"]
async fn transcribe_generated_audio() {
    let Ok(wav) = std::env::var("ANIMA_AUDIO_PROBE") else {
        panic!("请设置 ANIMA_AUDIO_PROBE=/path/to/16k_mono.wav");
    };
    let model = std::env::var("ANIMA_WHISPER_MODEL").unwrap_or_else(|_| {
        format!(
            "{}/Library/Application Support/com.anima.desktop/models/ggml-small.bin",
            std::env::var("HOME").unwrap_or_default()
        )
    });

    let samples = read_wav_f32(&wav);
    let secs = samples.len() as f32 / 16000.0;
    println!("音频 {:.1} 秒 / {} 采样", secs, samples.len());
    let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len().max(1) as f32).sqrt();
    println!("RMS {rms:.4}（接近 0 = 静音）");

    let stt = WhisperStt::load(SttConfig {
        model_path: model,
        // `ANIMA_WHISPER_LANG`：`yue` 粤语、`auto` 让它自己判（2026-10-06 粤语实测用；要 large-v3 系列才认 yue）
        language: std::env::var("ANIMA_WHISPER_LANG").unwrap_or_else(|_| "zh".to_string()),
        threads: 0,
        translate: false,
    })
    .expect("加载 whisper 失败");

    let t = stt.transcribe(&samples).await.expect("转写失败");
    println!("──── 转写结果 ────\n{}\n────────────────", t.text.trim());
    println!(
        "判据：文字里如果出现提示词里的台词，说明台词**驱动了配音**；\
         如果只是含混人声或环境音，说明音轨是氛围音而不是对白。"
    );
}
