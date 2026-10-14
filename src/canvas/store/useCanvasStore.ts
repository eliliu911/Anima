import { DEFAULT_IMAGE_SIZE, sizeId } from "../utils/dimensionUtils";
import { absolutePosition, descendantIds, fitGroup as fitGroupPure, groupNodes as groupNodesPure, layoutImageGroup, ungroup as ungroupPure } from "../group";
import { create } from "zustand";
import {
  applyNodeChanges,
  applyEdgeChanges,
  addEdge,
  type Connection,
  type EdgeChange,
  type NodeChange,
} from "@xyflow/react";
import type { CanvasNode, CanvasEdge, CanvasNodeType, CanvasReference } from "../types";
import { ARRANGE_MODES, arrangeAll, type ArrangeMode } from "../utils/arrange";
import { nodeReference, referenceGraph } from "../utils/referenceBindings";

export const CANVAS_STORAGE_KEY_PREFIX = "anima:canvas:project:";
export const CANVAS_LAST_PROJECT_KEY = "anima:canvas:last_project_id";
/** 上一次「自动整理」用的排法 —— 只是本机的习惯，丢了就回到横向 */
export const ARRANGE_MODE_KEY = "anima:canvas:arrange_mode";

function loadArrangeMode(): ArrangeMode {
  try {
    const v = getStorage()?.getItem(ARRANGE_MODE_KEY);
    return ARRANGE_MODES.some((m) => m.id === v) ? (v as ArrangeMode) : "horizontal";
  } catch {
    return "horizontal";
  }
}

export interface CanvasPersistedData {
  version: number;
  projectId: string | null;
  nodes: CanvasNode[];
  edges: CanvasEdge[];
  updatedAt: number;
}

export function getStorage(): Storage | null {
  try {
    if (
      typeof window !== "undefined" &&
      window.localStorage &&
      typeof window.localStorage.getItem === "function"
    ) {
      return window.localStorage;
    }
  } catch {
    // ignore
  }
  try {
    if (
      typeof globalThis !== "undefined" &&
      globalThis.localStorage &&
      typeof (globalThis.localStorage as unknown as Storage).getItem === "function"
    ) {
      const s = globalThis.localStorage as unknown as Storage;
      s.getItem("__anima_probe__");
      return s;
    }
  } catch {
    // ignore
  }
  return null;
}

export function getCanvasStorageKey(projectId: string | null): string {
  const normalized = projectId && projectId.trim() ? projectId.trim() : "default";
  return `${CANVAS_STORAGE_KEY_PREFIX}${normalized}`;
}

type CanvasGraph = { nodes: CanvasNode[]; edges: CanvasEdge[] };
// Live graphs keep running jobs across project switches; disk snapshots never do.
const projectGraphs = new Map<string | null, CanvasGraph>();
const pendingBackgroundSaves = new Set<string | null>();

export function recoverNodes(nodes: CanvasNode[]): CanvasNode[] {
  return nodes.map((node) => node.data.jobId
    // **挂着后台任务的节点不算中断。** 任务在后端跑，页面重新加载后 resumeCanvasJobs 按 id 取回结果
    ? ({ ...node, data: { ...node.data, isGenerating: true, referenceLoading: false } } as CanvasNode)
    : ({ ...node, data: { ...node.data,
    isGenerating: false, referenceLoading: false,
    ...(node.data.isGenerating || node.data.referenceLoading
      ? { error: "上次生成或上传已中断，请重新执行" } : {}),
  } } as CanvasNode));
}

/**
 * 老画布的字段补全。**只补缺失的，不改已有的值。**
 *
 * `generateAudio` 是后加的字段。存量画布里的视频节点读出来是 `undefined`，
 * 而下发时走的是 `Boolean(data.generateAudio)` —— `undefined` 会变成 `false`。
 * 于是用户一个开关都没动过，重新生成就静音了，而且没有任何提示。
 *
 * 加这个字段**之前**的行为是「有音轨」（后端缺省 `true`），所以缺失时补
 * `true` 才是保持原状。新建节点由 `addNode` 显式写 `false`，不受这里影响。
 */
/**
 * 存档格式版本。**2 = r2v 的迁移已经做过了**（见下面 `migratePersistedNodes`）。
 * 那条迁移以前每次载入都跑：模板里的参考生视频节点（设定卡走连线、节点上没绑 references）
 * 一刷新就被改回 auto，推成图生视频 —— 设定卡整张当了首帧（用户 2026-09-25 撞到）。
 */
export const CANVAS_DATA_VERSION = 2;

