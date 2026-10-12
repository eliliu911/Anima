import { describe, expect, it } from "vitest";
import type { StoryboardShot } from "../../../api/cinema";
import { framePlan, keyframeMode, KEYFRAME_MODE_LABEL } from "./keyframes";

const shot = (id: string, no: number, scene: number, extra: Partial<StoryboardShot> = {}): StoryboardShot =>
  ({ id, shot_num: no, scene_no: scene, duration: 5, status: "pending", prompts: { b: { frame: "f", video: "v", synthesized_at: 1 } }, ...extra }) as StoryboardShot;

describe("keyframeMode：按挂了什么自动定出片方式", () => {
  it("四种组合", () => {
    expect(keyframeMode(undefined)).toBe("reference");
    expect(keyframeMode({ first: true, last: false })).toBe("first");
    expect(keyframeMode({ first: true, last: true })).toBe("first_last");
    expect(keyframeMode({ first: false, last: true })).toBe("last_only");
    expect(KEYFRAME_MODE_LABEL.first_last).toBe("首尾帧");
  });
});

describe("framePlan：按场出首帧", () => {
  const shots = [
    shot("a", 2, 1),
    shot("b", 1, 1, { keyframes: { first: true, last: false } }),
    shot("c", 3, 1, { prompts: {} }),
    shot("d", 4, 2),
  ];
  // 2026-10-03：没合成的也出（后端按镜头表现拼画面提示词），只是数出来告诉用户
  it("只出勾了的场；已有首帧的不动；没合成的照样出", () => {
    expect(framePlan(shots, [1], "b", false)).toEqual({ ids: ["a", "c"], kept: 1, composed: 1, reference: 0 });
  });
  it("勾了重出：已有首帧的也出，按镜号排", () => {
    expect(framePlan(shots, [1, 2], "b", true).ids).toEqual(["b", "a", "c", "d"]);
  });
  // 2026-10-11 E05：弹窗写「出 31 镜首帧」，后端只出 19 镜 —— 建议参考出片的不数
  it("建议参考出片的不出首帧，也不算进去", () => {
    expect(framePlan(shots, [1, 2], "b", false, new Set(["c", "d"]))).toEqual({ ids: ["a"], kept: 1, composed: 0, reference: 2 });
    // 勾了重出也不出：参考出片本来就不吃首帧
    expect(framePlan(shots, [1], "b", true, new Set(["b"])).ids).toEqual(["a", "c"]);
  });
  it("线路不对就当没合成 —— 照样出", () => {
    expect(framePlan(shots, [2], "a", false)).toEqual({ ids: ["d"], kept: 0, composed: 1, reference: 0 });
  });
});
