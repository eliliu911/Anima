import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import type { StoryboardShot } from "../../../api/cinema";
import { DEFAULT_RENDER, QUALITIES, durationLabel, estimateLtxMinutes, estimateMinutes, ltxSize, renderSize, videoEngineOf } from "./renderOptions";
import { renderTargetsFor } from "./RenderOptionsDialog";

describe("出片引擎（批量出片里也能选，和生成设置的「动效」同一个字段）", () => {
  it("引擎 id 归一：老数据的 ltx25 / minimax_h3 都认", () => {
    expect(videoEngineOf("ltx")).toBe("ltx");
    expect(videoEngineOf("ltx25")).toBe("ltx");
    expect(videoEngineOf("LTX2.5")).toBe("ltx");
    expect(videoEngineOf("h3")).toBe("h3");
    expect(videoEngineOf("minimax_h3")).toBe("h3");
    expect(videoEngineOf(undefined)).toBe("h3");
  });
  /** 和 Rust `cinema::shot_render_size`（frame_for 1280 → fit_dimensions）一致 */
  it("LTX 尺寸按项目画幅、长边 1280", () => {
    expect(ltxSize("16:9")).toEqual([1280, 736]);
    expect(ltxSize("9:16")).toEqual([736, 1280]);
    expect(ltxSize("1:1")).toEqual([1280, 1280]);
    expect(ltxSize("")).toEqual([736, 1280]);
  });
  it("LTX 耗时：每镜 40 秒 + 每秒成片 22 秒，多次翻倍", () => {
    expect(estimateLtxMinutes([10], 1)).toEqual({ minutes: 4, measured: true });
    expect(estimateLtxMinutes([10, 10], 2).minutes).toBe(17);
  });
  it("弹窗把选的引擎存回项目；LTX 时没首帧的镜头不排进这一批", () => {
    const src = readFileSync(fileURLToPath(new URL("./RenderOptionsDialog.tsx", import.meta.url)), "utf8");
    expect(src).toContain("video_engine: engine,");
    expect(src).not.toMatch(/video_engine: "h3",\s*\n\s*character_aspect/);
    expect(src).toContain("const noFrame = ltx ? fullPlan.ids.filter((id) => !shots.find((s) => s.id === id)?.keyframes?.first) : [];");
  });
});

