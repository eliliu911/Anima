//! 语音管线状态机的集成测试。
//!
//! 不碰真实麦克风：通过 `start_with_audio` 直接投喂采集事件，
//! LLM 走本地 mock HTTP 服务。这样 barge-in 这类最关键的行为才能被稳定验证。
//!
//! 输出音频设备是真实的（`Player::new()` 会开声卡），但测试用的假 TTS 返回空音频，
//! 所以不会真的出声。

mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anima_lib::audio::capture::CaptureEvent;
use anima_lib::audio::playback::Player;
use anima_lib::config::AppConfig;
use anima_lib::error::Result;
use anima_lib::llm::gateway::Gateway;
use anima_lib::stt::{SpeechToText, Transcript};
use anima_lib::tts::{Speech, TextToSpeech, TtsConfig, VoiceInfo};
use anima_lib::voice::{self, VoiceConfig, VoiceDeps, VoiceEvent, VoiceState};
use support::{spawn, spawn_many, MockSpec};
use tokio::sync::mpsc;

/// 固定返回一句话的假 STT
struct FakeStt(String);

#[async_trait::async_trait]
impl SpeechToText for FakeStt {
    async fn transcribe(&self, _: &[f32]) -> Result<Transcript> {
        Ok(Transcript {
            text: self.0.clone(),
            language: "zh".into(),
            confidence: 1.0,
        })
    }
}

/// 假 TTS：记录被要求合成过哪些句子，返回空音频（不出声）
#[derive(Clone)]
struct FakeTts {
    spoken: Arc<Mutex<Vec<String>>>,
    /// 合成耗时，用来模拟「合成还没完就被打断」
    delay: Duration,
}

impl FakeTts {
    fn new(delay: Duration) -> Self {
        Self {
            spoken: Arc::new(Mutex::new(Vec::new())),
            delay,
        }
    }
    fn sentences(&self) -> Vec<String> {
        self.spoken.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl TextToSpeech for FakeTts {
    async fn synthesize(&self, text: &str, _: &TtsConfig) -> Result<Speech> {
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        self.spoken.lock().unwrap().push(text.to_string());
        Ok(Speech {
            audio: Vec::new(),
            words: Vec::new(),
        })
    }
    async fn voices(&self) -> Result<Vec<VoiceInfo>> {
        Ok(Vec::new())
    }
}

/// 收集管线抛出的所有事件
#[derive(Clone, Default)]
struct EventLog(Arc<Mutex<Vec<VoiceEvent>>>);

impl EventLog {
    fn sink(&self) -> voice::EventSink {
        let inner = Arc::clone(&self.0);
        Arc::new(move |event| inner.lock().unwrap().push(event))
    }

    fn states(&self) -> Vec<VoiceState> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                VoiceEvent::State { state } => Some(*state),
                _ => None,
            })
            .collect()
    }

    fn transcripts(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                VoiceEvent::Transcript { text } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    fn assistant_text(&self) -> String {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                VoiceEvent::AssistantDelta { delta } => Some(delta.clone()),
                _ => None,
            })
            .collect()
    }

    fn count_turn_complete(&self) -> usize {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, VoiceEvent::TurnComplete))
            .count()
    }

    fn count_interrupted(&self) -> usize {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, VoiceEvent::Interrupted))
            .count()
    }

    fn errors(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                VoiceEvent::Error { message } => Some(message.clone()),
                _ => None,
            })
            .collect()
    }
}

/// 把网关的实时分层指到 mock 服务
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

/// 实时档、后台档各配一个模型，都指到同一个 mock 服务
fn gateway_two_tiers(base_url: &str, realtime: &str, background: &str) -> Arc<Gateway> {
    let mut config = AppConfig::default();
    for p in &mut config.providers {
        if p.id == "lmstudio" {
            p.base_url = base_url.to_string();
            p.model = realtime.into();
        }
    }
    config.realtime_provider = "lmstudio".into();
    config.background_provider = "lmstudio".into();
    config.realtime_model = Some(realtime.into());
    config.background_model = Some(background.into());
    Arc::new(Gateway::new(config).expect("网关初始化失败"))
}

