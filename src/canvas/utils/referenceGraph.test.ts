import { beforeEach, expect, it } from "vitest";
import { useCanvasStore } from "../store/useCanvasStore";
import { nodeReference } from "./referenceBindings";
import { videoInputImages, videoInputReferences } from "../engine/dagExecutor";
import { compilePromptMentions } from "./referenceMentions";
import { inferVideoMode } from "./videoMode";

beforeEach(() => useCanvasStore.getState().clearCanvas());
/**
 * 拖两张上传的图进视频节点 → 参考生视频，不需要先出图。
 *
 * **钉的是推导结果，不是存下来的 `mode` 字段。** 绑定这一步以前会无条件
 * 把 `mode` 写成 `r2v`（连一张首帧也是），现在它只写 `auto`，
 * 由 `inferVideoMode` 从连线推导 —— 用户实际拿到的是推导结果。
 */
it("dragging uploaded image inputs to video selects R2V without requiring generated outputs", () => {
  const s = useCanvasStore.getState();
  const a = s.addNode("imageGen", undefined, { references: [{ path: "/a.png", name: "A" }] });
  const b = s.addNode("imageGen", undefined, { references: [{ path: "/b.png", name: "B" }] });
  const v = s.addNode("videoGen", undefined, { mode: "auto" });
  for (const source of [a, b]) s.onConnect({ source, target: v, sourceHandle: "image-out", targetHandle: "prompt-in" });
  const graph = useCanvasStore.getState();
  expect(graph.nodes).toHaveLength(3);
  expect(graph.edges).toHaveLength(2);
  expect(inferVideoMode(v, graph.nodes, graph.edges).mode).toBe("r2v");
  expect(videoInputImages(v, graph.nodes, graph.edges)).toEqual(["/a.png", "/b.png"]);
});
it("creates multiple material cards linked to one video and disconnects without deleting material", () => {
  const s = useCanvasStore.getState();
  const id = s.addNode("videoGen");
  s.bindReference(s.activeProjectId, id, {path: "/one.png", name: "One"});
  s.bindReference(s.activeProjectId, id, {path: "/two.png", name: "Two"});
  const graph = useCanvasStore.getState();
  expect(graph.nodes.filter((n) => n.data.referenceOnly)).toHaveLength(2);
  expect(graph.edges).toHaveLength(2);
  expect(videoInputImages(id, graph.nodes, graph.edges)).toEqual(["/one.png", "/two.png"]);
  s.onEdgesChange([{id: graph.edges[0].id, type: "remove"}]);
  const after = useCanvasStore.getState();
  expect(after.nodes).toHaveLength(3);
  expect(videoInputImages(id, after.nodes, after.edges)).toEqual(["/two.png"]);
});
it("connects existing nodes without duplicating them and cleans binding on deletion", () => {
  const s = useCanvasStore.getState();
  const source = s.addNode("imageGen", undefined, {outputImageUrl: "/one.png"});
  const target = s.addNode("videoGen");
  s.bindReference(s.activeProjectId, target, nodeReference(useCanvasStore.getState().nodes[0]));
  expect(useCanvasStore.getState().nodes).toHaveLength(2);
  s.removeNode(source);
  expect(useCanvasStore.getState().edges).toHaveLength(0);
  expect(useCanvasStore.getState().nodes[0].data.references).toEqual([]);
});

/**
 * **编号和下发必须来自同一个数组。**
 *
 * 现状：`bindReference` 自己会去重，所以 `data.references` 的下标目前和实际
 * 下发顺序是一致的 —— 用哪个算 `<Picture N>` 暂时都对。**但那是巧合，不是
 * 保证**：下发数组还会沿图追加上游 imageGen 输出和角色头像，并按路径去重；
 * 只要哪天绑定端放开重复、或者追加顺序变一变，下标就整体前移，`<Picture 3>`
 * 指到第 2 张图上 —— 换了张脸，而任务成功、没有任何告警。
 *
 * 所以这里钉的是「编号依据 == 下发数组」这条不变量本身。
 */
it("参考图的编号依据就是实际下发的数组", () => {
  const s = useCanvasStore.getState();
  const v = s.addNode("videoGen", undefined, { mode: "r2v" });
  s.bindReference(s.activeProjectId, v, { path: "/xiao.png", name: "筱筱" });
  s.bindReference(s.activeProjectId, v, { path: "/xia.png", name: "夏禾" });
  const g = useCanvasStore.getState();

  const resolved = videoInputReferences(v, g.nodes, g.edges);
  // 名字跟着下发顺序一起出来，调用方不需要再去翻另一个数组
  expect(resolved).toEqual([
    { path: "/xiao.png", name: "筱筱" },
    { path: "/xia.png", name: "夏禾" },
  ]);
  // 老接口必须是新接口的投影，不能是第二份真相
  expect(videoInputImages(v, g.nodes, g.edges)).toEqual(resolved.map((r) => r.path));
  expect(compilePromptMentions("@筱筱 看向 @夏禾", resolved, "h3")).toBe(
    "<Picture 1> 看向 <Picture 2>",
  );
});

/** 沿图追加进来的上游图也占槽位 —— 它们没有名字，@ 不到，但会顶掉编号。 */
it("上游角色节点的头像也占一个 Picture 槽位", () => {
  const s = useCanvasStore.getState();
  const v = s.addNode("videoGen", undefined, { mode: "r2v" });
  const c = s.addNode("character", undefined, { name: "夏禾", avatarUrl: "/xia.png" });
  s.onConnect({ source: c, target: v, sourceHandle: "image-out", targetHandle: "prompt-in" });
  const g = useCanvasStore.getState();

  const resolved = videoInputReferences(v, g.nodes, g.edges);
  expect(resolved.map((r) => r.path)).toContain("/xia.png");
  // 角色节点带名字，所以它是可以被 @ 到的
  expect(resolved.find((r) => r.path === "/xia.png")?.name).toBe("夏禾");
});
