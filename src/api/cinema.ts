import { invoke } from "@tauri-apps/api/core";
import type { Job } from "./jobs";

/** 项目级的生成设置。和后端 `cinema::ModelSettings` 一一对应。 */
export interface ModelSettings {
  /** 资产出图（文生图） */
  t2i_engine: string;
  /** 分镜首帧（图生图） */
  i2i_engine: string;
  /** 出片 */
  video_engine: string;
  /** 画幅 id，取自 `CapabilityCatalog.aspects` */
  character_aspect: string;
  scene_aspect: string;
  prop_aspect: string;
  /** 提示词线路。合成和出片都用它。老项目没有这个字段时后端按 B */
  prompt_route?: PromptRoute;
  /** 出片设置。老项目没有时后端给默认（双采 · 480p） */
  render?: RenderSettings;
}

/** 出片画质 = H3 采样档。见 Rust `cinema::RenderQuality` */
export type RenderQuality = "fast" | "standard" | "staged" | "dense" | "lightx2v";

/** 出片设置，整个项目统一。见 Rust `cinema::RenderSettings` */
export interface RenderSettings {
  /** **参考出片**的画质（字段名沿用老的 `quality`，老项目存的就是它） */
  quality: RenderQuality;
  /** **首帧出片**的画质（2026-10-07 起分开选）。老项目没有这一项 = LightX2V */
  keyframe_quality?: RenderQuality;
  resolution: "480p" | "576p" | "720p";
  /** 画面比例，如 "16:9"；空 = 跟项目 */
  aspect: string;
  /** 每镜出几次 */
  takes: number;
  generate_audio: boolean;
  burn_subtitles: boolean;
}

/** 提示词线路：A 中文结构（纯填空），B H3 官方六段（多一次 AI 整理）。两条物理隔离。 */
export type PromptRoute = "a" | "b";

/** 一镜里的一个时间段。各段是同一空间里的连续变化，不是切镜。见 Rust `shot_table::ShotSegment` */
export interface ShotSegment {
  start: number;
  end: number;
  title?: string;
  /** 画面 */
  visual?: string;
  /** 禁止 */
  forbid?: string;
  /** 约束 */
  constraints?: string;
  /** 站位与朝向 */
  blocking?: string;
  /** 运镜：只一个主运镜 */
  camera?: string;
  /** 声音 */
  sound?: string;
}

/** 画面里要交代的场景和道具，带尺寸和摆放 */
export interface SceneItem {
  name: string;
  detail?: string;
}

/**
 * 这一场的固定布景：整场所有镜头共用（镜头表每场写一次）。站位、朝向和光的方向都从它推。
 * 见 Rust `shot_table::SceneStage`。
 */
export interface SceneStage {
  /** 拿哪个方向当「正前方」 */
  facing?: string;
  /** 搬不走的东西在场里的位置 */
  anchors?: string[];
  /** 每一个光源在哪、朝哪照、冷暖（整场固定） */
  lights?: string[];
  /** 主要是谁和谁的关系、机位在哪一侧 */
  axis?: string;
  /** 每个人一开始在哪、朝哪 */
  people?: string[];
  /** 时段和天气 */
  time?: string;
}

/** 参考图用途。取值照 drama-skills 分镜 SKILL */
export type RefUsage =
  | "identity" | "outfit" | "geography" | "composition" | "scale"
  | "effect" | "start_frame" | "end_frame" | "style";

/** 参考图槽位。`order` 就是提示词里的标签编号（参考图 N / `<Picture N>`） */
export interface RefSlot {
  order: number;
  asset_id: string;
  usage: RefUsage;
  /** 控制什么，必须正面写 */
  control: string;
  not_control?: string;
}

/** 某条线路给一镜合成出来的两条提示词 */
/** 一镜出过的一次。写明是哪条线路出的，A / B 对比看片靠它。 */
export interface VideoTake {
  route: PromptRoute;
  /** 视频文件绝对路径 */
  path: string;
  created_at: number;
  seed: number;
  /** 挂了几张参考图，0 = 文生。**老数据只有这个数** */
  pictures: number;
  /**
   * 挂的是哪几张，按顺序。
   *
   * 光有张数答不出「它用了哪张场景图」—— 而那是回看历史 take 时
   * 唯一想知道的事。顺序有意义：`<Picture N>` 的编号就是这个顺序。
   * 老数据是空的。
   */
  picture_paths?: string[];
  /** 出片档位，如「双采 · 896×512」；老数据没有 */
  quality?: string;
  /**
   * 分镜页出的，还是画布回填的。见 Rust `cinema::TakeOrigin`。
   *
   * **回填的不算「这条线路出过片」** —— 算进去的话批量出片会跳过这一镜，
   * 而且会把真正该报的过期警告盖掉。老数据没这个字段，当 `render`。
   */
  origin?: "render" | "canvas";
}

export interface RoutePrompts {
  /** 分镜提示词：出首帧时原样发送 */
  frame: string;
  /** 视频运动提示词：出片时原样发送 */
  video: string;
  /** 合成时间（秒），0 = 没合成过 */
  synthesized_at: number;
  /** 合成时镜头字段的指纹。字段改过后对不上，校验会报「要重新合成」 */
  fields_hash?: string;
}

export interface CinemaProject {
  id: string;
  title: string;
  genre: string;
  aspect_ratio: string; // renderOptions.ts 的 ASPECTS 里的一种："16:9" / "9:16" / "21:9" …
  description: string;
  created_at: number;
  updated_at: number;
  episode_count: number;
  cover_image?: string | null;
  /** "series"（多集共用角色与风格）| "standalone"（独立项目，单集） */
  kind: "series" | "standalone";
  /** `"scripted"`（我有剧本）/ `"freeform"`（我直接创作）。后者没有第 1 步。 */
  content_mode?: string;
  /** 新建镜头的默认模式：`"first_frame"`（画面优先）/ `"reference"`（节奏优先）。 */
  default_shot_mode?: string;
  /** 项目里每个角色的默认形象：资产 id → 形象 id。没写的用主形象 */
  default_looks?: Record<string, string>;
  /** 绑定的剧本编辑器项目 id。没有 = 还没绑（按名字认出来一次就会自动绑上） */
  script_project_id?: string | null;
  /** 出图/出片用哪个引擎、各类资产什么画幅。 */
  model_settings?: ModelSettings;
}

/**
 * 工作区里的一条。**统计值由后端每次算出来，不是存的** ——
 * 存一个 status 字段就一定会和事实漂移。
 */
export interface WorkspaceItem extends CinemaProject {
  shot_count: number;
  /** 已经出过片的镜头数 */
  rendered_count: number;
  has_final: boolean;
  status: "draft" | "active" | "done";
}

export const cinemaWorkspace = () => invoke<WorkspaceItem[]>("cinema_workspace");

/** 一句台词。 */
export interface DialogueLine {
  /**
   * 说话人。**必须和 `prompt` 里 `[角色:名字]` 用的名字逐字一致** ——
   * 出片模型靠这个名字把声音绑到画面里那个人身上。对不上的话双人对话会塌成
   * 一个人在说（实测过，而且不报错），所以后端会退回不标记说话人。
   */
  speaker: string;
  line: string;
  /** 音高/音色/语速/口音，和画面描写同一种语言。 */
  voice?: string;
  emotion?: string;
  /** 画外音。为真时会写明嘴唇不动 —— 否则画面里的人会跟着旁白动嘴。 */
  off_screen?: boolean;
}

