//! 语音合成（TTS）。
//!
//! 上层只依赖 [`TextToSpeech`] trait —— 阶段二先用 [`edge`]（微软 Edge 的免费
//! 在线接口，中文音色自然、无需下模型），将来换本地 CosyVoice / GPT-SoVITS
//! 不用动语音管线。
//!
//! **按句合成**而不是整段合成：整段合成的话用户要等模型把话说完才听到声音；
//! 按句切分能让「第一句合成」与「后续生成」并行，首字延迟只取决于第一句。
//! 切句由 [`crate::audio::playback::SentenceSplitter`] 负责。

pub mod cosyvoice;
pub mod dict;
pub mod edge;
pub mod emotion_voice;
pub mod indextts;
pub mod openai;
pub mod preflight;
pub mod sovits;
pub mod voices;

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;

/// 可选的 TTS 引擎。
///
/// 与 LLM 那边的 `ProviderKind` 同一个套路：上层只认 [`TextToSpeech`]，
/// 换引擎不动语音管线。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
// 这里的字符串是**前端契约**：TtsEngineSettings 按它分支决定显示哪些字段。
// 两个变体加了显式 rename —— serde 的 snake_case 会拆成 open_ai_compatible /
// gpt_so_vits，和引擎 id 的写法不一致，手改 config.json 时容易搞混。
// 实际线格式钉在 wire_format 测试里。
#[serde(rename_all = "snake_case")]
pub enum TtsEngineKind {
    /// 微软 Edge 朗读。云端、免费、中文自然，**唯一会返回词边界时间戳的**。
    /// 音色是固定目录（中文只有 14 个），换不了。
    Edge,
    /// OpenAI 兼容的 `/v1/audio/speech`。覆盖面最广：
    /// Kokoro-FastAPI、Fish Speech、以及不少本地服务都实现了这个形状。
    // alias 是为了兼容改名之前已经写进 config.json 的值。
    // 少了它，老配置会整份解析失败 —— 而 serde 的报错只指向一个字段，
    // 后果却是全部设置退回默认值
    #[serde(rename = "openai_compatible", alias = "open_ai_compatible")]
    OpenAiCompatible,
    /// GPT-SoVITS 的 api_v2。二次元音色包基本都是这个生态，
    /// 靠参考音频做零样本克隆。
    #[serde(rename = "gpt_sovits", alias = "gpt_so_vits")]
    GptSoVits,
    /// CosyVoice2 的 FastAPI。零样本克隆，流式支持最成熟。
    /// 返回裸 PCM，要自己包 WAV 头。
    CosyVoice2,
    /// IndexTTS2。情感控制强；部署形态较散，所以端点路径可配。
    ///
    /// **在 Apple Silicon 上不可用**：实测 M4 Max + MPS 每句 13.7 秒
    /// （RTF 4.0），fp16 更差（77 秒）。它的官方加速手段 deepspeed 与
    /// flash-attn 全是 CUDA 专用。留着适配器是为了有 N 卡时能用。
    IndexTts2,
    /// F5-TTS。零样本克隆，**非自回归**（flow matching + DiT）——
    /// 这是它在 Mac 上能用的根本原因，实测 2.8× 实时。
    ///
    /// 线格式就是 OpenAI 兼容那一套，所以复用同一个适配器；单独一个变体
    /// 只为了在设置面板里有自己的说明和默认端口。
    /// 服务见 `tools/f5-tts/server.py`。
    F5Tts,
}

