//! 工作区（系列 → 分镜）里的导演世界：不用跑画布。
//!
//! 用户 2026-09-27：「工作区没有导演世界的入口吗？还得跑一趟画布，再点击项目，再创建节点，再进入导演世界？」
//!
//! 三件事，都挂在分镜详情里：
//!
//! 1. **3D 片场属于地点**（场景资产）：拆机间生成一次，第 1~3 场十六镜都用它
//!    （[`cinema_generate_scene_world`]，写进 `SceneContract::world`）。
//! 2. **场面属于镜头**：人偶、相机、运镜路径、最近一次输出的人偶稿存在 `StoryboardShot::director_scene`
//!    （[`cinema_set_director_scene`]，整集保存时以磁盘为准）。
//! 3. **人偶稿 → 角色首帧**（[`cinema_render_frame_from_director`]）：导演世界输出的是「片场 + 彩色人偶」，
//!    直接当首帧出片会出一段人偶戏。这一步按人偶的颜色把角色图换进去，结果挂进这一镜的首帧 / 尾帧。
//!    **这一步还没有同题对照验证过**（导演世界重做计划 P6 一直没跑），界面上标着「试验」。

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::State;

use crate::cinema::{KeyframeSlot, StoryboardShot};
use crate::error::{Error, Result};
use crate::scene_contract::SceneWorld;
use crate::AppState;

/// 资产在哪个作用域（全局 → 项目 → 本集，和出片找图同一个顺序）。
fn find_asset_scope(config_dir: &Path, project_id: &str, episode_id: &str, asset_id: &str) -> Option<crate::assets::Scope> {
    use crate::assets::Scope;
    [
        Scope::Global,
        Scope::Project { project_id: project_id.into() },
        Scope::Episode { project_id: project_id.into(), episode_id: episode_id.into() },
    ]
    .into_iter()
    .find(|scope| crate::assets::load_store(config_dir, scope).assets.iter().any(|a| a.id == asset_id))
}

/// 从场景资产当前选中的那张图（通常是 360 全景）生成这个地点的 3D 片场，存进场景资产。
///
/// 全景会先投影成 SHARP 吃得下的单视图（`world_sharp::generate` 里做，视场按 SHARP 的默认焦距算）。
/// 实测拆机间 12 秒、66 MB。
#[tauri::command]
pub async fn cinema_generate_scene_world(
    state: State<'_, AppState>,
    project_id: String,
    episode_id: String,
    asset_id: String,
) -> Result<SceneWorld> {
    generate_scene_world(&state.config_dir, &project_id, &episode_id, &asset_id).await
}

pub(crate) async fn generate_scene_world(config_dir: &Path, project_id: &str, episode_id: &str, asset_id: &str) -> Result<SceneWorld> {
    let scope = find_asset_scope(config_dir, project_id, episode_id, asset_id).ok_or_else(|| Error::config(format!("找不到场景资产「{asset_id}」")))?;
    let image = crate::cinema::asset_image_path(config_dir, project_id, episode_id, asset_id, "")
        .ok_or_else(|| Error::config("这个场景还没有图 —— 先在资产里出一张全景图"))?;
    let out = crate::world_sharp::generate(config_dir, crate::world_sharp::WorldGenerateArgs { image_path: image.to_string_lossy().to_string(), device: "mps".into() })
        .await
        .map_err(Error::runtime)?;
    let world = SceneWorld {
        path: out.path,
        format: out.format,
        convention: "opencv_camera".into(),
        source_image: image.to_string_lossy().to_string(),
        created_at: crate::jobs::now(),
        // 新生成的片场从 1 开始 —— 换了文件，旧的缩放不一定还对
        scale: None,
    };
    // 生成要十几秒，重读一遍再写，免得冲掉这期间别处的改动
    let mut store = crate::assets::load_store(config_dir, &scope);
    let a = store.assets.iter_mut().find(|a| a.id == asset_id).ok_or_else(|| Error::config("场景资产在生成期间被删了"))?;
    a.scene.get_or_insert_with(Default::default).world = Some(world.clone());
    crate::assets::save_store(config_dir, &scope, &store)?;
    Ok(world)
}

/// 合并写这一镜的导演世界场面（对象的顶层键逐个覆盖）。只改磁盘上这一镜的这一个字段。
#[tauri::command]
pub fn cinema_set_director_scene(
    state: State<'_, AppState>,
    project_id: String,
    episode_id: String,
    shot_id: String,
    patch: Value,
) -> Result<Value> {
    set_director_scene(&state.config_dir, &project_id, &episode_id, &shot_id, patch)
}

