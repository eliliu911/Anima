/**
 * 分镜页页头：**一排四个步骤，每一步的状态和按钮放在同一张卡片里**。
 *
 * 用户 2026-09-29：「顶上这两排按钮和进度，越来越 confuse……现在感觉看着很不合理」。原来是两排都编了 1 / 2 / 3 ——
 * 一排是按钮（生成镜头 / 合成 / 出片），一排是状态（确认镜头 / 准备资产 / 合成），编号意思还不一样；
 * 细化、按场出片、按场出首帧散在按钮排里，看不出归哪一步；主按钮的字还会变成进度；下面再叠最多五条彩色横幅。
 *
 * 现在：
 * - 四步 **镜头 → 提示词 → 首帧（可选）→ 出片**，顺序就是真实的依赖顺序
 * - 每张卡片：标题、状态、（要注意的一行）、（在跑的进度）、这一步自己的按钮
 * - **只有「下一步」那张高亮**，它的主按钮是主色，其它都是普通按钮
 * - 横幅合成一条提醒栏：没通过的默认展开，「有话要说」的默认收起
 */
import { useEffect, useRef, useState, type ReactNode } from "react";

export interface StepCardProps {
  no: number;
  title: string;
  /** 「可选」这一步不做也能往下走 */
  optional?: boolean;
  done?: boolean;
  /** 下一步该做这个 */
  current?: boolean;
  status: string;
  /** 状态下面那一行。`warn` 为真时标红 */
  detail?: string;
  warn?: boolean;
  /** 正在跑的进度（「合成中 · 3/6」）。**进度写在这里，不改按钮上的字** */
  running?: string;
  children?: ReactNode;
}

export function StepCard({ no, title, optional, done, current, status, detail, warn, running, children }: StepCardProps) {
  return (
    <div
      className="studio-tile flex min-w-0 flex-col gap-0.5 rounded-xl px-3 py-2"
      style={{ boxShadow: `0 0 0 ${current ? 1.5 : 1}px ${current ? "var(--studio-primary)" : "var(--studio-border-subtle)"}` }}
    >
      <p className="flex items-center gap-1.5 text-[12.5px] font-medium">
        {done ? (
          <span style={{ color: "var(--studio-done-fg)" }} aria-label="已完成">
            ✓
          </span>
        ) : (
          <span className="font-mono text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
            {no}
          </span>
        )}
        <span>{title}</span>
        {optional && (
          <span className="text-[10.5px] font-normal" style={{ color: "var(--studio-text-muted)" }}>
            可选
          </span>
        )}
        {current && (
          <span
            className="ml-auto rounded px-1.5 py-0.5 text-[10.5px] font-normal"
            style={{ background: "color-mix(in srgb, var(--studio-primary) 16%, transparent)", color: "var(--studio-primary)" }}
          >
            下一步
          </span>
        )}
      </p>
      {/* 状态和说明并成一行（2026-10-06 用户：卡片高度做小一些），放不下就截断、悬停看全文 */}
      <p className="truncate text-[11.5px]" style={{ color: "var(--studio-text-2)" }} title={detail ? `${status} · ${detail}` : status}>
        {status}
        {detail && (
          <span className="text-[11px]" style={{ color: warn ? "var(--studio-failed-fg)" : "var(--studio-text-muted)" }}>
            {" · "}
            {detail}
          </span>
        )}
      </p>
      {running && (
        <p className="text-[11px]" style={{ color: "var(--studio-running-fg)" }}>
          {running}
        </p>
      )}
      <div className="mt-auto flex flex-wrap items-center gap-1.5 pt-1">{children}</div>
    </div>
  );
}

