import { invoke } from "@tauri-apps/api/core";

/**
 * 出图 / 出片的任务登记。
 *
 * **这些是真实状态**，不是界面上编的：Rust 侧在每次渲染命令进出时
 * 写盘，失败原文照抄。应用重启时上一轮没跑完的会被判为失败 ——
 * 否则队列上会永远挂着几个跑不完的任务。
 */
export type JobKind = "frame" | "video" | "shot_table" | "prompts" | "video_batch" | "frame_batch" | "canvas" | "stitch" | "asset_images" | "style_compare" | "timeline" | "fcp_export";

/** 任务状态变化时后端发的事件，载荷是整条 Job。见 Rust `jobs::EVENT` */
export const JOB_EVENT = "cinema://job-updated";

export interface JobProgress {
  done: number;
  total: number;
  label: string;
  /** 预计什么时候跑完（unix 秒）。出片任务拿得到 ComfyUI 步数时才有 */
  eta_at?: number;
}
export type JobStatus = "running" | "done" | "failed";

export interface Job {
  id: string;
  kind: JobKind;
  status: JobStatus;
  project_id: string;
  episode_id: string;
  shot_id: string;
  shot_num: number;
  prompt: string;
  created_at: number;
  finished_at?: number;
  output?: string;
  /** 失败原文。**不要在界面上概括它** —— 「生成失败」和「ComfyUI 没起」
   *  是完全不同的两件事，而后者用户自己就能修。 */
  error?: string;
  /** 跑到哪了（生成镜头表按场报） */
  progress?: JobProgress;
  /** 这一镜实际走的链路（「FL2VA 首帧 · LightX2V 4 次」）—— 后端从提交给 ComfyUI 的那张图里读的，不是前端按设置推的 */
  pipeline?: string;
  /** 成功时一句话结果 */
  summary?: string;
  /** 用户需要知道的备注。存在任务上，切走页面也不丢 */
  notes?: string[];
  /**
   * **成了，但有话要说。** 和 `notes` 严格分开 ——
   * `notes` 那一段在界面上的标题是「没通过检查（没有保存）」。
   */
  remarks?: string[];
  /**
   * **这一轮的提示已经过时了** —— 它跑完之后，上游重新跑了一轮。
   *
   * 重新生成镜头表之后，上一轮「合成有 4 镜没过」说的还是旧镜号。
   * 记录本身留着（队列里还要能查「上次为什么失败」），
   * 只是那些横幅不再显示。后端在登记新任务时打这个标记。
   */
  superseded?: boolean;
  /** 批量任务要处理的镜头 id（按顺序），界面据此标「排队中」 */
  targets?: string[];
  /** 用户点了停止：当前这一镜跑完就不再继续 */
  cancel_requested?: boolean;
  /** 画布任务的完整返回（路径、备注、首帧……）。见 Rust `canvas_jobs` */
  result?: unknown;
}

/** 所有项目里正在跑的任务。右下角提示框在界面刷新后靠它恢复 */
export function cinemaJobsRunning(): Promise<Job[]> {
  return invoke<Job[]>("cinema_jobs_running");
}

export function cinemaJobs(projectId: string, episodeId: string): Promise<Job[]> {
  return invoke<Job[]>("cinema_jobs", { projectId, episodeId });
}

/** 停止一条在跑的批量任务。**正在出的那一镜会跑完**（H3 不响应中断），之后不再继续。 */
export function cinemaCancelJob(jobId: string): Promise<Job> {
  return invoke<Job>("cinema_cancel_job", { jobId });
}

/** 清掉已结束的。正在跑的不动。 */
export function cinemaJobsClear(projectId: string, episodeId: string): Promise<Job[]> {
  return invoke<Job[]>("cinema_jobs_clear", { projectId, episodeId });
}

/** 批量出片排队名单里的一项。见 Rust `render_queue::QueueItem` */
export interface QueueItem {
  id: string;
  /** 镜号；暂停位、长镜头整场没有 */
  shot_num?: number | null;
  label: string;
  /** 暂停位：批量走到它就停下等，撤掉才接着出 */
  hold: boolean;
}

/** 批量出片的排队名单。`pending` 按「必须连着出」分好组：要接上一镜的和上一镜在同一组，只能整组挪 */
export interface QueueView {
  job_id: string;
  project_id: string;
  episode_id: string;
  done: number;
  current?: QueueItem | null;
  pending: QueueItem[][];
}

export function cinemaRenderQueue(jobId: string): Promise<QueueView> {
  return invoke<QueueView>("cinema_render_queue", { jobId });
}

/** 存新顺序：没轮到的那段，平铺成一列。拆开了要连着出的组，后端会拒绝并说明原因 */
export function cinemaReorderQueue(jobId: string, pending: string[]): Promise<QueueView> {
  return invoke<QueueView>("cinema_reorder_queue", { jobId, pending });
}

export function cinemaReleaseHold(jobId: string, hold: string): Promise<QueueView> {
  return invoke<QueueView>("cinema_release_hold", { jobId, hold });
}
