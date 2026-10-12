import { describe, expect, it } from "vitest";
import { mergeHistory } from "./useResultHistory";
import type { StudioResult } from "./useStudioStore";

function r(id: string, extra: Partial<StudioResult> = {}): StudioResult {
  return {
    id,
    mode: "t2i",
    engineId: "zimage",
    prompt: id,
    status: "done",
    path: `/tmp/${id}.png`,
    startedAt: 0,
    ...extra,
  } as StudioResult;
}

describe("装回历史时的合并", () => {
  /**
   * 读盘是异步的，一两百毫秒足够用户点一次生成。直接覆盖会把那条正在跑的
   * 顶掉 —— 它还在跑，界面上却没有了，而且不报错。
   */
  it("挂载期间刚点的生成不会被顶掉", () => {
    const running = r("刚点的", { status: "running", path: undefined });
    const out = mergeHistory([running], [r("上次的")]);
    expect(out.map((x) => x.id)).toEqual(["刚点的", "上次的"]);
  });

  it("正在跑的那条排在最前，不会被挤到历史后面", () => {
    const out = mergeHistory([r("新")], [r("旧1"), r("旧2")]);
    expect(out[0].id).toBe("新");
  });

  /** 同 id 时内存那份是活的，磁盘那份是旧快照。 */
  it("同一个 id 以内存里那份为准", () => {
    const live = r("x", { status: "running", path: undefined });
    const stale = r("x", { status: "done", path: "/tmp/x.png" });
    const out = mergeHistory([live], [stale]);
    expect(out).toHaveLength(1);
    expect(out[0].status).toBe("running");
  });

  it("两边都空时是空的，不会凭空多出东西", () => {
    expect(mergeHistory([], [])).toEqual([]);
  });

  it("内存里空的时候历史原样进来", () => {
    expect(mergeHistory([], [r("a"), r("b")]).map((x) => x.id)).toEqual(["a", "b"]);
  });
});
