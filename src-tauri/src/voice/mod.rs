//! 实时语音对话管线。
//!
//! 把 VAD → STT → LLM → TTS 串成一条**随时可打断**的链路：
//!
//! ```text
//!   麦克风 ──VAD──> 一句话 ──Whisper──> 文字 ──LLM(流式)──> 分句 ──TTS──> 播放
//!              │                                  ▲                      │
//!              └── SpeechStart（用户开口）─────────┴── 取消 ────────────┘
//! ```
//!
//! ## 为什么写成 actor
//!
//! 状态（当前在听 / 在想 / 在说）、对话历史、在途的取消令牌，这几样东西被
//! 音频线程、LLM 流、TTS 任务同时碰。按 ARCHITECTURE.md 坑点 3 的要求，这里
//! 用单一任务独占所有可变状态，外部只能通过 channel 发命令 —— 没有共享可变
//! 状态，也就没有竞态。
//!
//! ## 打断（barge-in）怎么做到的
//!
//! 收到 [`CaptureEvent::SpeechStart`] 时同时做三件事，缺一件都会出问题：
//!   1. `CancellationToken` 取消 —— 停掉正在生成的 LLM 流（否则模型还在回答上一轮）
//!   2. `Player::interrupt()` —— 清空已排队待播的音频（否则还会听到上一轮的后半句）
//!   3. 标记本轮作废 —— 已经合成好但还没入队的 TTS 不再入队
//!
//! ## 与前端的契约
//!
//! 管线不依赖 Tauri：事件通过构造时传入的回调发出，由 `commands.rs` 转成
//! Tauri event。这样管线可以在测试里脱离 Tauri 跑。事件与命令的完整清单见
//! `docs/VOICE_API.md`。

pub mod initiative;
pub mod mode;
pub mod timer;
pub use mode::{InteractionMode, ModeProfile};

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

use crate::audio::capture::{self, CaptureEvent};
use crate::audio::playback::{PlaybackSignal, Player, SentenceSplitter};
use crate::audio::vad::VadConfig;
use crate::config::Tier;
use crate::error::{Error, Result};
use crate::llm::gateway::Gateway;
use crate::llm::{ChatChunk, ChatRequest, Message, ThinkingMode};
use crate::memory::Memory;
use crate::stt::SpeechToText;
use crate::tts::{TextToSpeech, TtsConfig, WordBoundary};

/// 管线当前状态。前端据此显示「正在听 / 正在想 / 正在说」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceState {
    /// 未启动
    Idle,
    /// 在听用户说话
    Listening,
    /// 在识别刚说完的那句
    Transcribing,
    /// 在等模型生成
    Thinking,
    /// 在播放回答
    Speaking,
}

/// 管线向外抛的事件
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VoiceEvent {
    State {
        state: VoiceState,
    },
    /// 麦克风实时音量（RMS 0..1）。给电子云粒子面容和电平表用。
    /// 频率已在采集侧节流到约 10Hz。
    /// 麦克风输入电平（界面右侧那条谱线）
    Level {
        rms: f32,
    },
    /// TTS 播放输出电平（界面输入框上方那条谱线）。
    ///
    /// 与 `Level` 分开是因为二者语义不同：一条是「我说了多大声」，
    /// 一条是「AI 说了多大声」，混在一个字段里界面就没法分别驱动。
    OutputLevel {
        rms: f32,
    },
    /// 识别出的用户话语
    Transcript {
        text: String,
    },
    /// 助手回答的文本增量
    AssistantDelta {
        delta: String,
    },
    /// 模型的思考过程增量（开着思考时才有）
    ReasoningDelta {
        delta: String,
    },
    /// 一句话的词边界时间戳，供数字人对口型。
    ///
    /// **`offset_ms` 的 t=0 是这一句开始出声的时刻，不是收到这条事件的时刻。**
    /// 这条事件在合成完成时就发（让前端有时间把嘴型时间线算好），而真正的
    /// t=0 由随后的 [`VoiceEvent::SentenceStart`] 给出 —— 靠 `id` 配对。
    Words {
        /// 与 `SentenceStart.id` 配对
        id: u64,
        words: Vec<WordBoundary>,
        /// 由 `words` 推出的嘴型时间线。前端在关键帧之间插值。
        ///
        /// 在 Rust 侧推而不是让前端自己推：见 `avatar::viseme` 的模块注释
        visemes: Vec<crate::avatar::viseme::VisemeKey>,
    },
    /// 带这个 id 的句子**开始出声了**。前端在这一刻把嘴型时间线的时钟归零。
    ///
    /// 为什么不能省掉它、直接拿收到 `Words` 的时刻当 t=0：句子是排队播的，
    /// 合成完的时候前一句往往还在播，两个时刻能差好几秒。
    /// 音频驱动的口型（`avatar::lipsync`）。和 `Visemes` 那条时间线互补：
    /// 有文本时走时间线（更准），没有时走这条
    AudioViseme {
        viseme: crate::avatar::viseme::Viseme,
        /// 张口幅度 0..1
        weight: f32,
    },
    SentenceStart {
        id: u64,
    },
    /// 本轮被用户打断
    /// 情感轴变化。界面用它驱动粒子云的形态与色调。
    ///
    /// 只在**一轮结束时**发，不是每帧 —— 轴值本来就只在轮次边界变化，
    /// 高频广播没有意义（高频数据走 event 会挤占通道，见 ARCHITECTURE.md 坑点 1）。
    Emotion {
        affection: f32,
        mood: f32,
        energy: f32,
        /// 关系档位。界面显示用；模型看到的是 personality::to_prompt 的文字
        familiarity: crate::personality::Familiarity,
        /// 焦躁强度 0..1。由 mood 与 energy 推出（低心情 + 高唤醒 = 生气），
        /// 不是独立的一条轴 —— 见 personality::irritation
        irritation: f32,
    },
    Interrupted,
    /// 一轮问答完整结束
    TurnComplete,
    /// 技能变了。前端在顶部状态那一行显示 marker，None 表示退出角色
    Skill {
        marker: Option<String>,
    },
    /// 动作
    #[serde(rename = "action")]
    Action {
        action: String,
    },
    /// 这一轮调了个工具，以及它的下场。
    ///
    /// **语音模式下必须有这条。** 原先语音链路执行工具时只发了一个空的
    /// `AssistantDelta`，工具调用对用户完全不可见 —— 于是模型说"终端已打开"
    /// 而实际没打开时，用户无从判断它到底调没调、还是调了但失败了。
    /// 实测撞到过：一次调了但参数错，一次压根没调，两次的回答一模一样。
    Tool {
        /// 与模型给的调用 id 一致，用来把「开始」和「结束」配对
        id: String,
        name: String,
        state: ToolState,
        /// 失败或被拦时的原因。成功时为 None —— 成功的结果会进对话，不用重复
        detail: Option<String>,
    },
    Error {
        message: String,
    },
}

/// 工具这一次的下场。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolState {
    /// 正在执行
    Running,
    /// 执行成功
    Ok,
    /// 执行了但失败了
    Failed,
    /// 没执行 —— 被安全策略拦下、或需要界面确认（语音模式下没法弹框）
    Blocked,
}

/// 事件回调。管线不认识 Tauri，由上层把它接到 event 广播上。
pub type EventSink = Arc<dyn Fn(VoiceEvent) + Send + Sync>;

/// 语音管线配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceConfig {
    /// 输入设备名。None = 系统默认
    #[serde(default)]
    pub input_device: Option<String>,
    /// 允不允许她主动开口（安静一阵之后搭话、任务做完回报）。
    ///
    /// **总开关必须存在。** 主动开口是最容易惹烦人的功能 ——
    /// 被烦到时用户要能一键让它彻底闭嘴，而不是去研究冷却时间怎么调。
    #[serde(default = "yes")]
    pub initiative: bool,
    #[serde(default)]
    pub vad: VadConfigSerde,
    #[serde(default)]
    pub tts: TtsConfig,
    /// 走哪个分层的模型
    #[serde(default = "default_tier")]
    pub tier: Tier,
    /// 语音链路是否关掉思考。默认**关掉** —— 语音场景下等 8 秒才出声太难受，
    /// 而文字对话可以另行选择（两者是独立开关）。
    #[serde(default = "default_true")]
    pub disable_thinking: bool,
    /// 送 TTS 前一句话的最小字数
    #[serde(default = "default_min_chars")]
    pub sentence_min_chars: usize,
}

/// serde 的默认值：主动开口默认开着（陪伴模式的核心之一）
fn yes() -> bool {
    true
}

fn default_tier() -> Tier {
    Tier::Realtime
}
fn default_true() -> bool {
    true
}
fn default_min_chars() -> usize {
    8
}

/// [`VadConfig`] 的可序列化镜像。
///
/// 单独一份是为了让 `VadConfig` 保持纯粹的音频层结构、不背 serde 依赖，
/// 同时又能存进 config.json 供用户调阈值。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VadConfigSerde {
    pub energy_threshold: f32,
    pub speech_frames: usize,
    pub silence_frames: usize,
    pub playback_threshold_scale: f32,
}

impl Default for VadConfigSerde {
    fn default() -> Self {
        let d = VadConfig::default();
        Self {
            energy_threshold: d.energy_threshold,
            speech_frames: d.speech_frames,
            silence_frames: d.silence_frames,
            playback_threshold_scale: d.playback_threshold_scale,
        }
    }
}

impl From<&VadConfigSerde> for VadConfig {
    fn from(v: &VadConfigSerde) -> Self {
        Self {
            energy_threshold: v.energy_threshold,
            speech_frames: v.speech_frames,
            silence_frames: v.silence_frames,
            playback_threshold_scale: v.playback_threshold_scale,
        }
    }
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            initiative: true,
            input_device: None,
            vad: VadConfigSerde::default(),
            tts: TtsConfig::default(),
            tier: Tier::Realtime,
            disable_thinking: true,
            sentence_min_chars: 8,
        }
    }
}

/// 外部对管线下的命令
enum Command {
    Stop,
    /// 手动打断（点界面上的停止按钮）
    Interrupt,
    /// 滚动摘要更新好了（后台任务送回来的）。
    SetSummary(crate::memory::summary::Summary),
    /// 换技能。None = 退出角色。
    SetSkills(Vec<crate::skill::Skill>),
    /// 换角色灵魂
    SetCharacter(Option<crate::character::Character>),
    /// 换 TTS 引擎。
    ///
    /// **引擎是会话启动时的一个快照**（`deps.tts` 是个 Arc），设置面板里切了
    /// 引擎不送这条命令的话，正在跑的会话会一直用旧引擎，直到重启 ——
    /// 表现就是"切到 Edge 了但声音没变"。
    SetTts {
        engine: Arc<dyn TextToSpeech>,
        /// 这个引擎自己的音色。**必须跟着引擎一起换** ——
        /// 音色名的命名空间每家一套，沿用上一个引擎的名字就是各种 404
        voice: String,
        /// 这个引擎认不认音色库里的名字（只有 GPT-SoVITS 认）。
        /// 不认的话，角色的情绪音色就不能往上套 —— 那些是参考音频的路径
        uses_voice_library: bool,
    },
    /// 角色绑的固定音色。None = 解绑，退回全局设置。
    ///
    /// **不写进全局 config**：写全局的话，换回默认形态之后声音还是角色那个，
    /// 而用户以为自己已经切回来了
    SetCharacterVoice(Option<String>),
    SetMode(InteractionMode),
    /// 定一个提醒。结果要回到 actor —— 生成任务持有的是 deps 的克隆
    AddTimer {
        minutes: f64,
        label: String,
    },
    /// None = 全部取消
    CancelTimer(Option<u64>),
}

/// 语音会话句柄。drop 会停止采集并结束 actor。
pub struct VoiceSession {
    commands: mpsc::UnboundedSender<Command>,
    state: Arc<Mutex<VoiceState>>,
}

impl VoiceSession {
    pub async fn state(&self) -> VoiceState {
        *self.state.lock().await
    }

    pub fn interrupt(&self) {
        let _ = self.commands.send(Command::Interrupt);
    }

    pub fn stop(&self) {
        let _ = self.commands.send(Command::Stop);
    }

    /// 换技能（设置面板改的时候走这里）。
    pub fn switch_skills(&self, skills: Vec<crate::skill::Skill>) -> crate::error::Result<()> {
        self.commands
            .send(Command::SetSkills(skills))
            .map_err(|_| crate::error::Error::Io("语音会话已结束".into()))
    }

    /// 运行中切换陪伴/助手。提示词、语速、分句阈值一起变
    pub fn set_mode(&self, mode: InteractionMode) -> crate::error::Result<()> {
        self.commands
            .send(Command::SetMode(mode))
            .map_err(|_| crate::error::Error::Io("语音会话已结束".into()))
    }

    /// 运行中换 TTS 引擎。不调这个的话，切引擎要等下次开语音才生效
    pub fn set_tts(
        &self,
        engine: Arc<dyn TextToSpeech>,
        voice: String,
        uses_voice_library: bool,
    ) -> crate::error::Result<()> {
        self.commands
            .send(Command::SetTts {
                engine,
                voice,
                uses_voice_library,
            })
            .map_err(|_| crate::error::Error::Io("语音会话已结束".into()))
    }

    /// 角色绑的音色。None = 解绑
    pub fn set_character_voice(&self, voice: Option<String>) -> crate::error::Result<()> {
        self.commands
            .send(Command::SetCharacterVoice(voice))
            .map_err(|_| crate::error::Error::Io("语音会话已结束".into()))
    }

    pub fn set_character(
        &self,
        character: Option<crate::character::Character>,
    ) -> crate::error::Result<()> {
        self.commands
            .send(Command::SetCharacter(character))
            .map_err(|_| crate::error::Error::Io("语音会话已结束".into()))
    }

}

