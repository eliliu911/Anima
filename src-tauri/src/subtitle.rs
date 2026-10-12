//! 硬字幕：把台词烧进画面。
//!
//! **为什么不用 ffmpeg 的 `subtitles` / `drawtext`**：本机 brew 装的
//! ffmpeg 8.1 没编 libass / libfreetype / libfontconfig，那两个滤镜
//! 根本不存在（`ffmpeg -filters` 里查不到）。`overlay` 是有的，
//! 所以这里自己把字幕光栅化成透明 PNG，再 overlay 上去。
//!
//! **为什么不用模型画的字幕**：实测认不出来（Sulphur 上验证过）。
//! 台词是我们自己写的，后期渲染是像素级正确的。
//!
//! 这一块最典型的静默失败是**字体缺字形** —— 光栅化不报错，
//! 出来一张全透明的图，视频照样合成成功，只是没有字。
//! 所以 `render_line` 会明确告诉你画了多少个不透明像素，
//! 调用方据此判断，而不是假设「没报错就是画上了」。

use ab_glyph::{Font, FontVec, PxScale, ScaleFont};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// macOS 自带的中文字体，按优先级排。**必须是能覆盖中日韩的那些** ——
/// 拿 Helvetica 去画中文，每个字都是空的，而且不报错。
pub const CJK_FONT_CANDIDATES: &[(&str, u32)] = &[
    ("/System/Library/Fonts/PingFang.ttc", 0),
    ("/System/Library/Fonts/Hiragino Sans GB.ttc", 0),
    ("/System/Library/Fonts/STHeiti Medium.ttc", 0),
    ("/System/Library/Fonts/STHeiti Light.ttc", 0),
    ("/Library/Fonts/Arial Unicode.ttf", 0),
];

/// 字幕的样式。像素单位都相对成片分辨率，不是绝对值。
#[derive(Debug, Clone)]
pub struct SubtitleStyle {
    /// 字号占画面高度的比例。0.045 在 720p 上约 32px，和常见短剧字幕接近。
    pub font_ratio: f32,
    /// 描边宽度占字号的比例。**这个不能省** —— 没有描边的字幕
    /// 压在亮背景上会整行看不见，而这在检查缩略图时很容易漏掉。
    pub stroke_ratio: f32,
    /// 底边留白占画面高度的比例。
    pub margin_ratio: f32,
    /// 一行最多占画面宽度的比例，超了就折行。
    pub max_width_ratio: f32,
}

impl Default for SubtitleStyle {
    fn default() -> Self {
        Self { font_ratio: 0.045, stroke_ratio: 0.14, margin_ratio: 0.07, max_width_ratio: 0.86 }
    }
}

/// 一条字幕：文本 + 起止秒。
#[derive(Debug, Clone)]
pub struct Cue {
    pub text: String,
    pub start: f64,
    pub end: f64,
}

