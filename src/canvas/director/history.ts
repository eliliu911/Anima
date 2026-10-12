/**
 * 导演世界编辑器的撤销 / 重做。
 *
 * 只记人偶列表（相机、画幅不进撤销栈 —— 转个视角不该占一步撤销）。
 *
 * **连续的同类操作合并成一步**：拖滑块、连按方向键微调，每一下都记的话，
 * 撤销要按几十次才回到拖之前。同一个 `kind`、间隔不到 `mergeMs` 的记录合并。
 */
export interface History<T> {
  /** 改之前调，传改之前的值 */
  record(before: T, kind?: string, now?: number): void;
  undo(current: T): T | null;
  redo(current: T): T | null;
  canUndo(): boolean;
  canRedo(): boolean;
}

export function createHistory<T>(limit = 100, mergeMs = 500): History<T> {
  const past: T[] = [];
  const future: T[] = [];
  let lastKind: string | null = null;
  let lastAt = -Infinity;
  return {
    record(before, kind, now = Date.now()) {
      const merge = kind != null && kind === lastKind && now - lastAt < mergeMs;
      lastKind = kind ?? null;
      lastAt = now;
      future.length = 0;
      if (merge) return;
      past.push(before);
      if (past.length > limit) past.shift();
    },
    undo(current) {
      const prev = past.pop();
      if (prev === undefined) return null;
      future.push(current);
      lastKind = null;
      return prev;
    },
    redo(current) {
      const next = future.pop();
      if (next === undefined) return null;
      past.push(current);
      lastKind = null;
      return next;
    },
    canUndo: () => past.length > 0,
    canRedo: () => future.length > 0,
  };
}
