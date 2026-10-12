import { describe, it, expect } from "vitest";
import { computeProgress, formatTokens, CHARS_PER_TOKEN , buildActivitySummary} from "../progressMetrics";

describe("等待期间的进度指标", () => {
  it("流式期间用实时秒数，不看 durationSeconds", () => {
    // 关键回归：曾经因为"有正文就当结束"而停表，秒数冻住、界面像卡死
    const p = computeProgress({ streaming: true, liveElapsed: 37, durationSeconds: 11 });
    expect(p.elapsedSec).toBe(37);
  });

  it("结束后用 durationSeconds 定格", () => {
    const p = computeProgress({ streaming: false, liveElapsed: 99, durationSeconds: 11 });
    expect(p.elapsedSec).toBe(11);
  });

  it("没有 durationSeconds 时退回实时值，而不是显示 0", () => {
    const p = computeProgress({ streaming: false, liveElapsed: 8 });
    expect(p.elapsedSec).toBe(8);
  });

  it("流式期间按字符估 token，并标记为估算", () => {
    const p = computeProgress({ streaming: true, liveElapsed: 10, streamedChars: 2350 });
    expect(p.tokenCount).toBe(Math.round(2350 / CHARS_PER_TOKEN));
    expect(p.isEstimate).toBe(true);
  });

  it("精确值一到就盖掉估算值，且不再标 ~", () => {
    // Done 带回来的 usage 是唯一精确的数；继续显示估算值等于给错的数
    const p = computeProgress({
      streaming: false,
      liveElapsed: 10,
      streamedChars: 2350,
      usage: { prompt: 500, completion: 812 },
    });
    expect(p.tokenCount).toBe(812);
    expect(p.isEstimate).toBe(false);
  });

  it("一个字都还没到时不显示 token，也不标估算", () => {
    // 纯 prefill 阶段：唯一能动的只有秒数，硬凑一个 0 tokens 是噪声
    const p = computeProgress({ streaming: true, liveElapsed: 5 });
    expect(p.tokenCount).toBe(0);
    expect(p.isEstimate).toBe(false);
    expect(p.tokensPerSec).toBe(0);
  });

  it("速率只在流式期间给", () => {
    const live = computeProgress({ streaming: true, liveElapsed: 10, streamedChars: 2350 });
    expect(live.tokensPerSec).toBeCloseTo(100, 0);
    const done = computeProgress({
      streaming: false,
      liveElapsed: 10,
      durationSeconds: 10,
      usage: { prompt: 1, completion: 1000 },
    });
    expect(done.tokensPerSec).toBe(0);
  });

  it("秒数为 0 时不算速率，避免除零出 Infinity", () => {
    const p = computeProgress({ streaming: true, liveElapsed: 0, streamedChars: 1000 });
    expect(Number.isFinite(p.tokensPerSec)).toBe(true);
    expect(p.tokensPerSec).toBe(0);
  });

  it("后端给了 reasoning_tokens 就带出来", () => {
    const p = computeProgress({
      streaming: false,
      liveElapsed: 10,
      usage: { prompt: 60, completion: 91, reasoning: 73 },
    });
    expect(p.reasoningTokens).toBe(73);
  });

  it("后端不给时是 undefined，不能变成 0", () => {
    // "不知道"和"没思考"是两件事。显示 `91 tokens（思考 0）` 会让人以为
    // 模型没思考，而实际可能是 Ollama 压根不报这个数。
    const p = computeProgress({
      streaming: false,
      liveElapsed: 10,
      usage: { prompt: 60, completion: 91 },
    });
    expect(p.reasoningTokens).toBeUndefined();
    expect(p.reasoningTokens).not.toBe(0);
  });

  it("流式期间没有精确 usage，思考数也就没有", () => {
    // 实测：LM Studio 的 usage 只在最后一帧给（90 帧里只有第 90 帧带），
    // 这是 OpenAI 流式协议的标准行为 —— include_usage 是"末尾追加一帧"，
    // 不是逐帧累计。所以流式期间拿不到，只能按字符估总数。
    const p = computeProgress({ streaming: true, liveElapsed: 5, streamedChars: 1000 });
    expect(p.reasoningTokens).toBeUndefined();
    expect(p.isEstimate).toBe(true);
  });

  it("千位以上缩写成 k", () => {
    expect(formatTokens(999)).toBe("999");
    expect(formatTokens(1000)).toBe("1.0k");
    expect(formatTokens(12345)).toBe("12.3k");
  });
});

