import { describe, expect, it } from "vitest";
import { upstreamShot, shotPromptParts } from "./shotLink";
import type { CanvasEdge, CanvasNode } from "../types";

const 分镜节点 = (id: string, shotId: string): CanvasNode =>
  ({
    id,
    type: "shot",
    position: { x: 0, y: 0 },
    data: { projectId: "p1", episodeId: "ep_01", shotId, label: `第 ${shotId} 镜`, frame: "画面", unresolved: [] },
  }) as CanvasNode;

const 图片节点 = (id: string): CanvasNode =>
  ({ id, type: "imageGen", position: { x: 0, y: 0 }, data: {} }) as CanvasNode;

const 连 = (from: string, to: string): CanvasEdge => ({ id: `${from}-${to}`, source: from, target: to });

describe("upstreamShot", () => {
  it("隔着几层图片节点也要找得到", () => {
    const nodes = [分镜节点("s", "shot-3"), 图片节点("a"), 图片节点("b")];
    const edges = [连("s", "a"), 连("a", "b")];
    expect(upstreamShot("b", nodes, edges)?.shotId).toBe("shot-3");
  });

  it("上游没有分镜节点时给 null —— 而不是随便挑一镜", () => {
    const nodes = [图片节点("a"), 图片节点("b")];
    expect(upstreamShot("b", nodes, [连("a", "b")])).toBeNull();
  });

  it("分镜还没挑镜头时不算数", () => {
    const n = 分镜节点("s", "");
    const nodes = [{ ...n, data: { ...n.data, shotId: "" } } as CanvasNode, 图片节点("a")];
    expect(upstreamShot("a", nodes, [连("s", "a")])).toBeNull();
  });

  it("连成环也要停下来，不能把界面转死", () => {
    const nodes = [图片节点("a"), 图片节点("b")];
    const edges = [连("a", "b"), 连("b", "a")];
    expect(upstreamShot("a", nodes, edges)).toBeNull();
  });

  it("接了远近两个分镜时取近的那个", () => {
    const nodes = [分镜节点("远", "shot-远"), 图片节点("中"), 分镜节点("近", "shot-近"), 图片节点("末")];
    const edges = [连("远", "中"), 连("中", "末"), 连("近", "末")];
    expect(upstreamShot("末", nodes, edges)?.shotId).toBe("shot-近");
  });
});

describe("shotPromptParts", () => {
  it("原样发后端那一段", () => {
    expect(shotPromptParts({ label: "", frame: "  中景，林深站在垫子上  ", unresolved: [] })).toEqual([
      "中景，林深站在垫子上",
    ]);
  });

  it("没取到就什么都不发 —— 占位串会被画进图里", () => {
    expect(shotPromptParts({ label: "第 1 镜", frame: "", unresolved: [] })).toEqual([]);
  });
});
