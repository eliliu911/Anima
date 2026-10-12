/**
 * 主题开关：跟随系统 → 浅色 → 深色 → 跟随系统。
 *
 * **翻的是整个应用**（写 `<html>` 的 `data-theme`），不是只翻 Studio ——
 * Studio 的色板引用 `theme.css` 里 `:root` 的系统色，只翻一半会得到
 * 「深色 token + 浅色系统色」，一半黑一半白，而且不报错。
 */
import { THEME_LABEL, useThemeMode } from "./useThemeMode";

const IconSun = () => (
  <svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
    <circle cx="10" cy="10" r="3.4" />
    <path d="M10 2v2M10 16v2M18 10h-2M4 10H2M15.7 4.3l-1.4 1.4M5.7 14.3l-1.4 1.4M15.7 15.7l-1.4-1.4M5.7 5.7 4.3 4.3" />
  </svg>
);
const IconMoon = () => (
  <svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round">
    <path d="M16.2 12.4A6.8 6.8 0 0 1 7.6 3.8a6.8 6.8 0 1 0 8.6 8.6z" />
  </svg>
);
/**
 * 跟随系统：**左半是太阳（带光芒），右半是月亮（填实）**。
 *
 * 原来画的是「半边填实的圆」，只是个饼图，看不出和主题有关系。
 * 这个是 macOS「自动」外观那个motif —— 一眼就懂「两种都可能」。
 */
const IconAuto = () => (
  <svg
    width="16"
    height="16"
    viewBox="0 0 20 20"
    fill="none"
    stroke="currentColor"
    strokeWidth="1.4"
    strokeLinecap="round"
  >
    {/* 圆要小一点：16px 下圆占满了就没地方放光芒，看着只剩一个实心点。 */}
    <circle cx="10" cy="10" r="4.4" />
    {/* 右半填实 = 月 */}
    <path d="M10 5.6a4.4 4.4 0 0 1 0 8.8z" fill="currentColor" stroke="none" />
    {/* 左半外侧的光芒 = 日 */}
    <path d="M10 4.2V1.8M5.9 5.9 4.2 4.2M4.2 10H1.8M5.9 14.1 4.2 15.8M10 15.8v2.4" />
  </svg>
);

export function ThemeToggle({ className }: { className?: string }) {
  const { mode, cycle } = useThemeMode();
  return (
    <button
      type="button"
      onClick={cycle}
      title={`主题：${THEME_LABEL[mode]}（点击切换）`}
      aria-label={`主题：${THEME_LABEL[mode]}`}
      className={`flex shrink-0 items-center justify-center rounded-lg p-2 transition-colors hover:bg-white/5 ${className ?? ""}`}
      style={{ color: "var(--studio-text-2)" }}
    >
      {mode === "light" ? <IconSun /> : mode === "dark" ? <IconMoon /> : <IconAuto />}
    </button>
  );
}
