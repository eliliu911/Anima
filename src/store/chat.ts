import { create } from "zustand";

import * as api from "../api";
import type {
  ConfigView,
  Message,
  StreamEvent,
  ThinkingEffort,
  Tier,
  ToolCall,
} from "../api";
import { convertFileSrc } from "@tauri-apps/api/core";
import { DeltaBuffer } from "./deltaBuffer";
import { mergeOnSessionLoad, reconstructHistoryTurn, serializeTurnTools } from "./sessionTurns";
import { useProject } from "./project";
import { historyBudget as computeHistoryBudget } from "./historyBudget";

/** UI 侧的消息模型：比发给模型的 Message 多了渲染需要的字段 */
export interface ChatTurn {
  id: string;
  messageId?: number;
  role: "user" | "assistant";
  /**
   * 发给模型的内容。斜杠命令展开后是几百字的规程，存的是**展开后**那份。
   *
   * 历史预算算的是这一份，不是 `displayText` —— 反了的话，一条看起来只有
   * 二十个字符的消息实际占掉几百，预算会持续算少。
   */
  content: string;
  /**
   * 界面上显示的内容。只有斜杠命令会用到：显示 `/review src/lru.ts`，
   * 而 `content` 是展开后的规程全文。
   *
   * 不设则显示 `content`。
   */
  displayText?: string;
  images?: string[];
  /** 推理模型的思考过程，UI 折叠显示 */
  reasoning: string;
  toolCalls: ToolCall[];
  /** 工具执行的完整结果，用于跨轮上下文保留 */
  toolResults?: { callId: string; name: string; content: string; ok?: boolean }[];
  /**
   * 这一轮属于哪个会话。
   *
   * **切会话时用来判断哪些轮次不能丢。** 之前 `loadSession` 直接用数据库里的
   * 消息整体替换 `turns`，而正在流的那一轮还没落库（要等 `done`），于是：
   * 切走再切回来，内容从 UI 和数据库**双双消失**，而后端还在继续烧 token。
   * 更隐蔽的是 `done` 那里 `turns.find(...)` 也找不到人，所以连补救落库都没有。
   */
  sessionId?: string;
  /** 是否还在流式接收中 */
  streaming: boolean;
  /** 开始流式生成的时间戳 (ms) */
  startedAt?: number;
  /**
   * 流式过程中已收到的字符数（正文 + 思考）。
   *
   * 用来在等待时给出**活着的**反馈。真实 token 数只在 `Done` 里到一次
   * （`agent.rs` 会吞掉中间轮次的 Done，所以流式中拿不到），
   * 光靠它的话用户在几十秒里看不到任何变化，会以为界面卡死。
   */
  streamedChars?: number;
  /** 最终 token 用量，来自 `Done` 事件。在此之前只能用 streamedChars 估算 */
  usage?: { prompt: number; completion: number; reasoning?: number };
  /** 本轮对话累计执行/思考的总耗时 (秒) */
  durationSeconds?: number;
  /** 本轮实际使用的后端，用于在气泡上标注 */
  provider?: string;
  model?: string;
  /** 被打断的轮次单独标记，UI 上给个提示 */
  cancelled?: boolean;
  /**
   * 这一轮是因为**工具往返用尽**才停的，不是模型说完了。
   *
   * 撞上限时 Anima 会摘掉工具让模型收场，收尾看起来和正常结束一样，
   * 但内容往往是半截的。不标出来的话用户只能看到"话没说完就停了"，
   * 而且会以为是模型不行。
   */
  roundsExhausted?: boolean;
  error?: string;
}

/** 某个后端的模型列表拉取状态 */
export interface ModelList {
  status: "idle" | "loading" | "ok" | "error";
  list: string[];
  error?: string;
  /**
   * 这个后端里能吃图的模型 id。
   *
   * **`undefined` 表示查不到，不是"没有视觉模型"。** 只有 OpenAI 兼容那条路
   * 能查（LM Studio 的 `/api/v0/models` 带 `type: "vlm"`）。查不到时界面
   * 什么都不说 —— 谎报"不支持视觉"比不说更糟。
   */
  visionModels?: string[];
}

const EMPTY_MODELS: ModelList = { status: "idle", list: [] };

/**
 * 当前链路选中的那个模型能不能吃图。
 *
 * 返回 `undefined` = 不知道（后端查不到能力），调用方应当什么都不显示。
 *
 * 不能靠模型名推断：实测同一个基座 `qwen/qwen3.8-27b` 是 vlm、
 * `qwen3.8-27b-uncensored` 是 llm —— 差别只在打包时有没有带 mmproj
 * （视觉编码器 + 投影层），名字上看不出来。
 */
export function modelSupportsVision(
  config: ConfigView | null,
  models: Record<string, ModelList>,
  tier: Tier,
): boolean | undefined {
  if (!config) return undefined;
  const providerId = tier === "realtime" ? config.realtime_provider : config.background_provider;
  const provider = config.providers.find((p) => p.id === providerId);
  const vision = models[providerId]?.visionModels;
  if (!provider || !vision) return undefined;
  const model = (tier === "realtime" ? config.realtime_model : config.background_model) ?? provider.model;
  return vision.includes(model);
}

interface ChatState {
  turns: ChatTurn[];
  config: ConfigView | null;
  /** provider id -> 该后端真实可用的模型列表（从后端拉取，不写死） */
  models: Record<string, ModelList>;
  /** 当前在途的流。非空即代表"正在生成"，UI 据此显示停止按钮 */
  activeStreamId: string | null;
  /** 当前分层选择，决定这轮走实时还是后台模型 */
  tier: Tier;
  systemPrompt: string;
  /** 全局错误（连接失败、配置错误等），与单轮的 error 分开 */
  error: string | null;
  /**
   * 当前后端加载的模型有多大上下文窗口（token）。`null` = 还没读到 / 读不到。
   *
   * **读来的，不是填的。** 用户在 LM Studio 里改一次加载参数，
   * 填的那个数就漂了；而漂了的表现是撞窗口报 500。
   */
  contextWindow: number | null;
  /** 滚动摘要：被窗口裁掉的历史压缩成的摘要，注入 system prompt */
  summary: { text: string; focus: string[] };
  /**
   * 已经压缩进摘要的最后一个轮次 id。
   *
   * **不能省。** `buildBoundedMessages` 是对完整 `turns` 的纯函数，而 `turns`
   * 从不缩短，所以每次发送算出的 `droppedTurns` 都包含上一次已经压过的那些。
   * 而 `compress()` 的语义是累积（新摘要 = 旧摘要 + 这次裁掉的），
   * 于是最老的几轮会被反复压进摘要，在 300 字上限下把新内容挤出去 ——
   * 表现为「记得很久以前的事，却忘了刚才的事」。
   *
   * 语音侧没有这个问题，因为它靠 `history.drain()` 真的把内容移走了。
   */
  compactedThrough: string | null;
  /** 压缩在途。并发压缩会互相覆盖，而且后发起的用的是更旧的 previous */
  compacting: boolean;

