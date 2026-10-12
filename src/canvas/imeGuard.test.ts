import { readFileSync, readdirSync } from "node:fs";
import { describe, expect, it } from "vitest";

/**
 * **中文打不进去的输入框，扫一遍。**
 *
 * # 这个坑犯过三次
 *
 * 直接 `value={x} onChange={e => 存起来(e.target.value)}` 的输入框，
 * 用拼音打中文会打出 `qqiqinqinaqin'ai` 这种串：每敲一个字母都往上写一次，
 * 父组件把存下来的值回灌进 `value`，那次重渲染把输入法的**组合缓冲**冲掉。
 *
 * 英文察觉不到（一个字母就是一个字符），所以这类 bug **只有真的用中文
 * 打字才会发现** —— 而这是个中文工具。
 *
 * `ComposingInput` / `ComposingTextarea` 的模块注释里写着「新加输入框时：
 * 能用这个就用这个」。2026-09-20 又犯了一次（试音色节点）——
 * **一个每次都要记得的约定等于没有约定**，所以改成测试拦。
 */
const ROOT = new URL("./", import.meta.url);

function tsxFiles(dir: URL): URL[] {
  const out: URL[] = [];
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const u = new URL(e.name + (e.isDirectory() ? "/" : ""), dir);
    if (e.isDirectory()) out.push(...tsxFiles(u));
    else if (e.name.endsWith(".tsx")) out.push(u);
  }
  return out;
}

/**
 * 一个标签的完整文本。**不能用非贪婪正则截到第一个 `>`** ——
 * `onChange={(e) => …}` 里的箭头就带着一个 `>`，截到那儿会把后面的
 * `onCompositionStart` 漏掉，于是写对了的组件被当成违规（第一版就是这样）。
 * 按花括号深度扫。
 */
function tagAt(src: string, from: number): string {
  let depth = 0;
  for (let i = from; i < src.length; i++) {
    const c = src[i];
    if (c === "{") depth++;
    else if (c === "}") depth--;
    else if (c === ">" && depth === 0) return src.slice(from, i + 1);
  }
  return src.slice(from);
}

/**
 * 会出拼音串的输入框。判据两条都要满足：
 *
 * 1. `onChange` 里**直接往画布 store 写**（`updateNodeData` 之类）——
 *    本地 `useState` 的输入框不会被回灌，中文打得好好的，不该拦；
 * 2. 没有自己处理组合态（手写 `onCompositionStart` 也算写对了）。
 */
function offenders(src: string): string[] {
  const bad: string[] = [];
  for (const m of src.matchAll(/<(input|textarea)\b/g)) {
    const tag = tagAt(src, m.index!);
    if (!/\bvalue=/.test(tag) || !/\bonChange=/.test(tag)) continue;
    if (/type="(range|checkbox|radio|color|number|file)"/.test(tag)) continue;
    if (/onCompositionStart/.test(tag)) continue;
    if (!/updateNodeData\(|updateProjectNodeData\(|useCanvasStore\.getState\(\)\.update/.test(tag)) continue;
    bad.push(tag.replace(/\s+/g, " ").slice(0, 110));
  }
  return bad;
}

describe("中文输入法", () => {
  it("扫描逻辑本身有效（不然下面那条会假绿）", () => {
    expect(offenders('<input value={x} onChange={(e) => updateNodeData(id, { a: e.target.value })} />')).toHaveLength(1);
    // **箭头函数里的 `>` 不能把标签截断** —— 截断的话写对了的组件会被当成违规
    expect(offenders('<textarea value={x} onChange={(e) => updateNodeData(id, {})} onCompositionStart={g} />')).toHaveLength(0);
    expect(offenders('<input type="range" value={x} onChange={(e) => updateNodeData(id, {})} />')).toHaveLength(0);
    // 本地 useState 的输入框不会被回灌，中文打得好好的，不该拦
    expect(offenders('<input value={query} onChange={(e) => setQuery(e.target.value)} />')).toHaveLength(0);
    expect(offenders('<input value={x} />')).toHaveLength(0);
  });

  it("画布里没有会把输入法缓冲冲掉的输入框", () => {
    const hits: string[] = [];
    for (const f of tsxFiles(ROOT)) {
      for (const tag of offenders(readFileSync(f, "utf8"))) {
        hits.push(`${f.pathname.split("/canvas/")[1]}：${tag}`);
      }
    }
    expect(
      hits,
      `这些输入框打中文会出拼音串，改用 ComposingInput / ComposingTextarea：\n${hits.join("\n")}`,
    ).toEqual([]);
  });
});
