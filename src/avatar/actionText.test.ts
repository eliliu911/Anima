import { describe, expect, it } from "vitest";
import { stripActionTags } from "./actionText";

describe("气泡里的动作标签", () => {
  it("成对标签整个剥掉", () => {
    expect(stripActionTags("<action:heart>好啊，给你一个爱心喔💖</action:heart>")).toBe(
      "好啊，给你一个爱心喔💖",
    );
  });

  it("单个开标签也剥", () => {
    expect(stripActionTags("你好呀<action:wave>")).toBe("你好呀");
  });

  /**
   * 流式输出时标签是一个字一个字蹦出来的。剥不掉的话用户会看着
   * `<acti`、`<action:hea` 一路长出来 —— 比直接显示完整标签还难看
   */
  it("还没写完的半截标签也藏起来", () => {
    expect(stripActionTags("说点什么<action:hea")).toBe("说点什么");
    expect(stripActionTags("说点什么</acti")).toBe("说点什么");
  });

  it("没有标签的文本原样返回（同一个引用，不白做一遍替换）", () => {
    const plain = "普通的一句话 😊";
    expect(stripActionTags(plain)).toBe(plain);
  });

  it("模型编出来的 HTML 也剥掉", () => {
    expect(stripActionTags("<span></span> 好啊，伸个懒腰～<span></span> 这颗心给你💖")).toBe(
      " 好啊，伸个懒腰～ 这颗心给你💖",
    );
  });

  it("不会误伤别的尖括号", () => {
    expect(stripActionTags("1 < 2 而且 3 > 2")).toBe("1 < 2 而且 3 > 2");
  });
});
