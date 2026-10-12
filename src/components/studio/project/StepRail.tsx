import { useLayoutEffect, useRef, useState } from "react";
/**
 * 项目内的六步导航。
 *
 * # 路标，不是路障 —— 这是整套改版最重要的一条
 *
 * 旧短剧面板的六个格子是路障：角色资产没做完，点分镜/视频/音频/导出
 * 会被**弹回第 2 步**（`CinemaStudioPanel.tsx` 的 `handleSelectStage`）。
 * 用户的原话是「流程太乱」，根子就在这里 —— 真实的活不是线性的：
 * 第 7 镜出得不像要回去改角色，风格不对要回创作台重抽。
 *
 * LumenX 的六个格子是路标。它自己的注释写着 Assembly
 * 「soft-gated (lock + label) but stays **CLICKABLE**」——
 * 界面上 Step 4 打着 ✓ 而 Step 2 还是 ○，照样能点。
 *
 * **所以这里没有任何 `disabled`。** 缺什么由那一步自己说，
 * 不由导航拦着不让进。有测试钉住这条。
 */
import type * as React from "react";
import { openPanel } from "../../../api/voice";
import { ThemeToggle } from "../ThemeToggle";

/**
 * **「动效」那一步于 2026-09-08 删除。**
 *
 * 它当初是按「分镜排版 / 批量出片」两件事分开设计的，但后来逐镜出首帧、
 * 出视频、改提示词全都做进了分镜页 —— 动效那页只剩一个批量按钮，
 * 出完 5/5 之后整页无事可做（副标题还写着「每镜多取选一条」，
 * 那个功能从没实现）。批量已并入分镜页的工具条。
 */
export type StepId = "script" | "style" | "assets" | "storyboard" | "assembly";

export interface StepState {
  /** 已经做过了（有产出） */
  done: boolean;
  /** 一句话状态，显示在标题下面。空 = 不显示 */
  note?: string;
  /** 软锁：显示一把锁和原因，但**照样能点** */
  softLock?: string;
}

export interface Step {
  id: StepId;
  num: number;
  label: string;
  en: string;
}

/**
 * 按内容模式给出这个项目真正的步骤序列。
 *
 * **「我直接创作」时剧本那一步不存在** —— 不是灰掉、不是软锁，是不出现。
 * 直接写分镜提示词的人根本不需要剧本编辑器，让它杵在第一位只会让人
 * 以为"必须先写点什么"。（抄自 LumenX 的 `content_mode: freeform`。）
 *
 * **序号必须重算。** 只过滤不重算的话，第一项会显示成「2. 风格」——
 * 而右边还写着 STEP 02，看起来像少了一步、或者哪里出错了。
 */
export function stepsFor(contentMode: string | undefined): Step[] {
  const kept = contentMode === "freeform" ? STEPS.filter((s) => s.id !== "script") : STEPS;
  return kept.map((s, i) => ({ ...s, num: i + 1 }));
}

/**
 * 每一步的图标。
 *
 * **图标是给扫视用的，不是装饰。** 六步的中文名都是两个字，
 * 光靠文字要逐个读；一个图形轮廓比两个汉字先到眼睛。
 * 24×24 视口，`currentColor` 描边 —— 跟着文字颜色走，深浅主题都不用管。
 */
const ICONS: Record<StepId, string> = {
  // 摊开的书
  script: "M12 6.5C10.5 5.2 8.4 4.7 5 4.7v12c3.4 0 5.5.5 7 1.8 1.5-1.3 3.6-1.8 7-1.8v-12c-3.4 0-5.5.5-7 1.8zM12 6.5v12",
  // 调色盘
  style: "M12 4a8 8 0 1 0 0 16c.9 0 1.5-.7 1.5-1.5 0-.4-.2-.8-.4-1-.3-.3-.4-.6-.4-1 0-.8.7-1.5 1.5-1.5H16a4 4 0 0 0 4-4c0-3.9-3.6-7-8-7zM7.5 12.5h.01M9.5 8.5h.01M14.5 8.5h.01",
  // 两个人
  assets: "M9 11a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM3.5 19c0-2.8 2.5-4.5 5.5-4.5s5.5 1.7 5.5 4.5M16 6.2a3 3 0 0 1 0 5.6M17.5 14.8c1.9.5 3 1.9 3 4.2",
  // 场记板
  storyboard: "M3.5 8.5h17v10a1.5 1.5 0 0 1-1.5 1.5H5a1.5 1.5 0 0 1-1.5-1.5v-10zM3.5 8.5 5 4.2l16 1.4-.6 2.9M8.6 4.5 7.4 8.5M13.4 4.9 12.2 8.5",
  // 胶片 + 播放
  // 层叠的帧
  assembly: "M6.5 3.5h11v11h-11zM4 7v12a1.5 1.5 0 0 0 1.5 1.5h11",
};

