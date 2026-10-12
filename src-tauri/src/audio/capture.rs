//! 麦克风采集。
//!
//! 链路：cpal 输入流 → 转单声道 → 重采样到 16kHz → 按 20ms 分帧 → VAD →
//! 把「一句话」的采样点交给上层。
//!
//! 两个关键约束：
//!   1. cpal 的回调跑在音频线程上，**绝对不能阻塞**（不能加锁等待、不能做 IO、
//!      不能分配大块内存），否则会爆音甚至丢流。所以回调里只做「转换 + 塞进
//!      无锁通道」，重活都在下游线程。
//!   2. `SpeechStart` 必须尽早发出去 —— 它是 barge-in 的触发信号。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::vad::{EnergyVad, Vad, VadConfig, VadEvent, FRAME_SAMPLES, SAMPLE_RATE};
use crate::error::{Error, Result};

/// 采集端产出的事件
#[derive(Debug, Clone)]
pub enum CaptureEvent {
    /// 用户开口了。上层收到就该立刻打断正在播放的 TTS 与在途的 LLM 流。
    SpeechStart,
    /// 一句话说完，附带 16kHz 单声道采样点，可直接送 STT。
    Utterance(Vec<f32>),
    /// 实时音量（RMS，0..1），给 UI 画电平/驱动粒子面容用。
    /// 刻意与音频数据分开发送，避免 UI 刷新拖累识别链路。
    Level(f32),
}

/// 一个正在运行的采集会话。drop 即停止采集并释放设备。
///
/// # 为什么音频流不在这个结构里
///
/// `cpal::Stream` **不是 `Send`**（内部持有绑定设备的裸指针），所以它既不能跨线程
/// 传递，也不能放进任何需要跨线程移动的结构。做法与 [`crate::audio::playback::Player`]
/// 一致：让流在自己的专属线程上原地待着，这里只保留一个关闭信号。
///
/// 这一点是踩了个很隐蔽的坑才搞清的。原先的写法是把流放在本结构里，然后：
///
/// ```ignore
/// std::thread::spawn(move || {
///     while let Ok(ev) = session.events.recv() { … }   // 只用到 .events
/// });
/// ```
///
/// Rust 2021 的**闭包精确捕获**只会搬走真正用到的 `events` 字段，于是流在闭包创建时
/// 就被**当场 drop** —— 采集"启动成功"、无任何报错、但一个采样都收不到。更糟的是
/// 编译器因此完全没有报错：非 `Send` 的字段压根没被捕获。改成整体捕获后编译器立刻
/// 指出 `cpal::Stream` 不是 `Send`，才暴露了真正的架构约束。
pub struct CaptureSession {
    events: Receiver<CaptureEvent>,
    /// drop 它会让独占音频流的线程退出，从而停止采集、释放设备
    _shutdown: std::sync::mpsc::Sender<()>,
}

impl CaptureSession {
    /// 阻塞取下一个采集事件。发送端关闭（会话结束）时返回 Err。
    ///
    /// 存在的意义见结构体文档：方法调用会捕获整个 `session`，
    /// 从而保证音频流不被精确捕获规则提前回收。
    pub fn recv(&self) -> std::result::Result<CaptureEvent, std::sync::mpsc::RecvError> {
        self.events.recv()
    }
}

/// 列出可用的输入设备名
pub fn input_devices() -> Result<Vec<String>> {
    let host = cpal::default_host();
    let devices = host
        .input_devices()
        .map_err(|e| Error::Io(format!("枚举输入设备失败：{e}")))?;
    Ok(devices.filter_map(|d| d.name().ok()).collect())
}

