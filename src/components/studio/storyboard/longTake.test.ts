import { describe, expect, it } from "vitest";
import type { StoryboardShot } from "../../../api/cinema";
import { CONTEXT_FRAME_CHOICES, contextFramesHint, longTakeCost, planLongTake } from "./longTake";

const 镜 = (num: number, secs: number) => ({ id: `s${num}`, shot_num: num, duration: secs }) as StoryboardShot;

describe("一场能不能整段出", () => {
  /** 用户要的那个例子：1 分钟室内打斗。这两个数就是勾选框旁边要显示的。 */
  it("4×15 秒 = 57.6 秒成片、约 4.5 小时", () => {
    const p = planLongTake([1, 2, 3, 4].map((i) => 镜(i, 15)));
    if (!p.ok) throw new Error(p.why);
    expect(p.netSecs).toBeCloseTo(57.583, 2);
    expect(p.etaHours).toBeCloseTo(4.5, 1);
    expect(p.warning).toBeUndefined();
    expect(p.overlapShare).toBeLessThan(0.07);
  });

  /** 用户问的冒烟档。能跑，但两件事要说出来。 */
  it("3+3+3 能跑，但要报实验性和三成重叠", () => {
    const p = planLongTake([1, 2, 3].map((i) => 镜(i, 3)));
    if (!p.ok) throw new Error(p.why);
    expect(p.netSecs).toBeCloseTo(175 / 24, 3); // 实测 175 帧
    expect(p.overlapShare).toBeCloseTo(0.306, 2);
    expect(p.warning).toContain("17n+5");
    expect(p.warning).toContain("31%");
    expect(p.warning).toContain("验管道");
    // 40 分钟量级，不是小时
    expect(longTakeCost(p)[1]).toContain("分钟");
  });

  /** **窗口必须等长** —— 上游节点只有一个 window_duration_seconds。 */
  it("时长不一样要拦下来，并说清是哪几种", () => {
    const p = planLongTake([镜(1, 15), 镜(2, 10)]);
    expect(p.ok).toBe(false);
    if (p.ok) return;
    expect(p.why).toContain("15 / 10");
  });

  it("超上限、低于下限、只有一镜都拦得住", () => {
    expect(planLongTake([镜(1, 20), 镜(2, 20)]).ok).toBe(false);
    expect(planLongTake([镜(1, 2), 镜(2, 2)]).ok).toBe(false);
    expect(planLongTake([镜(1, 15)]).ok).toBe(false);
  });

  /**
   * **重叠档位可选,每一档要说清代价。** 短窗口配大重叠是在白烧算力。
   * 后端那半边由 `long_take::重叠档位可选_每一档都说得出占多少` 钉着。
   */
  it("换成 5 帧重叠，净时长更长、浪费更少", () => {
    const big = planLongTake([1, 2, 3].map((i) => 镜(i, 3)), 22);
    const small = planLongTake([1, 2, 3].map((i) => 镜(i, 3)), 5);
    if (!big.ok || !small.ok) throw new Error("排不出来");
    expect(small.netSecs).toBeGreaterThan(big.netSecs);
    expect(small.overlapShare).toBeLessThan(0.08);
    expect(big.overlapShare).toBeGreaterThan(0.3);
  });

  it("每一档都说得出占窗口多少", () => {
    for (const f of CONTEXT_FRAME_CHOICES) {
      expect(contextFramesHint(f, 3)).toContain("%");
    }
    expect(contextFramesHint(5, 3)).toContain("运动衔接最弱");
    expect(contextFramesHint(22, 3)).toContain("推荐");
  });

  /** 代价必须逐条说出来，不能只是一个勾。 */
  it("代价里必须有「整条重来」和「中途不能改」", () => {
    const p = planLongTake([1, 2, 3, 4].map((i) => 镜(i, 15)));
    if (!p.ok) throw new Error(p.why);
    const c = longTakeCost(p).join("\n");
    expect(c).toContain("整条重来");
    expect(c).toContain("中途不能改");
    expect(c).toContain("先按分镜出一遍");
  });
});
