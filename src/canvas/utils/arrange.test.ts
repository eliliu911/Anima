import { describe, it, expect, beforeEach } from "vitest";
import type { CanvasEdge, CanvasNode } from "../types";
import { arrangeAll, arrangePositions, components, GAP, type ArrangeMode } from "./arrange";
import { nodeSize } from "../group";
import { useCanvasStore, ARRANGE_MODE_KEY, getStorage } from "../store/useCanvasStore";

const n = (id: string, type: string, x = 0, y = 0, h?: number): CanvasNode =>
  ({ id, type, position: { x, y }, data: {}, ...(h ? { measured: { width: 440, height: h } } : {}) }) as unknown as CanvasNode;
const e = (s: string, t: string): CanvasEdge => ({ id: `${s}>${t}`, source: s, target: t }) as CanvasEdge;

/** 《动作复刻示例》那种：两张图 + 8 段（分段视频 → 出片），出片节点很高 */
function kotte() {
  const nodes = [n("charA", "imageGen", 0, 0), n("charB", "imageGen", 0, 400)];
  const edges: CanvasEdge[] = [];
  for (let k = 1; k <= 8; k++) {
    nodes.push(n(`clip${k}`, "videoGen", 500, k * 100, 520), n(`part${k}`, "videoGen", 1000, k * 100, 760));
    edges.push(e(`clip${k}`, `part${k}`), e("charA", `part${k}`), e("charB", `part${k}`));
  }
  return { nodes, edges };
}

function overlaps(nodes: CanvasNode[], pos: Map<string, { x: number; y: number }>) {
  const box = nodes.map((x) => ({ id: x.id, ...pos.get(x.id)!, ...nodeSize(x) }));
  const hits: string[] = [];
  for (let i = 0; i < box.length; i++) for (let j = i + 1; j < box.length; j++) {
    const a = box[i], b = box[j];
    if (a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height) hits.push(`${a.id}×${b.id}`);
  }
  return hits;
}

describe("自动整理画布：三种排法", () => {
  it.each<ArrangeMode>(["horizontal", "vertical", "grid"])("%s：按真实尺寸排，谁都不压谁（以前固定 480 行高，高节点叠在一起）", (mode) => {
    const { nodes, edges } = kotte();
    const pos = arrangePositions(nodes, edges, mode);
    expect(pos.size).toBe(nodes.length);
    expect(overlaps(nodes, pos)).toEqual([]);
  });

  it("横向：上游在左、下游在右；同一列按高度往下排，间距是 GAP", () => {
    const { nodes, edges } = kotte();
    const pos = arrangePositions(nodes, edges, "horizontal");
    for (let k = 1; k <= 8; k++) {
      expect(pos.get(`part${k}`)!.x).toBeGreaterThan(pos.get(`clip${k}`)!.x);
      expect(pos.get(`part${k}`)!.x).toBeGreaterThan(pos.get("charA")!.x);
    }
    expect(pos.get("part2")!.y - pos.get("part1")!.y).toBe(760 + GAP);
  });

  it("纵向：上游在上、下游在下，同一层横着排", () => {
    const { nodes, edges } = kotte();
    const pos = arrangePositions(nodes, edges, "vertical");
    for (let k = 1; k <= 8; k++) expect(pos.get(`part${k}`)!.y).toBeGreaterThan(pos.get(`clip${k}`)!.y);
    expect(pos.get("part1")!.y).toBe(pos.get("part8")!.y);
    expect(pos.get("part2")!.x).toBeGreaterThan(pos.get("part1")!.x);
  });

  it("网格：不看连线，码成接近正方形", () => {
    const { nodes } = kotte();
    const pos = arrangePositions(nodes, [], "grid");
    const cols = new Set([...pos.values()].map((p) => p.x)).size;
    const rows = new Set([...pos.values()].map((p) => p.y)).size;
    expect(cols).toBe(5);   // 18 个 → ceil(√18) = 5 列
    expect(rows).toBe(4);
  });

  it("同一层按上游的位置排：第 3 段的出片跟在第 3 段的视频旁边，连线不交叉", () => {
    // 数组顺序故意打乱：出片节点倒着放
    const nodes = [n("a", "videoGen", 0, 0), n("b", "videoGen", 0, 900), n("c", "videoGen", 0, 1800),
      n("C", "videoGen", 800, 0), n("B", "videoGen", 800, 900), n("A", "videoGen", 800, 1800)];
    const pos = arrangePositions(nodes, [e("a", "A"), e("b", "B"), e("c", "C")], "horizontal");
    expect(pos.get("A")!.y).toBeLessThan(pos.get("B")!.y);
    expect(pos.get("B")!.y).toBeLessThan(pos.get("C")!.y);
  });

  it("有环也能排完，不卡死", () => {
    const nodes = [n("a", "prompt"), n("b", "prompt"), n("c", "prompt")];
    const pos = arrangePositions(nodes, [e("a", "b"), e("b", "c"), e("c", "a")], "horizontal");
    expect(pos.size).toBe(3);
    expect(overlaps(nodes, pos)).toEqual([]);
  });
});

