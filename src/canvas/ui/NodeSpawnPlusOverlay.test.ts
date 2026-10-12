import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { magnetOffset, MAGNET_PX } from "./NodeSpawnPlusOverlay";

/** 用户 2026-09-24：「缩小加号，并跟 LibTV 一样，在 1cm 范围内增加鼠标吸附跟随」 */
describe("连线加号的吸附跟随", () => {
  it("约 1 厘米（38 屏幕像素）以内跟着鼠标走，外面回原位", () => {
    expect(MAGNET_PX).toBe(38);
    expect(magnetOffset(20, 10, 1)).toEqual({ x: 20, y: 10 });
    expect(magnetOffset(30, 30, 1)).toEqual({ x: 0, y: 0 });  // 42 > 38
  });

  it("按屏幕距离判断、按节点坐标挪：画布缩到一半时要挪两倍，才正好停在鼠标下", () => {
    expect(magnetOffset(20, 0, 0.5)).toEqual({ x: 40, y: 0 });
    expect(magnetOffset(20, 0, 2)).toEqual({ x: 10, y: 0 });
  });

  it("加号缩小到 20 像素（以前 28）", () => {
    const src = readFileSync(new URL("./NodeSpawnPlusOverlay.tsx", import.meta.url), "utf8");
    expect(src).toContain("!h-5 !w-5");
    expect(src).not.toContain("!h-7 !w-7");
  });
});
