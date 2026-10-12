/**
 * 面容形态的选择与诊断。
 *
 * 单独一个 store 而不是穿 props：选择在设置面板里做，渲染在 App 里，
 * 诊断信息又要从渲染侧回流到设置面板 —— 三方共享，穿 props 会把
 * SettingsModal 的签名和 App 的结构绑在一起。
 *
 * 注意这里**只放低频状态**（换模型是点一下的事）。每帧变化的嘴型权重不在这里，
 * 它在 `avatar/mouth.ts` 的模块级单例上 —— 放进 store 等于每帧一次重渲染。
 */
import { create } from "zustand";
import type { AvatarReport } from "../components/Avatar3D";

/** 存 localStorage：这是纯界面偏好，不值得进 config.json */
const KEY = "anima.faceModel";

export type HologramStyle = "dot" | "wireframe" | "glitch" | "ripple";
export type FaceMode = "abstract" | "concrete" | "hologram" | "live2d" | "neural_stream";

export interface Live2DConfig {
  /** 构图预设：half (半身特写/日常伴侣), full (全身展示), custom (自定义) */
  focusPreset: "half" | "full" | "custom";
  /** 缩放倍率 (0.2 ~ 3.0, 默认 1.0) */
  scale: number;
  /** 水平偏移 (-100 ~ 100 像素/百分比, 默认 0) */
  offsetX: number;
  /** 垂直偏移 (-100 ~ 100 像素/百分比, 默认 0) */
  offsetY: number;
  /** 是否开启光标与视线跟随 */
  mouseTracking: boolean;
  /** 视线灵敏度 (0.2 ~ 2.0, 默认 1.0) */
  gazeMultiplier: number;
  /** 待机眼神微动 / 微跳视 */
  idleMovement: boolean;
  /** 自动眨眼 */
  blink: boolean;
  /** 眨眼模式：auto (模型内置) / force (算法定时强制) */
  blinkMode: "auto" | "force";
  /** 默认待机动作名 */
  idleMotion: string;
  /** 动作播放速度 (0.5 ~ 2.0, 默认 1.0) */
  motionSpeed: number;
  /** 渲染清晰度倍率 (1 ~ 3, 默认 2) */
  renderScale: number;
  /** 帧率上限 (30 / 60 / 0 不限) */
  fpsLimit: 30 | 60 | 0;
  /** 桌面柔和投影特效 */
  dropShadow: boolean;
}

export const DEFAULT_LIVE2D_CONFIG: Live2DConfig = {
  focusPreset: "half",
  scale: 1.0,
  offsetX: 0,
  offsetY: 0,
  mouseTracking: true,
  gazeMultiplier: 1.0,
  idleMovement: true,
  blink: true,
  blinkMode: "auto",
  idleMotion: "",
  motionSpeed: 1.0,
  renderScale: 2,
  fpsLimit: 0,
  dropShadow: true,
};

export interface NeuralConfig {
  /** 推理引擎：musetalk / liveportrait */
  engine: "musetalk" | "liveportrait";
  /** 底模类型：video (视频循环) / image (单张肖像) */
  sourceType: "video" | "image";
  /** 视频/图片源文件路径或预设名 */
  sourcePath: string;
  /** 构图适配模式：contain (完整自适应/不裁切), cover (铺满裁剪), custom (自定义) */
  fitMode: "contain" | "cover" | "custom";
  /** 缩放倍率 (0.2 ~ 3.0, 默认 1.0) */
  scale: number;
  /** 水平偏移 (-100 ~ 100 像素/百分比, 默认 0) */
  offsetX: number;
  /** 垂直偏移 (-100 ~ 100 像素/百分比, 默认 0) */
  offsetY: number;
  /** 帧率 (25 / 30 / 60) */
  fps: 25 | 30 | 60;
  /** 渲染分辨率 (512 / 768 / 1080) */
  resolution: 512 | 768 | 1080;
  /** 绿幕/智能透明背景抠像 */
  transparentBackground: boolean;
  /** 边缘羽化强度 (0.0 ~ 1.0) */
  featherRadius: number;
}