/** 卡片里的按钮。`primary` 只给「下一步」那张卡片的主按钮 */
export function StepButton({
  primary,
  danger,
  disabled,
  title,
  onClick,
  children,
}: {
  primary?: boolean;
  danger?: boolean;
  disabled?: boolean;
  title?: string;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      title={title}
      onClick={onClick}
      className="rounded-lg px-2.5 py-0.5 text-[11.5px] leading-5 disabled:opacity-40"
      style={
        primary
          ? { background: "var(--studio-primary)", color: "var(--studio-on-primary)", fontWeight: 500 }
          : { background: "var(--studio-bg-inset)", color: danger ? "var(--studio-failed-fg)" : "var(--studio-text-2)", boxShadow: "0 0 0 1px var(--studio-border)" }
      }
    >
      {children}
    </button>
  );
}

/** 卡片里的「更多」：不常用、或者会覆盖东西的操作收在这里 */
export function MoreMenu({ children, label = "更多" }: { children: ReactNode; label?: string }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  }, [open]);
  return (
    <div ref={ref} className="relative">
      <StepButton onClick={() => setOpen((v) => !v)}>{label} ▾</StepButton>
      {open && (
        <div
          className="absolute left-0 top-full z-30 mt-1 flex min-w-[220px] flex-col gap-0.5 rounded-lg p-1"
          style={{ background: "var(--studio-bg-elevated)", boxShadow: "0 0 0 1px var(--studio-border), 0 8px 24px rgba(0,0,0,0.25)" }}
        >
          {children}
        </div>
      )}
    </div>
  );
}

export function MenuItem({
  onClick,
  onBlur,
  disabled,
  title,
  danger,
  children,
}: {
  onClick: () => void;
  onBlur?: () => void;
  disabled?: boolean;
  title?: string;
  danger?: boolean;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      title={title}
      onClick={onClick}
      onBlur={onBlur}
      className="rounded-md px-2.5 py-1.5 text-left text-[12px] disabled:opacity-40"
      style={{ color: danger ? "var(--studio-failed-fg)" : "var(--studio-text)" }}
    >
      {children}
    </button>
  );
}

export interface Notice {
  title: string;
  items?: string[];
}

/**
 * 提醒栏：一条，不是一摞横幅。
 *
 * `errors` 是没成功、没保存的（失败原文照抄 —— 「ComfyUI 没起」用户自己就能修，概括成「失败」就修不了），默认展开；
 * `infos` 是成了但有话要说的，默认收起。**两类不能混**：以前混在一个标题「没通过检查」下面，存好了的镜头被读成失败。
 */
export function NoticePanel({ errors, infos }: { errors: Notice[]; infos: Notice[] }) {
  const [showInfo, setShowInfo] = useState(false);
  if (errors.length === 0 && infos.length === 0) return null;
  const infoCount = infos.reduce((n, i) => n + Math.max(1, i.items?.length ?? 0), 0);
  const block = (n: Notice, key: string) => (
    <div key={key} className="leading-relaxed">
      <p className="font-medium">{n.title}</p>
      {n.items && n.items.length > 0 && (
        <ul className="mt-0.5 list-disc pl-4">
          {n.items.map((it) => (
            <li key={it}>{it}</li>
          ))}
        </ul>
      )}
    </div>
  );
  return (
    <section className="flex flex-col gap-2 rounded-lg px-3 py-2 text-[11.5px]" style={{ boxShadow: "0 0 0 1px var(--studio-border-subtle)" }} aria-label="提醒">
      {errors.length > 0 && (
        <div className="flex flex-col gap-1.5" style={{ color: "var(--studio-failed-fg)" }}>
          {errors.map((n, i) => block(n, `e${i}`))}
        </div>
      )}
      {infos.length > 0 && (
        <div style={{ color: "var(--studio-text-2)" }}>
          <button type="button" onClick={() => setShowInfo((v) => !v)} className="text-left text-[11.5px]" style={{ color: "var(--studio-text-2)" }}>
            {showInfo ? "▾" : "▸"} {infoCount} 条提醒（不影响出片）
          </button>
          {showInfo && <div className="mt-1.5 flex flex-col gap-1.5" style={{ color: "var(--studio-accent)" }}>{infos.map((n, i) => block(n, `i${i}`))}</div>}
        </div>
      )}
    </section>
  );
}