/// 一个引擎的连接配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsEngineConfig {
    pub id: String,
    pub label: String,
    pub kind: TtsEngineKind,
    /// 本地服务地址。Edge 不用填（走微软固定端点）
    #[serde(default)]
    pub base_url: String,
    /// **这个引擎自己**记住的音色。
    ///
    /// 音色名的命名空间是每家一套：Edge 是 `zh-CN-XiaoxiaoNeural`，
    /// Kokoro 是 `zf_xiaobei`，GPT-SoVITS 是共享音色库里的路径 `yaemiko/d`。
    /// 早先只有一个全局 `TtsConfig::voice`，换引擎时那个名字被原样带过去 ——
    /// **而且不报错的失败方式各不相同**：Kokoro 会去 HuggingFace 找
    /// `voices/zh-TW-HsiaoYuNeural.pt` 然后 404，SoVITS 会说"需要参考音频"，
    /// Edge 拿到 `yaemiko/d` 直接合成失败。用户看到的都是"没声音"。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<String>,
    /// 需要鉴权时的凭据 id（走 secrets，不落明文）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_secret: Option<String>,
    /// GPT-SoVITS 用：参考音频路径与其文本。
    /// 零样本克隆的音色**由参考音频决定**，不是 voice 名字决定的。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ref_audio_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ref_text: Option<String>,
    /// 服务返回裸 PCM 时用的采样率。
    ///
    /// 配错**不会报错**，只会整体音调偏移（当 24000 用实际 22050 会偏高），
    /// 所以必须能改。CosyVoice2 是 24000，初代 CosyVoice 是 22050。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pcm_sample_rate: Option<u32>,
    /// 端点路径。IndexTTS 各 fork 不统一（/tts、/api/tts、/v1/tts 都见过），
    /// 写死一个就等于只支持其中一种。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint_path: Option<String>,
}

impl TtsEngineConfig {
    /// 默认引擎列表。新增引擎时加在这里，`load` 会自动补进已有 config.json。
    pub fn defaults() -> Vec<Self> {
        vec![
            Self {
                voice: None,
                id: "edge".into(),
                label: "Edge 朗读（云端）".into(),
                kind: TtsEngineKind::Edge,
                base_url: String::new(),
                api_key_secret: None,
                ref_audio_path: None,
                ref_text: None,
                pcm_sample_rate: None,
                endpoint_path: None,
            },
            Self {
                voice: None,
                id: "openai_compatible".into(),
                label: "OpenAI 兼容（本地）".into(),
                kind: TtsEngineKind::OpenAiCompatible,
                // Kokoro-FastAPI 的默认端口
                base_url: "http://127.0.0.1:8880".into(),
                api_key_secret: None,
                ref_audio_path: None,
                ref_text: None,
                pcm_sample_rate: None,
                endpoint_path: None,
            },
            Self {
                voice: None,
                id: "f5_tts".into(),
                label: "F5-TTS（本地克隆）".into(),
                kind: TtsEngineKind::F5Tts,
                // tools/f5-tts/server.py 的端口。刻意不用 8880 ——
                // 那是 Kokoro 的，两个服务要能同时开着
                base_url: "http://127.0.0.1:8881".into(),
                api_key_secret: None,
                ref_audio_path: None,
                ref_text: None,
                pcm_sample_rate: None,
                endpoint_path: None,
            },
            Self {
                voice: None,
                id: "gpt_sovits".into(),
                label: "GPT-SoVITS（本地克隆）".into(),
                kind: TtsEngineKind::GptSoVits,
                // api_v2.py 的默认端口
                base_url: "http://127.0.0.1:9880".into(),
                api_key_secret: None,
                ref_audio_path: None,
                ref_text: None,
                pcm_sample_rate: None,
                endpoint_path: None,
            },
            Self {
                voice: None,
                id: "cosyvoice2".into(),
                label: "CosyVoice2（本地克隆）".into(),
                kind: TtsEngineKind::CosyVoice2,
                // runtime/python/fastapi/server.py 的默认端口
                base_url: "http://127.0.0.1:50000".into(),
                api_key_secret: None,
                ref_audio_path: None,
                ref_text: None,
                pcm_sample_rate: Some(24_000),
                endpoint_path: None,
            },
            Self {
                voice: None,
                id: "indextts2".into(),
                label: "IndexTTS2（本地克隆）".into(),
                kind: TtsEngineKind::IndexTts2,
                base_url: "http://127.0.0.1:8000".into(),
                api_key_secret: None,
                ref_audio_path: None,
                ref_text: None,
                pcm_sample_rate: Some(24_000),
                endpoint_path: Some("/tts".into()),
            },
        ]
    }
}