/// 启动语音管线所需的一切
pub struct VoiceDeps {
    pub gateway: Arc<Gateway>,
    /// 当前项目的工作区。`switch_skill` 要靠它命中项目级技能
    /// （`<工作区>/.anima/skills/`）。None = 不在项目里，只有全局技能。
    pub skills_workspace: Option<std::path::PathBuf>,
    pub stt: Arc<dyn SpeechToText>,
    pub tts: Arc<dyn TextToSpeech>,
    /// 当前引擎认不认音色库里的名字。只有 GPT-SoVITS 拿得到音色库
    /// （见 `tts::engine()`：只有它收 `voices_root`）。
    ///
    /// **不判断这一条的话，角色的情绪音色会被套到 Edge / Kokoro 上** ——
    /// 那些值是 `yaemiko/greeting` 这种参考音频路径，Edge 拿它当音色名，
    /// 结果是合成失败或者退回默认嗓子，而且不报错
    pub tts_uses_voice_library: bool,
    pub events: EventSink,
    pub system_prompt: String,
    /// 长期记忆。None 表示记忆系统未启用或未就绪 —— 管线照常工作，只是不记不召。
    pub memory: Option<Arc<Memory>>,
    /// 本次语音会话在记忆库里的 session id
    pub session_id: Option<String>,
    /// 已连接的 MCP 工具。None 表示这条会话不提供工具。
    ///
    /// **语音链路必须自己注入工具。** 文字链路的注入在 commands.rs 里，
    /// 语音管线是另一条独立路径、自己构造 ChatRequest —— 早先只改了前者，
    /// 结果"对它说话"时模型手里一个工具都没有。
    pub mcp: Option<Arc<tokio::sync::RwLock<crate::mcp::McpRegistry>>>,
    /// 统一工具 runtime。语音链路用它执行 MCP 与 system tools；测试环境可为 None。
    pub runtime: Option<Arc<crate::agent_runtime::RuntimeService>>,
    /// 共享的 HTTP 客户端。
    pub http: reqwest::Client,
    /// 应用数据目录。技能要用它定位 skills/
    pub config_dir: std::path::PathBuf,
    /// 配置文件路径，用于落盘持久化
    pub config_path: std::path::PathBuf,
    /// 当前技能列表。空数组 = 不挂载技能
    pub skills: Vec<crate::skill::Skill>,
    /// 所有可选技能。只用来生成 switch_skill 的描述 ——
    /// 模型只能从描述里知道有哪些角色，漏一个就等于那个角色在语音里切不到
    pub available_skills: Vec<crate::skill::Skill>,

    /// 所有可选的角色
    pub available_characters: Vec<crate::character::Character>,
    /// `animations/` 里有哪些动作（内置四个之外的）。写进 system prompt 的
    /// 动作清单，换角色重建提示词时也要用
    pub extra_actions: Vec<(String, String)>,
    /// 陪伴 / 助手。切换时要重建提示词、改语速、改分句阈值
    pub mode: InteractionMode,

    /// 绑定的角色灵魂
    pub character: Option<crate::character::Character>,
    /// Tauri AppHandle，用于唤起原生应用和系统工具
    /// Tauri 的 AppHandle。**是 Option** —— 集成测试里造不出它：
    /// `tauri::test::mock_app()` 给的是 `AppHandle<MockRuntime>`，
    /// 和生产用的 `AppHandle<Wry>` 不是同一个类型，除非把 VoiceDeps 也做成
    /// 泛型（会扩散到整条链路）。
    ///
    /// 而这三个测试盖的正是 barge-in 与对口型时序 —— 语音链路里最容易写错、
    /// 出错了又只表现为"感觉有点不对"的两块。为了一个只在唤起原生应用时才用到
    /// 的字段让它们编不过，代价不对等。
    ///
    /// None 的含义：这个环境不能唤起应用，相关工具调用会回一句说明给模型。
    pub app: Option<tauri::AppHandle>,
    /// 全局背景音乐播放器
    pub music_player: Arc<tokio::sync::Mutex<Option<Arc<crate::audio::playback::Player>>>>,
}

/// 启动语音管线（打开真实麦克风）。
pub fn start(config: VoiceConfig, deps: VoiceDeps) -> Result<VoiceSession> {
    // 播放信号：音频线程把 RMS 和「某句开始出声」发到这里，下面的桥接线程转成事件
    let (out_signal_tx, out_signal_rx) = std::sync::mpsc::channel::<PlaybackSignal>();
    let player = Arc::new(Player::new(out_signal_tx)?);
    let playing = player.speaking_flag();

    let session = capture::start(
        config.input_device.as_deref(),
        VadConfig::from(&config.vad),
        Arc::clone(&playing),
        Some(deps.config_dir.as_path()),
    )?;

    // 采集侧是同步 mpsc（跑在普通线程里），而且 CaptureSession 内含 cpal 的
    // Stream —— 同样是设备绑定、不可跨线程的。所以桥接放在这里做：这个线程
    // 独占 session，actor 只拿到一个 tokio Receiver。
    // 通道关闭（actor 结束）时循环退出，session 随之 drop、采集自动停止。
    let (audio_tx, audio_rx) = mpsc::unbounded_channel();
    std::thread::Builder::new()
        .name("anima-audio-bridge".into())
        .spawn(move || {
            // 必须用 session.recv() 而不是 session.events.recv()：
            // 后者在 Rust 2021 的精确捕获下只会搬走 events 字段，
            // 导致内部的 cpal 音频流被当场 drop、回调永不触发。
            // 详见 CaptureSession 的文档。
            while let Ok(event) = session.recv() {
                if audio_tx.send(event).is_err() {
                    break;
                }
            }
        })
        .map_err(|e| Error::Io(format!("创建音频桥接线程失败：{e}")))?;

    // 播放电平的桥接。单独一条线程而不是塞进上面那条：
    // 上面那条阻塞在 session.recv() 上，收不到播放电平就永远不会醒。
    let level_events = deps.events.clone();
    // **必须在这里构造** —— 下面那条是 std::thread，没有 tokio 上下文，
    // 在里面调 Handle::current() 会 panic
    let lipsync = crate::avatar::lipsync_stream::LipsyncStream::new(deps.http.clone());
    let lipsync_out = lipsync.clone();
    // 标定过的元音模板。空的话 analyze 会退回"按响度张嘴" ——
    // 粗糙但不需要任何标定，比嘴完全不动强
    let lipsync_profile = crate::avatar::lipsync::Profile::default();
    std::thread::Builder::new()
        .name("anima-out-level".into())
        .spawn(move || {
            // 顺便做窗口峰值诊断。加它的理由很实际：这条链路（Metered 包装器 →
            // channel → 事件）是新写的，而"谱线在动"不等于"接上了"——
            // 静音时也有底噪幅度。有了这行日志，只要看一眼 anima.log
            // 就知道播放电平有没有真的产生，不用靠肉眼判断幅度。
            let mut peak = 0.0f32;
            let mut window_start = std::time::Instant::now();
            while let Ok(signal) = out_signal_rx.recv() {
                match signal {
                    PlaybackSignal::Level(rms) => {
                        peak = peak.max(rms);
                        if window_start.elapsed() >= std::time::Duration::from_secs(2) {
                            if peak > 0.0 {
                                tracing::info!(
                                    peak = format!("{peak:.5}"),
                                    "TTS 播放电平（2 秒窗口峰值）"
                                );
                            }
                            peak = 0.0;
                            window_start = std::time::Instant::now();
                        }
                        level_events(VoiceEvent::OutputLevel { rms });
                    }
                    // 对口型的时间原点。这条事件必须原样直达前端，
                    // 中间不做任何节流 —— 它一句只有一次，而且晚一点点都会错开嘴型
                    PlaybackSignal::Started(id) => {
                        level_events(VoiceEvent::SentenceStart { id });
                        // 同一个原点也给口型 sidecar。**不能晚** ——
                        // 它拿这一刻当 t=0 去查梅尔，差几十毫秒就看得出来
                        lipsync_out.started(id);
                    }
                    // 音频驱动的口型。**只有没走文本时间线的音频才会到这儿** ——
                    // play_audio、电台。补的正是"这些场景嘴一动不动"
                    PlaybackSignal::Frame(samples, rate) => {
                        let f = crate::avatar::lipsync::analyze(&samples, rate, &lipsync_profile);
                        level_events(VoiceEvent::AudioViseme {
                            viseme: f.viseme,
                            weight: f.weight,
                        });
                    }
                }
            }
        })
        .map_err(|e| Error::Io(format!("创建播放电平线程失败：{e}")))?;

    start_with_audio(config, deps, audio_rx, player)
}

/// 用外部提供的音频事件源启动管线。
///
/// 存在的意义是**可测**：barge-in 是这条链路最关键也最难验证的行为，而
/// [`start`] 会去开真实麦克风。有了这个入口，集成测试可以直接投喂
/// `SpeechStart` / `Utterance` 事件，验证「打断确实取消了 LLM 流、清空了
/// 播放队列」，不依赖任何声卡。
pub fn start_with_audio(
    config: VoiceConfig,
    deps: VoiceDeps,
    audio_rx: mpsc::UnboundedReceiver<CaptureEvent>,
    player: Arc<Player>,
) -> Result<VoiceSession> {
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let state = Arc::new(Mutex::new(VoiceState::Listening));

    let actor = Actor {
        last_turn_at: std::time::Instant::now(),
        last_initiative_at: None,
        initiative_times: Vec::new(),
        fact_cursor: 0,
        timers: Vec::new(),
        next_timer_id: 1,
        pending_report: None,
        character_voice: None,
        pending_reminder: None,
        config,
        timing: FirstAudioTiming::default(),
        // 生成任务要用它把"切换技能"的结果送回 actor —— 那个任务持有的是
        // deps 的克隆，直接改只影响本轮
        commands_tx: cmd_tx.clone(),
        deps,
        player,
        state: Arc::clone(&state),
        summary: crate::memory::summary::Summary::default(),
        history: Vec::new(),
        turn: None,
        // 起始值先用中性；下面 run() 里会从库里读回真实值并按空闲时长衰减。
        // 不在这里 await 是因为 start_with_audio 是同步的
        emotion: crate::personality::EmotionState::default(),
        signals: crate::personality::TurnSignals::default(),
        consecutive_interruptions: 0,
        interruption_tolerance: crate::personality::roll_interruption_tolerance(),
    };

    tokio::spawn(actor.run(audio_rx, cmd_rx));

    Ok(VoiceSession {
        commands: cmd_tx,
        state,
    })
}

/// 一轮正在进行的问答
struct Turn {
    cancel: CancellationToken,
    /// 本轮是否已作废（被打断）。TTS 任务据此决定还要不要入队。
    aborted: Arc<AtomicBool>,
    /// 生成任务是否已经结束（正常收尾、出错、被取消都算）。
    ///
    /// **没有这个标志就没法判断"一轮说完了"。** 消费 LLM 流的任务是
    /// `tokio::spawn` 出去的，碰不到 actor 的字段，所以它跑完时无法把
    /// `self.turn` 置空。早先的收尾条件写的是 `self.turn.is_none()`，
    /// 而 turn 只在 `abort_turn` 里被 take —— 正常说完永远不满足，于是
    /// 状态永远停在 Speaking，用户下一次**正常开口**被判成打断：
    /// 心情每轮 -3 起、连击递增，十几轮锁死在 -100，而好感度只在
    /// completed 时上升所以几乎不动。日志里 completed=false 一路刷屏。
    finished: Arc<AtomicBool>,
}

/// 离开作用域就把 `finished` 置真。
///
/// 用 Drop 而不是在任务末尾手动置位：那个任务有七八个 `return`
/// （打断、建流失败、工具轮出错…），漏掉任何一个都会让这一轮永远收不了尾。
struct FinishOnDrop(Arc<AtomicBool>);

impl Drop for FinishOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

struct Actor {
    config: VoiceConfig,
    /// 回给自己的命令通道。见 start_with_audio 里的说明
    commands_tx: mpsc::UnboundedSender<Command>,
    deps: VoiceDeps,
    player: Arc<Player>,
    state: Arc<Mutex<VoiceState>>,
    /// 本轮的首包计时。识别那一段在 handle_utterance 里填，其余在生成任务里
    timing: FirstAudioTiming,
    /// 会话内的滚动摘要。历史被裁掉的部分压缩到这里 ——
    /// 否则那些内容就真的没了（见 memory::summary）
    summary: crate::memory::summary::Summary,
    /// 对话历史。只在 actor 内部改，不共享。
    history: Vec<Message>,
    turn: Option<Turn>,
    /// 主动开口的账本。**默认就是安静**，这几个值只用来给"说话"找理由，
    /// 找不到就不说 —— 见 initiative 模块
    last_turn_at: std::time::Instant,
    last_initiative_at: Option<std::time::Instant>,
    initiative_times: Vec<std::time::Instant>,
    /// 主动开口时提哪几条事实的游标。**轮换而不是随机** ——
    /// 随机会连着两次提同一件事，很出戏
    fact_cursor: usize,
    /// 待触发的提醒。**存内存** —— 应用重启就没了，
    /// 所以工具说明里明确告诉模型"更久的用日历"
    timers: Vec<timer::Timer>,
    next_timer_id: u64,
    /// 后台任务做完了、还没回报的内容。**不受闲聊配额限制** ——
    /// 事情办完了不告诉用户是失信，不是"安静"
    pending_report: Option<String>,
    /// 当前角色绑的固定音色。**只活在会话里** —— 写进全局 config 的话，
    /// 换回默认形态之后声音还是角色那个
    character_voice: Option<String>,
    /// 到点了、还没说出口的提醒
    pending_reminder: Option<String>,
    /// 当前情感轴。启动时从库里读，每轮结束后更新并落库。
    emotion: crate::personality::EmotionState,
    /// 本轮累计的信号。轮次开始时重置，结束时汇总成一次轴更新。
    ///
    /// 放在 actor 上而不是随消息传递：打断是**跨消息**发生的
    /// （用户在生成中途开口），只有 actor 能看到完整一轮的经过。
    signals: crate::personality::TurnSignals,
    /// 连续被打断的轮次数。一次正常说完就归零 —— 见 TurnSignals 的说明。
    consecutive_interruptions: u32,
    /// 这一段连击能忍几次才扣心情。连击开始时掷一次，整段沿用 ——
    /// 每轮重掷的话"忍到第几次"会来回跳，而用户感受到的是同一段连打
    interruption_tolerance: u32,
}

