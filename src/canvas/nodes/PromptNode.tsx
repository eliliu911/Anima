import { useState, useRef, useEffect } from "react";
import { Handle, Position, type NodeProps } from "@xyflow/react";
import { BaseNodeWrapper } from "./BaseNodeWrapper";
import { useCanvasStore } from "../store/useCanvasStore";
import { NodeHeader } from "../ui/NodeHeader";
import { FileText } from "../ui/icons";
import type { PromptNode } from "../types";

export function PromptNodeComponent({ id, data, selected }: NodeProps<PromptNode>) {
  const updateNodeData = useCanvasStore((s) => s.updateNodeData);
  const removeNode = useCanvasStore((s) => s.removeNode);
  const [localPrompt, setLocalPrompt] = useState(data.prompt || "");
  const isComposingRef = useRef(false);

  useEffect(() => {
    if (!isComposingRef.current) {
      setLocalPrompt(data.prompt || "");
    }
  }, [data.prompt]);


  return (
    <div className="flex flex-col gap-2 select-none" style={{ width: 440 }}>
      {/* 顶部悬浮标题 (对齐 DramaClaw: 📄 文本) */}
      <NodeHeader
        icon={<FileText className="h-3.5 w-3.5 text-white/60" />}
        title="文本"
        onDelete={() => removeNode(id)}
      />

      {/* 主卡片: 单层纯净灰黑统一背景，大幅扩大输入编辑区域 */}
      <BaseNodeWrapper id={id} selected={selected} sourceHandleId="prompt-out">
        {/* Target handle */}
        <Handle
          type="target"
          position={Position.Left}
          id="prompt-in"
          className="!w-2.5 !h-5 !rounded-full !bg-zinc-300 hover:!bg-blue-400 hover:!border-white/50 !border !border-zinc-800 shadow-md transition-transform hover:scale-125 cursor-crosshair z-10"
        />

        <div className="p-4 w-full min-h-[160px] flex flex-col justify-between">
          <textarea
            value={localPrompt}
            onChange={(e) => {
              setLocalPrompt(e.target.value);
              if (!isComposingRef.current) {
                updateNodeData(id, { prompt: e.target.value });
              }
            }}
            onCompositionStart={() => {
              isComposingRef.current = true;
            }}
            onCompositionEnd={(e) => {
              isComposingRef.current = false;
              updateNodeData(id, { prompt: e.currentTarget.value });
            }}
            onBlur={() => updateNodeData(id, { prompt: localPrompt })}
            onKeyDown={(e) => e.stopPropagation()}
            placeholder="写下故事、场景或角色灵感。例如：雨夜旧车站里，一封迟到的信改变了所有人的选择。"
            rows={6}
            className="nodrag nopan nowheel w-full flex-1 resize-none bg-transparent text-sm leading-relaxed text-white/90 placeholder:text-white/30 focus:outline-none"
          />
        </div>
      </BaseNodeWrapper>
    </div>
  );
}

