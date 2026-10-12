import { create } from "zustand";

import * as voiceApi from "../api/voice";
import { useProject } from "./project";
import { mouth } from "../avatar/mouth";
import { useToolLog } from "./toolLog";
import type {
  MemoryStats,
  TtsVoiceInfo,
  VoiceConfig,
  VoiceEvent,
  VoiceState,
  VoiceStatus,
} from "../api/voice";
import { useChat } from "./chat";

/**
 * 语音会话状态。
 *
 * 单独一个 store 而不是塞进 `chat`：语音是一个长期会话（一次启动、多轮往复、
 * 随时打断），而 chat 是「每轮一条流」。生命周期不同，混在一起会让两套状态
 * 互相纠缠。两者通过 `pushVoiceTurn` 交汇 —— 语音识别出的话与助手回答会
 * 写进同一份对话历史，界面上看起来就是一个连续的会话。
 */

/** 语音会话在跑、但还没进入具体子状态时的兜底 */
const ACTIVE_STATES: VoiceState[] = [
  "listening",
  "transcribing",
  "thinking",
  "speaking",
];

export interface EmotionState {
  affection: number;
  mood: number;
  energy: number;
  familiarity: "stranger" | "acquainted" | "familiar" | "close";
  irritation: number;
}

export interface VoiceStore {
  /** 会话是否已启动（用户点了麦克风按钮） */
  running: boolean;
  /** 当前角色的标记（顶部状态那一行显示）。null = 没有扮演角色 */
  skillMarker: string | null;
  /** 正在启动/停止中，用来禁用按钮避免连点 */
  busy: boolean;
  state: VoiceState;
  /** 麦克风电平 0..1，驱动粒子核心与电平条 */
  level: number;
  /** AI 输出电平（输入框上方那条谱线） */
  outputLevel: number;
  /** 情感状态 */
  emotion: EmotionState | null;
  /** 当前触发的动作 */
  primeEmotion: () => Promise<void>;
  action: string | null;
  /** 每收到一个动作事件就 +1。**同一个动作连着做两次时，只有名字是不够的** ——
   *  渲染层靠"名字变了没有"来判断要不要重播，名字没变就永远不播第二次 */
  actionSeq: number;
  /** 是否找到了识别模型。false 时点按钮会失败，UI 该提前提示 */
  sttReady: boolean;
  modelPath: string;
  config: VoiceConfig | null;
  error: string | null;
  /** 最近一次被打断的时间戳，用来给界面一个瞬时反馈 */
  interruptedAt: number | null;

  ttsVoices: TtsVoiceInfo[];
  inputDevices: string[];
  memory: MemoryStats | null;

  init: () => Promise<void>;
  toggle: () => Promise<void>;
  interrupt: () => Promise<void>;
  refreshStatus: () => Promise<void>;
  loadOptions: () => Promise<void>;
  refreshMemory: () => Promise<void>;
  applyConfig: (patch: Partial<VoiceConfig>) => Promise<void>;
  dismissError: () => void;
}

/**
 * 全局订阅只挂一次。存 Promise 而不是解绑函数：`listen` 是异步的，
 * 存解绑函数就要先 await，而 StrictMode 的两次调用会在 await 期间都通过检查。
 *
 * **守卫必须挂在 window 上，不能用模块级变量。**
 * 模块级变量在 Vite 热重载时会被重置成 null —— 于是又订阅一次，
 * 而旧模块那个监听器还活着。表现是每条消息成对重复、且"生成中"卡住
 * （结束事件只能匹配到其中一条 turn）。改一次 store 就多攒一个监听器。
 *
 * **但只挂 window 会带来另一个坑，必须配合下面的 hot.dispose 一起用。**
 * window 上的守卫在热重载后仍然存在，于是新模块看到"已经订阅过了"就不再订阅 ——
 * 而实际在收事件的是**旧模块那个 handler**。表现是：改了事件处理逻辑、
 * 界面也刷新了，但新逻辑根本不执行。
 *
 * 实测吃过一次：加了对口型的 `words` / `sentence_start` 两个分支，热重载之后
 * 嘴一直不动，一路去查 TTS 引擎和时间戳，而 Rust 侧一切正常 ——
 * 事件压根没进新 handler。
 */
const SUB_KEY = "__animaVoiceSubscription";

