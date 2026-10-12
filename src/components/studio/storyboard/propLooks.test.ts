import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const read = (rel: string) => readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");

/**
 * 道具也有形象（2026-09-28）：协议铭牌量产款是金属牌，星澜那一块是「星澜专属」款。
 * 以前形象只对角色开放 —— 资产页里道具只有「图片」、分镜详情里道具选不了形象，
 * 后端也只给角色读形象，换了也不生效。后端那半由 Rust 测试钉着，这里钉界面这半。
 */
describe("道具的形象", () => {
  it("资产页：道具和角色一样显示形象条、能加形象；场景仍然只有图片", () => {
    const src = read("../library/AssetInspector.tsx");
    expect(src).toContain('const hasLooks = isCharacter || asset.kind === "prop";');
    expect(src).toMatch(/tab === "looks" && \(hasLooks\s*\? <LooksTab/);
    // 「还没填人物本身」只对角色说 —— 道具没有人物本身
    expect(src).toContain('const noIdentity = asset.kind === "character" && !(asset.identity ?? "").trim();');
  });

  it("分镜详情：出场道具也能换这一镜的形象", () => {
    const src = read("./ShotDetail.tsx");
    const fn = src.slice(src.indexOf("function LookRows"), src.indexOf("return (", src.indexOf("function LookRows")));
    expect(fn).toContain("...(draft.prop_ids ?? [])");
    expect(fn).toContain('a.kind === "character" || a.kind === "prop"');
  });

  it("出图工作台：道具有多个形象时也能选出到哪个形象", () => {
    expect(read("../assets/AssetWorkbench.tsx")).toContain('(asset.kind === "character" || asset.kind === "prop") && looks.length > 1');
  });
});
