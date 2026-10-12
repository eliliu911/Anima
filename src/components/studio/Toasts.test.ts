import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import type { Toast } from "./toastStore";
import { mergeSlots, TOAST_AVOID_ATTR, TOAST_EDGE_PX, TOAST_GAP_PX, toastBottom } from "./Toasts";

const VP = { width: 1400, height: 900 };
/** 画布缩放工具条：右下角，`bottom-6 right-6`，高约 36 */
const zoomBar = { top: 900 - 24 - 36, bottom: 900 - 24, left: 1400 - 24 - 160, right: 1400 - 24 };

/**
 * 用户 2026-09-26：状态卡「需要根据右下角有没有内容来调整位置，不能在每个页面都在这儿，下面空着呢」。
 * 以前是写死 `bottom: 76px`（为画布工具条留的）。
 */
describe("通知栏的位置", () => {
  it("右下角没东西就贴底", () => {
    expect(toastBottom([], VP)).toBe(TOAST_EDGE_PX);
  });

  it("有工具条就抬到它上面", () => {
    expect(toastBottom([zoomBar], VP)).toBe(900 - zoomBar.top + TOAST_GAP_PX);
  });

  it("不在通知那一列的、在窗口上半部分的、隐藏的，都不顶", () => {
    const leftCorner = { ...zoomBar, left: 0, right: 200 };
    const topRight = { top: 10, bottom: 50, left: 1200, right: 1380 };
    const hidden = { top: 0, bottom: 0, left: 0, right: 0 };
    expect(toastBottom([leftCorner, topRight, hidden], VP)).toBe(TOAST_EDGE_PX);
  });

  /** 右下角的工具条得真的挂上标记 —— 没挂的话它照样会被状态卡盖住，而且不报错 */
  it("画布和导演世界右下角的工具条挂了让开标记", () => {
    for (const f of ["../../canvas/ui/CanvasZoomControl.tsx", "../../canvas/director/DirectorWorldEditor.tsx"]) {
      const src = readFileSync(fileURLToPath(new URL(f, import.meta.url)), "utf8");
      expect(src, f).toContain(`${TOAST_AVOID_ATTR}=""`);
    }
  });
});

const toast = (id: string, kind: Toast["kind"] = "progress"): Toast =>
  ({ id, kind, title: id, body: "", ttl: null }) as Toast;

/**
 * 「这个区域是一个状态堆叠区，有其他任务就向上挤抬起，有任务完成消失后自动落下。不是死的。」
 */
describe("通知栏是堆叠区", () => {
  it("新来的追加在最下面", () => {
    const cur = mergeSlots([], [toast("a")]);
    expect(mergeSlots(cur, [toast("a"), toast("b")]).map((x) => x.t.id)).toEqual(["a", "b"]);
  });

  it("消失的留在原位标成退场，不跳到末尾", () => {
    const cur = mergeSlots([], [toast("a"), toast("b"), toast("c")]);
    const next = mergeSlots(cur, [toast("a"), toast("c")]);
    expect(next.map((x) => [x.t.id, x.out])).toEqual([
      ["a", false],
      ["b", true],
      ["c", false],
    ]);
  });

  it("还在的换成最新内容（进度条就地变成成功）", () => {
    const cur = mergeSlots([], [toast("a")]);
    expect(mergeSlots(cur, [toast("a", "success")])[0].t.kind).toBe("success");
  });

  it("高度过渡写在 CSS 里：进场从 0 长出来，退场收拢", () => {
    const css = readFileSync(fileURLToPath(new URL("../../studio-theme.css", import.meta.url)), "utf8");
    expect(css).toMatch(/\.studio-toast-slot \{[^}]*grid-template-rows: 1fr/);
    expect(css).toMatch(/@starting-style \{\s*\.anima-studio \.studio-toast-slot \{\s*grid-template-rows: 0fr/);
    expect(css).toMatch(/\.studio-toast-slot\[data-out\] \{\s*grid-template-rows: 0fr/);
  });
});
