import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const read = (rel: string) =>
  readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8")
    // 剥注释 —— 「文件里有这个字符串」不等于「界面上用了它」
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/^\s*\/\/.*$/gm, "");

const ROOT = fileURLToPath(new URL("./", import.meta.url));

function tsxFiles(dir = ROOT): string[] {
  const { readdirSync, statSync } = require("node:fs") as typeof import("node:fs");
  const { join } = require("node:path") as typeof import("node:path");
  return readdirSync(dir).flatMap((n: string) => {
    const p = join(dir, n);
    if (statSync(p).isDirectory()) return tsxFiles(p);
    return n.endsWith(".tsx") ? [p] : [];
  });
}

const SHELL = read("./SidePanel.tsx");
const PICKER = read("./AssetRefPicker.tsx");
const QUEUE = read("./jobs/QueuePanel.tsx");
const DRAWER = read("./PromptDrawer.tsx");

describe("右侧滑出面板（共用壳）", () => {
  /**
   * **必须 portal 到 body。**
   *
   * `position: fixed` 的参照物不一定是窗口 —— 祖先链上任何一个带
   * `transform` / `filter` / `backdrop-filter` / `contain` 的元素都会
   * 把它变成新的参照物，而 Studio 里这三样到处都是。
   *
   * 真出过事：资产选择器写成 `fixed inset-y-0 right-0` 放在镜头卡里，
   * 结果贴在了那张卡的右边、高度只有卡那么高 —— CSS 没错，参照物错了，
   * 而且不报任何错。
   */
  it("portal 出去，不受祖先影响", () => {
    expect(SHELL).toContain("createPortal(");
    expect(SHELL).toContain('querySelector(".anima-studio")');
  });

  it("贴窗口右边、占满整高", () => {
    expect(SHELL).toContain("fixed inset-y-0 right-0");
    expect(SHELL).toContain("<aside");
    expect(SHELL).toContain('role="region"');
  });

  /** 面板在右边，而鼠标多半停在左边的内容上 —— 三条出口都要有。 */
  it("点遮罩 / Esc / ✕ 都能关，且都走同一个入口", () => {
    const at = SHELL.indexOf('aria-hidden="true"');
    expect(at).toBeGreaterThan(0);
    expect(SHELL.slice(at, at + 260)).toContain("onClick={requestClose}");
    expect(SHELL).toContain('e.key === "Escape" && requestClose()');
    expect(SHELL).toContain('aria-label="关闭"');
    // ✕ 也走 requestClose，不是直接 onClose
    const x = SHELL.indexOf('aria-label="关闭"');
    expect(SHELL.slice(Math.max(0, x - 200), x)).toContain("onClick={requestClose}");
  });

  /**
   * **只有进场动画的话，关闭是「啪」地消失** —— 和滑进来完全不对称，
   * 看着像被强制关掉的。所以关闭要先播出场动画再卸载，
   * 而且**所有关闭路径都得走同一个入口**，否则会出现
   * 「有时候会滑、有时候不会」。
   */
  it("关闭先播出场动画再卸载", () => {
    expect(SHELL).toContain("studio-queue-out");
    expect(SHELL).toContain("studio-scrim-out");
    expect(SHELL).toContain("setTimeout(onClose, EXIT_MS)");
  });

  /** 面板里的内容（比如"选中一个资产就关"）也要能走带动画的关闭。 */
  it("给内容留了带动画关闭的通路", () => {
    expect(SHELL).toContain("useSidePanelClose");
    expect(PICKER, "选中资产之后走的是父组件的卸载，没有动画").toContain("close()");
  });

  /**
   * **两个面板必须共用这一个壳。**
   *
   * 各写一份的时候宽度、动画、遮罩、关闭方式全都对不齐 ——
   * 用户会以为它们是两种不同的东西。
   */
  it("队列、资产选择器、Prompt 抽屉都走它", () => {
    for (const [name, src] of [
      ["资产选择器", PICKER],
      ["任务队列", QUEUE],
      ["Prompt 抽屉", DRAWER],
    ] as const) {
      expect(src, `${name} 没用共用壳`).toContain("<SidePanel");
      expect(src, `${name} 还留着自己的 aside`).not.toContain("<aside");
      expect(src, `${name} 还留着自己的定位`).not.toContain("fixed inset-y-0");
    }
  });

  /**
   * **这条是全类扫描，不是点名。**
   *
   * 上面那份名单是逐个文件点名的，而 `PromptDrawer` 从来没被加进去 ——
   * 它就是这么漂走的：自己写了一份壳，宽度 380 vs 340、底色用
   * `--studio-bg-surface`、没有滑动、没有 Esc、而且**没有 portal**，
   * 于是 `position: fixed` 的参照物被祖先接管，面板不贴窗口顶边。
   * 用户一眼看出来「这个和 QUEUE 不是一个东西」。
   *
   * 点名的名单只能防住已知的那几个。新加一个面板照样会漏，
   * 所以这里扫全部 tsx。
   */
  it("studio 里没有第二份右侧面板的壳", () => {
    const offenders = tsxFiles()
      .filter((f) => !f.endsWith("SidePanel.tsx"))
      .filter((f) => {
        const src = read(f.slice(ROOT.length).replace(/^/, "./"));
        // 「贴右边、占满整高」是右侧滑出面板的指纹
        return /fixed\s+inset-y-0\s+right-0/.test(src);
      })
      .map((f) => f.slice(ROOT.length));
    expect(offenders, "这些文件自己写了一份右侧面板，改用 <SidePanel>").toEqual([]);
  });

  /** 换壳时最容易丢的就是键盘路径。 */
  it("选择器的键盘路径没丢", () => {
    expect(PICKER).toContain("autoFocus");
    expect(PICKER).toContain('e.key === "Enter"');
  });

  /** 侧边栏是整屏高的，列表该吃掉剩下的空间，不要再限死 vh。 */
  it("选择器列表撑满剩余高度", () => {
    expect(PICKER).not.toContain("max-h-[46vh]");
    expect(PICKER).toContain("min-h-0 flex-1 overflow-y-auto");
  });
});

