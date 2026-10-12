import type { CanvasEdge, CanvasNode, ShotNodeData } from "../types";
import { findUpstream } from "./upstream";

/** 一张画布产物要收回哪一镜。 */
export interface ShotTarget {
  projectId: string;
  episodeId: string;
  shotId: string;
  label: string;
}

/**
 * 顺着连线往上找**这张图是给哪一镜出的**。
 *
 * 典型接法是 `分镜 → 图片`，但中间常常隔着技能节点、镜头上下文、
 * 甚至几层图生图 —— 只看直接上游的话，用户多接一层「收为首帧」就消失了，
 * 而且不给任何理由。
 */
export function upstreamShot(
  nodeId: string,
  nodes: CanvasNode[],
  edges: CanvasEdge[],
): ShotTarget | null {
  return findUpstream(nodeId, nodes, edges, (n) => {
    if (n.type !== "shot") return null;
    const d = n.data as ShotNodeData;
    return d.projectId && d.episodeId && d.shotId
      ? {
          projectId: d.projectId,
          episodeId: d.episodeId,
          shotId: d.shotId,
          label: d.label || "这一镜",
        }
      : null;
  });
}

/**
 * 分镜节点往下游发什么。
 *
 * **只发后端算好的那一段，一个字都不加。** 想在这儿补一句「电影感」
 * 之类的话，画布出的图就又和成片不一样了 —— 而这个节点存在的全部理由
 * 就是让它们一样。
 *
 * 没取到就发空 —— 发一句「（还没选镜头）」出去会被当成画面内容画进图里。
 */
export function shotPromptParts(d: ShotNodeData): string[] {
  const f = d.frame?.trim();
  return f ? [f] : [];
}
