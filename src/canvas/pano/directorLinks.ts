import type { CanvasEdge, CanvasNode } from "../types";
import { findUpstream } from "../utils/upstream";
import type { EpisodeData, StageSubject } from "../../api/cinema";
import type { NamedCamera, CameraPath } from "../director/scene";
/** 一条站位。导演世界算出来的带朝向和姿态；老数据 / 测试里可能只有前三项 */
type StageSubjectOut = { actor: string; zone: string; depth: string; facing?: string; posture?: string };

/**
 * 导演世界往外接的两条线：**它属于哪个场景**、**它的站位写给哪一镜**。
 *
 * 这两件事都不能从画布本身推出来 —— 画布是自由创作，它不知道剧集分镜。
 * 但把它们留空的话，导演世界就是一个「摆完看看就算了」的玩具：
 * 摆好的站位出不去，对齐好的世界校准换个节点又得重来。
 */

/**
 * 顺着连线往上找**这张全景是哪个场景的**。
 *
 * 典型接法是 `全景出图 → 图片 → 导演世界`，而 `sceneAssetId` 只在
 * 最上游那个「全景出图」节点上。中间隔着几层图片节点都要能找到。
 *
 * 遍历本身（含环保护）在 [`findUpstream`] 里 —— 分镜节点用的是同一份。
 */
export function upstreamSceneAsset(
  nodeId: string,
  nodes: CanvasNode[],
  edges: CanvasEdge[],
): { assetId: string; sceneName: string } | null {
  return findUpstream(nodeId, nodes, edges, (n) => {
    if (n.type !== "panorama") return null;
    const d = n.data as { sceneAssetId?: string; sceneName?: string };
    return d.sceneAssetId ? { assetId: d.sceneAssetId, sceneName: d.sceneName ?? "" } : null;
  });
}

/**
 * 把站位写进分集里的某一镜。**返回新的 episode，不改原来那份。**
 *
 * 找不到那一镜时返回 `null` 而不是悄悄什么都不做 —— 「点了保存、
 * 提示成功、打开一看没变」是这个项目里反复出现的那种失败。
 */
export function withBlocking(
  episode: EpisodeData,
  shotId: string,
  blocking: StageSubjectOut[],
): EpisodeData | null {
  const shots = episode.storyboard ?? [];
  if (!shots.some((s) => s.id === shotId)) return null;
  return {
    ...episode,
    storyboard: shots.map((s) =>
      s.id === shotId ? { ...s, blocking: blocking as StageSubject[] } : s,
    ),
  };
}

const legacyDirectorLine = /^\[导演机位:[^\]]+\]\s*\{.*\}$/;

/** 老版本把 JSON 塞在 camera_note 里；绑定时一次清干净，避免重复绑定累积。 */
export function withoutLegacyDirectorMarkers(note: string): string {
  return note.split("\n").filter((line) => !legacyDirectorLine.test(line)).join("\n");
}

function signed(value: number, positive: string, negative: string): string | null {
  if (Math.abs(value) < 0.5) return null;
  return `${value > 0 ? positive : negative}${Math.abs(value).toFixed(0)}°`;
}

/**
 * 机位位移按**起始镜头自己的方向**拆成前后 / 左右 / 升降。
 *
 * 只报「平移约 1.2 米」的话，推镜和横移写出来是同一句，模型只能猜。
 * 导演世界的相机约定（`scene.applyCamera`）：yaw 0 朝 −Z，yaw 为正是向右转；
 * 于是朝前 = (sin θ, −cos θ)，朝右 = (cos θ, sin θ)（x, z 平面）。
 */
