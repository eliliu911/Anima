import { nodeTaskTitle } from "../utils/nodeTaskTitle";
import { useCanvasStore } from "../store/useCanvasStore";
import { inferCurrentResolution } from "../ui/CanvasVideoSettingsPopover";
import type {
  CanvasNode,
  CanvasEdge,
  PromptNodeData,
  CharacterNodeData,
  ImageGenNodeData,
  VideoGenNodeData,
  ShotContextNodeData,
  ShotNodeData,
  SkillNodeData,
  PanoramaNodeData,
} from "../types";
import { promptText } from "../../shared/shotLanguage";
import { shotPromptParts } from "../utils/shotLink";
import { studioCapabilities } from "../../api/studio";
import { resolveCanvasEngine, canonicalCanvasEngine, canvasEngines } from "../utils/modelCatalog";
import { toViewpoints, viewsFromContract } from "../../components/studio/assets/sceneViews";
import { pathFromMediaUrl } from "../../components/studio/MediaThumb";
import { compileCameraMoves } from "../utils/cameraMoves";
import { prefersChinese } from "../../shared/shotLanguage";
import { nodeReference, MAX_VIDEO_REFERENCES } from "../utils/referenceBindings";
import { canvasLoraConfig } from "../utils/loraConfig";
import { collectLoras } from "../utils/collectLoras";
import { isLongTake, planLongTake } from "../utils/longTakeWindows";
import { effectiveQuality } from "../utils/h3Quality";
import { willRerun, type RerunDeps } from "../utils/rerunScope";
import { compileAudioMentions, compilePromptMentions } from "../utils/referenceMentions";
import { audioLengthNote, emptyUpstreamAudios, MAX_VIDEO_AUDIOS, videoInputAudios } from "../utils/audioClips";
import { inferVideoMode, isReferenceMode } from "../utils/videoMode";
import { MIN_VIDEO_SECS } from "../utils/videoLimits";
import { depthSourceVideo, effectiveMotionEngine, isRefMotion, upstreamDepth, videoSourceOf } from "../utils/depthChain";
import { nodeMedia, upscaleSource } from "../utils/upscaleChain";
import { inferImageMode, ref2iEngine, MAX_IMAGE_REFERENCES } from "../utils/imageMode";
import { inNode, studioGenerate, canvasLongTake, canvasDepthCapture, canvasDepthVideo, canvasImageOp, canvasVideoInterpolate, canvasVideoUpscale } from "./canvasTasks";

/** 一张参考图在**实际下发顺序**里的位置和它的名字。 */
export interface VideoInputReference {
  path: string;
  name?: string;
}

/**
 * 视频节点实际会收到的参考图，**按下发顺序、已去重**。
 *
 * Stop at each image output: its own source images are not extra R2V slots.
 *
 * **这是 `<Picture N>` 编号的唯一真相。** 出过事：编号按 `data.references`
 * 的下标算，而真正发出去的是这里的返回值 —— 两个列表只要出现一次按路径去重
 * 的塌缩（同一张图被引用两次、角色节点和参考列表指向同一个文件），下标就整体
 * 前移，`<Picture 3>` 指到第 2 张图上。换了张脸，任务照样成功、没有任何告警。
 * 所以名字要和路径一起从这里出去，不能让调用方再去翻另一个数组。
 */
export function videoInputReferences(
  targetId: string,
  nodes: CanvasNode[],
  edges: CanvasEdge[],
): VideoInputReference[] {
  resolveUpstreamPipeline(targetId, nodes, edges);
  const visited = new Set<string>();
  const target = nodes.find((n) => n.id === targetId);
  // Map 而不是 Set：去重依据仍是路径，但要把名字一起带住
  const refs = new Map<string, VideoInputReference>();
  const add = (path: string, name?: string) => {
    if (!path) return;
    const existing = refs.get(path);
    if (existing) {
      // 同一张图被引用两次：保留先出现的那个名字，不新增槽位
      if (!existing.name && name) existing.name = name;
      return;
    }
    refs.set(path, { path, name });
  };

  for (const r of (target?.data as VideoGenNodeData | undefined)?.references ?? []) {
    if (!r.sourceNodeId) {
      add(pathFromMediaUrl(r.path), r.name);
      continue;
    }
    const source = nodes.find((n) => n.id === r.sourceNodeId);
    if (!source) throw new Error(`画布参考已删除：${r.name}`);
    add(pathFromMediaUrl(nodeReference(source).path), r.name);
  }

  function visit(id: string) {
    if (visited.has(id)) return;
    visited.add(id);
    const node = nodes.find((n) => n.id === id);
    if (!node) return;
    if (node.type === "imageGen") {
      const data = node.data as ImageGenNodeData;
      const url = data.outputImageUrl || data.references?.[0]?.path;
      // imageGen 节点没有名字字段 —— 它占一个 <Picture N> 槽位，但没法被 @ 到
      if (url) add(pathFromMediaUrl(url));
      return;
    }
    if (node.type === "character") {
      const data = node.data as CharacterNodeData;
      if (data.avatarUrl) add(pathFromMediaUrl(data.avatarUrl), data.name);
    }
    if (node.type === "upscale") {
      // 放大后的图当参考图；放大后的视频不是参考图。都不再往上找 —— 否则原图和放大图各占一个槽位
      const media = nodeMedia(node);
      if (media?.kind === "image") add(media.path);
      return;
    }
    if (node.type === "videoGen" || node.type === "depthCapture") return;
    edges.filter((e) => e.target === id).forEach((e) => visit(e.source));
  }
  edges.filter((e) => e.target === targetId).forEach((e) => visit(e.source));
  return [...refs.values()];
}

/** 只要路径的老接口。编号相关的逻辑请用 {@link videoInputReferences}。 */
export function videoInputImages(targetId: string, nodes: CanvasNode[], edges: CanvasEdge[]): string[] {
  return videoInputReferences(targetId, nodes, edges).map((r) => r.path);
}

/**
 * 手动取景的节点：它的产出是**人在里面转、点截图**得来的，执行不了。
 *
 * 执行队列追到这里就停。不停的话：全景截图组连着查看器、查看器连着全景出图，
 * 下游一出图就会顺着线把**全景重新生成一遍**（参数改过就算脏）——
 * 白压一次 GPU，而截图并不会因此更新。
 */
const MANUAL_CAPTURE_TYPES = new Set(["panoViewer", "directorWorld"]);

/**
 * 拓扑排序：逆向追踪目标节点的所有上游依赖，返回严格的执行队列（依赖在前，目标在后）。
 * 支持防环检测。遇到手动取景的节点（见 `MANUAL_CAPTURE_TYPES`）不再往上追。
 */
