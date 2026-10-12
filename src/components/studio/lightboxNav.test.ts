import { describe, expect, it } from "vitest";
import { indexOfId, step, viewables } from "./lightboxNav";
import type { StudioResult } from "./useStudioStore";

function r(p: Partial<StudioResult> & { id: string }): StudioResult {
  return {
    status: "done",
    mode: "t2i",
    prompt: "",
    path: `/tmp/${p.id}.png`,
    ...p,
  } as StudioResult;
}

describe("灯箱的下标", () => {
  /**
   * **这是这块唯一真正会错的地方。** 一屏里混着正在跑的卡片时，
   * 「点了第 3 张卡」和「灯箱里的第 3 张」不是一回事。
   * 而且它错得很安静：打开的是一张真图，只是不是你点的那张。
   */
  it("跑到一半的和失败的不在可看列表里", () => {
    const list = viewables([
      r({ id: "a" }),
      r({ id: "b", status: "running", path: undefined }),
      r({ id: "c", status: "failed", path: undefined, error: "炸了" }),
      r({ id: "d" }),
    ]);
    expect(list.map((v) => v.id)).toEqual(["a", "d"]);
  });

  it("点第 4 张卡（前面夹着两个没图的）打开的是它自己", () => {
    const results = [
      r({ id: "a" }),
      r({ id: "b", status: "running", path: undefined }),
      r({ id: "c", status: "failed", path: undefined }),
      r({ id: "d" }),
    ];
    const list = viewables(results);
    // 卡片位置是 3，可看列表里的位置是 1
    expect(results.findIndex((x) => x.id === "d")).toBe(3);
    expect(indexOfId(list, "d")).toBe(1);
    expect(list[indexOfId(list, "d")!].id).toBe("d");
  });

  it("还在跑的那张没有大图可看", () => {
    const list = viewables([r({ id: "a" }), r({ id: "b", status: "running", path: undefined })]);
    expect(indexOfId(list, "b")).toBeNull();
  });

  /** `done` 但没有 path 的也不能进 —— 进去就是一张碎图。 */
  it("done 但没有文件的也排除掉", () => {
    expect(viewables([r({ id: "a", path: undefined })])).toEqual([]);
  });

  it("mp4 认成视频", () => {
    const list = viewables([r({ id: "a", path: "/tmp/a.MP4" }), r({ id: "b" })]);
    expect(list.map((v) => v.isVideo)).toEqual([true, false]);
  });

  describe("翻页", () => {
    it("到头就停，不绕回去", () => {
      expect(step(0, -1, 3)).toBe(0);
      expect(step(2, 1, 3)).toBe(2);
    });

    it("中间正常翻", () => {
      expect(step(1, -1, 3)).toBe(0);
      expect(step(1, 1, 3)).toBe(2);
    });

    it("空列表不会算出负下标", () => {
      expect(step(0, -1, 0)).toBe(0);
      expect(step(0, 1, 0)).toBe(0);
    });
  });
});
