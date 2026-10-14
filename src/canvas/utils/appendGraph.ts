import type { CanvasEdge, CanvasNode } from "../types";

/** 估一个节点占多宽：量过就用量过的，没量过按常见卡片宽 */
const widthOf = (n: CanvasNode) => (n as { measured?: { width?: number } }).measured?.width ?? (n as { width?: number }).width ?? 420;

/**
 * **把一组节点追加到画布上，不动已有的。**（用户 2026-10-13：「点击载入，原本画布的内容会被清空，
 * 这不合理。我们是无限画布，不是单工作流画布。」）
 *
 * - 位置：整组平移到现有内容的右边（隔 `gap`），顶边和现有内容对齐；画布是空的就原样放
 * - id：和已有节点撞了就换一个（`原id-2`、`-3`……），连线的两端和 id 跟着换 ——
 *   同一份工作流载两次、或者模板里的 id 碰巧和画布上的一样时，不会串线到别人的节点上
 */
export function appendGraph(
  existing: CanvasNode[],
  existingEdges: CanvasEdge[],
  nodes: CanvasNode[],
  edges: CanvasEdge[],
  gap = 240,
): { nodes: CanvasNode[]; edges: CanvasEdge[]; addedIds: string[] } {
  const taken = new Set(existing.map((n) => n.id));
  const rename = new Map<string, string>();
  for (const n of nodes) {
    let id = n.id;
    for (let k = 2; taken.has(id); k++) id = `${n.id}-${k}`;
    taken.add(id);
    rename.set(n.id, id);
  }

  let dx = 0;
  let dy = 0;
  if (existing.length && nodes.length) {
    const right = Math.max(...existing.map((n) => n.position.x + widthOf(n)));
    const top = Math.min(...existing.map((n) => n.position.y));
    const left = Math.min(...nodes.map((n) => n.position.x));
    const ntop = Math.min(...nodes.map((n) => n.position.y));
    dx = right + gap - left;
    dy = top - ntop;
  }

  const placed = nodes.map((n) => ({
    ...n,
    id: rename.get(n.id)!,
    position: { x: n.position.x + dx, y: n.position.y + dy },
    // 编组里的子节点：父节点也跟着换名（子节点的位置是相对父节点的，不再平移）
    ...((n as { parentId?: string }).parentId
      ? { parentId: rename.get((n as { parentId?: string }).parentId!) ?? (n as { parentId?: string }).parentId, position: n.position }
      : {}),
  })) as CanvasNode[];

  const edgeIds = new Set(existingEdges.map((e) => e.id));
  const placedEdges = edges.map((e) => {
    const source = rename.get(e.source) ?? e.source;
    const target = rename.get(e.target) ?? e.target;
    let id = source === e.source && target === e.target ? e.id : `${source}>${target}`;
    for (let k = 2; edgeIds.has(id); k++) id = `${e.id}-${k}`;
    edgeIds.add(id);
    return { ...e, id, source, target };
  });

  return { nodes: [...existing, ...placed], edges: [...existingEdges, ...placedEdges], addedIds: placed.map((n) => n.id) };
}
