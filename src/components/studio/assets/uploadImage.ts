/**
 * 给已有资产上传现成的图。纯函数部分，抽出来好测。
 *
 * 用户要求（2026-09-13）：不一定每张都生成，别处有现成的图要能直接拿来用。
 * 上传的图作为这个资产的**一版图**加进去并设为封面 —— 和生成的并列，旧的不删，随时能切回去。
 */

/** 能收的格式。出片链路（ComfyUI）认这几种；SVG / HEIC 进去会静默出错，所以在入口挡住。 */
export const ACCEPTED_TYPES = ["image/png", "image/jpeg", "image/webp"] as const;

/** 单张上限。再大读成 data URL 会卡界面，参考图也用不到这么大。 */
export const MAX_BYTES = 30 * 1024 * 1024;

/** 这个文件能不能传。返回 `null` = 可以；否则是给人看的原因。 */
export function uploadProblem(file: { name: string; type: string; size: number }): string | null {
  if (!(ACCEPTED_TYPES as readonly string[]).includes(file.type)) {
    return `「${file.name}」不是 PNG / JPG / WebP 图片`;
  }
  if (file.size > MAX_BYTES) {
    return `「${file.name}」有 ${Math.round(file.size / 1024 / 1024)} MB，超过 30 MB`;
  }
  return null;
}

/** 记在这一版图上的来源说明，界面上看得出这张是传的不是生成的。 */
export const uploadNote = (fileName: string) => `上传：${fileName}`;
