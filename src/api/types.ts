// 与 src-tauri/src 里的 Rust 类型一一对应。
// 改动任何一边都要同步另一边 —— serde 的 tag/rename 决定了这里的字面量。

export type Role = "system" | "user" | "assistant" | "tool";

/**
 * 思考档位，顺序即「从省到费」。
 *
 * `auto` 不是"最费"，而是"不干预、用模型默认" —— UI 上应当和滑杆分开渲染。
 */
export type ThinkingEffort = "off" | "low" | "medium" | "high" | "auto";

export interface ThinkingPolicy {
  realtime: ThinkingEffort;
  background: ThinkingEffort;
}

export interface ToolCall {
  id: string;
  name: string;
  /** 原始 JSON 字符串，可能需要自行 parse */
  arguments: string;
}

export interface Message {
  role: Role;
  content: string;
  images?: string[];
  tool_calls?: ToolCall[];
  tool_call_id?: string;
}

/** 对应 Rust 的 Tier —— 实时链路 vs 后台任务 */
export type Tier = "realtime" | "background";

export type ProviderKind =
  | "open_ai"
  | "anthropic"
  | "open_ai_compatible"
  | "ollama"
  | "azure_open_ai";

export interface ProviderStatus {
  id: string;
  label: string;
  kind: ProviderKind;
  base_url: string;
  model: string;
  supports_tools: boolean;
  requires_key: boolean;
  /** 仅 Azure：对话接口的 api-version，留空则用适配器默认值 */
  api_version?: string;
  /** 后端只告诉我们"配了没有"，永远不回传 key 本身 */
  has_key: boolean;
  /** key 由 .env.local 的环境变量注入，界面里改不掉 */
  key_from_env: boolean;
  /**
   * 这条 key 存在哪儿。`null` = 没配。
   *
   * `env` 在界面里改不掉；`file` 是应用数据目录的 `secrets.env`（权限 600，
   * 现在写入的默认去处）；`keychain` 是老数据，重新保存一次就迁过去。
   */
  key_source?: "env" | "file" | "keychain" | null;
  /** 模型能力与预算分级："local" | "cloud"（由后端统一判定） */
  model_tier?: "local" | "cloud";
  /**
   * 当前选中的模型**实际支持**哪些思考档位，按从省到费排列。
   *
   * 由后端 `llm::thinking::supported_levels` 判定 —— 前端不要自己猜。
   * 空数组 = 这个模型不支持任何干预，**不要渲染控件**：
   * 一个点了没反应的滑杆比没有滑杆更糟。
   */
  thinking_levels?: ThinkingEffort[];
}

/** 界面主题。**取值必须和 Rust 侧 `normalize_theme_mode` 对得上。** */
export type ThemeMode = "system" | "light" | "dark";

export interface ConfigView {
  providers: ProviderStatus[];
  realtime_provider: string;
  background_provider: string;
  /** 工具调用的备用后端。null = 自动挑一个配了密钥的云端 */
  tool_fallback_provider: string | null;
  /** 每条链路各自的模型。null = 沿用该后端的默认模型 */
  realtime_model: string | null;
  background_model: string | null;
  tool_fallback_model: string | null;
  /** 启动时默认显示的窗口："avatar" | "chat" */
  startup_window?: string;
  /** 界面主题：跟随系统 / 强制浅色 / 强制深色 */
  theme_mode?: ThemeMode;
  /**
   * 出图默认底模：`qwen21` / `zimage` / `sdxl` / `krea2`（`chroma` 2026-10-01 下架）。
   *
   * 本机实测稳态耗时 211s / 45s / 39s / 108s。
   * 默认 `zimage`：质量远高于 SDXL，代价只多 15%。
   */
  image_engine?: string;
  language?: string | null;
  /**
   * SDXL 那条路用哪个 checkpoint。空 = 按内置优先级自动挑。
   *
   * 做成可选是因为默认值本身有倾向：内置优先级把 `RealVisXL` 排第一，
   * 那是欧美写实底模；做国风角色设定卡时 `AWPortrait_XL` 明显更合适。
   */
  sdxl_checkpoint?: string | null;
  /** H3 参考生视频用原生 Ref2VA 权重吗。默认 false = 兼容模式（2026-10-06 用户定） */
  h3_native_ref2va?: boolean;
  /** 本机装没装原生 Ref2VA 权重。没装时上面那个开关不起作用 */
  h3_ref2va_installed?: boolean;
  /** 设定卡专用 LoRA。只在 `layout: character_sheet` 时生效 */
  sheet_loras?: { name: string; strength: number }[];
  zimage_loras?: { name: string; strength: number }[];
  /** @deprecated 用 `thinking` 代替。保留只为兼容还没改完的调用点。 */
  disable_thinking: boolean;
  /** 每条链路各自的思考档位 */
  thinking: ThinkingPolicy;
}

