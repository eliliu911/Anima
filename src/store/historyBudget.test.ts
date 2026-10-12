import { describe, expect, it } from "vitest";
import {
  budgetLabel,
  CHARS_PER_TOKEN,
  FALLBACK_CHARS,
  historyBudget,
  RESERVED_TOKENS,
  TIGHT_BUDGET_TOKENS,
} from "./historyBudget";

describe("从窗口反算预算", () => {
  /** 用户那台机器的真实数字：ornith-1.5-35b-a3b 加载在 33024。 */
  it("33024 的窗口算出来能塞进去", () => {
    const b = historyBudget(33024, "local");
    expect(b.source).toBe("window");
    expect(b.windowTokens).toBe(33024);
    // (33024 - 8000) * 1.9 ≈ 47547
    expect(b.chars).toBe(Math.floor((33024 - RESERVED_TOKENS) * CHARS_PER_TOKEN));
    // 换回 token 必须留得下预留量
    expect(b.chars / CHARS_PER_TOKEN + RESERVED_TOKENS).toBeLessThanOrEqual(33024);
  });

  /**
   * **换算比被实测打脸过两次，必须钉住。**
   *
   * 我按仓库里原有的「中文 1.92」算，然后拿真实请求对账：
   * 37706 字符我算成 19800 token，而 LM Studio 报的是 38014 —— 差一倍。
   * 因为这条流量混着工具 JSON schema、路径、代码，接近 1 字符 1 token。
   *
   * 往上调之前先拿一次真实请求对账，别再靠推理。
   */
  it("换算比不高于实测的 1.0", () => {
    expect(CHARS_PER_TOKEN).toBeLessThanOrEqual(1.0);
  });

  /**
   * **预留必须盖住「模型边生成边撑爆窗口」这件事。**
   *
   * 实测：`n_tokens = 33023, truncated = 1`（窗口 33024）。
   * prompt eval 只有 631 token —— 不是请求太大，是那一轮模型连着写了
   * 2666 token 的正文。预留还要同时盖住常驻工具的 schema（约 9000）
   * 和 system prompt。
   */
  it("预留量盖得住工具 schema 加一次长回复", () => {
    // 实测账：常驻工具 9118 字符 + system/技能 ~2000 + 一次回复 2666~4000
    expect(RESERVED_TOKENS).toBeGreaterThanOrEqual(15_000);
  });

  /**
   * **预算偏小要提醒。** 偏小的失败样子是「模型忘了前面说过什么」——
   * 比报错难查得多，所以宁可提前说一句。
   */
  it("预算偏紧时提示去调大窗口", () => {
    const tight = budgetLabel(historyBudget(33024, "local"));
    expect(tight).toContain("偏紧");
    expect(tight).toContain("LM Studio");
    // 窗口够大就不该唠叨
    const roomy = budgetLabel(historyBudget(131072, "local"));
    expect(roomy).not.toContain("偏紧");
    // 阈值本身要在「够几轮多轮任务」这个量级上
    expect(TIGHT_BUDGET_TOKENS).toBeGreaterThanOrEqual(18_000);
  });

  /** 33024 的窗口下，历史预算不该把窗口占到六成以上。 */
  it("真实窗口下预算留得住余量", () => {
    const b = historyBudget(33024, "local");
    expect(b.chars / CHARS_PER_TOKEN).toBeLessThanOrEqual(33024 * 0.6);
  });

  it("窗口越大预算越大", () => {
    expect(historyBudget(131072, "local").chars).toBeGreaterThan(
      historyBudget(33024, "local").chars,
    );
  });

  /** **查不到就回落，不猜。** 猜大了就是这个 bug，猜小了白浪费上下文。 */
  it("查不到窗口时回落到固定档", () => {
    for (const tier of ["local", "cloud"] as const) {
      const b = historyBudget(null, tier);
      expect(b.source).toBe("fallback");
      expect(b.chars).toBe(FALLBACK_CHARS[tier]);
      expect(b.windowTokens).toBeNull();
    }
  });

  it("窗口是 0 或负数当查不到", () => {
    expect(historyBudget(0, "local").source).toBe("fallback");
    expect(historyBudget(-1, "local").source).toBe("fallback");
    expect(historyBudget(NaN, "local").source).toBe("fallback");
  });

  /**
   * 窗口比预留还小（embedding 模型是 512）时要给个下限 ——
   * 算出 0 会让历史整个消失，那比截断更难查。
   */
  it("窗口小于预留量时不给出 0", () => {
    const b = historyBudget(512, "local");
    expect(b.chars).toBeGreaterThan(0);
    expect(b.source).toBe("window");
  });
});

describe("界面上那一行", () => {
  /** **必须说清是读来的还是默认的** —— 显示一个猜的数比不显示更糟。 */
  it("两种来源说的不是同一句", () => {
    const read = budgetLabel(historyBudget(33024, "local"));
    const guess = budgetLabel(historyBudget(null, "local"));
    expect(read).not.toBe(guess);
    expect(read).toContain("33024");
    expect(read).toContain("读到");
    expect(guess).toContain("默认");
    expect(guess, "默认档不该报一个具体窗口").not.toMatch(/\d+ token/);
  });
});

// ── 接线 ──
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const strip = (t: string) =>
  t.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^\s*\/\/.*$/gm, "");
const CHAT = strip(readFileSync(fileURLToPath(new URL("./chat.ts", import.meta.url)), "utf8"));

/**
 * **算出来的预算要真的被用上。**
 *
 * 只写一个函数不接线的话，测试全绿而发出去的还是那个写死的 48000 ——
 * 而那正是撞窗口报 500 的那个数。
 */
describe("接线", () => {
  it("组装历史时用的是算出来的预算", () => {
    expect(CHAT).toContain("computeHistoryBudget(get().contextWindow, budgetTier)");
    expect(CHAT).toContain("const historyBudget = budget.chars");
  });

  it("配置变了就重新读窗口", () => {
    expect(CHAT).toContain("refreshContextWindow()");
    // setConfig 和 refreshConfig 两条路都要触发 —— 切后端走的是前者
    const i = CHAT.indexOf("setConfig(config) {");
    expect(CHAT.slice(i, i + 140)).toContain("refreshContextWindow");
  });

  /**
   * **读失败保持上一个值。** 清成 null 会让预算突然掉回默认档，
   * 而那一档正是会撞窗口的。
   */
  it("读失败不把窗口清掉", () => {
    const i = CHAT.indexOf("async refreshContextWindow()");
    const body = CHAT.slice(i, i + 520);
    expect(body).toContain("if (n !== null) set({ contextWindow: n })");
    expect(body, "读失败把窗口清空了").not.toContain("set({ contextWindow: null })");
  });
});
