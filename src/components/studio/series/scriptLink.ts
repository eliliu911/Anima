/**
 * 工作区系列 ↔ 剧本编辑器项目的对应。
 *
 * **先按绑定的 id，再按名字兜底。** 以前只按名字、而且逐字比 —— 系列名常带书名号
 * （「《回收师》」对不上「回收师」），一对不上「故事原文」就只会停在默认位置。
 * 按名字认出来的，调用方要把它存成绑定（`cinemaLinkScriptProject`），之后谁改名都不断。
 */
import type { ScriptProject } from "../../../api/scriptLibrary";

/** 比名字时去掉书名号、引号、空白，大小写不敏感 */
export function normalizeTitle(t: string): string {
  return t.replace(/[《》〈〉「」『』“”‘’"'\s]/g, "").toLowerCase();
}

export type ScriptMatch = { project: ScriptProject; via: "id" | "name" } | null;

export function resolveScriptProject(
  projects: ScriptProject[],
  cinema: { title: string; script_project_id?: string | null },
): ScriptMatch {
  if (cinema.script_project_id) {
    const hit = projects.find((p) => p.id === cinema.script_project_id);
    if (hit) return { project: hit, via: "id" };
  }
  const want = normalizeTitle(cinema.title);
  if (!want) return null;
  const same = projects.filter((p) => normalizeTitle(p.title) === want);
  // 同名的不止一个就不猜 —— 让人选
  return same.length === 1 ? { project: same[0], via: "name" } : null;
}
