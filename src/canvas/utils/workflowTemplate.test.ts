import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import type { CanvasEdge, CanvasNode } from "../types";
import { slugOf, templateNeeds, toTemplate } from "./workflowTemplate";

const node = (id: string, type: string, data: Record<string, unknown>): CanvasNode =>
  ({ id, type, position: { x: 0, y: 0 }, data } as CanvasNode);

describe("存成模板时剥掉什么", () => {
  const nodes = [
    node("sh", "shot", { projectId: "p1", episodeId: "ep_05", shotId: "s3", label: "第 3 镜", frame: "E05 那一镜的正文" }),
    node("img", "imageGen", {
      prompt: "一个女人站在窗前", engine: "zimage", width: 1024, height: 1024, seed: 7,
      outputImageUrl: "/E05/产物.png", outputImages: ["/E05/产物.png"],
      error: "上一次失败了", notes: ["上一次的备注"], isGenerating: true, lastExecutedSignature: "abc",
      references: [{ path: "/E05/场景.png", name: "客厅" }, { path: "/E05/人.png", name: "筱筱" }],
    }),
  ];
  const edges: CanvasEdge[] = [{ id: "e", source: "sh", target: "img" } as CanvasEdge];
  const t = toTemplate("首帧流程", nodes, edges);
  const img = t.nodes[1].data as Record<string, unknown>;
  const sh = t.nodes[0].data as Record<string, unknown>;

  it("产物一律不进模板 —— 否则载入后看着像这次出的", () => {
    for (const k of ["outputImageUrl", "outputImages", "isGenerating", "lastExecutedSignature"]) {
      expect(img[k], k).toBeUndefined();
    }
  });

  it("陈旧的错误和备注不进模板", () => {
    expect(img.error).toBeUndefined();
    expect(img.notes).toBeUndefined();
  });

  it("接法和参数留着 —— 那才是工作流本身", () => {
    expect(img.prompt).toBe("一个女人站在窗前");
    expect(img.engine).toBe("zimage");
    expect([img.width, img.height, img.seed]).toEqual([1024, 1024, 7]);
    expect(t.edges).toHaveLength(1);
  });

  /** 绝对路径载进另一集就指着旧集的文件，**而那张图还在，所以不报错**。 */
  /**
   * **指向图里另一个节点的绑定是接法，不是素材。**
   *
   * 首尾帧就靠它：视频节点上绑两条 `{sourceNodeId, name}`，一条叫「尾帧」，
   * `inferVideoMode` 据此推 fl2v。这种绑定没有路径、没有资产 id，
   * 换个项目照样成立 —— 一起剥掉的话首尾帧的模板根本存不下来。
   */
  it("指向图里另一个节点的绑定要留着（首尾帧靠它）", () => {
    const t2 = toTemplate("首尾帧", [
      node("a", "imageGen", { engine: "zimage" }),
      node("b", "imageGen", { engine: "zimage" }),
      node("v", "videoGen", { engine: "h3", references: [
        { sourceNodeId: "a", path: "/E05/首.png", name: "首帧" },
        { sourceNodeId: "b", path: "/E05/尾.png", name: "尾帧" },
        { path: "/E05/别的.png", name: "外部素材", assetId: "as_1" },
      ] }),
    ], []);
    const v = t2.nodes[2].data as { references?: { sourceNodeId?: string; path: string; name: string }[] };
    expect(v.references?.map((r) => [r.sourceNodeId, r.name])).toEqual([["a", "首帧"], ["b", "尾帧"]]);
    // **路径还是不能带** —— 换个项目它指着旧集的文件，而那张图还在，所以不报错
    expect(JSON.stringify(t2)).not.toContain("/E05/");
    // 指着具体素材的那条照旧只留名字
    expect(t2.slots).toEqual([{ nodeId: "v", names: ["外部素材"] }]);
  });

  it("参考图只留名字不留路径", () => {
    expect(img.references).toBeUndefined();
    expect(t.slots).toEqual([{ nodeId: "img", names: ["客厅", "筱筱"] }]);
    expect(JSON.stringify(t)).not.toContain("/E05/场景.png");
  });

  it("分镜绑定和缓存的那一镜正文都剥掉", () => {
    expect(sh.projectId).toBeUndefined();
    expect(sh.episodeId).toBeUndefined();
    expect(sh.shotId).toBeUndefined();
    expect(sh.frame).toBe("");
    expect(JSON.stringify(t)).not.toContain("E05 那一镜的正文");
  });

  it("只留两端都在的边（存选中的一部分时）", () => {
    const part = toTemplate("一半", [nodes[1]], edges);
    expect(part.edges).toHaveLength(0);
  });
});

