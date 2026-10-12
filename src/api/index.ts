// Tauri command 的类型化封装。UI 层只用这里的函数，不直接调 invoke，
// 这样 command 名字和参数形状只在一个地方写死。

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import {
  type RuntimeApprovalRequest,
  STREAM_EVENT,
  type ChatHandle,
  type CompressResult,
  type ConfigView,
  type Message,
  type StreamEvent,
  type ThemeMode,
  type ThinkingEffort,
  type Tier,
} from "./types";

export * from "./types";

export const RUNTIME_APPROVAL_EVENT = "runtime://approval_required";

export interface StartChatArgs {
  messages: Message[];
  tier?: Tier;
  workspacePath?: string | null;
  /** 当前会话 id。任务台账按它分键 */
  sessionId?: string | null;
  temperature?: number;
  maxTokens?: number;
}

export function startChat(args: StartChatArgs): Promise<ChatHandle> {
  return invoke("start_chat", {
    args: {
      messages: args.messages,
      tier: args.tier ?? null,
      workspace_path: args.workspacePath ?? null,
      session_id: args.sessionId ?? null,
      temperature: args.temperature ?? null,
      max_tokens: args.maxTokens ?? null,
    },
  });
}

export interface SavedChatImageAttachment {
  path: string;
  file_url: string;
}

export function saveChatImageAttachment(args: {
  dataUrl: string;
  filename: string;
  workspacePath?: string | null;
}): Promise<SavedChatImageAttachment> {
  return invoke("save_chat_image_attachment", {
    args: {
      data_url: args.dataUrl,
      filename: args.filename,
      workspace_path: args.workspacePath ?? null,
    },
  });
}

export function saveGenerationLog(outputPath: string, mode: string, details: unknown): Promise<string> {
  return invoke("save_generation_log", { args: { output_path: outputPath, mode, details: JSON.stringify(details, null, 2) } });
}

export interface DroppedAttachmentInfo {
  name: string;
  size: number;
  is_image: boolean;
  data_url?: string;
  local_path?: string;
  file_url?: string;
  text_content?: string;
  extension: string;
}

export function loadDroppedAttachment(args: {
  path: string;
  workspacePath?: string | null;
}): Promise<DroppedAttachmentInfo> {
  return invoke("load_dropped_attachment", {
    args: {
      path: args.path,
      workspace_path: args.workspacePath ?? null,
    },
  });
}

/** 任务台账的一步。**没有 status** —— 进度由系统记录，不是模型自报的 */
export interface TaskLedger {
  goal: string;
  steps: string[];
  plan_revision: number;
  goal_revisions: string[];
  /** path -> "read" | "modified" */
  touched: Record<string, "read" | "modified">;
  runs: { cmd: string; ok: boolean; round: number }[];
  last_failure: { tool: string; target: string; excerpt: string; round: number } | null;
  rounds: number;
  updated_at: number;
  archived: boolean;
}

export function taskLedgerGet(
  sessionId?: string | null,
  workspacePath?: string | null,
): Promise<TaskLedger | null> {
  return invoke("task_ledger_get", {
    sessionId: sessionId ?? null,
    workspacePath: workspacePath ?? null,
  });
}

export function taskLedgerArchive(
  sessionId?: string | null,
  workspacePath?: string | null,
): Promise<void> {
  return invoke("task_ledger_archive", {
    sessionId: sessionId ?? null,
    workspacePath: workspacePath ?? null,
  });
}

/** 取消一轮对话。语音打断（barge-in）接入后由 VAD 调用。 */
export function cancelChat(streamId: string): Promise<boolean> {
  return invoke("cancel_chat", { streamId });
}

/** 压缩被裁掉的历史消息，返回累积摘要。与语音侧共用同一套 summary 引擎。 */
export function compressHistory(
  previousSummary: string,
  previousFocus: string[],
  droppedMessages: { role: string; content: string }[],
): Promise<CompressResult> {
  return invoke("compress_history", {
    previousSummary,
    previousFocus,
    droppedMessages,
  });
}

