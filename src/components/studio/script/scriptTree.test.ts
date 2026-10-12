import { describe, it, expect } from "vitest";
import { groupDocuments } from "./ScriptTree";
import type { ScriptDocument, ScriptProject } from "../../../api/scriptLibrary";

const doc = (p: Partial<ScriptDocument> & { id: string }): ScriptDocument => ({
  title: p.id, text: "", doc: null, revision: 0,
  project_id: null, kind: "screenplay", derived_from: null, ...p,
});
const proj = (id: string, title: string): ScriptProject =>
  ({ id, title, note: "", created_at: 0, updated_at: 0 });

describe("剧本资源树的分组", () => {
  it("按项目和「原文 / 剧本」两层分开", () => {
    const groups = groupDocuments(
      [proj("p1", "山村来客")],
      [
        doc({ id: "s1", project_id: "p1", kind: "source" }),
        doc({ id: "e1", project_id: "p1", kind: "screenplay" }),
      ],
      "",
    );
    const p1 = groups.find((g) => g.id === "p1")!;
    expect(p1.sources.map((d) => d.id)).toEqual(["s1"]);
    expect(p1.screenplays.map((d) => d.id)).toEqual(["e1"]);
  });

  /**
   * **存量数据全在「未归档」里。**
   * 这一组哪怕空着也要留着 —— 藏起来等于让人以为东西丢了。
   */
  it("未归档那一组永远存在，即使是空的", () => {
    expect(groupDocuments([], [], "").some((g) => g.id === null)).toBe(true);
  });

  /**
   * 老文档没有 kind / project_id 字段，读出来是 undefined。
   * 当成剧本 + 未归档 —— 那正是它们本来的语义。
   */
  it("缺字段的老文档落到「未归档 · 剧本」", () => {
    const legacy = { id: "old", title: "旧稿", text: "正文", doc: null, revision: 3 } as ScriptDocument;
    const loose = groupDocuments([], [legacy], "").find((g) => g.id === null)!;
    expect(loose.screenplays.map((d) => d.id)).toEqual(["old"]);
    expect(loose.sources).toEqual([]);
  });

  it("搜索同时匹配标题和正文", () => {
    const docs = [
      doc({ id: "a", title: "第一卷", text: "无关" }),
      doc({ id: "b", title: "无关", text: "青石村的黄昏" }),
      doc({ id: "c", title: "无关", text: "无关" }),
    ];
    const got = groupDocuments([], docs, "青石村").find((g) => g.id === null)!;
    expect(got.screenplays.map((d) => d.id)).toEqual(["b"]);
  });

  /**
   * **归属指向一个不存在的项目时，稿子不能人间蒸发。**
   *
   * 正常流程里删项目会解开归属，所以这不该发生。但「不该发生」的事真发生时
   * （删到一半失败、数据被外部改过），后果是用户的稿子从界面上消失，
   * 而且不报错 —— 宁可显示在「未归档」里也不能不显示。
   */
  it("归属的项目不存在时落到未归档，不会凭空消失", () => {
    const groups = groupDocuments([], [doc({ id: "x", project_id: "gone" })], "");
    const loose = groups.find((g) => g.id === null)!;
    expect(loose.screenplays.map((d) => d.id)).toEqual(["x"]);
  });

  /**
   * **改编方案要和原文、剧本分开归组。**
   *
   * 它是原文和剧本之间那一层 —— 混进剧本里的话，下游（抽实体、生成分镜）
   * 会把它当剧本用，而它没有台词也没有分场，生成出来是一堆空镜头。
   */
  it("改编方案单独一组，不混进剧本", () => {
    const docs = [
      { id: "s", title: "原著", text: "原", kind: "source", project_id: "p1" },
      { id: "m", title: "方案", text: "方", kind: "plan", project_id: "p1" },
      { id: "e", title: "第 1 集", text: "本", kind: "screenplay", project_id: "p1" },
    ] as never;
    const g = groupDocuments([{ id: "p1", title: "余波", note: "" } as never], docs, "")[0];
    expect(g.sources.map((d) => d.id)).toEqual(["s"]);
    expect(g.plans.map((d) => d.id)).toEqual(["m"]);
    expect(g.screenplays.map((d) => d.id)).toEqual(["e"]);
  });

  /** 老数据没有 `kind`，**默认是剧本** —— 存量文档就是当剧本用的。 */
  it("没有 kind 的老文档算剧本", () => {
    const docs = [{ id: "x", title: "旧", text: "t" }] as never;
    const g = groupDocuments([], docs, "")[0];
    expect(g.screenplays.map((d) => d.id)).toEqual(["x"]);
    expect(g.plans).toEqual([]);
  });
});
