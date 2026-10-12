import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

/**
 * 编辑器不能有浏览器默认的蓝色聚焦框。
 *
 * **原来的规则写成了 `.script-doc :focus`（后代选择器）**，
 * 而 contenteditable 就是 `.script-doc` 本身 —— 于是那条规则盖的是
 * 里面的子元素，编辑器自己的蓝框一直在。看代码像是处理了，实际没有。
 */
const CSS = readFileSync(
  fileURLToPath(new URL("../../../studio-theme.css", import.meta.url)),
  "utf8",
);

describe("剧本编辑器的聚焦框", () => {
  it("元素自身的 :focus 被盖住了，不只是后代", () => {
    expect(CSS).toContain(".anima-studio .script-doc:focus");
  });

  it("focus-visible 也盖住 —— 键盘聚焦走的是这条", () => {
    expect(CSS).toContain(".anima-studio .script-doc:focus-visible");
  });

  it("这些选择器所在的块真的把 outline 关掉了", () => {
    const i = CSS.indexOf(".anima-studio .script-doc:focus");
    const block = CSS.slice(i, CSS.indexOf("}", i));
    expect(block, `没有 outline: none：${block}`).toMatch(/outline:\s*none/);
  });
});
