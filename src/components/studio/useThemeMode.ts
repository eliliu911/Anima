import { useCallback, useEffect, useState } from "react";
import { setThemeMode, type ThemeMode } from "../../api";
import { getConfig } from "../../api";

/** 循环顺序：跟随系统 → 浅色 → 深色 → 跟随系统。 */
export const THEME_ORDER: ThemeMode[] = ["system", "light", "dark"];

export const THEME_LABEL: Record<ThemeMode, string> = {
  system: "跟随系统",
  light: "浅色",
  dark: "深色",
};

/**
 * 把主题写到 `<html>` 的 `data-theme` 上。
 *
 * **写在根元素上，不是写在 `.anima-studio` 上。** Studio 的色板引用的是
 * `theme.css` 里 `:root` 的系统色（`--paper` / `--ink` / `--celadon`）——
 * 只翻 Studio 自己那一层的话，会得到「深色 token + 浅色系统色」，
 * 一半黑一半白，而且不报错。
 *
 * `system` 时**必须把属性删掉**，不能写 `data-theme="system"` ——
 * CSS 里的守卫是 `:not([data-theme="light"])` 和 `[data-theme="dark"]`，
 * 留一个认不出的值等于永远走不到媒体查询那条。
 */
export interface ThemeTarget {
  setAttribute(name: string, value: string): void;
  removeAttribute(name: string): void;
}

/** `root` 可注入只是为了能测 —— 项目里没有 jsdom。默认就是 `<html>`。 */
export function applyThemeMode(mode: ThemeMode, root?: ThemeTarget) {
  const el = root ?? document.documentElement;
  if (mode === "system") el.removeAttribute("data-theme");
  else el.setAttribute("data-theme", mode);
}

export function useThemeMode() {
  const [mode, setMode] = useState<ThemeMode>("system");

  // 应用主题在 `main.tsx` 入口做（每个窗口一次）。这里只把当前值读回来，
  // 好让按钮显示对的图标 —— 重复 apply 一次没坏处，但没必要。
  useEffect(() => {
    let alive = true;
    void getConfig()
      .then((c) => alive && setMode((c.theme_mode ?? "system") as ThemeMode))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  const cycle = useCallback(() => {
    setMode((cur) => {
      const next = THEME_ORDER[(THEME_ORDER.indexOf(cur) + 1) % THEME_ORDER.length];
      // **先应用再存盘。** 反过来的话，存盘慢一拍时点击像是没反应。
      applyThemeMode(next);
      void setThemeMode(next).catch(() => {});
      return next;
    });
  }, []);

  return { mode, cycle };
}
