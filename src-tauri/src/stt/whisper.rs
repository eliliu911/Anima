//! whisper.cpp（whisper-rs）实现，开 Metal 走 Apple GPU。
//!
//! 两个工程上的注意点：
//!
//! 1. **推理是同步且吃满 CPU/GPU 的**，不能直接在 async 任务里跑 —— 会把
//!    tokio 的工作线程堵住，连带影响正在流式输出的 LLM 和正在播放的 TTS。
//!    所以扔到 `spawn_blocking`。
//! 2. **模型加载很慢**（small 约几百毫秒到数秒），必须复用同一个 context，
//!    不能每次识别都重新加载。context 本身不是 `Sync`，所以用 `Mutex` 串行化 ——
//!    反正同一时刻只有一句话要识别，不存在并发需求。

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use super::{SpeechToText, SttConfig, Transcript};
use crate::error::{Error, Result};

/// 短于这个长度的音频直接跳过。
///
/// Whisper 对 <0.3 秒的输入经常凭空「幻听」出一句话（典型是「谢谢观看」
/// 这类训练数据里的高频尾句），宁可当作没听见。
const MIN_SAMPLES: usize = (0.3 * crate::audio::vad::SAMPLE_RATE as f32) as usize;

pub struct WhisperStt {
    /// Arc + Mutex：clone 给 spawn_blocking 用，Mutex 保证串行推理
    context: Arc<Mutex<WhisperContext>>,
    config: SttConfig,
    threads: i32,
}

impl WhisperStt {
    /// 加载模型。耗时操作，应该在应用启动时做一次。
    pub fn load(config: SttConfig) -> Result<Self> {
        if config.model_path.is_empty() {
            return Err(Error::config(
                "未配置 Whisper 模型路径，请先在设置里指定 ggml-*.bin".to_string(),
            ));
        }
        if !std::path::Path::new(&config.model_path).exists() {
            return Err(Error::config(format!(
                "Whisper 模型文件不存在：{}",
                config.model_path
            )));
        }

        // whisper.cpp 默认会往 stderr 逐 token 打调试日志（`whisper_full_with_state:
        // id = 0, decoder = 0, token = …`）。那不只是噪声 —— 一句话几千行输出本身
        // 就要耗掉数秒，直接把识别延迟拖垮。FullParams 的 set_print_* 管不到它，
        // 必须装这个钩子把 C++ 侧日志接进 Rust 的日志门面，由 EnvFilter 统一过滤。
        // 内部有 Once 保护，重复调用安全。
        whisper_rs::install_logging_hooks();

        let started = std::time::Instant::now();
        let context = WhisperContext::new_with_params(
            &config.model_path,
            WhisperContextParameters::default(),
        )
        .map_err(|e| Error::config(format!("加载 Whisper 模型失败：{e}")))?;

        // 0 表示自动：留一两个核给音频线程和 UI，别把机器占死
        let threads = if config.threads > 0 {
            config.threads as i32
        } else {
            let cores = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4);
            (cores.saturating_sub(2)).max(2) as i32
        };

        tracing::info!(
            model = %config.model_path,
            threads,
            elapsed = ?started.elapsed(),
            "Whisper 模型已加载"
        );

        Ok(Self {
            context: Arc::new(Mutex::new(context)),
            config,
            threads,
        })
    }
}

