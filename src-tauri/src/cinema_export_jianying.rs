//! 剪映 (CapCut / JianYing) 草稿工程一键导出器
//!
//! 将当前分集已渲染的影视分镜、对白配音音频以及字幕直接映射为剪映工程结构
//! (draft_content.json + draft_meta_info.json)。
//! 用户将导出文件夹放入剪映草稿目录（如 ~/Movies/JianyingPro/User Data/Projects/com.lveditor.draft/）
//! 即可在剪映中一键打开并进行二次精剪。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const US: u64 = 1_000_000;

fn to_us(sec: f64) -> u64 {
    (sec.max(0.0) * US as f64).round() as u64
}

fn gen_id(kind: &str, idx: usize) -> String {
    let mut h: u64 = 5381;
    for b in kind.bytes() {
        h = h.wrapping_shl(5).wrapping_add(h).wrapping_add(b as u64);
    }
    let hex = format!("{:012x}", (h.wrapping_add(idx as u64)) & 0xffffffffffff);
    format!("00000000-0000-4000-8000-{hex}")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JyClipInput {
    pub name: String,
    pub path: String,
    pub duration_sec: f64,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JyAudioInput {
    pub name: String,
    pub path: String,
    pub start_sec: f64,
    pub duration_sec: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JySubtitleInput {
    pub text: String,
    pub start_sec: f64,
    pub duration_sec: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JianYingDraftInput {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub clips: Vec<JyClipInput>,
    pub voiceovers: Vec<JyAudioInput>,
    pub bgm_path: Option<String>,
    pub subtitles: Vec<JySubtitleInput>,
}

pub fn build_jianying_draft(input: &JianYingDraftInput) -> Value {
    let mut videos = Vec::new();
    let mut audios = Vec::new();
    let mut texts = Vec::new();
    let mut video_segments = Vec::new();

    let mut cursor_us: u64 = 0;
    for (i, c) in input.clips.iter().enumerate() {
        let mat_id = gen_id("video", i);
        let dur_us = to_us(c.duration_sec.max(1.0));
        videos.push(json!({
            "id": mat_id,
            "type": "video",
            "material_name": c.name,
            "path": c.path,
            "duration": dur_us,
            "width": c.width.unwrap_or(input.width),
            "height": c.height.unwrap_or(input.height),
        }));
        video_segments.push(json!({
            "id": gen_id("vseg", i),
            "material_id": mat_id,
            "target_timerange": { "start": cursor_us, "duration": dur_us },
            "source_timerange": { "start": 0, "duration": dur_us },
            "speed": 1.0,
            "volume": 1.0,
            "visible": true,
            "render_index": i,
            "clip": { "alpha": 1.0 }
        }));
        cursor_us += dur_us;
    }
    let total_us = cursor_us;

    let mut audio_segments = Vec::new();
    for (i, a) in input.voiceovers.iter().enumerate() {
        let mat_id = gen_id("audio", i);
        let dur_us = to_us(a.duration_sec.max(0.1));
        audios.push(json!({
            "id": mat_id,
            "type": "extract_music",
            "name": a.name,
            "path": a.path,
            "duration": dur_us
        }));
        audio_segments.push(json!({
            "id": gen_id("aseg", i),
            "material_id": mat_id,
            "target_timerange": { "start": to_us(a.start_sec), "duration": dur_us },
            "source_timerange": { "start": 0, "duration": dur_us },
            "speed": 1.0,
            "volume": 1.0
        }));
    }

    if let Some(ref bgm) = input.bgm_path {
        let mat_id = gen_id("bgm", 0);
        audios.push(json!({
            "id": mat_id,
            "type": "music",
            "name": "BGM",
            "path": bgm,
            "duration": total_us
        }));
    }

    let mut text_segments = Vec::new();
    for (i, s) in input.subtitles.iter().enumerate() {
        let mat_id = gen_id("text", i);
        texts.push(json!({
            "id": mat_id,
            "type": "text",
            "content": s.text,
            "font_size": 8.0
        }));
        text_segments.push(json!({
            "id": gen_id("tseg", i),
            "material_id": mat_id,
            "target_timerange": { "start": to_us(s.start_sec), "duration": to_us(s.duration_sec) },
            "render_index": i
        }));
    }

    let mut tracks = vec![json!({
        "type": "video",
        "attribute": 0,
        "flag": 0,
        "segments": video_segments
    })];

    if !audio_segments.is_empty() {
        tracks.push(json!({
            "type": "audio",
            "attribute": 0,
            "flag": 0,
            "segments": audio_segments
        }));
    }

    if !text_segments.is_empty() {
        tracks.push(json!({
            "type": "text",
            "attribute": 0,
            "flag": 0,
            "segments": text_segments
        }));
    }

    json!({
        "id": gen_id("draft", 0),
        "name": input.name,
        "duration": total_us,
        "fps": input.fps,
        "canvas_config": {
            "width": input.width,
            "height": input.height,
            "ratio": "original"
        },
        "platform": { "os": "mac", "app_version": "5.9.0" },
        "version": 360000,
        "materials": {
            "videos": videos,
            "audios": audios,
            "texts": texts,
            "stickers": [],
            "effects": [],
            "transitions": []
        },
        "tracks": tracks
    })
}

pub fn build_jianying_meta(name: &str, draft_id: &str, total_us: u64) -> Value {
    json!({
        "draft_id": draft_id,
        "draft_name": name,
        "draft_fold_path": "",
        "tm_duration": total_us,
        "draft_removable": true
    })
}

/// 将当前短剧分集导出为完整的剪映草稿工程目录
pub fn export_episode_to_jianying(
    target_dir: &Path,
    draft_input: &JianYingDraftInput,
) -> std::io::Result<PathBuf> {
    let project_dir = target_dir.join(&draft_input.name);
    std::fs::create_dir_all(&project_dir)?;

    let draft_content = build_jianying_draft(draft_input);
    let draft_id = draft_content["id"].as_str().unwrap_or("draft_001");
    let total_us = draft_content["duration"].as_u64().unwrap_or(0);
    let draft_meta = build_jianying_meta(&draft_input.name, draft_id, total_us);

    std::fs::write(
        project_dir.join("draft_content.json"),
        serde_json::to_string_pretty(&draft_content).unwrap_or_default(),
    )?;

    std::fs::write(
        project_dir.join("draft_meta_info.json"),
        serde_json::to_string_pretty(&draft_meta).unwrap_or_default(),
    )?;

    Ok(project_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draft_json_structure_matches_schema() {
        let input = JianYingDraftInput {
            name: "第1集_由美子归来".into(),
            width: 1080,
            height: 1920,
            fps: 30,
            clips: vec![JyClipInput {
                name: "shot_1".into(),
                path: "/path/to/shot_1.png".into(),
                duration_sec: 4.0,
                width: Some(1080),
                height: Some(1920),
            }],
            voiceovers: vec![JyAudioInput {
                name: "vo_1".into(),
                path: "/path/to/vo_1.mp3".into(),
                start_sec: 0.5,
                duration_sec: 3.0,
            }],
            bgm_path: None,
            subtitles: vec![JySubtitleInput {
                text: "由美子：“属于我的，我将亲手拿回。”".into(),
                start_sec: 0.5,
                duration_sec: 3.0,
            }],
        };

        let draft = build_jianying_draft(&input);
        assert_eq!(draft["name"], "第1集_由美子归来");
        assert_eq!(draft["duration"], 4_000_000);
        assert_eq!(draft["canvas_config"]["width"], 1080);
        assert_eq!(draft["tracks"].as_array().unwrap().len(), 3);
    }
}
