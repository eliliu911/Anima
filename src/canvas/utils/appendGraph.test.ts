import { describe, expect, it } from "vitest";
import type { CanvasEdge, CanvasNode } from "../types";
import { appendGraph } from "./appendGraph";

const node = (id: string, x: number, y: number, extra: Record<string, unknown> = {}) =>
  ({ id, type: "imageGen", position: { x, y }, data: {}, ...extra }) as unknown as CanvasNode;
const edge = (s: string, t: string) => ({ id: `${s}>${t}`, source: s, target: t }) as CanvasEdge;

describe("appendGraph（载入工作流追加到画布，不清空）", () => {
  it("原有节点和连线都留着，新的一组放在右边、顶边对齐", () => {
    const old = [node("a", 0, 100, { measured: { width: 300 } }), node("b", 500, 300, { measured: { width: 400 } })];
    const g = appendGraph(old, [edge("a", "b")], [node("x", 40, 40), node("y", 620, 40)], [edge("x", "y")]);
    expect(g.nodes.slice(0, 2)).toEqual(old);
    expect(g.edges[0]).toEqual(edge("a", "b"));
    const x = g.nodes.find((n) => n.id === "x")!;
    // 现有内容最右 = 500 + 400 = 900，隔 240；左边 40 → 平移 1100；顶 100 - 40 = 60
    expect(x.position).toEqual({ x: 1140, y: 100 });
    expect(g.nodes.find((n) => n.id === "y")!.position).toEqual({ x: 1720, y: 100 });
    expect(g.addedIds).toEqual(["x", "y"]);
  });

  it("空画布原样放", () => {
    const g = appendGraph([], [], [node("x", 40, 40)], []);
    expect(g.nodes[0].position).toEqual({ x: 40, y: 40 });
  });

  it("同一份载两次：id 撞了换名，连线跟着换，不串到旧节点上", () => {
    const first = appendGraph([], [], [node("card", 0, 0), node("v", 600, 0)], [edge("card", "v")]);
    const second = appendGraph(first.nodes, first.edges, [node("card", 0, 0), node("v", 600, 0)], [edge("card", "v")]);
    expect(second.addedIds).toEqual(["card-2", "v-2"]);
    expect(second.edges).toHaveLength(2);
    expect(second.edges[1]).toMatchObject({ source: "card-2", target: "v-2" });
    expect(new Set(second.edges.map((e) => e.id)).size).toBe(2);
    expect(new Set(second.nodes.map((n) => n.id)).size).toBe(4);
  });

  it("编组里的子节点跟着父节点换名、位置不再平移（它是相对父节点的）", () => {
    const g = appendGraph([node("g", 0, 0)], [], [node("g", 0, 0, { type: "group" }), node("c", 10, 10, { parentId: "g" })], []);
    const c = g.nodes.find((n) => n.id === "c")!;
    expect((c as { parentId?: string }).parentId).toBe("g-2");
    expect(c.position).toEqual({ x: 10, y: 10 });
  });
});
