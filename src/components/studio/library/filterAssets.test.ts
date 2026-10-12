import { describe, expect, it } from "vitest";
import type { AssetKind, AssetView } from "../../../api/assets";
import { countAssets, filterAndSort, groupAssets, orderForPaging, paginate, projectOptions, recencyOf } from "./filterAssets";
import { shouldBlur } from "./nsfw";

function mk(over: Partial<AssetView> & { id: string; kind: AssetKind }): AssetView {
  return {
    name: over.id,
    description: "",
    starred: false,
    locked: false,
    created_at: 0,
    updated_at: 0,
    unit: {
      selected_image_id: null,
      image_variants: [],
      selected_video_id: null,
      video_variants: [],
      image_prompt: null,
      video_prompt: null,
    },
    character: null,
    scope: { kind: "global" },
    scope_label: "全局",
    cover_url: null,
    image_urls: [],
    video_urls: [],
    reference_count: 0,
    ...over,
  } as AssetView;
}

const 老茶师 = mk({ id: "c1", kind: "character", name: "老茶师", description: "白须", reference_count: 3 });
const 小徒弟 = mk({ id: "c2", kind: "character", name: "小徒弟", starred: true });
const 老茶馆 = mk({ id: "s1", kind: "scene", name: "老茶馆", scope: { kind: "project", project_id: "p1" }, scope_label: "茶馆", project_title: "茶馆" });
const 铜壶 = mk({ id: "p1", kind: "prop", name: "铜壶", reference_count: 1 });
const ALL = [老茶师, 小徒弟, 老茶馆, 铜壶];

const 默认 = { typeFilter: "all", starredOnly: false, query: "", sort: "default" } as const;

describe("计数", () => {
  it("分类型统计并单独数星标", () => {
    expect(countAssets(ALL)).toEqual({ all: 4, character: 2, scene: 1, prop: 1, starred: 1 });
  });
});

describe("筛选", () => {
  it("按类型筛", () => {
    const r = filterAndSort(ALL, { ...默认, typeFilter: "character" });
    expect(r.map((a) => a.id)).toEqual(["c1", "c2"]);
  });

  it("只看星标", () => {
    const r = filterAndSort(ALL, { ...默认, starredOnly: true });
    expect(r.map((a) => a.id)).toEqual(["c2"]);
  });

  it("搜索同时命中名字和描述", () => {
    expect(filterAndSort(ALL, { ...默认, query: "老茶" }).map((a) => a.id)).toEqual(["c1", "s1"]);
    // 「白须」只在描述里 —— 只搜名字的话这条会漏
    expect(filterAndSort(ALL, { ...默认, query: "白须" }).map((a) => a.id)).toEqual(["c1"]);
  });

  it("搜索忽略大小写和首尾空格", () => {
    const en = mk({ id: "x", kind: "prop", name: "Teapot" });
    expect(filterAndSort([en], { ...默认, query: "  teaPOT " }).map((a) => a.id)).toEqual(["x"]);
  });

  it("筛选和排序叠加时两个都生效", () => {
    const r = filterAndSort(ALL, { ...默认, typeFilter: "character", sort: "usage" });
    expect(r.map((a) => a.id)).toEqual(["c1", "c2"]);
  });
});

describe("排序", () => {
  it("默认不改变顺序", () => {
    expect(filterAndSort(ALL, 默认).map((a) => a.id)).toEqual(["c1", "c2", "s1", "p1"]);
  });

  it("按使用次数降序", () => {
    expect(filterAndSort(ALL, { ...默认, sort: "usage" }).map((a) => a.id)).toEqual([
      "c1", "p1", "c2", "s1",
    ]);
  });

  it("不修改传进来的数组", () => {
    const input = [...ALL];
    filterAndSort(input, { ...默认, sort: "name" });
    expect(input.map((a) => a.id)).toEqual(["c1", "c2", "s1", "p1"]);
  });

  it("「最近」看的是变体池里最新那张，不是建资产的时间", () => {
    const 老角色新图 = mk({
      id: "old",
      kind: "character",
      created_at: 1,
      updated_at: 1,
      unit: {
        selected_image_id: null,
        image_variants: [{ id: "v", rel_path: "", created_at: 9_000, prompt_used: null, favorited: false, from_upload: false }],
        selected_video_id: null,
        video_variants: [],
        image_prompt: null,
        video_prompt: null,
      },
    });
    const 新角色无图 = mk({ id: "new", kind: "character", created_at: 5_000, updated_at: 5_000 });
    expect(recencyOf(老角色新图)).toBe(9_000);
    const r = filterAndSort([新角色无图, 老角色新图], { ...默认, sort: "recent" });
    expect(r.map((a) => a.id)).toEqual(["old", "new"]);
  });
});

