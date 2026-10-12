import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

/**
 * 主题的失败**全是安静的**：漏一个 token 就是浅色下白底白字、
 * 深色下黑底黑字 —— 不报错，只是看不见。所以从源码上钉。
 */
const CSS_PATH = fileURLToPath(new URL("../../studio-theme.css", import.meta.url));
const css = readFileSync(CSS_PATH, "utf8");
const STUDIO_DIR = fileURLToPath(new URL(".", import.meta.url));
const LIGHT = ".anima-studio {\n  color-scheme: light;";
const DARK = ':root[data-theme="dark"] .anima-studio {';

function tsxFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((n) => {
    const p = join(dir, n);
    if (statSync(p).isDirectory()) return tsxFiles(p);
    return n.endsWith(".tsx") ? [p] : [];
  });
}

/** 取某个选择器块的正文。 */
function bodyOf(blockStart: string): string {
  const i = css.indexOf(blockStart);
  if (i < 0) throw new Error(`找不到选择器块：${blockStart}`);
  const rest = css.slice(i);
  const end = rest.search(/\n\}\n/);
  return rest.slice(0, end < 0 ? rest.length : end);
}

/**
 * 块里定义的所有 token（含 `--seed-*`）。
 *
 * **值是 `var(--seed-*)` 的别名不算**（如 `--studio-primary: var(--seed-primary)`）——
 * 深色只改 seed，别名自动跟着走，重定义反而是噪音。
 */
