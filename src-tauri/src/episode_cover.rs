//! **分集**封面：和 [`crate::cover`] 同一条路 —— 文字交给 Z-Image 画进画面。
//!
//! # 和 `cover.rs` 的分工（别混）
//!
//! | | [`crate::cover`] | 这里 |
//! |---|---|---|
//! | 作用域 | **项目**封面，一个项目一张 | **分集**封面，一集一张 |
//! | 入口 | 项目卡「⋮」 | 组装页 |
//! | 文字 | 一个片名 | **三行带层级**：系列名小、剧名大、集号 |
//! | 提示词来源 | 后台模型读项目信息写 | 本集梗概 + 固定版式句 |
//! | 产物 | 写回 `project.json` 的 `cover_image` | 写回 `episode.json` 的 `cover_image` |
//!
//! # 为什么文字是模型画的，不是代码画的
//!
//! 这里**返过一次工**，值得写下来：第一版用代码画字（`imageproc` 描字形 +
//! 底部压暗层），出来是白字压在一条硬边浅灰块上、系列名骑在压暗层边缘、
//! 而且模型自己还在旁边画了一堆乱码汉字 —— 三样叠在一起很难看。
//!
//! 而 `cover.rs` 让 Z-Image 直接画片名，出来是漂亮的书法大字。
//! CLAUDE.md 那张表早就写着 Z-Image 画中文 **6/6**，第一版是我没信那张表。
//!
//! 代价是「八集版式完全一致」保证不了（模型每次的字体位置会有出入）。
//! 用户看过两种效果之后选了好看那个 —— **这是已经拍过板的取舍，别再翻回去。**
//!
//! # 这条链路的静默失败
//!
//! 出图引擎**必须是 Z-Image**：Krea 2 画汉字是 4/6、Klein 是 0/12
//! （CLAUDE.md 那张表）。换成别的引擎不会报错，只会出一张画着乱码的封面。

use crate::error::{Error, Result};

/// 三行文字的**固定版式句**。系列名小、剧名大、集号在下 —— 这就是「固定格式」。
///
/// 单独拆出来是为了能测：版式写没写对不该靠出一张图去看。
/// 空的那行**不写进去** —— 否则会出现「小字写着「」」这种句子，
/// 模型多半会照着画一对空引号。
pub fn text_spec(series: &str, title: &str, episode: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !series.trim().is_empty() {
        parts.push(format!("画面最上方一行小字写着「{}」", series.trim()));
    }
    if !title.trim().is_empty() {
        parts.push(format!("下面用醒目的中文书法大字写着「{}」", title.trim()));
    }
    if !episode.trim().is_empty() {
        parts.push(format!("大字下方一行小字写着「{}」", episode.trim()));
    }
    if parts.is_empty() {
        return "画面里不要出现任何文字、水印或标志。".into();
    }
    format!("{}。除了这{}行字，画面里不要出现其他任何文字、水印或标志。", parts.join("，"), parts.len())
}

/// 从一集的信息拼出封面提示词。
///
/// 顺序是**风格 → 画面内容 → 构图 → 文字**，不能乱：
/// 风格词必须排在构图词之前，否则会被后面一串摄影术语淹掉
/// （记忆 `style-must-precede-composition`，出过事）。
pub fn prompt_from_episode(series: &str, title: &str, episode: &str, synopsis: &str, style: &str) -> String {
    let mut p = String::new();
    if !style.trim().is_empty() {
        p.push_str(style.trim());
        p.push_str("，");
    }
    let gist = synopsis.trim();
    if !gist.is_empty() {
        p.push_str(gist);
    } else if !title.trim().is_empty() {
        p.push_str(title.trim());
    }
    p.push_str("。电影海报构图，主体位于画面中下部，上方留出干净的空间放标题，戏剧性的光影。");
    p.push_str(&text_spec(series, title, episode));
    p
}

