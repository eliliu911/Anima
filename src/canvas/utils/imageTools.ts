import { useCanvasStore } from "../store/useCanvasStore";
import { executeSingleNode } from "../engine/dagExecutor";
import { nodeMedia } from "./upscaleChain";
import { DEFAULT_LOCAL_EDIT_ENGINE, LOCAL_EDIT_TEMPLATE } from "./localEdit";

/**
 * 图片节点工具条上的工具。每个都**在右边新建一个图片节点**、把原图接进去再跑 ——
 * 原图不动，结果能接着往下连。
 *
 * | 工具 | 走哪 | 实测 |
 * |---|---|---|
 * | 抠图 | BiRefNet | 3 秒 |
 * | 超分 ×2 | RealESRGAN | 2 秒 |
 * | 四视图 | Krea 2 QuadView（大脸特写 + 正/侧/背） | 约 5 分钟 |
 * | 参考表 | Krea 2 DynamicSheet（七个区） | 约 7 分半 |
 */
export type ImageTool = "remove_bg" | "upscale" | "quad_view" | "entity_sheet";

export const IMAGE_TOOLS: { id: ImageTool; label: string; hint: string }[] = [
  { id: "remove_bg", label: "抠图", hint: "去掉背景，出透明底 PNG · 约 3 秒" },
  { id: "upscale", label: "超分 ×2", hint: "放大一倍，细节更锐 · 约 2 秒" },
  { id: "quad_view", label: "四视图", hint: "照这个人出大脸特写 + 正 / 侧 / 背 · Krea 2 约 5 分钟" },
  { id: "entity_sheet", label: "参考表", hint: "照这个人出七区参考表（转身、动作、表情、细节）· Krea 2 约 7 分半" },
];

export function toolLabel(tool: ImageTool): string {
  return IMAGE_TOOLS.find((t) => t.id === tool)?.label ?? tool;
}

/**
 * 局部修改：在右边新建一个编辑节点，接上原图，**不立刻跑**。
 *
 * 和上面那些工具不一样 —— 它们参数都是定的，点了就能跑；
 * 局部修改必须先说清改什么，所以只把模板预填进提示词框，等用户写完再跑。
 *
 * 引擎写成 `qwen21_edit`：一张图 + 一句指令走的是 `ref2i`，
 * **只有 qwen21_edit / krea2_edit 吃这个模式**。
 */
export function spawnImageEdit(sourceNodeId: string): string | null {
  const store = useCanvasStore.getState();
  const source = store.nodes.find((n) => n.id === sourceNodeId);
  if (!source) return null;
  const pos = { x: source.position.x + (source.measured?.width ?? 440) + 60, y: source.position.y };
  const id = store.addNode("imageGen", pos, {
    engine: DEFAULT_LOCAL_EDIT_ENGINE,
    label: "局部修改",
    prompt: LOCAL_EDIT_TEMPLATE,
    // 记下源图：出完之后拿它和结果并排比，看「不该变的动没动」
    editSourcePath: nodeMedia(source)?.path,
  });
  store.setEdges([
    ...useCanvasStore.getState().edges,
    { id: `edit-${sourceNodeId}-${id}`, source: sourceNodeId, target: id, sourceHandle: "image-out", targetHandle: "prompt-in" },
  ]);
  // **故意不跑。** 模板里还留着 `{要改的东西}` 占位，照这个发出去
  // 会得到一张字面画着「要改的东西」的图。
  return id;
}

/** 在源节点右边新建一个工具节点，接上原图，立刻跑。返回新节点 id。 */
export function spawnImageTool(sourceNodeId: string, tool: ImageTool): string | null {
  if (tool === "upscale") return spawnUpscale(sourceNodeId);
  const store = useCanvasStore.getState();
  const source = store.nodes.find((n) => n.id === sourceNodeId);
  if (!source) return null;
  const pos = { x: source.position.x + (source.measured?.width ?? 440) + 60, y: source.position.y };
  const id = store.addNode("imageGen", pos, { imageTool: tool, label: toolLabel(tool) });
  store.setEdges([
    ...useCanvasStore.getState().edges,
    { id: `tool-${sourceNodeId}-${id}`, source: sourceNodeId, target: id, sourceHandle: "image-out", targetHandle: "prompt-in" },
  ]);
  void executeSingleNode(id);
  return id;
}

/** 在视频节点右边新建一个补帧节点，接上原视频，立刻跑。 */
export function spawnVideoInterpolate(sourceNodeId: string): string | null {
  return spawnVideoTool(sourceNodeId, "interpolate");
}

/**
 * 节点右侧输出接口的名字。**连线的 `sourceHandle` 要和源节点的接口对上**，
 * 对不上的话线画在错的位置或者干脆不显示（xyflow 找不到那个接口）。
 * 以前这里各处写死：补帧写死 `video-out`、超分只分视频和「其它」—— 从超分节点、
 * 角色节点往后接就对不上（2026-09-23 加「超分后补帧」时发现）。
 */
export function outHandleOf(type: string | undefined): string {
  switch (type) {
    case "videoGen": return "video-out";
    case "upscale": return "upscale-out";
    case "character": return "char-out";
    default: return "image-out";
  }
}

/** 视频工具：补帧 ×2 / 超分 ×2。在右边新建节点、接上原视频、立刻跑 */
export function spawnVideoTool(sourceNodeId: string, tool: "interpolate" | "upscale"): string | null {
  if (tool === "upscale") return spawnUpscale(sourceNodeId);
  const store = useCanvasStore.getState();
  const source = store.nodes.find((n) => n.id === sourceNodeId);
  if (!source) return null;
  const pos = { x: source.position.x + (source.measured?.width ?? 440) + 60, y: source.position.y };
  const id = store.addNode("videoGen", pos, { videoTool: tool, label: "补帧 ×2" });
  store.setEdges([
    ...useCanvasStore.getState().edges,
    { id: `tool-${sourceNodeId}-${id}`, source: sourceNodeId, target: id, sourceHandle: outHandleOf(source.type), targetHandle: "prompt-in" },
  ]);
  void executeSingleNode(id);
  return id;
}

/**
 * 在右边新建一个超分节点、接上源、立刻跑。图片工具条和视频节点上的「超分 ×2」都走这里 ——
 * 和从「+」菜单加出来的是同一种节点，不再是两套。
 */
export function spawnUpscale(sourceNodeId: string): string | null {
  const store = useCanvasStore.getState();
  const source = store.nodes.find((n) => n.id === sourceNodeId);
  if (!source) return null;
  const pos = { x: source.position.x + (source.measured?.width ?? 440) + 60, y: source.position.y };
  const id = store.addNode("upscale", pos, {});
  store.setEdges([
    ...useCanvasStore.getState().edges,
    { id: `tool-${sourceNodeId}-${id}`, source: sourceNodeId, target: id, sourceHandle: outHandleOf(source.type), targetHandle: "prompt-in" },
  ]);
  void executeSingleNode(id);
  return id;
}
