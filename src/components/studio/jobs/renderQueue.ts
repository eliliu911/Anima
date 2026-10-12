/**
 * 排队名单的拖动（纯函数，好测）。拖的单位是**组**：要接上一镜的几镜在同一组里，只能一起挪 ——
 * 后端 `render_queue::check_order` 还会再查一遍。
 */

/** 把第 `from` 组挪到第 `to` 个位置之前（`to` 是挪之前的下标；`to === groups.length` = 放到最后） */
export function moveGroup<T>(groups: T[][], from: number, to: number): T[][] {
  if (from < 0 || from >= groups.length || to < 0 || to > groups.length || to === from || to === from + 1) return groups;
  const next = groups.slice();
  const [g] = next.splice(from, 1);
  next.splice(to > from ? to - 1 : to, 0, g);
  return next;
}

/** 分组铺平成后端要的一列 id */
export function flatIds(groups: { id: string }[][]): string[] {
  return groups.flat().map((i) => i.id);
}

/** 暂停位的 id 前缀。见 Rust `render_queue::HOLD_PREFIX` */
export const HOLD_PREFIX = "hold:";

interface BatchLike {
  kind: string;
  status: string;
  targets?: string[];
  progress?: { done: number; total: number; label: string } | null;
}

/** 批量正停在暂停位上：回暂停位的说明（「冒烟测试」）；没停回 `null`。后端停下时进度标签写的是「暂停 · <说明>」 */
export function holdLabel(job: BatchLike): string | null {
  if (job.kind !== "video_batch" || job.status !== "running") return null;
  const label = job.progress?.label ?? "";
  return label.startsWith("暂停 · ") ? label.slice("暂停 · ".length) : null;
}

/**
 * 名单里**镜头**的进度（暂停位不算镜头）：`position` = 正在出的是第几镜（停在暂停位上时 = 已出完的镜数），
 * `total` = 一共几镜，`left` = 还没出的镜数（不含正在出的那一镜）。名单还没记的老任务退回进度里的数。
 */
export function shotCounts(job: BatchLike): { position: number; total: number; left: number } {
  const p = job.progress;
  const targets = job.targets ?? [];
  if (!p || targets.length === 0) {
    const total = p?.total ?? 0;
    return { position: Math.min((p?.done ?? 0) + 1, total), total, left: Math.max(total - (p?.done ?? 0) - 1, 0) };
  }
  const queued = p.label.startsWith("排队中");
  const cur = queued ? -1 : p.done;
  const isShot = (id: string) => !id.startsWith(HOLD_PREFIX);
  const total = targets.filter(isShot).length;
  const upTo = targets.slice(0, cur + 1).filter(isShot).length;
  const onShot = cur >= 0 && cur < targets.length && isShot(targets[cur]);
  const left = targets.slice(cur + 1).filter(isShot).length;
  return { position: onShot ? upTo : upTo, total, left };
}
