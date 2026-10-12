//! Cinema pipeline tools exposed through the shared runtime.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::json;

use super::{
    RuntimeContext, ToolCallRequest, ToolFailure, ToolFailureKind, ToolHandler, ToolKind,
    ToolOutcome, ToolRegistry, ToolSpec,
};
use crate::mcp::Operation;

#[derive(Clone)]
pub struct CinemaExportJianyingTool {
    config_dir: PathBuf,
}

#[derive(Clone)]
pub struct CinemaRenderShotVideoTool {
    config_dir: PathBuf,
}

#[derive(Clone)]
pub struct CinemaRenderShotTool {
    config_dir: PathBuf,
    http: reqwest::Client,
}

#[derive(Clone)]
pub struct CinemaStitchEpisodeTool {
    config_dir: PathBuf,
}

#[derive(Clone)]
pub struct CinemaGenerateAiSequelTool {
    config_dir: PathBuf,
}

#[derive(Clone)]
pub struct CinemaGenerateShotAudioTool {
    config_dir: PathBuf,
    gateway: Arc<crate::llm::gateway::Gateway>,
    http: reqwest::Client,
}

#[derive(Clone)]
pub struct CinemaGenerateCharacterSheetTool {
    config_dir: PathBuf,
    http: reqwest::Client,
    app_handle: Option<tauri::AppHandle>,
}

#[derive(Clone)]
pub struct CinemaSliceCharacterSheetTool {
    config_dir: PathBuf,
}

#[derive(Clone)]
pub struct CinemaUpdateCharacterAssetSlotTool {
    config_dir: PathBuf,
}

impl CinemaExportJianyingTool {
    pub fn new(config_dir: impl Into<PathBuf>) -> Self {
        Self {
            config_dir: config_dir.into(),
        }
    }
}

impl CinemaRenderShotVideoTool {
    pub fn new(config_dir: impl Into<PathBuf>) -> Self {
        Self {
            config_dir: config_dir.into(),
        }
    }
}

impl CinemaRenderShotTool {
    pub fn new(config_dir: impl Into<PathBuf>, http: reqwest::Client) -> Self {
        Self {
            config_dir: config_dir.into(),
            http,
        }
    }
}

impl CinemaStitchEpisodeTool {
    pub fn new(config_dir: impl Into<PathBuf>) -> Self {
        Self {
            config_dir: config_dir.into(),
        }
    }
}

impl CinemaGenerateAiSequelTool {
    pub fn new(config_dir: impl Into<PathBuf>) -> Self {
        Self {
            config_dir: config_dir.into(),
        }
    }
}

impl CinemaGenerateShotAudioTool {
    pub fn new(
        config_dir: impl Into<PathBuf>,
        gateway: Arc<crate::llm::gateway::Gateway>,
        http: reqwest::Client,
    ) -> Self {
        Self {
            config_dir: config_dir.into(),
            gateway,
            http,
        }
    }
}

impl CinemaGenerateCharacterSheetTool {
    pub fn new(
        config_dir: impl Into<PathBuf>,
        http: reqwest::Client,
        app_handle: Option<tauri::AppHandle>,
    ) -> Self {
        Self {
            config_dir: config_dir.into(),
            http,
            app_handle,
        }
    }
}

impl CinemaSliceCharacterSheetTool {
    pub fn new(config_dir: impl Into<PathBuf>) -> Self {
        Self {
            config_dir: config_dir.into(),
        }
    }
}

impl CinemaUpdateCharacterAssetSlotTool {
    pub fn new(config_dir: impl Into<PathBuf>) -> Self {
        Self {
            config_dir: config_dir.into(),
        }
    }
}

pub fn register_cinema_tools(registry: &mut ToolRegistry, config_dir: impl Into<PathBuf>) {
    let config_dir = config_dir.into();
    registry.register(CinemaExportJianyingTool::new(config_dir.clone()));
    registry.register(CinemaRenderShotVideoTool::new(config_dir.clone()));
    registry.register(CinemaStitchEpisodeTool::new(config_dir.clone()));

    registry.register(CinemaGenerateAiSequelTool::new(config_dir));
}

