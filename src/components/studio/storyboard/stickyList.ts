/**
 * 分镜页左边的镜头列表**自己滚**，不跟着右边的详情一起走（用户 2026-09-29）。
 *
 * 2026-10-06 起整页正好一屏高、顶上的步骤卡片固定，列表和详情是网格里各自滚的两栏
 * （用户：「顶部 4 步骤卡片不参与下拉页面」）。以前是列表 `sticky` 贴顶、高度按整个滚动区量 ——
 * 页面没滚下去时列表底部被卡片顶出屏幕，右边详情短的话永远拉不到底。量高度那一套随之删了。
 */

/** 选中的那一镜不在列表可见范围里就滚过去（从队列跳过来的时候） */
export function revealSelected(list: HTMLElement | null, shotId: string | null) {
  if (!list || !shotId) return;
  const item = list.querySelector<HTMLElement>(`[data-shot-id="${CSS.escape(shotId)}"]`);
  item?.scrollIntoView({ block: "nearest" });
}
