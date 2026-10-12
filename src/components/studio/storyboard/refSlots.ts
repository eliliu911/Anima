/**
 * 分镜详情里手动增删参考图槽位。
 *
 * 用户 2026-09-27：E01 第 3 镜传送链上挂的仿生人应该和女主同一型号，出片却是一排光头假人 ——
 * 镜头表生成时只把它们写进了「场景物件」的文字里，没挂星澜的图；而界面上参考图那一栏是只读的，
 * 用户想补也补不上。
 *
 * 槽位编号就是提示词里 `<Picture N>` 的 N，增删之后**按现有顺序从 1 重排**。
 * 改了槽位这一镜的提示词就过期了（槽位在指纹里），要重新合成 —— 这是对的：编号全变了。
 */
import type { RefSlot } from "../../../api/cinema";

/** 能手动选的用途。起始帧 / 结束帧不在这里 —— 那两个在「关键帧」一栏挂 */
export const MANUAL_USAGES: RefSlot["usage"][] = ["identity", "outfit", "geography", "composition", "scale", "effect", "style"];

const renumber = (slots: RefSlot[]): RefSlot[] =>
  [...slots].sort((a, b) => a.order - b.order).map((s, i) => ({ ...s, order: i + 1 }));

export function addSlot(slots: RefSlot[], slot: Omit<RefSlot, "order">): RefSlot[] {
  return renumber([...slots, { ...slot, order: Number.MAX_SAFE_INTEGER }]);
}

export function removeSlot(slots: RefSlot[], index: number): RefSlot[] {
  const sorted = [...slots].sort((a, b) => a.order - b.order);
  return renumber(sorted.filter((_, i) => i !== index));
}

/**
 * 新加的槽位默认用途：地方 → 空间布局；东西、出场的人 → 身份；
 * **不出场的人 → 造型状态**（只借外形，比如同型号的仿生人群演）—— 后端对「身份」却不出场的会报问题。
 */
export function defaultUsage(kind: string, inCast = true): RefSlot["usage"] {
  if (kind === "scene") return "geography";
  return kind === "character" && !inCast ? "outfit" : "identity";
}
