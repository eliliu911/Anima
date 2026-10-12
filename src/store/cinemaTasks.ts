import { create } from "zustand";

export interface SheetTaskInfo {
  characterId: string;
  projectId: string;
  label: string;
  done: number;
  total: number;
  isProcessing: boolean;
}

interface CinemaTaskStore {
  activeSheetTasks: Record<string, SheetTaskInfo>;
  setSheetTask: (characterId: string, task: SheetTaskInfo | null) => void;
  updateSheetTaskProgress: (characterId: string, done: number, total: number, label: string) => void;
}

export const useCinemaTasks = create<CinemaTaskStore>((set) => ({
  activeSheetTasks: {},
  setSheetTask: (characterId, task) =>
    set((state) => {
      const next = { ...state.activeSheetTasks };
      if (task) {
        next[characterId] = task;
      } else {
        delete next[characterId];
      }
      return { activeSheetTasks: next };
    }),
  updateSheetTaskProgress: (characterId, done, total, label) =>
    set((state) => {
      const existing = state.activeSheetTasks[characterId];
      if (!existing) return state;
      return {
        activeSheetTasks: {
          ...state.activeSheetTasks,
          [characterId]: { ...existing, done, total, label, isProcessing: done < total || total === 0 },
        },
      };
    }),
}));
