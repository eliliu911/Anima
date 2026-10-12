import { describe, expect, it } from "vitest";
import type { StoryboardShot } from "../../../api/cinema";
import { manualSecs, needsRegen, sceneMode, windows, withSceneMode } from "./oneTake";

const shot = (scene: number, duration: number, one_take = false) => ({ id: `${scene}-${duration}`, shot_num: 1, scene_no: scene, duration, one_take } as unknown as StoryboardShot);

describe("一场一镜出", () => {
  it("等分窗口 —— 和 Rust one_take::windows 同一组数", () => {
    expect(windows(15)).toEqual({ n: 1, each: 15 });
    expect(windows(12)).toEqual({ n: 1, each: 12 });
    expect(windows(30)).toEqual({ n: 2, each: 15 });
    expect(windows(20)).toEqual({ n: 2, each: 10 });
    expect(windows(31)).toEqual({ n: 3, each: 10.5 });
    expect(windows(60)).toEqual({ n: 4, each: 15 });
    expect(windows(90)).toEqual({ n: 4, each: 15 });
    expect(windows(1)).toEqual({ n: 1, each: 4 });
  });

  it("单场设置优先于分集默认", () => {
    const ep = { one_take: true, scene_takes: [{ scene_no: 2, one_take: false }, { scene_no: 3, one_take: true, secs: 30 }], long_take_scenes: [4] };
    expect(sceneMode(ep, 1)).toBe("one_take");
    expect(sceneMode(ep, 2)).toBe("split");
    expect(manualSecs(ep, 3)).toBe(30);
    expect(manualSecs(ep, 1)).toBeNull();
    expect(sceneMode({ ...ep, one_take: false, scene_takes: [] }, 4)).toBe("long_take");
  });

  it("换出法：一镜出去掉长镜头标记；整集默认一镜出时改回分镜要记一条 false", () => {
    const ep = { one_take: false, scene_takes: [], long_take_scenes: [1, 2] };
    expect(withSceneMode(ep, 2, "one_take", 30)).toEqual({ scene_takes: [{ scene_no: 2, one_take: true, secs: 30 }], long_take_scenes: [1] });
    expect(withSceneMode({ ...ep, scene_takes: [{ scene_no: 2, one_take: true }] }, 2, "split")).toEqual({ scene_takes: [], long_take_scenes: [1] });
    expect(withSceneMode({ ...ep, one_take: true }, 2, "split").scene_takes).toEqual([{ scene_no: 2, one_take: false }]);
    expect(withSceneMode(ep, 3, "long_take").long_take_scenes).toEqual([1, 2, 3]);
  });

  it("镜头和设的出法对不上才要重新生成", () => {
    const one = { one_take: false, scene_takes: [{ scene_no: 1, one_take: true }], long_take_scenes: [] };
    expect(needsRegen(one, 1, [shot(1, 4), shot(1, 2)])).toContain("改成一镜出");
    expect(needsRegen(one, 1, [shot(1, 15, true)])).toBeNull();
    const thirty = { ...one, scene_takes: [{ scene_no: 1, one_take: true, secs: 30 }] };
    expect(needsRegen(thirty, 1, [shot(1, 15, true)])).toContain("30 秒");
    expect(needsRegen(thirty, 1, [shot(1, 15, true), shot(1, 15, true)])).toBeNull();
    expect(needsRegen({ one_take: false, scene_takes: [], long_take_scenes: [] }, 1, [shot(1, 15, true)])).toContain("改回分镜");
    expect(needsRegen(one, 2, [shot(1, 4)])).toBeNull();
  });
});
