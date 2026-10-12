import { describe, expect, it } from "vitest";
import { canDub, dubLabel, dubSummary, overflows, restitchNote, voicesUsed } from "./dubState";
import type { DubOutcome } from "../../../api/cinema";

const o = (p: Partial<DubOutcome> = {}): DubOutcome => ({
  shot_id: "s1", lines: 2, voices: [["甲", "va", false], ["乙", "vb", false]],
  out_secs: 8, video_secs: 8, overflow_secs: null, replaced_source_audio: true, ...p,
});

describe("配音", () => {
  it("没配过和配过一部分说法不同 —— 都写「配音」的话不知道再点是补齐还是重来", () => {
    expect(dubLabel(false, 0, 5)).toBe("给 5 镜配音");
    expect(dubLabel(false, 2, 5)).toContain("还差 3");
    expect(dubLabel(false, 5, 5)).toContain("重新配音");
    expect(dubLabel(true, 0, 5)).toBe("配音中…");
  });

  it("没有带台词的镜头时灰掉 —— 点了必定报错", () => {
    expect(canDub(false, 0)).toBe(false);
    expect(dubLabel(false, 0, 0)).toBe("没有带台词的镜头");
    expect(canDub(false, 3)).toBe(true);
    expect(canDub(true, 3)).toBe(false);
  });

  it("换掉原声的镜数要说出来 —— 那几镜口型会对不上", () => {
    const s = dubSummary([o(), o({ shot_id: "s2", replaced_source_audio: false })]);
    expect(s).toContain("2 镜 / 4 句");
    expect(s).toContain("1 镜换掉了原声");
    expect(s).toContain("口型");
  });

  it("全都没有原声时不吓唬人", () => {
    expect(dubSummary([o({ replaced_source_audio: false })])).not.toContain("口型");
  });

  it("**配音超长必须单列**：拼接带 -shortest，超出那截会被砍掉且不报错", () => {
    const out = overflows([o(), o({ shot_id: "s2", overflow_secs: 1.8 })]);
    expect(out).toEqual([{ shot_id: "s2", secs: 1.8 }]);
  });

  it("**全是同一把嗓子要看得出来** —— 那等于角色音色没配上，而它不报错", () => {
    expect(voicesUsed([o({ voices: [["甲", "x", false], ["乙", "x", false]] })])).toEqual(["x"]);
    expect(voicesUsed([o()])).toEqual(["va", "vb"]);
    // 空音色不算（那是回落到全局，不是一把具名的嗓子）
    expect(voicesUsed([o({ voices: [["甲", "  ", false]] })])).toEqual([]);
  });

  it("没配音时不显示总结，不是显示空壳", () => {
    expect(dubSummary([])).toBe("");
    expect(overflows([])).toEqual([]);
  });
});

describe("配完之后", () => {
  /**
   * 配音只写 wav，**不碰视频**；换音轨发生在合成那一步。所以已经合成过的
   * 片子在配完音之后一点变化都没有 —— 不报错，就是没变。
   * 用户 2026-09-18 问的正是这条。
   */
  it("**已经合成过的必须说「再合成一次」** —— 否则片子不会变，而且不报错", () => {
    const n = restitchNote(3, true);
    expect(n).toContain("还没进片子");
    expect(n).toContain("合成一集");
  });

  it("还没合成过的不用催 —— 他接下来本来就要合成", () => {
    expect(restitchNote(3, false)).toContain("会自动用上");
    expect(restitchNote(3, false)).not.toContain("还没进片子");
  });

  it("没配音时不说话，不是显示空壳", () => {
    expect(restitchNote(0, true)).toBeNull();
    expect(restitchNote(0, false)).toBeNull();
  });
});
