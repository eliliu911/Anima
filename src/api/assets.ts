import { invoke } from "@tauri-apps/api/core";
import type { Job } from "./jobs";

/**
 * 资产库的类型化封装。
 *
 * **显示 URL 一律由 Rust 侧给。** 磁盘上存的是相对 `media/` 的路径；
 * 前端要是自己拼，就得知道 `media/` 在哪、作用域怎么映射成目录 ——
 * 那等于把存储布局复制一份到前端，改布局时两边必然走散。
 */

export type AssetKind = "character" | "scene" | "prop";

/** 资产存在哪一级。分镜引用时从近到远解析：分集 → 剧集 → 全局。 */
export type Scope =
  | { kind: "global" }
  | { kind: "project"; project_id: string }
  | { kind: "episode"; project_id: string; episode_id: string };

export interface MediaVariant {
  id: string;
  /** 相对路径。显示用同下标的 `image_urls` / `video_urls`，别拿这个当 src */
  rel_path: string;
  created_at: number;
  prompt_used: string | null;
  /** 收藏的变体不参与自动清理 */
  favorited: boolean;
  from_upload: boolean;
  /** 这张图在流水线里是什么。`null` = 没标过（存量变体都是这个状态）。 */
  role?: VariantRole | null;
}

export interface AssetUnit {
  selected_image_id: string | null;
  image_variants: MediaVariant[];
  selected_video_id: string | null;
  video_variants: MediaVariant[];
  image_prompt: string | null;
  video_prompt: string | null;
}

/** 角色专属。场景/道具为 null。 */
export interface CharacterTraits {
  persona: string;
  trigger_words: string;
  seed: number | null;
  lora_model: string | null;
  lora_weight: number | null;
  consistency_mode: string | null;
  clothing_prompt: string | null;
  voice_id: string | null;
  /** 给 H3 看的中文声音描述（音高/音色/语速/口音）。和 voice_id 不是一回事 —— 那个是 TTS 的模板名 */
  h3_voice?: string | null;
  asset_sheet: unknown | null;
  visual_dna: unknown | null;
  /**
   * 只有声音、没有能出镜的身体：旁白、电话那头的人、纯语音 AI。
   *
   * 分镜读它 —— 标了的角色台词做成画外音，不单独给它镜头。
   * 可选是因为旧数据没有这个字段，后端按 false 处理。
   */
  disembodied?: boolean;
}

/** 一个挂上的 LoRA。名字含子目录前缀（`zimage/xxx.safetensors`）。 */
export interface LoraPick {
  name: string;
  strength: number;
}

/**
 * 单个资产的出图配置。每一项都是**覆盖**，`undefined` = 用项目设置。
 *
 * `loras: []` 和 `loras: undefined` 不是一回事 ——
 * 空数组是「明确不挂」，`undefined` 是「没设过」。
 */
export interface AssetGen {
  engine?: string;
  aspect?: string;
  loras?: LoraPick[];
}

/**
 * 场景的**空间契约** —— 一张文字地图。
 *
 * 和角色的 `visual_dna` 是对称的两件事：一个锁人长什么样，
 * 一个锁地方长什么样。它派生宫格的视角描述，也提供每一条提示词
 * 都要带的「地理锁」。
 *
 * 身份和状态是分开的：时段、天气、停电、演员站位**都不进这里** ——
 * 那些变了，空间还是同一个空间。
 */
