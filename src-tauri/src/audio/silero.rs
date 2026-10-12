//! Silero VAD (v5, ONNX) 实现的语音活动检测。
//!
//! ## 为什么换掉能量 VAD
//!
//! 用户的原话是「经常话还没说完，系统就认为我说完了，而且这个问题很严重」。
//!
//! 能量 + 零穿越率那套的硬伤是：**说话时音量偏低的部分会被判成静音** ——
//! 尾音、轻辅音、句中换气、以及本来就说得轻的时候。一旦被判成静音，
//! 600ms 的结束计时器就提前开始跑，于是话说到一半就被切断。
//!
//! Silero 是学出来的语音检测器，对低音量语音稳得多，也不会把风扇、空调、
//! 键盘声当成人说话（那是能量 VAD 的另一个硬伤，底噪自适应也纠正不了）。
//!
//! ## 两件事要分开看
//!
//! 换 Silero 只治「低音量被当成静音」。而「停下来想词就被切断」是**超时长度**
//! 的问题 —— 同样的 600ms 配上 Silero 照样会切断思考中的停顿。
//! 所以默认超时也一起放宽了，见 [`super::vad::VadConfig`]。
//!
//! ## 窗口对不齐这件事要小心
//!
//! Silero v5 只吃**正好 512 个采样**（16kHz 下 32ms），而管线的帧是 320 个
//! 采样（20ms，为了打断判定及时）。两者不是整数倍，所以这里攒一个环形缓冲：
//! 每来一帧就攒进去，够 512 就推一次，不够就沿用上一次的判定。
//!
//! **判定的时间粒度仍然是 20ms 一帧** —— 上层的 `speech_frames` /
//! `silence_frames` 是按帧计数的，粒度一变，那两个配置的含义就悄悄改了。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ort::session::Session;
use ort::value::Value;

use super::vad::{Vad, VadConfig, VadEvent, FRAME_SAMPLES, SAMPLE_RATE};
use crate::error::{Error, Result};

/// Silero v5 的输入窗口长度（16kHz）。这是模型规定的，不能改
const WINDOW: usize = 512;

/// **喂给模型的不是 512，而是 64 + 512 = 576。**
///
/// v5 要求调用方自己保留上一窗末尾的 64 个采样当上下文，拼在当前窗前面
/// （官方 Python 封装里那个 `self._context`）。少了它，模型内部的 STFT
/// 算出来的是垃圾 —— 而输入形状是动态的（`[-1, -1]`），所以**不报任何错**，
/// 只是概率恒定在 0.0005 左右，跟输入完全无关。
///
/// 实测（同一个模型、同一段音频）：
///
/// | 输入 | 只喂 512 | 拼上 64 上下文 |
/// |---|---|---|
/// | 静音 | 0.0005 | 0.0052 |
/// | 0.5 幅正弦 | 0.0008 | 0.5327 |
///
/// 这就是"说话完全没反应"的原因。
const CONTEXT: usize = 64;

/// 状态张量的元素个数。形状是 [2, 1, 128] —— v4 是 h/c 两个 [2,1,64]，
/// v5 合成了一个，**别混**
const STATE_LEN: usize = 2 * 128;

/// 判定为「开始说话」的概率阈值
const START_PROB: f32 = 0.5;
/// 判定为「仍在说话」的概率阈值。
///
/// 比 START 低是刻意的（滞回）：说话过程中概率会正常地上下波动，
/// 用同一个阈值会让状态在边界反复翻转，而每次翻转都会重置静音计数 ——
/// 表现就是断句时机飘忽。
const CONTINUE_PROB: f32 = 0.35;

/// 模型文件名
pub const MODEL_FILE: &str = "silero_vad.onnx";

/// 模型放在哪。与 Whisper 同一个目录（`<配置目录>/models/`）
pub fn model_path(config_dir: &Path) -> PathBuf {
    config_dir.join("models").join(MODEL_FILE)
}

/// 把每帧的「是不是语音」变成 [`VadEvent`]。
///
/// 单独抽出来是为了能测：这段状态机的错法（漏报开口、提前报结束）
/// 在真实使用中只表现成"打断不灵"或"话没说完就被切"，反推不到原因。
/// 而它本身是纯逻辑，不需要模型。
#[derive(Debug)]
pub struct SpeechGate {
    speech_frames: usize,
    silence_frames: usize,
    voiced_run: usize,
    silent_run: usize,
    in_speech: bool,
}

