import * as THREE from "three";
import { clampFov, normalizeDeg, type CaptureAspect, type Shot } from "./panoMath";
import { cropRectForAspect } from "./panoMath";
import { cameraToSphere, clampPlanetFov, stereoScale } from "./panoOrient";

/**
 * 全景舞台 —— 一张等距圆柱贴图铺在球内侧，相机站在球心。
 *
 * 全景查看器用它。导演世界有自己的引擎（`director/engine.ts`，要放人偶、gizmo、飞行相机），
 * 但**环球几何体共用 `makeSphereGeometry`** —— 贴图朝向只能有一份定义。
 *
 * # 材质那四个开关缺一不可
 *
 * 抄 DramaClaw 的 `viewerApp` 里那段（它用 PlayCanvas，我们用 three）：
 *
 * - 球要**从里面看得见、而且不镜像** —— 见 `makeSphereGeometry`
 * - 用 `MeshBasicMaterial` 而不是 Standard —— 全景图**自带光照**，
 *   再被场景灯照一遍就是错的（DramaClaw 是 `useLighting=false` + emissive）
 * - `depthWrite: false` —— 环境球不该挡住站在里面的标记物
 * - `colorSpace = SRGBColorSpace` —— 不设的话整张图发灰，而且**不报错**
 */

/** 球半径。够大就行 —— 站在球心，半径只影响和标记物的相对尺度。 */
export const SPHERE_RADIUS = 120;

/**
 * 环境球的几何体：**X 取反 + 绕 Y 转 -90°**。
 *
 * # 为什么不是 `SphereGeometry` + `BackSide`
 *
 * 原来就是那样写的，结果**整张全景左右镜像**：站在球里往右转，贴图的 u 在变小。
 * 看场景图看不出来（店堂左右颠倒一样像店堂），要拿带字的贴图才看得出字是反的
 * —— 2026-09-16 做接缝按钮时对照原图才发现。截出去的图左右全是反的，
 * 「门在左边」进了分镜会变成「门在右边」。
 *
 * three 官方全景示例的做法是 `scale(-1, 1, 1)`：X 取反同时把三角形绕序翻过来，
 * 正面朝内，默认的 `FrontSide` 就能从里面看到，而且不镜像。
 *
 * 再绕 Y 转 -90°，让**贴图正中（u=0.5）落在相机默认朝向 -Z** —— 全景图生成时
 * 主体在正中，yaw 0 就该看到它。左右接缝（u=0/1）因此在正后方。
 * 这两条约定有测试钉着（`panoOrient.test.ts`），`seamView` 依赖它。
 */
export function makeSphereGeometry(): THREE.SphereGeometry {
  const g = new THREE.SphereGeometry(SPHERE_RADIUS, 64, 48);
  g.scale(-1, 1, 1);
  g.rotateY(-Math.PI / 2);
  return g;
}

export interface StageOptions {
  canvas: HTMLCanvasElement;
  /** 等距圆柱全景图的 URL（媒体协议或 data:）。 */
  textureUrl: string;
  onError?: (message: string) => void;
}

export type PanoProjection = "rect" | "planet";

/**
 * 小行星的着色器。整屏一个四边形，逐像素反算视线方向再去贴图里取色 ——
 * 普通相机只能做透视投影（到 180° 就无穷大），球极投影只能这样画。
 *
 * 公式和 `panoOrient` 的 `stereoRay` / `equirectUv` 一一对应，JS 版有测试。
 */
const PLANET_VERTEX = /* glsl */ `
varying vec2 vUv;
void main() {
  vUv = uv;
  gl_Position = vec4(position.xy, 0.0, 1.0);
}
`;

const PLANET_FRAGMENT = /* glsl */ `
#define PI 3.141592653589793
uniform sampler2D map;
uniform float aspect;
uniform float scale;
uniform mat3 rot;
varying vec2 vUv;
void main() {
  vec2 p = vec2((vUv.x * 2.0 - 1.0) * aspect, vUv.y * 2.0 - 1.0) * scale;
  float r = length(p);
  float th = 2.0 * atan(r * 0.5);
  vec3 d = r > 1e-6 ? vec3(sin(th) * p / r, -cos(th)) : vec3(0.0, 0.0, -1.0);
  d = normalize(rot * d);
  float u = 0.5 + atan(d.x, -d.z) / (2.0 * PI);
  float v = 1.0 - acos(clamp(d.y, -1.0, 1.0)) / PI;
  gl_FragColor = texture2D(map, vec2(u, v));
  #include <colorspace_fragment>
}
`;