export const DEFAULT_NEURAL_CONFIG: NeuralConfig = {
  engine: "musetalk",
  sourceType: "video",
  sourcePath: "default",
  fitMode: "contain",
  scale: 1.0,
  offsetX: 0,
  offsetY: 0,
  fps: 30,
  resolution: 768,
  transparentBackground: true,
  featherRadius: 0.2,
};

interface FaceState {
  /** 面容形态：abstract (抽象粒子云), concrete (具象 3D), hologram (全息 3D), live2d (2D 纸片人), neural_stream (4D 神经数字人) */
  mode: FaceMode;
  /** 全息形态的具体子风格 */
  hologramStyle: HologramStyle;
  /** 全息形态的主题色 (hex) */
  hologramColor: string;
  /** 选中的模型文件名 (GLB/VRM 或 Live2D model3.json)。为空时 fallback 到内置头/默认 */
  modelId: string | null;
  /** Live2D 专属细化配置 */
  live2dConfig: Live2DConfig;
  /** 神经流数字人专属细化配置 */
  neuralConfig: NeuralConfig;
  /** Avatar3D / Live2D 报回来的诊断。用粒子云时为 null */
  report: AvatarReport | null;
  setMode: (mode: FaceMode) => void;
  setHologramStyle: (style: HologramStyle) => void;
  setHologramColor: (color: string) => void;
  setModelId: (id: string | null) => void;
  updateLive2dConfig: (partial: Partial<Live2DConfig>) => void;
  resetLive2dConfig: () => void;
  updateNeuralConfig: (partial: Partial<NeuralConfig>) => void;
  resetNeuralConfig: () => void;
  setReport: (r: AvatarReport | null) => void;
}

const syncChannel = typeof BroadcastChannel !== "undefined" ? new BroadcastChannel("anima_face_sync") : null;

function broadcastState(state: Partial<FaceState>) {
  syncChannel?.postMessage(state);
}

function loadSavedLive2DConfig(): Live2DConfig {
  try {
    const raw = localStorage.getItem(KEY + ".live2dConfig");
    if (raw) return { ...DEFAULT_LIVE2D_CONFIG, ...JSON.parse(raw) };
  } catch {}
  return { ...DEFAULT_LIVE2D_CONFIG };
}

function loadSavedNeuralConfig(): NeuralConfig {
  try {
    const raw = localStorage.getItem(KEY + ".neuralConfig");
    if (raw) return { ...DEFAULT_NEURAL_CONFIG, ...JSON.parse(raw) };
  } catch {}
  return { ...DEFAULT_NEURAL_CONFIG };
}

