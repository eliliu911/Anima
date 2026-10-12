import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { STEPS } from "./StepRail";

/**
 * 步骤条的图标和滑动条。
 *
 * 这两样都渲染不出来测（没有 jsdom），所以从源码上钉住那几条
 * **错了不报错、只是看着不对** 的性质。
 */
const SRC = readFileSync(
  fileURLToPath(new URL("./StepRail.tsx", import.meta.url)),
  "utf8",
);

describe("步骤图标", () => {
  it("每一步都有图标 —— 少一个就是一个空位", () => {
    const block = SRC.slice(SRC.indexOf("const ICONS"), SRC.indexOf("export const STEPS"));
    for (const s of STEPS) {
      expect(block, `${s.id} 没有图标`).toContain(`${s.id}:`);
    }
  });

  /** 图标之间不能重合 —— 六个一样的形状等于没有图标。 */
  it("六个图标各不相同", () => {
    const block = SRC.slice(SRC.indexOf("const ICONS"), SRC.indexOf("export const STEPS"));
    const paths = [...block.matchAll(/"(M[^"]{10,})"/g)].map((m) => m[1]);
    expect(paths).toHaveLength(STEPS.length);
    expect(new Set(paths).size, "有重复的图标路径").toBe(STEPS.length);
  });

  /** 用 `currentColor` 才能跟着深浅主题走；写死颜色在另一个主题下会消失。 */
  it("图标描边用 currentColor", () => {
    expect(SRC).toContain('stroke="currentColor"');
  });
});

describe("滑动选中条", () => {
  /**
   * **位置要量出来，不能按「序号 × 行高」算。**
   * 选中项比其它项高（多一行状态字），算出来的必然对不齐，
   * 而且越往下偏得越多 —— 不报错，只是条子停在错的地方。
   */
  it("位置来自实际元素的 offsetTop / offsetHeight", () => {
    expect(SRC).toContain("el.offsetTop");
    expect(SRC).toContain("el.offsetHeight");
  });

  it("步骤数和状态变化时重新量", () => {
    // 锚在**调用**上，不是 import 上 —— 第一版查 `indexOf("useLayoutEffect")`，
    // 命中的是文件第一行的 import，切出来的当然不含依赖数组。
    const i = SRC.indexOf("useLayoutEffect(() => {");
    // `+ 3` 把 `]);` 也切进来 —— 不带的话字符串结尾是 `states` 而不是
    // `states]`，正则永远匹配不上：**对正确代码也是红的**。
    const deps = SRC.slice(i, SRC.indexOf("]);", i) + 3);
    expect(deps, `依赖里缺东西：${deps.slice(-120)}`).toMatch(/\[active, steps\.length, states\]/);
  });

  it("有过渡动画 —— 否则就是瞬移，跟原来的没区别", () => {
    expect(SRC).toMatch(/transition:\s*"top \d+ms/);
  });

  /** 旧的做法是给每个按钮加 `border-left`，两套并存会看到两条。 */
  it("旧的逐项 borderLeft 已经拆掉", () => {
    expect(SRC).not.toContain("borderLeft: `2px solid");
  });
});
