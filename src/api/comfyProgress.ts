import { invoke } from "@tauri-apps/api/core";

/**
 * ComfyUI 跑到哪一步了。
 *
 * 数据来自 ComfyUI 的日志 —— `/queue` 和 `/history` 都不含进度，
 * 而 H3/WeeTodd 的视频节点也不走 ComfyUI 的 websocket `progress` 事件，
 * 它自己往 stdout 打 `step 2/4  706.8s  eta 24.0 min`。
 * 详见 Rust `comfy_progress` 的模块注释。
 */
export interface ComfyProgress {
  /** 给人看的一句话。**永远有值** —— 拿不到步数时是「已跑 N 分钟」 */
  text: string;
  step: number | null;
  total: number | null;
  /** 估计还要多少秒。**拿不到步数时是 null** —— 不编 */
  eta_secs: number | null;
}

export const comfyProgress = (startedAt?: number) =>
  invoke<ComfyProgress>("comfy_progress", { startedAt: startedAt ?? null });
