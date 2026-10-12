import { describe, it, expect, vi, beforeEach } from "vitest";
import { useCanvasStore, recoverNodes } from "../store/useCanvasStore";

vi.mock("../../api/canvasJobs", () => ({ canvasJobStart: vi.fn(), waitForCanvasJob: vi.fn(), canvasJobGet: vi.fn() }));
import { canvasJobStart, waitForCanvasJob } from "../../api/canvasJobs";
import { inNode, canvasDepthVideo, liveJobs } from "./canvasTasks";
import { resumeCanvasJobs } from "./resumeJobs";
import type { Job } from "../../api/jobs";

const state = () => useCanvasStore.getState();
const job = (over: Partial<Job>): Job => ({
  id: "job_1", kind: "canvas", status: "done", project_id: "", episode_id: "", shot_id: "", shot_num: 0,
  prompt: "动作迁移", created_at: 0, ...over,
});

describe("画布生成走后台任务", () => {
  beforeEach(() => {
    state().clearCanvas();
    vi.clearAllMocks();
    liveJobs.clear();
  });

  /** 任务 id 要在等结果期间一直记在节点上 —— 页面这时候重新加载，就靠它找回来 */
  it("发起时把任务 id 记到节点上，结束后清掉", async () => {
    const id = state().addNode("videoGen");
    vi.mocked(canvasJobStart).mockResolvedValue("job_1");
    let release!: (j: Job) => void;
    vi.mocked(waitForCanvasJob).mockImplementation(() => new Promise((r) => { release = r; }));
    const run = inNode({ projectId: state().activeProjectId, nodeId: id, label: "动作迁移" },
      () => canvasDepthVideo({ depthPath: "/d.mp4", imagePath: "/g.png", prompt: "跳舞" }));
    await vi.waitFor(() => expect(state().nodes.find((n) => n.id === id)?.data.jobId).toBe("job_1"));
    expect(canvasJobStart).toHaveBeenCalledWith(expect.objectContaining({ nodeId: id, label: "动作迁移",
      task: expect.objectContaining({ kind: "depth_video" }) }));
    release(job({ result: { path: "/v.mp4", first_frame: "", notes: [] } }));
    await expect(run).resolves.toEqual({ path: "/v.mp4", first_frame: "", notes: [] });
    expect(state().nodes.find((n) => n.id === id)?.data.jobId).toBeUndefined();
  });

  it("任务失败时把原因抛出来", async () => {
    const id = state().addNode("videoGen");
    vi.mocked(canvasJobStart).mockResolvedValue("job_1");
    vi.mocked(waitForCanvasJob).mockResolvedValue(job({ status: "failed", error: "ComfyUI 没开" }));
    await expect(inNode({ projectId: state().activeProjectId, nodeId: id, label: "补帧" },
      () => canvasDepthVideo({ depthPath: "/d.mp4", imagePath: "/g.png", prompt: "" }))).rejects.toThrow("ComfyUI 没开");
  });

  /** 以前存画布时生成中的节点一律判「已中断」 */
  it("挂着任务的节点重新加载后还是生成中，不判中断", () => {
    const id = state().addNode("videoGen", undefined, { isGenerating: true, jobId: "job_1" });
    const other = state().addNode("videoGen", undefined, { isGenerating: true });
    // 存盘和读盘都过 recoverNodes
    const nodes = recoverNodes(state().nodes);
    expect(nodes.find((n) => n.id === id)?.data).toMatchObject({ isGenerating: true, jobId: "job_1" });
    expect(nodes.find((n) => n.id === id)?.data.error).toBeUndefined();
    expect(nodes.find((n) => n.id === other)?.data.error).toContain("中断");
  });

  it("重新加载后按任务 id 把结果写回节点", async () => {
    const id = state().addNode("videoGen", undefined, { isGenerating: true, jobId: "job_1", outputVideos: ["/old.mp4"] });
    vi.mocked(waitForCanvasJob).mockResolvedValue(job({ result: { path: "/v.mp4", first_frame: "/ff.png", notes: ["768×512"] } }));
    resumeCanvasJobs(state().activeProjectId);
    await vi.waitFor(() => expect(state().nodes.find((n) => n.id === id)?.data.outputVideoUrl).toBe("/v.mp4"));
    const d = state().nodes.find((n) => n.id === id)!.data;
    expect(d).toMatchObject({ isGenerating: false, motionFirstFrame: "/ff.png", outputVideos: ["/v.mp4", "/old.mp4"] });
    expect(d.jobId).toBeUndefined();
  });

  it("这一轮页面自己在等的任务不重复接", () => {
    state().addNode("videoGen", undefined, { isGenerating: true, jobId: "job_1" });
    liveJobs.add("job_1");
    resumeCanvasJobs(state().activeProjectId);
    expect(waitForCanvasJob).not.toHaveBeenCalled();
  });

  it("应用重启导致任务失败时，告诉人去 ComfyUI 输出目录找", async () => {
    const id = state().addNode("videoGen", undefined, { isGenerating: true, jobId: "job_1" });
    vi.mocked(waitForCanvasJob).mockResolvedValue(job({ status: "failed", error: "应用退出时这个任务还没跑完，结果丢了。重新生成一次。" }));
    resumeCanvasJobs(state().activeProjectId);
    await vi.waitFor(() => expect(state().nodes.find((n) => n.id === id)?.data.error).toContain("output/Anima"));
  });
});
