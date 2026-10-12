/**
 * 「Z-Image 常挂 LoRA」那几行的纯逻辑。
 *
 * 2026-09-17 一次性暴露了三个连环 bug，抽出来是为了能测：
 *
 * 1. **选项标签把整段说明拼进去了**（`${名字} — ${说明}`）。说明原本是一句话，
 *    那天改成带实测数据和出处的长文之后，`<option>` 直接撑爆。
 * 2. 撑爆之后 `flex-1` 的 `<select>` 把强度输入框和「移除」按钮**挤出了可视区** ——
 *    用户报的是「删不了」，其实按钮一直在，只是看不见。**bug 1 导致了 bug 2。**
 * 3. **配置里的 LoRA 被删掉之后，界面不说话。** `<select>` 的 value 对不上任何
 *    `<option>`，浏览器就显示第一个选项 —— 于是**界面显示的和实际存的不是一回事**，
 *    而用户完全看不出来。这条最阴：他以为常挂的是 A，实际配置里是个已经不存在的 B。
 */

/** 选项上显示什么。**只给名字**，说明进 `title`。 */
export function optionLabel(name: string): string {
  return name;
}

export interface RowOption {
  value: string;
  label: string;
  /** 这条在盘上已经没有了 —— 配置里的残留 */
  missing?: boolean;
}

/**
 * 一行下拉该有哪些选项。
 *
 * **当前值不在可用列表里时，补一条「已不存在」的选项并选中它** ——
 * 否则浏览器会静默显示第一个选项，界面就在骗人。
 */
export function rowOptions(available: string[], current: string): RowOption[] {
  const opts: RowOption[] = available.map((n) => ({ value: n, label: optionLabel(n) }));
  if (current && !available.includes(current)) {
    opts.unshift({
      value: current,
      label: `⚠️ ${current}（文件已不存在，点「移除」清掉）`,
      missing: true,
    });
  }
  return opts;
}

/** 这条配置是不是指向一个已经没有的文件。 */
export function isMissing(available: string[], current: string): boolean {
  return !!current && !available.includes(current);
}
