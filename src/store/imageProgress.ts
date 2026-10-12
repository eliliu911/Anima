import { create } from "zustand";

/**
 * 图片生成的实时进度。
 *
 * 后端（`image_tool.rs`）在 6 个阶段和**每一条失败路径**上都发
 * `RuntimeEvent::ImageGenerationProgress`，`agent.rs` 按心跳转发成
 * `anima://image-generation-progress`。在这之前**前端根本没订阅**，
 * 于是超时或报错的那张图会永远停在骨架屏上转 —— 用户看到的是
 * 「第二版超时了」却没有任何失败提示，只能一直等。
 *
 * 按 **prompt + 尺寸** 归档而不是 call_id：模型超时后会用同一个 prompt
 * **重试**，那是一次新的工具调用（新 call_id）。按 call_id 存的话，
 * 重试的进度会开出一条新记录，而卡片是按 prompt 去重的（见
 * `getTurnImageCalls`），两边对不上 —— 卡片仍旧显示旧那条的失败态。
 */
export interface ImageProgress {
  /** connecting / model / queued / running / downloading / done / error */
  stage: string;
  /** 后端给的中文短标签，直接显示 */
  label: string;
  /**
   * 这一张用的是哪个底模（`Z-Image Turbo` / `Chroma1-HD` / `SDXL`）。
   *
   * 骨架屏必须显示它 —— 三个引擎实测耗时 45s / 331s / 39s，差 8.5 倍。
   * 不说在用哪个，等 5 分钟和等 45 秒在界面上是一模一样的，
   * 用户只会以为卡住了。
   */
  engine?: string;
  /** 0~1 */
  progress: number;
  outputPath?: string;
  /** 仅失败时有：具体原因 */
  message?: string;
}

/**
 * 卡片和进度共用的归档键。两边必须一致，否则对不上。
 *
 * **不要把 seed 加进这个键。** 试过一次，是错的：
 *
 * 加了之后「同提示词出多张」确实不再折叠，但代价是**满屏骨架屏** ——
 * 一次三张就是三个大方块占满整屏，比原来的问题糟得多。而且这个键在
 * 代码里有**三处**在拼（这里、`getTurnImageCalls`、`MarkdownRenderer`
 * 的查表），只改了前两处，渲染那处仍按老键查 —— 进度存进了没人读的格子，
 * 徽章、进度、队列信息全部消失，骨架屏永远转。
 *
 * ComfyUI 是单实例串行的，同一时刻只有一个任务在跑。所以「一张卡 +
 * 当前在跑那个的引擎和队列」本来就是准确的描述，不需要拆成多张。
 */
export function progressSlot(prompt: string, width?: number, height?: number): string {
  return `${prompt}${width ?? 1024}x${height ?? 1024}`;
}

interface ImageProgressState {
  /** slot -> 最新进度 */
  bySlot: Record<string, ImageProgress>;
  apply: (slot: string, progress: ImageProgress) => void;
  clear: () => void;
}

export const useImageProgress = create<ImageProgressState>((set) => ({
  bySlot: {},
  apply: (slot, progress) =>
    set((state) => {
      const prev = state.bySlot[slot];
      // **成功不能被后来的失败盖回去。**
      //
      // 模型超时后重试，同一个 slot 会先后收到「error」和新一轮的进度。
      // 若已经 done 了，之后再来的 error 是**上一次**那趟的尾巴，不该改写结果 ——
      // 否则用户会看到图已经出来了、卡片却写着失败。
      if (prev?.stage === "done" && progress.stage !== "done") {
        return state;
      }
      return { bySlot: { ...state.bySlot, [slot]: progress } };
    }),
  clear: () => set({ bySlot: {} }),
}));
