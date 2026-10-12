/**
 * 一个项目内部：左侧导航 + 当前页。
 *
 * - **系列项目**：左栏分「系列」「本集」两组（`SeriesRail`），点进来先到系列概览。
 *   风格、角色与资产是整部剧一份的，在系列组；剧本 / 本集选角 / 分镜 / 组装在本集组。
 * - **独立项目**（单集）：还是原来的五步（`StepRail`）。
 *
 * 项目/分集数据在这一层加载一次，往下传 —— 每一步各自去拉的话，
 * 生成完一镜之后各步看到的会是不同版本的同一份数据。
 */
import { useStudioChanged } from "../useStudioChanged";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  cinemaCreateEpisode,
  cinemaDeleteEpisode,
  cinemaGetProjectDetail,
  type CinemaProject,
  type EpisodeData,
} from "../../../api/cinema";
import { STEPS, StepRail, stepsFor, type StepId } from "./StepRail";
import {
  SeriesRail,
  episodeSteps,
  initialSel,
  type RailSel,
  type SeriesPage,
} from "./SeriesRail";
import { SeriesOverview } from "../series/SeriesOverview";
import { seriesAssetsInbox } from "../../../api/seriesAssets";
import { StepActionSlot } from "./StepAction";
import { computeStepStates } from "./stepStates";
import { styleGet } from "../../../api/style";
import { assetsList } from "../../../api/assets";
import { PromptConfigDialog } from "../PromptConfigDialog";
import { ModelSettingsDialog } from "./ModelSettingsDialog";
import { ScriptStep } from "../steps/ScriptStep";
import { StyleStep } from "../steps/StyleStep";
import { AssetsStep } from "../steps/AssetsStep";
import { StoryboardStep } from "../steps/StoryboardStep";
import { AssemblyStep } from "../steps/AssemblyStep";

