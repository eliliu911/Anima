//! 短剧配音：**按角色音色逐句合成**，拼成一镜一条音轨。
//!
//! # 这条路和 H3 自带的声音是二选一
//!
//! 拼接时 [`crate::cinema::pick_shot_audio`] 的规则是「有 `audio/<镜头id>.wav`
//! 就用它，否则用源视频自带的音轨」。而源视频自带的音轨是 H3 生成的语音
//! —— 它的**口型是跟着那段语音做的**。
//!
//! 所以配音一旦生成，这一镜就变成了配音片：音色、方言、情绪都归我们管，
//! 但**口型对不上**。这是个真取舍，不是能两全的事，界面上必须说清楚。
//!
//! 什么时候值得换：
//!
//! - 要特定音色（同一个角色跨集必须是同一把嗓子）；
//! - 要方言 —— H3 读的是规范汉字，写方言用字只会被按普通话念；
//! - H3 这一镜的配音听着不对，而画面是好的（重出一次要几分钟，配音几秒）。
//!
//! # 音色绑在角色上，不绑在分镜上
//!
//! | 管什么 | 放哪 | 为什么 |
//! |---|---|---|
//! | **哪把嗓子** | 角色资产 `character.voice_id` | 同一个人跨集必须是同一把嗓子 |
//! | **这一句怎么说** | 台词行 `emotion` | 每句都不同，绑角色没意义 |
//! | **例外** | 台词行 `voice_override` | 回忆里的少年版、电话失真、旁白兼任 |
//!
//! 绑在分镜上的话，一集三十镜要配三十次，改一次要改三十处 ——
//! 那是「一份清单抄在多处」，这个仓库反复栽的那个坑。
//!
//! **覆盖必须看得见。** 不显示的话会出现「我明明改了角色音色，这一镜怎么没变」，
//! 而那种问题从现象上完全查不回来。[`plan`] 返回的 `overridden` 就是给界面标的。
//!
//! # 别和 `DialogueLine.voice` 搞混
//!
//! 两个字段长得像，服务的是**两条不同的音频链路**：
//!
//! | 字段 | 是什么 | 给谁 |
//! |---|---|---|
//! | `DialogueLine.voice` | 中文描述（「沙哑，带川渝口音」） | H3，写进提示词 |
//! | `character.voice_id` | 模板名（`catboss`） | TTS 引擎，本模块 |
//!
//! 一条是「描述给模型听」，一条是「点名一份参考音频」。
//!
//! # 音色从哪来
//!
//! 角色资产的 `character.voice_id`，直接当 [`crate::tts::TtsConfig::voice`] 用。
//! 各引擎对这个名字的解释不同，**这是有意的**：
//!
//! | 引擎 | `voice_id` 是什么 |
//! |---|---|
//! | GPT-SoVITS | 共享音色库（`<应用数据目录>/voices/`）里的名字，零样本克隆 |
//! | F5-TTS / OpenAI 兼容 | 服务端自己的音色名 |
//! | Edge | `zh-CN-XiaoxiaoNeural` 这类固定音色 |
//!
//! 只有 GPT-SoVITS 读我们的音色库（见 [`crate::tts::uses_voice_library`]）。
//!
//! # 补静音不是修饰，是正确性
//!
//! 拼接那一步带 `-shortest`，**取最短的那一路**。配音比画面短的话
//! 视频会被截掉一截 —— 表现是「这一镜莫名其妙短了半秒」，不报任何错。
//! 所以末尾用 `apad=whole_dur=<画面时长>` 垫到画面长度。
//!
//! `whole_dur` 而不是 `-t`：前者只补不截，配音比画面长时原样留着
//! （`-shortest` 会在拼接时裁掉尾巴，但那是**台词写太长**，要报给用户看，
//! 不该在这里悄悄切掉）。

use crate::cinema::DialogueLine;
use crate::error::{Error, Result};
use crate::tts::{TextToSpeech, TtsConfig};
use std::path::Path;
use std::sync::Arc;

