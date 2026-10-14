import { invoke } from "@tauri-apps/api/core";

/**
 * 画布上的试音色：同一段词、不同嗓子，一次出一条 wav。
 *
 * 引擎和音色的取法和 `cinema_dub` 完全一样 —— 另写一份必然出现
 * 「分镜页配出来是这个声，画布上试听是另一个声」。
 */
export interface CanvasSpeakOut {
  audio_path: string;
  /** 实际用的音色和引擎。**回显，不是回声** —— 传了不存在的音色时
   *  引擎可能悄悄退回默认，这两个字段是唯一看得出来的地方。 */
  voice: string;
  engine_id: string;
  secs: number;
  /** 声音复制时：实际用的参考（原音频里第几秒到第几秒）和它的文字。文字错了会「音色像但吐字乱」 */
  clone?: {
    ref_text: string;
    ref_text_edited: boolean;
    from_secs: number;
    to_secs: number;
    source_secs: number;
    /** 参考不到 3 秒，末尾补了静音（GPT-SoVITS 要 3~10 秒） */
    padded: boolean;
  };
}

/** 生成参数（照 LibTV 的音频节点）。都不传 = 原样 */
export interface SpeakOptions {
  /** 声音复制：参考音频本机路径 */
  refAudio?: string;
  /** 参考音频说的话。空 = 自动（Whisper） */
  refText?: string;
  /** 语速倍数 0.5~2 */
  speed?: number;
  /** 声调（半音）-12~12 */
  pitch?: number;
  /** 音量倍数 0~2 */
  volume?: number;
  sampleRate?: 8000 | 16000 | 24000 | 48000;
  format?: "wav" | "mp3" | "pcm" | "ogg_opus";
}

export const canvasSpeak = (nodeId: string, args: { text: string; voice?: string; engineId?: string } & SpeakOptions) =>
  invoke<CanvasSpeakOut>("canvas_speak", {
    nodeId,
    args: {
      text: args.text, voice: args.voice ?? "", engine_id: args.engineId ?? "",
      ref_audio: args.refAudio ?? "", ref_text: args.refText ?? "",
      speed: args.speed ?? 1, pitch: args.pitch ?? 0, volume: args.volume ?? 1,
      sample_rate: args.sampleRate ?? 24000, format: args.format ?? "wav",
    },
  });
