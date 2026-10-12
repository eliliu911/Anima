/**
 * 节点的模板说明（`data.guide`）。
 *
 * 和 `notes` 分开：`notes` 是**运行提示**，执行器每次开跑先清空再写；
 * `guide` 是写模板的人留给用户的「第几步、做什么、怎么验收」，运行不碰、
 * 另存模板时保留（`workflowTemplate.ts` 的产物清单里没有它）。
 *
 * 模板是 JSON 文件，也可能是代理写的 —— 类型不对的一律当没有，不让卡片崩。
 */
export function nodeGuide(data: unknown): string[] {
  const g = (data as { guide?: unknown } | undefined)?.guide;
  if (!Array.isArray(g)) return [];
  return g.filter((x): x is string => typeof x === "string" && x.trim().length > 0);
}
