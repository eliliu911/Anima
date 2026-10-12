import { readFileSync, readdirSync } from "node:fs";
import { describe, expect, it } from "vitest";

/**
 * 进度指示的接线。
 *
 * 纯逻辑在 Rust 的 `comfy_progress`（8 条测试）。这里盯的是
 * **它真的被接到界面上了** —— 「算得对但没接上」在这个仓库抓到过好几次。
 */
const read = (p: string) => readFileSync(new URL(p, import.meta.url), "utf8");

describe("ComfyUI 进度接线", () => {
  const gallery = read("./CanvasMediaGallery.tsx");
  const hook = read("./useComfyProgress.ts");

  it("画册在生成时轮询进度，并把那句话显示出来", () => {
    expect(gallery).toContain("useComfyProgress(isGenerating)");
    expect(gallery).toContain("{comfy.text}");
  });

  /** 「第几张」和「这一张跑到哪了」是两件事，不能互相替换。 */
  it("进度是另起一行，没有替换掉原来那句「渲染中 (n/m)」", () => {
    expect(gallery).toContain('{kind === "image" ? "ComfyUI 渲染中" : "ComfyUI 视频渲染中"} ({currentRenderingStep}/{targetCount})');
  });

  /** **拿不到进度时这一行不出现**，界面退回原样 —— 不显示一个占位数字。 */
  it("拿不到进度就不显示这一行", () => {
    expect(gallery).toContain("{comfy && (");
    expect(hook).toContain("setP(null)");
  });

  /** 不生成时不轮询 —— 别为一个看不见的数字占着 CPU。 */
  it("只在生成时轮询，停了就清掉", () => {
    expect(hook).toContain("if (!isGenerating)");
    expect(hook).toContain("window.clearInterval");
  });
});

/**
 * **不做进度条。**
 *
 * 条形需要一个可信的百分比，而我们有时候拿不到（日志读不到、WeeTodd 换了格式）。
 * 那时候只能说「已跑 N 分钟」，一根停在某个位置的条比一行字更容易被误读。
 * DC 在这一层显示模拟百分比 —— 那是另一种取舍，不是我们的选择。
 */
describe("不编百分比", () => {
  it("画布里没有任何地方按已用时间算百分比", () => {
    const dir = new URL("./", import.meta.url);
    for (const f of readdirSync(dir).filter((f) => f.endsWith(".tsx") || f.endsWith(".ts"))) {
      if (f.endsWith(".test.ts")) continue;
      const s = readFileSync(new URL(f, dir), "utf8");
      // 模拟进度的特征：用 Math.exp / elapsed 算出一个 percent
      expect(s, `${f} 看起来在按时间模拟百分比`).not.toMatch(/percent[\s\S]{0,80}Math\.exp/);
    }
  });
});
