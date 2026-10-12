import { describe, it, expect } from "vitest";
import { WEETODD_VIDEO_SIZES, WEETODD_RATIO_VIDEO_SIZES } from "./CanvasVideoResolutionSelect";

describe("WeeTodd/MLX 视频分辨率预设与规格", () => {
  it("LTX 2.5 包含 768x512, 1344x768, 1920x1088 (1080p)", () => {
    const ltxSizes = WEETODD_VIDEO_SIZES.ltx;
    expect(ltxSizes).toBeDefined();
    expect(ltxSizes.length).toBe(3);

    expect(ltxSizes[0]).toEqual({
      label: "768×512",
      width: 768,
      height: 512,
      verified: true,
      secs: 121,
      peak_gb: 22.88,
    });

    expect(ltxSizes[1]).toEqual({
      label: "1344×768",
      width: 1344,
      height: 768,
      verified: true,
      secs: 473,
      peak_gb: 123.45,
    });

    expect(ltxSizes[2]).toEqual({
      label: "1920×1088 1080p",
      width: 1920,
      height: 1088,
      verified: true,
      secs: 839,
      peak_gb: 103.78,
    });
  });

  it("MiniMax H3 包含 640x384 (预览) 与 768x512", () => {
    const h3Sizes = WEETODD_VIDEO_SIZES.h3;
    expect(h3Sizes).toBeDefined();
    expect(h3Sizes.length).toBe(2);

    expect(h3Sizes[0]).toEqual({
      label: "640×384 预览",
      width: 640,
      height: 384,
      verified: true,
      secs: 222,
      peak_gb: 15.81,
    });

    expect(h3Sizes[1]).toEqual({
      label: "768×512",
      width: 768,
      height: 512,
      verified: false,
      secs: null,
      peak_gb: null,
    });
  });

  it("所有预设分辨率的长宽均严格满足 32 像素整倍数对齐约束", () => {
    for (const engine of ["ltx", "h3"]) {
      for (const size of WEETODD_VIDEO_SIZES[engine]) {
        expect(size.width % 32).toBe(0);
        expect(size.height % 32).toBe(0);
      }
    }
  });
});

describe("多画幅比例支持 (16:9, 9:16, 1:1, 4:3, 3:4, 21:9)", () => {
  it("LTX 与 H3 完整覆盖 6 种主流画幅比例", async () => {
    const { WEETODD_RATIO_VIDEO_SIZES, VIDEO_RATIO_PRESETS } = await import("./CanvasVideoResolutionSelect");
    expect(VIDEO_RATIO_PRESETS.map((p) => p.id)).toEqual(["16:9", "9:16", "1:1", "4:3", "3:4", "21:9"]);

    for (const engine of ["ltx", "h3"]) {
      const ratios = WEETODD_RATIO_VIDEO_SIZES[engine];
      expect(ratios).toBeDefined();
      for (const ratio of ["16:9", "9:16", "1:1", "4:3", "3:4", "21:9"]) {
        expect(ratios[ratio]?.length).toBeGreaterThanOrEqual(2);
        // 所有比例下的每一个尺寸均必须严格符合 32 像素物理网格约束且不超过 1920
        for (const s of ratios[ratio]) {
          expect(s.width % 32).toBe(0);
          expect(s.height % 32).toBe(0);
          expect(s.width).toBeLessThanOrEqual(1920);
          expect(s.height).toBeLessThanOrEqual(1920);
          expect(s.width).toBeGreaterThanOrEqual(64);
          expect(s.height).toBeGreaterThanOrEqual(64);
        }
      }
    }
  });

  it("inferVideoRatio 正确推导不同画幅尺寸", async () => {
    const { inferVideoRatio } = await import("./CanvasVideoResolutionSelect");
    expect(inferVideoRatio(768, 512)).toBe("16:9");
    expect(inferVideoRatio(1344, 768)).toBe("16:9");
    expect(inferVideoRatio(512, 768)).toBe("9:16");
    expect(inferVideoRatio(768, 1344)).toBe("9:16");
    expect(inferVideoRatio(512, 512)).toBe("1:1");
    expect(inferVideoRatio(768, 768)).toBe("1:1");
    expect(inferVideoRatio(1024, 768)).toBe("4:3");
    expect(inferVideoRatio(672, 512)).toBe("4:3");
    expect(inferVideoRatio(768, 1024)).toBe("3:4");
    expect(inferVideoRatio(512, 672)).toBe("3:4");
    expect(inferVideoRatio(1792, 768)).toBe("21:9");
    expect(inferVideoRatio(1216, 512)).toBe("21:9");
  });
});

// H3 的长边上限是 1344、32 对齐（后端 `weetodd::fit_dimensions`）。超了不报错，
// 后端静默缩小 —— 界面显示 1792×768，实际出 1344×576。
describe("H3 每一档尺寸都在后端上限内", () => {
  it("长边 ≤ 1344 且宽高都是 32 的倍数", () => {
    for (const [ratio, sizes] of Object.entries(WEETODD_RATIO_VIDEO_SIZES.h3)) {
      for (const s of sizes) {
        expect(Math.max(s.width, s.height), `${ratio} ${s.label}`).toBeLessThanOrEqual(1344);
        expect(s.width % 32, `${ratio} ${s.label}`).toBe(0);
        expect(s.height % 32, `${ratio} ${s.label}`).toBe(0);
      }
    }
  });

  it("21:9 有一档正好是 1344×576", () => {
    expect(WEETODD_RATIO_VIDEO_SIZES.h3["21:9"].some((s) => s.width === 1344 && s.height === 576)).toBe(true);
  });
});
