import type { Node, Edge } from "@xyflow/react";
import type { Scope } from "../api/assets";

export interface CanvasReference {
  sourceNodeId?: string;
  path: string;
  name: string;
  assetId?: string;
  scope?: Scope;
  variantId?: string;
}

export type CanvasNodeType =
  | "prompt"
  | "shot"
  | "character"
  | "imageGen"
  | "videoGen"
  | "shotContext"
  | "skill"
  | "panorama"
  | "panoViewer"
  | "directorWorld"
  | "depthCapture"
  | "lora"
  | "audio"
  | "audioClip"
  | "upscale"
  | "group";

/**
 * 分镜节点 —— 把分集里的**某一镜**接进画布。
 *
 * # 它补的是画布和成片之间的断口
 *
 * 画布本来是**第二条管线**：分集里的分镜进不去，要用就对着自由文本框
 * 重打一遍。重打的那一遍不带角色身份锚点、不带场景地理锁、不带景别
 * 运镜和风格定调 —— 画布里出的图和成片里出的图长得不一样，
 * 而两边都不报错，只有把两张图并排放才看得出来。
 *
 * 所以这个节点**不自己拼提示词**，它向后端要
 * （`cinema_shot_prompt_preview`），拿到的和出片那一刻发出去的是同一段。
 *
 * # 为什么把提示词缓存在节点里
 *
 * 下游拼提示词是同步的（`aggregateNodeInputs`），那里不能 await。
 * 缓存的代价是分镜在别处改了这里不会自己变 —— 所以节点上有刷新，
 * 并且**显示取的是哪一镜**，让人看得出手里这份是不是旧的。
 */
export interface ShotNodeData {
  projectId?: string;
  episodeId?: string;
  shotId?: string;
  /** 「余波 · S01E01 · 第 3 镜」—— 人要看得出自己接的是哪一镜 */
  label: string;
  /** 后端算好的首帧提示词。空 = 还没取到，下游不发任何东西 */
  frame: string;
  /** 提示词里有几个引用没在资产库里对上。对不上就只剩名字，描述不进提示词 */
  unresolved: string[];
  /** 台词装不装得下这个镜长。H3 超容量会**静默截断**，所以必须显示 */
  dialogueWarning?: string;
  loading?: boolean;
  error?: string;
  [key: string]: unknown;
}

/**
 * 镜头上下文节点 —— **画布上的连续性契约**。
 *
 * 成片链路里，跨镜一致性靠的是场景空间契约 + 角色身份锚点 + 站位，
 * 这些东西一条都进不了画布：画布上只有一个自由的文本框。
 * 于是同一个画布里连出来的两镜，地理、光线、谁站哪边全靠运气。
 *
 * 这个节点把那套东西搬过来。它**不生成任何东西**，只往下游发文字。
 *
 * # 地理锁和文字地图是分开的两份
 *
 * `viewMap` 长，发了参考图就该省掉（图已经说清楚了，再写一遍会打架）；
 * `lock` 极短，**发了参考图也要发** —— 一张场景板只画了一个方向，
 * 换个方向的镜头里「柜台在哪、有几扇门」图上根本没有。
 *
 * # 身份和状态分开填
 *
 * `state`（时段 / 天气 / 停电）**不进空间身份**。那些变了，
 * 空间还是同一个空间 —— 混在一起的表现是「同一个房间被当成两个场景」。
 */
export interface ShotContextNodeData {
  /** 绑的场景资产。空 = 还没挑，节点上要显示「先挑一个场景」 */
  sceneAssetId?: string;
  sceneName: string;
  /** 挑中的视角名（来自契约的 `views`）。空 = 不限定方向 */
  viewName?: string;
  /** 视角的文字地图。从契约取下来**缓存在节点里** —— 下游拼提示词是同步的 */
  viewMap: string;
  /** 地理锁。短，且无条件发 */
  lock: string;
  /** 站位：谁在画面哪一侧。**屏幕空间，不是世界坐标** —— 我们没有几何 */
  blocking: string;
  /** 时段 / 天气 / 灯的开关。会变的那一半 */
  state: string;
  [key: string]: unknown;
}

