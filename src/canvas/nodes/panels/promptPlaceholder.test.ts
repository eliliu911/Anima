import { describe, expect, it } from "vitest";
import { promptPlaceholder } from "./VideoOperationPanel";

// 用户 2026-09-25：「直接按这种，在输入框留出说明就可以，不用黄色提示词」（照 LibTV）
describe("视频输入框的占位文字 = 这个模式该写什么", () => {
  const inf = (mode: string, via?: string) => ({ mode, via, reason: "", images: [] }) as never;
  it("每种模式一句，带 @ 的模式都提示 @引用素材", () => {
    expect(promptPlaceholder(inf("t2v"), false)).toBe("描述你想要生成的画面内容");
    expect(promptPlaceholder(inf("r2v"), false)).toBe("描述你想要生成的画面内容，@引用素材");
    expect(promptPlaceholder(inf("i2v"), false)).toContain("首帧之后");
    expect(promptPlaceholder(inf("fl2v"), false)).toContain("首帧到尾帧");
    expect(promptPlaceholder(inf("motion", "video"), false)).toContain("参考视频");
    expect(promptPlaceholder(inf("motion", "depth"), false)).toContain("深度");
  });
  it("上游传了文字下来时提示追加", () => {
    expect(promptPlaceholder(inf("r2v"), true)).toContain("追加");
  });
});
