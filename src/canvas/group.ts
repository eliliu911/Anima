/**
 * 画布的「组」—— ⌘G 成组、⇧⌘G 解组、组框自动包住成员。
 *
 * 改写自 DramaClaw（ClaymoreLab, Elastic License 2.0）：
 *   frontend/src/stores/canvasStore.ts  → groupNodes / fitGroupToChildren / ungroupNode
 *                                          / resolveAbsolutePosition / getNodeSize
 *                                          / addPanoCaptureGroup（截图排成网格再成组）
 *   frontend/src/features/canvas/domain/groupColors.ts → 组配色
 * 只搬了「普通组」这一种；DC 的分镜组（宫格排版、缩略图重排）、主线投影组没搬。
 * 这里是纯函数版本，store 负责撤销栈和自动保存。出处规则见 DECISIONS.md 2026-09-16。
 *
 * # 成员坐标是相对组框的
 *
 * ReactFlow 的子节点 `position` 是**相对父节点左上角**的。所以：
 * - 成组时把成员的绝对坐标减去组框位置
 * - 解组时加回去
 * - 「整理画布」之类按绝对坐标排版的逻辑，只能动顶层节点
 * - 组节点在数组里**必须排在成员前面**，ReactFlow 才能先算出父节点
 *
 * # 成员可以自由拖出框（libtv 式）
 *
 * 成员不设 `extent: "parent"`，不被钳在框里；拖完之后 `fitGroup` 把框撑大包住。
 * 只撑大不缩小 —— 用户手动拉大的框不会被收回去。
 */
import type { CanvasEdge, CanvasNode } from "./types";

/** 组框留白。顶部多留一截给浮在框上沿的标题。 */
export const GROUP_SIDE_PAD = 20;
export const GROUP_TOP_PAD = 34;
export const GROUP_BOTTOM_PAD = 20;
const GROUP_MIN_WIDTH = 220;
const GROUP_MIN_HEIGHT = 140;

/** 没测量到尺寸时的兜底宽高。和 `CanvasView.handleAddNode` 的估算保持一致。 */
const FALLBACK_SIZE: Record<string, { width: number; height: number }> = {
  imageGen: { width: 440, height: 380 },
  videoGen: { width: 440, height: 380 },
  shot: { width: 440, height: 240 },
  shotContext: { width: 440, height: 240 },
  skill: { width: 440, height: 240 },
  audioClip: { width: 320, height: 190 },
  panorama: { width: 440, height: 240 },
  panoViewer: { width: 864, height: 420 },
  directorWorld: { width: 864, height: 460 },
};

/** 纯图片节点（`referenceOnly`）：400 宽，标题 + 260 高的图。见 `ImageGenNode` */
export const IMAGE_CELL = { width: 400, height: 304 };

export function nodeSize(node: CanvasNode): { width: number; height: number } {
  const fb = (node.data as { referenceOnly?: unknown } | undefined)?.referenceOnly
    ? IMAGE_CELL
    : FALLBACK_SIZE[node.type ?? ""] ?? { width: 320, height: 240 };
  const style = node.style as { width?: unknown; height?: unknown } | undefined;
  const pick = (measured: unknown, explicit: unknown, styled: unknown, fallback: number) =>
    typeof measured === "number" ? measured
      : typeof explicit === "number" ? explicit
        : typeof styled === "number" ? styled
          : fallback;
  return {
    width: pick(node.measured?.width, node.width, style?.width, fb.width),
    height: pick(node.measured?.height, node.height, style?.height, fb.height),
  };
}

/** 顺着 `parentId` 链把相对坐标累加成画布绝对坐标。防环。 */
export function absolutePosition(node: CanvasNode, byId: Map<string, CanvasNode>): { x: number; y: number } {
  let { x, y } = node.position;
  let parent = node.parentId;
  const seen = new Set<string>();
  while (parent && !seen.has(parent)) {
    seen.add(parent);
    const p = byId.get(parent);
    if (!p) break;
    x += p.position.x;
    y += p.position.y;
    parent = p.parentId;
  }
  return { x, y };
}

/** 一个节点的全部后代（成员、成员的成员……）。 */
export function descendantIds(nodes: CanvasNode[], rootIds: Iterable<string>): Set<string> {
  const out = new Set<string>();
  const queue = [...rootIds];
  while (queue.length) {
    const id = queue.shift()!;
    for (const n of nodes) {
      if (n.parentId === id && !out.has(n.id)) {
        out.add(n.id);
        queue.push(n.id);
      }
    }
  }
  return out;
}

