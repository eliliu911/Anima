import type { VideoGenNodeData } from "../types";
import { estimateMinutes } from "../../components/studio/storyboard/renderOptions";
import type { RenderQuality, RenderSettings } from "../../api/cinema";
import { effectiveQuality } from "./h3Quality";
import { CONTEXT_FRAMES, MAX_WINDOW_SECS, MIN_WINDOW_SECS, EXPERIMENTAL_BELOW_SECS, netSecs } from "../../components/studio/storyboard/longTake";

/**
 * 画布上的长镜头：**一个节点，里面一张窗口表**。
 *
 * ```text
 * ┌─ 视频节点 ────────────────┐
 * │ 长镜头 · 4 个窗口 × 4s    │
 * │   窗口 1  提示词…         │  一次提交 → 一个 MP4
 * │   窗口 2  提示词…         │
 * │   窗口 3  提示词…         │
 * │   窗口 4  提示词…         │
 * └───────────────────────────┘
 * ```
 *
 * # 为什么不是「视频节点连视频节点」
 *
 * 第一版是窗口链：连起来的 N 个视频节点 = N 个窗口。**底层一直是对的**
 * （一次提交、一个 mp4），但界面上那是 N 个长得一模一样的出片节点，
 * 每个都有播放器、尺寸、上传、收片 —— 而窗口 2..N **永远不会有视频**。
 *
 * 用户 2026-09-21 两次指出这件事：「是一个指令给后端，后端最终也是生成
 * 1 个视频，你这完全不[对]」。我第一次只在标题栏写了「窗口 N/M」——
 * 那是贴标签，四个假播放器一个没少，他第二次原话是「也没给我改好啊」。
 *
 * **表达和行为不一致时，改表达要改到形状，不是改到文案。**
 *
 * # 代价：每窗口的参考图
 *
 * 链那一版每个窗口能各连各的图。收进一个节点之后，上游的图**每个窗口挂同一组**
 * （2026-10-11 起；以前只给窗口 1，后面的窗口光靠重叠帧接，15 秒窗口后半段人和道具会漂）。
 * 真要「同一条长镜头里换人换景」，那是给窗口行加参考槽的事，不是退回四个节点。
 */

export interface LongTakePlan {
  ok: true;
  windows: number;
  windowSecs: number;
  /** 去掉重叠之后的成片时长 */
  netSecs: number;
  /** 重叠占一个窗口的比例。3 秒窗口配 22 帧是 31%，那部分发布时会被裁掉 */
  overlapShare: number;
  /** 预计要跑多久（小时）。查工作台那张按链路 × 加速档 × 清晰度实测的表（`renderOptions.estimateMinutes`，参考出片那一列）。
   * 以前固定按每秒 4.5 分钟（09-20 实测）—— 720P + Larry 6 步实际每秒 16.4 分钟，星澜 V2 显示「2.3 小时」、实际 8 小时（10-12） */
  etaHours: number;
  /** 能跑，但有话要说 */
  warning?: string;
}

export interface LongTakeBlocked {
  ok: false;
  /** 为什么不行。**要能直接告诉用户怎么改** */
  why: string;
}

/** 这个视频节点是不是一条长镜头。**判据是窗口表，不是连线。** */
export function isLongTake(d: VideoGenNodeData | undefined): boolean {
  return (d?.longTakeWindows?.length ?? 0) >= 2;
}

/**
 * 这个节点能不能整段出。**和 Rust 的 `long_take::plan` 判的是同一组条件** ——
 * 至少两个窗口、窗口时长在 2.5~15 秒之间。
 *
 * 窗口等长在这里是**天然成立**的：一个节点只有一个 `durationSecs`。
 * 链那一版要专门查一次「窗口必须等长」，现在这种错误根本构造不出来。
 */
export function planLongTake(d: VideoGenNodeData | undefined, frames: number = CONTEXT_FRAMES): LongTakePlan | LongTakeBlocked {
  const wins = d?.longTakeWindows ?? [];
  if (wins.length < 2) return { ok: false, why: "长镜头至少要两个窗口 —— 点「加一个窗口」" };
  if ((d?.engine ?? "") !== "h3") return { ok: false, why: `长镜头只有 H3 能做，这个节点是 ${d?.engine || "（没选引擎）"}` };
  const w = d?.durationSecs ?? 0;
  if (w > MAX_WINDOW_SECS) return { ok: false, why: `每个窗口 ${w} 秒，超过 H3 的 ${MAX_WINDOW_SECS} 秒上限 —— 把窗口调短、窗口数加多` };
  if (w < MIN_WINDOW_SECS) return { ok: false, why: `每个窗口只有 ${w} 秒，短于 H3 的 ${MIN_WINDOW_SECS} 秒下限 —— 把窗口调长` };

  const n = wins.length;
  const overlap = frames / 24;
  const overlapShare = overlap / w;
  return {
    ok: true,
    windows: n,
    windowSecs: w,
    netSecs: netSecs(w, n, frames),
    overlapShare,
    etaHours: longTakeEtaMinutes(d, w * n) / 60,
    warning:
      w < EXPERIMENTAL_BELOW_SECS
        ? `每个窗口只有 ${w} 秒，低于 ${EXPERIMENTAL_BELOW_SECS} 秒是 H3 的实验性档位：实际时长会向上吸附到 17n+5 帧栅格，而且 ${overlap.toFixed(1)} 秒的重叠占了窗口的 ${Math.round(overlapShare * 100)}%，这部分发布时会被裁掉。`
        : undefined,
  };
}

