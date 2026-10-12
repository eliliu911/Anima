import type { CapabilityCatalog } from "../../api/studio";

/**
 * 图片节点的模式，和视频节点一样**由连线推导**。
 *
 * | 连了几张图 | 模式 |
 * |---|---|
 * | 0 | `t2i` 文生图 |
 * | 1，模型是普通底模 | `i2i` 图生图（改这张图） |
 * | 1，模型是 Krea2Edit / Qwen 编辑 | `ref2i` 参考生图（照着这张图里的人 / 场景出新构图） |
 * | 2~3 | `ref2i` 参考生图 |
 *
 * 以前绑两张图直接抛「图生图只支持一张源图」，而后端早有参考生图 ——
 * 资产页能做「这个人放进这个场景」，画布做不了。
 */
export type ImageMode = "t2i" | "i2i" | "ref2i" | "pose" | "scene_sheet";

/** 节点上的两个开关。和连线一起决定模式 */
export interface ImageModeOptions {
  /** 「照这张图的姿势出」：连着的第一张图只取姿势（DWPose 抽骨架 + SDXL OpenPose） */
  pose?: boolean;
  /** 「场景四宫格」：不连图时出 2×2 同一场景的四个机位 */
  sceneSheet?: boolean;
}

/** 参考生图最多几张。和后端能力目录 `ref2i.images` 一致。 */
export const MAX_IMAGE_REFERENCES = 3;

/** 参考生图能用的引擎。目录读不到时的兜底，和 `studio.rs` 的 `ref2i` 一致。 */
const FALLBACK_REF2I = ["krea2_edit", "qwen21_edit"];

export function ref2iEngines(catalog?: CapabilityCatalog | null): string[] {
  return catalog?.modes.find((m) => m.id === "ref2i")?.engines ?? FALLBACK_REF2I;
}

export function inferImageMode(refCount: number, engine: string, catalog?: CapabilityCatalog | null, opts: ImageModeOptions = {}): { mode: ImageMode; reason: string } {
  // 开关只在连线对得上时生效；对不上时**照连线推导、并说出来**，不悄悄无视
  if (opts.pose && refCount >= 1) {
    return {
      mode: "pose",
      reason: "照第一张图的姿势出 —— 自动抽骨架（DWPose），用 SDXL 出图" + (refCount > 1 ? `（连了 ${refCount} 张，只用第一张）` : ""),
    };
  }
  if (opts.sceneSheet && refCount === 0) {
    return { mode: "scene_sheet", reason: "场景四宫格 —— 同一个场景一次出正面、左前、右前、深处四个机位，再切成四张" };
  }
  if (refCount === 0) return { mode: "t2i", reason: opts.pose ? "开了「照姿势」但没连图 —— 先连一张姿势参考图；现在按文生图出" : "没有连接图片 —— 文生图" };
  if (opts.sceneSheet) {
    // 宫格是纯文生，连着图没法出 —— 继续往下按连线推导
  }
  if (refCount > MAX_IMAGE_REFERENCES) {
    return { mode: "ref2i", reason: `连了 ${refCount} 张图，参考生图最多 ${MAX_IMAGE_REFERENCES} 张，请去掉几张` };
  }
  if (refCount >= 2) {
    return { mode: "ref2i", reason: `连了 ${refCount} 张图 —— 参考生图：场景图放第一张、人物图放后面` };
  }
  if (ref2iEngines(catalog).includes(engine)) {
    return { mode: "ref2i", reason: "连了 1 张图、选的是参考生图模型 —— 照着图里的人 / 场景出新构图" };
  }
  return { mode: "i2i", reason: "连了 1 张图 —— 图生图（在这张图上改）" };
}

/**
 * 参考生图的引擎。选的模型不支持参考生图时换成第一个支持的，**并说出来** ——
 * 悄悄换会让人以为自己选的模型出了这张图。
 */
export function ref2iEngine(requested: string, catalog: CapabilityCatalog): { engine: string; note?: string } {
  const allowed = ref2iEngines(catalog);
  if (allowed.includes(requested)) return { engine: requested };
  const engine = allowed[0];
  if (!engine) throw new Error("本机没有支持参考生图的模型（Krea2Edit / Qwen 编辑）");
  const label = catalog.engines.find((e) => e.id === engine)?.label ?? engine;
  return { engine, note: `参考生图不支持「${requested || "当前模型"}」，这次用的是 ${label}` };
}
