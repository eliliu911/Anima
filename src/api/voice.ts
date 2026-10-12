/**
 * 语音管线与记忆系统的 Tauri 绑定。
 *
 * 与 `api/index.ts`（文字对话）分开：两者生命周期不同 ——
 * 文字是「一问一答，每轮一条流」，语音是「一个长期会话，中间随时打断」。
 *
 * 契约详见 docs/VOICE_API.md。
 */
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export const VOICE_EVENT = "anima://voice";

export type VoiceState =
  | "idle"
  | "listening"
  | "transcribing"
  | "thinking"
  | "speaking";

export interface WordBoundary {
  offset_ms: number;
  duration_ms: number;
  text: string;
}

/** Oculus Viseme 那 15 个口型。字符串就是模型里 morph target 的名字 */
export type Viseme =
  | "sil" | "PP" | "FF" | "TH" | "DD" | "kk" | "CH"
  | "SS" | "nn" | "RR" | "aa" | "E" | "I" | "O" | "U";

/** 嘴型时间线上的一个关键帧。at_ms 相对这一句**开始出声**的时刻 */
export interface VisemeKey {
  at_ms: number;
  viseme: Viseme;
  weight: number;
}

/** 一个可用的面容模型（3D 或 Live2D） */
export interface AvatarModel {
  /** 相对模型目录的路径，可能带子目录（`alice/head.glb` 或 `live2d/Hiyori/Hiyori.model3.json`）。分隔符固定是 `/` */
  id: string;
  /** 显示名称 */
  name: string;
  /** 所在子目录，顶层的为空。界面按它分组 */
  folder: string;
  bytes: number;
  /** 模型格式分类：3d (glb/vrm)、live2d (model3.json) 或 neural (肖像图/视频) */
  kind?: "3d" | "live2d" | "neural";
}

/** Rust 侧 VoiceEvent 的镜像，用 kind 区分 */
export type VoiceEvent =
  | { kind: "state"; state: VoiceState }
  | { kind: "level"; rms: number }
  /** TTS 播放电平。与 level 分开：一条是"我说了多大声"，一条是"AI 说了多大声" */
  | { kind: "output_level"; rms: number }
  | { kind: "transcript"; text: string }
  | { kind: "assistant_delta"; delta: string }
  | { kind: "reasoning_delta"; delta: string }
  /**
   * 一句话的词边界与嘴型时间线。
   *
   * **`at_ms` 的 t=0 不是收到这条事件的时刻**，而是随后 `sentence_start`
   * 到达的时刻 —— 句子是排队播的，合成完的时候前一句往往还在播。
   * 靠 id 配对。
   */
  | { kind: "words"; id: number; words: WordBoundary[]; visemes: VisemeKey[] }
  /** 这个 id 的句子开始出声了。嘴型时间线在这一刻起算 */
  | { kind: "sentence_start"; id: number }
  /**
   * 音频驱动的口型（从正在播的声音里实时算出来的）。
   *
   * **只有没走文本时间线的音频才会有** —— `play_audio`、电台。
   * 那些场景拿不到文本，`words` 那条时间线不存在，嘴以前是完全不动的。
   * 精度低于时间线那条（只分得出元音），所以两者不会同时出现。
   */
  | { kind: "audio_viseme"; viseme: Viseme; weight: number }
  /** 情感轴变化。只在轮次边界发，不是每帧 */
  | {
      kind: "emotion";
      affection: number;
      mood: number;
      energy: number;
      familiarity: "stranger" | "acquainted" | "familiar" | "close";
      /** 焦躁强度 0..1（低心情+高唤醒推出，不是独立轴） */
      irritation: number;
    }
  | { kind: "interrupted" }
  | { kind: "turn_complete" }
  | { kind: "skill"; marker: string | null }
  /** 触发角色动作 */
  | { kind: "action"; action: string }
  /**
   * 这一轮调了个工具，以及它的下场。
   *
   * **语音模式下必须显示它。** 少了这条，模型说"终端已打开"而实际没打开时，
   * 用户无从判断它是调了但失败、还是压根没调 —— 两种情况下的回答一模一样。
   */
  | {
      kind: "tool";
      id: string;
      name: string;
      state: "running" | "ok" | "failed" | "blocked";
      detail: string | null;
    }
  | { kind: "error"; message: string };

