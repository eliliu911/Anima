//! 打一遍当前选中的 TTS 引擎，看它给不给对口型能用的数据。
//!
//! 「嘴不动」的排查里，这是第一个该问的问题：引擎有没有返回词边界。
//! 没有的话后面所有环节都不用查了 —— 前端根本收不到时间线。
//!
//! ```bash
//! scripts/with-env.sh cargo run --example tts-probe -- "你好，我是 Anima。"
//! ```
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let text = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "你好，我是 Anima。".to_string());

    let config_dir = dirs::config_dir()
        .ok_or("找不到配置目录")?
        .join("com.anima.desktop");
    let app = anima_lib::config::AppConfig::load(&config_dir.join("config.json"))?;
    let selected = app
        .tts_engines
        .iter()
        .find(|e| e.id == app.tts_engine)
        .ok_or_else(|| format!("配置里找不到 TTS 引擎「{}」", app.tts_engine))?;
    let mut tts_config = app.voice.tts.clone();
    // 允许用环境变量指定音色，方便验单个音色能不能用
    if let Ok(v) = std::env::var("ANIMA_PROBE_VOICE") {
        if !v.is_empty() {
            tts_config.voice = v;
        }
    }
    println!(
        "引擎：{} ({:?})   音色：{}",
        selected.id, selected.kind, tts_config.voice
    );

    let engine = anima_lib::tts::engine(
        selected,
        reqwest::Client::new(),
        &anima_lib::tts::voices::voices_dir(&config_dir),
    )?;

    let t0 = std::time::Instant::now();
    let speech = engine.synthesize(&text, &tts_config).await?;
    println!(
        "合成耗时 {:?}，音频 {} 字节",
        t0.elapsed(),
        speech.audio.len()
    );

    match anima_lib::tts::wav_duration_ms(&speech.audio) {
        Some(ms) => println!("时长 {ms}ms（从 WAV 头算出）"),
        None => println!("⚠️  解不出时长 —— 词边界会是空的，嘴就不会动"),
    }

    println!("词边界 {} 个", speech.words.len());
    for w in speech.words.iter().take(8) {
        println!(
            "   {:>6}ms +{:>4}ms  {}",
            w.offset_ms, w.duration_ms, w.text
        );
    }
    if speech.words.is_empty() {
        println!("⚠️  没有词边界 —— 前端收不到 words 事件，嘴不会动");
        return Ok(());
    }

    let keys = anima_lib::avatar::viseme::schedule(&speech.words);
    println!("嘴型关键帧 {} 个", keys.len());
    for k in keys.iter().take(8) {
        let name = serde_json::to_string(&k.viseme).unwrap_or_default();
        println!("   {:>6}ms  {}", k.at_ms, name.trim_matches('"'));
    }
    Ok(())
}
