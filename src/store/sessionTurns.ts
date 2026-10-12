import type { ChatTurn } from "./chat";

/**
 * 切会话时决定「哪些轮次留、哪些换掉」。
 *
 * 修的是这个 bug：切到别的对话再切回来，**正在生成的内容清空了**。
 *
 * 原因是 `loadSession` 用数据库里的消息整体替换 `turns`，而正在流的那一轮
 * 还没落库（要等 `done` 才写）。于是它从 UI 消失；更隐蔽的是 `done` 那里
 * `turns.find(...)` 也找不到人 —— **连补救落库都不会发生**，
 * 而后端的流还在跑、token 照烧。用户看到的就是"等了半天，一切走就全没了"。
 *
 * 别的会话在流的轮次也必须留在 `turns` 里：不留的话后续增量就写进了空气。
 * 它们不显示，靠 [`visibleTurns`] 在渲染层过滤。
 */
export function mergeOnSessionLoad(
  history: ChatTurn[],
  liveTurns: ChatTurn[],
  sessionId: string,
  liveStreamId: string | null,
): { turns: ChatTurn[]; activeStreamId: string | null } {
  // 停止按钮只该对**当前会话**的流生效
  const stillOurs = liveTurns.some((t) => t.sessionId === sessionId);
  return {
    turns: [...history, ...liveTurns],
    activeStreamId: stillOurs ? liveStreamId : null,
  };
}

/**
 * 当前会话该显示哪些轮次。
 *
 * 没有 `sessionId` 的（语音轮次等）一律显示 —— 它们不归任何文字会话管。
 */
export function visibleTurns(turns: ChatTurn[], sessionId: string | null): ChatTurn[] {
  return turns.filter((t) => !t.sessionId || t.sessionId === sessionId);
}

/**
 * 将从 SQLite 读出的历史消息还原为 UI 所需的 `ChatTurn`。
 *
 * 自动识别 `[上传图片]` 与 `[附件文档]` 注入块：
 * 1. 从 `file_url` / `local_path` 还原 `images` 图片列表（通过 convertFileSrc 转换为 Tauri 安全 Asset 协议），重新在界面上渲染图片缩略图；
 * 2. 剥离注入的 prompt 胶水说明，恢复原本干净的用户提问作为 `displayText`，避免重启后消息气泡变成冗长 URL。
 */
export function reconstructHistoryTurn(
  m: {
    id: number;
    role: string;
    content: string;
    tool_json?: string | null;
  },
  sessionId: string,
  toAssetUrl: (path: string) => string = (p) => p,
): ChatTurn {
  let displayText: string | undefined;
  let images: string[] | undefined;
  const content = m.content;

  if (m.role === "user") {
    if (content.includes("[上传图片]")) {
      const fileUrls = Array.from(content.matchAll(/file_url:\s*([^\r\n]+)/g)).map((match) => match[1].trim());
      const localPaths = Array.from(content.matchAll(/local_path:\s*([^\r\n]+)/g)).map((match) => match[1].trim());
      const extracted = fileUrls.length > 0 ? fileUrls : localPaths.map((p) => `file://${p}`);
      if (extracted.length > 0) {
        images = extracted.map((u) => {
          const raw = u.replace(/^file:\/\//, "");
          return toAssetUrl(raw);
        });
      }

      const parts = content.split(/\n\n+/);
      const userParts = parts.filter(
        (p) =>
          !p.startsWith("[上传图片]") &&
          !p.startsWith("- Image ") &&
          !p.startsWith("  file_url:") &&
          !p.startsWith("  local_path:") &&
          !p.startsWith("[注意：") &&
          !p.startsWith("[附件文档")
      );
      if (userParts.length > 0) {
        displayText = userParts.join("\n\n").trim();
      } else {
        displayText = "已上传图片";
      }
    } else if (content.includes("[附件文档")) {
      const parts = content.split(/\n\n+/);
      const userParts = parts.filter(
        (p) => !p.startsWith("[附件文档") && !p.startsWith("```")
      );
      if (userParts.length > 0) {
        displayText = userParts.join("\n\n").trim();
      }
    }
  }

  return {
    id: `turn-history-${m.id}`,
    messageId: m.id,
    role: m.role as "user" | "assistant",
    content,
    displayText,
    images,
    sessionId,
    reasoning: "",
    ...parseTurnTools(m.tool_json),
    streaming: false,
  };
}

/**
 * 单条工具结果落库时保留的字符数上限。
 *
 * 图片/视频工具的结果只有几百字（一段 markdown + seed），怎么截都够；
 * 真正会撑爆的是网页抓取、读文件这类，一条能上百 KB。这张表是永久保存的，
 * 不设上限等于让每一次抓网页都在磁盘上留一份全文。
 *
 * 送给模型时另有 recent/medium/archived 三档预算再截一次，所以这里
 * 只需要"别把库撑爆"，不需要精确。
 */
export const TOOL_RESULT_PERSIST_CHARS = 20_000;

/** 落库/还原用的形状。字段名保持和 `ChatTurn` 一致，省一层映射。 */
export interface PersistedTools {
  toolCalls: ChatTurn["toolCalls"];
  toolResults: NonNullable<ChatTurn["toolResults"]>;
}

/**
 * 把一轮的工具调用与结果序列化成落库用的 JSON。
 *
 * 没有工具就返回 `undefined` —— 让绝大多数普通对话的这一列保持 NULL，
 * 而不是存一堆 `{"toolCalls":[],"toolResults":[]}`。
 */
export function serializeTurnTools(t: {
  toolCalls?: ChatTurn["toolCalls"];
  toolResults?: ChatTurn["toolResults"];
}): string | undefined {
  const calls = t.toolCalls ?? [];
  const results = t.toolResults ?? [];
  if (calls.length === 0 && results.length === 0) return undefined;
  const payload: PersistedTools = {
    toolCalls: calls,
    toolResults: results.map((r) => ({
      ...r,
      content:
        r.content.length > TOOL_RESULT_PERSIST_CHARS
          ? `${r.content.slice(0, TOOL_RESULT_PERSIST_CHARS)}\n…（结果过长，已截断）`
          : r.content,
    })),
  };
  return JSON.stringify(payload);
}

/**
 * 还原落库的工具调用。
 *
 * 任何解析失败都退回"这一轮没有工具"，**不能让一条坏记录打不开整个对话** ——
 * 这一列是后加的，老库里全是 NULL，新库里也可能被手工改坏。
 */
export function parseTurnTools(json: string | null | undefined): PersistedTools {
  const empty: PersistedTools = { toolCalls: [], toolResults: [] };
  if (!json) return empty;
  try {
    const v = JSON.parse(json) as Partial<PersistedTools>;
    return {
      toolCalls: Array.isArray(v.toolCalls) ? v.toolCalls : [],
      toolResults: Array.isArray(v.toolResults) ? v.toolResults : [],
    };
  } catch {
    return empty;
  }
}
