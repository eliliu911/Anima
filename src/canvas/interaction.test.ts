import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it, vi } from "vitest";
import {
  dispatchCanvasShortcut,
  SHORTCUT_GROUPS,
  flowInteractionProps,
  keyEventForRow,
  resolveCanvasShortcut,
  type KeyLike,
} from "./interaction";

const key = (over: Partial<KeyLike>): KeyLike => ({
  key: "", code: "", metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, ...over,
});

const rows = SHORTCUT_GROUPS.flatMap((g) => g.rows);

/**
 * 已经真正接好的「拖动回调里实现的」手势。**现在是空的**：⌥ 拖复制、
 * 取消节点放置还没做，所以面板里不能列。做完一个往这里加一个。
 */
const IMPLEMENTED_HANDLERS = new Set<string>();

describe("快捷键说明面板只列真正接好的", () => {
  /**
   * 原来面板是手写 JSX，列了 ⌘G 成组、⌥ 拖复制、V/H 工具、双指平移……
   * 一半没实现。这条逐行核对：**每一行要么能被按出来，要么手势配置里真开着**。
   */
  it("每一行都有实现来源", () => {
    for (const r of rows) {
      expect(r.action || r.flow || r.handler, `「${r.label}」没有指向任何实现`).toBeTruthy();
    }
  });

  it("列出的每个组合键都能被 resolveCanvasShortcut 按出来", () => {
    for (const r of rows) {
      const ev = keyEventForRow(r);
      if (!ev) continue;
      expect(resolveCanvasShortcut(ev, false), `「${r.label}」${r.keys?.join("")} 按不出来`).toBe(r.action);
    }
  });

  it("手势行在 flowInteractionProps 里真的开着", () => {
    const move = flowInteractionProps("move", true);
    const hand = flowInteractionProps("hand", true);
    for (const r of rows.filter((x) => x.flow)) {
      switch (r.flow) {
        case "trackpadPan":
          expect(move.panOnScroll && !move.zoomOnScroll).toBe(true);
          break;
        case "pinchZoom":
          expect(move.zoomOnPinch).toBe(true);
          break;
        case "middleDragPan":
          expect(move.panOnDrag).toContain(1);
          expect(hand.panOnDrag).toContain(1);
          break;
        case "spaceDragPan":
          expect(move.panActivationKeyCode).toBe("Space");
          break;
        case "boxSelect":
          expect(move.selectionOnDrag).toBe(true);
          expect(move.selectionKeyCode).toBeNull();
          expect(move.panOnDrag, "左键被平移占了就框选不了").not.toContain(0);
          break;
        case "multiSelect":
          expect(move.multiSelectionKeyCode).toContain("Meta");
          break;
        case "handDragPan":
          expect(hand.panOnDrag).toContain(0);
          break;
        default:
          throw new Error(`没核对的手势：${r.flow}`);
      }
    }
  });

  it("拖动回调类的手势只有做完了才能列", () => {
    for (const r of rows.filter((x) => x.handler)) {
      expect(IMPLEMENTED_HANDLERS.has(r.handler!), `「${r.label}」还没实现`).toBe(true);
    }
  });
});

describe("画布手势（照 DramaClaw）", () => {
  /** 用户 2026-09-16 报的：前后滚滚轮是放大缩小，应该是上下平移。 */
  it("默认滚轮 / 双指滑动是平移，不是缩放", () => {
    const p = flowInteractionProps("move", true);
    expect(p.panOnScroll).toBe(true);
    expect(p.zoomOnScroll).toBe(false);
    expect(p.zoomOnPinch).toBe(true);
  });

  it("关掉触控板平移后滚轮回到缩放", () => {
    const p = flowInteractionProps("move", false);
    expect(p.panOnScroll).toBe(false);
    expect(p.zoomOnScroll).toBe(true);
  });

  it("抓手工具下左键拖动平移、节点不可拖不可选", () => {
    const p = flowInteractionProps("hand", true);
    expect(p.panOnDrag).toContain(0);
    expect(p.selectionOnDrag).toBe(false);
    expect(p.nodesDraggable).toBe(false);
    expect(p.elementsSelectable).toBe(false);
  });

  /** ReactFlow 自带的删除不进撤销栈，而且只删它自己知道的那几个。 */
  it("删除键交给我们自己处理", () => {
    expect(flowInteractionProps("move", true).deleteKeyCode).toBeNull();
  });

  it("双击不缩放", () => {
    expect(flowInteractionProps("move", true).zoomOnDoubleClick).toBe(false);
  });
});

