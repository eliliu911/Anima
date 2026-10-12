/**
 * `/skill` —— 在对话里查看、挂载、卸载技能。
 *
 * # 为什么要有它
 *
 * 技能分两类。**任务型**（`kind: task`）本来就走 `/` 弹层，选一次这一轮生效，
 * 跟着工作区走 —— 那条路一直是通的。**身份型**（persona）是持续生效的，
 * 而它的开关只在设置面板里，那个面板是**全局**的。
 *
 * 于是出现一个错位：身份技能可以是项目层的（`.anima/skills/`），
 * 而开关是全局的 —— 在 A 项目勾上一个只有 A 有的身份，切到 B 还挂着。
 * 用户的原话：「那个页面是全局设计的，不应该作为单独的项目或者
 * conversation 来使用」。
 *
 * `/skill` 不能修好那个错位（开关还是写全局配置），但它做到两件事：
 * **把开关搬进对话**，以及**把全局这件事明说出来**。
 */

/** 一次 `/skill` 要干什么。 */
export type SkillOp =
  | { op: "list" }
  | { op: "add"; id: string }
  | { op: "remove"; id: string }
  | { op: "error"; message: string };

/**
 * 解析 `/skill` 后面那串参数。
 *
 * 认这几种写法：`add|on|+` 和 `remove|rm|off|-|减`。
 * **中文和符号都收** —— 用户会打 `/skill 减 xxx`，那是他自然的写法，
 * 报「未知子命令」不如直接认。
 */
export function parseSkillCommand(args: string): SkillOp {
  const parts = args.trim().split(/\s+/).filter(Boolean);
  if (parts.length === 0) return { op: "list" };

  const [verb, ...rest] = parts;
  const id = rest.join(" ").trim();
  const ADD = ["add", "on", "+", "加", "挂载", "启用"];
  const DEL = ["remove", "rm", "del", "off", "-", "减", "卸载", "禁用"];

  if (ADD.includes(verb)) {
    return id ? { op: "add", id } : { op: "error", message: "要挂载哪个？用法：/skill add <id>" };
  }
  if (DEL.includes(verb)) {
    return id
      ? { op: "remove", id }
      : { op: "error", message: "要卸载哪个？用法：/skill remove <id>" };
  }
  // **认不出的动词不要猜成 id。** `/skill foo` 更可能是打错了子命令，
  // 而当成「挂载名叫 foo 的技能」会给出一句完全不相干的报错。
  return {
    op: "error",
    message: `不认识「${verb}」。用法：/skill（看列表）· /skill add <id> · /skill remove <id>`,
  };
}

export interface SkillLike {
  id: string;
  name: string;
  /**
   * 中文界面下的显示名。**要用它** —— 内置那八个身份技能全都有中文名
   * （「家庭医生」「心理咨询师」…），而列表里显示 `Family Doctor`
   * 等于让用户在两套叫法之间自己对应。
   *
   * 回落规则和设置面板一致：`(中文界面 && name_zh) || name`。
   */
  name_zh?: string;
  kind?: string;
  scope?: string;
}

/** 显示名。`zh` = 当前是不是中文界面。 */
export function displayName(s: SkillLike, zh: boolean): string {
  return (zh && s.name_zh) || s.name || s.id;
}

/**
 * `/skill` 不带参数时打印什么。
 *
 * 三件事必须说清楚：
 *
 * 1. **任务型不用挂载** —— 直接 `/<id>` 就跑。混在一起列会让人去 add 它们，
 *    而 add 一个任务型技能是没有意义的动作
 * 2. **身份型的开关是全局的** —— 这是当前的实现事实，藏着不说更糟
 * 3. **解析失败的要列出来** —— 那些技能在磁盘上、看起来像装好了，
 *    而它们从来没被加载过。以前这条只进日志（实测：`kind: spec`
 *    这种别的体系的取值会让整份文件被静默跳过）
 */
export function renderSkillList(
  all: readonly SkillLike[],
  active: readonly string[],
  broken: readonly { id: string; error: string }[] = [],
): string {
  const on = new Set(active);
  const tasks = all.filter((s) => (s.kind ?? "persona") === "task");
  const personas = all.filter((s) => (s.kind ?? "persona") !== "task");
  const lines: string[] = [];

  if (personas.length > 0) {
    lines.push("身份技能（持续生效，开关是全局的）：");
    for (const s of personas) {
      const mark = on.has(s.id) ? "●" : "○";
      const scope = s.scope === "project" ? " · 项目" : "";
      lines.push(`  ${mark} ${s.id}${scope} —— ${displayName(s, true)}`);
    }
    lines.push("  用 /skill add <id> 挂载、/skill remove <id> 卸载");
  }

  if (tasks.length > 0) {
    if (lines.length > 0) lines.push("");
    lines.push("任务技能（不用挂载，直接打 / 选它，这一轮生效）：");
    for (const s of tasks) {
      const scope = s.scope === "project" ? " · 项目" : "";
      lines.push(`  /${s.id}${scope} —— ${displayName(s, true)}`);
    }
  }

  if (broken.length > 0) {
    if (lines.length > 0) lines.push("");
    lines.push("这几个没能加载（文件在，但解析失败）：");
    for (const b of broken) lines.push(`  ✗ ${b.id} —— ${b.error}`);
  }

  if (lines.length === 0) {
    return "这个工作区没有认出技能。放到 .anima/skills/<id>/SKILL.md，要有 --- frontmatter 和非空正文";
  }
  return lines.join("\n");
}

