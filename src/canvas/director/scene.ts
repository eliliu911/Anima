/**
 * 导演世界的场景数据：谁、在哪、什么姿势，以及从机位算出来的站位。
 *
 * 数据形状参考 DramaClaw（ClaymoreLab, Elastic License 2.0）
 * frontend/src/features/viewer-kit/three-d/directorManifest.ts 的 DirectorFrameMeta，
 * 画幅框裁切参考 engine/viewerApp.ts 的 calculateFrameCaptureRect。
 * 出处规则见 DECISIONS.md 2026-09-16。
 *
 * # 导演世界是拿来干什么的
 *
 * 1. 有场景（全景）
 * 2. 在场景里摆人偶：几个人、各自的位置、朝向、姿势
 * 3. 导出到画布：合成图 + 纯背景 + 每个人偶的元数据
 * 4. 给每个人偶挂上角色图出图 —— **位置和动作按人偶，长相按角色图**
 *
 * **颜色 = 身份**：人偶颜色来自分集颜色表（`src-tauri/src/actor_colors.rs`），
 * 下游靠颜色认出「画面里那个品红的是谁」。
 *
 * # 为什么从「方位角 + 距离」改成 xyz
 *
 * 第一版相机钉在球心，只能转不能走，所以用方位角记位置。DC 的导演台相机能飞、
 * 人能拖能转能缩放 —— 那就得是真正的三维坐标。单位按米，地面 y = 0，
 * 人偶站直约 1.83 米。全景里没有可信的绝对尺度，米只是「和人偶一样大的那个米」。
 */
import * as THREE from "three";
import { isPoseName, poseLabel, type PoseName } from "./poses";

export interface DirectorActor {
  id: string;
  /** 绑定的角色名，或「匿名人物 N」。绑了角色就要和 `[角色:名字]` 逐字一致 */
  label: string;
  /** `#RRGGBB`。主线模式下来自分集颜色表，锁定 */
  color: string;
  /** 颜色表里的角色资产 id。匿名人物没有 */
  assetId?: string;
  /** 脚底中心，米 */
  position: [number, number, number];
  /**
   * 朝向。**0 = 正脸朝 +Z**（人偶模型原样的朝向）。
   * 相机默认在原点看向 -Z，所以摆在相机前面、yaw 0 的人偶是正脸对镜头。
   */
  yawDeg: number;
  scale: number;
  pose: PoseName;
  /**
   * 是什么（导演台界面升级 U3.5）。没有 = 人物。
   * - `prop` 道具：画成一块立牌，上面是道具资产的图（没图就是一个方块）；
   * - `extra` 群演：灰色人偶，共用一个颜色，不绑角色，输出时一起换成路人。
   * 道具和群演不参加撞色 / 没绑角色的检查，也不写进分镜站位。
   */
  kind?: "person" | "prop" | "extra";
  /** 道具的图（本机路径）。画立牌用 */
  imagePath?: string;
}

/** 群演共用的颜色：调色盘外的中性灰，和任何角色都撞不上 */
export const CROWD_COLOR = "#9E9E9E";
/** 道具立牌的描边色 */
export const PROP_COLOR = "#FFB000";

/** 是不是真正的角色（道具、群演都不算） */
export function isPerson(a: DirectorActor): boolean {
  return !a.kind || a.kind === "person";
}

/**
 * 在一个点周围铺一片群演（照 libtv「群众 3×3」）：`cols × rows` 个灰色人偶，间距 `spacing` 米。
 * 列沿画面左右、行往纵深排（按相机看过去的方向，不是世界坐标轴），大致面向相机，
 * 位置、朝向、高矮稍微错开 —— 整整齐齐一排排站着不像人群。
 */
