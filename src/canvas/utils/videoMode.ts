import type { CanvasNode, CanvasEdge, VideoGenNodeData } from "../types";
import { videoInputReferences } from "../engine/dagExecutor";
import { isRefMotion, upstreamDepth, upstreamVideoNodes } from "./depthChain";
import { emptyUpstreamAudios, videoInputAudios } from "./audioClips";

/**
 * 视频生成模式。
 *
 * `auto` 不是一种模式，是「按连线推导」的意思 —— 它是默认值。
 */
export type VideoMode = "t2v" | "i2v" | "fl2v" | "r2v" | "motion";

export interface ModeInference {
  mode: VideoMode;
  /** 为什么是这个模式。**要显示给用户看** —— 见下面的注释。 */
  reason: string;
  /** 实际会发出去的参考图路径。 */
  images: string[];
  /** 用户手动选了一个和推导结果不同的模式时，这里是推导结果。 */
  overridden?: VideoMode;
  /**
   * 手选和推导不一样，但**这个手选本身是正当的**（一张图选参考生视频 = 拿它当人物外观参考）。
   * 界面不标黄、执行器不写成运行提示；`overridden` 照留，菜单里「改回跟着连线走」还要用它。
   */
  deliberate?: boolean;
  /** 动作迁移的原视频从哪来：接了深度节点，还是原视频直接连进来（原片复刻）。只在 `motion` 时有 */
  via?: "depth" | "video";
  /** 连着的图一共几张：已经有图的 + 还没出图的（见 `pendingUpstreamImages`）。模式菜单按它决定哪几项能选 */
  imageCount?: number;
}

/**
 * 直接连着、但**还没出图**的出图节点。
 *
 * 点「生成」时执行器会先把它们跑出来（`executeNodePipeline` 级联上游），所以推模式要把它们算进去 ——
 * 以前只数已经有图的，连着一个空的图片节点就推成「文生视频」，下面的预检却说「这张图还没出，出了之后它就是首帧」，
 * 两处说法打架（用户 2026-09-24 截图）。
 */
export function pendingUpstreamImages(nodeId: string, nodes: CanvasNode[], edges: CanvasEdge[]): CanvasNode[] {
  return edges
    .filter((e) => e.target === nodeId)
    .map((e) => nodes.find((n) => n.id === e.source))
    .filter((n): n is CanvasNode => {
      if (!n || n.type !== "imageGen") return false;
      const d = n.data as { outputImageUrl?: string; outputImages?: string[] };
      return !d.outputImageUrl && !d.outputImages?.length;
    });
}

/**
 * 从连线推导视频模式。
 *
 * # 为什么不让用户选
 *
 * 「文生视频 / 图生视频 / 首尾帧 / 参考生视频」这四个名字对着数据结构才有意义：
 * 它们说的其实就是**这个节点连了几张图、是什么样的图**。让用户先理解四个名字、
 * 再自己保证连线和选择一致，等于把我们的数据模型摊给他看 ——
 * 而选错了的结果是生成失败，或者更糟：模式和连线不一致时悄悄按错的那个跑。
 *
 * 连线本身已经说明了一切：
 *
 * | 连了什么 | 模式 |
 * |---|---|
 * | 什么都没连 | `t2v` 文生视频 |
 * | 一张图 | `i2v` 图生视频（那张是首帧） |
 * | 两张图，其中一张标了尾帧 | `fl2v` 首尾帧 |
 * | 多张带名字的图 | `r2v` 参考生视频（身份参考） |
 *
 * # 推导结果和理由都要显示
 *
 * 只显示模式名，用户还是不知道为什么 —— 尤其是他以为自己连了三张图、
 * 实际上有一张没连上的时候。理由里带上**数量**，那是最容易对不上的那一项。
 */
