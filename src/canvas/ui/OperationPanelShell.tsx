import { useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { Minimize2 } from "./icons";

interface OperationPanelShellProps {
  children: ReactNode | ((props: { isExpanded: boolean; toggleExpand: () => void }) => ReactNode);
  className?: string;
  widthClass?: string;
}

export function OperationPanelShell({
  children,
  className = "",
  widthClass = "w-[560px]",
}: OperationPanelShellProps) {
  const [expanded, setExpanded] = useState(false);
  const toggleExpand = () => setExpanded((v) => !v);

  const content = typeof children === "function" ? children({ isExpanded: expanded, toggleExpand }) : children;

  if (!expanded) {
    return (
      <div className={`nodrag nopan nowheel relative flex flex-col gap-3 rounded-2xl border border-white/10 bg-[#282828] p-3.5 shadow-2xl transition-all ${widthClass} ${className}`}>
        {content}
      </div>
    );
  }

  return createPortal(
    <div
      className="fixed inset-0 z-[100] flex items-center justify-center bg-black/70 p-6 backdrop-blur-md"
      onClick={() => setExpanded(false)}
    >
      <div
        className="relative w-[720px] max-w-full flex flex-col gap-3 rounded-2xl border border-white/15 bg-[#282828] p-5 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <button
          type="button"
          onClick={() => setExpanded(false)}
          className="absolute top-4 right-4 text-white/50 hover:text-white transition-colors"
          title="收起"
        >
          <Minimize2 className="h-4 w-4" />
        </button>
        {content}
      </div>
    </div>,
    document.body,
  );
}

