import { describe, expect, it } from "vitest";
import { withDisembodied, withVoiceId } from "./disembodied";
import type { CharacterTraits } from "../../../api/assets";

const 有设定的角色: CharacterTraits = {
  persona: "林深",
  trigger_words: "linshen",
  seed: 42,
  lora_model: "zimage/x.safetensors",
  lora_weight: 0.8,
  consistency_mode: "lora",
  clothing_prompt: "深灰色连帽衫",
  voice_id: "voice-7",
  asset_sheet: { pages: 3 },
  visual_dna: { marks: ["左眼下方一颗痣"] },
};

describe("withDisembodied", () => {
  it("只改这一位，别的一位都不动", () => {
    const out = withDisembodied(有设定的角色, true);
    expect(out.disembodied).toBe(true);
    // 逐项对 —— 整份替换的语义下，少带一个字段就是永久丢一个字段。
    expect(out).toEqual({ ...有设定的角色, disembodied: true });
  });

  it("取消勾选也只改这一位", () => {
    const out = withDisembodied({ ...有设定的角色, disembodied: true }, false);
    expect(out.disembodied).toBe(false);
    expect(out.visual_dna).toEqual(有设定的角色.visual_dna);
  });

  it("角色还没有设定时给一份空的，而不是返回 null", () => {
    const out = withDisembodied(null, true);
    expect(out.disembodied).toBe(true);
    expect(out.persona).toBe("");
    expect(out.visual_dna).toBeNull();
  });
});

describe("音色", () => {
  /**
   * `assets_update` 的 `character` 是**整份替换**。随手传 `{voice_id}`
   * 会把设定卡和视觉 DNA 清空 —— 而且不报错，下次出图才发现脸变了。
   */
  it("**别的一位都不能动** —— 设定卡和视觉 DNA 要原样留着", () => {
    const traits = {
      persona: "阎王", trigger_words: "yanwang", seed: 42,
      lora_model: "x.safetensors", lora_weight: 0.8, consistency_mode: "lora",
      clothing_prompt: "黑色宽袖官服", voice_id: null,
      asset_sheet: { turnarounds: [1, 2] }, visual_dna: { face: "方脸" },
      disembodied: true,
    } as never as Parameters<typeof withVoiceId>[0];
    const out = withVoiceId(traits, "yanwang-low");
    expect(out.voice_id).toBe("yanwang-low");
    expect(out.asset_sheet).toEqual({ turnarounds: [1, 2] });
    expect(out.visual_dna).toEqual({ face: "方脸" });
    expect(out.seed).toBe(42);
    expect(out.disembodied).toBe(true);
  });

  it("空串存成 null —— 「配了个空的」和「没配」留一种就够了", () => {
    expect(withVoiceId(null, "").voice_id).toBeNull();
    expect(withVoiceId(null, "   ").voice_id).toBeNull();
    expect(withVoiceId(null, " a ").voice_id).toBe("a");
  });

  it("角色还没有设定时给空壳，不是不传 —— 不传的话存不下去，表现为「点了没反应」", () => {
    const out = withVoiceId(null, "v");
    expect(out.persona).toBe("");
    expect(out.voice_id).toBe("v");
  });
});
