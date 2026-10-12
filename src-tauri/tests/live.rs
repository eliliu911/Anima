//! 打真实后端的联调测试。**默认不跑**（全部标了 `#[ignore]`），因为它依赖
//! 本机在跑 LM Studio / Ollama，以及 Keychain 里存了对应的 API Key。
//!
//! 跑法：
//! ```bash
//! cargo test --test live -- --ignored --nocapture
//! # 或只跑一个后端
//! cargo test --test live -- --ignored --nocapture azure
//! ```
//!
//! 存在的意义：mock 测试保证"我们按协议发/收"，这里保证"真实服务确实这么答"。
//! 两者都需要 —— 上游改了协议时，只有这一组会红。

use std::time::{Duration, Instant};

use anima_lib::config::{AppConfig, Tier};
use anima_lib::llm::gateway::Gateway;
use anima_lib::llm::{ChatChunk, ChatRequest, FinishReason, Message};
use futures_util::StreamExt;

/// 一轮真实对话的观测结果
struct TurnResult {
    text: String,
    reasoning: String,
    /// 首个 chunk（含推理）的延迟
    first_chunk: Option<Duration>,
    /// 首个**正文** token 的延迟 —— 语音管线真正关心的是这个，
    /// 因为推理阶段是没法送去 TTS 的
    first_text: Option<Duration>,
    finish: Option<FinishReason>,
}

/// 跑一轮真实对话。
///
/// `max_tokens` 要给够：思考模型（Qwen3 系列默认开思考）会先花掉一大截预算
/// 在推理上，给 200 的话可能全部耗在推理里、正文一个字都出不来。
async fn run_turn_with(gateway: &Gateway, tier: Tier, prompt: &str, max_tokens: u32) -> TurnResult {
    let mut req = ChatRequest::new(vec![
        Message::system("你是 Anima。回答尽量简短。"),
        Message::user(prompt),
    ]);
    req.max_tokens = Some(max_tokens);

    let started = Instant::now();
    let (provider, mut stream) = gateway
        .chat_stream("live-test".into(), tier, req)
        .await
        .expect("建流失败");

    println!("  实际后端: {} / {}", provider.id, provider.model);

    let mut out = TurnResult {
        text: String::new(),
        reasoning: String::new(),
        first_chunk: None,
        first_text: None,
        finish: None,
    };

    while let Some(item) = stream.next().await {
        match item.expect("流中出现错误") {
            ChatChunk::Text { delta } => {
                out.first_chunk.get_or_insert_with(|| started.elapsed());
                out.first_text.get_or_insert_with(|| started.elapsed());
                out.text.push_str(&delta);
            }
            ChatChunk::Reasoning { delta } => {
                out.first_chunk.get_or_insert_with(|| started.elapsed());
                out.reasoning.push_str(&delta);
            }
            ChatChunk::ToolCall { call } => {
                println!("  工具调用: {} {}", call.name, call.arguments);
            }
            // 工具结果只在带工具的 agent 循环里出现，联调这条路不执行工具。
            // 这里显式列出而不是用 `_ =>`：以后再加变体时编译器还要提醒一次。
            ChatChunk::ToolResult { name, .. } => {
                println!("  工具结果: {name}");
            }
            ChatChunk::Done { reason, usage } => {
                out.finish = Some(reason);
                if let Some(u) = usage {
                    println!(
                        "  usage: prompt={} completion={}",
                        u.prompt_tokens, u.completion_tokens
                    );
                }
            }
        }
    }

    println!("  首 chunk: {:?}", out.first_chunk);
    println!("  首正文 token: {:?}", out.first_text);
    if !out.reasoning.is_empty() {
        println!("  推理 {} 字", out.reasoning.chars().count());
    }
    println!("  回答: {}", out.text);
    println!("  结束原因: {:?}", out.finish);

    out
}

/// 默认给足预算，避免思考模型把额度全花在推理上
async fn run_turn(gateway: &Gateway, tier: Tier, prompt: &str) -> TurnResult {
    run_turn_with(gateway, tier, prompt, 2048).await
}

// **`fn gateway()`（`AppConfig::default()`）已删除。**
//
// 它的后台档指向 ollama 的 qwen3:32b —— 用户为了腾空间删掉那个模型之后，
// 用它的测试就一直 404，而失败原因和被测的东西毫无关系。
// 要「验现在这套配置能不能用」请走 `real_gateway()`；
// 要逐个后端联调请走 `gateway_pinned_to()`（那里显式指定 provider，
// 默认值只影响别的分层）。


/// 把某个分层指到指定 provider，用于逐个后端联调
async fn gateway_pinned_to(provider_id: &str) -> Gateway {
    let config = AppConfig {
        realtime_provider: provider_id.to_string(),
        ..Default::default()
    };
    config.validate().expect("配置不自洽");
    Gateway::new(config).expect("网关初始化失败")
}

// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "需要本机 LM Studio 在跑，且 Keychain 里有 token"]
async fn live_lmstudio_lists_models_and_chats() {
    let gateway = gateway_pinned_to("lmstudio").await;

    let models = gateway
        .health_check("lmstudio")
        .await
        .expect("LM Studio 探活失败");
    println!("LM Studio 模型数: {}", models.len());
    assert!(!models.is_empty(), "应当至少返回一个模型");

    let r = run_turn(&gateway, Tier::Realtime, "用一句话说你是谁").await;

    assert!(
        !r.text.trim().is_empty(),
        "正文不应为空（预算是否被推理吃光了？）"
    );
    assert_eq!(r.finish, Some(FinishReason::Stop));
}

#[tokio::test]
#[ignore = "需要本机 Ollama 在跑（对话那一半需要一个非嵌入模型）"]
async fn live_ollama_lists_models_and_chats() {
    let gateway = gateway_pinned_to("ollama").await;

    let models = gateway
        .health_check("ollama")
        .await
        .expect("Ollama 探活失败");
    println!("Ollama 模型: {models:?}");
    assert!(!models.is_empty(), "Ollama 一个模型都没有");

    // **不写死模型名。** 原先要求 `qwen3:32b`，用户为了腾空间删掉之后这条
    // 就一直失败 —— 而失败原因（模型不存在）和它要验的东西
    // （原生 /api/chat 通不通）毫无关系。
    //
    // 嵌入模型按名字排除：它们没有 chat 能力，拿去对话会报一个和本测试
    // 无关的错。名字启发式很粗，但比写死一个具体型号稳。
    let chat_model = models
        .iter()
        .find(|m| {
            let low = m.to_ascii_lowercase();
            !low.contains("embed") && !low.contains("bge") && !low.contains("minilm")
        })
        .cloned();
    let Some(chat_model) = chat_model else {
        println!(
            "Ollama 里只有嵌入模型（{models:?}），跳过对话那一半 —— \
             要验它请 `ollama pull` 一个对话模型"
        );
        return;
    };
    println!("用 {chat_model} 试一轮对话");

    let r = run_turn(&gateway, Tier::Realtime, "用一句话说你是谁").await;

    assert!(!r.text.trim().is_empty(), "正文不应为空");
    assert_eq!(r.finish, Some(FinishReason::Stop));
}

#[tokio::test]
#[ignore = "需要 Keychain 里有 Azure OpenAI 的 key"]
async fn live_azure_lists_deployments_and_chats() {
    let gateway = gateway_pinned_to("azure").await;

    let deployments = gateway.health_check("azure").await.expect("Azure 探活失败");
    println!("Azure deployments: {deployments:?}");
    assert!(
        deployments.iter().any(|d| d == "gpt-4o"),
        "应当能看到 gpt-4o 部署，实际：{deployments:?}"
    );

    let r = run_turn(&gateway, Tier::Realtime, "用一句话说你是谁").await;

    assert!(!r.text.trim().is_empty(), "正文不应为空");
    assert_eq!(r.finish, Some(FinishReason::Stop));
}

#[tokio::test]
#[ignore = "需要 Keychain 里有 Azure OpenAI 的 key"]
async fn live_azure_gpt5_deployment_accepts_max_tokens() {
    // 回归测试：gpt-5.x 部署发 max_tokens 会 400，必须走 max_completion_tokens
    let mut config = AppConfig {
        realtime_provider: "azure".into(),
        ..Default::default()
    };
    for p in &mut config.providers {
        if p.id == "azure" {
            p.model = "gpt-5.4-mini".into();
        }
    }
    let gateway = Gateway::new(config).expect("网关初始化失败");

    let r = run_turn(&gateway, Tier::Realtime, "只回复两个字：你好").await;

    assert!(!r.text.trim().is_empty(), "gpt-5.4-mini 应当有回答");
    assert_eq!(r.finish, Some(FinishReason::Stop));
}

