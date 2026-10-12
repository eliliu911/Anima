/**
 * 系列项目的左栏：**「系列」一组 + 「本集」一组**。
 *
 * # 为什么要分两组（2026-09-25 用户定的）
 *
 * 以前系列项目和单集项目用同一条五步导航，五步全挂在「第 N 集」下面。
 * 可风格、系列资产、生成设置是整部剧一份的东西 —— 风格页自己的副标题就写着
 * 「定整部剧的视觉基调」，却排在单集的步骤里；资产页同时装着系列池和本集名单。
 * 于是「系列通用资产」放在哪都别扭：它本来就不属于某一集。
 *
 * 现在：
 * - **系列**：概览 / 角色与资产 / 风格 / 故事原文（跳剧本编辑器）
 * - **本集**：剧本 / 本集选角 / 分镜 / 组装。集数是组标题上的下拉，
 *   完整的分集列表在概览页（卡片）—— 30 集的长列表不再挤在左栏里。
 *
 * 单集的独立项目不走这里，还是 `StepRail`（它就是只有一集的项目，分两组是多余的）。
 *
 * # 仍然是路标，不是路障
 *
 * 和 `StepRail` 一样没有任何 `disabled`，理由见那个文件头。
 */
import type * as React from "react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Check, ChevronDown } from "../../../canvas/ui/icons";
import { cardTitle } from "../series/overviewStats";
import type { EpisodeData } from "../../../api/cinema";
import { openPanel } from "../../../api/voice";
import { ThemeToggle } from "../ThemeToggle";
import { STEPS, type StepId, type StepState } from "./StepRail";

export type SeriesPage = "overview" | "assets" | "style";

/** 左栏选中的是哪一项。 */
export type RailSel =
  | { level: "series"; page: SeriesPage }
  | { level: "episode"; step: EpisodeStepId };

/** 本集那一组的步骤。**风格不在这里** —— 它是整部剧一份的。 */
export type EpisodeStepId = Exclude<StepId, "style">;

export interface EpisodeStep {
  id: EpisodeStepId;
  num: number;
  label: string;
  en: string;
}

/**
 * 本集那一组有哪几步。风格挪到系列组，资产改叫「本集选角」。
 * 「我直接创作」的项目没有剧本那一步（和 `stepsFor` 同一条规则），序号重排。
 */
export function episodeSteps(contentMode: string | undefined): EpisodeStep[] {
  return STEPS.filter((s) => s.id !== "style")
    .filter((s) => contentMode !== "freeform" || s.id !== "script")
    .map((s, i) => ({
      id: s.id as EpisodeStepId,
      num: i + 1,
      label: s.id === "assets" ? "本集选角" : s.label,
      en: s.id === "assets" ? "CAST" : s.en,
    }));
}

/**
 * 从外面带进来的步骤（Chat 的「在 Studio 打开」、剧本一览）落到哪一项。
 *
 * - 风格 → 系列组的风格
 * - 资产 → 系列的「角色与资产」：从 Chat 跳过来多半是「刚建了资产去看看」，
 *   那些现在都在系列池里
 * - 其它 → 那一集的那一步
 * - 没指定 → 概览（点进系列先看全局，不再直接掉进第 1 集的剧本）
 */
export function initialSel(step: StepId | undefined): RailSel {
  if (!step) return { level: "series", page: "overview" };
  if (step === "style") return { level: "series", page: "style" };
  if (step === "assets") return { level: "series", page: "assets" };
  return { level: "episode", step };
}

const SERIES_ITEMS: { page: SeriesPage | "source"; label: string; icon: string }[] = [
  // 四宫格
  { page: "overview", label: "概览", icon: "M4 4h7v7H4zM13 4h7v7h-7zM4 13h7v7H4zM13 13h7v7h-7z" },
  // 两个人
  {
    page: "assets",
    label: "角色与资产",
    icon: "M9 11a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM3.5 19c0-2.8 2.5-4.5 5.5-4.5s5.5 1.7 5.5 4.5M16 6.2a3 3 0 0 1 0 5.6M17.5 14.8c1.9.5 3 1.9 3 4.2",
  },
  // 调色盘
  {
    page: "style",
    label: "风格",
    icon: "M12 4a8 8 0 1 0 0 16c.9 0 1.5-.7 1.5-1.5 0-.4-.2-.8-.4-1-.3-.3-.4-.6-.4-1 0-.8.7-1.5 1.5-1.5H16a4 4 0 0 0 4-4c0-3.9-3.6-7-8-7zM7.5 12.5h.01M9.5 8.5h.01M14.5 8.5h.01",
  },
  // 摊开的书
  {
    page: "source",
    label: "故事原文",
    icon: "M12 6.5C10.5 5.2 8.4 4.7 5 4.7v12c3.4 0 5.5.5 7 1.8 1.5-1.3 3.6-1.8 7-1.8v-12c-3.4 0-5.5.5-7 1.8zM12 6.5v12",
  },
];

