import type * as React from "react";
import { useTranslation } from "react-i18next";
import { openPanel } from "../../api/voice";
import { ThemeToggle } from "./ThemeToggle";
import { KeepAwakeBadge } from "./KeepAwakeBadge";

/**
 * 左侧全局导航。**对齐 LumenX 的 GlobalSidebar**：品牌区 → 主导航 → 设置。
 *
 * # 未做的功能也列出来
 *
 * 剧本编辑器**不在这里** —— 剧本属于分集，编辑器就是项目里的第 1 步。
 * LumenX 两处都有，那正是「同一个东西两个入口」造成混乱的老毛病。
 * **没做的不隐藏，列出来并标「未做」** —— 隐藏会让人以为产品只有这么点东西，
 * 列出来才说明白「路线图是什么、现在走到哪」。点上去给一句话说明，不是死链。
 */
interface Item {
  id: string;
  label: string;
  icon: () => React.ReactElement;
  /** 已经做好的走这个面板；未做的为 null */
  panel: string | null;
  note?: string;
}

/* ── 图标。线性 1.5px，和 LumenX 一个调子；不引图标库，四个而已 ── */

const IconWorkspace = () => (
  <svg width="17" height="17" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5">
    <rect x="2.5" y="3.5" width="6" height="6" rx="1.4" />
    <rect x="11.5" y="3.5" width="6" height="6" rx="1.4" />
    <rect x="2.5" y="12.5" width="6" height="4" rx="1.4" />
    <rect x="11.5" y="12.5" width="6" height="4" rx="1.4" />
  </svg>
);

const IconLibrary = () => (
  <svg width="17" height="17" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5">
    <path d="M3 6.2 10 3l7 3.2-7 3.2z" strokeLinejoin="round" />
    <path d="M3 10.2 10 13.4l7-3.2" strokeLinejoin="round" opacity=".65" />
    <path d="M3 13.9 10 17l7-3.1" strokeLinejoin="round" opacity=".4" />
  </svg>
);

const IconScript = () => (
  <svg width="17" height="17" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5">
    <path d="M5 2.8h6.5L15.5 7v10.2H5z" strokeLinejoin="round" />
    <path d="M11.3 2.8V7h4.2" strokeLinejoin="round" opacity=".6" />
    <path d="M7.6 10.5h5M7.6 13.3h3.4" strokeLinecap="round" />
  </svg>
);

const IconCanvas = () => (
  <svg width="17" height="17" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5">
    <rect x="2.5" y="3" width="5" height="5" rx="1" />
    <rect x="12.5" y="12" width="5" height="5" rx="1" />
    <circle cx="15" cy="5.5" r="2.5" />
    <circle cx="5" cy="14.5" r="2.5" />
    <path d="M7.5 5.5h5M5 8v4M15 8v4M7.5 14.5h5" strokeDasharray="1.5 1.5" opacity=".6" />
  </svg>
);

const IconWorkbench = () => (
  <svg width="17" height="17" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5">
    <path d="m10 2.6 1.7 4.2 4.5.3-3.5 2.9 1.1 4.4L10 12l-3.8 2.4 1.1-4.4L3.8 7.1l4.5-.3z" strokeLinejoin="round" />
  </svg>
);


const IconSettings = () => (
  <svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5">
    <circle cx="10" cy="10" r="2.6" />
    <path d="M10 2.5v2M10 15.5v2M17.5 10h-2M4.5 10h-2M15.3 4.7l-1.4 1.4M6.1 13.9l-1.4 1.4M15.3 15.3l-1.4-1.4M6.1 6.1 4.7 4.7" strokeLinecap="round" />
  </svg>
);

