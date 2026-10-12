/**
 * 创作台的队列面板。
 *
 * **和分镜那边同一个壳**（`SidePanel`），但数据源不同：分镜的队列读的是
 * Rust 侧落盘的渲染任务（`jobs.rs`），这里读的是画廊自己的 `results` ——
 * 创作台的提交本来就是同一份状态，再落一次盘只会多一份要对齐的真相。
 *
 * 失败的那些也列在这里，**带原文**。失败原因在这条链路上几乎都是可操作的
 * （ComfyUI 没起、内存不够、模型缺文件），概括掉等于让人从头猜。
 */
import { useMemo } from "react";
import { SidePanel } from "./SidePanel";
import { useStudioStore, type StudioResult } from "./useStudioStore";

const RUNNING = new Set(["pending", "running"]);

function elapsed(r: StudioResult, now: number): string {
  const end = r.endedAt ?? now;
  const s = Math.max(0, Math.round((end - r.startedAt) / 1000));
  if (s < 60) return `${s}s`;
  return `${Math.floor(s / 60)}m ${s % 60}s`;
}

export function ResultQueuePanel({ onClose }: { onClose: () => void }) {
  const results = useStudioStore((s) => s.results);
  const removeResult = useStudioStore((s) => s.removeResult);
  const now = Date.now();

  const { running, failed } = useMemo(
    () => ({
      running: results.filter((r) => RUNNING.has(r.status)),
      failed: results.filter((r) => r.status === "failed"),
    }),
    [results],
  );

  return (
    <SidePanel
      title="队列"
      subtitle={`${running.length} 个在跑 · ${failed.length} 个失败`}
      onClose={onClose}
    >
      <div className="min-h-0 flex-1 overflow-y-auto px-3 py-2.5">
        {running.length === 0 && failed.length === 0 && (
          <p className="mt-8 text-center text-[12px]" style={{ color: "var(--studio-text-muted)" }}>
            现在没有任务在跑
          </p>
        )}

        {running.length > 0 && (
          <Section label="运行中" n={running.length}>
            {running.map((r) => (
              <Row key={r.id} r={r} now={now} />
            ))}
          </Section>
        )}

        {failed.length > 0 && (
          <Section label="失败" n={failed.length}>
            {failed.map((r) => (
              <Row key={r.id} r={r} now={now} onDismiss={() => removeResult(r.id)} />
            ))}
          </Section>
        )}
      </div>
    </SidePanel>
  );
}

function Section({ label, n, children }: { label: string; n: number; children: React.ReactNode }) {
  return (
    <section className="mt-3 first:mt-0">
      <div className="mb-1.5 flex items-center gap-1.5 px-1">
        <span className="studio-eyebrow">{label}</span>
        <span
          className="font-mono text-[0.55rem] tabular-nums"
          style={{ color: "var(--studio-text-muted)" }}
        >
          {n}
        </span>
      </div>
      <div className="flex flex-col gap-1.5">{children}</div>
    </section>
  );
}

function Row({
  r,
  now,
  onDismiss,
}: {
  r: StudioResult;
  now: number;
  onDismiss?: () => void;
}) {
  const live = RUNNING.has(r.status);
  return (
    <div
      className="flex flex-col gap-1.5 rounded-lg px-2.5 py-2"
      style={{ background: "var(--studio-bg-inset)" }}
    >
      <div className="flex items-center gap-2">
        <span
          className="h-1.5 w-1.5 shrink-0 rounded-full"
          style={{
            background: live ? "var(--studio-running-fg)" : "var(--studio-failed-fg)",
            animation: live ? "studio-pulse 1.4s ease-in-out infinite" : "none",
          }}
          aria-hidden="true"
        />
        <span className="font-mono text-[10px]" style={{ color: "var(--studio-text-muted)" }}>
          {r.mode} · {r.engineId}
        </span>
        <span
          className="ml-auto font-mono text-[10px] tabular-nums"
          style={{ color: "var(--studio-text-muted)" }}
        >
          {elapsed(r, now)}
        </span>
        {onDismiss && (
          <button
            type="button"
            onClick={onDismiss}
            aria-label="从列表里移掉"
            title="从列表里移掉"
            className="grid h-5 w-5 shrink-0 place-items-center rounded"
            style={{ color: "var(--studio-text-muted)" }}
          >
            <svg width="10" height="10" viewBox="0 0 24 24" fill="none" aria-hidden="true">
              <path d="M6 6l12 12M18 6L6 18" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" />
            </svg>
          </button>
        )}
      </div>
      <p
        className="text-[11px] leading-snug"
        style={{
          color: "var(--studio-text-muted)",
          display: "-webkit-box",
          WebkitLineClamp: 2,
          WebkitBoxOrient: "vertical",
          overflow: "hidden",
        }}
      >
        {r.prompt.trim() || "（没有提示词）"}
      </p>
      {/* **失败原文原样显示。**「生成失败」和「ComfyUI 没起」是两回事，
          后者用户自己就能修。 */}
      {r.error && (
        <p
          className="rounded px-2 py-1.5 text-[10.5px] leading-relaxed"
          style={{
            background: "color-mix(in srgb, var(--studio-failed-fg) 12%, transparent)",
            color: "var(--studio-failed-fg)",
            wordBreak: "break-word",
          }}
        >
          {r.error}
        </p>
      )}
    </div>
  );
}
