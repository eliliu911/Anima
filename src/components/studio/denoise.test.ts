import { describe, expect, it } from "vitest";
import {
  clampDenoise,
  DENOISE_DEFAULT,
  DENOISE_MAX,
  DENOISE_MIN,
  denoiseHint,
  fmtDenoise,
  usesDenoise,
} from "./denoise";

describe("哪些模式有重绘幅度", () => {
  it("只有图生图", () => {
    expect(usesDenoise("i2i")).toBe(true);
    for (const m of ["t2i", "sheet", "pose", "t2v", "i2v", "fl2v", "r2v", ""]) {
      expect(usesDenoise(m), `${m} 不该有 denoise`).toBe(false);
    }
  });
});

describe("区间", () => {
  /**
   * **不给到 0。** denoise 到 0 时采样器一步都不跑，出来就是原图 ——
   * 那不是「几乎不变」，是「什么都没发生」，而且不报错。
   */
  it("下限不到 0", () => {
    expect(DENOISE_MIN).toBeGreaterThan(0);
    expect(clampDenoise(0)).toBe(DENOISE_MIN);
    expect(clampDenoise(-1)).toBe(DENOISE_MIN);
  });

  it("上限是 1", () => {
    expect(DENOISE_MAX).toBe(1);
    expect(clampDenoise(2)).toBe(1);
  });

  it("认不出的值退回默认", () => {
    expect(clampDenoise(NaN)).toBe(DENOISE_DEFAULT);
  });

  it("默认值在区间里", () => {
    expect(DENOISE_DEFAULT).toBeGreaterThanOrEqual(DENOISE_MIN);
    expect(DENOISE_DEFAULT).toBeLessThanOrEqual(DENOISE_MAX);
  });
});

describe("说明文字", () => {
  /** **用结果说话。**「强 / 弱」没有信息量，「构图会重来」才有。 */
  it("每一档都说清会发生什么", () => {
    for (const v of [0.2, 0.3, 0.45, 0.65, 0.85, 1]) {
      const h = denoiseHint(v);
      expect(h.length, `${v} 的说明太短`).toBeGreaterThan(8);
      expect(h, `${v} 用了没信息量的形容词`).not.toMatch(/^(强|弱|中等)$/);
    }
  });

  /** 相邻的档说的必须是不同的事，否则这个滑杆等于只有一档。 */
  it("各档说的不是同一件事", () => {
    const seen = new Set([0.25, 0.45, 0.65, 0.85, 1].map(denoiseHint));
    expect(seen.size).toBe(5);
  });

  /** **1.0 要明说「源图没用了」** —— 拉到头还以为在图生图是最容易的误会。 */
  it("拉到 1 时说清源图不起作用", () => {
    expect(denoiseHint(1)).toContain("没有意义");
  });

  it("显示成两位小数", () => {
    expect(fmtDenoise(0.65000000000000002)).toBe("0.65");
    expect(fmtDenoise(1)).toBe("1.00");
  });
});

// ── 接线 ──
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const strip = (t: string) =>
  t.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^\s*\/\/.*$/gm, "");
const src = (rel: string) =>
  strip(readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8"));

/**
 * **这个旋钮一路要真的通到工具。**
 *
 * 后端早就认 `denoise` 了，而界面上没有出口 —— 于是图生图一直只有
 * 默认那一档 0.65。这一条盯的就是「加了控件但没接线」。
 */
describe("重绘幅度接到了底", () => {
  it("参数面板只在图生图显示它", () => {
    const panel = src("./ParamsPanel.tsx");
    expect(panel).toContain("usesDenoise(mode)");
    expect(panel).toContain("setDenoise(");
  });

  it("创作台把它发出去，且只在图生图带上", () => {
    const studio = src("./AnimaStudio.tsx");
    const i = studio.indexOf("denoise:");
    expect(i, "没往请求里放 denoise").toBeGreaterThan(0);
    expect(studio.slice(i, i + 90)).toContain("usesDenoise(st.mode)");
  });

  /**
   * **默认值两边必须一致。** 不一致的话界面显示 0.65、实际发的是别的数，
   * 而那种偏差没人看得出来。
   */
  it("默认值和后端写的是同一个", () => {
    const rust = readFileSync(
      fileURLToPath(new URL("../../../src-tauri/src/agent_runtime/image_tool.rs", import.meta.url)),
      "utf8",
    );
    // 后端：`.unwrap_or(if image_path_opt.is_some() { 0.65 } else { 1.0 })`
    const m = /image_path_opt\.is_some\(\)\s*\{\s*([0-9.]+)/.exec(rust);
    expect(m, "后端那行默认值的写法变了，这条守卫要跟着改").toBeTruthy();
    expect(Number(m![1])).toBe(DENOISE_DEFAULT);
  });
});
