/**
 * 画布上的导演世界节点：一张卡片 +「进入导演世界」。真正的编辑在全屏编辑器里。
 *
 * 照 DramaClaw（ClaymoreLab, Elastic License 2.0）
 * frontend/src/features/canvas/nodes/ThreeDWorldNode.tsx 改写：没有输出时卡片中间一个入口按钮，
 * 有了输出显示上次的合成图、入口挪到右上角；「输出到画布节点」生成「导演世界输出」组
 * （合成图 + 纯背景，两列，从本节点连线过去）。出处规则见 DECISIONS.md 2026-09-16。
 *
 * 用途见 `director/scene.ts` 文件头：摆人偶定位置和动作，导出去给出图用。
 */
import { useEffect, useMemo, useState } from "react";
import { Handle, Position, type NodeProps } from "@xyflow/react";
import { BaseNodeWrapper } from "./BaseNodeWrapper";
import { useCanvasStore } from "../store/useCanvasStore";
import { selectContentNodes } from "../store/selectContentNodes";
import { NodeHeader } from "../ui/NodeHeader";
import { Orbit } from "../ui/icons";
import { saveChatImageAttachment, saveGenerationLog } from "../../api";
import { canvasSaveVideo } from "../../api/canvasDepth";
import { canvasJobGet, canvasJobStart, waitForCanvasJob } from "../../api/canvasJobs";
import { assetsList, assetsUpdate, type AssetView, type Scope } from "../../api/assets";
import { cinemaGetProjectDetail, cinemaSaveEpisode } from "../../api/cinema";
import { MediaThumb } from "../../components/studio/MediaThumb";
import { upstreamSceneAsset } from "../pano/directorLinks";
import { DirectorWorldEditor, type DirectorOutput, type DirectorScene } from "../director/DirectorWorldEditor";
import { DEFAULT_EYE_HEIGHT, migrateMarkers, readActors, readCamera, readNamedCameras } from "../director/scene";
import type { DirectorWorldNode, ShotNodeData } from "../types";

