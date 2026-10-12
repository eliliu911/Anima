//! Studio 的领域操作：建项目、建分集、写剧本、建资产、设风格。
//!
//! **Studio 页面的按钮和 Anima Chat 的工具共用这一份。** 命令（`#[tauri::command]`）带 `State`，
//! 工具里没有；而且逻辑放在这里才测得到。方案见 `docs/plans/2026-09-15-Chat接Studio方案.md` 第 0 批第 2 步。
//!
//! 每个函数只做一件用户说得出口的事，返回给人（和模型）看的精简结果。

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::assets::{load_store, save_store, Asset, AssetKind, Scope};
use crate::cinema::{cinema_projects_dir, CinemaProject, CreateProjectReq, EpisodeData};
use crate::error::{Error, Result};

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn read_json(file: &Path) -> Result<Value> {
    let text = std::fs::read_to_string(file).map_err(|e| Error::config(format!("读不到 {}：{e}", file.display())))?;
    serde_json::from_str(&text).map_err(|e| Error::config(format!("{} 坏了：{e}", file.display())))
}

fn write_json(file: &Path, v: &impl Serialize) -> Result<()> {
    if let Some(p) = file.parent() {
        std::fs::create_dir_all(p).map_err(|e| Error::runtime(format!("建目录失败：{e}")))?;
    }
    std::fs::write(file, serde_json::to_string_pretty(v).unwrap_or_default()).map_err(|e| Error::runtime(format!("写 {} 失败：{e}", file.display())))
}

fn blank_episode(num: u32, title: &str, synopsis: &str) -> EpisodeData {
    EpisodeData {
        long_take_scenes: Vec::new(),
        one_take: false,
        scene_takes: Vec::new(),
        long_take_context_frames: None,
        scene_renders: Vec::new(),
        id: format!("ep_{num:02}"),
        // 空 = 跟项目的风格走（绝大多数情况）。锁住某一集时才填
        style_lock: String::new(),
        episode_num: num,
        title: if title.trim().is_empty() { format!("第 {num} 集") } else { title.trim().to_string() },
        synopsis: synopsis.trim().to_string(),
        script_text: String::new(),
        script_doc: None,
        target_secs: 0,
        storyboard_version: crate::cinema::STORYBOARD_VERSION,
        actor_colors: Vec::new(),
        storyboard: Vec::new(),
        final_video_url: None,
    }
}

/// 建项目（带一个空的第 1 集）。
///
/// id 用秒级时间戳 —— 同一秒建两个会撞，所以撞了就往后挪一秒。
pub fn create_project(config_dir: &Path, req: CreateProjectReq) -> Result<CinemaProject> {
    let title = req.title.trim().to_string();
    if title.is_empty() {
        return Err(Error::config("项目得有个名字"));
    }
    let root = cinema_projects_dir(config_dir);
    let mut stamp = now_secs();
    while root.join(format!("drama_{stamp}")).exists() {
        stamp += 1;
    }
    let id = format!("drama_{stamp}");
    let proj_dir = root.join(&id);
    std::fs::create_dir_all(proj_dir.join("episodes/ep_01/shots")).map_err(|e| Error::config(format!("无法创建项目目录：{e}")))?;
    let now = now_secs();
    let project = CinemaProject {
        default_looks: Default::default(),
        script_project_id: None,
        id,
        title,
        genre: req.genre.trim().to_string(),
        aspect_ratio: req.aspect_ratio.filter(|a| !a.trim().is_empty()).unwrap_or_else(|| "9:16".to_string()),
        description: req.description.trim().to_string(),
        created_at: now,
        updated_at: now,
        episode_count: 1,
        cover_image: None,
        kind: req.kind.filter(|k| k == "series" || k == "standalone").unwrap_or_else(|| "series".into()),
        content_mode: req.content_mode.unwrap_or_else(|| "scripted".into()),
        model_settings: Default::default(),
        default_shot_mode: req.default_shot_mode.unwrap_or_default(),
    };
    write_json(&proj_dir.join("project.json"), &project)?;
    // **新项目是空的**：剧本从第 1 步写或导入，分镜由剧本生成
    crate::episode_lock::put(&proj_dir.join("episodes/ep_01/episode.json"), &blank_episode(1, "", ""))?;
    Ok(project)
}

