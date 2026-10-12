import { describe, it, expect } from "vitest";
import { inferCommitTarget, roleAllows, rolesFor, ROLE_LABEL, type VariantRole } from "./commitTarget";
import type { CanvasNode, CanvasEdge } from "../types";

const node = (id: string, type: string, data: Record<string, unknown> = {}): CanvasNode =>
  ({ id, type, position: { x: 0, y: 0 }, data } as unknown as CanvasNode);
const edge = (source: string, target: string): CanvasEdge =>
  ({ id: `${source}-${target}`, source, target } as CanvasEdge);

describe("提交落点的角色约束", () => {
  it("和 Rust 侧一致：角色的只能给角色，场景的只能给场景", () => {
    expect(roleAllows("portrait", "character")).toBe(true);
    expect(roleAllows("portrait", "scene")).toBe(false);
    expect(roleAllows("scene_reverse", "scene")).toBe(true);
    expect(roleAllows("scene_reverse", "character")).toBe(false);
  });

  it("通用参考图哪类资产都能放", () => {
    for (const k of ["character", "scene", "prop"] as const) {
      expect(roleAllows("reference", k)).toBe(true);
    }
  });

  /** 界面上列出来又拒绝是最差的体验。 */
  it("rolesFor 只列这类资产真能用的", () => {
    expect(rolesFor("character")).toEqual(["portrait", "turnaround", "reference"]);
    expect(rolesFor("scene")).toEqual(["scene_master", "scene_reverse", "scene_pano", "reference"]);
    expect(rolesFor("prop")).toEqual(["reference"]);
    for (const k of ["character", "scene", "prop"] as const) {
      for (const r of rolesFor(k)) expect(roleAllows(r, k)).toBe(true);
    }
  });

  it("每个角色都有中文名", () => {
    const all = Object.keys(ROLE_LABEL) as VariantRole[];
    expect(all.every((r) => ROLE_LABEL[r].length > 0)).toBe(true);
  });
});

describe("从来源推断提交落点", () => {
  it("设定卡版式直接判成三视图 —— 这条最硬，没有别的可能", () => {
    const n = [node("a", "imageGen", { layout: "character_sheet" })];
    expect(inferCommitTarget("a", n, [])).toEqual({ kind: "character", role: "turnaround" });
    const n2 = [node("a", "imageGen", { layout: "entity_sheet" })];
    expect(inferCommitTarget("a", n2, [])).toEqual({ kind: "character", role: "turnaround" });
  });

  it("上游连着角色节点就判成角色定妆", () => {
    const nodes = [node("c", "character", { name: "沈星晴" }), node("g", "imageGen")];
    expect(inferCommitTarget("g", nodes, [edge("c", "g")]))
      .toEqual({ kind: "character", role: "portrait" });
  });

  it("版式压过提示词关键词 —— 版式是硬证据，词是软证据", () => {
    const n = [node("a", "imageGen", { layout: "character_sheet", prompt: "客厅的反打镜头" })];
    expect(inferCommitTarget("a", n, [])).toEqual({ kind: "character", role: "turnaround" });
  });

  it("提示词关键词认得出场景的三种", () => {
    const mk = (prompt: string) => inferCommitTarget("a", [node("a", "imageGen", { prompt })], []);
    expect(mk("客厅的反打").role).toBe("scene_reverse");
    expect(mk("2:1 全景图").role).toBe("scene_pano");
    expect(mk("客厅主镜，正打").role).toBe("scene_master");
    expect(mk("角色三视图").kind).toBe("character");
  });

  /**
   * **猜错比不猜更糟。** 标错了下游会当真；标成通用参考图至少是诚实的。
   */
  it("看不出来时回落到通用参考图，不乱猜一个具体角色", () => {
    const n = [node("a", "imageGen", { prompt: "一只橘猫趴在窗台上" })];
    expect(inferCommitTarget("a", n, []).role).toBe("reference");
  });

  it("节点不存在时不崩", () => {
    expect(inferCommitTarget("nope", [], []).role).toBe("reference");
  });
});