/// 从分集标题拆出「集号」和「剧名」。
///
/// 用户定的格式是 `E01-《猫的年终述职》`（记忆 `lifu-series-style-direction`）。
/// 拆开是因为两行字号不同 —— 集号小、剧名大。
///
/// **拆不出来就整个当剧名**，不猜：宁可少一行，也不要把剧名切坏。
pub fn split_episode_title(raw: &str) -> (String, String) {
    let t = raw.trim();
    // `E01-《…》` / `E01 《…》` / `E01-…`
    let (ep, rest) = match t.find(['-', '－', ' ']) {
        Some(i) => {
            let head = t[..i].trim();
            let looks_like_ep = head.len() >= 2
                && head.chars().next().is_some_and(|c| c.eq_ignore_ascii_case(&'E'))
                && head[1..].chars().all(|c| c.is_ascii_digit());
            if looks_like_ep {
                (head.to_string(), t[i + t[i..].chars().next().map_or(1, char::len_utf8)..].trim())
            } else {
                (String::new(), t)
            }
        }
        None => (String::new(), t),
    };
    (ep, strip_book_quotes(rest))
}

/// 去掉**整串两端成对的**书名号。
///
/// 不能用 `trim_matches(|c| c == '《' || c == '》')` —— 它从两端反复剥，
/// `Epic-《史诗》` 会被剥成 `Epic-《史诗`（尾巴掉了、中间那个还在）。
/// 只有整串被一对包住时才剥，是唯一不会切坏标题的做法。
fn strip_book_quotes(s: &str) -> String {
    let t = s.trim();
    match (t.strip_prefix('《'), t.ends_with('》')) {
        (Some(inner), true) => inner[..inner.len() - '》'.len_utf8()].trim().to_string(),
        _ => t.to_string(),
    }
}

/// 生成一集的封面，回封面的本机路径。
pub async fn generate_episode_cover(
    config_dir: &std::path::Path,
    project_id: &str,
    episode_id: &str,
) -> Result<std::path::PathBuf> {
    let proj_dir = crate::cinema::cinema_projects_dir(config_dir).join(project_id);
    let ep_dir = proj_dir.join("episodes").join(episode_id);
    if !ep_dir.join("episode.json").is_file() {
        return Err(Error::config("这一集不在了".to_string()));
    }

    let read_json = |p: std::path::PathBuf| -> serde_json::Value {
        std::fs::read_to_string(p).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(serde_json::Value::Null)
    };
    let project = read_json(proj_dir.join("project.json"));
    let episode = read_json(ep_dir.join("episode.json"));
    let g = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();

    let series = strip_book_quotes(&g(&project, "title"));
    let (ep_no, title) = split_episode_title(&g(&episode, "title"));
    let style = crate::style::load_art(config_dir, project_id).effective_positive();
    let prompt = prompt_from_episode(&series, &title, &ep_no, &g(&episode, "synopsis"), &style);
    let (w, h) = crate::cover::cover_size(&g(&project, "aspect_ratio"));

    // **引擎写死 zimage**：只有它能把中文一字不差画进画面（CLAUDE.md 那张表 6/6）。
    let args: crate::studio::GenerateArgs = serde_json::from_value(serde_json::json!({
        "mode": "t2i", "engine": "zimage", "prompt": prompt,
        "width": w, "height": h, "auto_loras": false
    }))
    .map_err(|e| Error::runtime(format!("出图参数写错了：{e}")))?;
    let out = crate::studio::generate_with(config_dir, args).await?;
    let src = out.path.filter(|_| out.ok).ok_or_else(|| {
        Error::runtime(if out.message.trim().is_empty() { "封面没出来".into() } else { out.message.clone() })
    })?;

    // **故意不写回 `episode.json`。**
    //
    // `EpisodeData` 没有 `cover_image` 这个字段，往那份 JSON 里塞一个
    // 额外的键**下一次 `cinema_save_episode` 就会把它抹掉** —— 改一个镜头
    // 封面指针就没了，而且不报错。封面在不在，只认 `cover.png` 这一个事实。
    let dst = ep_dir.join("cover.png");
    std::fs::copy(&src, &dst).map_err(|e| Error::runtime(format!("封面存不进本集目录：{e}")))?;
    let _ = episode;
    Ok(dst)
}

/// 片头封面停留几秒。**改这里就等于改片头长度。**
///
/// 2.5 秒是「看清三行字」和「别让人等」之间的折中：一行大字约 0.6 秒读完，
/// 三行 + 一点呼吸 ≈ 2.5。
pub const COVER_INTRO_SECS: f64 = 2.5;