  /* UI 浮层状态与主题 */
  themeMode: "jarvis" | "glass";
  isSettingsOpen: boolean;
  isMcpOpen: boolean;

  setThemeMode: (mode: "jarvis" | "glass") => void;

  init: () => Promise<void>;
  /**
   * 发一轮。`displayText` 只影响界面显示，不影响发给模型的内容
   * —— 斜杠命令用它把几百字的规程显示成一行 `/review src/lru.ts`。
   */
  send: (text: string, images?: string[], displayText?: string) => Promise<void>;
  cancel: () => Promise<void>;
  clear: () => void;
  setTier: (tier: Tier) => void;
  /** 设置当前链路的思考档位。档位是按链路记的，不跟着模型走 */
  setThinkingEffort: (effort: ThinkingEffort) => Promise<void>;
  setSystemPrompt: (prompt: string) => void;
  refreshConfig: () => Promise<void>;
  setConfig: (config: ConfigView) => void;
  /** 读当前后端加载的模型窗口。见 `contextWindow`。 */
  refreshContextWindow: () => Promise<void>;
  dismissError: () => void;
  toggleSettings: () => void;
  toggleMcp: () => void;

  deleteTurn: (turnId: string) => Promise<void>;
  rollbackTurn: (turnId: string) => Promise<string | undefined>;
  /** 拉取单个后端的真实模型列表 */
  /**
   * 语音链路写入对话历史。
   *
   * 语音和文字共用同一份 `turns`，界面上才是一个连续的会话；但语音不走
   * `send()`（那条路会再发一次 LLM 请求 —— 语音的请求是 Rust 侧管线自己发的）。
   * 所以单独开三个方法，只操作 UI 状态、不触发任何后端调用。
   */
  pushVoiceTurn: (
    role: "user" | "assistant",
    content: string,
    streaming?: boolean,
  ) => string;
  appendVoiceDelta: (
    turnId: string,
    delta: string,
    field: "content" | "reasoning",
  ) => void;
  finishVoiceTurn: (
    turnId: string,
    opts: { cancelled?: boolean; error?: string },
  ) => void;

  /**
   * 拉这个后端的模型列表。`reset`：先清掉旧列表 —— 改了地址或密钥之后用，
   * 旧列表是旧端点的，留着会让人以为改了没生效（2026-09-29 换 Azure 端点时就这样）
   */
  loadModels: (providerId: string, opts?: { reset?: boolean }) => Promise<void>;
  /** 拉取所有"有希望连上"的后端的模型列表 */
  loadAllModels: () => Promise<void>;
  
  loadSession: (sessionId: string) => Promise<void>;
}

let nextId = 0;
const newId = () => `turn-${++nextId}`;

/**
 * 阶段一的占位 System Prompt。
 *
 * 阶段三接入性格引擎后，这里会换成由情感轴（好感度/心情/活跃度）动态拼接的
 * 结果（见 ARCHITECTURE.md 3.3），所以刻意做成可编辑的一段文本而不是写死。
 */
const DEFAULT_SYSTEM_PROMPT =
  "You are Anima, an intelligent, charismatic, and efficient desktop personal Agent and creative assistant.\n\n" +
  "[Tool & Formatting Guidelines]\n" +
  "- When web search results are retrieved and sufficient, immediately summarize and answer based on the results without launching redundant continuous searches.\n" +
  "- When presenting code snippets, test cases, configs, or terminal commands, you MUST wrap them in standard Markdown code blocks (e.g. ```typescript ... ```). Do not output unformatted raw code.\n" +
  "- When summarizing code changes or technical analysis, use clear Markdown headers (`##`) and bullet points (`- `) with concise overviews. Do not dump excessive implementation trivia.\n" +
  "- Maintain a lively, natural conversational tone.";

/**
 * 每轮拼给模型的历史字符上限。超出的从最老一端裁掉（先压成摘要）。
 *
 * **这个数是字符不是 token。** 本机实测的换算比（中文对话 1.92、Rust 源码 2.82、
 * 混合历史 2.30，平均约 2.35 字符/token）下：
 *
 *   local  48000 字符 ≈ 20400 token
 *   cloud 120000 字符 ≈ 51000 token
 *
 * local 定在 48000 的理由是**它要落在模型实际加载的上下文之内**。LM Studio
 * 默认按 32768 token 加载（模型原生支持 262144，是 LM Studio 在收着），
 * 32768 token ≈ 77000 字符 —— 48000 用掉其中约六成，给回复、工具定义和
 * system prompt 留下余量。
 *
 * 想再往上调之前先确认两件事，否则会踩坑：
 *   1. **超过约 65000 字符就要同步抬高 LM Studio 的加载上下文**，
 *      否则请求会被后端截断，而且不会有明显报错。
 *   2. prefill 成本随实际用量线性涨。本机 10K token 提示词实测：
 *      coder-next 3.10s、3.6-35b-a3b 3.20s、gpt-oss-120b 5.48s，
 *      但稠密的 qwen3.8-27b 要 18.25s —— 拿它当聊天主力时这个预算太大了。
 *      预算目前只按 local/cloud 分两档，撞到这种模型只能换模型。
 */
/**
 * **已经不是主判据了。** 现在预算从模型的上下文窗口反算
 * （见 `historyBudget.ts`），这两个数只在**读不到窗口时**回落用
 * （Ollama / MLX / 云端都没有那个接口）。
 *
 * 留在这里是因为 `buildBoundedMessages` 的默认参数还指着它，
 * 而那个默认值的存在理由见下面那条注释。
 */
export const MAX_HISTORY_CHARS = {
  local: 48_000,
  cloud: 120_000,
} as const;

export function getModelBudgetTier(config: ConfigView | null, tier: Tier): "local" | "cloud" {
  if (!config) return "local";
  const providerId = tier === "realtime" ? config.realtime_provider : config.background_provider;
  const provider = config.providers.find((p) => p.id === providerId);
  if (!provider) return "local";
  return provider.model_tier ?? "local";
}