/** 一个人在画面里站哪。**屏幕空间**，不是世界坐标。 */
export interface StageSubject {
  /** 角色名（同 `[角色:名字]`） */
  actor: string;
  /** 左 / 中 / 右 */
  zone?: string;
  /** 前景 / 中景 / 后景 */
  depth?: string;
  facing?: string;
  posture?: string;
}

export interface StoryboardShot {
  id: string;
  shot_num: number;
  /** 这一镜是按「一场一镜出」生成的（镜数时长由代码给、镜内可以切镜）。见 `EpisodeData.one_take` */
  one_take?: boolean;
  /** 景别的**词表 id**（`medium` / `close_up` / …）。同 `camera_movement`。 */
  scale: string;
  /**
   * 运镜的**词表 id**（`push_in` / `static` / …），不是显示串。
   * 取值和显示名见 `shared/shot-language.json`，走 `src/shared/shotLanguage.ts`。
   *
   * 旧数据里可能是「推镜头 (Zoom In)」这种显示串，也可能是用户自己打的词 ——
   * Rust 读盘时会迁移前者、**原样保留后者**，所以这里不能假定它一定查得到。
   */
  camera_movement: string;
  /**
   * 机位的**词表 id**（`eye_level` / `low_angle` / `over_shoulder` / …）。
   *
   * **和运镜是两件事。** 旧词表把「俯视」塞在运镜里，而那是机位 ——
   * 摄影机原地朝下拍 ≠ 镜头从上往下摇过去。LumenX 的分镜卡上
   * 景别/机位/运镜是三栏并列的，我们以前把它们挤进了一个枚举。
   *
   * 空串 = 没指定。**不要兜底成平视** —— 那句话会让模型去调人物高度。
   */
  camera_angle: string;
  /**
   * 这一镜自己的运镜指令，一句话。词表里那句是通用的（`push_in` 永远是
   * 「镜头缓慢推近，逐步收紧到主体」），这一句才说清**怎么动、动到哪、
   * 为了什么**：「机位平缓移动，将画面焦点从怒骂的大臣转移到被刀刃
   * 逼近的主角脸上」。它会原样进出片提示词。
   *
   * 空着就只有词表通用句 —— 旧数据全是空的，不会更差。
   */
  camera_note: string;
  /** 导演世界绑定的结构化机位。几何数据不会直接进入模型提示词。 */
  director_camera?: {
    name: string;
    camera: { position: number[]; yawDeg: number; pitchDeg: number; fovDeg: number };
    path?: { id: string; name: string; durationSec: number; cameras: Array<{ position: number[]; yawDeg: number; pitchDeg: number; fovDeg: number }> } | null;
    /** 从几何轨迹编译出的自然语言运镜约束。 */
    motion_note: string;
    rgb_video?: string | null;
    depth_video?: string | null;
    first_frame?: string | null;
    last_frame?: string | null;
  };
  /**
   * 工作区里打开导演世界时这一镜的场面（人偶、相机、运镜路径）和最近一次输出（人偶稿 / 运镜首尾帧）。
   * 只由 `cinemaSetDirectorScene` 写，整集保存时以磁盘为准。
   */
  director_scene?: DirectorSceneState | null;
  dialogue: string;
  /** @deprecated 单数，表达不了双人镜头。读旧数据用，新代码写 `character_ids` */
  character_id: string;
  /** 这一镜出场的角色（资产 id） */
  character_ids: string[];
  scene_id?: string | null;
  prop_ids: string[];
  /**
   * 这一镜走哪条链路。**逐镜，不是项目级** ——
   * 一集里两种镜头必然混着用。
   *
   * - `first_frame`：提示词 → 出首帧 → 首帧 + 提示词 → 视频。
   *   需要精确控构图/服装/道具位置的镜头走这条。
   * - `reference`：提示词 + @ 引用的资产参考图 → 直接出视频。
   *   人物要动起来的对话和动作镜头走这条，不需要先出首帧。
   */
  mode: "first_frame" | "reference";
  /**
   * 接着上一镜继续：关键帧**直接用上一镜视频的末帧**。
   *
   * 只在「同一机位、同一时刻的直接延续」时成立 —— 机位变了、景别变了、
   * 时间跳了，这么做就是错的：新镜头会从一张属于旧机位的画面开始。
   * 生成分镜时由模型判断，拿不准就关；第一镜永远关。
   */
  continues_previous?: boolean;
  /**
   * **这一镜怎么接上一镜。** 三档，不是一个勾选框 ——
   * 见 `storyboard/continuity.ts` 和 Rust 的 `cinema::Continuity`。
   *
   * ⚠️ **不要直接读** —— 用 `effectiveContinuity(shot)`，
   * 老数据的 `continues_previous` 要在那里折进来。
   */
  continuity?: "none" | "ref_video" | "first_frame";
  /** 「接上一镜」是手动选的：自动设（Rust `shot_route::apply_auto_continuity`）不覆盖它 */
  continuity_manual?: boolean;
  /**
   * 出片方式手动指定（2026-10-11）。不填 = 跟着内容建议；`keyframe` / `reference` = 在镜头详情里选的，
   * 建议、按场出首帧、接上一镜都照它（Rust `shot_route::content_route`）。实际走哪条仍看有没有首帧
   */
  route_choice?: ShotRoute;
  prompt: string;
  /** 第几场（从 1 开始），生成镜头表时由分场结果写入 */
  scene_no?: number;
  /** 场名：「地点 · 时间」 */
  scene_name?: string;
  /** 这一镜为什么存在（唯一职责） */
  duty?: string;
  /** 剧本来源：这一镜负责拍剧本里的哪几句 */
  source?: string;
  /** 光影氛围：光源、方向、光比、色温，一个主光 */
  lighting?: string;
  scene_items?: SceneItem[];
  /** 这一场的固定布景（整场共用）。老镜头没有 */
  stage?: SceneStage;
  /** 按时间段写的画面描述 */
  segments?: ShotSegment[];
  /** 这一镜里换用的形象：资产 id → 形象 id。没写的用项目默认形象 */
  look_overrides?: Record<string, string>;
  ref_slots?: RefSlot[];
  /** 每条线路各自合成的提示词。出片 / 出首帧原样发送当前线路那一份 */
  prompts?: Partial<Record<PromptRoute, RoutePrompts>>;
  /** 这一镜出过的每一次（场记板上的「次」），新的在后。见 Rust `cinema::VideoTake` */
  takes?: VideoTake[];
  /** 镜头表里写了、资产库里没有的资产。合成提示词时按文字兜底，见 Rust `shot_table::UnlinkedAsset` */
  unlinked_assets?: { kind: "character" | "scene" | "prop"; name: string }[];
  /**
   * 这一镜**动起来**的部分：谁在动、怎么动、事情怎么发展。
   *
   * 和 `prompt` 分工：`prompt` 是静态的那一帧长什么样（喂首帧），
   * 这个是沿时间轴的变化（只进出片）。以前两边共用 `prompt`，
   * 结果首帧读到一堆动作、出片读到一堆静态描写。
   *
   * 旧数据没有这个字段 —— 空着就退回老行为。
   */
  action?: string;
  /**
   * 这一镜**结束时**画面该是什么样。空 = 不约束结尾。
   *
   * 两种用法成本差很远：
   * - **文字约束**（总是生效）：编进出片提示词，零成本。
   * - **真尾帧图**（只在 `<shot>_end.png` 确实存在时）：走首尾帧，是硬约束。
   *   **不自动生成** —— 每镜多一张图，而且 H3 的首尾帧要 54 GB 内存峰值。
   *
   * 和 `continues_previous` 是一对：这一镜落在哪，决定下一镜从哪开始接得顺不顺。
   */
  end_frame?: string;
  /**
   * 结构化台词。**非空时压过 `dialogue` 那个字符串。**
   *
   * `dialogue` 是人写人读的那份（剧本原文，界面显示的也是它），
   * 这份是给模型用的，多了说话人绑定、音色和画外音标记。
   */
  dialogue_lines?: DialogueLine[];
  /** 情绪**轨迹**，不是状态。两个标签 = 一次转折，编译成 `mood: A → B`。 */
  emotion_tags?: string[];
  /** 站位。**屏幕空间**（左/中/右 × 前/中/后），不是世界坐标。 */
  blocking?: StageSubject[];
  /** 环境声。和 `sound_effect` 一起进 H3 的 `overall_soundscape`。 */
  ambient_sound?: string;
  /** 具体的一响（关门、碎裂、脚步）。 */
  sound_effect?: string;
  /**
   * 配乐。**留空 = 不要配乐**，而且这是绝大多数镜头该有的值。
   *
   * H3 的音轨和画面一起无条件生成，这个字段不发它就自己编一段 ——
   * 「无端配背景音乐」就是这么来的。
   */
  score?: string;
  negative_prompt?: string;
  /** 首帧文件当前这一版（mtime）。读盘时重算，给媒体 URL 做缓存击穿。 */
  image_rev?: number;
  /** 出片文件当前这一版。 */
  video_rev?: number;
  duration: number;
  /**
   * 宫格草图。**每次读盘重算** —— 它只是 `shots/<id>_sketch.png` 在不在的投影。
   *
   * 和 `image_url` 是两回事：草图是**布局稿**（本机 3×3 每格 512×341，
   * 能定构图和光向，定不了材质和脸），关键帧才是成品。
   * 界面上必须标出这个区别，否则草图会被当成品用下去。
   */
  sketch_url?: string | null;
  /**
   * 挂了哪几张关键帧。**出片模式按它自动定**（后端 `Keyframes`）：
   * 有首帧 → 首帧图生视频；首帧 + 尾帧 → 首尾帧；都没有 → 参考生视频。
   * 真相在磁盘上，读盘 / 保存时后端重算；没挂时后端不发这个字段。
   */
  keyframes?: Keyframes;
  /** 尾帧图（`shots/<id>_end.png`），读盘时重算。 */
  end_image_url?: string | null;
  image_url?: string | null;
  video_url?: string | null;
  audio_url?: string | null;
  status: "pending" | "generating" | "ready" | "error";
}

