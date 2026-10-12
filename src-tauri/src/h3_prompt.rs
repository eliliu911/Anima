//! MiniMax H3 的提示词编译。
//!
//! # 为什么要单独一层
//!
//! H3 **不是**「一段描述 → 一段视频」。它期待一份有结构的稿子：三个具名字段、
//! 带说话人 ID 的台词。写成散文它照样出片，只是**画面和声音对不上**，
//! 而这件事不报错 —— 任务成功、文件也在、时长也对。
//!
//! 规格全文在 `skills/film-director/reference/h3-prompt-format.md`
//! （提炼自 `ComfyUI-MiniMaxH3-Prompt-Writer` 的官方指南）。
//! 那份文档是这个模块的说明书，改这里之前先去读它。
//!
//! # 这里解决的三个实际症状
//!
//! - **乱入背景音乐**：`non_diegetic_music` 字段以前根本不存在。H3 的音轨和
//!   画面一起无条件生成（`audio_vae` 是模型的一部分，工作流里没有开关），
//!   没人跟它说配乐的事，它就自己编一段。**没有内容时要写 `N/A`，
//!   留空不等于没有。**
//! - **双人对话口型全跑到一个人身上**：以前发的是 `角色说：台词`，
//!   没有说话人。画面里两个人，模型只能猜，猜错不报错。
//! - **动作生硬**：运镜以前是词表里的一个中文词（「跟拍」），
//!   H3 要的是「类型 + 幅度 + 速度」写成句子。
//!
//! # 一次生成只有一个镜头
//!
//! Anima 的一镜 = 一次 H3 调用 = 一段视频，所以永远只有 `[Shot 1]`，
//! 不需要切点。多镜头稿子（`At 00:03.500, the camera cuts to...`）是
//! 单次生成多镜的写法，我们这条链路用不到。

/// 一句台词。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct H3Line {
    /// 说话人的显示名（资产名，或剧本里写的名字）。
    pub speaker: String,
    /// 台词原文。**逐字保留，连标点一起**，不翻译不改写。
    pub text: String,
    /// 音高 / 音色 / 语速 / 口音。空着也能出，但首次出现时交代够效果更稳。
    pub voice: String,
    /// 画外音。为真时编译出 off-screen voiceover，并紧接着写明嘴唇不动 ——
    /// 不写这一句，画面里的人就会跟着旁白动嘴。
    pub off_screen: bool,
}

/// H3 中文的自然语速，**可发声字/秒**。
///
/// 数据来自 `drama-skills` 的实测（同一镜 20 次生成，7~108 字 × 5/10/13 秒）：
///
/// | 送入字数 ÷ 镜长 | 表现 |
/// |---|---|
/// | ≤ 约 4.1 | 自然语速，完整 |
/// | 约 4.1 ~ 9 | 完整但**明显赶**（最快到自然语速的 2.5 倍） |
/// | > 约 9 | **静默截断**：超出的部分根本不发声 |
pub const NATURAL_CPS: f64 = 4.1;

/// 超过这个就会被静默截断。
pub const TRUNCATE_CPS: f64 = 9.0;

/// 台词容量的核算结果。
#[derive(Debug, Clone, PartialEq)]
pub enum DialogueFit {
    /// 自然语速说得完。
    Natural,
    /// 说得完，但明显赶。
    Rushed { cps: f64, suggest_secs: f64 },
    /// **会被静默截断** —— 而且接口照样返回成功。
    Truncated { cps: f64, spoken: usize, suggest_secs: f64 },
}

/// 可发声字数。**不含标点** —— 这是实测口径。
///
/// 把标点算进去会高估约两成，而高估的方向恰好是「以为装不下」，
/// 比低估安全，但仍然会误报。
pub fn speakable_chars(text: &str) -> usize {
    text.chars()
        .filter(|c| {
            !c.is_whitespace()
                && !c.is_ascii_punctuation()
                && !matches!(
                    c,
                    '，' | '。' | '！' | '？' | '；' | '：' | '、' | '…'
                        | '「' | '」' | '『' | '』' | '（' | '）' | '《' | '》'
                        | '—' | '·' | '“' | '”' | '‘' | '’'
                )
        })
        .count()
}

