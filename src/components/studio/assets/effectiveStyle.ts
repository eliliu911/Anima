import type { ArtDirection, StyleCatalog } from "../../../api/style";

/**
 * 这个项目**实际**会用的风格词。
 *
 * `ArtDirection.positive_prompt` 留空的语义是「用预设自带的」，
 * **不是「不要风格」**。直接读那个字段的话，界面上会显示成
 * 「风格基线：无」，而出图时后端照样把预设的词拼进去 ——
 * 看到的和发出去的不是一个东西，且不报错。
 */
export interface EffectiveStyle {
  /** 预设名。找不到预设时是空串。 */
  name: string;
  positive: string;
  negative: string;
}

export function effectiveStyle(
  art: ArtDirection | null,
  catalog: StyleCatalog | null,
): EffectiveStyle {
  if (!art) return { name: "", positive: "", negative: "" };
  const preset = catalog?.presets.find((p) => p.id === art.preset_id) ?? null;
  return {
    name: preset?.name ?? "",
    // 覆盖了就用覆盖的，**空串表示没覆盖**，回落到预设
    positive: art.positive_prompt.trim() || preset?.positive_prompt || "",
    negative: art.negative_prompt.trim() || preset?.negative_prompt || "",
  };
}
