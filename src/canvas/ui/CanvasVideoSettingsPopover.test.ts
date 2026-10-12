import { describe, it, expect } from "vitest";
import {
  RATIO_OPTIONS,
  RESOLUTION_OPTIONS,
  inferCurrentRatio,
  inferCurrentResolution,
  resolveVideoDimensions,
} from "./CanvasVideoSettingsPopover";

describe("CanvasVideoSettingsPopover 比例与清晰度推导", () => {
  it("定义了 7 种比例选项和 3 种清晰度选项", () => {
    expect(RATIO_OPTIONS.map((r) => r.id)).toEqual([
      "Auto",
      "16:9",
      "4:3",
      "1:1",
      "3:4",
      "9:16",
      "21:9",
    ]);
    expect(RESOLUTION_OPTIONS).toEqual(["480P", "720P", "1080P"]);
  });

  it("支持从 width 和 height 正确推导当前画幅比例", () => {
    expect(inferCurrentRatio(undefined, 768, 512)).toBe("16:9");
    expect(inferCurrentRatio(undefined, 512, 768)).toBe("9:16");
    expect(inferCurrentRatio(undefined, 512, 512)).toBe("1:1");
    expect(inferCurrentRatio(undefined, 672, 512)).toBe("4:3");
    expect(inferCurrentRatio(undefined, 512, 672)).toBe("3:4");
    expect(inferCurrentRatio(undefined, 1216, 512)).toBe("21:9");
    expect(inferCurrentRatio("Auto", 512, 768)).toBe("Auto");
  });

  it("支持从 width 和 height 正确推导当前清晰度，4K 自动回退至 1080P", () => {
    expect(inferCurrentResolution(undefined, 640, 384)).toBe("480P");
    expect(inferCurrentResolution(undefined, 768, 512)).toBe("720P");
    expect(inferCurrentResolution(undefined, 1344, 768)).toBe("1080P");
    expect(inferCurrentResolution("4K", 1344, 768)).toBe("1080P");
  });

  it("能够根据引擎、比例和清晰度换算实际像素尺寸", () => {
    // H3 和工作台出片设置同一套尺寸（2026-10-08：以前画布 720P 实际是 768×512、1080P 实际是 1344×768）
    expect(resolveVideoDimensions("h3", "16:9", "480P")).toEqual({ width: 896, height: 512 });
    expect(resolveVideoDimensions("h3", "16:9", "720P")).toEqual({ width: 1312, height: 736 });
    expect(resolveVideoDimensions("h3", "9:16", "480P")).toEqual({ width: 512, height: 896 });
    expect(resolveVideoDimensions("h3", "16:9", "1080P"), "H3 没有 1080P，按 720P").toEqual({ width: 1312, height: 736 });
    // H3 按实际尺寸说清晰度：老节点存着「1080P」其实是 1344×768
    expect(inferCurrentResolution("1080P", 1344, 768, "h3")).toBe("720P");
    expect(inferCurrentResolution(undefined, 640, 384, "h3")).toBe("480P");
    expect(inferCurrentResolution(undefined, 896, 512, "h3")).toBe("480P");
    // 老节点：标签写 720P、实际 768×512 —— 按实际尺寸算 480P（用户 10-08 截图）
    expect(inferCurrentResolution("720P", 768, 512, "h3")).toBe("480P");
    expect(inferCurrentResolution("480P", 1312, 736, "h3")).toBe("720P");

    const ltx_16_9_720 = resolveVideoDimensions("ltx", "16:9", "720P");
    expect(ltx_16_9_720).toEqual({ width: 1344, height: 768 });
  });
});
