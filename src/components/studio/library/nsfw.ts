import { create } from "zustand";

/**
 * NSFW 缩略图显示偏好。**默认模糊**（用户 2026-09-15 定）。
 *
 * 两层：
 * - `showAll`：工具条上的 👁 开关，记在本机（localStorage），下次打开还是这个状态
 * - `revealed`：单张点了眼睛临时显示的，只在这次会话里
 *
 * `localStorage` 在测试环境和隐私模式下会抛 —— 读写都包起来，出错就当默认模糊。
 */
const KEY = "anima.library.nsfwShowAll";

function readShowAll(): boolean {
  try {
    return localStorage.getItem(KEY) === "1";
  } catch {
    return false;
  }
}

interface NsfwState {
  showAll: boolean;
  revealed: Set<string>;
  setShowAll: (on: boolean) => void;
  toggle: (key: string) => void;
}

export const useNsfw = create<NsfwState>((set) => ({
  showAll: readShowAll(),
  revealed: new Set(),
  setShowAll: (on) => {
    try {
      localStorage.setItem(KEY, on ? "1" : "0");
    } catch {
      // 记不住就只管这次
    }
    set({ showAll: on });
  },
  toggle: (key) =>
    set((s) => {
      const next = new Set(s.revealed);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return { revealed: next };
    }),
}));

/** 这张图现在该不该模糊 */
export function shouldBlur(nsfw: boolean, key: string, state: Pick<NsfwState, "showAll" | "revealed">): boolean {
  return nsfw && !state.showAll && !state.revealed.has(key);
}
