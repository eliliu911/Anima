import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const SRC = readFileSync(fileURLToPath(new URL("./DimensionSelector.tsx", import.meta.url)), "utf8");

describe("画布尺寸选择器", () => {
  /** 这里曾经写死一份带 4K 的表，而 4K 在这台机器上跑不了。图片尺寸只能从后端拿。 */
  it("图片尺寸从后端能力目录来，不是写死的表", () => {
    expect(SRC).toContain("studioCapabilities");
    expect(SRC, "又出现写死的图片分辨率表了").not.toMatch(/IMAGE_RESOLUTIONS|IMAGE_DIMENSION_MAP|calculateImageDimensions/);
    expect(SRC, "又出现 4K 选项了").not.toMatch(/["']4k["']/i);
  });

  it("没实测的尺寸标出来，不借别的尺寸的秒数", () => {
    expect(SRC).toContain("未实测");
  });
});
