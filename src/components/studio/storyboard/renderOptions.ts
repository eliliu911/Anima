/**
 * 出片选项卡的纯函数：默认值、尺寸、耗时估算。
 *
 * 尺寸算法和 Rust `RenderSettings::size` 一致（界面上显示的尺寸 = 实际发给引擎的）。
 */
import type { RenderQuality, RenderSettings } from "../../../api/cinema";

export const DEFAULT_RENDER: RenderSettings = {
  quality: "staged",
  keyframe_quality: "lightx2v",
  resolution: "480p",
  aspect: "",
  takes: 1,
  generate_audio: true,
  // **默认不烧。** 烧进画面的字幕事后改不了（字体、时间、错别字都得重出片）；
  // 合成一集时另出一份 final.srt，播放器、剪映、达芬奇都能用。要硬字幕在生成设置里开
  burn_subtitles: false,
};

/**
 * `lora`：这一档实际挂的加速 LoRA（用户 2026-10-07：「不然出来后不知道用的哪个」）。
 * 写法用大白话（用户：「不理解什么叫前 2 次不挂……19 次求值是什么意思」）：一「步」= 去噪一次 = 把大模型完整算一遍；
 * 加速 LoRA = 让模型 4 步左右就画完的补丁。
 * **和 Rust `weetodd::H3Tier::turbo_lora` / `H3SamplingPlan` 对得上**（有测试读源码核对文件名）。
 */
export const QUALITIES: { id: RenderQuality; label: string; hint: string; lora: string }[] = [
  { id: "fast", label: "快速", hint: "只看构图和动作", lora: "全程挂 Larry v4 完整版加速补丁（minimax_h3_turbo_v4_step600_ema），只走 4 步" },
  { id: "standard", label: "Larry 4 步", hint: "快，适合先看一遍", lora: "全程挂 Larry v4 完整版加速补丁（minimax_h3_turbo_v4_step600_ema），只走 4 步" },
  { id: "staged", label: "Larry 6 步", hint: "Larry 官方推荐的 6 步，细节和人物最稳", lora: "全程挂 Larry v4 完整版加速补丁（minimax_h3_turbo_v4_step600_ema），走 6 步。以前这一档叫「双采」（2 步原版 + 4 步精简补丁），10-07 换掉" },
  { id: "dense", label: "精细 19 步", hint: "不加速，最慢（约 3 倍）", lora: "不用加速补丁，原版模型完整走 19 步，最慢（约双采 3 倍）" },
  // 2026-10-07 EP03 第 10 镜 480p 同种子对照：20.2 分钟（双采 28.9），礼服颜色、铭牌都守得更牢。只管首帧出片和兼容模式参考出片
  { id: "lightx2v", label: "LightX2V 4 步", hint: "快约三成，颜色和首帧守得稳", lora: "全程挂 LightX2V 首尾帧 4 步 v0.1 加速补丁（minimax_h3_fl2v_turbo_4step_v0.1，强度 0.0625），只走 4 步" },
];

/**
 * 两条链路各自能选哪几档、默认哪档（2026-10-07 对照，见 docs/reports/2026-10-07-加速LoRA对照）：
 * - 首帧出片：LightX2V 4 步最好（Larry 在首帧出片里多出一个人、铭牌冒乱字）；
 * - 参考出片：Larry 6 步定稿（人物服装都对），LightX2V 4 步快约三成、适合预览。
 * 「快速」不再给：换成 Larry 之后它和 Larry 4 步一样（老项目存的照旧能出）。
 */
export const KEYFRAME_QUALITIES: { id: RenderQuality; tag?: string }[] = [
  { id: "lightx2v", tag: "推荐" },
  { id: "standard" },
  { id: "staged" },
  { id: "dense" },
];
export const REFERENCE_QUALITIES: { id: RenderQuality; tag?: string }[] = [
  { id: "staged", tag: "定稿" },
  { id: "lightx2v", tag: "预览" },
  { id: "standard" },
  { id: "dense" },
];
export const DEFAULT_KEYFRAME_QUALITY: RenderQuality = "lightx2v";
/** 老数据存的「快速」按 Larry 4 步显示 */
export const shownQuality = (q: RenderQuality): RenderQuality => (q === "fast" ? "standard" : q);

