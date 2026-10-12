import { describe, it, expect, beforeEach } from "vitest";
import {
  useCanvasStore,
  getCanvasStorageKey,
  loadPersistedCanvas,
  CANVAS_DATA_VERSION,
  CANVAS_STORAGE_KEY_PREFIX,
} from "./useCanvasStore";

// Mock localStorage for testing persistence & project isolation
const mockStorageMap = new Map<string, string>();
const testStorage: Storage = {
  getItem: (key: string) => mockStorageMap.get(key) ?? null,
  setItem: (key: string, value: string) => {
    mockStorageMap.set(key, value);
  },
  removeItem: (key: string) => {
    mockStorageMap.delete(key);
  },
  clear: () => {
    mockStorageMap.clear();
  },
  key: (index: number) => Array.from(mockStorageMap.keys())[index] ?? null,
  get length() {
    return mockStorageMap.size;
  },
};

(globalThis as unknown as { localStorage: Storage }).localStorage = testStorage;
if (typeof window !== "undefined") {
  (window as unknown as { localStorage: Storage }).localStorage = testStorage;
}

describe("画布状态管理 useCanvasStore", () => {
  beforeEach(() => {
    mockStorageMap.clear();
    useCanvasStore.getState().clearCanvas();
  });

  it("能添加不同类型的节点且带合理默认值", () => {
    const store = useCanvasStore.getState();

    const pId = store.addNode("prompt", { x: 10, y: 20 });
    const cId = store.addNode("character", { x: 30, y: 40 });
    const iId = store.addNode("imageGen", { x: 50, y: 60 });
    const vId = store.addNode("videoGen", { x: 70, y: 80 });

    const nodes = useCanvasStore.getState().nodes;
    expect(nodes.length).toBe(4);

    const pNode = nodes.find((n) => n.id === pId);
    expect(pNode?.type).toBe("prompt");
    expect(pNode?.data.scale).toBe("medium_shot");

    const cNode = nodes.find((n) => n.id === cId);
    expect(cNode?.type).toBe("character");
    expect(cNode?.data.name).toBe("新角色");

    const iNode = nodes.find((n) => n.id === iId);
    expect(iNode?.type).toBe("imageGen");
    expect(iNode?.data.engine).toBe("zimage");

    const vNode = nodes.find((n) => n.id === vId);
    expect(vNode?.type).toBe("videoGen");
    expect(vNode?.data.engine).toBe("ltx");
    expect(vNode?.data.width).toBe(768);
    expect(vNode?.data.height).toBe(512);
  });

  /**
   * 多选（⌘+点击 / 框选）之后的删除、复制、粘贴要对**所有**选中节点生效。
   * 原来 store 只认 `selectedNodeId` 一个，⌘C 只复制到其中一个、
   * Delete 只删一个、节点之间的连线粘贴后全没。
   */
  describe("多选", () => {
    const setup = () => {
      const s = useCanvasStore.getState();
      const a = s.addNode("prompt", { x: 0, y: 0 });
      const b = s.addNode("imageGen", { x: 300, y: 0 });
      const c = s.addNode("prompt", { x: 0, y: 300 });
      s.onConnect({ source: a, target: b, sourceHandle: null, targetHandle: null });
      useCanvasStore.setState((st) => ({
        nodes: st.nodes.map((n) => ({ ...n, selected: n.id === a || n.id === b })),
        selectedNodeId: null,
      }));
      return { a, b, c };
    };

    it("selectedIds 以 node.selected 为准", () => {
      const { a, b } = setup();
      expect(useCanvasStore.getState().selectedIds().sort()).toEqual([a, b].sort());
    });

    /**
     * addNode 会把新节点设成 selectedNodeId，框选不更新它。
     * 原来取并集，「加了个 C，再框选 A、B」之后删除 / 成组都会把 C 带上。
     */
    it("框选之后不把刚加的节点带进来", () => {
      const { a, b, c } = setup();
      useCanvasStore.setState({ selectedNodeId: c });
      expect(useCanvasStore.getState().selectedIds().sort()).toEqual([a, b].sort());
    });

    it("删除所有选中节点，一次撤销全部回来", () => {
      const { c } = setup();
      const before = useCanvasStore.getState().nodes.length;
      expect(useCanvasStore.getState().removeSelectedNodes()).toBe(2);
      expect(useCanvasStore.getState().nodes.map((n) => n.id)).toEqual([c]);
      expect(useCanvasStore.getState().edges).toHaveLength(0);
      useCanvasStore.getState().undo();
      expect(useCanvasStore.getState().nodes).toHaveLength(before);
    });

    it("复制粘贴多个节点，连同它们之间的连线，id 全换新", () => {
      const { a, b } = setup();
      expect(useCanvasStore.getState().copySelectedNodes()).toBe(2);
      const ids = useCanvasStore.getState().pasteClipboard();
      const st = useCanvasStore.getState();
      expect(ids).toHaveLength(2);
      expect(ids).not.toContain(a);
      expect(ids).not.toContain(b);
      const newEdges = st.edges.filter((e) => ids.includes(e.source) && ids.includes(e.target));
      expect(newEdges, "节点之间的连线要跟着粘过来").toHaveLength(1);
      // 粘出来的被选中，原来的取消选中
      expect(st.nodes.filter((n) => n.selected).map((n) => n.id).sort()).toEqual([...ids].sort());
    });

    it("连续粘贴不叠在同一个位置", () => {
      setup();
      useCanvasStore.getState().copySelectedNodes();
      const first = useCanvasStore.getState().pasteClipboard();
      const second = useCanvasStore.getState().pasteClipboard();
      const pos = (id: string) => useCanvasStore.getState().nodes.find((n) => n.id === id)!.position;
      expect(pos(second[0])).not.toEqual(pos(first[0]));
    });
  });

  /** 成组之后删除 / 复制 / 整理画布都要把组和成员当一个整体。 */
  describe("组", () => {
    const setupGroup = () => {
      const s = useCanvasStore.getState();
      const a = s.addNode("prompt", { x: 100, y: 100 });
      const b = s.addNode("prompt", { x: 500, y: 100 });
      const c = s.addNode("prompt", { x: 100, y: 600 });
      useCanvasStore.setState((st) => ({ nodes: st.nodes.map((n) => ({ ...n, selected: n.id === a || n.id === b })) }));
      const g = useCanvasStore.getState().groupSelected()!;
      return { a, b, c, g };
    };

    it("⌘G 成组，一次撤销回到成组前", () => {
      const { a, g } = setupGroup();
      expect(g).toBeTruthy();
      expect(useCanvasStore.getState().nodes.find((n) => n.id === a)!.parentId).toBe(g);
      useCanvasStore.getState().undo();
      expect(useCanvasStore.getState().nodes.find((n) => n.id === g)).toBeUndefined();
      expect(useCanvasStore.getState().nodes.find((n) => n.id === a)!.parentId).toBeUndefined();
    });

    /** 12 张截图一次落上画布，撤销也得一次撤干净 —— 不然要按 13 下。 */
    it("截图组一步落上画布、一步撤销", () => {
      const s = useCanvasStore.getState();
      const pano = s.addNode("panoViewer", { x: 0, y: 0 });
      const before = useCanvasStore.getState().nodes.length;
      const images = Array.from({ length: 12 }, (_, i) => ({ path: `/tmp/${i}.png`, label: `图${i}` }));
      const g = useCanvasStore.getState().addImageGroup(pano, images, { cols: 4, label: "全景截图 · 12 视角" })!;
      const st = useCanvasStore.getState();
      expect(st.nodes.find((n) => n.id === g)!.type).toBe("group");
      expect(st.nodes.filter((n) => n.parentId === g)).toHaveLength(12);
      expect(new Set(st.nodes.map((n) => n.id)).size).toBe(st.nodes.length);
      expect(st.selectedIds()).toEqual([g]);
      useCanvasStore.getState().undo();
      expect(useCanvasStore.getState().nodes.length).toBe(before);
    });

    /** 只删框的话，成员 parentId 悬空、坐标还是相对的，会跳到原点附近。 */
    it("删组连同成员一起删", () => {
      const { c, g } = setupGroup();
      useCanvasStore.setState((st) => ({ nodes: st.nodes.map((n) => ({ ...n, selected: n.id === g })) }));
      useCanvasStore.getState().removeSelectedNodes();
      expect(useCanvasStore.getState().nodes.map((n) => n.id)).toEqual([c]);
    });

    it("复制组带上成员，粘贴出来的成员挂在新组下", () => {
      const { g } = setupGroup();
      useCanvasStore.setState((st) => ({ nodes: st.nodes.map((n) => ({ ...n, selected: n.id === g })), selectedNodeId: g }));
      useCanvasStore.getState().copySelectedNodes();
      const ids = useCanvasStore.getState().pasteClipboard();
      const st = useCanvasStore.getState();
      const newGroup = st.nodes.find((n) => ids.includes(n.id) && n.type === "group")!;
      const newMembers = st.nodes.filter((n) => n.parentId === newGroup.id);
      expect(newMembers).toHaveLength(2);
      // 新组排在它的成员前面
      const order = st.nodes.map((n) => n.id);
      for (const m of newMembers) expect(order.indexOf(newGroup.id)).toBeLessThan(order.indexOf(m.id));
    });

    /** 单独复制组里的一个成员：粘出来是顶层节点，坐标要是绝对坐标。 */
    it("单独复制成员，粘出来是顶层节点", () => {
      const { a } = setupGroup();
      const absBefore = (() => {
        const st = useCanvasStore.getState();
        const n = st.nodes.find((x) => x.id === a)!;
        const p = st.nodes.find((x) => x.id === n.parentId)!;
        return { x: n.position.x + p.position.x, y: n.position.y + p.position.y };
      })();
      useCanvasStore.setState((st) => ({ nodes: st.nodes.map((n) => ({ ...n, selected: n.id === a })), selectedNodeId: a }));
      useCanvasStore.getState().copySelectedNodes();
      const [pid] = useCanvasStore.getState().pasteClipboard();
      const pasted = useCanvasStore.getState().nodes.find((n) => n.id === pid)!;
      expect(pasted.parentId).toBeUndefined();
      expect(pasted.position).toEqual({ x: absBefore.x + 40, y: absBefore.y + 40 });
    });

    it("整理画布不动组里成员的相对坐标", () => {
      const { a } = setupGroup();
      const rel = useCanvasStore.getState().nodes.find((n) => n.id === a)!.position;
      useCanvasStore.getState().arrangeNodes();
      expect(useCanvasStore.getState().nodes.find((n) => n.id === a)!.position).toEqual(rel);
    });

    /**
     * 组内连线折算到组之后很容易出环。原来的拓扑排序遇环无限入队，界面卡死。
     * 这条测试跑完就说明没卡住。
     */
    it("连线成环时整理画布不会卡死", () => {
      const { a, b, c } = setupGroup();
      const s = useCanvasStore.getState();
      s.onConnect({ source: a, target: c, sourceHandle: null, targetHandle: null });
      s.onConnect({ source: c, target: b, sourceHandle: null, targetHandle: null });
      useCanvasStore.setState((st) => ({ edges: [...st.edges, { id: "cyc", source: c, target: a }] }));
      expect(() => useCanvasStore.getState().arrangeNodes()).not.toThrow();
    });
  });

  it("更新节点数据 updateNodeData", () => {
    const store = useCanvasStore.getState();
    const id = store.addNode("prompt");

    store.updateNodeData(id, { prompt: "更新后的提示词", scale: "extreme_wide" });

    const updated = useCanvasStore.getState().nodes.find((n) => n.id === id);
    expect(updated?.data.prompt).toBe("更新后的提示词");
    expect(updated?.data.scale).toBe("extreme_wide");
  });

  it("删除节点时自动级联清理连接的边", () => {
    const store = useCanvasStore.getState();
    const n1 = store.addNode("prompt");
    const n2 = store.addNode("imageGen");

    store.setEdges([
      { id: "e1", source: n1, target: n2 },
      { id: "e2", source: "other", target: "another" },
    ]);

    expect(useCanvasStore.getState().edges.length).toBe(2);

    store.removeNode(n1);

    expect(useCanvasStore.getState().nodes.some((n) => n.id === n1)).toBe(false);
    expect(useCanvasStore.getState().edges.some((e) => e.source === n1 || e.target === n1)).toBe(
      false,
    );
    // 未受影响的边保留
    expect(useCanvasStore.getState().edges.length).toBe(1);
  });

  describe("画布持久化与项目隔离机制", () => {
    it("正确生成项目存储键 getCanvasStorageKey", () => {
      expect(getCanvasStorageKey(null)).toBe(`${CANVAS_STORAGE_KEY_PREFIX}default`);
      expect(getCanvasStorageKey("")).toBe(`${CANVAS_STORAGE_KEY_PREFIX}default`);
      expect(getCanvasStorageKey("   ")).toBe(`${CANVAS_STORAGE_KEY_PREFIX}default`);
      expect(getCanvasStorageKey("proj_123")).toBe(`${CANVAS_STORAGE_KEY_PREFIX}proj_123`);
    });

    it("容错恢复 loadPersistedCanvas：异常或损坏的 JSON 能安全降级为空画布", () => {
      expect(loadPersistedCanvas("non_existent")).toEqual({ nodes: [], edges: [] });

      testStorage.setItem(getCanvasStorageKey("corrupt_json"), "{ invalid json syntax");
      expect(loadPersistedCanvas("corrupt_json")).toEqual({ nodes: [], edges: [] });

      testStorage.setItem(getCanvasStorageKey("missing_arrays"), JSON.stringify({ nodes: null }));
      expect(loadPersistedCanvas("missing_arrays")).toEqual({ nodes: [], edges: [] });
    });

    it("支持手动 saveCanvas 并可被 loadPersistedCanvas 重新读出", () => {
      const store = useCanvasStore.getState();
      store.loadCanvas("proj_movie_a");

      const n1 = store.addNode("prompt", { x: 100, y: 100 });
      store.saveCanvas();

      const key = getCanvasStorageKey("proj_movie_a");
      const savedRaw = testStorage.getItem(key);
      expect(savedRaw).toBeTruthy();

      const parsed = JSON.parse(savedRaw!);
      expect(parsed.projectId).toBe("proj_movie_a");
      expect(parsed.nodes.length).toBe(1);
      expect(parsed.nodes[0].id).toBe(n1);
    });

    it("完整项目隔离：切换项目时互相独立且自动保存恢复", () => {
      const store = useCanvasStore.getState();

      // 1. 进入项目 Alpha
      store.loadCanvas("project_alpha");
      expect(useCanvasStore.getState().activeProjectId).toBe("project_alpha");
      expect(useCanvasStore.getState().nodes.length).toBe(0);

      // 2. 在项目 Alpha 中添加 2 个节点并连线
      const aNode1 = store.addNode("prompt", { x: 10, y: 10 });
      const aNode2 = store.addNode("imageGen", { x: 20, y: 20 });
      store.setEdges([{ id: "edge_a", source: aNode1, target: aNode2 }]);
      store.saveCanvas();

      expect(useCanvasStore.getState().nodes.length).toBe(2);
      expect(useCanvasStore.getState().edges.length).toBe(1);

      // 3. 切换到项目 Beta
      store.loadCanvas("project_beta");
      expect(useCanvasStore.getState().activeProjectId).toBe("project_beta");
      // 项目 Beta 刚进入时是空的
      expect(useCanvasStore.getState().nodes.length).toBe(0);
      expect(useCanvasStore.getState().edges.length).toBe(0);

      // 4. 在项目 Beta 中添加 1 个视频节点
      const bNode1 = store.addNode("videoGen", { x: 300, y: 300 });
      store.saveCanvas();

      expect(useCanvasStore.getState().nodes.length).toBe(1);
      expect(useCanvasStore.getState().nodes[0].id).toBe(bNode1);

      // 5. 切回项目 Alpha：Alpha 的 2 个节点和边完好无损恢复！
      store.loadCanvas("project_alpha");
      expect(useCanvasStore.getState().activeProjectId).toBe("project_alpha");
      expect(useCanvasStore.getState().nodes.length).toBe(2);
      expect(useCanvasStore.getState().nodes.map((n) => n.id)).toEqual([aNode1, aNode2]);
      expect(useCanvasStore.getState().edges.length).toBe(1);
      expect(useCanvasStore.getState().edges[0].id).toBe("edge_a");

      // 6. 再次切回项目 Beta：Beta 的视频节点同样完好无损！
      store.loadCanvas("project_beta");
      expect(useCanvasStore.getState().nodes.length).toBe(1);
      expect(useCanvasStore.getState().nodes[0].id).toBe(bNode1);
    });

    it("clearCanvas 只清空当前活跃项目的画布与存储，不影响其他项目", () => {
      const store = useCanvasStore.getState();

      // 项目 A 保存 1 个节点
      store.loadCanvas("proj_preserve");
      store.addNode("character");
      store.saveCanvas();

      // 项目 B 保存 1 个节点
      store.loadCanvas("proj_to_clear");
      store.addNode("prompt");
      store.saveCanvas();

      // 清空项目 B
      store.clearCanvas();
      expect(useCanvasStore.getState().nodes.length).toBe(0);

      // 检查存储：项目 B 为空
      const bData = loadPersistedCanvas("proj_to_clear");
      expect(bData.nodes.length).toBe(0);

      // 检查项目 A：不受任何影响
      store.loadCanvas("proj_preserve");
      expect(useCanvasStore.getState().nodes.length).toBe(1);
      expect(useCanvasStore.getState().nodes[0].type).toBe("character");
    });

    it("支持网格吸附与连线显隐状态切换", () => {
      const store = useCanvasStore.getState();
      expect(store.snapToGrid).toBe(true);
      expect(store.showEdges).toBe(true);

      store.toggleSnapToGrid();
      expect(useCanvasStore.getState().snapToGrid).toBe(false);

      store.toggleShowEdges();
      expect(useCanvasStore.getState().showEdges).toBe(false);

      store.setSnapToGrid(true);
      store.setShowEdges(true);
      expect(useCanvasStore.getState().snapToGrid).toBe(true);
      expect(useCanvasStore.getState().showEdges).toBe(true);
    });

    it("arrangeNodes 能将复杂 DAG 节点按拓扑分层平整排列在 20px 网格上", () => {
      const store = useCanvasStore.getState();
      const pId = store.addNode("prompt", { x: 999, y: 888 });
      const iId = store.addNode("imageGen", { x: 123, y: 456 });

      store.onConnect({
        source: pId,
        target: iId,
        sourceHandle: "prompt-out",
        targetHandle: "prompt-in",
      });

      store.arrangeNodes();

      const nodes = useCanvasStore.getState().nodes;
      const pNode = nodes.find((n) => n.id === pId)!;
      const iNode = nodes.find((n) => n.id === iId)!;

      // 拓扑排序中 pNode 是上游 (Rank 0)，iNode 是下游 (Rank 1)
      expect(pNode.position.x).toBeLessThan(iNode.position.x);
      // 检查网格对齐 (必须是 20 的整数倍)
      expect(pNode.position.x % 20).toBe(0);
      expect(pNode.position.y % 20).toBe(0);
      expect(iNode.position.x % 20).toBe(0);
      expect(iNode.position.y % 20).toBe(0);
    });
  });
});

