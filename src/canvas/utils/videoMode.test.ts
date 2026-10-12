import { describe, it, expect, beforeEach } from "vitest";
import { useCanvasStore } from "../store/useCanvasStore";
import { inferVideoMode, liveNotes, MODE_LABEL } from "./videoMode";

beforeEach(() => useCanvasStore.getState().clearCanvas());

const graph = () => {
  const s = useCanvasStore.getState();
  return { nodes: s.nodes, edges: s.edges };
};

describe("视频模式由连线推导", () => {
  it("什么都没连 = 文生视频", () => {
    const v = useCanvasStore.getState().addNode("videoGen");
    const { nodes, edges } = graph();
    const got = inferVideoMode(v, nodes, edges);
    expect(got.mode).toBe("t2v");
    expect(got.images).toEqual([]);
    expect(got.reason).toContain("没有连接任何图片");
  });

  it("连一张图 = 图生视频，说明它是首帧", () => {
    const s = useCanvasStore.getState();
    const v = s.addNode("videoGen");
    s.bindReference(s.activeProjectId, v, { path: "/a.png", name: "客厅" });
    const { nodes, edges } = graph();
    const got = inferVideoMode(v, nodes, edges);
    expect(got.mode).toBe("i2v");
    expect(got.reason).toContain("首帧");
    expect(got.images).toEqual(["/a.png"]);
  });

  it("连多张带名字的图 = 参考生视频", () => {
    const s = useCanvasStore.getState();
    const v = s.addNode("videoGen");
    s.bindReference(s.activeProjectId, v, { path: "/a.png", name: "沈星晴" });
    s.bindReference(s.activeProjectId, v, { path: "/b.png", name: "老村民" });
    const { nodes, edges } = graph();
    const got = inferVideoMode(v, nodes, edges);
    expect(got.mode).toBe("r2v");
    // 每张都 @ 得到：理由里直接告诉怎么指它们
    expect(got.reason).toContain("@图片1、@图片2");
  });

  /**
   * **没名字的参考要说出来。**
   * H3 靠名字把声音和身份绑到画面里那个人身上，`@画布图片` 指不到任何人。
   */
  it("没名字的参考也 @ 得到（@图片N），不再说「@ 不到」", () => {
    const s = useCanvasStore.getState();
    const v = s.addNode("videoGen");
    s.bindReference(s.activeProjectId, v, { path: "/a.png", name: "沈星晴" });
    s.bindReference(s.activeProjectId, v, { path: "/b.png", name: "画布图片" });
    const { nodes, edges } = graph();
    const got = inferVideoMode(v, nodes, edges);
    expect(got.mode).toBe("r2v");
    expect(got.reason).not.toContain("@ 不到");
    expect(got.reason).toContain("@图片2");
  });

  it("有一张标了尾帧 = 首尾帧", () => {
    const s = useCanvasStore.getState();
    const v = s.addNode("videoGen");
    s.bindReference(s.activeProjectId, v, { path: "/a.png", name: "开场" });
    s.bindReference(s.activeProjectId, v, { path: "/b.png", name: "尾帧" });
    const { nodes, edges } = graph();
    const got = inferVideoMode(v, nodes, edges);
    expect(got.mode).toBe("fl2v");
    expect(got.reason).toContain("尾帧");
    expect(got.images).toEqual(["/a.png", "/b.png"]);
  });

  /** 先连了尾帧也要按「首、尾」发 —— 否则视频倒着走，不报错。 */
  it("首尾帧先连尾帧时顺序照样是首、尾", () => {
    const s = useCanvasStore.getState();
    const v = s.addNode("videoGen");
    s.bindReference(s.activeProjectId, v, { path: "/end.png", name: "尾帧" });
    s.bindReference(s.activeProjectId, v, { path: "/start.png", name: "开场" });
    s.bindReference(s.activeProjectId, v, { path: "/x.png", name: "多余" });
    const { nodes, edges } = graph();
    const got = inferVideoMode(v, nodes, edges);
    expect(got.mode).toBe("fl2v");
    expect(got.images).toEqual(["/start.png", "/end.png"]);
    expect(got.reason).toContain("多出的 1 张");
  });

  it("视频节点能绑到 9 张参考图", () => {
    const s = useCanvasStore.getState();
    const v = s.addNode("videoGen");
    for (let i = 0; i < 9; i++) s.bindReference(s.activeProjectId, v, { path: `/r${i}.png`, name: `人${i}` });
    const node = useCanvasStore.getState().nodes.find((n) => n.id === v)!;
    expect((node.data.references as unknown[]).length).toBe(9);
    expect(() => s.bindReference(s.activeProjectId, v, { path: "/r9.png", name: "人9" })).toThrow("最多绑定 9 张");
  });

  /**
   * **理由里要带数量。**
   * 用户以为连了三张、实际有一张没连上的时候，只报模式名他看不出来。
   */
  it("理由里带数量", () => {
    const s = useCanvasStore.getState();
    const v = s.addNode("videoGen");
    for (const n of ["甲", "乙", "丙"]) {
      s.bindReference(s.activeProjectId, v, { path: `/${n}.png`, name: n });
    }
    const { nodes, edges } = graph();
    expect(inferVideoMode(v, nodes, edges).reason).toContain("3 张");
  });

  /**
   * 手动选了就尊重他，**但要把推导结果带出来** ——
   * 不带的话他看不出自己选的和连线不一致，而那正是最容易出错的地方。
   */
  it("手动选择压过推导，但推导结果照样报出来", () => {
    const s = useCanvasStore.getState();
    const v = s.addNode("videoGen", undefined, { mode: "t2v" });
    s.bindReference(s.activeProjectId, v, { path: "/a.png", name: "客厅" });
    const { nodes, edges } = graph();
    const got = inferVideoMode(v, nodes, edges);
    expect(got.mode).toBe("t2v");
    expect(got.overridden).toBe("i2v");
    expect(got.reason).toContain("手动选了");
    expect(got.reason).toContain("连线看起来是图生视频");
  });

  it("auto 和不填都走推导", () => {
    const s = useCanvasStore.getState();
    const a = s.addNode("videoGen", undefined, { mode: "auto" });
    const b = s.addNode("videoGen");
    const { nodes, edges } = graph();
    expect(inferVideoMode(a, nodes, edges).mode).toBe("t2v");
    expect(inferVideoMode(a, nodes, edges).overridden).toBeUndefined();
    expect(inferVideoMode(b, nodes, edges).overridden).toBeUndefined();
  });

  /** 参考指向已删除的节点时不该让推导崩 —— 那是执行时该报的另一个错。 */
  it("参考已删除时不崩，按没连处理", () => {
    const s = useCanvasStore.getState();
    const v = s.addNode("videoGen", undefined, {
      references: [{ path: "/x.png", name: "没了", sourceNodeId: "gone" }],
    });
    const { nodes, edges } = graph();
    expect(() => inferVideoMode(v, nodes, edges)).not.toThrow();
    expect(inferVideoMode(v, nodes, edges).mode).toBe("t2v");
  });

  it("每个模式都有中文名", () => {
    for (const m of ["t2v", "i2v", "fl2v", "r2v"] as const) {
      expect(MODE_LABEL[m].length).toBeGreaterThan(0);
    }
  });
});

