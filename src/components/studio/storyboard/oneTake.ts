/**
 * 一场一镜出（不分镜，提示词里按时间轴写运镜）。**和 Rust 的 `one_take.rs` 是同一份判断**，
 * 两边都有测试钉着同一组数。
 *
 * 用户 2026-09-23：「一共就 15 秒，被分成 2 秒的，根本到 Comfy 里就被 PASS 了，
 * 所以我需要一个直接通过在提示词里分时间轴运镜，这样就省得短视频再分镜了。」
 *
 * 一场有三种出法：
 * - **分镜**：照常拆镜头，一镜一次生成
 * - **一镜出**：镜数和时长由代码定（≤ 15 秒一镜，更长的等分），镜内按时间轴切镜；两镜以上自动整段出
 * - **长镜头**：原来那个 —— 镜头照常拆，但要等长，整场串成一条窗口链
 */
import type { EpisodeData, SceneTake, StoryboardShot } from "../../../api/cinema";

export type SceneMode = "split" | "one_take" | "long_take";

/** 一个窗口最长多少秒（H3 单次上限）。和 Rust `WINDOW_MAX_SECS` 同一个数 */
export const WINDOW_MAX_SECS = 15;
export const MAX_TOTAL_SECS = 60;
export const MIN_TOTAL_SECS = 4;

type Ep = Pick<EpisodeData, "one_take" | "scene_takes" | "long_take_scenes">;

/** 这一场现在设的是哪种出法。单场设置优先，没设的跟分集默认走 */
export function sceneMode(ep: Ep, sceneNo: number): SceneMode {
  const t = (ep.scene_takes ?? []).find((x) => x.scene_no === sceneNo);
  if (t ? t.one_take : !!ep.one_take) return "one_take";
  return (ep.long_take_scenes ?? []).includes(sceneNo) ? "long_take" : "split";
}

/** 这一场手填的总时长。空 = 自动 */
export function manualSecs(ep: Ep, sceneNo: number): number | null {
  return (ep.scene_takes ?? []).find((x) => x.scene_no === sceneNo && x.one_take)?.secs ?? null;
}

function roundHalfUp(x: number): number {
  return Math.max(0, Math.ceil(x * 2 - 1e-4)) / 2;
}

/** 总时长等分成几镜、每镜几秒。和 Rust `one_take::windows` 同一个算法 */
export function windows(total: number): { n: number; each: number } {
  const t = Math.min(Math.max(total, MIN_TOTAL_SECS), MAX_TOTAL_SECS);
  const n = Math.max(1, Math.ceil(t / WINDOW_MAX_SECS));
  return { n, each: Math.min(roundHalfUp(t / n), WINDOW_MAX_SECS) };
}

/**
 * 换一场的出法之后，集上那两个字段该是什么。**只算不存** —— 存盘和要不要重新生成由调用方决定。
 *
 * - 一镜出：记一条 `one_take: true`（带手填时长）；长镜头标记先去掉 —— 生成完两镜以上后端会自动加回
 * - 分镜：整集默认一镜出时要记一条 `one_take: false` 才盖得住，否则删掉这一场的记录
 * - 长镜头：同「分镜」，再加进 `long_take_scenes`
 */
export function withSceneMode(ep: Ep, sceneNo: number, mode: SceneMode, secs: number | null = null): { scene_takes: SceneTake[]; long_take_scenes: number[] } {
  const others = (ep.scene_takes ?? []).filter((t) => t.scene_no !== sceneNo);
  const lt = (ep.long_take_scenes ?? []).filter((n) => n !== sceneNo);
  const sorted = (xs: SceneTake[]) => [...xs].sort((a, b) => a.scene_no - b.scene_no);
  if (mode === "one_take") {
    const t: SceneTake = { scene_no: sceneNo, one_take: true, ...(secs ? { secs } : {}) };
    return { scene_takes: sorted([...others, t]), long_take_scenes: lt };
  }
  const takes = ep.one_take ? sorted([...others, { scene_no: sceneNo, one_take: false }]) : others;
  return { scene_takes: takes, long_take_scenes: mode === "long_take" ? [...lt, sceneNo].sort((a, b) => a - b) : lt };
}

/**
 * 这一场现有的镜头和设的出法对不对得上。对不上就要重新生成这一场。
 *
 * 设成一镜出而镜头不是一镜出生成的（或者手填的时长和现在的不一样），
 * 或者改回分镜 / 长镜头而镜头还是一镜出的。
 */
export function needsRegen(ep: Ep, sceneNo: number, shots: StoryboardShot[]): string | null {
  const mine = shots.filter((s) => (s.scene_no ?? 0) === sceneNo);
  if (!mine.length) return null;
  const mode = sceneMode(ep, sceneNo);
  const allOne = mine.every((s) => s.one_take);
  if (mode === "one_take") {
    if (!allOne) return `这一场现在是 ${mine.length} 镜分镜，改成一镜出要重新生成这一场`;
    const want = manualSecs(ep, sceneNo);
    const total = mine.reduce((a, s) => a + s.duration, 0);
    if (want) {
      const w = windows(want);
      if (w.n !== mine.length || Math.abs(w.each * w.n - total) > 0.01) return `时长改成了 ${want} 秒（现在是 ${total} 秒），要重新生成这一场`;
    }
    return null;
  }
  return mine.some((s) => s.one_take) ? "这一场现在是一镜出生成的，改回分镜要重新生成这一场" : null;
}
