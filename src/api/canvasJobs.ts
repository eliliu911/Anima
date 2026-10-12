import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { JOB_EVENT, type Job } from "./jobs";

/**
 * 画布生成的后台任务（`src-tauri/src/canvas_jobs.rs`）。
 *
 * 画布以前是前端自己 await 结果，页面一重新加载就断。现在登记成后台任务：
 * 点完立刻回任务 id，后台跑完结果存进任务，页面重新加载后按 id 取回。
 */
export type CanvasTask =
  | { kind: "generate"; args: Record<string, unknown> }
  | { kind: "depth_capture"; args: { video_path: string } }
  | { kind: "depth_video"; args: Record<string, unknown> }
  | { kind: "image_op"; args: { image_path: string; op: string } }
  | { kind: "interpolate"; args: { video_path: string } }
  | { kind: "video_upscale"; args: { video_path: string; refine: boolean; prompt: string } }
  | { kind: "world_generate"; args: { image_path: string; device?: "mps" | "cpu" | "cuda" } }
  | { kind: "pano_cube"; args: { image_path: string; face_size?: number } }
  /** 长镜头：视频节点连视频节点 = 窗口链，一次提交出一个 MP4 */
  | {
      kind: "long_take";
      args: {
        windows: { prompt: string; images: string[] }[];
        window_secs: number;
        context_frames?: number;
        width: number;
        height: number;
        seed?: number;
        generate_audio?: boolean;
      };
    };

export function canvasJobStart(args: { projectId: string | null; nodeId: string; label: string; task: CanvasTask }): Promise<string> {
  return invoke<string>("canvas_job_start", {
    args: { project_id: args.projectId, node_id: args.nodeId, label: args.label, task: args.task },
  });
}

export function canvasJobGet(jobId: string): Promise<Job | null> {
  return invoke<Job | null>("canvas_job_get", { jobId });
}

/**
 * 取消一条任务 —— **真的去 ComfyUI 撤**，不是只把界面上的圈停掉。
 *
 * 回来的那句话要原样显示：没找到对应的 ComfyUI 任务时它会明说，
 * 而「界面说取消了、ComfyUI 还在跑」正是这个功能存在之前的那个病。
 */
export function canvasJobCancel(jobId: string): Promise<string> {
  return invoke<string>("canvas_job_cancel", { jobId });
}

/**
 * 等一条任务结束。**事件 + 轮询两条腿**：事件会在页面重新加载的空档里丢，
 * 光靠事件的话那一条永远等不到；光靠轮询又慢。每 3 秒兜底查一次。
 */
export function waitForCanvasJob(jobId: string): Promise<Job> {
  return new Promise((resolve, reject) => {
    let done = false;
    let unlisten: (() => void) | null = null;
    const finish = (job: Job) => {
      if (done) return;
      done = true;
      clearInterval(timer);
      unlisten?.();
      resolve(job);
    };
    void listen<Job>(JOB_EVENT, (e) => {
      if (e.payload.id === jobId && e.payload.status !== "running") finish(e.payload);
    }).then((f) => {
      if (done) f();
      else unlisten = f;
    });
    const check = () =>
      canvasJobGet(jobId)
        .then((job) => {
          if (!job) {
            if (!done) {
              done = true;
              clearInterval(timer);
              unlisten?.();
              reject(new Error("任务记录找不到了（可能被清掉了）—— 重新生成一次"));
            }
            return;
          }
          if (job.status !== "running") finish(job);
        })
        .catch(() => {
          // 查不到先不算失败，下一轮再查
        });
    const timer = setInterval(check, 3000);
    void check();
  });
}
