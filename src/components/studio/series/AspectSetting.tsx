import { useState } from "react";
import { cinemaSetAspectRatio, type CinemaProject, type EpisodeData } from "../../../api/cinema";
import { ASPECTS } from "../storyboard/renderOptions";
import { ASPECT_LABEL, renderedOnAspect } from "./overviewStats";

/**
 * **整部剧的画幅**，放在概览最上面。
 *
 * 以前画幅只能在新建项目时选，之后界面上没有任何地方能改，只在项目页右上角显示一行灰字 ——
 * 《回收师》建成了 9:16，出了一版竖屏预告才发现整部剧要 16:9（2026-09-26 用户：
 * 「应该在概览里把这个剧集画幅设置露出来，在概览里就把他们处理好」）。
 *
 * 选项和出片设置里的比例是同一组（`renderOptions.ts` 的 `ASPECTS`，含 21:9 宽银幕）。
 * 已经按当前画幅出过首帧 / 视频时，**换之前先说出来**：提示词不用重合成（画幅不进提示词），
 * 但那些画面尺寸对不上，要重出。
 */
export function AspectSetting({
  project,
  episodes,
  onChanged,
}: {
  project: CinemaProject;
  episodes: EpisodeData[];
  onChanged: () => void;
}) {
  const [confirm, setConfirm] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const done = renderedOnAspect(episodes);
  const current = project.aspect_ratio;

  const apply = async (a: string) => {
    setBusy(true);
    setError(null);
    try {
      await cinemaSetAspectRatio(project.id, a);
      setConfirm(null);
      onChanged();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const pick = (a: string) => {
    if (a === current || busy) return;
    if (done.frames + done.videos > 0) setConfirm(a);
    else void apply(a);
  };

  return (
    <section className="mb-5 rounded-xl px-4 py-3" style={{ background: "var(--studio-bg-inset)" }}>
      <div className="flex flex-wrap items-center gap-3">
        <div className="min-w-0">
          <p className="text-[12.5px] font-medium">
            画幅 <span className="font-mono">{current}</span>
            {ASPECT_LABEL[current] && (
              <span className="ml-1.5 text-[11.5px] font-normal" style={{ color: "var(--studio-text-muted)" }}>
                {ASPECT_LABEL[current]}
              </span>
            )}
          </p>
          <p className="mt-0.5 text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
            整部剧的首帧、出片、成片都按这个比例；合成好的提示词不受影响
          </p>
        </div>
        <div className="ml-auto flex flex-wrap gap-1.5">
          {ASPECTS.map((a) => {
            const on = a === current;
            return (
              <button
                key={a}
                type="button"
                disabled={busy}
                onClick={() => pick(a)}
                title={ASPECT_LABEL[a] ? `${a} ${ASPECT_LABEL[a]}` : a}
                className="rounded-lg px-2.5 py-1 font-mono text-[11.5px]"
                style={{
                  background: on ? "color-mix(in srgb, var(--studio-primary) 16%, transparent)" : "var(--studio-bg-base)",
                  color: on ? "var(--studio-primary)" : "var(--studio-text-2)",
                  boxShadow: on ? "0 0 0 1px var(--studio-primary)" : "0 0 0 1px var(--studio-border-subtle)",
                }}
              >
                {a}
              </button>
            );
          })}
        </div>
      </div>
      {confirm && (
        <div
          className="mt-2.5 flex flex-wrap items-center gap-2 rounded-lg px-3 py-2 text-[11.5px]"
          style={{ background: "var(--studio-pending-bg)", color: "var(--studio-pending-fg)" }}
        >
          <span className="min-w-0 flex-1">
            已经按 {current} 出了 {done.frames} 张首帧、{done.videos} 条视频 —— 换成 {confirm} 之后它们尺寸对不上，要重出。
          </span>
          <button type="button" disabled={busy} onClick={() => void apply(confirm)} className="rounded-md px-2.5 py-1 font-medium underline">
            {busy ? "改中…" : `换成 ${confirm}`}
          </button>
          <button type="button" onClick={() => setConfirm(null)} className="rounded-md px-2 py-1">
            先不换
          </button>
        </div>
      )}
      {error && (
        <p className="mt-2 text-[11px]" style={{ color: "var(--studio-failed-fg)" }}>
          {error}
        </p>
      )}
    </section>
  );
}
