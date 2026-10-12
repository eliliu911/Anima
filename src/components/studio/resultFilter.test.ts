import { describe, expect, it } from "vitest";
import { counts, filterResults, kindOf, VIDEO_MODES } from "./resultFilter";
import type { StudioResult } from "./useStudioStore";

function r(p: Partial<StudioResult> & { id: string }): StudioResult {
  return {
    status: "done",
    mode: "t2i",
    engineId: "zimage",
    prompt: "",
    path: `/tmp/${p.id}.png`,
    startedAt: 0,
    ...p,
  } as StudioResult;
}

describe("结果分类", () => {
  /**
   * 模式只说了**打算**出什么，文件才是**实际**出了什么。
   * 只看模式的话，视频会被塞进「图片」页签里 —— 点开是个播放器，不报错。
   */
  it("有文件时以文件为准，不看模式", () => {
    expect(kindOf(r({ id: "a", mode: "t2i", path: "/tmp/a.mp4" }))).toBe("video");
    expect(kindOf(r({ id: "b", mode: "i2v", path: "/tmp/b.png" }))).toBe("image");
  });

  it("扩展名不区分大小写", () => {
    expect(kindOf(r({ id: "a", path: "/tmp/a.MP4" }))).toBe("video");
  });

  /** 还在跑的没有文件，只能退回模式判断 —— 但必须真的判，不能一律当图片。 */
  it("没有文件时退回模式", () => {
    expect(kindOf(r({ id: "a", mode: "i2v", status: "running", path: undefined }))).toBe("video");
    expect(kindOf(r({ id: "b", mode: "t2i", status: "running", path: undefined }))).toBe("image");
  });

  it("四种视频模式都认得", () => {
    for (const m of ["t2v", "i2v", "r2v", "v2v"]) {
      expect(VIDEO_MODES.has(m), `${m} 不在视频模式表里`).toBe(true);
      expect(kindOf(r({ id: m, mode: m, status: "running", path: undefined }))).toBe("video");
    }
  });
});

describe("页签过滤", () => {
  const all = [
    r({ id: "img1" }),
    r({ id: "vid1", mode: "i2v", path: "/tmp/v.mp4" }),
    r({ id: "img2" }),
    r({ id: "running", status: "running", mode: "t2v", path: undefined }),
  ];

  it("全部就是全部，一个不少", () => {
    expect(filterResults(all, "all")).toHaveLength(4);
  });

  it("图片页签里没有视频", () => {
    expect(filterResults(all, "image").map((x) => x.id)).toEqual(["img1", "img2"]);
  });

  /** 还在跑的视频也要出现在「视频」页签里，否则它跑完之前人找不到它。 */
  it("视频页签包含还在跑的视频任务", () => {
    expect(filterResults(all, "video").map((x) => x.id)).toEqual(["vid1", "running"]);
  });

  /**
   * 页签上的数字数的是**全部**，不是当前这一档 ——
   * 否则「视频」旁边的数字要切过去才知道，等于没有信息。
   */
  it("三个页签的计数都基于全量", () => {
    expect(counts(all)).toEqual({ all: 4, image: 2, video: 2 });
  });

  it("图片数 + 视频数 = 总数，不会漏也不会重", () => {
    const c = counts(all);
    expect(c.image + c.video).toBe(c.all);
  });
});
