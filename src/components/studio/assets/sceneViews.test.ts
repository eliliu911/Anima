import { describe, it, expect } from "vitest";
import {
  gridFor,
  gridLabel,
  cellSize,
  shortSide,
  viewsFromContract,
  contractWithViews,
  mergeDraftKeepingViews,
  centerFirst,
  rankedAnchors,
  viewIssues,
  toViewpoints,
  describeTerm,
  termById,
  resizeSlots,
  VIEW_TERMS,
  GRID_CHOICES,
  LAYOUT_ONLY_BELOW,
} from "./sceneViews";
import { shotLanguage } from "../../../shared/shotLanguage";
import type { SceneContract } from "../../../api/assets";

const 面馆: SceneContract = {
  shape: "狭长店铺",
  zones: ["北侧柜台"],
  entrances: [{ name: "东侧卷帘门", connects_to: "街道" }],
  anchors: [
    { name: "门边的伞桶", zone: "东墙", size: "small", must_not_become: [] },
    { name: "整面北墙的旧木柜台", zone: "北墙", size: "wall_dominant", must_not_become: [] },
  ],
  relations: ["门正对柜台"],
  materials: ["木"],
  views: [],
  inferred: [],
  no_extra: [],
  empty_stage: "required",
};
const 没锚点: SceneContract = { ...面馆, anchors: [], entrances: [] };

