import { describe, expect, it, vi, beforeEach } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

// vitest 跑在 node 里：没有 window / localStorage，给一份内存版
const mem = new Map<string, string>();
const storage = { getItem: (k: string) => mem.get(k) ?? null, setItem: (k: string, v: string) => void mem.set(k, v) };
vi.stubGlobal("localStorage", storage);
vi.stubGlobal("window", { localStorage: storage });

vi.mock("@xyflow/react", () => ({
  useReactFlow: () => ({ zoomTo: vi.fn(), getZoom: () => 1, fitView: vi.fn() }),
  useViewport: () => ({ x: 0, y: 0, zoom: 0.85 }),
}));

import { REDUCE_MOTION_KEY, TOOLBAR_COLLAPSED_KEY } from "../store/canvasToolStore";

/**
 * 服务端渲染时 zustand 读的是 store **创建时**的状态（getServerSnapshot），测试里 setState 改不到渲染结果 ——
 * 所以先把开关写进 localStorage，再重新加载模块，让 store 在创建时读到它。
 */
async function render(flags: { collapsed?: boolean; reduceMotion?: boolean } = {}) {
  mem.set(TOOLBAR_COLLAPSED_KEY, flags.collapsed ? "1" : "0");
  mem.set(REDUCE_MOTION_KEY, flags.reduceMotion ? "1" : "0");
  vi.resetModules();
  const { CanvasZoomControl } = await import("./CanvasZoomControl");
  return renderToStaticMarkup(createElement(CanvasZoomControl, { showMiniMap: false, onToggleMiniMap: () => {} }));
}

// 用户 2026-09-25：「底部工具栏里添加个减少动态效果的按钮……增加个收起展开按钮……收起时只保留缩放，回到中心和地图」
describe("画布底栏", () => {
  beforeEach(() => mem.clear());

  it("展开：工具都在，多了「减少动态效果」", async () => {
    const html = await render();
    for (const t of ["移动工具", "抓手工具", "触控板平移", "自动整理画布", "网格吸附", "隐藏画布连线", "减少动态效果", "缩小", "放大", "适合屏幕", "小地图"]) {
      expect(html, t).toContain(t);
    }
    expect(html).toContain('aria-label="收起工具栏"');
  });

  it("收起：只留缩放、百分比、适合屏幕、小地图", async () => {
    const html = await render({ collapsed: true });
    for (const t of ["缩小", "85%", "放大", "适合屏幕", "小地图"]) expect(html, t).toContain(t);
    for (const t of ["移动工具", "抓手工具", "触控板平移", "自动整理画布", "网格吸附", "隐藏画布连线", "减少动态效果"]) {
      expect(html, t).not.toContain(t);
    }
    expect(html).toContain('aria-label="展开工具栏"');
  });

  it("两个开关都记住（下次打开还是这样）", async () => {
    vi.resetModules();
    const { useCanvasToolStore } = await import("../store/canvasToolStore");
    const s = useCanvasToolStore.getState();
    s.toggleReduceMotion();
    s.toggleToolbarCollapsed();
    expect(useCanvasToolStore.getState().reduceMotion).toBe(true);
    expect(localStorage.getItem(REDUCE_MOTION_KEY)).toBe("1");
    expect(localStorage.getItem(TOOLBAR_COLLAPSED_KEY)).toBe("1");
    useCanvasToolStore.getState().toggleReduceMotion();
    expect(localStorage.getItem(REDUCE_MOTION_KEY)).toBe("0");
  });

  it("「减少动态效果」开着时按钮是按下状态", async () => {
    expect(await render({ reduceMotion: true })).toMatch(/aria-pressed="true"[^>]*title="减少动态效果：开/);
  });
});
