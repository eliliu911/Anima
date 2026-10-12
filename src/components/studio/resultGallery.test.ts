import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const read = (rel: string) =>
  readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8")
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/^\s*\/\/.*$/gm, "");

const GALLERY = read("./ResultGallery.tsx");
const STUDIO = read("./AnimaStudio.tsx");
const QUEUE = read("./ResultQueuePanel.tsx");

describe("创作台的结果区", () => {
  /**
   * **头部不能藏在「没有结果」的早退后面。**
   *
   * 原来 `results.length === 0` 直接 return 空状态，整条头部
   * （分类页签 + 视图切换 + 队列）都不渲染。而持久化一直没接，
   * 每次重开都是 0 条 —— 于是那几个控件永远不出现。
   * 「没有分类」「没有视图切换」「没有队列」三个现象，一个根因。
   */
  it("空状态也渲染头部控件", () => {
    const early = GALLERY.indexOf("if (results.length === 0) {");
    expect(early, "又把头部藏进早退里了").toBe(-1);
    expect(GALLERY).toContain("<header");
    // 头部在返回值里，且排在空状态之前
    expect(GALLERY.indexOf("<header")).toBeLessThan(GALLERY.indexOf("shownResults.length === 0"));
  });

  /** 「一条都没有」和「这一档没有」是两回事，混成一句会让人以为切页签就有。 */
  it("两种空状态分开说", () => {
    expect(GALLERY).toContain("results.length === 0 ? (");
    expect(GALLERY).toContain("切回「全部」");
  });

  it("有分类页签和视图切换", () => {
    for (const s of ["全部", "图片", "视频"]) expect(GALLERY).toContain(s);
    expect(GALLERY).toContain('title="网格"');
    expect(GALLERY).toContain('title="大图"');
  });

  /** 队列和分镜那边共用同一个壳，否则宽度、动画、关闭方式都会对不齐。 */
  it("队列走共用的 SidePanel", () => {
    expect(GALLERY).toContain("<ResultQueuePanel");
    expect(QUEUE).toContain("<SidePanel");
    expect(QUEUE, "自己写了个 aside").not.toContain("<aside");
  });

  /** 在跑的条数要显示在按钮上 —— 不显示的话没人会去点它。 */
  it("队列按钮带在跑的角标", () => {
    const at = GALLERY.indexOf('title="队列"');
    expect(at).toBeGreaterThan(0);
    expect(GALLERY.slice(at, at + 500)).toContain("running > 0");
  });

  /**
   * **持久化必须真的被调用。**
   *
   * Rust 侧的 `studio_results` / `studio_save_results` 连同
   * `reconcile_history` 早就写好并测过了 —— 但前端一行都没调过，
   * 所以每次重开结果都是空的。命令存在 ≠ 功能存在。
   */
  it("创作台接上了结果持久化", () => {
    expect(STUDIO).toContain("useResultPersistence()");
  });

  /** 失败原文要原样显示 —— 概括掉等于让人从头猜。 */
  it("队列里失败的带原文", () => {
    expect(QUEUE).toContain("{r.error}");
  });
});

describe("创作台左栏", () => {
  /**
   * **生成按钮不能跟着参数区一起滚走。**
   *
   * 它是这一页唯一那个"让事情发生"的控件。引擎参数一多就被推到屏幕外，
   * 每次都要先滚到底才点得到。LumenX 是钉在底部常驻的。
   */
  it("生成按钮在滚动区外面，常驻底部", () => {
    const scroll = STUDIO.indexOf("studio-scroll flex min-h-0 flex-1");
    expect(scroll, "左栏没有独立的滚动区").toBeGreaterThan(0);
    const btn = STUDIO.indexOf('className="studio-primary-btn');
    expect(btn, "找不到生成按钮").toBeGreaterThan(0);
    // 按钮在滚动区的闭合之后
    const scrollEnd = STUDIO.indexOf("</div>\n\n          {cat && (", scroll);
    expect(scrollEnd, "滚动区没闭合在按钮之前").toBeGreaterThan(0);
    expect(btn).toBeGreaterThan(scrollEnd);
    expect(STUDIO.slice(btn - 200, btn + 200)).toContain("w-full");
  });
});