export function resolveUpstreamPipeline(
  targetNodeId: string,
  nodes: CanvasNode[],
  edges: CanvasEdge[],
): CanvasNode[] {
  const visited = new Set<string>();
  const visiting = new Set<string>();
  const executionOrder: CanvasNode[] = [];

  function dfs(currentId: string) {
    if (visiting.has(currentId)) {
      throw new Error(`检测到循环依赖环路: 节点 ${currentId}`);
    }
    if (visited.has(currentId)) return;

    visiting.add(currentId);

    const current = nodes.find((n) => n.id === currentId);
    const boundary = currentId !== targetNodeId && MANUAL_CAPTURE_TYPES.has(current?.type ?? "");
    // 找到所有以当前节点为目标的连线（流入的依赖）
    const incomingEdges = boundary ? [] : edges.filter((edge) => edge.target === currentId);
    for (const edge of incomingEdges) {
      dfs(edge.source);
    }

    visiting.delete(currentId);
    visited.add(currentId);

    const node = nodes.find((n) => n.id === currentId);
    if (node) executionOrder.push(node);
  }

  dfs(targetNodeId);
  return executionOrder;
}

/** 动作迁移没写提示词时的描述。人由人物图定、动作由深度定，这句只交代「照着做、别乱改」 */
export const DEFAULT_MOTION_PROMPT =
  "The person from the reference image performs exactly the same motion as the guide video, keeping their face, hairstyle and outfit unchanged. Fixed camera, natural soft lighting, smooth realistic movement.";

/**
 * 汇总目标节点的所有上游输入，提取组合后的 Prompt 与参考图（支持多跳递归继承，如 文本->图片->视频）
 */
/**
 * 镜头上下文节点发下去的那几段。
 *
 * 顺序是有讲究的：文字地图 → 地理锁 → 站位 → 状态。
 * 地图和锁在前是因为**提示词越靠后越容易被稀释**，而地理是这一层要锁的
 * 东西；状态（时段/天气）放最后是因为它本来就是会变的那一半。
 */
export function shotContextParts(d: ShotContextNodeData): string[] {
  const out: string[] = [];
  const push = (label: string, v?: string) => {
    const t = (v ?? "").trim();
    if (t) out.push(`${label}：${t}`);
  };
  push("空间", d.viewMap);
  push("地理锁", d.lock);
  push("站位", d.blocking);
  push("时段", d.state);
  return out;
}

/**
 * 技能节点发下去的词条文本。
 *
 * `picks` 存的是 `groupId:termId`，按下游引擎现算中英 ——
 * 中文引擎（Z-Image / Krea 2 / H3）发中文，其余发英文。
 * 认不出的 id 原样跳过，不要发一个半截的 `shotSize:` 出去。
 */
export function skillPromptParts(d: SkillNodeData, engine: string): string[] {
  const out: string[] = [];
  for (const pick of d.picks ?? []) {
    const i = pick.indexOf(":");
    if (i <= 0) continue;
    const groupId = pick.slice(0, i);
    const termId = pick.slice(i + 1);
    if (!termId) continue;
    const text = promptText(groupId, termId, engine).trim();
    // `promptText` 认不出 id 时会把 id 原样返回 —— 那不是提示词，别发。
    if (!text || text === termId) continue;
    out.push(text);
  }
  return out;
}

/** 出图 / 出片 / 超分 / 深度捕捉：它们的产物是媒体，走到这里就不再往上找首帧。 */
const MEDIA_BOUNDARY_TYPES = new Set(["imageGen", "videoGen", "upscale", "depthCapture"]);

