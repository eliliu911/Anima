import { describe, expect, it } from "vitest";
import type { StoryboardShot } from "../../../api/cinema";
import { continuityAdvice, effectiveContinuity } from "./continuity";

const shot = (p: Partial<StoryboardShot>) =>
  ({ id: "s", shot_num: 1, scene_no: 1, duration: 5, ...p }) as StoryboardShot;

describe("这一镜怎么接上一镜", () => {
  /**
   * **老字段要折进来。** 两边各读各的就是两份真相：界面显示「不接」，
   * 而出片去拿了上一镜的末帧。Rust 那边由
   * `cinema::continuity_wiring_tests::老数据的接上一镜折进新字段` 钉着。
   */
  it("老数据的 continues_previous 折成「接住最后一帧」", () => {
    expect(effectiveContinuity(shot({ continues_previous: true }))).toBe("first_frame");
    // 新字段设过了就听新的 —— 用户改过的不能被老字段翻回去
    expect(
      effectiveContinuity(shot({ continues_previous: true, continuity: "ref_video" } as Partial<StoryboardShot>)),
    ).toBe("ref_video");
    expect(effectiveContinuity(shot({}))).toBe("none");
  });

  it("第一镜整组禁用，并说清为什么", () => {
    const a = continuityAdvice(shot({ shot_num: 1 }), null);
    expect(a.blocked).toContain("第一镜");
  });

  /** 跨场接上一镜会把上一场的光线布景带过来 —— 用户明确提过这一条。 */
  it("跨场推荐不接，并说出是哪一场", () => {
    const a = continuityAdvice(shot({ shot_num: 5, scene_no: 2 }), shot({ shot_num: 4, scene_no: 1 }));
    expect(a.recommended).toBe("none");
    expect(a.why).toContain("第 1 场");
  });

  it("同场紧邻推荐「运镜与光线」", () => {
    const a = continuityAdvice(shot({ shot_num: 3, scene_no: 2 }), shot({ shot_num: 2, scene_no: 2 }));
    expect(a.recommended).toBe("ref_video");
    expect(a.why).toContain("第 2 场");
  });

  /** **不推荐字面首帧** —— 它要求姿势严丝合缝，得人自己判断。 */
  it("从不主动推荐「接住最后一帧」", () => {
    for (const [cur, prev] of [
      [shot({ shot_num: 2, scene_no: 1 }), shot({ shot_num: 1, scene_no: 1 })],
      [shot({ shot_num: 2, scene_no: 2 }), shot({ shot_num: 1, scene_no: 1 })],
    ] as const) {
      expect(continuityAdvice(cur, prev).recommended).not.toBe("first_frame");
    }
  });
});
