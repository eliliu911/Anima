import { describe, expect, it, beforeEach } from "vitest";
import type { CanvasEdge, CanvasNode } from "../types";
import { upstreamCards, removeUpstreamCard, cardRole, swapFrameCards } from "./UpstreamCards";
import { videoInputReferences } from "../engine/dagExecutor";
import { useCanvasStore } from "../store/useCanvasStore";

const n = (id: string, type: string, data: Record<string, unknown> = {}): CanvasNode =>
  ({ id, type, position: { x: 0, y: 0 }, data } as CanvasNode);
const e = (s: string, t: string): CanvasEdge => ({ id: `${s}>${t}`, source: s, target: t } as CanvasEdge);

describe("面板顶上的上游卡片（照 LibTV）", () => {
  it("连着还没出图的图片节点也有一张卡（占位），编号 1", () => {
    const cards = upstreamCards("v", [n("v", "videoGen"), n("i", "imageGen")], [e("i", "v")]);
    expect(cards).toHaveLength(1);
    expect(cards[0]).toMatchObject({ kind: "image", index: 1, sourceNodeId: "i" });
    expect(cards[0].path).toBeUndefined();
  });

  it("编号顺序和真正发出去的 <Picture N> 一致：先绑定的参考图，再按连线顺序", () => {
    const nodes = [
      n("v", "videoGen", { references: [{ path: "/up.png", name: "上传的" }] }),
      n("a", "imageGen", { outputImageUrl: "/a.png" }), n("b", "imageGen", { outputImageUrl: "/b.png" }),
    ];
    const edges = [e("a", "v"), e("b", "v")];
    const cards = upstreamCards("v", nodes, edges);
    expect(cards.map((c) => c.path)).toEqual(videoInputReferences("v", nodes, edges).map((r) => r.path));
    expect(cards.map((c) => c.index)).toEqual([1, 2, 3]);
  });

  it("绑定了同一个上游节点的参考图和连线只算一张", () => {
    const nodes = [n("v", "videoGen", { references: [{ path: "/a.png", name: "林晚", sourceNodeId: "a" }] }), n("a", "imageGen", { outputImageUrl: "/a.png" })];
    expect(upstreamCards("v", nodes, [e("a", "v")])).toHaveLength(1);
  });

  it("视频单独编号（V1）；提示词节点也是一张卡（文本卡，不编号）", () => {
    const nodes = [n("v", "videoGen"), n("src", "videoGen", { outputVideoUrl: "/s.mp4" }), n("i", "imageGen"), n("p", "prompt", { prompt: "雨夜街头" })];
    const cards = upstreamCards("v", nodes, [e("src", "v"), e("i", "v"), e("p", "v")]);
    expect(cards.map((c) => [c.kind, c.index])).toEqual([["video", 1], ["image", 1], ["text", 0]]);
    expect(cards[2].label).toContain("雨夜街头");
  });

  /** 用户 09-24：「风格按钮点击打开选择后，作为风格卡片使用，LibTV 这里用的就是 LoRA」 */
  it("连进来的 LoRA 节点和「风格」按钮里挑的 LoRA 都是风格卡", () => {
    const nodes = [n("v", "imageGen", { loras: [{ name: "zimage/Pintrest_ZiT.safetensors", strength: 0.8 }] }), n("l", "lora", { name: "zimage/asianMix.safetensors", strength: 0.6 })];
    const cards = upstreamCards("v", nodes, [e("l", "v")]);
    expect(cards.map((c) => [c.kind, c.index])).toEqual([["style", 0], ["style", 0]]);
    expect(cards[0].label).toBe("asianMix");
    expect(cards[1]).toMatchObject({ loraName: "zimage/Pintrest_ZiT.safetensors" });
    expect(cards[1].label).toContain("Pintrest_ZiT");
  });

  it("图片的编号不被文本卡、风格卡挤占", () => {
    const nodes = [n("v", "videoGen"), n("p", "prompt", { prompt: "x" }), n("a", "imageGen", { outputImageUrl: "/a.png" }), n("l", "lora", {}), n("b", "imageGen")];
    const cards = upstreamCards("v", nodes, [e("p", "v"), e("a", "v"), e("l", "v"), e("b", "v")]);
    expect(cards.filter((c) => c.kind === "image").map((c) => c.index)).toEqual([1, 2]);
  });
});