function tokensIn(blockStart: string): Set<string> {
  const body = bodyOf(blockStart);
  const out = new Set<string>();
  for (const m of body.matchAll(/(--(?:studio|seed)-[\w-]+)\s*:\s*([^;]+);/g)) {
    if (/^var\(--seed-/.test(m[2].trim())) continue;
    out.add(m[1]);
  }
  return out;
}

const themeCss = readFileSync(
  fileURLToPath(new URL("../../theme.css", import.meta.url)),
  "utf8",
);

/**
 * 在 `theme.css` 的浅色 / 深色块里查一个变量的字面值。
 *
 * Studio 的 token 现在**引用 Anima 的系统色**（`var(--surface)` 之类），
 * 所以要一路跟过去才拿得到真实颜色。跟着查也更强：
 * Anima 哪天把配色改到「卡片比窗口还黑」，这里一样会报。
 */
function animaVar(name: string, dark: boolean): string {
  // 深色定义在 `@media (prefers-color-scheme: dark)` 之后
  const from = dark ? themeCss.indexOf("@media (prefers-color-scheme: dark)") : 0;
  const scope = dark ? themeCss.slice(from) : themeCss.slice(0, themeCss.indexOf("@media"));
  const m = scope.match(new RegExp(`\\${name}:\\s*(#[0-9a-fA-F]{3,8})`));
  if (!m) throw new Error(`theme.css ${dark ? "深色" : "浅色"} 里没有 ${name}`);
  return expand(m[1].toLowerCase());
}

function expand(h: string): string {
  return h.length === 4
    ? `#${h[1]}${h[1]}${h[2]}${h[2]}${h[3]}${h[3]}`
    : h.slice(0, 7);
}

/** 取 studio token 的最终颜色，`var(--x)` 会一路跟到 theme.css。 */
function hex(name: string, blockStart: string): string {
  const body = bodyOf(blockStart);
  const m = body.match(new RegExp(`${name}:\\s*([^;]+);`));
  if (!m) throw new Error(`${blockStart} 里没有 ${name}`);
  const raw = m[1].trim();
  const ref = raw.match(/^var\((--[\w-]+)\)$/);
  if (ref) return animaVar(ref[1], blockStart === DARK);
  const lit = raw.match(/^#[0-9a-fA-F]{3,8}/);
  if (lit) return expand(lit[0].toLowerCase());
  throw new Error(`${name} 的值解析不了：${raw}`);
}

describe("浅色 / 深色两套 token", () => {
  it("浅色里定义的每一个 token，深色里都要有", () => {
    // 漏一个 = 那个颜色在深色下仍然是浅色值。
    const light = tokensIn(LIGHT);
    const dark = tokensIn(DARK);
    const missing = [...light].filter((t) => !dark.has(t));
    expect(missing, `深色里少了：${missing.join(" ")}`).toEqual([]);
  });

  it("深色也不该多出浅色里没有的 token", () => {
    const light = tokensIn(LIGHT);
    const extra = [...tokensIn(DARK)].filter((t) => !light.has(t));
    expect(extra, `深色多出来：${extra.join(" ")}`).toEqual([]);
  });

  it("四个 seed 深色里全都重定义了", () => {
    // 别名靠 seed 转向。seed 漏一个，一整族颜色都停在浅色值上。
    const dark = tokensIn(DARK);
    for (const seed of ["--seed-primary", "--seed-primary-2", "--seed-accent", "--seed-accent-2"]) {
      expect(dark.has(seed), `深色里没有 ${seed}`).toBe(true);
    }
  });

  it("跟随系统的那份和手动指定深色那份，token 集合一致", () => {
    const media = tokensIn(':root:not([data-theme="light"]) .anima-studio {');
    expect([...media].sort()).toEqual([...tokensIn(DARK)].sort());
  });
});

describe("引用了就必须定义", () => {
  /**
   * **这条是最重要的一条，因为它抓的是「整块塌掉」。**
   *
   * `var(--studio-primary)` 没定义时不会报错 —— 它解析成空，
   * 于是外面那层 `color-mix(...)` 整条失效，背景直接塌成透明。
   * 表现是白底白字 / 封面变成一块空白，而且控制台一声不吭。
   *
   * 真实事故：重写色板时把 `--studio-primary: var(--seed-primary)`
   * 这几行别名整个漏了，主色按钮和封面渐变全塌。
   * 而当时的 token 对齐测试**恰好把别名排除在外**，一点没报。
   */
  const defined = new Set<string>();
  for (const m of css.matchAll(/(--(?:studio|seed)-[\w-]+)\s*:/g)) defined.add(m[1]);

  it("CSS 里用到的每个 --studio-* 都定义了", () => {
    const used = new Set(
      [...css.matchAll(/var\((--(?:studio|seed)-[\w-]+)/g)].map((m) => m[1]),
    );
    const missing = [...used].filter((t) => !defined.has(t));
    expect(missing, `用了没定义：${missing.join(" ")}`).toEqual([]);
  });

  it("组件里用到的每个 --studio-* 也都定义了", () => {
    const used = new Set<string>();
    for (const f of tsxFiles(STUDIO_DIR)) {
      for (const m of readFileSync(f, "utf8").matchAll(/var\((--studio-[\w-]+)/g)) {
        used.add(m[1]);
      }
    }
    const missing = [...used].filter((t) => !defined.has(t));
    expect(missing, `组件用了但没定义：${missing.join(" ")}`).toEqual([]);
  });

  it("别名那几行必须在 —— 它们是最容易被顺手删掉的", () => {
    for (const t of ["--studio-primary", "--studio-primary-hover", "--studio-accent"]) {
      expect(bodyOf(LIGHT), `浅色缺 ${t}`).toContain(`${t}: var(--seed-`);
    }
  });
});

describe("深色不是纯黑", () => {
  const lum = (h: string) =>
    (parseInt(h.slice(1, 3), 16) + parseInt(h.slice(3, 5), 16) + parseInt(h.slice(5, 7), 16)) / 3;

  it("四档背景层层拉开，肉眼分得出", () => {
    // 原来 base #0c0b0e、inset #0a090c —— 窗口和卡片都黑，
    // 卡片比窗口还黑，层次全糊在一起。
    const inset = lum(hex("--studio-bg-inset", DARK));
    const base = lum(hex("--studio-bg-base", DARK));
    const surface = lum(hex("--studio-bg-surface", DARK));
    const elevated = lum(hex("--studio-bg-elevated", DARK));
    expect(inset).toBeLessThan(base);
    expect(base).toBeLessThan(surface);
    expect(surface).toBeLessThan(elevated);
    // 每一档至少差 6/255，否则和没分一样
    expect(surface - base).toBeGreaterThanOrEqual(6);
    expect(elevated - surface).toBeGreaterThanOrEqual(6);
  });

  it("最深的那一档也不是纯黑", () => {
    expect(lum(hex("--studio-bg-base", DARK))).toBeGreaterThan(14);
  });

  it("浅色的窗口底不是纯白", () => {
    // 纯白在大屏上一样刺眼；卡片可以是白的，窗口底不行。
    expect(lum(hex("--studio-bg-base", LIGHT))).toBeLessThan(250);
  });

  it("浅色下卡片要比窗口底亮 —— 不然卡片看不出是卡片", () => {
    expect(lum(hex("--studio-bg-surface", LIGHT))).toBeGreaterThan(
      lum(hex("--studio-bg-base", LIGHT)),
    );
  });

  it("底色是渐变，不是一块平色", () => {
    expect(css).toMatch(/background:\s*\n?\s*linear-gradient\([^)]*--studio-bg-base/);
  });
});

describe("对齐 Anima 系统色", () => {
  /**
   * Studio 的主色/文字/背景**必须引用 `theme.css`**，不能自己复制一份。
   * 复制的那份不会跟着 Anima 改配色走，两边越漂越远 ——
   * 而且没有任何东西会报错。
   */
  it("主色和文字色都引用系统变量，不写死", () => {
    for (const block of [LIGHT, DARK]) {
      // **必须限定在块内。** 切到文件末尾的话，另一个块里的值
      // 会把这个块的断言满足掉 —— 变异测试第一次跑就抓到了。
      const body = bodyOf(block);
      for (const token of ["--seed-primary", "--studio-text", "--studio-bg-base"]) {
        const m = body.match(new RegExp(`${token}:\\s*([^;]+);`));
        expect(m, `${token} 不见了`).not.toBeNull();
        expect(m![1].trim(), `${token} 写死了：${m![1]}`).toMatch(/^var\(--/);
      }
    }
  });

  it("没有橘黄系的强调色 —— 那是从参考项目抄来的，和 Anima 不是一个方向", () => {
    // #ffa94d / #ffbf73 / #d97b28 之类
    const seeds = css.match(/--seed-accent[^;]*;/g) ?? [];
    expect(seeds.length).toBeGreaterThan(0);
    for (const s of seeds) expect(s).not.toMatch(/#(ff|d9|e8)[0-9a-f]{4}/i);
  });

  it("状态色落在 Anima 自己的语义色上", () => {
    for (const block of [LIGHT, DARK]) {
      const body = bodyOf(block);
      expect(body, block).toMatch(/--studio-pending-fg:\s*var\(--celadon\)/);
      expect(body, block).toMatch(/--studio-running-fg:\s*var\(--amber\)/);
      expect(body, block).toMatch(/--studio-failed-fg:\s*var\(--clay\)/);
    }
  });
});

describe("悬停不抢「选中」的表达", () => {
  /** 取某个选择器规则的声明块。 */
  function ruleBody(selector: string): string {
    const i = css.indexOf(selector);
    if (i < 0) throw new Error(`找不到规则：${selector}`);
    const open = css.indexOf("{", i);
    return css.slice(open, css.indexOf("}", open));
  }

  /**
   * **悬停用中性色，不用主色。**
   *
   * 一圈饱和的主色描边看着廉价，更要紧的是它把「鼠标在这儿」
   * 说成了「这个被选中了」—— 两个状态长得一样，就等于都没表达。
   */
  it("悬停的描边是中性的", () => {
    const body = ruleBody(".anima-studio .studio-tile:hover");
    expect(body).toContain("0 0 0 1px var(--studio-border),");
    expect(body, `悬停还在用主色：${body}`).not.toContain("--studio-primary");
  });

  it("菜单开着时也不用主色描边", () => {
    const body = ruleBody('.anima-studio .studio-tile[data-menu-open="true"],');
    expect(body).not.toContain("--studio-primary");
  });

  it("选中才用主色 —— 那一个是真在表达状态", () => {
    const body = ruleBody('.anima-studio .studio-tile[data-active="true"]');
    expect(body).toContain("--studio-primary");
  });
});

describe("圆角不漏底", () => {
  /** 取声明块，**去掉注释** —— 注释里提到 `overflow` 会让断言误判。 */
  function ruleBody(selector: string): string {
    const i = css.indexOf(selector);
    if (i < 0) throw new Error(`找不到规则：${selector}`);
    const open = css.indexOf("{", i);
    return css.slice(open, css.indexOf("}", open)).replace(/\/\*[\s\S]*?\*\//g, "");
  }

  /**
   * **卡片的描边必须是 inset 阴影，不能是 `border`。**
   *
   * `border` + `overflow:hidden` 会让子元素裁到**内边距盒**的圆角，
   * 而元素自己的背景画到**边框盒** —— 两条弧被 1px 边框错开，
   * 角上露出一条卡片底色。染色实验里那是一道显眼的红弧。
   */
  /**
   * 描边**既不能是 `border`，也不能是 `inset` 阴影**：
   * · `border` 让子元素裁到内边距盒、背景画到边框盒，角上露一条底色；
   * · `inset` 阴影画在**子元素下面**，封面盖住上半圈，
   *   只在底部信息条周围露出三条边 —— 看着像给那一条单独加了个框。
   * 外扩阴影贴边框盒外沿走，谁也盖不住。
   */
  it("卡片的描边是外扩阴影，不是 border 也不是 inset", () => {
    for (const sel of [
      ".anima-studio .studio-tile {",
      ".anima-studio .studio-tile:hover",
      '.anima-studio .studio-tile[data-active="true"]',
      '.anima-studio .studio-tile[data-menu-open="true"],',
    ]) {
      const body = ruleBody(sel);
      expect(body, `${sel} 还在用 border`).not.toMatch(/\bborder:\s/);
      expect(body, `${sel} 的描边是 inset，会被封面盖掉一半`).not.toMatch(
        /0 0 0 1px[^,;]*inset/,
      );
      expect(body, `${sel} 没有描边`).toMatch(/box-shadow:[\s\S]*0 0 0 1px/);
    }
  });

  /**
   * **卡片本身不能 `overflow:hidden`** —— 那样会把里面的下拉菜单
   * 一起裁掉，菜单只能在卡片内部弹，压在封面上。
   * 图片位自己切圆角（`.studio-tile-media`）。
   */
  it("卡片不裁溢出，图片位自己切", () => {
    expect(ruleBody(".anima-studio .studio-tile {")).not.toContain("overflow");
    const media = ruleBody(".anima-studio .studio-tile-media {");
    expect(media).toContain("overflow: hidden");
    expect(media).toContain("border-radius");
  });

  it("圆角半径两边对得上 —— 对不上就会露出一条边", () => {
    const tile = ruleBody(".anima-studio .studio-tile {").match(/border-radius:\s*(\d+)px/);
    const media = ruleBody(".anima-studio .studio-tile-media {").match(/border-radius:\s*(\d+)px/);
    expect(tile).not.toBeNull();
    expect(media).not.toBeNull();
    expect(media![1]).toBe(tile![1]);
  });

  it("每个卡片封面都带 studio-tile-media", () => {
    // 漏一个的话那张卡的图片会方角地盖出圆角外面。
    const bad: string[] = [];
    for (const f of tsxFiles(STUDIO_DIR)) {
      const src = readFileSync(f, "utf8");
      if (!src.includes("studio-tile")) continue;
      // 卡片里 aspect-* 的块就是图片位
      for (const m of src.matchAll(/className="([^"]*aspect-[^"]*)"/g)) {
        const cls = m[1];
        // 自己带圆角的（虚线占位块之类）不是卡片里的图片位
        if (/\brounded-/.test(cls) || cls.includes("h-full")) continue;
        if (!cls.includes("studio-tile-media")) {
          bad.push(`${f.replace(STUDIO_DIR, "")}: ${cls}`);
        }
      }
    }
    expect(bad, `图片位没切圆角：\n${bad.join("\n")}`).toEqual([]);
  });
});

describe("组件里不许写死颜色", () => {
  it("没有裸的十六进制色值", () => {
    // 写死的话必然在两套主题里的某一套下看不见 —— 而且不报错。
    const bad: string[] = [];
    for (const f of tsxFiles(STUDIO_DIR)) {
      const src = readFileSync(f, "utf8");
      for (const m of src.matchAll(/#[0-9a-fA-F]{3,8}\b/g)) {
        // SVG 的 currentColor 路径数据里不会有 #；注释里的示例排除掉
        const line = src.slice(src.lastIndexOf("\n", m.index) + 1, src.indexOf("\n", m.index));
        if (line.trim().startsWith("*") || line.trim().startsWith("//")) continue;
        bad.push(`${f.replace(STUDIO_DIR, "")}: ${m[0]}`);
      }
    }
    expect(bad, `写死的颜色：\n${bad.join("\n")}`).toEqual([]);
  });
});
