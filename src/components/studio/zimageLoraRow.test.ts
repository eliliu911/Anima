import { describe, expect, it } from "vitest";
import { isMissing, optionLabel, rowOptions } from "./zimageLoraRow";

const AVAIL = [
  "zimage/360viewer_Redmond_zit.safetensors",
  "zimage/Pintrest_ZiT.safetensors",
];

describe("选项标签", () => {
  /**
   * **说明不能拼进选项**。说明现在带实测数据和出处，拼进去会把 `<select>` 撑爆，
   * 而撑爆之后 `flex-1` 会把强度框和「移除」按钮挤出可视区 —— 用户报的
   * 「删不了」就是这么来的。
   */
  it("只给名字，不拼说明", () => {
    const l = optionLabel("zimage/Pintrest_ZiT.safetensors");
    expect(l).toBe("zimage/Pintrest_ZiT.safetensors");
    expect(l).not.toContain("—");
    expect(l.length).toBeLessThan(60);
  });
});

describe("配置指向的文件被删掉之后", () => {
  /**
   * **这条最要紧。** `<select value>` 对不上任何 `<option>` 时，浏览器会
   * 静默显示第一个选项 —— 界面说常挂的是 A，配置里其实是已经不存在的 B，
   * 而用户完全看不出来。必须补一条选项把真相显示出来。
   */
  it("补一条「已不存在」并排在最前", () => {
    const gone = "zimage/Straight_hair_zimgt.safetensors";
    const opts = rowOptions(AVAIL, gone);
    expect(opts[0].value).toBe(gone);
    expect(opts[0].missing).toBe(true);
    expect(opts[0].label).toContain("已不存在");
    // 原有的选项一个不少
    expect(opts.slice(1).map((o) => o.value)).toEqual(AVAIL);
  });

  it("当前值还在时不补，也不改顺序", () => {
    const opts = rowOptions(AVAIL, AVAIL[1]);
    expect(opts.map((o) => o.value)).toEqual(AVAIL);
    expect(opts.some((o) => o.missing)).toBe(false);
  });

  it("isMissing 和补选项这两件事口径一致", () => {
    expect(isMissing(AVAIL, "zimage/没了.safetensors")).toBe(true);
    expect(isMissing(AVAIL, AVAIL[0])).toBe(false);
    // 空值不算缺失 —— 那是「还没选」，不是「选了个没有的」
    expect(isMissing(AVAIL, "")).toBe(false);
    expect(rowOptions(AVAIL, "").some((o) => o.missing)).toBe(false);
  });
});