pub fn register_cinema_audio_tools(
    registry: &mut ToolRegistry,
    config_dir: impl Into<PathBuf>,
    gateway: Arc<crate::llm::gateway::Gateway>,
    http: reqwest::Client,
) {
    registry.register(CinemaGenerateShotAudioTool::new(config_dir, gateway, http));
}

pub fn register_cinema_http_tools(
    registry: &mut ToolRegistry,
    config_dir: impl Into<PathBuf>,
    http: reqwest::Client,
    app_handle: Option<tauri::AppHandle>,
) {
    let config_dir = config_dir.into();
    registry.register(CinemaRenderShotTool::new(config_dir.clone(), http.clone()));
    registry.register(CinemaGenerateCharacterSheetTool::new(
        config_dir.clone(),
        http,
        app_handle,
    ));
    registry.register(CinemaSliceCharacterSheetTool::new(config_dir.clone()));
    registry.register(CinemaUpdateCharacterAssetSlotTool::new(config_dir));
}

#[async_trait]
impl ToolHandler for CinemaExportJianyingTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "cinema_export_jianying",
            "Export the specified episode into a Jianying / CapCut draft project directory.",
            ToolKind::Cinema,
            Operation::Write,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "project_id": { "type": "string" },
                "episode_id": { "type": "string" }
            },
            "required": ["project_id", "episode_id"]
        }))
        .with_idempotent(false)
        .with_timeout(Duration::from_secs(30))
    }

    async fn execute(&self, _ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let Some(project_id) = request.arguments.get("project_id").and_then(|v| v.as_str()) else {
            return invalid_params(&request, "缺少 project_id 参数");
        };
        let Some(episode_id) = request.arguments.get("episode_id").and_then(|v| v.as_str()) else {
            return invalid_params(&request, "缺少 episode_id 参数");
        };
        let started = Instant::now();
        match crate::cinema::cinema_export_jianying_project_with_config_dir(
            &self.config_dir,
            project_id.to_string(),
            episode_id.to_string(),
        )
        .await
        {
            Ok(path) => {
                let payload = json!({
                    "project_id": project_id,
                    "episode_id": episode_id,
                    "exported_path": path,
                });
                ToolOutcome::ok(
                    &request,
                    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string()),
                    vec![path],
                    started.elapsed(),
                )
            }
            Err(err) => ToolOutcome::failed(
                &request,
                cinema_tool_failure(err.to_string()),
                started.elapsed(),
            ),
        }
    }
}