#[async_trait]
impl SpeechToText for WhisperStt {
    async fn transcribe(&self, samples: &[f32]) -> Result<Transcript> {
        if samples.len() < MIN_SAMPLES {
            // 太短就直接返回空，避免 Whisper 幻听
            return Ok(Transcript {
                text: String::new(),
                language: self.config.language.clone(),
                confidence: 0.0,
            });
        }

        let context = Arc::clone(&self.context);
        let samples = samples.to_vec();
        let language = self.config.language.clone();
        let translate = self.config.translate;
        let threads = self.threads;

        // 推理是同步阻塞的，扔到专门的阻塞线程池，别堵住 tokio
        let joined = tokio::task::spawn_blocking(move || -> Result<Transcript> {
            let ctx = context.blocking_lock();
            let mut state = ctx
                .create_state()
                .map_err(|e| Error::decode(format!("创建 Whisper state 失败：{e}")))?;

            // Greedy 就够了：对话场景下 beam search 带来的准确率提升有限，
            // 但延迟明显更高，而语音链路对延迟敏感。
            let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
            params.set_n_threads(threads);
            params.set_translate(translate);
            // **"auto" 也要传下去。** 原先是"auto 时不设置"，而 whisper.cpp
            // 在没设置语种时的默认是**英文**，不是自动检测 —— 于是配成 auto
            // 实际等于强制英文。实测撞到过：中文素材被转写成英文句子
            // （"As long as you understand, you will find the topic..."），
            // 而这种错误在下游的表现是"音色像但吐字乱"，极难归因。
            //
            // whisper.cpp 本身认 "auto" 这个语种代码，直接传给它才是自动检测。
            params.set_language(Some(&language));
            // 关掉所有实时打印，否则会往 stdout 刷一堆东西污染日志
            params.set_print_special(false);
            params.set_print_progress(false);
            params.set_print_realtime(false);
            params.set_print_timestamps(false);
            // 抑制非语音 token（呼吸、咳嗽标记之类）
            params.set_suppress_blank(true);

            state
                .full(params, &samples)
                .map_err(|e| Error::decode(format!("Whisper 推理失败：{e}")))?;

            let segments = state
                .full_n_segments()
                .map_err(|e| Error::decode(format!("读取识别结果失败：{e}")))?;

            let mut text = String::new();
            // 平均 token 概率 —— 判"是不是幻听"唯一有用的数字。
            // whisper.cpp 的 no_speech_thold 是空实现（whisper-rs 的文档里写着
            // "not implemented"），所以只能自己从 token 概率算
            let mut prob_sum = 0.0f32;
            let mut prob_n = 0usize;
            for i in 0..segments {
                if let Ok(segment) = state.full_get_segment_text(i) {
                    text.push_str(&segment);
                }
                let tokens = state.full_n_tokens(i).unwrap_or(0);
                for t in 0..tokens {
                    // 跳过特殊/时间戳 token，它们的概率跟内容无关
                    let is_special = state
                        .full_get_token_text(i, t)
                        .map(|s| s.starts_with("[_") || s.starts_with("<|"))
                        .unwrap_or(true);
                    if is_special {
                        continue;
                    }
                    if let Ok(p) = state.full_get_token_prob(i, t) {
                        prob_sum += p;
                        prob_n += 1;
                    }
                }
            }
            let confidence = if prob_n == 0 {
                0.0
            } else {
                prob_sum / prob_n as f32
            };

            let cleaned = clean_transcript(&text);
            if !cleaned.is_empty() && looks_like_hallucination(&cleaned, confidence) {
                tracing::info!(
                    text = %cleaned,
                    confidence = %format!("{confidence:.2}"),
                    "丢弃疑似幻听的识别结果"
                );
                return Ok(Transcript {
                    text: String::new(),
                    language,
                    confidence,
                });
            }

            Ok(Transcript {
                text: cleaned,
                language,
                confidence,
            })
        })
        .await
        .map_err(|e| Error::Io(format!("识别任务异常退出：{e}")))?;

        joined
    }
}

/// 说不出口的字幕体。这些是 Whisper 训练语料（大量带轴字幕）留下的高频尾句，
/// 在**没有人说话**时最容易被它凭空吐出来。日志里真实撞到过的：
/// `字幕:J Chong`、`菲爾:哈囉`、`謝謝`、`拜拜` —— 全是繁体，而这台机器上的
/// 对话通篇简体，这本身就是它来自字幕语料的指纹。
///
/// 只列**没人会对着助手说出来的整句**。像"谢谢""拜拜"这种单独出现时无法区分
/// 真假的短语不放进来 —— 那种交给可信度和音量闸门判，宁可放过也不误杀。
const SUBTITLE_ARTIFACTS: &[&str] = &[
    "谢谢观看",
    "謝謝觀看",
    "感谢观看",
    "感謝觀看",
    "谢谢收看",
    "謝謝收看",
    "请订阅",
    "請訂閱",
    "订阅我的频道",
    "訂閱我的頻道",
    "字幕由",
    "字幕组",
    "字幕組",
    "中文字幕",
    "下集再见",
    "下集再見",
];

