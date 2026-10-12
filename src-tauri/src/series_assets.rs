//! 系列通用资产：把分集里的资产收进系列池。
//!
//! # 为什么要有
//!
//! 提取实体以前只写本集（见 `cinema::import_entities` 的旧注释）：第一集提取出来的
//! 陆川，第二集用不到，第二集再提取一次又长出一个陆川。设定卡出在哪一个上都不对。
//!
//! 现在提取默认直接进系列池、同名并入已有的；这个模块处理**已经落在分集里的**那些
//! （老数据、以及以后用户手动建的本集资产）：
//!
//! | 情况 | 做什么 |
//! |---|---|
//! | 系列里没有同类同名的 | **原样搬过去，id 不变**。镜头按 id 引用，所以一镜都不用改 |
//! | 系列里已经有同类同名的 | 并进那一个：镜头引用改指过去，候选图和形象带过去，空着的描述补上，本集这份删掉 |
//! | 用户说「只留本集」 | 打个标记（[`crate::assets::Asset::episode_only`]），不再提醒 |
//!
//! # 为什么搬过去不能用 `copy_asset_between`
//!
//! 那个函数给副本发一个**新 id**（它是给「提升到全局库」用的 —— 两份并存）。
//! 这里是搬家不是复印：换了 id，镜头里的 `character_ids` 就全指空了，
//! 表现是「这一镜的人突然不像了」，而且不报错。
//!
//! # 写盘之前先备份
//!
//! 每次调用先把这个项目的资产索引和所有 `episode.json` 复制到 `backups/series-<时间>/`。
//! 合并会改镜头，改错了要能退回去。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::assets::{load_store, save_store, Asset, AssetUnit, AssetView, Scope};
use crate::error::{Error, Result};

fn project_scope(project_id: &str) -> Scope {
    Scope::Project { project_id: project_id.to_string() }
}

fn episode_scope(project_id: &str, episode_id: &str) -> Scope {
    Scope::Episode { project_id: project_id.to_string(), episode_id: episode_id.to_string() }
}

/// 这个项目的分集 id，按目录名排序（`ep_01`、`ep_02`…）。
fn episode_ids(config_dir: &Path, project_id: &str) -> Vec<String> {
    let dir = crate::cinema::cinema_projects_dir(config_dir).join(project_id).join("episodes");
    let mut ids: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().join("episode.json").is_file())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    ids
}

fn episode_file(config_dir: &Path, project_id: &str, episode_id: &str) -> PathBuf {
    crate::cinema::cinema_projects_dir(config_dir)
        .join(project_id)
        .join("episodes")
        .join(episode_id)
        .join("episode.json")
}

/// 同类、名字去掉首尾空白后相等 —— 和提取那一步判「已有」的规则一样。
/// **不做模糊匹配**：「老茶馆（清晨）」和「老茶馆」自动合并的话，错了用户看不出来。
pub fn same_entity(a: &Asset, b: &Asset) -> bool {
    a.kind == b.kind && a.name.trim() == b.name.trim()
}

/// 把这个项目的资产索引和全部分集文件复制一份。返回备份目录。
fn backup(config_dir: &Path, project_id: &str) -> Result<PathBuf> {
    let dir = config_dir.join("backups").join(format!("series-{}", crate::jobs::now()));
    std::fs::create_dir_all(&dir).map_err(|e| Error::runtime(format!("建备份目录失败：{e}")))?;
    let mut files = vec![project_scope(project_id).store_file(config_dir)];
    for ep in episode_ids(config_dir, project_id) {
        files.push(episode_scope(project_id, &ep).store_file(config_dir));
        files.push(episode_file(config_dir, project_id, &ep));
    }
    for (i, f) in files.iter().enumerate().filter(|(_, f)| f.is_file()) {
        let name = format!(
            "{i:03}_{}",
            f.strip_prefix(config_dir).unwrap_or(f).to_string_lossy().replace('/', "__")
        );
        std::fs::copy(f, dir.join(name)).map_err(|e| Error::runtime(format!("备份失败：{e}")))?;
    }
    Ok(dir)
}

/// 挪一个媒体文件。同一块盘上 `rename` 就够；跨盘（不太可能）退回复制再删。
fn move_file(src: &Path, dst: &Path) -> Result<()> {
    if let Some(parent) = dst.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    std::fs::copy(src, dst).map_err(|e| Error::runtime(format!("挪媒体文件失败（{}）：{e}", src.display())))?;
    let _ = std::fs::remove_file(src);
    Ok(())
}

/// 把一个 unit 里的媒体从 `from_media` 挪到 `to_media`，路径改成 `<owner_id>/<文件名>`。
///
/// 文件不在了（被手动删过）就跳过那一张 —— 为了一张丢了的图整个搬家失败，比少一张图更糟。
fn relocate_unit(unit: &mut AssetUnit, from_media: &Path, to_media: &Path, owner_id: &str) -> Result<()> {
    let mut missing = Vec::new();
    for v in unit.image_variants.iter_mut().chain(unit.video_variants.iter_mut()) {
        let src = from_media.join(&v.rel_path);
        let file = Path::new(&v.rel_path)
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| format!("{}.png", v.id));
        let rel = format!("{owner_id}/{file}");
        if !src.is_file() {
            missing.push(v.id.clone());
            continue;
        }
        move_file(&src, &to_media.join(&rel))?;
        v.rel_path = rel;
    }
    unit.image_variants.retain(|v| !missing.contains(&v.id));
    unit.video_variants.retain(|v| !missing.contains(&v.id));
    Ok(())
}

