/**
 * 导演世界编辑器：全屏对话框，左侧分区侧栏，右侧视口。
 *
 * 改写自 DramaClaw（ClaymoreLab, Elastic License 2.0）
 * frontend/src/features/viewer-kit/three-d/ThreeDDirectorDialog.tsx：
 * 布局（320px 侧栏 + 视口）、片场 / 添加 / 选中 / 输出四个分区、画幅框、准星、
 * 选中标签、底部提示条、快捷键表（去掉了道具 / 占位 / 挂载相关的键）。
 * 道具和群演后来按我们自己的做法加回来了（界面升级 U3.5，用户 2026-09-27：「道具也可以增加进来了」「群众也需要」）：
 * 道具是资产封面做的立牌，群演是一片灰人偶，换成角色那一步分别处理（`studio_director.rs`）。
 * 出处规则见 DECISIONS.md 2026-09-16。
 *
 * # 两种模式（和 DC 一致）
 *
 * - **主线模式**：选了分集。人物**只能从本集颜色表里选**，颜色锁定 ——
 *   下游靠颜色认人，随手换色就认错人。颜色表在打开分集时自动补齐。
 * - **自由模式**：没选分集。匿名人物 N + 自选颜色。
 *
 * # 按键
 *
 * 打开时在 window 捕获阶段截下所有按键，**不让画布收到** ——
 * 画布的 Delete 删节点、V 切工具、Tab 加节点、⌘Z 撤销画布，在这里都不该生效。
 */
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { Camera, Clapperboard, Keyboard, Loader2, RotateCcw, RotateCw, Route, Send, Trash2, UsersRound, X } from "lucide-react";
import { mediaDataUrl, pathFromMediaUrl } from "../../components/studio/MediaThumb";
import { assetsList, type AssetView } from "../../api/assets";
import { readSplatBlobUrl } from "../../api/world";
import {
  cinemaAssignActorColors,
  cinemaGetProjectDetail,
  cinemaSaveShot,
  cinemaScripts,
  type ActorColor,
  type EpisodeData,
  type ScriptSummary,
} from "../../api/cinema";
import { overwriteNote, shotOptions, withBlocking, withDirectorCamera } from "../pano/directorLinks";
import { ACTOR_PALETTE, nextFreeColor } from "./actorPalette";
import { createDirectorEngine, type DirectorEngine } from "./engine";
import { DEFAULT_SPHERE_RADIUS, SPHERE_RADIUS_RANGE } from "./world";
import { CAMERA_COLORS, cameraColor } from "./cameraMarker";
import { createHistory } from "./history";
import { POSES, poseLabel, type PoseName } from "./poses";
import { blockingSources, castCheck, type BlockingSource } from "./carry";
import { CAMERA_PRESETS, framingCamera, framingFromPoint, motionKeyframes, motionKeyframesAtPoint, pickShotTarget, SIZE_FRAMING, type CameraPreset } from "./framing";
import { display } from "../../shared/shotLanguage";
import {
  FRAME_ASPECTS,
  actorProblems,
  aspectRatio,
  bindIdentity,
  blockingFor,
  blockingSentence,
  buildFrameMeta,
  cameraPreset,
  crowdGrid,
  directorViewFrom,
  isPerson,
  PROP_COLOR,
  lookAtAngles,
  nextCameraName,
  defaultCamera,
  facingCameraYaw,
  frameRect,
  interpolateCamera,
  nextAnonymousLabel,
  type DirectorActor,
  type DirectorCamera,
  type DirectorFrameMeta,
  type NamedCamera,
  type CameraPath,
  type Rect,
} from "./scene";

export interface DirectorScene {
  actors: DirectorActor[];
  camera: DirectorCamera;
  aspect: string;
  /** 分集上下文。自由模式为 null */
  context: { projectId: string; episodeId: string; shotId: string | null } | null;
  namedCameras?: NamedCamera[];
  cameraPaths?: CameraPath[];
  /** 当前机位（输出、写进分镜、运镜、绑定都按它）。没有 = 第一个 */
  activeCameraId?: string | null;
}

export interface DirectorOutput {
  combined: string;
  envOnly: string;
  meta: DirectorFrameMeta;
}
export interface DirectorTrajectoryOutput { videoDataUrl: string; firstFrame: string; lastFrame: string; path: CameraPath }

export interface DirectorWorldEditorProps {
  open: boolean;
  onClose: () => void;
  /** 全景图（本机路径或 data URL）。没有就是黑色空世界 */
  panoSource: string | null;
  splatSource?: { path: string; format: "ply" | "ksplat" | "spz"; convention?: "opencv_camera"; scale?: number } | null;
  correction: { roll: number; pitch: number; yaw: number };
  frontYawDeg: number;
  eyeHeight: number;
  groundHeight?: number;
  initial: DirectorScene;
  /** 这一镜还没摆过、打开时沿用了第几镜的站位（工作区传；画布节点没有） */
  carriedFrom?: number | null;
  /** 场景有变化就调（已防抖）。存进节点 */
  onSave: (scene: DirectorScene) => void;
  /** 校准改了 */
  onCalibrationChange: (patch: { correction?: DirectorWorldEditorProps["correction"]; frontYawDeg?: number; eyeHeight?: number; groundHeight?: number; splatScale?: number }) => void;
  /** 输出到画布节点 */
  onOutput: (output: DirectorOutput) => Promise<void>;
  onTrajectoryOutput?: (output: DirectorTrajectoryOutput) => Promise<void>;
}

const NUDGE = 0.1;

type LeftTab = "stage" | "people" | "cameras" | "motion" | "output";
/** 左侧图标栏。顺序就是做一镜的顺序：选片场 → 摆人 → 定机位 → 运镜 → 输出 */
const LEFT_TABS: { id: LeftTab; label: string; icon: typeof Camera }[] = [
  { id: "stage", label: "片场", icon: Clapperboard },
  { id: "people", label: "人物", icon: UsersRound },
  { id: "cameras", label: "机位", icon: Camera },
  { id: "motion", label: "运镜", icon: Route },
  { id: "output", label: "输出", icon: Send },
];
const FRAME_PADDING = 16;

export function DirectorWorldEditor(props: DirectorWorldEditorProps) {
  if (!props.open) return null;
  return createPortal(<EditorBody {...props} />, document.body);
}

