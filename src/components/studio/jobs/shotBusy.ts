/**
 * 「这一镜正在跑什么」——**从任务登记表推，不是从组件本地 state 推**。
 *
 * # 修的是什么
 *
 * 原来是 `const [busyShot, setBusyShot] = useState(...)`。切到别的页再回来，
 * 组件重新挂载，那个 state 就没了 —— 按钮回到「出视频」，看着像没在跑。
 * 而任务其实一直在 Rust 侧跑，产物也会落盘。
 *
 * 更疼的是防重复提交的那个 ref 也一起丢了：回来再点一次会**再排一个
 * 同样的任务**，而视频是串行跑的，那就是白等十几分钟，还覆盖掉第一次的结果。
 *
 * 同一份真相服务端本来就有（`cinema_jobs` 读的是 Rust 写的 `jobs.json`），
 * 只是界面没用它。
 */
import { holdLabel, shotCounts } from "./renderQueue";
import type { Job, JobKind } from "../../../api/jobs";

/**
 * 这一镜正在跑的那一类。没有就是 `null`。
 *
 * 同一镜同时有首帧和出片在跑时，**出片优先显示** —— 它慢得多，
 * 是用户真正在等的那个。
 */
export function runningKind(jobs: readonly Job[], shotId: string): JobKind | null {
  const mine = jobs.filter((j) => j.shot_id === shotId && j.status === "running");
  if (mine.length === 0) return null;
  return mine.some((j) => j.kind === "video") ? "video" : "frame";
}

/** 这一镜有没有在跑的任务 —— 用来挡住第二次提交。 */
export function isShotBusy(jobs: readonly Job[], shotId: string): boolean {
  return runningKind(jobs, shotId) !== null;
}

/**
 * 上一轮还在跑、这一轮不跑了的镜头。
 *
 * 用来在任务落地时自动刷新 —— 不刷的话产物已经在磁盘上了，
 * 而界面还显示「还没出片」，得手动切页才看得到。
 *
 * **比的是「跑完了」，不是「成功了」。** 失败也要刷 ——
 * 失败原文在任务里，界面要能显示出来。
 */
export function settledShots(prev: readonly Job[], next: readonly Job[]): string[] {
  const wasRunning = new Set(
    prev.filter((j) => j.status === "running").map((j) => j.id),
  );
  if (wasRunning.size === 0) return [];
  const out = new Set<string>();
  for (const j of next) {
    if (wasRunning.has(j.id) && j.status !== "running") out.add(j.shot_id);
  }
  return [...out];
}

/**
 * 上一轮的任务快照，**存在模块级，活过组件卸载**。
 *
 * # 为什么不能放 ref
 *
 * [`settledShots`] 的判据是「上一轮在跑、这一轮不跑了」。快照放在组件的
 * `useRef` 里的话，**切走一次就清零** —— 回来时 `prev` 是空的，
 * `wasRunning.size === 0` 直接返回空，于是**在别处跑完的任务永远检测不到**，
 * 界面继续显示那份旧数据。
 *
 * 2026-09-18 实测撞上：分镜页点「生成镜头」，切到别的步骤，
 * 任务在后台跑完并写进了 `episode.json`（盘上确实有 6 个镜头），
 * 回来一看「还没有镜头」。**任务队列里明明写着已完成。**
 *
 * 而界面偏偏就是这么引导的 —— 生成中那行字写着「可以先去别的步骤」。
 * 照着做就会踩到。
 *
 * # 为什么按 项目/分集 分键
 *
 * 一个键一份快照。不分键的话切换分集会把上一集的快照当成本集的上一轮，
 * 于是「上一集刚跑完的任务」被算成本集的落地，触发一次无谓的重读；
 * 反过来也会漏。
 *
 * # 为什么不持久化
 *
 * 这是**会话内**的东西：应用重启后本来就该重读一切，没有「上一轮」可言。
 * 写进磁盘只会多一份要维护的状态。
 */
const seen = new Map<string, Job[]>();

/**
 * 快照的键：**消费方 + 项目 + 分集**。
 *
 * # 为什么要带消费方
 *
 * 三个步骤都在调 `useJobs`，而同一时刻只挂着一个。共用一份快照的话，
 * **你切到哪一页，哪一页就把这次落地消费掉了** —— `AssetsStep` 连回调都
 * 没传，它只是静悄悄地把快照推进到 done。等你回分镜页，快照已经是 done，
 * 照样不触发，bug 原样还在。
 *
 * 各存各的之后：没挂着的那个消费方快照不动，重挂时它看到的还是
 * 「上一轮在跑」→ 这一轮 done，落地检测得到。
 *
 * # 为什么空值也要占位
 *
 * 项目/分集为空时也要有各自的键，否则它们会共用同一份。
 */
export function jobsKey(consumer: string, projectId: string, episodeId: string): string {
    return `${consumer}\u0000${projectId}\u0000${episodeId}`;
}

/** 取上一轮的快照。没有就是空数组（第一次进来，本来就没有「上一轮」）。 */
export function lastJobs(key: string): Job[] {
    return seen.get(key) ?? [];
}

/** 记下这一轮。**每轮轮询都要记**，不只是有变化的时候。 */
export function rememberJobs(key: string, jobs: readonly Job[]): void {
    seen.set(key, [...jobs]);
}

