import { describe, expect, it } from "vitest";
import { byGroup, group } from "./shotLanguage";

describe("运镜按物理分组", () => {
  /**
   * **五类都要有词。** 少一类说明某种运动根本无词可用 ——
   * `lens`（纯镜头）那一类就是这么补上的：以前一条都没有，
   * 想要「机位不动、画面拉近」时只能错用推镜头。
   */
  it("五类都在，而且每条都归了类", () => {
    const g = group("move");
    expect(g?.groupNames, "运镜没有分组显示名").toBeTruthy();
    const labels = byGroup(g).map((s) => s.label);
    for (const want of ["平移（机器真的在动）", "旋转（原地转）", "纯镜头（机器不动）", "复合", "静止"]) {
      expect(labels, `少了「${want}」这一段`).toContain(want);
    }
    for (const t of g?.items ?? []) {
      expect(t.group, `${t.id} 没标分组`).toBeTruthy();
    }
  });

  /** 分段之后一条都不能少 —— 少了的那条在下拉框里就选不到了。 */
  it("分段不丢词", () => {
    const g = group("move");
    const flat = byGroup(g).flatMap((s) => s.items.map((t) => t.id));
    expect(flat).toEqual(g?.items.map((t) => t.id));
  });

  /** 不分组的词表（景别、机位）只出一段，`label` 为空 —— 不该冒出 optgroup。 */
  it("不分组的词表只出一段且没有标签", () => {
    for (const id of ["shotSize", "angle"]) {
      const sec = byGroup(group(id));
      expect(sec, `${id} 分成了多段`).toHaveLength(1);
      expect(sec[0].label, `${id} 冒出了分组标签`).toBe("");
    }
  });

  /**
   * **推镜头和变焦推近必须归到不同的类** —— 那正是这次分组要解决的混淆：
   * 一个是机器往前走（透视变），一个只是焦距变（透视不变）。
   *
   * **判据是 `group` 这个键，不是分到第几段。** 第一版写的是「不在同一段」，
   * 而 `byGroup` 只合并**相邻**的同名段 —— 新词排在词表末尾，
   * 把 `zoom_in` 错标成 `translate` 也会自成一段，断言照样绿。
   * 变异测试当场抓到了。
   */
  it("推镜头和变焦推近归在不同类", () => {
    const items = group("move")?.items ?? [];
    const at = (id: string) => items.find((t) => t.id === id);
    expect(at("push_in")?.group).toBe("translate");
    expect(at("zoom_in")?.group).toBe("lens");
  });

  it("传 undefined 不炸", () => {
    expect(byGroup(undefined)).toEqual([]);
  });
});
