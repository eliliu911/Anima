//! 把一条长音频裁成可用的参考音频，并**用 Whisper 转写裁过的那一段**。
//!
//! ```text
//! cargo run --bin prep-voice-ref -- <输入音频> <输出目录/名字> [目标秒数]
//! ```
//!
//! ## 为什么需要它
//!
//! 零样本克隆要的是"参考音频 + **那段音频的**文字"。用户手上的素材常常是：
//!
//! - **只有音频没有文字**（自己录的、从视频里扒的）
//! - **太长**（实测用户拿来的两条是 22 秒和 25 秒）。F5-TTS 会截断超长参考，
//!   而且参考长度直接加进**每一次**合成的耗时
//!
//! 手工裁音频容易，但裁完文字就对不上了 —— 而"文字和音频不匹配"的症状是
//! **音色像但吐字乱**，极难归因。
//!
//! 所以顺序必须是：**先裁音频，再转写裁过的那一段**。这样文字天然匹配。
//!
//! 项目里已经有 whisper.cpp（Metal）和 ggml 模型，不需要再下任何东西。

use std::path::{Path, PathBuf};

use anima_lib::stt::{self, whisper::WhisperStt, SpeechToText, SttConfig};

/// Whisper 的输入采样率。它只吃 16kHz
const WHISPER_HZ: u32 = 16_000;

/// 参考音频写出的采样率。和音色库里其它参考保持一致
const OUT_HZ: u32 = 24_000;

/// 默认裁多长。8 秒是实测的甜点：够模型抓稳音色，又不会让每次合成都变慢
const DEFAULT_SECONDS: f32 = 8.0;

fn resample(x: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || x.is_empty() {
        return x.to_vec();
    }
    let n = ((x.len() as f64) * to as f64 / from as f64).round() as usize;
    // 线性插值：参考音频只是给模型听音色、给 ASR 认字，不进最终输出
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

/// 在整段里找最适合当参考的那个窗口。
///
/// 判据是**有声比例最高**，而不是能量最大：能量最大的窗口往往是一句喊叫或大笑，
/// 那个语气会渗进之后所有的合成。有声比例高意味着"一直在正常说话"。
///
/// 窗口起点还要对齐到静音处 —— 从一个字中间切进去，参考的第一个音素就是残缺的。
fn best_window(x: &[f32], sr: u32, want: usize) -> (usize, usize) {
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

    // 滑动统计有声帧数
    let mut count: i64 = voiced[..win_frames.min(voiced.len())]
        .iter()
        .filter(|&&v| v)
        .count() as i64;
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

    // 把起点往前挪到最近的静音帧：从字中间切进去会让第一个音素残缺
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

fn write_wav(path: &Path, samples: &[f32], sr: u32) -> Result<(), String> {
    let wav = anima_lib::audio::playback::encode_mono_wav(samples, sr);
    std::fs::write(path, wav).map_err(|e| format!("写 {} 失败：{e}", path.display()))
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!(
            "用法：prep-voice-ref <输入音频> <输出前缀> [秒数，默认 {DEFAULT_SECONDS}]\n\
             \n\
             例：prep-voice-ref ~/a.mp3 voice-library/jialan/default 8\n\
             会写出 default.wav（{OUT_HZ}Hz 单声道）和 default.txt（Whisper 转写）\n\
             \n\
             先裁再转写，所以文字天然和音频匹配 —— 反过来做的话，\n\
             文字和音频不匹配的症状是「音色像但吐字乱」，极难归因。"
        );
        return Err("参数不足".into());
    }
    let input = PathBuf::from(shellexpand(&args[0]));
    let out_prefix = PathBuf::from(&args[1]);
    let want_sec: f32 = args
        .get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_SECONDS);

    let bytes = std::fs::read(&input).map_err(|e| format!("读不了 {}：{e}", input.display()))?;
    let (samples, sr) = anima_lib::audio::playback::decode_mono(bytes)
        .map_err(|e| format!("解码失败（{}）：{e}", input.display()))?;
    let total = samples.len() as f32 / sr as f32;
    println!("输入：{} —— {total:.1}s，{sr}Hz", input.display());

    let want = (want_sec * sr as f32) as usize;
    let (from, to) = best_window(&samples, sr, want);
    let clip = &samples[from..to];
    println!(
        "裁出：{:.1}s ~ {:.1}s（{:.1}s）",
        from as f32 / sr as f32,
        to as f32 / sr as f32,
        clip.len() as f32 / sr as f32
    );

    // ── 转写**裁过的那一段** ──
    let config_dir = dirs_config()?;
    let model_path = stt::discover_model(&config_dir).ok_or_else(|| {
        format!(
            "找不到 Whisper 模型。把 ggml-*.bin 放进 {}",
            stt::default_model_dir(&config_dir).display()
        )
    })?;
    let stt_config = SttConfig {
        model_path,
        // 语种交给它自己判 —— 素材可能是中文也可能是日文。
        // 注意 whisper.cpp 里"不设置语种"等于英文，所以 auto 必须真的传下去
        language: "auto".into(),
        ..SttConfig::default()
    };
    println!("转写模型：{}", stt_config.model_path);

    let for_whisper = resample(clip, sr, WHISPER_HZ);
    let stt = tokio::task::spawn_blocking(move || WhisperStt::load(stt_config))
        .await
        .map_err(|e| format!("加载模型任务异常：{e}"))?
        .map_err(|e| format!("加载 Whisper 失败：{e}"))?;
    let transcript = stt
        .transcribe(&for_whisper)
        .await
        .map_err(|e| format!("转写失败：{e}"))?;
    let text = transcript.text.trim().to_string();
    if text.is_empty() {
        return Err("转写结果是空的 —— 这一段可能没有人声".into());
    }
    println!("语种：{}", transcript.language);
    println!("转写：{text}");

    // ── 写出 ──
    if let Some(parent) = out_prefix.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("建目录失败：{e}"))?;
    }
    let out_wav = out_prefix.with_extension("wav");
    let out_txt = out_prefix.with_extension("txt");
    write_wav(&out_wav, &resample(clip, sr, OUT_HZ), OUT_HZ)?;
    std::fs::write(&out_txt, &text).map_err(|e| format!("写 {} 失败：{e}", out_txt.display()))?;

    println!("\n已写出：");
    println!("  {}", out_wav.display());
    println!("  {}", out_txt.display());
    println!("\n**请核对上面那句转写** —— 它错了会让克隆吐字乱（音色还是像的，所以很难发现）。");
    println!("重启服务后生效：./scripts/anima.sh restart");
    Ok(())
}

/// 展开开头的 `~`。这个工具是手工在命令行跑的，路径带 ~ 很常见
fn shellexpand(path: &str) -> String {
    match path.strip_prefix("~/") {
        Some(rest) => match std::env::var_os("HOME") {
            Some(home) => Path::new(&home).join(rest).to_string_lossy().to_string(),
            None => path.to_string(),
        },
        None => path.to_string(),
    }
}

/// 应用数据目录 —— Whisper 模型就放在它下面的 models/。
fn dirs_config() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or("取不到 HOME")?;
    Ok(Path::new(&home).join("Library/Application Support/com.anima.desktop"))
}
