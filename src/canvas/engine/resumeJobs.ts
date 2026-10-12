import { waitForCanvasJob } from "../../api/canvasJobs";
import { useCanvasStore } from "../store/useCanvasStore";
import { liveJobs } from "./canvasTasks";
import { depthSourceVideo } from "../utils/depthChain";
import type { CanvasNode } from "../types";

/**
 * 页面重新加载后，把还挂着后台任务的节点接回来。
 *
 * 节点上记着 `jobId`（`canvasTasks.ts` 发起任务时写的）。这一轮页面里执行器没在等它 ——
 * 说明等它的那个页面已经没了 —— 就在这里按 id 等结果，按节点类型写回。
 *
 * 写回比执行器那边**简单**：批量出图只接回正在跑的那一张，后面没排上的就没了（节点上会说）。
 */
const resuming = new Set<string>();

type Result = {
  path?: string | null;
  paths?: string[];
  notes?: string[];
  first_frame?: string;
  /** 长镜头（`canvas_ops::CanvasLongTakeOut`）回的是这个，不是 path */
  video_path?: string;
};

export function applyJobResult(node: CanvasNode, result: Result): Record<string, unknown> {
  const paths = result.paths?.length ? result.paths : result.path ? [result.path] : result.video_path ? [result.video_path] : [];
  const notes = [...(result.notes ?? []), "页面中途重新加载过，结果是从后台任务取回的"];
  const done = { isGenerating: false, error: undefined, jobId: undefined, jobKind: undefined, notes };
  if (!paths.length) return { ...done, error: "后台任务跑完了，但没有产物" };
  if (node.type === "depthCapture") {
    return { ...done, depthVideoUrl: paths[0] };
  }
  if (node.type === "upscale") {
    return { ...done, outputUrl: paths[0], sourcePath: node.data.requestedSourcePath };
  }
  if (node.type === "videoGen") {
    const old = (node.data.outputVideos as string[] | undefined) ?? [];
    return {
      ...done,
      outputVideoUrl: paths[0],
      outputVideos: [...paths, ...old.filter((p) => !paths.includes(p))],
      ...(result.first_frame ? { motionFirstFrame: result.first_frame } : {}),
    };
  }
  const old = (node.data.outputImages as string[] | undefined) ?? [];
  return { ...done, outputImageUrl: paths[0], outputImages: [...paths, ...old.filter((p) => !paths.includes(p))] };
}

export function resumeCanvasJobs(projectId: string | null) {
  const { nodes } = useCanvasStore.getState().getProjectCanvas(projectId);
  for (const node of nodes) {
    const jobId = node.data.jobId as string | undefined;
    if (!jobId || liveJobs.has(jobId) || resuming.has(jobId)) continue;
    resuming.add(jobId);
    const update = (patch: Record<string, unknown>) =>
      useCanvasStore.getState().updateProjectNodeData(projectId, node.id, patch);
    update({ isGenerating: true, error: undefined });
    void waitForCanvasJob(jobId)
      .then((job) => {
        const latest = useCanvasStore.getState().getProjectCanvas(projectId).nodes.find((n) => n.id === node.id);
        // 节点被删了、或者已经又发起了新任务，就不写回
        if (!latest || latest.data.jobId !== jobId) return;
        if (job.status === "failed") {
          update({ isGenerating: false, jobId: undefined, jobKind: undefined,
            error: `${job.error || "没有返回原因"}${/应用退出/.test(job.error ?? "") ? "（ComfyUI 那边可能已经跑完，片子在 ComfyUI 的 output/Anima 目录里）" : ""}` });
          return;
        }
        const patch = applyJobResult(latest, (job.result ?? {}) as Result);
        if (latest.type === "depthCapture") {
          const { nodes: ns, edges } = useCanvasStore.getState().getProjectCanvas(projectId);
          patch.sourceVideoPath = depthSourceVideo(latest.id, ns, edges).path;
        }
        update(patch);
      })
      .catch((e) => update({ isGenerating: false, jobId: undefined, jobKind: undefined, error: String(e) }))
      .finally(() => resuming.delete(jobId));
  }
}
