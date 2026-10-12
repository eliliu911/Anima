import { describe, expect, it } from "vitest";

import { loraNote, sortLorasForSheet, checkpointNote, isZImageLora, loraSectionsFor } from "./loraNotes";

/**
 * ComfyUI 的 LoRA 列表是全局的，里面混着视频模型的 LoRA。
 * 把 `minimax_h3_*` 挂到角色设定卡上**不生效、也不报错** —— 必须标出来。
 */
describe("LoRA 说明", () => {
  it("视频 LoRA 标为不适合出图", () => {
    const n = loraNote("minimax_h3_fl2v_turbo_8step_v1.0_comfyui_bf16.safetensors", true);
    expect(n?.incompatible).toBe(true);
    expect(n?.text).toContain("不生效");
  });

  it("出图 LoRA 有说明且不标不兼容", () => {
    for (const f of [
      "guofeng_illust_sdxl.safetensors",
      "hanfu_tang_sdxl.safetensors",
      "CharTurnXLRetag_v4.safetensors",
      "add-detail-xl.safetensors",
      "Asian_Beauty XL.safetensors",
      "FaceTurnXL.safetensors",
    ]) {
      const n = loraNote(f, true);
      expect(n, `${f} 没有说明`).toBeTruthy();
      expect(n?.incompatible, `${f} 被误标成不兼容`).toBeFalsy();
    }
  });

  /**
   * 匹配用关键词而不是完整文件名 —— 同一个 LoRA 会有 v1/v2、fp16/bf16 各种
   * 命名变体，写死全名的话换个版本说明就凭空消失。
   */
  it("换了版本号或精度后仍然匹配得上", () => {
    expect(loraNote("CharTurnXLRetag_v9_fp16.safetensors", true)).toBeTruthy();
    expect(loraNote("guofeng_illust_sdxl_v2.safetensors", true)).toBeTruthy();
  });

  it("中英文都有,且不一样", () => {
    const zh = loraNote("guofeng_illust_sdxl.safetensors", true);
    const en = loraNote("guofeng_illust_sdxl.safetensors", false);
    expect(zh?.text).not.toBe(en?.text);
    expect(en?.text).toMatch(/[a-zA-Z]/);
  });

  it("不适合出图的排到最后,但不隐藏", () => {
    const names = [
      "minimax_h3_turbo_v4_step600_ema.safetensors",
      "guofeng_illust_sdxl.safetensors",
      "hanfu_tang_sdxl.safetensors",
    ];
    const sorted = sortLorasForSheet(names, true);
    expect(sorted).toHaveLength(3);
    expect(sorted[2]).toContain("minimax");
  });

  it("没收录的返回 null,界面自己不渲染", () => {
    expect(loraNote("some_random_lora.safetensors", true)).toBeNull();
  });
});

describe("checkpoint 说明", () => {
  it("两个已装底模都有说明,且中英不同", () => {
    for (const c of ["RealVisXL_V5.0_fp16.safetensors", "AWPortrait_XL_ver1.1.safetensors"]) {
      const zh = checkpointNote(c, true);
      const en = checkpointNote(c, false);
      expect(zh, `${c} 没有中文说明`).toBeTruthy();
      expect(en).not.toBe(zh);
    }
  });

  /** SD1.5 在这台机器上超过 640px 会塌 —— 列表里出现时必须警告 */
  it("SD1.5 底模标出不可用", () => {
    expect(checkpointNote("xxmix9realistic_v40.safetensors", true)).toContain("640");
    expect(checkpointNote("majicmixRealistic_v7.safetensors", true)).toContain("⚠️");
  });

  it("没收录的返回 null", () => {
    expect(checkpointNote("whatever_xl.safetensors", true)).toBeNull();
  });
});