/// 空着的字段用 `src` 补上。**只补空的，不覆盖** —— 系列里那份可能是用户改过的。
pub fn fill_missing(target: &mut Asset, src: &Asset) {
    if target.description.trim().is_empty() && !src.description.trim().is_empty() {
        target.description = src.description.clone();
    }
    if target.identity.trim().is_empty() && !src.identity.trim().is_empty() {
        target.identity = src.identity.clone();
    }
    if let (Some(t), Some(s)) = (target.character.as_mut(), src.character.as_ref()) {
        if t.persona.trim().is_empty() {
            t.persona = s.persona.clone();
        }
        if t.visual_dna.is_none() {
            t.visual_dna = s.visual_dna.clone();
        }
        if t.spec.is_none() {
            t.spec = s.spec.clone();
        }
        if t.voice_id.is_none() {
            t.voice_id = s.voice_id.clone();
        }
    } else if target.character.is_none() && src.character.is_some() {
        target.character = src.character.clone();
    }
    if target.scene.is_none() && src.scene.is_some() {
        target.scene = src.scene.clone();
    }
}

/// 一镜里把 `from` 的引用全换成 `to`。返回改了没有。
///
/// 覆盖所有按 id 引用资产的字段：出场角色、旧的单角色字段、场景、道具、参考图槽位；
/// `look_overrides` 里源资产那一条去掉（形象 id 属于源资产，并过去之后对不上了）。
pub fn rewrite_shot(shot: &mut Value, from: &str, to: &str) -> bool {
    let mut changed = false;
    for key in ["character_ids", "prop_ids"] {
        if let Some(arr) = shot[key].as_array_mut() {
            if !arr.iter().any(|v| v == from) {
                continue;
            }
            // 同一镜里两个都在：去掉源的那个就行，不然会出现两次同一个人
            if arr.iter().any(|v| v == to) {
                arr.retain(|v| v != from);
            } else {
                for v in arr.iter_mut().filter(|v| *v == from) {
                    *v = Value::String(to.into());
                }
            }
            changed = true;
        }
    }
    for key in ["character_id", "scene_id"] {
        if shot[key] == from {
            shot[key] = Value::String(to.into());
            changed = true;
        }
    }
    if let Some(slots) = shot["ref_slots"].as_array_mut() {
        for s in slots.iter_mut().filter(|s| s["asset_id"] == from) {
            s["asset_id"] = Value::String(to.into());
            changed = true;
        }
    }
    if let Some(m) = shot["look_overrides"].as_object_mut() {
        changed |= m.remove(from).is_some();
    }
    changed
}

/// 这个项目所有分集里，把 `from` 的引用改成 `to`。返回改了几镜。
fn rewrite_project_shots(config_dir: &Path, project_id: &str, from: &str, to: &str) -> Result<usize> {
    let mut n = 0;
    for ep in episode_ids(config_dir, project_id) {
        let f = episode_file(config_dir, project_id, &ep);
        // 读不出来的分集跳过（和以前一样）；读得出来的在分集锁里改，没改到就不写
        if std::fs::read_to_string(&f).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).is_none() {
            continue;
        }
        n += crate::episode_lock::update_json(&f, |v| {
            let mut touched = 0;
            if let Some(board) = v["storyboard"].as_array_mut() {
                for shot in board.iter_mut() {
                    if rewrite_shot(shot, from, to) {
                        touched += 1;
                    }
                }
            }
            Ok(touched)
        })
        .map_err(|e| Error::runtime(format!("写分集失败：{e}")))?;
    }
    // 项目默认形象表里源资产那一条也去掉（形象 id 是源资产的）
    let pf = crate::cinema::cinema_projects_dir(config_dir).join(project_id).join("project.json");
    if let Some(mut v) = std::fs::read_to_string(&pf).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
        if v["default_looks"].get(from).is_some() {
            v["default_looks"].as_object_mut().map(|m| m.remove(from));
            let _ = std::fs::write(&pf, serde_json::to_string_pretty(&v).unwrap_or_default());
        }
    }
    Ok(n)
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AdoptOutcome {
    /// 原样搬进系列，id 不变
    Moved { id: String, name: String },
    /// 系列里已有同名的，并了进去
    Absorbed { from_id: String, into_id: String, name: String, shots: usize },
}

/// 一个分集资产收进系列池（不备份 —— 调用方负责）。
fn adopt_one(config_dir: &Path, project_id: &str, episode_id: &str, id: &str) -> Result<AdoptOutcome> {
    let ep_sc = episode_scope(project_id, episode_id);
    let pr_sc = project_scope(project_id);
    let mut ep_store = load_store(config_dir, &ep_sc);
    let idx = ep_store
        .assets
        .iter()
        .position(|a| a.id == id)
        .ok_or_else(|| Error::runtime("这一集里找不到这个资产"))?;
    let mut src = ep_store.assets[idx].clone();
    let ep_media = ep_sc.media_dir(config_dir);
    let pr_media = pr_sc.media_dir(config_dir);
    let mut pr_store = load_store(config_dir, &pr_sc);

    let outcome = if let Some(target) = pr_store.assets.iter_mut().find(|t| same_entity(t, &src)) {
        // —— 并进已有的那一个 ——
        let into_id = target.id.clone();
        relocate_unit(&mut src.unit, &ep_media, &pr_media, &into_id)?;
        for l in src.looks.iter_mut() {
            relocate_unit(&mut l.unit, &ep_media, &pr_media, &into_id)?;
        }
        fill_missing(target, &src);
        if target.unit.selected_image_id.is_none() {
            target.unit.selected_image_id = src.unit.selected_image_id.clone();
        }
        target.unit.image_variants.extend(src.unit.image_variants.clone());
        target.unit.video_variants.extend(src.unit.video_variants.clone());
        // 源资产的形象带过去；同名形象不重复加
        for l in src.looks.clone() {
            if !target.looks.iter().any(|t| t.name == l.name) {
                target.looks.push(l);
            }
        }
        target.updated_at = crate::assets::now_ms();
        let name = target.name.clone();
        save_store(config_dir, &pr_sc, &pr_store)?;
        ep_store.assets.remove(idx);
        save_store(config_dir, &ep_sc, &ep_store)?;
        let shots = rewrite_project_shots(config_dir, project_id, &src.id, &into_id)?;
        AdoptOutcome::Absorbed { from_id: src.id.clone(), into_id, name, shots }
    } else {
        // —— 原样搬家，id 不变 ——
        let owner = src.id.clone();
        for unit in src.all_units_mut() {
            relocate_unit(unit, &ep_media, &pr_media, &owner)?;
        }
        src.episode_only = false;
        src.updated_at = crate::assets::now_ms();
        let name = src.name.clone();
        pr_store.assets.push(src);
        save_store(config_dir, &pr_sc, &pr_store)?;
        ep_store.assets.remove(idx);
        save_store(config_dir, &ep_sc, &ep_store)?;
        AdoptOutcome::Moved { id: owner, name }
    };
    // 本集媒体目录里这个资产的子目录空了就删掉（不空说明还有别的东西，别动）
    let _ = std::fs::remove_dir(ep_media.join(id));
    Ok(outcome)
}