/** 一条台词在窗口里的落点。 */
export interface WindowLine {
  /** 第几句 */
  n: number;
  /** 什么时候开口（窗口内秒数） */
  at: number;
  /** 说完要多久 */
  needs: number;
  /** 说不说得完 */
  fits: boolean;
}

/** 中文口语每秒几个字。**和 Rust `prompt_route_b::SPOKEN_CPS` 必须一致。** */
export const SPOKEN_CPS = 4.1;

/**
 * 从一个窗口的提示词里扒出台词落点。
 *
 * # 为什么长镜头非看这个不可
 *
 * 长镜头是**一次提交、不能只重来某一窗口**（H3 的潜变量存不了盘）。
 * 所以「第三个窗口的台词说不完」这种事，必须在提交**之前**看出来 ——
 * 跑完才发现的代价是整条重来。
 *
 * 格式和 Rust 侧的 `line_timings` 同源：`Around the N.N-second mark,`（老提示词 `At 00:SS.S,`）定开口时刻，
 * `<d>…</d>` 包台词。剥掉 `[Chinese]` 这类语言标签再数字。
 */
export function windowLines(prompt: string, windowSecs: number): WindowLine[] {
  const out: WindowLine[] = [];
  let rest = prompt;
  let n = 0;
  for (;;) {
    const open = rest.indexOf("<d>");
    if (open < 0) break;
    const close = rest.indexOf("</d>", open);
    if (close < 0) break;
    n += 1;
    let body = rest.slice(open + 3, close);
    const tag = body.trimStart().startsWith("[") ? body.indexOf("]") : -1;
    if (tag >= 0) body = body.slice(tag + 1);
    // 只数会念出来的字（标点和空白不占时间）
    const chars = [...body].filter((c) => /[^\s，。！？、：；""''（）,.!?:;"'()\-—…]/.test(c)).length;
    const at = lastTimecode(rest.slice(0, open));
    if (at !== null) {
      const needs = chars / SPOKEN_CPS;
      out.push({ n, at, needs, fits: at + needs <= windowSecs + 1e-6 });
    }
    rest = rest.slice(close + 4);
  }
  return out;
}

/**
 * 末尾最近的一个时刻。**和 Rust `prompt_route_b::timecodes` 同一套**：官方切镜 `At 00:03.500`、
 * 镜内节拍 `Around the 3.5-second mark`（2026-10-06 起按 H3 官方规范这么写），以及老提示词的 `At 00:03.5`。
 * 只认 `around the` / `at the` 开头的 `-second mark` —— 对齐指令里的 `aligns with the 8.00-second mark` 不算。
 */
function lastTimecode(text: string): number | null {
  let found: { at: number; v: number } | null = null;
  const take = (at: number, v: number) => {
    if (Number.isFinite(v) && (!found || at >= found.at)) found = { at, v };
  };
  for (const m of text.matchAll(/At (\d{2}):(\d+(?:\.\d+)?)/g)) take(m.index ?? 0, Number(m[1]) * 60 + Number(m[2]));
  for (const m of text.matchAll(/(?:around|at) the (\d+(?:\.\d+)?)-second mark/gi)) take(m.index ?? 0, Number(m[1]));
  return (found as { at: number; v: number } | null)?.v ?? null;
}

/**
 * 开一条长镜头：把节点现在的提示词当窗口 1，再补一个空窗口。
 *
 * **不丢原来的提示词** —— 点「改成长镜头」之前写的东西是有用的，
 * 清掉会让人以为这个开关是破坏性的，于是没人敢点。
 */
export function openLongTake(prompt: string): string[] {
  return [prompt, ""];
}

/** 长镜头（窗口链一律是参考出片）要跑几分钟：节点的清晰度和加速档查实测表 */
export function longTakeEtaMinutes(d: VideoGenNodeData | undefined, sampledSecs: number): number {
  const short = Math.min(d?.width || 640, d?.height || 384);
  const resolution: RenderSettings["resolution"] = short <= 544 ? "480p" : short <= 608 ? "576p" : "720p";
  const quality = (effectiveQuality(d?.h3Quality as string | undefined, "r2v") ?? "staged") as RenderQuality;
  const r = { quality, keyframe_quality: "lightx2v", resolution, aspect: "16:9", takes: 1, generate_audio: true, burn_subtitles: false } as RenderSettings;
  return estimateMinutes(r, sampledSecs, quality, "reference").minutes;
}

