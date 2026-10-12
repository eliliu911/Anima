/**
 * 导演世界的 three 引擎：全景球 + 地面 + 人偶 + 相机操作 + 选中 / 拖动 + 截图。
 *
 * 改写自 DramaClaw（ClaymoreLab, Elastic License 2.0）
 * frontend/src/features/viewer-kit/three-d/engine/viewerApp.ts（loadPano、placeMarker、
 * selection marker、TranslateGizmo、captureScreenshot 的四档 renderMode）和
 * engine/flyCamera.ts（左键看、右键平移、滚轮推拉、WASD/QE 飞）。
 * DC 用 PlayCanvas，这里换成 three。出处规则见 DECISIONS.md 2026-09-16。
 *
 * # 数据归 React，引擎只负责画
 *
 * 人偶列表的唯一真相在编辑器的 state 里，引擎每次 `setActors` 做一次对账
 * （新增 / 更新 / 删除）。引擎自己改了什么（拖动 gizmo、放置、选中、相机）
 * 一律通过回调报上去，由 React 决定要不要写回来。反过来的话（引擎持有人偶、
 * React 去读），撤销、存盘、侧栏显示会各有一份，迟早对不上。
 *
 * # 世界的约定（和 `scene.ts` 一致）
 *
 * - 单位米，地面 y = 0，相机默认在 (0, 眼高, 0) 看向 -Z
 * - 全景球球心在 (0, 眼高, 0)；「正前方」偏移转的是球，不是相机
 * - 人偶 yaw 0 正脸朝 +Z
 */
import * as THREE from "three";
import { TransformControls } from "three/examples/jsm/controls/TransformControls.js";
import { createCameraMarker, type CameraMarkerObject } from "./cameraMarker";
import type { SphereCorrectionDeg } from "../pano/panoStage";
import { DEFAULT_SPHERE_RADIUS, DOME_RADIUS, createInfiniteGrid, createPanoDome } from "./world";
import { createMannequin, loadMannequinLibrary, type Mannequin } from "./mannequins";
import { createPropObject } from "./props";
import { applyCamera, defaultCamera, splatPlacement, type DirectorActor, type DirectorCamera, type Rect } from "./scene";

export type CaptureMode = "combined" | "env_only" | "actor_overlay_black" | "actor_mask";

export interface DirectorEngineOptions {
  canvas: HTMLCanvasElement;
  onSelect?: (id: string | null) => void;
  /** 拖 gizmo 松手时报位置 */
  onActorMoved?: (id: string, position: [number, number, number]) => void;
  /** 右键 / Shift+左键点在地面上 */
  onPlace?: (point: [number, number, number]) => void;
  /** 相机停下来之后报一次（拖完、滚完、键盘松开） */
  onCameraChange?: (camera: DirectorCamera) => void;
  onStatus?: (message: string) => void;
  onError?: (message: string) => void;
  /** 道具立牌的图：本地路径 → 能给 `<img>` 用的地址。没给就一律画方块 */
  loadImage?: (path: string) => Promise<string>;
  /** 点中了场景里的机位图标 */
  onSelectCamera?: (id: string) => void;
  /** 拖机位的三维坐标轴（移动或旋转），松手时报它现在的位置和朝向 */
  onCameraPose?: (id: string, pose: Pick<DirectorCamera, "position" | "yawDeg" | "pitchDeg" | "rollDeg">) => void;
}

export interface ActorLabel {
  id: string;
  /** 头顶在画布里的 CSS 像素位置 */
  x: number;
  y: number;
  visible: boolean;
}

export interface OrientationAxis {
  id: "x" | "y" | "z";
  x: number;
  y: number;
  depth: number;
}

export interface CameraMarker {
  id: string;
  camera: DirectorCamera;
  /** 画幅宽高比 */
  aspect: number;
  active: boolean;
  /** 选中的机位挂三维坐标轴（没选中人偶的时候） */
  selected: boolean;
  /** 机位图标和取景框的颜色 */
  color?: string;
}

