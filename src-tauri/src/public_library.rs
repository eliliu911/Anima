//! 公共素材库：随手可用、不用生成的现成素材（第一批是 Uncle Mugen 的 527 张视觉小说背景）。
//!
//! # 放在哪
//!
//! `~/AI/assets/public/<合集>/`（可用 `ANIMA_PUBLIC_ASSETS` 改）。**不进 git**（一个合集就 100 MB），
//! 每个合集一份 `source.json`：名字、作者、来源、授权、目录的中文分类名。来历记在 `docs/ASSET_LEDGER.md`。
//!
//! # 和资产库的关系
//!
//! 公共库只读。在资产库「公共素材」页签里挑一张 →「加入」→ 复制进全局库或某个项目，成为普通的场景资产，
//! 之后出分镜、@ 引用都和自己建的一样。**不直接把 527 张导成资产** —— 那样自己的资产会被淹没。
//!
//! # 同一个地方的不同时段合成一个
//!
//! 这批图大量是同一机位的日 / 夕 / 夜（`Classroom_01_day` / `_evening` / `_night`、`room_dawn_light_off`）。
//! 按文件名里第一个时段词切开：前面是地点，后面是时段和其余修饰。加入时整组进同一个资产，每个时段一张变体。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::assets::{self, Asset, AssetKind, Scope};
use crate::error::{Error, Result};

pub fn root() -> PathBuf {
    std::env::var_os("ANIMA_PUBLIC_ASSETS")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join("AI/assets/public")))
        .unwrap_or_else(|| PathBuf::from("/nonexistent-public-assets"))
}

