/**
 * Step 4 · 分镜。分镜重构第 3 步（见 `docs/plans/2026-09-13-分镜重构方案.md`）。
 *
 * 页头三步进度：确认镜头 → 准备资产 → 合成提示词；左边按场分组的镜头卡；右边当前镜头的详情。
 *
 * - 镜头表由后端 `cinema_generate_shot_table` 生成，会覆盖现有的表。
 * - **生成镜头表是后端任务**：状态、进度、结果备注都在任务登记表里（`useJobs` 读它，
 *   任务一变就刷）。切走再回来、界面刷新都不丢；右下角提示框也跟着同一份任务走。
 * - 改字段失焦就存，存完重新校验。
 * - **合成提示词也是后端任务**（按项目当前线路，整集或单镜）；合成完的提示词在右侧详情里看、改。
 * - 出片在方案第 5 步接上。
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { assetsList, type AssetView } from "../../../api/assets";
import {
  cinemaGenerateShotTable,
  cinemaRegenerateScene,
  cinemaRenderEpisodeVideos,
  cinemaRepairStoryboard,
  cinemaRepairableIssues,
  cinemaIssueFixes,
  type IssueFixOffer,
  cinemaQueueShotVideos,
  cinemaQueueSynthesize,
  cinemaRenderShotVideo,
  cinemaRenderEpisodeFrames,
  cinemaRenderLongTake,
  cinemaSaveEpisode,
  cinemaSynthesizePrompts,
  cinemaRefineTimelines,
  cinemaSaveShot,
  cinemaStalePrompts,
  cinemaValidateStoryboard,
  cinemaShotRoutes,
  type ShotRouteInfo,
  type PromptRoute,
  type ShotIssue,
  type StalePrompts,
  type StoryboardShot,
} from "../../../api/cinema";
import { cinemaCancelJob } from "../../../api/jobs";
import { QueueButton, QueuePanel, useJobs } from "../jobs/QueuePanel";
import { batchWaitingOn } from "../jobs/jobBuckets";
import { batchState, frameBatchState, promptsState, runningKind, shotTableState, timelineState } from "../jobs/shotBusy";
import { StepAction } from "../project/StepAction";
import { RenderOptionsDialog, type RenderTarget } from "../storyboard/RenderOptionsDialog";
import { FrameBatchDialog } from "../storyboard/FrameBatchDialog";
import { MenuItem, MoreMenu, NoticePanel, StepButton, StepCard } from "../storyboard/PipelineBar";
import { ShotDirectorWorld } from "../storyboard/ShotDirectorWorld";
import { SynthesizeDialog } from "../storyboard/SynthesizeDialog";
import { RefineDialog } from "../storyboard/RefineDialog";
import { needsRefine, refineRound } from "../storyboard/timelineDensity";
import { ShotDetail } from "../storyboard/ShotDetail";
import { revealSelected } from "../storyboard/stickyList";
import { CONTEXT_FRAMES, CONTEXT_FRAME_CHOICES, contextFramesHint, longTakeCost, planLongTake } from "../storyboard/longTake";
import { manualSecs, needsRegen, sceneMode, type SceneMode, withSceneMode } from "../storyboard/oneTake";
import { MediaThumb } from "../MediaThumb";
import { useToasts } from "../toastStore";
import { batchRound, batchTargets, batchUnits, framesFirst, longTakeClip, queuePosition, type BatchPlan, nextStep, runningRoute, scenesNeedingRegen, synthLine, toolbarHints, groupByScene, issuesOf, isSynthesized, promptRound, secs, stepStatus, renderTakes } from "../storyboard/storyboardView";
import type { StepProps } from "./types";

export function StoryboardStep({ project, episode, reload, onError }: StepProps) {
  const [queueOpen, setQueueOpen] = useState(false);
  const [assets, setAssets] = useState<AssetView[]>([]);
  const [issues, setIssues] = useState<ShotIssue[]>([]);
  /** 每一镜走首帧出片还是参考出片（Rust `shot_route`）：卡片角标、详情、批量分组都用这一份 */
  const [routes, setRoutes] = useState<ShotRouteInfo[]>([]);
  /** 「自动修正」真能改掉的那几条（后端试修过）。按钮的数字和显不显示都看它 */
  const [repairable, setRepairable] = useState<ShotIssue[]>([]);
  /** 每条问题旁边的「修复」按钮（后端给改法） */
  const [fixes, setFixes] = useState<IssueFixOffer[]>([]);
  const pushToast = useToasts((s) => s.push);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  /** 左边的镜头列表（自己滚）：选中的那一镜不在可见范围里时滚过去 */
  const listEl = useRef<HTMLDivElement | null>(null);
  /** 出片选项卡：批量出片或某一镜出片前弹出，选好画质清晰度等再开始。 */
  /** 合成提示词弹窗：选线路、选范围 */
  const [synthDialog, setSynthDialog] = useState(false);
  const [renderDialog, setRenderDialog] = useState<RenderTarget | null>(null);
  /** 按场出首帧的弹窗 */
  const [frameDialog, setFrameDialog] = useState(false);
  /** 正在工作区里打开导演世界的那一镜 */
  const [directorShotId, setDirectorShotId] = useState<string | null>(null);
  /**
   * 详情面板的版本号。导演世界关掉时 +1，**逼详情面板重挂**：它有本地草稿，
   * 不重挂的话草稿还是打开导演世界之前的，一失焦就把导演世界写进分镜的站位、机位冲回去。
   */
  const [detailRev, setDetailRev] = useState(0);
  /** 重新生成要点两下：会覆盖现有镜头。**不用 `window.confirm`** —— Tauri 的 WKWebView 里它不弹。 */
  const [confirmRegen, setConfirmRegen] = useState(false);
  const projectId = project?.id ?? "";
  const episodeId = episode?.id ?? "";
  // **在早退之前调。** 这个仓库有 hooksOrder 守卫盯着「早退之后还有 hook」。
  const { jobs, buckets, running } = useJobs(projectId, episodeId, queueOpen, () => void reload(), "storyboard");

  const shots = useMemo(() => episode?.storyboard ?? [], [episode?.storyboard]);
  // 最近一次生成镜头表的任务：进度、失败原文、备注都从它来。
  const { job: shotJob, generating, progress, notes } = shotTableState(jobs);
  // 合成提示词那条任务：进度、没通过的镜头、失败原文
  const { job: promptJob, generating: synthesizing, progress: synthProgress, notes: synthNotes, remarks: synthRemarks } = promptsState(jobs);
  const route: PromptRoute = project?.model_settings?.prompt_route ?? "b";
  /** 两条线路各自过期的镜头（卡片上的 A / B 标签用）。分集一刷新就重问 */
  const [staleRoutes, setStaleRoutes] = useState<StalePrompts | null>(null);
  useEffect(() => {
    if (!projectId || !episodeId) return;
    let alive = true;
    void cinemaStalePrompts(projectId, episodeId)
      .then((st) => alive && setStaleRoutes(st))
      .catch(() => alive && setStaleRoutes(null));
    return () => {
      alive = false;
    };
  }, [projectId, episodeId, shots]);

  // 细化时间线那条任务。每细化完一镜就落盘：进度一动就重读；细化的正好是打开着的那一镜，
  // 详情面板要重挂 —— 它有本地草稿，不重挂的话一失焦就把旧时间段存回去，盖掉刚细化的
  const { job: timelineJob, generating: refining, progress: refineProgress } = timelineState(jobs);
  const refineDone = refineProgress?.done ?? 0;
  const [refineDialog, setRefineDialog] = useState(false);

  // 合成任务每做完一镜就落盘：进度一动就重读分集，已合成的镜头马上显示出来，不用等整轮跑完
  const synthDone = synthProgress?.done ?? 0;
  useEffect(() => {
    if (synthesizing && synthDone > 0) void reload();
    // reload 每次渲染都是新函数，放进依赖会一直重读
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [synthesizing, synthDone]);

  useEffect(() => {
    if (!refining || refineDone === 0) return;
    // 刚细化完的那一镜（后端细化完才把进度推到 done，标签还是那一镜）
    const n = Number(/第\s*(\d+)\s*镜/.exec(refineProgress?.label ?? "")?.[1] ?? NaN);
    const shownId = selectedId ?? groupByScene(shots)[0]?.shots[0]?.id ?? null;
    const shown = shots.find((s) => s.id === shownId);
    void reload().then(() => {
      if (shown && shown.shot_num === n) setDetailRev((r) => r + 1);
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [refining, refineDone]);

  const refreshIssues = useCallback(async () => {
    if (!projectId || !episodeId) return;
    try {
      setIssues(await cinemaValidateStoryboard(projectId, episodeId));
    } catch {
      setIssues([]);
    }
    setRoutes(await cinemaShotRoutes(projectId, episodeId).catch(() => []));
    // 单独取：它要多跑一遍修正和校验，失败了不该连问题清单一起没了
    setRepairable(await cinemaRepairableIssues(projectId, episodeId).catch(() => []));
    setFixes(await cinemaIssueFixes(projectId, episodeId).catch(() => []));
  }, [projectId, episodeId]);

  useEffect(() => {
    if (!projectId) return;
    let alive = true;
    void Promise.all([
      assetsList({ kind: "project", project_id: projectId }),
      episodeId ? assetsList({ kind: "episode", project_id: projectId, episode_id: episodeId }) : Promise.resolve([] as AssetView[]),
      assetsList({ kind: "global" }).catch(() => [] as AssetView[]),
    ])
      .then(([p, e, g]) => alive && setAssets([...e, ...p, ...g]))
      .catch(() => alive && setAssets([]));
    return () => {
      alive = false;
    };
  }, [projectId, episodeId, episode?.storyboard]);

  useEffect(() => {
    void refreshIssues();
  }, [refreshIssues, episode?.storyboard]);

  // 列表自己滚之后，选中的那一镜可能在列表外面（从队列跳过来）—— 滚过去
  useEffect(() => revealSelected(listEl.current, selectedId), [selectedId]);

  if (!project || !episode) return null;

  const status = stepStatus(shots, issues, assets, route, promptJob);
  const groups = groupByScene(shots);
  const selected = shots.find((s) => s.id === selectedId) ?? groups[0]?.shots[0] ?? null;

  const generate = async () => {
    if (!episode.script_text.trim()) {
      onError("先去剧本那一步写好剧本 —— 镜头是从剧本生成的");
      return;
    }
    if (shots.length && !confirmRegen) {
      setConfirmRegen(true);
      return;
    }
    setConfirmRegen(false);
    onError(null);
    setSelectedId(null);
    try {
      // 立刻返回登记的任务；之后的进度、结果都走任务事件。
      await cinemaGenerateShotTable(project.id, episode.id);
    } catch (e) {
      onError(String(e));
    }
  };

  /** 单镜合成：这一集在合成时排到那一轮后面（加入合成队列），没在合成就直接开一轮 */
  const queueSynthesize = (shotId: string) => {
    onError(null);
    cinemaQueueSynthesize(project.id, episode.id, [shotId]).catch((e) => onError(String(e)));
  };

  /** 细化时间线。立即返回，进度走任务。 */
  const refine = async (shotIds: string[]) => {
    onError(null);
    try {
      await cinemaRefineTimelines(project.id, episode.id, shotIds);
    } catch (e) {
      onError(String(e));
    }
  };

  const { job: batchJob, generating: batching, progress: batchProgress, notes: batchNotes } = batchState(jobs);
  const { generating: framing, progress: framingProgress } = frameBatchState(jobs);
  const videoRunning = batching || jobs.some((j) => j.kind === "video" && j.status === "running");
  /** 出片。立即登记任务、后台跑；结果走任务事件和刷新（出片要几分钟，不在这里等）。有片在出时排到后面（加入队列） */
  const render = (shotId: string) => {
    onError(null);
    if (videoRunning) {
      cinemaQueueShotVideos(project.id, episode.id, [shotId]).catch((e) => onError(String(e)));
      return;
    }
    cinemaRenderShotVideo({ project_id: project.id, episode_id: episode.id, shot_id: shotId }).catch((e) => onError(String(e)));
  };
  const batch = batchTargets(shots, route, issues, episode.long_take_scenes ?? [], shots, episode.scene_renders ?? []);
  // 出片弹窗里能换线路：当前线路没有能出的，另一条有，按钮也要能点
  const otherBatch = batchTargets(shots, route === "a" ? "b" : "a", issues, episode.long_take_scenes ?? [], shots, episode.scene_renders ?? []);
  // **长镜头的场也算「有得出」** —— 不算的话，一集全是长镜头时
  // 批量出片按钮是灰的，而它明明有活可干
  const anyBatch =
    batch.ids.length > 0 || otherBatch.ids.length > 0 ||
    batch.longTakeScenes.length > 0 || otherBatch.longTakeScenes.length > 0;
  const hints = toolbarHints({ route, shots: shots.length, generating, synthesizing, videoRunning, batch, otherBatch: otherBatch.ids.length, synthesized: status.synthesized, stale: status.stale });
  // 还没首帧也没合成的：先出首帧、出完自动合成一次，别先合成（见 `framesFirst`）
  const pendingFrames = framesFirst(shots, route).length;
  const next = nextStep({ shots: shots.length, synthesized: status.synthesized, stale: status.stale, canRender: anyBatch, busy: generating || synthesizing || videoRunning || framing, framesFirst: pendingFrames });
  /**
   * 批量出片：**长镜头的场整段出，其余逐镜出，同一个任务按场次顺序出完。**
   *
   * 以前长镜头先单独提交、再提交逐镜那批 —— 后一下会被「这一集有镜头正在出片」拒掉
   * （2026-09-29 夜里其余 32 镜一整夜没排上）。顺序见 `batchUnits`。
   *
   * 整段出片那个单独的按钮还留着（只想重出某一场时用），但不必须先点它。
   */
  const renderAll = async (plan: BatchPlan) => {
    onError(null);
    try {
      const units = batchUnits(shots, plan.ids, plan.longTakeScenes);
      if (units.length) await cinemaRenderEpisodeVideos(project.id, episode.id, units);
    } catch (e) {
      onError(String(e));
    }
  };

  /** 按场出首帧。后台任务，每一镜各登记一条首帧任务 */
  const renderFrames = async (ids: string[], redo: boolean) => {
    onError(null);
    try {
      await cinemaRenderEpisodeFrames(project.id, episode.id, ids, redo);
    } catch (e) {
      onError(String(e));
    }
  };

  /** 只重新生成一场。后台任务，进度走生成镜头表那条 */
  const regenerateScene = async (sceneNo: number) => {
    onError(null);
    try {
      await cinemaRegenerateScene(project.id, episode.id, sceneNo);
    } catch (e) {
      onError(String(e));
    }
  };
  const regenScenes = scenesNeedingRegen(shots, issues);

  /**
   * 换一场的出法（分镜 / 一镜出 / 长镜头）。**只存设置，不自动重新生成** ——
   * 镜头对不上时，场标题下面会出一个「重新生成这一场」的按钮，点了才换（不静默覆盖已有镜头）。
   */
  const onSetSceneMode = async (sceneNo: number, mode: SceneMode, secs: number | null = null) => {
    onError(null);
    try {
      await cinemaSaveEpisode(project.id, { ...episode, ...withSceneMode(episode, sceneNo, mode, secs) });
      await reload();
    } catch (e) {
      onError(String(e));
    }
  };

  /** 整集默认不分镜。**已经生成的镜头不动**，重新生成镜头表（或单场重新生成）才按新设置来 */
  const onToggleEpisodeOneTake = async (on: boolean) => {
    onError(null);
    try {
      await cinemaSaveEpisode(project.id, { ...episode, one_take: on });
      await reload();
    } catch (e) {
      onError(String(e));
    }
  };

  /** 换重叠帧数。存在集上 —— 一集里的长镜头共用一档。 */
  const onSetContextFrames = async (frames: number) => {
    onError(null);
    try {
      await cinemaSaveEpisode(project.id, { ...episode, long_take_context_frames: frames });
      await reload();
    } catch (e) {
      onError(String(e));
    }
  };

  const onRenderLongTake = async (sceneNo: number) => {
    onError(null);
    try {
      await cinemaRenderLongTake(project.id, episode.id, sceneNo);
    } catch (e) {
      onError(String(e));
    }
  };

  /**
   * 详情面板存一镜。**只存这一镜**（`cinema_save_shot`）：以前整集保存，后台刚细化 / 合成写回的别的镜头
   * 会被界面里还没刷新的旧版本盖掉。出片结果、关键帧、导演世界场面后端以盘上为准。回存没存上。
   */
  /** 「自动修正」—— 顶上那个按钮和问题旁边的「自动修正」共用 */
  const runRepair = () =>
    cinemaRepairStoryboard(project.id, episode.id)
      .then((n) => {
        // **点了要有回音**：改了几处、剩下的为什么改不了（2026-09-30「点了没反应」）
        const left = issues.length - repairable.length;
        pushToast(
          n > 0
            ? { kind: "success", title: `自动修正改了 ${n} 处`, body: left > 0 ? `还剩 ${left} 条要手动改或重新生成，见下面的提示。` : undefined }
            : { kind: "error", title: "没有能自动改的", body: "剩下的问题代码改不了，看每一镜下面的提示手动改。" },
        );
        return reload();
      })
      .catch((e) => onError(String(e)));

  const saveShot = async (edited: StoryboardShot, base: StoryboardShot): Promise<boolean> => {
    try {
      await cinemaSaveShot(project.id, episode.id, edited, base);
      await reload();
      return true;
    } catch (e) {
      onError(`保存失败：${e}`);
      return false;
    }
  };

  return (
    // **整页正好一屏高，自己不滚**（2026-10-06 用户：「顶部 4 步骤卡片不参与下拉页面」）：
    // 顶上的卡片和提醒栏固定，下面的镜头列表和详情各自滚。以前整页跟着滚、列表 sticky 贴顶，
    // 列表高度按整个滚动区算 —— 页面没滚下去时列表底部被卡片顶出屏幕，右边详情短的话永远拉不到底
    <div className="relative flex h-full min-h-0 flex-col gap-3 px-7 pb-4 pt-5">
      {synthDialog && (
        <SynthesizeDialog
          project={project}
          episodeId={episodeId}
          shots={shots}
          issues={issues}
          running={runningRoute(promptJob)}
          progress={synthProgress}
          onClose={() => setSynthDialog(false)}
          // 弹窗里要看得见报错（另一条线路还在跑之类），所以这里不吞错
          onStart={(route, ids) => cinemaSynthesizePrompts(project.id, episode.id, ids, route).then(() => undefined)}
        />
      )}

      {refineDialog && (
        <RefineDialog
          shots={shots}
          route={route}
          issues={issues}
          onClose={() => setRefineDialog(false)}
          onStart={(ids) => void refine(ids)}
        />
      )}

      {renderDialog && (
        <RenderOptionsDialog
          project={project}
          target={renderDialog}
          shots={shots}
          issues={issues}
          routes={routes}
          onClose={() => setRenderDialog(null)}
          onSaved={() => void reload()}
          longTakeScenes={episode.long_take_scenes ?? []}
          sceneRenders={episode.scene_renders ?? []}
          onStart={(p) => (renderDialog.kind === "single" ? p.ids[0] && render(p.ids[0]) : void renderAll(p))}
        />
      )}

      {directorShotId && (() => {
        const shot = shots.find((s) => s.id === directorShotId);
        if (!shot) return null;
        return (
          <ShotDirectorWorld
            projectId={project.id}
            episodeId={episode.id}
            aspect={project.aspect_ratio}
            shot={shot}
            shots={shots}
            sceneAsset={assets.find((a) => a.id === shot.scene_id && a.kind === "scene") ?? null}
            onError={(m) => onError(m)}
            onClose={() => {
              setDirectorShotId(null);
              setDetailRev((n) => n + 1);
              void reload();
            }}
          />
        );
      })()}

      {frameDialog && (
        <FrameBatchDialog
          project={project}
          shots={shots}
          issues={issues}
          routes={routes}
          onClose={() => setFrameDialog(false)}
          onSaved={() => void reload()}
          onStart={(ids, redo) => void renderFrames(ids, redo)}
        />
      )}

      {queueOpen && (
        <QueuePanel
          projectId={project.id}
          episodeId={episode.id}
          jobs={jobs}
          buckets={buckets}
          onClose={() => setQueueOpen(false)}
          onJump={(shotId) => {
            setQueueOpen(false);
            setSelectedId(shotId);
          }}
        />
      )}

      <StepAction>
        <QueueButton running={running} total={jobs.length} open={queueOpen} onToggle={() => setQueueOpen((v) => !v)} />
      </StepAction>

      {/* 四步：镜头 → 首帧（可选）→ 提示词 → 出片（首帧在前：先合成的话出完首帧要再合一遍）。每一步的状态和按钮在同一张卡片里，只有下一步高亮（见 PipelineBar） */}
      <section className="grid shrink-0 grid-cols-1 gap-2 md:grid-cols-2 xl:grid-cols-4" aria-label="分镜流程">
        <StepCard
          no={1}
          title="镜头"
          done={status.shots > 0 && status.shotsWithIssues === 0}
          current={next === "shots"}
          status={status.shots ? `${status.shots} 镜 · ${status.shotsWithIssues ? `${status.shotsWithIssues} 个有问题` : "没有问题"}` : "还没有镜头"}
          detail={
            status.shots === 0
              ? undefined
              : status.missingAssets || status.unlinkedAssets
                ? `资产 ${[status.missingAssets ? `${status.missingAssets} 个没图` : "", status.unlinkedAssets ? `${status.unlinkedAssets} 个库里没有` : ""].filter(Boolean).join(" · ")}（按文字兜底）`
                : "资产都有图"
          }
          running={
            generating
              ? progress && progress.total > 0
                ? `生成中 · 第 ${Math.min(progress.done + 1, progress.total)}/${progress.total} 场`
                : "生成中…"
              : refining
                ? refineProgress && refineProgress.total > 0
                  ? `细化中 · ${Math.min(refineProgress.done + 1, refineProgress.total)}/${refineProgress.total}`
                  : "细化中…"
                : undefined
          }
        >
          {shots.length === 0 ? (
            <StepButton primary={next === "shots"} disabled={generating} title={hints.regen} onClick={() => void generate()}>
              由剧本生成镜头
            </StepButton>
          ) : (
            <>
              <StepButton
                disabled={generating || refining}
                onClick={() => setRefineDialog(true)}
                title="把一笔带过的时间段写细：同一条视频，只把每一刻发生什么写满（结构不动）。细化后要重新合成提示词"
              >
                {shots.filter(needsRefine).length ? `细化 ${shots.filter(needsRefine).length} 镜` : "细化时间线"}
              </StepButton>
              {repairable.length > 0 && (
                <StepButton
                  disabled={generating}
                  onClick={() => void runRepair()}
                  title="夹英文、「或者」二选一、台词在声音里重复、一镜内「切至」、说话人不在出场人物里、原文里编的句子和标记 —— 这几类由代码直接改掉，不用重新生成。漏拍的剧本句子修不了（是少了一个镜头），要重新生成镜头表"
                >
                  自动修正 {repairable.length} 处
                </StepButton>
              )}
              <MoreMenu>
                {/* **会覆盖整张表，要点两次。** 不用 `window.confirm` —— Tauri 的 WKWebView 里它不弹 */}
                <MenuItem danger disabled={generating} title={hints.regen} onClick={() => void generate()} onBlur={() => setConfirmRegen(false)}>
                  {confirmRegen ? `会覆盖现在的 ${shots.length} 个镜头，再点一次确认` : "重新生成镜头"}
                </MenuItem>
                <label
                  className="flex items-center gap-1.5 rounded-md px-2.5 py-1.5 text-[12px]"
                  style={{ opacity: generating ? 0.6 : 1 }}
                  title="短片用：每场不拆镜头，15 秒以内一镜出，更长的等分成几个 15 秒以内的镜头整段出；镜头里按时间轴写切镜和运镜。已经生成的镜头不动，重新生成镜头表后生效。单场可以在场标题下面改回分镜"
                >
                  <input type="checkbox" checked={!!episode.one_take} disabled={generating} onChange={(e) => void onToggleEpisodeOneTake(e.target.checked)} />
                  <span>这一集不分镜（每场一镜出）</span>
                </label>
              </MoreMenu>
            </>
          )}
        </StepCard>

        <StepCard
          no={2}
          title="首帧"
          optional
          current={next === "frames"}
          status={`有首帧 ${shots.filter((s) => s.keyframes?.first).length}/${shots.length}`}
          detail={pendingFrames ? `先出首帧再合成：出完自动合成那几镜（只合一次，不花第二轮）` : "出完自动按首帧合成那几镜"}
          running={
            framing
              ? framingProgress && framingProgress.total > 0
                ? `出首帧中 · ${Math.min(framingProgress.done + 1, framingProgress.total)}/${framingProgress.total}`
                : "出首帧中…"
              : undefined
          }
        >
          {shots.length > 0 && (
            <StepButton
              primary={next === "frames"}
              disabled={generating || synthesizing || framing}
              onClick={() => setFrameDialog(true)}
              title="先出首帧锁住画面（场景、机位、道具摆放），出片就从这张图开始动。不用先合成提示词（按镜头表现拼画面，不花 token），出完自动按首帧合成一次。单镜的首帧 / 尾帧在右边详情里出或导入"
            >
              按场出首帧
            </StepButton>
          )}
        </StepCard>

        <StepCard
          no={3}
          title="提示词"
          done={status.shots > 0 && status.synthesized === status.shots && status.stale === 0}
          current={next === "synth"}
          status={(["a", "b"] as PromptRoute[]).map((r) => `${r.toUpperCase()} ${shots.filter((s) => isSynthesized(s, r)).length}/${shots.length}${r === route ? "（出片用）" : ""}`).join(" · ")}
          detail={
            pendingFrames
              ? `${pendingFrames} 镜还没首帧：要出首帧的先出（出完自动合成），不出首帧的直接合成`
              : synthLine(shots, route, staleRoutes)
          }
          // 没首帧、等着出首帧的那些没合成是正常的，不标红；过期的、或有了首帧却没合成的（自动合成没成）才标
          warn={shots.length > 0 && (status.stale > 0 || status.synthesized < status.shots - pendingFrames)}
          running={
            synthesizing
              ? `合成中 · 线路 ${(runningRoute(promptJob) ?? route).toUpperCase()}${synthProgress && synthProgress.total > 0 ? ` · ${Math.min(synthProgress.done + 1, synthProgress.total)}/${synthProgress.total}` : "…"}`
              : undefined
          }
        >
          {shots.length > 0 && (
            <StepButton primary={next === "synth"} disabled={generating} title={hints.synth} onClick={() => setSynthDialog(true)}>
              合成 A / B
            </StepButton>
          )}
        </StepCard>

        <StepCard
          no={4}
          title="出片"
          done={shots.length > 0 && shots.every((s) => renderTakes(s, route).length > 0)}
          current={next === "render"}
          status={`已出 ${shots.filter((s) => renderTakes(s, route).length > 0).length}/${shots.length}`}
          detail={
            !anyBatch
              ? "先合成提示词"
              : [
                  // **长镜头的场也算在这一次里**，不说的话人会以为它没被排上
                  `能出 ${[batch.longTakeScenes.length ? `${batch.longTakeScenes.length} 场长镜头` : "", batch.ids.length ? `${batch.ids.length} 镜` : ""].filter(Boolean).join(" + ") || "0 镜"}`,
                  batch.blocked ? `${batch.blocked} 镜没合成或合成后改过，会跳过` : "",
                ]
                  .filter(Boolean)
                  .join(" · ")
          }
          running={(() => {
            if (!batching) return undefined;
            // 还在等前一镜（重启后续接的 / 单镜出片后加的队列）：和右下角通知条同一个说法，
            // 不然这里「1/1」、那边「还剩 2 镜」对不上（2026-10-06 用户截图）
            const waiting = batchJob ? batchWaitingOn(batchJob, jobs) : undefined;
            if (waiting) return `批量出片中 · 第 ${waiting.shot_num || "?"} 镜在出 · 还剩 ${(batchJob?.targets?.length ?? 0) + 1} 镜`;
            return batchProgress && batchProgress.total > 0
              ? `批量出片中 · ${Math.min(batchProgress.done + 1, batchProgress.total)}/${batchProgress.total}`
              : "批量出片中…";
          })()}
        >
          {shots.length > 0 && (
            <>
              <StepButton primary={next === "render"} disabled={generating || synthesizing || videoRunning || !anyBatch} title={hints.batch} onClick={() => setRenderDialog({ kind: "batch" })}>
                {batch.redo ? "全部重出" : "批量出片"}
              </StepButton>
              <StepButton disabled={generating || synthesizing || videoRunning || !anyBatch} title="勾一场或几场，只出这几场的镜头" onClick={() => setRenderDialog({ kind: "scenes", sceneNos: [] })}>
                按场出片
              </StepButton>
              {batching && batchJob && (
                <StepButton
                  danger
                  disabled={!!batchJob.cancel_requested}
                  onClick={() => void cinemaCancelJob(batchJob.id).catch((e) => onError(String(e)))}
                  title="正在出的那一镜会跑完（H3 中途停不下来），之后不再继续"
                >
                  {batchJob.cancel_requested ? "停止中 · 当前这镜跑完就停" : "停止批量出片"}
                </StepButton>
              )}
            </>
          )}
        </StepCard>
      </section>

      {/*
        **一条提醒栏，不是一摞横幅。** 没成功 / 没保存的默认展开、标红（失败原文照抄）；
        存好了、只是有话要说的默认收起 —— 2026-09-20 之前这两类混在一个标题「没通过检查（没有保存）」下面，存好了的镜头被读成失败
      */}
      {/* 提醒栏展开了也只占一截，自己滚 —— 不然一展开就把下面的列表挤没了 */}
      <div className="max-h-[30vh] shrink-0 overflow-y-auto empty:hidden">
      <NoticePanel
        errors={[
          ...(shotJob?.status === "failed" && !shotJob.superseded ? [{ title: `上一次生成镜头没成功：${shotJob.error}` }] : []),
          ...(promptJob?.status === "failed" && !promptJob.superseded ? [{ title: `上一次合成提示词没成功：${promptJob.error}` }] : []),
          ...(batchJob?.status === "failed" && !batchJob.superseded ? [{ title: `上一次批量出片没出成：${batchJob.error}` }] : []),
          ...(batchNotes.length > 0
            ? [{ title: `上一次批量出片${batchJob?.summary ? `（${batchJob.summary}）` : ""}有 ${batchNotes.length} 镜跳过或失败，可以单独重出：`, items: batchNotes }]
            : []),
          ...(synthNotes.length > 0 ? [{ title: `上一次合成有 ${synthNotes.length} 镜没通过检查（没有保存，可以单独重新合成）：`, items: synthNotes }] : []),
        ]}
        infos={[
          ...(synthRemarks.length > 0
            ? [{ title: `上一次合成有 ${synthRemarks.length} 镜已经存下来了，但有话要说（不影响出片；想更好可以单独重合成）：`, items: synthRemarks }]
            : []),
          ...(notes.length > 0 ? [{ title: `生成时有 ${notes.length} 处需要你知道：`, items: notes }] : []),
        ]}
      />
      </div>

      {shots.length === 0 ? (
        <section className="studio-tile px-5 py-8 text-center">
          <p className="text-[13px]" style={{ color: "var(--studio-text-2)" }}>
            {generating ? "正在读剧本、分场、拆镜头，一场一场来，可以先去别的步骤。" : "还没有镜头。点右上角「由剧本生成镜头」。"}
          </p>
        </section>
      ) : (
        <div className="grid min-h-0 flex-1 gap-3" style={{ gridTemplateColumns: "minmax(240px, 300px) minmax(0, 1fr)", gridTemplateRows: "minmax(0, 1fr)" }}>
          {/* **左边的镜头列表、右边的详情各自滚**（用户 2026-09-29 / 10-06）：网格占满卡片下面剩下的高度
              （`flex-1` + 一行 `minmax(0, 1fr)`），两栏都是 `min-h-0 overflow-y-auto`。
              **四周留 2px（p-0.5，再用 -ml-0.5 拉回对齐）**：选中框是画在按钮外面的 1.5px 描边，
              自己滚的容器会把边缘外的东西裁掉，不留的话选中框左边被切掉一截（用户 09-29 截图） */}
          <div ref={listEl} className="-ml-0.5 flex min-h-0 flex-col gap-2 overflow-y-auto overscroll-contain p-0.5 pr-1">
            {groups.map((g) => (
              <div key={g.sceneNo} className="flex flex-col gap-1.5">
                <div className="flex items-center gap-2 px-1 text-[11.5px]" style={{ color: "var(--studio-text-muted)" }}>
                  {/* 场名很长（「盛唐 · 金銮殿 · 日（第三幕）」），一行截断、悬停看全名，不挤掉右边的按钮 */}
                  <span className="min-w-0 flex-1 truncate" title={g.sceneNo ? `第 ${g.sceneNo} 场 · ${g.sceneName}` : g.sceneName}>
                    {g.sceneNo ? `第 ${g.sceneNo} 场 · ${g.sceneName}` : g.sceneName}
                  </span>
                  {regenScenes.includes(g.sceneNo) && (
                    <button
                      type="button"
                      disabled={generating}
                      onClick={() => void regenerateScene(g.sceneNo)}
                      title="这一场有漏拍的剧本句子。只重新生成这一场，其它场的镜头、提示词、出过的片都不动"
                      className="shrink-0 whitespace-nowrap rounded-md px-2 py-0.5 text-[11px]"
                      style={{
                        color: "var(--studio-failed-fg)",
                        background: "color-mix(in srgb, var(--studio-failed-fg) 12%, transparent)",
                        boxShadow: "0 0 0 1px color-mix(in srgb, var(--studio-failed-fg) 35%, transparent)",
                        opacity: generating ? 0.6 : 1,
                      }}
                    >
                      重生成
                    </button>
                  )}
                  <span className="shrink-0 whitespace-nowrap">{g.shots.length} 镜</span>
                </div>

                {/*
                  **长镜头：一场整段出。**

                  用户 2026-09-20：「我有一个分集……就是要一个 1 分钟的长镜头，
                  就 1 场，室内打斗……但是不分镜，就可以勾选这个选项」。

                  **不能做成一个安静的勾选框** —— 它把「一镜一个任务、随时能停、
                  能单独重做」换成了「一场一个任务、几小时、中途不能改、
                  某一段不满意要整条重来」。所以代价当场逐条列出来。
                */}
                {g.sceneNo > 0 && (() => {
                  const frames = episode.long_take_context_frames ?? CONTEXT_FRAMES;
                  const p = planLongTake(g.shots, frames);
                  const on = (episode.long_take_scenes ?? []).includes(g.sceneNo);
                  /*
                    **出法三选一**（2026-09-23 加了「一镜出」）：分镜 / 一镜出 / 长镜头。
                    一镜出两镜以上时后端自动勾上长镜头，所以下面那块长镜头的代价说明照样出现。
                  */
                  const mode = sceneMode(episode, g.sceneNo);
                  const manual = manualSecs(episode, g.sceneNo);
                  const regen = needsRegen(episode, g.sceneNo, shots);
                  const total = g.shots.reduce((a, x) => a + x.duration, 0);
                  // 整段出过的片：整条记在场上（不切开），在这里看
                  // 重新生成过这一场（镜头换了）的旧整段片不算（`sceneRenderMatches`）
                  const done = on ? longTakeClip(episode.long_take_scenes ?? [], episode.scene_renders ?? [], g.sceneNo, route, episode.storyboard) : null;
                  return (
                    <div className="mx-1 rounded-md px-2 py-1.5 text-[11px]" style={{ background: "var(--studio-bg-inset)" }}>
                      <div className="flex flex-wrap items-center gap-1.5">
                        <span style={{ color: "var(--studio-text-muted)" }}>出法</span>
                        <select
                          value={mode}
                          disabled={generating}
                          onChange={(e) => void onSetSceneMode(g.sceneNo, e.target.value as SceneMode, manual)}
                          className="rounded-md px-1 py-0.5 text-[11px]"
                          style={{ background: "var(--studio-bg-input)", color: "var(--studio-text)" }}
                        >
                          <option value="split" className="bg-zinc-900">分镜</option>
                          <option value="one_take" className="bg-zinc-900">一镜出（不分镜）</option>
                          <option value="long_take" disabled={!p.ok && mode !== "long_take"} className="bg-zinc-900">长镜头（镜头等长、整段出）</option>
                        </select>
                        {mode === "one_take" && (
                          <label className="flex items-center gap-1" title="留空 = 自动：剧本里写明的时长（「全片 15 秒」）> 整集只有一场时用分集时长 > 按台词字数 ÷ 4 + 动作估">
                            <input
                              key={`${g.sceneNo}-${manual ?? ""}`}
                              type="number"
                              min={4}
                              max={60}
                              step={0.5}
                              defaultValue={manual ?? ""}
                              placeholder="自动"
                              disabled={generating}
                              onBlur={(e) => {
                                const v = Number(e.target.value) || null;
                                if (v !== manual) void onSetSceneMode(g.sceneNo, "one_take", v);
                              }}
                              onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
                              className="w-14 rounded-md px-1 py-0.5 text-[11px]"
                              style={{ background: "var(--studio-bg-input)", color: "var(--studio-text)" }}
                            />
                            <span style={{ color: "var(--studio-text-muted)" }}>秒</span>
                          </label>
                        )}
                      </div>
                      {mode === "one_take" && !regen && (
                        <p className="mt-1" style={{ color: "var(--studio-text-muted)" }}>
                          {g.shots.length} 镜 × {secs(g.shots[0]?.duration ?? 0)} 秒 = {secs(total)} 秒 · 镜内按时间轴切镜
                          {g.shots.length > 1 ? " · 超过 15 秒已等分，整段出" : ""}
                        </p>
                      )}
                      {regen && (
                        <button
                          type="button"
                          disabled={generating}
                          onClick={() => void regenerateScene(g.sceneNo)}
                          title="只重新生成这一场，其它场的镜头、提示词、出过的片都不动"
                          className="mt-1.5 rounded-md px-2 py-1 text-left"
                          style={{ background: "var(--studio-primary)", color: "var(--studio-on-primary)", opacity: generating ? 0.6 : 1 }}
                        >
                          {regen} · 重新生成这一场
                        </button>
                      )}
                      {done && (
                        <div className="mt-1.5 flex items-start gap-2">
                          <MediaThumb path={done.path} kind="video" className="aspect-video w-48 shrink-0 rounded-md object-contain" />
                          <p className="leading-relaxed" style={{ color: "var(--studio-text-2)" }}>
                            已整段出过 · 线路 {done.route.toUpperCase()}{done.quality ? ` · ${done.quality}` : ""} · {new Date(done.created_at * 1000).toLocaleString()}
                            <br />
                            <span style={{ color: "var(--studio-text-muted)" }}>批量出片会跳过这一场；导出到 FCP 时这一场放这一整条</span>
                          </p>
                        </div>
                      )}
                      {mode === "long_take" && !p.ok ? (
                        <p className="mt-1" style={{ color: "var(--studio-text-muted)" }}>{p.why}</p>
                      ) : !p.ok || regen ? null : on ? (
                        <>
                          <ul className="mt-1 list-disc pl-4" style={{ color: "var(--studio-text-muted)" }}>
                            {longTakeCost(p).map((c) => <li key={c}>{c}</li>)}
                          </ul>
                          {p.warning && (
                            <p className="mt-1" style={{ color: "var(--studio-accent)" }}>{p.warning}</p>
                          )}
                          {/*
                            **重叠帧数可选。** 短窗口配大重叠是在白烧算力 ——
                            3 秒窗口配 22 帧，31% 的采样发布时会被裁掉。
                            而重叠越小，台词落在裁切点中间的概率也越小。
                          */}
                          <label className="mt-1.5 flex items-center gap-1.5">
                            <span style={{ color: "var(--studio-text-muted)" }}>窗口重叠</span>
                            <select
                              value={frames}
                              disabled={generating}
                              onChange={(e) => void onSetContextFrames(Number(e.target.value))}
                              className="rounded-md px-1 py-0.5 text-[11px]"
                              style={{ background: "var(--studio-bg-input)", color: "var(--studio-text)" }}
                            >
                              {CONTEXT_FRAME_CHOICES.map((f) => (
                                <option key={f} value={f} className="bg-zinc-900">{f} 帧</option>
                              ))}
                            </select>
                          </label>
                          <p style={{ color: "var(--studio-text-muted)" }}>{contextFramesHint(frames, p.windowSecs)}</p>
                          <button
                            type="button"
                            disabled={generating}
                            onClick={() => void onRenderLongTake(g.sceneNo)}
                            className="mt-1.5 rounded-md px-2 py-1"
                            style={{ background: "var(--studio-primary)", color: "var(--studio-on-primary)", opacity: generating ? 0.6 : 1 }}
                          >
                            {done ? "重新整段出片" : "整段出片"}
                          </button>
                        </>
                      ) : mode === "long_take" ? (
                        <p className="mt-1" style={{ color: "var(--studio-text-muted)" }}>
                          {p.windows} 镜 × {p.windowSecs} 秒 → {p.netSecs.toFixed(1)} 秒一镜到底
                        </p>
                      ) : null}
                    </div>
                  );
                })()}

                {(() => {
                /*
                  **整段出的场：这一场的镜头卡连成一组。** 用户 10-01：第 5 场是一条长镜头，
                  左边却是三张互不相干的卡片，看不出它们是一起出的。内容不变，只是连起来 ——
                  外框一条主色边 + 左侧一道竖线，卡片之间只留一条细分隔线。
                */
                const joined = g.sceneNo > 0 && (episode.long_take_scenes ?? []).includes(g.sceneNo);
                const clip = joined ? longTakeClip(episode.long_take_scenes ?? [], episode.scene_renders ?? [], g.sceneNo, route, episode.storyboard) : null;
                const cards = g.shots.map((s, k) => {
                  const own = issuesOf(issues, s);
                  const on = selected?.id === s.id;
                  return (
                    <button
                      key={s.id}
                      type="button"
                      data-shot-id={s.id}
                      onClick={() => setSelectedId(s.id)}
                      className={`flex flex-col gap-1 px-3 py-2 text-left ${joined ? "" : "rounded-xl"}`}
                      style={
                        joined
                          ? {
                              background: on ? "color-mix(in srgb, var(--studio-primary) 10%, var(--studio-bg-inset))" : "var(--studio-bg-inset)",
                              boxShadow: on ? "inset 0 0 0 1.5px var(--studio-primary)" : undefined,
                              borderTop: k > 0 ? "1px solid var(--studio-border-subtle)" : undefined,
                            }
                          : {
                              background: "var(--studio-bg-inset)",
                              boxShadow: `0 0 0 ${on ? 1.5 : 1}px ${on ? "var(--studio-primary)" : "var(--studio-border-subtle)"}`,
                            }
                      }
                    >
                      <span className="text-[12.5px] font-medium">
                        第 {s.shot_num} 镜 · {secs(s.duration)} 秒
                      </span>
                      <span className="line-clamp-2 text-[11px]" style={{ color: "var(--studio-text-2)" }}>
                        {(s.segments ?? []).map((seg) => seg.title || seg.visual).filter(Boolean).join("；") || s.prompt || "（没有画面描述）"}
                      </span>
                      <span className="flex flex-wrap gap-1 text-[10.5px]">
                        {own.length > 0 && <Tag tone="danger">{own.length} 个问题</Tag>}
                        {(() => {
                          // 走哪条链路：有首帧 = 首帧出片。被拦（首帧里缺中途进场的人）标红
                          const ri = routes.find((r) => r.shot_id === s.id);
                          if (!ri) return null;
                          return (
                            <Tag tone={ri.blocker ? "danger" : ri.actual === "keyframe" ? "keyframe" : "reference"} title={ri.blocker ?? (ri.actual !== ri.advised ? `按分镜内容建议${ri.advised === "keyframe" ? "首帧" : "参考"}出片：${ri.reason}` : ri.reason)}>
                              {ri.actual === "keyframe" ? "首帧出片" : "参考出片"}
                              {ri.blocker ? " · 被拦" : ri.actual !== ri.advised ? " · 建议改" : ""}
                            </Tag>
                          );
                        })()}
                        {(() => {
                          const r = batchRound(s, batchJob, runningKind(jobs, s.id) === "video");
                          if (r) return <Tag tone="muted">{r === "running" ? "出片中" : "排队出片"}</Tag>;
                          const n = renderTakes(s, route).length;
                          if (joined) {
                            return (
                              <>
                                {clip && <Tag tone="done">整场已出</Tag>}
                                {n > 0 && <Tag tone="muted">单独出 {n} 次</Tag>}
                              </>
                            );
                          }
                          return n > 0 ? <Tag tone="done">已出片 {n} 次</Tag> : null;
                        })()}
                        {(["a", "b"] as PromptRoute[]).map((r) => {
                          const round = r === route ? promptRound(s, promptJob, route) : null;
                          // 合成过但过期了（比如刚出了首帧）不能还显示绿色「已合成」—— 按线路问的过期名单说了算
                          const stale = isSynthesized(s, r) && !!staleRoutes?.[r].includes(s.id);
                          const label = round === "running" ? "合成中" : round === "queued" ? "排队中" : stale ? "要重合成" : isSynthesized(s, r) ? "已合成" : "未合成";
                          const tone = round === "running" || round === "queued" ? "muted" : stale ? "danger" : isSynthesized(s, r) ? "done" : "muted";
                          return (
                            <Tag key={r} tone={tone}>
                              {r.toUpperCase()} {label}
                            </Tag>
                          );
                        })}
                      </span>
                    </button>
                  );
                });
                return joined ? (
                  <div
                    className="flex flex-col overflow-hidden rounded-xl"
                    style={{
                      boxShadow: "0 0 0 1px color-mix(in srgb, var(--studio-primary) 45%, transparent)",
                      borderLeft: "3px solid color-mix(in srgb, var(--studio-primary) 60%, transparent)",
                    }}
                    title={`第 ${g.sceneNo} 场整段出：这 ${g.shots.length} 镜一起出成一条长镜头`}
                  >
                    {cards}
                  </div>
                ) : (
                  cards
                );
                })()}
              </div>
            ))}
          </div>

          <div className="min-h-0 overflow-y-auto overscroll-contain pr-1">
          {selected && (
            <ShotDetail
              // 合成完重挂：详情面板有本地草稿，不重挂的话草稿里还是旧提示词，一失焦就把新合成的覆盖掉
              key={`${selected.id}:${selected.prompts?.a?.synthesized_at ?? 0}:${selected.prompts?.b?.synthesized_at ?? 0}:${detailRev}`}
              shot={selected}
              // 镜号紧挨着的上一镜 —— 衔接那一组据此禁用和推荐
              prev={[...shots].filter((s) => s.shot_num < selected.shot_num).sort((a, b) => a.shot_num - b.shot_num).pop() ?? null}
              assets={assets}
              defaultLooks={project.default_looks}
              issues={issuesOf(issues, selected)}
              routeInfo={routes.find((r) => r.shot_id === selected.id) ?? null}
              fixes={fixes.filter((f) => f.shot_num === selected.shot_num)}
              onRepairAll={() => void runRepair()}
              route={route}
              onSave={saveShot}
              synthesizing={synthesizing}
              round={promptRound(selected, promptJob, route)}
              videoRunning={videoRunning}
              rendering={runningKind(jobs, selected.id) === "video"}
              queued={queuePosition(batchJob, selected.id)}
              frameRunning={runningKind(jobs, selected.id) === "frame"}
              onOpenDirector={() => setDirectorShotId(selected.id)}
              onRender={() => setRenderDialog({ kind: "single", shotId: selected.id })}
              regenerating={generating}
              onRegenerateScene={() => void regenerateScene(selected.scene_no ?? 0)}
              onSynthesize={() => queueSynthesize(selected.id)}
              synthQueued={queuePosition(promptJob, selected.id)}
              refining={refining}
              refineRound={refineRound(selected, timelineJob)}
              onRefine={() => void refine([selected.id])}
              projectId={projectId}
              episodeId={episodeId}
              onPicked={() => void reload()}
              longTake={(episode.long_take_scenes ?? []).includes(selected.scene_no ?? 0)}
              longTakeClip={(() => {
                const no = selected.scene_no ?? 0;
                const r = no > 0 ? longTakeClip(episode.long_take_scenes ?? [], episode.scene_renders ?? [], no, route, episode.storyboard) : null;
                if (!r) return null;
                const nums = shots.filter((x) => (x.scene_no ?? 0) === no).map((x) => x.shot_num);
                return { sceneNo: no, first: Math.min(...nums), last: Math.max(...nums), path: r.path, route: r.route, created_at: r.created_at, quality: r.quality };
              })()}
            />
          )}
          </div>
        </div>
      )}
    </div>
  );
}

/** 角标颜色。`keyframe` / `reference` 是出片链路（首帧蓝、参考紫）—— 灰色看着像「没生效」（用户 2026-10-08） */
export type TagTone = "danger" | "done" | "muted" | "keyframe" | "reference";
const TAG_COLOR: Record<TagTone, string> = {
  danger: "var(--studio-failed-fg)",
  done: "var(--studio-done-fg)",
  muted: "var(--studio-text-muted)",
  keyframe: "var(--studio-route-keyframe-fg)",
  reference: "var(--studio-route-reference-fg)",
};

export function Tag({ tone, children, title }: { tone: TagTone; children: React.ReactNode; title?: string }) {
  const color = TAG_COLOR[tone];
  return (
    <span title={title} className="rounded px-1.5 py-0.5" style={{ color, background: `color-mix(in srgb, ${color} 14%, transparent)` }}>
      {children}
    </span>
  );
}

