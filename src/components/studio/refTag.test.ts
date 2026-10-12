import { describe, expect, it } from "vitest";
import { refTag } from "../../api/assets";

/**
 * 前端插的标签必须能被 Rust 侧 `parse_asset_refs` 认出来。
 * 两边格式漂了不会报错 —— 标签会当普通文字进提示词，
 * 模型把方括号和「角色」两个字画进画面。
 */
describe("引用标签格式", () => {
  it("三种类型的标签和 Rust 侧的 tag_label 一致", () => {
    // Rust: AssetKind::tag_label() → "角色" / "场景" / "道具"
    expect(refTag("character", "老茶师")).toBe("[角色:老茶师]");
    expect(refTag("scene", "老茶馆")).toBe("[场景:老茶馆]");
    expect(refTag("prop", "铜壶")).toBe("[道具:铜壶]");
  });

  it("用的是半角冒号", () => {
    // Rust 两种冒号都收，但我们自己插的统一半角。
    expect(refTag("character", "x")).toContain(":");
    expect(refTag("character", "x")).not.toContain("：");
  });
});
