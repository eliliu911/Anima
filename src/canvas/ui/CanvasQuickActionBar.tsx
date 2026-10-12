import { useState } from "react";
import {
  Plus,
  Play,
  History,
  Keyboard,
  HelpCircle,
  FolderOpen,
} from "lucide-react";
import * as Tooltip from "@radix-ui/react-tooltip";
import { CanvasAddNodePanel } from "./CanvasAddNodePanel";
import { closeWithToggle } from "./menuClose";
import { CanvasShortcutsPanel } from "./CanvasShortcutsPanel";
import { useCanvasStore } from "../store/useCanvasStore";
import { executeNodePipeline } from "../engine/dagExecutor";
import type { CanvasNodeType } from "../types";

interface CanvasQuickActionBarProps {
  onAddNode: (type: CanvasNodeType, initialData?: Record<string, unknown>) => void;
  onClear: () => void;
  isAddMenuOpen?: boolean;
  onToggleAddMenu?: () => void;
  showShortcuts?: boolean;
  onToggleShortcuts?: () => void;
  /** 工作流目录开着没有。**必须有一个入口** —— 做了面板而到不了等于没做 */
  showWorkflows?: boolean;
  onToggleWorkflows?: () => void;
}

function DockItem({
  label,
  shortcut,
  children,
  onClick,
  disabled = false,
  active = false,
  className = "",
}: {
  label: string;
  shortcut?: string;
  children: React.ReactNode;
  onClick?: () => void;
  disabled?: boolean;
  active?: boolean;
  className?: string;
}) {
  return (
    <Tooltip.Root delayDuration={150}>
      <Tooltip.Trigger asChild>
        <button
          type="button"
          onClick={onClick}
          disabled={disabled}
          className={`flex h-8 w-8 items-center justify-center rounded-xl transition-all duration-150 relative ${
            disabled
              ? "opacity-30 cursor-not-allowed text-zinc-500"
              : active
              ? "bg-blue-500/20 text-blue-400 border border-blue-500/40 shadow-[0_0_12px_rgba(59,130,246,0.3)]"
              : "text-zinc-400 hover:text-zinc-100 hover:bg-white/10"
          } ${className}`}
        >
          {children}
        </button>
      </Tooltip.Trigger>
      <Tooltip.Portal>
        <Tooltip.Content
          side="top"
          sideOffset={8}
          className="z-50 flex items-center gap-1.5 rounded-lg border border-white/10 bg-zinc-950/90 px-2.5 py-1 text-[11px] font-medium text-zinc-200 shadow-2xl backdrop-blur-md animate-in fade-in-0 zoom-in-95"
        >
          <span>{label}</span>
          {shortcut && (
            <kbd className="rounded bg-white/10 px-1 py-0.5 text-[10px] font-mono text-zinc-400">
              {shortcut}
            </kbd>
          )}
          <Tooltip.Arrow className="fill-zinc-950/90" />
        </Tooltip.Content>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}

export function CanvasQuickActionBar({
  onAddNode,
  isAddMenuOpen: propIsAddMenuOpen,
  onToggleAddMenu: propOnToggleAddMenu,
  showShortcuts: propShowShortcuts,
  onToggleShortcuts: propOnToggleShortcuts,
  showWorkflows,
  onToggleWorkflows,
}: CanvasQuickActionBarProps) {
  const [internalAddMenuOpen, setInternalAddMenuOpen] = useState(false);
  const [internalShowShortcuts, setInternalShowShortcuts] = useState(false);
  const [showHelp, setShowHelp] = useState(false);

  const isAddMenuOpen = propIsAddMenuOpen ?? internalAddMenuOpen;
  const toggleAddMenu = propOnToggleAddMenu ?? (() => setInternalAddMenuOpen((v) => !v));
  /**
   * **幂等地关掉添加菜单。**
   *
   * 上层可能只给一个 toggle（`propOnToggleAddMenu`），而 toggle 调两次
   * 等于没调 —— 「加完节点菜单不关」这个 bug 就是这么来的。
   * 想关的地方一律走这个，不要直接调 toggle。
   */
  const closeAddMenu = () => closeWithToggle(isAddMenuOpen, toggleAddMenu);

  const showShortcuts = propShowShortcuts ?? internalShowShortcuts;
  const toggleShortcuts = propOnToggleShortcuts ?? (() => setInternalShowShortcuts((v) => !v));

  const selectedNodeId = useCanvasStore((s) => s.selectedNodeId);

  const handleRunActive = async () => {
    if (!selectedNodeId) return;
    await executeNodePipeline(selectedNodeId);
  };

  return (
    <Tooltip.Provider>
      {/* **居中用布局，不用 `-translate-x-1/2`**（2026-10-08 用户：「为什么 libtv 里的文字比我们的清晰」）：
          宽度是奇数时 transform 落在半个像素上，WebKit 不把 transform 对齐到像素，整排文字发虚。
          铺满宽度的外层不吃点击（pointer-events-none），里面的面板和工具条照常点 */}
      <div className="pointer-events-none absolute inset-x-0 bottom-6 z-30 flex flex-col items-center select-none [&>*]:pointer-events-auto">
        {/* 快捷键说明面板 */}
        <CanvasShortcutsPanel
          isOpen={showShortcuts}
          onClose={() => {
            if (propOnToggleShortcuts) propOnToggleShortcuts();
            else setInternalShowShortcuts(false);
          }}
        />

        {/* 展开的添加面板 */}
        {isAddMenuOpen && (
          <div className="mb-3">
            <CanvasAddNodePanel
              onSelectNode={(type, initData) => {
                onAddNode(type, initData);
                // **加完就关。** 用 `closeAddMenu` 而不是 `toggleAddMenu`：
                // toggle 被调两次就等于没调，而这段以前正是被调了两次
                // （面板选完自己又 `onClose` 了一下），表现是点了不关。
                closeAddMenu();
              }}
            />
          </div>
        )}

        {/* 帮助指南面板 */}
        {showHelp && (
          <div className="mb-3 w-80 rounded-2xl border border-white/10 bg-[#101217]/95 p-4 text-xs text-white/80 shadow-2xl backdrop-blur-2xl animate-in fade-in-0 zoom-in-95">
            <div className="mb-2 font-semibold text-white/90">使用指引与技巧</div>
            <ul className="space-y-1.5 text-white/60">
              <li>• <span className="text-white/80">添加节点</span>：按 <kbd className="px-1 rounded bg-white/10 font-mono">Tab</kbd> 或点击下方 [+]</li>
              <li>• <span className="text-white/80">快速连线</span>：按住卡片右侧边缘 (+) 拖拽连线</li>
              <li>• <span className="text-white/80">网格吸附</span>：右下角控制菜单内磁吸图标，拖拽自动 20px 对齐</li>
              <li>• <span className="text-white/80">自动整理</span>：右下角控制菜单内排版图标，按 DAG 拓扑一键排列</li>
              <li>• <span className="text-white/80">显示/隐藏连线</span>：右下角控制菜单内眼睛图标，随时切换清爽视图</li>
              <li>• <span className="text-white/80">缩放视口</span>：滚轮捏合或快捷键 <kbd className="px-1 rounded bg-white/10 font-mono">⌘ 0</kbd> 自适应全屏</li>
            </ul>
          </div>
        )}

        {/* 底部悬浮毛玻璃胶囊 Dock 栏 (严格对齐 Liblib / DramaClaw / shadcn 风格) */}
        <div className="flex items-center gap-1 rounded-2xl border border-white/15 bg-[#12141a]/92 p-1.5 shadow-2xl backdrop-blur-2xl">
          {/* [+] 添加触发按钮 */}
          <DockItem
            label={isAddMenuOpen ? "关闭菜单" : "添加新节点"}
            shortcut="Tab"
            onClick={() => {
              toggleAddMenu();
              if (showShortcuts) toggleShortcuts();
              setShowHelp(false);
            }}
            className={
              isAddMenuOpen
                ? "!bg-white !text-black rotate-45 shadow-lg shadow-white/20"
                : "!bg-white !text-black hover:scale-105 shadow-md shadow-white/10"
            }
          >
            <Plus className="h-4 w-4 stroke-[2.5]" />
          </DockItem>

          {/* [Play] 执行生成当前选中节点 */}
          <DockItem
            label={selectedNodeId ? "生成当前节点" : "未选中生成节点"}
            shortcut="Enter"
            disabled={!selectedNodeId}
            onClick={handleRunActive}
          >
            <Play className="h-3.5 w-3.5 fill-current" />
          </DockItem>

          <div className="h-4 w-px bg-white/10 mx-0.5" />

          {/* [History] 历史记录 */}
          <DockItem label="生成历史记录">
            <History className="h-3.5 w-3.5" />
          </DockItem>

          {/* [FolderOpen] 工作流目录：把当前这套接法存下来 / 载入一套 */}
          <DockItem
            label="工作流"
            active={showWorkflows}
            onClick={() => {
              onToggleWorkflows?.();
              closeAddMenu();
              if (showShortcuts) toggleShortcuts();
              setShowHelp(false);
            }}
          >
            <FolderOpen className="h-3.5 w-3.5" />
          </DockItem>

          {/* [Keyboard] 快捷键说明 */}
          <DockItem
            label="快捷键说明"
            shortcut="?"
            active={showShortcuts}
            onClick={() => {
              toggleShortcuts();
              closeAddMenu();
              setShowHelp(false);
            }}
          >
            <Keyboard className="h-3.5 w-3.5" />
          </DockItem>

          {/* [HelpCircle] 帮助指南 */}
          <DockItem
            label="使用指引与帮助"
            active={showHelp}
            onClick={() => {
              setShowHelp(!showHelp);
              if (showShortcuts) toggleShortcuts();
              closeAddMenu();
            }}
          >
            <HelpCircle className="h-3.5 w-3.5" />
          </DockItem>
        </div>
      </div>
    </Tooltip.Provider>
  );
}
