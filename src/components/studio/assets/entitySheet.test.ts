import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import {
  ENTITY_TYPES,
  ENTITY_TYPE_LABELS,
  QUAD_ZONES,
  REF_BOOST_DEFAULT,
  REF_BOOST_MAX,
  SHEET_ZONES,
  guessEntityType,
  locksFromDescription,
  refBoostHint,
  sheetBlocker,
} from "./entitySheet";

const read = (p: string) => readFileSync(new URL(`../../../../${p}`, import.meta.url), "utf8");

describe("实体类型", () => {
  /**
   * **两边是同一份名单，分叉不报错。**
   *
   * Rust 侧 `normalize_entity_type` 认不出的会回落成 `STYLIZED_CHARACTER`，
   * 所以前端多一个词的表现是「选了 A、发出去变成 B」—— 界面上看不出来。
   */
  it("和 Rust 侧的名单逐字一致", () => {
    const rust = read("src-tauri/src/agent_runtime/entity_sheet.rs");
    const block = rust.slice(
      rust.indexOf("pub const ENTITY_TYPES"),
      rust.indexOf("];", rust.indexOf("pub const ENTITY_TYPES")),
    );
    for (const t of ENTITY_TYPES) {
      expect(block, `${t} 不在 Rust 那份名单里`).toContain(`"${t}"`);
    }
    // 反向也要查：Rust 有而这里没有的话，那个类型永远选不到
    const inRust = [...block.matchAll(/"([A-Z_]+)"/g)].map((m) => m[1]);
    expect(inRust.sort()).toEqual([...ENTITY_TYPES].sort());
  });

  it("每个类型都有中文名 —— 下拉里不能出现英文枚举", () => {
    for (const t of ENTITY_TYPES) {
      expect(ENTITY_TYPE_LABELS[t], `${t} 没有中文名`).toBeTruthy();
    }
  });

  /** 猜错了不要紧（用户能改），但**不能猜出名单外的值**。 */
  it("猜出来的一定在名单里", () => {
    const cases: [string, string][] = [
      ["character", "三个月大的小三花猫，毛茸茸"],
      ["character", "一个机械改造的赏金猎人"],
      ["character", "深海里的触手怪物"],
      ["character", "穿制服的高中女生"],
      ["prop", "普通棕色瓦楞纸搬家箱"],
      ["prop", "一辆生锈的皮卡车"],
      ["scene", "陌生的房间"],
      ["character", ""],
      ["", ""],
    ];
    for (const [kind, desc] of cases) {
      const t = guessEntityType(kind, desc);
      expect(ENTITY_TYPES as readonly string[], `${kind}/${desc} 猜出了 ${t}`).toContain(t);
    }
  });

  it("常见几类猜得对", () => {
    expect(guessEntityType("character", "三个月大的小三花猫")).toBe("ANIMAL");
    expect(guessEntityType("character", "机械改造的赏金猎人")).toBe("ROBOT");
    expect(guessEntityType("character", "深海触手怪物")).toBe("CREATURE");
    expect(guessEntityType("character", "穿制服的高中女生")).toBe("STYLIZED_CHARACTER");
    expect(guessEntityType("prop", "瓦楞纸箱")).toBe("OBJECT");
    expect(guessEntityType("prop", "生锈的皮卡车")).toBe("VEHICLE");
  });
});

describe("身份锚点", () => {
  /**
   * 描述里的「无」是资产表单没填时留下的占位，**不能进提示词** ——
   * 那会让模型去画一个「无」。`clean_description` 在 Rust 侧治的是同一件事。
   */
  it("丢掉没有信息量的段", () => {
    const locks = locksFromDescription("橘色短发，眼神凶狠，无，未知，穿深色制服");
    expect(locks).toEqual(["橘色短发", "眼神凶狠", "穿深色制服"]);
    expect(locks).not.toContain("无");
  });

  it("中英文标点都能切", () => {
    expect(locksFromDescription("a, b、c；d\ne")).toEqual(["a", "b", "c", "d", "e"]);
  });

  /** 作者模板写的是 4~7 条，多了会开始互相矛盾。 */
  it("最多八条", () => {
    const many = Array.from({ length: 20 }, (_, i) => `锚点${i}`).join("，");
    expect(locksFromDescription(many)).toHaveLength(8);
  });

  it("空描述给空数组而不是一条空串", () => {
    expect(locksFromDescription("")).toEqual([]);
    expect(locksFromDescription("，，、；")).toEqual([]);
  });
});

