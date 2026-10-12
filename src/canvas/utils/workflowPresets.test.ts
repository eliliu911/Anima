import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { PRESETS } from "./workflowPresets";
import { planLongTake } from "./longTakeWindows";
import type { VideoGenNodeData } from "../types";
import { auditVideoNode } from "./runAudit";

const TYPES = new Set(
  (/export type CanvasNodeType =([\s\S]*?);/.exec(
    readFileSync(new URL("../types.ts", import.meta.url), "utf8"),
  )?.[1] ?? "").match(/"[^"]+"/g)?.map((s) => s.slice(1, -1)) ?? [],
);

describe("内置模板", () => {
  it("类型表读到了（不然下面那条会假绿）", () => {
    expect(TYPES.size).toBeGreaterThan(8);
    expect(TYPES.has("videoGen")).toBe(true);
  });

  /**
   * 写一个不存在的类型进去，载入后画布上会出现一个渲染不出来的空节点，
   * **而且不报错**。
   */
  it("只用真实存在的节点类型", () => {
    for (const p of PRESETS) {
      for (const n of p.build().nodes) {
        expect(TYPES.has(n.type ?? ""), `${p.id} 用了不存在的类型 ${n.type}`).toBe(true);
      }
    }
  });

  it("边的两端都在自己的节点里 —— 悬空边会让执行器追不到上游", () => {
    for (const p of PRESETS) {
      const { nodes, edges } = p.build();
      const ids = new Set(nodes.map((n) => n.id));
      for (const e of edges) {
        expect(ids.has(e.source) && ids.has(e.target), `${p.id} 有悬空边`).toBe(true);
      }
    }
  });

  it("每套都有唯一 id 和节点 id", () => {
    expect(new Set(PRESETS.map((p) => p.id)).size).toBe(PRESETS.length);
    for (const p of PRESETS) {
      const { nodes } = p.build();
      expect(new Set(nodes.map((n) => n.id)).size).toBe(nodes.length);
    }
  });

  /** 内置模板自己就得通过我们自己的预检 —— 发一套「连了也白连」的接法最糟。 */
  it("长镜头模板是一个节点三个窗口，摆上去就能跑", () => {
    const { nodes, edges } = PRESETS.find((p) => p.id === "long_take_3")!.build();
    expect(nodes).toHaveLength(1);
    expect(edges).toHaveLength(0);
    const plan = planLongTake(nodes[0].data as VideoGenNodeData);
    expect(plan.ok, plan.ok ? "" : plan.why).toBe(true);
    if (plan.ok) expect(plan.windows).toBe(3);
  });

  it("动作迁移模板推导出来就是动作迁移，没有白连的线", () => {
    const { nodes, edges } = PRESETS.find((p) => p.id === "motion_transfer")!.build();
    const out = nodes[3];
    const a = auditVideoNode(out.id, nodes, edges);
    expect(a.mode).toBe("motion");
    // 模板是**刚摆上去的**，上游那张人物图还没生成 —— 预检会说「这一张还没出」，
    // 那是对的（等图出来它就生效了），不算白连
    expect(a.inputs.filter((i) => !i.used && !i.role.includes("还没出"))).toEqual([]);
    expect(a.conflicts).toEqual([]);
  });

  it("原片复刻模板：原视频直连出片，推导出来是动作迁移（原片复刻），没有深度节点、没有白连的线", () => {
    const { nodes, edges } = PRESETS.find((p) => p.id === "source_replica")!.build();
    expect(nodes.some((n) => n.type === "depthCapture")).toBe(false);
    const out = nodes[2];
    const a = auditVideoNode(out.id, nodes, edges);
    expect(a.mode).toBe("motion");
    expect(a.inputs.filter((i) => !i.used && !i.role.includes("还没出"))).toEqual([]);
    expect(a.conflicts).toEqual([]);
  });
});

describe("接线", () => {
  it("工作流面板把模板列出来了", () => {
    const panel = readFileSync(new URL("../ui/CanvasWorkflowPanel.tsx", import.meta.url), "utf8");
    expect(panel).toContain("PRESETS");
  });
});
