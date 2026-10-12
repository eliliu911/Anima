import type { CanvasEdge, CanvasNode, CanvasReference, DepthCaptureNodeData, ImageGenNodeData, UpscaleNodeData, VideoGenNodeData } from "../types";
import { isRefMotion } from "./depthChain";
import { upscaleStale } from "./upscaleChain";
import { isReferenceMode } from "./videoMode";

/**
 * 点一下生成，**会连带跑哪些上游**。
 *
 * # 为什么要能先看见
 *
 * 画布上点生成会沿着连线把脏了的上游一并重跑 —— 这是对的，
 * 但用户看不见范围。一张 1024 的图几十秒，一段视频几十分钟，
 * 「我只是想重出这一张」结果把上游一条链全跑了一遍，代价差两个数量级。
 *
 * # 一个所有者
 *
 * 判据原来散在 `executeNodePipeline` 的三个 if 里。预览再抄一遍必然漂移，
 * 而漂移的表现是「预览说要跑 2 个，实际跑了 5 个」—— 比不给预览更糟。
 * 所以执行器改成调这里，两边读的是同一份判断。
 */

/**
 * 这个上游节点会不会被重跑。
 *
 * 两个回调**由调用方传进来**，不用模块级注入：它们住在 `dagExecutor` 里，
 * 而这个模块经 `videoMode` 被 `dagExecutor` 依赖着 —— 注入版本在模块
 * 初始化时就会撞上 TDZ（2026-09-20 当场踩到）。参数传递没有这个问题。
 */
export interface RerunDeps {
  /** 深度节点当前该用哪个源视频 */
  depthSourcePath: (id: string) => string | undefined;
  /** 图片节点当前输入的签名 */
  signatureOf: (id: string, nodes: CanvasNode[], edges: CanvasEdge[]) => string | undefined;
}

export function willRerun(
  node: CanvasNode,
  targetId: string,
  nodes: CanvasNode[],
  edges: CanvasEdge[],
  deps: RerunDeps,
): boolean {
  if (node.id === targetId) return false;
  const target = nodes.find((n) => n.id === targetId);

  // **绑定成参考图的上游节点不重跑。**
  // 判据读的是**推导出来的模式**，不是存下来的 `mode` 字段 —— 绑定那一步
  // 不再替用户写死 `r2v`，读存下来的字段会让上传的源图被当成「要重新生成」，
  // 白跑一次出图，而且把用户上传的那张覆盖掉。
  if (
    target?.type === "videoGen" &&
    (target.data.references as CanvasReference[] | undefined)?.some((r) => r.sourceNodeId === node.id) &&
    isReferenceMode(targetId, nodes, edges)
  ) return false;

  if (node.type === "depthCapture") {
    // 原片复刻两条拿原视频本身当参考，深度用不上 —— 跑一次白等几分钟
    if (target?.type === "videoGen" && isRefMotion((target.data as VideoGenNodeData).motionEngine)) return false;
    const d = node.data as DepthCaptureNodeData;
    return !d.depthVideoUrl || d.sourceVideoPath !== deps.depthSourcePath(node.id);
  }
  if (node.type === "upscale") {
    return !(node.data as UpscaleNodeData).outputUrl || upscaleStale(node, nodes, edges);
  }
  if (node.type === "imageGen" && !node.data.referenceOnly) {
    const d = node.data as ImageGenNodeData;
    // 签名对不上 = 上游输入变过，这张图已经不是按当前输入出的
    return !d.outputImageUrl || Boolean(d.lastExecutedSignature && d.lastExecutedSignature !== deps.signatureOf(node.id, nodes, edges));
  }
  return false;
}

export interface RerunPreview {
  /** 会重跑的上游，按执行顺序 */
  willRun: CanvasNode[];
  /** 有几个上游因为已经是新的而跳过 */
  skipped: number;
}

/** 给界面用的一句话摘要。`queue` 由调用方用 `resolveUpstreamPipeline` 算好。 */
export function previewRerun(
  queue: CanvasNode[],
  targetId: string,
  nodes: CanvasNode[],
  edges: CanvasEdge[],
  deps: RerunDeps,
): RerunPreview {
  const upstream = queue.filter((n) => n.id !== targetId);
  const willRun = upstream.filter((n) => willRerun(n, targetId, nodes, edges, deps));
  return { willRun, skipped: upstream.length - willRun.length };
}
