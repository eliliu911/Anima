import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

/**
 * **提示词编辑区要有高度上限。**
 *
 * 它是 `contentEditable`，只设了 `minHeight` 就会跟着内容无限长 ——
 * H3 的 Base 三段格式一千多字，实测把整个操作面板撑到
 * **底部那排参数（时长 / 画幅 / 生成按钮）被顶出屏幕**
 * （用户 2026-09-21 撞到）。
 */
const src = readFileSync(new URL("./PromptMentionEditor.tsx", import.meta.url), "utf8");

describe("提示词编辑区的高度", () => {
  it("有上限，而且超了是内部滚动", () => {
    expect(src).toContain("maxHeight: `${maxRows * 24}px`");
    expect(src).toContain('overflowY: "auto"');
  });

  it("上限可调，默认 10 行", () => {
    expect(src).toContain("maxRows = 10");
  });

  /** 没有 `nowheel` 的话，在编辑区里滚滚轮会变成缩放画布。 */
  it("带 nowheel —— 否则滚动变成缩放画布", () => {
    expect(src).toMatch(/nodrag nopan nowheel/);
  });

  /**
   * **横向必须显式 hidden。**
   *
   * CSS 规范：`overflow-x` / `overflow-y` 里只要有一个不是 `visible`，
   * 另一个的 `visible` 就**计算成 `auto`**。所以光写 `overflowY: "auto"`
   * 会顺带给出一条横向滚动条 —— 0.31.1 就是这么带出来的。
   * 文字本来 `pre-wrap` + `break-word` 会换行，横向永远不需要滚。
   */
  it("横向滚动显式关掉，不靠默认值", () => {
    // **钉样式对象里的相邻关系，不是钉那个字符串。**
    // 第一版写 `toContain('overflowX: "hidden"')` —— 而上面那段注释里
    // 也有这句话，删掉样式那行测试照样绿（变异当场抓出来的假绿）。
    expect(src).toMatch(/overflowY: "auto",\s*\n\s*overflowX: "hidden",/);
  });

  it("最小高度还留着 —— 空的时候不能塌成一条线", () => {
    expect(src).toContain("minHeight: `${rows * 24}px`");
  });
});
