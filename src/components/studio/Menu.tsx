/**
 * 下拉菜单项。
 *
 * **每一项都带图标。** 纯文字的菜单要逐行读才知道有什么，
 * 图标让「删除」在余光里就能认出来 —— 尤其是危险项。
 */
import type * as React from "react";

export const MenuIcon = {
  open: () => (
    <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <path d="M9.5 2.5H13.5V6.5" />
      <path d="M13.5 2.5 8 8" />
      <path d="M12.5 9.8v3a1.2 1.2 0 0 1-1.2 1.2H3.2A1.2 1.2 0 0 1 2 12.8V4.7a1.2 1.2 0 0 1 1.2-1.2h3" />
    </svg>
  ),
  trash: () => (
    <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <path d="M2.5 4h11" />
      <path d="M6 4V2.8A.8.8 0 0 1 6.8 2h2.4a.8.8 0 0 1 .8.8V4" />
      <path d="M4 4v8.2a1.3 1.3 0 0 0 1.3 1.3h5.4A1.3 1.3 0 0 0 12 12.2V4" />
      <path d="M6.6 6.8v4M9.4 6.8v4" opacity=".6" />
    </svg>
  ),
  series: () => (
    <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <rect x="2" y="3" width="12" height="9" rx="1.4" />
      <path d="M2 6h12" />
      <path d="M5 3v3M11 3v3" opacity=".55" />
    </svg>
  ),
  file: () => (
    <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <path d="M4 2h5l3 3v9H4z" />
      <path d="M9 2v3h3" opacity=".6" />
      <path d="M6 8.5h4M6 11h2.6" opacity=".6" />
    </svg>
  ),
  image: () => (
    <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <rect x="2" y="3" width="12" height="10" rx="1.4" />
      <circle cx="5.8" cy="6.6" r="1.2" opacity=".7" />
      <path d="m3 12 3.6-3.4 2.4 2.2 1.8-1.6L14 12" opacity=".7" />
    </svg>
  ),
  canvas: () => (
    <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <rect x="2" y="2" width="4" height="4" rx="0.8" />
      <rect x="10" y="10" width="4" height="4" rx="0.8" />
      <circle cx="12" cy="4" r="2" />
      <circle cx="4" cy="12" r="2" />
      <path d="M6 4h4M4 6v4M12 6v4M6 12h4" strokeDasharray="1 1" opacity=".6" />
    </svg>
  ),
};

export function MenuItem({
  icon: Icon,
  label,
  hint,
  danger,
  onClick,
}: {
  icon?: () => React.ReactElement;
  label: string;
  hint?: string;
  danger?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="studio-menu-item"
      data-danger={danger || undefined}
    >
      {Icon ? <Icon /> : <span className="w-[13px]" />}
      <span className="flex min-w-0 flex-col">
        <span className="truncate">{label}</span>
        {hint && (
          <span className="truncate text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
            {hint}
          </span>
        )}
      </span>
    </button>
  );
}