// 用户 2026-09-25：「用角色卡的话，就会提示是首帧」—— 一张设定卡是人物外观，不是第一帧
describe("一张角色设定卡", () => {
  it("上传的设定卡连进来（图片节点标题是原文件名）= 参考生视频，不当首帧", () => {
    const s = useCanvasStore.getState();
    const img = s.addNode("imageGen", { x: 0, y: 0 }, { referenceOnly: true, label: "Rabbit Racer 设定卡（蓝）.png",
      outputImageUrl: "/uploads/8f3a.png", outputImages: ["/uploads/8f3a.png"] });
    const v = s.addNode("videoGen", { x: 500, y: 0 }, { engine: "h3" });
    s.setEdges([{ id: "e", source: img, target: v }]);
    const { nodes, edges } = graph();
    const got = inferVideoMode(v, nodes, edges);
    expect(got.mode).toBe("r2v");
    expect(got.reason).toContain("不当首帧");
    expect(got.overridden).toBeUndefined();
  });

  it("直接绑的参考图名字里带三视图 / character sheet 也认", () => {
    for (const name of ["陆瑾 三视图", "hero_character_sheet.png", "角色卡-A"]) {
      useCanvasStore.getState().clearCanvas();
      const s = useCanvasStore.getState();
      const v = s.addNode("videoGen");
      s.bindReference(s.activeProjectId, v, { path: "/x.png", name });
      const { nodes, edges } = graph();
      expect(inferVideoMode(v, nodes, edges).mode, name).toBe("r2v");
    }
  });

  it("普通的一张图照旧是首帧", () => {
    const s = useCanvasStore.getState();
    const img = s.addNode("imageGen", { x: 0, y: 0 }, { referenceOnly: true, label: "客厅.png", outputImageUrl: "/u/1.png" });
    const v = s.addNode("videoGen", { x: 500, y: 0 });
    s.setEdges([{ id: "e", source: img, target: v }]);
    const { nodes, edges } = graph();
    expect(inferVideoMode(v, nodes, edges).mode).toBe("i2v");
  });

  it("一张普通图、手选参考生视频：按参考生视频跑，说明是中性的（不当成选错了）", () => {
    const s = useCanvasStore.getState();
    const img = s.addNode("imageGen", { x: 0, y: 0 }, { referenceOnly: true, label: "人物.png", outputImageUrl: "/u/2.png" });
    const v = s.addNode("videoGen", { x: 500, y: 0 }, { mode: "r2v" });
    s.setEdges([{ id: "e", source: img, target: v }]);
    const { nodes, edges } = graph();
    const got = inferVideoMode(v, nodes, edges);
    expect(got.mode).toBe("r2v");
    expect(got.deliberate).toBe(true);
    expect(got.overridden).toBe("i2v"); // 菜单里「改回跟着连线走」要用
    expect(got.reason).not.toContain("但连线看起来");
  });

  it("两张图手选图生视频照旧警告（那才是容易选错的）", () => {
    const s = useCanvasStore.getState();
    const v = s.addNode("videoGen", { x: 0, y: 0 }, { mode: "i2v" });
    s.bindReference(s.activeProjectId, v, { path: "/a.png", name: "甲" });
    s.bindReference(s.activeProjectId, v, { path: "/b.png", name: "乙" });
    const { nodes, edges } = graph();
    const got = inferVideoMode(v, nodes, edges);
    expect(got.overridden).toBe("r2v");
    expect(got.deliberate).toBeFalsy();
  });
});

