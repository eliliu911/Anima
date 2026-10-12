/**
 * 画布的操作方式和快捷键 —— **照 DramaClaw（对标 libtv）来**。
 *
 * # 为什么集中在一个纯模块里
 *
 * 原来快捷键说明面板是一大块写死的 JSX，键盘处理散在 `CanvasView` 和
 * `CanvasZoomControl` 两处。三处各写各的，结果面板里列了一半根本没实现的
 * 东西（⌘G 成组、⌥ 拖复制、V/H 工具、双指平移……），而且没人发现。
 *
 * 现在：
 * - `resolveCanvasShortcut` 是**唯一**的按键 → 动作映射，`CanvasView` 只负责分发
 * - `flowInteractionProps` 是**唯一**的鼠标 / 触控板手势配置
 * - `SHORTCUT_GROUPS` 面板数据里每一行都指向上面两者之一，测试逐条核对 ——
 *   往面板里加一条没实现的，测试直接挂。这是 DC 那边的原则：
 *   「只列画布里真正接好的快捷键，不列想做但没做的」。
 */
import { SelectionMode, type KeyCode } from "@xyflow/react";

export type CanvasTool = "move" | "hand";

export type CanvasAction =
  | "openAddNode"
  | "escape"
  | "selectAll"
  | "copy"
  | "paste"
  | "group"
  | "ungroup"
  | "undo"
  | "redo"
  | "delete"
  | "toggleMinimap"
  | "toggleShortcuts"
  | "organize"
  | "toolMove"
  | "toolHand"
  | "zoomIn"
  | "zoomOut"
  | "fitView";

export interface KeyLike {
  key: string;
  /** 物理键位。**字母键要靠它判断** —— 见下面 `letter` 的注释。 */
  code: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}

/**
 * 字母键按物理键位认，不按 `event.key` 认。
 *
 * **Mac 上按住 Option，`event.key` 会变成另一个字符**：⌥⇧F 给的是 `Ï`，不是 `F`。
 * 原来「整理画布」判断 `e.key === "f"`，所以这个快捷键在 Mac 上一直没生效，
 * 而说明面板里照样列着。
 */
function letter(e: KeyLike, ch: string): boolean {
  return e.code === `Key${ch.toUpperCase()}`;
}

/**
 * 按键 → 画布动作。不认识的返回 `null`，调用方放行。
 *
 * `isTyping` 为真时（焦点在输入框里）只认 Esc —— 否则在节点里打字按个 V
 * 会切到移动工具、按 Delete 会删节点。
 */
export function resolveCanvasShortcut(e: KeyLike, isTyping: boolean): CanvasAction | null {
  if (e.key === "Escape") return "escape";
  if (isTyping) return null;

  const mod = e.metaKey || e.ctrlKey;
  const bare = !mod && !e.altKey && !e.shiftKey;

  if (e.key === "Tab" && !mod && !e.altKey) return "openAddNode";
  if (e.key === "Delete" || e.key === "Backspace") return bare ? "delete" : null;

  // ── 带 ⌘ / Ctrl ──
  if (mod && !e.altKey) {
    if (letter(e, "z")) return e.shiftKey ? "redo" : "undo";
    if (e.ctrlKey && !e.metaKey && letter(e, "y") && !e.shiftKey) return "redo";
    if (e.shiftKey) {
      if (letter(e, "g")) return "ungroup";
      return null;
    }
    if (letter(e, "a")) return "selectAll";
    if (letter(e, "c")) return "copy";
    if (letter(e, "v")) return "paste";
    if (letter(e, "g")) return "group";
    // ⌘+ 在美式键盘上是 ⌘=（不按 Shift），两种都认
    if (e.key === "=" || e.key === "+" || e.code === "Equal" || e.code === "NumpadAdd") return "zoomIn";
    if (e.key === "-" || e.code === "Minus" || e.code === "NumpadSubtract") return "zoomOut";
    if (e.key === "0" || e.code === "Digit0" || e.code === "Numpad0") return "fitView";
    return null;
  }

  if (e.altKey && e.shiftKey && !mod && letter(e, "f")) return "organize";

  if (bare) {
    // V / H 必须是光秃秃的按键：带 ⌘ 的 V 是粘贴，带 Option 的字母多半是输入法在组字
    if (letter(e, "v")) return "toolMove";
    if (letter(e, "h")) return "toolHand";
    if (letter(e, "m")) return "toggleMinimap";
  }
  if (e.key === "?" && !mod) return "toggleShortcuts";
  return null;
}

