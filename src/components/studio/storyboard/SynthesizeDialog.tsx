import { useEffect, useState } from "react";
import { cinemaStalePrompts, type CinemaProject, type PromptRoute, type ShotIssue, type StalePrompts, type StoryboardShot } from "../../../api/cinema";
import { framesFirst, synthTargets } from "./storyboardView";

/** 两条线路给人看的说明。**状态写实话**：哪条出片验证过、哪条没有。 */
export const ROUTE_INFO: Record<PromptRoute, { name: string; how: string; speed: string; status: string }> = {
  a: {
    name: "线路 A · 中文结构",
    how: "照 libtv 的中文脚本结构纯填空：出场角色、场景、参考图规则、分时间段描述、台词、输出约束。不经过模型。",
    speed: "每镜不到 1 秒",
    status: "提示词结构检查通过；用我们合成的提示词还没出片验证过",
  },
  b: {
    name: "线路 B · H3 官方六段",
    how: "H3 官方参考生视频六段结构：结构和参考图编号由代码定，英文描述由后台模型写，台词由代码逐字填入。",
    speed: "每镜约 10 秒（后台模型）",
    status: "已整集出片跑通",
  },
};

/**
 * 合成提示词：**左边线路 A、右边线路 B，各合各的**。
 *
 * 2026-09-29 用户：「直接分成左右两个分块，单独逻辑处理……不然现在有 confuse」。原来是一个弹窗里选线路 + 选范围 + 一个按钮，
 * 而且**合成哪条就把哪条存成出片线路** —— 合成完 A 再切到 B，B 的过期镜头看不见（v0.42.107 修了一半），出片线路也跟着被改了。
 *
 * 现在：
 * - 每块有自己的范围、自己的镜数、自己的按钮，合成的就是那一块的线路（`onStart(route, ids)`）
 * - **合成不改出片线路**。出片用哪条在出片设置里选；这里只标出「出片用这条」
 * - 同一集同一时间只能跑一个合成任务（后端独占）：一边在跑，另一边的按钮写明「等线路 X 合成完」
 * - 点了开始不关弹窗：A 几秒就好，接着就能点 B；跑完分集刷新，两边的镜数跟着更新
 */
export function SynthesizeDialog({
  project,
  episodeId,
  shots,
  issues,
  running,
  progress,
  onClose,
  onStart,
}: {
  project: CinemaProject;
  episodeId: string;
  shots: StoryboardShot[];
  issues: ShotIssue[];
  /** 正在合成的线路（见 `runningRoute`） */
  running: PromptRoute | null;
  progress?: { done: number; total: number } | null;
  onClose: () => void;
  onStart: (route: PromptRoute, shotIds: string[]) => Promise<void>;
}) {
  const current: PromptRoute = project.model_settings?.prompt_route ?? "b";
  /**
   * 两条线路各自过期的镜头。**问题清单只报出片线路的**，另一条要靠这个（见 `synthTargets`）。
   * 分集一刷新（合成跑完）就重问一次
   */
  const [stale, setStale] = useState<StalePrompts | null>(null);
  useEffect(() => {
    let alive = true;
    void cinemaStalePrompts(project.id, episodeId)
      .then((s) => alive && setStale(s))
      .catch(() => alive && setStale(null));
    return () => {
      alive = false;
    };
  }, [project.id, episodeId, shots]);

  return (
    <div className="studio-scrim fixed inset-0 z-50 flex items-center justify-center">
      <div className="studio-modal flex max-h-[86vh] w-[860px] max-w-[94vw] flex-col overflow-hidden" onClick={(e) => e.stopPropagation()}>
        <header className="px-5 pb-3 pt-4" style={{ borderBottom: "1px solid var(--studio-border-subtle)" }}>
          <h2 className="studio-display text-[16px]">合成提示词</h2>
          <p className="mt-0.5 text-[11.5px]" style={{ color: "var(--studio-text-muted)" }}>
            两条线路各合成各的，互不覆盖。出片用哪条在出片设置里选 —— 现在是{ROUTE_INFO[current].name}
          </p>
        </header>

        <div className="studio-scroll grid min-h-0 flex-1 grid-cols-1 gap-3 overflow-y-auto px-5 py-4 md:grid-cols-2">
          {(["a", "b"] as PromptRoute[]).map((r) => (
            <RoutePanel
              key={r}
              route={r}
              isCurrent={r === current}
              shots={shots}
              issues={issues}
              stale={stale}
              running={running}
              progress={running === r ? progress : null}
              onStart={(ids) => onStart(r, ids)}
            />
          ))}
        </div>

        <footer className="flex items-center justify-end px-5 py-3" style={{ borderTop: "1px solid var(--studio-border-subtle)" }}>
          <button type="button" onClick={onClose} className="rounded-lg px-3 py-1.5 text-[12px]" style={{ color: "var(--studio-text-2)" }}>
            关闭
          </button>
        </footer>
      </div>
    </div>
  );
}

