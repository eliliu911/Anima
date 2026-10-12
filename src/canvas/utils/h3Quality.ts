import { KEYFRAME_QUALITIES, QUALITIES, REFERENCE_QUALITIES } from "../../components/studio/storyboard/renderOptions";
import type { VideoMode } from "./videoMode";

/**
 * 画布视频节点的 **H3 加速档**（用户 2026-10-08：「昨天的加速节点在画布里没有」）。
 *
 * 和工作台「出片设置」**同一套 id、同一份说明**（`renderOptions.QUALITIES`），按链路给不同的可选项和默认值 ——
 * 和工作台一样分首帧 / 参考两组（10-07 对照：首帧 LightX2V 最好；参考 Larry 6 步定稿）：
 *
 * | 链路 | 可选 | 默认 |
 * |---|---|---|
 * | 首帧 / 首尾帧（FL2VA） | LightX2V 4 / Larry 4 / Larry 6 / 精细 19 | LightX2V 4 |
 * | 图片参考（Ref2VA） | Larry 6 / LightX2V 4 / Larry 4 / 精细 19 | Larry 6 |
 * | 文生（T2VA） | Larry 4 / Larry 6 / 精细 19 | Larry 4（= 以前按尺寸自动的那档）|
 *
 * 文生不给 LightX2V：它是拿首尾帧训的，文生没对照过。全能参考（原片复刻）走自己那条验过的配方，这里不管。
 */
export type CanvasH3Quality = "lightx2v" | "standard" | "staged" | "dense";

const T2V_QUALITIES: { id: CanvasH3Quality; tag?: string }[] = [{ id: "standard", tag: "推荐" }, { id: "staged" }, { id: "dense" }];

/** 这条链路能选哪几档（第一个是默认）。全能参考 / 动作迁移回空：不给选 */
export function qualityChoices(mode: VideoMode): { id: CanvasH3Quality; tag?: string }[] {
  if (mode === "i2v" || mode === "fl2v") return KEYFRAME_QUALITIES as { id: CanvasH3Quality; tag?: string }[];
  if (mode === "r2v") return REFERENCE_QUALITIES as { id: CanvasH3Quality; tag?: string }[];
  if (mode === "t2v") return T2V_QUALITIES;
  return [];
}

/** 实际用的那一档：节点存的这条链路能选就照用，否则用这条链路的默认（换了模式不会带着不合适的档过去） */
export function effectiveQuality(stored: string | undefined, mode: VideoMode): CanvasH3Quality | undefined {
  const choices = qualityChoices(mode);
  if (!choices.length) return undefined;
  return choices.find((c) => c.id === stored)?.id ?? choices[0].id;
}

/** 名字和大白话说明（和工作台出片设置同一份） */
export function qualityInfo(id: CanvasH3Quality) {
  return QUALITIES.find((q) => q.id === id)!;
}