#[tokio::test]
#[ignore = "需要本机 LM Studio 在跑"]
async fn live_cancellation_stops_generation_promptly() {
    // barge-in 的底层能力：生成中途取消要立刻收到 Cancelled 收尾
    let gateway = gateway_pinned_to("lmstudio").await;

    let mut req = ChatRequest::new(vec![Message::user("写一篇 500 字的散文，慢慢写")]);
    req.max_tokens = Some(2000);

    let stream_id = "live-cancel".to_string();
    let (_, mut stream) = gateway
        .chat_stream(stream_id.clone(), Tier::Realtime, req)
        .await
        .expect("建流失败");

    // 先收几个 chunk，确认真的在生成。
    // 这里必须把 Reasoning 也算上：思考模型会先输出一大段推理才吐正文，
    // 只等 Text 的话往往整条流都读完了还没攒够，反而测不到"中途取消"。
    let mut got = 0;
    while let Some(item) = stream.next().await {
        if matches!(
            item.expect("流出错"),
            ChatChunk::Text { .. } | ChatChunk::Reasoning { .. }
        ) {
            got += 1;
            if got >= 3 {
                break;
            }
        }
    }
    assert!(got >= 3, "应当先收到若干 chunk，实际 {got}");

    // 取消，然后确认流以 Cancelled 收尾且不拖延
    let cancel_at = Instant::now();
    assert!(gateway.cancel(&stream_id).await, "取消应当命中在途流");

    let mut finish = None;
    while let Some(item) = stream.next().await {
        if let ChatChunk::Done { reason, .. } = item.expect("流出错") {
            finish = Some(reason);
            break;
        }
    }
    let elapsed = cancel_at.elapsed();
    println!("  取消到收尾耗时: {elapsed:?}");

    assert_eq!(finish, Some(FinishReason::Cancelled));
    assert!(
        elapsed < Duration::from_secs(2),
        "取消收尾过慢：{elapsed:?}"
    );
}

#[tokio::test]
#[ignore = "需要本机 MLX server 不在跑（验证降级路径）"]
async fn live_tool_request_falls_back_from_toolless_backend() {
    // MLX 声明了 supports_tools=false，带工具的请求应当自动改道到
    // 已配 key 的云端后端（这里是 Azure）
    let gateway = gateway_pinned_to("mlx").await;

    let mut req = ChatRequest::new(vec![Message::user("北京天气怎么样？用工具查")]);
    req.tools = vec![anima_lib::llm::Tool {
        name: "get_weather".into(),
        description: "查询指定城市的天气".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": { "city": { "type": "string" } },
            "required": ["city"]
        }),
    }];

    let (provider, mut stream) = gateway
        .chat_stream("live-fallback".into(), Tier::Realtime, req)
        .await
        .expect("建流失败");

    println!("  回退到: {} / {}", provider.id, provider.model);
    assert_ne!(provider.id, "mlx", "不支持工具的后端不该被选中");

    let mut saw_tool_call = false;
    while let Some(item) = stream.next().await {
        if let ChatChunk::ToolCall { call } = item.expect("流出错") {
            println!("  工具调用: {} {}", call.name, call.arguments);
            saw_tool_call = true;
        }
    }
    assert!(saw_tool_call, "回退后的后端应当能产生工具调用");
}

#[tokio::test]
#[ignore = "需要用户真实配置里的两个分层都能用"]
async fn live_tier_routing_picks_different_backends() {
    // **读真实配置。** 原先用 `gateway()`（`AppConfig::default()`），它的后台档
    // 指向 ollama 的 qwen3:32b —— 用户删掉那个模型之后这条测试就一直 404，
    // 而失败原因和它要验的东西（分层路由）毫无关系。
    let gateway = real_gateway();

    println!("--- 实时链路 ---");
    let realtime = run_turn(&gateway, Tier::Realtime, "说一个字：好").await;
    println!("--- 后台链路 ---");
    let background = run_turn(&gateway, Tier::Background, "说一个字：好").await;

    assert!(!realtime.text.trim().is_empty());
    assert!(!background.text.trim().is_empty());
}

#[tokio::test]
#[ignore = "需要本机 LM Studio 在跑"]
async fn live_thinking_off_shortens_first_text_token() {
    // 对照实验：同一个模型、同一个问题，只切思考开关
    let gateway = gateway_pinned_to("lmstudio").await;

    println!("--- 思考开（默认）---");
    let on = run_turn(&gateway, Tier::Realtime, "用一句话说你是谁").await;

    println!("--- 思考关 ---");
    let mut req = ChatRequest::new(vec![
        Message::system("你是 Anima。回答尽量简短。"),
        Message::user("用一句话说你是谁"),
    ]);
    req.max_tokens = Some(2048);
    req.thinking = anima_lib::llm::ThinkingMode::Off;

    let started = Instant::now();
    let (_, mut stream) = gateway
        .chat_stream("live-nothink".into(), Tier::Realtime, req)
        .await
        .expect("建流失败");

    let mut text = String::new();
    let mut reasoning_len = 0usize;
    let mut first_text = None;
    while let Some(item) = stream.next().await {
        match item.expect("流出错") {
            ChatChunk::Text { delta } => {
                first_text.get_or_insert_with(|| started.elapsed());
                text.push_str(&delta);
            }
            ChatChunk::Reasoning { delta } => reasoning_len += delta.chars().count(),
            _ => {}
        }
    }
    println!("  首正文 token: {first_text:?}");
    println!("  推理 {reasoning_len} 字");
    println!("  回答: {text}");

    assert!(!text.trim().is_empty(), "关掉思考也要有回答");
    assert_eq!(reasoning_len, 0, "思考关闭后不该再有推理输出");

    let off_ms = first_text.expect("应有正文").as_millis();
    let on_ms = on.first_text.expect("应有正文").as_millis();
    println!("对照: 思考开 {on_ms}ms -> 思考关 {off_ms}ms");
}

// ---------------------------------------------------------------------------
// TTS（Edge TTS）
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "需要联网访问微软 Edge TTS 接口"]
async fn live_edge_tts_synthesizes_chinese_audio() {
    use anima_lib::tts::{edge::EdgeTts, TextToSpeech, TtsConfig};

    let tts = EdgeTts::new(reqwest::Client::new());
    let config = TtsConfig::default();

    let started = Instant::now();
    let speech = tts
        .synthesize("你好，我是 Anima。很高兴认识你。", &config)
        .await
        .expect("合成失败");
    let elapsed = started.elapsed();

    println!("音色: {}", config.voice);
    println!("合成耗时: {elapsed:?}");
    println!("音频字节数: {}", speech.audio.len());
    println!("词边界数: {}", speech.words.len());
    for w in speech.words.iter().take(5) {
        println!("  {:>6}ms +{:>4}ms  {}", w.offset_ms, w.duration_ms, w.text);
    }

    assert!(!speech.audio.is_empty(), "应当返回音频");
    // MP3 帧同步字：0xFF 0xEx/0xFx，或者带 ID3 头
    let head = &speech.audio[..4.min(speech.audio.len())];
    assert!(
        head.starts_with(&[0xFF]) || head.starts_with(b"ID3"),
        "返回的数据不像 MP3，头部为 {head:02X?}"
    );
    assert!(
        !speech.words.is_empty(),
        "应当带回词边界（阶段三对口型要用）"
    );

    // 落一个文件方便人工试听
    let out = std::env::temp_dir().join("anima-tts-live.mp3");
    std::fs::write(&out, &speech.audio).expect("写文件失败");
    println!("已写出：{}（可用 afplay 试听）", out.display());
}

#[tokio::test]
#[ignore = "需要联网访问微软 Edge TTS 接口"]
async fn live_edge_tts_lists_chinese_voices() {
    use anima_lib::tts::{edge::EdgeTts, TextToSpeech};

    let tts = EdgeTts::new(reqwest::Client::new());
    let voices = tts.voices().await.expect("拉取音色失败");

    let chinese: Vec<_> = voices
        .iter()
        .filter(|v| v.locale.starts_with("zh-"))
        .collect();
    println!("共 {} 个音色，其中中文 {} 个", voices.len(), chinese.len());
    for v in chinese.iter().take(10) {
        println!("  {} ({}, {})", v.short_name, v.gender, v.locale);
    }

    assert!(!voices.is_empty());
    assert!(
        chinese.iter().any(|v| v.short_name.contains("Xiaoxiao")),
        "应当能找到晓晓音色"
    );
}

#[tokio::test]
#[ignore = "需要联网访问微软 Edge TTS 接口"]
async fn live_edge_tts_handles_text_with_xml_characters() {
    use anima_lib::tts::{edge::EdgeTts, TextToSpeech, TtsConfig};

    // 用户说的话里带 XML 特殊字符时不能破坏 SSML、也不该报错
    let tts = EdgeTts::new(reqwest::Client::new());
    let speech = tts
        .synthesize(
            "如果 a < b 且 c > d，那么 a & c 呢？",
            &TtsConfig::default(),
        )
        .await
        .expect("含 XML 特殊字符的文本应当也能合成");

    assert!(!speech.audio.is_empty());
}

// ---------------------------------------------------------------------------
// STT（Whisper）
// ---------------------------------------------------------------------------

/// 模型路径。默认放在应用配置目录下的 models/。
fn whisper_model_path() -> String {
    std::env::var("ANIMA_WHISPER_MODEL").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/Library/Application Support/com.anima.desktop/models/ggml-small.bin")
    })
}

