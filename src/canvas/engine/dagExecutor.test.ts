import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  resolveUpstreamPipeline,
  aggregateNodeInputs,
  executeNodePipeline,
  executeSingleNode,
  videoInputImages,
  computeImageNodeSignature,
  withSharedStyle,
} from "./dagExecutor";
import { useCanvasStore } from "../store/useCanvasStore";
import type { CanvasNode, CanvasEdge } from "../types";
import { canvasCatalogFixture } from "../utils/modelCatalog.fixture";

vi.mock("./canvasTasks", () => ({
  // 画布生成改走后台任务（canvasTasks.ts）；测试里直接替身，inNode 原样执行
  inNode: (_ctx: unknown, fn: () => unknown) => fn(),
  liveJobs: new Set<string>(),
  studioGenerate: vi.fn(),
  canvasLongTake: vi.fn(),
  canvasDepthCapture: vi.fn(),
  canvasDepthVideo: vi.fn(),
  canvasImageOp: vi.fn(),
  canvasVideoInterpolate: vi.fn(),
  canvasVideoUpscale: vi.fn(),
}));
vi.mock("../../api/studio", () => ({
  studioGenerate: vi.fn(),
  studioCapabilities: vi.fn(),
}));

import { studioCapabilities } from "../../api/studio";
import { studioGenerate, canvasLongTake } from "./canvasTasks";

