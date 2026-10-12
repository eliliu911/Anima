import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";

const read = (p: string) => readFileSync(new URL(p, import.meta.url), "utf8");

/** 用户 2026-09-24 对照 LibTV 要的：选中节点时下面的设定面板不跟画布缩放 */
describe("节点下方的设定面板挂在 NodeToolbar 上（不跟画布缩放）", () => {
  it.each(["../nodes/VideoGenNode.tsx", "../nodes/ImageGenNode.tsx"])("%s 的面板包在 NodePanelDock 里", (p) => {
    const src = read(p);
    expect(src).toContain("<NodePanelDock visible={Boolean(selected)}>");
    // 以前是 `{selected && (<div …><XxxOperationPanel` 直接画在节点里 —— 跟着缩放、还把节点撑高
    expect(src).not.toMatch(/\{selected && \(\s*<div className="animate-in[^"]*">\s*<(Video|Image)OperationPanel/);
  });

  it("NodePanelDock 用的是 React Flow 的 NodeToolbar，挂在节点下方", () => {
    const src = read("./NodePanelDock.tsx");
    expect(src).toContain("<NodeToolbar");
    expect(src).toContain("position={Position.Bottom}");
    expect(src).toContain("nowheel");
  });

  it("面板不再用负边距去对齐 440 宽的节点（NodeToolbar 自己居中）", () => {
    expect(read("./OperationPanelShell.tsx")).not.toMatch(/-ml-\[/);
    expect(read("../nodes/panels/VideoOperationPanel.tsx")).not.toMatch(/widthClass="[^"]*-ml-\[/);
  });
});
