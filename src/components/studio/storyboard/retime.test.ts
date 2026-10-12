import { describe, expect, it } from "vitest";
import type { ShotSegment, StoryboardShot } from "../../../api/cinema";
import { retimeSegments, scaleSecondMentions, withDuration } from "./retime";

const seg = (start: number, end: number, visual = ""): ShotSegment => ({ start, end, visual });

/** 2026-09-30 EP02 第 7 镜：11 秒加到 15 秒，下面的时间线不跟着变，提示照样在 */
describe("改时长时分段时间线跟着伸缩", () => {
  it("11 秒 → 15 秒：按比例、半秒取整、末段对齐新时长", () => {
    const out = retimeSegments([seg(0, 3), seg(3, 7, "王大爷约第5秒开口"), seg(7, 11)], 11, 15)!;
    expect(out.map((s) => [s.start, s.end])).toEqual([[0, 4], [4, 9.5], [9.5, 15]]);
    // 描写里写死的秒数一起换算（5 × 15/11 ≈ 6.8 → 7）
    expect(out[1].visual).toBe("王大爷约第7秒开口");
  });

  it("缩短也行；每段至少半秒", () => {
    const out = retimeSegments([seg(0, 1), seg(1, 2), seg(2, 9)], 9, 4)!;
    expect(out.map((s) => [s.start, s.end])).toEqual([[0, 0.5], [0.5, 1], [1, 4]]);
  });

  it("放不下这么多段、或者时间线本来就没对齐旧时长：不动，留给检查去报", () => {
    expect(retimeSegments([seg(0, 1), seg(1, 2), seg(2, 3), seg(3, 4), seg(4, 5), seg(5, 6), seg(6, 7), seg(7, 8), seg(8, 9)], 9, 4)).toBe(null);
    expect(retimeSegments([seg(0, 3), seg(3, 9)], 11, 15)).toBe(null);
    expect(retimeSegments([], 11, 15)).toBe(null);
    expect(retimeSegments([seg(0, 11)], 11, 11)).toBe(null);
  });

  it("秒数换算：小数、空格、不超过镜头时长", () => {
    expect(scaleSecondMentions("约第2.5秒开口，第 9 秒收尾", 2, 15)).toBe("约第5秒开口，第15秒收尾");
    expect(scaleSecondMentions("第3秒", 1.25, 15)).toBe("第4秒"); // 3.75 → 4
    expect(scaleSecondMentions("第1秒", 1.25, 15)).toBe("第1.5秒");
    expect(scaleSecondMentions("没有秒数", 2, 15)).toBe("没有秒数");
    // EP03 第 20 镜的写法「约5.1秒」也要换（14 → 20.5 秒，× 1.464 → 7.5，半秒刻度）
    expect(scaleSecondMentions("约5.1秒星澜平稳开口；约 0.5 秒带着笑意", 20.5 / 14, 20.5)).toBe("约7.5秒星澜平稳开口；约0.5秒带着笑意");
    expect(scaleSecondMentions("大约一会儿", 2, 15)).toBe("大约一会儿");
  });

  it("整镜：时长和分段一起换；伸缩不了就只换时长", () => {
    const shot = { id: "s", shot_num: 7, duration: 11, segments: [seg(0, 5), seg(5, 11)] } as StoryboardShot;
    const a = withDuration(shot, 15);
    expect(a.duration).toBe(15);
    expect(a.segments!.at(-1)!.end).toBe(15);
    const broken = { ...shot, segments: [seg(0, 5), seg(5, 9)] } as StoryboardShot;
    const b = withDuration(broken, 15);
    expect(b.duration).toBe(15);
    expect(b.segments).toBe(broken.segments);
  });
});
