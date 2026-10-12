import { describe, expect, it } from "vitest";

import { contentRendersImage, getTurnImageCalls } from "./ChatHistory";

/** 造一次 generate_image 的工具调用 */
function call(id: string, prompt: string) {
  return { id, name: "generate_image", arguments: JSON.stringify({ prompt, width: 1024, height: 1024 }) };
}
/** 造一条带出图路径的工具结果 */
function result(callId: string, path: string) {
  return { callId, name: "generate_image", content: `![](file://${path})`, ok: true };
}

describe("生成图卡片", () => {
  it("重试同一个 prompt 不再叠加出第三张骨架屏", () => {
    // 用户要两张：A 成功、B 超时；模型重试 B → 第三次工具调用
    const calls = [call("c1", "方案一"), call("c2", "方案二"), call("c3", "方案二")];
    const results = [result("c1", "/tmp/a.png")];

    const cards = getTurnImageCalls(calls, results);

    // 修之前是 3 张（每次工具调用一张），越重试越多
    expect(cards).toHaveLength(2);
    expect(cards.map((c) => c.prompt)).toEqual(["方案一", "方案二"]);
  });

  it("重试成功之后，已出的图不会被后来失败的那次盖回骨架屏", () => {
    // 第二次调用成功出图，第三次（更晚）又失败了
    const calls = [call("c1", "方案二"), call("c2", "方案二"), call("c3", "方案二")];
    const results = [result("c2", "/tmp/b.png")];

    const cards = getTurnImageCalls(calls, results);

    expect(cards).toHaveLength(1);
    expect(cards[0].path).toBe("file:///tmp/b.png");
  });

  it("不同 prompt 各占一张，不会被合并", () => {
    const cards = getTurnImageCalls([call("c1", "方案一"), call("c2", "方案二")], []);
    expect(cards).toHaveLength(2);
  });

  it("没有生成图的轮次返回空", () => {
    expect(getTurnImageCalls([{ id: "x", name: "view_file", arguments: "{}" }], [])).toHaveLength(0);
    expect(getTurnImageCalls([], [])).toHaveLength(0);
  });

  /**
   * 两张不同的图**绝不能拿到同一个路径**。
   *
   * 原来的结果匹配有一条兜底：`|| r.name.includes("generate_image")` ——
   * call id 对不上就随便找一条生成图的结果。后果是每张卡都拿到第一张图的路径：
   * 用户生成两个不同的 logo，界面上并排显示两张一模一样的图，
   * 而磁盘上那两个文件其实完全不同（821816 vs 737810 字节，SHA 也不一样）。
   *
   * 张冠李戴比不显示更糟：用户会以为模型生成了两张相同的图。
   */
  it("call id 对不上时宁可没有路径，也不能张冠李戴", () => {
    const calls = [call("c1", "灵魂之火"), call("c2", "展翅之鸟")];
    // 结果里的 callId 和调用对不上（真实会话里出现过）
    const results = [
      { callId: "other-1", name: "generate_image", content: "![](file:///tmp/logo1.png)", ok: true },
    ];

    const cards = getTurnImageCalls(calls, results);

    expect(cards).toHaveLength(2);
    // 两张都不该拿到那条对不上的结果
    expect(cards[0].path).toBeUndefined();
    expect(cards[1].path).toBeUndefined();
  });

  it("call id 对得上时各自拿各自的路径", () => {
    const calls = [call("c1", "灵魂之火"), call("c2", "展翅之鸟")];
    const results = [result("c1", "/tmp/logo1.png"), result("c2", "/tmp/logo2.png")];

    const cards = getTurnImageCalls(calls, results);

    expect(cards.map((c) => c.path)).toEqual(["file:///tmp/logo1.png", "file:///tmp/logo2.png"]);
    // 正名：两张路径必须不同 —— 这正是用户看到「两个一模一样」时坏掉的那条
    expect(cards[0].path).not.toBe(cards[1].path);
  });
});

describe("图片工具失败要传到卡片上", () => {
  it("ok:false 的结果标记成 failed，否则骨架屏永远转", () => {
    const got = getTurnImageCalls(
      [{ id: "c1", name: "generate_image", arguments: JSON.stringify({ prompt: "x" }) }],
      [{ callId: "c1", name: "generate_image", content: "LoRA 对不上", ok: false }],
    );
    expect(got[0].failed).toBe(true);
  });

  /** 重试成功后不该被之前那次失败的记录盖回失败态 */
  it("同 prompt 重试成功时不留失败标记", () => {
    const call = (id: string) => ({
      id,
      name: "generate_image",
      arguments: JSON.stringify({ prompt: "x", width: 1024, height: 1024 }),
    });
    const got = getTurnImageCalls(
      [call("c1"), call("c2")],
      [
        { callId: "c1", name: "generate_image", content: "失败", ok: false },
        { callId: "c2", name: "generate_image", content: "![i](file:///tmp/a.png)", ok: true },
      ],
    );
    expect(got).toHaveLength(1);
    expect(got[0].failed).toBe(false);
    expect(got[0].path).toBe("file:///tmp/a.png");
  });
});

describe("正文图片渲染判定", () => {
  it("流式输出里的半截 markdown 不能算已经渲染图片", () => {
    expect(contentRendersImage("![Logo](file:///tmp/logo.png")).toBe(false);
    expect(contentRendersImage("![Logo](")).toBe(false);
  });

  it("完整图片 markdown 才允许撤掉独立生成卡片", () => {
    expect(contentRendersImage("![Logo](file:///tmp/logo.png)", "file:///tmp/logo.png")).toBe(true);
  });
});
