/**
 * 「长出一个连好线的节点」的菜单 + 执行。右侧「+」、左侧「+」、拖线松手三处共用（见 `utils/spawnLinked.ts`）。
 */
import { useCallback } from "react";
import { FileText, Film, ImageIcon, Layers, Maximize2, Music, Video } from "./icons";
import { useCanvasStore } from "../store/useCanvasStore";
import { spawnVideoTool } from "../utils/imageTools";
import { avoidOverlap, linkFor, nodeWidthOf, spawnPosition, type SpawnChoice, type SpawnDirection } from "../utils/spawnLinked";
import { absolutePosition } from "../group";

const ICON: Record<string, typeof FileText> = {
  prompt: FileText,
  imageGen: ImageIcon,
  videoGen: Video,
  audioClip: Music,
  audio: Music,
  depthCapture: Layers,
  upscale: Maximize2,
};

/** 建节点 + 连线。`drop` = 拖线松手处的画布坐标；不给就按方向放在旁边 */
export function useSpawnLinked() {
  return useCallback((direction: SpawnDirection, fromId: string, handle: string | null | undefined, choice: SpawnChoice, drop?: { x: number; y: number }) => {
    const store = useCanvasStore.getState();
    const from = store.nodes.find((n) => n.id === fromId);
    if (!from) return;
    if (choice.action === "interpolate") {
      spawnVideoTool(fromId, "interpolate");
      return;
    }
    // 分组里的节点 `position` 是相对分组的 —— 按画布绝对坐标摆，再躲开已有的节点
    const byId = new Map(store.nodes.map((n) => [n.id, n]));
    const w = nodeWidthOf(choice.type);
    const want = spawnPosition(direction, { position: absolutePosition(from, byId), width: from.measured?.width }, w, drop);
    const others = store.nodes
      .filter((n) => n.type !== "group")
      .map((n) => ({ ...absolutePosition(n, byId), w: n.measured?.width ?? 440, h: n.measured?.height ?? 300 }));
    // 拖线松手的放在松手处（位置是用户自己挑的，2026-10-10 实测躲让会把它挤到画面外）；点「+」的才自动躲开别的节点
    const pos = drop ? want : avoidOverlap(want, { w, h: 280 }, others);
    const id = store.addNode(choice.type, pos);
    const link = linkFor(direction, { id: fromId, type: from.type, handle }, { id, type: choice.type });
    if (link) useCanvasStore.getState().onConnect(link);
  }, []);
}

export function SpawnMenu({
  title,
  choices,
  onPick,
  className = "",
  style,
}: {
  title: string;
  choices: SpawnChoice[];
  onPick: (c: SpawnChoice) => void;
  className?: string;
  style?: React.CSSProperties;
}) {
  return (
    <div
      role="menu"
      aria-label={title}
      className={`nodrag nopan nowheel z-50 flex min-w-[168px] flex-col gap-1 rounded-xl border border-white/10 bg-[#232428] p-1.5 shadow-2xl cursor-default [-webkit-font-smoothing:antialiased] ${className}`}
      style={style}
      onPointerDown={(e) => e.stopPropagation()}
      onMouseDown={(e) => e.stopPropagation()}
      onClick={(e) => e.stopPropagation()}
    >
      <div className="px-2 py-1 text-[11px] text-white/40">{title}</div>
      {choices.length === 0 && <div className="px-2 py-1.5 text-xs text-white/40">这里接不了别的节点</div>}
      {choices.map((c) => {
        const I = c.action === "interpolate" ? Film : ICON[c.type] ?? FileText;
        return (
          <button
            key={`${c.type}-${c.label}`}
            type="button"
            role="menuitem"
            title={c.hint}
            onClick={() => onPick(c)}
            className="flex items-center gap-2 whitespace-nowrap rounded-lg px-2 py-1.5 text-[13px] text-white/85 transition-colors hover:bg-white/10 hover:text-white"
          >
            <I className="h-3.5 w-3.5 text-white/60" />
            <span>{c.label}</span>
          </button>
        );
      })}
    </div>
  );
}
