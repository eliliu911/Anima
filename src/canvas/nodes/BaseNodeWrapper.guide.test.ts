// 卡片上的模板说明（guide）。服务端渲染读不到 zustand 的当前状态，所以 store 换成替身。
import { describe, it, expect, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

const nodes: { id: string; data: Record<string, unknown> }[] = [];
vi.mock("../store/useCanvasStore", () => ({
  useCanvasStore: (selector: (s: { nodes: typeof nodes }) => unknown) => selector({ nodes }),
}));

import { BaseNodeWrapper } from "./BaseNodeWrapper";

const STEP = "【第 3 步 / 共 7 步】点「生成」出教室首帧";
const card = (id: string, notes?: string[]) =>
  renderToStaticMarkup(createElement(BaseNodeWrapper, { id, notes, hideSourceHandle: true, children: createElement("div", null, "卡片内容") }));

describe("卡片上的模板说明", () => {
  it("显示 guide；和 guide 重复的运行提示只显示一次，其余运行提示照常显示在下面", () => {
    nodes.push({ id: "a", data: { guide: [STEP, "看图：窗户开着"] } });
    const html = card("a", [STEP, "用了 45 秒"]);
    expect(html.split(STEP).length - 1).toBe(1);
    expect(html).toContain("看图：窗户开着");
    expect(html).toContain("用了 45 秒");
    expect(html.indexOf(STEP)).toBeLessThan(html.indexOf("用了 45 秒"));
  });

  it("没有 guide 的节点和以前一样", () => {
    nodes.push({ id: "b", data: {} });
    const html = card("b", ["用了 45 秒"]);
    expect(html).toContain("用了 45 秒");
    expect(html).not.toContain("border-sky-400");
  });
});
