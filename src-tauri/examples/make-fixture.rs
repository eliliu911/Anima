//! 生成 VAD 测试用的语音固件：本机 TTS 合一句，重采样到 16k 存成 WAV。
//!
//! 固件进版本库（约 40KB）。理由：**判据必须是真实语音**。
//! 用正弦或噪声做对照都不成立 —— Silero 学的是人声，而我一开始那条
//! 「静音不是语音」的测试在 VAD 完全瞎掉时照样通过。
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let body = serde_json::json!({
        "model": "kokoro", "voice": "zf_xiaobei", "response_format": "wav", "speed": 1.0,
        "input": "你好，这是一句用来测试语音检测的话。"
    });
    let wav = reqwest::Client::new()
        .post("http://127.0.0.1:8880/v1/audio/speech")
        .json(&body)
        .send()
        .await?
        .bytes()
        .await?
        .to_vec();
    let (samples, rate) = anima_lib::audio::playback::decode_mono(wav)?;
    let ratio = rate as f64 / 16_000.0;
    let mono: Vec<f32> = (0..(samples.len() as f64 / ratio) as usize)
        .map(|i| samples[((i as f64 * ratio) as usize).min(samples.len() - 1)])
        .collect();
    // 掐掉首尾静音，只留有声那一段 + 前后各 200ms
    let loud = |w: &[f32]| w.iter().fold(0.0f32, |m, v| m.max(v.abs())) > 0.02;
    let first = mono
        .chunks(320)
        .position(loud)
        .unwrap_or(0)
        .saturating_sub(10);
    let last = mono.chunks(320).rposition(loud).unwrap_or(0) + 10;
    // 截到 1.6 秒 —— 固件进版本库，够用就行
    let end = ((last * 320).min(mono.len())).min(first * 320 + 16_000 * 8 / 5);
    let clip = &mono[first * 320..end];
    let out = anima_lib::audio::playback::encode_mono_wav(clip, 16_000);
    let path = "tests/fixtures/speech-16k.wav";
    std::fs::write(path, &out)?;
    println!(
        "写出 {path}：{} 采样（{:.2}s），{} 字节",
        clip.len(),
        clip.len() as f32 / 16_000.0,
        out.len()
    );
    Ok(())
}