export function StudioSidebar({
  active,
  onSelect,
}: {
  active: string;
  onSelect: (id: string) => void;
}) {
  const { t } = useTranslation(["studio", "common"]);

  // **创作台排在最后。** 它是"随手做点东西"的地方，
  // 前面三个是"按项目推进"的流程 —— 顺序反映的是工作流，不是使用频率。
  // **工作区在最上面。** 它是一切的起点 —— 项目、剧本、分镜、成片
  // 全都从那儿开始。创作台排最后：它是「随手做点东西」的地方，
  // 不属于按项目推进的那条线。
  const items: Item[] = [
    {
      id: "workspace",
      label: t("studio:navWorkspace", { defaultValue: "工作区" }),
      icon: IconWorkspace,
      panel: null,
    },
    {
      id: "library",
      label: t("studio:navLibrary", { defaultValue: "资产库" }),
      icon: IconLibrary,
      panel: null,
    },
    {
      // **顶层入口是「另一条路」，不是第二个所有者。**
      // 这一页不存任何东西，只列出所有分集的剧本，点一条跳到那一集的第 1 步。
      // 「我就想写点东西，还没想好属于哪部剧」是个真实场景 ——
      // 创作台也是无项目的自由创作，剧本不该没有对应的入口。
      id: "scripts",
      label: t("studio:navScript", { defaultValue: "剧本编辑器" }),
      icon: IconScript,
      panel: null,
    },
    {
      id: "workbench",
      label: t("studio:navWorkbench", { defaultValue: "创作台" }),
      icon: IconWorkbench,
      panel: null,
    },
    {
      id: "canvas",
      label: t("studio:navCanvas", { defaultValue: "画布" }),
      icon: IconCanvas,
      panel: null,
    },
  ];

  return (
    <nav
      data-tauri-drag-region
      className="studio-chrome studio-titlebar-safe flex w-[200px] shrink-0 flex-col justify-between pb-4"
      style={{ borderRight: "1px solid var(--studio-border)" }}
    >
      <div className="flex flex-col gap-5">
        <div className="px-4">
          <div className="studio-display text-[18px] tracking-tight">ANIMA</div>
          <div className="studio-eyebrow" style={{ letterSpacing: "0.22em" }}>
            STUDIO
          </div>
        </div>

        <div className="flex flex-col gap-0.5 px-2">
          {items.map((it) => {
            const done = !it.note;
            const Icon = it.icon;
            return (
              <button
                key={it.id}
                type="button"
                title={it.note}
                onClick={() => {
                  if (it.panel) void openPanel(it.panel);
                  else onSelect(it.id);
                }}
                className="flex items-center gap-2.5 rounded-lg px-3 py-2.5 text-left text-[14px] transition-colors"
                style={{
                  background:
                    active === it.id ? "var(--studio-bg-elevated)" : "transparent",
                  color: done ? "var(--studio-text)" : "var(--studio-text-muted)",
                }}
              >
                <span className="shrink-0" style={{ opacity: done ? 0.9 : 0.55 }}>
                  <Icon />
                </span>
                <span className="flex-1">{it.label}</span>
                {/* 未做的明确标出来，而不是做成灰色让人猜 */}
                {!done && (
                  <span
                    className="font-mono text-[0.5rem] uppercase tracking-[0.1em]"
                    style={{ color: "var(--studio-text-muted)" }}
                  >
                    {it.panel
                      ? t("studio:navOld", { defaultValue: "作废" })
                      : t("studio:navTodo", { defaultValue: "未做" })}
                  </span>
                )}
              </button>
            );
          })}
        </div>
      </div>

      {/* `relative`：防休眠的弹出层按这一行定位，宽度落在侧边栏里（见 KeepAwakeBadge） */}
      <div className="relative flex items-center gap-1 px-2">
        <button
          type="button"
          onClick={() => void openPanel("settings")}
          className="flex flex-1 items-center gap-2.5 rounded-lg px-3 py-2.5 text-left text-[14px] transition-colors hover:bg-white/5"
          style={{ color: "var(--studio-text-2)" }}
        >
          <span className="shrink-0" style={{ opacity: 0.8 }}>
            <IconSettings />
          </span>
          {t("studio:navSettings", { defaultValue: "设置" })}
        </button>
        {/* 防休眠标记：今晚会不会睡、谁在挡（2026-09-30 用户：界面里没地方查看）。见 KeepAwakeBadge */}
        <KeepAwakeBadge />
        {/* 主题开关贴右边。**翻的是整个应用** —— 见 ThemeToggle 的文件头。 */}
        <ThemeToggle />
      </div>
    </nav>
  );
}
