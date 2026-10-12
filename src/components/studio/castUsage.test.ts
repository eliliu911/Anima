import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import type { ResolvedRef } from "../../api/assets";
import { castUsage, castUsageNote, R2V_IMAGE_LIMIT } from "./castUsage";

const ref = (
  name: string,
  opts: Partial<ResolvedRef> = {},
): ResolvedRef => ({
  kind: "character",
  name,
  asset_id: `id_${name}`,
  description: "",
  image_path: `/tmp/${name}.png`,
  image_url: `/tmp/${name}.png`,
  ...opts,
});

const shot = (mode: "first_frame" | "reference", character_id = "id_小花") =>
  ({ mode, character_id }) as const;

describe("这一镜的参考图有几张真会被用上", () => {
  /**
   * **默认这条路一张都不用。** 界面上头像行照样排着带图的圆片，
   * 用户理所当然以为锁住了长相 —— 而 Z-Image 首帧是文生图，
   * 那张图一个像素都没进去，出来的猫和上一镜不是同一只，且不报错。
   */
  it("首帧 + Z-Image：一张都用不上", () => {
    const u = castUsage([ref("小花"), ref("纸箱", { kind: "prop" })], shot("first_frame"), "zimage");
    expect(u).toEqual({ withImage: 2, used: 0, unused: ["小花", "纸箱"] });
    expect(castUsageNote(u, "first_frame")).toContain("多参考 R2V");
  });

  it("首帧 + Chroma / Krea 2 也一样", () => {
    for (const e of ["chroma", "krea2"]) {
      expect(castUsage([ref("小花")], shot("first_frame"), e).used).toBe(0);
    }
  });

  /** SDXL 只拿 `character_id` 那个角色的 avatar 锁脸，场景道具从不参与。 */
  it("首帧 + SDXL：只用主角那一张", () => {
    const u = castUsage(
      [ref("小花"), ref("大橘"), ref("纸箱", { kind: "prop" })],
      shot("first_frame", "id_小花"),
      "sdxl",
    );
    expect(u.used).toBe(1);
    expect(u.unused).toEqual(["大橘", "纸箱"]);
    expect(castUsageNote(u, "first_frame")).toContain("只锁了主角的脸");
  });

  it("首帧 + SDXL 但主角没绑：还是零", () => {
    const u = castUsage([ref("大橘")], shot("first_frame", "id_小花"), "sdxl");
    expect(u.used).toBe(0);
  });

  it("参考模式：全用上，没有可说的", () => {
    const u = castUsage([ref("小花"), ref("小雨")], shot("reference"), "zimage");
    expect(u).toEqual({ withImage: 2, used: 2, unused: [] });
    expect(castUsageNote(u, "reference")).toBeNull();
  });

  /** 上限和 Rust 侧 `assets::reference_images` 对齐，超了要说出来。 */
  it("参考模式超过上限：多出来的说清楚", () => {
    const refs = ["a", "b", "c", "d", "e"].map((n) => ref(n));
    const u = castUsage(refs, shot("reference"), "zimage");
    expect(u.used).toBe(R2V_IMAGE_LIMIT);
    expect(u.unused).toEqual(["e"]);
    expect(castUsageNote(u, "reference")).toContain("e");
  });

  /** 参考生图首帧：用了几张、挤掉谁，**照抄后端挑的**，不在前端重排。 */
  it("首帧 + 参考生图：按后端挑的结果说，槽位满了点名", () => {
    const refs = [ref("大殿", { kind: "scene" }), ref("皇帝"), ref("太监")];
    const u = castUsage(refs, shot("first_frame"), "krea2_edit", { names: ["大殿", "皇帝"], unused: ["太监"] });
    expect(u).toEqual({ withImage: 3, used: 2, unused: ["太监"], slotsFull: true });
    expect(castUsageNote(u, "first_frame")).toContain("首帧最多挂 4 张");
    expect(castUsageNote(u, "first_frame")).toContain("太监 的图没挂");
    const fits = castUsage(refs.slice(0, 2), shot("first_frame"), "qwen21_edit", { names: ["大殿", "皇帝"], unused: [] });
    expect(castUsageNote(fits, "first_frame"), "都放下了就别报警").toBeNull();
  });

  /** 没图的引用不算在里面 —— 那是 CastRow 另一条提示管的事。 */
  it("没图的引用不参与计数", () => {
    const u = castUsage(
      [ref("小花", { image_path: null }), ref("没入库", { asset_id: null })],
      shot("first_frame"),
      "zimage",
    );
    expect(u).toEqual({ withImage: 0, used: 0, unused: [] });
    expect(castUsageNote(u, "first_frame")).toBeNull();
  });

  /** 提示必须带出路 —— 「用不上」本身不可操作。 */
  it("每条提示都给得出下一步", () => {
    const first = castUsageNote(castUsage([ref("小花")], shot("first_frame"), "zimage"), "first_frame");
    expect(first).toMatch(/切「多参考 R2V」/);
    const over = castUsageNote(
      castUsage(["a", "b", "c", "d", "e"].map((n) => ref(n)), shot("reference"), "zimage"),
      "reference",
    );
    expect(over).toMatch(/删掉几个引用|拆成两镜/);
  });
});

describe("这条提示真的接到界面上了", () => {
  const CAST = readFileSync(
    fileURLToPath(new URL("./CastRow.tsx", import.meta.url)),
    "utf8",
  );

  it("CastRow 真的渲染了这条提示", () => {
    expect(CAST).toContain("castUsageNote");
    expect(CAST, "参考生图的挑选必须问后端，不能在前端另排一份").toContain("cinemaFrameReferencePick");
    const at = CAST.indexOf("{unused && (");
    expect(at, "算出来了却没渲染").toBeGreaterThan(0);
    // 用警告色 —— 「你以为挂上了、其实没有」和「还没有图」不是一个量级
    expect(CAST.slice(at, at + 400)).toContain("--studio-accent");
  });
});