/// 系列里加一集（空的，不带样板内容）。
pub fn create_episode(config_dir: &Path, project_id: &str, title: &str, synopsis: &str) -> Result<EpisodeData> {
    let proj_dir = cinema_projects_dir(config_dir).join(project_id);
    if !proj_dir.join("project.json").is_file() {
        return Err(Error::config(format!("找不到项目「{project_id}」")));
    }
    let next = episode_files(config_dir, project_id)
        .iter()
        .filter_map(|f| read_json(f).ok())
        .filter_map(|v| v["episode_num"].as_u64())
        .max()
        .unwrap_or(0) as u32
        + 1;
    let ep = blank_episode(next, title, synopsis);
    let dir = proj_dir.join("episodes").join(&ep.id);
    std::fs::create_dir_all(dir.join("shots")).map_err(|e| Error::config(format!("创建分集目录失败：{e}")))?;
    crate::episode_lock::put(&dir.join("episode.json"), &ep)?;
    let pf = proj_dir.join("project.json");
    if let Ok(mut p) = read_json(&pf) {
        p["episode_count"] = Value::from(next);
        p["updated_at"] = Value::from(now_secs());
        let _ = write_json(&pf, &p);
    }
    Ok(ep)
}

/// 一个项目所有分集的 episode.json，按目录名排
pub fn episode_files(config_dir: &Path, project_id: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(cinema_projects_dir(config_dir).join(project_id).join("episodes"))
        .map(|r| r.flatten().map(|e| e.path().join("episode.json")).filter(|p| p.is_file()).collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// 剧本格式检查的结果。**认不出不拦**，说出来让人决定改不改。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ScriptReport {
    pub chars: usize,
    /// 认出的场标题（【场景：…】 / 内景. / INT.）
    pub scenes: Vec<String>,
    /// 「角色：台词」里认出的说话人（去重，按出现顺序）
    pub speakers: Vec<String>,
    pub problems: Vec<String>,
}

/// 检查剧本格式：场标题、台词行。规则和前端 `script/format.ts` 同一套（场标题三种写法、「角色：台词」限长）。
pub fn check_script(text: &str) -> ScriptReport {
    let mut scenes = Vec::new();
    let mut speakers: Vec<String> = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let is_bracket = line.starts_with('【') && ["场景", "地点", "场"].iter().any(|k| line.trim_start_matches('【').trim_start().starts_with(k));
        let is_cn = ["内景", "外景", "内外景"].iter().any(|k| line.starts_with(k));
        let upper = line.to_ascii_uppercase();
        let is_west = upper.starts_with("INT.") || upper.starts_with("EXT.") || upper.starts_with("INT ") || upper.starts_with("EXT ");
        if is_bracket || is_cn || is_west {
            scenes.push(line.trim_matches(|c| c == '【' || c == '】').to_string());
            continue;
        }
        if let Some((name, rest)) = line.split_once('：').or_else(|| line.split_once(':')) {
            let name = name.trim();
            let bare = name.split(['（', '(']).next().unwrap_or("").trim();
            if !bare.is_empty() && bare.chars().count() <= 8 && !rest.trim().is_empty() && !bare.chars().any(|c| "，。！？、…,.!?【】".contains(c)) && !speakers.iter().any(|s| s == name) {
                speakers.push(name.to_string());
            }
        }
    }
    let mut problems = Vec::new();
    if text.trim().is_empty() {
        problems.push("剧本是空的".into());
    } else {
        if scenes.is_empty() {
            problems.push("没认出场标题。每一场开头写一行「【场景：地点 · 时间】」，分镜才能按场生成".into());
        }
        if speakers.is_empty() {
            problems.push("没认出台词行。台词写成「角色名：台词」，一人一行".into());
        }
    }
    ScriptReport { chars: text.chars().count(), scenes, speakers, problems }
}