export interface SceneContract {
  /** 整体形状和尺度，一句话 */
  shape: string;
  zones: string[];
  entrances: { name: string; connects_to: string }[];
  anchors: {
    name: string;
    /** 拓扑名，不是坐标 */
    zone: string;
    size: "small" | "medium" | "large" | "wall_dominant";
    /** 负向锁：它最容易被画错成什么 */
    must_not_become: string[];
  }[];
  /** **两两关系**，不是清单。「门正对柜台」是关系，「有门、有柜台」不是 */
  relations: string[];
  materials: string[];
  views: {
    name: string;
    from: string;
    toward: string;
    /** 视线尽头。这一项最能把两个机位区分开 */
    far: string;
    visible: string[];
    light: string;
  }[];
  /** **编的**。剧本没写、推断补上的部分，和锁死项分开存 */
  inferred: string[];
  no_extra: string[];
  empty_stage: "required" | "preferred" | "not_required";
  /**
   * 这个场景的全景**世界校准**：相机离地多高、网格地面在哪、一身位多大。
   *
   * 全景是生成的 —— 它的相机离地多高、房间多大**我们无从知道**，
   * 只能让人拖到地面网格贴着地板为止。对一次要花点功夫，
   * 存在画布节点里的话换个节点、换一天又得重来。
   *
   * 严格说它是**每张全景图**的，不是每个场景的（同一场景出两张全景，
   * 相机高度未必一样）。按场景存是个近似，取舍是「一个场景通常只有
   * 一张在用的全景」。
   */
  pano_calibration?: { eye_height: number; ground_height?: number; scale: number } | null;
  /**
   * 这个地点的 **3D 片场**（3DGS）。一个地点生成一次，所有在这儿的镜头共用（分镜详情里生成）。
   * 只由后端生成命令写；整份保存契约时不带它也不会被冲掉。
   */
  world?: SceneWorld | null;
}

/** 一个地点的 3D 片场文件。和 Rust `scene_contract::SceneWorld` 同形 */
export interface SceneWorld {
  path: string;
  format: "ply" | "ksplat" | "spz" | string;
  /** `opencv_camera` = SHARP 的原相机坐标，载入时要翻正 */
  convention: string;
  source_image: string;
  created_at: number;
  /** 片场缩放（SHARP 的单位只是大约米）。没有 = 1 */
  scale?: number | null;
}

/** 主形象的 id。主形象就是资产自己的描述 + 图片池 */
export const MAIN_LOOK = "main";

/**
 * 角色的一个**形象**（同一个人的不同发型、服装、造型）。主形象在 `look_views` 第一个。
 * 场景 / 道具只有主形象这一个。见 `docs/plans/2026-09-15-角色形象方案.md`。
 */
export interface LookView {
  id: string;
  /** 显示名；主形象没起名时是「主形象」 */
  name: string;
  is_main: boolean;
  /** 这个形象自己的描述（主形象就是资产的 description） */
  description: string;
  /** 发给模型的完整描述：人物本身 + 形象描述 */
  prompt: string;
  nsfw: boolean;
  unit: AssetUnit;
  cover_url: string | null;
  image_urls: string[];
  image_paths: string[];
}

export interface AssetView {
  id: string;
  kind: AssetKind;
  name: string;
  description: string;
  starred: boolean;
  locked: boolean;
  /** 用户说了「只留本集」（只对分集级资产有意义）。缺省 = false */
  episode_only?: boolean;
  created_at: number;
  updated_at: number;
  unit: AssetUnit;
  character: CharacterTraits | null;
  /** 场景的空间契约。只有场景资产有，而且要人写过才有 */
  scene?: SceneContract | null;

  scope: Scope;
  /** 「全局」/ 项目名 / 「项目名 · 第 1 集」 */
  scope_label: string;
  /** 属于哪个项目（项目名）；全局资产是 null。「按项目」按它分组 */
  project_title?: string | null;
  /** 当前封面。null = 还没有图，界面显示占位而不是碎图 */
  cover_url: string | null;
  image_urls: string[];
  /**
   * 每个图变体的**本机绝对路径**，顺序同 `image_urls`。
   *
   * 生成链路（`studioGenerate` 的 `images`）要的是这个，不是 URL ——
   * 传 URL 进去的表现是「找不到源图」，而看不出是哪一层错的。
   */
  image_paths: string[];
  video_urls: string[];
  /** 有多少个镜头在用它 */
  reference_count: number;
  /** 出图配置的覆盖。空 = 跟项目设置走。 */
  gen?: AssetGen | null;
  /** 人物本身：所有形象共用（脸、身材、年龄）。空 = 老数据，description 是整段完整描述 */
  identity?: string;
  /** 主形象的名字（空 = 主形象） */
  main_look_name?: string;
  /** 主形象是否 NSFW */
  nsfw?: boolean;
  /** 形象，主形象在第一个 */
  look_views: LookView[];
}

