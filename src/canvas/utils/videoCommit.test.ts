import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import type { CanvasEdge, CanvasNode } from "../types";
import { planVideoCommit } from "./videoCommit";

const shotNode = (id: string): CanvasNode => ({
  id, type: "shot", position: { x: 0, y: 0 },
  data: { projectId: "p1", episodeId: "ep_01", shotId: "s3", label: "第 3 镜" },
} as CanvasNode);

const videoNode = (id: string, data: Record<string, unknown>): CanvasNode => ({
  id, type: "videoGen", position: { x: 0, y: 0 }, data,
} as CanvasNode);

const edge = (s: string, t: string): CanvasEdge => ({ id: `${s}-${t}`, source: s, target: t } as CanvasEdge);

describe("视频回填的判据", () => {
  it("上游有分镜、有产物就能收", () => {
    const nodes = [shotNode("sh"), videoNode("v", {
      outputVideoUrl: "/out/a.mp4",
      lastRun: { prompt: "实际发出去的正文", engine: "h3", images: ["/r/1.png", "/r/2.png"], width: 640, height: 384, durationSecs: 5, at: 1 },
    })];
    const r = planVideoCommit("v", nodes, [edge("sh", "v")]);
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.shot.shotId).toBe("s3");
    expect(r.prompt).toBe("实际发出去的正文");
    // **记的是哪几张，不是几张** —— 只有张数答不出「用了哪张场景图」
    expect(r.pictures).toEqual(["/r/1.png", "/r/2.png"]);
    expect(r.note).toBeUndefined();
  });

  // 图片那边的教训：没有落点就不显示按钮。点一下再收到「不知道收到哪儿」比不显示更糟。
  it("上游没接分镜就不给收，并说清原因", () => {
    const nodes = [videoNode("v", { outputVideoUrl: "/out/a.mp4" })];
    const r = planVideoCommit("v", nodes, []);
    expect(r.ok).toBe(false);
    if (r.ok) return;
    expect(r.why).toContain("分镜");
  });

  it("还在出片 / 还没有产物都不给收", () => {
    expect(planVideoCommit("v", [shotNode("sh"), videoNode("v", {})], [edge("sh", "v")]).ok).toBe(false);
    const busy = planVideoCommit("v", [shotNode("sh"), videoNode("v", { outputVideoUrl: "/a.mp4", isGenerating: true })], [edge("sh", "v")]);
    expect(busy.ok).toBe(false);
  });

  /**
   * **这一条是整个功能的要害。**
   *
   * 编辑框里的内容和真正发出去的差好几步。拿 `prompt` 顶替 `lastRun.prompt`，
   * 会写出一条看起来完整、其实是假的记录 —— 而且永远不会报错。
   */
  it("记的是实际发出去的那一份，不是编辑框里的", () => {
    const nodes = [shotNode("sh"), videoNode("v", {
      prompt: "编辑框里写的",
      outputVideoUrl: "/out/a.mp4",
      lastRun: { prompt: "<Picture 1> 设定为 筱筱。编辑框里写的, zimgt_trigger", engine: "h3", images: ["/r/1.png"], width: 640, height: 384, durationSecs: 5, at: 1 },
    })];
    const r = planVideoCommit("v", nodes, [edge("sh", "v")]);
    expect(r.ok && r.prompt).toContain("<Picture 1>");
    expect(r.ok && r.prompt).not.toBe("编辑框里写的");
  });

  it("上传进来的视频老实说没有出处，不拿编辑框的词充数", () => {
    const nodes = [shotNode("sh"), videoNode("v", { prompt: "编辑框里写的", outputVideoUrl: "/upload/a.mp4" })];
    const r = planVideoCommit("v", nodes, [edge("sh", "v")]);
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.prompt).toBe("");
    expect(r.note).toContain("不是在这个节点上出的");
  });
});

// ── 接线 ──────────────────────────────────────────────────────────
describe("接线", () => {
  it("视频节点真的用了这个判据，并调 cinemaAdoptVideo", () => {
    const src = readFileSync(new URL("../nodes/VideoGenNode.tsx", import.meta.url), "utf8");
    expect(src).toContain("planVideoCommit(");
    expect(src).toContain("cinemaAdoptVideo(");
  });
});