/// 按配置造一个引擎实例。
/// 造一个引擎实例。
///
/// `voices_root` 是**已经解析好的**共享音色库根目录（用
/// [`voices::voices_dir`] 从应用数据目录算出来）。
///
/// **刻意收"已解析的路径"而不是 config_dir。** 解析要看 `ANIMA_VOICES`
/// 环境变量和当前目录往上找 `voice-library/`，把这件事放在构造函数里的后果是
/// 行为随环境漂移、测试也拿不到确定结果 —— 实测撞到过：测试传了临时目录，
/// 实际却命中了项目里真实的音色库，于是"缺参考音频要报错"那条断言失效。
/// 解析集中在调用边缘做一次，往下传的就是确定的路径。
/// 这个引擎认不认音色库（`voice-library/`）里的名字。
///
/// **只有 GPT-SoVITS 认** —— 看 [`engine`] 就知道：只有它收 `voices_root`。
/// 别的引擎的 `voice` 是它自己那套名字（Edge 是 `zh-CN-XiaoxiaoNeural`，
/// OpenAI 兼容是 `af_sky` 之类）。
///
/// 判断放在这里而不是散在调用方：角色的 `emotion_voices` 是音色库里的路径，
/// 套到不认识它的引擎上不会报错，只会**默默用错嗓子或者合成失败**。
/// 这个引擎该用哪个音色：它自己记着的那个，没有就用这一类的默认值。
///
/// **不要跨引擎沿用同一个音色名** —— 见 [`TtsEngineConfig::voice`] 的注释。
pub fn effective_voice(engine: &TtsEngineConfig) -> String {
    if let Some(v) = engine.voice.as_deref() {
        let v = v.trim();
        if !v.is_empty() {
            return v.to_string();
        }
    }
    match engine.kind {
        // Edge 必须给一个合法音色名，给空的话它不知道用谁
        TtsEngineKind::Edge => "zh-CN-XiaoxiaoNeural".to_string(),
        // 其余引擎给空串 = 用它自己的默认。零样本克隆的几家更是靠
        // 参考音频决定音色，硬塞一个名字反而是错的
        _ => String::new(),
    }
}

pub fn uses_voice_library(kind: TtsEngineKind) -> bool {
    matches!(kind, TtsEngineKind::GptSoVits)
}

pub fn engine(
    config: &TtsEngineConfig,
    http: reqwest::Client,
    voices_root: &std::path::Path,
) -> Result<Arc<dyn TextToSpeech>> {
    Ok(match config.kind {
        TtsEngineKind::Edge => Arc::new(edge::EdgeTts::new(http)),
        // F5-TTS 的线格式和 OpenAI 兼容完全一致，共用适配器
        TtsEngineKind::OpenAiCompatible | TtsEngineKind::F5Tts => {
            Arc::new(openai::OpenAiTts::new(http, config.clone())?)
        }
        TtsEngineKind::GptSoVits => {
            Arc::new(sovits::SovitsTts::new(http, config.clone(), voices_root)?)
        }
        TtsEngineKind::CosyVoice2 => Arc::new(cosyvoice::CosyVoiceTts::new(http, config.clone())?),
        TtsEngineKind::IndexTts2 => Arc::new(indextts::IndexTts::new(http, config.clone())?),
    })
}

/// 把 Edge 风格的语速字符串（`"+20%"`）换成倍数（`1.2`）。
///
/// 认不出来时返回 1.0 而不是报错：语速格式写错不该让整句话说不出来。
/// 上下界按 OpenAI 协议的 0.25~4.0 夹住 —— 超出范围服务端会拒掉整个请求。
pub fn openai_speed_from_rate(rate: &str) -> f32 {
    match rate.trim().trim_end_matches('%').parse::<f32>() {
        Ok(pct) => (1.0 + pct / 100.0).clamp(0.25, 4.0),
        Err(_) => 1.0,
    }
}

