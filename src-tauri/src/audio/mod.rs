//! 音频子系统（阶段二）。
//!
//! - [`capture`]：麦克风采集 + VAD 分句，产出可直接送 STT 的 16kHz 单声道音频
//! - [`playback`]：TTS 音频播放队列，支持**立即清空**（barge-in 的另一半）
//! - [`vad`]：语音活动检测，`SpeechStart` 是打断的触发信号
//! - [`wav`]：WAV 容器读写。**这是音频层的事** —— 原先在 tts 里，
//!   于是播放层为了用它反过来依赖了 TTS
//!
//! 设计要点见 ARCHITECTURE.md 坑点 1/2/7：高频数据不走 Tauri command、
//! 打断必须做成可取消状态机、回声消除要有降级方案。

pub mod capture;
pub mod playback;
pub mod silero;
pub mod vad;
pub mod wav;