/** arguments 可能带整个文件内容或大补丁，摘要里只需保留前 200 字符 */
export function truncateArgs(args: string, max = 200): string {
  const cleaned = stripInlineDataUrls(args);
  if (!cleaned) return "{}";
  return cleaned.length <= max ? cleaned : cleaned.slice(0, max) + `…(共 ${cleaned.length} 字符)`;
}

const INLINE_DATA_URL_RE =
  /data:((?:audio|video|image)\/[a-z0-9.+-]+);base64,[A-Za-z0-9+/=\r\n]+/gi;

export function stripInlineDataUrls(text: string): string {
  if (!text) return text;
  return text.replace(INLINE_DATA_URL_RE, (match, mime: string) => {
    return `data:${mime};base64,[omitted ${match.length} chars]`;
  });
}

/**
 * 一个工具轮里**所有**工具结果加起来的字符预算，在该轮的各条结果之间平分。
 *
 * **必须按轮分配，不能按条。** 早先是「每条 8000」，没有每轮上限 ——
 * 实测一个 13 次调用的正常任务（那个 LRU）会让单个轮次占掉
 * 48020 / 48000 字符，也就是 99.96% 的预算，把更早的所有轮次挤光，
 * 其中包括用户的原始需求「帮我实现 LRU」。
 * 结果和意图正好相反：模型看得到 13 份文件转储，看不到要它做什么。
 *
 * 而这个风险是被轮次上限放宽（12 → 40）激活的：12 轮时最多攒几次调用，
 * 40 轮时轻易上双位数。两个改动单独看都对，合在一起才出问题。
 *
 * 16000 ≈ 本地预算 48000 的三分之一：一轮工具最多占三分之一，
 * 剩下三分之二留给对话本身。
 */
/**
 * 细粒度微上下文压缩 (Micro-compaction) 的三级衰减阶梯：
 * 1. recent (最近 2 轮): 完整保留上下文（每轮最多 16000 字符）
 * 2. medium (3~5 轮前): 单行轻量摘要指纹（每轮最多 1200 字符）
 * 3. archived (6 轮以上): 仅保留工具签名与精简参数（无返回结果）
 */
export type ToolCompactionTier = "recent" | "medium" | "archived";

const RECENT_TURN_TOOL_BUDGET = 16_000;
const MEDIUM_TURN_TOOL_BUDGET = 1_200;
const MIN_TOOL_RESULT_CHARS = 100;

/** 折叠提示。长度要算进预算 —— 它是切片之后追加的 */
const FOLD_MARKER = "\n\n[...本条消息因上下文预算限制部分折叠...]";

/**
 * 智能提取工具返回的单行轻量摘要指纹（用于 medium 衰减层）
 * 优先依据后端传来的结构化 ok 判定，避免字符串猜测误判
 */
function extractToolFingerprint(toolName: string, content: string, ok?: boolean): string {
  if (!content && ok !== false) return "[执行完毕，无输出]";
  const trimmed = content.trim();
  const lowerName = toolName.toLowerCase();

  // 1. 文件读取类 (view_file, fs_read_text, code_outline)
  if (lowerName.includes("view") || lowerName.includes("read") || lowerName.includes("outline")) {
    if (ok === false) {
      const firstLine = trimmed.split("\n")[0]?.slice(0, 80) || "";
      return `[读取失败: ${firstLine}]`;
    }
    const lines = trimmed.split("\n").length;
    const bytes = content.length;
    return `[已读取内容: 共 ${lines} 行，${bytes} 字符]`;
  }

  // 2. 文件编辑与补丁 (fs_write_text, replace_file_content, apply_patch)
  if (lowerName.includes("write") || lowerName.includes("replace") || lowerName.includes("patch")) {
    if (ok === false) {
      const firstLine = trimmed.split("\n")[0]?.slice(0, 80) || "";
      return `[文件修改失败: ${firstLine}]`;
    }
    return "[文件修改成功并已落盘]";
  }

  // 3. 终端命令 (shell_command, exec_command)
  if (lowerName.includes("shell") || lowerName.includes("command") || lowerName.includes("exec")) {
    const firstLine = trimmed.split("\n")[0]?.slice(0, 80) || "";
    if (ok === false) {
      return `[命令失败: ${firstLine}]`;
    }
    if (ok === true || !trimmed.includes("退出码") || trimmed.includes("exit status: 0") || trimmed.includes("退出码 0") || trimmed.includes("退出码：0")) {
      return `[命令执行成功: ${firstLine || "退出码 0"}]`;
    }
    return `[命令失败: ${firstLine}]`;
  }

  // 4. 知识库检索 (search_knowledge)
  if (lowerName.includes("knowledge") || lowerName.includes("search")) {
    if (ok === false) {
      return "[知识库检索失败]";
    }
    const docs = trimmed.split("\n\n").length;
    return `[知识库检索命中: ${docs} 条相关片段]`;
  }

  // 5. 子智能体调研 (invoke_subagent)
  if (lowerName.includes("subagent")) {
    return "[子智能体调研完成]";
  }

  // 通用单行截断
  const singleLine = trimmed.replace(/\s+/g, " ").slice(0, 100);
  return singleLine ? `[${singleLine}]` : (ok === false ? "[执行失败]" : "[已完成]");
}

/**
 * 把一个工具轮次还原成**协议原样**的消息：`assistant.tool_calls` + 每个调用
 * 对应一条 `tool` 结果。
 *
 * 为什么非这样不可 —— `〔已调用工具: X 参数: Y 返回: Z〕` 那段文本是长在
 * **assistant 自己的消息里**的，对模型就是一份「我平时这么说话」的示范。
 * 实测里模型照抄了它：连截断标记 `…(共 830 字符)` 一起抄进正文。那个串只
 * 可能由本文件的 `truncateArgs` 生成，真实工具返回里**不存在**，所以它是
 * 模型在模仿渲染结果、而不是真的发起了调用。用户看到的是「秒出已生成成功」
 * 却没有图 —— 同一时段 ComfyUI 的 /history 一条新记录都没有。
 *
 * 只对最近的 recent 档这么做：medium / archived 本来就是**摘要**，恢复成
 * 协议原样等于放弃压缩。它们离当前轮次远，被模仿的概率也低得多。
 *
 * 返回 null 表示这一轮不适合结构化（缺 call id 等），调用方退回文本渲染 ——
 * 带 `tool_calls` 的 assistant 消息**必须**每个 id 都跟一条 tool 结果，
 * 配不齐时 OpenAI 会直接 400，宁可退回去。
 */