/// WAV 的播放时长。
///
/// 之所以要它：词边界估算需要总时长，而除 Edge 之外的引擎都不给时间戳。
/// 只解析 `fmt ` 与 `data` 两个必需 chunk —— 不做完整 RIFF 解析，
/// 因为这里只需要时长，多解析就多一处会崩的地方。
pub fn wav_duration_ms(bytes: &[u8]) -> Option<u64> {
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let u16le = |o: usize| u16::from_le_bytes([bytes[o], bytes[o + 1]]) as u64;
    let u32le =
        |o: usize| u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]) as u64;

    let mut pos = 12;
    let mut byte_rate = 0u64;
    // chunk 的顺序不保证，也可能夹着 LIST/fact 之类，所以要遍历
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32le(pos + 4);
        let body = pos + 8;
        if id == b"fmt " && body + 16 <= bytes.len() {
            let channels = u16le(body + 2);
            let sample_rate = u32le(body + 4);
            let bits = u16le(body + 14);
            byte_rate = sample_rate * channels * (bits / 8);
        } else if id == b"data" {
            if byte_rate == 0 {
                return None;
            }
            // size 字段可能是流式写入时留下的占位（0 或超长），以实际长度为准
            let actual = (bytes.len() - body) as u64;
            let data_len = if size == 0 || size > actual {
                actual
            } else {
                size
            };
            return Some(data_len * 1000 / byte_rate);
        }
        // chunk 按偶数对齐
        pos = body + size as usize + (size % 2) as usize;
    }
    None
}

/// 给裸 PCM 包 WAV 头。**实现在 [`crate::audio::wav`]** ——
/// 那是音频层的事，不是 TTS 的事。这里只是个转发，方便 tts 内部调用
pub use crate::audio::wav::wrap_pcm_s16le;

/// 在没有真实时间戳时，按字符权重把总时长摊到每个词上。
///
/// **这是估算，不是测量。** 用途只有一个：驱动数字人的嘴型。
/// 嘴型对不齐几十毫秒看不出来，但完全没有时间轴就只能张着嘴不动，
/// 那个更糟。所以宁可估。
///
/// 权重按发音耗时的粗略比例给：汉字/假名一个音节，拉丁字母按词计，
/// 标点给停顿而不给嘴型。
pub fn estimate_word_boundaries(text: &str, total_ms: u64) -> Vec<WordBoundary> {
    #[derive(Clone)]
    struct Unit {
        text: String,
        weight: f64,
        /// 标点只占时间不出声，不该产生嘴型
        silent: bool,
    }

    let mut units: Vec<Unit> = Vec::new();
    let mut latin = String::new();
    let flush_latin = |latin: &mut String, units: &mut Vec<Unit>| {
        if !latin.is_empty() {
            // 拉丁词按字母数折算音节，下限 1
            let weight = (latin.chars().count() as f64 / 2.6).max(1.0);
            units.push(Unit {
                text: std::mem::take(latin),
                weight,
                silent: false,
            });
        }
    };

    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch == '\'' {
            latin.push(ch);
            continue;
        }
        flush_latin(&mut latin, &mut units);
        if ch.is_whitespace() {
            continue;
        }
        // 句读给更长的停顿：不给的话每句之间嘴型会连着动，很假
        let (weight, silent) = match ch {
            '。' | '！' | '？' | '.' | '!' | '?' => (1.6, true),
            '，' | '、' | '；' | '：' | ',' | ';' | ':' => (1.0, true),
            _ if ch.is_ascii_punctuation() => (0.4, true),
            // CJK 一字一音节
            _ => (1.0, false),
        };
        units.push(Unit {
            text: ch.to_string(),
            weight,
            silent,
        });
    }
    flush_latin(&mut latin, &mut units);

    let total_weight: f64 = units.iter().map(|u| u.weight).sum();
    if total_weight <= 0.0 || total_ms == 0 {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut acc = 0.0f64;
    for unit in units {
        let start = (acc / total_weight) * total_ms as f64;
        acc += unit.weight;
        let end = (acc / total_weight) * total_ms as f64;
        if unit.silent {
            continue;
        }
        out.push(WordBoundary {
            offset_ms: start.round() as u64,
            duration_ms: (end - start).round().max(1.0) as u64,
            text: unit.text,
        });
    }
    out
}

