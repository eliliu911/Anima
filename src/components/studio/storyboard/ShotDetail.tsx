/**
 * 分镜页右侧：当前镜头的详情。字段都能改，**失焦就存**。
 *
 * 字段是两条提示词线路的共用输入（见 `shot_table`），这里不拼任何提示词。
 * 当前线路合成出来的提示词**可以直接改**（出片时原样发送改过的版本）；另一条线路只读。
 */
import { useRef, useState } from "react";
import type { AssetView } from "../../../api/assets";
import { cinemaPickTake, COMBAT_STYLES, type CombatStyle, type PromptRoute, type RefSlot, type ShotIssue, type ShotRouteInfo, type ShotSegment, type StoryboardShot } from "../../../api/cinema";
import { withDuration } from "./retime";
import { longShotHint, MAX_LONG_SHOT_SECS } from "./longShot";
import type { IssueFixOffer } from "../../../api/cinema";
import { display, group } from "../../../shared/shotLanguage";
import { CONTINUITY_HINT, CONTINUITY_LABEL, continuityAdvice, effectiveContinuity, type Continuity } from "./continuity";
import { MediaThumb } from "../MediaThumb";
import { effectiveLook, needsSceneRegen, renderBlocker, renderButtonLabel, renderTakes, secs, takesNewestFirst, type PromptRound } from "./storyboardView";
import { NsfwThumb } from "../library/NsfwThumb";
import { KeyframeBlock } from "./KeyframeBlock";
import { DirectorBlock } from "./DirectorBlock";
import { addSlot, defaultUsage, MANUAL_USAGES, removeSlot } from "./refSlots";
import { withoutLegacyDirectorMarkers } from "../../../canvas/pano/directorLinks";
import { shotDensity, TARGET_CPS, THIN_CPS, thinSegments } from "./timelineDensity";
import { stageLines } from "./sceneStage";
import { SegmentContracts } from "./SegmentContracts";

/** 远景 / 全景：五官看不清（和 Rust `timeline_lint::is_wide` 同一组词） */
const isWideScale = (scale?: string) => /远景|全景|wide|extreme long/i.test(scale ?? "");

const USAGE_LABEL: Record<RefSlot["usage"], string> = {
  identity: "身份",
  outfit: "造型状态",
  geography: "地理",
  composition: "构图",
  scale: "尺度",
  effect: "效果",
  start_frame: "起始帧",
  end_frame: "结束帧",
  style: "风格",
};

const INPUT = "w-full min-w-0 rounded-md px-2 py-1 text-[12px] outline-none";
const INPUT_STYLE = { background: "var(--studio-bg-input)", color: "var(--studio-text)" } as const;

type DirectorBinding = { name: string; payload: { camera?: { position?: number[]; yawDeg?: number; pitchDeg?: number; fovDeg?: number }; path?: { name?: string; durationSec?: number; cameras?: unknown[] } | null }; motionNote?: string };
function directorBinding(shot: StoryboardShot): DirectorBinding | null {
  if (shot.director_camera) return { name: shot.director_camera.name, payload: shot.director_camera, motionNote: shot.director_camera.motion_note };
  const line = [...(shot.camera_note ?? "").split("\n")].reverse().find((x) => x.startsWith("[导演机位:"));
  if (!line) return null;
  const m = line.match(/^\[导演机位:([^\]]+)\]\s*(\{.*\})$/);
  if (!m) return null;
  try { return { name: m[1], payload: JSON.parse(m[2]) }; } catch { return null; }
}
export function manualCameraNote(note: string): string { return withoutLegacyDirectorMarkers(note); }

/** 修复按钮的悬浮说明：说清点了会改什么 */
function fixTitle(offer: IssueFixOffer): string {
  const f = offer.fix;
  if (f.kind === "duration")
    return `把这一镜改成 ${f.secs} 秒，分段时间线按比例拉长。改完要重新合成提示词${f.secs > 15 ? "。超过 15 秒是超长镜：出片时自动分段、一次出完接成一条" : ""}`;
  if (f.kind === "speaker")
    return `把这句台词改成「${f.speaker}」说${f.off_screen ? "，并标成画外音（这一镜的出场人物和画面里都没有他）" : ""}。改完要重新合成提示词`;
  if (f.kind === "resynth") return "按当前线路重新合成这一镜：写手会被要求只写一个确定的版本（英文句子代码改不准，交给写手改）";
  if (f.kind === "mount") return `挂成这一镜的参考图（控制：${f.control}），挂在最后一张后面。挂上之后要重新合成提示词`;
  return "夹英文、切镜词这类由代码直接改掉，整集修一遍";
}

/** 这一镜所在那一场整段出的长镜头。 */
export interface LongTakeClipInfo {
  sceneNo: number;
  /** 这一场的头尾镜号 */
  first: number;
  last: number;
  path: string;
  route: PromptRoute;
  created_at: number;
  quality?: string;
}

