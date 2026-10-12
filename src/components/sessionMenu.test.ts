import { describe, it, expect } from "vitest";
import { isInsideMenu, MENU_CLASS } from "./sessionMenu";

/// 假的事件目标：只实现 `closest`，够用
function target(matches: string[]): unknown {
  return { closest: (sel: string) => (matches.includes(sel) ? {} : null) };
}

describe("菜单命中判定", () => {
  /// 这是整个 bug 的核心：点在菜单里必须**不关**菜单。
  /// 关了的话按钮在 mouseup 之前就被卸载，click 永远不触发 ——
  /// 用户看到的是"点删除没反应"，而后端一行日志都没有
  it("点在菜单里面算命中", () => {
    expect(isInsideMenu(target([`.${MENU_CLASS}`]))).toBe(true);
  });

  it("点在菜单外面不算", () => {
    expect(isInsideMenu(target([".treerow"]))).toBe(false);
  });

  /// document / window 没有 closest，不能因此抛异常 ——
  /// 监听是挂在 document 上的，抛了就再也关不掉菜单
  it("没有 closest 的目标算在外面，而不是崩掉", () => {
    expect(isInsideMenu(null)).toBe(false);
    expect(isInsideMenu(undefined)).toBe(false);
    expect(isInsideMenu({})).toBe(false);
    // document 就长这样：有 querySelector，没有 closest
    expect(isInsideMenu({ querySelector: () => null })).toBe(false);
  });

  /// class 名是和 CSS/JSX 对齐的约定，改一边忘另一边就悄悄退回原 bug
  it("选择器就是那个 class", () => {
    let seen = "";
    isInsideMenu({ closest: (s: string) => { seen = s; return null; } });
    expect(seen).toBe(".sessionmenu");
  });
});