/// 一个词的发音时间点。
///
/// 阶段三的数字人对口型（viseme）直接用这个 —— edge-tts 的
/// `audio.metadata` 帧本来就带词边界，不用另找方案。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WordBoundary {
    /// 相对音频起点的偏移（毫秒）
    pub offset_ms: u64,
    /// 该词的时长（毫秒）
    pub duration_ms: u64,
    pub text: String,
}

/// 一句话的合成结果
#[derive(Debug, Clone)]
pub struct Speech {
    /// 完整的 MP3 数据（24kHz 48kbps 单声道）
    pub audio: Vec<u8>,
    pub words: Vec<WordBoundary>,
}

/// 音色与韵律配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsConfig {
    /// 音色名，如 `zh-CN-XiaoxiaoNeural`
    pub voice: String,
    /// 语速，如 `+0%` / `+20%` / `-10%`
    pub rate: String,
    /// 音调，如 `+0Hz`
    pub pitch: String,
    /// 音量，如 `+0%`
    pub volume: String,
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            // 与用户已有 Python 原型一致：晓晓，女声，温暖自然
            voice: "zh-CN-XiaoxiaoNeural".into(),
            rate: "+0%".into(),
            pitch: "+0Hz".into(),
            volume: "+0%".into(),
        }
    }
}

/// 可选音色（供设置面板展示，从服务端实时拉取而非写死）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceInfo {
    pub name: String,
    pub short_name: String,
    pub gender: String,
    pub locale: String,
}

#[async_trait]
pub trait TextToSpeech: Send + Sync {
    /// 合成一句话。返回完整音频，可直接丢进播放队列。
    async fn synthesize(&self, text: &str, config: &TtsConfig) -> Result<Speech>;

    /// 拉取可用音色列表
    async fn voices(&self) -> Result<Vec<VoiceInfo>>;

}

#[cfg(test)]
mod tests {
    use super::*;

