import { beforeEach, describe, expect, it } from "vitest";
import type { AssetView } from "../../api/assets";
import { groupAssets, KIND_ORDER, loadViewMode, saveViewMode } from "./assetGroups";

const a = (name: string, kind: AssetView["kind"]) => ({ id: name, name, kind }) as AssetView;

describe("资产按类分组", () => {
  it("顺序固定：角色 → 场景 → 道具", () => {
    const g = groupAssets([a("纸箱", "prop"), a("小花", "character"), a("房间", "scene")]);
    expect(g.map((x) => x.kind)).toEqual(KIND_ORDER);
    expect(g.map((x) => x.label)).toEqual(["角色", "场景", "道具"]);
  });

  /** 留一个「场景 0」的空标题，会让人以为列表加载坏了。 */
  it("空的类不出现", () => {
    const g = groupAssets([a("小花", "character")]);
    expect(g).toHaveLength(1);
    expect(g[0].items.map((x) => x.name)).toEqual(["小花"]);
  });

  it("一个都没有时返回空数组，由调用方说一句完整的话", () => {
    expect(groupAssets([])).toEqual([]);
  });

  it("同类里保持传入顺序（近的在前）", () => {
    const g = groupAssets([a("本集的", "character"), a("项目的", "character")]);
    expect(g[0].items.map((x) => x.name)).toEqual(["本集的", "项目的"]);
  });
});

/**
 * **这套测试跑在 node 环境里，没有 `localStorage`。**
 * 塞一个内存版进去 —— 不塞的话「存过就记住」永远读到默认值，
 * 那条测试等于在测「函数是不是总返回 grid」。
 */
function stubStorage() {
  const map = new Map<string, string>();
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: {
      getItem: (k: string) => map.get(k) ?? null,
      setItem: (k: string, v: string) => void map.set(k, v),
      removeItem: (k: string) => void map.delete(k),
    },
  });
}

describe("平铺 / 列表的偏好", () => {
  beforeEach(stubStorage);

  it("默认平铺", () => {
    expect(loadViewMode()).toBe("grid");
  });

  it("存过 list 就记住", () => {
    saveViewMode("list");
    expect(loadViewMode()).toBe("list");
    saveViewMode("grid");
    expect(loadViewMode()).toBe("grid");
  });

  /**
   * **读写都要能扛住抛异常。** 隐私窗口、清过站点数据、
   * 或者某些环境下访问 `localStorage` 本身就会抛 ——
   * 一个记偏好的功能不该让整个面板打不开。
   */
  it("localStorage 抛异常时不炸，回到默认", () => {
    Object.defineProperty(globalThis, "localStorage", {
      configurable: true,
      value: {
        getItem: () => {
          throw new Error("blocked");
        },
        setItem: () => {
          throw new Error("blocked");
        },
      },
    });
    expect(loadViewMode()).toBe("grid");
    expect(() => saveViewMode("list")).not.toThrow();
  });

  /** 连 `localStorage` 这个全局都不存在时（SSR / 某些沙箱）也不能炸。 */
  it("完全没有 localStorage 时也不炸", () => {
    Object.defineProperty(globalThis, "localStorage", { configurable: true, value: undefined });
    expect(loadViewMode()).toBe("grid");
    expect(() => saveViewMode("list")).not.toThrow();
  });
});