/// 句与句之间的停顿。
///
/// 0 的话两句会黏在一起，听着像一个人抢话。太长又会让一镜的配音超出画面。
/// 0.28 秒是普通对话轮换的下限附近。
pub const GAP_SECS: f32 = 0.28;

/// 一句要合成的话。
#[derive(Debug, Clone, PartialEq)]
pub struct DubLine {
    pub speaker: String,
    pub text: String,
    /// 给引擎的音色名。空 = 用全局设置里的那个。
    pub voice: String,
    /// 这一句**没用角色的音色**，用的是台词行上的覆盖。
    ///
    /// 界面要标出来 —— 不标的话「改了角色音色这一镜却没变」查不回来。
    pub overridden: bool,
    /// 这一句的语气（中文）。目前只写进日志和返回值，
    /// **没有哪个本地克隆引擎收这个参数** —— 留着是为了将来接
    /// CosyVoice2 的 instruct 模式时不用再改一遍数据流。
    pub tone: String,
}

/// 镜头台词 → 要合成的句子表。
///
/// `voice_of` 按说话人名字给音色（没有就回 `None`）。`fallback` 是
/// 全局设置里的音色，角色没配时用它。
///
/// **空台词跳过**，画外音照常合成（它本来就只有声音）。
pub fn plan(
    lines: &[DialogueLine],
    voice_of: impl Fn(&str) -> Option<String>,
    fallback: &str,
) -> Vec<DubLine> {
    lines
        .iter()
        .filter(|l| !l.line.trim().is_empty())
        .map(|l| {
            let speaker = l.speaker.trim().to_string();
            // 三级：台词行覆盖 → 角色音色 → 全局默认。**空串一律当没配** ——
            // 把空串发给引擎的表现是「换了音色但没变化」，完全查不出原因。
            let over = l.voice_override.trim();
            let overridden = !over.is_empty();
            let voice = if overridden {
                over.to_string()
            } else {
                voice_of(&speaker)
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
                    .unwrap_or_else(|| fallback.trim().to_string())
            };
            let tone = [l.voice.trim(), l.emotion.trim()]
                .iter()
                .filter(|t| !t.is_empty())
                .copied()
                .collect::<Vec<_>>()
                .join("，");
            DubLine { speaker, text: l.line.trim().to_string(), voice, overridden, tone }
        })
        .collect()
}

/// 拼接用的 `filter_complex`。
///
/// # 为什么不用 `-f concat`
///
/// 那条路**只读第一个文件的流参数**，后面的文件不管实际是什么都按第一个解释
/// （这个仓库在片头拼接上栽过一次）。各句的采样率本来就可能不同 ——
/// 同一个引擎换音色都可能变。所以每一路显式 `aresample` + `aformat` 归一。
///
/// # 末尾的 `apad`
///
/// `whole_dur` 只补不截：配音短了垫到画面长度（否则 `-shortest` 会把画面截短），
/// 配音长了原样留着（那是台词写太长，要报出来，不该在这里悄悄切）。
pub fn concat_filter(n: usize, gap: f32, video_secs: f32) -> String {
    let mut parts: Vec<String> = Vec::new();
    for i in 0..n {
        parts.push(format!(
            "[{i}:a]aresample=24000,aformat=sample_fmts=s16:channel_layouts=mono,apad=pad_dur={gap}[a{i}]"
        ));
    }
    let chain: String = (0..n).map(|i| format!("[a{i}]")).collect();
    parts.push(format!("{chain}concat=n={n}:v=0:a=1,apad=whole_dur={video_secs}[out]"));
    parts.join(";")
}

/// 配音比画面长多少秒。`None` = 放得下。
///
/// 放不下的后果是**台词尾巴被砍掉**（拼接带 `-shortest`），而且不报错。
/// 所以这件事必须原样报给用户：要么改短台词，要么把这一镜重出得长一点。
pub fn overflow(speech_secs: f32, video_secs: f32) -> Option<f32> {
    // 0.15 秒以内当对齐误差，不打扰用户
    let d = speech_secs - video_secs;
    (d > 0.15).then_some(d)
}