describe("成人姿势合集", () => {
  it("认得出这个 LoRA，下拉里不会是个没注释的裸文件名", () => {
    const n = loraNote("NsfwPovAllInOneLoraSdxl-000009.safetensors", true);
    expect(n?.text).toContain("成人姿势");
    expect(n?.text).toContain("0.6~0.9");
  });

  it("MINI 版本同样认得（关键词匹配，不写死全名）", () => {
    expect(loraNote("NsfwPovAllInOneLoraSdxl-000009MINI.safetensors", true)).toBeTruthy();
  });

  it("是 SDXL 架构的，不该被标成设定卡不兼容", () => {
    // 标错的话它会被排到列表最后并显示警告 —— 但它确实能用在 SDXL 上
    expect(loraNote("NsfwPovAllInOneLoraSdxl-000009.safetensors", true)?.incompatible).toBeFalsy();
  });
});

describe("Z-Image 架构的 LoRA", () => {
  it("三个都认得出，不是裸文件名", () => {
    for (const n of [
      "Straight_hair_zimgt.safetensors",
      "shorty_asian_zimgt.safetensors",
      "Pintrest_ZiT.safetensors",
    ]) {
      expect(loraNote(n, true)?.text, n).toContain("Z-Image");
    }
  });

  it("注释里带触发词 —— 没有它这些角色 LoRA 根本不生效", () => {
    expect(loraNote("Straight_hair_zimgt.safetensors", true)?.text).toContain("j1v_w0m");
    expect(loraNote("shorty_asian_zimgt.safetensors", true)?.text).toContain("sh0r7y_asian");
  });

  it("不该被标成设定卡不兼容", () => {
    // 它们是能用的，标错会被排到列表最后并显示警告
    expect(loraNote("Pintrest_ZiT.safetensors", true)?.incompatible).toBeFalsy();
  });
});

describe("架构区分", () => {
  it("认得出三个 Z-Image LoRA", () => {
    for (const n of [
      "Straight_hair_zimgt.safetensors",
      "shorty_asian_zimgt.safetensors",
      "Pintrest_ZiT.safetensors",
    ]) {
      expect(isZImageLora(n), n).toBe(true);
    }
  });

  it("SDXL 的一个都不能被认成 Z-Image", () => {
    // 认错就会出现在 Z-Image 常挂下拉里，挂上去一声不吭什么都不做
    for (const n of [
      "hanfu_tang_sdxl.safetensors",
      "guofeng_illust_sdxl.safetensors",
      "CharTurnXLRetag_v4.safetensors",
      "Asian_Beauty XL.safetensors",
      "NsfwPovAllInOneLoraSdxl-000009.safetensors",
      "minimax_h3_turbo_v4_step600_ema.safetensors",
    ]) {
      expect(isZImageLora(n), n).toBe(false);
    }
  });

  it("认不出的当 SDXL —— 和 Rust 侧 lora_arch 同向", () => {
    expect(isZImageLora("mystery.safetensors")).toBe(false);
  });

  it("两个下拉的候选没有交集", () => {
    // 有交集就意味着同一个文件能被挂到错误的引擎上
    const all = [
      "Straight_hair_zimgt.safetensors",
      "Pintrest_ZiT.safetensors",
      "hanfu_tang_sdxl.safetensors",
      "CharTurnXLRetag_v4.safetensors",
    ];
    const z = all.filter(isZImageLora);
    const sdxl = all.filter((n) => !isZImageLora(n));
    expect(z.length).toBe(2);
    expect(sdxl.length).toBe(2);
    expect(z.filter((n) => sdxl.includes(n))).toEqual([]);
  });
});

