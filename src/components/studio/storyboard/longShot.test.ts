import { describe, expect, it } from "vitest";
import { isLongShot, longShotHint, longShotPlan } from "./longShot";

describe("超长镜（和 Rust long_shot::plan 同一份算法）", () => {
  it("15 秒以内不拆", () => {
    expect(isLongShot(15)).toBe(false);
    expect(longShotPlan(14)).toEqual({ windows: 1, windowSecs: 14 });
    expect(longShotHint(15)).toBeNull();
  });
  it("超过 15 秒等分，段数最少、每段不超过 15 秒", () => {
    for (const [secs, n] of [[15.5, 2], [20, 2], [22, 2], [30, 3], [45, 4]] as const) {
      const p = longShotPlan(secs);
      expect(p.windows).toBe(n);
      expect(p.windowSecs).toBeLessThanOrEqual(15);
      // 和 Rust 那边的测试同一个不变式：拼回总时长
      expect(Math.abs(p.windows * p.windowSecs - (p.windows - 1) * (22 / 24) - secs)).toBeLessThan(0.02);
    }
    expect(longShotHint(22)).toContain("分成 2 段");
  });
});