function migratePersistedNodes(nodes: CanvasNode[], fromVersion: number): CanvasNode[] {
  return nodes.map((node) => {
    if (node.type !== "videoGen") return node;
    let data = node.data;
    if (data.generateAudio === undefined) {
      data = { ...data, generateAudio: true };
    }
    // **老数据里的 r2v 大半不是用户选的。**
    //
    // 绑定参考图时曾经无条件把 mode 写成 `r2v` —— 连一张首帧也被标成
    // 参考生视频。而 r2v 要求多张带名字的身份参考，只连了一张的那些
    // 本来就跑不通（执行时会报「参考生视频需要连接至少一张图片」之类）。
    // 这些回到 `auto`，让推导说了算；真的连了多张的保持原样，
    // 那可能是用户自己选的。
    // **只对 2 版以前的存档做一次**：之后存下来的 r2v 都是用户 / 模板选的
    const refs = (data.references as unknown[] | undefined)?.length ?? 0;
    if (fromVersion < 2 && data.mode === "r2v" && refs < 2) {
      data = { ...data, mode: "auto" };
    }
    return data === node.data ? node : ({ ...node, data } as CanvasNode);
  });
}

function persistGraph(projectId: string | null, graph: CanvasGraph) {
  const storage = getStorage();
  if (!storage) throw new Error("本地存储不可用，数据尚未保存");
  const data: CanvasPersistedData = { version: CANVAS_DATA_VERSION, projectId,
    nodes: recoverNodes(graph.nodes), edges: graph.edges, updatedAt: Date.now() };
  storage.setItem(getCanvasStorageKey(projectId), JSON.stringify(data));
}

export function loadPersistedCanvas(projectId: string | null): {
  nodes: CanvasNode[];
  edges: CanvasEdge[];
} {
  const storage = getStorage();
  if (!storage) {
    return { nodes: [], edges: [] };
  }
  try {
    const raw = storage.getItem(getCanvasStorageKey(projectId));
    if (!raw) return { nodes: [], edges: [] };
    const parsed = JSON.parse(raw);
    if (parsed && Array.isArray(parsed.nodes) && Array.isArray(parsed.edges)) {
      const sanitizedEdges = parsed.edges.map((e: CanvasEdge) => ({
        ...e,
        sourceHandle: e.sourceHandle?.replace(/-plus$/, "") || e.sourceHandle,
        animated: true,
        style: { ...e.style, stroke: (e.style?.stroke === "#3b82f6" || !e.style?.stroke) ? "#d4d4d8" : e.style.stroke, strokeWidth: 2 },
      }));
      const fromVersion = typeof parsed.version === "number" ? parsed.version : 1;
      return { nodes: migratePersistedNodes(recoverNodes(parsed.nodes), fromVersion), edges: sanitizedEdges };
    }
  } catch (err) {
    console.warn("Failed to load persisted canvas for project", projectId, err);
  }
  return { nodes: [], edges: [] };
}

let saveTimer: ReturnType<typeof setTimeout> | null = null;

function scheduleAutoSave(
  get: () => CanvasState,
  set: (partial: Partial<CanvasState> | ((state: CanvasState) => Partial<CanvasState>)) => void,
) {
  if (!get().isSaving) set({ isSaving: true });
  if (saveTimer) {
    clearTimeout(saveTimer);
  }
  saveTimer = setTimeout(() => {
    saveTimer = null;
    get().saveCanvas();
  }, 400);
}

export interface CanvasState {
  bindReference: (projectId: string | null, targetId: string, reference: CanvasReference, replacePath?: string) => void;
  referenceTargetId: string | null;
  nodes: CanvasNode[];
  edges: CanvasEdge[];
  selectedNodeId: string | null;
  activeProjectId: string | null;
  isSaving: boolean;
  lastSavedAt: number | null;
  saveError: string | null;
  getProjectCanvas: (projectId: string | null) => CanvasGraph;
  updateProjectNodeData: (projectId: string | null, id: string, patch: Record<string, unknown>) => void;

  onNodesChange: (changes: NodeChange<CanvasNode>[]) => void;
  onEdgesChange: (changes: EdgeChange<CanvasEdge>[]) => void;
  onConnect: (connection: Connection) => void;

  addNode: (
    type: CanvasNodeType,
    position?: { x: number; y: number },
    initialData?: Record<string, unknown>,
  ) => string;
  updateNodeData: (id: string, patch: Record<string, unknown>) => void;
  removeNode: (id: string) => void;
  clearCanvas: () => void;
  setNodes: (nodes: CanvasNode[]) => void;
  setEdges: (edges: CanvasEdge[]) => void;
  setSelectedNodeId: (id: string | null) => void;

  loadCanvas: (projectId?: string | null) => void;
  saveCanvas: (projectId?: string | null) => boolean;
  setActiveProjectId: (projectId: string | null) => void;

