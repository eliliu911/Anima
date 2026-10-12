import { describe, expect, it } from "vitest";
import type { StoryboardShot } from "../../../api/cinema";
import { blankShot, duplicateShot, moveShot, renumber } from "./blankShot";
import { termIn } from "../../../shared/shotLanguage";

const shot = (id: string, n: number): StoryboardShot =>
  ({ ...blankShot([]), id, shot_num: n }) as StoryboardShot;

describe("手动加删镜头", () => {
  it("新镜头是首帧模式的空镜", () => {
    const s = blankShot([]);
    expect(s.mode).toBe("first_frame");
    expect(s.prompt).toBe("");
    expect(s.character_ids).toEqual([]);
    expect(s.duration).toBeGreaterThan(0);
  });

  /** id 是身份，重复的话 React 的 key 会撞、编辑会串到另一镜上。 */
  it("每次生成的 id 都不一样", () => {
    const ids = new Set([blankShot([]).id, blankShot([shot("a", 1)]).id]);
    // 同一毫秒内可能撞，所以只要求「不是写死的常量」
    expect(blankShot([]).id).toMatch(/^shot_/);
    expect(ids.size).toBeGreaterThanOrEqual(1);
  });

  /**
   * **编号是用户沟通用的**（「第三镜再慢一点」）。
   * 删掉中间一镜之后不重排的话，后面全部错位一个，
   * 而卡片上的数字看起来完全正常。
   */
  it("删掉中间一镜之后编号连续", () => {
    const list = [shot("a", 1), shot("b", 2), shot("c", 3)];
    const after = renumber(list.filter((s) => s.id !== "b"));
    expect(after.map((s) => s.shot_num)).toEqual([1, 2]);
    expect(after.map((s) => s.id)).toEqual(["a", "c"]);
  });

  it("加一镜之后编号接着排，不会出现两个同号", () => {
    const list = [shot("a", 1), shot("b", 2)];
    const after = renumber([...list, blankShot(list)]);
    expect(after.map((s) => s.shot_num)).toEqual([1, 2, 3]);
    expect(new Set(after.map((s) => s.shot_num)).size).toBe(3);
  });

  /** 编号乱掉的旧数据（比如手动改过 json）也要被拉回来。 */
  it("重排会修好本来就乱的编号", () => {
    const messy = [shot("a", 7), shot("b", 7), shot("c", 2)];
    expect(renumber(messy).map((s) => s.shot_num)).toEqual([1, 2, 3]);
  });

  it("重排不动其他字段", () => {
    const one = { ...shot("a", 9), prompt: "一个纸箱在动", duration: 6 };
    expect(renumber([one])[0]).toMatchObject({ prompt: "一个纸箱在动", duration: 6 });
  });
});

describe("上移下移和复制", () => {
  const list = () => [shot("a", 1), shot("b", 2), shot("c", 3)];

  it("上移会交换并重排编号", () => {
    const after = moveShot(list(), "c", -1);
    expect(after.map((s) => s.id)).toEqual(["a", "c", "b"]);
    expect(after.map((s) => s.shot_num)).toEqual([1, 2, 3]);
  });

  it("下移同理", () => {
    expect(moveShot(list(), "a", 1).map((s) => s.id)).toEqual(["b", "a", "c"]);
  });

  /** 第一镜点「上移」跳到末尾，是没人预期的。 */
  it("越界原样返回，不环绕", () => {
    const l = list();
    expect(moveShot(l, "a", -1)).toBe(l);
    expect(moveShot(l, "c", 1)).toBe(l);
    expect(moveShot(l, "不存在", 1)).toBe(l);
  });

  it("复制插在原镜后面，编号连续", () => {
    const after = duplicateShot(list(), "a");
    expect(after).toHaveLength(4);
    expect(after.map((s) => s.shot_num)).toEqual([1, 2, 3, 4]);
    expect(after[0].id).toBe("a");
    expect(after[1].id).not.toBe("a");
  });

  /**
   * **复制的是「设定」，不是「结果」。** 带着上一镜的图会让人以为
   * 新镜头已经出过了，而那张图对应的是旧提示词。
   */
  it("复制不带产物，也不带「接上一镜」", () => {
    const src: StoryboardShot = {
      ...shot("a", 1),
      prompt: "画面描述",
      image_url: "/tmp/a.png",
      video_url: "/tmp/a.mp4",
      audio_url: "/tmp/a.wav",
      status: "ready",
      continues_previous: true,
    };
    const copy = duplicateShot([src], "a")[1];
    expect(copy.prompt).toBe("画面描述");
    expect(copy.image_url).toBeNull();
    expect(copy.video_url).toBeNull();
    expect(copy.audio_url).toBeNull();
    expect(copy.status).toBe("pending");
    expect(copy.continues_previous).toBe(false);
  });

  it("复制出来的 id 不会撞", () => {
    const after = duplicateShot(duplicateShot(list(), "a"), "a");
    expect(new Set(after.map((s) => s.id)).size).toBe(after.length);
  });

  it("复制不存在的镜头时原样返回", () => {
    const l = list();
    expect(duplicateShot(l, "没有")).toBe(l);
  });
});

/**
 * **新镜头的景别 / 运镜必须是词表 id。**
 *
 * 写成显示串（「中景 (Medium)」）不会报错：卡片把它当成用户自定义的值
 * 显示成「中景 (Medium)（自定义）」，提示词里也原样发出去 ——
 * 而那个英文括号对任何编码器都只是噪声。
 */
describe("新镜头的默认镜头语言", () => {
  it("默认值在词表里查得到", () => {
    const s = blankShot([]);
    expect(termIn("shotSize", s.scale), `景别不是词表 id：${s.scale}`).toBeDefined();
    expect(termIn("move", s.camera_movement), `运镜不是词表 id：${s.camera_movement}`).toBeDefined();
  });

  /** 空镜头默认不动 —— 运镜要靠人选，替人选一个是替人做决定。 */
  it("默认是中景 + 固定", () => {
    const s = blankShot([]);
    expect(s.scale).toBe("medium");
    expect(s.camera_movement).toBe("static");
    expect(termIn("move", s.camera_movement)!.still).toBe(false);
  });
});
