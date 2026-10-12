/**
 * 系列概览：点进一个系列先到这里，不再直接掉进第 1 集的剧本。
 *
 * 从上到下回答三个问题：
 * 1. **整部剧做到哪了** —— 剧本 / 分镜 / 成片各几集
 * 2. **有没有要我处理的** —— 各集里还没进系列池的资产
 * 3. **每一集什么状态** —— 分集卡片，点一张进那一集
 *
 * 中间一排主演头像：出设定卡是出片的前提，谁还没图一眼看得出来（虚线框）。
 * 头像用 `object-contain` 不裁 —— 封面往往是设定卡，裁成方格就只剩一块（见 thumbFit.test）。
 */
import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import type { CinemaProject, EpisodeData } from "../../../api/cinema";
import { JOB_EVENT, type Job } from "../../../api/jobs";
import { tokenUsageSummary, type UsageSummary } from "../../../api/tokenUsage";
import { assetsList, type AssetView } from "../../../api/assets";
import { seriesAssetsInbox } from "../../../api/seriesAssets";
import { useStudioChanged } from "../useStudioChanged";
import { cardTitle, episodeProgress, mainCast, seriesStats } from "./overviewStats";
import { AspectSetting } from "./AspectSetting";
import { TokenUsagePanel } from "./TokenUsagePanel";
import { episodeUsageLabel } from "./tokenUsage";

