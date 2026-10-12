import { describe, expect, it } from "vitest";
import { countByKind, filterKind, KIND_TABS } from "./assetTabs";
import type { AssetView } from "../../../api/assets";

const a = (id: string, kind: string): AssetView =>
  ({ id, kind, name: id, description: "", scope_label: "本集" }) as AssetView;

const LIST = [a("小花", "character"), a("小雨", "character"), a("房间", "scene"), a("纸箱", "prop")];

describe("资产页签", () => {
  it("四个页签：全部 / 角色 / 场景 / 道具", () => {
    expect(KIND_TABS.map((t) => t.key)).toEqual(["all", "character", "scene", "prop"]);
  });

  it("全部原样返回，一个不少", () => {
    expect(filterKind(LIST, "all")).toHaveLength(4);
  });

  it("按类型筛得干净", () => {
    expect(filterKind(LIST, "character").map((x) => x.id)).toEqual(["小花", "小雨"]);
    expect(filterKind(LIST, "prop").map((x) => x.id)).toEqual(["纸箱"]);
  });

  /**
   * 三类之和必须等于总数。对不上意味着有资产的 kind 落在三类之外 ——
   * 那种资产会**在任何一个页签里都看不到**，而且不报错。
   */
  it("三类之和等于总数 —— 不会有资产落在页签之外", () => {
    const c = countByKind(LIST);
    expect(c.character + c.scene + c.prop).toBe(c.all);
  });

  it("空列表全是零", () => {
    expect(countByKind([])).toEqual({ all: 0, character: 0, scene: 0, prop: 0 });
  });

  /** 认不出的类型进不了任何页签，但仍然算进「全部」—— 数字要诚实。 */
  it("认不出的类型算进全部，但不算进任何一类", () => {
    const c = countByKind([...LIST, a("怪东西", "unknown")]);
    expect(c.all).toBe(5);
    expect(c.character + c.scene + c.prop).toBe(4);
  });
});