/** 人偶稿里一个人偶的信息（导演世界 `frame_meta.actors` 原样）。 */
export interface DraftActor {
  asset_id: string | null;
  label: string;
  color_name: string | null;
  marker_color: string;
  pose_label?: string | null;
  facing?: string | null;
  screen?: { x: number; y: number } | null;
  in_frame?: boolean;
  /** person / prop / extra（导演台界面升级 U3.5）。缺省 = person */
  role?: "person" | "prop" | "extra" | null;
  /** 道具立牌的那张图（后端查不到道具资产的图时的退路） */
  image_path?: string | null;
}

/** `StoryboardShot.director_scene` 的形状（后端当 JSON 存，前端约定这几个键） */
export interface DirectorSceneState {
  /** 编辑器的场面快照：重新打开时恢复 */
  scene?: unknown;
  /** 最近一次「输出」：人偶稿 + 纯背景 + 每个人偶是谁 */
  outputs?: { combined: string; env_only: string; actors: DraftActor[]; at: number } | null;
  /** 最近一次运镜输出：首尾帧人偶稿 */
  trajectory?: { rgb_video: string; first: string; last: string; at: number } | null;
}

/** 一镜挂的关键帧。和 Rust `cinema::Keyframes` 同形 */
export interface Keyframes {
  first: boolean;
  last: boolean;
}

/** 一场单独的「一镜出」设置。和 Rust `one_take::SceneTake` 同形 */
export interface SceneTake {
  scene_no: number;
  /** true = 这一场一镜出；false = 照常分镜 */
  one_take: boolean;
  /** 手填的总时长（秒）。空 = 自动：剧本写明的 > 整集只有一场时用分集时长 > 按台词估 */
  secs?: number | null;
}

/** 一场整段出的一次成片（长镜头）。和 Rust `cinema::SceneRender` 同形 */
export interface SceneRender {
  scene_no: number;
  route: PromptRoute;
  /** 整条视频的绝对路径 */
  path: string;
  created_at: number;
  seed: number;
  quality?: string;
  /** 按窗口顺序的镜头 id */
  shot_ids?: string[];
  prompts?: string[];
  fields_hashes?: string[];
  picture_paths?: string[][];
}

export interface EpisodeData {
  /**
   * **哪几场整段出**（长镜头），按场号。
   *
   * 按场号存不存在镜头上 —— 一场里的镜头必须整体一致，
   * 放镜头上就得维护「同一场的标记要一样」这条不变式，而它一定会被破坏。
   */
  long_take_scenes?: number[];
  /**
   * 长镜头窗口之间重叠多少帧。空 = 推荐值 22。
   * 只有 5 / 22 / 39 / 56 四档（H3 的 17n+5 潜空间栅格）。
   */
  long_take_context_frames?: number;
  /**
   * **整段出的场出过的片**（长镜头），新的在后。整条记在场上、不切开 —— H3 本来就是分窗口生成、拼好了交出来的。
   * 只由后端写，保存分集时后端沿用盘上的（界面这份不算数）
   */
  scene_renders?: SceneRender[];
  /**
   * **这一集默认不分镜**（每场一镜出）。镜数和每镜时长由代码定、镜内按时间轴切镜 ——
   * 见 Rust `one_take.rs`。给短片用：15 秒一场被拆出 2 秒的碎镜，H3 出不了。
   */
  one_take?: boolean;
  /** 单场的设置，优先于 `one_take`：某一场单独一镜出 / 改回分镜、手填总时长（空 = 自动估） */
  scene_takes?: SceneTake[];
  id: string;
  episode_num: number;
  title: string;
  synopsis: string;
  /** 剧本的**纯文本投影**。下游全部读它 */
  script_text: string;
  /** 剧本编辑器的结构化文档（ProseMirror JSON 字符串）。可为空 */
  script_doc?: string | null;
  storyboard: StoryboardShot[];
  /**
   * 这一集锁定的画风。空 = 跟项目走。
   *
   * 风格是项目级可变状态 —— 改了之后重合成旧分集会按新风格出，
   * **复现不了它自己的历史风格**。想锁住某一集时才填。
   */
  style_lock?: string;
  /** 分镜结构版本。和后端 `STORYBOARD_VERSION` 不一致的分镜读盘时会被清空 */
  storyboard_version?: number;
  final_video_url?: string | null;
  /**
   * 本集颜色表：每个出场角色一个唯一颜色。**颜色 = 身份** ——
   * 导演世界的人偶颜色、出图时认人都查它。只增不改，见 `src-tauri/src/actor_colors.rs`。
   */
  actor_colors?: ActorColor[];
}