/// 这一镜配音之后会怎么样，给界面看的一句话。
pub fn tradeoff_note(has_source_audio: bool) -> &'static str {
    if has_source_audio {
        "这一镜会改用配音，**替换掉 H3 生成的原声**。音色和方言归你管，代价是口型对不上（原口型是跟着 H3 那段语音做的）。"
    } else {
        "这一镜本来就没有声音，配上去只有好处。"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(speaker: &str, text: &str) -> DialogueLine {
        DialogueLine { speaker: speaker.into(), line: text.into(), ..Default::default() }
    }

    /// 每个人用自己的音色 —— **这就是「音色模板」的全部意思**。
    /// 一镜里两个人用同一把嗓子是这条路最容易犯的错。
    #[test]
    fn 每个说话人用各自的音色() {
        let lines = vec![line("小林", "半个？"), line("橘猫", "它现在还立着。")];
        let out = plan(&lines, |s| match s {
            "小林" => Some("linzhi".into()),
            "橘猫" => Some("catboss".into()),
            _ => None,
        }, "默认音色");
        assert_eq!(out.iter().map(|d| d.voice.as_str()).collect::<Vec<_>>(), ["linzhi", "catboss"]);
    }

    /// 角色没配音色时回落到全局设置。空串也算没配 ——
    /// 把空串当成音色发给引擎，表现是「换了音色但没变化」，查不出原因。
    #[test]
    fn 没配音色回落到全局_空串也算没配() {
        let lines = vec![line("旁白", "他等的人来了。")];
        assert_eq!(plan(&lines, |_| None, "全局").remove(0).voice, "全局");
        assert_eq!(plan(&lines, |_| Some("   ".into()), "全局").remove(0).voice, "全局");
    }

    /// **台词行的覆盖盖过角色音色**，而且要标出来。
    /// 不标的话「我改了角色音色，这一镜怎么没变」从现象上查不回来。
    #[test]
    fn 台词行覆盖盖过角色音色_并且标出来() {
        let mut l = line("林婉清", "那年我十六。");
        l.voice_override = "shen-teen".into();
        let out = plan(&[l], |_| Some("shen-adult".into()), "全局");
        assert_eq!(out[0].voice, "shen-teen");
        assert!(out[0].overridden, "覆盖了却没标出来");
    }

    /// 没覆盖时 `overridden` 必须是假 —— 恒为真的话界面上每一句都标着
    /// 「已覆盖」，等于没有这个标记。
    #[test]
    fn 没覆盖时不标() {
        let out = plan(&[line("林婉清", "在。")], |_| Some("shen".into()), "全局");
        assert_eq!(out[0].voice, "shen");
        assert!(!out[0].overridden);
    }

    /// 覆盖写成空白不算覆盖，照常用角色的。
    #[test]
    fn 空白覆盖不算覆盖() {
        let mut l = line("林婉清", "在。");
        l.voice_override = "  ".into();
        let out = plan(&[l], |_| Some("shen".into()), "全局");
        assert_eq!(out[0].voice, "shen");
        assert!(!out[0].overridden);
    }

    /// 空台词不合成。合成一段空文本，有的引擎回 0 字节、有的回一声杂音，
    /// 而它会被当成一句正经的话拼进去。
    #[test]
    fn 空台词跳过() {
        let lines = vec![line("甲", "  "), line("乙", "在。")];
        let out = plan(&lines, |_| None, "x");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "在。");
    }

    /// 语气带过来。现在没有引擎收，但数据流要先通 —— 不然接
    /// CosyVoice2 的 instruct 模式时要从镜头表一路改回来。
    #[test]
    fn 语气一路带过来() {
        let mut l = line("小林", "所以满意度是零。");
        l.emotion = "咬牙".into();
        l.voice = "嗓音偏低".into();
        assert_eq!(plan(&[l], |_| None, "x")[0].tone, "嗓音偏低，咬牙");
    }

    /// **每一路都要单独归一。** `-f concat` 只认第一个文件的参数，
    /// 各句采样率不同时后面几句会被按第一句解释（变调或全是杂音）。
    #[test]
    fn 每一路都归一采样率() {
        let f = concat_filter(3, GAP_SECS, 8.0);
        assert_eq!(f.matches("aresample=24000").count(), 3, "少归一了一路：{f}");
        assert_eq!(f.matches("aformat=").count(), 3, "{f}");
        assert!(f.contains("concat=n=3:v=0:a=1"), "{f}");
    }

    /// **末尾必须垫到画面时长。** 不垫的话拼接的 `-shortest` 会把画面截短，
    /// 表现是「这一镜莫名其妙短了半秒」，不报任何错。
    #[test]
    fn 末尾垫到画面时长_只补不截() {
        let f = concat_filter(1, GAP_SECS, 12.5);
        assert!(f.contains("apad=whole_dur=12.5"), "没垫到画面时长：{f}");
        assert!(!f.contains("atrim"), "不该截断 —— 配音超长是要报给用户的事：{f}");
    }

    /// 一句话也要走同一条路。`n=1` 时省掉 concat 会多一个分支，
    /// 而绝大多数镜头正好是一句 —— 分支少的那条反而更少被测到。
    #[test]
    fn 一句话也走同一条路() {
        let f = concat_filter(1, GAP_SECS, 5.0);
        assert!(f.contains("concat=n=1:v=0:a=1"), "{f}");
        assert!(f.contains("[0:a]aresample"), "{f}");
    }

    /// 配音超出画面要报出来 —— 拼接会把尾巴砍掉且不报错。
    #[test]
    fn 配音超长要报出来_小误差不打扰() {
        assert!(overflow(9.8, 8.0).is_some_and(|d| (d - 1.8).abs() < 0.01), "{:?}", overflow(9.8, 8.0));
        assert_eq!(overflow(8.1, 8.0), None, "0.1 秒是对齐误差，不该报");
        assert_eq!(overflow(6.0, 8.0), None);
    }

    /// 拿真 ffmpeg 跑一遍 filter。
    ///
    /// **字符串拼错只在用户点下去那一刻才炸** —— 上面那些断言只查得出
    /// 「字符串里有没有这几个词」，查不出 ffmpeg 认不认。
    /// 两个方向都要验：短了要垫到画面长度，长了**不能**被截掉。
    ///
    /// 没有 ffmpeg 就跳过 —— 让 CI 因为环境缺件而红，比没有这条测试更糟。
    #[test]
    fn 真跑一遍ffmpeg_短了垫长了不截() {
        let ff = crate::cinema::ffmpeg_binary();
        if std::process::Command::new(ff).arg("-version").output().is_err() {
            eprintln!("跳过：本机没有 ffmpeg");
            return;
        }
        let d = std::env::temp_dir().join(format!("anima-dub-test-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();

        let sine = |name: &str, secs: &str, rate: &str| {
            let p = d.join(name);
            let ok = std::process::Command::new(ff)
                .args(["-v", "error", "-y", "-f", "lavfi", "-i", &format!("sine=frequency=440:duration={secs}:sample_rate={rate}")])
                .arg(&p)
                .status()
                .is_ok_and(|s| s.success());
            assert!(ok, "造素材失败");
            p
        };
        let run = |ins: &[std::path::PathBuf], video: f32, out_name: &str| -> f64 {
            let out = d.join(out_name);
            let mut c = std::process::Command::new(ff);
            c.args(["-v", "error", "-y"]);
            for i in ins {
                c.arg("-i").arg(i);
            }
            let o = c
                .arg("-filter_complex")
                .arg(concat_filter(ins.len(), GAP_SECS, video))
                .args(["-map", "[out]", "-ac", "1", "-ar", "24000", "-c:a", "pcm_s16le"])
                .arg(&out)
                .output()
                .unwrap();
            assert!(o.status.success(), "ffmpeg 不认这条 filter：{}", String::from_utf8_lossy(&o.stderr));
            crate::cinema::clip_seconds(&out).expect("出参没时长")
        };

        // 两句、**采样率还不一样**（`-f concat` 正是在这里出错的），总共 2.1s，画面 8s
        let a = sine("a.wav", "1.2", "44100");
        let b = sine("b.wav", "0.9", "16000");
        let padded = run(&[a, b], 8.0, "padded.wav");
        assert!((padded - 8.0).abs() < 0.1, "没垫到画面时长，拼接的 -shortest 会把画面截短：{padded}");

        // 反方向：配音 5s，画面只有 3s —— **不能截**，那是要报给用户的事
        let long = sine("long.wav", "5", "24000");
        let over = run(&[long], 3.0, "over.wav");
        assert!(over > 4.9, "配音被悄悄截断了，台词尾巴会无声消失：{over}");
        assert!(overflow(over as f32, 3.0).is_some(), "超长没被识别出来");

        let _ = std::fs::remove_dir_all(&d);
    }

    /// 取舍必须写在界面上。没有原声时不该吓唬用户。
    #[test]
    fn 取舍两种情况说法不同() {
        assert!(tradeoff_note(true).contains("口型"));
        assert!(!tradeoff_note(false).contains("口型"));
    }
}

/// 一镜配完之后的交代。**每一条都要给用户看**。
#[derive(Debug, Clone, serde::Serialize)]
pub struct DubOutcome {
    pub shot_id: String,
    /// 合成了几句
    pub lines: usize,
    /// 每句用了哪把嗓子：`(说话人, 音色, 是不是逐句覆盖的)`
    pub voices: Vec<(String, String, bool)>,
    /// 成品时长（已经垫到画面时长，所以是 `max(配音, 画面)`）
    pub out_secs: f32,
    pub video_secs: f32,
    /// 配音超出画面多少秒。有值 = **台词尾巴会在拼接时被砍掉**。
    pub overflow_secs: Option<f32>,
    /// 这一镜原来有没有 H3 生成的声音 —— 决定了换配音是不是在牺牲口型。
    pub replaced_source_audio: bool,
}

/// 逐句合成 → 拼接 → 写 `audio/<镜头id>.wav`。
///
/// `video_secs` 给这一镜**实际渲染出来的**视频时长，不是分镜上写的秒数
/// ——两者常差半秒以上，而拼接的 `-shortest` 认的是前者。
///
/// # 每句一次调用，不是整镜一次
///
/// 整镜一次的话一个人会把所有人的台词都念了。旧的
/// `cinema_generate_shot_audio` 就是那样，所以它只在单人镜上看起来正常。
pub async fn dub_shot(
    out_path: &Path,
    shot_id: &str,
    lines: &[DubLine],
    video_secs: f32,
    replaced_source_audio: bool,
    tts: Arc<dyn TextToSpeech>,
    base: &TtsConfig,
) -> Result<DubOutcome> {
    if lines.is_empty() {
        return Err(Error::runtime("这一镜没有台词，没什么可配的"));
    }
    // **临时件放在成品旁边**，不去 /tmp：同一个文件系统，ffmpeg 读写不跨卷；
    // 而且留下的残骸看得见，不会在 /tmp 里攒着谁也不知道。
    let work = out_path.with_file_name(format!(".parts_{shot_id}"));
    let _ = std::fs::remove_dir_all(&work); // 上次没清干净的残骸
    std::fs::create_dir_all(&work).map_err(|e| Error::runtime(format!("建临时目录失败：{e}")))?;
    let _guard = scopeguard(&work);

    let mut parts: Vec<std::path::PathBuf> = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        let mut cfg = base.clone();
        cfg.voice = l.voice.clone();
        let speech = tts.synthesize(&l.text, &cfg).await.map_err(|e| {
            Error::runtime(format!("第 {} 句（{}，音色「{}」）合成失败：{e}", i + 1, l.speaker, l.voice))
        })?;
        if speech.audio.is_empty() {
            return Err(Error::runtime(format!(
                "第 {} 句（{}）合成回了空音频 —— 多半是音色「{}」在当前引擎里不存在",
                i + 1, l.speaker, l.voice
            )));
        }
        // 扩展名随便给，ffmpeg 按内容探测；引擎回 MP3 / PCM / WAV 都有
        let f = work.join(format!("{i:03}.audio"));
        std::fs::write(&f, &speech.audio).map_err(|e| Error::runtime(format!("写临时音频失败：{e}")))?;
        parts.push(f);
    }

    if let Some(parent) = out_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut cmd = std::process::Command::new(crate::cinema::ffmpeg_binary());
    cmd.arg("-y");
    for f in &parts {
        cmd.arg("-i").arg(f);
    }
    cmd.arg("-filter_complex")
        .arg(concat_filter(parts.len(), GAP_SECS, video_secs))
        .args(["-map", "[out]", "-ac", "1", "-ar", "24000", "-c:a", "pcm_s16le"])
        .arg(out_path);
    let out = cmd.output().map_err(|e| Error::runtime(format!("执行 FFmpeg 失败：{e}")))?;
    if !out.status.success() || !out_path.is_file() {
        let err: String = String::from_utf8_lossy(&out.stderr).chars().rev().take(1200).collect::<Vec<_>>().into_iter().rev().collect();
        return Err(Error::runtime(format!("拼接配音失败：{err}")));
    }

    // **成品长度就是 `max(配音, 画面)`** —— `apad=whole_dur` 只补不截。
    // 所以超出多少直接从成品量得出来，不用再单独探每一句。
    let out_secs = crate::cinema::clip_seconds(out_path).map(|d| d as f32).unwrap_or(video_secs);
    Ok(DubOutcome {
        shot_id: shot_id.to_string(),
        lines: lines.len(),
        voices: lines.iter().map(|l| (l.speaker.clone(), l.voice.clone(), l.overridden)).collect(),
        out_secs,
        video_secs,
        overflow_secs: overflow(out_secs, video_secs),
        replaced_source_audio,
    })
}

