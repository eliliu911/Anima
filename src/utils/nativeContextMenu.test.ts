import { describe, expect, it } from "vitest";
import { keepsNativeMenu } from "./nativeContextMenu";

/** 假元素：`closest` 按给定的选择器命中与否。 */
const el = (inside: "input" | "textarea" | "editable" | "image") => ({
  closest: (sel: string) => {
    const hit =
      (inside === "input" && sel.includes("input")) ||
      (inside === "textarea" && sel.includes("textarea")) ||
      (inside === "editable" && sel.includes("contenteditable"));
    return hit ? {} : null;
  },
});

describe("系统右键菜单", () => {
  it("图片、卡片这些地方不弹（那些菜单项在 Tauri 里全都无效）", () => {
    expect(keepsNativeMenu(el("image"), "")).toBe(false);
    expect(keepsNativeMenu(null, "")).toBe(false);
  });

  it("输入框和可编辑区域保留 —— 复制粘贴要用", () => {
    expect(keepsNativeMenu(el("input"), "")).toBe(true);
    expect(keepsNativeMenu(el("textarea"), "")).toBe(true);
    expect(keepsNativeMenu(el("editable"), "")).toBe(true);
  });

  it("选中了文字时保留 —— 要能复制", () => {
    expect(keepsNativeMenu(el("image"), "陆川")).toBe(true);
    // 只选中了空白不算
    expect(keepsNativeMenu(el("image"), "  \n")).toBe(false);
  });
});