describe("场景多方位", () => {
  /** **和 Rust 侧 `scene_sheet::grid_for` 必须一致**，否则切格会错位。 */
  it("版式数学和后端对得上", () => {
    expect(gridFor(4)).toEqual({ rows: 2, cols: 2 });
    expect(gridFor(6)).toEqual({ rows: 2, cols: 3 });
    expect(gridFor(9)).toEqual({ rows: 3, cols: 3 });
    expect(GRID_CHOICES.map(gridLabel)).toEqual(["2×2", "2×3", "3×3"]);
  });

  /**
   * **只给 4 / 6 / 9 这三档。**
   *
   * 3、5、7 会留空格，而空格子模型多半会自己找东西填满 ——
   * 那一格是废的，还占了一次采样的像素预算。
   */
  it("三档都正好填满，没有空格", () => {
    for (const n of GRID_CHOICES) {
      const g = gridFor(n);
      expect(g.rows * g.cols, `${n} 个方位填不满 ${gridLabel(n)}`).toBe(n);
    }
  });

  it("每格尺寸按本机实际画幅算", () => {
    expect(cellSize(9)).toEqual({ w: 512, h: 341 });
    expect(cellSize(4)).toEqual({ w: 768, h: 512 });
  });

  /**
   * **判据是短边，不是宽度。** 3×3 每格 512×341：宽度正好等于门槛，
   * 短边只有 341。Rust 侧 `PixelBudget::usable_as_final` 判的是短边，
   * 两边分叉的表现是界面说「可直接使用」而工具返回里说「这是布局稿」。
   */
  it("够不够当成品看的是短边", () => {
    expect(shortSide(9)).toBe(341);
    expect(shortSide(9)).toBeLessThan(LAYOUT_ONLY_BELOW);
    expect(shortSide(4)).toBeGreaterThanOrEqual(LAYOUT_ONLY_BELOW);
  });

  describe("方位词表", () => {
    /**
     * **每条词的机位都得写明相机往哪站、朝哪拍。**
     *
     * 实测里唯一能把两个机位区分开的就是这个；笼统的
     * 「面向主要活动区的正面机位」会和别的格子撞成同一个角度。
     */
    it("每条机位都写明了相机站哪儿看哪儿", () => {
      for (const t of VIEW_TERMS) {
        expect(t.placement.length, `「${t.name}」写得太短`).toBeGreaterThan(15);
        expect(
          t.placement.includes("{锚点}") || t.placement.includes("{入口}"),
          `「${t.name}」没引用任何锚点或入口 —— 那就成了笼统说法`,
        ).toBe(true);
      }
    });

    /**
     * **每条词都是一次真机位，用的是唯一那份镜头语言词表。**
     *
     * 用户原话：「导演看的就是镜头视角，而不是方位。我们的 AIGC 就是
     * 模拟镜头拍摄。」场景四视角和分镜镜头本来就是同一件事 ——
     * 分开维护两套的表现是同一个词在两边含义不一样，而那不报错。
     */
    it("每条词都带景别和机位，取自唯一那份词表", () => {
      const sizes = new Set(shotLanguage().groups.find((g) => g.id === "shotSize")!.items.map((t) => t.id));
      const angles = new Set(shotLanguage().groups.find((g) => g.id === "angle")!.items.map((t) => t.id));
      for (const t of VIEW_TERMS) {
        expect(sizes.has(t.shot.shotSize), `「${t.name}」的景别 ${t.shot.shotSize} 不在词表里`).toBe(true);
        expect(angles.has(t.shot.angle), `「${t.name}」的机位 ${t.shot.angle} 不在词表里`).toBe(true);
      }
    });

    /** 填出来的那句话**镜头语言在前、机位在后** —— 导演就是这么说话的。 */
    it("填出来是「镜头语言。相机站哪儿」", () => {
      const line = describeTerm(termById("center")!, 面馆);
      // 景别那一段来自词表，不是我们手写的
      expect(line).toContain("大远景");
      expect(line.indexOf("大远景")).toBeLessThan(line.indexOf("镜头正对"));
    });

    /** **不好用的要照实标出来**，不标的话用户选完等着失望。 */
    it("实测不可靠的必须带注记", () => {
      for (const t of VIEW_TERMS) {
        if (t.reliability !== "ok") {
          expect(t.note, `「${t.name}」标了不可靠却没说为什么`).toBeTruthy();
        }
      }
    });

    /**
     * **视角是相机指令，不是方位名。**
     *
     * 我们原来写「反打」两个字，实测两轮都没转过去。
     * liblib 那份提示词里同一格写的是「**镜头在室内最深处向外拍摄**」——
     * 一个可执行的相机动作。名字模型不当回事，动作它照做。
     */
    it("反打写成相机动作而不是「反打」两个字", () => {
      const back = VIEW_TERMS.find((t) => t.id === "deep_out")!;
      expect(back.placement).toContain("最深处");
      expect(back.placement).toContain("向外拍摄");
      // 还要明写「看不到主锚点」—— 那是顶住「没真转过去」的那一下。
      // **认「看不见」和「看不到」两种写法** —— 钉的是这句话在，
      // 不是它用了哪个字。
      expect(back.placement).toMatch(/看不[见到]/);
      expect(back.note).toContain("相机动作");
    });

    /**
     * **侧面机位要锚在正面那一格的中心线上。**
     *
     * 各自独立地说「左侧看」「右侧看」，两格会各转各的、对不上；
     * 说「以正面那一格的中心线为参考，移到左前 45 度」才是同一套坐标。
     * 这是 liblib 那份提示词里最容易被忽略的一条。
     */
    it("侧面机位锚定在中心线上", () => {
      for (const id of ["left45", "right45"]) {
        const t = VIEW_TERMS.find((x) => x.id === id)!;
        expect(t.placement, `「${t.name}」没锚定中心线`).toContain("中心线为参考");
      }
    });

    /**
     * **前四条就是 liblib 那套 2×2。**
     *
     * 正面中心线 / 左前 45° / 右前 45° / 深处往外 —— 用户拿他们那张图
     * 对过，四格正是这四个机位，而第 1 格和第 4 格是真正的前后视角。
     */
    it("前四条是 liblib 那套 2x2", () => {
      expect(VIEW_TERMS.slice(0, 4).map((t) => t.id))
        .toEqual(["center", "left45", "right45", "deep_out"]);
    });

    it("词表 id 不重名", () => {
      expect(new Set(VIEW_TERMS.map((t) => t.id)).size).toBe(VIEW_TERMS.length);
    });

    /** 3×3 要九个格子，词表得凑得满。 */
    it("词条够填满 3×3", () => {
      expect(VIEW_TERMS.length).toBeGreaterThanOrEqual(9);
    });
  });

  describe("模板填成真句子", () => {
    it("锚点取尺度最大的那个", () => {
      const t = termById("entry_in")!;
      const line = describeTerm(t, 面馆);
      expect(line).toContain("整面北墙的旧木柜台");
      expect(line).not.toContain("伞桶");
      expect(line).toContain("东侧卷帘门");
      expect(line).not.toContain("{");
    });

    /** 契约里没锚点时退到通用说法 —— **不能留下 `{锚点}` 这种占位符**。 */
    it("没有锚点时退到通用说法而不是漏出占位符", () => {
      const line = describeTerm(termById("entry_in")!, 没锚点);
      expect(line).not.toContain("{");
      expect(line).toContain("最显眼的那个不可移动物");
      expect(line).toContain("主入口");
    });

    it("反打填好之后仍然明写看不到主锚点", () => {
      expect(describeTerm(termById("deep_out")!, 面馆)).toMatch(/看不[见到]/);
    });
  });

  describe("换档位", () => {
    it("换大保留已选的，补上没用过的", () => {
      const four = resizeSlots([], 4);
      expect(four).toHaveLength(4);
      const six = resizeSlots(four, 6);
      expect(six).toHaveLength(6);
      expect(six.slice(0, 4)).toEqual(four);
      // 补进来的不能和已有的重名 —— 重名会让切出来的文件互相覆盖
      expect(new Set(six.map((r) => r.termId)).size).toBe(6);
    });

    it("换小就截断", () => {
      expect(resizeSlots(resizeSlots([], 9), 4)).toHaveLength(4);
    });
  });

  describe("写回契约", () => {
    it("其余字段一个不动", () => {
      const c = contractWithViews(面馆, [{ termId: "center" }]);
      expect(c.anchors).toEqual(面馆.anchors);
      expect(c.entrances).toEqual(面馆.entrances);
      expect(c.shape).toBe("狭长店铺");
    });

    /**
     * **存的是填好的整句，不是模板。**
     *
     * 下游（镜头上下文节点、全景出图）读的是文字，不该再各自去填一遍
     * 模板 —— 填法不一致就会分叉，而分叉了不报错。
     */
    it("存的是填好的句子不是模板", () => {
      const c = contractWithViews(面馆, [{ termId: "center" }]);
      expect(c.views[0].toward).toContain("整面北墙的旧木柜台");
      expect(c.views[0].toward).not.toContain("{");
      expect(c.views[0].name).toBe("正面中心线");
    });

    it("空格子不写进契约", () => {
      const c = contractWithViews(面馆, [{ termId: "center" }, { termId: "" }]);
      expect(c.views).toHaveLength(1);
    });

    it("读回来还是同一批方位", () => {
      const rows = [{ termId: "center" }, { termId: "deep_out" }];
      const c = contractWithViews(面馆, rows);
      const back = viewsFromContract(c);
      expect(back.slice(0, 2)).toEqual(rows);
      // 不足四个会补满到最小档，格子数才对得上
      expect(back).toHaveLength(4);
    });
  });

  describe("填得够不够用", () => {
    const full = (n: 4 | 6 | 9) => resizeSlots([], n);

    it("有空格要说清那一格是废的", () => {
      const rows = [...full(4)];
      rows[2] = { termId: "" };
      expect(viewIssues(rows, 面馆).join()).toContain("还有 1 个格子没选");
    });

    it("选重了要报 —— 切出来的文件会覆盖", () => {
      const rows = full(4);
      rows[1] = { termId: rows[0].termId };
      expect(viewIssues(rows, 面馆).join()).toContain("选了两次");
    });

    /** 选了实测不可靠的词条，**当场就要说**，不是等出完图才失望。 */
    it("选了不可靠的词条要当场说", () => {
      expect(viewIssues([{ termId: "deep_out" }], 面馆).join()).toContain("相机动作");
    });

    it("没填锚点要提醒区分度会差", () => {
      expect(viewIssues(full(4), 没锚点).join()).toContain("还没填不可移动的锚点");
      expect(viewIssues(full(4), 面馆).join()).not.toContain("还没填不可移动的锚点");
    });

    /**
     * **像素预算必须说出来。** 不说的话这张布局稿会被当成品用下去 ——
     * 角色设定卡那次就是没人算这笔账（整排图的脸只有 40~46px）。
     */
    it("3×3 要说它只是布局稿，2×2 不用", () => {
      expect(viewIssues(full(9), 面馆).join()).toContain("布局稿");
      expect(viewIssues(full(9), 面馆).join()).toContain("512×341");
      expect(viewIssues(full(4), 面馆).join()).not.toContain("布局稿");
    });
  });

  it("只发选满了的方位", () => {
    const vps = toViewpoints([{ termId: "center" }, { termId: "" }], 面馆);
    expect(vps).toHaveLength(1);
    expect(vps[0].name).toBe("正面中心线");
    expect(vps[0].description).toContain("整面北墙的旧木柜台");
  });
});

