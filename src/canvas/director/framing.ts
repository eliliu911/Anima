/**
 * 按景别 + 机位摆机、按运镜出关键帧（导演台界面升级 U3）。
 *
 * libtv 的「添加机位」是 15 个固定按钮。我们的镜头表每一镜本来就写了景别、机位、运镜，
 * 词表（`shared/shot-language.json`）和它的预设几乎一一对应 —— 所以同一个函数既给预设按钮用，
 * 也给「按分镜摆机」用：读这一镜的 scale / camera_angle / camera_movement，对准说话的人摆好。
 *
 * 全是纯函数，坐标约定和 `scene.ts` 一致：人偶 yaw 0 正脸朝 +Z，相机 yaw 0 看 −Z、往右转为正、俯仰正 = 抬头。
 */
import { isPerson, lookAtAngles, type DirectorActor, type DirectorCamera } from "./scene";

export type ShotSize = "extreme_wide" | "wide" | "full_body" | "medium" | "medium_close" | "close_up" | "extreme_close_up";
export type ShotAngle = "eye_level" | "low_angle" | "high_angle" | "birds_eye" | "dutch" | "over_shoulder" | "pov";
export type ShotSide = "front" | "side" | "back";

/**
 * 每个景别**画面竖直方向装下多高**（米，按 1.83 米的人），和对准哪个高度。
 * 中景 = 腰以上、近景 = 胸以上、特写 = 头、大特写 = 眼睛一带。
 */
export const SIZE_FRAMING: Record<ShotSize, { frameH: number; aimH: number }> = {
  extreme_wide: { frameH: 14, aimH: 1.0 },
  wide: { frameH: 6, aimH: 1.0 },
  full_body: { frameH: 2.3, aimH: 0.95 },
  medium: { frameH: 1.15, aimH: 1.35 },
  medium_close: { frameH: 0.7, aimH: 1.52 },
  close_up: { frameH: 0.38, aimH: 1.66 },
  extreme_close_up: { frameH: 0.16, aimH: 1.7 },
};

/** 相机相对对准点的仰角（度，正 = 相机在上面往下看） */
const ELEVATION: Record<ShotAngle, number> = {
  eye_level: 0,
  low_angle: -25,
  high_angle: 30,
  birds_eye: 80,
  dutch: 0,
  over_shoulder: 0,
  pov: 0,
};

const DUTCH_ROLL = 15;
/** 相机最低离地多少（别钻到地底下） */
const MIN_CAMERA_Y = 0.15;

const rad = (d: number) => (d * Math.PI) / 180;
const r2 = (v: number) => Math.round(v * 100) / 100;

function isSize(v: string): v is ShotSize {
  return v in SIZE_FRAMING;
}
function isAngle(v: string): v is ShotAngle {
  return v in ELEVATION;
}

export interface FramingInput {
  target: DirectorActor;
  /** 过肩 / 主观要用：另一个人（从他那边看过去）。没有就退回正面 */
  other?: DirectorActor | null;
  size: string;
  angle: string;
  side?: ShotSide;
  /** 过肩从哪边肩膀 */
  shoulder?: "left" | "right";
  /** 竖直视场角（three 的 fov 是竖直的） */
  fovDeg: number;
}

/** 这个人某个高度的点 */
function point(a: DirectorActor, h: number): [number, number, number] {
  return [a.position[0], a.position[1] + h * a.scale, a.position[2]];
}

/** 人脸朝向（水平单位向量） */
function faceDir(a: DirectorActor): [number, number] {
  const r = rad(a.yawDeg);
  return [Math.sin(r), Math.cos(r)];
}

function camAt(pos: [number, number, number], aim: [number, number, number], fovDeg: number, rollDeg = 0): DirectorCamera {
  const p: [number, number, number] = [r2(pos[0]), r2(Math.max(MIN_CAMERA_Y, pos[1])), r2(pos[2])];
  return { position: p, ...lookAtAngles(p, aim), fovDeg, rollDeg };
}

