import type { ScriptDocument } from "../../../api/scriptLibrary";

/**
 * 剧本库列表的排序。
 *
 * # 为什么不能直接 `localeCompare`
 *
 * 字典序把「第10章」排在「第2章」**前面**（因为 `1 < 2`）。
 * 章节和分集天然是数字序 —— 而排错顺序的目录比不排序更糟，
 * 因为它**看起来**是排过的。
 *
 * 所以按「数字段按数值比、非数字段按文本比」来 —— 通称自然序。
 */

/** 把字符串切成「文本段 / 数字段」交替的序列。 */
function chunks(value: string): (string | number)[] {
  const out: (string | number)[] = [];
  const re = /(\d+)|(\D+)/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(value))) {
    out.push(m[1] !== undefined ? Number(m[1]) : m[2]);
  }
  return out;
}

/**
 * 自然序比较。`第2章` < `第10章`，`S01E02` < `S01E10`。
 *
 * 数字段一律按数值比，所以前导零无所谓（`第01章` 和 `第1章` 等价）。
 */
export function naturalCompare(a: string, b: string): number {
  const x = chunks(a ?? "");
  const y = chunks(b ?? "");
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    const p = x[i];
    const q = y[i];
    if (p === undefined) return -1;
    if (q === undefined) return 1;
    if (typeof p === "number" && typeof q === "number") {
      if (p !== q) return p - q;
    } else {
      const r = String(p).localeCompare(String(q), "zh");
      if (r !== 0) return r;
    }
  }
  return 0;
}

/**
 * 按什么排。
 *
 * **方向不让用户选。** 每种排法只有一个说得通的方向：名字按顺序看、
 * 改动时间看最近的、字数看最长的。给一个 ▲▼ 只是多一次要想的事，
 * 而且想错了列表就乱了。
 */
export type SortKey = "name" | "updated" | "size";

export const SORT_OPTIONS: { id: SortKey; label: string }[] = [
  { id: "name", label: "按名称" },
  { id: "updated", label: "最近改动" },
  { id: "size", label: "字数最多" },
];

export const DEFAULT_SORT: SortKey = "name";

/**
 * 排一份列表出来。**不改原数组** —— 传进来的那份是 store 里的。
 *
 * 同名/同值时退回名称自然序，保证顺序**稳定**：不退的话两次渲染
 * 可能给出不同的排法，用户会看到列表自己在跳。
 */
export function sortDocs(docs: ScriptDocument[], key: SortKey): ScriptDocument[] {
  const byName = (a: ScriptDocument, b: ScriptDocument) =>
    naturalCompare(a.title ?? "", b.title ?? "");
  return [...docs].sort((a, b) => {
    if (key === "updated") {
      const d = (b.updated_at ?? 0) - (a.updated_at ?? 0);
      return d !== 0 ? d : byName(a, b);
    }
    if (key === "size") {
      const d = (b.text?.length ?? 0) - (a.text?.length ?? 0);
      return d !== 0 ? d : byName(a, b);
    }
    return byName(a, b);
  });
}
