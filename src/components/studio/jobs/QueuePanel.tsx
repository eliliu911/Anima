/**
 * 任务队列：按钮 + 右侧抽屉。版式照 LumenX 的 TaskQueuePanel。
 *
 * # 这里显示的是**真实状态**
 *
 * 数据来自 Rust 侧的任务登记（`jobs.rs`）：每次渲染命令进出时写盘，
 * 失败原文照抄。以前一次渲染就是个阻塞的 await，界面上只有一个转圈，
 * 跑完之前没有任何地方能回答「现在有几个在跑、刚才那个为什么失败」。
 *
 * # 没有取消按钮，这是故意的
 *
 * 真取消要拿到 ComfyUI 的 `prompt_id` 再去 `/queue` 删或 `/interrupt`，
 * 那个 id 现在没往外暴露。一个「点了把状态改成已取消、但 ComfyUI
 * 照跑」的按钮，比没有按钮更糟 —— 用户会以为机器空出来了。
 */
import { useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { cinemaJobs, cinemaJobsClear, JOB_EVENT, type Job } from "../../../api/jobs";
import { MediaThumb } from "../MediaThumb";
import { RenderQueueList } from "./RenderQueueList";
import { SidePanel } from "../SidePanel";
import { bucketJobs, defaultTab, elapsedLabel, pollMs, type JobTab } from "./jobBuckets";
import { jobTitle, jobsKey, lastJobs, rememberJobs, settledShots } from "./shotBusy";

/** 队列的数据源。按钮和面板共用一份 —— 各拉各的必然对不上。 */
/**
 * 轮询任务登记表。
 *
 * `onSettled` 在**有任务从「在跑」变成「跑完/失败」**时调一次 ——
 * 用来自动刷新分集。不刷的话产物已经在磁盘上了，而界面还显示
 * 「还没出片」，得手动切页才看得到。
 */
export function useJobs(
  projectId: string,
  episodeId: string,
  panelOpen: boolean,
  onSettled?: (shotIds: string[]) => void,
  /**
   * 这个调用点的名字。**每个调用点必须不同** —— 快照按它分键，
   * 共用的话你切到哪一页哪一页就把落地消费掉了（见 `jobsKey` 的注释）。
   */
  consumer = "default",
) {
  const [jobs, setJobs] = useState<Job[]>([]);
  // 回调放 ref —— 它每次渲染都是新函数，进依赖数组会把轮询循环重启，
  // 而重启会丢掉「上一轮是什么」，落地就永远检测不到。
  const settled = useRef(onSettled);
  settled.current = onSettled;
  // **上一轮的快照存在模块级，不是 ref** —— ref 会随卸载一起没，
  // 于是切走再回来时「上一轮」是空的，在别处跑完的任务永远检测不到。
  // 理由和实测见 `shotBusy.ts` 里 `seen` 那一段。
  const key = jobsKey(consumer, projectId, episodeId);
  const buckets = useMemo(() => bucketJobs(jobs), [jobs]);
  const running = buckets.running.length;

  useEffect(() => {
    let alive = true;
    let timer: number | undefined;
    const tick = async () => {
      try {
        const next = await cinemaJobs(projectId, episodeId);
        if (!alive) return;
        const done = settledShots(lastJobs(key), next);
        rememberJobs(key, next);
        setJobs(next);
        if (done.length > 0) settled.current?.(done);
        // **间隔按刚拿到的结果算，不是按渲染时的旧值。** 用旧值的话，
        // 提交完第一次轮询还停在 6 秒档，角标要 6 秒后才动。
        const ms = pollMs(next.filter((j) => j.status === "running").length, panelOpen);
        timer = window.setTimeout(() => void tick(), ms);
      } catch {
        // 读不到就当空的，下一轮再试 —— 队列读失败不该弹错误打断工作
        if (alive) timer = window.setTimeout(() => void tick(), 6000);
      }
    };
    void tick();
    // 任务状态一变就立刻刷，不等下一轮轮询 —— 刚点「生成」时按钮和角标应当马上动。
    const off = listen<Job>(JOB_EVENT, (e) => {
      if (!alive || e.payload.project_id !== projectId || e.payload.episode_id !== episodeId) return;
      if (timer) window.clearTimeout(timer);
      void tick();
    });
    return () => {
      alive = false;
      if (timer) window.clearTimeout(timer);
      void off.then((f) => f());
    };
  }, [projectId, episodeId, panelOpen, key]);

  return { jobs, buckets, running };
}

export function QueueButton({
  running,
  total,
  open,
  onToggle,
}: {
  running: number;
  total: number;
  open: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onToggle}
      aria-pressed={open}
      title={running > 0 ? `${running} 个任务正在跑` : `${total} 条任务记录`}
      className="flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 font-mono text-[11px] uppercase"
      style={{
        background: open ? "color-mix(in srgb, var(--studio-primary) 16%, transparent)" : "var(--studio-bg-inset)",
        color: open ? "var(--studio-primary)" : "var(--studio-text-2)",
      }}
    >
      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" aria-hidden="true">
        <path
          d="M3 6h2l1.5 1.5M3 12h2l1.5 1.5M3 18h2l1.5 1.5M11 6h10M11 12h10M11 18h10"
          stroke="currentColor"
          strokeWidth="1.8"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </svg>
      Queue
      {running > 0 && (
        // 定宽 + tabular-nums：数字从 1 跳到 10 时按钮不该跟着变宽
        <span
          className="ml-0.5 inline-flex min-w-[16px] items-center justify-center rounded-full px-1 text-[10px] font-semibold tabular-nums"
          style={{ background: "var(--studio-primary)", color: "var(--studio-on-primary)" }}
        >
          {running}
        </span>
      )}
    </button>
  );
}