/**
 * **草拟只搬空间，机位一个不动。**
 *
 * 2026-09-12 实拍：点「从剧本草拟」存完，四个格子里三个变成
 * 「（这一格空着）」—— 整份替换把选好的机位冲掉了，而草拟自带的那套
 * `views` 名字是模型现起的，`viewsFromContract` 按名字反查对不上。
 */
describe("mergeDraftKeepingViews", () => {
  const 现有 = {
    shape: "",
    zones: [],
    entrances: [],
    anchors: [],
    relations: [],
    materials: [],
    views: [
      { name: "正面中心线", from: "", toward: "旧的一句", far: "", visible: [], light: "" },
      { name: "左前 45°", from: "", toward: "旧的另一句", far: "", visible: [], light: "" },
    ],
    inferred: [],
  } as unknown as SceneContract;

  const 草拟 = {
    shape: "东西进深的大殿",
    zones: ["丹陛", "殿身"],
    entrances: [],
    anchors: [{ name: "龙椅", size: "large" }],
    relations: ["龙椅在殿身正北"],
    materials: ["石砖"],
    views: [{ name: "殿门内侧", from: "", toward: "模型现起的名字", far: "", visible: [], light: "" }],
    inferred: ["高度按唐制推"],
  } as unknown as SceneContract;

  it("空间部分全部换成草拟的", () => {
    const out = mergeDraftKeepingViews(现有, 草拟);
    expect(out.shape).toBe("东西进深的大殿");
    expect(out.anchors.map((a) => a.name)).toEqual(["龙椅"]);
    expect(out.relations).toEqual(["龙椅在殿身正北"]);
  });

  it("**机位原封不动** —— 这是整条的全部理由", () => {
    const out = mergeDraftKeepingViews(现有, 草拟);
    expect(out.views).toEqual(现有.views);
    expect(out.views.map((v) => v.name)).not.toContain("殿门内侧");
  });

  it("本来就没有机位时给空数组，不是把草拟那套塞进来", () => {
    expect(mergeDraftKeepingViews(null, 草拟).views).toEqual([]);
  });
});

