import { useRef, useState } from "react";
import { useCanvasStore } from "../store/useCanvasStore";
import { uploadImageInput } from "../utils/uploadImageInput";
import { isAudioFile, uploadAudioInput } from "../utils/audioClips";
import { Upload } from "./icons";

/**
 * 「上传资源」节点的上传按钮：**图片和音频都收，按文件自动分**（用户 2026-10-08）。
 * 图片 → 照旧当图片素材；音频 → 这个节点就地变成音频素材节点（连线保留）。
 */
export function CanvasMediaUploadButton({ nodeId, label = "上传图片 / 音频" }: { nodeId: string; label?: string }) {
  const input = useRef<HTMLInputElement>(null);
  const [error, setError] = useState("");
  const locked = useCanvasStore((s) => {
    const node = s.nodes.find((n) => n.id === nodeId);
    return !node || !!node.data.isGenerating || !!node.data.referenceLoading;
  });
  return <div className="nodrag nopan flex min-w-0 flex-col items-center gap-1">
    <button type="button" disabled={locked} onClick={() => input.current?.click()}
      className="flex items-center gap-1 rounded-md border border-white/10 bg-white/5 px-2 py-1 text-xs text-white/70 hover:bg-white/10 disabled:opacity-40">
      <Upload className="h-3 w-3" />{label}
    </button>
    <input ref={input} type="file" accept="image/*,audio/*,.mp3,.wav,.m4a,.flac,.ogg" className="hidden" onChange={(e) => {
      const file = e.target.files?.[0]; e.target.value = "";
      if (!file) return;
      setError("");
      const run = isAudioFile(file) ? uploadAudioInput(nodeId, file) : uploadImageInput(nodeId, file, "source");
      void run.catch((err) => setError(String(err)));
    }} />
    {error && <span role="alert" className="max-w-[240px] break-words text-xs text-red-400">{error}</span>}
  </div>;
}
