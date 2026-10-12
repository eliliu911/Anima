import type { AssetView } from "../../api/assets";
import { nodeMedia } from "./upscaleChain";
import type { CanvasReference, CanvasNode, CanvasEdge } from "../types";
import { MAX_IMAGE_REFERENCES } from "./imageMode";

/**
 * 视频节点最多绑几张参考图。
 *
 * **9 张，和 H3 规格、分镜出片（`video_tool::MAX_REFERENCE_IMAGES`）一致。**
 * 以前画布写死 4 —— 同一个人在工作区能挂 9 张，到画布第 5 张就报错。
 */
export const MAX_VIDEO_REFERENCES = 9;

export function referenceGraph(nodes: CanvasNode[], edges: CanvasEdge[], targetId: string, ref: CanvasReference, replacePath?: string) {
  const target = nodes.find((n) => n.id === targetId);
  if (!target) throw new Error("目标节点已删除");
  if (target.data.isGenerating) throw new Error("目标节点正在生成");
  const current = (target.data.references ?? []) as CanvasReference[];
  if (replacePath && !current.some((r) => r.path === replacePath)) throw new Error("参考已移除");
  const remaining = replacePath ? current.filter((r) => r.path !== replacePath) : current;
  const checked = addReference(remaining, ref, target.type === "videoGen" ? MAX_VIDEO_REFERENCES : MAX_IMAGE_REFERENCES);
  if (checked === remaining) return { nodes, edges };
  let source = ref.sourceNodeId ? nodes.find((n) => n.id === ref.sourceNodeId) : undefined;
  if (ref.sourceNodeId && !source) throw new Error("参考节点已删除");
  if (source) {
    const visited = new Set<string>();
    const visit = (id: string): boolean => {
      if (id === source!.id) return true;
      if (visited.has(id)) return false;
      visited.add(id);
      return edges.filter((e) => e.source === id).some((e) => visit(e.target));
    };
    if (visit(targetId)) throw new Error("参考连线不能形成循环");
  } else {
    source = { id: `reference-${crypto.randomUUID()}`, type: "imageGen",
      position: { x: target.position.x - 500, y: target.position.y + current.length * 320 },
      data: { engine: "", width: 1280, height: 720, seed: 0, referenceOnly: true,
        label: ref.name, outputImageUrl: ref.path, outputImages: [ref.path] } };
    nodes = [...nodes, source];
  }
  const bound = { ...ref, sourceNodeId: source.id };
  const references = replacePath ? current.map((r) => r.path === replacePath ? bound : r) : [...current, bound];
  const old = current.find((r) => r.path === replacePath);
  edges = edges.filter((e) => !(old?.sourceNodeId && e.source === old.sourceNodeId && e.target === targetId));
  if (!edges.some((e) => e.source === source.id && e.target === targetId)) {
    edges = [...edges, { id: `reference-${source.id}-${targetId}`, source: source.id, target: targetId,
      sourceHandle: source.type === "character" ? "char-out" : "image-out", targetHandle: "prompt-in",
      data: { reference: true }, style: { stroke: "#a1a1aa", strokeWidth: 1.5 } }];
  }
  // **绑一张图不等于参考生视频。**
  //
  // 这里以前无条件写 `mode: "r2v"` —— 连一张首帧也被标成参考生视频，
  // 而参考生视频只支持 H3、还要求多张带名字的身份参考。
  // 用户看到的就是「我明明只连了一张图，怎么变成参考模式了」。
  //
  // 现在只在**没设过**时写 `auto`，让 `inferVideoMode` 从连线推导。
  // **不覆盖用户明确选过的模式** —— 连线改了而选择过时，由推导报出冲突
  // （`ModeInference.overridden`），而不是替他悄悄改掉。
  return { nodes: nodes.map((n) => n.id === targetId ? { ...n, data: { ...n.data, references,
    ...(n.type === "videoGen" && !n.data.mode ? { mode: "auto" } : {}) } } as CanvasNode : n), edges };
}

export function nodeReference(node: CanvasNode): CanvasReference {
  const path = node.type === "imageGen" ? node.data.outputImageUrl || node.data.references?.[0]?.path
    : node.type === "character" ? node.data.avatarUrl
    : node.type === "upscale" && nodeMedia(node)?.kind === "image" ? node.data.outputUrl : undefined;
  if (typeof path !== "string" || !path) throw new Error("请选择已有图片的图像或角色节点");
  return { path, name: String(node.data.name || node.data.label || "画布图片"), sourceNodeId: node.id };
}

export function referencePatch(node: CanvasNode, reference: CanvasReference) {
  // 同上：交给推导，不在绑定这一步替用户定模式，也不覆盖他选过的
  return { references: addReference((node.data.references ?? []) as CanvasReference[], reference, node.type === "videoGen" ? 4 : 1),
    ...(node.type === "videoGen" && !node.data.mode ? { mode: "auto" } : {}) };
}

export function assetReference(asset: AssetView): CanvasReference {
  const index = asset.unit.image_variants.findIndex((v) => v.id === asset.unit.selected_image_id);
  const path = asset.image_paths[index];
  if (index < 0 || !path) throw new Error("资产没有可用的选中图片，请先到资产库选择图片版本");
  return { path, name: `${asset.name} · ${asset.scope_label}`, assetId: asset.id,
    scope: asset.scope, variantId: asset.unit.image_variants[index].id };
}

export function addReference(current: CanvasReference[], next: CanvasReference, limit: number) {
  if (current.some((r) => r.path === next.path)) return current;
  if (current.length >= limit) throw new Error(`当前模式最多绑定 ${limit} 张图片，请先移除旧图`);
  return [...current, next];
}
