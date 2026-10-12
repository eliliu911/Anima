import { useEffect, useRef } from "react";

/**
 * 从访达拖文件进来 —— **必须走 Tauri 的原生拖放事件，HTML5 的 `drop` 收不到。**
 *
 * # 为什么 `onDrop` 不灵
 *
 * Tauri 窗口默认开着 `dragDropEnabled`：系统拖进来的文件被原生层截走，
 * 变成 `onDragDropEvent`（给的是**本地路径**）。网页这一侧的 HTML5 `drop`
 * 要么不触发，要么 `dataTransfer.files` 是空的 —— 不报错，只是拖了没反应。
 * 用户 2026-09-26 在角色工作台的「上传图片（点选或拖进来）」上撞到。
 *
 * # 为什么不干脆关掉 `dragDropEnabled`
 *
 * 聊天栏（`CommandBar`）靠原生那条拿到**本地路径**和文档正文；
 * 关掉就退回 HTML5 的 `File`，路径没了。所以保留原生拖放，**按落点分发**：
 * 拖放区挂 `data-native-drop`，落在它上面的归它，别处照旧归聊天栏
 * （聊天栏用 `dropZoneAt` 让路）。
 */
export const NATIVE_DROP_ATTR = "data-native-drop";

/**
 * Tauri 给的落点是**物理像素**（相对 webview），`elementFromPoint` 要**逻辑像素**。
 * Retina 上两者差一倍 —— 不换算的话落点会偏到右下方，而且不报错。
 */
export function toLogical(p: { x: number; y: number }, dpr: number): { x: number; y: number } {
  const r = dpr > 0 ? dpr : 1;
  return { x: p.x / r, y: p.y / r };
}

/** 这个落点在哪个拖放区上。没有 = `null`（聊天栏据此判断要不要收）。 */
export function dropZoneAt(x: number, y: number, doc: Document = document): Element | null {
  return doc.elementFromPoint(x, y)?.closest(`[${NATIVE_DROP_ATTR}]`) ?? null;
}

/** 只认这几种图 —— 和 `uploadImage.ts` 的 `ACCEPTED_TYPES` 同一份（SVG / HEIC 进出片链路会静默出错）。 */
export function isAcceptedImagePath(path: string): boolean {
  return /\.(png|jpe?g|webp)$/i.test(path);
}

type Handlers = {
  /** 拖着经过时，当前落点所在的、归自己的那个区（不在自己的区上 = `null`）。 */
  onOver?: (zone: Element | null) => void;
  onDrop: (zone: Element, paths: string[]) => void;
};

/**
 * 监听原生拖放，只处理落在「自己的」拖放区上的。
 *
 * `owns(zone)` 判这个区归不归自己 —— 单个上传框就是 `zone === ref.current`，
 * 一排槽位就是 `container.contains(zone)`，再从 `data-slot` 读是第几个。
 */
export function useNativeFileDrop(owns: (zone: Element) => boolean, handlers: Handlers, enabled = true) {
  // 回调放 ref 里：每次渲染都是新函数，放进依赖会反复重新订阅，
  // 而订阅是异步的 —— 快速重订阅之间漏掉的那次 drop 就没了
  const h = useRef({ owns, ...handlers });
  h.current = { owns, ...handlers };

  useEffect(() => {
    if (!enabled) return;
    let unlisten: (() => void) | undefined;
    let alive = true;
    const mine = (position: { x: number; y: number }) => {
      const p = toLogical(position, window.devicePixelRatio);
      const zone = dropZoneAt(p.x, p.y);
      return zone && h.current.owns(zone) ? zone : null;
    };
    (async () => {
      try {
        const { getCurrentWebview } = await import("@tauri-apps/api/webview");
        const off = await getCurrentWebview().onDragDropEvent((event) => {
          const e = event.payload;
          if (e.type === "enter" || e.type === "over") {
            h.current.onOver?.(mine(e.position));
          } else if (e.type === "drop") {
            h.current.onOver?.(null);
            const zone = mine(e.position);
            if (zone && e.paths.length > 0) h.current.onDrop(zone, e.paths);
          } else {
            h.current.onOver?.(null);
          }
        });
        if (alive) unlisten = off;
        else off();
      } catch {
        // 非 Tauri 环境（浏览器调试页）没有原生拖放，静默降级
      }
    })();
    return () => {
      alive = false;
      unlisten?.();
    };
  }, [enabled]);
}