describe("出片选项", () => {
  /**
   * `burn_subtitles` 默认 **false**：2026-09-15 起默认出软字幕（整集 SRT，
   * 剪映 / 播放器里挂），硬烧改成生成设置里的开关。后端 `default_burn_subtitles`
   * 同步改成了 false，当时漏改了这条前端测试。
   */
  it("默认：参考出片 Larry 6 步、首帧出片 LightX2V、480p，和后端默认一致", () => {
    expect(DEFAULT_RENDER).toEqual({ quality: "staged", keyframe_quality: "lightx2v", resolution: "480p", aspect: "", takes: 1, generate_audio: true, burn_subtitles: false });
    const rust = readFileSync(fileURLToPath(new URL("../../../../src-tauri/src/cinema.rs", import.meta.url)), "utf8");
    expect(rust).toContain("fn default_keyframe_quality() -> RenderQuality {\n    RenderQuality::LightX2V");
  });

  /** 和 Rust `RenderSettings::size` 的测试用同一组数：界面显示的尺寸必须等于实际发给引擎的。 */
  it("尺寸和后端算法一致", () => {
    expect(renderSize(DEFAULT_RENDER, "16:9")).toEqual([896, 512]);
    expect(renderSize(DEFAULT_RENDER, "9:16")).toEqual([512, 896]);
    expect(renderSize({ ...DEFAULT_RENDER, resolution: "720p" }, "16:9")).toEqual([1312, 736]);
    expect(renderSize({ ...DEFAULT_RENDER, resolution: "576p" }, "16:9")).toEqual([1024, 576]);
    expect(renderSize({ ...DEFAULT_RENDER, aspect: "1:1" }, "16:9")).toEqual([512, 512]);
    expect(renderSize({ ...DEFAULT_RENDER, aspect: "abc" }, "")).toEqual([896, 512]);
    const [w, h] = renderSize({ ...DEFAULT_RENDER, resolution: "720p", aspect: "21:9" }, "");
    expect(w).toBeLessThanOrEqual(1344);
    expect(w % 32).toBe(0);
    expect(h % 32).toBe(0);
  });

  /** 2026-10-08 按 EP03 整集实测重填：按链路分（参考出片同样 6 步贵约一倍半），实测和估算分开标 */
  it("耗时按链路查：720p 参考 Larry 6 每秒 16.4、首帧 LightX2V 每秒 6.7（实测）", () => {
    const r720 = { ...DEFAULT_RENDER, resolution: "720p" as const };
    expect(estimateMinutes(r720, 10, "staged", "reference")).toEqual({ minutes: 164, measured: true });
    expect(estimateMinutes(r720, 10, "lightx2v", "keyframe")).toEqual({ minutes: 67, measured: true });
    expect(estimateMinutes(DEFAULT_RENDER, 5, "staged", "reference")).toEqual({ minutes: 33, measured: true });
    expect(estimateMinutes(DEFAULT_RENDER, 10, "lightx2v", "keyframe")).toEqual({ minutes: 20, measured: true });
    // 不给链路 = 参考出片；每镜多次翻倍；没测过的标估算
    expect(estimateMinutes(DEFAULT_RENDER, 6)).toEqual(estimateMinutes(DEFAULT_RENDER, 6, "staged", "reference"));
    expect(estimateMinutes({ ...DEFAULT_RENDER, takes: 2 }, 5).minutes).toBe(66);
    expect(estimateMinutes(DEFAULT_RENDER, 6, "dense", "reference").measured).toBe(false);
    // 同一档参考出片比首帧出片贵
    expect(estimateMinutes(r720, 10, "staged", "reference").minutes).toBeGreaterThan(estimateMinutes(r720, 10, "staged", "keyframe").minutes);
    const dialog = readFileSync(fileURLToPath(new URL("./RenderOptionsDialog.tsx", import.meta.url)), "utf8");
    expect(dialog).toContain('estimateMinutes(draft, secsOf(kfShots), kfQuality, "keyframe")');
    expect(QUALITIES.map((q) => q.id)).toContain("lightx2v");
    expect(DEFAULT_RENDER.quality).toBe("staged");
    expect(durationLabel(16)).toBe("约 16 分钟");
    expect(durationLabel(130)).toBe("约 2 小时 10 分钟");
    expect(durationLabel(120)).toBe("约 2 小时");
  });

  it("选项卡开始时先把设置存进项目再出片", () => {
    const src = readFileSync(fileURLToPath(new URL("./RenderOptionsDialog.tsx", import.meta.url)), "utf8");
    const start = src.slice(src.indexOf("const start = async"), src.indexOf("return (", src.indexOf("const start = async")));
    expect(start.indexOf("cinemaSaveModelSettings")).toBeGreaterThan(-1);
    expect(start.indexOf("render: draft")).toBeGreaterThan(-1);
    expect(start).toContain("prompt_route: route");
    expect(start.indexOf("cinemaSaveModelSettings")).toBeLessThan(start.indexOf("onStart(plan)"));
  });

  it("线路切换：按所选线路算能出的镜头；单镜这条线路没合成就不能出", () => {
    const syn = (n: number, route: "a" | "b", extra: Partial<StoryboardShot> = {}) =>
      ({ id: `s${n}`, shot_num: n, scale: "", camera_movement: "", dialogue: "", prompt: "", duration: 8, status: "pending", prompts: { [route]: { frame: "f", video: "v", synthesized_at: 1 } }, ...extra }) as StoryboardShot;
    const shots = [syn(1, "b"), syn(2, "b"), syn(3, "a")];
    expect(renderTargetsFor({ kind: "batch" }, shots, [], "b").ids).toEqual(["s1", "s2"]);
    expect(renderTargetsFor({ kind: "batch" }, shots, [], "a")).toEqual({ ids: ["s3"], longTakeScenes: [], redo: false, blocked: 2, done: 0 });
    expect(renderTargetsFor({ kind: "single", shotId: "s1" }, shots, [], "b").ids).toEqual(["s1"]);
    expect(renderTargetsFor({ kind: "single", shotId: "s1" }, shots, [], "a")).toEqual({ ids: [], longTakeScenes: [], redo: false, blocked: 1 });
    const done = syn(1, "b", { takes: [{ route: "b", path: "x", created_at: 1, seed: 1, pictures: 1 }] });
    expect(renderTargetsFor({ kind: "single", shotId: "s1" }, [done], [], "b").redo).toBe(true);
  });

  /** 用户 2026-10-08：「批量出片不能选全部，已经出片的没办法在这里重新出」 */
  it("批量可以连出过的一起重出；默认照旧只出没出过的", () => {
    const syn = (n: number, route: "a" | "b", extra: Partial<StoryboardShot> = {}) =>
      ({ id: `s${n}`, shot_num: n, scale: "", camera_movement: "", dialogue: "", prompt: "", duration: 8, status: "pending", prompts: { [route]: { frame: "f", video: "v", synthesized_at: 1 } }, ...extra }) as StoryboardShot;
    const take = [{ route: "b" as const, path: "x", created_at: 1, seed: 1, pictures: 1 }];
    const shots = [syn(1, "b", { takes: take }), syn(2, "b"), syn(3, "b", { takes: take })];
    const fresh = renderTargetsFor({ kind: "batch" }, shots, [], "b");
    expect(fresh).toMatchObject({ ids: ["s2"], redo: false, done: 2 });
    const all = renderTargetsFor({ kind: "batch" }, shots, [], "b", [], [], true);
    expect(all).toMatchObject({ ids: ["s1", "s2", "s3"], redo: true, done: 2 });
    // 按场出也一样
    expect(renderTargetsFor({ kind: "scenes", sceneNos: [0] }, shots, [], "b", [], [], true).ids).toEqual(["s1", "s2", "s3"]);
    const src = readFileSync(fileURLToPath(new URL("./RenderOptionsDialog.tsx", import.meta.url)), "utf8");
    expect(src).toContain("连出过的 {freshPlan.done} 个一起重出");
  });
});

