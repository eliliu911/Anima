/**
 * 资产库 —— 角色 / 场景 / 道具的总览页。
 *
 * # 为什么它是一整页，不是一个抽屉
 *
 * 上一版把它做成了侧边抽屉，后端就是对 `.anima/generated/` 的一次
 * `read_dir`。那列的是**文件**，而分镜要引用的是**资产** ——
 * 文件没有 id，没有任何东西能指向它，于是它只能是只读画廊。
 *
 * 现在每个条目有 id、类型、归属、变体池、以及「被哪几个镜头引用」。
 * 那份「被引用」是整套体系的关键：没有它，删资产不报错、
 * 分镜静默失去参考图，表现是「生成的人突然不像了」。
 *
 * # 两个视图轴
 *
 * 「按类型」= 我要找一个角色；「按项目」= 我要看这部剧有什么。
 * 同一批数据两种分组，这是从 LumenX 抄的、代价最低收益最高的一处交互。
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  assetsList,
  sameScope,
  type AssetKind,
  type AssetView,
  type Scope,
} from "../../../api/assets";
import {
  countAssets,
  filterAndSort,
  groupAssets,
  orderForPaging,
  PAGE_SIZE,
  paginate,
  projectOptions,
  type ProjectFilter,
  type SortMode,
  type TypeFilter,
} from "./filterAssets";
import { AssetCard, CARD_MIN_WIDTH } from "./AssetCard";
import { AssetInspector } from "./AssetInspector";
import { NewAssetDialog } from "./NewAssetDialog";
import { AssetWorkbench } from "../assets/AssetWorkbench";
import { cinemaListProjects, type CinemaProject } from "../../../api/cinema";
import { EyeIcon } from "./NsfwThumb";
import { useNsfw } from "./nsfw";
import { useStudioChanged } from "../useStudioChanged";
import { PublicLibraryView } from "./PublicLibraryView";

const SORT_LABEL: Record<SortMode, string> = {
  default: "默认排序",
  name: "按名称",
  recent: "最近更新",
  usage: "引用最多",
};

/**
 * 资产库。
 *
 * # 2026-09-15 重设计
 *
 * - **类型是主轴**（最常做的是「找一个角色」），最左边的分段按钮
 * - **项目改成下拉筛选**，替掉「按类型 / 按项目」切换：一个轴一个控件。选了项目看这个项目的 + 全局的
 * - 👁 NSFW 总开关：默认模糊
 * - 卡片比例跟类型走；「全部」时按类型分段，各段自己的卡片宽度
 * - 详情面板 460px，按页签分：形象 / 人物 / 引用（见 `AssetInspector`）
 */