export interface VoiceConfig {
  input_device: string | null;
  vad: {
    energy_threshold: number;
    speech_frames: number;
    silence_frames: number;
    playback_threshold_scale: number;
  };
  tts: {
    voice: string;
    rate: string;
    pitch: string;
    volume: string;
  };
  tier: "realtime" | "background";
  disable_thinking: boolean;
  sentence_min_chars: number;
}

export interface VoiceStatus {
  state: VoiceState;
  /** 是否已找到可用的识别模型；false 时 voice_start 会失败 */
  stt_ready: boolean;
  model_path: string;
  config: VoiceConfig;
}

export interface TtsVoiceInfo {
  name: string;
  /** 这个才是 config.tts.voice 要填的值 */
  short_name: string;
  gender: string;
  locale: string;
}

export interface MemoryStats {
  sessions: number;
  messages: number;
  facts: number;
  dimensions: number;
}

/**
 * 启动语音会话。
 *
 * 首次调用会加载 Whisper 模型（约 200ms）并申请麦克风权限 ——
 * 系统会弹一次授权框，这是必需且只弹一次的。
 */
export function voiceStart(args?: { sessionId?: string | null; projectId?: string | null }): Promise<VoiceStatus> {
  return invoke("voice_start", { 
    sessionId: args?.sessionId ?? null, 
    projectId: args?.projectId ?? null 
  });
}

export function voiceStop(): Promise<VoiceStatus> {
  return invoke("voice_stop");
}

/** 手动打断当前回答。与语音打断走同一条路径，行为一致。 */
export function voiceInterrupt(): Promise<boolean> {
  return invoke("voice_interrupt");
}

export function voiceStatus(): Promise<VoiceStatus> {
  return invoke("voice_status");
}

/** 设备与 VAD 参数在启动时固化，改完需要 stop→start 才生效 */
export function setVoiceConfig(config: VoiceConfig): Promise<VoiceStatus> {
  return invoke("set_voice_config", { config });
}

export function listInputDevices(): Promise<string[]> {
  return invoke("list_input_devices");
}

/** 从微软服务端实时拉取，不要写死 */
export function listTtsVoices(): Promise<TtsVoiceInfo[]> {
  return invoke("list_tts_voices");
}

/** 一个引擎现在能不能用。见 Rust 侧 probe_tts_engine 的说明 */
export interface EngineProbe {
  id: string;
  reachable: boolean;
  voices: number;
  error: string | null;
}

/** 探活：拉音色列表（唯一一个所有适配器都实现、且没有副作用的调用） */
export function probeTtsEngine(engineId: string): Promise<EngineProbe> {
  return invoke("probe_tts_engine", { engineId });
}

/** 返回 MP3 字节，由前端自己播放，不占用管线的播放队列 */
export function previewTtsVoice(
  voice: string,
  text?: string,
): Promise<number[]> {
  return invoke("preview_tts_voice", { voice, text: text ?? null });
}

/** null 表示记忆系统未启用或嵌入后端连不上 */
export function memoryStats(): Promise<MemoryStats | null> {
  return invoke("memory_stats");
}

export function memoryClearFacts(): Promise<boolean> {
  return invoke("memory_clear_facts");
}

/**
 * 订阅语音事件。
 *
 * 必须在应用启动时就挂上，不要等 voiceStart 返回再订阅 —— `listen` 是异步注册的，
 * 中间那个窗口的事件会漏掉。文字对话那条通道踩过这个坑（见 DECISIONS.md）。
 */
