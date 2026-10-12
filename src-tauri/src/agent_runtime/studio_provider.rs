//! Anima Chat 调用 Anima Studio 的工具（`studio_*`）。
//!
//! 方案：`docs/plans/2026-09-15-Chat接Studio方案.md`。第 1 批（P0）：读、建项目、导剧本、建资产、设风格；
//! 第 2 批（P1）：出资产图、生成分镜、合成提示词、出片、合成一集 —— **只发起后台任务、立刻回任务 id**，
//! 和 Studio 里点按钮是同一个函数，进度在右下角。
//!
//! # 几条规矩
//!
//! - **调领域函数**（`studio_ops`、`assets`、`cinema` 里的非命令函数），不调 Tauri 命令
//! - **项目按名字或 id 都认**；名字重名时报错列出候选，不猜
//! - **返回精简**：本机模型窗口 33K，回摘要 + id，不回整份 JSON
//! - 写完发 `studio://changed`，Studio 页面不切页就刷新
//! - 命名统一 `studio_`，和旧流程的 `cinema_*` 工具分开
//!
//! 画布工作流（`studio_workflow_*`）：方案 `docs/plans/2026-09-23-Chat写画布工作流方案.md`。
//! 只存模板、不发起生成；保存前过 `canvas_workflow_lint`，会静默出错的一律拒绝。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{RuntimeContext, ToolCallRequest, ToolFailure, ToolFailureKind, ToolHandler, ToolKind, ToolOutcome, ToolRegistry, ToolSpec};
use crate::assets::{AssetKind, Scope};
use crate::cinema::cinema_projects_dir;
use crate::mcp::Operation;

/// 注册全部 Studio 工具
pub fn register_studio_tools(registry: &mut ToolRegistry, config_dir: impl Into<PathBuf>, app: Option<tauri::AppHandle>, gateway: Option<Arc<crate::llm::gateway::Gateway>>) {
    let deps = Deps { config_dir: config_dir.into(), app, gateway };
    for op in ALL_OPS {
        registry.register(StudioTool { op: *op, deps: deps.clone() });
    }
}

#[derive(Clone)]
struct Deps {
    config_dir: PathBuf,
    app: Option<tauri::AppHandle>,
    gateway: Option<Arc<crate::llm::gateway::Gateway>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    ListProjects,
    GetProject,
    CreateProject,
    CreateEpisode,
    ImportScript,
    SaveScriptDoc,
    ExtractEntities,
    CreateAssets,
    AddLook,
    ListAssets,
    ListStyles,
    SetStyle,
    JobStatus,
    RenderQueue,
    EditQueue,
    GenerateAssetImages,
    GenerateShotTable,
    SynthesizePrompts,
    RenderEpisode,
    StitchEpisode,
    WorkflowList,
    WorkflowRead,
    WorkflowCheck,
    WorkflowSave,
}

const ALL_OPS: &[Op] = &[
    Op::ListProjects, Op::GetProject, Op::CreateProject, Op::CreateEpisode, Op::ImportScript, Op::SaveScriptDoc,
    Op::ExtractEntities, Op::CreateAssets, Op::AddLook, Op::ListAssets, Op::ListStyles, Op::SetStyle, Op::JobStatus, Op::RenderQueue, Op::EditQueue,
    Op::GenerateAssetImages, Op::GenerateShotTable, Op::SynthesizePrompts, Op::RenderEpisode, Op::StitchEpisode,
    Op::WorkflowList, Op::WorkflowRead, Op::WorkflowCheck, Op::WorkflowSave,
];

struct StudioTool {
    op: Op,
    deps: Deps,
}

const PROJECT_PARAM: &str = "项目名字或 id（studio_list_projects 里的）";
const EPISODE_PARAM: &str = "分集：id（ep_01）、集数（1）或标题；不写 = 第 1 集";
const SHOTS_PARAM: &str = "镜号列表，比如 [3, 5]；不写 = 整集";

