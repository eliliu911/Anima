//! 量 Whisper 的平均 token 概率，用来定「幻听」的阈值。
//!
//! 为什么需要这个：Whisper **不会说"我没听见"**。喂它一段静音或风扇声，
//! 它照样会给你一句话（日志里真实出现过 `字幕:J Chong`、`菲爾:哈囉`）。
//! 唯一能把这种结果和真话区分开的数字就是平均 token 概率，
//! 而这个数字之前从来没人看过 —— 阈值只能量，不能猜。
//!
//!   cargo run --example stt-probe

use anima_lib::stt::whisper::WhisperStt;
use anima_lib::stt::{SpeechToText, SttConfig};

const SR: usize = 16_000;

/// 确定性伪随机噪声。不用 rand：这个探针的结论要能复现
fn noise(n: usize, amplitude: f32) -> Vec<f32> {
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    (0..n)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            ((seed >> 40) as f32 / 8_388_608.0 - 1.0) * amplitude
        })
        .collect()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = dirs::config_dir().unwrap().join("com.anima.desktop");
    let model = dir.join("models/ggml-small.bin");
    let stt = WhisperStt::load(SttConfig {
        model_path: model.to_string_lossy().to_string(),
        language: "auto".into(),
        ..Default::default()
    })?;

    let wav = std::fs::read("tests/fixtures/speech-16k.wav")?;
    let (speech, rate) = anima_lib::audio::playback::decode_mono(wav)?;
    assert_eq!(rate as usize, SR, "固件必须是 16kHz");

    let cases: Vec<(&str, Vec<f32>)> = vec![
        ("真实语音（固件）", speech.clone()),
        // 播放漏音就是这个形状：真语音，但弱
        (
            "真实语音 ×0.05（模拟回声漏音）",
            speech.iter().map(|s| s * 0.05).collect(),
        ),
        ("纯静音 2 秒", vec![0.0; SR * 2]),
        ("低电平噪声 2 秒", noise(SR * 2, 0.005)),
        ("中电平噪声 2 秒", noise(SR * 2, 0.05)),
        ("极弱语音 ×0.01", speech.iter().map(|s| s * 0.01).collect()),
        (
            "极弱语音 ×0.002",
            speech.iter().map(|s| s * 0.002).collect(),
        ),
        ("极低噪声 2 秒（近静音）", noise(SR * 2, 0.0005)),
        ("极低噪声 4 秒", noise(SR * 4, 0.001)),
        ("语音尾巴 0.5 秒 + 静音", {
            let mut v: Vec<f32> = speech.iter().rev().take(SR / 2).rev().cloned().collect();
            v.extend(std::iter::repeat(0.0).take(SR));
            v
        }),
    ];

    println!("\n{:<32} {:>6}  识别出的文字", "输入", "可信度");
    println!("{}", "─".repeat(78));
    for (label, samples) in cases {
        let t = stt.transcribe(&samples).await?;
        let shown = if t.text.is_empty() {
            "（空 —— 已被过滤或本来就没内容）".to_string()
        } else {
            t.text.chars().take(30).collect()
        };
        println!("{label:<32} {:>6.2}  {shown}", t.confidence);
    }
    println!(
        "\n定阈值看这两行的差距：真实语音 vs 静音/噪声。\
         SHORT_MIN_CONFIDENCE 要落在中间，且离真实语音那一侧远一些。"
    );
    Ok(())
}