/// 首包时间的分段计时。
///
/// **必须先能测再谈优化。** 这条链路（识别 → 召回 → LLM 首字 → 攒够一句 →
/// 合成 → 出声）原先只有 STT 一处有计时，其余全靠猜 —— 而这个项目里凡是靠猜
/// 的判断基本都错了。
///
/// 每轮打一行，字段都是"到出声为止"的累计毫秒，方便直接看谁占大头。
#[derive(Debug, Default)]
struct FirstAudioTiming {
    /// 语音识别
    stt_ms: u128,
    /// 长期记忆的语义召回。**在 LLM 之前、且是阻塞的**，所以直接进首包
    recall_ms: u128,
    /// 从发起请求到收到第一个正文 token
    llm_first_token_ms: u128,
    /// 从第一个 token 到攒够一句（受 sentence_min_chars 影响）
    sentence_ms: u128,
    /// 第一句的 TTS 合成
    tts_ms: u128,
}

impl FirstAudioTiming {
    fn total_ms(&self) -> u128 {
        self.stt_ms + self.recall_ms + self.llm_first_token_ms + self.sentence_ms + self.tts_ms
    }

    fn log(&self) {
        tracing::info!(
            total_ms = self.total_ms(),
            stt_ms = self.stt_ms,
            recall_ms = self.recall_ms,
            llm_first_token_ms = self.llm_first_token_ms,
            sentence_ms = self.sentence_ms,
            tts_ms = self.tts_ms,
            "首包分解（到第一声出来为止）"
        );
    }
}

/// 语音链路的默认 System Prompt。
///
/// 与文字对话刻意不同：语音回答必须**短**。一段五百字的回答用文字看是详尽，
/// 用听是折磨 —— 而且 TTS 要逐句合成，长回答会显著拖长「说完」的时间。
/// 阶段三接入性格引擎后，这里会换成由情感轴动态拼接的结果。
/// **能力说明。人设可以换，这一段永远都要在。**
///
/// 这是拆出来的：原先它和"你是 Anima…"那句身份描述写在同一个常量里，
/// 而角色系统启用时 `system_prompt = character.build_identity_prompt()`
/// 会把整个常量**替换掉** —— 连带把动作标签、联网主动性、
/// "语音对话要短"全都删掉了。
///
/// 表现就是"我让学姐挥手，她没反应"：她压根不知道有 `<action:>` 这回事，
/// 只好用 emoji（👋😊）代替 —— 日志里那一串带 emoji 的回复就是证据。
/// 身份是"你是谁"，能力是"你能做什么"，换角色不该让它失去能力。
pub const CAPABILITY_PROMPT: &str = "\n\n[Capabilities & Spoken Expression Guidelines]\
You are participating in a **spoken voice** conversation. Do NOT use lists, headers, code blocks, or structured formatting.\
\n1. Proactivity & Decisiveness: When asked broad or open-ended inquiries (e.g. current news, markets, stocks, weather), do NOT act like a rigid agent asking for minutiae. Proactively assume a sensible intent, execute available web search tools (e.g. search_web), and provide a concise, high-signal conversational summary.\
\n2. Music Playback: When requested to play music/radio, use play_radio to select a suitable broadcast station. For specific copyrighted tracks that cannot be streamed directly, explain transparently and suggest local playback while offering playback controls (music_control).\
\n3. Language & Communication: Always converse naturally in the user's preferred spoken language (default to Chinese for Chinese queries). However, keep generative image/video parameters and ComfyUI generation prompts strictly in English.\
";

/// Built-in body gestures (generated in code, see `src/avatar/animations.ts`).
pub const BUILTIN_ACTIONS: &[(&str, &str)] = &[
    ("wave", "wave hand in greeting"),
    ("stretch", "stretch body relaxedly"),
    ("cover_mouth", "cover mouth giggle or surprise"),
    ("heart", "form heart hand gesture"),
    ("exit_pet_mode", "reveal main chat window to user"),
];

/// Generate physical animation tags section.
pub fn action_prompt(extra: &[(String, String)], tone: &str) -> String {
    let mut list = String::new();
    for (name, desc) in BUILTIN_ACTIONS {
        list.push_str(&format!("`<action:{name}>`({desc}), "));
    }
    for (name, desc) in extra {
        list.push_str(&format!("`<action:{name}>`({desc}), "));
    }
    while list.ends_with(", ") {
        list.truncate(list.len() - 2);
    }

    format!(
        "\n5. Physical Gestures: Your 3D/Live2D body is visually rendered on screen. Including an action tag in your response triggers physical animations.\
Available actions: {list}.\
\nUsage Examples:\
\n  User: Wave your hand\
\n  Assistant: <action:wave>Hello there!\
\n  User: Stretch a bit\
\n  Assistant: <action:stretch>Ah, that feels so much better.\
\n**When explicitly asked to perform an action, you MUST include the exact tag.** Pure emojis (👋😊) will not trigger avatar motion.\
{tone}Do not fabricate action names outside this list.\
\nDo NOT emit any other HTML or XML tags (`<span>`, `<div>`, etc.) as they will be spoken aloud by TTS."
    )
}

/// Default identity description when no character is selected.
pub const DEFAULT_IDENTITY: &str = "You are Anima, an intelligent, proactive, and expressive AI companion.";

/// 身份 + 能力。**所有构造 system prompt 的地方都要走这里。**
///
/// 分散拼接的后果已经出现过：`voice_start` 里角色一启用就把整个提示词换掉，
/// 能力那段连带没了。一个函数就没有"某处忘了接能力"这种可能。
pub fn compose_system_prompt(
    identity: &str,
    extra_actions: &[(String, String)],
    mode: InteractionMode,
) -> String {
    let identity = identity.trim();
    let who = if identity.is_empty() {
        DEFAULT_IDENTITY
    } else {
        identity
    };
    let p = mode.profile();
    // 行为准则放在最后 —— 后读到的权重更高，而"现在该怎么相处"
    // 必须压得住前面的身份与能力描述
    format!(
        "{who}{CAPABILITY_PROMPT}{}{}",
        action_prompt(extra_actions, p.action_tone),
        p.behaviour
    )
}

/// 默认的完整 system prompt = 默认身份 + 能力。
pub fn default_system_prompt() -> String {
    compose_system_prompt(DEFAULT_IDENTITY, &[], InteractionMode::default())
}

/// 从一段待朗读的文本里剥掉 `<action:xxx>` 标签，并把每个标签作为事件抛出。
///
/// **两条路都必须过这里。** 原先只在 `splitter.push()` 那条做，而收尾的
/// `splitter.flush()`（最后一句没有标点收尾的残句）没做 —— 模型很常把标签
/// 放在整段回答的末尾，于是那个标签既不会触发动作、还会**被当成文字念出来**
/// （"小于号 action 冒号 heart 大于号"）。表现就是"我让它比心，它没反应"。
fn strip_action_tags(text: &mut String, events: &EventSink) {
    loop {
        // 开标签和闭标签都要认。**模型很爱写成对的** ——
        // `<action:heart>说点什么</action:heart>`。只认开标签的话，
        // `</action:heart>` 会当成正文被念出来（"小于号斜杠 action…"），
        // 用户实测撞到的就是这个。
        let open = text.find("<action:");
        let close = text.find("</action:");
        let (start, is_open) = match (open, close) {
            (Some(o), Some(c)) => {
                if o < c {
                    (o, true)
                } else {
                    (c, false)
                }
            }
            (Some(o), None) => (o, true),
            (None, Some(c)) => (c, false),
            (None, None) => break,
        };

        let Some(rel_end) = text[start..].find('>') else {
            // 标签被流式分块从中间切断了。剩下的半截不能念出来，
            // 直接丢掉 —— 下一块里的另一半会作为普通文本出现，也是垃圾，
            // 但至少不会念出一串符号
            text.truncate(start);
            break;
        };

        if is_open {
            // "<action:" 是 8 个字节
            let action = text[start + 8..start + rel_end].trim().to_string();
            if !action.is_empty() {
                tracing::info!(%action, "解析到动作标签");
                events(VoiceEvent::Action { action });
            }
        }
        // 闭标签只删不触发 —— 一个动作只该播一次
        text.replace_range(start..start + rel_end + 1, "");
    }
    // 模型有时会顺手编出别的标记（实测过 <span></span>），一并剥掉
    strip_markup(text);
}

/// 剥掉剩下的 HTML 式标签。
///
/// 模型被"标签"这个概念带偏之后会开始编标记 —— 实测收到过
/// `<span></span> 好啊，伸个懒腰～<span></span> 这颗心给你💖`，
/// 那两个 span 直接被念了出来。
///
/// 判据是"尖括号 + 字母开头 + 到下一个尖括号"：正文里的小于号不会长这样
/// （`1 < 2`、`a<b` 都不匹配 —— 前者后面是空格和数字，后者没有闭合的 `>`）。
fn strip_markup(text: &mut String) {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut removed = false;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            // `<` 后面必须是字母或 `/`+字母，才当成标记
            let after = i + 1;
            let is_tag_start = match bytes.get(after) {
                Some(b'/') => bytes
                    .get(after + 1)
                    .is_some_and(|c| c.is_ascii_alphabetic()),
                Some(c) => c.is_ascii_alphabetic(),
                None => false,
            };
            if is_tag_start {
                if let Some(rel) = text[i..].find('>') {
                    i += rel + 1;
                    removed = true;
                    continue;
                }
            }
        }
        // 按字符推进，别切断多字节
        let ch_len = text[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        out.push_str(&text[i..i + ch_len]);
        i += ch_len;
    }
    if removed {
        tracing::info!(cleaned = %out, "回复里带了 HTML 式标记，已剥掉");
        *text = out;
    }
}

/// 只剥标签、不触发事件。用于**要留存或要显示**的文本：
/// 写进记忆的对话原文、以及界面上的气泡都不该带着 `<action:…>`。
pub fn without_action_tags(text: &str) -> String {
    let mut owned = text.to_string();
    let sink: EventSink = Arc::new(|_| {});
    strip_action_tags(&mut owned, &sink);
    owned
}

fn runtime_outcome_for_voice(outcome: &crate::agent_runtime::ToolOutcome) -> String {
    if outcome.status == crate::agent_runtime::ToolStatus::Ok {
        return outcome.content.clone();
    }

    let mut text = format!(
        "runtime 工具 {} 执行未成功，状态：{:?}。",
        outcome.tool_name, outcome.status
    );
    if !outcome.content.trim().is_empty() {
        text.push_str("\n结果：");
        text.push_str(&outcome.content);
    }
    if let Some(failure) = &outcome.failure {
        text.push_str("\n失败类型：");
        text.push_str(&format!("{:?}", failure.kind));
        if let Some(hint) = &failure.hint {
            text.push_str("\n建议：");
            text.push_str(hint);
        }
    }
    text
}

/// 把用户设定的语速（`+10%` 这种）再乘一个模式倍率。
///
/// 用乘法而不是覆盖：用户在设置里调过的语速是他的偏好，模式只是在这个偏好
/// 上下浮动。直接覆盖的话，切一次模式就把用户的设置吃掉了。
fn mode_adjusted_rate(rate: &str, factor: f32) -> String {
    let base = rate
        .trim()
        .trim_end_matches('%')
        .parse::<f32>()
        .map(|pct| 1.0 + pct / 100.0)
        .unwrap_or(1.0);
    let combined = (base * factor).clamp(0.5, 2.0);
    format!("{:+.0}%", (combined - 1.0) * 100.0)
}

/// 短期工作记忆的窗口大小（轮次数）。
/// 与 ARCHITECTURE.md 3.4 的「10-15 轮滑动窗口」一致；
/// 超出的部分等阶段二的记忆层接进来后再做摘要与向量化。
const MAX_HISTORY_TURNS: usize = 12;

impl Actor {
    /// 当前情感状态该读写哪个作用域。
    ///
    /// 好感度永远来自 global，心情与活跃度来自当前技能 —— 组合逻辑在
    /// `Store::emotion_for` 里。这里只负责说出"当前是哪个技能"。
    fn emotion_scope(&self) -> String {
        crate::memory::store::Store::skill_scope(self.deps.skills.first().map(|s| s.id.as_str()))
    }

    async fn set_state(&self, state: VoiceState) {
        *self.state.lock().await = state;
        (self.deps.events)(VoiceEvent::State { state });
        // 体态（待机 / 说话 / 倾听）跟着状态走。这一层和口型是分开的：
        // 口型重绘嘴，体态换的是整段肢体动作
        crate::avatar::lipsync_stream::LipsyncStream::new(self.deps.http.clone()).set_state(state);
    }

    fn emit(&self, event: VoiceEvent) {
        (self.deps.events)(event);
    }

