import { useState, useRef, useEffect } from "react";
import { Handle, Position, type NodeProps } from "@xyflow/react";
import { BaseNodeWrapper } from "./BaseNodeWrapper";
import { useCanvasStore } from "../store/useCanvasStore";
import { MediaThumb } from "../../components/studio/MediaThumb";
import { NodeHeader } from "../ui/NodeHeader";
import { User } from "../ui/icons";
import type { CharacterNode } from "../types";

export function CharacterNodeComponent({ id, data, selected }: NodeProps<CharacterNode>) {
  const updateNodeData = useCanvasStore((s) => s.updateNodeData);

  const [name, setName] = useState(data.name || "");
  const [trigger, setTrigger] = useState(data.promptTrigger || "");
  const [appearance, setAppearance] = useState(data.appearancePrompt || "");
  const isComposingRef = useRef(false);

  useEffect(() => {
    if (!isComposingRef.current) {
      setName(data.name || "");
      setTrigger(data.promptTrigger || "");
      setAppearance(data.appearancePrompt || "");
    }
  }, [data.name, data.promptTrigger, data.appearancePrompt]);

  const removeNode = useCanvasStore((s) => s.removeNode);

  return (
    <div className="flex flex-col gap-2 select-none" style={{ width: 380 }}>
      <NodeHeader
        icon={<User className="h-3.5 w-3.5 text-white/60" />}
        title={name || "角色资产"}
        onDelete={() => removeNode(id)}
      />

      <BaseNodeWrapper id={id} selected={selected} sourceHandleId="char-out">
        <Handle
          type="target"
          position={Position.Left}
          id="char-in"
          className="!w-2.5 !h-5 !rounded-full !bg-zinc-300 hover:!bg-blue-400 hover:!border-white/50 !border !border-zinc-800 shadow-md transition-transform hover:scale-125 cursor-crosshair z-10"
        />

        <div className="p-4 flex flex-col gap-3">
          <div className="flex gap-3 items-start">
            {data.avatarUrl ? (
              <div className="w-12 h-12 rounded-xl overflow-hidden border border-white/10 shrink-0">
                <MediaThumb
                  path={data.avatarUrl}
                  alt={name}
                  className="w-full h-full object-cover"
                />
              </div>
            ) : (
              <div className="w-12 h-12 rounded-xl bg-white/5 border border-white/10 flex items-center justify-center text-white/60 font-bold text-sm shrink-0">
                {name?.[0] || "角"}
              </div>
            )}

        <div className="flex flex-col gap-1.5 flex-1 min-w-0">
          <div>
            <label className="text-[10px] text-zinc-400">角色名称</label>
            <input
              type="text"
              value={name}
              onChange={(e) => {
                setName(e.target.value);
                if (!isComposingRef.current) updateNodeData(id, { name: e.target.value });
              }}
              onCompositionStart={() => {
                isComposingRef.current = true;
              }}
              onCompositionEnd={(e) => {
                isComposingRef.current = false;
                updateNodeData(id, { name: e.currentTarget.value });
              }}
              onBlur={() => updateNodeData(id, { name })}
              onKeyDown={(e) => e.stopPropagation()}
              placeholder="角色名称"
              className="nodrag nopan nowheel w-full rounded bg-black/40 border border-white/10 px-2 py-1 text-xs text-zinc-200 focus:outline-none focus:border-purple-500/60"
            />
          </div>

          <div>
            <label className="text-[10px] text-zinc-400">触发词 (Trigger Tag)</label>
            <input
              type="text"
              value={trigger}
              onChange={(e) => {
                setTrigger(e.target.value);
                if (!isComposingRef.current) updateNodeData(id, { promptTrigger: e.target.value });
              }}
              onCompositionStart={() => {
                isComposingRef.current = true;
              }}
              onCompositionEnd={(e) => {
                isComposingRef.current = false;
                updateNodeData(id, { promptTrigger: e.currentTarget.value });
              }}
              onBlur={() => updateNodeData(id, { promptTrigger: trigger })}
              onKeyDown={(e) => e.stopPropagation()}
              placeholder="e.g. ayla_character"
              className="nodrag nopan nowheel w-full rounded bg-black/40 border border-white/10 px-2 py-1 text-[11px] text-purple-300 font-mono focus:outline-none focus:border-purple-500/60"
            />
          </div>
        </div>
      </div>

      <div className="flex flex-col gap-1 mt-1">
        <label className="text-[10px] text-zinc-400">外观特征描述</label>
        <textarea
          rows={2}
          value={appearance}
          onChange={(e) => {
            setAppearance(e.target.value);
            if (!isComposingRef.current)
              updateNodeData(id, { appearancePrompt: e.target.value });
          }}
          onCompositionStart={() => {
            isComposingRef.current = true;
          }}
          onCompositionEnd={(e) => {
            isComposingRef.current = false;
            updateNodeData(id, { appearancePrompt: e.currentTarget.value });
          }}
          onBlur={() => updateNodeData(id, { appearancePrompt: appearance })}
          onKeyDown={(e) => e.stopPropagation()}
          placeholder="发型、眼睛颜色、固定服装、体型特征..."
          className="nodrag nopan nowheel w-full rounded-lg bg-black/40 border border-white/10 px-2.5 py-1.5 text-xs text-zinc-200 placeholder-zinc-500 focus:outline-none focus:border-purple-500/60 resize-y"
        />
      </div>
    </div>
  </BaseNodeWrapper>
    </div>
  );
}
