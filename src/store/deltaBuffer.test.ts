import { describe, expect, it, vi } from "vitest";

import { DeltaBuffer, type PendingDelta } from "./deltaBuffer";

/** 收集每次写出的批次，用来断言"合并成了几次"。 */
function recorder() {
  const batches: Map<string, PendingDelta>[] = [];
  const buf = new DeltaBuffer((b) => batches.push(b), 16);
  return { buf, batches };
}

describe("DeltaBuffer", () => {
  it("一个窗口内的增量合并成一次写出，内容不丢不乱序", async () => {
    vi.useFakeTimers();
    const { buf, batches } = recorder();

    // 模拟按 token 推流：60 个 delta 落在同一个窗口里
    for (let i = 0; i < 60; i++) buf.push("t1", "text", `${i},`);
    expect(batches).toHaveLength(0); // 还没到窗口，一次都不该写

    vi.advanceTimersByTime(16);

    // 这条断言是重点：60 次推送**只触发 1 次**状态更新。
    // 不合并的话就是 60 次全量重渲染 —— 那正是主线程被占死的成因。
    expect(batches).toHaveLength(1);
    expect(batches[0].get("t1")?.text).toBe(
      Array.from({ length: 60 }, (_, i) => `${i},`).join(""),
    );
    vi.useRealTimers();
  });

  it("正文和思考各走各的，不会串", () => {
    vi.useFakeTimers();
    const { buf, batches } = recorder();
    buf.push("t1", "text", "正文");
    buf.push("t1", "reasoning", "思考");
    vi.advanceTimersByTime(16);
    expect(batches[0].get("t1")).toEqual({ text: "正文", reasoning: "思考" });
    vi.useRealTimers();
  });

  it("多个轮次同时在流时互不干扰", () => {
    vi.useFakeTimers();
    const { buf, batches } = recorder();
    buf.push("a", "text", "AAA");
    buf.push("b", "text", "BBB");
    vi.advanceTimersByTime(16);
    expect(batches).toHaveLength(1);
    expect(batches[0].get("a")?.text).toBe("AAA");
    expect(batches[0].get("b")?.text).toBe("BBB");
    vi.useRealTimers();
  });

  it("flush 会立刻写出攒着的内容 —— 非增量事件靠它保住顺序", () => {
    const { buf, batches } = recorder();
    buf.push("t1", "text", "工具调用之前的这段文字");
    // 处理 tool_call 之前必须 flush，否则工具卡片会插到这段文字前面
    buf.flush();
    expect(batches).toHaveLength(1);
    expect(batches[0].get("t1")?.text).toBe("工具调用之前的这段文字");
  });

  it("没有待写内容时 flush 是空操作，不制造多余的状态更新", () => {
    const { buf, batches } = recorder();
    buf.flush();
    buf.flush();
    expect(batches).toHaveLength(0);
  });

  it("flush 之后定时器不会再补一次重复写出", () => {
    vi.useFakeTimers();
    const { buf, batches } = recorder();
    buf.push("t1", "text", "x");
    buf.flush();
    vi.advanceTimersByTime(100);
    expect(batches).toHaveLength(1); // 只有 flush 那一次
    vi.useRealTimers();
  });

  it("discard 丢掉攒着的内容且不写出 —— 清空会话/切会话时用", () => {
    vi.useFakeTimers();
    const { buf, batches } = recorder();
    buf.push("t1", "text", "会被丢掉");
    buf.discard();
    vi.advanceTimersByTime(100);
    expect(batches).toHaveLength(0);
    expect(buf.pendingCount).toBe(0);
    vi.useRealTimers();
  });

  it("空字符串不占用一次调度", () => {
    vi.useFakeTimers();
    const { buf, batches } = recorder();
    buf.push("t1", "text", "");
    vi.advanceTimersByTime(100);
    expect(batches).toHaveLength(0);
    vi.useRealTimers();
  });
});
