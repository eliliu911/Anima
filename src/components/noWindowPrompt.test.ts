import { describe, expect, it } from "vitest";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

/** 把 `src/` 下所有 ts/tsx 走一遍。 */
function sources(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) sources(p, out);
    else if (/\.tsx?$/.test(name)) out.push(p);
  }
  return out;
}

/**
 * **`window.prompt` 在这个应用里永远不会弹框。**
 *
 * Tauri 的 WKWebView 没有实现它：不弹、不报错，直接返回 null。
 * 表现是按钮点下去**什么都不发生**，而且控制台干干净净。
 *
 * 已经踩过两次（剧本库的「新建项目」、LoRA 的说明和提示词模板），
 * 每次都是用户截图问「这个按钮是不是坏了」才发现的。
 * 写法本身看不出毛病，所以靠人记不住 —— 只能钉在这儿。
 *
 * 要就地填的时候用 `ComposingInput`（顺带把中文输入法也处理了）。
 * `confirm` 和 `alert` 不在此列 —— WKWebView 实现了那两个。
 */
describe("window.prompt", () => {
  it("全仓库不许再出现 —— 它在 WKWebView 里静默返回 null", () => {
    const offenders: string[] = [];
    for (const file of sources("src")) {
      if (file.endsWith("noWindowPrompt.test.ts")) continue;
      const text = readFileSync(file, "utf8");
      text.split("\n").forEach((line, i) => {
        // 注释里提这件事是允许的 —— 那些正是解释为什么不能用的地方。
        const code = line.replace(/\/\/.*$/, "").replace(/^\s*\*.*$/, "");
        if (/\bwindow\.prompt\s*\(/.test(code)) offenders.push(`${file}:${i + 1}`);
      });
    }
    expect(offenders, `这些地方点下去什么都不会发生：\n${offenders.join("\n")}`).toEqual([]);
  });
});
