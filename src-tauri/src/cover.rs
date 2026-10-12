//! 项目封面：读项目的名字、简介、剧本开头、主要角色和风格，自动出一张封面。
//!
//! # 为什么要有
//!
//! 工作区的项目卡以前拿「第一集第一个有首帧的镜头」当封面（`cinema::pick_project_cover`）。
//! 分镜重构之后出片走参考生视频、不再出首帧，于是 2026-09-15 用户看到两个项目卡都只剩渐变色。
//! 用户要的是：卡片「⋮」里点「生成封面」，提示词按项目里的剧本自动写。
//!
//! # 怎么出
//!
//! 1. 本机后台模型读项目信息，写一句中文海报提示词（槽位 `CoverPoster`，用户可在设置里改）
//! 2. 模型没开 / 没写出来 → 按模板拼一句，**不拦着出图**
//! 3. Z-Image 出图：它是唯一能把中文片名一字不差画进画面的引擎（CLAUDE.md 那张表 6/6）
//! 4. 存进项目目录 `cover_<时间>.png`，写回 `project.json` 的 `cover_image`，旧的封面文件删掉

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// 给模型看的项目信息。剧本只取开头 —— 封面要的是题材、人物、氛围，不是全文。
pub fn cover_brief(title: &str, genre: &str, description: &str, synopsis: &str, script: &str, characters: &[(String, String)], style: &str) -> String {
    let mut lines = vec![format!("片名：{}", title.trim())];
    for (label, v) in [("题材", genre), ("简介", description), ("第一集梗概", synopsis), ("画面风格", style)] {
        if !v.trim().is_empty() {
            lines.push(format!("{label}：{}", v.trim()));
        }
    }
    if !characters.is_empty() {
        lines.push("主要角色：".into());
        for (name, desc) in characters.iter().take(3) {
            let d: String = desc.chars().take(120).collect();
            lines.push(format!("- {name}：{d}"));
        }
    }
    let head: String = script.trim().chars().take(1500).collect();
    if !head.is_empty() {
        lines.push(format!("剧本开头：\n{head}"));
    }
    lines.join("\n")
}

/// 模型没写出来时的兜底：片名画进画面 + 题材 + 主角 + 风格。
/// 第一集的分集目录名。没有分集就回 `None`。
///
/// 封面画的是开场那批人，所以只看第一集 —— 后面几集的角色可能还没出场。
fn first_episode_id(config_dir: &std::path::Path, project_id: &str) -> Option<String> {
    let dir = crate::cinema::cinema_projects_dir(config_dir).join(project_id).join("episodes");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .collect();
    names.sort();
    names.into_iter().next()
}

pub fn fallback_prompt(title: &str, genre: &str, description: &str, characters: &[(String, String)], style: &str) -> String {
    let title = title.trim().trim_matches(|c| c == '《' || c == '》');
    let mut p = format!("电影海报风格的剧集封面，画面上方用醒目的书法大字写着片名「{title}」。");
    if !genre.trim().is_empty() {
        p.push_str(&format!("题材：{}。", genre.trim()));
    }
    if !description.trim().is_empty() {
        let d: String = description.trim().chars().take(80).collect();
        p.push_str(&format!("{d}。"));
    }
    if let Some((name, desc)) = characters.first() {
        let d: String = desc.chars().take(80).collect();
        p.push_str(&format!("主角{name}位于画面中央，{d}。"));
    }
    p.push_str("戏剧性的光影，电影感构图，主体突出，画面精致。");
    if !style.trim().is_empty() {
        p.push_str(style.trim());
    }
    p
}

/// 从模型返回里取出提示词：认 `{"prompt": "..."}`，也认直接一段文字。太短当没写出来。
pub fn parse_cover_prompt(raw: &str) -> Option<String> {
    let text = crate::prompts::extract_json(raw)
        .and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok())
        .and_then(|v| v.get("prompt").and_then(|p| p.as_str()).map(str::to_string))
        .unwrap_or_else(|| raw.trim().to_string());
    let text = text.trim().to_string();
    (text.chars().count() >= 20).then_some(text)
}

