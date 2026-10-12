import { describe, it, expect } from "vitest";
import { episodeNumberIn, seasonEpisode, episodeLabel, nextEpisodeName } from "./episodeLabel";
import type { ScriptDocument } from "../../../api/scriptLibrary";

const d = (title: string): ScriptDocument => ({ title }) as ScriptDocument;

describe("分集编号", () => {
  describe("从标题里读集号", () => {
    it("认中文写法", () => {
      expect(episodeNumberIn("第 1 集")).toBe(1);
      expect(episodeNumberIn("第12集")).toBe(12);
      expect(episodeNumberIn("余波 第 3 集")).toBe(3);
    });

    it("认 S01E02 和 EP03", () => {
      expect(episodeNumberIn("S01E02")).toBe(2);
      expect(episodeNumberIn("EP03")).toBe(3);
      expect(episodeNumberIn("E7 收网")).toBe(7);
    });

    /** **读不出来就返回 null，不猜。** 猜出来的编号会把顺序排错。 */
    it("读不出来不猜", () => {
      expect(episodeNumberIn("余波")).toBeNull();
      expect(episodeNumberIn("")).toBeNull();
      expect(episodeNumberIn("第 0 集")).toBeNull();
    });
  });

  /**
   * **季恒写 S01，不省掉。**
   *
   * 省掉的话以后加季时所有已有编号都要改写，而 `E01` 和 `S02E01` 并排
   * 时也看不出前者是哪一季。
   */
  it("编号写成 S01E02", () => {
    expect(seasonEpisode(2)).toBe("S01E02");
    expect(seasonEpisode(13)).toBe("S01E13");
    expect(seasonEpisode(1, 2)).toBe("S02E01");
  });

  describe("一行显示成什么", () => {
    /** 编号是结构、标题是内容，**分开返回**才能在界面上对齐成一列。 */
    it("编号和标题分开", () => {
      const l = episodeLabel(d("第 1 集 收网"), 0);
      expect(l.code).toBe("S01E01");
      expect(l.name).toBe("收网");
      expect(l.inferred).toBe(false);
    });

    /**
     * 标题里那段集号要去掉 —— 留着的话行首是「S01E01 第 1 集」，
     * 同一件事说两遍。
     */
    it("标题里的集号不重复显示", () => {
      expect(episodeLabel(d("第 1 集"), 0).name).toBe("");
      expect(episodeLabel(d("S01E02"), 5).name).toBe("");
      expect(episodeLabel(d("EP03 · 雨夜"), 9).name).toBe("雨夜");
    });

    /**
     * 标题里没写集号就**按位置补**，并标成 `inferred` ——
     * 这样任何一份剧本都有编号，而不是「有的有有的没有」；
     * 但补出来的要让人看得出是补的。
     */
    it("没写集号就按位置补，并且标出来", () => {
      const l = episodeLabel(d("余波"), 2);
      expect(l.code).toBe("S01E03");
      expect(l.name).toBe("余波");
      expect(l.inferred).toBe(true);
    });

    it("标题全是集号时名字为空，调用方自己兜", () => {
      expect(episodeLabel(d("  第 4 集  "), 0).name).toBe("");
    });
  });

  describe("新增一集叫什么", () => {
    /**
     * 接着**已有的最大集号**往下排，不是按数量 ——
     * 删掉第 2 集之后再加应该是第 4 集，叫第 3 集会和还在的那一集重名。
     */
    it("接最大集号往下排", () => {
      expect(nextEpisodeName([d("第 1 集"), d("第 3 集")])).toBe("S01E04");
    });

    it("空的从第一集开始", () => {
      expect(nextEpisodeName([])).toBe("S01E01");
    });

    it("没按集号命名的不参与排号", () => {
      expect(nextEpisodeName([d("随便起的名字")])).toBe("S01E01");
    });

    it("混着中文和 S01E 写法也算得对", () => {
      expect(nextEpisodeName([d("第 2 集"), d("S01E05")])).toBe("S01E06");
    });
  });
});
