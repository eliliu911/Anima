import { describe, it, expect } from "vitest";
import { isEmptyExchange, isEmptyPerformance, signalCount, toggleTag } from "./SegmentContracts";

describe("表演细节 / 攻防交换", () => {
  // 和 Rust `combat::signal_count` 同一口径（tests 里那条：慢眨 + 空串 + 吞咽 + 手 + 有来源的物理 = 4）
  it("信号计数和后端一致", () => {
    expect(signalCount({ face: ["慢眨", ""], body: ["吞咽"], hand: "手指松开枪柄", material: { source: "雨" } })).toBe(4);
    expect(signalCount(undefined)).toBe(0);
  });
  it("标签点一下加、再点一下去掉", () => {
    expect(toggleTag(undefined, "慢眨")).toEqual(["慢眨"]);
    expect(toggleTag(["慢眨", "短眨"], "慢眨")).toEqual(["短眨"]);
  });
  it("空合同 / 空交换认得出（清空后不留一个空对象当「已写」）", () => {
    expect(isEmptyPerformance({})).toBe(true);
    expect(isEmptyPerformance({ face: [] , material: { source: "" } })).toBe(true);
    expect(isEmptyPerformance({ gaze: "看他" })).toBe(false);
    expect(isEmptyExchange({ contact_at: 1.2 })).toBe(false);
    expect(isEmptyExchange({})).toBe(true);
  });
});