fn spec_of(op: Op) -> ToolSpec {
    let (name, desc, operation, params, timeout) = match op {
        Op::ListProjects => ("studio_list_projects", "列出 Anima Studio 里的系列和独立项目：名字、id、集数、镜头数、状态。", Operation::Read, json!({"type":"object","properties":{}}), 10),
        Op::GetProject => ("studio_get_project", "看一个项目的概况：分集、剧本字数、资产数（缺图的）、风格、分镜进度，以及**流程走到哪一步、下一步做什么**。建东西前先调它，不要凭记忆。", Operation::Read,
            json!({"type":"object","properties":{"project":{"type":"string","description":PROJECT_PARAM}},"required":["project"]}), 10),
        Op::CreateProject => ("studio_create_project", "新建系列（多集）或独立项目（单集），自带空的第 1 集。同名项目 10 分钟内重复建会直接返回已有的。", Operation::Write,
            json!({"type":"object","properties":{
                "title":{"type":"string","description":"项目名，比如《我在长安写诗词》"},
                "kind":{"type":"string","enum":["series","standalone"],"description":"series = 系列（多集），standalone = 独立项目"},
                "genre":{"type":"string","description":"题材，比如 古风穿越"},
                "aspect_ratio":{"type":"string","enum":["16:9","9:16"],"description":"画幅，不写 9:16"},
                "description":{"type":"string","description":"一两句简介"}},
                "required":["title","kind"]}), 15),
        Op::CreateEpisode => ("studio_create_episode", "给系列加一集（空的）。", Operation::Write,
            json!({"type":"object","properties":{"project":{"type":"string","description":PROJECT_PARAM},"title":{"type":"string"},"synopsis":{"type":"string","description":"这一集梗概"}},"required":["project"]}), 15),
        Op::ImportScript => ("studio_import_script", "把剧本写进某一集，并检查格式（场标题、「角色：台词」行）。剧本格式：每场开头一行「【场景：地点 · 时间】」，台词一人一行「角色名：台词」。这一集已有剧本或分镜时要 replace=true 才覆盖。", Operation::Write,
            json!({"type":"object","properties":{
                "project":{"type":"string","description":PROJECT_PARAM},
                "episode":{"type":"string","description":EPISODE_PARAM},
                "text":{"type":"string","description":"完整剧本正文"},
                "replace":{"type":"boolean","description":"已有剧本 / 分镜时是否覆盖，默认 false"}},
                "required":["project","text"]}), 15),
        Op::SaveScriptDoc => ("studio_save_script_doc", "把剧本存进「剧本编辑器」的剧本库（不绑项目）。还没决定要不要建项目时用。", Operation::Write,
            json!({"type":"object","properties":{"title":{"type":"string"},"text":{"type":"string"}},"required":["title","text"]}), 15),
        Op::ExtractEntities => ("studio_extract_entities", "读某一集剧本，认出角色 / 场景 / 道具和描述（后台模型，几十秒），**不落盘**。结果给用户看过、点头后再用 studio_create_assets 建。", Operation::Write,
            json!({"type":"object","properties":{"project":{"type":"string","description":PROJECT_PARAM},"episode":{"type":"string","description":EPISODE_PARAM}},"required":["project"]}), 300),
        Op::CreateAssets => ("studio_create_assets", "批量建资产（角色 / 场景 / 道具），默认建在这一集。同名同类已存在的跳过。角色的脸、身材、年龄写 identity（人物本身），服装发型写 description。", Operation::Write,
            json!({"type":"object","properties":{
                "project":{"type":"string","description":PROJECT_PARAM},
                "episode":{"type":"string","description":EPISODE_PARAM},
                "place":{"type":"string","enum":["episode","project","global"],"description":"建在哪：episode = 这一集（默认），project = 整个系列共用，global = 全局资产库"},
                "assets":{"type":"array","items":{"type":"object","properties":{
                    "kind":{"type":"string","enum":["character","scene","prop"]},
                    "name":{"type":"string"},
                    "description":{"type":"string","description":"场景 / 道具的外观；角色的服装发型"},
                    "identity":{"type":"string","description":"只给角色：性别、年龄、身高体型、脸型五官、肤色"}},
                    "required":["kind","name"]}}},
                "required":["project","assets"]}), 15),
        Op::AddLook => ("studio_add_look", "给角色加一个形象（同一个人的不同发型、服装、造型）。", Operation::Write,
            json!({"type":"object","properties":{
                "project":{"type":"string","description":PROJECT_PARAM},
                "character":{"type":"string","description":"角色名字或 id"},
                "name":{"type":"string","description":"形象名，比如 盛唐男装"},
                "description":{"type":"string","description":"这个形象的发型、服装、妆容、配饰"}},
                "required":["project","character","name"]}), 15),
        Op::ListAssets => ("studio_list_assets", "列一个项目的资产（这一集 + 项目 + 全局库）：名字、种类、形象、有没有图。", Operation::Read,
            json!({"type":"object","properties":{"project":{"type":"string","description":PROJECT_PARAM},"episode":{"type":"string","description":EPISODE_PARAM},"kind":{"type":"string","enum":["character","scene","prop"]}},"required":["project"]}), 10),
        Op::ListStyles => ("studio_list_styles", "列出内置画面风格预设（id、名字、适合什么题材）。", Operation::Read, json!({"type":"object","properties":{}}), 10),
        Op::SetStyle => ("studio_set_style", "给项目设画面风格：选一个预设 id，或写自定义风格词（英文逗号分隔效果最好）。", Operation::Write,
            json!({"type":"object","properties":{"project":{"type":"string","description":PROJECT_PARAM},"preset_id":{"type":"string"},"custom_prompt":{"type":"string"}},"required":["project"]}), 10),
        Op::JobStatus => ("studio_job_status", "查后台任务（生成分镜、合成提示词、出片、合成等）：给任务 id 查一个，或给项目查它正在跑和最近的任务。", Operation::Read,
            json!({"type":"object","properties":{"job_id":{"type":"string"},"project":{"type":"string","description":PROJECT_PARAM}}}), 10),
        Op::RenderQueue => ("studio_render_queue", "看这一集正在跑的批量出片的排队名单：已出几个、正在出哪一镜、没轮到的按「必须连着出」分好组（要接上一镜的和上一镜在同一组，不能拆开）。暂停位显示成 hold:…。", Operation::Read,
            json!({"type":"object","properties":{"project":{"type":"string","description":PROJECT_PARAM},"episode":{"type":"string","description":EPISODE_PARAM}},"required":["project"]}), 10),
        Op::EditQueue => ("studio_edit_queue", "改这一集正在跑的批量出片的排队名单（只动没轮到的）。action=reorder：order 给没轮到的全部新顺序（镜号数字，暂停位写它的 hold:… 原名）；action=hold：插一个暂停位，批量走到它就停下等、不交 ComfyUI 任何东西（after=插在第几镜后面，不写 = 正在出的那一镜之后），label 写干什么用；action=release：撤掉暂停位 hold（正在等的也能撤，批量接着出）。要接上一镜的不能和上一镜拆开，拆了会拒绝。", Operation::Write,
            json!({"type":"object","properties":{
                "project":{"type":"string","description":PROJECT_PARAM},
                "episode":{"type":"string","description":EPISODE_PARAM},
                "action":{"type":"string","enum":["reorder","hold","release"]},
                "order":{"type":"array","items":{},"description":"reorder：没轮到的全部新顺序，镜号或 hold:… 原名"},
                "after":{"type":"integer","description":"hold：插在第几镜后面"},
                "label":{"type":"string","description":"hold：这个暂停位干什么用，比如 冒烟测试"},
                "hold":{"type":"string","description":"release：要撤的暂停位原名（hold:…）"}},
                "required":["project","action"]}), 10),
        Op::GenerateAssetImages => ("studio_generate_asset_images", "给资产出设定图（后台任务，立刻回任务 id）：默认出这一集和项目里还没图的资产和角色形象；和 Studio 资产页「一键全出」同一套提示词、版式、风格。每张 1~2 分钟，串行。", Operation::Write,
            json!({"type":"object","properties":{
                "project":{"type":"string","description":PROJECT_PARAM},
                "episode":{"type":"string","description":EPISODE_PARAM},
                "assets":{"type":"array","items":{"type":"string"},"description":"只出这几个：资产名、形象名或「角色 · 形象」；不写 = 所有没图的"},
                "redo":{"type":"boolean","description":"有图的也重出一版（旧图留着），默认 false"}},
                "required":["project"]}), 30),
        Op::GenerateShotTable => ("studio_generate_shot_table", "按剧本生成这一集的分镜（后台模型，几分钟，后台任务）。这一集已有分镜时要 replace=true 才覆盖。资产最好先出好图。", Operation::Write,
            json!({"type":"object","properties":{
                "project":{"type":"string","description":PROJECT_PARAM},
                "episode":{"type":"string","description":EPISODE_PARAM},
                "replace":{"type":"boolean","description":"已有分镜时是否覆盖，默认 false"}},
                "required":["project"]}), 30),
        Op::SynthesizePrompts => ("studio_synthesize_prompts", "按项目当前线路合成出片提示词（后台任务）：整集或指定镜头。线路 B 每镜几十秒。", Operation::Write,
            json!({"type":"object","properties":{
                "project":{"type":"string","description":PROJECT_PARAM},
                "episode":{"type":"string","description":EPISODE_PARAM},
                "shots":{"type":"array","items":{"type":"integer"},"description":SHOTS_PARAM}},
                "required":["project"]}), 30),
        Op::RenderEpisode => ("studio_render_episode", "批量出片（后台任务，按项目当前线路和出片设置）：默认出合成好了、还没出过片的镜头；也可以指定镜号。每镜 15~40 分钟，整集要几个小时。", Operation::Write,
            json!({"type":"object","properties":{
                "project":{"type":"string","description":PROJECT_PARAM},
                "episode":{"type":"string","description":EPISODE_PARAM},
                "shots":{"type":"array","items":{"type":"integer"},"description":"镜号列表；不写 = 还没出过片的（都出过了就全部重出）"}},
                "required":["project"]}), 30),
        Op::StitchEpisode => ("studio_stitch_episode", "把出好的镜头合成一集成片（后台任务，几分钟）。upscale=true 逐镜超分到 1080P（每镜多一两分钟）。", Operation::Write,
            json!({"type":"object","properties":{
                "project":{"type":"string","description":PROJECT_PARAM},
                "episode":{"type":"string","description":EPISODE_PARAM},
                "upscale":{"type":"boolean","description":"超分到 1080P，默认 false"}},
                "required":["project"]}), 30),
        Op::WorkflowList => ("studio_workflow_list", "列出画布工作流模板：名字、属于哪个项目、节点数、保存时间。给 project 只列那个项目的。", Operation::Read,
            json!({"type":"object","properties":{"project":{"type":"string","description":PROJECT_PARAM}}}), 10),
        Op::WorkflowRead => ("studio_workflow_read", "读一份画布工作流模板，回完整 JSON 和 revision。**改已有模板前必须先读**，保存时把 revision 带回去。", Operation::Read,
            json!({"type":"object","properties":{"project":{"type":"string","description":PROJECT_PARAM},"name":{"type":"string","description":"工作流名字（studio_workflow_list 里的）"}},"required":["project","name"]}), 10),
        Op::WorkflowCheck => ("studio_workflow_check", "只检查、不保存：连线、引擎名、本机路径、提示词里的尺寸，以及 H3 视频提示词的格式（三段字段、声音层引号、人声栏、时间段对齐、台词说不说得完）。回 errors（会拒绝保存）和 warnings。", Operation::Read,
            json!({"type":"object","properties":{"workflow":{"type":"object","description":"工作流：{nodes: [...], edges: [...]}"}},"required":["workflow"]}), 10),
        Op::WorkflowSave => ("studio_workflow_save", "把一份工作流存成画布模板（不发起任何生成）。先过和 studio_workflow_check 一样的检查，有 errors 就不存。同名已存在时必须带 studio_workflow_read 给的 revision，对不上说明用户在界面上改过，要重新读。", Operation::Write,
            json!({"type":"object","properties":{
                "project":{"type":"string","description":PROJECT_PARAM},
                "name":{"type":"string","description":"工作流名字，比如「上课不要玩手机 · EP09 xxx」"},
                "workflow":{"type":"object","description":"工作流：{nodes: [{id, type, position:{x,y}, data:{...}}], edges: [{id, source, target}]}。节点说明写 data.guide（字符串数组）"},
                "revision":{"type":"string","description":"覆盖已有模板时必填：studio_workflow_read 回的 revision"}},
                "required":["project","name","workflow"]}), 15),
    };
    ToolSpec::new(name, desc, ToolKind::Cinema, operation)
        .with_parameters(params)
        .with_idempotent(operation == Operation::Read)
        .with_timeout(Duration::from_secs(timeout))
}

#[async_trait]
impl ToolHandler for StudioTool {
    fn spec(&self) -> ToolSpec {
        spec_of(self.op)
    }

    async fn execute(&self, _ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let started = Instant::now();
        let out = run(self.op, &self.deps, &request.arguments).await;
        match out {
            Ok(v) => ToolOutcome::ok(&request, serde_json::to_string_pretty(&v).unwrap_or_else(|_| "{}".into()), Vec::new(), started.elapsed()),
            Err(ToolErr::Params(m)) => ToolOutcome::failed(&request, ToolFailure::new(ToolFailureKind::InvalidParams, m), started.elapsed()),
            Err(ToolErr::Failed(m)) => ToolOutcome::failed(&request, ToolFailure::new(ToolFailureKind::Execution, m), started.elapsed()),
        }
    }
}

#[derive(Debug)]
enum ToolErr {
    /// 参数不对：模型改参数能好
    Params(String),
    /// 执行失败：原因原样给用户
    Failed(String),
}

impl From<crate::error::Error> for ToolErr {
    fn from(e: crate::error::Error) -> Self {
        ToolErr::Failed(e.to_string())
    }
}

type R<T> = std::result::Result<T, ToolErr>;

fn s<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str()).map(str::trim).filter(|v| !v.is_empty())
}

fn need<'a>(args: &'a Value, key: &str) -> R<&'a str> {
    s(args, key).ok_or_else(|| ToolErr::Params(format!("缺少参数 {key}")))
}

