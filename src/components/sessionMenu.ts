/// 会话行上那个 ⋯ 菜单的命中判定。
///
/// ## 为什么要单独拎出来
///
/// 「点删除完全没反应」这个 bug 的根因不在删除逻辑里，而在**关闭菜单的时机**：
///
/// 关菜单的监听挂在 `document` 的 `mousedown` 上，而 `mousedown` 比 `click` 早。
/// 按下"删除"的一瞬间：
///
/// 1. `mousedown` 冒到 document → 菜单关闭 → React 把删除按钮**卸载**
/// 2. `mouseup` 落到空处
/// 3. `click` **永远不触发** —— 元素在 down 和 up 之间消失了
///
/// 所以请求压根没发出去，后端一行日志都没有。这类 bug 最难查的地方在于
/// 它看起来像"后端删除坏了"，而后端其实从没被叫到。
///
/// 修法是：点在菜单内部时不关，交给按钮自己的 `onClick` 去关。
/// 判定拎成纯函数是为了能测 —— 这个项目的 vitest 没有 jsdom，
/// 直接测组件跑不起来。

/// 菜单容器的 class 名。**改样式时要连这里一起改** ——
/// 对不上的话菜单会在 click 之前被关掉，又回到"点了没反应"
export const MENU_CLASS = "sessionmenu";

/// 事件目标在菜单里面吗。
///
/// 参数按最小结构取（只要求有 `closest`），这样测试不需要真实 DOM。
/// document / window 这类没有 `closest` 的目标一律算"在外面"
export function isInsideMenu(target: unknown): boolean {
  const el = target as { closest?: (selector: string) => unknown } | null | undefined;
  if (!el || typeof el.closest !== "function") return false;
  return el.closest(`.${MENU_CLASS}`) != null;
}
