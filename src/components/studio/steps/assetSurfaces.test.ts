/**
 * 资产的两个面板各自在哪儿活着。
 *
 * # 为什么要扫源码
 *
 * 2026-09-18：音色字段加在了 `AssetInspector` 的「人物」页签里，
 * 而「资产」这一步点卡片直接开的是 `AssetWorkbench` ——
 * 那一页的 `AssetInspector` **根本打不开**（`selectedId` 除了被置空，
 * 没有任何地方设值）。用户：「角色绑定语音的入口我没看到」。
 *
 * 这类 bug 编译得过、测试全绿、界面也不报错，**只是那块 UI 永远不出现**。
 * 唯一能自动发现的办法就是看「渲染它的那一页有没有办法打开它」。
 */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const read = (p: string) => readFileSync(new URL(p, import.meta.url), "utf8");

describe("资产面板的可达性", () => {
  /**
   * **渲染了就必须打得开。** 一个 useState 的 setter 只出现在
   * 「置空」的位置，说明那块 UI 是死的。
   */
  it("AssetsStep 里渲染的 AssetInspector 必须有办法打开", () => {
    const src = read("./AssetsStep.tsx");
    if (!src.includes("<AssetInspector")) return; // 不渲染就没这个问题
    // 找 setSelectedId 的所有调用，至少要有一处传的不是 null
    const calls = [...src.matchAll(/setSelectedId\(([^)]*)\)/g)].map((m) => m[1].trim());
    expect(calls.length).toBeGreaterThan(0);
    expect(
      calls.some((a) => a !== "null" && a !== ""),
      `AssetsStep 渲染了 AssetInspector，但 setSelectedId 只被这样调用过：${calls.join(" / ")}` +
        " —— 只会置空，等于那个面板永远打不开。要么给它一个打开的入口，要么别渲染它。",
    ).toBe(true);
  });

  /**
   * 角色的音色入口必须在**用户点得到的那个面板**上。
   *
   * 「资产」这一步点角色卡片 → `AssetWorkbench`。只加在资产库的详情面板里，
   * 走这条路的用户一辈子看不到。
   */
  it("角色工作台上必须有音色入口 —— 那是「资产」这一步点卡片真正打开的面板", () => {
    const src = read("../assets/AssetWorkbench.tsx");
    expect(src).toContain("withVoiceId");
    expect(src).toContain("音色");
  });
});