export function expandToolTurn(t: ChatTurn): Message[] | null {
  if (t.role !== "assistant" || !t.toolCalls || t.toolCalls.length === 0) return null;
  if (t.toolCalls.some((c) => !c.id)) return null;

  // 每条结果分到的额度和 renderTurnContent 的 recent 档一致
  const resultCount = t.toolResults?.length || t.toolCalls.length || 1;
  const resultChars = Math.max(
    MIN_TOOL_RESULT_CHARS,
    Math.floor(RECENT_TURN_TOOL_BUDGET / resultCount),
  );

  const messages: Message[] = [
    {
      role: "assistant",
      content: t.content,
      tool_calls: t.toolCalls.map((c) => ({
        id: c.id,
        name: c.name,
        arguments: stripInlineDataUrls(c.arguments) || "{}",
      })),
    },
  ];

  for (const c of t.toolCalls) {
    const result = t.toolResults?.find((r) => r.callId === c.id);
    messages.push({
      role: "tool",
      tool_call_id: c.id,
      content: result ? truncateArgs(result.content, resultChars) : "[无返回]",
    });
  }

  return messages;
}

/**
 * 结构化形式**真正发出去**的字符数。
 *
 * 预算必须按这一份算，不能沿用文本渲染的长度：文本渲染把参数截到 200 字符，
 * 结构化形式保留完整参数（截断的 JSON 不再是合法 JSON，不能截）。第一版忘了
 * 这件事，「超长参数」那条测试立刻把总量从 48000 顶到 480022 —— 十倍。
 */
export function structuredMessagesLen(messages: Message[]): number {
  return messages.reduce(
    (sum, m) =>
      sum +
      m.content.length +
      (m.tool_calls?.reduce((a, c) => a + c.name.length + c.arguments.length, 0) ?? 0),
    0,
  );
}

/** 一个轮次真正会发出去的 content —— 预算计算和组装必须用同一份渲染结果 */
export function renderTurnContent(
  t: ChatTurn,
  opts: { compactionTier: ToolCompactionTier },
): string {
  if (t.role !== "assistant" || !t.toolCalls || t.toolCalls.length === 0) {
    return t.content;
  }

  // **档位必填，没有默认值。**
  //
  // 早先它有个默认：`opts.recentToolResults !== false ? "recent" : "medium"`。
  // 而 `undefined !== false` 是 true —— 所以任何**不传 opts** 的调用都会拿到
  // `recent`（16000 字符）。摘要压缩那条路正是这样：它把即将被丢弃的轮次
  // 送去压缩，每轮送 16000 而不是 1200，**6.7 倍**，送的还是最不重要的内容。
  //
  // 修掉那一个调用点不够 —— 危险的是默认值本身。改成必填，让编译器盯着，
  // 将来新加的调用点必须自己说清要哪一档。
  const tier: ToolCompactionTier = opts.compactionTier;

  // archived 级别：只保留工具调用签名与极简参数，彻底剥离返回结果
  if (tier === "archived") {
    const toolSummaries = t.toolCalls
      .map((c) => `〔已调用工具: ${c.name} 参数: ${truncateArgs(c.arguments, 80)}〕`)
      .join("\n");
    return t.content ? `${t.content}\n\n${toolSummaries}` : toolSummaries;
  }

  const turnBudget = tier === "recent" ? RECENT_TURN_TOOL_BUDGET : MEDIUM_TURN_TOOL_BUDGET;
  const resultCount = t.toolResults?.length || t.toolCalls.length || 1;
  const toolResultChars = Math.max(
    MIN_TOOL_RESULT_CHARS,
    Math.floor(turnBudget / resultCount),
  );

  const toolSummaries = t.toolCalls
    .map((c) => {
      const result =
        t.toolResults?.find((r) => r.callId === c.id) ??
        (t.toolCalls.length === 1 ? t.toolResults?.[0] : undefined);

      if (!result) {
        return `〔已调用工具: ${c.name} 参数: ${truncateArgs(c.arguments, 120)}〕`;
      }

      if (tier === "medium") {
        const fingerprint = extractToolFingerprint(c.name, result.content, result.ok);
        return `〔已调用工具: ${c.name} 参数: ${truncateArgs(c.arguments, 120)}\n  状态: ${fingerprint}〕`;
      }

      // recent 级别保留详细切片
      const resultSnippet = `\n  返回: ${truncateArgs(result.content, toolResultChars)}`;
      return `〔已调用工具: ${c.name} 参数: ${truncateArgs(c.arguments, 200)}${resultSnippet}〕`;
    })
    .join("\n");

  return t.content ? `${t.content}\n\n${toolSummaries}` : toolSummaries;
}

/**
 * 从这次算出的 `droppedTurns` 里，摘出**尚未压缩进摘要**的那一段。
 *
 * `compress()` 是累积语义，重复投喂同一批内容会让摘要越来越偏向最老的轮次。
 * 阈值判断交给 Rust 侧的 `worth_summarizing()`（`compress_history` command 里已经
 * 调了它，不够压就原样返回旧摘要）——这里不再抄一份 4 的阈值，
 * 否则改一处另一处不会跟着变。
 */
export function takeUncompacted(
  droppedTurns: ChatTurn[],
  compactedThrough: string | null,
): ChatTurn[] {
  if (!compactedThrough) return droppedTurns;
  const idx = droppedTurns.findIndex((t) => t.id === compactedThrough);
  // 找不到说明那一轮已经不在裁剪范围里（例如会话被清空过），保守地全量重压
  return idx < 0 ? droppedTurns : droppedTurns.slice(idx + 1);
}

// 智能上下文滑动窗口管理：防止历史记录或大文件读取过多撑爆本地 LLM 上下文
export interface BoundedMessagesResult {
  messages: Message[];
  /** 被预算裁掉的轮次，用于异步压缩成摘要 */
  droppedTurns: ChatTurn[];
}

type TurnPlan = { content: string; structured: Message[] | null; len: number };