#[async_trait]
impl ToolHandler for CinemaGenerateAiSequelTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "cinema_generate_ai_sequel",
            "Generate continuous sequel script for current episode based on previous episode hook and direction.",
            ToolKind::Cinema,
            Operation::Write,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "project_id": { "type": "string" },
                "episode_id": { "type": "string" },
                "prev_episode_id": { "type": "string" },
                "direction": { "type": ["string", "null"] }
            },
            "required": ["project_id", "episode_id", "prev_episode_id"]
        }))
        .with_idempotent(false)
        .with_timeout(Duration::from_secs(30))
    }

    async fn execute(&self, _ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let Some(project_id) = string_arg(&request, "project_id") else {
            return invalid_params(&request, "缺少 project_id 参数");
        };
        let Some(episode_id) = string_arg(&request, "episode_id") else {
            return invalid_params(&request, "缺少 episode_id 参数");
        };
        let req = match serde_json::from_value::<crate::cinema::GenerateAiSequelReq>(
            request.arguments.clone(),
        ) {
            Ok(req) => req,
            Err(err) => return invalid_params(&request, format!("剧本企划参数无效：{err}")),
        };
        let started = Instant::now();
        match crate::cinema::cinema_generate_ai_sequel_with_config_dir(&self.config_dir, req).await
        {
            Ok(sequel) => {
                let ep_file = crate::cinema::cinema_projects_dir(&self.config_dir)
                    .join(project_id)
                    .join("episodes")
                    .join(episode_id)
                    .join("episode.json");
                if !ep_file.is_file() {
                    return ToolOutcome::failed(
                        &request,
                        ToolFailure::new(ToolFailureKind::Execution, "分集不存在"),
                        started.elapsed(),
                    );
                }
                if std::fs::read_to_string(&ep_file).ok().and_then(|content| {
                    serde_json::from_str::<crate::cinema::EpisodeData>(&content).ok()
                }).is_none() {
                    return ToolOutcome::failed(
                        &request,
                        ToolFailure::new(ToolFailureKind::Execution, "读取当前分集失败"),
                        started.elapsed(),
                    );
                }
                if let Err(err) = crate::episode_lock::update_file(&ep_file, |ep| {
                    ep.title = sequel.title.clone();
                    ep.synopsis = sequel.synopsis.clone();
                    ep.script_text = sequel.script.clone();
                    ep.storyboard.clear();
                    Ok(())
                }) {
                    return ToolOutcome::failed(
                        &request,
                        ToolFailure::new(
                            ToolFailureKind::Execution,
                            format!("保存续写剧本失败：{err}"),
                        ),
                        started.elapsed(),
                    );
                }
                let payload = json!({
                    "project_id": project_id,
                    "episode_id": episode_id,
                    "title": sequel.title,
                    "synopsis": sequel.synopsis,
                    "script_chars": sequel.script.chars().count(),
                    "episode_file": ep_file.to_string_lossy(),
                });
                ToolOutcome::ok(
                    &request,
                    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string()),
                    vec![ep_file.to_string_lossy().to_string()],
                    started.elapsed(),
                )
            }
            Err(err) => ToolOutcome::failed(
                &request,
                cinema_tool_failure(err.to_string()),
                started.elapsed(),
            ),
        }
    }
}

#[async_trait]
impl ToolHandler for CinemaRenderShotTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "cinema_render_shot",
            "Render keyframe image for the specified storyboard shot and write back to episode.json.",
            ToolKind::Cinema,
            Operation::Write,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "project_id": { "type": "string" },
                "episode_id": { "type": "string" },
                "shot_id": { "type": "string" },
                "engine_config": { "type": "object" }
            },
            "required": ["project_id", "episode_id", "shot_id"]
        }))
        .with_idempotent(false)
        .with_timeout(Duration::from_secs(240))
    }

    async fn execute(&self, _ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let Some(project_id) = string_arg(&request, "project_id") else {
            return invalid_params(&request, "缺少 project_id 参数");
        };
        let Some(episode_id) = string_arg(&request, "episode_id") else {
            return invalid_params(&request, "缺少 episode_id 参数");
        };
        let Some(shot_id) = string_arg(&request, "shot_id") else {
            return invalid_params(&request, "缺少 shot_id 参数");
        };
        let engine_config = request
            .arguments
            .get("engine_config")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok());
        let req = crate::cinema::RenderShotReq {
            project_id: project_id.to_string(),
            episode_id: episode_id.to_string(),
            shot_id: shot_id.to_string(),
            engine_config,
        };
        let started = Instant::now();
        match crate::cinema::cinema_render_shot_with_deps(&self.config_dir, self.http.clone(), req)
            .await
        {
            Ok(result) => {
                let image_path = crate::cinema::cinema_projects_dir(&self.config_dir)
                    .join(project_id)
                    .join("episodes")
                    .join(episode_id)
                    .join("shots")
                    .join(format!("{shot_id}.png"));
                let mut artifacts = Vec::new();
                if image_path.is_file() {
                    artifacts.push(image_path.to_string_lossy().to_string());
                }
                let payload = json!({
                    "project_id": project_id,
                    "episode_id": episode_id,
                    "shot_id": result.shot_id,
                    "status": result.status,
                    "image_path": image_path.to_string_lossy(),
                    "has_image": !result.image_url.is_empty(),
                });
                ToolOutcome::ok(
                    &request,
                    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string()),
                    artifacts,
                    started.elapsed(),
                )
            }
            Err(err) => ToolOutcome::failed(
                &request,
                cinema_tool_failure(err.to_string()),
                started.elapsed(),
            ),
        }
    }
}

