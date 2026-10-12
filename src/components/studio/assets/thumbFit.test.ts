import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

/**
 * **资产的图不许裁。**
 *
 * 角色的封面往往是设定卡 —— 一张图里塞了大头特写 + 三个全身。
 * 用 `object-cover` 裁成方格/卡片比例，就只剩其中一块，
 * 而这些缩略图的用处恰恰是「认出这是谁」「判断这张脸对不对」。
 *
 * 不报错，只是看不全 —— 用户第一眼就看出来了：「需要可以点击打开预览图，
 * 不然看不全」。
 *
 * 例外只有两处，都在下面列着并说明了理由。
 */
const STUDIO = fileURLToPath(new URL("../", import.meta.url));

/** 允许裁的：整幅装饰性封面、以及和项目同比例的镜头画面。 */
const ALLOWED = [
  // 工作区卡片的大封面：整幅铺满是它的设计意图，裁掉边缘无损辨识
  "workspace/WorkspacePage.tsx",
  // 风格缩略图：示范的是**画风**（色彩 / 光线 / 质感），裁边不丢信息。
  // 图是 768 方、卡位是横的，用 `contain` 会留黑边 —— 看着很不专业。
  // 「不许裁」那条针对的是设定卡：那种图裁一刀就少半个版面。
  "steps/StyleStep.tsx",
  // 第 5、6 步的镜头缩略图，同上 —— 它们显示的是 `s.video_url || s.image_url`，
  // 也就是按项目画幅出的那一帧
  "steps/MotionStep.tsx",
  "steps/AssemblyStep.tsx",
];

function walk(dir: string): string[] {
  const { readdirSync, statSync } = require("node:fs") as typeof import("node:fs");
  const { join } = require("node:path") as typeof import("node:path");
  return readdirSync(dir).flatMap((n: string) => {
    const p = join(dir, n);
    if (statSync(p).isDirectory()) return walk(p);
    return n.endsWith(".tsx") ? [p] : [];
  });
}

describe("资产缩略图不裁切", () => {
  const offenders = walk(STUDIO)
    .filter((f) => !ALLOWED.some((a) => f.endsWith(a)))
    .filter((f) => readFileSync(f, "utf8").includes("object-cover"))
    .map((f) => f.slice(STUDIO.length));

  it("除了列出的两处例外，没有别的地方用 object-cover", () => {
    expect(offenders, `这些地方在裁资产图：${offenders.join(", ")}`).toEqual([]);
  });

  it("例外清单本身不是空的 —— 空的话这条检查等于没设边界", () => {
    expect(ALLOWED.length).toBeGreaterThan(0);
  });

  /**
   * **例外要写得出理由。** 清单越长这条规则越没用 ——
   * 五条以上就该回头想想是不是规则本身划错了。
   */
  it("例外不超过五条", () => {
    expect(ALLOWED.length).toBeLessThanOrEqual(5);
  });

  it("候选图和封面都用 contain", () => {
    const wb = readFileSync(`${STUDIO}assets/AssetWorkbench.tsx`, "utf8");
    expect(wb).toContain("aspect-square w-full object-contain");
    const card = readFileSync(`${STUDIO}library/AssetCard.tsx`, "utf8");
    expect(card).toContain("object-contain");
  });
});
