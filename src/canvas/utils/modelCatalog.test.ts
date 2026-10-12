import { describe, expect, it } from "vitest";
import { canvasEngines, resolveCanvasEngine } from "./modelCatalog";
import { canvasCatalogFixture as catalog } from "./modelCatalog.fixture";

describe("canvas model catalog", () => {
  it("only lists models allowed in the chosen mode", () => {
    expect(canvasEngines(catalog, "r2v").map((e) => e.id)).toEqual(["h3"]);
    expect(canvasEngines(catalog, "unsupported")).toEqual([]);
  });
  it("uses the backend default only when no model was specified", () => {
    expect(resolveCanvasEngine(catalog, "t2v", "")).toBe("h3");
    expect(() => resolveCanvasEngine(catalog, "t2v", "sulphur")).toThrow("不可用");
  });
  // Chroma 2026-10-01 下架：老 id `chroma1` 以前会被认成 `chroma`，现在两个都不认、明确报错
  it("retired Chroma ids fail loudly instead of being substituted", () => {
    expect(() => resolveCanvasEngine(catalog, "t2i", "chroma1")).toThrow("不可用");
    expect(() => resolveCanvasEngine(catalog, "t2i", "chroma")).toThrow("不可用");
    // 引擎在、模式不对：照样拦
    expect(() => resolveCanvasEngine(catalog, "r2v", "zimage")).toThrow("不支持");
  });
  it("does not resurrect an engine removed from the backend directory", () => {
    // 拿 fixture 里真有的引擎来删 —— 删一个本来就不在的，这条就成了恒真的空测试
    expect(resolveCanvasEngine(catalog, "t2i", "krea2")).toBe("krea2");
    const removed = { ...catalog, engines: catalog.engines.filter((e) => e.id !== "krea2") };
    expect(() => resolveCanvasEngine(removed, "t2i", "krea2")).toThrow("不可用");
  });
});