describe("DAG 执行器 (dagExecutor)", () => {
  it("resolves a canvas reference's latest output and rejects deleted sources", () => {
    const store = useCanvasStore.getState();
    store.clearCanvas();
    const source = store.addNode("imageGen", undefined, { outputImageUrl: "/new.png" });
    const target = store.addNode("videoGen", undefined, { mode: "r2v", references: [{ sourceNodeId: source, path: "/old.png", name: "Hero" }] });
    expect(videoInputImages(target, useCanvasStore.getState().nodes, [])).toEqual(["/new.png"]);
    store.removeNode(source);
    expect(() => videoInputImages(target, useCanvasStore.getState().nodes, [])).toThrow("已删除");
  });
  beforeEach(() => {
    useCanvasStore.getState().clearCanvas();
    vi.clearAllMocks();
    vi.mocked(studioCapabilities).mockResolvedValue(canvasCatalogFixture);
  });

  describe("resolveUpstreamPipeline 拓扑依赖解析", () => {
    it("正确解析单向上游拓扑并保持拓扑顺序", () => {
      const nodes: CanvasNode[] = [
        { id: "node-p", type: "prompt", position: { x: 0, y: 0 }, data: { label: "提示词", prompt: "test" } },
        { id: "node-img", type: "imageGen", position: { x: 100, y: 0 }, data: { engine: "zimage", width: 1024, height: 1024, seed: 1 } },
        { id: "node-vid", type: "videoGen", position: { x: 200, y: 0 }, data: { engine: "h3", durationSecs: 2 } },
      ];

      const edges: CanvasEdge[] = [
        { id: "e1", source: "node-p", target: "node-img" },
        { id: "e2", source: "node-img", target: "node-vid" },
      ];

      const order = resolveUpstreamPipeline("node-vid", nodes, edges);
      const ids = order.map((n) => n.id);
      expect(ids).toEqual(["node-p", "node-img", "node-vid"]);
    });

    /**
     * 全景截图组 ← 查看器 ← 全景图 ← 全景出图。下游出图时**不能追过查看器** ——
     * 否则全景出图参数改过就会被重跑一遍，而截图不会因此更新。
     */
    it("追到全景查看器 / 导演世界就停，不再往上", () => {
      for (const viewer of ["panoViewer", "directorWorld"] as const) {
        const nodes = [
          { id: "gen", type: "imageGen", position: { x: 0, y: 0 }, data: { engine: "zimage", width: 2048, height: 1024, seed: 1 } },
          { id: "view", type: viewer, position: { x: 0, y: 0 }, data: {} },
          { id: "shot", type: "imageGen", position: { x: 0, y: 0 }, data: { referenceOnly: true, outputImageUrl: "/a.png" } },
          { id: "out", type: "imageGen", position: { x: 0, y: 0 }, data: { engine: "zimage", width: 1024, height: 576, seed: 1 } },
        ] as unknown as CanvasNode[];
        const edges: CanvasEdge[] = [
          { id: "e1", source: "gen", target: "view" },
          { id: "e2", source: "view", target: "shot" },
          { id: "e3", source: "shot", target: "out" },
        ];
        expect(resolveUpstreamPipeline("out", nodes, edges).map((n) => n.id)).toEqual(["view", "shot", "out"]);
        // 以查看器自己为目标时照常追上游
        expect(resolveUpstreamPipeline("view", nodes, edges).map((n) => n.id)).toEqual(["gen", "view"]);
        // 下游拿到的参考图是截图，不是全景原图
        expect(aggregateNodeInputs("out", nodes, edges).inputImageUrl).toBe("/a.png");
      }
    });

    it("检测到环路依赖时准确抛出错误", () => {
      const nodes: CanvasNode[] = [
        { id: "A", type: "imageGen", position: { x: 0, y: 0 }, data: { engine: "zimage", width: 1024, height: 1024, seed: 1 } },
        { id: "B", type: "imageGen", position: { x: 100, y: 0 }, data: { engine: "zimage", width: 1024, height: 1024, seed: 1 } },
      ];

      const edges: CanvasEdge[] = [
        { id: "e1", source: "A", target: "B" },
        { id: "e2", source: "B", target: "A" },
      ];

      expect(() => resolveUpstreamPipeline("B", nodes, edges)).toThrow("检测到循环依赖环路");
    });
  });

  describe("镜头上下文节点与技能节点", () => {
    const target = (engine: string): CanvasNode => ({
      id: "target",
      type: "imageGen",
      position: { x: 300, y: 0 },
      data: { engine, width: 1024, height: 1024, seed: 1 },
    });
    const wire = (from: string): CanvasEdge[] => [
      { id: `e-${from}`, source: from, target: "target", targetHandle: "prompt-in" },
    ];

    it("上下文节点把地理锁和站位发给下游", () => {
      const nodes: CanvasNode[] = [
        {
          id: "ctx",
          type: "shotContext",
          position: { x: 0, y: 0 },
          data: {
            sceneName: "安和面馆",
            viewName: "入口正打",
            viewMap: "从东侧卷帘门内侧朝店堂深处看，视线尽头是靠北墙的旧木柜台",
            lock: "地理锁：北侧旧木柜台在北墙（大件，不要缩小）。",
            blocking: "沈星晴在画面左侧，背对门",
            state: "傍晚，柜台的暖灯亮着",
          },
        },
        target("zimage"),
      ];
      const out = aggregateNodeInputs("target", nodes, wire("ctx")).combinedPrompt;
      expect(out).toContain("空间：从东侧卷帘门内侧");
      expect(out).toContain("地理锁：北侧旧木柜台");
      expect(out).toContain("站位：沈星晴在画面左侧");
      expect(out).toContain("时段：傍晚");
    });

    /**
     * **空节点一个字都不发。**
     *
     * 发一句「空间：」出去比不发更糟 —— 模型会去理解那个空标题，
     * 而用户看不出多了这么一段。
     */
    it("空的上下文节点什么都不发", () => {
      const nodes: CanvasNode[] = [
        {
          id: "ctx",
          type: "shotContext",
          position: { x: 0, y: 0 },
          data: { sceneName: "", viewMap: "", lock: "", blocking: "  ", state: "" },
        },
        target("zimage"),
      ];
      expect(aggregateNodeInputs("target", nodes, wire("ctx")).combinedPrompt).toBe("");
    });

    /**
     * **发的是词表里的提示词，不是显示串。**
     *
     * 「推镜头」四个字发给模型基本不起作用。这个坑在分镜卡片上踩过一次
     * （显示串同时当 id 和提示词用）。
     */
    it("技能节点发词表文本而不是显示串", () => {
      const nodes: CanvasNode[] = [
        {
          id: "sk",
          type: "skill",
          position: { x: 0, y: 0 },
          data: { picks: ["shotSize:close_up"] },
        },
        target("sdxl"),
      ];
      const out = aggregateNodeInputs("target", nodes, wire("sk")).combinedPrompt;
      expect(out).not.toBe("close_up");
      expect(out.toLowerCase()).toContain("close");
    });

    /** 中文引擎发中文 —— 选错了不报错，只是那一段基本不生效。 */
    it("技能节点按下游引擎选中英", () => {
      const nodes = (engine: string): CanvasNode[] => [
        {
          id: "sk",
          type: "skill",
          position: { x: 0, y: 0 },
          data: { picks: ["shotSize:close_up"] },
        },
        target(engine),
      ];
      const zh = aggregateNodeInputs("target", nodes("zimage"), wire("sk")).combinedPrompt;
      const en = aggregateNodeInputs("target", nodes("sdxl"), wire("sk")).combinedPrompt;
      expect(zh).not.toBe(en);
      expect(/[\u4e00-\u9fa5]/.test(zh)).toBe(true);
      expect(/[\u4e00-\u9fa5]/.test(en)).toBe(false);
    });

    /** 认不出的 id 跳过，**不要发一个半截的 id 出去**。 */
    it("认不出的词条被跳过", () => {
      const nodes: CanvasNode[] = [
        {
          id: "sk",
          type: "skill",
          position: { x: 0, y: 0 },
          data: { picks: ["shotSize:根本没有这条", "乱写的", "shotSize:"] },
        },
        target("zimage"),
      ];
      expect(aggregateNodeInputs("target", nodes, wire("sk")).combinedPrompt).toBe("");
    });

    /** 上下文排在画面描述前面 —— 提示词越靠后越容易被稀释。 */
    it("上下文排在画面描述前面", () => {
      const nodes: CanvasNode[] = [
        {
          id: "ctx",
          type: "shotContext",
          position: { x: 0, y: 0 },
          data: { sceneName: "面馆", viewMap: "", lock: "地理锁：柜台靠北墙。", blocking: "", state: "" },
        },
        {
          id: "p1",
          type: "prompt",
          position: { x: 0, y: 100 },
          data: { label: "", prompt: "雨夜，霓虹" },
        },
        target("zimage"),
      ];
      const edges: CanvasEdge[] = [
        { id: "e1", source: "ctx", target: "target", targetHandle: "prompt-in" },
        { id: "e2", source: "p1", target: "target", targetHandle: "prompt-in" },
      ];
      const out = aggregateNodeInputs("target", nodes, edges).combinedPrompt;
      expect(out.indexOf("地理锁")).toBeLessThan(out.indexOf("雨夜"));
    });
  });

  describe("360° 全景节点", () => {
    const pano = (data: Record<string, unknown>): CanvasNode[] => [
      { id: "pano", type: "panorama", position: { x: 0, y: 0 }, data: { sceneName: "面馆", prompt: "", longSide: 2048, warnings: [], ...data } },
      { id: "target", type: "imageGen", position: { x: 300, y: 0 }, data: { engine: "zimage", width: 1024, height: 1024, seed: 1 } },
    ];
    const edges: CanvasEdge[] = [
      { id: "e1", source: "pano", target: "target", targetHandle: "prompt-in" },
    ];

    /**
     * **画幅由全景节点定死，顶掉下游自己设的。**
     *
     * 等距圆柱投影就是 2:1，别的比例当全景用会在上下两极撕开。
     * 和「视频模式由连线推导」同一个道理：图里已经决定了的事不该再问用户。
     */
    it("顶掉下游节点的画幅", () => {
      const inputs = aggregateNodeInputs("target", pano({ prompt: "全景" }), edges);
      expect(inputs.forcedSize).toEqual({ width: 2048, height: 1024 });
    });

    it("没有全景节点时画幅不被顶", () => {
      const nodes: CanvasNode[] = [
        { id: "p1", type: "prompt", position: { x: 0, y: 0 }, data: { label: "", prompt: "雨夜" } },
        { id: "target", type: "imageGen", position: { x: 300, y: 0 }, data: { engine: "zimage", width: 1024, height: 1024, seed: 1 } },
      ];
      const inputs = aggregateNodeInputs("target", nodes, [
        { id: "e", source: "p1", target: "target", targetHandle: "prompt-in" },
      ]);
      expect(inputs.forcedSize).toBeNull();
    });

    /**
     * **签名要认 `forcedSize`。**
     *
     * 不认的话，接上全景节点之后画幅变了而签名没变，节点显示「已是最新」，
     * 用户拿到的还是上一张方图 —— 又一次静默失败。
     */
    it("接上全景节点会让签名变化", () => {
      const withPano = computeImageNodeSignature("target", pano({ prompt: "全景" }), edges);
      const without = computeImageNodeSignature("target", pano({ prompt: "全景" }), []);
      expect(withPano).not.toBe(without);
      expect(withPano).toContain("2048");
    });

    it("空提示词的全景节点什么都不发", () => {
      const inputs = aggregateNodeInputs("target", pano({ prompt: "" }), edges);
      expect(inputs.combinedPrompt).toBe("");
    });
  });

  describe("aggregateNodeInputs 节点输入聚合", () => {
    it("正确汇总 Prompt 节点与 Character 节点输入", () => {
      const nodes: CanvasNode[] = [
        {
          id: "p1",
          type: "prompt",
          position: { x: 0, y: 0 },
          data: { label: "镜头词", prompt: "cyberpunk city, neon rain", scale: "close_up" },
        },
        {
          id: "c1",
          type: "character",
          position: { x: 0, y: 100 },
          data: {
            characterId: "char_01",
            name: "薇薇安",
            promptTrigger: "vivian_girl",
            appearancePrompt: "silver short hair, blue eyes",
            avatarUrl: "/path/to/avatar.png",
          },
        },
        {
          id: "target",
          type: "imageGen",
          position: { x: 200, y: 50 },
          data: { engine: "zimage", width: 1024, height: 1024, seed: 1 },
        },
      ];

      const edges: CanvasEdge[] = [
        { id: "e1", source: "p1", target: "target", targetHandle: "prompt-in" },
        { id: "e2", source: "c1", target: "target", targetHandle: "prompt-in" },
      ];

      const inputs = aggregateNodeInputs("target", nodes, edges);
      expect(inputs.combinedPrompt).toContain("vivian_girl");
      expect(inputs.combinedPrompt).toContain("cyberpunk city, neon rain");
      expect(inputs.combinedPrompt).toContain("shot scale: close_up");
      expect(inputs.character?.name).toBe("薇薇安");
    });

    it("连接至 ref-in 时正确获取参考图路径", () => {
      const nodes: CanvasNode[] = [
        {
          id: "c1",
          type: "character",
          position: { x: 0, y: 0 },
          data: {
            characterId: "char_01",
            name: "角色",
            promptTrigger: "char_trig",
            appearancePrompt: "desc",
            avatarUrl: "/path/to/char.png",
          },
        },
        {
          id: "target",
          type: "imageGen",
          position: { x: 200, y: 0 },
          data: { engine: "zimage", width: 1024, height: 1024, seed: 1 },
        },
      ];

      const edges: CanvasEdge[] = [
        { id: "e1", source: "c1", target: "target", targetHandle: "ref-in" },
      ];

      const inputs = aggregateNodeInputs("target", nodes, edges);
      expect(inputs.inputImageUrl).toBe("/path/to/char.png");
    });

    it("上游图片节点的提示词（如角色三视图）绝不泄露给下游视频节点", () => {
      const nodes: CanvasNode[] = [
        {
          id: "img1",
          type: "imageGen",
          position: { x: 0, y: 0 },
          data: {
            engine: "zimage",
            width: 1024,
            height: 1024,
            seed: 1,
            prompt: "清冷腹黑病娇女学生，高精度3D半写实国漫女性角色设定图，三视图，纯白色背景",
            outputImageUrl: "/output/xiaoxiao.png",
          },
        },
        {
          id: "img2",
          type: "imageGen",
          position: { x: 0, y: 200 },
          data: {
            engine: "zimage",
            width: 1024,
            height: 1024,
            seed: 2,
            prompt: "冷感疏离暗系文艺少女，高精度3D半写实国漫女性角色设定图，三视图，纯白色背景",
            outputImageUrl: "/output/xiahe.png",
          },
        },
        {
          id: "video1",
          type: "videoGen",
          position: { x: 300, y: 100 },
          data: {
            engine: "h3",
            width: 640,
            height: 384,
            durationSecs: 5,
            mode: "r2v",
            prompt: "@筱筱 和 @夏禾 走在校园的小路上",
          },
        },
      ];

      const edges: CanvasEdge[] = [
        { id: "e1", source: "img1", target: "video1" },
        { id: "e2", source: "img2", target: "video1" },
      ];

      const inputs = aggregateNodeInputs("video1", nodes, edges);
      // 上游图片节点的画图提示词（三视图/纯白色背景）绝不可进入视频节点的 combinedPrompt
      expect(inputs.combinedPrompt).toBe("");
      expect(inputs.inputImageUrl).toBe("/output/xiahe.png");
      expect(inputs.upstreamImageNode).toBeDefined();
    });

    it("位于图片节点更上游的提示词节点亦被图片资产边界阻断，不泄露给下游视频节点", () => {
      const nodes: CanvasNode[] = [
        {
          id: "p_char",
          type: "prompt",
          position: { x: 0, y: 0 },
          data: { label: "角色设定词", prompt: "角色立绘，三视图全身展开" },
        },
        {
          id: "img1",
          type: "imageGen",
          position: { x: 200, y: 0 },
          data: { engine: "zimage", width: 1024, height: 1024, seed: 1, outputImageUrl: "/output/char.png" },
        },
        {
          id: "p_video",
          type: "prompt",
          position: { x: 200, y: 150 },
          data: { label: "视频氛围", prompt: "傍晚夕阳，电影质感光影" },
        },
        {
          id: "video1",
          type: "videoGen",
          position: { x: 400, y: 50 },
          data: { engine: "h3", width: 640, height: 384, durationSecs: 5, mode: "r2v" },
        },
      ];

      const edges: CanvasEdge[] = [
        { id: "e1", source: "p_char", target: "img1" },
        { id: "e2", source: "img1", target: "video1" },
        { id: "e3", source: "p_video", target: "video1" },
      ];

      const inputs = aggregateNodeInputs("video1", nodes, edges);
      // 只有直连视频节点的 p_video 提示词传递下来，img1 上游的 p_char 被媒体边界阻隔
      expect(inputs.combinedPrompt).toBe("傍晚夕阳，电影质感光影");
      expect(inputs.combinedPrompt).not.toContain("三视图");
    });
  });

  describe("executeSingleNode & executeNodePipeline", () => {
    it("preserves backend notices", async () => {
      const id = useCanvasStore.getState().addNode("imageGen", undefined, { engine: "zimage" });
      vi.mocked(studioGenerate).mockResolvedValue({ ok: true, path: "/out.png", message: "ok", notes: ["backend warning"] });
      await executeSingleNode(id);
      expect(studioGenerate).toHaveBeenCalledWith(expect.objectContaining({ engine: "zimage" }));
      expect(useCanvasStore.getState().nodes.find((n) => n.id === id)?.data.notes).toContain("backend warning");
    });
    // chroma / chroma1：2026-10-01 连权重一起下架。老节点不偷偷换成别的引擎 —— 和 flux 同一个规矩
    it.each(["flux", "sulphur", "chroma", "chroma1", "unknown"])("does not silently substitute unsupported engine %s", async (engine) => {
      const id = useCanvasStore.getState().addNode("imageGen", undefined, { engine });
      expect(await executeSingleNode(id)).toBe(false);
      expect(studioGenerate).not.toHaveBeenCalled();
    });
    it("catalog failure blocks generation and releases busy state", async () => {
      const id = useCanvasStore.getState().addNode("videoGen");
      vi.mocked(studioCapabilities).mockRejectedValue(new Error("catalog offline"));
      expect(await executeSingleNode(id)).toBe(false);
      expect(studioGenerate).not.toHaveBeenCalled();
      expect(useCanvasStore.getState().nodes.find((n) => n.id === id)?.data.isGenerating).toBe(false);
    });
    it("shows video notices too", async () => {
      const id = useCanvasStore.getState().addNode("videoGen");
      vi.mocked(studioGenerate).mockResolvedValue({ ok: true, path: "/out.mp4", message: "ok", notes: ["video warning"] });
      await executeSingleNode(id);
      expect(useCanvasStore.getState().nodes.find((n) => n.id === id)?.data.notes).toEqual(["video warning"]);
    });
    it("sends a bound asset image as I2I input without inserting its path into the prompt", async () => {
      const store = useCanvasStore.getState();
      const id = store.addNode("imageGen", undefined, { prompt: "portrait", references: [{ path: "/asset.png", name: "Hero", assetId: "a" }] });
      vi.mocked(studioGenerate).mockResolvedValue({ ok: true, path: "/out.png", message: "ok", notes: [] });
      await executeSingleNode(id);
      expect(studioGenerate).toHaveBeenCalledWith(expect.objectContaining({ mode: "i2i", images: ["/asset.png"], prompt: "portrait" }));
    });

    it("bound local image is used for automatic I2V", async () => {
      const id = useCanvasStore.getState().addNode("videoGen", undefined, { references: [{ path: "/upload.png", name: "Upload" }] });
      vi.mocked(studioGenerate).mockResolvedValue({ ok: true, path: "/out.mp4", message: "ok", notes: [] });
      await executeSingleNode(id);
      expect(studioGenerate).toHaveBeenCalledWith(expect.objectContaining({ mode: "i2v", images: ["/upload.png"] }));
    });

    it("merges bound R2V assets with upstream images and deduplicates", async () => {
      const id = referenceGraph(1);
      useCanvasStore.getState().updateNodeData(id, { references: [{ path: "/bound.png", name: "Bound" }, { path: "/ref0.png", name: "Same" }] });
      vi.mocked(studioGenerate).mockResolvedValue({ ok: true, path: "/out.mp4", message: "ok", notes: [] });
      await executeSingleNode(id);
      expect(studioGenerate).toHaveBeenCalledWith(expect.objectContaining({ mode: "r2v", images: ["/bound.png", "/ref0.png"] }));
    });

    /**
     * **手动选的模式和连线不一致时，不拦，但要说出来。**
     *
     * 以前这里抛「多参考图需要切换为参考生视频」—— 那等于让用户在不理解
     * 四个模式名的情况下自己去猜哪个对。现在模式由连线推导，用户手动选过
     * 就尊重他，但把推导结果写进节点的 notes：默默按他选的跑、悄悄只用
     * 其中一张图，是更糟的那一种 —— 他会以为两张都用上了。
     */
    it("手动选的模式和连线不一致时照跑，但把冲突写进 notes", async () => {
      const id = referenceGraph(0);
      useCanvasStore.getState().updateNodeData(id, {
        mode: "i2v",
        references: [{ path: "/a.png", name: "甲" }, { path: "/b.png", name: "乙" }],
      });
      vi.mocked(studioGenerate).mockResolvedValue({ ok: true, path: "/out.mp4", message: "ok", notes: [] });
      expect(await executeSingleNode(id)).toBe(true);
      expect(studioGenerate).toHaveBeenCalledWith(expect.objectContaining({ mode: "i2v" }));
      const notes = useCanvasStore.getState().nodes.find((n) => n.id === id)?.data.notes as string[];
      expect(notes.some((n) => n.includes("手动选了"))).toBe(true);
      expect(notes.some((n) => n.includes("图片参考"))).toBe(true);
    });

    // 用户 2026-09-25：运镜「可以叠几个，用 @ 的形式在提示词里标记顺序」—— 发出去的是运镜描述，按标签的前后
    it("提示词里的 @运镜 标签原位换成运镜描述，顺序照写的顺序", async () => {
      const store = useCanvasStore.getState();
      const id = store.addNode("videoGen", undefined, { engine: "ltx", prompt: "女孩站在走廊里，@运镜·跟拍 然后 @运镜·推镜头 收尾" });
      vi.mocked(studioGenerate).mockResolvedValue({ ok: true, path: "/out.mp4", message: "ok", notes: [] });
      expect(await executeSingleNode(id)).toBe(true);
      const sent = vi.mocked(studioGenerate).mock.calls.at(-1)![0].prompt as string;
      expect(sent).not.toContain("@运镜");
      expect(sent).not.toContain("运镜·");
      const follow = sent.indexOf("跟随"), push = sent.indexOf("推近");
      expect(follow).toBeGreaterThanOrEqual(0);
      expect(push).toBeGreaterThan(follow);
    });

    it("removing a binding removes it from the next request", async () => {
      const store = useCanvasStore.getState();
      const id = store.addNode("imageGen", undefined, { references: [{ path: "/asset.png", name: "A" }] });
      store.updateNodeData(id, { references: [] });
      vi.mocked(studioGenerate).mockResolvedValue({ ok: true, path: "/out.png", message: "ok", notes: [] });
      await executeSingleNode(id);
      expect(studioGenerate).toHaveBeenCalledWith(expect.objectContaining({ mode: "t2i", images: [] }));
    });
    function referenceGraph(count: number, engine = "h3") {
      const store = useCanvasStore.getState();
      const target = store.addNode("videoGen", undefined, { mode: "r2v", engine });
      const ids = Array.from({ length: count }, (_, i) => store.addNode("imageGen", undefined, { outputImageUrl: `/ref${i}.png` }));
      store.setEdges(ids.map((id, i) => ({ id: `ref-${i}`, source: id, target })));
      return target;
    }

    it("R2V sends all connected references through the existing Studio route", async () => {
      const id = referenceGraph(4);
      vi.mocked(studioGenerate).mockResolvedValue({ ok: true, path: "/out.mp4", message: "ok", notes: [] });
      await executeNodePipeline(id);
      expect(studioGenerate).toHaveBeenCalledTimes(1);
      expect(studioGenerate).toHaveBeenCalledWith(expect.objectContaining({ mode: "r2v", engine: "h3", images: ["/ref0.png", "/ref1.png", "/ref2.png", "/ref3.png"] }));
    });

    // 2026-10-11：长镜头节点点「生成」以前按空提示词出单条（H3 跑 50 分钟出一条参考图陈列）；
    // 面板上另有「整段出片」—— 用户：「生成按钮就是生成按钮」。现在「生成」直接出整条
    describe("长镜头节点点生成", () => {
      function longTakeNode() {
        const id = referenceGraph(4);
        useCanvasStore.getState().updateNodeData(id, {
          prompt: "", longTakeWindows: ["窗口一的提示词", "窗口二的提示词"], durationSecs: 15, width: 960, height: 544, h3Quality: "lightx2v",
        });
        return id;
      }

      it("一次发整条：每个窗口都挂四张图、带加速档，不走单条出片", async () => {
        const id = longTakeNode();
        vi.mocked(canvasLongTake).mockResolvedValue({ video_path: "/ops/longtake_x.mp4", windows: 2, net_secs: 29.1 });
        await executeNodePipeline(id);
        expect(studioGenerate).not.toHaveBeenCalled();
        expect(canvasLongTake).toHaveBeenCalledTimes(1);
        const arg = vi.mocked(canvasLongTake).mock.calls[0][0];
        expect(arg.windows.map((w) => w.prompt)).toEqual(["窗口一的提示词", "窗口二的提示词"]);
        for (const w of arg.windows) expect(w.images).toEqual(["/ref0.png", "/ref1.png", "/ref2.png", "/ref3.png"]);
        expect(arg).toMatchObject({ windowSecs: 15, width: 960, height: 544, h3Quality: "lightx2v" });
        const data = useCanvasStore.getState().nodes.find((n) => n.id === id)!.data;
        // 播放器、超分、收进分镜只认 outputVideoUrl；lastRun 记「当时到底发了什么」
        expect(data.outputVideoUrl).toBe("/ops/longtake_x.mp4");
        expect(String((data.lastRun as { prompt: string }).prompt)).toContain("【窗口 2】窗口二的提示词");
        expect(data.error).toBeUndefined();
      });

      // 2026-10-11：「提示词风格」写进节点自己的提示词框，长镜头以前整个忽略
      it("节点提示词框（提示词风格）加进每个窗口", async () => {
        const id = longTakeNode();
        useCanvasStore.getState().updateNodeData(id, { prompt: "cinematic photorealism, 35mm lens" });
        vi.mocked(canvasLongTake).mockResolvedValue({ video_path: "/ops/x.mp4", windows: 2, net_secs: 29.1 });
        await executeNodePipeline(id);
        const arg = vi.mocked(canvasLongTake).mock.calls[0][0];
        for (const w of arg.windows) expect(w.prompt).toContain("cinematic photorealism, 35mm lens");
      });

      it("有窗口没写提示词就不发", async () => {
        const id = longTakeNode();
        useCanvasStore.getState().updateNodeData(id, { longTakeWindows: ["有", "  "] });
        await executeNodePipeline(id);
        expect(canvasLongTake).not.toHaveBeenCalled();
        expect(useCanvasStore.getState().nodes.find((n) => n.id === id)?.data.error).toContain("第 2 个窗口还没写提示词");
      });
    });

    it("compiles @图片1 and @图片2 mentions into <Picture 1> and <Picture 2> for H3 R2V", async () => {
      const id = referenceGraph(2);
      useCanvasStore.getState().updateNodeData(id, {
        prompt: "让 @图片1 与 @图片2 在雪地中相拥",
      });
      vi.mocked(studioGenerate).mockResolvedValue({ ok: true, path: "/out.mp4", message: "ok", notes: [] });
      await executeNodePipeline(id);
      expect(studioGenerate).toHaveBeenCalledWith(
        expect.objectContaining({
          prompt: "让 <Picture 1> 与 <Picture 2> 在雪地中相拥",
          images: ["/ref0.png", "/ref1.png"],
        })
      );
    });

    it("R2V execution ignores upstream image prompts so character turnaround descriptions do not pollute video", async () => {
      const store = useCanvasStore.getState();
      const target = store.addNode("videoGen", undefined, {
        mode: "r2v",
        engine: "h3",
        prompt: "@筱筱 和 @夏禾 走在校园的小路上",
        references: [
          { path: "/xiaoxiao.png", name: "筱筱" },
          { path: "/xiahe.png", name: "夏禾" },
        ],
      });
      const img1 = store.addNode("imageGen", undefined, {
        outputImageUrl: "/xiaoxiao.png",
        prompt: "高精度 3D半写实女性角色设定图，三视图，纯白色背景",
      });
      const img2 = store.addNode("imageGen", undefined, {
        outputImageUrl: "/xiahe.png",
        prompt: "冷感暗系少女设定图，三视图，正面侧面背面",
      });
      store.setEdges([
        { id: "e1", source: img1, target },
        { id: "e2", source: img2, target },
      ]);

      vi.mocked(studioGenerate).mockResolvedValue({ ok: true, path: "/out.mp4", message: "ok", notes: [] });
      await executeNodePipeline(target);

      expect(studioGenerate).toHaveBeenCalledWith(
        expect.objectContaining({
          prompt: "<Picture 1> 和 <Picture 2> 走在校园的小路上",
          images: ["/xiaoxiao.png", "/xiahe.png"],
        })
      );
      const calledPrompt = vi.mocked(studioGenerate).mock.calls.find((c) => c[0].mode === "r2v")?.[0].prompt;
      expect(calledPrompt).not.toContain("三视图");
      expect(calledPrompt).not.toContain("纯白色背景");
      expect(calledPrompt).not.toContain("角色设定图");
    });

    it.each([[0, "h3"], [10, "h3"], [1, "ltx"]])("rejects invalid reference count/model (%s, %s) before submission", async (count, engine) => {
      const id = referenceGraph(count as number, engine as string);
      expect(await executeSingleNode(id)).toBe(false);
      expect(studioGenerate).not.toHaveBeenCalled();
      expect(useCanvasStore.getState().nodes.find((n) => n.id === id)?.data.error).toBeTruthy();
    });

    it("deduplicates references and does not add an image's ancestors as extra slots", () => {
      const id = referenceGraph(2);
      const store = useCanvasStore.getState();
      const images = store.nodes.filter((n) => n.type === "imageGen");
      store.updateNodeData(images[1].id, { outputImageUrl: "/ref0.png" });
      const ancestor = store.addNode("imageGen", undefined, { outputImageUrl: "/ancestor.png" });
      store.setEdges([...store.edges, { id: "ancestor", source: ancestor, target: images[0].id }]);
      const current = useCanvasStore.getState();
      expect(videoInputImages(id, current.nodes, current.edges)).toEqual(["/ref0.png"]);
    });

    /** 以前绑两张图直接抛「只支持一张源图」，而后端早有参考生图。 */
    it("image node with two references runs reference-to-image and says which model it used", async () => {
      const store = useCanvasStore.getState();
      const id = store.addNode("imageGen", undefined, { engine: "zimage", prompt: "她坐在茶馆里" });
      store.bindReference(store.activeProjectId, id, { path: "/scene.png", name: "茶馆" });
      store.bindReference(store.activeProjectId, id, { path: "/girl.png", name: "林晚" });
      vi.mocked(studioGenerate).mockResolvedValue({ ok: true, path: "/out.png", message: "ok", notes: [] });
      expect(await executeSingleNode(id)).toBe(true);
      expect(studioGenerate).toHaveBeenCalledWith(expect.objectContaining({ mode: "ref2i", engine: "krea2_edit", images: ["/scene.png", "/girl.png"] }));
      const notes = useCanvasStore.getState().nodes.find((n) => n.id === id)!.data.notes as string[];
      expect(notes.join("\n")).toContain("场景图放第一张");
      expect(notes.join("\n")).toContain("zimage");
    });

    it("pose toggle sends the first reference as pose image on SDXL", async () => {
      const store = useCanvasStore.getState();
      const id = store.addNode("imageGen", undefined, { engine: "zimage", prompt: "女孩在海边", poseFromReference: true });
      store.bindReference(store.activeProjectId, id, { path: "/pose.png", name: "舞姿" });
      vi.mocked(studioGenerate).mockResolvedValue({ ok: true, path: "/out.png", message: "ok", notes: [] });
      await executeSingleNode(id);
      expect(studioGenerate).toHaveBeenCalledWith(expect.objectContaining({ mode: "pose", engine: "sdxl", images: ["/pose.png"] }));
    });

    it("scene sheet sends fixed 2x2 viewpoints and keeps every cut cell", async () => {
      const store = useCanvasStore.getState();
      const id = store.addNode("imageGen", undefined, { engine: "zimage", prompt: "老街茶馆", sceneSheet: true });
      vi.mocked(studioGenerate).mockResolvedValue({ ok: true, path: "/sheet.png", paths: ["/sheet.png", "/c1.png", "/c2.png", "/c3.png", "/c4.png"], message: "ok", notes: [] });
      expect(await executeSingleNode(id)).toBe(true);
      const call = vi.mocked(studioGenerate).mock.calls[0][0];
      expect(call.mode).toBe("scene_sheet");
      expect(call.viewpoints).toHaveLength(4);
      expect(call.viewpoints![0].name).toContain("正面");
      expect(useCanvasStore.getState().nodes.find((n) => n.id === id)!.data.outputImages).toHaveLength(5);
    });

    it("image node with one reference stays image-to-image on a normal model", async () => {
      const store = useCanvasStore.getState();
      const id = store.addNode("imageGen", undefined, { engine: "zimage", prompt: "换成夜景" });
      store.bindReference(store.activeProjectId, id, { path: "/a.png", name: "源图" });
      vi.mocked(studioGenerate).mockResolvedValue({ ok: true, path: "/out.png", message: "ok", notes: [] });
      await executeSingleNode(id);
      expect(studioGenerate).toHaveBeenCalledWith(expect.objectContaining({ mode: "i2i", images: ["/a.png"] }));
    });

    /** 首尾帧以前发的是空数组，后端报「需要 2 张图」—— 画布上的首尾帧从来没跑通过。 */
    it("FL2V sends start then end frame", async () => {
      const store = useCanvasStore.getState();
      const id = store.addNode("videoGen", undefined, { engine: "ltx", prompt: "走过去" });
      store.bindReference(store.activeProjectId, id, { path: "/end.png", name: "尾帧" });
      store.bindReference(store.activeProjectId, id, { path: "/start.png", name: "开场" });
      vi.mocked(studioGenerate).mockResolvedValue({ ok: true, path: "/out.mp4", message: "ok", notes: [] });
      await executeSingleNode(id);
      expect(useCanvasStore.getState().nodes.find((n) => n.id === id)?.data.error).toBeUndefined();
      expect(studioGenerate).toHaveBeenCalledWith(expect.objectContaining({ mode: "fl2v", images: ["/start.png", "/end.png"] }));
    });

    it("includes a character's actual image in R2V", async () => {
      const id = referenceGraph(0);
      const store = useCanvasStore.getState();
      const character = store.addNode("character", undefined, { avatarUrl: "/character.png" });
      store.setEdges([{ id: "character", source: character, target: id }]);
      vi.mocked(studioGenerate).mockResolvedValue({ ok: true, path: "/out.mp4", message: "ok", notes: [] });
      await executeNodePipeline(id);
      expect(studioGenerate).toHaveBeenCalledWith(expect.objectContaining({ mode: "r2v", images: ["/character.png"] }));
    });

    it("explicit T2V does not attach connected images", async () => {
      const id = referenceGraph(1);
      useCanvasStore.getState().updateNodeData(id, { mode: "t2v" });
      vi.mocked(studioGenerate).mockResolvedValue({ ok: true, path: "/out.mp4", message: "ok", notes: [] });
      await executeSingleNode(id);
      expect(studioGenerate).toHaveBeenCalledWith(expect.objectContaining({ mode: "t2v", images: [] }));
    });
    it("文生图时调用 studioGenerate 且成功写入 outputImageUrl", async () => {
      const store = useCanvasStore.getState();
      const pId = store.addNode("prompt");
      store.updateNodeData(pId, { prompt: "a cute robot exploring space" });

      const imgId = store.addNode("imageGen");
      store.updateNodeData(imgId, { engine: "zimage", width: 1024, height: 1024 });

      store.setEdges([
        {
          id: "e1",
          source: pId,
          target: imgId,
          targetHandle: "prompt-in",
        },
      ]);

      vi.mocked(studioGenerate).mockResolvedValueOnce({
        ok: true,
        path: "/mock/workspace/generated/studio_t2i_12345.png",
        message: "ok",
        notes: [],
      });

      await executeNodePipeline(imgId);

      expect(studioGenerate).toHaveBeenCalledWith(
        expect.objectContaining({
          mode: "t2i",
          engine: "zimage",
          prompt: "a cute robot exploring space",
          width: 1024,
          height: 1024,
          images: [],
        }),
      );

      const targetData = useCanvasStore.getState().nodes.find((n) => n.id === imgId)?.data;
      expect(targetData?.outputImageUrl).toBe("/mock/workspace/generated/studio_t2i_12345.png");
      expect(targetData?.isGenerating).toBe(false);
      expect(targetData?.error).toBeUndefined();
    });

    it("带上游图片时自动使用 i2i 模式传图", async () => {
      const store = useCanvasStore.getState();
      const prevImgId = store.addNode("imageGen");
      store.updateNodeData(prevImgId, {
        outputImageUrl: "/mock/first_step.png",
      });

      const nextImgId = store.addNode("imageGen");
      store.setEdges([
        {
          id: "e1",
          source: prevImgId,
          target: nextImgId,
          targetHandle: "ref-in",
        },
      ]);

      vi.mocked(studioGenerate).mockResolvedValueOnce({
        ok: true,
        path: "/mock/second_step.png",
        message: "ok",
        notes: [],
      });

      await executeNodePipeline(nextImgId);

      expect(studioGenerate).toHaveBeenCalledWith(
        expect.objectContaining({
          mode: "i2i",
          images: ["/mock/first_step.png"],
        }),
      );

      const targetData = useCanvasStore.getState().nodes.find((n) => n.id === nextImgId)?.data;
      expect(targetData?.outputImageUrl).toBe("/mock/second_step.png");
    });

    it("视频节点生成时使用 t2v 或 i2v 模式", async () => {
      const store = useCanvasStore.getState();
      const vidId = store.addNode("videoGen");
      store.updateNodeData(vidId, { engine: "h3", durationSecs: 4 });

      vi.mocked(studioGenerate).mockResolvedValueOnce({
        ok: true,
        path: "/mock/video_out.mp4",
        message: "ok",
        notes: [],
      });

      await executeNodePipeline(vidId);

      expect(studioGenerate).toHaveBeenCalledWith(
        expect.objectContaining({
          mode: "t2v",
          engine: "h3",
          duration: 4,
          images: [],
        }),
      );

      const targetData = useCanvasStore.getState().nodes.find((n) => n.id === vidId)?.data;
      expect(targetData?.outputVideoUrl).toBe("/mock/video_out.mp4");
    });

    it("生成失败时正确记录错误信息", async () => {
      const store = useCanvasStore.getState();
      const imgId = store.addNode("imageGen");

      vi.mocked(studioGenerate).mockResolvedValueOnce({
        ok: false,
        path: null,
        message: "ComfyUI 离线：无法连接到本地服务",
        notes: [],
      });

      await executeNodePipeline(imgId);

      const targetData = useCanvasStore.getState().nodes.find((n) => n.id === imgId)?.data;
      expect(targetData?.isGenerating).toBe(false);
      expect(targetData?.error).toBe("ComfyUI 离线：无法连接到本地服务");
      expect(targetData?.outputImageUrl).toBeUndefined();
    });

    it("上游提示词修改后，执行下游时自动重新生成上游图片，避免使用旧图", async () => {
      const store = useCanvasStore.getState();
      const pId = store.addNode("prompt", undefined, { prompt: "初始猫咪提示词" });
      const imgId = store.addNode("imageGen", undefined, { engine: "zimage" });
      const vidId = store.addNode("videoGen", undefined, { engine: "h3" });

      store.setEdges([
        { id: "e1", source: pId, target: imgId, targetHandle: "prompt-in" },
        { id: "e2", source: imgId, target: vidId, targetHandle: "prompt-in" },
      ]);

      // 首次生成图片节点
      vi.mocked(studioGenerate).mockResolvedValueOnce({
        ok: true,
        path: "/mock/cat.png",
        message: "ok",
        notes: [],
      });
      await executeSingleNode(imgId);

      const firstImgData = useCanvasStore.getState().nodes.find((n) => n.id === imgId)?.data;
      expect(firstImgData?.outputImageUrl).toBe("/mock/cat.png");
      expect(firstImgData?.lastExecutedSignature).toBeDefined();

      // 用户修改上游提示词为小狗
      store.updateNodeData(pId, { prompt: "更新后的小狗提示词" });

      // 重新执行下游视频节点：必须自动先重新生成上游图片，再执行视频
      vi.mocked(studioGenerate)
        .mockResolvedValueOnce({
          ok: true,
          path: "/mock/dog_new.png",
          message: "ok",
          notes: [],
        })
        .mockResolvedValueOnce({
          ok: true,
          path: "/mock/dog_video.mp4",
          message: "ok",
          notes: [],
        });

      await executeNodePipeline(vidId);

      // 验证图片节点被重新生成，且拿到最新的小狗图片
      const updatedImgData = useCanvasStore.getState().nodes.find((n) => n.id === imgId)?.data;
      expect(updatedImgData?.outputImageUrl).toBe("/mock/dog_new.png");

      // 验证视频节点使用了新生成的狗图片作为首帧
      expect(studioGenerate).toHaveBeenLastCalledWith(
        expect.objectContaining({
          mode: "i2v",
          images: ["/mock/dog_new.png"],
        })
      );
    });

    it("共享上游节点在并发触发时通过执行锁去重，不重复提交任务", async () => {
      const store = useCanvasStore.getState();
      const sharedImgId = store.addNode("imageGen", undefined, { engine: "zimage", prompt: "共享背景" });

      let resolveGenerate: (val: any) => void;
      const delayedGenerate = new Promise((resolve) => {
        resolveGenerate = resolve;
      });

      vi.mocked(studioGenerate).mockImplementationOnce(() => delayedGenerate as any);

      // 同时调用两次 executeSingleNode
      const promise1 = executeSingleNode(sharedImgId);
      const promise2 = executeSingleNode(sharedImgId);

      // 解锁生成
      resolveGenerate!({
        ok: true,
        path: "/mock/shared_bg.png",
        message: "ok",
        notes: [],
      });

      const [res1, res2] = await Promise.all([promise1, promise2]);

      expect(res1).toBe(true);
      expect(res2).toBe(true);
      // 确认后端只被调用了一次
      expect(studioGenerate).toHaveBeenCalledTimes(1);
    });

    it("支持生成多张图片（imageCount: 2/4）并保留 outputImages 列表", async () => {
      const store = useCanvasStore.getState();
      const imgId = store.addNode("imageGen", undefined, {
        engine: "zimage",
        prompt: "多张画作",
        imageCount: 2,
        seed: 100,
      });

      vi.mocked(studioGenerate)
        .mockResolvedValueOnce({
          ok: true,
          path: "/mock/art_seed1.png",
          message: "ok",
          notes: [],
        })
        .mockResolvedValueOnce({
          ok: true,
          path: "/mock/art_seed2.png",
          message: "ok",
          notes: [],
        });

      await executeSingleNode(imgId);

      expect(studioGenerate).toHaveBeenCalledTimes(2);
      expect(studioGenerate).toHaveBeenNthCalledWith(
        1,
        expect.objectContaining({ seed: 100 })
      );
      expect(studioGenerate).toHaveBeenNthCalledWith(
        2,
        expect.objectContaining({ seed: 107 })
      );

      const targetData = useCanvasStore.getState().nodes.find((n) => n.id === imgId)?.data;
      expect(targetData?.outputImageUrl).toBe("/mock/art_seed1.png");
      expect(targetData?.outputImages).toEqual(["/mock/art_seed1.png", "/mock/art_seed2.png"]);
    });
  });
});

describe("长镜头窗口加上节点的提示词风格", () => {
  it("H3 官方格式放进画风那段的位置；其余接在最前面；已经有了不重复加", () => {
    const w = "subject_definitions:\n<Subject 1> is X.\n\ndetailed_description:\nStyle paragraph.\n[Shot 1] A.";
    expect(withSharedStyle(w, "photoreal")).toBe("subject_definitions:\n<Subject 1> is X.\n\ndetailed_description:\nVisual style: photoreal.\nStyle paragraph.\n[Shot 1] A.");
    expect(withSharedStyle("一个女孩在跑。", "写实")).toBe("写实\n\n一个女孩在跑。");
    expect(withSharedStyle(w, "")).toBe(w);
    const once = withSharedStyle(w, "photoreal");
    expect(withSharedStyle(once, "photoreal")).toBe(once);
  });
});