export interface AssetReference {
  project_id: string;
  project_title: string;
  episode_id: string;
  episode_title: string;
  shot_id: string;
  shot_num: number;
}

/** 一份待入库的媒体。data URL 只在这一层出现，落盘后存的是路径。 */
export type MediaInput =
  | { kind: "path"; path: string }
  | { kind: "data_url"; data_url: string };

export const assetsList = (scope?: Scope) =>
  invoke<AssetView[]>("assets_list", { scope: scope ?? null });

export const assetsCreate = (req: {
  scope: Scope;
  kind: AssetKind;
  name: string;
  description?: string;
  media?: MediaInput | null;
}) => invoke<AssetView>("assets_create", { req });

export const assetsUpdate = (req: {
  scope: Scope;
  id: string;
  name?: string;
  description?: string;
  starred?: boolean;
  locked?: boolean;
  /** `{}` = 清空覆盖回到项目设置；不传 = 这次不动它。 */
  gen?: AssetGen;
  /** 空契约会被存成 null —— 留个空壳会让界面显示「已有空间契约」而它什么都锁不住。 */
  scene?: SceneContract;
  /** **整份替换**角色设定，不是逐字段合并 —— 传之前先把现有的那份读出来改。 */
  character?: CharacterTraits;
  /** 人物本身（所有形象共用） */
  identity?: string;
}) => invoke<AssetView>("assets_update", { req });

/** 加一个形象。新形象没有图，候选图用 `assetsAddVariant` 带 `look_id` 加 */
export const assetsAddLook = (scope: Scope, id: string, name: string, description = "") =>
  invoke<AssetView>("assets_add_look", { scope, id, name, description });

/** 改形象的名字、描述、NSFW。`lookId` 为 `main` 时改的是资产自己的字段 */
export const assetsUpdateLook = (req: { scope: Scope; id: string; look_id: string; name?: string; description?: string; nsfw?: boolean }) =>
  invoke<AssetView>("assets_update_look", { req });

export interface MergeReq {
  from_scope: Scope;
  from_id: string;
  to_scope: Scope;
  to_id: string;
  /** 合并后角色叫什么。空 = 不改名 */
  new_name: string;
  /** 目标角色原来那套（主形象）叫什么。空 = 不改 */
  main_look_name: string;
  /** 源角色变成的形象叫什么 */
  look_name: string;
}

export interface MergePlan {
  from_name: string;
  to_name: string;
  new_name: string;
  main_look_name: string;
  look_name: string;
  images: number;
  shots: { project_title: string; episode_title: string; shot_num: number; changes: string[] }[];
  /** 同一镜两个造型都出场。非空时不能合并 */
  conflicts: string[];
  note: string;
}

/** 从两个名字猜合并后的名字：回 [新名字, 主形象名, 新形象名] */
export const assetsMergeSuggest = (fromName: string, toName: string) =>
  invoke<[string, string, string]>("assets_merge_suggest", { fromName, toName });

/** 预览合并：会改哪些镜头、有没有同框冲突。不写盘 */
export const assetsMergePlan = (req: MergeReq) => invoke<MergePlan>("assets_merge_plan", { req });

/** 执行合并：源角色变成目标角色的一个形象，镜头、台词说话人一起改，源角色删掉。改之前自动备份 */
export const assetsMergeApply = (req: MergeReq) => invoke<void>("assets_merge_apply", { req });

/** 删形象，连它的候选图文件一起删。主形象不能删。用它的镜头落回项目默认 / 主形象 */
export const assetsRemoveLook = (scope: Scope, id: string, lookId: string) =>
  invoke<AssetView>("assets_remove_look", { scope, id, lookId });

export const assetsReferences = (assetId: string) =>
  invoke<AssetReference[]>("assets_references", { assetId });

/**
 * 删资产。`force` 之前必须先让用户看过引用列表 ——
 * 不查引用就删，分镜会静默失去参考图，表现是「生成的人突然不像了」。
 */
