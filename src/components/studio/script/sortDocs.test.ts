import { describe, it, expect } from "vitest";
import { naturalCompare, sortDocs, SORT_OPTIONS, DEFAULT_SORT } from "./sortDocs";
import type { ScriptDocument } from "../../../api/scriptLibrary";

const d = (title: string, text = "", updated = 0): ScriptDocument =>
  ({ title, text, updated_at: updated }) as ScriptDocument;

describe("剧本库排序", () => {
  describe("自然序", () => {
    /**
     * **字典序把「第10章」排在「第2章」前面**（因为 `1 < 2`）。
     * 章节和分集天然是数字序，而排错顺序的目录比不排序更糟 ——
     * 它**看起来**是排过的。
     */
    it("数字按数值比，不是按字符比", () => {
      const got = ["第10章", "第2章", "第1章"].sort(naturalCompare);
      expect(got).toEqual(["第1章", "第2章", "第10章"]);
    });

    it("前导零不影响", () => {
      expect(naturalCompare("第01章", "第1章")).toBe(0);
      const got = ["第06章-最后一条指令", "第01章-电量还剩多少", "第10章-x"].sort(naturalCompare);
      expect(got[0]).toContain("第01章");
      expect(got[2]).toContain("第10章");
    });

    it("S01E02 排在 S01E10 前面", () => {
      const got = ["S01E10", "S01E02", "S01E01"].sort(naturalCompare);
      expect(got).toEqual(["S01E01", "S01E02", "S01E10"]);
    });

    it("纯文字按中文排序", () => {
      expect(naturalCompare("阿", "祖")).toBeLessThan(0);
    });

    it("前缀短的排前面", () => {
      expect(naturalCompare("第1章", "第1章-补")).toBeLessThan(0);
    });

    it("空串不炸", () => {
      expect(naturalCompare("", "")).toBe(0);
      expect(naturalCompare("", "a")).toBeLessThan(0);
    });
  });

  describe("三种排法", () => {
    const docs = [
      d("第03章", "三".repeat(300), 300),
      d("第01章", "一".repeat(100), 100),
      d("第02章", "二".repeat(999), 200),
    ];

    it("按名称是自然序", () => {
      expect(sortDocs(docs, "name").map((x) => x.title)).toEqual(["第01章", "第02章", "第03章"]);
    });

    /** 「最近改动」看的是**最近**，所以新的在前。 */
    it("最近改动是新的在前", () => {
      expect(sortDocs(docs, "updated").map((x) => x.title)).toEqual(["第03章", "第02章", "第01章"]);
    });

    /** 「字数最多」看的是**最多**，所以长的在前。 */
    it("字数最多是长的在前", () => {
      expect(sortDocs(docs, "size").map((x) => x.title)).toEqual(["第02章", "第03章", "第01章"]);
    });

    /**
     * **不改原数组** —— 传进来的那份是 store 里的，
     * 原地排会让别处读到被搅乱的顺序。
     */
    it("不改原数组", () => {
      const before = docs.map((x) => x.title);
      sortDocs(docs, "size");
      expect(docs.map((x) => x.title)).toEqual(before);
    });

    /**
     * **同值时退回名称序，保证稳定。**
     * 不退的话两次渲染可能给出不同的排法，用户会看到列表自己在跳。
     */
    it("同值时退回名称序", () => {
      const tie = [d("乙", "x", 5), d("甲", "x", 5)];
      expect(sortDocs(tie, "updated").map((x) => x.title)).toEqual(["甲", "乙"]);
      expect(sortDocs(tie, "size").map((x) => x.title)).toEqual(["甲", "乙"]);
    });

    /** 存量数据没有 `updated_at`，读出来是 undefined —— 不能因此炸或乱序。 */
    it("没有时间戳的当 0，退回名称序", () => {
      const legacy = [{ title: "乙", text: "" }, { title: "甲", text: "" }] as ScriptDocument[];
      expect(sortDocs(legacy, "updated").map((x) => x.title)).toEqual(["甲", "乙"]);
    });
  });

  /** 默认按名称 —— 章节和分集本来就有顺序，那是最常想看的那个。 */
  it("默认按名称，三个选项都在", () => {
    expect(DEFAULT_SORT).toBe("name");
    expect(SORT_OPTIONS.map((o) => o.id)).toEqual(["name", "updated", "size"]);
    for (const o of SORT_OPTIONS) expect(o.label).toBeTruthy();
  });
});