export const STEPS: { id: StepId; num: number; label: string; en: string }[] = [
  { id: "script", num: 1, label: "剧本", en: "SCRIPT" },
  { id: "style", num: 2, label: "风格", en: "STYLE" },
  { id: "assets", num: 3, label: "资产", en: "ASSETS" },
  { id: "storyboard", num: 4, label: "分镜", en: "STORYBOARD" },
  { id: "assembly", num: 5, label: "组装", en: "ASSEMBLY" },
];

export function StepRail({
  active,
  steps,
  states,
  projectTitle,
  episodeLabel,
  onSelect,
  onBack,
  children,
}: {
  active: StepId;
  /** 这个项目真正有哪几步。由 `stepsFor(project.content_mode)` 给出。 */
  steps: Step[];
  states: Record<StepId, StepState>;
  projectTitle: string;
  episodeLabel: string | null;
  onSelect: (id: StepId) => void;
  onBack: () => void;
  /** 分集列表之类的额外内容，插在步骤上方 */
  children?: React.ReactNode;
}) {
  const listRef = useRef<HTMLDivElement | null>(null);
  const itemRefs = useRef<Partial<Record<StepId, HTMLButtonElement | null>>>({});
  const [bar, setBar] = useState({ top: 0, height: 0 });

  /**
   * 量选中项的位置，喂给滑动条。
   *
   * **量，不算。** 选中项比其它项高（多一行状态字），
   * 按「序号 × 行高」算必然对不齐 —— 而且是越往下偏得越多。
   *
   * 依赖里带上 `steps.length` 和 `states`：freeform 项目少一步、
   * 状态字变长变短都会改变行高，不重量的话条子会停在错的地方。
   */
  useLayoutEffect(() => {
    const el = itemRefs.current[active];
    const list = listRef.current;
    if (!el || !list) return;
    setBar({ top: el.offsetTop, height: el.offsetHeight });
  }, [active, steps.length, states]);

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

      <div className="mb-4 px-4">
        <p className="studio-display truncate text-[15px]">{projectTitle}</p>
        {episodeLabel && (
          <p className="studio-eyebrow mt-0.5 truncate">{episodeLabel}</p>
        )}
      </div>

      {children}

      <div ref={listRef} className="relative flex flex-col gap-0.5 px-2">
        {/* **滑动的选中条。**
            以前是给每个按钮加 `border-left`，切换时是"这里灭那里亮"。
            改成一条独立的元素在 Y 轴上滑 —— 眼睛能跟住它从哪儿到哪儿，
            这在一个六步流程里是有意义的信息，不只是好看。

            位置**量出来**而不是按序号乘行高：选中项比其它项高
            （多一行状态字），按固定行高算必然对不齐。 */}
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
        {steps.map((s) => {
          const st = states[s.id];
          const on = active === s.id;
          return (
            <button
              key={s.id}
              ref={(el) => {
                itemRefs.current[s.id] = el;
              }}
              type="button"
              /* 没有 disabled。见文件头。 */
              onClick={() => onSelect(s.id)}
              title={st.softLock}
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
                style={{
                  color: on ? "var(--studio-primary)" : "var(--studio-text-muted)",
                  transition: "color 160ms",
                }}
              >
                <path d={ICONS[s.id]} />
              </svg>
              <span className="min-w-0 flex-1">
                <span
                  className="block truncate text-[13px]"
                  style={{ color: on ? "var(--studio-text)" : "var(--studio-text-2)" }}
                >
                  {s.num}. {s.label}
                </span>
                <span
                  className="block truncate font-mono text-[0.5rem] uppercase tracking-[0.12em]"
                  style={{ color: "var(--studio-text-muted)" }}
                >
                  {st.note || `STEP 0${s.num}`}
                </span>
              </span>
              <StepMark done={st.done} softLock={!!st.softLock} />
            </button>
          );
        })}
      </div>

      {/* 项目页里左栏换成了六步导航，全局侧栏那一行够不着了 ——
          底部补同样一行，位置和外面对齐。 */}
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

/** ✓ 做过了 / 🔒 软锁（照样能点）/ ○ 还没做 */
function StepMark({ done, softLock }: { done: boolean; softLock: boolean }) {
  if (done) {
    return (
      <span className="shrink-0 text-[12px]" style={{ color: "var(--studio-primary)" }}>
        ✓
      </span>
    );
  }
  if (softLock) {
    return (
      <span className="shrink-0 text-[10px]" style={{ color: "var(--studio-text-muted)" }}>
        🔒
      </span>
    );
  }
  return (
    <span
      className="shrink-0 text-[10px]"
      style={{ color: "var(--studio-text-muted)", opacity: 0.6 }}
    >
      ○
    </span>
  );
}