export function aggregateNodeInputs(
  targetNodeId: string,
  nodes: CanvasNode[],
  edges: CanvasEdge[],
) {
  let upstreamOrder: CanvasNode[] = [];
  try {
    upstreamOrder = resolveUpstreamPipeline(targetNodeId, nodes, edges);
  } catch {
    const incoming = edges.filter((e) => e.target === targetNodeId);
    upstreamOrder = incoming
      .map((e) => nodes.find((n) => n.id === e.source))
      .filter((n): n is CanvasNode => Boolean(n));
  }

  const targetNode = nodes.find((n) => n.id === targetNodeId);

  // 追踪沿纯提示词路径可到达目标节点的上游提示源（阻止穿透 imageGen/videoGen 等媒体生成资产边界）
  const promptSourceNodeIds = new Set<string>();
  const visitedPrompt = new Set<string>();
  function findPromptSources(currId: string) {
    const incoming = edges.filter((e) => e.target === currId);
    for (const edge of incoming) {
      const srcId = edge.source;
      if (visitedPrompt.has(srcId)) continue;
      visitedPrompt.add(srcId);
      const srcNode = nodes.find((n) => n.id === srcId);
      if (!srcNode) continue;
      // imageGen 与 videoGen 节点为媒体生成资产边界，其自身及更上游用于画图的提示词不应作为下游提示词下发
      if (srcNode.type === "imageGen" || srcNode.type === "videoGen") {
        continue;
      }
      promptSourceNodeIds.add(srcId);
      findPromptSources(srcId);
    }
  }
  findPromptSources(targetNodeId);

  // **首帧只认「直接连着」的媒体节点，不越过它去拿更上游的图。**
  //
  // 以前这里把整条上游都走一遍、最后一个有图的胜出。EP07 续篇
  // 「记忆消除器图 → 续篇首帧 → 续篇视频」：首帧节点还在跑、还没图，
  // 视频就拿到了记忆消除器那张 —— 预览上显示它，单独重跑视频时也真用它当首帧，
  // 跑一个多小时、不报错（2026-09-23 用户截图发现）。
  // 和上面的提示词一样：出图 / 出片 / 超分节点是边界，走到就停。
  const nearNodeIds = new Set<string>();
  (function walkNear(id: string) {
    for (const edge of edges.filter((e) => e.target === id)) {
      if (nearNodeIds.has(edge.source)) continue;
      nearNodeIds.add(edge.source);
      const src = nodes.find((n) => n.id === edge.source);
      if (src && !MEDIA_BOUNDARY_TYPES.has(src.type)) walkNear(src.id);
    }
  })(targetNodeId);

  let promptParts: string[] = [];
  // **上下文和技能各自成段，不和画面描述混在一句里。**
  //
  // 全用逗号连成一句的话，模型分不清哪句说的是画面、哪句说的是镜头 ——
  // 这一条在成片链路的分段拼装里已经验过（`shared/shot-language.json`
  // 的 assembly.note）。
  const contextParts: string[] = [];
  const skillParts: string[] = [];
  // 技能词条发中文还是英文，看下游引擎。**存的是词表 id 不是显示串** ——
  // 存显示串的老毛病是模型只收到「推镜头」四个字。
  const targetEngine =
    (targetNode?.data as { engine?: string } | undefined)?.engine ?? "";
  let characterRef: CharacterNodeData | null = null;
  /** 上游把画幅定死了。`null` = 下游节点自己说了算。 */
  let forcedSize: { width: number; height: number } | null = null;
  let inputImageUrl: string | null = null;
  let upstreamImageNode: ImageGenNodeData | null = null;

  for (const sourceNode of upstreamOrder) {
    if (sourceNode.id === targetNodeId) continue;

    if (sourceNode.type === "prompt") {
      if (promptSourceNodeIds.has(sourceNode.id)) {
        const pData = sourceNode.data as PromptNodeData;
        if (pData.prompt?.trim()) {
          promptParts.push(pData.prompt.trim());
        }
        if (pData.scale && pData.scale !== "medium_shot") {
          promptParts.push(`shot scale: ${pData.scale}`);
        }
        if (pData.cameraMovement && pData.cameraMovement !== "static") {
          promptParts.push(`camera: ${pData.cameraMovement}`);
        }
      }
    } else if (sourceNode.type === "character") {
      const cData = sourceNode.data as CharacterNodeData;
      characterRef = cData;
      if (cData.avatarUrl && !inputImageUrl && nearNodeIds.has(sourceNode.id)) {
        inputImageUrl = cData.avatarUrl;
      }
      if (promptSourceNodeIds.has(sourceNode.id)) {
        // 若目标为视频节点且为参考生视频 (r2v) 模式，角色外观由参考图 <Picture N> 提供，无需重复堆叠外观长句
        const isVideoR2v =
          targetNode?.type === "videoGen" &&
          ((targetNode.data as VideoGenNodeData)?.mode === "r2v" ||
            Boolean((targetNode.data as VideoGenNodeData)?.references?.length));
        if (!isVideoR2v) {
          if (cData.promptTrigger?.trim()) {
            promptParts.unshift(cData.promptTrigger.trim());
          }
          if (cData.appearancePrompt?.trim()) {
            promptParts.push(cData.appearancePrompt.trim());
          }
        }
      }
    } else if (sourceNode.type === "shot") {
      if (promptSourceNodeIds.has(sourceNode.id)) {
        // **排在最前面。** 这一段是后端算好的完整画面描述，
        // 后面那些（技能词、上下文）是给它加料的，顺序反了会被挤掉。
        contextParts.unshift(...shotPromptParts(sourceNode.data as ShotNodeData));
      }
    } else if (sourceNode.type === "shotContext") {
      if (promptSourceNodeIds.has(sourceNode.id)) {
        contextParts.push(...shotContextParts(sourceNode.data as ShotContextNodeData));
      }
    } else if (sourceNode.type === "panorama") {
      if (promptSourceNodeIds.has(sourceNode.id)) {
        const pano = sourceNode.data as PanoramaNodeData;
        if (pano.prompt?.trim()) contextParts.push(pano.prompt.trim());
        // **画幅由全景节点定死，不看下游节点自己设了什么。**
        //
        // 等距圆柱投影就是 2:1，别的比例出来的图当全景用会在上下两极
        // 撕开。和「视频模式由连线推导」同一个道理：图里已经决定了的事
        // 不该再问用户一遍，而问了之后用户填错是默认结果。
        const long = pano.longSide || 2048;
        forcedSize = { width: Math.round(long), height: Math.round(long / 2) };
      }
    } else if (sourceNode.type === "skill") {
      if (promptSourceNodeIds.has(sourceNode.id)) {
        skillParts.push(...skillPromptParts(sourceNode.data as SkillNodeData, targetEngine));
      }
    } else if (sourceNode.type === "upscale") {
      if (!nearNodeIds.has(sourceNode.id)) continue;
      const media = nodeMedia(sourceNode);
      if (media?.kind === "image") inputImageUrl = media.path;
    } else if (sourceNode.type === "imageGen") {
      if (!nearNodeIds.has(sourceNode.id)) continue;
      const imgData = sourceNode.data as ImageGenNodeData;
      upstreamImageNode = imgData;
      if (imgData.outputImageUrl) {
        inputImageUrl = imgData.outputImageUrl;
      }
      // 注意：imageGen 节点是图像生成节点，输出是图像而非文本提示词；
      // 其自身用于生成三视图/角色卡/场景图的提示词绝不可泄露给下游视频或图生图节点，
      // 避免下游视频模型误将角色设定图、三视图、白底等画面结构当成视频生成要求。
    }
  }

  // **上下文排在最前。** 提示词越靠后越容易被稀释，而地理和站位
  // 正是这条路要锁的东西；画面描述和技能词在它之后。
  const combined = [
    ...contextParts,
    ...(promptParts.length ? [promptParts.join(", ")] : []),
    ...skillParts,
  ].filter((x) => x.trim());

  return {
    combinedPrompt: combined.join("\n"),
    character: characterRef,
    forcedSize,
    inputImageUrl,
    upstreamImageNode,
  };
}


/**
 * 计算图像节点的当前输入签名（用于上游变更后的失效与重新执行判定）
 */
export function computeImageNodeSignature(
  nodeId: string,
  nodes: CanvasNode[],
  edges: CanvasEdge[]
): string {
  const node = nodes.find((n) => n.id === nodeId);
  if (!node || node.type !== "imageGen") return "";
  const data = node.data as ImageGenNodeData;
  const inputs = aggregateNodeInputs(nodeId, nodes, edges);
  const combinedPrompt = [inputs.combinedPrompt, data.prompt?.trim()].filter(Boolean).join(", ");
  const source = data.references?.[0]?.path || inputs.inputImageUrl || "";
  const engine = data.engine || "";
  // **签名要认 `forcedSize`。** 不认的话，接上全景节点之后画幅变了
  // 而签名没变，节点显示「已是最新」，用户拿到的还是上一张方图。
  const width = inputs.forcedSize?.width ?? data.width ?? 1024;
  const height = inputs.forcedSize?.height ?? data.height ?? 1024;
  const count = data.imageCount || 1;
  const refs = (data.references || []).map((r) => `${r.path}:${r.name}:${r.assetId || ""}`).join("|");
  return JSON.stringify({ combinedPrompt, source, engine, width, height, count, refs, seed: data.seed ?? null, loras: data.loras, injectTriggers: data.injectTriggers });
}

/** 针对共享上游节点及多次并发调用的单例执行锁 */
export const activeExecutionLocks = new Map<string, Promise<boolean>>();

/**
 * 执行单节点的生成任务（图像生成或视频生成，具备共享锁保证不重复提交）
 */
/**
 * 长镜头：节点提示词框里的内容（提示词风格、上游提示词节点）加进每个窗口。
 * H3 官方格式的窗口放进 `detailed_description:` 开头（画风那段的位置），其余的接在最前面。
 */