export interface GroupResult {
  nodes: CanvasNode[];
  groupId: string;
}

/**
 * 把选中的节点打成一个组。少于两个返回 `null`。
 *
 * 选中的节点里如果有「另一个选中节点的后代」，它跟着祖先走，不单独算成员 ——
 * 否则同一个节点会被挂到两个父节点下。
 */
export function groupNodes(
  nodes: CanvasNode[],
  ids: string[],
  opts: { groupId: string; label?: string },
): GroupResult | null {
  const byId = new Map(nodes.map((n) => [n.id, n] as const));
  const picked = [...new Set(ids)].filter((id) => byId.has(id));
  if (picked.length < 2) return null;
  const pickedSet = new Set(picked);
  const members = picked.filter((id) => {
    let p = byId.get(id)?.parentId;
    const seen = new Set<string>();
    while (p && !seen.has(p)) {
      if (pickedSet.has(p)) return false;
      seen.add(p);
      p = byId.get(p)?.parentId;
    }
    return true;
  });
  if (members.length < 2) return null;

  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
  for (const id of members) {
    const n = byId.get(id)!;
    const abs = absolutePosition(n, byId);
    const s = nodeSize(n);
    minX = Math.min(minX, abs.x);
    minY = Math.min(minY, abs.y);
    maxX = Math.max(maxX, abs.x + s.width);
    maxY = Math.max(maxY, abs.y + s.height);
  }
  const gx = Math.round(minX - GROUP_SIDE_PAD);
  const gy = Math.round(minY - GROUP_TOP_PAD);
  const width = Math.round(Math.max(GROUP_MIN_WIDTH, maxX - minX + GROUP_SIDE_PAD * 2));
  const height = Math.round(Math.max(GROUP_MIN_HEIGHT, maxY - minY + GROUP_TOP_PAD + GROUP_BOTTOM_PAD));
  const existing = nodes.filter((n) => n.type === "group").length;

  const group = {
    id: opts.groupId,
    type: "group",
    position: { x: gx, y: gy },
    width,
    height,
    style: { width, height },
    selected: true,
    data: { label: opts.label?.trim() || `组 ${existing + 1}` },
  } as unknown as CanvasNode;

  const memberSet = new Set(members);
  const firstIndex = nodes.findIndex((n) => memberSet.has(n.id));
  const out: CanvasNode[] = [];
  nodes.forEach((n, i) => {
    // 组节点插在第一个成员之前 —— ReactFlow 要求父节点先于子节点出现
    if (i === firstIndex) out.push(group);
    if (memberSet.has(n.id)) {
      const abs = absolutePosition(n, byId);
      out.push({
        ...n,
        parentId: group.id,
        extent: undefined,
        position: { x: Math.round(abs.x - gx), y: Math.round(abs.y - gy) },
        selected: false,
      } as CanvasNode);
    } else {
      out.push({ ...n, selected: false } as CanvasNode);
    }
  });
  return { nodes: out, groupId: group.id };
}

/**
 * 解组：删掉组框，成员坐标换回绝对坐标。连在组框本身上的边一并删掉。
 * 不是组、或者组里没有成员，返回 `null`。
 */
export function ungroup(
  nodes: CanvasNode[],
  edges: CanvasEdge[],
  groupId: string,
): { nodes: CanvasNode[]; edges: CanvasEdge[] } | null {
  const group = nodes.find((n) => n.id === groupId && n.type === "group");
  if (!group) return null;
  const byId = new Map(nodes.map((n) => [n.id, n] as const));
  if (!nodes.some((n) => n.parentId === groupId)) return null;
  return {
    nodes: nodes
      .filter((n) => n.id !== groupId)
      .map((n) => {
        if (n.parentId !== groupId) return n;
        const abs = absolutePosition(n, byId);
        return {
          ...n,
          parentId: undefined,
          extent: undefined,
          position: { x: Math.round(abs.x), y: Math.round(abs.y) },
          selected: true,
        } as CanvasNode;
      }),
    edges: edges.filter((e) => e.source !== groupId && e.target !== groupId),
  };
}

