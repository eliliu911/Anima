import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import type { CanvasEdge, CanvasNode } from "../types";
import { collectLoras, isBakedIn, loraFits, upstreamLoraCount } from "./collectLoras";

const lora = (id: string, name?: string, strength = 1): CanvasNode =>
  ({ id, type: "lora", position: { x: 0, y: 0 }, data: { name, strength } }) as CanvasNode;
const gen = (id: string): CanvasNode =>
  ({ id, type: "imageGen", position: { x: 0, y: 0 }, data: { engine: "zimage", width: 1, height: 1 } }) as CanvasNode;
const edge = (s: string, t: string): CanvasEdge => ({ id: `${s}-${t}`, source: s, target: t }) as CanvasEdge;

describe("一个生成节点最终挂哪些 LoRA", () => {
  it("上游的 LoRA 节点会合进来", () => {
    const nodes = [lora("l1", "zimage/a.safetensors", 0.8), lora("l2", "zimage/b.safetensors", 0.5), gen("g")];
    const edges = [edge("l1", "g"), edge("l2", "g")];
    expect(collectLoras("g", nodes, edges)).toEqual([
      { name: "zimage/a.safetensors", strength: 0.8 },
      { name: "zimage/b.safetensors", strength: 0.5 },
    ]);
  });

  /** **不做两套数据。** 工具条里选的和连线挂的合成一份。 */
  it("和节点自己选的合成一份，同名只留一个", () => {
    const nodes = [lora("l1", "zimage/a.safetensors", 0.8), gen("g")];
    const edges = [edge("l1", "g")];
    const own = [
      { name: "zimage/a.safetensors", strength: 0.2 },
      { name: "zimage/c.safetensors", strength: 1 },
    ];
    const got = collectLoras("g", nodes, edges, own);
    expect(got).toHaveLength(2);
    // 上游优先 —— 它在画布上看得见，更可信
    expect(got[0]).toEqual({ name: "zimage/a.safetensors", strength: 0.8 });
    expect(got[1].name).toBe("zimage/c.safetensors");
  });

  /** 刚拖出来还没配名字是正常状态，不该报错也不该产生空条目。 */
  it("没配名字的 LoRA 节点跳过，但仍然算进数量", () => {
    const nodes = [lora("l1"), gen("g")];
    const edges = [edge("l1", "g")];
    expect(collectLoras("g", nodes, edges)).toEqual([]);
    expect(upstreamLoraCount("g", nodes, edges)).toBe(1);
  });

  /** `strength: 0` 是「挂上但先关掉」的对照用法，要留着。 */
  it("权重 0 的留着", () => {
    const nodes = [lora("l1", "zimage/a.safetensors", 0), gen("g")];
    expect(collectLoras("g", nodes, [edge("l1", "g")])).toEqual([{ name: "zimage/a.safetensors", strength: 0 }]);
  });

  it("别的节点连过来的不算", () => {
    const nodes = [gen("other"), gen("g")];
    expect(collectLoras("g", nodes, [edge("other", "g")])).toEqual([]);
  });
});

describe("挂不挂得上", () => {
  it("按 studio_loras 给的兼容引擎判", () => {
    expect(loraFits(["zimage"], "zimage")).toBe(true);
    expect(loraFits(["zimage"], "sdxl")).toBe(false);
    // 读不到兼容列表时不拦 —— 宁可让它试，也别凭空禁掉
    expect(loraFits(undefined, "sdxl")).toBe(true);
  });

  /**
   * **H3 的采样预设已经内置挂了 turbo LoRA。** 再选一次 WeeTodd 的
   * preflight 会拒 —— 而本机唯一的 H3 架构 LoRA 正好就是它，
   * 用户在视频节点上看到的就是这一个「选了会失败」的东西。
   */
  it("认得出预设已经内置的那个", () => {
    expect(isBakedIn("minimax_h3_turbo_v4_step600_ema_pruned_comfyui.safetensors")).toBe(true);
    expect(isBakedIn("zimage/Straight_hair_zimgt.safetensors")).toBe(false);
  });
});

/**
 * **算出来了还得有人用。**
 *
 * 这条是变异测试逼出来的：把执行器里那次 `collectLoras` 换回 `data`，
 * 上面所有单元测试照样全绿 —— 而实际后果是「LoRA 节点画了、连了、
 * 界面上看着挂上了，生成时一个都没发下去」，**完全静默**。
 *
 * 纯函数测试只能证明函数本身对；接没接上是另一件事，这个仓库为这一类
 * 问题修了一整天（建议挂错字段挡住整集出片、h3 嗓音不进指纹）。
 */
describe("接线", () => {
  it("执行器真的把上游 LoRA 节点合进去了", () => {
    const src = readFileSync(fileURLToPath(new URL("../engine/dagExecutor.ts", import.meta.url)), "utf8");
    const calls = src.match(/collectLoras\(nodeId, nodes, edges/g) ?? [];
    // 出图、出片、长镜头三条路各一次 —— 少一条就是「图片能挂、视频挂不上」
    expect(calls.length, "出图、出片或长镜头少接了一条").toBe(3);
    expect(src, "算出来却没传给 canvasLoraConfig").toContain("loras: collectLoras(nodeId, nodes, edges");
  });
});