export interface ActorColor {
  asset_id: string;
  name: string;
  /** `#RRGGBB` */
  color: string;
  /** 英文色名，写进出图提示词用 */
  color_name: string;
}

export interface ActorColorAssignment {
  colors: ActorColor[];
  /** 调色盘不够用、没分到颜色的角色名。**非空要显示出来** */
  unassigned: string[];
}

export interface CinemaProjectDetail {
  project: CinemaProject;
  episodes: EpisodeData[];
}

export interface CreateProjectReq {
  title: string;
  genre?: string;
  aspect_ratio?: string;
  description?: string;
  /** 不给按系列 */
  kind?: "series" | "standalone";
  /** `"scripted"` / `"freeform"`。后者的项目没有第 1 步。 */
  content_mode?: string;
  /** `"first_frame"` / `"reference"`。新建镜头的默认模式。 */
  default_shot_mode?: string;
}

export interface CreateEpisodeReq {
  project_id: string;
  title?: string;
  synopsis?: string;
  script_text?: string;
}

export async function cinemaListProjects(): Promise<CinemaProject[]> {
  return invoke<CinemaProject[]>("cinema_list_projects");
}

export async function cinemaCreateProject(req: CreateProjectReq): Promise<CinemaProject> {
  return invoke<CinemaProject>("cinema_create_project", { req });
}

export async function cinemaGetProjectDetail(projectId: string): Promise<CinemaProjectDetail> {
  return invoke<CinemaProjectDetail>("cinema_get_project_detail", { projectId });
}

export async function cinemaCreateEpisode(req: CreateEpisodeReq): Promise<EpisodeData> {
  return invoke<EpisodeData>("cinema_create_episode", { req });
}

/** 镜头表的一条校验问题。见 Rust `shot_table::ShotIssue` */
export interface ShotIssue {
  shot_num: number;
  /** duration / segments / dialogue / assets / ref_slots */
  field: string;
  message: string;
}

/**
 * 由剧本生成镜头表。**后端任务，立刻返回登记的那条任务**；进度、结果、备注都在任务上，
 * 通过 `JOB_EVENT` 推送。会覆盖这一集现有的镜头表。同一集同时只能跑一个。
 */
export const cinemaGenerateShotTable = (projectId: string, episodeId: string) =>
  invoke<Job>("cinema_generate_shot_table", { projectId, episodeId });

/**
 * 按项目当前线路合成提示词。**后台任务**，立即返回登记的任务；进度、结果走任务事件。
 * `shotIds` 为空 = 整集。
 */
export const cinemaSynthesizePrompts = (projectId: string, episodeId: string, shotIds: string[] = [], route?: PromptRoute) =>
  invoke<Job>("cinema_synthesize_prompts", { projectId, episodeId, shotIds, route: route ?? null });

/**
 * 细化时间线：一镜一次，把每一刻写满（结构不动，只换时间段）。**后台任务**，立即返回；
 * 逐镜跑、可停，某一镜失败记下接着做。细化过的镜头提示词会过期，要重新合成。
 */
/** 出首帧 / 尾帧会发给出图模型的东西（和真出图同一个函数拼，不出图） */
export interface KeyframePreview {
  engine: string;
  prompt: string;
  /** 挂的参考图（本机路径），顺序就是提示词里的「图 N / Image N」 */
  images: string[];
  negative_prompt?: string;
}
export const cinemaKeyframePreview = (projectId: string, episodeId: string, shotId: string, which: "first" | "last") =>
  invoke<KeyframePreview>("cinema_keyframe_preview", { projectId, episodeId, shotId, which });

export const cinemaRefineTimelines = (projectId: string, episodeId: string, shotIds: string[]) =>
  invoke<Job>("cinema_refine_timelines", { projectId, episodeId, shotIds });

/**
 * 批量出片（按项目当前线路）。**后台任务**，立即返回登记的任务；某一镜失败记下、接着出下一镜。
 */
export const cinemaRenderEpisodeVideos = (projectId: string, episodeId: string, shotIds: string[]) =>
  invoke<Job>("cinema_render_episode_videos", { projectId, episodeId, shotIds });

/**
 * **加入出片队列**：这一集有片在出时，排到它后面（有批量在跑就追加进那一条；只有单镜在出就开一条批量、等它出完再开始）。
 * 立即返回那条批量任务。
 */
/**
 * 合成这几镜；这一集已经在合成同一条线路的话，**排到那一轮后面**（和出片的「加入队列」一样，追加进在跑那一条的名单）。
 * 在合另一条线路时报错。`route` 不传 = 项目当前线路。
 */
export const cinemaQueueSynthesize = (projectId: string, episodeId: string, shotIds: string[], route?: PromptRoute) =>
  invoke<Job>("cinema_queue_synthesize", { projectId, episodeId, shotIds, route: route ?? null });

export const cinemaQueueShotVideos = (projectId: string, episodeId: string, shotIds: string[]) =>
  invoke<Job>("cinema_queue_shot_videos", { projectId, episodeId, shotIds });

/**
 * 批量出首帧。**后台任务**；`redo = false` 时已经有首帧的镜头跳过（导入 / 挑过的不会被一键盖掉）。
 * 每一镜挂这一镜的场景、人物、道具图（参考生图引擎），裁成项目画幅。
 */
export const cinemaRenderEpisodeFrames = (projectId: string, episodeId: string, shotIds: string[], redo: boolean) =>
  invoke<Job>("cinema_render_episode_frames", { projectId, episodeId, shotIds, redo });

/** 出片链路：首帧出片（只发首帧）/ 参考出片（发人物场景参考图）。见 Rust `shot_route` */
export type ShotRoute = "keyframe" | "reference";

/** 一镜走哪条链路、为什么、有没有被拦（Rust `cinema::ShotRouteInfo`） */
export interface ShotRouteInfo {
  shot_id: string;
  shot_num: number;
  /** 实际走哪条：有首帧 = 首帧出片 */
  actual: ShotRoute;
  /** 按分镜内容建议走哪条 */
  advised: ShotRoute;
  reason: string;
  /** 首帧出片却有人中途进场：拦下的原因 */
  blocker: string | null;
  /** 「接上一镜」按规则该是什么 */
  auto_continuity: "none" | "ref_video" | "first_frame";
}

export const cinemaShotRoutes = (projectId: string, episodeId: string) =>
  invoke<ShotRouteInfo[]>("cinema_shot_routes", { projectId, episodeId });

/** 导入（`media` 给图）或摘掉（`media` 为 `null`）一镜的首帧 / 尾帧。返回这一镜的新状态 */
export const cinemaSetShotKeyframe = (
  projectId: string,
  episodeId: string,
  shotId: string,
  which: "first" | "last",
  media: import("./assets").MediaInput | null,
) => invoke<StoryboardShot>("cinema_set_shot_keyframe", { projectId, episodeId, shotId, which, media });

