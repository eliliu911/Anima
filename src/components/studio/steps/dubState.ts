/**
 * 「配音」那一块的纯逻辑。
 *
 * 配音和 H3 原声是**二选一**：拼接时有 `audio/<镜头id>.wav` 就用它。
 * 所以这一块每句话都要让人清楚「按下去会失去什么」——
 * 失去的是口型（H3 的嘴型是跟着它自己生成的那段语音做的）。
 */
import type { DubOutcome } from "../../../api/cinema";

/** 按钮上写什么。**没配过和配过一部分说法不同** —— 都写「配音」的话，
 *  用户不知道再点一次是补齐还是重来（实际是全部重来）。 */
export function dubLabel(busy: boolean, dubbed: number, total: number): string {
  if (busy) return "配音中…";
  if (total === 0) return "没有带台词的镜头";
  if (dubbed === 0) return `给 ${total} 镜配音`;
  if (dubbed >= total) return `重新配音（${total} 镜）`;
  return `补齐配音（还差 ${total - dubbed} 镜）`;
}

/** 能不能点。没有带台词的镜头时点了必定报错，不如灰掉。 */
export function canDub(busy: boolean, total: number): boolean {
  return !busy && total > 0;
}

/**
 * 配完之后的交代。
 *
 * **超长必须单列出来**：拼接带 `-shortest`，配音比画面长的那一截会被
 * 直接砍掉，而且不报错 —— 表现是「这句话说到一半没了」。
 */
export function dubSummary(outcomes: DubOutcome[]): string {
  if (outcomes.length === 0) return "";
  const lines = outcomes.reduce((n, o) => n + o.lines, 0);
  const replaced = outcomes.filter((o) => o.replaced_source_audio).length;
  const parts = [`${outcomes.length} 镜 / ${lines} 句已配音`];
  if (replaced > 0) parts.push(`其中 ${replaced} 镜换掉了原声（这几镜口型会对不上）`);
  return parts.join("，");
}

/** 哪几镜配音比画面长，超出多少。空 = 都放得下。 */
export function overflows(outcomes: DubOutcome[]): { shot_id: string; secs: number }[] {
  return outcomes
    .filter((o) => o.overflow_secs !== null && o.overflow_secs > 0)
    .map((o) => ({ shot_id: o.shot_id, secs: o.overflow_secs as number }));
}

/**
 * 用了哪些音色，去重。
 *
 * **全是同一把嗓子要能一眼看出来** —— 那基本等于角色音色没配上
 * （回落到了全局设置），而它不报错。
 */
export function voicesUsed(outcomes: DubOutcome[]): string[] {
  const seen = new Set<string>();
  for (const o of outcomes) for (const [, v] of o.voices) if (v.trim()) seen.add(v.trim());
  return [...seen];
}

/**
 * 配完之后**必须重新合成一集**才会进片子。
 *
 * # 为什么这句话必须有
 *
 * 配音只写 `audio/<镜头id>.wav`，**一点都不碰视频**。换音轨发生在合成那一步
 * （`pick_shot_audio` 看这个文件在不在）。所以已经合成过的 `final.mp4`
 * 在配完音之后**一点变化都没有** —— 不报错，就是没变。
 *
 * 用户 2026-09-18 问到了这里：「可以在组装页的话，分镜视频都已经处理完了，
 * 声音怎么办？」他问的正是这条，而那时候界面上一个字都没提。
 *
 * 回 `null` = 不用说（还没配音，或者还没合成过）。
 */
export function restitchNote(dubbedShots: number, hasFinal: boolean): string | null {
  if (dubbedShots === 0) return null;
  return hasFinal
    ? "配音已经写好了，但**还没进片子** —— 再点一次「合成一集」才会换上。"
    : "配音已经写好了，合成这一集时会自动用上。";
}
