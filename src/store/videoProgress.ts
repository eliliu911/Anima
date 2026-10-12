import { create } from "zustand";

/**
 * 视频生成的实时进度。
 *
 * 后端（`video_tool.rs`）在 6 个阶段和每一条失败路径上都发
 * `RuntimeEvent::VideoGenerationProgress`，`agent.rs` 按心跳转发成
 * `anima://video-generation-progress`。
 *
 * 支持双重槽位寻址：优先按 call_id 精准对齐，同时兜底兼容 prompt+时长。
 */
export interface VideoProgress {
  callId?: string;
  /** connecting / model / queued / running / downloading / done / error */
  stage: string;
  /** 后端给的中文短标签，直接显示 */
  label: string;
  /** 0~1 */
  progress: number;
  outputPath?: string;
  /** 仅失败时有：具体原因 */
  message?: string;
}

/** 卡片和进度共用的归档键。两边必须一致，否则对不上。 */
export function videoProgressSlot(prompt: string, durationSecs?: number, callId?: string): string {
  if (callId && callId.trim().length > 0) {
    return `call:${callId}`;
  }
  return `${prompt} ${durationSecs ?? 4}s`;
}

interface VideoProgressState {
  /** slot -> 最新进度 */
  bySlot: Record<string, VideoProgress>;
  apply: (slot: string, progress: VideoProgress) => void;
  clear: () => void;
}

export const useVideoProgress = create<VideoProgressState>((set) => ({
  bySlot: {},
  apply: (slot, progress) =>
    set((state) => {
      const prev = state.bySlot[slot];
      // 成功不能被后来的失败盖回去
      if (prev?.stage === "done" && progress.stage !== "done") {
        return state;
      }
      return { bySlot: { ...state.bySlot, [slot]: progress } };
    }),
  clear: () => set({ bySlot: {} }),
}));
