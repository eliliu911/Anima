import type { GenerateArgs, GenerateOutput } from "../../api/studio";
import type { DepthVideoResult } from "../../api/canvasDepth";
import { canvasJobStart, waitForCanvasJob, type CanvasTask } from "../../api/canvasJobs";
import { useCanvasStore } from "../store/useCanvasStore";

/**
 * 画布的生成调用，**全部走后台任务**。
 *
 * 函数名、参数和原来直接调后端的那几个（`studioGenerate` / `canvasDepthCapture` …）一模一样，
 * 执行器只改了从哪 import。差别在里面：先登记任务、把任务 id 记到节点上，再等结果 ——
 * 页面中途重新加载的话，`resumeCanvasJobs` 按节点上的 id 把结果取回来。
 *
 * **是哪个节点**靠 `inNode` 传进来：它在同步调用的那一瞬间设上、立刻清掉，
 * 任务函数在第一个 await 之前就读走，所以几个节点同时跑也不会串。
 */
export interface NodeCtx {
  projectId: string | null;
  nodeId: string;
  label: string;
}

let current: NodeCtx | null = null;

export function inNode<T>(ctx: NodeCtx, fn: () => T): T {
  const prev = current;
  current = ctx;
  try {
    return fn();
  } finally {
    current = prev;
  }
}

/** 这一轮页面里正在由执行器等着的任务 —— 恢复时跳过，别两边一起写回 */
export const liveJobs = new Set<string>();

async function run<T>(task: CanvasTask, fallbackLabel: string): Promise<T> {
  const ctx = current;
  if (!ctx) throw new Error("画布任务没有绑定节点（内部错误）");
  const jobId = await canvasJobStart({ projectId: ctx.projectId, nodeId: ctx.nodeId, label: ctx.label || fallbackLabel, task });
  liveJobs.add(jobId);
  useCanvasStore.getState().updateProjectNodeData(ctx.projectId, ctx.nodeId, { jobId, jobKind: task.kind });
  try {
    const job = await waitForCanvasJob(jobId);
    if (job.status === "failed") throw new Error(job.error || "没有返回原因");
    return job.result as T;
  } finally {
    liveJobs.delete(jobId);
    useCanvasStore.getState().updateProjectNodeData(ctx.projectId, ctx.nodeId, { jobId: undefined, jobKind: undefined });
  }
}

export async function studioGenerate(args: GenerateArgs): Promise<GenerateOutput> {
  return run<GenerateOutput>({ kind: "generate", args: args as unknown as Record<string, unknown> }, "生成");
}

/**
 * 长镜头：把链上每个窗口的提示词和参考图一次发下去，出一个 MP4。
 *
 * **产物不记在任何一个窗口节点名下** —— 它是整条链的片子，
 * 记成「第一个节点的输出」会让人以为那就是那一窗口的成果。
 * 落在链头节点的 `longTakeUrl` 上，界面上标明是整条链的。
 */
export async function canvasLongTake(args: {
  windows: { prompt: string; images: string[] }[];
  windowSecs: number;
  contextFrames?: number;
  width: number;
  height: number;
  generateAudio?: boolean;
  /** 节点上选的 H3 加速档（和单条出片同一套 id）。不给后端用 Standard */
  h3Quality?: string;
  /** 节点上选的 H3 采样方法。不给 = euler */
  h3Sampling?: string;
  /** 节点上写的 seed。不给后端随机 */
  seed?: number;
  /** 节点选的 LoRA（已按引擎核过、激活词已拼进各窗口提示词，见 `canvasLoraConfig`） */
  loras?: { name: string; strength: number }[];
}): Promise<{ video_path: string; windows: number; net_secs: number }> {
  return run(
    {
      kind: "long_take",
      args: {
        windows: args.windows,
        window_secs: args.windowSecs,
        context_frames: args.contextFrames,
        width: args.width,
        height: args.height,
        generate_audio: args.generateAudio !== false,
        ...(args.h3Quality ? { h3_quality: args.h3Quality } : {}),
        ...(args.h3Sampling ? { h3_sampling: args.h3Sampling } : {}),
        ...(args.seed ? { seed: args.seed } : {}),
        ...(args.loras?.length ? { loras: args.loras } : {}),
      },
    },
    "长镜头",
  );
}

export async function canvasDepthCapture(videoPath: string): Promise<{ path: string }> {
  return run({ kind: "depth_capture", args: { video_path: videoPath } }, "深度动作捕捉");
}

export async function canvasDepthVideo(args: {
  depthPath: string; imagePath: string; imagePaths?: string[]; control?: "canny" | "depth"; prompt: string; startSecs?: number; seed?: number;
  generateAudio?: boolean; engine?: "ltx" | "h3" | "h3_ref"; resolution?: string; durationSecs?: number; keepBackground?: boolean; sourceVideoPath?: string;
}): Promise<DepthVideoResult> {
  return run({
    kind: "depth_video",
    args: {
      depth_path: args.depthPath, image_path: args.imagePath, image_paths: args.imagePaths ?? [], control: args.control ?? null, prompt: args.prompt,
      start_secs: args.startSecs ?? 0, seed: args.seed ?? null, generate_audio: Boolean(args.generateAudio),
      engine: args.engine ?? "ltx", resolution: args.resolution ?? null, duration_secs: args.durationSecs ?? null,
      keep_background: Boolean(args.keepBackground),
      source_video_path: args.sourceVideoPath ?? null,
    },
  }, "动作迁移");
}

export async function canvasImageOp(imagePath: string, op: "remove_bg" | "upscale"): Promise<{ path: string }> {
  return run({ kind: "image_op", args: { image_path: imagePath, op } }, op === "remove_bg" ? "抠图" : "超分");
}

/**
 * 视频超分 ×2。默认 RealESRGAN：2026-09-14 同题实测 125 秒，效果和 LTX 生成式超分（3000 秒）几乎一样，
 * 镜头运动时还不出重影 —— 用户逐个看过后定的。
 */
export async function canvasVideoUpscale(videoPath: string): Promise<{ path: string }> {
  return run({ kind: "video_upscale", args: { video_path: videoPath, refine: false, prompt: "" } }, "超分");
}

export async function canvasVideoInterpolate(videoPath: string): Promise<{ path: string }> {
  return run({ kind: "interpolate", args: { video_path: videoPath } }, "补帧");
}
