/**
 * MCP（工具接入）的 Tauri 绑定。
 *
 * 与 Rust 侧 `src/mcp/` 和 `src/agent.rs` 一一对应。
 * 权限相关的字段命名刻意跟 Rust 保持一致，避免两边理解漂移 ——
 * 这层是安全边界，含糊会出事。
 */
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export const TOOL_EVENT = "anima://tool-activity";
export const CONFIRM_EVENT = "anima://tool-confirm";

/** 操作类别，风险从低到高 */
export type Operation = "read" | "write" | "delete" | "execute";
export type Access = "deny" | "ask" | "allow";
export type Risk = "low" | "medium" | "high" | "critical";
/** 审批模式：保守（白名单外都问）/ 自动（读写免问） */
export type ApprovalMode = "conservative" | "auto";
/** 分类依据。heuristic = 关键词猜的，可能不准 */
export type DecisionReason = "tool_override" | "configured_class" | "heuristic";

export interface ServerPermissions {
  read: Access;
  write: Access;
  delete: Access;
  execute: Access;
  /** 按工具名纠正分类（启发式猜错时用） */
  classify: Record<string, Operation>;
  /** 按工具名直接指定许可，优先级最高 */
  overrides: Record<string, Access>;
}

export interface CommandPolicy {
  /** 免问的命令前缀（只对执行类工具生效） */
  allow_prefixes: string[];
  /** 命中即拒绝的子串 */
  deny_substrings: string[];
}

export type McpTransport =
  | {
      /** 判别字段是 `kind` 而不是 `type` —— Rust 侧是 #[serde(tag = "kind")]。
       *  写错的话整份 config.json 会解析失败（实际踩过），
       *  线格式钉在 Rust 的 transport_wire_format 测试里 */
      kind: "stdio";
      command: string;
      args: string[];
      env: Record<string, string>;
      /** 环境变量名 → 凭据 id。凭据本身不进配置文件 */
      env_from_secrets: Record<string, string>;
    }
  | {
      kind: "http";
      url: string;
      headers: Record<string, string>;
      headers_from_secrets: Record<string, string>;
    };

export interface McpServerConfig {
  id: string;
  label: string;
  enabled: boolean;
  transport: McpTransport;
  permissions: ServerPermissions;
  approval_mode: ApprovalMode;
  commands: CommandPolicy;
  startup_timeout_secs: number;
}

export interface ServerStatus {
  id: string;
  label: string;
  connected: boolean;
  tool_count: number;
  /** 连不上的原因。**一定要显示** —— 排查全靠这句 */
  error: string | null;
}

export interface ToolInfo {
  qualified_name: string;
  server_id: string;
  name: string;
  description: string;
  operation: Operation;
  access: Access;
  reason: DecisionReason;
}

export interface McpView {
  servers: McpServerConfig[];
  statuses: ServerStatus[];
  tools: ToolInfo[];
}

/** 工具执行的实时动态 */
export type ToolActivity =
  | { phase: "started"; name: string; arguments: string }
  | { phase: "running"; name: string; elapsed_secs: number; hint?: string }
  | { phase: "finished"; name: string; ok: boolean; preview: string }
  | { phase: "rejected"; name: string; reason: string }
  | { phase: "declined"; name: string };

export interface ToolActivityEvent extends Record<string, unknown> {
  stream_id: string;
}

export interface ToolConfirmEvent {
  stream_id: string;
  request_id: string;
  tool_name: string;
  /** 闸门给的说明，原样展示 */
  reason: string;
  risk: Risk;
  arguments: string;
}

export const mcpApi = {
  view: () => invoke<McpView>("mcp_view"),
  reconnect: () => invoke<ServerStatus[]>("mcp_reconnect"),
  upsertServer: (server: McpServerConfig) =>
    invoke<ServerStatus[]>("mcp_upsert_server", { args: { server } }),
  removeServer: (serverId: string) =>
    invoke<ServerStatus[]>("mcp_remove_server", { serverId }),
  setPermissions: (args: {
    server_id: string;
    permissions: ServerPermissions;
    approval_mode: ApprovalMode;
    commands: CommandPolicy;
  }) => invoke<ServerStatus[]>("mcp_set_permissions", { args }),
  /**
   * 回应一次确认。返回 false 表示这个请求已经失效
   * （重复回应，或那条流已结束）—— 此时应把确认框收掉。
   */
  respondConfirmation: (requestId: string, approved: boolean) =>
    invoke<boolean>("respond_tool_confirmation", {
      response: { request_id: requestId, approved },
    }),
};

export function onToolActivity(
  handler: (event: ToolActivityEvent & ToolActivity) => void,
) {
  return listen<ToolActivityEvent & ToolActivity>(TOOL_EVENT, (e) =>
    handler(e.payload),
  );
}

export function onToolConfirm(handler: (event: ToolConfirmEvent) => void) {
  return listen<ToolConfirmEvent>(CONFIRM_EVENT, (e) => handler(e.payload));
}

/** 默认权限：与 Rust 侧 ServerPermissions::default() 保持一致 */
export function defaultPermissions(): ServerPermissions {
  return {
    read: "allow",
    write: "ask",
    // 删除默认拒绝而不是询问 —— 不可逆，语音场景下容易随口答应
    delete: "deny",
    execute: "ask",
    classify: {},
    overrides: {},
  };
}
