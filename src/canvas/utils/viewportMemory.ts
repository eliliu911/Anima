import { getStorage } from "../store/useCanvasStore";

/**
 * **画布的位置和缩放，按工程各记一份。**（用户 2026-10-13：「没有记录当前选择的工程，画布位置，画布缩放等，
 * 每次关闭打开都要重新设置一遍」）—— 以前 `defaultViewport` 写死 `{0, 0, 1}`，从来不存。
 *
 * 存在本机（和节点图同一个本地存储）；读不到 / 坏了就当没存过，不报错。
 */
export type Viewport = { x: number; y: number; zoom: number };

export const viewportKey = (projectId: string | null) => `anima:canvas:viewport:${projectId ?? "__global__"}`;

export function loadViewport(projectId: string | null): Viewport | null {
  try {
    const raw = getStorage()?.getItem(viewportKey(projectId));
    if (!raw) return null;
    const v = JSON.parse(raw);
    const ok = [v?.x, v?.y, v?.zoom].every((n) => typeof n === "number" && Number.isFinite(n)) && v.zoom > 0;
    return ok ? { x: v.x, y: v.y, zoom: v.zoom } : null;
  } catch {
    return null;
  }
}

export function saveViewport(projectId: string | null, v: Viewport) {
  try {
    getStorage()?.setItem(viewportKey(projectId), JSON.stringify({ x: v.x, y: v.y, zoom: v.zoom }));
  } catch {
    /* 存不了就只在这次会话里有效 */
  }
}
