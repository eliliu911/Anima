/**
 * `/` 命令的纯逻辑：什么时候弹、匹配哪些、怎么解析。
 *
 * 抽出来是因为这几条规则都有**具体的反例**，而在一个 1100 行的组件里
 * 没法为它们写测试。真正会出事的三条都在这里：
 *
 * 1. 绝对路径 `/Users/you/...` 也以 `/` 开头，不能被当成命令吞掉；
 * 2. 多行输入里第二行开头打斜杠不该弹（那通常是在写 Markdown 列表或路径）；
 * 3. 命令名打完、开始打参数之后就该收起弹层 —— 否则 Enter 会被弹层劫走，
 *    用户永远发不出 `/review src/lru.ts`。
 */

import type { SlashEntry } from "../api/voice";

/**
 * 当前输入是不是「正在敲命令名」。返回已经敲了的部分（不含斜杠），
 * 不是的话返回 null。
 *
 * 判据是**整个 draft** 匹配 `/<非空白>*`：
 * - `"/"`、`"/rev"` → 在敲命令名，弹
 * - `"/review "`、`"/review a.ts"` → 已经在敲参数，收起（否则 Enter 被劫）
 * - `"/Users/x"` → 形式上仍是「命令名」，弹是对的 —— 但下面的匹配会一条都
 *   命中不了，于是弹层不显示。**靠匹配结果兜住路径，而不是靠正则去猜。**
 * - `"看这个\n/foo"` → 不弹（第一个字符不是斜杠）
 */
export function slashQuery(draft: string): string | null {
  const m = /^\/(\S*)$/.exec(draft);
  return m ? m[1] : null;
}

/**
 * 把查询词匹配到候选上。子串 + 首字母缩写，不引 fuzzy 库。
 *
 * 排序：完全相等 → 前缀 → 其它。**技能排在内置动作前面**，
 * 因为内置动作是固定的几个、用户很快就记住位置，而技能是会增长的那部分。
 */
export function filterEntries(entries: SlashEntry[], query: string): SlashEntry[] {
  const q = query.trim().toLowerCase();
  const scored = entries
    .map((e) => ({ e, s: score(e, q) }))
    .filter((x) => x.s > 0)
    .sort((a, b) => {
      if (b.s !== a.s) return b.s - a.s;
      // 同分时技能优先
      const ka = a.e.kind === "skill" ? 0 : 1;
      const kb = b.e.kind === "skill" ? 0 : 1;
      if (ka !== kb) return ka - kb;
      return a.e.id.localeCompare(b.e.id);
    });
  return scored.map((x) => x.e);
}

function score(e: SlashEntry, q: string): number {
  if (!q) return 1;
  const id = e.id.toLowerCase();
  if (id === q) return 100;
  if (id.startsWith(q)) return 50;
  if (id.includes(q)) return 20;
  // 首字母缩写：`cr` 命中 `code-reviewer`
  const initials = id
    .split(/[-_]/)
    .map((p) => p[0] ?? "")
    .join("");
  if (initials.startsWith(q)) return 30;
  // 标题里出现也算（中文命令名靠这条）
  if (e.title.toLowerCase().includes(q)) return 10;
  return 0;
}

/** 提交时解析 `/<name> <args...>`。不是命令形式就返回 null。 */
export function parseSubmitted(draft: string): { name: string; args: string } | null {
  const m = /^\/(\S+)\s*([\s\S]*)$/.exec(draft.trim());
  if (!m) return null;
  return { name: m[1], args: m[2].trim() };
}

/**
 * 选中一个候选之后，输入框该变成什么。
 *
 * 需要参数的留一个空格等着用户打；不需要参数的返回 null 表示「立刻执行」。
 */
export function completionFor(entry: SlashEntry): string | null {
  return entry.takes_args ? `/${entry.id} ` : null;
}

/** ↑↓ 之后的新下标，环绕。 */
export function moveIndex(current: number, delta: number, len: number): number {
  if (len === 0) return 0;
  return (current + delta + len) % len;
}

/**
 * 弹层是不是**刚刚打开**（上一次没开、这一次开了）。
 *
 * # 为什么要认这个瞬间
 *
 * 候选列表原来只在挂载和切工作区时拉一次。而技能是**会在对话里被创建的** ——
 * Agent 往 `.anima/skills/` 写完几个 SKILL.md，文件是好的、后端也认，
 * 但菜单还是进会话时那一份，得重启或切一次项目才看得见。
 *
 * 真出过事：Agent 报告「两个 Skill 已完整落地并校验无误」，用户去打 `/`，
 * 一个都没有。
 *
 * 在「刚打开」这一刻重拉一次就够了 —— 每次按键都拉是浪费，
 * 而后端本来就每次从磁盘读（几个文件、几 KB）。
 */
export function justOpened(prev: string | null, next: string | null): boolean {
  return prev === null && next !== null;
}