#[derive(Debug, Default, Deserialize)]
struct SourceMeta {
    #[serde(default)]
    name: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    license: String,
    /// 相对合集目录的文件夹 → 中文分类名
    #[serde(default)]
    categories: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Collection {
    pub id: String,
    pub name: String,
    pub author: String,
    pub url: String,
    pub license: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicVariant {
    /// 「白天」「黄昏 · 关灯」；没有时段词的是空串
    pub label: String,
    pub path: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicScene {
    /// `合集/文件夹/地点`，加入时按它找回来
    pub id: String,
    pub collection: String,
    pub category: String,
    pub name: String,
    /// 封面在第一个（白天优先）
    pub variants: Vec<PublicVariant>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicLibrary {
    pub root: String,
    pub collections: Vec<Collection>,
    pub scenes: Vec<PublicScene>,
}

const TIMES: &[(&str, &str)] = &[
    ("day", "白天"), ("noon", "正午"), ("morning", "早晨"), ("afternoon", "下午"), ("sunrise", "日出"), ("dawn", "黎明"),
    ("evening", "傍晚"), ("dusk", "黄昏"), ("sunset", "日落"), ("night", "夜晚"),
];
const WORDS: &[(&str, &str)] = &[("light", "灯"), ("on", "开"), ("off", "关"), ("fog", "雾"), ("rain", "雨"), ("snow", "雪")];

/// 文件名切成词：下划线、空格、横线分开，字母和数字之间也分开（`afternoon01` → afternoon, 01）；去掉「copy」
fn tokens(stem: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in stem.split(|c: char| c == '_' || c == ' ' || c == '-').filter(|p| !p.is_empty()) {
        let mut cur = String::new();
        for c in part.chars() {
            if !cur.is_empty() && cur.chars().last().is_some_and(|l| l.is_ascii_digit() != c.is_ascii_digit()) {
                out.push(std::mem::take(&mut cur));
            }
            cur.push(c);
        }
        out.push(cur);
    }
    out.retain(|t| !t.eq_ignore_ascii_case("copy"));
    out
}

fn time_rank(t: &str) -> Option<usize> {
    TIMES.iter().position(|(k, _)| t.eq_ignore_ascii_case(k))
}

/// `(地点, 时段标签)`。时段词在最前（`afternoon01`）时地点是其余部分
pub fn split_time(stem: &str) -> (String, String) {
    let t = tokens(stem);
    let Some(i) = t.iter().position(|x| time_rank(x).is_some()) else {
        return (t.join(" "), String::new());
    };
    // 时段在最前（afternoon01）：其余部分是机位，编号也属于机位（afternoon01 和 day01 是同一个机位）
    let (base, rest) = if i == 0 { (t[1..].join(" "), &t[..1]) } else { (t[..i].join(" "), &t[i..]) };
    let label = rest
        .iter()
        .map(|w| {
            TIMES.iter().chain(WORDS).find(|(k, _)| w.eq_ignore_ascii_case(k)).map(|(_, z)| z.to_string()).unwrap_or_else(|| w.to_string())
        })
        .collect::<Vec<_>>();
    // 「灯 开」→「开灯」
    let label = label.join(" ").replace("灯 开", "开灯").replace("灯 关", "关灯");
    (base, label.split(' ').filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · "))
}

/// 给人看的名字：去掉 `bg` 前缀，词之间空格
fn display_name(base: &str) -> String {
    let t: Vec<&str> = base.split(' ').filter(|w| !w.is_empty()).collect();
    let t: Vec<&str> = if t.first().is_some_and(|w| w.eq_ignore_ascii_case("bg")) && t.len() > 1 { t[1..].to_vec() } else { t };
    t.join(" ")
}

fn cover_rank(label: &str) -> usize {
    // 白天 → 正午 → 早晨 → 下午 → 没写时段 → 其余
    ["白天", "正午", "早晨", "下午"].iter().position(|k| label.starts_with(k)).unwrap_or(if label.is_empty() { 4 } else { 5 })
}

pub fn scan(root: &Path) -> PublicLibrary {
    let mut collections = Vec::new();
    let mut scenes: BTreeMap<String, PublicScene> = BTreeMap::new();
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root).map(|r| r.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect()).unwrap_or_default();
    dirs.sort();
    for dir in dirs {
        let cid = dir.file_name().unwrap_or_default().to_string_lossy().to_string();
        if cid.starts_with('_') || cid.starts_with('.') {
            continue;
        }
        let meta: SourceMeta = std::fs::read_to_string(dir.join("source.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        collections.push(Collection { id: cid.clone(), name: if meta.name.is_empty() { cid.clone() } else { meta.name.clone() }, author: meta.author.clone(), url: meta.url.clone(), license: meta.license.clone() });
        let mut files = Vec::new();
        collect_images(&dir, &mut files);
        files.sort();
        for f in files {
            let rel_dir = f.parent().and_then(|p| p.strip_prefix(&dir).ok()).map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
            let category = meta.categories.get(&rel_dir).cloned().unwrap_or_else(|| rel_dir.rsplit('/').next().unwrap_or("").to_string());
            let stem = f.file_stem().unwrap_or_default().to_string_lossy().to_string();
            let (base, label) = split_time(&stem);
            let id = format!("{cid}/{rel_dir}/{base}");
            let name = match display_name(&base) {
                n if n.is_empty() || n.chars().all(|c| c.is_ascii_digit() || c == ' ') => format!("{category} {n}").trim().to_string(),
                n => n,
            };
            let scene = scenes.entry(id.clone()).or_insert_with(|| PublicScene { id, collection: cid.clone(), category, name, variants: Vec::new() });
            scene.variants.push(PublicVariant { label, path: f.to_string_lossy().to_string(), url: assets::media_url(&f) });
        }
    }
    let mut scenes: Vec<PublicScene> = scenes.into_values().collect();
    for s in &mut scenes {
        s.variants.sort_by_key(|v| cover_rank(&v.label));
    }
    PublicLibrary { root: root.to_string_lossy().to_string(), collections, scenes }
}

fn collect_images(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_images(&p, out);
        } else if p.extension().and_then(|x| x.to_str()).is_some_and(|x| matches!(x.to_ascii_lowercase().as_str(), "webp" | "png" | "jpg" | "jpeg")) {
            out.push(p);
        }
    }
}

/// 把一个公共场景复制进资产库，成为普通场景资产。每个时段一张变体，封面是白天那张
pub fn import(config_dir: &Path, root: &Path, scene_id: &str, scope: &Scope, name: &str) -> Result<Asset> {
    let lib = scan(root);
    let scene = lib.scenes.into_iter().find(|s| s.id == scene_id).ok_or_else(|| Error::config("公共素材库里没有这一项（文件可能被移走了）"))?;
    let name = if name.trim().is_empty() { scene.name.clone() } else { name.trim().to_string() };
    let mut store = assets::load_store(config_dir, scope);
    if store.assets.iter().any(|a| a.kind == AssetKind::Scene && a.name.trim() == name) {
        return Err(Error::config(format!("这里已经有叫「{name}」的场景了，换个名字")));
    }
    let collection = lib.collections.iter().find(|c| c.id == scene.collection).map(|c| c.name.clone()).unwrap_or_default();
    let mut asset = Asset::new(AssetKind::Scene, &name);
    // 封面（白天）最后加：最后加的那张是选中的
    for v in scene.variants.iter().rev() {
        let bytes = std::fs::read(&v.path).map_err(|e| Error::config(format!("读不到素材文件 {}：{e}", v.path)))?;
        let ext = Path::new(&v.path).extension().and_then(|x| x.to_str()).unwrap_or("webp").to_string();
        let note = format!("公共素材 · {collection} · {}{}", scene.category, if v.label.is_empty() { String::new() } else { format!(" · {}", v.label) });
        let mut added = assets::add_variant(config_dir, scope, &mut asset, &bytes, &ext, false, Some(note), true, None)?;
        // 素材不是抽卡的产物：收藏起来，免得超过变体上限时被自动清掉
        added.favorited = true;
        if let Some(x) = asset.unit.image_variants.iter_mut().find(|x| x.id == added.id) {
            x.favorited = true;
        }
    }
    store.assets.push(asset.clone());
    assets::save_store(config_dir, scope, &store)?;
    Ok(asset)
}

#[tauri::command]
pub fn public_library_list() -> PublicLibrary {
    scan(&root())
}

#[tauri::command]
pub fn public_library_import(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
    scene_id: String,
    scope: Scope,
    name: String,
) -> Result<assets::AssetView> {
    let asset = import(&state.config_dir, &root(), &scene_id, &scope, &name)?;
    let pid = match &scope {
        Scope::Global => None,
        Scope::Project { project_id } | Scope::Episode { project_id, .. } => Some(project_id.as_str()),
    };
    crate::studio_ops::emit_changed(&app, "assets", pid);
    Ok(assets::to_view(&state.config_dir, &scope, asset, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 本机真实素材库的分组概况（`cargo test --lib public_library::tests::本机素材库 -- --ignored --nocapture`）
    #[test]
    #[ignore]
    fn 本机素材库() {
        let lib = scan(&root());
        let files: usize = lib.scenes.iter().map(|s| s.variants.len()).sum();
        println!("{} 个合集，{} 组，{} 张", lib.collections.len(), lib.scenes.len(), files);
        let mut by: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
        for s in &lib.scenes {
            let e = by.entry(&s.category).or_default();
            e.0 += 1;
            e.1 += s.variants.len();
        }
        println!("{by:?}");
        for s in lib.scenes.iter().filter(|s| s.variants.len() > 3).take(25) {
            println!("{} | {} | {:?}", s.category, s.name, s.variants.iter().map(|v| v.label.as_str()).collect::<Vec<_>>());
        }
    }

    #[test]
    fn 按第一个时段词切开地点和时段() {
        assert_eq!(split_time("Classroom_01_day"), ("Classroom 01".into(), "白天".into()));
        assert_eq!(split_time("room_dawn_light_off"), ("room".into(), "黎明 · 关灯".into()));
        assert_eq!(split_time("room_noon_light_on"), ("room".into(), "正午 · 开灯".into()));
        assert_eq!(split_time("afternoon01"), ("01".into(), "下午".into()));
        assert_eq!(split_time("Auditorium_Outside_Sunrise"), ("Auditorium Outside".into(), "日出".into()));
        assert_eq!(split_time("mall_a_01 copy"), ("mall a 01".into(), String::new()));
        assert_eq!(split_time("marina_fog"), ("marina fog".into(), String::new()), "雾不是时段，不拆");
        assert_eq!(split_time("condo_Day 01"), ("condo".into(), "白天 · 01".into()));
    }

    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("public");
        let c = root.join("mugen/pack1/School");
        std::fs::create_dir_all(&c).unwrap();
        for f in ["Classroom_01_night.webp", "Classroom_01_day.webp", "Classroom_01_evening.webp", "bg_hospital_room.webp", "Terms of Usage.txt"] {
            std::fs::write(c.join(f), f.as_bytes()).unwrap();
        }
        let m = root.join("mugen/pack2/Megalopolitan Education");
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(m.join("day01.webp"), b"d").unwrap();
        std::fs::write(m.join("afternoon01.webp"), b"a").unwrap();
        std::fs::create_dir_all(root.join("_zip")).unwrap();
        std::fs::write(root.join("_zip/x.webp"), b"x").unwrap();
        std::fs::write(root.join("mugen/source.json"), r#"{"name":"Uncle Mugen","categories":{"pack1/School":"学校","pack2/Megalopolitan Education":"都市校园"}}"#).unwrap();
        (d, root)
    }

    #[test]
    fn 扫描_同地点合成一组_白天当封面_分类用中文() {
        let (_d, root) = fixture();
        let lib = scan(&root);
        assert_eq!(lib.collections.len(), 1, "下划线开头的目录不算合集");
        let names: Vec<_> = lib.scenes.iter().map(|s| (s.name.as_str(), s.category.as_str(), s.variants.len())).collect();
        assert_eq!(names, [("Classroom 01", "学校", 3), ("hospital room", "学校", 1), ("都市校园 01", "都市校园", 2)]);
        let class = &lib.scenes[0];
        assert_eq!(class.variants.iter().map(|v| v.label.as_str()).collect::<Vec<_>>(), ["白天", "傍晚", "夜晚"]);
        assert!(class.variants[0].url.starts_with("anima-media://"));
    }

    #[test]
    fn 加入资产库_整组进一个场景_封面白天_变体收藏_重名拒绝() {
        let (d, root) = fixture();
        let cfg = d.path().join("cfg");
        let scope = Scope::Project { project_id: "p1".into() };
        let a = import(&cfg, &root, "mugen/pack1/School/Classroom 01", &scope, "教室").unwrap();
        assert_eq!((a.kind, a.name.as_str(), a.unit.image_variants.len()), (AssetKind::Scene, "教室", 3));
        let cover = a.unit.selected_image().unwrap();
        assert!(cover.prompt_used.as_deref().unwrap().ends_with("学校 · 白天"), "{:?}", cover.prompt_used);
        assert!(a.unit.image_variants.iter().all(|v| v.favorited));
        assert!(a.description.is_empty(), "描述会进出图提示词，不写来源");
        let stored = assets::load_store(&cfg, &scope).assets;
        assert_eq!(stored.len(), 1);
        assert_eq!(std::fs::read(assets::variant_abs_path(&cfg, &scope, cover)).unwrap(), b"Classroom_01_day.webp");
        assert!(import(&cfg, &root, "mugen/pack1/School/Classroom 01", &scope, "教室").is_err());
        assert!(import(&cfg, &root, "mugen/nope", &scope, "").is_err());
    }
}
