import { invoke } from "@tauri-apps/api/core";

/**
 * 风格定调。
 *
 * 预设的**文字**改自 LumenX（MIT），**缩略图由本机引擎重出** ——
 * 他们那份是 wanx/gpt-image 出的，我们的编码器不同，
 * 同一段词出来不是同一张画。
 */
export interface StyleCategory {
  id: string;
  name: string;
  name_zh: string;
  sort_order: number;
}

export interface StylePreset {
  id: string;
  category: string;
  name: string;
  name_zh: string;
  subtitle_zh: string;
  description: string;
  best_for: string[];
  avoid_for: string[];
  positive_prompt: string;
  negative_prompt: string;
  sample_prompt: string;
  /** 我们自己出的缩略图；null = 还没出，显示占位（不显示别人的图） */
  thumb_url: string | null;
}

export interface StyleCatalog {
  categories: StyleCategory[];
  presets: StylePreset[];
  thumbs_ready: number;
}

export interface ArtDirection {
  preset_id: string;
  /** 留空 = 用预设的，不是「不要风格」 */
  positive_prompt: string;
  negative_prompt: string;
  /**
   * 镜头与表演偏好（本项目）。生成镜头表、细化时间线、合成提示词时当成「用户定下的」规则带进去。
   * 老版本存的设置里没有这个键，所以可选。
   */
  shot_notes?: string;
  updated_at: number;
}

export interface StyleSuggestion {
  name: string;
  description: string;
  reason: string;
  positive_prompt: string;
  negative_prompt: string;
}

export const styleCatalog = () => invoke<StyleCatalog>("style_catalog");

/**
 * 构图模板的示例图 URL，按模板 id。没出过的**不在这个表里** ——
 * 界面显示占位，不显示别的模板的图。
 *
 * 出图脚本：`cargo run --bin render-template-thumbs`
 */
export const styleTemplateThumbs = () =>
  invoke<Record<string, string>>("style_template_thumbs");
export const styleGet = (projectId: string) =>
  invoke<ArtDirection>("style_get", { projectId });
export const styleSave = (projectId: string, art: ArtDirection) =>
  invoke<ArtDirection>("style_save", { projectId, art });
export const styleAnalyze = (script: string) =>
  invoke<StyleSuggestion[]>("style_analyze", { script });