    /// 拼一个最小合法 WAV 头，用于时长解析测试
    fn wav(sample_rate: u32, channels: u16, bits: u16, data_len: u32) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend(b"RIFF");
        b.extend((36 + data_len).to_le_bytes());
        b.extend(b"WAVE");
        b.extend(b"fmt ");
        b.extend(16u32.to_le_bytes());
        b.extend(1u16.to_le_bytes()); // PCM
        b.extend(channels.to_le_bytes());
        b.extend(sample_rate.to_le_bytes());
        let byte_rate = sample_rate * channels as u32 * (bits as u32 / 8);
        b.extend(byte_rate.to_le_bytes());
        b.extend((channels * bits / 8).to_le_bytes());
        b.extend(bits.to_le_bytes());
        b.extend(b"data");
        b.extend(data_len.to_le_bytes());
        b.extend(std::iter::repeat_n(0u8, data_len as usize));
        b
    }

    #[test]
    fn wav_duration_is_computed_from_the_header() {
        // 24kHz 单声道 16bit → 每秒 48000 字节
        assert_eq!(wav_duration_ms(&wav(24000, 1, 16, 48000)), Some(1000));
        assert_eq!(wav_duration_ms(&wav(24000, 1, 16, 24000)), Some(500));
        // 立体声每秒字节数翻倍，时长应减半
        assert_eq!(wav_duration_ms(&wav(24000, 2, 16, 48000)), Some(500));
    }

    #[test]
    fn wav_duration_skips_unknown_chunks() {
        // 真实文件常在 fmt 和 data 之间夹 LIST；只认前两个 chunk 会读错
        let mut b = wav(24000, 1, 16, 0);
        let insert_at = b.len() - 8; // "data" 头之前
        let mut extra = Vec::new();
        extra.extend(b"LIST");
        extra.extend(4u32.to_le_bytes());
        extra.extend(b"INFO");
        b.splice(insert_at..insert_at, extra);
        b.extend(std::iter::repeat_n(0u8, 48000));
        assert_eq!(wav_duration_ms(&b), Some(1000));
    }

    #[test]
    fn wav_duration_trusts_actual_length_over_a_bogus_size_field() {
        // 流式写出的 wav 常把 data size 留成 0 或写成占位的超大值
        let mut b = wav(24000, 1, 16, 0);
        b.extend(std::iter::repeat_n(0u8, 48000));
        assert_eq!(wav_duration_ms(&b), Some(1000));
    }

    #[test]
    fn non_wav_input_yields_no_duration() {
        // mp3 之类要如实返回 None，不能瞎猜一个值出来
        assert_eq!(wav_duration_ms(b"ID3\x04\x00\x00\x00\x00\x00\x00"), None);
        assert_eq!(wav_duration_ms(&[]), None);
    }

    #[test]
    fn estimated_boundaries_cover_the_whole_duration_in_order() {
        let words = estimate_word_boundaries("你好世界", 4000);
        assert_eq!(words.len(), 4, "四个汉字应当四个词元：{words:?}");
        assert_eq!(words[0].offset_ms, 0);
        // 等权重 → 每字 1 秒
        for (i, w) in words.iter().enumerate() {
            assert_eq!(w.offset_ms, i as u64 * 1000, "第 {i} 个字的起点");
            assert_eq!(w.duration_ms, 1000);
        }
        // 不留空洞也不越界
        let last = words.last().unwrap();
        assert_eq!(last.offset_ms + last.duration_ms, 4000);
    }

    #[test]
    fn punctuation_takes_time_but_produces_no_mouth_shape() {
        // 标点要占时间（否则句间嘴型会连着动），但本身不该出嘴型
        let words = estimate_word_boundaries("你好。世界", 5000);
        assert!(
            words.iter().all(|w| w.text != "。"),
            "标点不该出现在词边界里：{words:?}"
        );
        // 句号占掉的那 1.6 份权重体现为一段空隙
        let gap_start = words[1].offset_ms + words[1].duration_ms;
        assert!(
            words[2].offset_ms > gap_start,
            "句号后应当有停顿，实际 {} → {}",
            gap_start,
            words[2].offset_ms
        );
    }

    #[test]
    fn latin_words_are_kept_whole_not_split_per_letter() {
        // 按字母切会让英文单词产生一串极短的嘴型，看起来像抽搐
        let words = estimate_word_boundaries("说 hello 吧", 3000);
        let texts: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(texts, vec!["说", "hello", "吧"]);
    }

    #[test]
    fn empty_or_zero_duration_input_is_handled() {
        assert!(estimate_word_boundaries("", 1000).is_empty());
        assert!(estimate_word_boundaries("你好", 0).is_empty());
        // 纯标点没有可发音单元
        assert!(estimate_word_boundaries("。。。", 1000).is_empty());
    }

    #[test]
    fn wrapped_pcm_is_a_decodable_wav_with_the_right_duration() {
        // 24kHz 单声道 16bit，1 秒 = 48000 字节
        let pcm = vec![0u8; 48_000];
        let wav = wrap_pcm_s16le(&pcm, 24_000, 1);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(wav_duration_ms(&wav), Some(1000));
        assert_eq!(wav.len(), 44 + 48_000);
    }

    #[test]
    fn odd_length_pcm_drops_the_dangling_byte() {
        // 半个采样点会被解成噪声，听起来是一声爆音
        let wav = wrap_pcm_s16le(&vec![0u8; 4801], 24_000, 1);
        assert_eq!(wav.len(), 44 + 4800);
    }

    #[test]
    fn engine_defaults_are_distinct_and_include_edge() {
        let engines = TtsEngineConfig::defaults();
        let ids: Vec<&str> = engines.iter().map(|e| e.id.as_str()).collect();
        assert!(ids.contains(&"edge"), "Edge 必须在默认列表里：{ids:?}");
        // id 重复会让"按 id 找引擎"取到错的那个
        let unique: std::collections::BTreeSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len(), "引擎 id 不能重复：{ids:?}");
        // 本地引擎必须带 base_url，否则一选就报错
        for e in &engines {
            if e.kind != TtsEngineKind::Edge {
                assert!(!e.base_url.is_empty(), "{} 缺少默认 base_url", e.id);
            }
            // 返回裸 PCM 的引擎必须有采样率，否则包头会用错默认值、音调偏移
            if matches!(e.kind, TtsEngineKind::CosyVoice2 | TtsEngineKind::IndexTts2) {
                assert!(e.pcm_sample_rate.is_some(), "{} 缺少 pcm_sample_rate", e.id);
            }
        }
    }

    #[test]
    fn sovits_requires_a_reference_audio_and_says_so() {
        // 零样本克隆没有参考音频就无从克隆，错误信息要说清要填什么
        let mut config = TtsEngineConfig::defaults()
            .into_iter()
            .find(|e| e.kind == TtsEngineKind::GptSoVits)
            .unwrap();
        config.ref_audio_path = None;
        // 音色库根传一个确定为空的临时目录 —— 现在有两个参考来源
        // （手填路径 / 共享音色库），只有两个都没有才该报错
        let empty = tempfile::tempdir().unwrap();
        // Arc<dyn TextToSpeech> 没有 Debug，所以不能用 unwrap_err
        let msg = match engine(&config, reqwest::Client::new(), empty.path()) {
            Err(err) => err.to_string(),
            Ok(_) => panic!("缺参考音频时必须报错，而不是等到说话时才失败"),
        };
        assert!(msg.contains("参考音频"), "实际：{msg}");
    }
}

