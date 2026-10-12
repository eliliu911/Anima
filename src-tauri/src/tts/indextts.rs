//! IndexTTS2 的 HTTP 服务。
//!
//! ## 为什么这个适配器把端点路径做成可配
//!
//! IndexTTS 的部署形态比 Edge / OpenAI 兼容那些散：官方仓库、各种 fork、
//! 以及社区打包的一键启动器，端点路径和字段名并不统一（`/tts`、`/api/tts`、
//! `/v1/tts` 都见过）。写死一个就等于只支持其中一种。
//!
//! 所以这里只固定"POST 一个 JSON、拿回音频字节"这个骨架，路径可配。
//! 如果你的部署字段名对不上，报错会把服务端的原文带出来 —— 照着改配置即可。
//!
//! **注意：这个适配器是照文档形状写的，还没有对着真实服务联调过。**
//! 第一次连很可能要按实际返回调整，见 DECISIONS.md 对应条目。

use async_trait::async_trait;
use serde::Serialize;

use super::{TextToSpeech, TtsConfig, TtsEngineConfig, VoiceInfo};
use crate::error::{Error, Result};

pub struct IndexTts {
    http: reqwest::Client,
    endpoint: String,
    ref_audio_path: Option<String>,
    ref_text: String,
    sample_rate: u32,
}

#[derive(Serialize)]
struct TtsRequest<'a> {
    text: &'a str,
    /// 参考音频路径（服务端可读）。IndexTTS2 用它做音色克隆
    #[serde(skip_serializing_if = "Option::is_none")]
    speaker_audio: Option<&'a str>,
    #[serde(skip_serializing_if = "str::is_empty")]
    speaker_text: &'a str,
    speed: f32,
}

impl IndexTts {
    pub fn new(http: reqwest::Client, config: TtsEngineConfig) -> Result<Self> {
        if config.base_url.trim().is_empty() {
            return Err(Error::config(format!(
                "TTS 引擎「{}」缺少 base_url",
                config.label
            )));
        }
        let base = config.base_url.trim_end_matches('/');
        // 端点可配，默认最常见的那个
        let path = config
            .endpoint_path
            .clone()
            .filter(|p| !p.trim().is_empty())
            .unwrap_or_else(|| "/tts".to_string());
        let path = if path.starts_with('/') {
            path
        } else {
            format!("/{path}")
        };

        Ok(Self {
            http,
            endpoint: format!("{base}{path}"),
            ref_audio_path: config.ref_audio_path.filter(|p| !p.trim().is_empty()),
            ref_text: config.ref_text.unwrap_or_default(),
            sample_rate: config.pcm_sample_rate.unwrap_or(24_000),
        })
    }
}

#[async_trait]
impl TextToSpeech for IndexTts {
    async fn synthesize(&self, text: &str, config: &TtsConfig) -> Result<super::Speech> {
        let resp = self
            .http
            .post(&self.endpoint)
            .json(&TtsRequest {
                text,
                speaker_audio: self.ref_audio_path.as_deref(),
                speaker_text: &self.ref_text,
                speed: super::openai_speed_from_rate(&config.rate),
            })
            .send()
            .await
            .map_err(|e| Error::Network(format!("IndexTTS 请求失败：{e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            // 字段名对不上时服务端会说清缺什么，所以原文要带出来
            return Err(Error::Network(format!(
                "IndexTTS 返回 {status}（端点 {}）：{}",
                self.endpoint,
                body.chars().take(400).collect::<String>()
            )));
        }

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| Error::Network(format!("读取音频失败：{e}")))?
            .to_vec();

        if bytes.is_empty() {
            return Err(Error::Decode("IndexTTS 返回了空音频".into()));
        }
        if bytes.starts_with(b"{") {
            return Err(Error::Network(format!(
                "IndexTTS 返回的是 JSON 而不是音频（端点 {} 可能不对）：{}",
                self.endpoint,
                String::from_utf8_lossy(&bytes)
                    .chars()
                    .take(400)
                    .collect::<String>()
            )));
        }

        // wav / 裸 PCM 都见过，按实际内容判断
        let audio = if bytes.starts_with(b"RIFF") {
            bytes
        } else {
            super::wrap_pcm_s16le(&bytes, self.sample_rate, 1)
        };

        let words = match super::wav_duration_ms(&audio) {
            Some(ms) => super::estimate_word_boundaries(text, ms),
            None => Vec::new(),
        };
        Ok(super::Speech { audio, words })
    }

    async fn voices(&self) -> Result<Vec<VoiceInfo>> {
        Ok(Vec::new())
    }
}
