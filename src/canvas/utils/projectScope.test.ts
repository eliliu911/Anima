import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

/**
 * **画布上按工程分开的那几处。**
 *
 * 用户 2026-09-21：「不同工程下会混入其他工程的任务」。
 * 两处会混：Dock 栏的工作流目录、分镜节点的分集下拉。
 * 两处的**取舍不一样**，所以分开钉：
 *
 * | | 默认 | 为什么不一刀切 |
 * |---|---|---|
 * | 工作流 | 只列当前工程，别的工程收在一栏里 | 模板存在的理由就是换个项目还能跑，硬过滤等于砍掉一半 |
 * | 分集下拉 | 只列当前工程 | 但**已经挂着的那一集永远留着** —— 筛掉的话下拉变空，看起来像分镜丢了 |
 */
const read = (p: string) => readFileSync(new URL(p, import.meta.url), "utf8");

describe("工作流按工程分", () => {
  const panel = read("../ui/CanvasWorkflowPanel.tsx");

  it("默认只列当前工程的", () => {
    expect(panel).toContain("list.filter((w) => w.project_id === projectId)");
    expect(panel).toContain("{mine.map(");
  });

  /** 后端不筛、界面筛 —— 筛了的话没法说「别的工程里还有 N 套」。 */
  it("别的工程那一栏留着，但默认收起", () => {
    expect(panel).toContain("list.filter((w) => w.project_id !== projectId)");
    expect(panel).toContain("useState(false)");
    expect(panel).toContain("别的工程里还有");
  });

  it("存 / 读 / 删都带上工程", () => {
    expect(panel).toContain("canvasWorkflowSave(projectId,");
    expect(panel).toContain("canvasWorkflowLoad(owner, wf)");
    expect(panel).toContain("canvasWorkflowDelete(owner, wf)");
  });
});

describe("分集下拉按工程分", () => {
  const node = read("../nodes/ShotNode.tsx");

  /**
   * **钉整条过滤表达式，不是钉里面某个片段。**
   *
   * 第一版分开查 `e.project_id === data.projectId`，而那串在文件别处
   * （`projectTitle` 的查找）也有 —— 把它从过滤里删掉，测试照样绿。
   * 变异当场认了出来：**又一条假绿测试**。
   */
  it("只列当前工程的；没选工程时不筛；已经挂着的那一集永远留着", () => {
    expect(node).toContain(
      "(e) => !activeProjectId || e.project_id === activeProjectId || e.project_id === data.projectId,",
    );
  });

  it("被筛掉的说出来，不悄悄少几项", () => {
    expect(node).toContain("hiddenCount");
    expect(node).toContain("别的工程还有");
  });
});
