import { describe, expect, it } from "vitest";
import {
  advancePhase,
  appendLog,
  buildButtonLabel,
  canApply,
  canBuild,
  checkSummary,
  elapsedText,
  LOG_MAX,
  phaseLabel,
  staleBuildNote,
  stepState,
} from "./updateState";
import type { UpdateCheck, UpdateProgress } from "../api/updater";

const ok: UpdateCheck = {
  repo: "/repo",
  branch: "main",
  behind: 3,
  commits: [],
  running_sha: "abc1234",
  behind_built: 0,
  built_commits: [],
  running_dirty: false,
  head_sha: "def5678",
  bundle_ready: false,
  blocked: null,
};

describe("版本与更新", () => {
  /** **blocked 有值时按钮必须锁死** —— 点下去必定失败，亮着只会让人试三次 */
  it("不能更新时按钮不可点", () => {
    for (const why of ["工作区有改动", "找不到仓库", "在别的分支"]) {
      expect(canBuild("checked", { ...ok, blocked: why })).toBe(false);
    }
    expect(canBuild("checked", ok)).toBe(true);
    expect(canBuild("checked", null)).toBe(false);
  });

  it("构建中不能重复点", () => {
    expect(canBuild("building", ok)).toBe(false);
    expect(canBuild("checking", ok)).toBe(false);
  });

  /** **换包会退出应用**，构建没完成就换 = 装一个半成品 */
  it("只有构建完成才能换包", () => {
    expect(canApply("ready")).toBe(true);
    for (const s of ["idle", "checking", "checked", "building", "failed"] as const) {
      expect(canApply(s)).toBe(false);
    }
  });

  /** **不是百分比** —— cargo 不告诉你总数，凑一个只会在 87% 停十分钟 */
  it("显示已编译个数而不是百分比", () => {
    const prog = (over: Partial<UpdateProgress>): UpdateProgress => ({
      phase: "cargo",
      label: "编译 serde",
      crates: 42,
      line: "",
      elapsed: 0,
      ...over,
    });
    expect(phaseLabel(prog({}))).toContain("已编译 42 个");
    expect(phaseLabel(prog({}))).not.toContain("%");
    // 还没开始编时不显示「已编译 0 个」
    expect(phaseLabel(prog({ phase: "git", label: "拉取新代码", crates: 0 }))).toBe("拉取新代码");
  });

  /** 没有更新也要说清楚，不能什么都不显示 */
  it("查完总有一句话", () => {
    expect(checkSummary({ ...ok, behind: 0 })).toContain("最新");
    expect(checkSummary(ok)).toContain("3 个新提交");
    expect(checkSummary({ ...ok, blocked: "工作区有改动" })).toBe("工作区有改动");
  });

  /**
   * **这条是 2026-09-18 现场抓到的漏**：面板说「已经是最新的」，
   * 而用户跑的二进制比磁盘上的代码旧两个提交 —— 因为只比了 HEAD↔origin。
   */
  it("跑的是旧构建时必须说出来", () => {
    expect(staleBuildNote({ ...ok, behind: 0, behind_built: 2 })).toContain("2 个提交");
    expect(staleBuildNote({ ...ok, behind: 0, behind_built: 2 })).toContain("重新构建");
    // 远端一致也要报 —— 这两段是独立的
    expect(staleBuildNote({ ...ok, behind: 0, behind_built: 1 })).not.toBeNull();
  });

  it("带未提交改动编出来的构建要标出来", () => {
    expect(staleBuildNote({ ...ok, behind_built: 0, running_dirty: true })).toContain("未提交");
    expect(staleBuildNote({ ...ok, behind_built: 0, running_dirty: false })).toBeNull();
  });

  /** 跑的就是当前代码时不要多话 */
  it("构建是新的就什么都不说", () => {
    expect(staleBuildNote({ ...ok, behind_built: 0, running_dirty: false })).toBeNull();
  });

  it("按钮文案区分「有新提交」和「只是重新构建」", () => {
    expect(buildButtonLabel("checked", ok)).toContain("3 个新提交");
    expect(buildButtonLabel("checked", { ...ok, behind: 0, behind_built: 2 })).toContain("重新构建");
    expect(buildButtonLabel("building", ok)).toContain("构建中");
    expect(buildButtonLabel("ready", ok)).toContain("已构建");
  });

  /**
   * **2026-09-28 用户：「构建完了，然后点检查更新，版本号没变，但是只有重新构建，
   * 就会又做一遍相同版本的构建」。** 两个口子：
   */
  describe("不重复构建同一个版本", () => {
    const latest: UpdateCheck = { ...ok, behind: 0, behind_built: 0, running_dirty: false };
    const builtNotApplied: UpdateCheck = { ...ok, behind: 0, behind_built: 1, bundle_ready: true };

    it("跑的就是最新代码：构建按钮不可点、写明不用构建，也没有换包", () => {
      expect(canBuild("checked", latest)).toBe(false);
      expect(buildButtonLabel("checked", latest)).toContain("不用构建");
      expect(checkSummary(latest)).toContain("正在跑的就是");
      expect(canApply("checked", latest)).toBe(false);
      expect(staleBuildNote(latest)).toBeNull();
    });

    it("构建完没换包、又点了检查更新：直接能换包，不再叫人重新构建", () => {
      expect(canApply("checked", builtNotApplied)).toBe(true);
      expect(canBuild("checked", builtNotApplied)).toBe(false);
      expect(buildButtonLabel("checked", builtNotApplied)).toContain("不用再编");
      expect(staleBuildNote(builtNotApplied)).toContain("换包并重启");
      expect(staleBuildNote(builtNotApplied)).not.toContain("重新构建当前代码");
    });

    it("包是旧的（没构建过当前代码）才叫人重新构建", () => {
      const stale = { ...builtNotApplied, bundle_ready: false };
      expect(canBuild("checked", stale)).toBe(true);
      expect(canApply("checked", stale)).toBe(false);
      expect(staleBuildNote(stale)).toContain("重新构建");
    });

    it("远端有新提交：不管磁盘上的包，都要下载并构建", () => {
      const remote = { ...builtNotApplied, behind: 2 };
      expect(canBuild("checked", remote)).toBe(true);
      expect(canApply("checked", remote)).toBe(false);
      expect(buildButtonLabel("checked", remote)).toContain("2 个新提交");
    });

    it("工作区脏等被拦下时，什么都不能点", () => {
      const blocked = { ...builtNotApplied, blocked: "工作区有改动" };
      expect(canBuild("checked", blocked)).toBe(false);
      expect(canApply("checked", blocked)).toBe(false);
    });
  });
});

