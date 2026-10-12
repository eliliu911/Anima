//! 对口型的**时间对齐**测试。
//!
//! 验的是一件很容易写错、而且写错了不会报错的事：嘴型时间线的 t=0 必须是
//! 「这一句真正开始出声」的时刻，不是「这一句合成完」的时刻。
//!
//! 句子是排队播的 —— 第 N+1 句合成完的时候第 N 句往往还在播。两个时刻能差
//! 好几秒，拿错了整句嘴型全错开，而**画面上只会显得"有点不对"**，
//! 没有任何日志或报错指向原因。
//!
//! 这些测试会开真实声卡（`Player::new`）并播一小段静音，所以它们真的走完了
//! 解码 → 混音 → 取样这条路。假 TTS 返回的是无声 WAV，不会真的响。

mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anima_lib::audio::capture::CaptureEvent;
use anima_lib::audio::playback::{encode_mono_wav, Player};
use anima_lib::config::AppConfig;
use anima_lib::error::Result;
use anima_lib::llm::gateway::Gateway;
use anima_lib::stt::{SpeechToText, Transcript};
use anima_lib::tts::{estimate_word_boundaries, Speech, TextToSpeech, TtsConfig, VoiceInfo};
use anima_lib::voice::{self, VoiceConfig, VoiceDeps, VoiceEvent};
use support::{spawn, MockSpec};
use tokio::sync::mpsc;

struct FakeStt;

#[async_trait::async_trait]
impl SpeechToText for FakeStt {
    async fn transcribe(&self, _: &[f32]) -> Result<Transcript> {
        Ok(Transcript {
            text: "说点什么".into(),
            language: "zh".into(),
            confidence: 1.0,
        })
    }
}

/// 返回**真实可播的**无声 WAV 的假 TTS。
///
/// 必须返回真音频：`SentenceStart` 是在解码器交出第一个采样时报的，
/// 空音频永远不会有第一个采样 —— 那也是设计要求（对着不存在的声音报开始，
/// 前端会让嘴动一段却没有声音）。
struct WavTts {
    /// 每句音频的时长
    per_sentence: Duration,
}

#[async_trait::async_trait]
impl TextToSpeech for WavTts {
    async fn synthesize(&self, text: &str, _: &TtsConfig) -> Result<Speech> {
        let rate = 24_000u32;
        let samples = vec![0.0f32; (rate as f64 * self.per_sentence.as_secs_f64()) as usize];
        let ms = self.per_sentence.as_millis() as u64;
        Ok(Speech {
            audio: encode_mono_wav(&samples, rate),
            words: estimate_word_boundaries(text, ms),
        })
    }
    async fn voices(&self) -> Result<Vec<VoiceInfo>> {
        Ok(Vec::new())
    }
}

#[derive(Clone, Default)]
struct EventLog(Arc<Mutex<Vec<VoiceEvent>>>);