/// 这一集的封面文件。没有就是 `None` —— 勾了「加封面」但还没生成时不能崩。
pub fn cover_file(ep_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let p = ep_dir.join("cover.png");
    p.is_file().then_some(p)
}

/// 成片有没有加过片头。**判据是母版在不在** —— 加片头时原成片被改名存成
/// `final_nocover.mp4`，它存在就说明 `final.mp4` 是带片头的那版。
///
/// 不另外记一个标记位：那就又是「一份事实存两处」，而这两处一定会错开。
pub fn has_cover_intro(ep_dir: &std::path::Path) -> bool {
    ep_dir.join(MASTER).is_file()
}

/// 片头给字幕带来的偏移（秒）。没加片头就是 0。
///
/// **字幕是按镜头时长重算的，不知道片头存在** —— 不加这个偏移，
/// 整集字幕会早 [`COVER_INTRO_SECS`] 秒，而且哪里都不报错。
pub fn cover_intro_offset(ep_dir: &std::path::Path) -> f64 {
    if has_cover_intro(ep_dir) { COVER_INTRO_SECS } else { 0.0 }
}

/// 片头那一小段片子，做出来就留着 —— 导出到剪映时时间轴上要放同一段。
pub fn intro_clip(ep_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let p = ep_dir.join("cover_intro.mp4");
    p.is_file().then_some(p)
}

/// 不带片头的成片母版。加片头 = 从母版重做，所以按几次都一样。
pub const MASTER: &str = "final_nocover.mp4";

/// 重新合成之后要把母版清掉 —— 新的 `final.mp4` 就是新的母版，
/// 留着旧的会让 [`has_cover_intro`] 说谎。
pub fn clear_cover_intro(ep_dir: &std::path::Path) {
    let _ = std::fs::remove_file(ep_dir.join(MASTER));
}

/// 一段片子的流参数。**片头必须逐项对齐成片的这几项**，原因见 [`build_cover_intro`]。
#[derive(Debug, Clone, PartialEq)]
pub struct ClipParams {
    pub width: u32,
    pub height: u32,
    /// 帧率，形如 `24/1`
    pub fps: String,
    pub sample_rate: u32,
    pub channels: u32,
}

impl Default for ClipParams {
    fn default() -> Self {
        Self { width: 1080, height: 1920, fps: "24/1".into(), sample_rate: 32000, channels: 2 }
    }
}

/// 声道布局名。ffmpeg 的 `anullsrc` 要的是名字，不是数字。
pub fn channel_layout(channels: u32) -> &'static str {
    if channels >= 2 { "stereo" } else { "mono" }
}

/// 读一段片子的流参数。读不出来的项用默认值补 —— 让不出片比出个坏片好。
pub fn probe_params(path: &std::path::Path) -> ClipParams {
    let run = |args: &[&str]| -> String {
        std::process::Command::new(crate::agent_runtime::video_tool::ffprobe_binary())
            .args(["-v", "error"])
            .args(args)
            .arg(path)
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    };
    let v = run(&["-select_streams", "v:0", "-show_entries", "stream=width,height,r_frame_rate", "-of", "csv=p=0"]);
    let a = run(&["-select_streams", "a:0", "-show_entries", "stream=sample_rate,channels", "-of", "csv=p=0"]);
    let d = ClipParams::default();
    let vp: Vec<&str> = v.split(',').collect();
    let ap: Vec<&str> = a.split(',').collect();
    let num = |s: Option<&&str>, def: u32| s.and_then(|x| x.trim().parse().ok()).filter(|n| *n > 0).unwrap_or(def);
    ClipParams {
        width: num(vp.first(), d.width),
        height: num(vp.get(1), d.height),
        fps: vp.get(2).map(|s| s.trim()).filter(|s| s.contains('/') && !s.starts_with("0/")).unwrap_or("24/1").to_string(),
        sample_rate: num(ap.first(), d.sample_rate),
        channels: num(ap.get(1), d.channels),
    }
}

