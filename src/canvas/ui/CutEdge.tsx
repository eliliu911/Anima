import { useEffect, useRef, useState } from "react";
import { BaseEdge, EdgeLabelRenderer, getBezierPath, type EdgeProps } from "@xyflow/react";
import { useCanvasStore } from "../store/useCanvasStore";
import { useCanvasToolStore } from "../store/canvasToolStore";
import type { CanvasReference, VideoGenNodeData } from "../types";
import { Scissors } from "./icons";
import "./CutEdge.css";

/**
 * 断开一根连线。**连同目标节点上绑着这个上游的参考一起去掉** —— 只删线的话，参考图还挂在节点上照样发出去，
 * 用户看着线没了、生成出来却还带着那张图。同一个上游还有别的线连着就留着参考。
 */
export function cutEdge(edgeId: string) {
  const store = useCanvasStore.getState();
  const edge = store.edges.find((e) => e.id === edgeId);
  if (!edge) return;
  store.onEdgesChange([{ id: edgeId, type: "remove" }]);
  const still = useCanvasStore.getState().edges.some((e) => e.source === edge.source && e.target === edge.target);
  if (still) return;
  const target = useCanvasStore.getState().nodes.find((n) => n.id === edge.target);
  const refs = ((target?.data as VideoGenNodeData | undefined)?.references ?? []) as CanvasReference[];
  const left = refs.filter((r) => r.sourceNodeId !== edge.source);
  if (left.length !== refs.length) useCanvasStore.getState().updateNodeData(edge.target, { references: left });
}

/** 彗星一圈多长（px）。线上每隔这么长走一颗 */
export const COMET_PERIOD = 180;
/**
 * 彗星的层：从头到尾越来越长、越来越淡、越来越细。**每层的头对齐在同一个位置**，
 * 叠出来就是「头亮、从中间往尾巴渐隐」（SVG 没法沿路径做渐变，只能这样叠）。
 */
export const COMET_LAYERS: { len: number; opacity: number; width: number }[] = [
  { len: 64, opacity: 0.1, width: 1.6 },
  { len: 44, opacity: 0.2, width: 2 },
  { len: 28, opacity: 0.35, width: 2.4 },
  { len: 16, opacity: 0.6, width: 2.8 },
  { len: 7, opacity: 1, width: 3.2 },
];

/**
 * 每层的 dashoffset 动画区间。dash 从 `-offset` 画到 `-offset + len`，头在 `len - offset`；
 * 要所有层的头重合，offset 就得是 `len - 头的位置` —— 从 `len + 周期` 走到 `len`，头往终点方向走一个周期。
 */
export function cometOffsets(len: number, period = COMET_PERIOD): [number, number] {
  return [len + period, len];
}

const REDUCED = "(prefers-reduced-motion: reduce)";

/**
 * 系统开没开「减少动态效果」。**跟着系统设置实时变**（切了不用重启）。
 */
export function useSystemReducedMotion(): boolean {
  const [on, setOn] = useState(() => typeof window !== "undefined" && Boolean(window.matchMedia?.(REDUCED).matches));
  useEffect(() => {
    const mq = window.matchMedia?.(REDUCED);
    if (!mq) return;
    const f = () => setOn(mq.matches);
    mq.addEventListener?.("change", f);
    return () => mq.removeEventListener?.("change", f);
  }, []);
  return on;
}

/**
 * 连线要不要用简洁样式：系统设置**或**画布底栏的「减少动态效果」按钮，任一个开着就是。
 *
 * 简洁样式 = 旧样式（用户 2026-09-25）：只有实线和虚线 —— 没有光晕、没有彗星；
 * 选中节点、或节点在跑时，那几根线是移动的虚线。
 */
export function useReducedMotion(): boolean {
  const system = useSystemReducedMotion();
  const app = useCanvasToolStore((s) => s.reduceMotion);
  return system || app;
}

/** 这根线底下那条 path 挂哪些类名。`plain` = 减少动态效果：不要光晕、流动用虚线 */
export function edgeClasses({ active, running, hover, plain }: { active: boolean; running: boolean; hover: boolean; plain: boolean }): string | undefined {
  const c = plain
    ? ["canvas-edge-plain", (active || running) && "canvas-edge-dash"]
    : [active && "canvas-edge-active", hover && "canvas-edge-hover"];
  return c.filter(Boolean).join(" ") || undefined;
}