#[cfg(test)]
mod wire_format {
    use super::*;

    /// 前端按 kind 字符串分支显示不同的表单字段。这个串对不上的话
    /// 界面不会报错，只会**永远不显示那些字段** —— 静默失效最难查，
    /// 所以把实际序列化结果钉在测试里。
    #[test]
    fn engine_kind_wire_names_are_what_the_frontend_expects() {
        let cases = [
            (TtsEngineKind::Edge, "\"edge\""),
            (TtsEngineKind::OpenAiCompatible, "\"openai_compatible\""),
            (TtsEngineKind::GptSoVits, "\"gpt_sovits\""),
            (TtsEngineKind::CosyVoice2, "\"cosy_voice2\""),
            (TtsEngineKind::IndexTts2, "\"index_tts2\""),
        ];
        for (kind, expected) in cases {
            assert_eq!(
                serde_json::to_string(&kind).unwrap(),
                expected,
                "{kind:?} 的线格式变了，src/api/voice.ts 的 TtsEngineKind 要同步改"
            );
        }
    }

    /// 改名之前写进 config.json 的值必须还能读。
    ///
    /// 少了 alias 的后果不是"这个字段读不出来"，而是**整份配置解析失败**、
    /// 全部设置退回默认值 —— 一个字段的改名炸掉用户所有配置。
    #[test]
    fn legacy_wire_names_still_deserialize() {
        let cases = [
            ("\"open_ai_compatible\"", TtsEngineKind::OpenAiCompatible),
            ("\"gpt_so_vits\"", TtsEngineKind::GptSoVits),
        ];
        for (json, expected) in cases {
            let parsed: TtsEngineKind = serde_json::from_str(json)
                .unwrap_or_else(|e| panic!("旧值 {json} 应当仍能解析：{e}"));
            assert_eq!(parsed, expected);
        }
    }
}

#[cfg(test)]
mod voice_library_tests {
    use super::*;

