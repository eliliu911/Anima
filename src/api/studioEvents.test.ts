import { describe, expect, it } from "vitest";
import { affects } from "./studioEvents";

describe("Studio 改动事件", () => {
  it("种类、项目对得上才算", () => {
    expect(affects({ kind: "assets", project_id: "p1" }, {})).toBe(true);
    expect(affects({ kind: "assets", project_id: "p1" }, { kinds: ["project"] })).toBe(false);
    expect(affects({ kind: "assets", project_id: "p1" }, { projectId: "p2" })).toBe(false);
    expect(affects({ kind: "assets" }, { projectId: "p2" }), "全局资产的改动每个项目页都要刷").toBe(true);
    expect(affects({ kind: "episode", project_id: "p2" }, { kinds: ["episode", "project"], projectId: "p2" })).toBe(true);
  });
});
