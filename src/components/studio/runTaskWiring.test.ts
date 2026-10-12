import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const read = (rel: string) =>
  readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8")
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/^\s*\/\/.*$/gm, "");

/**
 * 所有生成统一走右下角通知条。这几处是前端直接等结果的生成：
 * 不走 runTask 的话，切走页面结果就没人报了，而且又回到各页各写各的提示。
 */
describe("前端生成都走 runTask", () => {
  const cases: [string, RegExp][] = [
    ["./steps/StyleStep.tsx", /runTask\([\s\S]*?\(\) => styleAnalyze\(/],
    ["./steps/CharacterWorkbench.tsx", /runTask\([\s\S]*?\(\) => cinemaGenerateCharacterSheetViaComfyui\(/],
  ];
  for (const [file, pattern] of cases) {
    it(file, () => expect(read(file)).toMatch(pattern));
  }

  /** 创作台保留画廊，通知条是加上去的。引擎返回 ok=false 不抛错，得自己抛才会显示失败。 */
  it("./AnimaStudio.tsx 每张都走，引擎说没成也要报失败，且不盖掉画廊里的备注", () => {
    const src = read("./AnimaStudio.tsx");
    expect(src).toMatch(/runTask\([\s\S]*?const out = await studioGenerate\(/);
    expect(src).toContain('if (!out.ok) throw out.message');
    expect(src).toContain('if (cur?.status !== "failed")');
  });

  it("./script/ScriptsPage.tsx 两处生成都走", () => {
    const src = read("./script/ScriptsPage.tsx");
    expect(src.match(/\(\) => generateScreenplay\(/g)).toHaveLength(2);
    expect(src.match(/await generateScreenplay\(/g)).toBeNull();
  });
});
