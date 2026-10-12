import { useEffect, useRef } from "react";

/**
 * 「点外面 / 按 Esc 关掉」的浮层。
 *
 * # 为什么不用一个 `fixed inset-0` 的透明层接点击
 *
 * 那是最常见的写法，但它**盖在触发它的元素上**：卡片立刻失去 `:hover`，
 * 抬升动画退回，元素移动，鼠标又落回卡片上 —— 于是 hover 反复横跳，
 * 表现就是卡片疯狂闪烁。而且遮罩把 hover 吃掉之后，
 * 鼠标移出去也不会触发任何 leave，菜单就不肯关。
 *
 * 改成在 document 上听 `pointerdown` —— 不插入任何元素，
 * 什么都不遮，hover 状态完全不受影响。
 *
 * 用 `pointerdown` 而不是 `click`：`click` 要等抬起，
 * 中间那段时间里浮层还在，拖拽选中会穿帮。
 */
export function useDismissable<T extends HTMLElement>(
  open: boolean,
  onClose: () => void,
) {
  const ref = useRef<T | null>(null);

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (e: PointerEvent) => {
      const el = ref.current;
      if (el && e.target instanceof Node && el.contains(e.target)) return;
      onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    // `capture` 让它先于被点元素自己的处理器跑 —— 否则点另一张卡片的 ⋮
    // 会先开新的再关旧的，两个菜单会同时闪一下。
    document.addEventListener("pointerdown", onPointerDown, true);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown, true);
      document.removeEventListener("keydown", onKey);
    };
  }, [open, onClose]);

  return ref;
}
