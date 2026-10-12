import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { parseScript } from "./format";

/**
 * **提示词产出的格式，编辑器必须认得。**
 *
 * 这一段是 2026-09-11 用 `SCREENPLAY_FROM_SOURCE` 打真实模型
 * （Azure gpt-4o）拿回来的原始输出，只剥掉了代码围栏。
 *
 * 测的是「提示词写出去的格式，行识别接不接得住」—— 自己编的样例只能
 * 证明解析器自洽，证明不了模型会那么写。而格式错了**不报错**，
 * 只表现为排版乱、后面抽角色和分镜跟着乱。
 */
describe("原文→剧本 提示词产出的格式", () => {
  const text = readFileSync("src-tauri/tests/fixtures/screenplay_llm_output.txt", "utf8");
  const lines = parseScript(text);

  it("场景标题认得出", () => {
    const heads = lines.filter((l) => l.type === "sceneHeading");
    expect(heads.length).toBeGreaterThanOrEqual(1);
    expect(heads[0].text).toContain("乡间公路");
  });

  it("角色名和台词配成对，中间不会串成动作描写", () => {
    const cues = lines.filter((l) => l.type === "characterCue");
    expect(cues.length).toBeGreaterThanOrEqual(3);
    // 每个角色名后面必须跟着台词或语气提示 —— 跟着动作描写就说明空行规则错了，
    // 而那意味着这句台词不会被念出来
    for (let i = 0; i < lines.length; i += 1) {
      if (lines[i].type !== "characterCue") continue;
      const next = lines[i + 1];
      expect(next, `「${lines[i].text}」后面没有东西`).toBeTruthy();
      expect(["dialogue", "parenthetical"], `「${lines[i].text}」后面是 ${next.type}`)
        .toContain(next.type);
    }
  });

  it("语气提示认得出，不会被当成台词念出来", () => {
    expect(lines.some((l) => l.type === "parenthetical")).toBe(true);
  });

  it("动作描写落在 action，不会被误判成台词", () => {
    const actions = lines.filter((l) => l.type === "action");
    expect(actions.length).toBeGreaterThanOrEqual(3);
    expect(actions.some((l) => l.text.includes("行李箱"))).toBe(true);
  });

  it("角色名前后统一 —— 换个叫法下游就当成两个人", () => {
    const names = new Set(lines.filter((l) => l.type === "characterCue").map((l) => l.text.trim()));
    expect([...names].sort()).toEqual(["沈星晴", "老人"]);
  });
});
