//! GPT-SoVITS 的 `api_v2.py`。
//!
//! 这是二次元音色的主要来源：它做**零样本克隆** —— 音色由参考音频决定，
//! 不是由一个音色名决定的。所以这个引擎的"选音色"实际上是"选参考音频"，
//! 与 Edge / OpenAI 那套"从目录里挑一个名字"是两种模型。
//!
//! 协议：`POST /tts`，JSON body，返回裸音频（默认 wav）。
//! 词边界同样没有，靠 [`super::estimate_word_boundaries`] 估。

use async_trait::async_trait;
use serde::Serialize;

use super::{TextToSpeech, TtsConfig, TtsEngineConfig, VoiceInfo};
use crate::error::{Error, Result};

pub struct SovitsTts {
    http: reqwest::Client,
    /// 共享音色库的根目录（已解析）。见 `super::engine` 的说明
    voices_root: std::path::PathBuf,
    base_url: String,
    ref_audio_path: String,
    ref_text: String,
}

#[derive(Serialize)]
struct TtsRequest<'a> {
    text: &'a str,
    text_lang: &'a str,
    ref_audio_path: &'a str,
    /// 同一个人的其它参考，做音色融合。空数组等于不融合
    aux_ref_audio_paths: Vec<String>,
    prompt_text: &'a str,
    prompt_lang: &'a str,
    text_split_method: &'a str,
    media_type: &'a str,
    speed_factor: f32,
    streaming_mode: bool,
}

/// 判断一段文本该按哪种语言送给 GPT-SoVITS 的 `text_lang` / `prompt_lang`。
///
/// **不要图省事传 `"auto"`。** 它的自动检测在这里有个会静默毁掉音色的坑：
///
/// `auto` 走的是 `LangSegmenter.getTexts(text)`（不带 default_lang），而
/// **繁体中文会被判成日语** —— 繁体字形和日文汉字大量重叠（過還嗎會著）。
/// 后果是参考文本走了日语 G2P，音素跟参考音频完全对不上，GPT-SoVITS 拿到一对
/// 自相矛盾的（文本, 音频）提示，音色迁移直接崩：听起来"完全不是这个人"，
/// 而且**没有任何报错**。实测过：同一段参考，只把这个参数从 `auto` 改成
/// `all_zh`，音色就回来了。
///
/// `all_zh` 不是"全部按中文读" —— 它是 `getTexts(text, "zh")`，
/// 认不出的段落才默认中文，英文照样会被切出来单独走英文前端。所以对中文素材
/// 它严格优于 `auto`，没有"英文被读成拼音"的副作用（那是我们当初选 auto 的
/// 理由，那个理由是错的）。
///
/// 无假名的日文汉字文本没法和中文区分，这里判成中文 —— 音色库是中文素材，
/// 这个方向猜错的代价更小。真要日语音色，转写稿里几乎必然有假名。
fn lang_of(text: &str) -> &'static str {
    let mut han = false;
    for c in text.chars() {
        match c {
            // 平假名 / 片假名 → 日语。这是唯一可靠的日文标志
            '\u{3040}'..='\u{309F}' | '\u{30A0}'..='\u{30FF}' => return "ja",
            // 谚文
            '\u{AC00}'..='\u{D7AF}' | '\u{1100}'..='\u{11FF}' => return "ko",
            '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}' => han = true,
            _ => {}
        }
    }
    if han {
        "all_zh"
    } else if text.trim().is_empty() {
        // 没有参考文本时这个字段无所谓，留 auto 与旧行为一致
        "auto"
    } else {
        "en"
    }
}

