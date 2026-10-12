//! 资产批量出图的后台任务（`JobKind::AssetImages`）。
//!
//! # 为什么搬到后端
//!
//! 以前是资产页的前端循环：切到别的步骤、页面刷新，循环就断了，右下角那张卡停在「出图中 3/11」。
//! Chat 发起出图时根本没有页面在跑循环。现在和合成一集一样：登记任务、后台串行出、每出一个发
//! `studio://changed`，资产页和右下角都从任务表读。
//!
//! 提示词、版式、画幅、风格和工作台同一套（`asset_prompt`，和前端逐字对拍）。
//!
//! # 和前端那版的两处不同
//!
//! - **形象也出**：角色的非主形象没图时单独算一个（主形象判据是资产封面，其余是形象自己的封面）。
//! - **描述带上「人物本身」**（`look_description`）：前端那版只读服装描述，脸型年龄没进提示词。

use std::path::Path;

use serde::Serialize;
use serde_json::json;

use crate::asset_prompt::{self as ap, Kind};
use crate::assets::{self, Asset, AssetKind, Scope};
use crate::error::{Error, Result};
use crate::jobs::{self, JobKind};

/// 一张要出的图
#[derive(Debug, Clone, Serialize)]
pub struct PlanItem {
    #[serde(skip)]
    pub scope: Scope,
    pub asset_id: String,
    /// 非主形象才有
    #[serde(skip_serializing_if = "Option::is_none")]
    pub look_id: Option<String>,
    /// 给人看的：「林婉清」「林婉清 · 现代」
    pub label: String,
    pub kind: AssetKind,
    #[serde(skip)]
    pub mode: &'static str,
    pub template: &'static str,
    #[serde(skip)]
    pub width: u32,
    #[serde(skip)]
    pub height: u32,
    #[serde(skip)]
    pub prompt: String,
    #[serde(skip)]
    pub negative: String,
    #[serde(skip)]
    pub viewpoints: Vec<(String, String)>,
}

