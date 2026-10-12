import type { CanvasEdge, CanvasNode, VideoGenNodeData } from "../types";
import { inferVideoMode } from "./videoMode";
import { loraFits, isBakedIn } from "./collectLoras";
import { MAX_VIDEO_REFERENCES } from "./referenceBindings";

/**
 * 出片前的预检：**这次实际会发出什么，哪些连线白连了。**
 *
 * # 为什么非有不可
 *
 * 这个项目最贵的一类错误是「连线看起来成功、模型根本不吃这个输入」——
 * 跑完四十分钟才发现参考图没生效，而任务状态是成功。已经踩过的原型：
 * LoRA 挂到不匹配的架构上静默失效、参考图键名写错被静默忽略、
 * `<Video 1>` 只写进提示词没写进输入绑定。
 *
 * # 为什么只报不拦
 *
 * 拦连线要有一张「谁能连谁」的完整矩阵，而那张矩阵一旦有一格判错，
 * 毁掉的是本来能用的流程 —— 比不拦更糟。这里**只说被忽略了什么**，
 * 判断权留给用户。真正确定无疑的（LoRA 架构对不上）才标成冲突。
 *
 * # 判据从哪来
 *
 * 逐条抄自 `dagExecutor` 实际的消费规则（`aggregateNodeInputs` 的
 * else-if 链、`videoInputReferences` 的 visit），**不是按节点名字猜的**。
 */

export interface AuditInput {
  nodeId: string;
  /** 这个上游是什么 */
  from: string;
  /** 它贡献什么。没被用时这里写原因 */
  role: string;
  used: boolean;
}

export interface RunAudit {
  /** 实际会用的模式 */
  mode: string;
  modeReason?: string;
  inputs: AuditInput[];
  /** 确定会出错或确定无效的 */
  conflicts: string[];
}

/** 节点类型 → 给人看的名字。预检和重跑范围共用，别再写第二份。 */
export const TYPE_LABEL: Record<string, string> = {
  prompt: "提示词节点", shot: "分镜", shotContext: "镜头上下文", skill: "镜头语言",
  character: "角色", imageGen: "图片", videoGen: "视频", upscale: "超分/补帧",
  depthCapture: "深度动作捕捉", lora: "LoRA", panorama: "全景",
  panoViewer: "全景取景", directorWorld: "导演世界", group: "分组",
};

/**
 * 预检一个视频节点。
 *
 * `loraEngines` 是每个 LoRA 名字对应的兼容引擎（`studio_loras` 给的，
 * 由张量键名判定架构 —— 不是按文件名猜的）。取不到就不判，
 * **宁可不报，也不要报错的**。
 */