/**
 * 出一镜的首帧：用项目设置里的首帧模型 + 这一镜合成好的分镜提示词；参考生图引擎会挂场景、人物、道具图。
 * 这一镜设了「首帧接上一镜末帧」时直接拿上一镜的末帧。
 */
export const cinemaRenderShotFrame = (projectId: string, episodeId: string, shotId: string) =>
  invoke<RenderShotResult>("cinema_render_shot", { req: { project_id: projectId, episode_id: episodeId, shot_id: shotId, engine_config: null } });

/** 从场景资产的全景生成这个地点的 3D 片场（SHARP，十几秒），存进场景资产 */
export const cinemaGenerateSceneWorld = (projectId: string, episodeId: string, assetId: string) =>
  invoke<import("./assets").SceneWorld>("cinema_generate_scene_world", { projectId, episodeId, assetId });

/** 合并写这一镜的导演世界场面（顶层键覆盖） */
export const cinemaSetDirectorScene = (projectId: string, episodeId: string, shotId: string, patch: DirectorSceneState) =>
  invoke<DirectorSceneState>("cinema_set_director_scene", { projectId, episodeId, shotId, patch });

/** 人偶稿 → 角色首帧 / 尾帧（试验）：按人偶颜色换成角色图，结果挂进这一镜的关键帧 */
export const cinemaRenderFrameFromDirector = (projectId: string, episodeId: string, shotId: string, which: "first" | "last", draftPath: string, actors: DraftActor[]) =>
  invoke<StoryboardShot>("cinema_render_frame_from_director", { projectId, episodeId, shotId, which, draftPath, actors });

/** 出一镜的尾帧：首帧当第一张参考图，按这一镜的结尾画面出。要先有首帧 */
export const cinemaRenderEndFrame = (projectId: string, episodeId: string, shotId: string) =>
  invoke<StoryboardShot>("cinema_render_end_frame", { projectId, episodeId, shotId });

/** 只重新生成一场（其它场的镜头、提示词、出过的片都不动）。后台任务，立即返回登记的任务 */
export const cinemaRegenerateScene = (projectId: string, episodeId: string, sceneNo: number) =>
  invoke<Job>("cinema_regenerate_scene", { projectId, episodeId, sceneNo });

/** 自动修正镜头表里反复出现的固定写法（夹英文、二选一、台词重复、切镜、说话人），返回改了几处 */
export const cinemaRepairStoryboard = (projectId: string, episodeId: string) =>
  invoke<number>("cinema_repair_storyboard", { projectId, episodeId });

/**
 * 「自动修正」**真能改掉**的那几条（后端在内存里修一遍、修完不见了的）。按钮上的数字以它为准 ——
 * 按问题文字猜会把改不了的也算进去（EP02 邻居甲那类没有资产的说话人），点了没反应
 */
export const cinemaRepairableIssues = (projectId: string, episodeId: string) =>
  invoke<ShotIssue[]>("cinema_repairable_issues", { projectId, episodeId });

/** 红色问题框右边的「修复」按钮。和 Rust `cinema::IssueFixOffer` 同形 */
export type IssueFix =
  | { kind: "duration"; secs: number }
  | { kind: "speaker"; line: number; speaker: string; off_screen: boolean }
  | { kind: "repair" }
  /** 重新合成这一镜（当前线路）—— 线路 B 正文的英文二选一，合成时写手会被要求只写一个版本 */
  | { kind: "resynth" }
  /** 把这张有图、却没挂上的资产挂成参考图（悬浮车那类：镜头表生成后才出的图） */
  | { kind: "mount"; asset_id: string; usage: RefSlot["usage"]; control: string };

export interface IssueFixOffer {
  shot_num: number;
  /** 挂在哪一条问题旁边（按原文对上） */
  message: string;
  label: string;
  fix: IssueFix;
}

/** 每条问题能怎么一键修（后端算：加长到几秒、改成谁说、能不能自动修正）。改由界面在那一镜上改、走正常保存 */
export const cinemaIssueFixes = (projectId: string, episodeId: string) =>
  invoke<IssueFixOffer[]>("cinema_issue_fixes", { projectId, episodeId });

/** 校验这一集的镜头表 */
/** 两条线路各自过期的镜头 id。见 Rust `cinema::StalePrompts` —— 问题清单只报当前线路的，合成弹窗要两条都知道 */
export interface StalePrompts {
  a: string[];
  b: string[];
}
export const cinemaStalePrompts = (projectId: string, episodeId: string) =>
  invoke<StalePrompts>("cinema_stale_prompts", { projectId, episodeId });

export const cinemaValidateStoryboard = (projectId: string, episodeId: string) =>
  invoke<ShotIssue[]>("cinema_validate_storyboard", { projectId, episodeId });

/** 给本集出场角色补齐颜色并存盘。幂等，已有的颜色不动。 */
export async function cinemaAssignActorColors(projectId: string, episodeId: string): Promise<ActorColorAssignment> {
  return invoke("cinema_assign_actor_colors", { projectId, episodeId });
}

/** 一场整段出（长镜头）。约束和代价见 `storyboard/longTake.ts` */
export function cinemaRenderLongTake(projectId: string, episodeId: string, sceneNo: number): Promise<Job> {
  return invoke<Job>("cinema_render_long_take", { projectId, episodeId, sceneNo });
}

export async function cinemaSaveEpisode(projectId: string, episode: EpisodeData): Promise<void> {
  return invoke("cinema_save_episode", { projectId, episode });
}

/**
 * **只存这一镜**（分镜详情用）。`base` 是草稿从哪一份开始改的：后台写回的时间段 / 提示词界面没动就保留，
 * 两边都改了就报错不存。出片结果、关键帧、导演世界场面以盘上为准。
 */
export function cinemaSaveShot(projectId: string, episodeId: string, shot: StoryboardShot, base: StoryboardShot): Promise<void> {
  return invoke("cinema_save_shot", { projectId, episodeId, shot, base });
}

/**
 * 在访达 / 资源管理器里**定位到这个文件**（选中它，不是打开目录）。
 *
 * 「组装 / 导出」那一步的产出是一个 mp4，而真正要做的事
 * （发给别人、拖进剪辑软件、存到别处）都得先在文件管理器里找到它。
 */
export async function cinemaRevealFile(path: string): Promise<void> {
  return invoke<void>("cinema_reveal_file", { path });
}

/**
 * 合成一集，后台任务版。`upscale` 时逐镜 RealESRGAN ×2、整集出 1080P ——
 * 一镜一两分钟，整集几十分钟，所以登记成任务，切页、刷新都不断。
 */
/**
 * 按项目的名字、简介、剧本开头、主要角色和风格自动出一张封面（本机后台模型写提示词 + Z-Image），
 * 写回项目的 `cover_image`。约一两分钟。回新封面的本机路径。
 */
/** 设项目里某个角色的默认形象。`main` = 主形象 */
export async function cinemaSetDefaultLook(projectId: string, assetId: string, lookId: string): Promise<void> {
  return invoke<void>("cinema_set_default_look", { projectId, assetId, lookId });
}

export async function cinemaGenerateCover(projectId: string): Promise<string> {
  return invoke<string>("cinema_generate_cover", { projectId });
}

/** 上传一张图当项目封面（data URL）。存成项目目录里的 PNG，替掉旧封面。回新封面的本机路径。 */
export function cinemaUploadCover(projectId: string, dataUrl: string): Promise<string> {
  return invoke<string>("cinema_upload_cover", { projectId, dataUrl });
}

