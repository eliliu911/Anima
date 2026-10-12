/**
 * **去掉系统默认的右键菜单。**
 *
 * WKWebView 在图片上右键会弹「Open Image in New Window / Download Image /
 * Copy Image / Copy Subject / Share…」—— 在 Tauri 窗口里**一个都不管用**
 * （没有「新窗口」、下载没有落点、分享面板接不到 webview 里的资源）。
 * 用户 2026-09-26：「系统默认菜单全都无效，可以尝试去掉。」
 *
 * **保留的两种情况**，因为那里的菜单是真的有用的：
 *
 * - 输入框 / 可编辑区域：复制、粘贴、拼写检查
 * - 选中了文字：复制
 *
 * 自己画了右键菜单的地方（导演世界的 3D 画布）照旧自己 `preventDefault`，
 * 这里不影响它们。
 */
const EDITABLE = 'input, textarea, [contenteditable=""], [contenteditable="true"], [contenteditable="plaintext-only"]';

/** 这次右键要不要放行系统菜单。只依赖 `closest`，好测。 */
export function keepsNativeMenu(
  target: { closest(selector: string): unknown } | null,
  selectedText: string,
): boolean {
  if (selectedText.trim().length > 0) return true;
  return !!target?.closest(EDITABLE);
}

/**
 * 装到 `document` 上。**开发模式不装** —— 那里右键「检查元素」是调试的主入口。
 * 返回卸载函数。
 */
export function installContextMenuGuard(doc: Document = document, dev = import.meta.env.DEV): () => void {
  if (dev) return () => {};
  const onMenu = (e: MouseEvent) => {
    const t = e.target instanceof Element ? e.target : null;
    if (!keepsNativeMenu(t, doc.getSelection()?.toString() ?? "")) e.preventDefault();
  };
  doc.addEventListener("contextmenu", onMenu);
  return () => doc.removeEventListener("contextmenu", onMenu);
}