/**
 * 技能节点 —— 把镜头语言词表接到画布上。
 *
 * 词表是 `shared/shot-language.json`，**三边读同一份字节**
 * （Rust / 前端 / 生成 skills markdown 的脚本）。这里不另建一套：
 * 另建一套的表现是「分镜里选的『推镜头』和画布里选的不是同一个东西」，
 * 而两边都不报错。
 *
 * 挑中的是**词表 id**（`groupId:termId`），不是显示串 ——
 * 发给模型的文本按下游引擎的语言现算（中文引擎发中文）。
 * 存显示串的老毛病是模型只收到「推镜头」四个字。
 */
/**
 * 360° 全景节点 —— 把空间契约铺成一圈。
 *
 * # 它也替掉了一个假按钮
 *
 * 菜单里的「导演世界」本来建出来的是个**角色节点**（名字叫「新角色」）。
 * 和「镜头上下文」是同一种毛病：菜单承诺了能力，点下去是别的东西。
 *
 * # 为什么值得单独一个节点
 *
 * 全景不是「一张很宽的图」。它是等距圆柱投影，**左右边缘必须无缝**，
 * 长宽比必须 2:1，而且最典型的失败样子是同一扇门被画成左右各一扇 ——
 * 这几件事都得在提示词里明说，指望用户自己写等于指望它不发生。
 *
 * 连到图片节点时画幅由它定死，不让用户选 —— 和「视频模式由连线推导」
 * 同一个道理：图里已经决定了的事不该再问一遍。
 */
export interface PanoramaNodeData {
  sceneAssetId?: string;
  sceneName: string;
  /** 编译好的全景提示词。挑场景那一刻算好缓存住 */
  prompt: string;
  /** 长边像素。高 = 长边 / 2，等距圆柱投影不给别的选择 */
  longSide: number;
  /** 告警：视角不够、有推断项。**不阻断生成** */
  warnings: string[];
  [key: string]: unknown;
}

/**
 * 360° 全景查看器 —— **它的产出不是「浏览」，是取景截图**。
 *
 * # 这是我们第一次拿到「保证」那一层
 *
 * 跨镜地理一致性一直卡在同一处：让模型转到对面去，**它不转**。
 * 实跑两轮，「柜台反打」出来的还是从店堂朝柜台看，四个方位实际只落在
 * 两个机位上（`docs/ANIMA_PRODUCTION_ANALYSIS.md`「反打不转身」）。
 *
 * 这条路不问模型转不转，**它转的是相机**：同一张球面贴图，正打 yaw=0°、
 * 反打 yaw=180°，两张图的地理关系是几何保证的，不是提示词求来的。
 *
 * # 输入是上游的图，不是场景资产
 *
 * 吃的是上游图片节点出的那张等距圆柱全景（也能吃宫格图，只是那样球面
 * 会糊 —— 宫格不是全景）。所以典型接法是：
 * `全景出图 → 图片 → 360° 全景查看器`。
 */
export interface PanoViewerNodeData {
  /** 相机方位。yaw 0 = 正前方（`frontYawDeg` 定的那个正前方） */
  yawDeg: number;
  pitchDeg: number;
  fovDeg: number;
  /**
   * 球面校正。**生成的全景不可能正好水平**，而歪的地平线转到侧面会
   * 变成明显的倾斜 —— 校一次，所有机位都正。
   */
  correction: { roll: number; pitch: number; yaw: number };
  /** 正前方偏移：只改读数的原点，不动贴图 */
  frontYawDeg: number;
  aspect: string;
  /** `planet` = 小行星（球极投影）。不写 = 普通透视 */
  projection?: "planet";
  /** 小行星模式自己的视场角（90–330°），和透视的 `fovDeg` 分开存 */
  planetFovDeg?: number;
  /** 截下来的图（本机路径） */
  outputImages?: string[];
  outputImageUrl?: string;
  [key: string]: unknown;
}

/**
 * 导演世界 —— 站进全景球里摆人和道具。
 *
 * # 它不需要等 3DGS
 *
 * DramaClaw 的导演世界**世界来源有三种**：`sog`（3DGS）、`pano360`（全景球）、
 * `mesh`，三者并列，摆位和截图能力完全一样。所以全景就够站人；
 * 3DGS 是**换一种世界来源**，不是换一条路。
 *
 * # 它真正的产出是站位
 *
 * 分镜的 `blocking`（`StageSubject`）现在是手写的「沈星晴在画面左侧」。
 * 站进球里摆一次，屏幕左右和前后景是**算出来的** —— 相机转到反打，
 * 同一个人自动从画面左变成画面右。手写做不到这件事，
 * 而两镜之间左右对不上正是「乱套」的来源。
 */
