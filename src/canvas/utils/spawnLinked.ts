/**
 * **从一个节点「长出」一个连好线的新节点** —— 右侧「+」、左侧「+」、拖线松手菜单三处共用（2026-10-10 用户：
 * 「目前节点只有右侧有'+'，左侧没做功能。另外点击'+'拖出连线时，如果旁边没有其他节点，需要弹出新增节点的菜单。如 libtv」）。
 *
 * - `down`：新节点接在它**后面**（它的输出 → 新节点的输入）。右侧「+」、从输出口拖线松手
 * - `up`：新节点接在它**前面**（新节点的输出 → 它的输入）。左侧「+」、从输入口拖线松手
 *
 * 三处以前各写各的：右侧「+」自己拼连线、写死 `targetHandle: "prompt-in"`，也不走 `onConnect` ——
 * 图片接视频节点时「挂成参考图」那条规则就漏了。这里只算「接什么、怎么连」，连线由调用方交给 `onConnect`。
 */
import type { CanvasNodeType } from "../types";

/** 每种节点的**输入口** id（节点组件里 `type="target"` 的那个）。没有的不能往前接 */
export const INPUT_HANDLE: Partial<Record<CanvasNodeType, string>> = {
  prompt: "prompt-in",
  imageGen: "prompt-in",
  videoGen: "prompt-in",
  upscale: "prompt-in",
  depthCapture: "prompt-in",
  character: "char-in",
  skill: "skill-in",
  audio: "text-in",
  shot: "context-in",
  shotContext: "context-in",
  panorama: "pano-in",
  panoViewer: "pano-view-in",
  directorWorld: "dw-in",
};

/** 每种节点的**输出口** id（`BaseNodeWrapper` 的 `sourceHandleId`） */
export const OUTPUT_HANDLE: Partial<Record<CanvasNodeType, string>> = {
  prompt: "prompt-out",
  imageGen: "image-out",
  videoGen: "video-out",
  upscale: "upscale-out",
  depthCapture: "depth-out",
  character: "char-out",
  skill: "skill-out",
  audio: "audio-out",
  audioClip: "audio-out",
  shot: "context-out",
  shotContext: "context-out",
  panorama: "pano-out",
  panoViewer: "pano-view-out",
  directorWorld: "dw-out",
  lora: "lora-out",
};

export type SpawnDirection = "up" | "down";

export interface SpawnChoice {
  type: CanvasNodeType;
  label: string;
  hint?: string;
  /** 补帧不是新建空节点，是直接起一个补帧任务（`spawnVideoTool`），见调用方 */
  action?: "interpolate";
}

/** 视频节点出了片、或者超分节点放大的是视频（由调用方算好传进来，这里不读 store） */
export interface SpawnContext {
  type: CanvasNodeType | string | undefined;
  /** 上游是一段视频（视频节点 / 放大视频的超分） */
  isVideo?: boolean;
  /** 已经有视频产物（补帧要它） */
  hasVideo?: boolean;
}

/**
 * 能长出哪些节点。
 *
 * - **往后接**：和以前右侧「+」那份一样 —— 音频后面只能接视频（当参考音频）；深度动作只接在视频后面；超分接在图 / 角色 / 视频 / 超分后面；补帧要已经有视频
 * - **往前接**：只给真能连进去的 —— 视频节点吃文本、图片（挂成参考图）、视频（参考视频）、音频（参考音频）；
 *   图片节点吃文本、图片（改图）；文本节点吃文本；超分、深度动作吃图片 / 视频；别的输入口只给文本
 */
