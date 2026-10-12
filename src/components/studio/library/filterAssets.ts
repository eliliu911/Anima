/**
 * 资产库的筛选 / 排序 / 分组。
 *
 * 抽成纯函数是为了能测 —— 「页面显示得对不对」全在这里，
 * 而它一旦错了是**安静的**：少一个资产不会报错，只是列表里没有。
 */
import type { AssetKind, AssetView } from "../../../api/assets";

export type TypeFilter = AssetKind | "all";
export type ViewAxis = "type" | "source";
export type SortMode = "default" | "name" | "recent" | "usage";

export interface Group {
  key: string;
  title: string;
  items: AssetView[];
}

export interface Counts {
  all: number;
  character: number;
  scene: number;
  prop: number;
  starred: number;
}

export function countAssets(all: AssetView[]): Counts {
  const c: Counts = { all: all.length, character: 0, scene: 0, prop: 0, starred: 0 };
  for (const a of all) {
    c[a.kind] += 1;
    if (a.starred) c.starred += 1;
  }
  return c;
}

/**
 * 「最近」用的时间戳：**变体池里最新的一张**，不是建资产的时间。
 *
 * 一个老角色刚出了新图就该排最前面 —— 按 `created_at` 排的话它永远沉底，
 * 而那正是用户此刻最想找的东西。
 */
export function recencyOf(a: AssetView): number {
  let max = a.updated_at;
  for (const v of a.unit.image_variants) if (v.created_at > max) max = v.created_at;
  for (const v of a.unit.video_variants) if (v.created_at > max) max = v.created_at;
  return max;
}

/** 项目筛选：全部 / 只看全局 / 某个项目（含全局库，项目里用得到全局资产） */
export type ProjectFilter = "all" | "global" | { project_id: string };

/** 项目下拉的选项：出现过的项目，按名字排 */
export function projectOptions(all: AssetView[]): { id: string; title: string }[] {
  const map = new Map<string, string>();
  for (const a of all) {
    if (a.scope.kind === "global") continue;
    map.set(a.scope.project_id, a.project_title || a.scope.project_id);
  }
  return [...map.entries()].map(([id, title]) => ({ id, title })).sort((a, b) => a.title.localeCompare(b.title, "zh"));
}

export function matchesProject(a: AssetView, filter: ProjectFilter): boolean {
  if (filter === "all") return true;
  if (filter === "global") return a.scope.kind === "global";
  return a.scope.kind === "global" || a.scope.project_id === filter.project_id;
}

/** 搜索也搜形象名和形象描述 —— 「礼服」要能找到有礼服形象的角色 */
function matchesQuery(a: AssetView, q: string): boolean {
  if (a.name.toLowerCase().includes(q) || a.description.toLowerCase().includes(q)) return true;
  return (a.look_views ?? []).some((l) => l.name.toLowerCase().includes(q) || l.description.toLowerCase().includes(q));
}

export function filterAndSort(
  all: AssetView[],
  opts: { typeFilter: TypeFilter; starredOnly: boolean; query: string; sort: SortMode; project?: ProjectFilter },
): AssetView[] {
  const q = opts.query.trim().toLowerCase();
  const list = all.filter((a) => {
    if (opts.typeFilter !== "all" && a.kind !== opts.typeFilter) return false;
    if (opts.project && !matchesProject(a, opts.project)) return false;
    if (opts.starredOnly && !a.starred) return false;
    if (!q) return true;
    return matchesQuery(a, q);
  });
  if (opts.sort === "name") {
    return [...list].sort((a, b) => a.name.localeCompare(b.name, "zh"));
  }
  if (opts.sort === "recent") {
    return [...list].sort((a, b) => recencyOf(b) - recencyOf(a));
  }
  if (opts.sort === "usage") {
    return [...list].sort((a, b) => b.reference_count - a.reference_count);
  }
  return list;
}

const KIND_ORDER: AssetKind[] = ["character", "scene", "prop"];
const KIND_TITLE: Record<AssetKind, string> = {
  character: "角色",
  scene: "场景",
  prop: "道具",
};

/**
 * 同一批数据，两种分组轴。
 *
 * 「按类型」= 我要找一个角色；「按项目」= 我要看这部剧有什么。
 * **分组不能丢资产** —— 两个轴分出来的条目总数必须等于输入长度。
 */
export function groupAssets(list: AssetView[], axis: ViewAxis): Group[] {
  const map = new Map<string, Group>();
  for (const a of list) {
    // **按项目 id 分，不按归属标签分。** 以前按标签：两个项目的分集都叫 ep_01，
    // 两部剧的资产被合进同一组「分集 ep_01」（2026-09-15 用户）。同一个项目的剧集级、分集级资产放一组。
    const projectId = a.scope.kind === "global" ? null : a.scope.project_id;
    const key = axis === "type" ? a.kind : projectId ? `project:${projectId}` : "global";
    const title = axis === "type" ? KIND_TITLE[a.kind] : projectId ? a.project_title || projectId : "全局";
    let g = map.get(key);
    if (!g) {
      g = { key, title, items: [] };
      map.set(key, g);
    }
    g.items.push(a);
  }
  const out = [...map.values()];
  if (axis === "type") {
    out.sort((a, b) => KIND_ORDER.indexOf(a.key as AssetKind) - KIND_ORDER.indexOf(b.key as AssetKind));
  } else {
    // 项目按名字排，全局放最后
    out.sort((a, b) => (a.key === "global" ? 1 : 0) - (b.key === "global" ? 1 : 0) || a.title.localeCompare(b.title, "zh"));
  }
  return out;
}

/** 资产库一页放几个。卡片最窄 150px，宽屏一行 6~8 个，48 个大约六七行。 */
export const PAGE_SIZE = 48;

/**
 * 分页前的排序：「全部」视图下先按类型（角色 → 场景 → 道具）排好，类型内保持原来的顺序。
 *
 * 不先排的话，按筛选顺序切页，第 1 页可能是 20 个角色、3 个道具、25 个场景，
 * 分段标题跟着页码乱跳 —— 翻页时像东西在重新洗牌。
 */
export function orderForPaging(list: AssetView[], typeFilter: TypeFilter): AssetView[] {
  if (typeFilter !== "all") return list;
  return list
    .map((a, i) => ({ a, i }))
    .sort((x, y) => KIND_ORDER.indexOf(x.a.kind) - KIND_ORDER.indexOf(y.a.kind) || x.i - y.i)
    .map((x) => x.a);
}

/** 取第 `page` 页（从 1 开始）。页码超出范围时夹到最后一页 —— 删掉资产后不会停在一个空页上。 */
export function paginate<T>(list: T[], page: number, size = PAGE_SIZE): { items: T[]; page: number; pages: number } {
  const pages = Math.max(1, Math.ceil(list.length / size));
  const p = Math.min(Math.max(1, page), pages);
  return { items: list.slice((p - 1) * size, p * size), page: p, pages };
}
