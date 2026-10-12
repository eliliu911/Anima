/**
 * 系列概览页的数字。纯函数，测得到。
 *
 * **判据和左栏的步骤标记一致**（`stepStates.ts`）：剧本有字才算写了，
 * 有镜头才算出了分镜，有合成片才算成片。两处说法不一样的话，
 * 用户会不知道该信哪一个。
 */
import type { EpisodeData } from "../../../api/cinema";
import type { AssetView } from "../../../api/assets";

export interface SeriesStats {
  total: number;
  scripted: number;
  boarded: number;
  finished: number;
}

export function seriesStats(episodes: EpisodeData[]): SeriesStats {
  return {
    total: episodes.length,
    scripted: episodes.filter((e) => (e.script_text ?? "").trim().length > 0).length,
    boarded: episodes.filter((e) => (e.storyboard ?? []).length > 0).length,
    finished: episodes.filter((e) => !!e.final_video_url).length,
  };
}

/** 一张分集卡上那行进度。只说做到了哪一步，不说「没做」的。 */
export function episodeProgress(e: EpisodeData): string {
  if (e.final_video_url) return "已合成";
  const shots = e.storyboard ?? [];
  if (shots.length) {
    const rendered = shots.filter((s) => s.video_url).length;
    return rendered ? `${shots.length} 镜 · 出片 ${rendered}/${shots.length}` : `${shots.length} 镜 · 未出片`;
  }
  const n = (e.script_text ?? "").trim().length;
  return n ? `剧本 ${n} 字` : "还没写剧本";
}

/** 卡片上的标题：去掉「E01-」这类前缀（卡片左上角已经有集号）。 */
export function cardTitle(e: EpisodeData): string {
  return e.title.replace(/^E\d+\s*[-·—:：]?\s*/, "").replace(/^《(.*)》$/, "$1") || e.title;
}

/**
 * 主演条：系列里的角色，**有图的排前面**，同样有图按名字。
 * 画外音角色不进主演条 —— 它没有脸。
 */
export function mainCast(assets: AssetView[]): AssetView[] {
  return assets
    .filter((a) => a.kind === "character" && !a.character?.disembodied)
    .sort((a, b) => Number(!!b.cover_url) - Number(!!a.cover_url) || a.name.localeCompare(b.name, "zh"));
}

/**
 * 已经按当前画幅出过的首帧和视频有多少 —— 换画幅之前要说出来。
 *
 * 画幅不进合成提示词，所以提示词不用重来；但首帧、视频的尺寸是按画幅出的，
 * 换了之后这些要重出才对得上（组装时会被加黑边或裁掉）。
 */
export function renderedOnAspect(episodes: EpisodeData[]): { frames: number; videos: number } {
  let frames = 0;
  let videos = 0;
  for (const e of episodes) {
    for (const s of e.storyboard ?? []) {
      if (s.image_url) frames += 1;
      if (s.video_url) videos += 1;
    }
  }
  return { frames, videos };
}

/** 画幅的叫法。没有叫法的（4:3、3:4）只显示比例。 */
export const ASPECT_LABEL: Record<string, string> = {
  "16:9": "横屏",
  "21:9": "宽银幕",
  "9:16": "竖屏",
  "1:1": "方形",
};
