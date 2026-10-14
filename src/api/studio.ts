import { invoke } from "@tauri-apps/api/core";
import { loraNote } from "../components/loraNotes";

/** 一种生成模式。**由 Rust 侧的能力目录给出，前端不硬编码。** */
export interface ModeSpec {
  id: string;
  /** 完整名称，用在句子和悬浮说明里 */
  label: string;
  /** 按钮上的短名字（「文生」「四图」）。见 Rust 侧 `ModeSpec::short` */
  short: string;
  /** "image" | "video" */
  kind: string;
  /** 需要几张输入图（0 = 纯文生） */
  images: number;
  /** 媒体输入区的标题。「参考图」和「首帧图」是两件事，写错会让人塞错图 */
  media_label: string | null;
  /** 支持这个模式的引擎 id */
  engines: string[];
}

/** 一个引擎的能力。每一条都对应一个实测结论。 */
/** 一个可选尺寸。`verified` = 这台机器上真跑过。 */
export interface SizeOption {
  label: string;
  width: number;
  height: number;
  verified: boolean;
  /** 实测耗时（秒）。null = 没测过 —— 代价要在点下去之前看得见 */
  secs: number | null;
  /** 实测峰值内存（GB） */
  peak_gb: number | null;
}

/** 资产库里的一个条目。 */
export interface Asset {
  path: string;
  name: string;
  /** image | video */
  kind: string;
  bytes: number;
  at: number;
}

export const studioAssets = (workspacePath?: string | null) =>
  invoke<Asset[]>("studio_assets", { workspacePath: workspacePath ?? null });
export const studioDeleteAsset = (path: string, workspacePath?: string | null) =>
  invoke<void>("studio_delete_asset", { path, workspacePath: workspacePath ?? null });

/** 一条 Prompt 模板。 */
export interface PromptTemplate {
  id: string;
  title: string;
  body: string;
  /** image | video | general */
  kind: string;
}

/** 一条历史记录。 */
export interface PromptHistoryItem {
  prompt: string;
  mode: string;
  engine: string;
  /** 毫秒时间戳 */
  at: number;
}

export const studioTemplates = () => invoke<PromptTemplate[]>("studio_templates");
export const studioSaveTemplate = (tpl: PromptTemplate) =>
  invoke<PromptTemplate[]>("studio_save_template", { tpl });
export const studioDeleteTemplate = (id: string) =>
  invoke<PromptTemplate[]>("studio_delete_template", { id });
export const studioHistory = () => invoke<PromptHistoryItem[]>("studio_history");
export const studioAddHistory = (item: PromptHistoryItem) =>
  invoke<PromptHistoryItem[]>("studio_add_history", { item });

/** 磁盘上的一个 LoRA。 */
export interface LoraOption {
  /** ComfyUI 枚举出来的名字，含子目录前缀 */
  name: string;
  label: string;
  /** zimage / sdxl / ltx / h3 */
  arch: string;
  /** 哪些引擎能用它 —— 挂错架构是 0 adapters 且不报错 */
  engines: string[];
  /** 猜出来的激活词。**是猜的**，UI 必须显示并允许去掉 */
  triggers: string[];
  /** 训练时的高频标签。**不是作者写的说明**，只能当线索 */
  concepts: string[];
  /** 用户自己写的备注，优先于训练标签 */
  note: string | null;
  /** **作者标的**激活词（从 Civitai 抄下来的）。和 `triggers` 的区别是它不是猜的。 */
  confirmed_triggers: string[];
  /**
   * 成人向。**按发布页作者标的记，不是我们判的。**
   *
   * 只影响界面上标不标出来 —— 不拦截。不标的问题是：挂了个成人向 LoRA
   * 去出角色资产，出来的东西不对劲而看不出为什么。
   */
  nsfw: boolean;
  /** 风格窗口的分类（画风 / 人像 / 角色 / 服装 / 场景 / 功能 / 动作 / 人体）；空 = 没分过 */
  category: string;
  /** 出处页面（Civitai / HF） */
  source_url: string;
  /** 示例图本地路径（作者样例）；null = 显示占位 */
  thumb: string | null;
}

/** 时长的取值范围。每个引擎不一样。 */
export interface DurationSpec {
  min: number;
  max: number;
  step: number;
  default: number;
  /** 超过这个值会切成多段接续。null = 不分段 */
  window_secs: number | null;
  /** 直接显示给用户的约束说明 */
  note: string | null;
}

