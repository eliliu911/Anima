import type { StudioResult } from "./useStudioStore";

/**
 * 结果区的过滤与分类。
 *
 * # 为什么按「出的文件」分，不只按模式
 *
 * 模式只说了**打算**出什么，文件才是**实际**出了什么。这两者会不一致：
 * 一个模式 id 写错、或者以后加了「图生图顺带出个预览片」这种，
 * 按模式分类就会把视频塞进「图片」页签里 —— 点开是个播放器，
 * 而且不报错。所以有文件的时候以文件为准，没文件（还在跑）才退回模式。
 */

/** 出视频的模式。**新增视频模式时必须同步这里**，否则它会被当成图片。 */
export const VIDEO_MODES = new Set(["t2v", "i2v", "r2v", "v2v"]);

export type ResultKind = "image" | "video";
export type FilterKey = "all" | ResultKind;

/** 这条结果算图还是算视频。 */
export function kindOf(r: StudioResult): ResultKind {
  if (r.path) return r.path.toLowerCase().endsWith(".mp4") ? "video" : "image";
  return VIDEO_MODES.has(r.mode) ? "video" : "image";
}

/** 按页签过滤。`all` 原样返回，**不复制** —— 下游只读。 */
export function filterResults(results: StudioResult[], key: FilterKey): StudioResult[] {
  if (key === "all") return results;
  return results.filter((r) => kindOf(r) === key);
}

/**
 * 三个页签各有多少条。
 *
 * **数的是全部，不是当前页签的** —— 页签上的数字要在切换前就能看到，
 * 否则「图片」旁边永远显示当前这一档的数量，等于没有信息。
 */
export function counts(results: StudioResult[]): Record<FilterKey, number> {
  let image = 0;
  let video = 0;
  for (const r of results) {
    if (kindOf(r) === "video") video++;
    else image++;
  }
  return { all: results.length, image, video };
}