async fn run(op: Op, d: &Deps, args: &Value) -> R<Value> {
    let cfg = d.config_dir.as_path();
    match op {
        Op::ListProjects => Ok(list_projects(cfg)),
        Op::GetProject => {
            let p = resolve_project(cfg, need(args, "project")?)?;
            Ok(project_overview(cfg, &p))
        }
        Op::CreateProject => {
            let title = need(args, "title")?;
            let kind = s(args, "kind").unwrap_or("standalone");
            if let Some(existing) = recent_same_title(cfg, title) {
                return Ok(json!({"project_id": existing.id, "title": existing.title, "episode_id": "ep_01", "summary": format!("项目《{}》刚建过（10 分钟内），没有重复建", existing.title), "studio_link": {"project_id": existing.id}}));
            }
            let req: crate::cinema::CreateProjectReq = serde_json::from_value(json!({
                "title": title, "kind": kind, "genre": s(args, "genre").unwrap_or(""),
                "aspect_ratio": s(args, "aspect_ratio"), "description": s(args, "description").unwrap_or("")
            })).map_err(|e| ToolErr::Params(e.to_string()))?;
            let p = crate::studio_ops::create_project(cfg, req)?;
            emit(d, "project", Some(&p.id));
            Ok(json!({"project_id": p.id, "title": p.title, "kind": p.kind, "episode_id": "ep_01",
                "summary": format!("建好了{}《{}》（{}），带空的第 1 集。下一步：用 studio_import_script 导入剧本", if p.kind == "series" { "系列" } else { "独立项目" }, p.title, p.aspect_ratio),
                "studio_link": {"project_id": p.id}}))
        }
        Op::CreateEpisode => {
            let p = resolve_project(cfg, need(args, "project")?)?;
            let ep = crate::studio_ops::create_episode(cfg, &p.id, s(args, "title").unwrap_or(""), s(args, "synopsis").unwrap_or(""))?;
            emit(d, "episode", Some(&p.id));
            Ok(json!({"project_id": p.id, "episode_id": ep.id, "title": ep.title, "summary": format!("《{}》加了{}", p.title, ep.title), "studio_link": {"project_id": p.id, "episode_id": ep.id}}))
        }
        Op::ImportScript => {
            let p = resolve_project(cfg, need(args, "project")?)?;
            let ep = resolve_episode(cfg, &p.id, s(args, "episode"))?;
            let text = args.get("text").and_then(|v| v.as_str()).filter(|t| !t.trim().is_empty()).ok_or_else(|| ToolErr::Params("缺少参数 text（剧本正文）".into()))?;
            let replace = args.get("replace").and_then(|v| v.as_bool()).unwrap_or(false);
            let r = crate::studio_ops::import_script(cfg, &p.id, &ep, text, replace).map_err(|e| ToolErr::Params(e.to_string()))?;
            emit(d, "episode", Some(&p.id));
            Ok(json!({"project_id": p.id, "episode_id": ep, "chars": r.chars, "scenes": r.scenes.len(), "scene_titles": head(&r.scenes, 12), "speakers": head(&r.speakers, 20), "problems": r.problems,
                "summary": if r.problems.is_empty() { format!("剧本写进了《{}》{}：{} 字、{} 场、说话人 {}。下一步：studio_extract_entities 认出角色场景道具", p.title, ep, r.chars, r.scenes.len(), r.speakers.join("、")) } else { format!("剧本写进去了，但格式有问题：{}", r.problems.join("；")) },
                "studio_link": {"project_id": p.id, "episode_id": ep}}))
        }
        Op::SaveScriptDoc => {
            let title = need(args, "title")?;
            let text = args.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let doc = crate::script_library::create_screenplay_doc(cfg, title, text).map_err(ToolErr::Failed)?;
            emit(d, "script_doc", None);
            Ok(json!({"doc_id": doc.id, "title": doc.title, "summary": format!("剧本《{}》存进了剧本库（{} 字）", doc.title, text.chars().count())}))
        }
        Op::ExtractEntities => {
            let p = resolve_project(cfg, need(args, "project")?)?;
            let ep = resolve_episode(cfg, &p.id, s(args, "episode"))?;
            let gw = d.gateway.as_ref().ok_or_else(|| ToolErr::Failed("后台模型没接上，认不了".into()))?;
            let list = crate::cinema::extract_entities(cfg, gw, &p.id, &ep).await?;
            let items: Vec<Value> = list.iter().map(|e| json!({"kind": e.kind, "name": e.name, "description": clip(&e.description, 120), "already_exists": e.exists})).collect();
            Ok(json!({"project_id": p.id, "episode_id": ep, "entities": items,
                "summary": format!("认出 {} 个（已存在 {} 个）。先给用户看，用户点头后用 studio_create_assets 建（description 可以照这里的，角色另写 identity）", list.len(), list.iter().filter(|e| e.exists).count())}))
        }
        Op::CreateAssets => {
            let p = resolve_project(cfg, need(args, "project")?)?;
            let ep = resolve_episode(cfg, &p.id, s(args, "episode"))?;
            let scope = match s(args, "place").unwrap_or("episode") {
                "project" => Scope::Project { project_id: p.id.clone() },
                "global" => Scope::Global,
                "episode" => Scope::Episode { project_id: p.id.clone(), episode_id: ep.clone() },
                other => return Err(ToolErr::Params(format!("place 只能是 episode / project / global，不是「{other}」"))),
            };
            let items: Vec<crate::studio_ops::NewAsset> = serde_json::from_value(args.get("assets").cloned().unwrap_or(Value::Null))
                .map_err(|e| ToolErr::Params(format!("assets 写得不对：{e}（每项要 kind 和 name）")))?;
            if items.is_empty() {
                return Err(ToolErr::Params("assets 是空的".into()));
            }
            let r = crate::studio_ops::create_assets(cfg, &scope, &items).map_err(|e| ToolErr::Params(e.to_string()))?;
            emit(d, "assets", Some(&p.id));
            let names = |v: &[crate::studio_ops::CreatedAsset]| v.iter().map(|a| a.name.clone()).collect::<Vec<_>>().join("、");
            Ok(json!({"created": r.created, "skipped": r.skipped,
                "summary": format!("建了 {} 个：{}{}。资产还没有图：下一步 studio_generate_asset_images 出图", r.created.len(), names(&r.created), if r.skipped.is_empty() { String::new() } else { format!("；已存在跳过：{}", names(&r.skipped)) }),
                "studio_link": {"project_id": p.id, "episode_id": ep, "page": "assets"}}))
        }
        Op::AddLook => {
            let p = resolve_project(cfg, need(args, "project")?)?;
            let who = need(args, "character")?;
            let name = need(args, "name")?;
            let (scope, asset) = find_character(cfg, &p.id, who)?;
            let mut store = crate::assets::load_store(cfg, &scope);
            let a = store.assets.iter_mut().find(|a| a.id == asset.id).ok_or_else(|| ToolErr::Failed("角色不见了".into()))?;
            let look = crate::assets::add_look(a, name, s(args, "description").unwrap_or("")).map_err(|e| ToolErr::Params(e.to_string()))?;
            let warn = a.identity.trim().is_empty();
            crate::assets::save_store(cfg, &scope, &store)?;
            emit(d, "assets", Some(&p.id));
            Ok(json!({"character_id": asset.id, "look_id": look.id, "summary": format!("「{}」加了形象「{}」{}", asset.name, look.name, if warn { "。这个角色还没写人物本身（脸、身材、年龄），不写的话每个形象出图脸会不一样" } else { "" })}))
        }
        Op::ListAssets => {
            let p = resolve_project(cfg, need(args, "project")?)?;
            let ep = resolve_episode(cfg, &p.id, s(args, "episode")).ok();
            let kind = s(args, "kind").map(crate::studio_ops::parse_kind).transpose().map_err(|e| ToolErr::Params(e.to_string()))?;
            Ok(list_assets(cfg, &p.id, ep.as_deref(), kind))
        }
        Op::ListStyles => Ok(json!({"styles": crate::style::all_presets().iter().map(|p| json!({"id": p.id, "name": p.name_zh, "for": p.best_for})).collect::<Vec<_>>()})),
        Op::SetStyle => {
            let p = resolve_project(cfg, need(args, "project")?)?;
            if s(args, "preset_id").is_none() && s(args, "custom_prompt").is_none() {
                return Err(ToolErr::Params("preset_id 和 custom_prompt 至少给一个".into()));
            }
            let art = crate::studio_ops::set_style(cfg, &p.id, s(args, "preset_id"), s(args, "custom_prompt")).map_err(|e| ToolErr::Params(e.to_string()))?;
            emit(d, "style", Some(&p.id));
            let name = crate::style::find_preset(&art.preset_id).map(|x| x.name_zh.clone()).unwrap_or_else(|| "自定义".into());
            Ok(json!({"project_id": p.id, "preset_id": art.preset_id, "summary": format!("《{}》的风格设成了「{}」", p.title, name)}))
        }
        Op::JobStatus => Ok(job_status(cfg, s(args, "job_id"), s(args, "project").map(|n| resolve_project(cfg, n)).transpose()?.as_ref())),
        Op::RenderQueue | Op::EditQueue => {
            let p = resolve_project(cfg, need(args, "project")?)?;
            let ep = resolve_episode(cfg, &p.id, s(args, "episode"))?;
            let job = crate::render_queue::running_batch(cfg, &p.id, &ep).ok_or_else(|| ToolErr::Params("这一集现在没有批量出片在跑".into()))?;
            let qv = crate::render_queue::view(cfg, &job);
            if op == Op::RenderQueue {
                return Ok(queue_json(&qv));
            }
            // 镜号 → 没轮到的名单里的 id
            let pending: Vec<&crate::render_queue::QueueItem> = qv.pending.iter().flatten().collect();
            let by_num = |n: u64| pending.iter().find(|i| i.shot_num.map(u64::from) == Some(n)).map(|i| i.id.clone());
            let job = match need(args, "action")? {
                "reorder" => {
                    let order = args.get("order").and_then(|v| v.as_array()).ok_or_else(|| ToolErr::Params("reorder 要给 order".into()))?;
                    let mut ids = Vec::new();
                    for x in order {
                        let id = match (x.as_u64(), x.as_str()) {
                            (Some(n), _) => by_num(n).ok_or_else(|| ToolErr::Params(format!("第 {n} 镜不在没轮到的名单里")))?,
                            (_, Some(h)) if crate::render_queue::is_hold(h) => h.to_string(),
                            (_, Some(t)) => t.trim_start_matches('第').trim_end_matches('镜').trim().parse::<u64>().ok().and_then(by_num).ok_or_else(|| ToolErr::Params(format!("认不出 {t}")))?,
                            _ => return Err(ToolErr::Params(format!("认不出 {x}"))),
                        };
                        ids.push(id);
                    }
                    if ids.len() != pending.len() {
                        return Err(ToolErr::Params(format!("没轮到的有 {} 项，order 给了 {} 项 —— 要给全（不想出的用界面停掉）", pending.len(), ids.len())));
                    }
                    crate::render_queue::reorder(cfg, &job.id, &ids).map_err(ToolErr::Params)?
                }
                "hold" => {
                    let after = match args.get("after").and_then(|v| v.as_u64()) {
                        Some(n) => Some(by_num(n).ok_or_else(|| ToolErr::Params(format!("第 {n} 镜不在没轮到的名单里")))?),
                        None => None,
                    };
                    let label = s(args, "label").unwrap_or("外部任务");
                    crate::render_queue::insert_hold(cfg, &job.id, label, after.as_deref()).map_err(ToolErr::Params)?.0
                }
                "release" => crate::render_queue::release_hold(cfg, &job.id, need(args, "hold")?).map_err(ToolErr::Params)?,
                other => return Err(ToolErr::Params(format!("action 只认 reorder / hold / release，不认 {other}"))),
            };
            Ok(queue_json(&crate::render_queue::view(cfg, &job)))
        }
        Op::GenerateAssetImages => {
            let p = resolve_project(cfg, need(args, "project")?)?;
            let ep = resolve_episode(cfg, &p.id, s(args, "episode"))?;
            let only: Vec<String> = args.get("assets").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
            let redo = args.get("redo").and_then(|v| v.as_bool()).unwrap_or(false);
            let (job, items) = crate::asset_batch::start(d.app.clone(), cfg, &p.id, Some(&ep), redo, &only).map_err(|e| ToolErr::Params(e.to_string()))?;
            let names: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
            Ok(started(&job, &p, &ep, "assets", format!("开始出 {} 张设定图（{}）：{}。每张 1~2 分钟，大约 {}~{} 分钟", items.len(), crate::asset_batch::describe(&items), clip(&names.join("、"), 200), items.len(), items.len() * 2)))
        }
        Op::GenerateShotTable => {
            let p = resolve_project(cfg, need(args, "project")?)?;
            let ep = resolve_episode(cfg, &p.id, s(args, "episode"))?;
            let v = read_episode(cfg, &p.id, &ep)?;
            if v["script_text"].as_str().is_none_or(|t| t.trim().is_empty()) {
                return Err(ToolErr::Params("这一集还没有剧本，先用 studio_import_script 导入".into()));
            }
            let existing = v["storyboard"].as_array().map_or(0, |a| a.len());
            if existing > 0 && !args.get("replace").and_then(|v| v.as_bool()).unwrap_or(false) {
                return Err(ToolErr::Params(format!("这一集已经有 {existing} 镜分镜，重新生成会覆盖（连同合成好的提示词）。用户确认后带 replace=true")));
            }
            let (app, gw) = (d.app.clone(), gateway_of(d)?);
            let job = crate::cinema::start_shot_table(app, cfg.to_path_buf(), gw, p.id.clone(), ep.clone())?;
            Ok(started(&job, &p, &ep, "storyboard", "开始生成分镜，按场来，几分钟。好了之后用 studio_get_project 看有没有问题，再合成提示词".into()))
        }
        Op::SynthesizePrompts => {
            let p = resolve_project(cfg, need(args, "project")?)?;
            let ep = resolve_episode(cfg, &p.id, s(args, "episode"))?;
            let ids = shot_ids_by_num(cfg, &p.id, &ep, args.get("shots"))?;
            if ids.is_empty() && read_episode(cfg, &p.id, &ep)?["storyboard"].as_array().is_none_or(|a| a.is_empty()) {
                return Err(ToolErr::Params("这一集还没有分镜，先 studio_generate_shot_table".into()));
            }
            let (app, gw) = (d.app.clone(), gateway_of(d)?);
            let n = ids.len();
            let job = crate::cinema::start_synthesize(app, cfg.to_path_buf(), gw, p.id.clone(), ep.clone(), ids, None)?;
            Ok(started(&job, &p, &ep, "storyboard", format!("开始合成提示词（{}，{}）", if n == 0 { "整集".to_string() } else { format!("{n} 镜") }, job.prompt)))
        }
        Op::RenderEpisode => {
            let p = resolve_project(cfg, need(args, "project")?)?;
            let ep = resolve_episode(cfg, &p.id, s(args, "episode"))?;
            let picked = shot_ids_by_num(cfg, &p.id, &ep, args.get("shots"))?;
            let (ids, redo, blocked) = if picked.is_empty() { crate::cinema::batch_render_targets(cfg, &p.id, &ep)? } else { (picked, false, 0) };
            if ids.is_empty() {
                return Err(ToolErr::Params(format!("没有能出片的镜头：{blocked} 镜还没合成提示词或合成后改过，先 studio_synthesize_prompts")));
            }
            let app = d.app.clone();
            let n = ids.len();
            let job = crate::cinema::start_render_batch(app, cfg.to_path_buf(), p.id.clone(), ep.clone(), ids)?;
            let mut msg = format!("开始出片 {n} 镜{}，每镜 15~40 分钟，全部大约 {}~{} 小时。某一镜失败会接着出下一镜", if redo { "（都出过了，这次是全部重出）" } else { "" }, (n * 15).div_ceil(60), (n * 40).div_ceil(60));
            if blocked > 0 {
                msg.push_str(&format!("；另有 {blocked} 镜没合成提示词或合成后改过，这次不出"));
            }
            if let Some(m) = crate::llm_memory::big_local_chat_model(cfg) {
                msg.push_str(&format!("。提醒用户：Chat 现在用的是本机大模型 {m}，出片期间会和出片抢内存，建议先把 Chat 换成小模型或 Ollama Cloud"));
            }
            Ok(started(&job, &p, &ep, "storyboard", msg))
        }
        Op::StitchEpisode => {
            let p = resolve_project(cfg, need(args, "project")?)?;
            let ep = resolve_episode(cfg, &p.id, s(args, "episode"))?;
            let rendered = read_episode(cfg, &p.id, &ep)?["storyboard"].as_array().map_or(0, |a| a.iter().filter(|s| s["video_url"].as_str().is_some_and(|u| !u.is_empty())).count());
            if rendered == 0 {
                return Err(ToolErr::Params("这一集还没有出好片的镜头，先 studio_render_episode".into()));
            }
            let upscale = args.get("upscale").and_then(|v| v.as_bool()).unwrap_or(false);
            let app = d.app.clone();
            let job = crate::cinema::start_stitch(app, cfg.to_path_buf(), p.id.clone(), ep.clone(), upscale, false)?;
            Ok(started(&job, &p, &ep, "assembly", format!("开始把 {rendered} 镜合成一集{}", if upscale { "，逐镜超分到 1080P（每镜一两分钟）" } else { "，几分钟" })))
        }
        Op::WorkflowList => {
            let only = s(args, "project").map(|k| resolve_project(cfg, k)).transpose()?;
            let titles: std::collections::HashMap<String, String> = all_projects(cfg).into_iter().map(|p| (p.id, p.title)).collect();
            let items: Vec<Value> = crate::canvas_workflows::list(cfg)
                .into_iter()
                .filter(|w| only.as_ref().map_or(true, |p| p.id == w.project_id))
                .take(50)
                .map(|w| {
                    let nodes = crate::canvas_workflows::load(cfg, &w.project_id, &w.name).ok()
                        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                        .and_then(|v| v["nodes"].as_array().map(Vec::len)).unwrap_or(0);
                    json!({"name": w.name, "project_id": w.project_id, "project": titles.get(&w.project_id).cloned().unwrap_or_default(), "nodes": nodes, "saved_at": w.saved_at})
                })
                .collect();
            Ok(json!({"count": items.len(), "workflows": items}))
        }
        Op::WorkflowRead => {
            let p = resolve_project(cfg, need(args, "project")?)?;
            let name = need(args, "name")?;
            let text = crate::canvas_workflows::load(cfg, &p.id, name).map_err(|e| ToolErr::Params(e.to_string()))?;
            let v: Value = serde_json::from_str(&text).map_err(|e| ToolErr::Failed(format!("这份工作流不是合法 JSON：{e}")))?;
            Ok(json!({"project_id": p.id, "name": name, "revision": workflow_revision(&text), "workflow": v}))
        }
        Op::WorkflowCheck => {
            let mut v = workflow_arg(args)?;
            let stripped = crate::canvas_workflow_lint::normalize(&mut v, s(args, "name").unwrap_or("未命名"), 0);
            let l = crate::canvas_workflow_lint::lint(&v, &crate::canvas_workflow_lint::known_engines());
            Ok(json!({"ok": l.errors.is_empty(), "errors": l.errors, "warnings": l.warnings, "stripped": stripped}))
        }
        Op::WorkflowSave => {
            let p = resolve_project(cfg, need(args, "project")?)?;
            let name = need(args, "name")?;
            let existing = crate::canvas_workflows::load(cfg, &p.id, name).ok();
            if let Some(cur) = &existing {
                match s(args, "revision") {
                    None => return Err(ToolErr::Params(format!("「{name}」已经有了。要覆盖就先 studio_workflow_read 读一遍、把 revision 带回来；想另存就换个名字"))),
                    Some(r) if r != workflow_revision(cur) => return Err(ToolErr::Params(format!("「{name}」在你读过之后被改过（多半是用户在画布上存的），revision 对不上。重新 studio_workflow_read，在新的版本上改"))),
                    Some(_) => {}
                }
            }
            let mut v = workflow_arg(args)?;
            let stripped = crate::canvas_workflow_lint::normalize(&mut v, name, now_ms());
            let l = crate::canvas_workflow_lint::lint(&v, &crate::canvas_workflow_lint::known_engines());
            if !l.errors.is_empty() {
                return Err(ToolErr::Params(format!("没存 —— 有 {} 处会静默出错：\n- {}", l.errors.len(), l.errors.join("\n- "))));
            }
            let text = serde_json::to_string_pretty(&v).map_err(|e| ToolErr::Failed(e.to_string()))?;
            crate::canvas_workflows::save(cfg, &p.id, name, &text)?;
            emit(d, "workflow", Some(&p.id));
            let nodes = v["nodes"].as_array().map_or(0, Vec::len);
            Ok(json!({"project_id": p.id, "name": name, "revision": workflow_revision(&text), "nodes": nodes, "warnings": l.warnings, "stripped": stripped,
                "summary": format!("{}工作流「{name}」（{nodes} 个节点）。用户在画布的工作流列表里载入它；这一步没有出图出片{}",
                    if existing.is_some() { "覆盖了" } else { "存好了" },
                    if l.warnings.is_empty() { String::new() } else { format!("。有 {} 条提醒要告诉用户", l.warnings.len()) })}))
        }
    }
}