/// 把封面图做成一段**带静音音轨**的片子，接在成片最前面。
///
/// # 参数必须逐项对齐成片，这是正确性不是讲究
///
/// **concat 只按第一个文件的流参数解后面所有文件。** 片头排在最前面，
/// 所以片头的参数就是整片的解码参数 —— 对不上的后果 2026-09-18 实测过：
/// 片头 24000Hz 单声道、成片 32000Hz 立体声，整条成片音轨解不出来，
/// ffmpeg 刷屏 `Error submitting packet to decoder` 然后 `Conversion failed!`。
///
/// 更糟的是**把两个文件的顺序倒过来就"成功"了**，只是一堆 DTS 警告 ——
/// 所以这个坑只在「片头在前」这一种排法下露头，随手试一下是试不出来的。
///
/// 另外两件同样写错也不报错的：**必须有音轨**（没有的话音画从第二段起整体
/// 错位）、**`setsar=1`**（像素宽高比不统一会被拉扁）。
pub fn build_cover_intro(ffmpeg: &str, cover: &std::path::Path, out: &std::path::Path, p: &ClipParams) -> Result<()> {
    let secs = COVER_INTRO_SECS.to_string();
    let (w, h) = (p.width, p.height);
    let vf = format!(
        "scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1,fps={}",
        p.fps
    );
    let silence = format!("anullsrc=channel_layout={}:sample_rate={}", channel_layout(p.channels), p.sample_rate);
    let o = std::process::Command::new(ffmpeg)
        .args(["-y", "-loop", "1", "-t", &secs, "-i", &cover.to_string_lossy()])
        .args(["-f", "lavfi", "-t", &secs, "-i", &silence])
        .args(["-vf", &vf])
        .args(["-c:v", "libx264", "-crf", "18", "-pix_fmt", "yuv420p"])
        .args(["-c:a", "aac", "-ar", &p.sample_rate.to_string(), "-ac", &p.channels.to_string(), "-shortest"])
        .args(["-movflags", "+faststart", &out.to_string_lossy()])
        .output()
        .map_err(|e| Error::runtime(format!("执行 FFmpeg 做片头失败：{e}")))?;
    if !o.status.success() || !out.is_file() {
        return Err(Error::runtime(format!("片头封面没做成：{}", tail_err(&o.stderr))));
    }
    Ok(())
}

/// 片头和成片的参数对不对得上 —— concat 之前必须成立。
///
/// **帧率不比**：视频帧率不一致 concat 会重新算时间戳，结果是对的。
/// 出事的是**音频参数**和**画幅**：前者让整条音轨解不出来，后者被拉扁。
pub fn params_match(master: &ClipParams, intro: &ClipParams) -> bool {
    master.width == intro.width
        && master.height == intro.height
        && master.sample_rate == intro.sample_rate
        && master.channels == intro.channels
}

/// ffmpeg 的报错要取**末尾**。前面全是编码器统计，真正的原因在最后几行。
fn tail_err(stderr: &[u8]) -> String {
    let s = String::from_utf8_lossy(stderr);
    s.lines()
        .filter(|l| {
            let l = l.trim();
            !l.is_empty() && !l.starts_with("[libx264") && !l.starts_with("frame=") && !l.starts_with("[aac @")
        })
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join(" / ")
}

