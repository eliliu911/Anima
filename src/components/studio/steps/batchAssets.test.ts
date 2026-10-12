import { describe, expect, it } from "vitest";
import {
  BATCH_GRID,
  batchSummary,
  characterTemplateByBody,
  characterTemplateFor,
  countMissing,
  defaultViews,
  missingCovers,
  needsImage,
  redoSummary,
  templateAspect,
} from "./batchAssets";
import { templateById, type TemplateId } from "../assets/assetPrompt";
import type { AssetView } from "../../../api/assets";

const a = (id: string, kind: string, cover: string | null): AssetView =>
  ({ id, name: id, kind, cover_url: cover }) as AssetView;

const 一屋子 = [
  a("林婉清", "character", null),
  a("皇帝", "character", "/p/1.png"),
  a("大臣", "character", null),
  a("金銮殿", "scene", null),
  a("长安城", "scene", "/p/2.png"),
  a("玉笏", "prop", null),
];

describe("还差哪些", () => {
  it("判据是封面，不是有没有变体", () => {
    expect(missingCovers(一屋子).map((x) => x.id)).toEqual([
      "林婉清", "大臣", "金銮殿", "玉笏",
    ]);
  });

  it("逐类报数 —— 「还差 4 个」不告诉人该去哪儿找", () => {
    expect(countMissing(一屋子)).toEqual({ character: 2, scene: 1, prop: 1 });
  });

  it("全都有图时一个都不报", () => {
    const 齐了 = 一屋子.map((x) => ({ ...x, cover_url: "/p/x.png" }));
    expect(missingCovers(齐了)).toEqual([]);
    expect(batchSummary(齐了, "zimage")).toBe("");
  });
});

describe("每个资产怎么出", () => {
  /**
   * **中文四视图只有 Z-Image 撑得住** —— 换引擎出来的是一张普通立绘，
   * 而且不报错。退回的版式必须是任何引擎都能用的那个。
   */
  it("引擎不对时角色退回英文版式", () => {
    expect(characterTemplateFor("zimage")).toBe("quadzh");
    expect(characterTemplateFor("sdxl")).toBe("simple");
    expect(characterTemplateFor("krea2")).toBe("simple");
  });

  it("退回之后要在那句话里说出来 —— 静默换版式的话人以为拿到的是四视图", () => {
    const zh = batchSummary(一屋子, "zimage");
    const en = batchSummary(一屋子, "sdxl");
    expect(zh).toContain("四视图（中文）");
    expect(en).toContain("简洁三视图");
    expect(en).not.toContain("四视图（中文）");
  });

  it("那句话逐类说，还说清楚场景出几格", () => {
    const s = batchSummary(一屋子, "zimage");
    expect(s).toContain("2 个角色");
    expect(s).toContain("1 个场景");
    expect(s).toContain("1 个道具");
    expect(s).toContain(`${BATCH_GRID} 格`);
  });

  it("没有角色时不提版式 —— 那句话只说要做的事", () => {
    const 只缺场景 = [a("金銮殿", "scene", null)];
    const s = batchSummary(只缺场景, "sdxl");
    expect(s).not.toContain("简洁三视图");
    expect(s).toContain("1 个场景");
  });
});

describe("没有空间契约的场景", () => {
  /**
   * 没有锚点时机位描述退化成笼统说法，四格区分度差一截 ——
   * 而图照出、任务成功、不报错。必须说出来。
   */
  it("点名说出来，但不阻断", () => {
    const s = batchSummary([a("金銮殿", "scene", null)], "zimage");
    expect(s).toContain("金銮殿");
    expect(s).toContain("空间契约");
    expect(s).toContain("1 个场景");
  });

  it("有锚点的场景不唠叨", () => {
    const 有契约 = {
      ...a("金銮殿", "scene", null),
      scene: { anchors: [{ name: "龙椅", size: "large" }] },
    } as unknown as AssetView;
    const s = batchSummary([有契约], "zimage");
    expect(s).not.toContain("空间契约");
  });

  it("已经有图的场景不算进来 —— 它不在这一批里", () => {
    const s = batchSummary([a("长安城", "scene", "/p/1.png")], "zimage");
    expect(s).toBe("");
  });
});

describe("默认机位", () => {
  it("四格互不相同", () => {
    const v = defaultViews();
    expect(v).toHaveLength(4);
    expect(new Set(v.map((x) => x.termId)).size).toBe(4);
    expect(v.every((x) => x.termId)).toBe(true);
  });
});

/**
 * **一批出砸之后必须还能重来。**
 *
 * 判据只有「有没有封面」的话，十一张废图也算「有图」，横幅撤掉，
 * 没有任何地方能重出 —— 用户 2026-09-12 当场撞上。
 */
