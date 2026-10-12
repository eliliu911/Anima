import { describe, expect, it } from "vitest";
import { polishKindForMode } from "./polishKind";
import type { VideoMode } from "./videoMode";

const ALL: VideoMode[] = ["t2v", "i2v", "fl2v", "r2v", "motion"];

describe("视频润色走哪一份提示词", () => {
  it("每种模式都有归宿", () => {
    for (const m of ALL) expect(polishKindForMode(m)).toBeTruthy();
  });

  /**
   * **文生视频不能套「首帧已定」那份。** 它没有首帧，那份提示词
   * 第 4 条会要求「不要重复描述静态外观」—— 结果画面里什么都没有。
   */
  it("文生视频走自己那一份", () => {
    expect(polishKindForMode("t2v")).toBe("t2v");
  });

  /**
   * **也不能退到出图润色。** 那是修 t2v 时的第一版：绕开了首帧假设，
   * 却换成一份静态画面的模板，时间、动作、声音一样没有。
   */
  it("没有一种视频模式会落到出图润色上", () => {
    for (const m of ALL) expect(polishKindForMode(m)).not.toBe("storyboard");
  });

  it("参考生视频走参考那一份", () => {
    expect(polishKindForMode("r2v")).toBe("r2v");
  });

  /** i2v / fl2v / motion 都是从一张已有的图开始动 —— 首帧假设成立 */
  it("有首帧的三种都走图生视频那一份", () => {
    for (const m of ["i2v", "fl2v", "motion"] as VideoMode[]) {
      expect(polishKindForMode(m)).toBe("video");
    }
  });
});
