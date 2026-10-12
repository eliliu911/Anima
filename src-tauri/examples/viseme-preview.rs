//! 打印一句话的嘴型时间线。调口型时用它比盯着模型看快得多 ——
//! 「嘴型不对」在画面上分不清是映射错了还是时间错了，这里一眼就能看出来。
//!
//! ```bash
//! cargo run --example viseme-preview -- "你好，我是 Anima。"
//! ```
fn main() {
    let text = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "你好，我是 Anima。".to_string());
    // 按每字 250ms 估时长，和真实语速接近
    let total = (text.chars().count() as u64) * 250;
    let words = anima_lib::tts::estimate_word_boundaries(&text, total);
    let keys = anima_lib::avatar::viseme::schedule(&words);
    println!("「{text}」 {total}ms → {} 个关键帧\n", keys.len());
    for k in &keys {
        let bar = "█".repeat((k.weight * 12.0) as usize);
        // 打 serde 名而不是 Debug 名。Debug 会打 `Pp`/`Aa`，而线上传给前端、
        // 也是模型里 morph target 用的是 `PP`/`aa` —— 两套名字对不上的话，
        // 照着这里的输出去查模型只会查不到
        let name = serde_json::to_string(&k.viseme).unwrap_or_default();
        println!("  {:>6}ms  {:<6} {bar}", k.at_ms, name.trim_matches('"'));
    }
}