export function inferVideoMode(
  nodeId: string,
  nodes: CanvasNode[],
  edges: CanvasEdge[],
): ModeInference {
  const node = nodes.find((n) => n.id === nodeId);
  const data = (node?.data ?? {}) as VideoGenNodeData;

  // 实际会发出去的那些图（已去重、含沿图追加的上游图）
  let refs: { path: string; name?: string }[] = [];
  try {
    refs = videoInputReferences(nodeId, nodes, edges);
  } catch {
    // 参考指向已删除的节点时 videoInputReferences 会抛 —— 那是另一个错误，
    // 不该让模式推导跟着崩。按「什么都没连」处理，真正的错误在执行时报。
    refs = [];
  }

  const endFrame = refs.some((r) => isEndFrame(r.name));
  const pending = pendingUpstreamImages(nodeId, nodes, edges).length;
  const imageCount = refs.length + pending;
  const pendingNote = pending ? `（其中 ${pending} 张上游还没出 —— 点生成时先把它跑出来）` : "";

  let inferred: ModeInference;
  // **接了深度动作捕捉就是动作迁移**，压过其他一切推导 —— 连着深度节点却按图生视频跑，
  // 那段动作就白抽了，而且不报错。
  //
  // **连几张人物图就换几个人**（2026-09-23 起）。以前只取第一张、其余静默丢掉 ——
  // 后端现在把原片首帧和所有人物图一起交给 Qwen 一次换掉，**按连线顺序从左到右对应**。
  if (upstreamDepth(nodeId, nodes, edges)) {
    if (refs.length === 0) {
      return { mode: "motion", via: "depth", imageCount, images: [], reason: pending
        ? "接了深度动作捕捉 —— 人物图还没出，点生成时先把它跑出来"
        : "接了深度动作捕捉 —— 还要再连一张人物图，才知道把动作换到谁身上" };
    }
    if (refs.length === 1) {
      return { mode: "motion", via: "depth", reason: `接了深度动作捕捉 —— 把这段动作换到「${refs[0].name || "这张图里的人"}」身上`, images: [refs[0].path] };
    }
    const order = refs.map((r, i) => `第 ${i + 1} 张${r.name ? `「${r.name}」` : ""}换从左数第 ${i + 1} 个人`).join("，");
    return {
      mode: "motion", via: "depth",
      reason: `接了深度动作捕捉 —— 连了 ${refs.length} 张人物图，换 ${refs.length} 个人：${order}（顺序就是连线顺序；提示词里另写了谁换谁，以提示词为准）`,
      images: refs.map((r) => r.path),
    };
  }
  // **原视频直接连进来 + 节点选了原片复刻 = 原片复刻**（2026-09-24 起）。原片复刻拿原视频本身当参考，不用深度 ——
  // 以前非得在中间摆一个「不用点」的深度节点，用户看着不知道为什么要有它。
  //
  // **必须节点上明确选了原片复刻才算**：旧画布里还留着长镜头窗口链那种视频连视频的线（现在什么都不传），
  // 光凭连线就切过来，那些节点一点「生成」就变成一小时的原片复刻。没选过的，节点上给一个「设成原片复刻」的按钮。
  // 补帧 / 超分那两个工具节点也是视频连视频，它们不是出片，排除。
  if (!data.videoTool && isRefMotion(data.motionEngine) && upstreamVideoNodes(nodeId, nodes, edges).length > 0) {
    const images = refs.map((r) => r.path);
    if (refs.length === 0) {
      return { mode: "motion", via: "video", imageCount, images, reason: pending
        ? "接了一段原视频 —— 原片复刻：人物图还没出，点生成时先把它跑出来"
        : "接了一段原视频 —— 原片复刻：还要再连人物图（三视图整张给），才知道把原片里的人换成谁" };
    }
    const list = refs.map((r, i) => `图片${i + 1}${r.name ? `「${r.name}」` : ""}`).join("、");
    return {
      mode: "motion", via: "video", images,
      reason: `接了一段原视频 —— 原片复刻：把原片里的人换成 ${list}（顺序就是连线顺序；多人时在提示词里按角色写谁换成谁，不要按左右）`,
    };
  }
  // **连了音频素材 = 参考生视频**（2026-10-08）：参考音频只有 H3 Ref2VA 收 —— 一张图也不能当首帧（首帧那条 FL2VA 不收音频）
  const audios = videoInputAudios(nodeId, nodes, edges);
  const audioCount = audios.length + emptyUpstreamAudios(nodeId, nodes, edges);
  if (audioCount > 0) {
    const tags = audios.map((a, i) => `<Audio ${i + 1}>「${a.name}」`).join("、") || `${audioCount} 段（还没放音频）`;
    inferred = imageCount === 0
      ? { mode: "r2v", reason: `连了参考音频 ${tags} —— 还要再连至少一张图（人物 / 场景）：H3 不收只有音频的参考`, images: [] }
      : {
        mode: "r2v",
        reason: `连了 ${imageCount} 张图 + 参考音频 ${tags} —— 参考生视频：图是 <Picture 1..>，音频是 <Audio 1..>（提示词里可以写 @音频1）` + pendingNote,
        images: refs.map((r) => r.path),
      };
  } else if (imageCount === 0) {
    inferred = { mode: "t2v", reason: "没有连接任何图片 —— 纯文生视频", images: [] };
  } else if (refs.length === 0 && pending === 1) {
    inferred = { mode: "i2v", reason: "连了 1 张图，上游还没出 —— 点生成时先把它跑出来，出了之后它就是首帧", images: [] };
  } else if (refs.length >= 2 && endFrame) {
    // **尾帧排最后。** 发图顺序就是首、尾 —— 以前按连线顺序发，
    // 先连了尾帧的话视频就倒着走，不报错。
    const end = refs.find((r) => isEndFrame(r.name))!;
    const start = refs.find((r) => r !== end)!;
    const extra = refs.length - 2;
    inferred = {
      mode: "fl2v",
      reason: `连了 ${refs.length} 张图，其中「${end.name}」标了尾帧 —— 按首尾帧生成` +
        (extra > 0 ? `（多出的 ${extra} 张不会用上）` : ""),
      images: [start.path, end.path],
    };
  } else if (imageCount === 1 && refs.length === 1 && (isCharacterSheet(refs[0].name, refs[0].path) || upstreamImageLabels(nodeId, nodes, edges).some((l) => isCharacterSheet(l)))) {
    // 一张**角色设定卡**：它是人物外观，不是第一帧。当首帧的话片子开头就是那张排版图
    // （用户 2026-09-25：「用角色卡的话，就会提示是首帧」）
    inferred = {
      mode: "r2v",
      reason: `连了 1 张角色设定卡${refs[0].name ? `「${refs[0].name}」` : ""} —— 只当人物外观参考，不当首帧`,
      images: [refs[0].path],
    };
  } else if (imageCount === 1) {
    inferred = {
      mode: "i2v",
      reason: "连了 1 张图 —— 它是首帧",
      images: refs.map((r) => r.path),
    };
  } else {
    // 多张图：图片参考（身份 / 场景参考）。**每张都 @ 得到** —— 候选就是面板顶上那排卡片，
    // 卡片编号 = @图片N = <Picture N>。以前没名字的图 @ 不到，这里写「其中 N 张没有名字，@ 不到」。
    const tags = Array.from({ length: imageCount }, (_, i) => `@图片${i + 1}`);
    const tagList = tags.length > 3 ? `${tags.slice(0, 2).join("、")}…${tags[tags.length - 1]}` : tags.join("、");
    inferred = {
      mode: "r2v",
      // **@ 不是必须的**（用户 2026-09-25）：连着的图都会发出去；点名只是写清谁是谁，直接写 <Picture N> 一样
      reason: `连了 ${imageCount} 张图 —— 图片参考：都只当外观参考，全部会发出去；要写清谁是谁，在提示词里用 ${tagList}（或直接写 <Picture 1>…）点名` + pendingNote,
      images: refs.map((r) => r.path),
    };
  }

  // 用户手动选过就尊重他，但**把推导结果也带出去** ——
  // 不带的话他看不出自己选的和连线不一致，而那正是最容易出错的地方。
  inferred.imageCount = imageCount;
  const picked = data.mode;
  if (picked === "r2v" && inferred.mode === "i2v" && inferred.images.length === 1) {
    // 一张图、手选参考生视频：拿它当人物外观参考，是正当用法，不当成「选错了」来警告
    return {
      imageCount,
      mode: "r2v",
      reason: "参考生视频 —— 这 1 张图只当人物外观参考，不当首帧",
      images: inferred.images,
      overridden: inferred.mode,
      deliberate: true,
    };
  }
  if (picked === "fl2v" && inferred.mode === "r2v" && inferred.images.length === 2) {
    // 两张图、手选首尾帧：菜单里明确选的，是正当用法。第 1 张开头、第 2 张结尾（卡片上标着，可以对调）。
    // 三张以上选首尾帧才警告 —— 多出来的用不上
    return {
      imageCount,
      mode: "fl2v",
      reason: "首尾帧 —— 第 1 张是开头、第 2 张是结尾（卡片上标着，点「对调首尾」可以换）",
      images: inferred.images,
      overridden: inferred.mode,
      deliberate: true,
    };
  }
  if (picked && picked !== "auto" && picked !== inferred.mode) {
    return {
      imageCount,
      mode: picked as VideoMode,
      reason: `${MANUAL_MODE_PREFIX}${MODE_LABEL[picked as VideoMode] ?? picked}，但连线看起来是${MODE_LABEL[inferred.mode]}（${inferred.reason}）`,
      images: inferred.images,
      overridden: inferred.mode,
    };
  }
  return inferred;
}