/**
 * 让组框包住所有成员。**只撑大不缩小**；成员溢出到左 / 上边时，组框整体往左上挪、
 * 成员反向挪回来，看上去成员没动、框长出去了。
 *
 * 已经包住时返回**原数组**（同一个引用），调用方可以据此跳过 `set`。
 */
export function fitGroup(nodes: CanvasNode[], groupId: string): CanvasNode[] {
  const group = nodes.find((n) => n.id === groupId && n.type === "group");
  if (!group) return nodes;
  const children = nodes.filter((n) => n.parentId === groupId);
  if (!children.length) return nodes;

  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
  for (const c of children) {
    const s = nodeSize(c);
    minX = Math.min(minX, c.position.x);
    minY = Math.min(minY, c.position.y);
    maxX = Math.max(maxX, c.position.x + s.width);
    maxY = Math.max(maxY, c.position.y + s.height);
  }
  const shiftX = Math.max(0, Math.round(GROUP_SIDE_PAD - minX));
  const shiftY = Math.max(0, Math.round(GROUP_TOP_PAD - minY));
  const cur = nodeSize(group);
  const nextW = Math.max(cur.width, Math.round(maxX + shiftX + GROUP_SIDE_PAD));
  const nextH = Math.max(cur.height, Math.round(maxY + shiftY + GROUP_BOTTOM_PAD));
  if (!shiftX && !shiftY && nextW === cur.width && nextH === cur.height) return nodes;

  return nodes.map((n) => {
    if (n.id === groupId) {
      return {
        ...n,
        position: { x: n.position.x - shiftX, y: n.position.y - shiftY },
        // width/height 要和 style 同步：ReactFlow 渲染时显式 width 优先于 style.width，
        // 只改 style 视觉上不生效
        width: nextW,
        height: nextH,
        style: { ...(n.style ?? {}), width: nextW, height: nextH },
        // 测量值是旧尺寸，留着的话下一次 nodeSize 会读到旧值
        measured: undefined,
      } as CanvasNode;
    }
    if ((shiftX || shiftY) && n.parentId === groupId) {
      return { ...n, position: { x: n.position.x + shiftX, y: n.position.y + shiftY } } as CanvasNode;
    }
    return n;
  });
}

export interface ImageGroupItem {
  id: string;
  path: string;
  label: string;
  /** 合进图片节点 data 的额外字段，比如 `captureMetadata`（这张图是谁、怎么截出来的） */
  data?: Record<string, unknown>;
}

/** 截图网格离源节点多远、格子之间留多宽 */
const IMAGE_GROUP_OFFSET = 80;
const IMAGE_GROUP_GAP = 24;

/**
 * 把一批截图摆到源节点右边，**排成网格、打成一个组**。
 *
 * - 一张图：不成组（一个成员的组没意义），直接放一个图片节点
 * - 多张：按 `cols` 列排（读序），外面套一个组，组名就是 `label`
 *
 * **每张图都从源节点连一条线过来** —— 工作流上要看得出这些图是哪个节点截出来的。
 * 连线不会让下游重跑上游：执行队列追到全景查看器就停（`dagExecutor` 的 `MANUAL_CAPTURE_TYPES`）。
 *
 * 图片节点是 `imageGen` 的 `referenceOnly` 形态 —— 和资产库拖进来的素材一样，
 * 能直接当下游的参考图连出去。返回新组 id（一张图时返回那个图片节点 id）。
 */
