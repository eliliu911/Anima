import { describe, expect, it } from "vitest";
import { nodeTaskTitle } from "./nodeTaskTitle";

describe("任务里显示的节点名", () => {
  it("有步骤说明：取第几步和那一句", () => {
    const n = { id: "partA", type: "videoGen", data: { guide: ["【第 5 步 / 共 6 步】第一段 · 对峙与追逐（0~15 秒）。参考出片……"] } };
    expect(nodeTaskTitle(n)).toBe("第 5 步 · 第一段 · 对峙与追逐");
  });
  it("没有步骤说明：节点类型 + id 末尾，同类节点分得开", () => {
    expect(nodeTaskTitle({ id: "videoGen-1791-abcd", type: "videoGen", data: {} })).toBe("视频 abcd");
    expect(nodeTaskTitle({ id: "videoGen-1791-wxyz", type: "videoGen", data: {} })).not.toBe("视频 abcd");
  });
});