export interface DirectorEngine {
  /** 人偶库加载完 */
  ready: Promise<void>;
  setPano(textureUrl: string | null): void;
  /** 载入一个 3D Gaussian Splat 世界；存在时会隐藏全景球。 */
  setSplat(source: { url: string; format: "ply" | "ksplat" | "spz"; convention?: "opencv_camera"; scale?: number } | null): Promise<void>;
  /** 片场缩放（以拍摄点为中心）。载入之后拖滑块用，不重新载入 */
  setSplatScale(scale: number): void;
  /** 导演视角里全景球的半径（米）。只影响导演视角怎么看，机位视角和输出不变 */
  setSphereRadius(r: number): void;
  /** 选中机位时坐标轴是移动还是旋转 */
  setCameraGizmoMode(mode: "translate" | "rotate"): void;
  /**
   * 视口放到这个机位上（透过它看），之后照常能绕、能拖、能缩放 —— 绕转中心在镜头前 `pivot` 米，
   * 不在几十米外（重置导演视角之后绕转半径很大，从机位出发绕会一下甩出去）。**不改机位本身**
   */
  viewFrom(c: DirectorCamera, pivot?: number): void;
  /** 3DGS 的导演视角用全景球作稳定代理，避免离开训练视点后高斯被拉花。 */
  setPanoramaProxy(on: boolean): void;
  setGridVisible(on: boolean): void;
  setGridOpacity(opacity: number): void;
  setGroundHeight(y: number): void;
  setGridSnap(on: boolean): void;
  setGaussianGroundSnap(on: boolean): void;
  /** 右上角三维方向器使用的相机空间轴端点（-1..1）。 */
  orientationAxes(): OrientationAxis[];
  /** 从世界轴方向看向当前绕转中心。 */
  snapDirectorView(axis: "+x" | "-x" | "+y" | "-y" | "+z" | "-z"): void;
  /** 恢复能完整看见全景球 / 片场的默认导演视角。 */
  resetDirectorView(): void;
  /** 导演视角往后退 `meters` 米（绕转中心不动）。新建机位之后用：机位就建在眼睛这里，不退一步看不见它 */
  pullBack(meters: number): void;
  setCorrection(c: SphereCorrectionDeg): void;
  setFrontYaw(deg: number): void;
  setEyeHeight(h: number): void;
  setActors(actors: DirectorActor[]): void;
  setSelected(id: string | null): void;
  getCamera(): DirectorCamera;
  setCamera(c: DirectorCamera): void;
  /** 临时用指定机位执行截图 / 预览，不改变导演观察相机及绕转中心。 */
  withCamera<T>(c: DirectorCamera, fn: () => T): T;
  /**
   * 键盘飞行（WASD/QE）。**引擎不自己监听 window** —— 编辑器打开时要把按键整个
   * 截下来不让画布收到（画布的 Delete、V、Tab、⌘Z 都有自己的意思），
   * 所以由编辑器统一接管再转给这里。返回 true = 这个键引擎用掉了。
   */
  keyDown(code: string, shift: boolean): boolean;
  /**
   * 相机位置锁在拍摄点 (0, 眼高, 0)。锁着时只能转和改视场角：滚轮调视场角，
   * WASD / 右键平移不动。见 world.ts —— 360 全景只有地面是准的，
   * 相机一离开拍摄点，墙和家具就拉花。
   */
  setCameraLocked(on: boolean): void;
  keyUp(code: string): void;
  /** 准星（画面中心）指着的地面点 */
  crosshairGround(): [number, number, number] | null;
  /** 准星指着的人偶 */
  pickAtCrosshair(): string | null;
  labels(): ActorLabel[];
  /**
   * 导演视角里画出来的机位（取景框线）。在编辑器专用层里，截图时自动隐藏。
   * 当前机位黄、选中的白、其余蓝（界面升级 U2）。
   */
  setCameraMarkers(markers: CameraMarker[]): void;
  /**
   * 操作方式。`fly` = 机位视角（原来的：拖动转头、滚轮调视场角 / 推拉、WASD 飞）；
   * `orbit` = 导演视角（照 libtv：左键绕中心转、**滚轮整体缩放**、右键平移中心、不限范围，
   * 拉远能看到整个全景球 / 整个 3D 片场）。2026-09-27 用户：「libtv 滚轮缩放是整体缩放的，这是对的。
   * 我们的缩放是把水平基准线锁住了，场景图还是那个」。
   */
  setNavMode(mode: "fly" | "orbit"): void;
  /** 世界坐标 → 画布里的 CSS 像素（给机位名字标签用） */
  project(point: [number, number, number]): { x: number; y: number; visible: boolean };
  /**
   * 截 `rectCss`（画幅框，CSS 像素）范围内的一帧，长边固定 `longEdge` 像素。
   *
   * **分辨率不能跟着窗口走** —— 直接读画布像素的话，小窗口、非 Retina 屏上
   * 截出来只有 600 多像素宽（验证台实测 672×378），拿去当出图的控制图不够用。
   */
  capture(mode: CaptureMode, rectCss: Rect, longEdge?: number): string;
  resize(width: number, height: number): void;
  dispose(): void;
}

const LOOK_SENSITIVITY = 0.3; // 度 / 像素，和 DC 的 orbitSensitivity 一致
const FLY_SPEED = 2.4; // 米 / 秒
const CLICK_SLOP = 4; // 像素：按下到抬起移动不超过这么多算点击

