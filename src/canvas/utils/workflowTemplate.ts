import type { CanvasEdge, CanvasNode, CanvasReference } from "../types";

/**
 * 画布 → 可复用的工作流模板。
 *
 * # 保存工作流 ≠ 保存画布
 *
 * ComfyUI 的工作流能原样复用，是因为**它的节点只有参数，没有「这一集第 7 镜」**。
 * 我们的节点有。把整张图存下来再载进另一集，会带进去四类东西，
 * **每一类都不报错**：
 *
 * | 带进去的 | 后果 |
 * |---|---|
 * | 分镜节点的 `projectId/episodeId/shotId` | 载进 E07 仍指着 E05 那一镜 |
 * | `outputImageUrl` / `outputVideoUrl` / `longTakeUrl` | 上一次的产物，看着像这次出的 |
 * | `CanvasReference.path`（**绝对路径**） | 指着旧集的文件 |
 * | `error` / `notes` | 陈旧的错误横幅 |
 *
 * 所以模板**只留接法和参数**，产物、绑定、参考图路径一律剥掉。
 * 载入时靠 {@link templateNeeds} 把缺口列出来，填齐才能跑 ——
 * 不列的话失败的样子是「载入显示成功、一跑全是默认值」。
 */

/** 产物字段：跑出来的东西，一律不进模板。 */
const PRODUCT_KEYS = [
  "outputImages", "outputImageUrl", "outputVideos", "outputVideoUrl",
  "longTakeUrl", "longTakeRunning", "outputUrl", "depthVideoUrl",
  "motionFirstFrame", "sourceVideoPath", "sourcePath", "requestedSourcePath",
  "lastRun", "lastExecutedSignature", "isGenerating", "progress",
  "error", "notes", "referenceError", "referenceLoading", "loading",
  "warnings", "unresolved", "dialogueWarning",
] as const;

/** 和「这一集这一镜」绑死的字段。 */
const BINDING_KEYS = ["projectId", "episodeId", "shotId", "context", "sceneAssetId", "avatarUrl"] as const;

export interface WorkflowTemplate {
  version: 1;
  name: string;
  nodes: CanvasNode[];
  edges: CanvasEdge[];
  /** 哪些节点要参考图，以及当初挂的是谁（只留名字当提示，不留路径） */
  slots: { nodeId: string; names: string[] }[];
  savedAt: number;
}

/** 把当前画布（或选中的一部分）收成模板。 */
export function toTemplate(name: string, nodes: CanvasNode[], edges: CanvasEdge[]): WorkflowTemplate {
  const ids = new Set(nodes.map((n) => n.id));
  const slots: WorkflowTemplate["slots"] = [];
  const clean = nodes.map((n) => {
    const d = { ...(n.data as Record<string, unknown>) };
    for (const k of PRODUCT_KEYS) delete d[k];
    for (const k of BINDING_KEYS) delete d[k];
    const refs = (n.data as { references?: CanvasReference[] }).references;
    if (refs?.length) {
      // **指向本图里另一个节点的绑定要留下 —— 那是接法，不是素材。**
      //
      // 首尾帧就靠它：视频节点上绑两条 `{sourceNodeId, name}`，其中一条叫
      // 「尾帧」，`inferVideoMode` 据此推出 fl2v。这种绑定里没有路径、
      // 没有资产 id，换个项目照样成立 —— 一起剥掉的话首尾帧的模板存不下来
      // （2026-09-21 摆首尾帧工作流时发现）。
      const inner = refs.filter((r) => r.sourceNodeId && ids.has(r.sourceNodeId) && !r.assetId);
      // 其余的（指着具体文件的）只留名字当提示，路径一律不带：
      // 绝对路径载进另一集就指着旧集的文件，而那张图还在，所以不报错。
      const outer = refs.filter((r) => !inner.includes(r));
      if (outer.length) slots.push({ nodeId: n.id, names: outer.map((r) => r.name).filter(Boolean) });
      if (inner.length) d.references = inner.map((r) => ({ sourceNodeId: r.sourceNodeId, path: "", name: r.name }));
      else delete d.references;
    }
    // 分镜节点：绑定剥掉了，缓存的提示词也得剥 —— 它是那一镜的正文
    if (n.type === "shot") { d.frame = ""; d.label = ""; }
    return { ...n, data: d, selected: false, dragging: false } as CanvasNode;
  });
  return {
    version: 1,
    name,
    nodes: clean,
    // 只留两端都在的边
    edges: edges.filter((e) => ids.has(e.source) && ids.has(e.target)).map((e) => ({ ...e, selected: false })),
    slots,
    savedAt: Date.now(),
  };
}

