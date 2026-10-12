/**
 * 风格广场 —— 拿**本片自己的镜头**铺开多个风格，用来挑。
 *
 * 和内置缩略图（`resources/style_thumbs/`）的区别：那 15 张是通用样例，
 * 离线脚本用每个预设的 `sample_prompt` 出的；这里出的是**你这个剧本的画面**。
 * 所以结果落在项目目录下，不进版本库，换项目互不干扰。
 */
import { invoke } from "@tauri-apps/api/core";
import type { Job } from "./jobs";

export interface StyleCompareSpec {
  /** 镜头提示词（**不含**风格词 —— 风格词由后端按 apply_style 拼） */
  shot_prompt: string;
  /** 要比哪些风格。空数组 = 全部 */
  preset_ids: string[];
  width: number;
  height: number;
  /** 所有格子共用同一个 seed，不同 seed 的对照没有意义 */
  seed: number;
  /** 锁构图用的控制图（暂未接线，留着占位） */
  control_image?: string | null;
}

export function styleCompareStart(projectId: string, spec: StyleCompareSpec): Promise<Job> {
  return invoke<Job>("style_compare_start", { projectId, spec });
}

/** 已出的对照图：`[风格 id, 绝对路径][]` */
export function styleCompareList(projectId: string): Promise<[string, string][]> {
  return invoke<[string, string][]>("style_compare_list", { projectId });
}
