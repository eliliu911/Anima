import { canvasSaveVideo } from "../../api/canvasDepth";
import { useCanvasStore } from "../store/useCanvasStore";

/**
 * 给视频节点上传一段视频当素材（深度动作捕捉的源）。
 *
 * 上传的视频放在节点的**输出**位上（`outputVideoUrl`），和生成出来的视频一样被下游读取 ——
 * 下游不需要知道它是传的还是生成的。`uploadedVideo` 标记只是为了界面上说清楚。
 */
export async function uploadVideoInput(nodeId: string, file: File) {
  const store = useCanvasStore.getState();
  const projectId = store.activeProjectId;
  const node = store.nodes.find((n) => n.id === nodeId);
  if (!node) throw new Error("节点已删除");
  if (node.data.isGenerating) throw new Error("节点正在生成，稍后再上传");
  if (!file.type.startsWith("video/")) throw new Error("请选择视频文件（mp4 / mov / webm）");
  const update = (patch: Record<string, unknown>) => store.updateProjectNodeData(projectId, nodeId, patch);
  update({ referenceLoading: true, error: undefined });
  try {
    const dataUrl = await new Promise<string>((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(String(reader.result));
      reader.onerror = () => reject(reader.error ?? new Error("视频读取失败"));
      reader.readAsDataURL(file);
    });
    const saved = await canvasSaveVideo(dataUrl, file.name);
    const latest = useCanvasStore.getState().getProjectCanvas(projectId).nodes.find((n) => n.id === nodeId);
    if (!latest) throw new Error("节点已删除");
    const old = (latest.data.outputVideos as string[] | undefined) ?? [];
    update({ outputVideoUrl: saved.path, outputVideos: [saved.path, ...old], uploadedVideo: true });
  } finally {
    update({ referenceLoading: false });
  }
}
