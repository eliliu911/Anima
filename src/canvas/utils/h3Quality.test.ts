import { readFileSync } from "node:fs";
import { describe, it, expect } from "vitest";
import { effectiveQuality, qualityChoices, qualityInfo } from "./h3Quality";

/** 用户 2026-10-08：「昨天的加速节点在画布里没有……这个菜单需要增加加速节点选项」 */
describe("画布 H3 加速档", () => {
  it("按链路给档位和默认：首帧 LightX2V、参考 Larry 6、文生 Larry 4；全能参考 / 动作迁移不给选", () => {
    expect(effectiveQuality(undefined, "i2v")).toBe("lightx2v");
    expect(effectiveQuality(undefined, "fl2v")).toBe("lightx2v");
    expect(effectiveQuality(undefined, "r2v")).toBe("staged");
    expect(effectiveQuality(undefined, "t2v")).toBe("standard");
    expect(effectiveQuality(undefined, "motion")).toBeUndefined();
    expect(qualityChoices("t2v").map((c) => c.id)).not.toContain("lightx2v");
  });
  it("存的档这条链路能选就照用，换了链路选不了就回默认", () => {
    expect(effectiveQuality("dense", "r2v")).toBe("dense");
    expect(effectiveQuality("lightx2v", "t2v")).toBe("standard");
    expect(effectiveQuality("乱写", "i2v")).toBe("lightx2v");
  });
  it("说明和工作台出片设置同一份；执行器把它发给后端（只 H3）", () => {
    expect(qualityInfo("lightx2v").lora).toContain("minimax_h3_fl2v_turbo_4step_v0.1");
    const exec = readFileSync(new URL("../engine/dagExecutor.ts", import.meta.url), "utf8");
    expect(exec).toContain('...(engine === "h3" && effectiveQuality(data.h3Quality, mode) ? { h3_quality: effectiveQuality(data.h3Quality, mode) } : {}),');
    const pop = readFileSync(new URL("../ui/CanvasVideoSettingsPopover.tsx", import.meta.url), "utf8");
    expect(pop).toContain('const blocked = isH3(engine) && res === "1080P";');
  });
});