pub(crate) fn set_director_scene(config_dir: &Path, project_id: &str, episode_id: &str, shot_id: &str, patch: Value) -> Result<Value> {
    let Value::Object(patch) = patch else { return Err(Error::config("导演世界场面必须是一个对象")) };
    // 在分集锁里合并：后台出片正好回写这一集时，两边的改动都要留下
    crate::episode_lock::update(config_dir, project_id, episode_id, |ep| {
        let shot = ep.storyboard.iter_mut().find(|s| s.id == shot_id).ok_or_else(|| Error::config(format!("找不到分镜「{shot_id}」")))?;
        let mut merged = match shot.director_scene.take() {
            Some(Value::Object(m)) => m,
            _ => serde_json::Map::new(),
        };
        for (k, v) in patch {
            merged.insert(k, v);
        }
        let merged = Value::Object(merged);
        shot.director_scene = Some(merged.clone());
        Ok(merged)
    })
}

/// 人偶稿里的一个人偶（导演世界输出的 `frame_meta.actors`，蛇形命名原样收）。
#[derive(Debug, Clone, Deserialize)]
pub struct DraftActor {
    #[serde(default)]
    pub asset_id: Option<String>,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub color_name: Option<String>,
    #[serde(default)]
    pub marker_color: String,
    #[serde(default)]
    pub pose_label: Option<String>,
    #[serde(default)]
    pub facing: Option<String>,
    #[serde(default)]
    pub screen: Option<ScreenPos>,
    #[serde(default = "yes")]
    pub in_frame: bool,
    /// person / prop / extra（导演台界面升级 U3.5）。老数据没有 = person
    #[serde(default)]
    pub role: Option<String>,
    /// 道具立牌用的那张图。只在道具资产查不到图时当退路
    #[serde(default)]
    pub image_path: Option<String>,
}

impl DraftActor {
    fn role(&self) -> &str {
        self.role.as_deref().unwrap_or("person")
    }
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct ScreenPos {
    pub x: f64,
    #[allow(dead_code)]
    pub y: f64,
}

/// 参考生图一次最多 10 张（Qwen 2.1），第 1 张是人偶稿。
const MAX_PEOPLE: usize = 9;

/// 人偶稿 → 角色首帧 / 尾帧（试验）。结果裁成人偶稿的尺寸，挂进这一镜的关键帧。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn cinema_render_frame_from_director(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    project_id: String,
    episode_id: String,
    shot_id: String,
    which: KeyframeSlot,
    draft_path: String,
    actors: Vec<DraftActor>,
) -> Result<StoryboardShot> {
    let num = crate::cinema::read_episode(&state.config_dir, &project_id, &episode_id)
        .ok()
        .and_then(|ep| ep.storyboard.iter().find(|s| s.id == shot_id).map(|s| s.shot_num))
        .unwrap_or(0);
    let label = if which == KeyframeSlot::First { "人偶稿出首帧" } else { "人偶稿出尾帧" };
    let job = crate::jobs::start(&state.config_dir, crate::jobs::JobKind::Frame, &project_id, &episode_id, &shot_id, num, label);
    crate::jobs::emit(Some(&app), crate::jobs::get(&state.config_dir, &job).as_ref());
    let out = render_frame_from_director(&state.config_dir, &project_id, &episode_id, &shot_id, which, Path::new(&draft_path), &actors).await;
    let done = match &out {
        Ok(s) => crate::jobs::finish(&state.config_dir, &job, s.image_url.as_deref(), None),
        Err(e) => crate::jobs::finish(&state.config_dir, &job, None, Some(&e.to_string())),
    };
    crate::jobs::emit(Some(&app), done.as_ref());
    out
}

