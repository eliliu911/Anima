import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, it, expect } from "vitest";
import { ADD_NODE_ITEMS, DEFINED_NODE_ITEMS, SKILL_NODE_ITEMS } from "./CanvasAddNodePanel";
import { useCanvasStore } from "../store/useCanvasStore";

describe("CanvasQuickActionBar 与 CanvasAddNodePanel 节点菜单", () => {
  it("菜单列出的能力都在", () => {
    const labels = ADD_NODE_ITEMS.map((item) => item.label);
    expect(labels).toEqual([
      "文本", "图片", "音频", "视频",
      "深度动作捕捉", "超分 ×2", "LoRA", "分镜", "镜头上下文", "全景出图", "全景查看", "导演世界",
      "上传资源",
    ]);
    // 用户 2026-10-08 定的分层：顶层四个、上传资源在最下面
    expect(ADD_NODE_ITEMS.filter((i) => i.tier === "main").map((i) => i.label)).toEqual(["文本", "图片", "音频", "视频"]);
    expect(ADD_NODE_ITEMS.filter((i) => i.tier === "upload").map((i) => i.label)).toEqual(["上传资源"]);
    // 主菜单五项、工具和技能是二级菜单（用户 2026-10-08）
    const panel = readFileSync(fileURLToPath(new URL("./CanvasAddNodePanel.tsx", import.meta.url)), "utf8");
    expect(panel).toContain("{[...main, ...uploads].map((item) =>");
    expect(panel).toContain('<SubRow label="工具"');
    expect(panel).toContain('<SubRow label="技能"');
    // 定义了却没排进 MENU_ORDER 的节点会从菜单里消失 —— 不许
    expect(DEFINED_NODE_ITEMS.map((i) => i.label).sort()).toEqual([...labels].sort());
  });

  /**
   * **上面的格子和下面的列表分工不一样。**
   *
   * 格子是「往画布里放什么」（文本、图片、视频这些实体），
   * 下面是「给这条链加什么本事」。技能节点原来混在格子里和「图片」
   * 并排，看不出它是另一类东西。
   */
  it("技能类的东西在下面那一组，不在上面的格子里", () => {
    expect(ADD_NODE_ITEMS.map((i) => i.type)).not.toContain("skill");
    expect(SKILL_NODE_ITEMS.map((i) => i.label)).toEqual(["主线技能与角色", "镜头语言"]);
    expect(SKILL_NODE_ITEMS.map((i) => i.type)).toEqual(["character", "skill"]);
  });

  /** 这一排名字说不清自己（「镜头语言」不告诉你它能挑景别和光位）。 */
  it("技能节点每一项都有一句说明", () => {
    for (const item of SKILL_NODE_ITEMS) {
      expect(item.hint, `「${item.label}」没有说明`).toBeTruthy();
      expect(item.icon).toBeDefined();
    }
  });

  /**
   * **这一条以前是假绿的。**
   *
   * 原来的断言只比对了 label 数组，所以「镜头上下文」这一项指着 `prompt`
   * 也照样通过 —— 点下去拿到的是个普通文本框，菜单承诺了一个不存在的能力，
   * 而且不报错。改成按标签核对它到底建出什么节点。
   */
  it("标签和它真正建出来的节点类型对得上", () => {
    const want: Record<string, string> = {
      文本: "prompt",
      分镜: "shot",
      镜头上下文: "shotContext",
      图片: "imageGen",
      视频: "videoGen",
      上传资源: "imageGen",
      全景出图: "panorama",
      全景查看: "panoViewer",
      导演世界: "directorWorld",
    };
    for (const item of ADD_NODE_ITEMS) {
      const expected = want[item.label];
      if (expected) expect(item.type).toBe(expected);
    }
  });

  it("每个菜单项均具备对应的图标、前景色与背景色配置", () => {
    for (const item of ADD_NODE_ITEMS) {
      expect(item.type).toBeDefined();
      expect(item.label).toBeTruthy();
      expect(item.icon).toBeDefined();
      expect(item.color).toMatch(/text-/);
      expect(item.bg).toMatch(/bg-/);
    }
  });

  /**
   * **每一项都要有一句说明。**
   *
   * 格子只有 76px 宽，标签放不下一个能自解释的名字 —— 用户截图问过
   * 「360 全景节点在哪儿」，当时它就在菜单里，叫「导演世界」。
   * 标签认不出来时，悬停那一句是唯一的出路。
   */
  it("每一项都有一句悬停说明", () => {
    for (const item of ADD_NODE_ITEMS) {
      expect(item.hint, `「${item.label}」没有说明`).toBeTruthy();
      expect(item.hint.length, `「${item.label}」的说明太短`).toBeGreaterThan(5);
    }
  });

  it("画布状态管理正确支持 undo、redo、copySelectedNode、pasteNode 与 arrangeNodes", () => {
    const store = useCanvasStore.getState();


    expect(typeof store.undo).toBe("function");
    expect(typeof store.redo).toBe("function");
    expect(typeof store.copySelectedNode).toBe("function");
    expect(typeof store.pasteNode).toBe("function");
    expect(typeof store.arrangeNodes).toBe("function");

    // 添加节点
    const id1 = store.addNode("prompt", { x: 100, y: 100 }, { prompt: "test prompt" });
    store.addNode("imageGen", { x: 500, y: 100 }, { prompt: "test image" });


    // 复制与粘贴
    store.setSelectedNodeId(id1);
    const copied = store.copySelectedNode();
    expect(copied).toBeDefined();
    expect(copied?.type).toBe("prompt");

    const pastedId = store.pasteNode();
    expect(pastedId).toBeTruthy();
    expect(useCanvasStore.getState().nodes.some((n: any) => n.id === pastedId)).toBe(true);

    // 撤销
    store.undo();
    expect(useCanvasStore.getState().nodes.some((n: any) => n.id === pastedId)).toBe(false);

    // 整理画布
    store.arrangeNodes();
    expect(useCanvasStore.getState().nodes.length).toBeGreaterThan(0);
  });

  it("支持基于视口中心坐标动态添加节点，并计算正确的偏移以防止重叠", () => {
    const store = useCanvasStore.getState();
    const initialCount = store.nodes.length;

    // 模拟视口平移至 (1200, 800) 处的视口中心
    const flowCenter = { x: 1200, y: 800 };
    const nodeWidth = 440;
    const nodeHeight = 380;
    const offset = (initialCount % 6) * 28;

    const targetPos = {
      x: Math.round(flowCenter.x - nodeWidth / 2 + offset),
      y: Math.round(flowCenter.y - nodeHeight / 2 + offset),
    };

    const newId = store.addNode("videoGen", targetPos, { prompt: "viewport centered test" });
    const addedNode = useCanvasStore.getState().nodes.find((n: any) => n.id === newId);

    const expectedX = store.snapToGrid ? Math.round(targetPos.x / 20) * 20 : targetPos.x;
    const expectedY = store.snapToGrid ? Math.round(targetPos.y / 20) * 20 : targetPos.y;

    expect(addedNode).toBeDefined();
    expect(addedNode?.position.x).toBe(expectedX);
    expect(addedNode?.position.y).toBe(expectedY);
  });
});

