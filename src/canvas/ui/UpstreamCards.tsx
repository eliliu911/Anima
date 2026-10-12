import { useReactFlow } from "@xyflow/react";
import type { CanvasEdge, CanvasNode, CanvasReference, ImageGenNodeData, LoraNodeData, VideoGenNodeData } from "../types";
import type { VideoMode } from "../utils/videoMode";
import { ArrowLeftRight } from "lucide-react";
import { MediaThumb, pathFromMediaUrl } from "../../components/studio/MediaThumb";
import { nodeMedia } from "../utils/upscaleChain";
import { useCanvasStore } from "../store/useCanvasStore";
import { ImageIcon, Layers, Type, Video, X } from "./icons";

/**
 * 节点面板顶上那一排「连进来了什么」的小卡片 —— 照 LibTV（用户 2026-09-24）。
 *
 * # 为什么要有
 *
 * 以前只有**已经有图**的参考才列出来（一行一张：缩略图、编号、@、改名、替换、删除）；
 * 连着一个还没出图的图片节点，面板上什么都看不到，只能去看连线。
 * 现在每一个连进来的图 / 视频都是一张卡片：没出图的显示占位图标，编号和发给模型的 `<Picture N>` 一致。
 *
 * # 编号的顺序
 *
 * 和 `videoInputReferences` 同一个顺序：先是节点上直接绑的参考图，再按连线顺序的上游。
 * 还没出图的也占位置 —— 点生成时执行器先把它跑出来，出来之后它就在这个位置上。视频单独编号（`<Video N>`）。
 *
 * # 操作
 *
 * - 悬停出 ×：断开这条连线（连同绑在节点上的那条参考）
 * - 双击：画布移到那个上游节点
 * - 单击绑定的参考图：展开它的名字 / @ / 替换（`onSelect`，由外面决定展开什么）
 *
 * # 不只是图和视频（用户 2026-09-24：「所有节点，都作为设定卡片里的正方形小卡片显示」）
 *
 * 提示词、LoRA、音频、深度这些连进来的节点也各是一张卡（不编号 —— 它们不占 <Picture N>）。
 * 「风格」按钮里挑的 LoRA 也是一张风格卡（照 LibTV：它的「风格」就是 LoRA），× 就是取消挂载。
 */
export interface UpstreamCard {
  key: string;
  kind: "image" | "video" | "text" | "style" | "audio" | "other";
  /** 没有 = 上游还没出 */
  path?: string;
  label: string;
  /** 从哪个节点来的；直接上传 / 资产库的参考图没有 */
  sourceNodeId?: string;
  /** 绑在节点 `data.references` 上的那一条（有它才能改名、替换） */
  reference?: CanvasReference;
  /** 图片的 <Picture N> / 视频的 <Video N>；其余类型 0（不编号） */
  index: number;
  /** 风格卡：「风格」按钮里挑的 LoRA（在节点 data.loras 上，不是连线） */
  loraName?: string;
}

const MEDIA_TYPES = new Set(["imageGen", "character", "upscale", "videoGen"]);
const TEXT_TYPES = new Set(["prompt", "shot", "shotContext", "skill", "panorama"]);
const TYPE_NAME: Record<string, string> = {
  imageGen: "图片", character: "角色", upscale: "超分", videoGen: "视频", prompt: "提示词", shot: "分镜",
  shotContext: "场景设定", skill: "技能", panorama: "全景", lora: "风格", audio: "音频", depthCapture: "深度",
};
const shortName = (s: string) => s.split("/").pop()!.replace(/\.safetensors$/, "");