/// 出图参数：人偶稿第 1 张，人按颜色排在后面；提示词逐个人偶写清楚「哪个颜色换成第几张图里的谁」。
pub(crate) fn draft_frame_args(config_dir: &Path, project_id: &str, episode_id: &str, shot: &StoryboardShot, draft: &Path, actors: &[DraftActor]) -> Result<Value> {
    if !draft.is_file() {
        return Err(Error::config("人偶稿文件不在了 —— 在导演世界里重新输出一次"));
    }
    let assets = crate::looks::for_shot(&crate::storyboard_gen::load_assets(config_dir, project_id, episode_id), shot);
    let mut images: Vec<String> = vec![draft.to_string_lossy().to_string()];
    let mut legend = vec![
        "Image 1 is the staging draft of this exact film frame. Its location, lighting, camera position, lens and composition are final. Every coloured mannequin in it is only a placeholder for a person.".to_string(),
    ];
    let mut swaps = Vec::new();
    let mut missing = Vec::new();
    // 挂上图的人 / 物件是第几张 —— 下面「这一镜画的是什么」里的 `<Subject N>` 换成名字时用
    let mut pictured: Vec<(String, usize)> = Vec::new();
    let in_frame: Vec<&DraftActor> = actors.iter().filter(|a| a.in_frame).collect();
    let place_of = |a: &DraftActor| a.screen.map(|p| if p.x < 1.0 / 3.0 { "left third of the frame" } else if p.x < 2.0 / 3.0 { "middle of the frame" } else { "right third of the frame" });
    // 人先占参考图槽（道具查不到图 / 槽满了还有立牌上那张图兜底，人没有）
    for a in in_frame.iter().filter(|a| a.role() == "person") {
        let colour = a.color_name.clone().filter(|c| !c.trim().is_empty()).unwrap_or_else(|| a.marker_color.clone());
        let how = [place_of(a).map(str::to_string), a.pose_label.clone(), a.facing.clone().map(|f| format!("facing {f}"))]
            .into_iter()
            .flatten()
            .filter(|s| !s.trim().is_empty())
            .collect::<Vec<_>>()
            .join(", ");
        let how = if how.is_empty() { String::new() } else { format!(" ({how})") };
        let hit = a.asset_id.as_deref().and_then(|id| assets.iter().find(|x| x.id == id)).and_then(|x| {
            crate::cinema::asset_image_path(config_dir, project_id, episode_id, &x.id, &x.look_id).map(|p| (x.name.clone(), p))
        });
        match hit {
            Some((name, path)) if images.len() <= MAX_PEOPLE => {
                images.push(path.to_string_lossy().to_string());
                let n = images.len();
                pictured.push((name.clone(), n));
                legend.push(format!("Image {n} is the person「{name}」: use this exact face, hair, body and costume; ignore the pose, framing and background of that image."));
                swaps.push(format!("Replace the {colour} mannequin{how} with the person from Image {n}「{name}」, keeping the mannequin's exact position, size, pose and facing."));
            }
            _ if a.asset_id.is_some() => missing.push(a.label.clone()),
            _ => swaps.push(format!("Replace the {colour} mannequin{how} with an ordinary person who fits the scene, keeping the mannequin's exact position, size, pose and facing.")),
        }
    }
    // 群演：一句话打包，别一个个点名 —— 九个灰人偶写九行，模型会把他们当主角
    let extras = in_frame.iter().filter(|a| a.role() == "extra").count();
    if extras > 0 {
        legend.push("The grey mannequins in Image 1 are placeholders for background extras.".into());
        swaps.push(format!(
            "Replace every grey mannequin ({extras} in total) with an ordinary background person who fits the scene — each one a different person in different clothes — keeping each mannequin's exact position, size, pose and facing. They are background extras, not the focus of the shot."
        ));
    }
    let props: Vec<&&DraftActor> = in_frame.iter().filter(|a| a.role() == "prop").collect();
    if !props.is_empty() {
        legend.push("Picture standees and boxes outlined in amber in Image 1 are placeholders for objects, not people.".into());
    }
    for a in props {
        let at = place_of(a).map(|p| format!(" in the {p}")).unwrap_or_default();
        let asset = a.asset_id.as_deref().and_then(|id| assets.iter().find(|x| x.id == id));
        let name = asset.map(|x| x.name.clone()).unwrap_or_else(|| a.label.clone());
        let path = asset
            .and_then(|x| crate::cinema::asset_image_path(config_dir, project_id, episode_id, &x.id, &x.look_id))
            .or_else(|| a.image_path.as_deref().map(PathBuf::from).filter(|p| p.is_file()));
        match path {
            Some(path) if images.len() <= MAX_PEOPLE => {
                images.push(path.to_string_lossy().to_string());
                let n = images.len();
                pictured.push((name.clone(), n));
                legend.push(format!("Image {n} is the object「{name}」: use this exact object's shape, materials and colours; ignore the background of that image."));
                swaps.push(format!("Replace the standee of「{name}」{at} with the real object from Image {n}, at the same position and size, sitting naturally in the scene with correct perspective, contact shadow and lighting."));
            }
            _ => swaps.push(format!("Replace the amber placeholder「{name}」{at} with a real「{name}」at the same position and size, sitting naturally in the scene with correct perspective, contact shadow and lighting.")),
        }
    }
    if !missing.is_empty() {
        return Err(Error::config(format!("这几个人偶绑的角色没有图（或参考图槽满了）：{} —— 先给角色出图", missing.join("、"))));
    }
    // 和出首帧同一套选法（见 `cinema::first_frame_source`）：按首帧写的、过期的不用，没合成过按镜头表现拼；
    // 线路 B 的 `<Subject N>` 换成名字（出图模型不认）
    let shot_desc = match crate::cinema::read_project(config_dir, project_id) {
        Ok(project) => crate::cinema::first_frame_body(config_dir, &project, episode_id, shot, &pictured),
        Err(_) => String::new(),
    };
    let mut prompt = format!(
        "Reference images:\n{}\n\n{}\nRemove every mannequin and placeholder: no mannequin, coloured placeholder, standee, outline, panel or text may remain. Change nothing else in Image 1.",
        legend.join("\n"),
        swaps.join("\n"),
    );
    if !shot_desc.trim().is_empty() {
        prompt.push_str(&format!("\n\nWhat this shot shows: {}", shot_desc.trim()));
    }
    let (w, h) = image::image_dimensions(draft).map_err(|e| Error::runtime(format!("人偶稿读不出来：{e}")))?;
    Ok(json!({
        "engine": "qwen21_edit",
        "prompt": prompt,
        "width": w, "height": h,
        "reference_images": images,
        "output_filename": format!("director_frame_{}.png", uuid::Uuid::new_v4()),
    }))
}

