/**
 * 图生图的重绘幅度。
 *
 * # 它是图生图**唯一**真正的旋钮
 *
 * 别的参数（尺寸、seed、步数）在文生图里也有。denoise 决定的是
 * 「这次是修一修，还是照着重画一张」—— 而这正是用户点开图生图想控的东西。
 *
 * 之前后端一直用 0.65 这个默认值，界面上**没有出口**，等于只有一档。
 *
 * # 为什么不给到 0
 *
 * denoise 到 0 时采样器一步都不跑（sigmas 是空的），出来就是原图 ——
 * 那不是「几乎不变」，是「什么都没发生」，而且不报错。
 * 下限卡在 0.2：再低下去肉眼已经看不出区别，只是白等一次采样。
 */
export const DENOISE_MIN = 0.2;
export const DENOISE_MAX = 1;
export const DENOISE_STEP = 0.05;
/** 后端 `image_tool` 的默认值。**两边必须一致** —— 不一致的话界面显示 0.65、
 *  实际发出去的是别的数，而那种偏差没人看得出来。 */
export const DENOISE_DEFAULT = 0.65;

/** 这个模式吃不吃 denoise。只有带源图重绘的那条。 */
export function usesDenoise(modeId: string): boolean {
  return modeId === "i2i";
}

/**
 * 当前这个值意味着什么。**用结果说话，不用形容词** ——
 * 「0.85 = 强」没有信息量，「构图会重来」才有。
 */
export function denoiseHint(v: number): string {
  if (v <= 0.35) return "只动表面：修瑕疵、调质感，构图和人物基本不变";
  if (v <= 0.55) return "保留构图，重画细节：换材质、换光线还认得出是同一张";
  if (v <= 0.75) return "同一个构图重新演绎：人物和场景会变，但布局还在";
  if (v < 1) return "只当参考：构图大改，源图更像是个氛围提示";
  return "等于不用源图：整张重画，图生图在这一档没有意义";
}

/** 显示成两位小数，避免 0.65000000000000002 这种浮点噪声。 */
export function fmtDenoise(v: number): string {
  return v.toFixed(2);
}

/** 夹到合法区间。界面和后端各夹一次 —— 后端那次是给 agent 那条路兜底的。 */
export function clampDenoise(v: number): number {
  if (!Number.isFinite(v)) return DENOISE_DEFAULT;
  return Math.min(DENOISE_MAX, Math.max(DENOISE_MIN, v));
}
