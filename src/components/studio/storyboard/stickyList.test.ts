import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const read = (rel: string) => readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");
const classOf = (src: string, marker: string) => (src.slice(src.indexOf(marker)).match(/className="([^"]+)"/) ?? [])[1]?.split(/\s+/) ?? [];

/**
 * 分镜页左边的镜头列表自己滚（用户 2026-09-29）；顶上的步骤卡片固定、不跟着页面滚（10-06）。
 * 以前列表 sticky 贴顶、高度按整个滚动区量：页面没滚下去时列表底部被卡片顶出屏幕，右边详情短的话永远拉不到底。
 */
describe("分镜页：卡片固定，列表和详情各自滚", () => {
  const src = read("../steps/StoryboardStep.tsx");

  it("整页一屏高、不长高；卡片和提醒栏不缩", () => {
    const root = classOf(src, "<div className=\"relative flex h-full");
    for (const c of ["flex", "h-full", "min-h-0", "flex-col"]) expect(root).toContain(c);
    expect(classOf(src, '<section className="grid shrink-0')).toContain("shrink-0");
    // 提醒栏展开了也只占一截、自己滚，不把列表挤没
    const notice = classOf(src, '<div className="max-h-[30vh]');
    for (const c of ["shrink-0", "overflow-y-auto"]) expect(notice).toContain(c);
  });

  it("网格吃掉剩下的高度，两栏各自滚（缺 min-h-0 的话网格被内容撑高、又变成整页滚）", () => {
    const grid = src.slice(src.indexOf('gridTemplateColumns: "minmax(240px, 300px)') - 80);
    expect(grid).toContain('className="grid min-h-0 flex-1 gap-3"');
    expect(grid).toContain('gridTemplateRows: "minmax(0, 1fr)"');
    const list = classOf(src, "<div ref={listEl}");
    for (const c of ["min-h-0", "overflow-y-auto", "overscroll-contain"]) expect(list).toContain(c);
    expect(list).not.toContain("sticky");
    // 选中框是按钮外面的 1.5px 描边，自己滚的容器会把边缘外面裁掉 —— 四周要留出来（09-29 左边被切掉一截）
    expect(list).toContain("p-0.5");
    expect(list).toContain("-ml-0.5");
    const detail = src.slice(src.indexOf("{selected && (\n            <ShotDetail") - 120, src.indexOf("{selected && (\n            <ShotDetail"));
    expect(detail).toContain('className="min-h-0 overflow-y-auto overscroll-contain');
  });

  it("每一镜带 data-shot-id，选中变了会滚到它（从队列跳过来时它可能在列表外面）", () => {
    expect(src).toContain("data-shot-id={s.id}");
    expect(src).toContain("revealSelected(listEl.current, selectedId)");
  });
});