/// 函数退出时删掉临时目录 —— **包括中途报错退出**。
///
/// 不用 `tempfile`：它只在 dev-dependencies 里，为一个删目录的动作
/// 给正式构建加一个依赖不划算。
fn scopeguard(dir: &Path) -> impl Drop {
    struct G(std::path::PathBuf);
    impl Drop for G {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    G(dir.to_path_buf())
}

// ─────────────────────────── 命令层 ───────────────────────────

/// 配音请求。`shot_id` 为空 = 整集。
#[derive(Debug, serde::Deserialize)]
pub struct DubReq {
    pub project_id: String,
    pub episode_id: String,
    #[serde(default)]
    pub shot_id: Option<String>,
}

/// 一镜 / 整集配音。
///
/// **不改 `episode.json`。** 判据是「`audio/<镜头id>.wav` 在不在」
/// （[`crate::cinema::pick_shot_audio`]），再往 JSON 里存一份状态就是
/// 两个所有者 —— 文件删了而 JSON 还说有，界面就开始说谎。
#[tauri::command]
pub async fn cinema_dub(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
    req: DubReq,
) -> Result<Vec<DubOutcome>> {
    use tauri::Emitter;

    let config_dir = state.config_dir.clone();
    let ep_dir = crate::cinema::cinema_projects_dir(&config_dir)
        .join(&req.project_id)
        .join("episodes")
        .join(&req.episode_id);
    let ep_file = ep_dir.join("episode.json");
    let ep: crate::cinema::EpisodeData =
        serde_json::from_str(&std::fs::read_to_string(&ep_file).unwrap_or_default())
            .map_err(|e| Error::config(format!("读取分集失败：{e}")))?;

    let (selected, base_cfg) = {
        let config = state.gateway.config();
        let config = config.read().await;
        let selected = config
            .tts_engines
            .iter()
            .find(|e| e.id == config.tts_engine)
            .cloned()
            .ok_or_else(|| Error::config(format!("找不到 TTS 引擎「{}」，请在设置里重新选", config.tts_engine)))?;
        let mut c = config.voice.tts.clone();
        c.voice = crate::tts::effective_voice(&selected);
        (selected, c)
    };
    let tts = crate::tts::engine(
        &selected,
        state.http.clone(),
        &crate::tts::voices::voices_dir(&config_dir),
    )?;

    // 说话人 → 角色资产上的音色。**本项目优先**（`RefScope::Project`）：
    // 全局库里可能有同名角色，拿错了是「音色莫名其妙不对」，查不回来。
    let scope = crate::assets::RefScope::Project {
        project_id: &req.project_id,
        episode_id: Some(&req.episode_id),
    };
    let voice_of = |speaker: &str| -> Option<String> {
        crate::assets::find_asset_by_name(&config_dir, crate::assets::AssetKind::Character, speaker, &scope)
            .and_then(|(_, a)| a.character)
            .and_then(|c| c.voice_id)
    };

    let shots: Vec<_> = match req.shot_id.as_deref() {
        Some(id) => ep.storyboard.iter().filter(|s| s.id == id).collect(),
        None => ep.storyboard.iter().collect(),
    };
    if shots.is_empty() {
        return Err(Error::config("找不到要配音的分镜"));
    }

    let shots_dir = ep_dir.join("shots");
    let audio_dir = ep_dir.join("audio");
    let total = shots.len();
    let mut out = Vec::new();
    for (i, shot) in shots.iter().enumerate() {
        let _ = app.emit(
            "anima://cinema-dub-progress",
            serde_json::json!({ "done": i, "total": total, "label": format!("第 {} 镜", shot.shot_num) }),
        );
        let lines = plan(&shot.dialogue_lines, voice_of, &base_cfg.voice);
        if lines.is_empty() {
            continue; // 没台词的镜头跳过，不是错误
        }
        // **画面时长取实际渲染出来的那一段**，不是分镜上写的秒数 ——
        // 两者常差半秒以上，而拼接的 `-shortest` 认的是前者。
        let vid = crate::cinema::stitch_source(shot, &shots_dir);
        let video_secs = vid
            .as_deref()
            .and_then(crate::cinema::clip_seconds)
            .map(|d| d as f32)
            .unwrap_or(shot.duration.max(1.0));
        let had_audio = vid
            .as_deref()
            .map(crate::agent_runtime::video_tool::has_audio_stream)
            .unwrap_or(false);

        let o = dub_shot(
            &audio_dir.join(format!("{}.wav", shot.id)),
            &shot.id,
            &lines,
            video_secs,
            had_audio,
            tts.clone(),
            &base_cfg,
        )
        .await
        .map_err(|e| Error::runtime(format!("第 {} 镜配音失败：{e}", shot.shot_num)))?;
        out.push(o);
    }
    let _ = app.emit(
        "anima://cinema-dub-progress",
        serde_json::json!({ "done": total, "total": total, "label": "完成" }),
    );
    if out.is_empty() {
        return Err(Error::runtime("这些分镜里没有一句台词"));
    }
    Ok(out)
}

/// 删掉配音，**换回 H3 生成的原声**（连带口型就又对上了）。
///
/// 回删掉几个。判据同样是文件在不在 —— 不存状态就没有对不上的可能。
#[tauri::command]
pub fn cinema_clear_dub(state: tauri::State<'_, crate::AppState>, req: DubReq) -> Result<usize> {
    let ep_dir = crate::cinema::cinema_projects_dir(&state.config_dir)
        .join(&req.project_id)
        .join("episodes")
        .join(&req.episode_id);
    let audio_dir = ep_dir.join("audio");
    let ep_file = ep_dir.join("episode.json");
    let ep: crate::cinema::EpisodeData =
        serde_json::from_str(&std::fs::read_to_string(&ep_file).unwrap_or_default())
            .map_err(|e| Error::config(format!("读取分集失败：{e}")))?;

    let mut n = 0usize;
    for shot in &ep.storyboard {
        if req.shot_id.as_deref().is_some_and(|id| id != shot.id) {
            continue;
        }
        let f = audio_dir.join(format!("{}.wav", shot.id));
        if f.is_file() && std::fs::remove_file(&f).is_ok() {
            n += 1;
        }
    }
    Ok(n)
}

/// 哪些镜头已经配过音。给界面标状态用 —— 判据还是文件在不在。
#[tauri::command]
pub fn cinema_dubbed_shots(state: tauri::State<'_, crate::AppState>, req: DubReq) -> Vec<String> {
    let audio_dir = crate::cinema::cinema_projects_dir(&state.config_dir)
        .join(&req.project_id)
        .join("episodes")
        .join(&req.episode_id)
        .join("audio");
    std::fs::read_dir(&audio_dir)
        .map(|it| {
            it.flatten()
                .filter_map(|e| {
                    let p = e.path();
                    (p.extension().and_then(|x| x.to_str()) == Some("wav"))
                        .then(|| p.file_stem()?.to_str().map(str::to_string))
                        .flatten()
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 当前 TTS 引擎能用的音色。
#[derive(Debug, Clone, serde::Serialize)]
pub struct VoiceTemplates {
    /// 当前引擎的显示名。**必须说清「这是谁的音色」** ——
    /// 音色名的命名空间每家一套（Edge 是 `zh-CN-XiaoxiaoNeural`，
    /// Kokoro 是 `zf_xiaobei`，GPT-SoVITS 是音色库里的 `yaemiko/d`），
    /// 不说是哪个引擎的，这份清单等于没有出处。
    pub engine: String,
    pub voices: Vec<String>,
    /// 列表为空时的**原因 + 怎么办**。有列表时为空串。
    pub hint: String,
}

/// 可选的音色模板。**问引擎自己要**，不是只看共享音色库。
///
/// # 2026-09-18 之前这里是错的
///
/// 原来只有 GPT-SoVITS 回非空，别的一律回空列表、界面写「填引擎认的音色名」。
/// 用户的原话：「音色这里让我填写音色名，我哪知道音色名是啥」—— 他说得对，
/// 那个提示等于什么都没说。
///
/// 而 `TextToSpeech::voices()` **每个引擎都实现了**：Edge 去微软拉完整目录，
/// OpenAI 兼容的试 `/v1/audio/voices`（Kokoro 有），GPT-SoVITS 回共享音色库。
/// 真正回空的只有 CosyVoice2 / IndexTTS2 —— 它们的音色**就是参考音频**，
/// 没有名字这回事。那种情况要明说，而不是让人对着空框猜。
#[tauri::command]
pub async fn cinema_voice_templates(state: tauri::State<'_, crate::AppState>) -> Result<VoiceTemplates> {
    let selected = {
        let config = state.gateway.config();
        let config = config.read().await;
        config.tts_engines.iter().find(|e| e.id == config.tts_engine).cloned()
    };
    let Some(selected) = selected else {
        return Ok(VoiceTemplates {
            engine: String::new(),
            voices: Vec::new(),
            hint: "设置里还没选 TTS 引擎。".into(),
        });
    };
    let engine_label = selected.label.clone();
    let kind = selected.kind;
    let tts = crate::tts::engine(&selected, state.http.clone(), &crate::tts::voices::voices_dir(&state.config_dir))?;

    // **拉不到不等于没有。** 服务没起来和「这个引擎本来就没有音色名」
    // 是两件事，混成一句话的话用户会去改错的东西。
    let voices = match tts.voices().await {
        Ok(v) => v.into_iter().map(|v| if v.short_name.is_empty() { v.name } else { v.short_name }).collect::<Vec<_>>(),
        Err(e) => {
            return Ok(VoiceTemplates {
                engine: engine_label,
                voices: Vec::new(),
                hint: format!("拉音色列表失败：{e}。服务没起来的话先去「服务」页看一眼。"),
            })
        }
    };
    let hint = if !voices.is_empty() {
        String::new()
    } else if crate::tts::uses_voice_library(kind) {
        format!(
            "音色库是空的。把参考音频（和同名的 .txt / .lab 转写稿）放进 {} 下面，一个人一个子目录。",
            crate::tts::voices::voices_dir(&state.config_dir).display()
        )
    } else {
        "这个引擎靠参考音频决定音色，没有音色名可填 —— 给不同角色分配不同嗓子，请在设置里换成 GPT-SoVITS 或 Edge。".into()
    };
    Ok(VoiceTemplates { engine: engine_label, voices, hint })
}