describe("老画布的字段补全", () => {
  it("存量视频节点缺 generateAudio 时补 true，不是 false", () => {
    // 加这个字段之前，后端缺省是 true（有音轨）。补 false 会让用户一个开关
    // 都没动过就静音 —— H3 的同步中文台词就在那条音轨里。
    const key = getCanvasStorageKey("p_legacy");
    localStorage.setItem(key, JSON.stringify({
      version: 1, projectId: "p_legacy", updatedAt: 0, edges: [],
      nodes: [{ id: "v1", type: "videoGen", position: { x: 0, y: 0 }, data: { mode: "i2v" } }],
    }));
    const { nodes } = loadPersistedCanvas("p_legacy");
    expect(nodes[0].data.generateAudio).toBe(true);
  });

  it("用户显式关掉的 false 不会被补回 true", () => {
    const key = getCanvasStorageKey("p_off");
    localStorage.setItem(key, JSON.stringify({
      version: 1, projectId: "p_off", updatedAt: 0, edges: [],
      nodes: [{ id: "v1", type: "videoGen", position: { x: 0, y: 0 },
        data: { mode: "i2v", generateAudio: false } }],
    }));
    const { nodes } = loadPersistedCanvas("p_off");
    expect(nodes[0].data.generateAudio).toBe(false);
  });
});