/// 等待某个条件成立，最长 `timeout`。轮询而非固定 sleep，避免慢机器上假失败。
async fn wait_until(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if cond() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    cond()
}

struct Harness {
    audio: mpsc::UnboundedSender<CaptureEvent>,
    log: EventLog,
    tts: FakeTts,
    _session: voice::VoiceSession,
}

fn start_harness(base_url: &str, tts_delay: Duration, said: &str) -> Harness {
    start_harness_with(gateway_to(base_url), tts_delay, said)
}

fn start_harness_with(gateway: Arc<Gateway>, tts_delay: Duration, said: &str) -> Harness {
    let log = EventLog::default();
    let tts = FakeTts::new(tts_delay);
    let (audio_tx, audio_rx) = mpsc::unbounded_channel();
    // 播放电平在这些测试里不关心：接收端立刻 drop，Metered 的 send 会失败，
    // 而它按设计会忽略失败（音频线程上绝不能阻塞）
    let (level_tx, _) = std::sync::mpsc::channel();
    let player = Arc::new(Player::new(level_tx).expect("打开音频输出失败"));

    let session = voice::start_with_audio(
        VoiceConfig {
            // 分句阈值调小，让测试里的短句也能触发合成
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
            // 这些测试不涉及角色扮演
            // 角色/技能：这些测试验的是打断与分句的控制流，不扮演任何角色
            skills: Vec::new(),
            character: None,
            available_characters: Vec::new(),
            extra_actions: Vec::new(),
            mode: Default::default(),
            config_path: std::env::temp_dir().join("anima-test-config.json"),
            available_skills: Vec::new(),
            gateway,
            stt: Arc::new(FakeStt(said.to_string())),
            tts: Arc::new(tts.clone()),
            events: log.sink(),
            system_prompt: "测试".into(),
            // 管线测的是控制流，记忆另有专门测试；None 表示不记不召
            memory: None,
            session_id: None,
            // 这些测试不接工具：它们验证的是打断与分句，与工具无关
            mcp: None,
            runtime: None,
        },
        audio_rx,
        player,
    )
    .expect("启动语音管线失败");

    Harness {
        audio: audio_tx,
        log,
        tts,
        _session: session,
    }
}

// ---------------------------------------------------------------------------

#[tokio::test]
async fn full_turn_goes_listening_transcribing_thinking_speaking() {
    let server = spawn(MockSpec::sse(&[
        r#"{"choices":[{"delta":{"content":"你好呀。"}}]}"#,
        r#"{"choices":[{"delta":{"content":"今天不错。"}}]}"#,
        "[DONE]",
    ]))
    .await;

    let h = start_harness(&server.base_url, Duration::ZERO, "你好");

    // 投喂一句说完的音频
    h.audio
        .send(CaptureEvent::Utterance(vec![0.1; 16_000]))
        .unwrap();

    assert!(
        wait_until(Duration::from_secs(5), || h
            .log
            .assistant_text()
            .contains("今天不错"))
        .await,
        "应当收到完整回答，实际：{:?}",
        h.log.assistant_text()
    );

    assert_eq!(h.log.transcripts(), vec!["你好"]);

    let states = h.log.states();
    assert!(
        states.contains(&VoiceState::Transcribing),
        "应当经过识别态：{states:?}"
    );
    assert!(
        states.contains(&VoiceState::Thinking),
        "应当经过思考态：{states:?}"
    );

    // 两句话应当分别送去合成（边生成边说，而不是攒完一次性说）
    assert!(
        wait_until(Duration::from_secs(3), || h.tts.sentences().len() >= 2).await,
        "应当逐句合成，实际：{:?}",
        h.tts.sentences()
    );
    assert!(
        h.log.errors().is_empty(),
        "不该有错误：{:?}",
        h.log.errors()
    );
}