/** 参考生视频开了原生 Ref2VA 时，那几镜不管选哪一档都改挂这个（见 Rust `H3SamplingPlan::current`） */
export const NATIVE_REF2VA_LORA_NOTE = "参考生视频若开了原生 Ref2VA：那几镜改挂 Ref2VA 专用 4 步 LoRA（LightX2V ref2v v0.1）";

export const RESOLUTIONS: { id: RenderSettings["resolution"]; label: string }[] = [
  { id: "480p", label: "480p" },
  { id: "576p", label: "576p" },
  { id: "720p", label: "720p" },
];

export const ASPECTS = ["16:9", "9:16", "1:1", "4:3", "3:4", "21:9"] as const;

/** 和 Rust `weetodd::fit_dimensions` 一致：长边最多 1344，32 对齐。 */
function fit(w: number, h: number): [number, number] {
  const scale = Math.min(1344 / Math.max(w, h), 1);
  const round = (v: number) => Math.max(1, Math.round((v * scale) / 32)) * 32;
  return [round(w), round(h)];
}

/** 实际出片尺寸。和 Rust `RenderSettings::size` 一致。 */
export function renderSize(r: RenderSettings, projectAspect: string): [number, number] {
  const short = r.resolution === "576p" ? 576 : r.resolution === "720p" ? 736 : 512;
  const m = /^\s*(\d+(?:\.\d+)?)\s*:\s*(\d+(?:\.\d+)?)\s*$/.exec(r.aspect.trim() || projectAspect);
  const [a, b] = m && Number(m[1]) > 0 && Number(m[2]) > 0 ? [Number(m[1]), Number(m[2])] : [16, 9];
  return a >= b ? fit(Math.round((short * a) / b), short) : fit(short, Math.round((short * b) / a));
}

/**
 * 每秒成片要几分钟，**按链路 × 画质 × 分辨率**。标了 measured 的是实测（本机 M4 Max），其余从同一条链路的实测推算
 * （步数按 4 / 6 / 19 换算，分辨率按像素数换算）。
 *
 * **2026-10-08 按 EP03 整集实测重填**（用户：「按昨晚实测的数更新耗时表」）。以前一张表不分链路，720p 参考出片写 7.2，
 * 实测 16.4 —— 参考出片每一步都带着参考图的 token 一起算，同样 6 步比首帧出片贵约一倍半，一个数管不了两条链路。
 *
 * | 实测来源 | 链路 · 画质 · 分辨率 | 每秒 |
 * |---|---|---|
 * | EP03 第 1/4/5/8/10/11 镜（6 镜 62 秒） | 首帧 · LightX2V · 720p | 6.7 |
 * | EP03 第 2/3/6/7/9 镜（5 镜 51 秒，4~8 人） | 参考 · Larry 6 · 720p | 16.4 |
 * | 10-07 第 10 镜对照 | 首帧 · LightX2V / Larry 6 · 480p | 2.0 / 3.0 |
 * | 10-07 第 2 镜参考对照（5 秒 33 分钟） | 参考 · Larry 6 · 480p | 6.6 |
 *
 * **长镜头更贵**：H3 注意力随帧数平方涨，同一档 6 秒和 14 秒的镜头每秒差五成（5.2 → 8.1）。这里是平均数，长镜头会超、短镜头会省。
 */
