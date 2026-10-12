import type { AssetView } from "../../../api/assets";

/**
 * 第 3 步顶部的 全部 / 角色 / 场景 / 道具 页签。
 *
 * # 为什么要分类，而不是平铺
 *
 * 这一页要回答的是「这一集有几个人、在哪儿、用了什么东西」。
 * 六张卡混在一起平铺，角色和道具长得一样，得逐张读名字才数得出来。
 *
 * # 计数为什么算两层的总和
 *
 * 页签上的数字是**本集 + 系列**，因为切页签时两块都跟着筛。
 * 只数其中一层的话，点进「场景」看到两块加起来有 3 个，
 * 而页签上写着 1 —— 数字和眼睛看到的对不上。
 */
export type KindFilter = "all" | "character" | "scene" | "prop";

export const KIND_TABS: { key: KindFilter; label: string }[] = [
  { key: "all", label: "全部" },
  { key: "character", label: "角色" },
  { key: "scene", label: "场景" },
  { key: "prop", label: "道具" },
];

/** `all` 原样返回，**不复制** —— 下游只读。 */
export function filterKind(list: AssetView[], kind: KindFilter): AssetView[] {
  if (kind === "all") return list;
  return list.filter((a) => a.kind === kind);
}

/** 四个页签各有多少。`all` 是总数，不是其它三个之和的近似。 */
export function countByKind(list: AssetView[]): Record<KindFilter, number> {
  const out: Record<KindFilter, number> = { all: list.length, character: 0, scene: 0, prop: 0 };
  for (const a of list) {
    if (a.kind === "character" || a.kind === "scene" || a.kind === "prop") out[a.kind] += 1;
  }
  return out;
}