export function ShotDetail({
  shot,
  prev,
  assets,
  issues,
  routeInfo = null,
  route,
  onSave,
  synthesizing,
  round,
  onSynthesize,
  videoRunning,
  rendering,
  queued = null,
  synthQueued = null,
  onRender,
  regenerating,
  onRegenerateScene,
  fixes = [],
  onRepairAll,
  defaultLooks,
  projectId,
  episodeId,
  onPicked,
  longTake = false,
  longTakeClip = null,
  frameRunning = false,
  onOpenDirector,
  refining = false,
  refineRound = null,
  onRefine,
}: {
  /** 这一集有细化时间线的任务在跑（同一集同时只能一个） */
  refining?: boolean;
  /** 这一镜在当前细化任务里的状态 */
  refineRound?: "running" | "queued" | null;
  /** 细化这一镜的时间线（把每一刻写满，结构不动） */
  onRefine?: () => void;
  /** 在工作区里打开这一镜的导演世界（不用跑画布） */
  onOpenDirector?: () => void;
  /** 这一镜有首帧 / 尾帧任务在跑 */
  frameRunning?: boolean;
  projectId: string;
  episodeId: string;
  /** 这一镜所在的场勾了「整段出」没有 —— 勾了就不能接上一镜；出片按钮改叫「单独出片」 */
  longTake?: boolean;
  /** 这一场整段出过的那一条（见 `storyboardView.longTakeClip`）。有它就不再说「还没出过片」 */
  longTakeClip?: LongTakeClipInfo | null;
  /** 换了「次」之后重读分集 —— 否则界面上还是旧的那条 */
  onPicked: () => void;
  shot: StoryboardShot;
  /** 同一集里镜号紧挨着的上一镜。第一镜为空 —— 衔接那一组据此禁用 */
  prev?: StoryboardShot | null;
  assets: AssetView[];
  /** 项目的默认形象表（资产 id → 形象 id） */
  defaultLooks?: Record<string, string>;
  issues: ShotIssue[];
  /** 这一镜走首帧出片还是参考出片（Rust `shot_route`）。没取到时为空 */
  routeInfo?: ShotRouteInfo | null;
  route: PromptRoute;
  /** 存这一镜。`base` = 草稿从哪一份开始改的（后端据此做三方比较）。回存没存上 */
  onSave: (next: StoryboardShot, base: StoryboardShot) => Promise<boolean>;
  /** 这一集有合成任务在跑（同一集同时只能跑一个，所以跑着的时候按钮不能点） */
  synthesizing: boolean;
  /** 这一镜在当前合成任务里的状态 */
  round: PromptRound;
  /** 只合成这一镜（当前线路） */
  onSynthesize: () => void;
  /** 这一集有出片任务在跑（出片一次一个） */
  videoRunning: boolean;
  /** 正在出的就是这一镜 */
  rendering: boolean;
  /** 这一镜在出片队列里排第几（还没轮到的里从 1 数）；不在队列里是 null（见 `queuePosition`） */
  queued?: number | null;
  /** 这一镜在合成队列里排第几（从 1 数）；不在队列里是 null */
  synthQueued?: number | null;
  /** 用当前线路出片 */
  onRender: () => void;
  /** 这一集正在生成镜头（整集或单场） */
  regenerating: boolean;
  /** 只重新生成这一镜所在的场 */
  onRegenerateScene: () => void;
  /** 每条问题旁边的「修复」按钮（后端算的改法，按问题原文对上）。改由这里在这一镜上改、走正常保存 */
  fixes?: IssueFixOffer[];
  /** 「自动修正」类的问题：整集修一遍（和顶上那个按钮同一个） */
  onRepairAll?: () => void;
}) {
  // 本地草稿：输入时只改草稿，失焦时整镜保存。父组件用 `key={shot.id}` 换镜头时重建。
  const [draft, setDraft] = useState<StoryboardShot>(shot);
  // 草稿从哪一份开始改的。存成功之后就是存进去的那份 —— 和它比，没改就不存；
  // 不和 `shot` 比：后台写回后界面刷新了、面板没重挂时，`shot` 是新的而草稿是旧的，一比就会把旧草稿存回去
  const base = useRef<StoryboardShot>(shot);
  const save = async (next: StoryboardShot = draft): Promise<boolean> => {
    if (JSON.stringify(next) === JSON.stringify(base.current)) return true;
    const ok = await onSave(next, base.current);
    if (ok) base.current = next;
    return ok;
  };
  const patch = (p: Partial<StoryboardShot>) => setDraft((d) => ({ ...d, ...p }));
  /**
   * 红色问题框右边的「修复」（用户 2026-09-30）。**和手动改走同一条路**：加长用的就是改时长那段 `withDuration`，
   * 改说话人就是改 `dialogue_lines`，然后正常保存。
   */
  const applyFix = async (offer: IssueFixOffer) => {
    const f = offer.fix;
    if (f.kind === "repair") {
      onRepairAll?.();
      return;
    }
    if (f.kind === "resynth") {
      onSynthesize();
      return;
    }
    if (f.kind === "mount") {
      if ((draft.ref_slots ?? []).some((r) => r.asset_id === f.asset_id)) return;
      const next = { ...draft, ref_slots: addSlot(draft.ref_slots ?? [], { asset_id: f.asset_id, usage: f.usage, control: f.control, not_control: "" }) };
      setDraft(next);
      await save(next);
      return;
    }
    if (f.kind === "duration") {
      const from = draft.duration;
      const next = withDuration(draft, f.secs);
      setDraft(next);
      setRetimed(`加长到 ${secs(f.secs)} 秒${next.segments !== draft.segments ? `，分段时间线跟着从 ${secs(from)} 秒拉长、描写里的「第几秒」一起换算了` : ""}`);
      await save(next);
      return;
    }
    const next = {
      ...draft,
      dialogue_lines: (draft.dialogue_lines ?? []).map((l, j) =>
        j === f.line ? { ...l, speaker: f.speaker, off_screen: l.off_screen || f.off_screen } : l,
      ),
    };
    setDraft(next);
    await save(next);
  };
  /** 时长改了几秒，分段时间线跟着按比例伸缩（见 `retime.ts`）—— 改完说一句改了什么 */
  const [retimed, setRetimed] = useState<string | null>(null);
  const saveDuration = async () => {
    const from = base.current.duration;
    const to = draft.duration;
    if (!(to > 0) || to === from) return save();
    const next = withDuration({ ...draft, duration: from }, to);
    const moved = next.segments !== draft.segments;
    setDraft(next);
    setRetimed(
      moved
        ? `分段时间线跟着从 ${secs(from)} 秒拉到 ${secs(to)} 秒，描写里写的「第几秒」也一起换算了。细化过的镜头改完要重新合成提示词`
        : `时长改成 ${secs(to)} 秒。分段时间线本来就没对齐 ${secs(from)} 秒，没有自动伸缩 —— 要整段重来就重新生成这一场`,
    );
    return save(next);
  };
  const segments = draft.segments ?? [];
  const setSegment = (i: number, p: Partial<ShotSegment>) =>
    patch({ segments: segments.map((s, j) => (j === i ? { ...s, ...p } : s)) });
  /** 改一段并**立刻保存**（点标签、清空合同）：拿改后的那份去存，不等下一轮渲染 */
  const commitSegment = (i: number, p: Partial<ShotSegment>) => {
    const next = { ...draft, segments: segments.map((s, j) => (j === i ? { ...s, ...p } : s)) };
    setDraft(next);
    void save(next);
  };
  const nameOf = (id: string) => assets.find((a) => a.id === id)?.name ?? id;
  const director = directorBinding(draft);

  return (
    <section
      className="flex min-w-0 flex-col gap-3 rounded-xl px-4 py-3"
      style={{ background: "var(--studio-bg-inset)", boxShadow: "0 0 0 1px var(--studio-border-subtle)" }}
    >
      <header className="flex flex-wrap items-center gap-2">
        <span className="text-[14px] font-medium">
          {draft.scene_no ? `第 ${draft.scene_no} 场 · ` : ""}第 {draft.shot_num} 镜
        </span>
        <span className="text-[12px]" style={{ color: "var(--studio-text-muted)" }}>
          {secs(draft.duration)} 秒 · {display("shotSize", draft.scale) || "未定景别"} · {display("move", draft.camera_movement) || "未定运镜"}
        </span>
        {director && <span className="rounded-full px-2 py-0.5 text-[10.5px]" style={{ color: "var(--studio-primary)", background: "color-mix(in srgb, var(--studio-primary) 12%, transparent)", boxShadow: "0 0 0 1px color-mix(in srgb, var(--studio-primary) 30%, transparent)" }}>3D片场已绑定</span>}
      </header>

      {issues.length > 0 && (
        <ul className="space-y-1 rounded-md px-3 py-2 text-[11.5px] leading-relaxed" style={{ background: "color-mix(in srgb, var(--studio-failed-fg) 12%, transparent)", color: "var(--studio-failed-fg)" }}>
          {issues.map((i) => {
            const fix = fixes.find((f) => f.message === i.message);
            return (
              <li key={`${i.field}-${i.message}`} className="flex items-start gap-2">
                <span className="min-w-0 flex-1">· {i.message}</span>
                {fix && (
                  <button
                    type="button"
                    onClick={() => void applyFix(fix)}
                    // 这一集正在合成时排到那一轮后面（加入合成队列）；只有这一镜正在合 / 已在排队时才灰
                    disabled={fix.fix.kind === "resynth" && (round === "running" || round === "queued")}
                    title={fixTitle(fix)}
                    className="shrink-0 rounded-md px-2 py-0.5 text-[11px] disabled:opacity-40"
                    style={{ background: "var(--studio-bg-elevated)", color: "var(--studio-text)", boxShadow: "0 0 0 1px var(--studio-border)" }}
                  >
                    {fix.label}
                  </button>
                )}
              </li>
            );
          })}
          {issues.some(needsSceneRegen) && draft.scene_no ? (
            <li className="mt-1.5">
              <button
                type="button"
                disabled={regenerating}
                onClick={onRegenerateScene}
                title="漏拍是少了镜头，代码补不出画面。只重新生成这一场：把上面的问题告诉模型，最多重试 3 次；其它场的镜头、提示词、出过的片都不动"
                className="rounded-md px-2.5 py-1 text-[11.5px]"
                style={{ background: "var(--studio-bg-elevated)", color: "var(--studio-text)", boxShadow: "0 0 0 1px var(--studio-border)", opacity: regenerating ? 0.6 : 1 }}
              >
                {regenerating ? "正在生成…" : `重新生成第 ${draft.scene_no} 场`}
              </button>
            </li>
          ) : null}
        </ul>
      )}

      <Block title="职责和时长">
        <div className="grid gap-2" style={{ gridTemplateColumns: "minmax(0,1fr) 90px" }}>
          <input className={INPUT} style={INPUT_STYLE} value={draft.duty ?? ""} placeholder="这一镜为什么存在"
            onChange={(e) => patch({ duty: e.target.value })} onBlur={() => save()} />
          <label className="flex items-center gap-1 text-[12px]">
            <input className={INPUT} style={INPUT_STYLE} type="number" min={4} max={MAX_LONG_SHOT_SECS} step={0.5} value={draft.duration}
              onChange={(e) => patch({ duration: Number(e.target.value) })} onBlur={() => void saveDuration()} />
            秒
          </label>
        </div>
        {retimed && (
          <p className="mt-1 text-[11px]" style={{ color: "var(--studio-text-muted)" }}>{retimed}</p>
        )}
        {longShotHint(draft.duration) && (
          <p className="mt-1 text-[11px]" style={{ color: "var(--studio-primary)" }}>{longShotHint(draft.duration)}</p>
        )}
        {draft.source && (
          <p className="mt-1 text-[11px]" style={{ color: "var(--studio-text-muted)" }}>剧本原文：{draft.source}</p>
        )}
      </Block>

      {/*
        **接上一镜是三选一，不是勾选框。**

        用户 2026-09-20 提的是「要不要以上一镜作为首帧参考」，但在 H3 这里
        那是三件不同的事（运镜参考 / 字面首帧 / 潜变量续写）。
        写「首帧参考」而实际发视频参考，就是又一次「界面说的和实际做的不一样」。

        **只推荐、不自动勾**，而且推荐要说出理由 —— 跨场接上一镜会把上一场的
        光线和布景带过来，那正是用户点名要避免的。
      */}
      <Block title="接上一镜">
        {(() => {
          const advice = continuityAdvice(draft, prev, longTake);
          const cur = effectiveContinuity(draft);
          // **自动设**（2026-10-07 用户：「不能光写推荐」）：规则在 Rust `shot_route::auto_continuity`，生成镜头表、首帧加上 / 摘掉、合成前都会按它设。
          // 手动点过的标 `continuity_manual`，自动不再覆盖；「恢复自动」回到规则给的值
          const auto = routeInfo?.auto_continuity ?? advice.recommended;
          const manual = !!draft.continuity_manual;
          return (
            <>
              <div className="flex flex-wrap gap-1.5">
                {(["none", "ref_video", "first_frame"] as Continuity[]).map((c) => {
                  const on = cur === c;
                  return (
                    <button
                      key={c}
                      type="button"
                      disabled={!!advice.blocked && c !== "none"}
                      title={CONTINUITY_HINT[c]}
                      onClick={() => {
                        // 直接存新的那份：`patch` 之后同一个闭包里的 `draft` 还是旧的，以前这一下存的是改之前的
                        // 点的就是自动那一档 = 不算手动（以后链路变了还跟着自动走）
                        const next = { ...draft, continuity: c, continuity_manual: c !== auto };
                        setDraft(next);
                        void save(next);
                      }}
                      className="rounded-md px-2 py-1 text-[12px] disabled:opacity-40"
                      style={{
                        background: on ? "color-mix(in srgb, var(--studio-primary) 14%, transparent)" : "var(--studio-bg-inset)",
                        boxShadow: `0 0 0 ${on ? 1.5 : 1}px ${on ? "var(--studio-primary)" : "var(--studio-border-subtle)"}`,
                        color: "var(--studio-text)",
                      }}
                    >
                      {CONTINUITY_LABEL[c]}
                      {!advice.blocked && auto === c ? " · 自动" : ""}
                    </button>
                  );
                })}
              </div>
              <p className="mt-1 text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
                {advice.blocked ||
                  (auto === "ref_video"
                    ? `${advice.why}；参考出片，自动接上一镜的机位和光线`
                    : routeInfo?.actual === "keyframe" || routeInfo?.advised === "keyframe"
                      ? "首帧出片：构图由首帧定，自动不接（「运镜与光线」在首帧出片里不起作用）"
                      : advice.why)}
              </p>
              {manual && (
                <p className="text-[11px]" style={{ color: "var(--studio-accent)" }}>
                  这是手动选的，自动设不会改它（自动会是「{CONTINUITY_LABEL[auto]}」）·{" "}
                  <button
                    type="button"
                    className="underline"
                    onClick={() => {
                      const next = { ...draft, continuity: auto, continuity_manual: false };
                      setDraft(next);
                      void save(next);
                    }}
                  >
                    恢复自动
                  </button>
                </p>
              )}
              {cur === "ref_video" && routeInfo?.actual === "keyframe" && (
                <p className="text-[11px]" style={{ color: "var(--studio-accent)" }}>这一镜挂了首帧（首帧出片），「运镜与光线」不起作用</p>
              )}
              {cur === "first_frame" && (
                <p className="text-[11px]" style={{ color: "var(--studio-text-muted)" }}>接住最后一帧 = 上一镜最后一帧当这一镜的首帧，走首帧出片，参考图不发</p>
              )}
              <p className="text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
                {CONTINUITY_HINT[cur]}
                {cur !== "none" ? " · 出片前上一镜必须先出片" : ""}
              </p>
            </>
          );
        })()}
      </Block>

      <Block title="景别 · 机位 · 运镜">
        <div className="grid grid-cols-3 gap-2">
          {([["shotSize", "scale"], ["angle", "camera_angle"], ["move", "camera_movement"]] as const).map(([g, field]) => (
            <select key={g} className={INPUT} style={INPUT_STYLE} value={draft[field] ?? ""}
              onChange={(e) => {
                const next = { ...draft, [field]: e.target.value };
                setDraft(next);
                save(next);
              }}>
              <option value="">{group(g)?.name ?? g}：未定</option>
              {(group(g)?.items ?? []).map((t) => (
                <option key={t.id} value={t.id}>{t.name}</option>
              ))}
            </select>
          ))}
        </div>
        <textarea className={`${INPUT} mt-2`} style={INPUT_STYLE} rows={2} value={manualCameraNote(draft.camera_note ?? "")} placeholder="补充运镜指令"
          onChange={(e) => {
            patch({
              camera_note: e.target.value,
              ...(!draft.director_camera && director ? {
                director_camera: {
                  name: director.name,
                  camera: director.payload.camera,
                  path: director.payload.path ?? null,
                  motion_note: director.motionNote ?? "",
                } as StoryboardShot["director_camera"],
              } : {}),
            });
          }} onBlur={() => save()} />
        <p className="mt-1 text-[10.5px]" style={{ color: "var(--studio-text-muted)" }}>修改运镜会使当前提示词过期；保存后请重新合成这一镜。</p>
        {director && (
          <div className="mt-2 rounded-lg px-3 py-2 text-[11px] leading-5" style={{ background: "color-mix(in srgb, var(--studio-primary) 8%, var(--studio-bg-elevated))", boxShadow: "0 0 0 1px color-mix(in srgb, var(--studio-primary) 22%, transparent)" }}>
            <div className="font-medium" style={{ color: "var(--studio-text)" }}>导演世界 · {director.name}</div>
            <div style={{ color: "var(--studio-text-muted)" }}>
              位置 {(director.payload.camera?.position ?? []).map((v) => Number(v).toFixed(2)).join(", ") || "—"}
              {director.payload.camera ? ` · 水平 ${Math.round(director.payload.camera.yawDeg ?? 0)}° · 俯仰 ${Math.round(director.payload.camera.pitchDeg ?? 0)}° · FOV ${Math.round(director.payload.camera.fovDeg ?? 0)}°` : ""}
            </div>
            <div style={{ color: "var(--studio-text-muted)" }}>
              {director.payload.path ? `${director.payload.path.name ?? "主运镜"} · ${director.payload.path.cameras?.length ?? 0} 个关键帧 · ${director.payload.path.durationSec ?? 0} 秒` : "固定机位，无运镜路径"}
            </div>
            {director.motionNote && <div style={{ color: "var(--studio-text-muted)" }}>模型运镜：{director.motionNote}</div>}
            {draft.director_camera?.first_frame && <div style={{ color: "var(--studio-text-muted)" }}>运镜首尾帧（人偶稿）已输出到画布：先在画布里换成角色，再到下面「关键帧」栏导入 —— 人偶稿直接当首帧，出片会是一段人偶戏</div>}
            {draft.director_camera?.depth_video && <div style={{ color: "var(--studio-text-muted)" }}>深度控制：已生成并关联 · {draft.director_camera.depth_video.split("/").pop()}</div>}
            <div style={{ color: "var(--studio-text-muted)" }}>站位和自然语言运镜会进入当前线路提示词；机位 JSON 仅用于 3D 控制，不会发送给模型。</div>
          </div>
        )}
      </Block>

      <Block title="画面（按时间段，同一空间里的连续变化）">
        {(() => {
          // 用户 2026-09-27：「把时间线写好，描述细致，而不是一笔带过」—— 写得多细、哪几段一笔带过，这里直接看得见
          const cps = shotDensity(draft);
          const thin = thinSegments(draft);
          const low = cps < THIN_CPS || thin.length > 0;
          return (
            <div className="mb-2 flex flex-wrap items-center gap-2 text-[11.5px]">
              <span style={{ color: low ? "var(--studio-accent)" : "var(--studio-text-muted)" }}>
                每秒约 {Math.round(cps)} 字（目标 {TARGET_CPS}）
                {thin.length > 0 && ` · 一笔带过：${thin.map((t) => `第 ${t.n} 段 ${secs(t.start)}~${secs(t.end)} 秒只有 ${t.chars} 字`).join("，")}`}
              </span>
              {onRefine && (
                <button
                  type="button"
                  disabled={refining || segments.length === 0}
                  onClick={async () => {
                    // **先存完再细化**：不等的话细化读的是改之前的时间段，或者细化写回之后草稿又把旧的存回去（2026-09-28 审查）
                    if (await save()) onRefine();
                  }}
                  title="结构不动（时长、景别、运镜、台词、人物都不改），只把这一镜每一刻发生什么写满。一镜几十秒；细化后要重新合成提示词"
                  className="rounded-md px-2.5 py-1 text-[11.5px]"
                  style={{ background: "var(--studio-bg-elevated)", boxShadow: "0 0 0 1px var(--studio-border)", opacity: refining || segments.length === 0 ? 0.6 : 1 }}
                >
                  {refineRound === "running" ? "细化中…" : refineRound === "queued" ? "排队中" : "细化时间线"}
                </button>
              )}
              {refining && !refineRound && <span style={{ color: "var(--studio-text-muted)" }}>别的镜头在细化，等这一轮跑完</span>}
            </div>
          );
        })()}
        <label className="mb-2 flex items-center gap-2 text-[12px]">
          <span style={{ color: "var(--studio-text-muted)" }}>打斗</span>
          <select className={INPUT} style={{ ...INPUT_STYLE, width: "auto" }} value={draft.combat_style ?? ""}
            onChange={(e) => { const next = { ...draft, combat_style: e.target.value as CombatStyle }; setDraft(next); void save(next); }}>
            <option value="">不是打斗镜</option>
            {COMBAT_STYLES.map((c) => <option key={c.id} value={c.id}>{c.label}</option>)}
          </select>
          <span className="text-[11px]" style={{ color: "var(--studio-text-muted)" }}>打斗镜每段写一个攻防交换；出片时挂项目设置里这一类的战斗 LoRA</span>
        </label>
        <div className="flex flex-col gap-2">
          {segments.map((seg, i) => (
            <div key={i} className="flex flex-col gap-1 rounded-md px-2 py-2" style={{ background: "var(--studio-bg-elevated)" }}>
              <div className="flex items-center gap-2 text-[12px]">
                <span className="font-mono" style={{ color: "var(--studio-text-muted)" }}>{secs(seg.start)}~{secs(seg.end)} 秒</span>
                <input className={INPUT} style={INPUT_STYLE} value={seg.title ?? ""} placeholder="这一段的标题"
                  onChange={(e) => setSegment(i, { title: e.target.value })} onBlur={() => save()} />
              </div>
              <SegText label="画面" value={seg.visual} onChange={(v) => setSegment(i, { visual: v })} onBlur={() => save()} />
              <div className="grid grid-cols-2 gap-1">
                <SegText label="禁止" value={seg.forbid} onChange={(v) => setSegment(i, { forbid: v })} onBlur={() => save()} />
                <SegText label="约束" value={seg.constraints} onChange={(v) => setSegment(i, { constraints: v })} onBlur={() => save()} />
                <SegText label="站位" value={seg.blocking} onChange={(v) => setSegment(i, { blocking: v })} onBlur={() => save()} />
                <SegText label="运镜" value={seg.camera} onChange={(v) => setSegment(i, { camera: v })} onBlur={() => save()} />
              </div>
              <SegText label="声音" value={seg.sound} onChange={(v) => setSegment(i, { sound: v })} onBlur={() => save()} />
              <SegmentContracts seg={seg} wide={isWideScale(draft.scale)}
                onChange={(p) => setSegment(i, p)} onCommit={(p) => commitSegment(i, p)} onBlur={() => save()} />
            </div>
          ))}
          {segments.length === 0 && <p className="text-[12px]" style={{ color: "var(--studio-text-muted)" }}>没有时间段</p>}
        </div>
      </Block>

      {stageLines(draft.stage).length > 0 && (
        <Block title="这一场的布景">
          {/* 整场共用、只读：改一镜就和同场别的镜对不上了。重新生成这一场的镜头表时会重写 */}
          <div className="flex flex-col gap-0.5 text-[12px]">
            {stageLines(draft.stage).map((r) => (
              <div key={r.label}>
                <span style={{ color: "var(--studio-text-muted)" }}>{r.label}：</span>
                <span style={{ color: "var(--studio-text-2)" }}>{r.text}</span>
              </div>
            ))}
          </div>
          <p className="mt-1 text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
            整场所有镜头共用，站位、朝向和光的方向都从这里推。重新生成这一场的镜头表时会一起重写。
          </p>
        </Block>
      )}

      <div className="grid gap-3" style={{ gridTemplateColumns: "repeat(2, minmax(0,1fr))" }}>
        <Block title="光影">
          <textarea className={INPUT} style={INPUT_STYLE} rows={3} value={draft.lighting ?? ""} placeholder="光源在哪、朝哪照、冷暖、明暗对比"
            onChange={(e) => patch({ lighting: e.target.value })} onBlur={() => save()} />
        </Block>
        <Block title="台词 · 声音">
          <div className="flex flex-col gap-1">
            {(draft.dialogue_lines ?? []).map((l, i) => (
              <div key={i} className="flex items-center gap-1 text-[12px]">
                <span className="shrink-0" style={{ color: "var(--studio-text-2)" }}>{l.speaker}{l.off_screen ? "（画外）" : ""}：</span>
                <input className={INPUT} style={INPUT_STYLE} value={l.line}
                  onChange={(e) => patch({ dialogue_lines: (draft.dialogue_lines ?? []).map((x, j) => (j === i ? { ...x, line: e.target.value } : x)) })}
                  onBlur={() => save()} />
              </div>
            ))}
            {(draft.dialogue_lines ?? []).length === 0 && <span className="text-[12px]" style={{ color: "var(--studio-text-muted)" }}>无台词</span>}
            <input className={INPUT} style={INPUT_STYLE} value={draft.ambient_sound ?? ""} placeholder="环境音"
              onChange={(e) => patch({ ambient_sound: e.target.value })} onBlur={() => save()} />
            <input className={INPUT} style={INPUT_STYLE} value={draft.sound_effect ?? ""} placeholder="音效"
              onChange={(e) => patch({ sound_effect: e.target.value })} onBlur={() => save()} />
          </div>
        </Block>
      </div>

      {/*
        **出片方式二选一**（2026-10-07 用户）：首帧 / 尾帧有图 = 首帧出片，参考图区折叠；删掉首帧自动恢复参考出片。
        按分镜内容的建议（有人中途进场 / 人太多 → 参考）和实际不一样时给出怎么改；首帧出片却缺中途进场的人 → 拦下（红字）。
        规则只在 Rust `shot_route` 一份，这里只显示它算好的。
      */}
      {routeInfo && (
        <Block title="出片方式">
          <p className="text-[12.5px]" style={{ color: "var(--studio-text)" }}>
            <b>{routeInfo.actual === "keyframe" ? "首帧出片" : "参考出片"}</b>
            <span style={{ color: "var(--studio-text-muted)" }}>
              {routeInfo.actual === "keyframe"
                ? ` · 挂了首帧：出片只发首帧${shot.keyframes?.last ? "和尾帧" : ""}，人物参考图不发`
                : " · 没有首帧：出片发这一镜的人物 / 场景 / 道具参考图，每个人的长相都锁得住"}
            </span>
          </p>
          <p className="mt-0.5 text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
            按分镜内容建议{routeInfo.advised === "keyframe" ? "首帧" : "参考"}出片：{routeInfo.reason}
          </p>
          {/* 规则判错时手动指定（2026-10-11 用户）：选「参考」的按场出首帧会跳过它；选「首帧」的照常出首帧 */}
          <div className="mt-1.5 flex items-center gap-1.5 text-[11px]">
            <span style={{ color: "var(--studio-text-muted)" }}>出片方式</span>
            {([[undefined, "跟随建议"], ["keyframe", "首帧"], ["reference", "参考"]] as const).map(([v, label]) => {
              const on = (draft.route_choice ?? undefined) === v;
              return (
                <button
                  key={label}
                  type="button"
                  className="rounded-md border px-2 py-0.5"
                  style={{
                    borderColor: on ? "var(--studio-primary)" : "var(--studio-border-subtle)",
                    color: on ? "var(--studio-text)" : "var(--studio-text-muted)",
                    background: on ? "color-mix(in srgb, var(--studio-primary) 14%, transparent)" : "transparent",
                  }}
                  onClick={() => {
                    const next = { ...draft, route_choice: v };
                    if (v === undefined) delete (next as { route_choice?: unknown }).route_choice;
                    setDraft(next);
                    void save(next);
                  }}
                >
                  {label}
                </button>
              );
            })}
          </div>
          {routeInfo.blocker ? (
            <p className="mt-0.5 text-[11px]" style={{ color: "var(--studio-failed-fg)" }}>出不了：{routeInfo.blocker}</p>
          ) : routeInfo.actual !== routeInfo.advised ? (
            <p className="mt-0.5 text-[11px]" style={{ color: "var(--studio-accent)" }}>
              {routeInfo.advised === "reference" ? "建议删掉首帧，改走参考出片（下面「关键帧」里点「摘掉」）" : "建议先出首帧（下面「关键帧」），出完自动改成首帧出片并重合成提示词"}
            </p>
          ) : null}
        </Block>
      )}

      <Fold
        title={routeInfo?.actual === "keyframe" ? "出首帧用的参考图（出片不发）" : routeInfo?.advised === "keyframe" ? "出场资产和参考图（出首帧时挂这些；出首帧之前出片按它们出）" : "出场资产和参考图"}
        open={routeInfo?.actual !== "keyframe"}
        hint={routeInfo?.actual === "keyframe" ? "这一镜首帧出片：参考图只在出首帧那一步用。摘掉首帧就恢复参考出片" : undefined}
      >
        <LookRows draft={draft} assets={assets} defaultLooks={defaultLooks} onPick={(assetId, lookId) => {
          const overrides = { ...(draft.look_overrides ?? {}) };
          if (lookId === null) delete overrides[assetId];
          else overrides[assetId] = lookId;
          const next = { ...draft, look_overrides: overrides };
          setDraft(next);
          save(next);
        }} />
        <p className="text-[12px]" style={{ color: "var(--studio-text-2)" }}>
          {[...(draft.character_ids ?? []), ...(draft.scene_id ? [draft.scene_id] : []), ...(draft.prop_ids ?? [])].map(nameOf).join("、") || "没有出场资产"}
        </p>
        {(draft.unlinked_assets ?? []).length > 0 && (
          <p className="mt-1 text-[11px]" style={{ color: "var(--studio-accent)" }}>
            资产库里没有：{(draft.unlinked_assets ?? []).map((u) => u.name).join("、")} —— 提示词里按文字描述；想锁住长相，去资产那一步建上并上传或生成图，再重新合成。
          </p>
        )}
        {(draft.scene_items ?? []).length > 0 && (
          <p className="mt-1 text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
            {(draft.scene_items ?? []).map((it) => `${it.name}：${it.detail ?? ""}`).join("；")}
          </p>
        )}
        <div className="mt-2 flex flex-col gap-1">
          {(draft.ref_slots ?? []).map((slot, i) => (
            <div key={i} className="grid items-center gap-2 text-[12px]" style={{ gridTemplateColumns: "auto minmax(0,1fr) auto" }}>
              <span>
                参考图 {slot.order} · {nameOf(slot.asset_id)} · {USAGE_LABEL[slot.usage]}
                {assets.find((a) => a.id === slot.asset_id && !a.cover_url) && (
                  <span style={{ color: "var(--studio-accent)" }}> · 还没图，先按文字描述</span>
                )}
              </span>
              <input className={INPUT} style={INPUT_STYLE} value={slot.control} placeholder="控制什么（正面写）"
                onChange={(e) => patch({ ref_slots: (draft.ref_slots ?? []).map((x, j) => (j === i ? { ...x, control: e.target.value } : x)) })}
                onBlur={() => save()} />
              <button type="button" className="rounded px-1.5 text-[11px]" style={{ color: "var(--studio-failed-fg)" }}
                title="不挂这张图了。提示词要重新合成（参考图编号变了）"
                onClick={() => {
                  const next = { ...draft, ref_slots: removeSlot(draft.ref_slots ?? [], i) };
                  setDraft(next);
                  save(next);
                }}>
                移除
              </button>
            </div>
          ))}
          {(draft.ref_slots ?? []).length === 0 && <span className="text-[12px]" style={{ color: "var(--studio-text-muted)" }}>没有参考图</span>}
          <AddSlotRow assets={assets} cast={draft.character_ids ?? []} taken={(draft.ref_slots ?? []).map((x) => x.asset_id)} onAdd={(slot) => {
            const next = { ...draft, ref_slots: addSlot(draft.ref_slots ?? [], slot) };
            setDraft(next);
            save(next);
          }} />
        </div>
      </Fold>

      <Block title="导演世界（3D 片场 · 摆人 · 机位 · 运镜）">
        <DirectorBlock projectId={projectId} episodeId={episodeId} shot={shot}
          sceneAsset={assets.find((a) => a.id === shot.scene_id && a.kind === "scene") ?? null}
          frameRunning={frameRunning} onOpen={() => onOpenDirector?.()} onChanged={onPicked} />
      </Block>

      {/* 读 `shot` 不读草稿：关键帧是后端按磁盘算的，草稿里那份是打开这一镜时的 */}
      <Fold
        title={routeInfo?.actual === "reference" && routeInfo.advised === "reference" ? "关键帧（这一镜参考出片；导入或出首帧会改成首帧出片）" : "关键帧（首帧 / 尾帧）"}
        open={!(routeInfo?.actual === "reference" && routeInfo.advised === "reference")}
      >
        <KeyframeBlock
          projectId={projectId}
          episodeId={episodeId}
          shot={shot}
          route={route}
          frameRunning={frameRunning}
          onChanged={onPicked}
          framePrompt={draft.prompts?.[route]?.frame ?? ""}
          onFramePrompt={(v) => patch({ prompts: { ...draft.prompts, [route]: { video: "", synthesized_at: 0, ...draft.prompts?.[route], frame: v } } })}
          endFrame={draft.end_frame ?? ""}
          onEndFrame={(v) => patch({ end_frame: v })}
          fallbackEnding={segments.at(-1)?.visual?.trim() ?? ""}
          save={() => save()}
        />
      </Fold>

      <Block title={`最终提示词 · 线路 ${route.toUpperCase()}（${route === "a" ? "中文结构" : "H3 官方格式"}）`}>
        {(() => {
          const current = draft.prompts?.[route];
          const other: PromptRoute = route === "a" ? "b" : "a";
          const setPrompt = (p: Partial<NonNullable<typeof current>>) =>
            patch({ prompts: { ...draft.prompts, [route]: { frame: "", video: "", synthesized_at: 0, ...current, ...p } } });
          return (
            <div className="flex flex-col gap-2">
              <div className="flex items-center gap-2">
                {(() => {
                  // **有别的镜头在合成时不再灰掉，改成「加入合成队列」**（2026-10-06 用户：「方式方法跟出片一致」）。
                  // 只有这一镜自己正在合 / 已经在排队时才灰
                  const busy = round === "running" || round === "queued";
                  const queueing = synthesizing && !busy;
                  return (
                    <button
                      type="button"
                      disabled={busy}
                      onClick={onSynthesize}
                      className="rounded-md px-2.5 py-1 text-[11.5px]"
                      style={{ background: "var(--studio-bg-elevated)", boxShadow: "0 0 0 1px var(--studio-border)", opacity: busy ? 0.6 : 1 }}
                    >
                      {round === "running"
                        ? "合成中…"
                        : round === "queued"
                          ? synthQueued
                            ? `合成队列第 ${synthQueued} 个`
                            : "排队中"
                          : queueing
                            ? "加入合成队列"
                            : current?.video?.trim()
                              ? "重新合成这一镜"
                              : "合成这一镜"}
                    </button>
                  );
                })()}
                <span className="text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
                  {round === "running"
                    ? "正在合成这一镜"
                    : round === "queued"
                      ? "这一轮合成还没轮到这一镜"
                      : synthesizing
                        ? "别的镜头在合成，点了排到这一轮后面，轮到时按那时的内容合"
                        : round === "done"
                          ? "这一轮已合成。改完点到别处就保存，出片时原样发送"
                          : current?.video?.trim()
                            ? "改完点到别处就保存，出片时原样发送"
                            : "还没合成"}
                </span>
              </div>
              {current?.video?.trim() && (
                <>
                  <label className="text-[11px]" style={{ color: "var(--studio-text-muted)" }}>视频运动提示词</label>
                  <textarea className={`${INPUT} font-mono`} style={INPUT_STYLE} rows={12} value={current.video}
                    onChange={(e) => setPrompt({ video: e.target.value })} onBlur={() => save()} />
                  <p className="text-[11px]" style={{ color: "var(--studio-text-muted)" }}>首帧 / 尾帧的提示词在上面「关键帧」里，就在两张图下面</p>
                </>
              )}
              <p className="text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
                线路 {other.toUpperCase()}：{draft.prompts?.[other]?.video?.trim() ? "也合成过（切到那条线路后可以看、可以改）" : "没合成"}
              </p>
            </div>
          );
        })()}
      </Block>

      <Block title="出片">
        {(() => {
          // 出片弹窗里能换线路：当前线路不能出、另一条能出时按钮照样能点
          const other: PromptRoute = route === "a" ? "b" : "a";
          // **有片在出时不再灰掉，改成「加入队列」**（2026-10-05 用户：「我不可能一直等着，需要可点击为加入列队」）
          const mine = rendering ? null : renderBlocker(shot, route, issues, false);
          const why = mine && renderBlocker(shot, other, issues, false) ? mine : null;
          const queueing = videoRunning && !rendering;
          const off = rendering || !!why || queued !== null;
          return (
            <div className="flex flex-col gap-2">
              <div className="flex items-center gap-2">
                <button
                  type="button"
                  disabled={off}
                  onClick={onRender}
                  className="rounded-md px-3 py-1 text-[12px] font-medium"
                  style={{ background: "var(--studio-primary)", color: "var(--studio-on-primary)", opacity: off ? 0.5 : 1 }}
                >
                  {queued !== null ? `已在队列 · 第 ${queued} 个` : queueing && !why ? "加入队列" : renderButtonLabel(rendering, renderTakes(shot, route).length > 0, longTake)}
                </button>
                <span className="text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
                  {rendering
                    ? "要几分钟到十几分钟，可以先去别的地方，跑完右下角会告诉你"
                    : why ??
                      (queued !== null ? "前面的出完自动轮到它，进度在右下角和 QUEUE 里" : null) ??
                      (queueing ? "这一集有镜头正在出片；点了排到后面，轮到自动出，不用等着" : null) ??
                      (longTake
                        ? "只出这一镜，整段那一条不受影响；组装和合成一集仍放整段那一条"
                        : "原样发送当前线路的提示词和参考图；每出一次都留着，不覆盖")}
                </span>
              </div>
              <TakeList shot={shot} route={route} projectId={projectId} episodeId={episodeId} onPicked={onPicked} longTakeClip={longTakeClip} />
            </div>
          );
        })()}
      </Block>
    </section>
  );
}