export function QueuePanel({
  projectId,
  episodeId,
  jobs,
  buckets,
  onClose,
  onJump,
}: {
  projectId: string;
  episodeId: string;
  jobs: Job[];
  buckets: Record<JobTab, Job[]>;
  onClose: () => void;
  onJump: (shotId: string) => void;
}) {
  // **默认页签只在打开那一刻定一次。** 每次轮询都重算的话，
  // 用户翻到「已完成」，一个任务跑完就把他弹回「运行中」。
  const [tab, setTab] = useState<JobTab>(() => defaultTab(buckets));
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000));

  // 「跑了多久」要自己走。只在轮询时更新的话，闲着的秒数会一跳一跳。
  useEffect(() => {
    const t = window.setInterval(() => setNow(Math.floor(Date.now() / 1000)), 1000);
    return () => window.clearInterval(t);
  }, []);

  const rows = buckets[tab];
  const TABS: [JobTab, string][] = [
    ["running", "运行中"],
    ["done", "已完成"],
    ["failed", "失败"],
  ];

  return (
    <SidePanel
      title="任务队列"
      subtitle={`${jobs.length} 条记录`}
      onClose={onClose}
      trailing={
        (buckets.done.length > 0 || buckets.failed.length > 0) && (
          <button
            type="button"
            onClick={() => void cinemaJobsClear(projectId, episodeId)}
            title="清掉已结束的。正在跑的不动"
            className="rounded-md px-2 py-1 text-[11px]"
            style={{ color: "var(--studio-text-muted)" }}
          >
            清空
          </button>
        )
      }
    >
        <div
          role="tablist"
          className="flex shrink-0 gap-1 px-3 py-2"
          style={{ borderBottom: "1px solid var(--studio-border-subtle)" }}
        >
          {TABS.map(([key, label]) => (
            <button
              key={key}
              type="button"
              role="tab"
              aria-selected={tab === key}
              onClick={() => setTab(key)}
              className="flex-1 rounded-md py-1.5 text-[11.5px]"
              style={{
                background: tab === key ? "var(--studio-bg-inset)" : "transparent",
                color: tab === key ? "var(--studio-text)" : "var(--studio-text-muted)",
              }}
            >
              {label}
              {buckets[key].length > 0 && (
                <span className="ml-1.5 tabular-nums" style={{ color: "var(--studio-primary)" }}>
                  {buckets[key].length}
                </span>
              )}
            </button>
          ))}
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto px-3 py-2.5">
          {rows.length === 0 ? (
            <p
              className="mt-8 text-center text-[12px]"
              style={{ color: "var(--studio-text-muted)" }}
            >
              {tab === "running"
                ? "现在没有任务在跑"
                : tab === "done"
                  ? "还没有跑完的任务"
                  : "没有失败的任务"}
            </p>
          ) : (
            <ul className="flex flex-col gap-1.5">
              {rows.map((j) => (
                <li key={j.id}>
                  <JobRow job={j} now={now} onJump={() => onJump(j.shot_id)} />
                </li>
              ))}
            </ul>
          )}
        </div>
    </SidePanel>
  );
}

