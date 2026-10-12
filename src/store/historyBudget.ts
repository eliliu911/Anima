/**
 * 历史预算：从**模型的上下文窗口**反算，而不是写死一个字符数。
 *
 * # 修的是什么
 *
 * 原来是 `MAX_HISTORY_CHARS = { local: 48_000, cloud: 120_000 }` ——
 * 只按 provider 的 local/cloud 分两档，**不问模型的窗口有多大**。
 *
 * 而这两个数的单位是**字符**，窗口的单位是 **token**。注释里记着本机实测的
 * 换算：中文对话 1.92、Rust 源码 2.82、混合 2.30。按平均的 2.35 算，
 * 48000 字符 ≈ 20400 token，落在 32768 的窗口里很宽裕 —— 那是设计时的算法。
 *
 * **但纯中文要按 1.92 算，同样 48000 字符是 25000 token。** 再加上没算进
 * 这 48000 的东西：system prompt、滚动摘要、技能正文（一份带 9 个 references
 * 的 SKILL.md 不短）、工具定义，以及**要留给回复的位置**（正在写 2500-3000 字
 * 的章节正文）。撞穿 33024 毫无难度。
 *
 * 实测症状：上游返回一个 HTML 的 500，不是结构化错误，日志里一条都没有。
 */

/**
 * 字符 / token 换算比。
 *
 * # 这个数被实测打脸过两次
 *
 * 仓库里原有的实测是「中文对话 1.92 / Rust 源码 2.82 / 混合 2.30」，
 * 我先按 1.9 算 —— 然后拿真实请求对了一次账：
 *
 *     我算的：37706 字符 ÷ 1.9 ≈ 19800 token
 *     LM Studio 报的：38014 token
 *
 * **差了一倍。** 原因是这条流量根本不是中文散文：它混着工具定义的
 * JSON schema（英文 + 大量标点）、文件路径、代码片段、
 * 以及压缩后的工具调用签名。那些的 token 密度接近 1 字符 1 token。
 *
 * 所以取 **1.0**：这是安全边界，宁可预算偏小（多压缩几轮历史），
 * 也不能偏大 —— 偏大的表现是上游 500，整轮白跑。
 *
 * 真要精确只能问 tokenizer，而 LM Studio 没有 tokenize 接口（试过，
 * `/api/v0/tokenize` 返回 `Unexpected endpoint`）。
 */
export const CHARS_PER_TOKEN = 1.0;

/**
 * 窗口里**不归历史用**的那部分（token）。
 *
 * # 为什么是 12000 而不是 8000
 *
 * 8000 是我猜的，然后被实测打脸：
 *
 *     task 47740 | stop processing: n_tokens = 33023, truncated = 1
 *
 * 窗口 33024，用满到 33023。发生的事不是「请求太大进不去」——
 * prompt eval 只有 631 token（大部分被 KV cache 复用）；是模型**边生成
 * 边把窗口撑满**：那一轮它连着写了 2666 个 token 的小说正文。
 *
 * 所以预留必须同时盖住三样：
 *
 * - **工具定义**：分层之后常驻 11 个约 9000 字符 ≈ 9000 token
 *   （按 1.0 算），这一项以前完全没算
 * - **system prompt + 技能正文**：几千 token
 * - **模型的回复**：写长正文时一次 2000-4000 token
 *
 账算全是这样（33024 的窗口）：
 *
 *     常驻工具 schema   ≈  9100   实测 11 个工具 9118 字符
 *     system + 技能正文 ≈  2000
 *     一次长回复        ≈  4000   实测那轮生成了 2666
 *     ─────────────────────────
 *     预留              = 15100 → 取 15000
 *     剩给历史          ≈ 18000 token
 *
 * **18000 token 的历史对多轮任务偏紧**，但这是 33024 这个窗口的物理现实，
 * 不是可以靠调系数解决的。`ornith-1.5-35b-a3b` 的
 * `max_context_length` 是 262144 —— **真正的解法是在 LM Studio 里
 * 把加载上下文调大**，[`budgetLabel`] 会在预算偏小时提示这件事。
 */
export const RESERVED_TOKENS = 15_000;

/**
 * 历史预算低于这个数就该提醒用户调大窗口（token）。
 *
 * **33024 的窗口算下来是 18024，正好落在线下** —— 那不是巧合，
 * 是那个窗口配这套工具确实紧。20000 勉强够几轮带工具的多轮任务；
 * 再低就会频繁丢上下文，而丢上下文的表现是
 * 「模型忘了前面说过什么」，比报错更难查。
 */
export const TIGHT_BUDGET_TOKENS = 20_000;

/** 查不到窗口时的回落。就是原来那两档，不改。 */
export const FALLBACK_CHARS = {
  local: 48_000,
  cloud: 120_000,
} as const;

export interface Budget {
  /** 历史最多拼多少字符 */
  chars: number;
  /** 这个数是怎么来的 —— **界面要显示它**，用户才知道 Anima 知不知道他的窗口 */
  source: "window" | "fallback";
  /** 读到的窗口（token）。`source === "fallback"` 时为 null */
  windowTokens: number | null;
}

/**
 * 算这次能拼多少字符历史。
 *
 * `windowTokens` 为 null（查不到）时回落到固定档 —— **不猜**。
 * 猜一个窗口出来，猜大了就是现在这个 bug，猜小了白白浪费上下文。
 */
export function historyBudget(
  windowTokens: number | null,
  tier: "local" | "cloud",
): Budget {
  if (windowTokens === null || !Number.isFinite(windowTokens) || windowTokens <= 0) {
    return { chars: FALLBACK_CHARS[tier], source: "fallback", windowTokens: null };
  }
  const usable = windowTokens - RESERVED_TOKENS;
  // 窗口比预留还小（比如 embedding 模型的 512）—— 给一个能用的下限，
  // 而不是 0 或负数。0 会让历史整个消失，那比截断更难查。
  const chars = Math.max(2_000, Math.floor(usable * CHARS_PER_TOKEN));
  return { chars, source: "window", windowTokens };
}

/**
 * 界面上那一行。**要说清是读来的还是默认的**，而且**预算偏小要提醒**。
 *
 * 偏小时的失败样子是「模型忘了前面说过什么」—— 比报错难查得多，
 * 所以宁可提前说一句。
 */
export function budgetLabel(b: Budget): string {
  if (b.source === "fallback") {
    return `上下文预算 ${Math.round(b.chars / 1000)}k 字符（默认值 —— 没读到这个后端的窗口）`;
  }
  const base = `上下文预算 ${Math.round(b.chars / 1000)}k 字符（按读到的窗口 ${b.windowTokens} token 算）`;
  if (b.chars / CHARS_PER_TOKEN < TIGHT_BUDGET_TOKENS) {
    return `${base}　⚠️ 偏紧：工具定义和回复要占掉一大半，多轮任务容易丢上下文。去 LM Studio 把加载上下文调大`;
  }
  return base;
}
