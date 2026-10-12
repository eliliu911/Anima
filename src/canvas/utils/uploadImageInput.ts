import { saveChatImageAttachment } from "../../api";
import { useCanvasStore } from "../store/useCanvasStore";
import type { CanvasReference } from "../types";
import { addReference, MAX_VIDEO_REFERENCES } from "./referenceBindings";
import { MAX_IMAGE_REFERENCES } from "./imageMode";

/** Upload first, then atomically bind the input; output fields are never touched. */
export async function uploadImageInput(nodeId: string, file: File, purpose: "reference" | "source" = "reference", replacePath?: string) {
  const store = useCanvasStore.getState();
  const projectId = store.activeProjectId;
  const update = (patch: Record<string, unknown>) =>
    store.updateProjectNodeData(projectId, nodeId, patch);
  const node = store.nodes.find((n) => n.id === nodeId);
  if (!node) throw new Error("节点已删除");
  if (node.data.isGenerating || node.data.referenceLoading) throw new Error("节点忙碌，请稍后上传");
  if (!file.type.startsWith("image/")) throw new Error("请选择图片文件");
  update({ referenceLoading: true });
  try {
    const dataUrl = await new Promise<string>((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(String(reader.result));
      reader.onerror = () => reject(reader.error ?? new Error("图片读取失败"));
      reader.onabort = () => reject(new Error("图片读取已取消"));
      reader.readAsDataURL(file);
    });
    const saved = await saveChatImageAttachment({ dataUrl, filename: file.name });
    if (!saved.path) throw new Error("图片保存失败：未返回本地路径");
    const latest = useCanvasStore.getState().getProjectCanvas(projectId).nodes.find((n) => n.id === nodeId);
    if (!latest) throw new Error("节点已删除");
    if (latest.data.isGenerating) throw new Error("节点已开始生成，请稍后重新绑定");
    const ref = { path: saved.path, name: file.name };
    // **图片节点上传 = 变成素材节点。** 图放在输出上，下游连一个图片节点才是图生图。
    // 以前是塞进这个节点自己的输入、让它自己变成图生图节点，于是贴着「源图 · 未生成」「图生图」，
    // 而用户只是想放一张图给下游用（2026-09-14）。
    if (purpose === "source" && latest.type === "imageGen") {
      update({ referenceOnly: true, label: latest.data.label && latest.data.label !== "上传资源" ? latest.data.label : file.name,
        outputImageUrl: saved.path, outputImages: [saved.path], references: [] });
      return;
    }
    if (purpose === "reference" && latest.type === "videoGen") {
      store.bindReference(projectId, nodeId, ref, replacePath);
      return;
    }
    const limit = latest.type === "videoGen" ? MAX_VIDEO_REFERENCES : MAX_IMAGE_REFERENCES;
    const current = (latest.data.references ?? []) as CanvasReference[];
    if (replacePath && !current.some((r) => r.path === replacePath)) throw new Error("待替换的参考已移除");
    const references = purpose === "source" ? [ref]
      : replacePath ? current.map((r) => r.path === replacePath ? ref : r)
      : addReference(current, ref, limit);
    update({ references,
      ...(latest.type === "videoGen" ? { mode: purpose === "source" ? "i2v" : "r2v" } : {}),
    });
  } finally {
    update({ referenceLoading: false });
  }
}