export interface EngineSpec {
  id: string;
  label: string;
  kind: string;
  /** 负向提示词**真的起作用**吗。false 时不要显示那个框 */
  negative_prompt: boolean;
  /** 提示词写长了会不会被摊薄 */
  long_prompt_ok: boolean;
  /** 该用多少步。null = **不要暴露这个参数** */
  steps: number | null;
  loras: boolean;
  /** 单次大约多久（秒） */
  secs: number;
  /** 直接显示给用户的实测提示 */
  notes: string[];
  /** 可选尺寸，第一个是默认 */
  sizes: SizeOption[];
  /**
   * 一次最多排几个。**不是图里的 batch_size** —— 我们的建图恒为 1，
   * 一次多张是多次提交、由客户端队列串行发出去。
   */
  max_batch: number;
  /** 参考生图引擎吃几张参考图（0 = 不是参考生图引擎）。见 Rust 侧 `EngineSpec::reference_slots` */
  reference_slots: number;
  /** 工作区（项目模型设置）里显不显示。Pony 和参考生图引擎目前是 false */
  project_settings: boolean;
  /** 时长范围。null = 出图，没有时长这个维度 */
  duration: DurationSpec | null;
}

/** 资产出图能选的画幅。由后端从实测尺寸推出来 —— 前端不另写一份。 */
export interface AspectOption {
  /** `"portrait" | "square" | "landscape"` */
  id: string;
  label: string;
  width: number;
  height: number;
}

export interface CapabilityCatalog {
  modes: ModeSpec[];
  engines: EngineSpec[];
  default_image_engine: string;
  default_video_engine: string;
  aspects: AspectOption[];
}

/**
 * 取能力目录。
 *
 * **这是 UI 唯一的引擎知识来源。** 别在前端另写一份「哪个引擎支持什么」——
 * 那会变成第二份真相，并且和 Rust 侧漂移；漂移的表现是「界面上有个不起作用
 * 的输入框」，又一个静默失败。
 */
/** 列出磁盘上的 LoRA。 */
/**
 * 这个 LoRA 是干嘛的 —— 列表里和悬浮提示都用它。
 *
 * 四级回落：**用户备注 > 内置说明 > 训练标签 > 直说没有**。
 *
 * - 用户备注最优先：他自己写的，比我们查的更清楚他要什么。
 * - **内置说明是这一级新加的**（2026-09-17）。`components/loraNotes.ts` 里那张表
 *   一直存在，但这个函数从来没查过它 —— 于是我们实测 + 联网核对出来的说明
 *   写了却不显示，界面上一排「没有说明」。**又是「机制写了、执行漏了」。**
 * - 训练标签**不是作者写的说明**，所以要标出来源，别让人以为那是官方描述。
 */
export function loraHint(l: LoraOption, zh = true): string {
  if (l.note && l.note.trim()) return l.note.trim();
  const built = loraNote(l.name, zh);
  if (built) return built.text;
  if (l.concepts.length > 0) return `训练标签：${l.concepts.join("、")}`;
  return "没有说明 —— 点右侧「备注」自己写一句";
}

/** 记一条 LoRA 备注。空字符串 = 删掉。 */
export async function setLoraNote(name: string, note: string): Promise<void> {
  await invoke("studio_set_lora_note", { name, note });
}

export async function studioLoras(): Promise<LoraOption[]> {
  return invoke<LoraOption[]>("studio_loras");
}

/**
 * 把激活词并进提示词。
 *
 * **只加缺的那些，且不改原文顺序。** 用户已经写了的不重复加；
 * 而且返回「实际加了什么」，让 UI 显示出来 ——
 * 静默改提示词是我们自己造的静默失败。
 */
export function mergeTriggers(
  prompt: string,
  triggers: string[],
): { prompt: string; added: string[] } {
  const lower = prompt.toLowerCase();
  const added = triggers.filter((t) => t.trim() && !lower.includes(t.toLowerCase()));
  if (added.length === 0) return { prompt, added: [] };
  const head = added.join(", ");
  return { prompt: prompt.trim() ? `${head}, ${prompt}` : head, added };
}

