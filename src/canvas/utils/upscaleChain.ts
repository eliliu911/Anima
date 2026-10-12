import type { CanvasNode, CanvasEdge, CharacterNodeData, ImageGenNodeData, UpscaleNodeData, VideoGenNodeData } from "../types";
import { pathFromMediaUrl } from "../../components/studio/MediaThumb";

/**
 * 超分节点的连线规则：
 *
 * ```text
 * [图片 / 角色 / 视频] ──→ [超分 ×2] ──→ 下游照常接（出图、出片、深度动作捕捉……）
 * ```
 *
 * 图片几秒；视频逐帧 RealESRGAN，每 1 秒片子约 25 秒（2026-09-14 实测）。
 * 长边已经 ≥ 1800 的视频后端会拒收 —— 再放大一倍太大了。
 */
export type MediaKind = "image" | "video";

const VIDEO_EXT = /\.(mp4|mov|m4v|webm|mkv)$/i;

export function mediaKind(path: string): MediaKind {
  return VIDEO_EXT.test(path.split("?")[0]) ? "video" : "image";
}

/** 一个节点能给下游的图或视频。没有就是 null */
export function nodeMedia(node: CanvasNode | undefined): { path: string; kind: MediaKind } | null {
  if (!node) return null;
  const local = (url: unknown) => (typeof url === "string" && url ? pathFromMediaUrl(url) ?? url : null);
  if (node.type === "videoGen") {
    const p = local((node.data as VideoGenNodeData).outputVideoUrl);
    return p ? { path: p, kind: "video" } : null;
  }
  if (node.type === "imageGen") {
    const d = node.data as ImageGenNodeData;
    const p = local(d.outputImageUrl || d.references?.[0]?.path);
    return p ? { path: p, kind: "image" } : null;
  }
  if (node.type === "character") {
    const p = local((node.data as CharacterNodeData).avatarUrl);
    return p ? { path: p, kind: "image" } : null;
  }
  if (node.type === "upscale") {
    const p = local((node.data as UpscaleNodeData).outputUrl);
    return p ? { path: p, kind: mediaKind(p) } : null;
  }
  return null;
}

/** 超分节点吃的是什么：直接连进来的那一个节点的图或视频。 */
export function upscaleSource(nodeId: string, nodes: CanvasNode[], edges: CanvasEdge[]): { path?: string; kind?: MediaKind; reason?: string } {
  const sources = edges
    .filter((e) => e.target === nodeId)
    .map((e) => nodes.find((n) => n.id === e.source))
    .filter((n): n is CanvasNode => Boolean(n) && ["imageGen", "videoGen", "character", "upscale"].includes(n!.type as string));
  if (sources.length === 0) return { reason: "左边连一张图或一段视频进来" };
  if (sources.length > 1) return { reason: `连了 ${sources.length} 个节点，一次只能放大一个` };
  const media = nodeMedia(sources[0]);
  if (!media) return { reason: "上游节点里还没有图或视频 —— 先上传或生成" };
  return media;
}

/** 超分节点的输出过期了没有：上游换了图 / 视频，结果还是旧的。 */
export function upscaleStale(node: CanvasNode, nodes: CanvasNode[], edges: CanvasEdge[]): boolean {
  const d = node.data as UpscaleNodeData;
  const want = upscaleSource(node.id, nodes, edges).path;
  return Boolean(d.outputUrl) && Boolean(want) && d.sourcePath !== want;
}
