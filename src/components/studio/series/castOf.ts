/**
 * 「本集选角」：系列池里哪些资产在这一集出场。
 *
 * 判据两条，满足一条就算：
 * 1. 分镜里按 id 引用了它（出场角色 / 场景 / 道具 / 参考图槽位）—— 最可靠
 * 2. 剧本里出现了它的名字 —— 还没出分镜时只能靠这个
 *
 * **名字至少两个字才按名字认。** 单字名（「七」「白」）在任何一段中文里
 * 都可能出现，按它匹配会把半个系列都算进来。
 */
import type { EpisodeData } from "../../../api/cinema";
import type { AssetView } from "../../../api/assets";

function shotRefs(ep: EpisodeData): Set<string> {
  const ids = new Set<string>();
  for (const s of ep.storyboard ?? []) {
    const raw = s as unknown as Record<string, unknown>;
    for (const k of ["character_ids", "prop_ids"]) {
      const v = raw[k];
      if (Array.isArray(v)) for (const x of v) if (typeof x === "string") ids.add(x);
    }
    for (const k of ["character_id", "scene_id"]) {
      const v = raw[k];
      if (typeof v === "string" && v) ids.add(v);
    }
    const slots = raw["ref_slots"];
    if (Array.isArray(slots)) {
      for (const sl of slots) {
        const id = (sl as { asset_id?: unknown })?.asset_id;
        if (typeof id === "string" && id) ids.add(id);
      }
    }
  }
  return ids;
}

export function castOf(series: AssetView[], ep: EpisodeData | null): AssetView[] {
  if (!ep) return [];
  const refs = shotRefs(ep);
  const text = ep.script_text ?? "";
  return series.filter((a) => {
    if (refs.has(a.id)) return true;
    const name = a.name.trim();
    return name.length >= 2 && text.includes(name);
  });
}