/** 顺着线走的彗星：选中节点的连线、正在跑的节点的连线都用它 */
function Comet({ path }: { path: string }) {
  return (
    <g className="canvas-edge-comet" pointerEvents="none">
      {COMET_LAYERS.map((l) => {
        const [from, to] = cometOffsets(l.len);
        return (
          // 虚线写在 style 里：React Flow 自带的 `.react-flow__edge.animated path` 会给线上**每一根** path 套虚线和动画，
          // 样式表压得过 SVG 属性、压不过行内样式。dashoffset 留在属性上 —— 写进 style 的话 <animate> 就动不了它
          <path key={l.len} d={path} fill="none" strokeLinecap="round" stroke="#60a5fa" strokeOpacity={l.opacity} strokeWidth={l.width}
            strokeDashoffset={to} style={{ strokeDasharray: `${l.len} ${COMET_PERIOD - l.len}`, animation: "none" }}>
            <animate attributeName="stroke-dashoffset" from={from} to={to} dur="1.4s" repeatCount="indefinite" />
          </path>
        );
      })}
    </g>
  );
}

/**
 * 画布的连线：原来的贝塞尔曲线，**鼠标移到线上时正中出一把剪刀**，点了剪断（照 LibTV，用户 2026-09-24）。
 * 以前断线要先点中那根线再按删除键，线细、叠在一起时很难点中。
 *
 * 选中节点时，它的连线变亮、上面走一颗蓝色彗星（头亮、带光晕、尾巴渐隐），一眼看出这个节点连着谁。
 * 节点在跑的时候它的连线也走彗星 —— 以前是变成虚线，用户 2026-09-25 说那是 bug。
 */
export function CutEdge(props: EdgeProps) {
  const { id, source, target, sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, style, markerEnd, interactionWidth } = props;
  // 选中节点时，连着它的线（上游、下游都算）变亮、有蓝色光点顺着线流（照 LibTV，用户 2026-09-25）。
  // 只订阅「选中的是不是我的两端」这一个布尔 —— 拖动、改数据都不会让每根线重渲染
  const active = useCanvasStore((s) => s.selectedNodeId !== null && (s.selectedNodeId === source || s.selectedNodeId === target));
  // 两端有一个在跑
  const running = useCanvasStore((s) => s.nodes.some((n) => (n.id === source || n.id === target) && Boolean((n.data as { isGenerating?: boolean }).isGenerating)));
  const [hover, setHover] = useState(false);
  const plain = useReducedMotion();
  // 剪刀画在 HTML 层（连线外面）：鼠标从线挪到剪刀上会先「离开连线」。立刻收起的话剪刀一消失鼠标又落回线上，来回闪 ——
  // 离开后等 200ms 再收，这期间进了剪刀就取消
  const hideTimer = useRef<number | undefined>(undefined);
  const show = () => { window.clearTimeout(hideTimer.current); setHover(true); };
  const hide = () => { window.clearTimeout(hideTimer.current); hideTimer.current = window.setTimeout(() => setHover(false), 200); };
  const [path, labelX, labelY] = getBezierPath({ sourceX, sourceY, sourcePosition, targetX, targetY, targetPosition });
  return (
    <g onMouseEnter={show} onMouseLeave={hide}>
      <BaseEdge id={id} path={path} style={style} className={edgeClasses({ active, running, hover, plain })} markerEnd={markerEnd} interactionWidth={interactionWidth ?? 24} />
      {!plain && (active || running) && <Comet path={path} />}
      {hover && (
        <EdgeLabelRenderer>
          <button
            type="button" title="剪断这根连线" aria-label="剪断连线"
            onMouseEnter={show} onMouseLeave={hide}
            onClick={(e) => { e.stopPropagation(); cutEdge(id); }}
            className="nodrag nopan pointer-events-auto absolute flex h-7 w-7 items-center justify-center rounded-full border border-white/20 bg-zinc-900/95 text-white/80 shadow-lg hover:border-red-400/70 hover:text-red-300"
            style={{ transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY}px)` }}
          >
            <Scissors width={14} height={14} />
          </button>
        </EdgeLabelRenderer>
      )}
    </g>
  );
}
