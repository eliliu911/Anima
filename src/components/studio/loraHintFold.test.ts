import { describe, expect, it } from "vitest";
import { FOLD_OVER, foldSummary, shouldFold } from "./loraHintFold";

const short = "多视图版式 —— 设定卡的核心，工具会自动挂";
const long =
  "Krea 2·服装｜职业通勤装 —— 实测出的是黑色及膝铅笔裙 + 米色真丝衬衫 + 黑丝袜 + 高跟鞋 + 手提包，全身构图。" +
  "触发词 bdq，不写等于没挂。别被「包臀裙」这个名字误导成性化用途，实测是 OL 风。";

describe("要不要折", () => {
  /**
   * **短说明不折。** 折起来等于多一次点击换零收益，而且展开后和折叠时
   * 长得一样，会让人以为点了没反应。
   */
  it("短的不折，长的折", () => {
    expect(shouldFold(short)).toBe(false);
    expect(shouldFold(long)).toBe(true);
  });

  it("空白不算长度", () => {
    expect(shouldFold(`   ${"字".repeat(FOLD_OVER)}   `)).toBe(false);
  });
});

describe("折叠时的摘要", () => {
  /**
   * **按句子边界截，不按字数硬切。** 硬切会把句子斩在半路，
   * 读起来像渲染出错了。
   */
  it("第一句就超长时，断在词组边界而不是词中间", () => {
    const s = foldSummary(long);
    expect(s.length).toBeLessThan(long.length);
    expect(s).toContain("职业通勤装");
    // 「高跟鞋 + 手提包」不能被斩成「高跟鞋 + 手」
    expect(s).not.toMatch(/\+ .…$/);
    expect(s.endsWith("…")).toBe(true);
  });

  /** 第一句在上限之内时就该断在句号，摘要自成一句。 */
  it("第一句够短时断在句号", () => {
    // 用例必须真的够长才会折 —— 短于 FOLD_OVER 的会原样返回，测不到分支
    const src = "实测：下巴变尖、颧骨收窄，五官更精致，肤质保持写实不磨皮，光也更柔。触发词 asian woman，作者推荐强度 0.8，比默认 1.0 低。";
    expect(src.length).toBeGreaterThan(FOLD_OVER);
    expect(foldSummary(src)).toBe("实测：下巴变尖、颧骨收窄，五官更精致，肤质保持写实不磨皮，光也更柔。");
  });

  /** 短的原样返回 —— 不加省略号，那会暗示还有下文。 */
  it("短的原样返回且不加省略号", () => {
    expect(foldSummary(short)).toBe(short);
    expect(foldSummary(short)).not.toContain("…");
  });

  /**
   * 上限之内**没有**句子边界时才硬切，而且要加省略号明说被截断了。
   * 不加的话用户会以为那就是全文。
   */
  it("找不到句子边界才硬切，并加省略号", () => {
    const nosep = "字".repeat(200);
    const s = foldSummary(nosep);
    expect(s.endsWith("…")).toBe(true);
    expect(s.length).toBeLessThanOrEqual(FOLD_OVER + 1);
  });

  /** 句子边界太靠前时宁可硬切 —— 否则摘要只剩两三个字，等于没有。 */
  it("句子边界太靠前时不用它", () => {
    const s = foldSummary("短。" + "字".repeat(200));
    expect(s).not.toBe("短。");
  });
});