/// 这一镜的台词在这个时长里说不说得完。
///
/// # 为什么必须在生成**之前**算
///
/// 截断时**接口不报错**：任务返回成功、画面正常、时长也对，
/// 只是台词后半段没有了。文字审查看不出来，秒数加总也仍然"正确" ——
/// **算术通过从来不是台词说得完的证据**。
///
/// 实测里送 78 字和送 108 字**都只说了 46 字** —— 多写的部分完全蒸发。
///
/// 延长镜头是有效的（实测：37 字从 5 秒改到 10 秒，语速 7.0 → 4.3，恢复完整），
/// 所以这里给出建议时长，而不是只说"太长了"。
pub fn check_dialogue_fit(lines: &[H3Line], duration_secs: f64) -> DialogueFit {
    let chars: usize = lines.iter().map(|l| speakable_chars(&l.text)).sum();
    let secs = duration_secs.max(0.1);
    let cps = chars as f64 / secs;
    let suggest = (chars as f64 / NATURAL_CPS * 10.0).ceil() / 10.0;
    if cps <= NATURAL_CPS {
        DialogueFit::Natural
    } else if cps <= TRUNCATE_CPS {
        DialogueFit::Rushed { cps, suggest_secs: suggest }
    } else {
        DialogueFit::Truncated {
            cps,
            spoken: (TRUNCATE_CPS * secs) as usize,
            suggest_secs: suggest,
        }
    }
}

impl DialogueFit {
    /// 说给人听的一句话。`None` = 没问题，不用打扰用户。
    pub fn warning(&self) -> Option<String> {
        match self {
            DialogueFit::Natural => None,
            DialogueFit::Rushed { cps, suggest_secs } => Some(format!(
                "台词偏长：{cps:.1} 字/秒（自然语速 {NATURAL_CPS}）。                 能说完，但会明显赶。镜头延长到 {suggest_secs:.1} 秒可恢复自然。"
            )),
            DialogueFit::Truncated { cps, spoken, suggest_secs } => Some(format!(
                "**台词会被静默截断**：{cps:.1} 字/秒，超过 {TRUNCATE_CPS} 的阈值，                 大约只会说出前 {spoken} 个字，剩下的根本不发声 ——                  而任务会返回成功、画面正常，听不出来就发现不了。                 镜头延长到 {suggest_secs:.1} 秒，或者拆镜。"
            )),
        }
    }
}