#[async_trait]
impl ToolHandler for CinemaRenderShotVideoTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "cinema_render_shot_video",
            "Generate dynamic video sequence from keyframe image for the specified storyboard shot and update episode.json.",
            ToolKind::Cinema,
            Operation::Write,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "project_id": { "type": "string" },
                "episode_id": { "type": "string" },
                "shot_id": { "type": "string" },
                "engine_config": { "type": "object" }
            },
            "required": ["project_id", "episode_id", "shot_id"]
        }))
        .with_idempotent(false)
        .with_timeout(Duration::from_secs(180))
    }

    async fn execute(&self, _ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let Some(project_id) = string_arg(&request, "project_id") else {
            return invalid_params(&request, "缺少 project_id 参数");
        };
        let Some(episode_id) = string_arg(&request, "episode_id") else {
            return invalid_params(&request, "缺少 episode_id 参数");
        };
        let Some(shot_id) = string_arg(&request, "shot_id") else {
            return invalid_params(&request, "缺少 shot_id 参数");
        };
        let engine_config = request
            .arguments
            .get("engine_config")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok());
        let req = crate::cinema::RenderShotReq {
            project_id: project_id.to_string(),
            episode_id: episode_id.to_string(),
            shot_id: shot_id.to_string(),
            engine_config,
        };
        let started = Instant::now();
        match crate::cinema::cinema_render_shot_video_with_config_dir(&self.config_dir, req).await {
            Ok(result) => {
                let video_path = crate::cinema::cinema_projects_dir(&self.config_dir)
                    .join(project_id)
                    .join("episodes")
                    .join(episode_id)
                    .join("shots")
                    .join(format!("{shot_id}.mp4"));
                let mut artifacts = Vec::new();
                if video_path.is_file() {
                    artifacts.push(video_path.to_string_lossy().to_string());
                }
                let payload = json!({
                    "project_id": project_id,
                    "episode_id": episode_id,
                    "shot_id": result.shot_id,
                    "status": result.status,
                    "video_path": video_path.to_string_lossy(),
                    "has_video": !result.video_url.is_empty(),
                });
                ToolOutcome::ok(
                    &request,
                    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string()),
                    artifacts,
                    started.elapsed(),
                )
            }
            Err(err) => ToolOutcome::failed(
                &request,
                cinema_tool_failure(err.to_string()),
                started.elapsed(),
            ),
        }
    }
}

#[async_trait]
impl ToolHandler for CinemaStitchEpisodeTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "cinema_stitch_episode",
            "Concatenate rendered shot videos and audio dialogue tracks into final cut episode video, updating episode.json.",
            ToolKind::Cinema,
            Operation::Write,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "project_id": { "type": "string" },
                "episode_id": { "type": "string" }
            },
            "required": ["project_id", "episode_id"]
        }))
        .with_idempotent(false)
        .with_timeout(Duration::from_secs(240))
    }

    async fn execute(&self, _ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let Some(project_id) = string_arg(&request, "project_id") else {
            return invalid_params(&request, "缺少 project_id 参数");
        };
        let Some(episode_id) = string_arg(&request, "episode_id") else {
            return invalid_params(&request, "缺少 episode_id 参数");
        };
        let started = Instant::now();
        match crate::cinema::cinema_stitch_episode_with_config_dir(
            &self.config_dir,
            project_id.to_string(),
            episode_id.to_string(),
        )
        .await
        {
            Ok(final_video_url) => {
                let final_path = crate::cinema::cinema_projects_dir(&self.config_dir)
                    .join(project_id)
                    .join("episodes")
                    .join(episode_id)
                    .join("final.mp4");
                let mut artifacts = Vec::new();
                if final_path.is_file() {
                    artifacts.push(final_path.to_string_lossy().to_string());
                }
                let payload = json!({
                    "project_id": project_id,
                    "episode_id": episode_id,
                    "final_video_url": final_video_url,
                    "final_video_path": final_path.to_string_lossy(),
                });
                ToolOutcome::ok(
                    &request,
                    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string()),
                    artifacts,
                    started.elapsed(),
                )
            }
            Err(err) => ToolOutcome::failed(
                &request,
                cinema_tool_failure(err.to_string()),
                started.elapsed(),
            ),
        }
    }
}