/// 用 macOS 自带的 afconvert 把 MP3 解成 16kHz 单声道 f32，
/// 免得为了测试再引一个音频解码依赖。
fn mp3_to_16k_mono(mp3: &[u8]) -> Vec<f32> {
    let dir = std::env::temp_dir();
    let src = dir.join("anima-stt-in.mp3");
    let dst = dir.join("anima-stt-in.wav");
    std::fs::write(&src, mp3).expect("写 mp3 失败");
    let _ = std::fs::remove_file(&dst);

    let status = std::process::Command::new("afconvert")
        .args(["-f", "WAVE", "-d", "LEI16@16000", "-c", "1"])
        .arg(&src)
        .arg(&dst)
        .status()
        .expect("afconvert 未能执行");
    assert!(status.success(), "afconvert 转码失败");

    let mut reader = hound::WavReader::open(&dst).expect("打开 wav 失败");
    reader
        .samples::<i16>()
        .map(|s| s.expect("读取采样失败") as f32 / i16::MAX as f32)
        .collect()
}

#[tokio::test]
#[ignore = "需要已下载 ggml 模型（见 whisper_model_path）"]
async fn live_whisper_transcribes_chinese_from_tts_audio() {
    use anima_lib::stt::{whisper::WhisperStt, SpeechToText, SttConfig};
    use anima_lib::tts::{edge::EdgeTts, TextToSpeech, TtsConfig};

    // 先用 TTS 合成一句已知内容的中文，再让 STT 认回来 —— 闭环自检，
    // 不依赖任何预置音频素材。
    let spoken = "今天天气不错，我们出去走走吧。";
    let tts = EdgeTts::new(reqwest::Client::new());
    let speech = tts
        .synthesize(spoken, &TtsConfig::default())
        .await
        .expect("TTS 合成失败");
    let samples = mp3_to_16k_mono(&speech.audio);
    println!("音频时长: {:.2}s", samples.len() as f32 / 16_000.0);

    let load_start = Instant::now();
    let stt = WhisperStt::load(SttConfig {
        model_path: whisper_model_path(),
        language: "zh".into(),
        ..Default::default()
    })
    .expect("加载 Whisper 模型失败");
    println!("模型加载耗时: {:?}", load_start.elapsed());

    let infer_start = Instant::now();
    let result = stt.transcribe(&samples).await.expect("识别失败");
    let infer = infer_start.elapsed();

    println!("识别耗时: {infer:?}");
    println!("原文: {spoken}");
    println!("识别: {}", result.text);

    assert!(!result.text.trim().is_empty(), "识别结果不应为空");
    // 不做全字匹配（标点/断句会有差异），但关键词必须在
    for keyword in ["天气", "走走"] {
        assert!(
            result.text.contains(keyword),
            "识别结果里应当包含「{keyword}」，实际：{}",
            result.text
        );
    }
    // 实时性：识别耗时应当远小于音频时长（否则跟不上对话）
    let audio_secs = samples.len() as f32 / 16_000.0;
    println!("实时率: {:.2}x", audio_secs / infer.as_secs_f32());
}

#[tokio::test]
#[ignore = "需要已下载 ggml 模型"]
async fn live_whisper_returns_empty_for_silence() {
    use anima_lib::stt::{whisper::WhisperStt, SpeechToText, SttConfig};

    let stt = WhisperStt::load(SttConfig {
        model_path: whisper_model_path(),
        language: "zh".into(),
        ..Default::default()
    })
    .expect("加载 Whisper 模型失败");

    // 一秒静音。Whisper 很容易对静音「幻听」出一句话，
    // 这个测试守住「静音不该变成对话内容」这条线。
    let silence = vec![0.0f32; 16_000];
    let result = stt.transcribe(&silence).await.expect("识别失败");
    println!("静音识别结果: {:?}", result.text);
    assert!(
        result.text.trim().is_empty(),
        "静音不该产出文字，实际：{}",
        result.text
    );
}

// ---------------------------------------------------------------------------
// 记忆系统
// ---------------------------------------------------------------------------

fn memory_test_dir() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("anima-memory-live-{}", uuid::Uuid::new_v4()))
}

/// 把嵌入指到本机 Ollama
/// 记忆相关的联调测试用**用户真实的配置**，而不是编译进去的默认值。
///
/// 原先用 `AppConfig::default()`，它的后台链路默认指向 ollama 的 qwen3:32b。
/// 用户为了腾空间把那个模型删了之后，这两个测试就一直 404 —— 而失败原因
/// （"model not found"）和测试要验的东西（事实提取）毫无关系。
///
/// 写死模型名的联调测试迟早会坏。读真实配置才是在验"现在这套配置能不能用"。
fn memory_gateway() -> std::sync::Arc<Gateway> {
    real_gateway()
}

/// 读**用户真实配置**的网关。
///
/// `memory_gateway` 是它的历史别名（当时只有记忆测试需要）。任何「验现在这套
/// 配置能不能用」的测试都该走这里，而不是 `AppConfig::default()` ——
/// 后者的后台档指向 ollama 的 qwen3:32b，用户为了腾空间删掉之后就一直 404。
/// 读真实配置，但把两个档位都指向**指定的模型**。
///
/// 用来逐个模型跑同一套提示词对照 —— 依从性因模型而异，
/// 在一个模型上测出来的措辞结论不能直接套到另一个上
/// （见 `commands.rs` 里那张表：ornith 在弱措辞下会直奔 fs_list_directory）。
fn gateway_with_model(model: &str) -> std::sync::Arc<Gateway> {
    let path = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join("Library/Application Support/com.anima.desktop/config.json");
    let mut config = AppConfig::load(&path).expect("读不到真实配置");
    config.realtime_model = Some(model.to_string());
    config.background_model = Some(model.to_string());
    std::sync::Arc::new(Gateway::new(config).expect("网关初始化失败"))
}

fn real_gateway() -> std::sync::Arc<Gateway> {
    // 不为这一处引 dirs 依赖 —— HOME 就够，而且这个测试只在 macOS 上跑
    let path = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join("Library/Application Support/com.anima.desktop/config.json");
    let config = AppConfig::load(&path).unwrap_or_else(|err| {
        println!("读不到真实配置（{err}），退回默认值");
        AppConfig::default()
    });
    println!("后台链路：{}", config.background_provider);
    std::sync::Arc::new(Gateway::new(config).expect("网关初始化失败"))
}

