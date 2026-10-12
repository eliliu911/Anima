//! 口型 sidecar 的联调测试。**默认不跑**（`#[ignore]`），需要本机 sidecar 在跑：
//!
//! ```bash
//! (cd tools/digital-human && .venv/bin/python server.py &)
//! curl -sX POST 'http://127.0.0.1:8765/load_avatar' -H 'Content-Type: application/json' \
//!      -d '{"model_id":"neural/xiaoya_idle.mp4"}'
//! cargo test --test live_lipsync -- --ignored --nocapture
//! ```
//!
//! 存在的意义：Rust 和 Python 之间是**跨语言的 HTTP 契约**，两边各自的单元测试
//! 谁也证明不了对面收到了什么。而这条链路失败时的表现是「嘴不动」——
//! 不报错、不掉线、日志里什么都没有。这里用真实的客户端打真实的服务，
//! 再回头问服务「你现在在做口型吗」，把那个静默的失败变成一条会红的测试。

use std::time::Duration;

use anima_lib::avatar::lipsync_stream::LipsyncStream;
use anima_lib::voice::VoiceState;

const BASE: &str = "http://127.0.0.1:8765";

async fn health(http: &reqwest::Client) -> Option<serde_json::Value> {
    http.get(format!("{BASE}/health"))
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()
}

/// 找一段真实音频。没有就跳过 —— **不要用合成正弦波**，
/// 那东西的梅尔频谱和人声完全不同，测不出「参数对不对」。
///
/// 从音色库里随便挑一条参考音频。原先写的是那个一键部署包的路径，
/// 而那是个下载目录 —— 用户一清理测试就静默跳过了。
fn sample_audio() -> Option<Vec<u8>> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .join("voice-library");
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).ok()?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "wav") {
                return std::fs::read(&path).ok();
            }
        }
    }
    None
}

#[tokio::test]
#[ignore = "需要本机 sidecar 在跑"]
async fn 整句音频送过去之后口型会起来() {
    let http = reqwest::Client::new();
    let Some(info) = health(&http).await else {
        eprintln!("跳过：sidecar 没在跑");
        return;
    };
    assert_eq!(
        info["lipsync"], true,
        "sidecar 在跑但没启用口型引擎 —— 检查 models/wav2lip256.pth 和素材是不是一套三段：{info}"
    );

    let Some(audio) = sample_audio() else {
        eprintln!("跳过：找不到参考音频");
        return;
    };

    let lipsync = LipsyncStream::new(http.clone());
    let id = 424242;

    lipsync.push_sentence(id, audio);
    // push 是 spawn 出去的（绝不能拖慢语音链路），所以要等它真的到
    tokio::time::sleep(Duration::from_millis(1500)).await;

    lipsync.started(id);
    tokio::time::sleep(Duration::from_millis(300)).await;

    let speaking = http
        .get(format!("{BASE}/lipsync/state"))
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .expect("查询口型状态失败")
        .json::<serde_json::Value>()
        .await
        .expect("口型状态不是 JSON");
    assert_eq!(
        speaking["speaking"], true,
        "送了音频又报了 start，口型却没起来：{speaking}"
    );

    // 打断要立刻停。**不停的话她嘴还在动而声音已经没了**
    lipsync.stop();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let after = http
        .get(format!("{BASE}/lipsync/state"))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(after["speaking"], false, "stop 之后口型没停：{after}");
}

#[tokio::test]
#[ignore = "需要本机 sidecar 在跑"]
async fn 体态会跟着状态切() {
    let http = reqwest::Client::new();
    if health(&http).await.is_none() {
        eprintln!("跳过：sidecar 没在跑");
        return;
    }
    let state = |c: reqwest::Client| async move {
        c.get(format!("{BASE}/lipsync/state"))
            .send()
            .await
            .unwrap()
            .json::<serde_json::Value>()
            .await
            .unwrap()
    };

    // **单段素材切不了体态，那不是故障。** 只有一段 idle 时
    // set_clip("speaking") 本来就该是无效操作
    let available = state(http.clone()).await;
    let clips: Vec<String> = serde_json::from_value(available["clips"].clone()).unwrap_or_default();
    if clips.len() < 2 {
        eprintln!("跳过：当前素材只有 {clips:?}，没有多段体态可切");
        return;
    }

    let lipsync = LipsyncStream::new(http.clone());
    for (voice_state, expect) in [
        (VoiceState::Speaking, "speaking"),
        (VoiceState::Listening, "listening"),
        (VoiceState::Idle, "idle"),
    ] {
        if !clips.iter().any(|c| c == expect) {
            continue;
        }
        lipsync.set_state(voice_state);
        tokio::time::sleep(Duration::from_millis(250)).await;
        let s = state(http.clone()).await;
        assert_eq!(s["clip"], expect, "{voice_state:?} 应该切到 {expect}：{s}");
    }
}
