import { describe, expect, it } from "vitest";

import { srcFromProgress } from "./MarkdownRenderer";

describe("done 阶段直接出图", () => {
  it("done 带路径就给出图源，不必等工具返回", () => {
    expect(srcFromProgress({ stage: "done", outputPath: "/tmp/a.png" })).toBe("file:///tmp/a.png");
    // 已经是 file:// 的不重复加前缀
    expect(srcFromProgress({ stage: "done", outputPath: "file:///tmp/a.png" })).toBe("file:///tmp/a.png");
  });

  it("没到 done、或 done 但没带路径，都不出图源", () => {
    expect(srcFromProgress({ stage: "running", outputPath: "/tmp/a.png" })).toBeUndefined();
    expect(srcFromProgress({ stage: "downloading" })).toBeUndefined();
    expect(srcFromProgress({ stage: "done" })).toBeUndefined();
    expect(srcFromProgress({ stage: "error", outputPath: "/tmp/a.png" })).toBeUndefined();
    expect(srcFromProgress(undefined)).toBeUndefined();
  });
});