const EPISODE_ICONS: Record<EpisodeStepId, string> = {
  // 一页纸
  script: "M6 3.5h8l4 4v13H6zM14 3.5v4h4M9 12h6M9 15.5h4",
  // 人 + 勾
  assets: "M9 11a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM3.5 19c0-2.8 2.5-4.5 5.5-4.5 1.3 0 2.5.3 3.4.9M15 17l2 2 4-4",
  // 场记板
  storyboard:
    "M3.5 8.5h17v10a1.5 1.5 0 0 1-1.5 1.5H5a1.5 1.5 0 0 1-1.5-1.5v-10zM3.5 8.5 5 4.2l16 1.4-.6 2.9M8.6 4.5 7.4 8.5M13.4 4.9 12.2 8.5",
  // 层叠的帧
  assembly: "M6.5 3.5h11v11h-11zM4 7v12a1.5 1.5 0 0 0 1.5 1.5h11",
};

const selKey = (s: RailSel | { level: "series"; page: "source" }) =>
  s.level === "series" ? `s:${s.page}` : `e:${s.step}`;

export function SeriesRail({
  projectTitle,
  sel,
  onSelect,
  onOpenSource,
  episodes,
  episodeId,
  onEpisode,
  steps,
  states,
  seriesNotes,
  onBack,
}: {
  projectTitle: string;
  sel: RailSel;
  onSelect: (s: RailSel) => void;
  /** 「故事原文」—— 跳到剧本编辑器里这个系列的项目 */
  onOpenSource: () => void;
  episodes: EpisodeData[];
  episodeId: string | null;
  onEpisode: (id: string) => void;
  steps: EpisodeStep[];
  states: Record<StepId, StepState>;
  /** 系列组各项下面那行小字（比如「3 个待归属」） */
  seriesNotes: Partial<Record<SeriesPage, { note?: string; done?: boolean; alert?: boolean }>>;
  onBack: () => void;
}) {
  const listRef = useRef<HTMLDivElement | null>(null);
  const itemRefs = useRef<Record<string, HTMLButtonElement | null>>({});
  const [bar, setBar] = useState({ top: 0, height: 0 });
  const active = selKey(sel);

  /** 量出来，不算 —— 理由见 `StepRail`。两组之间隔着组标题，更不能按序号乘行高。 */
  useLayoutEffect(() => {
    const el = itemRefs.current[active];
    if (!el || !listRef.current) return;
    setBar({ top: el.offsetTop, height: el.offsetHeight });
  }, [active, steps.length, states, seriesNotes, episodeId]);

  const item = (
    key: string,
    icon: string,
    label: string,
    note: string | undefined,
    on: boolean,
    onClick: () => void,
    mark: React.ReactNode,
    title?: string,
  ) => (
    <button
      key={key}
      ref={(el) => {
        itemRefs.current[key] = el;
      }}
      type="button"
      onClick={onClick}
      title={title}
      className="flex items-center gap-2.5 rounded-lg px-3 py-2 text-left"
      style={{ background: on ? "var(--studio-bg-elevated)" : "transparent" }}
    >
      <svg
        aria-hidden="true"
        width="17"
        height="17"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
        strokeLinejoin="round"
        className="shrink-0"
        style={{ color: on ? "var(--studio-primary)" : "var(--studio-text-muted)", transition: "color 160ms" }}
      >
        <path d={icon} />
      </svg>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-[13px]" style={{ color: on ? "var(--studio-text)" : "var(--studio-text-2)" }}>
          {label}
        </span>
        {note && (
          <span className="block truncate text-[10.5px]" style={{ color: "var(--studio-text-muted)" }}>
            {note}
          </span>
        )}
      </span>
      {mark}
    </button>
  );


  return (
    <nav
      data-tauri-drag-region
      className="studio-chrome studio-titlebar-safe flex w-[212px] shrink-0 flex-col overflow-y-auto pb-4"
      style={{ borderRight: "1px solid var(--studio-border)" }}
    >
      <button
        type="button"
        onClick={onBack}
        className="mx-2 mb-2.5 flex w-fit items-center gap-1 rounded-lg px-2 py-1 text-left text-[12px] transition-colors hover:bg-white/5"
        style={{ color: "var(--studio-text-2)" }}
      >
        <span style={{ opacity: 0.6 }}>‹</span>
        <span className="truncate">工作区</span>
      </button>

      <div className="mb-3 px-4">
        <p className="studio-display truncate text-[15px]">{projectTitle}</p>
        <p className="studio-eyebrow mt-0.5 truncate">系列 · {episodes.length} 集</p>
      </div>

      <div ref={listRef} className="relative flex flex-col gap-0.5 px-2">
        <span
          aria-hidden="true"
          className="pointer-events-none absolute left-2 w-[2px] rounded-full"
          style={{
            background: "var(--studio-primary)",
            top: bar.top,
            height: bar.height,
            opacity: bar.height > 0 ? 1 : 0,
            transition: "top 220ms cubic-bezier(0.22, 1, 0.36, 1), height 220ms",
          }}
        />

        <span className="studio-eyebrow px-3 pb-1 pt-1">系列</span>
        {SERIES_ITEMS.map((it) => {
          if (it.page === "source") {
            return item("s:source", it.icon, `${it.label} ↗`, "在剧本编辑器里打开", false, onOpenSource, null,
              "大纲和各章原文在剧本编辑器里，点这里跳过去");
          }
          const page = it.page;
          const n = seriesNotes[page];
          const on = sel.level === "series" && sel.page === page;
          const mark = n?.alert ? (
            <span className="h-1.5 w-1.5 shrink-0 rounded-full" style={{ background: "var(--studio-pending-fg)" }} />
          ) : n?.done ? (
            <span className="shrink-0 text-[12px]" style={{ color: "var(--studio-primary)" }}>✓</span>
          ) : null;
          return item(`s:${page}`, it.icon, it.label, n?.note, on, () => onSelect({ level: "series", page }), mark);
        })}

        <div className="mt-4 flex items-center gap-1 px-3 pb-1">
          <span className="studio-eyebrow shrink-0">本集</span>
          {/* **集数是这一组的标题，不是一串列表。** 30 集的列表挤在左栏里，
              步骤被推到屏幕下半截；完整列表在概览页。 */}
          <EpisodePicker episodes={episodes} episodeId={episodeId} onPick={onEpisode} />
        </div>
        {steps.map((s) => {
          const st = states[s.id];
          const on = sel.level === "episode" && sel.step === s.id;
          const mark = st.done ? (
            <span className="shrink-0 text-[12px]" style={{ color: "var(--studio-primary)" }}>✓</span>
          ) : st.softLock ? (
            <span className="shrink-0 text-[10px]" style={{ color: "var(--studio-text-muted)" }}>🔒</span>
          ) : (
            <span className="shrink-0 text-[10px]" style={{ color: "var(--studio-text-muted)", opacity: 0.6 }}>○</span>
          );
          return item(
            `e:${s.id}`,
            EPISODE_ICONS[s.id],
            `${s.num}. ${s.label}`,
            st.note,
            on,
            () => onSelect({ level: "episode", step: s.id }),
            mark,
            st.softLock,
          );
        })}
      </div>

      <div className="mt-auto flex items-center gap-1 px-2 pt-5">
        <button
          type="button"
          onClick={() => void openPanel("settings")}
          className="flex flex-1 items-center gap-2 rounded-lg px-3 py-2 text-left text-[13px] transition-colors hover:bg-white/5"
          style={{ color: "var(--studio-text-2)" }}
        >
          设置
        </button>
        <ThemeToggle />
      </div>
    </nav>
  );
}

