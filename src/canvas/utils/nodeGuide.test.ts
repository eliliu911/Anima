import { describe, it, expect, vi, beforeEach } from "vitest";
import { nodeGuide } from "./nodeGuide";
import { toTemplate } from "./workflowTemplate";
import { useCanvasStore } from "../store/useCanvasStore";
import { executeSingleNode } from "../engine/dagExecutor";
import { canvasCatalogFixture } from "./modelCatalog.fixture";
import type { CanvasNode } from "../types";

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
import { studioGenerate } from "../engine/canvasTasks";

const STEP = "【第 3 步 / 共 7 步】点「生成」出教室首帧";

describe("节点的模板说明 guide", () => {
  beforeEach(() => {
    useCanvasStore.getState().clearCanvas();
    vi.clearAllMocks();
    vi.mocked(studioCapabilities).mockResolvedValue(canvasCatalogFixture);
  });

  it("只认字符串数组；类型不对当没有", () => {
    expect(nodeGuide({ guide: [STEP, "看图验收"] })).toEqual([STEP, "看图验收"]);
    expect(nodeGuide({ guide: [STEP, 3, "", null] })).toEqual([STEP]);
    expect(nodeGuide({ guide: "一句话" })).toEqual([]);
    expect(nodeGuide(undefined)).toEqual([]);
  });

  it("另存模板：guide 留着，notes（运行提示）剥掉", () => {
    const node = { id: "n1", type: "imageGen", position: { x: 0, y: 0 },
      data: { engine: "zimage", prompt: "教室", guide: [STEP], notes: ["上次运行：用了 45 秒"] } } as unknown as CanvasNode;
    const t = toTemplate("EP07", [node], []);
    const d = t.nodes[0].data as Record<string, unknown>;
    expect(d.guide).toEqual([STEP]);
    expect(d.notes).toBeUndefined();
  });

  it("点生成：notes 换成这次的运行提示，guide 不动", async () => {
    const s = useCanvasStore.getState();
    const id = s.addNode("imageGen", undefined, { engine: "zimage", prompt: "教室", guide: [STEP], notes: ["旧的运行提示"] });
    vi.mocked(studioGenerate).mockResolvedValue({ ok: true, path: "/out.png", message: "ok", notes: ["这次的运行提示"] } as never);
    await executeSingleNode(id);
    const d = useCanvasStore.getState().nodes.find((n) => n.id === id)!.data as Record<string, unknown>;
    expect(d.outputImageUrl).toBe("/out.png");
    expect(d.guide).toEqual([STEP]);
    expect(d.notes).not.toContain("旧的运行提示");
  });
});