/** 「手选和连线不一致」那句提示的开头。节点上显示运行提示时靠它认出旧的模式提示（见 `liveNotes`） */
export const MANUAL_MODE_PREFIX = "你手动选了";

/**
 * 节点底部显示的运行提示：**模式那一条不显示**（存下来的旧的也滤掉）。
 *
 * 执行器开跑时把模式提示写进 `notes`，要到下次运行才会清 —— 于是改了模式、改了判断规则之后，
 * 那条旧黄字还一直挂着（用户 2026-09-25：「这里还是显示黄字儿」）。
 */
export function liveNotes(stored: string[] | undefined, _current: ModeInference): string[] | undefined {
  // 模式那条**不在节点上显示**（用户 2026-09-25：「不用黄色提示词」）—— 完整理由在模式按钮的悬停提示里
  const rest = (stored ?? []).filter((n) => !n.startsWith(MANUAL_MODE_PREFIX));
  return rest.length ? rest : undefined;
}

export const MODE_LABEL: Record<VideoMode, string> = {
  t2v: "文生视频",
  i2v: "图生视频",
  fl2v: "首尾帧",
  r2v: "图片参考",
  motion: "动作迁移",
};

/**
 * 这张图是不是角色设定卡（三视图 / 设定图 / character sheet）—— 看名字和文件名。
 * 设定卡是人物外观参考；当首帧的话，生成的片子开头就是那张排版图。
 */
