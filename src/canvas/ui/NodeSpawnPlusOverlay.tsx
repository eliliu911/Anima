import { useState, useRef, useEffect } from "react";
import { Handle, Position, useViewport } from "@xyflow/react";
import { Plus } from "./icons";
import { nodeMedia } from "../utils/upscaleChain";
import { useCanvasStore } from "../store/useCanvasStore";
import { INPUT_HANDLE, spawnChoices, type SpawnContext } from "../utils/spawnLinked";
import { SpawnMenu, useSpawnLinked } from "./SpawnMenu";
import type { CanvasNodeType } from "../types";

/**
 * 鼠标离「+」多近就吸过去（屏幕像素）。约 1 厘米 —— 照 LibTV：鼠标靠近时加号跟着鼠标走，
 * 不用对准一个 20 像素的小圆（用户 2026-09-24）。和画布缩放无关，按屏幕距离算
 */
export const MAGNET_PX = 38;

/** 鼠标相对「+」原位的屏幕偏移 → 加号该挪多少（节点内坐标，要除以缩放）。超出吸附范围就回原位 */
export function magnetOffset(dx: number, dy: number, zoom: number, radius = MAGNET_PX): { x: number; y: number } {
  if (Math.hypot(dx, dy) > radius || zoom <= 0) return { x: 0, y: 0 };
  return { x: dx / zoom, y: dy / zoom };
}

interface NodeSpawnPlusOverlayProps {
  nodeId: string;
  selected?: boolean;
  handleId?: string;
  /**
   * 哪一侧。`right`（默认）= 往后接：拖出连线连别的节点的输入，点一下长出下游；
   * `left` = 往前接：拖出连线连别的节点的输出，点一下长出一个连进来的上游（2026-10-10 用户：「左侧没做功能」）
   */
  side?: "right" | "left";
}

/** 菜单要的上下文：这个节点是什么、是不是视频、有没有片（从 store 里的节点算） */
export function spawnContextOf(node: { type?: string; data?: unknown } | undefined): SpawnContext {
  const media = node ? nodeMedia(node as never) : null;
  const isVideo = node?.type === "videoGen" || (node?.type === "upscale" && media?.kind === "video");
  return { type: node?.type as CanvasNodeType | undefined, isVideo, hasVideo: isVideo && media?.kind === "video" };
}

export function NodeSpawnPlusOverlay({
  nodeId,
  selected,
  handleId,
  side = "right",
}: NodeSpawnPlusOverlayProps) {
  const [menuOpen, setMenuOpen] = useState(false);
  const sourceNode = useCanvasStore((s) => s.nodes.find((n) => n.id === nodeId));
  const spawn = useSpawnLinked();
  const left = side === "left";
  const direction = left ? "up" : "down";
  // 左侧的口 = 这个节点的输入口（没有输入口的节点不显示左侧「+」，见 BaseNodeWrapper）
  const ownHandle = left ? INPUT_HANDLE[sourceNode?.type as CanvasNodeType] : handleId;
  const choices = spawnChoices(direction, spawnContextOf(sourceNode));
  const pointerDownPosRef = useRef<{ x: number; y: number; time: number } | null>(null);
  // 吸附跟随：原位锚点不动（量它的位置），加号本身按偏移挪过去
  const anchorRef = useRef<HTMLDivElement>(null);
  const [offset, setOffset] = useState({ x: 0, y: 0 });
  const { zoom } = useViewport();
  const visible = Boolean(selected || menuOpen);
  useEffect(() => {
    // 菜单开着、或者没显示时不跟；拖连线（按着键）时也不跟，否则连线起点一直在动
    if (!visible || menuOpen) { setOffset({ x: 0, y: 0 }); return; }
    const move = (e: PointerEvent) => {
      if (e.buttons) return;
      const r = anchorRef.current?.getBoundingClientRect();
      if (!r) return;
      const next = magnetOffset(e.clientX - (r.left + r.width / 2), e.clientY - (r.top + r.height / 2), zoom);
      setOffset((cur) => (cur.x === next.x && cur.y === next.y ? cur : next));
    };
    window.addEventListener("pointermove", move);
    return () => window.removeEventListener("pointermove", move);
  }, [visible, menuOpen, zoom]);

  useEffect(() => {
    if (!menuOpen) return;
    const handleOutsideClick = () => setMenuOpen(false);
    window.addEventListener("pointerdown", handleOutsideClick);
    return () => window.removeEventListener("pointerdown", handleOutsideClick);
  }, [menuOpen]);

  return (
    <div
      className={`absolute ${left ? "-left-9" : "-right-9"} top-1/2 -translate-y-1/2 z-30 transition-all duration-150 group/handle ${
        visible
          ? "opacity-100 pointer-events-auto"
          : "opacity-0 pointer-events-none"
      }`}
    >
      <div className="relative" ref={anchorRef}>
        {/* 卡片边缘到悬浮加号胶囊的虚线引桥 */}
        <div className={`absolute ${left ? "left-full" : "right-full"} top-1/2 w-4 h-px -translate-y-1/2 border-t border-dashed border-white/30 pointer-events-none`} />

        {/* 可直接拖动连线的加号胶囊。外面这层管吸附位移（Handle 自己带 !transform-none）。
            右侧是输出口（source），左侧是输入口（target）—— 拖到空白处松手的菜单由画布的 onConnectEnd 弹 */}
        <div style={{ transform: `translate(${offset.x}px, ${offset.y}px)` }} className="transition-transform duration-100 ease-out">
        <Handle
          type={left ? "target" : "source"}
          position={left ? Position.Left : Position.Right}
          id={`${ownHandle || (left ? "in" : "out")}-plus`}
          onPointerDown={(e) => {
            pointerDownPosRef.current = {
              x: e.clientX,
              y: e.clientY,
              time: Date.now(),
            };
          }}
          onClick={(e) => {
            if (pointerDownPosRef.current) {
              const dx = e.clientX - pointerDownPosRef.current.x;
              const dy = e.clientY - pointerDownPosRef.current.y;
              const dist = Math.sqrt(dx * dx + dy * dy);
              const duration = Date.now() - pointerDownPosRef.current.time;
              // 拖动位移超过 5px 或长按超过 400ms 属于连线手势，不触发点击菜单
              if (dist > 5 || duration > 400) {
                return;
              }
            }
            e.stopPropagation();
            setMenuOpen((prev) => !prev);
          }}
          className="!static !transform-none !flex !h-5 !w-5 !items-center !justify-center !rounded-full !border !border-white/30 !bg-[#18181b] !text-white/90 !shadow-xl transition-all duration-150 hover:!scale-110 hover:!border-blue-400 hover:!text-white hover:!shadow-[0_0_12px_rgba(59,130,246,0.6)] cursor-crosshair active:scale-95"
          title={left ? "拖动连接上游节点 · 点击新建一个连进来的节点" : "拖动连接下游节点 · 点击快速衍生"}
        >
          <Plus className="h-3 w-3 stroke-[2.5] pointer-events-none" />
        </Handle>
        </div>

        {menuOpen && (
          <SpawnMenu
            title={left ? "生成后接入该节点" : "引用该节点生成"}
            choices={choices}
            className={`absolute ${left ? "right-7" : "left-7"} top-1/2 -translate-y-1/2`}
            onPick={(c) => {
              setMenuOpen(false);
              spawn(direction, nodeId, ownHandle, c);
            }}
          />
        )}
      </div>
    </div>
  );
}