export function AssetLibraryPage() {
  const [all, setAll] = useState<AssetView[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const [typeFilter, setTypeFilter] = useState<TypeFilter>("all");
  /** 「公共素材」页签：现成背景，只读，挑了再加进资产库 */
  const [publicView, setPublicView] = useState(false);
  const [project, setProject] = useState<ProjectFilter>("all");
  const [starredOnly, setStarredOnly] = useState(false);
  const [query, setQuery] = useState("");
  const [sort, setSort] = useState<SortMode>("default");
  const nsfwAll = useNsfw((s) => s.showAll);
  const setNsfwAll = useNsfw((s) => s.setShowAll);

  const [selected, setSelected] = useState<{ scope: Scope; id: string } | null>(null);
  const [creating, setCreating] = useState(false);
  /** 出图工作台：哪个资产、哪个形象、用哪个项目的风格和出图设置 */
  const [bench, setBench] = useState<{ scope: Scope; id: string; lookId: string; project: CinemaProject } | null>(null);
  const [benchError, setBenchError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    setLoading(true);
    try {
      setAll(await assetsList());
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);
  useStudioChanged(() => void reload(), { kinds: ["assets", "project"] });

  // 计数跟着项目筛选走 —— 选了「盛唐」后「角色 10」还显示全库的数就对不上
  const inProject = useMemo(() => filterAndSort(all, { typeFilter: "all", starredOnly: false, query: "", sort: "default", project }), [all, project]);
  const counts = useMemo(() => countAssets(inProject), [inProject]);
  const projects = useMemo(() => projectOptions(all), [all]);

  const filtered = useMemo(
    () => filterAndSort(all, { typeFilter, starredOnly, query, sort, project }),
    [all, typeFilter, starredOnly, query, sort, project],
  );

  // **分页**（用户 2026-09-26：内容多了会拉得很长）。筛选 / 搜索 / 排序一变就回第 1 页
  const [page, setPage] = useState(1);
  const listRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => setPage(1), [typeFilter, project, starredOnly, query, sort]);
  const paged = useMemo(() => paginate(orderForPaging(filtered, typeFilter), page, PAGE_SIZE), [filtered, typeFilter, page]);
  const goPage = (p: number) => {
    setPage(p);
    listRef.current?.scrollTo({ top: 0 });
  };
  // 分段标题上写这一类**总共**多少个，不是这一页的 —— 否则翻到第 2 页像是东西变少了
  const totals = useMemo(() => countAssets(filtered), [filtered]);

  const groups = useMemo(() => groupAssets(paged.items, "type"), [paged.items]);

  const current = selected
    ? all.find((a) => a.id === selected.id && sameScope(a.scope, selected.scope)) ?? null
    : null;
  const benchAsset = bench ? all.find((a) => a.id === bench.id && sameScope(a.scope, bench.scope)) ?? null : null;

  /**
   * 打开出图工作台。**和工作区资产那一步是同一个工作台、同一套选项。**
   * 项目里的资产用它所在项目的风格和出图设置；全局资产不属于任何项目 —— 用默认设置、不带项目风格。
   */
  const openBench = async (asset: AssetView, lookId: string) => {
    setBenchError(null);
    let project: CinemaProject = GLOBAL_PROJECT;
    if (asset.scope.kind !== "global") {
      const pid = asset.scope.project_id;
      try {
        project = (await cinemaListProjects()).find((p) => p.id === pid) ?? GLOBAL_PROJECT;
      } catch (e) {
        setBenchError(`读不到项目设置：${e}`);
        return;
      }
    }
    setBench({ scope: asset.scope, id: asset.id, lookId, project });
  };

  const pills: { id: TypeFilter; label: string; count: number }[] = [
    { id: "all", label: "全部", count: counts.all },
    { id: "character", label: "角色", count: counts.character },
    { id: "scene", label: "场景", count: counts.scene },
    { id: "prop", label: "道具", count: counts.prop },
  ];
  const projectValue = project === "all" ? "all" : project === "global" ? "global" : `p:${project.project_id}`;

  return (
    <div className="flex min-w-0 flex-1 overflow-hidden">
      <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
        <header
          className="studio-banner flex shrink-0 items-start justify-between px-7 py-5"
          style={{ borderBottom: "1px solid var(--studio-border-subtle)" }}
          data-tauri-drag-region
        >
          <div className="flex flex-col gap-1">
            <span className="studio-eyebrow" style={{ letterSpacing: "0.2em" }}>
              ASSET LIBRARY
              <span style={{ color: "var(--studio-primary)", fontWeight: 600 }}>
                {" · "}角色 · 场景 · 道具
              </span>
            </span>
            <div className="flex items-baseline gap-[10px]">
              <h1 className="studio-display text-[2rem]">资产库</h1>
              <span className="text-[12px]" style={{ color: "var(--studio-text-muted)" }}>
                {all.length} 个资产
              </span>
            </div>
          </div>
          <button
            type="button"
            onClick={() => setCreating(true)}
            className="rounded-full px-4 py-2 text-[13px] font-medium"
            style={{ background: "var(--studio-primary)", color: "var(--studio-on-primary)" }}
          >
            + 新建资产
          </button>
        </header>

        {/* ── 过滤条：左边「看什么」，右边「怎么找」 ── */}
        <div className="flex shrink-0 flex-wrap items-center gap-2.5 px-7 py-3.5">
          <div className="flex items-center gap-1 rounded-full p-0.5" style={chipGroup} role="tablist" aria-label="资产类型">
            {pills.map((p) => (
              <button
                key={p.id}
                type="button"
                role="tab"
                aria-selected={!publicView && typeFilter === p.id}
                onClick={() => {
                  setTypeFilter(p.id);
                  setPublicView(false);
                }}
                className="rounded-full px-3 py-1.5 text-[12px]"
                style={{
                  background: !publicView && typeFilter === p.id ? "var(--studio-bg-elevated)" : "transparent",
                  color: !publicView && typeFilter === p.id ? "var(--studio-text)" : "var(--studio-text-muted)",
                }}
              >
                {p.label} <span className="font-mono text-[0.6rem] opacity-60">{p.count}</span>
              </button>
            ))}
          </div>
          <button
            type="button"
            role="tab"
            aria-selected={publicView}
            onClick={() => {
              setPublicView(true);
              setSelected(null);
            }}
            className="rounded-full px-3 py-1.5 text-[12px]"
            style={{ ...chipGroup, color: publicView ? "var(--studio-primary)" : "var(--studio-text-muted)" }}
            title="现成的背景素材，挑一个加进全局库或项目"
          >
            公共素材
          </button>
          {!publicView && (
          <select
            aria-label="项目"
            value={projectValue}
            onChange={(e) => {
              const v = e.target.value;
              setProject(v === "all" ? "all" : v === "global" ? "global" : { project_id: v.slice(2) });
            }}
            className="rounded-full px-3 py-1.5 text-[12px] outline-none"
            style={{ ...chipGroup, color: project === "all" ? "var(--studio-text-2)" : "var(--studio-primary)" }}
          >
            <option value="all">全部项目</option>
            <option value="global">只看全局库</option>
            {projects.map((p) => (
              <option key={p.id} value={`p:${p.id}`}>
                {p.title}
              </option>
            ))}
          </select>
          )}

          <div className="ml-auto flex min-w-0 flex-wrap items-center gap-2">
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder={publicView ? "搜名字、分类、时段（夜晚、黄昏…）" : "搜名字、描述、形象…"}
              className="w-[220px] min-w-0 rounded-full px-4 py-1.5 text-[12px] outline-none"
              style={{ ...chipGroup, color: "var(--studio-text)" }}
            />
            {!publicView && (<>
            <button
              type="button"
              onClick={() => setStarredOnly((v) => !v)}
              aria-pressed={starredOnly}
              className="rounded-full px-3 py-1.5 text-[12px]"
              style={{ ...chipGroup, color: starredOnly ? "var(--studio-primary)" : "var(--studio-text-muted)" }}
              title="只看星标"
            >
              ★ {counts.starred}
            </button>
            <button
              type="button"
              onClick={() => setNsfwAll(!nsfwAll)}
              aria-pressed={nsfwAll}
              className="flex items-center gap-1.5 rounded-full px-3 py-1.5 text-[12px]"
              style={{ ...chipGroup, color: nsfwAll ? "var(--studio-primary)" : "var(--studio-text-muted)" }}
              title={nsfwAll ? "NSFW 图全部显示中，点一下改回模糊" : "NSFW 图默认模糊，点一下全部显示"}
            >
              <EyeIcon off={!nsfwAll} />
              NSFW
            </button>
            <select
              aria-label="排序"
              value={sort}
              onChange={(e) => setSort(e.target.value as SortMode)}
              className="rounded-full px-3 py-1.5 text-[12px] outline-none"
              style={{ ...chipGroup, color: "var(--studio-text-2)" }}
            >
              {(Object.keys(SORT_LABEL) as SortMode[]).map((k) => (
                <option key={k} value={k}>
                  {SORT_LABEL[k]}
                </option>
              ))}
            </select>
            </>)}
          </div>
        </div>

        {/* ── 内容 ── */}
        {publicView ? (
          <div className="flex min-h-0 flex-1 px-7">
            <PublicLibraryView query={query} onImported={() => void reload()} />
          </div>
        ) : (
        <div ref={listRef} className="min-h-0 flex-1 overflow-y-auto px-7 pb-10">
          {error && (
            <p className="py-6 text-[13px]" style={{ color: "var(--studio-danger)" }}>
              读取失败：{error}
            </p>
          )}
          {!error && loading && all.length === 0 && (
            <p className="py-6 text-[13px]" style={{ color: "var(--studio-text-muted)" }}>
              正在读取…
            </p>
          )}
          {!error && !loading && groups.length === 0 && <EmptyState hasAny={all.length > 0} />}
          {groups.map((g) => (
            <section key={g.key} className="mb-7">
              {/* 只看一类时不再重复写标题 */}
              {typeFilter === "all" && (
                <div className="mb-3 flex items-baseline gap-2">
                  <h2 className="studio-display text-[15px]">{g.title}</h2>
                  <span className="font-mono text-[0.6rem]" style={{ color: "var(--studio-text-muted)" }}>
                    {totals[g.key as AssetKind] ?? g.items.length}
                  </span>
                </div>
              )}
              <div
                className="grid gap-3"
                style={{ gridTemplateColumns: `repeat(auto-fill, minmax(${CARD_MIN_WIDTH[g.key as AssetKind] ?? 150}px, 1fr))` }}
              >
                {g.items.map((a) => (
                  <AssetCard
                    key={`${a.scope_label}/${a.id}`}
                    asset={a}
                    active={current?.id === a.id && sameScope(current.scope, a.scope)}
                    showScope={project === "all"}
                    onOpen={() => setSelected({ scope: a.scope, id: a.id })}
                  />
                ))}
              </div>
            </section>
          ))}
          {paged.pages > 1 && <Pager page={paged.page} pages={paged.pages} total={filtered.length} onGo={goPage} />}
        </div>
        )}
      </div>

      {current && !publicView && (
        <AssetInspector
          asset={current}
          onClose={() => setSelected(null)}
          onChanged={reload}
          onGenerate={(lookId) => void openBench(current, lookId)}
        />
      )}

      {bench && benchAsset && (
        <AssetWorkbench
          project={bench.project}
          asset={benchAsset}
          initialLookId={bench.lookId}
          onClose={() => setBench(null)}
          onChanged={reload}
        />
      )}
      {benchError && (
        <p className="fixed bottom-4 left-1/2 z-50 -translate-x-1/2 rounded-lg px-3 py-2 text-[12px]" style={{ background: "var(--studio-bg-elevated)", color: "var(--studio-failed-fg)" }}>
          {benchError}
        </p>
      )}

      {creating && (
        <NewAssetDialog
          onClose={() => setCreating(false)}
          onCreated={() => {
            setCreating(false);
            void reload();
          }}
        />
      )}
    </div>
  );
}

/** 全局资产出图时的项目上下文：没有项目风格，出图设置走默认 */
const GLOBAL_PROJECT: CinemaProject = {
  id: "",
  title: "全局资产库",
  genre: "",
  aspect_ratio: "16:9",
  description: "",
  created_at: 0,
  updated_at: 0,
  episode_count: 0,
  kind: "standalone",
};

/**
 * 翻页条：上一页 / 页码 / 下一页。页码多时只显示首尾和当前页附近，中间用省略号。
 */
function Pager({ page, pages, total, onGo }: { page: number; pages: number; total: number; onGo: (p: number) => void }) {
  const nums: (number | "…")[] = [];
  for (let p = 1; p <= pages; p++) {
    if (p === 1 || p === pages || Math.abs(p - page) <= 1) nums.push(p);
    else if (nums[nums.length - 1] !== "…") nums.push("…");
  }
  const btn = (on: boolean): React.CSSProperties => ({
    ...chipGroup,
    background: on ? "color-mix(in srgb, var(--studio-primary) 16%, transparent)" : chipGroup.background,
    color: on ? "var(--studio-primary)" : "var(--studio-text-2)",
  });
  return (
    <nav className="mt-2 flex flex-wrap items-center justify-center gap-1.5" aria-label="分页">
      <button type="button" disabled={page <= 1} onClick={() => onGo(page - 1)} className="rounded-full px-3 py-1.5 text-[12px] disabled:opacity-40" style={btn(false)}>
        上一页
      </button>
      {nums.map((n, i) =>
        n === "…" ? (
          <span key={`e${i}`} className="px-1 text-[12px]" style={{ color: "var(--studio-text-muted)" }}>
            …
          </span>
        ) : (
          <button key={n} type="button" aria-current={n === page ? "page" : undefined} onClick={() => onGo(n)} className="min-w-[32px] rounded-full px-2.5 py-1.5 font-mono text-[12px]" style={btn(n === page)}>
            {n}
          </button>
        ),
      )}
      <button type="button" disabled={page >= pages} onClick={() => onGo(page + 1)} className="rounded-full px-3 py-1.5 text-[12px] disabled:opacity-40" style={btn(false)}>
        下一页
      </button>
      <span className="ml-2 text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
        共 {total} 个 · 每页 {PAGE_SIZE}
      </span>
    </nav>
  );
}

const chipGroup: React.CSSProperties = {
  background: "var(--studio-bg-inset)",
  border: "1px solid var(--studio-border-subtle)",
};

/**
 * 空状态要分清两种情况：**一个资产都没有** vs **筛掉了**。
 * 混成一句「暂无资产」的话，用户会以为东西丢了。
 */
function EmptyState({ hasAny }: { hasAny: boolean }) {
  return (
    <div
      className="mt-10 rounded-2xl px-8 py-14 text-center"
      style={{ background: "var(--studio-bg-inset)" }}
    >
      <p className="studio-eyebrow mb-3" style={{ letterSpacing: "0.2em" }}>
        CAST · SCENES · PROPS
      </p>
      <p className="text-[15px]">
        {hasAny ? "当前筛选下没有资产" : "还没有任何资产"}
      </p>
      <p className="mt-2 text-[12px]" style={{ color: "var(--studio-text-muted)" }}>
        {hasAny
          ? "换个类型或清掉搜索词试试"
          : "在创作台出一张满意的图，点「录入资产库」；或者直接「+ 新建资产」先占个名字"}
      </p>
    </div>
  );
}

export default AssetLibraryPage;
