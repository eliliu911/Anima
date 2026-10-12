import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";

const read = (p: string) => readFileSync(new URL(`../../${p}`, import.meta.url), "utf8");

/**
 * 默认 System Prompt **抄在三处**：`store/chat.ts` 的
 * `DEFAULT_SYSTEM_PROMPT`、以及两份语言包的 `settings.defaultPrompt`。
 *
 * 这个仓库栽在「一份清单抄在两处」上已经不止一次。这里不强求三份逐字相同
 * （中英文本来就不同），但**能力声明必须同步** —— 2026-09-08 删掉哼唱时
 * 就漏了这两处：工具没了，提示词还在告诉模型「你会唱歌，用 sing 工具」，
 * 于是它会去调一个不存在的工具，而这条路不报错、只表现为答非所问。
 */
describe("默认 System Prompt 的三份副本", () => {
  const sources: [string, string][] = [
    ["store/chat.ts", read("src/store/chat.ts")],
    ["zh-CN/settings.json", read("src/locales/zh-CN/settings.json")],
    ["en-US/settings.json", read("src/locales/en-US/settings.json")],
  ];

  /**
   * 提示词里不能宣传**已经不存在的工具**。
   *
   * 名单是「删掉过的能力」，不是「所有工具」—— 后者会在每次加工具时误报。
   */
  it("不宣传已经删掉的能力", () => {
    const gone = ["sing tool", "sing 工具", "你会唱歌", "You can sing"];
    for (const [name, body] of sources) {
      for (const g of gone) {
        expect(body, `${name} 里还写着「${g}」，但那个工具已经删了`).not.toContain(g);
      }
    }
  });

  /** 三份都还在 —— 少一份的表现是某个语言下提示词整个空掉。 */
  it("三份都存在且非空", () => {
    expect(sources[0][1]).toContain("DEFAULT_SYSTEM_PROMPT");
    for (const [name, body] of sources.slice(1)) {
      expect(body, `${name} 少了 defaultPrompt`).toContain('"defaultPrompt"');
    }
  });
});