export function upstreamCards(nodeId: string, nodes: CanvasNode[], edges: CanvasEdge[]): UpstreamCard[] {
  const target = nodes.find((n) => n.id === nodeId);
  if (!target) return [];
  const out: Omit<UpstreamCard, "index">[] = [];
  const seenPaths = new Set<string>();
  const seenSources = new Set<string>();
  for (const ref of ((target.data as VideoGenNodeData).references ?? []) as CanvasReference[]) {
    const src = ref.sourceNodeId ? nodes.find((n) => n.id === ref.sourceNodeId) : undefined;
    const path = src ? nodeMedia(src)?.path : ref.path;
    if (path && seenPaths.has(path)) continue;
    if (path) seenPaths.add(path);
    if (ref.sourceNodeId) seenSources.add(ref.sourceNodeId);
    out.push({ key: `ref:${ref.path}`, kind: "image", path, label: ref.name || "参考图", sourceNodeId: ref.sourceNodeId, reference: ref });
  }
  for (const e of edges.filter((x) => x.target === nodeId)) {
    const src = nodes.find((n) => n.id === e.source);
    if (!src || seenSources.has(src.id) || src.type === "group") continue;
    seenSources.add(src.id);
    const type = src.type ?? "";
    const d = src.data as { name?: string; label?: string; prompt?: string };
    if (MEDIA_TYPES.has(type)) {
      const media = nodeMedia(src);
      if (media?.path && seenPaths.has(media.path)) continue;
      if (media?.path) seenPaths.add(media.path);
      const kind = type === "videoGen" || media?.kind === "video" ? "video" : "image";
      out.push({ key: `edge:${e.id}`, kind, path: media?.path, label: String(d.name || d.label || TYPE_NAME[type] || "上游"), sourceNodeId: src.id });
    } else if (type === "lora") {
      const ln = (src.data as LoraNodeData).name;
      out.push({ key: `edge:${e.id}`, kind: "style", label: ln ? shortName(ln) : "风格（还没选 LoRA）", sourceNodeId: src.id });
    } else if (TEXT_TYPES.has(type)) {
      const text = String(d.prompt || d.label || "").trim();
      out.push({ key: `edge:${e.id}`, kind: "text", label: `${TYPE_NAME[type]}${text ? `：${text.slice(0, 40)}` : ""}`, sourceNodeId: src.id });
    } else {
      out.push({ key: `edge:${e.id}`, kind: type === "audio" ? "audio" : "other", label: String(d.label || TYPE_NAME[type] || type), sourceNodeId: src.id });
    }
  }
  // 「风格」按钮里挑的 LoRA：挂在节点自己身上，没有连线
  for (const l of ((target.data as ImageGenNodeData).loras ?? []) as { name: string; strength: number }[]) {
    out.push({ key: `lora:${l.name}`, kind: "style", label: `${shortName(l.name)} · 强度 ${l.strength}`, loraName: l.name });
  }
  let img = 0, vid = 0;
  return out.map((c) => ({ ...c, index: c.kind === "video" ? ++vid : c.kind === "image" ? ++img : 0 }));
}

/** 断开一张卡片：连线和绑在节点上的参考一起去掉 */
export function removeUpstreamCard(nodeId: string, card: UpstreamCard) {
  const store = useCanvasStore.getState();
  if (card.loraName) {
    const node = store.nodes.find((n) => n.id === nodeId);
    const loras = ((node?.data as ImageGenNodeData | undefined)?.loras ?? []) as { name: string }[];
    store.updateNodeData(nodeId, { loras: loras.filter((l) => l.name !== card.loraName) });
    return;
  }
  if (card.sourceNodeId) {
    store.onEdgesChange(store.edges.filter((e) => e.target === nodeId && e.source === card.sourceNodeId).map((e) => ({ id: e.id, type: "remove" as const })));
  }
  const node = store.nodes.find((n) => n.id === nodeId);
  const refs = ((node?.data as VideoGenNodeData | undefined)?.references ?? []) as CanvasReference[];
  const left = refs.filter((r) => (card.sourceNodeId ? r.sourceNodeId !== card.sourceNodeId : r.path !== card.reference?.path));
  if (left.length !== refs.length) store.updateNodeData(nodeId, { references: left });
}

/**
 * 视频节点上，图片卡的角标写**这张图在这次生成里干什么**（照 LibTV）。
 *
 * - 图生视频：那一张是「首帧」
 * - 首尾帧：「首帧」「尾帧」，多出来的「不用」—— 按实际发出去的那两张路径判，不按猜
 * - 图片参考 / 全能参考 / 出图节点：编号（= 提示词里的 @图片N、发给模型的 <Picture N>）
 *
 * 以前一律只写编号，两张图时看不出是首尾帧还是参考（用户 2026-09-25）。
 */
export function cardRole(card: UpstreamCard, mode: VideoMode | undefined, frames: string[]): string {
  if (card.kind !== "image") return card.kind === "video" ? `V${card.index}` : "";
  if (mode === "i2v") return card.index === 1 ? "首帧" : "不用";
  if (mode === "fl2v") {
    // 卡片上是节点输出的写法，推导结果是发给后端的写法（可能一个是 anima-media:// 一个是本地路径）—— 统一了再比
    const at = card.path ? frames.map(pathFromMediaUrl).indexOf(pathFromMediaUrl(card.path)) : card.index - 1;
    return at === 0 ? "首帧" : at === 1 ? "尾帧" : "不用";
  }
  return String(card.index);
}

/**
 * 首尾帧对调：把这两张图的**连线顺序**换过来（首尾帧按顺序定，第 1 张开头、第 2 张结尾）。
 * 以前要把其中一张改名成带「尾帧」才能指定，用户看不出来。两张都是连线进来的、或都绑在节点上才能换；
 * 名字里带「尾帧」的那张按名字定，不看顺序（`inferVideoMode`）。
 */
export function swapFrameCards(nodeId: string, a: UpstreamCard, b: UpstreamCard): boolean {
  const store = useCanvasStore.getState();
  if (a.key.startsWith("edge:") && b.key.startsWith("edge:")) {
    const ia = store.edges.findIndex((e) => `edge:${e.id}` === a.key);
    const ib = store.edges.findIndex((e) => `edge:${e.id}` === b.key);
    if (ia < 0 || ib < 0) return false;
    const next = [...store.edges];
    [next[ia], next[ib]] = [next[ib], next[ia]];
    store.setEdges(next);
    return true;
  }
  if (a.reference && b.reference) {
    const node = store.nodes.find((n) => n.id === nodeId);
    const refs = [...(((node?.data as VideoGenNodeData | undefined)?.references ?? []) as CanvasReference[])];
    const ia = refs.findIndex((r) => r.path === a.reference!.path);
    const ib = refs.findIndex((r) => r.path === b.reference!.path);
    if (ia < 0 || ib < 0) return false;
    [refs[ia], refs[ib]] = [refs[ib], refs[ia]];
    store.updateNodeData(nodeId, { references: refs });
    return true;
  }
  return false;
}