/**
 * 这一镜出过的每一「次」。
 *
 * **每一条都能点回去。** 以前这里只是陈列 —— `takes` 明明是版本链
 * （`video_url` 指最新一条），却没有任何办法换回旧的那条。
 * 于是「重出一次」是个单向操作：出砸了就只能再出一次碰运气。
 * 画布回填之后这个问题更明显：收进来的片不一定比原来那条好。
 */
function TakeList({ shot, route, projectId, episodeId, onPicked, longTakeClip }: {
  shot: StoryboardShot; route: PromptRoute;
  projectId: string; episodeId: string; onPicked: () => void;
  longTakeClip?: LongTakeClipInfo | null;
}) {
  const takes = takesNewestFirst(shot);
  const [busy, setBusy] = useState("");
  const [err, setErr] = useState("");
  /*
    **整场出过就不说「还没出过片」。** 用户 10-01：第 5 场整段出过，22~24 镜详情里还写着
    「还没出过片」—— 片子在场上（长镜头不切开），不在各镜名下，但这一镜确实出过了。
  */
  const scene = longTakeClip ? (
    <div className="flex items-start gap-2 rounded-md p-1.5" style={{ background: "var(--studio-bg-elevated)", boxShadow: "0 0 0 1px color-mix(in srgb, var(--studio-primary) 45%, transparent)" }}>
      <MediaThumb path={longTakeClip.path} kind="video" className="aspect-video w-48 shrink-0 rounded object-contain" />
      <p className="text-[11px] leading-relaxed" style={{ color: "var(--studio-text-2)" }}>
        在第 {longTakeClip.sceneNo} 场整段出的长镜头里（第 {longTakeClip.first}–{longTakeClip.last} 镜）
        <br />
        <span style={{ color: "var(--studio-text-muted)" }}>
          线路 {longTakeClip.route.toUpperCase()}{longTakeClip.quality ? ` · ${longTakeClip.quality}` : ""} · {new Date(longTakeClip.created_at * 1000).toLocaleString()}
          <br />
          组装和合成一集放的是这一整条
        </span>
      </p>
    </div>
  ) : null;
  if (takes.length === 0) {
    return scene ?? <p className="text-[12px]" style={{ color: "var(--studio-text-muted)" }}>还没出过片</p>;
  }
  const pick = async (path: string) => {
    setBusy(path); setErr("");
    try { await cinemaPickTake(projectId, episodeId, shot.id, path); onPicked(); }
    catch (e) { setErr(String(e)); } finally { setBusy(""); }
  };
  return (
    <>
    {scene && (
      <div className="mb-2 flex flex-col gap-1">
        {scene}
        <p className="text-[11px]" style={{ color: "var(--studio-text-muted)" }}>下面是这一镜单独出的 —— 这一场按整段放的时候不用它们</p>
      </div>
    )}
    {err && <p className="mb-1 text-[11px]" style={{ color: "var(--studio-danger)" }} role="alert">{err}</p>}
    <div className="grid gap-2" style={{ gridTemplateColumns: "repeat(auto-fill, minmax(220px, 1fr))" }}>
      {takes.map((t, i) => {
        const current = shot.video_url === t.path;
        const canvas = t.origin === "canvas";
        return (
        <div key={t.path} className="rounded-md p-1.5" style={{ background: "var(--studio-bg-elevated)", boxShadow: current ? "0 0 0 1px var(--studio-accent)" : t.route === route && !canvas ? "0 0 0 1px var(--studio-border)" : "none" }}>
          <MediaThumb path={t.path} kind="video" className="aspect-video w-full rounded object-contain" />
          <p className="mt-1 text-[11px]" style={{ color: "var(--studio-text-2)" }}>
            第 {takes.length - i} 次 · {canvas ? "画布回填" : `线路 ${t.route.toUpperCase()}`}{t.quality ? ` · ${t.quality}` : ""} · {t.pictures ? `挂 ${t.pictures} 张图` : "文生"} · {new Date(t.created_at * 1000).toLocaleString()}
          </p>
          {/* **挂的是哪几张要看得见。** 只说「挂 2 张图」答不出
              「它用了哪张场景图」—— 那是回看时唯一想知道的事。老数据没有。 */}
          {t.picture_paths?.length ? (
            <p className="text-[11px] break-all" style={{ color: "var(--studio-text-muted)" }}
               title={t.picture_paths.join("\n")}>
              用图：{t.picture_paths.map((p) => p.split("/").pop()).join(" · ")}
            </p>
          ) : null}
          {current ? (
            <p className="text-[11px]" style={{ color: "var(--studio-accent)" }}>在用这一次</p>
          ) : (
            <button type="button" className="mt-0.5 text-[11px] underline" disabled={busy === t.path}
              style={{ color: "var(--studio-text-muted)" }} onClick={() => void pick(t.path)}>
              {busy === t.path ? "切换中…" : "用这一次"}
            </button>
          )}
        </div>
      );})}
    </div>
    </>
  );
}

