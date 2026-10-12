import { describe, expect, it } from "vitest";
import type { RefSlot } from "../../../api/cinema";
import { addSlot, defaultUsage, MANUAL_USAGES, removeSlot } from "./refSlots";

const slot = (order: number, asset_id: string): RefSlot => ({ order, asset_id, usage: "identity", control: "", not_control: "" });

describe("手动增删参考图槽位", () => {
  it("加在最后，编号从 1 连续", () => {
    const out = addSlot([slot(2, "b"), slot(1, "a")], { asset_id: "c", usage: "outfit", control: "仿生人外形", not_control: "" });
    expect(out.map((s) => [s.order, s.asset_id])).toEqual([[1, "a"], [2, "b"], [3, "c"]]);
    expect(out[2].control).toBe("仿生人外形");
  });

  it("删掉一张，后面的往前补号", () => {
    const out = removeSlot([slot(1, "a"), slot(2, "b"), slot(3, "c")], 1);
    expect(out.map((s) => [s.order, s.asset_id])).toEqual([[1, "a"], [2, "c"]]);
  });

  it("起始帧 / 结束帧不能手动选（在关键帧那栏挂）；默认用途按资产类型", () => {
    expect(MANUAL_USAGES).not.toContain("start_frame");
    expect(MANUAL_USAGES).not.toContain("end_frame");
    expect(defaultUsage("scene")).toBe("geography");
    expect(defaultUsage("character")).toBe("identity");
    expect(defaultUsage("character", false)).toBe("outfit");
  });
});