export interface SphereCorrectionDeg {
  roll: number;
  pitch: number;
  yaw: number;
}

export interface PanoStage {
  /** 相机朝向。角度制，yaw 0 = 球面贴图的正前方 + `frontYaw` 偏移。 */
  setView(view: { yawDeg: number; pitchDeg: number; fovDeg: number }): void;
  /** 球面校正 —— 生成的全景不可能正好水平，校一次所有机位都正。 */
  setCorrection(c: SphereCorrectionDeg): void;
  /** 正前方偏移：只改读数的原点，不动贴图。 */
  setFrontYaw(deg: number): void;
  /**
   * 投影方式。`planet` = 球极投影（小行星）：视场角可以到 330°，
   * 朝下看是小行星、朝上看是隧道。**只画环境球**，场景里加的其它物体不进这条路。
   */
  setProjection(p: PanoProjection): void;
  /** 画一帧。 */
  render(): void;
  /** 视口尺寸变了。 */
  resize(width: number, height: number): void;
  /** 当前这一帧按画幅裁出来的 PNG dataURL。 */
  capture(aspect: CaptureAspect): string;
  /** three 场景。 */
  scene: THREE.Scene;
  camera: THREE.PerspectiveCamera;
  /** 环境球本体。截「只有标记物」那一档时要把它关掉。 */
  sphere: THREE.Mesh;
  dispose(): void;
}

/**
 * 把 three 的加载失败说成人话。
 *
 * 最常见的一种是**传了本机路径而不是可加载的 URL** —— 要先过
 * `mediaDataUrl` 读成 data URL，直接把 `/Users/...` 丢给 `TextureLoader` 一定失败。
 */
export function describeLoadError(e: unknown, url: string): string {
  const looksLikePath = /^[/~]/.test(url) || /^[A-Za-z]:\\/.test(url);
  // data URL 动辄几 MB 的 base64，整串塞进报错框等于把界面撑爆
  const shown = url.length > 120 ? `${url.slice(0, 60)}…（共 ${url.length} 字符）` : url;
  const head = looksLikePath
    ? `贴图加载失败：传进来的是本机路径而不是可加载的 URL（${shown}）。要先过 mediaDataUrl。`
    : `贴图加载失败：${shown}`;
  if (e instanceof Error) return `${head} —— ${e.message}`;
  if (typeof e === "object" && e && "type" in e) {
    return `${head} —— ${(e as Event).type}`;
  }
  return head;
}

