import { describe, expect, it } from "vitest";
import {
  parseSkillCommand,
  renderSkillList,
  skillPickerEmptyHint,
  skillPickerEntries,
  skillPickerQuery,
  skillPickerTitle,
  validateTarget,
  type SkillLike,
} from "./skillCommand";

const sk = (id: string, kind = "persona", scope = "global", name = id): SkillLike =>
  ({ id, name, kind, scope });

describe("解析 /skill 的参数", () => {
  it("不带参数 = 看列表", () => {
    expect(parseSkillCommand("")).toEqual({ op: "list" });
    expect(parseSkillCommand("   ")).toEqual({ op: "list" });
  });

  /** 中文和符号都要收 —— 用户会打 `/skill 减 xxx`，那是他自然的写法。 */
  it("挂载和卸载的多种写法都认", () => {
    for (const v of ["add", "on", "+", "加", "挂载", "启用"]) {
      expect(parseSkillCommand(`${v} doc`), v).toEqual({ op: "add", id: "doc" });
    }
    for (const v of ["remove", "rm", "del", "off", "-", "减", "卸载", "禁用"]) {
      expect(parseSkillCommand(`${v} doc`), v).toEqual({ op: "remove", id: "doc" });
    }
  });

  it("少了 id 就说用法", () => {
    expect(parseSkillCommand("add")).toMatchObject({ op: "error" });
    expect((parseSkillCommand("add") as { message: string }).message).toContain("/skill add");
  });

  /**
   * **认不出的动词不要猜成 id。** `/skill foo` 更可能是打错了子命令，
   * 猜成「挂载 foo」会给出一句完全不相干的报错。
   */
  it("认不出的动词报用法，不当成 id", () => {
    const r = parseSkillCommand("foo");
    expect(r.op).toBe("error");
    expect((r as { message: string }).message).toContain("不认识");
  });

  it("id 里有空格也拼得回来", () => {
    expect(parseSkillCommand("add my skill")).toEqual({ op: "add", id: "my skill" });
  });
});

describe("列表怎么显示", () => {
  const all = [
    sk("doctor", "persona", "global", "家庭医生"),
    sk("short-drama-write", "task", "project", "短剧写作"),
  ];

  /** 任务型不用挂载 —— 混在一起列会让人去 add 它们，而那是无意义的动作。 */
  it("两类分开列，任务型给的是 / 用法", () => {
    const out = renderSkillList(all, ["doctor"]);
    expect(out).toContain("身份技能");
    expect(out).toContain("任务技能");
    expect(out).toContain("/short-drama-write");
    expect(out).toContain("不用挂载");
  });

  it("生效的标出来", () => {
    expect(renderSkillList(all, ["doctor"])).toContain("● doctor");
    expect(renderSkillList(all, [])).toContain("○ doctor");
  });

  /** **全局这件事要明说。** 藏着不说比说了更糟。 */
  it("说清身份技能的开关是全局的", () => {
    expect(renderSkillList(all, [])).toContain("全局");
  });

  it("项目层标出来", () => {
    expect(renderSkillList(all, [])).toContain("项目");
  });

  /**
   * **解析失败的必须列出来。**
   *
   * 那些技能在磁盘上、看着像装好了，而从来没被加载过。以前这条只进日志 ——
   * 实测 `kind: spec`（别的体系的取值）会让整份文件被静默跳过，
   * 而界面上完全看不出来。
   */
  it("加载失败的连原因一起列", () => {
    const out = renderSkillList(all, [], [
      { id: "juben-bianxie", error: "unknown variant `spec`" },
    ]);
    expect(out).toContain("juben-bianxie");
    expect(out).toContain("spec");
    expect(out).toContain("没能加载");
  });

  it("一个都没有时说清该放哪儿", () => {
    const out = renderSkillList([], []);
    expect(out).toContain(".anima/skills/");
    expect(out).toContain("frontmatter");
  });
});

describe("挂载/卸载前的检查", () => {
  const all = [sk("doctor"), sk("writer", "task")];

  it("认不出的 id 要把现有的列出来", () => {
    const msg = validateTarget(all, "nope", "add")!;
    expect(msg).toContain("doctor");
    expect(msg).toContain("writer");
  });

  /** 任务型不是「错误」，是用错了工具 —— 那个技能好好的，只是走另一条路。 */
  it("对任务型说清该怎么用，而不是报错", () => {
    const msg = validateTarget(all, "writer", "add")!;
    expect(msg).toContain("/writer");
    expect(msg).toContain("不需要挂载");
  });

  it("身份型放行", () => {
    expect(validateTarget(all, "doctor", "add")).toBeNull();
    expect(validateTarget(all, "doctor", "remove")).toBeNull();
  });
});

