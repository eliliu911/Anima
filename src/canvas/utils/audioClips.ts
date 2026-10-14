import { canvasSaveAudio } from "../../api/canvasDepth";
import { useCanvasStore } from "../store/useCanvasStore";
import type { AudioClipNodeData, AudioNodeData, CanvasEdge, CanvasNode } from "../types";

/** 音频节点在连线里显示的名字：上传的用文件名，复制 / 念出来的用音色 */
export function audioNodeName(d: AudioNodeData): string {
  if (d.uploaded && d.label) return String(d.label);
  return d.usedVoice ? `音频 · ${d.usedVoice}` : "音频";
}

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
 * 视频节点会收到的参考音频，**按连线顺序、按路径去重**。认两种直接连着的节点：
 * 音频素材（`audioClip`，放的文件）和试音色（`audio`，TTS 念出来的那段，2026-10-13 用户要的「补上这个缺口」——
 * 以前它连到视频节点上会被**静默忽略**）。——
 * 音频不会「穿过」别的节点往下传（和首帧只认直接连着的图同一条规矩）。
 */
export function videoInputAudios(targetId: string, nodes: CanvasNode[], edges: CanvasEdge[]): VideoInputAudio[] {
  const out: VideoInputAudio[] = [];
  for (const e of edges.filter((x) => x.target === targetId)) {
    const n = nodes.find((x) => x.id === e.source);
    if (n?.type === "audio") {
      const d = n.data as AudioNodeData;
      // 裸 PCM（高级设置里选的导出格式）没有文件头，H3 读不了 —— 不当参考音频
      if (!d.outputAudioUrl || d.outputAudioUrl.endsWith(".pcm") || out.some((a) => a.path === d.outputAudioUrl)) continue;
      out.push({ path: d.outputAudioUrl, name: audioNodeName(d), secs: d.secs, nodeId: n.id });
      continue;
    }
    if (n?.type !== "audioClip") continue;
    const d = n.data as AudioClipNodeData;
    if (!d.audioPath || out.some((a) => a.path === d.audioPath)) continue;
    out.push({ path: d.audioPath, name: String(d.label || "音频"), secs: d.secs, nodeId: n.id });
  }
  return out;
}

/** 直接连着、但还没有音频的节点：没放文件的音频素材、还没念的试音色（点生成时提醒，不悄悄当没连） */
export function emptyUpstreamAudios(targetId: string, nodes: CanvasNode[], edges: CanvasEdge[]): number {
  return edges.filter((e) => e.target === targetId)
    .map((e) => nodes.find((n) => n.id === e.source))
    .filter((n) => (n?.type === "audioClip" && !(n.data as AudioClipNodeData).audioPath)
      || (n?.type === "audio" && !(n.data as AudioNodeData).outputAudioUrl)).length;
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
 * 给节点放一段音频。**「上传资源」节点（图片素材形态）选了音频会就地变成音频节点**，连线保留 ——
 * 用户 2026-10-08：上传资源要能自动识别音频，不再只是「上传图片」。
 *
 * 2026-10-13 起变成的是**统一的音频节点**（`audio`：上传 / 生成 / 声音复制一个节点，照 LibTV）；
 * 老画布上的音频素材节点（`audioClip`）在它自己上面换一段时还是它自己。
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
  const loadingType = node.type === "audioClip" || node.type === "audio";
  if (loadingType) store.updateProjectNodeData(projectId, nodeId, { referenceLoading: true, error: undefined });
  try {
    const saved = await canvasSaveAudio(dataUrl, file.name);
    const latest = useCanvasStore.getState();
    const cur = latest.nodes.find((n) => n.id === nodeId);
    if (!cur) throw new Error("节点已删除");
    if (cur.type === "audioClip") {
      const label = cur.data.label && cur.data.label !== "外部素材" && cur.data.label !== "上传资源" && cur.data.label !== "音频素材" ? String(cur.data.label) : file.name;
      const data: AudioClipNodeData = { label, audioPath: saved.path, secs: saved.secs ?? undefined };
      latest.setNodes(latest.nodes.map((n) => (n.id === nodeId ? ({ ...n, data } as CanvasNode) : n)));
    } else {
      // 统一的音频节点：放上传的这段，清掉上一次生成留下的回显（音色、复制参考）—— 不然名字还写着上次那把嗓子
      const keep = cur.type === "audio" ? (cur.data as AudioNodeData) : {};
      const data: AudioNodeData = {
        ...keep, label: file.name, uploaded: true, outputAudioUrl: saved.path, secs: saved.secs ?? undefined,
        usedVoice: undefined, usedEngineId: undefined, cloneUsed: undefined, error: undefined,
      };
      latest.setNodes(latest.nodes.map((n) => (n.id === nodeId ? ({ ...n, type: "audio", data } as CanvasNode) : n)));
    }
    // 改了类型的节点，连出去的线换成音频的桩
    latest.setEdges(latest.edges.map((e) => (e.source === nodeId ? { ...e, sourceHandle: "audio-out" } : e)));
  } catch (e) {
    const t = useCanvasStore.getState().nodes.find((n) => n.id === nodeId)?.type;
    if (t === "audioClip" || t === "audio") {
      useCanvasStore.getState().updateProjectNodeData(projectId, nodeId, { error: String(e) });
    }
    throw e;
  } finally {
    const t = useCanvasStore.getState().nodes.find((n) => n.id === nodeId)?.type;
    if (t === "audioClip" || t === "audio") {
      useCanvasStore.getState().updateProjectNodeData(projectId, nodeId, { referenceLoading: false });
    }
  }
}