/// 封面尺寸跟项目画幅：长边 1024，短边按比例、对齐到 64（出图引擎要 64 的倍数）。
/// 16:9 → 1024×576，9:16 → 576×1024，21:9 → 1024×448。读不出画幅按横屏。
pub fn cover_size(aspect_ratio: &str) -> (u32, u32) {
    crate::cinema::frame_for(aspect_ratio, 1024, 64, (16.0, 9.0))
}

/// 新封面写好之后，删掉项目目录里旧的 `cover_*.png`（只删我们自己出的那种文件名）。
pub fn remove_old_covers(proj_dir: &Path, keep: &Path) {
    let Ok(entries) = std::fs::read_dir(proj_dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if p != keep && name.starts_with("cover_") && name.ends_with(".png") {
            let _ = std::fs::remove_file(&p);
        }
    }
}

/// **上传封面**：用户自己的图当项目封面（2026-09-27 用户要的）。回新封面的本机路径。
///
/// 和生成封面同一套存法：统一转成 `cover_<时间>.png` 存进项目目录、删掉旧的 `cover_*.png`、
/// 写回 `cover_image`。转 PNG 是为了让 `remove_old_covers` 下次换封面时能认出并清掉它。
#[tauri::command]
pub fn cinema_upload_cover(state: tauri::State<'_, crate::AppState>, project_id: String, data_url: String) -> Result<String> {
    use base64::Engine;
    let b64 = data_url.split_once(',').map(|(_, b)| b).unwrap_or(&data_url);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .map_err(|e| Error::config(format!("图片数据读不出来：{e}")))?;
    save_uploaded_cover(&state.config_dir, &project_id, &bytes)
}

/// 最大 30 MB —— 和资产上传一样；再大读成 data URL 会卡界面，封面也用不到。
const MAX_COVER_BYTES: usize = 30 * 1024 * 1024;

pub fn save_uploaded_cover(config_dir: &Path, project_id: &str, bytes: &[u8]) -> Result<String> {
    if bytes.len() > MAX_COVER_BYTES {
        return Err(Error::config(format!("图片有 {} MB，超过 30 MB", bytes.len() / 1024 / 1024)));
    }
    let img = image::load_from_memory(bytes).map_err(|e| Error::config(format!("不是能读的图片（PNG / JPG / WebP）：{e}")))?;
    let proj_dir = crate::cinema::cinema_projects_dir(config_dir).join(project_id);
    let proj_file = proj_dir.join("project.json");
    let raw = std::fs::read_to_string(&proj_file).map_err(|e| Error::config(format!("读不到项目：{e}")))?;
    let mut project: serde_json::Value = serde_json::from_str(&raw).map_err(|e| Error::config(format!("项目文件坏了：{e}")))?;
    let dst = proj_dir.join(format!("cover_{}.png", crate::jobs::now()));
    img.save_with_format(&dst, image::ImageFormat::Png).map_err(|e| Error::runtime(format!("封面存不进项目目录：{e}")))?;
    remove_old_covers(&proj_dir, &dst);
    let dst_str = dst.to_string_lossy().to_string();
    project["cover_image"] = serde_json::Value::String(dst_str.clone());
    std::fs::write(&proj_file, serde_json::to_string_pretty(&project).unwrap_or_default())
        .map_err(|e| Error::runtime(format!("写不回项目文件：{e}")))?;
    Ok(dst_str)
}

/// 生成封面，回新封面的本机路径。
#[tauri::command]
pub async fn cinema_generate_cover(state: tauri::State<'_, crate::AppState>, project_id: String) -> Result<String> {
    generate_cover(&state.config_dir, &state.gateway, &project_id).await
}

