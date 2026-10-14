import { describe, expect, it, beforeEach, vi } from "vitest";
import { getStorage } from "../store/useCanvasStore";
import { loadViewport, saveViewport, viewportKey } from "./viewportMemory";
import shell from "../../components/studio/StudioShell.tsx?raw";
import view from "../CanvasView.tsx?raw";

// 测试环境没有本地存储：装一个最小的（getStorage 每次调用时现取，换得掉）
function fakeStorage(): Storage {
  const m = new Map<string, string>();
  return {
    get length() { return m.size; },
    clear: () => m.clear(), key: (i: number) => [...m.keys()][i] ?? null,
    getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, String(v)), removeItem: (k: string) => void m.delete(k),
  };
}

describe("画布位置和缩放按工程记住", () => {
  let storage: Storage;
  beforeEach(() => { vi.stubGlobal("localStorage", fakeStorage()); storage = getStorage()!; });
  const usable = true;

  it.runIf(usable)("存了读得回来；两个工程各是各的；自由创作也单独一份", () => {
    saveViewport("p1", { x: -1200, y: 340, zoom: 0.42 });
    saveViewport(null, { x: 5, y: 6, zoom: 2 });
    expect(loadViewport("p1")).toEqual({ x: -1200, y: 340, zoom: 0.42 });
    expect(loadViewport(null)).toEqual({ x: 5, y: 6, zoom: 2 });
    expect(loadViewport("没存过")).toBeNull();
  });

  it.runIf(usable)("坏数据当没存过", () => {
    storage!.setItem(viewportKey("p1"), "{坏的");
    expect(loadViewport("p1")).toBeNull();
    storage!.setItem(viewportKey("p1"), JSON.stringify({ x: 1, y: 2, zoom: 0 }));
    expect(loadViewport("p1")).toBeNull();
  });

  it("画布挪完就存、打开按工程恢复", () => {
    expect(view).toContain("onMoveEnd={(_, vp) => saveViewport(useCanvasStore.getState().activeProjectId, vp)}");
    expect(view).toContain("defaultViewport={loadViewport(activeProjectId) ?? { x: 0, y: 0, zoom: 1 }}");
    expect(view).not.toContain("defaultViewport={{ x: 0, y: 0, zoom: 1 }}");
  });
});

// 2026-10-13：外层把画布工程初值写成 null，画布一打开就切到「自由创作」、还删掉记住的上次工程
describe("打开画布不冲掉记住的工程", () => {
  it("外层初值是 undefined（= 没指定），不是 null", () => {
    expect(shell).toContain("useState<string | null | undefined>(undefined)");
    expect(shell).not.toMatch(/canvasProjectId, setCanvasProjectId\] = useState<string \| null>\(null\)/);
  });
  it("画布只在外层明确指定了工程时才切", () => {
    expect(view).toContain("if (propProjectId !== undefined && propProjectId !== activeProjectId)");
  });
});