/** 一条线路自己的一块：范围、镜数、按钮都只管这条线路 */
function RoutePanel({
  route,
  isCurrent,
  shots,
  issues,
  stale,
  running,
  progress,
  onStart,
}: {
  route: PromptRoute;
  isCurrent: boolean;
  shots: StoryboardShot[];
  issues: ShotIssue[];
  stale: StalePrompts | null;
  running: PromptRoute | null;
  progress?: { done: number; total: number } | null;
  onStart: (shotIds: string[]) => Promise<void>;
}) {
  const info = ROUTE_INFO[route];
  const [onlyNeeded, setOnlyNeeded] = useState(true);
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const needed = synthTargets(shots, route, issues, true, stale);
  const ids = onlyNeeded ? needed : synthTargets(shots, route, issues, false, stale);
  // 这次要合的里面还没首帧、也没合成过的：要出首帧的话该先出（出完自动合成），现在合了出完首帧还得再合一遍
  const noFrame = shots.filter((s) => ids.includes(s.id) && framesFirst([s], route).length > 0).length;
  const mine = running === route;
  const other = running !== null && !mine;
  const other_name = running === "a" ? "线路 A" : "线路 B";

  const start = async () => {
    setStarting(true);
    setError(null);
    try {
      await onStart(ids);
    } catch (e) {
      setError(String(e));
    } finally {
      setStarting(false);
    }
  };

  const status = mine
    ? progress && progress.total > 0
      ? `合成中 ${Math.min(progress.done + 1, progress.total)}/${progress.total}…`
      : "合成中…"
    : ids.length
      ? `合成 ${ids.length} 镜 · ${route === "b" ? `约 ${Math.max(1, Math.round((ids.length * 10) / 60))} 分钟` : "几秒"}`
      : "都是最新的，不用合成";

  return (
    <section className="flex flex-col rounded-xl px-4 py-3" style={{ boxShadow: "0 0 0 1px var(--studio-border)" }}>
      <p className="flex items-baseline gap-2 text-[13px] font-medium">
        {info.name}
        {isCurrent && (
          <span className="rounded px-1.5 py-0.5 text-[10.5px] font-normal" style={{ background: "var(--studio-bg-inset)", color: "var(--studio-text-2)" }}>
            出片用这条
          </span>
        )}
      </p>
      <p className="mt-1.5 text-[11.5px] leading-relaxed" style={{ color: "var(--studio-text-2)" }}>
        {info.how}
      </p>
      <p className="mt-1 text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
        {info.speed} · {info.status}
      </p>

      <div className="mt-3 grid grid-cols-2 gap-2">
        {[
          { v: true, label: `没合成或过期的 ${needed.length} 镜` },
          { v: false, label: `整集 ${shots.length} 镜` },
        ].map((o) => (
          <button
            key={String(o.v)}
            type="button"
            aria-pressed={onlyNeeded === o.v}
            onClick={() => setOnlyNeeded(o.v)}
            className="rounded-lg px-2 py-2 text-[12px]"
            style={{
              background: onlyNeeded === o.v ? "var(--studio-bg-elevated)" : "transparent",
              color: onlyNeeded === o.v ? "var(--studio-text)" : "var(--studio-text-muted)",
              boxShadow: `0 0 0 1px ${onlyNeeded === o.v ? "var(--studio-text-2)" : "var(--studio-border)"}`,
            }}
          >
            {o.label}
          </button>
        ))}
      </div>

      {noFrame > 0 && (
        <p className="mt-2 text-[11.5px] leading-relaxed" style={{ color: "var(--studio-pending-fg)" }}>
          其中 {noFrame} 镜还没首帧。要出首帧的，先去「按场出首帧」—— 出完会自动合成一次；现在合成的话，出完首帧还得再合一遍。不出首帧（用参考图出片）的照常合成。
        </p>
      )}

      <div className="mt-3 flex items-center gap-2">
        <p className="min-w-0 flex-1 text-[11.5px]" style={{ color: mine ? "var(--studio-running-fg)" : "var(--studio-text-muted)" }}>
          {status}
        </p>
        <button
          type="button"
          disabled={starting || mine || other || ids.length === 0}
          onClick={() => void start()}
          title={other ? `${other_name}在合成，同一集一次只能合成一条线路，等它跑完` : undefined}
          className="shrink-0 rounded-lg px-3 py-1.5 text-[12px] font-medium"
          style={{ background: "var(--studio-primary)", color: "var(--studio-on-primary)", opacity: starting || mine || other || ids.length === 0 ? 0.5 : 1 }}
        >
          {other ? `等${other_name}合成完` : `合成${route === "a" ? "线路 A" : "线路 B"}`}
        </button>
      </div>
      {error && (
        <p className="mt-1.5 text-[11.5px]" style={{ color: "var(--studio-failed-fg)" }}>
          {error}
        </p>
      )}
    </section>
  );
}