#[async_trait]
impl ToolHandler for CinemaGenerateShotAudioTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "cinema_generate_shot_audio",
            "Synthesize TTS voiceover and sound effects for the specified shot dialogue, updating episode.json.",
            ToolKind::Cinema,
            Operation::Write,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "project_id": { "type": "string" },
                "episode_id": { "type": "string" },
                "shot_id": { "type": "string" },
                "engine_config": { "type": "object" }
            },
            "required": ["project_id", "episode_id", "shot_id"]
        }))
        .with_idempotent(false)
        .with_timeout(Duration::from_secs(180))
    }

    async fn execute(&self, _ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let Some(project_id) = string_arg(&request, "project_id") else {
            return invalid_params(&request, "缺少 project_id 参数");
        };
        let Some(episode_id) = string_arg(&request, "episode_id") else {
            return invalid_params(&request, "缺少 episode_id 参数");
        };
        let Some(shot_id) = string_arg(&request, "shot_id") else {
            return invalid_params(&request, "缺少 shot_id 参数");
        };
        let engine_config = request
            .arguments
            .get("engine_config")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok());
        let req = crate::cinema::RenderShotReq {
            project_id: project_id.to_string(),
            episode_id: episode_id.to_string(),
            shot_id: shot_id.to_string(),
            engine_config,
        };
        let started = Instant::now();
        match crate::cinema::cinema_generate_shot_audio_with_deps(
            &self.config_dir,
            self.gateway.as_ref(),
            self.http.clone(),
            req,
        )
        .await
        {
            Ok(result) => {
                let audio_path = crate::cinema::cinema_projects_dir(&self.config_dir)
                    .join(project_id)
                    .join("episodes")
                    .join(episode_id)
                    .join("audio")
                    .join(format!("{shot_id}.wav"));
                let mut artifacts = Vec::new();
                if audio_path.is_file() {
                    artifacts.push(audio_path.to_string_lossy().to_string());
                }
                let payload = json!({
                    "project_id": project_id,
                    "episode_id": episode_id,
                    "shot_id": result.shot_id,
                    "status": result.status,
                    "audio_path": audio_path.to_string_lossy(),
                    "has_audio": !result.audio_url.is_empty(),
                });
                ToolOutcome::ok(
                    &request,
                    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string()),
                    artifacts,
                    started.elapsed(),
                )
            }
            Err(err) => ToolOutcome::failed(
                &request,
                cinema_tool_failure(err.to_string()),
                started.elapsed(),
            ),
        }
    }
}

