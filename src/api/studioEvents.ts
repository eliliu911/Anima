import { listen } from "@tauri-apps/api/event";

/**
 * Studio 数据被 Studio 页面以外的地方（Anima Chat 的工具）改了。
 * 和 Rust `studio_ops::CHANGED_EVENT` 对应。页面听到对得上的就重新读 ——
 * 这些页面平时只在打开时读一次，不听的话 Chat 建完项目得手动切页才看得到。
 */
export const STUDIO_CHANGED = "studio://changed";

export type StudioChangeKind = "project" | "episode" | "assets" | "style" | "script_doc";

export interface StudioChange {
  kind: StudioChangeKind;
  project_id?: string;
}

export function onStudioChanged(cb: (c: StudioChange) => void): Promise<() => void> {
  return listen<StudioChange>(STUDIO_CHANGED, (e) => cb(e.payload));
}

/**
 * 这次改动和这个页面有没有关系。
 * - `kinds` 空 = 什么种类都算
 * - `projectId` 给了：改动没带项目（全局资产）也算，带了就要对得上
 */
export function affects(change: StudioChange, opts: { kinds?: StudioChangeKind[]; projectId?: string }): boolean {
  if (opts.kinds?.length && !opts.kinds.includes(change.kind)) return false;
  if (opts.projectId && change.project_id && change.project_id !== opts.projectId) return false;
  return true;
}
