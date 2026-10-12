/**
 * 侧栏「Skills Used」那一格显示什么。
 *
 * # 它以前测的是一件不存在的事
 *
 * 原来是扫会话里**名字带 `skill` 的工具调用**：
 *
 * ```ts
 * if (tc.name.toLowerCase().includes("skill")) { ... }
 * ```
 *
 * 而 Anima 里**没有任何一个工具叫这个名字**。技能不是模型去调的动作 ——
 * 它是「选中即生效」的一层：SKILL.md 的正文进 System Prompt，
 * frontmatter 里的 `tools` **收窄模型能调什么**。
 *
 * 所以那个数字**结构上永远是 0**，跟装了几个技能、用没用过都无关。
 * 用户看到 0 会以为技能没加载 —— 而实际上五个都加载了，
 * 只是没有任何东西能证明这件事。
 *
 * 现在它显示的是 `list_skills` 返回的**真实状态**：磁盘上认出了哪些、
 * 这次会话生效的是哪些。
 */
import type { Skill } from "../api/voice";

export interface SkillRow {
  id: string;
  name: string;
  /** 生效中 = 正文已经进了 System Prompt */
  active: boolean;
  /** 项目层（`.anima/skills/`）还是全局 */
  scope: string;
  /** `task` = 走 `/` 弹层，这一轮生效；其余是身份型，挂上持续生效 */
  kind: string;
  /**
   * 这个技能把工具面收窄成了几个。`null` = 不限制。
   *
   * **空数组是「不限制」，不是「一个工具都不给」** —— 后者会让忘写
   * 这一项的技能悄悄失去所有工具，Rust 侧的注释里专门写了这条。
   */
  toolCount: number | null;
}

/**
 * 排序：**生效的排前面**，然后项目层，最后按 id。
 *
 * 生效的排前面是因为那一格的问题只有一个 ——「现在到底哪个在起作用」。
 */
export function skillRows(skills: readonly Skill[], active: readonly string[]): SkillRow[] {
  const on = new Set(active);
  return skills
    .map((s) => ({
      id: s.id,
      name: s.name || s.id,
      active: on.has(s.id),
      scope: s.scope ?? "global",
      kind: s.kind ?? "persona",
      toolCount: s.tools && s.tools.length > 0 ? s.tools.length : null,
    }))
    .sort((a, b) => {
      if (a.active !== b.active) return a.active ? -1 : 1;
      if (a.scope !== b.scope) return a.scope === "project" ? -1 : 1;
      return a.id.localeCompare(b.id);
    });
}

/**
 * 标题旁边那个数字。
 *
 * **显示的是「生效几个」，不是「认出几个」。** 认出多少是设置面板的事；
 * 这一格回答的是「这次会话里什么在起作用」。
 */
export function activeCount(rows: readonly SkillRow[]): number {
  return rows.filter((r) => r.active).length;
}

/**
 * 空态说什么。**「一个都没认出来」和「认出来了但没启用」是两回事**，
 * 而以前它们共用一句「当前项目未调用技能」—— 那句话把第二种情况
 * 说成了第一种，用户会去查加载，而其实只要去勾一下。
 */
export function emptyHint(rows: readonly SkillRow[]): string {
  if (rows.length === 0) {
    return "这个工作区没有认出技能。放到 .anima/skills/<id>/SKILL.md，要有 --- frontmatter 和非空正文";
  }
  // **两类要分开数。** 「认出 13 个，一个都没挂载」把 5 个根本不需要
  // 挂载的任务技能也算了进去 —— 用户会去 add 它们，而那是无意义的动作。
  // 用户的原话：「一共显示出来 7 个……右边显示认出 13 个，你这咋处理的？」
  const tasks = rows.filter((r) => r.kind === "task").length;
  const personas = rows.length - tasks;
  if (personas === 0) {
    return `认出 ${tasks} 个任务技能 —— 不用挂载，在输入框打 / 选它就跑`;
  }
  const tail = tasks > 0 ? `；另有 ${tasks} 个任务技能，打 / 直接用` : "";
  return `${personas} 个身份技能一个都没挂 —— 打 /skill add 挑一个${tail}`;
}