#[async_trait]
impl ToolHandler for CinemaGenerateCharacterSheetTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "cinema_generate_character_sheet_via_comfyui",
            "Invoke local ComfyUI to generate character turnaround views, expressions, poses, and facial angle assets.",
            ToolKind::Cinema,
            Operation::Write,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "project_id": { "type": "string" },
                "character_id": { "type": "string" },
                "name": { "type": "string" },
                "trigger_words": { "type": "string" },
                "clothing_prompt": { "type": ["string", "null"] },
                "front_image_url": { "type": ["string", "null"] },
                "checkpoint": { "type": ["string", "null"] },
                "lora": { "type": ["string", "null"] },
                "lora_weight": { "type": ["number", "null"] },
                "consistency_mode": { "type": ["string", "null"] },
                "seed": { "type": ["integer", "null"] },
                "visual_dna": { "type": ["object", "null"] },
                "use_reference_for_turnaround": { "type": ["boolean", "null"] },
                "tiers": { "type": ["array", "null"], "items": { "type": "string" } },
                "engine": { "type": ["string", "null"] },
                "debug_one_slot": { "type": ["boolean", "null"] },
                "max_attempts": { "type": ["integer", "null"] }
            },
            "required": ["project_id", "character_id", "name", "trigger_words"]
        }))
        .with_idempotent(false)
        .with_timeout(Duration::from_secs(900))
    }

    async fn execute(&self, _ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let req = match serde_json::from_value::<crate::cinema::GenerateSheetComfyReq>(
            request.arguments.clone(),
        ) {
            Ok(req) => req,
            Err(err) => return invalid_params(&request, format!("角色资产参数无效：{err}")),
        };
        let project_id = req.project_id.clone();
        let character_id = req.character_id.clone();
        let started = Instant::now();
        match crate::cinema::cinema_generate_character_sheet_via_comfyui_with_deps(
            self.app_handle.as_ref(),
            &self.config_dir,
            self.http.clone(),
            req,
        )
        .await
        {
            Ok(sheet) => {
                let asset_dir = character_asset_dir(&self.config_dir, &project_id, &character_id);
                let counts = character_sheet_counts(&sheet);
                let payload = json!({
                    "project_id": project_id,
                    "character_id": character_id,
                    "asset_dir": asset_dir.to_string_lossy(),
                    "counts": counts,
                    "warnings": sheet.warnings,
                });
                let artifacts = asset_dir
                    .is_dir()
                    .then(|| asset_dir.to_string_lossy().to_string())
                    .into_iter()
                    .collect();
                ToolOutcome::ok(
                    &request,
                    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string()),
                    artifacts,
                    started.elapsed(),
                )
            }
            Err(err) => ToolOutcome::failed(
                &request,
                cinema_tool_failure(err.to_string()),
                started.elapsed(),
            ),
        }
    }
}

#[async_trait]
impl ToolHandler for CinemaSliceCharacterSheetTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "cinema_slice_character_sheet",
            "Import a composite character sprite sheet and slice into turnaround views, expressions, and pose slots.",
            ToolKind::Cinema,
            Operation::Write,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "project_id": { "type": "string" },
                "character_id": { "type": "string" },
                "sheet_image_data": { "type": "string" }
            },
            "required": ["project_id", "character_id", "sheet_image_data"]
        }))
        .with_idempotent(false)
        .with_timeout(Duration::from_secs(60))
    }

    async fn execute(&self, _ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let req =
            match serde_json::from_value::<crate::cinema::SliceSheetReq>(request.arguments.clone())
            {
                Ok(req) => req,
                Err(err) => {
                    return invalid_params(&request, format!("素材大图切割参数无效：{err}"))
                }
            };
        let project_id = req.project_id.clone();
        let character_id = req.character_id.clone();
        let started = Instant::now();
        match crate::cinema::cinema_slice_character_sheet_with_config_dir(&self.config_dir, req)
            .await
        {
            Ok(sheet) => {
                let asset_dir = character_asset_dir(&self.config_dir, &project_id, &character_id);
                let payload = json!({
                    "project_id": project_id,
                    "character_id": character_id,
                    "asset_dir": asset_dir.to_string_lossy(),
                    "counts": character_sheet_counts(&sheet),
                    "warnings": sheet.warnings,
                });
                let artifacts = asset_dir
                    .is_dir()
                    .then(|| asset_dir.to_string_lossy().to_string())
                    .into_iter()
                    .collect();
                ToolOutcome::ok(
                    &request,
                    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string()),
                    artifacts,
                    started.elapsed(),
                )
            }
            Err(err) => ToolOutcome::failed(
                &request,
                cinema_tool_failure(err.to_string()),
                started.elapsed(),
            ),
        }
    }
}

