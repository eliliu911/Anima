//! 把一个角色合并进另一个角色，当它的一个形象（方案第 8 步）。
//!
//! # 为什么要有
//!
//! 形象功能之前，同一个人的不同造型只能建成两个资产：「林婉清（现代）」「林婉清（盛唐）」。
//! 系统不知道是同一个人，脸要写两遍。合并之后是一个角色「林婉清」，主形象「盛唐」+ 形象「现代」。
//!
//! # 会改什么（先预览，确认后才写盘）
//!
//! | 地方 | 怎么改 |
//! |---|---|
//! | 目标角色 | 可以改名；主形象可以起名；加一个形象（源角色的描述、NSFW、候选图；源角色自己的形象也一起带过来） |
//! | 所有分集的镜头 | 出场角色、旧的单角色字段、参考图槽位里的源角色 → 目标角色；这一镜记「用新形象」 |
//! | 台词说话人 | 源角色名、目标角色旧名 → 新名字（提示词按名字认说话人，不改就对不上） |
//! | 项目默认形象表 | 去掉指向源角色的条目 |
//! | 源角色 | 从资产库删掉。**候选图文件不删**：同一作用域时新形象直接引用原文件；不同作用域时复制一份 |
//!
//! 一镜里**同时有源角色和目标角色**（同一个人两个造型同框）没法表示成「一个角色一个形象」，
//! 预览里列成冲突，有冲突不执行。
//!
//! 写盘前把要改的每个文件复制一份到 `backups/merge-<时间>/`。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::assets::{load_store, save_store, Asset, AssetKind, Look, Scope};
use crate::error::{Error, Result};