export function ProjectPage({
  projectId,
  initialEpisodeId,
  initialStep,
  onBack,
  onOpenScripts,
}: {
  projectId: string;
  /** 从剧本一览跳进来时指定落在哪一集 */
  initialEpisodeId?: string;
  initialStep?: StepId;
  onBack: () => void;
  /** 系列组的「故事原文」：跳到剧本编辑器里绑定的项目（没绑就按名字认，认出来自动绑） */
  onOpenScripts?: (story: { cinemaProjectId: string; title: string; scriptProjectId?: string | null }) => void;
}) {
  const [project, setProject] = useState<CinemaProject | null>(null);
  const [episodes, setEpisodes] = useState<EpisodeData[]>([]);
  const [episodeId, setEpisodeId] = useState<string | null>(null);
  const [step, setStep] = useState<StepId>(initialStep ?? "script");
  /** 系列项目左栏选中的是哪一项。独立项目不用它（用上面的 `step`）。 */
  const [sel, setSel] = useState<RailSel>(() => initialSel(initialStep));
  /** 各集里还没进系列池的资产数 —— 左栏「角色与资产」下面那个提醒 */
  const [pendingAssets, setPendingAssets] = useState(0);
  const [error, setError] = useState<string | null>(null);
  // 步骤页主操作的挂载点（见 StepAction）。**用 state 不用 ref** ——
  // ref 变化不会触发重渲染，Provider 会一直拿到初始的 null，
  // 表现是按钮永远不出现，而且不报错。
  const [actionEl, setActionEl] = useState<HTMLDivElement | null>(null);
  const [promptsOpen, setPromptsOpen] = useState(false);
  const [deleting, setDeleting] = useState<EpisodeData | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [hasStyle, setHasStyle] = useState(false);
  /** 资产计数。**项目级 + 分集级两处** —— 提取的产物默认落在分集级。 */
  const [assetTally, setAssetTally] = useState({ total: 0, withImage: 0 });

  const reload = useCallback(async () => {
    try {
      const d = await cinemaGetProjectDetail(projectId);
      setProject(d.project);
      setEpisodes(d.episodes);
      setEpisodeId((cur) => {
        // 外面指定了就用它 —— 从剧本一览点进来是要看**那一集**
        if (
          initialEpisodeId &&
          d.episodes.some((e) => e.id === initialEpisodeId)
        ) {
          return initialEpisodeId;
        }
        return cur && d.episodes.some((e) => e.id === cur)
          ? cur
          : (d.episodes[0]?.id ?? null);
      });
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, [projectId, initialEpisodeId]);

  useEffect(() => {
    void reload();
  }, [reload]);
  // Chat 改了这个项目（导了剧本、建了资产、设了风格）：重读项目，侧栏标记跟着重算
  const [externalRev, setExternalRev] = useState(0);
  useStudioChanged(() => {
    void reload();
    setExternalRev((n) => n + 1);
  }, { projectId });

  // 风格和资产数只影响导航上的标记，拉不到就当没有 —— 不要因为
  // 一个标记查不到就让整个项目页报错。
  useEffect(() => {
    let alive = true;
    void styleGet(projectId)
      .then(
        (a) =>
          alive &&
          setHasStyle(!!a.preset_id.trim() || !!a.positive_prompt.trim()),
      )
      .catch(() => {});
    // **两级都要数。** 提取出来的实体默认落在分集级（「本集待归属」），
    // 只数项目级的话刚提取完六个角色，侧栏还是个空圈。
    void Promise.all([
      assetsList({ kind: "project", project_id: projectId }),
      episodeId
        ? assetsList({ kind: "episode", project_id: projectId, episode_id: episodeId })
        : Promise.resolve([]),
    ])
      .then(([proj, ep]) => {
        if (!alive) return;
        const all = [...proj, ...ep];
        setAssetTally({
          total: all.length,
          withImage: all.filter((a) => !!a.cover_url).length,
        });
      })
      .catch(() => {});
    void seriesAssetsInbox(projectId)
      .then((ib) => alive && setPendingAssets(ib.pending.length))
      .catch(() => {});
    return () => {
      alive = false;
    };
    // **episodeId 必须在依赖里。** 换一集之后分集级资产整个换了一批，
    // 不重算的话侧栏显示的是上一集的数字。
  }, [projectId, episodeId, step, sel, externalRev]);

  const episode = episodes.find((e) => e.id === episodeId) ?? null;

  /**
   * 每一步的状态。**只是标记，不参与能不能点。**
   *
   * 判断都保守：剧本有字才算做过，风格选了才算，资产要真有条目。
   * 报一个我们验证不了的「完成」比不报更糟。
   */
  /**
   * 这个项目真正有哪几步。「我直接创作」的项目没有剧本那一步。
   * 见 `stepsFor` —— 序号是重算的，不是把第 1 步挖掉留个洞。
   */
  const steps = useMemo(
    () => stepsFor(project?.content_mode),
    [project?.content_mode],
  );

  // 每一步的状态。**只是标记，不参与能不能点。** 见 stepStates.ts
  const states = useMemo(
    () =>
      computeStepStates(episode, {
        hasStyle,
        assetCount: assetTally.total,
        assetsWithImage: assetTally.withImage,
      }),
    [episode, hasStyle, assetTally],
  );

  const addEpisode = async () => {
    if (!project) return;
    try {
      const ep = await cinemaCreateEpisode({ project_id: project.id });
      await reload();
      setEpisodeId(ep.id);
    } catch (e) {
      setError(String(e));
    }
  };

  if (error && !project) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-2">
        <p className="text-[14px]">打不开这个项目</p>
        <p className="text-[12px]" style={{ color: "var(--studio-text-2)" }}>
          {error}
        </p>
        <button
          type="button"
          onClick={onBack}
          className="mt-2 text-[12px] underline"
        >
          回工作区
        </button>
      </div>
    );
  }

  const common = {
    project,
    episode,
    reload,
    onError: setError,
    onOpenSettings: () => setSettingsOpen(true),
  };

  const isSeries = !!project && project.kind !== "standalone";
  const epSteps = episodeSteps(project?.content_mode);
  const epTag = episode ? `E${String(episode.episode_num).padStart(2, "0")}` : "";

  /** 进某一集。从概览点卡片、左栏换集都走这里：已经在本集组就留在同一步 */
  const openEpisode = (id: string) => {
    setEpisodeId(id);
    setSel((cur) => (cur.level === "episode" ? cur : { level: "episode", step: epSteps[0]?.id ?? "storyboard" }));
  };

  /** 页头。独立项目和本集组照旧是 STEP 0n；系列组是 SERIES。 */
  const header = (() => {
    if (!isSeries) {
      const st = STEPS.find((x) => x.id === step)!;
      return { eyebrow: `STEP 0${st.num}`, en: st.en, title: st.label, sub: STEP_SUB[step] };
    }
    if (sel.level === "series") return SERIES_HEADER[sel.page];
    const st = epSteps.find((x) => x.id === sel.step) ?? epSteps[0];
    return {
      eyebrow: `${epTag} · STEP 0${st.num}`,
      en: st.en,
      title: st.label,
      sub: st.id === "assets" ? "这一集出场的系列角色、场景、道具，和只在本集的" : STEP_SUB[st.id],
    };
  })();

  const seriesNotes: Partial<Record<SeriesPage, { note?: string; done?: boolean; alert?: boolean }>> = {
    assets: pendingAssets > 0 ? { note: `${pendingAssets} 个待归属`, alert: true } : {},
    style: hasStyle ? { done: true } : { note: "未定调" },
  };

  return (
    <>
      {isSeries ? (
        <SeriesRail
          projectTitle={project?.title ?? "…"}
          sel={sel}
          onSelect={setSel}
          onOpenSource={() =>
            project &&
            onOpenScripts?.({ cinemaProjectId: project.id, title: project.title, scriptProjectId: project.script_project_id })
          }
          episodes={episodes}
          episodeId={episodeId}
          onEpisode={openEpisode}
          steps={epSteps}
          states={states}
          seriesNotes={seriesNotes}
          onBack={onBack}
        />
      ) : (
        <StepRail
          active={step}
          steps={steps}
          states={states}
          projectTitle={project?.title ?? "…"}
          episodeLabel={null}
          onSelect={setStep}
          onBack={onBack}
        />
      )}

      <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
        <StepHeader
          {...header}
          project={project}
          onOpenPrompts={() => setPromptsOpen(true)}
          onOpenSettings={() => setSettingsOpen(true)}
          actionRef={setActionEl}
        />
        {error && (
          <p
            className="shrink-0 px-7 py-2 text-[12px]"
            style={{
              background: "var(--studio-bg-inset)",
              color: "var(--studio-text-2)",
            }}
          >
            {error}
          </p>
        )}
        <StepActionSlot.Provider value={actionEl}>
          <div className="min-h-0 flex-1 overflow-y-auto">
            {!isSeries && project && (
              <>
                {step === "script" && <ScriptStep {...common} />}
                {step === "style" && <StyleStep {...common} />}
                {step === "assets" && <AssetsStep {...common} />}
                {step === "storyboard" && <StoryboardStep {...common} />}
                {step === "assembly" && <AssemblyStep {...common} />}
              </>
            )}
            {isSeries && project && sel.level === "series" && (
              <>
                {sel.page === "overview" && (
                  <SeriesOverview
                    project={project}
                    episodes={episodes}
                    onOpenEpisode={openEpisode}
                    onGoAssets={() => setSel({ level: "series", page: "assets" })}
                    onAddEpisode={() => void addEpisode()}
                    onDeleteEpisode={setDeleting}
                    onProjectChanged={() => void reload()}
                  />
                )}
                {sel.page === "assets" && <AssetsStep {...common} mode="series" episodes={episodes} />}
                {sel.page === "style" && <StyleStep {...common} />}
              </>
            )}
            {isSeries && project && sel.level === "episode" && (
              <>
                {sel.step === "script" && <ScriptStep {...common} />}
                {sel.step === "assets" && (
                  <AssetsStep
                    {...common}
                    mode="cast"
                    onGoSeriesAssets={() => setSel({ level: "series", page: "assets" })}
                  />
                )}
                {sel.step === "storyboard" && <StoryboardStep {...common} />}
                {sel.step === "assembly" && <AssemblyStep {...common} />}
              </>
            )}
          </div>
        </StepActionSlot.Provider>
      </div>

      {promptsOpen && (
        <PromptConfigDialog onClose={() => setPromptsOpen(false)} />
      )}
      {deleting && project && (
        <DeleteEpisodeDialog
          projectId={project.id}
          episode={deleting}
          onClose={() => setDeleting(null)}
          onDeleted={async () => {
            setDeleting(null);
            await reload();
          }}
        />
      )}
      {settingsOpen && project && (
        <ModelSettingsDialog
          project={project}
          onClose={() => setSettingsOpen(false)}
          onSaved={setProject}
        />
      )}
    </>
  );
}