export function withSharedStyle(windowPrompt: string, shared: string): string {
  const s = shared.trim();
  if (!s || windowPrompt.includes(s)) return windowPrompt;
  const tag = "detailed_description:";
  const at = windowPrompt.indexOf(tag);
  if (at < 0) return `${s}\n\n${windowPrompt}`;
  const head = windowPrompt.slice(0, at + tag.length);
  const rest = windowPrompt.slice(at + tag.length).replace(/^\s*/, "");
  return `${head}\nVisual style: ${s}.\n${rest}`;
}

export async function executeSingleNode(nodeId: string, projectId = useCanvasStore.getState().activeProjectId): Promise<boolean> {
  const key = JSON.stringify([projectId, nodeId]);
  const existingLock = activeExecutionLocks.get(key);
  if (existingLock) {
    return existingLock;
  }

  const taskPromise = (async () => {
    try {
      return await executeSingleNodeInternal(nodeId, projectId);
    } finally {
      activeExecutionLocks.delete(key);
    }
  })();

  activeExecutionLocks.set(key, taskPromise);
  return taskPromise;
}

async function executeSingleNodeInternal(nodeId: string, projectId: string | null): Promise<boolean> {
  const { nodes, edges } = useCanvasStore.getState().getProjectCanvas(projectId);
  const updateNodeData = (id: string, patch: Record<string, unknown>) =>
    useCanvasStore.getState().updateProjectNodeData(projectId, id, patch);
  const node = nodes.find((n) => n.id === nodeId);
  if (!node) return false;
  // 发起的每个生成都登记成后台任务、任务 id 记到这个节点上（见 canvasTasks.ts）
  // 任务名带上是哪个节点（「第 5 步 · 第一段 · 对峙与追逐 · 生成」）：右下角、任务表里分得清（2026-10-11 用户截图）
  const task = (label: string) => ({ projectId, nodeId, label: `${nodeTaskTitle(node)} · ${label}` });
  if (node.data.referenceLoading) {
    updateNodeData(nodeId, { error: "参考图保存中，请稍后生成" });
    return false;
  }

  const inputs = aggregateNodeInputs(nodeId, nodes, edges);
  const submittedSignature = computeImageNodeSignature(nodeId, nodes, edges);
  updateNodeData(nodeId, { isGenerating: true, error: undefined, referenceError: undefined, notes: [] });

  try {
    if (node.type === "depthCapture") {
      const src = depthSourceVideo(nodeId, nodes, edges);
      if (!src.path) throw new Error(src.reason);
      const out = await inNode(task("深度动作捕捉"), () => canvasDepthCapture(src.path!));
      updateNodeData(nodeId, { depthVideoUrl: out.path, sourceVideoPath: src.path, isGenerating: false, error: undefined });
      return true;
    }
    if (node.type === "upscale") {
      const src = upscaleSource(nodeId, nodes, edges);
      if (!src.path) throw new Error(src.reason);
      updateNodeData(nodeId, { requestedSourcePath: src.path });
      const out = src.kind === "video"
        ? await inNode(task("超分"), () => canvasVideoUpscale(src.path!))
        : await inNode(task("超分"), () => canvasImageOp(src.path!, "upscale"));
      updateNodeData(nodeId, { outputUrl: out.path, sourcePath: src.path, isGenerating: false, error: undefined,
        notes: [src.kind === "video" ? "超分 ×2（RealESRGAN）· 宽高各放大一倍，时长和音轨不变" : "超分 ×2（RealESRGAN）· 宽高各放大一倍"] });
      return true;
    }
    if (node.type === "imageGen") {
      const data = node.data as ImageGenNodeData;
      const combined = [inputs.combinedPrompt, data.prompt?.trim()].filter(Boolean).join(", ");
      const promptToUse = combined || "masterpiece, high quality digital illustration";
      // 绑定的图 + 连进来的上游图，按顺序、去重。参考生图靠顺序区分「场景在前、人物在后」
      const imageRefs = videoInputReferences(nodeId, nodes, edges);
      if (data.imageTool) {
        const src = imageRefs[0]?.path;
        if (!src) throw new Error("工具节点没有连着源图 —— 从一张图的工具条上重新点");
        let path: string;
        if (data.imageTool === "remove_bg" || data.imageTool === "upscale") {
          path = (await inNode(task(data.imageTool === "remove_bg" ? "抠图" : "超分"), () => canvasImageOp(src, data.imageTool as "remove_bg" | "upscale"))).path;
        } else {
          const layout = data.imageTool;
          const out = await inNode(task("生成"), () => studioGenerate({
            mode: layout,
            engine: "krea2",
            // 版式提示词只有一个所有者，在 Rust 的 entity_sheet::sheet_prompt；这里只送零件
            prompt: "",
            negative_prompt: "",
            entity_name: imageRefs[0]?.name || "角色",
            entity_type: "STYLIZED_CHARACTER",
            // 画幅由后端定
            width: 1536,
            height: 1024,
            images: [src],
            ref_boost: 4,
          }));
          if (!out.ok || !out.path) throw new Error(out.message || "引擎没有返回图片");
          path = out.path;
        }
        updateNodeData(nodeId, { outputImageUrl: path, outputImages: [path], isGenerating: false, error: undefined });
        return true;
      }
      const catalog = await studioCapabilities();
      const inferredImage = inferImageMode(imageRefs.length, canonicalCanvasEngine(data.engine || ""), catalog, {
        pose: data.poseFromReference, sceneSheet: data.sceneSheet,
      });
      if (imageRefs.length > MAX_IMAGE_REFERENCES) throw new Error(inferredImage.reason);
      const mode = inferredImage.mode;
      const inputPath = imageRefs[0]?.path ?? null;
      let engineNote: string | undefined;
      let engine: string;
      if (mode === "ref2i") {
        const picked = ref2iEngine(canonicalCanvasEngine(data.engine || ""), catalog);
        engine = picked.engine;
        engineNote = picked.note;
      } else if (mode === "pose") {
        // 姿势锁定只有 SDXL 的 OpenPose ControlNet —— 挂到别的底模上不生效且不报错
        engine = "sdxl";
        if (data.engine && canonicalCanvasEngine(data.engine) !== "sdxl") engineNote = `照姿势只能用 SDXL（OpenPose 只有 SDXL 版），这次没用「${data.engine}」`;
      } else if (mode === "scene_sheet") {
        if (!combined.trim()) throw new Error("写一句场景描述：这是个什么地方、里面有什么");
        const picked = canvasEngines(catalog, "scene_sheet").some((e) => e.id === canonicalCanvasEngine(data.engine || "")) ? canonicalCanvasEngine(data.engine || "") : catalog.default_image_engine;
        const out = await inNode(task("生成"), () => studioGenerate({
          mode: "scene_sheet",
          engine: picked,
          prompt: promptToUse,
          negative_prompt: "",
          // 宫格画幅固定横幅；节点上选的尺寸在这条路上不生效
          width: 1536,
          height: 1024,
          images: [],
          auto_loras: false,
          // 和资产页同一套固定 2×2 机位（正面中心线排第一格，左前 / 右前以它为参考）
          viewpoints: toViewpoints(viewsFromContract(null), null),
        }));
        if (!out.ok || !out.path) throw new Error(out.message || "引擎没有返回图片");
        const all: string[] = out.paths?.length ? out.paths : [out.path];
        updateNodeData(nodeId, { outputImageUrl: all[0], outputImages: all, isGenerating: false, error: undefined,
          notes: [inferredImage.reason, `第 1 张是联合图，后面 ${Math.max(0, all.length - 1)} 张是切好的机位`, ...(out.notes ?? [])] });
        return true;
      } else {
        engine = resolveCanvasEngine(catalog, mode, data.engine);
      }
      // 上游全景节点定死的画幅优先于本节点自己设的 —— 见 `forcedSize`。
      const width = inputs.forcedSize?.width ?? data.width ?? 1024;
      const height = inputs.forcedSize?.height ?? data.height ?? 1024;
      const seed = data.seed ?? Math.floor(Math.random() * 9999999);
      const batchCount = Math.min(4, Math.max(1, data.imageCount || 1));
      // **上游的 LoRA 节点合进来。** 工具条里选的和连线挂的是同一份数据 ——
      // 做成两套必然出现「界面显示挂了 A、实际发的是 B」。见 `collectLoras`。
      const loraConfig = await canvasLoraConfig(
        { ...data, loras: collectLoras(nodeId, nodes, edges, data.loras ?? []) },
        engine,
        promptToUse,
      );

      const paths: string[] = [];
      const notesAccum: string[] = [
        ...(mode === "ref2i" || mode === "pose" ? [inferredImage.reason] : []),
        ...(engineNote ? [engineNote] : []),
        ...(loraConfig.added.length ? [`LoRA 激活词：${loraConfig.added.join(", ")}`] : []),
      ];
      let lastError: string | undefined;

      for (let i = 0; i < batchCount; i++) {
        const currentSeed = seed + i * 7;
        try {
          const out = await inNode(task("生成"), () => studioGenerate({
            mode,
            engine,
            prompt: loraConfig.prompt,
            loras: loraConfig.loras,
            negative_prompt: "",
            width,
            height,
            seed: currentSeed,
            images: mode === "ref2i" ? imageRefs.map((r) => r.path) : inputPath ? [inputPath] : [],
          }));

          if (out.ok && out.path) {
            paths.push(out.path);
            if (out.notes?.length) {
              notesAccum.push(...out.notes);
            }
            updateNodeData(nodeId, {
              outputImageUrl: paths[0],
              outputImages: [...paths],
            });
          } else {
            lastError = out.message || "图像生成失败";
            break;
          }
        } catch (error) {
          lastError = String(error);
          break;
        }
      }

      if (paths.length > 0) {
        const current = useCanvasStore.getState().getProjectCanvas(projectId);
        if (!current.nodes.some((n) => n.id === nodeId)) return false;
        const stale = submittedSignature !== computeImageNodeSignature(nodeId, current.nodes, current.edges);
        updateNodeData(nodeId, {
          outputImageUrl: paths[0],
          outputImages: paths,
          lastExecutedSignature: lastError ? `partial:${submittedSignature}` : submittedSignature,
          notes: [
            ...(engine !== data.engine && data.engine && !engineNote ? [`模型标识已规范为 ${engine}（原 ${data.engine}）`] : []),
            ...notesAccum,
            ...(stale ? ["输入已变化，此结果对应旧输入，请重新生成"] : []),
          ],
          isGenerating: false,
          error: lastError ? `批次未完成（${paths.length}/${batchCount}），已保留成功图片：${lastError}` : undefined,
        });
        return !stale && !lastError;
      } else {
        updateNodeData(nodeId, {
          isGenerating: false,
          error: lastError || "图像生成失败",
        });
        return false;
      }
    } else if (node.type === "videoGen") {
      const data = node.data as VideoGenNodeData;
      if (data.videoTool === "interpolate" || data.videoTool === "upscale") {
        const src = depthSourceVideo(nodeId, nodes, edges);
        if (!src.path) throw new Error(src.reason);
        const upscale = data.videoTool === "upscale";
        const out = upscale
          ? await inNode(task("超分"), () => canvasVideoUpscale(src.path!))
          : await inNode(task("补帧"), () => canvasVideoInterpolate(src.path!));
        updateNodeData(nodeId, { outputVideoUrl: out.path, outputVideos: [out.path], isGenerating: false, error: undefined,
          notes: [upscale ? "超分 ×2（RealESRGAN）· 宽高各放大一倍，时长和音轨不变" : "补帧 ×2（RIFE）· 帧率翻倍，时长和音轨不变"] });
        return true;
      }
      // **长镜头节点不走这条。** 它的提示词在窗口表里、节点自己的提示词栏是空的 ——
      // 2026-10-11 用户点了这个节点的「生成」，这里照单条出片跑：空提示词 + 四张参考图，
      // H3 跑了 50 分钟出一条 15 秒的「参考图陈列」（枪的产品图、两台机器人），任务显示成功。
      // **长镜头（窗口表）也走「生成」。** 以前面板上另有一个「整段出片」，而这里按空提示词出单条 ——
      // 2026-10-11 用户点了生成，H3 跑 50 分钟出一条 15 秒的参考图陈列；用户：「生成按钮就是生成按钮」。
      if (isLongTake(data)) {
        const chain = planLongTake(data);
        if (!chain.ok) throw new Error(chain.why);
        const windows = data.longTakeWindows ?? [];
        // 上游的图每个窗口都挂同一组（分镜页的长镜头一直这样）；LoRA 和单条出片同一套（核架构、激活词拼进每个窗口）
        const upstream = videoInputImages(nodeId, nodes, edges);
        if (!upstream.length) throw new Error("长镜头要至少一张参考图（人物或场景）—— 连上图片节点或角色");
        const picks = collectLoras(nodeId, nodes, edges, data.loras ?? []);
        const configs = await Promise.all(windows.map((w) => canvasLoraConfig({ loras: picks, injectTriggers: data.injectTriggers }, "h3", w.trim())));
        // **节点自己的提示词框也要用上**（2026-10-11 用户在风格里选了「提示词风格」，它写进的是这个框 ——
        // 以前长镜头只读窗口表，这一段整个被忽略，720P 跑了才发现）。每个窗口都加一份，放进画风那段
        const shared = [inputs.combinedPrompt, data.prompt?.trim()].filter(Boolean).join(", ");
        const payload = windows.map((w, i) => ({ prompt: w.trim() ? withSharedStyle(configs[i].prompt, shared) : "", images: upstream }));
        const empty = payload.findIndex((w) => !w.prompt);
        if (empty >= 0) throw new Error(`第 ${empty + 1} 个窗口还没写提示词`);
        const width = data.width || 640;
        const height = data.height || 384;
        const out = await inNode(task("生成"), () => canvasLongTake({
          windows: payload,
          windowSecs: chain.windowSecs,
          width,
          height,
          generateAudio: data.generateAudio !== false,
          // 窗口链一律是参考（Ref2VA）—— 按参考那组档位取
          h3Quality: effectiveQuality(data.h3Quality as string | undefined, "r2v"),
          loras: picks,
        }));
        updateNodeData(nodeId, {
          outputVideoUrl: out.video_path,
          outputVideos: [out.video_path, ...(data.outputVideos ?? []).filter((p) => p !== out.video_path)],
          isGenerating: false,
          error: undefined,
          notes: [`长镜头：${chain.windows} 个窗口 × ${chain.windowSecs}s，一次串起来出完（约 ${out.net_secs.toFixed(1)}s）`],
          lastRun: { prompt: payload.map((w, i) => `【窗口 ${i + 1}】${w.prompt}`).join("\n\n"), engine: "h3", images: upstream, width, height, durationSecs: out.net_secs, at: Date.now() },
        });
        return true;
      }
      const combined = [inputs.combinedPrompt, data.prompt?.trim()].filter(Boolean).join(", ");
      // 运镜标签（@运镜·跟拍）原位换成运镜描述 —— 必须在 compilePromptMentions 之前，它会把认不出的 @ 摘成纯文字。
      // 原片复刻那条也用这个 promptToUse，所以放在最前面
      let promptToUse = compileCameraMoves(combined || "cinematic motion, smooth camera pan",
        /[\u4e00-\u9fa5]/.test(combined.replace(/@运镜·[^\s@，,。]+/g, "")) || prefersChinese(String(data.engine ?? "")));
      // **模式由连线推导，不让用户选。**
      //
      // 那两条「多参考图需要切换为参考生视频」「文生视频不使用参考图」的报错
      // 以前在这里 —— 它们是**模式和连线不一致**的症状，而不一致本来就
      // 不该发生：连线已经说明了一切。现在推导出来的模式必然和连线自洽，
      // 那两个错误也就不可能出现了。
      const inferred = inferVideoMode(nodeId, nodes, edges);
      // **手动选的模式和连线不一致时，不拦，但要说出来。**
      //
      // 拦下来（以前是抛「多参考图需要切换为参考生视频」）等于让用户在不理解
      // 四个模式名的情况下自己去猜哪个对。而默默按他选的跑、悄悄只用其中
      // 一张图，是更糟的那一种 —— 他会以为两张都用上了。
      const modeNotes = inferred.overridden && !inferred.deliberate ? [inferred.reason] : [];
      if (inferred.mode === "motion") {
        const depth = upstreamDepth(nodeId, nodes, edges);
        // 原视频直接连进来（没有深度节点）= 原片复刻，只有那两档
        const engine = effectiveMotionEngine(data.motionEngine, Boolean(depth));
        // 原片复刻两条拿原视频本身当参考：深度节点不用跑，原视频取深度节点上游那个视频，或者直接连进来的那个
        const bySource = isRefMotion(engine);
        const sourceVideoPath = !bySource ? null : depth ? depthSourceVideo(depth.nodeId, nodes, edges) : videoSourceOf(nodeId, nodes, edges);
        if (bySource) {
          if (!sourceVideoPath?.path) throw new Error(sourceVideoPath?.reason ?? "上游没有原视频");
        } else {
          if (!depth?.depthPath) throw new Error("上游的深度动作捕捉还没跑 —— 先点它的「开始捕捉」");
          if (depth.stale) throw new Error("上游换了视频，深度还是旧的 —— 先让深度动作捕捉重新跑一次");
        }
        if (!inferred.images.length) throw new Error(inferred.reason);
        // **不写提示词也让它跑。** 以前这里直接报错，而面板上的占位文字却写着「可以直接点生成」——
        // 用户照着点了、收到一个错误（2026-09-14）。人是谁由人物图定，动作由深度定，提示词是锦上添花。
        const motionPrompt = combined.trim() || DEFAULT_MOTION_PROMPT;
        const out = await inNode(task("动作迁移"), () => canvasDepthVideo({
          depthPath: depth?.depthPath ?? "",
          imagePath: pathFromMediaUrl(inferred.images[0]) ?? inferred.images[0],
          // 连几张就换几个人，按连线顺序从左到右对应（后端把原片首帧和这些图一起交给 Qwen）
          imagePaths: inferred.images.map((p) => pathFromMediaUrl(p) ?? p),
          control: data.motionControl ?? "canny",
          prompt: motionPrompt,
          startSecs: data.motionStartSecs ?? 0,
          generateAudio: Boolean(data.generateAudio),
          engine,
          resolution: inferCurrentResolution(data.resolution, data.width, data.height),
          durationSecs: data.durationSecs || 5,
          keepBackground: Boolean(data.motionKeepBackground),
          sourceVideoPath: bySource ? sourceVideoPath!.path : depth?.sourceVideoPath,
          // 节点上写了 seed 就照用 —— 模板要复现某一次实跑，全靠同一个 seed
          seed: typeof data.seed === "number" && data.seed > 0 ? data.seed : undefined,
        }));
        updateNodeData(nodeId, {
          outputVideoUrl: out.path,
          outputVideos: [out.path, ...(data.outputVideos ?? []).filter((p) => p !== out.path)],
          motionFirstFrame: out.first_frame || undefined,
          notes: [
            inferred.reason,
            ...(combined.trim() ? [] : ["没写提示词，用的是默认描述。写清发型、发长、衣服和场景（比如「齐肩短发女孩穿粉色蕾丝吊带连体衣，在白色舞蹈室里跳舞」）更不容易被改头发、加衣服"]),
            ...out.notes,
          ],
          isGenerating: false,
          error: undefined,
        });
        return true;
      }
      const source = data.references?.[0]?.path || inputs.inputImageUrl;
      const inputPath = source ? pathFromMediaUrl(source) : null;
      const mode = inferred.mode;
      const engine = resolveCanvasEngine(await studioCapabilities(), mode, data.engine);
      // **编号和下发用同一个数组。** 见 videoInputReferences 的注释：
      // 用 data.references 的下标算 <Picture N>，一次去重塌缩就会整体错位。
      const resolvedRefs = mode === "r2v" ? videoInputReferences(nodeId, nodes, edges) : [];
      const references = resolvedRefs.map((r) => r.path);

      // **@提及对所有模式都要编译。** 只在 r2v 编译的话，i2v / t2v / fl2v
      // 会把字面量 `@筱筱` 原样发给模型 —— 不报错，只是画面里没有那个角色。
      // 非 r2v 模式没有 <Picture> 槽位，编译成角色名本身（去掉 @ 记号）。
      // 参考音频（音频素材节点直接连着的）。**先编译音频的 @** —— 下一步会把对不上图片的 @名字 摘掉
      const audioRefs = videoInputAudios(nodeId, nodes, edges);
      if (emptyUpstreamAudios(nodeId, nodes, edges)) throw new Error("连着的音频素材节点还没放音频 —— 先在它上面上传一段");
      if (audioRefs.length && mode !== "r2v") throw new Error("参考音频只有参考生视频（H3）能用 —— 把模式改回「跟着连线走」");
      if (audioRefs.length > MAX_VIDEO_AUDIOS) throw new Error(`H3 一次最多 ${MAX_VIDEO_AUDIOS} 段参考音频，现在连了 ${audioRefs.length} 段`);
      const audios = audioRefs.map((a) => a.path);
      promptToUse = compileAudioMentions(promptToUse, audioRefs, engine);
      promptToUse = compilePromptMentions(promptToUse, resolvedRefs, engine);

      if (mode === "r2v") {
        if (engine !== "h3") throw new Error("参考生视频仅支持 MiniMax H3，请切换模型");
        if (!references.length) throw new Error("参考生视频需要连接至少一张图片或带图片的角色节点");
        if (references.length > MAX_VIDEO_REFERENCES) throw new Error(`参考生视频最多支持 ${MAX_VIDEO_REFERENCES} 张不同参考图，请减少连接`);

        // 提示词里没写 <Audio N> 时补一句是哪段（和下面图片那句一样）—— 不写的话 H3 不知道这段音频干什么用
        if (audioRefs.length && !promptToUse.includes("<Audio")) {
          promptToUse = `${audioRefs.map((a, i) => `<Audio ${i + 1}> 是「${a.name}」`).join("，")}。\n${promptToUse}`;
        }
        // MiniMax H3 Ref2VA 依赖 <Picture 1>, <Picture 2> 建立角色映射
        if (resolvedRefs.length && !promptToUse.includes("<Picture")) {
          const mapping = resolvedRefs
            .map((r, i) => (r.name ? `<Picture ${i + 1}> 设定为 ${r.name}` : ""))
            .filter(Boolean);
          if (mapping.length > 0) {
            promptToUse = `${mapping.join("，")}。保持各自独立特征不要混淆。\n${promptToUse}`;
          }
        }
      }
      if (mode === "i2v" && !inputPath) throw new Error("图生视频需要连接首帧图片");
      // **首尾帧要把两张图按「首、尾」的顺序发出去。** 以前这里只给 r2v / i2v 带图，
      // fl2v 发的是空数组 —— 后端报「需要 2 张图」，画布上的首尾帧从来没跑通过。
      const endpoints = mode === "fl2v" ? inferred.images.slice(0, 2).map((p) => pathFromMediaUrl(p) ?? p) : [];
      if (mode === "fl2v" && endpoints.length < 2) throw new Error("首尾帧需要两张图：一张首帧、一张名字里带「尾帧」的图");
      // 最短 3 秒：存着 2 秒的老节点一跑就报错（见 MIN_VIDEO_SECS）。动作迁移那条另算，不走这里
      const duration = Math.max(MIN_VIDEO_SECS, data.durationSecs || 5);
      const width = data.width || (engine === "h3" ? 640 : 768);
      const height = data.height || (engine === "h3" ? 384 : 512);
      const batchCount = Math.min(4, Math.max(1, (data.videoCount as number) || 1));
      // **上游的 LoRA 节点合进来。** 工具条里选的和连线挂的是同一份数据 ——
      // 做成两套必然出现「界面显示挂了 A、实际发的是 B」。见 `collectLoras`。
      const loraConfig = await canvasLoraConfig(
        { ...data, loras: collectLoras(nodeId, nodes, edges, data.loras ?? []) },
        engine,
        promptToUse,
      );

      // **在提交那一刻记下实际发出去的那一份。**
      // 事后从节点数据重算等于再写一遍这几十行推导，必然漂移。
      updateNodeData(nodeId, {
        lastRun: {
          prompt: loraConfig.prompt,
          engine,
          images: mode === "r2v" ? references : mode === "fl2v" ? endpoints : mode === "i2v" && inputPath ? [inputPath] : [],
          ...(audios.length ? { audios } : {}),
          width, height, durationSecs: duration, at: Date.now(),
        },
      });

      const paths: string[] = [];
      const audioNote = audioLengthNote(audioRefs, duration);
      const notesAccum: string[] = [
        ...modeNotes,
        ...(audioNote ? [audioNote] : []),
        ...(loraConfig.added.length ? [`LoRA 激活词：${loraConfig.added.join(", ")}`] : []),
      ];
      let lastError: string | undefined;

      for (let i = 0; i < batchCount; i++) {
        try {
          const out = await inNode(task("生成"), () => studioGenerate({
            mode,
            engine,
            prompt: loraConfig.prompt,
            loras: loraConfig.loras,
            negative_prompt: "",
            width,
            height,
            duration,
            generate_audio: Boolean(data.generateAudio),
            // H3 官方格式：后端按出片方式整理（只动格式），见 Rust `h3_spec::canvas_official`
            h3_official_format: Boolean(data.h3OfficialFormat),
            // 加速档：节点选的，没选按链路默认（见 utils/h3Quality）。只 H3
            ...(engine === "h3" && effectiveQuality(data.h3Quality, mode) ? { h3_quality: effectiveQuality(data.h3Quality, mode) } : {}),
            images: mode === "r2v" ? references : mode === "fl2v" ? endpoints : mode === "i2v" && inputPath ? [inputPath] : [],
            ...(audios.length ? { audios } : {}),
          }));

          if (out.ok && out.path) {
            paths.push(out.path);
            if (out.notes?.length) {
              notesAccum.push(...out.notes);
            }
            updateNodeData(nodeId, {
              outputVideoUrl: paths[0],
              outputVideos: [...paths],
            });
          } else {
            lastError = out.message || "视频生成失败";
            break;
          }
        } catch (error) {
          lastError = String(error);
          break;
        }
      }

      if (paths.length > 0) {
        updateNodeData(nodeId, {
          outputVideoUrl: paths[0],
          outputVideos: paths,
          notes: notesAccum,
          isGenerating: false,
          error: lastError ? `批次未完成（${paths.length}/${batchCount}），已保留成功视频：${lastError}` : undefined,
        });
        return !lastError;
      } else {
        updateNodeData(nodeId, {
          isGenerating: false,
          error: lastError || "视频生成失败",
        });
        return false;
      }
    }


    updateNodeData(nodeId, { isGenerating: false });
    return true;
  } catch (err) {
    updateNodeData(nodeId, {
      isGenerating: false,
      error: `生成请求异常: ${err}`,
    });
    return false;
  }
}