export function getConfig(): Promise<ConfigView> {
  return invoke("get_config");
}

export function updateProvider(args: {
  id: string;
  baseUrl?: string;
  model?: string;
  supportsTools?: boolean;
  /** 仅 Azure；传空字符串表示回退到适配器默认版本 */
  apiVersion?: string;
}): Promise<ConfigView> {
  return invoke("update_provider", {
    args: {
      id: args.id,
      base_url: args.baseUrl ?? null,
      model: args.model ?? null,
      supports_tools: args.supportsTools ?? null,
      api_version: args.apiVersion ?? null,
    },
  });
}

export function setTierProvider(
  tier: Tier,
  providerId: string,
): Promise<ConfigView> {
  return invoke("set_tier_provider", { tier, providerId });
}

/** 传空字符串即删除该后端的 key */
export function setApiKey(
  providerId: string,
  key: string,
): Promise<ConfigView> {
  return invoke("set_api_key", { providerId, key });
}

/** 探活并返回该后端可用的模型列表 */
export function testProvider(providerId: string): Promise<string[]> {
  return invoke("test_provider", { providerId });
}

/**
 * 这个后端有哪些模型能吃图。
 *
 * **`null` 表示查不到，不是"没有视觉模型"** —— 只有 OpenAI 兼容那条路能查
 * （走 LM Studio 的 `/api/v0/models` 读 `type: "vlm"`）。拿到 null 时界面应当
 * 什么都不说，而不是谎报"不支持视觉"。
 *
 * 不能靠模型名推断：实测同一个基座 `qwen/qwen3.8-27b` 是 vlm、
 * `qwen3.8-27b-uncensored` 是 llm，差别只在打包时有没有带 mmproj。
 */
export function providerVisionModels(providerId: string): Promise<string[] | null> {
  return invoke("provider_vision_models", { providerId });
}

/**
 * 这个后端**当前加载的**模型有多大上下文窗口（token）。
 *
 * `null` = 查不到（这个后端没有这个接口）—— 不是"窗口是 0"。
 * 拿到 null 时要回落到默认预算，并且在界面上说明是默认值。
 */
export function providerContextWindow(providerId: string): Promise<number | null> {
  return invoke("provider_context_window", { providerId });
}

/**
 * 设置某条链路的思考档位。
 *
 * 按链路而不是按模型：同一个模型可以同时服务实时档和后台档，
 * 按模型记的话，在后台档开了思考、切回实时档会跟着开。
 */
export function setThinkingEffort(
  tier: Tier,
  effort: ThinkingEffort,
): Promise<ConfigView> {
  return invoke("set_thinking_effort", { tier, effort });
}

/**
 * 订阅流式事件，由调用方按 `stream_id` 自行分发。
 *
 * 用 event 而不是 command 返回值传 token，是为了避免高频 IPC 开销
 * （见 ARCHITECTURE.md 坑点 1）。
 *
 * 之所以是"订阅全部再自行分发"而不是"按 stream_id 订阅"：`startChat` 要先
 * 返回才知道 stream_id，而 `listen` 本身是异步注册的 —— 等拿到 id 再订阅，
 * 中间那个窗口的 token 就漏了。所以在应用启动时挂一个长期监听。
 */
export async function subscribeStreams(
  onEvent: (event: StreamEvent) => void,
): Promise<() => void> {
  return listen<StreamEvent>(STREAM_EVENT, ({ payload }) => onEvent(payload));
}

/**
 * 指定工具调用的备用后端。传 `null` 恢复自动挑。
 *
 * 自动规则是"任何配了密钥的云端"，而用户可能有好几个云端、只想让其中某一个
 * 跑工具（比如便宜的那个）。
 */
export async function setToolFallback(providerId: string | null): Promise<ConfigView> {
  return invoke("set_tool_fallback", { providerId });
}

