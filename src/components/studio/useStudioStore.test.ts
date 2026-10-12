import { describe, expect, it } from "vitest";
import { canDispatch, type StudioResult } from "./useStudioStore";

const r = (status: StudioResult["status"]): StudioResult => ({
  id: Math.random().toString(36), mode: "t2i", engineId: "zimage",
  prompt: "x", status, startedAt: 0,
});

describe("并发闸门", () => {
  // ComfyUI 串行执行；并发发三个会让后面的计时器空转成"超时"
  it("在飞的任务达到上限就不再发", () => {
    expect(canDispatch([], 1)).toBe(true);
    expect(canDispatch([r("running")], 1)).toBe(false);
    expect(canDispatch([r("pending")], 1)).toBe(false);
    // 已完成和失败的不占额度
    expect(canDispatch([r("done"), r("failed")], 1)).toBe(true);
    expect(canDispatch([r("running")], 2)).toBe(true);
    expect(canDispatch([r("running"), r("pending")], 2)).toBe(false);
  });

  it("并发上限至少是 1，配置成 0 也不能死锁", () => {
    expect(canDispatch([], 0)).toBe(true);
    expect(canDispatch([], -5)).toBe(true);
  });
});

import { reconcileParams } from "./useStudioStore";

const eng = (o: Partial<Parameters<typeof reconcileParams>[0]> = {}) => ({
  sizes: [{ width: 768, height: 512 }, { width: 1344, height: 768 }],
  duration: { min: 1, max: 20, step: 0.5, default: 5 },
  max_batch: 2,
  ...o,
});

describe("换引擎后的参数收敛", () => {
  // 无脑重置 = 每次切引擎都要重设；无脑保留 = 把 1344×768 带到只支持 640×384 的引擎上
  it("不越界的留住，越界的换成新引擎的默认", () => {
    const keep = reconcileParams(eng(), {
      size: { width: 1344, height: 768 }, duration: 15, batch: 2,
    });
    expect(keep.size).toEqual({ width: 1344, height: 768 });
    expect(keep.duration).toBe(15);
  });

  // 无脑重置会让人每次切引擎都重设一遍；夹进范围才对
  it("时长夹进新引擎的范围而不是弹回默认", () => {
    const h3 = eng({ duration: { min: 2.5, max: 15, step: 0.5, default: 5 } });
    expect(reconcileParams(h3, { size: null, duration: 20, batch: 1 }).duration)
      .toBe(15);
    expect(reconcileParams(h3, { size: null, duration: 1, batch: 1 }).duration)
      .toBe(2.5);
    // 在范围里的原样留住
    expect(reconcileParams(h3, { size: null, duration: 8, batch: 1 }).duration)
      .toBe(8);

    const swap = reconcileParams(eng({ sizes: [{ width: 640, height: 384 }] }), {
      size: { width: 1344, height: 768 }, duration: 15, batch: 2,
    });
    expect(swap.size).toEqual({ width: 640, height: 384 });
  });

  it("出图引擎不该留下时长", () => {
    const r = reconcileParams(eng({ duration: null }), { size: null, duration: 15, batch: 1 });
    expect(r.duration).toBeNull();
  });

  it("批量夹在 1 和引擎上限之间", () => {
    expect(reconcileParams(eng({ max_batch: 2 }), { size: null, duration: null, batch: 4 }).batch).toBe(2);
    expect(reconcileParams(eng(), { size: null, duration: null, batch: 0 }).batch).toBe(1);
  });
});
