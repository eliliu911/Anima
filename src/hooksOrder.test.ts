import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

/**
 * **早退之后不许再调 hook。**
 *
 * React 靠调用顺序把 hook 和它的状态对上号。组件里先 `return`、下面还有
 * `useState` / `useEffect` 的话，走早退那条路时 hook 就少了几个 ——
 * React 抛 `Rendered more hooks than during the previous render`，
 * **整个窗口白屏**，不是某个区域坏掉。
 *
 * 这条只能从源码上钉。项目里没有 jsdom，渲染测不了；而且它只在
 * 「数据还没到」那条路上才炸 —— 开发时手头的项目多半是有数据的，
 * 一路点下来什么事都没有，直到用户打开一个空的。
 *
 * 真事：给剧本步骤加自动保存时，`useRef`/`useEffect` 插在了
 * `if (!project || !episode) return` 的下面，打开没有分集的项目直接白屏。
 *
 * # 怎么认
 *
 * 靠缩进，不配平花括号 —— JSX 里花括号太多，字符串和正则里的括号很容易
 * 把配平带偏。全仓库 prettier 2 空格，组件顶层语句一律在第 2 列，
 * 所以「顶层 if 块里的 return」和「顶层的 hook 调用」都能稳定认出来。
 *
 * **已知不覆盖**：`const X = () => {}` 形式声明的组件。目前这么写的只有
 * 一堆无状态图标和 `MarkdownRenderer`。下面有一条覆盖率断言盯着 ——
 * 真把组件改成箭头函数写法，那条会先红，而不是这份检查悄悄变成空转。
 */
const SRC = fileURLToPath(new URL(".", import.meta.url));

function tsxFiles(dir = SRC): string[] {
  return readdirSync(dir).flatMap((n) => {
    const p = join(dir, n);
    if (statSync(p).isDirectory()) return tsxFiles(p);
    return n.endsWith(".tsx") ? [p] : [];
  });
}