export function isCharacterSheet(name: string | undefined, path?: string): boolean {
  const s = `${name ?? ""} ${(path ?? "").split("/").pop() ?? ""}`.toLowerCase();
  return ["设定卡", "设定图", "角色卡", "人设图", "三视图", "四视图", "character sheet", "character_sheet", "charsheet", "turnaround", "reference sheet", "model sheet"]
    .some((k) => s.includes(k));
}

/**
 * 直连上游图片节点的标题。上传的图，标题就是原文件名（「Rabbit Racer 设定卡（蓝）.png」）——
 * 而传给推导的参考只有保存后的路径（重新起过名）、没有名字，只看它认不出设定卡。
 */
function upstreamImageLabels(nodeId: string, nodes: CanvasNode[], edges: CanvasEdge[]): string[] {
  return edges
    .filter((e) => e.target === nodeId)
    .map((e) => nodes.find((n) => n.id === e.source))
    .filter((n): n is CanvasNode => n?.type === "imageGen")
    .map((n) => String((n.data as { label?: string }).label ?? ""));
}

/** 这个参考是不是被标成了尾帧。 */
function isEndFrame(name: string | undefined): boolean {
  const n = (name ?? "").trim().toLowerCase();
  return ["尾帧", "末帧", "end", "last", "end frame", "last frame"].some((k) => n.includes(k));
}


/**
 * 这个视频节点是不是「参考图直接出片」那条路。
 *
 * `r2v` 和 `i2v` 都是拿绑定的图当输入，**都不该重跑上游节点** ——
 * 上游那张图已经在了（可能还是用户自己上传的），重跑等于白花一次出图，
 * 还会把上传的那张覆盖掉。
 */
export function isReferenceMode(
  nodeId: string,
  nodes: CanvasNode[],
  edges: CanvasEdge[],
): boolean {
  const m = inferVideoMode(nodeId, nodes, edges).mode;
  return m === "r2v" || m === "i2v" || m === "fl2v" || m === "motion";
}