/// 平均 token 概率的硬地板：低于它的结果一律当作没听见，不论长短。
///
/// **这些数是量出来的**（`cargo run --example stt-probe`，本机 ggml-small）：
///
/// | 输入 | 平均 token 概率 | 识别出什么 |
/// |---|---|---|
/// | 真实语音 | 0.72 | 对 |
/// | 真实语音 ×0.05（漏音那么弱） | 0.66 | 对 |
/// | 真实语音 ×0.01 | 0.60 | 仍然对 |
/// | 低电平噪声 | **0.01** | `" injured cr Stonehallviol…` |
/// | 中电平噪声 | **0.03** | `Ola selon puede` |
///
/// 真语音的地板（0.60）和噪声的天花板（0.03）之间差一个数量级，
/// 阈值放在 0.30 两边都留足余量。
///
/// 我原先按"真实语音 0.9 左右"写了 0.70 —— **那是猜的，而且猜错了**，
/// 0.70 会把正常说话也丢掉。这一版的数字来自探针实测。
const MIN_CONFIDENCE: f32 = 0.30;
/// 短句额外收紧一档。幻听几乎总是短句，而短句本身信息量小、
/// 错杀的代价也小（用户再说一遍就行）。
const SHORT_CHARS: usize = 8;
const SHORT_MIN_CONFIDENCE: f32 = 0.50;

/// 这句话是不是 Whisper 凭空编的。
///
/// 背景：麦克风一直开着，风扇声、键盘声、耳机漏音都会让 VAD 判定"有人说话"，
/// 于是一段非语音音频被送进 Whisper，而 Whisper **不会说"我没听见"** ——
/// 它一定会给你一句话。实际后果是 Anima 认真回答一句没人说过的话，
/// 然后自己的回答又被听回去，无限自问自答。
///
/// **可信度不是万能的**：探针能复现的幻听都是低概率的英文乱码（0.01~0.03），
/// 而日志里真实撞到的是**中文字幕套话**（`謝謝`、`字幕:J Chong`），
/// 我没能用合成输入复现出来，也就不知道它们的概率有多高 ——
/// 模型对自己最熟的套话可能相当自信。所以中文套话这一类靠上面的名单和
/// 冒号形状来拦，不指望可信度。
fn looks_like_hallucination(text: &str, confidence: f32) -> bool {
    if SUBTITLE_ARTIFACTS.iter().any(|p| text.contains(p)) {
        return true;
    }
    // "名字:台词" 的字幕行形状。**冒号不会被说出来** —— 前缀短、不含空格，
    // 后面还跟着内容，这个组合在真人口述里基本不出现
    if let Some(idx) = text.find([':', '：']) {
        let label = &text[..idx];
        let spoken = label.chars().count();
        if (1..=6).contains(&spoken)
            && !label.contains(char::is_whitespace)
            && text[idx..].chars().count() > 1
        {
            return true;
        }
    }
    if confidence < MIN_CONFIDENCE {
        return true;
    }
    text.chars().count() <= SHORT_CHARS && confidence < SHORT_MIN_CONFIDENCE
}

