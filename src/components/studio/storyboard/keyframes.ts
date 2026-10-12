/**
 * 关键帧（首帧 / 尾帧）的界面规则。
 *
 * 用户 2026-09-27：「把首帧，首尾帧功能补进分镜里。可以是先生成，也可以是导入首尾帧，
 * 或者首帧，根据导入情况自动识别。」
 *
 * **出片方式不让人选，按挂了什么自动定**（后端 `cinema::Keyframes` 是同一张表）。
 * 起因：E01 第 1 镜只靠参考图出，场景、道具、机位全是模型编的；
 * 同一镜先出首尾帧再出视频，一次就对。
 */
import type { Keyframes, PromptRoute, StoryboardShot } from "../../../api/cinema";

export type KeyframeMode = "reference" | "first" | "first_last" | "last_only";

export function keyframeMode(k: Keyframes | null | undefined): KeyframeMode {
  if (k?.first && k.last) return "first_last";
  if (k?.first) return "first";
  if (k?.last) return "last_only";
  return "reference";
}

export const KEYFRAME_MODE_LABEL: Record<KeyframeMode, string> = {
  reference: "参考生视频",
  first: "首帧图生视频",
  first_last: "首尾帧",
  last_only: "只有尾帧",
};

export const KEYFRAME_MODE_HINT: Record<KeyframeMode, string> = {
  reference: "没有关键帧：资产图只管长相，画面、机位、道具摆放由模型自己编。要锁住画面就先出或导入首帧",
  first: "从首帧这张图开始动：场景、机位、衣服、道具都照这张图，提示词只管接下来怎么动",
  first_last: "开头落在首帧、结尾落在尾帧，中间的运动由模型补。推拉摇移最稳的一种",
  last_only: "只有尾帧出不了片 —— 补一张首帧，或者把尾帧摘掉",
};

/** 挂上 / 摘掉关键帧之后提示词要重新合成：三种出片方式的正文写法不同 */
export const KEYFRAME_RESYNTH_NOTE = "换了出片方式，这一镜的提示词要重新合成才能出片（只换一张图不用）";

/**
 * 这一镜在当前线路上有没有合成好的分镜提示词。
 *
 * **没有也能出首帧**（2026-10-03）：后端按镜头表现拼一份（线路 A 的写法，不调模型、不花 token），
 * 见 `cinema::first_frame_prompt`。出完首帧自动按首帧模式合成一次 —— 原来要「合成 → 出首帧 → 再合成」，
 * 第一轮合成出完首帧就作废，用户：「这不是犯傻吗」。
 */
export function hasFramePrompt(shot: StoryboardShot, route: PromptRoute): boolean {
  return !!shot.prompts?.[route]?.frame?.trim();
}

export interface FramePlan {
  /** 这一次要出首帧的镜头，按镜号 */
  ids: string[];
  /** 已经有首帧、这次不动的镜数（没勾「重出」时） */
  kept: number;
  /** 这次要出的镜头里还没合成提示词的：照样出（按镜头表现拼画面提示词），出完自动合成 */
  composed: number;
  /** 勾中的场里建议走参考出片的镜数：不出首帧 */
  reference: number;
}

/**
 * 按场出首帧：这几场里哪些镜要出。
 *
 * **已经有首帧的默认不动** —— 导入的、挑过的首帧不能被一键盖掉。
 * 后端 `cinema_render_episode_frames` 按同一条规矩再筛一遍（界面状态可能是旧的）。
 */
/**
 * `byRef` = 建议走参考出片的镜头 id（Rust `shot_route::content_route`，含手动选的）：后端不给它们出首帧，这里也不数进去 ——
 * 以前弹窗按场把每一镜都数进来，E05 显示「出 31 镜首帧 · 约 134 分钟」，后端实际只出 19 镜（2026-10-11 用户：「写 31，不是 19」）
 */
export function framePlan(shots: StoryboardShot[], sceneNos: number[], route: PromptRoute, redo: boolean, byRef: ReadonlySet<string> = new Set()): FramePlan {
  const plan: FramePlan = { ids: [], kept: 0, composed: 0, reference: 0 };
  for (const s of [...shots].sort((a, b) => a.shot_num - b.shot_num)) {
    if (!sceneNos.includes(s.scene_no ?? 0)) continue;
    if (byRef.has(s.id)) plan.reference += 1;
    else if (s.keyframes?.first && !redo) plan.kept += 1;
    else {
      plan.ids.push(s.id);
      if (!hasFramePrompt(s, route)) plan.composed += 1;
    }
  }
  return plan;
}
