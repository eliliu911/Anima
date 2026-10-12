/**
 * 关掉一个「只给了 toggle」的菜单。
 *
 * # 为什么这么小的东西要单开一个文件
 *
 * 因为它错过一次，而且**错的样子是「点了没反应」**：
 *
 * 添加节点面板选完一项会调 `onSelectNode`（上层在里面关了菜单），
 * 然后**又调一次** `onClose`。上层给的两个回调是同一个 toggle ——
 * 调两次等于没调，菜单纹丝不动。用户报的原话是
 * 「当点击添加了某个节点，这个添加节点工具菜单要自动关闭」。
 *
 * 代码里其实已经有一处写对了（`if (isAddMenuOpen) toggleAddMenu()`），
 * 只是另一处没有。一个只在某些地方遵守的约定等于没有约定，
 * 所以把它变成一个有测试钉着的函数。
 *
 * 想关菜单的地方一律走这里，**不要直接调 toggle**。
 */
export function closeWithToggle(isOpen: boolean, toggle: () => void): void {
  if (isOpen) toggle();
}