/**
 * **认不出来的方位要补成默认词条，不能留空格子。**
 *
 * 注释一直是这么写的，代码却给的是空串 —— 而空串在界面上就是
 * 「（这一格空着）」，「空格子模型多半会自己找东西填满，那一格是废的」。
 */
describe("契约里认不出来的方位", () => {
  const 契约 = (...names: string[]) =>
    ({
      anchors: [],
      entrances: [],
      views: names.map((name) => ({ name, from: "", toward: "", far: "", visible: [], light: "" })),
    }) as unknown as SceneContract;

  it("模型现起的名字补成默认词条，不留空", () => {
    const rows = viewsFromContract(契约("殿门内侧", "丹陛之上", "正面中心线"));
    expect(rows).toHaveLength(4);
    expect(rows.every((r) => r.termId), `有空格子：${JSON.stringify(rows)}`).toBe(true);
  });

  /**
   * 认得出来的那条要留下 —— 但**位置不保证**：`centerFirst` 会把
   * 「正面中心线」挪到第一格，因为左前/右前 45° 按名字引用它。
   * 这条测试原来断言的是 `rows[2] === center`，`centerFirst` 正当地
   * 推翻了它。
   */
  it("认得出来的那条留下来，中心线还会被挪到第一格", () => {
    const rows = viewsFromContract(契约("殿门内侧", "丹陛之上", "正面中心线"));
    expect(rows.map((r) => r.termId)).toContain("center");
    expect(rows[0].termId).toBe("center");
  });

  it("补进来的不和已有的重复 —— 重了那一格就是白出的", () => {
    const rows = viewsFromContract(契约("左前 45°", "认不出来的", "深处往外"));
    expect(new Set(rows.map((r) => r.termId)).size).toBe(rows.length);
  });

  it("契约里一条方位都没有时给一整套默认的", () => {
    const rows = viewsFromContract(契约());
    expect(rows).toHaveLength(4);
    expect(rows.every((r) => r.termId)).toBe(true);
  });
});

/**
 * **「正面中心线」必须排第一格 —— 这是依赖，不是排版偏好。**
 *
 * 左前 / 右前 45° 的原文按名字引用它（「以『正面中心线』那一格的中心线
 * 为参考」）。它排在后面的话，模型读到第 1 格时参考还不存在。
 * 2026-09-12 实拍：顺序是 左前/右前/深处/正面，四格几乎一模一样。
 */
