import { describe, it, expect } from "vitest";
import {
  ASPECT_RATIOS,
  DEFAULT_IMAGE_SIZE,
  calculateVideoDimensions,
  imageSizesByRatio,
  inferRatio,
  inferVideoResolution,
  pickSizeForRatio,
  videoRatioIds,
  type CatalogSize,
} from "./dimensionUtils";

const sz = (w: number, h: number, verified = false, secs: number | null = null): CatalogSize => ({
  label: `${w}×${h}`, width: w, height: h, verified, secs,
});

describe("画幅尺寸与分辨率工具 (dimensionUtils)", () => {
  describe("inferRatio 取最近的一档", () => {
    it("常规画幅", () => {
      expect(inferRatio(1024, 576)).toBe("16:9");
      expect(inferRatio(1280, 736)).toBe("16:9");
      expect(inferRatio(576, 1024)).toBe("9:16");
      expect(inferRatio(1024, 1024)).toBe("1:1");
      expect(inferRatio(1152, 864)).toBe("4:3");
      expect(inferRatio(864, 1152)).toBe("3:4");
      expect(inferRatio(1536, 640)).toBe("21:9");
    });

    /** 以前 1216×832 落不进任何一档的容差，返回默认「16:9」—— 一张 3:2 的图标着 16:9。 */
    it("1216×832 是 3:2，不是 16:9", () => {
      expect(inferRatio(1216, 832)).toBe("3:2");
    });

    /** 以前按顺序取第一个落进容差的：832×1216（0.684）先碰到 3:4 就返回了，其实最近的是 2:3。 */
    it("832×1216 是 2:3，不是先碰到的 3:4", () => {
      expect(inferRatio(832, 1216)).toBe("2:3");
    });
  });

  describe("图片尺寸只从后端来", () => {
    const catalog = [sz(1024, 1024, true, 45), sz(1216, 832, true, 45), sz(1024, 576), sz(1280, 736), sz(2560, 1440, true, 242)];

    it("按画幅分组，组内从小到大", () => {
      const g = imageSizesByRatio(catalog);
      const wide = g.find((x) => x.ratio.id === "16:9")!;
      expect(wide.sizes.map((s) => `${s.width}x${s.height}`)).toEqual(["1024x576", "1280x736", "2560x1440"]);
    });

    it("只返回有尺寸的画幅 —— 不给用户一个点了没反应的按钮", () => {
      const ids = imageSizesByRatio(catalog).map((x) => x.ratio.id);
      expect(ids).not.toContain("21:9");
      expect(ids).toContain("3:2");
    });

    it("切画幅时优先挑实测过的", () => {
      expect(pickSizeForRatio(catalog, "16:9", { width: 1024, height: 576 })).toMatchObject({ width: 2560, height: 1440 });
    });

    it("这个画幅没有尺寸时不瞎给", () => {
      expect(pickSizeForRatio(catalog, "21:9")).toBeNull();
    });

    it("默认尺寸是 32 的倍数（LTX 的 VAE 下采样要求，后端有同样的测试）", () => {
      expect(DEFAULT_IMAGE_SIZE.width % 32).toBe(0);
      expect(DEFAULT_IMAGE_SIZE.height % 32).toBe(0);
    });
  });

  describe("视频尺寸（本次没动）", () => {
    it("16:9 / 9:16", () => {
      expect(calculateVideoDimensions("16:9", "720p")).toEqual({ width: 1280, height: 720 });
      expect(calculateVideoDimensions("9:16", "1080p")).toEqual({ width: 1080, height: 1920 });
      expect(inferVideoResolution(1920, 1080)).toBe("1080p");
    });

    /** 新加的 3:2 / 2:3 视频表里没有 —— 视频模式不能出现这两个按钮，点了会静默退回 1280×720。 */
    it("视频画幅按钮不含视频表里没有的 3:2 / 2:3", () => {
      expect(videoRatioIds()).not.toContain("3:2");
      expect(ASPECT_RATIOS.map((a) => a.id)).toContain("3:2");
    });
  });
});