/// 把 `名字：台词` 这种一行拆成说话人和台词。
///
/// 剧本里本来就是这么写的，而 `StoryboardShot::dialogue` 存的是原文。
/// 拆不出说话人时返回 `None`，调用方回落到出场角色。
///
/// 支持 `名字（压低声音）：台词` —— 括号里的当作语气，进 `voice`。
pub fn split_speaker(raw: &str) -> Option<(String, String, String)> {
    let line = raw.trim();
    // 全角冒号优先 —— 中文剧本用的是它
    let at = line.find('：').or_else(|| line.find(':'))?;
    let (head, tail) = line.split_at(at);
    let text = tail.trim_start_matches(['：', ':']).trim();
    if text.is_empty() {
        return None;
    }
    let head = head.trim();
    // **拆不出来就不拆。** 判据有两条，都是为了别把正文里的冒号当成说话人：
    //
    // 1. 名字不会很长（12 字够放「村民（压低声音）」这种带语气的写法）
    // 2. 名字里不会有句读 —— 有逗号句号的几乎一定是叙述，不是人名
    //
    // 拆不出来时调用方回落到这一镜的出场角色，比认错人安全。
    const SENTENCE_MARKS: [char; 12] =
        ['，', '。', '！', '？', '；', '、', ',', '.', '!', '?', ';', '…'];
    if head.is_empty()
        || head.chars().count() > 12
        || head.contains(SENTENCE_MARKS)
    {
        return None;
    }
    // 括号里的当语气
    let (name, voice) = match head.find(['（', '(']) {
        Some(i) => {
            let (n, v) = head.split_at(i);
            (n.trim(), v.trim_matches(['（', '(', '）', ')']).trim())
        }
        None => (head, ""),
    };
    if name.is_empty() {
        return None;
    }
    Some((name.to_string(), text.to_string(), voice.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(speaker: &str, text: &str) -> H3Line {
        H3Line { speaker: speaker.into(), text: text.into(), voice: String::new(), off_screen: false }
    }



    /// **实测口径：不含标点。** 算进去会高估约两成。
    #[test]
    fn 可发声字数不含标点() {
        assert_eq!(speakable_chars("你哪来的？"), 4);
        assert_eq!(speakable_chars("我就住在山那边。"), 7);
        assert_eq!(speakable_chars("「别动，」他说。"), 4);
        assert_eq!(speakable_chars("I'm leaving."), 9);
    }

    /// **接口不报错，所以必须在生成之前算。**
    ///
    /// 实测：送 78 字和送 108 字**都只说了 46 字** —— 多写的部分完全蒸发，
    /// 而任务返回成功、画面正常、时长也对。
    #[test]
    fn 台词超容量会被判成静默截断() {
        let long = "这条路我走了三十年，从来没见过有人往山那边去，你要是真想进去，得趁天亮，天一黑路就找不着了，到时候谁也帮不了你。";
        let lines = [H3Line {
            speaker: "老村民".into(), text: long.into(),
            voice: String::new(), off_screen: false,
        }];
        let fit = check_dialogue_fit(&lines, 5.0);
        match &fit {
            DialogueFit::Truncated { cps, spoken, suggest_secs } => {
                assert!(*cps > TRUNCATE_CPS, "{cps}");
                assert!(*spoken < speakable_chars(long), "截断点该小于全文");
                assert!(*suggest_secs > 5.0, "该建议延长镜头");
            }
            other => panic!("这么长的台词不该判成 {other:?}"),
        }
        let w = fit.warning().expect("必须给出警告");
        assert!(w.contains("静默截断"), "{w}");
        assert!(w.contains("返回成功"), "得说清楚为什么听不出来：{w}");
    }

    /// 中间那一档：说得完但赶。**给建议时长，不是只说「太长了」** ——
    /// 实测里延长镜头确实管用（37 字 5 秒 → 10 秒，语速 7.0 回到 4.3）。
    #[test]
    fn 台词偏长时提示赶并给建议时长() {
        // 37 个可发声字，5 秒 → 7.4 字/秒
        let text = "这条路我走了三十年从来没见过有人往山那边去你要是真想进去得趁天亮";
        let lines = [H3Line {
            speaker: "老村民".into(), text: text.into(),
            voice: String::new(), off_screen: false,
        }];
        match check_dialogue_fit(&lines, 5.0) {
            DialogueFit::Rushed { cps, suggest_secs } => {
                assert!(cps > NATURAL_CPS && cps <= TRUNCATE_CPS, "{cps}");
                // 32 字 ÷ 4.1 ≈ 7.9 秒
                assert!(suggest_secs > 7.0 && suggest_secs < 9.0, "{suggest_secs}");
            }
            other => panic!("应该判成赶，实际 {other:?}"),
        }
    }

    #[test]
    fn 台词说得完时不打扰用户() {
        let lines = [H3Line {
            speaker: "她".into(), text: "你哪来的？".into(),
            voice: String::new(), off_screen: false,
        }];
        let fit = check_dialogue_fit(&lines, 5.0);
        assert_eq!(fit, DialogueFit::Natural);
        assert_eq!(fit.warning(), None, "没问题就别弹警告");
    }

    #[test]
    fn 没有台词时不算超容量() {
        assert_eq!(check_dialogue_fit(&[], 1.0), DialogueFit::Natural);
    }








    #[test]
    fn 从剧本原文拆说话人() {
        assert_eq!(
            split_speaker("沈星晴：你哪来的？"),
            Some(("沈星晴".into(), "你哪来的？".into(), String::new()))
        );
        // 括号里的当语气
        assert_eq!(
            split_speaker("村民（压低声音）：别问了。"),
            Some(("村民".into(), "别问了。".into(), "压低声音".into()))
        );
        // 半角冒号
        assert_eq!(
            split_speaker("Anna: I'm leaving."),
            Some(("Anna".into(), "I'm leaving.".into(), String::new()))
        );
        // 正文里的冒号不是说话人分隔符
        // 正文里的冒号不是说话人分隔符
        assert_eq!(split_speaker("她看着窗外，心里想的是另一件事"), None);
        // 太长 —— 人名不会这么长
        assert_eq!(
            split_speaker("这一段很长的叙述文字里恰好出现了一个冒号：后面还有很多内容"),
            None
        );
        // 带句读 —— 有逗号的几乎一定是叙述
        assert_eq!(split_speaker("她停下，回头：你说什么"), None);
        // 但正常的名字要拆得出来，包括八个字带语气的
        assert_eq!(
            split_speaker("穿灰夹克的男人：站住。"),
            Some(("穿灰夹克的男人".into(), "站住。".into(), String::new()))
        );
        // 冒号后面空的不算
        assert_eq!(split_speaker("沈星晴："), None);
    }




}

/// 打真实 ComfyUI 的验证。**默认 `#[ignore]`**，要本机 H3 权重和跑着的 ComfyUI。
///
/// ```bash
/// cd src-tauri && cargo test --lib h3_live -- --ignored --nocapture
/// ```
///
/// **为什么非得有这一层**：格式对不对是模型侧的事，单元测试只能证明
/// 我们发出去的字符串长什么样，证明不了 H3 听不听。而这条链路的失败全是
/// 静默的 —— 任务成功、文件也在、时长也对，只是声音不对。
#[cfg(test)]
mod live {

    fn out_dir() -> std::path::PathBuf {
        let d = std::path::PathBuf::from("/tmp/anima-h3-live");
        let _ = std::fs::create_dir_all(&d);
        d
    }

    async fn render(name: &str, prompt: &str) -> std::path::PathBuf {
        use crate::agent_runtime::weetodd;
        let graph = weetodd::build_h3_graph(&weetodd::H3Params {
            continuity: None,
            mode: weetodd::H3Mode::Text,
            prompt,
            width: 640,
            height: 384,
            duration_secs: 5.0,
            seed: 20_260_910,
            tier: weetodd::H3Tier::Preview,
            filename_prefix: "Anima/h3live",
            loras: &[],
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap();
        let out = out_dir().join(format!("{name}.mp4"));
        let bytes = crate::agent_runtime::video_tool::submit_and_fetch_video(
            &client, &graph, name, Some(&out),
        )
        .await
        .unwrap_or_else(|e| panic!("{name} 出片失败：{e}"));
        std::fs::write(&out, &bytes).unwrap();
        println!("  {name}: {} ({:.1} MB)", out.display(), bytes.len() as f64 / 1e6);
        out
    }


    /// 把出片的音轨转写出来核对。**这一步不能省。**
    ///
    /// 有音轨、mean_volume 正常，都不代表台词对 —— 那只是「有声音」。
    /// 项目自己的记录里就有：源片 ffprobe 报 `aac,audio`、任务成功，
    /// 而实际是数字静音。**统计值只能否定，不能肯定。**
    #[tokio::test]
    #[ignore = "要本机 ComfyUI 出片 + Whisper 模型"]
    async fn h3_live_转写核对台词() {
        use crate::stt::{whisper::WhisperStt, SpeechToText, SttConfig};

        let model = dirs::config_dir()
            .map(|d| d.join("com.anima.desktop/models/ggml-small.bin"))
            .filter(|p| p.is_file())
            .expect("找不到 ggml-small.bin");
        let stt = WhisperStt::load(SttConfig {
            model_path: model.to_string_lossy().into(),
            language: "zh".into(),
            ..Default::default()
        })
        .expect("加载 Whisper 失败");

        for name in ["new_format", "old_format"] {
            let mp4 = out_dir().join(format!("{name}.mp4"));
            if !mp4.is_file() {
                println!("  {name}: 还没出片，先跑 h3_live_新老格式对照");
                continue;
            }
            // 转成 Whisper 要的 16k 单声道 f32
            let wav = out_dir().join(format!("{name}.f32"));
            let ok = std::process::Command::new("/opt/homebrew/bin/ffmpeg")
                .args(["-y", "-v", "error", "-i"])
                .arg(&mp4)
                .args(["-vn", "-ac", "1", "-ar", "16000", "-f", "f32le"])
                .arg(&wav)
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            assert!(ok, "{name} 抽音轨失败");
            let bytes = std::fs::read(&wav).unwrap();
            let samples: Vec<f32> = bytes
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect();
            let peak = samples.iter().fold(0f32, |a, s| a.max(s.abs()));
            let t = stt.transcribe(&samples).await.expect("转写失败");
            println!("
  === {name} ===");
            println!("  峰值 {peak:.4}  置信度 {:.3}", t.confidence);
            println!("  转写：{}", t.text.trim());
        }
        println!("
  对照台词：「你是哪里人？」/「我就住在山那边。」");
        println!("  **还要自己听一遍** —— 转写对不代表口型对在人身上。");
    }

}
