import type { CapabilityCatalog } from "../../api/studio";

export function canonicalCanvasEngine(id: string) {
  // 以前这里把老 id `chroma1` 认成 `chroma`；Chroma 2026-10-01 下架了，两个都不再认
  return id;
}

export function canvasEngines(catalog: CapabilityCatalog, mode: string) {
  const allowed = catalog.modes.find((m) => m.id === mode)?.engines ?? [];
  return catalog.engines.filter((e) => allowed.includes(e.id));
}

export function resolveCanvasEngine(catalog: CapabilityCatalog, mode: string, requested: string) {
  const engine = canonicalCanvasEngine(requested || (mode.endsWith("2v") ? catalog.default_video_engine : catalog.default_image_engine));
  if (!canvasEngines(catalog, mode).some((e) => e.id === engine)) {
    throw new Error(`模型 ${requested || engine} 不支持 ${mode} 或已不可用，请重新选择模型`);
  }
  return engine;
}