/// 这一批用的引擎：项目的「资产出图」引擎，读不到用 Z-Image
pub fn project_engine(config_dir: &Path, project_id: &str) -> String {
    std::fs::read_to_string(crate::cinema::cinema_projects_dir(config_dir).join(project_id).join("project.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v["model_settings"]["t2i_engine"].as_str().map(str::to_string))
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "zimage".into())
}

fn kind_order(k: AssetKind) -> usize {
    match k {
        AssetKind::Character => 0,
        AssetKind::Scene => 1,
        AssetKind::Prop => 2,
    }
}

/// `only` 里写的名字 / id 对不对得上这个资产（或它的某个形象）
fn wanted(only: &[String], asset: &Asset, look: Option<(&str, &str)>) -> bool {
    if only.is_empty() {
        return true;
    }
    only.iter().map(|s| s.trim()).any(|w| {
        let asset_hit = w == asset.id || w == asset.name.trim();
        match look {
            // 点名了资产 → 它的所有形象；点名了形象（id、形象名、「角色 · 形象」）→ 只这个
            Some((id, name)) => asset_hit || w == id || w == name.trim() || w == format!("{} · {}", asset.name.trim(), name.trim()),
            None => asset_hit,
        }
    })
}

/// 算一个形象要怎么出
pub fn item_for(scope: &Scope, asset: &Asset, look: Option<(&str, &str)>, engine: &str, style_pos: &str, style_neg: &str) -> PlanItem {
    let kind = Kind::from_asset(asset.kind);
    let description = asset.look_description(look.map(|l| l.0));
    // **没有身体的角色走头像版式。** 人物设定图那段构图本身就会画出一个人，
    // 描述里写「蓝色卡通 AI 助理头像」也盖不过它（见 `character_template_by_body`）。
    let template = if kind == Kind::Character { ap::character_template_by_body(engine, &description) } else { "simple" };
    let prompt = ap::final_positive(&ap::build_prompt(kind, &asset.name, &description, template), style_pos, true, kind);
    let negative = ap::final_negative(style_neg, ap::template_negative(kind, template), true);
    let (mode, (width, height), viewpoints) = match kind {
        // 宫格画幅写死横幅，和工作台那条路一致
        Kind::Scene => ("scene_sheet", (1536, 1024), ap::default_viewpoints(asset.scene.as_ref())),
        Kind::Character => ("t2i", crate::studio::size_for_aspect(ap::template_aspect(template)).unwrap_or((1024, 1024)), Vec::new()),
        Kind::Prop => ("t2i", crate::studio::size_for_aspect("square").unwrap_or((1024, 1024)), Vec::new()),
    };
    PlanItem {
        scope: scope.clone(),
        asset_id: asset.id.clone(),
        look_id: look.map(|l| l.0.to_string()),
        label: match look {
            Some((_, name)) => format!("{} · {}", asset.name.trim(), name.trim()),
            None => asset.name.trim().to_string(),
        },
        kind: asset.kind,
        mode,
        template,
        width,
        height,
        prompt,
        negative,
        viewpoints,
    }
}

/// 这一批要出哪些。本集那一层在前、项目那一层在后；角色 → 场景 → 道具。
///
/// - `redo = false`：只出还没图的（主形象看资产封面，其余形象看形象自己的封面）
/// - `redo = true`：全部重出一遍（加一版，旧的留在变体里）
/// - `only`：只出点了名的（资产名 / id、形象名 / id、「角色 · 形象」）
pub fn plan(config_dir: &Path, project_id: &str, episode_id: Option<&str>, redo: bool, only: &[String]) -> Vec<PlanItem> {
    let engine = project_engine(config_dir, project_id);
    let art = crate::style::load_art(config_dir, project_id);
    let (pos, neg) = (art.effective_positive(), art.effective_negative());
    let mut scopes = Vec::new();
    if let Some(ep) = episode_id.filter(|e| !e.is_empty()) {
        scopes.push(Scope::Episode { project_id: project_id.into(), episode_id: ep.into() });
    }
    scopes.push(Scope::Project { project_id: project_id.into() });

    let mut out = Vec::new();
    for scope in &scopes {
        for a in assets::load_store(config_dir, scope).assets {
            // **标了「从不出镜」的角色不出图。**
            //
            // 它根本没有能拍出来的身体（旁白员、电话那头的人、纯语音 AI），
            // 给它出一张立绘既浪费一次生成，出来的图也没有任何地方会用 ——
            // 分镜不给它镜头（`episode_cast` 把它排除掉），
            // 提示词里它只按文字出现。
            //
            // 2026-09-19 用户在 E05 上撞到：小青、小橙标了从不出镜，
            // 横幅照样说「还有 4 个角色形象没有设定图」，
            // 而「一键全出」会真的给它们排两次生成。
            //
            // **`only` 点名时例外**：用户明确点了它的名，那是他的决定。
            if a.character.as_ref().is_some_and(|c| c.disembodied) && only.is_empty() {
                continue;
            }
            if (redo || a.unit.selected_image().is_none()) && wanted(only, &a, None) {
                out.push(item_for(scope, &a, None, &engine, &pos, &neg));
            }
            for l in &a.looks {
                if (redo || l.unit.selected_image().is_none()) && wanted(only, &a, Some((&l.id, &l.name))) {
                    out.push(item_for(scope, &a, Some((&l.id, &l.name)), &engine, &pos, &neg));
                }
            }
        }
    }
    out.sort_by_key(|i| kind_order(i.kind));
    out
}

/// 按类说清楚这一批：「3 个角色、2 个场景」
pub fn describe(items: &[PlanItem]) -> String {
    let n = |k: AssetKind| items.iter().filter(|i| i.kind == k).count();
    [(n(AssetKind::Character), "个角色形象"), (n(AssetKind::Scene), "个场景"), (n(AssetKind::Prop), "个道具")]
        .into_iter()
        .filter(|(c, _)| *c > 0)
        .map(|(c, w)| format!("{c} {w}"))
        .collect::<Vec<_>>()
        .join("、")
}

/// 出一张并入库。回入库了几张
async fn run_one(config_dir: &Path, item: &PlanItem, engine: &str) -> std::result::Result<usize, String> {
    let args: crate::studio::GenerateArgs = serde_json::from_value(json!({
        "mode": item.mode,
        "engine": engine,
        "prompt": item.prompt,
        "negative_prompt": item.negative,
        "width": item.width,
        "height": item.height,
        "images": [],
        // 资产图的画风由项目风格决定：设置里常挂的 LoRA 不挂（见 GenerateArgs::auto_loras）
        "loras": [],
        "auto_loras": false,
        "viewpoints": item.viewpoints.iter().map(|(n, d)| json!({"name": n, "description": d})).collect::<Vec<_>>(),
    }))
    .map_err(|e| format!("参数拼装失败：{e}"))?;
    let out = crate::studio::generate_with(config_dir, args).await.map_err(|e| e.to_string())?;
    if !out.ok {
        return Err(if out.message.trim().is_empty() { "引擎没有返回图片".into() } else { out.message });
    }
    let files: Vec<String> = if !out.paths.is_empty() { out.paths } else { out.path.into_iter().collect() };
    if files.is_empty() {
        return Err("没有产出文件".into());
    }
    add_files(config_dir, item, &files).map_err(|e| e.to_string())?;
    Ok(files.len())
}

/// 出好的文件挂到资产（形象）上。**现读现写**：出图要一两分钟，这期间用户可能改了资产
pub fn add_files(config_dir: &Path, item: &PlanItem, files: &[String]) -> Result<()> {
    let mut store = assets::load_store(config_dir, &item.scope);
    let a = store.assets.iter_mut().find(|a| a.id == item.asset_id).ok_or_else(|| Error::runtime("资产在出图期间被删掉了"))?;
    for f in files {
        let bytes = std::fs::read(f).map_err(|e| Error::runtime(format!("读不到出图结果 {f}：{e}")))?;
        let ext = Path::new(f).extension().and_then(|e| e.to_str()).unwrap_or("png").to_string();
        assets::add_variant_to_look(config_dir, &item.scope, a, item.look_id.as_deref(), &bytes, &ext, false, Some(item.prompt.clone()), false, None)?;
    }
    assets::save_store(config_dir, &item.scope, &store)
}

/// 登记并在后台开跑。按集登记（资产页和队列按集读任务），同一集同时只跑一批
pub fn start(app: Option<tauri::AppHandle>, config_dir: &Path, project_id: &str, episode_id: Option<&str>, redo: bool, only: &[String]) -> Result<(jobs::Job, Vec<PlanItem>)> {
    let items = plan(config_dir, project_id, episode_id, redo, only);
    if items.is_empty() {
        return Err(Error::config(if only.is_empty() {
            if redo || plan(config_dir, project_id, episode_id, true, &[]).is_empty() { "这个项目还没有资产，先建资产" } else { "资产都有图了 —— 要重出请选「全部重出」" }
        } else {
            "点名的资产没找到，或者都已经有图了（要重出请带上 redo）"
        }));
    }
    let title = format!("资产出图 · {}", describe(&items));
    let job_id = jobs::start_exclusive(config_dir, JobKind::AssetImages, project_id, episode_id.unwrap_or(""), "", 0, &title)
        .map_err(|_| Error::config("这个项目已经在批量出图了 —— 进度在右下角"))?;
    let targets: Vec<String> = items.iter().map(|i| i.asset_id.clone()).collect();
    jobs::set_targets(config_dir, &job_id, &targets);
    jobs::set_progress(config_dir, &job_id, 0, items.len(), &items[0].label);
    let started = jobs::get(config_dir, &job_id).ok_or_else(|| Error::runtime("任务登记后读不回来"))?;
    if let Some(app) = &app {
        jobs::emit(Some(app), Some(&started));
    }

    let config_dir = config_dir.to_path_buf();
    let project_id = project_id.to_string();
    let engine = project_engine(&config_dir, &project_id);
    let batch = items.clone();
    tauri::async_runtime::spawn(async move {
        let emit = |job: Option<jobs::Job>| {
            if let Some(app) = &app {
                jobs::emit(Some(app), job.as_ref());
            }
        };
        let mut failed = Vec::new();
        let mut attempted = 0;
        for (i, item) in batch.iter().enumerate() {
            if jobs::cancel_requested(&config_dir, &job_id) {
                break;
            }
            attempted += 1;
            emit(jobs::set_progress(&config_dir, &job_id, i, batch.len(), &item.label));
            match run_one(&config_dir, item, &engine).await {
                Ok(_) => {
                    if let Some(app) = &app {
                        crate::studio_ops::emit_changed(app, "assets", Some(&project_id));
                    }
                }
                Err(e) => failed.push(format!("{}：{e}", item.label)),
            }
        }
        let summary = finish_summary(batch.len(), attempted, &failed);
        let job = if !failed.is_empty() && failed.len() == attempted {
            jobs::finish_with(&config_dir, &job_id, None, Some(&summary), &failed, Some(&failed[0]))
        } else {
            jobs::finish_with(&config_dir, &job_id, None, Some(&summary), &failed, None)
        };
        emit(job);
    });
    Ok((started, items))
}

/// `attempted` < `total` 说明中途停了
pub fn finish_summary(total: usize, attempted: usize, failed: &[String]) -> String {
    let ok = attempted - failed.len();
    let head = if failed.is_empty() && attempted == total { format!("{ok} 张设定图都出好了") } else if failed.is_empty() { format!("出了 {ok} 张") } else { format!("出了 {ok} 张，{} 张没出来", failed.len()) };
    if attempted < total { format!("{head}（中途停止，还有 {} 张没轮到）", total - attempted) } else { head }
}

/// 资产页「一键全出 / 全部重出」
#[tauri::command]
pub fn assets_batch_generate_start(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
    project_id: String,
    episode_id: Option<String>,
    redo: bool,
    only: Option<Vec<String>>,
) -> Result<jobs::Job> {
    start(Some(app), &state.config_dir, &project_id, episode_id.as_deref(), redo, &only.unwrap_or_default()).map(|(j, _)| j)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, Scope, Scope) {
        let d = tempfile::tempdir().unwrap();
        let proj = crate::cinema::cinema_projects_dir(d.path()).join("p1");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(proj.join("project.json"), r#"{"model_settings":{"t2i_engine":"krea2"}}"#).unwrap();
        (d, Scope::Project { project_id: "p1".into() }, Scope::Episode { project_id: "p1".into(), episode_id: "ep_01".into() })
    }

    fn put(dir: &Path, scope: &Scope, list: Vec<Asset>) {
        let mut s = assets::load_store(dir, scope);
        s.assets = list;
        assets::save_store(dir, scope, &s).unwrap();
    }

    /// **标了「从不出镜」的角色不排出图。**
    ///
    /// 它没有能拍出来的身体（旁白员、电话那头的人、纯语音 AI）——
    /// 出来的图没有任何地方会用：分镜不给它镜头，提示词里它只按文字出现。
    ///
    /// 2026-09-19 用户在 E05 上撞到：小青、小橙标了从不出镜，
    /// 而「一键全出」会真的给它们排两次生成。
    #[test]
    fn 从不出镜的角色不排出图() {
        let (d, proj, _) = setup();
        let mut 小青 = Asset::new(AssetKind::Character, "小青");
        小青.character = Some(crate::assets::CharacterTraits { disembodied: true, ..Default::default() });
        let 陈先生 = Asset::new(AssetKind::Character, "陈先生");
        put(d.path(), &proj, vec![小青, 陈先生]);

        let labels: Vec<String> = plan(d.path(), "p1", None, false, &[]).into_iter().map(|i| i.label).collect();
        assert_eq!(labels, ["陈先生"], "给没有身体的角色排了出图");

        // **重出也不排** —— 它本来就不该有图
        let labels: Vec<String> = plan(d.path(), "p1", None, true, &[]).into_iter().map(|i| i.label).collect();
        assert_eq!(labels, ["陈先生"]);

        // **点名时例外**：用户明确点了它的名，那是他的决定
        let labels: Vec<String> = plan(d.path(), "p1", None, true, &["小青".into()]).into_iter().map(|i| i.label).collect();
        assert_eq!(labels, ["小青"], "用户点名了还不给出，那是越权");
    }

    #[test]
    fn 缺图的才出_本集在前_角色场景道具排序_形象单算() {
        let (d, proj, ep) = setup();
        let mut shen = Asset::new(AssetKind::Character, "林婉清");
        shen.identity = "二十岁女性".into();
        shen.description = "唐装".into();
        let look = assets::add_look(&mut shen, "现代", "白衬衫").unwrap();
        let prop = Asset::new(AssetKind::Prop, "手机");
        let mut done = Asset::new(AssetKind::Scene, "书房");
        assets::add_variant(d.path(), &proj, &mut done, b"x", "png", false, None, false, None).unwrap();
        put(d.path(), &proj, vec![prop, shen, done]);
        put(d.path(), &ep, vec![Asset::new(AssetKind::Scene, "长安街")]);

        let items = plan(d.path(), "p1", Some("ep_01"), false, &[]);
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, ["林婉清", "林婉清 · 现代", "长安街", "手机"]);
        assert_eq!(items[1].look_id.as_deref(), Some(look.id.as_str()));
        // 人物本身进提示词；形象用自己的服装
        assert!(items[0].prompt.starts_with("林婉清，二十岁女性唐装"), "{}", items[0].prompt);
        assert!(items[1].prompt.contains("白衬衫") && !items[1].prompt.contains("唐装"));
        // 引擎读项目：krea2 不撑中文四视图 → 简洁三视图
        assert_eq!(items[0].template, "simple");
        assert_eq!((items[2].mode, items[2].width, items[2].height, items[2].viewpoints.len()), ("scene_sheet", 1536, 1024, 4));
        assert_eq!(items[3].mode, "t2i");
        assert_eq!(describe(&items), "2 个角色形象、1 个场景、1 个道具");

        // 重出：有图的也算
        assert_eq!(plan(d.path(), "p1", Some("ep_01"), true, &[]).len(), 5);
        // 点名：资产名带出它的形象；「角色 · 形象」只要这一个
        assert_eq!(plan(d.path(), "p1", None, false, &["林婉清".into()]).len(), 2);
        let one = plan(d.path(), "p1", None, false, &["林婉清 · 现代".into()]);
        assert_eq!(one.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(), ["林婉清 · 现代"]);
        assert!(plan(d.path(), "p1", None, false, &["皇帝".into()]).is_empty());
    }

    #[test]
    fn 项目风格插进提示词_没设引擎用_zimage() {
        let (d, proj, _) = setup();
        std::fs::write(crate::cinema::cinema_projects_dir(d.path()).join("p1").join("project.json"), "{}").unwrap();
        crate::studio_ops::set_style(d.path(), "p1", None, Some("ink wash painting")).unwrap();
        put(d.path(), &proj, vec![Asset::new(AssetKind::Character, "皇帝")]);
        let items = plan(d.path(), "p1", None, false, &[]);
        assert_eq!(items[0].template, "quadzh");
        let at = items[0].prompt.find("ink wash painting").expect("风格要进提示词");
        assert!(at < items[0].prompt.find("\n\n").unwrap(), "风格必须在构图段之前");
    }

    #[test]
    fn 出好的图挂到对应形象上_封面换成新图() {
        let (d, proj, _) = setup();
        let mut shen = Asset::new(AssetKind::Character, "林婉清");
        let look = assets::add_look(&mut shen, "现代", "").unwrap();
        put(d.path(), &proj, vec![shen]);
        let items = plan(d.path(), "p1", None, false, &[]);
        let f = d.path().join("out.png");
        std::fs::write(&f, b"png").unwrap();
        add_files(d.path(), &items[1], &[f.to_string_lossy().into()]).unwrap();
        let a = &assets::load_store(d.path(), &proj).assets[0];
        assert!(a.unit.selected_image().is_none(), "主形象不该被挂上");
        let l = a.looks.iter().find(|l| l.id == look.id).unwrap();
        assert_eq!(l.unit.image_variants.len(), 1);
        assert!(l.unit.selected_image().unwrap().prompt_used.as_deref().unwrap().contains("林婉清"));
        // 形象出过了，再算缺图只剩主形象
        assert_eq!(plan(d.path(), "p1", None, false, &[]).len(), 1);
    }

    #[test]
    fn 收尾那句话() {
        assert_eq!(finish_summary(3, 3, &[]), "3 张设定图都出好了");
        assert_eq!(finish_summary(3, 3, &["a：x".into()]), "出了 2 张，1 张没出来");
        assert_eq!(finish_summary(5, 2, &["a：x".into()]), "出了 1 张，1 张没出来（中途停止，还有 3 张没轮到）");
    }
}