#[derive(Debug, Clone, Deserialize)]
pub struct MergeReq {
    pub from_scope: Scope,
    pub from_id: String,
    pub to_scope: Scope,
    pub to_id: String,
    /// 合并后角色叫什么。空 = 不改名
    #[serde(default)]
    pub new_name: String,
    /// 目标角色原来那套叫什么（主形象名）。空 = 不改
    #[serde(default)]
    pub main_look_name: String,
    /// 源角色变成的形象叫什么
    pub look_name: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ShotChange {
    pub project_title: String,
    pub episode_title: String,
    pub shot_num: u32,
    pub changes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MergePlan {
    pub from_name: String,
    pub to_name: String,
    pub new_name: String,
    pub main_look_name: String,
    pub look_name: String,
    /// 带过来的候选图张数
    pub images: usize,
    pub shots: Vec<ShotChange>,
    /// 同框冲突。非空时不能执行
    pub conflicts: Vec<String>,
    /// 这些镜头改完之后提示词要重新合成（已经出的片不受影响）
    pub note: String,
}

/// 从两个名字猜合并后的名字：「林婉清（现代）」+「林婉清（盛唐）」→ 林婉清 / 盛唐 / 现代。
/// 猜不出（名字不是「同一个名字 + 括号注」）就不改名，形象名用源角色全名。
pub fn suggest_names(from_name: &str, to_name: &str) -> (String, String, String) {
    fn split(n: &str) -> (String, String) {
        let n = n.trim();
        for (open, close) in [('（', '）'), ('(', ')')] {
            if let Some(i) = n.find(open) {
                if n.ends_with(close) {
                    let base = n[..i].trim().to_string();
                    let note = n[i + open.len_utf8()..n.len() - close.len_utf8()].trim().to_string();
                    if !base.is_empty() && !note.is_empty() {
                        return (base, note);
                    }
                }
            }
        }
        (n.to_string(), String::new())
    }
    let (fb, fnote) = split(from_name);
    let (tb, tnote) = split(to_name);
    if fb == tb && !fnote.is_empty() {
        (tb, tnote, fnote)
    } else {
        (String::new(), String::new(), from_name.trim().to_string())
    }
}

/// 改一镜（JSON 上改，不经过类型 —— 类型没建模的字段原样留着）。回改了什么。
pub fn merge_shot(shot: &mut Value, from_id: &str, to_id: &str, look_id: &str, old_names: &[&str], new_name: &str) -> std::result::Result<Vec<String>, String> {
    let ids_of = |v: &Value| -> Vec<String> { v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default() };
    let mut chars = ids_of(&shot["character_ids"]);
    let legacy = shot["character_id"].as_str().unwrap_or("").to_string();
    let slots_have = |id: &str| shot["ref_slots"].as_array().is_some_and(|a| a.iter().any(|s| s["asset_id"] == id));
    let has_from = chars.iter().any(|c| c == from_id) || legacy == from_id || slots_have(from_id);
    let has_to = chars.iter().any(|c| c == to_id) || legacy == to_id || slots_have(to_id);
    let mut changes = Vec::new();
    if has_from && has_to {
        return Err("同一镜里两个造型都出场".into());
    }
    if has_from {
        for c in chars.iter_mut() {
            if c == from_id {
                *c = to_id.to_string();
            }
        }
        shot["character_ids"] = serde_json::json!(chars);
        if legacy == from_id {
            shot["character_id"] = Value::String(to_id.into());
        }
        if let Some(slots) = shot["ref_slots"].as_array_mut() {
            for s in slots.iter_mut().filter(|s| s["asset_id"] == from_id) {
                s["asset_id"] = Value::String(to_id.into());
            }
        }
        if !shot["look_overrides"].is_object() {
            shot["look_overrides"] = serde_json::json!({});
        }
        shot["look_overrides"][to_id] = Value::String(look_id.into());
        changes.push("出场角色、参考图换成合并后的角色，这一镜用新形象".to_string());
    }
    if !new_name.is_empty() {
        let mut renamed = 0;
        if let Some(lines) = shot["dialogue_lines"].as_array_mut() {
            for l in lines.iter_mut() {
                if l["speaker"].as_str().is_some_and(|s| s.trim() != new_name && old_names.contains(&s.trim())) {
                    l["speaker"] = Value::String(new_name.into());
                    renamed += 1;
                }
            }
        }
        if renamed > 0 {
            changes.push(format!("{renamed} 句台词的说话人改成「{new_name}」"));
        }
    }
    Ok(changes)
}

struct Episode {
    file: PathBuf,
    project_title: String,
    value: Value,
}

fn episodes(config_dir: &Path) -> Vec<Episode> {
    let mut out = Vec::new();
    let Ok(projects) = std::fs::read_dir(config_dir.join("projects")) else { return out };
    for p in projects.flatten().filter(|p| p.path().is_dir()) {
        let title = std::fs::read_to_string(p.path().join("project.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v["title"].as_str().map(String::from))
            .unwrap_or_else(|| p.file_name().to_string_lossy().to_string());
        let Ok(eps) = std::fs::read_dir(p.path().join("episodes")) else { continue };
        for e in eps.flatten() {
            let file = e.path().join("episode.json");
            if let Some(value) = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()) {
                out.push(Episode { file, project_title: title.clone(), value });
            }
        }
    }
    out
}

fn find(config_dir: &Path, scope: &Scope, id: &str) -> Result<Asset> {
    load_store(config_dir, scope).assets.into_iter().find(|a| a.id == id).ok_or_else(|| Error::runtime("找不到这个角色"))
}

/// 算一遍会改什么，不写盘。`look_id` 给执行时用；预览时是占位
fn compute(config_dir: &Path, req: &MergeReq, look_id: &str) -> Result<(MergePlan, Vec<(PathBuf, Value)>)> {
    if req.from_id == req.to_id && req.from_scope == req.to_scope {
        return Err(Error::runtime("不能合并进自己"));
    }
    let from = find(config_dir, &req.from_scope, &req.from_id)?;
    let to = find(config_dir, &req.to_scope, &req.to_id)?;
    if from.kind != AssetKind::Character || to.kind != AssetKind::Character {
        return Err(Error::runtime("只有角色能合并成形象"));
    }
    if req.look_name.trim().is_empty() {
        return Err(Error::runtime("给新形象起个名字"));
    }
    let new_name = Some(req.new_name.trim()).filter(|n| !n.is_empty()).unwrap_or(to.name.as_str()).to_string();
    let old_names = [from.name.as_str(), to.name.as_str()];
    let mut shots = Vec::new();
    let mut conflicts = Vec::new();
    let mut writes = Vec::new();
    for mut ep in episodes(config_dir) {
        let ep_title = ep.value["title"].as_str().unwrap_or("").to_string();
        let mut touched = false;
        if let Some(board) = ep.value["storyboard"].as_array_mut() {
            for shot in board.iter_mut() {
                let num = shot["shot_num"].as_u64().unwrap_or(0) as u32;
                match merge_shot(shot, &req.from_id, &req.to_id, look_id, &old_names, &new_name) {
                    Ok(changes) if !changes.is_empty() => {
                        touched = true;
                        shots.push(ShotChange { project_title: ep.project_title.clone(), episode_title: ep_title.clone(), shot_num: num, changes });
                    }
                    Ok(_) => {}
                    Err(why) => conflicts.push(format!("{} · {} · 第 {num} 镜：{why}", ep.project_title, ep_title)),
                }
            }
        }
        if touched {
            writes.push((ep.file, ep.value));
        }
    }
    let images = from.all_units().map(|u| u.image_variants.len()).sum();
    let plan = MergePlan {
        from_name: from.name.clone(),
        to_name: to.name.clone(),
        new_name,
        main_look_name: req.main_look_name.trim().to_string(),
        look_name: req.look_name.trim().to_string(),
        images,
        shots,
        conflicts,
        note: "改到的镜头提示词会标「要重新合成」；已经出好的片不受影响。".into(),
    };
    Ok((plan, writes))
}

pub fn plan_merge(config_dir: &Path, req: &MergeReq) -> Result<MergePlan> {
    compute(config_dir, req, "look_preview").map(|(p, _)| p)
}

/// 执行合并。回合并后的目标角色
pub fn apply_merge(config_dir: &Path, req: &MergeReq) -> Result<Asset> {
    let look_id = format!("look_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    let (plan, episode_writes) = compute(config_dir, req, &look_id)?;
    if !plan.conflicts.is_empty() {
        return Err(Error::runtime(format!("有同框冲突，没有合并：{}", plan.conflicts.join("；"))));
    }
    let from = find(config_dir, &req.from_scope, &req.from_id)?;

    // 备份：要写的分集、两个作用域的资产索引、项目文件
    // 备份不放 projects/ 底下 —— 那里的每个子目录都会被当成项目扫
    let backup = config_dir.join("backups").join(format!("merge-{}", crate::jobs::now()));
    std::fs::create_dir_all(&backup).map_err(|e| Error::runtime(format!("建备份目录失败：{e}")))?;
    let mut to_backup: Vec<PathBuf> = episode_writes.iter().map(|(f, _)| f.clone()).collect();
    to_backup.push(req.from_scope.store_file(config_dir));
    to_backup.push(req.to_scope.store_file(config_dir));
    if let Ok(ps) = std::fs::read_dir(config_dir.join("projects")) {
        to_backup.extend(ps.flatten().map(|p| p.path().join("project.json")).filter(|p| p.is_file()));
    }
    for (i, f) in to_backup.iter().enumerate().filter(|(_, f)| f.is_file()) {
        let name = format!("{i:02}_{}", f.strip_prefix(config_dir).unwrap_or(f).to_string_lossy().replace('/', "__"));
        std::fs::copy(f, backup.join(name)).map_err(|e| Error::runtime(format!("备份失败：{e}")))?;
    }

    // 新形象：源角色的主形象 + 源角色自己的形象
    let from_media = req.from_scope.media_dir(config_dir);
    let to_media = req.to_scope.media_dir(config_dir);
    let same_scope = req.from_scope == req.to_scope;
    let relocate = |unit: &mut crate::assets::AssetUnit| -> Result<()> {
        if same_scope {
            return Ok(()); // 同一作用域：相对路径照样指得到原文件
        }
        for v in unit.image_variants.iter_mut().chain(unit.video_variants.iter_mut()) {
            let file = Path::new(&v.rel_path).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| format!("{}.png", v.id));
            let rel = format!("{}/{}", req.to_id, file);
            let dst = to_media.join(&rel);
            if let Some(parent) = dst.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::copy(from_media.join(&v.rel_path), &dst).map_err(|e| Error::runtime(format!("复制候选图失败：{e}")))?;
            v.rel_path = rel;
        }
        Ok(())
    };
    let now = crate::assets::now_ms();
    let mut new_looks = Vec::new();
    let mut main_unit = from.unit.clone();
    relocate(&mut main_unit)?;
    new_looks.push(Look { id: look_id.clone(), name: plan.look_name.clone(), description: from.description.clone(), nsfw: from.nsfw, unit: main_unit, created_at: now, updated_at: now });
    for l in &from.looks {
        let mut unit = l.unit.clone();
        relocate(&mut unit)?;
        new_looks.push(Look { id: l.id.clone(), name: format!("{} · {}", plan.look_name, l.name), unit, ..l.clone() });
    }

    // 1. 目标角色
    let mut to_store = load_store(config_dir, &req.to_scope);
    let to = to_store.assets.iter_mut().find(|a| a.id == req.to_id).ok_or_else(|| Error::runtime("找不到目标角色"))?;
    if plan.new_name != to.name {
        to.name = plan.new_name.clone();
    }
    if !plan.main_look_name.is_empty() {
        to.main_look_name = plan.main_look_name.clone();
    }
    if to.identity.trim().is_empty() && !from.identity.trim().is_empty() {
        to.identity = from.identity.clone();
    }
    to.looks.extend(new_looks);
    to.updated_at = now;
    let merged = to.clone();
    // 2. 源角色从资产库去掉（同一作用域时在同一份索引里）
    if same_scope {
        to_store.assets.retain(|a| a.id != req.from_id);
        save_store(config_dir, &req.to_scope, &to_store)?;
    } else {
        save_store(config_dir, &req.to_scope, &to_store)?;
        let mut from_store = load_store(config_dir, &req.from_scope);
        from_store.assets.retain(|a| a.id != req.from_id);
        save_store(config_dir, &req.from_scope, &from_store)?;
    }
    // 3. 分集。**不写 `compute` 算好的那份** —— 那是开头读的，复制媒体、改资产库这段时间里
    // 出片回写、导演世界都可能写过；在分集锁里按最新的那份重新合并一遍。
    // 冲突开头已经查过；这期间新冒出来的冲突那一镜保持原样（不硬改成同框两个同一人）
    let old_names = [plan.from_name.as_str(), plan.to_name.as_str()];
    for (file, _) in &episode_writes {
        crate::episode_lock::update_json(file, |v| {
            for shot in v["storyboard"].as_array_mut().into_iter().flatten() {
                let _ = merge_shot(shot, &req.from_id, &req.to_id, &look_id, &old_names, &plan.new_name);
            }
            Ok(())
        })
        .map_err(|e| Error::runtime(format!("写分集失败：{e}")))?;
    }
    // 4. 项目默认形象表里去掉源角色
    if let Ok(ps) = std::fs::read_dir(config_dir.join("projects")) {
        for p in ps.flatten() {
            let f = p.path().join("project.json");
            let Some(mut v) = std::fs::read_to_string(&f).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else { continue };
            if v["default_looks"].get(&req.from_id).is_some() {
                v["default_looks"].as_object_mut().map(|m| m.remove(&req.from_id));
                let _ = std::fs::write(&f, serde_json::to_string_pretty(&v).unwrap_or_default());
            }
        }
    }
    Ok(merged)
}

#[tauri::command]
pub fn assets_merge_plan(state: tauri::State<'_, crate::AppState>, req: MergeReq) -> Result<MergePlan> {
    plan_merge(&state.config_dir, &req)
}

#[tauri::command]
pub fn assets_merge_apply(state: tauri::State<'_, crate::AppState>, req: MergeReq) -> Result<()> {
    apply_merge(&state.config_dir, &req).map(|_| ())
}

/// 界面上预填的名字
#[tauri::command]
pub fn assets_merge_suggest(from_name: String, to_name: String) -> (String, String, String) {
    suggest_names(&from_name, &to_name)
}

#[cfg(test)]
mod live {
    use super::*;

    /// 真实项目上合并。`ANIMA_MERGE_APPLY=1` 才写盘，否则只打印预览。
    /// `ANIMA_MERGE_PROJECT` `ANIMA_MERGE_EPISODE` `ANIMA_MERGE_FROM` `ANIMA_MERGE_TO`
    #[test]
    #[ignore = "改真实项目"]
    fn 真实项目合并() {
        let cfg = dirs::config_dir().unwrap().join("com.anima.desktop");
        let env = |k: &str| std::env::var(k).unwrap();
        let scope = Scope::Episode { project_id: env("ANIMA_MERGE_PROJECT"), episode_id: env("ANIMA_MERGE_EPISODE") };
        let store = load_store(&cfg, &scope);
        let name_of = |id: &str| store.assets.iter().find(|a| a.id == id).map(|a| a.name.clone()).unwrap();
        let (from, to) = (env("ANIMA_MERGE_FROM"), env("ANIMA_MERGE_TO"));
        let (new_name, main_look_name, look_name) = suggest_names(&name_of(&from), &name_of(&to));
        let req = MergeReq { from_scope: scope.clone(), from_id: from, to_scope: scope, to_id: to, new_name, main_look_name, look_name };
        let plan = plan_merge(&cfg, &req).unwrap();
        println!("{}", serde_json::to_string_pretty(&plan).unwrap());
        if std::env::var("ANIMA_MERGE_APPLY").is_ok() {
            let merged = apply_merge(&cfg, &req).unwrap();
            println!("已合并：{} 主形象「{}」+ {:?}", merged.name, merged.main_look_name, merged.looks.iter().map(|l| (&l.name, l.unit.image_variants.len())).collect::<Vec<_>>());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::{add_variant, AssetStore};

    #[test]
    fn 猜名字() {
        assert_eq!(suggest_names("林婉清（现代）", "林婉清（盛唐）"), ("林婉清".into(), "盛唐".into(), "现代".into()));
        assert_eq!(suggest_names("Emma (young)", "Emma (old)"), ("Emma".into(), "old".into(), "young".into()));
        assert_eq!(suggest_names("小花", "大橘"), (String::new(), String::new(), "小花".into()));
    }

    #[test]
    fn 改一镜_同框冲突拦下() {
        let mut shot = serde_json::json!({
            "shot_num": 4, "character_id": "from", "character_ids": ["from", "minister"],
            "ref_slots": [{"order": 1, "asset_id": "from"}, {"order": 2, "asset_id": "minister"}],
            "dialogue_lines": [{"speaker": "大臣"}, {"speaker": "林婉清（盛唐）"}, {"speaker": "林婉清（现代）"}],
            "unknown_field": 1
        });
        let changes = merge_shot(&mut shot, "from", "to", "look_x", &["林婉清（现代）", "林婉清（盛唐）"], "林婉清").unwrap();
        assert_eq!(changes.len(), 2);
        assert_eq!(shot["character_ids"], serde_json::json!(["to", "minister"]));
        assert_eq!(shot["character_id"], "to");
        assert_eq!(shot["ref_slots"][0]["asset_id"], "to");
        assert_eq!(shot["ref_slots"][1]["asset_id"], "minister");
        assert_eq!(shot["look_overrides"]["to"], "look_x");
        assert_eq!(shot["dialogue_lines"][0]["speaker"], "大臣");
        assert_eq!(shot["dialogue_lines"][1]["speaker"], "林婉清");
        assert_eq!(shot["dialogue_lines"][2]["speaker"], "林婉清");
        assert_eq!(shot["unknown_field"], 1, "没建模的字段原样留着");
        // 只有目标角色的镜头：不加覆盖，只改说话人
        let mut t = serde_json::json!({"character_ids": ["to"], "dialogue_lines": [{"speaker": "林婉清（盛唐）"}]});
        let c = merge_shot(&mut t, "from", "to", "look_x", &["林婉清（现代）", "林婉清（盛唐）"], "林婉清").unwrap();
        assert_eq!(c.len(), 1);
        assert!(t.get("look_overrides").is_none());
        // 不改名：源角色名的台词改成目标角色名，目标角色名的不算改动
        let mut k = serde_json::json!({"character_ids": ["to"], "dialogue_lines": [{"speaker": "大橘"}, {"speaker": "小花"}]});
        let c = merge_shot(&mut k, "from", "to", "l", &["小花", "大橘"], "大橘").unwrap();
        assert_eq!((c.len(), k["dialogue_lines"][1]["speaker"].as_str()), (1, Some("大橘")));
        assert_eq!(c[0], "1 句台词的说话人改成「大橘」");
        // 同框
        let mut both = serde_json::json!({"character_ids": ["from", "to"]});
        assert!(merge_shot(&mut both, "from", "to", "l", &[], "").is_err());
    }

    #[test]
    fn 合并_目标加形象_镜头改过来_源角色删掉_有备份() {
        let d = tempfile::tempdir().unwrap();
        let cfg = d.path();
        let scope = Scope::Episode { project_id: "p1".into(), episode_id: "ep_01".into() };
        let mut modern = Asset::new(AssetKind::Character, "林婉清（现代）");
        modern.description = "白衬衫".into();
        add_variant(cfg, &scope, &mut modern, b"modern", "png", false, None, false, None).unwrap();
        let mut tang = Asset::new(AssetKind::Character, "林婉清（盛唐）");
        tang.description = "官服".into();
        let (mid, tid) = (modern.id.clone(), tang.id.clone());
        save_store(cfg, &scope, &AssetStore { assets: vec![modern, tang] }).unwrap();
        let proj = cfg.join("projects").join("p1");
        std::fs::create_dir_all(proj.join("episodes/ep_01")).unwrap();
        std::fs::write(proj.join("project.json"), format!(r#"{{"title":"盛唐","default_looks":{{"{mid}":"x"}}}}"#)).unwrap();
        std::fs::write(proj.join("episodes/ep_01/episode.json"), serde_json::json!({
            "id": "ep_01", "title": "第一集",
            "storyboard": [
                {"shot_num": 1, "character_ids": [mid], "ref_slots": [{"asset_id": mid}]},
                {"shot_num": 2, "character_ids": [tid], "dialogue_lines": [{"speaker": "林婉清（盛唐）"}]},
                {"shot_num": 3, "character_ids": []}
            ]
        }).to_string()).unwrap();
        let (new_name, main_name, look_name) = suggest_names("林婉清（现代）", "林婉清（盛唐）");
        let req = MergeReq { from_scope: scope.clone(), from_id: mid.clone(), to_scope: scope.clone(), to_id: tid.clone(), new_name, main_look_name: main_name, look_name };

        let plan = plan_merge(cfg, &req).unwrap();
        assert_eq!((plan.new_name.as_str(), plan.images, plan.shots.len(), plan.conflicts.len()), ("林婉清", 1, 2, 0));
        assert!(load_store(cfg, &scope).assets.iter().any(|a| a.id == mid), "预览不写盘");

        let merged = apply_merge(cfg, &req).unwrap();
        assert_eq!((merged.name.as_str(), merged.main_look_name.as_str(), merged.looks.len()), ("林婉清", "盛唐", 1));
        let look = &merged.looks[0];
        assert_eq!((look.name.as_str(), look.description.as_str()), ("现代", "白衬衫"));
        let img = crate::assets::variant_abs_path(cfg, &scope, look.unit.selected_image().unwrap());
        assert_eq!(std::fs::read(img).unwrap(), b"modern", "同一作用域：新形象引用原文件");
        let store = load_store(cfg, &scope);
        assert_eq!(store.assets.len(), 1, "源角色删掉");
        let ep: Value = serde_json::from_str(&std::fs::read_to_string(proj.join("episodes/ep_01/episode.json")).unwrap()).unwrap();
        assert_eq!(ep["storyboard"][0]["character_ids"], serde_json::json!([tid]));
        assert_eq!(ep["storyboard"][0]["look_overrides"][&tid], look.id.as_str());
        assert_eq!(ep["storyboard"][1]["dialogue_lines"][0]["speaker"], "林婉清");
        let pj: Value = serde_json::from_str(&std::fs::read_to_string(proj.join("project.json")).unwrap()).unwrap();
        assert!(pj["default_looks"].get(&mid).is_none());
        let backups: Vec<_> = std::fs::read_dir(cfg.join("backups")).unwrap().flatten().collect();
        assert_eq!(backups.len(), 1);
        assert!(std::fs::read_dir(backups[0].path()).unwrap().count() >= 3, "分集、资产索引、项目文件都要备份");
        // 合并进自己、同框都拒绝
        assert!(plan_merge(cfg, &MergeReq { from_id: tid.clone(), ..req.clone() }).is_err());
    }
}
