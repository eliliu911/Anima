//! 开口之前先查一遍：**这套引擎 + 这个音色，现在能不能说出话来**。
//!
//! ## 为什么要有它
//!
//! 这个项目上"没声音"查了好几轮，每一次的根因都是**配置不匹配**，而且
//! 每一次都要等到真的要说话时才暴露 —— 用户看到的只是"她不吭声"：
//!
//! | 实际发生的 | 用户看到的 |
//! |---|---|
//! | Kokoro 拿到 Edge 的音色名，去 HuggingFace 找 `voices/zh-TW-HsiaoYuNeural.pt` | 404，没声音 |
//! | GPT-SoVITS 没配参考音频 | "需要参考音频"，没声音 |
//! | Edge 拿到音色库路径 `yaemiko/d` | 合成失败，没声音 |
//! | 本地服务没起来 | 请求超时，没声音 |
//!
//! 这四种的修法完全不同，但症状一模一样。**问题不在于哪一条没写对，
//! 而在于它们全都要到合成那一刻才说话。**
//!
//! 这个模块把判断提前：给一份引擎配置和一个音色名，**不发一次合成请求**
//! 就能说出"它会不会失败、为什么、怎么改"。这是纯函数，所以能测。
//!
//! ## 只做静态判断
//!
//! 服务通不通是另一回事（那要发请求，`probe_tts_engine` 干那个）。
//! 这里只查**光看配置就能看出来的**问题 —— 那正好覆盖了上面四条里的三条。

use super::{TtsEngineConfig, TtsEngineKind};

/// 一条查出来的问题
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Problem {
    /// 人话描述。**要说清楚怎么改** —— 只说"配置错误"等于没说
    pub message: String,
    /// 严重程度：致命的说不出话，警告的能说但多半不是用户想要的
    pub fatal: bool,
}

impl Problem {
    fn fatal(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            fatal: true,
        }
    }
    fn warn(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            fatal: false,
        }
    }
}

/// 一个音色名**看起来**属于哪家。
///
/// 只看形状，不查任何服务：
///
/// - `zh-CN-XiaoxiaoNeural` —— 带地区前缀和 `Neural` 后缀，是 Edge
/// - `yaemiko/d` —— 带斜杠，是共享音色库里的路径，只有 GPT-SoVITS 认
/// - 其余（`af_sky`、`zf_xiaobei`）—— OpenAI 兼容那一类自己的名字
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceFlavor {
    Edge,
    VoiceLibrary,
    Plain,
}

pub fn flavor_of(voice: &str) -> VoiceFlavor {
    let v = voice.trim();
    if v.contains('/') {
        return VoiceFlavor::VoiceLibrary;
    }
    // `xx-YY-*Neural`：Edge 的音色名都是这个形状
    let parts: Vec<&str> = v.split('-').collect();
    if parts.len() >= 3 && v.ends_with("Neural") {
        return VoiceFlavor::Edge;
    }
    VoiceFlavor::Plain
}

/// 查一遍。空数组 = 看不出问题
pub fn check(engine: &TtsEngineConfig, voice: &str) -> Vec<Problem> {
    let mut out = Vec::new();
    let voice = voice.trim();

    // 本地服务型引擎必须有地址
    if !matches!(engine.kind, TtsEngineKind::Edge) && engine.base_url.trim().is_empty() {
        out.push(Problem::fatal(format!(
            "「{}」没填服务地址（base_url），连不上就说不出话",
            engine.label
        )));
    }

    match engine.kind {
        TtsEngineKind::Edge => {
            if voice.is_empty() {
                out.push(Problem::fatal(
                    "Edge 必须指定音色（比如 zh-CN-XiaoxiaoNeural），留空它不知道用谁".to_string(),
                ));
            } else if flavor_of(voice) != VoiceFlavor::Edge {
                out.push(Problem::fatal(format!(
                    "音色「{voice}」不是 Edge 的音色名。\
                     Edge 的名字长这样：zh-CN-XiaoxiaoNeural。\
                     这个名字看起来是别的引擎的 —— 换引擎时音色没跟着换"
                )));
            }
        }
        TtsEngineKind::GptSoVits => {
            // 零样本克隆：音色由参考音频决定。两条路，至少要有一条
            let has_library_voice =
                !voice.is_empty() && flavor_of(voice) == VoiceFlavor::VoiceLibrary;
            let has_manual_ref = engine
                .ref_audio_path
                .as_deref()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            if !has_library_voice && !has_manual_ref {
                out.push(Problem::fatal(
                    "GPT-SoVITS 需要参考音频：在设置里选一个音色（共享音色库里的，\
                     形如 yaemiko/d），或者手工填参考音频路径与参考文本"
                        .to_string(),
                ));
            }
            if has_manual_ref
                && engine
                    .ref_text
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or("")
                    .is_empty()
            {
                out.push(Problem::warn(
                    "填了参考音频但没填参考文本。零样本克隆两个都要，\
                     只有音频的话音色会飘"
                        .to_string(),
                ));
            }
            if flavor_of(voice) == VoiceFlavor::Edge {
                out.push(Problem::fatal(format!(
                    "音色「{voice}」是 Edge 的名字，GPT-SoVITS 不认。\
                     它要的是共享音色库里的路径（形如 yaemiko/d）"
                )));
            }
        }
        TtsEngineKind::OpenAiCompatible | TtsEngineKind::F5Tts => match flavor_of(voice) {
            VoiceFlavor::Edge => out.push(Problem::fatal(format!(
                "音色「{voice}」是 Edge 的名字。\
                     这个引擎（Kokoro 一类）会拿它当文件名去下载，然后 404 —— \
                     换一个它自己的音色名（形如 zf_xiaobei）"
            ))),
            VoiceFlavor::VoiceLibrary => out.push(Problem::fatal(format!(
                "音色「{voice}」是共享音色库里的路径，只有 GPT-SoVITS 认。\
                     这个引擎要它自己的音色名"
            ))),
            VoiceFlavor::Plain => {}
        },
        TtsEngineKind::CosyVoice2 | TtsEngineKind::IndexTts2 => {
            // 这两家也是零样本克隆，音色由参考音频决定，音色名基本不用
            if flavor_of(voice) == VoiceFlavor::Edge {
                out.push(Problem::warn(format!(
                    "音色「{voice}」是 Edge 的名字，这个引擎多半会忽略它 —— \
                     它的音色由参考音频决定"
                )));
            }
        }
    }
    out
}

