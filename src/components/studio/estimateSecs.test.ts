import { describe, expect, it } from "vitest";
import { estimateSecs } from "./estimateSecs";

const h3 = {
  secs: 384,
  sizes: [
    { width: 640, height: 384, secs: 222 },
    { width: 768, height: 512, secs: 384 },
  ],
};

describe("estimateSecs", () => {
  it("选中的画幅有实测值就用它 —— 这是原来那个档位选择器给的信息", () => {
    expect(estimateSecs(h3, { width: 640, height: 384 }, 5, 1)).toBe(222);
    expect(estimateSecs(h3, { width: 768, height: 512 }, 5, 1)).toBe(384);
  });

  it("换画幅要换数 —— 两档给一样的数等于这一栏没意义", () => {
    const a = estimateSecs(h3, { width: 640, height: 384 }, 5, 1);
    const b = estimateSecs(h3, { width: 768, height: 512 }, 5, 1);
    expect(a).not.toBe(b);
  });

  it("批量要乘上去 —— ComfyUI 串行跑，两张就是两倍", () => {
    expect(estimateSecs(h3, { width: 640, height: 384 }, 5, 2)).toBe(444);
  });

  it("画幅没实测值时按时长缩放引擎基准（基准是 5 秒那一档）", () => {
    const eng = { secs: 384, sizes: [{ width: 1280, height: 720, secs: null }] };
    expect(estimateSecs(eng, { width: 1280, height: 720 }, 10, 1)).toBe(768);
  });

  it("出图没有时长，用引擎基准", () => {
    const zimage = { secs: 45, sizes: [{ width: 1024, height: 1024, secs: null }] };
    expect(estimateSecs(zimage, { width: 1024, height: 1024 }, null, 1)).toBe(45);
  });

  it("还没挑画幅时不去翻画幅表", () => {
    expect(estimateSecs(h3, null, 5, 1)).toBe(384);
  });
});
