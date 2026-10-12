import { describe, expect, it, beforeEach } from "vitest";

// 这个 store 在模块初始化时就读 localStorage / window —— node 环境里
// 两个都没有。**桩必须在 import 之前装**，所以这里用动态 import
const store = new Map<string, string>();
(globalThis as never as { localStorage: unknown }).localStorage = {
  getItem: (k: string) => store.get(k) ?? null,
  setItem: (k: string, v: string) => void store.set(k, v),
  removeItem: (k: string) => void store.delete(k),
};
(globalThis as never as { window: unknown }).window = {
  addEventListener: () => {},
  removeEventListener: () => {},
};

const { useFace } = await import("./face");

/**
 * 面容 store 的幂等性。
 *
 * ## 为什么这条值得单独测
 *
 * `modelId` 和 `mode` 是 `Avatar3D` 那个 effect 的依赖 —— 它们一变，
 * **整个 three 场景就会重建**：重新从磁盘读 GLB、重新解析、重新对框相机。
 * 那要好几秒，表现是所有窗口白屏、角色层消失再回来。
 *
 * 而这两个 setter 会被三条路调用：后端广播 `anima://avatar-model`、
 * 跨窗口 localStorage 同步、界面点击。**重复设成同一个值是常态** ——
 * 实测撞到过：在角色形态里改一下音色（和面容毫无关系），后端照样广播
 * 一次面容，于是脸被拆了重装。
 */
describe("面容 store 的幂等性", () => {
  beforeEach(() => {
    useFace.setState({ modelId: "a.vrm", mode: "concrete", report: null });
  });

  /// 设成同一个值不能产生新的 state 对象 —— 那会让订阅者重渲染
  it("重复设同一个模型不改变状态", () => {
    const before = useFace.getState();
    useFace.getState().setModelId("a.vrm");
    expect(useFace.getState()).toBe(before);
  });

  it("重复设同一个形态不改变状态", () => {
    const before = useFace.getState();
    useFace.getState().setMode("concrete");
    expect(useFace.getState()).toBe(before);
  });

  it("真的换模型时要生效", () => {
    useFace.getState().setModelId("b.vrm");
    expect(useFace.getState().modelId).toBe("b.vrm");
  });

  it("真的换形态时要生效", () => {
    useFace.getState().setMode("abstract");
    expect(useFace.getState().mode).toBe("abstract");
  });

  /// 换模型要清掉上一个模型的诊断报告 —— 否则设置面板里显示的
  /// 还是上一个模型的口型通道数
  it("换模型会清掉上一个模型的报告", () => {
    useFace.setState({ report: { channels: 14, scheme: "vowel", found: ["aa"] } as never });
    useFace.getState().setModelId("b.vrm");
    expect(useFace.getState().report).toBeNull();
  });
});
