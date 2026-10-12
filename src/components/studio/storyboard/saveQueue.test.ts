import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createDebouncedSave, createSerialWriter } from "./saveQueue";

/** 一个手动结束的异步写：记录开始、结束的顺序 */
function slowWrite(log: string[], name: string) {
  let done!: () => void;
  let fail!: (e: Error) => void;
  const job = () =>
    new Promise<void>((res, rej) => {
      log.push(`start ${name}`);
      done = () => {
        log.push(`end ${name}`);
        res();
      };
      fail = (e) => {
        log.push(`fail ${name}`);
        rej(e);
      };
    });
  return { job, finish: () => done(), fail: (e: Error) => fail(e) };
}

const tick = () => new Promise((r) => setTimeout(r, 0));

describe("场面存盘：一个接一个写", () => {
  it("后发的等前一个写完才开始 —— 旧的不会晚于新的落盘", async () => {
    const w = createSerialWriter();
    const log: string[] = [];
    const a = slowWrite(log, "旧");
    const b = slowWrite(log, "新");
    void w.run(a.job);
    void w.run(b.job);
    await tick();
    expect(log).toEqual(["start 旧"]);
    a.finish();
    await tick();
    b.finish();
    await tick();
    expect(log).toEqual(["start 旧", "end 旧", "start 新", "end 新"]);
  });

  it("关窗等的是队列里全部，不只是最后一个；前一个失败不堵后面", async () => {
    const w = createSerialWriter();
    const log: string[] = [];
    const a = slowWrite(log, "1");
    const b = slowWrite(log, "2");
    void w.run(a.job).catch(() => undefined);
    void w.run(b.job);
    let idle = false;
    void w.idle().then(() => (idle = true));
    await tick();
    a.fail(new Error("磁盘满"));
    await tick();
    expect(idle).toBe(false);
    b.finish();
    await tick();
    expect(idle).toBe(true);
    expect(log).toEqual(["start 1", "fail 1", "start 2", "end 2"]);
  });
});

describe("校准存盘：停手再存，关窗立刻存完", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("调完滑块 800ms 内关窗：flush 立刻存、等它落盘", async () => {
    const saved: number[] = [];
    let value = 1.6;
    const s = createDebouncedSave(async () => void saved.push(value), 800, () => undefined);
    value = 1.7;
    s.schedule();
    value = 1.75;
    s.schedule();
    vi.advanceTimersByTime(300);
    expect(saved).toEqual([]);
    expect(s.pending()).toBe(true);
    await s.flush();
    expect(saved).toEqual([1.75]);
    expect(s.pending()).toBe(false);
    // 定时器已经取消，不会再存一次
    vi.advanceTimersByTime(2000);
    expect(saved).toEqual([1.75]);
  });

  it("到点照常存；最后一次存失败时 flush 报错（窗口据此不关）", async () => {
    const errors: unknown[] = [];
    let ok = true;
    const s = createDebouncedSave(async () => {
      if (!ok) throw new Error("资产被锁");
    }, 800, (e) => errors.push(e));
    ok = false;
    s.schedule();
    await vi.advanceTimersByTimeAsync(800);
    expect(errors).toHaveLength(1);
    await expect(s.flush()).rejects.toThrow("资产被锁");
    ok = true;
    s.schedule();
    await expect(s.flush()).resolves.toBeUndefined();
  });
});
