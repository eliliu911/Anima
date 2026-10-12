// 超分之后再补帧：补帧必须吃超分后的那个文件，不是超分之前的原片（2026-09-23 用户问出来的）。
import { describe, it, expect, vi, beforeEach } from "vitest";
import { spawnVideoTool, spawnUpscale, outHandleOf } from "./imageTools";
import { useCanvasStore } from "../store/useCanvasStore";
import { canvasCatalogFixture } from "./modelCatalog.fixture";

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

import { studioCapabilities } from "../../api/studio";
import { canvasVideoInterpolate, canvasVideoUpscale } from "../engine/canvasTasks";

const node = (id: string) => useCanvasStore.getState().nodes.find((n) => n.id === id)!;
const edgeInto = (id: string) => useCanvasStore.getState().edges.find((e) => e.target === id)!;

describe("视频工具链：超分 → 补帧", () => {
  beforeEach(() => {
    useCanvasStore.getState().clearCanvas();
    vi.clearAllMocks();
    vi.mocked(studioCapabilities).mockResolvedValue(canvasCatalogFixture);
  });

  it("超分节点后面接补帧：补的是超分后的文件，连线从 upscale-out 出", async () => {
    const s = useCanvasStore.getState();
    const v = s.addNode("videoGen", undefined, { engine: "h3", outputVideoUrl: "/480p.mp4" });
    vi.mocked(canvasVideoUpscale).mockResolvedValue({ path: "/960p.mp4" } as never);
    vi.mocked(canvasVideoInterpolate).mockResolvedValue({ path: "/960p-48fps.mp4" } as never);
    // 超分节点走的是 canvasImageOp 那条还是视频超分，按节点实现来 —— 这里直接给它一个产物
    const u = s.addNode("upscale", undefined, { outputUrl: "/960p.mp4", sourcePath: "/480p.mp4" });
    s.setEdges([{ id: "vu", source: v, target: u, sourceHandle: "video-out", targetHandle: "prompt-in" }]);

    const i = spawnVideoTool(u, "interpolate")!;
    expect(edgeInto(i).sourceHandle).toBe("upscale-out");
    await vi.waitFor(() => expect(node(i).data.outputVideoUrl).toBe("/960p-48fps.mp4"));
    expect(canvasVideoInterpolate).toHaveBeenCalledWith("/960p.mp4");
    expect(canvasVideoInterpolate).not.toHaveBeenCalledWith("/480p.mp4");
  });

  it("视频节点后面接补帧：补的是它自己的片子，连线从 video-out 出", async () => {
    const v = useCanvasStore.getState().addNode("videoGen", undefined, { engine: "h3", outputVideoUrl: "/480p.mp4" });
    vi.mocked(canvasVideoInterpolate).mockResolvedValue({ path: "/480p-48fps.mp4" } as never);
    const i = spawnVideoTool(v, "interpolate")!;
    expect(edgeInto(i).sourceHandle).toBe("video-out");
    await vi.waitFor(() => expect(canvasVideoInterpolate).toHaveBeenCalledWith("/480p.mp4"));
  });

  it("接口名：每种源节点都对得上它自己的输出口", () => {
    expect(outHandleOf("videoGen")).toBe("video-out");
    expect(outHandleOf("upscale")).toBe("upscale-out");
    expect(outHandleOf("character")).toBe("char-out");
    expect(outHandleOf("imageGen")).toBe("image-out");
    const c = useCanvasStore.getState().addNode("character", undefined, { avatarUrl: "/a.png" });
    const u = spawnUpscale(c)!;
    expect(edgeInto(u).sourceHandle).toBe("char-out");
  });
});
