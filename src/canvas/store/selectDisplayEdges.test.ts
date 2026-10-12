import { describe, expect, it } from "vitest";
import type { CanvasEdge, CanvasNode } from "../types";
import { createDisplayEdgesSelector } from "./selectDisplayEdges";
const node = (id: string, isGenerating = false): CanvasNode => ({
  id, type: "imageGen", position: { x: 0, y: 0 }, data: { isGenerating },
} as CanvasNode);
const edges: CanvasEdge[] = [
  { id: "in", source: "a", target: "b", animated: true, className: "custom" },
  { id: "out", source: "b", target: "c" },
  { id: "other", source: "c", target: "d" },
];
describe("runtime edge animation", () => {
  it("marks only edges touching running nodes without modifying saved edges", () => {
    const select = createDisplayEdgesSelector();
    const result = select({ nodes: [node("a"), node("b", true), node("c")], edges });
    expect(result[0].className).toBe("custom canvas-edge-running");
    expect(result[1].animated).toBe(true);
    expect(result[1].className).toBe("canvas-edge-running");
    expect(result[2]).toBe(edges[2]);
    expect(edges[0].className).toBe("custom");
    expect(edges[1].animated).toBeUndefined();
  });
  it("removes running styling on completion, failure, cancellation or project switch", () => {
    for (const data of [{ isGenerating: false }, { isGenerating: false, error: "failed" },
      { isGenerating: false, cancelled: true }]) {
      const select = createDisplayEdgesSelector();
      select({ nodes: [node("b", true)], edges });
      expect(select({ nodes: [{ ...node("b"), data } as CanvasNode], edges })[0]).toBe(edges[0]);
      select({ nodes: [node("b", true)], edges });
      expect(select({ nodes: [], edges })[0]).toBe(edges[0]);
    }
  });
  it("preserves the edge array through dragging and unrelated data updates", () => {
    const select = createDisplayEdgesSelector();
    const nodes = [node("b", true)];
    const result = select({ nodes, edges });
    expect(select({ nodes: [{ ...nodes[0], position: { x: 30, y: 40 } }], edges })).toBe(result);
    expect(select({ nodes: [{ ...nodes[0], data: { ...nodes[0].data, prompt: "new" } } as CanvasNode], edges })).toBe(result);
    expect(select({ nodes, edges: [...edges, { id: "new", source: "b", target: "d" }] })[3].className).toBe("canvas-edge-running");
  });
});
