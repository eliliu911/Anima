import type { AudioClipNodeData, AudioNodeData, CanvasEdge, CanvasNode, PromptNodeData, ShotNodeData } from "../types";

/**
 * **声音复制的参考音频**（2026-10-13，照 LibTV：上游连进来的音频节点 = 参考）。
 * 只认**直接连着的**：音频素材（放的文件）或另一个试音色念好的那段。第一个有音频的算数。
 */
export function referenceAudio(nodeId: string, nodes: CanvasNode[], edges: CanvasEdge[]): { path: string; name: string; nodeId: string } | null {
  for (const e of edges.filter((x) => x.target === nodeId)) {
    const n = nodes.find((x) => x.id === e.source);
    if (n?.type === "audioClip") {
      const d = n.data as AudioClipNodeData;
      if (d.audioPath) return { path: d.audioPath, name: String(d.label || "音频"), nodeId: n.id };
    }
    if (n?.type === "audio") {
      const d = n.data as AudioNodeData;
      // 裸 PCM 没有文件头，解码不了
      if (d.outputAudioUrl && !d.outputAudioUrl.endsWith(".pcm")) return { path: d.outputAudioUrl, name: d.uploaded && d.label ? String(d.label) : d.usedVoice ? `音频 · ${d.usedVoice}` : "音频", nodeId: n.id };
    }
  }
  return null;
}

/** 会拿一段任意音频复制声音的引擎（和 Rust `canvas_audio::clone_capable` 同一张表）。
 *  Edge / Kokoro 只有自带音色；F5-TTS 走 OpenAI 兼容接口，只认音色库里的名字 */
export const CLONE_KINDS = ["gpt_sovits", "cosy_voice2", "index_tts2"] as const;
export const canClone = (kind: string | undefined) => !!kind && (CLONE_KINDS as readonly string[]).includes(kind);

/** 直接连着、但还没有音频的音频节点（复制时提醒，不悄悄按默认音色念） */
export function emptyReferenceAudio(nodeId: string, nodes: CanvasNode[], edges: CanvasEdge[]): boolean {
  return edges.filter((x) => x.target === nodeId).some((e) => {
    const n = nodes.find((x) => x.id === e.source);
    return (n?.type === "audioClip" && !(n.data as AudioClipNodeData).audioPath) || (n?.type === "audio" && !(n.data as AudioNodeData).outputAudioUrl);
  }) && !referenceAudio(nodeId, nodes, edges);
}

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
      // **不穿过音频节点**：上游连的音频是声音复制的参考，它上游那段词是参考念的，不是这里要念的
      if (n.type === "audio" || n.type === "audioClip") continue;
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
