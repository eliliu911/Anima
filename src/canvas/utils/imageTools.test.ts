import { describe, it, expect, vi, beforeEach } from "vitest";
import { useCanvasStore } from "../store/useCanvasStore";
import { spawnImageTool, spawnVideoInterpolate, spawnVideoTool } from "./imageTools";
import { executeSingleNode } from "../engine/dagExecutor";

vi.mock("../engine/canvasTasks", () => ({
  // 画布生成改走后台任务（canvasTasks.ts）；测试里直接替身，inNode 原样执行
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
import { studioGenerate } from "../engine/canvasTasks";
import { canvasImageOp, canvasVideoInterpolate, canvasVideoUpscale } from "../engine/canvasTasks";
import { canvasCatalogFixture } from "./modelCatalog.fixture";

const state = () => useCanvasStore.getState();

describe("图片工具：在右边新建节点，接上原图再跑", () => {
  beforeEach(() => {
    state().clearCanvas();
    vi.clearAllMocks();
    vi.mocked(studioCapabilities).mockResolvedValue(canvasCatalogFixture);
  });

  it("抠图走图片工具命令，原图节点不动", async () => {
    const src = state().addNode("imageGen", undefined, { outputImageUrl: "/girl.png" });
    vi.mocked(canvasImageOp).mockResolvedValue({ path: "/cut.png" });
    const id = spawnImageTool(src, "remove_bg")!;
    await vi.waitFor(() => expect(state().nodes.find((n) => n.id === id)?.data.outputImageUrl).toBe("/cut.png"));
    expect(canvasImageOp).toHaveBeenCalledWith("/girl.png", "remove_bg");
    expect(state().nodes.find((n) => n.id === src)?.data.outputImageUrl).toBe("/girl.png");
    expect(state().edges.some((e) => e.source === src && e.target === id)).toBe(true);
  });

  it("四视图走 Krea 2 版式，源图当输入", async () => {
    const src = state().addNode("imageGen", undefined, { outputImageUrl: "/girl.png" });
    const id = state().addNode("imageGen", undefined, { imageTool: "quad_view" });
    state().setEdges([{ id: "e", source: src, target: id }]);
    vi.mocked(studioGenerate).mockResolvedValue({ ok: true, path: "/quad.png", message: "", notes: [] });
    expect(await executeSingleNode(id)).toBe(true);
    expect(studioGenerate).toHaveBeenCalledWith(expect.objectContaining({ mode: "quad_view", engine: "krea2", images: ["/girl.png"] }));
  });

  it("工具节点没连源图时报原因", async () => {
    const id = state().addNode("imageGen", undefined, { imageTool: "upscale" });
    expect(await executeSingleNode(id)).toBe(false);
    expect(canvasImageOp).not.toHaveBeenCalled();
    expect(state().nodes.find((n) => n.id === id)?.data.error).toContain("源图");
  });

  it("补帧吃上游视频节点的视频", async () => {
    const src = state().addNode("videoGen", undefined, { outputVideoUrl: "/a.mp4" });
    vi.mocked(canvasVideoInterpolate).mockResolvedValue({ path: "/a48.mp4" });
    const id = spawnVideoInterpolate(src)!;
    await vi.waitFor(() => expect(state().nodes.find((n) => n.id === id)?.data.outputVideoUrl).toBe("/a48.mp4"));
    expect(canvasVideoInterpolate).toHaveBeenCalledWith("/a.mp4");
  });

  it("视频上的「超分 ×2」建的是超分节点，吃上游视频，原视频不动", async () => {
    const src = state().addNode("videoGen", undefined, { outputVideoUrl: "/a.mp4" });
    vi.mocked(canvasVideoUpscale).mockResolvedValue({ path: "/a2x.mp4" });
    const id = spawnVideoTool(src, "upscale")!;
    expect(state().nodes.find((n) => n.id === id)?.type).toBe("upscale");
    await vi.waitFor(() => expect(state().nodes.find((n) => n.id === id)?.data.outputUrl).toBe("/a2x.mp4"));
    expect(canvasVideoUpscale).toHaveBeenCalledWith("/a.mp4");
    expect(canvasVideoInterpolate).not.toHaveBeenCalled();
    expect(canvasImageOp).not.toHaveBeenCalled();
    expect(state().nodes.find((n) => n.id === id)?.data.sourcePath).toBe("/a.mp4");
    expect(state().nodes.find((n) => n.id === src)?.data.outputVideoUrl).toBe("/a.mp4");
  });

  it("图片工具条的「超分 ×2」也是超分节点，走图片超分", async () => {
    const src = state().addNode("imageGen", undefined, { outputImageUrl: "/girl.png" });
    vi.mocked(canvasImageOp).mockResolvedValue({ path: "/girl2x.png" });
    const id = spawnImageTool(src, "upscale")!;
    expect(state().nodes.find((n) => n.id === id)?.type).toBe("upscale");
    await vi.waitFor(() => expect(state().nodes.find((n) => n.id === id)?.data.outputUrl).toBe("/girl2x.png"));
    expect(canvasImageOp).toHaveBeenCalledWith("/girl.png", "upscale");
    expect(canvasVideoUpscale).not.toHaveBeenCalled();
  });
});
