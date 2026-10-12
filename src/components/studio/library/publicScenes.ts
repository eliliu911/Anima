import type { PublicScene } from "../../../api/publicLibrary";

/** 分类页签：按素材数从多到少，「全部」在最前 */
export function categoryTabs(scenes: readonly PublicScene[]): { id: string; label: string; count: number }[] {
  const counts = new Map<string, number>();
  for (const s of scenes) counts.set(s.category, (counts.get(s.category) ?? 0) + 1);
  const cats = [...counts.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0], "zh"));
  return [{ id: "all", label: "全部", count: scenes.length }, ...cats.map(([c, n]) => ({ id: c, label: c, count: n }))];
}

/** 分类 + 搜索（名字、分类、时段都搜得到：搜「夜晚」出所有有夜景的） */
export function filterScenes(scenes: readonly PublicScene[], category: string, query: string): PublicScene[] {
  const q = query.trim().toLowerCase();
  return scenes.filter((s) => {
    if (category !== "all" && s.category !== category) return false;
    if (!q) return true;
    return [s.name, s.category, ...s.variants.map((v) => v.label)].some((t) => t.toLowerCase().includes(q));
  });
}

/** 时段标签给人看的写法：没写时段的叫「原图」 */
export const variantLabel = (label: string) => label || "原图";

/**
 * 加入时默认的资产名：分类 + 名字（「学校 · Classroom 01」）。
 * 名字会被 @ 引用、会进分镜，所以弹窗里让人改成自己剧里的叫法（「教室」）。
 */
export function defaultAssetName(s: PublicScene): string {
  return s.name.startsWith(s.category) ? s.name : `${s.category} · ${s.name}`;
}
