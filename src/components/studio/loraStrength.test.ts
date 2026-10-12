import { describe, expect, it } from "vitest";
import { formatStrength, LORA_DEFAULT, LORA_MAX, LORA_STEP } from "./LoraPanel";

describe("LoRA 权重显示", () => {
  /**
   * 步进 0.05 的浮点累加会冒出 `0.7000000000000001`。直接渲染的话，
   * 那一格的宽度会随着数字长度左右跳，滑杆跟着晃。
   */
  it("浮点噪声被压成两位", () => {
    expect(formatStrength(0.1 + 0.05 + 0.05 + 0.5)).toBe("0.70");
    expect(formatStrength(0.7000000000000001)).toBe("0.70");
  });

  it("宽度恒定 —— 滑杆不会左右晃", () => {
    const widths = new Set(
      [0, LORA_DEFAULT, 1, LORA_MAX].map((v) => formatStrength(v).length),
    );
    expect(widths.size, `宽度不一致：${[...widths]}`).toBe(1);
  });

  /** 滑杆上每一格都要落在能显示出来的值上，否则拖到某一格会看到怪数字。 */
  it("滑杆每一格都显示得干净", () => {
    for (let v = 0; v <= LORA_MAX + 1e-9; v += LORA_STEP) {
      const s = formatStrength(v);
      expect(s, `${v} 显示成了 ${s}`).toMatch(/^\d\.\d{2}$/);
      // 显示出来的值读回去必须还是同一格
      expect(Math.abs(Number(s) - v)).toBeLessThan(LORA_STEP / 2);
    }
  });

  it("默认值落在滑杆范围内、且正好是一格", () => {
    expect(LORA_DEFAULT).toBeGreaterThan(0);
    expect(LORA_DEFAULT).toBeLessThanOrEqual(LORA_MAX);
    expect(Math.round(LORA_DEFAULT / LORA_STEP) * LORA_STEP).toBeCloseTo(LORA_DEFAULT, 10);
  });
});
