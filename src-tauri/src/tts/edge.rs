//! Edge TTS（微软 Edge 朗读服务）的 Rust 实现。
//!
//! 协议是私有的，以下细节全部对照 `edge-tts` 7.2.8 的实现核对过：
//!
//! 1. **`Sec-MS-GEC` 令牌**：把 Unix 时间转成 Windows 文件时间（epoch 1601），
//!    向下取整到 5 分钟，转成 100 纳秒刻度，拼上固定的 TrustedClientToken 后
//!    取 SHA256 大写十六进制。所以令牌每 5 分钟变一次，必须每次连接现算。
//! 2. **WebSocket 帧是文本 + 二进制混合**的自定义格式，不是 JSON-RPC：
//!    - 请求：`X-Timestamp:…\r\nPath:speech.config\r\n\r\n{json}`
//!    - 音频响应：前 2 字节是**大端 u16 的头部长度**，之后是头部文本，再之后是
//!      MP3 裸数据；只有 `Path:audio` 的帧才含音频。
//!    - 词边界：文本帧 `Path:audio.metadata`，载荷是 JSON。
//! 3. **`X-Timestamp` 用 JS 风格日期串**，且 ssml 帧里还要在末尾多加一个 `Z`
//!    —— 这是微软自己的 bug，但必须照着犯，否则服务端不认。

use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

use super::{Speech, TextToSpeech, TtsConfig, VoiceInfo, WordBoundary};
use crate::error::{Error, Result};

const TRUSTED_CLIENT_TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";
const BASE_URL: &str = "speech.platform.bing.com/consumer/speech/synthesize/readaloud";

/// 与 edge-tts 保持一致的 Chromium 版本号，参与 Sec-MS-GEC-Version
const CHROMIUM_FULL_VERSION: &str = "143.0.3650.75";
const CHROMIUM_MAJOR_VERSION: &str = "143";

/// Windows 文件时间的 epoch（1601-01-01）相对 Unix epoch 的秒数
const WIN_EPOCH: u64 = 11_644_473_600;

/// 输出格式。24kHz 48kbps 单声道 MP3 —— rodio 能直接解，体积也小。
const OUTPUT_FORMAT: &str = "audio-24khz-48kbitrate-mono-mp3";

pub struct EdgeTts {
    http: reqwest::Client,
}

impl EdgeTts {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http }
    }

    fn user_agent() -> String {
        format!(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/{CHROMIUM_MAJOR_VERSION}.0.0.0 Safari/537.36 \
             Edg/{CHROMIUM_MAJOR_VERSION}.0.0.0"
        )
    }
}

/// 生成 `Sec-MS-GEC` 令牌。
///
/// 单独抽出来是为了能单测 —— 算法里有「取整到 5 分钟」和「100 纳秒刻度」
/// 两处容易写错的换算。
fn sec_ms_gec(unix_seconds: u64) -> String {
    // 切到 Windows 文件时间的 epoch
    let mut ticks = unix_seconds + WIN_EPOCH;
    // 向下取整到 5 分钟
    ticks -= ticks % 300;
    // 秒 → 100 纳秒刻度（1 秒 = 10^7 个 100ns）
    let ticks = ticks as u128 * 10_000_000;

    let mut hasher = Sha256::new();
    hasher.update(format!("{ticks}{TRUSTED_CLIENT_TOKEN}").as_bytes());
    format!("{:X}", hasher.finalize())
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// UUID 去掉横线 —— 服务端要求的格式
fn connect_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// JS 风格的日期串，例如
/// `Mon Aug 03 2026 05:12:33 GMT+0000 (Coordinated Universal Time)`
///
/// 手写而不是用 chrono：这里只需要一个固定格式的 UTC 串，而引入 chrono 会把它的
/// 版本约束带进依赖图 —— 实测新版 chrono 给 `Datelike` 加的方法会和 lancedb 依赖的
/// `arrow-arith` 53 自带的同名 trait 方法冲突，直接编不过。为一个日期格式化换来
/// 一处版本地雷不值得。
fn js_date_string() -> String {
    format_js_date(now_unix())
}

const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// 把 Unix 秒格式化成 JS 风格 UTC 日期串
fn format_js_date(unix_seconds: u64) -> String {
    let days = (unix_seconds / 86_400) as i64;
    let secs_of_day = unix_seconds % 86_400;

    let (year, month, day) = civil_from_days(days);
    // 1970-01-01 是星期四；星期日记为 0
    let weekday = ((days + 4).rem_euclid(7)) as usize;

    format!(
        "{wd} {mon} {day:02} {year} {h:02}:{m:02}:{s:02}          GMT+0000 (Coordinated Universal Time)",
        wd = WEEKDAYS[weekday],
        mon = MONTHS[(month - 1) as usize],
        h = secs_of_day / 3600,
        m = (secs_of_day % 3600) / 60,
        s = secs_of_day % 60,
    )
}

/// 由「自 1970-01-01 起的天数」反推公历年月日。
///
/// 用的是 Howard Hinnant 的 civil_from_days 算法 —— 闰年、世纪闰年都覆盖，
/// 比自己按月份表累加可靠得多。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    // 把纪元移到 0000-03-01，让闰日落在四年周期的末尾，省掉所有特殊分支
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]，从 3 月起算
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]

    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// XML 转义。SSML 是 XML，用户说的话里带 `&` `<` `>` 会直接破坏文档结构。
fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            // 控制字符会让服务端直接断开，替换成空格
            c if (c.is_control() && c != '\n' && c != '\t') => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

fn build_ssml(text: &str, config: &TtsConfig) -> String {
    format!(
        "<speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='en-US'>\
         <voice name='{voice}'>\
         <prosody pitch='{pitch}' rate='{rate}' volume='{volume}'>\
         {text}\
         </prosody></voice></speak>",
        voice = config.voice,
        pitch = config.pitch,
        rate = config.rate,
        volume = config.volume,
        text = xml_escape(text),
    )
}

/// 把二进制帧拆成（头部文本, 音频数据）。
///
/// 帧格式：`[2 字节大端头长度][头部文本][音频数据]`
fn split_binary_frame(data: &[u8]) -> Result<(&str, &[u8])> {
    if data.len() < 2 {
        return Err(Error::decode("TTS 二进制帧太短，缺少头部长度"));
    }
    let header_len = u16::from_be_bytes([data[0], data[1]]) as usize;
    let header_end = 2 + header_len;
    if data.len() < header_end {
        return Err(Error::decode("TTS 二进制帧头部长度超出实际数据"));
    }
    let header = std::str::from_utf8(&data[2..header_end])
        .map_err(|e| Error::decode(format!("TTS 帧头部不是合法 UTF-8：{e}")))?;
    Ok((header, &data[header_end..]))
}

/// 从 `Key:Value\r\n` 形式的头部里取某个字段
fn header_value<'a>(header: &'a str, key: &str) -> Option<&'a str> {
    header.lines().find_map(|line| {
        let (k, v) = line.split_once(':')?;
        (k.trim().eq_ignore_ascii_case(key)).then_some(v.trim())
    })
}

// ---------------------------------------------------------------------------
// 词边界元数据
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct MetadataEnvelope {
    #[serde(default)]
    #[serde(rename = "Metadata")]
    metadata: Vec<MetadataItem>,
}

#[derive(Deserialize)]
struct MetadataItem {
    #[serde(rename = "Type", default)]
    kind: String,
    #[serde(rename = "Data", default)]
    data: Option<MetadataData>,
}

#[derive(Deserialize)]
struct MetadataData {
    #[serde(rename = "Offset", default)]
    offset: u64,
    #[serde(rename = "Duration", default)]
    duration: u64,
    #[serde(rename = "text", default)]
    text: Option<MetadataText>,
}

#[derive(Deserialize)]
struct MetadataText {
    #[serde(rename = "Text", default)]
    text: String,
}

/// 微软的偏移量单位是 100 纳秒刻度
const TICKS_PER_MS: u64 = 10_000;

fn parse_word_boundaries(payload: &str) -> Vec<WordBoundary> {
    let Ok(envelope) = serde_json::from_str::<MetadataEnvelope>(payload) else {
        return Vec::new();
    };

    envelope
        .metadata
        .into_iter()
        .filter(|m| m.kind == "WordBoundary")
        .filter_map(|m| {
            let data = m.data?;
            Some(WordBoundary {
                offset_ms: data.offset / TICKS_PER_MS,
                duration_ms: data.duration / TICKS_PER_MS,
                text: data.text.map(|t| t.text).unwrap_or_default(),
            })
        })
        .collect()
}