/**
 * 执行指定节点的生成任务（按 DAG 依赖自动执行未完成或已失效的上游任务）
 */
/** 重跑判据要的两个回调。**预览和执行必须传同一份** —— 见 `willRerun`。 */
export const rerunDeps = (nodes: CanvasNode[], edges: CanvasEdge[]): RerunDeps => ({
  depthSourcePath: (id) => depthSourceVideo(id, nodes, edges).path,
  signatureOf: (id, n, e) => computeImageNodeSignature(id, n, e),
});

function getNode(projectId: string | null, nodeId: string): CanvasNode | undefined {
  return useCanvasStore.getState().getProjectCanvas(projectId).nodes.find((n) => n.id === nodeId);
}

/**
 * 等一个正在生成的节点停下来（出图、失败、取消都算），返回它停下时的样子。
 * 节点被删了返回 `undefined`。重新打开画布时没挂任务的 `isGenerating` 会被清掉
 * （`recoverNodes`），所以这里不会因为残留状态一直等下去。
 */
export function waitForNodeIdle(projectId: string | null, nodeId: string): Promise<CanvasNode | undefined> {
  return new Promise((resolve) => {
    const now = getNode(projectId, nodeId);
    if (!now?.data.isGenerating) return resolve(now);
    const unsubscribe = useCanvasStore.subscribe(() => {
      const n = getNode(projectId, nodeId);
      if (n?.data.isGenerating) return;
      unsubscribe();
      resolve(n);
    });
  });
}

