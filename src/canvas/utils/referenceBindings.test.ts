import { describe, expect, it } from "vitest";
import type { AssetView } from "../../api/assets";
import { assetReference, addReference } from "./referenceBindings";

const asset = {
  id: "hero", name: "主角", scope_label: "项目 A / 第 2 集",
  scope: { kind: "episode", project_id: "a", episode_id: "2" },
  unit: { selected_image_id: "v2", image_variants: [{ id: "v1" }, { id: "v2" }] },
  image_paths: ["/old.png", "/chosen.png"],
} as AssetView;

describe("canvas reference binding", () => {
  it("uses selected variant path and retains asset identity and scope", () => {
    expect(assetReference(asset)).toEqual({ path: "/chosen.png", name: "主角 · 项目 A / 第 2 集", assetId: "hero", scope: asset.scope, variantId: "v2" });
  });
  it("does not silently pick another version when selection is missing", () => {
    expect(() => assetReference({ ...asset, unit: { ...asset.unit, selected_image_id: "missing" } })).toThrow("选中图片");
  });
  it("rejects missing local paths instead of substituting a preview URL", () => {
    expect(() => assetReference({ ...asset, image_paths: [] })).toThrow("选中图片");
  });
  it("deduplicates paths and rejects overflow", () => {
    const first = assetReference(asset);
    expect(addReference([first], first, 1)).toEqual([first]);
    expect(() => addReference([first], { path: "/other.png", name: "Other" }, 1)).toThrow("最多绑定");
  });
});
