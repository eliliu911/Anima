import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { stageLines } from "./sceneStage";

describe("这一场的固定布景", () => {
  it("和后端同一个顺序、同一套标签；空的项不显示；老镜头没有就是空", () => {
    expect(stageLines(undefined)).toEqual([]);
    expect(stageLines({})).toEqual([]);
    expect(
      stageLines({ time: "零点，暴雨", facing: "从拆机间正门往里看", lights: ["冷蓝顶灯：厂房顶部", " ", "暖黄工作灯：拆机台上方"], anchors: [] }),
    ).toEqual([
      { label: "方向基准", text: "从拆机间正门往里看" },
      { label: "光源", text: "冷蓝顶灯：厂房顶部；暖黄工作灯：拆机台上方" },
      { label: "时段天气", text: "零点，暴雨" },
    ]);
  });

  it("详情面板在光影前面显示它，只读", () => {
    const src = readFileSync(fileURLToPath(new URL("./ShotDetail.tsx", import.meta.url)), "utf8");
    const at = src.indexOf('<Block title="这一场的布景">');
    expect(at).toBeGreaterThan(0);
    expect(at).toBeLessThan(src.indexOf('<Block title="光影">'));
    const block = src.slice(at, src.indexOf("</Block>", at));
    expect(block).not.toContain("onChange");
  });
});