impl SovitsTts {
    pub fn new(
        http: reqwest::Client,
        config: TtsEngineConfig,
        voices_root: &std::path::Path,
    ) -> Result<Self> {
        if config.base_url.trim().is_empty() {
            return Err(Error::config(format!(
                "TTS 引擎「{}」缺少 base_url",
                config.label
            )));
        }
        // 参考音频是必需的 —— 没有它 GPT-SoVITS 不知道要克隆什么音色。
        // **但现在有两个来源**：引擎配置里手工填的路径，或者共享音色库。
        //
        // 所以这里的校验不能再只看 ref_audio_path。原先那样写的后果是：
        // 共享库明明有音色，构造时就直接报错"请填 wav 路径"，而报错文字还写着
        // "不是选一个音色名" —— 正好和新行为相反。
        //
        // 仍然在构造时检查（而不是拖到第一句话才报），因为"两个来源都没有"是个
        // 配置问题，越早说越好。
        let ref_audio_path = config
            .ref_audio_path
            .clone()
            .filter(|p| !p.trim().is_empty())
            .unwrap_or_default();
        if ref_audio_path.is_empty() && super::voices::list_in(voices_root).is_empty() {
            return Err(Error::config(format!(
                "GPT-SoVITS 需要参考音频：音色由参考音频决定。两个办法 ——\
                 往共享音色库（{}）里放 <人名>/<变体>.wav 和同名 .txt/.lab，\
                 再跑 tools/voices/organize.py；\
                 或者在设置里手工填服务端能读到的 wav 路径（5~20 秒清晰人声）。",
                voices_root.display()
            )));
        }

        Ok(Self {
            http,
            voices_root: voices_root.to_path_buf(),
            base_url: config.base_url.trim_end_matches('/').to_string(),
            ref_audio_path,
            // 参考音频对应的文本。留空虽然能跑，但音色相似度会明显下降
            ref_text: config.ref_text.unwrap_or_default(),
        })
    }
}

#[async_trait]
impl TextToSpeech for SovitsTts {
    async fn synthesize(&self, text: &str, config: &TtsConfig) -> Result<super::Speech> {
        let speed = super::openai_speed_from_rate(&config.rate);

        // ── 参考音频：优先从**共享音色库**按 `voice` 解析 ──
        //
        // 引擎配置里的 ref_audio_path / ref_text 仍然有效（手工指定任意文件），
        // 但默认路径是共享库：F5-TTS 和 GPT-SoVITS 用同一套参考，用户在设置里
        // 选一个音色名两边都能用。
        //
        // **原先必须手工填两个字段**（服务端可读的绝对路径 + 那段话的文字），
        // 而且两者必须自己保证一致 —— 写错的表现是"音色像但吐字乱"，极难归因。
        let from_library = (!config.voice.trim().is_empty())
            .then(|| super::voices::resolve_in(&self.voices_root, config.voice.trim()).ok())
            .flatten();
        let (ref_audio, ref_text, aux) = match &from_library {
            Some(v) => (
                v.audio.to_string_lossy().to_string(),
                v.text.clone(),
                // 同前缀组的其它片段一起送进去做融合。见 voices::group_of ——
                // 只用一条参考时音色稳定性受那一条的录音条件支配
                v.aux
                    .iter()
                    .map(|p| p.to_string_lossy().to_string())
                    .collect(),
            ),
            // 手工指定路径的场合没有"同一个人的其它片段"这个概念
            None => (
                self.ref_audio_path.clone(),
                self.ref_text.clone(),
                Vec::new(),
            ),
        };
        if ref_audio.trim().is_empty() {
            return Err(Error::config(
                "GPT-SoVITS 需要参考音频：在设置里选一个音色（共享音色库里的），                 或手工填参考音频路径与参考文本"
                    .to_string(),
            ));
        }

        let resp = self
            .http
            .post(format!("{}/tts", self.base_url))
            .json(&TtsRequest {
                text,
                // 两个语言字段都必须自己判，不能传 "auto" —— 见 [`lang_of`]。
                // prompt_lang 判错的表现是"音色完全不是这个人"且不报错
                text_lang: lang_of(text),
                ref_audio_path: &ref_audio,
                aux_ref_audio_paths: aux,
                prompt_text: &ref_text,
                prompt_lang: lang_of(&ref_text),
                // 切句已经由 Anima 的 SentenceSplitter 做完了，
                // 这里传"不切"，否则会二次切分打乱逐句播放的节奏
                text_split_method: "cut0",
                media_type: "wav",
                speed_factor: speed,
                // 这里要整句返回。流式模式返回的是分块裸 PCM，
                // 而上层期望一句一个可播放的完整音频
                streaming_mode: false,
            })
            .send()
            .await
            .map_err(|e| Error::Network(format!("GPT-SoVITS 请求失败：{e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Network(format!(
                "GPT-SoVITS 返回 {status}：{}",
                body.chars().take(300).collect::<String>()
            )));
        }

