import { useState, useRef, useEffect } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { useActiveMusic } from "../store/activeMusic";

interface MusicPlayerCardProps {
  title?: string;
  genre?: string;
  bpm?: number;
  duration?: number;
  audioUrl: string;
  autoPlay?: boolean;
}

export function MusicPlayerCard({
  title = "AI 生成音乐",
  genre,
  bpm,
  duration,
  audioUrl,
  autoPlay = false,
}: MusicPlayerCardProps) {
  const [isPlaying, setIsPlaying] = useState(autoPlay);
  const [currentTime, setCurrentTime] = useState(0);
  const [totalDuration, setTotalDuration] = useState(duration || 0);
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const resolvedAudioUrl = audioUrl.startsWith("file://")
    ? convertFileSrc(audioUrl.slice(7))
    : audioUrl;

  const currentTrack = useActiveMusic((s) => s.currentTrack);
  const playTrack = useActiveMusic((s) => s.playTrack);
  const pauseTrack = useActiveMusic((s) => s.pauseTrack);
  const resumeTrack = useActiveMusic((s) => s.resumeTrack);
  const updateProgress = useActiveMusic((s) => s.updateProgress);
  const isCurrentTrack = currentTrack?.audioUrl === audioUrl;

  // 卡片只做控制与展示，实际播放交给底部全局播放器，避免同一音频双路播放。
  useEffect(() => {
    if (isCurrentTrack && currentTrack) {
      setIsPlaying(currentTrack.isPlaying);
      setCurrentTime(currentTrack.currentTime);
      if (currentTrack.duration) {
        setTotalDuration(Math.floor(currentTrack.duration));
      }
    }
  }, [
    isCurrentTrack,
    currentTrack?.isPlaying,
    currentTrack?.currentTime,
    currentTrack?.duration,
  ]);

  useEffect(() => {
    const audio = audioRef.current;
    if (!audio) return;

    const onTimeUpdate = () => {
      setCurrentTime(audio.currentTime);
      updateProgress(audio.currentTime, audio.duration || totalDuration);
    };
    const onLoadedMetadata = () => {
      if (audio.duration && !isNaN(audio.duration)) {
        setTotalDuration(Math.floor(audio.duration));
      }
    };
    const onEnded = () => {
      setIsPlaying(false);
      setCurrentTime(0);
      pauseTrack();
    };

    audio.addEventListener("timeupdate", onTimeUpdate);
    audio.addEventListener("loadedmetadata", onLoadedMetadata);
    audio.addEventListener("ended", onEnded);

    if (autoPlay) {
      playTrack({
        title,
        genre,
        duration: totalDuration || duration,
        audioUrl,
        localPath: audioUrl.startsWith("file://") ? audioUrl.slice(7) : undefined,
      });
      setIsPlaying(true);
    }

    return () => {
      audio.removeEventListener("timeupdate", onTimeUpdate);
      audio.removeEventListener("loadedmetadata", onLoadedMetadata);
      audio.removeEventListener("ended", onEnded);
    };
  }, [autoPlay, audioUrl, title, genre, duration]);

  const togglePlay = () => {
    if (isCurrentTrack && isPlaying) {
      pauseTrack();
      setIsPlaying(false);
    } else if (isCurrentTrack) {
      resumeTrack();
      setIsPlaying(true);
    } else {
      playTrack({
        title,
        genre,
        duration: totalDuration || duration,
        audioUrl,
        localPath: audioUrl.startsWith("file://") ? audioUrl.slice(7) : undefined,
      });
      setIsPlaying(true);
    }
  };

  const handleSeek = (e: React.ChangeEvent<HTMLInputElement>) => {
    const time = Number(e.target.value);
    setCurrentTime(time);
    if (audioRef.current) {
      audioRef.current.currentTime = time;
    }
  };

  const formatTime = (secs: number) => {
    const m = Math.floor(secs / 60);
    const s = Math.floor(secs % 60);
    return `${m}:${s < 10 ? "0" : ""}${s}`;
  };

  return (
    <div className="my-2 p-3.5 rounded-2xl bg-[var(--surface-2)]/80 border border-[var(--line)] max-w-md shadow-2xs">
      <audio ref={audioRef} src={resolvedAudioUrl} preload="metadata" />

      <div className="flex items-center justify-between gap-3 mb-2.5">
        <div className="flex items-center gap-2.5 min-w-0">
          <div className="w-8 h-8 rounded-xl bg-[var(--celadon)]/15 text-[var(--celadon)] flex items-center justify-center flex-shrink-0">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M9 18V5l10-2v13" />
              <circle cx="6" cy="18" r="3" />
              <circle cx="16" cy="16" r="3" />
            </svg>
          </div>
          <div className="min-w-0">
            <div className="text-[12.5px] font-semibold text-[var(--ink)] truncate">
              {title}
            </div>
            <div className="flex items-center gap-2 text-[11px] text-[var(--ink-3)]">
              {genre && <span className="capitalize">{genre}</span>}
              {bpm && <span>· {bpm} BPM</span>}
              <span>· {formatTime(totalDuration)}</span>
            </div>
          </div>
        </div>

        <button
          type="button"
          onClick={togglePlay}
          className="w-8 h-8 rounded-full bg-[var(--celadon)] text-white hover:opacity-90 flex items-center justify-center transition-all cursor-pointer flex-shrink-0 shadow-xs"
          title={isPlaying ? "暂停" : "播放"}
        >
          {isPlaying ? (
            <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor">
              <rect x="6" y="4" width="4" height="16" rx="1" />
              <rect x="14" y="4" width="4" height="16" rx="1" />
            </svg>
          ) : (
            <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor" className="translate-x-0.5">
              <polygon points="5 3 19 12 5 21 5 3" />
            </svg>
          )}
        </button>
      </div>

      <div className="flex items-center gap-2">
        <span className="text-[10.5px] font-mono text-[var(--ink-3)] min-w-[28px]">
          {formatTime(currentTime)}
        </span>
        <input
          type="range"
          min={0}
          max={totalDuration || 100}
          value={currentTime}
          onChange={handleSeek}
          className="flex-1 h-1.5 bg-[var(--surface-3)] rounded-lg appearance-none cursor-pointer accent-[var(--celadon)]"
        />
        <span className="text-[10.5px] font-mono text-[var(--ink-3)] min-w-[28px] text-right">
          {formatTime(totalDuration)}
        </span>
      </div>
    </div>
  );
}