#[tokio::test]
async fn speech_start_during_response_interrupts_the_turn() {
    // 上游慢慢吐很多句，模拟正在长篇回答
    let events: Vec<String> = (0..60)
        .map(|i| format!(r#"{{"choices":[{{"delta":{{"content":"第{i}句话。"}}}}]}}"#))
        .collect();
    let refs: Vec<&str> = events.iter().map(|s| s.as_str()).collect();
    let server = spawn(MockSpec::sse(&refs).with_delay(Duration::from_millis(40))).await;

    let h = start_harness(&server.base_url, Duration::ZERO, "讲个长故事");

    h.audio
        .send(CaptureEvent::Utterance(vec![0.1; 16_000]))
        .unwrap();

    // 等到确实开始回答
    assert!(
        wait_until(Duration::from_secs(5), || !h
            .log
            .assistant_text()
            .is_empty())
        .await,
        "应当先开始回答"
    );
    let before = h.log.assistant_text().len();
    let sentences_before = h.tts.sentences().len();

    // 用户开口 —— 打断
    h.audio.send(CaptureEvent::SpeechStart).unwrap();

    assert!(
        wait_until(Duration::from_secs(3), || h.log.count_interrupted() >= 1).await,
        "应当抛出 Interrupted 事件"
    );

    // 打断之后文本和合成都应当停下来，不再增长
    tokio::time::sleep(Duration::from_millis(600)).await;
    let after = h.log.assistant_text().len();
    let sentences_after = h.tts.sentences().len();

    assert!(
        after - before < 200,
        "打断后不该继续产出大量文本：打断前 {before} 字节，之后 {after} 字节"
    );
    assert!(
        sentences_after - sentences_before <= 1,
        "打断后不该继续把新句子送去合成：{sentences_before} → {sentences_after}"
    );

    // 打断后应当回到听的状态
    assert!(
        wait_until(Duration::from_secs(2), || {
            h.log.states().last() == Some(&VoiceState::Listening)
        })
        .await,
        "打断后应当回到 Listening，实际状态序列：{:?}",
        h.log.states()
    );
}

#[tokio::test]
async fn speech_start_while_idle_is_not_treated_as_interruption() {
    let server = spawn(MockSpec::sse(&["[DONE]"])).await;
    let h = start_harness(&server.base_url, Duration::ZERO, "你好");

    // 没有在回答时开口，只是正常开始说话，不该报打断
    h.audio.send(CaptureEvent::SpeechStart).unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert_eq!(
        h.log.count_interrupted(),
        0,
        "空闲时开口不该算打断：{:?}",
        h.log.states()
    );
}

#[tokio::test]
async fn empty_transcript_does_not_reach_the_model() {
    // 识别为空（静音/噪声）时不该惊动模型，也不该产生一轮对话
    let server = spawn(MockSpec::sse(&["[DONE]"])).await;

    let log = EventLog::default();
    let tts = FakeTts::new(Duration::ZERO);
    let (audio_tx, audio_rx) = mpsc::unbounded_channel();
    // 播放电平在这些测试里不关心：接收端立刻 drop，Metered 的 send 会失败，
    // 而它按设计会忽略失败（音频线程上绝不能阻塞）
    let (level_tx, _) = std::sync::mpsc::channel();
    let player = Arc::new(Player::new(level_tx).expect("打开音频输出失败"));

    let _session = voice::start_with_audio(
        VoiceConfig::default(),
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
            // 这些测试不涉及角色扮演
            // 角色/技能：这些测试验的是打断与分句的控制流，不扮演任何角色
            skills: Vec::new(),
            character: None,
            available_characters: Vec::new(),
            extra_actions: Vec::new(),
            mode: Default::default(),
            config_path: std::env::temp_dir().join("anima-test-config.json"),
            available_skills: Vec::new(),
            gateway: gateway_to(&server.base_url),
            // 空识别结果
            stt: Arc::new(FakeStt(String::new())),
            tts: Arc::new(tts.clone()),
            events: log.sink(),
            system_prompt: "测试".into(),
            // 管线测的是控制流，记忆另有专门测试；None 表示不记不召
            memory: None,
            session_id: None,
            // 这些测试不接工具：它们验证的是打断与分句，与工具无关
            mcp: None,
            runtime: None,
        },
        audio_rx,
        player,
    )
    .expect("启动失败");

    audio_tx
        .send(CaptureEvent::Utterance(vec![0.0; 16_000]))
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    assert!(log.transcripts().is_empty(), "空识别不该抛 Transcript");
    assert!(tts.sentences().is_empty(), "空识别不该触发合成");
    assert!(
        !log.states().contains(&VoiceState::Thinking),
        "空识别不该进入思考态：{:?}",
        log.states()
    );
}

#[tokio::test]
async fn tts_synthesised_after_interruption_is_not_enqueued() {
    // 合成很慢：打断发生在「合成完成之前」，那一句合成好了也不能再入队，
    // 否则用户打断后还会听到这一句
    let server = spawn(MockSpec::sse(&[
        r#"{"choices":[{"delta":{"content":"这句会被打断。"}}]}"#,
        "[DONE]",
    ]))
    .await;

    let h = start_harness(&server.base_url, Duration::from_millis(800), "说点什么");

    h.audio
        .send(CaptureEvent::Utterance(vec![0.1; 16_000]))
        .unwrap();

    // 等到 LLM 文本已到、TTS 正在慢慢合成
    assert!(
        wait_until(Duration::from_secs(5), || !h
            .log
            .assistant_text()
            .is_empty())
        .await,
        "应当先收到文本"
    );

    // 在合成完成前打断
    h.audio.send(CaptureEvent::SpeechStart).unwrap();
    assert!(
        wait_until(Duration::from_secs(2), || h.log.count_interrupted() >= 1).await,
        "应当抛出 Interrupted"
    );

    // 给足时间让那次慢合成跑完
    tokio::time::sleep(Duration::from_millis(1200)).await;

    // 合成本身可能已经完成（无法取消 await 中的 HTTP），
    // 但关键是它不能被播放 —— 状态不该停留在 Speaking
    assert!(
        h.log.states().last() != Some(&VoiceState::Speaking),
        "打断后不该处于 Speaking：{:?}",
        h.log.states()
    );
}

#[tokio::test]
async fn manual_interrupt_behaves_like_barge_in() {
    let events: Vec<String> = (0..40)
        .map(|i| format!(r#"{{"choices":[{{"delta":{{"content":"句{i}。"}}}}]}}"#))
        .collect();
    let refs: Vec<&str> = events.iter().map(|s| s.as_str()).collect();
    let server = spawn(MockSpec::sse(&refs).with_delay(Duration::from_millis(40))).await;

    let h = start_harness(&server.base_url, Duration::ZERO, "讲故事");
    h.audio
        .send(CaptureEvent::Utterance(vec![0.1; 16_000]))
        .unwrap();

    assert!(
        wait_until(Duration::from_secs(5), || !h
            .log
            .assistant_text()
            .is_empty())
        .await,
        "应当先开始回答"
    );

    // 界面上点「停止」，走的是同一条打断路径
    h._session.interrupt();

    assert!(
        wait_until(Duration::from_secs(2), || h.log.count_interrupted() >= 1).await,
        "手动打断也应当抛 Interrupted"
    );
}

/// 一轮正常说完之后，状态必须回到 Listening —— 而且随后用户正常开口
/// **不能**被算成打断。
///
/// 这是个真实踩过的 bug：收尾条件写的是 `self.turn.is_none()`，而 turn 只在
/// abort_turn 里被 take，正常说完永远不满足。于是状态永远停在 Speaking，
/// 用户每一次正常开口都被判成 barge-in —— 心情每轮 -3 起、连击递增，
/// 十几轮就锁死在 -100，好感度则因为只在 completed 时上升而几乎不动。
/// 光看画面完全看不出来，是翻日志里连片的 `completed=false` 才发现的。
#[tokio::test]
async fn a_completed_turn_returns_to_listening_and_the_next_utterance_is_not_a_barge_in() {
    let server = spawn(MockSpec::sse(&[
        r#"{"choices":[{"delta":{"content":"好的。"}}]}"#,
        "[DONE]",
    ]))
    .await;

    let h = start_harness(&server.base_url, Duration::ZERO, "你好");

    h.audio
        .send(CaptureEvent::Utterance(vec![0.1; 16_000]))
        .unwrap();

    // 等这一轮收尾
    assert!(
        wait_until(Duration::from_secs(5), || h.log.count_turn_complete() >= 1).await,
        "一轮说完应当抛 TurnComplete，实际状态序列：{:?}",
        h.log.states()
    );

    // 收尾后必须落回 Listening，否则下一次开口会被当成打断
    assert_eq!(
        h.log.states().last(),
        Some(&VoiceState::Listening),
        "说完之后应当回到 Listening：{:?}",
        h.log.states()
    );

    // 现在用户正常开口。这是新一轮的开始，不是打断
    h.audio.send(CaptureEvent::SpeechStart).unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert_eq!(
        h.log.count_interrupted(),
        0,
        "说完之后再开口是新一轮，不该抛 Interrupted"
    );
}

/// **断流不能把已经拿到的内容丢掉。**
///
/// 真实事故：模型吐了 `<action:wave>你好呀` 之后连接断了。原先的代码
/// 在流出错时直接 `return`，于是收尾那段没跑 —— 分句器里攒着的半句
/// 没被说出去、这一轮也没落库。用户看到的是"她挥了挥手，然后什么都没有"，
/// 而且日志里一个字都查不到（错误只发给了前端，而对话窗当时没开）。
///
/// 现在改成 `break 'rounds`，走完收尾：有多少说多少。
#[tokio::test]
async fn a_broken_stream_still_says_what_it_already_got() {
    // 吐半句就断：没有 [DONE]，连接直接关掉
    let server = spawn(MockSpec {
        status: 200,
        content_type: "text/event-stream",
        chunks: vec![r#"data: {"choices":[{"delta":{"content":"你好呀"}}]}"#.to_string() + "\n\n"],
        delay: Duration::from_millis(1),
    })
    .await;

    let h = start_harness(&server.base_url, Duration::ZERO, "你好");
    h.audio
        .send(CaptureEvent::Utterance(vec![0.1; 16_000]))
        .unwrap();

    assert!(
        wait_until(Duration::from_secs(5), || !h.tts.sentences().is_empty()).await,
        "断流之前拿到的半句必须说出去，实际一句都没说：{:?}",
        h.tts.sentences()
    );
    assert!(
        h.tts.sentences().iter().any(|s| s.contains("你好呀")),
        "说出去的应该是已经拿到的那半句：{:?}",
        h.tts.sentences()
    );
}

/// **并发启动只能开出一个会话。**
///
/// 真实事故：两个窗口同时点麦克风，两次 `voice_start` 都看到"没在跑"、
/// 都往下走，于是同时开了几个采集实例抢同一个麦克风 —— 表现是
/// "语音无法开启"，日志里能看到连续几条「开始麦克风采集」而中间
/// 没有任何「已停止」。
///
/// 根因是"检查有没有在跑"和"把会话存进去"之间隔着两百多行异步操作
/// （建 STT、组装提示词、开采集），窗口大得离谱。而前端的 busy 标志
/// 挡不住 —— 每个窗口是独立 webview，那是**各自的** busy。
///
/// 这里用同一段逻辑的最小复现：一个"检查—等待—写入"的临界区，
/// 并发跑很多次，断言只有一个赢家。
#[tokio::test]
async fn a_check_then_assign_critical_section_needs_a_lock() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Mutex as AsyncMutex;

    async fn race(lock: Option<Arc<AsyncMutex<()>>>) -> usize {
        let slot = Arc::new(AsyncMutex::new(false));
        let winners = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();

        for _ in 0..8 {
            let slot = Arc::clone(&slot);
            let winners = Arc::clone(&winners);
            let lock = lock.clone();
            tasks.push(tokio::spawn(async move {
                let _guard = match &lock {
                    Some(l) => Some(l.lock().await),
                    None => None,
                };
                // 检查
                if *slot.lock().await {
                    return;
                }
                // 中间隔着一段异步操作 —— 真实代码里是建 STT、开采集
                tokio::time::sleep(Duration::from_millis(20)).await;
                // 写入
                *slot.lock().await = true;
                winners.fetch_add(1, Ordering::Relaxed);
            }));
        }
        for t in tasks {
            let _ = t.await;
        }
        winners.load(Ordering::Relaxed)
    }

    // 没有锁：所有人都挤进去了 —— 这正是"语音无法开启"的成因
    let without = race(None).await;
    assert!(without > 1, "没有锁的话本该抢进去多个，实际 {without}");

    // 有锁：只有一个赢家
    let with = race(Some(Arc::new(AsyncMutex::new(())))).await;
    assert_eq!(with, 1, "有锁之后只能有一个会话，实际 {with}");
}


/// **实时档的小模型一要调工具，就交给后台档**（2026-09-26 用户定的）。
///
/// 语音要秒回，所以实时档配小模型；可小模型调工具会犯傻卡死 ——
/// 表现是「她说了句好我查一下，然后再也没声音」。这里验三件事：
/// 1. 第二次请求发给的是后台档的模型，并带着「由你接手」的交代；
/// 2. 小模型的那次工具调用**没被执行**（后面没有第三次请求、没有工具结果）；
/// 3. 开场白照常念了，后台模型的回答也念了。
#[tokio::test]
async fn realtime_tool_call_is_handed_to_background_tier() {
    let server = spawn_many(vec![
        MockSpec::sse(&[
            r#"{"choices":[{"delta":{"content":"好，我查一下。"}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","type":"function","function":{"name":"switch_character","arguments":"{}"}}]}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            "[DONE]",
        ]),
        MockSpec::sse(&[
            r#"{"choices":[{"delta":{"content":"后台模型来回答。"}}]}"#,
            "[DONE]",
        ]),
    ])
    .await;

    let h = start_harness_with(gateway_two_tiers(&server.base_url, "small-e4b", "big-35b"), Duration::ZERO, "帮我查一下");
    h.audio.send(CaptureEvent::Utterance(vec![0.1; 16_000])).unwrap();

    assert!(
        wait_until(Duration::from_secs(5), || h.log.assistant_text().contains("后台模型来回答")).await,
        "后台档的回答没出来，实际：{:?}，错误：{:?}",
        h.log.assistant_text(),
        h.log.errors()
    );
    let bodies = server.request_bodies();
    assert_eq!(bodies.len(), 2, "应该正好两次请求（小模型一次、后台一次），实际 {}", bodies.len());
    assert_eq!(bodies[0]["model"], "small-e4b");
    assert_eq!(bodies[1]["model"], "big-35b", "想调工具之后没有换到后台档");
    let second = bodies[1]["messages"].to_string();
    assert!(second.contains("由你接手"), "后台模型没收到接手的交代：{second}");
    assert!(second.contains("好，我查一下"), "已经念出去的开场白没留在历史里");
    assert!(!second.contains("\"tool_call_id\""), "小模型的工具调用被执行了（历史里出现了工具结果）");
    assert!(
        wait_until(Duration::from_secs(3), || h.tts.sentences().iter().any(|s| s.contains("后台模型来回答"))).await,
        "后台模型的回答没念出来：{:?}",
        h.tts.sentences()
    );
}
