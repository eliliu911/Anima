/**
 * 侧边栏底部的防休眠标记：这台机器现在会不会自己睡过去、谁在挡着。
 *
 * 用户 2026-09-30：「我不知道 Anima 的防休眠进程生没生效，因为界面里没地方查看」。
 * 那一夜 Anima 的防休眠跟着一个被误判失败的任务撤了，而用户只能猜。读的是系统真实的断言（`pmset -g assertions`），
 * Anima 自己的、终端里开的 `caffeinate`、别的应用按着的都列出来 —— 回答的是「今晚会不会睡」，不是「Anima 记着按了没有」。
 */
import { useCallback, useEffect, useState } from "react";
import { keepAwakeStatus, type KeepAwakeStatus } from "../../api/keepAwake";
import { useDismissable } from "./useDismissable";
import { animaLine, awake, blockerLine } from "./keepAwakeView";

/** 多久刷新一次标记（打开详情时立刻刷新一次） */
const POLL_MS = 60_000;

export function KeepAwakeBadge() {
  const [status, setStatus] = useState<KeepAwakeStatus | null>(null);
  const [open, setOpen] = useState(false);
  const refresh = useCallback(() => {
    keepAwakeStatus().then(setStatus).catch(() => setStatus(null));
  }, []);
  useEffect(() => {
    refresh();
    const t = setInterval(refresh, POLL_MS);
    return () => clearInterval(t);
  }, [refresh]);
  const ref = useDismissable<HTMLDivElement>(open, () => setOpen(false));

  const on = status ? awake(status) : false;
  const title = !status ? "防休眠：读不到状态" : on ? "防休眠：这台机器现在不会自己睡（点开看是谁在挡）" : "防休眠：没有东西在挡，机器空闲时会睡";
  const now = Math.floor(Date.now() / 1000);

  return (
    // **不设 relative**：弹出层按侧边栏底部那一行定位（StudioSidebar 里那一行是 relative），
    // 左右贴着侧边栏、往上弹到侧边栏的空白处。原来按这个按钮定位、宽 300px，伸进右边工作区被项目卡片盖住（用户截图）
    <div ref={ref}>
      <button
        type="button"
        aria-label={title}
        title={title}
        aria-pressed={open}
        onClick={() => {
          if (!open) refresh();
          setOpen((v) => !v);
        }}
        className="flex shrink-0 items-center justify-center rounded-lg p-2 transition-colors hover:bg-white/5"
        style={{ color: on ? "var(--studio-primary)" : "var(--studio-text-muted)" }}
      >
        {/* 一杯咖啡：caffeinate 的意思 */}
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
          <path d="M4 9h12v5a5 5 0 0 1-5 5H9a5 5 0 0 1-5-5z" />
          <path d="M16 11h1.5a2.5 2.5 0 0 1 0 5H16" />
          <path d="M8 3.5c0 1 1 1.5 1 2.5M12 3.5c0 1 1 1.5 1 2.5" opacity={on ? 1 : 0.5} />
        </svg>
      </button>
      {open && (
        <div role="dialog" aria-label="防休眠" className="studio-menu absolute bottom-full left-2 right-2 z-50 mb-2 flex flex-col gap-2 p-3 text-[12px]">
          {!status ? (
            <p style={{ color: "var(--studio-text-muted)" }}>读不到防休眠状态（`pmset` 没有返回）。</p>
          ) : (
            <>
              <p className="font-medium" style={{ color: on ? "var(--studio-primary)" : "var(--studio-danger)" }}>
                {on ? "这台机器现在不会自己睡" : "没有东西在挡 —— 机器空闲时会睡过去"}
              </p>
              <div>
                <p className="studio-eyebrow mb-0.5">Anima</p>
                <p className="leading-snug" style={{ color: "var(--studio-text-2)" }}>{animaLine(status, now)}</p>
              </div>
              <div>
                <p className="studio-eyebrow mb-0.5">系统里在挡睡眠的</p>
                {status.blockers.length === 0 ? (
                  <p style={{ color: "var(--studio-text-muted)" }}>没有</p>
                ) : (
                  <ul className="flex flex-col gap-0.5">
                    {status.blockers.map((b) => (
                      <li key={b.pid} className="leading-snug" style={{ color: "var(--studio-text-2)" }} title={`pid ${b.pid} · ${b.name}`}>
                        · {blockerLine(b)}
                      </li>
                    ))}
                  </ul>
                )}
              </div>
              <p className="text-[11px] leading-snug" style={{ color: "var(--studio-text-muted)" }}>
                只管会不会睡，不管屏幕 —— 屏幕照常熄灭，不影响出片。
              </p>
            </>
          )}
        </div>
      )}
    </div>
  );
}
