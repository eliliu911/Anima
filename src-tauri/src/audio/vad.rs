//! 语音活动检测（VAD）。
//!
//! 职责：从连续的麦克风流里切出「一句话」，并尽早告诉上层「用户开口了」——
//! 后者是 barge-in 的触发信号，必须比「说完了」早得多，否则打断就来不及。
//!
//! 这里先给出一个基于能量 + 零穿越率的实现。架构文档规划的是 Silero VAD
//! (ONNX)，准确率更高；[`Vad`] trait 就是为了那次替换而存在的 —— 上层只依赖
//! trait，换实现不用动语音管线。

use serde::Serialize;

/// 语音管线统一的采样率。Whisper 要求 16kHz 单声道，所以整条链路都按这个走，
/// 采集端负责把设备的原生采样率重采样过来。
pub const SAMPLE_RATE: u32 = 16_000;

/// VAD 的处理粒度。20ms @ 16kHz = 320 采样点。
/// 选 20ms 是因为它既够短（打断判定及时），又够长（避免单帧噪声导致抖动）。
pub const FRAME_SAMPLES: usize = 320;

/// VAD 对一帧音频的判定
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VadEvent {
    /// 仍在静音
    Silence,
    /// 刚检测到用户开口 —— barge-in 的触发点
    SpeechStart,
    /// 语音持续中
    Speech,
    /// 一句话结束（静音超过 `silence_timeout`）
    SpeechEnd,
}

pub trait Vad: Send {
    /// 喂入一帧 [`FRAME_SAMPLES`] 长度的 f32 采样（范围 -1.0..1.0）
    fn process(&mut self, frame: &[f32]) -> VadEvent;

    /// 重置内部状态（切换设备、结束一轮对话时调用）
    fn reset(&mut self);

    /// 临时缩放判定阈值。`1.0` 为正常，越大越不敏感。
    ///
    /// 存在的理由是回声：TTS 正在出声时麦克风会录到扬声器的声音，不收紧阈值
    /// 就会被自己的声音不断误判成「用户开口了」，打断刚说出口的第一句话。
    /// CoreAudio 的回声消除在外接音箱下不可靠，所以必须有这条降级方案
    /// （ARCHITECTURE.md 坑点 7）。
    fn set_threshold_scale(&mut self, scale: f32);
}

#[derive(Debug, Clone)]
pub struct VadConfig {
    /// 判定为语音的能量阈值（RMS）。太低会把风扇声当人声。
    pub energy_threshold: f32,
    /// 连续多少帧超过阈值才认为「真的开口了」。
    /// 用来滤掉键盘敲击、桌面碰撞这类单帧脉冲。
    pub speech_frames: usize,
    /// 连续多少帧静音才认为「说完了」。600ms 左右比较自然：
    /// 太短会把句中停顿当成结束，太长则回答迟迟不来。
    pub silence_frames: usize,
    /// TTS 播放期间的阈值倍数。用来压掉「听见自己」导致的误打断。
    /// 调太高会打断不灵，调太低会自己打断自己。
    pub playback_threshold_scale: f32,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            energy_threshold: 0.012,
            // 3 帧 = 60ms
            speech_frames: 3,
            // 45 帧 = 900ms。
            //
            // 原来是 30（600ms），用户反馈"经常话还没说完就被认为说完了"。
            // 600ms 对**停下来想词**这种正常停顿来说太短 —— 换 Silero 只能解决
            // "低音量被判成静音"，停顿本身该等多久是另一件事，必须一起放宽。
            //
            // 代价是每轮多等 300ms 才开始识别。可以在设置里调回去 ——
            // 打断的响应速度不受影响，那是 speech_frames 管的（60ms）
            silence_frames: 45,
            playback_threshold_scale: 3.5,
        }
    }
}

