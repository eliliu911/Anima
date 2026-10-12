import { describe, expect, it } from "vitest";
import { coverGradient, GRADIENT_COUNT } from "./coverGradient";

describe("渐变封面", () => {
  it("同一个 id 永远是同一个渐变", () => {
    // 每次渲染换颜色的话，用户没法靠颜色记住项目。
    const a = coverGradient("drama_123");
    for (let i = 0; i < 5; i++) expect(coverGradient("drama_123")).toBe(a);
  });

  it("不同 id 会散开，不是全撞在一个上", () => {
    const seen = new Set<string>();
    for (let i = 0; i < 60; i++) seen.add(coverGradient(`drama_${i}`));
    expect(seen.size).toBe(GRADIENT_COUNT);
  });

  it("空字符串也给得出一个渐变，不返回 undefined", () => {
    // 没标题没 id 的草稿项目会走到这里 —— 返回 undefined 的话卡片是透明的。
    expect(coverGradient("")).toContain("linear-gradient");
  });

  it("只用 token，不写死色值", () => {
    // 换主题时颜色要跟着走。
    for (let i = 0; i < 20; i++) {
      const g = coverGradient(`x${i}`);
      expect(g).toContain("var(--studio-");
      expect(g).not.toMatch(/#[0-9a-f]{3,8}\b/i);
    }
  });
});
