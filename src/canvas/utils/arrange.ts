import type { CanvasEdge, CanvasNode } from "../types";
import { nodeSize } from "../group";

/**
 * 「自动整理画布」的三种排法。
 *
 * - `horizontal` 横向：按连线分层，上游在左、下游在右，同一层竖着排
 * - `vertical` 纵向：同样分层，上游在上、下游在下，同一层横着排
 * - `grid` 网格：不看连线，按现在的先后顺序（从上到下、从左到右）码成接近正方形的网格
 *
 * # 为什么要按节点的真实尺寸排
 *
 * 以前固定「每列 520、每行 480」—— 视频节点六七百高，一整理就叠在一起，
 * 步骤说明压在下一个节点的标题上（2026-09-24 用户截图）。
 * 现在每层的宽 / 高取这一层里最大的那个节点，节点之间统一留 `GAP`。
 *
 * # 同一层里谁先谁后
 *
 * 按上游在前一层里的平均位置排（重心法，一遍）。不排的话同一层按节点数组顺序，
 * 连线互相交叉，「看起来乱」多半是这个。
 */
export type ArrangeMode = "horizontal" | "vertical" | "grid";

export const ARRANGE_MODES: { id: ArrangeMode; label: string; hint: string }[] = [
  { id: "horizontal", label: "横向", hint: "按连线从左到右：上游在左，下游在右" },
  { id: "vertical", label: "纵向", hint: "按连线从上到下：上游在上，下游在下" },
  { id: "grid", label: "网格", hint: "不看连线，按现在的先后顺序码成整齐的网格" },
];

export const GAP = 80;
const ORIGIN = { x: 100, y: 120 };

type Pos = { x: number; y: number };

/** 分层：没有上游的在第 0 层，其余 = 最长上游链的长度。有环时层数封顶，不会死循环。 */
export function ranks(nodes: CanvasNode[], edges: CanvasEdge[]): Map<string, number> {
  const ids = new Set(nodes.map((n) => n.id));
  const incoming = new Map<string, string[]>();
  nodes.forEach((n) => incoming.set(n.id, []));
  edges.forEach((e) => {
    if (ids.has(e.source) && ids.has(e.target) && e.source !== e.target) incoming.get(e.target)!.push(e.source);
  });
  const rank = new Map<string, number>();
  const cap = nodes.length;
  // 反复松弛：每轮每个节点取「上游最大层 + 1」，没有变化就停；有环时最多跑 cap 轮
  nodes.forEach((n) => rank.set(n.id, 0));
  for (let round = 0; round < cap; round++) {
    let changed = false;
    for (const n of nodes) {
      const ups = incoming.get(n.id)!;
      const r = ups.length ? Math.min(cap, Math.max(...ups.map((u) => rank.get(u)! + 1))) : 0;
      if (r !== rank.get(n.id)) { rank.set(n.id, r); changed = true; }
    }
    if (!changed) break;
  }
  return rank;
}

/** 算每个节点的新位置。纯函数：不改入参，只返回 id → 左上角坐标。 */
export function arrangePositions(nodes: CanvasNode[], edges: CanvasEdge[], mode: ArrangeMode): Map<string, Pos> {
  const out = new Map<string, Pos>();
  if (nodes.length === 0) return out;
  const size = new Map(nodes.map((n) => [n.id, nodeSize(n)]));

  if (mode === "grid") {
    // 先后顺序 = 现在的位置（先上下后左右），整理完大致还在原来那一片
    const order = [...nodes].sort((a, b) => a.position.y - b.position.y || a.position.x - b.position.x);
    const cols = Math.max(1, Math.ceil(Math.sqrt(order.length)));
    const colW: number[] = [];
    const rowH: number[] = [];
    order.forEach((n, i) => {
      const { width, height } = size.get(n.id)!;
      const c = i % cols, r = Math.floor(i / cols);
      colW[c] = Math.max(colW[c] ?? 0, width);
      rowH[r] = Math.max(rowH[r] ?? 0, height);
    });
    const colX = colW.map((_, c) => ORIGIN.x + colW.slice(0, c).reduce((s, w) => s + w + GAP, 0));
    const rowY = rowH.map((_, r) => ORIGIN.y + rowH.slice(0, r).reduce((s, h) => s + h + GAP, 0));
    order.forEach((n, i) => out.set(n.id, { x: colX[i % cols], y: rowY[Math.floor(i / cols)] }));
    return out;
  }

  const rank = ranks(nodes, edges);
  const layers: CanvasNode[][] = [];
  nodes.forEach((n) => {
    const r = rank.get(n.id)!;
    (layers[r] ??= []).push(n);
  });
  const dense = layers.filter(Boolean);

  // 第 0 层按现在的位置排（保留用户原来的上下 / 左右关系），后面每层按上游的平均序号排
  const along = (n: CanvasNode) => (mode === "horizontal" ? n.position.y : n.position.x);
  dense[0].sort((a, b) => along(a) - along(b));
  const index = new Map<string, number>();
  dense[0].forEach((n, i) => index.set(n.id, i));
  for (let l = 1; l < dense.length; l++) {
    const bary = new Map<string, number>();
    dense[l].forEach((n) => {
      const ups = edges.filter((e) => e.target === n.id && index.has(e.source)).map((e) => index.get(e.source)!);
      bary.set(n.id, ups.length ? ups.reduce((s, v) => s + v, 0) / ups.length : Number.POSITIVE_INFINITY);
    });
    dense[l].sort((a, b) => bary.get(a.id)! - bary.get(b.id)! || along(a) - along(b));
    dense[l].forEach((n, i) => index.set(n.id, i));
  }

  // 横向：每层一列，列宽 = 这一层最宽的节点；纵向反过来
  let cross = mode === "horizontal" ? ORIGIN.x : ORIGIN.y;
  for (const layer of dense) {
    let main = mode === "horizontal" ? ORIGIN.y : ORIGIN.x;
    let thickness = 0;
    for (const n of layer) {
      const { width, height } = size.get(n.id)!;
      out.set(n.id, mode === "horizontal" ? { x: cross, y: main } : { x: main, y: cross });
      main += (mode === "horizontal" ? height : width) + GAP;
      thickness = Math.max(thickness, mode === "horizontal" ? width : height);
    }
    cross += thickness + GAP * 1.5;
  }
  return out;
}

