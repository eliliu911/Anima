import { describe, expect, it } from "vitest";
import type { EpisodeData } from "../../../api/cinema";
import type { AssetView } from "../../../api/assets";
import { ASPECT_LABEL, cardTitle, episodeProgress, mainCast, renderedOnAspect, seriesStats } from "./overviewStats";

const ep = (o: Partial<EpisodeData>): EpisodeData =>
  ({ id: "e", episode_num: 1, title: "E01-《第三次》", synopsis: "", script_text: "", storyboard: [], ...o }) as EpisodeData;

describe("系列概览的数字", () => {
  it("剧本 / 分镜 / 成片分开数", () => {
    const s = seriesStats([
      ep({ script_text: "有字" }),
      ep({ script_text: "  " }),
      ep({ script_text: "x", storyboard: [{} as never] }),
      ep({ final_video_url: "a.mp4" }),
    ]);
    expect(s).toEqual({ total: 4, scripted: 2, boarded: 1, finished: 1 });
  });
});

describe("分集卡", () => {
  it("进度只说做到哪一步", () => {
    expect(episodeProgress(ep({}))).toBe("还没写剧本");
    expect(episodeProgress(ep({ script_text: "一二三" }))).toBe("剧本 3 字");
    expect(episodeProgress(ep({ storyboard: [{ video_url: "a" }, {}] as never }))).toBe("2 镜 · 出片 1/2");
    expect(episodeProgress(ep({ storyboard: [{}] as never }))).toBe("1 镜 · 未出片");
    expect(episodeProgress(ep({ final_video_url: "x" }))).toBe("已合成");
  });

  it("标题去掉集号前缀和书名号", () => {
    expect(cardTitle(ep({ title: "E01-《第三次》" }))).toBe("第三次");
    expect(cardTitle(ep({ title: "E12 永远不会忘记你" }))).toBe("永远不会忘记你");
    expect(cardTitle(ep({ title: "第 1 集" }))).toBe("第 1 集");
  });
});

describe("主演条", () => {
  const a = (name: string, o: Partial<AssetView> = {}) =>
    ({ id: name, name, kind: "character", cover_url: null, character: null, ...o }) as AssetView;

  it("有图的在前，画外音和非角色不进", () => {
    const list = mainCast([
      a("程远"),
      a("星澜", { cover_url: "x" }),
      a("园丁", { character: { disembodied: true } as never }),
      a("巷口", { kind: "scene" }),
    ]);
    expect(list.map((x) => x.name)).toEqual(["星澜", "程远"]);
  });
});

describe("画幅（2026-09-26）", () => {
  const ep = (shots: { image_url?: string | null; video_url?: string | null }[]) =>
    ({ storyboard: shots }) as unknown as EpisodeData;

  it("数出已经按当前画幅出过的首帧和视频 —— 换画幅前要说出来", () => {
    expect(renderedOnAspect([ep([{ image_url: "a" }, { image_url: "b", video_url: "v" }, {}]), ep([])])).toEqual({ frames: 2, videos: 1 });
    expect(renderedOnAspect([])).toEqual({ frames: 0, videos: 0 });
  });

  it("宽银幕有叫法", () => {
    expect(ASPECT_LABEL["21:9"]).toBe("宽银幕");
  });
});