        let audio = resp
            .bytes()
            .await
            .map_err(|e| Error::Network(format!("读取音频失败：{e}")))?
            .to_vec();

        if audio.is_empty() {
            return Err(Error::Decode("GPT-SoVITS 返回了空音频".into()));
        }
        // 出错时它也会返回 200 但 body 是 JSON，这种情况要认出来
        if audio.starts_with(b"{") {
            return Err(Error::Network(format!(
                "GPT-SoVITS 返回的不是音频：{}",
                String::from_utf8_lossy(&audio)
                    .chars()
                    .take(300)
                    .collect::<String>()
            )));
        }

        let words = match super::wav_duration_ms(&audio) {
            Some(ms) => super::estimate_word_boundaries(text, ms),
            None => Vec::new(),
        };
        Ok(super::Speech { audio, words })
    }

    async fn voices(&self) -> Result<Vec<VoiceInfo>> {
        // 零样本克隆没有"引擎自带的音色列表"，但**共享音色库里有**。
        // 返回它们，设置面板的下拉框才有东西可选 —— 原先这里返回空数组，
        // 用户面对一个点不开的框，只能去手填路径
        let refs = super::voices::list_in(&self.voices_root);
        if !refs.is_empty() {
            return Ok(refs
                .into_iter()
                .map(|v| VoiceInfo {
                    name: v.name.clone(),
                    short_name: v.name,
                    gender: String::new(),
                    locale: String::new(),
                })
                .collect());
        }
        // 零样本克隆没有"音色列表"这个概念：音色 = 参考音频。
        // 返回空让界面去提示填参考音频，而不是伪造一份列表
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::lang_of;

    #[test]
    fn traditional_chinese_is_chinese_not_japanese() {
        // 这是真实的回归：taimei / ADSister 的转写稿是繁体，传 auto 时被
        // GPT-SoVITS 判成 ja，音色完全跑掉
        assert_eq!(lang_of("嗨，你今天過得還好嗎？我會一直陪著你"), "all_zh");
    }

    #[test]
    fn simplified_and_mixed_are_chinese() {
        assert_eq!(lang_of("人气太高了又不是我的错"), "all_zh");
        // 中英混排仍然判中文 —— all_zh 会把英文段落切出去单独处理
        assert_eq!(lang_of("我在用 LM Studio 跑 Qwen 模型"), "all_zh");
    }

    #[test]
    fn kana_means_japanese() {
        assert_eq!(lang_of("こんにちは、元気ですか"), "ja");
        assert_eq!(lang_of("ドーナツ"), "ja");
        // 汉字在前、假名在后也要认出来（不能只看第一个字）
        assert_eq!(lang_of("元気ですか"), "ja");
    }

    #[test]
    fn hangul_means_korean() {
        assert_eq!(lang_of("안녕하세요"), "ko");
    }

    #[test]
    fn ascii_only_is_english() {
        assert_eq!(lang_of("Hello, this is Anima."), "en");
    }

    #[test]
    fn empty_keeps_auto() {
        assert_eq!(lang_of(""), "auto");
        assert_eq!(lang_of("   "), "auto");
    }
}