/** 组和组之间的间距（比节点之间的大，一眼看出是两套） */
export const COMPONENT_GAP = GAP * 3;

/**
 * 按连线分成互不相连的几组（无向）。没有任何连线的散节点归成一组「散件」，排在最后。
 * 组的顺序 = 现在的位置（组里最靠上的那个，再看最靠左）—— 整理完上下关系不变。
 */
export function components(nodes: CanvasNode[], edges: CanvasEdge[]): CanvasNode[][] {
  const ids = new Set(nodes.map((n) => n.id));
  const parent = new Map(nodes.map((n) => [n.id, n.id]));
  const find = (x: string): string => {
    let r = x;
    while (parent.get(r) !== r) r = parent.get(r)!;
    parent.set(x, r);
    return r;
  };
  const linked = new Set<string>();
  edges.forEach((e) => {
    if (!ids.has(e.source) || !ids.has(e.target) || e.source === e.target) return;
    linked.add(e.source); linked.add(e.target);
    parent.set(find(e.source), find(e.target));
  });
  const groups = new Map<string, CanvasNode[]>();
  const loose: CanvasNode[] = [];
  nodes.forEach((n) => {
    if (!linked.has(n.id)) { loose.push(n); return; }
    const r = find(n.id);
    (groups.get(r) ?? groups.set(r, []).get(r)!).push(n);
  });
  const key = (g: CanvasNode[]) => {
    const top = Math.min(...g.map((n) => n.position.y));
    const left = Math.min(...g.map((n) => n.position.x));
    return [top, left] as const;
  };
  const sorted = [...groups.values()].sort((a, b) => key(a)[0] - key(b)[0] || key(a)[1] - key(b)[1]);
  return loose.length ? [...sorted, loose] : sorted;
}

/**
 * **整张画布的整理：每套工作流各排各的。**（用户 2026-10-13：「当有多组工作流的时候，不同工作流里的节点会混在一起」）
 *
 * 以前整张画布当一张图分层 —— 两套工作流的第 0 层（参考图）进同一列、第 1 层（视频）进同一列，交错在一起。
 * 现在先按连线分组（[`components`]），每组用 [`arrangePositions`] 排好，
 * 横向排法的组从上往下摞、纵向排法的组从左往右摆，组间留 [`COMPONENT_GAP`]。
 */
export function arrangeAll(nodes: CanvasNode[], edges: CanvasEdge[], mode: ArrangeMode): Map<string, Pos> {
  const out = new Map<string, Pos>();
  const size = new Map(nodes.map((n) => [n.id, nodeSize(n)]));
  // 横向：组往下摞（offset 是 y）；纵向：组往右摆（offset 是 x）；网格：往下摞
  const stackDown = mode !== "vertical";
  let offset = stackDown ? ORIGIN.y : ORIGIN.x;
  for (const group of components(nodes, edges)) {
    const pos = arrangePositions(group, edges, mode);
    let lo = Infinity, hi = -Infinity;
    pos.forEach((p, id) => {
      const s = size.get(id)!;
      lo = Math.min(lo, stackDown ? p.y : p.x);
      hi = Math.max(hi, stackDown ? p.y + s.height : p.x + s.width);
    });
    pos.forEach((p, id) => out.set(id, stackDown ? { x: p.x, y: p.y - lo + offset } : { x: p.x - lo + offset, y: p.y }));
    offset += hi - lo + COMPONENT_GAP;
  }
  return out;
}
