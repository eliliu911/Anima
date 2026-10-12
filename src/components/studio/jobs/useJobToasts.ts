import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { cinemaJobsRunning, JOB_EVENT, type Job } from "../../../api/jobs";
import { useToasts } from "../toastStore";
import { batchWaitingOn, currentBatchChild, isBatchChild } from "./jobBuckets";
import { CANVAS_GROUP_TOAST, canvasGroupToast, jobToastId, toastForJob } from "./jobToast";

/**
 * 把后台任务接到右下角通知条。**挂在 Studio 最外层、只挂一处**（`StudioShell`），
 * 挂在页面里的话切页面就退订，任务结束时没人接。
 *
 * 进来先问一次还在跑的任务 —— 界面刷新后通知条是空的，而任务还在后端跑。
 */
export function useJobToasts() {
  useEffect(() => {
    let alive = true;
    // 事件先到过的任务，恢复时跳过：否则「查在跑的」回来得比「完成」事件晚，
    // 会把已经完成的那条盖回转圈，而且再也没有事件来纠正它。
    const seen = new Set<string>();
    // 最近看到的每条任务的最新状态：判断一条出片是不是批量出片里的一镜要用
    const known = new Map<string, Job>();
    const show = (job: Job) => {
      if (!alive) return;
      known.set(job.id, job);
      const all = [...known.values()];
      // 批量出片里的一镜不单独弹：批量那一条已经在报「第 N 个 · 第 N 镜」，失败原因也记在批量结果里。
      // **但它的 ComfyUI 进度要交给批量那一条显示**（步数、还要多久都写在这一镜身上）
      if (isBatchChild(job, all)) {
        useToasts.getState().dismiss(jobToastId(job));
        const batch = all.find((b) => b.status === "running" && currentBatchChild(b, all)?.id === job.id);
        if (batch) useToasts.getState().upsert(jobToastId(batch), toastForJob(batch, { current: job }));
        return;
      }
      // 排队中的批量正在等的那一镜：不单独弹，并进批量那一条（见 `batchWaitingOn`）
      const waitingBatch = all.find((b) => batchWaitingOn(b, all)?.id === job.id);
      if (waitingBatch) {
        useToasts.getState().dismiss(jobToastId(job));
        useToasts.getState().upsert(jobToastId(waitingBatch), toastForJob(waitingBatch, { waiting: job }));
        return;
      }
      // **画布任务：在跑的合成一条**，跑完 / 失败才各自弹（见 `canvasGroupToast`）
      if (job.kind === "canvas") {
        const running = all.filter((j) => j.kind === "canvas" && j.status === "running").sort((a, b) => a.created_at - b.created_at);
        const group = canvasGroupToast(running);
        if (group) useToasts.getState().upsert(CANVAS_GROUP_TOAST, group);
        else useToasts.getState().dismiss(CANVAS_GROUP_TOAST);
        if (job.status === "running") useToasts.getState().dismiss(jobToastId(job));
        else useToasts.getState().upsert(jobToastId(job), toastForJob(job));
        return;
      }
      const ctx = { current: currentBatchChild(job, all), waiting: batchWaitingOn(job, all) };
      if (ctx.waiting) useToasts.getState().dismiss(jobToastId(ctx.waiting));
      useToasts.getState().upsert(jobToastId(job), toastForJob(job, ctx));
    };
    const off = listen<Job>(JOB_EVENT, (e) => {
      seen.add(e.payload.id);
      show(e.payload);
    });
    void cinemaJobsRunning()
      .then((jobs) => {
        const fresh = jobs.filter((j) => !seen.has(j.id));
        // 先把批量任务记下，再逐条显示 —— 否则子任务先到时还不知道它属于批量
        fresh.forEach((j) => known.set(j.id, j));
        fresh.forEach(show);
      })
      .catch(() => {
        // 读不到就算了：事件照样会来，只是刷新前那几条要等下一次状态变化才出现
      });
    return () => {
      alive = false;
      void off.then((f) => f());
    };
  }, []);
}