export function DirectorWorldNodeComponent({ id, data, selected }: NodeProps<DirectorWorldNode>) {
  const updateNodeData = useCanvasStore((s) => s.updateNodeData);
  const removeNode = useCanvasStore((s) => s.removeNode);
  const addImageGroup = useCanvasStore((s) => s.addImageGroup);
  const nodes = useCanvasStore(selectContentNodes);
  const edges = useCanvasStore((s) => s.edges);
  const projectId = useCanvasStore((s) => s.activeProjectId);
  const [open, setOpen] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [splatPath, setSplatPath] = useState(data.splat?.path ?? "");
  const [generatingWorld, setGeneratingWorld] = useState(false);
  const [splittingPano, setSplittingPano] = useState(false);

  const acceptWorldJob = (job: Awaited<ReturnType<typeof canvasJobGet>>) => {
    if (!job || job.status === "running") return;
    setGeneratingWorld(false);
    if (job.status !== "done") {
      setErr(job.error || "3D片场生成失败");
      return;
    }
    const result = job.result as { path?: unknown } | undefined;
    const path = typeof result?.path === "string" ? result.path : "";
    if (!path) { setErr("SHARP 已结束，但没有返回 PLY 文件"); return; }
    setSplatPath(path);
    setErr(null);
    // SHARP 出的是原相机坐标，载入时要翻正、抬到眼高（scene.splatPlacement）
    updateNodeData(id, { splat: { path, format: "ply", convention: "opencv_camera" }, worldJobId: undefined });
  };

  useEffect(() => {
    if (!data.worldJobId) return;
    setGeneratingWorld(true);
    void canvasJobGet(data.worldJobId).then((job) => {
      if (job?.status === "running") void waitForCanvasJob(data.worldJobId!).then(acceptWorldJob);
      else acceptWorldJob(job);
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [data.worldJobId]);

  const generateWorld = async () => {
    if (!source || generatingWorld) return;
    setErr(null);
    setGeneratingWorld(true);
    try {
      const jobId = await canvasJobStart({ projectId, nodeId: id, label: "生成3D片场", task: { kind: "world_generate", args: { image_path: source, device: "mps" } } });
      updateNodeData(id, { worldJobId: jobId });
      acceptWorldJob(await waitForCanvasJob(jobId));
    } catch (e) {
      setGeneratingWorld(false);
      setErr(String(e));
    }
  };

  const splitPano = async () => {
    if (!source || splittingPano) return;
    setSplittingPano(true);
    setErr(null);
    try {
      const jobId = await canvasJobStart({ projectId, nodeId: id, label: "拆分360全景六面", task: { kind: "pano_cube", args: { image_path: source, face_size: 1536 } } });
      updateNodeData(id, { panoCubeJobId: jobId });
      const job = await waitForCanvasJob(jobId);
      const result = job.result as { path?: unknown } | undefined;
      if (job.status !== "done" || typeof result?.path !== "string") throw new Error(job.error || "全景六面预处理失败");
      updateNodeData(id, { cubemapManifest: result.path, panoCubeJobId: undefined });
    } catch (e) { setErr(String(e)); }
    finally { setSplittingPano(false); }
  };

  /** 上游图片节点出的全景图 */
  const source = useMemo(() => {
    for (const e of edges.filter((x) => x.target === id)) {
      const n = nodes.find((x) => x.id === e.source);
      const url = (n?.data as { outputImageUrl?: string } | undefined)?.outputImageUrl;
      if (url) return url;
    }
    return null;
  }, [id, nodes, edges]);

  const eyeHeight = data.calibration?.eyeHeight ?? DEFAULT_EYE_HEIGHT;
  const correction = data.correction ?? { roll: 0, pitch: 0, yaw: 0 };
  const frontYawDeg = data.frontYawDeg ?? 0;
  const splat = data.splat?.path ? data.splat : null;

  const applySplatPath = () => {
    const path = splatPath.trim();
    if (!path) {
      updateNodeData(id, { splat: undefined });
      return;
    }
    const ext = path.split(".").pop()?.toLowerCase();
    if (ext !== "ply" && ext !== "ksplat" && ext !== "spz") {
      setErr("3DGS 文件只支持 .ply、.ksplat 或 .spz");
      return;
    }
    setErr(null);
    updateNodeData(id, { splat: { path, format: ext } });
  };

  /**
   * 人偶。**老节点只有 `markers`（方位角 + 距离）**，第一次打开时换算成 xyz 人偶；
   * 编辑器存盘时写 `actors`、清掉 `markers`。
   */
  const actors = useMemo(
    () => (Array.isArray(data.actors) ? readActors(data.actors) : migrateMarkers(data.markers ?? [], data.calibration?.scale ?? 1)),
    [data.actors, data.markers, data.calibration?.scale],
  );

  /** 连着的分镜节点带出分集和镜头（没存过上下文时用） */
  const linkedShot = useMemo(() => {
    for (const e of edges.filter((x) => x.source === id || x.target === id)) {
      const other = nodes.find((n) => n.id === (e.source === id ? e.target : e.source));
      if (other?.type !== "shot") continue;
      const d = other.data as ShotNodeData;
      if (d.projectId && d.episodeId) return { projectId: d.projectId, episodeId: d.episodeId, shotId: d.shotId ?? null };
    }
    return null;
  }, [id, nodes, edges]);

  const initial: DirectorScene = {
    actors,
    camera: readCamera(data.camera, eyeHeight),
    aspect: typeof data.aspect === "string" ? data.aspect : "16:9",
    context: (data.context as DirectorScene["context"]) ?? linkedShot,
    namedCameras: readNamedCameras(data.namedCameras, eyeHeight),
    cameraPaths: data.cameraPaths,
    activeCameraId: typeof data.activeCameraId === "string" ? data.activeCameraId : null,
  };

  // ── 世界校准：从场景资产读回来，改了可以存回去 ──
  const scene = useMemo(() => upstreamSceneAsset(id, nodes, edges), [id, nodes, edges]);
  const [sceneAsset, setSceneAsset] = useState<AssetView | null>(null);
  const scope: Scope = useMemo(
    () => (projectId ? { kind: "project", project_id: projectId } : { kind: "global" }),
    [projectId],
  );
  useEffect(() => {
    if (!scene) return;
    let alive = true;
    assetsList(scope)
      .then((list) => {
        if (!alive) return;
        const a = list.find((x) => x.id === scene.assetId) ?? null;
        setSceneAsset(a);
        const saved = a?.scene?.pano_calibration;
        // 节点里已经有值时不覆盖 —— 那是这次正在调的
        if (saved && !data.calibration) {
          updateNodeData(id, { calibration: { eyeHeight: saved.eye_height, scale: saved.scale } });
        }
      })
      .catch(() => alive && setSceneAsset(null));
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scene?.assetId, scope]);

  const [calNote, setCalNote] = useState("");
  const saveCalibration = async () => {
    if (!sceneAsset) return;
    try {
      await assetsUpdate({
        scope: sceneAsset.scope,
        id: sceneAsset.id,
        scene: {
          ...(sceneAsset.scene ?? {
            shape: "", zones: [], entrances: [], anchors: [], relations: [],
            materials: [], views: [], inferred: [], no_extra: [], empty_stage: "required",
          }),
          pano_calibration: { eye_height: eyeHeight, scale: data.calibration?.scale ?? 1 },
        },
      });
      setCalNote(`已存进《${scene?.sceneName || sceneAsset.name}》`);
    } catch (e) {
      setErr(String(e));
    }
  };

  // ── 输出到画布 ──
  const onOutput = async ({ combined, envOnly, meta }: DirectorOutput) => {
    const stamp = Date.now();
    const [c, e] = await Promise.all([
      saveChatImageAttachment({ dataUrl: combined, filename: `director-world_${id}_combined_${stamp}.png` }),
      saveChatImageAttachment({ dataUrl: envOnly, filename: `director-world_${id}_env-only_${stamp}.png` }),
    ]);
    if (!c?.path || !e?.path) throw new Error("截图没保存下来");
    const metaJson = JSON.stringify(meta);
    const captureMetadata = (render_mode: string) => ({
      viewer: "director_world",
      render_mode,
      source_node_id: id,
      frame_meta: meta,
    });
    updateNodeData(id, {
      lastCombined: c.path,
      outputImages: [...((data.outputImages as string[] | undefined) ?? []), c.path],
      outputImageUrl: c.path,
      frameMeta: metaJson,
    });
    addImageGroup(
      id,
      [
        { path: c.path, label: "导演合成图", data: { captureMetadata: captureMetadata("combined") } },
        { path: e.path, label: "纯背景图", data: { captureMetadata: captureMetadata("env_only") } },
      ],
      { cols: 2, label: "导演世界输出", sourceHandle: "dw-out" },
    );
  };

  const onTrajectoryOutput = async (output: import("../director/DirectorWorldEditor").DirectorTrajectoryOutput) => {
    const stamp = Date.now();
    const [video, first, last] = await Promise.all([
      canvasSaveVideo(output.videoDataUrl, `director-trajectory-${stamp}.webm`),
      saveChatImageAttachment({ dataUrl: output.firstFrame, filename: `director-trajectory-${stamp}-first.png` }),
      saveChatImageAttachment({ dataUrl: output.lastFrame, filename: `director-trajectory-${stamp}-last.png` }),
    ]);
    const depthJobId = await canvasJobStart({ projectId, nodeId: id, label: "导演运镜深度", task: { kind: "depth_capture", args: { video_path: video.path } } });
    const depthJob = await waitForCanvasJob(depthJobId);
    const depth = depthJob.result as { path?: unknown } | undefined;
    const depthPath = typeof depth?.path === "string" ? depth.path : undefined;
    const relationship = { source: data.splat?.path ?? source, rgb_video: video.path, depth_video: depthPath, first_frame: first.path, last_frame: last.path, camera_path: output.path };
    await Promise.all([
      saveGenerationLog(video.path, "导演世界3D运镜 RGB 渲染", relationship),
      saveGenerationLog(first.path, "导演世界3D运镜首帧", relationship),
      saveGenerationLog(last.path, "导演世界3D运镜尾帧", relationship),
      ...(depthPath ? [saveGenerationLog(depthPath, "导演世界运镜深度推理", relationship)] : []),
    ]);
    updateNodeData(id, {
      trajectoryVideo: video.path,
      trajectoryDepthVideo: depthPath,
      trajectoryFirstFrame: first.path,
      trajectoryLastFrame: last.path,
      trajectoryMeta: JSON.stringify(output.path),
    });
    const target = (data.context as DirectorScene["context"]) ?? linkedShot;
    // **不自动挂成这一镜的首帧 / 尾帧。** 这两张是「片场 + 彩色人偶」的导演合成稿，
    // 不是成品帧：挂上去之后出片走首帧图生视频，H3 从人偶开始动，出一段人偶戏 ——
    // 任务照样成功、不报错。还会盖掉这一镜已经导入 / 出好的首帧。
    // 正路：画布里先把人偶换成角色（参考生图），满意了再到分镜「关键帧」栏导入。
    if (target?.shotId) {
      const detail = await cinemaGetProjectDetail(target.projectId);
      const episode = detail.episodes.find((e) => e.id === target.episodeId);
      if (episode) {
        const next = { ...episode, storyboard: episode.storyboard.map((shot) => shot.id === target.shotId ? {
          ...shot,
          director_camera: shot.director_camera ? {
            ...shot.director_camera,
            rgb_video: video.path,
            depth_video: depthPath ?? null,
            first_frame: first.path,
            last_frame: last.path,
          } : shot.director_camera,
        } : shot) };
        await cinemaSaveEpisode(target.projectId, next);
      }
    }
    addImageGroup(id, [{ path: first.path, label: "运镜首帧（人偶稿）" }, { path: last.path, label: "运镜尾帧（人偶稿）" }], { cols: 2, label: "导演运镜首尾帧 · 先换成角色再挂进分镜", sourceHandle: "dw-out" });
    if (depthJob.status !== "done") throw new Error(depthJob.error || "深度视频生成失败");
  };

  const preview = typeof data.lastCombined === "string" ? data.lastCombined : null;
  const count = actors.length;

  return (
    <div className="flex flex-col gap-2 select-none" style={{ width: 320 }}>
      <NodeHeader
        icon={<Orbit className="h-3.5 w-3.5 text-white/60" />}
        title={source ? "导演世界" : "导演世界 · 等待上游连接全景图"}
        onDelete={() => removeNode(id)}
      />
      <BaseNodeWrapper id={id} selected={selected} sourceHandleId="dw-out">
        <Handle
          type="target"
          position={Position.Left}
          id="dw-in"
          className="!w-2.5 !h-5 !rounded-full !bg-zinc-300 hover:!bg-blue-400 hover:!border-white/50 !border !border-zinc-800 shadow-md transition-transform hover:scale-125 cursor-crosshair z-10"
        />
        <div className="relative h-[180px] overflow-hidden rounded-2xl bg-[#0f0f0f]">
          {preview && <MediaThumb path={preview} alt="导演合成图" className="h-full w-full object-cover" />}
          {preview ? (
            <button
              type="button"
              className="nodrag absolute right-2 top-2 z-20 inline-flex h-6 items-center rounded-full bg-black/45 px-2.5 text-[10px] font-medium text-white/90 backdrop-blur-md hover:bg-black/60"
              onClick={(ev) => {
                ev.stopPropagation();
                setOpen(true);
              }}
              onPointerDown={(ev) => ev.stopPropagation()}
            >
              进入导演世界
            </button>
          ) : (
            <div className="absolute inset-0 flex flex-col items-center justify-center gap-2">
              <button
                type="button"
                className="nodrag rounded-full bg-gradient-to-b from-white to-zinc-300 px-4 py-1.5 text-[11px] font-medium text-black shadow-[0_6px_20px_rgba(255,255,255,0.15)] hover:from-white hover:to-white"
                onClick={(ev) => {
                  ev.stopPropagation();
                  setOpen(true);
                }}
                onPointerDown={(ev) => ev.stopPropagation()}
              >
                进入导演世界
              </button>
              <span className="px-6 text-center text-[10.5px] leading-relaxed text-white/35">
                {source ? "摆人偶，定位置和动作，输出到画布" : "没连全景图也能进（黑色空世界）"}
              </span>
            </div>
          )}
          <div className="pointer-events-none absolute bottom-2 left-2 rounded-full bg-black/45 px-2 py-0.5 text-[10px] text-white/70">
            {count ? `${count} 人物` : "还没摆人"}
            {initial.context ? " · 主线" : ""}
          </div>
        </div>
        <div className="flex gap-2 border-t border-white/5 px-3 py-2">
          <input
            className="nodrag min-w-0 flex-1 rounded-md border border-white/10 bg-black/25 px-2 py-1 text-[10px] text-white/70 outline-none focus:border-cyan-400/50"
            value={splatPath}
            onChange={(e) => setSplatPath(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && applySplatPath()}
            placeholder="可选：粘贴 .ply / .ksplat / .spz 绝对路径"
          />
          <button type="button" className="nodrag rounded-md border border-white/10 px-2 text-[10px] text-white/55 hover:text-white" onClick={applySplatPath}>
            {splat ? "更新" : "载入"}
          </button>
        </div>
        {source && (
          <div className="flex flex-col gap-1.5 border-t border-white/5 px-3 py-2">
            <button type="button" disabled={generatingWorld} className="nodrag w-full rounded-md border border-cyan-400/25 bg-cyan-400/10 py-1.5 text-[10.5px] text-cyan-100 hover:bg-cyan-400/15 disabled:cursor-wait disabled:opacity-50" onClick={() => void generateWorld()}>
              {generatingWorld ? "SHARP 正在生成3D片场…" : splat ? "重新生成3D片场" : "从上游场景图生成3D片场"}
            </button>
            <button type="button" disabled={splittingPano} className="nodrag w-full rounded-md border border-white/10 py-1.5 text-[10.5px] text-white/60 hover:text-white disabled:cursor-wait disabled:opacity-50" onClick={() => void splitPano()}>
              {splittingPano ? "正在拆分360六面…" : data.cubemapManifest ? "重新生成全景六面" : "生成全景六面（多视角预处理）"}
            </button>
          </div>
        )}
        {(sceneAsset || err || splat) && (
          <div className="flex flex-col gap-1 px-3 py-2 text-[10.5px] text-white/45">
            {splat && <span className="truncate text-cyan-200/70">3DGS：{splat.path}</span>}
            {typeof data.cubemapManifest === "string" && <span className="truncate text-white/50">六面清单：{data.cubemapManifest}</span>}
            {sceneAsset && (
              <div className="flex items-center justify-between gap-2">
                <span className="truncate">世界校准：眼高 {eyeHeight.toFixed(2)} 米</span>
                <button type="button" className="nodrag shrink-0 text-white/60 hover:text-white" onClick={() => void saveCalibration()}>
                  {calNote || "存进场景"}
                </button>
              </div>
            )}
            {err && <span className="text-rose-200/80">{err}</span>}
          </div>
        )}
      </BaseNodeWrapper>
      <DirectorWorldEditor
        open={open}
        onClose={() => setOpen(false)}
        panoSource={source}
        splatSource={splat}
        correction={correction}
        frontYawDeg={frontYawDeg}
        eyeHeight={eyeHeight}
        initial={initial}
        onSave={(s) =>
          updateNodeData(id, {
            actors: s.actors,
            camera: s.camera,
            aspect: s.aspect,
            context: s.context,
            namedCameras: s.namedCameras,
            cameraPaths: s.cameraPaths,
            activeCameraId: s.activeCameraId ?? undefined,
            markers: [],
          })
        }
        onCalibrationChange={(p) =>
          updateNodeData(id, {
            ...(p.correction ? { correction: p.correction } : {}),
            ...(p.frontYawDeg !== undefined ? { frontYawDeg: p.frontYawDeg } : {}),
            ...(p.eyeHeight !== undefined ? { calibration: { eyeHeight: p.eyeHeight, scale: data.calibration?.scale ?? 1 } } : {}),
            // 片场缩放跟着片场文件走（换了片场从 1 重来）
            ...(p.splatScale !== undefined && data.splat ? { splat: { ...data.splat, scale: p.splatScale } } : {}),
          })
        }
        onOutput={onOutput}
        onTrajectoryOutput={onTrajectoryOutput}
      />
    </div>
  );
}
