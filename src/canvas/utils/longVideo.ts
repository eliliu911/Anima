import { CONTEXT_FRAMES } from "../../components/studio/storyboard/longTake";

/**
 * 画布的**超长视频**：H3 时长拉过 15 秒，还是一段提示词、点的还是「生成」，后台分段串成一整条（Rust `long_video`）。
 * 这里只算「分几段、每段多长」给界面显示 —— **和后端同一个公式**（`long_shot::plan`），两边各算各的会漂。
 */
export const MAX_LONG_VIDEO_SECS = 75;
/** H3 一次最长多少秒；超过就分段 */
export const H3_SINGLE_MAX_SECS = 15;

export function longVideoPlan(total: number): { windows: number; windowSecs: number } | null {
  if (!(total > H3_SINGLE_MAX_SECS + 0.01)) return null;
  const ov = CONTEXT_FRAMES / 24;
  const n = Math.max(2, Math.ceil((total - ov) / (H3_SINGLE_MAX_SECS - ov)));
  const w = (total + (n - 1) * ov) / n;
  return { windows: n, windowSecs: Math.round(w * 100) / 100 };
}