export interface DirectorWorldNodeData {
  /** SHARP 后台任务；刷新页面后用它找回输出。 */
  worldJobId?: string;
  panoCubeJobId?: string;
  cubemapManifest?: string;
  namedCameras?: import("./director/scene").NamedCamera[];
  cameraPaths?: import("./director/scene").CameraPath[];
  /** 当前机位 id（界面升级 U2） */
  activeCameraId?: string;
  trajectoryVideo?: string;
  trajectoryDepthVideo?: string;
  trajectoryFirstFrame?: string;
  trajectoryLastFrame?: string;
  trajectoryMeta?: string;
  /**
   * 可选的 3DGS 世界。存在时优先于上游全景图。
   * 第一阶段先接入已有 .ply / .ksplat / .spz；场景图生成 3DGS 的后台任务后续接入。
   */
  splat?: {
    path: string;
    format: "ply" | "ksplat" | "spz";
    /**
     * 这份 splat 用的坐标系。SHARP 出的是**原相机坐标（OpenCV：+Z 朝前、+Y 朝下、原点是拍摄点）**，
     * 载入时要翻过来、抬到眼高（见 `scene.splatPlacement`）。手动载入的不知道来源，原样放。
     */
    convention?: SplatConvention;
    /** 片场缩放，没有 = 1（见 Rust `SceneWorld::scale`） */
    scale?: number;
  };
  /** 球面校正，和全景查看器同一套 */
  correction: { roll: number; pitch: number; yaw: number };
  frontYawDeg: number;
  /** 画幅 */
  aspect: string;
  /**
   * 世界校准：相机离地多高（米）。
   *
   * **全景里没有任何绝对尺度** —— 只能让人拖到地面网格贴着全景里的地板为止。
   * `scale` 是旧版「一身位多大」，只在迁移旧标记时用。
   */
  calibration?: { eyeHeight: number; scale: number };
  /** 人偶。见 `director/scene.ts` 的 `DirectorActor` */
  actors?: import("./director/scene").DirectorActor[];
  /** 编辑器里的相机 */
  camera?: import("./director/scene").DirectorCamera;
  /** 分集上下文。有 = 主线模式，人物只能从本集颜色表选 */
  context?: { projectId: string; episodeId: string; shotId: string | null } | null;
  /**
   * 旧版站位标记（方位角 + 距离）。**只读**：打开编辑器时换算成 `actors`，
   * 存盘后清空。见 `migrateMarkers`。
   */
  markers?: {
    id: string;
    kind: "actor" | "prop";
    label: string;
    yawDeg: number;
    distance: number;
    color: string;
  }[];
  /** 上次输出的合成图，卡片上显示 */
  lastCombined?: string;
  outputImages?: string[];
  outputImageUrl?: string;
  /** 最近一次的控制帧元数据（JSON 字符串），给下游读 */
  frameMeta?: string;
  [key: string]: unknown;
}

export interface SkillNodeData {
  /** `groupId:termId` 列表，例如 `shotSize:close_up` */
  picks: string[];
  [key: string]: unknown;
}

/**
 * 深度动作捕捉 —— 吃上游视频节点的视频，出一段黑白深度视频（越近越白）。
 *
 * 照 libtv 的做法：它自己不生成画面，下游视频节点接上它 + 一张人物图，
 * 就按这段动作出新视频。**只做画布节点，不接工作区**（用户 2026-09-14 定）。
 */
/**
 * 一个 LoRA 挂载点。连到出图 / 出片节点上就生效。
 *
 * # 为什么做成节点，而不是只留生成节点里那个按钮
 *
 * 用户 2026-09-20：「画布上还无法使用特定的 LoRA 作为节点」。
 *
 * 工具条里那个弹层是**每个生成节点各配各的** —— 看不见挂了什么，
 * 也没法两个节点共用一套。而画布本来的模型就是**连线决定一切**
 * （`inferVideoMode` 就是数连了几张图）。做成节点之后：挂了什么一眼看见、
 * 一次配好接给多个生成节点、复制一份就是一套新配方。
 *
 * **两者共用同一份数据** —— 执行时上游 LoRA 节点会合并进生成节点的
 * `loras`，不做两套（见 `collectLoras`）。
 */
