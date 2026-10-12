import { describe, expect, it, beforeEach } from "vitest";
import { readFileSync } from "node:fs";
import { COMET_LAYERS, COMET_PERIOD, cometOffsets, cutEdge, edgeClasses } from "./CutEdge";
import { useCanvasStore } from "../store/useCanvasStore";

describe("连线上的剪刀", () => {
  beforeEach(() => useCanvasStore.getState().clearCanvas());

  it("剪断：线没了，目标节点上绑着这个上游的参考也去掉（只删线的话那张图照样发出去）", () => {
    const s = useCanvasStore.getState();
    const a = s.addNode("imageGen", { x: 0, y: 0 }, { outputImageUrl: "/a.png" });
    const b = s.addNode("imageGen", { x: 0, y: 400 }, { outputImageUrl: "/b.png" });
    const v = s.addNode("videoGen", { x: 500, y: 0 }, {});
    s.setEdges([{ id: "ea", source: a, target: v }, { id: "eb", source: b, target: v }]);
    s.updateNodeData(v, { references: [{ path: "/a.png", name: "A", sourceNodeId: a }, { path: "/b.png", name: "B", sourceNodeId: b }] });
    cutEdge("ea");
    const st = useCanvasStore.getState();
    expect(st.edges.map((e) => e.id)).toEqual(["eb"]);
    expect(st.nodes.find((n) => n.id === v)!.data.references).toEqual([{ path: "/b.png", name: "B", sourceNodeId: b }]);
  });

  it("所有连线都用它（edgeTypes 的 default）", () => {
    const view = readFileSync(new URL("../CanvasView.tsx", import.meta.url), "utf8");
    expect(view).toContain("const edgeTypes = { default: CutEdge }");
    expect(view).toContain("edgeTypes={edgeTypes}");
  });

  /** 用户 2026-09-25：「增加对选中节点的链接线开启动态效果」 */
  it("选中节点、或两端有节点在跑时，线上走一颗彗星；减少动态效果时不动", () => {
    const edge = readFileSync(new URL("./CutEdge.tsx", import.meta.url), "utf8");
    expect(edge).toContain("s.selectedNodeId === source || s.selectedNodeId === target");
    expect(edge).toContain("{!plain && (active || running) && <Comet path={path} />}");
    expect(edge).toContain("prefers-reduced-motion");
    // canvas-sharpness.css 给连线颜色加了 !important：变亮必须走类名 + !important，内联样式压不过（09-25 查实）
    const css = readFileSync(new URL("./CutEdge.css", import.meta.url), "utf8");
    expect(edge).toContain("className={edgeClasses({ active, running, hover, plain })}");
    expect(css).toMatch(/canvas-edge-active\s*\{[^}]*!important/);
    // React Flow 的 `.react-flow__edge.animated path` 会给每根 path 套虚线：彗星的虚线必须写在行内样式里
    expect(edge).toContain("style={{ strokeDasharray: `${l.len} ${COMET_PERIOD - l.len}`, animation: \"none\" }}");
  });

  /** 用户 2026-09-25：「蓝色动态条…从中心开始到尾部的透明渐变。类似彗星效果」 */
  it("彗星：每层的头对齐在同一处，越往尾巴越长、越淡、越细", () => {
    for (const l of COMET_LAYERS) {
      const [from, to] = cometOffsets(l.len);
      // dash 画在 [-offset, -offset + len]，头 = len - offset：动画起止两端所有层都得一样
      expect(l.len - from).toBe(COMET_LAYERS[0].len - cometOffsets(COMET_LAYERS[0].len)[0]);
      expect(l.len - to).toBe(0);
      expect(from - to).toBe(COMET_PERIOD);
    }
    const byLen = [...COMET_LAYERS].sort((a, b) => b.len - a.len);
    for (let i = 1; i < byLen.length; i++) {
      expect(byLen[i].opacity).toBeGreaterThan(byLen[i - 1].opacity);
      expect(byLen[i].width).toBeGreaterThan(byLen[i - 1].width);
    }
    expect(Math.max(...COMET_LAYERS.map((l) => l.len))).toBeLessThan(COMET_PERIOD / 2);
  });

  /** 用户 2026-09-25：「工作流开始后变成了虚线」「鼠标悬浮后也变成了动态虚线，需要的是适当增加全线光韵效果」 */
  it("跑着、悬停、选中的线都不变虚线；悬停 / 选中是全线光晕", () => {
    const css = readFileSync(new URL("../canvas-sharpness.css", import.meta.url), "utf8");
    expect(css).not.toMatch(/stroke-dasharray:\s*\d/);
    expect(css).toMatch(/\.react-flow__edge:hover \.react-flow__edge-path,[^{]*\{[^}]*filter: drop-shadow[^}]*drop-shadow/);
  });

  /** 用户 2026-09-25：「减少动态效果」时回退旧样式 —— 只有虚线和实线，选中节点 / 生成时虚线移动 */
  it("减少动态效果：没有光晕和彗星，选中或在跑的线是移动虚线，其余实线", () => {
    const plain = { plain: true, hover: true };
    expect(edgeClasses({ ...plain, active: false, running: false })).toBe("canvas-edge-plain");
    expect(edgeClasses({ ...plain, active: true, running: false })).toBe("canvas-edge-plain canvas-edge-dash");
    expect(edgeClasses({ ...plain, active: false, running: true })).toBe("canvas-edge-plain canvas-edge-dash");
    // 悬停在减少动态效果下也不加光晕类
    expect(edgeClasses({ ...plain, active: false, running: false })).not.toContain("hover");
    // 正常模式：选中 / 悬停各自的类，没有虚线
    expect(edgeClasses({ plain: false, active: true, running: true, hover: true })).toBe("canvas-edge-active canvas-edge-hover");
    expect(edgeClasses({ plain: false, active: false, running: false, hover: false })).toBeUndefined();

    const css = readFileSync(new URL("./CutEdge.css", import.meta.url), "utf8");
    expect(css).toMatch(/canvas-edge-plain \{[^}]*filter: none !important/);
    expect(css).toMatch(/canvas-edge-plain\.canvas-edge-dash \{[^}]*stroke-dasharray: 4 6 !important;[^}]*animation: canvas-edge-dash-move[^}]*!important/);
  });
});

// 用户 2026-09-25：「把所有节点生成状态右上角这个蓝色光点去掉」—— 所有节点共用 BaseNodeWrapper，这里没有就都没有
describe("节点生成中", () => {
  it("右上角没有闪烁的蓝点", () => {
    const wrapper = readFileSync(new URL("../nodes/BaseNodeWrapper.tsx", import.meta.url), "utf8");
    expect(wrapper).not.toContain("animate-ping");
    expect(wrapper).toContain("<CancelJobButton nodeId={id} />");
  });
});
