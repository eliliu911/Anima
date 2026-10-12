import { describe, expect, it } from "vitest";
import type { Skill } from "../api/voice";
import { activeCount, emptyHint, skillRows } from "./skillsLoaded";

const sk = (id: string, extra: Partial<Skill> = {}): Skill =>
  ({ id, name: id, prompt: "正文", tools: [], scope: "global", kind: "persona", ...extra }) as Skill;

describe("技能行", () => {
  it("生效的排最前", () => {
    const rows = skillRows([sk("b"), sk("a"), sk("c")], ["c"]);
    expect(rows.map((r) => r.id)).toEqual(["c", "a", "b"]);
    expect(rows[0].active).toBe(true);
  });

  it("同为生效时项目层在前", () => {
    const rows = skillRows(
      [sk("g", { scope: "global" }), sk("p", { scope: "project" })],
      ["g", "p"],
    );
    expect(rows.map((r) => r.id)).toEqual(["p", "g"]);
  });

  /**
   * **空 tools = 不限制，不是一个工具都不给。** Rust 侧的注释专门写了这条 ——
   * 显示成「0 个工具」会让人以为这个技能把模型废了。
   */
  it("没写 tools 显示成不限制", () => {
    expect(skillRows([sk("a")], [])[0].toolCount).toBeNull();
    expect(skillRows([sk("a", { tools: ["view_file", "git_status"] })], [])[0].toolCount).toBe(2);
  });

  it("名字空了退回 id", () => {
    expect(skillRows([sk("x", { name: "" })], [])[0].name).toBe("x");
  });
});

describe("标题旁那个数字", () => {
  /** 这一格回答的是「什么在起作用」，不是「装了几个」。 */
  it("数的是生效的，不是认出的", () => {
    const rows = skillRows([sk("a"), sk("b"), sk("c")], ["b"]);
    expect(rows.length).toBe(3);
    expect(activeCount(rows)).toBe(1);
  });

  it("一个都没启用就是 0", () => {
    expect(activeCount(skillRows([sk("a"), sk("b")], []))).toBe(0);
  });
});

describe("空态", () => {
  /**
   * **「一个都没认出来」和「认出来了但没启用」是两回事。**
   * 以前共用一句「当前项目未调用技能」，把第二种说成了第一种 ——
   * 用户会去查加载，而其实只要去勾一下。
   */
  /**
   * **任务型不用挂载。** 一屋子任务技能却提示「去勾选」，
   * 是把人往一个无意义的动作上引。
   */
  it("全是任务技能时不提示去勾选", () => {
    const rows = skillRows([sk("a", { kind: "task" }), sk("b", { kind: "task" })], []);
    const msg = emptyHint(rows);
    expect(msg).toContain("不用挂载");
    expect(msg).not.toContain("勾选");
  });

  it("两种空态说的不是同一件事", () => {
    const none = emptyHint([]);
    const some = emptyHint(skillRows([sk("a"), sk("b")], []));
    expect(none).not.toBe(some);
    // 没认出来时要说清放哪儿、什么格式
    expect(none).toContain(".anima/skills/");
    expect(none).toContain("frontmatter");
    // 认出来了就别再让人去查加载
    expect(some).toContain("2");
    expect(some).toContain("/skill add");
  });

  /**
   * **两类要分开数。**「认出 13 个，一个都没挂载」把 5 个根本不需要挂载的
   * 任务技能也算了进去，用户会去 add 它们。
   */
  it("身份和任务分开数", () => {
    const rows = skillRows(
      [sk("p1"), sk("p2"), sk("t1", { kind: "task" }), sk("t2", { kind: "task" })],
      [],
    );
    const msg = emptyHint(rows);
    expect(msg).toContain("2 个身份技能");
    expect(msg).toContain("2 个任务技能");
    expect(msg, "把两类混成一个数了").not.toContain("4");
  });
});

// ── 接线 ──
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const strip = (t: string) =>
  t.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^\s*\/\/.*$/gm, "");
const PANEL = strip(
  readFileSync(fileURLToPath(new URL("./CodeViewerPanel.tsx", import.meta.url)), "utf8"),
);

/**
 * **那一格必须读真实状态，不能再扫工具名。**
 *
 * Anima 里没有任何一个工具的名字带 `skill` —— 技能是「选中即生效」，
 * 不是模型去调的动作。扫工具名的话那个数字结构上永远是 0，
 * 而用户看到 0 会以为技能没加载。
 */
describe("Skills Used 接线", () => {
  it("读 list_skills，不扫工具名", () => {
    expect(PANEL).toContain("listSkills(");
    expect(PANEL).toContain("skillRows(view.skills, view.active)");
    expect(PANEL, "又在扫工具名了").not.toContain('tc.name.toLowerCase().includes("skill")');
  });

  it("标题数字数的是生效的那几个", () => {
    expect(PANEL).toContain("activeCount(skillRowsState)");
    // 旧的那份必须删干净，否则两份真相并存
    expect(PANEL).not.toContain("skillsUsedSummary");
  });

  it("列表只列生效中的", () => {
    const i = PANEL.indexOf("skillRowsState");
    expect(i).toBeGreaterThan(0);
    expect(PANEL).toContain("filter((sk) => sk.active)");
  });

  /** 读失败不该弹错误打断工作 —— 它只是个侧栏格子。 */
  it("读不到就当空的", () => {
    const i = PANEL.indexOf("listSkills(");
    expect(PANEL.slice(i, i + 320)).toContain(".catch(");
  });
});
