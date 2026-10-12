import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { describeLoadError } from "./panoStage";

const read = (rel: string) => readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");

describe("全景贴图加载失败的报错", () => {
  /**
   * **`TextureLoader` 的失败回调给的是 `ErrorEvent`，不是 `Error`。**
   * 原来 `String(e)`，界面上只剩一句 `[object Event]`，真正原因被吞掉 ——
   * 2026-09-16 全景查看器和导演世界两个节点都是这样报的。
   */
  it("Event 不能被打印成 [object Event]", () => {
    const msg = describeLoadError(new Event("error"), "anima://x.png");
    expect(msg).not.toContain("[object Event]");
    expect(msg).toContain("error");
  });

  it("传进来的是本机路径时直接说破", () => {
    const msg = describeLoadError(new Event("error"), "/Users/a/pano.png");
    expect(msg).toContain("本机路径");
    expect(msg).toContain("mediaDataUrl");
  });

  it("Error 照常带上 message", () => {
    expect(describeLoadError(new Error("boom"), "x")).toContain("boom");
  });

  /** data URL 几 MB，整串进报错框会把界面撑爆。 */
  it("超长 URL 要截断", () => {
    const huge = "data:image/png;base64," + "A".repeat(50_000);
    const msg = describeLoadError(new Event("error"), huge);
    expect(msg.length).toBeLessThan(400);
  });
});

describe("两个全景节点的贴图来源", () => {
  /**
   * 节点里的 `source` 是上游 `outputImageUrl` —— **本机路径**。
   * 原来直接喂给 `createPanoStage`，必然加载失败。
   * 这条钉住：建舞台之前必须先过 `mediaDataUrl`。
   */
  for (const f of ["../nodes/PanoViewerNode.tsx"]) {
    it(`${f.split("/").pop()} 不把原始路径直接当贴图`, () => {
      const src = read(f);
      expect(src).not.toMatch(/createPanoStage\(\{\s*canvas,\s*textureUrl:\s*source/);
      expect(src).toContain("mediaDataUrl(source)");
    });

    /**
     * 舞台异步建好后必须踢一下依赖它的 effect，否则初始视角、已摆的标记
     * 都不会上屏，直到用户碰一下参数 —— 看起来就是「摆了但画面上没有」。
     */
    it(`${f.split("/").pop()} 建台后会触发重新同步`, () => {
      const src = read(f);
      expect(src).toContain("setReady((n) => n + 1)");
      expect(src).toMatch(/, ready\]\)/);
    });
  }

  /**
   * 贴图不能走 `anima-media://`：没有 CORS 头会污染 canvas，截图时 `toDataURL`
   * 抛 SecurityError；探测失败还会把全会话的 `protocolBroken` 置真，
   * 拖慢所有缩略图。
   */
  it("mediaDataUrl 不碰全局的 protocolBroken", () => {
    const src = read("../../components/studio/MediaThumb.tsx");
    const body = src.slice(src.indexOf("export async function mediaDataUrl"));
    const fn = body.slice(0, body.indexOf("\n}\n"));
    expect(fn).not.toContain("protocolBroken");
    expect(fn).not.toContain("mediaSrc(");
  });
});
