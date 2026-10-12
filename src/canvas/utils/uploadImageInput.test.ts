import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { saveChatImageAttachment } from "../../api";
import { studioCapabilities } from "../../api/studio";
import { studioGenerate } from "../engine/canvasTasks";
import { canvasCatalogFixture } from "./modelCatalog.fixture";
import { useCanvasStore } from "../store/useCanvasStore";
import { inferVideoMode } from "./videoMode";
import { executeSingleNode, executeNodePipeline } from "../engine/dagExecutor";
import { uploadImageInput } from "./uploadImageInput";

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
vi.mock("../../api", () => ({ saveChatImageAttachment: vi.fn() }));
vi.mock("../../api/studio", () => ({ studioGenerate: vi.fn(), studioCapabilities: vi.fn() }));
const file = { name: "source.png", type: "image/png" } as File;

beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(studioCapabilities).mockResolvedValue(canvasCatalogFixture);
  useCanvasStore.getState().clearCanvas();
  vi.stubGlobal("FileReader", class {
    result = "data:image/png;base64,YQ==";
    onload?: () => void;
    readAsDataURL() { this.onload?.(); }
  });
  vi.mocked(saveChatImageAttachment).mockResolvedValue({ path: "/saved/source.png", file_url: "file:///saved/source.png" });
  vi.mocked(studioGenerate).mockResolvedValue({ ok: true, path: "/generated/out.png", message: "ok", notes: [] });
});
afterEach(() => vi.unstubAllGlobals());
const data = (id: string) => useCanvasStore.getState().nodes.find((n) => n.id === id)!.data;