export interface ChatHandle {
  stream_id: string;
  /** 本轮实际使用的后端，可能因工具调用降级而与设置里选的不同 */
  provider_id: string;
  model: string;
}

/**
 * `tool_rounds_exhausted` 不是"模型说完了"，是**我们不让它继续了** ——
 * 工具往返撞到上限（本地 12 轮 / 云端 25 轮），Anima 摘掉工具让它收场。
 * 这时内容往往是半截的，必须在界面上说清楚，否则用户分不清是模型的问题
 * 还是我们截断了它。
 */
export type FinishReason =
  | "stop"
  | "length"
  | "tool_calls"
  | "cancelled"
  | "tool_rounds_exhausted"
  /** 被服务商的内容过滤掐断（OpenAI / Azure `content_filter`），内容是半截的 */
  | "content_filter";

export interface Usage {
  prompt_tokens: number;
  completion_tokens: number;
  /**
   * 输出 token 里有多少花在思考上。
   *
   * **只有部分后端给** —— 实测 LM Studio 在
   * `completion_tokens_details.reasoning_tokens` 里给，Anthropic 和 Ollama
   * 原生都不给。拿不到时不显示，而不是显示 0 让人误以为没思考。
   */
  reasoning_tokens?: number;
}

/**
 * 流式事件。Rust 侧 ChatChunk 用 `#[serde(tag = "kind")]`，
 * 加上 StreamEvent 的 `#[serde(flatten)]`，最终线上格式就是扁平的
 * `{ stream_id, kind, ... }`。error 事件由 StreamErrorEvent 单独发出。
 */
export type StreamEvent =
  | { stream_id: string; kind: "text"; delta: string }
  | { stream_id: string; kind: "reasoning"; delta: string }
  | { stream_id: string; kind: "tool_call"; call: ToolCall }
  | {
      stream_id: string;
      kind: "tool_result";
      call_id: string;
      name: string;
      content: string;
      ok?: boolean;
    }
  | {
      stream_id: string;
      kind: "done";
      reason: FinishReason;
      usage: Usage | null;
    }
  | { stream_id: string; kind: "error"; message: string };

/** Rust 侧的事件名常量，改动需同步 commands.rs 的 STREAM_EVENT */
export const STREAM_EVENT = "anima://llm-stream";

/** compress_history 命令的返回值 */
export interface CompressResult {
  summary: string;
  focus: string[];
  /** 这批消息是否真的被压进摘要。为 false 时不能推进 compactedThrough，否则内容会被永久跳过 */
  compressed: boolean;
}

export interface Project {
  id: string;
  name: string;
  description: string | null;
  system_prompt: string | null;
  character_id?: string | null;
  workspace_path?: string | null;
  created_at: number;
}

export interface WorkspaceStatus {
  path: string;
  is_valid_dir: boolean;
  is_initialized: boolean;
  has_prompt: boolean;
  has_rules: boolean;
  has_project_meta: boolean;
  knowledge_doc_count: number;
  prompt_content?: string | null;
  rules_content?: string | null;
  project_meta?: Record<string, any> | null;
  /** **这一次** init 新建了哪些文件（相对工作区）。check_status 那条路上恒为空 */
  created: string[];
}

export interface WorkspaceFileResult {
  resolved_path: string;
  relative_path: string;
  content: string;
}

export interface KnowledgeDocInfo {
  name: string;
  path: string;
  size_bytes: number;
  updated_at: number;
  extension: string;
}

export interface FileAccessPolicy {
  allow_reads: boolean;
  allow_writes: boolean;
  allowed_paths: string[];
  denied_paths: string[];
}

export interface TerminalCommandPolicy {
  auto_approve_commands: string[];
  blocked_commands: string[];
  sandbox_mode: string; // "standard" | "unsandboxed"
}

export interface ProjectPermissions {
  security_preset: string; // "inherit" | "auto_read" | "strict"
  file_access: FileAccessPolicy;
  terminal_policy: TerminalCommandPolicy;
}

export interface WorkspaceInitReq {
  path: string;
  project_name: string;
  template_type?: string;
}


export type RuntimeApprovalDecision = "allow" | "deny";
export type RuntimeApprovalScope = "once" | "session" | "workspace";