/// 工具参数里的工作流：对象，或者一段 JSON 字符串（有的模型会把大对象序列化成字符串传）。
fn workflow_arg(args: &Value) -> R<Value> {
    let v = match args.get("workflow") {
        Some(Value::String(t)) => serde_json::from_str(t).map_err(|e| ToolErr::Params(format!("workflow 不是合法 JSON：{e}")))?,
        Some(v @ Value::Object(_)) => v.clone(),
        _ => return Err(ToolErr::Params("缺少参数 workflow（{nodes, edges}）".into())),
    };
    Ok(v)
}

/// 文件内容的指纹。覆盖时拿它判断「读过之后有没有人改过」。
fn workflow_revision(text: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(text.as_bytes()).iter().take(8).map(|b| format!("{b:02x}")).collect()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// 后台任务发起后的统一返回
fn started(job: &crate::jobs::Job, p: &ProjectRef, ep: &str, page: &str, summary: String) -> Value {
    json!({"job_id": job.id, "project_id": p.id, "episode_id": ep,
        "summary": format!("{summary}。进度在右下角，也可以用 studio_job_status 查（job_id {}）", job.id),
        "studio_link": {"project_id": p.id, "episode_id": ep, "page": page}})
}

/// **只有后台模型是必需的，界面不是。**
///
/// 2026-09-18 之前这里要求两样都有，于是 `anima-mcp`（无头，没有 `AppHandle`）
/// 调这几个工具必定失败。但 `app` 从头到尾只用来**发一个界面进度事件** ——
/// 没有它任务照样登记、照样跑完，只是右下角不动。
///
/// 真正拦着无头的是 `jobs.json` 的跨进程写冲突，那个已经用 flock 解决了
/// （见 `jobs.rs`）。所以这里放宽成「界面可选」。
fn gateway_of(d: &Deps) -> R<Arc<crate::llm::gateway::Gateway>> {
    d.gateway.clone().ok_or_else(|| ToolErr::Failed("后台模型没接上，发起不了".into()))
}

fn read_episode(cfg: &Path, project_id: &str, episode_id: &str) -> R<Value> {
    let f = cinema_projects_dir(cfg).join(project_id).join("episodes").join(episode_id).join("episode.json");
    serde_json::from_str(&std::fs::read_to_string(&f).map_err(|e| ToolErr::Failed(format!("读不到分集：{e}")))?).map_err(|e| ToolErr::Failed(format!("分集文件坏了：{e}")))
}

/// 镜号 → 镜头 id。不写回空（= 整集）；有对不上的镜号报出来
/// 排队名单给模型看：分组用镜号写，暂停位写原名
fn queue_json(q: &crate::render_queue::QueueView) -> Value {
    let name = |i: &crate::render_queue::QueueItem| i.shot_num.map(|n| json!(n)).unwrap_or_else(|| json!(i.id));
    json!({
        "job_id": q.job_id,
        "done": q.done,
        "current": q.current.as_ref().map(|i| i.label.clone()),
        "pending_groups": q.pending.iter().map(|g| g.iter().map(name).collect::<Vec<_>>()).collect::<Vec<_>>(),
        "summary": format!("已出 {} 个；正在出：{}；没轮到的 {} 组（同一组的必须连着出）",
            q.done, q.current.as_ref().map_or("（还没开始）".into(), |i| i.label.clone()), q.pending.len()),
    })
}

fn shot_ids_by_num(cfg: &Path, project_id: &str, episode_id: &str, shots: Option<&Value>) -> R<Vec<String>> {
    let Some(list) = shots.and_then(|v| v.as_array()).filter(|a| !a.is_empty()) else { return Ok(Vec::new()) };
    let v = read_episode(cfg, project_id, episode_id)?;
    let board = v["storyboard"].as_array().cloned().unwrap_or_default();
    let mut ids = Vec::new();
    let mut missing = Vec::new();
    for n in list {
        let num = n.as_u64().or_else(|| n.as_str().and_then(|t| t.trim_start_matches('第').trim_end_matches('镜').trim().parse().ok()));
        match num.and_then(|num| board.iter().find(|s| s["shot_num"].as_u64() == Some(num))) {
            Some(s) => ids.push(s["id"].as_str().unwrap_or_default().to_string()),
            None => missing.push(n.to_string()),
        }
    }
    if !missing.is_empty() {
        return Err(ToolErr::Params(format!("这一集没有第 {} 镜（共 {} 镜）", missing.join("、"), board.len())));
    }
    Ok(ids)
}

fn emit(d: &Deps, kind: &'static str, project_id: Option<&str>) {
    if let Some(app) = &d.app {
        crate::studio_ops::emit_changed(app, kind, project_id);
    }
}

fn clip(s: &str, n: usize) -> String {
    let t: String = s.chars().take(n).collect();
    if s.chars().count() > n { format!("{t}…") } else { t }
}

fn head<T: Clone>(v: &[T], n: usize) -> Vec<T> {
    v.iter().take(n).cloned().collect()
}

// ── 审批弹窗上的一句话 ────────────────────────────────────────────────

/// Studio 写工具的审批摘要：只看参数、不读盘（弹窗要马上出来）。不是 Studio 工具回 `None`
pub fn approval_summary(tool_name: &str, args: &Value) -> Option<String> {
    let g = |k: &str| s(args, k).unwrap_or("");
    let proj = || format!("「{}」", g("project").trim_matches(|c| c == '《' || c == '》'));
    let ep = || s(args, "episode").map(|e| format!("的{e}")).unwrap_or_else(|| "第 1 集".into());
    Some(match tool_name {
        "studio_create_project" => {
            let extra: Vec<&str> = [g("aspect_ratio"), g("genre")].into_iter().filter(|x| !x.is_empty()).collect();
            format!("新建{}「{}」{}，自带空的第 1 集",
                if g("kind") == "series" { "系列" } else { "独立项目" },
                g("title").trim_matches(|c| c == '《' || c == '》'),
                if extra.is_empty() { String::new() } else { format!("（{}）", extra.join("，")) })
        }
        "studio_create_episode" => format!("给{}加一集{}", proj(), s(args, "title").map(|t| format!("「{t}」")).unwrap_or_default()),
        "studio_import_script" => {
            let n = args.get("text").and_then(|v| v.as_str()).map(|t| t.chars().count()).unwrap_or(0);
            let replace = args.get("replace").and_then(|v| v.as_bool()).unwrap_or(false);
            format!("把 {n} 字的剧本写进{}{}{}", proj(), if s(args, "episode").is_some() { ep() } else { "第 1 集".into() }, if replace { "，**覆盖原有剧本；已有分镜会对不上**" } else { "" })
        }
        "studio_save_script_doc" => format!("把剧本《{}》存进剧本库", g("title")),
        "studio_edit_queue" => match g("action") {
            "hold" => format!("在{}正在跑的出片队列里插一个暂停位「{}」{}（批量走到它就停下等）", proj(), g("label"), args.get("after").and_then(|v| v.as_u64()).map(|n| format!("，放在第 {n} 镜后面")).unwrap_or_else(|| "，放在正在出的那一镜后面".into())),
            "release" => format!("撤掉{}出片队列里的暂停位 {}，批量接着出", proj(), g("hold")),
            _ => format!("调整{}正在跑的出片队列里没轮到的镜头顺序", proj()),
        },
        "studio_extract_entities" => format!("读{}{}的剧本，让后台模型认出角色、场景、道具（只列出来，不建；几十秒）", proj(), if s(args, "episode").is_some() { ep() } else { "第 1 集".into() }),
        "studio_create_assets" => {
            let items = args.get("assets").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let group = |k: &str, label: &str| {
                let names: Vec<String> = items.iter().filter(|i| i["kind"] == k).filter_map(|i| i["name"].as_str().map(String::from)).collect();
                (!names.is_empty()).then(|| format!("{} 个{label}（{}）", names.len(), names.join("、")))
            };
            let parts: Vec<String> = [group("character", "角色"), group("scene", "场景"), group("prop", "道具")].into_iter().flatten().collect();
            let place = match g("place") { "project" => "整个项目共用", "global" => "全局资产库", _ => "这一集" };
            format!("在{}新建 {}，放在{place}；同名已存在的会跳过", proj(), if parts.is_empty() { "资产".into() } else { parts.join("、") })
        }
        "studio_add_look" => format!("给{}里的「{}」加形象「{}」", proj(), g("character"), g("name")),
        "studio_generate_asset_images" => {
            let only: Vec<&str> = args.get("assets").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
            let what = if only.is_empty() { if args.get("redo").and_then(|v| v.as_bool()).unwrap_or(false) { "所有资产全部重出一版".to_string() } else { "还没图的资产和角色形象".to_string() } } else { only.join("、") };
            format!("给{}{}出设定图：{what}。每张 1~2 分钟，串行跑，期间 ComfyUI 被占用", proj(), if s(args, "episode").is_some() { ep() } else { "第 1 集".into() })
        }
        "studio_generate_shot_table" => format!("按{}{}的剧本生成分镜（后台模型，几分钟）{}", proj(), if s(args, "episode").is_some() { ep() } else { "第 1 集".into() },
            if args.get("replace").and_then(|v| v.as_bool()).unwrap_or(false) { "，**覆盖现有分镜和合成好的提示词**" } else { "" }),
        "studio_synthesize_prompts" => format!("给{}{}合成出片提示词：{}（线路 B 每镜几十秒）", proj(), if s(args, "episode").is_some() { ep() } else { "第 1 集".into() }, shots_label(args)),
        "studio_render_episode" => format!("给{}{}出片：{}。每镜 15~40 分钟，整集可能要好几个小时。出片前会卸掉后台模型（Chat 的模型保留）；Chat 如果用的是本机大模型，出片期间建议换成小模型或 Ollama Cloud", proj(), if s(args, "episode").is_some() { ep() } else { "第 1 集".into() },
            if args.get("shots").and_then(|v| v.as_array()).is_some_and(|a| !a.is_empty()) { shots_label(args) } else { "合成好了、还没出过片的镜头".into() }),
        "studio_stitch_episode" => format!("把{}{}出好的镜头合成一集{}", proj(), if s(args, "episode").is_some() { ep() } else { "第 1 集".into() },
            if args.get("upscale").and_then(|v| v.as_bool()).unwrap_or(false) { "，逐镜超分到 1080P（每镜多一两分钟）" } else { "（几分钟）" }),
        "studio_workflow_save" => {
            let n = args.get("workflow").and_then(|w| w.get("nodes")).and_then(|n| n.as_array()).map_or(0, Vec::len);
            format!("把画布工作流「{}」（{n} 个节点）存进{}的模板列表{}；只存模板，不出图不出片", g("name"), proj(),
                if s(args, "revision").is_some() { "，**覆盖同名的那一份**" } else { "" })
        }
        "studio_set_style" => format!("把{}的画面风格设成{}", proj(), s(args, "preset_id").map(|p| format!("预设「{}」", crate::style::find_preset(p).map(|x| x.name_zh.clone()).unwrap_or_else(|| p.to_string()))).unwrap_or_else(|| format!("自定义「{}」", clip(g("custom_prompt"), 40)))),
        _ => return None,
    })
}

/// 只能「允许一次」的工具：发起的是几十分钟到几小时的显卡任务，模型重试一次就是白跑一轮
pub fn once_only(tool_name: &str) -> bool {
    matches!(tool_name, "studio_generate_asset_images" | "studio_generate_shot_table" | "studio_synthesize_prompts" | "studio_render_episode" | "studio_stitch_episode")
}

fn shots_label(args: &Value) -> String {
    match args.get("shots").and_then(|v| v.as_array()).filter(|a| !a.is_empty()) {
        Some(a) => format!("第 {} 镜", a.iter().map(|n| n.to_string().trim_matches('"').to_string()).collect::<Vec<_>>().join("、")),
        None => "整集".into(),
    }
}

// ── 找项目、分集、角色 ────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct ProjectRef {
    id: String,
    title: String,
    kind: String,
    created_at: u64,
}

