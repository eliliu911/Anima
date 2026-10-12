/**
 * 时间线写得细不细（和后端 `timeline_detail.rs` 同一套数）。
 *
 * 用户 2026-09-27：「同样 12 秒的视频……把时间线写好，描述细致，而不是一笔带过。」
 * 量法：画面 + 站位 + 运镜每秒多少字，**不算约束**（约束每段原样重复，不是细节）。
 * Kotte 母版中位 42 字/秒；《回收师》EP01 画面这一栏中位 15。
 */
import type { ShotSegment, StoryboardShot } from "../../../api/cinema";
import type { Job } from "../../../api/jobs";

/** 目标（字/秒） */
export const TARGET_CPS = 40;
/** 低于这个算一笔带过 */
export const THIN_CPS = 20;
/** 至少这么长的段才判「一笔带过」—— 1 秒的过渡段写一句话是正常的 */
const THIN_MIN_SECS = 2;

const count = (t: string | undefined) => (t ?? "").replace(/\s/g, "").length;

export function segmentChars(seg: ShotSegment): number {
  return count(seg.visual) + count(seg.blocking) + count(seg.camera);
}

/** 整镜每秒多少字 */
export function shotDensity(shot: Pick<StoryboardShot, "segments" | "duration">): number {
  if (!shot.duration || shot.duration <= 0) return 0;
  return (shot.segments ?? []).reduce((n, s) => n + segmentChars(s), 0) / shot.duration;
}

export interface ThinSegment {
  /** 第几段（从 1 数） */
  n: number;
  start: number;
  end: number;
  chars: number;
}

/** 一笔带过的那几段：够长、却写得稀 */
export function thinSegments(shot: Pick<StoryboardShot, "segments">): ThinSegment[] {
  return (shot.segments ?? [])
    .map((s, i) => ({ n: i + 1, start: s.start, end: s.end, chars: segmentChars(s) }))
    .filter((s) => s.end - s.start >= THIN_MIN_SECS && s.chars / (s.end - s.start) < THIN_CPS);
}

/** 这一镜要不要细化：整镜偏薄，或者有一段一笔带过 */
export function needsRefine(shot: Pick<StoryboardShot, "segments" | "duration">): boolean {
  return (shot.segments?.length ?? 0) > 0 && (shotDensity(shot) < TARGET_CPS * 0.9 || thinSegments(shot).length > 0);
}

/**
 * 批量细化挑哪些：选中的场里偏薄的；出过片的默认不动（细化了就得重新合成、重出）。
 * 回要做的镜头 id，以及因为「已经够细」「已经出过片」没挑进来的各几镜。
 */
export function refinePlan(shots: StoryboardShot[], sceneNos: number[], includeRendered: boolean): { ids: string[]; dense: number; rendered: number } {
  const inScenes = [...shots].filter((s) => sceneNos.includes(s.scene_no ?? 0)).sort((a, b) => a.shot_num - b.shot_num);
  const thin = inScenes.filter(needsRefine);
  const ids = thin.filter((s) => includeRendered || !hasVideo(s)).map((s) => s.id);
  return { ids, dense: inScenes.length - thin.length, rendered: thin.length - ids.length };
}

export function hasVideo(s: StoryboardShot): boolean {
  return !!s.video_url || (s.takes?.length ?? 0) > 0;
}

/**
 * 这一镜在细化任务里的状态：正在细化 / 排队中 / 不在这一轮（`null`）。
 * 进度标签是「第 N 镜」，`targets` 是这一轮要做的镜头 id（后端 `cinema_refine_timelines`）。
 */
export function refineRound(shot: Pick<StoryboardShot, "id" | "shot_num">, job: Job | null): "running" | "queued" | null {
  if (!job || job.kind !== "timeline" || job.status !== "running") return null;
  const targets = job.targets ?? [];
  const i = targets.indexOf(shot.id);
  if (i < 0) return null;
  const current = /第\s*(\d+)\s*镜/.exec(job.progress?.label ?? "")?.[1];
  if (current !== undefined && Number(current) === shot.shot_num && (job.progress?.done ?? 0) <= i) return "running";
  return i >= (job.progress?.done ?? 0) ? "queued" : null;
}