export interface LoraNodeData {
  /** ComfyUI 枚举出来的名字，含子目录前缀（`zimage/xxx.safetensors`） */
  name?: string;
  /** 0~1.5。0 = 挂上但不生效（留着对照用） */
  strength: number;
  /** 把它的激活词并进提示词。默认开 */
  injectTriggers?: boolean;
  notes?: string[];
  [key: string]: unknown;
}

export interface DepthCaptureNodeData {
  /** 抽深度用的是哪段视频。上游换了视频，这里对不上就该重新捕捉 */
  sourceVideoPath?: string;
  /** 深度视频的本机路径 */
  depthVideoUrl?: string;
  isGenerating?: boolean;
  error?: string;
  notes?: string[];
  [key: string]: unknown;
}

/**
 * 组节点：一个框把几个节点圈在一起，成员的 `position` 相对组框。
 * 逻辑在 `group.ts`（改写自 DramaClaw 的普通组）。
 */
export interface GroupNodeData {
  label: string;
  /** 组底色的基础色（`GROUP_COLORS` 里的 value），不设就是默认灰 */
  color?: string;
  [key: string]: unknown;
}

/** 超分节点：上游一张图或一段视频 → ×2（RealESRGAN）。见 `utils/upscaleChain.ts` */
/**
 * 试音色：同一段词，几把嗓子并排听。
 *
 * # 它补的是什么
 *
 * 配音能力早就有（`cinema_dub`），但绑死在「某一集的某一镜」上 ——
 * 换个音色要走「改角色资产 → 整镜重新配音 → 听」，比三把嗓子得来回三趟，
 * 每趟还会覆盖掉上一趟。这里把试音色当成一个**动作**，不是一个库。
 *
 * # 口型这件事
 *
 * H3 原声的口型是跟着它自己生成的那段语音做的，**换成 TTS 配音口型就对不上**。
 * 这是个真取舍（见 Rust `cinema_dub` 的模块注释）。这个节点**不改变取舍**，
 * 只是让取舍看得见：并排听完再决定这一镜要原声还是要配音。
 */
export interface AudioNodeData {
  /** 要念的词。上游接了提示词节点就用上游的 */
  text?: string;
  /** 空 = 用当前引擎的默认音色 */
  voice?: string;
  /** 空 = 用设置里当前选的引擎 */
  engineId?: string;
  outputAudioUrl?: string;
  /** **实际用的**音色和引擎。和上面两个分开 —— 传了个不存在的音色时
   *  引擎会悄悄退回默认，只有这里看得出来 */
  usedVoice?: string;
  usedEngineId?: string;
  secs?: number;
  isGenerating?: boolean;
  error?: string;
  [key: string]: unknown;
}

/**
 * **音频素材**（2026-10-08）：放一段本机音频（歌、人声、音效），接到视频节点上当 H3 的参考音频 `<Audio N>`。
 *
 * 和「试音色」（`audio`，TTS 念出来的）是两回事：这个节点不生成东西，只放文件，和图片素材节点一样。
 * 「上传资源」节点选了音频文件会自动变成它。
 */
export interface AudioClipNodeData {
  label?: string;
  /** 本机绝对路径 */
  audioPath?: string;
  /** 时长（秒），后端 ffprobe 读的。读不出是空 */
  secs?: number;
  referenceLoading?: boolean;
  error?: string;
  [key: string]: unknown;
}

export interface UpscaleNodeData {
  /** 放大的是哪个文件。上游换了，这里对不上就该重做 */
  sourcePath?: string;
  /** 正在放大的那个文件（任务跑完才挪到 sourcePath —— 失败了不能把旧结果标成新的） */
  requestedSourcePath?: string;
  /** 结果的本机路径。图还是视频看扩展名 */
  outputUrl?: string;
  isGenerating?: boolean;
  error?: string;
  notes?: string[];
  [key: string]: unknown;
}

export interface PromptNodeData {
  label: string;
  prompt: string;
  scale?: string;
  cameraMovement?: string;
  tags?: string[];
  [key: string]: unknown;
}

