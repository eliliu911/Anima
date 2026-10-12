//! Codex-style agent runtime primitives.
//!
//! This module is the shared contract for Anima's next execution layer.  The
//! existing chat loop, MCP registry, ComfyUI jobs, and cinema pipeline can move
//! behind these types incrementally instead of each inventing its own timeout,
//! approval, retry, and logging semantics.

pub mod approval;
pub mod cinema_provider;
pub mod code_outline;
pub mod fetch_url;
pub mod entity_sheet;
pub mod panorama;
pub mod scene_sheet;
pub mod shot_sheet;
pub mod studio_provider;
pub mod image_engine;
pub mod image_tool;
pub mod weetodd;
pub mod video_tool;
pub mod comfyui_provider;
pub mod context;
pub mod event;
pub mod job;
pub mod knowledge_tool;
pub mod local_tools;
pub mod lora;
pub mod lsp_client;
pub mod mcp_bridge;
pub mod policy;
pub mod pty_session;
pub mod registry;
pub mod tool_tier;
pub mod research;
pub mod service;
pub mod subagent;
pub mod tool;
pub mod user_activity;
pub mod worktree;

pub use approval::{ApprovalDecision, ApprovalRequest, ApprovalScope};
pub use cinema_provider::{
    register_cinema_audio_tools, register_cinema_http_tools, register_cinema_tools,
    CinemaExportJianyingTool, CinemaGenerateAiSequelTool, 
    CinemaGenerateCharacterSheetTool, CinemaGenerateShotAudioTool, CinemaRenderShotTool,
    CinemaRenderShotVideoTool, CinemaSliceCharacterSheetTool, CinemaStitchEpisodeTool,
    CinemaUpdateCharacterAssetSlotTool,
};
pub use code_outline::CodeOutlineTool;
pub use comfyui_provider::{
    register_comfyui_tools, ComfyUiRunningArgsTool, ComfyUiStartTool, ComfyUiStatusTool,
};
pub use context::{ModelTier, RuntimeContext, RuntimeIds};
pub use event::{EventLog, RuntimeEvent};
pub use job::{Artifact, Job, JobKind, JobStatus, JobStep, JobStepAction, StepStatus};
pub mod comfy_jobs;
pub mod comfy_watch;
pub use knowledge_tool::{MemoryCell, SearchKnowledgeTool};
pub use local_tools::{
    register_local_tools, ListDirectoryTool, ReadTextFileTool, ShellCommandTool, WriteTextFileTool,
};
pub use mcp_bridge::{
    runtime_mcp_prompts, runtime_mcp_resources, runtime_mcp_specs, runtime_mcp_tool_infos,
    McpRuntimeTool, RuntimeMcpToolInfo,
};
pub use policy::{FileCapability, NetworkPolicy, PathVerdict, RuntimePolicy, SandboxMode};
pub use registry::{web_search_spec, StaticToolHandler, ToolHandler, ToolRegistry, ToolRouter};
pub use research::{EvidenceBundle, EvidenceItem, ResearchQuery, ResearchTrigger};
pub use service::{
    RuntimeApprovalMode, RuntimeCreateCinemaJobArgs, RuntimeCreateJobArgs,
    RuntimeResolveApprovalArgs, RuntimeRunJobArgs, RuntimeService, RuntimeSetJobStatusArgs,
    RuntimeSetJobStepActionArgs, RuntimeToolCallArgs, RuntimeToolCallResult,
    RuntimeUpdateJobStepArgs, ToolProfile};
pub use tool::{
    RetryPolicy, ToolCallRequest, ToolFailure, ToolFailureKind, ToolKind, ToolOutcome, ToolSpec,
    ToolStatus,
};