export function crowdGrid(
  center: [number, number, number],
  camPos: [number, number, number],
  existing: DirectorActor[],
  cols = 3,
  rows = 3,
  spacing = 1.2,
): DirectorActor[] {
  const base = existing.filter((a) => a.kind === "extra").length;
  // 相机 → 中心的水平方向；相机正好站在中心上就当看向 -Z
  let fx = center[0] - camPos[0];
  let fz = center[2] - camPos[2];
  const len = Math.hypot(fx, fz);
  if (len < 1e-6) {
    fx = 0;
    fz = -1;
  } else {
    fx /= len;
    fz /= len;
  }
  const rx = -fz;
  const rz = fx;
  const out: DirectorActor[] = [];
  let n = 0;
  for (let r = 0; r < rows; r++) {
    for (let c = 0; c < cols; c++) {
      n += 1;
      const jitter = ((n * 37) % 11) / 11 - 0.5;
      const across = (c - (cols - 1) / 2) * spacing + jitter * 0.3;
      const deep = (r - (rows - 1) / 2) * spacing - jitter * 0.3;
      const pos: [number, number, number] = [
        round2(center[0] + rx * across + fx * deep),
        center[1],
        round2(center[2] + rz * across + fz * deep),
      ];
      out.push({
        id: `extra-${Date.now()}-${n}-${Math.random().toString(36).slice(2, 5)}`,
        label: `群演 ${base + n}`,
        color: CROWD_COLOR,
        kind: "extra",
        position: pos,
        yawDeg: round2(facingCameraYaw(pos, camPos) + jitter * 40),
        scale: round2(0.94 + (((n * 53) % 13) / 13) * 0.12),
        pose: "standing",
      });
    }
  }
  return out;
}

export interface DirectorCamera {
  position: [number, number, number];
  /** 0 = 看向 -Z，往右转为正 */
  yawDeg: number;
  /** 正 = 抬头 */
  pitchDeg: number;
  fovDeg: number;
  /** 横滚（荷兰角），正 = 画面顺时针歪。没有 = 0。老数据没这个字段 */
  rollDeg?: number;
}

/**
 * 一个机位。导演视角里画成一个取景框，能选中、改属性（界面升级 U2）。
 * `lookAt` = 注视的人物 id：那个人挪了，机位自动重新对准他胸口。
 */
export interface NamedCamera {
  id: string;
  name: string;
  camera: DirectorCamera;
  lookAt?: string | null;
  /** 场景里机位图标和取景框的颜色（`#RRGGBB`）。没有 = 默认橙色（cameraMarker.ts） */
  color?: string;
}
export interface CameraPath { id: string; name: string; durationSec: number; cameras: DirectorCamera[] }

export function interpolateCamera(a: DirectorCamera, b: DirectorCamera, t: number): DirectorCamera {
  const k = Math.max(0, Math.min(1, t));
  const lerp = (x: number, y: number) => x + (y - x) * k;
  let yawDelta = ((b.yawDeg - a.yawDeg + 540) % 360) - 180;
  return { position: [lerp(a.position[0], b.position[0]), lerp(a.position[1], b.position[1]), lerp(a.position[2], b.position[2])], yawDeg: a.yawDeg + yawDelta * k, pitchDeg: lerp(a.pitchDeg,b.pitchDeg), fovDeg: lerp(a.fovDeg,b.fovDeg), rollDeg: lerp(a.rollDeg ?? 0, b.rollDeg ?? 0) };
}

export type FrameAspect = "16:9" | "2:3" | "9:16" | "1:1" | "4:3";

export const FRAME_ASPECTS: { id: FrameAspect; ratio: number }[] = [
  { id: "16:9", ratio: 16 / 9 },
  { id: "2:3", ratio: 2 / 3 },
  { id: "9:16", ratio: 9 / 16 },
  { id: "1:1", ratio: 1 },
  { id: "4:3", ratio: 4 / 3 },
];

export function aspectRatio(a: string): number {
  return FRAME_ASPECTS.find((x) => x.id === a)?.ratio ?? 16 / 9;
}

export const DEFAULT_EYE_HEIGHT = 1.6;

export function defaultCamera(eyeHeight = DEFAULT_EYE_HEIGHT): DirectorCamera {
  return { position: [0, eyeHeight, 0], yawDeg: 0, pitchDeg: -8, fovDeg: 70 };
}

/** 「匿名人物 3」—— 取最大编号 + 1，删过中间的也不会重名 */
export function nextAnonymousLabel(actors: DirectorActor[]): string {
  let max = 0;
  for (const a of actors) {
    const m = a.label.match(/^匿名人物 (\d+)$/);
    if (m) max = Math.max(max, Number(m[1]));
  }
  return `匿名人物 ${max + 1}`;
}

