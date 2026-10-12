//! 把 TTS 音频分一路给口型 sidecar。
//!
//! ## 为什么是 Rust 直接送，而不是经前端转发
//!
//! **时间原点在 Rust 手里。** 播放器交出第一个采样时才知道「音频此刻开始响了」
//! —— 那是 [`crate::audio::playback::PlaybackSignal::Started`]。绕一圈到前端
//! 再回来会多出一段不确定的延迟，而口型差几十毫秒就看得出来。
//!
//! 另一半原因是**信息量**：wav2lip 吃的是梅尔频谱（16kHz / 80 维 /
//! 约 200ms 上下文），前端手里只有降维成几个标量的 viseme，那点信息喂不了它。
//! 所以送过去的是整句的原始音频字节，让 sidecar 自己解码。
//!
//! ## 绝对不能拖慢语音链路
//!
//! 这条路上任何一次等待都会推迟她开口，而打断（barge-in）的响应也走同一个
//! 状态机。所以每次调用都是 **spawn 出去、不等结果、带短超时**；
//! sidecar 没开着就静默跳过 —— 它不在的时候语音必须完全照常。

use std::time::Duration;

use tokio::runtime::Handle;

use crate::voice::VoiceState;

/// sidecar 的控制面。端口写死是因为它就是本机的一个子进程。
const BASE: &str = "http://127.0.0.1:8765";

/// 短到不会拖慢任何东西。**超时了就当这一句没有口型**，
/// 而不是让语音等它
const TIMEOUT: Duration = Duration::from_millis(800);

/// 口型 sidecar 的客户端。
///
/// **带着 runtime handle**，因为调用点分布在两种上下文里：
/// 句子音频是在 async 任务里送的，而「播放开始」那一下发生在音频桥接的
/// **std::thread** 里 —— 那儿没有 tokio 上下文，`tokio::spawn` 会直接 panic。
/// 在异步侧构造一次、克隆进线程，两边就都能用。
#[derive(Clone)]
pub struct LipsyncStream {
    http: reqwest::Client,
    rt: Handle,
}

impl LipsyncStream {
    /// 必须在 tokio 上下文里构造。
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            rt: Handle::current(),
        }
    }

    /// 一整句的音频。**要赶在播放开始之前送到** —— 送晚了 sidecar 会先记下
    /// start、等音频到了再接上，口型不会丢，但会少掉开头那一点点。
    pub fn push_sentence(&self, id: u64, audio: Vec<u8>) {
        let client = self.http.clone();
        self.rt.spawn(async move {
            let _ = client
                .post(format!("{BASE}/lipsync/sentence?id={id}"))
                .timeout(TIMEOUT)
                .header("content-type", "application/octet-stream")
                .body(audio)
                .send()
                .await;
        });
    }

    /// 这一句**此刻**开始响。对齐就靠这一下。
    pub fn started(&self, id: u64) {
        self.fire(format!("{BASE}/lipsync/started?id={id}"));
    }

    /// 被打断了：立刻停口型，别把剩下的帧播完。
    pub fn stop(&self) {
        self.fire(format!("{BASE}/lipsync/stop"));
    }

    /// 体态：说话 / 倾听 / 待机。口型之外的那一层。
    pub fn set_state(&self, state: VoiceState) {
        self.fire(format!("{BASE}/lipsync/clip?name={}", clip_for(state)));
    }

    fn fire(&self, url: String) {
        let client = self.http.clone();
        self.rt.spawn(async move {
            let _ = client.post(url).timeout(TIMEOUT).send().await;
        });
    }
}

/// 状态 → 体态片段。**逐个列出，不用 `_ =>` 兜底** ——
/// 新增状态时漏掉的表现是体态卡在待机，不报任何错。
pub fn clip_for(state: VoiceState) -> &'static str {
    match state {
        VoiceState::Speaking => "speaking",
        VoiceState::Listening | VoiceState::Transcribing => "listening",
        VoiceState::Thinking | VoiceState::Idle => "idle",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 体态映射覆盖每一个状态() {
        for (state, expect) in [
            (VoiceState::Speaking, "speaking"),
            (VoiceState::Listening, "listening"),
            (VoiceState::Transcribing, "listening"),
            (VoiceState::Thinking, "idle"),
            (VoiceState::Idle, "idle"),
        ] {
            assert_eq!(clip_for(state), expect, "{state:?} 的体态映射错了");
        }
    }
}
