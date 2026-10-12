import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import type { CanvasEdge, CanvasNode } from "../types";
import { auditVideoNode, ignoredInputs } from "./runAudit";

const n = (id: string, type: string, data: Record<string, unknown> = {}): CanvasNode =>
  ({ id, type, position: { x: 0, y: 0 }, data } as CanvasNode);
const e = (s: string, t: string): CanvasEdge => ({ id: `${s}>${t}`, source: s, target: t } as CanvasEdge);

const roleOf = (nodes: CanvasNode[], edges: CanvasEdge[], srcId: string, loraEngines?: Record<string, string[] | undefined>) =>
  auditVideoNode("v", nodes, edges, loraEngines).inputs.find((i) => i.nodeId === srcId);

describe("哪些连线白连了", () => {
  /**
   * **这一条是整个预检存在的理由。**
   * 文生视频不吃图，但连线看起来成功 —— 跑完才发现那张图根本没发出去。
   */
  it("文生模式下连进来的图不会发出去，要说出来", () => {
    const nodes = [n("v", "videoGen", { engine: "h3", mode: "t2v" }), n("i", "imageGen", { outputImageUrl: "/a.png" })];
    const r = roleOf(nodes, [e("i", "v")], "i");
    expect(r?.used).toBe(false);
    expect(r?.role).toContain("不会发出去");
  });

  /**
   * **「上游还没出图」和「文生不吃图」是两件事。**
   *
   * 图还没生成时 `videoInputReferences` 拿不到路径，模式就推成 t2v ——
   * 照那个报「这张图不会发出去」是**误报**：等图出来它就是 i2v 了。
   * 用户会以为连线连错了去改图，而图本来就是对的。
   * 2026-09-21 摆长镜头工作流时被我们自己的预检抓出来的。
   */
  /** 2026-09-24 起：点生成时执行器会先把上游这张图跑出来，所以它**会用上** —— 不再报成「连了但没用上」 */
  it("上游的图还没出：算用上（首帧），说的是「还没出，点生成时先跑它」而不是「不会发出去」", () => {
    const nodes = [n("v", "videoGen", { engine: "h3" }), n("i", "imageGen", {})];
    const r = roleOf(nodes, [e("i", "v")], "i");
    expect(r?.used).toBe(true);
    expect(r?.role).toContain("还没出");
    expect(r?.role).toContain("首帧");
    expect(r?.role).not.toContain("不会发出去");
  });

  it("图生模式下同一张图就是首帧，正常生效", () => {
    const nodes = [n("v", "videoGen", { engine: "h3" }), n("i", "imageGen", { outputImageUrl: "/a.png" })];
    expect(roleOf(nodes, [e("i", "v")], "i")?.used).toBe(true);
  });

  /**
   * 视频连视频这条边**什么都不传**。
   *
   * 它曾经是长镜头的窗口链；2026-09-21 长镜头改成「一个节点一张窗口表」
   * 之后就彻底没用了。**旧画布上这种线还在**，而它现在一声不吭地什么也不做
   * —— 预检必须把它报出来，不管两头的引擎和时长长什么样。
   */
  it("视频连视频一律是白连，并说清长镜头改到哪去了", () => {
    const same = [n("v", "videoGen", { engine: "h3", durationSecs: 3 }), n("u", "videoGen", { engine: "h3", durationSecs: 3 })];
    const r = roleOf(same, [e("u", "v")], "u");
    expect(r?.used).toBe(false);
    expect(r?.role).toContain("在一个节点里加窗口");
    // 引擎/时长不同也是同一句 —— 这条边的命运和它们无关
    const diff = [n("v", "videoGen", { engine: "h3", durationSecs: 3 }), n("u", "videoGen", { engine: "ltx", durationSecs: 9 })];
    expect(roleOf(diff, [e("u", "v")], "u")?.role).toBe(r?.role);
  });

  /**
   * 接上深度会**强制**切动作迁移（`inferVideoMode` 的早退），
   * 所以「连了深度却没用上」这种情况不存在 —— 预检里也没有那个分支，
   * 到不了的分支比没有分支更糟。
   */
  it("深度一接上就强制切动作迁移，不存在白连", () => {
    const nodes = [n("v", "videoGen", { engine: "h3", mode: "t2v" }), n("d", "depthCapture", { depthVideoUrl: "/d.mp4" }), n("i", "imageGen", { outputImageUrl: "/p.png" })];
    const a = auditVideoNode("v", nodes, [e("d", "v"), e("i", "v")]);
    expect(a.mode).toBe("motion");
    expect(a.inputs.find((x) => x.nodeId === "d")?.used).toBe(true);
  });

  /**
   * 全景取景 / 导演世界的截图是靠它们**往下游新建图片节点**送的
   * （`addImageGroup`）。直连过来 `aggregateNodeInputs` 一个分支都不匹配，
   * 收不到任何东西 —— 而且完全不报错。
   */
  it("全景取景直连视频节点收不到图", () => {
    const nodes = [n("v", "videoGen", { engine: "h3" }), n("p", "panoViewer", { outputImageUrl: "/shot.png" })];
    const r = roleOf(nodes, [e("p", "v")], "p");
    expect(r?.used).toBe(false);
    expect(r?.role).toContain("图片节点");
  });

  it("没有图的角色节点，在参考生视频里贡献不了任何东西", () => {
    const nodes = [
      n("v", "videoGen", { engine: "h3", mode: "r2v", references: [{ path: "/a.png", name: "甲" }, { path: "/b.png", name: "乙" }] }),
      n("c", "character", { name: "丙", appearancePrompt: "长发" }),
    ];
    const r = roleOf(nodes, [e("c", "v")], "c");
    expect(r?.used).toBe(false);
    expect(r?.role).toContain("没有图");
  });
});