/// 有没有致命问题（说不出话）
pub fn is_fatal(problems: &[Problem]) -> bool {
    problems.iter().any(|p| p.fatal)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine(kind: TtsEngineKind) -> TtsEngineConfig {
        let mut e = TtsEngineConfig::defaults().remove(0);
        e.kind = kind;
        e.label = format!("{kind:?}");
        e.base_url = "http://127.0.0.1:9880".into();
        e
    }

    #[test]
    fn recognises_which_family_a_voice_name_belongs_to() {
        assert_eq!(flavor_of("zh-CN-XiaoxiaoNeural"), VoiceFlavor::Edge);
        assert_eq!(flavor_of("zh-TW-HsiaoYuNeural"), VoiceFlavor::Edge);
        assert_eq!(flavor_of("yaemiko/d"), VoiceFlavor::VoiceLibrary);
        assert_eq!(flavor_of("furina/greeting"), VoiceFlavor::VoiceLibrary);
        assert_eq!(flavor_of("zf_xiaobei"), VoiceFlavor::Plain);
        assert_eq!(flavor_of("af_sky"), VoiceFlavor::Plain);
    }

    /// 这条是**真实事故**的回归测试：引擎切到 Kokoro，音色还是 Edge 的，
    /// Kokoro 去 HuggingFace 找 `voices/zh-TW-HsiaoYuNeural.pt` 然后 404
    #[test]
    fn catches_an_edge_voice_handed_to_kokoro() {
        let p = check(
            &engine(TtsEngineKind::OpenAiCompatible),
            "zh-TW-HsiaoYuNeural",
        );
        assert!(is_fatal(&p));
        assert!(
            p[0].message.contains("404"),
            "要说清楚会怎么失败：{}",
            p[0].message
        );
    }

    /// 也是真实事故：默认形态 + Edge，但实际跑的是 SoVITS 的音色
    #[test]
    fn catches_a_library_path_handed_to_edge() {
        let p = check(&engine(TtsEngineKind::Edge), "yaemiko/d");
        assert!(is_fatal(&p));
        assert!(
            p[0].message.contains("zh-CN-XiaoxiaoNeural"),
            "要给出正确的样子"
        );
    }

    /// 还是真实事故："GPT-SoVITS 需要参考音频"
    #[test]
    fn catches_sovits_without_any_reference() {
        let p = check(&engine(TtsEngineKind::GptSoVits), "");
        assert!(is_fatal(&p));
        assert!(p[0].message.contains("参考音频"));
    }

    /// 配好了就不该报 —— 误报比不报更糟，用户会学会忽略它
    #[test]
    fn a_correct_setup_reports_nothing() {
        assert!(check(&engine(TtsEngineKind::Edge), "zh-CN-XiaoxiaoNeural").is_empty());
        assert!(check(&engine(TtsEngineKind::GptSoVits), "yaemiko/d").is_empty());
        assert!(check(&engine(TtsEngineKind::OpenAiCompatible), "zf_xiaobei").is_empty());
    }

    /// 零样本克隆也可以手工填参考音频，那样音色名留空是对的
    #[test]
    fn sovits_with_a_manual_reference_is_fine() {
        let mut e = engine(TtsEngineKind::GptSoVits);
        e.ref_audio_path = Some("/x/ref.wav".into());
        e.ref_text = Some("这是参考文本".into());
        assert!(check(&e, "").is_empty());
    }

    /// 只填音频不填文本能跑，但音色会飘 —— 提醒，不拦
    #[test]
    fn a_reference_without_its_text_is_a_warning_not_an_error() {
        let mut e = engine(TtsEngineKind::GptSoVits);
        e.ref_audio_path = Some("/x/ref.wav".into());
        let p = check(&e, "");
        assert!(!is_fatal(&p));
        assert!(p[0].message.contains("参考文本"));
    }

    #[test]
    fn local_engines_need_an_address() {
        let mut e = engine(TtsEngineKind::OpenAiCompatible);
        e.base_url = String::new();
        let p = check(&e, "zf_xiaobei");
        assert!(is_fatal(&p));
        assert!(p[0].message.contains("base_url"));
    }

    /// Edge 走微软固定端点，不该要求填地址
    #[test]
    fn edge_does_not_need_an_address() {
        let mut e = engine(TtsEngineKind::Edge);
        e.base_url = String::new();
        assert!(check(&e, "zh-CN-XiaoxiaoNeural").is_empty());
    }

    /// 每一条都要说清楚怎么改。只说"配置错误"等于没说
    #[test]
    fn every_message_says_how_to_fix_it() {
        let cases = [
            (engine(TtsEngineKind::Edge), "yaemiko/d"),
            (
                engine(TtsEngineKind::OpenAiCompatible),
                "zh-CN-XiaoxiaoNeural",
            ),
            (engine(TtsEngineKind::GptSoVits), ""),
        ];
        for (e, v) in cases {
            for p in check(&e, v) {
                assert!(
                    p.message.chars().count() > 20,
                    "太短，说不清：{}",
                    p.message
                );
            }
        }
    }
}
