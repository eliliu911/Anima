import { describe, expect, it } from "vitest";

import type { ChatTurn } from "./chat";
// **从真模块导入** —— 在测试里另写一份副本的话，
// 它和 loadSession 实际跑的代码可以完全不同，测了等于没测。
import {
  mergeOnSessionLoad,
  reconstructHistoryTurn,
  visibleTurns,
  serializeTurnTools,
  parseTurnTools,
  TOOL_RESULT_PERSIST_CHARS,
} from "./sessionTurns";

function turn(over: Partial<ChatTurn>): ChatTurn {
  return {
    id: "t",
    role: "assistant",
    content: "",
    reasoning: "",
    toolCalls: [],
    streaming: false,
    ...over,
  };
}

describe("切会话时的轮次取舍", () => {
  const A = "session-a";
  const B = "session-b";

  it("切走再切回来，正在生成的内容还在", () => {
    const streaming = turn({
      id: "live",
      sessionId: A,
      streaming: true,
      content: "已经流出来的半截内容",
    });

    // 切到 B：在流的那一轮必须留在 store 里，否则后续增量写进空气
    const atB = mergeOnSessionLoad([turn({ id: "b1", sessionId: B })], [streaming], B, "stream-1");
    expect(atB.turns).toContain(streaming);
    // 但不该显示在 B 里
    expect(visibleTurns(atB.turns, B).map((t) => t.id)).toEqual(["b1"]);
    // 停止按钮也不该对别的会话的流生效
    expect(atB.activeStreamId).toBeNull();

    // 切回 A：内容必须还在，且重新可见
    const atA = mergeOnSessionLoad([turn({ id: "a1", sessionId: A })], atB.turns.filter((t) => t.streaming), A, "stream-1");
    const shown = visibleTurns(atA.turns, A);
    expect(shown.map((t) => t.id)).toEqual(["a1", "live"]);
    expect(shown.find((t) => t.id === "live")?.content).toBe("已经流出来的半截内容");
    expect(atA.activeStreamId).toBe("stream-1");
  });

  it("已经跑完的轮次不留 —— 它们已落库，留着会和历史重复", () => {
    const finished = turn({ id: "done", sessionId: A, streaming: false, content: "旧回复" });
    // loadSession 只把 `streaming` 的挑出来，所以跑完的那些压根不会进 liveTurns
    const merged = mergeOnSessionLoad([turn({ id: "a1", sessionId: A })], [], A, null);
    expect(merged.turns.map((t) => t.id)).toEqual(["a1"]);
    expect(merged.turns).not.toContain(finished);
  });

  it("没有 sessionId 的轮次（语音等）一律显示", () => {
    const voice = turn({ id: "voice", content: "语音说的" });
    expect(visibleTurns([voice, turn({ id: "other", sessionId: B })], A).map((t) => t.id)).toEqual([
      "voice",
    ]);
  });

  it("同时有两个会话在流时互不串台", () => {
    const liveA = turn({ id: "la", sessionId: A, streaming: true, content: "A 的内容" });
    const liveB = turn({ id: "lb", sessionId: B, streaming: true, content: "B 的内容" });
    const merged = mergeOnSessionLoad([], [liveA, liveB], A, "stream-a");
    expect(visibleTurns(merged.turns, A).map((t) => t.id)).toEqual(["la"]);
    expect(visibleTurns(merged.turns, B).map((t) => t.id)).toEqual(["lb"]);
    // 两条都还在 store 里，增量不会丢
    expect(merged.turns).toHaveLength(2);
  });

  it("重启/加载历史消息时能正确还原带有图片附件的用户轮次（包含空格路径）", () => {
    const rawContent = `[上传图片]
以下图片已经保存为本地文件。若用户要求根据上传图片生成视频、图生视频或 I2V，必须调用 animate_image，并把对应的 file_url 或 local_path 原样作为 image_path 参数传入。
- Image 1: 450x_auto___so.webp (22.5 KB)
  file_url: file:///Users/you/Library/Application Support/com.anima.desktop/uploads/450x_auto___so.webp
  local_path: /Users/you/Library/Application Support/com.anima.desktop/uploads/450x_auto___so.webp

请帮根据这个图片生成视频，内容，温柔的背景音乐。手轻轻抚摸头发，眼神深情。`;

    const turn = reconstructHistoryTurn(
      { id: 101, role: "user", content: rawContent },
      A,
      (p) => `asset://${p}`,
    );

    expect(turn.displayText).toBe("请帮根据这个图片生成视频，内容，温柔的背景音乐。手轻轻抚摸头发，眼神深情。");
    expect(turn.images).toEqual(["asset:///Users/you/Library/Application Support/com.anima.desktop/uploads/450x_auto___so.webp"]);
    expect(turn.content).toBe(rawContent);
  });
});

