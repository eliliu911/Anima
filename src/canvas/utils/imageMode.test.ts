import { describe, it, expect } from "vitest";
import { inferImageMode, ref2iEngine } from "./imageMode";
import { canvasCatalogFixture } from "./modelCatalog.fixture";

describe("图片节点模式由连线推导", () => {
  it("0 张文生、1 张图生、2~3 张参考生", () => {
    expect(inferImageMode(0, "zimage").mode).toBe("t2i");
    expect(inferImageMode(1, "zimage").mode).toBe("i2i");
    expect(inferImageMode(2, "zimage").mode).toBe("ref2i");
    expect(inferImageMode(3, "zimage").reason).toContain("场景图放第一张");
  });

  it("1 张图但选了参考生图模型 = 参考生图", () => {
    expect(inferImageMode(1, "krea2_edit").mode).toBe("ref2i");
    expect(inferImageMode(1, "qwen21_edit").mode).toBe("ref2i");
  });

  it("超过 3 张说清楚上限", () => {
    expect(inferImageMode(4, "krea2_edit").reason).toContain("最多 3 张");
  });

  it("选的模型不支持参考生图时换一个，并说出来", () => {
    const r = ref2iEngine("zimage", canvasCatalogFixture);
    expect(["krea2_edit", "qwen21_edit"]).toContain(r.engine);
    expect(r.note).toContain("zimage");
    expect(ref2iEngine("qwen21_edit", canvasCatalogFixture)).toEqual({ engine: "qwen21_edit" });
  });

  it("照姿势：连了图才生效，只用第一张", () => {
    expect(inferImageMode(1, "zimage", null, { pose: true }).mode).toBe("pose");
    expect(inferImageMode(2, "zimage", null, { pose: true }).reason).toContain("只用第一张");
    const none = inferImageMode(0, "zimage", null, { pose: true });
    expect(none.mode).toBe("t2i");
    expect(none.reason).toContain("先连一张姿势参考图");
  });

  it("场景四宫格：不连图才生效", () => {
    expect(inferImageMode(0, "zimage", null, { sceneSheet: true }).mode).toBe("scene_sheet");
    expect(inferImageMode(1, "zimage", null, { sceneSheet: true }).mode).toBe("i2i");
  });
});