function moveWords(first: { position: number[]; yawDeg: number }, last: { position: number[] }): string[] {
  const d = [0, 1, 2].map((i) => (last.position[i] ?? 0) - (first.position[i] ?? 0));
  const t = (first.yawDeg * Math.PI) / 180;
  const forward = d[0] * Math.sin(t) - d[2] * Math.cos(t);
  const right = d[0] * Math.cos(t) + d[2] * Math.sin(t);
  const up = d[1];
  const word = (v: number, pos: string, neg: string) => (Math.abs(v) < 0.05 ? null : `${v > 0 ? pos : neg}约 ${Math.abs(v).toFixed(2)} 米`);
  return [word(forward, "机位向前推", "机位向后拉"), word(right, "向右横移", "向左横移"), word(up, "升高", "降低")].filter((x): x is string => !!x);
}

/** 把几何轨迹编译成人能审阅、模型能理解的运镜句子。 */
export function directorMotionNote(camera: NamedCamera, path?: CameraPath): string {
  if (!path || path.cameras.length < 2) {
    return `固定机位，保持水平 ${camera.camera.yawDeg.toFixed(0)}°、俯仰 ${camera.camera.pitchDeg.toFixed(0)}°、FOV ${camera.camera.fovDeg.toFixed(0)}°。`;
  }
  const first = path.cameras[0];
  const last = path.cameras[path.cameras.length - 1];
  const changes = [
    ...moveWords(first, last),
    signed(last.yawDeg - first.yawDeg, "向右摇 ", "向左摇 "),
    signed(last.pitchDeg - first.pitchDeg, "向上摇 ", "向下摇 "),
    Math.abs(last.fovDeg - first.fovDeg) >= 0.5 ? `FOV 从 ${first.fovDeg.toFixed(0)}°${last.fovDeg < first.fovDeg ? "收紧" : "放宽"}到 ${last.fovDeg.toFixed(0)}°` : null,
  ].filter(Boolean);
  return `${path.durationSec.toFixed(1)} 秒内${changes.length ? changes.join("，") : "保持固定机位"}，按 ${path.cameras.length} 个关键帧平滑运动。`;
}

/** 将导演世界机位和轨迹存为结构化字段；JSON 不再混入提示词。 */
export function withDirectorCamera(episode: EpisodeData, shotId: string, camera: NamedCamera, path?: CameraPath): EpisodeData | null {
  const shots = episode.storyboard ?? [];
  if (!shots.some((s) => s.id === shotId)) return null;
  return { ...episode, storyboard: shots.map((s) => s.id === shotId ? {
    ...s,
    camera_note: withoutLegacyDirectorMarkers(s.camera_note ?? ""),
    director_camera: { name: camera.name, camera: camera.camera, path: path ?? null, motion_note: directorMotionNote(camera, path) },
  } : s) };
}

/**
 * 写之前先说清楚会覆盖掉什么。
 *
 * 那一镜原来手写过站位的话，**这次写入会整条换掉** ——
 * 不说的话用户会以为是往上加。
 */
export function overwriteNote(
  episode: EpisodeData,
  shotId: string,
  blocking: StageSubjectOut[],
): string {
  const shot = (episode.storyboard ?? []).find((s) => s.id === shotId);
  const had = shot?.blocking?.length ?? 0;
  if (!blocking.length) {
    return "这一镜画面里没有人 —— 写进去等于把原有站位清空";
  }
  if (had) {
    return `会覆盖这一镜原有的 ${had} 条站位，换成现在画面里的 ${blocking.length} 条`;
  }
  return `写入 ${blocking.length} 条站位`;
}

/** 给选择器用的一行：哪一集第几镜、画面是什么。 */
export interface ShotOption {
  id: string;
  label: string;
}

/**
 * 一集里的镜头列表。
 *
 * 标签带上**提示词开头**而不是只有编号 —— 用户记得的是画面内容，
 * 不是「第 7 镜」。这一条在分镜卡折叠态上已经验过一次。
 */
export function shotOptions(episode: EpisodeData | null): ShotOption[] {
  return (episode?.storyboard ?? []).map((s) => {
    const gist = (s.prompt || s.dialogue || "").trim().slice(0, 18);
    return { id: s.id, label: `第 ${s.shot_num} 镜${gist ? ` · ${gist}` : ""}` };
  });
}
