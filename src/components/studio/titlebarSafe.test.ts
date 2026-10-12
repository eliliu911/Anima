import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

/**
 * 窗口用的是 `TitleBarStyle::Overlay` + `hidden_title` —— macOS 的红绿灯
 * **浮在内容上**，占据左上角大约 (13..72, 14..34)。
 *
 * 左侧那一列的第一个元素必须避开这块区域，否则会贴着红绿灯。
 * 这条只能从源码上钉：症状是纯视觉的（挤在一起），不报任何错，
 * 代码评审时也极容易看漏。
 */
function read(rel: string): string {
  return readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");
}

describe("红绿灯避让", () => {
  it.each([
    ["全局侧栏", "./StudioSidebar.tsx"],
    ["六步导航", "./project/StepRail.tsx"],
    ["系列项目的两组导航", "./project/SeriesRail.tsx"],
  ])("%s 顶上留了红绿灯的位置", (_name, path) => {
    expect(read(path)).toContain("studio-titlebar-safe");
  });

  /** 全屏覆盖层 portal 到 body，拿不到 --studio-titlebar-h，只能写死 —— 至少要盖过 34px */
  it("导演世界编辑器（全屏覆盖层）顶上也留了位置", () => {
    const src = read("../../canvas/director/DirectorWorldEditor.tsx");
    const m = src.match(/data-tauri-drag-region[\s\S]{0,200}?pt-\[(\d+)px\]/);
    expect(m).not.toBeNull();
    expect(Number(m![1])).toBeGreaterThanOrEqual(38);
  });

  it("避让高度盖得住红绿灯", () => {
    const css = readFileSync(
      fileURLToPath(new URL("../../studio-theme.css", import.meta.url)),
      "utf8",
    );
    const m = css.match(/--studio-titlebar-h:\s*(\d+)px/);
    expect(m).not.toBeNull();
    // 红绿灯下沿约 34px。留不够的话就是「看起来挤」，留太多是一块空白。
    const h = Number(m![1]);
    expect(h).toBeGreaterThanOrEqual(34);
    expect(h).toBeLessThanOrEqual(48);
  });

  it("右侧页头不需要避让 —— 它从 x=212 开始，够不着红绿灯", () => {
    // 反向钉一下：别哪天顺手给右边也加上，白留一块空白。
    expect(read("./library/AssetLibraryPage.tsx")).not.toContain("studio-titlebar-safe");
  });
});
