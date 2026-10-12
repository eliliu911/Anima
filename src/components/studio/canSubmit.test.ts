import { describe, expect, it } from "vitest";
import { canSubmit } from "./canSubmit";

const base = { engineId: "zimage", prompt: "一只猫", images: [], needImages: 0 };

describe("能不能点生成", () => {
  it("齐了就能点", () => {
    expect(canSubmit(base)).toEqual({ ok: true, reason: "生成" });
  });

  /**
   * **空提示词点生成，出来的是一张随机图。** 引擎不会报错 ——
   * 它照样跑完 45 秒然后给你一张跟你毫无关系的画。
   */
  it("提示词空着不能点", () => {
    expect(canSubmit({ ...base, prompt: "" }).ok).toBe(false);
    expect(canSubmit({ ...base, prompt: "   \n  " }).ok).toBe(false);
    expect(canSubmit({ ...base, prompt: "" }).reason).toContain("提示词");
  });

  it("没引擎不能点", () => {
    expect(canSubmit({ ...base, engineId: null }).ok).toBe(false);
    expect(canSubmit({ ...base, engineId: null }).reason).toContain("引擎");
  });

  /** 图生图 / 首尾帧这些模式少一张图就跑不了，而错误要等到后端才报。 */
  it("模式要图但没给够", () => {
    const c = canSubmit({ ...base, needImages: 2, images: ["/a.png"], mediaLabel: "首尾帧" });
    expect(c.ok).toBe(false);
    expect(c.reason).toContain("还差 1 张首尾帧");
  });

  it("图给够了就能点", () => {
    expect(canSubmit({ ...base, needImages: 2, images: ["/a.png", "/b.png"] }).ok).toBe(true);
  });

  /** 槽位是可以留空的（数组里有 null），空位不算数。 */
  it("空槽位不算已给", () => {
    const c = canSubmit({ ...base, needImages: 1, images: [null, "  "] });
    expect(c.ok).toBe(false);
    expect(c.reason).toContain("还差 1 张");
  });

  it("没写媒体标题时说「输入图」，不留一个空词", () => {
    expect(canSubmit({ ...base, needImages: 1, images: [], mediaLabel: null }).reason).toContain(
      "输入图",
    );
    expect(canSubmit({ ...base, needImages: 1, images: [], mediaLabel: "  " }).reason).toContain(
      "输入图",
    );
  });

  /**
   * **三种原因要能分开。** 灰着不解释比不灰更让人卡住 ——
   * 用户只能挨个试是没选引擎、还是提示词空、还是少传图。
   */
  it("三种原因各说各的", () => {
    const rs = [
      canSubmit({ ...base, engineId: null }).reason,
      canSubmit({ ...base, prompt: "" }).reason,
      canSubmit({ ...base, needImages: 1, images: [] }).reason,
    ];
    expect(new Set(rs).size).toBe(3);
  });
});
