import { invoke } from "@tauri-apps/api/core";

/** 系统里一个正在挡睡眠的进程。和 Rust `keep_awake::SleepBlocker` 同形 */
export interface SleepBlocker {
  pid: number;
  /** 进程名，如 `caffeinate`、`Claude` */
  process: string;
  name: string;
  /** 还剩多少秒自动松手；null = 没有时限 */
  secs_left: number | null;
  /** `caffeinate -w <pid>` 代哪个进程按着 */
  on_behalf_of: number | null;
  /** Anima 自己按的 */
  anima: boolean;
}

export interface KeepAwakeStatus {
  /** Anima 因为有任务在跑正按着 */
  anima_holding: boolean;
  /** 从什么时候开始按的（秒） */
  since: number | null;
  /** 任务记录还说在跑，但连续 stall_secs 没有任何动静，松手了（多半挂住了；一有新进度会重新按住） */
  stalled: boolean;
  /** 最近一次看到任务还活着（秒）。从没看到过为 null */
  last_alive: number | null;
  /** 连续多久没动静才松手（秒） */
  stall_secs: number;
  /** 系统里所有在挡系统睡眠的（Anima 的、用户在终端开的 caffeinate、别的应用） */
  blockers: SleepBlocker[];
}

/** 防休眠现在的样子：读的是系统真实的断言（`pmset -g assertions`），不只是 Anima 自己记的 */
export const keepAwakeStatus = () => invoke<KeepAwakeStatus>("keep_awake_status");
