import { describe, expect, it } from "vitest";
import type { PublicScene } from "../../../api/publicLibrary";
import { categoryTabs, defaultAssetName, filterScenes, variantLabel } from "./publicScenes";

const sc = (name: string, category: string, labels: string[]): PublicScene => ({
  id: `c/${category}/${name}`,
  collection: "c",
  category,
  name,
  variants: labels.map((label) => ({ label, path: `/p/${name}-${label}.webp`, url: "" })),
});

const lib = [sc("Classroom 01", "学校", ["白天", "夜晚"]), sc("rooftop", "学校", ["正午"]), sc("beach", "自然", ["日落"]), sc("都市校园 01", "都市校园", ["下午"])];

describe("公共素材库", () => {
  it("分类按数量排，全部在最前", () => {
    expect(categoryTabs(lib).map((t) => `${t.label}${t.count}`)).toEqual(["全部4", "学校2", "都市校园1", "自然1"]);
  });

  it("搜名字、分类、时段；分类筛选叠加搜索", () => {
    expect(filterScenes(lib, "all", "夜晚").map((s) => s.name)).toEqual(["Classroom 01"]);
    expect(filterScenes(lib, "all", "CLASS").length).toBe(1);
    expect(filterScenes(lib, "学校", "").length).toBe(2);
    expect(filterScenes(lib, "自然", "classroom")).toEqual([]);
  });

  it("默认资产名带分类，名字已经以分类开头的不重复", () => {
    expect(defaultAssetName(lib[0])).toBe("学校 · Classroom 01");
    expect(defaultAssetName(lib[3])).toBe("都市校园 01");
    expect(variantLabel("")).toBe("原图");
  });
});
