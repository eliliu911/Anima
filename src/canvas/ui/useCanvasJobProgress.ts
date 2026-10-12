import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { JOB_EVENT, type Job } from "../../api/jobs";
import { canvasJobGet } from "../../api/canvasJobs";
import { videoProgressLine, withPipeline } from "../../components/studio/jobs/jobToast";

/**
 * **这一个节点自己的任务**跑到哪了（2026-10-11 用户截图：两个视频节点都显示「第 3/4 步 · 约还要 15 分钟」，
 * 而 ComfyUI 一次只跑一个、另一个在排队）。
 *
 * 后端出片的等待循环按自己的 prompt 算：排队时「排队中 · 前面还有 N 个」，轮到时步数和预计完成时间，
 * 写进这条任务（`jobs::note_progress`，画布任务 v0.43.30 起在 `jobs::scoped` 里）。这里订阅任务事件拿它。
 * 回 `null` = 这条任务还没记进度（还没提交给 ComfyUI，或者这类任务不走那个等待循环）—— 调用方退回全局的 ComfyUI 日志。
 */
export function useCanvasJobProgress(jobId: string | undefined): string | null {
  const [line, setLine] = useState<string | null>(null);
  useEffect(() => {
    setLine(null);
    if (!jobId) return;
    let alive = true;
    const apply = (job: Job | null | undefined) => {
      if (!alive || !job || job.id !== jobId) return;
      const p = job.status === "running" ? videoProgressLine(job.progress) : null;
      setLine(p ? withPipeline(job, p) : null);
    };
    void canvasJobGet(jobId).then(apply).catch(() => {});
    const off = listen<Job>(JOB_EVENT, (e) => apply(e.payload));
    return () => {
      alive = false;
      void off.then((f) => f());
    };
  }, [jobId]);
  return line;
}
