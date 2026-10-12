/**
 * 应用自更新。**走「本机重新构建」那条路** —— 装的人有完整仓库，
 * 不需要签名密钥和分发端点。细节见 `src-tauri/src/updater.rs` 的模块注释。
 */
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export const UPDATER_EVENT = "updater://progress";

export interface UpdateCheck {
  repo: string | null;
  branch: string;
  /** 本地 HEAD 落后 origin 几个提交 */
  behind: number;
  commits: string[];
  /** 正在跑的这个构建是从哪个提交编出来的（可能带 `-dirty`） */
  running_sha: string;
  /**
   * **正在跑的构建落后本地代码几个提交。**
   *
   * 这一段才是最常踩的：拉过代码 / 自己改过，但没重新构建。
   * 只比 HEAD↔origin 的话它永远是 0 个提示都没有 ——
   * 2026-09-18 用户当场撞上过：面板说「已经是最新的」，
   * 而他跑的二进制比磁盘上的代码旧两个提交。
   */
  behind_built: number;
  built_commits: string[];
  /** 当前构建是带未提交改动编出来的 */
  running_dirty: boolean;
  /** 本地仓库 HEAD（短 sha） */
  head_sha: string;
  /**
   * **磁盘上构建好的包就是用当前 HEAD 编的，只差换包。**
   *
   * 为真时不该再让人「重新构建」—— 那是把同一份代码再编十几分钟（2026-09-28 用户指出）。
   */
  bundle_ready: boolean;
  /** 有值就不能更新，而且这句话要直接显示给用户 */
  blocked: string | null;
}

export type UpdatePhase = "git" | "frontend" | "cargo" | "link" | "bundle" | "tick" | "done" | "failed";

export interface UpdateProgress {
  phase: UpdatePhase;
  label: string;
  /** 已编译的 crate 数。**不是百分比** —— cargo 不告诉你总数，凑一个只会骗人 */
  crates: number;
  /** **原始输出行**，给日志窗用。空 = 这条是心跳，没有新输出 */
  line: string;
  /** 从开始构建到现在多少秒。**链接期完全静默，全靠它证明还活着** */
  elapsed: number;
}

/** 看落后几个提交。没网时 behind 是 0，不会误报「有更新」 */
export async function updaterCheck(): Promise<UpdateCheck> {
  return invoke<UpdateCheck>("updater_check");
}

/** 拉代码 + 构建。**不换包** —— 换包要先退出应用，是 `updaterApply` */
export async function updaterBuild(): Promise<void> {
  return invoke<void>("updater_build");
}

/** 换包并重启。**会退出当前应用**，调用前要让用户确认 */
export async function updaterApply(): Promise<void> {
  return invoke<void>("updater_apply");
}

export function onUpdateProgress(handler: (p: UpdateProgress) => void) {
  return listen<UpdateProgress>(UPDATER_EVENT, (e) => handler(e.payload));
}
