import type { CanvasEdge, CanvasNode, LoraNodeData } from "../types";

export interface LoraPick {
  name: string;
  strength: number;
}

/**
 * 一个生成节点最终挂哪些 LoRA。
 *
 * **上游的 LoRA 节点 + 节点自己工具条里选的，合成一份。**
 * 做成两套数据的话必然出现「界面显示挂了 A，实际发的是 B」——
 * 这个仓库刚为同一类问题（`continues_previous` / `continuity`）收敛过一次。
 *
 * 规则：
 * - 同名只留一个，**上游节点优先**（它在画布上看得见，更可信）；
 * - `strength <= 0` 的留着但不算激活词 —— 它是「挂上但先关掉」的对照用法；
 * - 名字没选的 LoRA 节点直接跳过，不报错（刚拖出来还没配是正常状态）。
 */
export function collectLoras(
  nodeId: string,
  nodes: CanvasNode[],
  edges: CanvasEdge[],
  own: LoraPick[] = [],
): LoraPick[] {
  const upstream = edges
    .filter((e) => e.target === nodeId)
    .map((e) => nodes.find((n) => n.id === e.source))
    .filter((n): n is CanvasNode => !!n && n.type === "lora")
    .map((n) => n.data as LoraNodeData)
    .filter((d) => (d.name ?? "").trim())
    .map((d) => ({ name: d.name!.trim(), strength: d.strength }));

  const out: LoraPick[] = [];
  for (const p of [...upstream, ...own]) {
    if (!out.some((x) => x.name === p.name)) out.push(p);
  }
  return out;
}

/** 这个生成节点上游连了几个 LoRA 节点（含没配名字的）。界面上标数量用。 */
export function upstreamLoraCount(nodeId: string, nodes: CanvasNode[], edges: CanvasEdge[]): number {
  return edges.filter((e) => e.target === nodeId).filter((e) => nodes.find((n) => n.id === e.source)?.type === "lora").length;
}

/**
 * 这个 LoRA 挂不挂得上这个引擎。**在节点上当场标红，不等到点生成才报。**
 *
 * `engines` 是 `studio_loras` 给的兼容引擎列表 —— 它由张量键名判定的架构
 * 推出来（见 `studio::LoraTarget::engines`），不是按文件名猜的。
 */
export function loraFits(engines: string[] | undefined, engine: string): boolean {
  return !engines || engines.includes(engine);
}

/**
 * H3 的采样预设**已经内置挂了 turbo LoRA**，再选一次是重复挂载。
 *
 * `WeeToddH3ValidatedSamplingPreset` 的第二个输出就是它。WeeTodd 的 preflight
 * 会拒绝重复挂载 —— 但那是出片时才报，而本机 `models/loras/` 下**唯一**的
 * H3 架构 LoRA 正好就是它，所以用户在视频节点上打开 LoRA 面板，
 * 看到的就是这一个「选了会失败」的东西。
 */
export const BAKED_IN_LORAS = ["minimax_h3_turbo"];

export function isBakedIn(name: string): boolean {
  const base = name.split("/").pop() ?? name;
  return BAKED_IN_LORAS.some((w) => base.toLowerCase().includes(w));
}