/** 按景别和机位摆一个机位，对准 `target`。认不出的景别按中景、认不出的机位按平视 */
export function framingCamera(input: FramingInput): DirectorCamera {
  const size: ShotSize = isSize(input.size) ? input.size : "medium";
  const angle: ShotAngle = isAngle(input.angle) ? input.angle : "eye_level";
  const { target, other, fovDeg } = input;
  const f = SIZE_FRAMING[size];
  const aim = point(target, f.aimH);
  const dist = (f.frameH * target.scale) / 2 / Math.tan(rad(fovDeg) / 2);

  // 过肩：站在另一个人身后、偏到一侧肩膀，从他肩膀上看过去
  if (angle === "over_shoulder" && other) {
    const dx = other.position[0] - target.position[0];
    const dz = other.position[2] - target.position[2];
    const len = Math.hypot(dx, dz) || 1;
    const back: [number, number] = [dx / len, dz / len];
    // 「左肩」= 站在他身后看过去时他的左边
    const lat = input.shoulder === "right" ? -1 : 1;
    const right: [number, number] = [-back[1], back[0]];
    const eye = point(other, 1.62);
    const pos: [number, number, number] = [eye[0] + back[0] * 0.7 + right[0] * 0.38 * lat, eye[1] + 0.05, eye[2] + back[1] * 0.7 + right[1] * 0.38 * lat];
    return camAt(pos, aim, fovDeg);
  }
  // 主观：另一个人的眼睛看过去
  if (angle === "pov" && other) {
    return camAt(point(other, 1.66), point(target, 1.6), fovDeg);
  }

  const [fx, fz] = faceDir(target);
  const turn = input.side === "back" ? Math.PI : input.side === "side" ? Math.PI / 2 : 0;
  const dir: [number, number] = [fx * Math.cos(turn) + fz * Math.sin(turn), -fx * Math.sin(turn) + fz * Math.cos(turn)];
  const e = rad(ELEVATION[angle]);
  const pos: [number, number, number] = [aim[0] + dir[0] * dist * Math.cos(e), aim[1] + dist * Math.sin(e), aim[2] + dir[1] * dist * Math.cos(e)];
  return camAt(pos, aim, fovDeg, angle === "dutch" ? DUTCH_ROLL : 0);
}

/** 预设机位（照 libtv 的「添加机位」，换成我们的词表） */
export interface CameraPreset {
  label: string;
  size: ShotSize;
  angle: ShotAngle;
  side?: ShotSide;
  shoulder?: "left" | "right";
  fovDeg?: number;
}
export const CAMERA_PRESETS: CameraPreset[] = [
  { label: "正面中景", size: "medium", angle: "eye_level" },
  { label: "正面近景", size: "medium_close", angle: "eye_level" },
  { label: "正面特写", size: "close_up", angle: "eye_level" },
  { label: "正面全景", size: "full_body", angle: "eye_level" },
  { label: "侧面中景", size: "medium", angle: "eye_level", side: "side" },
  { label: "侧面近景", size: "medium_close", angle: "eye_level", side: "side" },
  { label: "背面中景", size: "medium", angle: "eye_level", side: "back" },
  { label: "俯拍全景", size: "full_body", angle: "high_angle" },
  { label: "45° 俯拍", size: "medium", angle: "high_angle" },
  { label: "低角度仰拍", size: "medium", angle: "low_angle" },
  { label: "低角度广角", size: "full_body", angle: "low_angle", fovDeg: 85 },
  { label: "过肩（左）", size: "medium_close", angle: "over_shoulder", shoulder: "left" },
  { label: "过肩（右）", size: "medium_close", angle: "over_shoulder", shoulder: "right" },
  { label: "鸟瞰", size: "wide", angle: "birds_eye" },
  { label: "荷兰角", size: "medium", angle: "dutch" },
];

/**
 * 按运镜从一个机位出关键帧。`shot` = 这一镜落定的画面（推镜的终点、拉镜的起点）。
 * 固定、跟拍、手持这类出不了两帧的返回 `null`（保持单个机位）。
 */
export function motionKeyframes(shot: DirectorCamera, aim: [number, number, number], move: string): DirectorCamera[] | null {
  const yaw = rad(shot.yawDeg);
  const fwd: [number, number, number] = [Math.sin(yaw) * Math.cos(rad(shot.pitchDeg)), Math.sin(rad(shot.pitchDeg)), -Math.cos(yaw) * Math.cos(rad(shot.pitchDeg))];
  const right: [number, number] = [Math.cos(yaw), Math.sin(yaw)];
  const d = Math.hypot(aim[0] - shot.position[0], aim[1] - shot.position[1], aim[2] - shot.position[2]) || 3;
  const at = (p: [number, number, number], extra: Partial<DirectorCamera> = {}): DirectorCamera => ({ ...shot, position: [r2(p[0]), r2(Math.max(MIN_CAMERA_Y, p[1])), r2(p[2])], ...extra });
  const back = (k: number) => at([shot.position[0] - fwd[0] * d * k, shot.position[1] - fwd[1] * d * k, shot.position[2] - fwd[2] * d * k]);
  const side = (m: number) => at([shot.position[0] + right[0] * m, shot.position[1], shot.position[2] + right[1] * m]);
  const orbitAt = (deg: number, dy = 0) => {
    const dx = shot.position[0] - aim[0];
    const dz = shot.position[2] - aim[2];
    const c = Math.cos(rad(deg));
    const s = Math.sin(rad(deg));
    const p: [number, number, number] = [aim[0] + dx * c - dz * s, shot.position[1] + dy, aim[2] + dx * s + dz * c];
    const q: [number, number, number] = [r2(p[0]), r2(Math.max(MIN_CAMERA_Y, p[1])), r2(p[2])];
    return { ...shot, position: q, ...lookAtAngles(q, aim) };
  };
  switch (move) {
    case "push_in":
      return [back(0.35), shot];
    case "pull_out":
      return [shot, back(0.35)];
    case "truck_left":
      return [shot, side(-1)];
    case "truck_right":
      return [shot, side(1)];
    case "pan_left":
      return [shot, { ...shot, yawDeg: shot.yawDeg - 25 }];
    case "pan_right":
      return [shot, { ...shot, yawDeg: shot.yawDeg + 25 }];
    case "tilt_up":
      return [shot, { ...shot, pitchDeg: Math.min(89, shot.pitchDeg + 15) }];
    case "tilt_down":
      return [shot, { ...shot, pitchDeg: Math.max(-89, shot.pitchDeg - 15) }];
    case "pedestal_up":
      return [shot, at([shot.position[0], shot.position[1] + 0.8, shot.position[2]])];
    case "pedestal_down":
      return [shot, at([shot.position[0], shot.position[1] - 0.8, shot.position[2]])];
    case "zoom_in":
      return [shot, { ...shot, fovDeg: r2(shot.fovDeg * 0.6) }];
    case "zoom_out":
      return [{ ...shot, fovDeg: r2(shot.fovDeg * 0.6) }, shot];
    case "dolly_zoom":
      return [back(0.35), { ...shot, fovDeg: r2(Math.min(110, shot.fovDeg * 1.4)) }];
    case "orbit":
      return [orbitAt(-25), shot, orbitAt(25)];
    case "orbit_up":
      return [orbitAt(-25), shot, orbitAt(25, 0.8)];
    case "orbit_down":
      return [orbitAt(-25, 0.8), shot, orbitAt(25)];
    default:
      return null;
  }
}