  undo: () => void;
  redo: () => void;
  copySelectedNode: () => CanvasNode | null;
  pasteNode: () => string | null;
  /**
   * 当前选中的所有节点 id。**以 ReactFlow 的 `node.selected` 为准**；
   * 一个都没标记时才退回 `selectedNodeId`（程序里单选的那一个）。
   */
  selectedIds: () => string[];
  selectAllNodes: () => void;
  /** 删掉所有选中节点，一次进撤销栈。返回删了几个。 */
  removeSelectedNodes: () => number;
  /** 复制所有选中节点（连同它们**之间**的连线）。返回复制了几个。 */
  copySelectedNodes: () => number;
  /** 粘贴剪贴板。返回新节点 id。 */
  pasteClipboard: () => string[];
  /** ⌘G：把选中节点打成组。返回组 id，不够两个返回 null。 */
  groupSelected: () => string | null;
  /** ⇧⌘G：解散选中的组。返回解散了几个。 */
  ungroupSelected: () => number;
  ungroupNode: (groupId: string) => boolean;
  /** 组框包住成员（只撑大）。纯排版修正，不进撤销栈。 */
  fitGroup: (groupId: string) => void;
  /** 直接设几个节点的位置。拖动联动用，和拖动本身一样不进撤销栈 */
  moveNodes: (positions: Record<string, { x: number; y: number }>) => void;
  /**
   * 一批截图落到源节点右边：多张排网格成组，一张就单放。**一步撤销**。
   * 返回新组（或那张图）的 id。
   */
  addImageGroup: (
    sourceId: string,
    images: { path: string; label: string; data?: Record<string, unknown> }[],
    opts: { cols: number; label: string; sourceHandle?: string },
  ) => string | null;
  duplicateNode: (id: string) => string | null;
  /** 自动整理画布。不传 = 用上一次的排法（`arrangeMode`） */
  arrangeNodes: (mode?: ArrangeMode) => void;
  /** 上一次整理用的排法，默认横向 */
  arrangeMode: ArrangeMode;

  snapToGrid: boolean;
  setSnapToGrid: (snap: boolean) => void;
  toggleSnapToGrid: () => void;
  showEdges: boolean;
  setShowEdges: (show: boolean) => void;
  toggleShowEdges: () => void;
}


interface HistorySnapshot {
  nodes: CanvasNode[];
  edges: CanvasEdge[];
}

const historyPast: HistorySnapshot[] = [];
const historyFuture: HistorySnapshot[] = [];
/**
 * 剪贴板：**多个节点 + 它们之间的连线**。
 *
 * 原来只存一个节点，⌘+点击多选之后按 ⌘C 只复制到其中一个，
 * 而且节点之间连好的线粘贴后全没了。
 */
let clipboard: { nodes: CanvasNode[]; edges: CanvasEdge[] } | null = null;

function newNodeId(type: string | undefined): string {
  return `${type ?? "node"}-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`;
}

function snapshotGraph(nodes: CanvasNode[], edges: CanvasEdge[]): HistorySnapshot {
  return {
    nodes: JSON.parse(JSON.stringify(nodes)),
    edges: JSON.parse(JSON.stringify(edges)),
  };
}

function pushHistory(nodes: CanvasNode[], edges: CanvasEdge[]) {
  historyPast.push(snapshotGraph(nodes, edges));
  if (historyPast.length > 30) historyPast.shift();
  historyFuture.length = 0;
}

const storageInstance = getStorage();
const initialProjectId = (() => {
  try { return storageInstance?.getItem(CANVAS_LAST_PROJECT_KEY) || null; }
  catch { return null; }
})();

const initialGraph = loadPersistedCanvas(initialProjectId);

