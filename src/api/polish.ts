import { invoke } from "@tauri-apps/api/core";

/**
 * AI 润色：草稿 → 双语提示词。
 *
 * 每条链路各对应「设置 → 提示词」里的一个槽位，改那里就改这里 ——
 * **不要在前端再拼一份提示词**，两份真相会让「我改了没生效」重演。
 */
export type PolishKind = "storyboard" | "video" | "t2v" | "r2v" | "prose";

export interface Polished {
  prompt_cn: string;
  prompt_en: string;
  /**
   * 有值 = 结果能用但有话要说（目前只有「模型基本没改」）。
   *
   * **模型回拒绝不走这里，走的是抛错** —— 那种结果不能当提示词用，
   * 放行的话「我无法协助」会被原样发给出图模型。
   */
  warning?: string;
}

export interface PolishArgs {
  kind: PolishKind;
  draft: string;
  /** 迭代时传上一版中文，让模型有个中文锚点去定位反馈。 */
  prevCn?: string;
  feedback?: string;
  /** 分镜润色的可用资产清单（`{ASSETS}`）。 */
  assets?: string[];
  /** 参考生视频的参考槽清单（`{SLOTS}`）。 */
  slots?: string[];
  /** 决定 `{MAX_CHARS}` —— 逐引擎，SDXL 超过 150 字会被静默摊薄。 */
  engine?: string;
}

export function cinemaPolishPrompt(a: PolishArgs): Promise<Polished> {
  return invoke<Polished>("cinema_polish_prompt", {
    kind: a.kind,
    draft: a.draft,
    prevCn: a.prevCn ?? "",
    feedback: a.feedback ?? "",
    assets: a.assets ?? null,
    slots: a.slots ?? null,
    engine: a.engine ?? null,
  });
}