export async function cinemaStitchEpisodeStart(
  projectId: string,
  episodeId: string,
  upscale: boolean,
  cover: boolean,
): Promise<Job> {
  return invoke<Job>("cinema_stitch_episode_start", { projectId, episodeId, upscale, cover });
}

/**
 * 出这一集的封面：本集梗概 + 项目风格出画面，**系列名小字、剧名大字、集号**
 * 三行由 Z-Image 直接画进画面。约一分钟。回封面的本机路径。
 */
export async function cinemaGenerateEpisodeCover(projectId: string, episodeId: string): Promise<string> {
  return invoke<string>("cinema_generate_episode_cover", { projectId, episodeId });
}

/**
 * 这一集现有的封面路径，没有回 `null`。
 *
 * **问的是磁盘上有没有 `cover.png`，不是 `episode.json` 里的字段** ——
 * `EpisodeData` 没有封面字段，存进去会被下一次保存抹掉。
 */
/**
 * 给成片加 / 撤片头封面。**不重新渲染任何镜头** —— 已经合成好、
 * 甚至已经超分过的成片也能直接加，几秒钟的事。按几次都一样（从母版重做）。
 */
export async function cinemaSetCoverIntro(projectId: string, episodeId: string, on: boolean): Promise<void> {
  return invoke<void>("cinema_set_cover_intro", { projectId, episodeId, on });
}

/** 成片现在是不是带片头的那版 */
export async function cinemaHasCoverIntro(projectId: string, episodeId: string): Promise<boolean> {
  return invoke<boolean>("cinema_has_cover_intro", { projectId, episodeId });
}

export async function cinemaEpisodeCover(projectId: string, episodeId: string): Promise<string | null> {
  return invoke<string | null>("cinema_episode_cover", { projectId, episodeId });
}

export interface CharacterAngle {
  angle_name: string; // "0° 正面特写", "左侧 45°", "右侧 45°", "90° 正侧剪影", "电影仰角"
  image_url: string;
  description: string;
}

export interface CharacterViewItem {
  key: string;
  title: string;
  image_url: string;
  description: string;
  category: "turnaround" | "expression" | "pose" | "angle";
}

export interface CharacterAssetSheet {
  turnarounds: CharacterViewItem[]; // 全身三视图：正面、侧面、背面
  expressions: CharacterViewItem[]; // 6 大表情模组：喜怒哀乐哭笑怒惊
  poses: CharacterViewItem[];       // 4 大动作姿态：站姿、奔跑、跃起、崩溃
  angles: CharacterAngle[];         // 5 维面部机位
  /**
   * 这次生成里出的问题。**要显示出来。**
   *
   * 早先这些只进 Rust 侧的日志 —— 于是「表情那一档六张全被 QC 拒掉」
   * 表现成界面上一个空的「表情模组 (0/6)」，用户分不清是没做、
   * 做失败了、还是做完了没显示。整档失败是这套流程里最贵的失败，
   * 不能是安静的。
   */
  warnings?: string[];
}

export interface CharacterVisualDna {
  eyes?: string;
  jaw?: string;
  nose?: string;
  mouth?: string;
  hair?: string;
  hair_color?: string;
  skin?: string;
  signature_outfit?: string;
}

export interface CinemaCharacter {
  id: string;
  name: string;
  role: string;
  avatar_url: string;
  trigger_words: string;
  lora_model?: string | null;
  lora_weight: number;
  consistency_mode: "prompt_only" | "pulid" | "ip_adapter_faceid" | "instant_id" | "lora" | "reference_only";
  clothing_prompt?: string | null;
  multi_angles?: CharacterAngle[];
  asset_sheet?: CharacterAssetSheet | null;
  visual_dna?: CharacterVisualDna | null;
  /**
   * 这个角色的固定种子，首次生成时自动记下。
   *
   * 它管的是**可复现**：同样的提示词 + 同样的种子 = 同一张图。
   * 跨镜头的角色一致性靠的是参考图（IPAdapter/PuLID），不是它 ——
   * 两者解决的是不同问题，都需要。
   */
  seed?: number | null;
  /** 它住在哪一层：`null` = 项目级（跨集共用），有值 = 本集级。 */
  episode_id?: string | null;
}

export interface CinemaEngineConfig {
  llm_provider: string; // "ollama" | "lmstudio" | "custom"
  llm_endpoint: string; // "http://127.0.0.1:11434"
  llm_model: string; // "qwen2.5:7b", "deepseek-r1:14b", "llama3.2"
  checkpoint: string; // "AnimagineXL_V3.1.safetensors", "RealVisXL_V5.0_fp16.safetensors"
  lora?: string | null;
  lora_weight: number; // 0.8
  consistency_engine: "pulid" | "ip_adapter_faceid" | "instant_id" | "reference_only" | "lora";
  sampler: string; // "euler", "dpmpp_2m_karras"
  steps: number; // 25
  cfg: number; // 7.0
  video_engine: string; // "ltx" | "minimax_h3" | "liveportrait" | "camera_preview"；"wan2.1" 已下架，"hunyuan_i2v" | "svd" | "animatediff" 只是占位
  video_workflow_id: string;
  custom_workflow_json?: string | null;
  /** 把台词烧进画面。默认 true —— 模型自己画的中文字幕认不出来（实测），
   *  而台词是我们自己写的，后期渲染是像素级正确的。 */
  burn_subtitles?: boolean;
}

export interface RenderShotReq {
  project_id: string;
  episode_id: string;
  shot_id: string;
  engine_config?: CinemaEngineConfig;
}

export interface RenderShotResult {
  shot_id: string;
  image_url: string;
  video_url: string;
  audio_url: string;
  status: string;
}

/**
 * 读这个项目的角色。
 *
 * 传了 `episodeId` 就**两层都读**：本集（刚提取、还没归属）排前面，
 * 然后是项目级（跨集共用）。不传只读项目级 —— 那样刚提取出来的角色
 * 点进去会说「找不到」。
 */
export async function cinemaListCharacters(
  projectId: string,
  episodeId?: string,
): Promise<CinemaCharacter[]> {
  return invoke<CinemaCharacter[]>("cinema_list_characters", {
    projectId,
    episodeId: episodeId ?? null,
  });
}

export async function cinemaRenderShotVideo(req: RenderShotReq): Promise<RenderShotResult> {
  return invoke<RenderShotResult>("cinema_render_shot_video", { req });
}