/// 清理识别结果。
///
/// Whisper 会输出一些对下游没用、甚至会污染对话历史的东西：
/// 前后空白、`[BLANK_AUDIO]` 这类标记、以及整段被方括号/括号包住的音效描述。
fn clean_transcript(raw: &str) -> String {
    let trimmed = raw.trim();

    // 整段都是 [xxx] 或 (xxx) 的，是音效/静音标记而非说话内容
    let looks_like_annotation = (trimmed.starts_with('[') && trimmed.ends_with(']'))
        || (trimmed.starts_with('(') && trimmed.ends_with(')'))
        || (trimmed.starts_with('（') && trimmed.ends_with('）'));
    if looks_like_annotation {
        return String::new();
    }

    trimmed
        .replace("[BLANK_AUDIO]", "")
        .replace("[SOUND]", "")
        .replace("[MUSIC]", "")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 日志里真实出现过的四条。前两条形状上不可能是人说的，
    /// 后两条得靠可信度 —— 单独看"謝謝"是没法判真假的
    #[test]
    fn real_hallucinations_from_the_log_are_dropped() {
        // 字幕署名行：没人会把这句话说出口
        assert!(looks_like_hallucination("字幕:J Chong", 0.9));
        // "名字:台词" 形状
        assert!(looks_like_hallucination("菲爾:哈囉", 0.9));
        // 低可信度的短句
        assert!(looks_like_hallucination("謝謝", 0.4));
        assert!(looks_like_hallucination("拜拜", 0.3));
    }

    /// **这一半更重要。** 过滤器宁可放过也不能误杀 ——
    /// 把用户真说的话吞掉，表现是"叫它没反应"，比多回一句更难查
    #[test]
    fn genuine_speech_survives() {
        // 探针实测：真实语音 0.60~0.72。这一档必须全部放过
        assert!(!looks_like_hallucination("謝謝", 0.72));
        assert!(!looks_like_hallucination("拜拜", 0.66));
        assert!(!looks_like_hallucination("你好", 0.60));
        assert!(!looks_like_hallucination(
            "你好，我现在正在说一句比较长的话",
            0.60
        ));
        // 用户真的可能提到字幕
        assert!(!looks_like_hallucination("把字幕关掉", 0.7));
    }

    /// 长的英文乱码（噪声上编出来的）也要拦住 —— 它不短，靠硬地板
    #[test]
    fn long_gibberish_is_dropped_by_the_floor() {
        assert!(looks_like_hallucination(
            "\" injured cr Stonehallviol the of the",
            0.01
        ));
    }

    /// 冒号规则不能把正常带冒号的话吃掉
    #[test]
    fn colon_rule_only_targets_the_subtitle_shape() {
        // 前缀太长，不是说话人标签
        assert!(!looks_like_hallucination(
            "我今天想说的是这样一件事：明天要开会",
            0.7
        ));
        // 冒号后面没内容
        assert!(!looks_like_hallucination("好的：", 0.7));
    }

    #[test]
    fn trims_whitespace() {
        assert_eq!(clean_transcript("  你好  "), "你好");
    }

    #[test]
    fn drops_pure_annotation_segments() {
        // 整段都是标记 —— 不该进对话历史
        assert_eq!(clean_transcript("[BLANK_AUDIO]"), "");
        assert_eq!(clean_transcript("(音乐)"), "");
        assert_eq!(clean_transcript("（笑声）"), "");
    }

    #[test]
    fn strips_inline_markers_but_keeps_speech() {
        assert_eq!(clean_transcript("你好[BLANK_AUDIO]"), "你好");
    }

    #[test]
    fn keeps_text_containing_brackets_mid_sentence() {
        // 只有「整段是标记」才丢，句中出现括号是正常内容
        assert_eq!(
            clean_transcript("他说（很小声）我知道了"),
            "他说（很小声）我知道了"
        );
    }

    /// `unwrap_err` 要求 T: Debug，而 WhisperStt 内含不可 Debug 的 WhisperContext
    fn expect_load_err(config: SttConfig) -> Error {
        match WhisperStt::load(config) {
            Ok(_) => panic!("期望加载失败，实际成功了"),
            Err(err) => err,
        }
    }

    #[test]
    fn missing_model_path_is_a_config_error() {
        let err = expect_load_err(SttConfig::default());
        assert!(err.to_string().contains("模型"), "实际：{err}");
    }

    #[test]
    fn nonexistent_model_file_is_reported_clearly() {
        let err = expect_load_err(SttConfig {
            model_path: "/definitely/not/here/ggml-tiny.bin".into(),
            ..Default::default()
        });
        assert!(err.to_string().contains("不存在"), "实际：{err}");
    }
}