export function createPanoStage(opts: StageOptions): PanoStage {
  const renderer = new THREE.WebGLRenderer({
    canvas: opts.canvas,
    antialias: true,
    // **必须**：不保留的话 `toDataURL` 在下一帧之前就已经是空的了，
    // 截出来是一张全黑 —— 而且不报错。
    preserveDrawingBuffer: true,
  });
  renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));

  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(70, 16 / 9, 0.01, SPHERE_RADIUS * 4);
  camera.position.set(0, 0, 0);

  const geometry = makeSphereGeometry();
  const material = new THREE.MeshBasicMaterial({
    depthWrite: false,
    color: 0x111111,
  });
  const sphere = new THREE.Mesh(geometry, material);
  scene.add(sphere);

  const planetMaterial = new THREE.ShaderMaterial({
    uniforms: {
      map: { value: null as THREE.Texture | null },
      aspect: { value: 16 / 9 },
      scale: { value: 1 },
      rot: { value: new THREE.Matrix3() },
    },
    vertexShader: PLANET_VERTEX,
    fragmentShader: PLANET_FRAGMENT,
    depthTest: false,
    depthWrite: false,
  });
  const planetQuad = new THREE.Mesh(new THREE.PlaneGeometry(2, 2), planetMaterial);
  planetQuad.frustumCulled = false;
  const planetScene = new THREE.Scene();
  planetScene.add(planetQuad);
  const planetCamera = new THREE.OrthographicCamera(-1, 1, 1, -1, 0, 1);

  const loader = new THREE.TextureLoader();
  loader.load(
    opts.textureUrl,
    (tex) => {
      // 不设色彩空间的话整张图发灰，**而且不报错**
      tex.colorSpace = THREE.SRGBColorSpace;
      tex.minFilter = THREE.LinearFilter;
      // 左右边缘首尾相接：小行星模式按 u 取色，跨过接缝时不重复的话会出一条竖线
      tex.wrapS = THREE.RepeatWrapping;
      material.map = tex;
      planetMaterial.uniforms.map.value = tex;
      material.color.set(0xffffff);
      material.needsUpdate = true;
    },
    undefined,
    // **`TextureLoader` 的失败回调给的是 `ErrorEvent`，不是 `Error`。**
    // 原来写成 `String(e)`，于是界面上只显示一句 `[object Event]` ——
    // 真正的失败原因（多半是 URL 根本加载不了）被吞掉了。
    (e) => opts.onError?.(describeLoadError(e, opts.textureUrl)),
  );

  let frontYaw = 0;
  let projection: PanoProjection = "rect";
  /** 小行星用的视图参数。透视相机的 fov 上限 170，小行星要到 330，所以另存一份 */
  const planetView = { yawTotal: 0, pitch: 0, fov: 270 };
  let correction: SphereCorrectionDeg = { roll: 0, pitch: 0, yaw: 0 };

  const draw = () => {
    if (projection === "planet" && planetMaterial.uniforms.map.value) {
      const size = renderer.getSize(new THREE.Vector2());
      planetMaterial.uniforms.aspect.value = size.x / Math.max(1, size.y);
      planetMaterial.uniforms.scale.value = stereoScale(planetView.fov);
      const q = cameraToSphere(correction, planetView.yawTotal, planetView.pitch);
      planetMaterial.uniforms.rot.value.setFromMatrix4(new THREE.Matrix4().makeRotationFromQuaternion(q));
      renderer.render(planetScene, planetCamera);
    } else {
      renderer.render(scene, camera);
    }
  };

  const deg = (d: number) => (d * Math.PI) / 180;

  return {
    scene,
    camera,
    sphere,
    setCorrection(c) {
      // 球自己转，相机不动 —— 这样「校正」和「机位」是两件独立的事，
      // 校完之后每个机位都跟着正了。
      sphere.rotation.set(deg(c.pitch), deg(c.yaw), deg(c.roll), "YXZ");
      correction = { ...c };
    },
    setFrontYaw(d) {
      frontYaw = d;
    },
    setProjection(p) {
      projection = p;
    },
    setView({ yawDeg, pitchDeg, fovDeg }) {
      // 小行星朝正下方要能到 -90°；透视相机到 ±90° 会万向锁，还是卡 89
      planetView.yawTotal = normalizeDeg(yawDeg + frontYaw);
      planetView.pitch = Math.max(-90, Math.min(90, pitchDeg));
      planetView.fov = clampPlanetFov(fovDeg);
      camera.fov = clampFov(fovDeg);
      camera.updateProjectionMatrix();
      const yaw = deg(normalizeDeg(yawDeg + frontYaw));
      const pitch = deg(Math.max(-89, Math.min(89, pitchDeg)));
      // 相机永远在球心，只转方向。**先绕 Y 再绕 X**（YXZ），
      // 反过来的话抬头之后左右就变成斜的了。
      camera.rotation.set(pitch, -yaw, 0, "YXZ");
    },
    resize(width, height) {
      if (width <= 0 || height <= 0) return;
      renderer.setSize(width, height, false);
      camera.aspect = width / Math.max(1, height);
      camera.updateProjectionMatrix();
    },
    render() {
      draw();
    },
    capture(aspect) {
      draw();
      const src = renderer.domElement;
      const rect = cropRectForAspect({ width: src.width, height: src.height }, aspect);
      const out = document.createElement("canvas");
      out.width = rect.width;
      out.height = rect.height;
      const ctx = out.getContext("2d");
      if (!ctx) throw new Error("拿不到 2d context，截不了图");
      ctx.drawImage(src, rect.x, rect.y, rect.width, rect.height, 0, 0, rect.width, rect.height);
      return out.toDataURL("image/png");
    },
    dispose() {
      geometry.dispose();
      material.map?.dispose();
      material.dispose();
      planetQuad.geometry.dispose();
      planetMaterial.dispose();
      renderer.dispose();
    },
  };
}

/** 把一组机位依次拍下来。**串行** —— 每一张都要等这一帧画完。 */
export async function captureShots(
  stage: PanoStage,
  shots: Shot[],
  aspect: CaptureAspect,
): Promise<{ shot: Shot; dataUrl: string }[]> {
  const out: { shot: Shot; dataUrl: string }[] = [];
  for (const shot of shots) {
    stage.setView(shot);
    // 等一帧，确保 setView 之后的矩阵真的用上了
    await new Promise((r) => requestAnimationFrame(r));
    out.push({ shot, dataUrl: stage.capture(aspect) });
  }
  return out;
}
