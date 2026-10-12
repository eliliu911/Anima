/**
 * 场景里的一个机位：一个 SVG 摄像机图标 + 取景框线（照 libtv 的机位图标）。
 *
 * 用户 2026-09-28：「当前摄像机只有几何线描述摄像机，不够明确。我需要你添加一个 SVG 的摄像机图片，
 * 类似 libtv 那样的照相机，最好也增加三维坐标轴可调节，类似人偶的调节方式，并可控制相机颜色。」
 *
 * - 图标是**始终朝着屏幕、固定像素大小**的精灵：拉远了也看得见、点得中；
 * - 图标是侧面画的摄像机，镜头朝右 —— 取景框在屏幕上朝左时图标跟着镜像，看起来就是「镜头朝这边」；
 * - 取景框线仍然在：它才说明机位真正朝哪、视场多宽；
 * - `pivot` 放在机位点上、按机位转好 —— 三维坐标轴（TransformControls）挂在它上面拖。
 */
import * as THREE from "three";
import type { DirectorCamera } from "./scene";

/** 机位颜色的几个常用色（右栏色块）。第一个是默认，照 libtv 的橙色 */
export const CAMERA_COLORS = ["#FF9F1C", "#FFC82E", "#5AB4FF", "#4ADE80", "#F472B6", "#A78BFA", "#F87171", "#E5E7EB"];
export const DEFAULT_CAMERA_COLOR = CAMERA_COLORS[0];

export interface CameraMarkerInput {
  id: string;
  camera: DirectorCamera;
  /** 画幅宽高比 */
  aspect: number;
  active: boolean;
  selected: boolean;
  color?: string;
  /** 不画（机位视角里正透过它看的那个机位：图标和取景框就在眼睛上） */
  hidden?: boolean;
}

const HEX = /^#[0-9a-fA-F]{6}$/;
export function cameraColor(c: string | undefined | null): string {
  return c && HEX.test(c) ? c.toUpperCase() : DEFAULT_CAMERA_COLOR;
}

/** 颜色调暗一点（描边、镜头筒用） */
function shade(hex: string, k: number): string {
  const n = parseInt(hex.slice(1), 16);
  const ch = (v: number) => Math.max(0, Math.min(255, Math.round(v * k))).toString(16).padStart(2, "0");
  return `#${ch((n >> 16) & 255)}${ch((n >> 8) & 255)}${ch(n & 255)}`;
}

/**
 * 摄像机图标（侧面、镜头朝右）：机身 + 顶上两个片盘 + 镜头遮光罩。
 * 选中时白色描边；当前机位左上角一个红点（录制灯）。
 */
export function cameraIconSvg(color: string, selected: boolean, active: boolean): string {
  const c = cameraColor(color);
  const dark = shade(c, 0.55);
  const stroke = selected ? "#FFFFFF" : dark;
  const sw = selected ? 4 : 2.5;
  return (
    `<svg xmlns="http://www.w3.org/2000/svg" width="192" height="128" viewBox="0 0 96 64">` +
    `<circle cx="26" cy="15" r="11" fill="${c}" stroke="${stroke}" stroke-width="${sw}"/>` +
    `<circle cx="26" cy="15" r="3.5" fill="${dark}"/>` +
    `<circle cx="50" cy="15" r="11" fill="${c}" stroke="${stroke}" stroke-width="${sw}"/>` +
    `<circle cx="50" cy="15" r="3.5" fill="${dark}"/>` +
    `<rect x="10" y="26" width="54" height="30" rx="6" fill="${c}" stroke="${stroke}" stroke-width="${sw}"/>` +
    `<path d="M64 33 L88 24 L88 58 L64 49 Z" fill="${shade(c, 0.8)}" stroke="${stroke}" stroke-width="${sw}" stroke-linejoin="round"/>` +
    `<rect x="17" y="33" width="20" height="4" rx="2" fill="${dark}"/>` +
    (active ? `<circle cx="54" cy="36" r="4.5" fill="#FF3B30" stroke="#FFFFFF" stroke-width="1.5"/>` : "") +
    `</svg>`
  );
}

const textures = new Map<string, THREE.Texture>();
function iconTexture(color: string, selected: boolean, active: boolean): THREE.Texture {
  const key = `${cameraColor(color)}|${selected}|${active}`;
  let t = textures.get(key);
  if (!t) {
    // 没有 DOM（单元测试跑在 node 里）就给一张空贴图 —— 图标只是看的，几何和逻辑照样能测
    t = typeof document === "undefined"
      ? new THREE.Texture()
      : new THREE.TextureLoader().load(`data:image/svg+xml,${encodeURIComponent(cameraIconSvg(color, selected, active))}`);
    t.colorSpace = THREE.SRGBColorSpace;
    textures.set(key, t);
  }
  return t;
}