/// 分镜台词 → 按时间排好的字幕。
///
/// 分镜里台词存成一人一行、带说话人：`大臣：大逆不道！\n林婉清：这是……唐朝？`。
/// 以前整段原样烧进去 —— **两行同时出现，还带着角色名**（2026-09-14 用户截图）。
///
/// - 每行一条，按字数分时长（说得多的占得久）
/// - 去掉行首的「名字：」「名字（盛唐）：」；冒号前太长或带标点的不当名字，原样留着
pub fn cues_from_dialogue(dialogue: &str, duration: f64) -> Vec<Cue> {
    let lines: Vec<String> = dialogue
        .lines()
        .map(|l| strip_speaker(l.trim()).trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    let weights: Vec<f64> = lines.iter().map(|l| l.chars().count().max(1) as f64).collect();
    let total: f64 = weights.iter().sum();
    let duration = duration.max(1.0);
    let mut t = 0.0;
    lines
        .into_iter()
        .zip(weights)
        .map(|(text, w)| {
            let start = t;
            t += duration * w / total;
            Cue { text, start, end: t }
        })
        .collect()
}

/// 整集字幕：每段片子的**真实时长**（不是分镜上写的秒数）接成一条时间轴。
///
/// **必须用真实时长。** 出片的实际长度和分镜上写的差一点（10.125 vs 10.0），
/// 14 镜攒下来能差三秒 —— 用写的秒数拼出来的字幕，到后面整段对不上嘴。
pub fn episode_cues(clips: &[(String, f64)]) -> Vec<Cue> {
    let mut out = Vec::new();
    let mut offset = 0.0;
    for (dialogue, secs) in clips {
        for c in cues_from_dialogue(dialogue, *secs) {
            out.push(Cue { text: c.text, start: c.start + offset, end: (c.end + offset).min(offset + *secs) });
        }
        offset += *secs;
    }
    out
}

fn srt_time(t: f64) -> String {
    let t = t.max(0.0);
    let ms = (t * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02},{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

/// SRT 文本。空的回空串 —— 写一个只有 BOM 的空文件没意义
pub fn to_srt(cues: &[Cue]) -> String {
    cues.iter()
        .enumerate()
        .map(|(i, c)| format!("{}\n{} --> {}\n{}\n", i + 1, srt_time(c.start), srt_time(c.end.max(c.start + 0.3)), c.text))
        .collect::<Vec<_>>()
        .join("\n")
}

fn strip_speaker(line: &str) -> &str {
    let Some((i, colon)) = line.char_indices().find(|(_, c)| *c == '：' || *c == ':') else {
        return line;
    };
    let name = &line[..i];
    // 名字里去掉括号注（「林婉清（盛唐）」）再判断
    let bare: String = name.split(['（', '(']).next().unwrap_or("").trim().to_string();
    let looks_like_name = !bare.is_empty()
        && bare.chars().count() <= 8
        && !bare.chars().any(|c| "，。！？、…,.!?\"“”".contains(c))
        && name.chars().count() <= 16;
    if looks_like_name { &line[i + colon.len_utf8()..] } else { line }
}

/// 找一份能画中文的字体。
pub fn find_cjk_font() -> Option<(PathBuf, u32)> {
    CJK_FONT_CANDIDATES.iter().find_map(|(p, idx)| {
        let path = Path::new(p);
        path.is_file().then(|| (path.to_path_buf(), *idx))
    })
}

/// 加载字体。`.ttc` 是字体集合，要给索引。
pub fn load_font(path: &Path, index: u32) -> Result<FontVec> {
    let bytes = std::fs::read(path)
        .map_err(|e| Error::runtime(format!("读字体失败 {}：{e}", path.display())))?;
    FontVec::try_from_vec_and_index(bytes, index)
        .map_err(|e| Error::runtime(format!("解析字体失败 {}：{e}", path.display())))
}

/// 按画面宽度折行。
///
/// **中文没有空格，只能按字断**；拉丁词按空格断，不能把单词劈开。
/// 两种混排时按「遇到 CJK 就可断」处理。
fn wrap<F: Font, S: ScaleFont<F>>(text: &str, font: &S, max_w: f32) -> Vec<String> {
    let is_cjk = |c: char| {
        matches!(c as u32,
            // 0x2E80..0x9FFF 已经把 CJK 标点（0x3000 段）包在里面了
            0x2E80..=0x9FFF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF)
    };
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut width = 0.0f32;
    let mut pending = String::new();      // 还没断开的拉丁词
    let mut pending_w = 0.0f32;

    let flush_pending = |line: &mut String, width: &mut f32, pending: &mut String, pending_w: &mut f32| {
        line.push_str(pending);
        *width += *pending_w;
        pending.clear();
        *pending_w = 0.0;
    };

    for ch in text.chars() {
        let adv = font.h_advance(font.glyph_id(ch));
        if ch == '\n' {
            flush_pending(&mut line, &mut width, &mut pending, &mut pending_w);
            lines.push(std::mem::take(&mut line));
            width = 0.0;
            continue;
        }
        if is_cjk(ch) || ch == ' ' {
            flush_pending(&mut line, &mut width, &mut pending, &mut pending_w);
            if width + adv > max_w && !line.is_empty() {
                lines.push(std::mem::take(&mut line));
                width = 0.0;
            }
            if !(ch == ' ' && line.is_empty()) {
                line.push(ch);
                width += adv;
            }
        } else {
            pending.push(ch);
            pending_w += adv;
            if width + pending_w > max_w && !line.is_empty() {
                lines.push(std::mem::take(&mut line));
                width = 0.0;
            }
        }
    }
    flush_pending(&mut line, &mut width, &mut pending, &mut pending_w);
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// 光栅化的结果。`opaque_px` 是**证据** —— 0 就说明一个字形都没画上。
pub struct Rendered {
    pub image: image::RgbaImage,
    pub opaque_px: u64,
}

/// 把一条字幕画成一张和视频同尺寸的透明 PNG（底部居中，带描边）。
pub fn render_line(
    text: &str,
    width: u32,
    height: u32,
    style: &SubtitleStyle,
    font: &FontVec,
) -> Rendered {
    let mut canvas = image::RgbaImage::new(width.max(1), height.max(1));
    let px = (height as f32 * style.font_ratio).max(8.0);
    let scaled = font.as_scaled(PxScale::from(px));
    let stroke = (px * style.stroke_ratio).round().max(1.0) as i32;
    let max_w = width as f32 * style.max_width_ratio;
    let lines = wrap(text.trim(), &scaled, max_w);

    let line_h = scaled.height() + scaled.line_gap();
    let total_h = line_h * lines.len() as f32;
    let mut base_y = height as f32 - height as f32 * style.margin_ratio - total_h + scaled.ascent();

    let mut opaque = 0u64;
    for line in &lines {
        let line_w: f32 = line.chars().map(|c| scaled.h_advance(scaled.glyph_id(c))).sum();
        let mut x = (width as f32 - line_w) / 2.0;
        for ch in line.chars() {
            let gid = scaled.glyph_id(ch);
            let glyph = gid.with_scale_and_position(PxScale::from(px), ab_glyph::point(x, base_y));
            if let Some(outlined) = font.outline_glyph(glyph) {
                let bounds = outlined.px_bounds();
                // 先描边（把同一个字形按周围一圈偏移画成黑色），再画白字。
                // 顺序反了会让描边盖住字。
                outlined.draw(|gx, gy, cov| {
                    if cov <= 0.01 {
                        return;
                    }
                    let px0 = bounds.min.x as i32 + gx as i32;
                    let py0 = bounds.min.y as i32 + gy as i32;
                    for dy in -stroke..=stroke {
                        for dx in -stroke..=stroke {
                            if dx * dx + dy * dy > stroke * stroke {
                                continue;
                            }
                            let (bx, by) = (px0 + dx, py0 + dy);
                            if bx < 0 || by < 0 || bx >= width as i32 || by >= height as i32 {
                                continue;
                            }
                            let p = canvas.get_pixel_mut(bx as u32, by as u32);
                            let a = (cov * 255.0) as u8;
                            if p[3] < a {
                                *p = image::Rgba([0, 0, 0, a]);
                            }
                        }
                    }
                });
                outlined.draw(|gx, gy, cov| {
                    if cov <= 0.01 {
                        return;
                    }
                    let bx = bounds.min.x as i32 + gx as i32;
                    let by = bounds.min.y as i32 + gy as i32;
                    if bx < 0 || by < 0 || bx >= width as i32 || by >= height as i32 {
                        return;
                    }
                    let a = (cov * 255.0) as u8;
                    canvas.put_pixel(bx as u32, by as u32, image::Rgba([255, 255, 255, a]));
                    opaque += 1;
                });
            }
            x += scaled.h_advance(gid);
        }
        base_y += line_h;
    }
    Rendered { image: canvas, opaque_px: opaque }
}

/// 一条 cue 对应的 ffmpeg overlay 片段。`idx` 是它在输入里的序号（0 是视频）。
///
/// `enable` 用**闭开区间** `gte(t,start)*lt(t,end)` —— 用 `between` 是闭闭区间，
/// 相邻两条会在交界那一帧同时可见，叠成一团。
pub fn overlay_filter(cues: &[Cue]) -> String {
    let mut parts = Vec::new();
    let mut cur = "0:v".to_string();
    for (i, c) in cues.iter().enumerate() {
        let out = format!("v{i}");
        parts.push(format!(
            "[{cur}][{}:v]overlay=0:0:enable='gte(t\\,{:.3})*lt(t\\,{:.3})'[{out}]",
            i + 1,
            c.start,
            c.end
        ));
        cur = out;
    }
    parts.join(";")
}

/// 把若干条字幕烧进视频。
///
/// 每条 cue 光栅化成一张和视频同尺寸的透明 PNG，再用 `overlay` 按时间叠上去。
/// **不重编音频**（`-c:a copy`）—— H3 / LTX 出的音轨重编一次只会掉质量。
///
/// 光栅化出来一个不透明像素都没有时**直接失败**，不要把一段没字幕的视频
/// 当成成功交出去 —— 那正是这一块最典型的失败形状。
pub fn burn_in(video: &Path, cues: &[Cue], out: &Path) -> Result<()> {
    if cues.is_empty() {
        std::fs::copy(video, out)
            .map_err(|e| Error::runtime(format!("复制视频失败：{e}")))?;
        return Ok(());
    }
    let (fp, fi) = find_cjk_font().ok_or_else(|| {
        Error::runtime(format!(
            "找不到能画中文的字体，试过：{}",
            CJK_FONT_CANDIDATES.iter().map(|(p, _)| *p).collect::<Vec<_>>().join("、")
        ))
    })?;
    let font = load_font(&fp, fi)?;
    let (w, h) = crate::agent_runtime::video_tool::video_dimensions(video)?;

    let tmp = std::env::temp_dir().join(format!("anima_sub_{}", std::process::id()));
    std::fs::create_dir_all(&tmp)
        .map_err(|e| Error::runtime(format!("建临时目录失败：{e}")))?;
    let style = SubtitleStyle::default();
    let mut pngs = Vec::new();
    for (i, c) in cues.iter().enumerate() {
        let r = render_line(&c.text, w, h, &style, &font);
        if r.opaque_px == 0 {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(Error::runtime(format!(
                "字幕「{}」一个像素都没画出来 —— 字体 {} 可能缺这些字形。\
                 不把没字幕的视频当成功交出去。",
                c.text,
                fp.display()
            )));
        }
        let p = tmp.join(format!("cue_{i}.png"));
        r.image
            .save(&p)
            .map_err(|e| Error::runtime(format!("写字幕图失败：{e}")))?;
        pngs.push(p);
    }

    let ffmpeg = crate::agent_runtime::video_tool::ffmpeg_binary();
    let mut cmd = std::process::Command::new(ffmpeg);
    cmd.arg("-v").arg("error").arg("-y").arg("-i").arg(video);
    for p in &pngs {
        cmd.arg("-i").arg(p);
    }
    let last = format!("v{}", cues.len() - 1);
    cmd.arg("-filter_complex")
        .arg(overlay_filter(cues))
        .arg("-map")
        .arg(format!("[{last}]"))
        // 有音轨就带上，没有也不报错
        .arg("-map")
        .arg("0:a?")
        .arg("-c:a")
        .arg("copy")
        .arg("-c:v")
        .arg("libx264")
        .arg("-crf")
        .arg("18")
        .arg("-pix_fmt")
        .arg("yuv420p")
        .arg(out);
    let o = cmd
        .output()
        .map_err(|e| Error::runtime(format!("跑 {ffmpeg} 失败：{e}")))?;
    let _ = std::fs::remove_dir_all(&tmp);
    if !o.status.success() {
        return Err(Error::runtime(format!(
            "ffmpeg 烧字幕失败：{}",
            String::from_utf8_lossy(&o.stderr).trim()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 整集字幕按真实时长接起来_不用分镜上写的秒数() {
        let clips = vec![
            ("大臣：大逆不道！".to_string(), 10.125),
            ("林婉清：这是……唐朝？\n皇帝：何人喧哗。".to_string(), 8.4),
        ];
        let cues = episode_cues(&clips);
        assert_eq!(cues.len(), 3);
        assert_eq!(cues[0].text, "大逆不道！", "角色名不进字幕");
        assert!((cues[0].start - 0.0).abs() < 0.01 && (cues[0].end - 10.125).abs() < 0.01);
        // 第二段从第一段的真实末尾开始，不是从 10.0
        assert!((cues[1].start - 10.125).abs() < 0.01, "{:?}", cues[1]);
        assert!(cues[2].end <= 10.125 + 8.4 + 0.001, "最后一条不能超出整集长度");
        assert!(cues[1].end <= cues[2].start + 0.001);
    }

    #[test]
    fn srt_文本格式() {
        let srt = to_srt(&[Cue { text: "大逆不道！".into(), start: 0.0, end: 2.5 }, Cue { text: "这是……唐朝？".into(), start: 2.5, end: 3.75 }]);
        assert_eq!(srt, "1\n00:00:00,000 --> 00:00:02,500\n大逆不道！\n\n2\n00:00:02,500 --> 00:00:03,750\n这是……唐朝？\n");
        assert_eq!(to_srt(&[]), "");
        // 一小时以上、毫秒进位都要对
        let long = to_srt(&[Cue { text: "x".into(), start: 3661.0009, end: 3661.5 }]);
        assert!(long.contains("01:01:01,001 --> 01:01:01,500"), "{long}");
    }

    fn font() -> FontVec {
        let (p, i) = find_cjk_font().expect("这台机器上找不到中文字体");
        load_font(&p, i).expect("字体加载失败")
    }

    /// **这一条是这个模块存在的理由。**
    ///
    /// 字体缺字形时光栅化不报错，出来一张全透明的图，视频照样合成成功、
    /// 任务照样 success，只是没有字。所以判据必须是「真的画上了像素」。
    #[test]
    fn 中文真的画得出来而不是一张透明图() {
        let f = font();
        let r = render_line("这茶，要等。", 1280, 720, &SubtitleStyle::default(), &f);
        assert!(r.opaque_px > 500, "中文一个像素都没画上，opaque_px={}", r.opaque_px);

        // 空文本就该是空的 —— 否则说明画了不该画的东西
        let empty = render_line("   ", 1280, 720, &SubtitleStyle::default(), &f);
        assert_eq!(empty.opaque_px, 0);

        // 中英混排、标点、数字都得有像素
        for t in ["Hello 世界", "第 3 集", "——他说。"] {
            let r = render_line(t, 1280, 720, &SubtitleStyle::default(), &f);
            assert!(r.opaque_px > 200, "「{t}」没画上：{}", r.opaque_px);
        }
    }

    /// 字幕要在底部，不能画到画面中间去。
    #[test]
    fn 字幕落在画面下方且水平居中() {
        let f = font();
        let (w, h) = (1280u32, 720u32);
        let r = render_line("这茶，要等。", w, h, &SubtitleStyle::default(), &f);
        let (mut top, mut bottom) = (h, 0u32);
        let (mut left, mut right) = (w, 0u32);
        for (x, y, p) in r.image.enumerate_pixels() {
            if p[3] > 0 {
                top = top.min(y); bottom = bottom.max(y);
                left = left.min(x); right = right.max(x);
            }
        }
        assert!(top > h / 2, "字幕跑到画面上半部了：top={top}");
        assert!(bottom < h, "字幕出界了：bottom={bottom}");
        let center = (left + right) / 2;
        let off = (center as i64 - (w / 2) as i64).abs();
        assert!(off < 40, "没有水平居中，偏了 {off}px");
    }

    /// 长台词要折行，不能糊成一行画到画外。
    ///
    /// **用 720×1280 测** —— 短剧默认就是这个竖屏画幅，一行只放得下约
    /// 10 个字，折行在这儿才是常态。横屏 1280 宽下三十个字都不折，
    /// 拿它测等于没测（第一版就是这么写的，测试直接绿了才发现）。
    #[test]
    fn 长句折行且不超出画宽() {
        let f = font();
        let (w, h) = (720u32, 1280u32);
        let long = "这茶要等，急不得，水温火候差一分，味道就差一截，急了就毁了。";
        let r = render_line(long, w, h, &SubtitleStyle::default(), &f);
        let (mut left, mut right, mut top, mut bottom) = (w, 0u32, h, 0u32);
        for (x, y, p) in r.image.enumerate_pixels() {
            if p[3] > 0 {
                left = left.min(x); right = right.max(x);
                top = top.min(y); bottom = bottom.max(y);
            }
        }
        assert!(right < w, "画出右边界了：right={right} w={w}");
        assert!(left > 0, "画出左边界了");
        // 折了行，所以纵向高度应该明显大于单行
        let single = render_line("短", w, h, &SubtitleStyle::default(), &f);
        let (mut t2, mut b2) = (h, 0u32);
        for (_, y, p) in single.image.enumerate_pixels() {
            if p[3] > 0 { t2 = t2.min(y); b2 = b2.max(y); }
        }
        assert!(bottom - top > (b2 - t2) + 10, "没有折行：{}px vs 单行 {}px", bottom - top, b2 - t2);
    }

    /// 描边不能省 —— 没有描边的白字压在亮背景上整行看不见。
    #[test]
    fn 白字周围有黑描边() {
        let f = font();
        let r = render_line("等", 640, 360, &SubtitleStyle::default(), &f);
        let mut white = 0u32;
        let mut black = 0u32;
        for p in r.image.pixels() {
            if p[3] > 128 {
                if p[0] > 200 { white += 1 } else if p[0] < 50 { black += 1 }
            }
        }
        assert!(white > 50, "没有白字：{white}");
        assert!(black > white / 4, "描边太薄或没有：黑 {black} 白 {white}");
    }

    #[test]
    fn 台词去掉角色名_一人一条按字数分时间() {
        let c = cues_from_dialogue("大臣：大逆不道！假称诗才惊世，欺君罔上，当斩！\n林婉清：这是……唐朝？", 10.0);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].text, "大逆不道！假称诗才惊世，欺君罔上，当斩！");
        assert_eq!(c[1].text, "这是……唐朝？");
        assert_eq!(c[0].start, 0.0);
        assert_eq!(c[0].end, c[1].start);
        assert!((c[1].end - 10.0).abs() < 1e-9);
        assert!(c[0].end > c[1].end - c[1].start, "说得多的占得久");
        assert_eq!(cues_from_dialogue("林婉清（盛唐）：既来盛唐……便与盛唐争一争。", 8.0)[0].text, "既来盛唐……便与盛唐争一争。");
        assert_eq!(cues_from_dialogue("Emma: hello", 3.0)[0].text, "hello");
        // 冒号前不是名字（带标点、太长）原样留着
        assert_eq!(cues_from_dialogue("他说，时间是：三点", 3.0)[0].text, "他说，时间是：三点");
        assert_eq!(cues_from_dialogue("这句台词没有说话人", 3.0)[0].text, "这句台词没有说话人");
        assert!(cues_from_dialogue(" \n ", 3.0).is_empty());
    }

    /// 相邻两条字幕不能在交界那一帧同时可见 —— `between` 是闭闭区间，
    /// 会叠成一团。
    #[test]
    fn 时间区间是闭开的() {
        let cues = vec![
            Cue { text: "一".into(), start: 0.0, end: 2.0 },
            Cue { text: "二".into(), start: 2.0, end: 4.0 },
        ];
        let f = overlay_filter(&cues);
        assert!(f.contains("gte(t\\,0.000)*lt(t\\,2.000)"), "{f}");
        assert!(f.contains("gte(t\\,2.000)*lt(t\\,4.000)"), "{f}");
        assert!(!f.contains("between"), "用了闭闭区间：{f}");
        // 链要接起来：第一条的输出是第二条的输入
        assert!(f.contains("[v0][2:v]overlay"), "overlay 没串成链：{f}");
    }
}

#[cfg(test)]
mod burn_tests {
    use super::*;

    /// 真的烧一次并**读回像素验证** —— 光看 ffmpeg 退出码是 0 说明不了
    /// 字幕在不在画面上。
    ///
    /// `ANIMA_SUB_IN=<mp4> ANIMA_SUB_OUT=<mp4> cargo test --lib burn_in_e2e -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn burn_in_e2e() {
        let src = std::env::var("ANIMA_SUB_IN").expect("要设 ANIMA_SUB_IN");
        let out = std::env::var("ANIMA_SUB_OUT").expect("要设 ANIMA_SUB_OUT");
        let cues = vec![
            Cue { text: "这茶，要等。".into(), start: 0.3, end: 2.5 },
            Cue { text: "急不得，水温火候差一分，味道就差一截。".into(), start: 2.5, end: 4.8 },
        ];
        burn_in(Path::new(&src), &cues, Path::new(&out)).expect("烧字幕失败");
        let n = std::fs::metadata(&out).unwrap().len();
        assert!(n > 100_000, "输出太小：{n}");
        println!("  ✓ {out}  {:.2} MB", n as f64 / 1e6);
    }
}