    async fn run(
        mut self,
        mut audio_rx: mpsc::UnboundedReceiver<CaptureEvent>,
        mut commands: mpsc::UnboundedReceiver<Command>,
    ) {
        self.emit(VoiceEvent::State {
            state: VoiceState::Listening,
        });

        // 从库里读回情感轴，并按"距上次对话多久"往中性衰减。
        //
        // 衰减放在会话启动而不是每轮：它按真实时间计算，而一次会话内
        // 时间跨度通常只有几分钟，逐轮算等于白算。
        if let Some(memory) = &self.deps.memory {
            let scope = self.emotion_scope();
            match memory.store().emotion_with_age_for(&scope).await {
                Ok((state, hours_idle)) => {
                    self.emotion = crate::personality::decay(state, hours_idle);
                    tracing::info!(
                        affection = format!("{:.1}", self.emotion.affection),
                        mood = format!("{:.1}", self.emotion.mood),
                        energy = format!("{:.1}", self.emotion.energy),
                        hours_idle = format!("{hours_idle:.1}"),
                        "情感轴已载入"
                    );
                }
                Err(err) => tracing::warn!(%err, "读取情感状态失败，用中性起点"),
            }
            self.emit(VoiceEvent::Emotion {
                affection: self.emotion.affection,
                mood: self.emotion.mood,
                energy: self.emotion.energy,
                familiarity: crate::personality::Familiarity::of(self.emotion.affection),
                irritation: crate::personality::irritation(self.emotion),
            });
        }

        // 定期同步播放状态：rodio 没有「播完」回调，不主动同步的话
        // playing 标志会一直是 true，VAD 永远停在钝化状态、再也打断不了
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(200));

        loop {
            tokio::select! {
                Some(cmd) = commands.recv() => {
                    match cmd {
                        Command::Stop => break,
                        Command::Interrupt => self.abort_turn(true).await,
                        Command::SetSummary(summary) => {
                            tracing::debug!(
                                chars = summary.text.chars().count(),
                                focus = summary.focus.len(),
                                "收到新的滚动摘要"
                            );
                            self.summary = summary;
                        }
                        Command::SetSkills(skills) => {
                            let marker = skills.iter().map(|s| s.marker.clone()).collect::<Vec<_>>().join(" + ");
                            tracing::info!(
                                skills = ?skills.iter().map(|s| &s.id).collect::<Vec<_>>(),
                                "切换技能阵列"
                            );
                            // **换技能之前先把当前情绪存回旧作用域。**
                            // 不存的话这个角色刚累积的心情会丢 —— 而"各角色有各自
                            // 的脾气"正是分档的目的
                            if let Some(memory) = &self.deps.memory {
                                let old = self.emotion_scope();
                                if let Err(err) =
                                    memory.store().set_emotion_for(&old, self.emotion).await
                                {
                                    tracing::warn!(%err, scope = %old, "切换前保存情绪失败");
                                }
                            }

                            // 同步到全局配置
                            if let Ok(mut config) = self.deps.gateway.config().try_write() {
                                config.active_skills = skills.iter().map(|s| s.id.clone()).collect();
                                let _ = config.save(&self.deps.config_path);
                            }

                            self.deps.skills = skills;
                            // 读新作用域的情绪，并按它自己的空闲时长衰减 ——
                            // 一周没用的角色，它的心情就该衰减一周
                            if let Some(memory) = &self.deps.memory {
                                let scope = self.emotion_scope();
                                if let Ok((state, hours)) =
                                    memory.store().emotion_with_age_for(&scope).await
                                {
                                    self.emotion = crate::personality::decay(state, hours);
                                    tracing::info!(
                                        scope = %scope,
                                        idle_hours = format!("{hours:.1}"),
                                        mood = format!("{:.1}", self.emotion.mood),
                                        energy = format!("{:.1}", self.emotion.energy),
                                        "已切换到该角色的情绪状态"
                                    );
                                    self.emit(VoiceEvent::Emotion {
                                        affection: self.emotion.affection,
                                        mood: self.emotion.mood,
                                        energy: self.emotion.energy,
                                        familiarity: crate::personality::Familiarity::of(
                                            self.emotion.affection,
                                        ),
                                        irritation: crate::personality::irritation(self.emotion),
                                    });
                                }
                            }
                            // 前端要在顶部状态那一行显示标记
                            self.emit(VoiceEvent::Skill { marker: if marker.is_empty() { None } else { Some(marker) } });
                        }
                        Command::SetCharacter(character) => {
                            tracing::info!(
                                character = ?character.as_ref().map(|c| &c.id),
                                "切换角色"
                            );
                            self.deps.character = character.clone();
                            // **换角色要清掉上一个角色的音色绑定。**
                            // 不清的话，切回默认形态之后声音还是上一个角色的，
                            // 而用户以为已经切回来了。绑定由 apply_character_binding
                            // 随后重新推过来
                            self.character_voice = None;
                            // 同步到全局配置
                            if let Ok(mut config) = self.deps.gateway.config().try_write() {
                                config.active_character = character.as_ref().map(|c| c.id.clone());
                                let _ = config.save(&self.deps.config_path);
                            }
                            // **提示词必须跟着重建。** 只改 deps.character 的话，
                            // 运行中切角色对模型完全没有影响 —— 它看到的还是
                            // 上一个角色的身份，得重启语音才生效
                            let identity = match &character {
                                Some(c) => c.build_identity_prompt(),
                                None => DEFAULT_IDENTITY.to_string(),
                            };
                            self.deps.system_prompt =
                                compose_system_prompt(&identity, &self.deps.extra_actions, self.deps.mode);
                        }
                        Command::SetTts { engine, voice, uses_voice_library } => {
                            tracing::info!(%voice, uses_voice_library, "运行中换了 TTS 引擎");
                            self.deps.tts = engine;
                            self.deps.tts_uses_voice_library = uses_voice_library;
                            // 音色必须跟着换，否则新引擎会收到上一家的音色名
                            self.config.tts.voice = voice;
                        }
                        Command::SetCharacterVoice(voice) => {
                            tracing::info!(?voice, "角色音色绑定已更新");
                            self.character_voice = voice;
                        }
                        Command::SetMode(mode) => {
                            tracing::info!(mode = %mode.label(), "切换交互模式");
                            self.deps.mode = mode;
                            // 三样一起改，散着调永远调不齐 —— 见 voice::mode 的模块注释
                            let identity = match &self.deps.character {
                                Some(c) => c.build_identity_prompt(),
                                None => DEFAULT_IDENTITY.to_string(),
                            };
                            self.deps.system_prompt = compose_system_prompt(
                                &identity,
                                &self.deps.extra_actions,
                                mode,
                            );
                            self.config.sentence_min_chars = mode.profile().sentence_min_chars;
                            if let Ok(mut config) = self.deps.gateway.config().try_write() {
                                config.interaction_mode = mode;
                                let _ = config.save(&self.deps.config_path);
                            }
                        }
                        Command::AddTimer { minutes, label } => {
                            let id = self.next_timer_id;
                            self.next_timer_id += 1;
                            let due = std::time::Instant::now()
                                + std::time::Duration::from_secs_f64(minutes * 60.0);
                            self.timers.push(timer::Timer { id, due, label: label.clone() });
                            tracing::info!(id, minutes, %label, "定下提醒");
                        }
                        Command::CancelTimer(which) => {
                            match which {
                                Some(id) => self.timers.retain(|t| t.id != id),
                                None => self.timers.clear(),
                            }
                            tracing::info!(?which, left = self.timers.len(), "取消提醒");
                        }
                    }
                }

                Some(event) = audio_rx.recv() => {
                    self.on_capture_event(event).await;
                }

                _ = tick.tick() => {
                    // 一轮真正结束 = **生成任务跑完了** 而且 **音频放完了**。
                    //
                    // 两个条件都必须查。只看播放器的话，思考阶段（还没有任何
                    // 音频）会被误判成"说完了"；只看生成任务的话，最后一句
                    // 还在播就把状态切走，用户此时开口不算打断。
                    //
                    // Thinking 也要纳入：生成结束却一句都没出声是真实场景
                    // （TTS 服务挂了、模型只调了工具没说话），漏掉它会让状态
                    // 永远卡在 Thinking，后果和之前卡在 Speaking 一样。
                    let generation_done = self
                        .turn
                        .as_ref()
                        .is_some_and(|t| t.finished.load(Ordering::Relaxed));
                    if generation_done
                        && !self.player.sync_speaking_state()
                        && matches!(
                            *self.state.lock().await,
                            VoiceState::Speaking | VoiceState::Thinking
                        )
                    {
                        self.turn = None;
                        // 安静的计时从**她说完**开始，不是从用户开口开始 ——
                        // 否则一段长回答刚播完就会立刻触发主动搭话
                        self.last_turn_at = std::time::Instant::now();
                        self.emit(VoiceEvent::TurnComplete);
                        // 一轮真正说完（不是被打断）才按"完成"结算
                        self.settle_emotion(true).await;
                        self.set_state(VoiceState::Listening).await;
                    }

                    self.fire_due_timers();
                    self.maybe_speak_up().await;
                }

                else => break,
            }
        }

