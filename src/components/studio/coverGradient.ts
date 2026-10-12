/**
 * 没有封面图时的确定性渐变封面。
 *
 * 思路来自 LumenX 的 `atelierCover.ts`（MIT）：**同一个 id 永远得到
 * 同一个渐变** —— 每次渲染换一个颜色的话，用户没法靠颜色记住项目。
 *
 * 颜色全部走我们自己的 token，不写死具体色值。
 */
/**
 * 封面用**专用的深色两端**（`--studio-cover-1/2`），不跟着主题变浅 ——
 * 封面是「图片位」，上面压着白字；浅色主题下底色跟着变浅的话，
 * 白字就没了，而且不报错，只是看不见。
 */
const GRADIENTS = [
  "linear-gradient(150deg, color-mix(in oklab, var(--studio-primary) 46%, var(--studio-cover-1)) -10%, var(--studio-cover-2) 76%)",
  "linear-gradient(135deg, color-mix(in oklab, var(--studio-accent) 38%, var(--studio-cover-1)) -14%, var(--studio-cover-2) 74%)",
  "linear-gradient(160deg, color-mix(in oklab, var(--studio-primary) 34%, var(--studio-cover-1)) -16%, var(--studio-cover-1) 52%, var(--studio-cover-2) 100%)",
  "linear-gradient(140deg, var(--studio-cover-1) 0%, color-mix(in oklab, var(--studio-primary) 42%, var(--studio-cover-2)) 150%)",
  "linear-gradient(200deg, color-mix(in oklab, var(--studio-accent) 30%, var(--studio-cover-1)) -8%, var(--studio-cover-2) 70%)",
];

export function coverGradient(seed: string): string {
  let h = 0;
  for (let i = 0; i < seed.length; i++) h = (h * 31 + seed.charCodeAt(i)) >>> 0;
  return GRADIENTS[h % GRADIENTS.length];
}

export const GRADIENT_COUNT = GRADIENTS.length;
