import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import type { ShotSegment, StoryboardShot } from "../../../api/cinema";
import type { Job } from "../../../api/jobs";
import { needsRefine, refinePlan, refineRound, segmentChars, shotDensity, thinSegments } from "./timelineDensity";

const seg = (start: number, end: number, visual: string, extra: Partial<ShotSegment> = {}): ShotSegment =>
  ({ start, end, title: "", visual, forbid: "", constraints: "", blocking: "", camera: "", sound: "", ...extra }) as ShotSegment;
const shot = (id: string, num: number, scene: number, segments: ShotSegment[], extra: Partial<StoryboardShot> = {}) =>
  ({ id, shot_num: num, scene_no: scene, duration: segments.at(-1)?.end ?? 0, segments, ...extra }) as unknown as StoryboardShot;

describe("时间线写得细不细（和后端同一套数）", () => {
  it("画面 + 站位 + 运镜算字，约束、声音、空白不算", () => {
    const s = seg(0, 4, "字 字", { blocking: "站", camera: "推", constraints: "保持不动", sound: "雨声" });
    expect(segmentChars(s)).toBe(4);
    expect(shotDensity(shot("a", 1, 1, [s]))).toBe(1);
    expect(shotDensity({ segments: [], duration: 0 })).toBe(0);
  });

  it("一笔带过：够长（≥2 秒）却每秒不到 20 字；短过渡段写一句是正常的", () => {
    // EP01 第 27 镜那种：12 秒 62 字
    const s27 = shot("s27", 27, 6, [seg(0, 12, "字".repeat(62))]);
    expect(thinSegments(s27)).toEqual([{ n: 1, start: 0, end: 12, chars: 62 }]);
    expect(needsRefine(s27)).toBe(true);
    const ok = shot("ok", 1, 1, [seg(0, 1, "眨眼"), seg(1, 12, "字".repeat(480))]);
    expect(thinSegments(ok)).toEqual([]);
    expect(needsRefine(ok)).toBe(false);
  });

  it("批量细化：只挑选中场里偏薄的；出过片的默认不动，勾上才动", () => {
    const thin = (id: string, num: number, scene: number, extra: Partial<StoryboardShot> = {}) => shot(id, num, scene, [seg(0, 10, "一句")], extra);
    const shots = [
      thin("a", 1, 1, { video_url: "x.mp4" }),
      thin("b", 2, 1),
      shot("c", 3, 1, [seg(0, 10, "字".repeat(420))]),
      thin("d", 4, 2),
    ];
    expect(refinePlan(shots, [1], false)).toEqual({ ids: ["b"], dense: 1, rendered: 1 });
    expect(refinePlan(shots, [1], true).ids).toEqual(["a", "b"]);
    expect(refinePlan(shots, [1, 2], false).ids).toEqual(["b", "d"]);
  });

  it("这一镜在细化任务里：正在细化 / 排队中 / 做过了 / 不在这一轮", () => {
    const job = (done: number, label: string, status: Job["status"] = "running") =>
      ({ id: "j", kind: "timeline", status, targets: ["a", "b", "c"], progress: { done, total: 3, label } }) as unknown as Job;
    const s = (id: string, num: number) => ({ id, shot_num: num });
    expect(refineRound(s("b", 2), job(1, "第 2 镜"))).toBe("running");
    expect(refineRound(s("c", 3), job(1, "第 2 镜"))).toBe("queued");
    expect(refineRound(s("a", 1), job(1, "第 2 镜"))).toBeNull();
    expect(refineRound(s("z", 9), job(1, "第 2 镜"))).toBeNull();
    expect(refineRound(s("b", 2), job(3, "第 3 镜", "done"))).toBeNull();
  });
});

describe("分镜页接线", () => {
  const src = readFileSync(fileURLToPath(new URL("../steps/StoryboardStep.tsx", import.meta.url)), "utf8");
  it("细化进度一动就重读；细化的正好是打开着的那一镜时详情面板重挂（否则草稿把旧时间段存回去）", () => {
    expect(src).toContain("}, [refining, refineDone]);");
    expect(src).toMatch(/void reload\(\)\.then\(\(\) => \{\s*if \(shown && shown\.shot_num === n\) setDetailRev/);
    expect(src).toContain("onRefine={() => void refine([selected.id])}");
  });
});

describe("保存和细化的先后（2026-09-28 审查）", () => {
  const detail = readFileSync(fileURLToPath(new URL("./ShotDetail.tsx", import.meta.url)), "utf8");
  const step = readFileSync(fileURLToPath(new URL("../steps/StoryboardStep.tsx", import.meta.url)), "utf8");
  it("点细化时先等草稿存完，存失败就不细化", () => {
    expect(detail).toContain("if (await save()) onRefine();");
    expect(detail).toMatch(/const save = async \(next: StoryboardShot = draft\): Promise<boolean>/);
  });
  it("详情面板只存这一镜、带上草稿从哪一份开始改的；不再整集保存", () => {
    const fn = step.slice(step.indexOf("const saveShot = async"), step.indexOf("return (", step.indexOf("const saveShot = async")));
    expect(fn).toContain("cinemaSaveShot(project.id, episode.id, edited, base)");
    expect(fn).not.toContain("cinemaSaveEpisode");
    expect(detail).toContain("const ok = await onSave(next, base.current);");
  });
  it("接上一镜：存的是改之后的那份", () => {
    expect(detail).toContain("const next = { ...draft, continuity: c, continuity_manual: c !== auto };");
  });
});
