import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import type { CanvasEdge, CanvasNode } from "../types";
import { inferVideoMode } from "./videoMode";
import { modeBlocked, MODE_GROUPS, MODE_OPTIONS, pickInGroup } from "../ui/VideoModeMenu";

const n = (id: string, type: string, data: Record<string, unknown> = {}): CanvasNode =>
  ({ id, type, position: { x: 0, y: 0 }, data } as CanvasNode);
const e = (s: string, t: string): CanvasEdge => ({ id: `${s}>${t}`, source: s, target: t } as CanvasEdge);

/**
 * 2026-09-24 用户截图：连着一个还没出图的图片节点，模式写「文生视频」、理由写「没有连接任何图片」，
 * 下面的预检却说「这张图还没出，出了之后它就是首帧」—— 两处打架。点生成时执行器会先把上游这张图跑出来，
 * 所以模式要按「会有这张图」推。
 */
describe("连着但还没出图的图片节点也算", () => {
  it("一张空的图片节点 → 图生视频，理由说还没出、点生成时先跑它", () => {
    const got = inferVideoMode("v", [n("v", "videoGen"), n("i", "imageGen")], [e("i", "v")]);
    expect(got.mode).toBe("i2v");
    expect(got.reason).toContain("还没出");
    expect(got.reason).not.toContain("没有连接任何图片");
    expect(got.imageCount).toBe(1);
    expect(got.images).toEqual([]);  // 还没有路径 —— 真正发出去的图等上游跑完再取
  });

  it("两张空的 → 参考生视频，理由里带上「还没出」", () => {
    const got = inferVideoMode("v", [n("v", "videoGen"), n("a", "imageGen"), n("b", "imageGen")], [e("a", "v"), e("b", "v")]);
    expect(got.mode).toBe("r2v");
    expect(got.imageCount).toBe(2);
    expect(got.reason).toContain("还没出");
  });

  it("出了图的和没出的一起数", () => {
    const got = inferVideoMode("v", [n("v", "videoGen"), n("a", "imageGen", { outputImageUrl: "/a.png" }), n("b", "imageGen")], [e("a", "v"), e("b", "v")]);
    expect(got.imageCount).toBe(2);
    expect(got.images).toEqual(["/a.png"]);
  });

  it("什么都没连还是文生视频", () => {
    const got = inferVideoMode("v", [n("v", "videoGen")], []);
    expect(got).toMatchObject({ mode: "t2v", imageCount: 0 });
  });
});

describe("底栏的视频生成模式菜单（照 LibTV）", () => {
  // 用户 2026-09-25：「就像 libtv 里这样，把它分清楚」—— 五项、LibTV 的顺序，每项一行说明
  it("五项：文生 / 全能参考 / 图生 / 首尾帧 / 图片参考，每项都有说明", () => {
    // 用户 2026-10-08：菜单三组 —— 文生 / 首帧·首尾帧 / 图片参考·全能参考
    expect(MODE_GROUPS.map((g) => [g.label, g.subs])).toEqual([["文生视频", ["t2v"]], ["首帧 / 首尾帧", ["i2v", "fl2v"]], ["参考", ["r2v", "omni"]]]);
    const none = () => null;
    expect(pickInGroup(["i2v", "fl2v"], "t2v", "i2v", none)).toBe("i2v");
    expect(pickInGroup(["i2v", "fl2v"], "fl2v", "i2v", none), "当前就在组里不动").toBe("fl2v");
    expect(pickInGroup(["r2v", "omni"], "i2v", "i2v", (m) => (m === "omni" ? "x" : null))).toBe("r2v");
    expect(pickInGroup(["i2v", "fl2v"], "t2v", "t2v", () => "先连图")).toBeNull();
    expect(MODE_OPTIONS.map((m) => m.id)).toEqual(["t2v", "omni", "i2v", "fl2v", "r2v"]);
    expect(MODE_OPTIONS.map((m) => m.label)).toEqual(["文生视频", "全能参考", "图生视频", "首尾帧", "图片参考"]);
    for (const m of MODE_OPTIONS) expect(m.hint.length, m.id).toBeGreaterThan(5);
  });

  it("全能参考要连一段参考视频", () => {
    expect(modeBlocked("omni", 2, false)).toContain("参考视频");
    expect(modeBlocked("omni", 2, true)).toBeNull();
  });

  it("跟连线对不上的置灰：连着图不能文生，少于两张不能首尾帧，没图不能图生 / 参考", () => {
    expect(modeBlocked("t2v", 0)).toBeNull();
    expect(modeBlocked("t2v", 1)).toContain("断开");
    expect(modeBlocked("i2v", 0)).not.toBeNull();
    expect(modeBlocked("i2v", 1)).toBeNull();
    expect(modeBlocked("fl2v", 1)).not.toBeNull();
    expect(modeBlocked("fl2v", 2)).toBeNull();
    expect(modeBlocked("r2v", 0)).not.toBeNull();
    expect(modeBlocked("r2v", 3)).toBeNull();
  });

  it("放在底栏、模型选择旁边；顶上那一排的旧选择器（跟着连线走 / 首帧生视频）去掉了", () => {
    const panel = readFileSync(new URL("../nodes/panels/VideoOperationPanel.tsx", import.meta.url), "utf8");
    expect(panel).toContain("<VideoModeMenu");
    expect(panel.indexOf("<VideoModeMenu")).toBeGreaterThan(panel.indexOf("<CanvasEngineSelect"));
    expect(panel).not.toContain("首帧生视频");
    expect(panel).not.toContain('value="auto">跟着连线走');
  });
});
