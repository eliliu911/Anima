import { describe, expect, it } from "vitest";
import { buildLine, type BuildInfo } from "./BuildInfoFooter";

const b = (o: Partial<BuildInfo> = {}): BuildInfo => ({
  version: "0.9.0", git_sha: "abc1234", built_at: "2026-09-17 23:00 UTC", debug: false, ...o,
});

describe("构建信息那一行", () => {
  it("正式构建：版本 + sha + 时间", () => {
    expect(buildLine(b())).toBe("Anima 0.9.0 · abc1234 · 2026-09-17 23:00 UTC");
  });

  /**
   * **`-dirty` 必须显示出来。** 带未提交改动编出来的 0.9.0 和发布的 0.9.0
   * 不是一个东西 —— 用户报 bug 说「0.9.0 有问题」时，这个标记决定了
   * 要不要去翻他本地的改动。
   */
  it("脏工作区标出来，且 sha 里不重复带后缀", () => {
    const l = buildLine(b({ git_sha: "abc1234-dirty" }));
    expect(l).toContain("dirty");
    expect(l).toContain("abc1234");
    // 「abc1234-dirty · dirty」这种重复读起来像出错了
    expect(l).not.toContain("abc1234-dirty");
  });

  it("debug 构建标出来", () => {
    expect(buildLine(b({ debug: true }))).toContain("debug");
  });

  /** 两个标记同时出现时要能一起显示，不能互相顶掉。 */
  it("debug 和 dirty 能同时显示", () => {
    const l = buildLine(b({ debug: true, git_sha: "abc1234-dirty" }));
    expect(l).toContain("debug");
    expect(l).toContain("dirty");
  });

  /** 正式构建不该出现任何标记尾巴。 */
  it("正式构建没有多余的分隔符", () => {
    expect(buildLine(b())).not.toMatch(/·\s*·/);
    expect(buildLine(b())).not.toMatch(/·\s*$/);
  });
});
