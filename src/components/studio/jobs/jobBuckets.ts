import type { Job } from "../../../api/jobs";

export type JobTab = "running" | "done" | "failed";

/**
 * 按状态分三堆，各堆内**新的在前**。
 *
 * 单拎出来是因为「在跑几个」这个数字要在两个地方用（按钮上的角标、
 * 面板的页签），各算一遍必然漂 —— 而漂了之后表现是「角标写着 2、
 * 点开只有 1 个」，看起来像队列坏了。
 */
/**
 * 这条出片是不是正在跑的批量出片里的一镜。
 *
 * 批量出片每一镜仍各登记一条出片任务（结果和失败原文要留在队列里），但**在跑的时候**
 * 批量那一条已经说了「第 9/14 个 · 第 9 镜」，再单列一条就是重复（用户截图：右下角两条、队列两条）。
 */
export function isBatchChild(job: Job, all: readonly Job[]): boolean {
  const parent = job.kind === "video" ? "video_batch" : job.kind === "frame" ? "frame_batch" : null;
  return (
    parent !== null &&
    all.some(
      (b) =>
        b.kind === parent &&
        b.status === "running" &&
        b.project_id === job.project_id &&
        b.episode_id === job.episode_id &&
        (b.targets ?? []).includes(job.shot_id),
    )
  );
}

/**
 * 批量里**正在出的那一镜**（它自己的出片任务）。ComfyUI 的步数、还要多久写在它身上，不在批量那一条上 ——
 * 批量的通知条要从这里取（2026-10-06 用户：「批量出片怎么不显示时间和进度了」）。
 */
export function currentBatchChild(batch: Job, all: readonly Job[]): Job | undefined {
  const kind = batch.kind === "video_batch" ? "video" : batch.kind === "frame_batch" ? "frame" : null;
  if (!kind || batch.status !== "running") return undefined;
  return all
    .filter(
      (j) =>
        j.kind === kind &&
        j.status === "running" &&
        j.project_id === batch.project_id &&
        j.episode_id === batch.episode_id &&
        (batch.targets ?? []).includes(j.shot_id),
    )
    .sort((a, b) => b.created_at - a.created_at)[0];
}

/**
 * 这条批量**正在等的那一镜**：批量还在「排队中 · 等前一镜出完」，同一集里有一条不在它名单里的出片在跑
 * （单镜出片后点的「加入队列」、或者重启后接着收的那一镜，见 `render_resume`）。
 *
 * 2026-10-06 用户截图：重启后右下角一条「第 3 镜 · 出片中」、一条「第 1/1 个 · 排队中」，
 * 以为原来 1/3 的队列丢了 —— 其实第 2 镜已出完、第 3 镜在续接、第 5 镜在排。两条要合成一条说。
 */
export function batchWaitingOn(batch: Job, all: readonly Job[]): Job | undefined {
  if (batch.kind !== "video_batch" || batch.status !== "running") return undefined;
  if (!batch.progress?.label?.startsWith("排队中 · 等前一镜")) return undefined;
  return all
    .filter(
      (j) =>
        j.kind === "video" &&
        j.status === "running" &&
        j.project_id === batch.project_id &&
        j.episode_id === batch.episode_id &&
        !(batch.targets ?? []).includes(j.shot_id),
    )
    .sort((a, b) => b.created_at - a.created_at)[0];
}

export function bucketJobs(jobs: Job[]): Record<JobTab, Job[]> {
  const out: Record<JobTab, Job[]> = { running: [], done: [], failed: [] };
  // 在跑的批量子任务不单列；跑完的留着（产物缩略图、失败原文要看得到）
  for (const j of jobs) if (!(j.status === "running" && isBatchChild(j, jobs))) out[j.status].push(j);
  for (const k of Object.keys(out) as JobTab[]) {
    out[k].sort((a, b) => b.created_at - a.created_at);
  }
  return out;
}

/**
 * 打开面板时默认落在哪一页。
 *
 * **有在跑的看在跑的，没在跑但有失败的看失败的。** 固定落在「运行中」
 * 的话，跑完一批去点队列会看到一个空列表 —— 而用户点开它，
 * 十有八九正是因为有东西没出来。
 */
export function defaultTab(b: Record<JobTab, Job[]>): JobTab {
  if (b.running.length > 0) return "running";
  if (b.failed.length > 0) return "failed";
  return "done";
}

/** 跑了多久 / 跑了多久才结束。秒、分、时三档。 */
export function elapsedLabel(job: Job, nowSec: number): string {
  const end = job.finished_at ?? nowSec;
  const s = Math.max(0, end - job.created_at);
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m${s % 60 ? ` ${s % 60}s` : ""}`;
  return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`;
}

/**
 * 轮询间隔。**有任务在跑时才勤快。**
 *
 * 一直用 1 秒的话，一个开着不动的窗口每分钟白读 60 次文件；
 * 一直用 6 秒的话，点了生成之后要等 6 秒角标才动，看起来像没点上。
 */
export function pollMs(runningCount: number, panelOpen: boolean): number {
  if (runningCount > 0) return 1000;
  return panelOpen ? 3000 : 6000;
}
