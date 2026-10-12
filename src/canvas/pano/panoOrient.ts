/**
 * 全景球的朝向换算：「锁定当前视角」和「看接缝」。
 *
 * 和 `panoMath` 分开是因为这两件要做真正的旋转合成，得用 three 的四元数 ——
 * 而 `panoMath` 刻意保持零依赖。
 *
 * # 约定（必须和 `panoStage` 完全一致，改一边另一边要跟着改）
 *
 * - 相机：`camera.rotation.set(pitch, -yawTotal, 0, "YXZ")`，`yawTotal = yaw + frontYaw`
 * - 球面校正：`sphere.rotation.set(pitch, yaw, roll, "YXZ")`
 * - 相机默认看 -Z
 */
import * as THREE from "three";
import { normalizeDeg } from "./panoMath";
import type { SphereCorrectionDeg } from "./panoStage";

const RAD = Math.PI / 180;

function cameraQuat(yawTotalDeg: number, pitchDeg: number): THREE.Quaternion {
  return new THREE.Quaternion().setFromEuler(new THREE.Euler(pitchDeg * RAD, -yawTotalDeg * RAD, 0, "YXZ"));
}

function sphereQuat(c: SphereCorrectionDeg): THREE.Quaternion {
  return new THREE.Quaternion().setFromEuler(new THREE.Euler(c.pitch * RAD, c.yaw * RAD, c.roll * RAD, "YXZ"));
}

/** 相机在这个朝向下看到的是**贴图上**哪个方向（球的局部坐标）。给测试核对用。 */
export function textureDirectionAt(c: SphereCorrectionDeg, yawTotalDeg: number, pitchDeg: number): THREE.Vector3 {
  const world = new THREE.Vector3(0, 0, -1).applyQuaternion(cameraQuat(yawTotalDeg, pitchDeg));
  return world.applyQuaternion(sphereQuat(c).invert());
}

/**
 * 锁定当前视角：把「现在看的方向」烘进球面校正，之后 yaw 0 / pitch 0 / 正前方偏移 0
 * 看到的就是现在这个画面。
 *
 * **不能像 DramaClaw 那样把角度直接加到校正上。** 校正是 YXZ 欧拉角，
 * 相机有俯仰时两次旋转不可交换，直接相加之后回正看到的不是原来那个点 ——
 * 仰头 40° 锁定，回正后会偏出去十几度。这里按四元数合成：
 * 新校正 = 相机旋转⁻¹ × 旧校正。
 */
export function lockViewCorrection(
  c: SphereCorrectionDeg,
  yawTotalDeg: number,
  pitchDeg: number,
): SphereCorrectionDeg {
  const q = cameraQuat(yawTotalDeg, pitchDeg).invert().multiply(sphereQuat(c));
  const e = new THREE.Euler().setFromQuaternion(q, "YXZ");
  const round = (v: number) => Math.round(v * 100) / 100;
  return {
    pitch: round(e.x / RAD),
    yaw: round(normalizeDeg(e.y / RAD)),
    roll: round(e.z / RAD),
  };
}

/**
 * 转去看贴图的左右接缝，返回**显示用的** yaw（已扣掉正前方偏移）和 pitch。
 *
 * 等距圆柱贴图的左右边在球上是同一条经线，在球的局部 +Z（`makeSphereGeometry`
 * 把贴图正中转到了 -Z，接缝就在正后方）。生成的全景这里最容易有断层，
 * 所以要能一键转过去检查。
 *
 * DramaClaw 的 seam 是「正前方 -180°」，那其实就是正后方，和贴图接缝无关 ——
 * 校过正前方之后它指的就不是接缝了。这里按贴图算，校正、偏移怎么改都指得准。
 */
