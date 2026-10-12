/**
 * 这一次能不能点「生成」，以及**点不了的话是因为什么**。
 *
 * # 为什么要把原因也算出来
 *
 * 一个灰着的按钮，如果不说为什么灰，用户只能挨个试：是没选引擎？
 * 提示词太短？还是少传了图？这条链路上「点不动」的原因有三种，
 * 而它们的解法完全不同。灰着不解释，比不灰更让人卡住。
 */
export interface SubmitCheck {
  ok: boolean;
  /** `ok` 为 false 时给人看的一句话。`ok` 时是操作说明。 */
  reason: string;
}

export function canSubmit(args: {
  engineId: string | null | undefined;
  prompt: string;
  /** 已经放好的输入图（可能有空位）。 */
  images: (string | null | undefined)[];
  /** 这个模式需要几张输入图。0 = 纯文生。 */
  needImages: number;
  mediaLabel?: string | null;
}): SubmitCheck {
  if (!args.engineId) return { ok: false, reason: "先选一个引擎" };
  if (!args.prompt.trim()) return { ok: false, reason: "写点提示词再生成" };
  const filled = args.images.filter((p) => !!p && p.trim() !== "").length;
  if (filled < args.needImages) {
    const label = args.mediaLabel?.trim() || "输入图";
    const missing = args.needImages - filled;
    return { ok: false, reason: `还差 ${missing} 张${label}` };
  }
  return { ok: true, reason: "生成" };
}
