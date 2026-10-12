import { describe, expect, it } from "vitest";

import { getTurnImageCalls, getTurnVideoCalls } from "../ChatHistory";
import { reconstructHistoryTurn, serializeTurnTools } from "../../store/sessionTurns";

/**
 * **重启后图片消失**的最后一环。
 *
 * 落库和还原各自有测试，但真正决定用户看不看得见图的是这一段：
 * 还原出来的 `toolCalls` / `toolResults` 喂给 `getTurnImageCalls`，
 * 得真的吐出图片路径。
 *
 * 最容易断的是 **call id**：`getTurnImageCalls` 只按 `callId === tc.id` 配对，
 * 配不上就没有路径 —— 而还原出来的历史轮次**没有进度事件兜底**，
 * 结果是一张永远转不完的骨架屏。比丢图更难查，因为它看起来像"还在生成"。
 */
describe("重启后还原的图片卡", () => {
  const turn = {
    toolCalls: [{ id: "call_abc", name: "generate_image", arguments: '{"prompt":"图书馆里的少女"}' }],
    toolResults: [
      {
        callId: "call_abc",
        name: "generate_image",
        content:
          "Seed: `372008618`\n\n![生成的图像](file:///Users/x/.anima/generated/img_372008618.png)",
        ok: true,
      },
    ],
  };

  /** 走一遍真实路径：序列化 → 存 → 读回 → 还原成 ChatTurn */
  const restored = () =>
    reconstructHistoryTurn(
      { id: 42, role: "assistant", content: "给你生成好了", tool_json: serializeTurnTools(turn) },
      "sess-1",
    );

  it("还原后的轮次能渲染出图片路径", () => {
    const cards = getTurnImageCalls(restored().toolCalls, restored().toolResults);
    expect(cards).toHaveLength(1);
    expect(cards[0].path).toBe("file:///Users/x/.anima/generated/img_372008618.png");
  });

  it("call id 必须一起活下来", () => {
    // 只按 call id 配对，且历史轮次没有进度事件兜底 ——
    // id 丢了就是一张永远转不完的骨架屏，看起来像"还在生成"
    const r = restored();
    expect(r.toolCalls[0].id).toBe("call_abc");
    expect(r.toolResults?.[0].callId).toBe("call_abc");
  });

  it("没存工具的老消息不会凭空冒出卡片", () => {
    const old = reconstructHistoryTurn({ id: 1, role: "assistant", content: "旧的" }, "s");
    expect(getTurnImageCalls(old.toolCalls, old.toolResults)).toEqual([]);
  });

  it("视频走视频那条路，不会被当成图片", () => {
    const v = {
      toolCalls: [{ id: "c_v", name: "generate_video", arguments: '{"prompt":"海边"}' }],
      toolResults: [{ callId: "c_v", name: "generate_video", content: "[视频](file:///x/v.mp4)", ok: true }],
    };
    const r = reconstructHistoryTurn(
      { id: 9, role: "assistant", content: "好了", tool_json: serializeTurnTools(v) },
      "s",
    );
    expect(getTurnImageCalls(r.toolCalls, r.toolResults)).toEqual([]);
    expect(getTurnVideoCalls(r.toolCalls, r.toolResults).length).toBeGreaterThan(0);
  });
});
