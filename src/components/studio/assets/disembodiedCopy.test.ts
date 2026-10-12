import { describe, expect, it } from "vitest";
import { DISEMBODIED_HINT, DISEMBODIED_LABEL, DISEMBODIED_NOTE, showImageUi } from "./disembodiedCopy";

describe("只有声音的角色", () => {
  /**
   * 用户 2026-09-18 的原话：「这个角色有可能出镜，有可能旁白，
   * 你说你是勾选还不勾选？」—— 歧义全在「是不是这一次」上。
   */
  it("**标题必须说「从不」** —— 少了这两个字就读成逐次的选择", () => {
    expect(DISEMBODIED_LABEL).toContain("从不");
  });

  it("**说明必须带反例**「偶尔旁白不要勾」—— 只举正例的话「旁白」这个词两边都沾", () => {
    expect(DISEMBODIED_HINT).toContain("偶尔");
    expect(DISEMBODIED_HINT).toContain("不要勾");
    expect(DISEMBODIED_HINT).toContain("逐句");
  });

  it("说明要讲清勾上之后会发生什么，不只是它是什么", () => {
    expect(DISEMBODIED_HINT).toContain("不给他出图");
    expect(DISEMBODIED_HINT).toContain("不单独给他镜头");
  });

  it("**勾了「没有身体」就不再问怎么画他的身体** —— 原来两者并排显示，正是困惑的来源", () => {
    expect(showImageUi("character", true)).toBe(false);
    expect(showImageUi("character", false)).toBe(true);
    // 场景 / 道具没有这一位，永远显示
    expect(showImageUi("scene", true)).toBe(true);
    expect(showImageUi("prop", false)).toBe(true);
  });

  it("出图界面消失时要说怎么退回去，不能只是没了", () => {
    expect(DISEMBODIED_NOTE).toContain("取消");
  });
});