impl SpeechGate {
    pub fn new(config: &VadConfig) -> Self {
        Self {
            speech_frames: config.speech_frames.max(1),
            silence_frames: config.silence_frames.max(1),
            voiced_run: 0,
            silent_run: 0,
            in_speech: false,
        }
    }

    pub fn reset(&mut self) {
        self.voiced_run = 0;
        self.silent_run = 0;
        self.in_speech = false;
    }

    /// 喂一帧的判定结果
    pub fn push(&mut self, voiced: bool) -> VadEvent {
        if voiced {
            self.voiced_run += 1;
            self.silent_run = 0;
        } else {
            self.silent_run += 1;
            self.voiced_run = 0;
        }

        if self.in_speech {
            if self.silent_run >= self.silence_frames {
                self.in_speech = false;
                self.voiced_run = 0;
                VadEvent::SpeechEnd
            } else {
                VadEvent::Speech
            }
        } else if self.voiced_run >= self.speech_frames {
            self.in_speech = true;
            self.silent_run = 0;
            VadEvent::SpeechStart
        } else {
            VadEvent::Silence
        }
    }
}

/// Silero VAD。
///
/// `Session` 用 Mutex 包着：`Vad::process` 拿的是 `&mut self`，本来不需要锁，
/// 但 ort 的 Session 在某些 feature 组合下不是 `Sync`，而 `Vad` 要求 `Send`。
/// 用 Mutex 是最省事且没有额外开销的做法（永远只有一个持有者）。
pub struct SileroVad {
    session: Mutex<Session>,
    /// 攒够 512 个采样才能推一次
    buffer: Vec<f32>,
    state: Vec<f32>,
    /// 上一窗末尾的 64 个采样。见 [`CONTEXT`] —— 少了它模型等于瞎的
    context: Vec<f32>,
    /// 上一次推理的结论。窗口还没攒够时沿用它
    last_voiced: bool,
    gate: SpeechGate,
    threshold_scale: f32,
    /// 播放期间的音量闸门要用的底噪估计。跟能量 VAD 同一套：
    /// 只在非语音帧更新，指数滑动平均
    noise_floor: f32,
    /// 音量闸门的下限，来自 `VadConfig::energy_threshold`
    energy_threshold: f32,
    /// 最近一次推理的概率。诊断用 —— "说话没反应"的时候，
    /// 唯一有用的数字就是它
    last_prob: f32,
}

impl SileroVad {
    /// 最近一次推理给出的"是语音"的概率。诊断用（`examples/vad-probe.rs`）
    pub fn last_probability(&self) -> f32 {
        self.last_prob
    }

    /// 加载模型。文件不在就报错 —— 调用方负责退回能量 VAD。
    pub fn load(path: &Path, config: &VadConfig) -> Result<Self> {
        if !path.is_file() {
            return Err(Error::config(format!(
                "找不到 Silero 模型：{}",
                path.display()
            )));
        }
        // 自己读文件再用 commit_from_memory，而不是 commit_from_file ——
        // 后者被 ort 的 feature 门着（我们关了默认特性）。自己读还有个好处：
        // 读不出来时的错误信息里有真实路径
        let bytes = std::fs::read(path)
            .map_err(|e| Error::config(format!("读取 {} 失败：{e}", path.display())))?;
        let mut builder =
            Session::builder().map_err(|e| Error::config(format!("创建 ONNX 会话失败：{e}")))?;
        let session = builder
            .commit_from_memory(&bytes)
            .map_err(|e| Error::config(format!("加载 Silero 模型失败：{e}")))?;

        Ok(Self {
            session: Mutex::new(session),
            buffer: Vec::with_capacity(WINDOW + FRAME_SAMPLES),
            state: vec![0.0; STATE_LEN],
            context: vec![0.0; CONTEXT],
            last_voiced: false,
            gate: SpeechGate::new(config),
            threshold_scale: 1.0,
            noise_floor: 0.0,
            energy_threshold: config.energy_threshold,
            last_prob: 0.0,
        })
    }