/// 基于短时能量 + 零穿越率的 VAD。
///
/// 加零穿越率是为了区分「浊音/元音」（低 ZCR、高能量）和「摩擦噪声」
/// （高 ZCR、能量也可能不低，比如空调风声、纸张摩擦）。只看能量的话，
/// 稳定的环境噪声很容易被误判成一直在说话。
///
/// **已知局限**：底噪只在非语音帧更新（否则会把人声本身学进底噪）。因此如果
/// 稳定噪声的能量本来就超过 [`VadConfig::energy_threshold`]（例如旁边一台一直
/// 响的风扇），它会被持续判成语音，自适应阈值也纠正不了。采集层用「单句最长
/// 时长」兜底，保证不会无限增长；根治要换成架构文档规划的 Silero VAD (ONNX)，
/// 这也是 [`Vad`] trait 存在的原因。
pub struct EnergyVad {
    config: VadConfig,
    /// 当前是否处于「语音中」
    in_speech: bool,
    /// 连续超阈值帧数
    voiced_run: usize,
    /// 连续静音帧数
    silent_run: usize,
    /// 环境噪声底噪估计，用于自适应阈值
    noise_floor: f32,
    /// 阈值缩放系数，播放 TTS 时被调高
    threshold_scale: f32,
}

impl EnergyVad {
    pub fn new(config: VadConfig) -> Self {
        Self {
            config,
            in_speech: false,
            voiced_run: 0,
            silent_run: 0,
            noise_floor: 0.0,
            threshold_scale: 1.0,
        }
    }

    /// 有效阈值 = 配置阈值与「底噪 + 余量」取大者。
    ///
    /// 固定阈值在安静房间里偏高（说话轻一点就检测不到），在嘈杂环境里偏低
    /// （一直误触发）。所以跟着底噪走。
    fn effective_threshold(&self) -> f32 {
        (self.noise_floor * 2.5).max(self.config.energy_threshold) * self.threshold_scale
    }

    fn rms(frame: &[f32]) -> f32 {
        if frame.is_empty() {
            return 0.0;
        }
        let sum: f32 = frame.iter().map(|s| s * s).sum();
        (sum / frame.len() as f32).sqrt()
    }

    /// 零穿越率：相邻采样点符号变化的比例
    fn zero_crossing_rate(frame: &[f32]) -> f32 {
        if frame.len() < 2 {
            return 0.0;
        }
        let crossings = frame
            .windows(2)
            .filter(|w| (w[0] >= 0.0) != (w[1] >= 0.0))
            .count();
        crossings as f32 / (frame.len() - 1) as f32
    }
}

impl Vad for EnergyVad {
    fn process(&mut self, frame: &[f32]) -> VadEvent {
        let rms = Self::rms(frame);
        let zcr = Self::zero_crossing_rate(frame);

        // ZCR 过高基本可以断定不是人声的浊音段（0.5 对应接近每两点一变号，
        // 那是噪声或高频摩擦音的特征）
        let voiced = rms > self.effective_threshold() && zcr < 0.5;

        if voiced {
            self.voiced_run += 1;
            self.silent_run = 0;
        } else {
            self.silent_run += 1;
            self.voiced_run = 0;
            // 只在非语音帧更新底噪，避免把人声本身学进底噪里。
            // 指数滑动平均，系数取小值让底噪只跟随长期变化。
            self.noise_floor = if self.noise_floor == 0.0 {
                rms
            } else {
                self.noise_floor * 0.95 + rms * 0.05
            };
        }

        if self.in_speech {
            if self.silent_run >= self.config.silence_frames {
                self.in_speech = false;
                self.silent_run = 0;
                VadEvent::SpeechEnd
            } else {
                VadEvent::Speech
            }
        } else if self.voiced_run >= self.config.speech_frames {
            self.in_speech = true;
            VadEvent::SpeechStart
        } else {
            VadEvent::Silence
        }
    }

    fn set_threshold_scale(&mut self, scale: f32) {
        // 下限 1.0：只允许调得更钝，不允许调得比配置更灵敏，
        // 否则回声抑制会被误用成「提高灵敏度」
        self.threshold_scale = scale.max(1.0);
    }

