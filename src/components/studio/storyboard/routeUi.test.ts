import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { DEFAULT_KEYFRAME_QUALITY, KEYFRAME_QUALITIES, REFERENCE_QUALITIES, shownQuality } from "./renderOptions";

/** 2026-10-07 用户：出片方式自动定、界面二选一、出片设置按首帧 / 参考分开选画质 */
describe("出片方式和出片设置", () => {
  const dialog = readFileSync(new URL("./RenderOptionsDialog.tsx", import.meta.url), "utf8");
  const detail = readFileSync(new URL("./ShotDetail.tsx", import.meta.url), "utf8");

  it("两栏画质：首帧出片默认 LightX2V，参考出片默认 Larry 6 步；不再给「快速」", () => {
    expect(KEYFRAME_QUALITIES[0]).toEqual({ id: "lightx2v", tag: "推荐" });
    expect(REFERENCE_QUALITIES[0]).toEqual({ id: "staged", tag: "定稿" });
    expect(DEFAULT_KEYFRAME_QUALITY).toBe("lightx2v");
    for (const list of [KEYFRAME_QUALITIES, REFERENCE_QUALITIES]) expect(list.map((q) => q.id)).not.toContain("fast");
    expect(shownQuality("fast")).toBe("standard");
    expect(dialog).toContain('qualityColumn("首帧出片", kfShots.length, KEYFRAME_QUALITIES, kfQuality, (q) => set("keyframe_quality", q)');
    expect(dialog).toContain('qualityColumn("参考出片"');
  });

  it("这一批按实际链路分组：有首帧 = 首帧出片", () => {
    expect(dialog).toContain('(routeOf(s)?.actual ?? (s.keyframes?.first ? "keyframe" : "reference")) === "keyframe"');
    expect(dialog).toContain("首帧出片 <b>{kfShots.length}</b> · 参考出片 <b>{refShots.length}</b>");
  });

  it("引擎和提示词线路收进「高级」，默认折起", () => {
    expect(dialog).toContain("const [advanced, setAdvanced] = useState(false);");
    expect(dialog.indexOf("{advanced && (")).toBeLessThan(dialog.indexOf('label="出片引擎'));
  });

  it("详情二选一：首帧出片折起参考图，参考出片（内容也建议参考）折起关键帧", () => {
    expect(detail).toContain('open={routeInfo?.actual !== "keyframe"}');
    expect(detail).toContain('open={!(routeInfo?.actual === "reference" && routeInfo.advised === "reference")}');
    expect(detail).toContain("出不了：{routeInfo.blocker}");
  });

  /** 用户 2026-10-08：链路角标是灰色，看着像「没生效」；红绿已被问题 / 完成占了 —— 首帧蓝、参考紫 */
  it("链路角标有自己的颜色，三套主题都定义了", () => {
    const step = readFileSync(new URL("../steps/StoryboardStep.tsx", import.meta.url), "utf8");
    expect(step).toContain('<Tag tone={ri.blocker ? "danger" : ri.actual === "keyframe" ? "keyframe" : "reference"}');
    const css = readFileSync(new URL("../../../studio-theme.css", import.meta.url), "utf8");
    expect(css.match(/--studio-route-keyframe-fg:/g)?.length).toBe(3);
    expect(css.match(/--studio-route-reference-fg:/g)?.length).toBe(3);
  });
});
