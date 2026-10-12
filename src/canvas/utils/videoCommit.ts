import type { CanvasEdge, CanvasNode, VideoGenNodeData } from "../types";
import { upstreamShot, type ShotTarget } from "./shotLink";

/**
 * 画布上的一段视频能不能收回某一镜。
 *
 * # 它补的断口
 *
 * 图片早就能「收为首帧」（`CanvasImageAssetToolbar`），视频不能 ——
 * 在画布上把一镜修好之后，只能去 Finder 里找文件再手动放进分集目录，
 * **而放错地方不报错**。
 */
export interface VideoCommitPlan {
  ok: true;
  shot: ShotTarget;
  /** 要收的那个文件 */
  path: string;
  /** 上一次实际发出去的正文。**没跑过就是空** —— 不拿编辑框里的顶替 */
  prompt: string;
  engine: string;
  /** 实际用的参考图，按顺序。没跑过就是空 */
  pictures: string[];
  /** 要说给用户听的话。比如「这段片子不是在画布上出的」 */
  note?: string;
}

export interface VideoCommitBlocked {
  ok: false;
  why: string;
}

/**
 * 能收就给全套参数，不能收就说为什么。
 *
 * **拿 `lastRun` 而不是 `data.prompt`。** 编辑框里的内容和真正发出去的
 * 差好几步（上游聚合、`@提及`编译、`<Picture N>` 映射、LoRA 激活词）。
 * 存 `data.prompt` 会写出一条**看起来完整、其实是假的**记录 ——
 * 而回看历史 take 时唯一想知道的就是「当时到底发了什么」。
 */
export function planVideoCommit(
  nodeId: string,
  nodes: CanvasNode[],
  edges: CanvasEdge[],
): VideoCommitPlan | VideoCommitBlocked {
  const node = nodes.find((n) => n.id === nodeId);
  if (!node || node.type !== "videoGen") return { ok: false, why: "这不是视频节点" };
  const d = node.data as VideoGenNodeData;
  const path = d.outputVideoUrl;
  if (!path) return { ok: false, why: "这个节点还没有视频" };
  if (d.isGenerating) return { ok: false, why: "还在出片，等它跑完" };

  const shot = upstreamShot(nodeId, nodes, edges);
  if (!shot) return { ok: false, why: "上游没有分镜节点 —— 接一个进来才知道收到哪一镜" };

  // **产物和记录要对得上。** 换过主选（画册里点了另一条）之后，
  // `lastRun` 记的仍是那一批的正文，但具体哪一条是哪一次分不出来 ——
  // 这种情况下记正文没问题（同一批同一份提示词），记参考图也没问题。
  const run = d.lastRun;
  return {
    ok: true,
    shot,
    path,
    prompt: run?.prompt ?? "",
    engine: run?.engine ?? d.engine ?? "",
    pictures: run?.images ?? [],
    // 上传进来的、或者旧节点：老实说这条 take 没有出处，别装作有
    note: run ? undefined : "这段片子不是在这个节点上出的（上传或旧数据），收进去的那一「次」不会带提示词",
  };
}
