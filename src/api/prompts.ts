import { invoke } from "@tauri-apps/api/core";

/**
 * 各 AI 环节的 system prompt。
 *
 * 留空 = 用内置默认，填了整段替换。**模型不在这里选** ——
 * 走「设置 → 模型中枢 → 链路分配」里的「后台推理」那一条，那是这个项目里唯一决定
 * 「本轮用哪个模型」的地方，在这儿再放一个就是第二份真相。
 */
export interface PromptSlotView {
  key: string;
  label: string;
  /** 这个槽位认的占位符，如 {TEXT} / {DRAFT} */
  placeholders: string[];
  default_text: string;
  /** 用户改过的；没改过是空串 */
  custom_text: string;
  overridden: boolean;
  /** 这份覆盖是照旧版内置默认改的，内置的那份后来变了。**不自动迁移** —— 那是用户的文本 */
  default_changed: boolean;
  /**
   * `{METHOD}` 换成哪家技能的原文（编译进程序，编辑框里看不到）；没有 `{METHOD}` 的槽位是 null。
   * 不标的话，用户会以为编辑框里那几行就是全部规则。
   */
  method_source: string | null;
}

export const promptSlots = () => invoke<PromptSlotView[]>("prompt_slots");

/** 传空字符串 = 恢复默认 */
export const promptSaveSlot = (key: string, text: string) =>
  invoke<PromptSlotView[]>("prompt_save_slot", { key, text });

/**
 * 一次试跑的结果。
 *
 * `checked: false` 时 `issues` 恒为空 —— **界面要说「这一槽没法验」**，
 * 空的问题列表会被读成「验过了，没问题」。
 */
export interface TryOutcome {
  raw: string;
  checked: boolean;
  issues: string[];
}

/**
 * 试跑一个槽位，并用**生产链路的解析器**验一遍输出。
 *
 * `draft` 是编辑框里那一份，语义是「就用这段跑」：非空 = 用这段，
 * 空串 = 用内置默认。**不要在和默认文案相同时省掉它** ——
 * 那样后端会去读已保存的自定义覆盖，跑的和屏幕上显示的不是同一份。
 *
 * **不落盘** —— 试跑的意思是「先看看」，不该有副作用。
 */
export const promptTry = (key: string, sample: string, draft: string) =>
  invoke<TryOutcome>("prompt_try", { key, sample, draft });

/** 导出整份提示词覆盖（JSON 文本）。换机器、回滚、分享调参结果用。 */
export const promptExport = () => invoke<string>("prompt_export");

/** 从一段 JSON **整份替换**提示词覆盖 —— 不是合并（合并会得到两份的混合，谁都说不清）。 */
export const promptImport = (text: string) => invoke<PromptSlotView[]>("prompt_import", { text });