/// 一个分集资产收进系列池。先备份。
pub fn adopt(config_dir: &Path, project_id: &str, episode_id: &str, id: &str) -> Result<AdoptOutcome> {
    backup(config_dir, project_id)?;
    adopt_one(config_dir, project_id, episode_id, id)
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct AdoptSummary {
    pub moved: usize,
    pub absorbed: usize,
    /// 标了「只留本集」、没动的
    pub kept: usize,
}

/// 所有分集里还待归属的，全部收进系列（同名自动合并）。先备份一次。
///
/// **按集号顺序处理。** 第一集的陆川先搬进去，第二集的陆川就并进它 ——
/// 反过来也行，但按顺序的话「系列里那一份」是最早出现的那一份，好解释。
pub fn adopt_all(config_dir: &Path, project_id: &str) -> Result<AdoptSummary> {
    backup(config_dir, project_id)?;
    let mut sum = AdoptSummary::default();
    for ep in episode_ids(config_dir, project_id) {
        let store = load_store(config_dir, &episode_scope(project_id, &ep));
        for a in store.assets {
            if a.episode_only {
                sum.kept += 1;
                continue;
            }
            match adopt_one(config_dir, project_id, &ep, &a.id)? {
                AdoptOutcome::Moved { .. } => sum.moved += 1,
                AdoptOutcome::Absorbed { .. } => sum.absorbed += 1,
            }
        }
    }
    Ok(sum)
}

/// 标记 / 取消「只留本集」。
pub fn set_episode_only(config_dir: &Path, project_id: &str, episode_id: &str, id: &str, on: bool) -> Result<()> {
    let sc = episode_scope(project_id, episode_id);
    let mut store = load_store(config_dir, &sc);
    let a = store
        .assets
        .iter_mut()
        .find(|a| a.id == id)
        .ok_or_else(|| Error::runtime("这一集里找不到这个资产"))?;
    a.episode_only = on;
    a.updated_at = crate::assets::now_ms();
    save_store(config_dir, &sc, &store)
}

#[derive(Debug, Clone, Serialize)]
pub struct InboxItem {
    pub episode_id: String,
    pub episode_num: u32,
    pub episode_title: String,
    pub asset: AssetView,
    /// 系列里同类同名的那一个（收进去时会并进它）
    pub match_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct SeriesInbox {
    /// 待归属：在分集里、没标「只留本集」
    pub pending: Vec<InboxItem>,
    /// 标了「只留本集」的
    pub kept: Vec<InboxItem>,
}

/// 系列页顶上那块：各集里还没进系列池的资产。
pub fn inbox(config_dir: &Path, project_id: &str) -> SeriesInbox {
    let series = load_store(config_dir, &project_scope(project_id)).assets;
    // 引用数一次算完：逐个资产调 `scan_references` 会把全部项目的分集读 N 遍
    let mut refs: HashMap<String, usize> = HashMap::new();
    let mut eps = Vec::new();
    for ep in episode_ids(config_dir, project_id) {
        let text = std::fs::read_to_string(episode_file(config_dir, project_id, &ep)).unwrap_or_default();
        let data = serde_json::from_str::<crate::cinema::EpisodeData>(&text).ok();
        // 集号和标题从裸 JSON 读 —— 分集结构对不上类型（老数据缺字段）时也要显示得出是哪一集
        let raw = serde_json::from_str::<Value>(&text).unwrap_or_default();
        let num = raw["episode_num"].as_u64().unwrap_or(0) as u32;
        let title = raw["title"].as_str().map(String::from).unwrap_or_else(|| ep.clone());
        eps.push((ep, num, title, data));
    }
    let mut out = SeriesInbox::default();
    for (ep, num, title, _) in &eps {
        let sc = episode_scope(project_id, ep);
        for a in load_store(config_dir, &sc).assets {
            let n = *refs.entry(a.id.clone()).or_insert_with(|| {
                eps.iter()
                    .filter_map(|(_, _, _, d)| d.as_ref())
                    .flat_map(|d| d.storyboard.iter())
                    .filter(|s| s.references_asset(&a.id))
                    .count()
            });
            let match_id = series.iter().find(|s| same_entity(s, &a)).map(|s| s.id.clone());
            let kept = a.episode_only;
            let item = InboxItem {
                episode_id: ep.clone(),
                episode_num: *num,
                episode_title: title.clone(),
                asset: crate::assets::to_view(config_dir, &sc, a, n),
                match_id,
            };
            if kept {
                out.kept.push(item);
            } else {
                out.pending.push(item);
            }
        }
    }
    out
}

/// 从设定文档里认出角色 / 场景 / 道具（**不落盘**）。系列页「从设定文档建资产」用。
///
/// # 为什么一份文档调一次模型
///
/// 场景设计一份就七千多字、几十个条目。几份拼成一段喂进去，模型要么截断
/// （后台档有输出上限），要么后半段的条目描述越写越短。一份一份来，
/// 每份的条目数都在它能从容写完的范围里。
///
/// 结果按「类型/名字」去重，同名的保留描述更长的那一份。系列里已有同类同名的标 `exists`。
///
/// # 每份文档只收它自己那一类
///
/// 2026-09-25 拿《回收师》三份设定文档实跑（qwen3.8-27b，25 分钟）：164 个，一半是噪声 ——
/// 场景设计里顺带写到的孙富豪、指挥官被抽成了角色，「可复用道具」那一行的鸡腿、水塔、
/// 天线被抽成了道具，人物小传里提到的地方又被抽成了一套名字不一样的场景
/// （「下城铁皮屋（陆川住所）」和「陆川的铁皮屋」各一个）。
/// 设定文档本来就是按类分的，标题说了是什么就只收什么（见 [`kinds_for_title`]）。
pub async fn extract_from_texts(
    config_dir: &Path,
    gateway: &crate::Gateway,
    project_id: &str,
    docs: Vec<DesignDoc>,
) -> Result<Vec<crate::prompts::ExtractedEntity>> {
    let cfg = crate::prompts::load(config_dir);
    let mut out: Vec<crate::prompts::ExtractedEntity> = Vec::new();
    let mut failures = Vec::new();
    for doc in docs.iter().filter(|d| !d.text.trim().is_empty()) {
        let raw = crate::prompts::run(gateway, &cfg, crate::prompts::PromptSlot::EntityExtraction, &[("TEXT", doc.text.trim())]).await?;
        let parsed = crate::prompts::parse_entities(&raw);
        let total = parsed.len();
        let found = keep_own_kind(&doc.title, parsed);
        if found.is_empty() {
            failures.push(empty_reason(&doc.title, total, &raw));
        }
        merge_entities(&mut out, found);
    }
    if out.is_empty() {
        return Err(Error::runtime(failures.join("；")));
    }
    let series = load_store(config_dir, &project_scope(project_id)).assets;
    for e in out.iter_mut() {
        let Ok(kind) = crate::studio_ops::parse_kind(&e.kind) else { continue };
        e.exists = series.iter().any(|a| a.kind == kind && a.name.trim() == e.name.trim());
    }
    Ok(out)
}

/// 同一个人的几套造型。
#[derive(Debug, Clone)]
pub struct CharacterGroup {
    /// 角色名（造型括号去掉）
    pub name: String,
    /// 主形象：名字就是角色名的那一条；没有的话是第一个造型
    pub main: crate::prompts::ExtractedEntity,
    /// 主形象的名字（「出厂陪伴款」）。主形象本来就叫角色名时是空的
    pub main_look: String,
    /// 其余造型：(形象名, 条目)
    pub looks: Vec<(String, crate::prompts::ExtractedEntity)>,
}

/// 「星澜（战斗款）」→ (星澜, 战斗款)。只认**名字以人设名开头、后面紧跟一对括号**的 ——
/// 模型把 XN-0001 的人设也标成了星澜，但它是另一台机器，名字也不是「星澜（…）」开头，不能并。
pub fn split_look(name: &str, persona: &str) -> Option<(String, String)> {
    let name = name.trim();
    let base = persona.trim();
    if base.is_empty() {
        return None;
    }
    let rest = name.strip_prefix(base)?.trim();
    let inner = rest
        .strip_prefix('（')
        .and_then(|r| r.strip_suffix('）'))
        .or_else(|| rest.strip_prefix('(').and_then(|r| r.strip_suffix(')')))?
        .trim();
    (!inner.is_empty()).then(|| (base.to_string(), inner.to_string()))
}

/// 把角色按人设分组：同一个人的几套造型合成一组（一个角色 + 几个形象）。
/// 场景、道具原样返回在第二个值里。**顺序保持提取的顺序**。
pub fn group_characters(
    es: Vec<crate::prompts::ExtractedEntity>,
) -> (Vec<CharacterGroup>, Vec<crate::prompts::ExtractedEntity>) {
    let mut groups: Vec<CharacterGroup> = Vec::new();
    let mut rest = Vec::new();
    for e in es {
        if e.kind != "character" {
            rest.push(e);
            continue;
        }
        let (base, look) = match split_look(&e.name, &e.persona) {
            Some((b, l)) => (b, Some(l)),
            None => (e.name.trim().to_string(), None),
        };
        match (groups.iter_mut().find(|g| g.name == base), look) {
            // 已有这一组，又来一个造型
            (Some(g), Some(l)) => {
                if g.main_look != l && !g.looks.iter().any(|(n, _)| *n == l) {
                    g.looks.push((l, e));
                }
            }
            // 已有这一组（先来的是造型），现在来了不带括号的本名 —— 它当主形象，原来的主形象降成形象
            (Some(g), None) => {
                if !g.main_look.is_empty() {
                    let old = std::mem::replace(&mut g.main, e);
                    let old_look = std::mem::take(&mut g.main_look);
                    g.looks.insert(0, (old_look, old));
                }
            }
            (None, look) => groups.push(CharacterGroup {
                name: base,
                main: e,
                main_look: look.unwrap_or_default(),
                looks: Vec::new(),
            }),
        }
    }
    // **本体条目没写衣服、又有造型条目的，第一套造型当主形象。**
    // 2026-09-26 实测：人物小传里写了「外貌」和「造型」两行，模型会交回一条只有长相的「陆川」
    // 加两条带衣服的「陆川（拆机工装）」「陆川（上城西装）」。本体当主形象的话，主形象没有衣服，
    // 出设定卡时模型替我们现编一套 —— 每张都不一样。本体的长相在每套造型里都有，丢掉不损失什么。
    for g in groups.iter_mut() {
        if g.main_look.is_empty() && !g.looks.is_empty() && !has_outfit(&g.main) {
            let (name, e) = g.looks.remove(0);
            g.main = e;
            g.main_look = name;
        }
    }
    (groups, rest)
}

/// 这一条写了衣服没有（槽位里上衣 / 下装 / 鞋有一项就算；没有槽位时看描述里有没有「服装」）。
pub fn has_outfit(e: &crate::prompts::ExtractedEntity) -> bool {
    match &e.spec {
        Some(s) => [&s.outfit.top, &s.outfit.bottom, &s.outfit.shoes].iter().any(|v| !v.trim().is_empty()),
        None => e.description.contains("服装"),
    }
}

/// 一组造型建成（或并进）一个角色。回新加了几个形象。
pub fn apply_looks(target: &mut Asset, group: &CharacterGroup) -> usize {
    if target.main_look_name.trim().is_empty() && !group.main_look.is_empty() && target.looks.is_empty() {
        target.main_look_name = group.main_look.clone();
    }
    let now = crate::assets::now_ms();
    let mut added = 0;
    let all = std::iter::once((&group.main_look, &group.main)).chain(group.looks.iter().map(|(n, e)| (n, e)));
    for (name, e) in all {
        if name.is_empty() || *name == target.main_look_name || target.looks.iter().any(|l| l.name == *name) {
            continue;
        }
        target.looks.push(crate::assets::Look {
            id: format!("look_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]),
            name: name.clone(),
            description: e.description.trim().to_string(),
            nsfw: false,
            unit: Default::default(),
            created_at: now,
            updated_at: now,
        });
        added += 1;
    }
    added
}

/// 一份设定文档。
#[derive(Debug, Clone, serde::Deserialize)]
pub struct DesignDoc {
    pub title: String,
    pub text: String,
}

/// 按标题判断这份文档是哪一类的设定。认不出（世界观、总纲）就三类都收。
pub fn kinds_for_title(title: &str) -> &'static [&'static str] {
    if title.contains("人物") || title.contains("角色") {
        &["character"]
    } else if title.contains("场景") {
        &["scene"]
    } else if title.contains("道具") {
        &["prop"]
    } else {
        &["character", "scene", "prop"]
    }
}

/// 一份文档什么都没收到时，说清楚是哪一种。
///
/// **「解析不出」和「解析出了但都不是这一类」是两回事。** 2026-09-26 qwen3.6-aggressive 拿场景设计
/// 交回来的全是角色，按类过滤后一个不剩 —— 报成「没从模型的回复里解析出实体」，
/// 看上去像 JSON 坏了，其实是模型没按文档类型干活，该换模型而不是去查解析器。
pub fn empty_reason(title: &str, parsed: usize, raw: &str) -> String {
    if parsed > 0 {
        let want = kinds_for_title(title)
            .iter()
            .map(|k| match *k { "character" => "角色", "scene" => "场景", _ => "道具" })
            .collect::<Vec<_>>()
            .join("/");
        format!("「{title}」：模型认出了 {parsed} 个，但没有一个是{want} —— 它没按这份文档的类型提取，换个后台模型再试")
    } else {
        let head: String = raw.chars().take(120).collect();
        format!("「{title}」：没从模型的回复里解析出实体。它返回的是：{head}")
    }
}

/// 只留这份文档那一类的。
pub fn keep_own_kind(title: &str, found: Vec<crate::prompts::ExtractedEntity>) -> Vec<crate::prompts::ExtractedEntity> {
    let keep = kinds_for_title(title);
    found.into_iter().filter(|e| keep.contains(&e.kind.as_str())).collect()
}

/// 同类同名的合成一条，描述更长的留下。
pub fn merge_entities(out: &mut Vec<crate::prompts::ExtractedEntity>, found: Vec<crate::prompts::ExtractedEntity>) {
    for e in found {
        if e.name.trim().is_empty() {
            continue;
        }
        match out.iter_mut().find(|o| o.kind == e.kind && o.name.trim() == e.name.trim()) {
            Some(o) if e.description.chars().count() > o.description.chars().count() => *o = e,
            Some(_) => {}
            None => out.push(e),
        }
    }
}

#[tauri::command]
pub async fn series_extract_from_texts(
    state: tauri::State<'_, crate::AppState>,
    project_id: String,
    docs: Vec<DesignDoc>,
) -> Result<Vec<crate::prompts::ExtractedEntity>> {
    extract_from_texts(&state.config_dir, &state.gateway, &project_id, docs).await
}

#[tauri::command]
pub fn series_assets_inbox(state: tauri::State<'_, crate::AppState>, project_id: String) -> SeriesInbox {
    inbox(&state.config_dir, &project_id)
}

#[tauri::command]
pub fn series_assets_adopt(
    state: tauri::State<'_, crate::AppState>,
    project_id: String,
    episode_id: String,
    id: String,
) -> Result<AdoptOutcome> {
    adopt(&state.config_dir, &project_id, &episode_id, &id)
}

#[tauri::command]
pub fn series_assets_adopt_all(state: tauri::State<'_, crate::AppState>, project_id: String) -> Result<AdoptSummary> {
    adopt_all(&state.config_dir, &project_id)
}

#[tauri::command]
pub fn series_assets_keep_episode(
    state: tauri::State<'_, crate::AppState>,
    project_id: String,
    episode_id: String,
    id: String,
    on: bool,
) -> Result<()> {
    set_episode_only(&state.config_dir, &project_id, &episode_id, &id, on)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::{AssetKind, MediaVariant};
    use serde_json::json;

    fn put(cfg: &Path, sc: &Scope, a: Asset) {
        let mut s = load_store(cfg, sc);
        s.assets.push(a);
        save_store(cfg, sc, &s).unwrap();
    }

    fn with_image(cfg: &Path, sc: &Scope, mut a: Asset, file: &str) -> Asset {
        let rel = format!("{}/{file}", a.id);
        let abs = sc.media_dir(cfg).join(&rel);
        std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
        std::fs::write(&abs, b"png").unwrap();
        let v: MediaVariant = serde_json::from_value(json!({"id": format!("v_{file}"), "rel_path": rel, "created_at": 0})).unwrap();
        a.unit.selected_image_id = Some(v.id.clone());
        a.unit.image_variants.push(v);
        a
    }

    fn episode(cfg: &Path, ep: &str, num: u32, shots: Value) {
        let f = episode_file(cfg, "p1", ep);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        let v = json!({"id": ep, "episode_num": num, "title": format!("第{num}集"), "storyboard": shots});
        std::fs::write(f, serde_json::to_string(&v).unwrap()).unwrap();
    }

    fn read_episode(cfg: &Path, ep: &str) -> Value {
        serde_json::from_str(&std::fs::read_to_string(episode_file(cfg, "p1", ep)).unwrap()).unwrap()
    }

    #[test]
    fn 系列里没有同名的_原样搬过去_id不变_图跟着走() {
        let d = tempfile::tempdir().unwrap();
        let cfg = d.path();
        let ep = episode_scope("p1", "ep_01");
        episode(cfg, "ep_01", 1, json!([{"id": "s1", "shot_num": 1, "character_ids": []}]));
        let a = with_image(cfg, &ep, Asset::new(AssetKind::Character, "陆川"), "a.png");
        let id = a.id.clone();
        put(cfg, &ep, a);

        let out = adopt(cfg, "p1", "ep_01", &id).unwrap();
        assert_eq!(out, AdoptOutcome::Moved { id: id.clone(), name: "陆川".into() });
        let series = load_store(cfg, &project_scope("p1")).assets;
        assert_eq!(series.len(), 1);
        assert_eq!(series[0].id, id, "搬家换了 id —— 镜头引用会全部指空");
        assert!(load_store(cfg, &ep).assets.is_empty());
        let rel = &series[0].unit.image_variants[0].rel_path;
        assert!(project_scope("p1").media_dir(cfg).join(rel).is_file(), "图没搬到系列的媒体目录");
    }

    #[test]
    fn 系列里有同名的_并进去_镜头改指过去() {
        let d = tempfile::tempdir().unwrap();
        let cfg = d.path();
        let ep = episode_scope("p1", "ep_02");
        let pr = project_scope("p1");
        let target = Asset::new(AssetKind::Character, "陆川");
        let tid = target.id.clone();
        put(cfg, &pr, target);
        let mut src = with_image(cfg, &ep, Asset::new(AssetKind::Character, " 陆川 "), "b.png");
        src.description = "回收师".into();
        let sid = src.id.clone();
        put(cfg, &ep, src);
        episode(
            cfg,
            "ep_02",
            2,
            json!([
                {"id": "s1", "shot_num": 1, "character_ids": [sid], "ref_slots": [{"asset_id": sid}], "look_overrides": {sid.clone(): "look_x"}},
                {"id": "s2", "shot_num": 2, "character_ids": [tid, sid]},
                {"id": "s3", "shot_num": 3, "character_ids": ["别人"]}
            ]),
        );

        let out = adopt(cfg, "p1", "ep_02", &sid).unwrap();
        assert!(matches!(out, AdoptOutcome::Absorbed { ref into_id, shots: 2, .. } if *into_id == tid), "{out:?}");
        let series = load_store(cfg, &pr).assets;
        assert_eq!(series.len(), 1, "并完系列里多出了一个");
        assert_eq!(series[0].description, "回收师", "空着的描述没补上");
        assert_eq!(series[0].unit.image_variants.len(), 1, "本集那份的图没带过去");
        assert_eq!(series[0].unit.selected_image_id.as_deref(), Some("v_b.png"));
        assert!(load_store(cfg, &ep).assets.is_empty());

        let v = read_episode(cfg, "ep_02");
        assert_eq!(v["storyboard"][0]["character_ids"], json!([tid]));
        assert_eq!(v["storyboard"][0]["ref_slots"][0]["asset_id"], json!(tid));
        assert!(v["storyboard"][0]["look_overrides"].get(&sid).is_none(), "源资产的形象覆盖还留着");
        assert_eq!(v["storyboard"][1]["character_ids"], json!([tid]), "同一镜里出现两次");
        assert_eq!(v["storyboard"][2]["character_ids"], json!(["别人"]));
    }

    #[test]
    fn 不覆盖系列里已经写好的描述() {
        let mut t = Asset::new(AssetKind::Character, "星澜");
        t.description = "用户改过的".into();
        let mut s = Asset::new(AssetKind::Character, "星澜");
        s.description = "模型抽的".into();
        fill_missing(&mut t, &s);
        assert_eq!(t.description, "用户改过的");
    }

    #[test]
    fn 同名不同类不合并() {
        let a = Asset::new(AssetKind::Prop, "铜壶");
        let b = Asset::new(AssetKind::Scene, "铜壶");
        assert!(!same_entity(&a, &b));
        assert!(same_entity(&a, &Asset::new(AssetKind::Prop, "铜壶 ")));
    }

    #[test]
    fn 全部收进系列_跨集同名的并成一个_只留本集的不动() {
        let d = tempfile::tempdir().unwrap();
        let cfg = d.path();
        episode(cfg, "ep_01", 1, json!([]));
        episode(cfg, "ep_02", 2, json!([]));
        let e1 = episode_scope("p1", "ep_01");
        let e2 = episode_scope("p1", "ep_02");
        put(cfg, &e1, Asset::new(AssetKind::Character, "陆川"));
        put(cfg, &e2, Asset::new(AssetKind::Character, "陆川"));
        let mut passer = Asset::new(AssetKind::Character, "路人甲");
        passer.episode_only = true;
        put(cfg, &e2, passer);

        let sum = adopt_all(cfg, "p1").unwrap();
        assert_eq!(sum, AdoptSummary { moved: 1, absorbed: 1, kept: 1 });
        let series = load_store(cfg, &project_scope("p1")).assets;
        assert_eq!(series.len(), 1);
        assert_eq!(load_store(cfg, &e2).assets.len(), 1, "只留本集的被收走了");
        assert!(cfg.join("backups").read_dir().unwrap().next().is_some(), "没备份");
    }

    #[test]
    fn 待归属清单_分开只留本集的_并标出会并进谁() {
        let d = tempfile::tempdir().unwrap();
        let cfg = d.path();
        episode(cfg, "ep_01", 1, json!([]));
        let e1 = episode_scope("p1", "ep_01");
        let t = Asset::new(AssetKind::Character, "夏予安");
        let tid = t.id.clone();
        put(cfg, &project_scope("p1"), t);
        put(cfg, &e1, Asset::new(AssetKind::Character, "夏予安"));
        let mut k = Asset::new(AssetKind::Prop, "旧手机");
        k.episode_only = true;
        put(cfg, &e1, k);

        let ib = inbox(cfg, "p1");
        assert_eq!(ib.pending.len(), 1);
        assert_eq!(ib.pending[0].match_id.as_deref(), Some(tid.as_str()));
        assert_eq!(ib.pending[0].episode_title, "第1集");
        assert_eq!(ib.kept.len(), 1);
    }

    fn 抽(kind: &str, name: &str, desc: &str) -> crate::prompts::ExtractedEntity {
        crate::prompts::ExtractedEntity {
            anchors: None, kind: kind.into(), name: name.into(), description: desc.into(),
            persona: String::new(), exists: false, disembodied: false, incidental: false, spec: None,
        }
    }

    fn 人(name: &str, persona: &str, desc: &str) -> crate::prompts::ExtractedEntity {
        let mut e = 抽("character", name, desc);
        e.persona = persona.into();
        e
    }

    #[test]
    fn 造型名拆得出来_人设不是名字开头的不拆() {
        assert_eq!(split_look("星澜（战斗款）", "星澜"), Some(("星澜".into(), "战斗款".into())));
        assert_eq!(split_look("陆川(上城伪装)", "陆川"), Some(("陆川".into(), "上城伪装".into())));
        assert_eq!(split_look("XN-0001", "星澜"), None, "XN-0001 是另一台机器");
        assert_eq!(split_look("星澜", "星澜"), None);
        assert_eq!(split_look("星澜（）", "星澜"), None);
    }

    #[test]
    fn 同一个人的几套造型合成一组() {
        let (g, rest) = group_characters(vec![
            人("星澜（出厂陪伴款）", "星澜", "粉白"),
            人("陆川", "陆川", "工装"),
            人("星澜（战斗款）", "星澜", "冰蓝"),
            抽("scene", "巷口", ""),
            人("XN-0001", "星澜", "积灰"),
            人("星澜（战斗款）", "星澜", "重复的"),
        ]);
        assert_eq!(rest.len(), 1);
        let names: Vec<&str> = g.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["星澜", "陆川", "XN-0001"]);
        assert_eq!(g[0].main_look, "出厂陪伴款");
        assert_eq!(g[0].looks.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["战斗款"], "重复的造型又加了一次");
    }

    #[test]
    fn 后来的本名当主形象_先来的造型降成形象() {
        let (g, _) = group_characters(vec![人("夏予安（便装）", "夏予安", "服装：连帽衫"), 人("夏予安", "夏予安", "服装：制服")]);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].main.description, "服装：制服");
        assert!(g[0].main_look.is_empty());
        assert_eq!(g[0].looks[0].0, "便装");
    }

    #[test]
    fn 本体没写衣服_第一套造型当主形象() {
        let (g, _) = group_characters(vec![
            人("陆川", "陆川", "长方脸"),
            人("陆川（拆机工装）", "陆川", "长方脸。服装：旧工装外套"),
            人("陆川（上城西装）", "陆川", "长方脸。服装：借来的西装"),
        ]);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].main_look, "拆机工装", "主形象没有衣服 —— 出设定卡时模型会现编一套");
        assert!(g[0].main.description.contains("工装"));
        assert_eq!(g[0].looks.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["上城西装"]);
        // 本体写了衣服的，照旧当主形象
        let (g, _) = group_characters(vec![人("星澜", "星澜", "服装：出厂陪伴服"), 人("星澜（战斗款）", "星澜", "服装：硬化外壳")]);
        assert!(g[0].main_look.is_empty());
    }

    #[test]
    fn 造型加成形象_已有的不重复加() {
        let (g, _) = group_characters(vec![人("星澜（出厂陪伴款）", "星澜", "粉白"), 人("星澜（战斗款）", "星澜", "冰蓝")]);
        let mut a = Asset::new(AssetKind::Character, "星澜");
        assert_eq!(apply_looks(&mut a, &g[0]), 1);
        assert_eq!(a.main_look_name, "出厂陪伴款");
        assert_eq!(a.looks[0].name, "战斗款");
        assert_eq!(a.looks[0].description, "冰蓝");
        assert_eq!(apply_looks(&mut a, &g[0]), 0, "第二次又加了一遍");
    }

    #[test]
    fn 解析出了但类型不对_和解析不出分开说() {
        let m = empty_reason("05-场景设计", 20, "{...}");
        assert!(m.contains("认出了 20 个") && m.contains("场景"), "{m}");
        assert!(!m.contains("没从模型的回复里解析出"), "类型不对被报成了解析失败");
        assert!(empty_reason("05-场景设计", 0, "乱码").contains("没从模型的回复里解析出实体"));
    }

    #[test]
    fn 设定文档按标题只收自己那一类() {
        assert_eq!(kinds_for_title("02-人物小传"), &["character"]);
        assert_eq!(kinds_for_title("角色表"), &["character"]);
        assert_eq!(kinds_for_title("05-场景设计"), &["scene"]);
        assert_eq!(kinds_for_title("06-道具设计"), &["prop"]);
        assert_eq!(kinds_for_title("01-世界观设定").len(), 3, "认不出的三类都收");
        let got = keep_own_kind("05-场景设计", vec![抽("scene", "巷口", ""), 抽("character", "孙富豪", ""), 抽("prop", "鸡腿", "")]);
        assert_eq!(got.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["巷口"], "场景设计里顺带写到的人和道具混进来了");
    }

    #[test]
    fn 几份文档抽出来的同名实体合成一条_描述长的留下() {
        let mut out = vec![抽("character", "陆川", "回收师")];
        merge_entities(
            &mut out,
            vec![
                抽("character", "陆川 ", "二十四岁，云穹回收站夜班回收师，旧工装外套"),
                抽("prop", "陆川", "不该并进角色"),
                抽("scene", "巷口", "窄巷"),
                抽("scene", " ", "空名字丢掉"),
            ],
        );
        assert_eq!(out.len(), 3);
        assert!(out[0].description.starts_with("二十四岁"), "短的把长的盖掉了");
    }

    #[test]
    fn 只留本集的标记能打能取消() {
        let d = tempfile::tempdir().unwrap();
        let cfg = d.path();
        let e1 = episode_scope("p1", "ep_01");
        let a = Asset::new(AssetKind::Scene, "巷口");
        let id = a.id.clone();
        put(cfg, &e1, a);
        set_episode_only(cfg, "p1", "ep_01", &id, true).unwrap();
        assert!(load_store(cfg, &e1).assets[0].episode_only);
        set_episode_only(cfg, "p1", "ep_01", &id, false).unwrap();
        assert!(!load_store(cfg, &e1).assets[0].episode_only);
    }
}