describe("全部重出", () => {
  const 全有图 = 一屋子.map((x) => ({ ...x, cover_url: "/p/x.png" }));

  it("全都有图时，缺图那条横幅不出现", () => {
    expect(batchSummary(全有图, "zimage")).toBe("");
  });

  it("但重出那条要出现", () => {
    const s = redoSummary(全有图);
    expect(s).toContain("6 个");
    // **必须说清楚旧的留着** —— 不说的话这个按钮读起来像会毁掉已有的图
    expect(s).toContain("旧的那版留在变体里");
  });

  it("还有缺图的时候不显示重出 —— 两条横幅同时出现是噪音", () => {
    expect(redoSummary(一屋子)).toBe("");
  });

  it("一个资产都没有时两条都不显示", () => {
    expect(redoSummary([])).toBe("");
    expect(batchSummary([], "zimage")).toBe("");
  });
});

describe("角色按形象算缺图", () => {
  const 有形象 = {
    ...a("林婉清", "character", "/p/main.png"),
    look_views: [
      { id: "main", cover_url: "/p/main.png" },
      { id: "look_1", cover_url: null },
      { id: "look_2", cover_url: null },
    ],
  } as unknown as AssetView;

  it("主形象有图、另外两个形象没图 → 差 2 张（后台那批会出它们）", () => {
    expect(countMissing([有形象])).toEqual({ character: 2, scene: 0, prop: 0 });
    expect(batchSummary([有形象], "zimage")).toContain("2 个角色形象");
  });

  it("还有形象缺图时不显示「全部重出」", () => {
    expect(redoSummary([有形象])).toBe("");
  });
});

describe("从不出镜的角色不出图", () => {
  const 无身体 = (id: string): AssetView =>
    ({ id, name: id, kind: "character", cover_url: null, character: { disembodied: true } }) as never as AssetView;
  const 有身体 = (id: string): AssetView =>
    ({ id, name: id, kind: "character", cover_url: null, character: { disembodied: false } }) as never as AssetView;

  /**
   * 2026-09-19 用户在 E05 上撞到：小青、小橙标了「从不出镜」，
   * 横幅照样说「还有 4 个角色形象没有设定图」，而「一键全出」
   * 会真的给它们排两次生成 —— 出来的图没有任何地方会用。
   */
  it("**不算进「还差几张」**", () => {
    const items = [无身体("小青"), 无身体("小橙"), 有身体("陈先生"), 有身体("王小姐")];
    expect(countMissing(items).character).toBe(2);
    expect(missingCovers(items).map((x) => x.id)).toEqual(["陈先生", "王小姐"]);
  });

  it("全都是从不出镜时，横幅一个字都不该说", () => {
    expect(batchSummary([无身体("小青"), 无身体("小橙")], "zimage")).toBe("");
  });

  it("场景和道具不受影响 —— 它们没有这一位", () => {
    const s = ({ id: "咖啡馆", name: "咖啡馆", kind: "scene", cover_url: null }) as AssetView;
    expect(needsImage(s)).toBe(true);
    expect(countMissing([s]).scene).toBe(1);
  });

  it("有身体的角色照常算", () => {
    expect(needsImage(有身体("陈先生"))).toBe(true);
    expect(needsImage(无身体("小青"))).toBe(false);
  });
});

/**
 * **没有身体的角色不能拿到全身三视图。**
 *
 * 2026-09-19 实跑：小青的描述是「蓝色卡通 AI 助理头像」，而工作台
 * 一打开就是「简洁三视图」，构图段里写着
 * 「Right half: three equally-sized full-body standing poses」——
 * 光这个构图就足以把任何描述拉成一个人，和描述写什么无关。
 */
describe("按描述挑角色版式", () => {
  it("描述说是头像就走头像版式", () => {
    expect(characterTemplateByBody("zimage", "描述：蓝色卡通 AI 助理头像。")).toBe("avatar");
    expect(characterTemplateByBody("sdxl", "一团没有轮廓的蓝色光")).toBe("avatar");
    expect(characterTemplateByBody("zimage", "手机屏幕上的 icon")).toBe("avatar");
  });

  /**
   * **认不出的一律走原版式。** 第一版反过来列「有身体」的关键词 ——
   * 「唐装」不在表里，一个穿唐装的人被判成了头像。
   */
  it("认不出无实体信号的不动", () => {
    for (const d of ["描述：盛唐女官，女性，28岁", "穿唐装的中年男人", "一只橘猫", ""]) {
      expect(characterTemplateByBody("zimage", d), `误判成头像：${d}`).not.toBe("avatar");
    }
  });

  /** 头像版式出方图 —— 横幅会在两侧留一大片空。 */
  it("头像版式是方图", () => {
    expect(templateAspect("avatar")).toBe("square");
  });

  /**
   * **认不出的 id 不能回落成头像。**
   * `avatar` 现在排在清单第一个，而回落写的是「数组第一个」的话，
   * 任何一个拼错的 id 都会让真人角色变成一个图标,而且不报错。
   */
  it("认不出的版式回落到三视图，不是头像", () => {
    expect(templateById("nope" as TemplateId).id).toBe("simple");
  });
});
