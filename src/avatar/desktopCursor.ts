/**
 * 角色层用的光标位置。
 *
 * ## 为什么不能用 pointermove
 *
 * 角色层是个**鼠标穿透**的窗口。穿透状态下浏览器收不到任何指针事件 ——
 * 于是所有靠 `pointermove` 的东西都会失效：
 *
 * - 穿透状态机判不出"光标压在她身上"，就永远退不出穿透（死循环，
 *   表现是"鼠标怎么都定位不到她"）
 * - 视线跟随收不到目标，眼睛永远不动
 *
 * 所以位置只能从系统层面轮询（`@tauri-apps/api/window` 的 `cursorPosition`），
 * 由角色层窗口统一喂进来，其它地方从这里读。
 *
 * ## 为什么是模块级单例
 *
 * 读它的是渲染循环（每帧），走 React state 会引发重渲染。这里只是一份
 * 每 33ms 更新一次的坐标，用最朴素的方式共享就好。
 */

/** 光标相对角色层内容区的位置（CSS 像素）。`null` = 不在窗口范围内 */
let current: { x: number; y: number } | null = null;
/** 有没有人在喂。没有的话读取方应该退回 `pointermove` */
let feeding = false;

export function feedDesktopCursor(pos: { x: number; y: number } | null) {
  feeding = true;
  current = pos;
}

export function stopFeedingDesktopCursor() {
  feeding = false;
  current = null;
}

/** 有没有人在轮询系统光标。没有就说明这不是角色层窗口 */
export function hasDesktopCursor(): boolean {
  return feeding;
}

export function desktopCursor(): { x: number; y: number } | null {
  return current;
}