/** 只给测试用：清掉快照，免得用例之间互相影响。 */
export function resetJobsMemory(): void {
    seen.clear();
}

/** 按集登记的任务（生成镜头表、合成提示词）在界面上要的全部状态。 */
export interface EpisodeJobState {
  /** 这一集最近一次这类任务，没有就是 `null` */
  job: Job | null;
  generating: boolean;
  /** 只有在跑的时候才有 */
  progress: Job["progress"] | null;
  /** **没成的那些**，只有成功的那次才显示 —— 失败了看 `job.error` */
  notes: string[];
  /** **成了但有话要说的那些。** 和 `notes` 分开，界面上的措辞完全不同 */
  remarks: string[];
}

/**
 * 从任务登记表推生成镜头表的状态。**任务列表是新的在前**（`cinema_jobs` 排好的），
 * 所以第一条 `shot_table` 就是最近一次。
 *
 * 分镜页切走再回来会重挂，这里不依赖任何组件本地 state，所以不丢。
 */
export function episodeJobState(jobs: readonly Job[], kind: "shot_table" | "prompts" | "video_batch" | "frame_batch" | "stitch" | "asset_images" | "timeline" | "fcp_export"): EpisodeJobState {
  const job = jobs.find((j) => j.kind === kind) ?? null;
  const generating = job?.status === "running";
  return {
    job,
    generating,
    progress: generating ? job?.progress ?? null : null,
    // **上游重跑过就不再显示** —— 那些提示说的是旧镜号，
    // 留着只会把人支到已经不存在的镜头上。见 `Job.superseded`。
    notes: fresh(job) ? job!.notes ?? [] : [],
    remarks: fresh(job) ? job!.remarks ?? [] : [],
  };
}

/** 跑完了、而且这一轮的账还算数（上游没有重跑过）。 */
const fresh = (job: Job | null) => job?.status === "done" && !job.superseded;

/** 生成镜头表的状态。 */
export const shotTableState = (jobs: readonly Job[]) => episodeJobState(jobs, "shot_table");

/** 合成提示词的状态。 */
export const promptsState = (jobs: readonly Job[]) => episodeJobState(jobs, "prompts");

/** 批量出片的状态。 */
export const batchState = (jobs: readonly Job[]) => episodeJobState(jobs, "video_batch");

/** 批量出首帧的状态。 */
export const frameBatchState = (jobs: readonly Job[]) => episodeJobState(jobs, "frame_batch");

/** 细化时间线的状态。 */
export const timelineState = (jobs: readonly Job[]) => episodeJobState(jobs, "timeline");

/** 队列里一条任务的标题。生成镜头表是按集的，没有镜号，按场报进度。 */
export function jobTitle(job: Job): string {
  if (job.kind === "video_batch") {
    const p = job.progress;
    if (job.status !== "running" || !p || p.total <= 0) return "批量出片";
    // 暂停位不是镜头：不算进「第几个 / 共几个」（2026-10-10 用户看不懂「第 1/10 个」—— 那个 1 是暂停位）
    const c = shotCounts(job);
    if (holdLabel(job) !== null) return `批量出片 · 已暂停 · 还有 ${c.left} 镜`;
    return c.position > 0 ? `批量出片 · 第 ${c.position}/${c.total} 个` : `批量出片 · 共 ${c.total} 个`;
  }
  if (job.kind === "frame_batch") {
    const p = job.progress;
    return job.status === "running" && p && p.total > 0
      ? `出首帧 · 第 ${Math.min(p.done + 1, p.total)}/${p.total} 个`
      : "批量出首帧";
  }
  if (job.kind === "prompts") {
    const p = job.progress;
    return job.status === "running" && p && p.total > 0
      ? `合成提示词 · 第 ${Math.min(p.done + 1, p.total)}/${p.total} 个`
      : "合成提示词";
  }
  if (job.kind === "timeline") {
    const p = job.progress;
    return job.status === "running" && p && p.total > 0
      ? `细化时间线 · 第 ${Math.min(p.done + 1, p.total)}/${p.total} 个`
      : job.prompt || "细化时间线";
  }
  if (job.kind === "stitch") return job.prompt || "合成一集";
  if (job.kind === "fcp_export") return job.prompt || "导出到 Final Cut Pro";
  if (job.kind === "asset_images") {
    const p = job.progress;
    return job.status === "running" && p && p.total > 0 ? `资产出图 · 第 ${Math.min(p.done + 1, p.total)}/${p.total} 张` : "资产出图";
  }
  if (job.kind === "shot_table") {
    const p = job.progress;
    return job.status === "running" && p && p.total > 0
      ? `生成镜头 · 第 ${Math.min(p.done + 1, p.total)}/${p.total} 场`
      : "生成镜头";
  }
  // 长镜头按场登记，没有镜号（和 `jobToast` 同一个判法）
  const scene = /^scene_(\d+)$/.exec(job.shot_id);
  if (scene) return `第 ${scene[1]} 场 · 长镜头`;
  return `第 ${job.shot_num || "?"} 镜 · ${job.kind === "video" ? "出片" : "首帧"}`;
}
