import { describe, expect, it } from "vitest";
import { enginesForMode, pickEngine, type CapabilityCatalog } from "./studio";

const cat: CapabilityCatalog = {
  aspects: [{ id: "portrait", label: "竖 832×1216", width: 832, height: 1216 }],
  default_image_engine: "zimage",
  default_video_engine: "ltx",
  modes: [
    { id: "t2i", label: "文生图", short: "文生", kind: "image", images: 0, media_label: null, engines: ["zimage", "sdxl"] },
    { id: "sheet", label: "设定卡", short: "设定", kind: "image", images: 0, media_label: null, engines: ["sdxl"] },
    { id: "t2v", label: "文生视频", short: "文生", kind: "video", images: 0, media_label: null, engines: ["ltx", "h3"] },
    { id: "r2v", label: "参考生", short: "参考生", kind: "video", images: 1, media_label: "参考图片", engines: ["h3"] },
  ],
  engines: [
    { id: "zimage", label: "Z-Image", kind: "image", negative_prompt: false, long_prompt_ok: true, steps: 8, loras: true, secs: 45, notes: ["负向无效"], sizes: [{ label: "a", width: 1024, height: 1024, verified: true, secs: 45, peak_gb: null }], max_batch: 4, reference_slots: 0, project_settings: true, duration: null },
    { id: "sdxl", label: "SDXL", kind: "image", negative_prompt: true, long_prompt_ok: false, steps: 20, loras: true, secs: 39, notes: [], sizes: [{ label: "a", width: 1024, height: 1024, verified: true, secs: 45, peak_gb: null }], max_batch: 4, reference_slots: 0, project_settings: true, duration: null },
    { id: "ltx", label: "LTX", kind: "video", negative_prompt: false, long_prompt_ok: true, steps: null, loras: true, secs: 121, notes: [], sizes: [{ label: "a", width: 1024, height: 1024, verified: true, secs: 45, peak_gb: null }], max_batch: 4, reference_slots: 0, project_settings: true, duration: null },
    { id: "h3", label: "H3", kind: "video", negative_prompt: false, long_prompt_ok: true, steps: null, loras: true, secs: 384, notes: [], sizes: [{ label: "a", width: 1024, height: 1024, verified: true, secs: 45, peak_gb: null }], max_batch: 4, reference_slots: 0, project_settings: true, duration: null },
  ],
};

describe("能力目录", () => {
  it("只列出该模式真正支持的引擎", () => {
    expect(enginesForMode(cat, "sheet").map((e) => e.id)).toEqual(["sdxl"]);
    expect(enginesForMode(cat, "r2v").map((e) => e.id)).toEqual(["h3"]);
    expect(enginesForMode(cat, "不存在的模式")).toEqual([]);
  });

  // 无脑重置 = 每次切模式都要重选；无脑保留 = 留下一个不支持的引擎
  it("换模式时能留就留，留不住才退到默认", () => {
    expect(pickEngine(cat, "t2i", "sdxl")).toBe("sdxl");
    // 设定卡只有 SDXL —— 从 zimage 切过去必须换掉
    expect(pickEngine(cat, "sheet", "zimage")).toBe("sdxl");
    // 图像切视频：退到视频默认，不能留着 zimage
    expect(pickEngine(cat, "t2v", "zimage")).toBe("ltx");
    // 参考生只有 H3，默认的 ltx 也留不住
    expect(pickEngine(cat, "r2v", "ltx")).toBe("h3");
    // 没有当前引擎时用默认
    expect(pickEngine(cat, "t2i", null)).toBe("zimage");
  });
});

import { mergeTriggers } from "./studio";

describe("激活词注入", () => {
  // 静默改提示词是我们自己造的静默失败 —— 必须返回实际加了什么
  it("只补缺的，且告诉调用方加了什么", () => {
    const r = mergeTriggers("老茶师注水", ["j1v_w0m"]);
    expect(r.prompt).toBe("j1v_w0m, 老茶师注水");
    expect(r.added).toEqual(["j1v_w0m"]);
  });

  it("已经写过的不重复加（忽略大小写）", () => {
    const r = mergeTriggers("J1V_W0M 老茶师", ["j1v_w0m"]);
    expect(r.prompt).toBe("J1V_W0M 老茶师");
    expect(r.added).toEqual([]);
  });

  it("空提示词时不留下多余的逗号", () => {
    expect(mergeTriggers("", ["a1_x"]).prompt).toBe("a1_x");
    expect(mergeTriggers("  ", ["a1_x", "b2_y"]).prompt).toBe("a1_x, b2_y");
  });

  it("没有激活词时原样返回", () => {
    const r = mergeTriggers("老茶师", []);
    expect(r.prompt).toBe("老茶师");
    expect(r.added).toEqual([]);
  });
});

import { loraHint, type LoraOption } from "./studio";

const lo = (o: Partial<LoraOption>): LoraOption => ({
  name: "x.safetensors", label: "x", arch: "sdxl", engines: ["sdxl"],
  triggers: [], confirmed_triggers: [], concepts: [], note: null, nsfw: false, category: "", source_url: "", thumb: null, ...o,
});

describe("LoRA 说明", () => {
  it("用户备注优先于训练标签", () => {
    expect(loraHint(lo({ note: "汉服唐风", concepts: ["solo", "still life"] })))
      .toBe("汉服唐风");
  });

  // 训练标签不是作者写的说明 —— 必须标出来源，别让人以为是官方描述
  it("没有备注就给训练标签，并标明来源", () => {
    const h = loraHint(lo({ concepts: ["back view", "side view"] }));
    expect(h).toContain("训练标签");
    expect(h).toContain("back view");
  });

  it("两样都没有时直说没有，而不是编一句", () => {
    const h = loraHint(lo({}));
    expect(h).toContain("没有说明");
  });

  it("空白备注当成没有", () => {
    expect(loraHint(lo({ note: "   ", concepts: ["a"] }))).toContain("训练标签");
  });
});

describe("说明的四级回落", () => {
  /**
   * **内置说明这一级是 2026-09-17 补的。** `components/loraNotes.ts` 那张表
   * 一直存在，但 `loraHint` 从来没查过它 —— 实测 + 联网核对写出来的说明
   * 一个字都不显示，界面上一排「没有说明」。
   */
  it("没有用户备注时，走内置说明而不是直说没有", () => {
    const h = loraHint(lo({ name: "krea/QuadView_krea2_v1.safetensors" }));
    expect(h).not.toContain("没有说明");
    expect(h).toContain("四视图");
  });

  /** 用户自己写的仍然最优先 —— 他比我们清楚他要什么。 */
  it("用户备注压过内置说明", () => {
    const h = loraHint(lo({ name: "krea/QuadView_krea2_v1.safetensors", note: "我改过的" }));
    expect(h).toBe("我改过的");
  });

  /** 内置说明压过训练标签 —— 标签是猜的，说明是查过的。 */
  it("内置说明压过训练标签", () => {
    const h = loraHint(lo({
      name: "krea/QuadView_krea2_v1.safetensors",
      concepts: ["solo", "standing"],
    }));
    expect(h).not.toContain("训练标签");
  });

  /** 没收录的还是回落到原来那条路，别因为加了一级就把老行为改掉。 */
  it("没收录的仍然给训练标签", () => {
    const h = loraHint(lo({ name: "krea/从来没见过的东西.safetensors", concepts: ["back view"] }));
    expect(h).toContain("训练标签");
  });
});
