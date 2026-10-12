import { useEffect, type ReactNode, type RefObject } from "react";
import { ChevronRight, RotateCcw } from "./icons";
import type { PanoStage } from "../pano/panoStage";

/**
 * 全景类节点（360° 全景查看器、导演世界）共用的「左视口 + 右侧可收起参数栏」。
 *
 * # 为什么是横排
 *
 * 原来参数堆在视口下面，节点又高又窄，调一个滑块要把眼睛从画面挪到下面再挪回来。
 * 照 DramaClaw 的做法改成左右排：画面在左、参数在右，改参数时画面一直在余光里。
 * 参数栏能收起 —— 定好之后只看画面。
 *
 * 宽度：展开时节点约 860，收起约 560。
 */
export const PANEL_WIDTH = 300;

export function nodeWidth(panelOpen: boolean, viewportWidth = 540): number {
  return panelOpen ? viewportWidth + PANEL_WIDTH : viewportWidth;
}

export function PanoSplitLayout({
  height,
  panelOpen,
  onTogglePanel,
  viewport,
  panel,
}: {
  height: number;
  panelOpen: boolean;
  onTogglePanel: () => void;
  viewport: ReactNode;
  panel: ReactNode;
}) {
  return (
    <div
      className="flex w-full overflow-hidden rounded-lg border border-white/[0.08] bg-black"
      style={{ height }}
    >
      <div className="relative min-w-0 flex-1">
        {viewport}
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            onTogglePanel();
          }}
          onPointerDown={(e) => e.stopPropagation()}
          title={panelOpen ? "收起参数栏" : "展开参数栏"}
          className="nodrag absolute right-2 top-2 z-10 inline-flex h-7 w-7 items-center justify-center rounded-full border border-white/10 bg-black/40 text-white/70 backdrop-blur-sm transition-colors hover:bg-black/60 hover:text-white"
        >
          <ChevronRight
            className="h-4 w-4"
            style={{ transform: panelOpen ? undefined : "rotate(180deg)" }}
          />
        </button>
      </div>
      {panelOpen && (
        <div
          // nopan / nowheel：否则在参数栏里滚动会拖动整个画布
          className="nodrag nopan nowheel flex h-full shrink-0 flex-col gap-4 overflow-y-auto overflow-x-hidden border-l border-white/[0.08] bg-[rgb(25,26,31)] p-3.5 text-[12px] text-white/80"
          style={{ width: PANEL_WIDTH }}
          onWheel={(e) => e.stopPropagation()}
        >
          {panel}
        </div>
      )}
    </div>
  );
}

/** 参数栏里的一节。标题左边、读数或「重置」在右边。 */
export function PanelSection({
  title,
  aside,
  children,
}: {
  title: string;
  aside?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="flex flex-col gap-2">
      <header className="flex items-center justify-between gap-3 text-[11px] font-medium text-white/60">
        <span>{title}</span>
        {aside && <span className="font-mono text-[11px] text-white/50">{aside}</span>}
      </header>
      {children}
    </section>
  );
}

export function ResetButton({ onClick, label = "重置" }: { onClick: () => void; label?: string }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="inline-flex items-center gap-1 rounded px-1 text-[11px] text-white/50 hover:text-white/85"
    >
      <RotateCcw className="h-3 w-3" />
      {label}
    </button>
  );
}

/**
 * 滑块 + 数字框。**两个都要有** —— 滑块用来大致拖，数字框用来填准：
 * 「roll 1.5°」这种校正值拖是拖不准的。
 */
export function SliderRow({
  label,
  value,
  min,
  max,
  step = 1,
  unit = "°",
  onChange,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  unit?: string;
  onChange: (v: number) => void;
}) {
  const clamp = (v: number) => Math.max(min, Math.min(max, v));
  // 小数位跟着步长走：0.05 m 一档的相机离地要显示两位，只保留一位会把 1.65 显示成 1.7
  const decimals = (String(step).split(".")[1] ?? "").length;
  const shown = Number(value.toFixed(decimals));
  return (
    <label className="flex items-center gap-2">
      <span className="w-[40px] shrink-0 text-[11px] text-white/45">{label}</span>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => onChange(clamp(Number(e.target.value)))}
        className="nodrag nopan min-w-0 flex-1 accent-white/70"
      />
      <input
        type="number"
        min={min}
        max={max}
        step={step}
        value={shown}
        onChange={(e) => {
          const v = Number(e.target.value);
          if (Number.isFinite(v)) onChange(clamp(v));
        }}
        className="nodrag h-7 w-[56px] shrink-0 rounded-md border border-white/[0.14] bg-transparent px-1.5 text-right font-mono text-[11px] text-white/85 outline-none hover:border-white/25 focus:border-white/40"
      />
      <span className="min-w-[10px] shrink-0 text-[11px] text-white/35">{unit}</span>
    </label>
  );
}

/** 小胶囊按钮，用于预设值（20° / 35° …、正前 / 右 / 后 / 左、画幅）。 */
export function Chip({
  active,
  onClick,
  title,
  disabled,
  children,
}: {
  active?: boolean;
  onClick: () => void;
  title?: string;
  disabled?: boolean;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={title}
      disabled={disabled}
      className={
        "rounded-full border px-2.5 py-1 text-[11px] transition-colors disabled:opacity-40 " +
        (active
          ? "border-white/40 bg-white/15 text-white"
          : "border-white/[0.14] text-white/65 hover:border-white/30 hover:text-white/90")
      }
    >
      {children}
    </button>
  );
}

/**
 * 视口尺寸变了就同步给舞台。
 *
 * **参数栏一收一放，视口宽度就变了**，而舞台只在创建时 `resize` 过一次 ——
 * 不跟着改的话画面被横向拉伸，而且截图的比例也跟着错。
 */
export function useStageAutoResize(
  canvasRef: RefObject<HTMLCanvasElement | null>,
  stageRef: RefObject<PanoStage | null>,
  ready: number,
) {
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => {
      const stage = stageRef.current;
      if (stage && canvas.clientWidth > 0 && canvas.clientHeight > 0) {
        stage.resize(canvas.clientWidth, canvas.clientHeight);
      }
    });
    ro.observe(canvas);
    return () => ro.disconnect();
  }, [canvasRef, stageRef, ready]);
}
