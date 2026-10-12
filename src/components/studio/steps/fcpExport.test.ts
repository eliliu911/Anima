import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { alignFcpNames, defaultFcpName, loadFcpPrefs, saveFcpPrefs } from "./fcpExport";

const read = (rel: string) => readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");

function memory() {
  const m = new Map<string, string>();
  return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) };
}

describe("导出到 FCP 的名字和记忆", () => {
  it("默认名照用户的 S01E01_标题 习惯，集名里带的 E01- 和书名号去掉", () => {
    expect(defaultFcpName(1, "E01-《第三次》")).toBe("S01E01_第三次");
    expect(defaultFcpName(12, "第三次")).toBe("S01E12_第三次");
    expect(defaultFcpName(3, "")).toBe("S01E03_第 3 集");
  });

  it("导出位置按项目记、名字按集记；存坏了照样给默认", () => {
    const s = memory();
    expect(loadFcpPrefs("p", "ep_01", 1, "E01-《第三次》", s)).toEqual({ dest: "", event: "S01E01_第三次", project: "S01E01_第三次_初剪" });
    saveFcpPrefs("p", "ep_01", { dest: "/Volumes/素材盘/回收师", event: "02_S01E01_第三次", project: "S01E01_第三次" }, s);
    expect(loadFcpPrefs("p", "ep_01", 1, "x", s).event).toBe("02_S01E01_第三次");
    // 另一集：位置沿用，名字用它自己的默认
    expect(loadFcpPrefs("p", "ep_02", 2, "E02-《救》", s)).toEqual({ dest: "/Volumes/素材盘/回收师", event: "S01E02_救", project: "S01E02_救_初剪" });
    s.setItem("anima.fcpExport.p", "{坏的");
    expect(loadFcpPrefs("p", "ep_01", 1, "E01-《第三次》", s).event).toBe("S01E01_第三次");
  });
});

/**
 * 2026-09-29 用户：剪映那个导出「是废的，试过根本不能用」，要 FCP；而且不想先「合成一集」。
 * 剪映按钮原来藏在「成片」卡片里 —— 不合成连入口都没有。
 */
describe("组装页的 FCP 入口", () => {
  const step = read("./AssemblyStep.tsx");

  it("入口在操作条上，不用先合成；剪映按钮拿掉了", () => {
    const bar = step.slice(step.indexOf("{/* 操作条"), step.indexOf("rendered.length === 0 && ("));
    expect(bar).toContain("setFcpOpen(");
    expect(bar).toContain("{fcpLabel}");
    expect(step).toMatch(/: "导出到 Final Cut Pro…";/);
    // 只要有一镜出了片就能导 —— 不看有没有成片
    expect(bar).toMatch(/disabled=\{rendered\.length === 0 \|\| fcpBusy\}/);
    expect(step).not.toContain("cinemaExportJianyingProject");
  });

  it("导出位置由用户选，没选不给导", () => {
    expect(step).toContain("cinemaChooseExportFolder(");
    expect(step).toMatch(/disabled=\{[^}]*!fcp\.dest/);
  });

  /** 用户 2026-10-05：「需要你做命名规则预设」「不要我后面每次都修改」 */
  it("按资源库对齐的名字只换没手改过的", () => {
    const suggested = { event: "03_S01E02_本机无武器", project: "S01E02_本机无武器_初剪" };
    // 记着的是老默认名（截图里那样）：两个都换
    const old = { dest: "/x", event: "S01E02_本机无武器", project: "S01E02_本机无武器" };
    expect(alignFcpNames(old, suggested, 2, "E02-《本机无武器》")).toEqual({ dest: "/x", ...suggested });
    // 空的也换
    expect(alignFcpNames({ dest: "/x", event: "", project: "" }, suggested, 2, "本机无武器")).toEqual({ dest: "/x", ...suggested });
    // 自己改过的留着
    const mine = { dest: "/x", event: "我自己的事件", project: "S01E02_精剪" };
    expect(alignFcpNames(mine, suggested, 2, "本机无武器")).toEqual(mine);
  });
});
