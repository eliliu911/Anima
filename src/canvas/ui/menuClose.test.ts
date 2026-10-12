import { describe, it, expect, vi } from "vitest";
import { closeWithToggle } from "./menuClose";

describe("用 toggle 关菜单", () => {
  it("开着就关", () => {
    const toggle = vi.fn();
    closeWithToggle(true, toggle);
    expect(toggle).toHaveBeenCalledTimes(1);
  });

  /**
   * **关着的时候不能调** —— 调了就是把它打开。
   * 这正是「加完节点菜单不关」那个 bug 的机制。
   */
  it("已经关了就不动它", () => {
    const toggle = vi.fn();
    closeWithToggle(false, toggle);
    expect(toggle).not.toHaveBeenCalled();
  });

  /** 连调两次也只关一次 —— 幂等是这个函数存在的全部理由。 */
  it("调两次和调一次一样", () => {
    let open = true;
    const toggle = vi.fn(() => {
      open = !open;
    });
    closeWithToggle(open, toggle);
    closeWithToggle(open, toggle);
    expect(open).toBe(false);
    expect(toggle).toHaveBeenCalledTimes(1);
  });
});
