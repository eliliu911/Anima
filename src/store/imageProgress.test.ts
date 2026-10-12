import { beforeEach, describe, expect, it } from "vitest";

import { progressSlot, useImageProgress, type ImageProgress } from "./imageProgress";

function p(stage: string, over: Partial<ImageProgress> = {}): ImageProgress {
  return { stage, label: stage, progress: 0, ...over };
}

describe("图片生成进度", () => {
  beforeEach(() => useImageProgress.getState().clear());

  it("成功之后不会被后来的失败盖回去", () => {
    const slot = progressSlot("方案二", 1024, 1024);
    const { apply } = useImageProgress.getState();

    apply(slot, p("done", { outputPath: "/tmp/b.png", progress: 1 }));
    // 重试那趟的尾巴晚到了一条 error —— 图已经出来了，不该改写成失败
    apply(slot, p("error", { message: "ComfyUI 超时" }));

    const cur = useImageProgress.getState().bySlot[slot];
    expect(cur.stage).toBe("done");
    expect(cur.outputPath).toBe("/tmp/b.png");
  });

  it("失败之后重试的新进度可以正常覆盖", () => {
    const slot = progressSlot("方案二", 1024, 1024);
    const { apply } = useImageProgress.getState();

    apply(slot, p("error", { message: "超时" }));
    apply(slot, p("running", { progress: 0.4 }));

    expect(useImageProgress.getState().bySlot[slot].stage).toBe("running");
  });

  /**
   * 归档键必须和卡片那边一致 —— 卡片按 `prompt + 尺寸` 去重（见
   * `getTurnImageCalls`）。两边算法不一致的话，进度会存进一个没人读的格子，
   * 卡片照样永远转骨架屏，**而且不报任何错**。
   */
  it("归档键按 prompt + 尺寸区分，同 prompt 不同尺寸不串", () => {
    expect(progressSlot("A", 1024, 1024)).toBe(progressSlot("A", 1024, 1024));
    expect(progressSlot("A", 1024, 1024)).not.toBe(progressSlot("A", 512, 512));
    expect(progressSlot("A", 1024, 1024)).not.toBe(progressSlot("B", 1024, 1024));
    // 缺省尺寸要落到和显式 1024 同一个格子 —— 后端不一定每条都带尺寸
    expect(progressSlot("A")).toBe(progressSlot("A", 1024, 1024));
  });

  it("不同 prompt 各存各的", () => {
    const { apply } = useImageProgress.getState();
    apply(progressSlot("A"), p("done"));
    apply(progressSlot("B"), p("error"));
    const all = useImageProgress.getState().bySlot;
    expect(all[progressSlot("A")].stage).toBe("done");
    expect(all[progressSlot("B")].stage).toBe("error");
  });
});