// 用户 2026-09-25：「这里还是显示黄字儿」—— 那是修之前那次运行存下来的模式提示
describe("节点底部的运行提示：模式那条按现在的推导算", () => {
  const stale = "你手动选了参考生视频，但连线看起来是图生视频（连了 1 张图 —— 它是首帧）";
  it("旧的模式提示不再显示；别的运行提示照留", () => {
    const now = { mode: "r2v" as const, reason: "参考生视频 —— 这 1 张图只当人物外观参考，不当首帧", images: ["/a.png"], overridden: "i2v" as const, deliberate: true };
    expect(liveNotes([stale, "LoRA 激活词：hmmotion"], now)).toEqual(["LoRA 激活词：hmmotion"]);
    expect(liveNotes([stale], now)).toBeUndefined();
  });
  // 用户 2026-09-25：「不用黄色提示词」—— 模式说明不在节点上显示，理由在模式按钮的悬停提示里
  it("选得和连线不一致时也不在节点上挂黄字", () => {
    const now = { mode: "i2v" as const, reason: "你手动选了图生视频，但连线看起来是图片参考", images: ["/a.png", "/b.png"], overridden: "r2v" as const };
    expect(liveNotes([stale], now)).toBeUndefined();
  });
});

// 用户 2026-09-25：模式菜单是明确给人选的 —— 两张图选首尾帧是正当用法，不当成选错
describe("两张图手选首尾帧", () => {
  const setup = (n: number) => {
    const s = useCanvasStore.getState();
    const ids = Array.from({ length: n }, (_, i) =>
      s.addNode("imageGen", { x: 0, y: i * 400 }, { referenceOnly: true, label: `图${i}.png`, outputImageUrl: `/p${i}.png` }));
    const v = s.addNode("videoGen", { x: 600, y: 0 }, { mode: "fl2v" });
    s.setEdges(ids.map((id, i) => ({ id: `e${i}`, source: id, target: v })));
    const { nodes, edges } = graph();
    return inferVideoMode(v, nodes, edges);
  };
  it("两张：按连线顺序首尾，中性说明", () => {
    const got = setup(2);
    expect(got.mode).toBe("fl2v");
    expect(got.images).toEqual(["/p0.png", "/p1.png"]);
    expect(got.deliberate).toBe(true);
  });
  it("三张：照旧警告（第三张用不上）", () => {
    const got = setup(3);
    expect(got.mode).toBe("fl2v");
    expect(got.deliberate).toBeFalsy();
    expect(got.reason).toContain("你手动选了");
  });
});