    fn reset(&mut self) {
        self.in_speech = false;
        self.voiced_run = 0;
        self.silent_run = 0;
        // 底噪估计刻意保留：换一轮对话环境噪声通常没变，
        // 重新学一遍反而会在开头几帧误判。
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    /// 造一帧正弦波当作「浊音」：能量高、零穿越率低
    fn voiced_frame(amplitude: f32) -> Vec<f32> {
        (0..FRAME_SAMPLES)
            .map(|i| {
                // 200Hz @ 16kHz，接近男声基频，一帧内 ZCR 很低
                (i as f32 * TAU * 200.0 / SAMPLE_RATE as f32).sin() * amplitude
            })
            .collect()
    }

    fn silent_frame() -> Vec<f32> {
        vec![0.0; FRAME_SAMPLES]
    }

    /// 交替正负的高频信号：能量高但 ZCR 也极高，应当被判为非语音
    fn noise_frame(amplitude: f32) -> Vec<f32> {
        (0..FRAME_SAMPLES)
            .map(|i| if i % 2 == 0 { amplitude } else { -amplitude })
            .collect()
    }

    #[test]
    fn detects_speech_start_after_enough_voiced_frames() {
        let mut vad = EnergyVad::new(VadConfig::default());
        let frame = voiced_frame(0.3);

        // 前两帧还在观察期，不该急着报开口
        assert_eq!(vad.process(&frame), VadEvent::Silence);
        assert_eq!(vad.process(&frame), VadEvent::Silence);
        // 第三帧满足 speech_frames = 3
        assert_eq!(vad.process(&frame), VadEvent::SpeechStart);
        assert_eq!(vad.process(&frame), VadEvent::Speech);
    }

    #[test]
    fn reports_speech_end_only_after_silence_timeout() {
        let mut vad = EnergyVad::new(VadConfig::default());
        let voiced = voiced_frame(0.3);
        for _ in 0..5 {
            vad.process(&voiced);
        }

        let silent = silent_frame();
        // 帧数从配置读，别写死 —— 写死的话调超时会让这条无端变红，
        // 而它想验的是"到点才结束"这个行为，不是那个具体的数
        let timeout = VadConfig::default().silence_frames;
        for i in 0..timeout - 1 {
            assert_eq!(
                vad.process(&silent),
                VadEvent::Speech,
                "第 {i} 帧静音（共需 {timeout} 帧）就结束了，句中停顿会被切断"
            );
        }
        assert_eq!(vad.process(&silent), VadEvent::SpeechEnd);
    }

    #[test]
    fn single_impulse_does_not_trigger_speech() {
        // 键盘敲击这类单帧脉冲不该触发 barge-in
        let mut vad = EnergyVad::new(VadConfig::default());
        assert_eq!(vad.process(&voiced_frame(0.9)), VadEvent::Silence);
        assert_eq!(vad.process(&silent_frame()), VadEvent::Silence);
        assert_eq!(vad.process(&voiced_frame(0.9)), VadEvent::Silence);
        assert_eq!(vad.process(&silent_frame()), VadEvent::Silence);
    }

    #[test]
    fn high_zero_crossing_noise_is_not_speech() {
        // 能量远超阈值，但 ZCR 极高 —— 是噪声不是人声
        let mut vad = EnergyVad::new(VadConfig::default());
        let noise = noise_frame(0.5);
        for _ in 0..20 {
            assert_eq!(vad.process(&noise), VadEvent::Silence);
        }
    }

    #[test]
    fn silence_stays_silent() {
        let mut vad = EnergyVad::new(VadConfig::default());
        for _ in 0..100 {
            assert_eq!(vad.process(&silent_frame()), VadEvent::Silence);
        }
    }

    #[test]
    fn adapts_threshold_to_noise_floor() {
        let mut vad = EnergyVad::new(VadConfig::default());

        // 环境噪声要低于固定阈值，底噪才学得到（见 EnergyVad 文档里的局限说明）。
        // 幅度 0.01 的正弦 RMS ≈ 0.0071 < 0.012。
        let ambient = voiced_frame(0.01);
        for _ in 0..200 {
            assert_eq!(vad.process(&ambient), VadEvent::Silence);
        }
        // 底噪学到约 0.0071，有效阈值被抬到 0.0071×2.5 ≈ 0.0177
        assert!(vad.effective_threshold() > 0.015);

        // 幅度 0.024 的正弦 RMS ≈ 0.017：**超过**固定阈值 0.012，
        // 但没超过抬高后的阈值 —— 这正是自适应起作用的证据
        let borderline = voiced_frame(0.024);
        for _ in 0..5 {
            assert_eq!(vad.process(&borderline), VadEvent::Silence);
        }

        // 明显更大的声音仍然要能检出
        let loud = voiced_frame(0.4);
        vad.process(&loud);
        vad.process(&loud);
        assert_eq!(vad.process(&loud), VadEvent::SpeechStart);
    }

    #[test]
    fn steady_noise_above_fixed_threshold_is_a_known_limitation() {
        // 记录已知局限，避免将来有人以为这是 bug：
        // 底噪只在**非语音帧**更新（否则会把人声学进底噪），所以一旦稳定噪声
        // 本身就超过固定阈值，它会被一直判成语音，自适应也救不回来。
        // 真实场景就是「旁边一台一直响的风扇」。根治办法是换 Silero VAD。
        let mut vad = EnergyVad::new(VadConfig::default());
        let loud_ambient = voiced_frame(0.1); // RMS ≈ 0.071 ≫ 0.012

        vad.process(&loud_ambient);
        vad.process(&loud_ambient);
        assert_eq!(
            vad.process(&loud_ambient),
            VadEvent::SpeechStart,
            "这是当前实现的已知行为，不是期望行为"
        );
        // 兜底在采集层：单句超过 MAX_UTTERANCE_SECONDS 会被强制切断，
        // 不会无限增长把内存吃光。
    }
}

#[cfg(test)]
mod scale_tests {
    use super::*;
    use std::f32::consts::TAU;

