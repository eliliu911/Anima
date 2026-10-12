/**
 * 资产页。**复用资产库那一套组件**，作用域锁到当前项目。
 * 全局库在左侧一级导航的「资产库」里，两者是同一个存储的两个视角。
 *
 * # 三种模式（2026-09-25 起）
 *
 * | 模式 | 在哪 | 显示什么 |
 * |---|---|---|
 * | `series` | 系列项目 · 系列组「角色与资产」 | 待归属（各集还没进系列的）+ 系列池全部 |
 * | `cast` | 系列项目 · 本集组「本集选角」 | 系列池里**这一集出场的** + 只在本集的 |
 * | `combined` | 单集的独立项目 · 第 3 步 | 和以前一样：本集 + 系列两层 |
 *
 * 系列的定义（新建、出设定卡）在 `series`；`cast` 只回答「这一集谁出场」。
 */
import { useStudioChanged } from "../useStudioChanged";
import { useCallback, useEffect, useMemo, useState } from "react";
import { assetsList, type AssetView } from "../../../api/assets";
import { AssetWorkbench } from "../assets/AssetWorkbench";
import { AssetCard } from "../library/AssetCard";
import { NewAssetDialog } from "../library/NewAssetDialog";
import { PublicLibraryView } from "../library/PublicLibraryView";
import { groupAssets } from "../library/filterAssets";
import { countByKind, filterKind, KIND_TABS, type KindFilter } from "./assetTabs";
import { batchSummary, redoSummary } from "./batchAssets";
import { effectiveGen } from "../assets/effectiveGen";
import { assetsBatchGenerateStart } from "../../../api/assets";
import { useJobs } from "../jobs/QueuePanel";
import { episodeJobState } from "../jobs/shotBusy";
import type { StepProps } from "./types";
import { SeriesInboxPanel } from "../series/SeriesInboxPanel";
import { castOf } from "../series/castOf";
import { DesignDocsExtract } from "../series/DesignDocsExtract";
import type { EpisodeData } from "../../../api/cinema";

export type AssetsMode = "series" | "cast" | "combined";

