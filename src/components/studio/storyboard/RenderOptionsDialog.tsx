import { useEffect, useState } from "react";
import { cinemaSaveModelSettings, type CinemaProject, type PromptRoute, type RenderQuality, type RenderSettings, type SceneRender, type ShotIssue, type ShotRouteInfo, type StoryboardShot } from "../../../api/cinema";
import { styleCatalog, styleGet } from "../../../api/style";
import { ASPECTS, DEFAULT_KEYFRAME_QUALITY, DEFAULT_RENDER, durationLabel, estimateLtxMinutes, estimateMinutes, KEYFRAME_QUALITIES, ltxSize, QUALITIES,
  NATIVE_REF2VA_LORA_NOTE, REFERENCE_QUALITIES, renderSize, RESOLUTIONS, shownQuality, VIDEO_ENGINES, videoEngineOf, type VideoEngine } from "./renderOptions";
import { batchTargets, renderBlocker, renderTakes, sceneRows, type BatchPlan } from "./storyboardView";
import { ROUTE_INFO } from "./SynthesizeDialog";

/** 这次出片出哪些：整集批量、按场（勾几场）、或某一镜。 */
export type RenderTarget = { kind: "batch" } | { kind: "scenes"; sceneNos: number[] } | { kind: "single"; shotId: string };

/**
 * 某条线路下这次能出哪几镜。批量：没出过的优先、都出过就全部重出；
 * 单镜：这一镜能出就是它。
 *
 * **批量会把勾了「整段出」的场摘成 `longTakeScenes`**，那几场整段出、
 * 不逐镜出（见 `batchTargets`）。单镜那条路不碰长镜头 ——
 * 只想重出某一镜时用的就是它。
 */
export function renderTargetsFor(
  target: RenderTarget,
  shots: StoryboardShot[],
  issues: ShotIssue[],
  route: PromptRoute,
  longTakeScenes: number[] = [],
  /** 整段出过的场（分集的 `scene_renders`）。这条线路出过的场不再排 */
  sceneRenders: readonly Pick<SceneRender, "scene_no" | "route">[] = [],
  /** 出过的也一起重出（见 `batchTargets`）。单镜不看它 */
  includeDone = false,
): BatchPlan {
  if (target.kind === "batch") return batchTargets(shots, route, issues, longTakeScenes, shots, sceneRenders, includeDone);
  if (target.kind === "scenes") {
    // 先把镜头限定在勾选的场里，其余规则（没出过的优先、长镜头整段出、没合成的不出）照批量走
    const picked = new Set(target.sceneNos);
    return batchTargets(
      shots.filter((s) => picked.has(s.scene_no ?? 0)),
      route,
      issues,
      longTakeScenes.filter((n) => picked.has(n)),
      shots,
      sceneRenders,
      includeDone,
    );
  }
  const shot = shots.find((s) => s.id === target.shotId);
  const ok = !!shot && renderBlocker(shot, route, issues, false) === null;
  return { ids: ok ? [target.shotId] : [], longTakeScenes: [], redo: !!shot && renderTakes(shot, route).length > 0, blocked: ok ? 0 : 1 };
}

/**
 * 出片前弹出的选项卡。**改的是项目级出片设置**（整集统一），点开始时先保存再出片。
 *
 * 批量出片和单镜出片都走它 —— 一个入口，免得两处各选各的。
 */