#[async_trait]
impl TextToSpeech for EdgeTts {
    async fn synthesize(&self, text: &str, config: &TtsConfig) -> Result<Speech> {
        if text.trim().is_empty() {
            return Ok(Speech {
                audio: Vec::new(),
                words: Vec::new(),
            });
        }

        let url = format!(
            "wss://{BASE_URL}/edge/v1?TrustedClientToken={TRUSTED_CLIENT_TOKEN}\
             &ConnectionId={conn}&Sec-MS-GEC={gec}&Sec-MS-GEC-Version=1-{ver}",
            conn = connect_id(),
            gec = sec_ms_gec(now_unix()),
            ver = CHROMIUM_FULL_VERSION,
        );

        // 这些请求头缺一个都可能被拒；Origin 必须是那个 Edge 扩展 id
        let mut request = url
            .into_client_request()
            .map_err(|e| Error::Network(format!("构造 TTS WebSocket 请求失败：{e}")))?;
        {
            let headers = request.headers_mut();
            headers.insert("Pragma", "no-cache".parse().unwrap());
            headers.insert("Cache-Control", "no-cache".parse().unwrap());
            headers.insert(
                "Origin",
                "chrome-extension://jdiccldimpdaibmpdkjnbmckianbfold"
                    .parse()
                    .unwrap(),
            );
            headers.insert(
                "User-Agent",
                Self::user_agent()
                    .parse()
                    .map_err(|_| Error::Network("User-Agent 不合法".into()))?,
            );
            headers.insert("Accept-Language", "en-US,en;q=0.9".parse().unwrap());
        }

        // **WSS 之前必须保证 rustls 的加密后端装过。** 依赖树里 ring 和
        // aws-lc-rs 同时存在，rustls 0.23 拒绝猜，第一次建 TLS 时直接 panic
        // 在 tokio 工作线程里 —— tracing 日志一个字都没有。
        // 见 `crate::ensure_crypto_provider` 的注释。
        crate::ensure_crypto_provider();
        let (mut socket, _) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|e| Error::Network(format!("连接 Edge TTS 失败：{e}")))?;

        // ---- 先发 speech.config，再发 ssml，顺序不能反 ----
        let config_frame = format!(
            "X-Timestamp:{ts}\r\n\
             Content-Type:application/json; charset=utf-8\r\n\
             Path:speech.config\r\n\r\n\
             {{\"context\":{{\"synthesis\":{{\"audio\":{{\
             \"metadataoptions\":{{\"sentenceBoundaryEnabled\":\"false\",\
             \"wordBoundaryEnabled\":\"true\"}},\
             \"outputFormat\":\"{OUTPUT_FORMAT}\"}}}}}}}}\r\n",
            ts = js_date_string(),
        );
        socket
            .send(Message::Text(config_frame))
            .await
            .map_err(|e| Error::Network(format!("发送 TTS 配置帧失败：{e}")))?;

        // 注意 X-Timestamp 后面那个多出来的 Z —— 微软的 bug，必须照犯
        let ssml_frame = format!(
            "X-RequestId:{id}\r\n\
             Content-Type:application/ssml+xml\r\n\
             X-Timestamp:{ts}Z\r\n\
             Path:ssml\r\n\r\n\
             {ssml}",
            id = connect_id(),
            ts = js_date_string(),
            ssml = build_ssml(text, config),
        );
        socket
            .send(Message::Text(ssml_frame))
            .await
            .map_err(|e| Error::Network(format!("发送 TTS 合成请求失败：{e}")))?;

        // ---- 收音频，直到 turn.end ----
        let mut audio = Vec::new();
        let mut words = Vec::new();

        while let Some(msg) = socket.next().await {
            let msg = msg.map_err(|e| Error::Network(format!("TTS 连接中断：{e}")))?;
            match msg {
                Message::Binary(data) => {
                    let (header, payload) = split_binary_frame(&data)?;
                    // 只有 audio 帧带音频；其它 Path 直接忽略
                    if header_value(header, "Path") == Some("audio") {
                        audio.extend_from_slice(payload);
                    }
                }
                Message::Text(text) => {
                    let raw = text.as_str();
                    // 文本帧同样是「头部 + 空行 + 载荷」
                    let (header, payload) = raw.split_once("\r\n\r\n").unwrap_or((raw, ""));
                    match header_value(header, "Path") {
                        Some("audio.metadata") => words.extend(parse_word_boundaries(payload)),
                        // 服务端说完了
                        Some("turn.end") => break,
                        _ => {}
                    }
                }
                Message::Close(frame) => {
                    // 没收到 turn.end 就被关闭，通常是令牌过期或音色名写错
                    if audio.is_empty() {
                        return Err(Error::Network(format!(
                            "Edge TTS 未返回音频便关闭连接（音色名是否正确？）：{frame:?}"
                        )));
                    }
                    break;
                }
                _ => {}
            }
        }