/**
 * `add` / `remove` 前的检查。通过返回 `null`，否则返回要显示的那句话。
 *
 * **任务型不能被挂载**要单独说 —— 它不是错误，是用错了工具：
 * 那个技能好好的，只是走另一条路。
 */
export function validateTarget(
  all: readonly SkillLike[],
  id: string,
  op: "add" | "remove",
): string | null {
  const hit = all.find((s) => s.id === id);
  if (!hit) {
    const ids = all.map((s) => s.id).join("、");
    return ids
      ? `没有叫「${id}」的技能。现在有：${ids}`
      : `没有叫「${id}」的技能，这个工作区一个都没认出来`;
  }
  if ((hit.kind ?? "persona") === "task") {
    return `「${id}」是任务技能，不需要挂载 —— 直接打 /${id} 就会执行这一轮。`;
  }
  return op === "add" || op === "remove" ? null : null;
}

// ---------------------------------------------------------------------------
// 二级挑选器：`/skill add ` 之后直接弹可选列表
// ---------------------------------------------------------------------------

/**
 * `/skill add ` / `/skill remove ` 后面要不要弹挑选器。
 *
 * # 为什么不让人打 id
 *
 * 第一版是 `/skill add <id>` —— 用户得先 `/skill` 看一遍列表、记住 id、
 * 再打一遍。用户的原话：「谁会看 ID 啊」。
 *
 * 现在打到 `/skill add ` 就弹出可选的那几个，上下键选、回车确认，
 * 和 `/` 弹层完全一样的操作 —— 那套交互本来就在，只是没接过来。
 *
 * **动词后面必须已经有空格**（或者正在打后半截），否则打到 `/skill a`
 * 的时候就弹了，而那时用户可能是想打 `add` 之外的东西。
 */
export function skillPickerQuery(draft: string): { op: "add" | "remove"; q: string } | null {
  const m = /^\/skill\s+(\S+)(?:\s+(\S*))?$/.exec(draft);
  if (!m) return null;
  const verb = m[1];
  // 只有动词后面真的跟了空格才算 —— `m[2]` 是 undefined 表示还没打空格
  if (m[2] === undefined) return null;
  const cmd = parseSkillCommand(`${verb} x`);
  if (cmd.op !== "add" && cmd.op !== "remove") return null;
  return { op: cmd.op, q: m[2] };
}

/** 挑选器里的一行。 */
export interface SkillPick {
  id: string;
  name: string;
  /** 右边那行小字：说清它现在什么状态 */
  hint: string;
}

/**
 * 挑选器列什么。
 *
 * - `add` 列**还没挂上的身份技能**。任务型不在这儿 —— 它不需要挂载，
 *   列出来只会让人做一个无意义的动作
 * - `remove` 列**已经挂着的**
 *
 * `q` 做子串过滤，id 和名字都匹配（用户更可能记得中文名）。
 */
export function skillPickerEntries(
  op: "add" | "remove",
  all: readonly SkillLike[],
  active: readonly string[],
  q: string,
  zh = true,
): SkillPick[] {
  const on = new Set(active);
  const pool = all.filter((s) => {
    if ((s.kind ?? "persona") === "task") return false;
    return op === "add" ? !on.has(s.id) : on.has(s.id);
  });
  const needle = q.trim().toLowerCase();
  return pool
    .filter(
      (s) =>
        !needle ||
        s.id.toLowerCase().includes(needle) ||
        s.name.toLowerCase().includes(needle) ||
        (s.name_zh ?? "").toLowerCase().includes(needle),
    )
    .map((s) => ({
      id: s.id,
      name: displayName(s, zh),
      hint: s.scope === "project" ? "项目" : "全局",
    }));
}

/**
 * 挑选器空了的时候说什么。**不能什么都不显示** ——
 * 打了 `/skill add ` 却一片空白，看着像卡住了。
 */
/**
 * 挑选器顶上那行说明。
 *
 * **必须说清自己在列什么。** 用户看到侧栏写「认出 13 个」、弹层里只有 8 条，
 * 会以为漏了 —— 而那 5 个是任务技能，本来就不该出现在「挂载」这个动作里。
 * 数字对不上的时候，缺的不是数字，是那句话。
 */
export function skillPickerTitle(
  op: "add" | "remove",
  all: readonly SkillLike[],
  shown: number,
): string {
  const personas = all.filter((s) => (s.kind ?? "persona") !== "task").length;
  const tasks = all.length - personas;
  const head =
    op === "add"
      ? `可挂载的身份技能 ${shown}/${personas}`
      : `已挂载 ${shown}`;
  return tasks > 0 ? `${head}　（另有 ${tasks} 个任务技能，不用挂载，打 / 直接选）` : head;
}

export function skillPickerEmptyHint(
  op: "add" | "remove",
  all: readonly SkillLike[],
  q: string,
): string {
  const personas = all.filter((s) => (s.kind ?? "persona") !== "task");
  if (personas.length === 0) {
    return "这个工作区没有身份技能。任务技能不用挂载，直接打 / 选它就跑";
  }
  if (q.trim()) return `没有匹配「${q.trim()}」的技能`;
  return op === "add" ? "身份技能都已经挂上了" : "现在一个身份技能都没挂";
}
