/**
 * 嘴型时间线的播放器。
 *
 * ## 为什么不放进 Zustand
 *
 * 嘴型每帧都要读（60fps）。放进 store 意味着每帧一次 `set()` → 每帧一次
 * React 重渲染，整个界面会被一张嘴拖死。所以这里是一个**模块级的可变对象**：
 * store 只负责把事件喂进来，渲染循环直接来读。
 *
 * 这和粒子云用 ref 而不是 state 是同一个理由（见 AvatarCanvas）。
 *
 * ## 时间原点
 *
 * Rust 侧分两步给：`words` 事件先送来时间线（此时这一句可能还在排队），
 * `sentence_start` 到达时才算 t=0。所以这里要**暂存**再**起播**。
 *
 * 为什么不能拿收到 `words` 的时刻当 t=0：句子是排队播的，合成完的时候
 * 前一句往往还在播，两个时刻能差好几秒 —— 那样整句嘴型全错开。
 */
import type { Viseme, VisemeKey } from "../api/voice";

/** 15 个通道的权重表 */
export type MouthWeights = Record<Viseme, number>;

export const VISEMES: Viseme[] = [
  "sil", "PP", "FF", "TH", "DD", "kk", "CH",
  "SS", "nn", "RR", "aa", "E", "I", "O", "U",
];

function zeroWeights(): MouthWeights {
  const w = {} as MouthWeights;
  for (const v of VISEMES) w[v] = 0;
  return w;
}

/**
 * 一个口型切到下一个要多久（毫秒）。
 *
 * 嘴型不是瞬间切换的：相邻的两个口型要交叉淡化，否则嘴会一格一格地跳。
 *
 * **关键帧之间是「保持」，不是「衰减」。** 一开始写成了脉冲式衰减
 * （到点冲上去，再按固定时长退回中性），实测「妈妈骂马」在音节之间嘴会退到
 * 几乎全闭（sil 0.93）—— 那是在扇动，不是在说话。真实说话里一个元音会一直
 * 保持到下一个发音动作接管。
 */
const BLEND_MS = 60;

/** 时间线播完多久后彻底归零 */
const TAIL_MS = 200;

class MouthTrack {
  /** 已收到时间线但还没开始播的句子 */
  private pending = new Map<number, VisemeKey[]>();
  /** 正在播的时间线 */
  private keys: VisemeKey[] = [];
  /** 正在播的这一句的 t=0，对应 performance.now() */
  private startedAt = 0;
  private playing = false;
  /** 扫描游标。每帧从上次的位置往前找，不用每帧遍历整条时间线 */
  private cursor = 0;
  private weights = zeroWeights();
  /** 音频驱动那条的最近一帧。见 [`audio`] */
  private audioViseme: Viseme | null = null;
  private audioWeight = 0;
  private audioAt = 0;

  /**
   * 时间线到了，但可能还没开始播 —— 先存着。
   *
   * 暂存表要限长：正常情况下 `sentence_start` 一定会来，但如果用户在这一句
   * 入队前就打断了，它永远不会来。不清理的话这张表会随会话一直长。
   */
  stage(id: number, keys: VisemeKey[]) {
    this.pending.set(id, keys);
    if (this.pending.size > 8) {
      // 队列不可能积压这么多句，最老的那些必然是被打断丢掉的
      const oldest = this.pending.keys().next().value;
      if (oldest !== undefined) this.pending.delete(oldest);
    }
  }

  /**
   * 音频驱动的口型（`avatar::lipsync` 算出来的）。
   *
   * **只在没有文本时间线时生效** —— `play_audio`、电台那些场景。
   * 时间线在跑的时候直接忽略：那条精度更高（15 个 viseme、含辅音），
   * 被这条粗的盖掉是退步，而且看不出是被盖了。
   *
   * 带一个过期时间：音频停了之后事件就不来了，不过期的话嘴会僵在
   * 最后那个形状上。
   */
  audio(viseme: Viseme, weight: number) {
    if (this.playing) return;
    this.audioViseme = viseme;
    this.audioWeight = weight;
    this.audioAt = performance.now();
  }

  /** 这一句开始出声了 —— 时间线从现在起算 */
  start(id: number) {
    const keys = this.pending.get(id);
    if (!keys) return;
    this.pending.delete(id);
    this.keys = keys;
    this.startedAt = performance.now();
    this.cursor = 0;
    this.playing = true;
  }

  /**
   * 停下并闭嘴。打断、一轮结束、会话结束时调用。
   *
   * 不是立刻归零，而是让 `sample` 按 RELEASE 自然收回去 —— 硬切会看到
   * 嘴「啪」一下闭上。
   */
  stop() {
    this.playing = false;
    this.keys = [];
    this.pending.clear();
    this.audioViseme = null;
    this.audioWeight = 0;
  }

  /** 现在有嘴型在动吗。给渲染侧省下无意义的计算 */
  get active(): boolean {
    return this.playing;
  }

