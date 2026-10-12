import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { applyThemeMode, THEME_ORDER, type ThemeTarget } from "./useThemeMode";

/** 记下所有属性操作的桩 —— 项目里没有 jsdom。 */
function stub() {
  const attrs = new Map<string, string>();
  const target: ThemeTarget = {
    setAttribute: (k, v) => void attrs.set(k, v),
    removeAttribute: (k) => void attrs.delete(k),
  };
  return { target, attrs };
}

describe("主题属性", () => {
  it("浅色 / 深色写成 data-theme", () => {
    for (const m of ["light", "dark"] as const) {
      const { target, attrs } = stub();
      applyThemeMode(m, target);
      expect(attrs.get("data-theme")).toBe(m);
    }
  });

  /**
   * **`system` 必须把属性删掉，不能写成 `data-theme="system"`。**
   *
   * CSS 的守卫是 `:not([data-theme="light"])` 和 `[data-theme="dark"]` ——
   * 留一个认不出的值，等于永远走不到媒体查询那条，界面卡在浅色不动，
   * 而且不报任何错。
   */
  it("跟随系统时删掉属性，不是写成 system", () => {
    const { target, attrs } = stub();
    applyThemeMode("dark", target);
    applyThemeMode("system", target);
    expect(attrs.has("data-theme")).toBe(false);
  });

  it("来回切换不残留", () => {
    const { target, attrs } = stub();
    for (const m of ["light", "dark", "system", "dark", "system"] as const) {
      applyThemeMode(m, target);
    }
    expect(attrs.has("data-theme")).toBe(false);
  });
});

describe("循环顺序", () => {
  it("三态，从跟随系统开始", () => {
    expect(THEME_ORDER).toEqual(["system", "light", "dark"]);
  });

  it("循环一圈回到原点", () => {
    let cur: (typeof THEME_ORDER)[number] = "system";
    for (let i = 0; i < THEME_ORDER.length; i++) {
      cur = THEME_ORDER[(THEME_ORDER.indexOf(cur) + 1) % THEME_ORDER.length];
    }
    expect(cur).toBe("system");
  });
});

describe("翻的是整个应用，不是只翻 Studio", () => {
  const read = (rel: string) => readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");

  /**
   * Studio 的 token 引用 `theme.css` 里 `:root` 上的系统色。
   * 只翻 `.anima-studio` 那一层的话，会得到「深色 token + 浅色系统色」——
   * 一半黑一半白，而且不报错。
   */
  it("默认目标是 documentElement", () => {
    expect(read("./useThemeMode.ts")).toContain("document.documentElement");
  });

  it("Studio 的深色选择器由根元素驱动", () => {
    const css = read("../../studio-theme.css");
    expect(css).toContain(':root[data-theme="dark"] .anima-studio');
    expect(css).toContain(':root:not([data-theme="light"]) .anima-studio');
    // 自己那一层的写法不能再出现，否则只翻一半
    expect(css).not.toMatch(/\.anima-studio\[data-theme="dark"\]/);
  });

  it("主界面的媒体查询有 light 守卫 —— 否则手动选浅色按不回去", () => {
    const css = read("../../theme.css");
    const i = css.indexOf("@media (prefers-color-scheme: dark)");
    expect(css.slice(i, i + 200)).toContain(':root:not([data-theme="light"])');
  });
});