function EditorBody({
  onClose,
  panoSource,
  splatSource,
  correction,
  frontYawDeg,
  eyeHeight,
  groundHeight = 0,
  initial,
  carriedFrom,
  onSave,
  onCalibrationChange,
  onOutput,
  onTrajectoryOutput,
}: DirectorWorldEditorProps) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const badgeRef = useRef<HTMLDivElement | null>(null);
  /** 外部片场视口里的机位名字标签 DOM（位置每帧按引擎投影更新，不走 React） */
  const cameraTagRefs = useRef(new Map<string, HTMLButtonElement>());
  const actorTagRefs = useRef(new Map<string, HTMLButtonElement>());
  const axisDotRefs = useRef(new Map<string, HTMLButtonElement>());
  const axisLineRefs = useRef(new Map<string, HTMLDivElement>());
  const namedCamerasRef = useRef<NamedCamera[]>([]);
  const engineRef = useRef<DirectorEngine | null>(null);

  const [actors, setActorsRaw] = useState<DirectorActor[]>(initial.actors);
  const [camera, setCamera] = useState<DirectorCamera>(initial.camera);
  // **每个场面至少一个机位**：老存档没有机位时，把原来的相机收成「机位 1」—— 打开看到的画面和以前一样
  const [namedCameras, setNamedCameras] = useState<NamedCamera[]>(() =>
    initial.namedCameras?.length ? initial.namedCameras : [{ id: "camera-1", name: "机位 1", camera: initial.camera }],
  );
  /** 当前机位：输出、写进分镜、运镜、绑定都按它。`camera` state 始终是它的相机 */
  const [activeCameraId, setActiveCameraId] = useState<string>(() => {
    const list = initial.namedCameras?.length ? initial.namedCameras : [{ id: "camera-1" }];
    return list.some((item) => item.id === initial.activeCameraId) ? (initial.activeCameraId as string) : list[0].id;
  });
  /** 当前在右栏显示属性的机位。导演视角下与选中人物互斥。 */
  const [selectedCameraId, setSelectedCameraId] = useState<string | null>(null);
  /**
   * 两个视角（界面升级 U2，照 libtv）：
   * - `camera` 机位视角：仍从外部看完整片场，右栏编辑当前摄像机并显示真实取景预览；
   * - `director` 导演视角：从外部摆人、选机位。两个页签共用独立观察相机。
   */
  const [viewMode, setViewModeRaw] = useState<"camera" | "director">("camera");
  /**
   * 机位视角里视口是不是正**透过当前机位**看（用户 2026-09-28：「点击某一个机位，要自动切换到这个机位的真实视角，可移动，
   * 但是点击重置视角要恢复到这个摄像机本身的观测视角」）。自己转 / 拖 / 缩放开了就是 false，点重置视角回到 true。
   * 为 true 时右栏改机位参数，视口跟着走
   */
  const [throughLens, setThroughLens] = useState(true);
  /** 坐标轴拖机位：移动还是旋转（用户：「摄像机缺少了旋转功能」） */
  const [cameraGizmoMode, setCameraGizmoModeRaw] = useState<"translate" | "rotate">("translate");
  const viewModeRef = useRef(viewMode);
  viewModeRef.current = viewMode;
  const directorCamRef = useRef<DirectorCamera | null>(null);
  const [cameraPreview, setCameraPreview] = useState<string | null>(null);
  const [cameraPaths, setCameraPaths] = useState<CameraPath[]>(initial.cameraPaths ?? []);
  const [aspect, setAspect] = useState(initial.aspect || "16:9");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [panelHidden, setPanelHidden] = useState(false);
  const [tab, setTab] = useState<LeftTab>("stage");
  const [stageActive, setStageActive] = useState(false);
  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  const [status, setStatus] = useState("加载人偶…");
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState<"output" | "blocking" | "trajectory" | null>(null);
  const [viewSize, setViewSize] = useState({ w: 0, h: 0 });

  // ── 分集上下文 + 颜色表 ──
  const [scripts, setScripts] = useState<ScriptSummary[]>([]);
  const [context, setContext] = useState(initial.context);
  const [episode, setEpisode] = useState<EpisodeData | null>(null);
  const [colorTable, setColorTable] = useState<ActorColor[]>([]);
  const [unassigned, setUnassigned] = useState<string[]>([]);
  const [identityId, setIdentityId] = useState("");
  const [anonColor, setAnonColor] = useState(() => nextFreeColor(initial.actors.map((a) => a.color)));
  const [newScale, setNewScale] = useState(1);
  /** 能摆进来的道具资产（分集 → 剧集 → 全局；自由模式只有全局） */
  const [propAssets, setPropAssets] = useState<AssetView[]>([]);
  const [propId, setPropId] = useState("");
  /** 导演视角里全景球多大（照 libtv 的「球形半径」）。看的偏好，存本机，不进场面 */
  const [sphereRadius, setSphereRadiusRaw] = useState(() => {
    try {
      const v = Number(localStorage.getItem(SPHERE_RADIUS_KEY));
      return v >= SPHERE_RADIUS_RANGE[0] && v <= SPHERE_RADIUS_RANGE[1] ? v : DEFAULT_SPHERE_RADIUS;
    } catch {
      return DEFAULT_SPHERE_RADIUS;
    }
  });
  const [panoramaProxy, setPanoramaProxy] = useState(true);
  const [actorLabels, setActorLabels] = useState(true);
  const [gridSnap, setGridSnap] = useState(true);
  const [gaussianSnap, setGaussianSnap] = useState(false);
  const [groundVisible, setGroundVisible] = useState(true);
  const [groundOpacity, setGroundOpacity] = useState(0.5);
  const previousGroundHeight = useRef(groundHeight);
  const setSphereRadius = (r: number) => {
    setSphereRadiusRaw(r);
    engineRef.current?.setSphereRadius(r);
    try {
      localStorage.setItem(SPHERE_RADIUS_KEY, String(r));
    } catch {
      // 存不了就只管这一次
    }
  };
  /** 顶部「沿用了第 N 镜的站位」提示，点「知道了」或清空后收起 */
  const [carried, setCarried] = useState<number | null>(carriedFrom ?? null);
  const [copyFromId, setCopyFromId] = useState("");
  /** 复制会替换场上已有的人：第一下只是亮出「确认替换」 */
  const [copyArmed, setCopyArmed] = useState(false);
  /**
   * 镜头锁在拍摄点。**默认锁** —— 360 全景只有地面是准的，离开拍摄点墙和家具就拉花
   * （2026-09-16 实测：往前走一米多柜台就拖成长条）。要俯视看站位时临时解锁。
   */
  const [cameraLocked, setCameraLocked] = useState(!splatSource);
  const [blockingNote, setBlockingNote] = useState("");

  const mainline = !!context;
  useEffect(() => {
    if (selectedId) setSelectedCameraId(null);
  }, [selectedId]);
  const selected = viewMode === "camera" ? null : actors.find((a) => a.id === selectedId) ?? null;
  // 右栏：**机位视角**显示摄像机（当前机位）；**导演视角**是 3D 场景（用户 2026-09-28：「在导演视角要切换回 3D 场景右边栏」）——
  // 导演视角里选中机位只是为了拖三维坐标轴，右栏不跟着换
  const selectedCam = viewMode === "camera" ? namedCameras.find((c) => c.id === activeCameraId) ?? null : null;
  const problems = useMemo(
    () => actorProblems(actors, mainline, { cam: camera, eyeHeight, hasPano: !!panoSource && !splatSource }),
    [actors, mainline, camera, eyeHeight, panoSource, splatSource],
  );

  // 最新值给按键 / 回调用，避免闭包拿到旧 state
  const latest = useRef({ actors, camera, selectedId, stageActive, shortcutsOpen });
  latest.current = { actors, camera, selectedId, stageActive, shortcutsOpen };

  /**
   * 改人偶一律走这里：先进撤销栈再改。`kind` 相同的连续操作（连按方向键、拖滑块）合并成一步。
   * 同步更新 `latest`，同一个事件里连着改两次时第二次拿到的是改过的值。
   */
  const historyRef = useRef(createHistory<DirectorActor[]>());
  const setActors = (next: DirectorActor[] | ((list: DirectorActor[]) => DirectorActor[]), kind = "edit") => {
    const prev = latest.current.actors;
    const value = typeof next === "function" ? next(prev) : next;
    if (value === prev) return;
    historyRef.current.record(prev, kind);
    latest.current = { ...latest.current, actors: value };
    setActorsRaw(value);
  };
  const applyHistory = (dir: "undo" | "redo") => {
    const h = historyRef.current;
    const cur = latest.current.actors;
    const value = dir === "undo" ? h.undo(cur) : h.redo(cur);
    if (!value) {
      setStatus(dir === "undo" ? "没有可以撤销的了" : "没有可以重做的了");
      return;
    }
    latest.current = { ...latest.current, actors: value };
    setActorsRaw(value);
    if (latest.current.selectedId && !value.some((a) => a.id === latest.current.selectedId)) setSelectedId(null);
    setStatus(dir === "undo" ? "已撤销" : "已重做");
  };

  useEffect(() => {
    void cinemaScripts().then(setScripts).catch(() => setScripts([]));
  }, []);

  useEffect(() => {
    let alive = true;
    const scopes = context
      ? [
          { kind: "episode" as const, project_id: context.projectId, episode_id: context.episodeId },
          { kind: "project" as const, project_id: context.projectId },
          { kind: "global" as const },
        ]
      : [{ kind: "global" as const }];
    void Promise.all(scopes.map((sc) => assetsList(sc).catch(() => [] as AssetView[]))).then((lists) => {
      if (!alive) return;
      const seen = new Set<string>();
      const props = lists.flat().filter((a) => a.kind === "prop" && !seen.has(a.id) && (seen.add(a.id), true));
      setPropAssets(props);
      setPropId((cur) => (props.some((a) => a.id === cur) ? cur : props[0]?.id ?? ""));
    });
    return () => {
      alive = false;
    };
  }, [context?.projectId, context?.episodeId]);

  /** 打开分集：读分集 + 补齐颜色表 */
  useEffect(() => {
    if (!context) {
      setEpisode(null);
      setColorTable([]);
      setUnassigned([]);
      return;
    }
    let alive = true;
    (async () => {
      try {
        const [detail, colors] = await Promise.all([
          cinemaGetProjectDetail(context.projectId),
          cinemaAssignActorColors(context.projectId, context.episodeId),
        ]);
        if (!alive) return;
        setEpisode(detail.episodes.find((e) => e.id === context.episodeId) ?? null);
        setColorTable(colors.colors);
        setUnassigned(colors.unassigned);
        setIdentityId((cur) => (colors.colors.some((c) => c.asset_id === cur) ? cur : colors.colors[0]?.asset_id ?? ""));
      } catch (e) {
        if (alive) setErr(`读分集失败：${String(e)}`);
      }
    })();
    return () => {
      alive = false;
    };
  }, [context?.projectId, context?.episodeId]);

  // ── 引擎 ──
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const engine = createDirectorEngine({
      canvas,
      onSelect: (id) => {
        setSelectedId(id);
        if (id) setSelectedCameraId(null);
      },
      onActorMoved: (id, position) =>
        setActors((list) => list.map((a) => (a.id === id ? { ...a, position } : a)), `move:${Date.now()}`),
      onPlace: (point) => placeActorAt(point),
      // 两个页签都从外面观察完整片场；机位参数只在右栏改，观察相机不写进机位
      onCameraChange: (c) => {
        // 导演视角：记下来，切回来时回到这里；机位视角：自己挪开了，不再跟着机位走
        if (viewModeRef.current === "director") directorCamRef.current = c;
        else setThroughLens(false);
      },
      onStatus: (m) => setStatus(m),
      onError: (m) => setErr(m),
      loadImage: (path) => mediaDataUrl(path),
      // 点中机位图标：选中它（挂三维坐标轴）；拖坐标轴松手：写回这个机位的位置
      // 导演视角：选中它、挂坐标轴；机位视角：透过它看
      onSelectCamera: (id) => {
        if (viewModeRef.current === "camera") {
          lookThroughRef.current(id);
          return;
        }
        setSelectedId(null);
        setSelectedCameraId(id);
      },
      onCameraPose: (id, pose) => poseCameraRef.current(id, pose),
    });
    engineRef.current = engine;
    engine.setSphereRadius(sphereRadiusRef.current);
    engine.setNavMode("orbit");
    const observer = directorViewFrom(initial.camera);
    directorCamRef.current = observer;
    engine.setCamera(observer);
    engine.setActors(latest.current.actors);
    void engine.ready.then(() => setStatus((s) => (s === "加载人偶…" ? "" : s)));
    const ro = new ResizeObserver(() => {
      const w = canvas.clientWidth;
      const h = canvas.clientHeight;
      engine.resize(w, h);
      setViewSize({ w, h });
    });
    ro.observe(canvas);
    return () => {
      ro.disconnect();
      engine.dispose();
      engineRef.current = null;
    };
    // 引擎只建一次；后面的变化各自同步
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const engine = engineRef.current;
    if (!engine) return;
    if (!panoSource) {
      engine.setPano(null);
      return;
    }
    let alive = true;
    mediaDataUrl(panoSource)
      .then((url) => alive && engine.setPano(url))
      .catch((e) => alive && setErr(`取不到全景图：${String(e)}`));
    return () => {
      alive = false;
    };
  }, [panoSource]);

  useEffect(() => {
    const engine = engineRef.current;
    if (!engine) return;
    let alive = true;
    if (!splatSource?.path) {
      void engine.setSplat(null);
      return;
    }
    // **不走 mediaDataUrl**：那条是 base64、上限 24 MB，SHARP 的片场一份就五六十 MB。
    // 按二进制读成 blob 地址（见 `world_read_splat`），用完释放。
    let blobUrl: string | null = null;
    readSplatBlobUrl(splatSource.path)
      .then((url) => {
        blobUrl = url;
        if (!alive) return;
        return engine.setSplat({ url, format: splatSource.format, convention: splatSource.convention, scale: splatSource.scale });
      })
      .catch((e) => alive && setErr(`取不到 3DGS 文件：${String(e)}`));
    return () => {
      alive = false;
      if (blobUrl) URL.revokeObjectURL(blobUrl);
    };
  }, [splatSource?.path, splatSource?.format, splatSource?.convention]);

  useEffect(() => engineRef.current?.setCorrection(correction), [correction]);
  useEffect(() => engineRef.current?.setFrontYaw(frontYawDeg), [frontYawDeg]);
  useEffect(() => engineRef.current?.setEyeHeight(eyeHeight), [eyeHeight]);
  useEffect(() => engineRef.current?.setGroundHeight(groundHeight), [groundHeight]);
  useEffect(() => {
    const delta = groundHeight - previousGroundHeight.current;
    previousGroundHeight.current = groundHeight;
    if (Math.abs(delta) < 1e-6) return;
    setActorsRaw((list) => list.map((a) => ({ ...a, position: [a.position[0], Math.round((a.position[1] + delta) * 100) / 100, a.position[2]] })));
  }, [groundHeight]);
  useEffect(() => engineRef.current?.setActors(actors), [actors]);
  useEffect(() => engineRef.current?.setSelected(selectedId), [selectedId]);
  // 锁拍摄点现在是「创建机位」的约束；外部观察相机始终自由绕转
  useEffect(() => engineRef.current?.setCameraLocked(false), [cameraLocked, viewMode]);
  useEffect(() => engineRef.current?.setSplatScale(splatSource?.scale ?? 1), [splatSource?.scale]);
  useEffect(() => engineRef.current?.setPanoramaProxy(panoramaProxy), [panoramaProxy]);
  useEffect(() => engineRef.current?.setGridVisible(groundVisible), [groundVisible]);
  useEffect(() => engineRef.current?.setGridOpacity(groundOpacity), [groundOpacity]);
  useEffect(() => engineRef.current?.setGridSnap(gridSnap), [gridSnap]);
  useEffect(() => engineRef.current?.setGaussianGroundSnap(gaussianSnap), [gaussianSnap]);
  useEffect(() => {
    // 全景球必须留在拍摄点；3DGS 的价值正是允许相机产生真实位移和视差。
    setCameraLocked(!splatSource);
  }, [splatSource?.path]);

  // ── 存盘（防抖）──
  const saveRef = useRef(onSave);
  saveRef.current = onSave;
  useEffect(() => {
    const t = window.setTimeout(() => saveRef.current({ actors, camera, aspect, context, namedCameras, cameraPaths, activeCameraId }), 600);
    return () => window.clearTimeout(t);
  }, [actors, camera, aspect, context, namedCameras, cameraPaths, activeCameraId]);

  // ── 机位（界面升级 U2）──
  // `camera` 是当前出片机位；外部观察相机独立保存，不跟着它跳
  useEffect(() => {
    setNamedCameras((list) => {
      const i = list.findIndex((c) => c.id === activeCameraId);
      if (i < 0 || JSON.stringify(list[i].camera) === JSON.stringify(camera)) return list;
      const next = [...list];
      next[i] = { ...next[i], camera };
      return next;
    });
  }, [camera, activeCameraId]);
  const setShotCamera = (c: DirectorCamera) => {
    setCamera(c);
  };
  const activateCamera = (id: string) => {
    const target = namedCameras.find((c) => c.id === id);
    if (!target) return;
    setActiveCameraId(id);
    setShotCamera(target.camera);
    setStatus(`当前机位：${target.name}`);
  };
  const updateNamedCamera = (id: string, patch: Partial<NamedCamera>) => {
    setNamedCameras((list) => list.map((c) => (c.id === id ? { ...c, ...patch } : c)));
    if (id === activeCameraId && patch.camera) setShotCamera(patch.camera);
  };
  /**
   * 坐标轴拖完一个机位（移动或旋转）。转过了就不再自动注视 —— 否则注视那一步马上把角度对回去，等于白转。
   * 引擎回调闭包是建引擎那一刻的，读最新的走 ref
   */
  const poseCameraRef = useRef<(id: string, pose: Pick<DirectorCamera, "position" | "yawDeg" | "pitchDeg" | "rollDeg">) => void>(() => undefined);
  poseCameraRef.current = (id, pose) => {
    const c = namedCameras.find((x) => x.id === id);
    if (!c) return;
    const turned = Math.abs(pose.yawDeg - c.camera.yawDeg) > 0.05 || Math.abs(pose.pitchDeg - c.camera.pitchDeg) > 0.05 || Math.abs((pose.rollDeg ?? 0) - (c.camera.rollDeg ?? 0)) > 0.05;
    updateNamedCamera(id, { camera: { ...c.camera, ...pose }, ...(turned ? { lookAt: null } : {}) });
  };
  /** 机位视角：视口放到这个机位上（透过它看），它成为当前机位。之后能自己转开，重置视角回来 */
  const lookThrough = (id: string | null) => {
    const target = namedCameras.find((c) => c.id === id) ?? namedCameras.find((c) => c.id === activeCameraId);
    if (!target) return;
    setActiveCameraId(target.id);
    setShotCamera(target.camera);
    setSelectedId(null);
    setSelectedCameraId(target.id);
    engineRef.current?.viewFrom(target.camera);
    setThroughLens(true);
    setStatus(`机位视角：透过「${target.name}」看 —— 可以转、拖、缩放去看周围（不改机位），点「重置视角」回到这个机位`);
  };
  const lookThroughRef = useRef(lookThrough);
  lookThroughRef.current = lookThrough;
  const setCameraGizmoMode = (m: "translate" | "rotate") => {
    setCameraGizmoModeRaw(m);
    engineRef.current?.setCameraGizmoMode(m);
  };
  const deleteNamedCamera = (id: string) => {
    if (namedCameras.length <= 1) {
      setStatus("至少留一个机位");
      return;
    }
    const rest = namedCameras.filter((c) => c.id !== id);
    setNamedCameras(rest);
    if (selectedCameraId === id) setSelectedCameraId(null);
    if (activeCameraId === id) {
      setActiveCameraId(rest[0].id);
      setShotCamera(rest[0].camera);
    }
  };
  // ── 预设机位 / 按分镜摆机（界面升级 U3）──
  /** 预设和按分镜摆机对准谁：选中的人；没选就这一镜说话的人；再没有就第一个 */
  const shotRow = context?.shotId ? episode?.storyboard.find((x) => x.id === context.shotId) ?? null : null;
  const framingTarget = () => {
    const picked = pickShotTarget(actors, shotRow ?? {});
    const target = selected ?? picked.target;
    const other = actors.find((a) => isPerson(a) && a.id !== target?.id) ?? null;
    return { target, other };
  };
  /**
   * 全景片场、镜头锁在拍摄点：摆机只转向 + 调视场角（全景挪开拍摄点就拉花）。3D 片场或解了锁才真的挪相机
   */
  const fixedPoint = !!panoSource && !splatSource && cameraLocked;
  const addPresetCamera = (p: CameraPreset) => {
    const { target, other } = framingTarget();
    if (!target) {
      setStatus("先在人物页摆一个人 —— 预设机位要对准人");
      return;
    }
    const cam = fixedPoint
      ? framingFromPoint([0, eyeHeight, 0], { target, other, size: p.size, angle: p.angle })
      : framingCamera({ target, other, size: p.size, angle: p.angle, side: p.side, shoulder: p.shoulder, fovDeg: p.fovDeg ?? 50 });
    const item: NamedCamera = { id: `camera-${Date.now()}`, name: p.label, camera: cam, lookAt: target.id };
    setNamedCameras((list) => [...list, item]);
    setActiveCameraId(item.id);
    setShotCamera(cam);
    setStatus(`新建机位「${p.label}」，对准 ${target.label}（当前机位）${fixedPoint && p.angle !== "eye_level" && p.angle !== "dutch" ? " · 全景锁在拍摄点，只能平视转过去；要真的仰拍 / 俯拍 / 过肩，用 3D 片场或解锁镜头" : ""}`);
  };
  const frameFromShot = () => {
    if (!shotRow) return;
    const { target, other } = framingTarget();
    if (!target) {
      setStatus("先在人物页把这一镜的人摆上 —— 按分镜摆机要对准人");
      return;
    }
    const size = shotRow.scale || "medium";
    const angle = shotRow.camera_angle || "eye_level";
    const aimH = (SIZE_FRAMING as Record<string, { aimH: number }>)[size]?.aimH ?? 1.35;
    const aim: [number, number, number] = [target.position[0], target.position[1] + aimH * target.scale, target.position[2]];
    const cam = fixedPoint ? framingFromPoint([0, eyeHeight, 0], { target, other, size, angle }) : framingCamera({ target, other, size, angle, fovDeg: 50 });
    const frames = fixedPoint ? motionKeyframesAtPoint(cam, shotRow.camera_movement) : motionKeyframes(cam, aim, shotRow.camera_movement);
    const name = `第 ${shotRow.shot_num} 镜`;
    const start = frames ? frames[0] : cam;
    // 有运镜时不挂注视目标：注视会把关键帧的起点重新对准，推拉的路径就歪了
    const existing = namedCameras.find((c) => c.name === name);
    const item: NamedCamera = { id: existing?.id ?? `camera-${Date.now()}`, name, camera: start, lookAt: frames ? null : target.id };
    setNamedCameras((list) => (existing ? list.map((c) => (c.id === item.id ? item : c)) : [...list, item]));
    setActiveCameraId(item.id);
    setShotCamera(start);
    if (frames) {
      setCameraPaths([{ id: cameraPaths[0]?.id ?? `path-${Date.now()}`, name: "主运镜", durationSec: Math.max(1, shotRow.duration || 5), cameras: frames }]);
    }
    const words = [display("shotSize", size) || size, display("angle", shotRow.camera_angle) || "平视", display("move", shotRow.camera_movement) || "固定"].join(" · ");
    setStatus(`按${name}摆好：${words}，对准 ${target.label}${frames ? `，运镜 ${frames.length} 个关键帧 / ${shotRow.duration} 秒` : ""}${fixedPoint ? " · 全景锁在拍摄点：用转向 + 视场角摆，推拉换成变焦" : ""}`);
  };

  const setViewMode = (m: "camera" | "director") => {
    const engine = engineRef.current;
    if (!engine || m === viewModeRef.current) return;
    viewModeRef.current = m;
    setViewModeRaw(m);
    if (m === "director") {
      // 回到离开导演视角时那个位置
      engine.setCamera(directorCamRef.current ?? directorViewFrom(camera));
      setThroughLens(false);
      setStatus("导演视角：左键绕着转、滚轮缩放、右键平移 · 点机位图标挂坐标轴（移动 / 旋转）· P 切回机位视角");
    } else {
      lookThrough(activeCameraId);
    }
  };
  /** 截图 / 输出永远按当前机位截：导演视角下临时切过去，截完切回来（同一帧里，看不到闪） */
  const withShotView = <T,>(fn: (engine: DirectorEngine) => T): T | null => {
    const engine = engineRef.current;
    if (!engine) return null;
    return engine.withCamera(camera, () => fn(engine));
  };
  // 注视目标：那个人挪了，机位重新对准他胸口
  useEffect(() => {
    for (const c of namedCameras) {
      if (!c.lookAt) continue;
      const a = actors.find((x) => x.id === c.lookAt);
      if (!a) continue;
      const aim = lookAtAngles(c.camera.position, [a.position[0], a.position[1] + 1.0 * a.scale, a.position[2]]);
      if (Math.abs(aim.yawDeg - c.camera.yawDeg) > 0.05 || Math.abs(aim.pitchDeg - c.camera.pitchDeg) > 0.05) {
        updateNamedCamera(c.id, { camera: { ...c.camera, ...aim } });
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [actors, namedCameras]);
  // 两个页签都从外部检查片场，因此始终画出机位
  useEffect(() => {
    engineRef.current?.setCameraMarkers(
      namedCameras.map((c) => ({
        id: c.id,
        camera: c.camera,
        aspect: aspectRatio(aspect),
        active: c.id === activeCameraId,
        // 坐标轴只在导演视角挂；机位视角里正透过它看的那个不画（图标和取景框就在眼睛上）
        selected: viewMode === "director" && c.id === selectedCameraId,
        hidden: viewMode === "camera" && throughLens && c.id === activeCameraId,
        color: c.color,
      })),
    );
  }, [viewMode, namedCameras, activeCameraId, selectedCameraId, aspect, throughLens]);
  // 机位视角正透过当前机位看时，右栏改它（操作杆、输入框、注视目标），视口跟着走
  useEffect(() => {
    if (viewMode === "camera" && throughLens) engineRef.current?.viewFrom(camera);
  }, [viewMode, throughLens, camera]);

  const close = useCallback(() => {
    saveRef.current({ actors: latest.current.actors, camera: latest.current.camera, aspect, context, namedCameras, cameraPaths, activeCameraId });
    onClose();
  }, [aspect, context, onClose, namedCameras, cameraPaths, activeCameraId]);

  namedCamerasRef.current = namedCameras;
  // ── 选中标签跟着人走；机位名字跟着机位走 ──
  useEffect(() => {
    let raf = 0;
    const tick = () => {
      const eng = engineRef.current;
      if (eng) {
        for (const axis of eng.orientationAxes()) {
          const dot = axisDotRefs.current.get(axis.id);
          const line = axisLineRefs.current.get(axis.id);
          const dx = axis.x * 24;
          const dy = axis.y * 24;
          if (dot) {
            dot.style.transform = `translate(${dx}px, ${dy}px) translate(-50%, -50%)`;
            dot.style.opacity = String(0.58 + Math.max(-1, Math.min(1, -axis.depth)) * 0.22);
            dot.style.zIndex = String(Math.round(20 - axis.depth * 10));
          }
          if (line) {
            const length = Math.hypot(dx, dy);
            line.style.width = `${length}px`;
            line.style.transform = `rotate(${Math.atan2(dy, dx)}rad)`;
            line.style.opacity = String(0.45 + Math.max(-1, Math.min(1, -axis.depth)) * 0.2);
          }
          const back = axisDotRefs.current.get(`-${axis.id}`);
          if (back) {
            back.style.transform = `translate(${-dx}px, ${-dy}px) translate(-50%, -50%)`;
            back.style.opacity = String(0.42 + Math.max(-1, Math.min(1, axis.depth)) * 0.18);
            back.style.zIndex = String(Math.round(20 + axis.depth * 10));
          }
        }
        const labels = eng.labels();
        for (const a of latest.current.actors) {
          const tag = actorTagRefs.current.get(a.id);
          if (!tag) continue;
          const l = labels.find((x) => x.id === a.id);
          tag.style.opacity = actorLabels && l?.visible ? "1" : "0";
          tag.style.pointerEvents = actorLabels && l?.visible ? "auto" : "none";
          if (l) tag.style.transform = `translate(${l.x}px, ${l.y}px) translate(-50%, -100%)`;
        }
        for (const c of namedCamerasRef.current) {
          const tag = cameraTagRefs.current.get(c.id);
          if (!tag) continue;
          // 标签往镜头朝的方向推出去 0.8 米：全景锁在拍摄点时所有机位起点都在同一处，标签放在起点上会叠成一坨
          const y = (c.camera.yawDeg * Math.PI) / 180;
          const pt = (c.camera.pitchDeg * Math.PI) / 180;
          const p = eng.project([
            c.camera.position[0] + Math.sin(y) * Math.cos(pt) * 0.8,
            c.camera.position[1] + Math.sin(pt) * 0.8 + 0.3,
            c.camera.position[2] - Math.cos(y) * Math.cos(pt) * 0.8,
          ]);
          // 正透过它看的那个机位不挂标签（就在眼前，挡画面）
          const show = p.visible && !(lensRef.current.through && c.id === lensRef.current.active);
          tag.style.opacity = show ? "1" : "0";
          tag.style.pointerEvents = show ? "auto" : "none";
          tag.style.transform = `translate(${p.x}px, ${p.y}px) translate(-50%, -100%)`;
        }
      }
      const el = badgeRef.current;
      const engine = engineRef.current;
      const id = latest.current.selectedId;
      if (el && engine && id) {
        const l = engine.labels().find((x) => x.id === id);
        if (l?.visible) {
          el.style.opacity = "1";
          el.style.transform = `translate(${l.x}px, ${l.y}px) translate(-50%, -100%)`;
        } else {
          el.style.opacity = "0";
        }
      }
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [actorLabels]);

  const frame: Rect = useMemo(
    () => frameRect(viewSize.w, viewSize.h, aspect, FRAME_PADDING),
    [viewSize.w, viewSize.h, aspect],
  );

  // 右栏摄像机的小窗预览：按所选机位临时渲染，不改变外部观察视角（停手 250ms 再截）
  useEffect(() => {
    const cameraId = viewMode === "camera" ? activeCameraId : selectedCameraId;
    if (!cameraId) {
      setCameraPreview(null);
      return;
    }
    const c = namedCameras.find((x) => x.id === cameraId);
    const engine = engineRef.current;
    if (!c || !engine) return;
    const t = window.setTimeout(() => {
      engine.withCamera(c.camera, () => {
        setCameraPreview(engine.capture("combined", frame, 480));
      });
    }, 250);
    return () => window.clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [viewMode, activeCameraId, selectedCameraId, namedCameras, actors, frame]);


  // ── 人偶操作 ──
  const identity = colorTable.find((c) => c.asset_id === identityId) ?? null;

  const placeActorAt = useCallback(
    (point: [number, number, number] | null) => {
      if (!point) {
        setStatus("准星没有指到地面 —— 把视角往下压一点");
        return;
      }
      const cam = engineRef.current?.getCamera() ?? latest.current.camera;
      const list = latest.current.actors;
      const yawDeg = facingCameraYaw(point, cam.position);
      if (mainlineRef.current) {
        const id = identityRef.current;
        const who = colorTableRef.current.find((c) => c.asset_id === id);
        if (!who) {
          setStatus("本集颜色表里没有可放的人物");
          return;
        }
        const existing = list.find((a) => a.assetId === who.asset_id);
        if (existing) {
          // 同一个人放两次会撞色 —— 下游就分不清谁是谁。挪过去而不是再放一个
          setActors(list.map((a) => (a.id === existing.id ? { ...a, position: point } : a)), `place:${Date.now()}`);
          setSelectedId(existing.id);
          setStatus(`「${who.name}」已经在场，挪到这里`);
          return;
        }
        const actor: DirectorActor = {
          id: `actor-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`,
          label: who.name,
          color: who.color,
          assetId: who.asset_id,
          position: point,
          yawDeg,
          scale: newScaleRef.current,
          pose: "standing",
        };
        setActors([...list, actor], `place:${Date.now()}`);
        setSelectedId(actor.id);
        return;
      }
      const color = anonColorRef.current;
      const actor: DirectorActor = {
        id: `actor-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`,
        label: nextAnonymousLabel(list),
        color,
        position: point,
        yawDeg,
        scale: newScaleRef.current,
        pose: "standing",
      };
      const next = [...list, actor];
      setActors(next, `place:${Date.now()}`);
      setSelectedId(actor.id);
      setAnonColor(nextFreeColor(next.map((a) => a.color)));
    },
    [],
  );

  /** 道具放到准星：有图的是一块立牌（就是资产的封面），没图 / 不绑资产是一个方块 */
  const placePropAtCrosshair = (asset: AssetView | null) => {
    const point = engineRef.current?.crosshairGround() ?? null;
    if (!point) {
      setStatus("准星没有指到地面 —— 把视角往下压一点");
      return;
    }
    const list = latest.current.actors;
    const cam = engineRef.current?.getCamera() ?? latest.current.camera;
    const actor: DirectorActor = {
      id: `prop-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`,
      label: asset?.name ?? `道具 ${list.filter((a) => a.kind === "prop").length + 1}`,
      color: PROP_COLOR,
      kind: "prop",
      assetId: asset?.id,
      imagePath: asset?.cover_url ? pathFromMediaUrl(asset.cover_url) : undefined,
      position: point,
      yawDeg: facingCameraYaw(point, cam.position),
      scale: 1,
      pose: "standing",
    };
    setActors([...list, actor], `place:${Date.now()}`);
    setSelectedId(actor.id);
  };

  /** 在准星铺一片群演（3×3） */
  const placeCrowdAtCrosshair = () => {
    const point = engineRef.current?.crosshairGround() ?? null;
    if (!point) {
      setStatus("准星没有指到地面 —— 把视角往下压一点");
      return;
    }
    const list = latest.current.actors;
    const cam = engineRef.current?.getCamera() ?? latest.current.camera;
    const crowd = crowdGrid(point, cam.position, list);
    setActors([...list, ...crowd], `crowd:${Date.now()}`);
    setSelectedId(null);
    setStatus(`铺了 ${crowd.length} 个群演 —— 出首帧时统一换成路人，⌘Z 一次撤掉整片`);
  };

  // ── 站位跨镜头沿用（carry.ts）──
  const copySources: BlockingSource[] = shotRow && episode ? blockingSources(episode.storyboard, shotRow) : [];
  const copySource = copySources.find((c) => c.shotId === copyFromId) ?? copySources[0] ?? null;
  const cast = castCheck(actors, shotRow, colorTable, propAssets);
  const copyBlocking = (src: BlockingSource) => {
    setActors(src.actors.map((a) => ({ ...a })), `copy:${Date.now()}`);
    setSelectedId(null);
    setCopyArmed(false);
    setCarried(null);
    setStatus(`复制了第 ${src.shotNum} 镜的站位（${src.actors.length} 个）—— ⌘Z 撤销`);
  };
  const clearBlocking = () => {
    setActors([], `clear:${Date.now()}`);
    setSelectedId(null);
    setCarried(null);
    setStatus("清空了 —— 这一镜从头摆（⌘Z 撤销）");
  };
  const removeActors = (ids: string[]) => {
    const drop = new Set(ids);
    setActors(latest.current.actors.filter((a) => !drop.has(a.id)), `remove:${Date.now()}`);
    if (latest.current.selectedId && drop.has(latest.current.selectedId)) setSelectedId(null);
  };
  /** 缺的角色直接放到准星（身份跟着切过去） */
  const placeIdentity = (assetId: string) => {
    identityRef.current = assetId;
    setIdentityId(assetId);
    placeActorAt(engineRef.current?.crosshairGround() ?? null);
  };

  // placeActorAt 被引擎回调闭包持有，读的值走 ref
  const mainlineRef = useRef(mainline);
  mainlineRef.current = mainline;
  const identityRef = useRef(identityId);
  identityRef.current = identityId;
  const colorTableRef = useRef(colorTable);
  colorTableRef.current = colorTable;
  const anonColorRef = useRef(anonColor);
  anonColorRef.current = anonColor;
  const newScaleRef = useRef(newScale);
  newScaleRef.current = newScale;
  const sphereRadiusRef = useRef(sphereRadius);
  sphereRadiusRef.current = sphereRadius;
  const selectedCameraIdRef = useRef(selectedCameraId);
  selectedCameraIdRef.current = selectedCameraId;
  const lensRef = useRef({ through: false, active: activeCameraId });
  lensRef.current = { through: viewMode === "camera" && throughLens, active: activeCameraId };

  const updateSelected = (fn: (a: DirectorActor) => DirectorActor, kind = "edit") => {
    const id = latest.current.selectedId;
    if (!id) return false;
    setActors((list) => list.map((a) => (a.id === id ? fn(a) : a)), `${kind}:${id}`);
    return true;
  };

  /** 删一个人。误删了 ⌘Z 撤销 —— 所以不再要求 Shift（2026-09-16 用户反馈「好难操控」） */
  const deleteActor = (id: string | null) => {
    if (!id) return;
    const who = latest.current.actors.find((a) => a.id === id);
    if (!who) return;
    setActors((list) => list.filter((a) => a.id !== id), `delete:${Date.now()}`);
    if (latest.current.selectedId === id) setSelectedId(null);
    setStatus(`已删除「${who.label}」· ⌘Z 撤销`);
  };
  const deleteSelected = () => deleteActor(latest.current.selectedId);

  const cyclePose = (dir: 1 | -1) =>
    updateSelected((a) => {
      const i = POSES.findIndex((p) => p.id === a.pose);
      return { ...a, pose: POSES[(i + dir + POSES.length) % POSES.length].id };
    });

  const cycleIdentity = () => {
    if (!colorTable.length) return;
    const i = colorTable.findIndex((c) => c.asset_id === identityId);
    const next = colorTable[(i + 1) % colorTable.length];
    setIdentityId(next.asset_id);
    setStatus(`人物预设：${next.name}`);
  };

  const applyCameraPreset = (kind: "face" | "behind" | "look") => {
    const engine = engineRef.current;
    const a = latest.current.actors.find((x) => x.id === latest.current.selectedId);
    if (!engine || !a) return;
    setShotCamera(cameraPreset(a, kind, camera));
  };

  const resetCamera = () => setShotCamera(defaultCamera(eyeHeight));

  /** 在当前视角新建一个机位（导演视角里飞到哪就建在哪），并设为当前机位 */
  /**
   * 新建机位 = 当前视角。用户 2026-09-28：「没有在当前视角创建，多次创建的机位全重叠在一起」。
   *
   * - 机位视角：复制眼前这个机位（之后转动改的是新的这个）；
   * - 导演视角 + 全景锁在拍摄点：机位**放在拍摄点**、朝导演视角现在看的方向 ——
   *   全景只在拍摄点准，建在导演相机那儿（常常在球外几十米）出的图没法用。
   *   用的是导演相机的朝向，不是绕转中心：左键绕着转时中心不动，按中心算的话怎么转都是同一个朝向；
   * - 导演视角 + 3D 片场 / 解了锁：机位放在导演相机的位置，建完导演视角往后退一步，看得见刚建的机位。
   *
   * 已经有一个一模一样的机位（位置差不到 5 厘米、角度差不到 1°）就不再叠一个，选中它。
   */
  const saveNamedCamera = () => {
    const engine = engineRef.current;
    const seen = engine?.getCamera() ?? camera;
    const inDirector = viewModeRef.current === "director";
    const atCapture = inDirector && fixedPoint;
    const base: DirectorCamera = !inDirector
      ? { ...camera, rollDeg: camera.rollDeg ?? 0 }
      : atCapture
        ? { position: [0, eyeHeight, 0], yawDeg: seen.yawDeg, pitchDeg: Math.max(-60, Math.min(60, seen.pitchDeg)), fovDeg: camera.fovDeg, rollDeg: 0 }
        : { ...seen, rollDeg: seen.rollDeg ?? 0 };
    const same = namedCameras.find(
      (c) =>
        Math.hypot(c.camera.position[0] - base.position[0], c.camera.position[1] - base.position[1], c.camera.position[2] - base.position[2]) < 0.05 &&
        Math.abs(c.camera.yawDeg - base.yawDeg) < 1 &&
        Math.abs(c.camera.pitchDeg - base.pitchDeg) < 1,
    );
    if (inDirector && same) {
      setSelectedCameraId(same.id);
      setSelectedId(null);
      setStatus(`「${same.name}」就在这个位置、朝这个方向 —— 先转一下视角再新建`);
      return;
    }
    const item: NamedCamera = { id: `camera-${Date.now()}`, name: nextCameraName(namedCameras), camera: base };
    setNamedCameras((list) => [...list, item]);
    setActiveCameraId(item.id);
    setCamera(item.camera);
    setSelectedCameraId(inDirector ? item.id : null);
    if (inDirector && !atCapture) engine?.pullBack(2.5);
    setStatus(
      !inDirector
        ? `已新建${item.name}（复制眼前这个机位）—— 现在转动画面改的是${item.name}`
        : atCapture
          ? `已新建${item.name}：全景锁在拍摄点，机位放在拍摄点、朝你现在看的方向`
          : `已新建${item.name}：就在导演视角的位置，视角往后退了一步好看见它`,
    );
  };
  const addPathKeyframe = () => {
    const current = camera;
    setCameraPaths((paths) => {
      const path = paths[0] ?? { id: `path-${Date.now()}`, name: "主运镜", durationSec: 3, cameras: [] };
      const next = { ...path, cameras: [...path.cameras, current] };
      return paths.length ? [next, ...paths.slice(1)] : [next];
    });
    setStatus("已把当前机位加入主运镜");
  };

  const renderTrajectory = async () => {
    const engine = engineRef.current;
    const canvas = canvasRef.current;
    const path = cameraPaths[0];
    if (!engine || !canvas || !path || path.cameras.length < 2 || !onTrajectoryOutput) return;
    setBusy("trajectory"); setErr(null);
    const original = engine.getCamera();
    try {
      const stream = canvas.captureStream(24);
      const mime = MediaRecorder.isTypeSupported("video/webm;codecs=vp9") ? "video/webm;codecs=vp9" : "video/webm";
      const recorder = new MediaRecorder(stream, { mimeType: mime, videoBitsPerSecond: 12_000_000 });
      const chunks: Blob[] = [];
      recorder.ondataavailable = (e) => e.data.size && chunks.push(e.data);
      const stopped = new Promise<void>((resolve) => { recorder.onstop = () => resolve(); });
      engine.setCamera(path.cameras[0]);
      const firstFrame = engine.capture("combined", frame);
      recorder.start();
      const started = performance.now();
      await new Promise<void>((resolve) => {
        const tick = (now: number) => {
          const progress = Math.min(1, (now - started) / (path.durationSec * 1000));
          const scaled = progress * (path.cameras.length - 1);
          const i = Math.min(path.cameras.length - 2, Math.floor(scaled));
          engine.setCamera(interpolateCamera(path.cameras[i], path.cameras[i + 1], scaled - i));
          if (progress < 1) requestAnimationFrame(tick); else resolve();
        };
        requestAnimationFrame(tick);
      });
      const lastFrame = engine.capture("combined", frame);
      recorder.stop(); await stopped;
      const blob = new Blob(chunks, { type: mime });
      const videoDataUrl = await new Promise<string>((resolve, reject) => { const r = new FileReader(); r.onload = () => resolve(String(r.result)); r.onerror = () => reject(r.error); r.readAsDataURL(blob); });
      await onTrajectoryOutput({ videoDataUrl, firstFrame, lastFrame, path });
      setStatus("运镜 RGB、深度与首尾帧已输出");
    } catch (e) { setErr(`轨迹输出失败：${String(e)}`); }
    finally { engine.setCamera(original); setCamera(original); setBusy(null); }
  };

  // ── 按键 ──
  useEffect(() => {
    const isTyping = (t: EventTarget | null) => {
      const el = t as HTMLElement | null;
      return !!el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT" || el.isContentEditable);
    };
    const onKeyDown = (e: KeyboardEvent) => {
      // 一律不让画布收到（画布的 ⌘Z 撤的是画布，不是这里）
      e.stopPropagation();
      if (isTyping(e.target)) return;
      if ((e.metaKey || e.ctrlKey) && !e.altKey) {
        const c = keyCode(e);
        if (c === "KeyZ") {
          applyHistory(e.shiftKey ? "redo" : "undo");
          e.preventDefault();
        } else if (c === "KeyY" && e.ctrlKey) {
          applyHistory("redo");
          e.preventDefault();
        }
        return;
      }
      if (e.altKey) return;
      const engine = engineRef.current;
      const { stageActive: active, shortcutsOpen: sc } = latest.current;
      const code = keyCode(e);
      let handled = true;
      switch (code) {
        case "Escape":
          // 一层一层退：快捷键面板 → 取消选中 → 退出交互 → 关编辑器
          if (sc) setShortcutsOpen(false);
          else if (latest.current.selectedId) setSelectedId(null);
          // 导演视角里选中的机位（挂着坐标轴）也按 Esc 放下 —— 右栏不显示它，没有「取消选中」可点
          else if (selectedCameraIdRef.current) setSelectedCameraId(null);
          else if (active) setStageActive(false);
          else close();
          break;
        case "Tab":
        case "KeyH":
          setPanelHidden((v) => !v);
          break;
        case "Slash":
          if (e.shiftKey) setShortcutsOpen((v) => !v);
          else handled = false;
          break;
        default:
          handled = false;
      }
      if (!handled && active && engine) {
        handled = true;
        switch (code) {
          case "Digit1":
          case "Numpad1":
            cycleIdentity();
            break;
          case "KeyC":
            setSelectedId(engine.pickAtCrosshair());
            break;
          case "KeyF": {
            const p = engine.crosshairGround();
            if (p) updateSelected((a) => ({ ...a, position: [p[0], a.position[1], p[2]] }));
            break;
          }
          case "KeyG":
            updateSelected((a) => ({ ...a, position: [a.position[0], groundHeight, a.position[2]] }));
            break;
          case "ArrowUp":
          case "KeyI":
            updateSelected((a) => ({ ...a, position: [a.position[0], a.position[1], round2(a.position[2] - NUDGE)] }), "nudge");
            break;
          case "ArrowDown":
          case "KeyK":
            updateSelected((a) => ({ ...a, position: [a.position[0], a.position[1], round2(a.position[2] + NUDGE)] }), "nudge");
            break;
          case "ArrowLeft":
          case "KeyJ":
            updateSelected((a) => ({ ...a, position: [round2(a.position[0] - NUDGE), a.position[1], a.position[2]] }), "nudge");
            break;
          case "ArrowRight":
          case "KeyL":
            updateSelected((a) => ({ ...a, position: [round2(a.position[0] + NUDGE), a.position[1], a.position[2]] }), "nudge");
            break;
          case "KeyU":
            updateSelected((a) => ({ ...a, position: [a.position[0], round2(a.position[1] + NUDGE), a.position[2]] }), "nudge");
            break;
          case "KeyO":
            // 和拖动一样允许低于网格，见 engine.ts 的 dragging-changed
            updateSelected((a) => ({ ...a, position: [a.position[0], round2(a.position[1] - NUDGE), a.position[2]] }), "nudge");
            break;
          case "KeyR":
            updateSelected((a) => ({ ...a, yawDeg: a.yawDeg - 15 }), "rotate");
            break;
          case "KeyT":
            updateSelected((a) => ({ ...a, yawDeg: a.yawDeg + 15 }), "rotate");
            break;
          case "Minus":
            updateSelected((a) => ({ ...a, scale: clampScale(a.scale / 1.08) }), "scale");
            break;
          case "Equal":
            updateSelected((a) => ({ ...a, scale: clampScale(a.scale * 1.08) }), "scale");
            break;
          case "Comma":
            updateSelected((a) => ({ ...a, scale: clampScale(a.scale * 0.9) }), "scale");
            break;
          case "Period":
            updateSelected((a) => ({ ...a, scale: clampScale(a.scale * 1.1) }), "scale");
            break;
          case "BracketLeft":
            cyclePose(-1);
            break;
          case "BracketRight":
            cyclePose(1);
            break;
          case "KeyP":
            setViewMode(viewModeRef.current === "camera" ? "director" : "camera");
            break;
          case "KeyB":
            applyCameraPreset("behind");
            break;
          case "KeyN":
            applyCameraPreset("face");
            break;
          case "KeyV":
            applyCameraPreset("look");
            break;
          case "Backspace":
          case "Delete":
            deleteSelected();
            break;
          default:
            handled = engine.keyDown(code, e.shiftKey);
        }
      }
      if (handled) e.preventDefault();
    };
    const onKeyUp = (e: KeyboardEvent) => {
      e.stopPropagation();
      engineRef.current?.keyUp(keyCode(e));
    };
    window.addEventListener("keydown", onKeyDown, { capture: true });
    window.addEventListener("keyup", onKeyUp, { capture: true });
    return () => {
      window.removeEventListener("keydown", onKeyDown, { capture: true });
      window.removeEventListener("keyup", onKeyUp, { capture: true });
    };
    // 读的都是 ref / setState，函数每次渲染新建但行为只依赖 ref
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [close, colorTable, identityId]);

  // ── 输出 ──
  const colorNames = useMemo(() => Object.fromEntries(ACTOR_PALETTE.map((p) => [p.hex, p.name])), []);

  const outputToCanvas = async () => {
    const engine = engineRef.current;
    if (!engine) return;
    setBusy("output");
    setErr(null);
    try {
      const cam = camera;
      const shots = withShotView((e) => ({ combined: e.capture("combined", frame), envOnly: e.capture("env_only", frame) }));
      if (!shots) return;
      const { combined, envOnly } = shots;
      const meta = buildFrameMeta({
        sourceUrl: panoSource,
        splat: splatSource,
        context: context ? { project_id: context.projectId, episode_id: context.episodeId, shot_id: context.shotId } : null,
        aspect,
        camera: cam,
        actors,
        colorNames,
      });
      saveRef.current({ actors, camera: cam, aspect, context, namedCameras, cameraPaths, activeCameraId });
      await onOutput({ combined, envOnly, meta });
      setStatus("已输出到画布节点");
    } catch (e) {
      setErr(`输出失败：${String(e)}`);
    } finally {
      setBusy(null);
    }
  };

  const blockingRows = useMemo(
    () => blockingFor(actors, camera, aspectRatio(aspect)),
    [actors, camera, aspect],
  );

  /**
   * 只存这一镜（`cinema_save_shot`）。以前这里整集保存，拿的是**打开导演世界那一刻**读的那份分集 ——
   * 导演世界开着的时候后台细化 / 合成写回了别的镜头（或者这一镜的时间段），一点「写进分镜」就被整个盖回旧的
   * （2026-09-28 查出来）。`base` 是导演世界手里那一份，后端据此保留后台写回的时间段和提示词。
   */
  const saveShotOf = async (next: EpisodeData, shotId: string) => {
    const shot = next.storyboard.find((s) => s.id === shotId);
    const base = episode?.storyboard.find((s) => s.id === shotId);
    if (!context || !shot || !base) throw new Error("那一镜不见了 —— 刷新一下再试");
    await cinemaSaveShot(context.projectId, context.episodeId, shot, base);
  };

  const writeBlocking = async () => {
    if (!context?.shotId || !episode) return;
    const rows = blockingFor(actors, camera, aspectRatio(aspect));
    const next = withBlocking(episode, context.shotId, rows);
    if (!next) {
      setErr("那一镜不见了 —— 刷新一下再试");
      return;
    }
    setBusy("blocking");
    try {
      await saveShotOf(next, context.shotId);
      setEpisode(next);
      setBlockingNote(`已写进${shotOptions(next).find((o) => o.id === context.shotId)?.label ?? "那一镜"}`);
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(null);
    }
  };
  const bindCameraToShot = async () => {
    const active = namedCameras.find((c) => c.id === activeCameraId);
    if (!context?.shotId || !episode || !active) return;
    const next = withDirectorCamera(episode, context.shotId, active, cameraPaths[0]);
    if (!next) { setErr("目标镜头不存在"); return; }
    setBusy("blocking");
    try { await saveShotOf(next, context.shotId); setEpisode(next); setStatus("机位和运镜已绑定到分镜"); }
    catch (e) { setErr(String(e)); } finally { setBusy(null); }
  };

  const pickEpisode = (value: string) => {
    setBlockingNote("");
    if (!value) {
      setContext(null);
      return;
    }
    const [projectId, episodeId] = value.split("/");
    setContext({ projectId, episodeId, shotId: null });
  };

  // ── 界面 ──
  // 布局照 libtv 3D 导演台（2026-09-27 用户：「左右两侧栏，可以处理不同的功能。左侧做了菜单栏」）：
  // 左边图标栏切面板，中间视口，右边属性栏跟着选中的东西变（选了人 = 人物属性，没选 = 世界校准）。
  // 方案：docs/plans/2026-09-27-导演台界面升级.md（U1 只搬家，功能和行为不变）
  const panelTitle = LEFT_TABS.find((t) => t.id === tab)?.label ?? "";
  return (
    <div
      className="fixed inset-0 z-[1000] grid bg-[#0b0b0d] text-[#f4f4f5]"
      style={{ gridTemplateColumns: panelHidden ? "52px minmax(0,1fr)" : "52px 272px minmax(0,1fr) 288px" }}
      onPointerDown={(e) => e.stopPropagation()}
      onWheel={(e) => e.stopPropagation()}
    >
      {/* 图标栏。顶部让出 macOS 红绿灯（约 13..72, 14..34），让出来的那条可以拖动窗口 */}
      <nav className="flex min-h-0 flex-col items-center gap-1 border-r border-white/[0.08] bg-[#141418] pb-3 pt-[46px]" data-tauri-drag-region onPointerDown={() => setStageActive(false)}>
        {LEFT_TABS.map((t) => (
          <button
            key={t.id}
            type="button"
            title={t.label}
            aria-label={t.label}
            aria-pressed={!panelHidden && tab === t.id}
            onClick={() => {
              setTab(t.id);
              setPanelHidden(false);
            }}
            className={`flex h-10 w-10 flex-col items-center justify-center gap-0.5 rounded-[10px] text-[9.5px] ${!panelHidden && tab === t.id ? "bg-white/[0.12] text-white" : "text-white/50 hover:bg-white/[0.06] hover:text-white"}`}
          >
            <t.icon className="h-4 w-4" />
            {t.label}
          </button>
        ))}
        <div className="flex-1" />
        <button
          type="button"
          onClick={close}
          aria-label="关闭"
          title="关闭（Esc）"
          className="flex h-8 w-8 items-center justify-center rounded-full bg-white/10 text-white/60 hover:bg-white/15 hover:text-white"
        >
          <X className="h-3.5 w-3.5" />
        </button>
      </nav>

      {!panelHidden && (
        <aside
          className="min-h-0 overflow-y-auto border-r border-white/[0.08] bg-[#191a1f] px-4 pb-4 text-[12px]"
          onPointerDown={() => setStageActive(false)}
        >
          <div data-tauri-drag-region className="sticky top-0 z-20 -mx-4 mb-1 bg-[#191a1f]/90 px-4 pb-2 pt-[46px] backdrop-blur-xl">
            <div className="truncate text-[15px] font-semibold text-white">导演世界 · {panelTitle}</div>
          </div>

          {tab === "stage" && (
            <>
          <Section title="片场">
            <Field label="分集（选了就是主线模式）">
              <select
                value={context ? `${context.projectId}/${context.episodeId}` : ""}
                onChange={(e) => pickEpisode(e.target.value)}
                className={selectClass}
              >
                <option value="">不选 · 自由模式</option>
                {scripts.map((s) => (
                  <option key={`${s.project_id}/${s.episode_id}`} value={`${s.project_id}/${s.episode_id}`}>
                    {s.project_title} · {s.episode_title}
                  </option>
                ))}
              </select>
            </Field>
            {context && (
              <Field label="镜头">
                <select
                  value={context.shotId ?? ""}
                  onChange={(e) => {
                    setBlockingNote("");
                    setContext({ ...context, shotId: e.target.value || null });
                  }}
                  className={selectClass}
                >
                  <option value="">不指定</option>
                  {shotOptions(episode).map((o) => (
                    <option key={o.id} value={o.id}>
                      {o.label}
                    </option>
                  ))}
                </select>
              </Field>
            )}
            <div className="grid grid-cols-2 gap-2">
              <Field label="世界来源">
                <select className={selectClass} value="pano" disabled>
                  <option value="pano">{splatSource ? `3DGS · ${splatSource.format.toUpperCase()}` : panoSource ? "360 全景图" : "无来源"}</option>
                </select>
              </Field>
              <Field label="画幅">
                <select value={aspect} onChange={(e) => setAspect(e.target.value)} className={selectClass}>
                  {FRAME_ASPECTS.map((a) => (
                    <option key={a.id} value={a.id}>
                      {a.id}
                    </option>
                  ))}
                </select>
              </Field>
            </div>
          </Section>
          <Section title="场景">
            {actors.length === 0 && namedCameras.length === 0 && <p className="text-[11.5px] leading-5 text-white/40">还没有人物和机位。</p>}
            {namedCameras.map((item) => (
              <button key={item.id} type="button" className={`${infoClass} text-left`} onClick={() => { engineRef.current?.setCamera(item.camera); setCamera(item.camera); }}>
                <span className="flex min-w-0 items-center gap-2"><Camera className="h-3.5 w-3.5 shrink-0" /><span className="truncate">{item.name}</span></span>
              </button>
            ))}
            {actors.map((a) => (
              <button key={a.id} type="button" className={`${infoClass} text-left ${a.id === selectedId ? "border-[#ffc82e]/60" : ""}`} onClick={() => { setSelectedId(a.id); setStageActive(true); }}>
                <span className="flex min-w-0 items-center gap-2"><Swatch color={a.color} /><span className="truncate">{a.label}</span></span>
                <span className="shrink-0 text-[11px] text-white/40">{kindLabel(a)}</span>
              </button>
            ))}
          </Section>
            </>
          )}

          {tab === "people" && (
            <>
          {shotRow && (
            <Section title={`第 ${shotRow.shot_num} 镜的站位`}>
              {carried !== null && (
                <p className="text-[11.5px] leading-5 text-sky-100/80">
                  沿用了第 {carried} 镜的站位，位置原样。机位没带 —— 到机位页点「按分镜摆机」。
                </p>
              )}
              {cast.absent.length > 0 && (
                <div className="flex flex-col gap-1.5 rounded-[10px] border border-amber-300/25 bg-amber-300/[0.06] p-2 text-[12px] text-amber-100/90">
                  <span>这一镜分镜里没有：{cast.absent.map((a) => `「${a.label}」`).join("")}</span>
                  <div className="flex flex-wrap gap-1.5">
                    {cast.absent.map((a) => (
                      <button key={a.id} type="button" className="rounded-md border border-white/15 px-2 py-0.5 text-[11px] text-white/80 hover:bg-white/10" onClick={() => removeActors([a.id])}>
                        删掉「{a.label}」
                      </button>
                    ))}
                    {cast.absent.length > 1 && (
                      <button type="button" className="rounded-md border border-white/15 px-2 py-0.5 text-[11px] text-white/80 hover:bg-white/10" onClick={() => removeActors(cast.absent.map((a) => a.id))}>
                        全部删掉
                      </button>
                    )}
                  </div>
                  <span className="text-[11px] text-white/45">人在屋里但不入画的话留着也行 —— 只要他不出现在取景框里。</span>
                </div>
              )}
              {(cast.missing.length > 0 || cast.missingProps.length > 0) && (
                <div className="flex flex-col gap-1.5 rounded-[10px] border border-sky-300/25 bg-sky-300/[0.06] p-2 text-[12px] text-sky-100/90">
                  <span>这一镜还要：{[...cast.missing.map((c) => c.name), ...cast.missingProps.map((p) => p.name)].map((n) => `「${n}」`).join("")}</span>
                  <div className="flex flex-wrap gap-1.5">
                    {cast.missing.map((c) => (
                      <button key={c.asset_id} type="button" className="rounded-md border border-white/15 px-2 py-0.5 text-[11px] text-white/80 hover:bg-white/10" onClick={() => placeIdentity(c.asset_id)}>
                        「{c.name}」放到准星
                      </button>
                    ))}
                    {cast.missingProps.map((p) => (
                      <button key={p.id} type="button" className="rounded-md border border-white/15 px-2 py-0.5 text-[11px] text-white/80 hover:bg-white/10" onClick={() => placePropAtCrosshair(propAssets.find((a) => a.id === p.id) ?? null)}>
                        道具「{p.name}」放到准星
                      </button>
                    ))}
                  </div>
                </div>
              )}
              {copySources.length > 0 ? (
                <>
                  <Field label="从同一场景的别的镜头复制站位">
                    <select
                      value={copySource?.shotId ?? ""}
                      onChange={(e) => {
                        setCopyFromId(e.target.value);
                        setCopyArmed(false);
                      }}
                      className={selectClass}
                    >
                      {copySources.map((c) => (
                        <option key={c.shotId} value={c.shotId}>
                          第 {c.shotNum} 镜 · {c.actors.filter(isPerson).length} 人
                          {c.actors.some((a) => a.kind === "extra") ? ` · 群演 ${c.actors.filter((a) => a.kind === "extra").length}` : ""}
                          {c.actors.some((a) => a.kind === "prop") ? ` · 道具 ${c.actors.filter((a) => a.kind === "prop").length}` : ""}
                        </option>
                      ))}
                    </select>
                  </Field>
                  <OutlineButton
                    disabled={!copySource}
                    onClick={() => {
                      if (!copySource) return;
                      if (actors.length && !copyArmed) {
                        setCopyArmed(true);
                        return;
                      }
                      copyBlocking(copySource);
                    }}
                  >
                    {copyArmed ? `确认替换场上 ${actors.length} 个（⌘Z 能撤）` : `复制第 ${copySource?.shotNum ?? ""} 镜的站位`}
                  </OutlineButton>
                </>
              ) : (
                <p className="text-[11px] leading-5 text-white/40">
                  {shotRow.scene_id
                    ? "同一个场景的别的镜头还没摆过人。摆好这一镜，同场景的后面几镜打开时会自动沿用。"
                    : "这一镜没有场景资产，不能沿用 / 复制站位（站位是片场里的坐标，没有片场就没有「同一个地方」）。"}
                </p>
              )}
            </Section>
          )}
          <Section title="添加人物">
            {mainline ? (
              <>
                <Field label="人物身份（本集颜色表）">
                  <select
                    value={identityId}
                    onChange={(e) => setIdentityId(e.target.value)}
                    disabled={!colorTable.length}
                    className={selectClass}
                  >
                    {colorTable.map((c) => (
                      <option key={c.asset_id} value={c.asset_id}>
                        {c.name}
                        {actors.some((a) => a.assetId === c.asset_id) ? "（已在场）" : ""}
                      </option>
                    ))}
                  </select>
                </Field>
                {identity && (
                  <div className={infoClass}>
                    <span className="truncate">
                      {identity.name} · {identity.color_name}
                    </span>
                    <Swatch color={identity.color} />
                  </div>
                )}
                {!colorTable.length && (
                  <p className="text-[12px] leading-5 text-white/54">
                    本集分镜里还没有能出镜的角色，颜色表是空的 —— 先在分镜里用 [角色:名字] 标出人物。
                  </p>
                )}
                {unassigned.length > 0 && (
                  <p className="text-[12px] leading-5 text-amber-200/80">
                    调色盘不够用，这些角色没分到颜色：{unassigned.join("、")}
                  </p>
                )}
              </>
            ) : (
              <>
                <div className={infoClass}>
                  <span>
                    下一个人物：{nextAnonymousLabel(actors)} · {anonColor}
                  </span>
                  <Swatch color={anonColor} />
                </div>
                <Field label="人物颜色">
                  <div className="flex flex-wrap gap-2">
                    {ACTOR_PALETTE.map((p) => (
                      <button
                        key={p.hex}
                        type="button"
                        title={p.name}
                        onClick={() => setAnonColor(p.hex)}
                        className="h-6 w-6 rounded-full border-2"
                        style={{
                          backgroundColor: p.hex,
                          borderColor: anonColor === p.hex ? "#fff" : "transparent",
                        }}
                      />
                    ))}
                  </div>
                </Field>
              </>
            )}
            <Range label={`缩放 ${newScale.toFixed(2)}`} min={0.4} max={2.4} step={0.05} value={newScale} onChange={setNewScale} />
            <PrimaryButton
              disabled={mainline && !identity}
              onClick={() => placeActorAt(engineRef.current?.crosshairGround() ?? null)}
            >
              把人物放到准星
            </PrimaryButton>
            <p className="text-[11px] leading-5 text-white/40">也可以在画面里右键 / Shift+左键点地面放置。</p>
          </Section>
          <Section title="道具">
            <Field label="道具资产">
              <select value={propId} onChange={(e) => setPropId(e.target.value)} className={selectClass}>
                {propAssets.map((a) => (
                  <option key={a.id} value={a.id}>
                    {a.name}
                    {a.cover_url ? "" : "（没图）"} · {a.scope_label}
                  </option>
                ))}
                <option value="">不绑资产（方块占位）</option>
              </select>
            </Field>
            <OutlineButton onClick={() => placePropAtCrosshair(propAssets.find((a) => a.id === propId) ?? null)}>把道具放到准星</OutlineButton>
            <p className="text-[11px] leading-5 text-white/40">
              有图的道具是一块立牌（资产封面），没图是一个方块。出首帧时按资产的图换成真东西；缩放滑块调大小。
            </p>
          </Section>
          <Section title="群演">
            <OutlineButton onClick={placeCrowdAtCrosshair}>在准星铺一片群演（3×3）</OutlineButton>
            <p className="text-[11px] leading-5 text-white/40">
              灰色人偶，面向镜头、稍微错开。出首帧时统一换成各不相同的路人，不逐个点名；选中单个可以挪、删。
            </p>
          </Section>
          <Section title={`场上 · 人物 ${actors.filter(isPerson).length} · 群演 ${actors.filter((a) => a.kind === "extra").length} · 道具 ${actors.filter((a) => a.kind === "prop").length}`}>
            {actors.length === 0 ? (
              <p className="text-[11.5px] leading-5 text-white/40">还没有人。右键 / Shift+左键点地面放一个。</p>
            ) : (
              <ul className="flex flex-col gap-1">
                {actors.map((a) => (
                  <li
                    key={a.id}
                    className={`group flex items-center gap-2 rounded-[10px] border px-2 py-1.5 ${
                      a.id === selectedId ? "border-[#ffc82e]/60 bg-[#ffc82e]/[0.08]" : "border-transparent hover:bg-white/[0.04]"
                    }`}
                  >
                    <button
                      type="button"
                      className="flex min-w-0 flex-1 items-center gap-2 text-left"
                      onClick={() => {
                        setSelectedId(a.id);
                        setStageActive(true);
                      }}
                      title="选中"
                    >
                      <Swatch color={a.color} />
                      <span className="truncate text-[12px] text-white/85">{a.label}</span>
                      <span className="shrink-0 text-[11px] text-white/40">{kindLabel(a)}</span>
                    </button>
                    <button
                      type="button"
                      className="flex h-6 w-6 shrink-0 items-center justify-center rounded-md text-white/35 hover:bg-red-500/15 hover:text-red-200"
                      onClick={() => deleteActor(a.id)}
                      title={`删除「${a.label}」（⌘Z 撤销）`}
                      aria-label={`删除 ${a.label}`}
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </Section>
            </>
          )}

          {tab === "cameras" && (
            <>
          <Section title="机位">
            <div className="grid grid-cols-2 gap-2">
              <OutlineButton onClick={saveNamedCamera} title="按眼前的视角新建一个机位。导演视角里、全景锁在拍摄点时，机位放在拍摄点、朝你现在看的方向">新建机位</OutlineButton>
              <OutlineButton onClick={resetCamera}>
                <RotateCcw className="h-3.5 w-3.5" /> 重置镜头
              </OutlineButton>
            </div>
            {namedCameras.map((item) => (
              <div key={item.id} className="flex items-center gap-2">
                <button
                  type="button"
                  className={`${infoClass} min-w-0 flex-1 text-left ${item.id === selectedCameraId ? "border-white/40" : ""}`}
                  title="切到机位视角、透过这个机位看（它成为当前机位：输出、写进分镜、运镜都按它）"
                  onClick={() => {
                    if (viewModeRef.current !== "camera") setViewMode("camera");
                    lookThrough(item.id);
                  }}
                >
                  <span className="flex min-w-0 items-center gap-2">
                    <Camera className="h-3.5 w-3.5 shrink-0" style={{ color: item.id === activeCameraId ? "#ffc82e" : undefined }} />
                    <span className="truncate">{item.name}</span>
                  </span>
                  <span className="shrink-0 text-[11px] text-white/40">{item.id === activeCameraId ? "当前" : `FOV ${Math.round(item.camera.fovDeg)}°`}</span>
                </button>
                <button type="button" className="text-white/35 hover:text-red-200" title="删除机位" onClick={() => deleteNamedCamera(item.id)}>×</button>
              </div>
            ))}
            <p className="text-[11px] leading-5 text-white/40">
              黄色是当前机位：输出、写进分镜、运镜、绑定分镜都按它。导演视角（P）里能看到每个机位、点名字改属性。
            </p>
            {shotRow && (
              <div className="flex flex-col gap-1.5 rounded-[12px] border border-[#ffc82e]/25 bg-[#ffc82e]/[0.05] p-2.5">
                <div className="text-[12px] font-medium text-white/85">按分镜摆机 · 第 {shotRow.shot_num} 镜</div>
                <div className="text-[11.5px] leading-5 text-white/55">
                  {display("shotSize", shotRow.scale) || "未定景别"} · {display("angle", shotRow.camera_angle) || "平视"} · {display("move", shotRow.camera_movement) || "固定"} · {shotRow.duration} 秒
                  {actors.length ? ` → 对准 ${framingTarget().target?.label ?? ""}` : ""}
                </div>
                <PrimaryButton disabled={!actors.length} onClick={frameFromShot}>
                  <Camera className="h-3.5 w-3.5" /> 按分镜摆机
                </PrimaryButton>
                <p className="text-[11px] leading-5 text-white/40">
                  {actors.length ? "按这一镜的景别和机位摆好「第 N 镜」机位；推拉摇移升降会顺手出好运镜关键帧（时长 = 这一镜秒数）。对准说话的人，选中了人就对准选中的。" : "先在人物页把这一镜的人摆上。"}
                </p>
              </div>
            )}
            <Field label={`预设机位 · 对准 ${framingTarget().target?.label ?? "（先摆人）"}`}>
              <div className="mt-1.5 grid grid-cols-3 gap-1.5">
                {CAMERA_PRESETS.map((p) => (
                  <button
                    key={p.label}
                    type="button"
                    disabled={!actors.length}
                    onClick={() => addPresetCamera(p)}
                    className="flex h-12 flex-col items-center justify-center gap-0.5 rounded-[10px] border border-white/[0.10] bg-[#242426]/70 text-[11px] text-white/70 hover:border-white/25 hover:text-white disabled:opacity-40"
                  >
                    <Camera className="h-3.5 w-3.5" />
                    {p.label}
                  </button>
                ))}
              </div>
            </Field>
            <Field label="对准选中人物">
              <div className="mt-1.5 grid grid-cols-3 gap-2">
                <OutlineButton disabled={!selected} onClick={() => applyCameraPreset("face")}>正面 N</OutlineButton>
                <OutlineButton disabled={!selected} onClick={() => applyCameraPreset("behind")}>背后 B</OutlineButton>
                <OutlineButton disabled={!selected} onClick={() => applyCameraPreset("look")}>看向 V</OutlineButton>
              </div>
            </Field>
            <label className="flex items-start gap-2 rounded-[10px] border border-white/[0.08] bg-black/20 px-3 py-2 text-[11.5px] leading-5 text-white/60">
              <input
                type="checkbox"
                className="mt-1"
                checked={cameraLocked}
                onChange={(e) => setCameraLocked(e.target.checked)}
              />
              <span>
                新机位锁在拍摄点（推荐）
                <span className="block text-[11px] text-white/38">
                  {splatSource
                    ? "有 3D 片场时默认不锁，新建和预设机位可以有真实位移；单张图生成的片场离原拍摄点越远破绽越多"
                    : "开启后，新建和预设机位留在全景拍摄点，只改变朝向和 FOV；外部观察相机始终可以自由绕转。"}
                </span>
              </span>
            </label>
          </Section>
            </>
          )}

          {tab === "motion" && (
            <>
          <Section title="运镜">
            <OutlineButton onClick={addPathKeyframe}>加入运镜关键帧（当前机位）</OutlineButton>
            <p className="text-[11px] leading-5 text-white/40">
              主运镜已有 {cameraPaths[0]?.cameras.length ?? 0} 个关键帧。机位、位置、俯仰和视场角均写入节点，可绑定分镜并用于首尾帧或轨迹渲染。
            </p>
            <PrimaryButton disabled={busy !== null || (cameraPaths[0]?.cameras.length ?? 0) < 2} onClick={() => void renderTrajectory()}>
              {busy === "trajectory" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Camera className="h-3.5 w-3.5" />}
              输出运镜 RGB、深度和首尾帧
            </PrimaryButton>
            <OutlineButton disabled={!context?.shotId || !episode || !namedCameras.length || busy !== null} onClick={() => void bindCameraToShot()}>
              将当前机位与主运镜绑定到当前分镜
            </OutlineButton>
          </Section>
            </>
          )}

          {tab === "output" && (
            <>
          <Section title="输出">
            {problems.length > 0 && (
              <ul className="flex list-disc flex-col gap-1 rounded-[12px] border border-amber-300/25 bg-amber-400/[0.06] py-2 pl-7 pr-3 text-[12px] leading-5 text-amber-100/85">
                {problems.map((p) => (
                  <li key={p}>{p}</li>
                ))}
              </ul>
            )}
            <PrimaryButton disabled={busy !== null || problems.length > 0} onClick={() => void outputToCanvas()}>
              {busy === "output" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Camera className="h-3.5 w-3.5" />}
              输出人偶稿（合成图 + 纯背景）
            </PrimaryButton>
            <p className="text-[11px] leading-5 text-white/40">
              合成图保留人物遮挡；纯背景隐藏人物。每个人偶的颜色、身份、姿势、画面位置一起带出去。
            </p>
            {context?.shotId && episode ? (
              <>
                <div className={`${infoClass} leading-5`}>{blockingSentence(blockingRows)}</div>
                <OutlineButton disabled={busy !== null || problems.length > 0} onClick={() => void writeBlocking()}>
                  {busy === "blocking" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : null}
                  写进分镜
                </OutlineButton>
                <p className="text-[11px] leading-5 text-white/40">
                  {blockingNote || overwriteNote(episode, context.shotId, blockingRows)}
                </p>
              </>
            ) : (
              <p className="text-[11px] leading-5 text-white/40">选了分集和镜头，就能把站位写进分镜。</p>
            )}
          </Section>
            </>
          )}
        </aside>
      )}

      <main
        ref={viewportRef}
        className="relative min-h-0 overflow-hidden bg-black"
        onPointerDown={() => setStageActive(true)}
      >
        <canvas ref={canvasRef} className="block h-full w-full" />

        {/* 视角切换（照 libtv）。P 键同 */}
        <div className="absolute left-1/2 top-3 z-20 flex -translate-x-1/2 items-center gap-0.5 rounded-full border border-white/[0.12] bg-black/60 p-0.5 text-[12px] backdrop-blur-md">
          {([["director", "导演视角"], ["camera", "机位视角"]] as const).map(([m, label]) => (
            <button
              key={m}
              type="button"
              aria-pressed={viewMode === m}
              onClick={() => setViewMode(m)}
              className={`rounded-full px-3 py-1 ${viewMode === m ? "bg-white/[0.16] text-white" : "text-white/55 hover:text-white"}`}
            >
              {label}
            </button>
          ))}
          {viewMode === "camera" && (
            <span className="px-2 text-[11px] text-white/45">{namedCameras.find((c) => c.id === activeCameraId)?.name}</span>
          )}
        </div>
        {viewMode === "director" && selectedCameraId && !selectedId && (
          <div
            className="absolute left-1/2 top-12 z-20 flex -translate-x-1/2 items-center gap-0.5 rounded-full border border-white/[0.12] bg-black/60 p-0.5 text-[11.5px] backdrop-blur-md"
            onPointerDown={(e) => e.stopPropagation()}
          >
            <span className="px-2 text-white/50">{namedCameras.find((c) => c.id === selectedCameraId)?.name}</span>
            {([["translate", "移动"], ["rotate", "旋转"]] as const).map(([m, label]) => (
              <button
                key={m}
                type="button"
                aria-pressed={cameraGizmoMode === m}
                onClick={() => setCameraGizmoMode(m)}
                title={m === "rotate" ? "拖三个色环转机位：绿 = 水平、红 = 俯仰、蓝 = 横滚" : "拖三色箭头挪机位"}
                className={`rounded-full px-3 py-0.5 ${cameraGizmoMode === m ? "bg-white/[0.16] text-white" : "text-white/55 hover:text-white"}`}
              >
                {label}
              </button>
            ))}
          </div>
        )}

        {/* 外部观察方向器：跟观察相机实时旋转，点轴切正视图；照 libtv 放在视口右上角。 */}
          <div className="absolute right-4 top-4 z-20 flex flex-col items-center gap-1.5" onPointerDown={(e) => e.stopPropagation()}>
            <div className="relative h-[68px] w-[68px] rounded-full border border-white/[0.1] bg-[#17191f]/85 shadow-lg backdrop-blur-md" aria-label="三维方向器">
              <span className="absolute left-1/2 top-1/2 h-2 w-2 -translate-x-1/2 -translate-y-1/2 rounded-full bg-white/35" />
              {(["x", "y", "z"] as const).map((axis) => (
                <div key={`line-${axis}`} ref={(el) => { if (el) axisLineRefs.current.set(axis, el); else axisLineRefs.current.delete(axis); }} className={`absolute left-1/2 top-1/2 h-px origin-left ${axis === "x" ? "bg-[#ff6470]" : axis === "y" ? "bg-[#52dfad]" : "bg-[#5f8cff]"}`} />
              ))}
              {(["x", "y", "z"] as const).map((axis) => (
                <button
                  key={`-${axis}`}
                  type="button"
                  ref={(el) => { if (el) axisDotRefs.current.set(`-${axis}`, el); else axisDotRefs.current.delete(`-${axis}`); }}
                  title={`沿负 ${axis.toUpperCase()} 轴查看`}
                  aria-label={`沿负 ${axis.toUpperCase()} 轴查看`}
                  onClick={() => engineRef.current?.snapDirectorView(`-${axis}` as "-x" | "-y" | "-z")}
                  className="absolute left-1/2 top-1/2 h-2.5 w-2.5 rounded-full border border-white/10 bg-white/35 shadow"
                />
              ))}
              {(["x", "y", "z"] as const).map((axis) => (
                <button
                  key={axis}
                  type="button"
                  ref={(el) => { if (el) axisDotRefs.current.set(axis, el); else axisDotRefs.current.delete(axis); }}
                  title={`沿 ${axis.toUpperCase()} 轴查看`}
                  aria-label={`沿 ${axis.toUpperCase()} 轴查看`}
                  onClick={() => engineRef.current?.snapDirectorView(`+${axis}` as "+x" | "+y" | "+z")}
                  className={`absolute left-1/2 top-1/2 flex h-5 w-5 items-center justify-center rounded-full text-[9px] font-bold text-white shadow ${axis === "x" ? "bg-[#f05261]" : axis === "y" ? "bg-[#32c995]" : "bg-[#3974f6]"}`}
                >
                  {axis.toUpperCase()}
                </button>
              ))}
            </div>
            <button
              type="button"
              // 两个视角的重置不一样：导演视角回到 XYZ 标准视角；机位视角回到当前机位自己的视角
              title={viewMode === "camera" ? "回到当前机位的视角" : "回到标准视角（看全片场）"}
              onClick={() => (viewMode === "camera" ? lookThrough(activeCameraId) : engineRef.current?.resetDirectorView())}
              className="rounded-md border border-white/[0.1] bg-[#24262d]/90 px-2.5 py-1 text-[11px] text-white/70 shadow backdrop-blur-md hover:bg-white/[0.14] hover:text-white">
              重置视角
            </button>
          </div>

        {/* 外部片场视口：人物和机位标签都可选，机位属性在右栏编辑 */}
          <div className="pointer-events-none absolute inset-0 z-[7]">
            {actors.map((a) => (
              <button
                key={`actor-${a.id}`}
                type="button"
                ref={(el) => {
                  if (el) actorTagRefs.current.set(a.id, el);
                  else actorTagRefs.current.delete(a.id);
                }}
                onPointerDown={(e) => e.stopPropagation()}
                onClick={() => { setSelectedCameraId(null); setSelectedId(a.id); }}
                className={`pointer-events-auto absolute left-0 top-0 whitespace-nowrap rounded-full border px-2 py-0.5 text-[11px] font-medium backdrop-blur-md ${a.id === selectedId ? "border-[#ffc82e]/70 bg-black/65 text-[#ffe8a6]" : "border-white/25 bg-black/55 text-white/80"}`}
                style={{ opacity: 0 }}
              >
                {a.label}
              </button>
            ))}
            {namedCameras.map((c) => (
              <button
                key={c.id}
                type="button"
                ref={(el) => {
                  if (el) cameraTagRefs.current.set(c.id, el);
                  else cameraTagRefs.current.delete(c.id);
                }}
                onPointerDown={(e) => e.stopPropagation()}
                onClick={() => {
                  if (viewMode === "camera") {
                    lookThrough(c.id);
                    return;
                  }
                  setSelectedId(null);
                  setSelectedCameraId(c.id);
                }}
                onDoubleClick={() => {
                  setViewMode("camera");
                  lookThrough(c.id);
                }}
                title={viewMode === "camera" ? "透过这个机位看" : "点一下选中（挂坐标轴）· 双击透过它看"}
                className={`pointer-events-auto absolute left-0 top-0 flex items-center gap-1 whitespace-nowrap rounded-full border px-2 py-0.5 text-[11px] font-medium backdrop-blur-md ${
                  c.id === activeCameraId ? "border-[#ffc82e]/70 bg-black/65 text-[#ffe8a6]" : c.id === selectedCameraId ? "border-white/70 bg-black/65 text-white" : "border-sky-300/40 bg-black/55 text-sky-100/85"
                }`}
                style={{ opacity: 0 }}
              >
                <Camera className="h-3 w-3" /> {c.name}
              </button>
            ))}
          </div>

        {carried !== null && (
          <div
            className="absolute left-1/2 top-14 z-10 flex max-w-[calc(100%-32px)] -translate-x-1/2 items-center gap-2 rounded-full border border-sky-300/30 bg-black/70 px-3 py-1.5 text-[12px] text-sky-100/90 backdrop-blur-md"
            title="同一个场景里最近摆过的那一镜，人、群演、道具的位置原样带过来；机位不带 —— 到机位页点「按分镜摆机」"
          >
            <span className="truncate">沿用了第 {carried} 镜的站位</span>
            <button type="button" className="shrink-0 whitespace-nowrap rounded-full border border-white/15 px-2 py-0.5 text-[11px] text-white/80 hover:bg-white/10" onClick={clearBlocking}>
              清空重摆
            </button>
            <button type="button" className="shrink-0 text-white/45 hover:text-white" onClick={() => setCarried(null)} aria-label="知道了" title="知道了">
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
        )}

        {/* 准星 */}
        <div className="pointer-events-none absolute left-1/2 top-1/2 z-[5] h-[18px] w-[18px] -translate-x-1/2 -translate-y-1/2">
          <span className="absolute left-2 top-0 h-[18px] w-[2px] bg-white/85 shadow-[0_0_8px_rgba(0,0,0,0.75)]" />
          <span className="absolute left-0 top-2 h-[2px] w-[18px] bg-white/85 shadow-[0_0_8px_rgba(0,0,0,0.75)]" />
        </div>

        {/* 选中标签 */}
        {selected && (
          <div
            ref={badgeRef}
            className="pointer-events-none absolute left-0 top-0 z-[6] whitespace-nowrap rounded-full border border-[#ffc82e]/60 bg-black/60 px-2.5 py-1 text-[11px] font-medium text-[#ffe8a6]"
            style={{ opacity: 0 }}
          >
            {KIND_TITLE[selected.kind ?? "person"]} · {selected.label} · ({selected.position.map((v) => v.toFixed(2)).join(", ")})
          </div>
        )}

        <div className="pointer-events-none absolute bottom-6 left-6 z-10 flex h-6 items-center gap-2 rounded-[8px] bg-black/45 px-2.5 text-[12px] text-white/64 backdrop-blur-md">
          <span>{stageActive ? "导演世界交互中 · Esc 退出" : "点击画面进入导演世界"}</span>
          <span>|</span>
          <span>{panelHidden ? "Tab 回到默认状态" : "Tab 收起侧栏"}</span>
        </div>

        {/* 右下角的状态卡要让开它（见 studio/Toasts.tsx） */}
        <div data-toast-avoid="" className="absolute bottom-6 right-6 z-10 flex items-center gap-2">
          <button type="button" onClick={cycleIdentity} className={glassButton}>
            <span className="font-semibold">1 · 人物</span>
            <span className="max-w-[120px] truncate text-white/58">
              {mainline ? identity?.name ?? `${colorTable.length} 个可选` : nextAnonymousLabel(actors)}
            </span>
          </button>
          <button type="button" onClick={() => setShortcutsOpen((v) => !v)} className={glassButton}>
            <Keyboard className="h-3.5 w-3.5" /> 快捷键
          </button>
        </div>

        {shortcutsOpen && (
          <div className="absolute right-4 top-4 z-20 w-[360px] max-w-[calc(100%-2rem)] rounded-2xl border border-white/15 bg-[#141416]/95 p-4 text-xs text-white/70 shadow-2xl backdrop-blur-xl">
            <div className="mb-3 flex items-center justify-between">
              <div>
                <div className="text-sm font-semibold text-white">导演世界快捷键</div>
                <div className="mt-1 text-[11px] text-white/40">先点击画面进入导演世界，再用快捷键。</div>
              </div>
              <button type="button" className="rounded-full border border-white/15 px-2 py-1 text-[11px]" onClick={() => setShortcutsOpen(false)}>
                关闭
              </button>
            </div>
            <div className="grid gap-2">
              {SHORTCUTS.map(([keys, label]) => (
                <div key={keys} className="grid grid-cols-[132px_minmax(0,1fr)] gap-3 rounded-lg border border-white/10 bg-white/[0.035] px-3 py-2">
                  <div className="font-mono text-[11px] font-semibold text-white">{keys}</div>
                  <div className="leading-5">{label}</div>
                </div>
              ))}
            </div>
          </div>
        )}
      </main>

      {!panelHidden && (
        <aside
          className="min-h-0 overflow-y-auto border-l border-white/[0.08] bg-[#191a1f] px-4 pb-4 text-[12px]"
          onPointerDown={() => setStageActive(false)}
        >
          <div data-tauri-drag-region className="sticky top-0 z-20 -mx-4 mb-1 bg-[#191a1f]/90 px-4 pb-2 pt-[46px] backdrop-blur-xl">
            <div className="truncate text-[15px] font-semibold text-white">{selected ? KIND_TITLE[selected.kind ?? "person"] : selectedCam ? "摄像机" : "3D 场景"}</div>
          </div>
          {selected ? (
            <>
          <Section title={selected ? `选中 · ${selected.label}` : "选中"}>
            <button type="button" className="self-start text-[11px] text-white/45 hover:text-white" onClick={() => setSelectedId(null)}>
              取消选中 · 回到片场设置（Esc）
            </button>
            <div className={infoClass}>
              {selected ? (
                <span className="truncate">
                  {KIND_TITLE[selected.kind ?? "person"]} · {selected.label} · ({selected.position.map((v) => v.toFixed(2)).join(", ")}) · {kindLabel(selected)}
                </span>
              ) : (
                <span>未选中</span>
              )}
            </div>
            {mainline && selected && isPerson(selected) && (
              <Field label="身份">
                <select
                  value={selected?.assetId ?? ""}
                  disabled={!selected}
                  onChange={(e) => {
                    const who = colorTable.find((c) => c.asset_id === e.target.value);
                    if (!selected || !who) return;
                    const r = bindIdentity(actors, selected.id, who);
                    if ("error" in r) setErr(r.error);
                    else {
                      setErr(null);
                      setActors(r.actors, `bind:${Date.now()}`);
                    }
                  }}
                  className={selectClass}
                >
                  <option value="" disabled>
                    未绑定
                  </option>
                  {colorTable.map((c) => (
                    <option key={c.asset_id} value={c.asset_id}>
                      {c.name}
                    </option>
                  ))}
                </select>
              </Field>
            )}
            {selected?.kind !== "prop" && (
            <Field label="人物姿势">
              <select
                value={selected?.pose ?? "standing"}
                disabled={!selected}
                onChange={(e) => updateSelected((a) => ({ ...a, pose: e.target.value as PoseName }))}
                className={selectClass}
              >
                {POSES.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.label}
                  </option>
                ))}
              </select>
            </Field>
            )}
            <Range
              label={`缩放 ${(selected?.scale ?? 1).toFixed(2)}`}
              min={selected?.kind === "prop" ? 0.1 : 0.4}
              max={selected?.kind === "prop" ? 6 : 2.4}
              step={0.05}
              value={selected?.scale ?? 1}
              disabled={!selected}
              onChange={(v) => updateSelected((a) => ({ ...a, scale: v }), "scale-slider")}
            />
            <div className="grid grid-cols-3 gap-2">
              <OutlineButton disabled={!selected} onClick={() => updateSelected((a) => ({ ...a, yawDeg: a.yawDeg - 15 }))}>
                <RotateCcw className="h-3.5 w-3.5" /> 左转
              </OutlineButton>
              <OutlineButton disabled={!selected} onClick={() => updateSelected((a) => ({ ...a, yawDeg: a.yawDeg + 15 }))}>
                <RotateCw className="h-3.5 w-3.5" /> 右转
              </OutlineButton>
              <OutlineButton
                disabled={!selected}
                onClick={() => updateSelected((a) => ({ ...a, position: [a.position[0], groundHeight, a.position[2]] }))}
              >
                落地
              </OutlineButton>
            </div>
            <button
              type="button"
              disabled={!selected}
              onClick={deleteSelected}
              className="flex h-9 w-full items-center justify-center gap-1.5 rounded-[10px] border border-red-400/20 bg-red-500/[0.07] text-[12px] text-red-200/75 hover:border-red-300/30 hover:bg-red-500/[0.11] hover:text-red-100 disabled:opacity-40"
            >
              <Trash2 className="h-3.5 w-3.5" /> 删除选中
            </button>
          </Section>
            </>
          ) : selectedCam ? (
            <CameraInspector
              cam={selectedCam}
              active={selectedCam.id === activeCameraId}
              preview={cameraPreview}
              previewAspect={aspectRatio(aspect)}
              cameras={namedCameras}
              actors={actors}
              onSelectCamera={(id) => lookThrough(id)}
              onChange={(patch) => updateNamedCamera(selectedCam.id, patch)}
              onActivate={() => activateCamera(selectedCam.id)}
              onDelete={() => deleteNamedCamera(selectedCam.id)}
              onDeselect={() => viewMode === "camera" ? setViewMode("director") : setSelectedCameraId(null)}
            />
          ) : (
            <>
          <Section title="世界校准">
            <p className="text-[11px] leading-[1.7] text-white/42">
              全景的地板就是网格所在的地面，一格 1 米。<b className="font-medium text-white/60">眼高就是尺度</b>：
              调到地砖、门高、桌子和 1.83 米的人偶比例对得上为止；地平线歪了调俯仰 / 倾斜。
            </p>
            <Range label={`眼高 ${eyeHeight.toFixed(2)} 米`} min={0.3} max={4} step={0.05} value={eyeHeight} onChange={(v) => onCalibrationChange({ eyeHeight: v })} />
            <Range label={`地面高度 ${groundHeight.toFixed(2)} 米`} min={-4} max={4} step={0.05} value={groundHeight} onChange={(v) => onCalibrationChange({ groundHeight: v })} />
            {panoSource && (
              <>
                <Range
                  label={`全景球半径 ${sphereRadius} 米（导演视角）`}
                  min={SPHERE_RADIUS_RANGE[0]}
                  max={SPHERE_RADIUS_RANGE[1]}
                  step={1}
                  value={sphereRadius}
                  onChange={setSphereRadius}
                />
                <p className="text-[11px] leading-[1.7] text-white/42">
                  两个页签都从外部显示完整全景球；机位的真实 3DGS 取景显示在右侧预览，正式输出也按它。网格所在平面就是真实地面。
                </p>
              </>
            )}
            {splatSource && (
              <>
                <Range
                  label={`片场缩放 ${Math.round((splatSource.scale ?? 1) * 100)}%`}
                  min={0.25}
                  max={4}
                  step={0.01}
                  value={splatSource.scale ?? 1}
                  onChange={(v) => onCalibrationChange({ splatScale: v })}
                />
                <p className="text-[11px] leading-[1.7] text-white/42">
                  3D 片场的单位只是「大约米」。先在台子 / 门旁边放一个人（1.83 米），拖片场缩放到比例对；再拖眼高让网格贴住地板。
                  缩放以拍摄点为中心，机位视角里画面不变。
                </p>
              </>
            )}
            <Range label={`水平旋转 ${Math.round(frontYawDeg)}°`} min={-180} max={180} step={1} value={frontYawDeg} onChange={(v) => onCalibrationChange({ frontYawDeg: v })} />
            <Range label={`俯仰 ${correction.pitch.toFixed(1)}°`} min={-30} max={30} step={0.5} value={correction.pitch} onChange={(v) => onCalibrationChange({ correction: { ...correction, pitch: v } })} />
            <Range label={`倾斜 ${correction.roll.toFixed(1)}°`} min={-30} max={30} step={0.5} value={correction.roll} onChange={(v) => onCalibrationChange({ correction: { ...correction, roll: v } })} />
            <OutlineButton
              onClick={() => onCalibrationChange({ correction: { roll: 0, pitch: 0, yaw: 0 }, frontYawDeg: 0, eyeHeight: 1.6, groundHeight: 0 })}
            >
              <RotateCcw className="h-3.5 w-3.5" /> 重置世界校准
            </OutlineButton>
          </Section>
          <Section title="场景辅助">
            {splatSource && panoSource && <SwitchRow label="全景球" hint="导演视角用稳定的完整球，避免高斯离开训练视点后畸变" checked={panoramaProxy} onChange={setPanoramaProxy} />}
            <SwitchRow label="角色标签" checked={actorLabels} onChange={setActorLabels} />
            <SwitchRow label="网格吸附" hint="开着：人偶按格子走，落在 1 米网格的交叉点上；关着：随便拖。只管水平位置，拖绿箭头调高度不受影响" checked={gridSnap} onChange={setGridSnap} />
            {splatSource && <SwitchRow label="高斯地面吸附" hint="开着：人偶只能待在地面上（贴 3D 片场的地表），不能上下拖；关着：能上天入地。和网格吸附一起开 = 先按格子定位置，再贴地" checked={gaussianSnap} onChange={setGaussianSnap} />}
            <SwitchRow label="地面" hint="网格平面就是角色落脚与校准使用的真实地面" checked={groundVisible} onChange={setGroundVisible} />
            {groundVisible && <Range label={`透明度 ${groundOpacity.toFixed(2)}`} min={0.05} max={1} step={0.05} value={groundOpacity} onChange={setGroundOpacity} />}
          </Section>
            </>
          )}

          <div className="mt-3 rounded-[12px] border border-white/[0.08] bg-black/20 px-3 py-2 text-[12px] leading-5 text-white/54">
            <div>
              {actors.filter(isPerson).length} 人物
              {actors.some((a) => a.kind === "extra") && ` · ${actors.filter((a) => a.kind === "extra").length} 群演`}
              {actors.some((a) => a.kind === "prop") && ` · ${actors.filter((a) => a.kind === "prop").length} 道具`} · {namedCameras.length} 机位
            </div>
            {status && <div className="truncate">{status}</div>}
            {err && <div className="mt-1 text-red-300">{err}</div>}
          </div>
        </aside>
      )}
    </div>
  );
}

/**
 * 快捷键说明。**每一行都要真的接好** —— 测试对着上面的 switch 逐条核对
 * （`DirectorWorldEditor.test.ts`），和画布快捷键面板同一个规矩。
 */
export const SHORTCUTS: [string, string][] = [
  ["左键拖 / 右键拖", "环绕观察 / 平移观察中心"],
  ["P", "切换导演视角 / 机位视角"],
  ["滚轮", "整体缩放片场（两个页签一致）"],
  ["W A S D · Q E", "平移观察中心 · 升降（Shift 加速）"],
  ["右键 / Shift+左键", "在地面放置人物"],
  ["1", "切换要放置的人物"],
  ["C / F / G", "选准星指着的人 / 移到准星 / 落地"],
  ["↑ ↓ ← → / I J K L / U O", "微调选中人物的位置"],
  ["R / T · - / = · [ ]", "左右转 · 缩放 · 切换姿势"],
  ["B / N / V", "镜头切到背后 / 正面 / 看向选中人物"],
  ["Delete", "删除选中人物"],
  ["⌘Z / ⇧⌘Z", "撤销 / 重做（摆人、挪动、删除、姿势都能撤）"],
  ["Tab / Esc", "收起侧栏 / 取消选中（再按退出交互）"],
];

/**
 * 物理键位。`event.code` 为空时（远程桌面、部分输入法、自动化工具）按 `event.key` 推回来，
 * 否则这些环境下所有快捷键都没反应，而且不报错。
 */
export function keyCode(e: { code: string; key: string }): string {
  if (e.code) return e.code;
  const k = e.key;
  if (/^[a-zA-Z]$/.test(k)) return `Key${k.toUpperCase()}`;
  if (/^[0-9]$/.test(k)) return `Digit${k}`;
  const map: Record<string, string> = {
    "-": "Minus", "_": "Minus", "=": "Equal", "+": "Equal", "[": "BracketLeft", "{": "BracketLeft",
    "]": "BracketRight", "}": "BracketRight", ",": "Comma", "<": "Comma", ".": "Period", ">": "Period",
    "/": "Slash", "?": "Slash", " ": "Space",
  };
  return map[k] ?? k; // Escape / Tab / Delete / Backspace / ArrowUp… 的 key 和 code 同名
}

function round2(v: number) {
  return Math.round(v * 100) / 100;
}

function clampScale(v: number) {
  return Math.max(0.4, Math.min(2.4, Math.round(v * 100) / 100));
}

const selectClass =
  "mt-1.5 h-9 w-full rounded-[10px] border border-white/[0.10] bg-black/25 px-2.5 text-[12px] text-white outline-none focus:border-white/25 disabled:opacity-50";
const infoClass =
  "flex items-center justify-between gap-2 rounded-[12px] border border-white/[0.08] bg-black/20 px-3 py-2 text-[12px] text-white/58";
const glassButton =
  "inline-flex h-6 items-center gap-1 whitespace-nowrap rounded-[8px] bg-black/45 px-2.5 text-[12px] text-white/64 backdrop-blur-md hover:bg-black/60 hover:text-white";

/**
 * 右栏「摄像机」：机位视角显示当前机位，导演视角显示选中机位（照 libtv）。
 * 位置 / 角度直接改数字；注视目标选一个人，那人挪了机位自动重新对准。
 */
function CameraInspector({
  cam,
  active,
  preview,
  previewAspect,
  cameras,
  actors,
  onSelectCamera,
  onChange,
  onActivate,
  onDelete,
  onDeselect,
}: {
  cam: NamedCamera;
  active: boolean;
  preview: string | null;
  /** 画幅宽高比：预览框跟它一样 */
  previewAspect: number;
  cameras: NamedCamera[];
  actors: DirectorActor[];
  onSelectCamera: (id: string) => void;
  onChange: (patch: Partial<NamedCamera>) => void;
  onActivate: () => void;
  onDelete: () => void;
  onDeselect: () => void;
}) {
  const c = cam.camera;
  const setCam = (p: Partial<DirectorCamera>) => onChange({ camera: { ...c, ...p } });
  const num = (v: string, fb: number) => (Number.isFinite(Number(v)) && v.trim() !== "" ? Number(v) : fb);
  const axis = ["X", "Y", "Z"] as const;
  // 操作杆每帧回调一次：读最新的位置，不用渲染那一刻闭包里的
  const latestCam = useRef(c);
  latestCam.current = c;
  const onMove = (p: [number, number, number]) => onChange({ camera: { ...latestCam.current, position: p } });
  return (
    <Section title={`摄像机 · ${cam.name}${active ? "（当前）" : ""}`}>
      <button type="button" className="self-start text-[11px] text-white/45 hover:text-white" onClick={onDeselect}>
        取消选中 · 回到片场设置
      </button>
      <div
        className="relative mx-auto overflow-hidden rounded-[10px] border border-white/[0.08] bg-black/40"
        style={{ aspectRatio: String(previewAspect), width: previewAspect >= 1 ? "100%" : `${Math.round(previewAspect * 100)}%`, maxHeight: 220 }}
      >
        {preview ? <img src={preview} alt={cam.name} className="h-full w-full object-contain" /> : <div className="flex h-full items-center justify-center text-[11px] text-white/35">预览生成中…</div>}
        <span className="absolute left-2 top-1.5 rounded-full bg-black/55 px-2 py-0.5 text-[10.5px] text-white/70">FOV {Math.round(c.fovDeg)}°</span>
      </div>
      <Field label="名称">
        <input className={selectClass} value={cam.name} onChange={(e) => onChange({ name: e.target.value })} />
      </Field>
      <Field label="切换机位">
        <select className={selectClass} value={cam.id} onChange={(e) => onSelectCamera(e.target.value)}>
          {cameras.map((item) => <option key={item.id} value={item.id}>{item.name}</option>)}
        </select>
      </Field>
      {/* 不用 Field（那是 <label>）：点操作杆会被 label 转给第一个输入框 */}
      <div className="text-[12px] font-medium text-white/56">
        位置（米）
        <div className="mt-1.5 grid grid-cols-3 gap-1.5">
          {axis.map((a, i) => (
            <div key={a} className="flex min-w-0 flex-col gap-1">
              <label className="flex items-center gap-1 rounded-[8px] border border-white/[0.10] bg-black/25 px-2 text-[11px]" style={{ color: AXIS_COLOR[a] }}>
                {a}
                <input
                  type="number"
                  step={0.05}
                  className="h-8 w-full min-w-0 bg-transparent text-[12px] text-white outline-none"
                  value={Math.round(c.position[i] * 100) / 100}
                  onChange={(e) => {
                    const p = [...c.position] as [number, number, number];
                    p[i] = num(e.target.value, p[i]);
                    setCam({ position: p });
                  }}
                />
              </label>
              <JogLever
                color={AXIS_COLOR[a]}
                title={`拖着往右 = ${a} 变大、往左 = 变小；拖得越远走得越快，松手停`}
                onJog={(d) => {
                  const p = [...latestCam.current.position] as [number, number, number];
                  p[i] = Math.round((p[i] + d) * 1000) / 1000;
                  onMove(p);
                }}
              />
            </div>
          ))}
        </div>
        <p className="mt-1 text-[10.5px] leading-[1.6] text-white/35">
          滑杆往哪边拖就往哪边挪，拖得越远越快，松手停；也可以在导演视角里点机位图标，拖它的三色坐标轴。
        </p>
      </div>
      <div className="text-[12px] font-medium text-white/56">
        机位颜色
        <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
          {CAMERA_COLORS.map((hex) => (
            <button
              key={hex}
              type="button"
              title={hex}
              aria-label={`机位颜色 ${hex}`}
              onClick={() => onChange({ color: hex })}
              className="h-6 w-6 rounded-full border-2"
              style={{ backgroundColor: hex, borderColor: cameraColor(cam.color) === hex ? "#fff" : "transparent" }}
            />
          ))}
          <input
            type="color"
            title="自定义颜色"
            aria-label="自定义机位颜色"
            value={cameraColor(cam.color).toLowerCase()}
            onChange={(e) => onChange({ color: e.target.value.toUpperCase() })}
            className="h-6 w-8 cursor-pointer rounded border border-white/15 bg-transparent"
          />
        </div>
      </div>
      <Field label="角度（水平 / 俯仰 / 横滚）">
        <div className="mt-1.5 grid grid-cols-3 gap-1.5">
          {([["yawDeg", c.yawDeg], ["pitchDeg", c.pitchDeg], ["rollDeg", c.rollDeg ?? 0]] as const).map(([k, v]) => (
            <input
              key={k}
              type="number"
              step={1}
              className="h-8 w-full min-w-0 rounded-[8px] border border-white/[0.10] bg-black/25 px-2 text-[12px] text-white outline-none"
              value={Math.round(v * 10) / 10}
              title={k === "rollDeg" ? "横滚：荷兰角，正 = 画面顺时针歪" : k === "pitchDeg" ? "俯仰：正 = 抬头" : "水平：正 = 向右转"}
              onChange={(e) => {
                const n = num(e.target.value, v);
                // 手动改角度就不再自动对准人
                onChange({ lookAt: null, camera: { ...c, [k]: k === "pitchDeg" ? Math.max(-89, Math.min(89, n)) : k === "rollDeg" ? Math.max(-45, Math.min(45, n)) : n } });
              }}
            />
          ))}
        </div>
      </Field>
      <Range label={`视野角度 FOV ${Math.round(c.fovDeg)}°`} min={15} max={110} step={1} value={c.fovDeg} onChange={(v) => setCam({ fovDeg: v })} />
      <Field label="注视目标">
        <select
          className={selectClass}
          value={cam.lookAt ?? ""}
          onChange={(e) => {
            const id = e.target.value || null;
            const a = actors.find((x) => x.id === id);
            onChange(a ? { lookAt: id, camera: { ...c, ...lookAtAngles(c.position, [a.position[0], a.position[1] + 1.0 * a.scale, a.position[2]]) } } : { lookAt: null });
          }}
        >
          <option value="">不注视（手动角度）</option>
          {actors.filter((a) => a.kind !== "extra").map((a) => (
            <option key={a.id} value={a.id}>
              {a.label}
            </option>
          ))}
        </select>
      </Field>
      <div className="grid grid-cols-1 gap-2">
        <OutlineButton disabled={active} onClick={onActivate}>设为当前机位</OutlineButton>
      </div>
      <button
        type="button"
        onClick={onDelete}
        className="flex h-8 w-full items-center justify-center gap-1.5 rounded-[10px] border border-red-400/20 bg-red-500/[0.07] text-[12px] text-red-200/75 hover:border-red-300/30 hover:text-red-100"
      >
        <Trash2 className="h-3.5 w-3.5" /> 删除机位
      </button>
    </Section>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="mt-3 flex flex-col gap-2 rounded-[14px] border border-white/[0.08] bg-[#1d1e24]/80 p-3">
      <header className="text-[12px] font-semibold text-white/80">{title}</header>
      {children}
    </section>
  );
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="block text-[12px] font-medium text-white/56">
      {label}
      {children}
    </label>
  );
}

/** 坐标轴的颜色（和场景里 X 红 / Y 绿 / Z 蓝的轴一致） */
const AXIS_COLOR = { X: "#F87171", Y: "#4ADE80", Z: "#60A5FA" } as const;
/** 操作杆拉满时每秒挪多少米。按偏移量平方算：轻轻拨一下是厘米级，拉满是每秒几米 */
const JOG_MAX_SPEED = 3;

/**
 * 回弹式操作杆（用户 2026-09-28：「给 X，Y，Z 增加可滑动操作杆，方便摄像机位置的调节」）。
 *
 * 不做成普通滑杆：片场几十米，一条 80 像素的滑杆要么调不细、要么走不远。操作杆按住往一边拖，
 * **离中间越远走得越快**（速度 ∝ 偏移²），松手回中间、停。
 */
function JogLever({ onJog, color, title }: { onJog: (delta: number) => void; color: string; title: string }) {
  const track = useRef<HTMLDivElement>(null);
  const [offset, setOffset] = useState(0);
  const off = useRef(0);
  const raf = useRef(0);
  const last = useRef(0);
  const jog = useRef(onJog);
  jog.current = onJog;
  const measure = (clientX: number) => {
    const r = track.current?.getBoundingClientRect();
    if (!r || r.width <= 0) return 0;
    return Math.max(-1, Math.min(1, ((clientX - r.left) / r.width) * 2 - 1));
  };
  const tick = (t: number) => {
    const dt = Math.min(0.05, (t - last.current) / 1000);
    last.current = t;
    const o = off.current;
    if (o !== 0) jog.current(Math.sign(o) * o * o * JOG_MAX_SPEED * dt);
    raf.current = requestAnimationFrame(tick);
  };
  const stop = () => {
    cancelAnimationFrame(raf.current);
    raf.current = 0;
    off.current = 0;
    setOffset(0);
  };
  useEffect(() => () => cancelAnimationFrame(raf.current), []);
  return (
    <div
      ref={track}
      role="slider"
      aria-label={title}
      aria-valuemin={-1}
      aria-valuemax={1}
      aria-valuenow={Math.round(offset * 100) / 100}
      title={title}
      className="relative h-5 cursor-ew-resize touch-none select-none rounded-full border border-white/[0.10] bg-black/30"
      onPointerDown={(e) => {
        e.preventDefault();
        // 抓不住指针（非真实指针、窗口失焦）也照样能拨
        try {
          e.currentTarget.setPointerCapture(e.pointerId);
        } catch {
          // 没抓住就靠 pointerleave 以外的 up / cancel 停
        }
        off.current = measure(e.clientX);
        setOffset(off.current);
        last.current = performance.now();
        if (!raf.current) raf.current = requestAnimationFrame(tick);
      }}
      onPointerMove={(e) => {
        if (!raf.current) return;
        off.current = measure(e.clientX);
        setOffset(off.current);
      }}
      onPointerUp={stop}
      onPointerCancel={stop}
      onLostPointerCapture={stop}
    >
      <span className="absolute left-1/2 top-1/2 h-2.5 w-px -translate-x-1/2 -translate-y-1/2 bg-white/25" />
      <span
        className="absolute top-1/2 h-3.5 w-3.5 -translate-x-1/2 -translate-y-1/2 rounded-full border border-white/70 shadow"
        style={{ left: `${50 + offset * 45}%`, backgroundColor: color, transition: offset === 0 ? "left 120ms ease-out" : "none" }}
      />
    </div>
  );
}

function Range(props: { label: string; min: number; max: number; step: number; value: number; disabled?: boolean; onChange: (v: number) => void }) {
  return (
    <label className="block text-[12px] text-white/56">
      {props.label}
      <input
        type="range"
        min={props.min}
        max={props.max}
        step={props.step}
        value={props.value}
        disabled={props.disabled}
        onChange={(e) => props.onChange(Number(e.target.value))}
        className="mt-1.5 w-full accent-sky-400 disabled:opacity-40"
      />
    </label>
  );
}

function SwitchRow({ label, hint, checked, onChange }: { label: string; hint?: string; checked: boolean; onChange: (value: boolean) => void }) {
  return (
    <label className="flex cursor-pointer items-center justify-between gap-3 rounded-[10px] border border-white/[0.07] bg-black/15 px-3 py-2">
      <span className="min-w-0">
        <span className="block text-[12px] text-white/80">{label}</span>
        {hint && <span className="mt-0.5 block text-[10.5px] leading-4 text-white/38">{hint}</span>}
      </span>
      <input type="checkbox" className="h-4 w-4 shrink-0 accent-sky-400" checked={checked} onChange={(e) => onChange(e.target.checked)} />
    </label>
  );
}

const SPHERE_RADIUS_KEY = "anima.director.sphereRadius";

const KIND_TITLE: Record<NonNullable<DirectorActor["kind"]>, string> = { person: "人物", extra: "群演", prop: "道具" };

/** 列表里名字后面那个小字：人物写姿势，群演写「群演 · 姿势」，道具写有没有图 */
function kindLabel(a: DirectorActor): string {
  if (a.kind === "prop") return a.imagePath ? "道具 · 立牌" : "道具 · 方块";
  if (a.kind === "extra") return `群演 · ${poseLabel(a.pose)}`;
  return poseLabel(a.pose);
}

function Swatch({ color }: { color: string }) {
  return <span className="h-4 w-4 shrink-0 rounded-full border border-white/20" style={{ backgroundColor: color }} />;
}

function PrimaryButton({ onClick, disabled, children }: { onClick: () => void; disabled?: boolean; children: ReactNode }) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className="flex h-9 w-full items-center justify-center gap-1.5 rounded-[10px] bg-white text-[12px] font-medium text-black hover:bg-white/90 disabled:opacity-40"
    >
      {children}
    </button>
  );
}

// **不折行**：按钮是固定高度的，折成两行会顶出边框（2026-09-28 用户截图「新建机位（当前视角）」）。放不下显示省略号，全文在 title 里
function OutlineButton({ onClick, disabled, title, children }: { onClick: () => void; disabled?: boolean; title?: string; children: ReactNode }) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      title={title}
      className="flex h-8 w-full min-w-0 items-center justify-center gap-1.5 overflow-hidden whitespace-nowrap rounded-[10px] border border-white/[0.10] bg-[#242426]/70 px-2 text-[12px] text-white/70 hover:border-white/20 hover:text-white disabled:opacity-40"
    >
      {children}
    </button>
  );
}