/// 开始采集。
///
/// - `device_name` 为 None 时用系统默认输入设备。
/// - `playing` 由播放侧置位，表示「TTS 正在出声」。采集侧据此收紧 VAD 阈值，
///   压掉麦克风听见扬声器导致的误打断（见 [`Vad::set_threshold_scale`]）。
pub fn start(
    device_name: Option<&str>,
    vad_config: VadConfig,
    playing: Arc<AtomicBool>,
    // config_dir：应用配置目录，用来找 Silero 模型。None = 不用 Silero
    config_dir: Option<&std::path::Path>,
) -> Result<CaptureSession> {
    let silero_model = config_dir.map(super::silero::model_path);
    let device_name = device_name.map(|s| s.to_string());

    let (event_tx, event_rx) = mpsc::channel::<CaptureEvent>();
    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();
    // 用 sync_channel(0) 把"设备是否打开成功"同步回传，
    // 保持 start() 的语义是「返回即在采集」，而不是延迟到第一帧才发现失败
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<Result<()>>(0);

    std::thread::Builder::new()
        .name("anima-audio-in".into())
        .spawn(move || {
            // 整个音频流的生命周期都在这个线程内，不跨线程移动
            let opened = (|| -> Result<(cpal::Stream, Receiver<Vec<f32>>, u32)> {
                let host = cpal::default_host();
                let device = match &device_name {
                    Some(name) => host
                        .input_devices()
                        .map_err(|e| Error::Io(format!("枚举输入设备失败：{e}")))?
                        .find(|d| d.name().map(|n| &n == name).unwrap_or(false))
                        .ok_or_else(|| Error::config(format!("找不到输入设备「{name}」")))?,
                    None => host.default_input_device().ok_or_else(|| {
                        Error::config(
                            "没有可用的麦克风。若是权限问题，请在系统设置→隐私与安全性→麦克风里允许 Anima"
                                .to_string(),
                        )
                    })?,
                };

                let config = device
                    .default_input_config()
                    .map_err(|e| Error::Io(format!("读取输入设备配置失败：{e}")))?;
                let device_rate = config.sample_rate().0;
                let channels = config.channels() as usize;

                tracing::info!(
                    device = %device.name().unwrap_or_default(),
                    rate = device_rate,
                    channels,
                    format = ?config.sample_format(),
                    "开始麦克风采集"
                );

                let (raw_tx, raw_rx) = mpsc::channel::<Vec<f32>>();
                let err_fn = |err| tracing::error!(%err, "音频输入流出错");

                let stream = match config.sample_format() {
                    cpal::SampleFormat::F32 => device.build_input_stream(
                        &config.into(),
                        move |data: &[f32], _| {
                            let _ = raw_tx.send(to_mono(data, channels));
                        },
                        err_fn,
                        None,
                    ),
                    cpal::SampleFormat::I16 => device.build_input_stream(
                        &config.into(),
                        move |data: &[i16], _| {
                            let floats: Vec<f32> =
                                data.iter().map(|s| *s as f32 / i16::MAX as f32).collect();
                            let _ = raw_tx.send(to_mono(&floats, channels));
                        },
                        err_fn,
                        None,
                    ),
                    cpal::SampleFormat::U16 => device.build_input_stream(
                        &config.into(),
                        move |data: &[u16], _| {
                            let floats: Vec<f32> = data
                                .iter()
                                .map(|s| (*s as f32 / u16::MAX as f32) * 2.0 - 1.0)
                                .collect();
                            let _ = raw_tx.send(to_mono(&floats, channels));
                        },
                        err_fn,
                        None,
                    ),
                    other => {
                        return Err(Error::config(format!("不支持的采样格式：{other:?}")))
                    }
                }
                .map_err(|e| Error::Io(format!("创建输入流失败：{e}")))?;

                stream
                    .play()
                    .map_err(|e| Error::Io(format!("启动输入流失败：{e}")))?;

                Ok((stream, raw_rx, device_rate))
            })();

            match opened {
                Ok((stream, raw_rx, device_rate)) => {
                    if ready_tx.send(Ok(())).is_err() {
                        return; // 调用方已放弃
                    }
                    // 下游处理（重采样→分帧→VAD）放另一个线程：raw_rx 是 Send 的，
                    // 而且不能在音频线程里做重活
                    std::thread::Builder::new()
                        .name("anima-audio-pipe".into())
                        .spawn(move || {
                            run_pipeline(raw_rx, event_tx, device_rate, vad_config, playing, silero_model);
                        })
                        .ok();

                    // 阻塞在这里：只要会话还活着，流就必须活着。
                    // CaptureSession 被 drop 时 Sender 关闭，recv 返回 Err，线程退出。
                    let _ = shutdown_rx.recv();
                    drop(stream);
                    tracing::info!("麦克风采集已停止");
                }
                Err(err) => {
                    let _ = ready_tx.send(Err(err));
                }
            }
        })
        .map_err(|e| Error::Io(format!("创建音频输入线程失败：{e}")))?;

    ready_rx
        .recv()
        .map_err(|_| Error::Io("音频输入线程意外退出".into()))??;

    Ok(CaptureSession {
        events: event_rx,
        _shutdown: shutdown_tx,
    })
}

/// 多声道交织数据取平均转单声道
fn to_mono(data: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return data.to_vec();
    }
    data.chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect()
}