function getSubscription(): Promise<() => void> | null {
  return (window as unknown as Record<string, Promise<() => void> | null>)[SUB_KEY] ?? null;
}
function setSubscription(p: Promise<() => void> | null): void {
  (window as unknown as Record<string, Promise<() => void> | null>)[SUB_KEY] = p;
}

// 这个模块要被热重载替换掉之前，先把旧订阅退掉并清空守卫，
// 让新模块能重新订阅。少了这一步，改动后的事件处理逻辑永远不生效
// （详见上面 SUB_KEY 的注释）。生产构建里 import.meta.hot 是 undefined
if (import.meta.hot) {
  import.meta.hot.dispose(() => {
    const pending = getSubscription();
    setSubscription(null);
    void pending?.then((unlisten) => unlisten()).catch(() => {});
  });
}

export const useVoice = create<VoiceStore>((set, get) => {
  /**
   * 语音这一轮的助手气泡 id。
   *
   * 语音的 assistant_delta 是流式来的，需要往同一个气泡里累积；
   * turn_complete / interrupted 时结束这个气泡。
   */
  let assistantTurnId: string | null = null;

  const handle = (event: VoiceEvent) => {
    const chat = useChat.getState();

    switch (event.kind) {
      case "state":
        // **running 要双向跟着状态走。**
        //
        // 只在活跃时置 true 的话，另一个窗口关掉麦克风之后这边永远显示
        // "开着" —— 每个窗口是独立的 webview、各有一份 store，
        // 它们之间唯一的同步渠道就是这条事件流。
        set({ state: event.state, running: ACTIVE_STATES.includes(event.state) });
        break;

      case "emotion":
        set({
          emotion: {
            affection: event.affection,
            mood: event.mood,
            energy: event.energy,
            familiarity: event.familiarity,
            irritation: event.irritation,
          },
        });
        break;

      case "action":
        // 「桌宠模式」这个概念没有了 —— 角色层本身就是桌宠，一直在。
        // 这个动作现在的含义是"把窗口叫出来给我看"
        if (event.action === "exit_pet_mode") {
          import("../api/voice").then(({ openPanel }) => {
            void openPanel("chat").catch(() => {});
          });
          break;
        }
        // 名字 + 序号一起更新。序号是"这是第几次要求"，
        // 用户连着说两次「挥挥手」时靠它把第二次也播出来
        set((s) => ({ action: event.action, actionSeq: s.actionSeq + 1 }));
        break;

      case "output_level":
        set({ outputLevel: event.rms });
        break;

      case "level":
        set({ level: event.rms });
        break;

      case "transcript":
        // 用户说完一句：写进对话历史，并开一个空的助手气泡等着填
        chat.pushVoiceTurn("user", event.text);
        assistantTurnId = chat.pushVoiceTurn("assistant", "", true);
        break;

      case "assistant_delta":
        if (!assistantTurnId) {
          // 没有 transcript 就先来了 delta（理论上不该发生），补一个气泡
          assistantTurnId = chat.pushVoiceTurn("assistant", "", true);
        }
        chat.appendVoiceDelta(assistantTurnId, event.delta, "content");
        break;

      case "reasoning_delta":
        if (assistantTurnId) {
          chat.appendVoiceDelta(assistantTurnId, event.delta, "reasoning");
        }
        break;

      case "words":
        // 时间线先到，t=0 要等 sentence_start —— 见 avatar/mouth.ts
        mouth.stage(event.id, event.visemes);
        break;

      // 音频驱动的口型：play_audio / 电台。
      // 有文本时间线时 MouthTrack 会自己忽略它 —— 那条更准
      case "audio_viseme":
        mouth.audio(event.viseme, event.weight);
        break;

      case "sentence_start":
        mouth.start(event.id);
        break;

      case "interrupted":
        // 打断时必须闭嘴。少了这一句，被掐掉的那一句的嘴型会继续播完 ——
        // 声音没了嘴还在动
        mouth.stop();
        if (assistantTurnId) {
          chat.finishVoiceTurn(assistantTurnId, { cancelled: true });
          assistantTurnId = null;
        }
        set({ interruptedAt: Date.now() });
        break;

      case "turn_complete":
        mouth.stop();
        if (assistantTurnId) {
          chat.finishVoiceTurn(assistantTurnId, {});
          assistantTurnId = null;
        }
        break;
      case "tool":
        // 进 MCP 面板的执行日志 —— 那一栏本来就是为这个设计的。
        // 语音链路原先根本没往里写，所以那里永远是"暂无记录"
        useToolLog.getState().record({
          key: event.id,
          name: event.name,
          status: event.state === "blocked" ? "rejected" : event.state,
          detail: event.detail ?? undefined,
        });
        break;

      case "skill":
        // 顶部状态那一行显示它。切换由模型（语音）或设置面板触发，
        // 两条路都走同一个事件，界面只认这一个来源
        set({ skillMarker: event.marker });
        break;

      case "error":
        if (assistantTurnId) {
          chat.finishVoiceTurn(assistantTurnId, { error: event.message });
          assistantTurnId = null;
        }
        set({ error: event.message });
        break;

      // words 事件（对口型用）阶段三再消费
      default:
        break;
    }
  };

  return {
    running: false,
    skillMarker: null,
    busy: false,
    state: "idle",
    level: 0,
    outputLevel: 0,
    emotion: null,
    action: null,
    actionSeq: 0,
    sttReady: false,
    modelPath: "",
    config: null,
    error: null,
    interruptedAt: null,
    ttsVoices: [],
    inputDevices: [],
    memory: null,

    async init() {
      // 同步占位必须在任何 await 之前，否则 StrictMode 下会注册两个监听器
      if (!getSubscription()) {
        setSubscription(voiceApi.subscribeVoice(handle));
      }
      const pending = getSubscription();

      await get().refreshStatus();

      try {
        await pending;
      } catch (err) {
        setSubscription(null);
        set({ error: `语音事件订阅失败，将收不到语音状态：${err}` });
      }
    },

    async refreshStatus() {
      try {
        const status: VoiceStatus = await voiceApi.voiceStatus();
        set({
          state: status.state,
          sttReady: status.stt_ready,
          modelPath: status.model_path,
          config: status.config,
          running: ACTIVE_STATES.includes(status.state),
        });
      } catch (err) {
        // 非 Tauri 环境（lab.html）会失败，不当成错误刷给用户
        console.debug("读取语音状态失败", err);
      }
    },

    /** 开机拉一次当前情感。事件只在会话启动/每轮结束时来，冷启动时前端是空的 */
    async primeEmotion() {
      try {
        const emotion = await voiceApi.getEmotion();
        if (emotion) set({ emotion });
      } catch {
        // 记忆没就绪就没有情感可显示，不是错误
      }
    },

    async toggle() {
      if (get().busy) return;
      set({ busy: true, error: null });
      try {
        if (get().running) {
          const status = await voiceApi.voiceStop();
          set({ running: false, state: status.state, level: 0, outputLevel: 0 });
        } else {
          const projectState = useProject.getState();
          let sessionId = projectState.activeSessionId;
          const projectId = projectState.activeProjectId;
          
          if (!sessionId) {
            await projectState.createSession(projectId);
            sessionId = useProject.getState().activeSessionId;
          }

          const status = await voiceApi.voiceStart({ sessionId, projectId });
          set({
            running: true,
            state: status.state,
            sttReady: status.stt_ready,
            modelPath: status.model_path,
            config: status.config,
          });
        }
      } catch (err) {
        // 最常见的失败是「找不到识别模型」，错误文案里已经说明放哪
        set({ error: String(err), running: false });
      } finally {
        set({ busy: false });
      }
    },

    async interrupt() {
      try {
        await voiceApi.voiceInterrupt();
      } catch (err) {
        set({ error: String(err) });
      }
    },

    async loadOptions() {
      // 两个都是「拿不到就算了」的增强信息，各自独立失败
      try {
        set({ inputDevices: await voiceApi.listInputDevices() });
      } catch (err) {
        console.debug("读取麦克风列表失败", err);
      }
      try {
        set({ ttsVoices: await voiceApi.listTtsVoices() });
      } catch (err) {
        console.debug("读取音色列表失败", err);
      }
    },

    async refreshMemory() {
      try {
        set({ memory: await voiceApi.memoryStats() });
      } catch (err) {
        console.debug("读取记忆统计失败", err);
      }
    },

    async applyConfig(patch) {
      const current = get().config;
      if (!current) return;
      const next: VoiceConfig = {
        ...current,
        ...patch,
        tts: { ...current.tts, ...(patch.tts ?? {}) },
        vad: { ...current.vad, ...(patch.vad ?? {}) },
      };
      try {
        const status = await voiceApi.setVoiceConfig(next);
        set({ config: status.config });
      } catch (err) {
        set({ error: String(err) });
      }
    },

    dismissError() {
      set({ error: null });
    },
  };
});
