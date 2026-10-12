/**
 * 导演世界的「世界」：地面投影的全景 + 跟着相机走的无穷网格。
 *
 * # 为什么全景不能再贴在大球上
 *
 * 第一版照 DC 把全景贴在半径 120 米的球上。那等于**全景画在无穷远处**，
 * 它的地板不是一个真实存在的面：人偶站在 y=0 的地面上，网格一格 1 米，
 * 而全景里的地砖在哪、多大，和这套坐标毫无关系 ——
 * 人偶放在 14 米处，画面上可能落在 3 米外的地砖上（2026-09-16 用户截图指出）。
 * DC 的 360 来源也是同一个球，同样没对齐；它看着像无穷 3D 空间，靠的是跟着相机走的网格。
 *
 * # 地面投影
 *
 * 全景相机离地 h 米时，画面里**低于地平线 θ 度**的地板像素，正好是距离
 * `h / tan θ` 米的地面点。所以把球的下半部分压平成 y=0 的平面（three 的
 * GroundedSkybox 同一个做法），全景的地板就成了真实的地面：
 *
 * - 网格、人偶、全景地砖在同一套米制坐标里
 * - **眼高 h 就是尺度**：调到全景里的地砖 / 门 / 桌子和网格、1.83 米的人偶对得上为止
 * - 相机离开拍摄点走几步，地面仍然是对的（墙面还是近似，离开越远越歪）
 *
 * # 工作视口始终是完整的球（2026-09-28）
 *
 * 用户对比 libtv：「你这个导演世界不是个球型啊，怎么是个半球」「半球底部图形畸变，完全看不出地面」。
 * 压平的地面只在**拍摄点附近**看是对的；从外面看，地平线附近那一圈像素被摊到几十米外，拉成放射状的线。
 * 所以导演视角和机位属性视角都保留完整球，照 libtv 从外部检查片场；网格是独立的真实地面。
 * 机位小窗和输出临时切到对应摄像机。旧的地面投影几何仍保留给兼容测试和迁移，但不再用于工作视口或截图。
 *
 * 取色都按「到拍摄点的方向」算，所以两种形状从拍摄点看过去是**同一张图**，换形状不影响机位视角。
 *
 * # 取色按方向算，不按顶点 uv
 *
 * 压平只改顶点位置、不改「从拍摄点看过去的方向」，所以贴图坐标由片元到拍摄点的方向
 * 在着色器里算（公式和 `panoOrient.equirectUv` 一样，那边有测试）。这样球面校正、
 * 正前方偏移都只是一个旋转矩阵，不用重建几何体，也没有接缝三角形被拉穿的问题。
 */
import * as THREE from "three";
import type { SphereCorrectionDeg } from "../pano/panoStage";

const RAD = Math.PI / 180;

/**
 * 地面投影的穹顶几何体，**球心在原点**（放的时候挪到 (0, h, 0)）。
 *
 * 改写自 three `examples/jsm/objects/GroundedSkybox.js`（MIT）：下半球里
 * 低于 `-1.5h` 的点径向缩放到 y = -h 的平面上，之间平滑过渡。
 */
export function groundedDomeGeometry(radius: number, height: number, resolution = 128): THREE.BufferGeometry {
  return domeGeometry(radius, height, resolution, true);
}

/** 完整的球（导演视角用），球心在原点 */
export function sphereDomeGeometry(radius: number, resolution = 96): THREE.BufferGeometry {
  return domeGeometry(radius, 0, resolution, false);
}

function domeGeometry(radius: number, height: number, resolution: number, grounded: boolean): THREE.BufferGeometry {
  const g = new THREE.SphereGeometry(radius, 2 * resolution, resolution);
  if (!grounded) return g;
  const pos = g.getAttribute("position");
  const v = new THREE.Vector3();
  const y1 = (-height * 3) / 2;
  for (let i = 0; i < pos.count; i++) {
    v.fromBufferAttribute(pos, i);
    if (v.y < 0) {
      const f = v.y < y1 ? -height / v.y : 1 - (v.y * v.y) / (3 * y1 * y1);
      v.multiplyScalar(f);
      pos.setXYZ(i, v.x, v.y, v.z);
    }
  }
  pos.needsUpdate = true;
  g.computeBoundingSphere();
  return g;
}

/**
 * 世界方向 → 贴图方向的旋转：先扣掉正前方偏移，再扣掉球面校正。
 * 和全景查看器的约定一致（`panoOrient.textureDirectionAt`）。
 */