export function auditVideoNode(
  nodeId: string,
  nodes: CanvasNode[],
  edges: CanvasEdge[],
  loraEngines?: Record<string, string[] | undefined>,
): RunAudit {
  const node = nodes.find((n) => n.id === nodeId);
  if (!node || node.type !== "videoGen") return { mode: "", inputs: [], conflicts: [] };
  const d = node.data as VideoGenNodeData;
  const inferred = inferVideoMode(nodeId, nodes, edges);
  const mode = inferred.mode;
  const engine = d.engine ?? "";

  const inputs: AuditInput[] = [];
  const conflicts: string[] = [];
  const push = (n: CanvasNode, role: string, used: boolean) =>
    inputs.push({ nodeId: n.id, from: TYPE_LABEL[n.type ?? ""] ?? n.type ?? "", role, used });

  for (const e of edges.filter((e) => e.target === nodeId)) {
    const src = nodes.find((n) => n.id === e.source);
    if (!src) continue;
    switch (src.type) {
      case "prompt": case "shot": case "shotContext": case "skill":
        push(src, "提示词", true); break;
      case "panorama":
        push(src, "提示词 + 锁死 2:1 画幅", true); break;
      case "character": {
        const hasImg = Boolean((src.data as { avatarUrl?: string }).avatarUrl);
        if (mode === "r2v") {
          hasImg ? push(src, "身份参考图（占一个 <Picture> 槽位）", true)
                 : push(src, "这个角色没有图 —— 参考生视频只认图，它的外观描述不会发出去", false);
        } else {
          push(src, "外观描述进提示词", true);
        }
        break;
      }
      case "imageGen": case "upscale": {
        // **上游还没出图，和「文生不吃图」是两件事。**
        //
        // 图还没生成时 `videoInputReferences` 拿不到路径，模式就推成 t2v ——
        // 照那个报「这张图不会发出去」是**误报**：等图出来它就是 i2v 了。
        // 用户会以为连线连错了去改图，而图本来就是对的（2026-09-21 自查发现）。
        const ready = Boolean(
          (src.data as { outputImageUrl?: string; outputUrl?: string }).outputImageUrl ??
          (src.data as { outputUrl?: string }).outputUrl,
        );
        // 还没出图：点生成时执行器会先把它跑出来，模式推导也按「会有这张图」算（`pendingUpstreamImages`）——
        // 所以它**会用上**，不该报成「连了但没用上」（2026-09-24 用户截图：上面说文生视频、下面说这张没用上，两处打架）
        if (!ready) push(src, mode === "motion" ? "人物图 · 还没出，点生成时先跑它" : mode === "i2v" ? "首帧 · 还没出，点生成时先跑它" : "参考图 · 还没出，点生成时先跑它", true);
        else if (mode === "t2v") push(src, "文生视频不吃图 —— 这张图不会发出去", false);
        else if (mode === "motion") push(src, inferred.via === "video" ? "人物图（原片复刻里按连线顺序是图片 1、图片 2……）" : "人物图（动作由深度定）", true);
        else if (mode === "fl2v") push(src, "首帧或尾帧", true);
        else if (mode === "r2v") push(src, "参考图", true);
        else push(src, "首帧", true);
        break;
      }
      case "videoGen": {
        // **这条边什么都不传**（`videoInputReferences` 遇到 videoGen 直接 return）。
        //
        // 它曾经是长镜头的窗口链，2026-09-21 改成「一个节点一张窗口表」之后
        // 就彻底没有用处了（见 `longTakeWindows` 的模块注释）。**旧画布上
        // 这种线还在**，而它现在一声不吭地什么也不做 —— 正是这个预检要抓的。
        // 2026-09-24 起有一个例外：节点选了原片复刻，这段视频就是原片（参考视频 1）
        if (mode === "motion" && inferred.via === "video") push(src, "原视频（原片复刻的参考视频 1）", true);
        else push(src, "视频连视频不传任何东西 —— 要拿它做原片复刻，在这个节点上点「设成原片复刻」；长镜头改成在一个节点里加窗口了", false);
        break;
      }
      case "depthCapture":
        // 接上深度就**强制**切动作迁移（见 `inferVideoMode` 的早退）——
        // 所以不存在「连了深度却没用上」这种情况，不写那个到不了的分支。
        push(src, "动作深度（模式被强制切成动作迁移）", true);
        break;
      case "lora": {
        const name = String((src.data as { name?: string }).name ?? "");
        if (!name) { push(src, "还没选 LoRA 文件", false); break; }
        const fits = loraFits(loraEngines?.[name], engine);
        if (!fits) {
          conflicts.push(`LoRA「${name}」挂不上 ${engine} —— 架构对不上，挂上去不生效且不报错`);
          push(src, `架构对不上 ${engine}`, false);
        } else if (isBakedIn(name)) {
          conflicts.push(`LoRA「${name}」已经内置在采样预设里了，再挂一次是重复挂载`);
          push(src, "重复挂载", false);
        } else {
          push(src, "LoRA", true);
        }
        break;
      }
      case "panoViewer": case "directorWorld":
        // 这两个的截图是靠它们自己往下游**新建图片节点**送的（`addImageGroup`），
        // 直连过来收不到任何东西。
        push(src, "截图要用它自己生成的那些图片节点，这条直连不传图", false);
        break;
      default:
        push(src, "不贡献任何输入", false);
    }
  }

  // ── 确定会出错的 ──
  const refs = inferred.images ?? [];
  if (mode === "r2v" && engine !== "h3") conflicts.push("参考生视频只有 MiniMax H3 支持，当前引擎出不了");
  if (mode === "r2v" && refs.length > MAX_VIDEO_REFERENCES)
    conflicts.push(`参考图 ${refs.length} 张，超过上限 ${MAX_VIDEO_REFERENCES} 张`);
  if (mode === "fl2v" && refs.length < 2)
    conflicts.push("首尾帧要两张图：一张首帧、一张名字里带「尾帧」的");

  return { mode, modeReason: inferred.overridden && !inferred.deliberate ? inferred.reason : undefined, inputs, conflicts };
}

/** 被忽略的那些。界面上只要显示这一部分就够了 —— 生效的不用说。 */
export function ignoredInputs(a: RunAudit): AuditInput[] {
  return a.inputs.filter((i) => !i.used);
}
