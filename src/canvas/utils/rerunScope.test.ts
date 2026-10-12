import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import type { CanvasEdge, CanvasNode } from "../types";
import { previewRerun, willRerun, type RerunDeps } from "./rerunScope";

const n = (id: string, type: string, data: Record<string, unknown> = {}): CanvasNode =>
  ({ id, type, position: { x: 0, y: 0 }, data } as CanvasNode);
const e = (s: string, t: string): CanvasEdge => ({ id: `${s}>${t}`, source: s, target: t } as CanvasEdge);
const deps = (sig = "s", depth?: string): RerunDeps => ({ depthSourcePath: () => depth, signatureOf: () => sig });

describe("哪些上游会被连带重跑", () => {
  it("还没出过图的上游要跑", () => {
    const nodes = [n("v", "videoGen", {}), n("i", "imageGen", {})];
    expect(willRerun(nodes[1], "v", nodes, [e("i", "v")], deps())).toBe(true);
  });

  it("已经出好、签名也对得上的上游不跑", () => {
    const nodes = [n("v", "videoGen", {}), n("i", "imageGen", { outputImageUrl: "/a.png", lastExecutedSignature: "sig-1" })];
    expect(willRerun(nodes[1], "v", nodes, [e("i", "v")], deps("sig-1"))).toBe(false);
  });

  it("上游输入变过（签名对不上）就要跑", () => {
    const nodes = [n("v", "videoGen", {}), n("i", "imageGen", { outputImageUrl: "/a.png", lastExecutedSignature: "sig-旧" })];
    expect(willRerun(nodes[1], "v", nodes, [e("i", "v")], deps("sig-新"))).toBe(true);
  });

  /**
   * **绑定成参考图的上游不重跑。** 读存下来的 `mode` 字段会让用户上传的
   * 源图被当成「要重新生成」—— 白跑一次出图，而且把上传的那张覆盖掉。
   */
  it("被当成参考图绑进来的上游不重跑", () => {
    const nodes = [
      n("v", "videoGen", { references: [{ path: "/a.png", name: "甲", sourceNodeId: "i" }, { path: "/b.png", name: "乙" }] }),
      // **签名故意写成过期的** —— 不这样的话这条测试会靠 imageGen 分支
      // 恰好也返回 false 而「通过」，删掉上面那个早退也照样绿（假绿）。
      n("i", "imageGen", { outputImageUrl: "/a.png", lastExecutedSignature: "旧" }),
    ];
    expect(willRerun(nodes[1], "v", nodes, [e("i", "v")], deps("新"))).toBe(false);
  });

  it("深度：源视频换过就要重抽", () => {
    const d = n("d", "depthCapture", { depthVideoUrl: "/d.mp4", sourceVideoPath: "/旧.mp4" });
    expect(willRerun(d, "v", [d], [], deps("s", "/新.mp4"))).toBe(true);
    expect(willRerun(d, "v", [d], [], deps("s", "/旧.mp4"))).toBe(false);
  });

  it("目标节点自己不算在上游范围里", () => {
    const t = n("v", "videoGen", {});
    expect(willRerun(t, "v", [t], [], deps())).toBe(false);
  });

  it("摘要分清要跑几个、跳过几个", () => {
    const a = n("a", "imageGen", {});
    const b = n("b", "imageGen", { outputImageUrl: "/b.png", lastExecutedSignature: "s" });
    const v = n("v", "videoGen", {});
    const p = previewRerun([a, b, v], "v", [a, b, v], [e("a", "v"), e("b", "v")], deps("s"));
    expect(p.willRun.map((x) => x.id)).toEqual(["a"]);
    expect(p.skipped).toBe(1);
  });
});

/**
 * **判据只能有一个所有者。** 原来散在 `executeNodePipeline` 的三个 if 里，
 * 预览再抄一遍必然漂移 —— 而「预览说要跑 2 个、实际跑了 5 个」比不给预览更糟。
 */
describe("执行器和预览读同一份判据", () => {
  const exec = readFileSync(new URL("../engine/dagExecutor.ts", import.meta.url), "utf8");

  it("执行器调 willRerun，没有自己再判一遍", () => {
    expect(exec.match(/willRerun\(/g)?.length ?? 0).toBeGreaterThanOrEqual(3);
    expect(exec).not.toContain("upscaleStale(current");
    expect(exec).not.toMatch(/const isDirty = !currentData\?\.outputImageUrl/);
  });

  /**
   * 2026-09-24 起面板**不再显示**「点生成会先跑 N 个上游」这条（用户要求去掉）：面板顶上的上游卡片已经能看出
   * 哪些还没出图（占位图标），这条是重复的。判断「会连带跑哪些」的逻辑还在，执行器照用。
   */
  it("视频面板不再显示「点生成会先跑 N 个上游」", () => {
    const panel = readFileSync(new URL("../nodes/panels/VideoOperationPanel.tsx", import.meta.url), "utf8");
    expect(panel).not.toContain("点生成会先跑");
    expect(panel).not.toContain("previewRerun(");
  });
});
