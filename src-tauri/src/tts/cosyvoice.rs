//! CosyVoice2 的 FastAPI 服务（`runtime/python/fastapi/server.py`）。
//!
//! 零样本克隆：`POST /inference_zero_shot`，multipart 表单，
//! 字段是 `tts_text` / `prompt_text` / `prompt_wav`。
//!
//! ## 两个和别的引擎不一样的地方
//!
//! 1. **返回的是没有头的裸 PCM**，不是 wav。所以要自己包 WAV 头 ——
//!    不包的话 rodio 解不出来（播放会直接失败），而且也算不出时长。
//!    采样率必须配对：包错了音调会整体偏移（配 24000 实际 22050 会偏高）。
//! 2. **参考音频要由 Anima 上传**，不是给服务端一个路径。
//!    所以路径是**本机**路径，与 GPT-SoVITS 那种"服务端自己读"相反 ——
//!    这点混了就会得到 404 或者根本没声音。

use async_trait::async_trait;

use super::{TextToSpeech, TtsConfig, TtsEngineConfig, VoiceInfo};
use crate::error::{Error, Result};

pub struct CosyVoiceTts {
    http: reqwest::Client,
    base_url: String,
    ref_audio_path: String,
    ref_text: String,
    sample_rate: u32,
}

impl CosyVoiceTts {
    pub fn new(http: reqwest::Client, config: TtsEngineConfig) -> Result<Self> {
        if config.base_url.trim().is_empty() {
            return Err(Error::config(format!(
                "TTS 引擎「{}」缺少 base_url",
                config.label
            )));
        }
        let ref_audio_path = config
            .ref_audio_path
            .clone()
            .filter(|p| !p.trim().is_empty())
            .ok_or_else(|| {
                Error::config(
                    "CosyVoice2 需要参考音频：音色由参考音频决定。\
                     请填**本机**可读的 wav 路径（由 Anima 上传给服务端，\
                     不是服务端自己去读的路径）。"
                        .to_string(),
                )
            })?;

        // 存在性在这里就查：等到第一句话要说时才发现文件不在，
        // 用户看到的只会是"没声音"
        if !std::path::Path::new(&ref_audio_path).is_file() {
            return Err(Error::config(format!(
                "参考音频不存在或不是文件：{ref_audio_path}"
            )));
        }

        Ok(Self {
            http,
            base_url: config.base_url.trim_end_matches('/').to_string(),
            ref_audio_path,
            ref_text: config.ref_text.unwrap_or_default(),
            // CosyVoice2 默认 24kHz；初代 CosyVoice 是 22050。
            // 配错只会音调偏移而不会报错，所以做成可配
            sample_rate: config.pcm_sample_rate.unwrap_or(24_000),
        })
    }
}

#[async_trait]
impl TextToSpeech for CosyVoiceTts {
    async fn synthesize(&self, text: &str, _config: &TtsConfig) -> Result<super::Speech> {
        let wav = tokio::fs::read(&self.ref_audio_path)
            .await
            .map_err(|e| Error::Io(format!("读取参考音频失败（{}）：{e}", self.ref_audio_path)))?;

        let form = reqwest::multipart::Form::new()
            .text("tts_text", text.to_string())
            .text("prompt_text", self.ref_text.clone())
            .part(
                "prompt_wav",
                reqwest::multipart::Part::bytes(wav)
                    .file_name("prompt.wav")
                    .mime_str("audio/wav")
                    .map_err(|e| Error::config(format!("构造 multipart 失败：{e}")))?,
            );

        let resp = self
            .http
            .post(format!("{}/inference_zero_shot", self.base_url))
            .multipart(form)
            .send()
            .await
            .map_err(|e| Error::Network(format!("CosyVoice2 请求失败：{e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Network(format!(
                "CosyVoice2 返回 {status}：{}",
                body.chars().take(300).collect::<String>()
            )));
        }

        let pcm = resp
            .bytes()
            .await
            .map_err(|e| Error::Network(format!("读取音频失败：{e}")))?
            .to_vec();

        if pcm.is_empty() {
            return Err(Error::Decode("CosyVoice2 返回了空音频".into()));
        }
        // 出错时它也可能 200 + JSON body
        if pcm.starts_with(b"{") {
            return Err(Error::Network(format!(
                "CosyVoice2 返回的不是音频：{}",
                String::from_utf8_lossy(&pcm)
                    .chars()
                    .take(300)
                    .collect::<String>()
            )));
        }

        // 已经是 wav 的话（不同版本行为不一致）就别再包一层
        let audio = if pcm.starts_with(b"RIFF") {
            pcm
        } else {
            super::wrap_pcm_s16le(&pcm, self.sample_rate, 1)
        };

        let words = match super::wav_duration_ms(&audio) {
            Some(ms) => super::estimate_word_boundaries(text, ms),
            None => Vec::new(),
        };
        Ok(super::Speech { audio, words })
    }

    async fn voices(&self) -> Result<Vec<VoiceInfo>> {
        // 零样本克隆没有音色列表：音色 = 参考音频
        Ok(Vec::new())
    }
}
