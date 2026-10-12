/**
 * 局部修改：改图里的一小块，其余不动。
 *
 * # 能力早就有，缺的是三样别的东西
 *
 * 画布上连一张图 + 选 `qwen21_edit` 就已经是指令式编辑了（`ref2i` 模式，
 * `depth_capture` 里「保留原背景换人」用的就是这条）。缺的是：
 *
 * 1. **没人知道它能这么用** —— 得先懂「一张图 + 这个引擎 = 改图」；
 * 2. **没有「其余不变」的说法** —— 指令式编辑最典型的失败是
 *    「它把整张图重画了一遍，脸也跟着变了」，而这不报错；
 * 3. **没法对比** —— 改完看不出哪儿变了。
 *
 * # 模板给用户看，不偷偷追加
 *
 * 「其余一切不变」这句话**写进提示词框里**，用户看得见也改得掉。
 * 在提交那一刻悄悄追加是这个项目反复踩过的坑：界面显示的和实际发出去的
 * 不是一份，出了问题查不回来。
 */

/** 用户没写内容时，提示词框里预填的样子。`{要改的}` 是让人替换的占位。 */
export const LOCAL_EDIT_TEMPLATE =
  "只把{要改的东西}换成{改成什么}。画面其余部分保持完全不变：构图、人物五官、姿势、光线、颜色、背景都不要动。";

/** 占位还没替换 —— 直接发出去会得到一张字面画着「要改的东西」的图。 */
export function stillPlaceholder(prompt: string): boolean {
  return /\{[^}]*\}/.test(prompt);
}

/**
 * 局部修改用哪个引擎。
 *
 * 只有这两个吃「一张图 + 一句指令」。**不能回落到别的引擎** ——
 * 别的引擎在 `ref2i` 下压根不可用（`resolveCanvasEngine` 会抛），
 * 而如果哪天它不抛了，回落的表现是「图被整张重画」，比报错糟得多。
 */
export const LOCAL_EDIT_ENGINES = ["qwen21_edit", "krea2_edit"] as const;

/**
 * 默认用 Qwen-Image-Edit。
 *
 * 2026-09-13 的对照（`docs/plans/2026-09-13-出图模型对照-QwenEdit与Pony.md`）：
 * Qwen **场景 / 构图 / 听指令强**，Krea2Edit 脸更像本人。
 * 局部修改要的正是「听指令」和「别的地方别动」，所以默认 Qwen；
 * 改人脸那类才值得换 Krea2Edit。
 */
export const DEFAULT_LOCAL_EDIT_ENGINE = "qwen21_edit";

export function localEditEngine(available: string[]): string {
  return LOCAL_EDIT_ENGINES.find((e) => available.includes(e)) ?? DEFAULT_LOCAL_EDIT_ENGINE;
}
