import { mergeTriggers, studioCapabilities, studioLoras } from "../../api/studio";
import type { ImageGenNodeData } from "../types";

export async function canvasLoraConfig(data: Pick<ImageGenNodeData, "loras" | "injectTriggers">, engine: string, prompt: string) {
  const loras = data.loras ?? [];
  if (!loras.length) return { prompt, loras, added: [] as string[] };
  const caps = await studioCapabilities();
  if (!caps.engines.find((e) => e.id === engine)?.loras) throw new Error("当前模型不支持自定义 LoRA，请清空已选或更换模型");
  const catalog = await studioLoras();
  const triggers: string[] = [];
  for (const pick of loras) {
    const entry = catalog.find((l) => l.name === pick.name);
    if (!entry || !entry.engines.includes(engine)) throw new Error(`LoRA 不存在或不兼容当前模型：${pick.name}`);
    if (!Number.isFinite(pick.strength) || pick.strength < 0 || pick.strength > 1.5) throw new Error(`LoRA 权重无效：${pick.name}`);
    if (pick.strength > 0) triggers.push(...(entry.confirmed_triggers.length ? entry.confirmed_triggers : entry.triggers));
  }
  const unique = [...new Map(triggers.map((t) => [t.trim().toLowerCase(), t.trim()])).values()];
  const merged = mergeTriggers(prompt, data.injectTriggers === false ? [] : unique);
  return { ...merged, loras };
}
