import { memo, useEffect, useMemo, useState, useRef } from "react";
import {
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
  Background,
  MiniMap,
  BackgroundVariant,
  ConnectionMode,
  type NodeTypes,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import "./canvas-sharpness.css";

import { useShallow } from "zustand/react/shallow";
import { useCanvasStore } from "./store/useCanvasStore";
import { SpawnMenu, useSpawnLinked } from "./ui/SpawnMenu";
import { spawnContextOf } from "./ui/NodeSpawnPlusOverlay";
import { spawnChoices, type SpawnDirection } from "./utils/spawnLinked";
import { selectDisplayEdges } from "./store/selectDisplayEdges";
import { PromptNodeComponent } from "./nodes/PromptNode";
import { CharacterNodeComponent } from "./nodes/CharacterNode";
import { ImageGenNodeComponent } from "./nodes/ImageGenNode";
import { VideoGenNodeComponent } from "./nodes/VideoGenNode";
import { ShotNodeComponent } from "./nodes/ShotNode";
import { ShotContextNodeComponent } from "./nodes/ShotContextNode";
import { SkillNodeComponent } from "./nodes/SkillNode";
import { PanoramaNodeComponent } from "./nodes/PanoramaNode";
import { PanoViewerNodeComponent } from "./nodes/PanoViewerNode";
import { DirectorWorldNodeComponent } from "./nodes/DirectorWorldNode";
import { DepthCaptureNodeComponent } from "./nodes/DepthCaptureNode";
import { LoraNodeComponent } from "./nodes/LoraNode";
import { AudioNodeComponent } from "./nodes/AudioNode";
import { AudioClipNodeComponent } from "./nodes/AudioClipNode";
import { UpscaleNodeComponent } from "./nodes/UpscaleNode";
import { GroupNodeComponent } from "./nodes/GroupNode";
import { resumeCanvasJobs } from "./engine/resumeJobs";
import { CanvasQuickActionBar } from "./ui/CanvasQuickActionBar";
import { CanvasWorkflowPanel } from "./ui/CanvasWorkflowPanel";
import { CanvasAssetLibrary } from "./ui/CanvasAssetLibrary";
import { CanvasZoomControl } from "./ui/CanvasZoomControl";
import { CutEdge } from "./ui/CutEdge";

/** 所有连线都用它：鼠标移到线上正中出剪刀。放在组件外，避免每次渲染换新对象让 React Flow 重建连线 */
const edgeTypes = { default: CutEdge };
import { cinemaWorkspace } from "../api/cinema";
import type { CanvasNodeType } from "./types";
import { nodeReference } from "./utils/referenceBindings";
import { capturePartners } from "./group";
import { flowInteractionProps, dispatchCanvasShortcut, type CanvasAction } from "./interaction";
import { useCanvasToolStore } from "./store/canvasToolStore";

export interface CanvasViewProps {
  projectId?: string | null;
  onProjectChange?: (projectId: string | null) => void;
}

function CanvasViewInner({ projectId: propProjectId, onProjectChange }: CanvasViewProps = {}) {
  const { screenToFlowPosition, zoomTo, getZoom, fitView } = useReactFlow();
  const tool = useCanvasToolStore((s) => s.tool);
  const setTool = useCanvasToolStore((s) => s.setTool);
  const trackpadPan = useCanvasToolStore((s) => s.trackpadPan);
  const canvasRoot = useRef<HTMLDivElement>(null);
  const reactFlowWrapper = useRef<HTMLDivElement>(null);
  const [showMiniMap, setShowMiniMap] = useState(false);
  /** 资产库那一侧的错误。**要显示出来** —— 吞掉的话用户只会看到「拖了没反应」。 */
  const [libError, setLibError] = useState("");
  const [isAddMenuOpen, setIsAddMenuOpen] = useState(false);
  // **拖线松手没连上节点 → 在松手处弹「新建并连线」菜单**（2026-10-10 用户：「如 libtv……松开鼠标时，弹出菜单」）。
  // `screen` 摆菜单，`flow` 放新节点；从输出口拖出来的往后接（down），从输入口（左侧「+」）拖出来的往前接（up）
  const [dropMenu, setDropMenu] = useState<{ screen: { x: number; y: number }; flow: { x: number; y: number }; direction: SpawnDirection; fromId: string; handle: string | null } | null>(null);
  const spawnLinked = useSpawnLinked();
  useEffect(() => {
    if (!dropMenu) return;
    const close = () => setDropMenu(null);
    const esc = (e: KeyboardEvent) => { if (e.key === "Escape") close(); };
    // 松手那一下的 pointerup 之后才挂，不会被它自己关掉
    const t = window.setTimeout(() => window.addEventListener("pointerdown", close), 0);
    window.addEventListener("keydown", esc);
    return () => { window.clearTimeout(t); window.removeEventListener("pointerdown", close); window.removeEventListener("keydown", esc); };
  }, [dropMenu]);
  const [showShortcuts, setShowShortcuts] = useState(false);
  const [showWorkflows, setShowWorkflows] = useState(false);
  const [workspaceProjects, setWorkspaceProjects] = useState<Array<{ id: string; title: string }>>([]);

  const {
    nodes,
    activeProjectId,
    isSaving,
    saveError,
    lastSavedAt,
    saveCanvas,
    onNodesChange,
    onEdgesChange,
    onConnect,
    addNode,
    clearCanvas,
    loadCanvas,
    setSelectedNodeId,
    undo,
    redo,
    removeSelectedNodes,
    selectAllNodes,
    copySelectedNodes,
    pasteClipboard,
    arrangeNodes,
    groupSelected,
    ungroupSelected,
    fitGroup,
    snapToGrid,
    showEdges,
  } = useCanvasStore(useShallow((state) => ({
    nodes: state.nodes,
    activeProjectId: state.activeProjectId,
    isSaving: state.isSaving,
    saveError: state.saveError,
    lastSavedAt: state.lastSavedAt,
    saveCanvas: state.saveCanvas,
    onNodesChange: state.onNodesChange,
    onEdgesChange: state.onEdgesChange,
    onConnect: state.onConnect,
    addNode: state.addNode,
    clearCanvas: state.clearCanvas,
    loadCanvas: state.loadCanvas,
    setSelectedNodeId: state.setSelectedNodeId,
    undo: state.undo,
    redo: state.redo,
    removeSelectedNodes: state.removeSelectedNodes,
    selectAllNodes: state.selectAllNodes,
    copySelectedNodes: state.copySelectedNodes,
    pasteClipboard: state.pasteClipboard,
    arrangeNodes: state.arrangeNodes,
    groupSelected: state.groupSelected,
    ungroupSelected: state.ungroupSelected,
    fitGroup: state.fitGroup,
    snapToGrid: state.snapToGrid,
    showEdges: state.showEdges,
  })));

  const edges = useCanvasStore(selectDisplayEdges);

  useEffect(() => {
    canvasRoot.current?.focus({ preventScroll: true });
  }, []);

  // 仅处理焦点位于画布内的快捷键。**按键 → 动作的映射只在 `interaction.ts` 里**，这里只管分发 ——
  // 说明面板列的每一条都有测试核对它能被 `resolveCanvasShortcut` 按出来。
  useEffect(() => {
    const run = (action: CanvasAction) => {
      switch (action) {
        case "escape":
          setIsAddMenuOpen(false);
          setShowShortcuts(false);
          setSelectedNodeId(null);
          return;
        case "openAddNode":
          setIsAddMenuOpen((v) => !v);
          setShowShortcuts(false);
          return;
        case "toggleMinimap":
          setShowMiniMap((v) => !v);
          return;
        case "toggleShortcuts":
          setShowShortcuts((v) => !v);
          setIsAddMenuOpen(false);
          return;
        case "selectAll":
          selectAllNodes();
          return;
        case "copy":
          copySelectedNodes();
          return;
        case "paste":
          pasteClipboard();
          return;
        case "undo":
          undo();
          return;
        case "redo":
          redo();
          return;
        case "organize":
          arrangeNodes();
          return;
        case "delete":
          removeSelectedNodes();
          return;
        case "toolMove":
          setTool("move");
          return;
        case "toolHand":
          setTool("hand");
          return;
        case "zoomIn":
          zoomTo(Math.min(getZoom() * 1.2, 8), { duration: 140 });
          return;
        case "zoomOut":
          zoomTo(Math.max(getZoom() / 1.2, 0.1), { duration: 140 });
          return;
        case "fitView":
          void fitView({ padding: 0.2, maxZoom: 1, duration: 200 });
          return;
        case "group":
          groupSelected();
          return;
        case "ungroup":
          ungroupSelected();
          return;
      }
    };
    const handleKeyDown = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement | null;
      const tag = el?.tagName?.toLowerCase();
      const isTyping = tag === "input" || tag === "textarea" || tag === "select" || !!el?.isContentEditable;
      dispatchCanvasShortcut(e, {
        withinCanvas: !!el && !!canvasRoot.current?.contains(el),
        isTyping,
      }, run, () => window.getSelection()?.removeAllRanges());
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [selectAllNodes, removeSelectedNodes, undo, redo, copySelectedNodes, pasteClipboard, arrangeNodes, groupSelected, ungroupSelected, setSelectedNodeId, setTool, zoomTo, getZoom, fitView]);

  /**
   * 拖完组里的成员，把它们所在的组框撑大包住（libtv 式：拖动中框不动，松手再包）。
   * 成员不钳在框里，可以拖到框外 —— 拖出去框就跟着长。
   */
  /**
   * 导演世界节点和它的「导演世界输出」组拖动联动（照 DC）。只管单个节点的拖动 ——
   * 框选一起拖的时候两边本来就都在选区里，再联动就移两次。
   */
  const linkDrag = useRef<{ start: { x: number; y: number }; partners: Record<string, { x: number; y: number }> } | null>(null);
  const beginLinkedDrag = (node: { id: string; position: { x: number; y: number } }, dragged: { id: string }[] | undefined) => {
    linkDrag.current = null;
    if (dragged && dragged.length > 1) return;
    const all = useCanvasStore.getState().nodes;
    const ids = capturePartners(node.id, all);
    if (!ids.length) return;
    const partners: Record<string, { x: number; y: number }> = {};
    for (const id of ids) {
      const p = all.find((n) => n.id === id);
      if (p) partners[id] = { ...p.position };
    }
    linkDrag.current = { start: { ...node.position }, partners };
  };
  const continueLinkedDrag = (node: { position: { x: number; y: number } }) => {
    const l = linkDrag.current;
    if (!l) return;
    const dx = node.position.x - l.start.x;
    const dy = node.position.y - l.start.y;
    const next: Record<string, { x: number; y: number }> = {};
    for (const [id, p] of Object.entries(l.partners)) next[id] = { x: p.x + dx, y: p.y + dy };
    useCanvasStore.getState().moveNodes(next);
  };

  const fitParentsOf = (dragged: { id: string }[]) => {
    const all = useCanvasStore.getState().nodes;
    const parents = new Set<string>();
    for (const d of dragged) {
      const p = all.find((n) => n.id === d.id)?.parentId;
      if (p) parents.add(p);
    }
    parents.forEach((g) => fitGroup(g));
  };


  useEffect(() => {
    let alive = true;
    cinemaWorkspace()
      .then((items) => {
        if (alive && Array.isArray(items)) {
          setWorkspaceProjects(
            items.map((p) => ({ id: p.id, title: p.title || "未命名项目" })),
          );
        }
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  useEffect(() => {
    if (propProjectId !== undefined && propProjectId !== activeProjectId) {
      loadCanvas(propProjectId);
    }
  }, [propProjectId, activeProjectId, loadCanvas]);

  // 页面重新加载或切回画布时，把还挂着后台任务的节点接回来（见 engine/resumeJobs.ts）
  useEffect(() => {
    resumeCanvasJobs(activeProjectId);
  }, [activeProjectId]);

  const nodeTypes: NodeTypes = useMemo(
    () => ({
      prompt: memo(PromptNodeComponent),
      character: memo(CharacterNodeComponent),
      imageGen: memo(ImageGenNodeComponent),
      videoGen: memo(VideoGenNodeComponent),
      shot: memo(ShotNodeComponent),
      shotContext: memo(ShotContextNodeComponent),
      skill: memo(SkillNodeComponent),
      panorama: memo(PanoramaNodeComponent),
      panoViewer: memo(PanoViewerNodeComponent),
      directorWorld: memo(DirectorWorldNodeComponent),
      depthCapture: memo(DepthCaptureNodeComponent),
      lora: memo(LoraNodeComponent),
      audio: memo(AudioNodeComponent),
      audioClip: memo(AudioClipNodeComponent),
      upscale: memo(UpscaleNodeComponent),
      group: memo(GroupNodeComponent),
    }),
    [],
  );

  const handleAddNode = (type: CanvasNodeType, initialData?: Record<string, unknown>) => {
    let flowPos: { x: number; y: number } = { x: 140, y: 160 };
    try {
      const rect = reactFlowWrapper.current?.getBoundingClientRect();
      const centerScreen = rect
        ? { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 }
        : { x: window.innerWidth / 2, y: window.innerHeight / 2 };
      flowPos = screenToFlowPosition(centerScreen);
    } catch {
      // fallback to default if screenToFlowPosition is not available
    }

    const nodeWidth =
      type === "videoGen" || type === "imageGen" || type === "shot" || type === "shotContext" || type === "skill" || type === "panorama" || type === "panoViewer" || type === "directorWorld"
        ? 440
        : 320;
    const nodeHeight = type === "videoGen" || type === "imageGen" ? 380 : 240;
    const offset = (nodes.length % 6) * 28;

    const x = Math.round(flowPos.x - nodeWidth / 2 + offset);
    const y = Math.round(flowPos.y - nodeHeight / 2 + offset);
    addNode(type, { x, y }, initialData);
  };

  return (
    <div
      ref={canvasRoot}
      tabIndex={-1}
      onPointerDownCapture={(event) => {
        const target = event.target as HTMLElement;
        // Clicking blank canvas must release an editor's focus; native controls
        // and contenteditable descendants retain their own keyboard behavior.
        const control = target.closest('input, textarea, select, button, a, [contenteditable]:not([contenteditable="false"]), [tabindex]');
        if (!control || control === canvasRoot.current) {
          canvasRoot.current?.focus({ preventScroll: true });
        }
      }}
      className="relative w-full h-full flex flex-col bg-[#101012] overflow-hidden select-none outline-none"
    >
      {/* Canvas Top Subtle Header */}
      <div
        className="flex items-center justify-between px-4 py-2 z-10 border-b select-none"
        style={{
          background: "rgba(18, 18, 20, 0.7)",
          borderColor: "rgba(255, 255, 255, 0.06)",
          backdropFilter: "blur(16px)",
        }}
      >
        <div className="flex items-center gap-3">
          <span className="text-xs font-semibold text-zinc-200 tracking-tight">无限生产力画布</span>
          <span className="text-[10px] font-mono px-1.5 py-0.5 rounded bg-purple-500/10 text-purple-400 border border-purple-500/20">
            ANIMA CANVAS
          </span>
          <span className="text-[11px] text-zinc-500">
            {nodes.length} 节点 · {edges.length} 连接
          </span>

          <div className="h-3.5 w-px bg-white/10 mx-1" />

          {/* Project Isolation Selector */}
          <div className="flex items-center gap-1.5">
            <span className="text-[11px] text-zinc-400 font-medium">工程:</span>
            <select
              value={activeProjectId ?? ""}
              onChange={(e) => {
                const val = e.target.value === "" ? null : e.target.value;
                loadCanvas(val);
                onProjectChange?.(val);
              }}
              className="text-xs bg-zinc-900/90 text-zinc-200 border border-zinc-700/60 rounded-md px-2 py-1 focus:outline-none focus:border-purple-500 hover:border-zinc-500 transition-colors cursor-pointer"
              title="切换当前画布绑定的制作工程，自动隔离并保存节点图"
            >
              <option value="">自由创作 (全局默认草稿)</option>
              {workspaceProjects.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.title}
                </option>
              ))}
            </select>
          </div>

          {/* Save Status Badge */}
          <span
            className="text-[10px] flex items-center gap-1.5 px-2 py-0.5 rounded-full border border-white/5 bg-zinc-900/60 text-zinc-400"
            title="工程修改将自动实时持久化保存至本地存储"
          >
            {saveError ? (
              <button type="button" title={saveError} onClick={() => saveCanvas()} className="text-red-300">保存失败 · 重试</button>
            ) : isSaving ? (
              <>
                <span className="w-1.5 h-1.5 rounded-full bg-amber-400 animate-pulse" />
                <span>保存中...</span>
              </>
            ) : (
              <>
                <span className="w-1.5 h-1.5 rounded-full bg-emerald-400" />
                <span>{lastSavedAt ? "已自动保存" : "尚无新保存"}</span>
              </>
            )}
          </span>
        </div>

        <div className="text-[11px] text-zinc-500 flex items-center gap-3">
          <span>拖动节点胶囊连线 · 点击卡片唤出参数面板</span>
        </div>
      </div>

      {/* Main Flow Canvas Viewport */}
      <div ref={reactFlowWrapper} className="flex-1 w-full h-full relative">
        <CanvasAssetLibrary projectId={activeProjectId} onError={setLibError} onAddNode={handleAddNode} />
        {libError && (
          <p
            role="alert"
            className="absolute top-3 left-[256px] z-10 max-w-[320px] rounded-lg border border-red-500/30 bg-red-950/80 px-3 py-2 text-[11px] text-red-200 backdrop-blur"
            onClick={() => setLibError("")}
          >
            {libError}
          </p>
        )}
        <ReactFlow
          nodes={nodes}
          edges={edges}
          onNodesChange={onNodesChange}
          onEdgesChange={onEdgesChange}
          onConnect={onConnect}
          onConnectEnd={(event, state) => {
            // 连上了（或者没在连）就不管；松在空白处才弹
            if (state.isValid || !state.fromNode || !state.fromHandle) return;
            const p = "changedTouches" in event ? event.changedTouches[0] : event;
            if (!p) return;
            const screen = { x: p.clientX, y: p.clientY };
            setDropMenu({
              screen,
              flow: screenToFlowPosition(screen),
              direction: state.fromHandle.type === "target" ? "up" : "down",
              fromId: state.fromNode.id,
              handle: state.fromHandle.id ?? null,
            });
          }}
          nodeTypes={nodeTypes}
          edgeTypes={edgeTypes}
          onNodeClick={(_, node) => {
            const state = useCanvasStore.getState();
            const target = state.nodes.find((n) => n.id === state.referenceTargetId);
            if (!target) { setSelectedNodeId(node.id); return; }
            try {
              if (target.id === node.id) throw new Error("不能引用自身");
              if (target.data.isGenerating || target.data.referenceLoading) throw new Error("目标节点忙碌");
              state.bindReference(state.activeProjectId, target.id, nodeReference(node));
              state.updateNodeData(target.id, { referenceError: "" });
              useCanvasStore.setState({ referenceTargetId: null });
              setSelectedNodeId(target.id);
            } catch (e) { state.updateNodeData(target.id, { referenceError: String(e) }); }
          }}
          onNodeDragStart={(_, node, dragged) => beginLinkedDrag(node, dragged)}
          onNodeDrag={(_, node) => continueLinkedDrag(node)}
          onNodeDragStop={(_, node, dragged) => {
            continueLinkedDrag(node);
            linkDrag.current = null;
            fitParentsOf(dragged?.length ? dragged : [node]);
          }}
          onSelectionDragStop={(_, dragged) => fitParentsOf(dragged)}
          onPaneClick={() => {
            setSelectedNodeId(null);
            setIsAddMenuOpen(false);
          }}
          snapToGrid={snapToGrid}
          snapGrid={[20, 20]}
          className={[!showEdges ? "hide-canvas-edges" : "", tool === "hand" ? "canvas-tool-hand" : ""].join(" ")}
          connectionMode={ConnectionMode.Loose}
          connectionRadius={80}
          defaultViewport={{ x: 0, y: 0, zoom: 1 }}
          minZoom={0.1}
          maxZoom={8}
          colorMode="dark"
          connectionLineStyle={{ stroke: "rgba(205, 209, 216, 0.65)", strokeWidth: 2 }}
          defaultEdgeOptions={{
            animated: true,
            style: { stroke: "rgba(176, 176, 183, 0.45)", strokeWidth: 2 },
          }}
          {...flowInteractionProps(tool, trackpadPan)}
        >
          <Background variant={BackgroundVariant.Dots} gap={20} size={1} color="#333338" />
          {showMiniMap && (
            <MiniMap
              zoomable
              pannable
              nodeColor="#3b82f6"
              maskColor="rgba(0, 0, 0, 0.7)"
              className="!bg-zinc-950/90 !border !border-white/10 !rounded-xl overflow-hidden shadow-2xl !bottom-20 !right-6"
            />
          )}

          {/* 右下角屏幕缩放百分比与控制栏 (对齐需求与 DramaClaw) */}
          <CanvasZoomControl
            showMiniMap={showMiniMap}
            onToggleMiniMap={() => setShowMiniMap(!showMiniMap)}
          />
        </ReactFlow>

        {/* 底部常驻悬浮 Dock 栏与添加菜单 (对齐截图 1) */}
        <CanvasQuickActionBar
          onAddNode={handleAddNode}
          onClear={clearCanvas}
          isAddMenuOpen={isAddMenuOpen}
          onToggleAddMenu={() => setIsAddMenuOpen((v) => !v)}
          showShortcuts={showShortcuts}
          onToggleShortcuts={() => setShowShortcuts((v) => !v)}
          showWorkflows={showWorkflows}
          onToggleWorkflows={() => setShowWorkflows((v) => !v)}
        />

        {dropMenu && (
          <SpawnMenu
            title={dropMenu.direction === "up" ? "生成后接入该节点" : "引用该节点生成"}
            choices={spawnChoices(dropMenu.direction, spawnContextOf(nodes.find((n) => n.id === dropMenu.fromId)))}
            className="fixed"
            style={{ left: dropMenu.screen.x + 8, top: dropMenu.screen.y - 16 }}
            onPick={(c) => {
              const m = dropMenu;
              setDropMenu(null);
              spawnLinked(m.direction, m.fromId, m.handle, c, m.flow);
            }}
          />
        )}

        <CanvasWorkflowPanel open={showWorkflows} onClose={() => setShowWorkflows(false)} projects={workspaceProjects} />

      </div>
    </div>
  );
}

export function CanvasView(props: CanvasViewProps = {}) {
  return (
    <ReactFlowProvider>
      <CanvasViewInner {...props} />
    </ReactFlowProvider>
  );
}

export default CanvasView;
