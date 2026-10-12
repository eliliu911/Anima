//! 语音识别（STT）。
//!
//! 上层只依赖 [`SpeechToText`] trait。阶段二用 [`whisper`]（whisper.cpp +
//! Metal 加速，完全本地、不联网）。
//!
//! 输入约定：**16kHz 单声道 f32**，正好是 [`crate::audio::capture`] 的输出格式，
//! 也是 Whisper 本身要求的采样率，所以中间不需要再转换。

pub mod whisper;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcript {
    pub text: String,
    /// 识别出的语种（`zh` / `en` …）。配置成自动检测时用得上。
    pub language: String,
    /// 平均 token 概率（0~1）。判"这句是不是 Whisper 凭空编的"要用它 ——
    /// whisper.cpp 的 no_speech_thold 是空实现，只能自己从 token 概率算。
    /// 没有识别内容时为 0
    pub confidence: f32,
}

// PartialEq：语音会话启动时要判断"缓存里那个模型是不是同一份配置加载的"，
// 配置一致才能复用（换模型、改线程数都必须重新加载）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SttConfig {
    /// GGML 模型文件路径。换模型（base → small → large-v3-turbo）只改这里，
    /// 不用改代码。
    pub model_path: String,
    /// 识别语种。`auto` 表示自动检测；明确指定能少一步检测、也更准。
    pub language: String,
    /// 解码线程数。0 表示由实现按 CPU 核数决定。
    #[serde(default)]
    pub threads: u16,
    /// 是否翻译成英文（几乎总是 false —— 我们要原文）
    #[serde(default)]
    pub translate: bool,
}

impl Default for SttConfig {
    fn default() -> Self {
        Self {
            model_path: String::new(),
            // 与用户已有 Python 原型一致：固定中文
            language: "zh".into(),
            threads: 0,
            translate: false,
        }
    }
}

/// 默认模型存放目录：`<应用配置目录>/models/`。
///
/// 首次启动时 `model_path` 是空的，前端应引导用户下载或指定模型。
/// 这里只负责给出约定位置，不自动下载 —— 几百 MB 的下载应该由用户明确触发。
pub fn default_model_dir(config_dir: &std::path::Path) -> std::path::PathBuf {
    config_dir.join("models")
}

/// 在约定目录里找一个可用的 ggml 模型。
///
/// 存在的意义：用户手动把 `ggml-*.bin` 丢进去就能直接用，不必再去设置里填路径。
pub fn discover_model(config_dir: &std::path::Path) -> Option<String> {
    let dir = default_model_dir(config_dir);
    let entries = std::fs::read_dir(dir).ok()?;

    let mut candidates: Vec<_> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("ggml-") && n.ends_with(".bin"))
                .unwrap_or(false)
        })
        .collect();

    // 文件大的通常是更强的模型，优先用它
    candidates.sort_by_key(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0));
    candidates
        .pop()
        .and_then(|p| p.to_str().map(|s| s.to_string()))
}

#[async_trait]
pub trait SpeechToText: Send + Sync {
    /// 把一段 16kHz 单声道 f32 音频转成文字
    async fn transcribe(&self, samples: &[f32]) -> Result<Transcript>;
}
