import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { engineMonogram, engineSecsLabel } from "./CanvasEngineSelect";

/** 用户 2026-09-24：「跟 LibTV 一样做成统一的独立上下箭头，模型显示图标和截断，减少长度」 */
describe("底栏的模型选择", () => {
  it("缩写图标：一眼分得清是哪家", () => {
    expect(engineMonogram("h3")).toBe("H3");
    expect(engineMonogram("ltx")).toBe("LTX");
    expect(engineMonogram("zimage")).toBe("Z");
    expect(engineMonogram("krea2_edit")).toBe("K2");
    expect(engineMonogram("mystery")).toBe("MY");
  });

  it("单次耗时：秒 / 分钟", () => {
    expect(engineSecsLabel(45)).toBe("45s");
    expect(engineSecsLabel(1020)).toBe("17min");
    expect(engineSecsLabel(0)).toBe("");
  });

  it("不再是系统下拉框（系统的双箭头和旁边几个下拉长得不一样）；名字截断、限宽", () => {
    const src = readFileSync(new URL("./CanvasEngineSelect.tsx", import.meta.url), "utf8");
    expect(src).not.toContain("<select");
    expect(src).toContain("<ChevronDown");
    expect(src).toContain("truncate");
    expect(src).toContain("max-w-[150px]");
  });
});
