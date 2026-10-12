import { describe, expect, it, beforeEach } from "vitest";
import { studioLinkFromToolResult, takePendingStudioLink } from "./studioNav";

describe("跳到 Studio", () => {
  beforeEach(() => {
    const store = new Map<string, string>();
    Object.defineProperty(globalThis, "localStorage", {
      configurable: true,
      value: { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v), removeItem: (k: string) => void store.delete(k) },
    });
  });

  it("从工具返回里认出跳转目标", () => {
    expect(studioLinkFromToolResult('{"summary":"x","studio_link":{"project_id":"p1","episode_id":"ep_01","page":"assets"}}')).toEqual({ project_id: "p1", episode_id: "ep_01", page: "assets" });
    expect(studioLinkFromToolResult('{"summary":"x"}')).toBeNull();
    expect(studioLinkFromToolResult("不是 JSON")).toBeNull();
  });

  it("窗口刚打开时从备份里取，取一次就没了，过期的不要", () => {
    localStorage.setItem("anima.studio.pendingNav", JSON.stringify({ project_id: "p1", at: 1000 }));
    expect(takePendingStudioLink(2000)?.project_id).toBe("p1");
    expect(takePendingStudioLink(2000)).toBeNull();
    localStorage.setItem("anima.studio.pendingNav", JSON.stringify({ project_id: "p1", at: 1000 }));
    expect(takePendingStudioLink(1000 + 120_000)).toBeNull();
  });
});
