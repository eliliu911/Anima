import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import type { KeepAwakeStatus, SleepBlocker } from "../../api/keepAwake";
import { animaLine, awake, blockerLine, hm } from "./keepAwakeView";

const blocker = (p: Partial<SleepBlocker>): SleepBlocker => ({ pid: 1, process: "caffeinate", name: "", secs_left: null, on_behalf_of: null, anima: false, ...p });
const status = (p: Partial<KeepAwakeStatus>): KeepAwakeStatus => ({ anima_holding: false, since: null, stalled: false, last_alive: null, stall_secs: 86400, blockers: [], ...p });

/** 2026-09-30 用户：「我不知道 Anima 的防休眠进程生没生效，因为界面里没地方查看」 */
describe("防休眠标记", () => {
  it("有任何一个在挡就算不会睡；一个都没有才说会睡", () => {
    expect(awake(status({}))).toBe(false);
    expect(awake(status({ blockers: [blocker({ secs_left: 64800 })] }))).toBe(true);
  });

  it("Anima 那一行：在按 / 太久没进度松手了 / 没在按，三种说法不一样", () => {
    expect(animaLine(status({ anima_holding: true, since: 1000 }), 1000 + 2 * 3600 + 5 * 60)).toContain("已 2 小时 5 分");
    // 2026-10-02：不再说「最多按住 24 小时」—— 那个上限把一整集的批量出片放掉了
    const holding = animaLine(status({ anima_holding: true, since: 1000 }), 1060);
    expect(holding).toContain("有进度就一直按着，连续 24 小时没进度才松手");
    expect(holding).not.toContain("最多按住");
    const stalled = animaLine(status({ stalled: true, last_alive: 1000 }), 1000 + 25 * 3600);
    expect(stalled).toContain("已经 25 小时没有任何进度");
    expect(stalled).toContain("多半挂住了");
    expect(stalled).toContain("一有新进度会重新按住");
    expect(animaLine(status({ stalled: true }), 0)).toContain("连续 24 小时没有任何进度");
    // 用户 10-02 问「是不是应该批量开启时自动开启」—— 本来就是自动的，没在按的时候要把这句说出来
    expect(animaLine(status({}), 0)).toContain("一开跑就自动开启");
  });

  it("每一个挡睡眠的：谁、还剩多久", () => {
    expect(blockerLine(blocker({ secs_left: 64383 }))).toBe("caffeinate（终端里开的） · 还剩 17 小时 53 分");
    expect(blockerLine(blocker({ anima: true, on_behalf_of: 12355 }))).toBe("Anima 的（跟着任务） · 跟着进程 12355，它退出就松手");
    expect(blockerLine(blocker({ process: "Claude" }))).toBe("Claude · 没有时限");
    expect(hm(59)).toBe("1 分钟");
    expect(hm(3600)).toBe("1 小时");
  });

  it("侧边栏底部每一页都看得到", () => {
    const SIDEBAR = readFileSync(fileURLToPath(new URL("./StudioSidebar.tsx", import.meta.url)), "utf8");
    expect(SIDEBAR).toContain("<KeepAwakeBadge />");
    // 弹出层贴着侧边栏、不伸进右边工作区（2026-09-30 用户截图：被项目卡片盖住）：按底部那一行定位、左右各留 8px
    expect(SIDEBAR).toContain('<div className="relative flex items-center gap-1 px-2">');
    const BADGE = readFileSync(fileURLToPath(new URL("./KeepAwakeBadge.tsx", import.meta.url)), "utf8");
    // 读系统真实状态，定时刷新、打开时立刻刷新
    expect(BADGE).toContain("keepAwakeStatus().then(setStatus)");
    expect(BADGE).toContain("setInterval(refresh, POLL_MS)");
    expect(BADGE).toContain("absolute bottom-full left-2 right-2 z-50");
    expect(BADGE).not.toContain("w-[300px]");
  });
});