export function subscribeVoice(
  onEvent: (event: VoiceEvent) => void,
): Promise<() => void> {
  return listen<VoiceEvent>(VOICE_EVENT, ({ payload }) => onEvent(payload));
}

/**
 * 把 preview_tts_voice 返回的字节播出来。
 *
 * **MIME 不能写死 audio/mpeg**：Edge 返回 mp3，而 Kokoro / CosyVoice 这些
 * 本地引擎返回 wav。写死之后只能靠浏览器嗅探，不同 WebView 的行为并不一致。
 * 按前四个字节判断：RIFF = wav，其余按 mp3。
 */
export function playMp3Bytes(bytes: number[]): HTMLAudioElement {
  const data = new Uint8Array(bytes);
  const isWav =
    data.length > 4 &&
    data[0] === 0x52 && data[1] === 0x49 && data[2] === 0x46 && data[3] === 0x46;
  const blob = new Blob([data], { type: isWav ? "audio/wav" : "audio/mpeg" });
  const url = URL.createObjectURL(blob);
  const audio = new Audio(url);
  // 播完就释放，否则每次试听都漏一个 blob URL
  audio.addEventListener("ended", () => URL.revokeObjectURL(url));
  void audio.play();
  return audio;
}


/* ─── TTS 引擎切换 ───────────────────────────────────────────
   与 LLM 网关同一个模式：上层只认 trait，换引擎不动语音管线。

   要点：**只有 Edge 会返回真实的词边界时间戳**。本地引擎的时间轴是按
   字符权重估算的（见 Rust 侧 estimate_word_boundaries），够驱动嘴型，
   但不是测量值 —— 阶段三做精细对口型时要记得这个差别。 */

export type TtsEngineKind =
  | "edge"
  | "openai_compatible"
  | "gpt_sovits"
  | "cosy_voice2"
  | "index_tts2"
  // F5-TTS 的线格式就是 OpenAI 兼容那套，Rust 侧共用同一个适配器。
  // 单独一个值只为了在设置面板里有自己的说明和默认端口
  | "f5_tts";

export interface TtsEngineConfig {
  id: string;
  label: string;
  kind: TtsEngineKind;
  base_url: string;
  /**
   * **这个引擎自己**记的音色。音色名每家一套（Edge 是
   * `zh-CN-XiaoxiaoNeural`，Kokoro 是 `zf_xiaobei`，SoVITS 是音色库路径），
   * 共用一份的结果是换引擎之后把上一家的名字丢给新引擎 —— 各种 404
   */
  voice?: string | null;
  api_key_secret?: string | null;
  /** GPT-SoVITS 用：音色由参考音频决定，不是选一个音色名 */
  ref_audio_path?: string | null;
  ref_text?: string | null;
  /** 服务返回裸 PCM 时的采样率。配错不报错，只会音调偏移 */
  pcm_sample_rate?: number | null;
  /** 端点路径。IndexTTS 各 fork 不统一，所以可配 */
  endpoint_path?: string | null;
}

export interface TtsEngineView {
  engines: TtsEngineConfig[];
  selected: string;
}

export const ttsEngineApi = {
  list: () => invoke<TtsEngineView>("tts_engines"),
  /** 切换引擎。本地引擎配错会在这里就报错，而不是等到说话时 */
  select: (engineId: string) => invoke<void>("set_tts_engine", { engineId }),
  update: (engine: TtsEngineConfig) =>
    invoke<void>("update_tts_engine", { engine }),
};