/** Consume native select/copy behavior only inside the focused canvas.
 * Text editors keep their native selection and clipboard shortcuts.
 */
export function dispatchCanvasShortcut(
  event: KeyLike & { defaultPrevented: boolean; isComposing?: boolean; preventDefault(): void; stopPropagation(): void },
  context: { withinCanvas: boolean; isTyping: boolean },
  run: (action: CanvasAction) => void,
  clearTextSelection: () => void,
): boolean {
  if (!context.withinCanvas || event.defaultPrevented || event.isComposing) return false;
  const action = resolveCanvasShortcut(event, context.isTyping);
  if (!action) return false;
  if (action !== "escape") {
    event.preventDefault();
    event.stopPropagation();
  }
  if (action === "selectAll") clearTextSelection();
  run(action);
  return true;
}

/** 鼠标左键 = 0，中键 = 1。 */
const MIDDLE = 1;
const LEFT = 0;

/**
 * ReactFlow 的手势配置。
 *
 * | 手势 | 移动工具（V） | 抓手工具（H） |
 * |---|---|---|
 * | 滚轮 / 双指滑动 | 平移（触控板平移开着时） | 同左 |
 * | 双指捏合 / ⌘+滚轮 | 缩放 | 缩放 |
 * | 左键拖空白处 | **框选** | 平移 |
 * | 中键拖 | 平移 | 平移 |
 * | 空格 + 左键拖 | 平移 | 平移 |
 * | ⌘ / Ctrl + 点击 | 多选 | — |
 *
 * **触控板平移默认开**（和 DC 一样）：笔记本没有鼠标中键，开箱就要能两指滑动
 * 平移。关掉之后滚轮回到「缩放」—— 给习惯滚轮缩放的鼠标用户。
 *
 * 用户 2026-09-16 反馈的问题就是这一条：原来没设 `panOnScroll`，ReactFlow 默认
 * 滚轮 = 缩放，前后滚一下画面就放大缩小。
 */
export interface FlowInteraction {
  panOnScroll: boolean;
  zoomOnScroll: boolean;
  zoomOnPinch: boolean;
  panOnDrag: number[];
  panActivationKeyCode: KeyCode;
  selectionOnDrag: boolean;
  selectionMode: SelectionMode;
  selectionKeyCode: KeyCode | null;
  multiSelectionKeyCode: KeyCode;
  deleteKeyCode: KeyCode | null;
  zoomOnDoubleClick: boolean;
  nodesDraggable: boolean;
  elementsSelectable: boolean;
}

export function flowInteractionProps(tool: CanvasTool, trackpadPan: boolean): FlowInteraction {
  const hand = tool === "hand";
  return {
    panOnScroll: trackpadPan,
    zoomOnScroll: !trackpadPan,
    zoomOnPinch: true,
    // 抓手下左键也能拖着平移；移动工具下左键留给框选
    panOnDrag: hand ? [LEFT, MIDDLE] : [MIDDLE],
    panActivationKeyCode: "Space",
    selectionOnDrag: !hand,
    selectionMode: SelectionMode.Partial,
    // 框选不需要按 Shift，直接左键拖（libtv / DC 同款）
    selectionKeyCode: null,
    multiSelectionKeyCode: ["Meta", "Control"],
    // 删除自己处理：要能进撤销栈、要能一次删多个。ReactFlow 自带的删不进撤销栈
    deleteKeyCode: null,
    zoomOnDoubleClick: false,
    // 抓手下节点不可拖不可选：左键落在节点上也交给画布去平移
    nodesDraggable: !hand,
    elementsSelectable: !hand,
  };
}

// ─────────────────────────────────────────────────────────────────
// 说明面板
// ─────────────────────────────────────────────────────────────────

