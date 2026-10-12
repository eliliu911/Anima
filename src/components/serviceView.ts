/**
 * 「服务」页的纯逻辑。
 *
 * 最要紧的一条：**「离线」不等于「有问题」**。
 * CosyVoice2 没开是完全正常的状态，把它标成红的会让人去修一个不存在的问题；
 * 而 ComfyUI 没开是真的会挡住出图。区别在 `optional`。
 */
import type { ServiceStatus } from "../api/services";

export type Level = "ok" | "warn" | "idle";

/** 一个服务该显示成什么颜色 */
export function levelOf(s: ServiceStatus): Level {
  if (s.online) return "ok";
  return s.optional ? "idle" : "warn";
}

/** 按分组归拢，**保持 Rust 侧给的顺序**（按「坏了影响多大」排过） */
export function byGroup(list: ServiceStatus[]): { key: string; label: string; items: ServiceStatus[] }[] {
  const out: { key: string; label: string; items: ServiceStatus[] }[] = [];
  for (const s of list) {
    let g = out.find((x) => x.key === s.group);
    if (!g) {
      g = { key: s.group, label: s.group_label, items: [] };
      out.push(g);
    }
    g.items.push(s);
  }
  return out;
}

/** 顶部那一句总结。**只数真正要紧的** —— 可选服务没开不该算进「异常」 */
export function summary(list: ServiceStatus[]): { text: string; bad: number } {
  const bad = list.filter((s) => levelOf(s) === "warn").length;
  const online = list.filter((s) => s.online).length;
  if (list.length === 0) return { text: "还没探测", bad: 0 };
  if (bad === 0) return { text: `${online}/${list.length} 在线，要紧的都在`, bad: 0 };
  return { text: `${online}/${list.length} 在线，${bad} 个该开的没开`, bad };
}