function round2(v: number) {
  return Math.round(v * 100) / 100;
}

function normalize180(v: number) {
  return ((v + 540) % 360) - 180;
}

/**
 * 旧版标记（方位角 + 距离，相机在球心）→ 人偶。
 *
 * - 旧版距离单位是「身位」× 校准尺度，这里按 1 身位 = 1 米换
 * - 朝向设成面朝相机
 * - **道具标记丢掉** —— 这一轮只放人（2026-09-16 定）
 */
export function migrateMarkers(
  markers: { id: string; kind: string; label: string; yawDeg: number; distance: number; color: string }[],
  scale = 1,
): DirectorActor[] {
  return markers
    .filter((m) => m.kind === "actor")
    .map((m) => {
      const rad = (m.yawDeg * Math.PI) / 180;
      const d = Math.max(0.5, m.distance) * scale;
      const x = round2(Math.sin(rad) * d);
      const z = round2(-Math.cos(rad) * d);
      return {
        id: m.id,
        label: m.label,
        color: m.color,
        position: [x, 0, z] as [number, number, number],
        yawDeg: facingCameraYaw([x, 0, z], [0, 0, 0]),
        scale: 1,
        pose: "standing" as PoseName,
      };
    });
}

/** 站在 `at`、正脸朝向 `target` 的 yaw（水平面上） */
export function facingCameraYaw(at: [number, number, number], target: [number, number, number]): number {
  const dx = target[0] - at[0];
  const dz = target[2] - at[2];
  if (Math.hypot(dx, dz) < 1e-9) return 0;
  // 正脸方向 (sin yaw, cos yaw)
  return round2(normalize180((Math.atan2(dx, dz) * 180) / Math.PI));
}

/** 从节点数据里读出人偶，坏数据丢掉而不是整个崩 */
export function readActors(raw: unknown): DirectorActor[] {
  if (!Array.isArray(raw)) return [];
  return raw.flatMap((o) => {
    if (!o || typeof o !== "object") return [];
    const x = o as Partial<DirectorActor>;
    const pos =
      Array.isArray(x.position) && x.position.length === 3 && x.position.every((v) => Number.isFinite(v))
        ? (x.position as [number, number, number])
        : null;
    if (!x.id || !pos) return [];
    const actor: DirectorActor = {
      id: String(x.id),
      label: String(x.label ?? ""),
      color: /^#[0-9a-fA-F]{6}$/.test(String(x.color)) ? String(x.color) : "#FFFFFF",
      position: pos,
      yawDeg: Number.isFinite(x.yawDeg) ? Number(x.yawDeg) : 0,
      scale: Number(x.scale) > 0 ? Number(x.scale) : 1,
      pose: isPoseName(x.pose) ? x.pose : "standing",
    };
    if (x.assetId) actor.assetId = String(x.assetId);
    if (x.kind === "prop" || x.kind === "extra") actor.kind = x.kind;
    if (typeof x.imagePath === "string" && x.imagePath) actor.imagePath = x.imagePath;
    return [actor];
  });
}

export function readCamera(raw: unknown, eyeHeight = DEFAULT_EYE_HEIGHT): DirectorCamera {
  const d = defaultCamera(eyeHeight);
  if (!raw || typeof raw !== "object") return d;
  const c = raw as Partial<DirectorCamera>;
  const pos =
    Array.isArray(c.position) && c.position.length === 3 && c.position.every((v) => Number.isFinite(v))
      ? (c.position as [number, number, number])
      : d.position;
  const num = (v: unknown, fb: number) => (Number.isFinite(v) ? Number(v) : fb);
  return {
    position: pos,
    yawDeg: num(c.yawDeg, d.yawDeg),
    pitchDeg: Math.max(-89, Math.min(89, num(c.pitchDeg, d.pitchDeg))),
    fovDeg: Math.max(10, Math.min(120, num(c.fovDeg, d.fovDeg))),
    rollDeg: Math.max(-45, Math.min(45, num(c.rollDeg, 0))),
  };
}