/**
 * 换集的下拉。**不用系统的 `<select>`**（用户 2026-09-26：「不好看」，要画布引擎选择器那种）：
 * 按钮是「集号 + 截断的标题 + 统一的向下箭头」，浮层每项是集号方块、标题、进度小标签，选中的打勾。
 *
 * 浮层**挂到外层、按按钮位置定位**：左栏是 `overflow-y-auto`，
 * 绝对定位的浮层会被它裁掉（横向也裁 —— overflow 一个方向设了 auto，另一个方向就不再是 visible）。
 * 高度限到 18rem，一屏大约八集，打开时滚到当前那一集。
 */
function EpisodePicker({
  episodes,
  episodeId,
  onPick,
}: {
  episodes: EpisodeData[];
  episodeId: string | null;
  onPick: (id: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState({ top: 0, left: 0 });
  const btn = useRef<HTMLButtonElement | null>(null);
  const pop = useRef<HTMLDivElement | null>(null);
  const current = episodes.find((e) => e.id === episodeId);
  const num = (e: EpisodeData) => String(e.episode_num).padStart(2, "0");

  useLayoutEffect(() => {
    if (!open || !btn.current) return;
    const r = btn.current.getBoundingClientRect();
    setPos({ top: r.bottom + 6, left: Math.max(8, r.left - 8) });
  }, [open]);
  useEffect(() => {
    if (!open) return;
    // **只滚浮层自己。** `scrollIntoView` 会连带滚动所有祖先，整页被往上推
    const box = pop.current;
    const sel = box?.querySelector<HTMLElement>("[aria-selected='true']");
    if (box && sel) box.scrollTop = sel.offsetTop - box.clientHeight / 2 + sel.offsetHeight / 2;
    const close = (e: PointerEvent) => {
      const t = e.target as Node;
      if (!pop.current?.contains(t) && !btn.current?.contains(t)) setOpen(false);
    };
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    window.addEventListener("pointerdown", close);
    window.addEventListener("keydown", esc);
    return () => {
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("keydown", esc);
    };
  }, [open]);

  return (
    <>
      <button
        ref={btn}
        type="button"
        aria-label="切换到哪一集"
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        title={current ? `第 ${current.episode_num} 集 · ${current.title}` : undefined}
        className="ml-auto flex min-w-0 items-center gap-1 rounded-md px-1.5 py-0.5 text-[11.5px] transition-colors hover:bg-white/5"
        style={{ color: "var(--studio-text-2)" }}
      >
        {current && (
          <>
            <span className="font-mono text-[10.5px]" style={{ color: "var(--studio-text-muted)" }}>
              E{num(current)}
            </span>
            <span className="truncate">{cardTitle(current)}</span>
          </>
        )}
        <ChevronDown className="h-3 w-3 shrink-0 opacity-50" />
      </button>
      {open &&
        createPortal(
          <div
            ref={pop}
            role="listbox"
            aria-label="切换到哪一集"
            className="fixed z-[60] flex max-h-72 w-64 flex-col gap-0.5 overflow-y-auto rounded-2xl p-1.5 shadow-2xl backdrop-blur"
            style={{
              top: pos.top,
              left: pos.left,
              background: "color-mix(in srgb, var(--studio-bg-elevated) 96%, transparent)",
              border: "1px solid var(--studio-border)",
            }}
          >
            {episodes.map((e) => {
              const on = e.id === episodeId;
              return (
                <button
                  key={e.id}
                  type="button"
                  role="option"
                  aria-selected={on}
                  title={e.synopsis || e.title}
                  onClick={() => {
                    onPick(e.id);
                    setOpen(false);
                  }}
                  className="flex items-center gap-2.5 rounded-xl px-2 py-1.5 text-left transition-colors hover:bg-white/10"
                  style={{ background: on ? "var(--studio-bg-inset)" : undefined }}
                >
                  <span
                    className="flex h-6 w-6 shrink-0 items-center justify-center rounded-lg font-mono text-[10px] font-semibold"
                    style={{ background: "var(--studio-bg-inset)", color: "var(--studio-text-2)" }}
                  >
                    {num(e)}
                  </span>
                  <span className="min-w-0 flex-1 truncate text-[12px]" style={{ color: "var(--studio-text)" }}>
                    {cardTitle(e)}
                  </span>
                  {/* 进度只在出了分镜之后标（「12 镜」「已合成」）—— 只有剧本的集每行都写字数，是噪声 */}
                  {!on && (e.storyboard?.length || e.final_video_url) ? (
                    <span
                      className="shrink-0 rounded-md px-1.5 py-0.5 text-[10px]"
                      style={{ background: "var(--studio-bg-inset)", color: "var(--studio-text-muted)" }}
                    >
                      {e.final_video_url ? "已合成" : `${e.storyboard.length} 镜`}
                    </span>
                  ) : null}
                  {on && <Check className="h-3 w-3 shrink-0" style={{ color: "var(--studio-text-2)" }} />}
                </button>
              );
            })}
          </div>,
          // 挂到最近的 `.anima-studio` 里，不是 body：主题色变量只定义在它上面；
          // 也不能给浮层自己加这个类 —— 它带 `position: relative`，会把 `fixed` 盖掉
          btn.current?.closest(".anima-studio") ?? document.body,
        )}
    </>
  );
}
