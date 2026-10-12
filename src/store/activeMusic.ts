import { create } from "zustand";

export interface ActiveMusicTrack {
  title: string;
  genre?: string;
  duration?: number;
  localPath?: string;
  audioUrl: string;
  isPlaying: boolean;
  currentTime: number;
}

interface ActiveMusicState {
  currentTrack: ActiveMusicTrack | null;
  playTrack: (track: Omit<ActiveMusicTrack, "isPlaying" | "currentTime">) => void;
  pauseTrack: () => void;
  resumeTrack: () => void;
  stopTrack: () => void;
  updateProgress: (currentTime: number, duration?: number) => void;
}

export const useActiveMusic = create<ActiveMusicState>((set, get) => ({
  currentTrack: null,
  playTrack: (track) => {
    set({
      currentTrack: {
        ...track,
        isPlaying: true,
        currentTime: 0,
      },
    });
  },
  pauseTrack: () => {
    const cur = get().currentTrack;
    if (cur) {
      set({ currentTrack: { ...cur, isPlaying: false } });
    }
  },
  resumeTrack: () => {
    const cur = get().currentTrack;
    if (cur) {
      set({ currentTrack: { ...cur, isPlaying: true } });
    }
  },
  stopTrack: () => {
    set({ currentTrack: null });
  },
  updateProgress: (currentTime, duration) => {
    const cur = get().currentTrack;
    if (cur) {
      set({
        currentTrack: {
          ...cur,
          currentTime,
          duration: duration || cur.duration,
        },
      });
    }
  },
}));