/** 可以折起来的一块：二选一时没用上的那一块默认折起（`open` 变了会跟着变） */
function Fold({ title, open, hint, children }: { title: string; open: boolean; hint?: string; children: React.ReactNode }) {
  return (
    <details key={String(open)} open={open} className="min-w-0">
      <summary className="studio-eyebrow mb-1.5 cursor-pointer select-none">{title}</summary>
      {hint && <p className="mb-1 text-[11px]" style={{ color: "var(--studio-text-muted)" }}>{hint}</p>}
      {children}
    </details>
  );
}

function Block({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="min-w-0">
      <p className="studio-eyebrow mb-1.5">{title}</p>
      {children}
    </div>
  );
}

function SegText({ label, value, onChange, onBlur }: { label: string; value?: string; onChange: (v: string) => void; onBlur: () => void }) {
  return (
    <label className="flex items-start gap-1 text-[12px]">
      <span className="w-8 shrink-0 pt-1" style={{ color: "var(--studio-text-muted)" }}>{label}</span>
      <textarea className={INPUT} style={INPUT_STYLE} rows={label === "画面" ? 2 : 1} value={value ?? ""}
        onChange={(e) => onChange(e.target.value)} onBlur={onBlur} />
    </label>
  );
}

/**
 * 出场角色的形象：有两个以上形象的角色一行一个下拉。选的是**这一镜**用哪个，不选 = 跟项目默认。
 * 换了之后这一镜的提示词会标「要重新合成」（描述和参考图都换了）。
 */