export function seamView(c: SphereCorrectionDeg, frontYawDeg: number): { yawDeg: number; pitchDeg: number } {
  const d = new THREE.Vector3(0, 0, 1).applyQuaternion(sphereQuat(c));
  const pitch = Math.asin(Math.max(-1, Math.min(1, d.y))) / RAD;
  // 相机方向 = (cos p · sin y, sin p, -cos p · cos y)
  const yawTotal = Math.atan2(d.x, -d.z) / RAD;
  return {
    yawDeg: Math.round(normalizeDeg(yawTotal - frontYawDeg) * 100) / 100,
    pitchDeg: Math.round(pitch * 100) / 100,
  };
}

// ─────────────────────────────────────────────────────────────────
// 小行星（球极投影）
// ─────────────────────────────────────────────────────────────────

/** 小行星模式的视场角范围。球极投影超过 180° 才看得到「整颗星球」 */
export const PLANET_FOV_MIN = 90;
export const PLANET_FOV_MAX = 330;
export const PLANET_FOV_DEFAULT = 270;

export function clampPlanetFov(fovDeg: number): number {
  return Math.max(PLANET_FOV_MIN, Math.min(PLANET_FOV_MAX, fovDeg));
}

/**
 * 球的局部方向 → 等距圆柱贴图坐标。**必须和 `makeSphereGeometry` 的 uv 一致**
 * （测试逐顶点核对）：u=0.5 在 -Z、往 +X 变大、接缝在 +Z；v=1 在正上方。
 * 小行星的着色器里是同一个公式的 GLSL 版。
 */
export function equirectUv(d: THREE.Vector3): { u: number; v: number } {
  const n = d.clone().normalize();
  return {
    u: 0.5 + Math.atan2(n.x, -n.z) / (2 * Math.PI),
    v: 1 - Math.acos(Math.max(-1, Math.min(1, n.y))) / Math.PI,
  };
}

/** 球极投影的缩放：屏幕上下边缘（半高 = 1）正好落在视场角的一半。 */
export function stereoScale(fovDeg: number): number {
  return 2 * Math.tan(((clampPlanetFov(fovDeg) / 4) * Math.PI) / 180);
}

/**
 * 屏幕上一点 → 相机空间里的视线方向（球极投影）。
 *
 * `px, py` 以屏幕中心为原点，**半高 = 1**（左右是 ±宽高比）。
 * 和普通透视的区别：透视是 r = tan θ，到 90° 就无穷大；球极投影是 r = 2·tan(θ/2)，
 * 180° 以外也画得出来，而且**局部不变形**（小圆还是圆）—— 这就是小行星
 * 不像 DramaClaw 那样「超广角朝下看、边缘拉成长条」的原因。
 */
export function stereoRay(px: number, py: number, fovDeg: number): THREE.Vector3 {
  const s = stereoScale(fovDeg);
  const x = px * s;
  const y = py * s;
  const r = Math.hypot(x, y);
  if (r < 1e-9) return new THREE.Vector3(0, 0, -1);
  const th = 2 * Math.atan(r / 2);
  return new THREE.Vector3((Math.sin(th) * x) / r, (Math.sin(th) * y) / r, -Math.cos(th));
}

/**
 * 相机空间 → 球局部空间的旋转。着色器里用它的矩阵版，这里给测试和 `planetUv` 用。
 * 和 `textureDirectionAt` 同一套约定。
 */
export function cameraToSphere(c: SphereCorrectionDeg, yawTotalDeg: number, pitchDeg: number): THREE.Quaternion {
  return sphereQuat(c).invert().multiply(cameraQuat(yawTotalDeg, pitchDeg));
}

/** 小行星模式下屏幕上一点看到的贴图坐标。着色器的 JS 对照版。 */
export function planetUv(
  px: number,
  py: number,
  view: { correction: SphereCorrectionDeg; yawTotalDeg: number; pitchDeg: number; fovDeg: number },
): { u: number; v: number } {
  const ray = stereoRay(px, py, view.fovDeg).applyQuaternion(
    cameraToSphere(view.correction, view.yawTotalDeg, view.pitchDeg),
  );
  return equirectUv(ray);
}
