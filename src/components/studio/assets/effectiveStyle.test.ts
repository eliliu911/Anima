import { describe, expect, it } from "vitest";
import { effectiveStyle } from "./effectiveStyle";
import type { ArtDirection, StyleCatalog } from "../../../api/style";

const CAT = {
  categories: [],
  thumbs_ready: 0,
  presets: [
    {
      id: "ink",
      name: "水墨",
      positive_prompt: "ink wash painting",
      negative_prompt: "photorealistic",
    },
  ],
} as unknown as StyleCatalog;

const art = (p: Partial<ArtDirection>): ArtDirection =>
  ({ preset_id: "ink", positive_prompt: "", negative_prompt: "", updated_at: 0, ...p });

describe("有效风格", () => {
  /**
   * 留空的语义是「用预设自带的」，**不是「不要风格」**。
   * 直接读字段的话界面显示「无」，而出图时后端照样拼进去 ——
   * 看到的和发出去的不是一个东西。
   */
  it("没覆盖时回落到预设自带的词", () => {
    const s = effectiveStyle(art({}), CAT);
    expect(s.positive).toBe("ink wash painting");
    expect(s.negative).toBe("photorealistic");
    expect(s.name).toBe("水墨");
  });

  it("覆盖了就用覆盖的", () => {
    const s = effectiveStyle(art({ positive_prompt: "赛璐璐" }), CAT);
    expect(s.positive).toBe("赛璐璐");
    // 负向没覆盖，仍然回落
    expect(s.negative).toBe("photorealistic");
  });

  it("只有空白也算没覆盖", () => {
    expect(effectiveStyle(art({ positive_prompt: "   " }), CAT).positive).toBe(
      "ink wash painting",
    );
  });

  it("还没定调时是空的，不是崩掉", () => {
    expect(effectiveStyle(null, CAT)).toEqual({ name: "", positive: "", negative: "" });
  });

  /** 目录还没加载完时也不能崩 —— 那时应该显示覆盖值或空，而不是白屏。 */
  it("目录没到时用覆盖值，没覆盖就是空", () => {
    expect(effectiveStyle(art({ positive_prompt: "赛璐璐" }), null).positive).toBe("赛璐璐");
    expect(effectiveStyle(art({}), null).positive).toBe("");
  });

  it("预设 id 对不上任何预设时不假装有名字", () => {
    const s = effectiveStyle(art({ preset_id: "不存在" }), CAT);
    expect(s.name).toBe("");
    expect(s.positive).toBe("");
  });
});
