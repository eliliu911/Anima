import { describe, expect, it } from "vitest";
import { flatIds, moveGroup } from "./renderQueue";

describe("排队名单拖动", () => {
  const g = [["a"], ["b1", "b2"], ["c"]];
  it("整组挪到前面 / 后面", () => {
    expect(moveGroup(g, 2, 0)).toEqual([["c"], ["a"], ["b1", "b2"]]);
    expect(moveGroup(g, 0, 3)).toEqual([["b1", "b2"], ["c"], ["a"]]);
    expect(moveGroup(g, 1, 0)).toEqual([["b1", "b2"], ["a"], ["c"]]);
  });
  it("放回原位或越界：不变", () => {
    expect(moveGroup(g, 1, 1)).toBe(g);
    expect(moveGroup(g, 1, 2)).toBe(g);
    expect(moveGroup(g, 5, 0)).toBe(g);
  });
  it("铺平成 id", () => {
    expect(flatIds([[{ id: "a" }], [{ id: "b1" }, { id: "b2" }]])).toEqual(["a", "b1", "b2"]);
  });
});

import { holdLabel, shotCounts } from "./renderQueue";

describe("暂停位不算镜头（2026-10-10 用户：「这两个进度信息我看不懂」）", () => {
  const job = (label: string, done: number, targets: string[]) => ({ kind: "video_batch", status: "running", targets, progress: { done, total: targets.length, label } });
  const t = ["hold:冒烟测试", "s15", "s16", "s17"];
  it("停在暂停位上：认得出说明，还有 3 镜", () => {
    const j = job("暂停 · 冒烟测试", 0, t);
    expect(holdLabel(j)).toBe("冒烟测试");
    expect(shotCounts(j)).toEqual({ position: 0, total: 3, left: 3 });
  });
  it("出镜头时：暂停位不进分母，也不占名次", () => {
    expect(holdLabel(job("第 15 镜", 1, t))).toBeNull();
    expect(shotCounts(job("第 15 镜", 1, t))).toEqual({ position: 1, total: 3, left: 2 });
    expect(shotCounts(job("第 17 镜", 3, t))).toEqual({ position: 3, total: 3, left: 0 });
  });
  it("还在排队中（一镜都没开始）", () => {
    expect(shotCounts(job("排队中 · 等前一镜出完", 0, ["s15", "s16"]))).toEqual({ position: 0, total: 2, left: 2 });
  });
});