/** 存下来的机位列表 → 校验过的机位（坏的丢掉）。画布节点和工作区读存档都走它 */
/** 新机位叫什么：「机位 N」里第一个没被占的 N（删过一个再新建不会撞名） */
export function nextCameraName(cams: { name: string }[]): string {
  const used = new Set(cams.map((c) => c.name.trim()));
  let n = 1;
  while (used.has(`机位 ${n}`)) n++;
  return `机位 ${n}`;
}

export function readNamedCameras(raw: unknown, eyeHeight = DEFAULT_EYE_HEIGHT): NamedCamera[] {
  if (!Array.isArray(raw)) return [];
  return raw
    .filter((c): c is { id: string; name?: unknown; camera?: unknown; lookAt?: unknown; color?: unknown } => !!c && typeof c === "object" && typeof (c as { id?: unknown }).id === "string")
    .map((c) => ({
      id: c.id,
      name: typeof c.name === "string" && c.name.trim() ? c.name : "机位",
      camera: readCamera(c.camera, eyeHeight),
      lookAt: typeof c.lookAt === "string" ? c.lookAt : null,
      ...(typeof c.color === "string" && /^#[0-9a-fA-F]{6}$/.test(c.color) ? { color: c.color.toUpperCase() } : {}),
    }));
}

/**
 * 导演视角的起始位置：站在机位身后 3 米、高 1.2 米，稍微往下看 —— 一进来就能看到机位和它对着的东西。
 */
export function directorViewFrom(shot: DirectorCamera): DirectorCamera {
  const r = (shot.yawDeg * Math.PI) / 180;
  const fwd = [Math.sin(r), -Math.cos(r)];
  return {
    position: [round2(shot.position[0] - fwd[0] * 3), round2(shot.position[1] + 1.2), round2(shot.position[2] - fwd[1] * 3)],
    yawDeg: shot.yawDeg,
    pitchDeg: -15,
    fovDeg: 70,
    rollDeg: 0,
  };
}

// ─────────────────────────────────────────────────────────────────
// 画幅框
// ─────────────────────────────────────────────────────────────────

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/**
 * 视口里按画幅居中放一个最大的框，四周留 `padding`。
 *
 * **截图和界面上画的框必须是同一个框** —— 两边各算一遍的话，
 * 人偶在框边上时截出来会被切掉或多出一截，而界面上看着是对的。
 */
export function frameRect(viewW: number, viewH: number, aspect: string, padding = 0): Rect {
  const w = Math.max(1, viewW - padding * 2);
  const h = Math.max(1, viewH - padding * 2);
  const r = aspectRatio(aspect);
  let fw = w;
  let fh = w / r;
  if (fh > h) {
    fh = h;
    fw = h * r;
  }
  return {
    x: Math.round((viewW - fw) / 2),
    y: Math.round((viewH - fh) / 2),
    width: Math.round(fw),
    height: Math.round(fh),
  };
}

// ─────────────────────────────────────────────────────────────────
// 站位：从机位算出每个人在画面哪儿
// ─────────────────────────────────────────────────────────────────

/** 和 `api/cinema.ts` 的 `StageSubject` 对齐 */
export interface StageSubjectOut {
  actor: string;
  zone: string;
  depth: string;
  facing: string;
  posture: string;
}

/** 和引擎同一个相机约定：先绕 Y 再绕 X，yaw 往右为正 */
export function applyCamera(c: THREE.PerspectiveCamera, cam: DirectorCamera, aspect: number) {
  c.fov = cam.fovDeg;
  c.aspect = aspect;
  c.position.set(...cam.position);
  c.rotation.set((cam.pitchDeg * Math.PI) / 180, (-cam.yawDeg * Math.PI) / 180, ((cam.rollDeg ?? 0) * Math.PI) / 180, "YXZ");
  c.updateProjectionMatrix();
  c.updateMatrixWorld(true);
}

/**
 * 一个人偶在**画幅框**里的位置。取身体中心（脚底往上 0.9 米 × 缩放）投影。
 *
 * `aspect` 是画幅的宽高比：截图只截框内，所以站位也按框算，不按整个视口算。
 */
export function projectActor(a: DirectorActor, cam: DirectorCamera, aspect: number) {
  const c = new THREE.PerspectiveCamera(cam.fovDeg, aspect, 0.01, 1000);
  applyCamera(c, cam, aspect);
  const world = new THREE.Vector3(a.position[0], a.position[1] + 0.9 * a.scale, a.position[2]);
  const view = world.clone().applyMatrix4(c.matrixWorldInverse);
  const ndc = world.clone().project(c);
  const inFrame = view.z < 0 && Math.abs(ndc.x) <= 1 && Math.abs(ndc.y) <= 1;
  const distance = Math.hypot(a.position[0] - cam.position[0], a.position[2] - cam.position[2]);
  return { inFrame, ndcX: ndc.x, ndcY: ndc.y, distance };
}

/** 画面左中右。中间三分之一算中央 —— 三分法 */
export function zoneOf(ndcX: number): string {
  if (ndcX < -1 / 3) return "画面左侧";
  if (ndcX > 1 / 3) return "画面右侧";
  return "画面中央";
}

/** 离相机多远。和分镜 `StageSubject.depth` 同一套词 */
export function depthOf(distance: number): string {
  if (distance <= 2.5) return "前景";
  if (distance <= 6) return "中景";
  return "后景";
}

/** 人物朝向相对镜头：正面 / 背面 / 侧面（±50° 以内算正 / 背） */
export function facingOf(a: DirectorActor, cam: DirectorCamera): string {
  const r = (a.yawDeg * Math.PI) / 180;
  const fx = Math.sin(r);
  const fz = Math.cos(r);
  const tx = cam.position[0] - a.position[0];
  const tz = cam.position[2] - a.position[2];
  const len = Math.hypot(tx, tz);
  if (len < 1e-6) return "正面";
  const cos = (fx * tx + fz * tz) / len;
  const lim = Math.cos((50 * Math.PI) / 180);
  if (cos > lim) return "正面";
  if (cos < -lim) return "背面";
  return "侧面";
}

/**
 * 这一机位的站位，直接就是分镜要的那份。**画面外的不写** ——
 * 写了等于告诉模型去画一个它看不见的人。从左到右排。
 */
export function blockingFor(actors: DirectorActor[], cam: DirectorCamera, aspect: number): StageSubjectOut[] {
  return actors
    .filter((a) => isPerson(a) && a.label.trim())
    .map((a) => ({ a, p: projectActor(a, cam, aspect) }))
    .filter(({ p }) => p.inFrame)
    .sort((x, y) => x.p.ndcX - y.p.ndcX)
    .map(({ a, p }) => ({
      actor: a.label.trim(),
      zone: zoneOf(p.ndcX),
      depth: depthOf(p.distance),
      facing: facingOf(a, cam),
      posture: poseLabel(a.pose),
    }));
}

export function blockingSentence(rows: StageSubjectOut[]): string {
  if (!rows.length) return "这一镜画面里没有人";
  return rows.map((r) => `${r.actor}在${r.zone}${r.depth}，${r.facing}，${r.posture}`).join("；");
}

// ─────────────────────────────────────────────────────────────────
// 控制帧元数据
// ─────────────────────────────────────────────────────────────────

/**
 * 跟着合成图走，说清**哪个颜色是谁、在画面哪儿、什么姿势**。
 * 第 4 步（挂角色图出图）读的就是这份。字段对照 DC 的 `DirectorFrameMeta.actors`。
 */
export interface DirectorFrameMeta {
  schema_version: "anima_director_frame_v2";
  source:
    | { type: "pano360"; url: string | null }
    | { type: "splat"; path: string; format: "ply" | "ksplat" | "spz" };
  /** 分集上下文。自由模式为 null */
  context: { project_id: string; episode_id: string; shot_id: string | null } | null;
  camera: DirectorCamera & { frame_aspect: string };
  actors: {
    id: string;
    label: string;
    asset_id: string | null;
    /** person / prop / extra（界面升级 U3.5）。老数据没有 = person */
    role: "person" | "prop" | "extra";
    /** 道具立牌的图 */
    image_path: string | null;
    marker_color: string;
    color_name: string | null;
    pose: PoseName;
    pose_label: string;
    position: [number, number, number];
    yaw_deg: number;
    scale: number;
    in_frame: boolean;
    zone: string | null;
    depth: string;
    facing: string;
    /** 身体中心在画幅框里的位置，0–1，左上角为原点。画面外为 null */
    screen: { x: number; y: number } | null;
  }[];
  blocking: StageSubjectOut[];
}

export function buildFrameMeta(args: {
  sourceUrl: string | null;
  splat?: { path: string; format: "ply" | "ksplat" | "spz" } | null;
  context: DirectorFrameMeta["context"];
  aspect: string;
  camera: DirectorCamera;
  actors: DirectorActor[];
  colorNames?: Record<string, string>;
}): DirectorFrameMeta {
  const { camera, actors } = args;
  const r = aspectRatio(args.aspect);
  const round3 = (v: number) => Math.round(v * 1000) / 1000;
  return {
    schema_version: "anima_director_frame_v2",
    source: args.splat
      ? { type: "splat", path: args.splat.path, format: args.splat.format }
      : { type: "pano360", url: args.sourceUrl },
    context: args.context,
    camera: { ...camera, frame_aspect: args.aspect },
    actors: actors.map((a) => {
      const p = projectActor(a, camera, r);
      return {
        id: a.id,
        label: a.label,
        asset_id: a.assetId ?? null,
        role: a.kind ?? "person",
        image_path: a.imagePath ?? null,
        marker_color: a.color,
        color_name: args.colorNames?.[a.color.toUpperCase()] ?? null,
        pose: a.pose,
        pose_label: poseLabel(a.pose),
        position: a.position,
        yaw_deg: Math.round(a.yawDeg),
        scale: a.scale,
        in_frame: p.inFrame,
        zone: p.inFrame ? zoneOf(p.ndcX) : null,
        depth: depthOf(p.distance),
        facing: facingOf(a, camera),
        screen: p.inFrame ? { x: round3((p.ndcX + 1) / 2), y: round3((1 - p.ndcY) / 2) } : null,
      };
    }),
    blocking: blockingFor(actors, camera, r),
  };
}

// ─────────────────────────────────────────────────────────────────
// 相机预设
// ─────────────────────────────────────────────────────────────────

/** 从 `from` 看向 `target` 的 yaw / pitch（和 `DirectorCamera` 同一约定） */
export function lookAtAngles(from: [number, number, number], target: [number, number, number]) {
  const dx = target[0] - from[0];
  const dy = target[1] - from[1];
  const dz = target[2] - from[2];
  const flat = Math.hypot(dx, dz);
  return {
    yawDeg: round2((Math.atan2(dx, -dz) * 180) / Math.PI),
    pitchDeg: round2((Math.atan2(dy, Math.max(1e-9, flat)) * 180) / Math.PI),
  };
}

/**
 * 相机切到人偶正面 / 背后 / 原地看向它。距离和抬高取自 DC 的
 * cameraFaceSelected（1.8 米）/ cameraBehindSelected（1.6 米），视线对准胸口。
 */
export function cameraPreset(
  a: DirectorActor,
  kind: "face" | "behind" | "look",
  current: DirectorCamera,
): DirectorCamera {
  const chest: [number, number, number] = [a.position[0], a.position[1] + 1.0 * a.scale, a.position[2]];
  if (kind === "look") return { ...current, ...lookAtAngles(current.position, chest) };
  const r = (a.yawDeg * Math.PI) / 180;
  const face = [Math.sin(r), Math.cos(r)];
  const dist = kind === "face" ? 1.8 : -1.6;
  const pos: [number, number, number] = [
    round2(chest[0] + face[0] * dist * a.scale),
    round2(chest[1] + 0.45 * a.scale),
    round2(chest[2] + face[1] * dist * a.scale),
  ];
  return { ...current, position: pos, ...lookAtAngles(pos, chest) };
}

// ─────────────────────────────────────────────────────────────────
// 输出前的检查
// ─────────────────────────────────────────────────────────────────

/**
 * 输出前必须解决的问题。**非空就不许输出 / 写进分镜。**
 *
 * 颜色 = 身份，所以这两种情况下游一定会认错人：
 * - 两个人同色 —— 自由模式下手动选成一样的，或者切到主线模式之前摆的匿名人物
 *   恰好和本集某个角色同色
 * - 主线模式下有没绑定角色的人 —— 颜色表里查不到他是谁
 */
export function actorProblems(
  actors: DirectorActor[],
  mainline: boolean,
  camera?: { cam: DirectorCamera; eyeHeight: number; hasPano: boolean },
): string[] {
  const out: string[] = [];
  if (camera?.hasPano) {
    const d = offsetFromCapturePoint(camera.cam, camera.eyeHeight);
    if (d > CAPTURE_POINT_TOLERANCE) {
      out.push(`镜头离开拍摄点 ${d.toFixed(1)} 米 —— 全景只有地面是准的，墙和家具会拉花。先点「回到拍摄点」`);
    }
  }
  // 道具、群演不参加：群演本来就共用一个颜色，道具靠图认
  const people = actors.filter(isPerson);
  const byColor = new Map<string, DirectorActor[]>();
  for (const a of people) {
    const k = a.color.toUpperCase();
    byColor.set(k, [...(byColor.get(k) ?? []), a]);
  }
  for (const [color, list] of byColor) {
    if (list.length > 1) {
      out.push(`${list.map((a) => `「${a.label}」`).join("和")}同色（${color}），下游分不清谁是谁`);
    }
  }
  if (mainline) {
    for (const a of people) {
      if (!a.assetId) out.push(`「${a.label}」没有绑定本集角色 —— 选中它，在「身份」里选一个，或者删掉`);
    }
  }
  return out;
}

/**
 * 把一个人绑定到颜色表里的角色：名字、颜色、资产 id 一起换。
 * 那个角色已经被别人占了就返回错误 —— 同一个人出现两次也是撞色。
 */
export function bindIdentity(
  actors: DirectorActor[],
  actorId: string,
  who: { asset_id: string; name: string; color: string },
): { actors: DirectorActor[] } | { error: string } {
  const taken = actors.find((a) => a.id !== actorId && a.assetId === who.asset_id);
  if (taken) return { error: `「${who.name}」已经在场了` };
  return {
    actors: actors.map((a) =>
      a.id === actorId ? { ...a, label: who.name, color: who.color, assetId: who.asset_id } : a,
    ),
  };
}

/** 相机离拍摄点多远（米）。360 全景来源下超过 `CAPTURE_POINT_TOLERANCE` 截出来的背景就是变形的 */
export function offsetFromCapturePoint(cam: DirectorCamera, eyeHeight: number): number {
  return Math.hypot(cam.position[0], cam.position[1] - eyeHeight, cam.position[2]);
}

export const CAPTURE_POINT_TOLERANCE = 0.3;

/**
 * splat 放进导演世界时的旋转 / 位置（交给 gaussian-splats-3d 的 `addSplatScene`）。
 *
 * **SHARP 的输出是原相机坐标系**：原点是拍摄点，+Z 朝前、+Y 朝下（OpenCV）。
 * 2026-09-27 实跑拆机间核对过：所有点 z ∈ [1.85, 6.75]；脚下的拆机台 y ≈ +0.8~1.05，
 * 上方的墙和顶 y 到 −2.2。导演世界是 three 的约定：相机朝 −Z、+Y 朝上，默认机位在 (0, 眼高, 0)。
 * 不转的话片场**在相机背后、上下颠倒、沉在地面以下** —— 载入成功、不报错，只是什么都看不对。
 *
 * 绕 X 转 180°（四元数 [1,0,0,0]：(x,y,z) → (x,−y,−z)），再把原点抬到眼高。
 * 不知道坐标系的（手动载入）原样放。
 */
export function splatPlacement(
  convention: "opencv_camera" | undefined,
  eyeHeight: number,
  scale = 1,
): { rotation?: [number, number, number, number]; position?: [number, number, number]; scale?: [number, number, number] } {
  // 缩放以原点（拍摄点）为中心：机位视角看到的画面不变，只有片场相对人偶 / 网格变大变小
  const sc = scale !== 1 && scale > 0 ? { scale: [scale, scale, scale] as [number, number, number] } : {};
  if (convention !== "opencv_camera") return sc;
  return { rotation: [1, 0, 0, 0], position: [0, eyeHeight, 0], ...sc };
}
