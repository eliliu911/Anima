/**
 * 系列资产相关的判断和文案。纯函数，测得到。
 */
import type { ImportSummary } from "../../../api/cinema";
import type { AdoptOutcome, AdoptSummary } from "../../../api/seriesAssets";

/** 提取弹窗里每个实体的去向。点一下换下一个。 */
export type Pick = "series" | "episode" | "skip";

export function nextPick(p: Pick, allowEpisode = true): Pick {
  if (!allowEpisode) return p === "series" ? "skip" : "series";
  return p === "series" ? "episode" : p === "episode" ? "skip" : "series";
}

/**
 * 设定文档默认勾哪几份：标题里带人物 / 角色 / 场景 / 道具 / 设定 / 世界观的。
 * 大纲、分集、章节原文不勾 —— 那些是剧情，从剧情里抽出来的描述是「这一场他穿什么」，
 * 不是「他长什么样」。
 */
export function isDesignDoc(title: string): boolean {
  return /人物|角色|场景|道具|设定|世界观/.test(title) && !/^第\s*\d+\s*章/.test(title);
}

/** 类型 + 名字才唯一 —— 「铜壶」既可以是道具也可以是场景。和后端 `episode_only` 的键同一个格式。 */
export function entityKey(e: { kind: string; name: string }): string {
  return `${e.kind}/${e.name.trim()}`;
}

/**
 * 「星澜（战斗款）」+ 人设「星澜」→ 「星澜」。和后端 `series_assets::split_look` 同一条规则：
 * 名字以人设名开头、后面紧跟一对括号才拆（XN-0001 的人设也被标成星澜，但它是另一台机器）。
 * 拆不出来就是名字本身。
 */
export function characterBase(e: { name: string; persona?: string }): string {
  const name = e.name.trim();
  const base = (e.persona ?? "").trim();
  if (!base || !name.startsWith(base)) return name;
  const rest = name.slice(base.length).trim();
  const m = rest.match(/^[（(](.+)[）)]$/);
  return m && m[1].trim() ? base : name;
}

/**
 * 系列里这一类现在几个、应用后几个。
 *
 * **同一个人的几套造型算一个**（后端合成一个角色 + 几个形象）。
 *
 * **已有的不加一。** 「陆川」系列里已经有了，提取出来也只是并进去 ——
 * 显示成 `7 → 8` 会让人以为又长出一个陆川，那正是这次要消灭的问题。
 */
export function seriesCount(
  entities: { kind: string; name: string; persona?: string }[],
  picks: Record<string, Pick>,
  existing: Set<string>,
  kind: string,
): { before: number; after: number } {
  const before = [...existing].filter((k) => k.startsWith(`${kind}/`)).length;
  const fresh = new Set(
    entities
      .filter((e) => e.kind === kind && picks[entityKey(e)] === "series")
      .map((e) => `${kind}/${kind === "character" ? characterBase(e) : e.name.trim()}`)
      .filter((k) => !existing.has(k)),
  );
  return { before, after: before + fresh.size };
}

export function importNote(s: ImportSummary): string {
  const parts: string[] = [];
  if (s.added) parts.push(`系列里新建 ${s.added} 个`);
  if (s.merged) parts.push(`${s.merged} 个系列里已有，并进去了`);
  if (s.looks) parts.push(`同一个人的其它造型加成了 ${s.looks} 个形象`);
  if (s.episode_only) parts.push(`${s.episode_only} 个只留本集`);
  if (!parts.length) return "没有改动";
  return `${parts.join("，")}。去「角色与资产」能看到；提示词里用 @ 引用它们`;
}

export function adoptNote(o: AdoptOutcome): string {
  return o.kind === "moved"
    ? `「${o.name}」已进系列`
    : `「${o.name}」系列里已有，并进去了${o.shots ? `（改了 ${o.shots} 镜的引用）` : ""}`;
}

export function adoptAllNote(s: AdoptSummary): string {
  const parts: string[] = [];
  if (s.moved) parts.push(`${s.moved} 个进了系列`);
  if (s.absorbed) parts.push(`${s.absorbed} 个同名的并进了已有的`);
  if (!parts.length) return "没有待归属的";
  return `${parts.join("，")}。改之前的数据备份在 backups/series-*`;
}
