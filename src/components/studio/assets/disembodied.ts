import type { CharacterTraits } from "../../../api/assets";

/**
 * 改「只有声音」这一位，**别的一位都不动**。
 *
 * # 为什么值得单独一个函数
 *
 * `assets_update` 的 `character` 是**整份替换**，不是逐字段合并
 * （`assets.rs`：`if let Some(v) = req.character { a.character = Some(v) }`）。
 * 所以随手传一个 `{ disembodied: true }` 会把 persona、声音 id、
 * 视觉 DNA、设定卡全部清空 —— 而且不报错，下次出图才发现脸变了。
 *
 * 角色还没有设定时给一份空的，而不是不传 —— 不传的话开关存不下去，
 * 表现为「点了没反应」。
 */
export function withDisembodied(
  traits: CharacterTraits | null | undefined,
  on: boolean,
): CharacterTraits {
  return { ...base(traits), disembodied: on };
}

/**
 * 改「音色」这一位，别的一位都不动。同上，**必须铺开** ——
 * 只传 `{ voice_id }` 会把设定卡和视觉 DNA 洗掉，而且不报错。
 *
 * 空串存成 `null`：配音那边把空串当「没配」回落到全局音色
 * （`cinema_dub::plan`），存成 `""` 和存成 `null` 效果一样，
 * 但读出来一个是「配了个空的」一个是「没配」——留一种就够了。
 */
export function withVoiceId(
  traits: CharacterTraits | null | undefined,
  voice: string,
): CharacterTraits {
  return { ...base(traits), voice_id: voice.trim() || null };
}

/** 角色还没有设定时的空壳。**不传的话开关存不下去，表现为「点了没反应」**。 */
function base(traits: CharacterTraits | null | undefined): CharacterTraits {
  return (
    traits ?? {
      persona: "",
      trigger_words: "",
      seed: null,
      lora_model: null,
      lora_weight: null,
      consistency_mode: null,
      clothing_prompt: null,
      voice_id: null,
      asset_sheet: null,
      visual_dna: null,
    }
  );
}