/** 删一集之前先说清楚里面有什么（和工作区删项目同一个样子）。 */
function DeleteEpisodeDialog({
  projectId,
  episode,
  onClose,
  onDeleted,
}: {
  projectId: string;
  episode: EpisodeData;
  onClose: () => void;
  onDeleted: () => void | Promise<void>;
}) {
  const [summary, setSummary] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    cinemaDeleteEpisode(projectId, episode.id, false)
      .then(setSummary)
      .catch((e) => setError(String(e)));
  }, [projectId, episode.id]);

  return (
    <div
      className="studio-scrim fixed inset-0 z-50 flex items-center justify-center"
    >
      <div
        className="studio-modal max-h-[86vh] w-[400px] max-w-[92vw] overflow-y-auto"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="px-6 pb-2 pt-5">
          <h2 className="studio-display text-[17px]">删除这一集</h2>
          <p className="mt-2 text-[13px]" style={{ color: "var(--studio-text-2)" }}>
            {summary ?? (error ? `「${episode.title}」` : "正在数里面有什么…")}
          </p>
          <p className="mt-2 text-[12px]" style={{ color: "var(--studio-text-muted)" }}>
            连同这一集的剧本、分镜、关键帧、视频和本集资产一起删掉，不可恢复。其它集的编号不变。
          </p>
          {error && (
            <p className="mt-2 text-[12px]" style={{ color: "var(--studio-danger)" }}>
              {error}
            </p>
          )}
        </div>
        <footer className="flex justify-end gap-2 px-6 py-4">
          <button
            type="button"
            onClick={onClose}
            className="rounded-lg px-4 py-2 text-[13px]"
            style={{ background: "var(--studio-bg-inset)", color: "var(--studio-text-2)" }}
          >
            不删了
          </button>
          <button
            type="button"
            disabled={busy || !summary}
            onClick={async () => {
              setBusy(true);
              setError(null);
              try {
                await cinemaDeleteEpisode(projectId, episode.id, true);
                await onDeleted();
              } catch (e) {
                setError(String(e));
                setBusy(false);
              }
            }}
            className="rounded-lg px-4 py-2 text-[13px] font-medium"
            style={{
              background: "var(--studio-danger)",
              color: "var(--studio-on-danger)",
              opacity: busy || !summary ? 0.5 : 1,
            }}
          >
            {busy ? "删除中…" : "确认删除"}
          </button>
        </footer>
      </div>
    </div>
  );
}