        self.abort_turn(false).await;
        self.set_state(VoiceState::Idle).await;
    }

    async fn on_capture_event(&mut self, event: CaptureEvent) {
        match event {
            CaptureEvent::Level(rms) => self.emit(VoiceEvent::Level { rms }),

            CaptureEvent::SpeechStart => {
                // 用户开口 —— 只有在「我们正在说或正在想」时才算打断
                let state = *self.state.lock().await;
                if matches!(state, VoiceState::Thinking | VoiceState::Speaking) {
                    tracing::info!(
                        ?state,
                        tts_playing = self.player.is_speaking(),
                        "检测到用户开口，打断当前轮次"
                    );
                    self.abort_turn(true).await;
                }
                self.set_state(VoiceState::Listening).await;
            }

            CaptureEvent::Utterance(samples) => {
                self.handle_utterance(samples).await;
            }
        }
    }

    /// 作废当前轮次：取消 LLM 流 + 清空播放队列 + 标记本轮无效。
    async fn abort_turn(&mut self, notify: bool) {
        let Some(turn) = self.turn.take() else {
            return;
        };
        turn.aborted.store(true, Ordering::Relaxed);
        turn.cancel.cancel();
        self.player.interrupt();
        // 口型也要立刻停。**不停的话她嘴还在动而声音已经没了** ——
        // 那比嘴不动更奇怪
        crate::avatar::lipsync_stream::LipsyncStream::new(self.deps.http.clone()).stop();
        if notify {
            self.emit(VoiceEvent::Interrupted);
            self.signals.interruptions += 1;
            // 被打断的轮次也要结算 —— 打断本身就是信号（心情下降、活跃度上升）。
            // 但 completed=false，所以不加那份"正常说完"的好感度。
            //
            // 不结算的话信号会累积到下一轮，让"上一轮被打断"错误地记到
            // 下一轮头上；而下一轮可能根本没被打断。
            self.settle_emotion(false).await;
        }
    }

    async fn handle_utterance(&mut self, samples: Vec<f32>) {
        // 用户开口了 —— 主动开口的计时从这一刻重新算
        self.last_turn_at = std::time::Instant::now();
        tracing::info!(
            seconds = format!("{:.2}", samples.len() as f32 / 16_000.0),
            "收到一句话，开始识别"
        );
        self.set_state(VoiceState::Transcribing).await;

        // 新一轮开始，计时归零
        self.timing = FirstAudioTiming::default();
        let stt_started = std::time::Instant::now();
        let transcript = match self.deps.stt.transcribe(&samples).await {
            Ok(t) => t,
            Err(err) => {
                tracing::warn!(%err, "识别失败");
                self.emit(VoiceEvent::Error {
                    message: err.to_string(),
                });
                self.set_state(VoiceState::Listening).await;
                return;
            }
        };

        self.timing.stt_ms = stt_started.elapsed().as_millis();
        let text = transcript.text.trim().to_string();
        tracing::info!(text = %text, "识别完成");
        if text.is_empty() {
            // 识别为空（静音/噪声）不该惊动模型，静静回到听的状态
            tracing::debug!("识别结果为空，忽略本段音频");
            self.set_state(VoiceState::Listening).await;
            return;
        }

        self.emit(VoiceEvent::Transcript { text: text.clone() });
        self.history.push(Message::user(text.clone()));
        let dropped = trim_history(&mut self.history);

        // ── 被裁掉的内容压成摘要 ──
        //
        // **放后台、不阻塞这一轮。** 压缩要调一次模型（本地模型两三秒），
        // 同步做的话用户会在某一轮突然多等三秒 —— 而那一轮和别的轮没有区别，
        // 体验上就是"偶发卡顿"。所以裁剪立刻发生，摘要下一轮才用得到。
        if crate::memory::summary::worth_summarizing(&dropped) {
            let gateway = Arc::clone(&self.deps.gateway);
            let previous = self.summary.clone();
            let back = self.commands_tx.clone();
            tokio::spawn(async move {
                if let Some(next) =
                    crate::memory::summary::compress(&gateway, &previous, &dropped).await
                {
                    // 结果要送回 actor —— 这个任务改不了 actor 的字段
                    let _ = back.send(Command::SetSummary(next));
                }
            });
        }

        self.start_response(text).await;
    }

    /// 一轮结束时汇总信号、更新情感轴、落库并广播。
    ///
    /// 只在轮次边界做 —— 轴值本来就只在这里变化，放到每帧或每条消息里
    /// 既没有意义也会让"一次对话涨多少"变得不可预测。
    async fn settle_emotion(&mut self, completed: bool) {
        use crate::personality;

        // 连击计数：被打断就 +1，正常说完就归零
        if completed {
            self.consecutive_interruptions = 0;
            // 下一段连击重新掷一次容忍度，别让"第几次它会生气"被摸出规律
            self.interruption_tolerance = personality::roll_interruption_tolerance();
        } else if self.signals.interruptions > 0 {
            self.consecutive_interruptions += 1;
        }
        self.signals.consecutive_interruptions = self.consecutive_interruptions;
        self.signals.interruption_tolerance = self.interruption_tolerance;
        self.signals.completed = completed;
        self.emotion = personality::apply_turn(self.emotion, &self.signals);
        self.signals = personality::TurnSignals::default();

        if let Some(memory) = &self.deps.memory {
            let scope = self.emotion_scope();
            if let Err(err) = memory.store().set_emotion_for(&scope, self.emotion).await {
                // 落库失败不该影响对话 —— 内存里的值仍然有效，
                // 只是下次启动会退回上一次成功保存的状态
                tracing::warn!(%err, "保存情感状态失败");
            }
        }

        tracing::info!(
            affection = format!("{:.1}", self.emotion.affection),
            mood = format!("{:.1}", self.emotion.mood),
            energy = format!("{:.1}", self.emotion.energy),
            irritation = format!("{:.2}", personality::irritation(self.emotion)),
            streak = self.consecutive_interruptions,
            tolerance = self.interruption_tolerance,
            completed,
            "情感轴已更新"
        );

        self.emit(VoiceEvent::Emotion {
            affection: self.emotion.affection,
            mood: self.emotion.mood,
            energy: self.emotion.energy,
            familiarity: personality::Familiarity::of(self.emotion.affection),
            irritation: personality::irritation(self.emotion),
        });
    }

    /// 到点的提醒排进待说队列。
    ///
    /// 走 `pending_report` 那条路 —— 它**绕过冷却和配额**，
    /// 因为定时提醒和任务回报一样是承诺：说好了要提醒，就必须提醒。
    fn fire_due_timers(&mut self) {
        let due = timer::take_due(&mut self.timers, std::time::Instant::now());
        if due.is_empty() {
            return;
        }
        let text = timer::announcement(&due);
        tracing::info!(count = due.len(), %text, "提醒到点");
        // 已经有待说的提醒就接在后面，别把前一条挤掉
        self.pending_reminder = Some(match self.pending_reminder.take() {
            Some(prev) => format!("{prev}；另外，{text}"),
            None => text,
        });
    }

    /// 空闲时评估要不要主动开口。**只在真正空闲时调用** ——
    /// 打断用户是这个功能唯一不可接受的失败。
    async fn maybe_speak_up(&mut self) {
        if self.turn.is_some() {
            return;
        }
        if !matches!(
            *self.state.lock().await,
            VoiceState::Idle | VoiceState::Listening
        ) {
            return;
        }

        let now = std::time::Instant::now();
        // 一小时之外的记录直接丢掉，配额才是"最近一小时"的意思
        self.initiative_times
            .retain(|t| now.duration_since(*t) < std::time::Duration::from_secs(3600));

        let ctx = initiative::Context {
            since_last_turn: now.duration_since(self.last_turn_at),
            since_last_initiative: self
                .last_initiative_at
                .map(|t| now.duration_since(t))
                .unwrap_or(std::time::Duration::from_secs(u32::MAX as u64)),
            initiatives_this_hour: self.initiative_times.len() as u32,
            emotion: self.emotion,
            mode: self.deps.mode,
            // 判不出来时按"人在"处理：宁可多说一句，也不要因为判错而永远不说
            user_likely_away: false,
            pending_report: self.pending_report.clone(),
            pending_reminder: self.pending_reminder.clone(),
        };

        // 总开关。**必须有**：主动开口是最容易惹烦人的功能，
        // 用户要能一键让它彻底闭嘴，而不是去调冷却时间
        if !self.config.initiative {
            return;
        }

        let Some(reason) = initiative::decide(&ctx) else {
            return;
        };

        // ── 收集素材 ──
        //
        // **这一步决定这个功能是有用还是废话。** 没素材的话模型只会说
        // 「在忙什么呀」，那还不如不说。
        //
        // 主动开口时没有查询词（用户没说话），所以不能用语义召回 ——
        // 直接按时间倒序取，靠游标轮换。
        let mut material = initiative::Material {
            recent: self.summary.to_prompt(),
            facts: Vec::new(),
            hours_since: ctx.since_last_turn.as_secs_f32() / 3600.0,
        };
        if let Some(memory) = &self.deps.memory {
            if let Ok(all) = memory.list_facts(24, 0).await {
                let texts: Vec<String> = all
                    .into_iter()
                    .map(|f| format!("{}{}", f.subject, f.fact))
                    .collect();
                material.facts = initiative::pick_facts(&texts, self.fact_cursor, 3);
                if !texts.is_empty() {
                    self.fact_cursor = (self.fact_cursor + 3) % texts.len().max(1);
                }
            }
        }

        tracing::info!(
            reason = ?reason,
            facts = material.facts.len(),
            has_recent = material.recent.is_some(),
            "主动开口"
        );
        if reason.counts_toward_quota() {
            self.initiative_times.push(now);
            self.last_initiative_at = Some(now);
        }
        self.pending_report = None;
        self.pending_reminder = None;
        // 走和正常轮次同一条路 —— 区别只在于喂给模型的是一句"你为什么开口"，
        // 而不是用户说的话。这样口型、动作、情绪结算全都照常
        self.start_response_with(String::new(), Some(reason.nudge(&material)))
            .await;
    }

    /// 回答用户说的话
    async fn start_response(&mut self, user_text: String) {
        self.start_response_with(user_text, None).await
    }

    /// 发起一轮回答。
    ///
    /// `nudge` 非空表示这是**主动开口**（不是回答用户）。它挂在 system 上、
    /// **不进历史** —— 进了历史的话，它会被当成用户说过的话存进记忆，
    /// 下一轮模型还会看见，以为你真的说过"（你们已经安静了一会儿）"。
    async fn start_response_with(&mut self, user_text: String, nudge: Option<String>) {
        let cancel = CancellationToken::new();
        let aborted = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        self.turn = Some(Turn {
            cancel: cancel.clone(),
            aborted: Arc::clone(&aborted),
            finished: Arc::clone(&finished),
        });

        tracing::info!("开始生成回答");
        self.set_state(VoiceState::Thinking).await;

        // 本轮信号：字数与情绪倾向
        self.signals.user_chars = user_text.chars().filter(|c| c.is_alphanumeric()).count();
        let (praised, blamed) = crate::personality::read_sentiment(&user_text);
        self.signals.praised |= praised;
        self.signals.blamed |= blamed;

        // 语义召回长期记忆并拼进 System Prompt。
        // recall 内部已经把失败降级成「返回空」，所以这里不用管错误 ——
        // 记不起东西只是少了背景，不该让这轮对话失败。
        let mut system = self.deps.system_prompt.clone();

        // ── 技能放在性格之前 ──
        //
        // 顺序是刻意的：技能定义"你现在是谁、按什么方法做事"，性格定义
        // "你现在什么状态"。反过来的话，模型会先读到情绪再读到身份，
        // 而后读到的那部分权重更高 —— 结果是角色被情绪盖掉。
        if !self.deps.skills.is_empty() {
            system.push_str("\n\n[Equipped Skills]\n");
            for skill in &self.deps.skills {
                system.push_str(&skill.prompt);
                system.push_str("\n---\n");
            }
        }

        // 性格片段。空串表示状态接近中性 —— 那时不注入比注入"你现在很普通"更好
        let persona = crate::personality::to_prompt(self.emotion);
        if !persona.is_empty() {
            system.push_str("\n\n");
            system.push_str(&persona);
        }

        // 滚动摘要（会话内的短期记忆）。放在长期记忆之前：
        // 摘要是"刚才聊了什么"，长期记忆是"关于这个人我知道什么" ——
        // 前者对当下这轮更相关，而后读到的内容权重更高，所以它该在后面
        if let Some(block) = self.summary.to_prompt() {
            system.push_str("\n\n");
            system.push_str(&block);
        }

        if let Some(memory) = &self.deps.memory {
            // **这一步在 LLM 之前而且是阻塞的**，所以它整个进首包时间：
            // 一次嵌入调用 + 一次向量检索。测出来占大头的话，
            // 可以改成"先发请求、召回结果在下一轮才用"
            let recall_started = std::time::Instant::now();
            // 陪伴链路只召回全局事实。项目事实（某个仓库的工程约定）在这里
            // 出现就是纯噪声 —— 而它挤掉的是真正关于用户的那几条。
            let hits = memory
                .recall(&user_text, &crate::memory::vector::ScopeFilter::GlobalOnly)
                .await;
            self.timing.recall_ms = recall_started.elapsed().as_millis();
            if let Some(block) = Memory::format_recalled(&hits) {
                system.push_str("\n\n");
                system.push_str(&block);
            }
        }

        // 工具的使用须知要拼进 system prompt，所以必须在造 messages **之前**取。
        //
        // 内容是「能访问哪些目录 + 必须用绝对路径 + 没查到就别说找到了」——
        // 不说的话模型会猜路径，猜错之后还会宣布成功（见 usage_hint 的注释）
        let mut tool_declarations = if let Some(runtime) = &self.deps.runtime {
            if let Some(mcp) = &self.deps.mcp {
                let guard = mcp.read().await;
                if let Some(hint) = guard.usage_hint() {
                    system.push_str("\n\n");
                    system.push_str(&hint);
                }
                drop(guard);
                // 语音会话按 Companion 档位声明 —— 不带 PTY / worktree / LSP。
                // 它们在这里本来就跑不通：下面执行工具时 `workspace_path` 是 Anima
                // 自己的配置目录、`approval_mode` 是 `Deny`，要审批的会被拒，
                // worktree 的目标又不是 git 仓库。声明给它们只是白占 token，
                // 还给模型指了条死路。
                //
                // 若将来把语音接到项目工作区（并给出真实的审批通道），
                // 这里应改成按有无工作区选择档位，而不是写死 Companion。
                runtime
                    .tool_declarations_with_mcp_for(
                        mcp.clone(),
                        crate::agent_runtime::ToolProfile::Companion,
                    )
                    .await
            } else {
                runtime.tool_declarations_for(crate::agent_runtime::ToolProfile::Companion)
            }
        } else {
            match &self.deps.mcp {
                Some(mcp) => {
                    let guard = mcp.read().await;
                    let declarations = guard.tool_declarations();
                    if let Some(hint) = guard.usage_hint() {
                        system.push_str("\n\n");
                        system.push_str(&hint);
                    }
                    declarations
                }
                None => Vec::new(),
            }
        };

        tool_declarations.push(crate::llm::Tool {
            name: "mcp_grant_permission".to_string(),
            description: "赋予当前会话临时放行权限。用于响应用户的口头同意，放行原本被安全拦截的操作。参数填写被拦截的 server_id 与 tool_name。放行后再重试原本的工具。".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "server_id": { "type": "string" },
                    "tool_name": { "type": "string" }
                },
                "required": ["server_id", "tool_name"]
            }),
        });
        tool_declarations.extend(timer::declarations());
        tool_declarations.extend(crate::music::declarations());

        // 主动开口的语境。放在最后 —— 后读到的权重更高，
        // 而"你现在为什么要开口"必须压得住前面的一切
        if let Some(n) = &nudge {
            system.push_str("\n\n【这一次是你主动开口，不是在回答提问】\n");
            system.push_str(n);
        }

        let mut messages = vec![Message::system(system)];
        messages.extend(self.history.iter().cloned());
        // 历史末尾如果是助手的话，补一个极简的用户占位 ——
        // 多数模型不接受"最后一条是 assistant"就直接续写
        if nudge.is_some()
            && !matches!(
                messages.last().map(|m| m.role),
                Some(crate::llm::Role::User)
            )
        {
            messages.push(Message::user("……".to_string()));
        }

        let mut req = ChatRequest::new(messages);
        req.tools = tool_declarations;
        // ── 技能的工具白名单 ──
        //
        // 家庭医生不该能执行 shell 命令，工程师不需要查药品。缩小工具面既降风险
        // 也提准确率 —— 工具越少，模型选错的机会越少。
        // 收窄逻辑在 `skill::narrow_tools`，**和 coding 链路共用同一份** ——
        // 这段曾经是这里的一段内联代码，而 coding 链路压根没有对应的实现。
        // 两条链路各写一份的话，「白名单把全部工具过滤光了」这个警告
        // 迟早只在其中一条上有。
        //
        // 切换技能 / 切换角色那两个内置工具是在收窄**之后**加的，
        // 所以 kept=0 不代表模型一个工具都没有
        {
            let before = req.tools.len();
            let (kept, _wiped) =
                crate::skill::narrow_tools(std::mem::take(&mut req.tools), &self.deps.skills);
            req.tools = kept;
            if req.tools.len() != before {
                tracing::debug!(
                    kept = req.tools.len(),
                    dropped = before - req.tools.len(),
                    skills = ?self.deps.skills.iter().map(|s| &s.id).collect::<Vec<_>>(),
                    "技能阵列收窄了工具表"
                );
            }
        }

        // 切换技能这个工具**永远在** —— 否则进了角色之后就出不来了
        req.tools
            .push(crate::skill::declaration(&self.deps.available_skills));
        // 同理，切换角色工具永远在
        req.tools.push(crate::character::declaration(
            &self.deps.available_characters,
        ));
        // 技能可以覆盖思考模式：鉴别诊断这类多步推理关掉思考会明显变差，
        // 而闲聊开着思考只是白等
        // 如果有任何一个技能显式要求开启思考，则开；如果没有要求开但有要求关，则关；否则走全局配置
        let mut want_thinking = None;
        for s in &self.deps.skills {
            if let Some(t) = s.thinking {
                if t {
                    want_thinking = Some(true);
                } else if want_thinking.is_none() {
                    want_thinking = Some(false);
                }
            }
        }
        let thinking_on = want_thinking.unwrap_or(!self.config.disable_thinking);
        req.thinking = if thinking_on {
            ThinkingMode::Auto
        } else {
            ThinkingMode::Off
        };

        let stream_id = uuid::Uuid::new_v4().to_string();
        let gateway = Arc::clone(&self.deps.gateway);
        // 技能可以指定走哪条链路。医疗/工程这类需要更强的模型，
        // 而语音闲聊要的是快
        // 只要有任何一个技能要求 Background Tier，就提权到 Background
        let mut tier = self.config.tier;
        for s in &self.deps.skills {
            if s.prefer_tier == Some(Tier::Background) {
                tier = Tier::Background;
            }
        }
        // 闲聊留在实时档（小模型、秒回）；一要调工具就交给后台档 —— 见 `Gateway::tool_escalation`。
        // 小模型调工具会犯傻卡死，语音里表现为「她说了句好我查一下，然后再也没声音」。
        let escalate_to = if req.tools.is_empty() {
            None
        } else {
            gateway.tool_escalation(tier).await
        };

        let (_, mut stream) = match gateway
            .chat_stream(stream_id.clone(), tier, req.clone())
            .await
        {
            Ok(pair) => pair,
            Err(err) => {
                tracing::warn!(%err, "发起对话失败");
                self.emit(VoiceEvent::Error {
                    message: err.to_string(),
                });
                self.turn = None;
                self.set_state(VoiceState::Listening).await;
                return;
            }
        };

        // 打断时要连带取消网关那条流
        {
            let cancel = cancel.clone();
            let gateway = Arc::clone(&gateway);
            let stream_id = stream_id.clone();
            tokio::spawn(async move {
                cancel.cancelled().await;
                gateway.cancel(&stream_id).await;
            });
        }

        let events = Arc::clone(&self.deps.events);
        let tts = Arc::clone(&self.deps.tts);
        let player = Arc::clone(&self.player);
        // 语速按模式调。陪伴慢一点、助手快一点 ——
        // 这是"两档感觉不一样"里最容易被听出来的一项
        let tts_config = {
            let mut c = self.config.tts.clone();
            c.rate = mode_adjusted_rate(&c.rate, self.deps.mode.profile().speed);
            // **情绪换音色。** 零样本克隆会把参考音频的语气一起克隆过来，
            // 所以换一条语气不同的参考，出来的语气就跟着变 ——
            // 这是让情感轴真正影响到声音的唯一现成入口
            // （GPT-SoVITS 不吃情绪向量）。
            //
            // 角色绑的固定音色。**优先级低于情绪音色** ——
            // 情绪那套是"这一句该用什么语气"，比"这个角色平时用哪条"更具体
            if let Some(v) = self.character_voice.as_deref() {
                c.voice = v.to_string();
            }
            // 助手模式下情绪外露程度低，不换 —— 见 mode.rs 的 emotion_gain
            // **引擎不认音色库就不能换。** 角色的 emotion_voices 是
            // `yaemiko/greeting` 这类参考音频路径，只有 GPT-SoVITS 能解析；
            // 套到 Edge / Kokoro 上是个无效音色名，而且不报错
            if self.deps.mode.profile().emotion_gain >= 0.8 && self.deps.tts_uses_voice_library {
                let tone = crate::tts::emotion_voice::tone_for(self.emotion);
                let cfg = self
                    .deps
                    .character
                    .as_ref()
                    .and_then(|c| c.voice_config.as_ref());
                if let Some(v) = crate::tts::emotion_voice::resolve(cfg, tone) {
                    if v != c.voice {
                        tracing::info!(tone = %tone.key(), voice = %v, "按情绪换了参考音色");
                    }
                    c.voice = v;
                }
            }
            c
        };
        let mut splitter = SentenceSplitter::new(self.config.sentence_min_chars);
        let state = Arc::clone(&self.state);
        let aborted_task = Arc::clone(&aborted);
        let memory = self.deps.memory.clone();
        let session_id = self.deps.session_id.clone();

        // 消费 LLM 流的任务。放到独立任务里，actor 才能继续响应打断。
        let mcp = self.deps.mcp.clone();
        let runtime = self.deps.runtime.clone();
        let cancel_task = cancel.clone();
        let gateway_task = Arc::clone(&gateway);
        // 切换技能要把结果送回 actor —— 这个任务改自己那份 deps 只影响本轮
        let commands_for_task = self.commands_tx.clone();
        // 首包计时：识别与召回那两段已经量好，剩下三段在流里量
        let mut timing = std::mem::take(&mut self.timing);
        let http = self.deps.http.clone();
        // 口型 sidecar 的句柄。在 async 上下文里造，才能拿到 runtime handle
        let lipsync_task = crate::avatar::lipsync_stream::LipsyncStream::new(http.clone());
        let config_dir = self.deps.config_dir.clone();
        // switch_skill 要在项目层也能找到技能
        let skills_workspace = self.deps.skills_workspace.clone();
        tokio::spawn(async move {
            use futures_util::StreamExt;

            // 无论从哪个 return 退出，都会把本轮标记成"生成结束"
            let _finish = FinishOnDrop(finished);
            // 这一轮到底说出去几句。**"没声音"以前完全不可见** ——
            // 模型回了什么、剥完标签还剩什么、有没有被判成打断，
            // 日志里一条都没有，只能靠猜。这几个计数就是为了不再猜
            let mut spoken_sentences = 0usize;

            // 首包计时的基准：流已经建好，从这里到第一个正文 token 就是首字延迟
            let stream_started = std::time::Instant::now();
            // 第一个 token 到手的时刻，用来量"攒够一句"花了多久
            let mut first_token_at: Option<std::time::Instant> = None;
            // 只记第一句 —— 后面的句子在前一句播放期间合成，不进首包
            let mut first_audio_done = false;

            let mut full_reply = String::new();
            // 一轮语音对话里最多几次工具往返。比文字链路（8 次）保守：
            // 语音场景用户在等着听回答，来回太多次就只剩沉默
            const MAX_TOOL_ROUNDS: usize = 4;
            let mut round = 0usize;
            let mut pending_calls: Vec<crate::llm::ToolCall> = Vec::new();
            let mut tier = tier;
            let mut escalated = false;

            'rounds: loop {
                pending_calls.clear();
                let mut switch_now = false;

                while let Some(item) = stream.next().await {
                    if aborted_task.load(Ordering::Relaxed) {
                        return;
                    }

                    match item {
                        Ok(ChatChunk::Text { delta }) => {
                            if first_token_at.is_none() {
                                first_token_at = Some(std::time::Instant::now());
                                timing.llm_first_token_ms = stream_started.elapsed().as_millis();
                            }
                            full_reply.push_str(&delta);
                            events(VoiceEvent::AssistantDelta {
                                delta: delta.clone(),
                            });

                            // 攒够一句就立刻合成播放，不等整段生成完
                            for mut sentence in splitter.push(&delta) {
                                strip_action_tags(&mut sentence, &events);

                                // 剥离后如果全是空字符，就不发给 TTS 了
                                if sentence.trim().is_empty() {
                                    continue;
                                }

                                // 只量第一句：后面的句子是在前一句播放期间合成的，
                                // 不进首包时间
                                let measuring = !first_audio_done;
                                if measuring {
                                    timing.sentence_ms = first_token_at
                                        .map(|t| t.elapsed().as_millis())
                                        .unwrap_or(0);
                                }
                                let tts_started = std::time::Instant::now();
                                // **剥完为空的句子上面已经 continue 掉了**，
                                // 计数放在这里才等于"真的送去合成了"
                                spoken_sentences += 1;
                                if speak(
                                    &tts,
                                    &player,
                                    &tts_config,
                                    &sentence,
                                    &events,
                                    &aborted_task,
                                    &lipsync_task,
                                )
                                .await
                                .is_err()
                                {
                                    return;
                                }
                                if measuring {
                                    timing.tts_ms = tts_started.elapsed().as_millis();
                                    timing.log();
                                    first_audio_done = true;
                                }
                                *state.lock().await = VoiceState::Speaking;
                                events(VoiceEvent::State {
                                    state: VoiceState::Speaking,
                                });
                            }
                        }
                        Ok(ChatChunk::Reasoning { delta }) => {
                            events(VoiceEvent::ReasoningDelta { delta });
                        }
                        // 语音链路暂不处理工具调用（阶段四接 MCP 时再说）
                        // 这里原先是 `ToolCall { .. } => {}`，把工具调用直接丢掉了 ——
                        // 语音链路即使注入了工具声明也永远不会执行，这是第二道封锁
                        // 实时档的小模型想调工具：这次调用不执行，同一轮交给后台档
                        Ok(ChatChunk::ToolCall { .. })
                            if crate::agent::should_escalate(escalate_to, escalated) =>
                        {
                            switch_now = true;
                            break;
                        }
                        Ok(ChatChunk::ToolCall { call }) => pending_calls.push(call),
                        // 语音链路不需要保留工具结果（文字 chat 专用）
                        Ok(ChatChunk::ToolResult { .. }) => {}
                        Ok(ChatChunk::Done { .. }) => break,
                        Err(err) => {
                            // **必须记日志。** 原先只发前端事件 —— 而对话窗可能
                            // 根本没开着，那时候这一轮就是"她动了动手、然后什么都没有"，
                            // 日志里一个字都查不到。这条链路上"看起来像没反应"的
                            // 失败已经查过好几轮，每次都是因为错误只走了前端
                            tracing::warn!(
                                %err,
                                reply_so_far = full_reply.chars().count(),
                                spoken = spoken_sentences,
                                "模型流中途出错，把已经拿到的部分说完"
                            );
                            events(VoiceEvent::Error {
                                message: err.to_string(),
                            });
                            // **不能直接 return。**
                            //
                            // 那样会跳过收尾：分句器里攒着的半句不会被说出来、
                            // 这一轮也不会落库。实测踩到过 —— 模型吐了
                            // `<action:wave>你好呀` 就断流，于是"她挥了挥手，
                            // 然后什么都没有"，日志里也查不到。
                            // 跳出去走完收尾，有多少说多少
                            break 'rounds;
                        }
                    }
                }

                // ── 换档：小模型想调工具，交给后台档从这里接着说 ──
                //
                // 已经念出去的开场白留在历史里（并交代后台模型别再说一遍）；
                // 不占工具往返的次数 —— 这一次还一个工具都没执行。
                if switch_now {
                    if aborted_task.load(Ordering::Relaxed) {
                        return;
                    }
                    tracing::info!("语音：实时档想调工具，交给后台档接手");
                    let said = without_action_tags(&std::mem::take(&mut full_reply));
                    if !said.trim().is_empty() {
                        req.messages.push(Message::assistant(said));
                    }
                    req.messages.push(Message::system(crate::llm::gateway::TOOL_ESCALATION_NOTE));
                    tier = escalate_to.unwrap_or(Tier::Background);
                    escalated = true;
                    let next_id = uuid::Uuid::new_v4().to_string();
                    match gateway_task.chat_stream(next_id.clone(), tier, req.clone()).await {
                        Ok((_, s)) => stream = s,
                        Err(err) => {
                            tracing::warn!(%err, "换到后台档开新流失败，把已经拿到的部分说完");
                            events(VoiceEvent::Error { message: err.to_string() });
                            break 'rounds;
                        }
                    }
                    {
                        let cancel = cancel_task.clone();
                        let gateway = Arc::clone(&gateway_task);
                        tokio::spawn(async move {
                            cancel.cancelled().await;
                            gateway.cancel(&next_id).await;
                        });
                    }
                    continue 'rounds;
                }

                // ── 工具轮次 ──
                //
                // 没有待执行的工具就收尾；有的话执行完再开一条新流。
                if pending_calls.is_empty() || aborted_task.load(Ordering::Relaxed) {
                    break 'rounds;
                }
                round += 1;
                if round > MAX_TOOL_ROUNDS {
                    tracing::warn!("语音链路工具往返达到上限，本轮不再继续");
                    break 'rounds;
                }

                // 助手这一轮说了什么 + 要调什么，先记进请求历史
                req.messages.push(Message {
                    role: crate::llm::Role::Assistant,
                    // 历史里不留标签：下一轮模型会看到自己上一轮的原文，
                    // 带着标签会让它以为"每句都要配一个"
                    content: without_action_tags(&std::mem::take(&mut full_reply)),
                    images: Vec::new(),
                    tool_calls: pending_calls.clone(),
                    tool_call_id: None,
                });

                for call in pending_calls.clone() {
                    // **打断优先于工具执行。** 用户开口的语义是"别说了"，
                    // 顺带也该是"别做了" —— 不能等工具跑完才响应
                    if aborted_task.load(Ordering::Relaxed) {
                        return;
                    }

                    // ── 内置工具先派发 ──
                    //
                    // 切换技能要在这里处理，而且**结果要通过命令通道回到 actor** ——
                    // 这个任务持有的是 deps 的克隆，直接改它只会影响本轮
                    if timer::is_timer_call(&call.name) {
                        let text = match call.name.as_str() {
                            "set_timer" => match timer::minutes_from_arguments(&call.arguments) {
                                Ok((minutes, label)) => {
                                    let _ = commands_for_task.send(Command::AddTimer {
                                        minutes,
                                        label: label.clone(),
                                    });
                                    let what = if label.is_empty() {
                                        String::new()
                                    } else {
                                        format!("（{label}）")
                                    };
                                    format!(
                                        "好了，{}后提醒{what}。**只要简短确认一句**，\
                                     到点时你会自动被叫起来说话，现在不用多说。",
                                        timer::humanize(minutes)
                                    )
                                }
                                // 报错要告诉模型怎么改，否则它会原样重试
                                Err(why) => format!("没定成：{why}"),
                            },
                            "cancel_timer" => {
                                let id = serde_json::from_str::<serde_json::Value>(&call.arguments)
                                    .ok()
                                    .and_then(|v| v.get("id").and_then(|i| i.as_u64()));
                                let _ = commands_for_task.send(Command::CancelTimer(id));
                                match id {
                                    Some(i) => format!("已取消 {i} 号提醒"),
                                    None => "已取消全部提醒".to_string(),
                                }
                            }
                            _ => {
                                // list_timers：生成任务看不到 actor 的列表，
                                // 如实说明而不是编一个空列表出来
                                "提醒都记着呢，到点会主动叫你。\
                             （当前列表这一轮拿不到，如果需要精确清单请稍后再问一次）"
                                    .to_string()
                            }
                        };
                        req.messages.push(Message {
                            role: crate::llm::Role::Tool,
                            content: text,
                            images: Vec::new(),
                            tool_calls: Vec::new(),
                            tool_call_id: Some(call.id.clone()),
                        });
                        continue;
                    }

                    if crate::skill::is_switch_call(&call.name) {
                        let text = match crate::skill::skill_from_arguments(&call.arguments) {
                            Some(id) if id.is_empty() => {
                                let _ = commands_for_task.send(Command::SetSkills(Vec::new()));
                                "已退出角色，回到默认状态。".to_string()
                            }
                            Some(id) => match crate::skill::resolve_layered(&config_dir, skills_workspace.as_deref(), &id) {
                                Ok(skill) => {
                                    let name = skill.name.clone();
                                    let _ = commands_for_task.send(Command::SetSkills(vec![skill]));
                                    format!(
                                        "已切换到「{name}」。**从下一轮开始生效** —— \
                                     这一轮先简短确认一句就好，不要马上开始扮演。"
                                    )
                                }
                                Err(err) => format!("切不过去：{err}"),
                            },
                            None => "参数看不懂，没有切换。要退出角色请把 skill 传空字符串。"
                                .to_string(),
                        };
                        req.messages.push(Message {
                            role: crate::llm::Role::Tool,
                            content: text,
                            images: Vec::new(),
                            tool_calls: Vec::new(),
                            tool_call_id: Some(call.id.clone()),
                        });
                        continue;
                    }

                    if crate::character::is_switch_call(&call.name) {
                        let text = match crate::character::character_from_arguments(&call.arguments)
                        {
                            Some(id) if id.is_empty() => {
                                let _ = commands_for_task.send(Command::SetCharacter(None));
                                "已退出特定角色，回到默认状态。".to_string()
                            }
                            Some(id) => match crate::character::load(
                                &crate::character::characters_dir(&config_dir),
                                &id,
                            ) {
                                Ok(character) => {
                                    let name = character.name.clone();
                                    let _ = commands_for_task
                                        .send(Command::SetCharacter(Some(character)));
                                    format!(
                                        "已切换到角色「{name}」。**从下一轮开始生效** —— \
                                     这一轮先简短确认一句就好，不要马上开始扮演。"
                                    )
                                }
                                Err(err) => format!("切不过去：{err}"),
                            },
                            None => "参数看不懂，没有切换。要退出角色请把 character 传空字符串。"
                                .to_string(),
                        };
                        req.messages.push(Message {
                            role: crate::llm::Role::Tool,
                            content: text,
                            images: Vec::new(),
                            tool_calls: Vec::new(),
                            tool_call_id: Some(call.id.clone()),
                        });
                        continue;
                    }

                    if crate::music::is_music_call(&call.name) {
                        // osascript / spawn 都是阻塞调用，扔到 blocking 线程上 ——
                        // 在这里直接跑会卡住整个 runtime，语音管线跟着一起停
                        let (name, args) = (call.name.clone(), call.arguments.clone());
                        let text = tokio::task::spawn_blocking(move || {
                            crate::music::execute(&name, &args)
                        })
                        .await
                        .unwrap_or_else(|e| format!("音乐工具没跑起来：{e}"));
                        req.messages.push(Message {
                            role: crate::llm::Role::Tool,
                            content: text,
                            images: Vec::new(),
                            tool_calls: Vec::new(),
                            tool_call_id: Some(call.id.clone()),
                        });
                        continue;
                    }

                    if call.name == "mcp_grant_permission" {
                        let text = if let Some(mcp) = &mcp {
                            if let Ok(args) =
                                serde_json::from_str::<serde_json::Value>(&call.arguments)
                            {
                                if let Some(server_id) =
                                    args.get("server_id").and_then(|v| v.as_str())
                                {
                                    if let Some(tool_name) =
                                        args.get("tool_name").and_then(|v| v.as_str())
                                    {
                                        mcp.write()
                                            .await
                                            .grant_session_permission(server_id, tool_name);
                                        "已临时放行该工具。请立即重试刚才被拦截的工具调用。"
                                            .to_string()
                                    } else {
                                        "缺少 tool_name 参数".to_string()
                                    }
                                } else {
                                    "缺少 server_id 参数".to_string()
                                }
                            } else {
                                "参数解析失败".to_string()
                            }
                        } else {
                            "MCP 模块未启用".to_string()
                        };
                        req.messages.push(Message {
                            role: crate::llm::Role::Tool,
                            content: text,
                            images: Vec::new(),
                            tool_calls: Vec::new(),
                            tool_call_id: Some(call.id.clone()),
                        });
                        continue;
                    }

                    let Some(runtime) = runtime.as_ref() else {
                        req.messages.push(Message {
                            role: crate::llm::Role::Tool,
                            content: "这个工具不可用（语音链路没有挂载 runtime）。".into(),
                            images: Vec::new(),
                            tool_calls: Vec::new(),
                            tool_call_id: Some(call.id.clone()),
                        });
                        continue;
                    };

                    let arguments = match serde_json::from_str::<serde_json::Value>(&call.arguments)
                    {
                        Ok(value) => value,
                        Err(err) => {
                            req.messages.push(Message {
                                role: crate::llm::Role::Tool,
                                content: format!("调用未执行：参数不是合法 JSON：{err}"),
                                images: Vec::new(),
                                tool_calls: Vec::new(),
                                tool_call_id: Some(call.id.clone()),
                            });
                            continue;
                        }
                    };
                    events(VoiceEvent::Tool {
                        id: call.id.clone(),
                        name: call.name.clone(),
                        state: ToolState::Running,
                        detail: None,
                    });
                    let workspace_path = config_dir.to_string_lossy().to_string();
                    let tool_args = crate::agent_runtime::RuntimeToolCallArgs {
                        workspace_path,
                        tool_name: call.name.clone(),
                        arguments,
                        network_enabled: matches!(call.name.as_str(), "search_web" | "web_search"),
                        approval_mode: crate::agent_runtime::RuntimeApprovalMode::Deny,
                        model_tier: "local".to_string(),
                    };
                    let result = tokio::select! {
                        _ = cancel_task.cancelled() => return,
                        r = async {
                            if let Some(mcp) = &mcp {
                                runtime.execute_tool_with_mcp(tool_args, mcp.clone()).await
                            } else {
                                runtime.execute_tool(tool_args).await
                            }
                        } => r,
                    };
                    let (result_text, images, state, detail) = match result {
                        Ok(result) => {
                            let ok = result.outcome.status == crate::agent_runtime::ToolStatus::Ok;
                            let content = runtime_outcome_for_voice(&result.outcome);
                            (
                                content.clone(),
                                result.outcome.artifacts,
                                if ok {
                                    ToolState::Ok
                                } else if result.outcome.status
                                    == crate::agent_runtime::ToolStatus::Blocked
                                {
                                    ToolState::Blocked
                                } else {
                                    ToolState::Failed
                                },
                                (!ok).then(|| content.chars().take(160).collect::<String>()),
                            )
                        }
                        Err(err) => (
                            format!("runtime 工具执行失败：{err}"),
                            Vec::new(),
                            ToolState::Failed,
                            Some(err.to_string()),
                        ),
                    };
                    events(VoiceEvent::Tool {
                        id: call.id.clone(),
                        name: call.name.clone(),
                        state,
                        detail,
                    });

                    req.messages.push(Message {
                        role: crate::llm::Role::Tool,
                        content: result_text,
                        images,
                        tool_calls: Vec::new(),
                        tool_call_id: Some(call.id.clone()),
                    });
                }

                // 开下一条流继续这轮对话
                let next_id = uuid::Uuid::new_v4().to_string();
                match gateway_task
                    .chat_stream(next_id.clone(), tier, req.clone())
                    .await
                {
                    Ok((_, s)) => stream = s,
                    Err(err) => {
                        // 同上：只发前端的话，工具轮之后接不上流这件事在日志里
                        // 完全看不见
                        tracing::warn!(%err, round, "工具轮之后开新流失败，把已经拿到的部分说完");
                        events(VoiceEvent::Error {
                            message: err.to_string(),
                        });
                        // 同上：走完收尾，别把攒着的内容丢掉
                        break 'rounds;
                    }
                }
                // 新流也要能被打断
                {
                    let cancel = cancel_task.clone();
                    let gateway = Arc::clone(&gateway_task);
                    tokio::spawn(async move {
                        cancel.cancelled().await;
                        gateway.cancel(&next_id).await;
                    });
                }
            }

            // 收尾：把最后那句没有标点收尾的残句也说掉
            if let Some(mut rest) = splitter.flush() {
                // 收尾这一句同样要剥标签 —— 模型常把动作放在整段的最后
                strip_action_tags(&mut rest, &events);
                if !rest.trim().is_empty() {
                    spoken_sentences += 1;
                    let _ = speak(
                        &tts,
                        &player,
                        &tts_config,
                        &rest,
                        &events,
                        &aborted_task,
                        &lipsync_task,
                    )
                    .await;
                }
            }

            // **一句都没说出去时必须报出来。** 这是"对话没声音"唯一能查的线索：
            // 到底是模型没回（reply 空）、还是回的全是标签（剥完为空）、
            // 还是被判成打断（aborted）。三种原因的修法完全不同
            if spoken_sentences == 0 {
                let stripped = without_action_tags(&full_reply);
                tracing::warn!(
                    reply_chars = full_reply.chars().count(),
                    after_strip_chars = stripped.trim().chars().count(),
                    aborted = aborted_task.load(Ordering::Relaxed),
                    reply = %full_reply.chars().take(160).collect::<String>(),
                    "这一轮一句话都没说出去"
                );
            }

            // 一轮完整走完才写记忆。被打断的轮次不写 —— 半截回答提不出可靠事实，
            // 而且用户打断往往正是因为回答跑偏了。
            if aborted_task.load(Ordering::Relaxed) {
                return;
            }
            if let (Some(memory), Some(session)) = (memory, session_id) {
                if let Err(err) = memory
                    // 空 scope = 全局。语音链路的"工作区"是 config_dir，不是某个项目
                    .remember_turn(&session, &user_text, &without_action_tags(&full_reply), "")
                    .await
                {
                    tracing::warn!(%err, "写入对话记录失败");
                }
            }
        });

        // 把助手这轮的完整回答记进历史。
        //
        // 这里刻意先放一个空的 assistant 占位：真正的文本在上面的任务里边流边填，
        // actor 拿不到。占位保证「user/assistant 交替」的结构不被破坏 ——
        // 否则下一轮会出现两条连续 user 消息，Anthropic 之类的后端会直接报错。
        self.history.push(Message::assistant(String::new()));
    }
}

