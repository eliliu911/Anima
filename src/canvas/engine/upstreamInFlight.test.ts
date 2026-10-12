// EP07 续篇的连法：上传图 A（记忆消除器）→ 出图 B（续篇首帧）→ 视频 V。
// B 还在跑、还没出图的时候，V 拿到的首帧必须是 B 的，不能越过 B 落到 A 上。
import { describe, it, expect, vi, beforeEach } from "vitest";
import { aggregateNodeInputs, executeNodePipeline } from "./dagExecutor";
import { useCanvasStore } from "../store/useCanvasStore";
import { canvasCatalogFixture } from "../utils/modelCatalog.fixture";

vi.mock("./canvasTasks", () => ({
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
import { studioGenerate } from "./canvasTasks";

function graph(bData: Record<string, unknown> = {}) {
  const s = useCanvasStore.getState();
  const a = s.addNode("imageGen", undefined, { outputImageUrl: "/neuralyzer.png", prompt: "" });
  const b = s.addNode("imageGen", undefined, { engine: "zimage", prompt: "续篇首帧", ...bData });
  const v = s.addNode("videoGen", undefined, { engine: "h3", durationSecs: 5, prompt: "续篇" });
  s.setEdges([
    { id: "ab", source: a, target: b },
    { id: "bv", source: b, target: v },
  ]);
  return { a, b, v };
}
const node = (id: string) => useCanvasStore.getState().nodes.find((n) => n.id === id)!;
const videoCalls = () => vi.mocked(studioGenerate).mock.calls.map((c) => c[0]).filter((a) => a.mode !== "t2i" && a.mode !== "i2i" && a.mode !== "ref2i");
const imageCalls = () => vi.mocked(studioGenerate).mock.calls.map((c) => c[0]).filter((a) => a.mode === "t2i" || a.mode === "i2i" || a.mode === "ref2i");

describe("上游还在跑的时候点下游", () => {
  beforeEach(() => {
    useCanvasStore.getState().clearCanvas();
    vi.clearAllMocks();
    vi.mocked(studioCapabilities).mockResolvedValue(canvasCatalogFixture);
  });

  it("直接上游没出图时，首帧是空的 —— 不越过它拿更上游那张", () => {
    const { v } = graph();
    const { nodes, edges } = useCanvasStore.getState();
    const inputs = aggregateNodeInputs(v, nodes, edges);
    expect(inputs.inputImageUrl).toBeNull();
    // 卡片上显示「等待上游图片生成首帧」要靠这个指向 B，不是 A
    expect(inputs.upstreamImageNode?.prompt).toBe("续篇首帧");
  });

  it("直接上游出了图，首帧就是它", () => {
    const { v } = graph({ outputImageUrl: "/sequel-first.png" });
    const { nodes, edges } = useCanvasStore.getState();
    expect(aggregateNodeInputs(v, nodes, edges).inputImageUrl).toBe("/sequel-first.png");
  });

  it("先点 B 再点 V：B 只提交一次，V 用 B 这一次的图", async () => {
    const { b, v } = graph();
    let finishB!: (x: unknown) => void;
    vi.mocked(studioGenerate).mockImplementation((args) =>
      args.mode === "i2i" || args.mode === "t2i" || args.mode === "ref2i"
        ? (new Promise((r) => { finishB = r; }) as never)
        : (Promise.resolve({ ok: true, path: "/v.mp4", message: "ok", notes: [] }) as never));
    const runB = executeNodePipeline(b);
    await vi.waitFor(() => expect(imageCalls()).toHaveLength(1));
    const runV = executeNodePipeline(v);
    await new Promise((r) => setTimeout(r, 20));
    expect(imageCalls()).toHaveLength(1);
    expect(videoCalls()).toHaveLength(0);
    finishB({ ok: true, path: "/sequel-first.png", message: "ok", notes: [] });
    await Promise.all([runB, runV]);
    expect(imageCalls()).toHaveLength(1);
    expect(videoCalls()).toHaveLength(1);
    expect(videoCalls()[0].images).toEqual(["/sequel-first.png"]);
    expect(node(v).data.outputVideoUrl).toBe("/v.mp4");
  });

  it("B 是重新打开画布后接着跑的后台任务（没有执行锁）：点 V 不重复提交 B，等它跑完", async () => {
    const { b, v } = graph({ isGenerating: true, jobId: "job-1" });
    vi.mocked(studioGenerate).mockResolvedValue({ ok: true, path: "/v.mp4", message: "ok", notes: [] } as never);
    const runV = executeNodePipeline(v);
    await new Promise((r) => setTimeout(r, 20));
    expect(imageCalls()).toHaveLength(0);
    expect(videoCalls()).toHaveLength(0);
    // 后台任务取回结果
    useCanvasStore.getState().updateNodeData(b, { isGenerating: false, jobId: undefined, outputImageUrl: "/sequel-first.png" });
    await runV;
    expect(imageCalls()).toHaveLength(0);
    expect(videoCalls()).toHaveLength(1);
    expect(videoCalls()[0].images).toEqual(["/sequel-first.png"]);
  });

  it("等着的上游失败了：V 不跑，报上游失败", async () => {
    const { b, v } = graph({ isGenerating: true, jobId: "job-1" });
    const runV = executeNodePipeline(v);
    await new Promise((r) => setTimeout(r, 20));
    useCanvasStore.getState().updateNodeData(b, { isGenerating: false, jobId: undefined, error: "ComfyUI 挂了" });
    await runV;
    expect(videoCalls()).toHaveLength(0);
    expect(String(node(v).data.error)).toContain("上游");
  });
});
