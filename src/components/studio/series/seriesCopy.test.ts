import { describe, expect, it } from "vitest";
import { adoptAllNote, adoptNote, characterBase, entityKey, importNote, isDesignDoc, nextPick, seriesCount } from "./seriesCopy";

describe("提取弹窗的去向", () => {
  it("点一下依次是 进系列 → 只留本集 → 不要 → 进系列", () => {
    expect(nextPick("series")).toBe("episode");
    expect(nextPick("episode")).toBe("skip");
    expect(nextPick("skip")).toBe("series");
  });

  it("系列页没有「只留本集」—— 那里不属于任何一集", () => {
    expect(nextPick("series", false)).toBe("skip");
    expect(nextPick("skip", false)).toBe("series");
  });

  it("设定文档默认勾人物 / 场景 / 道具，不勾大纲和章节", () => {
    expect(isDesignDoc("02-人物小传")).toBe(true);
    expect(isDesignDoc("05-场景设计")).toBe(true);
    expect(isDesignDoc("06-道具设计")).toBe(true);
    expect(isDesignDoc("01-世界观设定")).toBe(true);
    expect(isDesignDoc("04-分集大纲")).toBe(false);
    expect(isDesignDoc("第03章-前女友的婚车")).toBe(false);
  });

  it("键和后端一样是 类型/名字，名字去掉首尾空白", () => {
    expect(entityKey({ kind: "character", name: " 陆川 " })).toBe("character/陆川");
  });
});

describe("系列数量对比", () => {
  const existing = new Set(["character/陆川", "character/星澜", "scene/巷口"]);
  const es = [
    { kind: "character", name: "陆川" },
    { kind: "character", name: "夏予安" },
    { kind: "character", name: "路人甲" },
    { kind: "scene", name: "巷口" },
  ];

  it("已有的并进去不加一，只留本集的不算", () => {
    const picks = {
      "character/陆川": "series",
      "character/夏予安": "series",
      "character/路人甲": "episode",
    } as const;
    expect(seriesCount(es, picks, existing, "character")).toEqual({ before: 2, after: 3 });
  });

  it("同一个人的几套造型算一个角色", () => {
    const looks = [
      { kind: "character", name: "夏予安（特勤制服）", persona: "夏予安" },
      { kind: "character", name: "夏予安（便装）", persona: "夏予安" },
      { kind: "character", name: "XN-0001", persona: "星澜" },
    ];
    const picks = Object.fromEntries(looks.map((e) => [entityKey(e), "series" as const]));
    expect(seriesCount(looks, picks, existing, "character")).toEqual({ before: 2, after: 4 });
  });

  it("不要的不算", () => {
    const picks = { "character/夏予安": "skip" } as const;
    expect(seriesCount(es, picks, existing, "character")).toEqual({ before: 2, after: 2 });
  });
});

describe("造型拆名", () => {
  it("和后端同一条规则", () => {
    expect(characterBase({ name: "星澜（战斗款）", persona: "星澜" })).toBe("星澜");
    expect(characterBase({ name: "陆川(上城伪装)", persona: "陆川" })).toBe("陆川");
    expect(characterBase({ name: "XN-0001", persona: "星澜" })).toBe("XN-0001");
    expect(characterBase({ name: "星澜（）", persona: "星澜" })).toBe("星澜（）");
    expect(characterBase({ name: "路人甲" })).toBe("路人甲");
  });
});

describe("文案", () => {
  it("新建和并入分开说", () => {
    expect(importNote({ added: 3, merged: 4, episode_only: 1 })).toBe(
      "系列里新建 3 个，4 个系列里已有，并进去了，1 个只留本集。去「角色与资产」能看到；提示词里用 @ 引用它们",
    );
    expect(importNote({ added: 0, merged: 0, episode_only: 0 })).toBe("没有改动");
    expect(importNote({ added: 1, merged: 0, episode_only: 0, looks: 2 })).toBe(
      "系列里新建 1 个，同一个人的其它造型加成了 2 个形象。去「角色与资产」能看到；提示词里用 @ 引用它们",
    );
  });

  it("单个收进系列：搬家和并入说法不同", () => {
    expect(adoptNote({ kind: "moved", id: "c1", name: "陆川" })).toBe("「陆川」已进系列");
    expect(adoptNote({ kind: "absorbed", from_id: "a", into_id: "b", name: "星澜", shots: 2 })).toBe(
      "「星澜」系列里已有，并进去了（改了 2 镜的引用）",
    );
  });

  it("全部收进系列", () => {
    expect(adoptAllNote({ moved: 5, absorbed: 2, kept: 1 })).toBe(
      "5 个进了系列，2 个同名的并进了已有的。改之前的数据备份在 backups/series-*",
    );
    expect(adoptAllNote({ moved: 0, absorbed: 0, kept: 3 })).toBe("没有待归属的");
  });
});
