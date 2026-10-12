/**
 * **渲染时就跑的回调，不能引用后面才声明的 const。**
 *
 * # 为什么要有这一条
 *
 * 2026-09-20 出过事：`AssetWorkbench` 里写了
 *
 * ```ts
 * const initialTemplate = useMemo(() => f(engine), [asset.id]);   // 第 179 行
 * // ...
 * const [engine, setEngine] = useState(eff.engine);               // 第 236 行
 * ```
 *
 * 点开任意一张角色卡就白屏，报 `Cannot access 'ge' before initialization`。
 *
 * **`tsc` 拦不住这一类。** 变量是在闭包里用的，编译器没法断定闭包什么时候跑，
 * 所以 `npm run build`（含 `tsc --noEmit`）一路绿灯 —— 构建成功、
 * 单元测试全绿、只有真点进那个界面才炸。这正是这个项目最典型的失败形状。
 *
 * # 只盯两种回调
 *
 * `useMemo(...)` 和 `useState(() => ...)` 的回调**在渲染时同步执行**，
 * 所以它们读到的后声明 const 一定在 TDZ 里。
 *
 * `useCallback` / `useEffect` / 事件处理函数**不算** —— 它们晚于整个函数体运行，
 * 那时候所有 const 都初始化好了，引用后面的声明是完全合法的（而且很常见）。
 * 把它们也算进来的话，这条测试会变成一串噪音，然后被人加白名单关掉。
 */
import { readFileSync, readdirSync, statSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const SRC = fileURLToPath(new URL("../src", import.meta.url));

function tsxFiles(dir: string, out: string[] = []): string[] {
  for (const e of readdirSync(dir)) {
    const p = join(dir, e);
    if (statSync(p).isDirectory()) tsxFiles(p, out);
    else if (p.endsWith(".tsx") && !p.includes(".test.")) out.push(p);
  }
  return out;
}

/** 组件顶层的 `const`/`let` 声明：名字 → 行号（0 基）。 */
function topLevelDecls(lines: string[]): Map<string, number> {
  const out = new Map<string, number>();
  // **只认缩进两格的** —— 那是组件函数体的顶层，也就是渲染时会跑到的那一层。
  // 认更深的缩进会把回调内部自己的局部变量也收进来。
  const decl = /^ {2}(?:const|let)\s+(\[[^\]]*\]|\{[^}]*\}|[A-Za-z_$][\w$]*)\s*(?::[^=]+)?=/;
  lines.forEach((line, i) => {
    const m = decl.exec(line);
    if (!m) return;
    for (const name of m[1].replace(/[[\]{}]/g, " ").split(",")) {
      const id = name.split(":").pop()!.replace(/\.\.\./, "").trim();
      if (/^[A-Za-z_$][\w$]*$/.test(id) && !out.has(id)) out.set(id, i);
    }
  });
  return out;
}

/** 从 `at` 处的 `(` 开始，找配平的 `)`。回它的下标；找不到回 -1。 */
function matchParen(src: string, at: number): number {
  let depth = 0;
  for (let i = at; i < src.length; i++) {
    if (src[i] === "(") depth++;
    else if (src[i] === ")" && --depth === 0) return i;
  }
  return -1;
}

describe("渲染时就跑的回调", () => {
  it("不引用后面才声明的 const", () => {
    const bad: string[] = [];
    for (const file of tsxFiles(SRC)) {
      const src = readFileSync(file, "utf8");
      const lines = src.split("\n");
      const decls = topLevelDecls(lines);
      // 每个字符落在第几行
      const lineOf: number[] = [];
      let ln = 0;
      for (const ch of src) {
        lineOf.push(ln);
        if (ch === "\n") ln++;
      }
      // **泛型参数要放行。** 第一版写的是 `\buseMemo\(` —— 而真正出事的那一句是
      // `useMemo<TemplateId>(`，正则压根没匹配上，测试对着 bug 报绿。
      for (const m of src.matchAll(
        /\buseMemo\s*(?:<[^>]*>)?\s*\(|\buseState\s*(?:<[^>]*>)?\s*\(\s*\(\s*\)\s*=>/g,
      )) {
        const open = src.indexOf("(", m.index!);
        const close = matchParen(src, open);
        if (close < 0) continue;
        const body = src.slice(open, close);
        const at = lineOf[m.index!];
        // 回调自己声明的名字不算
        const own = new Set(
          [...body.matchAll(/\b(?:const|let|function)\s+([A-Za-z_$][\w$]*)/g)].map((x) => x[1]),
        );
        for (const t of body.matchAll(/(^|[^.\w$])([A-Za-z_$][\w$]*)/g)) {
          const id = t[2];
          if (own.has(id)) continue;
          const declAt = decls.get(id);
          if (declAt !== undefined && declAt > at) {
            bad.push(
              `${file.slice(SRC.length + 1)}:${at + 1} 的回调用了「${id}」，` +
                `而它在第 ${declAt + 1} 行才声明 —— 渲染时会落进 TDZ`,
            );
          }
        }
      }
    }
    expect(bad, bad.join("\n")).toEqual([]);
  });
});