type Rates = Record<RenderQuality, Record<RenderSettings["resolution"], { m: number; measured: boolean }>>;
const est = (m: number) => ({ m, measured: false });
const real = (m: number) => ({ m, measured: true });
const MINUTES_PER_SECOND: Record<"keyframe" | "reference", Rates> = {
  keyframe: {
    lightx2v: { "480p": real(2.0), "576p": est(3.7), "720p": real(6.7) },
    standard: { "480p": est(2.0), "576p": est(3.7), "720p": est(6.7) },
    fast: { "480p": est(2.0), "576p": est(3.7), "720p": est(6.7) },
    staged: { "480p": real(3.0), "576p": est(5.5), "720p": est(10) },
    dense: { "480p": est(9.5), "576p": est(17), "720p": est(32) },
  },
  reference: {
    staged: { "480p": real(6.6), "576p": est(9), "720p": real(16.4) },
    lightx2v: { "480p": est(4.6), "576p": est(6.3), "720p": est(11.5) },
    standard: { "480p": est(4.4), "576p": est(6), "720p": est(11) },
    fast: { "480p": est(4.4), "576p": est(6), "720p": est(11) },
    dense: { "480p": est(21), "576p": est(28), "720p": est(52) },
  },
};

/**
 * 预计耗时（分钟）。`seconds` 是这一批镜头的总时长。`quality` 不给 = 参考出片那一档；`route` 不给 = 参考出片。
 */
export function estimateMinutes(
  r: RenderSettings,
  seconds: number,
  quality: RenderQuality = r.quality,
  route: "keyframe" | "reference" = "reference",
): { minutes: number; measured: boolean } {
  const rate = MINUTES_PER_SECOND[route][quality][r.resolution];
  return { minutes: Math.round(rate.m * seconds * Math.max(1, r.takes)), measured: rate.measured };
}

/** 「约 2 小时 10 分钟」 */
export function durationLabel(minutes: number): string {
  if (minutes < 60) return `约 ${minutes} 分钟`;
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return `约 ${h} 小时${m ? ` ${m} 分钟` : ""}`;
}

/**
 * 出片引擎。**和生成设置里的「动效」是同一个字段**（`model_settings.video_engine`）——
 * 用户 2026-10-06：「把生成设置的功能同步放进批量出片设置里，这样就不用两边都要查看设置了」。
 */
export type VideoEngine = "h3" | "ltx";

export const VIDEO_ENGINES: { id: VideoEngine; label: string; hint: string }[] = [
  { id: "h3", label: "MiniMax H3", hint: "按参考图 / 首帧出，能念中文台词、对口型" },
  { id: "ltx", label: "LTX 2.5", hint: "只认首帧，快约 10 倍；念不了中文台词" },
];

/** 项目里存的引擎 id 归一（老数据有 `minimax_h3` / `ltx25`）。认不出的按 H3 */
export function videoEngineOf(id: string | undefined | null): VideoEngine {
  const v = (id ?? "").trim().toLowerCase();
  return v === "ltx" || v === "ltx25" || v === "ltx2.5" ? "ltx" : "h3";
}

/**
 * LTX 的出片尺寸：**不看画质 / 清晰度，也不看出片弹窗里的比例**，按项目画幅、长边 1280 出。
 * 和 Rust `cinema::shot_render_size` 一致（`frame_for(画幅, 1280, 32)` 再 `fit_dimensions`）。
 */
export function ltxSize(projectAspect: string): [number, number] {
  const m = /^\s*(\d+(?:\.\d+)?)\s*:\s*(\d+(?:\.\d+)?)\s*$/.exec(projectAspect);
  const [a, b] = m && Number(m[1]) > 0 && Number(m[2]) > 0 ? [Number(m[1]), Number(m[2])] : [9, 16];
  const short = (x: number) => Math.max(1, Math.round(x / 32)) * 32;
  return a >= b ? fit(1280, short((1280 * b) / a)) : fit(short((1280 * a) / b), 1280);
}

/** LTX 预计耗时：每镜固定约 40 秒 + 每秒成片约 22 秒（2026-09 本机实测，LTX 约比 H3 快 10 倍） */
export function estimateLtxMinutes(durations: number[], takes: number): { minutes: number; measured: boolean } {
  const secs = durations.reduce((a, d) => a + 40 + 22 * Math.max(0, d), 0) * Math.max(1, takes);
  return { minutes: Math.max(1, Math.round(secs / 60)), measured: true };
}
