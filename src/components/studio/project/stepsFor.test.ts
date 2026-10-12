import { describe, expect, it } from "vitest";
import { STEPS, stepsFor } from "./StepRail";

describe("按内容模式决定步骤序列", () => {
  it("有剧本时六步齐全", () => {
    expect(stepsFor("scripted").map((s) => s.id)).toEqual(STEPS.map((s) => s.id));
  });

  it("老项目（没有这个字段）也是六步 —— 行为不变", () => {
    expect(stepsFor(undefined)).toHaveLength(STEPS.length);
    expect(stepsFor("")).toHaveLength(STEPS.length);
  });

  it("直接创作时没有剧本那一步", () => {
    expect(stepsFor("freeform").map((s) => s.id)).not.toContain("script");
    expect(stepsFor("freeform")).toHaveLength(STEPS.length - 1);
  });

  /**
   * **序号必须重算。** 只过滤不重算的话第一项显示成「2. 风格」，
   * 而右边还写着 STEP 02 —— 看起来像少了一步或者哪儿出错了。
   */
  it("序号从 1 连续重排，不留洞", () => {
    for (const mode of ["scripted", "freeform"]) {
      const nums = stepsFor(mode).map((s) => s.num);
      expect(nums, `${mode} 的序号不连续：${nums}`).toEqual(
        nums.map((_, i) => i + 1),
      );
    }
  });

  /**
   * **重排必须产生新对象。** 原地改 `s.num` 的话，改的是 `STEPS` 里那些
   * 对象本身 —— 打开一个 freeform 项目会把常量污染掉，
   * 再打开普通项目时序号就从 1 开始少一个，而且不报错。
   *
   * 查的必须是**留下来的**那几步：第一版查 `STEPS[0]`（剧本），
   * 而它恰好被过滤掉了，永远不会被改 —— 测试全绿，什么也没测到。
   */
  it("重排不会改到那份常量本身", () => {
    const before = STEPS.map((s) => s.num);
    stepsFor("freeform");
    expect(STEPS.map((s) => s.num), "STEPS 被就地改了").toEqual(before);
    // **序号从 STEPS 自己推，不写死** —— 2026-09-08 删掉「动效」那一步时，
    // 这里原来写的 `[1,2,3,4,5,6]` 就红了。写死的话每次增删步骤都得改一遍，
    // 而它想钉的其实是「1 开始、连续」，和总共几步无关。
    expect(STEPS.map((s) => s.num)).toEqual(STEPS.map((_, i) => i + 1));
    expect(STEPS.length, "步骤没了？").toBeGreaterThanOrEqual(4);
  });

  it("认不出来的值当成有剧本 —— 保守方向：宁可多一步", () => {
    expect(stepsFor("乱写的").map((s) => s.id)).toContain("script");
  });
});
