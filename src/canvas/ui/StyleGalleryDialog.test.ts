import { describe, expect, it } from "vitest";
import type { LoraOption } from "../../api/studio";
import { ADULT_TAB, ALL_TAB, PROMPT_TAB, galleryLoras, galleryTabs, togglePromptStyle } from "./StyleGalleryDialog";

const lo = (name: string, o: Partial<LoraOption>): LoraOption => ({
  name, label: name, arch: "zimage", engines: ["zimage"], triggers: [], confirmed_triggers: [], concepts: [],
  note: null, nsfw: false, category: "", source_url: "", thumb: null, ...o,
});
const all = [
  lo("style.safetensors", { category: "画风" }),
  lo("girl.safetensors", { category: "人像" }),
  lo("nsfw.safetensors", { category: "人体", nsfw: true }),
  lo("krea.safetensors", { category: "画风", engines: ["krea2"] }),
  lo("odd.safetensors", { category: "" }),
];

// 用户 2026-09-25：风格窗口「仅显示当前节点上模型对应的 LoRA」「成人向单独一个分类、默认收起」
describe("风格窗口", () => {
  it("只列当前引擎能挂的", () => {
    expect(galleryLoras(all, "krea2", ALL_TAB).map((l) => l.name)).toEqual(["krea.safetensors"]);
    expect(galleryLoras(all, "zimage", ALL_TAB).map((l) => l.name)).not.toContain("krea.safetensors");
  });

  it("成人向不混进「全部」和别的分类，只在成人向里", () => {
    expect(galleryLoras(all, "zimage", ALL_TAB).map((l) => l.name)).not.toContain("nsfw.safetensors");
    expect(galleryLoras(all, "zimage", "人体")).toEqual([]);
    expect(galleryLoras(all, "zimage", ADULT_TAB).map((l) => l.name)).toEqual(["nsfw.safetensors"]);
  });

  it("分类标签：只列有东西的，没分过的归「其它」，提示词风格一直在，成人向勾了才有", () => {
    const off = galleryTabs(all, "zimage", false, true).map((t) => t.id);
    expect(off).toEqual([ALL_TAB, "画风", "人像", "其它", PROMPT_TAB]);
    expect(galleryTabs(all, "zimage", true, true).map((t) => t.id)).toContain(ADULT_TAB);
    expect(galleryTabs(all, "zimage", false, true)[0].count).toBe(3); // 不算成人向
  });

  it("「角色卡」是一个分类（设定卡 / 参考表那类 LoRA）", () => {
    const sheets = [...all, lo("sheet.safetensors", { category: "角色卡" })];
    expect(galleryTabs(sheets, "zimage", false, true).map((t) => t.id)).toContain("角色卡");
    expect(galleryLoras(sheets, "zimage", "角色卡").map((l) => l.name)).toEqual(["sheet.safetensors"]);
  });

  it("引擎不能挂 LoRA：只剩提示词风格", () => {
    expect(galleryTabs(all, "t2v_only", false, false).map((t) => t.id)).toEqual([PROMPT_TAB]);
  });

  it("搜索名字 / 备注 / 激活词", () => {
    const withNote = [...all, lo("cyber.safetensors", { note: "赛博机甲", confirmed_triggers: ["Cyber Bob"] })];
    expect(galleryLoras(withNote, "zimage", ALL_TAB, "机甲").map((l) => l.name)).toEqual(["cyber.safetensors"]);
    expect(galleryLoras(withNote, "zimage", ALL_TAB, "cyber bob").map((l) => l.name)).toEqual(["cyber.safetensors"]);
  });
});

describe("提示词风格：点一下加、再点拿掉", () => {
  const style = "BJD 3D 国漫质感，柔和体积光";
  it("加在后面，用逗号接", () => {
    expect(togglePromptStyle("一个女孩在窗边", style)).toBe(`一个女孩在窗边，${style}`);
    expect(togglePromptStyle("", style)).toBe(style);
  });
  it("再点拿掉，不动其它内容", () => {
    expect(togglePromptStyle(`一个女孩在窗边，${style}`, style)).toBe("一个女孩在窗边");
    expect(togglePromptStyle(togglePromptStyle("开头", style), style)).toBe("开头");
  });
});
