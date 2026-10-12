import { canvasSaveAudio } from "../../api/canvasDepth";
import { useCanvasStore } from "../store/useCanvasStore";
import type { AudioClipNodeData, CanvasEdge, CanvasNode } from "../types";

/**
 * 画布的**参考音频**（2026-10-08 用户：「在画布里增加音频参考，类似图片做参考图一样」）。
 *
 * 音频素材节点（`audioClip`）直接连到视频节点上 = 这段音频作为 H3 Ref2VA 的 `<Audio N>` 发出去。
 * 编号和图片 `<Picture N>` 各自独立（官方 Ref2VA 指南 2.5），按连线顺序。
 */

/** H3 一次最多几段参考音频（WeeTodd `ref2va.MAX_REFERENCE_AUDIOS`，Rust `weetodd::H3_MAX_REFERENCE_AUDIOS`） */
export const MAX_VIDEO_AUDIOS = 3;

/** 认音频文件：先看 MIME，系统给空 MIME 时看扩展名 */
export function isAudioFile(file: { type: string; name: string }): boolean {
  if (file.type.startsWith("audio/")) return true;
  return !file.type && /\.(mp3|wav|m4a|aac|flac|ogg)$/i.test(file.name);
}

export interface VideoInputAudio {
  path: string;
  name: string;
  secs?: number;
  nodeId: string;
}

/**
 * 视频节点会收到的参考音频，**按连线顺序、按路径去重**。只认直接连着的音频素材节点 ——
 * 音频不会「穿过」别的节点往下传（和首帧只认直接连着的图同一条规矩）。
 */
export function videoInputAudios(targetId: string, nodes: CanvasNode[], edges: CanvasEdge[]): VideoInputAudio[] {
  const out: VideoInputAudio[] = [];
  for (const e of edges.filter((x) => x.target === targetId)) {
    const n = nodes.find((x) => x.id === e.source);
    if (n?.type !== "audioClip") continue;
    const d = n.data as AudioClipNodeData;
    if (!d.audioPath || out.some((a) => a.path === d.audioPath)) continue;
    out.push({ path: d.audioPath, name: String(d.label || "音频"), secs: d.secs, nodeId: n.id });
  }
  return out;
}

/** 直接连着、但还没放音频的音频素材节点（点生成时提醒，不悄悄当没连） */
export function emptyUpstreamAudios(targetId: string, nodes: CanvasNode[], edges: CanvasEdge[]): number {
  return edges.filter((e) => e.target === targetId)
    .map((e) => nodes.find((n) => n.id === e.source))
    .filter((n) => n?.type === "audioClip" && !(n.data as AudioClipNodeData).audioPath).length;
}

/** 片长和音频长度对不上时要说的话。音频会被截到片长（后端 `stage_reference_audio`） */
export function audioLengthNote(audios: VideoInputAudio[], durationSecs: number): string | null {
  const notes = audios.flatMap((a, i) => {
    if (!a.secs) return [];
    if (a.secs > durationSecs + 0.05) return [`<Audio ${i + 1}>「${a.name}」${a.secs.toFixed(1)} 秒，只用前 ${durationSecs} 秒`];
    if (a.secs < durationSecs - 0.5) return [`<Audio ${i + 1}>「${a.name}」只有 ${a.secs.toFixed(1)} 秒，片长 ${durationSecs} 秒 —— 后面没声音`];
    return [];
  });
  return notes.length ? notes.join("；") : null;
}

/**
 * 给节点放一段音频。**「上传资源」节点（图片素材形态）选了音频会就地变成音频素材节点**，连线保留 ——
 * 用户 2026-10-08：上传资源要能自动识别音频，不再只是「上传图片」。
 */
export async function uploadAudioInput(nodeId: string, file: File) {
  const store = useCanvasStore.getState();
  const projectId = store.activeProjectId;
  const node = store.nodes.find((n) => n.id === nodeId);
  if (!node) throw new Error("节点已删除");
  if (!isAudioFile(file)) throw new Error("请选择音频文件（mp3 / wav / m4a / flac / ogg）");
  const dataUrl = await new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(reader.error ?? new Error("音频读取失败"));
    reader.readAsDataURL(file);
  });
  if (node.type === "audioClip") store.updateProjectNodeData(projectId, nodeId, { referenceLoading: true, error: undefined });
  try {
    const saved = await canvasSaveAudio(dataUrl, file.name);
    const latest = useCanvasStore.getState();
    const cur = latest.nodes.find((n) => n.id === nodeId);
    if (!cur) throw new Error("节点已删除");
    const label = cur.data.label && cur.data.label !== "外部素材" && cur.data.label !== "上传资源" && cur.type === "audioClip" ? String(cur.data.label) : file.name;
    const data: AudioClipNodeData = { label, audioPath: saved.path, secs: saved.secs ?? undefined };
    latest.setNodes(latest.nodes.map((n) => (n.id === nodeId ? ({ ...n, type: "audioClip", data } as CanvasNode) : n)));
    // 改了类型的节点，连出去的线换成音频的桩
    latest.setEdges(latest.edges.map((e) => (e.source === nodeId ? { ...e, sourceHandle: "audio-out" } : e)));
  } catch (e) {
    if (useCanvasStore.getState().nodes.find((n) => n.id === nodeId)?.type === "audioClip") {
      useCanvasStore.getState().updateProjectNodeData(projectId, nodeId, { error: String(e) });
    }
    throw e;
  } finally {
    if (useCanvasStore.getState().nodes.find((n) => n.id === nodeId)?.type === "audioClip") {
      useCanvasStore.getState().updateProjectNodeData(projectId, nodeId, { referenceLoading: false });
    }
  }
}
