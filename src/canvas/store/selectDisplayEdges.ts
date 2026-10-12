import type { CanvasEdge, CanvasNode } from "../types";

/** Runtime styling only; never persist running markers in workflow files. */
export function createDisplayEdgesSelector() {
  let lastNodes: CanvasNode[] | undefined;
  let lastEdges: CanvasEdge[] | undefined;
  let running = new Set<string>();
  let display: CanvasEdge[] = [];
  return ({ nodes, edges }: { nodes: CanvasNode[]; edges: CanvasEdge[] }): CanvasEdge[] => {
    let changed = false;
    if (nodes !== lastNodes) {
      lastNodes = nodes;
      const next = new Set(nodes.filter((n) => n.data.isGenerating).map((n) => n.id));
      changed = next.size !== running.size || [...next].some((id) => !running.has(id));
      running = next;
    }
    if (!changed && edges === lastEdges) return display;
    lastEdges = edges;
    display = edges.map((edge) => running.has(edge.source) || running.has(edge.target)
      ? { ...edge, animated: true, className: [edge.className, "canvas-edge-running"].filter(Boolean).join(" ") }
      : edge);
    return display;
  };
}

export const selectDisplayEdges = createDisplayEdgesSelector();
