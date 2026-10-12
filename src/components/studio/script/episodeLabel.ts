import type { ScriptDocument } from "../../../api/scriptLibrary";

/**
 * 剧本库的层级：**项目 → 原文 / 剧本 → 分集**。
 *
 * # 为什么要有编号，而不是只有标题
 *
 * 用户原话：「项目：余波，下面两个子目录，原文，剧本，然后各自里面有章节。
 * 这样才符合情节。比如 **S01E01 就是分集**。」
 *
 * 这是行业里的写法，而且它带着**顺序**：`S01E02` 一眼看出在 `S01E01`
 * 之后，而「余波」「第 1 集」「未命名剧本」混在一张平表里看不出先后 ——
 * 而剧本最要紧的属性之一就是顺序。
 *
 * # 季暂时恒为 1
 *
 * 数据里还没有「季」这个东西。**恒写 S01 而不是省掉它**：
 * 省掉的话以后加季时所有已有编号都要改写，而 `E01` 和 `S02E01` 并排
 * 时也看不出前者是哪一季。
 */
export const SEASON = 1;

/** 从标题里读集号。读不出来返回 `null` —— **不猜**。 */
export function episodeNumberIn(title: string): number | null {
  const m = /(?:第\s*(\d+)\s*集|[Ss](\d+)[Ee](\d+)|[Ee][Pp]?\s*(\d+))/.exec(title ?? "");
  if (!m) return null;
  const n = Number(m[1] ?? m[3] ?? m[4]);
  return Number.isFinite(n) && n > 0 ? n : null;
}

export function pad2(n: number): string {
  return String(Math.max(0, Math.floor(n))).padStart(2, "0");
}

/** `S01E03` 这种写法。 */
export function seasonEpisode(n: number, season = SEASON): string {
  return `S${pad2(season)}E${pad2(n)}`;
}

/**
 * 一份剧本在列表里显示成什么。
 *
 * 标题里写了集号就用它，没写就**按它在列表里的位置**补一个 ——
 * 这样任何一份剧本都有编号，而不是「有的有有的没有」。
 *
 * 标题和编号**分开返回**：编号是结构，标题是内容，
 * 界面上要能把结构那一列对齐。
 */
export interface EpisodeLabel {
  /** `S01E02` */
  code: string;
  /** 去掉集号之后剩下的标题。全被吃掉时为空串 —— 调用方自己兜。 */
  name: string;
  /** 集号是从标题里读到的，还是按位置补的。**补的要标出来**。 */
  inferred: boolean;
}

export function episodeLabel(doc: ScriptDocument, index: number): EpisodeLabel {
  const title = (doc.title ?? "").trim();
  const found = episodeNumberIn(title);
  const n = found ?? index + 1;
  // 标题里那段集号去掉 —— 留着的话行首会是「S01E01 第 1 集」，同一件事说两遍
  const name = title
    .replace(/第\s*\d+\s*集/g, "")
    .replace(/[Ss]\d+[Ee]\d+/g, "")
    .replace(/[Ee][Pp]?\s*\d+/g, "")
    .replace(/^[\s·、:：\-—]+|[\s·、:：\-—]+$/g, "")
    .trim();
  return { code: seasonEpisode(n), name, inferred: found === null };
}

/**
 * 新增一集叫什么。**接着已有的最大集号往下排**，不是按数量 ——
 * 删掉第 2 集之后再加一集应该是第 4 集，叫第 3 集会和还在的那一集重名。
 */
export function nextEpisodeName(screenplays: ScriptDocument[]): string {
  let max = 0;
  for (const d of screenplays) {
    const n = episodeNumberIn(d.title ?? "");
    if (n) max = Math.max(max, n);
  }
  return seasonEpisode(max + 1);
}