/// 把剧本写进某一集。清掉编辑器的结构化文档（`script_doc`），让编辑器按新文本重新排版。
/// **分集已经有分镜时不覆盖**，除非 `replace`：剧本换了，旧分镜对不上。
pub fn import_script(config_dir: &Path, project_id: &str, episode_id: &str, text: &str, replace: bool) -> Result<ScriptReport> {
    let file = cinema_projects_dir(config_dir).join(project_id).join("episodes").join(episode_id).join("episode.json");
    if read_json(&file).is_err() {
        return Err(Error::config(format!("找不到分集「{episode_id}」")));
    }
    // 「有没有剧本 / 分镜」要和写在同一把锁里判断 —— 判完到写之间分镜刚生成好的话，照样会被盖掉
    crate::episode_lock::update_json(&file, |ep| {
        let has_script = ep["script_text"].as_str().is_some_and(|s| !s.trim().is_empty());
        let has_board = ep["storyboard"].as_array().is_some_and(|a| !a.is_empty());
        if (has_script || has_board) && !replace {
            return Err(Error::config(format!(
                "这一集已经有{}。确定要换掉就传 replace=true（{}）",
                if has_board { "剧本和分镜" } else { "剧本" },
                if has_board { "旧分镜会对不上新剧本，要重新生成" } else { "旧剧本会被覆盖" }
            )));
        }
        ep["script_text"] = Value::String(text.to_string());
        ep["script_doc"] = Value::Null;
        Ok(())
    })?;
    Ok(check_script(text))
}