/** 一个技能（角色）。内容来自 skills/<id>/SKILL.md，改完不用重启 */
export interface Skill {
  id: string;
  name: string;
  /** 顶部状态那一行显示的标记，两到四个字 */
  marker: string;
  description: string;
  /**
   * 中文界面下的显示文案，空则回落到英文那三个。
   *
   * **放在 SKILL.md 的 frontmatter 里而不是应用的语言包里** —— 技能是用户
   * 可以自己新增和编辑的文件，别人加一个自己的技能时应该也能起中文名，
   * 而不是必须去改 i18n 资源。
   */
  name_zh?: string;
  marker_zh?: string;
  description_zh?: string;
  prefer_tier: "realtime" | "background" | null;
  thinking: boolean | null;
  /** 工具白名单。空数组 = 不限制 */
  tools: string[];
  /** 内容来源与署名（上游许可要求保留） */
  source: string;
  kind?: "persona" | "task";
  surface?: "companion" | "coding" | "both";
  prompt?: string;
  /**
   * 这个技能是从哪一层找到的。
   *
   * `project` = `<工作区>/.anima/skills/`，跟着仓库走；`global` = 全局。
   * **同 id 时项目层覆盖全局层**，所以这个标记不是装饰 ——
   * 「我改了全局那份却没生效」只有显示来源才解释得清。
   */
  scope: "project" | "global";
}

export interface SaveSkillArgs {
  id: string;
  name: string;
  marker?: string;
  description?: string;
  prefer_tier?: "realtime" | "background" | null;
  thinking?: boolean | null;
  tools?: string[];
  source?: string;
  kind?: "persona" | "task";
  surface?: "companion" | "coding" | "both";
  prompt?: string;
  is_project_level?: boolean;
  workspace_path?: string | null;
}

export interface SkillView {
  skills: Skill[];
  /** 当前生效的技能 id 列表 */
  active: string[];
  /**
   * **没能加载的那些**：文件在磁盘上、看着像装好了，而解析失败了。
   *
   * 以前这件事只有一行 WARN 日志 —— 实测 `kind: spec`（别的技能体系的
   * 取值）会让整份 SKILL.md 被静默跳过，而界面上完全看不出来。
   */
  broken?: { id: string; error: string }[];
}

/** `workspacePath` 决定能不能看到项目级技能（`.anima/skills/`）。 */
export function listSkills(workspacePath?: string | null): Promise<SkillView> {
  return invoke("list_skills", { workspacePath: workspacePath ?? null });
}

export function saveSkill(args: SaveSkillArgs): Promise<Skill> {
  return invoke("save_skill", { args });
}

export function deleteSkill(id: string, workspacePath?: string | null): Promise<void> {
  return invoke("delete_skill", { id, workspacePath: workspacePath ?? null });
}

export interface SlashEntry {
  /** 敲进去的名字，不含斜杠 */
  id: string;
  title: string;
  description: string;
  /** skill = 展开成提示词发出去；builtin = 前端执行一个本地动作 */
  kind: "skill" | "builtin";
  takes_args: boolean;
  /** builtin 的动作标识，前端 switch 它 */
  action: string | null;
}

/** `/` 弹层的候选列表：任务技能 + 内置动作。身份型技能不在这里。 */
export function listSlashCommands(workspacePath?: string | null): Promise<SlashEntry[]> {
  return invoke("list_slash_commands", { workspacePath: workspacePath ?? null });
}

/**
 * 把一个技能命令展开成要发出去的那段文本。
 *
 * **在后端展开**，理由和 PROMPT.md 一样：用户改完 SKILL.md 应该立刻生效，
 * 前端缓存一份的话文件里写的和模型收到的会静默对不上。
 */
export function expandSlashCommand(
  id: string,
  args: string,
  workspacePath?: string | null,
): Promise<string> {
  return invoke("expand_slash_command", { id, args, workspacePath: workspacePath ?? null });
}

export function selectSkills(skills: string[], workspacePath?: string | null): Promise<void> {
  // 保存时也要带工作区：否则界面上能勾项目技能、一保存就报"没有这个技能"
  return invoke("select_skills", { skills, workspacePath: workspacePath ?? null });
}

export interface Character {
  id: string;
  name: string;
  marker: string;
  description: string;
  binding?: CharacterBinding;
}