export function layoutImageGroup(
  nodes: CanvasNode[],
  edges: CanvasEdge[],
  sourceId: string,
  items: ImageGroupItem[],
  opts: { cols: number; groupId: string; label: string; sourceHandle?: string },
): { nodes: CanvasNode[]; edges: CanvasEdge[]; focusId: string } | null {
  const source = nodes.find((n) => n.id === sourceId);
  if (!source || !items.length) return null;
  const byId = new Map(nodes.map((n) => [n.id, n] as const));
  const abs = absolutePosition(source, byId);
  const left = Math.round(abs.x + nodeSize(source).width + IMAGE_GROUP_OFFSET);
  const top = Math.round(abs.y);
  const cols = Math.max(1, Math.floor(opts.cols));

  const images = items.map((item, i) => ({
    id: item.id,
    type: "imageGen",
    position: {
      x: left + (i % cols) * (IMAGE_CELL.width + IMAGE_GROUP_GAP),
      y: top + Math.floor(i / cols) * (IMAGE_CELL.height + IMAGE_GROUP_GAP),
    },
    selected: false,
    data: {
      ...(item.data ?? {}),
      referenceOnly: true,
      label: item.label,
      outputImageUrl: item.path,
      outputImages: [item.path],
      references: [{ path: item.path, name: item.label }],
      prompt: "",
    },
  }) as unknown as CanvasNode);

  const nextEdges: CanvasEdge[] = [
    ...edges,
    ...images.map((img) => ({
      id: `e-${sourceId}-${img.id}`,
      source: sourceId,
      sourceHandle: opts.sourceHandle ?? null,
      target: img.id,
      targetHandle: "prompt-in",
      animated: true,
      // 和手动连线（`onConnect`）同一个样式
      style: { stroke: "rgba(176, 176, 183, 0.45)", strokeWidth: 2 },
    }) as CanvasEdge),
  ];

  const base = nodes.map((n) => (n.selected ? ({ ...n, selected: false } as CanvasNode) : n));
  if (images.length === 1) {
    const only = { ...images[0], selected: true } as CanvasNode;
    return { nodes: [...base, only], edges: nextEdges, focusId: only.id };
  }

  const r = groupNodes([...base, ...images], images.map((n) => n.id), {
    groupId: opts.groupId,
    label: opts.label,
  });
  if (!r) return null;
  return { nodes: r.nodes, edges: nextEdges, focusId: r.groupId };
}

/**
 * 拖动联动的另一方：导演世界节点 ↔ 它输出的「导演世界输出」组。
 *
 * 改写自 DramaClaw（ClaymoreLab, Elastic License 2.0）
 * frontend/src/features/canvas/Canvas.tsx 的 findLinkedCapturePartnerIds。
 *
 * 两者在数据上是**各自独立**的顶层节点（源节点不是组的成员），ReactFlow 不会一起拖。
 * 配对关系从图里推：组里带 `captureMetadata.source_node_id` 的成员，
 * 就是那个源节点输出的。**不另存一份配对表** —— 删了连线、换了成员都不会过期。
 *
 * 只在**两边都是顶层节点**时联动。被拖的节点本身在某个组里的话不联动：
 * 否则会把自己的父组当成另一方，拖一下移两次（DC 那边踩过：卡顿、重叠）。
 */
export function capturePartners(draggedId: string, nodes: CanvasNode[]): string[] {
  const byId = new Map(nodes.map((n) => [n.id, n] as const));
  const dragged = byId.get(draggedId);
  if (!dragged || dragged.parentId) return [];
  const sourceOf = (n: CanvasNode) =>
    ((n.data as { captureMetadata?: { source_node_id?: unknown } }).captureMetadata?.source_node_id as string | undefined) ??
    null;
  const out = new Set<string>();
  if (dragged.type === "group") {
    for (const child of nodes) {
      if (child.parentId !== draggedId) continue;
      const src = sourceOf(child);
      const s = src ? byId.get(src) : undefined;
      if (s && !s.parentId) out.add(s.id);
    }
  } else {
    for (const child of nodes) {
      if (sourceOf(child) !== draggedId || !child.parentId) continue;
      const g = byId.get(child.parentId);
      if (g && g.type === "group" && !g.parentId) out.add(g.id);
    }
  }
  out.delete(draggedId);
  return [...out];
}

/** 组配色（参考 libtv / DC）。存的是基础色，渲染时叠透明度。 */
export const GROUP_COLORS: { key: string; label: string; value: string }[] = [
  { key: "red", label: "红", value: "#ef4444" },
  { key: "orange", label: "橙", value: "#f97316" },
  { key: "yellow", label: "黄", value: "#eab308" },
  { key: "green", label: "绿", value: "#22c55e" },
  { key: "cyan", label: "青", value: "#06b6d4" },
  { key: "blue", label: "蓝", value: "#3b82f6" },
  { key: "purple", label: "紫", value: "#8b5cf6" },
  { key: "pink", label: "粉", value: "#ec4899" },
  { key: "gray", label: "灰", value: "#6b7280" },
];

export function groupBackground(color?: string | null): string | undefined {
  return color ? `${color}1f` : undefined; // ≈12% 底色
}

export function groupBorder(color?: string | null): string | undefined {
  return color ? `${color}66` : undefined; // ≈40% 描边
}