/**
 * 设置某条链路用哪个模型。传空串 = 清掉，沿用该后端的默认模型。
 *
 * 模型属于链路而不是后端 —— 否则两条链路指向同一个后端时就只能共用一个模型。
 */
export async function setRouteModel(
  route: "realtime" | "background" | "tool_fallback",
  model: string,
): Promise<ConfigView> {
  return invoke("set_route_model", { route, model });
}

/** 设置系统启动时默认展示的视图 ("avatar" 虚拟人 / "chat" 对话窗) */
/** 启动时默认开哪个窗口。**取值必须和 Rust 侧 `normalize_startup_window` 对得上** ——
 *  对不上的话会被静默归一化成 avatar，表现是「选了但没生效」。 */
/**
 * 切换界面主题。**主界面和 Studio 一起翻** ——
 * Studio 的 token 引用的是 `theme.css` 里 `:root` 上的系统色，
 * 只翻一半会得到「深色 token + 浅色系统色」。
 */
export async function setThemeMode(mode: ThemeMode): Promise<ConfigView> {
  return invoke("set_theme_mode", { mode });
}

export async function setStartupWindow(
  window: "avatar" | "chat" | "anima_studio",
): Promise<ConfigView> {
  return invoke("set_startup_window", { window });
}

/** ComfyUI 认得的 SDXL checkpoint 列表。问 ComfyUI 而不是扫目录。 */
export async function listSdxlCheckpoints(): Promise<string[]> {
  return invoke("list_sdxl_checkpoints");
}

/** ComfyUI 认得的 LoRA 列表。 */
export async function listLoras(): Promise<string[]> {
  return invoke("list_loras");
}

/**
 * 设定卡专用 LoRA。传空数组 = 清空。
 *
 * **只影响角色设定卡那条路。** 刻意不做全局常挂 —— LoRA 只在 SDXL 架构上
 * 有效，而默认引擎是 Z-Image，全局设置会在那条路上静默失效。
 */
export async function setSheetLoras(
  loras: { name: string; strength: number }[],
): Promise<ConfigView> {
  return invoke("set_sheet_loras", { loras });
}

/** Z-Image 那条路常挂的 LoRA。和设定卡那批**不是同一批权重**，架构不同。 */
export async function setZimageLoras(
  loras: { name: string; strength: number }[],
): Promise<ConfigView> {
  return invoke("set_zimage_loras", { loras });
}

/** SDXL 那条路用哪个 checkpoint。空字符串 = 恢复自动挑选。 */
export async function setSdxlCheckpoint(checkpoint: string): Promise<ConfigView> {
  return invoke("set_sdxl_checkpoint", { checkpoint });
}

/** H3 参考生视频 / 长镜头用原生 Ref2VA（true）还是兼容模式（false，默认）。下一次提交出片生效 */
export async function setH3NativeRef2va(enabled: boolean): Promise<ConfigView> {
  return invoke("set_h3_native_ref2va", { enabled });
}

/** 出图默认底模。实测耗时 zimage 45s / sdxl 39s / krea2 108s / qwen21 211s。（chroma 2026-10-01 下架） */
export async function setImageEngine(
  engine: "zimage" | "sdxl" | "krea2" | "qwen21",
): Promise<ConfigView> {
  return invoke("set_image_engine", { engine });
}

// --- Projects & Sessions ---
export function getProjects(): Promise<import("./types").Project[]> {
  return invoke("get_projects");
}

/**
 * 新建项目，返回**后端生成的 id**。
 *
 * 后端签名是 `create_project(name) -> String`：id 和 created_at 由它自己产生，
 * 所以这里传的 `id` / `created_at` 会被忽略 —— 调用方必须用返回值。
 *
 * `description` / `system_prompt` 目前也**没有落库**（命令里写死了 None），
 * 「每个项目独立人设」那条还没接上。留着这两个参数是为了接上时不用改调用方。
 */