export function createDirectorEngine(opts: DirectorEngineOptions): DirectorEngine {
  const { canvas } = opts;
  const renderer = new THREE.WebGLRenderer({
    canvas,
    antialias: true,
    // 截图要在同一帧里读像素，不保留的话读到的是空的
    preserveDrawingBuffer: true,
  });
  renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
  renderer.setClearColor(0x000000, 1);

  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(70, 16 / 9, 0.05, 2000);
  let cam: DirectorCamera = defaultCamera();

  // ── 世界：地面投影的全景（见 world.ts：眼高就是尺度，全景地板 = y=0 地面）──
  let eyeHeight = cam.position[1];
  const dome = createPanoDome(eyeHeight);
  dome.setShape("sphere");
  scene.add(dome.mesh);
  let panoTexture: THREE.Texture | null = null;
  let correction: SphereCorrectionDeg = { roll: 0, pitch: 0, yaw: 0 };
  let frontYaw = 0;

  // ── 3DGS 世界（按需加载，避免普通全景用户承担大包初始化成本）──
  type SplatViewer = {
    addSplatScene(path: string, options: Record<string, unknown>): Promise<unknown>;
    removeSplatScenes(indexes: number[], showLoadingUI?: boolean): Promise<unknown>;
    getSceneCount(): number;
    getSplatScene(index: number): { position: THREE.Vector3; scale: THREE.Vector3 };
    splatMesh: THREE.Object3D & { visible: boolean };
    raycaster: {
      ray: { origin: THREE.Vector3; direction: THREE.Vector3 };
      setFromCameraAndScreenPosition(camera: THREE.Camera, screenPosition: THREE.Vector2, screenDimensions: THREE.Vector2): void;
      intersectSplatMesh(mesh: THREE.Object3D, hits: Array<{ origin: THREE.Vector3; distance?: number }>): void;
    };
    update(): void;
    render(): void;
    dispose(): Promise<void>;
  };
  let splatViewer: SplatViewer | null = null;
  let splatLoadToken = 0;
  /** 当前载入的 splat 的坐标系；眼高改了要跟着挪（见 setEyeHeight） */
  let splatConvention: "opencv_camera" | undefined;
  let splatScale = 1;
  let hasSplat = false;
  let panoramaProxy = true;
  let groundHeight = 0;
  let gridSnap = true;
  let gaussianGroundSnap = false;
  let sphereRadius = DEFAULT_SPHERE_RADIUS;

  async function ensureSplatViewer(): Promise<SplatViewer> {
    if (splatViewer) return splatViewer;
    const mod = await import("@mkkellogg/gaussian-splats-3d");
    splatViewer = new mod.Viewer({
      selfDrivenMode: false,
      renderer,
      camera,
      threeScene: scene,
      useBuiltInControls: false,
      sharedMemoryForWorkers: false,
      // **动态场景**：载入之后还能挪片场（眼高滑块要把片场和相机一起抬，见 setEyeHeight）。
      // 静态模式下位置在载入时就烘进每个高斯里，之后改 position 不生效 —— 2026-09-27 用户把眼高拖到 4 米，
      // 只有相机上去了、片场没动，从 2.4 米高处俯视一张放大的桌子，网格怎么也对不上地面。
      dynamicScene: true,
      // **必须关。** 2026-09-27 本机实测（M4 Max / ANGLE Metal）：开着 GPU 排序，
      // 117 万个高斯全部画了（一次 draw call、234 万三角形、没有 GL 报错），画面却是全黑。
      // 关掉走 worker 里的 CPU 排序就正常 —— 09-16 冒烟验过的也是这条（18.8 MB 缓冲转移 ✅）。
      // 另：库必须锁 0.4.6，0.4.7 在同样条件下连库自带的查看器都是黑的（package.json 里写死了版本）。
      gpuAcceleratedSort: false,
      halfPrecisionCovariancesOnGPU: true,
      integerBasedSort: true,
    }) as unknown as SplatViewer;
    return splatViewer;
  }

  async function clearSplat(viewer = splatViewer) {
    if (!viewer) return;
    const count = viewer.getSceneCount();
    if (count > 0) await viewer.removeSplatScenes(Array.from({ length: count }, (_, i) => i), false);
  }

  // ── 无穷网格 + 坐标轴（截图时隐藏）──
  const editorOnly = new THREE.Group();
  const grid = createInfiniteGrid();
  editorOnly.add(grid.group);
  scene.add(editorOnly);

  function syncWorldVisibility() {
    const proxy = hasSplat && navMode === "orbit" && panoramaProxy && !!panoTexture;
    dome.mesh.visible = !hasSplat || proxy;
    if (splatViewer?.splatMesh) splatViewer.splatMesh.visible = hasSplat && !proxy;
  }

  // ── 人偶 ──
  const actorsGroup = new THREE.Group();
  scene.add(actorsGroup);
  const mannequins = new Map<string, { m: Mannequin; data: DirectorActor }>();
  let pendingActors: DirectorActor[] = [];
  let library: Awaited<ReturnType<typeof loadMannequinLibrary>> | null = null;

  // ── 离地圈：人偶离开地面时，脚下地面上一个同色的圈（照 libtv）──
  const groundRings = new Map<string, THREE.Mesh>();
  const ringGeometry = new THREE.RingGeometry(0.34, 0.42, 48).rotateX(-Math.PI / 2);
  /** 离地超过这么多（米）才画 */
  const RING_MIN_LIFT = 0.05;
  function syncGroundRings() {
    for (const [id, ring] of groundRings) {
      if (mannequins.has(id)) continue;
      editorOnly.remove(ring);
      (ring.material as THREE.Material).dispose();
      groundRings.delete(id);
    }
    for (const [id, { m, data }] of mannequins) {
      const p = m.root.position;
      let ring = groundRings.get(id);
      if (!ring) {
        ring = new THREE.Mesh(ringGeometry, new THREE.MeshBasicMaterial({ transparent: true, opacity: 0.85, depthWrite: false, side: THREE.DoubleSide }));
        ring.renderOrder = 5;
        groundRings.set(id, ring);
        editorOnly.add(ring);
      }
      (ring.material as THREE.MeshBasicMaterial).color.set(data.color);
      ring.position.set(p.x, groundHeight + 0.01, p.z);
      ring.scale.setScalar(Math.max(0.6, data.scale));
      ring.visible = Math.abs(p.y - groundHeight) > RING_MIN_LIFT;
    }
  }

  // ── 选中框 + gizmo ──
  const selectionBox = new THREE.Box3Helper(new THREE.Box3(), new THREE.Color(0xffc82e));
  selectionBox.visible = false;
  editorOnly.add(selectionBox);
  let selectedId: string | null = null;

  const gizmo = new TransformControls(camera, canvas);
  gizmo.setMode("translate");
  gizmo.setSpace("world");
  gizmo.setSize(0.9);
  const gizmoHelper = gizmo.getHelper();
  scene.add(gizmoHelper);
  let gizmoDragging = false;
  /** 这一次拖用的是哪根轴（按下时记下；松手时 `gizmo.axis` 可能已经被悬停改掉） */
  let dragAxis: string | null = null;
  gizmo.addEventListener("dragging-changed", (e) => {
    gizmoDragging = Boolean((e as unknown as { value: boolean }).value);
    if (gizmoDragging) dragAxis = gizmo.axis;
    // 拖的是机位：松手报位置，不吸网格、不落地（机位本来就在空中）
    const camMarker = [...markers.values()].find((m) => m.pivot === gizmo.object);
    if (!gizmoDragging && camMarker) {
      const p = camMarker.pivot.position;
      // pivot 的欧拉角顺序就是 YXZ（和 applyCamera 一样）：x = 俯仰，-y = 水平，z = 横滚
      const r = camMarker.pivot.rotation;
      const deg = (v: number) => Math.round(((v * 180) / Math.PI) * 10) / 10;
      opts.onCameraPose?.(camMarker.id, {
        position: [round2(p.x), round2(p.y), round2(p.z)],
        yawDeg: deg(-r.y),
        pitchDeg: Math.max(-89, Math.min(89, deg(r.x))),
        rollDeg: Math.max(-45, Math.min(45, deg(r.z))),
      });
      return;
    }
    if (!gizmoDragging && selectedId) {
      const entry = mannequins.get(selectedId);
      if (entry) {
        const p = entry.m.root.position;
        // 网格吸附在拖的时候就做了（objectChange，只吸被拖的水平轴）；松手不再把 x/z 一起取整 ——
        // 以前只拖高度，x 也会从 0.6 跳到 1（用户 2026-09-28：「高斯地面吸附和网格吸附都有开关，但是没有关联」）
        // **高度（照 libtv）**（用户 2026-09-28：「为什么人偶一拖动就会被水平线吸回来。libtv 里是可以上天入地的」
        // 「高斯地面吸附就是只能在水平线上」）：
        // - 「高斯地面吸附」关着：拖到哪就是哪，上天入地都行 —— v0.42.79 在这里不管开关、每次松手都把 y 设回地面；
        // - 开着：人偶只能待在地面上 —— 绿箭头隐藏（`syncGizmo`），松手贴 3D 片场的地表，找不到地表就落在网格地面上。
        // 人偶离地时脚下画一个圈（`syncGroundRings`），看得出悬空多高、落点在哪。回到地面按 G。
        if (gaussianGroundSnap) p.y = splatGroundAt(p.x, p.z)?.[1] ?? groundHeight;
        const pos: [number, number, number] = [round2(p.x), round2(p.y), round2(p.z)];
        entry.data = { ...entry.data, position: pos };
        opts.onActorMoved?.(selectedId, pos);
      }
    }
  });
  gizmo.addEventListener("objectChange", () => {
    // **网格吸附（照 libtv）：开着的时候人偶只能按格子走，落在网格交叉点上；关着随便拖。**
    // 用户 2026-09-28：「网格吸附是让人偶等内容吸附在网格交叉点」「不吸附则可以自动移动，吸附只能按格子移动」。
    // 水平方向一拖（X、Z、平面），x 和 z 一起吸到最近的交叉点（拖着就看得到，不是松手才跳）；
    // 只拖绿色 Y 箭头不动水平位置 —— 高度不吸，能上天入地；高度归「高斯地面吸附」管（松手时贴地表，见上面）
    const entry = selectedId ? mannequins.get(selectedId) : null;
    if (gridSnap && entry && gizmo.object === entry.m.root && dragAxis && dragAxis !== "Y") {
      const p = entry.m.root.position;
      p.x = Math.round(p.x);
      p.z = Math.round(p.z);
    }
    updateSelectionBox();
    syncGroundRings();
  });

  function round2(v: number) {
    return Math.round(v * 100) / 100;
  }

  function updateSelectionBox() {
    const entry = selectedId ? mannequins.get(selectedId) : null;
    if (!entry) {
      selectionBox.visible = false;
      return;
    }
    const b = entry.m.bounds();
    b.expandByScalar(0.06);
    selectionBox.box.copy(b);
    selectionBox.visible = true;
  }

  function applyActor(m: Mannequin, a: DirectorActor, prev?: DirectorActor) {
    m.root.position.set(...a.position);
    m.root.rotation.set(0, (a.yawDeg * Math.PI) / 180, 0);
    m.root.scale.setScalar(a.scale);
    if (!prev || prev.color !== a.color) m.setColor(a.color);
    if (!prev || prev.pose !== a.pose) m.setPose(a.pose);
  }

  function reconcile(actors: DirectorActor[]) {
    if (!library) return;
    const keep = new Set(actors.map((a) => a.id));
    for (const [id, entry] of mannequins) {
      if (!keep.has(id)) {
        if (gizmo.object === entry.m.root) gizmo.detach();
        actorsGroup.remove(entry.m.root);
        entry.m.dispose();
        mannequins.delete(id);
      }
    }
    for (const a of actors) {
      let existing = mannequins.get(a.id);
      // 人 / 群演 / 道具换了类别，或者道具换了图：外形不一样，整个重建
      if (existing && ((existing.data.kind ?? "person") !== (a.kind ?? "person") || existing.data.imagePath !== a.imagePath)) {
        if (gizmo.object === existing.m.root) gizmo.detach();
        actorsGroup.remove(existing.m.root);
        existing.m.dispose();
        mannequins.delete(a.id);
        existing = undefined;
      }
      if (existing) {
        // 拖动中不要被 React 回写的旧位置打回去
        if (gizmoDragging && a.id === selectedId) continue;
        try {
          applyActor(existing.m, a, existing.data);
        } catch (e) {
          opts.onError?.(String(e instanceof Error ? e.message : e));
        }
        existing.data = a;
      } else {
        try {
          const m =
            a.kind === "prop"
              ? createPropObject(a.color, a.imagePath && opts.loadImage ? opts.loadImage(a.imagePath) : null)
              : createMannequin(library, a.color, a.pose);
          applyActor(m, a);
          actorsGroup.add(m.root);
          mannequins.set(a.id, { m, data: a });
        } catch (e) {
          opts.onError?.(String(e instanceof Error ? e.message : e));
        }
      }
    }
    syncGizmo();
    updateSelectionBox();
    syncGroundRings();
  }

  /** 坐标轴挂谁：选中的人偶优先，其次选中的机位。机位能切「移动 / 旋转」，人偶只移动 */
  function syncGizmo() {
    const entry = selectedId ? mannequins.get(selectedId) : null;
    const camMarker = !entry && selectedCameraId ? markers.get(selectedCameraId) : null;
    const target = entry?.m.root ?? camMarker?.pivot ?? null;
    const rotate = !!camMarker && cameraGizmoMode === "rotate";
    gizmo.setMode(rotate ? "rotate" : "translate");
    // 开着「高斯地面吸附」的人偶只能在地面上：不给上下拖的那根轴（机位不受影响）
    gizmo.showY = !(entry && gaussianGroundSnap);
    // 旋转按机位自己的轴转（绕它的上方向 = 水平、绕右方向 = 俯仰、绕镜头方向 = 横滚）；移动按世界轴
    gizmo.setSpace(rotate ? "local" : "world");
    if (target) {
      if (gizmo.object !== target) gizmo.attach(target);
    } else if (gizmo.object) {
      gizmo.detach();
    }
  }

  const ready = loadMannequinLibrary()
    .then((lib) => {
      library = lib;
      reconcile(pendingActors);
    })
    .catch((e) => {
      opts.onError?.(String(e instanceof Error ? e.message : e));
    });

  // ── 机位标记（SVG 摄像机图标 + 取景框，见 cameraMarker.ts）──
  // **按 id 留着同一个对象**：三维坐标轴挂在机位的 pivot 上，每次 React 回传都清掉重建的话坐标轴就挂空了
  const markerGroup = new THREE.Group();
  editorOnly.add(markerGroup);
  const markers = new Map<string, CameraMarkerObject>();
  let selectedCameraId: string | null = null;
  let cameraGizmoMode: "translate" | "rotate" = "translate";
  function reconcileMarkers(list: CameraMarker[]) {
    const keep = new Set(list.map((m) => m.id));
    for (const [id, m] of markers) {
      if (keep.has(id)) continue;
      if (gizmo.object === m.pivot) gizmo.detach();
      markerGroup.remove(m.pivot);
      m.dispose();
      markers.delete(id);
    }
    for (const m of list) {
      const existing = markers.get(m.id);
      if (existing) {
        // 正在拖这个机位：别让 React 回写的旧位置把它打回去
        existing.update(m, { keepPose: gizmoDragging && gizmo.object === existing.pivot });
      } else {
        const obj = createCameraMarker(m);
        markers.set(m.id, obj);
        markerGroup.add(obj.pivot);
      }
    }
    selectedCameraId = list.find((m) => m.selected)?.id ?? null;
    syncGizmo();
  }
  function pickCamera(clientX: number, clientY: number): string | null {
    rayAt(clientX, clientY);
    const hit = raycaster.intersectObjects([...markers.values()].map((m) => m.sprite), false)[0];
    return (hit?.object.userData.cameraId as string | undefined) ?? null;
  }

  // ── 相机 ──
  function syncCamera() {
    const size = renderer.getSize(new THREE.Vector2());
    applyCamera(camera, cam, size.x / Math.max(1, size.y));
  }

  function forwardFlat(): THREE.Vector3 {
    const r = (cam.yawDeg * Math.PI) / 180;
    return new THREE.Vector3(Math.sin(r), 0, -Math.cos(r));
  }

  function rightFlat(): THREE.Vector3 {
    const r = (cam.yawDeg * Math.PI) / 180;
    return new THREE.Vector3(Math.cos(r), 0, Math.sin(r));
  }

  let cameraLocked = true;

  // ── 导演视角：绕中心转 ──
  let navMode: "fly" | "orbit" = "fly";
  const orbit = { target: new THREE.Vector3(0, 1, -3), dist: 6, yaw: 0, pitch: -20 };
  function orbitForward(): THREE.Vector3 {
    const y = (orbit.yaw * Math.PI) / 180;
    const p = (orbit.pitch * Math.PI) / 180;
    return new THREE.Vector3(Math.sin(y) * Math.cos(p), Math.sin(p), -Math.cos(y) * Math.cos(p));
  }
  function applyOrbit() {
    const pos = orbit.target.clone().sub(orbitForward().multiplyScalar(orbit.dist));
    cam = { position: [round2(pos.x), round2(pos.y), round2(pos.z)], yawDeg: round2(orbit.yaw), pitchDeg: round2(orbit.pitch), fovDeg: cam.fovDeg, rollDeg: 0 };
    syncCamera();
  }
  /** 从一个相机进入绕转：中心取它前方 `dist` 米 */
  function orbitFrom(c: DirectorCamera, dist = orbit.dist) {
    orbit.yaw = c.yawDeg;
    orbit.pitch = Math.max(-89, Math.min(89, c.pitchDeg));
    orbit.dist = dist;
    const y = (c.yawDeg * Math.PI) / 180;
    const p = (c.pitchDeg * Math.PI) / 180;
    orbit.target.set(...c.position).add(new THREE.Vector3(Math.sin(y) * Math.cos(p), Math.sin(p), -Math.cos(y) * Math.cos(p)).multiplyScalar(dist));
  }

  function moveCamera(delta: THREE.Vector3) {
    if (navMode === "orbit") {
      // 导演视角不限范围：平移的是绕转中心
      orbit.target.add(delta);
      applyOrbit();
      return;
    }
    if (cameraLocked) return;
    const p = new THREE.Vector3(...cam.position).add(delta);
    // 别飞到地底下去，也别飞出全景球
    p.y = Math.max(0.1, p.y);
    const horiz = Math.hypot(p.x, p.z);
    // 离拍摄点太远，全景的墙面就歪得没法看了
    const maxR = DOME_RADIUS * 0.8;
    if (horiz > maxR) {
      p.x *= maxR / horiz;
      p.z *= maxR / horiz;
    }
    cam = { ...cam, position: [round2(p.x), round2(p.y), round2(p.z)] };
    syncCamera();
  }

  let cameraDirty = false;
  function markCameraChanged() {
    cameraDirty = true;
  }
  function flushCamera() {
    if (cameraDirty) {
      cameraDirty = false;
      opts.onCameraChange?.(cam);
    }
  }

  // ── 指针 ──
  const raycaster = new THREE.Raycaster();
  function rayAt(clientX: number, clientY: number) {
    const rect = canvas.getBoundingClientRect();
    const ndc = new THREE.Vector2(
      ((clientX - rect.left) / rect.width) * 2 - 1,
      -((clientY - rect.top) / rect.height) * 2 + 1,
    );
    raycaster.setFromCamera(ndc, camera);
    return raycaster.ray;
  }

  function groundPoint(clientX: number, clientY: number): [number, number, number] | null {
    const hit = rayAt(clientX, clientY).intersectPlane(new THREE.Plane(new THREE.Vector3(0, 1, 0), -groundHeight), new THREE.Vector3());
    if (!hit) return null;
    // 两个吸附叠加：先按网格定水平位置，再在那个位置上找 3D 片场的地表定高度
    // （以前找到地表就直接用地表的点，网格吸附被跳过 —— 两个开关互不相干）
    const [x, y, z] = snapPoint(hit);
    const surface = gaussianGroundSnap ? splatGroundAt(x, z) : null;
    return surface ? [x, round2(surface[1]), z] : [x, y, z];
  }

  function snapPoint(p: THREE.Vector3): [number, number, number] {
    return [round2(gridSnap ? Math.round(p.x) : p.x), round2(p.y), round2(gridSnap ? Math.round(p.z) : p.z)];
  }

  /** 在给定 x/z 上向下打高斯射线，取最接近校准地面的表面。 */
  function splatGroundAt(x: number, z: number): [number, number, number] | null {
    if (!splatViewer || !hasSplat) return null;
    const hits: Array<{ origin: THREE.Vector3; distance?: number }> = [];
    splatViewer.raycaster.ray.origin.set(x, groundHeight + 8, z);
    splatViewer.raycaster.ray.direction.set(0, -1, 0);
    splatViewer.raycaster.intersectSplatMesh(splatViewer.splatMesh, hits);
    const candidates = hits.filter((h) => Math.abs(h.origin.y - groundHeight) <= 2.5);
    if (!candidates.length) return null;
    const hit = candidates.reduce((a, b) => Math.abs(a.origin.y - groundHeight) <= Math.abs(b.origin.y - groundHeight) ? a : b);
    return snapPoint(hit.origin);
  }

  /** 用蒙皮后的包围盒挑人：比逐三角形射线快，躺倒、蹲下的人也挑得中 */
  function pickActor(clientX: number, clientY: number): string | null {
    const ray = rayAt(clientX, clientY);
    let best: { id: string; d: number } | null = null;
    for (const [id, { m }] of mannequins) {
      const p = ray.intersectBox(m.bounds(), new THREE.Vector3());
      if (!p) continue;
      const d = p.distanceTo(ray.origin);
      if (!best || d < best.d) best = { id, d };
    }
    return best?.id ?? null;
  }

  let drag: { button: number; x: number; y: number; lastX: number; lastY: number; moved: boolean; shift: boolean } | null = null;

  const onPointerDown = (e: PointerEvent) => {
    // 指针落在 gizmo 的轴上：交给 gizmo
    if (gizmo.axis) return;
    canvas.setPointerCapture(e.pointerId);
    drag = { button: e.button, x: e.clientX, y: e.clientY, lastX: e.clientX, lastY: e.clientY, moved: false, shift: e.shiftKey };
  };

  const onPointerMove = (e: PointerEvent) => {
    if (!drag || gizmoDragging) return;
    const dx = e.clientX - drag.lastX;
    const dy = e.clientY - drag.lastY;
    drag.lastX = e.clientX;
    drag.lastY = e.clientY;
    if (Math.hypot(e.clientX - drag.x, e.clientY - drag.y) > CLICK_SLOP) drag.moved = true;
    if (!drag.moved) return;
    if (navMode === "orbit") {
      if (drag.button === 0 && !drag.shift) {
        orbit.yaw = normalize180(orbit.yaw + dx * LOOK_SENSITIVITY);
        orbit.pitch = Math.max(-89, Math.min(89, orbit.pitch - dy * LOOK_SENSITIVITY));
        applyOrbit();
      } else if (drag.button === 2 || drag.button === 1) {
        // 平移中心：离得越远平移越快
        const k = 0.0016 * orbit.dist;
        const up = new THREE.Vector3(0, 1, 0).applyQuaternion(camera.quaternion);
        const right = new THREE.Vector3(1, 0, 0).applyQuaternion(camera.quaternion);
        moveCamera(right.multiplyScalar(-dx * k).add(up.multiplyScalar(dy * k)));
      }
      markCameraChanged();
      return;
    }
    if (drag.button === 0 && !drag.shift) {
      cam = {
        ...cam,
        yawDeg: normalize180(cam.yawDeg + dx * LOOK_SENSITIVITY),
        pitchDeg: Math.max(-89, Math.min(89, cam.pitchDeg - dy * LOOK_SENSITIVITY)),
      };
      syncCamera();
      markCameraChanged();
    } else if (drag.button === 2 || drag.button === 1) {
      const k = 0.01;
      const up = new THREE.Vector3(0, 1, 0);
      moveCamera(rightFlat().multiplyScalar(-dx * k).add(up.multiplyScalar(dy * k)));
      markCameraChanged();
    }
  };

  const onPointerUp = (e: PointerEvent) => {
    if (!drag) return;
    const d = drag;
    drag = null;
    if (canvas.hasPointerCapture(e.pointerId)) canvas.releasePointerCapture(e.pointerId);
    if (d.moved) {
      flushCamera();
      return;
    }
    if (d.button === 2 || (d.button === 0 && d.shift)) {
      const p = groundPoint(e.clientX, e.clientY);
      if (p) opts.onPlace?.(p);
      return;
    }
    if (d.button === 0) {
      // 机位图标画在最上层（不参与深度），先看点没点中它
      const camId = editorOnly.visible ? pickCamera(e.clientX, e.clientY) : null;
      if (camId) {
        opts.onSelectCamera?.(camId);
        return;
      }
      // **点空白不取消选中**（用户反馈：一点空白选中就丢了，接着按键没反应）。取消选中按 Esc
      const id = pickActor(e.clientX, e.clientY);
      if (id && id !== selectedId) opts.onSelect?.(id);
    }
  };

  const onContextMenu = (e: Event) => e.preventDefault();

  let wheelTimer = 0;
  const onWheel = (e: WheelEvent) => {
    e.preventDefault();
    if (navMode === "orbit") {
      // 整体缩放：沿视线离中心近 / 远，按比例（远处一滚走得多，近处走得少）
      orbit.dist = Math.max(0.3, Math.min(400, orbit.dist * Math.exp(e.deltaY * 0.0015)));
      applyOrbit();
    } else if (cameraLocked) {
      // 锁在拍摄点时滚轮改视场角（推拉会离开拍摄点）
      cam = { ...cam, fovDeg: Math.max(20, Math.min(110, round2(cam.fovDeg + Math.sign(e.deltaY) * 2))) };
      syncCamera();
    } else {
      const dir = new THREE.Vector3();
      camera.getWorldDirection(dir);
      moveCamera(dir.multiplyScalar(-e.deltaY * 0.004));
    }
    markCameraChanged();
    window.clearTimeout(wheelTimer);
    wheelTimer = window.setTimeout(flushCamera, 200);
  };

  canvas.addEventListener("pointerdown", onPointerDown);
  canvas.addEventListener("pointermove", onPointerMove);
  canvas.addEventListener("pointerup", onPointerUp);
  canvas.addEventListener("contextmenu", onContextMenu);
  canvas.addEventListener("wheel", onWheel, { passive: false });

  // ── 键盘飞行（按键由编辑器转进来）──
  const keys = new Set<string>();
  const FLY_CODES = new Set(["KeyW", "KeyA", "KeyS", "KeyD", "KeyQ", "KeyE"]);

  function centerClient(): [number, number] {
    const r = canvas.getBoundingClientRect();
    return [r.left + r.width / 2, r.top + r.height / 2];
  }

  // ── 渲染循环 ──
  let raf = 0;
  let last = performance.now();
  const loop = (now: number) => {
    const dt = Math.min(0.1, (now - last) / 1000);
    last = now;
    if (keys.size) {
      const v = new THREE.Vector3();
      if (keys.has("KeyW")) v.add(forwardFlat());
      if (keys.has("KeyS")) v.sub(forwardFlat());
      if (keys.has("KeyD")) v.add(rightFlat());
      if (keys.has("KeyA")) v.sub(rightFlat());
      if (keys.has("KeyE")) v.y += 1;
      if (keys.has("KeyQ")) v.y -= 1;
      if (v.lengthSq() > 0) {
        v.normalize().multiplyScalar(FLY_SPEED * (keys.has("Shift") ? 3 : 1) * dt);
        moveCamera(v);
        markCameraChanged();
      }
    }
    grid.follow(camera);
    for (const m of markers.values()) m.faceScreen(camera);
    if (splatViewer && splatViewer.getSceneCount() > 0) {
      splatViewer.update();
      splatViewer.render();
    } else {
      renderer.render(scene, camera);
    }
    raf = requestAnimationFrame(loop);
  };
  syncCamera();
  raf = requestAnimationFrame(loop);

  function normalize180(v: number) {
    return ((v + 540) % 360) - 180;
  }

  return {
    ready: ready.then(() => undefined),
    setPano(url) {
      if (!url) {
        panoTexture?.dispose();
        panoTexture = null;
        dome.setTexture(null);
        syncWorldVisibility();
        return;
      }
      opts.onStatus?.("加载全景图…");
      new THREE.TextureLoader().load(
        url,
        (tex) => {
          tex.colorSpace = THREE.SRGBColorSpace;
          tex.minFilter = THREE.LinearFilter;
          tex.wrapS = THREE.RepeatWrapping;
          panoTexture?.dispose();
          panoTexture = tex;
          dome.setTexture(tex);
          syncWorldVisibility();
          opts.onStatus?.("");
        },
        undefined,
        () => opts.onError?.("全景图加载失败"),
      );
    },
    async setSplat(source) {
      const token = ++splatLoadToken;
      if (!source) {
        await clearSplat();
        hasSplat = false;
        syncWorldVisibility();
        return;
      }
      opts.onStatus?.("加载 3DGS 片场…");
      try {
        const viewer = await ensureSplatViewer();
        await clearSplat(viewer);
        if (token !== splatLoadToken) return;
        const { SceneFormat } = await import("@mkkellogg/gaussian-splats-3d");
        const format = source.format === "ply" ? SceneFormat.Ply : source.format === "ksplat" ? SceneFormat.KSplat : SceneFormat.Spz;
        splatConvention = source.convention;
        splatScale = source.scale && source.scale > 0 ? source.scale : 1;
        await viewer.addSplatScene(source.url, {
          format,
          // SHARP 的原相机坐标 → 导演世界（见 scene.splatPlacement）。眼高取载入这一刻的
          ...splatPlacement(source.convention, eyeHeight, splatScale),
          showLoadingUI: false,
          progressiveLoad: false,
          splatAlphaRemovalThreshold: 5,
        });
        if (token !== splatLoadToken) {
          await clearSplat(viewer);
          return;
        }
        hasSplat = true;
        syncWorldVisibility();
        opts.onStatus?.("");
      } catch (e) {
        hasSplat = false;
        syncWorldVisibility();
        opts.onError?.(`3DGS 加载失败：${String(e instanceof Error ? e.message : e)}`);
      }
    },
    setCorrection(c) {
      correction = c;
      dome.setOrientation(correction, frontYaw);
    },
    setFrontYaw(deg) {
      frontYaw = deg;
      dome.setOrientation(correction, frontYaw);
    },
    setEyeHeight(h) {
      eyeHeight = Math.max(0.2, h);
      dome.setHeight(eyeHeight);
      // SHARP 片场的原点就是拍摄点：眼高变了，片场跟着相机一起抬 / 降，
      // 相对画面不动、只有地面网格相对片场移动 —— 和全景球一个道理，拖到网格贴住地板为止
      if (splatConvention === "opencv_camera" && splatViewer && splatViewer.getSceneCount() > 0) {
        splatViewer.getSplatScene(0).position.set(...splatPlacement(splatConvention, eyeHeight).position!);
      }
      if (cameraLocked) {
        cam = { ...cam, position: [0, eyeHeight, 0] };
        syncCamera();
      }
    },
    setActors(actors) {
      pendingActors = actors;
      reconcile(actors);
    },
    setSelected(id) {
      selectedId = id;
      syncGizmo();
      updateSelectionBox();
    },
    getCamera() {
      return cam;
    },
    setCamera(c) {
      cam = c;
      if (navMode === "orbit") orbitFrom(c);
      syncCamera();
    },
    withCamera(c, fn) {
      const saved = cam;
      cam = c;
      syncCamera();
      try {
        return fn();
      } finally {
        cam = saved;
        syncCamera();
      }
    },
    setSphereRadius(r) {
      sphereRadius = r;
      dome.setSphereRadius(r);
    },
    setCameraGizmoMode(mode) {
      cameraGizmoMode = mode;
      syncGizmo();
    },
    viewFrom(c, pivot = 2) {
      cam = { ...c };
      if (navMode === "orbit") orbitFrom(c, pivot);
      syncCamera();
    },
    setPanoramaProxy(on) {
      panoramaProxy = on;
      syncWorldVisibility();
    },
    setGridVisible(on) {
      grid.group.visible = on;
    },
    setGridOpacity(opacity) {
      grid.setOpacity(opacity);
    },
    setGroundHeight(y) {
      groundHeight = y;
      grid.setHeight(y);
      syncGroundRings();
    },
    setGridSnap(on) {
      gridSnap = on;
    },
    setGaussianGroundSnap(on) {
      gaussianGroundSnap = on;
      syncGizmo();
    },
    orientationAxes() {
      const inverse = camera.quaternion.clone().invert();
      return ([
        ["x", new THREE.Vector3(1, 0, 0)],
        ["y", new THREE.Vector3(0, 1, 0)],
        ["z", new THREE.Vector3(0, 0, 1)],
      ] as const).map(([id, axis]) => {
        const p = axis.applyQuaternion(inverse);
        return { id, x: p.x, y: -p.y, depth: p.z };
      });
    },
    snapDirectorView(axis) {
      if (navMode !== "orbit") return;
      const views: Record<typeof axis, [number, number]> = {
        "+x": [-90, 0], "-x": [90, 0], "+y": [0, -89], "-y": [0, 89], "+z": [0, 0], "-z": [180, 0],
      };
      [orbit.yaw, orbit.pitch] = views[axis];
      applyOrbit();
      markCameraChanged();
      flushCamera();
    },
    resetDirectorView() {
      if (navMode !== "orbit") return;
      orbit.target.set(0, groundHeight + 1, 0);
      orbit.dist = Math.max(8, sphereRadius * 1.7);
      orbit.yaw = 35;
      orbit.pitch = -20;
      applyOrbit();
      markCameraChanged();
      flushCamera();
    },
    pullBack(meters) {
      if (navMode !== "orbit") return;
      orbit.dist += meters;
      applyOrbit();
      opts.onCameraChange?.(cam);
    },
    setSplatScale(scale) {
      splatScale = scale > 0 ? scale : 1;
      if (splatViewer && splatViewer.getSceneCount() > 0) splatViewer.getSplatScene(0).scale.setScalar(splatScale);
    },
    setNavMode(mode) {
      if (mode === navMode) return;
      navMode = mode;
      if (mode === "orbit") orbitFrom(cam, 6);
      // 两种页签都按 libtv 保持完整球；机位取景只是临时换相机，不再把球底压成地面。
      dome.setShape("sphere");
      syncWorldVisibility();
    },
    keyDown(code, shift) {
      if (shift) keys.add("Shift");
      else keys.delete("Shift");
      if (!FLY_CODES.has(code) || (cameraLocked && navMode === "fly")) return false;
      keys.add(code);
      return true;
    },
    keyUp(code) {
      keys.delete(code);
      if (code.startsWith("Shift")) keys.delete("Shift");
      if (![...keys].some((k) => FLY_CODES.has(k))) flushCamera();
    },
    setCameraLocked(on) {
      cameraLocked = on;
      if (on) {
        keys.clear();
        cam = { ...cam, position: [0, eyeHeight, 0] };
        syncCamera();
        opts.onCameraChange?.(cam);
      }
    },
    crosshairGround() {
      const [x, y] = centerClient();
      return groundPoint(x, y);
    },
    pickAtCrosshair() {
      const [x, y] = centerClient();
      return pickActor(x, y);
    },
    labels() {
      const rect = canvas.getBoundingClientRect();
      const out: ActorLabel[] = [];
      for (const [id, { m }] of mannequins) {
        const b = m.bounds();
        const top = new THREE.Vector3((b.min.x + b.max.x) / 2, b.max.y + 0.15, (b.min.z + b.max.z) / 2);
        const view = top.clone().applyMatrix4(camera.matrixWorldInverse);
        const ndc = top.project(camera);
        out.push({
          id,
          x: ((ndc.x + 1) / 2) * rect.width,
          y: ((1 - ndc.y) / 2) * rect.height,
          visible: view.z < 0 && Math.abs(ndc.x) <= 1.1 && Math.abs(ndc.y) <= 1.1,
        });
      }
      return out;
    },
    setCameraMarkers(list) {
      reconcileMarkers(list);
    },
    project(point) {
      const rect = canvas.getBoundingClientRect();
      const p = new THREE.Vector3(...point);
      const view = p.clone().applyMatrix4(camera.matrixWorldInverse);
      const ndc = p.project(camera);
      return {
        x: ((ndc.x + 1) / 2) * rect.width,
        y: ((1 - ndc.y) / 2) * rect.height,
        visible: view.z < 0 && Math.abs(ndc.x) <= 1.05 && Math.abs(ndc.y) <= 1.05,
      };
    },
    capture(mode, rectCss, longEdge = 1920) {
      const before = {
        editor: editorOnly.visible,
        gizmo: gizmoHelper.visible,
        sphere: dome.mesh.visible,
        shape: dome.shape(),
        actors: actorsGroup.visible,
        splat: splatViewer?.splatMesh.visible ?? false,
      };
      try {
        // 完整球在拍摄点内看到的就是正常透视全景；压平下半球会把地面拉成放射状。
        dome.setShape("sphere");
        if (hasSplat && splatViewer?.splatMesh) {
          dome.mesh.visible = false;
          splatViewer.splatMesh.visible = true;
        }
        editorOnly.visible = false;
        gizmoHelper.visible = false;
        if (mode === "env_only") actorsGroup.visible = false;
        if (mode === "actor_overlay_black" || mode === "actor_mask") dome.mesh.visible = false;
        if (mode === "actor_mask") for (const { m } of mannequins.values()) m.setMaskMaterial(true);
        // 临时把像素比调到「画幅框长边 = longEdge」，渲染完再调回去
        renderer.setPixelRatio(longEdge / Math.max(1, rectCss.width, rectCss.height));
        syncCamera();
        if (splatViewer && splatViewer.getSceneCount() > 0) {
          splatViewer.update();
          splatViewer.render();
        } else {
          renderer.render(scene, camera);
        }
        const src = renderer.domElement;
        const k = src.width / Math.max(1, canvas.clientWidth);
        const sx = Math.max(0, Math.round(rectCss.x * k));
        const sy = Math.max(0, Math.round(rectCss.y * k));
        const sw = Math.min(src.width - sx, Math.round(rectCss.width * k));
        const sh = Math.min(src.height - sy, Math.round(rectCss.height * k));
        const out = document.createElement("canvas");
        out.width = Math.max(1, sw);
        out.height = Math.max(1, sh);
        const ctx = out.getContext("2d");
        if (!ctx) throw new Error("拿不到 2d context，截不了图");
        ctx.drawImage(src, sx, sy, sw, sh, 0, 0, out.width, out.height);
        return out.toDataURL("image/png");
      } finally {
        editorOnly.visible = before.editor;
        gizmoHelper.visible = before.gizmo;
        dome.mesh.visible = before.sphere;
        dome.setShape(before.shape);
        actorsGroup.visible = before.actors;
        if (splatViewer?.splatMesh) splatViewer.splatMesh.visible = before.splat;
        for (const { m } of mannequins.values()) m.setMaskMaterial(false);
        renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
        syncCamera();
        if (splatViewer && splatViewer.getSceneCount() > 0) {
          splatViewer.update();
          splatViewer.render();
        } else {
          renderer.render(scene, camera);
        }
      }
    },
    resize(width, height) {
      if (width <= 0 || height <= 0) return;
      renderer.setSize(width, height, false);
      syncCamera();
    },
    dispose() {
      cancelAnimationFrame(raf);
      window.clearTimeout(wheelTimer);
      canvas.removeEventListener("pointerdown", onPointerDown);
      canvas.removeEventListener("pointermove", onPointerMove);
      canvas.removeEventListener("pointerup", onPointerUp);
      canvas.removeEventListener("contextmenu", onContextMenu);
      canvas.removeEventListener("wheel", onWheel);
      gizmo.detach();
      gizmo.dispose();
      for (const ring of groundRings.values()) (ring.material as THREE.Material).dispose();
      groundRings.clear();
      ringGeometry.dispose();
      for (const m of markers.values()) m.dispose();
      markers.clear();
      for (const { m } of mannequins.values()) m.dispose();
      panoTexture?.dispose();
      splatLoadToken++;
      if (splatViewer) void splatViewer.dispose();
      dome.dispose();
      grid.dispose();
      renderer.dispose();
    },
  };
}