const STEP_SUB: Record<StepId, string> = {
  script: "写这一集的剧本；写完从这里提取角色/场景/道具",
  style: "定整部剧的视觉基调，后面每一次生成都会带上它",
  assets: "本项目的角色、场景与道具",
  storyboard: "由剧本生成镜头，逐镜出首帧或直接出片，也可以整批一起出",
  assembly: "镜头顺序、合成一集、导出",
};

/** 系列组各页的页头 */
const SERIES_HEADER: Record<SeriesPage, { eyebrow: string; en: string; title: string; sub: string }> = {
  overview: { eyebrow: "SERIES", en: "OVERVIEW", title: "概览", sub: "整部剧做到哪了、有什么要处理的、每一集的状态" },
  assets: {
    eyebrow: "SERIES",
    en: "CAST & ASSETS",
    title: "角色与资产",
    sub: "整部剧共用的角色、场景、道具 —— 设定卡在这里出，每一集都用这一份",
  },
  style: { eyebrow: "SERIES", en: "STYLE", title: "风格", sub: "定整部剧的视觉基调，后面每一次生成都会带上它" },
};

function StepHeader({
  eyebrow,
  en,
  title,
  sub,
  project,
  onOpenPrompts,
  onOpenSettings,
  actionRef,
}: {
  /** 「STEP 03」「E01 · STEP 02」「SERIES」 */
  eyebrow: string;
  en: string;
  title: string;
  sub: string;
  project: CinemaProject | null;
  onOpenPrompts: () => void;
  onOpenSettings: () => void;
  /** 步骤页的主操作挂在这里（见 `StepAction`）。 */
  actionRef: (el: HTMLDivElement | null) => void;
}) {
  return (
    <header
      className="studio-banner flex shrink-0 items-start justify-between px-7 py-5"
      style={{ borderBottom: "1px solid var(--studio-border-subtle)" }}
      data-tauri-drag-region
    >
      <div className="flex flex-col gap-1">
        <span className="studio-eyebrow" style={{ letterSpacing: "0.2em" }}>
          {eyebrow}
          <span style={{ color: "var(--studio-primary)", fontWeight: 600 }}>
            {" · "}
            {en}
          </span>
        </span>
        <h1 className="studio-display text-[2rem]">{title}</h1>
        <p
          className="text-[12px]"
          style={{ color: "var(--studio-text-muted)" }}
        >
          {sub}
        </p>
      </div>
      {/* 右侧分两行：上面是配置入口（提示词 / 生成设置），
          下面是这一步的主操作。**不能挤在同一行** —— 主操作和配置
          入口是两类东西，并排放会让「生成分镜」看起来只是第三个设置项，
          而且它比另外两个都重要。分行之后它和标题块的底边对齐，
          正好落在 banner 右下角。 */}
      <div className="flex shrink-0 flex-col items-end justify-between gap-3 self-stretch">
        <div className="flex items-center gap-3">
          {/* AI 环节的提示词都在这里改。放在项目页头上，是因为
            「AI 把我的话改成了什么」这件事只有在做的时候才会想调。

            **叫「提示词设置」不叫「提示词」。** 2026-09-19 用户的原话：
            「按钮叫提示词，但是这个名字一直被我忽略了」——
            光写「提示词」读起来像个标签（这一页到处都是提示词），
            不像一个能点进去改的地方。带上「设置」才和旁边的「生成设置」
            成一组，眼睛扫过去知道这两个是同一类东西。 */}
          <button
            type="button"
            onClick={onOpenPrompts}
            title="提示词设置：抽实体 / 分场 / 生成镜头 / B 路写手 / 各类润色，14 个环节都能改，改完能试跑"
            className="rounded-lg px-2.5 py-1.5 text-[11.5px]"
            style={{
              background: "var(--studio-bg-inset)",
              color: "var(--studio-text-2)",
            }}
          >
            提示词设置
          </button>
          {/* 和提示词并排：一个管「AI 说什么」，一个管「用什么引擎、多大画幅」。
            都是整个项目一份的东西，所以都在项目页头上。 */}
          <button
            type="button"
            onClick={onOpenSettings}
            title="生成设置：出图/出片引擎、各类资产的画幅"
            className="rounded-lg px-2.5 py-1.5 text-[11.5px]"
            style={{
              background: "var(--studio-bg-inset)",
              color: "var(--studio-text-2)",
            }}
          >
            生成设置
          </button>
          {project && (
            <span
              className="font-mono text-[0.55rem]"
              style={{ color: "var(--studio-text-muted)" }}
            >
              {project.aspect_ratio}
            </span>
          )}
        </div>
        {/* 主操作。挂载点始终存在（空的时候不占高度），
            所以上面那行的位置不会因为步骤切换而跳。 */}
        <div ref={actionRef} className="flex items-center gap-2 empty:hidden" />
      </div>
    </header>
  );
}

export default ProjectPage;
