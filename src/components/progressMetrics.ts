/**
 * 等待期间的进度指标计算。
 *
 * 抽成纯函数是为了能单测 —— 这里有两条曾经出过错、而且很容易再犯的规则：
 *
 * 1. **流式期间必须用实时秒数**，不能因为已经有正文就停表。
 *    曾经的写法是 `streaming={turn.streaming && !turn.content}`，
 *    模型一吐第一个字 streaming 就变 false、计时 effect early return，
 *    秒数冻住而后面还有好几轮工具在跑 —— 看起来就是卡死。
 * 2. **精确值一到就要盖掉估算值。** 流式期间只能按字符估
 *    （`Done` 才带 usage，而 agent.rs 会吞掉中间轮次的 Done），
 *    结束后还显示估算值就是在给用户错的数。
 */

/** 字符 → token，本机实测：中文 1.92 / Rust 源码 2.82 / 混合历史 2.30，取 2.35 */
export const CHARS_PER_TOKEN = 2.35;

export interface ProgressInput {
  streaming: boolean;
  liveElapsed: number;
  durationSeconds?: number;
  streamedChars?: number;
  usage?: { prompt: number; completion: number; reasoning?: number };
  /**
   * 模型**最后一次吐出 token 时**的 elapsed 秒数。`undefined` 表示还没开始吐。
   *
   * 速率必须用它当分母，不能用 `liveElapsed`。
   *
   * LLM 说完之后，整轮并没有结束 —— 工具还在跑，`streaming` 仍然是 true，
   * `liveElapsed` 继续涨，而 `tokenCount` 已经冻住了。用 `liveElapsed` 当分母的话
   * **速率会一直往下掉**：模型早就说完了，界面上那个 tok/s 还在动，
   * 看起来像模型还在慢吞吞地生成。用户的原话是「比较 confuse」。
   */
  tokenElapsed?: number;
}

export interface Progress {
  elapsedSec: number;
  tokenCount: number;
  /** true 表示这是按字符估的，展示时要带 "~" */
  isEstimate: boolean;
  /** 仅流式期间给；0 表示不显示 */
  tokensPerSec: number;
  /**
   * 输出 token 里花在思考上的那部分。`undefined` = 这个后端不给这个数。
   *
   * **不要用 0 代替 undefined** —— "不知道"和"没思考"是两件事，
   * 显示 `91 tokens（其中 0 在思考）` 会让人以为模型没思考，
   * 而实际可能是 Ollama 压根不报这个数。
   */
  reasoningTokens?: number;
}

export function computeProgress(i: ProgressInput): Progress {
  const elapsedSec = i.streaming ? i.liveElapsed : (i.durationSeconds ?? i.liveElapsed);
  const estimated = i.streamedChars ? Math.round(i.streamedChars / CHARS_PER_TOKEN) : 0;
  const exact = i.usage?.completion;
  const tokenCount = exact ?? estimated;
  const isEstimate = exact === undefined && estimated > 0;
  // 分母是**产出 token 的那段时间**，不是整轮时长。
  // 模型说完之后工具还在跑，那段时间不该稀释速率。
  const rateWindow = i.tokenElapsed ?? elapsedSec;
  const tokensPerSec =
    i.streaming && rateWindow > 0 && tokenCount > 0 ? tokenCount / rateWindow : 0;
  return {
    elapsedSec,
    tokenCount,
    isEstimate,
    tokensPerSec,
    reasoningTokens: i.usage?.reasoning,
  };
}

/** 1234 → "1.2k"。等待时这个数涨得很快，四位数以上看着晃眼 */
export function formatTokens(n: number): string {
  return n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n);
}


/**
 * 顶部那条摘要的文案。
 *
 * ## 「跑的时候只在底部显示数字，完成后只在顶部显示」
 *
 * 同一组数字（秒数 / token / 速率）此前在三个地方同时显示：顶部摘要、
 * 「Thought for Ns」那一行、底部的 Working 行。三份一模一样的数字，
 * 白占三行竖向空间。
 *
 * 规则定成：
 *
 * - **跑的时候**：数字只在底部那条 Working 行。顶部摘要只说「做了什么」
 *   （跑了几条命令、读了几个文件），不带秒数和 token —— 那两个数在底部一直在动。
 * - **完成之后**：底部那条消失，顶部摘要带上最终数字。
 *
 * 抽成纯函数是因为这条规则很容易在改样式时被顺手破坏，而破坏之后
 * 界面只是"看起来啰嗦一点"，没人会当成 bug 报。
 */
export function buildActivitySummary(i: {
  streaming: boolean;
  reasoning: boolean;
  elapsedSec: number;
  cmdCount: number;
  searchCount: number;
  fileCount: number;
  tokenCount: number;
  tokenIsEstimate: boolean;
  tokensPerSec: number;
  reasoningTokens?: number;
}): string {
  const parts: string[] = [];
  const timeLabel = i.elapsedSec > 0 ? `${i.elapsedSec}s` : "";

  // 时间只在完成后进顶部摘要
  if (!i.streaming && timeLabel) {
    parts.push(i.reasoning ? `Thought for ${timeLabel}` : `Worked for ${timeLabel}`);
  }
  if (i.cmdCount > 0) parts.push(`ran ${i.cmdCount} command${i.cmdCount > 1 ? "s" : ""}`);
  if (i.searchCount > 0)
    parts.push(`searched ${i.searchCount} time${i.searchCount > 1 ? "s" : ""}`);
  if (i.fileCount > 0) parts.push(`referenced ${i.fileCount} file${i.fileCount > 1 ? "s" : ""}`);

  // token 同理：跑的时候底部那条在显示，这里不重复
  if (!i.streaming && i.tokenCount > 0) {
    // 思考占比只在后端真的给了这个数时显示（LM Studio 给，Anthropic / Ollama 不给）
    const think =
      i.reasoningTokens !== undefined && i.reasoningTokens > 0
        ? `（思考 ${formatTokens(i.reasoningTokens)}）`
        : "";
    const rate = i.tokensPerSec > 0 ? ` · ${i.tokensPerSec.toFixed(1)} tok/s` : "";
    parts.push(
      `${i.tokenIsEstimate ? "~" : ""}${formatTokens(i.tokenCount)} tokens${think}${rate}`,
    );
  }

  if (parts.length > 0) return parts.join(", ");
  // 兜底也不带数字 —— 底部那条负责证明"还活着"
  return i.streaming ? "正在执行…" : "Executed operations";
}
