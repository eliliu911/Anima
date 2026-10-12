/**
 * Step 6 · 组装。
 *
 * **没有出片的镜头时不拦人** —— 进得来，里面写清楚缺什么。
 * 这是和旧面板最根本的区别：那边会把你弹回第 2 步。
 */
import { useEffect, useMemo, useState } from "react";
import {
  cinemaEpisodeCover,
  cinemaHasCoverIntro,
  cinemaSetCoverIntro,
  cinemaChooseExportFolder,
  cinemaFcpNames,
  cinemaExportFcpStart,
  cinemaExportSrt,
  cinemaGenerateEpisodeCover,
  cinemaRevealFile,
  cinemaSaveEpisode,
  cinemaStitchEpisodeStart,
  type SrtExport,
} from "../../../api/cinema";
import { alignFcpNames, loadFcpPrefs, saveFcpPrefs, type FcpPrefs } from "./fcpExport";
import { canToggleIntro, coverButtonLabel, coverHint, introActionLabel, introStateLabel, syncIncludeCover } from "./coverIntro";
import { mediaSrc } from "../MediaThumb";
import { vttUrl } from "./subtitleTrack";
import { useJobs } from "../jobs/QueuePanel";
import { episodeJobState } from "../jobs/shotBusy";
import { DubBar } from "./DubBar";
import { moveShot } from "../storyboard/blankShot";
import { assemblyTiles } from "../storyboard/storyboardView";
import { MediaThumb } from "../MediaThumb";
import type { StepProps } from "./types";

