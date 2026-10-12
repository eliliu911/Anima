import { describe, expect, it } from "vitest";
import type { AssetView } from "../../api/assets";
import { inCurrentScope, orderAssets } from "./CanvasAssetLibrary";

const a = (id: string, scope: AssetView["scope"]) => ({ id, name: id, kind: "character", scope, scope_label: id } as unknown as AssetView);
const list = [
  a("ep", { kind: "episode", project_id: "p1", episode_id: "e1" }),
  a("glob", { kind: "global" }),
  a("other", { kind: "project", project_id: "p2" }),
  a("proj", { kind: "project", project_id: "p1" }),
];

// 用户 2026-09-25：顶部资产库自由画布只列全局 3 个、切到项目就「空的」，节点弹窗却列得出 49 个
describe("画布顶部资产库", () => {
  it("什么都不丢：所有库的都列出来", () => {
    expect(orderAssets(list, "p1")).toHaveLength(4);
    expect(orderAssets(list, null)).toHaveLength(4);
  });
  it("当前项目（含它的分集）排前面，其余保持原顺序", () => {
    expect(orderAssets(list, "p1").map((x) => x.id)).toEqual(["ep", "proj", "glob", "other"]);
  });
  it("自由画布：全局的排前面", () => {
    expect(orderAssets(list, null).map((x) => x.id)).toEqual(["glob", "ep", "other", "proj"]);
  });
  it("别的项目的不算当前库（要标出处）", () => {
    expect(inCurrentScope(list[2], "p1")).toBe(false);
    expect(inCurrentScope(list[1], "p1")).toBe(false);
  });
});