describe("点 × 断开", () => {
  beforeEach(() => useCanvasStore.getState().clearCanvas());

  it("连线和绑在节点上的那条参考一起去掉", () => {
    const s = useCanvasStore.getState();
    const img = s.addNode("imageGen", { x: 0, y: 0 }, { outputImageUrl: "/a.png" });
    const vid = s.addNode("videoGen", { x: 500, y: 0 }, {});
    s.setEdges([e(img, vid)]);
    s.updateNodeData(vid, { references: [{ path: "/a.png", name: "林晚", sourceNodeId: img }] });
    const card = upstreamCards(vid, useCanvasStore.getState().nodes, useCanvasStore.getState().edges)[0];
    removeUpstreamCard(vid, card);
    const st = useCanvasStore.getState();
    expect(st.edges.filter((x) => x.target === vid)).toEqual([]);
    expect(st.nodes.find((x) => x.id === vid)!.data.references).toEqual([]);
  });

  it("风格卡的 × 是取消挂载这个 LoRA，别的 LoRA 留着", () => {
    const s = useCanvasStore.getState();
    const img = s.addNode("imageGen", { x: 0, y: 0 }, { loras: [{ name: "a.safetensors", strength: 1 }, { name: "b.safetensors", strength: 0.5 }] });
    const card = upstreamCards(img, useCanvasStore.getState().nodes, useCanvasStore.getState().edges).find((c) => c.loraName === "a.safetensors")!;
    removeUpstreamCard(img, card);
    expect(useCanvasStore.getState().nodes.find((x) => x.id === img)!.data.loras).toEqual([{ name: "b.safetensors", strength: 0.5 }]);
  });

  it("直接上传的参考图（没有上游节点）按路径去掉", () => {
    const s = useCanvasStore.getState();
    const vid = s.addNode("videoGen", { x: 0, y: 0 }, {});
    s.updateNodeData(vid, { references: [{ path: "/x.png", name: "x" }, { path: "/y.png", name: "y" }] });
    const card = upstreamCards(vid, useCanvasStore.getState().nodes, useCanvasStore.getState().edges)[0];
    removeUpstreamCard(vid, card);
    expect(useCanvasStore.getState().nodes.find((x) => x.id === vid)!.data.references).toEqual([{ path: "/y.png", name: "y" }]);
  });
});

// 用户 2026-09-25：「我用两张角色图，你能分清是首尾帧还是参考生视频吗？」—— 角标写这张图在这次生成里干什么
describe("视频节点的图片卡角标：首帧 / 尾帧 / 编号", () => {
  beforeEach(() => useCanvasStore.getState().clearCanvas());

  const twoImages = () => {
    const s = useCanvasStore.getState();
    const a = s.addNode("imageGen", { x: 0, y: 0 }, { referenceOnly: true, label: "开场.png", outputImageUrl: "/a.png" });
    const b = s.addNode("imageGen", { x: 0, y: 400 }, { referenceOnly: true, label: "结尾.png", outputImageUrl: "/b.png" });
    return { a, b };
  };

  it("首尾帧：按实际发出去的两张标；对调后首尾真的换了", async () => {
    const { inferVideoMode } = await import("../utils/videoMode");
    const { a, b } = twoImages();
    const s = useCanvasStore.getState();
    const v = s.addNode("videoGen", { x: 600, y: 0 }, { mode: "fl2v" });
    s.setEdges([{ id: "ea", source: a, target: v }, { id: "eb", source: b, target: v }]);
    const roles = () => {
      const st = useCanvasStore.getState();
      const inf = inferVideoMode(v, st.nodes, st.edges);
      return { sent: inf.images, roles: upstreamCards(v, st.nodes, st.edges).map((c) => cardRole(c, inf.mode, inf.images)) };
    };
    expect(roles()).toEqual({ sent: ["/a.png", "/b.png"], roles: ["首帧", "尾帧"] });

    const st = useCanvasStore.getState();
    const [c1, c2] = upstreamCards(v, st.nodes, st.edges);
    expect(swapFrameCards(v, c1, c2)).toBe(true);
    const after = roles();
    expect(after.sent).toEqual(["/b.png", "/a.png"]);
    // 卡片顺序跟着连线换了，角标仍是「首帧、尾帧」—— 现在第一张是结尾那张图
    expect(after.roles).toEqual(["首帧", "尾帧"]);
    expect(upstreamCards(v, useCanvasStore.getState().nodes, useCanvasStore.getState().edges)[0].path).toBe("/b.png");
  });

  it("图生视频标「首帧」；图片参考标编号（和 @图片N 一致）", () => {
    const card = (index: number, path: string) => ({ key: `k${index}`, kind: "image" as const, index, path, label: "" });
    expect(cardRole(card(1, "/a.png"), "i2v", ["/a.png"])).toBe("首帧");
    expect(cardRole(card(2, "/b.png"), "r2v", ["/a.png", "/b.png"])).toBe("2");
    expect(cardRole(card(3, "/c.png"), "fl2v", ["/a.png", "/b.png"])).toBe("不用");
  });

  it("路径写法不同（anima-media:// 和本地路径）也认得出是同一张", () => {
    const card = { key: "k", kind: "image" as const, index: 2, path: "anima-media://localhost/?p=%2Ftmp%2Fend.png", label: "" };
    expect(cardRole(card, "fl2v", ["/tmp/start.png", "/tmp/end.png"])).toBe("尾帧");
  });
});
