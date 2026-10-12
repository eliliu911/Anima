import { describe, it, expect } from "vitest";
import { longVideoPlan } from "./longVideo";

describe("超长视频分段（和 Rust long_shot::plan 同一个公式）", () => {
  it("15 秒以内不分段", () => {
    expect(longVideoPlan(15)).toBeNull();
    expect(longVideoPlan(8)).toBeNull();
  });
  // 数字和 Rust 测试里的对得上：22 → 2 × 11.46（long_shot 模块注释）、30 → 3 × 10.61、75 → 6 段
  it("段数和每段秒数", () => {
    expect(longVideoPlan(22)).toEqual({ windows: 2, windowSecs: 11.46 });
    expect(longVideoPlan(30)).toEqual({ windows: 3, windowSecs: 10.61 });
    expect(longVideoPlan(75)?.windows).toBe(6);
    expect(longVideoPlan(75)!.windowSecs).toBeLessThanOrEqual(15);
  });
});