export function worldToTexture(c: SphereCorrectionDeg, frontYawDeg: number): THREE.Matrix3 {
  const sphere = new THREE.Quaternion().setFromEuler(new THREE.Euler(c.pitch * RAD, c.yaw * RAD, c.roll * RAD, "YXZ"));
  const front = new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(0, 1, 0), frontYawDeg * RAD);
  const q = front.multiply(sphere).invert();
  return new THREE.Matrix3().setFromMatrix4(new THREE.Matrix4().makeRotationFromQuaternion(q));
}

const DOME_VERTEX = /* glsl */ `
varying vec3 vDir;
void main() {
  vDir = position;
  gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
}
`;

const DOME_FRAGMENT = /* glsl */ `
#define PI 3.141592653589793
uniform sampler2D map;
uniform float hasMap;
uniform mat3 rot;
varying vec3 vDir;
void main() {
  if (hasMap < 0.5) { gl_FragColor = vec4(0.0, 0.0, 0.0, 1.0); return; }
  vec3 d = normalize(rot * normalize(vDir));
  float u = 0.5 + atan(d.x, -d.z) / (2.0 * PI);
  float v = 1.0 - acos(clamp(d.y, -1.0, 1.0)) / PI;
  gl_FragColor = texture2D(map, vec2(u, v));
  #include <colorspace_fragment>
}
`;

export interface PanoDome {
  mesh: THREE.Mesh;
  setTexture(tex: THREE.Texture | null): void;
  setOrientation(c: SphereCorrectionDeg, frontYawDeg: number): void;
  /** 拍摄高度（米）= 尺度 */
  setHeight(h: number): void;
  dispose(): void;
  /** 完整球是当前工作视口；grounded 仅保留旧数据兼容。 */
  setShape(shape: DomeShape): void;
  shape(): DomeShape;
  /** 导演视角那个球的半径（米） */
  setSphereRadius(r: number): void;
}

export type DomeShape = "grounded" | "sphere";

export const DOME_RADIUS = 60;
/** 导演视角的球默认多大。一间屋子的尺度，人偶在里面看得清 */
export const DEFAULT_SPHERE_RADIUS = 10;
export const SPHERE_RADIUS_RANGE: [number, number] = [4, 40];

export function createPanoDome(height: number): PanoDome {
  const material = new THREE.ShaderMaterial({
    uniforms: {
      map: { value: null as THREE.Texture | null },
      hasMap: { value: 0 },
      rot: { value: new THREE.Matrix3() },
    },
    vertexShader: DOME_VERTEX,
    fragmentShader: DOME_FRAGMENT,
    side: THREE.BackSide,
    depthWrite: false,
  });
  let h = height;
  let shape: DomeShape = "grounded";
  let grounded = groundedDomeGeometry(DOME_RADIUS, h);
  let sphereR = DEFAULT_SPHERE_RADIUS;
  let sphere: THREE.BufferGeometry | null = null;
  const mesh = new THREE.Mesh(grounded, material);
  mesh.position.set(0, h, 0);
  mesh.renderOrder = -1;
  mesh.frustumCulled = false;
  return {
    mesh,
    setTexture(tex) {
      material.uniforms.map.value = tex;
      material.uniforms.hasMap.value = tex ? 1 : 0;
    },
    setOrientation(c, front) {
      material.uniforms.rot.value = worldToTexture(c, front);
    },
    // 只画内面：从球外看，近处那半个球被剔掉，看进去是远处那半个球的里面（像切开的模型屋），
    // 球里的人偶不会被球面挡住
    setShape(next) {
      shape = next;
      if (next === "sphere" && !sphere) sphere = sphereDomeGeometry(sphereR);
      mesh.geometry = next === "sphere" ? sphere! : grounded;
    },
    shape: () => shape,
    setSphereRadius(r) {
      const next = Math.min(SPHERE_RADIUS_RANGE[1], Math.max(SPHERE_RADIUS_RANGE[0], r));
      if (Math.abs(next - sphereR) < 1e-6) return;
      sphereR = next;
      sphere?.dispose();
      sphere = null;
      if (shape === "sphere") {
        sphere = sphereDomeGeometry(sphereR);
        mesh.geometry = sphere;
      }
    },
    setHeight(next) {
      if (Math.abs(next - h) < 1e-6) return;
      h = next;
      grounded.dispose();
      grounded = groundedDomeGeometry(DOME_RADIUS, h);
      if (shape === "grounded") mesh.geometry = grounded;
      mesh.position.y = h;
    },
    dispose() {
      grounded.dispose();
      sphere?.dispose();
      material.dispose();
    },
  };
}

// ─────────────────────────────────────────────────────────────────
// 无穷网格
// ─────────────────────────────────────────────────────────────────

const GRID_VERTEX = /* glsl */ `
varying vec3 vWorld;
void main() {
  vec4 w = modelMatrix * vec4(position, 1.0);
  vWorld = w.xyz;
  gl_Position = projectionMatrix * viewMatrix * w;
}
`;