describe("画质下面写明用的哪个加速 LoRA（用户 2026-10-07：不然出来后不知道用的哪个）", () => {
  const rust = readFileSync(fileURLToPath(new URL("../../../../src-tauri/src/agent_runtime/weetodd.rs", import.meta.url)), "utf8");
  const constOf = (name: string) => new RegExp(`pub const ${name}: &str = "([^"]+)"`).exec(rust)?.[1];
  it("每一档都有说明", () => {
    for (const q of QUALITIES) expect(q.lora.length).toBeGreaterThan(5);
  });
  it("文件名和 Rust 里挂的一致", () => {
    const lightx2v = constOf("H3_LIGHTX2V_TURBO_LORA")!;
    expect(QUALITIES.find((q) => q.id === "lightx2v")!.lora).toContain(lightx2v.replace(".safetensors", ""));
    expect(rust).toContain("pub const H3_LIGHTX2V_STRENGTH: f64 = 0.0625;");
    expect(QUALITIES.find((q) => q.id === "lightx2v")!.lora).toContain("0.0625");
    const larry = constOf("H3_LARRY_TURBO_LORA")!;
    for (const id of ["fast", "standard", "staged"] as const) expect(QUALITIES.find((q) => q.id === id)!.lora).toContain(larry.replace(".safetensors", ""));
    expect(QUALITIES.find((q) => q.id === "dense")!.lora).toContain("不用加速补丁");
  });
});