/**
 * **重启后图片消失**的回归。
 *
 * `messages` 表长期只有 `role`/`content` 四列，工具调用与结果一个字没存；
 * 还原时 `toolCalls` 被写死成 `[]`。图片卡是从工具结果渲染的，所以
 * 重启回到对话里图片凭空消失 —— 文件一直在磁盘上，丢的只是记录里的引用。
 * 本机 79 条 assistant 消息里只有 2 条还看得见图，那 2 条是模型碰巧
 * 自己在正文里回显了链接，是运气不是设计。
 */
describe("工具调用的落库与还原", () => {
  const imageCall = { id: "call_1", name: "generate_image", arguments: '{"prompt":"图书馆"}' };
  const imageResult = {
    callId: "call_1",
    name: "generate_image",
    content: "![生成的图像](file:///x/img_372008618.png)\nSeed: 372008618",
    ok: true,
  };

  it("图片工具的调用与结果能原样走一个来回", () => {
    const json = serializeTurnTools({ toolCalls: [imageCall], toolResults: [imageResult] });
    expect(json).toBeTruthy();
    const back = parseTurnTools(json);
    expect(back.toolCalls).toEqual([imageCall]);
    expect(back.toolResults[0].content).toContain("img_372008618.png");
  });

  it("还原出的历史轮次带得上图片卡所需的工具结果", () => {
    const json = serializeTurnTools({ toolCalls: [imageCall], toolResults: [imageResult] });
    const turn = reconstructHistoryTurn(
      { id: 7, role: "assistant", content: "给你生成好了", tool_json: json },
      "sess-1",
    );
    expect(turn.toolCalls).toHaveLength(1);
    expect(turn.toolResults?.[0].content).toContain("file:///x/img_372008618.png");
  });

  it("没有工具的普通对话不写这一列", () => {
    expect(serializeTurnTools({ toolCalls: [], toolResults: [] })).toBeUndefined();
    expect(serializeTurnTools({})).toBeUndefined();
  });

  it("老消息（这一列是 NULL）照常打开，当作没有工具", () => {
    const turn = reconstructHistoryTurn({ id: 1, role: "assistant", content: "旧的" }, "s");
    expect(turn.toolCalls).toEqual([]);
    expect(parseTurnTools(null).toolCalls).toEqual([]);
  });

  it("坏 JSON 不能打不开整个对话", () => {
    expect(parseTurnTools("{这不是 JSON").toolCalls).toEqual([]);
    expect(parseTurnTools('{"toolCalls":"不是数组"}').toolCalls).toEqual([]);
  });

  it("超长结果截断，避免每次抓网页都在库里留一份全文", () => {
    const huge = "x".repeat(TOOL_RESULT_PERSIST_CHARS + 5000);
    const back = parseTurnTools(
      serializeTurnTools({
        toolCalls: [{ id: "c", name: "fetch", arguments: "{}" }],
        toolResults: [{ callId: "c", name: "fetch", content: huge }],
      }),
    );
    expect(back.toolResults[0].content.length).toBeLessThan(huge.length);
    expect(back.toolResults[0].content).toContain("已截断");
  });
});