/// 线性插值重采样。
///
/// 对 VAD + Whisper 这个用途足够：两者都不在意重采样引入的轻微高频失真，
/// 而线性插值几乎不耗 CPU。真要做高保真录音才需要多相滤波器。
fn resample_linear(input: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate || input.is_empty() {
        return input.to_vec();
    }
    let ratio = from_rate as f64 / to_rate as f64;
    let out_len = ((input.len() as f64) / ratio).floor() as usize;
    let mut out = Vec::with_capacity(out_len);

    for i in 0..out_len {
        let pos = i as f64 * ratio;
        let idx = pos.floor() as usize;
        let frac = (pos - idx as f64) as f32;
        let a = input[idx];
        // 末尾没有下一个点时直接用当前点，避免越界
        let b = *input.get(idx + 1).unwrap_or(&a);
        out.push(a + (b - a) * frac);
    }
    out
}

/// 一句话最长时长。超过就强制切断，防止环境噪声让一句话无限增长、
/// 最终把内存和 Whisper 的上下文都撑爆。
const MAX_UTTERANCE_SECONDS: usize = 30;

/// 开口前要往回补的音频长度。
///
/// VAD 需要连续几帧才确认「开口了」，那几帧本身也是语音的一部分。
/// 不往回补的话，识别结果会缺字（典型现象是第一个字被吃掉）。
const PREROLL_FRAMES: usize = 8;