    fn tone(amplitude: f32) -> Vec<f32> {
        (0..FRAME_SAMPLES)
            .map(|i| (i as f32 * TAU * 200.0 / SAMPLE_RATE as f32).sin() * amplitude)
            .collect()
    }

    #[test]
    fn raising_scale_suppresses_borderline_speech() {
        let mut vad = EnergyVad::new(VadConfig::default());
        // 幅度 0.05 的正弦 RMS ≈ 0.035，正常灵敏度下算语音
        let frame = tone(0.05);
        vad.process(&frame);
        vad.process(&frame);
        assert_eq!(vad.process(&frame), VadEvent::SpeechStart);

        // 换一个实例，播放期间的阈值倍数下同样的输入不该触发
        let mut vad = EnergyVad::new(VadConfig::default());
        vad.set_threshold_scale(VadConfig::default().playback_threshold_scale);
        for _ in 0..10 {
            assert_eq!(vad.process(&frame), VadEvent::Silence);
        }
    }

    #[test]
    fn loud_speech_still_interrupts_during_playback() {
        // 收紧阈值不能把真正的打断也挡掉 —— 用户提高音量必须能打断
        let mut vad = EnergyVad::new(VadConfig::default());
        vad.set_threshold_scale(VadConfig::default().playback_threshold_scale);
        let loud = tone(0.5);
        vad.process(&loud);
        vad.process(&loud);
        assert_eq!(vad.process(&loud), VadEvent::SpeechStart);
    }

    #[test]
    fn scale_below_one_is_clamped() {
        let mut vad = EnergyVad::new(VadConfig::default());
        vad.set_threshold_scale(0.1);
        assert_eq!(vad.threshold_scale, 1.0, "不允许调得比配置更灵敏");
    }
}