describe("确定会出错的才标成冲突", () => {
  /** 验收标准：SDXL 的 LoRA 接到别的引擎上，当场说不行。 */
  it("LoRA 架构对不上当场报，不等跑完", () => {
    const nodes = [n("v", "videoGen", { engine: "h3" }), n("l", "lora", { name: "sdxl/某个.safetensors" })];
    const a = auditVideoNode("v", nodes, [e("l", "v")], { "sdxl/某个.safetensors": ["sdxl"] });
    expect(a.conflicts.join()).toContain("挂不上 h3");
    expect(a.conflicts.join()).toContain("不报错");
  });

  it("查不到这个 LoRA 的架构时不乱报 —— 宁可不报也不要报错的", () => {
    const nodes = [n("v", "videoGen", { engine: "h3" }), n("l", "lora", { name: "未知.safetensors" })];
    expect(auditVideoNode("v", nodes, [e("l", "v")], {}).conflicts).toEqual([]);
  });

  it("参考生视频不是 H3 就出不了", () => {
    const nodes = [n("v", "videoGen", { engine: "ltx", mode: "r2v", references: [{ path: "/a.png" }, { path: "/b.png" }] })];
    expect(auditVideoNode("v", nodes, []).conflicts.join()).toContain("H3");
  });

  it("没有问题时冲突是空的 —— 不制造噪音", () => {
    const nodes = [n("v", "videoGen", { engine: "h3" }), n("p", "prompt", { prompt: "一段词" })];
    const a = auditVideoNode("v", nodes, [e("p", "v")]);
    expect(a.conflicts).toEqual([]);
    expect(ignoredInputs(a)).toEqual([]);
  });
});

// ── 接线 ──────────────────────────────────────────────────────────
describe("接线", () => {
  it("视频面板真的把预检显示出来了", () => {
    const panel = readFileSync(new URL("../nodes/panels/VideoOperationPanel.tsx", import.meta.url), "utf8");
    expect(panel).toContain("auditVideoNode(");
    expect(panel).toContain("ignoredInputs(");
  });
});