    /// 推一个 512 采样的窗口，返回是语音的概率
    fn infer(&mut self, window: &[f32]) -> Result<f32> {
        // 拼上上一窗的尾巴。**这一步不能省** —— 见 CONTEXT 的注释
        let mut framed = Vec::with_capacity(CONTEXT + WINDOW);
        framed.extend_from_slice(&self.context);
        framed.extend_from_slice(window);

        if std::env::var_os("ANIMA_VAD_DEBUG").is_some() {
            let rms = (window.iter().map(|v| v * v).sum::<f32>() / window.len() as f32).sqrt();
            eprintln!("  [窗口] 送入 {} 采样 rms {rms:.4}", framed.len());
        }

        let audio = Value::from_array(([1_usize, framed.len()], framed))
            .map_err(|e| Error::config(format!("构造音频张量失败：{e}")))?;
        let state = Value::from_array(([2_usize, 1, 128], self.state.clone()))
            .map_err(|e| Error::config(format!("构造状态张量失败：{e}")))?;
        // **sr 必须是标量（rank 0），不是形状 [1]。**
        //
        // v5 内部有个 `If` 节点，靠 `sr == 16000` 决定走 16k 还是 8k 分支，
        // 而 8k 分支期待的窗口是 256 采样。形状不对时那个比较不成立，
        // 于是走了 8k 分支去解 512 个采样 —— **不报任何错**，只是概率恒等于
        // 0.0005 左右。实测就是这样：真实语音 398 帧全判成静音。
        let sr = Value::from_array(([] as [usize; 0], vec![SAMPLE_RATE as i64]))
            .map_err(|e| Error::config(format!("构造采样率张量失败：{e}")))?;

        let mut session = self
            .session
            .lock()
            .map_err(|_| Error::config("Silero 会话锁被污染".to_string()))?;
        let outputs = session
            .run(ort::inputs![
                "input" => audio,
                "state" => state,
                "sr" => sr,
            ])
            .map_err(|e| Error::config(format!("Silero 推理失败：{e}")))?;

        // **状态必须回写。** Silero 是有状态的：不回写等于每 32ms 重新开始判断，
        // 连续语音会被切成一段一段，表现又回到"话没说完就被切"
        let (_, next_state) = outputs["stateN"]
            .try_extract_tensor::<f32>()
            .map_err(|e| Error::config(format!("读取状态失败：{e}")))?;
        self.state.clear();
        self.state.extend_from_slice(next_state);

        let (_, prob) = outputs["output"]
            .try_extract_tensor::<f32>()
            .map_err(|e| Error::config(format!("读取概率失败：{e}")))?;
        let prob = prob.first().copied().unwrap_or(0.0);

        // 记下这一窗的尾巴，给下一窗当上下文
        self.context.clear();
        self.context
            .extend_from_slice(&window[window.len().saturating_sub(CONTEXT)..]);
        Ok(prob)
    }
}

impl SileroVad {
    /// 播放期间的音量闸门。`threshold_scale == 1.0`（没在放）时恒为真 ——
    /// 平时完全交给 Silero，不要让能量判定回来压低灵敏度。
    fn loud_enough(&self, rms: f32) -> bool {
        if self.threshold_scale <= 1.0 {
            return true;
        }
        rms >= (self.noise_floor * 2.5).max(self.energy_threshold) * self.threshold_scale
    }
}

/// 一窗的均方根音量
fn rms_of(window: &[f32]) -> f32 {
    if window.is_empty() {
        return 0.0;
    }
    (window.iter().map(|s| s * s).sum::<f32>() / window.len() as f32).sqrt()
}

impl Vad for SileroVad {
    fn process(&mut self, frame: &[f32]) -> VadEvent {
        self.buffer.extend_from_slice(frame);

        // 攒够就推。可能一帧攒不满，也可能一次攒出多个窗口
        while self.buffer.len() >= WINDOW {
            let window: Vec<f32> = self.buffer.drain(..WINDOW).collect();
            let rms = rms_of(&window);
            match self.infer(&window) {
                Ok(prob) => {
                    self.last_prob = prob;
                    let speechlike = if self.last_voiced {
                        prob >= CONTINUE_PROB
                    } else {
                        prob >= START_PROB
                    };
                    // 播放期间还要过音量闸门 —— **概率闸门在这里没用**。
                    //
                    // 原先是把 START_PROB 乘 threshold_scale（3.5 → 0.95），
                    // 照搬了能量 VAD 的写法。但那套语义换到 Silero 就不成立了：
                    // 能量 VAD 的阈值是**音量**，乘 3.5 意思是"得响 3.5 倍"；
                    // Silero 输出的是**"这是不是语音"的概率**，而扬声器里放出来的
                    // TTS 恰好是最标准的语音 —— 实测它给自己的合成语音打
                    // 1.0000，0.95 的门槛形同不存在。表现就是 Anima 被自己的
                    // 声音打断、然后回答一句没人说过的话。
                    //
                    // 所以判"是不是语音"仍然交给 Silero（它比能量准得多），
                    // 而"是不是**你**在说"这一层重新用音量来判：麦克风漏进来的
                    // 回声比人对着麦克风说话弱得多，这个差别是物理的。
                    self.last_voiced = speechlike && self.loud_enough(rms);
                }
                Err(err) => {
                    // 推理失败不能让麦克风哑掉。保持上一次的结论，
                    // 让上层的状态机继续走
                    tracing::warn!(%err, "Silero 推理失败，沿用上一帧的判定");
                }
            }

            if !self.last_voiced {
                // 只在非语音帧更新，避免把人声本身学进底噪
                self.noise_floor = if self.noise_floor == 0.0 {
                    rms
                } else {
                    self.noise_floor * 0.95 + rms * 0.05
                };
            }
        }

        self.gate.push(self.last_voiced)
    }

