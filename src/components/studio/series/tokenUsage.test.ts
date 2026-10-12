import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { episodeUsageLabel, formatTokens, formatUsd } from "./tokenUsage";

const t = (p: number, c: number, usd: number, priced_all = true) => ({ calls: 1, prompt_tokens: p, completion_tokens: c, reasoning_tokens: 0, usd, priced_all });

describe("Token 用量的显示", () => {
  it("数字用 k / M 缩写（用户要的：1.1k、12k）", () => {
    expect(formatTokens(860)).toBe("860");
    expect(formatTokens(1_100)).toBe("1.1k");
    expect(formatTokens(2_000)).toBe("2k");
    expect(formatTokens(12_345)).toBe("12k");
    expect(formatTokens(140_400)).toBe("140k");
    expect(formatTokens(1_234_567)).toBe("1.2M");
    expect(formatTokens(12_345_678)).toBe("12M");
  });

  it("费用永远带「约」；有认不出价的写「起」；全是本地模型不写钱", () => {
    expect(formatUsd(t(1, 1, 1.234))).toBe("约 $1.23");
    expect(formatUsd(t(1, 1, 1.234, false))).toBe("约 $1.23 起");
    expect(formatUsd(t(1, 1, 0))).toBe("");
    expect(formatUsd(t(1, 1, 0.001))).toBe("约 <$0.01");
  });

  it("分集卡片：没用过模型不显示；用过只写 token 缩写，不写钱（卡片放不下）", () => {
    expect(episodeUsageLabel(undefined)).toBe("");
    expect(episodeUsageLabel(t(0, 0, 0))).toBe("");
    expect(episodeUsageLabel(t(10_000, 4_000, 0.08))).toBe("14k tok");
    expect(episodeUsageLabel(t(900, 200, 0))).toBe("1.1k tok");
  });
});

/** 2026-09-29 用户：「在概览里，最下面，添加一个 Token 统计…在每个分集卡片右侧空白位置，增加一个 Token 使用量」 */
describe("概览页接线", () => {
  const src = readFileSync(fileURLToPath(new URL("./SeriesOverview.tsx", import.meta.url)), "utf8");

  it("最下面是 Token 统计，分集卡片上有这一集的用量", () => {
    expect(src.lastIndexOf("<TokenUsagePanel")).toBeGreaterThan(src.indexOf("+ 加一集"));
    expect(src).toContain("episodeUsageLabel(usage?.episodes[e.id])");
  });

  it("任务跑完自动刷新，不用重进页面", () => {
    expect(src).toContain("listen<Job>(JOB_EVENT");
  });
});