export const assetsDelete = (scope: Scope, id: string, force = false) =>
  invoke<AssetReference[]>("assets_delete", { scope, id, force });

/**
 * 一个变体在流水线里的用途。**和 Rust 侧 `VariantRole` 一一对应。**
 *
 * 以前变体只有「有没有图」，没有「是哪种图」—— 角色的定妆照、三视图、
 * 场景的主镜和反打全挤在一个池子里，谁也说不清哪张能拿去干什么。
 */
export type VariantRole =
  | "portrait" | "turnaround"
  | "scene_master" | "scene_reverse" | "scene_pano"
  | "reference";

export const assetsAddVariant = (req: {
  scope: Scope;
  id: string;
  media: MediaInput;
  prompt_used?: string | null;
  /** 这张图在流水线里是什么。空 = 通用参考图。**错配会被后端拒绝。** */
  role?: VariantRole | null;
  /** 加到哪个形象。空 / `main` = 主形象 */
  look_id?: string | null;
}) => invoke<AssetView>("assets_add_variant", { req });

export const assetsSelectVariant = (
  scope: Scope,
  id: string,
  variantId: string,
  isVideo = false,
) => invoke<AssetView>("assets_select_variant", { scope, id, variantId, isVideo });

export const assetsFavoriteVariant = (
  scope: Scope,
  id: string,
  variantId: string,
  on: boolean,
) => invoke<AssetView>("assets_favorite_variant", { scope, id, variantId, on });

/**
 * 删掉一张候选图（或候选片）。**连文件一起删，不可逆。**
 *
 * 自动清理只在超过 12 张时从最旧的开始删 —— 于是「出了十张废图」
 * 这种常态下一张都去不掉。
 *
 * **删到封面头上时后端会自动改选最新的那张** —— 封面就是 `@` 引用
 * 真正发出去的那张，指向一个不存在的变体的话，分镜里那个引用出图时
 * 只剩名字、长相带不上，而且不报错。
 */
export const assetsRemoveVariant = (scope: Scope, id: string, variantId: string) =>
  invoke<AssetView>("assets_remove_variant", { scope, id, variantId });

/** 录入全局库：本集调好的资产 → 全局共享池。源资产原样保留。 */
export const assetsPromote = (from: Scope, id: string) =>
  invoke<AssetView>("assets_promote", { from, id });

/** 两个 Scope 是不是同一个。作用域是 tagged union，不能用 === 比。 */
export function sameScope(a: Scope, b: Scope): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === "global") return true;
  if (a.kind === "project" && b.kind === "project")
    return a.project_id === b.project_id;
  if (a.kind === "episode" && b.kind === "episode")
    return a.project_id === b.project_id && a.episode_id === b.episode_id;
  return false;
}

export const KIND_LABEL: Record<AssetKind, string> = {
  character: "角色",
  scene: "场景",
  prop: "道具",
};

/** 一条 `@` 引用的解析结果。`asset_id` 为 null = 提示词里写了但库里没有。 */
export interface ResolvedRef {
  kind: AssetKind;
  name: string;
  asset_id: string | null;
  description: string;
  image_path: string | null;
  image_url: string | null;
}

export const assetsResolveRefs = (prompt: string) =>
  invoke<ResolvedRef[]>("assets_resolve_refs", { prompt });

/** 插入一个内联引用标签。**必须和 Rust 侧 `parse_asset_refs` 认的格式一致。** */
export function refTag(kind: AssetKind, name: string): string {
  return `[${KIND_LABEL[kind]}:${name}]`;
}

/**
 * 批量出资产设定图（后台任务，见 Rust `asset_batch`）。切页、刷新都不断；每出一张发 `studio://changed`。
 *
 * - `redo`：有图的也重出一遍（加一版，旧的留在变体里）
 * - `only`：只出点了名的（资产名 / id、形象名 / id）
 */
export const assetsBatchGenerateStart = (projectId: string, episodeId: string | null, redo: boolean, only?: string[]) =>
  invoke<Job>("assets_batch_generate_start", { projectId, episodeId, redo, only: only ?? null });
