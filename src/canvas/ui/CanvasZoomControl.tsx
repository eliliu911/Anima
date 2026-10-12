import { useState, useRef, useEffect, useCallback } from "react";
import { useReactFlow, useViewport } from "@xyflow/react";
import {
  Minus,
  Plus,
  Maximize,
  Map,
  Wand2,
  Grid3X3,
  Eye,
  EyeOff,
  Hand,
  MousePointer2,
  Touchpad,
  Zap,
  ZapOff,
  ChevronsLeft,
  ChevronsRight,
} from "lucide-react";
import { useCanvasStore } from "../store/useCanvasStore";
import { useCanvasToolStore } from "../store/canvasToolStore";
import { ARRANGE_MODES } from "../utils/arrange";
import { useSystemReducedMotion } from "./CutEdge";

interface CanvasZoomControlProps {
  showMiniMap: boolean;
  onToggleMiniMap: () => void;
}

const ZOOM_PRESETS = [50, 100, 200];

export function CanvasZoomControl({
  showMiniMap,
  onToggleMiniMap,
}: CanvasZoomControlProps) {
  const { zoomTo, getZoom, fitView } = useReactFlow();
  const { zoom } = useViewport();
  const percent = Math.round(zoom * 100);

  const [menuOpen, setMenuOpen] = useState(false);
  const [draft, setDraft] = useState<string>("");
  const rootRef = useRef<HTMLDivElement | null>(null);
  const inputRef = useRef<HTMLInputElement | null>(null);

  const snapToGrid = useCanvasStore((s) => s.snapToGrid);
  const toggleSnapToGrid = useCanvasStore((s) => s.toggleSnapToGrid);
  const showEdges = useCanvasStore((s) => s.showEdges);
  const toggleShowEdges = useCanvasStore((s) => s.toggleShowEdges);
  const arrangeNodes = useCanvasStore((s) => s.arrangeNodes);
  const arrangeMode = useCanvasStore((s) => s.arrangeMode);
  const [arrangeOpen, setArrangeOpen] = useState(false);
  const arrangeRef = useRef<HTMLDivElement>(null);
  // 点菜单外面就收起
  useEffect(() => {
    if (!arrangeOpen) return;
    const close = (e: PointerEvent) => { if (!arrangeRef.current?.contains(e.target as Node)) setArrangeOpen(false); };
    window.addEventListener("pointerdown", close);
    return () => window.removeEventListener("pointerdown", close);
  }, [arrangeOpen]);
  const tool = useCanvasToolStore((s) => s.tool);
  const setTool = useCanvasToolStore((s) => s.setTool);
  const trackpadPan = useCanvasToolStore((s) => s.trackpadPan);
  const toggleTrackpadPan = useCanvasToolStore((s) => s.toggleTrackpadPan);
  const reduceMotion = useCanvasToolStore((s) => s.reduceMotion);
  const toggleReduceMotion = useCanvasToolStore((s) => s.toggleReduceMotion);
  const systemReduced = useSystemReducedMotion();
  // 收起时只留缩放 / 适合屏幕 / 小地图（用户 2026-09-25：「不然这个条越来越长」）
  const collapsed = useCanvasToolStore((s) => s.toolbarCollapsed);
  const toggleCollapsed = useCanvasToolStore((s) => s.toggleToolbarCollapsed);

  const handleZoomIn = useCallback(() => {
    const next = getZoom() * 1.2;
    zoomTo(Math.min(next, 8), { duration: 140 });
  }, [getZoom, zoomTo]);

  const handleZoomOut = useCallback(() => {
    const next = getZoom() / 1.2;
    zoomTo(Math.max(next, 0.1), { duration: 140 });
  }, [getZoom, zoomTo]);

  const handleFitView = useCallback(() => {
    void fitView({ padding: 0.2, maxZoom: 1, duration: 200 });
  }, [fitView]);

  const handleZoomToPercent = useCallback(
    (value: number) => {
      const clamped = Math.min(8, Math.max(0.1, value / 100));
      zoomTo(clamped, { duration: 160 });
    },
    [zoomTo],
  );

  // ⌘+ / ⌘− / ⌘0 统一在 CanvasView 里按 `resolveCanvasShortcut` 分发。
  // 原来这里另挂了一份监听，挪走之后不删的话一次按键会缩放两次。

  // 点击外部关闭弹层
  useEffect(() => {
    if (!menuOpen) return;
    const handlePointerDown = (e: PointerEvent) => {
      if (rootRef.current?.contains(e.target as Node)) return;
      setMenuOpen(false);
    };
    window.addEventListener("pointerdown", handlePointerDown);
    return () => window.removeEventListener("pointerdown", handlePointerDown);
  }, [menuOpen]);

  const openMenu = () => {
    setDraft(String(percent));
    setMenuOpen(true);
    window.requestAnimationFrame(() => inputRef.current?.select());
  };

  const commitDraft = () => {
    const value = Number.parseFloat(draft);
    if (Number.isFinite(value) && value > 0) {
      handleZoomToPercent(value);
    }
  };

  return (
    <div
      ref={rootRef}
      // 右下角的状态卡要让开它（见 studio/Toasts.tsx 的 TOAST_AVOID_ATTR）
      data-toast-avoid=""
      className="nodrag nopan nowheel absolute bottom-6 right-6 z-30 select-none flex items-center"
      onPointerDown={(e) => e.stopPropagation()}
    >
      {/* 预设与缩放菜单弹窗：无边框深色质感，融入画布 */}
      {menuOpen && (
        <div
          role="menu"
          className="absolute bottom-full right-0 mb-2 w-48 rounded-xl bg-[#141416]/95 p-1.5 shadow-2xl backdrop-blur-2xl text-xs flex flex-col gap-1 animate-in fade-in-0 zoom-in-95"
        >
          <div className="flex items-center rounded-lg bg-white/5 px-2.5 py-1 mb-1">
            <input
              ref={inputRef}
              value={draft}
              onChange={(e) => setDraft(e.target.value.replace(/[^\d.]/g, ""))}
              onKeyDown={(e) => {
                e.stopPropagation();
                if (e.key === "Enter") {
                  commitDraft();
                  setMenuOpen(false);
                } else if (e.key === "Escape") {
                  setMenuOpen(false);
                }
              }}
              onBlur={commitDraft}
              inputMode="decimal"
              className="w-full bg-transparent text-xs tabular-nums text-white outline-none"
              placeholder="输入比例"
            />
            <span className="text-white/40 text-xs">%</span>
          </div>

          <button
            type="button"
            className="flex w-full items-center justify-between rounded-lg px-2.5 py-1.5 text-left text-white/80 hover:bg-white/10 hover:text-white transition-colors"
            onClick={() => {
              handleZoomIn();
              setMenuOpen(false);
            }}
          >
            <span>放大</span>
            <span className="text-[10px] text-white/40">⌘ +</span>
          </button>

          <button
            type="button"
            className="flex w-full items-center justify-between rounded-lg px-2.5 py-1.5 text-left text-white/80 hover:bg-white/10 hover:text-white transition-colors"
            onClick={() => {
              handleZoomOut();
              setMenuOpen(false);
            }}
          >
            <span>缩小</span>
            <span className="text-[10px] text-white/40">⌘ -</span>
          </button>

          <button
            type="button"
            className="flex w-full items-center justify-between rounded-lg px-2.5 py-1.5 text-left text-white/80 hover:bg-white/10 hover:text-white transition-colors"
            onClick={() => {
              handleFitView();
              setMenuOpen(false);
            }}
          >
            <span>适合屏幕</span>
            <span className="text-[10px] text-white/40">⌘ 0</span>
          </button>

          <div className="my-0.5 h-px bg-white/10" />

          {ZOOM_PRESETS.map((p) => (
            <button
              key={p}
              type="button"
              className="flex w-full items-center justify-between rounded-lg px-2.5 py-1 text-left text-white/70 hover:bg-white/10 hover:text-white transition-colors"
              onClick={() => {
                handleZoomToPercent(p);
                setMenuOpen(false);
              }}
            >
              <span>{p}%</span>
            </button>
          ))}
        </div>
      )}

      {/* 底部横向统一控制栏：去掉生硬边框，半透明深色玻璃，无缝融入画布 */}
      <div className="flex items-center gap-1 rounded-xl bg-[#18181b]/70 backdrop-blur-xl px-1.5 py-1 text-xs text-zinc-400 shadow-xl">
        {/* 收起 / 展开：收起时只留缩放、适合屏幕、小地图 */}
        <button
          type="button"
          onClick={toggleCollapsed}
          aria-label={collapsed ? "展开工具栏" : "收起工具栏"}
          aria-expanded={!collapsed}
          className="flex h-7 w-5 items-center justify-center rounded-lg text-zinc-500 hover:bg-white/10 hover:text-zinc-200 transition-colors"
          title={collapsed ? "展开工具栏" : "收起工具栏（只留缩放、适合屏幕和小地图）"}
        >
          {collapsed ? <ChevronsLeft className="h-3.5 w-3.5" /> : <ChevronsRight className="h-3.5 w-3.5" />}
        </button>

        {!collapsed && (<>
        {/* 0. 指针工具：V 移动（左键拖框选）/ H 抓手（左键拖平移） */}
        <button
          type="button"
          onClick={() => setTool("move")}
          className={`flex h-7 w-7 items-center justify-center rounded-lg transition-colors ${
            tool === "move" ? "bg-white/15 text-zinc-100" : "hover:bg-white/10 hover:text-zinc-200"
          }`}
          title="移动工具 (V) · 左键拖空白处框选"
        >
          <MousePointer2 className="h-3.5 w-3.5" />
        </button>
        <button
          type="button"
          onClick={() => setTool("hand")}
          className={`flex h-7 w-7 items-center justify-center rounded-lg transition-colors ${
            tool === "hand" ? "bg-white/15 text-zinc-100" : "hover:bg-white/10 hover:text-zinc-200"
          }`}
          title="抓手工具 (H) · 左键拖动平移画布"
        >
          <Hand className="h-3.5 w-3.5" />
        </button>
        {/* 触控板平移：开 = 滚轮/双指滑动平移、捏合缩放；关 = 滚轮缩放 */}
        <button
          type="button"
          onClick={toggleTrackpadPan}
          className={`flex h-7 w-7 items-center justify-center rounded-lg transition-colors ${
            trackpadPan ? "bg-white/15 text-zinc-100" : "hover:bg-white/10 hover:text-zinc-200"
          }`}
          title={
            trackpadPan
              ? "触控板平移：开 —— 滚轮 / 双指滑动平移，捏合或 ⌘+滚轮缩放。点击改成滚轮缩放"
              : "触控板平移：关 —— 滚轮缩放。点击改成滚轮 / 双指滑动平移"
          }
        >
          <Touchpad className="h-3.5 w-3.5" />
        </button>

        <div className="mx-0.5 h-3.5 w-px bg-white/10" />

        {/* 1. 画布整理：点开选排法（横向 / 纵向 / 网格）；⌥⇧F 按上一次的排法 */}
        <div className="relative" ref={arrangeRef}>
          <button
            type="button"
            onClick={() => setArrangeOpen((v) => !v)}
            className={`flex h-7 w-7 items-center justify-center rounded-lg transition-colors ${arrangeOpen ? "bg-white/15 text-zinc-100" : "hover:bg-white/10 hover:text-zinc-200"}`}
            title={`自动整理画布 (⌥ ⇧ F，按上次的「${ARRANGE_MODES.find((m) => m.id === arrangeMode)?.label}」)`}
            aria-haspopup="menu"
            aria-expanded={arrangeOpen}
          >
            <Wand2 className="h-3.5 w-3.5" />
          </button>
          {arrangeOpen && (
            <div role="menu" aria-label="整理方式"
              className="absolute bottom-9 right-0 z-50 flex w-52 flex-col gap-0.5 rounded-xl border border-white/10 bg-zinc-900/95 p-1 shadow-xl backdrop-blur">
              {ARRANGE_MODES.map((m) => (
                <button
                  key={m.id} type="button" role="menuitem"
                  onClick={() => { arrangeNodes(m.id); setArrangeOpen(false); }}
                  className={`rounded-lg px-2.5 py-1.5 text-left hover:bg-white/10 ${m.id === arrangeMode ? "text-zinc-100" : "text-zinc-300"}`}
                >
                  <span className="block text-[12px]">{m.label}{m.id === arrangeMode && <span className="ml-1.5 text-[10px] text-white/40">上次用的</span>}</span>
                  <span className="block text-[10px] leading-snug text-white/40">{m.hint}</span>
                </button>
              ))}
            </div>
          )}
        </div>

        {/* 2. 网格吸附 */}
        <button
          type="button"
          onClick={() => toggleSnapToGrid()}
          className={`flex h-7 w-7 items-center justify-center rounded-lg transition-colors ${
            snapToGrid
              ? "bg-white/15 text-zinc-100"
              : "hover:bg-white/10 hover:text-zinc-200"
          }`}
          title={snapToGrid ? "网格吸附：开启" : "网格吸附：关闭"}
        >
          <Grid3X3 className="h-3.5 w-3.5" />
        </button>

        {/* 3. 显示/隐藏连线 */}
        <button
          type="button"
          onClick={() => toggleShowEdges()}
          className={`flex h-7 w-7 items-center justify-center rounded-lg transition-colors ${
            !showEdges
              ? "bg-amber-500/20 text-amber-300"
              : "hover:bg-white/10 hover:text-zinc-200"
          }`}
          title={showEdges ? "隐藏画布连线" : "显示画布连线"}
        >
          {showEdges ? <Eye className="h-3.5 w-3.5" /> : <EyeOff className="h-3.5 w-3.5" />}
        </button>

        {/* 4. 减少动态效果：连线回到旧样式（只有实线和虚线）。系统设置开着时按钮点不动 —— 关不掉系统的 */}
        <button
          type="button"
          onClick={toggleReduceMotion}
          disabled={systemReduced}
          aria-pressed={reduceMotion || systemReduced}
          className={`flex h-7 w-7 items-center justify-center rounded-lg transition-colors disabled:cursor-not-allowed ${
            reduceMotion || systemReduced ? "bg-white/15 text-zinc-100" : "hover:bg-white/10 hover:text-zinc-200"
          }`}
          title={
            systemReduced
              ? "减少动态效果：系统设置里开着（要关去 系统设置 › 辅助功能 › 显示）"
              : reduceMotion
                ? "减少动态效果：开 —— 连线只有实线和虚线，选中 / 生成中的线是移动虚线。点击恢复光晕和彗星"
                : "减少动态效果：关。点击后连线只用实线和虚线，没有光晕和彗星"
          }
        >
          {reduceMotion || systemReduced ? <ZapOff className="h-3.5 w-3.5" /> : <Zap className="h-3.5 w-3.5" />}
        </button>

        <div className="mx-0.5 h-3.5 w-px bg-white/10" />
        </>)}

        {/* 缩小 */}
        <button
          type="button"
          onClick={handleZoomOut}
          className="flex h-7 w-7 items-center justify-center rounded-lg hover:bg-white/10 hover:text-zinc-200 transition-colors"
          title="缩小 (⌘ -)"
        >
          <Minus className="h-3.5 w-3.5" />
        </button>

        {/* 屏幕缩放百分比 */}
        <button
          type="button"
          onClick={() => (menuOpen ? setMenuOpen(false) : openMenu())}
          className="min-w-[42px] rounded-lg px-1.5 py-1 text-center text-xs font-mono tabular-nums text-zinc-200 hover:bg-white/10 transition-colors"
          title="缩放比例 / 菜单"
        >
          {percent}%
        </button>

        {/* 放大 */}
        <button
          type="button"
          onClick={handleZoomIn}
          className="flex h-7 w-7 items-center justify-center rounded-lg hover:bg-white/10 hover:text-zinc-200 transition-colors"
          title="放大 (⌘ +)"
        >
          <Plus className="h-3.5 w-3.5" />
        </button>

        <div className="mx-0.5 h-3.5 w-px bg-white/10" />

        {/* 适合屏幕 */}
        <button
          type="button"
          onClick={handleFitView}
          className="flex h-7 w-7 items-center justify-center rounded-lg hover:bg-white/10 hover:text-zinc-200 transition-colors"
          title="适合屏幕 (⌘ 0)"
        >
          <Maximize className="h-3.5 w-3.5" />
        </button>

        <div className="mx-0.5 h-3.5 w-px bg-white/10" />

        {/* 小地图 */}
        <button
          type="button"
          onClick={onToggleMiniMap}
          className={`flex h-7 w-7 items-center justify-center rounded-lg transition-colors ${
            showMiniMap
              ? "bg-purple-500/25 text-purple-300"
              : "hover:bg-white/10 hover:text-zinc-200"
          }`}
          title={showMiniMap ? "收起小地图 (M)" : "打开小地图 (M)"}
        >
          <Map className="h-3.5 w-3.5" />
        </button>
      </div>
    </div>
  );
}
