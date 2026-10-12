import { describe, it, expect } from "vitest";
import {
  remapReferenceMentions,
  compilePromptMentions,
  sameOrder,
} from "./referenceMentions";

describe("referenceMentions", () => {
  describe("sameOrder", () => {
    it("returns true for identical arrays", () => {
      expect(sameOrder(["a", "b"], ["a", "b"])).toBe(true);
      expect(sameOrder([], [])).toBe(true);
    });

    it("returns false for different arrays", () => {
      expect(sameOrder(["a", "b"], ["b", "a"])).toBe(false);
      expect(sameOrder(["a"], ["a", "b"])).toBe(false);
    });
  });

  describe("remapReferenceMentions", () => {
    it("remaps @图片2 to @图片1 when previous image 1 is removed", () => {
      const out = remapReferenceMentions("让 @图片2 动起来", [
        { prefix: "图片", prevIds: ["A", "B"], nextIds: ["B"] },
      ]);
      expect(out).toBe("让 @图片1 动起来");
    });

    it("removes deleted mention with trailing space", () => {
      const out = remapReferenceMentions("先 @图片1 再 @图片2 收尾", [
        { prefix: "图片", prevIds: ["A", "B"], nextIds: ["B"] },
      ]);
      expect(out).toBe("先 再 @图片1 收尾");
    });

    it("remaps on reorder", () => {
      const out = remapReferenceMentions("@图片1 和 @图片2", [
        { prefix: "图片", prevIds: ["A", "B"], nextIds: ["B", "A"] },
      ]);
      expect(out).toBe("@图片2 和 @图片1");
    });

    it("remaps character names when renamed", () => {
      const out = remapReferenceMentions(
        "让 @筱筱 看着 @夏禾",
        [],
        [{ prevNames: ["筱筱", "夏禾"], nextNames: ["小雅", "夏禾"] }],
      );
      expect(out).toBe("让 @小雅 看着 @夏禾");
    });

    it("preserves unrelated mentions", () => {
      const out = remapReferenceMentions("@图片9 保留", [
        { prefix: "图片", prevIds: ["A", "B"], nextIds: ["A"] },
      ]);
      expect(out).toBe("@图片9 保留");
    });
  });

  describe("compilePromptMentions", () => {
    it("translates @图片1 to <Picture 1> for H3", () => {
      const out = compilePromptMentions("让 @图片1 与 @图片2 拥抱", [], "h3");
      expect(out).toBe("让 <Picture 1> 与 <Picture 2> 拥抱");
    });

    it("handles space between prefix and number like @图片 1", () => {
      const out = compilePromptMentions("聚焦 @图片 1 的神态", [], "h3");
      expect(out).toBe("聚焦 <Picture 1> 的神态");
    });

    it("maps character name mention @筱筱 and @夏禾 to corresponding picture tags", () => {
      const refs = [
        { name: "筱筱", path: "/img1.png" },
        { name: "夏禾", path: "/img2.png" },
      ];
      const out = compilePromptMentions("@筱筱 看着 @夏禾 笑了", refs, "h3");
      expect(out).toBe("<Picture 1> 看着 <Picture 2> 笑了");
    });

    it("maps character names even without spaces in Chinese sentence", () => {
      const refs = [
        { name: "筱筱", path: "/img1.png" },
        { name: "夏禾", path: "/img2.png" },
      ];
      const out = compilePromptMentions("让@筱筱和@夏禾拥抱在一起", refs, "h3");
      expect(out).toBe("让<Picture 1>和<Picture 2>拥抱在一起");
    });
  });
});

describe("Picture 编号的唯一真相", () => {
  it("按下发顺序编号，不按 data.references 的下标", () => {
    // 实际下发出去的数组（已去重、已沿图追加）
    const sent = [
      { path: "/a.png", name: "筱筱" },
      { path: "/b.png", name: "夏禾" },
    ];
    expect(compilePromptMentions("@夏禾 看向 @筱筱", sent, "h3")).toBe(
      "<Picture 2> 看向 <Picture 1>",
    );
  });

  it("同一张图被引用两次时不占两个槽位，后面的编号不前移", () => {
    // 用户把同一张图连了两次：去重后只有 2 张，@夏禾 必须是 <Picture 2> 而不是 <Picture 3>
    const sent = [
      { path: "/a.png", name: "筱筱" },
      { path: "/b.png", name: "夏禾" },
    ];
    const out = compilePromptMentions("@筱筱 和 @夏禾 对视", sent, "h3");
    expect(out).toBe("<Picture 1> 和 <Picture 2> 对视");
    expect(out).not.toContain("<Picture 3>");
  });

  it("非 H3 链路没有 Picture 槽位，摘掉 @ 保留名字", () => {
    // i2v / t2v / fl2v：原来完全不编译，字面量 @筱筱 直接发给模型
    const refs = [{ path: "/a.png", name: "筱筱" }];
    expect(compilePromptMentions("@筱筱 转身走向窗边", refs, "ltx")).toBe(
      "筱筱 转身走向窗边",
    );
  });

  it("没对上任何参考的 @名字 也不能当字面量发出去", () => {
    expect(compilePromptMentions("@未知角色 站在门口", [], "ltx")).toBe(
      "未知角色 站在门口",
    );
  });
});