fn all_projects(cfg: &Path) -> Vec<ProjectRef> {
    let mut v: Vec<ProjectRef> = std::fs::read_dir(cinema_projects_dir(cfg))
        .map(|r| {
            r.flatten()
                .filter_map(|e| {
                    let p: Value = serde_json::from_str(&std::fs::read_to_string(e.path().join("project.json")).ok()?).ok()?;
                    Some(ProjectRef {
                        id: p["id"].as_str()?.to_string(),
                        title: p["title"].as_str().unwrap_or("").to_string(),
                        kind: p["kind"].as_str().unwrap_or("series").to_string(),
                        created_at: p["created_at"].as_u64().unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    v
}

/// 书名号、空格不算：「我在长安写诗词」和「《我在长安写诗词》」是同一个
fn norm_title(t: &str) -> String {
    t.chars().filter(|c| !matches!(c, '《' | '》' | ' ' | '　')).collect::<String>().to_lowercase()
}

fn resolve_project(cfg: &Path, key: &str) -> R<ProjectRef> {
    let all = all_projects(cfg);
    if let Some(p) = all.iter().find(|p| p.id == key) {
        return Ok(p.clone());
    }
    let k = norm_title(key);
    let hits: Vec<&ProjectRef> = all.iter().filter(|p| norm_title(&p.title) == k).collect();
    match hits.len() {
        1 => Ok(hits[0].clone()),
        0 => Err(ToolErr::Params(format!(
            "没有叫「{key}」的项目。现有：{}",
            all.iter().take(20).map(|p| format!("《{}》({})", p.title.trim_matches(|c| c == '《' || c == '》'), p.id)).collect::<Vec<_>>().join("、")
        ))),
        _ => Err(ToolErr::Params(format!("有 {} 个项目都叫「{key}」，用 id 指定：{}", hits.len(), hits.iter().map(|p| p.id.clone()).collect::<Vec<_>>().join("、")))),
    }
}

/// 最近 10 分钟内建过同名项目（模型重试时不建第二个）
fn recent_same_title(cfg: &Path, title: &str) -> Option<ProjectRef> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let k = norm_title(title);
    all_projects(cfg).into_iter().find(|p| norm_title(&p.title) == k && now.saturating_sub(p.created_at) < 600)
}

fn resolve_episode(cfg: &Path, project_id: &str, key: Option<&str>) -> R<String> {
    let eps: Vec<(String, u64, String)> = crate::studio_ops::episode_files(cfg, project_id)
        .iter()
        .filter_map(|f| {
            let v: Value = serde_json::from_str(&std::fs::read_to_string(f).ok()?).ok()?;
            Some((v["id"].as_str()?.to_string(), v["episode_num"].as_u64().unwrap_or(0), v["title"].as_str().unwrap_or("").to_string()))
        })
        .collect();
    if eps.is_empty() {
        return Err(ToolErr::Failed("这个项目还没有分集".into()));
    }
    let Some(key) = key else { return Ok(eps[0].0.clone()) };
    let num = key.trim_start_matches('第').trim_end_matches('集').trim().parse::<u64>().ok();
    eps.iter()
        .find(|(id, n, title)| id == key || Some(*n) == num || title == key)
        .map(|e| e.0.clone())
        .ok_or_else(|| ToolErr::Params(format!("没有分集「{key}」。现有：{}", eps.iter().map(|(id, _, t)| format!("{id}（{t}）")).collect::<Vec<_>>().join("、"))))
}

fn find_character(cfg: &Path, project_id: &str, key: &str) -> R<(Scope, crate::assets::Asset)> {
    let scopes = crate::assets::lookup_scopes(cfg, &crate::assets::RefScope::Project { project_id, episode_id: None });
    for sc in &scopes {
        if let Some(a) = crate::assets::load_store(cfg, sc).assets.into_iter().find(|a| a.kind == AssetKind::Character && (a.id == key || a.name.trim() == key)) {
            return Ok((sc.clone(), a));
        }
    }
    let names: Vec<String> = scopes.iter().flat_map(|sc| crate::assets::load_store(cfg, sc).assets).filter(|a| a.kind == AssetKind::Character).map(|a| a.name).collect();
    Err(ToolErr::Params(format!("这个项目里没有角色「{key}」。现有：{}", names.join("、"))))
}

// ── 读 ───────────────────────────────────────────────────────────────

fn list_projects(cfg: &Path) -> Value {
    let items = crate::cinema::workspace_items(cfg);
    json!({
        "count": items.len(),
        "projects": items.iter().take(30).map(|w| json!({
            "id": w.project.id, "title": w.project.title,
            "kind": if w.project.kind == "series" { "系列" } else { "独立项目" },
            "episodes": w.episode_count, "shots": w.shot_count, "rendered": w.rendered_count,
            "status": match w.status { "done" => "已完成", "active" => "进行中", _ => "草稿" },
        })).collect::<Vec<_>>()
    })
}

fn list_assets(cfg: &Path, project_id: &str, episode_id: Option<&str>, kind: Option<AssetKind>) -> Value {
    let scopes = crate::assets::lookup_scopes(cfg, &crate::assets::RefScope::Project { project_id, episode_id });
    let mut rows = Vec::new();
    for sc in &scopes {
        let place = match sc { Scope::Global => "全局库", Scope::Project { .. } => "项目", Scope::Episode { .. } => "本集" };
        for a in crate::assets::load_store(cfg, sc).assets {
            if kind.is_some_and(|k| k != a.kind) {
                continue;
            }
            let looks: Vec<String> = std::iter::once(if a.main_look_name.trim().is_empty() { "主形象".to_string() } else { a.main_look_name.clone() }).chain(a.looks.iter().map(|l| l.name.clone())).collect();
            rows.push(json!({
                "id": a.id, "kind": a.kind.id_prefix(), "name": a.name, "place": place,
                "has_image": a.unit.selected_image().is_some(),
                "looks": if a.kind == AssetKind::Character && looks.len() > 1 { json!(looks) } else { Value::Null },
            }));
        }
    }
    let total = rows.len();
    json!({"count": total, "without_image": rows.iter().filter(|r| r["has_image"] == false).count(), "assets": head(&rows, 60)})
}

/// 流程走到哪一步：剧本 → 资产 → 资产图 → 分镜 → 提示词 → 出片 → 合成
fn project_overview(cfg: &Path, p: &ProjectRef) -> Value {
    let art = crate::style::load_art(cfg, &p.id);
    let style = crate::style::find_preset(&art.preset_id).map(|x| x.name_zh.clone()).or_else(|| (!art.positive_prompt.trim().is_empty()).then(|| "自定义".to_string()));
    let mut episodes = Vec::new();
    let mut next = None;
    for f in crate::studio_ops::episode_files(cfg, &p.id) {
        let Ok(v) = serde_json::from_str::<Value>(&std::fs::read_to_string(&f).unwrap_or_default()) else { continue };
        let id = v["id"].as_str().unwrap_or("").to_string();
        let board = v["storyboard"].as_array().cloned().unwrap_or_default();
        let script_chars = v["script_text"].as_str().map(|t| t.trim().chars().count()).unwrap_or(0);
        let synthesized = board.iter().filter(|s| s["prompts"].as_object().is_some_and(|m| m.values().any(|p| p["video"].as_str().is_some_and(|t| !t.trim().is_empty())))).count();
        let rendered = board.iter().filter(|s| s["video_url"].as_str().is_some_and(|u| !u.is_empty())).count();
        let has_final = v["final_video_url"].as_str().is_some_and(|u| !u.is_empty());
        let scopes = crate::assets::lookup_scopes(cfg, &crate::assets::RefScope::Project { project_id: &p.id, episode_id: Some(&id) });
        let assets: Vec<crate::assets::Asset> = scopes.iter().filter(|sc| **sc != Scope::Global).flat_map(|sc| crate::assets::load_store(cfg, sc).assets).collect();
        let without_image = assets.iter().filter(|a| a.unit.selected_image().is_none()).count();
        let step = if script_chars == 0 {
            "导入剧本（studio_import_script）"
        } else if assets.is_empty() {
            "认出角色场景道具并建资产（studio_extract_entities → studio_create_assets）"
        } else if without_image > 0 {
            "给资产出图（studio_generate_asset_images）"
        } else if board.is_empty() {
            "生成分镜（studio_generate_shot_table）"
        } else if synthesized < board.len() {
            "合成提示词（studio_synthesize_prompts）"
        } else if rendered < board.len() {
            "出片（studio_render_episode）"
        } else if !has_final {
            "合成一集（studio_stitch_episode）"
        } else {
            "这一集已经完成"
        };
        if next.is_none() && step != "这一集已经完成" {
            next = Some(format!("{id}：{step}"));
        }
        episodes.push(json!({"id": id, "title": v["title"], "script_chars": script_chars, "assets": assets.len(), "assets_without_image": without_image,
            "shots": board.len(), "prompts_ready": synthesized, "rendered": rendered, "final": has_final, "next": step}));
    }
    json!({"project_id": p.id, "title": p.title, "kind": if p.kind == "series" { "系列" } else { "独立项目" }, "style": style, "episodes": episodes,
        "next": next.unwrap_or_else(|| "所有分集都完成了".into()), "studio_link": {"project_id": p.id}})
}

fn job_status(cfg: &Path, job_id: Option<&str>, project: Option<&ProjectRef>) -> Value {
    let log = crate::jobs::load(cfg);
    let brief = |j: &crate::jobs::Job| json!({
        "id": j.id, "kind": format!("{:?}", j.kind), "status": format!("{:?}", j.status), "title": clip(&j.prompt, 60),
        "progress": j.progress.as_ref().map(|p| format!("{}/{} {}", p.done, p.total, p.label)),
        "error": j.error.as_deref().map(|e| clip(e, 300)), "summary": j.summary,
    });
    if let Some(id) = job_id {
        return log.jobs.iter().find(|j| j.id == id).map(brief).unwrap_or_else(|| json!({"error": format!("没有任务「{id}」")}));
    }
    let mine: Vec<&crate::jobs::Job> = log.jobs.iter().rev().filter(|j| project.is_none_or(|p| j.project_id == p.id)).collect();
    let running: Vec<Value> = mine.iter().filter(|j| j.status == crate::jobs::JobStatus::Running).map(|j| brief(j)).collect();
    let recent: Vec<Value> = mine.iter().filter(|j| j.status != crate::jobs::JobStatus::Running).take(5).map(|j| brief(j)).collect();
    json!({"running": running, "recent": recent})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deps(dir: &Path) -> Deps {
        Deps { config_dir: dir.to_path_buf(), app: None, gateway: None }
    }

    async fn call(d: &Deps, op: Op, args: Value) -> R<Value> {
        run(op, d, &args).await
    }

    #[test]
    fn 审批弹窗说人话() {
        let a = approval_summary("studio_create_assets", &json!({"project": "《盛唐》", "assets": [
            {"kind": "character", "name": "林婉清"}, {"kind": "character", "name": "皇帝"}, {"kind": "scene", "name": "金銮殿"}]})).unwrap();
        assert_eq!(a, "在「盛唐」新建 2 个角色（林婉清、皇帝）、1 个场景（金銮殿），放在这一集；同名已存在的会跳过");
        assert_eq!(approval_summary("studio_create_project", &json!({"title": "《盛唐》", "kind": "standalone", "aspect_ratio": "16:9"})).unwrap(), "新建独立项目「盛唐」（16:9），自带空的第 1 集");
        assert!(approval_summary("studio_import_script", &json!({"project": "盛唐", "text": "十个字的剧本正文啊", "replace": true})).unwrap().contains("覆盖"));
        // 每个写工具都有摘要；不是 Studio 工具不管
        for op in ALL_OPS {
            let sp = spec_of(*op);
            if sp.operation == Operation::Write {
                assert!(approval_summary(&sp.name, &json!({})).is_some(), "{} 没有审批摘要", sp.name);
            }
        }
        assert!(approval_summary("shell_command", &json!({})).is_none());
        // 生成类只能允许一次；建东西的可以本会话允许
        assert!(ALL_OPS.iter().map(|o| spec_of(*o).name).filter(|n| once_only(n)).count() == 5);
        assert!(!once_only("studio_create_assets"));
        use crate::agent_runtime::approval::{ApprovalRequest, ApprovalScope};
        let render = ApprovalRequest::new("studio_render_episode", Operation::Write, crate::mcp::Risk::Medium, "", &json!({}));
        assert!(render.once_only);
        assert_eq!(render.clamp_scope(Some(ApprovalScope::Workspace)), ApprovalScope::Once, "前端传了本项目允许也只算一次");
        let create = ApprovalRequest::new("studio_create_assets", Operation::Write, crate::mcp::Risk::Medium, "", &json!({}));
        assert_eq!(create.clamp_scope(Some(ApprovalScope::Session)), ApprovalScope::Session);
        let r = approval_summary("studio_render_episode", &json!({"project": "盛唐", "shots": [3, 5]})).unwrap();
        assert!(r.contains("第 3、5 镜") && r.contains("小时") && r.contains("Ollama Cloud"), "{r}");
        assert!(approval_summary("studio_generate_shot_table", &json!({"project": "盛唐", "replace": true})).unwrap().contains("覆盖"));
    }

    #[test]
    fn 工具名都是_studio_前缀_读工具不走审批() {
        for op in ALL_OPS {
            let s = spec_of(*op);
            assert!(s.name.starts_with("studio_"), "{}", s.name);
            assert!(s.parameters["type"] == "object");
        }
        assert_eq!(spec_of(Op::ListProjects).operation, Operation::Read);
        assert_eq!(spec_of(Op::CreateAssets).operation, Operation::Write);
        assert_eq!(ALL_OPS.len(), 24);
    }

    /// 从零到资产建好：建项目 → 导剧本 → 建资产 → 加形象 → 看概况，每一步的「下一步」对得上
    #[tokio::test]
    async fn 建项目到建资产走一遍() {
        let dir = tempfile::tempdir().unwrap();
        let d = deps(dir.path());
        let p = call(&d, Op::CreateProject, json!({"title": "《我在长安写诗词》", "kind": "standalone", "aspect_ratio": "16:9"})).await.unwrap();
        let pid = p["project_id"].as_str().unwrap().to_string();
        // 模型重试：不建第二个；名字带不带书名号都认
        let again = call(&d, Op::CreateProject, json!({"title": "我在长安写诗词", "kind": "standalone"})).await.unwrap();
        assert_eq!(again["project_id"], pid.as_str());
        assert_eq!(all_projects(dir.path()).len(), 1);

        let ov = call(&d, Op::GetProject, json!({"project": "我在长安写诗词"})).await.unwrap();
        assert!(ov["next"].as_str().unwrap().contains("导入剧本"), "{ov}");

        let imp = call(&d, Op::ImportScript, json!({"project": pid, "episode": "1", "text": "【场景：金銮殿 · 日】\n皇帝：斩。\n林婉清：这是唐朝？"})).await.unwrap();
        assert_eq!((imp["scenes"].as_u64(), imp["speakers"].as_array().unwrap().len()), (Some(1), 2));
        assert!(matches!(call(&d, Op::ImportScript, json!({"project": pid, "text": "别的"})).await, Err(ToolErr::Params(_))), "已有剧本不覆盖");

        let ov = call(&d, Op::GetProject, json!({"project": pid})).await.unwrap();
        assert!(ov["next"].as_str().unwrap().contains("建资产"), "{ov}");

        let made = call(&d, Op::CreateAssets, json!({"project": pid, "assets": [
            {"kind": "character", "name": "林婉清", "identity": "女，28 岁", "description": "男装官服"},
            {"kind": "scene", "name": "金銮殿", "description": "内景"}
        ]})).await.unwrap();
        assert_eq!(made["created"].as_array().unwrap().len(), 2);
        let again = call(&d, Op::CreateAssets, json!({"project": pid, "assets": [{"kind": "character", "name": "林婉清"}]})).await.unwrap();
        assert_eq!(again["skipped"].as_array().unwrap().len(), 1);

        let look = call(&d, Op::AddLook, json!({"project": pid, "character": "林婉清", "name": "现代", "description": "白衬衫"})).await.unwrap();
        assert!(look["look_id"].as_str().unwrap().starts_with("look_"));
        assert!(matches!(call(&d, Op::AddLook, json!({"project": pid, "character": "李白", "name": "x"})).await, Err(ToolErr::Params(m)) if m.contains("林婉清")), "找不到时列出现有的");

        let list = call(&d, Op::ListAssets, json!({"project": pid})).await.unwrap();
        assert_eq!((list["count"].as_u64(), list["without_image"].as_u64()), (Some(2), Some(2)));
        assert_eq!(list["assets"][0]["looks"], json!(["主形象", "现代"]));
        let ov = call(&d, Op::GetProject, json!({"project": pid})).await.unwrap();
        assert!(ov["next"].as_str().unwrap().contains("出图"), "{ov}");
    }

    /// P1 的生成工具：前提不满足时说清楚缺哪一步、下一步用哪个工具，不发起任务
    #[tokio::test]
    async fn 生成类工具_前提不够时指路() {
        let dir = tempfile::tempdir().unwrap();
        let d = deps(dir.path());
        let p = call(&d, Op::CreateProject, json!({"title": "米小团", "kind": "standalone"})).await.unwrap();
        let pid = p["project_id"].as_str().unwrap().to_string();
        let err = |r: R<Value>| match r { Err(ToolErr::Params(m)) => m, other => panic!("应该是参数错误：{other:?}") };

        assert!(err(call(&d, Op::GenerateAssetImages, json!({"project": pid})).await).contains("还没有资产"));
        assert!(err(call(&d, Op::GenerateShotTable, json!({"project": pid})).await).contains("studio_import_script"));
        assert!(err(call(&d, Op::SynthesizePrompts, json!({"project": pid})).await).contains("studio_generate_shot_table"));
        assert!(err(call(&d, Op::StitchEpisode, json!({"project": pid})).await).contains("studio_render_episode"));

        call(&d, Op::ImportScript, json!({"project": pid, "text": "【场景：巷口 · 夜】\n小猫：喵。"})).await.unwrap();
        // 已有分镜：不带 replace 不覆盖
        let f = cinema_projects_dir(dir.path()).join(&pid).join("episodes").join("ep_01").join("episode.json");
        let mut ep: Value = serde_json::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap();
        let shot = json!({"id": "shot_a", "shot_num": 1, "scene_no": 1, "scale": "medium", "camera_movement": "static", "score": "", "prompt": "", "duration": 5.0, "status": "pending", "dialogue": ""});
        serde_json::from_value::<crate::cinema::StoryboardShot>(shot.clone()).expect("样例镜头要能被出片那边读");
        ep["storyboard"] = json!([shot]);
        std::fs::write(&f, ep.to_string()).unwrap();
        assert!(err(call(&d, Op::GenerateShotTable, json!({"project": pid})).await).contains("replace=true"));
        // 镜号对不上
        assert!(err(call(&d, Op::SynthesizePrompts, json!({"project": pid, "shots": [1, 9]})).await).contains("第 9 镜"));
        assert_eq!(shot_ids_by_num(dir.path(), &pid, "ep_01", Some(&json!(["第1镜"]))).unwrap(), ["shot_a"]);
        // 没合成提示词：出不了片，指向合成
        assert!(err(call(&d, Op::RenderEpisode, json!({"project": pid})).await).contains("studio_synthesize_prompts"));
        // 出哪几镜：合成过、还没用这条线路出过的优先；都出过就全部重出；没合成的数进 blocked
        let pf = cinema_projects_dir(dir.path()).join(&pid).join("project.json");
        let mut proj: Value = serde_json::from_str(&std::fs::read_to_string(&pf).unwrap()).unwrap();
        proj["model_settings"]["prompt_route"] = json!("b");
        std::fs::write(&pf, proj.to_string()).unwrap();
        let take = json!({"route": "b", "path": "/x.mp4", "created_at": 1, "seed": 1});
        let with = |id: &str, num: u32, takes: Value| json!({"id": id, "shot_num": num, "scene_no": 1, "scale": "medium", "camera_movement": "static", "score": "", "prompt": "", "duration": 5.0, "status": "pending", "dialogue": "",
            "prompts": {"b": {"video": "推近", "synthesized_at": 1}}, "takes": takes});
        let mut shots = vec![with("shot_a", 1, json!([])), with("shot_b", 2, json!([take])), json!({"id": "shot_c", "shot_num": 3, "scene_no": 1, "scale": "", "camera_movement": "", "score": "", "prompt": "", "duration": 5.0, "status": "pending", "dialogue": ""})];
        ep["storyboard"] = json!(shots);
        std::fs::write(&f, ep.to_string()).unwrap();
        assert_eq!(crate::cinema::batch_render_targets(dir.path(), &pid, "ep_01").unwrap(), (vec!["shot_a".to_string()], false, 1));
        shots[0]["takes"] = json!([take]);
        ep["storyboard"] = json!(shots);
        std::fs::write(&f, ep.to_string()).unwrap();
        assert_eq!(crate::cinema::batch_render_targets(dir.path(), &pid, "ep_01").unwrap(), (vec!["shot_a".to_string(), "shot_b".to_string()], true, 1));
        // **勾了整段出的场不进逐镜那一批。** 不摘的话这一场的镜头会被一镜一镜
        // 再出一遍：GPU 白烧，而且出来的 take 和整段出的成片是两套东西
        // （2026-09-20 用户问「批量出片能不能和长镜头做成一起的」时查出来的）。
        ep["long_take_scenes"] = json!([1]);
        std::fs::write(&f, ep.to_string()).unwrap();
        assert_eq!(
            crate::cinema::batch_render_targets(dir.path(), &pid, "ep_01").unwrap(),
            (vec![], false, 1),
            "第 1 场勾了整段出，它的镜头还被逐镜排上了",
        );
        ep["long_take_scenes"] = json!([]);
        std::fs::write(&f, ep.to_string()).unwrap();
        // 接不上界面时是执行失败，不是参数错误
        assert!(matches!(call(&d, Op::SynthesizePrompts, json!({"project": pid})).await, Err(ToolErr::Failed(_))));
    }

    #[tokio::test]
    async fn 名字找不到或重名时列出候选() {
        let dir = tempfile::tempdir().unwrap();
        let d = deps(dir.path());
        for _ in 0..2 {
            let req: crate::cinema::CreateProjectReq = serde_json::from_value(json!({"title": "盛唐"})).unwrap();
            crate::studio_ops::create_project(dir.path(), req).unwrap();
        }
        let dup = call(&d, Op::GetProject, json!({"project": "盛唐"})).await;
        assert!(matches!(dup, Err(ToolErr::Params(m)) if m.contains("2 个项目")));
        let none = call(&d, Op::GetProject, json!({"project": "米小团"})).await;
        assert!(matches!(none, Err(ToolErr::Params(m)) if m.contains("现有")));
        assert!(matches!(call(&d, Op::CreateAssets, json!({"project": "x"})).await, Err(ToolErr::Params(_))));
    }

    #[tokio::test]
    async fn 设风格_查任务() {
        let dir = tempfile::tempdir().unwrap();
        let d = deps(dir.path());
        let p = call(&d, Op::CreateProject, json!({"title": "米小团", "kind": "series"})).await.unwrap();
        let pid = p["project_id"].as_str().unwrap();
        let first = crate::style::all_presets()[0].id.clone();
        let r = call(&d, Op::SetStyle, json!({"project": pid, "preset_id": first})).await.unwrap();
        assert_eq!(r["preset_id"], first.as_str());
        assert!(matches!(call(&d, Op::SetStyle, json!({"project": pid})).await, Err(ToolErr::Params(_))));
        let ep = call(&d, Op::CreateEpisode, json!({"project": pid, "title": "第二集"})).await.unwrap();
        assert_eq!(ep["episode_id"], "ep_02");
        let jobs = call(&d, Op::JobStatus, json!({"project": pid})).await.unwrap();
        assert!(jobs["running"].as_array().unwrap().is_empty());
    }

    fn small_workflow(prompt: &str) -> Value {
        json!({
            "nodes": [
                {"id": "img", "type": "imageGen", "position": {"x": 0, "y": 0},
                 "data": {"engine": "krea2", "prompt": prompt, "guide": ["【第 1 步 / 共 1 步】出图"], "outputImageUrl": "/Users/me/old.png"}},
                {"id": "up", "type": "imageGen", "position": {"x": 0, "y": 300},
                 "data": {"engine": "krea2", "prompt": "", "guide": ["【第 0 步】上传参考图"]}},
            ],
            "edges": [{"id": "e1", "source": "up", "target": "img"}],
        })
    }

    /// 存 → 读 → 覆盖：不带 revision 拒、revision 过期拒、带对的才覆盖
    #[tokio::test]
    async fn 工作流存读覆盖() {
        let dir = tempfile::tempdir().unwrap();
        let d = deps(dir.path());
        let p = call(&d, Op::CreateProject, json!({"title": "上课不要玩手机", "kind": "series"})).await.unwrap();
        let pid = p["project_id"].as_str().unwrap().to_string();

        let saved = call(&d, Op::WorkflowSave, json!({"project": "上课不要玩手机", "name": "EP09 测试", "workflow": small_workflow("一张教室的照片")})).await.unwrap();
        assert_eq!(saved["nodes"], 2);
        assert!(saved["stripped"].as_array().unwrap().iter().any(|x| x == "img.outputImageUrl"), "产物字段要剥掉");
        let file = dir.path().join("anima/canvas/workflows").join(&pid).join("EP09 测试.json");
        let on_disk: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(on_disk["version"], 1);
        assert!(on_disk["nodes"][0]["data"].get("outputImageUrl").is_none());
        assert!(on_disk["nodes"][0]["data"]["guide"].is_array());

        let listed = call(&d, Op::WorkflowList, json!({"project": &pid})).await.unwrap();
        assert_eq!(listed["workflows"][0]["name"], "EP09 测试");
        assert_eq!(listed["workflows"][0]["nodes"], 2);

        let read = call(&d, Op::WorkflowRead, json!({"project": &pid, "name": "EP09 测试"})).await.unwrap();
        let rev = read["revision"].as_str().unwrap().to_string();

        // 同名、不带 revision：拒
        let e = call(&d, Op::WorkflowSave, json!({"project": &pid, "name": "EP09 测试", "workflow": small_workflow("改过的")})).await.unwrap_err();
        assert!(matches!(e, ToolErr::Params(ref m) if m.contains("studio_workflow_read")), "{e:?}");

        // 用户在界面上又存了一次：revision 过期，拒
        std::fs::write(&file, std::fs::read_to_string(&file).unwrap().replace("一张教室的照片", "用户改的")).unwrap();
        let e = call(&d, Op::WorkflowSave, json!({"project": &pid, "name": "EP09 测试", "workflow": small_workflow("改过的"), "revision": rev})).await.unwrap_err();
        assert!(matches!(e, ToolErr::Params(ref m) if m.contains("被改过")), "{e:?}");
        assert!(std::fs::read_to_string(&file).unwrap().contains("用户改的"), "用户的版本不能被盖掉");

        // 重新读、带新的 revision：覆盖成功
        let rev2 = call(&d, Op::WorkflowRead, json!({"project": &pid, "name": "EP09 测试"})).await.unwrap()["revision"].as_str().unwrap().to_string();
        call(&d, Op::WorkflowSave, json!({"project": &pid, "name": "EP09 测试", "workflow": small_workflow("改过的"), "revision": rev2})).await.unwrap();
        assert!(std::fs::read_to_string(&file).unwrap().contains("改过的"));
    }

    #[tokio::test]
    async fn 工作流有错误就不存() {
        let dir = tempfile::tempdir().unwrap();
        let d = deps(dir.path());
        call(&d, Op::CreateProject, json!({"title": "上课不要玩手机", "kind": "series"})).await.unwrap();
        let mut wf = small_workflow("横构图 16:9 的教室");
        wf["edges"] = json!([{"id": "e1", "source": "up", "target": "ghost"}]);
        let e = call(&d, Op::WorkflowSave, json!({"project": "上课不要玩手机", "name": "坏的", "workflow": wf.clone()})).await.unwrap_err();
        let ToolErr::Params(m) = e else { panic!("应当是参数错误") };
        assert!(m.contains("ghost") && m.contains("尺寸"), "两条都要报：{m}");
        assert!(call(&d, Op::WorkflowList, json!({})).await.unwrap()["count"] == 0, "有错误就一个文件都不落");

        // check 只检查：同样的错误，不落盘
        let c = call(&d, Op::WorkflowCheck, json!({"workflow": wf})).await.unwrap();
        assert_eq!(c["ok"], false);
        assert_eq!(c["errors"].as_array().unwrap().len(), 2);

        // 工作流当字符串传也认
        let c = call(&d, Op::WorkflowCheck, json!({"workflow": small_workflow("一张照片").to_string()})).await.unwrap();
        assert_eq!(c["ok"], true, "{c}");
    }
}
