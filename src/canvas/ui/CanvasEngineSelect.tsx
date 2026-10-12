import { useEffect, useRef, useState } from "react";
import { studioCapabilities, type CapabilityCatalog, type EngineSpec } from "../../api/studio";
import { canonicalCanvasEngine, canvasEngines } from "../utils/modelCatalog";
import { Check, ChevronDown } from "./icons";

/** 引擎的小方块图标：没有各家的 logo，用缩写（照 LibTV 的「图标 + 名字」，一眼分得清是哪家） */
export function engineMonogram(id: string): string {
  const known: [RegExp, string][] = [
    [/^h3|minimax/i, "H3"], [/^ltx/i, "LTX"], [/^zimage|z_image/i, "Z"], [/^krea2/i, "K2"],
    [/^sdxl|xl/i, "XL"], [/^qwen/i, "Q"], [/^pony/i, "P"], [/^flux/i, "F"],
  ];
  return known.find(([re]) => re.test(id))?.[1] ?? id.slice(0, 2).toUpperCase();
}

/** 单次大约多久：45s / 2min */
export function engineSecsLabel(secs: number): string {
  if (!secs || secs <= 0) return "";
  return secs < 60 ? `${Math.round(secs)}s` : `${Math.round(secs / 60)}min`;
}

/**
 * 出图 / 出片面板底栏的模型选择 —— 照 LibTV（用户 2026-09-24：「做成统一的独立上下箭头，模型显示图标和截断，减少长度」）。
 *
 * 以前是系统下拉框：箭头是系统的双箭头，和旁边几个下拉长得不一样；名字全写出来（「LTX 2.5 (WeeTodd/MLX)」）占掉半行。
 * 现在按钮是「缩写图标 + 截断的名字 + 统一的向下箭头」，完整名字在悬停提示里；
 * 菜单往上弹，每项是图标、名字、单次耗时。实测说明放在悬停提示里 ——
 * 以前选中 / 悬停的那项下面多一行，列表高度跟着跳（用户 2026-09-25：「会撑大高度，去掉下面的说明」）。
 */
export function CanvasEngineSelect({ mode, alsoMode, value, onChange, disabled }: {
  mode: string;
  /** 再并上一个模式的引擎。连了一张图时既能图生图、也能参考生图，两边的模型都要列出来 */
  alsoMode?: string;
  value: string; onChange: (engine: string) => void; disabled?: boolean;
}) {
  const [catalog, setCatalog] = useState<CapabilityCatalog | null>(null);
  const [error, setError] = useState("");
  const [retry, setRetry] = useState(0);
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    let alive = true;
    setCatalog(null); setError("");
    void studioCapabilities().then((result) => { if (alive) setCatalog(result); })
      .catch((e) => { if (alive) setError(`读取模型目录失败：${e}`); });
    return () => { alive = false; };
  }, [retry]);
  useEffect(() => {
    if (!open) return;
    const close = (e: PointerEvent) => { if (!ref.current?.contains(e.target as Node)) setOpen(false); };
    window.addEventListener("pointerdown", close);
    return () => window.removeEventListener("pointerdown", close);
  }, [open]);
  const choices: EngineSpec[] = catalog
    ? [...canvasEngines(catalog, mode), ...(alsoMode ? canvasEngines(catalog, alsoMode) : [])]
        .filter((e, i, all) => all.findIndex((x) => x.id === e.id) === i)
    : [];
  const selected = canonicalCanvasEngine(value);
  const current = choices.find((e) => e.id === selected);
  const label = current?.label ?? (catalog ? `${value || "未选择"}（不可用）` : error ? "目录读取失败" : "读取模型…");

  return (
    <div className="relative min-w-0" ref={ref}>
      <button
        type="button" aria-label="生成模型" aria-haspopup="listbox" aria-expanded={open} title={label}
        disabled={disabled || !catalog}
        onClick={() => setOpen((v) => !v)}
        className={`nodrag nopan flex h-7 max-w-[150px] items-center gap-1.5 rounded-md px-1.5 text-xs hover:bg-white/10 disabled:opacity-40 ${current ? "text-white/80" : "text-amber-200/80"}`}
      >
        <span className="flex h-4 min-w-4 shrink-0 items-center justify-center rounded bg-white/15 px-0.5 text-[8px] font-bold leading-none text-white/85">
          {engineMonogram(selected || "?")}
        </span>
        <span className="truncate">{label}</span>
        <ChevronDown className="h-3 w-3 shrink-0 text-white/40" />
      </button>
      {error && <button type="button" title={error} onClick={() => setRetry((n) => n + 1)} className="text-xs text-red-300">重试</button>}
      {open && (
        <div role="listbox" aria-label="生成模型"
          className="nodrag nopan nowheel absolute bottom-9 left-0 z-50 flex max-h-80 w-72 flex-col gap-0.5 overflow-y-auto rounded-2xl border border-white/10 bg-zinc-900/95 p-1.5 shadow-2xl backdrop-blur">
          {choices.map((e) => {
            const on = e.id === selected;
            return (
              <button
                key={e.id} type="button" role="option" aria-selected={on} title={e.notes?.[0] ?? e.label}
                onClick={() => { onChange(e.id); setOpen(false); }}
                className={`flex items-center gap-2.5 rounded-xl px-2 py-2 text-left ${on ? "bg-white/10" : "hover:bg-white/10"}`}
              >
                <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg bg-white/10 text-[10px] font-bold text-white/85">
                  {engineMonogram(e.id)}
                </span>
                <span className="min-w-0 flex-1">
                  <span className="flex items-center gap-2">
                    <span className="truncate text-[12px] text-white/90">{e.label}</span>
                    {on && <Check className="h-3 w-3 shrink-0 text-white/70" />}
                  </span>
                </span>
                {engineSecsLabel(e.secs) && (
                  <span className="shrink-0 rounded-md bg-white/10 px-1.5 py-0.5 text-[10px] text-white/55">{engineSecsLabel(e.secs)}</span>
                )}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
