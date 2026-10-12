/**
 * 「版本与更新」那一块的纯逻辑。
 *
 * 拆出来是因为这里的状态机有几条**点错就出事**的规则：
 * 构建没完成不能换包、正在构建时按钮要锁、换包会退出应用所以必须先确认。
 */
import type { UpdateCheck, UpdatePhase, UpdateProgress } from "../api/updater";

export type Stage = "idle" | "checking" | "checked" | "building" | "ready" | "failed";

/** 阶段 → 给人看的一句话 */
export function phaseLabel(p: UpdateProgress): string {
  if (p.phase === "cargo" && p.crates > 0 && p.label) return `${p.label}（已编译 ${p.crates} 个）`;
  return p.label;
}

/**
 * 构建的几个阶段，**按顺序**。界面画成一条步骤条。
 *
 * 离散且真实 —— 比百分比诚实。**「链接」单独一格**，因为它最长也最安静：
 * 2026-09-18 实测增量构建只有 1 个 crate 要编，剩下六分钟全在链接，
 * 而 cargo 那段一个字都不输出，界面停在「已编译 1 个」看起来和卡死一样。
 */
export const PHASES: { key: UpdatePhase; label: string }[] = [
  { key: "git", label: "拉代码" },
  { key: "frontend", label: "前端" },
  { key: "cargo", label: "编译" },
  { key: "link", label: "链接" },
  { key: "bundle", label: "打包" },
];

/**
 * 某一格现在是什么状态。
 *
 * **走过的格子要标成「完成」**，只高亮当前那格的话，看不出来走到哪了。
 */
export function stepState(step: UpdatePhase, current: UpdatePhase | null): "done" | "now" | "todo" {
  if (!current) return "todo";
  if (current === "done") return "done";
  const i = PHASES.findIndex((p) => p.key === step);
  const c = PHASES.findIndex((p) => p.key === current);
  if (c < 0) return "todo"; // tick / failed 不推进阶段
  if (i < c) return "done";
  if (i === c) return "now";
  return "todo";
}

/** `tick` 和 `failed` 不是阶段，不能拿它们去推进步骤条 */
export function advancePhase(current: UpdatePhase | null, p: UpdateProgress): UpdatePhase | null {
  if (p.phase === "tick") return current;
  if (p.phase === "failed") return current;
  return p.phase;
}

/** 秒 → `3:05` 这种。**链接期只有它在动** */
export function elapsedText(secs: number): string {
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return `${m}:${String(s).padStart(2, "0")}`;
}

/** 日志窗最多留多少行。再多没人看，还会把内存撑起来 */
export const LOG_MAX = 300;

/** 往日志里追加一行，超上限从头丢 */
export function appendLog(log: string[], p: UpdateProgress): string[] {
  if (!p.line) return log; // 心跳没有输出
  const next = [...log, p.line];
  return next.length > LOG_MAX ? next.slice(next.length - LOG_MAX) : next;
}

/**
 * **正在跑的就是最新代码**：远端没有新提交、跑的构建就是本地 HEAD、也不是带改动编的。
 *
 * 这时候不该让人「重新构建」—— 编出来的还是同一个版本（2026-09-28 用户指出）。
 */
export function upToDate(c: UpdateCheck): boolean {
  return !c.blocked && c.behind === 0 && c.behind_built === 0 && !c.running_dirty;
}

/** 磁盘上已经有用当前代码编好的包、只差换包（远端也没有更新的） */
export function bundleReady(c: UpdateCheck | null): boolean {
  return !!c && !c.blocked && c.behind === 0 && c.bundle_ready;
}

/** 构建按钮上写什么 */
export function buildButtonLabel(stage: Stage, check: UpdateCheck): string {
  switch (stage) {
    case "building":
      return "构建中…";
    case "ready":
      return "已构建好";
    default:
      if (check.behind > 0) return `下载并构建（${check.behind} 个新提交）`;
      if (bundleReady(check)) return "已构建好，不用再编";
      if (upToDate(check)) return "已是最新，不用构建";
      return "重新构建当前代码";
  }
}

/**
 * **正在跑的构建和磁盘上的代码差多远。**
 *
 * 这是这一块最要紧的一句话，而且**优先于远端那段** —— 用户最常踩的是
 * 「拉过代码但没重新构建」，而不是「没拉代码」。
 * 只比 HEAD↔origin 的话这条永远不出现：两边一致，于是显示「已经是最新的」，
 * 而他跑的二进制其实旧了两个提交（2026-09-18 当场撞上）。
 *
 * 回 `null` = 跑的就是当前代码，不用说什么。
 */
export function staleBuildNote(c: UpdateCheck): string | null {
  // 构建好了、只差换包：**不要再叫人重新构建**
  if (bundleReady(c)) {
    return `新版本（${c.head_sha}）已经构建好了，还没换上 —— 点「换包并重启」就行，不用再构建`;
  }
  if (c.behind_built > 0) {
    return `你正在跑的构建落后本地代码 ${c.behind_built} 个提交 —— 点「重新构建当前代码」才会生效`;
  }
  if (c.running_dirty) {
    return "当前构建是带未提交改动编出来的，和同名版本不是一个东西";
  }
  return null;
}

/**
 * 构建按钮能不能点。
 *
 * **`blocked` 有值时一律不能点** —— 那几条（工作区脏、分叉、找不到仓库）
 * 点下去必定失败，让按钮亮着只会让人试三次。
 */
export function canBuild(stage: Stage, check: UpdateCheck | null): boolean {
  if (stage === "building" || stage === "checking") return false;
  if (!check) return false;
  if (check.blocked) return false;
  // 已经是最新 / 已经构建好只差换包：再编一遍是同一个版本
  return !upToDate(check) && !bundleReady(check);
}

/**
 * 换包按钮能不能点 —— **只有真的有一个构建好的新包才行**：
 * 刚构建完，或者查的时候发现磁盘上的包就是当前代码编的（构建完没换、又点了一次「检查更新」）。
 */
export function canApply(stage: Stage, check: UpdateCheck | null = null): boolean {
  if (stage === "ready") return true;
  return stage === "checked" && bundleReady(check);
}

/** 进度事件 → 新阶段 */
export function nextStage(current: Stage, p: UpdateProgress): Stage {
  if (p.phase === "failed") return "failed";
  if (p.phase === "done") return "ready";
  return current === "building" ? "building" : "building";
}

/** 查完之后的一句话。**没有更新也要说清楚**，不能什么都不显示 */
export function checkSummary(c: UpdateCheck): string {
  if (c.blocked) return c.blocked;
  if (c.behind > 0) return `有 ${c.behind} 个新提交`;
  if (upToDate(c)) return `已经是最新的：正在跑的就是 ${c.branch} 的最新代码${c.head_sha ? `（${c.head_sha}）` : ""}`;
  return `远端没有新提交（${c.branch}）`;
}