describe("创作台的标语", () => {
  /**
   * **`defaultValue` 只在 key 不存在时才生效。**
   *
   * `studio:subtitle` 在 locale 里的值是旧短剧面板的标语
   * （「超写实数字人全套制造工厂…」）—— 创作台代码里写着
   * `defaultValue: "本机 ComfyUI · 不绑定项目"`，但因为 key 存在，
   * 那句默认值永远不会出现。两个不同的界面共用一个 key，
   * 而 i18n 不会因此报任何错。
   */
  it("标语说的是创作台自己的事", () => {
    const zh = JSON.parse(
      readFileSync(fileURLToPath(new URL("../../locales/zh-CN/studio.json", import.meta.url)), "utf8"),
    ) as Record<string, string>;
    expect(zh.subtitle).toContain("ComfyUI");
    expect(zh.subtitle, "又串回旧短剧面板的标语了").not.toContain("数字人");
  });
});

describe("生成按钮", () => {
  /** 空提示词点生成，引擎不报错，照样跑完 45 秒给你一张随机图。 */
  it("按 canSubmit 的结果禁用，不是只看有没有引擎", () => {
    expect(STUDIO).toContain("disabled={!check.ok}");
    expect(STUDIO, "还在只看引擎").not.toContain("disabled={!engine}");
    expect(STUDIO).toContain("canSubmit({");
  });

  /** 灰着不解释比不灰更让人卡住 —— 三种原因解法完全不同。 */
  it("点不动时把原因写在按钮上", () => {
    expect(STUDIO).toContain("title={check.reason}");
    expect(STUDIO).toContain("check.ok ? t(\"studio:generate\"");
  });

  it("带 SVG 图标", () => {
    const at = STUDIO.indexOf("disabled={!check.ok}");
    expect(STUDIO.slice(at, at + 900)).toContain("<svg");
  });

  /** 图标和文字并排，别靠行内排版 —— 一挤就断行（队列那个按钮踩过）。 */
  it("图标和文字用 flex 排，不会断行", () => {
    const at = STUDIO.indexOf("studio-primary-btn");
    expect(STUDIO.slice(at, at + 120)).toContain("items-center");
  });
});

describe("结果区的胶囊按钮", () => {
  /**
   * 普通 `<button>` 里的图标和文字按**行内内容**走，横向一挤就断行 ——
   * 「队列」的图标和两个字被拆成上下两行，看着像坏了。
   */
  it("不会断行", () => {
    const at = GALLERY.indexOf("function Pill(");
    const body = GALLERY.slice(at, at + 900);
    expect(body).toContain("inline-flex");
    expect(body).toContain("whitespace-nowrap");
    expect(body).toContain("shrink-0");
  });
});

/**
 * **成功但有话要说的那几条，必须走到界面上。**
 *
 * 工具那边一直在拼这些告警（引擎回落、LoRA 名字对不上、提示词被摊薄），
 * 而创作台以前是 `error: out.ok ? undefined : out.message` ——
 * 成功时整段丢掉。工具尽了责，界面在最后一行扔了。
 * 表现就是：选 Z-Image + 图生图，实际出的是 SDXL 的图，界面一声不吭。
 */
describe("成功时的告警不能丢", () => {
  const strip = (t: string) =>
    t.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^\s*\/\/.*$/gm, "");
  const src = (rel: string) =>
    strip(
      readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8"),
    );

  it("创作台把 notes 写进结果，而不是只在失败时留 message", () => {
    const studio = src("./AnimaStudio.tsx");
    expect(studio).toContain("notes: out.notes");
    // 这一行本身是对的（失败要显示原因），但不能是唯一的一条
    expect(studio).toContain("error: out.ok ? undefined : out.message");
  });

  it("结果卡把每一条都渲染出来", () => {
    const card = src("./ResultCard.tsx");
    expect(card).toContain("r.notes?.map");
    // **用告警色，不是失败色** —— 图是好的，只是和你以为的不一样
    const i = card.indexOf("r.notes?.map");
    const block = card.slice(i, i + 320);
    expect(block).toContain("--studio-running-fg");
    expect(block, "把成功的结果画成失败了").not.toContain("--studio-failed-fg");
  });
});
