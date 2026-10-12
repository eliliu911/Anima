import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { studioCapabilities, type GenerateOutput } from "../../api/studio";
import { studioGenerate } from "./canvasTasks";
import { saveChatImageAttachment } from "../../api";
import { useCanvasStore as S, loadPersistedCanvas, getCanvasStorageKey } from "../store/useCanvasStore";
import { computeImageNodeSignature, executeNodePipeline, executeSingleNode } from "./dagExecutor";
import { canvasCatalogFixture } from "../utils/modelCatalog.fixture";
import { uploadImageInput } from "../utils/uploadImageInput";

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
vi.mock("../../api/studio", () => ({ studioCapabilities: vi.fn(), studioGenerate: vi.fn() }));
vi.mock("../../api", () => ({ saveChatImageAttachment: vi.fn() }));
let project = "";
let sequence = 0;
let fail = false;
let disk: Map<string, string>;
const result = { ok: true, path: "/result.png", message: "ok", notes: [] };
const flush = () => new Promise<void>((resolve) => setTimeout(resolve, 0));
const nodeData = (id: string) => S.getState().nodes.find((n) => n.id === id)!.data;

beforeEach(() => {
  vi.resetAllMocks();
  fail = false; disk = new Map();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => disk.get(key) ?? null,
    setItem: (key: string, value: string) => { if (fail) throw new Error("quota full"); disk.set(key, value); },
    removeItem: (key: string) => disk.delete(key),
  });
  project = `lifecycle-${++sequence}`;
  S.getState().loadCanvas(project);
  vi.mocked(studioCapabilities).mockResolvedValue(canvasCatalogFixture);
  vi.mocked(studioGenerate).mockResolvedValue(result);
});
afterEach(() => { fail = false; S.getState().saveCanvas(); vi.unstubAllGlobals(); });