    /// **只有 GPT-SoVITS 认音色库里的名字。**
    ///
    /// 角色的 `emotion_voices` 是 `yaemiko/greeting` 这种参考音频路径。
    /// 套到 Edge / Kokoro 上，它们会拿它当自己那套音色名去用 ——
    /// 结果是合成失败或者退回默认嗓子，**而且不报错**。
    ///
    /// 判断依据不是拍脑袋：看 `engine()` 就知道，只有 GptSoVits 收 `voices_root`
    #[test]
    fn only_the_cloning_engine_reads_the_voice_library() {
        assert!(uses_voice_library(TtsEngineKind::GptSoVits));
        for kind in [
            TtsEngineKind::Edge,
            TtsEngineKind::OpenAiCompatible,
            TtsEngineKind::CosyVoice2,
            TtsEngineKind::IndexTts2,
            TtsEngineKind::F5Tts,
        ] {
            assert!(!uses_voice_library(kind), "{kind:?} 不该被当成认音色库的");
        }
    }

    /// 新加引擎时默认落到"不认音色库"这一侧 —— 那是安全的一侧：
    /// 顶多是情绪不换音色，而不是把参考音频路径当音色名发出去
    #[test]
    fn a_new_engine_defaults_to_not_using_the_library() {
        let all = [
            TtsEngineKind::Edge,
            TtsEngineKind::OpenAiCompatible,
            TtsEngineKind::GptSoVits,
            TtsEngineKind::CosyVoice2,
            TtsEngineKind::IndexTts2,
            TtsEngineKind::F5Tts,
        ];
        assert_eq!(all.iter().filter(|k| uses_voice_library(**k)).count(), 1);
    }
}

#[cfg(test)]
mod voice_namespace_tests {
    use super::*;

    fn engine(kind: TtsEngineKind, voice: Option<&str>) -> TtsEngineConfig {
        let mut e = TtsEngineConfig::defaults().remove(0);
        e.kind = kind;
        e.voice = voice.map(str::to_string);
        e
    }

    /// 引擎记着音色就用它自己那个
    #[test]
    fn an_engine_uses_its_own_remembered_voice() {
        assert_eq!(
            effective_voice(&engine(TtsEngineKind::Edge, Some("zh-TW-HsiaoYuNeural"))),
            "zh-TW-HsiaoYuNeural"
        );
        assert_eq!(
            effective_voice(&engine(TtsEngineKind::OpenAiCompatible, Some("zf_xiaobei"))),
            "zf_xiaobei"
        );
    }

    /// **Edge 必须有一个合法音色名**，给空的话它不知道用谁
    #[test]
    fn edge_falls_back_to_a_real_voice() {
        assert_eq!(
            effective_voice(&engine(TtsEngineKind::Edge, None)),
            "zh-CN-XiaoxiaoNeural"
        );
        assert_eq!(
            effective_voice(&engine(TtsEngineKind::Edge, Some("  "))),
            "zh-CN-XiaoxiaoNeural"
        );
    }

    /// 别的引擎给空 = 用它自己的默认。零样本克隆的几家靠参考音频决定音色，
    /// **硬塞一个名字反而是错的**
    #[test]
    fn other_engines_fall_back_to_empty_not_to_an_edge_name() {
        for kind in [
            TtsEngineKind::OpenAiCompatible,
            TtsEngineKind::GptSoVits,
            TtsEngineKind::CosyVoice2,
        ] {
            let v = effective_voice(&engine(kind, None));
            assert!(v.is_empty(), "{kind:?} 退回了 {v}，那是别家的音色名");
        }
    }

    /// 只有 GPT-SoVITS 认音色库里的路径
    #[test]
    fn only_sovits_reads_the_shared_voice_library() {
        assert!(uses_voice_library(TtsEngineKind::GptSoVits));
        for kind in [
            TtsEngineKind::Edge,
            TtsEngineKind::OpenAiCompatible,
            TtsEngineKind::CosyVoice2,
        ] {
            assert!(!uses_voice_library(kind), "{kind:?} 不该去解析音色库路径");
        }
    }
}