export function spawnChoices(direction: SpawnDirection, ctx: SpawnContext): SpawnChoice[] {
  const t = ctx.type;
  if (direction === "down") {
    if (!t || !OUTPUT_HANDLE[t as CanvasNodeType]) return [];
    const audio = t === "audioClip" || t === "audio";
    const out: SpawnChoice[] = [];
    if (!audio) out.push({ type: "prompt", label: "文本" }, { type: "imageGen", label: "图片" });
    out.push({ type: "videoGen", label: "视频" });
    // 音频后面接音频 = 声音复制（照 LibTV）
    if (audio) out.push({ type: "audio", label: "音频", hint: "用这段的嗓子念新的词（声音复制）" });
    if (ctx.isVideo) out.push({ type: "depthCapture", label: "深度动作捕捉", hint: "抽出这段视频里的动作，下一步接视频节点 + 人物图做动作迁移" });
    if (["imageGen", "videoGen", "character", "upscale"].includes(String(t))) out.push({ type: "upscale", label: "超分 ×2", hint: "宽高各放大一倍、细节更锐（RealESRGAN）" });
    if (ctx.hasVideo) out.push({ type: "upscale", label: "补帧 ×2", hint: "帧率翻倍、动作更顺（RIFE），时长和声音不变 · 建好马上跑", action: "interpolate" });
    return out;
  }
  if (!t || !INPUT_HANDLE[t as CanvasNodeType]) return [];
  const text: SpawnChoice = { type: "prompt", label: "文本" };
  const image: SpawnChoice = { type: "imageGen", label: "图片" };
  const video: SpawnChoice = { type: "videoGen", label: "视频" };
  switch (t) {
    case "videoGen":
      return [text, { ...image, hint: "连进来挂成参考图" }, { ...video, hint: "连进来当参考视频" }, { type: "audio", label: "音频", hint: "连进来当参考音频（H3）" }];
    case "audio":
      return [text, { type: "audio", label: "音频", hint: "连进来当参考 = 声音复制" }];
    case "imageGen":
      return [text, { ...image, hint: "连进来改这张图" }];
    case "upscale":
    case "depthCapture":
      return [image, video];
    default:
      return [text];
  }
}

/** 新节点和它怎么连（交给 `onConnect`）。连不上（某一头没有口）回 `null` */
export function linkFor(
  direction: SpawnDirection,
  from: { id: string; type: CanvasNodeType | string | undefined; handle?: string | null },
  created: { id: string; type: CanvasNodeType },
): { source: string; target: string; sourceHandle: string | null; targetHandle: string | null } | null {
  if (direction === "down") {
    const targetHandle = INPUT_HANDLE[created.type];
    const sourceHandle = from.handle?.replace(/-plus$/, "") || OUTPUT_HANDLE[from.type as CanvasNodeType];
    if (!targetHandle || !sourceHandle) return null;
    return { source: from.id, target: created.id, sourceHandle, targetHandle };
  }
  const sourceHandle = OUTPUT_HANDLE[created.type];
  const targetHandle = from.handle?.replace(/-plus$/, "") || INPUT_HANDLE[from.type as CanvasNodeType];
  if (!sourceHandle || !targetHandle) return null;
  return { source: created.id, target: from.id, sourceHandle, targetHandle };
}

/** 新节点放哪：往后接放右边、往前接放左边，和它上沿对齐；拖线松手的放在松手处（节点竖着居中在鼠标上） */
export function spawnPosition(
  direction: SpawnDirection,
  from: { position: { x: number; y: number }; width?: number },
  newWidth: number,
  drop?: { x: number; y: number },
): { x: number; y: number } {
  if (drop) return { x: direction === "down" ? drop.x : drop.x - newWidth, y: drop.y - 60 };
  const gap = 60;
  return direction === "down"
    ? { x: from.position.x + (from.width || 440) + gap, y: from.position.y }
    : { x: from.position.x - newWidth - gap, y: from.position.y };
}

/** 新建节点的大致宽度（和 `CanvasView.handleAddNode` 同一张表） */
export function nodeWidthOf(type: CanvasNodeType): number {
  return ["videoGen", "imageGen", "shot", "shotContext", "skill", "panorama", "panoViewer", "directorWorld"].includes(type) ? 440 : 320;
}

export interface Rect { x: number; y: number; w: number; h: number }

const overlaps = (a: Rect, b: Rect, pad: number) => a.x < b.x + b.w + pad && b.x < a.x + a.w + pad && a.y < b.y + b.h + pad && b.y < a.y + a.h + pad;

/**
 * 放的地方压着别的节点就往下挪，直到空出来（最多挪 `tries` 次，挪不开就放原处）。
 * 2026-10-10 实测：左侧「+」长出来的文本节点盖在旁边两个节点上 —— 以前右侧「+」也是只算「右边 60 像素」、不看那里有没有东西
 */
export function avoidOverlap(pos: { x: number; y: number }, size: { w: number; h: number }, others: Rect[], step = 40, tries = 60, pad = 24): { x: number; y: number } {
  let y = pos.y;
  for (let i = 0; i < tries; i++) {
    const me = { x: pos.x, y, w: size.w, h: size.h };
    const hit = others.find((o) => overlaps(me, o, pad));
    if (!hit) return { x: pos.x, y };
    // 直接跳到挡住它的那个节点下面，比一格一格挪快
    y = Math.max(y + step, hit.y + hit.h + pad);
  }
  return pos;
}