export interface GenerateSheetComfyReq {
  project_id: string;
  character_id: string;
  name: string;
  trigger_words: string;
  clothing_prompt?: string | null;
  front_image_url?: string | null;
  checkpoint?: string | null;
  lora?: string | null;
  lora_weight?: number | null;
  consistency_mode?: CinemaCharacter["consistency_mode"] | null;
  seed?: number | null;
  visual_dna?: CharacterVisualDna | null;
  /**
   * 这次要生成哪几档。省略 = 只做三视图。
   *
   * **全套 18 张在 MPS 上约 10 分钟**，所以默认不全做 ——
   * 让用户自己决定什么时候花这个时间。
   */
  tiers?: Array<"turnaround" | "expressions" | "poses" | "angles"> | null;
  /**
   * 三视图是否也用底图当参考。**默认 false。**
   *
   * 参考图（多半是半身肖像）会把构图一起搬过来，实测整排会塌成
   * 「四个半身正面」；不挂时出来的是干净的六视图。
   * 身份一致性靠这一步的产出 —— 正面那格会成为后续分镜的参考图。
   */
  use_reference_for_turnaround?: boolean | null;
  /** 调试模式：只跑每档第一格，并让后端用低步数/单次尝试。 */
  debug_one_slot?: boolean | null;
  /** 单张图最多尝试次数。调试模式通常传 1。 */
  max_attempts?: number | null;
  /**
   * 三视图整排走哪条路。省略 = `"sdxl"`。
   *
   * **只影响三视图那一档** —— 表情/姿势/机位是单格生成，不依赖版式 LoRA。
   *
   * 两条路上底模、版式 LoRA、分辨率**全都不通用，而且混搭起来不报错**：
   * SDXL 的 LoRA 挂到 SD1.5 底模上照常出图，只是完全不起作用。
   * 所以选了引擎之后这几样由 Rust 侧一起决定，前端不要单独传 checkpoint/lora
   * 去凑 —— 传了不同族的会被忽略。
   */
  engine?: SheetEngine | null;
}

/**
 * 三视图整排的两条路。
 *
 * - `sdxl`：RealVisXL + CharTurnXL，1536×1024 一段出图。单格分辨率高一档。
 * - `sd15`：majicmix + three_sided_view，832×512 出图再放大到 1664×1024。
 *   实测东亚面孔天生就对、服装还原更准、格与格之间有真背景间隔所以切图更可靠。
 */
export type SheetEngine = "sdxl" | "sd15";

export async function cinemaGenerateCharacterSheetViaComfyui(
  req: GenerateSheetComfyReq,
): Promise<CharacterAssetSheet> {
  return invoke<CharacterAssetSheet>("cinema_generate_character_sheet_via_comfyui", { req });
}

/** 整集软字幕（和成片同名的 final.srt）。见 Rust `cinema::export_srt` */
export interface SrtExport {
  path: string;
  content: string;
}

/**
 * 重新生成整集字幕并回正文。合成一集时也会写一份，这里是「不重新合成也能拿到最新的」。
 *
 * 时间轴按每段片子的**真实时长**接（出片实际长度和分镜上写的差一点，攒下来能差几秒）。
 */
export async function cinemaExportSrt(projectId: string, episodeId: string): Promise<SrtExport> {
  return invoke<SrtExport>("cinema_export_srt", { projectId, episodeId });
}

/** 导出到 Final Cut Pro 的选项。见 Rust `cinema_export_fcp::ExportOptions` */
export interface FcpExportOptions {
  /** 用户选的位置（可以是外置硬盘）。这一集放进它下面以项目名命名的子文件夹 */
  dest_dir: string;
  /** FCP 里的事件名 —— 用户自己建事件，照他填的写 */
  event: string;
  /** FCP 里的项目名，也是子文件夹和 .fcpxml 的名字 */
  project: string;
  /** 逐镜 RealESRGAN ×2，和「合成一集」共用超分缓存 */
  upscale: boolean;
}

/** 后台任务：拷素材、写 .fcpxml + .srt、过 FCP 的格式校验，完了自动用 FCP 打开 */
export const cinemaExportFcpStart = (projectId: string, episodeId: string, options: FcpExportOptions) =>
  invoke<Job>("cinema_export_fcp_start", { projectId, episodeId, options });

/** FCP 里这一集的事件名 / 项目名：照导出位置里资源库已有的事件对齐（Rust `cinema_export_fcp::fcp_names`） */
export interface FcpNames {
  event: string;
  project: string;
  library: string | null;
}
export const cinemaFcpNames = (destDir: string, episodeNum: number, title: string) =>
  invoke<FcpNames>("cinema_fcp_names", { destDir, episodeNum, title });

/** 系统的选文件夹框（外置硬盘也在里面）。取消回 null */
export const cinemaChooseExportFolder = (defaultDir?: string) =>
  invoke<string | null>("cinema_choose_export_folder", { defaultDir: defaultDir || null });

export async function cinemaExportJianyingProject(
  projectId: string,
  episodeId: string,
): Promise<string> {
  return invoke<string>("cinema_export_jianying_project", { projectId, episodeId });
}

/** 从剧本抽出来的一条，**还没入库**。`exists` = 项目里已经有同名同类的了。 */
export interface ExtractedEntity {
  kind: "character" | "scene" | "prop";
  name: string;
  description: string;
  persona: string;
  exists: boolean;
  /**
   * **这一集顺带出现的**：龙套（邻居甲、打手乙）、一次性的普通物件（铁棍、电锅）。入库弹窗里默认「不要」，
   * 镜头里按文字描述出（用户 2026-09-30：资产库按整部剧的设定建，这些不需要单独建资产）。旧数据没有 = false
   */
  incidental?: boolean;
  /**
   * 只有声音、没有能出镜的身体：旁白、电话那头的人、纯语音 AI。
   *
   * 分镜会读它 —— 标了的角色台词做成画外音，不单独给镜头。不标的后果
   * 实测过：一个悬浮光点 AI 每说一句就被切一个特写，二十二镜里九镜
   * 是同一张画面。
   *
   * 可选是因为**旧数据里没有这个字段**，后端按 false 处理。
   */
  disembodied?: boolean;
}

/** 读剧本抽角色/场景/道具，**只返回预览，不入库**。 */
export const cinemaExtractEntities = (projectId: string, episodeId: string) =>
  invoke<ExtractedEntity[]>("cinema_extract_entities", { projectId, episodeId });


/** 把勾选的建成项目级资产，返回新建了几个。 */
/**
 * 存项目的生成设置。返回回写之后的项目。
 *
 * 后端会校验画幅 —— 传了引擎产不出的比例会**当场报错**，
 * 而不是存下来等出图那一刻炸。
 */
export const cinemaSaveModelSettings = (projectId: string, settings: ModelSettings) =>
  invoke<CinemaProject>("cinema_save_model_settings", { projectId, settings });

/** 首帧用参考生图引擎时挑了哪几张资产图。排序只在 Rust 的 `frame_reference_pick` 里写一份。 */
export interface FrameRefPick {
  images: string[];
  names: string[];
  unused: string[];
}

export const cinemaFrameReferencePick = (
  prompt: string,
  dialogueLines: DialogueLine[],
  blocking: StageSubject[],
  engine: string,
) => invoke<FrameRefPick>("cinema_frame_reference_pick", { prompt, dialogueLines, blocking, engine });

/**
 * 把勾选的实体建进**系列池**，系列里已有同类同名的并进去（只补空字段）。
 *
 * `episodeOnly` 是用户点成「只留本集」的那些（`类型/名字`），建成本集资产。
 * 2026-09-25 以前全部落本集、等「对齐」—— 对齐一直没做，每集的角色各是各的。
 */
export interface ImportSummary {
  /** 系列里新建的 */
  added: number;
  /** 系列里已有、并进去的 */
  merged: number;
  /** 只留本集的 */
  episode_only: number;
  /** 同一个人的其它造型，加成了形象（不算新角色）。老后端没有这个字段 */
  looks?: number;
}

