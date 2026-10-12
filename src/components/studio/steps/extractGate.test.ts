import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

/**
 * **提取实体不能有前置门禁。**
 *
 * 用户粘的是没有格式的散文 —— 那是常态。早先的 `extract()` 先做结构派生，
 * 读不到就 `onError(...)` 直接返回，**压根不问模型**：
 * 于是正常输入点下去就是一行红字，而能正确处理它的代码就在下一个函数里
 * 躺着没人调。
 *
 * 渲染测不了（没有 jsdom），所以从源码上钉：`extract` 里必须调模型，
 * 且调用之前不能有 `return`。
 */
const SRC = readFileSync(
  fileURLToPath(new URL("./ScriptStep.tsx", import.meta.url)),
  "utf8",
);

const EXTRACT = (() => {
  const i = SRC.indexOf("const extract = async () => {");
  expect(i, "找不到 extract —— 改名了？").toBeGreaterThan(-1);
  return SRC.slice(i, SRC.indexOf("\n  };", i));
})();

describe("提取实体这条路", () => {
  it("真的会问模型", () => {
    expect(EXTRACT).toContain("cinemaExtractEntities");
  });

  /**
   * 门禁的形状就是「调模型之前先 return」。只要模型调用不是这个函数里
   * 第一个有意义的动作之前有早退，粘散文就会被挡。
   */
  it("问模型之前没有早退", () => {
    const before = EXTRACT.slice(0, EXTRACT.indexOf("cinemaExtractEntities"));
    expect(before, `模型调用之前有 return：\n${before}`).not.toMatch(/^\s+return;/m);
  });

  it("不再拿「读不出结构」当错误", () => {
    expect(SRC).not.toContain("文档里没有可以直接读出来的角色和场景");
  });

  it("跑的时候有状态可看 —— 20~60 秒的等待不能是静默的", () => {
    expect(SRC).toContain("智能分析中");
    expect(EXTRACT).toContain("setExtracting(true)");
  });

  /** 第 1 步唯一的动作，按钮不该在任何情况下变成次要样式。 */
  it("按钮一直是主色，不因为读不出结构而变灰", () => {
    // 锚在**按钮那一行**上。`indexOf("提取实体")` 命中的是文件头注释里
    // 的那一处 —— 今天第三次栽在 indexOf 拿到第一个而不是想要的那个上了。
    const i = SRC.indexOf('{extracting ? "智能分析中…" : "提取实体"}');
    expect(i, "找不到按钮那一行").toBeGreaterThan(-1);
    const block = SRC.slice(Math.max(0, i - 1100), i);
    expect(block).toContain('background: text.trim() ? "var(--studio-primary)"');
    expect(block, "还在按 weak 决定按钮颜色").not.toContain("!weak");
  });
});
