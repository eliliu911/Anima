import { describe, expect, it } from "vitest";
import type { Job } from "../../../api/jobs";
import { bucketJobs, defaultTab, elapsedLabel, isBatchChild, pollMs } from "./jobBuckets";

const job = (id: string, status: Job["status"], at: number): Job => ({
  id,
  kind: "frame",
  status,
  project_id: "p1",
  episode_id: "e1",
  shot_id: "s1",
  shot_num: 1,
  prompt: "一个纸箱",
  created_at: at,
});

describe("任务分堆", () => {
  it("三堆各归各的，新的在前", () => {
    const b = bucketJobs([
      job("a", "running", 10),
      job("b", "done", 30),
      job("c", "running", 20),
      job("d", "failed", 5),
    ]);
    expect(b.running.map((j) => j.id)).toEqual(["c", "a"]);
    expect(b.done.map((j) => j.id)).toEqual(["b"]);
    expect(b.failed.map((j) => j.id)).toEqual(["d"]);
  });

  it("空列表不会崩，三堆都在", () => {
    expect(bucketJobs([])).toEqual({ running: [], done: [], failed: [] });
  });

  /**
   * **固定落在「运行中」的话，跑完一批点开队列会看到一个空列表。**
   * 而用户点开它，十有八九正是因为有东西没出来。
   */
  it("默认页签：有在跑看在跑，没在跑但有失败看失败", () => {
    expect(defaultTab(bucketJobs([job("a", "running", 1), job("b", "failed", 2)]))).toBe(
      "running",
    );
    expect(defaultTab(bucketJobs([job("b", "failed", 2), job("c", "done", 3)]))).toBe("failed");
    expect(defaultTab(bucketJobs([job("c", "done", 3)]))).toBe("done");
    expect(defaultTab(bucketJobs([]))).toBe("done");
  });

  it("耗时：没结束的算到现在，结束了的算到结束那一刻", () => {
    expect(elapsedLabel(job("a", "running", 100), 142)).toBe("42s");
    const done = { ...job("b", "done", 100), finished_at: 190 };
    expect(elapsedLabel(done, 999999)).toBe("1m 30s");
  });

  it("耗时：三档都对，且不出现负数", () => {
    expect(elapsedLabel(job("a", "running", 100), 100)).toBe("0s");
    expect(elapsedLabel(job("a", "running", 100), 50)).toBe("0s");
    expect(elapsedLabel(job("a", "running", 0), 60)).toBe("1m");
    expect(elapsedLabel(job("a", "running", 0), 3700)).toBe("1h 1m");
  });

  /** 一直 1 秒是白烧，一直 6 秒是「点了没反应」。 */
  it("轮询间隔：在跑时最勤，闲着且面板关着时最松", () => {
    expect(pollMs(2, false)).toBe(1000);
    expect(pollMs(2, true)).toBe(1000);
    expect(pollMs(0, true)).toBe(3000);
    expect(pollMs(0, false)).toBe(6000);
    expect(pollMs(0, false)).toBeGreaterThan(pollMs(0, true));
  });
});

describe("批量出片里的一镜不重复显示", () => {
  const base = { project_id: "p", episode_id: "e", prompt: "", created_at: 1, shot_num: 9 };
  const batch = { ...base, id: "b", kind: "video_batch", status: "running", shot_id: "", targets: ["s9", "s10"] } as Job;
  const child = { ...base, id: "v", kind: "video", status: "running", shot_id: "s9" } as Job;
  const single = { ...base, id: "x", kind: "video", status: "running", shot_id: "s1" } as Job;

  /** 用户截图：右下角两条、队列两条。 */
  it("在跑的子任务不进「运行中」，批量本身和不在这一批的照常", () => {
    const b = bucketJobs([batch, child, single]);
    expect(b.running.map((j) => j.id).sort()).toEqual(["b", "x"]);
  });

  it("跑完的子任务留在已完成 / 失败里；批量结束后不再算子任务", () => {
    expect(bucketJobs([batch, { ...child, status: "failed" }]).failed.map((j) => j.id)).toEqual(["v"]);
    expect(isBatchChild(child, [{ ...batch, status: "done" }, child])).toBe(false);
    expect(isBatchChild(child, [{ ...batch, episode_id: "other" }, child])).toBe(false);
    expect(isBatchChild(child, [batch, child])).toBe(true);
  });

  it("批量出首帧里的一镜同理；首帧不算出片批量的子任务", () => {
    const frames = { ...batch, id: "fb", kind: "frame_batch" } as Job;
    const frame = { ...child, id: "f", kind: "frame" } as Job;
    expect(isBatchChild(frame, [frames, frame])).toBe(true);
    expect(isBatchChild(frame, [batch, frame])).toBe(false);
    expect(isBatchChild(child, [frames, child])).toBe(false);
  });
});