export function buildBoundedMessages(
  systemPrompt: string,
  turns: ChatTurn[],
  currentAssistantTurnId: string,
  // 默认值引用常量，不要抄一份数字过来 —— 这里曾经硬编码 24_000，
  // 调整 MAX_HISTORY_CHARS 时它不会跟着变，漏传参数的调用方会静默用回旧值。
  // 必须显式标 number：MAX_HISTORY_CHARS 带 as const，不标的话参数类型会被
  // 窄化成字面量 48000，传 cloud 档的 120000 会编译不过。
  maxHistoryChars: number = MAX_HISTORY_CHARS.local,
  summary?: { text: string; focus: string[] },
): BoundedMessagesResult {
  // 把摘要注入 system prompt
  let effectiveSystemPrompt = systemPrompt;
  if (summary && (summary.text.trim() || summary.focus.length > 0)) {
    effectiveSystemPrompt += "\n\n## 之前聊过的（这是摘要，不是原话）\n";
    if (summary.text.trim()) {
      effectiveSystemPrompt += summary.text.trim() + "\n";
    }
    if (summary.focus.length > 0) {
      effectiveSystemPrompt += "\n还没完成的事：\n";
      effectiveSystemPrompt += summary.focus.map((f) => `- ${f}`).join("\n") + "\n";
    }
  }

  // 1. 过滤有效轮次
  const validTurns = turns.filter(
    (t) =>
      t.id !== currentAssistantTurnId &&
      !t.error &&
      !(t.role === "assistant" && !t.content.trim() && (!t.toolCalls || t.toolCalls.length === 0)),
  );

  if (validTurns.length === 0) {
    return {
      messages: [{ role: "system", content: effectiveSystemPrompt }],
      droppedTurns: [],
    };
  }

  const assistantToolTurns = validTurns
    .filter((t) => t.role === "assistant" && (t.toolCalls?.length ?? 0) > 0);
  const totalAssistantToolCount = assistantToolTurns.length;

  const recentToolTurnIds = new Set(
    assistantToolTurns.slice(-2).map((t) => t.id),
  );
  const mediumToolTurnIds = new Set(
    assistantToolTurns.slice(Math.max(0, totalAssistantToolCount - 5), Math.max(0, totalAssistantToolCount - 2)).map((t) => t.id),
  );

  // 预算和装配必须看**同一份**计划：recent 档的工具轮次发的是协议原样
  // （assistant.tool_calls + tool 结果），它的字符数和文本渲染不一样，
  // 按文本渲染记账会直接超预算 —— 实测顶到 480022 / 48000。
  const planTurn = (turn: ChatTurn): TurnPlan => {
    let compactionTier: ToolCompactionTier = "archived";
    if (recentToolTurnIds.has(turn.id)) {
      compactionTier = "recent";
    } else if (mediumToolTurnIds.has(turn.id)) {
      compactionTier = "medium";
    }
    const content = renderTurnContent(turn, { compactionTier });
    if (compactionTier !== "recent") return { content, structured: null, len: content.length };
    const structured = expandToolTurn(turn);
    if (!structured) return { content, structured: null, len: content.length };
    return { content, structured, len: structuredMessagesLen(structured) };
  };

  // 2. 最新的那一轮提问必须完整保留（使用渲染后的实际长度计算预算）
  const lastTurn = validTurns[validTurns.length - 1];
  const lastPlan = planTurn(lastTurn);
  const renderedLastContent = lastPlan.content;
  const priorTurns = validTurns.slice(0, -1);

  // 3. 从后向前累加历史，直到达到字符预算
  let charBudget = maxHistoryChars - (effectiveSystemPrompt.length + lastPlan.len);
  const selectedPrior: {
    turn: ChatTurn;
    content: string;
    structured: Message[] | null;
    folded: boolean;
  }[] = [];
  const droppedTurns: ChatTurn[] = [];

  // **用户轮次先占位。**
  //
  // 用户消息通常只有十几个字符，却承载着**唯一的意图**。为了多塞一份文件
  // 转储而把一条 15 字符的用户消息挤掉，永远是错的交换 —— 实测撞到过：
  // 一个工具轮占掉 99.96% 的预算，把「帮我实现 LRU」裁掉了，
  // 用户说「继续」时模型看得到 13 份文件内容、看不到要它做什么。
  //
  // 所以先把它们的长度从预算里扣掉，剩下的才拿去装助手轮次。
  const userReserve = priorTurns
    .filter((t) => t.role === "user")
    .reduce((sum, t) => sum + planTurn(t).len, 0);
  // 极端情况（用户消息本身就超预算）不做保留，退回原来的先来先得，
  // 否则会一条都装不下
  const reserveUserTurns = userReserve > 0 && userReserve < maxHistoryChars * 0.5;
  if (reserveUserTurns) charBudget -= userReserve;

  for (let i = priorTurns.length - 1; i >= 0; i--) {
    const turn = priorTurns[i];
    const plan = planTurn(turn);
    const renderedContent = plan.content;
    const turnLen = plan.len;

    // 用户轮次已经预留过额度，直接收下，不再从 charBudget 里扣
    if (reserveUserTurns && turn.role === "user") {
      selectedPrior.unshift({ turn, content: renderedContent, structured: plan.structured, folded: false });
      continue;
    }

    if (charBudget > 0) {
      if (turnLen <= charBudget) {
        selectedPrior.unshift({ turn, content: renderedContent, structured: plan.structured, folded: false });
        charBudget -= turnLen;
      } else {
        // 部分截断历史中的超长回复。
        // **折叠标记也要算进预算** —— 它是在切片之后追加的，
        // 不减掉的话总量会超出预算（实测超 54 字符）。
        const room = charBudget - FOLD_MARKER.length;
        if (room >= 200) {
          selectedPrior.unshift({
            turn,
            content: renderedContent.slice(0, room) + FOLD_MARKER,
            // 折叠过的轮次不能走协议原样：内容已从中间切断，配不出完整结果
            structured: null,
            folded: true,
          });
        }
        charBudget = 0;
        // 被裁掉的更早轮次收集起来用于压缩（用户轮次已保留，不算被裁）
        for (let j = 0; j < i; j++) {
          if (!(reserveUserTurns && priorTurns[j].role === "user")) {
            droppedTurns.push(priorTurns[j]);
          }
        }
        break;
      }
    } else {
      for (let j = 0; j <= i; j++) {
        if (!(reserveUserTurns && priorTurns[j].role === "user")) {
          droppedTurns.push(priorTurns[j]);
        }
      }
      break;
    }
  }

  const messages: Message[] = [
    { role: "system", content: effectiveSystemPrompt },
  ];

  if (droppedTurns.length > 0 && !summary?.text) {
    messages.push({
      role: "system",
      content: "[提示：更早的对话历史因上下文窗口配额已由系统自动折叠]",
    });
  }

  for (const item of selectedPrior) {
    // 对齐 Codex / Claude Code 多模态历史衰减（Vision Eviction）：
    // 历史轮次中的大型 Base64 图片在进入多轮对话时自动脱落，
    // 仅保留轻量文本标记，防止多轮后数兆图片数据重复发送撑爆显存和 Token 预算。
    const hasPriorImages = item.turn.images && item.turn.images.length > 0;
    const content = hasPriorImages
      ? `${item.content}\n\n[附图：${item.turn.images!.length} 张图片已在先前轮次处理]`
      : item.content;

    // 最近两轮的工具调用改发协议原样（见 expandToolTurn 的注释）。
    // 折叠过的轮次不能拆 —— 内容已被从中间切断，拆出来的 tool 结果对不上。
    if (item.structured && !hasPriorImages) {
      messages.push(...item.structured);
      continue;
    }

    messages.push({
      role: item.turn.role,
      content,
      images: undefined,
    });
  }

  messages.push({
    role: lastTurn.role,
    content: renderedLastContent,
    images: lastTurn.images && lastTurn.images.length > 0 ? lastTurn.images : undefined,
  });

  return { messages, droppedTurns };
}

