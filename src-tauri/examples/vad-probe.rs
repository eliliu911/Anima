//! 拿真实语音喂给 VAD，把每一窗的概率打出来。
//!
//! 「说话没反应」在日志里看不出原因：模型加载成功、推理也没报错，
//! 只是判定一直是"不是语音"。而概率本身从来没人看过。
use anima_lib::audio::silero::SileroVad;
use anima_lib::audio::vad::{Vad, VadConfig, FRAME_SAMPLES};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = dirs::config_dir().unwrap().join("com.anima.desktop");
    let model = anima_lib::audio::silero::model_path(&dir);

    // 用本机 TTS 合一段真人声当输入 —— 比找音频文件可靠
    let text = "你好，我现在正在说一句比较长的话，用来测试语音检测能不能认出来。";
    let body = serde_json::json!({
        "model": "kokoro", "input": text, "voice": "zf_xiaobei",
        "response_format": "wav", "speed": 1.0
    });
    let wav = reqwest::Client::new()
        .post("http://127.0.0.1:8880/v1/audio/speech")
        .json(&body)
        .send()
        .await?
        .bytes()
        .await?
        .to_vec();
    println!("合成到 {} 字节音频", wav.len());

    let (samples, rate) = anima_lib::audio::playback::decode_mono(wav)?;
    println!("解码：{} 采样 @ {}Hz", samples.len(), rate);

    // 线性重采样到 16k
    let target = 16_000usize;
    let ratio = rate as f64 / target as f64;
    let n = (samples.len() as f64 / ratio) as usize;
    let mono: Vec<f32> = (0..n)
        .map(|i| samples[((i as f64 * ratio) as usize).min(samples.len() - 1)])
        .collect();
    let peak = mono.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    println!("重采样到 {} 采样 @16kHz，峰值 {peak:.3}\n", mono.len());

    // 先看能量分布在哪些帧 —— 窗口里全是 0 的话，问题可能在输入而不是模型
    let peaks: Vec<f32> = mono
        .chunks(FRAME_SAMPLES)
        .map(|f| f.iter().fold(0.0f32, |m, v| m.max(v.abs())))
        .collect();
    let loud: Vec<usize> = peaks
        .iter()
        .enumerate()
        .filter(|(_, p)| **p > 0.02)
        .map(|(i, _)| i)
        .collect();
    println!(
        "有声帧 {} / {}（首个 {:?}，末个 {:?}）",
        loud.len(),
        peaks.len(),
        loud.first(),
        loud.last()
    );
    println!("前 8 帧峰值：{:?}\n", &peaks[..8.min(peaks.len())]);

    let mut vad = SileroVad::load(&model, &VadConfig::default())?;
    let mut events = std::collections::BTreeMap::new();
    for (i, frame) in mono.chunks(FRAME_SAMPLES).enumerate() {
        if frame.len() < FRAME_SAMPLES {
            break;
        }
        let ev = vad.process(frame);
        *events.entry(format!("{ev:?}")).or_insert(0) += 1;
        if (60..110).contains(&i) && i % 10 == 0 {
            println!(
                "  帧 {i:>3} ({:>4}ms)  概率 {:.4}  {ev:?}",
                i * 20,
                vad.last_probability(),
            );
        }
    }
    println!("\n各事件计数：{events:?}");
    Ok(())
}
