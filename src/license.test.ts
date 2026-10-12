import { describe, expect, it } from "vitest";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

const ROOT = new URL("../", import.meta.url).pathname;
const read = (p: string) => readFileSync(join(ROOT, p), "utf8");

/** 递归列出 src 下所有 .ts/.tsx */
function walk(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(join(ROOT, dir))) {
    const rel = `${dir}/${name}`;
    if (statSync(join(ROOT, rel)).isDirectory()) walk(rel, out);
    else if (/\.tsx?$/.test(name)) out.push(rel);
  }
  return out;
}

/**
 * **NOTICE 里那份文件清单不能过期。**
 *
 * 项目是双许可，但有约 4200 行改写自 DramaClaw（ClaymoreLab, ELv2）、仍按 ELv2，
 * 而 ELv2 的限制随衍生作品传递 —— 所以 NOTICE 必须**准确**列出是哪些文件。
 *
 * 这是法律文件，过期比代码注释过期严重得多：
 * - 少列 → 有 ELv2 代码没被声明，违反 ELv2「不得遮掩声明」那条
 * - 多列 → 声称某文件是衍生品而其实不是，平白给自己套上限制
 *
 * 所以两边都从源头读：**NOTICE 的清单** vs **实际带声明的文件**。
 * 这个仓库栽在「一份清单抄在两处」上很多次了，法律文件更不能例外。
 */
describe("LICENSE 与 NOTICE", () => {
  const MARK = "ClaymoreLab, Elastic License 2.0";

  const declared = read("NOTICE")
    .split("\n")
    .map((l) => l.trim())
    .filter((l) => l.startsWith("src/") && /\.tsx?$/.test(l))
    .sort();

  const actual = walk("src")
    .filter((f) => !f.endsWith(".test.ts") && !f.endsWith(".test.tsx"))
    .filter((f) => read(f).includes(MARK))
    .sort();

  it("NOTICE 列的文件和实际带 ELv2 声明的文件完全一致", () => {
    expect(actual, "实际带声明的文件和 NOTICE 对不上").toEqual(declared);
  });

  it("清单不为空 —— 空了说明匹配逻辑坏了，而不是真没有", () => {
    expect(declared.length).toBeGreaterThan(0);
  });

  /** ELv2 的三条限制必须在 ELv2 全文里，别抄成残缺版本（公开版：ELv2 只管 NOTICE 里那些文件，全文在 LICENSES/ELv2.txt）。 */
  it("LICENSES/ELv2.txt 是完整的 ELv2", () => {
    const l = read("LICENSES/ELv2.txt");
    expect(l).toContain("Elastic License 2.0");
    expect(l, "缺「不得作为托管服务提供」那条").toContain("hosted or managed");
    expect(l, "缺「不得移除声明」那条").toContain("alter, remove, or obscure");
    expect(l, "缺专利条款").toContain("Patents");
  });

  /** 双许可：非商业 PolyForm Noncommercial 1.0.0 + 商业另行授权；ELv2 那部分是例外 */
  it("LICENSE 写明双许可和 ELv2 例外，许可全文都在", () => {
    const l = read("LICENSE");
    expect(l).toContain("SONG LIU");
    expect(l).toContain("PolyForm Noncommercial License 1.0.0");
    expect(l).toContain("commercial license");
    expect(l).toContain("Elastic License 2.0");
    expect(read("LICENSES/PolyForm-Noncommercial-1.0.0.txt")).toContain("PolyForm Noncommercial License 1.0.0");
  });

  /** NOTICE 要说清为什么那部分是 ELv2 —— 否则下一个人会想「一起换成双许可不就好了」。 */
  it("NOTICE 解释了 DramaClaw 那部分为什么是 ELv2", () => {
    const n = read("NOTICE");
    expect(n).toContain("DramaClaw");
    expect(n).toContain("SONG LIU");
  });
});