/// 给成片加 / 撤片头封面。**不重新渲染任何镜头** ——
/// 已经合成好、甚至已经超分过的成片也能直接加。
///
/// # 为什么是后处理，不是拼接时插进去
///
/// 第一版是在 `stitch_episode` 的片段列表最前面插一段。两个问题：
///
/// 1. **已经合成好的片子加不了封面**，只能整集重拼。
/// 2. **字幕会早 2.5 秒**：`export_srt` 是按镜头时长重算的，
///    它看不见拼接过程里多出来的那一段，而且不报错。
///
/// 改成后处理之后两个都没了：母版 `final_nocover.mp4` 在不在，
/// 既是「加没加过」的唯一事实，也是字幕偏移的依据。
pub async fn set_cover_intro(
    config_dir: &std::path::Path,
    project_id: &str,
    episode_id: &str,
    on: bool,
) -> Result<()> {
    let ep_dir = crate::cinema::cinema_projects_dir(config_dir)
        .join(project_id)
        .join("episodes")
        .join(episode_id);
    let final_mp4 = ep_dir.join("final.mp4");
    let master = ep_dir.join(MASTER);

    if !on {
        // 撤：母版换回去。没有母版说明本来就没加过，不算错。
        if master.is_file() {
            std::fs::rename(&master, &final_mp4).map_err(|e| Error::runtime(format!("撤片头失败：{e}")))?;
        }
        return Ok(());
    }

    let cover = cover_file(&ep_dir).ok_or_else(|| Error::config("这一集还没有封面 —— 先点「生成封面」"))?;
    // **先从母版复原**，再重做 —— 按第二次不会叠出两个片头
    if master.is_file() {
        std::fs::rename(&master, &final_mp4).map_err(|e| Error::runtime(format!("读不到成片母版：{e}")))?;
    }
    if !final_mp4.is_file() {
        return Err(Error::config("这一集还没有成片 —— 先点「合成一集」"));
    }
    // **按成片的参数做片头** —— concat 拿第一个文件的参数解后面所有文件
    let params = probe_params(&final_mp4);
    let ffmpeg = crate::cinema::ffmpeg_binary();
    let intro = ep_dir.join("cover_intro.mp4");
    build_cover_intro(ffmpeg, &cover, &intro, &params)?;
    // **做完要回头核一遍。** 参数对不上时 concat 的失败方式取决于排法：
    // 片头在前会直接失败，倒过来只警告 —— 也就是说它可能出一个音轨是坏的成片
    // 而任务显示成功。宁可在这里停住。
    let got = probe_params(&intro);
    if !params_match(&params, &got) {
        return Err(Error::runtime(format!(
            "片头的流参数和成片对不上，接上去音轨会坏：成片 {params:?} / 片头 {got:?}"
        )));
    }

    std::fs::rename(&final_mp4, &master).map_err(|e| Error::runtime(format!("存不下成片母版：{e}")))?;
    let list = ep_dir.join("cover_concat.txt");
    std::fs::write(&list, format!("file '{}'\nfile '{}'\n", intro.display(), master.display()))
        .map_err(|e| Error::runtime(format!("写不下拼接清单：{e}")))?;
    let o = std::process::Command::new(ffmpeg)
        .args(["-y", "-f", "concat", "-safe", "0", "-i", &list.to_string_lossy()])
        .args(["-c:v", "libx264", "-crf", "18", "-pix_fmt", "yuv420p"])
        .args(["-c:a", "aac", "-ar", &params.sample_rate.to_string(), "-ac", &params.channels.to_string()])
        .args(["-movflags", "+faststart", &final_mp4.to_string_lossy()])
        .output()
        .map_err(|e| Error::runtime(format!("执行 FFmpeg 接片头失败：{e}")))?;
    let _ = std::fs::remove_file(&list);
    if !o.status.success() || !final_mp4.is_file() {
        // 接不上就把母版还回去，不能留下一个没有成片的分集
        let _ = std::fs::rename(&master, &final_mp4);
        return Err(Error::runtime(format!("片头没接上：{}", tail_err(&o.stderr))));
    }
    Ok(())
}

#[tauri::command]
pub async fn cinema_set_cover_intro(
    state: tauri::State<'_, crate::AppState>,
    project_id: String,
    episode_id: String,
    on: bool,
) -> Result<()> {
    set_cover_intro(&state.config_dir, &project_id, &episode_id, on).await
}

/// 成片现在是不是带片头的那版。界面进来时问一次。
#[tauri::command]
pub fn cinema_has_cover_intro(
    state: tauri::State<'_, crate::AppState>,
    project_id: String,
    episode_id: String,
) -> bool {
    has_cover_intro(
        &crate::cinema::cinema_projects_dir(&state.config_dir).join(project_id).join("episodes").join(episode_id),
    )
}