export function AssetsStep({
  project,
  episode,
  onError,
  mode = "combined",
  onGoSeriesAssets,
  episodes = [],
}: StepProps & {
  mode?: AssetsMode;
  /** 系列页「从设定文档建资产」要用（导入接口要一个分集 id） */
  episodes?: EpisodeData[];
  /** 本集选角里「去系列管理」的那条路 */
  onGoSeriesAssets?: () => void;
}) {
  const [items, setItems] = useState<AssetView[] | null>(null);
  const [epItems, setEpItems] = useState<AssetView[]>([]);
  /** 顶部页签：`all` 或某一类。 */
  const [kind, setKind] = useState<KindFilter>("all");
  /** 卡片高亮用。点卡片直接开 `AssetWorkbench`，所以它只会是 null —— 留着是为了卡片的 active 样式 */
  const selectedId: string | null = null;
  const [creating, setCreating] = useState(false);
  /** 从公共素材库挑背景加进这个项目 */
  const [picking, setPicking] = useState(false);
  const [pickQuery, setPickQuery] = useState("");
  /** 正在出图的那一张。点卡片直接进 —— 出形象是这一页的主要动作。 */
  const [bench, setBench] = useState<string | null>(null);
  /** 从详情面板某个形象上点「生成」进来时选中的形象 */
  const [benchLook, setBenchLook] = useState<string | undefined>(undefined);
  /** 点了「一键全出」、任务还没登记上的那一下 */
  const [batchStarting, setBatchStarting] = useState(false);
  /** 「从设定文档建资产」弹窗 + 做完之后那一句 */
  const [fromDocs, setFromDocs] = useState(false);
  const [docsNote, setDocsNote] = useState("");

  /** 系列池 —— 跨集共用的那些。新建资产默认建到这一层。 */
  const scope = useMemo(
    () => (project ? ({ kind: "project", project_id: project.id } as const) : null),
    [project],
  );
  /** 本集名单 —— 提取实体的产物先落在这儿，还没决定要不要进系列池。 */
  const epScope = useMemo(
    () =>
      // 系列页不读某一集的 —— 各集的待归属由 SeriesInboxPanel 一次读全
      project && episode && mode !== "series"
        ? ({ kind: "episode", project_id: project.id, episode_id: episode.id } as const)
        : null,
    [project, episode, mode],
  );

  /**
   * **两层都要读。**
   *
   * 提取实体写的是本集那一层（见 `import_entities`）。只读系列池的话，
   * 刚提取完的东西在这一页**一个都看不见** —— 而且不报错，
   * 看起来就是「提取没生效」。
   */
  const reload = useCallback(async () => {
    if (!scope) return;
    try {
      const [proj, ep] = await Promise.all([
        assetsList(scope),
        epScope ? assetsList(epScope) : Promise.resolve([] as AssetView[]),
      ]);
      setItems(proj);
      setEpItems(ep);
    } catch (e) {
      setItems([]);
      setEpItems([]);
      onError(String(e));
    }
  }, [scope, epScope, onError]);

  useEffect(() => {
    void reload();
  }, [reload]);
  // Chat 在这个项目里建了资产：不切页也刷出来
  useStudioChanged(() => void reload(), { kinds: ["assets"], projectId: project?.id });

  /** 批量统一用项目底模 —— 逐个资产的覆盖只在工作台里生效。 */
  const engine = useMemo(
    () => effectiveGen("character", null, project?.model_settings).engine,
    [project],
  );
  /** 批量出图是后台任务 —— 从任务表读在不在跑、跑到哪、哪几张没出来；切走再回来也对 */
  const batchEpisode = mode === "series" ? null : (episode?.id ?? null);
  const { jobs } = useJobs(project?.id ?? "", batchEpisode ?? "", false, undefined, "assets");
  const batchJob = episodeJobState(jobs, "asset_images");
  const batchBusy = batchStarting || batchJob.generating;
  const bp = batchJob.progress;
  const batchAt = bp && bp.total > 0 ? `${Math.min(bp.done + 1, bp.total)}/${bp.total} · ${bp.label}` : "";
  const batchFailed = batchJob.notes;


  if (!project || !scope) return null;

  const seriesAll = items ?? [];
  // 本集选角：系列池里只看这一集出场的
  const all = mode === "cast" ? castOf(seriesAll, episode) : seriesAll;
  const seriesNames = Object.fromEntries(seriesAll.map((a) => [a.id, a.name]));
  // **两边都按类型分组。** 平铺六张卡的话，角色、场景、道具混在一起，
  // 一眼看不出「这一集有几个人」—— 而那正是这一页要回答的问题。
  // 卡片图片位按本项目每一类的出图画幅（和出图工作台用的同一份：`effectiveGen`）
  const aspectOf = (k: string) => effectiveGen(k, null, project?.model_settings).aspect;
  const groups = groupAssets(filterKind(all, kind), "type");
  const epGroups = groupAssets(filterKind(epItems, kind), "type");
  const tally = countByKind([...epItems, ...all]);

  /**
   * 一键把还没出图的资产（和形象）全出一遍 —— **后台任务**（Rust `asset_batch`）。
   *
   * 以前是这里的前端循环：切到别的步骤、页面刷新，循环就断了。现在串行出图、失败记下接着出、
   * 每出一张发 `studio://changed`（上面的监听会刷卡片），进度和结果在右下角，Chat 发起的也一样。
   */
  const runBatch = async (redo = false) => {
    if (!project || batchBusy) return;
    setBatchStarting(true);
    onError(null);
    try {
      await assetsBatchGenerateStart(project.id, batchEpisode, redo);
    } catch (e) {
      onError(String(e));
    } finally {
      setBatchStarting(false);
    }
  };
  const [inboxAsset, setInboxAsset] = useState<AssetView | null>(null);
  const benchAsset =
    [...(items ?? []), ...epItems].find((a) => a.id === bench) ??
    (inboxAsset?.id === bench ? inboxAsset : null);

  return (
    <div className="flex h-full min-w-0">
      <div className="min-w-0 flex-1 overflow-y-auto px-7 py-5">
        <div className="mb-4 flex items-center gap-2">
          <button
            type="button"
            onClick={() => setCreating(true)}
            className="rounded-lg px-3.5 py-2 text-[12px] font-medium"
            style={{ background: "var(--studio-primary)", color: "var(--studio-on-primary)" }}
          >
            {mode === "cast" ? "+ 新建资产（进系列）" : "+ 新建资产"}
          </button>
          <button
            type="button"
            onClick={() => setPicking(true)}
            className="rounded-lg px-3.5 py-2 text-[12px]"
            style={{ border: "1px solid var(--studio-border-subtle)", color: "var(--studio-text)" }}
            title="现成的背景图，不用生成"
          >
            从公共素材库加场景
          </button>
          {mode === "series" && (
            <button
              type="button"
              onClick={() => setFromDocs(true)}
              className="rounded-lg px-3.5 py-2 text-[12px]"
              style={{ border: "1px solid var(--studio-border-subtle)", color: "var(--studio-text)" }}
              title="用剧本编辑器里的人物小传 / 场景设计 / 道具设计，一次建好整部剧的资产"
            >
              从设定文档建资产
            </button>
          )}
          <span className="text-[12px]" style={{ color: "var(--studio-text-muted)" }}>
            {mode === "cast"
              ? `本集出场 ${all.length} 个（系列共 ${seriesAll.length} 个）`
              : `系列 ${seriesAll.length} 个`}
            {mode !== "series" && epItems.length > 0 ? ` · 只在本集 ${epItems.length} 个` : ""}
            {" · 提示词里用 "}
            <code>@</code> 引用它们
          </span>
          {mode === "cast" && onGoSeriesAssets && (
            <button
              type="button"
              onClick={onGoSeriesAssets}
              className="ml-auto text-[12px]"
              style={{ color: "var(--studio-text-2)" }}
            >
              去系列「角色与资产」 ›
            </button>
          )}
        </div>

        {/* **还差几张要在最上面说。**
            以前只有一排占位渐变的卡片 —— 没图和有图长得差不多，
            漏掉一个要到出分镜时才发现（那时它只会以名字进提示词，
            长相不会带上）。这一条是 liblib 那套「准备资产」里最该抄的。 */}
        {batchSummary([...epItems, ...all], engine) && (
          <div
            className="mb-4 flex items-start gap-3 rounded-xl px-4 py-3"
            style={{ background: "var(--studio-bg-inset)" }}
          >
            <span className="mt-0.5 shrink-0 text-[13px]">⚠</span>
            <p className="min-w-0 flex-1 text-[12px] leading-relaxed">
              {batchSummary([...epItems, ...all], engine)}
            </p>
            <button
              type="button"
              disabled={batchBusy}
              onClick={() => void runBatch()}
              className="shrink-0 rounded-lg px-3.5 py-2 text-[12px] font-medium disabled:opacity-50"
              style={{ background: "var(--studio-primary)", color: "var(--studio-on-primary)" }}
            >
              {batchBusy ? `出图中 ${batchAt}` : "一键全出"}
            </button>
          </div>
        )}

        {/* **失败清单跑完要留着。** 不留的话，十个里失败两个，
            界面上只是少了两张图 —— 和「还没点过」长得一模一样。 */}
        {/* **全都有图之后还得有个入口。**
            判据只有「有没有封面」的话，一批出砸了也算「有图」，
            横幅撤掉就没有任何地方能重来 —— 用户 2026-09-12 当场撞上。 */}
        {redoSummary([...epItems, ...all]) && (
          <div
            className="mb-4 flex items-center gap-3 rounded-xl px-4 py-2.5"
            style={{ background: "var(--studio-bg-inset)" }}
          >
            <p className="min-w-0 flex-1 text-[12px]" style={{ color: "var(--studio-text-2)" }}>
              {redoSummary([...epItems, ...all])}
            </p>
            <button
              type="button"
              disabled={batchBusy}
              onClick={() => void runBatch(true)}
              className="shrink-0 rounded-lg px-3 py-1.5 text-[12px] disabled:opacity-50"
              style={{
                border: "1px solid var(--studio-border-subtle)",
                color: "var(--studio-text)",
              }}
            >
              {batchBusy ? `出图中 ${batchAt}` : "全部重出"}
            </button>
          </div>
        )}

        {batchFailed.length > 0 && (
          <div
            className="mb-4 rounded-xl px-4 py-3 text-[12px] leading-relaxed"
            style={{ background: "var(--studio-bg-inset)", color: "var(--studio-failed-fg)" }}
          >
            <p className="mb-1 font-medium">上一批有 {batchFailed.length} 张没出来：</p>
            {batchFailed.map((m) => (
              <p key={m}>· {m}</p>
            ))}
          </div>
        )}

        <div className="mb-4 flex items-center gap-1 rounded-full p-1"
             style={{ background: "var(--studio-bg-inset)", width: "fit-content" }}>
          {KIND_TABS.map(({ key, label }) => {
            const on = kind === key;
            return (
              <button
                key={key}
                type="button"
                onClick={() => setKind(key)}
                aria-pressed={on}
                className="rounded-full px-3 py-1.5 text-[12px] font-medium"
                style={{
                  background: on ? "var(--studio-bg-surface)" : "transparent",
                  color: on ? "var(--studio-text)" : "var(--studio-text-muted)",
                  boxShadow: on ? "0 0 0 1px var(--studio-border-subtle)" : "none",
                }}
              >
                {label}
                <span className="ml-1 font-mono text-[10px] opacity-60">{tally[key]}</span>
              </button>
            );
          })}
        </div>

        {docsNote && (
          <p className="mb-4 text-[12px]" style={{ color: "var(--studio-text-2)" }}>
            {docsNote}
          </p>
        )}

        {/* 系列页：各集里还没进系列的，在最上面逐条决定去向 */}
        {mode === "series" && (
          <SeriesInboxPanel
            projectId={project.id}
            seriesNames={seriesNames}
            onChanged={() => void reload()}
            onOpen={(it) => {
              setInboxAsset(it.asset);
              setBench(it.asset.id);
            }}
          />
        )}

        {/* **本集那一层。** 以前叫「本集待归属」，提取的东西全落这儿；
            现在提取默认进系列，这里只剩用户点成「只留本集」的和老数据。
            要把它们收进系列，去系列页的「待归属」。 */}
        {epGroups.length > 0 && (
          <div className="mb-7">
            <div className="mb-3 flex items-baseline gap-2">
              <h2 className="text-[13px] font-medium">只在本集</h2>
              <span
                className="font-mono text-[0.6rem]"
                style={{ color: "var(--studio-text-muted)" }}
              >
                {epItems.length}
              </span>
              <span className="text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
                {epItems.some((a) => !a.episode_only)
                  ? "其中有还没决定去向的 —— 去系列「角色与资产」的待归属处理"
                  : "其它集用不到"}
              </span>
            </div>
            {epGroups.map((g) => (
              <AssetGroup
                key={g.key}
                title={`本集${g.title}`}
                items={g.items}
                selectedId={selectedId}
                onOpen={setBench}
                onDeleted={() => void reload()}
                aspectOf={aspectOf}
              />
            ))}
          </div>
        )}

        {items === null && (
          <p className="text-[12px]" style={{ color: "var(--studio-text-muted)" }}>
            读取中…
          </p>
        )}
        {items !== null && all.length === 0 && epItems.length === 0 && (
          <div
            className="mt-6 rounded-2xl px-8 py-12 text-center"
            style={{ background: "var(--studio-bg-inset)" }}
          >
            <p className="text-[15px]">
              {mode === "cast" && seriesAll.length > 0 ? "这一集还没认出出场的资产" : mode === "series" ? "系列里还没有资产" : "这个项目还没有资产"}
            </p>
            <p className="mt-2 text-[12px]" style={{ color: "var(--studio-text-muted)" }}>
              {mode === "cast" && seriesAll.length > 0
                ? "剧本里写到系列角色的名字、或者分镜里用了它，就会出现在这里。"
                : "去剧本那一步点「提取实体」，剧本里的角色/场景/道具会直接进系列。也可以手动新建，或者从全局资产库复制。"}
            </p>
          </div>
        )}

        {groups.length > 0 && (
          <div className="mb-2 flex items-baseline gap-2">
            <h2 className="text-[13px] font-medium">{mode === "cast" ? "本集出场" : "系列资产"}</h2>
            <span
              className="font-mono text-[0.6rem]"
              style={{ color: "var(--studio-text-muted)" }}
            >
              {all.length}
            </span>
            <span className="text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
              {mode === "cast" ? "剧本里写到、或分镜里用到的系列资产" : "跨集共用，`@` 引用的就是这些"}
            </span>
          </div>
        )}
        {groups.map((g) => (
          <AssetGroup
            key={g.key}
            title={g.title}
            items={g.items}
            selectedId={selectedId}
            onOpen={setBench}
            onDeleted={() => void reload()}
            aspectOf={aspectOf}
          />
        ))}
      </div>

      {/* **这里原来还渲染了一个 `AssetInspector`（460px 的详情面板）。**
          它从 5cbc149 起就是死的：那次把卡片的 `onOpen` 改成直接开工作台，
          于是 `selectedId` 只剩「置空」一处调用，面板永远不出现。

          2026-09-18 被发现：音色字段加进了那个面板的「人物」页签，
          用户在这一页怎么点都找不到。**编译过、测试绿、不报错，就是不出现。**
          `assetSurfaces.test.ts` 现在盯着这件事。

          详情面板还活着的地方是独立的「资产库」页（`AssetLibraryPage`）。 */}

      {/* **`CharacterWorkbench`（三视图 / 表情 / 姿势 / 面部机位）也一起摘了。**
          它的唯一入口是上面那个孤儿面板里的「打开角色工作台 →」按钮，
          所以它从 5cbc149 起在整个应用里**一次都没被打开过**。

          文件留着（`./CharacterWorkbench.tsx`），没有删 —— 它和本页的
          `AssetWorkbench` 功能有重叠（构图模板里就有三视图 / 四视图 / 五分区），
          很可能是 5cbc149 有意取代它、只是忘了清理。**但这不该由我猜。**
          等用户定：要么给它一个入口，要么删掉。 */}

      {benchAsset && project && (
        <AssetWorkbench
          project={project}
          asset={benchAsset}
          initialLookId={benchLook}
          onClose={() => {
            setBench(null);
            setBenchLook(undefined);
          }}
          onChanged={reload}
          onDeleted={() => {
            setBench(null);
            setBenchLook(undefined);
            void reload();
          }}
        />
      )}

      {fromDocs && (
        <DesignDocsExtract
          project={project}
          episodes={episodes}
          onClose={() => setFromDocs(false)}
          onDone={(n) => {
            setFromDocs(false);
            setDocsNote(n);
            void reload();
          }}
        />
      )}

      {creating && (
        <NewAssetDialog
          scope={scope}
          onClose={() => setCreating(false)}
          onCreated={() => {
            setCreating(false);
            void reload();
          }}
        />
      )}

      {picking && (
        <div className="fixed inset-0 z-50 flex items-center justify-center p-6" style={{ background: "rgba(0,0,0,.55)" }} role="dialog" aria-modal="true">
          <div className="flex h-[85vh] w-full max-w-[1280px] flex-col overflow-hidden rounded-2xl" style={{ background: "var(--studio-bg-elevated)", border: "1px solid var(--studio-border)" }}>
            <div className="flex shrink-0 items-center gap-3 px-6 py-4" style={{ borderBottom: "1px solid var(--studio-border-subtle)" }}>
              <h2 className="studio-display text-[16px]">从公共素材库加场景</h2>
              <span className="text-[12px]" style={{ color: "var(--studio-text-muted)" }}>加到「{project.title}」，所有集都能用</span>
              <input
                value={pickQuery}
                onChange={(e) => setPickQuery(e.target.value)}
                placeholder="搜名字、分类、时段（夜晚、黄昏…）"
                className="ml-auto w-[260px] rounded-full px-4 py-1.5 text-[12px] outline-none"
                style={{ background: "var(--studio-bg-inset)", border: "1px solid var(--studio-border-subtle)", color: "var(--studio-text)" }}
              />
              <button type="button" onClick={() => setPicking(false)} className="px-2 text-[18px]" style={{ color: "var(--studio-text-muted)" }} aria-label="关闭">
                ×
              </button>
            </div>
            <div className="flex min-h-0 flex-1 px-6 pt-4">
              <PublicLibraryView
                query={pickQuery}
                fixedScope={{ scope, label: `「${project.title}」` }}
                onImported={() => void reload()}
              />
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

/**
 * 一组同类资产。**本集和系列共用这一个** —— 两边各写一遍的话，
 * 改了一处另一处不动，而表现只是「上面那块和下面那块长得不一样」。
 */
function AssetGroup({
  title,
  items,
  selectedId,
  onOpen,
  onDeleted,
  aspectOf,
}: {
  title: string;
  items: AssetView[];
  selectedId: string | null;
  onOpen: (id: string) => void;
  /** 卡片上的「删除」删完之后（2026-09-30 用户：资产页没地方删） */
  onDeleted: () => void;
  /** 这一类资产在本项目的出图画幅 —— 卡片图片位跟着它，不写死竖版（见 `ASPECT_CLASS`） */
  aspectOf: (kind: string) => string;
}) {
  return (
    <section className="mb-5">
      <div className="mb-2.5 flex items-baseline gap-2">
        <h3 className="text-[12.5px] font-medium" style={{ color: "var(--studio-text-2)" }}>
          {title}
        </h3>
        <span className="font-mono text-[0.6rem]" style={{ color: "var(--studio-text-muted)" }}>
          {items.length}
        </span>
      </div>
      <div className="grid grid-cols-[repeat(auto-fill,minmax(148px,1fr))] gap-3">
        {items.map((a) => (
          <AssetCard
            key={a.id}
            asset={a}
            active={a.id === selectedId}
            showScope={false}
            onOpen={() => onOpen(a.id)}
            onDeleted={onDeleted}
            aspect={aspectOf(a.kind)}
          />
        ))}
      </div>
    </section>
  );
}