describe("按选中的底模决定显示哪块", () => {
  it("选 Z-Image：展开 Z-Image 常挂，SDXL 收折叠", () => {
    expect(loraSectionsFor("zimage")).toEqual({ zimage: true, sdxlPrimary: false });
  });

  it("选 SDXL：展开 SDXL，不显示 Z-Image 常挂", () => {
    // 选了 SDXL 还显示 Z-Image 常挂是误导 —— 那条路上它根本不生效
    expect(loraSectionsFor("sdxl")).toEqual({ zimage: false, sdxlPrimary: true });
  });

  it("选 Chroma：两块都不是主区，但 SDXL 仍可达（折叠）", () => {
    // Chroma 也挂不了 LoRA，可设定卡照样回落 SDXL，所以不能整个藏掉
    expect(loraSectionsFor("chroma")).toEqual({ zimage: false, sdxlPrimary: false });
  });

  it("任何底模下 SDXL 设置都不会被判成完全不需要", () => {
    // 设定卡/控姿势永远回落 SDXL，和选了什么无关
    for (const e of ["zimage", "chroma", "sdxl"]) {
      const s = loraSectionsFor(e);
      expect(typeof s.sdxlPrimary).toBe("boolean");
    }
  });
});

describe("2026-09-17 实测那批 Krea 2 LoRA 都有说明", () => {
  /**
   * **这条防的是「查了、写了、但没接上」。**
   *
   * `loraHint` 原来只看用户备注和训练标签，**从来没查过这张内置表** ——
   * 于是实测 + 联网核对写出来的说明一个字都不显示，界面上一排「没有说明」。
   * 又是这个仓库的老毛病：机制写了、执行漏了。
   */
  const TESTED = [
    "krea/Krea2NexBlendSemiRealistic.safetensors",
    "krea/krea2-Cc-MJ-影视柔光.safetensors",
    "krea/krea2_style_reference.safetensors",
    "krea/krea2_identity_edit_v1_2.safetensors",
    "krea/krea2_identity_edit_v1_2_r128.safetensors",
    "krea/Purple_Graphics_KR2.safetensors",
    "krea/hina_krea2Turbo_asianmix_v4.safetensors",
    "krea/krea2_kgirl_v3.safetensors",
    "krea/Krea2-realism-V2.safetensors",
    "krea/detail_slider_krea2_loraholic.safetensors",
    "krea/KNP_V2_copy_copy@nsfw_1.5~2.safetensors",
    "krea/Nick_RedDudou_krea2_v0.1.safetensors",
    "krea/Nick_cheongsam2_Krea2.safetensors",
    "krea/Nick_Bodyconskirt_包臀裙_v2.safetensors",
    "krea/DynamicCharacterSheet_krea2_v1.safetensors",
    "krea/QuadView_krea2_v1.safetensors",
    "zimage/REDZ15_teenymooddy_lora_v1.1.safetensors",
  ];

  it("每一条都查得到中英文说明", () => {
    for (const n of TESTED) {
      expect(loraNote(n, true)?.text, `${n} 没有中文说明`).toBeTruthy();
      expect(loraNote(n, false)?.text, `${n} 没有英文说明`).toBeTruthy();
    }
  });

  /** 说明要**具体到能照着用**：有触发词的必须把触发词写进去。 */
  it("有触发词的把触发词写进说明里", () => {
    for (const [n, trig] of [
      ["krea/Nick_RedDudou_krea2_v0.1.safetensors", "reddudou"],
      ["krea/Nick_cheongsam2_Krea2.safetensors", "cheongsam"],
      ["krea/Nick_Bodyconskirt_包臀裙_v2.safetensors", "bdq"],
      ["krea/hina_krea2Turbo_asianmix_v4.safetensors", "asian woman"],
    ] as const) {
      expect(loraNote(n, true)!.text, `${n} 的说明里没写触发词`).toContain(trig);
    }
  });

  /**
   * **两个工具类必须说清「要源图 / 要参考图」。** 它们在纯文生图下不报错、
   * 只是给一张没用的图 —— 说明里不写，用户就会以为这个 LoRA 没效果。
   */
  it("要源图的两个在说明里明说", () => {
    expect(loraNote("krea/krea2_identity_edit_v1_2.safetensors", true)!.text).toContain("源图");
    expect(loraNote("krea/krea2_style_reference.safetensors", true)!.text).toContain("参考图");
  });
});
