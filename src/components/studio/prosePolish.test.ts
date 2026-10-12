import { describe, expect, it } from "vitest";
import { isSentenceEngine, looksLikeTags, pickProse, proseAdvice } from "./prosePolish";

const TAGS = "1girl, solo, masterpiece, best quality, lace lingerie, soft lighting";
const PROSE = "一个少女独自站在窗边，身上是蕾丝质地的衣物，柔和的侧光从窗外洒进来，画面安静。";

describe("哪些引擎吃长句", () => {
  it("认得出长句原生的编码器", () => {
    for (const e of ["zimage", "krea2", "qwen21", "h3", "minimax_h3", "ltx"]) {
      expect(isSentenceEngine(e), `${e} 没认出来`).toBe(true);
    }
  });

  /** SDXL 的 CLIP 是 77 token 分块取平均，要的就是短关键词。 */
  it("SDXL 不算", () => {
    expect(isSentenceEngine("sdxl")).toBe(false);
    expect(isSentenceEngine("")).toBe(false);
  });
});

describe("这串像不像关键词堆", () => {
  it("逗号分隔的短词算", () => {
    expect(looksLikeTags(TAGS)).toBe(true);
    expect(looksLikeTags("cat, window, sunlight, warm")).toBe(true);
  });

  it("已经是句子的不算", () => {
    expect(looksLikeTags(PROSE)).toBe(false);
    // 段数不够也不算 —— 两三个词不值得改写
    expect(looksLikeTags("一只猫, 窗台")).toBe(false);
    expect(looksLikeTags("")).toBe(false);
  });

  /**
   * **英文散文也不算。** 只测中文的话，「英文一律算标签」这条错误实现
   * 也能全绿 —— 而那会在用户粘一段英文描述时建议他「改写成句子」，
   * 而它本来就是句子。
   */
  it("英文句子不算关键词堆", () => {
    const en =
      "a young woman stands by the window, soft rim light falls across her face, " +
      "the room is quiet and dim, dust drifts through the beam";
    expect(en.split(",").length).toBeGreaterThanOrEqual(4);
    expect(looksLikeTags(en)).toBe(false);
    expect(proseAdvice("zimage", en).suggested).toBe(false);
  });

  /** 混着的：只要有一段是长句，就当它已经是描述了。 */
  it("混着长句的不算关键词堆", () => {
    expect(looksLikeTags(`${TAGS}, ${PROSE}`)).toBe(false);
  });
});

describe("按钮给不给、说什么", () => {
  /**
   * **在 SDXL 上不能给。** 那不是「没必要」，是「点了会更差」——
   * 一个让结果变坏的按钮比没有按钮更糟。
   */
  it("SDXL 上禁用并说清楚为什么", () => {
    const a = proseAdvice("sdxl", TAGS);
    expect(a.enabled).toBe(false);
    expect(a.suggested).toBe(false);
    expect(a.title).toContain("CLIP");
    expect(a.title).toContain("更差");
  });

  it("句子引擎 + 关键词堆时主动建议", () => {
    const a = proseAdvice("zimage", TAGS);
    expect(a.enabled).toBe(true);
    expect(a.suggested).toBe(true);
    expect(a.title).toContain("关键词");
  });

  /** 已经是句子了还能点（人想再顺一遍），只是不主动建议。 */
  it("已经是句子时能点但不建议", () => {
    const a = proseAdvice("zimage", PROSE);
    expect(a.enabled).toBe(true);
    expect(a.suggested).toBe(false);
  });

  it("提示词是空的就不能点", () => {
    const a = proseAdvice("zimage", "   ");
    expect(a.enabled).toBe(false);
    expect(a.title).toContain("先写点什么");
  });
});

describe("结果取哪一份", () => {
  const r = { prompt_cn: "中文那份", prompt_en: "the english one" };
  /** 和词表用同一个判断 —— 两份名单会分叉，而分叉不报错。 */
  it("中文引擎取中文，其余取英文", () => {
    expect(pickProse(r, "zimage")).toBe("中文那份");
    expect(pickProse(r, "h3")).toBe("中文那份");
    expect(pickProse(r, "qwen21")).toBe("中文那份");
    // Krea 2 的编码器是 Qwen3-VL。2026-09-08 实测：同一段中文提示词元素全中、
    // 「三个青苹果」数量都对（Z-Image 出成了四个）—— 所以它吃中文。
    expect(pickProse(r, "krea2")).toBe("中文那份");
    expect(pickProse(r, "sdxl")).toBe("the english one");
    // 不认识的 id 取英文。这一条以前是 `flux` 在占位（长句引擎但不偏中文），
    // FLUX.1 于 2026-09-08 退役后已经没有这种组合了 —— 但**回落方向**仍需钉住：
    // 名单是白名单，认不出就走英文，不能因为名单漏了一个而误发中文。
    expect(pickProse(r, "some-new-engine")).toBe("the english one");
  });
});

// ── 接线 ──
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const read = (p: string) => readFileSync(resolve(HERE, p), "utf8");
/** 剥注释 —— 「文件里有这个字符串」不等于「代码里用了它」。 */
const strip = (t: string) => t.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^\s*\/\/.*$/gm, "");
const PANEL = strip(read("./PromptPanel.tsx"));

describe("改写按钮接线", () => {
  it("按钮走 proseAdvice，不是自己再判一遍引擎", () => {
    expect(PANEL).toContain("proseAdvice(engine?.id");
    // 第二份判断会分叉，而分叉不报错
    expect(PANEL, "面板里又写了一份引擎名单").not.toContain("sdxl");
  });

  it("走 prose 这个槽位，并把引擎传下去", () => {
    const i = PANEL.indexOf("cinemaPolishPrompt(");
    expect(i, "没调润色").toBeGreaterThan(0);
    const call = PANEL.slice(i, i + 160);
    expect(call).toContain('kind: "prose"');
    expect(call).toContain("engine: engine?.id");
  });

  /**
   * **空结果不能覆盖原文。** 覆盖了就是把用户勾了半天的东西清空，
   * 而且没有撤销。
   */
  it("空结果不覆盖原文", () => {
    expect(PANEL).toContain("if (next) setPrompt(next)");
  });

  /**
   * **禁用时也要显示按钮。** 直接藏起来的话，用户在 SDXL 上
   * 会以为这个功能根本不存在，而真实原因是「在这个引擎上不该用」。
   */
  it("禁用而不是隐藏，并挂上原因", () => {
    const i = PANEL.indexOf("IconProse />");
    expect(i).toBeGreaterThan(0);
    const btn = PANEL.slice(Math.max(0, i - 700), i);
    expect(btn).toContain("disabled={!advice.enabled");
    expect(btn).toContain("title={advice.title}");
  });

  /** 失败和警告必须显示出来 —— 最典型的是模型回了一句拒绝。 */
  it("错误和警告有地方显示", () => {
    expect(PANEL).toContain("{proseErr && (");
    expect(PANEL).toContain("setProseErr(String(e))");
    expect(PANEL).toContain("if (r.warning) setProseErr(r.warning)");
  });
});
