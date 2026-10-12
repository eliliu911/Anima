/**
 * Token 用量的显示：数字怎么写、费用怎么说。
 *
 * **费用永远带「约」**：只按官方公布过的价估，Azure 以账单为准。
 * 里面有认不出价的云端模型时写「起」—— 不能让人以为那就是全部。
 */
import type { UsageTotals } from "../../../api/tokenUsage";

/**
 * 缩写：860 / 1.1k / 12k / 1.2M / 12M。
 *
 * 用户 2026-09-29：「卡片这里挤没了……直接用缩写。比如 1.1k, 12k」—— 原来写「1.4 万 tok · $0.08」，把镜数那一行挤成了省略号。
 */
export function formatTokens(n: number): string {
  const short = (v: number, unit: string) => `${v < 10 ? v.toFixed(1).replace(/\.0$/, "") : Math.round(v)}${unit}`;
  if (n < 1000) return String(n);
  if (n < 1_000_000) return short(n / 1000, "k");
  return short(n / 1_000_000, "M");
}

export function totalTokens(t: UsageTotals): number {
  return t.prompt_tokens + t.completion_tokens;
}

/** 「约 $1.23」；有认不出价的写「约 $1.23 起」；全是本地模型（不花钱）回空串 */
export function formatUsd(t: UsageTotals): string {
  if (t.usd <= 0 && t.priced_all) return "";
  const v = t.usd < 0.01 && t.usd > 0 ? "<$0.01" : `$${t.usd.toFixed(2)}`;
  return `约 ${v}${t.priced_all ? "" : " 起"}`;
}

/** 分集卡片右下角那一小行：只写 token 缩写，**不写钱**（钱在概览页底下那张表里）。没用过模型回空串（不显示「0」，免得以为坏了） */
export function episodeUsageLabel(t: UsageTotals | undefined): string {
  if (!t || totalTokens(t) === 0) return "";
  return `${formatTokens(totalTokens(t))} tok`;
}
