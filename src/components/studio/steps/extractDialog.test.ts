import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { defaultPick } from "./ExtractDialog";

/**
 * 提取弹窗的几条性质。渲染测不了（没有 jsdom），从源码上钉。
 */
const SRC = readFileSync(
  fileURLToPath(new URL("./ExtractDialog.tsx", import.meta.url)),
  "utf8",
);

describe("提取弹窗", () => {
  /**
   * 用户判断的是**系列的名单会变成什么样**，所以必须给
   * `系列当前 → 应用后`。数字怎么算（已有的不加一）见 seriesCopy.test.ts。
   */
  it("每一类都给出「系列 当前 → 应用后」的对比", () => {
    expect(SRC).toContain("系列 {count.before} → {count.after}");
    expect(SRC).toContain("seriesCount(items, picks, existing, key)");
  });

  /**
   * 读不到系列现有的时**不显示数字**。0 是一个具体的断言，
   * 而那时我们其实不知道 —— 显示成「0 → 3」会让人以为系列是空的。
   */
  it("读不到系列现有的时不假装是 0", () => {
    expect(SRC).toContain("existing ? seriesCount(");
    expect(SRC).toMatch(/\{count && \(/);
  });

  /** 「入库」听起来像永久写进资产库；「应用到本集」在默认进系列之后是错的。 */
  it("按钮不叫入库，也不再说「应用到本集」", () => {
    expect(SRC, "按钮文案里还有「入库」").not.toMatch(/["`]入库/);
    expect(SRC).not.toContain("应用到本集");
  });

  /**
   * 默认进系列，**包括已存在的**（后端并进去，不会多建）；只有顺带出现的默认不要（见下面那组）。
   */
  it("默认进系列，已有的不跳过", () => {
    expect(SRC).toContain("entities.map((e) => [entityKey(e), defaultPick(e)])");
    expect(defaultPick({ incidental: false })).toBe("series");
    expect(SRC).not.toContain("filter((e) => !e.exists)");
  });

  /** 只留本集的要真的传给后端，不然点了也白点 */
  it("只留本集的键传给后端", () => {
    expect(SRC).toMatch(/picks\[entityKey\(e\)\] === "episode"\)[\s\S]{0,40}\.map\(entityKey\)/);
    expect(SRC).toContain("cinemaImportEntities(projectId, episodeId, chosen, episodeOnly)");
  });

  it("系列里已有的标出来", () => {
    expect(SRC).toContain("已有");
    expect(SRC).toContain("characterBase(e)}` : k)");
  });

  it("描述收进悬浮提示，不占版面", () => {
    expect(SRC).toContain("e.description || e.persona");
  });
});

/** 2026-09-30 用户：EP02 一次进了 20 个龙套和一次性道具的资产（邻居甲、铁棍、电锅），这些不需要单独建 */
describe("顺带出现的默认不要", () => {
  it("模型标了 incidental 的默认「不要」，其余照旧进系列；旧数据没有这个字段 = 进系列", () => {
    expect(defaultPick({ incidental: true })).toBe("skip");
    expect(defaultPick({ incidental: false })).toBe("series");
    expect(defaultPick({})).toBe("series");
    expect(SRC).toContain("entities.map((e) => [entityKey(e), defaultPick(e)])");
    // 标出来，用户才知道为什么它默认被划掉了
    expect(SRC).toContain("{e.incidental && (");
    expect(SRC).toContain("标「顺带」的（龙套、一次性的普通物件）默认不要");
  });
});

