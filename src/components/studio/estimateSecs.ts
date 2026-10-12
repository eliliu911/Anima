/**
 * 「预计 ~N 秒」这个数。
 *
 * # 为什么跟着画幅走
 *
 * 这一栏原来读的是「画质档位」那个选择器 —— 而那个选择器 2026-09-12 拆了：
 * 它选的值从来没发给过工具，三档里有两档其实就是两个画幅
 * （H3 的 640×384 / 768×512），第三档实测画质看不出更好。
 * 理由见 DECISIONS.md。
 *
 * 拆掉时唯一有价值的东西是那个秒数 —— **代价要在点下去之前就看得见**。
 * 所以它搬到了画幅上：H3 的档位本来就是按长边推出来的，
 * 跟着画幅走才是实话。
 *
 * 没有实测值时退回引擎的基准值按时长缩放（基准是 5 秒那一档）。
 * 出图没有时长，直接用基准值。
 */
export function estimateSecs(
  engine: { secs: number; sizes: { width: number; height: number; secs: number | null }[] },
  size: { width: number; height: number } | null,
  duration: number | null,
  batch: number,
): number {
  const sized = size
    ? engine.sizes.find((s) => s.width === size.width && s.height === size.height)?.secs
    : null;
  const per = sized ?? (duration ? Math.round((engine.secs * duration) / 5) : engine.secs);
  return per * Math.max(1, batch);
}
