import { create } from "zustand";
import type { CanvasTool } from "../interaction";

/**
 * 画布的指针工具（V 移动 / H 抓手）和「触控板平移」开关。
 *
 * 独立成一个轻量 store：订阅它的只有画布本体和控制栏，
 * 不会因为节点数据变动而重渲染。
 */
const TRACKPAD_KEY = "anima.canvas.trackpadPan";

function readTrackpadPan(): boolean {
  // **默认开。** 笔记本没有鼠标中键，开箱就要能两指滑动平移；
  // 只有用户显式关过（存成 "0"）才保持关闭。
  try {
    const v = window.localStorage.getItem(TRACKPAD_KEY);
    return v === null ? true : v === "1";
  } catch {
    return true;
  }
}

/** 开关类的偏好：存成 "1" / "0"，读不到用默认值 */
function readFlag(key: string, fallback: boolean): boolean {
  try {
    const v = window.localStorage.getItem(key);
    return v === null ? fallback : v === "1";
  } catch {
    return fallback;
  }
}
function writeFlag(key: string, on: boolean) {
  try {
    window.localStorage.setItem(key, on ? "1" : "0");
  } catch {
    /* 存不进去就算了，下次从默认值开始 */
  }
}

export const REDUCE_MOTION_KEY = "anima.canvas.reduceMotion";
export const TOOLBAR_COLLAPSED_KEY = "anima.canvas.toolbarCollapsed";

interface CanvasToolState {
  tool: CanvasTool;
  setTool: (tool: CanvasTool) => void;
  trackpadPan: boolean;
  toggleTrackpadPan: () => void;
  /**
   * 画布里的「减少动态效果」（用户 2026-09-25 要的底栏按钮）。和系统设置是**或**的关系：
   * 任一个开着，连线就回到旧样式（只有实线和虚线，选中 / 在跑的线是移动虚线，没有光晕和彗星）。
   */
  reduceMotion: boolean;
  toggleReduceMotion: () => void;
  /** 底栏收起：只留缩放、适合屏幕、小地图（用户 2026-09-25：「不然这个条越来越长」） */
  toolbarCollapsed: boolean;
  toggleToolbarCollapsed: () => void;
}

export const useCanvasToolStore = create<CanvasToolState>((set, get) => ({
  tool: "move",
  setTool: (tool) => set({ tool }),
  trackpadPan: typeof window === "undefined" ? true : readTrackpadPan(),
  toggleTrackpadPan: () => {
    const next = !get().trackpadPan;
    try {
      window.localStorage.setItem(TRACKPAD_KEY, next ? "1" : "0");
    } catch {
      /* 存不进去就算了，下次从默认值开始 */
    }
    set({ trackpadPan: next });
  },
  reduceMotion: typeof window === "undefined" ? false : readFlag(REDUCE_MOTION_KEY, false),
  toggleReduceMotion: () => {
    const next = !get().reduceMotion;
    writeFlag(REDUCE_MOTION_KEY, next);
    set({ reduceMotion: next });
  },
  toolbarCollapsed: typeof window === "undefined" ? false : readFlag(TOOLBAR_COLLAPSED_KEY, false),
  toggleToolbarCollapsed: () => {
    const next = !get().toolbarCollapsed;
    writeFlag(TOOLBAR_COLLAPSED_KEY, next);
    set({ toolbarCollapsed: next });
  },
}));
