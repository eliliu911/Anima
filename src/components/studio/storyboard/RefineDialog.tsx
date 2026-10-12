/**
 * 「细化时间线」：勾几场，把偏薄的镜头一镜一镜写细（后台任务）。
 *
 * 用户 2026-09-27：「同样 12 秒的视频……把时间线写好，描述细致，而不是一笔带过。」
 * 结构不动（时长、景别、运镜、台词、人物），只换时间段；细化过的镜头提示词过期，要重新合成。
 */
import { useState } from "react";
import type { PromptRoute, ShotIssue, StoryboardShot } from "../../../api/cinema";
import { sceneRows } from "./storyboardView";
import { needsRefine, refinePlan, shotDensity, TARGET_CPS } from "./timelineDensity";

/** 一镜大约多少秒（本机后台模型实测 20~40 秒，带重试按 40 算） */
const SECS_PER_SHOT = 40;

export function RefineDialog({
  shots,
  route,
  issues,
  onClose,
  onStart,
}: {
  shots: StoryboardShot[];
  route: PromptRoute;
  issues: ShotIssue[];
  onClose: () => void;
  onStart: (ids: string[]) => void;
}) {
  const rows = sceneRows(shots, route, issues);
  // 默认勾上有偏薄镜头的场
  const [sceneNos, setSceneNos] = useState<number[]>(() => rows.filter((r) => shots.some((s) => (s.scene_no ?? 0) === r.no && needsRefine(s))).map((r) => r.no));
  const [includeRendered, setIncludeRendered] = useState(false);
  const plan = refinePlan(shots, sceneNos, includeRendered);
  const minutes = Math.max(1, Math.round((plan.ids.length * SECS_PER_SHOT) / 60));
  const toggle = (no: number) => setSceneNos((cur) => (cur.includes(no) ? cur.filter((n) => n !== no) : [...cur, no].sort((a, b) => a - b)));
  const inScene = (no: number) => shots.filter((s) => (s.scene_no ?? 0) === no);
  const avg = (list: StoryboardShot[]) => {
    const secs = list.reduce((n, s) => n + (s.duration || 0), 0);
    return secs > 0 ? Math.round(list.reduce((n, s) => n + shotDensity(s) * (s.duration || 0), 0) / secs) : 0;
  };

  return (
    <div className="studio-scrim fixed inset-0 z-50 flex items-center justify-center">
      <div className="studio-modal flex max-h-[86vh] w-[440px] max-w-[94vw] flex-col overflow-hidden" onClick={(e) => e.stopPropagation()}>
        <header className="px-5 pb-3 pt-4" style={{ borderBottom: "1px solid var(--studio-border-subtle)" }}>
          <h2 className="studio-display text-[16px]">细化时间线{plan.ids.length ? ` · ${plan.ids.length} 镜` : ""}</h2>
          <p className="mt-0.5 text-[11.5px]" style={{ color: "var(--studio-text-muted)" }}>
            还是同一条视频，只把每一刻发生什么写满（目标每秒约 {TARGET_CPS} 字）。时长、景别、运镜、台词、人物都不改；细化过的镜头要重新合成提示词
          </p>
        </header>

        <div className="studio-scroll min-h-0 flex-1 overflow-y-auto px-5 py-4">
          <p className="mb-2 text-[12px]" style={{ color: "var(--studio-text-2)" }}>细化哪几场（可多选）</p>
          <div className="flex flex-col gap-1.5">
            {rows.map((r) => {
              const on = sceneNos.includes(r.no);
              const list = inScene(r.no);
              const thin = list.filter(needsRefine).length;
              return (
                <button
                  key={r.no}
                  type="button"
                  aria-pressed={on}
                  onClick={() => toggle(r.no)}
                  className="rounded-lg px-2 py-2 text-[12px]"
                  style={{
                    background: on ? "var(--studio-bg-elevated)" : "transparent",
                    color: on ? "var(--studio-text)" : "var(--studio-text-muted)",
                    boxShadow: `0 0 0 1px ${on ? "var(--studio-text-2)" : "var(--studio-border)"}`,
                  }}
                >
                  <span className="flex items-baseline gap-2 text-left">
                    <span className="font-mono">{on ? "☑" : "☐"} 第 {r.no} 场</span>
                    <span className="min-w-0 flex-1 truncate">{r.name}</span>
                    <span className="shrink-0 text-[10.5px]" style={{ color: "var(--studio-text-muted)" }}>
                      {list.length} 镜 · 偏薄 {thin} · 每秒约 {avg(list)} 字
                    </span>
                  </span>
                </button>
              );
            })}
          </div>
          <div className="mt-1.5 flex gap-3 text-[11px]">
            <button type="button" onClick={() => setSceneNos(rows.map((r) => r.no))} style={{ color: "var(--studio-text-2)" }}>全选</button>
            <button type="button" onClick={() => setSceneNos([])} style={{ color: "var(--studio-text-2)" }}>清空</button>
          </div>

          <label className="mt-4 flex items-center gap-1.5 text-[12px]">
            <input type="checkbox" checked={includeRendered} onChange={(e) => setIncludeRendered(e.target.checked)} />
            已经出过片的也细化（之后要重新合成、重出）
          </label>
          <p className="mt-2 text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
            {[
              plan.dense ? `${plan.dense} 镜已经够细，不动` : "",
              plan.rendered ? `${plan.rendered} 镜已经出过片，不动` : "",
              plan.ids.length ? `预计约 ${minutes} 分钟，可以先去别的地方` : "选中的场里没有要细化的镜头",
            ]
              .filter(Boolean)
              .join(" · ")}
          </p>
        </div>

        <footer className="flex justify-end gap-2 px-5 py-3" style={{ borderTop: "1px solid var(--studio-border-subtle)" }}>
          <button type="button" onClick={onClose} className="rounded-lg px-3 py-1.5 text-[12px]" style={{ color: "var(--studio-text-2)" }}>取消</button>
          <button
            type="button"
            disabled={plan.ids.length === 0}
            onClick={() => {
              onStart(plan.ids);
              onClose();
            }}
            className="rounded-lg px-3 py-1.5 text-[12px] font-medium"
            style={{ background: "var(--studio-primary)", color: "var(--studio-on-primary)", opacity: plan.ids.length === 0 ? 0.6 : 1 }}
          >
            开始细化{plan.ids.length ? ` ${plan.ids.length} 镜` : ""}
          </button>
        </footer>
      </div>
    </div>
  );
}