export function UpstreamCards({ nodeId, locked, selectedKey, onSelect, mode, frames = [] }: {
  nodeId: string;
  locked?: boolean;
  selectedKey?: string;
  onSelect?: (card: UpstreamCard) => void;
  /** 视频节点的当前模式：角标写首帧 / 尾帧（见 `cardRole`）。出图节点不传，照旧写编号 */
  mode?: VideoMode;
  /** 首尾帧模式下实际发出去的两张（推导结果的 images） */
  frames?: string[];
}) {
  const nodes = useCanvasStore((s) => s.nodes);
  const edges = useCanvasStore((s) => s.edges);
  const flow = useReactFlow();
  const cards = upstreamCards(nodeId, nodes, edges);
  if (cards.length === 0) return null;

  const focus = (card: UpstreamCard) => {
    const n = card.sourceNodeId ? flow.getNode(card.sourceNodeId) : undefined;
    if (!n) return;
    const w = n.measured?.width ?? 440, h = n.measured?.height ?? 300;
    void flow.setCenter(n.position.x + w / 2, n.position.y + h / 2, { zoom: flow.getZoom(), duration: 300 });
  };

  return (
    <div className="flex flex-wrap items-center gap-2" aria-label="连进来的素材">
      {cards.map((c) => {
        const Icon = { image: ImageIcon, video: Video, text: Type, style: Layers, audio: Video, other: Layers }[c.kind];
        const badge = c.kind === "video" || c.kind === "image" ? cardRole(c, mode, frames) : null;
        const word = { image: "", video: "", text: "文本", style: "风格", audio: "音频", other: "" }[c.kind];
        const on = selectedKey === c.key;
        return (
          <div
            key={c.key}
            role="button" tabIndex={0}
            title={`${badge ? (c.kind === "video" ? `视频${c.index} · ` : `图片${c.index}${badge !== String(c.index) ? `（${badge}）` : ""} · 提示词里 @图片${c.index} · `) : ""}${c.label}${!c.path && (c.kind === "image" || c.kind === "video") ? "（上游还没出）" : ""}${c.sourceNodeId ? " · 双击可聚焦至节点" : ""}`}
            onClick={() => onSelect?.(c)}
            onDoubleClick={() => focus(c)}
            className={`group relative h-12 w-12 shrink-0 cursor-pointer overflow-visible rounded-lg border bg-white/5 ${on ? "border-white/60" : "border-white/10 hover:border-white/30"}`}
          >
            <div className="h-full w-full overflow-hidden rounded-lg">
              {c.path
                ? <MediaThumb path={c.path} kind={c.kind === "video" ? "video" : "image"} className="h-full w-full object-cover" />
                : <div className="flex h-full w-full flex-col items-center justify-center gap-0.5 text-white/45">
                    <Icon className="h-4 w-4" />{word && <span className="text-[9px] leading-none">{word}</span>}
                  </div>}
            </div>
            {badge && (
              <span className="absolute -left-1 -top-1 flex h-4 min-w-4 items-center justify-center rounded-full bg-zinc-950 px-1 text-[10px] font-medium text-white ring-1 ring-white/20">
                {badge}
              </span>
            )}
            {!locked && (
              <button
                type="button" aria-label={`断开 ${c.label}`} title="断开这个输入"
                onClick={(e) => { e.stopPropagation(); removeUpstreamCard(nodeId, c); }}
                className="absolute -right-1.5 -top-1.5 hidden h-4 w-4 items-center justify-center rounded-full bg-zinc-800 text-white/80 ring-1 ring-white/30 hover:bg-red-500 hover:text-white group-hover:flex"
              >
                <X width={10} height={10} />
              </button>
            )}
          </div>
        );
      })}
      {mode === "fl2v" && !locked && (() => {
        const imgs = cards.filter((c) => c.kind === "image");
        const byName = imgs.some((c) => /尾帧|末帧/.test(c.label));
        if (imgs.length < 2 || byName) return null;
        return (
          <button type="button" title="把首帧和尾帧对调（按连线顺序：第 1 张开头、第 2 张结尾）"
            onClick={() => swapFrameCards(nodeId, imgs[0], imgs[1])}
            className="nodrag nopan flex h-7 items-center gap-1 rounded-md border border-white/10 px-2 text-[11px] text-white/60 hover:bg-white/10 hover:text-white">
            <ArrowLeftRight className="h-3 w-3" />对调首尾
          </button>
        );
      })()}
    </div>
  );
}