describe("载入前的缺口清单", () => {
  const t = toTemplate("流程", [
    node("sh", "shot", { projectId: "p1", episodeId: "ep_05", shotId: "s3" }),
    node("l", "lora", { name: "zimage/有的.safetensors", strength: 1 }),
    node("l2", "lora", { name: "zimage/没有的.safetensors", strength: 1 }),
    node("img", "imageGen", { engine: "zimage", references: [{ path: "/a.png", name: "客厅" }] }),
    node("v", "videoGen", { engine: "没这个引擎" }),
  ], []);
  const needs = templateNeeds(t, { loras: ["zimage/有的.safetensors"], engines: ["zimage", "h3"] });
  const find = (s: string) => needs.find((n) => n.label.includes(s));

  it("分镜一定要用户自己挂 —— 模板里没有绑定", () => {
    expect(find("挂一镜")?.status).toBe("missing");
  });

  it("参考图要用户自己填，并提示原来挂的是谁", () => {
    const r = find("参考图");
    expect(r?.status).toBe("missing");
    expect(r?.label).toContain("客厅");
    expect(r?.nodeId).toBe("img");
  });

  it("LoRA 在不在本机要当场查出来 —— 挂上去不生效且不报错", () => {
    expect(find("有的.safetensors")?.status).toBe("ok");
    expect(find("没有的.safetensors")?.status).toBe("missing");
  });

  /**
   * **孤立节点是典型的静默失效。**
   *
   * 2026-09-21 用户在自己的工作流里当场看出来一个：一段写着「火箭筒」的
   * 文本节点悬在那儿谁也没连 —— 它不会进任何提示词，而没有任何地方会报错。
   * 留一个不起作用的节点比没有更糟：人会以为改它有用。
   */
  it("一根线都没连的节点要报出来", () => {
    const orphan = toTemplate("有孤儿", [
      node("a", "prompt", { prompt: "火箭筒" }),
      node("b", "imageGen", { engine: "zimage" }),
      node("c", "videoGen", { engine: "h3" }),
      node("g", "group", { label: "框" }),
    ], [{ id: "e", source: "b", target: "c" } as CanvasEdge]);
    const got = templateNeeds(orphan, { loras: [], engines: ["zimage", "h3"] })
      .filter((n) => n.kind === "orphan");
    expect(got.map((n) => n.nodeId)).toEqual(["a"]);
    expect(got[0].label).toContain("不会进任何提示词");
  });

  it("引擎不在就标出来，同一个引擎只报一次", () => {
    expect(find("引擎 zimage")?.status).toBe("ok");
    expect(find("引擎 没这个引擎")?.status).toBe("missing");
    expect(needs.filter((n) => n.label === "引擎 zimage")).toHaveLength(1);
  });
});

describe("文件名", () => {
  it("中文留着 —— 工作流是给人看的", () => {
    expect(slugOf("首帧→视频→回填")).toBe("首帧→视频→回填");
  });
  it("路径分隔符和非法字符换掉，空名给个兜底", () => {
    expect(slugOf("a/b:c")).toBe("a_b_c");
    expect(slugOf("   ")).toMatch(/^工作流-\d+$/);
  });
});

// ── 接线 ──────────────────────────────────────────────────────────
describe("接线", () => {
  const panel = readFileSync(new URL("../ui/CanvasWorkflowPanel.tsx", import.meta.url), "utf8");
  it("面板存的是模板不是整张画布", () => {
    expect(panel).toContain("toTemplate(");
    expect(panel).not.toMatch(/canvasWorkflowSave\([^)]*nodes,\s*edges/);
  });
  it("载入时先算缺口清单", () => {
    expect(panel).toContain("templateNeeds(");
  });

  /**
   * **面板渲染了但到不了 = 没做。** 这个坑踩过（记忆：rendered-but-unreachable-ui）。
   */
  it("画布上真的有入口", () => {
    const view = readFileSync(new URL("../CanvasView.tsx", import.meta.url), "utf8");
    const dock = readFileSync(new URL("../ui/CanvasQuickActionBar.tsx", import.meta.url), "utf8");
    expect(view).toContain("<CanvasWorkflowPanel");
    expect(view).toContain("onToggleWorkflows=");
    expect(dock).toContain("onToggleWorkflows?.()");
  });

  /** 载入要真的把图摆上画布，不是只弹个清单。 */
  it("载入把节点和边都摆上去", () => {
    expect(panel).toContain("setNodes(nodes)");
    expect(panel).toContain("setEdges(t.edges)");
  });

  /**
   * **点完即关，和添加节点面板一样**（用户 2026-09-21）。
   *
   * 但缺口清单原来就在这个面板里 —— 直接关等于把它扔了。
   * 所以缺口改成写进对应节点的 `notes`：「还要填 2 张参考图」应该长在
   * 那个图片节点上，而不是一张关掉就没的清单里。
   */
  it("载入和点模板都自动关窗，缺口写到节点上", () => {
    expect(panel.match(/onClose\(\);/g)?.length ?? 0).toBeGreaterThanOrEqual(2);
    expect(panel).toContain("notes: mine");
    expect(panel).toContain("载入后还要：");
    // 面板里那段清单已经删掉，不留死代码
    expect(panel).not.toContain("还要你填这些");
  });
});