impl EventLog {
    fn sink(&self) -> voice::EventSink {
        let inner = Arc::clone(&self.0);
        Arc::new(move |event| inner.lock().unwrap().push(event))
    }
    fn snapshot(&self) -> Vec<VoiceEvent> {
        self.0.lock().unwrap().clone()
    }
    /// 等到 `pred` 成立，或超时
    async fn wait_for(&self, label: &str, timeout: Duration, pred: impl Fn(&[VoiceEvent]) -> bool) {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if pred(&self.snapshot()) {
                return;
            }
            if tokio::time::Instant::now() > deadline {
                panic!("等 {label} 超时。收到的事件：{:#?}", self.snapshot());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

fn gateway_to(base_url: &str) -> Arc<Gateway> {
    let mut config = AppConfig::default();
    for p in &mut config.providers {
        if p.id == "lmstudio" {
            p.base_url = base_url.to_string();
            p.model = "mock".into();
        }
    }
    config.realtime_provider = "lmstudio".into();
    Arc::new(Gateway::new(config).expect("网关初始化失败"))
}

/// 起一条管线，投喂一句用户语音，返回事件日志。
fn start(
    base_url: &str,
    per_sentence: Duration,
) -> (
    mpsc::UnboundedSender<CaptureEvent>,
    EventLog,
    voice::VoiceSession,
) {
    let log = EventLog::default();
    let (audio_tx, audio_rx) = mpsc::unbounded_channel();
    // 播放信号的接收端立刻 drop：管线内部另有一份用于发事件的，
    // 这里 drop 的是 Player 自己那条无人接收的诊断通道
    let (signal_tx, signal_rx) = std::sync::mpsc::channel();
    let player = Arc::new(Player::new(signal_tx).expect("打开音频输出失败"));

    // 把播放信号转成事件 —— 真实运行时这一步在 voice::start 里，
    // 而这些测试走的是 start_with_audio，得自己接上
    let events = log.sink();
    std::thread::spawn(move || {
        while let Ok(signal) = signal_rx.recv() {
            if let anima_lib::audio::playback::PlaybackSignal::Started(id) = signal {
                events(VoiceEvent::SentenceStart { id });
            }
        }
    });

    let session = voice::start_with_audio(
        VoiceConfig {
            sentence_min_chars: 2,
            ..Default::default()
        },
        VoiceDeps {
            skills_workspace: None,
            tts_uses_voice_library: false,
            // 这些测试不唤起原生应用。见 VoiceDeps::app 的注释：
            // mock_app 给的是 AppHandle<MockRuntime>，和生产的 AppHandle<Wry>
            // 不是一个类型，所以这个字段做成了 Option
            app: None,
            music_player: Default::default(),
            http: reqwest::Client::new(),
            config_dir: std::env::temp_dir(),
            // 角色/技能：这些测试验的是打断与分句的控制流，不扮演任何角色
            skills: Vec::new(),
            character: None,
            available_characters: Vec::new(),
            extra_actions: Vec::new(),
            mode: Default::default(),
            config_path: std::env::temp_dir().join("anima-test-config.json"),
            available_skills: Vec::new(),
            gateway: gateway_to(base_url),
            stt: Arc::new(FakeStt),
            tts: Arc::new(WavTts { per_sentence }),
            events: log.sink(),
            system_prompt: "测试".into(),
            memory: None,
            session_id: None,
            mcp: None,
            runtime: None,
        },
        audio_rx,
        player,
    )
    .expect("启动语音管线失败");

    let _ = audio_tx.send(CaptureEvent::SpeechStart);
    let _ = audio_tx.send(CaptureEvent::Utterance(vec![0.1; 16_000]));
    (audio_tx, log, session)
}

fn words_ids(events: &[VoiceEvent]) -> Vec<u64> {
    events
        .iter()
        .filter_map(|e| match e {
            VoiceEvent::Words { id, .. } => Some(*id),
            _ => None,
        })
        .collect()
}

fn started_ids(events: &[VoiceEvent]) -> Vec<u64> {
    events
        .iter()
        .filter_map(|e| match e {
            VoiceEvent::SentenceStart { id } => Some(*id),
            _ => None,
        })
        .collect()
}

/// 词边界事件必须带一条能用的嘴型时间线，而且 id 能和播放开始配上。
#[tokio::test]
async fn words_carry_a_viseme_timeline_paired_by_id() {
    let server = spawn(MockSpec::sse(&[
        r#"{"choices":[{"delta":{"content":"你好呀。"}}]}"#,
        "[DONE]",
    ]))
    .await;
    let (_audio, log, _session) = start(&server.base_url, Duration::from_millis(300));

    log.wait_for("播放开始", Duration::from_secs(10), |e| {
        !started_ids(e).is_empty()
    })
    .await;

    let events = log.snapshot();
    let words: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            VoiceEvent::Words { id, visemes, .. } => Some((*id, visemes.clone())),
            _ => None,
        })
        .collect();
    assert!(!words.is_empty(), "应当收到词边界事件：{events:#?}");

    let (id, visemes) = &words[0];
    assert!(
        !visemes.is_empty(),
        "词边界必须带嘴型时间线，否则前端没东西可播"
    );
    assert!(
        visemes.windows(2).all(|w| w[0].at_ms <= w[1].at_ms),
        "时间线必须有序：{visemes:?}"
    );
    assert!(
        started_ids(&events).contains(id),
        "开始播的 id 必须和词边界的 id 对上，否则前端配不上对：words={:?} started={:?}",
        words_ids(&events),
        started_ids(&events)
    );
}

/// **这是本文件的核心。**
///
/// 第二句合成完的时候第一句还在播，所以第二句的 `Words` 会明显早于它的
/// `SentenceStart`。如果前端拿收到 `Words` 的时刻当 t=0，嘴型就会整句提前。
///
/// 这条断言把这个时间差**测出来**，而不是靠推理相信它存在。
#[tokio::test]
async fn second_sentence_starts_playing_well_after_its_words_arrive() {
    let server = spawn(MockSpec::sse(&[
        r#"{"choices":[{"delta":{"content":"第一句话。"}}]}"#,
        r#"{"choices":[{"delta":{"content":"第二句话。"}}]}"#,
        "[DONE]",
    ]))
    .await;
    // 每句 700ms：第一句还在播的时候第二句早就合成完了
    let (_audio, log, _session) = start(&server.base_url, Duration::from_millis(700));

    // 等两句都收到词边界
    log.wait_for("两句的词边界", Duration::from_secs(10), |e| {
        words_ids(e).len() >= 2
    })
    .await;
    let words_at = std::time::Instant::now();
    let ids = words_ids(&log.snapshot());
    let second = ids[1];

    // 再等第二句真的开始播
    log.wait_for("第二句开始播", Duration::from_secs(15), move |e| {
        started_ids(e).contains(&second)
    })
    .await;
    let lag = words_at.elapsed();
    println!("第二句：词边界到真正出声隔了 {lag:?}");

    assert!(
        lag >= Duration::from_millis(250),
        "第二句的词边界和它开始出声之间应当有明显间隔（实测 {lag:?}）—— \
         如果这个间隔是 0，说明句子没有排队，这个测试也就失去了意义"
    );

    // 两句的 id 必须不同，否则前端会用第一句的时间线去对第二句
    assert_ne!(ids[0], ids[1], "每句必须有各自的 id");
}

/// 打断之后，还没开始播的那些句子不该再报开始。
///
/// 这条正确性是「在第一个采样上报」白送的：`interrupt()` 清空队列，
/// 被丢掉的音频永远取不到第一个采样。但**白送的正确性也要测** ——
/// 将来有人改成「入队时就报」的话，这里会立刻红。
#[tokio::test]
async fn interrupted_sentences_never_report_start() {
    let server = spawn(MockSpec::sse(&[
        r#"{"choices":[{"delta":{"content":"第一句。"}}]}"#,
        r#"{"choices":[{"delta":{"content":"第二句。"}}]}"#,
        r#"{"choices":[{"delta":{"content":"第三句。"}}]}"#,
        "[DONE]",
    ]))
    .await;
    // 每句 1.2 秒，保证打断时后面的句子还在队列里
    let (audio, log, _session) = start(&server.base_url, Duration::from_millis(1200));

    // 等第一句开播
    log.wait_for("第一句开播", Duration::from_secs(10), |e| {
        !started_ids(e).is_empty()
    })
    .await;

    // 用户开口打断
    let _ = audio.send(CaptureEvent::SpeechStart);
    log.wait_for("打断事件", Duration::from_secs(5), |e| {
        e.iter().any(|x| matches!(x, VoiceEvent::Interrupted))
    })
    .await;

    // 给足时间让「如果会误报」的话报出来
    tokio::time::sleep(Duration::from_millis(600)).await;
    let events = log.snapshot();
    let started = started_ids(&events);
    assert_eq!(
        started.len(),
        1,
        "打断后不该再有句子开始出声，实际开始了 {started:?}（词边界 {:?}）",
        words_ids(&events)
    );
}