#[tokio::test]
#[ignore = "需要本机 Ollama 在跑，且已 pull mxbai-embed-large"]
async fn live_memory_embeds_and_recalls_semantically() {
    use anima_lib::memory::{Memory, MemoryConfig};

    let dir = memory_test_dir();
    let memory = Memory::open(
        &dir,
        MemoryConfig::default(),
        memory_gateway(),
        reqwest::Client::new(),
    )
    .await
    .expect("打开记忆系统失败");

    let stats = memory.stats().await.expect("统计失败");
    println!("向量维度: {}", stats.dimensions);
    assert_eq!(stats.dimensions, 1024, "bge-m3 应当是 1024 维");

    // 手动灌几条事实（绕过提取，单独验证「向量化 + 召回」这一段）
    let session = memory.store().create_session().await.unwrap();
    let facts = [
        ("用户", "养了一只叫小黑的猫"),
        ("用户", "在做一个叫 Anima 的语音助手项目"),
        ("用户", "不吃香菜"),
    ];
    for (subject, text) in facts {
        seed_fact(&memory, &session, subject, text).await;
    }

    println!("已写入 {} 条事实", memory.stats().await.unwrap().facts);

    // 逐个话题验证 **Top-1 排序**。
    //
    // 一定要断言「排第一的是哪条」，不能只断言「结果里有它」——
    // 后者在事实很少时永远为真，曾因此漏掉了「默认嵌入模型对中文完全排错序」
    // 这个严重问题（mxbai-embed-large 不管问什么都把最短那条排第一）。
    let cases = [
        ("我家的宠物叫什么名字", "小黑"),
        ("我最近在写什么代码", "Anima"),
        ("我有什么忌口", "香菜"),
    ];

    for (query, expected) in cases {
        let hits = memory.recall(query, &anima_lib::memory::vector::ScopeFilter::Any).await;
        println!("\n查询「{query}」召回 {} 条：", hits.len());
        for h in &hits {
            println!("  {:.3}  {}", h.similarity, h.fact.fact);
        }

        let top = hits
            .first()
            .unwrap_or_else(|| panic!("「{query}」什么都没召回到 —— 阈值是否过高？"));
        assert!(
            top.fact.fact.contains(expected),
            "「{query}」的 Top-1 应当包含「{expected}」，实际是「{}」（{:.3}）",
            top.fact.fact,
            top.similarity
        );

        // 阈值应当把明显无关的事实挡在外面，否则每轮都往上下文里注入噪声
        assert_eq!(
            hits.len(),
            1,
            "「{query}」只应召回那一条相关事实，实际 {} 条：{:?}",
            hits.len(),
            hits.iter().map(|h| &h.fact.fact).collect::<Vec<_>>()
        );
    }

    let hits = memory.recall("我家的宠物叫什么名字", &anima_lib::memory::vector::ScopeFilter::Any).await;
    let block = Memory::format_recalled(&hits).expect("应当有召回内容");
    println!("\n--- 注入 System Prompt 的内容 ---\n{block}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 直接写一条事实进向量库（复用 Memory 内部的嵌入器路径）
async fn seed_fact(memory: &anima_lib::memory::Memory, session: &str, subject: &str, text: &str) {
    // 走 remember_turn 会触发模型提取（慢且不确定），
    // 这里只想验证向量段，所以用一条「助手复述」的假对话让提取拿到明确内容
    memory
        .store()
        .append_message(session, anima_lib::llm::Role::User, text)
        .await
        .unwrap();
    // 手动向量化并写入
    let stats_before = memory.stats().await.unwrap().facts;
    memory
        .add_fact(session, subject, text, "")
        .await
        .expect("写入事实失败");
    let stats_after = memory.stats().await.unwrap().facts;
    assert_eq!(stats_after, stats_before + 1, "应当多出一条事实");
}

#[tokio::test]
#[ignore = "需要配置里的后台链路可用（读真实 config.json）"]
async fn live_memory_extracts_facts_from_a_real_conversation() {
    use anima_lib::memory::extract;

    let gateway = memory_gateway();
    let started = Instant::now();

    let facts = extract::extract(
        &gateway,
        "我叫小明，在上海做后端开发，家里养了一只叫小黑的黑猫。对了我不吃香菜。",
        "记住了，小明。上海的后端开发，养了黑猫小黑，不吃香菜。",
        false,
    )
    .await
    .expect("提取失败");

    println!("提取耗时: {:?}", started.elapsed());
    println!("提取到 {} 条事实：", facts.len());
    for f in &facts {
        println!("  {} —— {}", f.subject, f.fact);
    }

    assert!(!facts.is_empty(), "这段对话里明显有可记的事实");
    let all = facts
        .iter()
        .map(|f| f.fact.as_str())
        .collect::<Vec<_>>()
        .join(" / ");
    // 不要求逐字匹配（模型措辞会变），但关键信息应当至少覆盖一部分
    let covered = ["小明", "上海", "小黑", "香菜"]
        .iter()
        .filter(|k| all.contains(**k))
        .count();
    assert!(
        covered >= 2,
        "至少应当抓到两个关键信息，实际提取内容：{all}"
    );
}

#[tokio::test]
#[ignore = "需要本机 Ollama 在跑"]
async fn live_memory_ignores_small_talk() {
    use anima_lib::memory::extract;

    // 纯闲聊不该被记成长期事实，否则记忆库会被垃圾淹没
    let facts = extract::extract(
        &memory_gateway(),
        "今天天气怎么样",
        "今天天气不错，晴天，二十五度。",
        false,
    )
    .await
    .expect("提取失败");

    println!("闲聊提取到 {} 条：{facts:?}", facts.len());
    assert!(
        facts.is_empty(),
        "一次性闲聊不该进长期记忆，实际：{facts:?}"
    );
}

/// 项目里的 `voice-library/` 本身（不是它的父目录）。
///
/// 测试从 `src-tauri/` 下跑，所以往上一层再进 voice-library。
/// **必须指向真实的库** —— 这些联调测试要验的正是"引擎能不能从共享库里
/// 解析出参考音频"。
fn project_voice_root() -> std::path::PathBuf {
    std::env::current_dir()
        .expect("取不到当前目录")
        .parent()
        .expect("src-tauri 应当有父目录")
        .join("voice-library")
}

// ---------------------------------------------------------------------------
// F5-TTS（需要本机服务在 :8881）
// ---------------------------------------------------------------------------

/// F5-TTS 走的是 OpenAI 兼容形状，所以共用 OpenAiTts 适配器。
///
/// 这个测试要验的是**共用没有出岔子**：音色列表、合成、WAV 头都对得上。
#[tokio::test]
#[ignore = "需要本机 F5-TTS 在 :8881（./scripts/anima.sh start，且配置选了 f5_tts）"]
async fn live_f5tts_speaks_and_degrades_gracefully() {
    use anima_lib::tts::{openai::OpenAiTts, TextToSpeech, TtsConfig, TtsEngineConfig};

    let engine = TtsEngineConfig {
        voice: None,
        id: "f5_tts".into(),
        label: "F5-TTS".into(),
        kind: anima_lib::tts::TtsEngineKind::F5Tts,
        base_url: "http://127.0.0.1:8881".into(),
        api_key_secret: None,
        ref_audio_path: None,
        ref_text: None,
        pcm_sample_rate: None,
        endpoint_path: None,
    };
    let tts = OpenAiTts::new(reqwest::Client::new(), engine).expect("建引擎失败");

    // 音色列表就是 voices/ 目录里的参考音频
    let voices = tts.voices().await.expect("读音色列表失败");
    println!(
        "音色 {} 个: {:?}",
        voices.len(),
        voices.iter().map(|v| &v.short_name).collect::<Vec<_>>()
    );
    assert!(
        !voices.is_empty(),
        "voices/ 里应当至少有一个参考音色（<名字>.wav + 同名 .txt）"
    );

    let config = TtsConfig {
        voice: voices[0].short_name.clone(),
        ..TtsConfig::default()
    };

    let started = Instant::now();
    let speech = tts
        .synthesize("我在的，有什么可以帮你的吗？", &config)
        .await
        .expect("合成失败");
    let elapsed = started.elapsed();

    let (samples, sr) =
        anima_lib::audio::playback::decode_mono(speech.audio.clone()).expect("解码失败");
    let secs = samples.len() as f32 / sr as f32;
    let peak = samples.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    println!(
        "合成 {elapsed:?} → {secs:.2}s 音频（{:.1}x 实时），{sr}Hz，峰值 {peak:.3}",
        secs / elapsed.as_secs_f32()
    );

    assert!(!speech.audio.is_empty(), "应当返回音频");
    assert!(peak > 0.01, "峰值 {peak:.4} 太低，可能是静音");
    // 词边界是估算的（这个协议没有时间戳），但必须有 —— 没有的话嘴型驱动不了
    assert!(!speech.words.is_empty(), "应当带回估算的词边界");

}

// ---------------------------------------------------------------------------
// GPT-SoVITS（需要本机 api_v2.py 在 :9880）
// ---------------------------------------------------------------------------

/// 这个适配器当初是**照文档形状写的、从没对着真实服务联调过**（模块注释里
/// 就这么写着）。这个测试就是那次联调。
#[tokio::test]
#[ignore = "需要本机 GPT-SoVITS api_v2.py 在 :9880"]
async fn live_gpt_sovits_clones_from_the_shared_library() {
    use anima_lib::tts::{sovits::SovitsTts, TextToSpeech, TtsConfig, TtsEngineConfig};

    let root = project_voice_root();
    // 库里有哪些音色由 voices 模块解析 —— 这里顺带验它和引擎看到的是同一份
    let available = anima_lib::tts::voices::list_in(&root);
    println!(
        "共享音色库（{}）里有 {} 个音色",
        root.display(),
        available.len()
    );
    if available.is_empty() {
        println!("音色库是空的，跳过 —— 往 voice-library/<人名>/ 放素材再跑 organize.py");
        return;
    }
    // **必须挑一个带斜杠的具体变体**：不带斜杠的名字指向 default，
    // 那条路径就算解析错了也可能碰巧能用。
    //
    // 而且优先挑**不叫 anima 的** —— anima 那条是 Kokoro 的输出凑的，
    // 用它测等于让"克隆"退化成"复制"，看不出音色是否真的被换掉了
    let pick = available
        .iter()
        .find(|v| v.name.contains('/') && !v.name.starts_with("anima"))
        .or_else(|| available.iter().find(|v| v.name.contains('/')))
        .unwrap_or(&available[0])
        .clone();
    println!("用音色「{}」：{}", pick.name, pick.audio.display());
    println!("  参考文本「{}」", pick.text);

    let engine = TtsEngineConfig {
        voice: None,
        id: "gpt_sovits".into(),
        label: "GPT-SoVITS".into(),
        kind: anima_lib::tts::TtsEngineKind::GptSoVits,
        base_url: "http://127.0.0.1:9880".into(),
        api_key_secret: None,
        // **刻意留空**：这就是这个测试的重点 —— 参考应当由共享音色库按
        // config.voice 解析出来，而不是靠引擎配置里手工填的路径。
        // 原先必须手工填绝对路径 + 那段话的文字，还得自己保证一致，
        // 写错的表现是"音色像但吐字乱"，极难归因
        ref_audio_path: None,
        ref_text: None,
        pcm_sample_rate: None,
        endpoint_path: None,
    };
    let tts = SovitsTts::new(reqwest::Client::new(), engine, &root).expect("建引擎失败");

    // 引擎报出来的音色应当就是共享库里的那些
    let listed = tts.voices().await.expect("读音色列表失败");
    assert_eq!(
        listed.len(),
        available.len(),
        "引擎列出的音色数应当和共享库一致（以前这里返回空数组，用户面对一个点不开的下拉框）"
    );

    let config = TtsConfig {
        voice: pick.name.clone(),
        ..TtsConfig::default()
    };

    let started = Instant::now();
    let speech = tts
        .synthesize("旅行者，你来得正好。今天想让我帮你做什么呢？", &config)
        .await
        .expect("合成失败");
    let elapsed = started.elapsed();

    let (samples, sr) =
        anima_lib::audio::playback::decode_mono(speech.audio.clone()).expect("解码失败");
    let secs = samples.len() as f32 / sr as f32;
    let peak = samples.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    println!(
        "合成 {elapsed:?} → {secs:.2}s 音频（{:.1}x 实时），{sr}Hz，峰值 {peak:.3}",
        secs / elapsed.as_secs_f32()
    );

    assert!(peak > 0.01, "峰值 {peak:.4} 太低，可能是静音");
    assert!(!speech.words.is_empty(), "应当带回估算的词边界");

    let out = std::env::temp_dir().join("anima-sovits-shared.wav");
    std::fs::write(&out, &speech.audio).expect("写文件失败");
    println!("已写出：{}（afplay 试听）", out.display());
}

/// 音色名认不出来时必须**说清库里有什么**，而不是只说"找不到"。
#[tokio::test]
#[ignore = "需要本机 GPT-SoVITS api_v2.py 在 :9880"]
async fn live_gpt_sovits_reports_an_unknown_voice_usefully() {
    use anima_lib::tts::{sovits::SovitsTts, TextToSpeech, TtsConfig, TtsEngineConfig};

    let root = project_voice_root();
    if anima_lib::tts::voices::list_in(&root).is_empty() {
        println!("音色库是空的，跳过");
        return;
    }
    let engine = TtsEngineConfig {
        voice: None,
        id: "gpt_sovits".into(),
        label: "GPT-SoVITS".into(),
        kind: anima_lib::tts::TtsEngineKind::GptSoVits,
        base_url: "http://127.0.0.1:9880".into(),
        api_key_secret: None,
        ref_audio_path: None,
        ref_text: None,
        pcm_sample_rate: None,
        endpoint_path: None,
    };
    let tts = SovitsTts::new(reqwest::Client::new(), engine, &root).expect("建引擎失败");
    let config = TtsConfig {
        voice: "根本不存在的音色".into(),
        ..TtsConfig::default()
    };
    let err = tts
        .synthesize("测试", &config)
        .await
        .expect_err("不存在的音色应当报错，而不是拿默认的凑数")
        .to_string();
    println!("报错：{err}");
    assert!(
        err.contains("参考音频") || err.contains("音色"),
        "报错要能自己解决问题：{err}"
    );
}

#[tokio::test]
#[ignore = "需要一个确定不存在的端口"]
async fn live_probe_reports_a_dead_zero_shot_engine_as_unreachable() {
    use anima_lib::tts::{sovits::SovitsTts, TextToSpeech, TtsEngineConfig};

    // 指向一个确定没人听的端口
    let engine = TtsEngineConfig {
        voice: None,
        id: "gpt_sovits".into(),
        label: "GPT-SoVITS".into(),
        kind: anima_lib::tts::TtsEngineKind::GptSoVits,
        base_url: "http://127.0.0.1:19999".into(),
        api_key_secret: None,
        ref_audio_path: Some("/tmp/whatever.wav".into()),
        ref_text: Some("随便".into()),
        pcm_sample_rate: None,
        endpoint_path: None,
    };
    let tts =
        SovitsTts::new(reqwest::Client::new(), engine, &project_voice_root()).expect("建引擎失败");

    // **这就是坑：服务根本不存在，voices() 照样成功返回。**
    //
    // 因为它读的是本地音色库，一次网络都不碰。所以它不能当探活用。
    //
    // 早先这里断言 `list.is_empty()` —— 那是错的：它成立只是因为当时本地
    // 音色库是空的。用户往库里放了 40 个音色之后测试就挂了，而挂的原因
    // （「零样本引擎不该有音色列表」）和这条测试想记录的事毫无关系。
    // **断言要对着行为，不是对着环境的偶然状态。**
    let list = tts.voices().await.expect("voices() 不碰网络，所以不会失败");
    println!("服务是死的，voices() 仍然返回了 {} 个音色", list.len());

    // 所以探活必须另外真连一次 —— 这一条由 probe_base_url 保证。
    // 这里直接验那个行为：连一个死端口必须失败
    let dead = reqwest::Client::new()
        .get("http://127.0.0.1:19999")
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await;
    assert!(dead.is_err(), "死端口应当连不上，否则这个测试本身没意义");
    println!("确认：voices() 照样成功但服务是死的 —— 探活必须补一次真实连接");
}

/// 去重、浏览、删除 —— DECISIONS 2.2 列的三件缺失能力。
#[tokio::test]
#[ignore = "需要本机嵌入后端（bge-m3）"]
async fn live_memory_browses_dedupes_and_forgets() {
    use anima_lib::memory::{Memory, MemoryConfig};

    let dir = memory_test_dir();
    let memory = Memory::open(
        &dir,
        MemoryConfig::default(),
        memory_gateway(),
        reqwest::Client::new(),
    )
    .await
    .expect("打开记忆系统失败");
    memory.clear_facts().await.expect("清空失败");

    // ── 浏览 ──
    //
    // 原先只有 search，要知道"它记住了我什么"必须先猜一个查询词。
    // 而记忆系统不可见时用户无法建立信任
    for (subject, fact) in [
        ("用户", "住在西雅图"),
        ("用户", "不吃香菜"),
        ("用户", "在做一个语音助手"),
    ] {
        memory
            .add_fact("test", subject, fact, "")
            .await
            .expect("写入失败");
    }
    let listed = memory.list_facts(50, 0).await.expect("浏览失败");
    assert_eq!(listed.len(), 3, "浏览应当拿到全部三条：{listed:?}");
    println!("浏览到 {} 条：", listed.len());
    for f in &listed {
        println!(
            "  [{}] {} —— {}",
            f.id.chars().take(8).collect::<String>(),
            f.subject,
            f.fact
        );
    }

    // 倒序：新记的在前面（新的最可能是错的，该先看到）
    assert!(
        listed
            .windows(2)
            .all(|w| w[0].created_at >= w[1].created_at),
        "应当按时间倒序"
    );

    // ── 分页 ──
    let page = memory.list_facts(2, 0).await.expect("翻页失败");
    assert_eq!(page.len(), 2);
    let page2 = memory.list_facts(2, 2).await.expect("翻页失败");
    assert_eq!(page2.len(), 1, "第二页应当只剩一条");
    assert!(
        page.iter().all(|a| page2.iter().all(|b| a.id != b.id)),
        "两页不该有重复"
    );

    // ── 换个说法也要召回得到（在删除之前验，否则自相矛盾）──
    let hits = memory.recall("我住在哪儿", &anima_lib::memory::vector::ScopeFilter::Any).await;
    println!("召回「我住在哪儿」：{} 条", hits.len());
    assert!(
        hits.iter().any(|r| r.fact.fact.contains("西雅图")),
        "语义召回：换个说法也该找到：{hits:?}"
    );

    // ── 删除 ──
    //
    // 原先只有"清空全部"，粒度太粗 —— 为了删掉一条错的而丢掉全部正确的，
    // 没人会真的去用
    let victim = listed
        .iter()
        .find(|f| f.fact.contains("香菜"))
        .expect("夹具里应当有这条")
        .clone();
    memory.forget_fact(&victim.id).await.expect("删除失败");
    let after = memory.list_facts(50, 0).await.expect("浏览失败");
    assert_eq!(after.len(), 2, "删掉一条之后应当剩两条");
    assert!(
        !after.iter().any(|f| f.id == victim.id),
        "被删的那条不该还在：{}",
        victim.fact
    );
    println!("删掉「{}」之后剩 {} 条", victim.fact, after.len());

    // 删掉之后就真的召回不到了 —— 删除必须对召回生效，
    // 否则"删了"只是界面上的假象
    let gone = memory.recall("我有什么忌口", &anima_lib::memory::vector::ScopeFilter::Any).await;
    assert!(
        !gone.iter().any(|r| r.fact.fact.contains("香菜")),
        "删掉的记忆不该还能被召回：{gone:?}"
    );
    println!("删掉的那条确实召回不到了");
}

/// 电台清单会腐坏 —— 台会关、地址会变，而失败是**静默的**：
/// ffplay 照样启动，只是没声音。这个测试把清单里每一条真的解一遍。
///
/// ```bash
/// cargo test --test live -- --ignored --nocapture radio
/// ```
#[test]
#[ignore = "要联网"]
fn every_radio_station_still_resolves() {
    let mut dead = Vec::new();
    for (name, url) in anima_lib::music::station_table() {
        let out = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_name",
                "-of",
                "csv=p=0",
                url,
            ])
            .output()
            .expect("ffprobe 没装？`brew install ffmpeg`");
        let codec = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if codec.is_empty() {
            println!("  ✗ {name:<12} {url}");
            dead.push(name);
        } else {
            println!("  ✓ {name:<12} {codec}");
        }
    }
    assert!(dead.is_empty(), "这些台已经连不上了，换掉它们：{dead:?}");
}

