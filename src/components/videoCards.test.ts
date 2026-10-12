import { describe, expect, it } from "vitest";

import { contentRendersVideo, getTurnVideoCalls } from "./ChatHistory";
import { videoProgressSlot } from "../store/videoProgress";

/**
 * 视频卡片的回归。
 *
 * 背景：`VideoArtifactCard` 早就写好并导出了，但**一个调用方都没有** ——
 * `ChatHistory` 里 `video` 出现 0 次。所以生成出来的视频只在模型自己把
 * markdown 抄进正文时才显示，和图片修好之前是同一个形态。
 */
describe("视频卡片", () => {
  const call = (over: Record<string, unknown> = {}) => ({
    id: "v1",
    name: "generate_video",
    arguments: JSON.stringify({ prompt: "狐狸跑过雪地", duration_secs: 5, ...over }),
  });

  it("从 generate_video 的调用里取出任务", () => {
    const got = getTurnVideoCalls([call()], []);
    expect(got).toHaveLength(1);
    expect(got[0].prompt).toBe("狐狸跑过雪地");
    expect(got[0].durationSecs).toBe(5);
  });

  /** animate_image 用的是 motion_prompt，不是 prompt */
  it("animate_image 的参数名不同也要认", () => {
    const got = getTurnVideoCalls(
      [
        {
          id: "v2",
          name: "animate_image",
          arguments: JSON.stringify({ image_path: "a.png", motion_prompt: "镜头缓慢推近" }),
        },
      ],
      [],
    );
    expect(got).toHaveLength(1);
    expect(got[0].prompt).toBe("镜头缓慢推近");
  });

  it("从工具返回里取出 mp4 路径", () => {
    const got = getTurnVideoCalls(
      [call()],
      [
        {
          callId: "v1",
          name: "generate_video",
          content: "![生成的视频](file:///tmp/a.mp4)\n\n- 已保存",
          ok: true,
        },
      ],
    );
    expect(got[0].path).toBe("file:///tmp/a.mp4");
  });

  /**
   * **只按 call id 匹配。**
   *
   * 图片那边曾经有条"对不上就随便找一条同名工具的结果"的兜底，后果是
   * 两张卡显示同一张图，而磁盘上那两个文件其实完全不同。宁可少显示。
   */
  it("call id 对不上时不认，宁可显示骨架屏", () => {
    const got = getTurnVideoCalls(
      [call()],
      [{ callId: "别的id", name: "generate_video", content: "![x](file:///tmp/b.mp4)", ok: true }],
    );
    expect(got[0].path).toBeUndefined();
  });

  /** 视频耗时长，超时重试比图片更容易发生 —— 重试要替换而不是追加 */
  it("同一个任务重试时替换而不是多一张卡", () => {
    const got = getTurnVideoCalls(
      [
        { ...call(), id: "v1" },
        { ...call(), id: "v2" },
      ],
      [{ callId: "v2", name: "generate_video", content: "![x](file:///tmp/ok.mp4)", ok: true }],
    );
    expect(got).toHaveLength(1);
    expect(got[0].path).toBe("file:///tmp/ok.mp4"); // 留下已出片的那一条
  });

  /**
   * **卡片查进度用的键必须和写入侧一致。**
   *
   * 写入侧（CommandBar）存 `${prompt} ${duration_secs}s`；读取侧原来传的是
   * `videoProgressSlot(alt || "")` —— 时长写死缺省值。于是只要模型给的
   * duration_secs 不是缺省值，进度就落进一个没人读的格子：卡片永远转骨架屏、
   * 失败提示也不显示，**而且不报任何错**。
   */
  it("卡片带出的时长能算出和写入侧一样的键", () => {
    const card = getTurnVideoCalls([call({ duration_secs: 5 })], [])[0];
    expect(videoProgressSlot(card.prompt, card.durationSecs)).toBe(
      videoProgressSlot("狐狸跑过雪地", 5),
    );
    // 而漏传时长就会错开 —— 这正是修掉的那个 bug
    expect(videoProgressSlot(card.prompt)).not.toBe(
      videoProgressSlot(card.prompt, card.durationSecs),
    );
  });

  it("非视频工具不产生卡片", () => {
    expect(getTurnVideoCalls([{ id: "x", name: "generate_image", arguments: "{}" }], [])).toHaveLength(0);
    expect(getTurnVideoCalls([], [])).toHaveLength(0);
  });
});

/**
 * 「播放一会儿就自动关闭」的回归。
 *
 * 判据原来是 `content.includes(".mp4")` —— 模型用大白话写一句
 * 「视频已保存到 /Users/…/a.mp4」就成立。流式期间卡片在、视频在播；
 * 流一结束条件成立，卡片被卸载，而正文里没有 markdown 可渲染，
 * 视频直接消失。**不报任何错。**
 */
describe("正文是否已经渲染出视频", () => {
  const path = "file:///tmp/fox.mp4";

  it("裸路径只是提到，不算渲染", () => {
    expect(contentRendersVideo(`视频已保存到 /tmp/fox.mp4，可以查看。`)).toBe(false);
    expect(contentRendersVideo(`已生成并写入 ${path}`)).toBe(false);
  });

  it("markdown 嵌入才算渲染", () => {
    expect(contentRendersVideo(`![狐狸](${path})`)).toBe(true);
    expect(contentRendersVideo(`<video src="${path}" controls />`)).toBe(true);
  });

  it("带 path 时只认指向这一段的那条", () => {
    const other = "file:///tmp/other.mp4";
    expect(contentRendersVideo(`![别的](${other})`, path)).toBe(false);
    expect(contentRendersVideo(`![这个](${path})`, path)).toBe(true);
  });

  it("正文里只有图片没有视频时不算", () => {
    expect(contentRendersVideo("![一张图](file:///tmp/a.png)")).toBe(false);
  });
});

/**
 * 「第二次生成一直卡在生成阶段」的回归。
 *
 * 卡片的失败态原来**只认 ComfyUI 的进度事件**。可有些失败发生在提交 ComfyUI
 * 之前（LoRA 名字对不上、模型挑不出来），那时一条进度都不会发 ——
 * 骨架屏就永远转下去，而且不报任何错。
 */
describe("工具失败要传到卡片上", () => {
  it("ok:false 的结果标记成 failed", () => {
    const got = getTurnVideoCalls(
      [{ id: "v1", name: "generate_video", arguments: JSON.stringify({ prompt: "x" }) }],
      [{ callId: "v1", name: "generate_video", content: "LoRA 对不上", ok: false }],
    );
    expect(got[0].failed).toBe(true);
    expect(got[0].path).toBeUndefined();
  });

  it("成功的结果不标 failed", () => {
    const got = getTurnVideoCalls(
      [{ id: "v1", name: "generate_video", arguments: JSON.stringify({ prompt: "x" }) }],
      [{ callId: "v1", name: "generate_video", content: "![v](file:///tmp/a.mp4)", ok: true }],
    );
    expect(got[0].failed).toBe(false);
  });
});