#[async_trait]
impl ToolHandler for CinemaUpdateCharacterAssetSlotTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "cinema_update_character_asset_slot",
            "Upload or overwrite a single character turnaround view, expression, or pose asset slot.",
            ToolKind::Cinema,
            Operation::Write,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "project_id": { "type": "string" },
                "character_id": { "type": "string" },
                "category": { "type": "string" },
                "key": { "type": "string" },
                "image_data": { "type": "string" }
            },
            "required": ["project_id", "character_id", "category", "key", "image_data"]
        }))
        .with_idempotent(false)
        .with_timeout(Duration::from_secs(30))
    }

    async fn execute(&self, _ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let req = match serde_json::from_value::<crate::cinema::UpdateAssetSlotReq>(
            request.arguments.clone(),
        ) {
            Ok(req) => req,
            Err(err) => return invalid_params(&request, format!("资产槽位更新参数无效：{err}")),
        };
        let project_id = req.project_id.clone();
        let character_id = req.character_id.clone();
        let started = Instant::now();
        match crate::cinema::cinema_update_character_asset_slot_with_config_dir(
            &self.config_dir,
            req,
        )
        .await
        {
            Ok(sheet) => {
                let asset_dir = character_asset_dir(&self.config_dir, &project_id, &character_id);
                let payload = json!({
                    "project_id": project_id,
                    "character_id": character_id,
                    "asset_dir": asset_dir.to_string_lossy(),
                    "counts": character_sheet_counts(&sheet),
                    "warnings": sheet.warnings,
                });
                let artifacts = asset_dir
                    .is_dir()
                    .then(|| asset_dir.to_string_lossy().to_string())
                    .into_iter()
                    .collect();
                ToolOutcome::ok(
                    &request,
                    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string()),
                    artifacts,
                    started.elapsed(),
                )
            }
            Err(err) => ToolOutcome::failed(
                &request,
                cinema_tool_failure(err.to_string()),
                started.elapsed(),
            ),
        }
    }
}

fn character_asset_dir(
    config_dir: &std::path::Path,
    project_id: &str,
    character_id: &str,
) -> PathBuf {
    crate::cinema::cinema_projects_dir(config_dir)
        .join(project_id)
        .join("characters")
        .join(character_id)
        .join("angles")
}

fn character_sheet_counts(sheet: &crate::cinema::CharacterAssetSheet) -> serde_json::Value {
    json!({
        "turnarounds": sheet.turnarounds.len(),
        "expressions": sheet.expressions.len(),
        "poses": sheet.poses.len(),
        "angles": sheet.angles.len(),
    })
}

fn string_arg<'a>(request: &'a ToolCallRequest, key: &str) -> Option<&'a str> {
    request.arguments.get(key).and_then(|v| v.as_str())
}

fn cinema_tool_failure(message: String) -> ToolFailure {
    let lower = message.to_lowercase();
    if lower.contains("timeout") || message.contains("超时") {
        ToolFailure::new(ToolFailureKind::Timeout, message)
            .with_hint("检查 ComfyUI 队列是否堵塞，或降低步数/分辨率后重试。")
    } else if lower.contains("connection refused")
        || lower.contains("connection reset")
        || lower.contains("failed to connect")
        || message.contains("连接")
    {
        ToolFailure::new(ToolFailureKind::Transport, message)
            .with_hint("确认 ComfyUI 或本地服务已启动，端口和 endpoint 配置正确。")
    } else if lower.contains("missing")
        || lower.contains("not found")
        || lower.contains("no such file")
        || lower.contains("idencoder")
        || message.contains("缺少")
        || message.contains("没找到")
        || message.contains("找不到")
        || message.contains("不存在")
        || message.contains("权重")
        || message.contains("模型")
    {
        ToolFailure::new(ToolFailureKind::MissingDependency, message).with_hint(
            "检查对应 checkpoint、LoRA、ControlNet、IPAdapter/PuLID 权重和工作流节点是否安装。",
        )
    } else if lower.contains("comfyui")
        || lower.contains("prompt")
        || lower.contains("workflow")
        || message.contains("工作流")
    {
        ToolFailure::new(ToolFailureKind::Upstream, message)
            .with_hint("检查 ComfyUI 工作流节点、输入字段和模型族是否匹配。")
    } else {
        ToolFailure::new(ToolFailureKind::Execution, message)
    }
}

fn invalid_params(request: &ToolCallRequest, message: impl Into<String>) -> ToolOutcome {
    ToolOutcome::failed(
        request,
        ToolFailure::new(ToolFailureKind::InvalidParams, message),
        Duration::from_millis(0),
    )
}
