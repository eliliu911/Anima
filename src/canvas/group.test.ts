import { describe, expect, it } from "vitest";
import {
  GROUP_SIDE_PAD,
  GROUP_TOP_PAD,
  absolutePosition,
  descendantIds,
  fitGroup,
  groupNodes,
  IMAGE_CELL,
  capturePartners,
  layoutImageGroup,
  ungroup,
} from "./group";
import type { CanvasEdge, CanvasNode } from "./types";

const n = (id: string, x: number, y: number, extra: Partial<CanvasNode> = {}): CanvasNode =>
  ({ id, type: "prompt", position: { x, y }, width: 100, height: 50, data: {}, ...extra }) as CanvasNode;

const abs = (nodes: CanvasNode[], id: string) => {
  const byId = new Map(nodes.map((x) => [x.id, x] as const));
  return absolutePosition(byId.get(id)!, byId);
};

describe("成组", () => {
  it("少于两个节点不成组", () => {
    expect(groupNodes([n("a", 0, 0)], ["a"], { groupId: "g" })).toBeNull();
  });

  /** 成员坐标换成相对组框，但**画布上看起来一动不动**。 */
  it("成组前后成员的绝对位置不变", () => {
    const nodes = [n("a", 100, 200), n("b", 400, 260), n("c", 900, 900)];
    const r = groupNodes(nodes, ["a", "b"], { groupId: "g" })!;
    expect(abs(r.nodes, "a")).toEqual({ x: 100, y: 200 });
    expect(abs(r.nodes, "b")).toEqual({ x: 400, y: 260 });
    expect(r.nodes.find((x) => x.id === "a")!.parentId).toBe("g");
    expect(r.nodes.find((x) => x.id === "c")!.parentId).toBeUndefined();
  });

  it("组框包住成员并留出边距", () => {
    const r = groupNodes([n("a", 100, 200), n("b", 400, 260)], ["a", "b"], { groupId: "g" })!;
    const g = r.nodes.find((x) => x.id === "g")!;
    expect(g.position).toEqual({ x: 100 - GROUP_SIDE_PAD, y: 200 - GROUP_TOP_PAD });
    expect(g.width).toBe(400 + 100 - 100 + GROUP_SIDE_PAD * 2);
  });

  /** ReactFlow 要求父节点先于子节点出现，否则子节点位置算错。 */
  it("组节点排在所有成员前面", () => {
    const r = groupNodes([n("x", 0, 0), n("a", 100, 0), n("b", 300, 0)], ["b", "a"], { groupId: "g" })!;
    const ids = r.nodes.map((x) => x.id);
    expect(ids.indexOf("g")).toBeLessThan(ids.indexOf("a"));
    expect(ids.indexOf("g")).toBeLessThan(ids.indexOf("b"));
  });

  it("没起名时按已有组数编号", () => {
    const first = groupNodes([n("a", 0, 0), n("b", 200, 0)], ["a", "b"], { groupId: "g1" })!;
    const second = groupNodes([...first.nodes, n("c", 0, 500), n("d", 200, 500)], ["c", "d"], { groupId: "g2" })!;
    expect((second.nodes.find((x) => x.id === "g2")!.data as { label: string }).label).toBe("组 2");
  });

  /** 同时选中了组和组里的成员：成员跟着组走，不能被挂到两个父节点下。 */
  it("选中的后代不单独算成员", () => {
    const g1 = groupNodes([n("a", 0, 0), n("b", 200, 0), n("c", 0, 500)], ["a", "b"], { groupId: "g1" })!;
    const r = groupNodes(g1.nodes, ["g1", "a", "c"], { groupId: "g2" })!;
    expect(r.nodes.find((x) => x.id === "a")!.parentId).toBe("g1");
    expect(r.nodes.find((x) => x.id === "g1")!.parentId).toBe("g2");
    expect(abs(r.nodes, "a")).toEqual({ x: 0, y: 0 });
  });
});

describe("解组", () => {
  it("成员坐标还原成绝对坐标，组框和连在组框上的边删掉", () => {
    const g = groupNodes([n("a", 100, 200), n("b", 400, 260)], ["a", "b"], { groupId: "g" })!;
    const edges = [
      { id: "e1", source: "a", target: "b" },
      { id: "e2", source: "g", target: "a" },
    ] as CanvasEdge[];
    const r = ungroup(g.nodes, edges, "g")!;
    expect(r.nodes.find((x) => x.id === "g")).toBeUndefined();
    expect(r.nodes.find((x) => x.id === "a")!.position).toEqual({ x: 100, y: 200 });
    expect(r.nodes.find((x) => x.id === "a")!.parentId).toBeUndefined();
    expect(r.edges.map((e) => e.id)).toEqual(["e1"]);
  });

  it("不是组就不动", () => {
    expect(ungroup([n("a", 0, 0)], [], "a")).toBeNull();
  });
});

