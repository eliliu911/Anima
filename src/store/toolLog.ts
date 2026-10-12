/**
 * 工具执行日志。
 *
 * ## 为什么是 store 而不是组件里的 useState
 *
 * 日志有**两个来源**：文字链路的 agent 循环（`anima://tool-activity` 事件）和
 * 语音链路（`VoiceEvent::Tool`）。后者原先根本没进这里 —— 于是语音模式下
 * 执行日志永远是「暂无工具执行记录」，而那正是最需要它的场景：
 * 模型说"终端已打开"而实际没打开时，这条日志是唯一的判据。
 *
 * 放 store 还有个好处：日志不随面板开关消失。人是**先觉得不对、再去开面板**的，
 * 那时候日志必须还在。
 */
import { create } from "zustand";

export type ToolStatus = "running" | "ok" | "failed" | "rejected" | "declined";

export interface ToolLogLine {
  key: string;
  name: string;
  status: ToolStatus;
  detail?: string;
  at: number;
}

/** 最多留多少条。再多没人翻，而且会让面板一直重排 */
const MAX_LINES = 60;

interface ToolLogState {
  lines: ToolLogLine[];
  /**
   * 记一条。`key` 相同就**原地更新**。
   *
   * 「开始」和「结束」各占一行的话，一次调用看起来像调了两次，
   * 而且看不出它是还在跑还是跑完了 —— 恰恰是最该一眼看出来的信息。
   */
  record: (line: Omit<ToolLogLine, "at">) => void;
  clear: () => void;
}

export const useToolLog = create<ToolLogState>((set) => ({
  lines: [],
  record: (line) =>
    set((state) => {
      const at = Date.now();
      const index = state.lines.findIndex((l) => l.key === line.key);
      if (index >= 0) {
        const next = [...state.lines];
        next[index] = { ...line, at };
        return { lines: next };
      }
      return { lines: [...state.lines, { ...line, at }].slice(-MAX_LINES) };
    }),
  clear: () => set({ lines: [] }),
}));
