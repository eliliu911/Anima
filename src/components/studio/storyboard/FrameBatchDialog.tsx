/**
 * 「按场出首帧」：勾几场，一镜一镜出首帧（后台任务）。
 *
 * 出首帧挂不挂资产图取决于项目的首帧模型：参考生图引擎挂场景、人物、道具；文生图一张不挂 ——
 * 那样场景和道具就是模型编的（E01 第 1 镜就是这么错的），所以这里把当前模型摆出来，一键换。
 */
import { useEffect, useState } from "react";
import { cinemaSaveModelSettings, type CinemaProject, type PromptRoute, type ShotIssue, type ShotRouteInfo, type StoryboardShot } from "../../../api/cinema";
import { studioCapabilities, type EngineSpec } from "../../../api/studio";
import { framePlan } from "./keyframes";
import { sceneRows } from "./storyboardView";

/** 参考生图、槽最多的那个。首帧模型是文生图时推荐换成它 */
export const FRAME_REF_ENGINE = "qwen21_edit";

export function FrameBatchDialog({
  project,
  shots,
  issues,
  routes = [],
  onClose,
  onStart,
  onSaved,
}: {
  project: CinemaProject;
  shots: StoryboardShot[];
  issues: ShotIssue[];
  /** 每镜的出片建议：建议参考出片的不出首帧（后端也这么筛，这里只为把数字算对） */
  routes?: ShotRouteInfo[];
  onClose: () => void;
  onStart: (ids: string[], redo: boolean) => void;
  onSaved: (p: CinemaProject) => void;
}) {
  const route: PromptRoute = project.model_settings?.prompt_route ?? "b";
  const rows = sceneRows(shots, route, issues);
  const [sceneNos, setSceneNos] = useState<number[]>([]);
  const [redo, setRedo] = useState(false);
  const [engines, setEngines] = useState<EngineSpec[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    studioCapabilities().then((c) => setEngines(c.engines.filter((e) => e.kind === "image"))).catch(() => setEngines([]));
  }, []);

  const engineId = project.model_settings?.i2i_engine ?? "zimage";
  const engine = engines.find((e) => e.id === engineId);
  const refEngine = engines.find((e) => e.id === FRAME_REF_ENGINE);
  const textOnly = !!engine && engine.reference_slots === 0;
  const byRef = new Set(routes.filter((r) => r.advised === "reference").map((r) => r.shot_id));
  const plan = framePlan(shots, sceneNos, route, redo, byRef);
  const minutes = Math.round((plan.ids.length * (engine?.secs ?? 300)) / 60);
  const toggle = (no: number) => setSceneNos((cur) => (cur.includes(no) ? cur.filter((n) => n !== no) : [...cur, no].sort((a, b) => a - b)));

  const switchEngine = async () => {
    if (!project.model_settings) return;
    setBusy(true);
    setError(null);
    try {
      onSaved(await cinemaSaveModelSettings(project.id, { ...project.model_settings, i2i_engine: FRAME_REF_ENGINE }));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const hasShots = (no: number) => shots.filter((s) => (s.scene_no ?? 0) === no);
  return (
    <div className="studio-scrim fixed inset-0 z-50 flex items-center justify-center">
      <div className="studio-modal flex max-h-[86vh] w-[440px] max-w-[94vw] flex-col overflow-hidden" onClick={(e) => e.stopPropagation()}>
        <header className="px-5 pb-3 pt-4" style={{ borderBottom: "1px solid var(--studio-border-subtle)" }}>
          <h2 className="studio-display text-[16px]">按场出首帧{sceneNos.length ? ` · ${sceneNos.length} 场 ${plan.ids.length} 镜` : ""}</h2>
          <p className="mt-0.5 text-[11.5px]" style={{ color: "var(--studio-text-muted)" }}>
            有了首帧，出片就从这张图开始动（首帧图生视频）；再出或导入尾帧就是首尾帧。出完会自动按首帧合成一次提示词
          </p>
        </header>

        <div className="studio-scroll min-h-0 flex-1 overflow-y-auto px-5 py-4">
          <section className="mb-4">
            <p className="mb-2 text-[12px]" style={{ color: "var(--studio-text-2)" }}>首帧模型</p>
            <p className="text-[12px]">
              {engine?.label ?? engineId}
              <span className="ml-1.5 text-[11px]" style={{ color: textOnly ? "var(--studio-accent)" : "var(--studio-text-muted)" }}>
                {textOnly ? "文生图，不挂资产图 —— 场景、道具、脸都是模型按文字编的" : engine ? `挂这一镜的场景、人物、道具图（最多 ${engine.reference_slots} 张）` : ""}
              </span>
            </p>
            {textOnly && refEngine && (
              <button
                type="button"
                disabled={busy}
                onClick={() => void switchEngine()}
                className="mt-1.5 rounded-md px-2.5 py-1 text-[11.5px]"
                style={{ background: "var(--studio-bg-elevated)", boxShadow: "0 0 0 1px var(--studio-border)", color: "var(--studio-text)" }}
              >
                换成{refEngine.label}（约 {Math.round(refEngine.secs / 60)} 分钟一张）
              </button>
            )}
          </section>

          <section className="mb-4">
            <p className="mb-2 text-[12px]" style={{ color: "var(--studio-text-2)" }}>出哪几场（可多选）</p>
            <div className="flex flex-col gap-1.5">
              {rows.map((r) => {
                const on = sceneNos.includes(r.no);
                const withFrame = hasShots(r.no).filter((s) => s.keyframes?.first).length;
                const refCount = hasShots(r.no).filter((s) => byRef.has(s.id)).length;
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
                        {r.total} 镜{refCount ? ` · 参考出片 ${refCount}` : ""} · 有首帧 {withFrame}
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
          </section>

          <label className="flex items-center gap-1.5 text-[12px]">
            <input type="checkbox" checked={redo} onChange={(e) => setRedo(e.target.checked)} />
            已经有首帧的也重出（会盖掉导入的图）
          </label>
          <p className="mt-2 text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
            {[
              plan.reference ? `${plan.reference} 镜走参考出片，不出首帧` : "",
              plan.kept ? `${plan.kept} 镜已有首帧，不动` : "",
              plan.composed ? `${plan.composed} 镜还没合成提示词：按镜头表直接出（不花 token），出完自动合成一次` : "",
              plan.ids.length ? `预计约 ${minutes} 分钟` : "",
            ].filter(Boolean).join(" · ")}
          </p>
          {error && <p className="mt-1 text-[11px]" style={{ color: "var(--studio-failed-fg)" }}>{error}</p>}
        </div>

        <footer className="flex justify-end gap-2 px-5 py-3" style={{ borderTop: "1px solid var(--studio-border-subtle)" }}>
          <button type="button" onClick={onClose} className="rounded-lg px-3 py-1.5 text-[12px]" style={{ color: "var(--studio-text-2)" }}>取消</button>
          <button
            type="button"
            disabled={busy || plan.ids.length === 0}
            onClick={() => {
              onStart(plan.ids, redo);
              onClose();
            }}
            className="rounded-lg px-3 py-1.5 text-[12px] font-medium"
            style={{ background: "var(--studio-primary)", color: "var(--studio-on-primary)", opacity: busy || plan.ids.length === 0 ? 0.6 : 1 }}
          >
            出 {plan.ids.length} 镜首帧
          </button>
        </footer>
      </div>
    </div>
  );
}