let streamSub: Promise<() => void> | null = null;
function setStreamSub(p: Promise<() => void> | null) {
  streamSub = p;
}

export const useChat = create<ChatState>((set, get) => {
  const patchTurn = (id: string, fn: (turn: ChatTurn) => ChatTurn) => {
    set((state) => ({
      turns: state.turns.map((t) => (t.id === id ? fn(t) : t)),
    }));
  };

  /**
   * stream_id -> 该流对应的 UI 轮次 id。
   *
   * 需要这张表是因为事件监听是全局的，而并发的流不止一条（将来还有后台
   * 记忆整理任务），必须能把事件路由回正确的气泡。
   */
  const streamToTurn = new Map<string, string>();

  /**
   * 文本 / 思考增量的合帧缓冲。
   *
   * 后端是按 token 推 delta 的，而每一次 `patchTurn` 都会换掉整个 `turns` 数组
   * —— 于是**整条对话的每一轮**都重渲染，每轮的 markdown 都被 remark 重新解析一遍。
   * 实测这份成本随对话长度线性增长（20 轮 × 7.7KB 的对话约 94ms 一次），
   * 而它是**每个 delta 付一遍**的：一条 400 delta 的流累计能到 30~40 秒。
   *
   * 表现正是「流到一半不动了、计时器和 token 也停、加载动画还在转，
   * 几十秒后一次性全出来」—— 事件没停，是主线程被渲染占死了，
   * 排队的更新等到流结束才追上。窗口拖不动也是同一个原因
   * （`decorations: false`，拖拽由 webview 里的 JS 处理）。
   *
   * 所以这里按帧合并：一帧内到达的所有 delta 只触发一次 set。
   * 注意**非 delta 事件（tool_call / tool_result / done）必须先 flush 再处理**，
   * 否则顺序会乱 —— 工具卡片可能出现在它前面那段文字之前。
   */
  const deltaBuffer = new DeltaBuffer((batch) => {
    set((state) => ({
      turns: state.turns.map((t) => {
        const d = batch.get(t.id);
        if (!d) return t;
        return {
          ...t,
          content: t.content + d.text,
          reasoning: t.reasoning + d.reasoning,
          streamedChars: (t.streamedChars ?? 0) + d.text.length + d.reasoning.length,
        };
      }),
    }));
  });
  const turnToSession = new Map<string, string>();

  const handleEvent = (event: StreamEvent) => {
    const turnId = streamToTurn.get(event.stream_id);
    if (!turnId) return; // 不是当前 UI 关心的流（例如后台任务）

    switch (event.kind) {
      case "text":
        // 思考和正文都算进 streamedChars：等待期间大部分时间只有思考在流，
        // 只数正文的话计数会长时间不动，和"卡死"一样看不出区别。
        deltaBuffer.push(turnId, "text", event.delta);
        break;

      case "reasoning":
        deltaBuffer.push(turnId, "reasoning", event.delta);
        break;

      case "tool_call":
        deltaBuffer.flush();
        patchTurn(turnId, (t) => ({
          ...t,
          toolCalls: [...t.toolCalls, event.call],
        }));
        break;

      case "tool_result":
        deltaBuffer.flush();
        patchTurn(turnId, (t) => ({
          ...t,
          toolResults: [
            ...(t.toolResults ?? []),
            {
              callId: event.call_id,
              name: event.name,
              content: stripInlineDataUrls(event.content),
              ok: event.ok,
            },
          ],
        }));
        break;

      case "done": {
        deltaBuffer.flush();
        patchTurn(turnId, (t) => {
          const durationSeconds = t.startedAt ? Math.max(1, Math.round((Date.now() - t.startedAt) / 1000)) : undefined;
          return {
            ...t,
            streaming: false,
            cancelled: event.reason === "cancelled",
            roundsExhausted: event.reason === "tool_rounds_exhausted",
            durationSeconds: t.durationSeconds ?? durationSeconds,
            // usage 之前被整个丢掉了。它是唯一的**精确** token 数，
            // 拿到之后要替掉流式期间按字符估的那个值。
            usage: event.usage
              ? {
                  prompt: event.usage.prompt_tokens,
                  completion: event.usage.completion_tokens,
                  reasoning: event.usage.reasoning_tokens,
                }
              : t.usage,
          };
        });
        streamToTurn.delete(event.stream_id);
        // 只有当结束的正是当前在途流时才清空，避免后台流误清 UI 状态
        set((state) =>
          state.activeStreamId === event.stream_id
            ? { activeStreamId: null }
            : {},
        );

        // 持久化助手回复到 SQLite
        const targetSessionId =
          turnToSession.get(turnId) || useProject.getState().activeSessionId;
        const currentTurn = get().turns.find((t) => t.id === turnId);
        // 只有正文时才落库的话，"全程只调工具、一句话没说"的那一轮会整轮消失，
        // 连同它生成的图。所以有工具调用也算值得存。
        const toolJson = currentTurn ? serializeTurnTools(currentTurn) : undefined;
        if (targetSessionId && currentTurn && (currentTurn.content.trim() || toolJson)) {
          void api
            .appendSessionMessage(targetSessionId, "assistant", currentTurn.content.trim(), toolJson)
            .catch((err) => {
              console.error("Failed to persist assistant message:", err);
            });
        }
        turnToSession.delete(turnId);
        break;
      }

      case "error":
        patchTurn(turnId, (t) => {
          const durationSeconds = t.startedAt ? Math.max(1, Math.round((Date.now() - t.startedAt) / 1000)) : undefined;
          return {
            ...t,
            streaming: false,
            error: event.message,
            durationSeconds: t.durationSeconds ?? durationSeconds,
          };
        });
        streamToTurn.delete(event.stream_id);
        turnToSession.delete(turnId);
        set((state) =>
          state.activeStreamId === event.stream_id
            ? { activeStreamId: null, error: event.message }
            : { error: event.message },
        );
        break;
    }
  };

  return {
    turns: [],
    config: null,
    models: {},
    activeStreamId: null,
    tier: "realtime",
    compactedThrough: null,
    compacting: false,
    systemPrompt: DEFAULT_SYSTEM_PROMPT,
    error: null,
    contextWindow: null,
    summary: { text: "", focus: [] },

    themeMode: "jarvis",
    isSettingsOpen: false,
    isMcpOpen: false,

    setThemeMode(mode) {
      set({ themeMode: mode });
    },

    toggleSettings() {
      set((state) => ({ isSettingsOpen: !state.isSettingsOpen }));
    },

    toggleMcp() {
      set((state) => ({ isMcpOpen: !state.isMcpOpen }));
    },

    async init() {
      if (streamSub) return;
      try {
        setStreamSub(api.subscribeStreams(handleEvent));
        await get().refreshConfig();
      } catch (err) {
        setStreamSub(null);
        set({ error: `流式事件订阅失败，将收不到模型回复：${err}` });
      }
    },

    async send(text, images, displayText) {
      const trimmed = text.trim();
      const hasImages = images && images.length > 0;
      if ((!trimmed && !hasImages) || get().activeStreamId) return;

      const userTurn: ChatTurn = {
        id: newId(),
        role: "user",
        content: trimmed,
        displayText: displayText?.trim() || undefined,
        images: hasImages ? images : undefined,
        reasoning: "",
        toolCalls: [],
        streaming: false,
      };
      const assistantTurn: ChatTurn = {
        id: newId(),
        role: "assistant",
        content: "",
        reasoning: "",
        toolCalls: [],
        streaming: true,
        startedAt: Date.now(),
      };

      set((state) => ({
        turns: [...state.turns, userTurn, assistantTurn],
        error: null,
      }));

      // **预算从模型的窗口反算**，读不到才回落到固定档（见 historyBudget.ts）。
      //
      // 原来是写死的 48000 字符，而窗口的单位是 token —— 单位就不是一个东西。
      // 中文实测 1.92 字符/token，48000 字符是 25000 token，
      // 加上 system prompt、技能正文、工具定义和留给回复的位置，
      // 撞穿 33024 的窗口毫无难度，症状是上游一个 HTML 的 500。
      const budgetTier = getModelBudgetTier(get().config, get().tier);
      const budget = computeHistoryBudget(get().contextWindow, budgetTier);
      const historyBudget = budget.chars;
      const { messages, droppedTurns } = buildBoundedMessages(
        get().systemPrompt,
        get().turns,
        assistantTurn.id,
        historyBudget,
        get().summary,
      );

      // 被裁掉的轮次异步压缩成摘要（不阻塞本轮发送）。
      // 只送**这次新裁掉**的部分：见 compactedThrough 的注释。
      const freshlyDropped = takeUncompacted(droppedTurns, get().compactedThrough);
      if (freshlyDropped.length > 0 && !get().compacting) {
        const currentSummary = get().summary;
        const lastId = freshlyDropped[freshlyDropped.length - 1].id;
        const droppedMessages = freshlyDropped.map((t) => ({
          role: t.role,
          content: renderTurnContent(t, { compactionTier: "medium" }),
        }));
        set({ compacting: true });
        api
          .compressHistory(currentSummary.text, currentSummary.focus, droppedMessages)
          .then((result) => {
            // 只有真的压进去了才推进游标。Rust 侧「不够压」时会原样返回旧摘要，
            // 这时推进游标等于把这几轮判成已压缩，它们再也不会被压 —— 内容真丢。
            set({
              summary: { text: result.summary, focus: result.focus },
              ...(result.compressed ? { compactedThrough: lastId } : {}),
            });
          })
          .catch((err) => {
            console.warn("历史压缩失败（不影响本轮对话）:", err);
          })
          .finally(() => set({ compacting: false }));
      }

      const projectState = useProject.getState();
      let currentSessionId = projectState.activeSessionId;

      // 如果当前没有活跃会话，自动为用户新建一个并设为活跃会话
      if (!currentSessionId) {
        try {
          currentSessionId = await api.createSession(projectState.activeProjectId);
          projectState.setActiveSession(currentSessionId);
          void projectState.loadSessions(projectState.activeProjectId);
        } catch (err) {
          console.error("Failed to auto-create session:", err);
        }
      }

      if (currentSessionId) {
        turnToSession.set(assistantTurn.id, currentSessionId);
        // 回填会话归属 —— 切会话时靠它认出"这轮是别的会话的，别显示但也别丢"。
        // 只能在这里做：`currentSessionId` 可能是上面刚新建出来的。
        set((state) => ({
          turns: state.turns.map((t) =>
            t.id === userTurn.id || t.id === assistantTurn.id
              ? { ...t, sessionId: currentSessionId ?? undefined }
              : t,
          ),
        }));
        if (trimmed) {
          void api
            .appendSessionMessage(currentSessionId, "user", trimmed)
            .then(() => {
              void useProject.getState().loadSessions(useProject.getState().activeProjectId);
            })
            .catch((err) => {
              console.error("Failed to persist user message:", err);
            });
        }
      }

      try {
        const activeProject = projectState.projects.find(
          (p) => p.id === projectState.activeProjectId,
        );
        const handle = await api.startChat({
          messages,
          tier: get().tier,
          workspacePath: activeProject?.workspace_path ?? null,
          sessionId: useProject.getState().activeSessionId,
        });
        streamToTurn.set(handle.stream_id, assistantTurn.id);
        set({ activeStreamId: handle.stream_id });
        patchTurn(assistantTurn.id, (t) => ({
          ...t,
          provider: handle.provider_id,
          model: handle.model,
        }));
      } catch (err) {
        const message = String(err);
        patchTurn(assistantTurn.id, (t) => ({
          ...t,
          streaming: false,
          error: message,
        }));
        set({ activeStreamId: null, error: message });
      }
    },

    async cancel() {
      const streamId = get().activeStreamId;
      if (!streamId) return;
      await api.cancelChat(streamId);
      // 不在这里改 UI 状态：Rust 侧会补发一个 Done{cancelled}，
      // 统一由事件处理收尾，避免两条路径都改状态导致不一致。
    },

    clear() {
      // 有流在跑时先掐掉，否则它的 token 会往一个已经不存在的气泡里写
      const streamId = get().activeStreamId;
      if (streamId) void api.cancelChat(streamId);
      streamToTurn.clear();
      // compactedThrough 必须一起清。留着旧游标的话，新会话第一次裁剪时
      // takeUncompacted 找不到那个 id —— 虽然有兜底会全量重压，但语义上是错的。
      set({
        turns: [],
        activeStreamId: null,
        error: null,
        summary: { text: "", focus: [] },
        compactedThrough: null,
        compacting: false,
      });
    },

    async loadSession(sessionId) {
      // **先把还在流的轮次留下来再 clear。**
      //
      // 它们尚未落库（要等 `done`），一旦被数据库里的历史整体替换掉就彻底没了：
      // UI 上看不到、`done` 里 `turns.find(...)` 也找不到人，连补救落库都不会发生，
      // 而后端的流还在跑、token 照烧。表现就是"切走再切回来，正在生成的内容清空了"。
      //
      // 别的会话的在流轮次也要留在 `turns` 里 —— 不留的话增量就写进了空气。
      // 它们不该显示，靠渲染层按 `sessionId` 过滤（见 `ChatHistory`）。
      const live = get().turns.filter((t) => t.streaming);
      const liveStreamId = get().activeStreamId;

      get().clear();
      try {
        const msgs = await api.getSessionMessages(sessionId);
        const history: ChatTurn[] = msgs.map((m) =>
          reconstructHistoryTurn(m, sessionId, (p) => {
            try {
              return convertFileSrc(p);
            } catch {
              return p;
            }
          }),
        );
        set({ ...mergeOnSessionLoad(history, live, sessionId, liveStreamId), error: null });
      } catch (err) {
        set({ error: String(err) });
      }
    },

    async deleteTurn(turnId) {
      const turn = get().turns.find((t) => t.id === turnId);
      if (!turn) return;

      // 1. 从前端状态中移除
      set((state) => ({
        turns: state.turns.filter((t) => t.id !== turnId),
      }));

      // 2. 从本地 SQLite 数据库物理删除
      const activeSessionId = useProject.getState().activeSessionId;
      if (activeSessionId && turn.messageId) {
        try {
          await api.deleteSessionMessage(activeSessionId, turn.messageId);
        } catch (err) {
          console.error("Failed to delete message from database:", err);
        }
      }
    },

    async rollbackTurn(turnId) {
      const turns = get().turns;
      const index = turns.findIndex((t) => t.id === turnId);
      if (index === -1) return undefined;

      const targetTurn = turns[index];
      const userPrompt = targetTurn.role === "user" ? targetTurn.content : undefined;

      // 1. 从前端状态中截断从该轮开始的所有后续对话
      const remainingTurns = turns.slice(0, index);
      set({ turns: remainingTurns });

      // 2. 从 SQLite 数据库中物理删除从该条开始的所有后续记录
      const activeSessionId = useProject.getState().activeSessionId;
      if (activeSessionId && targetTurn.messageId) {
        try {
          await api.rollbackSessionToMessage(activeSessionId, targetTurn.messageId);
        } catch (err) {
          console.error("Failed to rollback messages in database:", err);
        }
      }

      return userPrompt;
    },

    setTier(tier) {
      set({ tier });
    },



    async setThinkingEffort(effort) {
      if (!get().config) return;
      try {
        // 存在**当前链路**名下。同一个模型可以同时服务两个档，
        // 按模型记的话在后台档开了思考、切回实时档会跟着开。
        set({ config: await api.setThinkingEffort(get().tier, effort) });
      } catch (err) {
        set({ error: String(err) });
      }
    },

    setSystemPrompt(systemPrompt) {
      set({ systemPrompt });
    },

    async refreshConfig() {
      try {
        const config = await api.getConfig();
        set({ config });
        // 配置到手后顺手拉真实模型列表，让设置面板一打开就有的选
        void get().loadAllModels();
        void get().refreshContextWindow();
      } catch (err) {
        set({ error: String(err) });
      }
    },

    /**
     * 读当前后端加载的模型窗口。
     *
     * **失败保持上一个值，不清空** —— 清成 null 会让预算突然掉回默认档，
     * 而那正是撞窗口报 500 的那一档。宁可用上一次读到的。
     */
    async refreshContextWindow() {
      const cfg = get().config;
      if (!cfg) return;
      const id = get().tier === "realtime" ? cfg.realtime_provider : cfg.background_provider;
      if (!id) return;
      try {
        const n = await api.providerContextWindow(id);
        if (n !== null) set({ contextWindow: n });
      } catch {
        // 读不到就保持原样
      }
    },

    setConfig(config) {
      set({ config });
      void get().refreshContextWindow();
    },

    dismissError() {
      set({ error: null });
    },

    pushVoiceTurn(role, content, streaming = false) {
      const id = newId();
      set((state) => ({
        turns: [
          ...state.turns,
          {
            id,
            role,
            content,
            reasoning: "",
            toolCalls: [],
            streaming,
            // 标注来源，气泡上可以显示"语音"
            provider: role === "assistant" ? "voice" : undefined,
          },
        ],
      }));
      return id;
    },

    appendVoiceDelta(turnId, delta, field) {
      patchTurn(turnId, (t) => ({ ...t, [field]: t[field] + delta }));
    },

    finishVoiceTurn(turnId, opts) {
      patchTurn(turnId, (t) => ({
        ...t,
        streaming: false,
        cancelled: opts.cancelled,
        error: opts.error,
      }));
    },

    async loadModels(providerId, opts) {
      const patch = (next: ModelList) =>
        set((state) => ({ models: { ...state.models, [providerId]: next } }));

      // 保留旧列表：刷新失败时不该把已经能用的选项清空 —— **连接参数没变的前提下**。
      // 改了地址或密钥（`reset`）就不留：旧列表是旧端点的
      const previous = opts?.reset ? EMPTY_MODELS : get().models[providerId] ?? EMPTY_MODELS;
      patch({ ...previous, status: "loading", error: undefined });

      try {
        const list = await api.testProvider(providerId);
        // 顺手查一次视觉能力。**失败不能影响模型列表** —— 只有 LM Studio 有
        // 这个接口，别的后端查不到是正常的，不该让整次刷新变成 error。
        const visionModels = await api.providerVisionModels(providerId).catch(() => null);
        patch({ status: "ok", list, visionModels: visionModels ?? undefined });
      } catch (err) {
        patch({ ...previous, status: "error", error: String(err) });
      }
    },

    async loadAllModels() {
      const config = get().config;
      if (!config) return;

      // 明确缺 key 的后端就不去打了，只会白等一个必然失败的请求
      const candidates = config.providers.filter(
        (p) => !p.requires_key || p.has_key,
      );

      await Promise.all(candidates.map((p) => get().loadModels(p.id)));
    },
  };
});