export function RenderOptionsDialog({
  project,
  target,
  shots,
  issues,
  routes = [],
  onClose,
  onStart,
  onSaved,
  longTakeScenes = [],
  sceneRenders = [],
}: {
  project: CinemaProject;
  target: RenderTarget;
  shots: StoryboardShot[];
  issues: ShotIssue[];
  /** 每一镜走首帧出片还是参考出片（Rust `shot_route`）。这一批按它分两组、两组各用各的画质 */
  routes?: ShotRouteInfo[];
  /** 勾了「整段出」的那几场。批量出片时它们整段出，不逐镜出 */
  longTakeScenes?: number[];
  /** 整段出过的场。这条线路出过的不再排 */
  sceneRenders?: readonly Pick<SceneRender, "scene_no" | "route">[];
  onClose: () => void;
  /** 选好之后要出的镜头（按所选线路算出来的） */
  /** 选好之后这次出什么。**整个计划传出去** —— 只传镜头 id 的话，
   *  长镜头的那几场就得由调用方再算一遍，两边必然漂移 */
  onStart: (plan: BatchPlan) => void;
  onSaved: (p: CinemaProject) => void;
}) {
  const [draft, setDraft] = useState<RenderSettings>({ ...DEFAULT_RENDER, ...project.model_settings?.render });
  const currentRoute: PromptRoute = project.model_settings?.prompt_route ?? "b";
  const [route, setRoute] = useState<PromptRoute>(currentRoute);
  // 按场出片：勾哪几场是对话框里的本地状态，计划跟着勾选算
  const [sceneNos, setSceneNos] = useState<number[]>(target.kind === "scenes" ? target.sceneNos : []);
  const effTarget: RenderTarget = target.kind === "scenes" ? { kind: "scenes", sceneNos } : target;
  const rows = target.kind === "scenes" ? sceneRows(shots, route, issues, sceneRenders) : [];
  const toggleScene = (no: number) =>
    setSceneNos((cur) => (cur.includes(no) ? cur.filter((n) => n !== no) : [...cur, no].sort((a, b) => a - b)));
  // **出片引擎和生成设置里的「动效」是同一个字段**（用户 2026-10-06：不用两边都查看设置）
  const [engine, setEngine] = useState<VideoEngine>(videoEngineOf(project.model_settings?.video_engine));
  const ltx = engine === "ltx";
  // 出过的要不要一起重出（批量 / 按场）。默认不，和以前一样只出没出过的
  const [includeDone, setIncludeDone] = useState(false);
  const freshPlan = renderTargetsFor(effTarget, shots, issues, route, longTakeScenes, sceneRenders);
  const fullPlan = includeDone ? renderTargetsFor(effTarget, shots, issues, route, longTakeScenes, sceneRenders, true) : freshPlan;
  // 有没出过的、也有出过的，才需要问；全都出过了本来就是「全部重出」
  const askDone = target.kind !== "single" && !freshPlan.redo && (freshPlan.done ?? 0) > 0;
  // LTX 只认首帧：没首帧的镜头后端会报「尚未生成关键帧」—— 不排进这一批，并说出来
  const noFrame = ltx ? fullPlan.ids.filter((id) => !shots.find((s) => s.id === id)?.keyframes?.first) : [];
  const plan: BatchPlan = noFrame.length ? { ...fullPlan, ids: fullPlan.ids.filter((id) => !noFrame.includes(id)) } : fullPlan;
  const picked = shots.filter((s) => plan.ids.includes(s.id));
  const single = target.kind === "single" ? shots.find((s) => s.id === target.shotId) : null;
  const title = single
    ? `第 ${single.shot_num} 镜${plan.redo ? "重新出片" : "出片"}`
    : target.kind === "scenes"
      ? `按场出片 · ${sceneNos.length ? `${sceneNos.length} 场 ${plan.ids.length} 镜` : "先勾场"}`
      : `${plan.redo ? "全部重出" : "批量出片"} · ${plan.ids.length} 镜`;
  const [style, setStyle] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    void Promise.all([styleGet(project.id), styleCatalog()])
      .then(([art, cat]) => {
        if (!alive) return;
        const preset = cat.presets.find((p) => p.id === art.preset_id);
        setStyle(preset ? preset.name_zh : art.positive_prompt.trim() ? "自定义风格" : "");
      })
      .catch(() => alive && setStyle(""));
    return () => {
      alive = false;
    };
  }, [project.id]);

  const set = <K extends keyof RenderSettings>(k: K, v: RenderSettings[K]) => setDraft((d) => ({ ...d, [k]: v }));
  const [w, h] = ltx ? ltxSize(project.aspect_ratio) : renderSize(draft, project.aspect_ratio);
  // **这一批按链路分两组**：有首帧 = 首帧出片（用首帧那一栏的画质），其余参考出片
  const routeOf = (s: StoryboardShot) => routes.find((r) => r.shot_id === s.id);
  const isKeyframe = (s: StoryboardShot) => (routeOf(s)?.actual ?? (s.keyframes?.first ? "keyframe" : "reference")) === "keyframe";
  const kfShots = picked.filter(isKeyframe);
  const refShots = picked.filter((s) => !isKeyframe(s));
  // 建议首帧出片、但还没首帧的：这次按参考出（出首帧之后才会走首帧）
  const waitingFrame = refShots.filter((s) => routeOf(s)?.advised === "keyframe");
  const kfQuality: RenderQuality = shownQuality(draft.keyframe_quality ?? DEFAULT_KEYFRAME_QUALITY);
  const refQuality: RenderQuality = shownQuality(draft.quality);
  const secsOf = (xs: StoryboardShot[]) => xs.reduce((a, s) => a + (s.duration || 0), 0);
  const kfEst = estimateMinutes(draft, secsOf(kfShots), kfQuality, "keyframe");
  const refEst = estimateMinutes(draft, secsOf(refShots), refQuality, "reference");
  const est = ltx
    ? estimateLtxMinutes(picked.map((s) => s.duration || 0), draft.takes)
    : { minutes: kfEst.minutes + refEst.minutes, measured: (kfShots.length === 0 || kfEst.measured) && (refShots.length === 0 || refEst.measured) };
  const [advanced, setAdvanced] = useState(false);

  const start = async () => {
    setBusy(true);
    setError(null);
    try {
      const saved = await cinemaSaveModelSettings(project.id, {
        t2i_engine: "zimage",
        i2i_engine: "zimage",
        character_aspect: "portrait",
        scene_aspect: "landscape",
        prop_aspect: "square",
        ...project.model_settings,
        video_engine: engine,
        render: draft,
        // 出片发的是项目当前线路的提示词：选了哪条就存成当前线路
        prompt_route: route,
      });
      onSaved(saved);
      onStart(plan);
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const qualityInfo = (id: RenderQuality) => QUALITIES.find((q) => q.id === id);
  /** 一栏画质：首帧出片 / 参考出片各一栏，各自显示用的是哪个加速 LoRA */
  const qualityColumn = (
    title: string,
    count: number,
    options: { id: RenderQuality; tag?: string }[],
    value: RenderQuality,
    onPick: (q: RenderQuality) => void,
    note?: string,
  ) => (
    <div className="flex min-w-0 flex-col gap-1.5 rounded-xl p-3" style={{ boxShadow: "0 0 0 1px var(--studio-border)", opacity: count ? 1 : 0.55 }}>
      <p className="text-[12px] font-medium" style={{ color: "var(--studio-text)" }}>
        {title} <span style={{ color: "var(--studio-text-muted)" }}>· {count ? `这一批 ${count} 镜` : "这一批没有"}</span>
      </p>
      {options.map((o) => {
        const q = qualityInfo(o.id);
        return (
          <Choice key={o.id} on={value === o.id} onClick={() => onPick(o.id)} title={q?.lora}>
            <span className="flex items-baseline justify-between gap-2 text-left">
              <span>{q?.label}</span>
              {o.tag && <span className="text-[10px]" style={{ color: "var(--studio-primary)" }}>{o.tag}</span>}
            </span>
            <span className="block text-left text-[10.5px]" style={{ color: "var(--studio-text-muted)" }}>{q?.hint}</span>
          </Choice>
        );
      })}
      <p className="text-[10.5px] leading-snug" style={{ color: "var(--studio-text-2)" }}>{qualityInfo(value)?.lora}</p>
      {note && <p className="text-[10.5px] leading-snug" style={{ color: "var(--studio-text-muted)" }}>{note}</p>}
    </div>
  );

  return (
    <div className="studio-scrim fixed inset-0 z-50 flex items-center justify-center">
      <div className="studio-modal flex max-h-[88vh] w-[580px] max-w-[94vw] flex-col overflow-hidden" onClick={(e) => e.stopPropagation()}>
        <header className="px-5 pb-3 pt-4" style={{ borderBottom: "1px solid var(--studio-border-subtle)" }}>
          <h2 className="studio-display text-[16px]">{title}</h2>
          <p className="mt-0.5 text-[11.5px]" style={{ color: "var(--studio-text-muted)" }}>
            出片设置整个项目统一，改了之后单镜和批量都按这一套。每一镜走首帧出片还是参考出片由分镜自动定
          </p>
        </header>

        <div className="studio-scroll min-h-0 flex-1 overflow-y-auto px-5 py-4">
          {target.kind === "scenes" && (
            <Group label="出哪几场（可多选）">
              <div className="flex flex-col gap-1.5">
                {rows.map((r) => {
                  const on = sceneNos.includes(r.no);
                  return (
                    <Choice key={r.no} on={on} onClick={() => toggleScene(r.no)} title={r.name}>
                      <span className="flex items-baseline gap-2 text-left">
                        <span className="font-mono">{on ? "☑" : "☐"} 第 {r.no} 场</span>
                        <span className="min-w-0 flex-1 truncate">{r.name}</span>
                        <span className="shrink-0 text-[10.5px]" style={{ color: "var(--studio-text-muted)" }}>
                          {r.total} 镜 · 已出 {r.rendered}
                          {r.ready < r.total ? ` · ${r.total - r.ready} 镜没合成` : ""}
                        </span>
                      </span>
                    </Choice>
                  );
                })}
              </div>
              <div className="mt-1.5 flex gap-3 text-[11px]">
                <button type="button" onClick={() => setSceneNos(rows.map((r) => r.no))} style={{ color: "var(--studio-text-2)" }}>全选</button>
                <button type="button" onClick={() => setSceneNos([])} style={{ color: "var(--studio-text-2)" }}>清空</button>
                <span style={{ color: "var(--studio-text-muted)" }}>同一场里设了「接上一镜」的，会按镜号顺序出</span>
              </div>
            </Group>
          )}

          {askDone && (
            <Group label="出哪些">
              <div className="flex gap-2">
                <Choice on={!includeDone} onClick={() => setIncludeDone(false)}>只出没出过的 {freshPlan.ids.length + freshPlan.longTakeScenes.length} 个</Choice>
                <Choice on={includeDone} onClick={() => setIncludeDone(true)}>连出过的 {freshPlan.done} 个一起重出</Choice>
              </div>
              <p className="mt-1 text-[11px]" style={{ color: "var(--studio-text-muted)" }}>重出是新增一条，旧的片子留着，可以在镜头详情里挑</p>
            </Group>
          )}

          {/* ① 这一批：按链路分好、哪些被拦 —— 打开就知道这次出什么 */}
          <section className="mb-4 rounded-xl px-3 py-2.5 text-[12px]" style={{ background: "var(--studio-bg-inset)" }}>
            <p style={{ color: "var(--studio-text)" }}>
              这一批 <b>{plan.ids.length}</b> 镜
              {!ltx && <>：首帧出片 <b>{kfShots.length}</b> · 参考出片 <b>{refShots.length}</b></>}
              {plan.longTakeScenes.length > 0 && <> · 长镜头 <b>{plan.longTakeScenes.length}</b> 场（按参考出）</>}
            </p>
            {waitingFrame.length > 0 && !ltx && (
              <p className="mt-0.5 text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
                其中第 {waitingFrame.map((s) => s.shot_num).join("、")} 镜按分镜内容该首帧出片、但还没出首帧：这次按参考出（先「按场出首帧」再出片就会走首帧）
              </p>
            )}
            {plan.blocked > 0 && (
              <p className="mt-0.5 text-[11px]" style={{ color: "var(--studio-accent)" }}>
                另有 {plan.blocked} 镜出不了、不在这一批里（没合成 / 过期，或首帧里缺中途进场的人 —— 在镜头列表里标红）
              </p>
            )}
            {(plan.chained ?? []).length > 0 && (
              <div className="mt-0.5 text-[11px]" style={{ color: "var(--studio-accent)" }}>
                <p>这几镜设了「接上一镜」，上一镜这次出不了，也不在这一批里：</p>
                {(plan.chained ?? []).map((c) => <p key={c}>· {c}</p>)}
              </div>
            )}
            {noFrame.length > 0 && (
              <p className="mt-0.5 text-[11px]" style={{ color: "var(--studio-accent)" }}>LTX 只能照首帧出：另有 {noFrame.length} 镜还没首帧，不在这一批里</p>
            )}
          </section>

          {ltx ? (
            <p className="mb-4 rounded-lg px-3 py-2 text-[11px]" style={{ color: "var(--studio-text-muted)", boxShadow: "0 0 0 1px var(--studio-border)" }}>
              现在用的是 LTX（在下面「高级」里换）：不看画质、清晰度和比例，固定按项目画幅（{project.aspect_ratio}）、长边 1280 出，这一批是 {w}×{h}。
            </p>
          ) : (
            <>
              {/* ② 画质：两条链路各选各的（2026-10-07 用户：要区分首帧生和参考生的画质选项） */}
              <Group label="画质（按出片方式分开选）">
                <div className="grid grid-cols-2 gap-3">
                  {qualityColumn("首帧出片", kfShots.length, KEYFRAME_QUALITIES, kfQuality, (q) => set("keyframe_quality", q), "只吃首帧：人、衣服、道具都要在首帧里画对")}
                  {qualityColumn("参考出片", refShots.length + plan.longTakeScenes.length, REFERENCE_QUALITIES, refQuality, (q) => set("quality", q), NATIVE_REF2VA_LORA_NOTE)}
                </div>
              </Group>

              {/* ③ 画面 */}
              <Group label="画面">
                <div className="grid grid-cols-3 gap-2">
                  {RESOLUTIONS.map((r) => (
                    <Choice key={r.id} on={draft.resolution === r.id} onClick={() => set("resolution", r.id)}>{r.label}</Choice>
                  ))}
                </div>
                <div className="mt-2 grid grid-cols-6 gap-2">
                  {ASPECTS.map((a) => (
                    <Choice key={a} on={(draft.aspect || project.aspect_ratio) === a} onClick={() => set("aspect", a)}>{a}</Choice>
                  ))}
                </div>
                <p className="mt-1 text-[11px]" style={{ color: "var(--studio-text-muted)" }}>出片尺寸 {w}×{h}</p>
              </Group>
            </>
          )}

          {/* ④ 其他 */}
          <Group label="其他">
            <div className="grid grid-cols-3 gap-2">
              {[1, 2, 3].map((n) => (
                <Choice key={n} on={draft.takes === n} onClick={() => set("takes", n)}>每镜 {n} 次</Choice>
              ))}
            </div>
            {draft.takes > 1 && (
              <p className="mt-1 text-[11px]" style={{ color: "var(--studio-text-muted)" }}>每次换一个种子，出完挑一条用；耗时跟着翻倍</p>
            )}
            <div className="mt-2 grid grid-cols-2 gap-2">
              <Choice on={draft.generate_audio} onClick={() => set("generate_audio", !draft.generate_audio)}>
                {draft.generate_audio ? "生成声音 · 开" : "生成声音 · 关"}
              </Choice>
              <Choice on={draft.burn_subtitles} onClick={() => set("burn_subtitles", !draft.burn_subtitles)}>
                {draft.burn_subtitles ? "烧入字幕 · 开" : "烧入字幕 · 关"}
              </Choice>
            </div>
            {ltx && draft.generate_audio && (
              <p className="mt-1 text-[11px]" style={{ color: "var(--studio-accent)" }}>
                LTX 念不了中文台词：出来只有环境声或含糊人声，台词要在组装页「配音」补
              </p>
            )}
            <p className="mt-2 text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
              风格：{style === null ? "读取中…" : style || "没选（去「风格」那一步选）"} · 改了风格要先重新合成，出片会拦下没合成或过期的镜头
            </p>
          </Group>

          {/* ⑤ 高级：很少改，默认折起只显示当前值 */}
          <section className="mb-1">
            <button type="button" onClick={() => setAdvanced((v) => !v)} className="text-[12px]" style={{ color: "var(--studio-text-2)" }}>
              {advanced ? "▾" : "▸"} 高级 · 出片引擎 {VIDEO_ENGINES.find((e) => e.id === engine)?.label} · 提示词{ROUTE_INFO[route].name.split(" · ")[0]}
            </button>
            {advanced && (
              <div className="mt-2">
                <Group label="出片引擎（和生成设置里的「动效」是同一项）">
                  <div className="grid grid-cols-2 gap-2">
                    {VIDEO_ENGINES.map((e) => (
                      <Choice key={e.id} on={engine === e.id} onClick={() => setEngine(e.id)} title={e.hint}>
                        {e.label}
                        <span className="block text-[10.5px]" style={{ color: "var(--studio-text-muted)" }}>{e.hint}</span>
                      </Choice>
                    ))}
                  </div>
                </Group>
                <Group label="提示词线路">
                  <div className="grid grid-cols-2 gap-2">
                    {(["b", "a"] as PromptRoute[]).map((r) => {
                      const p = renderTargetsFor(effTarget, shots, issues, r, longTakeScenes, sceneRenders, includeDone);
                      const n = p.ids.length;
                      return (
                        <Choice key={r} on={route === r} onClick={() => setRoute(r)} title={`${ROUTE_INFO[r].how}（${ROUTE_INFO[r].status}）`}>
                          {ROUTE_INFO[r].name.replace(" · ", "：")}
                          <span className="block text-[10.5px]" style={{ color: "var(--studio-text-muted)" }}>
                            {p.longTakeScenes.length ? `可出 ${p.longTakeScenes.length} 场长镜头${n ? ` + ${n} 镜` : ""}` : n ? `可出 ${n} 镜` : "没有能出的，先合成"}
                          </span>
                        </Choice>
                      );
                    })}
                  </div>
                </Group>
              </div>
            )}
          </section>
        </div>

        <footer className="flex items-center gap-3 px-5 py-3" style={{ borderTop: "1px solid var(--studio-border-subtle)" }}>
          <div className="min-w-0 flex-1 text-[11.5px]" style={{ color: "var(--studio-text-muted)" }}>
            <p>
              {plan.ids.length} 镜 · {w}×{h} · {durationLabel(est.minutes)}
              {est.measured ? "（按实测）" : "（估算）"}
            </p>
            {!ltx && kfShots.length > 0 && refShots.length > 0 && (
              <p>首帧 {kfShots.length} 镜 {durationLabel(kfEst.minutes)} + 参考 {refShots.length} 镜 {durationLabel(refEst.minutes)}</p>
            )}
            {error && <p style={{ color: "var(--studio-failed-fg)" }}>{error}</p>}
          </div>
          <button type="button" onClick={onClose} className="rounded-lg px-3 py-1.5 text-[12px]" style={{ color: "var(--studio-text-2)" }}>取消</button>
          <button
            type="button"
            disabled={busy || plan.ids.length === 0}
            onClick={() => void start()}
            className="rounded-lg px-3 py-1.5 text-[12px] font-medium"
            style={{ background: "var(--studio-primary)", color: "var(--studio-on-primary)", opacity: busy || plan.ids.length === 0 ? 0.6 : 1 }}
          >
            开始出片
          </button>
        </footer>
      </div>
    </div>
  );
}

function Group({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <section className="mb-4">
      <p className="mb-2 text-[12px]" style={{ color: "var(--studio-text-2)" }}>{label}</p>
      {children}
    </section>
  );
}

function Choice({ on, onClick, title, children }: { on: boolean; onClick: () => void; title?: string; children: React.ReactNode }) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={title}
      aria-pressed={on}
      className="rounded-lg px-2 py-2 text-[12px]"
      style={{
        background: on ? "var(--studio-bg-elevated)" : "transparent",
        color: on ? "var(--studio-text)" : "var(--studio-text-muted)",
        boxShadow: `0 0 0 1px ${on ? "var(--studio-text-2)" : "var(--studio-border)"}`,
      }}
    >
      {children}
    </button>
  );
}