export interface RuntimeApprovalRequest {
  id: string;
  tool_name: string;
  operation: string;
  risk: string;
  reason: string;
  arguments_preview: string;
  command_preview?: string | null;
  diff_preview?: string | null;
  scope: RuntimeApprovalScope;
  /**
   * 点「本项目允许」会写进 `.anima/project.json` 白名单的那个前缀。
   *
   * `null` = 这条命令没法泛化（复合命令、子 shell、程序名带路径），
   * 那时「本项目允许」会自动降级成本次会话。
   *
   * **必须显示出来。** 用户授权的范围和他以为的范围不一致，是这类对话框
   * 最容易出的问题 —— 他点的是「这一条」，实际放行的可能是「所有 npm run *」。
   */
  grant_prefix?: string | null;
  /** 给人看的一句话（Studio 工具有）。有它就大字显示，参数退到下面 */
  summary?: string | null;
  /** 只能「允许一次」（出图、出片这类占显卡很久的工具）：弹窗不给本会话 / 本项目允许 */
  once_only?: boolean;
  created_at_ms: number;
}

export type RuntimeJobKind =
  | "script_planning"
  | "character_assets"
  | "storyboard"
  | "shot_video"
  | "audio_generation"
  | "episode_assembly"
  | "export_jianying"
  | "generic";

export type RuntimeJobStatus =
  | "pending"
  | "running"
  | "paused"
  | "waiting_approval"
  | "succeeded"
  | "failed"
  | "cancelled";

export type RuntimeStepStatus =
  | "pending"
  | "running"
  | "succeeded"
  | "failed"
  | "skipped";

export type RuntimeJobStepAction =
  | {
      type: "runtime_tool";
      tool_name: string;
      arguments?: Record<string, any>;
      network_enabled?: boolean;
    };

export type RuntimeArtifact =
  | { text: { label: string; content: string } }
  | { file: { label: string; path: string } }
  | { url: { label: string; url: string } }
  | { json: { label: string; value: any } };

export interface RuntimeJobStep {
  id: string;
  title: string;
  status: RuntimeStepStatus;
  attempts: number;
  retry_policy: { max_attempts: number; base_delay_ms: number };
  depends_on: string[];
  action?: RuntimeJobStepAction | null;
  message?: string | null;
  provenance: RuntimeArtifact[];
  artifacts: RuntimeArtifact[];
}

export interface RuntimeJob {
  id: string;
  kind: RuntimeJobKind;
  title: string;
  status: RuntimeJobStatus;
  steps: RuntimeJobStep[];
  artifacts: RuntimeArtifact[];
  provenance: RuntimeArtifact[];
  error?: string | null;
  created_at_ms: number;
  updated_at_ms: number;
}

export interface RuntimeCreateJobArgs {
  kind: RuntimeJobKind;
  title: string;
  steps: string[];
}

export interface RuntimeRunJobArgs {
  job_id: string;
  workspace_path?: string | null;
  network_enabled?: boolean;
  approval_mode?: "wait" | "deny";
}

export interface SessionInfo {
  id: string;
  started_at: number;
  ended_at: number | null;
  title: string | null;
  project_id: string | null;
  archived: boolean;
}

export interface StoredMessage {
  id: number;
  session_id: string;
  role: Role;
  content: string;
  created_at: number;
  /**
   * 这一轮的工具调用与结果（JSON 字符串）。**没有它，重启后生成的图就没了。**
   *
   * 图片卡是从工具结果渲染的，而这张表长期只存 `content`（模型说的话），
   * 还原时 `toolCalls` 被写死成 `[]` —— 于是重启回到对话里图片凭空消失。
   * 文件一直在磁盘上，丢的只是记录里对它的引用。
   */
  tool_json?: string | null;
}

export interface FileTreeNode {
  name: string;
  path: string;
  is_dir: boolean;
  size_bytes: number;
  extension?: string | null;
  children?: FileTreeNode[] | null;
  /**
   * 这个目录还有内容没走下去（撞到了后端的深度上限）。
   *
   * **`children: null` 和 `children: []` 在界面上长得一样**，
   * 所以要靠这一位区分「没取回来」和「真的空」。展开时按需再取一层，
   * 见 `components/fileTree.ts`。
   */
  truncated?: boolean;
}

export interface WorkspaceFileResult {
  resolved_path: string;
  relative_path: string;
  content: string;
  is_binary?: boolean;
  media_type?: "image" | "audio" | "video" | "binary" | null;
  data_url?: string | null;
}

export interface WorkspaceGitFileChange {
  path: string;
  status: "modified" | "added" | "deleted" | "untracked" | "renamed";
  additions: number;
  deletions: number;
  is_staged: boolean;
}

export interface WorkspaceGitStatusResult {
  is_git: boolean;
  branch: string;
  clean: boolean;
  changes: WorkspaceGitFileChange[];
}
