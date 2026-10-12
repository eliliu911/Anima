import { describe, expect, it } from "vitest";
import { effectiveGen, toOverrides } from "./effectiveGen";
import type { ModelSettings } from "../../../api/cinema";

const P: ModelSettings = {
  t2i_engine: "zimage",
  i2i_engine: "zimage",
  video_engine: "h3",
  character_aspect: "portrait",
  scene_aspect: "landscape",
  prop_aspect: "square",
};

describe("出图配置：项目设置 → 资产覆盖", () => {
  it("没覆盖时全跟项目走", () => {
    const g = effectiveGen("character", null, P);
    expect(g.engine).toBe("zimage");
    expect(g.aspect).toBe("portrait");
    expect(g.overridden).toEqual({ engine: false, aspect: false, loras: false });
  });

  it("三类资产各拿各的画幅", () => {
    expect(effectiveGen("scene", null, P).aspect).toBe("landscape");
    expect(effectiveGen("prop", null, P).aspect).toBe("square");
  });

  it("覆盖了就用覆盖的，并标出来是覆盖的", () => {
    const g = effectiveGen("character", { engine: "sdxl", aspect: "square" }, P);
    expect(g.engine).toBe("sdxl");
    expect(g.aspect).toBe("square");
    expect(g.overridden.engine).toBe(true);
    expect(g.overridden.aspect).toBe(true);
  });

  /** 覆盖值和项目设置**碰巧一样**时不该标成「覆盖」—— 那会让人以为改过。 */
  it("覆盖值和项目一样时不算覆盖", () => {
    const g = effectiveGen("character", { engine: "zimage" }, P);
    expect(g.overridden.engine).toBe(false);
  });

  /**
   * **`[]` 和 `undefined` 不是一回事。**
   * 不分开的话，用户把 LoRA 全取消之后下次打开又冒出来了，
   * 而且没有任何地方告诉他为什么。
   */
  it("空数组是「明确不挂」，undefined 是「没设过」", () => {
    expect(effectiveGen("character", { loras: [] }, P).overridden.loras).toBe(true);
    expect(effectiveGen("character", {}, P).overridden.loras).toBe(false);
  });

  it("项目设置还没到时也给得出一份能用的", () => {
    const g = effectiveGen("character", null, null);
    expect(g.engine).toBe("zimage");
    expect(g.aspect).toBe("portrait");
  });
});

describe("收成覆盖", () => {
  /**
   * **和项目设置一样的那一项不存。** 存下来的话，以后改了项目设置，
   * 这个资产会莫名其妙不跟着变，而用户根本不记得自己「设」过它。
   */
  it("和项目一样的不存", () => {
    const o = toOverrides("character", { engine: "zimage", aspect: "portrait", loras: [] }, P);
    expect(o).toEqual({ engine: undefined, aspect: undefined, loras: undefined });
  });

  it("不一样的才存", () => {
    const o = toOverrides("scene", { engine: "sdxl", aspect: "landscape", loras: [] }, P);
    expect(o.engine).toBe("sdxl");
    expect(o.aspect).toBeUndefined();
  });

  it("挂了 LoRA 就存下来", () => {
    const l = [{ name: "zimage/a.safetensors", strength: 0.8 }];
    expect(toOverrides("character", { engine: "zimage", aspect: "portrait", loras: l }, P).loras)
      .toEqual(l);
  });

  /** 存完再读回来必须还是同一份 —— 不然「保存了但没生效」。 */
  it("存了再读回来结果一致", () => {
    const chosen = {
      engine: "sdxl",
      aspect: "square",
      loras: [{ name: "zimage/a.safetensors", strength: 0.7 }],
    };
    const back = effectiveGen("character", toOverrides("character", chosen, P), P);
    expect(back.engine).toBe(chosen.engine);
    expect(back.aspect).toBe(chosen.aspect);
    expect(back.loras).toEqual(chosen.loras);
  });
});
