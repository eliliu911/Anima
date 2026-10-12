import { useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { openUrl } from "@tauri-apps/plugin-opener";
import { studioLoras, type LoraOption } from "../../api/studio";
import { styleCatalog, type StylePreset } from "../../api/style";
import { LORA_DEFAULT, LORA_MAX, LORA_STEP, formatStrength } from "../../components/studio/LoraPanel";
import { MediaThumb } from "../../components/studio/MediaThumb";
import { Check, Layers, X } from "./icons";
import { PlayAllToggle, usePlayAll } from "./PlayAllToggle";

/**
 * 风格窗口 —— 照 LibTV 的风格广场（用户 2026-09-25）。
 *
 * - 只列**当前节点引擎**能挂的 LoRA（按张量键名认的架构，见 Rust `detect_arch`）
 * - 顶上按分类分（`_anima_notes.json` 的 `category`），另有一类「提示词风格」：不挂 LoRA，往提示词里加一段画风描述
 * - 每张卡是作者样例图（按文件哈希在 Civitai 反查、下到 `loras/_anima_thumbs/`）；没有的显示占位
 * - **成人向单独一类、默认收起**：勾「显示成人向」才出现，且不混进别的分类
 */
// 「角色卡」：出多视图设定卡 / 参考表的（用户 2026-09-25：「角色信息卡类别的，不要放在成人向里，增加一个角色卡」）
export const LORA_CATEGORIES = ["画风", "人像", "角色", "角色卡", "服装", "场景", "功能", "动作", "人体"] as const;
export const PROMPT_TAB = "提示词风格";
export const ADULT_TAB = "成人向";
export const ALL_TAB = "全部";
const OTHER = "其它";

const categoryOf = (l: LoraOption) => (l.category && (LORA_CATEGORIES as readonly string[]).includes(l.category) ? l.category : OTHER);

/** 这个引擎能挂、并且落在这个分类里的 LoRA。成人向只在「成人向」里出现 */
export function galleryLoras(all: LoraOption[], engine: string, tab: string, query = ""): LoraOption[] {
  const q = query.trim().toLowerCase();
  return all
    .filter((l) => l.engines.includes(engine))
    .filter((l) => (tab === ADULT_TAB ? l.nsfw : !l.nsfw))
    .filter((l) => tab === ALL_TAB || tab === ADULT_TAB || categoryOf(l) === tab)
    .filter((l) => !q || `${l.label} ${l.note ?? ""} ${l.confirmed_triggers.join(" ")}`.toLowerCase().includes(q));
}

/** 顶上的分类标签：只列有东西的；「提示词风格」一直在；「成人向」勾了才有 */
export function galleryTabs(all: LoraOption[], engine: string, showAdult: boolean, lorasSupported: boolean): { id: string; count: number }[] {
  if (!lorasSupported) return [{ id: PROMPT_TAB, count: -1 }];
  const usable = all.filter((l) => l.engines.includes(engine));
  const safe = usable.filter((l) => !l.nsfw);
  const tabs = [{ id: ALL_TAB, count: safe.length }];
  for (const c of [...LORA_CATEGORIES, OTHER]) {
    const n = safe.filter((l) => categoryOf(l) === c).length;
    if (n) tabs.push({ id: c, count: n });
  }
  tabs.push({ id: PROMPT_TAB, count: -1 });
  const adult = usable.filter((l) => l.nsfw).length;
  if (showAdult && adult) tabs.push({ id: ADULT_TAB, count: adult });
  return tabs;
}

/** 提示词风格：加一段 / 再点拿掉。按整段文字判断在不在，不动用户写的其它内容 */
export function togglePromptStyle(prompt: string, style: string): string {
  const s = style.trim();
  if (!s) return prompt;
  if (prompt.includes(s)) return prompt.replace(s, "").replace(/，\s*，/g, "，").replace(/^[，,\s]+|[，,\s]+$/g, "");
  return prompt.trim() ? `${prompt.trim()}，${s}` : s;
}

export function StyleGalleryDialog({ engine, engineLabel, lorasSupported, picks, prompt, disabled, onSetLora, onPromptChange, onClose }: {
  engine: string;
  engineLabel?: string;
  /** 这个引擎能不能挂 LoRA；不能就只剩提示词风格 */
  lorasSupported: boolean;
  picks: { name: string; strength: number }[];
  prompt: string;
  disabled?: boolean;
  /** strength = null：取消挂载 */
  onSetLora: (name: string, strength: number | null) => void;
  onPromptChange: (next: string) => void;
  onClose: () => void;
}) {
  const [loras, setLoras] = useState<LoraOption[] | null>(null);
  const [styles, setStyles] = useState<StylePreset[]>([]);
  const [error, setError] = useState("");
  const [showAdult, setShowAdult] = useState(false);
  const [tab, setTab] = useState<string>(lorasSupported ? ALL_TAB : PROMPT_TAB);
  const [query, setQuery] = useState("");
  const [playAll, setPlayAll] = usePlayAll();

  useEffect(() => {
    let live = true;
    studioLoras().then((l) => live && setLoras(l)).catch((e) => live && setError(String(e)));
    styleCatalog().then((c) => live && setStyles(c.presets)).catch(() => {});
    return () => { live = false; };
  }, []);
  useEffect(() => {
    const esc = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", esc);
    return () => window.removeEventListener("keydown", esc);
  }, [onClose]);
  // 收起成人向时，如果正停在那一类，回到全部
  useEffect(() => { if (!showAdult && tab === ADULT_TAB) setTab(ALL_TAB); }, [showAdult, tab]);

  const tabs = useMemo(() => galleryTabs(loras ?? [], engine, showAdult, lorasSupported), [loras, engine, showAdult, lorasSupported]);
  const items = useMemo(() => galleryLoras(loras ?? [], engine, tab, query), [loras, engine, tab, query]);
  const styleItems = useMemo(() => {
    const q = query.trim();
    return styles.filter((s) => !q || `${s.name_zh} ${s.subtitle_zh} ${s.description}`.includes(q));
  }, [styles, query]);
  const picked = (name: string) => picks.find((p) => p.name === name);

  return createPortal(
    <div className="nodrag nopan nowheel fixed inset-0 z-[100] flex items-center justify-center bg-black/60 backdrop-blur-sm"
      onPointerDown={(e) => { e.stopPropagation(); if (e.target === e.currentTarget) onClose(); }}
      onWheel={(e) => e.stopPropagation()}>
      <div role="dialog" aria-label="风格" className="flex h-[82vh] w-[min(1400px,92vw)] flex-col rounded-2xl border border-white/10 bg-[#18181b] shadow-2xl">
        <header className="flex items-center gap-3 border-b border-white/10 px-5 py-3">
          <Layers width={16} height={16} />
          <strong className="text-sm text-white">风格</strong>
          <span className="text-xs text-white/40">{engineLabel ? `只列「${engineLabel}」能用的` : ""}{picks.length ? ` · 已挂 ${picks.length} 个` : ""}</span>
          <input aria-label="搜索风格" placeholder="搜索名字、备注、激活词" value={query} onChange={(e) => setQuery(e.target.value)}
            className="ml-4 h-8 w-72 rounded-lg bg-white/5 px-3 text-xs text-white outline-none placeholder:text-white/30" />
          <span className="ml-auto" />
          {(loras ?? []).some((l) => l.thumb?.endsWith(".mp4") && l.engines.includes(engine)) && <PlayAllToggle on={playAll} onChange={setPlayAll} />}
          <label className="flex items-center gap-1.5 text-xs text-white/50">
            <input type="checkbox" checked={showAdult} onChange={(e) => setShowAdult(e.target.checked)} />显示成人向
          </label>
          <button type="button" aria-label="关闭风格窗口" onClick={onClose} className="rounded-md p-1 text-white/60 hover:bg-white/10 hover:text-white"><X /></button>
        </header>

        <nav className="flex flex-wrap gap-1.5 px-5 pt-3" aria-label="风格分类">
          {tabs.map((t) => (
            <button key={t.id} type="button" onClick={() => setTab(t.id)} aria-pressed={tab === t.id}
              className={`h-7 rounded-md px-3 text-xs ${tab === t.id ? "bg-white/15 text-white" : "text-white/55 hover:bg-white/10 hover:text-white"}`}>
              {t.id}{t.count >= 0 && <span className="ml-1 text-white/35">{t.count}</span>}
            </button>
          ))}
        </nav>

        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">
          {error && <p role="alert" className="text-xs text-red-300">读取 LoRA 失败：{error}</p>}
          {tab === PROMPT_TAB ? (
            <div className="grid grid-cols-[repeat(auto-fill,minmax(180px,1fr))] gap-4">
              {styleItems.map((s) => {
                const on = prompt.includes(s.positive_prompt.trim());
                return (
                  <button key={s.id} type="button" disabled={disabled} title={s.description || s.positive_prompt}
                    onClick={() => onPromptChange(togglePromptStyle(prompt, s.positive_prompt))}
                    className={`group overflow-hidden rounded-xl border text-left ${on ? "border-sky-400/70" : "border-white/10 hover:border-white/30"}`}>
                    <div className="relative aspect-[3/4] bg-white/5">
                      {s.thumb_url ? <MediaThumb path={s.thumb_url} className="h-full w-full object-cover" /> : <span className="flex h-full items-center justify-center text-xs text-white/30">还没有示例</span>}
                      {on && <span className="absolute right-2 top-2 rounded-full bg-sky-500 p-1 text-white"><Check className="h-3 w-3" /></span>}
                    </div>
                    <div className="px-2.5 py-2">
                      <div className="truncate text-[13px] text-white/90">{s.name_zh}</div>
                      <div className="truncate text-[11px] text-white/40">{s.subtitle_zh || "提示词风格 · 不挂 LoRA"}</div>
                    </div>
                  </button>
                );
              })}
            </div>
          ) : loras === null ? (
            <p className="text-xs text-white/40">读取 LoRA 中…</p>
          ) : items.length === 0 ? (
            <p className="text-xs text-white/40">这个分类下没有「{engineLabel || engine}」能用的 LoRA</p>
          ) : (
            <div className="grid grid-cols-[repeat(auto-fill,minmax(180px,1fr))] gap-4">
              {items.map((l) => {
                const p = picked(l.name);
                const trig = l.confirmed_triggers.length ? l.confirmed_triggers : l.triggers;
                return (
                  <div key={l.name} className={`overflow-hidden rounded-xl border ${p ? "border-sky-400/70" : "border-white/10 hover:border-white/30"}`}>
                    <button type="button" disabled={disabled} title={l.note || l.label}
                      onClick={() => onSetLora(l.name, p ? null : LORA_DEFAULT)}
                      className="relative block aspect-[3/4] w-full bg-white/5">
                      {l.thumb
                        // 视频类 LoRA 的示例是一段 mp4（作者样例），和运镜一样鼠标停上去才播
                        ? l.thumb.endsWith(".mp4")
                          ? <MediaThumb key={String(playAll)} path={l.thumb} kind="video" ambient hoverPlay={!playAll} className="h-full w-full object-cover" />
                          : <MediaThumb path={l.thumb} className="h-full w-full object-cover" />
                        : <span className="flex h-full flex-col items-center justify-center gap-1 px-3 text-center text-xs text-white/35"><Layers width={18} height={18} />{l.label}<span className="text-[10px]">还没有示例图</span></span>}
                      {p && <span className="absolute right-2 top-2 rounded-full bg-sky-500 p-1 text-white"><Check className="h-3 w-3" /></span>}
                      <span className="absolute left-2 top-2 rounded bg-black/60 px-1.5 py-0.5 text-[10px] text-white/80">{categoryOf(l)}</span>
                    </button>
                    <div className="flex flex-col gap-1 px-2.5 py-2">
                      <div className="flex items-center gap-1.5">
                        <span className="min-w-0 flex-1 truncate text-[13px] text-white/90" title={l.label}>{l.label}</span>
                        {l.source_url && (
                          <button type="button" title={`打开出处：${l.source_url}`} onClick={() => void openUrl(l.source_url)}
                            className="shrink-0 text-[10px] text-white/40 underline hover:text-white">出处</button>
                        )}
                      </div>
                      {trig.length > 0 && <div className="truncate text-[10px] text-white/40" title={trig.join(", ")}>激活词：{trig.join(", ")}</div>}
                      {p && (
                        <label className="flex items-center gap-2 text-[10px] text-white/60">
                          强度
                          <input type="range" min={0} max={LORA_MAX} step={LORA_STEP} value={p.strength} disabled={disabled}
                            onChange={(e) => onSetLora(l.name, Number(e.target.value))} className="min-w-0 flex-1" />
                          <span className="tabular-nums">{formatStrength(p.strength)}</span>
                        </label>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </div>
      </div>
    </div>,
    document.body,
  );
}