/** 一个角色绑了什么。全是可选 —— 没绑就跟随全局设置 */
export interface CharacterBinding {
  engine?: string | null;
  voice?: string | null;
  avatar?: string | null;
  /** 空数组 = 明确要求一个技能都不挂；null/缺省 = 跟随全局 */
  skills?: string[] | null;
}

export interface CharacterBindingView {
  binding: CharacterBinding;
  engines: { id: string; label: string }[];
  voices: TtsVoiceInfo[];
  /** 拉音色失败的原因。空列表 + 有原因 = 服务没起来 */
  voices_error: string | null;
  avatars: AvatarModel[];
  skills: { id: string; name: string }[];
}

/** 读绑定 + 所有可选项。`engine` 传了就按它拉音色（界面上刚改过、还没保存时用） */
export function characterBinding(
  id: string,
  engine?: string | null,
): Promise<CharacterBindingView> {
  return invoke("character_binding", { id, engine: engine ?? null });
}

export function setCharacterBinding(id: string, binding: CharacterBinding): Promise<void> {
  return invoke("set_character_binding", { id, binding });
}

/** 这一项的值是谁定的。角色定的就不给在全局面板上改 */
export type Origin = "character" | "default";

export interface PersonaField<T> {
  value: T;
  origin: Origin;
}

export interface ResolvedPersona {
  engine: PersonaField<string>;
  voice: PersonaField<string>;
  avatar: PersonaField<string | null>;
  skills: PersonaField<string[]>;
  /** 当前是不是默认形态。面容和语音**只在默认形态下可编辑** */
  is_default_form: boolean;
}

export interface PersonaView {
  resolved: ResolvedPersona;
  active: string | null;
  characters: Character[];
  engines: TtsEngineConfig[];
  avatars: AvatarModel[];
  skills: { id: string; name: string }[];
}

/**
 * 角色层要不要忽略鼠标事件。判定与滞后在 `avatar/passthrough.ts`，
 * 这里只是把结论送到系统 API
 */
/** 打开（或聚焦）一个面板窗口。位置用上次记住的 */
export function openPanel(label: string): Promise<void> {
  return invoke("open_panel", { label });
}

/** 记住这个面板窗口现在的位置与尺寸 */
export function rememberPanelGeometry(label: string): Promise<void> {
  return invoke("remember_panel_geometry", { label });
}

export function setAvatarPassthrough(ignore: boolean): Promise<void> {
  return invoke("set_avatar_passthrough", { ignore });
}

/** 查询角色层当前是否可见 */
export function getAvatarVisible(): Promise<boolean> {
  return invoke("get_avatar_visible");
}

/** 显示 / 隐藏角色层。托盘的「隐藏她」用这个 */
export function setAvatarVisible(visible: boolean): Promise<void> {
  return invoke("set_avatar_visible", { visible });
}

export function personaView(): Promise<PersonaView> {
  return invoke("persona_view");
}

export function createCharacter(id: string, name: string, description: string): Promise<Character> {
  return invoke("create_character", { id, name, description });
}

export function updateCharacter(id: string, name: string, description: string): Promise<Character> {
  return invoke("update_character", { id, name, description });
}

export function deleteCharacter(id: string): Promise<void> {
  return invoke("delete_character", { id });
}

export function selectCharacter(character: string | null): Promise<void> {
  return invoke("select_character", { character });
}

/** 一条长期记忆 */
export interface MemoryFact {
  id: string;
  /** 事实主体，通常是「用户」 */
  subject: string;
  fact: string;
  /** 来自哪次会话；手动添加的是 "manual" */
  session_id: string;
  /** Unix 秒 */
  created_at: number;
}

/**
 * 浏览长期记忆（按时间倒序）。
 *
 * 和 memoryRecall 是两件事：召回要先猜一个查询词，而"它记住了我什么"
 * 这个问题没有列表就永远回答不了。
 */
export function memoryListFacts(limit = 50, offset = 0): Promise<MemoryFact[]> {
  return invoke("memory_list_facts", { limit, offset });
}