export function SeriesOverview({
  project,
  episodes,
  onOpenEpisode,
  onGoAssets,
  onAddEpisode,
  onDeleteEpisode,
  onProjectChanged,
}: {
  project: CinemaProject;
  episodes: EpisodeData[];
  onOpenEpisode: (id: string) => void;
  onGoAssets: () => void;
  onAddEpisode: () => void;
  onDeleteEpisode: (e: EpisodeData) => void;
  /** 项目级设置（画幅）改了，项目页要重读 */
  onProjectChanged: () => void;
}) {
  const [assets, setAssets] = useState<AssetView[] | null>(null);
  const [pending, setPending] = useState(0);

  const reload = useCallback(() => {
    void assetsList({ kind: "project", project_id: project.id })
      .then(setAssets)
      .catch(() => setAssets([]));
    void seriesAssetsInbox(project.id)
      .then((ib) => setPending(ib.pending.length))
      .catch(() => setPending(0));
  }, [project.id]);
  useEffect(reload, [reload]);
  useStudioChanged(reload, { kinds: ["assets"], projectId: project.id });

  /**
   * Token 用量。**任务跑着的时候也要跟着涨** —— 这部剧的后台任务每报一次进度（一场 / 一镜跑完）就重读，
   * 攒 1.5 秒读一次，不是每个事件都读。
   */
  const [usage, setUsage] = useState<UsageSummary | null>(null);
  const reloadUsage = useCallback(() => {
    void tokenUsageSummary(project.id)
      .then(setUsage)
      .catch(() => setUsage(null));
  }, [project.id]);
  useEffect(() => {
    reloadUsage();
    let timer: ReturnType<typeof setTimeout> | undefined;
    const off = listen<Job>(JOB_EVENT, (e) => {
      if (e.payload.project_id !== project.id) return;
      clearTimeout(timer);
      timer = setTimeout(reloadUsage, 1500);
    });
    return () => {
      clearTimeout(timer);
      void off.then((f) => f());
    };
  }, [project.id, reloadUsage]);

  const stats = seriesStats(episodes);
  const cast = mainCast(assets ?? []);
  const others = (assets ?? []).length - cast.length;

  const stat = (label: string, value: string) => (
    <div className="rounded-xl px-4 py-3" style={{ background: "var(--studio-bg-inset)" }}>
      <p className="text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
        {label}
      </p>
      <p className="mt-0.5 font-mono text-[20px] tabular-nums">{value}</p>
    </div>
  );

  return (
    <div className="px-7 py-5">
      <div className="mb-5 grid grid-cols-4 gap-3">
        {stat("剧本", `${stats.scripted}/${stats.total}`)}
        {stat("系列角色", assets === null ? "…" : String(cast.length))}
        {stat("分镜", `${stats.boarded}/${stats.total}`)}
        {stat("成片", `${stats.finished}/${stats.total}`)}
      </div>

      {/* 画幅是整部剧最先要定、定了就别乱动的东西 —— 放在最上面 */}
      <AspectSetting project={project} episodes={episodes} onChanged={onProjectChanged} />

      {/* **待归属要在最上面说。** 它们不在系列池里，下一集提取时认不出来，
          出设定卡也找不到 —— 不提醒的话，要到第二集长出同名角色才发现。 */}
      {pending > 0 && (
        <button
          type="button"
          onClick={onGoAssets}
          className="mb-5 flex w-full items-center gap-3 rounded-xl px-4 py-3 text-left"
          style={{
            background: "var(--studio-pending-bg)",
            border: "1px solid var(--studio-pending-bd)",
            color: "var(--studio-pending-fg)",
          }}
        >
          <span className="min-w-0 flex-1 text-[12.5px]">
            各集里有 {pending} 个资产还没进系列 —— 其它集用不到，出设定卡也找不到它们
          </span>
          <span className="shrink-0 text-[12px] underline">去处理</span>
        </button>
      )}

      <section className="mb-6">
        <div className="mb-2.5 flex items-baseline gap-2">
          <h2 className="text-[13px] font-medium">系列主演</h2>
          <span className="text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
            {others > 0 ? `另有 ${others} 个场景、道具等` : ""}
          </span>
          <button
            type="button"
            onClick={onGoAssets}
            className="ml-auto text-[12px]"
            style={{ color: "var(--studio-text-2)" }}
          >
            角色与资产 ›
          </button>
        </div>
        {assets !== null && cast.length === 0 && (
          <p className="text-[12px]" style={{ color: "var(--studio-text-muted)" }}>
            系列里还没有角色。进任意一集的剧本点「提取实体」，角色会直接进系列。
          </p>
        )}
        <div className="flex flex-wrap gap-3">
          {cast.map((a) => (
            <button
              key={a.id}
              type="button"
              onClick={onGoAssets}
              title={a.cover_url ? a.name : `${a.name}：还没出图`}
              className="flex w-[64px] flex-col items-center gap-1"
            >
              <span
                className="block h-[56px] w-[56px] overflow-hidden rounded-lg"
                style={{
                  background: "var(--studio-bg-inset)",
                  border: a.cover_url ? "1px solid var(--studio-border-subtle)" : "1px dashed var(--studio-border)",
                }}
              >
                {a.cover_url && <img src={a.cover_url} alt="" className="h-full w-full object-contain" />}
              </span>
              <span
                className="w-full truncate text-center text-[11px]"
                style={{ color: a.cover_url ? "var(--studio-text-2)" : "var(--studio-text-muted)" }}
              >
                {a.name}
              </span>
            </button>
          ))}
        </div>
      </section>

      <section>
        <div className="mb-2.5 flex items-baseline gap-2">
          <h2 className="text-[13px] font-medium">分集</h2>
          <span className="font-mono text-[0.6rem]" style={{ color: "var(--studio-text-muted)" }}>
            {episodes.length}
          </span>
        </div>
        <div className="grid grid-cols-[repeat(auto-fill,minmax(200px,1fr))] gap-2.5">
          {episodes.map((e) => (
            <div
              key={e.id}
              className="group relative rounded-xl"
              style={{ border: "1px solid var(--studio-border-subtle)", background: "var(--studio-bg-surface)" }}
            >
              <button
                type="button"
                onClick={() => onOpenEpisode(e.id)}
                className="block w-full px-3.5 py-3 text-left"
                title={e.synopsis || undefined}
              >
                <span className="flex items-baseline gap-1.5">
                  <span className="font-mono text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
                    E{String(e.episode_num).padStart(2, "0")}
                  </span>
                  <span className="truncate text-[13px]">{cardTitle(e)}</span>
                </span>
                <span className="mt-1 flex items-baseline gap-2 text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
                  <span className="min-w-0 truncate">{episodeProgress(e)}</span>
                  {/* 这一集用了多少 token —— 卡片右边本来空着（用户 2026-09-29 要的位置） */}
                  <span className="ml-auto shrink-0 font-mono text-[10.5px] tabular-nums" title="这一集调用模型用掉的 token（输入 + 输出）。估算费用在页面最下面的 Token 用量里">
                    {episodeUsageLabel(usage?.episodes[e.id])}
                  </span>
                </span>
              </button>
              {/* 只剩一集时不给删 —— 后端也会拦 */}
              {episodes.length > 1 && (
                <button
                  type="button"
                  onClick={() => onDeleteEpisode(e)}
                  title="删除这一集"
                  aria-label={`删除 ${e.title}`}
                  className="absolute right-2 top-2 px-1 text-[13px] leading-none opacity-0 transition-opacity group-hover:opacity-100 focus:opacity-100"
                  style={{ color: "var(--studio-text-muted)" }}
                >
                  ×
                </button>
              )}
            </div>
          ))}
          <button
            type="button"
            onClick={onAddEpisode}
            className="rounded-xl px-3.5 py-3 text-left text-[12.5px]"
            style={{ border: "1px dashed var(--studio-border)", color: "var(--studio-text-muted)" }}
          >
            + 加一集
          </button>
        </div>
      </section>

      <TokenUsagePanel usage={usage} onRefresh={reloadUsage} />
    </div>
  );
}
