import React from "react";
import { X } from "./icons";
import { SHORTCUT_GROUPS } from "../interaction";

interface CanvasShortcutsPanelProps {
  isOpen: boolean;
  onClose: () => void;
}

function KeyBadge({ children }: { children: React.ReactNode }) {
  return (
    <kbd className="inline-flex items-center justify-center min-w-[24px] h-6 px-1.5 rounded-md bg-white/10 border border-white/15 text-[11px] font-mono text-white/90 shadow-sm">
      {children}
    </kbd>
  );
}

/**
 * 快捷键说明。**内容来自 `SHORTCUT_GROUPS`，不在这里写死。**
 *
 * 原来这里是一整块手写 JSX，和实际的键盘处理是两份东西，结果列了一半没实现的
 * 快捷键（⌘G 成组、⌥ 拖复制、V/H 工具、双指平移……）。现在面板只渲染数据，
 * 数据里每一行都有测试核对它真的接好了。
 */
export function CanvasShortcutsPanel({ isOpen, onClose }: CanvasShortcutsPanelProps) {
  if (!isOpen) return null;

  return (
    <div
      className="absolute bottom-full mb-3 z-50 w-[800px] max-w-[95vw] rounded-2xl border border-white/15 bg-[#0e1017]/95 p-6 shadow-2xl backdrop-blur-2xl text-xs text-white/80 select-none animate-in fade-in-0 zoom-in-95 duration-150"
      onMouseDown={(e) => e.stopPropagation()}
    >
      <button
        type="button"
        onClick={onClose}
        className="absolute top-4 right-4 rounded-lg p-1.5 text-white/40 hover:bg-white/10 hover:text-white transition-colors"
        title="关闭快捷键说明 (Esc)"
      >
        <X className="h-4 w-4" />
      </button>

      <div className="grid grid-cols-4 gap-6">
        {SHORTCUT_GROUPS.map((group) => (
          <div key={group.title} className="flex flex-col gap-3 min-w-0">
            <div className="text-white/90 font-semibold text-[13px] tracking-wide">{group.title}</div>
            <div className="flex flex-col gap-2.5">
              {group.rows.map((row) => (
                <div key={row.label} className="flex items-center justify-between gap-2">
                  <span className="truncate text-white/70">{row.label}</span>
                  <span className="flex shrink-0 items-center gap-1">
                    {row.keys?.map((k) => <KeyBadge key={k}>{k}</KeyBadge>)}
                    {row.gesture && (
                      <span className="text-[11px] text-white/45 whitespace-nowrap">{row.gesture}</span>
                    )}
                  </span>
                </div>
              ))}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
