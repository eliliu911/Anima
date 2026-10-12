import { describe, expect, it, vi } from "vitest";
import type { StudioResult } from "./useStudioStore";
import { createResultPersistence, settledKey } from "./resultPersistence";

const r = (id: string, status: StudioResult["status"], path?: string) =>
  ({ id, status, path, mode: "t2i", engineId: "zimage", prompt: "", startedAt: 0 }) as StudioResult;

describe("结果历史的存读", () => {
  /**
   * **这是这个模块存在的唯一理由。**
   *
   * 画廊状态的初值是空数组。挂载时如果"保存"先于"读取"跑到，
   * 那一份空数组会把磁盘上几百条历史整个覆盖掉 —— 不报错，
   * 用户只会觉得"记录又没了"。
   */
  it("读回来之前不许写", async () => {
    const save = vi.fn();
    const p = createResultPersistence(async () => [r("a", "done", "/a.png")], save);
    expect(await p.persist([])).toBe(false);
    expect(save).not.toHaveBeenCalled();
  });

  it("读完之后才放行", async () => {
    const save = vi.fn();
    const p = createResultPersistence(async () => [r("a", "done", "/a.png")], save);
    const applied: StudioResult[][] = [];
    await p.hydrate((x) => applied.push(x));
    expect(applied[0].map((x) => x.id)).toEqual(["a"]);
    expect(await p.persist([r("b", "done", "/b.png")])).toBe(true);
    expect(save).toHaveBeenCalledOnce();
  });

  /**
   * 读失败也要放开写 —— 否则一次读失败会让这一整轮的新结果全都存不下，
   * 而那比"没有历史"糟得多。
   */
  it("读失败当作没有历史，但仍然放开写", async () => {
    const save = vi.fn();
    const p = createResultPersistence(async () => {
      throw new Error("坏了");
    }, save);
    const applied: StudioResult[][] = [];
    await expect(p.hydrate((x) => applied.push(x))).resolves.toBeUndefined();
    expect(applied).toHaveLength(0);
    expect(await p.persist([r("b", "done")])).toBe(true);
  });
});

describe("什么时候才值得写盘", () => {
  /**
   * 一次生成期间 `results` 会因为进度更新变很多次。每次都写盘是几百次
   * 没必要的 IO —— 而这些中间态读回来时本来就会被 `reconcile_history`
   * 丢掉（它把 pending / running 直接过滤）。
   */
  it("只有正在跑的在变时，判定为没变", () => {
    const a = [r("x", "running"), r("y", "done", "/y.png")];
    const b = [r("x", "running"), r("y", "done", "/y.png")];
    expect(settledKey(a)).toBe(settledKey(b));
    // 新来一个 running 也不算变化
    expect(settledKey([...a, r("z", "pending")])).toBe(settledKey(a));
  });

  it("跑完了就算变化", () => {
    const before = settledKey([r("x", "running")]);
    const after = settledKey([r("x", "done", "/x.png")]);
    expect(after).not.toBe(before);
  });

  it("失败也算变化 —— 失败原因是要留下来的", () => {
    expect(settledKey([r("x", "failed")])).not.toBe(settledKey([r("x", "running")]));
  });

  it("产物路径变了算变化（同一条被重新生成）", () => {
    expect(settledKey([r("x", "done", "/a.png")])).not.toBe(
      settledKey([r("x", "done", "/b.png")]),
    );
  });

  it("删掉一条算变化", () => {
    expect(settledKey([r("x", "done", "/x.png")])).not.toBe(settledKey([]));
  });
});
