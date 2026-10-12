/**
 * 防休眠标记上写什么。拎出来是为了测得到 —— 这里说错一句（「不会睡」而其实会），用户就放心去睡了，第二天才发现一夜没跑。
 */
import type { KeepAwakeStatus, SleepBlocker } from "../../api/keepAwake";

/** 「17 小时 53 分」「12 分钟」 */
export function hm(secs: number): string {
  const m = Math.max(0, Math.round(secs / 60));
  return m >= 60 ? `${Math.floor(m / 60)} 小时${m % 60 ? ` ${m % 60} 分` : ""}` : `${m} 分钟`;
}

/** 这台机器现在会不会自己睡过去 */
export function awake(s: KeepAwakeStatus): boolean {
  return s.blockers.length > 0;
}

/** Anima 自己那一行 */
export function animaLine(s: KeepAwakeStatus, now: number): string {
  if (s.stalled) {
    const quiet = s.last_alive ? `已经 ${hm(now - s.last_alive)}没有任何进度` : `连续 ${hm(s.stall_secs)}没有任何进度`;
    return `${quiet}，松手了 —— 任务记录还说在跑，多半挂住了，去队列看看；一有新进度会重新按住`;
  }
  if (s.anima_holding && s.since) {
    const at = new Date(s.since * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
    return `有任务在跑，正在阻止休眠（${at} 起，已 ${hm(now - s.since)}；任务跑完自动松手，有进度就一直按着，连续 ${hm(s.stall_secs)}没进度才松手）`;
  }
  return "没有任务在跑，Anima 没在阻止休眠（出片、批量出片一开跑就自动开启，不用手动开）";
}

/** 列表里的一行 */
export function blockerLine(b: SleepBlocker): string {
  const who = b.anima ? "Anima 的（跟着任务）" : b.process === "caffeinate" ? "caffeinate（终端里开的）" : b.process;
  const left = b.secs_left != null ? `还剩 ${hm(b.secs_left)}` : b.on_behalf_of ? `跟着进程 ${b.on_behalf_of}，它退出就松手` : "没有时限";
  return `${who} · ${left}`;
}