describe("顶部摘要：数字只在完成后出现", () => {
  const base = {
    reasoning: true,
    elapsedSec: 12,
    cmdCount: 2,
    searchCount: 1,
    fileCount: 3,
    tokenCount: 1400,
    tokenIsEstimate: true,
    tokensPerSec: 20,
    reasoningTokens: 300,
  };

  it("跑的时候不带秒数、token 和速率", () => {
    // 那三个数在底部那条 Working 行里一直在动；顶部再显示一遍就是三份
    // 一模一样的数字白占竖向空间
    const s = buildActivitySummary({ ...base, streaming: true });
    expect(s).not.toMatch(/12s/);
    expect(s).not.toMatch(/tok/);
    expect(s).not.toMatch(/思考/);
    // 但「做了什么」要说
    expect(s).toContain("ran 2 commands");
    expect(s).toContain("referenced 3 files");
  });

  it("完成之后带上最终数字", () => {
    const s = buildActivitySummary({ ...base, streaming: false });
    expect(s).toContain("Thought for 12s");
    expect(s).toContain("1.4k tokens");
    expect(s).toContain("思考 300");
    expect(s).toContain("20.0 tok/s");
  });

  it("跑的时候还没有任何动作，兜底也不带数字", () => {
    const s = buildActivitySummary({
      ...base,
      streaming: true,
      cmdCount: 0,
      searchCount: 0,
      fileCount: 0,
    });
    expect(s).toBe("正在执行…");
  });

  it("没有 reasoning 时说 Worked 而不是 Thought", () => {
    const s = buildActivitySummary({ ...base, streaming: false, reasoning: false });
    expect(s).toContain("Worked for 12s");
    expect(s).not.toContain("Thought");
  });

  it("后端不给 reasoning_tokens 时不显示思考占比", () => {
    // LM Studio 给，Anthropic / Ollama 不给 —— 不给的时候不能显示成 0
    const s = buildActivitySummary({ ...base, streaming: false, reasoningTokens: undefined });
    expect(s).not.toContain("思考");
    expect(s).toContain("1.4k tokens");
  });
});

/**
 * **速率的分母是「产出 token 的那段时间」，不是整轮时长。**
 *
 * LLM 说完之后整轮并没有结束 —— 工具还在跑，`streaming` 仍然是 true，
 * `liveElapsed` 继续涨而 `tokenCount` 已经冻住。用 `liveElapsed` 当分母的话，
 * 界面上的 tok/s 会一直往下掉，看起来像模型还在慢吞吞地生成。
 * 用户的原话是「模型其实已经完成了这次 token……但是后面的 token 速率一直在动」。
 */
describe("工具执行期间的速率", () => {
  it("模型说完后速率冻住，不随工具执行时间下滑", () => {
    const atEnd = computeProgress({
      streaming: true,
      liveElapsed: 10,
      streamedChars: 1175,
      tokenElapsed: 10,
    });
    // 又跑了 50 秒工具，token 一个没多
    const later = computeProgress({
      streaming: true,
      liveElapsed: 60,
      streamedChars: 1175,
      tokenElapsed: 10,
    });
    expect(later.tokensPerSec).toBeCloseTo(atEnd.tokensPerSec, 5);
    expect(later.tokenCount).toBe(atEnd.tokenCount);
    // 但秒数要继续走 —— 那是"还活着"的唯一证据，不能一起冻住
    expect(later.elapsedSec).toBe(60);
  });

  it("没给 tokenElapsed 时退回整轮时长，不崩", () => {
    const p = computeProgress({ streaming: true, liveElapsed: 20, streamedChars: 470 });
    expect(p.tokensPerSec).toBeGreaterThan(0);
  });

  it("非流式时不给速率", () => {
    const p = computeProgress({
      streaming: false,
      liveElapsed: 60,
      durationSeconds: 10,
      usage: { prompt: 1, completion: 500 },
      tokenElapsed: 10,
    });
    expect(p.tokensPerSec).toBe(0);
  });
});