export const cinemaImportEntities = (
  projectId: string,
  episodeId: string,
  entities: ExtractedEntity[],
  episodeOnly: string[] = [],
) =>
  invoke<ImportSummary>("cinema_import_entities", { projectId, episodeId, entities, episodeOnly });

/**
 * 删项目。**不可逆。**
 *
 * 两步：`confirm=false` 只回一句「里面有什么」，让界面先说清楚要删掉多少
 * 东西；用户看过之后再带 `confirm=true` 真删。
 */
export const cinemaDeleteProject = (projectId: string, confirm: boolean) =>
  invoke<string>("cinema_delete_project", { projectId, confirm });

/** 删系列里的一集。`confirm=false` 只回「里面有什么」，见 Rust `studio_ops::delete_episode` */
export const cinemaDeleteEpisode = (projectId: string, episodeId: string, confirm: boolean) =>
  invoke<string>("cinema_delete_episode", { projectId, episodeId, confirm });

/**
 * 跨项目的剧本一览。
 *
 * **结构文档原样带过来，场数/角色数在前端算** ——
 * 那套识别规则在 `script/derive.ts`，在后端再写一份两边迟早对不上，
 * 而对不上时没有任何东西会报错。
 */
export interface ScriptSummary {
  project_id: string;
  project_title: string;
  project_kind: "series" | "standalone";
  episode_id: string;
  episode_num: number;
  episode_title: string;
  /** 纯文本字数（字符数，不是字节数） */
  chars: number;
  /** null = 还没在编辑器里打开过 */
  script_doc: string | null;
  shot_count: number;
}

export const cinemaScripts = () => invoke<ScriptSummary[]>("cinema_scripts");

/** 这一镜存下的最终提示词 —— 和出片、出首帧发送的是同一份。 */
export interface ShotPrompts {
  /** 项目当前线路 */
  route: PromptRoute;
  /** 当前线路的分镜提示词 */
  frame: string;
  /** 当前线路的视频运动提示词 */
  video: string;
  /** 每条线路各自存的 */
  routes: Partial<Record<PromptRoute, RoutePrompts>>;
}

/** 读一镜存下的提示词。画布的镜头节点用它，和成片发送的是同一份。 */
export const cinemaShotPromptPreview = (projectId: string, episodeId: string, shotId: string) =>
  invoke<ShotPrompts>("cinema_shot_prompt_preview", { projectId, episodeId, shotId });

/**
 * 把画布上的一张图收成这一镜的首帧。返回新的 `image_rev`。
 *
 * 复制到约定死的 `shots/<id>.png` —— 记路径的话画布那边一清理，
 * 成片就少一帧，而且到导出才发现。
 */
export const cinemaAdoptFrame = (
  projectId: string,
  episodeId: string,
  shotId: string,
  sourcePath: string,
) => invoke<number>("cinema_adopt_frame", { projectId, episodeId, shotId, sourcePath });

/** 回填一段视频之后的交代。 */
export interface AdoptVideoOut {
  video_url: string;
  /** 这一镜现在总共几「次」 */
  take_count: number;
}

/**
 * 把画布上出的一段视频收进某一镜。**追加成新的一「次」，不覆盖已有的片子** ——
 * 画布出的片不一定比原来那条好，用户得能用 {@link cinemaPickTake} 换回去。
 */
export const cinemaAdoptVideo = (a: {
  projectId: string;
  episodeId: string;
  shotId: string;
  sourcePath: string;
  prompt: string;
  engine: string;
  seed?: number;
  /** 实际用的参考图，按顺序。见 `VideoTake.picture_paths` */
  picturePaths?: string[];
}) => invoke<AdoptVideoOut>("cinema_adopt_video", a);

/** 换回某一「次」。只改 `video_url` 的指向，历史一条不动。 */
export const cinemaPickTake = (
  projectId: string,
  episodeId: string,
  shotId: string,
  takePath: string,
) => invoke<string>("cinema_pick_take", { projectId, episodeId, shotId, takePath });

// ── 配音（TTS 音色克隆）──────────────────────────────────────────────

/** 一镜配完之后的交代。每一条都要显示给用户。 */
export interface DubOutcome {
  shot_id: string;
  lines: number;
  /** `[说话人, 音色, 是不是逐句覆盖的]` */
  voices: [string, string, boolean][];
  out_secs: number;
  video_secs: number;
  /** 有值 = **台词尾巴会在拼接时被砍掉**（要么改短台词，要么重出更长的镜头） */
  overflow_secs: number | null;
  /** 这一镜原来有 H3 生成的原声 —— 换配音等于放弃口型 */
  replaced_source_audio: boolean;
}

/**
 * 配音。`shotId` 不传 = 整集。
 *
 * 配音一旦生成，拼接就**用它替代 H3 的原声**（`pick_shot_audio`）。
 * 音色、方言归你管，代价是口型对不上 —— 原口型是跟着 H3 那段语音做的。
 *
 * 进度走 `anima://cinema-dub-progress` 事件。
 */
export const cinemaDub = (projectId: string, episodeId: string, shotId?: string) =>
  invoke<DubOutcome[]>("cinema_dub", { req: { project_id: projectId, episode_id: episodeId, shot_id: shotId ?? null } });

/** 删掉配音，换回 H3 原声（口型也就跟着对上了）。回删掉几个。 */
export const cinemaClearDub = (projectId: string, episodeId: string, shotId?: string) =>
  invoke<number>("cinema_clear_dub", { req: { project_id: projectId, episode_id: episodeId, shot_id: shotId ?? null } });

/** 哪些镜头已经配过音。判据是文件在不在，不存状态 —— 不会有对不上的可能。 */
export const cinemaDubbedShots = (projectId: string, episodeId: string) =>
  invoke<string[]>("cinema_dubbed_shots", { req: { project_id: projectId, episode_id: episodeId, shot_id: null } });

/** 当前 TTS 引擎能用的音色。 */
export interface VoiceTemplates {
  /** 引擎显示名。**必须显示** —— 音色名的命名空间每家一套，不说出处等于没有出处 */
  engine: string;
  voices: string[];
  /** 列表为空时的原因 + 怎么办。有列表时是空串 */
  hint: string;
}

/**
 * 可选的音色。**问引擎自己要**，不是只看共享音色库。
 *
 * Edge 会回微软的完整目录，Kokoro 之类试 `/v1/audio/voices`，
 * GPT-SoVITS 回本机音色库。真正没有音色名的只有 CosyVoice2 / IndexTTS2
 * —— 它们的音色就是参考音频，那种情况 `hint` 会说明白。
 */
export const cinemaVoiceTemplates = () => invoke<VoiceTemplates>("cinema_voice_templates");

/** 绑定（或解绑，传 null）剧本编辑器里的项目 */
export const cinemaLinkScriptProject = (projectId: string, scriptProjectId: string | null) =>
  invoke<CinemaProject>("cinema_link_script_project", { projectId, scriptProjectId });

/** 改整部剧的画幅（`renderOptions.ts` 的 `ASPECTS` 里的一种，含 21:9）。只改这一个字段；已经出的首帧、视频要重出才对得上。 */
export const cinemaSetAspectRatio = (projectId: string, aspectRatio: string) =>
  invoke<CinemaProject>("cinema_set_aspect_ratio", { projectId, aspectRatio });
