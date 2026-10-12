import type { EngineSpec, ModeSpec } from "../../api/studio";

/**
 * 参考生图（`ref2i`）的上传槽规则。**三件事是分开的**，混在一个数里就会出错：
 *
 * - **槽位数**看引擎：Krea 2 参考生图两张、Qwen 三张。模式上的 `images` 只是上限。
 * - **最少几张**是 1：只给一张人物图或一张场景图都能出。
 *   直接拿模式的 `images`（3）当必填数的话，只放一张图时「生成」按钮点不了。
 * - **槽上写什么**：场景在前、人物在后 —— 两个引擎都是按这个顺序训练的，
 *   顺序就是含义。写成三个「图片」的话用户不知道该往哪个槽放哪张。
 */
export function slotCount(mode: ModeSpec | null, engine: EngineSpec | null): number {
  if (!mode) return 0;
  if (mode.id === "ref2i") return Math.min(mode.images, engine?.reference_slots || mode.images);
  return mode.images;
}

export function requiredImages(mode: ModeSpec | null): number {
  if (!mode) return 0;
  return mode.id === "ref2i" ? 1 : mode.images;
}

export function slotLabels(mode: ModeSpec | null, count: number): string[] | undefined {
  if (mode?.id !== "ref2i") return undefined;
  return ["场景", "人物", "人物"].slice(0, count);
}