/**
 * 1 米一格、5 米一条粗线，按屏幕像素宽度抗锯齿，远处淡出。
 * 线宽用 fwidth 算，所以远近都是一两个像素宽，不会远处糊成一片。
 */
const GRID_FRAGMENT = /* glsl */ `
uniform vec3 camPos;
uniform float fadeFar;
uniform float opacity;
varying vec3 vWorld;
// 线条强度：按屏幕像素算线宽；线挤到一两个像素一格时整体淡掉，
// 否则远处所有线糊成一条亮带（第一版就是这样）
float lineMask(vec2 p, float step) {
  vec2 c = p / step;
  vec2 w = fwidth(c);
  vec2 g = abs(fract(c - 0.5) - 0.5) / max(w, vec2(1e-4));
  float line = 1.0 - min(min(g.x, g.y), 1.0);
  float density = max(w.x, w.y);
  return line * (1.0 - smoothstep(0.25, 0.6, density));
}
void main() {
  vec2 p = vWorld.xz;
  float minor = lineMask(p, 1.0);
  float major = lineMask(p, 5.0);
  float dist = length(vWorld.xz - camPos.xz);
  float fade = 1.0 - smoothstep(fadeFar * 0.5, fadeFar, dist);
  float a = max(minor * 0.55, major * 0.9) * fade * opacity;
  if (a < 0.01) discard;
  // 亮线 + 深色描边的感觉靠混合实现：在灰地砖上也看得清
  vec3 col = mix(vec3(0.92, 0.94, 1.0), vec3(1.0), major);
  gl_FragColor = vec4(col, a);
}
`;

export interface InfiniteGrid {
  group: THREE.Group;
  /** 每帧调：网格跟着相机平移，看起来无穷远 */
  follow(camera: THREE.Camera): void;
  setHeight(y: number): void;
  setOpacity(opacity: number): void;
  dispose(): void;
}

/** 网格淡没的距离（米） */
export const GRID_FADE_FAR = 20;

export function createInfiniteGrid(): InfiniteGrid {
  const group = new THREE.Group();
  const size = 1000;
  const material = new THREE.ShaderMaterial({
    // **20 米淡没**（10 米开始淡）。以前 240 米 —— 网格一直铺到地平线，穿过后墙和箱子
    // 画在画面正中间，看着像一块浮在半空的玻璃（2026-09-27 用户：「水平线在视图中间，而不是在地上」）。
    // 水平相机的地平线本来就在画面正中，那没错；错的是网格铺出了房间。摆人都在 20 米以内。
    uniforms: { camPos: { value: new THREE.Vector3() }, fadeFar: { value: GRID_FADE_FAR }, opacity: { value: 1 } },
    vertexShader: GRID_VERTEX,
    fragmentShader: GRID_FRAGMENT,
    transparent: true,
    depthWrite: false,
    side: THREE.DoubleSide,
  });
  const plane = new THREE.Mesh(new THREE.PlaneGeometry(size, size), material);
  plane.rotation.x = -Math.PI / 2;
  plane.position.y = 0.002;
  plane.frustumCulled = false;
  group.add(plane);

  // 坐标轴：X 红、Z 蓝铺满地面，Y 绿竖一根（和 DC 的配色一致）
  const line = (a: THREE.Vector3, b: THREE.Vector3, color: number) =>
    new THREE.Line(new THREE.BufferGeometry().setFromPoints([a, b]), new THREE.LineBasicMaterial({ color, transparent: true, opacity: 0.85 }));
  const L = size / 2;
  group.add(line(new THREE.Vector3(-L, 0.004, 0), new THREE.Vector3(L, 0.004, 0), 0xff5252));
  group.add(line(new THREE.Vector3(0, 0.004, -L), new THREE.Vector3(0, 0.004, L), 0x5a8cff));
  group.add(line(new THREE.Vector3(0, 0, 0), new THREE.Vector3(0, 30, 0), 0x59f272));

  return {
    group,
    follow(camera) {
      // 平面整格跟着相机挪，线的位置按世界坐标算所以不会跟着跑
      plane.position.x = Math.round(camera.position.x);
      plane.position.z = Math.round(camera.position.z);
      material.uniforms.camPos.value.copy(camera.position);
    },
    setHeight(y) {
      group.position.y = y;
    },
    setOpacity(opacity) {
      material.uniforms.opacity.value = Math.max(0, Math.min(1, opacity));
    },
    dispose() {
      plane.geometry.dispose();
      material.dispose();
      group.traverse((o) => {
        const l = o as THREE.Line;
        if (l.isLine) {
          l.geometry.dispose();
          (l.material as THREE.Material).dispose();
        }
      });
    },
  };
}