#[cfg(test)]
mod live {
    /// 实跑：用剧本编辑器里某个项目的设定文档走一遍「从设定文档建资产」的提取。**不落盘**。
    /// `ANIMA_LIVE_STORY`=剧本项目名（默认 回收师），`ANIMA_LIVE_DOCS`=逗号分隔的文档标题前缀（默认 02,05,06）。
    #[tokio::test]
    #[ignore = "要本机后台模型和真实剧本库"]
    async fn 从设定文档提取实跑() {
        let config_dir = dirs::config_dir().unwrap().join("com.anima.desktop");
        let story = std::env::var("ANIMA_LIVE_STORY").unwrap_or_else(|_| "回收师".into());
        let prefixes: Vec<String> = std::env::var("ANIMA_LIVE_DOCS").unwrap_or_else(|_| "02,05,06".into()).split(',').map(String::from).collect();
        let dir = config_dir.join("studio").join("scripts");
        let projects: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("projects.json")).unwrap()).unwrap();
        let pid = projects.as_array().unwrap().iter().find(|p| p["title"] == story.as_str()).expect("没有这个剧本项目")["id"].as_str().unwrap().to_string();
        let mut texts = Vec::new();
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let Ok(v) = std::fs::read_to_string(e.path()).map(|t| serde_json::from_str::<serde_json::Value>(&t).unwrap_or_default()) else { continue };
            let title = v["title"].as_str().unwrap_or("");
            if v["project_id"] == pid.as_str() && prefixes.iter().any(|p| title.starts_with(p.as_str())) {
                println!("文档：{title}（{} 字）", v["text"].as_str().unwrap_or("").chars().count());
                texts.push(super::DesignDoc { title: title.to_string(), text: v["text"].as_str().unwrap_or("").to_string() });
            }
        }
        let mut app_cfg = crate::config::AppConfig::load(&config_dir.join("config.json")).unwrap();
        // `ANIMA_LIVE_MODEL`：换后台模型比速度和质量（同一个 LM Studio 里要先加载好）
        if let Ok(m) = std::env::var("ANIMA_LIVE_MODEL") {
            app_cfg.background_model = Some(m);
        }
        println!("后台模型：{:?}", app_cfg.background_model);
        let gateway = crate::llm::gateway::Gateway::new(app_cfg).unwrap();
        let t = std::time::Instant::now();
        let mut es = Vec::new();
        for doc in texts {
            let t1 = std::time::Instant::now();
            let title = doc.title.clone();
            let got = super::extract_from_texts(&config_dir, &gateway, "__live__", vec![doc]).await.unwrap();
            println!("  {title}：{:.0}s，{} 个", t1.elapsed().as_secs_f32(), got.len());
            super::merge_entities(&mut es, got);
        }
        println!("用时 {:.0}s，共 {} 个", t.elapsed().as_secs_f32(), es.len());
        for k in ["character", "scene", "prop"] {
            let names: Vec<&str> = es.iter().filter(|e| e.kind == k).map(|e| e.name.as_str()).collect();
            println!("{k} {}：{}", names.len(), names.join("、"));
        }
        let out = std::env::temp_dir().join(format!(
            "anima_design_extract_{}.json",
            std::env::var("ANIMA_LIVE_MODEL").unwrap_or_else(|_| "default".into())
        ));
        std::fs::write(&out, serde_json::to_string_pretty(&es).unwrap()).unwrap();
        println!("明细：{}", out.display());
    }
}