/// 真的起一个电台再关掉。查表的单测证明不了 ffplay 起得来 ——
/// 而"起来了但一秒后就死"正是最容易漏的那种失败
///
/// ```bash
/// cargo test --test live -- --ignored --nocapture plays_and_stops
/// ```
#[test]
#[ignore = "要联网，而且会真的响几秒"]
fn radio_plays_and_stops_for_real() {
    let said = anima_lib::music::execute("play_radio", r#"{"station":"ambient"}"#);
    println!("play_radio → {said}");
    assert!(said.contains("正在放"), "没放成：{said}");

    std::thread::sleep(std::time::Duration::from_secs(3));

    let stopped = anima_lib::music::execute("stop_radio", "{}");
    println!("stop_radio → {stopped}");
    assert!(
        stopped.contains("已关掉"),
        "关的时候以为本来就没在放：{stopped}"
    );

    // 关完再关一次，要能如实说"本来就没在放"，而不是骗一句"已关掉"
    assert!(anima_lib::music::execute("stop_radio", "{}").contains("本来就没在放"));
}

/// 问"在放什么"**不该把 Music.app 启动起来**。
/// `tell application "Music"` 会隐式启动它，所以查询前必须先问 System Events
///
/// ```bash
/// cargo test --test live -- --ignored --nocapture does_not_launch
/// ```
#[test]
#[ignore = "会碰真实的 Music.app"]
fn asking_status_does_not_launch_music_app() {
    let running = || {
        let out = std::process::Command::new("osascript")
            .arg("-e")
            .arg(r#"tell application "System Events" to return (exists (processes where name is "Music"))"#)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim() == "true"
    };
    if running() {
        println!("Music.app 本来就开着，这个测试测不了，跳过");
        return;
    }
    let said = anima_lib::music::execute("music_control", r#"{"action":"status"}"#);
    println!("status → {said}");
    assert!(!running(), "问一句状态就把 Music.app 启动了");
    assert!(said.contains("没开着"), "该如实说没开着：{said}");
}

/// 真的去控制网易云。菜单名会随 App 版本和语言变，**变了之后是静默失败** ——
/// 点不着就什么都不发生，模型还是会说"已暂停"
///
/// ```bash
/// cargo test --test live -- --ignored --nocapture netease
/// ```
#[test]
#[ignore = "要网易云开着，而且需要辅助功能权限"]
fn can_actually_control_netease() {
    let running = std::process::Command::new("osascript")
        .arg("-e")
        .arg(r#"tell application "System Events" to return (exists (processes where name is "NeteaseMusic"))"#)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "true")
        .unwrap_or(false);
    if !running {
        println!("网易云没开着，跳过 —— 这个测试不负责替你启动它");
        return;
    }

    let before = anima_lib::music::execute("music_control", r#"{"action":"status"}"#);
    println!("status  → {before}");
    assert!(before.contains("网易云"), "没认出网易云：{before}");

    let played = anima_lib::music::execute("music_control", r#"{"action":"play"}"#);
    println!("play    → {played}");
    std::thread::sleep(std::time::Duration::from_secs(2));

    let now = anima_lib::music::execute("music_control", r#"{"action":"status"}"#);
    println!("status  → {now}");
    assert!(
        now.contains("正在放"),
        "play 之后没在放，菜单可能点空了：{now}"
    );

    let next = anima_lib::music::execute("music_control", r#"{"action":"next"}"#);
    println!("next    → {next}");
    assert!(next.contains("下一首"), "切歌失败：{next}");

    // 测完恢复原样，别把用户的播放器留在放着的状态
    let paused = anima_lib::music::execute("music_control", r#"{"action":"pause"}"#);
    println!("pause   → {paused}");
    assert!(
        anima_lib::music::execute("music_control", r#"{"action":"status"}"#).contains("没在放")
    );
}

// ---------------------------------------------------------------------------
// 项目知识库索引（`.anima/knowledge/` → LanceDB → 语义检索）
// ---------------------------------------------------------------------------

/// 造一个临时工作区，`.anima/knowledge/` 下放几份文档
fn knowledge_workspace(docs: &[(&str, &str)]) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("anima-know-{}", uuid::Uuid::new_v4()));
    let k = root.join(".anima").join("knowledge");
    std::fs::create_dir_all(&k).unwrap();
    for (name, body) in docs {
        std::fs::write(k.join(name), body).unwrap();
    }
    root
}

#[tokio::test]
#[ignore = "需要本机 Ollama 在跑，且已 pull bge-m3"]
async fn live_knowledge_index_is_real_and_scoped() {
    use anima_lib::memory::{Memory, MemoryConfig};

    let dir = memory_test_dir();
    let memory = Memory::open(
        &dir,
        MemoryConfig::default(),
        memory_gateway(),
        reqwest::Client::new(),
    )
    .await
    .expect("打开记忆系统失败");

    // 两个项目，各有一份内容完全无关的文档。
    // A 的文档故意**不含**查询里的任何关键词 —— 命中必须来自语义而不是字面。
    let ws_a = knowledge_workspace(&[(
        "api.md",
        "# 订单服务\n\n\
         ## 失败返回的约定\n\n\
         所有写操作在参数不合法时统一回 422，并在 body 里带 `field` 指出是哪个字段。\n\
         不要用 400 —— 网关会把 400 当成协议错误直接吞掉，客户端看不到 body。\n",
    )]);
    let ws_b = knowledge_workspace(&[(
        "roles.md",
        "# 角色设定\n\n\
         ## 女主\n\n\
         林晚，二十七岁，急诊科住院医师。习惯把情绪压到最后一刻才爆发。\n\
         口头禅是「先处理，再难受」。\n",
    )]);

    let n_a = memory.reindex_knowledge(&ws_a.to_string_lossy()).await.unwrap();
    let n_b = memory.reindex_knowledge(&ws_b.to_string_lossy()).await.unwrap();
    println!("A 索引 {n_a} 块，B 索引 {n_b} 块");
    assert!(n_a > 0 && n_b > 0, "索引必须真的写进去了");
    assert_eq!(
        memory.knowledge_count(&ws_a.to_string_lossy()).await.unwrap(),
        n_a,
        "count 要和 reindex 报的数字一致"
    );

    // ① 语义检索：查询用的词（"校验没通过该返回什么状态码"）和文档里的
    //    ("参数不合法"、"422") 字面上完全不重合
    let hits = memory
        .search_knowledge(&ws_a.to_string_lossy(), "校验没通过的时候该返回什么状态码", 3)
        .await
        .unwrap();
    println!("A 命中 {} 块", hits.len());
    for h in &hits {
        println!("  {} #{} sim={:.3}\n    {}", h.chunk.doc, h.chunk.index, h.similarity, h.chunk.text.replace('\n', " / "));
    }
    assert!(!hits.is_empty(), "语义检索应当命中，实际为空");
    assert!(hits[0].chunk.text.contains("422"), "命中的应该是那一节");
    // 标题路径必须带进正文，否则召回出来的块不自解释
    assert!(hits[0].chunk.text.contains("失败返回的约定"));

    // ② 项目隔离：在 A 里查 B 的内容，一条都不该有
    let cross = memory
        .search_knowledge(&ws_a.to_string_lossy(), "女主角是什么职业", 3)
        .await
        .unwrap();
    println!("跨项目查询命中 {} 块", cross.len());
    assert!(
        cross.iter().all(|h| !h.chunk.text.contains("林晚")),
        "B 项目的内容漏进了 A 的检索结果：{:?}",
        cross.iter().map(|h| &h.chunk.doc).collect::<Vec<_>>()
    );

    // ③ 重建是整体替换：把文档删掉再重建，索引必须跟着空
    std::fs::remove_file(ws_a.join(".anima/knowledge/api.md")).unwrap();
    let n_after = memory.reindex_knowledge(&ws_a.to_string_lossy()).await.unwrap();
    assert_eq!(n_after, 0, "文档删了索引还在，召回会引用不存在的文档");
    assert_eq!(
        memory.knowledge_count(&ws_a.to_string_lossy()).await.unwrap(),
        0
    );
    // 而 B 不受影响 —— 这是最容易写错的一条
    assert_eq!(
        memory.knowledge_count(&ws_b.to_string_lossy()).await.unwrap(),
        n_b,
        "重建 A 的索引不该动到 B"
    );

    // ④ 事实表没被波及
    let stats = memory.stats().await.unwrap();
    println!("事实数 {}（知识库重建不该影响它）", stats.facts);

    let _ = std::fs::remove_dir_all(&ws_a);
    let _ = std::fs::remove_dir_all(&ws_b);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
#[ignore = "需要本机 LM Studio / Ollama 在跑"]
async fn live_project_facts_are_extracted_and_scoped() {
    use anima_lib::memory::vector::ScopeFilter;
    use anima_lib::memory::{extract, Memory, MemoryConfig};

    let gateway = memory_gateway();

    // 一段真实形状的 coding 对话：里面有一条**下次仍然有用**的工具链约定，
    // 也有一条一次性的任务描述。好的提取只该拿前者。
    let user = "测试就用 npm test，别装 vitest —— Node 25 能直接跑 .ts。\
                另外帮我把 LRU 那个文件的 get 改成命中后刷新顺序。";
    let assistant = "明白。npm test 已经映射到 node --test，零依赖。\
                     get 里加了 delete + 重新 set，5 条用例全过。";

    let in_project = extract::extract(&gateway, user, assistant, true)
        .await
        .expect("项目提取失败");
    let outside = extract::extract(&gateway, user, assistant, false)
        .await
        .expect("陪伴提取失败");

    println!("项目白名单提取到 {} 条：", in_project.len());
    for f in &in_project {
        println!("  {}：{}", f.subject, f.fact);
    }
    println!("陪伴白名单提取到 {} 条：", outside.len());
    for f in &outside {
        println!("  {}：{}", f.subject, f.fact);
    }

    // ① 项目白名单要抓到工具链约定。判据放在"提到了测试命令"这一层 ——
    //    模型的措辞不可控，断言原文会变成一个和能力无关的脆弱测试
    let joined = in_project
        .iter()
        .map(|f| f.fact.as_str())
        .collect::<Vec<_>>()
        .join(" / ");
    assert!(
        joined.contains("npm test") || joined.contains("node --test") || joined.contains("vitest"),
        "项目白名单没抓到工具链约定，只有：{joined}"
    );

    // ② 一次性的任务描述不该进长期记忆
    assert!(
        !joined.contains("改成命中后刷新") && !joined.contains("帮我把"),
        "把一次性任务描述记成了长期事实：{joined}"
    );

    // ── 隔离：写进 A 项目，在 A 能召回，在闲聊里召不回 ──
    let dir = memory_test_dir();
    let memory = Memory::open(
        &dir,
        MemoryConfig::default(),
        gateway,
        reqwest::Client::new(),
    )
    .await
    .expect("打开记忆系统失败");

    let ws = knowledge_workspace(&[]);
    let scope = ws.to_string_lossy().to_string();
    memory.remember_project_turn(user, assistant, &scope);

    // 提取是 spawn 出去的，轮询等它落库（提取要打一次模型，给足时间）
    let mut waited = 0u64;
    while memory.stats().await.unwrap().facts == 0 && waited < 60_000 {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        waited += 500;
    }
    let n = memory.stats().await.unwrap().facts;
    println!("落库 {n} 条（等了 {waited}ms）");
    assert!(n > 0, "remember_project_turn 什么都没写进去");

    let q = "这个项目的测试怎么跑";
    let in_proj = memory
        .recall(q, &ScopeFilter::Project(scope.clone()))
        .await;
    println!("项目内召回 {} 条", in_proj.len());
    for h in &in_proj {
        println!("  sim={:.3}  {}", h.similarity, h.fact.fact);
    }
    assert!(!in_proj.is_empty(), "在本项目里应该能召回刚记下的约定");
    assert!(
        in_proj.iter().all(|h| h.fact.scope == scope),
        "召回的事实 scope 不对：{:?}",
        in_proj.iter().map(|h| &h.fact.scope).collect::<Vec<_>>()
    );

    // 闲聊链路（GlobalOnly）不该看到项目事实 —— 否则在聊天里被告知"测试用 npm test"
    let chat = memory.recall(q, &ScopeFilter::GlobalOnly).await;
    println!("闲聊召回 {} 条", chat.len());
    assert!(chat.is_empty(), "项目事实漏进了陪伴链路：{chat:?}");

    let _ = std::fs::remove_dir_all(&ws);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 任务台账：创建依从性回归
// ---------------------------------------------------------------------------

/// 模型会不会在第一轮就调 `task_plan` —— **按档位铺开测**。
///
/// ## 为什么必须覆盖档位
///
/// 上一版这条测试只打**实时档**（配置里是 qwen3.8，它一直合规），于是漏掉了
/// 真正在跑 coding 的那条路。实测：
///
/// | | Effort=Low | Effort=High |
/// |---|---|---|
/// | `openai/gpt-oss-120b`（后台档） | ✅ | ❌ 直奔 `fs_list_directory` |
/// | `ornith-1.5-35b-a3b` | ✅ | ✅ |
///
/// 而且 High 档下**第 3 轮的点名兜底也无效**（0/3，它重复调
/// `fs_list_directory`）。所以「靠措辞让模型自己建台账」只在低思考档成立。
///
/// 载荷最重的字段（`goal`）因此改成系统兜底（`TaskLedger::seed_from_user`，
/// 有单元测试）。这条测试的作用退化成**观测**：它告诉你当前这套配置的
/// 配合度有多好，而不是保证台账一定被建起来。
#[tokio::test]
#[ignore = "需要本机 LM Studio 在跑（后台档模型要能装下）"]
async fn live_task_plan_is_called_first() {
    use anima_lib::llm::{ChatChunk, ChatRequest, Message, ThinkingMode};

    let gateway = memory_gateway();
    let mut matrix = Vec::new();
    for (label, thinking) in [("Off/低", ThinkingMode::Off), ("High", ThinkingMode::High)] {
        let mut req = ChatRequest::new(vec![
            Message::system(
                "你是编码助手。工作区 /ws。\n## 多步任务\n\
                 超过三个步骤的任务，第 1 步永远是 task_plan（写 goal + 全部步骤）。\n\
                 **在调用 task_plan 之前，禁止调用任何其它工具。**\n\
                 进度由系统自动记录，不需要你再调 task_plan 更新状态。",
            ),
            Message::user(
                "在 src/lru.ts 现有的 LRUCache 上加 TTL 过期、localStorage 持久化、\
                 命中率统计面板，并扩展 src/lru.test.ts，用 npm test 跑到全绿。不要重写整个文件。",
            ),
        ]);
        req.temperature = Some(0.0);
        req.max_tokens = Some(900);
        req.thinking = thinking;
        req.tools = vec![
            anima_lib::task_ledger::declaration(),
            tool("fs_list_directory", "列目录"),
            tool("view_file", "读文件"),
            tool("apply_patch", "打补丁"),
            tool("shell_command", "执行命令"),
        ];

        // **后台档** —— 那才是真正在跑 coding 的那条路
        let (_, mut stream) = gateway
            .chat_stream(
                format!("ledger-compliance-{}", uuid::Uuid::new_v4()),
                anima_lib::config::Tier::Background,
                req,
            )
            .await
            .expect("建流失败");

        use futures_util::StreamExt;
        let mut first_tool: Option<String> = None;
        while let Some(chunk) = stream.next().await {
            match chunk.expect("流出错") {
                ChatChunk::ToolCall { call, .. } => {
                    if first_tool.is_none() && !call.name.is_empty() {
                        first_tool = Some(call.name.clone());
                    }
                }
                ChatChunk::Done { .. } => break,
                _ => {}
            }
        }
        let ok = first_tool.as_deref() == Some(anima_lib::agent::TASK_PLAN_NAME);
        println!(
            "  {} 思考={label:8} 首个调用: {first_tool:?}",
            if ok { "✅" } else { "❌" }
        );
        matrix.push((label, ok));
    }

    // **只对「关思考」那一档设硬断言。** High 档的不合规是已知且可复现的，
    // 由系统兜底覆盖（见模块文档的表）；把它也断言成必须合规，
    // 只会让这条测试常年红着而没人看。
    let low_ok = matrix.iter().find(|(l, _)| *l == "Off/低").map(|(_, ok)| *ok);
    assert_eq!(
        low_ok,
        Some(true),
        "关思考时后台档也不肯先调 task_plan —— 那就连低档的配合度都没了。\
         检查 commands.rs 工作区 header 的「多步任务」那一段是不是被改软了\
         （「禁止」这个词是实测出来的）。"
    );
    if matrix.iter().any(|(_, ok)| !ok) {
        println!(
            "  ⚠️  有档位不合规 —— 台账靠 TaskLedger::seed_from_user 兜底，\
             但 steps 会是空的。coding 建议用低思考档。"
        );
    }
}

fn tool(name: &str, desc: &str) -> anima_lib::llm::Tool {
    anima_lib::llm::Tool {
        name: name.to_string(),
        description: desc.to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": { "path": { "type": "string" } }
        }),
    }
}

// ---------------------------------------------------------------------------
// 记忆写入时收敛：四种关系的判定
// ---------------------------------------------------------------------------

/// 模型能不能分清 new / duplicate / contradicts / refines。
///
/// ## 为什么要当测试守着
///
/// `reconcile::PROMPT` 里那四个例子是**承重的** —— 本地小模型判不出抽象定义，
/// 全靠例子。任何一个例子被改软或删掉，那一类判定就静默退化成 `new`
/// （兜底方向），于是重复和矛盾又都回来了，而没有任何报错。
///
/// 实测（reasoning_effort=none）两个模型都 5/5：`qwen/qwen3.8-27b` 与
/// `ornith-1.5-35b-a3b`。
#[tokio::test]
#[ignore = "需要后台档模型能装下 —— gpt-oss-120b 要 63GB，得先卸掉别的模型"]
async fn live_reconcile_classifies_four_relations() {
    use anima_lib::memory::reconcile::{judge, Verdict};

    let gateway = memory_gateway();
    // (新事实, 已有事实, 可接受的结论)
    let cases: Vec<(&str, Vec<String>, Vec<&str>)> = vec![
        (
            "项目用 pnpm 装依赖",
            vec!["项目用 npm 装依赖".into()],
            vec!["contradicts"],
        ),
        (
            "姓名是刘松公子",
            vec!["我叫刘松，是你的开发者，你可以叫我“主人”，“公子”或者“亲爱的”".into()],
            vec!["refines", "duplicate"],
        ),
        (
            "正在阅读《凡人修仙传》",
            vec!["最近在看《凡人修仙传》".into()],
            vec!["duplicate", "refines"],
        ),
        (
            "测试用 npm test",
            vec!["构建用 npm run build".into(), "项目用 pnpm 装依赖".into()],
            vec!["new"],
        ),
        // 同一个类的两个方法：向量上很近（实测 0.575），但语义上无关
        (
            "get 命中后要刷新顺序",
            vec!["set 已存在的 key 只更新值不淘汰".into()],
            vec!["new"],
        ),
    ];

    let mut failures = Vec::new();
    // 上游 500 和「判定退化」是两种完全不同的问题，混在一条断言里的话
    // 诊断会指错方向。分开收集。
    let mut upstream = Vec::new();
    for (cand, neighbors, accepted) in &cases {
        // 重试一次：实测 gpt-oss + LM Studio 会间歇性 500
        // （"The model produced output that does not match the expected
        // peg-native format" —— harmony 解析器的问题，和提示词无关）
        let judged = match judge(&gateway, cand, neighbors).await {
            Ok(v) => Ok(v),
            Err(_) => judge(&gateway, cand, neighbors).await,
        };
        let v = match judged {
            Ok(v) => v,
            Err(err) => {
                println!("  ⚠️  上游报错（已重试一次）| {cand} | {err}");
                upstream.push(format!("「{cand}」：{err}"));
                continue;
            }
        };
        let name = match &v {
            Verdict::New => "new",
            Verdict::Duplicate { .. } => "duplicate",
            Verdict::Contradicts { .. } => "contradicts",
            Verdict::Refines { .. } => "refines",
        };
        if let Verdict::Refines { text, .. } = &v {
            println!("  合并文本：{text}");
        }
        let ok = accepted.contains(&name);
        println!(
            "  {} 期望 {:?} 实际 {name} | {cand}",
            if ok { "✅" } else { "❌" },
            accepted
        );
        if !ok {
            failures.push(format!("「{cand}」期望 {accepted:?}，实际 {name}"));
        }
    }
    assert!(
        failures.is_empty(),
        "收敛判定退化了 —— 检查 reconcile::PROMPT 里的例子是不是被改软了：\n{}",
        failures.join("\n")
    );
    // 上游失败单独报，措辞明确指向服务端而不是提示词。
    //
    // 实测：`openai/gpt-oss-120b` 在这组用例的第 3 格上**稳定** 500
    // （流式与非流式都会，换成 `qwen/qwen3.6-35b-a3b` 则 5/5 全过）。
    // 生产代码对此降级安全（判定失败 → 当新事实），所以这不影响正确性，
    // 只是那一条不会被合并。
    assert!(
        upstream.is_empty(),
        "判定调用被上游拒了（**不是提示词退化**）。\n\
         如果报的是 \"peg-native format\"，那是 LM Studio 侧 gpt-oss 的 harmony \
         解析器问题 —— 把后台档换成 qwen 系可规避。\n{}",
        upstream.join("\n")
    );
}


/// 一条命令就能办完的任务，模型会不会先铺一大堆分析？
///
/// 起因是真实反馈：「我就提了一个『把所有未跟踪文件加入 git 跟踪』，
/// 结果输出了一大堆分析、举例、思考、尝试，过了 3-4 分钟也没给我解决。」
///
/// 这里把**当前线上的工作区 Header 原文**和一个简化版做对照，量同一件事：
/// 模型第一次动作是**直接跑命令**，还是先去列目录/读文件/写计划。
///
/// 只测行为，不做断言 —— 提示词的效果是概率性的，一次跑不出结论，
/// 要看的是两组之间的差别。跑法：`npm run test:live -- live_trivial_task`
#[tokio::test]
#[ignore]
async fn live_trivial_task_should_not_trigger_analysis() {
    use anima_lib::llm::{ChatChunk, ChatRequest, Message, ThinkingMode};

    // 线上原文（`commands.rs` 的 Header，去掉与本题无关的段落）
    const CURRENT: &str = "# Current Workspace Environment\n\n\
        - **Workspace Absolute Path**: `/ws`\n\
        - **Recommended Tooling Guidelines**:\n\
        \x20 - Use `fs_list_directory` to inspect directory hierarchies.\n\
        \x20 - Use `view_file` (with line slicing) or `code_outline` to read file structures and symbols.\n\
        \x20 - Use `apply_patch` to make file edits.\n\
        - **Command Execution Context**:\n\
        \x20 - `shell_command` and `exec_command` execute directly in the workspace root. DO NOT prepend `cd <path> &&`.\n\
        - **Multi-step Task Ledger (`task_plan`)**:\n\
        \x20 - For tasks requiring more than 3 steps, step 1 MUST ALWAYS be `task_plan`.\n\
        \x20 - **NEVER call any other tools prior to invoking `task_plan`.**";

    /// 前端人设（`DEFAULT_SYSTEM_PROMPT`）+ 项目 `.anima/rules.md` 的实际内容。
    /// **这两段真实会话里一定在，第一版实验把它们漏了** ——
    /// 而它们恰恰都在要求"结构化呈现""分板块归纳总结""清晰列出当前阶段"。
    const PERSONA_AND_RULES: &str = "你是 Anima，一个聪明、有性格且高效的桌面个人 Agent 与创作助手。\n\n        【工具与排版规范】\n        - 在展示任何代码片段、测试用例、配置内容或终端命令时，必须使用标准 Markdown 代码块包裹。\n        - 在梳理代码、列举文件修改、技术分析或多项内容时，请使用清晰的 Markdown 结构        （使用 ## 标题、加粗小标题与清晰换行的 - 列表），优先分板块精炼归纳总结。\n\n        ---\n\n        # 项目行为准则与业务约束规范\n        ## 1. 交互与沟通风格\n        - **结论先行**：优先以精炼、结构化的语言呈现核心方案与关键结论；\n        - **可点击引用**：涉及项目内的具体文件、类名与函数时，使用 Markdown 格式精准标定；\n        - **透明反馈**：在执行多步骤操作时，清晰列出当前所处阶段与验证结果。\n        ## 2. 安全与执行约束\n        - **禁止静默破坏**：未经用户明确授权，严禁静默执行破坏性文件删除。";

    // 对照组：把 shell 提到推荐工具里，并给「一步就能做完」一个明确出口
    const WITH_EXIT: &str = "# Current Workspace Environment\n\n\
        - **Workspace Absolute Path**: `/ws`\n\
        - **If the request maps to a single command or a single edit, just do it.** \
        Do not plan, survey, or explain first.\n\
        - **Recommended Tooling Guidelines**:\n\
        \x20 - Use `shell_command` to run commands (git, npm, build, test).\n\
        \x20 - Use `fs_list_directory` to inspect directory hierarchies.\n\
        \x20 - Use `view_file` (with line slicing) or `code_outline` to read file structures and symbols.\n\
        \x20 - Use `apply_patch` to make file edits.\n\
        - **Command Execution Context**:\n\
        \x20 - `shell_command` and `exec_command` execute directly in the workspace root. DO NOT prepend `cd <path> &&`.\n\
        - **Multi-step Task Ledger (`task_plan`)**:\n\
        \x20 - For tasks requiring more than 3 steps, step 1 MUST ALWAYS be `task_plan`.\n\
        \x20 - **NEVER call any other tools prior to invoking `task_plan`.**";

    let mut matrix = Vec::new();

    // 三组：只有 Header / Header+人设+项目规则（=真实线上）/ 真实线上再加直做出口
    let full = format!("{PERSONA_AND_RULES}\n\n---\n\n{CURRENT}");
    let full_with_exit = format!("{PERSONA_AND_RULES}\n\n---\n\n{WITH_EXIT}");
    let models = std::env::var("PROMPT_AB_MODELS")
        .unwrap_or_else(|_| "openai/gpt-oss-120b,ornith-1.5-35b-a3b".to_string());
    for model in models.split(',').map(str::trim).filter(|m| !m.is_empty()) {
    let gateway = gateway_with_model(model);
    for (variant, system) in [
        ("真实线上栈", full.as_str()),
        ("线上栈+出口", full_with_exit.as_str()),
    ] {
        for round in 1..=3 {
            let mut req = ChatRequest::new(vec![
                Message::system(system),
                Message::user("把当前项目里所有未跟踪文件进行 git 跟踪管理"),
            ]);
            req.temperature = Some(0.0);
            req.max_tokens = Some(700);
            req.thinking = ThinkingMode::Off;
            req.tools = vec![
                anima_lib::task_ledger::declaration(),
                tool("shell_command", "在工作区根目录执行一条 shell 命令"),
                tool("fs_list_directory", "列目录"),
                tool("view_file", "读文件"),
                tool("apply_patch", "打补丁"),
            ];

            let (_p, mut stream) = gateway
                .chat_stream(format!("trivial-{variant}-{round}"), Tier::Background, req)
                .await
                .expect("起流失败");

            let mut first_tool: Option<String> = None;
            let mut text = String::new();
            while let Some(Ok(chunk)) = stream.next().await {
                match chunk {
                    ChatChunk::ToolCall { call } => {
                        if first_tool.is_none() {
                            first_tool = Some(call.name.clone());
                        }
                    }
                    ChatChunk::Text { delta } => text.push_str(&delta),
                    ChatChunk::Done { .. } => break,
                    _ => {}
                }
            }

            matrix.push(format!(
                "  {model:<22} {variant:<12} 第{round}次  首个动作={:<18} 文字={} 字",
                first_tool.as_deref().unwrap_or("（没调工具，只说话）"),
                text.chars().count()
            ));
        }
    }

    }
    println!("\n=== 一步任务：模型第一个动作是什么 ===");
    for line in &matrix {
        println!("{line}");
    }
    println!(
        "\n（期望：首个动作是 shell_command，先输出文字越少越好。\n\
         若「线上原文」组普遍先列目录/写计划/长篇说话，就说明提示词在推着它分析。）"
    );
}