pub async fn generate_cover(config_dir: &Path, gateway: &crate::Gateway, project_id: &str) -> Result<String> {
    let proj_dir = crate::cinema::cinema_projects_dir(config_dir).join(project_id);
    let proj_file = proj_dir.join("project.json");
    let raw = std::fs::read_to_string(&proj_file).map_err(|e| Error::config(format!("读不到项目：{e}")))?;
    let mut project: serde_json::Value = serde_json::from_str(&raw).map_err(|e| Error::config(format!("项目文件坏了：{e}")))?;
    let s = |k: &str| project.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let (title, genre, description, aspect) = (s("title"), s("genre"), s("description"), s("aspect_ratio"));

    // 第一集（按目录名排序）
    let mut eps: Vec<PathBuf> = std::fs::read_dir(proj_dir.join("episodes"))
        .map(|r| r.flatten().map(|e| e.path()).filter(|p| p.join("episode.json").is_file()).collect())
        .unwrap_or_default();
    eps.sort();
    let (synopsis, script) = eps
        .first()
        .and_then(|p| std::fs::read_to_string(p.join("episode.json")).ok())
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .map(|v| {
            let g = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
            (g("synopsis"), g("script_text"))
        })
        .unwrap_or_default();

    // **项目层和第一集的分集层都要读。**
    //
    // 刚抽完实体的角色在「本集待归属」里（分集层），还没进系列 ——
    // 只读项目层的话，新项目做封面时一个角色都拿不到，
    // 而提示词又要求写主角外貌，模型只能**凭空编一个**
    // （2026-09-19 Codex Astra 的审查指出）。
    //
    // 第一集优先：封面画的是开场那批人。同名的只留一份（项目层的赢 ——
    // 它是归属好的那一份）。
    let mut characters: Vec<(String, String)> = crate::assets::load_store(config_dir, &crate::assets::Scope::Project { project_id: project_id.into() })
        .assets
        .into_iter()
        .filter(|a| a.kind == crate::assets::AssetKind::Character)
        .map(|a| (a.name, a.description))
        .collect();
    if let Some(first_ep) = first_episode_id(config_dir, project_id) {
        for a in crate::assets::load_store(
            config_dir,
            &crate::assets::Scope::Episode { project_id: project_id.into(), episode_id: first_ep },
        )
        .assets
        .into_iter()
        .filter(|a| a.kind == crate::assets::AssetKind::Character)
        {
            if !characters.iter().any(|(n, _)| n == &a.name) {
                characters.push((a.name, a.description));
            }
        }
    }
    let style = crate::style::load_art(config_dir, project_id).effective_positive();

    let brief = cover_brief(&title, &genre, &description, &synopsis, &script, &characters, &style);
    let cfg = crate::prompts::load(config_dir);
    let prompt = match crate::prompts::run(gateway, &cfg, crate::prompts::PromptSlot::CoverPoster, &[("PROJECT", brief.as_str())]).await {
        Ok(raw) => parse_cover_prompt(&raw),
        Err(e) => {
            tracing::warn!(error = %e, "封面提示词没写出来，用模板");
            None
        }
    }
    .unwrap_or_else(|| fallback_prompt(&title, &genre, &description, &characters, &style));

    let (width, height) = cover_size(&aspect);
    let args: crate::studio::GenerateArgs = serde_json::from_value(serde_json::json!({
        "mode": "t2i", "engine": "zimage", "prompt": prompt, "width": width, "height": height, "auto_loras": false
    }))
    .map_err(|e| Error::runtime(format!("出图参数写错了：{e}")))?;
    let out = crate::studio::generate_with(config_dir, args).await?;
    let src = out.path.filter(|_| out.ok).ok_or_else(|| {
        Error::runtime(if out.message.trim().is_empty() { "封面没出来".to_string() } else { out.message.clone() })
    })?;
    let dst = proj_dir.join(format!("cover_{}.png", crate::jobs::now()));
    std::fs::copy(&src, &dst).map_err(|e| Error::runtime(format!("封面存不进项目目录：{e}")))?;
    remove_old_covers(&proj_dir, &dst);

    let dst_str = dst.to_string_lossy().to_string();
    project["cover_image"] = serde_json::Value::String(dst_str.clone());
    std::fs::write(&proj_file, serde_json::to_string_pretty(&project).unwrap_or_default())
        .map_err(|e| Error::runtime(format!("写不回项目文件：{e}")))?;
    Ok(dst_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 上传的封面：转成 PNG 存进项目目录、旧封面删掉、项目里的 cover_image 指过去；不是图片的拒掉。
    #[test]
    fn 上传封面替换旧的_不是图片拒掉() {
        let d = tempfile::tempdir().unwrap();
        let dir = crate::cinema::cinema_projects_dir(d.path()).join("p1");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("project.json"), r#"{"id":"p1","cover_image":null}"#).unwrap();
        std::fs::write(dir.join("cover_1.png"), b"old").unwrap();
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbImage::new(8, 4).write_to(&mut png, image::ImageFormat::Png).unwrap();
        let path = save_uploaded_cover(d.path(), "p1", png.get_ref()).unwrap();
        assert!(std::path::Path::new(&path).is_file());
        assert!(!dir.join("cover_1.png").exists(), "旧封面要删掉");
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("project.json")).unwrap()).unwrap();
        assert_eq!(v["cover_image"], path);
        assert!(save_uploaded_cover(d.path(), "p1", b"not an image").is_err());
    }

    #[test]
    fn 给模型看的项目信息_空字段不写_剧本只取开头() {
        let chars = vec![("林婉清".to_string(), "二十八岁女子，男装官服".to_string())];
        let script = "字".repeat(3000);
        let b = cover_brief("《我在长安写诗词》", "穿越", "", "", &script, &chars, "电影感写实");
        assert!(b.starts_with("片名：《我在长安写诗词》"));
        assert!(!b.contains("简介") && !b.contains("梗概"));
        assert!(b.contains("- 林婉清：二十八岁女子") && b.contains("画面风格：电影感写实"));
        assert_eq!(b.chars().filter(|c| *c == '字').count(), 1500, "剧本只取开头 1500 字");
    }

    #[test]
    fn 模板兜底_片名画进画面_去书名号() {
        let p = fallback_prompt("《我在长安写诗词》", "穿越", "现代女子穿越盛唐", &[("林婉清".into(), "男装官服".into())], "");
        assert!(p.contains("「我在长安写诗词」") && !p.contains("《"));
        assert!(p.contains("主角林婉清") && p.contains("题材：穿越"));
    }

    #[test]
    fn 模型返回_json_或正文都认_太短不算() {
        assert_eq!(parse_cover_prompt(r#"{"prompt": "电影海报，片名「米小团」写在上方，一只橘猫站在屋檐上"}"#).unwrap(), "电影海报，片名「米小团」写在上方，一只橘猫站在屋檐上");
        assert!(parse_cover_prompt("电影海报风格封面，片名「米小团」，橘猫站在月光下的屋檐").is_some());
        assert!(parse_cover_prompt(r#"{"prompt": "猫"}"#).is_none());
    }

    #[test]
    fn 封面尺寸跟画幅() {
        assert_eq!(cover_size("16:9"), (1024, 576));
        assert_eq!(cover_size("9:16"), (576, 1024));
        assert_eq!(cover_size(""), (1024, 576));
    }

    #[test]
    fn 只删旧的封面文件() {
        let d = tempfile::tempdir().unwrap();
        for n in ["cover_1.png", "cover_2.png", "project.json", "cover_note.txt"] {
            std::fs::write(d.path().join(n), b"x").unwrap();
        }
        remove_old_covers(d.path(), &d.path().join("cover_2.png"));
        let mut left: Vec<_> = std::fs::read_dir(d.path()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
        left.sort();
        assert_eq!(left, ["cover_2.png", "cover_note.txt", "project.json"]);
    }
}