// 用户 2026-09-25：模板里的「设定卡 → 参考生视频」一刷新就变回图生视频 —— r2v 迁移以前每次载入都跑
describe("r2v 迁移只对老存档做一次", () => {
  const save = (key: string, version: number) => localStorage.setItem(getCanvasStorageKey(key), JSON.stringify({
    version, projectId: key, updatedAt: 0, edges: [],
    nodes: [{ id: "v1", type: "videoGen", position: { x: 0, y: 0 }, data: { mode: "r2v" } }],
  }));
  it("2 版以前的存档：只绑了 0~1 张参考的 r2v 回到 auto（那是老 bug 写进去的）", () => {
    save("p_v1", 1);
    expect(loadPersistedCanvas("p_v1").nodes[0].data.mode).toBe("auto");
  });
  it("新存档：r2v 原样保留（是用户或模板选的）", () => {
    save("p_v2", CANVAS_DATA_VERSION);
    expect(loadPersistedCanvas("p_v2").nodes[0].data.mode).toBe("r2v");
  });
  it("存下来的就是新版本号", () => {
    expect(CANVAS_DATA_VERSION).toBeGreaterThanOrEqual(2);
  });
});

describe("全选画布节点", () => {
  beforeEach(() => useCanvasStore.getState().clearCanvas());

  it("全选后复制粘贴保留所有节点和内部连线", () => {
    const store = useCanvasStore.getState();
    const a = store.addNode("prompt", { x: 0, y: 0 });
    const b = store.addNode("imageGen", { x: 500, y: 0 });
    store.onConnect({ source: a, target: b, sourceHandle: null, targetHandle: null });
    store.selectAllNodes();
    expect(store.selectedIds().sort()).toEqual([a, b].sort());
    expect(store.copySelectedNodes()).toBe(2);
    const pasted = store.pasteClipboard();
    expect(pasted).toHaveLength(2);
    const state = useCanvasStore.getState();
    expect(state.nodes).toHaveLength(4);
    expect(state.edges.filter((edge) => pasted.includes(edge.source) && pasted.includes(edge.target))).toHaveLength(1);
  });

  it("清除旧的单选及参考目标，跳过隐藏和不可选节点", () => {
    const store = useCanvasStore.getState();
    const a = store.addNode("prompt"), b = store.addNode("prompt"), c = store.addNode("prompt");
    useCanvasStore.setState((state) => ({
      nodes: state.nodes.map((node) => node.id === b ? { ...node, hidden: true, selected: true }
        : node.id === c ? { ...node, selectable: false } : node),
      selectedNodeId: c, referenceTargetId: c,
    }));
    store.selectAllNodes();
    expect(store.selectedIds()).toEqual([a]);
    expect(useCanvasStore.getState().selectedNodeId).toBeNull();
    expect(useCanvasStore.getState().referenceTargetId).toBeNull();
    const selected = useCanvasStore.getState().nodes;
    store.selectAllNodes();
    useCanvasStore.getState().nodes.forEach((node, i) => expect(node).toBe(selected[i]));
  });

  it("空画布全选安全，删除全选节点可以一次撤销", () => {
    const store = useCanvasStore.getState();
    store.selectAllNodes();
    expect(store.selectedIds()).toEqual([]);
    store.addNode("prompt"); store.addNode("imageGen");
    store.selectAllNodes();
    expect(store.removeSelectedNodes()).toBe(2);
    store.undo();
    expect(useCanvasStore.getState().nodes).toHaveLength(2);
  });
});