describe("能不能出", () => {
  /**
   * **没图必须挡住，而且说清怎么办。**
   *
   * 它是图生图 —— 没源图的话下游会报「找不到源图」，
   * 而用户在工作台里看不出那是因为自己还没出过图。
   */
  it("没图时挡住并给出下一步", () => {
    const why = sheetBlocker(false, true);
    expect(why).toBeTruthy();
    expect(why).toContain("先得有一张封面图");
  });

  it("引擎不在时也挡住，且说的是引擎", () => {
    const why = sheetBlocker(true, false);
    expect(why).toBeTruthy();
    expect(why).toContain("Krea 2");
  });

  it("都齐了就返回 null", () => {
    expect(sheetBlocker(true, true)).toBeNull();
  });
});

describe("保真度旋钮", () => {
  it("默认值是实测选出来的那个，且不超上限", () => {
    expect(REF_BOOST_DEFAULT).toBe(4);
    expect(REF_BOOST_DEFAULT).toBeLessThanOrEqual(REF_BOOST_MAX);
  });

  /** 和 Rust 侧的上限必须一致 —— 前端放行 20 而后端夹到 10，滑杆就骗人了。 */
  it("上限和 Rust 侧一致", () => {
    const rust = read("src-tauri/src/agent_runtime/entity_sheet.rs");
    expect(rust).toContain(`pub const REF_BOOST_MAX: f64 = ${REF_BOOST_MAX}.0;`);
    expect(rust).toContain(`pub const REF_BOOST_DEFAULT: f64 = ${REF_BOOST_DEFAULT}.0;`);
  });

  /**
   * **说结果，不说形容词。** 「强/弱」帮不了人决定要不要拉；
   * 「动作三姿会真的不同」「换装类要求会开始失败」才能。
   */
  it("每个档位都说得出会发生什么", () => {
    for (const v of [0, 0.5, 1, 2, 4, 6, 8, 10]) {
      const h = refBoostHint(v);
      expect(h.length, `${v} 的说明太短`).toBeGreaterThan(6);
    }
    expect(refBoostHint(0)).toContain("漂");
    expect(refBoostHint(4)).toContain("动作");
    expect(refBoostHint(10)).toContain("失败");
    // 相邻档位不能给同一句话 —— 那样拉滑杆没有反馈
    expect(refBoostHint(1)).not.toBe(refBoostHint(4));
    expect(refBoostHint(4)).not.toBe(refBoostHint(10));
  });
});

describe("七个区的说明", () => {
  /**
   * **把短板写在界面上。** 这条路要等 7 分半，等完了才发现「转身那排脸太小」
   * 或者「元数据是涂鸦」的话，那 7 分半是白等的。
   */
  it("把实测的两个短板明说了", () => {
    const all = SHEET_ZONES.join(" ");
    expect(all).toContain("110px");
    expect(all).toContain("涂鸦");
    expect(SHEET_ZONES).toHaveLength(7);
  });
});

describe("四视图和七分区的分工", () => {
  /**
   * **两组说明不能长一样。** 它们的差别是「身份参考」vs「取材」——
   * 说不清的话，用户会随便点一个，然后拿转身只有 110px 的那张去喂 R2V。
   */
  it("四视图强调脸大、七分区强调短板", () => {
    const q = QUAD_ZONES.join(" ");
    const s = SHEET_ZONES.join(" ");
    expect(q).not.toBe(s);
    // 四视图：把「特写最大」说出来，那是它存在的理由
    expect(q).toContain("486px");
    expect(QUAD_ZONES).toHaveLength(4);
    // 七分区：把「转身当不了身份参考」说出来
    expect(s).toContain("110px");
    expect(s).toContain("别当身份参考");
  });
});
