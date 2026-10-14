//! 一集导出成 Final Cut Pro 能直接打开的时间线（FCPXML 1.11）。
//!
//! # 为什么不是剪映那条
//!
//! `cinema_export_jianying` 按猜出来的格式写剪映的私有草稿 `draft_content.json`。
//! 剪映不公开这个格式、新版还加了密，外部写的草稿打不开也不报错 ——
//! 用户 2026-09-29：「那个功能是废的，试过根本不能用」。
//!
//! FCPXML 是苹果公开的交换格式，**FCP 装包里自带 DTD**
//! （`Interchange.framework/Versions/A/Resources/FCPXMLv1_11.dtd`）。
//! 所以导出结果每次都在本机用 `xmllint` 对着它校验过才交出去，不是「写完祈祷」。
//! 1.11 是 FCP 10.6 起都认的版本；本机的 FCP 带 1.11~1.14。
//!
//! # 只写事件和项目，不写资源库
//!
//! 用户自己建资源库和事件（《回收师》资源库、`02_S01E01_第三次` 这种事件名）。
//! 这里不带 `<library>`，导入时 FCP 问放进哪个资源库；事件名由用户填。
//!
//! # 媒体一律拷到用户选的文件夹
//!
//! 出片文件在 `~/Library/Application Support/…` 深处，文件名是 `shot_<uuid>-b-<时间戳>`，
//! 而且**重出一镜会换文件**。直接引用的话，FCP 里的片段迟早离线。
//! 所以拷到用户选的位置（可以是外置硬盘），文件名按镜号 + 内容指纹：
//! 同一条片子重导不重拷；重出过的镜头是新文件，老文件不动 —— 之前导进 FCP 的项目不会掉线。
//!
//! 每一镜用哪个文件、配音怎么盖原声，和「合成一集」是同一套规则（`stitch_source` / `pick_shot_audio`），
//! 只是最后不拼接，交给 FCP 剪。

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// 时间线帧率。出片就是 24 帧（`cinema_export_jianying` 那边也按这个写）
pub const FPS: u64 = 24;

/// 字幕的角色。ITT 是 FCP 新建字幕时的默认格式
const CAPTION_ROLE: &str = "iTT?captionFormat=ITT.zh-Hans";

#[derive(Debug, Clone, PartialEq)]
pub struct AudioInfo {
    pub channels: u32,
    pub rate: u32,
}

/// 一条视频文件（已经拷到导出目录里的那份）
#[derive(Debug, Clone)]
pub struct VideoMedia {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    /// 帧率 `(分子, 分母)`，24 帧就是 `(24, 1)`
    pub fps: (u64, u64),
    pub frames: u64,
    /// 没有音轨是 `None`（出片设置里关了 H3 原声）
    pub audio: Option<AudioInfo>,
}

/// 一条音频文件（配音、配乐）
#[derive(Debug, Clone)]
pub struct AudioMedia {
    pub path: PathBuf,
    pub secs: f64,
    pub info: AudioInfo,
}

/// 一句字幕。时间相对**这一镜**的开头
#[derive(Debug, Clone)]
pub struct Caption {
    pub text: String,
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone)]
pub enum Item {
    Clip {
        name: String,
        video: VideoMedia,
        /// TTS 配音。有它的时候片子自己的声音静音 —— 和「合成一集」一样是**替换**，不是叠加
        dub: Option<AudioMedia>,
        captions: Vec<Caption>,
        /// 打斗冲击点（片段内秒数, 标记文字），后期在这里加黑白冲击帧 / 闪白 / 震屏 / 冲击音。
        /// 从攻防交换算（`combat::contact_points`），没填交换的镜头是空的
        cues: Vec<(f64, String)>,
    },
    /// 还没出片的镜头：按分镜秒数留空，带一个待办标记。
    /// 不留的话后面每一镜都往前错位，时间线和分镜对不上。
    Gap { name: String, secs: f64 },
}

#[derive(Debug, Clone)]
pub struct Timeline {
    pub event: String,
    pub project: String,
    pub width: u32,
    pub height: u32,
    pub items: Vec<Item>,
    pub bgm: Option<AudioMedia>,
}

/// 秒 → 时间线帧数（向下取整：片段不能比素材长）
fn tl_frames(secs: f64) -> u64 {
    (secs.max(0.0) * FPS as f64 + 1e-6).floor() as u64
}

/// 时间线上的一个时刻。`0s` 单独写 —— FCP 导出自己也这么写
fn tl_time(frames: u64) -> String {
    if frames == 0 {
        "0s".into()
    } else {
        format!("{frames}/{FPS}s")
    }
}

impl VideoMedia {
    /// 素材自己的时长（按它自己的帧率）
    fn asset_duration(&self) -> String {
        let (num, den) = self.fps;
        format!("{}/{}s", self.frames * den, num)
    }
    /// 放上时间线占几帧
    fn tl_frames(&self) -> u64 {
        let (num, den) = self.fps;
        if num == 0 {
            return 0;
        }
        self.frames * FPS * den / num
    }
}

impl AudioMedia {
    fn samples(&self) -> u64 {
        (self.secs.max(0.0) * self.info.rate as f64).round() as u64
    }
    fn asset_duration(&self) -> String {
        format!("{}/{}s", self.samples(), self.info.rate)
    }
}

pub fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // XML 1.0 不允许的控制字符直接丢掉，留着的话整份文件解析失败
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

