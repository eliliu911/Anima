import type { CanvasEdge, CanvasNode, PromptNodeData, ShotNodeData } from "../types";

/**
 * 音频节点要念的词从哪来。
 *
 * # 为什么要走上游
 *
 * 试音色的价值在于**几把嗓子念同一段**。各节点自己打一遍词，
 * 打错一个字这次对比就不作数了 —— 而且不会有任何提示。
 * 接同一个上游才比得公平。
 *
 * # 分镜节点取的是台词，不是画面描述
 *
 * `ShotNodeData.frame` 是后端算好的**画面**提示词，把它念出来毫无意义。
 * 台词在 `dialogue` 上。取错的表现是音频节点念了一大段镜头语言，
 * **而且不报错**。
 */
export function upstreamSpeechText(nodeId: string, nodes: CanvasNode[], edges: CanvasEdge[]): string {
  const seen = new Set<string>([nodeId]);
  let frontier = edges.filter((e) => e.target === nodeId).map((e) => e.source);
  // 逐层往上找，找到第一个给得出词的就停。**带环保护**
  for (let depth = 0; depth < 8 && frontier.length; depth++) {
    const next: string[] = [];
    for (const id of frontier) {
      if (seen.has(id)) continue;
      seen.add(id);
      const n = nodes.find((x) => x.id === id);
      if (!n) continue;
      if (n.type === "prompt") {
        const t = (n.data as PromptNodeData).prompt?.trim();
        if (t) return t;
      }
      if (n.type === "shot") {
        // **只取台词。** frame 是画面描述，念出来毫无意义
        const t = (n.data as ShotNodeData & { dialogue?: string }).dialogue?.trim();
        if (t) return t;
      }
      next.push(...edges.filter((e) => e.target === id).map((e) => e.source));
    }
    frontier = next;
  }
  return "";
}