describe("canvas source upload", () => {
  /**
   * 上传的源图连到视频节点：**直接用那张图，不重跑上游**。
   *
   * 模式是 `i2v` 不是 `r2v` —— **一张图就是首帧**。
   * 以前绑定这一步无条件写 `r2v`，而参考生视频只支持 H3、
   * 还要求多张带名字的身份参考；一张图标成 r2v 是错的。
   */
  it("uses connected uploaded source directly without running upstream image generation", async () => {
    const s = useCanvasStore.getState();
    const source = s.addNode("imageGen");
    await uploadImageInput(source, file, "source");
    const video = s.addNode("videoGen", undefined, { engine: "h3" });
    s.onConnect({ source, target: video, sourceHandle: "image-out", targetHandle: "prompt-in" });
    await executeNodePipeline(video);
    expect(studioGenerate).toHaveBeenCalledTimes(1);
    expect(studioGenerate).toHaveBeenCalledWith(expect.objectContaining({ mode: "i2v", images: ["/saved/source.png"] }));
  });
  /**
   * 两张上传的图 → 参考生视频。
   *
   * **钉的是推导结果**，不是存下来的 `mode` 字段 —— 绑定这一步只写 `auto`，
   * 模式由连线推导，用户实际拿到的是推导结果。
   */
  it("reference input selects R2V from auto and submits multiple images", async () => {
    const id = useCanvasStore.getState().addNode("videoGen", undefined, { mode: "auto", engine: "h3" });
    await uploadImageInput(id, file);
    vi.mocked(saveChatImageAttachment).mockResolvedValue({ path: "/second.png", file_url: "file:///second.png" });
    await uploadImageInput(id, file);
    const g = useCanvasStore.getState();
    expect(inferVideoMode(id, g.nodes, g.edges).mode).toBe("r2v");
    await executeSingleNode(id);
    expect(studioGenerate).toHaveBeenCalledWith(expect.objectContaining({ mode: "r2v", images: ["/saved/source.png", "/second.png"] }));
  });
  it("replaces one reference in place while preserving the others", async () => {
    const id = useCanvasStore.getState().addNode("videoGen", undefined, { mode: "r2v", references: [{path: "/a.png", name: "A"}, {path: "/b.png", name: "B"}] });
    await uploadImageInput(id, file, "reference", "/a.png");
    expect(data(id).references).toEqual([expect.objectContaining({path: "/saved/source.png", name: "source.png", sourceNodeId: expect.any(String)}), {path: "/b.png", name: "B"}]);
  });
  /**
   * **图片节点上传 = 素材节点。** 图生图是「素材节点 → 图片节点」两个节点（用户 2026-09-14），
   * 不是一个节点自己既放图又生图。
   */
  it("image node upload turns it into a material node; image-to-image happens downstream", async () => {
    const s = useCanvasStore.getState();
    const id = s.addNode("imageGen");
    await uploadImageInput(id, file, "source");
    expect(data(id).referenceOnly).toBe(true);
    expect(data(id).outputImageUrl).toBe("/saved/source.png");
    expect(data(id).references).toEqual([]);
    const gen = s.addNode("imageGen", undefined, { engine: "zimage", prompt: "换成夜景" });
    s.onConnect({ source: id, target: gen, sourceHandle: "image-out", targetHandle: "prompt-in" });
    await executeNodePipeline(gen);
    expect(studioGenerate).toHaveBeenCalledTimes(1);
    expect(studioGenerate).toHaveBeenCalledWith(expect.objectContaining({ mode: "i2i", images: ["/saved/source.png"] }));
  });
  it("explicit first-frame upload selects I2V without storing an image as video", async () => {
    const id = useCanvasStore.getState().addNode("videoGen", undefined, { mode: "t2v" });
    await uploadImageInput(id, file, "source");
    expect(data(id).mode).toBe("i2v");
    expect(data(id).outputVideoUrl).toBeUndefined();
    await executeSingleNode(id);
    expect(studioGenerate).toHaveBeenCalledWith(expect.objectContaining({ mode: "i2v", images: ["/saved/source.png"] }));
  });
  it("preserves input and output if saving fails, and releases the upload lock", async () => {
    const refs = [{ path: "/old.png", name: "Old" }];
    const id = useCanvasStore.getState().addNode("imageGen", undefined, { references: refs, outputImageUrl: "/result.png" });
    vi.mocked(saveChatImageAttachment).mockRejectedValue(new Error("disk full"));
    await expect(uploadImageInput(id, file, "source")).rejects.toThrow("disk full");
    expect(data(id).references).toEqual(refs);
    expect(data(id).outputImageUrl).toBe("/result.png");
    expect(data(id).referenceLoading).toBe(false);
  });
  it("rejects a video file before reading or saving it", async () => {
    const id = useCanvasStore.getState().addNode("videoGen");
    await expect(uploadImageInput(id, { name: "a.mp4", type: "video/mp4" } as File, "source")).rejects.toThrow("图片文件");
    expect(saveChatImageAttachment).not.toHaveBeenCalled();
  });
  it("blocks concurrent upload and generation while saving", async () => {
    const id = useCanvasStore.getState().addNode("imageGen");
    let finish!: (v: { path: string; file_url: string }) => void;
    vi.mocked(saveChatImageAttachment).mockImplementation(() => new Promise((resolve) => { finish = resolve; }));
    const pending = uploadImageInput(id, file, "source");
    await Promise.resolve();
    await expect(uploadImageInput(id, file)).rejects.toThrow("忙碌");
    expect(await executeSingleNode(id)).toBe(false);
    expect(studioGenerate).not.toHaveBeenCalled();
    finish({ path: "/saved.png", file_url: "file:///saved.png" });
    await pending;
    expect(data(id).referenceLoading).toBe(false);
  });
  it("does not recreate a deleted node when upload completes", async () => {
    const id = useCanvasStore.getState().addNode("imageGen");
    vi.mocked(saveChatImageAttachment).mockImplementation(async () => {
      useCanvasStore.getState().removeNode(id);
      return { path: "/saved.png", file_url: "file:///saved.png" };
    });
    await expect(uploadImageInput(id, file)).rejects.toThrow("已删除");
    expect(useCanvasStore.getState().nodes).toEqual([]);
  });
  it("reference upload keeps R2V mode and appends instead of replacing", async () => {
    const id = useCanvasStore.getState().addNode("videoGen", undefined, { mode: "r2v", references: [{ path: "/old.png", name: "Old" }] });
    await uploadImageInput(id, file);
    expect(data(id).mode).toBe("r2v");
    expect(data(id).references).toHaveLength(2);
  });
});