export function createProject(args: {
  id?: string;
  name: string;
  description?: string;
  system_prompt?: string;
  character_id?: string;
  workspace_path?: string;
  created_at?: number;
}): Promise<string> {
  return invoke("create_project", {
    name: args.name,
    workspacePath: args.workspace_path ?? null,
  });
}

export function updateProject(args: {
  id: string;
  name: string;
  description?: string;
  system_prompt?: string;
  character_id?: string;
  workspace_path?: string;
}): Promise<void> {
  return invoke("update_project", {
    id: args.id,
    name: args.name,
    description: args.description ?? null,
    systemPrompt: args.system_prompt ?? null,
    characterId: args.character_id ?? null,
    workspacePath: args.workspace_path ?? null,
  });
}

export function deleteProject(id: string): Promise<void> {
  return invoke("delete_project", { id });
}

// --- Workspace Directory & Scaffolding ---
export function workspaceChooseFolder(): Promise<string | null> {
  return invoke("workspace_choose_folder");
}

export function workspaceCheckStatus(path: string): Promise<import("./types").WorkspaceStatus> {
  return invoke("workspace_check_status", { path });
}

export function workspaceInit(req: import("./types").WorkspaceInitReq): Promise<import("./types").WorkspaceStatus> {
  return invoke("workspace_init", { req });
}

export function workspaceSaveFiles(args: {
  path: string;
  promptContent?: string | null;
  rulesContent?: string | null;
}): Promise<void> {
  return invoke("workspace_save_files", {
    path: args.path,
    promptContent: args.promptContent ?? null,
    rulesContent: args.rulesContent ?? null,
  });
}

export function workspaceWriteFile(args: {
  workspacePath: string;
  relativePath: string;
  content: string;
}): Promise<void> {
  return invoke("workspace_write_file", {
    workspacePath: args.workspacePath,
    relativePath: args.relativePath,
    content: args.content,
  });
}

export interface GitCommitResult {
  commit_hash: string;
  branch: string;
  message: string;
  files_committed: string[];
}

export function workspaceCommit(args: {
  workspacePath: string;
  message: string;
  files?: string[];
}): Promise<GitCommitResult> {
  return invoke("workspace_commit_file", {
    workspacePath: args.workspacePath,
    message: args.message,
    files: args.files ?? null,
  });
}

export function workspaceOpenInFinder(path: string): Promise<void> {
  return invoke("workspace_open_in_finder", { path });
}

export function workspaceListKnowledgeDocs(path: string): Promise<import("./types").KnowledgeDocInfo[]> {
  return invoke("workspace_list_knowledge_docs", { path });
}

export function workspaceDeleteKnowledgeDoc(path: string, filename: string): Promise<void> {
  return invoke("workspace_delete_knowledge_doc", { path, filename });
}

export function workspaceGetPermissions(path: string): Promise<import("./types").ProjectPermissions> {
  return invoke("workspace_get_permissions", { path });
}

export function workspaceSavePermissions(path: string, permissions: import("./types").ProjectPermissions): Promise<void> {
  return invoke("workspace_save_permissions", { path, permissions });
}

export function workspaceReindexMemory(path: string): Promise<number> {
  return invoke("workspace_reindex_memory", { path });
}

/** 某个项目已索引多少块。**来自向量库本身**，不是"数了几个文件"。 */
export function workspaceKnowledgeCount(path: string): Promise<number> {
  return invoke("workspace_knowledge_count", { path });
}

export function workspaceReadFile(
  filePath: string,
  workspacePath?: string | null,
): Promise<import("./types").WorkspaceFileResult> {
  return invoke("workspace_read_file", {
    filePath,
    workspacePath: workspacePath ?? null,
  });
}

export function runtimeListPendingApprovals(): Promise<import("./types").RuntimeApprovalRequest[]> {
  return invoke("runtime_list_pending_approvals");
}

export function runtimeResolveApproval(
  id: string,
  decision: import("./types").RuntimeApprovalDecision,
  scope?: import("./types").RuntimeApprovalScope,
  reason?: string,
): Promise<boolean> {
  return invoke("runtime_resolve_approval", {
    args: { id, decision, scope: scope ?? null, reason: reason ?? null },
  });
}