export type NeedStatus = "ok" | "missing";

export interface TemplateNeed {
  kind: "shot" | "reference" | "lora" | "engine" | "orphan";
  /** 给人看的一句话 */
  label: string;
  status: NeedStatus;
  /** 要填这个东西的节点 */
  nodeId?: string;
}

/**
 * 载入前先列出这套工作流要什么。**填齐才该让它跑。**
 *
 * 不列的话失败的样子是「载入显示成功、一跑全是默认值」——
 * 这正是这个项目的主病：任务状态是成功，结果是错的。
 */
export function templateNeeds(
  t: WorkflowTemplate,
  have: { loras: string[]; engines: string[] },
): TemplateNeed[] {
  const out: TemplateNeed[] = [];
  for (const n of t.nodes) {
    const d = n.data as Record<string, unknown>;
    if (n.type === "shot") {
      out.push({ kind: "shot", label: "挂一镜（分镜节点还没绑）", status: "missing", nodeId: n.id });
    }
    if (n.type === "lora") {
      const name = String(d.name ?? d.loraName ?? "");
      if (name) {
        out.push({
          kind: "lora", nodeId: n.id, label: `LoRA ${name}`,
          status: have.loras.includes(name) ? "ok" : "missing",
        });
      }
    }
    const engine = typeof d.engine === "string" ? d.engine : "";
    if (engine && !out.some((o) => o.kind === "engine" && o.label.endsWith(engine))) {
      // **引擎那条也要带 nodeId。** 缺口是写到节点上给人看的，
      // 没有 nodeId 的那几条就落不到任何地方，等于没提示。
      out.push({ kind: "engine", nodeId: n.id, label: `引擎 ${engine}`, status: have.engines.includes(engine) ? "ok" : "missing" });
    }
  }
  // **孤立节点：一根线都没连的，什么都不做。**
  //
  // 2026-09-21 用户在自己的工作流里当场看出来一个：一段写着「火箭筒」的
  // 文本节点悬在那儿，谁也没连 —— 它不会进任何提示词，而**没有任何地方会报错**。
  // 留一个不起作用的节点比没有更糟：人会以为改它有用。
  //
  // 分组节点除外，它本来就是个框。
  const linked = new Set(t.edges.flatMap((e) => [e.source, e.target]));
  for (const n of t.nodes) {
    if (n.type === "group" || linked.has(n.id)) continue;
    const label = (n.data as { label?: string }).label;
    out.push({
      kind: "orphan", nodeId: n.id, status: "missing",
      label: `「${label || n.type}」一根线都没连 —— 它不会进任何提示词`,
    });
  }

  for (const s of t.slots) {
    if (!s.names.length) continue;
    out.push({
      kind: "reference", nodeId: s.nodeId,
      label: `${s.names.length} 张参考图（原来挂的是 ${s.names.join("、")}）`,
      status: "missing",
    });
  }
  return out;
}

/** 文件名。中文留着 —— 工作流是给人看的，`wf_1758...` 谁也认不出来。 */
export function slugOf(name: string): string {
  const s = name.trim().replace(/[\/\\:*?"<>|]/g, "_").slice(0, 60);
  return s || `工作流-${Date.now()}`;
}