/** 第 0 列的函数声明。首字母大写才可能是组件（`useX` 也算 —— 自定义 hook）。 */
const FN_DECL = /^(?:export )?(?:default )?function ([A-Za-z_]\w*)/;
const IS_COMPONENT = /^(?:[A-Z]|use[A-Z])/;
/** 第 0 列的 `}` —— 一个顶层声明到此为止。 */
const FN_END = /^\}/;
/** `  if (…) {` 顶层条件块，和它的收口 `  }`。 */
const TOP_IF_OPEN = /^ {2}if \(.*\{\s*$/;
const TOP_CLOSE = /^ {2}\}/;
/** 块里的 return。 */
const NESTED_RETURN = /^ {4,}return\b/;
/** `  if (…) return …;` 单行版，和顶层无条件 `  return …`。 */
const ONE_LINE_RETURN = /^ {2}if \(.*\)\s*return\b/;
const TOP_RETURN = /^ {2}return\b/;
/** 顶层的 hook 调用：`  const x = useY(` / `  useEffect(`。 */
const TOP_HOOK = /^ {2}(?:(?:const|let) .*?=\s*)?(use[A-Z]\w*)\s*[(<]/;

export interface Violation {
  fn: string;
  hook: string;
  hookLine: number;
  returnLine: number;
}

export function scan(src: string): { violations: Violation[]; components: string[] } {
  const lines = src.split("\n");
  const violations: Violation[] = [];
  const components: string[] = [];
  let fn = "";
  let returnedAt = 0; // 0 = 这个函数里还没早退
  let inTopIf = false;
  /**
   * 第 0 列上的**工具函数**要整段跳过。有的文件把它们嵌在组件体里却顶格写，
   * 于是它们自己的 `return`（正好在第 2 列）会被当成组件的早退 —— 一整片误报。
   */
  let skipping = false;

  for (const [i, line] of lines.entries()) {
    if (skipping) {
      if (FN_END.test(line)) skipping = false;
      continue;
    }
    const decl = FN_DECL.exec(line);
    if (decl) {
      if (IS_COMPONENT.test(decl[1])) {
        fn = decl[1];
        components.push(fn);
        returnedAt = 0;
        inTopIf = false;
      } else {
        skipping = true;
      }
      continue;
    }
    if (!fn) continue;
    if (FN_END.test(line)) {
      fn = "";
      continue;
    }

    if (TOP_IF_OPEN.test(line)) {
      inTopIf = true;
      continue;
    }
    if (inTopIf) {
      if (returnedAt === 0 && NESTED_RETURN.test(line)) returnedAt = i + 1;
      if (TOP_CLOSE.test(line)) inTopIf = false;
      continue;
    }
    if (returnedAt === 0) {
      if (ONE_LINE_RETURN.test(line) || TOP_RETURN.test(line)) returnedAt = i + 1;
      continue;
    }

    const m = TOP_HOOK.exec(line);
    if (m) violations.push({ fn, hook: m[1], hookLine: i + 1, returnLine: returnedAt });
  }
  return { violations, components };
}

describe("hooks 不能出现在早退之后", () => {
  it("认得出真实的那次事故", () => {
    // 就是把自动保存的两个 hook 放到守卫下面的那一版
    const broken = `
export function ScriptStep({ project, episode }: Props) {
  const [text, setText] = useState("");

  if (!project || !episode) {
    return <Empty>还没有分集</Empty>;
  }

  const saveRef = useRef<() => Promise<void>>(async () => {});
  useEffect(() => {
    setText("");
  }, [text]);
}
`;
    const { violations } = scan(broken);
    expect(violations.map((v) => v.hook)).toEqual(["useRef", "useEffect"]);
    expect(violations[0].returnLine).toBe(6);
  });

  it("顺序对了就不报", () => {
    const ok = `
export function ScriptStep({ project, episode }: Props) {
  const [text, setText] = useState("");
  const saveRef = useRef<() => Promise<void>>(async () => {});
  useEffect(() => {
    setText("");
  }, [text]);

  if (!project || !episode) {
    return <Empty>还没有分集</Empty>;
  }
  return <div />;
}
`;
    expect(scan(ok).violations).toEqual([]);
  });

  it("顶格写的工具函数：不算早退，而且它后面要接着扫", () => {
    // CodeViewerPanel 里真有这么一段：工具函数嵌在组件体里却写在第 0 列，
    // 它自己的 `return ""` 正好落在第 2 列上。
    //
    // **这里钉的是「接着扫」，不是「不误报」。** 只求不误报的话，
    // 「碰到第 0 列的 `}` 就清空当前函数」也能做到 —— 代价是工具函数
    // 之后的半个组件再也不被检查，而测试照样全绿。
    const src = `
export function Panel() {
  const a = useMemo(() => 1, []);
  if (!a) {
    return null;
  }

function helper(x: string): string {
  if (!x) return "";
  return x;
}

  const b = useMemo(() => 2, []);
}
`;
    const { violations } = scan(src);
    expect(violations).toHaveLength(1);
    expect(violations[0].hook).toBe("useMemo");
    // 早退算的是 Panel 的守卫（第 5 行），不是 helper 里那句
    expect(violations[0].returnLine).toBe(5);
    expect(violations[0].fn).toBe("Panel");
  });

  it("组件结束之后的东西不算在它头上", () => {
    // 箭头函数写的组件这份检查认不出来（见文件头的「已知不覆盖」）。
    // 认不出可以，**记到上一个组件账上不行** —— 那是纯粹的假阳性，
    // 而且指的行号在另一个组件里，看的人会以为自己疯了。
    const src = `
export function Panel() {
  const a = useMemo(() => 1, []);
  if (!a) {
    return null;
  }
  return <div />;
}

const Other = () => {
  const b = useState(0);
  return <div />;
};
`;
    expect(scan(src).violations).toEqual([]);
  });

  /**
   * **覆盖率断言。** 上面那套正则只认 `function Foo()` 这一种写法。
   * 哪天代码风格换成箭头函数，这份检查会一声不响地变成空转 ——
   * 95 个文件全过，因为一个组件都没认出来。所以钉一个下限。
   */
  it("确实认出了大量组件 —— 不是在空转", () => {
    const files = tsxFiles();
    expect(files.length).toBeGreaterThan(80);
    const total = files.reduce((n, f) => n + scan(readFileSync(f, "utf8")).components.length, 0);
    expect(total).toBeGreaterThan(120);
  });

  it.each(tsxFiles().map((f) => [f.slice(SRC.length), f]))("%s", (_name, file) => {
    const { violations } = scan(readFileSync(file, "utf8"));
    const detail = violations
      .map((v) => `  ${v.fn}: 第 ${v.hookLine} 行 ${v.hook}（早退在第 ${v.returnLine} 行）`)
      .join("\n");
    expect(violations, `早退之后还在调 hook：\n${detail}`).toEqual([]);
  });
});