pub(crate) async fn render_frame_from_director(
    config_dir: &Path,
    project_id: &str,
    episode_id: &str,
    shot_id: &str,
    which: KeyframeSlot,
    draft: &Path,
    actors: &[DraftActor],
) -> Result<StoryboardShot> {
    use crate::agent_runtime::{RuntimeContext, RuntimePolicy, ToolCallRequest, ToolHandler, ToolStatus};
    let ep = crate::cinema::read_episode(config_dir, project_id, episode_id)?;
    let shot = ep.storyboard.iter().find(|s| s.id == shot_id).ok_or_else(|| Error::config(format!("找不到分镜「{shot_id}」")))?;
    let args = draft_frame_args(config_dir, project_id, episode_id, shot, draft, actors)?;
    let want = (args["width"].as_u64().unwrap_or(0) as u32, args["height"].as_u64().unwrap_or(0) as u32);
    let ctx = RuntimeContext::new(config_dir.to_path_buf(), RuntimePolicy::workspace_write(config_dir));
    let out = crate::agent_runtime::image_tool::GenerateImageTool.execute(&ctx, ToolCallRequest::new("generate_image", args)).await;
    if out.status != ToolStatus::Ok {
        return Err(Error::runtime(out.failure.map(|f| f.message).unwrap_or(out.content)));
    }
    let path = out.artifacts.first().ok_or_else(|| Error::runtime("出图没有返回文件"))?;
    let img = image::open(path).map_err(|e| Error::runtime(format!("出来的图解不开：{e}")))?;
    // 参考生图的画幅跟第一张参考图（人偶稿）走，这里再对齐一次，防止节点对齐 32 带来的几个像素差
    let img = crate::cinema::fit_to_frame(img, want);
    let shots_dir: PathBuf = crate::cinema::episode_file(config_dir, project_id, episode_id).with_file_name("shots");
    std::fs::create_dir_all(&shots_dir).map_err(|e| Error::runtime(e.to_string()))?;
    let temp = shots_dir.join(format!("{}.png", uuid::Uuid::new_v4()));
    img.save_with_format(&temp, image::ImageFormat::Png).map_err(|e| Error::runtime(e.to_string()))?;
    let res = crate::cinema::set_shot_keyframe(config_dir, project_id, episode_id, shot_id, which, Some(&crate::assets::MediaInput::Path { path: temp.to_string_lossy().to_string() }));
    let _ = std::fs::remove_file(&temp);
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actor(asset: Option<&str>, colour: &str, x: f64) -> DraftActor {
        DraftActor {
            asset_id: asset.map(str::to_string),
            label: asset.unwrap_or("匿名人物 1").into(),
            color_name: Some(colour.into()),
            marker_color: "#E53935".into(),
            pose_label: Some("坐".into()),
            facing: Some("镜头".into()),
            screen: Some(ScreenPos { x, y: 0.5 }),
            in_frame: true,
            role: None,
            image_path: None,
        }
    }

    /// 场面合并写：只覆盖给出的顶层键，其余保留；不是对象的拒掉。
    #[test]
    fn director_scene_patch_merges_top_level_keys() {
        let dir = tempfile::tempdir().unwrap();
        let ep_dir = crate::cinema::cinema_projects_dir(dir.path()).join("p1/episodes/e1");
        std::fs::create_dir_all(&ep_dir).unwrap();
        let shot = json!({"id":"s1","shot_num":1,"scale":"","camera_movement":"","dialogue":"","prompt":"","duration":5.0,"status":"pending"});
        std::fs::write(ep_dir.join("episode.json"), json!({"id":"e1","episode_num":1,"title":"t","synopsis":"","script_text":"","storyboard_version":crate::cinema::STORYBOARD_VERSION,"storyboard":[shot]}).to_string()).unwrap();
        set_director_scene(dir.path(), "p1", "e1", "s1", json!({"scene": {"actors": []}, "outputs": {"combined": "a.png"}})).unwrap();
        let v = set_director_scene(dir.path(), "p1", "e1", "s1", json!({"scene": {"actors": [1]}})).unwrap();
        assert_eq!(v["outputs"]["combined"], "a.png", "没给的键被冲掉了");
        assert_eq!(v["scene"]["actors"][0], 1);
        assert!(set_director_scene(dir.path(), "p1", "e1", "s1", json!([1])).is_err());
    }

    /// 人偶稿第 1 张；每个绑了角色的人偶写清「哪个颜色、在哪、换成第几张图的谁」；没图的角色拒掉。
    #[test]
    fn draft_frame_prompt_maps_each_colour_to_a_picture() {
        use crate::assets::{Asset, AssetKind, MediaVariant, Scope};
        let dir = tempfile::tempdir().unwrap();
        let scope = Scope::Global;
        let media = scope.media_dir(dir.path());
        std::fs::create_dir_all(&media).unwrap();
        let mut store = crate::assets::load_store(dir.path(), &scope);
        let mut a = Asset::new(AssetKind::Character, "陆川");
        std::fs::write(media.join("jc.png"), b"x").unwrap();
        a.unit.image_variants.push(MediaVariant { id: "v".into(), rel_path: "jc.png".into(), created_at: 0, prompt_used: None, favorited: false, from_upload: false, role: None });
        a.unit.selected_image_id = Some("v".into());
        let jc = a.id.clone();
        store.assets.push(a);
        store.assets.push(Asset::new(AssetKind::Character, "没图的人"));
        let noimg = store.assets[1].id.clone();
        crate::assets::save_store(dir.path(), &scope, &store).unwrap();
        let draft = dir.path().join("draft.png");
        image::RgbImage::new(320, 180).save(&draft).unwrap();
        let shot: StoryboardShot = serde_json::from_value(json!({"id":"s1","shot_num":1,"scale":"","camera_movement":"","dialogue":"","prompt":"","duration":5.0,"status":"pending"})).unwrap();

        let args = draft_frame_args(dir.path(), "p1", "e1", &shot, &draft, &[actor(Some(&jc), "红", 0.2), actor(None, "蓝", 0.8)]).unwrap();
        assert_eq!(args["engine"], "qwen21_edit");
        let imgs: Vec<String> = serde_json::from_value(args["reference_images"].clone()).unwrap();
        assert!(imgs[0].ends_with("draft.png") && imgs[1].ends_with("jc.png") && imgs.len() == 2, "{imgs:?}");
        assert_eq!((args["width"].as_u64(), args["height"].as_u64()), (Some(320), Some(180)));
        let p = args["prompt"].as_str().unwrap();
        assert!(p.contains("Replace the 红 mannequin (left third of the frame, 坐, facing 镜头) with the person from Image 2「陆川」"), "{p}");
        assert!(p.contains("Replace the 蓝 mannequin (right third of the frame, 坐, facing 镜头) with an ordinary person"), "{p}");
        assert!(p.contains("Remove every mannequin"), "{p}");

        let err = draft_frame_args(dir.path(), "p1", "e1", &shot, &draft, &[actor(Some(&noimg), "绿", 0.5)]).unwrap_err();
        assert!(err.to_string().contains("没有图"), "{err}");
    }

    /// 群演打包成一句（不逐个点名，不报「没图」）；道具用它资产的图当参考，查不到图退回立牌上那张，再没有就按名字换。
    #[test]
    fn draft_frame_prompt_handles_props_and_extras() {
        use crate::assets::{Asset, AssetKind, MediaVariant, Scope};
        let dir = tempfile::tempdir().unwrap();
        let scope = Scope::Global;
        let media = scope.media_dir(dir.path());
        std::fs::create_dir_all(&media).unwrap();
        let mut store = crate::assets::load_store(dir.path(), &scope);
        let mut a = Asset::new(AssetKind::Prop, "机械臂");
        std::fs::write(media.join("arm.png"), b"x").unwrap();
        a.unit.image_variants.push(MediaVariant { id: "v".into(), rel_path: "arm.png".into(), created_at: 0, prompt_used: None, favorited: false, from_upload: false, role: None });
        a.unit.selected_image_id = Some("v".into());
        let arm = a.id.clone();
        store.assets.push(a);
        crate::assets::save_store(dir.path(), &scope, &store).unwrap();
        let draft = dir.path().join("draft.png");
        image::RgbImage::new(320, 180).save(&draft).unwrap();
        let loose = dir.path().join("loose.png");
        std::fs::write(&loose, b"x").unwrap();
        let shot: StoryboardShot = serde_json::from_value(json!({"id":"s1","shot_num":1,"scale":"","camera_movement":"","dialogue":"","prompt":"","duration":5.0,"status":"pending"})).unwrap();

        let role = |mut a: DraftActor, r: &str| {
            a.role = Some(r.into());
            a
        };
        let mut extra = role(actor(None, "灰", 0.5), "extra");
        extra.label = "群演 1".into();
        let mut prop = role(actor(Some(&arm), "", 0.8), "prop");
        prop.label = "机械臂".into();
        let mut loose_prop = role(actor(Some("gone"), "", 0.1), "prop");
        loose_prop.label = "旧箱子".into();
        loose_prop.image_path = Some(loose.to_string_lossy().to_string());
        let mut bare_prop = role(actor(None, "", 0.5), "prop");
        bare_prop.label = "椅子".into();
        let actors = [extra.clone(), role(actor(None, "灰", 0.2), "extra"), prop, loose_prop, bare_prop];

        let args = draft_frame_args(dir.path(), "p1", "e1", &shot, &draft, &actors).unwrap();
        let imgs: Vec<String> = serde_json::from_value(args["reference_images"].clone()).unwrap();
        assert!(imgs.len() == 3 && imgs[1].ends_with("arm.png") && imgs[2].ends_with("loose.png"), "{imgs:?}");
        let p = args["prompt"].as_str().unwrap();
        assert!(p.contains("Replace every grey mannequin (2 in total) with an ordinary background person"), "{p}");
        assert!(!p.contains("群演 1"), "群演不该逐个点名：{p}");
        assert!(p.contains("Image 2 is the object「机械臂」") && p.contains("Replace the standee of「机械臂」 in the right third of the frame with the real object from Image 2"), "{p}");
        assert!(p.contains("Image 3 is the object「旧箱子」"), "{p}");
        assert!(p.contains("Replace the amber placeholder「椅子」 in the middle of the frame with a real「椅子」"), "{p}");
        assert!(!p.contains("mannequin (middle of the frame, 坐, facing 镜头) with an ordinary person who fits"), "道具被当成人了：{p}");
    }

    /// 界面整份保存场景契约时不带 3D 片场，也不能把它冲掉；只挂着校准 / 片场的契约不当空契约丢掉。
    #[test]
    fn scene_world_survives_contract_save() {
        let mut c = crate::scene_contract::SceneContract::default();
        assert!(!c.has_world_data());
        c.world = Some(SceneWorld { path: "/w.ply".into(), format: "ply".into(), ..Default::default() });
        assert!(c.is_empty() && c.has_world_data(), "没锚点但有片场：不能当空契约丢");
        let src = include_str!("assets.rs");
        assert!(src.contains("v.world = a.scene.as_ref().and_then(|s| s.world.clone());"));
        assert!(src.contains("v.is_empty() && !v.has_world_data()"));
    }
}
