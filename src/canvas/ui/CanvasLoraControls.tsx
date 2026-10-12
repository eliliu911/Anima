import { useEffect, useState } from "react";
import { studioCapabilities, type EngineSpec } from "../../api/studio";
import { useCanvasStore } from "../store/useCanvasStore";
import type { ImageGenNodeData } from "../types";
import { Layers } from "./icons";
import { StyleGalleryDialog } from "./StyleGalleryDialog";

/**
 * 节点上的「风格」按钮：打开风格窗口（照 LibTV 的风格广场，见 `StyleGalleryDialog`）。
 * 挑中的 LoRA 在面板顶上出一张风格卡（UpstreamCards），× 就是取消挂载。
 */
export function CanvasLoraControls({ nodeId }: { nodeId: string }) {
  const node = useCanvasStore((s) => s.nodes.find((n) => n.id === nodeId));
  const [open, setOpen] = useState(false);
  const [engines, setEngines] = useState<EngineSpec[]>([]);
  useEffect(() => {
    if (!open) return;
    let live = true;
    studioCapabilities().then((c) => { if (live) setEngines(c.engines); }).catch(() => {});
    return () => { live = false; };
  }, [open]);
  if (!node) return null;
  const data = node.data as ImageGenNodeData;
  const picks = data.loras ?? [];
  const engine = engines.find((e) => e.id === data.engine);
  const update = (patch: Record<string, unknown>) => useCanvasStore.getState().updateNodeData(nodeId, patch);
  const setLora = (name: string, strength: number | null) => {
    const latest = useCanvasStore.getState().nodes.find((n) => n.id === nodeId)?.data as ImageGenNodeData;
    if (!latest || latest.isGenerating) return;
    const cur = latest.loras ?? [];
    const others = cur.filter((l) => l.name !== name);
    update({ loras: strength === null ? others : cur.some((l) => l.name === name)
      ? cur.map((l) => l.name === name ? { name, strength } : l) : [...others, { name, strength }] });
  };
  return <div className="canvas-lora-anchor">
    <button type="button" onClick={() => setOpen(true)} aria-expanded={open} title="风格：挂 LoRA，或加一段提示词风格"
      className="h-7 flex items-center gap-1.5 rounded-md px-2 text-white/70 hover:bg-white/10">
      <Layers width={14} height={14} />风格{picks.length ? ` · ${picks.length}` : ""}
    </button>
    {open && (
      <StyleGalleryDialog
        engine={String(data.engine ?? "")}
        engineLabel={engine?.label}
        // 能力表还没读回来时先当能挂（不然窗口一打开先闪成「只有提示词风格」）
        lorasSupported={engine ? Boolean(engine.loras) : true}
        picks={picks}
        prompt={String(data.prompt ?? "")}
        disabled={Boolean(data.isGenerating) || Boolean(data.referenceLoading)}
        onSetLora={setLora}
        onPromptChange={(p) => update({ prompt: p })}
        onClose={() => setOpen(false)}
      />
    )}
  </div>;
}