/** 图标高度（占视口高度的比例，精灵不随距离缩放） */
const ICON_H = 0.05;
const ICON_ASPECT = 96 / 64;
/** 取景框画多远（米） */
const FRUSTUM_D = 0.7;

export interface CameraMarkerObject {
  id: string;
  /** 放在机位点、按机位转好；坐标轴挂这里 */
  pivot: THREE.Object3D;
  /** 点选用 */
  sprite: THREE.Sprite;
  /** `keepPose`：正在拖这个机位的坐标轴 —— 位置和朝向都别被 React 回写打回去 */
  update(m: CameraMarkerInput, opts?: { keepPose?: boolean }): void;
  /** 每帧：取景框在屏幕上朝左时把图标镜像 */
  faceScreen(view: THREE.Camera): void;
  dispose(): void;
}

export function createCameraMarker(input: CameraMarkerInput): CameraMarkerObject {
  const pivot = new THREE.Object3D();
  const lineMat = new THREE.LineBasicMaterial({ transparent: true, depthTest: false });
  const lines = new THREE.LineSegments(new THREE.BufferGeometry(), lineMat);
  lines.renderOrder = 10;
  pivot.add(lines);
  const spriteMat = new THREE.SpriteMaterial({ sizeAttenuation: false, depthTest: false, transparent: true, side: THREE.DoubleSide });
  const sprite = new THREE.Sprite(spriteMat);
  sprite.renderOrder = 11;
  sprite.userData.cameraId = input.id;
  pivot.add(sprite);
  let shape = "";
  let flip = false;
  let scale = ICON_H;

  const obj: CameraMarkerObject = {
    id: input.id,
    pivot,
    sprite,
    update(m, opts) {
      const cam = m.camera;
      pivot.visible = !m.hidden;
      if (!opts?.keepPose) {
        pivot.position.set(...cam.position);
        pivot.rotation.set((cam.pitchDeg * Math.PI) / 180, (-cam.yawDeg * Math.PI) / 180, ((cam.rollDeg ?? 0) * Math.PI) / 180, "YXZ");
      }
      const nextShape = `${cam.fovDeg}|${m.aspect}`;
      if (nextShape !== shape) {
        shape = nextShape;
        const h = FRUSTUM_D * Math.tan((cam.fovDeg * Math.PI) / 360);
        const w = h * m.aspect;
        const c = [[-w, -h], [w, -h], [w, h], [-w, h]].map(([x, y]) => new THREE.Vector3(x, y, -FRUSTUM_D));
        const o = new THREE.Vector3();
        // 机位点连四角 + 画幅框 + 顶上小三角（看得出横滚）
        const pts = [o, c[0], o, c[1], o, c[2], o, c[3], c[0], c[1], c[1], c[2], c[2], c[3], c[3], c[0],
          new THREE.Vector3(-w * 0.3, h * 1.08, -FRUSTUM_D), new THREE.Vector3(0, h * 1.4, -FRUSTUM_D),
          new THREE.Vector3(0, h * 1.4, -FRUSTUM_D), new THREE.Vector3(w * 0.3, h * 1.08, -FRUSTUM_D)];
        lines.geometry.dispose();
        lines.geometry = new THREE.BufferGeometry().setFromPoints(pts);
      }
      const color = cameraColor(m.color);
      lineMat.color.set(m.selected ? "#FFFFFF" : color);
      lineMat.opacity = m.selected || m.active ? 0.95 : 0.7;
      spriteMat.map = iconTexture(color, m.selected, m.active);
      spriteMat.needsUpdate = true;
      scale = ICON_H * (m.selected ? 1.25 : 1);
      sprite.scale.set((flip ? -1 : 1) * scale * ICON_ASPECT, scale, 1);
    },
    faceScreen(view) {
      const o = pivot.getWorldPosition(new THREE.Vector3());
      const f = new THREE.Vector3(0, 0, -1).applyQuaternion(pivot.getWorldQuaternion(new THREE.Quaternion())).add(o);
      const a = o.project(view);
      const b = f.project(view);
      const next = b.x < a.x - 1e-4;
      if (next !== flip) {
        flip = next;
        sprite.scale.x = (flip ? -1 : 1) * scale * ICON_ASPECT;
      }
    },
    dispose() {
      lines.geometry.dispose();
      lineMat.dispose();
      spriteMat.dispose();
    },
  };
  obj.update(input);
  return obj;
}
