import type { AssetGen, LoraPick } from "../../../api/assets";
import type { ModelSettings } from "../../../api/cinema";

/**
 * 这次出图**实际**用什么：引擎、画幅、挂哪些 LoRA。
 *
 * # 两层：项目设置 → 资产覆盖
 *
 * 项目的生成设置是底，单个资产可以覆盖其中任意一项。
 * 「这个角色要挂专属 LoRA」「这张场景图要横幅」是常见需求，
 * 但不该逼人为此改整个项目的设置。
 *
 * # 一处必须分清的语义
 *
 * `loras: []`（空数组）和 `loras: undefined` **不是一回事**：
 *
 * - `undefined` = 没设过 → 跟项目走
 * - `[]` = **明确不挂** → 就是不挂
 *
 * 不分开的话，用户把 LoRA 全取消之后，下次打开又冒出来了，
 * 而且没有任何地方告诉他为什么。
 */
export interface EffectiveGen {
  engine: string;
  aspect: string;
  loras: LoraPick[];
  /** 哪几项是这个资产自己覆盖的 —— 界面上要标出来「这里和项目设置不一样」。 */
  overridden: { engine: boolean; aspect: boolean; loras: boolean };
}

const FALLBACK = {
  engine: "zimage",
  aspect: { character: "portrait", scene: "landscape", prop: "square" },
} as const;

export function effectiveGen(
  kind: string,
  gen: AssetGen | null | undefined,
  settings: ModelSettings | null | undefined,
): EffectiveGen {
  const projectAspect =
    kind === "scene"
      ? (settings?.scene_aspect ?? FALLBACK.aspect.scene)
      : kind === "prop"
        ? (settings?.prop_aspect ?? FALLBACK.aspect.prop)
        : (settings?.character_aspect ?? FALLBACK.aspect.character);

  return {
    engine: gen?.engine ?? settings?.t2i_engine ?? FALLBACK.engine,
    aspect: gen?.aspect ?? projectAspect,
    // 项目级没有 LoRA 设置 —— 不挂是默认
    loras: gen?.loras ?? [],
    overridden: {
      engine: !!gen?.engine && gen.engine !== (settings?.t2i_engine ?? FALLBACK.engine),
      aspect: !!gen?.aspect && gen.aspect !== projectAspect,
      loras: gen?.loras !== undefined && gen.loras !== null,
    },
  };
}

/**
 * 把界面上的选择收成一份要存的覆盖。
 *
 * **和项目设置一样的那一项就不存。** 存下来的话，以后改了项目设置，
 * 这个资产会莫名其妙不跟着变 —— 而用户根本不记得自己"设"过它。
 */
export function toOverrides(
  kind: string,
  chosen: { engine: string; aspect: string; loras: LoraPick[] },
  settings: ModelSettings | null | undefined,
): AssetGen {
  const base = effectiveGen(kind, null, settings);
  return {
    engine: chosen.engine === base.engine ? undefined : chosen.engine,
    aspect: chosen.aspect === base.aspect ? undefined : chosen.aspect,
    // LoRA 没有项目级默认，所以只有「一个都没挂」才算回到默认
    loras: chosen.loras.length === 0 ? undefined : chosen.loras,
  };
}
