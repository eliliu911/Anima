/**
 * 组装页「分集封面」那一块的纯逻辑。
 *
 * # 「设置」和「动作」是两件事，2026-09-18 拆开了
 *
 * 原来只有一个勾选框，**勾上就立刻去改已有的成片**。用户的原话是
 * 「已勾选就会执行，这就很奇怪」——他说得对：勾选框是「设置」的语义，
 * 一个几秒钟、会改动产物的操作不该长成勾选框。
 *
 * 现在分成两个，各在各的位置：
 *
 * | 控件 | 语义 | 在哪 |
 * |---|---|---|
 * | ☑ 合成时加片头封面 | 对**下一次合成**的设置，不碰现有成片 | 顶部操作条 |
 * | [加上片头] / [移除片头] | 对**已有成片**的动作 | 成片卡片上 |
 *
 * 两者看起来管同一个结果，但作用对象不同：一个管未来，一个管现在。
 * 合成完之后按钮的文案会自然反映出结果，不会互相打架。
 */

/** 生成按钮上写什么。已经有封面时是「重新生成」—— 点下去会覆盖，得说清楚 */
export function coverButtonLabel(hasCover: boolean, busy: boolean): string {
  if (busy) return "出封面中…";
  return hasCover ? "重新生成封面" : "生成封面";
}

/**
 * 勾选框现在应该是什么状态。
 *
 * **没有封面时强制回 false**，不是只把框禁掉：禁掉的框仍然可以留着
 * `checked=true` 的旧值，然后被原样发给后端。
 */
export function syncIncludeCover(hasCover: boolean, checked: boolean): boolean {
  return hasCover && checked;
}

/** 勾选框旁边的说明。没有封面时要说清楚为什么点不了 */
export function coverHint(hasCover: boolean, secs: number): string {
  return hasCover
    ? `下次合成时把封面接在片头，停留约 ${secs} 秒（不影响现在这个成片）`
    : "先生成封面才能加进片头";
}

/**
 * 成片卡上那个动作按钮的文案。**说的是「点下去会发生什么」**，
 * 不是「当前是什么状态」—— 后者会让人分不清点了是加还是减。
 */
export function introActionLabel(hasIntro: boolean, busy: boolean): string {
  if (busy) return "处理中…";
  return hasIntro ? "移除片头" : "加上片头";
}

/** 成片现在带不带片头，给人看的一句话 */
export function introStateLabel(hasIntro: boolean): string {
  return hasIntro ? "带片头封面" : "无片头";
}

/**
 * 动作按钮能不能点。
 *
 * **没有封面时连「加上」都不该亮** —— 点下去必定失败。
 * 但「移除」在没有封面时仍然要能点：封面可能是刚被删掉的，
 * 而成片里那段片头还在。
 */
export function canToggleIntro(hasCover: boolean, hasIntro: boolean, busy: boolean): boolean {
  if (busy) return false;
  return hasIntro || hasCover;
}
