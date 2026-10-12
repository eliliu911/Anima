import type { ReactNode } from "react";
import { NodeToolbar, Position } from "@xyflow/react";

/**
 * 选中节点时挂在它**下方**的设定面板（提示词、参考图、引擎、尺寸……）。
 *
 * # 不跟着画布缩放
 *
 * 以前面板直接画在节点里，画布缩到 40% 面板也跟着缩到 40% —— 看全局的时候参数看不清、按钮点不准；
 * 反过来放大看细节时面板又大得挡住半个画布。LibTV 的做法是面板不缩放，只跟着节点走（用户 2026-09-24 对比着看，
 * 说「这个功能就很好……保证参数可实用性，可以拿来用」）。
 *
 * React Flow 的 `NodeToolbar` 正好是这个：渲染在缩放层**外面**，位置按节点换算，大小恒定。
 * 顺带的好处：面板不再算进节点的尺寸，选中节点时节点不会被撑高，「自动整理」量到的也是节点本身。
 *
 * `nodrag nopan nowheel`：在面板上拖选文字、滚动下拉菜单时不拖动节点、不平移缩放画布。
 */
export function NodePanelDock({ visible, children }: { visible: boolean; children: ReactNode }) {
  return (
    <NodeToolbar isVisible={visible} position={Position.Bottom} offset={14} align="center" className="nodrag nopan nowheel">
      {children}
    </NodeToolbar>
  );
}