export interface ShortcutRow {
  label: string;
  /** 键帽。Mac 符号，渲染时不再转换 */
  keys?: string[];
  /** 键帽后面跟的手势说明，如「+ 拖动」 */
  gesture?: string;
  /** 这一行由哪个键盘动作实现（测试逐条核对它真的能被按出来） */
  action?: CanvasAction;
  /**
   * 这一行由哪个手势配置实现（测试核对 `flowInteractionProps` 里真的开着）。
   * 键盘动作和手势总得有一个 —— 两个都没有的行测试会挂。
   */
  flow?: "trackpadPan" | "pinchZoom" | "middleDragPan" | "spaceDragPan" | "boxSelect" | "multiSelect" | "handDragPan";
  /** 不是按键也不是 ReactFlow 配置、而是节点拖动回调里实现的（如 ⌥ 拖复制） */
  handler?: "altDragDuplicate" | "cancelPlacement";
}

export interface ShortcutGroup {
  title: string;
  rows: ShortcutRow[];
}

export const SHORTCUT_GROUPS: ShortcutGroup[] = [
  {
    title: "创作",
    rows: [
      { label: "新建节点", keys: ["Tab"], action: "openAddNode" },
      { label: "关闭添加节点面板", keys: ["Esc"], action: "escape" },
      { label: "全选节点", keys: ["⌘", "A"], action: "selectAll" },
      { label: "复制", keys: ["⌘", "C"], action: "copy" },
      { label: "粘贴", keys: ["⌘", "V"], action: "paste" },
      { label: "多选", keys: ["⌘"], gesture: "+ 点击", flow: "multiSelect" },
      { label: "成组", keys: ["⌘", "G"], action: "group" },
      { label: "解组", keys: ["⇧", "⌘", "G"], action: "ungroup" },
    ],
  },
  {
    title: "移动画布",
    rows: [
      { label: "触控板", gesture: "双指滑动", flow: "trackpadPan" },
      { label: "鼠标", gesture: "中键拖动", flow: "middleDragPan" },
      { label: "键盘", keys: ["空格"], gesture: "+ 拖动", flow: "spaceDragPan" },
      { label: "框选节点", gesture: "左键拖动空白处", flow: "boxSelect" },
      { label: "移动工具", keys: ["V"], action: "toolMove" },
      { label: "抓手工具", keys: ["H"], gesture: "+ 左键拖动", action: "toolHand" },
      { label: "整理画布", keys: ["⌥", "⇧", "F"], action: "organize" },
    ],
  },
  {
    title: "缩放",
    rows: [
      { label: "缩放", gesture: "双指捏合 / ⌘ + 滚轮", flow: "pinchZoom" },
      { label: "放大", keys: ["⌘", "+"], action: "zoomIn" },
      { label: "缩小", keys: ["⌘", "−"], action: "zoomOut" },
      { label: "适合屏幕", keys: ["⌘", "0"], action: "fitView" },
    ],
  },
  {
    title: "其他",
    rows: [
      { label: "撤销", keys: ["⌘", "Z"], action: "undo" },
      { label: "重做", keys: ["⇧", "⌘", "Z"], action: "redo" },
      { label: "删除", keys: ["Delete"], action: "delete" },
      { label: "缩略图开关", keys: ["M"], action: "toggleMinimap" },
      { label: "快捷键说明", keys: ["?"], action: "toggleShortcuts" },
    ],
  },
];

/**
 * 把一行面板条目变回一次按键，供测试核对「列出来的真的按得出来」。
 * 手势行返回 `null`（由 `flowInteractionProps` 另行核对）。
 */
export function keyEventForRow(row: ShortcutRow): KeyLike | null {
  if (!row.keys || !row.action) return null;
  const ev: KeyLike = { key: "", code: "", metaKey: false, ctrlKey: false, altKey: false, shiftKey: false };
  for (const k of row.keys) {
    if (k === "⌘") ev.metaKey = true;
    else if (k === "⌥") ev.altKey = true;
    else if (k === "⇧") ev.shiftKey = true;
    else if (k === "Tab" || k === "Delete") ev.key = k;
    else if (k === "Esc") ev.key = "Escape";
    else if (k === "空格") { ev.key = " "; ev.code = "Space"; }
    else if (k === "+") { ev.key = "="; ev.code = "Equal"; }
    else if (k === "−") { ev.key = "-"; ev.code = "Minus"; }
    else if (k === "?") ev.key = "?";
    else if (/^[0-9]$/.test(k)) { ev.key = k; ev.code = `Digit${k}`; }
    else if (/^[A-Z]$/.test(k)) { ev.key = ev.altKey ? "Ï" : k.toLowerCase(); ev.code = `Key${k}`; }
  }
  return ev;
}
