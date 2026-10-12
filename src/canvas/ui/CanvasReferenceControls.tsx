import { useEffect, useRef, useState } from "react";
import { uploadImageInput } from "../utils/uploadImageInput";
import { AssetPicker } from "../../components/studio/AssetPicker";
import { MediaThumb } from "../../components/studio/MediaThumb";
import { ComposingInput } from "../../components/ComposingInput";
import { useCanvasStore } from "../store/useCanvasStore";
import type { CanvasReference } from "../types";
import { referencePatch, assetReference } from "../utils/referenceBindings";
import { RotateCcw } from "./icons";
import { Layers, Upload, Plus, Sparkles } from "./icons";
import "./CanvasReferenceControls.css";
import { CanvasLoraControls } from "./CanvasLoraControls";
import { UpstreamCards } from "./UpstreamCards";
import type { VideoMode } from "../utils/videoMode";

export function CanvasReferenceControls({
  nodeId,
  children,
  rightAction,
  onMentionReference,
  videoMode,
  frames,
}: {
  nodeId: string;
  children?: React.ReactNode;
  rightAction?: React.ReactNode;
  onMentionReference?: (index: number, ref: CanvasReference) => void;
  /** 视频节点：当前模式和首尾帧，卡片角标写首帧 / 尾帧（见 UpstreamCards 的 cardRole） */
  videoMode?: VideoMode;
  frames?: string[];
}) {
  const node = useCanvasStore((s) => s.nodes.find((n) => n.id === nodeId));
  const input = useRef<HTMLInputElement>(null);
  const uploading = useRef(false);
  const replacePath = useRef<string | undefined>(undefined);
  const [menu, setMenu] = useState(false);
  const menuAnchor = useRef<HTMLDivElement>(null);
  const menuTrigger = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (!menu) return;
    const dismiss = (event: PointerEvent) => {
      if (!menuAnchor.current?.contains(event.target as Node)) setMenu(false);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") { setMenu(false); menuTrigger.current?.focus(); }
    };
    document.addEventListener("pointerdown", dismiss, true);
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("pointerdown", dismiss, true);
      document.removeEventListener("keydown", escape);
    };
  }, [menu]);
  const choosing = useCanvasStore((s) => s.referenceTargetId === nodeId);
  const [picker, setPicker] = useState(false);
  /** 点开的那张参考图（展开名字 / @ / 替换）。null = 都收着 */
  const [openRef, setOpenRef] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  if (!node) return null;
  const references = (node.data.references ?? []) as CanvasReference[];
  const locked = busy || !!node.data.isGenerating || !!node.data.referenceLoading;
  const bind = (reference: CanvasReference) => {
    const store = useCanvasStore.getState();
    const latest = store.nodes.find((n) => n.id === nodeId);
    if (!latest) throw new Error("节点已删除");
    if (latest.data.isGenerating || latest.data.referenceLoading) throw new Error("节点忙碌，不能修改参考图");
    if (latest.type === "videoGen") store.bindReference(store.activeProjectId, nodeId, reference);
    else store.updateNodeData(nodeId, referencePatch(latest, reference));
    setError("");
  };
  const pick = (reference: CanvasReference) => {
    try { bind(reference); } catch (e) { setError(String(e)); }
  };
  async function upload(files: File[]) {
    if (uploading.current) return;
    uploading.current = true; setBusy(true); setError("");
    try {
      const projectId = useCanvasStore.getState().activeProjectId;
      for (const file of files) {
        if (useCanvasStore.getState().activeProjectId !== projectId) break;
        await uploadImageInput(nodeId, file, "reference", replacePath.current);
      }
    } catch (e) { setError(String(e)); }
    finally {
      replacePath.current = undefined; uploading.current = false; setBusy(false);
    }
  }
  return (
    <div className="nodrag nopan nowheel flex flex-col gap-2 text-xs text-white/80">
      <div className="flex items-center justify-between text-xs text-white/70">
        <div className="relative flex items-center gap-1 flex-wrap">
          <div ref={menuAnchor} className="canvas-reference-anchor">
          <button
            ref={menuTrigger}
            aria-expanded={menu}
            type="button"
            disabled={locked}
            onClick={() => setMenu(!menu)}
            className="h-7 flex items-center gap-1.5 rounded-md px-2 text-white/70 hover:bg-white/10 hover:text-white transition-colors disabled:opacity-40 whitespace-nowrap"
          >
            <Plus className="h-3.5 w-3.5" />
            <span>{busy ? "保存中…" : "参考"}</span>
          </button>
          {menu && <div className="canvas-reference-menu" aria-label="参考来源">
            <button type="button" disabled={locked} className="canvas-reference-option" onClick={() => {
              useCanvasStore.setState({ referenceTargetId: nodeId }); setMenu(false);
            }}><Sparkles className="canvas-reference-option-icon" /><span><span className="canvas-reference-option-title">画布参考</span><span className="canvas-reference-option-detail">图片与角色节点</span></span></button>
            <button type="button" disabled={locked} className="canvas-reference-option" onClick={() => {
              replacePath.current = undefined; input.current?.click(); setMenu(false);
            }}><Upload className="canvas-reference-option-icon" /><span><span className="canvas-reference-option-title">外部参考</span><span className="canvas-reference-option-detail">本地图片素材</span></span></button>
          </div>}
          </div>
          <button
            type="button"
            disabled={locked}
            onClick={() => setPicker(true)}
            className="h-7 flex items-center gap-1.5 rounded-md px-2 text-white/70 hover:bg-white/10 hover:text-white transition-colors disabled:opacity-40 whitespace-nowrap"
          >
            <Layers className="h-3.5 w-3.5" />
            <span>资产库</span>
          </button>
          <input
            ref={input}
            type="file"
            accept="image/*"
            multiple
            className="hidden"
            onChange={(e) => {
              const files = Array.from(e.target.files ?? []);
              e.target.value = "";
              if (files.length) void upload(replacePath.current ? files.slice(0, 1) : files);
            }}
          />

          {children}
          <CanvasLoraControls nodeId={nodeId} />
        </div>

        {rightAction}
      </div>

      {choosing && <button type="button" onClick={() => useCanvasStore.setState({ referenceTargetId: null })}>选择画布图片或角色节点 · 取消</button>}
      {!!node.data.referenceError && (
        <div className="flex items-center justify-between text-xs text-red-400 bg-red-500/10 px-2 py-1 rounded">
          <span>{String(node.data.referenceError)}</span>
          <button
            type="button"
            onClick={() => useCanvasStore.getState().updateNodeData(nodeId, { referenceError: undefined })}
            className="text-white/40 hover:text-white ml-2 text-xs"
          >
            ✕
          </button>
        </div>
      )}
      {/* 连进来的每个图 / 视频一张卡片（照 LibTV）：编号 = 发给模型的 <Picture N>，悬停 × 断开，双击跳到那个节点。
          点一张绑在节点上的参考图，下面展开它的名字 / @ / 替换 —— 以前每张图常驻一整行，现在只展开点中的那张 */}
      <UpstreamCards nodeId={nodeId} locked={locked} mode={videoMode} frames={frames} selectedKey={openRef ? `ref:${openRef}` : undefined}
        onSelect={(c) => setOpenRef(c.reference && openRef !== c.reference.path ? c.reference.path : null)} />
      {references.map((ref, index) => ref.path !== openRef ? null : (
            <div key={ref.path} className="flex max-w-full items-center gap-2 rounded-md border border-white/10 p-1">
              <MediaThumb path={ref.path} className="h-10 w-10 shrink-0 rounded object-contain" />
              <span>{index + 1}.</span>
              {onMentionReference && (
                <button
                  type="button"
                  title={`在提示词中引用 @${ref.name?.trim() || `图片${index + 1}`}`}
                  aria-label={`引用 @${ref.name?.trim() || `图片${index + 1}`}`}
                  disabled={locked}
                  className="flex items-center gap-0.5 rounded bg-blue-500/25 px-1.5 py-0.5 text-[11px] font-medium text-blue-300 hover:bg-blue-500/40 transition-colors shrink-0"
                  onClick={() => onMentionReference(index + 1, ref)}
                >
                  @{ref.name?.trim() || `图片${index + 1}`}
                </button>
              )}
              {/* **这个名字几乎必然是中文** —— 它就是角色名，而且会变成
                  H3 的 `<Picture N> 设定为 X`。以前是普通受控 input，
                  每敲一个字母就回灌一次节点数据，把输入法的组合缓冲冲掉，
                  打出来是 `yu'jiyu'jin` 那种拼音串。见 `ComposingInput`。 */}
              <ComposingInput aria-label={`参考 ${index + 1} 名称`} value={ref.name} disabled={locked}
                className="w-28 min-w-0 bg-transparent" onCommit={(v) => {
                  useCanvasStore.getState().updateNodeData(nodeId, { references: references.map((r) => r.path === ref.path ? { ...r, name: v } : r) });
                }} />
              <button type="button" title="替换参考图" aria-label={`替换 ${ref.name}`} disabled={locked}
                onClick={() => { replacePath.current = ref.path; input.current?.click(); }}><RotateCcw width={14} height={14} /></button>
            </div>
      ))}
      {error && <p role="alert" className="break-words text-red-400">{error}</p>}
      <AssetPicker
        open={picker}
        only="image"
        onClose={() => setPicker(false)}
        onPick={(path) => pick({ path, name: path.split("/").pop() || "参考图" })}
        onPickAsset={(asset) => {
          try {
            pick(assetReference(asset));
          } catch (e) {
            setError(String(e));
          }
        }}
      />
    </div>
  );
}