    fn reset(&mut self) {
        self.buffer.clear();
        // 状态和上下文都要清零：上一句的尾音不该影响下一句的起判
        self.state.iter_mut().for_each(|v| *v = 0.0);
        self.context.clear();
        self.context.resize(CONTEXT, 0.0);
        self.last_voiced = false;
        self.gate.reset();
    }

    fn set_threshold_scale(&mut self, scale: f32) {
        self.threshold_scale = scale.max(0.1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> VadConfig {
        VadConfig {
            speech_frames: 3,
            silence_frames: 5,
            ..Default::default()
        }
    }

    #[test]
    fn gate_needs_consecutive_voiced_frames_to_start() {
        // 单帧噪声不该触发开口 —— 否则 barge-in 会被咳嗽声误触
        let mut gate = SpeechGate::new(&config());
        assert_eq!(gate.push(true), VadEvent::Silence);
        assert_eq!(gate.push(true), VadEvent::Silence);
        assert_eq!(gate.push(true), VadEvent::SpeechStart);
    }

    #[test]
    fn a_single_quiet_frame_does_not_end_the_utterance() {
        // **这条正对着用户的抱怨。** 说话中间的一帧低音量不能结束整句
        let mut gate = SpeechGate::new(&config());
        for _ in 0..3 {
            gate.push(true);
        }
        for _ in 0..4 {
            assert_eq!(gate.push(false), VadEvent::Speech, "还没到超时就不能结束");
        }
        assert_eq!(gate.push(false), VadEvent::SpeechEnd);
    }

    #[test]
    fn speaking_again_resets_the_silence_timer() {
        // 停顿之后又开口，计时器必须重新开始 —— 否则累计静音会提前触发结束
        let mut gate = SpeechGate::new(&config());
        for _ in 0..3 {
            gate.push(true);
        }
        for _ in 0..4 {
            gate.push(false);
        }
        assert_eq!(gate.push(true), VadEvent::Speech);
        for _ in 0..4 {
            assert_eq!(gate.push(false), VadEvent::Speech, "计时器应当被重置过");
        }
        assert_eq!(gate.push(false), VadEvent::SpeechEnd);
    }

    #[test]
    fn end_then_start_again() {
        let mut gate = SpeechGate::new(&config());
        for _ in 0..3 {
            gate.push(true);
        }
        for _ in 0..5 {
            gate.push(false);
        }
        // 结束之后要能重新开口
        assert_eq!(gate.push(true), VadEvent::Silence);
        assert_eq!(gate.push(true), VadEvent::Silence);
        assert_eq!(gate.push(true), VadEvent::SpeechStart);
    }

    /// **这条才是真正的判据，也是这次 bug 的回归测试。**
    ///
    /// 少了 64 采样的上下文时，模型的输出**恒定在 0.0005 左右、和输入完全无关**
    /// （静音 0.0005、真实语音 0.0005）。而它不报任何错 —— 输入形状是动态的，
    /// ONNX 照单全收。表现就是"说话完全没反应"。
    ///
    /// 而我原先只写了「静音不该被判成语音」那一条 —— **VAD 完全瞎掉的时候
    /// 那条照样通过**。一个在功能全废时仍然绿的测试，等于没有测试。
    ///
    /// 判据必须是**真实语音**：固件是本机 TTS 合的一句话，1.6 秒，进版本库
    /// （约 50KB）。不能用正弦或噪声代替 —— Silero 学的是人声，
    /// 实测正弦在这条链路上概率反而比静音更低。
    #[test]
    fn real_speech_is_detected_with_the_real_model() {
        let Some(mut vad) = real_model() else { return };
        let wav = match std::fs::read("tests/fixtures/speech-16k.wav") {
            Ok(b) => b,
            Err(_) => {
                eprintln!("跳过：缺 tests/fixtures/speech-16k.wav");
                return;
            }
        };
        let (samples, rate) = crate::audio::playback::decode_mono(wav).expect("解码固件失败");
        assert_eq!(rate, SAMPLE_RATE, "固件必须是 16kHz");

        let mut started = false;
        let mut speech = 0usize;
        let mut total = 0usize;
        for frame in samples.chunks(FRAME_SAMPLES) {
            if frame.len() < FRAME_SAMPLES {
                break;
            }
            total += 1;
            match vad.process(frame) {
                VadEvent::SpeechStart => {
                    started = true;
                    speech += 1;
                }
                VadEvent::Speech => speech += 1,
                _ => {}
            }
        }

        assert!(
            started,
            "整段真实语音里一次都没检测到开口 —— VAD 是瞎的。\
             先看 CONTEXT 那段注释"
        );
        let ratio = speech as f32 / total as f32;
        assert!(
            ratio > 0.6,
            "只有 {:.0}% 的帧被判成语音（{speech}/{total}）—— 说话会被切碎",
            ratio * 100.0
        );
    }

    /// **回声自激的回归测试。**
    ///
    /// 场景：Anima 正在说话，它自己的声音漏回麦克风。那是一段**衰减了的真实
    /// 语音** —— Silero 会给它很高的概率（实测对自己的 TTS 输出打 1.0000），
    /// 所以光靠概率闸门拦不住。原先把概率阈值乘 3.5（0.5→0.95）就以为防住了，
    /// 实际每一句都会被自己打断，然后模型去回答一句没人说过的话。
    ///
    /// 判据：同一段语音，衰减到 5%（相当于耳机/扬声器漏音）——
    /// 播放期间不该触发开口，而正常时必须触发。**两个断言都要有**：
    /// 只断言"播放时不触发"的话，把闸门写成恒假也能通过。
    #[test]
    fn attenuated_self_audio_does_not_barge_in_during_playback() {
        let wav = match std::fs::read("tests/fixtures/speech-16k.wav") {
            Ok(b) => b,
            Err(_) => {
                eprintln!("跳过：缺 tests/fixtures/speech-16k.wav");
                return;
            }
        };
        let (samples, _) = crate::audio::playback::decode_mono(wav).expect("解码固件失败");
        let leaked: Vec<f32> = samples.iter().map(|s| s * 0.05).collect();

        let started = |vad: &mut SileroVad, input: &[f32]| {
            input
                .chunks(FRAME_SAMPLES)
                .filter(|f| f.len() == FRAME_SAMPLES)
                .any(|f| vad.process(f) == VadEvent::SpeechStart)
        };

        let Some(mut playing) = real_model() else {
            return;
        };
        playing.set_threshold_scale(VadConfig::default().playback_threshold_scale);
        assert!(
            !started(&mut playing, &leaked),
            "播放期间漏回来的自己的声音把自己打断了 —— 概率闸门对自己的 TTS 无效，\
             音量闸门（loud_enough）没有生效"
        );

        let Some(mut idle) = real_model() else { return };
        assert!(
            started(&mut idle, &leaked),
            "没在播放时，这段（虽然轻）语音必须能触发开口 —— \
             音量闸门不该在平时压低灵敏度"
        );
    }

    /// 加载真实模型。不在就返回 None（跳过测试）——
    /// 它不进版本库（2.3MB 的可下载第三方资产）
    fn real_model() -> Option<SileroVad> {
        let dir = dirs::config_dir()?.join("com.anima.desktop");
        let path = model_path(&dir);
        if !path.is_file() {
            eprintln!("跳过：还没有 {}", path.display());
            return None;
        }
        Some(SileroVad::load(&path, &config()).expect("加载模型失败"))
    }

    /// 纯静音不该被判成语音。
    ///
    /// 这一条**单独不足以说明 VAD 是好的**（见上面那条），但它守另一个方向：
    /// 阈值设得过低、或者底噪被当成语音。
    #[test]
    fn silence_is_not_speech_with_the_real_model() {
        let Some(mut vad) = real_model() else { return };

        // 三秒纯静音
        let frame = [0.0f32; FRAME_SAMPLES];
        for i in 0..150 {
            let event = vad.process(&frame);
            assert_eq!(event, VadEvent::Silence, "第 {i} 帧静音被判成了 {event:?}");
        }
    }
}