describe("构建进度显示", () => {
  const p = (over: Partial<UpdateProgress> = {}): UpdateProgress => ({
    phase: "cargo",
    label: "编译 serde",
    crates: 3,
    line: "   Compiling serde v1.0",
    elapsed: 12,
    ...over,
  });

  /**
   * **心跳不能推进阶段。** 链接期每 2 秒来一条 `tick`，
   * 拿它去推阶段的话步骤条会被打回「未开始」。
   */
  it("心跳和失败都不推进阶段", () => {
    expect(advancePhase("cargo", p({ phase: "tick", line: "", label: "" }))).toBe("cargo");
    expect(advancePhase("cargo", p({ phase: "failed", label: "炸了" }))).toBe("cargo");
    expect(advancePhase("cargo", p({ phase: "link" }))).toBe("link");
  });

  /** **走过的格子要标成完成** —— 只高亮当前那格看不出走到哪 */
  it("步骤条标出走过的格子", () => {
    expect(stepState("git", "cargo")).toBe("done");
    expect(stepState("frontend", "cargo")).toBe("done");
    expect(stepState("cargo", "cargo")).toBe("now");
    expect(stepState("link", "cargo")).toBe("todo");
    // 构建完成时全部算完成
    expect(stepState("bundle", "done")).toBe("done");
    // 还没开始
    expect(stepState("git", null)).toBe("todo");
  });

  /** 心跳没有输出行，不该往日志里塞空行 */
  it("心跳不进日志", () => {
    expect(appendLog(["a"], p({ phase: "tick", line: "" }))).toEqual(["a"]);
    expect(appendLog(["a"], p({ line: "b" }))).toEqual(["a", "b"]);
  });

  it("日志超上限从头丢", () => {
    const long = Array.from({ length: LOG_MAX }, (_, i) => `l${i}`);
    const next = appendLog(long, p({ line: "新的" }));
    expect(next.length).toBe(LOG_MAX);
    expect(next[next.length - 1]).toBe("新的");
    expect(next[0]).toBe("l1");
  });

  /** 链接期只有秒数在动，格式要一眼看懂 */
  it("已用时间格式", () => {
    expect(elapsedText(0)).toBe("0:00");
    expect(elapsedText(65)).toBe("1:05");
    expect(elapsedText(600)).toBe("10:00");
  });

  /** 没有 label 的心跳不该显示成「（已编译 N 个）」 */
  it("心跳不冒充编译进度", () => {
    expect(phaseLabel(p({ phase: "tick", label: "", crates: 5 }))).toBe("");
  });
});
