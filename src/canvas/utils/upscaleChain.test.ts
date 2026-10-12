import { describe, it, expect, vi, beforeEach } from "vitest";
import { useCanvasStore } from "../store/useCanvasStore";
import { mediaKind, nodeMedia, upscaleSource, upscaleStale } from "./upscaleChain";
import { depthSourceVideo } from "./depthChain";
import { videoInputReferences, executeNodePipeline } from "../engine/dagExecutor";

vi.mock("../engine/canvasTasks", () => ({
  inNode: (_ctx: unknown, fn: () => unknown) => fn(),
  liveJobs: new Set<string>(),
  studioGenerate: vi.fn(),
  canvasDepthCapture: vi.fn(),
  canvasDepthVideo: vi.fn(),
  canvasImageOp: vi.fn(),
  canvasVideoInterpolate: vi.fn(),
  canvasVideoUpscale: vi.fn(),
}));
vi.mock("../../api/studio", () => ({ studioGenerate: vi.fn(), studioCapabilities: vi.fn() }));
import { canvasImageOp, canvasDepthCapture, canvasVideoUpscale } from "../engine/canvasTasks";

const state = () => useCanvasStore.getState();
const link = (source: string, target: string) =>
  state().setEdges([...state().edges, { id: `${source}-${target}`, source, target, targetHandle: "prompt-in" }]);

describe("超分节点的连线", () => {
  beforeEach(() => {
    state().clearCanvas();
    vi.clearAllMocks();
  });

  it("按扩展名分图和视频", () => {
    expect(mediaKind("/a/b.MP4")).toBe("video");
    expect(mediaKind("/a/b.mov")).toBe("video");
    expect(mediaKind("/a/b.png")).toBe("image");
  });

  it("只吃一个上游；没连、连多个、上游没东西都说清楚", () => {
    const up = state().addNode("upscale", undefined, {});
    expect(upscaleSource(up, state().nodes, state().edges).reason).toContain("连一张图");
    const img = state().addNode("imageGen", undefined, { outputImageUrl: "/g.png" });
    link(img, up);
    expect(upscaleSource(up, state().nodes, state().edges)).toEqual({ path: "/g.png", kind: "image" });
    const vid = state().addNode("videoGen", undefined, {});
    link(vid, up);
    expect(upscaleSource(up, state().nodes, state().edges).reason).toContain("一次只能放大一个");
    state().setEdges(state().edges.filter((e) => e.source !== img));
    expect(upscaleSource(up, state().nodes, state().edges).reason).toContain("还没有图或视频");
  });

  it("上游换了图，结果标成过期", () => {
    const img = state().addNode("imageGen", undefined, { outputImageUrl: "/g.png" });
    const up = state().addNode("upscale", undefined, { outputUrl: "/g2.png", sourcePath: "/g.png" });
    link(img, up);
    const node = () => state().nodes.find((n) => n.id === up)!;
    expect(upscaleStale(node(), state().nodes, state().edges)).toBe(false);
    state().updateNodeData(img, { outputImageUrl: "/h.png" });
    expect(upscaleStale(node(), state().nodes, state().edges)).toBe(true);
  });

  /** 放大后的结果要能接着往下连，否则这个节点是死胡同 */
  it("放大后的图给下游当参考图，只占一个槽位（不把原图也带上）", () => {
    const img = state().addNode("imageGen", undefined, { outputImageUrl: "/g.png" });
    const up = state().addNode("upscale", undefined, { outputUrl: "/g2.png", sourcePath: "/g.png" });
    const video = state().addNode("videoGen", undefined, {});
    link(img, up);
    link(up, video);
    expect(videoInputReferences(video, state().nodes, state().edges).map((r) => r.path)).toEqual(["/g2.png"]);
    expect(nodeMedia(state().nodes.find((n) => n.id === up))).toEqual({ path: "/g2.png", kind: "image" });
  });

  it("放大后的视频能接深度动作捕捉，不能当参考图", () => {
    const up = state().addNode("upscale", undefined, { outputUrl: "/v2.mp4" });
    const depth = state().addNode("depthCapture", undefined, {});
    link(up, depth);
    expect(depthSourceVideo(depth, state().nodes, state().edges).path).toBe("/v2.mp4");
    const video = state().addNode("videoGen", undefined, {});
    link(up, video);
    expect(videoInputReferences(video, state().nodes, state().edges)).toEqual([]);
  });

  it("下游点生成时，没跑过或过期的超分先跑", async () => {
    const img = state().addNode("imageGen", undefined, { outputImageUrl: "/g.png", referenceOnly: true });
    const up = state().addNode("upscale", undefined, {});
    const depthSrc = state().addNode("videoGen", undefined, { outputVideoUrl: "/dance.mp4" });
    const up2 = state().addNode("upscale", undefined, { outputUrl: "/dance2x.mp4", sourcePath: "/dance.mp4" });
    const depth = state().addNode("depthCapture", undefined, {});
    link(img, up);
    link(depthSrc, up2);
    link(up2, depth);
    vi.mocked(canvasImageOp).mockResolvedValue({ path: "/g2.png" });
    vi.mocked(canvasDepthCapture).mockResolvedValue({ path: "/d.mp4" });
    const out = state().addNode("imageGen", undefined, { prompt: "x" });
    link(up, out);
    await executeNodePipeline(depth);
    // 没过期的超分不重跑
    expect(canvasDepthCapture).toHaveBeenCalledWith("/dance2x.mp4");
    expect(canvasVideoUpscale).not.toHaveBeenCalled();
    await executeNodePipeline(out).catch(() => {});
    expect(canvasImageOp).toHaveBeenCalledWith("/g.png", "upscale");
    expect(state().nodes.find((n) => n.id === up)?.data.outputUrl).toBe("/g2.png");
  });
});
