import { describe, expect, it } from "vitest";
import { createHistory } from "./history";

describe("导演世界撤销", () => {
  it("撤销回到上一步，重做回来", () => {
    const h = createHistory<number>();
    h.record(1, "place", 0);
    h.record(2, "delete", 1000);
    expect(h.undo(3)).toBe(2);
    expect(h.undo(2)).toBe(1);
    expect(h.undo(1)).toBeNull();
    expect(h.redo(1)).toBe(2);
    expect(h.redo(2)).toBe(3);
    expect(h.redo(3)).toBeNull();
  });

  /** 拖滑块、连按方向键：一次撤销回到拖之前，而不是按几十下 */
  it("短时间内连续的同类操作合并成一步", () => {
    const h = createHistory<number>(100, 500);
    h.record(10, "nudge", 0);
    h.record(11, "nudge", 100);
    h.record(12, "nudge", 200);
    expect(h.undo(13)).toBe(10);
    expect(h.canUndo()).toBe(false);
  });

  it("隔久了或者换了操作就不合并", () => {
    const h = createHistory<number>(100, 500);
    h.record(10, "nudge", 0);
    h.record(11, "nudge", 2000);
    h.record(12, "rotate", 2100);
    expect(h.undo(13)).toBe(12);
    expect(h.undo(12)).toBe(11);
    expect(h.undo(11)).toBe(10);
  });

  it("新操作清掉重做栈", () => {
    const h = createHistory<number>();
    h.record(1, "a", 0);
    h.undo(2);
    h.record(1, "b", 5000);
    expect(h.canRedo()).toBe(false);
  });

  it("超过上限丢最早的", () => {
    const h = createHistory<number>(2, 0);
    h.record(1, "a", 0);
    h.record(2, "b", 10);
    h.record(3, "c", 20);
    expect(h.undo(4)).toBe(3);
    expect(h.undo(3)).toBe(2);
    expect(h.undo(2)).toBeNull();
  });
});
