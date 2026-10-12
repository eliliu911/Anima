/**
 * 超长镜：单镜超过 15 秒，出片时后台等分成几段，一次交给 H3 的窗口链出完、自动接成一条。
 * **和 Rust 的 `long_shot::plan` 是同一份算法** —— 两边各算各的会出现「界面说 2 段，后台拆成 3 段」。
 *
 * 用户 2026-10-05（EP03 第 20 镜台词放不下，又加长不了）：「单镜时长超过 15 秒后，自动做倍数拆分提交给 H3」
 * 「前台不需要展示出来」「单镜不能超过 45 秒」。所以界面上还是一镜，只在时长旁边说一句。
 */
import { CONTEXT_FRAMES } from "./longTake";

/** H3 一次生成的上限；超过它就是超长镜 */
export const MAX_SHOT_SECS = 15;
/** 超长镜的上限（用户定的） */
export const MAX_LONG_SHOT_SECS = 45;

export const isLongShot = (secs: number) => secs > MAX_SHOT_SECS + 0.01;

/** 拆几段、每段多少秒：等长窗口，`n × 每段 − (n − 1) × 重叠 = 总时长` */
export function longShotPlan(secs: number): { windows: number; windowSecs: number } {
  if (!isLongShot(secs)) return { windows: 1, windowSecs: secs };
  const ov = CONTEXT_FRAMES / 24;
  const n = Math.max(2, Math.ceil((secs - ov) / (MAX_SHOT_SECS - ov)));
  const w = (secs + (n - 1) * ov) / n;
  return { windows: n, windowSecs: Math.round(w * 100) / 100 };
}

/** 时长下面那句提示。不是超长镜返回 null */
export function longShotHint(secs: number): string | null {
  if (!isLongShot(secs)) return null;
  const { windows, windowSecs } = longShotPlan(secs);
  return `超长镜：出片时自动分成 ${windows} 段（每段约 ${windowSecs.toFixed(1)} 秒）一次交给 H3 出完，自动接成一条，用时约是普通一镜的 ${windows} 倍。` +
    `除最后一段，每段最后 2 秒不说话（那里是段与段的接缝）；不用首帧，也不能接上一镜。上限 ${MAX_LONG_SHOT_SECS} 秒`;
}
