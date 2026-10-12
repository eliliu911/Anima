import { useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { CAMERA_MOVES, MOTION_CATEGORIES, cameraMovesIn, cameraPreviewUrl, cameraShortName, type MotionPresetItem } from "../utils/cameraMoves";
import { Check, X } from "./icons";
import { MediaThumb } from "../../components/studio/MediaThumb";
import { PlayAllToggle, usePlayAll } from "./PlayAllToggle";

/**
 * 运镜窗口 —— 照 LibTV 的运镜广场（用户 2026-09-25）。
 *
 * 每个运镜一段循环小视频（`public/camera-previews/`，同一场景同一 seed，只换运镜），**鼠标停在卡片上才播**。
 * 点一个 = 在提示词光标处插一个 `@运镜·X` 标签；**可以叠几个，标签在提示词里的前后就是顺序**，
 * 卡片角上的数字就是它排第几。再点一下拿掉。
 */
export function CameraMoveDialog({ prompt, disabled, onPick, onRemove, onClose }: {
  prompt: string;
  disabled?: boolean;
  /** 插一个运镜标签（由面板插在光标处） */
  onPick: (item: MotionPresetItem) => void;
  /** 从提示词里拿掉这个运镜的标签 */
  onRemove: (item: MotionPresetItem) => void;
  onClose: () => void;
}) {
  const [tab, setTab] = useState("all");
  const [query, setQuery] = useState("");
  const [playAll, setPlayAll] = usePlayAll();
  useEffect(() => {
    const esc = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", esc);
    return () => window.removeEventListener("keydown", esc);
  }, [onClose]);

  const order = useMemo(() => cameraMovesIn(prompt).map((m) => m.id), [prompt]);
  const items = useMemo(() => {
    const q = query.trim().toLowerCase();
    const base = tab === "all" ? CAMERA_MOVES : MOTION_CATEGORIES.find((c) => c.id === tab)?.items ?? [];
    return base.filter((m) => !q || `${m.label} ${m.desc} ${m.zh} ${m.en}`.toLowerCase().includes(q));
  }, [tab, query]);

  return createPortal(
    <div className="nodrag nopan nowheel fixed inset-0 z-[100] flex items-center justify-center bg-black/60 backdrop-blur-sm"
      onPointerDown={(e) => { e.stopPropagation(); if (e.target === e.currentTarget) onClose(); }}
      onWheel={(e) => e.stopPropagation()}>
      <div role="dialog" aria-label="运镜" className="flex h-[78vh] w-[min(1100px,92vw)] flex-col rounded-2xl border border-white/10 bg-[#18181b] shadow-2xl">
        <header className="flex items-center gap-3 border-b border-white/10 px-5 py-3">
          <strong className="text-sm text-white">运镜</strong>
          <span className="text-xs text-white/40">可以选几个，提示词里标签的前后就是顺序{order.length ? ` · 已选 ${order.length} 个` : ""}</span>
          <span className="ml-auto" />
          <PlayAllToggle on={playAll} onChange={setPlayAll} />
          <input aria-label="搜索运镜" placeholder="搜索运镜名称" value={query} onChange={(e) => setQuery(e.target.value)}
            className="h-8 w-60 rounded-lg bg-white/5 px-3 text-xs text-white outline-none placeholder:text-white/30" />
          <button type="button" aria-label="关闭运镜窗口" onClick={onClose} className="rounded-md p-1 text-white/60 hover:bg-white/10 hover:text-white"><X /></button>
        </header>
        <nav className="flex flex-wrap gap-1.5 px-5 pt-3" aria-label="运镜分类">
          {[{ id: "all", name: "全部", n: CAMERA_MOVES.length }, ...MOTION_CATEGORIES.map((c) => ({ id: c.id, name: c.name, n: c.items.length }))].map((t) => (
            <button key={t.id} type="button" onClick={() => setTab(t.id)} aria-pressed={tab === t.id}
              className={`h-7 rounded-md px-3 text-xs ${tab === t.id ? "bg-white/15 text-white" : "text-white/55 hover:bg-white/10 hover:text-white"}`}>
              {t.name}<span className="ml-1 text-white/35">{t.n}</span>
            </button>
          ))}
        </nav>
        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">
          <div className="grid grid-cols-[repeat(auto-fill,minmax(160px,1fr))] gap-4">
            {items.map((m) => {
              const at = order.indexOf(m.id);
              return (
                <button key={m.id} type="button" disabled={disabled} title={`${m.desc}（${m.zh}）`}
                  onClick={() => (at >= 0 ? onRemove(m) : onPick(m))}
                  className={`overflow-hidden rounded-xl border text-left ${at >= 0 ? "border-sky-400/70" : "border-white/10 hover:border-white/30"}`}>
                  <div className="relative aspect-[2/3] bg-black">
                    <MediaThumb key={String(playAll)} path={cameraPreviewUrl(m)} kind="video" ambient hoverPlay={!playAll} className="h-full w-full object-cover" />
                    {at >= 0 && (
                      <span className="absolute right-2 top-2 flex h-5 min-w-5 items-center justify-center gap-0.5 rounded-full bg-sky-500 px-1.5 text-[11px] font-medium text-white">
                        <Check className="h-3 w-3" />{at + 1}
                      </span>
                    )}
                  </div>
                  <div className="px-2.5 py-2">
                    <div className="truncate text-[13px] text-white/90">{cameraShortName(m)}</div>
                    <div className="truncate text-[11px] text-white/40">{m.desc}</div>
                  </div>
                </button>
              );
            })}
          </div>
        </div>
      </div>
    </div>,
    document.body,
  );
}