/**
 * 加一张参考图：挑资产、用途、写「控制什么」。
 *
 * 例：传送链上挂的一排仿生人和女主同一型号 —— 挂星澜（换成出厂陪伴款形象），
 * 用途选「造型状态」，控制写「传送链上每一台仿生人的外形：银白长发、粉白出厂服」。
 */
function AddSlotRow({ assets, cast, taken, onAdd }: {
  assets: AssetView[];
  /** 这一镜的出场人物。不在里面的人默认只借外形（造型状态） */
  cast: string[];
  taken: string[];
  onAdd: (slot: Omit<RefSlot, "order">) => void;
}) {
  const [assetId, setAssetId] = useState("");
  const [usage, setUsage] = useState<RefSlot["usage"]>("identity");
  const [control, setControl] = useState("");
  const pickable = assets.filter((a) => a.cover_url && !taken.includes(a.id) && ["character", "scene", "prop"].includes(a.kind));
  const kindLabel: Record<string, string> = { character: "角色", scene: "场景", prop: "道具" };
  return (
    <div className="mt-1 grid items-center gap-2 text-[12px]" style={{ gridTemplateColumns: "minmax(0,1.1fr) 96px minmax(0,1.6fr) auto" }}>
      <select aria-label="加哪张参考图" className={INPUT} style={INPUT_STYLE} value={assetId}
        onChange={(e) => {
          setAssetId(e.target.value);
          const a = assets.find((x) => x.id === e.target.value);
          if (a) setUsage(defaultUsage(a.kind, cast.includes(a.id)));
        }}>
        <option value="">＋ 加一张参考图…</option>
        {pickable.map((a) => (
          <option key={a.id} value={a.id}>{kindLabel[a.kind] ?? a.kind} · {a.name}</option>
        ))}
      </select>
      <select aria-label="这张图的用途" className={INPUT} style={INPUT_STYLE} value={usage} disabled={!assetId}
        onChange={(e) => setUsage(e.target.value as RefSlot["usage"])}>
        {MANUAL_USAGES.map((u) => <option key={u} value={u}>{USAGE_LABEL[u]}</option>)}
      </select>
      <input className={INPUT} style={INPUT_STYLE} value={control} disabled={!assetId} placeholder="控制什么，比如「传送链上每台仿生人的外形」"
        onChange={(e) => setControl(e.target.value)} />
      <button type="button" disabled={!assetId || !control.trim()}
        className="rounded-md px-2 py-1 text-[11.5px] disabled:opacity-40"
        style={{ background: "var(--studio-bg-elevated)", boxShadow: "0 0 0 1px var(--studio-border)", color: "var(--studio-text)" }}
        title="挂上之后这一镜的提示词要重新合成"
        onClick={() => {
          onAdd({ asset_id: assetId, usage, control: control.trim(), not_control: "" });
          setAssetId("");
          setControl("");
        }}>
        挂上
      </button>
    </div>
  );
}

