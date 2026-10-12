import { describe, it, expect, vi, beforeEach } from "vitest";
import { executeNodePipeline, executeSingleNode } from "./dagExecutor";
import { useCanvasStore } from "../store/useCanvasStore";
import { inferVideoMode } from "../utils/videoMode";
import { motionSpec, upstreamDepth } from "../utils/depthChain";
import { canvasCatalogFixture } from "../utils/modelCatalog.fixture";

vi.mock("./canvasTasks", () => ({
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
import { studioGenerate } from "./canvasTasks";
import { canvasDepthCapture, canvasDepthVideo } from "./canvasTasks";

/** 源视频 → 深度 → 视频（+ 人物图） */
function chain(opts: { depth?: string; sourceVideoPath?: string; image?: boolean; prompt?: string } = {}) {
  const s = useCanvasStore.getState();
  const src = s.addNode("videoGen", undefined, { outputVideoUrl: "/dance.mp4", uploadedVideo: true });
  const depth = s.addNode("depthCapture", undefined, {
    ...(opts.depth ? { depthVideoUrl: opts.depth, sourceVideoPath: opts.sourceVideoPath ?? "/dance.mp4" } : {}),
  });
  const out = s.addNode("videoGen", undefined, { prompt: opts.prompt ?? "校服女孩在白色棚里跳街舞" });
  s.setEdges([
    { id: "e1", source: src, target: depth },
    { id: "e2", source: depth, target: out },
  ]);
  if (opts.image !== false) s.bindReference(s.activeProjectId, out, { path: "/girl.png", name: "林晚" });
  return { src, depth, out };
}

const state = () => useCanvasStore.getState();

describe("深度动作链（照 libtv）", () => {
  beforeEach(() => {
    state().clearCanvas();
    vi.clearAllMocks();
    vi.mocked(studioCapabilities).mockResolvedValue(canvasCatalogFixture);
  });

  it("视频节点接了深度节点 = 动作迁移，一张人物图就换一个人", () => {
    const { out } = chain();
    const got = inferVideoMode(out, state().nodes, state().edges);
    expect(got.mode).toBe("motion");
    expect(got.images).toEqual(["/girl.png"]);
    expect(got.reason).toContain("林晚");
  });

  it("连两张人物图：两个都换，按连线顺序从左到右对应，不再只用第一张", () => {
    const { out } = chain();
    state().bindReference(state().activeProjectId, out, { path: "/boy.png", name: "沈昭" });
    const got = inferVideoMode(out, state().nodes, state().edges);
    expect(got.mode).toBe("motion");
    expect(got.images).toEqual(["/girl.png", "/boy.png"]);
    expect(got.reason).toContain("换 2 个人");
    expect(got.reason).toContain("第 1 张「林晚」换从左数第 1 个人");
    expect(got.reason).toContain("第 2 张「沈昭」换从左数第 2 个人");
    expect(got.reason).not.toContain("只用第一张");
  });

  it("多人出片：所有人物图按顺序传给后端；控制信号默认轮廓线，选了深度就传深度", async () => {
    const { out } = chain({ depth: "/depth.mp4" });
    state().bindReference(state().activeProjectId, out, { path: "/boy.png", name: "沈昭" });
    vi.mocked(canvasDepthVideo).mockResolvedValue({ path: "/new.mp4", first_frame: "/ff.png", notes: [] });
    await executeSingleNode(out);
    expect(canvasDepthVideo).toHaveBeenLastCalledWith(expect.objectContaining({
      imagePath: "/girl.png", imagePaths: ["/girl.png", "/boy.png"], control: "canny",
    }));
    state().updateNodeData(out, { motionControl: "depth" });
    await executeSingleNode(out);
    expect(canvasDepthVideo).toHaveBeenLastCalledWith(expect.objectContaining({ control: "depth" }));
  });

  it("没连人物图时说清楚还缺什么", () => {
    const { out } = chain({ image: false });
    const got = inferVideoMode(out, state().nodes, state().edges);
    expect(got.mode).toBe("motion");
    expect(got.reason).toContain("人物图");
  });

  /** 源视频换了、深度还是旧的 —— 拿旧动作出片不报错，所以必须拦。 */
  it("上游换了视频，深度算过期", () => {
    const { out } = chain({ depth: "/depth.mp4", sourceVideoPath: "/old.mp4" });
    expect(upstreamDepth(out, state().nodes, state().edges)?.stale).toBe(true);
  });

  it("深度节点捕捉的是上游视频节点的视频", async () => {
    const { depth } = chain();
    vi.mocked(canvasDepthCapture).mockResolvedValue({ path: "/depth.mp4" });
    expect(await executeSingleNode(depth)).toBe(true);
    expect(canvasDepthCapture).toHaveBeenCalledWith("/dance.mp4");
    const d = state().nodes.find((n) => n.id === depth)!.data;
    expect(d.depthVideoUrl).toBe("/depth.mp4");
    expect(d.sourceVideoPath).toBe("/dance.mp4");
  });

  it("深度节点没连视频时报原因，不调后端", async () => {
    const depth = state().addNode("depthCapture");
    expect(await executeSingleNode(depth)).toBe(false);
    expect(canvasDepthCapture).not.toHaveBeenCalled();
    expect(state().nodes.find((n) => n.id === depth)!.data.error).toContain("视频节点");
  });

  it("原片复刻 · 快：深度节点不用跑，原视频取深度节点上游的那个视频，节点上的 seed 照传", async () => {
    const { out, depth } = chain();
    state().updateNodeData(out, { motionEngine: "h3_ref", durationSecs: 8.542, seed: 62001 });
    vi.mocked(canvasDepthVideo).mockResolvedValue({ path: "/new.mp4", first_frame: "", notes: [] });
    expect(await executeNodePipeline(out)).toBeUndefined();
    expect(canvasDepthCapture).not.toHaveBeenCalled();
    expect(canvasDepthVideo).toHaveBeenLastCalledWith(expect.objectContaining({
      engine: "h3_ref", depthPath: "", sourceVideoPath: "/dance.mp4", durationSecs: 8.542, seed: 62001,
    }));
    expect(state().nodes.find((n) => n.id === out)!.data.error).toBeUndefined();
    expect(state().nodes.find((n) => n.id === depth)!.data.depthVideoUrl).toBeUndefined();
  });

  it("原片复刻 · 快：上游没有视频就说清楚，不调后端", async () => {
    const s = state();
    const depth = s.addNode("depthCapture");
    const out = s.addNode("videoGen", undefined, { prompt: "p", motionEngine: "h3_ref" });
    s.setEdges([{ id: "e2", source: depth, target: out }]);
    s.bindReference(s.activeProjectId, out, { path: "/girl.png", name: "林晚" });
    expect(await executeSingleNode(out)).toBe(false);
    expect(canvasDepthVideo).not.toHaveBeenCalled();
    expect(state().nodes.find((n) => n.id === out)!.data.error).toContain("视频节点");
  });

  it("没写 seed 就不传，后端自己随机（以前的行为不变）", async () => {
    const { out } = chain({ depth: "/depth.mp4" });
    vi.mocked(canvasDepthVideo).mockResolvedValue({ path: "/new.mp4", first_frame: "", notes: [] });
    await executeSingleNode(out);
    expect(vi.mocked(canvasDepthVideo).mock.lastCall![0].seed).toBeUndefined();
  });

  /** 原视频直连出片（不经过深度节点）= 原片复刻 —— 用户 09-24：「既然不用深度跟踪，为啥还要画这个节点」 */
  function direct(opts: { engine?: string } = { engine: "h3_ref" }) {
    const s = useCanvasStore.getState();
    const src = s.addNode("videoGen", undefined, { outputVideoUrl: "/part01.mp4", uploadedVideo: true });
    const out = s.addNode("videoGen", undefined, { prompt: "把参考视频1中的角色替换成图片1", ...(opts.engine && opts.engine !== "none" ? { motionEngine: opts.engine } : {}) });
    s.setEdges([{ id: "e1", source: src, target: out }]);
    s.bindReference(s.activeProjectId, out, { path: "/a.png", name: "角色A" });
    s.bindReference(s.activeProjectId, out, { path: "/b.png", name: "角色B" });
    return { src, out };
  }

  it("原视频直接连出片节点 = 原片复刻，两张图按连线顺序是图片1、图片2", () => {
    const { out } = direct();
    const got = inferVideoMode(out, state().nodes, state().edges);
    expect(got).toMatchObject({ mode: "motion", via: "video", images: ["/a.png", "/b.png"] });
    expect(got.reason).toContain("原片复刻");
    expect(got.reason).toContain("图片1「角色A」、图片2「角色B」");
  });

  it("直连时按「原片复刻 · 快」跑，原视频就是连进来的那段", async () => {
    const { out } = direct();
    vi.mocked(canvasDepthVideo).mockResolvedValue({ path: "/new.mp4", first_frame: "", notes: [] });
    expect(await executeSingleNode(out)).toBe(true);
    expect(canvasDepthVideo).toHaveBeenLastCalledWith(expect.objectContaining({
      engine: "h3_ref", depthPath: "", sourceVideoPath: "/part01.mp4", imagePaths: ["/a.png", "/b.png"],
    }));
  });

  /** 旧画布里长镜头窗口链那种视频连视频的线还在 —— 光凭连线切成原片复刻，点一下就是一小时 */
  it("视频直连视频、但节点没选原片复刻：不是动作迁移，不偷偷切模式", async () => {
    for (const engine of [undefined, "ltx", "h3"]) {
      state().clearCanvas();
      const { out } = direct({ engine: engine ?? "none" });
      if (!engine) state().updateNodeData(out, { motionEngine: undefined });
      expect(inferVideoMode(out, state().nodes, state().edges).mode).not.toBe("motion");
    }
  });

  it("老画布里存着原生那档（h3_native，09-24 连模型删了）：还算原片复刻，按「原片复刻 · 快」跑", async () => {
    const { out } = direct({ engine: "h3_native" });
    expect(inferVideoMode(out, state().nodes, state().edges)).toMatchObject({ mode: "motion", via: "video" });
    vi.mocked(canvasDepthVideo).mockResolvedValue({ path: "/new.mp4", first_frame: "", notes: [] });
    await executeSingleNode(out);
    expect(canvasDepthVideo).toHaveBeenLastCalledWith(expect.objectContaining({ engine: "h3_ref", sourceVideoPath: "/part01.mp4" }));
  });

  it("老画布：接着深度节点、存的是原生那档 —— 也按「原片复刻 · 快」跑，深度不用跑", async () => {
    const { out } = chain();
    state().updateNodeData(out, { motionEngine: "h3_native" });
    vi.mocked(canvasDepthVideo).mockResolvedValue({ path: "/new.mp4", first_frame: "", notes: [] });
    await executeNodePipeline(out);
    expect(canvasDepthCapture).not.toHaveBeenCalled();
    expect(canvasDepthVideo).toHaveBeenLastCalledWith(expect.objectContaining({ engine: "h3_ref", sourceVideoPath: "/dance.mp4" }));
  });

  it("补帧 / 超分这两个工具节点也是视频连视频，不能被当成原片复刻", () => {
    const s = useCanvasStore.getState();
    const src = s.addNode("videoGen", undefined, { outputVideoUrl: "/x.mp4", uploadedVideo: true });
    const tool = s.addNode("videoGen", undefined, { videoTool: "interpolate" });
    s.setEdges([{ id: "e1", source: src, target: tool }]);
    expect(inferVideoMode(tool, state().nodes, state().edges).mode).not.toBe("motion");
  });

  it("接了深度节点时还是原来的四档，默认动作精准", async () => {
    const { out } = chain({ depth: "/depth.mp4" });
    expect(inferVideoMode(out, state().nodes, state().edges).via).toBe("depth");
    vi.mocked(canvasDepthVideo).mockResolvedValue({ path: "/new.mp4", first_frame: "", notes: [] });
    await executeSingleNode(out);
    expect(canvasDepthVideo).toHaveBeenLastCalledWith(expect.objectContaining({ engine: "ltx", depthPath: "/depth.mp4" }));
  });

  it("动作迁移走深度出片命令，不走普通出片", async () => {
    const { out } = chain({ depth: "/depth.mp4" });
    vi.mocked(canvasDepthVideo).mockResolvedValue({ path: "/new.mp4", first_frame: "/ff.png", notes: ["768×512"] });
    expect(await executeSingleNode(out)).toBe(true);
    expect(studioGenerate).not.toHaveBeenCalled();
    expect(canvasDepthVideo).toHaveBeenCalledWith(expect.objectContaining({
      depthPath: "/depth.mp4", imagePath: "/girl.png", prompt: expect.stringContaining("跳街舞"),
    }));
    const d = state().nodes.find((n) => n.id === out)!.data;
    expect(d.outputVideoUrl).toBe("/new.mp4");
    expect(d.motionFirstFrame).toBe("/ff.png");
  });

  it("选了人物清晰就把 h3 传给后端，默认是 ltx", async () => {
    const { out } = chain({ depth: "/depth.mp4" });
    vi.mocked(canvasDepthVideo).mockResolvedValue({ path: "/new.mp4", first_frame: "", notes: [] });
    await executeSingleNode(out);
    expect(canvasDepthVideo).toHaveBeenLastCalledWith(expect.objectContaining({ engine: "ltx" }));
    state().updateNodeData(out, { motionEngine: "h3" });
    await executeSingleNode(out);
    expect(canvasDepthVideo).toHaveBeenLastCalledWith(expect.objectContaining({ engine: "h3" }));
    state().updateNodeData(out, { motionEngine: "ltx", resolution: "1080P", durationSecs: 8 });
    await executeSingleNode(out);
    expect(canvasDepthVideo).toHaveBeenLastCalledWith(expect.objectContaining({ engine: "ltx", resolution: "1080P", durationSecs: 8 }));
    state().updateNodeData(out, { motionKeepBackground: true });
    await executeSingleNode(out);
    expect(canvasDepthVideo).toHaveBeenLastCalledWith(expect.objectContaining({ keepBackground: true, sourceVideoPath: "/dance.mp4" }));
    expect(state().nodes.find((n) => n.id === out)!.data.motionFirstFrame).toBeUndefined();
  });

  /** 以前没写提示词直接报错，而面板占位文字说可以直接点生成。 */
  it("没写提示词也能出片，用默认描述并说出来", async () => {
    const { out } = chain({ depth: "/depth.mp4", prompt: "" });
    vi.mocked(canvasDepthVideo).mockResolvedValue({ path: "/new.mp4", first_frame: "", notes: [] });
    expect(await executeSingleNode(out)).toBe(true);
    expect(canvasDepthVideo).toHaveBeenCalledWith(expect.objectContaining({ prompt: expect.stringContaining("guide video") }));
    expect((state().nodes.find((n) => n.id === out)!.data.notes as string[]).join()).toContain("默认描述");
  });

  it("深度过期时拒绝出片", async () => {
    const { out } = chain({ depth: "/depth.mp4", sourceVideoPath: "/old.mp4" });
    expect(await executeSingleNode(out)).toBe(false);
    expect(canvasDepthVideo).not.toHaveBeenCalled();
    expect(state().nodes.find((n) => n.id === out)!.data.error).toContain("重新跑");
  });

  it("点动作迁移的生成时，流水线先把没跑过的深度跑掉", async () => {
    const { out } = chain();
    vi.mocked(canvasDepthCapture).mockResolvedValue({ path: "/depth.mp4" });
    vi.mocked(canvasDepthVideo).mockResolvedValue({ path: "/new.mp4", first_frame: "/ff.png", notes: [] });
    await executeNodePipeline(out);
    expect(canvasDepthCapture).toHaveBeenCalledTimes(1);
    expect(canvasDepthVideo).toHaveBeenCalledWith(expect.objectContaining({ depthPath: "/depth.mp4" }));
  });
});

describe("动作迁移的规格跟设置走", () => {
  it("按清晰度出尺寸，H3 没有 1080P", () => {
    expect(motionSpec("ltx", "480P", 5)).toMatchObject({ tier: "480P", size: "768×512", secs: 5, clipped: false, downgraded: false });
    expect(motionSpec("ltx", "720P").size).toBe("1152×768");
    expect(motionSpec("ltx", "1080P").size).toBe("1536×1024");
    expect(motionSpec("h3", "720P").size).toBe("1280×736");
    expect(motionSpec("h3", "1080P")).toMatchObject({ tier: "720P", size: "1280×736", downgraded: true });
  });
  it("时长跟设置，超过上限按上限并标出来", () => {
    expect(motionSpec("ltx", "480P").secs).toBe(5);
    expect(motionSpec("ltx", "480P", 8).secs).toBe(8);
    expect(motionSpec("h3", "480P", 15)).toMatchObject({ secs: 10, clipped: true });
  });

  it("原片复刻固定原作的 960×544（和 Rust H3N_LANDSCAPE 同一个数），不跟清晰度走，一次最长 10 秒", () => {
    for (const res of ["480P", "720P", "1080P"]) {
      expect(motionSpec("h3_ref", res, 5)).toMatchObject({ tier: "0.5 MP", size: "960×544", secs: 5, clipped: false, downgraded: false });
    }
    expect(motionSpec("h3_ref", "480P", 12)).toMatchObject({ secs: 10, clipped: true });
    // WeeTodd 节点下限 2.5 秒：设得更短按 2.5 秒出，不然 ComfyUI 拒收
    expect(motionSpec("h3_ref", "480P", 1)).toMatchObject({ secs: 2.5 });
    expect(motionSpec("h3_ref", "720P", 8.542)).toMatchObject({ tier: "0.5 MP", size: "960×544", secs: 8.542, clipped: false, downgraded: false });
  });
});