/// 这一集有没有封面。有就回本机路径 —— **界面问的是磁盘，不是 JSON**。
#[tauri::command]
pub fn cinema_episode_cover(
    state: tauri::State<'_, crate::AppState>,
    project_id: String,
    episode_id: String,
) -> Option<String> {
    let ep_dir = crate::cinema::cinema_projects_dir(&state.config_dir)
        .join(project_id)
        .join("episodes")
        .join(episode_id);
    cover_file(&ep_dir).map(|p| p.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn cinema_generate_episode_cover(
    state: tauri::State<'_, crate::AppState>,
    project_id: String,
    episode_id: String,
) -> Result<String> {
    generate_episode_cover(&state.config_dir, &project_id, &episode_id)
        .await
        .map(|p| p.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **固定格式**：系列名小、剧名大、集号在下 —— 用户明确要求的那条。
    #[test]
    fn 版式句_系列名小剧名大集号在下() {
        let s = text_spec("每日小离谱", "猫的年终述职", "E01");
        assert!(s.contains("小字写着「每日小离谱」"), "系列名没写成小字：{s}");
        assert!(s.contains("大字写着「猫的年终述职」"), "剧名没写成大字：{s}");
        assert!(s.contains("小字写着「E01」"), "集号没写上：{s}");
        // 次序：系列名 → 剧名 → 集号
        let at = |k: &str| s.find(k).unwrap();
        assert!(at("每日小离谱") < at("猫的年终述职"));
        assert!(at("猫的年终述职") < at("E01"));
    }

    /// 空的那行**不能写成空引号** —— 模型会照着画一对空「」出来。
    #[test]
    fn 空的那行不写进版式句() {
        let s = text_spec("", "猫的年终述职", "");
        assert!(!s.contains("「」"), "出现了空引号：{s}");
        assert!(!s.contains("最上方一行小字") && !s.contains("大字下方"), "空的行还是写进去了：{s}");
        assert!(s.contains("这1行字"), "行数没跟着变：{s}");
        assert_eq!(text_spec("", "", ""), "画面里不要出现任何文字、水印或标志。");
    }

    /// 除了这三行，**不许有别的字** —— 不写的话模型会在旁边补一堆乱码汉字。
    #[test]
    fn 版式句禁止其他文字() {
        for s in [text_spec("系列", "标题", "E01"), text_spec("", "", "")] {
            assert!(s.contains("不要出现"), "没禁止多余文字：{s}");
        }
    }

    /// **风格词必须排在构图词之前**，否则会被后面一串摄影术语淹掉（出过事）。
    #[test]
    fn 风格排在构图之前() {
        let p = prompt_from_episode("每日小离谱", "猫的年终述职", "E01", "橘猫做年终述职", "BJD 人偶国漫");
        let at = |k: &str| p.find(k).unwrap();
        assert!(at("BJD 人偶国漫") < at("海报构图"), "风格被排到构图后面了：{p}");
        assert!(at("橘猫") < at("海报构图"), "剧情也该在构图之前：{p}");
        // 文字在最后
        assert!(at("海报构图") < at("小字写着"), "版式句该在构图之后：{p}");
    }

    /// 上方要留空 —— 大字要落在干净的地方，压在脸上就白搭了。
    #[test]
    fn 构图要求上方留空() {
        let p = prompt_from_episode("系列", "标题", "E01", "梗概", "");
        assert!(p.contains("上方留出干净的空间"), "没给标题留地方：{p}");
    }

    /// 没有梗概时退回标题，不能给一句只有风格的提示词。
    #[test]
    fn 没有梗概时用标题兜底() {
        let p = prompt_from_episode("系列", "猫的年终述职", "E01", "", "");
        assert!(p.starts_with("猫的年终述职"));
    }

    /// **集号和剧名要拆开** —— 两行字号不同，不拆就只能整行一个字号。
    #[test]
    fn 拆得出集号和剧名() {
        assert_eq!(split_episode_title("E01-《猫的年终述职》"), ("E01".into(), "猫的年终述职".into()));
        assert_eq!(split_episode_title("E12-《健身卡的独白》"), ("E12".into(), "健身卡的独白".into()));
        assert_eq!(split_episode_title("E03 《阎王的KPI》"), ("E03".into(), "阎王的KPI".into()));
        assert_eq!(split_episode_title("E05-AI替我相亲"), ("E05".into(), "AI替我相亲".into()));
    }

    /// **拆不出来就整个当剧名** —— 宁可少一行，也别把剧名切坏。
    #[test]
    fn 拆不出来时不乱切() {
        assert_eq!(split_episode_title("《没有集号的片子》"), ("".into(), "没有集号的片子".into()));
        assert_eq!(split_episode_title("第一集-开场"), ("".into(), "第一集-开场".into()));
        assert_eq!(split_episode_title("猫的年终述职"), ("".into(), "猫的年终述职".into()));
        // 「E」开头但后面不是纯数字的，不能误判成集号
        assert_eq!(split_episode_title("Epic-《史诗》"), ("".into(), "Epic-《史诗》".into()));
    }

    /// 没有封面时回 `None`，不能拿一个不存在的路径往下走。
    #[test]
    fn 没有封面文件时回空() {
        let d = tempfile::tempdir().unwrap();
        assert!(cover_file(d.path()).is_none());
        std::fs::write(d.path().join("cover.png"), b"x").unwrap();
        assert!(cover_file(d.path()).is_some());
    }

    /// 片头别太长也别太短 —— 三行字要看得清，又不能让人等。
    #[test]
    fn 片头时长在合理区间() {
        assert!((1.0..=6.0).contains(&COVER_INTRO_SECS), "片头时长不合理：{COVER_INTRO_SECS}");
    }

    /// 母版在不在 = 加没加过片头，**没有第二个标记位**。
    #[test]
    fn 片头状态只认母版() {
        let d = tempfile::tempdir().unwrap();
        assert!(!has_cover_intro(d.path()));
        assert_eq!(cover_intro_offset(d.path()), 0.0);
        std::fs::write(d.path().join(MASTER), b"x").unwrap();
        assert!(has_cover_intro(d.path()));
        assert_eq!(cover_intro_offset(d.path()), COVER_INTRO_SECS);
        clear_cover_intro(d.path());
        assert!(!has_cover_intro(d.path()), "重新合成之后母版必须清掉，否则字幕会平白多偏移");
    }

    /// **声道数要翻成布局名** —— `anullsrc` 要的是 `stereo`/`mono`，不是数字。
    #[test]
    fn 声道布局名() {
        assert_eq!(channel_layout(1), "mono");
        assert_eq!(channel_layout(2), "stereo");
        assert_eq!(channel_layout(0), "mono");
    }

    /// 读不出来的项要有默认值 —— 读不出就不出片，比出个音轨坏掉的片好。
    #[test]
    fn 读不出参数时用默认值() {
        let p = probe_params(std::path::Path::new("/nope/没有这个文件.mp4"));
        assert_eq!(p, ClipParams::default());
        assert!(p.sample_rate > 0 && p.channels > 0 && p.fps.contains('/'));
    }

    /// ffmpeg 的原因在**末尾**，前面全是编码器统计。取错方向就只能看到一堆百分比。
    #[test]
    fn 报错取末尾且滤掉编码器统计() {
        let raw = b"[libx264 @ 0x1] i16 v,h,dc,p: 29% 22%\n[aac @ 0x2] Qavg: 65536\nError submitting packet to decoder\nConversion failed!\n";
        let e = tail_err(raw);
        assert!(e.contains("Conversion failed!"), "丢了真正的原因：{e}");
        assert!(!e.contains("libx264"), "编码器统计没滤掉：{e}");
    }

    /// 音频参数和画幅**必须**一致（否则音轨解不出来 / 画面被拉扁），
    /// 帧率不一致没关系（concat 会重算时间戳）。
    #[test]
    fn 参数比对_比音频和画幅_不比帧率() {
        let m = ClipParams::default();
        assert!(params_match(&m, &m));
        assert!(params_match(&m, &ClipParams { fps: "25/1".into(), ..m.clone() }), "帧率不该拦");
        for bad in [
            ClipParams { sample_rate: 24000, ..m.clone() },
            ClipParams { channels: 1, ..m.clone() },
            ClipParams { width: 576, ..m.clone() },
            ClipParams { height: 1024, ..m.clone() },
        ] {
            assert!(!params_match(&m, &bad), "这个该拦下来：{bad:?}");
        }
    }

    /// 空标题不能崩。
    #[test]
    fn 空标题不崩() {
        assert_eq!(split_episode_title(""), ("".into(), "".into()));
        assert_eq!(split_episode_title("   "), ("".into(), "".into()));
    }
}