function LookRows({ draft, assets, defaultLooks, onPick }: {
  draft: StoryboardShot;
  assets: AssetView[];
  defaultLooks?: Record<string, string>;
  onPick: (assetId: string, lookId: string | null) => void;
}) {
  // 出场人物 + 只当参考图挂上的人物（比如拿女主当同型号仿生人的外形参考，要能换成出厂款）
  // + 道具（协议铭牌：量产款 / 星澜专属款）
  const ids = [...new Set([...(draft.character_ids ?? []), ...(draft.prop_ids ?? []), ...(draft.ref_slots ?? []).map((x) => x.asset_id)])];
  const rows = ids
    .map((id) => assets.find((a) => a.id === id && (a.kind === "character" || a.kind === "prop")))
    .filter((a): a is AssetView => !!a && (a.look_views ?? []).length > 1);
  if (rows.length === 0) return null;
  return (
    <div className="mb-2 flex flex-col gap-1.5">
      {rows.map((a) => {
        const eff = effectiveLook(draft, defaultLooks, a);
        const look = a.look_views.find((l) => l.id === eff.id) ?? a.look_views[0];
        const projectLook = effectiveLook({}, defaultLooks, a);
        const projectName = a.look_views.find((l) => l.id === projectLook.id)?.name ?? "主形象";
        return (
          <div key={a.id} className="flex items-center gap-2 text-[12px]">
            <span className="block h-8 w-6 shrink-0 overflow-hidden rounded" style={{ background: "var(--studio-bg-elevated)" }}>
              {look.cover_url && <NsfwThumb path={look.cover_url} nsfw={look.nsfw} revealKey={`${a.id}/${look.id}`} compact className="h-full w-full object-contain" />}
            </span>
            <span className="w-[72px] shrink-0 truncate" style={{ color: "var(--studio-text-2)" }}>{a.name}</span>
            <select
              aria-label={`${a.name}这一镜的形象`}
              value={eff.overridden ? eff.id : ""}
              onChange={(e) => onPick(a.id, e.target.value || null)}
              className={INPUT}
              style={{ ...INPUT_STYLE, maxWidth: 220 }}
            >
              <option value="">跟项目默认（{projectName}）</option>
              {a.look_views.map((l) => (
                <option key={l.id} value={l.id}>
                  {l.name}{l.nsfw ? " · NSFW" : ""}
                </option>
              ))}
            </select>
            {eff.overridden && <span className="shrink-0 text-[11px]" style={{ color: "var(--studio-accent)" }}>本镜换了形象</span>}
          </div>
        );
      })}
    </div>
  );
}