describe("整理按钮记住上一次的排法", () => {
  beforeEach(() => useCanvasStore.getState().clearCanvas());

  it("选过纵向，之后不带参数（⌥⇧F）也按纵向；存进本机", () => {
    const s = useCanvasStore.getState();
    const a = s.addNode("prompt", { x: 0, y: 0 });
    const b = s.addNode("prompt", { x: 0, y: 0 });
    s.setEdges([e(a, b)]);
    useCanvasStore.getState().arrangeNodes("vertical");
    expect(useCanvasStore.getState().arrangeMode).toBe("vertical");
    // 测试环境可能没有本地存储；有就得存上
    const st = getStorage();
    if (st && typeof st.getItem === "function" && st.getItem("__probe__") !== undefined) expect(st.getItem(ARRANGE_MODE_KEY)).toBe("vertical");
    useCanvasStore.getState().setNodes(useCanvasStore.getState().nodes.map((x) => ({ ...x, position: { x: 0, y: 0 } })));
    useCanvasStore.getState().arrangeNodes();
    const got = useCanvasStore.getState().nodes;
    const pa = got.find((x) => x.id === a)!.position, pb = got.find((x) => x.id === b)!.position;
    expect(pb.y).toBeGreaterThan(pa.y);
    expect(pb.x).toBe(pa.x);
  });
});

// 2026-10-13 用户：「当有多组工作流的时候，不同工作流里的节点会混在一起」
describe("多套工作流各排各的（arrangeAll）", () => {
  /** 两套一样的「4 张参考图 → 2 个视频」，第二套原来摆在第一套右边 */
  function twoWorkflows() {
    const nodes: CanvasNode[] = [];
    const edges: CanvasEdge[] = [];
    for (const [w, dx] of [["a", 0], ["b", 3000]] as const) {
      for (let i = 0; i < 4; i++) nodes.push(n(`${w}ref${i}`, "imageGen", dx, i * 430));
      for (let v = 0; v < 2; v++) {
        nodes.push(n(`${w}vid${v}`, "videoGen", dx + 600, v * 700, 600));
        for (let i = 0; i < 4; i++) edges.push(e(`${w}ref${i}`, `${w}vid${v}`));
      }
    }
    nodes.push(n("lonely", "prompt", 9000, 9000));
    return { nodes, edges };
  }
  const box = (nodes: CanvasNode[], pos: Map<string, { x: number; y: number }>, prefix: string) => {
    const mine = nodes.filter((x) => x.id.startsWith(prefix));
    return {
      top: Math.min(...mine.map((x) => pos.get(x.id)!.y)),
      bottom: Math.max(...mine.map((x) => pos.get(x.id)!.y + nodeSize(x).height)),
      left: Math.min(...mine.map((x) => pos.get(x.id)!.x)),
      right: Math.max(...mine.map((x) => pos.get(x.id)!.x + nodeSize(x).width)),
    };
  };

  it("按连线分组：两套 + 一个散件，散件排最后", () => {
    const { nodes, edges } = twoWorkflows();
    const g = components(nodes, edges).map((c) => c.map((x) => x.id[0]).join(""));
    expect(g).toHaveLength(3);
    expect(g[0]).toMatch(/^a+$/);
    expect(g[1]).toMatch(/^b+$/);
    expect(g[2]).toBe("l");
  });

  for (const mode of ["horizontal", "vertical", "grid"] as ArrangeMode[]) {
    it(`${mode}：两套的范围不交叉，也没有节点重叠`, () => {
      const { nodes, edges } = twoWorkflows();
      const pos = arrangeAll(nodes, edges, mode);
      expect(pos.size).toBe(nodes.length);
      expect(overlaps(nodes, pos)).toEqual([]);
      const a = box(nodes, pos, "a"), b = box(nodes, pos, "b");
      const apart = mode === "vertical" ? a.right < b.left : a.bottom < b.top;
      expect(apart, JSON.stringify({ a, b })).toBe(true);
    });
  }

  it("画布上点「自动整理」走的就是这一套（store 接线）", () => {
    const { nodes, edges } = twoWorkflows();
    useCanvasStore.getState().setNodes(nodes);
    useCanvasStore.getState().setEdges(edges);
    useCanvasStore.getState().arrangeNodes("horizontal");
    const got = useCanvasStore.getState().nodes;
    const pos = new Map(got.map((x) => [x.id, x.position]));
    const a = box(got, pos, "a"), b = box(got, pos, "b");
    expect(a.bottom < b.top, JSON.stringify({ a, b })).toBe(true);
  });

  it("以前的单张图排法会把两套的参考图排进同一列（这条是证明旧行为确实混在一起）", () => {
    const { nodes, edges } = twoWorkflows();
    const pos = arrangePositions(nodes, edges, "horizontal");
    const xs = new Set(nodes.filter((x) => x.id.includes("ref")).map((x) => pos.get(x.id)!.x));
    expect(xs.size).toBe(1);
  });
});