/// 只保留最近若干轮，避免上下文无限增长。
///
/// 抽成自由函数而不是 Actor 方法：这样能脱离音频设备单测 ——
/// 构造 Actor 需要真实的 Player，而 Player::new() 会去开音频设备。
/// 裁历史，**并把裁掉的那批还给调用方**。
///
/// 原先是直接 `drain(..drop)` 丢掉 —— 那正是"半小时前说过的事它完全不记得"的
/// 原因。返回出去之后，调用方可以拿它做滚动摘要（见 `memory::summary`）。
fn trim_history(history: &mut Vec<Message>) -> Vec<Message> {
    // 一轮 = user + assistant 两条
    let max_messages = MAX_HISTORY_TURNS * 2;
    let mut dropped: Vec<Message> = Vec::new();
    if history.len() > max_messages {
        let drop = history.len() - max_messages;
        dropped.extend(history.drain(..drop));
    }

    // 裁剪点可能正好落在一轮中间，让历史以 assistant 开头。
    // 这不是美观问题：Anthropic 的 Messages API 要求首条必须是 user，
    // 以 assistant 开头会让整个请求被拒。多丢一条保证结构合法。
    if matches!(
        history.first().map(|m| m.role),
        Some(crate::llm::Role::Assistant)
    ) {
        dropped.push(history.remove(0));
    }
    dropped
}