fn run_pipeline(
    raw: Receiver<Vec<f32>>,
    events: Sender<CaptureEvent>,
    device_rate: u32,
    vad_config: VadConfig,
    playing: Arc<AtomicBool>,
    silero_model: Option<std::path::PathBuf>,
) {
    let playback_scale = vad_config.playback_threshold_scale;
    let diag_threshold = vad_config.energy_threshold;
    let mut diag_peak = 0.0f32;
    let mut diag_frames = 0usize;
    // 连续多少个窗口一个采样都没有。**精确的 0 不是安静，是流死了**
    let mut dead_windows = 0usize;

    // **优先 Silero，退回能量。**
    //
    // 能量 VAD 会把说话时音量偏低的部分（尾音、轻辅音、说得轻的时候）判成静音，
    // 结束计时器于是提前开始跑 —— 表现就是"话没说完就被切断"。
    // Silero 是学出来的检测器，这一点稳得多。
    //
    // 模型不在就退回能量版：**少一个可下载的资产不该让麦克风哑掉**。
    // 但要把原因写进日志，否则用户只会觉得"换了 Silero 但没变好"
    let mut vad: Box<dyn Vad> = match silero_model.as_deref() {
        Some(path) => match super::silero::SileroVad::load(path, &vad_config) {
            Ok(v) => {
                tracing::info!(path = %path.display(), "VAD 用 Silero");
                Box::new(v)
            }
            Err(err) => {
                tracing::warn!(
                    %err,
                    "加载 Silero 失败，退回能量 VAD（话没说完被切断的问题会仍然存在）"
                );
                Box::new(EnergyVad::new(vad_config))
            }
        },
        None => Box::new(EnergyVad::new(vad_config)),
    };
    // 记住上一次设的倍数，避免每帧都写一次
    let mut was_playing = false;
    // 累积不足一帧的余量
    let mut pending: Vec<f32> = Vec::with_capacity(FRAME_SAMPLES * 4);
    // 当前这句话已经收集的采样点
    let mut utterance: Vec<f32> = Vec::new();
    // 开口前的滚动缓冲
    let mut preroll: std::collections::VecDeque<Vec<f32>> = std::collections::VecDeque::new();
    let max_samples = MAX_UTTERANCE_SECONDS * SAMPLE_RATE as usize;
    // 电平事件节流：每 5 帧（100ms）发一次就够 UI 用了
    let mut frames_since_level = 0usize;

    // 发送端断开（会话结束）时 recv 返回 Err，循环自然退出
    while let Ok(chunk) = raw.recv() {
        pending.extend(resample_linear(&chunk, device_rate, SAMPLE_RATE));

        while pending.len() >= FRAME_SAMPLES {
            // 播放状态一变就调整灵敏度。放在分帧循环里而不是外层，
            // 是为了让 TTS 一开始出声就立刻生效，不等下一个音频回调。
            let is_playing = playing.load(Ordering::Relaxed);
            if is_playing != was_playing {
                vad.set_threshold_scale(if is_playing { playback_scale } else { 1.0 });
                was_playing = is_playing;
            }

            let frame: Vec<f32> = pending.drain(..FRAME_SAMPLES).collect();
            let event = vad.process(&frame);

            frames_since_level += 1;
            if frames_since_level >= 5 {
                frames_since_level = 0;
                let rms = (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt();
                let _ = events.send(CaptureEvent::Level(rms));

                // 诊断用：定期把真实电平打出来。
                //
                // 存在的理由很实际：麦克风权限没给时 CoreAudio **不报错**，只是
                // 持续送静音，表现和"有信号但没过阈值"一模一样。没有这行日志
                // 就只能靠猜。峰值一直是 0 → 权限/设备问题；峰值有值但没触发
                // → 阈值问题。
                diag_peak = diag_peak.max(rms);
                diag_frames += 1;
                if diag_frames >= 30 {
                    // 这里的计数单位是"电平事件"（每 5 帧一次 = 100ms），
                    // 不是原始帧。之前写 100 以为是 2 秒，实际要 10 秒才出一条，
                    // 排查时迟钝得没法用。10 × 100ms = 每秒一条。
                    // **一直是精确的 0 说明流是死的，不是"很安静"。**
                    //
                    // 真实的麦克风即使在安静房间也有底噪（1e-5 量级）。
                    // 峰值连续多个窗口都是 0.00000，只有两种可能：权限没给，
                    // 或者 CoreAudio 的输入流被快速关闭再打开之后成了僵尸流 ——
                    // 后者实测撞到过：连点几次开关麦克风，之后就再也收不到声音，
                    // 而且**不报任何错**，日志里只是电平变成 0。
                    if diag_peak == 0.0 {
                        dead_windows += 1;
                        if dead_windows == 3 {
                            tracing::error!(
                                "麦克风连续 3 个窗口一个采样都没有 —— 这不是安静，是流死了。\
                                 两种原因：系统没给麦克风权限，或者刚才反复开关过语音\
                                 （CoreAudio 的输入流被快速重开之后会变成僵尸流）。\
                                 关掉语音等两秒再开，通常能恢复"
                            );
                        }
                    } else {
                        dead_windows = 0;
                    }
                    tracing::info!(
                        peak = format!("{:.5}", diag_peak),
                        threshold = format!("{:.5}", diag_threshold),
                        "麦克风电平（2 秒窗口峰值）"
                    );
                    diag_frames = 0;
                    diag_peak = 0.0;
                }
            }

            match event {
                VadEvent::Silence => {
                    // 静音期只维护 preroll 环形缓冲
                    preroll.push_back(frame);
                    if preroll.len() > PREROLL_FRAMES {
                        preroll.pop_front();
                    }
                }
                VadEvent::SpeechStart => {
                    // 先把 preroll 里的语音前导补进来，再放当前帧
                    utterance.clear();
                    for f in preroll.drain(..) {
                        utterance.extend_from_slice(&f);
                    }
                    utterance.extend_from_slice(&frame);
                    // 立刻上报，让上层去打断 TTS/LLM
                    if events.send(CaptureEvent::SpeechStart).is_err() {
                        return;
                    }
                }
                VadEvent::Speech => {
                    utterance.extend_from_slice(&frame);
                    if utterance.len() >= max_samples {
                        tracing::warn!("单句超过 {MAX_UTTERANCE_SECONDS} 秒，强制切断");
                        if events
                            .send(CaptureEvent::Utterance(std::mem::take(&mut utterance)))
                            .is_err()
                        {
                            return;
                        }
                        vad.reset();
                    }
                }
                VadEvent::SpeechEnd => {
                    utterance.extend_from_slice(&frame);
                    if events
                        .send(CaptureEvent::Utterance(std::mem::take(&mut utterance)))
                        .is_err()
                    {
                        return;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_conversion_averages_channels() {
        // 双声道交织：L=1.0 R=0.0 → 0.5
        let stereo = [1.0, 0.0, 1.0, 0.0];
        assert_eq!(to_mono(&stereo, 2), vec![0.5, 0.5]);
    }

    #[test]
    fn mono_passthrough_when_single_channel() {
        let mono = [0.1, 0.2, 0.3];
        assert_eq!(to_mono(&mono, 1), vec![0.1, 0.2, 0.3]);
    }

    #[test]
    fn resample_downsamples_by_expected_ratio() {
        // 48kHz → 16kHz 应当是 1/3 长度
        let input: Vec<f32> = (0..300).map(|i| i as f32).collect();
        let out = resample_linear(&input, 48_000, 16_000);
        assert_eq!(out.len(), 100);
        // 线性插值下，等差输入应当仍是等差（步长 3）
        assert_eq!(out[0], 0.0);
        assert_eq!(out[1], 3.0);
        assert_eq!(out[2], 6.0);
    }

    #[test]
    fn resample_is_noop_at_same_rate() {
        let input = vec![0.1, 0.2, 0.3];
        assert_eq!(resample_linear(&input, 16_000, 16_000), input);
    }

    #[test]
    fn resample_handles_empty_and_does_not_panic_at_tail() {
        assert!(resample_linear(&[], 48_000, 16_000).is_empty());
        // 只有一个采样点时，末尾插值没有下一个点可用，不能越界
        let out = resample_linear(&[0.5], 48_000, 16_000);
        assert!(out.len() <= 1);
    }
}