/** 删掉一条。记错了要能删 —— 只有"清空全部"的话没人会真的去用 */
export function memoryForgetFact(id: string): Promise<boolean> {
  return invoke("memory_forget_fact", { id });
}

/** 手动记一条。提取是模型做的，会漏 */
export function memoryAddFact(subject: string, fact: string): Promise<boolean> {
  return invoke("memory_add_fact", { subject, fact });
}

/** 列出 avatars/ 下的模型 */
export async function avatarModels(): Promise<AvatarModel[]> {
  return invoke<AvatarModel[]>("avatar_models");
}

/** 模型目录的绝对路径，界面上要显示给用户 */
export async function avatarModelsDir(): Promise<string> {
  return invoke<string>("avatar_models_dir");
}

/** 在系统访达/文件管理器中直接打开模型素材目录 */
export async function openAvatarModelsDir(): Promise<void> {
  return invoke("open_avatar_models_dir");
}

/**
 * 读出模型目录下某个文件的字节。
 *
 * 也用于取 `.gltf` 引用的 `.bin` 和贴图 —— 那些是相对路径，
 * 拼上模型所在目录再传进来即可（Rust 侧会挡路径穿越和软链接逃逸）。
 *
 * 走 command 而不是前端读文件：前端不持有文件系统权限（见 capabilities/）。
 * Tauri 会把 Vec<u8> 编码成数字数组，这里转回 ArrayBuffer。
 */
export async function avatarModelBytes(id: string): Promise<ArrayBuffer> {
  const bytes = await invoke<number[]>("avatar_model_bytes", { id });
  return new Uint8Array(bytes).buffer;
}

/** 列出 animations/ 下的 .vrma 动作 */
export async function avatarAnimations(): Promise<AvatarModel[]> {
  return invoke<AvatarModel[]>("avatar_animations");
}

/** 读一个 .vrma 的字节，交给前端的 GLTFLoader.parse() */
export async function avatarAnimationBytes(id: string): Promise<ArrayBuffer> {
  const bytes = await invoke<number[]>("avatar_animation_bytes", { id });
  return new Uint8Array(bytes).buffer;
}

/**
 * 往 anima.log 里写一行前端诊断。
 *
 * Tauri 的 webview 没有可读的控制台 —— 前端出问题只能靠界面上显示什么去猜。
 * 「模型加载了但不显示」这类问题画面上什么都没有，绕了好几轮才决定补这条。
 */
/**
 * 当前情感轴。语音没开过时也能读到 —— 情感事件只在会话启动和每轮结束时发，
 * 光靠事件的话应用刚打开时状态行是空的。
 */
export function getEmotion(): Promise<import("../store/voice").EmotionState | null> {
  return invoke("get_emotion");
}

export function logUi(scope: string, message: string): void {
  void invoke("log_ui", { scope, message }).catch(() => {});
}

/** 体检的一条结果。判断逻辑在 Rust 的 `diagnostics.rs`，那里有测试 */
export interface DiagCheck {
  id: string;
  label: string;
  value: string;
  level: "ok" | "warn";
  /** **退回到了什么 / 会怎么失败**，不是修复步骤 */
  detail?: string;
}

/**
 * 读当前实际生效的配置做体检。
 *
 * 会去探活 TTS 引擎，**最长可能等 6 秒**（服务端口开着但不回应的情况），
 * 界面上要给出加载态。
 */
export async function diagnostics(): Promise<DiagCheck[]> {
  return invoke<DiagCheck[]>("diagnostics");
}

export interface SidecarStatus {
  id: string;
  name: string;
  port: number;
  status: "online" | "starting" | "stopped" | "external";
  pid?: number | null;
}

/** 查询所有本地 Python Sidecar 守护进程的状态 */
export async function sidecarStatus(): Promise<SidecarStatus[]> {
  return invoke<SidecarStatus[]>("sidecar_status");
}