export async function executeNodePipeline(targetNodeId: string) {
  const store = useCanvasStore.getState();
  const { nodes, edges, activeProjectId: projectId } = store;
  const updateNodeData = (id: string, patch: Record<string, unknown>) =>
    useCanvasStore.getState().updateProjectNodeData(projectId, id, patch);

  const targetNode = nodes.find((n) => n.id === targetNodeId);
  if (!targetNode) return;
  if (targetNode.data.referenceLoading) {
    updateNodeData(targetNodeId, { error: "参考图保存中，请稍后生成" });
    return;
  }

  // 1. 验证上游拓扑（检测是否有环路）
  let executionQueue: CanvasNode[];
  try {
    executionQueue = resolveUpstreamPipeline(targetNodeId, nodes, edges);
  } catch (err) {
    updateNodeData(targetNodeId, { error: String(err) });
    return;
  }

  // 2. 级联执行未完成或已失效变更的上游图像节点
  for (const node of executionQueue) {
    if (node.id === targetNodeId) continue;
    // **上游正在生成：等它跑完，不替它再提交一次。**
    //
    // 同一个会话里有执行锁（`activeExecutionLocks`），重复调用会拿到同一个 promise；
    // 但重新打开画布后接着跑的后台任务（`jobId`）没有锁 —— 以前这里判「没图 = 要跑」，
    // 于是同一个节点又提交一次，两张图互相覆盖，下游用的是没人看过的那张。
    if (MEDIA_BOUNDARY_TYPES.has(node.type) && getNode(projectId, node.id)?.data.isGenerating) {
      const done = await waitForNodeIdle(projectId, node.id);
      if (!done || done.data.error) {
        updateNodeData(targetNodeId, { isGenerating: false, error: "等着的上游节点这次没跑成，流程已中断 —— 先看上游那个节点的报错" });
        return;
      }
    }
    const target = nodes.find((n) => n.id === targetNodeId);
    // **绑定成参考图的上游节点不重跑。**
    //
    // 判据读的是**推导出来的模式**，不是存下来的 `mode` 字段 ——
    // 绑定那一步不再替用户写死 `r2v`（连一张首帧也被标成参考生视频是错的），
    // 所以这里要问推导。读存下来的字段会让上传的源图被当成「要重新生成」，
    // 白跑一次出图，而且把用户上传的那张覆盖掉。
    if (
      target?.type === "videoGen" &&
      target.data.references?.some((r) => r.sourceNodeId === node.id) &&
      isReferenceMode(targetNodeId, nodes, edges)
    ) continue;
    if (node.type === "depthCapture") {
      const { nodes: cn, edges: ce } = useCanvasStore.getState().getProjectCanvas(projectId);
      const cur = cn.find((n) => n.id === node.id);
      // **判据只有一个所有者**（`willRerun`）—— 预览再抄一遍必然漂移，
      // 而漂移的表现是「预览说要跑 2 个，实际跑了 5 个」，比不给预览更糟。
      if (cur && willRerun(cur, targetNodeId, cn, ce, rerunDeps(cn, ce))) {
        const ok = await executeSingleNode(node.id, projectId);
        if (!ok) {
          updateNodeData(targetNodeId, { isGenerating: false, error: "上游的深度动作捕捉失败，流程已中断" });
          return;
        }
      }
      continue;
    }
    if (node.type === "upscale") {
      const { nodes: cn, edges: ce } = useCanvasStore.getState().getProjectCanvas(projectId);
      const current = cn.find((n) => n.id === node.id);
      if (current && willRerun(current, targetNodeId, cn, ce, rerunDeps(cn, ce))) {
        const ok = await executeSingleNode(node.id, projectId);
        if (!ok) {
          updateNodeData(targetNodeId, { isGenerating: false, error: "上游的超分失败，流程已中断" });
          return;
        }
      }
      continue;
    }
    if (node.type === "imageGen" && !node.data.referenceOnly) {
      const { nodes: currentNodes, edges: currentEdges } = useCanvasStore.getState().getProjectCanvas(projectId);
      const cur = currentNodes.find((n) => n.id === node.id);
      if (cur && willRerun(cur, targetNodeId, currentNodes, currentEdges, rerunDeps(currentNodes, currentEdges))) {
        const ok = await executeSingleNode(node.id, projectId);
        if (!ok) {
          updateNodeData(targetNodeId, {
            isGenerating: false,
            error: `上游图像生成节点执行失败，流程已中断`,
          });
          return;
        }
      }
    }
  }

  // 3. 执行目标节点
  const latest = useCanvasStore.getState().getProjectCanvas(projectId);
  try {
    const currentQueue = resolveUpstreamPipeline(targetNodeId, latest.nodes, latest.edges);
    if (JSON.stringify(currentQueue.map((n) => n.id)) !== JSON.stringify(executionQueue.map((n) => n.id)) ||
      currentQueue.some((n) => n.id !== targetNodeId && n.type === "imageGen" &&
        !(latest.nodes.find((t) => t.id === targetNodeId)?.data.references as import("../types").CanvasReference[] | undefined)?.some((r) => r.sourceNodeId === n.id) &&
        n.data.lastExecutedSignature && n.data.lastExecutedSignature !== computeImageNodeSignature(n.id, latest.nodes, latest.edges))) {
      throw new Error("上游输入或连接已变化，请重新执行流程");
    }
  } catch (err) {
    updateNodeData(targetNodeId, { error: String(err), isGenerating: false });
    return;
  }
  await executeSingleNode(targetNodeId, projectId);
}
