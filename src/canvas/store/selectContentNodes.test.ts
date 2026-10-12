import { describe, expect, it } from "vitest";
import { createContentNodesSelector } from "./selectContentNodes";
import { useCanvasStore } from "./useCanvasStore";
import type { CanvasNode } from "../types";

const graph = (count: number): CanvasNode[] => Array.from({ length: count }, (_, i) => ({
  id: String(i), type: "prompt", position: { x: i * 100, y: 0 }, data: { prompt: String(i) },
} as CanvasNode));

describe("canvas content subscriptions", () => {
  it("keeps 500-node DAG previews stable through 120 drag frames and selection/measurement updates", () => {
    const select = createContentNodesSelector();
    let nodes = graph(500);
    const original = select({ nodes });
    for (let frame = 0; frame < 120; frame++) {
      nodes = nodes.map((n, i) => i === 0 ? { ...n, position: { x: frame, y: 0 },
        selected: true, dragging: true, measured: { width: 440, height: 300 } } : n);
      expect(select({ nodes })).toBe(original);
    }
    nodes = nodes.map((n, i) => i === 1 ? { ...n, data: { ...n.data, prompt: "changed" } } as CanvasNode : n);
    expect(select({ nodes })).toBe(nodes);
    expect(select({ nodes: nodes.slice(1) })).not.toBe(nodes);
  });

  it("invalidates on order, identity and type changes", () => {
    const select = createContentNodesSelector();
    const nodes = graph(2);
    select({ nodes });
    const reordered = [...nodes].reverse();
    expect(select({ nodes: reordered })).toBe(reordered);
    const replaced = [{ ...reordered[0], id: "new" }, reordered[1]];
    expect(select({ nodes: replaced })).toBe(replaced);
    const retyped = [{ ...replaced[0], type: "imageGen" }, replaced[1]] as CanvasNode[];
    expect(select({ nodes: retyped })).toBe(retyped);
  });

  it("edge selection preserves all node identities; removal only changes affected references", () => {
    const nodes = graph(3);
    nodes[1] = { ...nodes[1], data: { ...nodes[1].data, references: [{ sourceNodeId: "0", path: "image.png" }] } } as CanvasNode;
    useCanvasStore.setState({ nodes, edges: [{ id: "e", source: "0", target: "1" }] });
    useCanvasStore.getState().onEdgesChange([{ id: "e", type: "select", selected: true }]);
    expect(useCanvasStore.getState().nodes).toBe(nodes);
    useCanvasStore.getState().onEdgesChange([{ id: "e", type: "remove" }]);
    const next = useCanvasStore.getState().nodes;
    expect(next[0]).toBe(nodes[0]);
    expect(next[2]).toBe(nodes[2]);
    expect(next[1].data.references).toEqual([]);
  });
});
