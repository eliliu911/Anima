import { KIND_LABEL, type AssetKind, type AssetView } from "../../api/assets";

/** 分类顺序固定：角色 → 场景 → 道具。和资产库那一页一致。 */
export const KIND_ORDER: AssetKind[] = ["character", "scene", "prop"];

export interface AssetGroup {
  kind: AssetKind;
  label: string;
  items: AssetView[];
}

/**
 * 按类分组。**空的类不出现**。
 *
 * # 为什么要分组
 *
 * 一条平铺的列表里，角色、场景、道具混在一起 —— 而挑引用时脑子里想的
 * 是「这一镜在哪儿、有谁、有什么东西」，本来就是分三类想的。
 * 混着列等于每次都要自己在心里再分一遍。
 *
 * # 为什么空的类不出现
 *
 * 留一个「场景 0」的空标题，会让人以为列表加载坏了。
 * 真的一个资产都没有时，由调用方给一句完整的话。
 */
export function groupAssets(assets: AssetView[]): AssetGroup[] {
  return KIND_ORDER.map((kind) => ({
    kind,
    label: KIND_LABEL[kind],
    items: assets.filter((a) => a.kind === kind),
  })).filter((g) => g.items.length > 0);
}

export type AssetViewMode = "grid" | "list";

const VIEW_KEY = "anima.studio.assetPicker.view";

/**
 * 记住上次用的是平铺还是列表。
 *
 * **读写都要 try/catch。** 隐私窗口、清过站点数据、或者某些环境下
 * 访问 `localStorage` 本身就会抛 —— 一个记偏好的功能不该让整个面板打不开。
 */
export function loadViewMode(): AssetViewMode {
  try {
    return localStorage.getItem(VIEW_KEY) === "list" ? "list" : "grid";
  } catch {
    return "grid";
  }
}

export function saveViewMode(mode: AssetViewMode): void {
  try {
    localStorage.setItem(VIEW_KEY, mode);
  } catch {
    /* 存不下就算了，下次回到默认 */
  }
}