describe("二级挑选器", () => {
  const all = [
    sk("doctor", "persona", "global", "家庭医生"),
    sk("coach", "persona", "project", "教练"),
    sk("writer", "task", "project", "短剧写作"),
  ];

  /** 动词后面没空格不该弹 —— 那时用户可能还在打别的词。 */
  it("要打到动词后的空格才弹", () => {
    expect(skillPickerQuery("/skill")).toBeNull();
    expect(skillPickerQuery("/skill a")).toBeNull();
    expect(skillPickerQuery("/skill add")).toBeNull();
    expect(skillPickerQuery("/skill add ")).toEqual({ op: "add", q: "" });
    expect(skillPickerQuery("/skill add doc")).toEqual({ op: "add", q: "doc" });
  });

  it("中文动词一样弹", () => {
    expect(skillPickerQuery("/skill 加 ")).toEqual({ op: "add", q: "" });
    expect(skillPickerQuery("/skill 减 ")).toEqual({ op: "remove", q: "" });
  });

  it("不是 /skill 的不管", () => {
    expect(skillPickerQuery("/init ")).toBeNull();
    expect(skillPickerQuery("/skill foo ")).toBeNull();
    expect(skillPickerQuery("随便打的字")).toBeNull();
  });

  /**
   * **任务型不进挑选器。** 它不需要挂载，列出来只会让人做一个
   * 无意义的动作 —— 而那正是第一版的毛病。
   */
  it("add 只列还没挂的身份技能", () => {
    const rows = skillPickerEntries("add", all, ["doctor"], "");
    expect(rows.map((r) => r.id)).toEqual(["coach"]);
  });

  it("remove 只列已经挂着的", () => {
    const rows = skillPickerEntries("remove", all, ["doctor"], "");
    expect(rows.map((r) => r.id)).toEqual(["doctor"]);
  });

  /** 用户更可能记得中文名，所以 id 和名字都要能搜。 */
  it("id 和中文名都能搜", () => {
    expect(skillPickerEntries("add", all, [], "doc").map((r) => r.id)).toEqual(["doctor"]);
    expect(skillPickerEntries("add", all, [], "教练").map((r) => r.id)).toEqual(["coach"]);
    expect(skillPickerEntries("add", all, [], "没有的")).toEqual([]);
  });

  /**
   * **要用中文名。** 内置那八个身份技能全都有 `name_zh`
   * （「家庭医生」「心理咨询师」…），列表里显示 `Family Doctor`
   * 等于让用户在两套叫法之间自己对应。回落规则和设置面板一致。
   */
  it("中文界面显示中文名，缺了才回落", () => {
    const withZh = [sk("family-doctor", "persona", "global", "Family Doctor")];
    (withZh[0] as { name_zh?: string }).name_zh = "家庭医生";
    expect(skillPickerEntries("add", withZh, [], "", true)[0].name).toBe("家庭医生");
    expect(skillPickerEntries("add", withZh, [], "", false)[0].name).toBe("Family Doctor");
    // 没有中文名就用英文的
    expect(skillPickerEntries("add", [sk("x", "persona", "global", "X")], [], "", true)[0].name)
      .toBe("X");
  });

  it("中文名也能搜到", () => {
    const withZh = [sk("family-doctor", "persona", "global", "Family Doctor")];
    (withZh[0] as { name_zh?: string }).name_zh = "家庭医生";
    expect(skillPickerEntries("add", withZh, [], "家庭", true)).toHaveLength(1);
  });

  /**
   * **弹层要说清自己在列什么。**
   *
   * 用户看到侧栏「认出 13 个」、弹层里只有 8 条，会以为漏了 ——
   * 而那 5 个是任务技能，本来就不该出现在「挂载」里。
   * 用户原话：「一共显示出来 7 个……右边显示认出 13 个，你这咋处理的？」
   */
  it("标题说清列的是哪一类、另外那些去哪了", () => {
    const title = skillPickerTitle("add", all, 2);
    expect(title).toContain("身份技能");
    expect(title).toContain("2/2");
    // all 里有 1 个 task
    expect(title).toContain("1 个任务技能");
    expect(title).toContain("不用挂载");
    // 没有任务技能时不要多那半句
    const onlyPersona = skillPickerTitle("add", [sk("a"), sk("b")], 2);
    expect(onlyPersona).not.toContain("任务技能");
  });

  it("标出项目层还是全局", () => {
    const rows = skillPickerEntries("add", all, [], "");
    expect(rows.find((r) => r.id === "coach")!.hint).toBe("项目");
    expect(rows.find((r) => r.id === "doctor")!.hint).toBe("全局");
  });

  /**
   * **空了不能一片空白** —— 打了 `/skill add ` 却什么都没有，看着像卡住了。
   * 而且三种空是三件不同的事。
   */
  it("三种空态说的不是同一件事", () => {
    const 全挂上了 = skillPickerEmptyHint("add", all, "");
    const 搜不到 = skillPickerEmptyHint("add", all, "zzz");
    const 没有身份技能 = skillPickerEmptyHint("add", [sk("w", "task")], "");
    expect(new Set([全挂上了, 搜不到, 没有身份技能]).size).toBe(3);
    expect(搜不到).toContain("zzz");
    expect(没有身份技能).toContain("不用挂载");
    expect(skillPickerEmptyHint("remove", all, "")).toContain("没挂");
  });
});

// ── 接线 ──
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const strip = (t: string) =>
  t.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^\s*\/\/.*$/gm, "");
