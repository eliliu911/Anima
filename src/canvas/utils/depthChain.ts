import type { CanvasNode, CanvasEdge, DepthCaptureNodeData, VideoGenNodeData } from "../types";
import { pathFromMediaUrl } from "../../components/studio/MediaThumb";
import { nodeMedia } from "./upscaleChain";

/**
 * 深度动作链的连线规则（照 libtv）：
 *
 * ```text
 * [视频] ──→ [深度动作捕捉] ──→ [视频：动作迁移]
 *                                   ↑
 *                     [人物图] ─────┘
 * ```
 */

/** 直接连进来的视频节点（上传的、生成的、超分节点出的视频）。还没有视频的也算 —— 连线已经说明了意图 */
export function upstreamVideoNodes(nodeId: string, nodes: CanvasNode[], edges: CanvasEdge[]): CanvasNode[] {
  return edges
    .filter((e) => e.target === nodeId)
    .map((e) => nodes.find((n) => n.id === e.source))
    // 超分节点出的视频也算视频源
    .filter((n): n is CanvasNode => Boolean(n) && (n!.type === "videoGen" || (n!.type === "upscale" && nodeMedia(n!)?.kind !== "image")));
}

/** 深度节点吃的是哪段视频：直接连进来的那个视频节点当前选中的视频。 */
export function depthSourceVideo(depthNodeId: string, nodes: CanvasNode[], edges: CanvasEdge[]): { path?: string; reason?: string } {
  return videoSourceOf(depthNodeId, nodes, edges);
}

/** 某个节点直接连进来的那段视频。深度节点、原片复刻（视频直连出片）共用 */
export function videoSourceOf(nodeId: string, nodes: CanvasNode[], edges: CanvasEdge[]): { path?: string; reason?: string } {
  const sources = upstreamVideoNodes(nodeId, nodes, edges);
  if (sources.length === 0) return { reason: "先连一个视频节点进来 —— 上传一段视频，或者用已经出好的视频" };
  if (sources.length > 1) return { reason: `连了 ${sources.length} 个视频节点，只能从一段视频里捕捉动作` };
  const media = sources[0].type === "videoGen"
    ? (() => { const url = (sources[0].data as VideoGenNodeData).outputVideoUrl; return url ? { path: pathFromMediaUrl(url) ?? url } : null; })()
    : nodeMedia(sources[0]);
  if (!media) return { reason: "上游视频节点里还没有视频 —— 先上传或生成一段" };
  return { path: media.path };
}

/** 视频节点上游有没有深度动作捕捉节点。有的话这个视频节点就是「动作迁移」。 */
export function upstreamDepth(videoNodeId: string, nodes: CanvasNode[], edges: CanvasEdge[]): { nodeId: string; depthPath?: string; sourceVideoPath?: string; stale: boolean } | null {
  const node = edges
    .filter((e) => e.target === videoNodeId)
    .map((e) => nodes.find((n) => n.id === e.source))
    .find((n) => n?.type === "depthCapture");
  if (!node) return null;
  const data = node.data as DepthCaptureNodeData;
  const current = depthSourceVideo(node.id, nodes, edges).path;
  const depthPath = data.depthVideoUrl ? pathFromMediaUrl(data.depthVideoUrl) ?? data.depthVideoUrl : undefined;
  // 上游换了视频、深度还是旧的：要重新捕捉，不能拿旧动作出片
  const stale = Boolean(depthPath) && Boolean(current) && data.sourceVideoPath !== current;
  return { nodeId: node.id, depthPath, sourceVideoPath: data.sourceVideoPath, stale };
}

/** 每条路一次最长取几秒。和 Rust `depth_capture::MAX_SECS_*` 一致 */
export const MOTION_MAX_SECS = { ltx: 10, h3: 10, h3_ref: 10 } as const;
/** 原片复刻 · 快最短 2.5 秒：WeeTodd 节点的下限。和 Rust `MIN_SECS_H3_REF` 一致 */
export const MIN_SECS_H3_REF = 2.5;

/**
 * 原片复刻：原视频本身当参考，不用深度。
 * `h3_native` 是删掉了的原生那档（2026-09-24，连模型一起删），老画布里存着的也算原片复刻、按「原片复刻 · 快」跑。
 */
export const isRefMotion = (engine?: string) => engine === "h3_ref" || engine === "h3_native";

export type MotionEngine = "ltx" | "h3" | "h3_ref";

/**
 * 实际走哪一档。**原视频直接连出片节点（没有深度节点）时只有原片复刻两档** ——
 * 另外两档要深度。（直连时 `inferVideoMode` 只在已经选了原片复刻时才推成动作迁移，这里的回落是兜底。）
 */
export function effectiveMotionEngine(engine: string | undefined, viaDepth: boolean): MotionEngine {
  if (isRefMotion(engine)) return "h3_ref";  // 含老的 h3_native
  if (viaDepth) return (engine as MotionEngine | undefined) ?? "ltx";
  return "h3_ref";
}

/**
 * 动作迁移按节点设置里的清晰度、时长出（以前固定 768×512 / 5 秒，下面的设置不起作用）。
 * 和 Rust `motion_size` / `motion_secs` 同一张表，节点上据此写明实际会出多大、多长。
 *
 * - 动作精准（LTX Union，宽高要能被 128 整除）：480P 768×512、720P 1152×768、1080P 1536×1024
 * - 人物清晰（H3）：480P 768×512、720P 1280×736；没有 1080P，按 720P
 *
 * 尺寸按横屏写，竖屏视频宽高对调（横竖跟源视频走）。
 */
export function motionSpec(engine: MotionEngine, resolution: string, durationSecs?: number) {
  const res = resolution.toUpperCase();
  // 原片复刻（原生 / 快）固定原作的 0.5 MP（960×544，和 Rust `H3N_LANDSCAPE` 同一个数），不跟清晰度走
  if (engine === "h3_ref") {
    const want = durationSecs && durationSecs > 0 ? durationSecs : 5;
    const max = MOTION_MAX_SECS[engine];
    return { tier: "0.5 MP", size: "960×544", secs: Math.max(MIN_SECS_H3_REF, Math.min(want, max)), clipped: want > max, downgraded: false };
  }
  const [tier, w, h] =
    engine === "h3"
      ? res === "720P" || res === "1080P" ? ["720P", 1280, 736] : ["480P", 768, 512]
      : res === "1080P" ? ["1080P", 1536, 1024] : res === "720P" ? ["720P", 1152, 768] : ["480P", 768, 512];
  const max = MOTION_MAX_SECS[engine];
  const want = durationSecs && durationSecs > 0 ? durationSecs : 5;
  return {
    tier: tier as string,
    size: `${w}×${h}`,
    secs: Math.max(1, Math.min(want, max)),
    clipped: want > max,
    downgraded: tier !== res,
  };
}
