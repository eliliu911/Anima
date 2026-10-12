import type { ReactNode } from "react";
import { X } from "./icons";

interface NodeHeaderProps {
  icon?: ReactNode;
  title: string;
  action?: ReactNode;
  onDelete?: () => void;
}

export function NodeHeader({ icon, title, action, onDelete }: NodeHeaderProps) {
  return (
    <div className="flex items-center justify-between px-1 py-1 text-xs text-white/70 select-none">
      <div className="flex items-center gap-1.5 font-medium">
        {icon && <span className="text-white/50">{icon}</span>}
        <span className="text-white/80">{title}</span>
      </div>
      <div className="flex items-center gap-1.5">
        {action}
        {onDelete && (
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              onDelete();
            }}
            className="rounded p-1 text-white/40 hover:bg-white/10 hover:text-white transition-colors"
            title="删除节点"
          >
            <X className="h-3 w-3" />
          </button>
        )}
      </div>
    </div>
  );
}