  /**
   * 取当前时刻的 15 个通道权重。
   *
   * 算法：找到当前生效的那个关键帧，**保持**它，直到下一个关键帧接管；
   * 交接的那 BLEND_MS 毫秒里两个口型交叉淡化。
   *
   * 「保持」是这里的要点。见 [`BLEND_MS`] 的注释 —— 写成脉冲式衰减的话，
   * 音节之间嘴会退回中性，看起来是在扇动而不是在说话。
   *
   * 闭嘴由时间线自己表达：Rust 侧在停顿处和句尾都放了 `sil` 关键帧
   * （见 `avatar::viseme::schedule`），所以这里不需要额外判断"该闭嘴了没"。
   */
  sample(): MouthWeights {
    const w = this.weights;
    for (const v of VISEMES) w[v] = 0;

    if (!this.playing || this.keys.length === 0) {
      // 没有文本时间线时，用音频驱动那条兜着。
      // **要过期**：音频停了事件就不来了，不过期嘴会僵在最后那个形状上。
      // 一帧 64ms，给 3 帧的宽限
      const av = this.audioViseme;
      const fresh = av !== null && performance.now() - this.audioAt < 200;
      if (fresh && av !== null && this.audioWeight > 0) {
        w[av] = this.audioWeight;
        w.sil = 1 - this.audioWeight;
      } else {
        w.sil = 1;
      }
      return w;
    }

    const t = performance.now() - this.startedAt;
    const last = this.keys[this.keys.length - 1];
    if (t > last.at_ms + TAIL_MS) {
      // 播完了。留在 playing=false，下一句会重新起播
      this.playing = false;
      this.keys = [];
      w.sil = 1;
      return w;
    }

    // 游标只前进：时间线是有序的（Rust 侧排过），而 t 单调增长。
    // 停在**最后一个已生效**的关键帧上
    while (
      this.cursor + 1 < this.keys.length &&
      this.keys[this.cursor + 1].at_ms <= t
    ) {
      this.cursor++;
    }

    const current = this.keys[this.cursor];
    // 第一个关键帧还没到时，从闭嘴淡入
    const blend =
      t < current.at_ms
        ? 0
        : Math.min(1, (t - current.at_ms) / BLEND_MS);

    let total = 0;
    const put = (v: Viseme, value: number) => {
      // 取最大而不是相加：前后两帧指向同一个口型时相加会超过 1，嘴会张到变形
      if (value > w[v]) w[v] = value;
      total += value;
    };

    put(current.viseme, current.weight * blend);
    if (this.cursor > 0 && blend < 1) {
      const prev = this.keys[this.cursor - 1];
      put(prev.viseme, prev.weight * (1 - blend));
    }

    // 剩下的份额给闭嘴。时间线里 sil 的 weight 是 0，靠的就是这一步 ——
    // 「没有别的口型在生效」本身就等于嘴该闭着
    if (total < 1) w.sil = Math.max(w.sil, 1 - total);
    return w;
  }
}

/**
 * 全局唯一的一条。
 *
 * 单例是对的：同一时刻只有一张嘴在说话，而播放队列也是全局唯一的。
 */
export const mouth = new MouthTrack();

/**
 * 「妈妈骂马，你好，我是 Anima」的嘴型时间线 —— 不出声，只动嘴。
 *
 * 拿它做自检：**不需要说话、不需要 TTS、不需要任何模型文件**，
 * 就能确认「时间线 → 插值 → morph target」这后半段是通的。
 *
 * 有它是因为这条链路的失败样子只有一种（嘴不动），而可能的原因散在六个地方。
 * 能把后半段单独摘出来验，前面几环就不用一起怀疑了 ——
 * 实测吃过一次亏：嘴不动，一路去查 TTS 引擎和时间戳，结果是事件没进新 handler。
 */
export function selfTest() {
  const keys: VisemeKey[] = [
    { at_ms: 0, viseme: "PP", weight: 0.75 },
    { at_ms: 75, viseme: "aa", weight: 1 },
    { at_ms: 250, viseme: "PP", weight: 0.75 },
    { at_ms: 325, viseme: "aa", weight: 1 },
    { at_ms: 520, viseme: "nn", weight: 0.75 },
    { at_ms: 590, viseme: "I", weight: 1 },
    { at_ms: 770, viseme: "kk", weight: 0.75 },
    { at_ms: 840, viseme: "aa", weight: 1 },
    { at_ms: 960, viseme: "O", weight: 0.8 },
    { at_ms: 1100, viseme: "sil", weight: 0 },
    { at_ms: 1400, viseme: "U", weight: 0.75 },
    { at_ms: 1470, viseme: "O", weight: 1 },
    { at_ms: 1700, viseme: "CH", weight: 0.75 },
    { at_ms: 1770, viseme: "I", weight: 1 },
    { at_ms: 2000, viseme: "E", weight: 1 },
    { at_ms: 2200, viseme: "SS", weight: 0.75 },
    { at_ms: 2400, viseme: "aa", weight: 1 },
    { at_ms: 2600, viseme: "sil", weight: 0 },
  ];
  // 用一个不会和真实句子撞车的 id：真实 id 从 1 开始单调递增
  const id = Number.MAX_SAFE_INTEGER;
  mouth.stage(id, keys);
  mouth.start(id);
}
