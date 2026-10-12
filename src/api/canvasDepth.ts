import { invoke } from "@tauri-apps/api/core";

/**
 * 画布「深度动作捕捉」链路的三个命令，对应 `src-tauri/src/depth_capture.rs`。
 *
 * ```text
 * [视频] ──→ [深度动作捕捉] ──→ [视频：动作迁移] ──→ 新视频
 *                                   ↑
 *                     [人物图] ─────┘
 * ```
 */

/** 保存上传的视频，返回本机绝对路径。 */
export function canvasSaveVideo(dataUrl: string, filename: string): Promise<{ path: string }> {
  return invoke("canvas_save_video", { args: { data_url: dataUrl, filename } });
}

/** 保存上传的音频（画布「音频素材」节点），返回本机绝对路径和时长（读不出时长是 null） */
export function canvasSaveAudio(dataUrl: string, filename: string): Promise<{ path: string; secs: number | null }> {
  return invoke("canvas_save_audio", { args: { data_url: dataUrl, filename } });
}

export interface DepthVideoResult {
  path: string;
  /** 自动对位后的首帧 —— 模型收到的第一帧就是它 */
  first_frame: string;
  notes: string[];
}