describe("按键解析", () => {
  /**
   * **Mac 上按住 Option，`event.key` 变成别的字符**：⌥⇧F 给的是 `Ï`。
   * 原来判断 `e.key === "f"`，整理画布在 Mac 上一直没生效。
   */
  it("⌥⇧F 在 Mac 上 key 是 Ï 也能认出来", () => {
    expect(resolveCanvasShortcut(key({ key: "Ï", code: "KeyF", altKey: true, shiftKey: true }), false)).toBe("organize");
  });

  it("⌘V 是粘贴，光秃秃的 V 才是移动工具", () => {
    expect(resolveCanvasShortcut(key({ key: "v", code: "KeyV", metaKey: true }), false)).toBe("paste");
    expect(resolveCanvasShortcut(key({ key: "v", code: "KeyV" }), false)).toBe("toolMove");
    expect(resolveCanvasShortcut(key({ key: "h", code: "KeyH" }), false)).toBe("toolHand");
  });

  /** 在节点的输入框里打字，按 V 不能切工具、按 Delete 不能删节点。 */
  it("焦点在输入框里只认 Esc", () => {
    expect(resolveCanvasShortcut(key({ key: "v", code: "KeyV" }), true)).toBeNull();
    expect(resolveCanvasShortcut(key({ key: "Backspace" }), true)).toBeNull();
    expect(resolveCanvasShortcut(key({ key: "z", code: "KeyZ", metaKey: true }), true)).toBeNull();
    expect(resolveCanvasShortcut(key({ key: "Escape" }), true)).toBe("escape");
  });

  it("⌘+ 在美式键盘上是 ⌘=，两种都算放大", () => {
    expect(resolveCanvasShortcut(key({ key: "=", code: "Equal", metaKey: true }), false)).toBe("zoomIn");
    expect(resolveCanvasShortcut(key({ key: "+", code: "Equal", metaKey: true, shiftKey: false }), false)).toBe("zoomIn");
  });

  it("⇧⌘Z 和 Ctrl+Y 都是重做", () => {
    expect(resolveCanvasShortcut(key({ key: "z", code: "KeyZ", metaKey: true, shiftKey: true }), false)).toBe("redo");
    expect(resolveCanvasShortcut(key({ key: "y", code: "KeyY", ctrlKey: true }), false)).toBe("redo");
  });

  it("带修饰键的 Delete 不删节点", () => {
    expect(resolveCanvasShortcut(key({ key: "Backspace", metaKey: true }), false)).toBeNull();
  });
});

describe("缩放快捷键只挂一处", () => {
  /** 挪到 CanvasView 统一分发之后，CanvasZoomControl 里那份不删的话一次按键缩放两次。 */
  it("CanvasZoomControl 不再自己监听键盘", () => {
    const src = readFileSync(fileURLToPath(new URL("./ui/CanvasZoomControl.tsx", import.meta.url)), "utf8");
    expect(src).not.toMatch(/addEventListener\("keydown"/);
  });

  it("面板不再是手写 JSX，读 SHORTCUT_GROUPS", () => {
    const src = readFileSync(fileURLToPath(new URL("./ui/CanvasShortcutsPanel.tsx", import.meta.url)), "utf8");
    expect(src).toContain("SHORTCUT_GROUPS");
    expect(src).not.toContain("取消节点放置");
  });
});


describe("全选节点与原生文字选择隔离", () => {
  const event = (over: Partial<KeyLike> = {}) => ({
    ...key({ key: "a", code: "KeyA", metaKey: true, ...over }),
    defaultPrevented: false, preventDefault: vi.fn(), stopPropagation: vi.fn(),
  });
  it("⌘A 和 Ctrl+A 全选节点并阻止页面文字全选，清理旧文字选区", () => {
    for (const modifiers of [{ metaKey: true }, { metaKey: false, ctrlKey: true }]) {
      const e = event(modifiers), run = vi.fn(), clear = vi.fn();
      expect(dispatchCanvasShortcut(e, { withinCanvas: true, isTyping: false }, run, clear)).toBe(true);
      expect(run).toHaveBeenCalledWith("selectAll");
      expect(e.preventDefault).toHaveBeenCalledOnce();
      expect(e.stopPropagation).toHaveBeenCalledOnce();
      expect(clear).toHaveBeenCalledOnce();
    }
  });
  it("输入框、富文本编辑器和画布外不拦截文字全选、复制、粘贴", () => {
    for (const context of [{ withinCanvas: true, isTyping: true }, { withinCanvas: false, isTyping: false }]) {
      for (const letter of ["a", "c", "v"]) {
        const e = event({ key: letter, code: `Key${letter.toUpperCase()}` });
        const run = vi.fn(), clear = vi.fn();
        expect(dispatchCanvasShortcut(e, context, run, clear)).toBe(false);
        expect(e.preventDefault).not.toHaveBeenCalled();
        expect(run).not.toHaveBeenCalled();
        expect(clear).not.toHaveBeenCalled();
      }
    }
  });
  it("画布复制交给节点复制，阻止浏览器复制 Anima 页面文字", () => {
    const e = event({ key: "c", code: "KeyC" }), run = vi.fn();
    dispatchCanvasShortcut(e, { withinCanvas: true, isTyping: false }, run, vi.fn());
    expect(run).toHaveBeenCalledWith("copy");
    expect(e.preventDefault).toHaveBeenCalledOnce();
  });
  it("不抢占已处理事件或输入法组字，也不误认额外修饰键", () => {
    for (const e of [{ ...event(), defaultPrevented: true }, { ...event(), isComposing: true },
      event({ shiftKey: true }), event({ altKey: true })]) {
      const run = vi.fn();
      expect(dispatchCanvasShortcut(e, { withinCanvas: true, isTyping: false }, run, vi.fn())).toBe(false);
      expect(run).not.toHaveBeenCalled();
    }
  });
});