/// 要建的一个资产
#[derive(Debug, Clone, serde::Deserialize)]
pub struct NewAsset {
    /// character / scene / prop
    pub kind: String,
    pub name: String,
    /// 场景道具的描述；角色的主形象描述（服装发型）
    #[serde(default)]
    pub description: String,
    /// 角色的人物本身（脸、身材、年龄）。场景道具忽略
    #[serde(default)]
    pub identity: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CreatedAsset {
    pub id: String,
    pub kind: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct CreateAssetsReport {
    pub created: Vec<CreatedAsset>,
    /// 同一个地方已经有同名同类的，没建（带已有的 id）
    pub skipped: Vec<CreatedAsset>,
}

pub fn parse_kind(kind: &str) -> Result<AssetKind> {
    match kind.trim() {
        "character" | "角色" => Ok(AssetKind::Character),
        "scene" | "场景" => Ok(AssetKind::Scene),
        "prop" | "道具" => Ok(AssetKind::Prop),
        other => Err(Error::config(format!("不认识的资产种类「{other}」，只有 character / scene / prop"))),
    }
}

/// 批量建资产。**同一个作用域里已有同名同类的跳过**（模型重试、用户说两遍都不会建出第二个）。
pub fn create_assets(config_dir: &Path, scope: &Scope, items: &[NewAsset]) -> Result<CreateAssetsReport> {
    let mut store = load_store(config_dir, scope);
    let mut report = CreateAssetsReport::default();
    for it in items {
        let kind = parse_kind(&it.kind)?;
        let name = it.name.trim();
        if name.is_empty() {
            return Err(Error::config("有一个资产没写名字"));
        }
        let entry = |id: &str| CreatedAsset { id: id.to_string(), kind: kind.id_prefix().to_string(), name: name.to_string() };
        if let Some(a) = store.assets.iter().find(|a| a.kind == kind && a.name.trim() == name) {
            report.skipped.push(entry(&a.id));
            continue;
        }
        let mut a = Asset::new(kind, name);
        a.description = it.description.trim().to_string();
        if kind == AssetKind::Character {
            a.identity = it.identity.trim().to_string();
        }
        report.created.push(entry(&a.id));
        store.assets.push(a);
    }
    save_store(config_dir, scope, &store)?;
    Ok(report)
}

/// 设项目风格：选一个内置预设，或者写自定义风格词（两个都给时自定义词追加在预设后面）。
pub fn set_style(config_dir: &Path, project_id: &str, preset_id: Option<&str>, positive: Option<&str>) -> Result<crate::style::ArtDirection> {
    let mut art = crate::style::load_art(config_dir, project_id);
    if let Some(id) = preset_id.map(str::trim).filter(|s| !s.is_empty()) {
        if crate::style::find_preset(id).is_none() {
            let ids: Vec<String> = crate::style::all_presets().iter().map(|p| format!("{}（{}）", p.id, p.name_zh)).collect();
            return Err(Error::config(format!("没有风格「{id}」。可选：{}", ids.join("、"))));
        }
        art.preset_id = id.to_string();
    }
    if let Some(p) = positive {
        art.positive_prompt = p.trim().to_string();
    }
    crate::style::save_art(config_dir, project_id, &art)?;
    Ok(crate::style::load_art(config_dir, project_id))
}

/// Studio 数据被 Studio 页面以外的地方（Chat 的工具）改了。页面听 `studio://changed` 重新读。
pub const CHANGED_EVENT: &str = "studio://changed";

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct StudioChange {
    /// project / episode / assets / style / script_doc
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
}

/// 删掉系列里的一集。**不可逆**：剧本、分镜、关键帧、视频、这一集自己的资产都在分集目录里，一起删。
///
/// 和删项目一样两步：不带 `confirm` 只回一句「里面有什么」，界面说清楚之后再带 `confirm` 真删。
/// 拦三种情况：这一集有任务在跑（删了任务会往不存在的目录里写）、
/// 只剩这一集（系列至少留一集，整个不要了去工作区删项目）、路径不在本项目的分集目录里。
/// 其它集**不重新编号**：镜头文件、任务记录、画布节点都按 `ep_xx` 认人，改号会断链。
pub fn delete_episode(config_dir: &Path, project_id: &str, episode_id: &str, confirm: bool) -> Result<String> {
    let base = cinema_projects_dir(config_dir).join(project_id).join("episodes");
    let dir = base.join(episode_id);
    let ep: EpisodeData = read_json(&dir.join("episode.json"))
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .ok_or_else(|| Error::config(format!("找不到分集「{episode_id}」")))?;
    let (root, real) = match (base.canonicalize(), dir.canonicalize()) {
        (Ok(r), Ok(d)) => (r, d),
        _ => return Err(Error::config("解析分集路径失败")),
    };
    if real.parent() != Some(root.as_path()) {
        return Err(Error::config("这个路径不在本项目的分集目录里，拒绝删除"));
    }
    if episode_files(config_dir, project_id).len() <= 1 {
        return Err(Error::config("只剩这一集了，删不了。整个系列都不要的话，去工作区删项目"));
    }
    let busy = crate::jobs::running(config_dir, project_id, episode_id);
    if !busy.is_empty() {
        return Err(Error::config(format!("这一集还有 {} 个任务在跑，等它们结束或取消后再删", busy.len())));
    }

    let rendered = ep.storyboard.iter().filter(|s| s.video_url.is_some()).count();
    let assets = load_store(config_dir, &Scope::Episode { project_id: project_id.into(), episode_id: episode_id.into() }).assets.len();
    let mut parts = vec![format!("{} 镜", ep.storyboard.len())];
    if rendered > 0 {
        parts.push(format!("{rendered} 已出片"));
    }
    if ep.final_video_url.is_some() {
        parts.push("有成片".into());
    }
    if assets > 0 {
        parts.push(format!("{assets} 个本集资产"));
    }
    if !ep.script_text.trim().is_empty() {
        parts.push(format!("剧本 {} 字", ep.script_text.chars().count()));
    }
    let summary = format!("「{}」：{}", ep.title, parts.join(" · "));
    if !confirm {
        return Ok(summary);
    }
    std::fs::remove_dir_all(&real).map_err(|e| Error::config(format!("删除失败：{e}")))?;
    let pf = cinema_projects_dir(config_dir).join(project_id).join("project.json");
    if let Ok(mut p) = read_json(&pf) {
        p["episode_count"] = Value::from(episode_files(config_dir, project_id).len());
        p["updated_at"] = Value::from(now_secs());
        let _ = write_json(&pf, &p);
    }
    tracing::info!(project = %project_id, episode = %episode_id, "删掉了分集：{summary}");
    Ok(summary)
}

pub fn emit_changed(app: &tauri::AppHandle, kind: &'static str, project_id: Option<&str>) {
    use tauri::Emitter;
    let _ = app.emit(CHANGED_EVENT, StudioChange { kind, project_id: project_id.map(String::from) });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(title: &str) -> CreateProjectReq {
        serde_json::from_value(serde_json::json!({ "title": title, "kind": "standalone", "aspect_ratio": "16:9" })).unwrap()
    }

    #[test]
    fn 建项目_带空的第一集_同一秒建两个不撞() {
        let d = tempfile::tempdir().unwrap();
        let a = create_project(d.path(), req("盛唐")).unwrap();
        let b = create_project(d.path(), req("米小团")).unwrap();
        assert_ne!(a.id, b.id);
        assert_eq!((a.kind.as_str(), a.aspect_ratio.as_str()), ("standalone", "16:9"));
        let ep: EpisodeData = serde_json::from_value(read_json(&d.path().join("projects").join(&a.id).join("episodes/ep_01/episode.json")).unwrap()).unwrap();
        assert!(ep.storyboard.is_empty() && ep.script_text.is_empty(), "新项目不带样板内容");
        assert!(create_project(d.path(), req("  ")).is_err());
    }

    #[test]
    fn 加分集_空的_编号往后排() {
        let d = tempfile::tempdir().unwrap();
        let p = create_project(d.path(), req("系列")).unwrap();
        let e2 = create_episode(d.path(), &p.id, "", "第二集梗概").unwrap();
        let e3 = create_episode(d.path(), &p.id, "决战", "").unwrap();
        assert_eq!((e2.id.as_str(), e2.title.as_str(), e3.id.as_str(), e3.title.as_str()), ("ep_02", "第 2 集", "ep_03", "决战"));
        assert!(e2.storyboard.is_empty());
        assert_eq!(read_json(&d.path().join("projects").join(&p.id).join("project.json")).unwrap()["episode_count"], 3);
        assert!(create_episode(d.path(), "nope", "", "").is_err());
    }

    #[test]
    fn 删分集_先报内容再删_留一集_不重新编号() {
        let d = tempfile::tempdir().unwrap();
        let p = create_project(d.path(), req("系列")).unwrap();
        let e2 = create_episode(d.path(), &p.id, "E02-《外卖超时》", "").unwrap();
        let e3 = create_episode(d.path(), &p.id, "", "").unwrap();
        import_script(d.path(), &p.id, &e2.id, "【场景：门口】\n阿哲：三十二分钟。", false).unwrap();
        let dir = |id: &str| d.path().join("projects").join(&p.id).join("episodes").join(id);

        let summary = delete_episode(d.path(), &p.id, &e2.id, false).unwrap();
        assert!(summary.contains("E02-《外卖超时》") && summary.contains("0 镜") && summary.contains("剧本"), "{summary}");
        assert!(dir("ep_02").is_dir(), "不带 confirm 不能删");

        delete_episode(d.path(), &p.id, &e2.id, true).unwrap();
        assert!(!dir("ep_02").exists());
        assert!(dir("ep_01").is_dir() && dir(&e3.id).is_dir(), "别的集不动、不改号");
        assert_eq!(read_json(&d.path().join("projects").join(&p.id).join("project.json")).unwrap()["episode_count"], 2);

        delete_episode(d.path(), &p.id, "ep_03", true).unwrap();
        assert!(delete_episode(d.path(), &p.id, "ep_01", true).unwrap_err().to_string().contains("只剩这一集"));
        assert!(dir("ep_01").is_dir());
        assert!(delete_episode(d.path(), &p.id, "../ep_01", false).is_err());
        assert!(delete_episode(d.path(), &p.id, "ep_09", false).is_err());
    }

    /// 界面上的「加集」（`cinema::cinema_create_episode`）曾经塞一集示例剧本和三个示例镜头。
    /// 它现在必须走 `create_episode`，不能再自己拼一集。
    #[test]
    fn 界面加集_走空集_不塞样板() {
        let src = include_str!("cinema.rs");
        let body = src.split("pub async fn cinema_create_episode").nth(1).unwrap().split("\n}\n").next().unwrap();
        assert!(body.contains("studio_ops::create_episode"), "界面加集要复用 studio_ops::create_episode");
        assert!(!body.contains("StoryboardShot"), "加集不能自带示例镜头");
        assert!(!src.contains("锋芒初现") && !src.contains("早已为你备好一切"), "示例剧本不能回来");
    }

    #[test]
    fn 剧本格式检查() {
        let r = check_script("【场景：金銮殿 · 日】\n鼓声震天。\n大臣：大逆不道！\n林婉清（盛唐）：这是……唐朝？\n大臣：斩！\n他看着窗外，心想：这茶要等。");
        assert_eq!(r.scenes, ["场景：金銮殿 · 日"]);
        assert_eq!(r.speakers, ["大臣", "林婉清（盛唐）"]);
        assert!(r.problems.is_empty());
        let bad = check_script("她走进办公室，打开电脑。");
        assert_eq!(bad.problems.len(), 2);
        assert_eq!(check_script("  ").problems, ["剧本是空的"]);
    }

    #[test]
    fn 导入剧本_已有内容不覆盖除非明说() {
        let d = tempfile::tempdir().unwrap();
        let p = create_project(d.path(), req("盛唐")).unwrap();
        let r = import_script(d.path(), &p.id, "ep_01", "【场景：殿】\n皇帝：斩。", false).unwrap();
        assert_eq!(r.speakers, ["皇帝"]);
        assert!(import_script(d.path(), &p.id, "ep_01", "新的", false).is_err(), "已有剧本不覆盖");
        import_script(d.path(), &p.id, "ep_01", "【场景：殿】\n皇帝：赦。", true).unwrap();
        let ep = read_json(&d.path().join("projects").join(&p.id).join("episodes/ep_01/episode.json")).unwrap();
        assert_eq!(ep["script_text"], "【场景：殿】\n皇帝：赦。");
        assert!(ep["script_doc"].is_null());
        assert!(import_script(d.path(), &p.id, "ep_09", "x", true).is_err());
    }

    #[test]
    fn 批量建资产_同名跳过_角色带人物本身() {
        let d = tempfile::tempdir().unwrap();
        let scope = Scope::Episode { project_id: "p1".into(), episode_id: "ep_01".into() };
        let items = vec![
            NewAsset { kind: "character".into(), name: "皇帝".into(), description: "龙袍".into(), identity: "男，40 岁".into() },
            NewAsset { kind: "场景".into(), name: "金銮殿".into(), description: "内景".into(), identity: "忽略".into() },
        ];
        let r = create_assets(d.path(), &scope, &items).unwrap();
        assert_eq!((r.created.len(), r.skipped.len()), (2, 0));
        let again = create_assets(d.path(), &scope, &items[..1]).unwrap();
        assert_eq!((again.created.len(), again.skipped.len(), again.skipped[0].id.as_str()), (0, 1, r.created[0].id.as_str()));
        let store = load_store(d.path(), &scope);
        let emperor = store.assets.iter().find(|a| a.name == "皇帝").unwrap();
        assert_eq!((emperor.identity.as_str(), emperor.description.as_str()), ("男，40 岁", "龙袍"));
        assert!(store.assets.iter().find(|a| a.name == "金銮殿").unwrap().identity.is_empty());
        assert!(create_assets(d.path(), &scope, &[NewAsset { kind: "monster".into(), name: "x".into(), description: String::new(), identity: String::new() }]).is_err());
    }

    #[test]
    fn 设风格_没有的预设报出可选的() {
        let d = tempfile::tempdir().unwrap();
        let first = crate::style::all_presets()[0].id.clone();
        let art = set_style(d.path(), "p1", Some(&first), Some("冷色调")).unwrap();
        assert_eq!((art.preset_id.as_str(), art.positive_prompt.as_str()), (first.as_str(), "冷色调"));
        let err = set_style(d.path(), "p1", Some("nope"), None).unwrap_err().to_string();
        assert!(err.contains("可选"), "{err}");
    }
}