const DOT: Record<Job["status"], string> = {
  running: "var(--studio-running-fg)",
  done: "var(--studio-done-fg)",
  failed: "var(--studio-failed-fg)",
};

function JobRow({ job, now, onJump }: { job: Job; now: number; onJump: () => void }) {
  // 在跑的出片批量：点标题展开排队名单，可以拖着改顺序（`RenderQueueList`）
  const queueable = job.kind === "video_batch" && job.status === "running";
  const [open, setOpen] = useState(false);
  return (
    <div
      className="flex flex-col gap-1.5 rounded-lg px-2.5 py-2"
      style={{ background: "var(--studio-bg-inset)" }}
    >
      <div
        className={`flex items-center gap-2 ${queueable ? "cursor-pointer" : ""}`}
        onClick={queueable ? () => setOpen((v) => !v) : undefined}
        title={queueable ? (open ? "收起排队名单" : "展开排队名单，可以拖着改顺序") : undefined}
      >
        <span
          className="h-1.5 w-1.5 shrink-0 rounded-full"
          style={{
            background: DOT[job.status],
            animation: job.status === "running" ? "studio-pulse 1.4s ease-in-out infinite" : "none",
          }}
          aria-hidden="true"
        />
        <span className="text-[12px] font-medium">
          {jobTitle(job)}
        </span>
        <span
          className="ml-auto font-mono text-[10px] tabular-nums"
          style={{ color: "var(--studio-text-muted)" }}
        >
          {elapsedLabel(job, now)}
        </span>
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            onJump();
          }}
          title="跳到这一镜"
          className="grid h-5 w-5 shrink-0 place-items-center rounded"
          style={{ color: "var(--studio-text-muted)" }}
        >
          <svg width="11" height="11" viewBox="0 0 24 24" fill="none" aria-hidden="true">
            <path d="M5 12h14M13 6l6 6-6 6" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
        </button>
      </div>

      <div className="flex gap-2">
        {job.output && (
          <div
            className="h-[34px] w-[52px] shrink-0 overflow-hidden rounded"
            style={{ background: "var(--studio-bg-base)" }}
          >
            {/* **contain 不是 cover。** 例外清单已经卡在五条上限上了，
                那条上限就是防止不断加例外把「资产图不许裁」稀释成空话。
                这个格子只有 52px，裁不裁都看不清细节，而留一点黑边
                完全无所谓 —— 没有理由为它开第六条例外。 */}
            <MediaThumb
              path={job.output}
              kind={job.kind === "video" ? "video" : "image"}
              // 首帧固定写 `shots/<id>.png`，重出一次路径不变 ——
              // 用完成时间做版本，否则这里也会一直显示旧图。
              version={job.finished_at}
              className="h-full w-full object-contain"
            />
          </div>
        )}
        <p
          className="min-w-0 flex-1 text-[11px] leading-snug"
          style={{
            color: "var(--studio-text-muted)",
            display: "-webkit-box",
            WebkitLineClamp: 2,
            WebkitBoxOrient: "vertical",
            overflow: "hidden",
          }}
        >
          {job.summary ?? (job.prompt.trim() || "（没有提示词）")}
        </p>
      </div>

      {queueable && open && <RenderQueueList jobId={job.id} refreshKey={`${job.progress?.done}|${(job.targets ?? []).join(",")}`} />}

      {/* **失败原文原样显示，不概括。**「生成失败」和「ComfyUI 没起」
          是完全不同的两件事，而后者用户自己就能修。 */}
      {job.error && (
        <p
          className="rounded px-2 py-1.5 text-[10.5px] leading-relaxed"
          style={{
            background: "color-mix(in srgb, var(--studio-failed-fg) 12%, transparent)",
            color: "var(--studio-failed-fg)",
            wordBreak: "break-word",
          }}
        >
          {job.error}
        </p>
      )}
    </div>
  );
}
