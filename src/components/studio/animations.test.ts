import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

/**
 * **组件里引用的每一个动画名，都必须真的定义了 `@keyframes`。**
 *
 * 名字对不上时 CSS **不报错**，只是什么都不动。表现是「进度环不转」——
 * 而这恰恰是最需要动的地方：转圈是在告诉用户「还活着，别再点了」。
 *
 * 真事：加进度条时我以为已经写过 keyframes（那条命令是
 * `grep … | head || 补上`，而管道的退出码是 `head` 的、永远为 0，
 * 所以补的那一支从来没跑过）。两处引用了一个不存在的动画名，
 * 一路过了类型检查、构建和 689 个测试，直到用户看见它不转。
 */
const STUDIO = fileURLToPath(new URL(".", import.meta.url));
/**
 * **所有 studio 样式表一起扫，不只是主题文件。**
 *
 * 原来只读 `studio-theme.css`。后来润色面板和 BorderGlow 各自带了
 * 组件级的 .css（keyframes 就定义在里面）—— 这条检查的前提
 * 「动画名都在主题文件里」当场失效：从 .tsx 引用 `shot-polish-sweep`
 * 会被误报成「未定义」，而它明明定义了。
 *
 * 一个会误报的守卫和一个失效的守卫一样糟：前者让人开始忽略它。
 */
function cssFiles(dir = STUDIO_ROOT): string[] {
  return readdirSync(dir).flatMap((n) => {
    const p = join(dir, n);
    if (statSync(p).isDirectory()) return cssFiles(p);
    return n.endsWith(".css") ? [p] : [];
  });
}

const STUDIO_ROOT = fileURLToPath(new URL(".", import.meta.url));
const CSS = [
  readFileSync(fileURLToPath(new URL("../../studio-theme.css", import.meta.url)), "utf8"),
  ...cssFiles().map((f) => readFileSync(f, "utf8")),
].join("\n");

function tsxFiles(dir = STUDIO): string[] {
  return readdirSync(dir).flatMap((n) => {
    const p = join(dir, n);
    if (statSync(p).isDirectory()) return tsxFiles(p);
    return n.endsWith(".tsx") ? [p] : [];
  });
}

/**
 * 取 `animation:` 那一行里**引号内**的第一个词。
 *
 * 不能直接匹配 `animation:` 后面的标识符 —— 那会把三元表达式的条件
 * 当成动画名（`animation: extracting ? "studio-spin …" : undefined`
 * 会抽出 `extracting`）。动画名一定在字符串字面量里，条件不是。
 */
function usedAnimations(src: string): string[] {
  const out: string[] = [];
  for (const line of src.split("\n")) {
    if (!line.includes("animation:")) continue;
    for (const m of line.matchAll(/["'`]\s*([a-zA-Z][\w-]*)[\s\d]/g)) {
      if (m[1] !== "none") out.push(m[1]);
    }
  }
  return out;
}

const DEFINED = new Set(
  [...CSS.matchAll(/@keyframes\s+([\w-]+)/g)].map((m) => m[1]),
);

describe("动画名对得上", () => {
  it("CSS 里确实定义了 keyframes —— 不是扫了个空", () => {
    expect(DEFINED.size).toBeGreaterThan(0);
  });

  // 「组件自带的 .css 也扫到了」那条删了：润色面板的 .css（2026-09-13）和 BorderGlow.css
  // （2026-09-16 清理死代码）都没了，现在 studio 下没有组件级样式表。
  // `cssFiles()` 的递归扫描留着 —— 以后再有组件带 .css，这条规则照样认得到。

  it("转圈的那个是定义了的（进度条靠它）", () => {
    expect(DEFINED.has("studio-spin")).toBe(true);
  });

  it.each(tsxFiles().map((f) => [f.slice(STUDIO.length), f]))("%s", (_n, file) => {
    const used = usedAnimations(readFileSync(file, "utf8"));
    const missing = used.filter((a) => !DEFINED.has(a));
    expect(missing, `引用了没定义的动画：${missing.join(", ")}`).toEqual([]);
  });
});