/// 绝对路径 → `file://` URL。**中文、空格都要按 UTF-8 百分号编码**，
/// 否则 FCP 找不到文件，片段显示成离线（它不会报「路径写错了」）。
pub fn file_url(path: &Path) -> String {
    let mut out = String::from("file://");
    for b in path.to_string_lossy().as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => out.push(*b as char),
            b => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

/// 所有要写进 `media-rep` 的文件。导出完逐个核对在不在
pub fn media_paths(t: &Timeline) -> Vec<&Path> {
    let mut out = Vec::new();
    for item in &t.items {
        if let Item::Clip { video, dub, .. } = item {
            out.push(video.path.as_path());
            if let Some(d) = dub {
                out.push(d.path.as_path());
            }
        }
    }
    if let Some(b) = &t.bgm {
        out.push(b.path.as_path());
    }
    out
}

fn file_name(path: &Path) -> String {
    path.file_stem().and_then(|s| s.to_str()).unwrap_or("media").to_string()
}

fn audio_attrs(info: &AudioInfo) -> String {
    format!(r#" hasAudio="1" audioSources="1" audioChannels="{}" audioRate="{}""#, info.channels, info.rate)
}

/// 生成整份 FCPXML。纯函数：不碰磁盘，给什么写什么
pub fn build(t: &Timeline) -> String {
    let mut ids = 0usize;
    let mut next_id = || {
        ids += 1;
        format!("r{ids}")
    };

    // ---- resources：时间线格式、每种素材格式、每个素材 ----
    let mut res = String::new();
    let seq_format = next_id();
    let _ = writeln!(
        res,
        r#"    <format id="{seq_format}" frameDuration="1/{FPS}s" width="{}" height="{}" colorSpace="1-1-1 (Rec. 709)"/>"#,
        t.width, t.height
    );
    let mut formats: Vec<((u32, u32, u64, u64), String)> = Vec::new();
    let mut format_for = |v: &VideoMedia, res: &mut String, next_id: &mut dyn FnMut() -> String| -> String {
        let key = (v.width, v.height, v.fps.0, v.fps.1);
        if let Some((_, id)) = formats.iter().find(|(k, _)| *k == key) {
            return id.clone();
        }
        let id = next_id();
        let _ = writeln!(
            res,
            r#"    <format id="{id}" frameDuration="{}/{}s" width="{}" height="{}" colorSpace="1-1-1 (Rec. 709)"/>"#,
            v.fps.1, v.fps.0, v.width, v.height
        );
        formats.push((key, id.clone()));
        id
    };
    // 每个 Clip 对应的 (视频素材 id, 配音素材 id)
    let mut refs: Vec<Option<(String, Option<String>)>> = Vec::new();
    for item in &t.items {
        match item {
            Item::Clip { video, dub, .. } => {
                let fmt = format_for(video, &mut res, &mut next_id);
                let vid = next_id();
                let audio = video.audio.as_ref().map(audio_attrs).unwrap_or_default();
                let _ = writeln!(
                    res,
                    r#"    <asset id="{vid}" name="{}" start="0s" duration="{}" hasVideo="1" format="{fmt}" videoSources="1"{audio}>"#,
                    xml_escape(&file_name(&video.path)),
                    video.asset_duration()
                );
                let _ = writeln!(res, r#"      <media-rep kind="original-media" src="{}"/>"#, xml_escape(&file_url(&video.path)));
                res.push_str("    </asset>\n");
                let dub_id = dub.as_ref().map(|d| {
                    let id = next_id();
                    audio_asset(&mut res, &id, d);
                    id
                });
                refs.push(Some((vid, dub_id)));
            }
            Item::Gap { .. } => refs.push(None),
        }
    }
    let bgm_id = t.bgm.as_ref().map(|b| {
        let id = next_id();
        audio_asset(&mut res, &id, b);
        id
    });

    // ---- spine ----
    let durations: Vec<u64> = t
        .items
        .iter()
        .map(|item| match item {
            Item::Clip { video, .. } => video.tl_frames(),
            Item::Gap { secs, .. } => tl_frames(*secs).max(1),
        })
        .collect();
    let total: u64 = durations.iter().sum();
    let mut spine = String::new();
    let mut offset = 0u64;
    let mut style = 0usize;
    for (i, item) in t.items.iter().enumerate() {
        let dur = durations[i];
        // 配乐挂在第一个片段上、往后一直铺，FCP 里是时间线最底下那一条
        let bgm = if i == 0 {
            bgm_id.as_ref().zip(t.bgm.as_ref()).map(|(id, b)| {
                format!(
                    "        <asset-clip ref=\"{id}\" lane=\"-2\" offset=\"0s\" name=\"{}\" start=\"0s\" duration=\"{}\" audioRole=\"music\">\n          <adjust-volume amount=\"-12dB\"/>\n        </asset-clip>\n",
                    xml_escape(&file_name(&b.path)),
                    tl_time(tl_frames(b.secs).min(total))
                )
            })
        } else {
            None
        };
        match item {
            Item::Clip { name, dub, captions, cues, .. } => {
                let Some(Some((vid, dub_id))) = refs.get(i) else { continue };
                let _ = writeln!(
                    spine,
                    r#"      <asset-clip ref="{vid}" offset="{}" name="{}" start="0s" duration="{}" tcFormat="NDF" audioRole="dialogue">"#,
                    tl_time(offset),
                    xml_escape(name),
                    tl_time(dur)
                );
                if dub.is_some() {
                    // 配音替换原声（「合成一集」也是这么做的）。静音而不是删掉：FCP 里想换回原声，把音量推回来就行
                    spine.push_str("        <adjust-volume amount=\"-96dB\"/>\n");
                }
                if let (Some(d), Some(id)) = (dub, dub_id) {
                    let _ = writeln!(
                        spine,
                        r#"        <asset-clip ref="{id}" lane="-1" offset="0s" name="{}" start="0s" duration="{}" audioRole="dialogue"/>"#,
                        xml_escape(&format!("{name} · 配音")),
                        tl_time(tl_frames(d.secs).min(dur))
                    );
                }
                if let Some(b) = &bgm {
                    spine.push_str(b);
                }
                for c in captions {
                    let start = tl_frames(c.start).min(dur);
                    let end = tl_frames(c.end).min(dur);
                    if end <= start || c.text.trim().is_empty() {
                        continue;
                    }
                    style += 1;
                    let text = xml_escape(c.text.trim());
                    let _ = writeln!(
                        spine,
                        r#"        <caption lane="1" offset="{}" name="{text}" duration="{}" role="{CAPTION_ROLE}">"#,
                        tl_time(start),
                        tl_time(end - start)
                    );
                    let _ = writeln!(spine, r#"          <text placement="bottom"><text-style ref="ts{style}">{text}</text-style></text>"#);
                    let _ = writeln!(
                        spine,
                        r#"          <text-style-def id="ts{style}"><text-style font=".SF NS Text" fontSize="13" fontFace="Regular" fontColor="1 1 1 1" backgroundColor="0 0 0 1"/></text-style-def>"#
                    );
                    spine.push_str("        </caption>\n");
                }
                let _ = writeln!(spine, r#"        <marker start="0s" duration="1/{FPS}s" value="{}"/>"#, xml_escape(name));
                // 冲击点记成待办标记（completed="0"）：FCP 时间线索引里按「未完成」能一条条筛出来做
                for (at, text) in cues {
                    let f = tl_frames(*at).min(dur.saturating_sub(1));
                    if f == 0 {
                        continue; // 和镜头名那个标记撞在同一帧，FCP 只显示一个
                    }
                    let _ = writeln!(spine, r#"        <marker start="{}" duration="1/{FPS}s" value="{}" completed="0"/>"#, tl_time(f), xml_escape(text));
                }
                spine.push_str("      </asset-clip>\n");
            }
            Item::Gap { name, .. } => {
                let _ = writeln!(
                    spine,
                    r#"      <gap name="{}" offset="{}" start="0s" duration="{}">"#,
                    xml_escape(name),
                    tl_time(offset),
                    tl_time(dur)
                );
                if let Some(b) = &bgm {
                    spine.push_str(b);
                }
                // completed="0" = 待办标记，FCP 的时间线索引里能按「未完成」筛出来
                let _ = writeln!(spine, r#"        <marker start="0s" duration="1/{FPS}s" value="{}" completed="0"/>"#, xml_escape(name));
                spine.push_str("      </gap>\n");
            }
        }
        offset += dur;
    }

    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE fcpxml>\n<fcpxml version=\"1.11\">\n  <resources>\n");
    out.push_str(&res);
    out.push_str("  </resources>\n");
    let _ = writeln!(out, r#"  <event name="{}">"#, xml_escape(&t.event));
    let _ = writeln!(out, r#"    <project name="{}">"#, xml_escape(&t.project));
    let _ = writeln!(
        out,
        r#"      <sequence format="{seq_format}" duration="{}" tcStart="0s" tcFormat="NDF" audioLayout="stereo" audioRate="48k">"#,
        tl_time(total)
    );
    out.push_str("        <spine>\n");
    for line in spine.lines() {
        let _ = writeln!(out, "    {line}");
    }
    out.push_str("        </spine>\n      </sequence>\n    </project>\n  </event>\n</fcpxml>\n");
    out
}

fn audio_asset(res: &mut String, id: &str, a: &AudioMedia) {
    let _ = writeln!(
        res,
        r#"    <asset id="{id}" name="{}" start="0s" duration="{}"{}>"#,
        xml_escape(&file_name(&a.path)),
        a.asset_duration(),
        audio_attrs(&a.info)
    );
    let _ = writeln!(res, r#"      <media-rep kind="original-media" src="{}"/>"#, xml_escape(&file_url(&a.path)));
    res.push_str("    </asset>\n");
}

// ---------------------------------------------------------------------------
// 校验：用 FCP 自己带的 DTD
// ---------------------------------------------------------------------------

/// 本机 FCP 装包里的 1.11 DTD。没装 FCP 就是 `None`
pub fn fcp_dtd() -> Option<PathBuf> {
    let apps = std::fs::read_dir("/Applications").ok()?;
    apps.flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("Final Cut Pro") && n.ends_with(".app")))
        .map(|app| app.join("Contents/Frameworks/Interchange.framework/Versions/A/Resources/FCPXMLv1_11.dtd"))
        .find(|p| p.is_file())
}

/// 对着 FCP 的 DTD 校验。`Ok(false)` = 这台机器上没法校验（没装 FCP 或没有 xmllint）
pub fn validate(fcpxml: &Path) -> Result<bool> {
    let Some(dtd) = fcp_dtd() else { return Ok(false) };
    // DTD 要传编码过的 file:// 地址：FCP 装包路径里有空格，直接传路径 xmllint 读不了 DTD（实测报 xmlSAX2ResolveEntity）
    let out = match std::process::Command::new("xmllint").arg("--noout").arg("--dtdvalid").arg(file_url(&dtd)).arg(fcpxml).output() {
        Ok(o) => o,
        Err(_) => return Ok(false),
    };
    if out.status.success() {
        return Ok(true);
    }
    let msg: String = String::from_utf8_lossy(&out.stderr).chars().take(1200).collect();
    Err(Error::runtime(format!("导出的时间线没通过 FCP 的格式校验（这是我们的 bug，不是你的操作问题）：{msg}")))
}

// ---------------------------------------------------------------------------
// 读素材信息（ffprobe）
// ---------------------------------------------------------------------------

/// 读视频的尺寸、帧率、帧数、音轨
pub fn probe_video(path: &Path) -> Result<VideoMedia> {
    let v = ffprobe_json(path)?;
    let streams = v["streams"].as_array().cloned().unwrap_or_default();
    let video = streams
        .iter()
        .find(|s| s["codec_type"] == "video")
        .ok_or_else(|| Error::runtime(format!("{} 里没有画面", path.display())))?;
    let fps = parse_rate(video["r_frame_rate"].as_str().unwrap_or("24/1")).unwrap_or((24, 1));
    let secs = video["duration"]
        .as_str()
        .or_else(|| v["format"]["duration"].as_str())
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    let frames = video["nb_frames"]
        .as_str()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or_else(|| (secs * fps.0 as f64 / fps.1 as f64).floor() as u64);
    if frames == 0 {
        return Err(Error::runtime(format!("{} 读不出时长", path.display())));
    }
    Ok(VideoMedia {
        path: path.to_path_buf(),
        width: video["width"].as_u64().unwrap_or(0) as u32,
        height: video["height"].as_u64().unwrap_or(0) as u32,
        fps,
        frames,
        audio: streams.iter().find(|s| s["codec_type"] == "audio").map(audio_info),
    })
}

/// 读音频的时长、声道、采样率
pub fn probe_audio(path: &Path) -> Result<AudioMedia> {
    let v = ffprobe_json(path)?;
    let stream = v["streams"]
        .as_array()
        .and_then(|a| a.iter().find(|s| s["codec_type"] == "audio").cloned())
        .ok_or_else(|| Error::runtime(format!("{} 里没有声音", path.display())))?;
    let secs = v["format"]["duration"].as_str().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
    Ok(AudioMedia { path: path.to_path_buf(), secs, info: audio_info(&stream) })
}

fn audio_info(s: &serde_json::Value) -> AudioInfo {
    AudioInfo {
        channels: s["channels"].as_u64().unwrap_or(2) as u32,
        rate: s["sample_rate"].as_str().and_then(|r| r.parse().ok()).unwrap_or(48000),
    }
}

fn parse_rate(s: &str) -> Option<(u64, u64)> {
    let (a, b) = s.split_once('/')?;
    let (a, b) = (a.parse::<u64>().ok()?, b.parse::<u64>().ok()?);
    (a > 0 && b > 0).then_some((a, b))
}

fn ffprobe_json(path: &Path) -> Result<serde_json::Value> {
    let out = std::process::Command::new(crate::agent_runtime::video_tool::ffprobe_binary())
        .args(["-v", "error", "-show_entries", "stream=codec_type,width,height,r_frame_rate,nb_frames,duration,sample_rate,channels:format=duration", "-of", "json"])
        .arg(path)
        .output()
        .map_err(|e| Error::runtime(format!("跑 ffprobe 失败：{e}")))?;
    serde_json::from_slice(&out.stdout).map_err(|e| Error::runtime(format!("读不出 {} 的信息：{e}", path.display())))
}

// ---------------------------------------------------------------------------
// 拷素材
// ---------------------------------------------------------------------------

/// 文件夹 / 文件名里不能有的字符换掉（`/` 会变成子目录，`:` 在访达里显示成 `/`）
pub fn safe_name(s: &str) -> String {
    let t: String = s.trim().chars().map(|c| if matches!(c, '/' | '\\' | ':' | '\0') { '_' } else { c }).collect();
    if t.is_empty() { "未命名".into() } else { t }
}

/// 内容指纹：路径 + 大小 + 修改时间（FNV-1a）。同一条片子重导得到同一个名字，重出过的镜头得到新名字
fn fingerprint(path: &Path) -> String {
    let meta = std::fs::metadata(path).ok();
    let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let mtime = meta
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut h: u64 = 0xcbf29ce484222325;
    for b in format!("{}|{size}|{mtime}", path.display()).bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:08x}", h & 0xffff_ffff)
}

/// 拷进 `dir`，名字是 `{stem}_{指纹}.{扩展名}`。已经有一样大小的同名文件就不再拷
pub fn copy_media(src: &Path, dir: &Path, stem: &str) -> Result<PathBuf> {
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("mp4").to_lowercase();
    let dest = dir.join(format!("{}_{}.{ext}", safe_name(stem), fingerprint(src)));
    let src_len = std::fs::metadata(src).map(|m| m.len()).map_err(|e| Error::runtime(format!("读不到 {}：{e}", src.display())))?;
    if std::fs::metadata(&dest).map(|m| m.len()).ok() == Some(src_len) {
        return Ok(dest);
    }
    std::fs::copy(src, &dest).map_err(|e| Error::runtime(format!("拷 {} 到 {} 失败：{e}", src.display(), dir.display())))?;
    Ok(dest)
}

// ---------------------------------------------------------------------------
// 一集 → 导出目录
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ExportOptions {
    /// 用户选的位置（可以是外置硬盘）。这一集的东西放进它下面、以项目名命名的子文件夹
    pub dest_dir: String,
    /// FCP 里的事件名。用户自己建事件，这里照他填的写
    pub event: String,
    /// FCP 里的项目名，也是导出子文件夹和 `.fcpxml` 的名字
    pub project: String,
    /// 逐镜 RealESRGAN ×2（和「合成一集」共用超分缓存）
    #[serde(default)]
    pub upscale: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Exported {
    pub fcpxml: String,
    pub folder: String,
    pub clips: usize,
    /// 还没出片、时间线上留了空的镜号
    pub missing: Vec<u32>,
    /// 有没有对着 FCP 的 DTD 校验过（没装 FCP 的机器校验不了）
    pub validated: bool,
}

/// 用户选的导出位置还在不在。外置硬盘拔了是最常见的原因，要说出来
fn check_dest(dest: &Path) -> Result<()> {
    if dest.as_os_str().is_empty() {
        return Err(Error::config("先选导出位置"));
    }
    if dest.is_dir() {
        return Ok(());
    }
    if dest.starts_with("/Volumes") {
        return Err(Error::config(format!("导出位置找不到：{} —— 外置硬盘是不是没挂上？", dest.display())));
    }
    Err(Error::config(format!("导出位置不在了：{}", dest.display())))
}

pub async fn export_episode(
    config_dir: &Path,
    project_id: &str,
    episode_id: &str,
    opts: &ExportOptions,
    progress: impl Fn(usize, usize, &str),
) -> Result<Exported> {
    let (event, project) = (opts.event.trim(), opts.project.trim());
    if event.is_empty() || project.is_empty() {
        return Err(Error::config("事件名和项目名都要填 —— FCP 里就叫这个"));
    }
    let dest = PathBuf::from(opts.dest_dir.trim());
    check_dest(&dest)?;

    let proj_dir = crate::cinema::cinema_projects_dir(config_dir).join(project_id);
    let ep_dir = proj_dir.join("episodes").join(episode_id);
    let ep: crate::cinema::EpisodeData = serde_json::from_str(
        &std::fs::read_to_string(ep_dir.join("episode.json")).map_err(|e| Error::config(format!("读不到分集：{e}")))?,
    )
    .map_err(|e| Error::config(format!("分集文件坏了：{e}")))?;
    let aspect = std::fs::read_to_string(proj_dir.join("project.json"))
        .ok()
        .and_then(|c| serde_json::from_str::<crate::cinema::CinemaProject>(&c).ok())
        .map(|p| p.aspect_ratio)
        .unwrap_or_else(|| "9:16".to_string());
    // 时间线按 1080P：超分过的片子正好铺满，没超分的 FCP 按比例放大（和「合成一集」一样不裁不拉）
    let (width, height) = crate::cinema::episode_frame(&aspect, true);

    let folder = dest.join(safe_name(project));
    let media_dir = folder.join("媒体");
    std::fs::create_dir_all(&media_dir).map_err(|e| Error::config(format!("在 {} 建不出文件夹：{e}", dest.display())))?;

    let shots_dir = ep_dir.join("shots");
    let audio_dir = ep_dir.join("audio");
    let upscaled_dir = ep_dir.join("upscaled");
    let mut items = Vec::new();
    let mut missing = Vec::new();
    // 台词 + 这一段在时间线上的秒数，给 SRT 用。空镜也要占位，不然后面的字幕全部提前
    let mut cue_src: Vec<(String, f64)> = Vec::new();

    // 成片加过片头封面的话，时间线上也要有它 —— 和剪映那条、SRT 是同一个规矩
    let intro = ep_dir.join("cover_intro.mp4");
    if crate::episode_cover::has_cover_intro(&ep_dir) && intro.is_file() {
        let video = probe_video(&copy_media(&intro, &media_dir, "片头封面")?)?;
        cue_src.push((String::new(), video.tl_frames() as f64 / FPS as f64));
        items.push(Item::Clip { name: "片头封面".into(), video, dub: None, captions: Vec::new(), cues: Vec::new() });
    }

    let route = crate::cinema::project_route(config_dir, project_id);
    // 已经整条放进去的长镜头场
    let mut placed_scenes: Vec<u32> = Vec::new();
    let total = ep.storyboard.len();
    for (i, shot) in ep.storyboard.iter().enumerate() {
        let label = format!("第 {} 镜", shot.shot_num);
        progress(i, total, &label);
        // **整段出的场放一整条**（长镜头，见 `EpisodeData::scene_renders`），这一场各镜不再各放一段。
        // 优先当前线路出的；只有另一条线路出过也用它，总比空着强
        // 判断和合成一集、剪映、组装页读同一个函数（`cinema::long_take_clip`）
        {
            if let Some(r) = crate::cinema::long_take_clip(&ep, shot.scene_no, route) {
                if placed_scenes.contains(&shot.scene_no) {
                    continue;
                }
                placed_scenes.push(shot.scene_no);
                let mut in_scene: Vec<&crate::cinema::StoryboardShot> = ep.storyboard.iter().filter(|s| s.scene_no == shot.scene_no).collect();
                in_scene.sort_by_key(|s| s.shot_num);
                let (first, last) = (in_scene[0].shot_num, in_scene[in_scene.len() - 1].shot_num);
                let src = PathBuf::from(&r.path);
                let take = if opts.upscale {
                    crate::cinema::upscaled_take(&src, &upscaled_dir).await.map_err(|e| Error::runtime(format!("第 {} 场长镜头超分失败：{e}", shot.scene_no)))?
                } else {
                    src
                };
                let video = probe_video(&copy_media(&take, &media_dir, &format!("第{first:02}-{last:02}镜_长镜头"))?)?;
                let secs = video.tl_frames() as f64 / FPS as f64;
                // 台词按各镜时长占比铺在这一条上（一镜一个窗口，窗口等长）
                let dur: f64 = in_scene.iter().map(|s| s.duration.max(0.1) as f64).sum();
                let (mut captions, mut cues, mut at) = (Vec::new(), Vec::new(), 0.0);
                for s in &in_scene {
                    let share = secs * s.duration.max(0.1) as f64 / dur;
                    cues.extend(shot_cues(s, share).into_iter().map(|(t, c)| (at + t, c)));
                    for c in crate::subtitle::cues_from_dialogue(&s.dialogue, share) {
                        captions.push(Caption { text: c.text, start: at + c.start, end: at + c.end });
                    }
                    cue_src.push((s.dialogue.clone(), share));
                    at += share;
                }
                let scene = shot.scene_name.trim().trim_start_matches("场景：").trim();
                let base = format!("第 {} 场 · 长镜头（第 {first}–{last} 镜）", shot.scene_no);
                let name = if scene.is_empty() { base } else { format!("{base} · {scene}") };
                items.push(Item::Clip { name, video, dub: None, captions, cues });
                continue;
            }
        }
        let Some(take) = crate::cinema::stitch_source(shot, &shots_dir) else {
            missing.push(shot.shot_num);
            let secs = tl_frames(shot.duration.max(1.0) as f64) as f64 / FPS as f64;
            cue_src.push((String::new(), secs));
            items.push(Item::Gap { name: format!("{label} · 未出片"), secs });
            continue;
        };
        let take = if opts.upscale {
            crate::cinema::upscaled_take(&take, &upscaled_dir).await.map_err(|e| Error::runtime(format!("{label}超分失败：{e}")))?
        } else {
            take
        };
        let stem = format!("第{:02}镜", shot.shot_num);
        let video = probe_video(&copy_media(&take, &media_dir, &stem)?)?;
        let secs = video.tl_frames() as f64 / FPS as f64;
        // 配音：有 TTS 文件就替换原声，和「合成一集」的 `pick_shot_audio` 同一个判断
        let dub_src = audio_dir.join(format!("{}.wav", shot.id));
        let dub = if dub_src.is_file() { Some(probe_audio(&copy_media(&dub_src, &media_dir, &format!("{stem}_配音"))?)?) } else { None };
        let captions = crate::subtitle::cues_from_dialogue(&shot.dialogue, secs)
            .into_iter()
            .map(|c| Caption { text: c.text, start: c.start, end: c.end })
            .collect();
        cue_src.push((shot.dialogue.clone(), secs));
        let scene = shot.scene_name.trim().trim_start_matches("场景：").trim();
        let name = if scene.is_empty() { label } else { format!("{label} · {scene}") };
        let cues = shot_cues(shot, secs);
        items.push(Item::Clip { name, video, dub, captions, cues });
    }
    progress(total, total, "写时间线");
    if items.iter().all(|i| matches!(i, Item::Gap { .. })) {
        return Err(Error::config("这一集还没有出好片的镜头，先出片"));
    }

    let bgm = ["bgm.flac", "bgm.wav", "bgm.mp3"].iter().map(|n| ep_dir.join(n)).find(|p| p.is_file());
    let bgm = match bgm {
        Some(p) => Some(probe_audio(&copy_media(&p, &media_dir, "配乐")?)?),
        None => None,
    };

    let timeline = Timeline { event: event.into(), project: project.into(), width, height, items, bgm };
    // 写进时间线的每一个文件都要真的在 —— FCP 打开后找不到文件只会显示离线，不说是哪里错
    for p in media_paths(&timeline) {
        if !p.is_file() {
            return Err(Error::runtime(format!("素材没拷过去：{}", p.display())));
        }
    }
    let fcpxml = folder.join(format!("{}.fcpxml", safe_name(project)));
    std::fs::write(&fcpxml, build(&timeline)).map_err(|e| Error::runtime(format!("写时间线失败：{e}")))?;
    let validated = validate(&fcpxml)?;

    // SRT 放在旁边：FCP 里「文件 → 导入 → 字幕」也能用，给别的软件也行
    let srt = crate::subtitle::to_srt(&crate::subtitle::episode_cues(&cue_src));
    let srt_path = folder.join(format!("{}.srt", safe_name(project)));
    if srt.trim().is_empty() {
        let _ = std::fs::remove_file(&srt_path);
    } else {
        std::fs::write(&srt_path, srt).map_err(|e| Error::runtime(format!("写字幕失败：{e}")))?;
    }

    let clips = timeline.items.iter().filter(|i| matches!(i, Item::Clip { .. })).count();
    Ok(Exported {
        fcpxml: fcpxml.to_string_lossy().to_string(),
        folder: folder.to_string_lossy().to_string(),
        clips,
        missing,
        validated,
    })
}

/// 后台任务版：超分时一镜一两分钟，整集几十分钟。导出完直接用 FCP 打开这份时间线
pub fn start(app: Option<tauri::AppHandle>, config_dir: PathBuf, project_id: String, episode_id: String, opts: ExportOptions) -> Result<crate::jobs::Job> {
    let title = if opts.upscale { "导出到 Final Cut Pro · 超分 1080P" } else { "导出到 Final Cut Pro" };
    let job_id = crate::jobs::start_exclusive(&config_dir, crate::jobs::JobKind::FcpExport, &project_id, &episode_id, "", 0, title)
        .map_err(|_| Error::config("这一集已经在导出了 —— 进度在右下角"))?;
    let started = crate::jobs::get(&config_dir, &job_id).ok_or_else(|| Error::runtime("任务登记后读不回来"))?;
    crate::jobs::emit(app.as_ref(), Some(&started));
    tauri::async_runtime::spawn(async move {
        let out = export_episode(&config_dir, &project_id, &episode_id, &opts, |done, total, label| {
            let job = crate::jobs::set_progress(&config_dir, &job_id, done, total, label);
            crate::jobs::emit(app.as_ref(), job.as_ref());
        })
        .await;
        let job = match out {
            Ok(ex) => {
                let mut notes = Vec::new();
                if !ex.missing.is_empty() {
                    let nums: Vec<String> = ex.missing.iter().map(|n| n.to_string()).collect();
                    notes.push(format!("第 {} 镜还没出片，时间线上按分镜秒数留了空、带待办标记", nums.join("、")));
                }
                if !ex.validated {
                    notes.push("这台机器上没找到 FCP 的格式定义文件，导出前没能校验".into());
                }
                // 双击 .fcpxml 就是 FCP 的导入 —— 替用户点这一下
                if let Err(e) = std::process::Command::new("open").arg(&ex.fcpxml).spawn() {
                    notes.push(format!("没能自动打开 FCP（{e}），去 {} 双击那个 .fcpxml", ex.folder));
                }
                let summary = format!("{} 镜 → {}", ex.clips, ex.folder);
                // 导出是成功的，这些是「成了但有话要说」—— 放 remarks，不放 notes（那一栏在界面上叫「没通过检查」）
                crate::jobs::finish_with_remarks(&config_dir, &job_id, Some(&ex.fcpxml), Some(&summary), &[], &notes, None)
            }
            Err(e) => crate::jobs::finish(&config_dir, &job_id, None, Some(&e.to_string())),
        };
        crate::jobs::emit(app.as_ref(), job.as_ref());
    });
    Ok(started)
}

#[tauri::command]
pub fn cinema_export_fcp_start(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
    project_id: String,
    episode_id: String,
    options: ExportOptions,
) -> Result<crate::jobs::Job> {
    start(Some(app), state.config_dir.clone(), project_id, episode_id, options)
}

/// FCP 里这一集该叫什么：事件名照资源库里已有的对齐，项目名 `S01E02_本机无武器_初剪`。
///
/// 用户 2026-10-05：「把组装里的事件名和项目名自动对齐到 FCPX 里。项目名为：S01E0X_剧集名_初剪」。
/// 他在《回收师》资源库里的事件是 `01_PV01_先行预告片`、`02_S01E01_第三次`（前面的序号是资源库里的排序），
/// 项目是 `S01E01_第三次_初剪`。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct FcpNames {
    pub event: String,
    pub project: String,
    /// 认出来的资源库（`.fcpbundle` 的名字）。没找到是 `None`，事件名就没有序号
    pub library: Option<String>,
}

/// `E02-《本机无武器》` → `本机无武器`（和前端 `defaultFcpName` 同一套去法）
fn episode_base_title(title: &str) -> String {
    let t = title.trim();
    let mut rest = t;
    let bytes = t.as_bytes();
    if bytes.first().is_some_and(|b| *b == b'E' || *b == b'e') {
        let digits = t[1..].chars().take_while(|c| c.is_ascii_digit()).count();
        if digits > 0 {
            let after = t[1 + digits..].trim_start();
            if let Some(c) = after.chars().next().filter(|c| "-_·:：".contains(*c)) {
                rest = after[c.len_utf8()..].trim_start();
            } else if after.len() < t[1 + digits..].len() {
                rest = after;
            }
        }
    }
    rest.replace(['《', '》'], "").trim().to_string()
}

/// 导出位置（或它的上一层）里的 FCP 资源库。
fn find_library(dest: &Path) -> Option<PathBuf> {
    for dir in [Some(dest), dest.parent()].into_iter().flatten() {
        let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
            .ok()?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "fcpbundle") && p.is_dir())
            .collect();
        found.sort();
        if let Some(p) = found.into_iter().next() {
            return Some(p);
        }
    }
    None
}

/// 资源库里的事件名（有 `CurrentVersion.fcpevent` 的子文件夹）。
fn library_events(library: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(library)
        .map(|it| {
            it.filter_map(|e| e.ok())
                .filter(|e| e.path().join("CurrentVersion.fcpevent").is_file())
                .filter_map(|e| e.file_name().to_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// 按资源库里已有的事件定这一集的名字：
/// - 已经有这一集的事件（名字里有 `S01E02`）→ 原样用它，导入时并进去；
/// - 没有 → 序号接着资源库里最大的往下排：`03_S01E02_本机无武器`；
/// - 找不到资源库 → `S01E02_本机无武器`（不猜序号）。
pub fn fcp_names(dest: &Path, episode_num: u32, title: &str) -> FcpNames {
    let code = format!("S01E{episode_num:02}");
    let base = episode_base_title(title);
    let base = if base.is_empty() { format!("第 {episode_num} 集") } else { base };
    let project = format!("{code}_{base}_初剪");
    let Some(library) = find_library(dest) else {
        return FcpNames { event: format!("{code}_{base}"), project, library: None };
    };
    let events = library_events(&library);
    let mine = events.iter().find(|e| {
        e.find(&code).is_some_and(|at| !e[at + code.len()..].starts_with(|c: char| c.is_ascii_digit()))
    });
    let event = match mine {
        Some(e) => e.clone(),
        None => {
            let next = events.iter().filter_map(|e| e.split('_').next()?.parse::<u32>().ok()).max().unwrap_or(0) + 1;
            format!("{next:02}_{code}_{base}")
        }
    };
    FcpNames { event, project, library: library.file_stem().and_then(|s| s.to_str()).map(str::to_string) }
}

#[tauri::command]
pub fn cinema_fcp_names(dest_dir: String, episode_num: u32, title: String) -> FcpNames {
    fcp_names(Path::new(&dest_dir), episode_num, &title)
}

/// 选导出位置（系统的选文件夹框，外置硬盘也在里面）。取消回 `None`。
/// `default_dir` 是上次选的，还在的话框就开在那里
#[tauri::command]
pub async fn cinema_choose_export_folder(default_dir: Option<String>) -> Result<Option<String>> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut script = String::from("POSIX path of (choose folder with prompt \"选择导出到哪里（外置硬盘也可以）\"");
        if let Some(d) = default_dir.filter(|d| Path::new(d).is_dir()) {
            let quoted = d.replace('\\', "\\\\").replace('"', "\\\"");
            let _ = write!(script, " default location (POSIX file \"{quoted}\")");
        }
        script.push(')');
        let out = std::process::Command::new("osascript").args(["-e", &script]).output();
        // 用户点了取消，osascript 以 -128 退出 —— 不是错误
        Ok(out.ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|p| !p.is_empty()))
    })
    .await
    .map_err(|e| Error::runtime(format!("选文件夹的框没打开：{e}")))?
}

/// 这一镜的打斗冲击点，按片子实际时长等比换算（分镜时长和出出来的片子常差零点几秒）
fn shot_cues(shot: &crate::cinema::StoryboardShot, secs: f64) -> Vec<(f64, String)> {
    cues_scaled(&shot.segments, shot.duration, secs)
}

fn cues_scaled(segments: &[crate::shot_table::ShotSegment], planned: f32, secs: f64) -> Vec<(f64, String)> {
    let scale = if planned > 0.1 { secs / planned as f64 } else { 1.0 };
    crate::combat::contact_points(segments).into_iter().map(|(t, c)| (t as f64 * scale, c)).collect()
}

#[cfg(test)]
mod tests {

    /// 用户 2026-10-05 的资源库：`01_PV01_先行预告片`、`02_S01E01_第三次`
    #[test]
    fn fcp_名字照资源库对齐() {
        let d = tempfile::tempdir().unwrap();
        let lib = d.path().join("《回收师》.fcpbundle");
        for e in ["01_PV01_先行预告片", "02_S01E01_第三次"] {
            std::fs::create_dir_all(lib.join(e)).unwrap();
            std::fs::write(lib.join(e).join("CurrentVersion.fcpevent"), b"").unwrap();
        }
        std::fs::create_dir_all(lib.join("__Temp")).unwrap();
        // 已有的那一集：原样用资源库里的事件名
        let e1 = fcp_names(d.path(), 1, "E01-《第三次》");
        assert_eq!((e1.event.as_str(), e1.project.as_str()), ("02_S01E01_第三次", "S01E01_第三次_初剪"));
        assert_eq!(e1.library.as_deref(), Some("《回收师》"));
        // 新的一集：序号接着排
        let e2 = fcp_names(d.path(), 2, "E02-《本机无武器》");
        assert_eq!((e2.event.as_str(), e2.project.as_str()), ("03_S01E02_本机无武器", "S01E02_本机无武器_初剪"));
        // S01E1 不能认成 S01E10
        let e10 = fcp_names(d.path(), 10, "第十集");
        assert_eq!(e10.event, "03_S01E10_第十集");
        // 导出位置是资源库里面一层也认
        let sub = d.path().join("素材");
        std::fs::create_dir_all(&sub).unwrap();
        assert_eq!(fcp_names(&sub, 2, "本机无武器").event, "03_S01E02_本机无武器");
        // 找不到资源库：不猜序号
        let none = tempfile::tempdir().unwrap();
        let n = fcp_names(none.path(), 2, "E02-《本机无武器》");
        assert_eq!((n.event.as_str(), n.library), ("S01E02_本机无武器", None));
        assert_eq!(episode_base_title("E02 · 本机无武器"), "本机无武器");
        assert_eq!(episode_base_title("《第三次》"), "第三次");
    }

    use super::*;

    fn video(path: &str, secs: u64, audio: bool) -> VideoMedia {
        VideoMedia {
            path: PathBuf::from(path),
            width: 896,
            height: 512,
            fps: (24, 1),
            frames: secs * 24,
            audio: audio.then_some(AudioInfo { channels: 2, rate: 32000 }),
        }
    }

    fn sample() -> Timeline {
        Timeline {
            event: "02_S01E01_第三次".into(),
            project: "S01E01 · 第三次 & <测试>".into(),
            width: 1920,
            height: 1080,
            items: vec![
                Item::Clip {
                    name: "第 1 镜 · 拆机间".into(),
                    video: video("/Volumes/外置 硬盘/回收师/媒体/第01镜_abcd.mp4", 8, true),
                    dub: None,
                    captions: vec![Caption { text: "你叫我什么？".into(), start: 1.0, end: 3.0 }],
                    cues: vec![(0.0, "撞镜头名".into()), (4.5, "冲击点：刀背砸中护臂 & <测试>".into()), (99.0, "越界夹到最后一帧".into())],
                },
                Item::Gap { name: "第 2 镜 · 未出片".into(), secs: 5.0 },
                Item::Clip {
                    name: "第 3 镜".into(),
                    video: VideoMedia { fps: (25, 1), frames: 250, ..video("/tmp/第03镜.mp4", 10, false) },
                    dub: Some(AudioMedia { path: "/tmp/第03镜_配音.wav".into(), secs: 12.0, info: AudioInfo { channels: 1, rate: 24000 } }),
                    captions: vec![Caption { text: "越界的不算".into(), start: 9.0, end: 30.0 }],
                    cues: Vec::new(),
                },
            ],
            bgm: Some(AudioMedia { path: "/tmp/配乐.mp3".into(), secs: 600.0, info: AudioInfo { channels: 2, rate: 44100 } }),
        }
    }

    #[test]
    fn 路径里的中文和空格按_utf8_编码() {
        let url = file_url(Path::new("/Volumes/外置 硬盘/第01镜.mp4"));
        assert!(url.starts_with("file:///Volumes/"));
        assert!(!url.contains(' ') && !url.contains('外'), "{url}");
        // 解回来要和原路径一模一样 —— 编错一个字节 FCP 就找不到文件
        let decoded = urlencoding::decode(url.trim_start_matches("file://")).unwrap();
        assert_eq!(decoded, "/Volumes/外置 硬盘/第01镜.mp4");
    }

    #[test]
    fn 片段首尾相接_没出片的镜头留空不错位() {
        let xml = build(&sample());
        // 第 1 镜 8 秒 = 192 帧；空镜 5 秒 = 120 帧从 192 开始；第 3 镜 25 帧 10 秒 = 240 帧从 312 开始
        assert!(xml.contains(r#"offset="0s" name="第 1 镜 · 拆机间" start="0s" duration="192/24s""#), "{xml}");
        assert!(xml.contains(r#"<gap name="第 2 镜 · 未出片" offset="192/24s" start="0s" duration="120/24s">"#));
        assert!(xml.contains(r#"offset="312/24s" name="第 3 镜" start="0s" duration="240/24s""#));
        assert!(xml.contains(r#"duration="552/24s" tcStart="0s""#), "总长不对");
        // 25 帧的素材按它自己的帧率写时长
        assert!(xml.contains(r#"duration="250/25s""#));
        assert!(xml.contains(r#"value="第 2 镜 · 未出片" completed="0""#), "空镜要是待办标记");
    }

    #[test]
    fn 有配音的镜头原声静音_配音挂在下面() {
        let xml = build(&sample());
        // 第 3 镜是最后一个，切到文件末尾就是它
        let clip3 = &xml[xml.find(r#"name="第 3 镜""#).unwrap()..];
        assert!(clip3.contains(r#"<adjust-volume amount="-96dB"/>"#));
        // 配音 12 秒，片段只有 10 秒：截到片段长度
        assert!(clip3.contains(r#"lane="-1" offset="0s" name="第 3 镜 · 配音" start="0s" duration="240/24s""#), "{clip3}");
        // 第 1 镜没有配音，原声不动
        let clip1 = &xml[xml.find(r#"name="第 1 镜 · 拆机间""#).unwrap()..xml.find("<gap").unwrap()];
        assert!(!clip1.contains("-96dB"));
    }

    #[test]
    fn 字幕落在自己那一镜里_越界的截掉() {
        let xml = build(&sample());
        assert!(xml.contains(r#"<caption lane="1" offset="24/24s" name="你叫我什么？" duration="48/24s""#), "{xml}");
        // 第 3 镜那句从 9 秒到 30 秒，片段只有 10 秒
        assert!(xml.contains(r#"offset="216/24s" name="越界的不算" duration="24/24s""#));
    }

    #[test]
    fn 特殊字符转义_配乐铺满整条() {
        let xml = build(&sample());
        assert!(xml.contains("S01E01 · 第三次 &amp; &lt;测试&gt;"));
        assert!(xml.contains(r#"<event name="02_S01E01_第三次">"#));
        // 配乐 600 秒比整集长：截到整集 552 帧
        assert!(xml.contains(r#"lane="-2" offset="0s" name="配乐" start="0s" duration="552/24s" audioRole="music""#));
    }

    /// **对着本机 FCP 的 DTD 校验。** 没装 FCP 的机器跳过（打印一行说明，不算通过也不算失败）
    /// 冲击点是片段里的待办标记：转义、夹到片段内；和镜头名撞在第 0 帧的丢掉（FCP 同帧只显示一个）
    #[test]
    fn 打斗冲击点写成待办标记() {
        let x = build(&sample());
        // 8 秒的片段：4.5s = 108 帧；99s 夹到最后一帧 191
        assert!(x.contains(r#"<marker start="108/24s" duration="1/24s" value="冲击点：刀背砸中护臂 &amp; &lt;测试&gt;" completed="0"/>"#), "{x}");
        assert!(x.contains(r#"<marker start="191/24s" duration="1/24s" value="越界夹到最后一帧" completed="0"/>"#), "{x}");
        assert!(!x.contains("撞镜头名"));
    }

    #[test]
    fn 换算到片子实际时长() {
        use crate::shot_table::{Exchange, ShotSegment};
        let mut seg: ShotSegment = serde_json::from_value(serde_json::json!({"start": 4.0, "end": 6.0})).unwrap();
        seg.exchange = Some(Exchange { contact_at: Some(5.0), result: "刀背砸中护臂".into(), ..Default::default() });
        let c = cues_scaled(&[seg], 10.0, 12.0);
        assert_eq!(c.len(), 1);
        assert!((c[0].0 - 6.0).abs() < 1e-6, "{c:?}");
    }

    #[test]
    fn 通过_fcp_自带_dtd_的校验() {
        let Some(_) = fcp_dtd() else {
            eprintln!("本机没有 FCP 的 DTD，跳过");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.fcpxml");
        std::fs::write(&path, build(&sample())).unwrap();
        assert!(validate(&path).unwrap(), "xmllint 没跑起来");
        // 反过来：坏掉的文件必须被拦下，不然上面那条是假绿
        let broken = build(&sample()).replace("<spine>", "<spine><bogus/>");
        std::fs::write(&path, broken).unwrap();
        assert!(validate(&path).is_err());
    }

    /// 拿真项目导一次（只读：素材拷到 `ANIMA_FCP_DEST`，项目数据不动）。
    /// `ANIMA_FCP_PROJECT` / `ANIMA_FCP_EPISODE` 选哪一集
    #[tokio::test]
    #[ignore]
    async fn 真项目导出实跑() {
        let cfg = dirs::home_dir().unwrap().join("Library/Application Support/com.anima.desktop");
        let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("要设 {k}"));
        let opts = ExportOptions {
            dest_dir: env("ANIMA_FCP_DEST"),
            event: std::env::var("ANIMA_FCP_EVENT").unwrap_or_else(|_| "Anima 导出测试".into()),
            project: std::env::var("ANIMA_FCP_NAME").unwrap_or_else(|_| "导出测试".into()),
            upscale: false,
        };
        let ex = export_episode(&cfg, &env("ANIMA_FCP_PROJECT"), &env("ANIMA_FCP_EPISODE"), &opts, |d, t, l| eprintln!("{d}/{t} {l}"))
            .await
            .unwrap();
        println!("{}", serde_json::to_string_pretty(&ex).unwrap());
    }

    #[test]
    fn 同一个文件重导同名_不重拷() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("shot_x-b-1.mp4");
        std::fs::write(&src, b"video").unwrap();
        let out = dir.path().join("媒体");
        std::fs::create_dir_all(&out).unwrap();
        let a = copy_media(&src, &out, "第01镜").unwrap();
        let b = copy_media(&src, &out, "第01镜").unwrap();
        assert_eq!(a, b);
        assert!(a.file_name().unwrap().to_str().unwrap().starts_with("第01镜_"));
        assert_eq!(std::fs::read_dir(&out).unwrap().count(), 1);
        assert_eq!(safe_name(" E01/第三次: 上 "), "E01_第三次_ 上");
    }
}