export interface GenerateArgs {
  /** H3：出片前按官方格式整理提示词（只动格式）。不传 = 不整理 */
  h3_official_format?: boolean;
  mode: string;
  engine: string;
  prompt: string;
  /** H3 加速档（`lightx2v` / `standard` / `staged` / `dense`，和工作台出片设置同一套 id）。不传 = 按尺寸自动 */
  h3_quality?: string;
  /** H3 采样方法（`euler` / `res_multistep`）。不传 = euler */
  h3_sampling?: string;
  /** 参考音频（本机路径）。只在 `r2v` + H3 时有用，最多 3 段，编号 `<Audio 1..>`（画布「音频素材」节点） */
  audios?: string[];
  negative_prompt: string;
  width: number;
  height: number;
  seed?: number;
  images: string[];
  duration?: number;
  generate_audio?: boolean;
  /**
   * `false` = **别挂设置里那两个 Z-Image 常驻 LoRA**。不传 = 照旧。
   *
   * 常驻 LoRA 的生效条件是「纯文生图 + 这次没自己传」，而**传一个空数组
   * 等于没传** —— 资产出图正好是这种情况，于是它们被静默挂上。
   * 2026-09-12 实拍：道具「键盘」那张图里三格是人像和头发（直发人像 LoRA）。
   */
  auto_loras?: boolean;
  /**
   * 场景宫格要出的方位。**空 = 用后端的兜底那套**（全景/正打/反打/入口），
   * 而那套笼统说法实测会让几格出成同一个机位。
   *
   * 一个方位就是空间契约里的一个 `SceneView`，描述要写成
   * 「两两关系 + 视线终点」才区分得开。
   */
  viewpoints?: { name: string; description: string }[];
  /** 宫格切完逐格 i2i 精修。**默认不开** —— 每格多一次出图。 */
  refine?: boolean;
  /** 图生图的重绘幅度。不给就用后端默认（0.65）。只对带源图的模式有意义。 */
  denoise?: number;
  /** 参考表的保真度旋钮（0~10）。不给就用后端实测的默认值 4。 */
  ref_boost?: number;
  /** 参考表：主体名字，会写进图上的元数据栏。 */
  entity_name?: string;
  /** 参考表：实体类型（`HUMAN` / `ANIMAL` / …），认不出的后端会回落。 */
  entity_type?: string;
  /**
   * 参考表：身份锚点，一条一个。
   *
   * **带方括号的那段模板不在前端拼** —— 唯一所有者是 Rust 的
   * `entity_sheet::sheet_prompt`。这里只送零件。
   */
  identity_locks?: string[];
  /** 参考表：风格句。不给就用模板自带的默认。 */
  style_line?: string;
  loras?: { name: string; strength: number }[];
  workspace_path?: string | null;
}

export interface GenerateOutput {
  path: string | null;
  /**
   * **所有**落盘的文件。宫格一次出 1 张联合图 + N 张切好的格子，
   * 只看 `path` 的话后面 N 张就丢了 —— 而它们才是要入库的东西。
   */
  paths?: string[];
  message: string;
  /** 跑成功了、但有话要说（引擎回落、LoRA 名字对不上、提示词被摊薄…）。 */
  notes: string[];
  ok: boolean;
}

/** 跑一次生成。走的是和对话里完全同一条路径（agent 工具本身）。 */
export async function studioGenerate(args: GenerateArgs): Promise<GenerateOutput> {
  return invoke<GenerateOutput>("studio_generate", { args });
}

/**
 * 结果画廊的存读。
 *
 * **不传 `workspacePath`** —— 和 `studio_generate` 保持一致（它也不传）。
 * 两边传得不一样的话，历史会落在一个根下、产物落在另一个根下，
 * 而 `reconcile_history` 会把「文件不在」的条目全丢掉 ——
 * 表现就是每次打开历史都是空的，且不报错。
 */
export const studioResults = () =>
  invoke<StudioResultEntry[]>("studio_results", { workspacePath: null });

export const studioSaveResults = (items: StudioResultEntry[]) =>
  invoke<void>("studio_save_results", { workspacePath: null, items });

/** 和前端 `StudioResult` 同构（Rust 侧用 serde rename 对齐了驼峰）。 */
export interface StudioResultEntry {
  id: string;
  mode: string;
  engineId: string;
  prompt: string;
  status: string;
  path?: string;
  seed?: number;
  error?: string;
  triggersAdded?: string[];
  startedAt: number;
  endedAt?: number;
}

export async function studioCapabilities(): Promise<CapabilityCatalog> {
  return invoke<CapabilityCatalog>("studio_capabilities");
}

/** 某个模式下可用的引擎。 */
export function enginesForMode(cat: CapabilityCatalog, modeId: string): EngineSpec[] {
  const mode = cat.modes.find((m) => m.id === modeId);
  if (!mode) return [];
  return mode.engines
    .map((id) => cat.engines.find((e) => e.id === id))
    .filter((e): e is EngineSpec => !!e);
}

/**
 * 换模式后该选哪个引擎。
 *
 * **保留当前引擎（如果它支持新模式），否则退到该种类的默认引擎。**
 * 无脑重置会让人每次切模式都要重选一遍；无脑保留则会留下一个不支持的引擎。
 */
export function pickEngine(
  cat: CapabilityCatalog,
  modeId: string,
  current: string | null,
): string | null {
  const usable = enginesForMode(cat, modeId);
  if (usable.length === 0) return null;
  if (current && usable.some((e) => e.id === current)) return current;
  const mode = cat.modes.find((m) => m.id === modeId);
  const fallback =
    mode?.kind === "video" ? cat.default_video_engine : cat.default_image_engine;
  return usable.some((e) => e.id === fallback) ? fallback : usable[0].id;
}

