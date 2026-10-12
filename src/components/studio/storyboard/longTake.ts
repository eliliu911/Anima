import type { StoryboardShot } from "../../../api/cinema";

/**
 * 长镜头：一场整段出，不分镜出。**和 Rust 的 `long_take::plan` 是同一份判断。**
 *
 * 两边各写各的会出现「界面说能跑，点下去后端拒了」—— 而用户不知道该信哪个。
 */

/** 重叠帧数的默认值。节点只收 5 / 22 / 39 / 56（H3 的 17n+5 潜空间栅格），推荐 22。 */
export const CONTEXT_FRAMES = 22;

/**
 * 一个窗口实际会出多少帧。
 *
 * **H3 的 latent 时间维要求帧数落在 `17n+5` 上**，不落在栅格上的会被
 * 向上吸附（Rust 侧 `video_tool::snap_frames_17n_plus_5`，同一个公式）。
 * 所以填 4 秒不是 96 帧而是 107 帧 —— 每个窗口都比填的长一点。
 */
export function windowFrames(windowSecs: number, fps = 24): number {
  const f = Math.max(5, Math.round(windowSecs * fps));
  return f + ((5 + 17 - (f % 17)) % 17);
}

/**
 * 去掉重叠之后的成片时长。
 *
 * # 为什么不能写成 `n × 窗口秒数 − (n−1) × 重叠`
 *
 * 那是最直觉的写法，也是这里一直写着的那份 —— **它漏了帧栅格吸附**。
 * 实测对不上：4 个 4 秒窗口按那个公式是 13.25 秒，而真跑出来的文件
 * 是 **15.08 秒**（362 帧）；3 个 3 秒窗口公式给 7.17 秒，实际 7.29 秒（175 帧）。
 *
 * 少算一两秒本身不致命，致命的是**它会让人以为窗口配错了**：
 * 想要 15 秒的人看到 13.25 会去加窗口，而他本来就已经到 15 秒了。
 *
 * 两个数字都是实测的，下面的测试拿它们当锚点。
 */
export function netSecs(windowSecs: number, windows: number, frames: number = CONTEXT_FRAMES, fps = 24): number {
  const f = windowFrames(windowSecs, fps);
  return (f + (windows - 1) * (f - frames)) / fps;
}

/** 节点认的全部档位。不在表里的值节点会拒。 */
export const CONTEXT_FRAME_CHOICES = [5, 22, 39, 56] as const;

/**
 * 这一档重叠该配什么说明。**短窗口配大重叠是在白烧算力** ——
 * 3 秒窗口配 22 帧，31% 的采样发布时会被裁掉。
 */
export function contextFramesHint(frames: number, windowSecs: number): string {
  const secs = frames / 24;
  const share = secs / Math.max(windowSecs, 0.1);
  const base =
    frames === 5
      ? "最小重叠：落在词中间的概率最小，但运动衔接最弱"
      : frames === 22
        ? "推荐：运动和声音的上下文足够，上游默认"
        : "大重叠：衔接最稳，但被裁掉的采样也最多";
  return `${base}。${secs.toFixed(2)} 秒，占窗口的 ${Math.round(share * 100)}%`;
}
export const MAX_WINDOW_SECS = 15;
export const MIN_WINDOW_SECS = 2.5;
/** 低于这个秒数是 H3 的实验性档位 —— 会向上吸附到 17n+5 帧栅格。 */
export const EXPERIMENTAL_BELOW_SECS = 5;

export interface LongTakePlan {
  ok: true;
  windowSecs: number;
  windows: number;
  /** 去掉重叠之后的成片时长 */
  netSecs: number;
  /** 重叠占一个窗口的比例。3 秒窗口是 30%，那三成算力发布时会被裁掉 */
  overlapShare: number;
  /** 预计要跑多久（小时）。4.5 分钟算力每秒成片，来自 2026-09-20 实测 */
  etaHours: number;
  /** 能跑，但有话要说 */
  warning?: string;
}

export interface LongTakeBlocked {
  ok: false;
  /** 为什么不行。**要能直接告诉用户怎么改** */
  why: string;
}

/** 这一场能不能整段出。 */
export function planLongTake(shots: StoryboardShot[], frames: number = CONTEXT_FRAMES): LongTakePlan | LongTakeBlocked {
  if (shots.length < 2) return { ok: false, why: "这一场只有一镜 —— 直接出片就行，不需要长镜头" };
  const secs = [...shots].sort((a, b) => a.shot_num - b.shot_num).map((s) => s.duration);
  const w = secs[0];
  // 半帧容差：界面上是 0.5 秒一档，浮点相等靠不住
  if (secs.some((v) => Math.abs(v - w) > 1 / 48)) {
    const kinds = [...new Set(secs.map((v) => v.toFixed(0)))].join(" / ");
    return {
      ok: false,
      why: `长镜头的窗口必须等长，而这一场是 ${kinds} 秒 —— 改成同一个时长再试（H3 的窗口时长只有一个值，所有窗口共用）`,
    };
  }
  if (w > MAX_WINDOW_SECS) {
    return { ok: false, why: `每一镜 ${w} 秒，超过 H3 单窗口的 ${MAX_WINDOW_SECS} 秒上限 —— 把这一场拆成更多、更短的镜头` };
  }
  if (w < MIN_WINDOW_SECS) {
    return { ok: false, why: `每一镜只有 ${w} 秒，短于 H3 单窗口的 ${MIN_WINDOW_SECS} 秒下限 —— 把镜头拉长一点` };
  }
  const n = shots.length;
  const overlap = frames / 24;
  const overlapShare = overlap / w;
  return {
    ok: true,
    windowSecs: w,
    windows: n,
    netSecs: netSecs(w, n, frames),
    overlapShare,
    etaHours: (w * n * 4.5) / 60,
    warning:
      w < EXPERIMENTAL_BELOW_SECS
        ? `每一镜只有 ${w} 秒，低于 ${EXPERIMENTAL_BELOW_SECS} 秒是 H3 的实验性档位：实际时长会向上吸附到 17n+5 帧栅格（比填的长一点），而且 ${overlap.toFixed(1)} 秒的重叠占了窗口的 ${Math.round(overlapShare * 100)}%，这部分算力发布时会被裁掉。适合验管道，不适合评内容。`
        : undefined,
  };
}

/**
 * 勾选时要当场说清的代价。**不能做成一个安静的勾选框** ——
 * 它把「一镜一个任务、随时能停、能单独重做」换成了
 * 「一场一个任务、几小时、中途不能改、某一段不满意要整条重来」。
 */
export function longTakeCost(p: LongTakePlan): string[] {
  return [
    `${p.windows} 镜 × ${p.windowSecs} 秒 → ${p.netSecs.toFixed(1)} 秒成片`,
    `一个任务，预计 ${p.etaHours < 1 ? `${Math.round(p.etaHours * 60)} 分钟` : `${p.etaHours.toFixed(1)} 小时`}，中途不能改`,
    "某一个窗口不满意，要整条重来（H3 的潜变量不能存盘）",
    "建议：先按分镜出一遍确认台词和站位，满意了再勾这个重出",
  ];
}
