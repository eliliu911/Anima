import type * as React from "react";
import { Handle, Position } from "@xyflow/react";
import { NodeSpawnPlusOverlay } from "../ui/NodeSpawnPlusOverlay";
import { CancelJobButton } from "../ui/CancelJobButton";
import { useCanvasStore } from "../store/useCanvasStore";
import { nodeGuide } from "../utils/nodeGuide";
import { INPUT_HANDLE } from "../utils/spawnLinked";
import type { CanvasNodeType } from "../types";

interface BaseNodeWrapperProps {
  id: string;
  selected?: boolean;
  isGenerating?: boolean;
  error?: string;
  notes?: string[];
  children: React.ReactNode;
  className?: string;
  sourceHandleId?: string;
  hideSourceHandle?: boolean;
}

export function BaseNodeWrapper({
  id,
  selected = false,
  isGenerating = false,
  error,
  notes,
  children,
  className = "",
  sourceHandleId,
  hideSourceHandle = false,
}: BaseNodeWrapperProps) {
  // **模板说明（`guide`）和运行提示（`notes`）是两样东西。** 以前步骤说明写在 notes 里，
  // 节点一跑就被执行器清掉、另存模板时也被当产物剥掉（2026-09-23 用户截图里跑过的节点
  // 都没了【第 N 步】）。guide 只有写模板的人会改，运行不碰、存模板保留。
  // 选原始值（引用稳定），过滤放到渲染里 —— 选择器每次回新数组会让 zustand 无限重渲染
  const nodeType = useCanvasStore((s) => s.nodes.find((n) => n.id === id)?.type) as CanvasNodeType | undefined;
  const rawGuide = useCanvasStore((s) => (s.nodes.find((n) => n.id === id)?.data as { guide?: unknown } | undefined)?.guide);
  const guide = nodeGuide({ guide: rawGuide });
  // 过渡期的模板 notes 和 guide 各存一份（旧版只认 notes）—— 同一句只显示一次
  const shownNotes = notes?.filter((n) => !guide.includes(n));
  return (
    <div
      className={`group/node rounded-2xl transition-colors duration-150 relative bg-[#1c1c20] border ${
        selected
          ? "border-blue-500/50 shadow-[0_0_0_1px_rgba(59,130,246,0.4),0_12px_32px_rgba(0,0,0,0.5)]"
          : "border-white/10 hover:border-white/20 shadow-[0_8px_20px_rgba(0,0,0,0.3)]"
      } ${className}`}
      style={{
        transform: "translateZ(0)",
      }}
    >
      {/* 节点右侧卡片边缘主输出桩：确保连线始终稳固连接卡片边缘，绝不断线 */}
      {!hideSourceHandle && sourceHandleId && (
        <Handle
          type="source"
          position={Position.Right}
          id={sourceHandleId}
          className="!w-2 !h-4 !rounded-full !bg-zinc-300 hover:!bg-white !border !border-zinc-800 shadow-md transition-transform hover:scale-125 cursor-crosshair z-10"
        />
      )}

      {/* 聚焦时显示的悬浮加号胶囊：支持按住直接拖动连线，或点击唤出衍生菜单 */}
      {!hideSourceHandle && (
        <NodeSpawnPlusOverlay
          nodeId={id}
          selected={selected}
          handleId={sourceHandleId}
        />
      )}

      {/* 生成中不再在右上角闪蓝点（用户 2026-09-25 要去掉）：在跑的节点由连线上的彗星 / 移动虚线和节点里的进度表示 */}

      {/* 取消。**放在这里而不是各个节点里** —— 图片、视频、超分、补帧、
          深度、长镜头全都用这个壳，写在壳上就一处都不会漏。 */}
      {isGenerating && <CancelJobButton nodeId={id} />}

      {children}
      {/* 左侧的加号：往前接（新建一个连进来的上游节点）。只给有输入口的节点（2026-10-10 用户：「左侧没做功能」）。
          **必须排在 children 后面**：它自己也是个输入口（`prompt-in-plus`），而没记 `targetHandle` 的老连线会接到节点的
          **第一个**输入口上 —— 排在前面时，所有老连线都接到了这个藏在节点外 36 像素的加号上，线头悬在半空（2026-10-11 用户截图） */}
      {nodeType && INPUT_HANDLE[nodeType] && <NodeSpawnPlusOverlay nodeId={id} selected={selected} side="left" />}
      {guide.length > 0 && <div className="border-t border-sky-400/15 px-3.5 py-2 text-xs text-white/60">
        {guide.map((line, index) => (
          <p className={`whitespace-pre-wrap break-words ${index === 0 ? "text-sky-200 font-medium" : ""}`} key={index}>{line}</p>
        ))}
      </div>}
      {!!shownNotes?.length && <div role="status" className="border-t border-amber-500/20 px-3.5 py-2 text-xs text-amber-200">
        {shownNotes.map((note, index) => <p className="whitespace-pre-wrap break-words" key={index}>{note}</p>)}
      </div>}

      {/* Error Footer */}
      {error && (
        <div className="border-t border-red-500/20 px-3.5 py-2 text-[11px] text-red-300 bg-red-950/40 rounded-b-2xl">
          {error}
        </div>
      )}
    </div>
  );
}
