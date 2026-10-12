import { describe, expect, it } from "vitest";
import src from "./BaseNodeWrapper.tsx?raw";

/**
 * 2026-10-11 用户截图：v0.43.21 加了左侧「+」之后，画布上原有的连线全都没接到节点上，线头悬在节点左边 36 像素的半空。
 * 左侧「+」本身是个输入口；没记 `targetHandle` 的老连线接节点的**第一个**输入口 —— 它排在 children 前面就抢了第一。
 */
describe("左侧「+」不能抢在节点自己的输入口前面", () => {
  it("在 children 之后渲染", () => {
    const children = src.indexOf("{children}");
    const leftPlus = src.indexOf('side="left"');
    expect(children).toBeGreaterThan(0);
    expect(leftPlus).toBeGreaterThan(children);
  });
});
