/**
 * 任务队列里点开一个出片批量：没轮到的名单，可以拖着改顺序（2026-10-10 用户：「14/23 个任务的列队，但是完全没办法调整顺序」）。
 *
 * - 拖的是**组**：要接上一镜的和上一镜合在一张卡片里，只能一起挪（后端 `render_queue::check_order` 再查一遍）
 * - 已经出完的和正在出的那一镜不在这里，不能动
 * - 暂停位（MCP 插的，给冒烟测试之类让出 GPU）带「撤掉」按钮：撤了批量接着出
 * - 名单每次轮询都跟着后端刷新；拖完立刻存，存不了（拆开了连着出的组）就显示原因、退回后端的顺序
 */
import { useCallback, useEffect, useState } from "react";
import { cinemaReleaseHold, cinemaRenderQueue, cinemaReorderQueue, type QueueView } from "../../../api/jobs";
import { flatIds, moveGroup } from "./renderQueue";

export function RenderQueueList({ jobId, refreshKey }: { jobId: string; refreshKey: unknown }) {
  const [view, setView] = useState<QueueView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [drag, setDrag] = useState<number | null>(null);
  const [over, setOver] = useState<number | null>(null);

  const load = useCallback(() => {
    cinemaRenderQueue(jobId).then(setView, (e) => setError(String(e)));
  }, [jobId]);
  // 任务每更新一次（进度、名单）就重读
  useEffect(load, [load, refreshKey]);

  if (!view) return error ? <p className="text-[10.5px]" style={{ color: "var(--studio-failed-fg)" }}>{error}</p> : null;

  const drop = (to: number) => {
    if (drag === null) return;
    const next = moveGroup(view.pending, drag, to);
    setDrag(null);
    setOver(null);
    if (next === view.pending) return;
    setView({ ...view, pending: next });
    setError(null);
    cinemaReorderQueue(jobId, flatIds(next)).then(setView, (e) => {
      setError(String(e));
      load();
    });
  };

  return (
    <div className="flex flex-col gap-1 pt-1">
      {/* 停在暂停位上：它是「正在出」的那一项、不在下面的名单里 —— 单独显示，带「撤掉」（2026-10-10 用户看不懂「正在出：暂停」） */}
      {view.current?.hold ? (
        <div className="flex items-start gap-2 rounded-md px-2 py-1.5" style={{ background: "color-mix(in srgb, var(--studio-accent) 12%, transparent)" }}>
          <p className="flex-1 text-[11px] leading-snug" style={{ color: "var(--studio-text)" }}>
            暂停中：{view.current.label.replace(/^暂停 · /, "")}。现在没在出片，不占 GPU；撤掉暂停位就从{view.pending[0]?.[0]?.label.split(" · ")[0] ?? "下一项"}接着出。
          </p>
          <button
            type="button"
            className="shrink-0 rounded px-1.5 py-0.5 text-[10.5px]"
            style={{ color: "var(--studio-failed-fg)" }}
            onClick={() => cinemaReleaseHold(jobId, view.current!.id).then(setView, (e) => setError(String(e)))}
          >
            撤掉
          </button>
        </div>
      ) : null}
      <p className="text-[10.5px]" style={{ color: "var(--studio-text-muted)" }}>
        {view.current && !view.current.hold ? `正在出：${view.current.label} · ` : ""}后面还有 {view.pending.flat().filter((i) => !i.hold).length} 镜 · 拖动卡片改顺序（连在一起的要接上一镜，只能整组挪）
      </p>
      {view.pending.length === 0 && <p className="text-[11px]" style={{ color: "var(--studio-text-muted)" }}>后面没有排队的了</p>}
      <ol className="flex flex-col">
        {view.pending.map((group, gi) => (
          <li
            key={group.map((i) => i.id).join("|")}
            draggable
            onDragStart={(e) => {
              setDrag(gi);
              e.dataTransfer.effectAllowed = "move";
            }}
            onDragEnd={() => {
              setDrag(null);
              setOver(null);
            }}
            onDragOver={(e) => {
              e.preventDefault();
              setOver(gi);
            }}
            onDrop={(e) => {
              e.preventDefault();
              drop(gi);
            }}
            className="my-0.5 flex cursor-grab flex-col gap-0.5 rounded-md px-2 py-1.5 active:cursor-grabbing"
            style={{
              background: "var(--studio-bg-base)",
              opacity: drag === gi ? 0.4 : 1,
              borderTop: over === gi && drag !== null && drag !== gi ? "2px solid var(--studio-primary)" : "2px solid transparent",
            }}
          >
            {group.map((item, ii) => (
              <div key={item.id} className="flex items-center gap-2 text-[11.5px]">
                <span className="w-4 shrink-0 text-right font-mono text-[10px] tabular-nums" style={{ color: "var(--studio-text-muted)" }}>
                  {ii === 0 ? gi + 1 : "↳"}
                </span>
                <span style={{ color: item.hold ? "var(--studio-accent)" : "var(--studio-text)" }}>{item.label}</span>
                {ii > 0 && <span className="text-[10px]" style={{ color: "var(--studio-text-muted)" }}>接上一镜</span>}
                {item.hold && (
                  <button
                    type="button"
                    className="ml-auto rounded px-1.5 text-[10.5px]"
                    style={{ color: "var(--studio-failed-fg)" }}
                    title="撤掉这个暂停位，批量接着往下出"
                    onClick={() => cinemaReleaseHold(jobId, item.id).then(setView, (e) => setError(String(e)))}
                  >
                    撤掉
                  </button>
                )}
              </div>
            ))}
          </li>
        ))}
        {/* 放到最后 */}
        {drag !== null && (
          <li
            onDragOver={(e) => {
              e.preventDefault();
              setOver(view.pending.length);
            }}
            onDrop={(e) => {
              e.preventDefault();
              drop(view.pending.length);
            }}
            className="h-5 rounded-md"
            style={{ borderTop: over === view.pending.length ? "2px solid var(--studio-primary)" : "2px solid transparent" }}
          />
        )}
      </ol>
      {error && <p className="text-[10.5px]" style={{ color: "var(--studio-failed-fg)", wordBreak: "break-word" }}>{error}</p>}
    </div>
  );
}
