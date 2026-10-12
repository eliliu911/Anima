import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { DEFAULT_LOCAL_EDIT_ENGINE, LOCAL_EDIT_ENGINES, LOCAL_EDIT_TEMPLATE, localEditEngine, stillPlaceholder } from "./localEdit";
import { inferImageMode } from "./imageMode";
import { canvasCatalogFixture } from "./modelCatalog.fixture";

describe("局部修改", () => {
  /**
   * 指令式编辑最典型的失败是「它把整张图重画了一遍，脸也跟着变了」——
   * 而这不报错。模板里那句「其余一切不变」是唯一的防线。
   */
  it("模板里写明其余不变，而且写的是哪些东西", () => {
    for (const w of ["构图", "五官", "姿势", "光线", "颜色", "背景"]) {
      expect(LOCAL_EDIT_TEMPLATE, w).toContain(w);
    }
  });

  /** 占位没替换就发出去，会得到一张字面画着「要改的东西」的图。 */
  it("占位还在时认得出来", () => {
    expect(stillPlaceholder(LOCAL_EDIT_TEMPLATE)).toBe(true);
    expect(stillPlaceholder("只把她手里的杯子换成手机。画面其余部分保持完全不变。")).toBe(false);
  });

  /**
   * **只有这两个吃「一张图 + 一句指令」。** 回落到别的引擎的表现是
   * 「图被整张重画」，比报错糟得多。
   */
  it("引擎只在能改图的那两个里选", () => {
    expect(localEditEngine(["zimage", "qwen21_edit"])).toBe("qwen21_edit");
    expect(localEditEngine(["krea2_edit"])).toBe("krea2_edit");
    // 一个都没有时给默认值，让 resolveCanvasEngine 去抛「不可用」——
    // 静默换成 zimage 才是最糟的
    expect(localEditEngine(["zimage"])).toBe(DEFAULT_LOCAL_EDIT_ENGINE);
    expect(LOCAL_EDIT_ENGINES).not.toContain("zimage");
  });

  /** 一张图 + 这个引擎，推导出来必须是 `ref2i`（指令式编辑那条路）。 */
  it("一张图配这个引擎推导出来就是指令式编辑", () => {
    expect(inferImageMode(1, DEFAULT_LOCAL_EDIT_ENGINE, canvasCatalogFixture).mode).toBe("ref2i");
  });
});

describe("接线", () => {
  const tools = readFileSync(new URL("./imageTools.ts", import.meta.url), "utf8");
  const bar = readFileSync(new URL("../ui/CanvasImageAssetToolbar.tsx", import.meta.url), "utf8");
  const node = readFileSync(new URL("../nodes/ImageGenNode.tsx", import.meta.url), "utf8");

  it("图片工具条上有入口", () => {
    expect(bar).toContain("spawnImageEdit(nodeId)");
  });

  it("新节点用会改图的引擎、预填模板", () => {
    expect(tools).toContain("engine: DEFAULT_LOCAL_EDIT_ENGINE");
    expect(tools).toContain("prompt: LOCAL_EDIT_TEMPLATE");
  });

  /**
   * **故意不自动跑。** 模板里还留着占位，照这个发出去会得到一张
   * 字面画着「要改的东西」的图 —— 白跑一次，而且看着像功能坏了。
   */
  it("局部修改不自动跑（别的工具才自动跑）", () => {
    const body = /export function spawnImageEdit[\s\S]*?\n}/.exec(tools)?.[0] ?? "";
    expect(body).not.toContain("executeSingleNode");
    expect(body).toContain("故意不跑");
  });

  it("记下源图，出完能并排比", () => {
    expect(tools).toContain("editSourcePath: nodeMedia(source)?.path");
    expect(node).toContain("<BeforeAfter before={data.editSourcePath}");
  });
});