describe("组框跟着成员撑大", () => {
  it("成员拖出右下边，框撑大", () => {
    const g = groupNodes([n("a", 0, 0), n("b", 200, 0)], ["a", "b"], { groupId: "g" })!;
    const moved = g.nodes.map((x) => (x.id === "b" ? { ...x, position: { x: 900, y: 700 } } : x)) as CanvasNode[];
    const fitted = fitGroup(moved, "g");
    const gg = fitted.find((x) => x.id === "g")!;
    expect(gg.width).toBeGreaterThanOrEqual(900 + 100 + GROUP_SIDE_PAD);
    expect(gg.height).toBeGreaterThanOrEqual(700 + 50);
  });

  /** 拖出左 / 上边：框往外长，成员在画布上的位置不能跳。 */
  it("成员拖出左上边，框往左上长、成员不跳", () => {
    const g = groupNodes([n("a", 0, 0), n("b", 200, 0)], ["a", "b"], { groupId: "g" })!;
    const moved = g.nodes.map((x) => (x.id === "a" ? { ...x, position: { x: -300, y: -200 } } : x)) as CanvasNode[];
    const before = abs(moved, "a");
    const fitted = fitGroup(moved, "g");
    expect(abs(fitted, "a")).toEqual(before);
    expect(abs(fitted, "b")).toEqual(abs(moved, "b"));
  });

  it("已经包住时返回同一个数组（不触发多余的状态更新）", () => {
    const g = groupNodes([n("a", 0, 0), n("b", 200, 0)], ["a", "b"], { groupId: "g" })!;
    expect(fitGroup(g.nodes, "g")).toBe(g.nodes);
  });

  /** 用户手动拉大的框不能被收回去。 */
  it("只撑大不缩小", () => {
    const g = groupNodes([n("a", 0, 0), n("b", 200, 0)], ["a", "b"], { groupId: "g" })!;
    const big = g.nodes.map((x) => (x.id === "g" ? { ...x, width: 2000, height: 2000, style: { width: 2000, height: 2000 } } : x)) as CanvasNode[];
    expect(fitGroup(big, "g")).toBe(big);
  });
});

describe("后代", () => {
  it("递归找到成员的成员", () => {
    const g1 = groupNodes([n("a", 0, 0), n("b", 200, 0), n("c", 0, 500)], ["a", "b"], { groupId: "g1" })!;
    const g2 = groupNodes(g1.nodes, ["g1", "c"], { groupId: "g2" })!;
    expect([...descendantIds(g2.nodes, ["g2"])].sort()).toEqual(["a", "b", "c", "g1"]);
  });
});

