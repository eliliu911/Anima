import { describe, it, expect } from "vitest";
import { panoramaPlan, PANO_LONG_SIDE } from "./panoramaPrompt";
import type { SceneContract } from "../../api/assets";

const 面馆: SceneContract = {
  shape: "东西进深的狭长一层店铺",
  zones: ["东侧卷帘门入口", "北侧柜台"],
  entrances: [{ name: "东侧卷帘门", connects_to: "街道" }],
  anchors: [
    { name: "北侧旧木柜台", zone: "北墙", size: "wall_dominant", must_not_become: ["现代吧台"] },
    { name: "南墙菜单板", zone: "南墙", size: "small", must_not_become: [] },
  ],
  relations: ["后厨出餐口在柜台正后方"],
  materials: ["磨损木柜台"],
  views: [
    { name: "入口正打", from: "卷帘门内侧", toward: "店堂深处", far: "旧木柜台", visible: [], light: "" },
    { name: "柜台反打", from: "柜台内侧", toward: "卷帘门", far: "门外的街", visible: [], light: "" },
  ],
  inferred: [],
  no_extra: ["第二扇门"],
  empty_stage: "required",
};

describe("360° 全景提示词", () => {
  /**
   * **场景名一个字都不发。**
   *
   * 宫格那条路实测过：三道禁令压不住，四格里两格把名字当招牌画上了门楣。
   * 删掉名字才有用。全景这条当初漏了，实跑前才发现 —— 同一个坑，
   * 修了一处没修另一处。
   */
  it("场景名不进提示词", () => {
    const p = panoramaPlan(面馆, "安和面馆");
    expect(p.prompt).not.toContain("安和面馆");
    // 形状照写 —— 场景长什么样全在这儿
    expect(p.prompt).toContain("东西进深的狭长一层店铺");
  });

  it("等距圆柱投影固定 2:1", () => {
    const p = panoramaPlan(面馆, "安和面馆");
    expect(p.width).toBe(PANO_LONG_SIDE);
    expect(p.height).toBe(PANO_LONG_SIDE / 2);
    const big = panoramaPlan(面馆, "安和面馆", 4096);
    expect(big.width / big.height).toBe(2);
  });

  /** 位置写成百分比，**不写坐标** —— 模型守不住坐标，守得住「四分之一处」。 */
  it("视角按契约顺序均分一圈", () => {
    const p = panoramaPlan(面馆, "安和面馆");
    expect(p.prompt).toContain("x=0% 是入口正打");
    expect(p.prompt).toContain("x=50% 是柜台反打");
    expect(p.prompt).not.toMatch(/\d+\s*px|\bbbox\b/);
  });

  /** 重复是全景最典型的失败样子，正向描述压不住，必须明说。 */
  it("明说同一件东西不许出现两次", () => {
    const p = panoramaPlan(面馆, "安和面馆");
    expect(p.prompt).toContain("只出现一次");
    expect(p.prompt).toContain("不要因为要填满一圈就复制");
  });

  it("带上地理锁和负向清单", () => {
    const p = panoramaPlan(面馆, "安和面馆");
    expect(p.prompt).toContain("地理锁：北侧旧木柜台");
    expect(p.prompt).toContain("不要出现**：第二扇门");
  });

  /**
   * **一个方向撑不起 360°。** 剩下四分之三只能由模型自己想 ——
   * 出得来、也好看，但它不是这个场景。
   */
  it("视角不够要告警", () => {
    const one = panoramaPlan({ ...面馆, views: [面馆.views[0]] }, "安和面馆");
    expect(one.warnings.join()).toContain("四分之三是编的");
    const none = panoramaPlan({ ...面馆, views: [] }, "安和面馆");
    expect(none.warnings.join()).toContain("整张全景都是编的");
    // 告警不阻断 —— 提示词照出
    expect(none.prompt).toContain("equirectangular");
  });

  it("推断项要单独提醒别当准", () => {
    const p = panoramaPlan({ ...面馆, inferred: ["天花板管线"] }, "安和面馆");
    expect(p.warnings.join()).toContain("天花板管线");
  });

  /** 没契约时**不给提示词** —— 给一句通用全景描述会让人以为它是这个场景。 */
  it("没有契约就不出提示词", () => {
    const p = panoramaPlan(null, "安和面馆");
    expect(p.prompt).toBe("");
    expect(p.warnings.join()).toContain("还没有空间契约");
    // 画幅照给，接上去也不会出成方图
    expect(p.width / p.height).toBe(2);
  });

  it("两个视角时不告警", () => {
    expect(panoramaPlan(面馆, "安和面馆").warnings).toEqual([]);
  });
});
