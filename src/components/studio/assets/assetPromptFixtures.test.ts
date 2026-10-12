/**
 * 资产出图提示词的**对拍样例**：TS（工作台交互用）和 Rust（`asset_prompt.rs`，Chat 工具和后台批量出图用）
 * 两份实现，拼出来必须一字不差。
 *
 * 样例文件 `shared/asset-prompt-fixtures.json` 由这边的实现生成，Rust 那边读同一个文件核对。
 * 改了提示词拼装时：`UPDATE_ASSET_PROMPT_FIXTURES=1 npx vitest run assetPromptFixtures` 重新生成，
 * 然后改 Rust 让 `cargo test asset_prompt` 过 —— **两边一起改，不许只改一边**。
 */
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { buildPrompt, finalNegative, finalPositive, flattenDescription, screenHint, styleForKind, templateNegative, type AssetKind, type TemplateId } from "./assetPrompt";
import { characterTemplateFor, templateAspect, defaultViews } from "../steps/batchAssets";
import { toViewpoints } from "./sceneViews";
import type { SceneContract } from "../../../api/assets";

const FILE = fileURLToPath(new URL("../../../../shared/asset-prompt-fixtures.json", import.meta.url));

const contract = (anchors: [string, SceneContract["anchors"][number]["size"]][], entrance?: string): SceneContract => ({
  shape: "", zones: [], relations: [], materials: [], views: [], inferred: [], no_extra: [], empty_stage: "required",
  entrances: entrance ? [{ name: entrance, connects_to: "" }] : [],
  anchors: anchors.map(([name, size]) => ({ name, zone: "", size, must_not_become: [] })),
});

const STYLE = "live-action film still, natural human proportions, realistic skin texture, cinematic lighting, 35mm film grain, 温暖色调, 人物表情细腻";

function cases() {
  const out: { fn: string; args: Record<string, unknown>; out: unknown }[] = [];
  const add = (fn: string, args: Record<string, unknown>, value: unknown) => out.push({ fn, args, out: value });
  const descs = ["", "描述：盛唐女官，女性，28岁。\n服装：上衣男装官服，下装官服下摆。\n特征：黑眼圈明显。", "白须，青灰长衫"];
  for (const d of descs) add("flatten_description", { description: d }, flattenDescription(d));
  for (const [name, d] of [["手机", "屏幕亮起显示提示"], ["电脑", "旧款台式机"], ["铜壶", "黄铜"], ["laptop", "screen showing messages"]]) {
    add("screen_hint", { name, description: d }, screenHint(name, d));
  }
  const entities: [AssetKind, string, string][] = [
    ["character", "林婉清", descs[1]], ["character", "皇帝", ""], ["scene", "金銮殿", "描述：盛唐大殿，金色龙柱"], ["prop", "手机", "屏幕亮起显示提示"], ["prop", "铜壶", "黄铜，壶嘴弯"],
  ];
  const templates: TemplateId[] = ["avatar", "simple", "detailed", "panel5", "quadzh"];
  for (const [kind, name, description] of entities) {
    // 道具多一张构图卡（`propcard`，道具信息卡）
    for (const template of kind === "character" ? templates : kind === "prop" ? (["simple", "propcard"] as TemplateId[]) : (["simple"] as TemplateId[])) {
      const prompt = buildPrompt(kind, { name, description }, template);
      add("build_prompt", { kind, name, description, template }, prompt);
      add("template_negative", { kind, template }, templateNegative(kind, template));
      for (const apply of [true, false]) add("final_positive", { prompt, style: STYLE, apply, kind }, finalPositive(prompt, STYLE, apply, kind));
    }
  }
  add("final_positive", { prompt: "一段没有空行的提示词", style: STYLE, apply: true, kind: "character" }, finalPositive("一段没有空行的提示词", STYLE, true, "character"));
  add("final_positive", { prompt: "", style: STYLE, apply: true, kind: "scene" }, finalPositive("", STYLE, true, "scene"));
  for (const kind of ["character", "scene", "prop"] as AssetKind[]) add("style_for_kind", { kind, style: STYLE }, styleForKind(kind, STYLE));
  add("final_negative", { style: "blurry, lowres", composition: "text, labels", apply: true }, finalNegative("blurry, lowres", "text, labels", true));
  add("final_negative", { style: "blurry", composition: "", apply: false }, finalNegative("blurry", "", false));
  for (const engine of ["zimage", "krea2", "sdxl"]) add("character_template_for", { engine }, characterTemplateFor(engine));
  for (const t of templates) add("template_aspect", { template: t }, templateAspect(t));
  const contracts: [string, SceneContract | null][] = [
    ["none", null],
    ["anchors", contract([["朱红巨柱", "wall_dominant"], ["龙椅", "large"], ["香炉", "small"], ["御案", "medium"]], "殿门")],
    ["one", contract([["柜台", "large"]])],
  ];
  for (const [label, c] of contracts) add("default_viewpoints", { contract: label }, toViewpoints(defaultViews(), c));
  return { contracts: Object.fromEntries(contracts), cases: out };
}

describe("资产出图提示词对拍样例", () => {
  it("和 shared/asset-prompt-fixtures.json 一致（Rust 读同一份）", () => {
    const now = cases();
    if (process.env.UPDATE_ASSET_PROMPT_FIXTURES || !existsSync(FILE)) {
      writeFileSync(FILE, JSON.stringify(now, null, 1) + "\n");
    }
    const saved = JSON.parse(readFileSync(FILE, "utf8"));
    expect(now).toEqual(saved);
  });
});
