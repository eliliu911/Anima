/**
 * 镜头语言 —— **唯一一份词表**（前端侧）。
 *
 * 读的就是 Rust 侧 `include_str!` 嵌进去的那一份字节：
 * 项目根的 `shared/shot-language.json`。三边读同一个文件
 * （Rust / 这里 / 生成 `skills/film-director/reference/*.md` 的脚本），
 * 所以不存在「改了一处、另两处还是旧的」。
 *
 * 这个模块**故意和 `src-tauri/src/shot_language.rs` 一一对应** ——
 * 函数名、语义、连「认不出的引擎按英文算」这个默认方向都一样。
 * 两边行为分叉的表现是「同一个 id 在界面上显示中文、发给模型的是英文」，
 * 而那不报错。`shotLanguage.test.ts` 里有一条守卫盯着两边读的是同一个路径。
 *
 * 设计和取舍见 `docs/PLAN_SHOT_LANGUAGE.md`。
 */
import raw from "../../shared/shot-language.json";

export interface Prompt {
  en: string;
  zh: string;
}

export interface Term {
  id: string;
  name: string;
  prompt: Prompt;
  /**
   * **静帧要不要用这条。** 运镜一律 false —— 静帧里没有「推」这回事。
   *
   * 以前这条规则散落在 Rust 侧 `frame_subject` 的注释里，现在它是数据。
   */
  still: boolean;
  /** 这一档景别建议的画幅。只有景别有。 */
  aspect?: "portrait" | "landscape" | "square";
  /**
   * **画幅不对会实际出问题**，不是不够理想。
   *
   * `aspect` 是给单张出图用的建议（那时候画幅是自己挑的）。剧集的画幅由
   * 项目定死，所以那边只在这一条为真时才提示 —— 否则 9:16 的竖屏短剧里
   * 七档景别会响三档，而竖屏剧用特写天经地义。**报太多等于没报。**
   */
  aspectHard?: boolean;
  /** 实测结论 / 注意事项。会被带进生成的 markdown。 */
  note?: string;
  /** 只对这些引擎有意义。空 / 缺省 = 都适用。 */
  engines?: string[];
  /**
   * **运镜按物理分组**：`translate`（机器真的在动）/ `rotate`（原地转）/
   * `lens`（机器不动，只有焦距或焦点变）/ `hybrid`（复合）/ `still`。
   *
   * 只有运镜有。界面按它分隔显示 —— 二十一条排成一长条选不动，
   * 而且**推镜头和变焦推近挨在一起时看不出区别**，那正是最容易选错的一对。
   */
  group?: string;
}

export interface Group {
  id: string;
  name: string;
  items: Term[];
  /** 分组的显示名（只有运镜有）。 */
  groupNames?: Record<string, string>;
}

/** 拼装用的固定段落。中英各一份 —— 按引擎选。 */
export interface Assembly {
  sections: string[];
  consistency: Prompt;
  outputRule: Prompt;
  referenceSingle: Prompt;
  referenceMulti: Prompt;
}

export interface ShotLanguage {
  groups: Group[];
  assembly: Assembly;
  chineseEngines: { ids: string[] };
}

/**
 * JSON 里还有 `$schema` / `note` / `aspectAware` 这些给人看的字段，
 * 而 TS 从 JSON 推出来的类型会把「只有部分条目才有 aspect」推成个
 * 难用的联合类型。这里一次性收敛到上面的接口 ——
 * 结构对不对由测试保证，不是由推导保证。
 */
const DATA = raw as unknown as ShotLanguage;

export function shotLanguage(): ShotLanguage {
  return DATA;
}

export function group(id: string): Group | undefined {
  return DATA.groups.find((g) => g.id === id);
}

/**
 * 这条词在这个引擎上的提示词。
 *
 * **中英按引擎选**：Z-Image 的编码器是 Qwen3-4B、Chroma 是 T5-XXL，
 * 中文原生；SDXL 的 CLIP 是 77 token 分块取平均，要英文关键词。
 * 选错了不报错，只是那一段基本不生效。
 */
export function textFor(t: Term, engine: string): string {
  return prefersChinese(engine) ? t.prompt.zh : t.prompt.en;
}

/**
 * 这个引擎的编码器吃不吃中文。
 *
 * 认不出的引擎**按英文算** —— 保守方向：英文在中文原生的编码器上
 * 也能用，反过来不成立。
 *
 * **名单在词表里**（`chineseEngines.ids`），不写在这儿 —— Rust 侧判的是
 * 同一件事，名单写两份就会分叉，而分叉的表现是「界面显示中文、
 * 发出去的是英文」。别名也在那份名单里（项目里存的可能是
 * `minimax_h3` / `ltx25` 这些老写法）。
 */
export function prefersChinese(engine: string): boolean {
  const e = engine.trim().toLowerCase();
  return DATA.chineseEngines.ids.includes(e);
}

/**
 * 组内按 id 找。
 *
 * **不用跨组的 `term`** —— 景别和运镜各存各的字段，拿运镜 id 去查景别
 * 应该查不到，而不是碰巧命中另一组里的一条。
 */
export function termIn(groupId: string, id: string): Term | undefined {
  const key = id.trim();
  return group(groupId)?.items.find((t) => t.id === key);
}

/**
 * 界面上显示什么。
 *
 * **认不出的原样显示。** 旧数据里有用户自己打的景别，
 * 显示成「大特写」是在骗人 —— 下拉框以前就是这么干的
 * （`options[0]` 兜底），选了什么和存了什么不一样、而且看不出来。
 */
export function display(groupId: string, value: string): string {
  return termIn(groupId, value)?.name ?? value.trim();
}

/** 提示词里写什么。认不出的原样发 —— 用户打的词本来就该进提示词。 */
export function promptText(groupId: string, value: string, engine: string): string {
  const t = termIn(groupId, value);
  return t ? textFor(t, engine) : value.trim();
}

/**
 * 把一组词按 `group` 分段，**保持词表里的原顺序**。
 *
 * 没有分组的组原样返回一段（`label` 为空）—— 景别、光位这些不分组。
 */
export function byGroup(g: Group | undefined): { label: string; items: Term[] }[] {
  if (!g) return [];
  const names = g.groupNames;
  if (!names) return [{ label: "", items: g.items }];
  const out: { label: string; items: Term[] }[] = [];
  for (const t of g.items) {
    const key = t.group ?? "";
    const label = names[key] ?? key;
    const last = out[out.length - 1];
    // **同名相邻才合并**，不是按 key 归堆 —— 归堆会打乱词表里的顺序，
    // 而那个顺序是有意排的（常用的在前）。
    if (last && last.label === label) last.items.push(t);
    else out.push({ label, items: [t] });
  }
  return out;
}