describe("canvas execution lifecycle", () => {
  it.each([false, true])("retains partial batches and stops downstream (throw=%s)", async (throws) => {
    const id = S.getState().addNode("imageGen", undefined, { imageCount: 4 });
    const target = S.getState().addNode("videoGen");
    S.getState().setEdges([{ id: "e", source: id, target }]);
    vi.mocked(studioGenerate).mockResolvedValueOnce(result);
    if (throws) vi.mocked(studioGenerate).mockRejectedValueOnce(new Error("offline"));
    else vi.mocked(studioGenerate).mockResolvedValueOnce({ ...result, ok: false, path: null, message: "offline" });
    await executeNodePipeline(target);
    expect(studioGenerate).toHaveBeenCalledTimes(2);
    expect(nodeData(id).outputImages).toEqual([result.path]);
    expect(nodeData(id).error).toContain("1/4");
    expect(nodeData(id).lastExecutedSignature).not.toBe(computeImageNodeSignature(id, S.getState().nodes, S.getState().edges));
    expect(nodeData(target).error).toBeTruthy();
    vi.mocked(studioGenerate).mockClear();
    await executeNodePipeline(target);
    expect(studioGenerate).toHaveBeenCalledTimes(5);
    expect(nodeData(id).error).toBeUndefined();
  });

  it("includes seed in the input signature and preserves seed zero", async () => {
    const id = S.getState().addNode("imageGen", undefined, { seed: 0 });
    const original = computeImageNodeSignature(id, S.getState().nodes, []);
    await executeSingleNode(id);
    expect(studioGenerate).toHaveBeenCalledWith(expect.objectContaining({ seed: 0 }));
    S.getState().updateNodeData(id, { seed: 1 });
    expect(computeImageNodeSignature(id, S.getState().nodes, [])).not.toBe(original);
  });

  it("freezes the submitted signature and stops downstream after an in-flight edit", async () => {
    const id = S.getState().addNode("imageGen", undefined, { prompt: "old" });
    const target = S.getState().addNode("videoGen");
    S.getState().setEdges([{ id: "e", source: id, target }]);
    const original = computeImageNodeSignature(id, S.getState().nodes, S.getState().edges);
    let finish!: (out: GenerateOutput) => void;
    vi.mocked(studioGenerate).mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const run = executeNodePipeline(target); await flush();
    S.getState().updateNodeData(id, { prompt: "new" });
    finish(result); await run;
    expect(nodeData(id).lastExecutedSignature).toBe(original);
    expect(nodeData(id).lastExecutedSignature).not.toBe(computeImageNodeSignature(id, S.getState().nodes, S.getState().edges));
    expect(studioGenerate).toHaveBeenCalledTimes(1);
    expect(nodeData(target).error).toBeTruthy();
  });

  it("finishes the pipeline in its original project after switching away", async () => {
    const id = S.getState().addNode("imageGen");
    const target = S.getState().addNode("videoGen");
    S.getState().setEdges([{ id: "e", source: id, target }]);
    let finish!: (out: GenerateOutput) => void;
    vi.mocked(studioGenerate).mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const run = executeNodePipeline(target); await flush();
    S.getState().loadCanvas(`${project}-B`);
    finish(result); await run;
    expect(S.getState().nodes).toEqual([]);
    expect(studioGenerate).toHaveBeenCalledTimes(2);
    S.getState().loadCanvas(project);
    expect(nodeData(id).outputImageUrl).toBe(result.path);
    expect(nodeData(id).isGenerating).toBe(false);
    expect(nodeData(target).outputVideoUrl).toBe(result.path);
  });

  it("preserves live busy state when switching back before the job finishes", async () => {
    const id = S.getState().addNode("imageGen");
    let finish!: (out: GenerateOutput) => void;
    vi.mocked(studioGenerate).mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const run = executeSingleNode(id); await flush();
    S.getState().loadCanvas(`${project}-B`); S.getState().loadCanvas(project);
    expect(nodeData(id).isGenerating).toBe(true);
    const duplicate = executeSingleNode(id);
    finish(result); await Promise.all([run, duplicate]);
    expect(studioGenerate).toHaveBeenCalledTimes(1);
    expect(nodeData(id).isGenerating).toBe(false);
  });

  it("disk snapshots and legacy restores clear transient locks but keep outputs", () => {
    const id = S.getState().addNode("imageGen", undefined, { isGenerating: true, referenceLoading: true, outputImageUrl: "/old.png" });
    S.getState().saveCanvas();
    const saved = loadPersistedCanvas(project).nodes[0].data;
    expect(saved.isGenerating).toBe(false); expect(saved.referenceLoading).toBe(false);
    expect(saved.outputImageUrl).toBe("/old.png"); expect(saved.error).toContain("中断");
    expect(nodeData(id).isGenerating).toBe(true);
    disk.set(getCanvasStorageKey("legacy"), JSON.stringify({ nodes: S.getState().nodes, edges: [] }));
    expect(loadPersistedCanvas("legacy").nodes[0].data.isGenerating).toBe(false);
  });

  it("save failure is visible, retains success timestamp and blocks project switch", () => {
    S.getState().addNode("imageGen"); S.getState().saveCanvas();
    const savedAt = S.getState().lastSavedAt;
    fail = true;
    expect(S.getState().saveCanvas()).toBe(false);
    expect(S.getState().saveError).toContain("quota full");
    expect(S.getState().lastSavedAt).toBe(savedAt);
    S.getState().loadCanvas(`${project}-B`);
    expect(S.getState().activeProjectId).toBe(project);
    fail = false;
    expect(S.getState().saveCanvas()).toBe(true);
    expect(S.getState().saveError).toBeNull();
  });

  it("auto-save failures expose an error instead of success", async () => {
    vi.useFakeTimers();
    try {
      S.getState().saveCanvas();
      const savedAt = S.getState().lastSavedAt;
      fail = true;
      S.getState().addNode("prompt");
      expect(S.getState().isSaving).toBe(true);
      await vi.advanceTimersByTimeAsync(400);
      expect(S.getState().isSaving).toBe(false);
      expect(S.getState().saveError).toContain("quota full");
      expect(S.getState().lastSavedAt).toBe(savedAt);
    } finally { vi.useRealTimers(); }
  });

  it("does not share a lock across projects even when node ids match", async () => {
    const id = S.getState().addNode("imageGen");
    const clone = structuredClone(S.getState().nodes[0]);
    const finishes: Array<(out: GenerateOutput) => void> = [];
    vi.mocked(studioGenerate).mockImplementation(() => new Promise((resolve) => { finishes.push(resolve); }));
    const first = executeSingleNode(id); await flush();
    S.getState().loadCanvas(`${project}-B`); S.getState().setNodes([clone]);
    const second = executeSingleNode(id); await flush();
    expect(studioGenerate).toHaveBeenCalledTimes(2);
    finishes[0]({ ...result, path: "/A.png" }); finishes[1]({ ...result, path: "/B.png" });
    await Promise.all([first, second]);
    expect(nodeData(id).outputImageUrl).toBe("/B.png");
    expect(S.getState().getProjectCanvas(project).nodes[0].data.outputImageUrl).toBe("/A.png");
  });

  it("retains an inactive project's completed result when saving fails, then retries", async () => {
    const id = S.getState().addNode("imageGen");
    let finish!: (out: GenerateOutput) => void;
    vi.mocked(studioGenerate).mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const run = executeSingleNode(id); await flush(); S.getState().loadCanvas(`${project}-B`);
    fail = true; finish(result); await run;
    expect(S.getState().saveError).toBeTruthy();
    expect(S.getState().getProjectCanvas(project).nodes[0].data.outputImageUrl).toBe(result.path);
    fail = false; expect(S.getState().saveCanvas()).toBe(true);
    expect(loadPersistedCanvas(project).nodes[0].data.outputImageUrl).toBe(result.path);
  });

  it("returns uploads to the original project too", async () => {
    const id = S.getState().addNode("imageGen");
    vi.stubGlobal("FileReader", class { result = "data:image/png;base64,YQ=="; onload?: () => void; readAsDataURL() { this.onload?.(); } });
    let finish!: (out: { path: string; file_url: string }) => void;
    vi.mocked(saveChatImageAttachment).mockImplementation(() => new Promise((resolve) => { finish = resolve; }));
    const run = uploadImageInput(id, { name: "a.png", type: "image/png" } as File, "source");
    await flush(); S.getState().loadCanvas(`${project}-B`);
    finish({ path: "/upload.png", file_url: "file:///upload.png" }); await run;
    S.getState().loadCanvas(project);
    expect(nodeData(id).outputImageUrl).toBe("/upload.png");
    expect(nodeData(id).referenceLoading).toBe(false);
  });
});
