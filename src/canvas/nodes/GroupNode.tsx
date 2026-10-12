/**
 * 组节点的界面。
 *
 * 改写自 DramaClaw（ClaymoreLab, Elastic License 2.0）
 * frontend/src/features/canvas/nodes/GroupNode.tsx 里「普通组」那部分：
 * 浮在框上沿的标题、组配色、成员几何一变就自动包住。分镜组那一大半没搬。
 * 出处规则见 DECISIONS.md 2026-09-16。
 */
import { memo, useEffect, useMemo, useState } from "react";
import { NodeResizer, type NodeProps } from "@xyflow/react";
import { Layers } from "lucide-react";
import { useShallow } from "zustand/react/shallow";
import { useCanvasStore } from "../store/useCanvasStore";
import { ComposingInput } from "../../components/ComposingInput";
import { GROUP_COLORS, groupBackground, groupBorder } from "../group";
import type { GroupNode } from "../types";

export const GroupNodeComponent = memo(function GroupNodeComponent({
  id,
  data,
  selected,
  dragging,
}: NodeProps<GroupNode>) {
  const updateNodeData = useCanvasStore((s) => s.updateNodeData);
  const ungroupNode = useCanvasStore((s) => s.ungroupNode);
  const fitGroup = useCanvasStore((s) => s.fitGroup);
  const [editing, setEditing] = useState(false);

  /**
   * 成员的位置和尺寸签名。**变了才重新包框** —— 覆盖图片加载完节点变高、
   * 参数栏展开节点变宽这类不经过拖动的尺寸变化。
   */
  const childrenDragging = useCanvasStore((s) => s.nodes.some((n) => n.parentId === id && n.dragging));
  const childSignature = useCanvasStore(
    useShallow((s) =>
      s.nodes
        .filter((n) => n.parentId === id)
        .map(
          (n) =>
            `${n.id}:${Math.round(n.position.x)},${Math.round(n.position.y)},` +
            `${Math.round(n.measured?.width ?? n.width ?? 0)},${Math.round(n.measured?.height ?? n.height ?? 0)}`,
        )
        .join("|"),
    ),
  );
  const childCount = useMemo(() => (childSignature ? childSignature.split("|").length : 0), [childSignature]);

  useEffect(() => {
    // 拖动中不包 —— 每一帧都改组框位置会和 ReactFlow 的拖动打架；松手后 CanvasView 统一收尾
    if (dragging || childrenDragging || !childSignature) return;
    fitGroup(id);
  }, [childSignature, dragging, childrenDragging, fitGroup, id]);

  const label = data.label || "组";

  return (
    <div
      className="relative h-full w-full rounded-[18px] border"
      style={{
        backgroundColor: groupBackground(data.color) ?? "rgba(255,255,255,0.025)",
        borderColor: selected ? "rgba(255,255,255,0.55)" : (groupBorder(data.color) ?? "rgba(255,255,255,0.12)"),
      }}
    >
      <NodeResizer
        isVisible={selected}
        minWidth={220}
        minHeight={140}
        lineClassName="!border-transparent"
        handleClassName="!h-2.5 !w-2.5 !rounded-sm !border !border-white/60 !bg-zinc-900"
      />

      {/* 标题浮在框上沿外面：框里留给成员，标题不占位 */}
      <div className="absolute -top-8 left-0 flex max-w-full items-center gap-2 text-xs text-white/70">
        <Layers className="h-3.5 w-3.5 shrink-0 text-white/50" />
        {editing ? (
          <ComposingInput
            autoFocus
            value={label}
            onCommit={(v) => updateNodeData(id, { label: v })}
            onBlur={() => setEditing(false)}
            onKeyDown={(e) => {
              e.stopPropagation();
              if (e.key === "Enter" || e.key === "Escape") setEditing(false);
            }}
            className="nodrag nopan w-40 rounded bg-white/10 px-1.5 py-0.5 text-xs text-white outline-none"
          />
        ) : (
          <span
            className="truncate font-medium text-white/80"
            onDoubleClick={(e) => {
              e.stopPropagation();
              setEditing(true);
            }}
            title="双击改名"
          >
            {label}
          </span>
        )}
        <span className="shrink-0 text-white/35">{childCount} 个</span>

        {selected && (
          <div className="nodrag nopan flex shrink-0 items-center gap-1 pl-1">
            {GROUP_COLORS.map((c) => (
              <button
                key={c.key}
                type="button"
                title={c.label}
                onClick={(e) => {
                  e.stopPropagation();
                  updateNodeData(id, { color: data.color === c.value ? undefined : c.value });
                }}
                className="h-3 w-3 rounded-full border"
                style={{
                  background: c.value,
                  borderColor: data.color === c.value ? "rgba(255,255,255,0.9)" : "transparent",
                }}
              />
            ))}
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                ungroupNode(id);
              }}
              className="ml-1 rounded px-1.5 py-0.5 text-[11px] text-white/60 hover:bg-white/10 hover:text-white"
              title="解组 (⇧⌘G)"
            >
              解组
            </button>
          </div>
        )}
      </div>
    </div>
  );
});