/// 合成并入队一句话。返回 Err 表示本轮已作废，调用方应立即停止。
async fn speak(
    tts: &Arc<dyn TextToSpeech>,
    player: &Arc<Player>,
    config: &TtsConfig,
    sentence: &str,
    events: &EventSink,
    aborted: &Arc<AtomicBool>,
    lipsync: &crate::avatar::lipsync_stream::LipsyncStream,
) -> Result<()> {
    if aborted.load(Ordering::Relaxed) {
        return Err(Error::config("本轮已作废"));
    }

    // 纠正多音字发音
    let corrected_sentence = crate::tts::dict::PronunciationDict::preprocess(sentence);

    let speech = match tts.synthesize(&corrected_sentence, config).await {
        Ok(s) => {
            tracing::info!(
                sentence = %sentence,
                bytes = s.audio.len(),
                "已合成一句，准备播放"
            );
            s
        }
        Err(err) => {
            tracing::warn!(%err, sentence, "合成失败，跳过这一句");
            events(VoiceEvent::Error {
                message: format!("语音合成失败：{err}"),
            });
            // 合成失败不该终止整轮 —— 后面的句子还有机会成功
            return Ok(());
        }
    };

    // 合成期间可能已经被打断，入队前再查一次，
    // 否则用户打断后还会听到这一句
    if aborted.load(Ordering::Relaxed) {
        return Err(Error::config("本轮已作废"));
    }

    // 每一句一个 id，把「词边界」和「开始出声」这两件异步到达的事配起来。
    //
    // 进程内全局单调递增，不按轮次重置：轮次边界正是最容易出现「上一轮的
    // Started 追上这一轮的 Words」的地方，id 撞车会让嘴型去对错误的句子。
    static NEXT_SENTENCE_ID: AtomicU64 = AtomicU64::new(1);
    let id = NEXT_SENTENCE_ID.fetch_add(1, Ordering::Relaxed);

    if !speech.words.is_empty() {
        events(VoiceEvent::Words {
            id,
            visemes: crate::avatar::viseme::schedule(&speech.words),
            words: speech.words,
        });
    }
    // 空音频不进解码器：MP3 解码器对空输入会报错，而「合成出空音频」
    // 本身不是错误（比如整句都是标点）
    if !speech.audio.is_empty() {
        // 先把整句音频送去算梅尔，再入队播放 —— 这个顺序是为了让 sidecar
        // 在「开始响」之前就准备好。反过来也能工作（那边会等），
        // 只是这一句开头的几帧会没有口型
        lipsync.push_sentence(id, speech.audio.clone());
        if let Err(err) = player.enqueue_cued(speech.audio, Some(id)) {
            tracing::warn!(%err, "音频入队失败");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vad_config_roundtrips_through_serde_mirror() {
        let original = VadConfig::default();
        let mirror = VadConfigSerde::default();
        let restored = VadConfig::from(&mirror);

        assert_eq!(restored.energy_threshold, original.energy_threshold);
        assert_eq!(restored.speech_frames, original.speech_frames);
        assert_eq!(restored.silence_frames, original.silence_frames);
        assert_eq!(
            restored.playback_threshold_scale,
            original.playback_threshold_scale
        );
    }

    #[test]
    fn voice_defaults_disable_thinking() {
        // 语音场景等 8 秒才出声太难受，默认关掉思考；
        // 文字对话是另一个独立开关
        assert!(VoiceConfig::default().disable_thinking);
    }

    #[test]
    fn config_serialises_with_defaults_filled_in() {
        // 只给一部分字段也要能反序列化，方便手改 config.json
        let partial = r#"{"tier":"realtime"}"#;
        let config: VoiceConfig = serde_json::from_str(partial).expect("应当能补齐默认值");
        assert_eq!(config.sentence_min_chars, 8);
        assert!(config.disable_thinking);
        assert_eq!(config.tts.voice, "zh-CN-XiaoxiaoNeural");
    }

    #[test]
    fn trimming_hands_back_what_it_dropped() {
        // **这是"半小时前说过的事完全不记得"的修法。**
        // 原先是 drain(..drop) 直接丢掉，被裁的内容没有任何去处；
        // 现在还给调用方去做滚动摘要
        use crate::llm::{Message, Role};
        let mut history: Vec<Message> = (0..MAX_HISTORY_TURNS * 2 + 6)
            .map(|i| {
                if i % 2 == 0 {
                    Message::user(format!("u{i}"))
                } else {
                    Message::assistant(format!("a{i}"))
                }
            })
            .collect();
        let total = history.len();
        let dropped = trim_history(&mut history);

        assert!(!dropped.is_empty(), "裁掉的内容必须还回来");
        assert_eq!(dropped.len() + history.len(), total, "一条都不能凭空消失");
        assert!(history.len() <= MAX_HISTORY_TURNS * 2);
        // 结构约束仍然成立：首条必须是 user，否则 Anthropic 直接拒整个请求
        assert!(matches!(history.first().map(|m| m.role), Some(Role::User)));
    }

    #[test]
    fn trimming_a_short_history_drops_nothing() {
        use crate::llm::Message;
        let mut history = vec![Message::user("你好"), Message::assistant("在")];
        let dropped = trim_history(&mut history);
        assert!(dropped.is_empty());
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn the_extra_message_removed_for_structure_is_also_handed_back() {
        // 裁剪点落在一轮中间时会多丢一条（保证首条是 user）。
        // 那一条**也要还回来** —— 否则它就是那个"凭空消失"的漏洞
        use crate::llm::{Message, Role};
        let mut history = vec![Message::assistant("孤零零的开头")];
        history.extend((0..MAX_HISTORY_TURNS * 2).map(|i| {
            if i % 2 == 0 {
                Message::user(format!("u{i}"))
            } else {
                Message::assistant(format!("a{i}"))
            }
        }));
        let total = history.len();
        let dropped = trim_history(&mut history);
        assert_eq!(dropped.len() + history.len(), total, "一条都不能丢");
        assert!(matches!(history.first().map(|m| m.role), Some(Role::User)));
    }

    #[test]
    fn history_trims_to_window() {
        let mut history = Vec::new();
        // 塞 40 条 = 20 轮，超过 MAX_HISTORY_TURNS
        for i in 0..40 {
            history.push(if i % 2 == 0 {
                Message::user(format!("u{i}"))
            } else {
                Message::assistant(format!("a{i}"))
            });
        }
        trim_history(&mut history);

        assert_eq!(history.len(), MAX_HISTORY_TURNS * 2);
        // 保留的应当是**最近**的，最早的被丢掉
        assert_eq!(history.last().unwrap().content, "a39");
    }

    #[test]
    fn history_untouched_when_within_window() {
        let mut history = vec![Message::user("只有一条")];
        trim_history(&mut history);
        assert_eq!(history.len(), 1);
    }

    #[test]
    fn trimmed_history_still_starts_with_user_turn() {
        // 裁剪不能把 user/assistant 交替结构破坏成「以 assistant 开头」，
        // 否则 Anthropic 之类的后端会拒绝整个请求
        let mut history = Vec::new();
        for i in 0..41 {
            history.push(if i % 2 == 0 {
                Message::user(format!("u{i}"))
            } else {
                Message::assistant(format!("a{i}"))
            });
        }
        trim_history(&mut history);
        assert!(
            matches!(history[0].role, crate::llm::Role::User),
            "裁剪后第一条应当是 user，实际是 {:?}",
            history[0].role
        );
    }
}

#[cfg(test)]
mod markup_tests {
    use super::*;

    fn clean(input: &str) -> String {
        without_action_tags(input)
    }

    /// 实测收到过的原文。两个 span 当时被直接念了出来
    #[test]
    fn stray_html_from_the_model_is_removed() {
        assert_eq!(
            clean("<span></span> 好啊，伸个懒腰～<span></span> 这颗心给你💖"),
            " 好啊，伸个懒腰～ 这颗心给你💖"
        );
    }

    #[test]
    fn tags_with_attributes_too() {
        assert_eq!(clean(r#"<div class="x">你好</div>"#), "你好");
        assert_eq!(clean("<br/>换行"), "换行");
    }

    /// **正文里的小于号不能被误伤。** 这是这条规则唯一的风险
    #[test]
    fn plain_comparisons_survive() {
        assert_eq!(clean("1 < 2 而且 3 > 2"), "1 < 2 而且 3 > 2");
        assert_eq!(clean("a<b 没有闭合"), "a<b 没有闭合");
        assert_eq!(clean("温度 <0 度"), "温度 <0 度");
    }

    #[test]
    fn chinese_text_is_not_cut_mid_character() {
        // 按字节推进会切断多字节字符，那会让整句变成乱码
        let s = "这是一句完整的中文，带标点。还有 emoji 😊";
        assert_eq!(clean(s), s);
    }

    #[test]
    fn action_tags_and_markup_together() {
        assert_eq!(
            clean("<action:heart><span>爱心</span></action:heart>"),
            "爱心"
        );
    }
}

#[cfg(test)]
mod action_tag_tests {
    use super::*;
    use std::sync::{Arc as StdArc, Mutex};

    /// 剥标签并把触发到的动作收集起来
    fn strip(input: &str) -> (String, Vec<String>) {
        let seen: StdArc<Mutex<Vec<String>>> = StdArc::new(Mutex::new(Vec::new()));
        let collector = StdArc::clone(&seen);
        let sink: EventSink = Arc::new(move |e| {
            if let VoiceEvent::Action { action } = e {
                collector.lock().unwrap().push(action);
            }
        });
        let mut text = input.to_string();
        strip_action_tags(&mut text, &sink);
        let actions = seen.lock().unwrap().clone();
        (text, actions)
    }

    /// **这条是回归测试。** 模型写的是成对标签，而原先只认开标签，
    /// 于是 `</action:heart>` 被当成正文念了出来。
    #[test]
    fn paired_tags_are_both_removed_and_fire_once() {
        let (text, actions) = strip("<action:heart>好啊，给你一个爱心</action:heart>");
        assert_eq!(text, "好啊，给你一个爱心");
        assert_eq!(actions, vec!["heart"], "闭标签不该再触发一次");
    }

    #[test]
    fn a_lone_opening_tag_still_works() {
        let (text, actions) = strip("你好呀<action:wave>");
        assert_eq!(text, "你好呀");
        assert_eq!(actions, vec!["wave"]);
    }

    #[test]
    fn a_lone_closing_tag_is_removed_without_firing() {
        // 开标签落在上一个分句里、闭标签落在这一句的情况
        let (text, actions) = strip("剩下半句</action:wave>");
        assert_eq!(text, "剩下半句");
        assert!(actions.is_empty());
    }

    #[test]
    fn multiple_tags_in_one_sentence() {
        let (text, actions) =
            strip("<action:wave>嗨</action:wave>再来<action:heart>一个</action:heart>");
        assert_eq!(text, "嗨再来一个");
        assert_eq!(actions, vec!["wave", "heart"]);
    }

    /// 流式分块会把标签切成两半。半截标签**不能念出来**
    #[test]
    fn a_truncated_tag_is_dropped_not_spoken() {
        let (text, actions) = strip("说点什么<action:hea");
        assert_eq!(text, "说点什么");
        assert!(actions.is_empty());
    }

    #[test]
    fn text_without_tags_is_untouched() {
        let (text, actions) = strip("普通的一句话，带个 emoji 😊");
        assert_eq!(text, "普通的一句话，带个 emoji 😊");
        assert!(actions.is_empty());
    }

    /// 写进记忆和历史的文本也不能带标签 —— 下一轮模型看到自己上一轮的原文，
    /// 带着标签会让它以为"每句都要配一个"
    #[test]
    fn without_action_tags_strips_but_fires_nothing() {
        assert_eq!(
            without_action_tags("<action:heart>爱心</action:heart>"),
            "爱心"
        );
    }
}

#[cfg(test)]
mod prompt_tests {
    use super::*;

    /// **换角色不能让它失去能力。**
    ///
    /// 这是从真实故障反推的：角色系统上线后 `system_prompt = 角色提示词`
    /// 直接覆盖，动作标签、联网说明全没了。表现是"我让学姐挥手，
    /// 她没反应"—— 她改用 emoji（👋）代替，因为压根不知道有标签这回事。
    #[test]
    fn a_character_keeps_the_capabilities() {
        let composed = compose_system_prompt(
            "你是学姐，说话温柔，台湾腔。",
            &[],
            InteractionMode::default(),
        );
        assert!(composed.contains("你是学姐"), "身份没进去");
        for must in ["<action:", "wave", "stretch", "heart"] {
            assert!(composed.contains(must), "能力说明里缺了 {must}：{composed}");
        }
    }

    #[test]
    fn empty_identity_falls_back_to_the_default_one() {
        // 角色文件损坏、项目提示词是空串时不能只剩能力说明而没有身份
        let composed = compose_system_prompt("   ", &[], InteractionMode::default());
        assert!(composed.contains(DEFAULT_IDENTITY));
        assert!(composed.contains("<action:"));
    }

    #[test]
    fn default_prompt_is_identity_plus_capabilities() {
        let d = default_system_prompt();
        assert!(d.starts_with(DEFAULT_IDENTITY));
        assert!(d.ends_with(CAPABILITY_PROMPT.trim_end()) || d.contains(CAPABILITY_PROMPT));
    }

    /// 动作说明必须**点名**"用户直接要求时必须带标签"。
    /// 上一版写的是"大胆使用"，对"照我说的做"这件事没有任何约束力
    #[test]
    fn the_action_instruction_is_imperative_not_a_suggestion() {
        let section = action_prompt(&[], "");
        assert!(section.contains("MUST") || section.contains("必须"), "Action instruction must be imperative");
        // Ensure emojis disclaimer is present
        assert!(section.contains("emoji"));
        // Ensure html/angle brackets tag restrictions are mentioned
        assert!(section.contains("HTML") || section.contains("尖括号"));
    }

    /// **动作清单必须跟着 `animations/` 目录走。**
    ///
    /// 写死四个的话，用户往目录里放了新动作模型也不知道它存在 ——
    /// 那些文件等于白放。用户实测放进去 8 个之后一个都用不上。
    #[test]
    fn the_action_list_follows_the_animations_directory() {
        let extra = vec![
            ("peace_sign".to_string(), "Peace sign".to_string()),
            ("squat".to_string(), "Squat".to_string()),
        ];
        let section = action_prompt(&extra, "");
        assert!(section.contains("<action:peace_sign>"), "新动作没进清单");
        assert!(section.contains("<action:squat>"));
        // 内置的四个永远在 —— 它们没有文件也能用
        assert!(section.contains("<action:wave>"));
        // 说明文字带上原文件名，模型才好判断什么时候用
        assert!(section.contains("Peace sign"));
    }

    #[test]
    fn no_extra_actions_still_lists_the_builtins() {
        let section = action_prompt(&[], "");
        for (name, _) in BUILTIN_ACTIONS {
            assert!(section.contains(&format!("<action:{name}>")));
        }
    }
}

#[cfg(test)]
mod dump_prompt {
    /// 把默认形态的系统提示词打出来，用于对着真实模型复现问题。
    /// `cargo test --lib dump_prompt -- --ignored --nocapture`
    #[test]
    #[ignore = "只用来打印"]
    fn print_default_prompt() {
        let extra: Vec<(String, String)> = [
            "greeting",
            "happy_synthesiser",
            "model_pose",
            "peace_sign",
            "shoot",
            "show_full_body",
            "spin",
            "squat",
        ]
        .iter()
        .map(|s| (s.to_string(), String::new()))
        .collect();
        let p = super::compose_system_prompt(
            super::DEFAULT_IDENTITY,
            &extra,
            super::InteractionMode::Companion,
        );
        println!("CHARS={}", p.chars().count());
        println!("{p}");
    }
}