export const useFace = create<FaceState>((set, get) => {
  const savedModel = localStorage.getItem(KEY);
  const savedMode = localStorage.getItem(KEY + ".mode") as FaceMode | null;
  const savedStyle = localStorage.getItem(KEY + ".style") as HologramStyle | null;
  const savedColor = localStorage.getItem(KEY + ".color") || "#1a9fff";
  const savedL2d = loadSavedLive2DConfig();
  const savedNeural = loadSavedNeuralConfig();

  // 兼容老版本：如果以前存了 modelId，按文件类型判断 live2d 或 concrete，否则 abstract
  const isLive2d = savedModel?.toLowerCase().endsWith(".model3.json") || savedModel?.toLowerCase().endsWith(".model.json");
  const initialMode = savedMode || (savedModel ? (isLive2d ? "live2d" : "concrete") : "abstract");

  return {
    mode: initialMode,
    hologramStyle: savedStyle || "dot",
    hologramColor: savedColor,
    modelId: savedModel,
    live2dConfig: savedL2d,
    neuralConfig: savedNeural,
    report: null,
    setMode: (mode) => {
      if (get().mode === mode) return;
      localStorage.setItem(KEY + ".mode", mode);
      set({ mode });
      broadcastState({ mode });
    },
    setHologramStyle: (style) => {
      localStorage.setItem(KEY + ".style", style);
      set({ hologramStyle: style });
      broadcastState({ hologramStyle: style });
    },
    setHologramColor: (color) => {
      localStorage.setItem(KEY + ".color", color);
      set({ hologramColor: color });
      broadcastState({ hologramColor: color });
    },
    setModelId: (id) => {
      if (get().modelId === id) return;
      if (id) localStorage.setItem(KEY, id);
      else localStorage.removeItem(KEY);
      set({ modelId: id, report: null });
      broadcastState({ modelId: id });
    },
    updateLive2dConfig: (partial) => {
      const updated = { ...DEFAULT_LIVE2D_CONFIG, ...get().live2dConfig, ...partial };
      localStorage.setItem(KEY + ".live2dConfig", JSON.stringify(updated));
      set({ live2dConfig: updated });
      broadcastState({ live2dConfig: updated });
    },
    resetLive2dConfig: () => {
      localStorage.removeItem(KEY + ".live2dConfig");
      set({ live2dConfig: { ...DEFAULT_LIVE2D_CONFIG } });
      broadcastState({ live2dConfig: { ...DEFAULT_LIVE2D_CONFIG } });
    },
    updateNeuralConfig: (partial) => {
      const updated = { ...DEFAULT_NEURAL_CONFIG, ...get().neuralConfig, ...partial };
      localStorage.setItem(KEY + ".neuralConfig", JSON.stringify(updated));
      set({ neuralConfig: updated });
      broadcastState({ neuralConfig: updated });
    },
    resetNeuralConfig: () => {
      localStorage.removeItem(KEY + ".neuralConfig");
      set({ neuralConfig: { ...DEFAULT_NEURAL_CONFIG } });
      broadcastState({ neuralConfig: { ...DEFAULT_NEURAL_CONFIG } });
    },
    setReport: (report) => {
      set({ report });
      broadcastState({ report });
    },
  };
});

// 跨窗口同步监听：当其他窗口修改面容形态时，当前窗口立即同步
if (syncChannel) {
  syncChannel.onmessage = (event) => {
    if (event.data && typeof event.data === "object") {
      const next = { ...event.data };
      if (next.live2dConfig) {
        next.live2dConfig = { ...DEFAULT_LIVE2D_CONFIG, ...next.live2dConfig };
      }
      if (next.neuralConfig) {
        next.neuralConfig = { ...DEFAULT_NEURAL_CONFIG, ...next.neuralConfig };
      }
      useFace.setState(next);
    }
  };
}

// 跨进程 localStorage 变化监听 (Safari / WebKit 兜底)
if (typeof window !== "undefined") {
  window.addEventListener("storage", (e) => {
    try {
      if (e.key === KEY) {
        useFace.setState({ modelId: e.newValue });
      } else if (e.key === KEY + ".mode") {
        if (e.newValue) useFace.setState({ mode: e.newValue as FaceMode });
      } else if (e.key === KEY + ".style") {
        if (e.newValue) useFace.setState({ hologramStyle: e.newValue as HologramStyle });
      } else if (e.key === KEY + ".color") {
        if (e.newValue) useFace.setState({ hologramColor: e.newValue });
      } else if (e.key === KEY + ".live2dConfig") {
        if (e.newValue) {
          useFace.setState({ live2dConfig: { ...DEFAULT_LIVE2D_CONFIG, ...JSON.parse(e.newValue) } });
        }
      } else if (e.key === KEY + ".neuralConfig") {
        if (e.newValue) {
          useFace.setState({ neuralConfig: { ...DEFAULT_NEURAL_CONFIG, ...JSON.parse(e.newValue) } });
        }
      }
    } catch {}
  });
}

