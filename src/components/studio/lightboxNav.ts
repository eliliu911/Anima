import { kindOf } from "./resultFilter";
import type { StudioResult } from "./useStudioStore";

/**
 * 灯箱的下标计算。
 *
 * # 为什么单拎出来
 *
 * **能看大图的只是结果里的一部分** —— 正在跑的、失败的都没有图。
 * 所以「点了第 3 张卡」和「灯箱里的第 3 张」不是一回事：一屏里混着
 * 一个 RUNNING 的时候，直接把卡片下标传给灯箱就会打开错的那张。
 *
 * 而且它错得很安静 —— 打开的是一张真图，只是不是你点的那张，
 * 图多起来根本看不出来。所以这里全部按 **id** 定位，不按位置。
 */

/**
 * 一张能看详情的结果。
 *
 * `path` 收窄成必填 —— 这正是「能不能看」的判据，
 * 让类型替我们保证「进了这个列表就一定有文件」。
 */
export interface Viewable extends Omit<StudioResult, "path"> {
  path: string;
  isVideo: boolean;
}

/** 挑出有图可看的那些，保持原来的顺序。 */
export function viewables(results: StudioResult[]): Viewable[] {
  return results
    .filter((r) => r.status === "done" && !!r.path)
    .map((r) => ({ ...r, path: r.path!, isVideo: kindOf(r) === "video" }));
}

/**
 * 某个结果在「可看列表」里排第几。不在列表里（还在跑 / 失败了）回 `null`。
 */
export function indexOfId(list: Viewable[], id: string): number | null {
  const i = list.findIndex((v) => v.id === id);
  return i < 0 ? null : i;
}

/**
 * 左右翻页。**到头就停，不绕回去** —— 绕回去的话，一直按 → 会以为
 * 还有更多，实际在原地转圈。
 */
export function step(index: number, delta: number, count: number): number {
  if (count <= 0) return 0;
  return Math.min(count - 1, Math.max(0, index + delta));
}