/** 按分镜摆机时对准谁：第一个开口说话、而且在场的人；没台词就用出场人物里第一个在场的；都没有就第一个人偶 */
export function pickShotTarget(
  actors: DirectorActor[],
  shot: { dialogue_lines?: { speaker: string; off_screen?: boolean; line?: string }[]; character_ids?: string[] },
): { target: DirectorActor | null; other: DirectorActor | null } {
  // 只在角色里挑：群演和道具不是这一镜的主体
  actors = actors.filter(isPerson);
  const byName = (name: string) => actors.find((a) => a.label.trim() === name.trim() || a.label.split("（")[0] === name.split("（")[0]);
  let target: DirectorActor | undefined;
  for (const l of shot.dialogue_lines ?? []) {
    if (l.off_screen || !l.line?.trim()) continue;
    target = byName(l.speaker);
    if (target) break;
  }
  if (!target) target = (shot.character_ids ?? []).map((id) => actors.find((a) => a.assetId === id)).find(Boolean);
  if (!target) target = actors[0];
  const other = target ? actors.find((a) => a.id !== target!.id) ?? null : null;
  return { target: target ?? null, other };
}

/**
 * **全景片场锁在拍摄点时**的摆机：不挪相机，只转向 + 选视场角，让画面正好装下这个景别。
 *
 * 全景只在拍摄点准（`world.ts`），挪开墙和家具就拉花 —— 所以预设 / 按分镜摆机在这种片场里
 * 不能像 3D 片场那样把相机搬到人面前。仰拍、俯拍、侧面这些要换位置的出不来，按平视摆（荷兰角照样横滚）。
 */
export function framingFromPoint(pos: [number, number, number], input: Omit<FramingInput, "fovDeg">): DirectorCamera {
  const size: ShotSize = isSize(input.size) ? input.size : "medium";
  const f = SIZE_FRAMING[size];
  const aim = point(input.target, f.aimH);
  const dist = Math.hypot(aim[0] - pos[0], aim[1] - pos[1], aim[2] - pos[2]) || 1;
  const fovDeg = r2(Math.max(10, Math.min(110, (2 * Math.atan((f.frameH * input.target.scale) / 2 / dist) * 180) / Math.PI)));
  return { position: pos, ...lookAtAngles(pos, aim), fovDeg, rollDeg: input.angle === "dutch" ? DUTCH_ROLL : 0 };
}

/**
 * 锁在拍摄点时的运镜：能挪位置的换成等效的不挪位置的 —— 推 / 拉 → 变焦、移 → 摇、升降 → 上下摇、环绕 → 摇。
 */
export function motionKeyframesAtPoint(shot: DirectorCamera, move: string): DirectorCamera[] | null {
  const wide = { ...shot, fovDeg: r2(Math.min(110, shot.fovDeg * 1.5)) };
  switch (move) {
    case "push_in":
    case "dolly_zoom":
      return [wide, shot];
    case "pull_out":
      return [shot, wide];
    case "truck_left":
      return motionKeyframes(shot, [0, 0, 0], "pan_left");
    case "truck_right":
    case "orbit":
    case "orbit_up":
    case "orbit_down":
      return motionKeyframes(shot, [0, 0, 0], "pan_right");
    case "pedestal_up":
      return motionKeyframes(shot, [0, 0, 0], "tilt_up");
    case "pedestal_down":
      return motionKeyframes(shot, [0, 0, 0], "tilt_down");
    case "pan_left":
    case "pan_right":
    case "tilt_up":
    case "tilt_down":
    case "zoom_in":
    case "zoom_out":
      return motionKeyframes(shot, [0, 0, 0], move);
    default:
      return null;
  }
}