export const useCanvasStore = create<CanvasState>((set, get) => ({

  nodes: initialGraph.nodes,
  edges: initialGraph.edges,
  selectedNodeId: null,
  referenceTargetId: null,
  activeProjectId: initialProjectId,
  isSaving: false,
  lastSavedAt: null,
  saveError: null,
  snapToGrid: true,
  arrangeMode: loadArrangeMode(),
  setSnapToGrid: (snapToGrid) => set({ snapToGrid }),
  toggleSnapToGrid: () => set((state) => ({ snapToGrid: !state.snapToGrid })),
  showEdges: true,
  setShowEdges: (showEdges) => set({ showEdges }),
  toggleShowEdges: () => set((state) => ({ showEdges: !state.showEdges })),

  getProjectCanvas: (projectId) => {
    const state = get();
    if (state.activeProjectId === projectId) return { nodes: state.nodes, edges: state.edges };
    return projectGraphs.get(projectId) ?? loadPersistedCanvas(projectId);
  },

  bindReference: (projectId, targetId, reference, replacePath) => {
    const graph = get().getProjectCanvas(projectId);
    const next = referenceGraph(graph.nodes, graph.edges, targetId, reference, replacePath);
    if (get().activeProjectId === projectId) {
      pushHistory(graph.nodes, graph.edges);
      set({ ...next, selectedNodeId: targetId });
      scheduleAutoSave(get, set);
    } else {
      projectGraphs.set(projectId, next);
      pendingBackgroundSaves.add(projectId);
      try { persistGraph(projectId, next); pendingBackgroundSaves.delete(projectId); }
      catch (err) { set({ saveError: `参考保存失败：${err}` }); }
    }
  },

  updateProjectNodeData: (projectId, id, patch) => {
    if (get().activeProjectId === projectId) { get().updateNodeData(id, patch); return; }
    const graph = get().getProjectCanvas(projectId);
    if (!graph.nodes.some((n) => n.id === id)) return;
    const next = { ...graph, nodes: graph.nodes.map((n) => n.id === id
      ? { ...n, data: { ...n.data, ...patch } } as CanvasNode : n) };
    projectGraphs.set(projectId, next);
    pendingBackgroundSaves.add(projectId);
    try {
      persistGraph(projectId, next);
      pendingBackgroundSaves.delete(projectId);
    } catch (err) {
      set({ saveError: `项目 ${projectId ?? "自由创作"} 保存失败：${err}`, isSaving: false });
    }
  },

  onNodesChange: (changes) => {
    const removed = changes.filter((c) => c.type === "remove").map((c) => c.id);
    if (removed.length) get().onEdgesChange(get().edges.filter((e) => removed.includes(e.source) || removed.includes(e.target)).map((e) => ({ id: e.id, type: "remove" })));
    set((state) => ({
      nodes: applyNodeChanges(changes, state.nodes) as CanvasNode[],
    }));
    scheduleAutoSave(get, set);
  },

  onEdgesChange: (changes) => {
    set((state) => {
      const edges = applyEdgeChanges(changes, state.edges) as CanvasEdge[];
      const remainingIds = new Set(edges.map((edge) => edge.id));
      const removed = state.edges.filter((edge) => !remainingIds.has(edge.id));
      if (!removed.length) return { edges };
      const sourcesByTarget = new Map<string, Set<string>>();
      for (const edge of removed) {
        const sources = sourcesByTarget.get(edge.target) ?? new Set<string>();
        sources.add(edge.source);
        sourcesByTarget.set(edge.target, sources);
      }
      return { edges, nodes: state.nodes.map((node) => {
        const sources = sourcesByTarget.get(node.id);
        if (!sources || !Array.isArray(node.data.references)) return node;
        const refs = node.data.references as CanvasReference[];
        const references = refs.filter((ref) => !sources.has(ref.sourceNodeId!));
        return references.length === refs.length ? node : {
          ...node, data: { ...node.data, references },
        } as CanvasNode;
      }) };

    });
    scheduleAutoSave(get, set);
  },

  onConnect: (connection) => {
    const state = get();
    const source = state.nodes.find((n) => n.id === connection.source);
    const target = state.nodes.find((n) => n.id === connection.target);
    if (target?.type === "videoGen" && (source?.type === "imageGen" || source?.type === "character")) {
      try {
        get().bindReference(state.activeProjectId, target.id, nodeReference(source));
        get().updateNodeData(target.id, { error: undefined });
      } catch (err) { get().updateNodeData(target.id, { error: String(err) }); }
      return;
    }
    const normalizedSourceHandle =
      connection.sourceHandle?.replace(/-plus$/, "") || connection.sourceHandle;
    // 左侧「+」是输入口（`prompt-in-plus` 之类），连进来的线落到真正的输入口上
    const normalizedTargetHandle = connection.targetHandle?.replace(/-plus$/, "") || connection.targetHandle;
    const normalizedConnection = {
      ...connection,
      sourceHandle: normalizedSourceHandle,
      targetHandle: normalizedTargetHandle,
      animated: true,
      style: { stroke: "rgba(176, 176, 183, 0.45)", strokeWidth: 2 },
    };
    set((state) => ({
      edges: addEdge(normalizedConnection, state.edges) as CanvasEdge[],
    }));
    scheduleAutoSave(get, set);
  },

  addNode: (type, position, initialData = {}) => {
    const id = `${type}-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`;
    const shouldSnap = get().snapToGrid;
    const rawPos = position ?? { x: 260 + Math.random() * 80, y: 160 + Math.random() * 80 };
    const defaultPos = shouldSnap
      ? {
          x: Math.round(rawPos.x / 20) * 20,
          y: Math.round(rawPos.y / 20) * 20,
        }
      : rawPos;

    let defaultData: Record<string, unknown> = {};
    if (type === "prompt") {
      defaultData = {
        label: "提示词节点",
        prompt: "",
        scale: "medium_shot",
        cameraMovement: "static",
      };
    } else if (type === "character") {
      defaultData = {
        characterId: "custom_char",
        name: "新角色",
        promptTrigger: "",
        appearancePrompt: "",
      };
    } else if (type === "shot") {
      // 空的分镜节点**不发任何东西**。发一句「（还没挑镜头）」出去
      // 会被下游当成画面内容画进图里。
      defaultData = { label: "", frame: "", unresolved: [] };
    } else if (type === "shotContext") {
      // 空的上下文节点**不发任何东西** —— 发一句「空间：」出去比不发更糟。
      defaultData = { sceneName: "", viewMap: "", lock: "", blocking: "", state: "" };
    } else if (type === "skill") {
      defaultData = { picks: [] };
    } else if (type === "audioClip") {
      defaultData = { label: "音频素材" };
    } else if (type === "directorWorld") {
      defaultData = {
        correction: { roll: 0, pitch: 0, yaw: 0 },
        frontYawDeg: 0,
        aspect: "16:9",
        calibration: { eyeHeight: 1.6, scale: 1 },
        actors: [],
        outputImages: [],
      };
    } else if (type === "panoViewer") {
      // 相机默认看正前方、70°（≈26mm，接近人眼舒适视角）。
      // 校正全 0 —— 要不要校由人看着图定，猜一个歪的更糟。
      defaultData = {
        yawDeg: 0,
        pitchDeg: 0,
        fovDeg: 70,
        correction: { roll: 0, pitch: 0, yaw: 0 },
        frontYawDeg: 0,
        aspect: "16:9",
        outputImages: [],
      };
    } else if (type === "panorama") {
      // 没挑场景之前 `prompt` 是空的，所以这个节点**什么都不发** ——
      // 空提示词接到图片节点上只会白跑一次。
      defaultData = { sceneName: "", prompt: "", longSide: 2048, warnings: [] };
    } else if (type === "imageGen") {
      defaultData = {
        engine: "zimage",
        // 默认尺寸跟后端尺寸表走（1024×576，16:9）。以前是 1280×720 ——
        // 不在后端表里，而且 720 不是 32 的倍数。
        width: DEFAULT_IMAGE_SIZE.width,
        height: DEFAULT_IMAGE_SIZE.height,
        ratio: "16:9",
        resolution: sizeId(DEFAULT_IMAGE_SIZE.width, DEFAULT_IMAGE_SIZE.height),
        seed: Math.floor(Math.random() * 9999999),
      };
    } else if (type === "depthCapture" || type === "upscale") {
      defaultData = {};
    } else if (type === "lora") {
      // **默认 1.0 而不是 0** —— 0 是「挂上但先关掉」的对照用法，
      // 拖出来就默认关掉的话，人会以为节点坏了
      defaultData = { strength: 1 };
    } else if (type === "videoGen") {
      defaultData = {
        engine: "ltx",
        width: 768,
        height: 512,
        ratio: "16:9",
        resolution: "720P",
        durationSecs: 5,
        cameraMovement: "zoom_in",
        generateAudio: false,
        videoCount: 1,
      };
    }

    const newNode: CanvasNode = {
      id,
      type,
      position: defaultPos,
      data: { ...defaultData, ...initialData },
    } as CanvasNode;

    pushHistory(get().nodes, get().edges);
    set((state) => ({
      nodes: [...state.nodes, newNode],
      selectedNodeId: id,
    }));
    scheduleAutoSave(get, set);

    return id;
  },

  updateNodeData: (id, patch) => {
    set((state) => ({
      nodes: state.nodes.map((n) =>
        n.id === id ? ({ ...n, data: { ...n.data, ...patch } } as CanvasNode) : n,
      ),
    }));
    scheduleAutoSave(get, set);
  },

  removeNode: (id) => {
    pushHistory(get().nodes, get().edges);
    // 删组连同成员一起删 —— 只删框的话成员的 parentId 指向一个不存在的节点，
    // 坐标还是相对的，会整体跳到画布原点附近
    const ids = new Set([id, ...descendantIds(get().nodes, [id])]);
    get().onEdgesChange(get().edges.filter((e) => ids.has(e.source) || ids.has(e.target)).map((e) => ({ id: e.id, type: "remove" })));
    set((state) => ({
      nodes: state.nodes.filter((n) => !ids.has(n.id)),
      edges: state.edges.filter((e) => !ids.has(e.source) && !ids.has(e.target)),
      selectedNodeId: state.selectedNodeId && ids.has(state.selectedNodeId) ? null : state.selectedNodeId,
    }));
    scheduleAutoSave(get, set);
  },

  clearCanvas: () => {
    pushHistory(get().nodes, get().edges);
    if (saveTimer) { clearTimeout(saveTimer); saveTimer = null; }
    set({ nodes: [], edges: [], selectedNodeId: null });
    get().saveCanvas();
  },

  setNodes: (nodes) => {
    set({ nodes });
    scheduleAutoSave(get, set);
  },
  setEdges: (edges) => {
    const sanitized = edges.map((e) => ({
      ...e,
      sourceHandle: e.sourceHandle?.replace(/-plus$/, "") || e.sourceHandle,
      animated: true,
      style: { ...e.style, stroke: (e.style?.stroke === "#3b82f6" || !e.style?.stroke) ? "#d4d4d8" : e.style.stroke, strokeWidth: 2 },
    }));
    set({ edges: sanitized });
    scheduleAutoSave(get, set);
  },
  setSelectedNodeId: (id) => set({ selectedNodeId: id }),

  undo: () => {
    if (historyPast.length === 0) return;
    const prev = historyPast.pop()!;
    historyFuture.push(snapshotGraph(get().nodes, get().edges));
    set({ nodes: prev.nodes, edges: prev.edges, selectedNodeId: null });
    scheduleAutoSave(get, set);
  },

  redo: () => {
    if (historyFuture.length === 0) return;
    const next = historyFuture.pop()!;
    historyPast.push(snapshotGraph(get().nodes, get().edges));
    set({ nodes: next.nodes, edges: next.edges, selectedNodeId: null });
    scheduleAutoSave(get, set);
  },

  selectAllNodes: () => set((state) => ({
    nodes: state.nodes.map((node) => {
      const selected = !node.hidden && node.selectable !== false;
      return !!node.selected === selected ? node : { ...node, selected };
    }),
    selectedNodeId: null,
    referenceTargetId: null,
  })),

  selectedIds: () => {
    const state = get();
    // **有选中标记就只认选中标记，不和 selectedNodeId 取并集。**
    // selectedNodeId 会被 addNode 设成刚加的节点，而框选不更新它 ——
    // 取并集的话「加了个 C，再框选 A、B 按 ⌘G」会把 C 也拉进组、Delete 会把 C 也删掉。
    const flagged = state.nodes.filter((n) => n.selected).map((n) => n.id);
    if (flagged.length) return flagged;
    return state.selectedNodeId && state.nodes.some((n) => n.id === state.selectedNodeId)
      ? [state.selectedNodeId]
      : [];
  },

  removeSelectedNodes: () => {
    const picked = get().selectedIds();
    if (!picked.length) return 0;
    // 选中的组连同成员一起删
    const ids = new Set([...picked, ...descendantIds(get().nodes, picked)]);
    // **一次**进撤销栈 —— 逐个调 removeNode 的话删 5 个要按 5 次 ⌘Z
    pushHistory(get().nodes, get().edges);
    get().onEdgesChange(
      get().edges
        .filter((e) => ids.has(e.source) || ids.has(e.target))
        .map((e) => ({ id: e.id, type: "remove" as const })),
    );
    set((state) => ({
      nodes: state.nodes.filter((n) => !ids.has(n.id)),
      edges: state.edges.filter((e) => !ids.has(e.source) && !ids.has(e.target)),
      selectedNodeId: state.selectedNodeId && ids.has(state.selectedNodeId) ? null : state.selectedNodeId,
    }));
    scheduleAutoSave(get, set);
    return ids.size;
  },

  copySelectedNodes: () => {
    const state = get();
    const picked = state.selectedIds();
    if (!picked.length) return 0;
    // 复制组要带上成员
    const ids = new Set([...picked, ...descendantIds(state.nodes, picked)]);
    const byId = new Map(state.nodes.map((n) => [n.id, n] as const));
    const nodes = state.nodes
      .filter((n) => ids.has(n.id))
      .map((n) => {
        // 父节点没被一起复制（单独复制了组里的某个成员）：粘出来是顶层节点，
        // 坐标要换成绝对的，否则相对坐标会让它跑到原点附近
        if (n.parentId && !ids.has(n.parentId)) {
          return { ...n, parentId: undefined, extent: undefined, position: absolutePosition(n, byId) };
        }
        return n;
      });
    clipboard = JSON.parse(
      JSON.stringify({
        nodes,
        edges: state.edges.filter((e) => ids.has(e.source) && ids.has(e.target)),
      }),
    );
    return picked.length;
  },

  pasteClipboard: () => {
    if (!clipboard || !clipboard.nodes.length) return [];
    pushHistory(get().nodes, get().edges);
    const OFFSET = 40;
    const remap = new Map<string, string>();
    for (const n of clipboard.nodes) remap.set(n.id, newNodeId(n.type));
    const pasted = clipboard.nodes.map((n) => {
      const data = { ...n.data } as Record<string, unknown>;
      // 生成结果不跟着复制 —— 粘出来的是一份「待生成」的配置
      delete data.outputImageUrl;
      delete data.outputImages;
      delete data.outputVideoUrl;
      delete data.outputVideos;
      data.isGenerating = false;
      // 节点之间的引用也要换成新 id，否则粘出来的那组还指着原来那组
      if (Array.isArray(data.references)) {
        data.references = (data.references as CanvasReference[]).map((r) =>
          r.sourceNodeId && remap.has(r.sourceNodeId) ? { ...r, sourceNodeId: remap.get(r.sourceNodeId)! } : r,
        );
      }
      const inside = !!n.parentId && remap.has(n.parentId);
      return {
        ...n,
        id: remap.get(n.id)!,
        parentId: inside ? remap.get(n.parentId!)! : undefined,
        // 只选中顶层那几个；组里的成员跟着组走
        selected: !inside,
        // 组内成员的坐标相对组框，不用错开；错开顶层的就够了
        position: inside ? n.position : { x: n.position.x + OFFSET, y: n.position.y + OFFSET },
        data,
      } as CanvasNode;
    });
    const pastedEdges = clipboard.edges.map(
      (e) =>
        ({
          ...e,
          id: `e-${remap.get(e.source)}-${remap.get(e.target)}-${Math.random().toString(36).slice(2, 6)}`,
          source: remap.get(e.source)!,
          target: remap.get(e.target)!,
          selected: false,
        }) as CanvasEdge,
    );
    // 连续粘贴一路往右下错开，不叠在同一个位置
    const inClipboard = new Set(clipboard.nodes.map((n) => n.id));
    clipboard = {
      nodes: clipboard.nodes.map((n) =>
        n.parentId && inClipboard.has(n.parentId)
          ? n
          : { ...n, position: { x: n.position.x + OFFSET, y: n.position.y + OFFSET } },
      ),
      edges: clipboard.edges,
    };
    set((state) => ({
      nodes: [...state.nodes.map((n) => ({ ...n, selected: false })), ...pasted],
      edges: [...state.edges, ...pastedEdges],
      selectedNodeId: (pasted.filter((n) => n.selected).pop() ?? pasted[pasted.length - 1]).id,
    }));
    scheduleAutoSave(get, set);
    return pasted.map((n) => n.id);
  },

  groupSelected: () => {
    const state = get();
    const r = groupNodesPure(state.nodes, state.selectedIds(), { groupId: newNodeId("group") });
    if (!r) return null;
    pushHistory(state.nodes, state.edges);
    set({ nodes: r.nodes, selectedNodeId: r.groupId });
    scheduleAutoSave(get, set);
    return r.groupId;
  },

  ungroupNode: (groupId) => {
    const state = get();
    const r = ungroupPure(state.nodes, state.edges, groupId);
    if (!r) return false;
    pushHistory(state.nodes, state.edges);
    set({ nodes: r.nodes, edges: r.edges, selectedNodeId: state.selectedNodeId === groupId ? null : state.selectedNodeId });
    scheduleAutoSave(get, set);
    return true;
  },

  ungroupSelected: () => {
    const state = get();
    const groups = state.selectedIds().filter((id) => state.nodes.some((n) => n.id === id && n.type === "group"));
    if (!groups.length) return 0;
    pushHistory(state.nodes, state.edges);
    let nodes = state.nodes;
    let edges = state.edges;
    let done = 0;
    for (const g of groups) {
      const r = ungroupPure(nodes, edges, g);
      if (r) { nodes = r.nodes; edges = r.edges; done += 1; }
    }
    set({ nodes, edges, selectedNodeId: null });
    scheduleAutoSave(get, set);
    return done;
  },

  addImageGroup: (sourceId, images, opts) => {
    const state = get();
    const r = layoutImageGroup(
      state.nodes,
      state.edges,
      sourceId,
      images.map((img) => ({ ...img, id: newNodeId("imageGen") })),
      { cols: opts.cols, label: opts.label, sourceHandle: opts.sourceHandle, groupId: newNodeId("group") },
    );
    if (!r) return null;
    pushHistory(state.nodes, state.edges);
    set({ nodes: r.nodes, edges: r.edges, selectedNodeId: r.focusId });
    scheduleAutoSave(get, set);
    return r.focusId;
  },

  moveNodes: (positions) => {
    set((state) => ({
      nodes: state.nodes.map((n) => (positions[n.id] ? ({ ...n, position: positions[n.id] } as CanvasNode) : n)),
    }));
    scheduleAutoSave(get, set);
  },

  fitGroup: (groupId) => {
    const nodes = get().nodes;
    const next = fitGroupPure(nodes, groupId);
    // 纯排版修正：不进撤销栈（否则拖一下产生两条撤销记录）。要持久化，所以照常自动保存
    if (next !== nodes) {
      set({ nodes: next });
      scheduleAutoSave(get, set);
    }
  },

  copySelectedNode: () => {
    const n = get().copySelectedNodes();
    return n && clipboard ? clipboard.nodes[0] : null;
  },

  pasteNode: () => get().pasteClipboard()[0] ?? null,

  duplicateNode: (id) => {
    const state = get();
    const target = state.nodes.find((n) => n.id === id);
    if (!target) return null;
    pushHistory(state.nodes, state.edges);
    const newId = `${target.type}-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`;
    const newNode: CanvasNode = {
      ...JSON.parse(JSON.stringify(target)),
      id: newId,
      selected: true,
      position: {
        x: target.position.x + 40,
        y: target.position.y + 40,
      },
      data: {
        ...target.data,
        outputImageUrl: undefined,
        outputImages: undefined,
        outputVideoUrl: undefined,
        outputVideos: undefined,
        isGenerating: false,
      },
    } as CanvasNode;

    set((s) => ({
      nodes: [...s.nodes.map((n) => ({ ...n, selected: false })), newNode],
      selectedNodeId: newId,
    }));
    scheduleAutoSave(get, set);
    return newId;
  },

  arrangeNodes: (mode) => {
    const full = get();
    if (full.nodes.length === 0) return;
    const use = mode ?? get().arrangeMode;
    pushHistory(full.nodes, full.edges);
    // **只排顶层节点。** 组里的成员坐标是相对组框的，按绝对坐标重排会把它们甩出去；
    // 组作为一个整体参与排版。成员之间的连线折算成它们所在顶层组之间的连线。
    const topOf = (id: string): string => {
      let cur = full.nodes.find((n) => n.id === id);
      const seen = new Set<string>();
      while (cur?.parentId && !seen.has(cur.parentId)) {
        seen.add(cur.parentId);
        cur = full.nodes.find((n) => n.id === cur!.parentId);
      }
      return cur?.id ?? id;
    };
    const top = full.nodes.filter((n) => !n.parentId);
    const edges = full.edges
      .map((e) => ({ ...e, source: topOf(e.source), target: topOf(e.target) }))
      .filter((e) => e.source !== e.target);
    // 算法和三种排法见 `utils/arrange.ts`；多套工作流各排各的（`arrangeAll`），不混成一张图
    const pos = arrangeAll(top, edges, use);
    const newNodes = full.nodes.map((n) => (n.parentId || !pos.has(n.id) ? n : ({ ...n, position: pos.get(n.id)! } as CanvasNode)));

    // 记住这次用的排法：快捷键 ⌥⇧F 和直接点按钮都按它来
    try { getStorage()?.setItem(ARRANGE_MODE_KEY, use); } catch { /* 存不了就只在这次会话里记 */ }
    set({ nodes: newNodes, arrangeMode: use });
    scheduleAutoSave(get, set);
  },


  loadCanvas: (targetProjectId) => {
    const newProjectId = targetProjectId ?? null;
    const state = get();
    if (!get().saveCanvas()) return;
    projectGraphs.set(state.activeProjectId, { nodes: state.nodes, edges: state.edges });
    try {
      const storage = getStorage();
      if (!storage) throw new Error("本地存储不可用");
      if (newProjectId) storage.setItem(CANVAS_LAST_PROJECT_KEY, newProjectId);
      else storage.removeItem(CANVAS_LAST_PROJECT_KEY);
    } catch (err) {
      set({ saveError: `切换项目失败：${err}` });
      return;
    }
    const graph = projectGraphs.get(newProjectId) ?? loadPersistedCanvas(newProjectId);
    set({ activeProjectId: newProjectId, nodes: graph.nodes, edges: graph.edges,
      selectedNodeId: null, referenceTargetId: null, isSaving: false, saveError: null, lastSavedAt: null });
  },

  saveCanvas: (targetProjectId) => {
    if (saveTimer) { clearTimeout(saveTimer); saveTimer = null; }
    const state = get();
    const pid = targetProjectId !== undefined ? targetProjectId : state.activeProjectId;
    try {
      persistGraph(pid, { nodes: state.nodes, edges: state.edges });
      for (const backgroundId of pendingBackgroundSaves) {
        if (backgroundId !== state.activeProjectId) {
          const graph = projectGraphs.get(backgroundId);
          if (graph) persistGraph(backgroundId, graph);
        }
        pendingBackgroundSaves.delete(backgroundId);
      }
      set({ isSaving: false, saveError: null, lastSavedAt: Date.now() });
      return true;
    } catch (err) {
      set({ isSaving: false, saveError: `保存失败：${err}` });
      return false;
    }
  },

  setActiveProjectId: (projectId) => {
    get().loadCanvas(projectId);
  },
}));

if (typeof window !== "undefined" && typeof window.addEventListener === "function") {
  window.addEventListener("beforeunload", () => {
    try {
      useCanvasStore.getState().saveCanvas();
    } catch {
      // ignore
    }
  });
}