describe("不许 portal 到 document.body", () => {
  /**
   * **所有 `--studio-*` 变量都定义在 `.anima-studio` 这个类上，不在 `:root`。**
   *
   * 挂到 body 之后 `var(--studio-bg-elevated)` 解析为空，
   * 整条 `background` 声明作废 —— 面板变成全透明，底下的内容直接透上来，
   * 遮罩也一起没了。实测过：`.anima-studio` 里是 `rgb(38,42,48)`，
   * 挂到 body 上是 `rgba(0,0,0,0)`。
   *
   * 这是"不报错"的一种：CSS 里一个无效的 `var()` 只是让那条声明消失。
   * 所以这条盯的是**整类**问题，不是某一个组件。
   */
  it("studio 里没有任何组件 portal 到 body", () => {
    const files = tsxFiles();
    expect(files.length, "扫了个空").toBeGreaterThan(20);
    // 直接读绝对路径 —— 绕回相对路径去拼 URL 只会把路径算错
    const bad = files
      .filter((f) => {
        const src = readFileSync(f, "utf8");
        return /createPortal\([\s\S]{0,400}document\.body/.test(src);
      })
      .map((f) => f.slice(ROOT.length));
    expect(bad, `这些组件挂到 body 上了，样式变量会全部解析为空：${bad.join(", ")}`).toEqual([]);
  });
});

describe("资产选择器只列本项目能用的", () => {
  /**
   * **`assetsList()` 不带参数列的是「所有项目的所有分集 + 全局」。**
   *
   * 于是分镜里会冒出别的片子的角色 —— 而插进去还真能解析
   * （`resolve_refs` 也扫所有作用域），出片时挂上一张完全不相干的脸，
   * 全程不报错。
   */
  /**
   * **全局库也不列。** 它是跨项目共享的池子，混在这一集的角色里只是噪音
   * （真事：一个别的片子的角色出现在猫的分镜里）。要用全局资产，
   * 先在第 3 步把它收进项目 —— 那一步本来就是干这个的。
   */
  it("传了项目就只取 本集 + 本项目", () => {
    const at = PICKER.indexOf("const load = projectId");
    expect(at, "没有按项目分支").toBeGreaterThan(0);
    const block = PICKER.slice(at, at + 700);
    expect(block).toContain('kind: "episode"');
    expect(block).toContain('kind: "project"');
    expect(block, "全局库不该列进来").not.toContain('kind: "global"');
  });

  /**
   * 通用创作台没有项目上下文，那里本来就是「随便挑一张参考图」的地方 ——
   * 缩成只剩全局库等于悄悄拿走了东西。
   */
  it("没有项目上下文时保持原样列全部", () => {
    const at = PICKER.indexOf("const load = projectId");
    const block = PICKER.slice(at, at + 900);
    expect(block, "没给无项目的情况留退路").toContain(": assetsList();");
  });
});