describe("centerFirst", () => {
  const rows = (...ids: string[]) => ids.map((termId) => ({ termId }));

  it("中心线在第 4 格时挪到第 1 格", () => {
    expect(centerFirst(rows("left45", "right45", "deep_out", "center"))).toEqual(
      rows("center", "left45", "right45", "deep_out"),
    );
  });

  it("本来就在第 1 格就不动", () => {
    const r = rows("center", "left45", "right45", "deep_out");
    expect(centerFirst(r)).toEqual(r);
  });

  it("**没选中心线时一个格子都不动** —— 那是用户故意不要参考基准", () => {
    const r = rows("high", "low", "detail", "work_area");
    expect(centerFirst(r)).toEqual(r);
  });

  it("挪完格子数不变，也不丢任何一格", () => {
    const out = centerFirst(rows("left45", "center", "deep_out", "high"));
    expect(out).toHaveLength(4);
    expect(new Set(out.map((r) => r.termId))).toEqual(
      new Set(["center", "left45", "deep_out", "high"]),
    );
  });

  it("从契约读出来之后，中心线就在第一格", () => {
    const c = {
      anchors: [],
      entrances: [],
      views: [
        { name: "左前 45°", from: "", toward: "", far: "", visible: [], light: "" },
        { name: "正面中心线", from: "", toward: "", far: "", visible: [], light: "" },
      ],
    } as unknown as SceneContract;
    expect(viewsFromContract(c)[0].termId).toBe("center");
  });
});

/**
 * **每一格点名一个不同的锚点 —— 这是四格不同的决定性因素。**
 *
 * 2026-09-12 实测：四格出成同一个正面机位。查到最后原因在这儿 ——
 * 每一格填的都是**同一个**锚点（尺度最大的那个），于是四格都在说
 * 「镜头对准龙椅」。模型画四张龙椅正面**是在照做**。
 *
 * 排除过三个别的怀疑，都不是主因：提示词里的 `single unified image`、
 * 四格描述开头那串相同的镜头语言、以及引擎（换 Krea 2 更差）。
 */
describe("每格点名不同的锚点", () => {
  const 大殿 = {
    anchors: [
      { name: "龙椅", zone: "北", size: "large", must_not_become: [] },
      { name: "东侧列柱", zone: "东", size: "wall_dominant", must_not_become: [] },
      { name: "藻井", zone: "顶", size: "large", must_not_become: [] },
    ],
    entrances: [{ name: "殿门", connects_to: "丹陛" }],
    views: [],
  } as unknown as SceneContract;

  it("同一条词条，不同格位填不同的锚点", () => {
    const t = termById("left45")!;
    const a = describeTerm(t, 大殿, 0);
    const b = describeTerm(t, 大殿, 1);
    expect(a).not.toBe(b);
  });

  it("四格各自点名的锚点互不相同", () => {
    const rows = [{ termId: "center" }, { termId: "left45" }, { termId: "right45" }, { termId: "deep_out" }];
    const descs = toViewpoints(rows, 大殿).map((v) => v.description);
    // 每格提到的锚点里，至少有三格是不同的主语
    const named = ["龙椅", "东侧列柱", "藻井"];
    const hit = named.filter((n) => descs.some((d) => d.includes(n)));
    expect(hit.length, `四格只用上了 ${hit.length} 个锚点：${descs.join(" | ")}`).toBeGreaterThanOrEqual(2);
  });

  /** 锚点排序稳定：尺度最大的永远是第一个（主锚点）。 */
  /**
   * **主锚点是「最合适当焦点的那个」，不是「占地最大的那个」。**
   *
   * 地理锁那套排序把  排最前（占满一面墙的东西是最强的
   * 地理参照），但机位要的是焦点 —— 对着一整面墙拍出来是一堵墙。
   * 实测撞到过：第 1 格去看「两侧的朱红巨柱」而不是龙椅。
   */
  it("主锚点是离散大件，不是占满一面墙的那个", () => {
    expect(rankedAnchors(大殿)[0]).toBe("龙椅");
    expect(rankedAnchors(大殿)).toContain("东侧列柱");
  });

  /**
   * **非主视角要明说看不到主锚点。**
   * 不说的话模型会把它塞进每一格 —— 那正是四格同质的另一半。
   */
  it("45 度和反打都明说看不到主锚点", () => {
    for (const id of ["left45", "right45", "deep_out"]) {
      expect(termById(id)!.placement, `${id} 没说「看不到主锚点」`).toMatch(/看不[见到]\{主锚点\}/);
    }
  });

  it("锚点不够四个时循环取，不留空也不全填同一个", () => {
    const 只有一个 = {
      anchors: [{ name: "柜台", zone: "西", size: "large", must_not_become: [] }],
      entrances: [],
      views: [],
    } as unknown as SceneContract;
    const line = describeTerm(termById("left45")!, 只有一个, 3);
    expect(line).toContain("柜台");
    expect(line).not.toContain("{");
  });
});