export function onRuntimeApproval(handler: (request: RuntimeApprovalRequest) => void) {
  return listen<RuntimeApprovalRequest>(RUNTIME_APPROVAL_EVENT, (event) =>
    handler(event.payload),
  );
}

export function runtimeCreateJob(
  args: import("./types").RuntimeCreateJobArgs,
): Promise<import("./types").RuntimeJob> {
  return invoke("runtime_create_job", { args });
}

export function runtimeListJobs(): Promise<import("./types").RuntimeJob[]> {
  return invoke("runtime_list_jobs");
}

export function runtimeStartJobRunner(
  args: import("./types").RuntimeRunJobArgs,
): Promise<import("./types").RuntimeJob> {
  return invoke("runtime_start_job_runner", {
    args: {
      job_id: args.job_id,
      workspace_path: args.workspace_path ?? null,
      network_enabled: args.network_enabled ?? false,
      approval_mode: args.approval_mode ?? "wait",
    },
  });
}

export function runtimeCancelJob(
  jobId: string,
  reason?: string | null,
): Promise<import("./types").RuntimeJob> {
  return invoke("runtime_cancel_job", { jobId, reason: reason ?? null });
}

export function runtimePauseJob(
  jobId: string,
  reason?: string | null,
): Promise<import("./types").RuntimeJob> {
  return invoke("runtime_pause_job", { jobId, reason: reason ?? null });
}

export function runtimeResumeJob(jobId: string): Promise<import("./types").RuntimeJob> {
  return invoke("runtime_resume_job", { jobId });
}

export function runtimeExportJobLog(jobId: string): Promise<string> {
  return invoke("runtime_export_job_log", { jobId });
}

/** 全部会话，不分项目。`getSessions(null)` 拿的是「没挂项目」的那些，不是全部 */
export function getAllSessions(includeArchived = false): Promise<import("./types").SessionInfo[]> {
  return invoke("get_all_sessions", { includeArchived });
}

export function createSession(projectId: string | null = null): Promise<string> {
  return invoke("create_session", { projectId });
}

export function getSessionMessages(sessionId: string): Promise<import("./types").StoredMessage[]> {
  return invoke("get_session_messages", { sessionId });
}

export function appendSessionMessage(
  sessionId: string,
  role: "user" | "assistant" | "system",
  content: string,
  /** 这一轮的工具调用与结果，JSON。不传就是这一轮没工具 */
  toolJson?: string,
): Promise<number> {
  return invoke("append_session_message", { sessionId, role, content, toolJson });
}

export function deleteSessionMessage(
  sessionId: string,
  messageId: number,
): Promise<void> {
  return invoke("delete_session_message", { sessionId, messageId });
}

export function rollbackSessionToMessage(
  sessionId: string,
  messageId: number,
): Promise<void> {
  return invoke("rollback_session_to_message", { sessionId, messageId });
}

export function updateSessionTitle(id: string, title: string): Promise<void> {
  return invoke("update_session_title", { id, title });
}

export function archiveSession(id: string): Promise<void> {
  return invoke("archive_session", { id });
}

export function deleteSession(id: string): Promise<void> {
  return invoke("delete_session", { id });
}

export function workspaceListDirectoryTree(
  workspacePath?: string | null,
  subPath?: string | null,
  maxDepth?: number | null,
): Promise<import("./types").FileTreeNode[]> {
  return invoke("workspace_list_directory_tree", {
    workspacePath: workspacePath ?? null,
    subPath: subPath ?? null,
    maxDepth: maxDepth ?? null,
  });
}

export function workspaceGitStatus(
  workspacePath?: string | null,
): Promise<import("./types").WorkspaceGitStatusResult> {
  return invoke("workspace_git_status", {
    workspacePath: workspacePath ?? null,
  });
}
