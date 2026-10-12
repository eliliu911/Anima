import { describe, it, expect } from "vitest";

/**
 * **界面代码建节点必须给位置。**
 *
 * 不给位置的节点落在整张画布的默认坐标上 —— 无限画布拖远之后，点了按钮根本看不见新节点。
 * 添加菜单修过一次，资产库又漏了一次（用户 2026-09-14 报）。所以用源码扫一遍钉住：
 * 界面上建节点要么走 CanvasView 的 handleAddNode（按视口中心摆），要么自己算位置（贴着源节点）。
 */
const sources = import.meta.glob(["../**/*.ts", "../**/*.tsx", "!../**/*.test.ts", "!../**/*.test.tsx", "!../store/**"], {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

describe("画布新建节点的位置", () => {
  it("界面代码里没有不给位置就建节点的", () => {
    const offenders = Object.entries(sources)
      .filter(([, code]) => /addNode\(\s*"[a-zA-Z]+"\s*,\s*undefined\b/.test(code))
      // 资产库在拿不到画布给的 onAddNode 时才走兜底，这一处是有意留的
      .filter(([file, code]) => !(file.endsWith("CanvasAssetLibrary.tsx") && code.includes("if (onAddNode) onAddNode(")))
      .map(([file]) => file);
    expect(offenders).toEqual([]);
    expect(Object.keys(sources).some((f) => f.endsWith("CanvasAssetLibrary.tsx"))).toBe(true);
  });

  it("资产库用的是画布给的摆放函数", () => {
    const view = Object.entries(sources).find(([f]) => f.endsWith("/CanvasView.tsx"))![1];
    expect(view).toMatch(/<CanvasAssetLibrary[^>]*onAddNode=\{handleAddNode\}/);
  });
});
