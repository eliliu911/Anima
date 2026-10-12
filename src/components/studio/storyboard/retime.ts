/**
 * 改了一镜的时长，**分段时间线跟着按比例伸缩**。
 *
 * 用户 2026-09-30：台词放不下，把第 7 镜从 11 秒加到 15 秒，提示照样在 —— 下面已经生成的时间线
 * （0~3 / 3~7 / 7~11 秒）不会跟着变，界面上又没地方改分段时间，于是换成「最后一段到 11 秒，镜头时长是 15 秒，对不上」。
 *
 * 做法：
 * - 每段的起止乘同一个比例，按 0.5 秒取整（镜头表的时间都是半秒刻度）；首段从 0 开始、末段对齐新时长，每段至少 0.5 秒
 * - 描写里写死的秒数（「约第5秒嗤笑着开口」「第2.5秒」）按同一个比例换算 —— 不换的话分段说 0~4 秒、正文说第 5 秒开口，
 *   合成提示词时会互相打架
 * - 末段本来就没对齐旧时长（时间线自己就是乱的）时不动 —— 那种按比例伸缩只会把错放大，留给检查去报
 */
import type { ShotSegment, StoryboardShot } from "../../../api/cinema";

const STEP = 0.5;
const round = (x: number) => Math.round(x / STEP) * STEP;
/** 和检查用的容差一致（Rust `shot_table::SEGMENT_TOLERANCE`） */
const TOLERANCE = 0.25;

const TEXT_FIELDS = ["visual", "sound", "blocking", "camera", "constraints", "forbid", "title"] as const;

/**
 * 「第5秒」「第 2.5 秒」「约5.1秒」→ 按比例换算后的秒数。
 *
 * 「约 X 秒」原来不换：EP03 第 20 镜（2026-10-05）的描写全是「约5.1秒星澜开口」这种写法，
 * 加长到 20.5 秒后分段拉长了、描写里的时刻还停在 14 秒的时间线上 —— 超长镜按描写时刻拆段时会拆偏。
 * 「约第 5 秒」里的「约」只是前缀，按「第」那条换。
 */
export function scaleSecondMentions(text: string, k: number, max: number): string {
  return text.replace(/(第|约)\s*(\d+(?:\.\d+)?)\s*秒/g, (_, p: string, n: string) => {
    const v = Math.min(max, Math.max(0, round(Number(n) * k)));
    return `${p}${Number.isInteger(v) ? v : v.toFixed(1)}秒`;
  });
}

/**
 * 时长从 `shot.duration` 改成 `next` 之后的分段。分段对不上旧时长、或者没有分段时原样返回 `null`（不动）。
 */
export function retimeSegments(segments: readonly ShotSegment[], from: number, next: number): ShotSegment[] | null {
  if (!segments.length || from <= 0 || next <= 0 || from === next) return null;
  const last = segments[segments.length - 1];
  if (Math.abs(last.end - from) > TOLERANCE || Math.abs(segments[0].start) > TOLERANCE) return null;
  const k = next / from;
  const n = segments.length;
  // 先算每个边界，再保证单调、每段至少 0.5 秒、末段落在新时长上
  const bounds = [0, ...segments.slice(0, -1).map((s) => round(s.end * k)), next];
  for (let i = 1; i < n; i++) bounds[i] = Math.max(bounds[i], bounds[i - 1] + STEP);
  for (let i = n - 1; i >= 1; i--) bounds[i] = Math.min(bounds[i], bounds[i + 1] - STEP);
  if (bounds.some((b, i) => i > 0 && b - bounds[i - 1] < STEP - 1e-9)) return null; // 新时长放不下这么多段
  return segments.map((s, i) => {
    const out: ShotSegment = { ...s, start: bounds[i], end: bounds[i + 1] };
    for (const f of TEXT_FIELDS) {
      const t = s[f];
      if (typeof t === "string" && t) out[f] = scaleSecondMentions(t, k, next);
    }
    return out;
  });
}

/** 改时长时一起改好的那一镜。时间线伸缩不了就只改时长 */
export function withDuration(shot: StoryboardShot, next: number): StoryboardShot {
  const segments = retimeSegments(shot.segments ?? [], shot.duration, next);
  return segments ? { ...shot, duration: next, segments } : { ...shot, duration: next };
}
