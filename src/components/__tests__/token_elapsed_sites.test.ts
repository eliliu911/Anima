import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";

/**
 * **速率的分母修过一次，但只修了一处。**
 *
 * 上次改了 `AgentActivityTrajectory`，而用户实际看到的
 * `Working... 38s · ~469 tok · 12.3/s` 是 `StreamStatusBar` 渲染的 ——
 * 那一处没改，于是"跟 Anima 说完话后速率还在掉"照旧。
 * 469/38 = 12.3，正好是用整轮时长当分母算出来的。
 *
 * 这条测试盯着**所有**调用 `computeProgress` 且带流式的地方都传了
 * `tokenElapsed`。再加第三处渲染时，它会红。
 */
describe("token 速率的分母", () => {
  const dir = join(__dirname, "..");
  const sites = ["StreamStatusBar.tsx", "AgentActivityTrajectory.tsx"];

  /**
   * 取 `computeProgress(` 那一对括号里的内容。
   *
   * **不能只查文件里有没有 `tokenElapsed`** —— `const tokenElapsed = useTokenElapsed(...)`
   * 那行在，整份文件就永远含这个词，把调用处的参数删掉测试照样绿。
   * 初版就是这么写的，变异测试当场把它照出来了。
   */
  function callArgs(src: string): string {
    const i = src.indexOf("computeProgress(");
    if (i < 0) return "";
    let depth = 0;
    for (let j = i + "computeProgress".length; j < src.length; j++) {
      if (src[j] === "(") depth++;
      else if (src[j] === ")") {
        depth--;
        if (depth === 0) return src.slice(i, j + 1);
      }
    }
    return "";
  }

  it.each(sites)("%s 的 computeProgress 参数里带 tokenElapsed", (f) => {
    const src = readFileSync(join(dir, f), "utf8");
    const args = callArgs(src);
    expect(args, `${f} 里没找到 computeProgress 调用`).not.toBe("");
    expect(args, `${f} 调了 computeProgress 却没把 tokenElapsed 传进去`).toContain("tokenElapsed");
  });

  it("两处都用同一个 hook，不各写一份", () => {
    // 各写一份必然会漂 —— 这次就是漂出来的
    for (const f of sites) {
      const src = readFileSync(join(dir, f), "utf8");
      expect(src, `${f} 没用共用 hook`).toContain("useTokenElapsed");
    }
  });

  it("没有第三处在算 tok/s 却漏掉分母", () => {
    // 目录扫一遍：谁 render 了 tokensPerSec，谁就必须传 tokenElapsed
    const fs = require("node:fs") as typeof import("node:fs");
    const offenders: string[] = [];
    for (const f of fs.readdirSync(dir)) {
      if (!f.endsWith(".tsx") && !f.endsWith(".ts")) continue;
      const src = fs.readFileSync(join(dir, f), "utf8");
      if (src.includes("computeProgress(") && src.includes("streaming: true") && !callArgs(src).includes("tokenElapsed")) {
        offenders.push(f);
      }
    }
    expect(offenders, `这些地方算速率却没给分母: ${offenders.join(", ")}`).toEqual([]);
  });
});