describe("截图排成组", () => {
  const src = n("pano", 100, 200, { width: 864, height: 420 });
  const items = (k: number) =>
    Array.from({ length: k }, (_, i) => ({ id: `img${i}`, path: `/tmp/${i}.png`, label: `图${i}` }));

  it("一张图不成组，放在源节点右边", () => {
    const r = layoutImageGroup([src], [], "pano", items(1), { cols: 2, groupId: "g", label: "x" })!;
    expect(r.nodes.some((x) => x.type === "group")).toBe(false);
    expect(r.focusId).toBe("img0");
    const img = r.nodes.find((x) => x.id === "img0")!;
    expect(img.position.x).toBeGreaterThan(100 + 864);
    expect(img.position.y).toBe(200);
    expect((img.data as { referenceOnly?: boolean; outputImageUrl?: string }).outputImageUrl).toBe("/tmp/0.png");
  });

  /** 12 视角排 4 列：第 5 张必须换行回到第一列 —— 行就是俯仰，排错了就读不懂。 */
  it("多张按列数排网格、读序不乱，外面套一个组", () => {
    const r = layoutImageGroup([src], [], "pano", items(12), { cols: 4, groupId: "g", label: "全景截图 · 12 视角" })!;
    const group = r.nodes.find((x) => x.id === "g")!;
    expect(group.type).toBe("group");
    expect((group.data as { label: string }).label).toBe("全景截图 · 12 视角");
    expect(r.nodes.filter((x) => x.parentId === "g")).toHaveLength(12);
    const at = (i: number) => abs(r.nodes, `img${i}`);
    expect(at(1).y).toBe(at(0).y);
    expect(at(1).x).toBeGreaterThanOrEqual(at(0).x + IMAGE_CELL.width);
    expect(at(4).x).toBe(at(0).x);
    expect(at(4).y).toBeGreaterThanOrEqual(at(0).y + IMAGE_CELL.height);
    expect(at(11).x).toBe(at(3).x);
    // 组节点在成员前面
    expect(r.nodes.findIndex((x) => x.id === "g")).toBeLessThan(r.nodes.findIndex((x) => x.id === "img0"));
  });

  /** 工作流上要看得出图是哪个节点截的：每张图一条线，从源节点的输出口连到图片的输入口。 */
  it("每张图都从源节点连一条线过来", () => {
    const old = { id: "keep", source: "x", target: "pano" } as CanvasEdge;
    for (const k of [1, 4]) {
      const r = layoutImageGroup([src], [old], "pano", items(k), { cols: 2, groupId: "g", label: "x", sourceHandle: "pano-view-out" })!;
      expect(r.edges[0]).toBe(old);
      const added = r.edges.slice(1);
      expect(added).toHaveLength(k);
      for (let i = 0; i < k; i++) {
        expect(added[i]).toMatchObject({ source: "pano", sourceHandle: "pano-view-out", target: `img${i}`, targetHandle: "prompt-in" });
      }
      expect(new Set(r.edges.map((e) => e.id)).size).toBe(r.edges.length);
    }
  });

  /** 截图之前选中的节点要取消选中 —— 否则紧接着按 Delete 会把源节点一起删掉。 */
  it("原来选中的节点取消选中", () => {
    const r = layoutImageGroup([{ ...src, selected: true }], [], "pano", items(4), { cols: 2, groupId: "g", label: "x" })!;
    expect(r.nodes.find((x) => x.id === "pano")!.selected).toBe(false);
    expect(r.nodes.filter((x) => x.selected).map((x) => x.id)).toEqual(["g"]);
  });

  it("源节点不存在或没有图时什么都不做", () => {
    expect(layoutImageGroup([src], [], "nope", items(2), { cols: 2, groupId: "g", label: "x" })).toBeNull();
    expect(layoutImageGroup([src], [], "pano", [], { cols: 2, groupId: "g", label: "x" })).toBeNull();
  });

  it("纯图片节点按图片格子算尺寸，不按生图节点", () => {
    const r = layoutImageGroup([src], [], "pano", items(2), { cols: 2, groupId: "g", label: "x" })!;
    const g = r.nodes.find((x) => x.id === "g")!;
    expect(g.width).toBe(IMAGE_CELL.width * 2 + 24 + GROUP_SIDE_PAD * 2);
  });
});

describe("拖动联动", () => {
  const meta = (source: string) => ({ referenceOnly: true, captureMetadata: { viewer: "director_world", source_node_id: source } });
  const build = () => {
    const dw = n("dw", 0, 0, { type: "directorWorld" } as Partial<CanvasNode>);
    const other = n("other", 0, 500);
    const r = layoutImageGroup([dw, other], [], "dw", [
      { id: "c", path: "/c.png", label: "导演合成图", data: meta("dw") },
      { id: "e", path: "/e.png", label: "纯背景图", data: meta("dw") },
    ], { cols: 2, groupId: "g", label: "导演世界输出" })!;
    return r.nodes;
  };

  it("截图节点带上额外 data", () => {
    const nodes = build();
    expect((nodes.find((x) => x.id === "c")!.data as { captureMetadata: { viewer: string } }).captureMetadata.viewer).toBe("director_world");
  });

  it("拖源节点带着输出组走，拖输出组带着源节点走", () => {
    const nodes = build();
    expect(capturePartners("dw", nodes)).toEqual(["g"]);
    expect(capturePartners("g", nodes)).toEqual(["dw"]);
    expect(capturePartners("other", nodes)).toEqual([]);
  });

  /** 源节点自己在组里时不联动 —— 否则父组被当成另一方，拖一下移两次 */
  it("被拖的节点在组里时不联动", () => {
    const nodes = build().map((x) => (x.id === "dw" ? ({ ...x, parentId: "outer" } as CanvasNode) : x));
    expect(capturePartners("dw", nodes)).toEqual([]);
    expect(capturePartners("g", nodes)).toEqual([]);
  });

  it("组里的截图不是这个源节点输出的，不算", () => {
    const nodes = build().map((x) =>
      x.parentId === "g" ? ({ ...x, data: { ...x.data, captureMetadata: { source_node_id: "someone-else" } } } as CanvasNode) : x,
    );
    expect(capturePartners("dw", nodes)).toEqual([]);
  });
});
