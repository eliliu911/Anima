import { describe, expect, it } from "vitest";
import { canToggleIntro, coverButtonLabel, coverHint, introActionLabel, introStateLabel, syncIncludeCover } from "./coverIntro";

describe("分集封面", () => {
  it("已经有封面时按钮要说清楚是覆盖", () => {
    expect(coverButtonLabel(false, false)).toBe("生成封面");
    expect(coverButtonLabel(true, false)).toContain("重新");
    expect(coverButtonLabel(true, true)).toContain("…");
  });

  /** 勾了之后封面被删 —— 不弹回去的话要跑完一整轮镜头才报错 */
  it("没有封面时勾选强制回 false", () => {
    expect(syncIncludeCover(false, true)).toBe(false);
    expect(syncIncludeCover(true, true)).toBe(true);
    expect(syncIncludeCover(true, false)).toBe(false);
  });

  it("点不了的时候要说为什么", () => {
    expect(coverHint(false, 2.5)).toContain("先生成封面");
    expect(coverHint(true, 2.5)).toContain("2.5");
  });
});

describe("片头：设置和动作分开", () => {
  /**
   * **按钮文案说的是「点下去会发生什么」**，不是「当前是什么状态」——
   * 后者会让人分不清点了是加还是减。
   */
  it("按钮文案是动作不是状态", () => {
    expect(introActionLabel(false, false)).toBe("加上片头");
    expect(introActionLabel(true, false)).toBe("移除片头");
    expect(introActionLabel(true, true)).toContain("…");
  });

  it("状态标签和动作标签不能是同一句", () => {
    expect(introStateLabel(true)).not.toBe(introActionLabel(true, false));
    expect(introStateLabel(false)).not.toBe(introActionLabel(false, false));
  });

  /**
   * **「移除」在没有封面时也要能点** —— 封面可能是刚被删掉的，
   * 而成片里那段片头还在。只看 hasCover 的话用户就卡住了。
   */
  it("没有封面时仍然能移除已有的片头", () => {
    expect(canToggleIntro(false, true, false)).toBe(true);
    expect(canToggleIntro(false, false, false)).toBe(false);
    expect(canToggleIntro(true, false, false)).toBe(true);
    expect(canToggleIntro(true, true, true)).toBe(false);
  });

  /** 勾选框的说明要讲清楚它**不碰现在这个成片** */
  it("勾选框说明要划清界限", () => {
    expect(coverHint(true, 2.5)).toContain("下次合成");
    expect(coverHint(true, 2.5)).toContain("不影响现在");
  });
});