const BAR = strip(
  readFileSync(fileURLToPath(new URL("./CommandBar.tsx", import.meta.url)), "utf8"),
);
const RUST = readFileSync(
  fileURLToPath(new URL("../../src-tauri/src/commands_voice.rs", import.meta.url)),
  "utf8",
);

describe("/skill 接线", () => {
  it("后端注册了它，而且声明要参数", () => {
    const i = RUST.indexOf('id: "skill".into()');
    expect(i, "没注册 /skill").toBeGreaterThan(0);
    const block = RUST.slice(i, i + 600);
    expect(block).toContain("takes_args: true");
    expect(block).toContain('action: Some("skill")');
  });

  /** 不带参数的话 `/skill add doc` 里的 id 到不了执行函数。 */
  it("参数真的传给了 runBuiltin", () => {
    expect(BAR).toContain("runBuiltin(matched.action, parsed.args)");
    expect(BAR).toContain('async function runBuiltin(action: string, args = "")');
  });

  it("三种操作都接上了", () => {
    expect(BAR).toContain("parseSkillCommand(args)");
    expect(BAR).toContain("renderSkillList(view.skills, view.active, view.broken ?? [])");
    expect(BAR).toContain("validateTarget(view.skills, cmd.id, cmd.op)");
    expect(BAR).toContain("selectSkills(next, workspacePath)");
  });

  /**
   * **加载失败的要带到列表里。** 那是这条命令最有价值的一半 ——
   * 以前一个解析失败的技能只有一行日志，界面上完全看不出来。
   */
  it("列表带上加载失败的那些", () => {
    expect(BAR).toContain("view.broken ?? []");
  });

  /** 「已经挂着了」和「挂上了」都成功，而用户要的是前者的确认。 */
  it("没变化也报一声", () => {
    expect(BAR).toContain("已经挂着了");
    expect(BAR).toContain("本来就没挂");
  });

  /**
   * **挑选器要和 `/` 弹层共用同一套键盘操作。**
   *
   * 各写一份的话，上下键在一个弹层里能用、另一个里不能 ——
   * 而两个弹层长得一模一样。
   */
  it("挑选器接进了同一个弹层和同一套键盘", () => {
    expect(BAR).toContain("skillPickerQuery(draft)");
    expect(BAR).toContain("(slashOpen || skillPickerOpen)");
    // 上下键的长度要按当前弹的是哪一个算
    expect(BAR).toContain("skillPickerOpen ? skillPicks.length : slashMatches.length");
    // 两个弹层不能同时显示
    expect(BAR).toContain("{slashOpen && !skillPickerOpen && (");
  });

  /** 空了也要开着弹层说明原因 —— 一片空白看着像卡住了。 */
  it("空挑选器显示原因而不是消失", () => {
    expect(BAR).toContain("skillPicks.length === 0");
    expect(BAR).toContain("skillPickerEmptyHint(");
    // 空的时候回车不该干事
    const i = BAR.indexOf("const hit = skillPicks[slashIndex]");
    expect(BAR.slice(i, i + 200)).toContain("if (hit && skillPicker)");
  });

  /** 挂完之后候选池要变，否则刚挂上的还留在 add 列表里。 */
  it("挂载卸载后刷新候选池", () => {
    const i = BAR.indexOf("selectSkills(next, workspacePath)");
    expect(BAR.slice(i, i + 200)).toContain("reloadSkillPool()");
  });

  /** 挂载写的是全局配置 —— 这是实现事实，藏着不说更糟。 */
  it("挂载成功时说明是全局的", () => {
    const i = BAR.indexOf("已挂载");
    expect(i).toBeGreaterThan(0);
    expect(BAR.slice(i, i + 120)).toContain("全局");
  });
});

describe("中文名接线", () => {
  it("弹层按当前界面语言选名字", () => {
    const i = BAR.indexOf("skillPickerEntries(");
    expect(i).toBeGreaterThan(0);
    expect(BAR.slice(i, i + 260)).toContain('i18n.language.startsWith("zh")');
  });

  it("弹层顶上有说明列的是什么", () => {
    expect(BAR).toContain("skillPickerTitle(skillPicker!.op, skillPool.all, skillPicks.length)");
  });
});

describe("打开弹层就重拉", () => {
  it("认得出「刚打开」这一刻并重拉两份数据", () => {
    expect(BAR).toContain("justOpened(prevSlashQ.current, slashQ)");
    const i = BAR.indexOf("justOpened(prevSlashQ.current, slashQ)");
    const block = BAR.slice(i, i + 260);
    expect(block).toContain("refreshSlash()");
    // 斜杠菜单和 /skill 的候选池是两份数据，都要刷
    expect(block).toContain("reloadSkillPool()");
  });

  /** 读失败要保持上一份 —— 清空会让已经打开的弹层突然空掉。 */
  it("读失败不清空已有候选", () => {
    const i = BAR.indexOf("const refreshSlash");
    const block = BAR.slice(i, i + 320);
    expect(block).toContain(".catch(() => {})");
    expect(block, "读失败把列表清空了").not.toContain("setSlashEntries([])");
  });
});
