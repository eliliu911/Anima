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
}

export const canvasSpeak = (nodeId: string, args: { text: string; voice?: string; engineId?: string }) =>
  invoke<CanvasSpeakOut>("canvas_speak", {
    nodeId,
    args: { text: args.text, voice: args.voice ?? "", engine_id: args.engineId ?? "" },
  });
