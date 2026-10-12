//! OpenAI 兼容的 `/v1/audio/speech`。
//!
//! 选这个形状是因为覆盖面最广 —— Kokoro-FastAPI、Fish Speech、以及不少本地
//! TTS 服务都实现了它，一个适配器能接一片生态，不用为每家写一个。
//!
//! **它不返回词边界时间戳**（协议里就没有这个字段），所以时间轴靠
//! [`super::estimate_word_boundaries`] 估。见那个函数的说明。

use async_trait::async_trait;
use serde::Serialize;

use super::{TextToSpeech, TtsConfig, TtsEngineConfig, VoiceInfo};
use crate::error::{Error, Result};

pub struct OpenAiTts {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
}

#[derive(Serialize)]
struct SpeechRequest<'a> {
    model: &'a str,
    input: &'a str,
    voice: &'a str,
    /// 要 wav 而不是 mp3：wav 能从头部精确算出时长，
    /// 而估算词边界需要总时长。mp3 得逐帧扫才知道，不值得。
    response_format: &'a str,
    speed: f32,
}

impl OpenAiTts {
    pub fn new(http: reqwest::Client, config: TtsEngineConfig) -> Result<Self> {
        if config.base_url.trim().is_empty() {
            return Err(Error::config(format!(
                "TTS 引擎「{}」缺少 base_url",
                config.label
            )));
        }
        let api_key = match &config.api_key_secret {
            Some(id) => crate::config::secrets::get_secret(id)?,
            None => None,
        };
        Ok(Self {
            http,
            base_url: config.base_url.trim_end_matches('/').to_string(),
            api_key,
        })
    }

    fn request(&self, url: String) -> reqwest::RequestBuilder {
        let req = self.http.post(url);
        match &self.api_key {
            Some(key) => req.bearer_auth(key),
            None => req,
        }
    }
}

#[async_trait]
impl TextToSpeech for OpenAiTts {
    async fn synthesize(&self, text: &str, config: &TtsConfig) -> Result<super::Speech> {
        // rate 在 Edge 那边是 "+20%" 这种字符串，这里要换算成倍数
        let speed = super::openai_speed_from_rate(&config.rate);

        let resp = self
            .request(format!("{}/v1/audio/speech", self.base_url))
            .json(&SpeechRequest {
                // 多数本地实现忽略 model 字段，但协议要求必填
                model: "tts-1",
                input: text,
                voice: &config.voice,
                response_format: "wav",
                speed,
            })
            .send()
            .await
            .map_err(|e| Error::Network(format!("TTS 请求失败：{e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Network(format!(
                "TTS 服务返回 {status}：{}",
                body.chars().take(300).collect::<String>()
            )));
        }

        let audio = resp
            .bytes()
            .await
            .map_err(|e| Error::Network(format!("读取音频失败：{e}")))?
            .to_vec();

        if audio.is_empty() {
            return Err(Error::Decode("TTS 服务返回了空音频".into()));
        }

        // 没有真实时间戳，用总时长摊出来。拿不到时长就不给词边界 ——
        // 与其发一组瞎猜的时间轴让嘴型乱动，不如让上层知道没有
        let words = match super::wav_duration_ms(&audio) {
            Some(ms) => super::estimate_word_boundaries(text, ms),
            None => {
                tracing::debug!("TTS 返回的不是可解析的 WAV，本句没有词边界");
                Vec::new()
            }
        };

        Ok(super::Speech { audio, words })
    }

    async fn voices(&self) -> Result<Vec<VoiceInfo>> {
        // 这个协议没有标准的音色列表接口。Kokoro-FastAPI 等实现提供了
        // /v1/audio/voices，有就用，没有就返回空让界面提示手填 ——
        // 不能编一份假列表出来。
        let url = format!("{}/v1/audio/voices", self.base_url);
        // **不要静默返回空数组。** 原先无论是"服务没起来"还是"这个实现没有
        // 音色接口"，都一律返回空 —— 界面上的空下拉框和"真的没有音色"
        // 分不出来，用户只能看到一个点不开的框。
        let resp = match self.http.get(&url).send().await {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                let status = r.status();
                // 404 是合理的：不是每个 OpenAI 兼容实现都提供这个接口
                if status == reqwest::StatusCode::NOT_FOUND {
                    tracing::info!(%url, "该服务没有音色列表接口，需手工填音色名");
                    return Ok(Vec::new());
                }
                return Err(Error::Network(format!(
                    "读取音色列表失败（{url}）：HTTP {status}"
                )));
            }
            Err(err) => {
                return Err(Error::Network(format!(
                    "连不上 TTS 服务（{url}）：{err}。请确认服务已启动。"
                )))
            }
        };

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Decode(format!("音色列表解析失败：{e}")))?;

        // 见过两种形状：{"voices": ["af_bella", ...]} 和顶层直接是数组
        let list = body
            .get("voices")
            .or(Some(&body))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        Ok(list
            .into_iter()
            .filter_map(|v| {
                let name = match &v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.get("name")?.as_str()?.to_string(),
                };
                Some(VoiceInfo {
                    name: name.clone(),
                    short_name: name,
                    gender: String::new(),
                    locale: String::new(),
                })
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::super::openai_speed_from_rate as parse_rate_percent;

    #[test]
    fn rate_string_converts_to_a_speed_multiplier() {
        assert_eq!(parse_rate_percent("+0%"), 1.0);
        assert_eq!(parse_rate_percent("+20%"), 1.2);
        assert_eq!(parse_rate_percent("-10%"), 0.9);
    }

    #[test]
    fn unparseable_rate_falls_back_to_normal_speed() {
        // 格式不对不该让整句话说不出来
        assert_eq!(parse_rate_percent("很快"), 1.0);
        assert_eq!(parse_rate_percent(""), 1.0);
    }

    #[test]
    fn speed_is_clamped_to_the_protocol_range() {
        // OpenAI 的合法区间是 0.25~4.0，超出会被服务端拒绝整个请求
        assert_eq!(parse_rate_percent("+900%"), 4.0);
        assert_eq!(parse_rate_percent("-99%"), 0.25);
    }
}
