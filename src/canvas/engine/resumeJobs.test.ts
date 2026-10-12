import { describe, it, expect, vi } from "vitest";

vi.mock("../../api/canvasJobs", () => ({ waitForCanvasJob: vi.fn() }));
vi.mock("./canvasTasks", () => ({ liveJobs: new Set<string>() }));

import { applyJobResult } from "./resumeJobs";
import type { CanvasNode } from "../types";

describe("页面重新加载后取回后台任务的结果", () => {
  const video = { id: "v", type: "videoGen", position: { x: 0, y: 0 }, data: {} } as unknown as CanvasNode;

  it("长镜头回的是 video_path，也认（以前判成「没有产物」）", () => {
    const patch = applyJobResult(video, { video_path: "/ops/longtake_abc.mp4" } as never);
    expect(patch.outputVideoUrl).toBe("/ops/longtake_abc.mp4");
    expect(patch.error).toBeUndefined();
  });

  it("单条出片照旧按 path / paths", () => {
    expect(applyJobResult(video, { path: "/a.mp4" }).outputVideoUrl).toBe("/a.mp4");
    expect(applyJobResult(video, {}).error).toContain("没有产物");
  });
});