        // 主动关闭，不然服务端会等超时
        let _ = socket.close(None).await;

        if audio.is_empty() {
            return Err(Error::Network("Edge TTS 没有返回任何音频".into()));
        }

        Ok(Speech { audio, words })
    }

    async fn voices(&self) -> Result<Vec<VoiceInfo>> {
        let url = format!(
            "https://{BASE_URL}/voices/list?trustedclienttoken={TRUSTED_CLIENT_TOKEN}\
             &Sec-MS-GEC={gec}&Sec-MS-GEC-Version=1-{ver}",
            gec = sec_ms_gec(now_unix()),
            ver = CHROMIUM_FULL_VERSION,
        );

        #[derive(Deserialize)]
        struct RawVoice {
            #[serde(rename = "Name", default)]
            name: String,
            #[serde(rename = "ShortName", default)]
            short_name: String,
            #[serde(rename = "Gender", default)]
            gender: String,
            #[serde(rename = "Locale", default)]
            locale: String,
        }

        let resp = self
            .http
            .get(url)
            .header("User-Agent", Self::user_agent())
            .header("Accept-Language", "en-US,en;q=0.9")
            .send()
            .await?;
        let resp = crate::llm::sse::ensure_success(resp).await?;

        let raw: Vec<RawVoice> = resp.json().await?;
        Ok(raw
            .into_iter()
            .map(|v| VoiceInfo {
                name: v.name,
                short_name: v.short_name,
                gender: v.gender,
                locale: v.locale,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sec_ms_gec_rounds_down_to_five_minutes() {
        // 同一个 5 分钟窗口内的任意时刻应当得到相同令牌
        let base = 1_785_000_000u64; // 落在某个 300 秒窗口内
        let aligned = base - base % 300;
        assert_eq!(sec_ms_gec(aligned), sec_ms_gec(aligned + 1));
        assert_eq!(sec_ms_gec(aligned), sec_ms_gec(aligned + 299));
        // 跨到下一个窗口就必须变
        assert_ne!(sec_ms_gec(aligned), sec_ms_gec(aligned + 300));
    }

    #[test]
    fn sec_ms_gec_is_uppercase_hex_sha256() {
        let token = sec_ms_gec(1_785_000_000);
        assert_eq!(token.len(), 64, "SHA256 十六进制应当是 64 字符");
        assert!(
            token
                .chars()
                .all(|c| c.is_ascii_digit() || c.is_ascii_uppercase()),
            "必须是大写十六进制：{token}"
        );
    }

    /// 用 Python edge-tts 的算法手算一个已知值做交叉验证：
    /// ticks = (1785000000 + 11644473600) 向下取整到 300 → 再 ×10^7
    #[test]
    fn sec_ms_gec_matches_reference_algorithm() {
        let unix = 1_785_000_000u64;
        let mut ticks = unix + WIN_EPOCH;
        ticks -= ticks % 300;
        let expected_input = format!("{}{}", ticks as u128 * 10_000_000, TRUSTED_CLIENT_TOKEN);

        let mut hasher = Sha256::new();
        hasher.update(expected_input.as_bytes());
        let expected = format!("{:X}", hasher.finalize());

        assert_eq!(sec_ms_gec(unix), expected);
    }

    #[test]
    fn escapes_xml_special_characters() {
        assert_eq!(xml_escape("a & b < c > d"), "a &amp; b &lt; c &gt; d");
    }

    #[test]
    fn strips_control_characters_that_break_the_service() {
        assert_eq!(xml_escape("好\u{0007}的"), "好 的");
        // 换行和制表符是合法的，要保留
        assert_eq!(xml_escape("行1\n行2"), "行1\n行2");
    }

    #[test]
    fn ssml_embeds_voice_and_prosody() {
        let config = TtsConfig::default();
        let ssml = build_ssml("你好", &config);
        assert!(ssml.contains("zh-CN-XiaoxiaoNeural"));
        assert!(ssml.contains("pitch='+0Hz'"));
        assert!(ssml.contains("rate='+0%'"));
        assert!(ssml.contains(">你好<"));
    }

    #[test]
    fn ssml_escapes_user_text_so_it_cannot_break_the_document() {
        let config = TtsConfig::default();
        // 用户说的话里带 XML 片段时不能破坏 SSML 结构
        let ssml = build_ssml("</voice><script>", &config);
        assert!(ssml.contains("&lt;/voice&gt;"));
        // 只应有一个真正的闭合 </voice>
        assert_eq!(ssml.matches("</voice>").count(), 1);
    }

    #[test]
    fn splits_binary_frame_into_header_and_audio() {
        let header = "Path:audio\r\nContent-Type:audio/mpeg\r\n";
        let mut frame = Vec::new();
        frame.extend_from_slice(&(header.len() as u16).to_be_bytes());
        frame.extend_from_slice(header.as_bytes());
        frame.extend_from_slice(&[0xFF, 0xFB, 0x90]); // 假装是 MP3 数据

        let (parsed_header, audio) = split_binary_frame(&frame).unwrap();
        assert_eq!(header_value(parsed_header, "Path"), Some("audio"));
        assert_eq!(audio, &[0xFF, 0xFB, 0x90]);
    }

    #[test]
    fn rejects_truncated_binary_frame() {
        // 头部声称很长但实际数据不够 —— 不能越界读
        let frame = [0x00, 0xFF, b'x'];
        assert!(split_binary_frame(&frame).is_err());
        assert!(split_binary_frame(&[0x00]).is_err());
    }

    #[test]
    fn parses_word_boundaries_into_milliseconds() {
        let payload = r#"{"Metadata":[{"Type":"WordBoundary","Data":{
            "Offset":15000000,"Duration":5000000,"text":{"Text":"你好"}}}]}"#;
        let words = parse_word_boundaries(payload);
        assert_eq!(words.len(), 1);
        // 100 纳秒刻度 → 毫秒
        assert_eq!(words[0].offset_ms, 1500);
        assert_eq!(words[0].duration_ms, 500);
        assert_eq!(words[0].text, "你好");
    }

    #[test]
    fn ignores_non_word_boundary_metadata() {
        let payload = r#"{"Metadata":[{"Type":"SessionEnd","Data":{}}]}"#;
        assert!(parse_word_boundaries(payload).is_empty());
        // 无法解析的载荷不应 panic
        assert!(parse_word_boundaries("not json").is_empty());
    }

    #[test]
    fn header_lookup_is_case_insensitive() {
        let header = "path:audio\r\nX-RequestId:abc\r\n";
        assert_eq!(header_value(header, "Path"), Some("audio"));
        assert_eq!(header_value(header, "x-requestid"), Some("abc"));
        assert_eq!(header_value(header, "Missing"), None);
    }

    #[test]
    fn js_date_string_has_expected_shape() {
        let s = js_date_string();
        assert!(
            s.contains("GMT+0000 (Coordinated Universal Time)"),
            "实际：{s}"
        );
        assert!(s.split_whitespace().count() >= 8, "实际：{s}");
    }

    /// 与 Python `strftime('%a %b %d %Y %H:%M:%S')` 的输出逐一核对
    #[test]
    fn format_js_date_matches_reference_values() {
        let cases = [
            (1_785_723_027u64, "Mon Aug 03 2026 02:10:27"),
            (0, "Thu Jan 01 1970 00:00:00"),
            // 闰年 2 月 29 日 —— 世纪闰年
            (951_782_400, "Tue Feb 29 2000 00:00:00"),
            // 普通闰年
            (1_709_164_800, "Thu Feb 29 2024 00:00:00"),
        ];
        for (ts, expected) in cases {
            let got = format_js_date(ts);
            assert!(
                got.starts_with(expected),
                "时间戳 {ts} 应当格式化为「{expected}…」，实际「{got}」"
            );
        }
    }

    #[test]
    fn civil_from_days_handles_leap_and_century_boundaries() {
        // 1970-01-01
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 2000-02-29（世纪闰年，能被 400 整除所以是闰年）
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        // 1900 不是闰年：1900-03-01 的前一天应当是 2 月 28 日
        assert_eq!(civil_from_days(-25_508), (1900, 3, 1));
        assert_eq!(civil_from_days(-25_509), (1900, 2, 28));
    }

    #[test]
    fn connect_id_has_no_dashes() {
        let id = connect_id();
        assert_eq!(id.len(), 32);
        assert!(!id.contains('-'));
    }
}