export interface CharacterNodeData {
  characterId: string;
  name: string;
  avatarUrl?: string;
  promptTrigger: string;
  appearancePrompt: string;
  [key: string]: unknown;
}

export interface ImageGenNodeData {
  /** 模板说明：第几步、做什么、怎么验收。**运行不清、存模板保留**，和运行提示 `notes` 分开（见 utils/nodeGuide.ts） */
  guide?: string[];
  loras?: { name: string; strength: number }[];
  injectTriggers?: boolean;
  references?: CanvasReference[];
  engine: string;
  notes?: string[];
  width: number;
  height: number;
  ratio?: string;
  resolution?: string;
  seed: number;
  prompt?: string;
  imageCount?: number;
  outputImages?: string[];
  lastExecutedSignature?: string;
  isGenerating?: boolean;
  progress?: number;
  outputImageUrl?: string;
  error?: string;
  isExpanded?: boolean;
  /** 这个节点是工具产物（抠图 / 超分 / 四视图 / 参考表），源图是连进来的第一张图。见 `utils/imageTools.ts` */
  imageTool?: "remove_bg" | "upscale" | "quad_view" | "entity_sheet";
  /**
   * 「局部修改」时记下的源图。**出完之后拿它和结果并排比。**
   *
   * 指令式编辑最典型的失败是「它把整张图重画了一遍，脸也跟着变了」——
   * 而这不报错，只看结果那一张根本看不出来。
   */
  editSourcePath?: string;
  /** 「照这张图的姿势出」开关。见 `inferImageMode` */
  poseFromReference?: boolean;
  /** 「场景四宫格」开关 */
  sceneSheet?: boolean;
  [key: string]: unknown;
}

export interface VideoGenNodeData {
  /** 模板说明：第几步、做什么、怎么验收。**运行不清、存模板保留**，和运行提示 `notes` 分开（见 utils/nodeGuide.ts） */
  guide?: string[];
  loras?: { name: string; strength: number }[];
  injectTriggers?: boolean;
  references?: CanvasReference[];
  mode?: "auto" | "t2v" | "i2v" | "fl2v" | "r2v";
  /**
   * 长镜头的窗口提示词，**一个窗口一条**。≥2 条时这个节点就是一条长镜头：
   * 一次提交、出一个 MP4。见 `utils/longTakeWindows`。
   *
   * 窗口时长共用节点的 `durationSecs`（H3 的窗口时长只有一个值），
   * 所以「窗口必须等长」这种错误在这个形状下构造不出来。
   *
   * **产物落在 `outputVideoUrl` 上。** 以前是窗口链、产物记在链头的
   * `longTakeUrl`，因为那时候「链头节点自己的输出」另有含义；
   * 收进一个节点之后没有这层歧义了，而走 `outputVideoUrl` 才能让
   * 播放器、超分、收进分镜这三样直接可用 —— 它们本来就只认这个字段。
   */
  longTakeWindows?: string[];
  /** 长镜头正在跑 */
  longTakeRunning?: boolean;
  engine: string;
  notes?: string[];
  durationSecs: number;
  width?: number;
  height?: number;
  ratio?: string;
  resolution?: string;
  prompt?: string;
  cameraMovement?: string;
  videoCount?: number;
  generateAudio?: boolean;
  /** H3 加速档（`lightx2v` / `standard` / `staged` / `dense`）。空 = 按链路默认（首帧 LightX2V、参考 Larry 6、文生 Larry 4），见 `utils/h3Quality` */
  h3Quality?: string;
  /** H3 出片前按官方格式整理提示词（只动格式、不改正文；默认关）。见 Rust `h3_spec::canvas_official` */
  h3OfficialFormat?: boolean;
  outputVideos?: string[];
  lastExecutedSignature?: string;
  isGenerating?: boolean;
  progress?: number;
  /**
   * **这个节点上一次实际发出去的那一份。**
   *
   * 不是 `prompt` —— 那是编辑框里的内容，和真正发出去的差好几步：
   * 上游聚合、`@提及`编译、`<Picture N>` 映射、LoRA 激活词。
   * 回填成 take 的时候记 `prompt` 等于存了一条**假记录**，
   * 而回看历史时唯一想知道的就是「当时到底发了什么」。
   *
   * 由 `dagExecutor` 在提交那一刻写，**没跑过就没有** ——
   * 上传进来的视频不该凭空多出一份提示词。
   */
  lastRun?: {
    prompt: string;
    engine: string;
    /** 按顺序的参考图路径。顺序有意义（场景在前、人物在后） */
    images: string[];
    /** 参考音频路径（`<Audio 1..>`），没接就不写 */
    audios?: string[];
    width: number;
    height: number;
    durationSecs: number;
    at: number;
  };
  outputVideoUrl?: string;
  error?: string;
  isExpanded?: boolean;
  /** 这个节点的视频是用户上传的素材，不是生成的 —— 没有提示词、不该重跑 */
  uploadedVideo?: boolean;
  /** 动作迁移：从深度视频第几秒开始取（一次最多 5 秒） */
  motionStartSecs?: number;
  /**
   * 动作迁移走哪条路。2026-09-14 同题实测：
   * `ltx` 动作逐帧对得上、快速动作时脸会糊，约 2~3 分钟；
   * `h3` 大部分关键姿势对得上、脸清楚，约 17 分钟。
   * `h3_ref` 原片复刻 · 快（2026-09-24 Kotte 8 段跑通的那一套）：原视频直接当参考、走 WeeTodd，8.5 秒约 65 分钟。
   * 背景、光、运镜、切镜都照原片，人换成人物图。原视频直接连进来，不用深度节点。
   * `h3_native` 是原生 H3 那档（「H3 动作迁移 2.0」原配方），2026-09-24 连模型一起删了 ——
   * 老画布里存着的按 `h3_ref` 跑（见 `effectiveMotionEngine`）。
   */
  motionEngine?: "ltx" | "h3" | "h3_native" | "h3_ref";
  /** 动作迁移用哪种控制信号：轮廓线（canny，默认，实测最强）或深度（大脸特写回落用）。只对「动作精准」生效 */
  motionControl?: "canny" | "depth";
  /**
   * 动作精准那一档保留原视频背景：先用 Qwen 编辑把原视频那一帧里的人换成人物图，再当首帧。
   * 实测墙面、窗影、地板全程保留，人第 20 帧左右走到原舞者位置；多约 5~6 分钟。
   */
  motionKeepBackground?: boolean;
  /** 动作迁移时自动对位出来的首帧，给人看模型收到的第一帧是什么 */
  motionFirstFrame?: string;
  /** 这个节点是工具产物：`interpolate` = 上游视频补帧 ×2 */
  videoTool?: "interpolate" | "upscale";
  [key: string]: unknown;
}