export function AssemblyStep({ project, episode, reload, onError }: StepProps) {
  const [starting, setStarting] = useState(false);
  /**
   * 这次改过顺序、但还没重新合成。
   *
   * **改了顺序之后下面那个成片就是旧的了** —— 不说的话你会看着旧片
   * 以为已经更新，而它和上面的镜头顺序对不上。
   *
   * 只是会话内的标记：`episode` 里没有记「成片是按哪个顺序拼的」，
   * 所以切走再回来这个提示会消失。要做准确的话得在拼接时把顺序签名
   * 存进 episode，那是 Rust 侧的改动 —— 眼下这条覆盖真正要紧的场景
   * （刚改完顺序，成片就在下面）。
   */
  const [reordered, setReordered] = useState(false);
  /**
   * 整集软字幕。**播放器挂的就是它**，导出按钮给的也是它 ——
   * 一份来源，不会出现「播放器里的字幕和导出的对不上」。
   */
  const [srt, setSrt] = useState<SrtExport | null>(null);
  /**
   * 导出到 Final Cut Pro。**不用先合成** —— 用户不用 Anima 的合成（没有转场、调色、配音），
   * 要的是一镜一段排好的时间线进 FCP 剪（2026-09-29）。剪映那条是废的，拿掉了。
   */
  const [fcpOpen, setFcpOpen] = useState(false);
  const [fcp, setFcp] = useState<FcpPrefs>({ dest: "", event: "", project: "" });
  const [fcpStarting, setFcpStarting] = useState(false);
  /**
   * 整片超分：逐镜 RealESRGAN ×2，整集出 1080P。
   *
   * 2026-09-14 同题对照（第 10 镜）：RealESRGAN 125 秒，和 LTX 生成式精修（51 分钟）肉眼几乎一样，
   * 而且镜头运动时没有重影。每镜的结果按出片文件缓存，重拼不重跑。
   */
  const [upscale, setUpscale] = useState(false);
  /**
   * 分集封面。**问的是磁盘上有没有 `cover.png`** —— `EpisodeData` 里没有
   * 封面字段，存进那份 JSON 会被下一次保存抹掉（见 `episode_cover.rs`）。
   */
  const [cover, setCover] = useState<string | null>(null);
  const [coverBusy, setCoverBusy] = useState(false);
  /** 出完封面要让 `<img>` 重新取一次 —— 路径没变，不加这个看到的还是旧图 */
  const [coverV, setCoverV] = useState(0);
  /** **只是下次合成的设置**，不反映现有成片的状态 */
  const [withCover, setWithCover] = useState(false);
  /** 现有成片**带不带**片头。和上面那个是两回事 */
  const [hasIntro, setHasIntro] = useState(false);
  const [introBusy, setIntroBusy] = useState(false);
  /** 片头几秒。和 Rust 的 `COVER_INTRO_SECS` 对应，只用来写提示文案 */
  const COVER_SECS = 2.5;
  /** 合成是后台任务 —— 从任务表读在不在跑，切走再回来按钮状态也对 */
  const { jobs } = useJobs(project?.id ?? "", episode?.id ?? "", false, () => void reload(), "assembly");
  const { generating: stitching, progress } = episodeJobState(jobs, "stitch");
  const busy = starting || stitching;
  const { generating: fcpRunning, progress: fcpProgress } = episodeJobState(jobs, "fcp_export");

  /**
   * 事件名 / 项目名按预设规则对齐 FCP 资源库（用户 2026-10-05：「需要你做命名规则预设」「不要我后面每次都修改」）。
   * 只换没手改过的（见 `alignFcpNames`）；读不到资源库也不报错，照旧用记着的
   */
  const alignFcp = async (prefs: FcpPrefs) => {
    if (!project || !episode || !prefs.dest) return prefs;
    try {
      const names = await cinemaFcpNames(prefs.dest, episode.episode_num, episode.title);
      const next = alignFcpNames(prefs, names, episode.episode_num, episode.title);
      if (next.event !== prefs.event || next.project !== prefs.project) saveFcpPrefs(project.id, episode.id, next);
      return next;
    } catch {
      return prefs;
    }
  };
  // 换了集就换成那一集记着的名字（导出位置按项目记），再按资源库对齐
  useEffect(() => {
    if (!project || !episode) return;
    const loaded = loadFcpPrefs(project.id, episode.id, episode.episode_num, episode.title);
    setFcp(loaded);
    let live = true;
    void alignFcp(loaded).then((next) => live && setFcp(next));
    return () => {
      live = false;
    };
  }, [project?.id, episode?.id]);

  // 进来就问一次：有没有封面、成片是不是带片头的那版。**两个都问磁盘**
  useEffect(() => {
    setCover(null);
    if (!project || !episode) return;
    let alive = true;
    void cinemaEpisodeCover(project.id, episode.id)
      .then((p) => alive && setCover(p))
      .catch(() => alive && setCover(null));
    void cinemaHasCoverIntro(project.id, episode.id)
      .then((on) => {
        if (!alive) return;
        setHasIntro(on);
        // 现有成片带片头的话，下次合成默认也带 —— 但这只是默认值，
        // 改它不会动现在这个成片
        setWithCover(on);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [project, episode]);

  // 成片在时把字幕读出来（顺带刷新磁盘上那份 final.srt）。没有台词时后端报错，安静收下
  const finalUrl = episode?.final_video_url;
  useEffect(() => {
    setSrt(null);
    if (!project || !episode || !finalUrl) return;
    let alive = true;
    cinemaExportSrt(project.id, episode.id)
      .then((s) => alive && setSrt(s))
      .catch(() => alive && setSrt(null));
    return () => {
      alive = false;
    };
  }, [project, episode, finalUrl]);

  // blob URL 要跟着内容换，并且换掉的那个要释放
  const trackUrl = useMemo(() => (srt?.content ? vttUrl(srt.content) : null), [srt?.content]);
  useEffect(() => () => {
    if (trackUrl) URL.revokeObjectURL(trackUrl);
  }, [trackUrl]);

  if (!project || !episode) return null;
  const hasCover = !!cover;
  const includeCover = syncIncludeCover(hasCover, withCover);
  const shots = episode.storyboard;
  /*
   * **格子和「合成一集」排的是同一份**（Rust `cinema::episode_segments`）：整段出的场占一格、
   * 放一整条长镜头。2026-10-01 用户截图：第 5 场整段出过，22~24 镜在这里一直显示「未出片」，
   * 合成时也被当成缺镜跳过。
   */
  const route = project.model_settings?.prompt_route ?? "b";
  const tiles = assemblyTiles(shots, episode.long_take_scenes ?? [], episode.scene_renders ?? [], route);
  const inLongTake = new Set(tiles.flatMap((t) => (t.kind === "long_take" ? t.shots.map((s) => s.id) : [])));
  // 出过片 = 自己有片，或者在一条整段出过的长镜头里
  const rendered = shots.filter((s) => s.video_url || inLongTake.has(s.id));

  /**
   * 挪一镜。**用的是分镜页那套 `moveShot`** —— 它顺带重排编号，
   * 不排的话后面全部错位一个，而拼接是按数组顺序来的、看不出错位。
   */
  const move = async (id: string, delta: -1 | 1) => {
    const next = moveShot(shots, id, delta);
    if (next === shots) return;
    onError(null);
    try {
      await cinemaSaveEpisode(project.id, { ...episode, storyboard: next });
      setReordered(true);
      await reload();
    } catch (e) {
      onError(`保存顺序失败：${e}`);
    }
  };
  // 成片总时长 —— 合成完只给一个预览的话，看不出「是不是全都拼进去了」
  const total = rendered.reduce((a, s) => a + (s.duration || 0), 0);

  // 进度和结果走右下角通知条（任务事件），跑完 useJobs 会刷新分集
  const stitch = async () => {
    setStarting(true);
    onError(null);
    try {
      await cinemaStitchEpisodeStart(project.id, episode.id, upscale, includeCover);
      setReordered(false);
    } catch (e) {
      onError(String(e));
    } finally {
      setStarting(false);
    }
  };
  /**
   * 成片卡上那个**动作**按钮：给已有的成片加 / 去片头。
   *
   * 和顶部那个勾选框是两件事：勾选框管「下次合成」，这个管「现在这个成片」。
   * 2026-09-18 拆开的 —— 原来合成一个勾选框，**勾上就立刻去改成片**，
   * 用户的原话是「已勾选就会执行，这就很奇怪」。
   *
   * 幂等：Rust 侧每次都从母版 `final_nocover.mp4` 重做，按几次都一样
   * （实测按两次仍是 52.67s）。
   */
  const toggleIntro = async () => {
    setIntroBusy(true);
    onError(null);
    try {
      await cinemaSetCoverIntro(project.id, episode.id, !hasIntro);
      setHasIntro(!hasIntro);
      await reload();
    } catch (e) {
      onError(`片头没改成：${e}`);
    } finally {
      setIntroBusy(false);
    }
  };

  const makeCover = async () => {
    setCoverBusy(true);
    onError(null);
    try {
      setCover(await cinemaGenerateEpisodeCover(project.id, episode.id));
      setCoverV((v) => v + 1);
    } catch (e) {
      onError(`封面没出来：${e}`);
    } finally {
      setCoverBusy(false);
    }
  };

  const updateFcp = (patch: Partial<FcpPrefs>) => {
    const next = { ...fcp, ...patch };
    setFcp(next);
    saveFcpPrefs(project.id, episode.id, next);
  };
  const pickFcpDest = async () => {
    try {
      const dir = await cinemaChooseExportFolder(fcp.dest);
      if (dir) {
        const next = { ...fcp, dest: dir };
        setFcp(next);
        saveFcpPrefs(project.id, episode.id, next);
        setFcp(await alignFcp(next));
      }
    } catch (e) {
      onError(String(e));
    }
  };
  // 进度和结果走右下角通知条；导完 Rust 那边直接用 FCP 打开这份时间线
  const exportFcp = async () => {
    setFcpStarting(true);
    onError(null);
    try {
      await cinemaExportFcpStart(project.id, episode.id, { dest_dir: fcp.dest, event: fcp.event.trim(), project: fcp.project.trim(), upscale });
      setFcpOpen(false);
    } catch (e) {
      onError(String(e));
    } finally {
      setFcpStarting(false);
    }
  };
  const fcpBusy = fcpStarting || fcpRunning;
  const fcpLabel = fcpRunning
    ? fcpProgress && fcpProgress.total > 0 && fcpProgress.done < fcpProgress.total
      ? `导出中 ${fcpProgress.done + 1}/${fcpProgress.total}…`
      : "导出中…"
    : "导出到 Final Cut Pro…";
  const unrendered = shots.length - rendered.length;

  const busyLabel =
    progress && progress.total > 0
      ? progress.done >= progress.total ? "拼接中…" : `合成中 ${progress.done + 1}/${progress.total}…`
      : "合成中…";

  /**
   * 版面是**横着铺开的三栏**（封面 / 成片 / 镜头顺序 + 导出），不是一路往下排。
   *
   * 之前是纵向堆叠：一块 1350px 宽的地方，每样东西只占左边 240px，
   * 成片得滚到最底下才看得见。三栏在窄窗口下靠 `flex-wrap` 自己折回去。
   */
  return (
    <div className="flex flex-col gap-5 px-7 py-5">
      {/* **配音单独一块，不进操作条。** 它替换掉 H3 的原声、牺牲口型，
          是有代价的动作，必须让人看见代价再按 —— 和上面片头那个坑同一个教训 */}
      {project && episode && (
        <DubBar
          projectId={project.id}
          episode={episode}
          hasFinal={!!episode.final_video_url}
          onError={onError}
        />
      )}
      {/* 操作条：合成这件事的全部开关都在这一行 */}
      <div className="flex flex-wrap items-center gap-2.5">
        <button
          type="button"
          disabled={busy || rendered.length === 0}
          onClick={() => void stitch()}
          className="rounded-lg px-3.5 py-2 text-[12px] font-medium"
          style={{
            background: rendered.length ? "var(--studio-primary)" : "var(--studio-bg-inset)",
            color: rendered.length ? "var(--studio-on-primary)" : "var(--studio-text-muted)",
          }}
        >
          {busy ? busyLabel : "合成一集"}
        </button>
        <label
          className="flex cursor-pointer items-center gap-1.5 text-[12px]"
          title="逐镜 RealESRGAN ×2 放大，整集出 1080P。一镜约一两分钟；重拼时已放大的镜头不重跑。"
        >
          <input type="checkbox" checked={upscale} disabled={busy} onChange={(e) => setUpscale(e.target.checked)} />
          超分到 1080P
        </label>
        {/* **这是设置，不是动作** —— 只影响下次合成，不碰现在这个成片。
            给已有成片加/去片头是成片卡上那个按钮。
            2026-09-18 拆开：原来合成一个勾选框，勾上就立刻去改成片，很反直觉。 */}
        <label
          className="flex items-center gap-1.5 text-[12px]"
          style={{ cursor: hasCover ? "pointer" : "not-allowed", opacity: hasCover ? 1 : 0.5 }}
          title={coverHint(hasCover, COVER_SECS)}
        >
          <input
            type="checkbox"
            checked={includeCover}
            disabled={busy || !hasCover}
            onChange={(e) => setWithCover(e.target.checked)}
          />
          合成时加片头封面
        </label>
        <span className="font-mono text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
          {rendered.length}/{shots.length} 镜 · {total.toFixed(1)}s
        </span>
        {/* **改过顺序就说出来。** 不说的话你会看着旧成片以为是新的，
            而它和镜头顺序对不上 —— 而且一点都不报错。 */}
        {reordered && (
          <span className="text-[11px]" style={{ color: "var(--studio-running-fg)" }}>
            顺序改过了，这个成片还是旧的 —— 重新「合成一集」
          </span>
        )}
        <button
          type="button"
          disabled={rendered.length === 0 || fcpBusy}
          onClick={() => setFcpOpen((o) => !o)}
          className="ml-auto rounded-lg px-3.5 py-2 text-[12px] disabled:opacity-40"
          style={{ background: "var(--studio-bg-inset)", color: "var(--studio-text)" }}
          title="一镜一段排好时间线、带字幕，素材拷到你选的位置，导完用 FCP 直接打开。不用先合成"
        >
          {fcpLabel}
        </button>
      </div>

      {fcpOpen && (
        <div
          className="flex max-w-[680px] flex-col gap-2.5 rounded-xl px-4 py-3 text-[12px]"
          style={{ background: "var(--studio-bg-inset)" }}
        >
          <div className="flex items-center gap-2">
            <span className="w-[64px] shrink-0" style={{ color: "var(--studio-text-muted)" }}>导出位置</span>
            <span className="min-w-0 flex-1 truncate font-mono text-[11px]" title={fcp.dest}>
              {fcp.dest || "还没选"}
            </span>
            <button
              type="button"
              onClick={() => void pickFcpDest()}
              className="shrink-0 rounded-lg px-2.5 py-1 text-[11px]"
              style={{ background: "var(--studio-bg-surface)" }}
            >
              选择…
            </button>
          </div>
          <label className="flex items-center gap-2">
            <span className="w-[64px] shrink-0" style={{ color: "var(--studio-text-muted)" }}>事件名</span>
            <input
              value={fcp.event}
              onChange={(e) => updateFcp({ event: e.target.value })}
              className="min-w-0 flex-1 rounded-lg px-2.5 py-1.5 text-[12px] outline-none"
              style={{ background: "var(--studio-bg-surface)", color: "var(--studio-text)" }}
            />
          </label>
          <label className="flex items-center gap-2">
            <span className="w-[64px] shrink-0" style={{ color: "var(--studio-text-muted)" }}>项目名</span>
            <input
              value={fcp.project}
              onChange={(e) => updateFcp({ project: e.target.value })}
              className="min-w-0 flex-1 rounded-lg px-2.5 py-1.5 text-[12px] outline-none"
              style={{ background: "var(--studio-bg-surface)", color: "var(--studio-text)" }}
            />
          </label>
          <p className="text-[11px] leading-relaxed" style={{ color: "var(--studio-text-muted)" }}>
            在「导出位置 / {fcp.project.trim() || "项目名"}」下面放 .fcpxml、.srt 和「媒体」文件夹，导完自动用 FCP 打开；
            导入时 FCP 会问放进哪个资源库。事件名和资源库里已有的一样时应该会并进那个事件 —— 第一次导入请看一眼。
            超分跟上面那个勾（现在{upscale ? "开着，一镜一两分钟" : "关着"}）。
            {unrendered > 0 && `还有 ${unrendered} 镜没出片，时间线上按分镜秒数留空、带待办标记。`}
          </p>
          <div>
            <button
              type="button"
              disabled={fcpBusy || !fcp.dest || !fcp.event.trim() || !fcp.project.trim()}
              onClick={() => void exportFcp()}
              className="rounded-lg px-3.5 py-1.5 text-[12px] font-medium disabled:opacity-40"
              style={{ background: "var(--studio-primary)", color: "var(--studio-on-primary)" }}
            >
              {fcpStarting ? "开始导出…" : "导出"}
            </button>
          </div>
        </div>
      )}

      {rendered.length === 0 && (
        <div className="rounded-2xl px-8 py-12 text-center" style={{ background: "var(--studio-bg-inset)" }}>
          <p className="text-[15px]">还没有可以合成的镜头</p>
          <p className="mt-2 text-[12px]" style={{ color: "var(--studio-text-muted)" }}>
            {shots.length === 0 ? "先去第 4 步由剧本生成分镜" : "去第 4 步逐镜「出视频」，至少要有一镜出了片"}
          </p>
        </div>
      )}

      <div className="flex flex-wrap items-start gap-6">
        {/* ── 封面 ────────────────────────────────── */}
        <section className="shrink-0">
          <p className="studio-eyebrow mb-2">分集封面</p>
          {hasCover ? (
            <img
              src={`${mediaSrc(cover!)}${mediaSrc(cover!).includes("?") ? "&" : "?"}v=${coverV}`}
              alt="分集封面"
              className="max-h-[400px] rounded-xl"
              style={{ background: "var(--studio-bg-inset)" }}
            />
          ) : (
            <div
              className="flex h-[240px] w-[150px] items-center justify-center rounded-xl text-[11px]"
              style={{ background: "var(--studio-bg-inset)", color: "var(--studio-text-muted)" }}
            >
              还没有封面
            </div>
          )}
          <div className="mt-2 flex flex-wrap items-center gap-1.5">
            <button
              type="button"
              disabled={coverBusy || busy}
              onClick={() => void makeCover()}
              className="rounded-lg px-2.5 py-1 text-[11px] disabled:opacity-40"
              style={{ background: "var(--studio-bg-inset)" }}
            >
              {coverButtonLabel(hasCover, coverBusy)}
            </button>
            {hasCover && (
              <button
                type="button"
                onClick={() => void cinemaRevealFile(cover!).catch((e) => onError(String(e)))}
                className="rounded-lg px-2.5 py-1 text-[11px]"
                style={{ background: "var(--studio-bg-inset)" }}
              >
                访达
              </button>
            )}
          </div>
          <p className="mt-1.5 max-w-[220px] text-[11px] leading-snug" style={{ color: "var(--studio-text-muted)" }}>
            {/* **模型画的字偶尔会掉笔画** —— 六个字的书法大字实测错过一个。
                说清楚「不对就重来一次」比假装它永远对要好。 */}
            {hasCover ? "字是模型画的，个别笔画不对就再生成一次" : "取本集梗概和项目画风，三行字直接画进画面"}
          </p>
        </section>

        {/* ── 成片 ────────────────────────────────── */}
        {episode.final_video_url && (
          <section className="shrink-0">
            <p className="studio-eyebrow mb-2">
              成片
              <span className="ml-2 font-normal" style={{ color: "var(--studio-text-muted)" }}>
                {introStateLabel(hasIntro)}
              </span>
            </p>
            {/* **软字幕**：画面里不烧字（出片设置里可以开硬字幕），字幕单独一份 SRT，
                这里挂给播放器，也能拿去给达芬奇之类的用。 */}
            <video
              key={trackUrl ?? "no-subs"}
              src={mediaSrc(episode.final_video_url, episode.storyboard.length)}
              className="max-h-[400px] rounded-xl"
              controls
              playsInline
              preload="metadata"
            >
              {trackUrl && <track kind="subtitles" srcLang="zh" label="中文字幕" src={trackUrl} default />}
            </video>
            <div className="mt-2 flex flex-wrap items-center gap-1.5">
              {/* **「组装 / 导出」这一步得导得出去。** 想发给别人、拖进剪辑
                  软件、存到别处，都得先在文件管理器里找到它。 */}
              <button
                type="button"
                onClick={() => void cinemaRevealFile(episode.final_video_url!).catch((e) => onError(String(e)))}
                className="rounded-lg px-2.5 py-1 text-[11px]"
                style={{ background: "var(--studio-bg-inset)", color: "var(--studio-text)" }}
                title={episode.final_video_url}
              >
                访达
              </button>
              <button
                type="button"
                disabled={!srt}
                onClick={() => void cinemaRevealFile(srt!.path).catch((e) => onError(String(e)))}
                className="rounded-lg px-2.5 py-1 text-[11px] disabled:opacity-40"
                style={{ background: "var(--studio-bg-inset)", color: "var(--studio-text)" }}
                title={srt ? srt.path : "这一集的镜头里没有台词"}
              >
                字幕 SRT{srt ? `（${srt.content.split("-->").length - 1} 条）` : "：没有台词"}
              </button>
              {/* **动作按钮** —— 文案说的是「点下去会发生什么」。
                  幂等：Rust 侧每次从母版重做，按几次都一样 */}
              <button
                type="button"
                disabled={!canToggleIntro(hasCover, hasIntro, introBusy)}
                onClick={() => void toggleIntro()}
                className="rounded-lg px-2.5 py-1 text-[11px] disabled:opacity-40"
                style={{ background: "var(--studio-bg-inset)", color: "var(--studio-text)" }}
                title={
                  hasIntro
                    ? "把片头那几秒去掉，字幕时间跟着回退"
                    : hasCover
                      ? `把封面接在最前面，约 ${COVER_SECS} 秒，不重渲染任何镜头`
                      : "先生成封面"
                }
              >
                {introActionLabel(hasIntro, introBusy)}
              </button>
            </div>
            {srt && (
              <p className="mt-1.5 max-w-[220px] text-[11px] leading-snug" style={{ color: "var(--studio-text-muted)" }}>
                播放器里的字幕可以在播放器菜单里关
              </p>
            )}
          </section>
        )}

        {/* ── 镜头顺序 ──────────────────────────────
            占满剩下的地方 —— 它是这一页唯一会长的东西 */}
        {rendered.length > 0 && (
          <section className="min-w-[280px] flex-1">
            <p className="studio-eyebrow mb-2">镜头顺序 —— 拿左右箭头调</p>
            <div className="flex flex-wrap gap-2">
              {tiles.map((t) => {
                if (t.kind === "long_take") {
                  /*
                    **长镜头占它那几镜的位置**：宽度 = 镜数 × 132 + 间隔，换行时整格一起换。
                    不放左右箭头 —— 整条不拆开，调这一场里哪镜先哪镜后改不了成片。
                  */
                  const n = t.shots.length;
                  const first = t.shots[0].shot_num;
                  const last = t.shots[n - 1].shot_num;
                  const secs = t.shots.reduce((a, s) => a + (s.duration || 0), 0);
                  const own = t.shots.filter((s) => s.video_url).map((s) => s.shot_num);
                  return (
                    <div
                      key={`scene-${t.sceneNo}`}
                      className="relative overflow-hidden rounded-lg"
                      style={{
                        width: n * 132 + (n - 1) * 8,
                        background: "var(--studio-bg-inset)",
                        boxShadow: "0 0 0 1px color-mix(in srgb, var(--studio-primary) 45%, transparent)",
                      }}
                      title={
                        `第 ${t.sceneNo} 场整段出的长镜头（第 ${first}–${last} 镜），合成一集和导出都放这一整条。` +
                        (own.length ? `\n第 ${own.join("、")} 镜单独出过，但这一场按整段放；要用单镜的，去分镜页把这一场的出法改回「分镜」。` : "")
                      }
                    >
                      {/* 高度和单镜格子一样（132 宽的 16:9），宽度撑满 —— 不能用 aspect-video：
                          那会按固定高度反推出 131px 宽，画面只占左边三分之一（对照页里看出来的） */}
                      <div className="flex w-full items-center justify-center" style={{ height: (132 * 9) / 16 }}>
                        <MediaThumb path={t.render.path} kind="video" className="h-full w-full object-cover" />
                      </div>
                      <div className="flex items-center justify-between gap-2 px-1.5 py-1">
                        <span className="font-mono text-[0.5rem]">
                          {first}–{last} · {secs.toFixed(1)}s
                        </span>
                        <span className="truncate text-[0.55rem]" style={{ color: "var(--studio-primary)" }}>
                          第 {t.sceneNo} 场 · 整段长镜头
                        </span>
                      </div>
                    </div>
                  );
                }
                const s = t.shot;
                const i = shots.findIndex((x) => x.id === s.id);
                return (
                  <div
                    key={s.id}
                    className="group relative w-[132px] overflow-hidden rounded-lg"
                    style={{ background: "var(--studio-bg-inset)", opacity: s.video_url ? 1 : 0.4 }}
                    title={s.video_url ? undefined : "这一镜还没出片，合成时会被跳过"}
                  >
                    <div className="flex aspect-video items-center justify-center">
                      {s.video_url || s.image_url ? (
                        <MediaThumb
                          path={s.video_url || s.image_url}
                          kind={s.video_url ? "video" : "image"}
                          className="h-full w-full object-cover"
                        />
                      ) : (
                        <span className="text-[10px]" style={{ color: "var(--studio-text-muted)" }}>
                          未出片
                        </span>
                      )}
                    </div>
                    <div className="flex items-center justify-between px-1.5 py-1">
                      <span className="font-mono text-[0.5rem]">
                        {s.shot_num} · {s.duration}s
                      </span>
                      {/* **左右箭头，不是拖拽。** 拖拽在换行的多行网格里
                          落点很难对准，而这里最常做的事就是「这一镜挪前一格」。 */}
                      <span className="flex gap-0.5 opacity-0 transition-opacity group-hover:opacity-100">
                        <button
                          type="button"
                          disabled={busy || i === 0}
                          onClick={() => void move(s.id, -1)}
                          title="往前挪一格"
                          className="rounded px-1 text-[10px] disabled:opacity-25"
                          style={{ background: "var(--studio-bg-surface)" }}
                        >
                          ←
                        </button>
                        <button
                          type="button"
                          disabled={busy || i === shots.length - 1}
                          onClick={() => void move(s.id, 1)}
                          title="往后挪一格"
                          className="rounded px-1 text-[10px] disabled:opacity-25"
                          style={{ background: "var(--studio-bg-surface)" }}
                        >
                          →
                        </button>
                      </span>
                    </div>
                  </div>
                );
              })}
            </div>
          </section>
        )}
      </div>
    </div>
  );
}
