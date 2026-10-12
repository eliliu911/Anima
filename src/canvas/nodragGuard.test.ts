import { readFileSync, readdirSync } from "node:fs";
import { describe, expect, it } from "vitest";

/**
 * **画布节点里能点的东西，必须带 `nodrag`。**
 *
 * # 不加会怎样
 *
 * ReactFlow 默认把节点上的 pointerdown 当成「开始拖这个节点」。
 * 下拉框最明显：**菜单打不开，节点跟着鼠标到处跑**
 * （2026-09-20 用户在试音色节点上撞到）。
 * 输入框是光标定位不了、选不中文字；滑杆是拖不动。
 *
 * # 为什么要看祖先
 *
 * 加在外层容器上一样有效（`ImageGenNode` 就是这么做的）。
 * 只看标签自己的话，那些靠容器解决的会被误报 —— 而**误报的护栏
 * 迟早被人加进白名单，然后就等于没有护栏**。所以这里按 JSX 嵌套扫，
 * 元素自己或任一祖先带 `nodrag` 都算过。
 */
const ROOT = new URL("./", import.meta.url);
/**
 * **只管这三种。** 按钮不在里面：拖起来只要指针没移动，pointerup 照样
 * 触发 click，所以它们本来就能用 —— 把它们算进来会逼着去改一大片
 * 能用的 代码，而护栏一旦开始误报就会被加白名单，然后等于没有。
 *
 * 另外这个扫描是**逐文件**看祖先的，浮层面板（`panels/`）的 `nodrag`
 * 可能加在父节点那个文件里，跨文件看不到 —— 又一条只收窄到这三种的理由。
 */
const INTERACTIVE = /^(select|textarea|input)$/;

function tsxFiles(dir: URL): URL[] {
  const out: URL[] = [];
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const u = new URL(e.name + (e.isDirectory() ? "/" : ""), dir);
    if (e.isDirectory()) out.push(...tsxFiles(u));
    else if (e.name.endsWith(".tsx")) out.push(u);
  }
  return out;
}

/** 从 `<` 扫到属于这个标签的 `>`。**按花括号深度** —— `(e) => …` 里的箭头带一个 `>`。 */
function tagEnd(src: string, from: number): number {
  let depth = 0;
  for (let i = from; i < src.length; i++) {
    const c = src[i];
    if (c === "{") depth++;
    else if (c === "}") depth--;
    else if (c === ">" && depth === 0) return i;
  }
  return src.length - 1;
}

/** 缺 `nodrag` 的可交互元素。按 JSX 嵌套算祖先。 */
export function missingNodrag(src: string): string[] {
  const bad: string[] = [];
  const stack: { name: string; nodrag: boolean }[] = [];
  const re = /<(\/?)([A-Za-z][\w.]*)/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(src))) {
    const [, slash, name] = m;
    if (slash) {
      // 关闭：弹到同名那一层为止（JSX 必配对，弹一层就够）
      for (let i = stack.length - 1; i >= 0; i--) {
        if (stack[i].name === name) { stack.length = i; break; }
      }
      continue;
    }
    const end = tagEnd(src, m.index);
    const tag = src.slice(m.index, end + 1);
    const selfClosing = src[end - 1] === "/";
    const nodrag = /\bnodrag\b/.test(tag);
    const covered = nodrag || stack.some((s) => s.nodrag);
    if (INTERACTIVE.test(name) && !covered) {
      bad.push(`<${name} ${tag.replace(/\s+/g, " ").slice(name.length + 1, 90)}`);
    }
    if (!selfClosing) stack.push({ name, nodrag });
    re.lastIndex = end;
  }
  return bad;
}

describe("画布节点里的可交互元素", () => {
  it("扫描逻辑本身有效（不然下面那条会假绿）", () => {
    expect(missingNodrag('<div><select value={x} /></div>')).toHaveLength(1);
    // 自己带
    expect(missingNodrag('<div><select className="nodrag" value={x} /></div>')).toHaveLength(0);
    // 祖先带 —— 这一条是不能只看标签自己的理由
    expect(missingNodrag('<div className="nodrag nopan"><label><input type="checkbox" /></label></div>')).toHaveLength(0);
    // 容器关掉之后不再算被罩住
    expect(missingNodrag('<div className="nodrag"><b/></div><select value={x} />')).toHaveLength(1);
    // 按钮不在管辖范围里
    expect(missingNodrag('<div><button onClick={f}>x</button></div>')).toHaveLength(0);
    // `(e) => …` 里的 `>` 不能把标签截断
    expect(missingNodrag('<select onChange={(e) => f(e)} className="nodrag" />')).toHaveLength(0);
  });

  it("每个能点的东西都被 nodrag 罩着", () => {
    const hits: string[] = [];
    for (const f of tsxFiles(new URL("nodes/", ROOT))) {
      for (const t of missingNodrag(readFileSync(f, "utf8"))) {
        hits.push(`${f.pathname.split("/canvas/")[1]}：${t}`);
      }
    }
    expect(
      hits,
      `这些控件点下去会变成拖节点（下拉框最明显：菜单打不开、节点跟着鼠标跑）：\n${hits.join("\n")}`,
    ).toEqual([]);
  });
});

/**
 * 画布节点里的 `<video controls>` 必须带 `nodrag`。
 *
 * 不带的话，拖进度条会被 ReactFlow 的拖拽处理器抢走指针：滑块永远跟着鼠标、
 * 节点切不了、菜单点不动，**只能强杀进程**（用户 2026-09-22 在深度动作捕捉节点上撞到）。
 *
 * 判据钉在 `MediaThumb` 上而不是各个节点上：整条画布里 `<video>` 只有那一处，
 * 而靠调用方在 className 里记得补 —— `CanvasMediaGallery` 补了，
 * `DepthCaptureNode` 和 `UpscaleNode` 没补，于是就出事了。
 */
describe("画布里的视频进度条", () => {
  const thumb = readFileSync(new URL("../components/studio/MediaThumb.tsx", import.meta.url), "utf8");

  it("MediaThumb 的 video 分支无条件加 nodrag nopan", () => {
    expect(thumb).toContain('className={`nodrag nopan ${className ?? ""}`}');
  });

  /** 整条画布只有这一个 video 元素 —— 多出来一个就说明有人又手写了一份。 */
  it("画布里没有别处手写 <video>", () => {
    const offenders = tsxFiles(new URL("./", import.meta.url))
      .filter((u) => readFileSync(u, "utf8").includes("<video"))
      .map((u) => u.pathname.split("/src/")[1]);
    expect(offenders).toEqual([]);
  });
});