export type PromptNode = Node<PromptNodeData, "prompt">;
export type ShotNode = Node<ShotNodeData, "shot">;
export type ShotContextNode = Node<ShotContextNodeData, "shotContext">;
export type SkillNode = Node<SkillNodeData, "skill">;
export type PanoramaNode = Node<PanoramaNodeData, "panorama">;
export type PanoViewerNode = Node<PanoViewerNodeData, "panoViewer">;
export type DirectorWorldNode = Node<DirectorWorldNodeData, "directorWorld">;
export type CharacterNode = Node<CharacterNodeData, "character">;
export type ImageGenNode = Node<ImageGenNodeData, "imageGen">;
export type VideoGenNode = Node<VideoGenNodeData, "videoGen">;
export type DepthCaptureNode = Node<DepthCaptureNodeData, "depthCapture">;
export type LoraNode = Node<LoraNodeData, "lora">;
export type AudioNode = Node<AudioNodeData, "audio">;
export type AudioClipNode = Node<AudioClipNodeData, "audioClip">;
export type UpscaleNode = Node<UpscaleNodeData, "upscale">;
export type GroupNode = Node<GroupNodeData, "group">;

export type CanvasNode =
  | PromptNode
  | ShotNode
  | CharacterNode
  | ImageGenNode
  | VideoGenNode
  | ShotContextNode
  | SkillNode
  | PanoramaNode
  | PanoViewerNode
  | DirectorWorldNode
  | DepthCaptureNode
  | LoraNode
  | AudioNode
  | AudioClipNode
  | UpscaleNode
  | GroupNode;
export type CanvasEdge = Edge;

/** 3DGS 文件的坐标系约定。`opencv_camera` = SHARP 的原相机坐标 */
export type SplatConvention = "opencv_camera";
