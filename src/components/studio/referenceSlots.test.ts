import { describe, expect, it } from "vitest";
import { requiredImages, slotCount, slotLabels } from "./referenceSlots";
import type { EngineSpec, ModeSpec } from "../../api/studio";

const mode = (id: string, images: number) => ({ id, images }) as ModeSpec;
const eng = (reference_slots: number) => ({ reference_slots }) as EngineSpec;

describe("参考生图的上传槽", () => {
  it("槽位数看引擎：Krea 2 两张、Qwen 三张", () => {
    expect(slotCount(mode("ref2i", 3), eng(2))).toBe(2);
    expect(slotCount(mode("ref2i", 3), eng(3))).toBe(3);
  });

  /** 模式上的 images（3）当必填数的话，只放一张图时「生成」点不了。 */
  it("最少一张就能生成，不是三张", () => {
    expect(requiredImages(mode("ref2i", 3))).toBe(1);
    expect(requiredImages(mode("fl2v", 2))).toBe(2);
  });

  it("槽上写场景在前、人物在后 —— 顺序就是含义", () => {
    expect(slotLabels(mode("ref2i", 3), 2)).toEqual(["场景", "人物"]);
    expect(slotLabels(mode("ref2i", 3), 3)).toEqual(["场景", "人物", "人物"]);
    expect(slotLabels(mode("i2i", 1), 1)).toBeUndefined();
  });

  it("别的模式照旧", () => {
    expect(slotCount(mode("i2i", 1), eng(0))).toBe(1);
    expect(slotCount(null, null)).toBe(0);
  });
});