describe("分组", () => {
  it("按类型分组，顺序固定为 角色 → 场景 → 道具", () => {
    const g = groupAssets([铜壶, 老茶馆, 老茶师], "type");
    expect(g.map((x) => x.title)).toEqual(["角色", "场景", "道具"]);
  });

  it("按项目分组用项目名，全局放最后", () => {
    const g = groupAssets(ALL, "source");
    expect(g.map((x) => x.title)).toEqual(["茶馆", "全局"]);
  });

  /** 两个项目的分集都叫 ep_01：以前按标签分组，两部剧被合进同一组 */
  it("不同项目的同名分集不合并；同一项目的剧集级、分集级资产放一组", () => {
    const ep = (id: string, project_id: string, title: string) =>
      mk({ id, kind: "character", scope: { kind: "episode", project_id, episode_id: "ep_01" }, scope_label: `${title} · 第 1 集`, project_title: title });
    const list = [ep("a", "p1", "盛唐"), ep("b", "p2", "米小团"), mk({ id: "c", kind: "scene", scope: { kind: "project", project_id: "p1" }, scope_label: "盛唐", project_title: "盛唐" })];
    const g = groupAssets(list, "source");
    expect(g.map((x) => [x.title, x.items.map((i) => i.id)])).toEqual([["米小团", ["b"]], ["盛唐", ["a", "c"]]]);
  });

  it("分组不能丢资产", () => {
    // 分组写错最典型的样子就是安静地少几条 —— 不报错，列表里就是没有。
    for (const axis of ["type", "source"] as const) {
      const total = groupAssets(ALL, axis).reduce((n, g) => n + g.items.length, 0);
      expect(total).toBe(ALL.length);
    }
  });

  it("空输入分出零个组，不是一个空组", () => {
    expect(groupAssets([], "type")).toEqual([]);
  });
});

describe("项目筛选、形象搜索、NSFW", () => {
  const p1 = mk({ id: "a", kind: "character", name: "林婉清", scope: { kind: "project", project_id: "p1" }, project_title: "盛唐",
    look_views: [{ id: "main", name: "主形象", is_main: true, description: "", prompt: "", nsfw: false, unit: {} as never, cover_url: null, image_urls: [], image_paths: [] },
      { id: "look_1", name: "舞会礼服", is_main: false, description: "黑色晚礼服", prompt: "", nsfw: true, unit: {} as never, cover_url: null, image_urls: [], image_paths: [] }] });
  const p2 = mk({ id: "b", kind: "character", name: "小花", scope: { kind: "project", project_id: "p2" }, project_title: "米小团" });
  const g = mk({ id: "c", kind: "prop", name: "铜壶" });
  const list = [p1, p2, g];
  const run = (over: object) => filterAndSort(list, { ...默认, ...over }).map((a) => a.id);

  it("选了项目：这个项目的 + 全局的；只看全局：只有全局", () => {
    expect(run({ project: { project_id: "p1" } })).toEqual(["a", "c"]);
    expect(run({ project: "global" })).toEqual(["c"]);
    expect(run({ project: "all" })).toEqual(["a", "b", "c"]);
  });

  it("项目下拉按名字排、不含全局", () => {
    expect(projectOptions(list)).toEqual([{ id: "p2", title: "米小团" }, { id: "p1", title: "盛唐" }]);
  });

  it("搜形象名、形象描述也能找到角色", () => {
    expect(run({ query: "礼服" })).toEqual(["a"]);
    expect(run({ query: "晚礼服" })).toEqual(["a"]);
  });

  it("NSFW 默认模糊；总开关或单张点开才显示", () => {
    const none = { showAll: false, revealed: new Set<string>() };
    expect(shouldBlur(true, "look_1", none)).toBe(true);
    expect(shouldBlur(false, "look_1", none)).toBe(false);
    expect(shouldBlur(true, "look_1", { ...none, showAll: true })).toBe(false);
    expect(shouldBlur(true, "look_1", { ...none, revealed: new Set(["look_1"]) })).toBe(false);
  });
});

/** 用户 2026-09-26：「资产库默认全部项目里的内容做分页，不然后面内容多了会拉的很长」 */
describe("分页", () => {
  it("按页切，超出范围夹到最后一页（删了资产不会停在空页）", () => {
    const xs = Array.from({ length: 100 }, (_, i) => i);
    expect(paginate(xs, 1, 48)).toMatchObject({ page: 1, pages: 3 });
    expect(paginate(xs, 3, 48).items).toEqual([96, 97, 98, 99]);
    expect(paginate(xs, 9, 48).page).toBe(3);
    expect(paginate([], 1, 48)).toEqual({ items: [], page: 1, pages: 1 });
  });

  it("「全部」视图先按类型排，类型内顺序不变 —— 翻页时分段不乱跳", () => {
    const list = [mk({ id: "p1", kind: "prop" }), mk({ id: "c1", kind: "character" }), mk({ id: "s1", kind: "scene" }), mk({ id: "c2", kind: "character" })];
    expect(orderForPaging(list, "all").map((a) => a.id)).toEqual(["c1", "c2", "s1", "p1"]);
    expect(orderForPaging(list, "prop")).toBe(list);
  });
});
