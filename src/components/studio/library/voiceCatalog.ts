/**
 * 音色目录：**从资产列表里算出来**，不另开接口。
 *
 * # 为什么不问后端
 *
 * 资产库本来就把所有资产连 `character.voice_id` 一起读进来了。
 * 再开一个「谁在用这个音色」的命令，就是同一份事实的第二个所有者 ——
 * 这个仓库在「一份清单抄在多处」上栽过很多次。两边一旦走散，
 * 表现是「资产库说 3 个角色在用，音色页说 2 个」，而没人知道哪个对。
 *
 * # 音色不是资产
 *
 * 角色 / 场景 / 道具是我们自己的数据（存在 `assets.json`、能出图、能挂参考图）。
 * 音色是**引擎的东西**：换个引擎整份列表就变，Edge 的在微软那边、
 * Kokoro 的在服务端 —— 既删不掉也改不了。所以这一页是**目录**，不是资产库：
 * 列出来、试听、看谁在用；只有本机音色库（GPT-SoVITS）那部分能增删。
 */
import type { AssetView } from "../../../api/assets";

/** 一个音色，以及用它的角色。 */
export interface VoiceEntry {
  name: string;
  /** 当前引擎报得出这个音色。假 = 角色配了但引擎里没有（**配了也不生效**） */
  inEngine: boolean;
  /** 用它的角色，按名字排 */
  usedBy: AssetView[];
}

/** 一个角色配的音色名（空串 = 没配）。 */
export function voiceOf(a: AssetView): string {
  return (a.character?.voice_id ?? "").trim();
}

/**
 * 引擎音色 + 角色实际配的，合成一份目录。
 *
 * **角色配了、但引擎里没有的也要列出来**（`inEngine: false`）——
 * 那种情况配音时会回落到全局音色，**不报错**。不列出来就永远发现不了。
 */
export function buildCatalog(engineVoices: string[], assets: AssetView[]): VoiceEntry[] {
  const used = new Map<string, AssetView[]>();
  for (const a of assets) {
    if (a.kind !== "character") continue;
    const v = voiceOf(a);
    if (!v) continue;
    used.set(v, [...(used.get(v) ?? []), a]);
  }
  const names = new Set<string>([...engineVoices, ...used.keys()]);
  return [...names]
    .map((name) => ({
      name,
      inEngine: engineVoices.includes(name),
      usedBy: (used.get(name) ?? []).sort((a, b) => a.name.localeCompare(b.name, "zh")),
    }))
    .sort((a, b) => {
      // **在用的排前面** —— 引擎目录动辄几百个（Edge 全量），
      // 按字母排的话自己配过的那几个会淹没在里面
      if ((b.usedBy.length > 0 ? 1 : 0) !== (a.usedBy.length > 0 ? 1 : 0)) {
        return b.usedBy.length - a.usedBy.length;
      }
      return a.name.localeCompare(b.name, "zh");
    });
}

/** 音色筛选：全部 / 未配音色 / 某个音色。 */
export type VoiceFilter = "all" | "none" | { voice: string };

/**
 * **「谁还没配」是配音前最想知道的事**，所以它是一等的筛选项，
 * 不是「音色为空」这种要自己拼的条件。
 */
export function matchesVoice(a: AssetView, f: VoiceFilter): boolean {
  if (f === "all") return true;
  // 只有角色有音色这回事；筛音色时场景道具一律不显示，
  // 而不是「它们没有音色所以算未配」—— 后者会让「未配音色」混进一堆道具
  if (a.kind !== "character") return false;
  const v = voiceOf(a);
  return f === "none" ? v === "" : v === f.voice;
}

/** 还没配音色的角色数。配音前的那句提示用它。 */
export function unvoicedCount(assets: AssetView[]): number {
  return assets.filter((a) => a.kind === "character" && !voiceOf(a)).length;
}
