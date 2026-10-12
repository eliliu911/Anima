/**
 * 嘴型插值器的回归测试。
 *
 * ## 这些测试守的是什么
 *
 * 插值器是整条对口型链路里**唯一没法从画面上判断对错**的一环。口型不对的时候
 * 画面只会显得"有点假"，而原因可能在时间线、在对齐、在插值、在模型的
 * morph target 名字上 —— 五个地方看起来一模一样。
 *
 * 所以这里的每条断言都对应一个**真实出现过或真实会出现**的错法，不是为了覆盖率。
 *
 * ## 时钟
 *
 * 插值器读 `performance.now()`。测试里把它替换成一个可控的假时钟 ——
 * 真实时钟下没法稳定复现"第 300 毫秒时嘴该是什么样"。
 */
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { mouth, VISEMES, type MouthWeights } from "./mouth";
import type { VisemeKey } from "../api/voice";

let clock = 0;
const realNow = performance.now.bind(performance);

beforeEach(() => {
  clock = 0;
  performance.now = () => clock;
  mouth.stop();
});

afterEach(() => {
  performance.now = realNow;
});

/** 「妈妈骂马」的真实时间线（Rust 侧 `avatar::viseme::schedule` 的实际产出） */
const MAMA: VisemeKey[] = [
  { at_ms: 0, viseme: "PP", weight: 0.75 },
  { at_ms: 75, viseme: "aa", weight: 1 },
  { at_ms: 250, viseme: "PP", weight: 0.75 },
  { at_ms: 325, viseme: "aa", weight: 1 },
  { at_ms: 500, viseme: "PP", weight: 0.75 },
  { at_ms: 575, viseme: "aa", weight: 1 },
  { at_ms: 750, viseme: "PP", weight: 0.75 },
  { at_ms: 825, viseme: "aa", weight: 1 },
  { at_ms: 1000, viseme: "sil", weight: 0 },
];

function at(ms: number): MouthWeights {
  clock = ms;
  return mouth.sample();
}

function play(keys: VisemeKey[], id = 1) {
  mouth.stage(id, keys);
  mouth.start(id);
}

/** 当前最强的那个口型 */
function dominant(w: MouthWeights): string {
  return VISEMES.reduce((best, v) => (w[v] > w[best] ? v : best), "sil");
}

describe("时间原点", () => {
  it("收到时间线但还没开始播时，嘴是闭着的", () => {
    // 句子是排队播的，`words` 到了不等于这一句在出声。
    // 这里要是开始动，嘴型会比声音早好几秒
    mouth.stage(1, MAMA);
    expect(at(0).sil).toBe(1);
    expect(at(500).sil).toBe(1);
  });

  it("sentence_start 之后才起算，t=0 是那一刻", () => {
    // 直接验语义：同一条时间线，不管在什么时刻起播，
    // **相对偏移相同的时刻嘴型必须完全一样**。
    // 拿收到 words 的时刻当 t=0 的话，这两条曲线会错开。
    const offsets = [0, 120, 300, 600, 900];

    mouth.stage(1, MAMA);
    mouth.start(1);
    const early = offsets.map((o) => ({ ...at(o) }));

    mouth.stop();
    mouth.stage(2, MAMA);
    clock = 5000; // 前面几句已经播了 5 秒
    mouth.start(2);
    const late = offsets.map((o) => ({ ...at(5000 + o) }));

    expect(late).toEqual(early);
  });

  it("配不上的 id 不会起播", () => {
    mouth.stage(1, MAMA);
    mouth.start(999);
    expect(at(100).sil).toBe(1);
  });
});

describe("插值本身", () => {
  it("元音在整个音节里保持，不会退回中性", () => {
    // **这条是真出现过的 bug。** 原先写成脉冲式衰减：到点冲上去、按固定时长
    // 退回中性。衰减本身非常平滑（所以"限制单帧变幅"那类断言全都会通过），
    // 但音节之间嘴会退到几乎全闭，看起来是在扇动而不是在说话。
    //
    // 断言的是语义而不是平滑度：从第一个元音到最后一个关键帧之间，
    // 嘴不许回到接近闭合。
    play(MAMA);
    let worstSil = 0;
    let worstAt = -1;
    for (let t = 100; t <= 950; t += 10) {
      const s = at(t).sil;
      if (s > worstSil) {
        worstSil = s;
        worstAt = t;
      }
    }
    expect(
      worstSil,
      `发声段里嘴退到了 sil=${worstSil.toFixed(2)}（${worstAt}ms）—— 这是在扇动，不是在说话`,
    ).toBeLessThan(0.5);
  });

  it("相邻口型之间是交叉淡化，不是硬切", () => {
    play(MAMA);
    // 250ms 是 aa → PP 的交接点，之后的一小段里两者应当同时有份额
    const mid = at(280);
    expect(mid.PP).toBeGreaterThan(0.05);
    expect(mid.aa).toBeGreaterThan(0.05);
  });

  it("单帧变化幅度有限，不会闪", () => {
    // 这条是 Anti 提的那种断言。它抓不到上面那个 bug（衰减也是平滑的），
    // 但它能抓另一类错：交接窗口写成 0、或者游标跳帧导致口型瞬间切换
    play(MAMA);
    let prev = at(0);
    for (let t = 16; t <= 1100; t += 16) {
      const now = at(t);
      for (const v of VISEMES) {
        expect(
          Math.abs(now[v] - prev[v]),
          `${v} 在 ${t}ms 单帧跳了 ${Math.abs(now[v] - prev[v]).toFixed(2)}`,
        ).toBeLessThan(0.4);
      }
      prev = now;
    }
  });

  it("权重不会超过 1", () => {
    // 前后两帧指向同一个口型时如果相加，嘴会张到穿模
    const same: VisemeKey[] = [
      { at_ms: 0, viseme: "aa", weight: 1 },
      { at_ms: 30, viseme: "aa", weight: 1 },
      { at_ms: 60, viseme: "aa", weight: 1 },
    ];
    play(same);
    for (let t = 0; t <= 120; t += 5) {
      for (const v of VISEMES) expect(at(t)[v]).toBeLessThanOrEqual(1);
    }
  });

  it("停顿处真的闭嘴", () => {
    // Rust 侧会在词间空档插一帧 sil。前端得让它真的生效，
    // 否则「你好，我是」的逗号处嘴会一直张着
    play([
      { at_ms: 0, viseme: "aa", weight: 1 },
      { at_ms: 300, viseme: "sil", weight: 0 },
      { at_ms: 800, viseme: "O", weight: 1 },
      { at_ms: 1100, viseme: "sil", weight: 0 },
    ]);
    expect(at(500).sil).toBeGreaterThan(0.9);
    expect(dominant(at(900))).toBe("O");
  });

  it("说完之后闭嘴并停下", () => {
    // 少了这一条的表现是「说完嘴半张着挂在脸上」
    play(MAMA);
    at(500);
    expect(at(1300).sil).toBe(1);
    expect(mouth.active).toBe(false);
  });
});

describe("打断", () => {
  it("停下之后立刻回到闭嘴，且不会被残留的时间线带着继续动", () => {
    play(MAMA);
    expect(dominant(at(400))).not.toBe("sil");
    mouth.stop();
    expect(at(450).sil).toBe(1);
    expect(at(600).sil).toBe(1);
  });

  it("停下会清掉还没起播的句子", () => {
    // 打断时后面几句可能已经收到时间线但还没出声。不清掉的话，
    // 下一轮的 sentence_start 会让**上一轮**的句子开始动嘴
    mouth.stage(7, MAMA);
    mouth.stop();
    mouth.start(7);
    expect(at(100).sil).toBe(1);
  });
});

describe("边界", () => {
  it("空时间线不崩", () => {
    play([]);
    expect(at(0).sil).toBe(1);
  });

  it("暂存表不会无限增长", () => {
    // 用户在某句入队前就打断的话，那句的 sentence_start 永远不会来。
    // 不清理的话这张表会随会话一直长
    for (let i = 0; i < 50; i++) mouth.stage(i, MAMA);
    // 最老的那些应当已经被丢掉 —— 用「最老的起不播」来验
    mouth.start(0);
    expect(at(50).sil).toBe(1);
    // 最新的仍然能起播。先把时钟归零再起播，采样点取在元音正中间 ——
    // 落在交接的 60ms 淡化窗口里的话，两个口型的份额接近，谁"占优"没有意义
    clock = 0;
    mouth.start(49);
    expect(dominant(at(200))).toBe("aa");
  });
});

describe("音频驱动的口型", () => {
  /// play_audio / 电台拿不到文本，时间线那条不存在 ——
  /// 这些场景以前嘴完全不动
  it("没有时间线时用音频那条", () => {
    mouth.audio("aa", 0.8);
    const w = mouth.sample();
    expect(w.aa).toBeCloseTo(0.8, 2);
    expect(w.sil).toBeCloseTo(0.2, 2);
  });

  /// **时间线在跑时必须忽略音频那条。** 时间线有 15 个 viseme、含辅音，
  /// 被这条只分元音的盖掉是退步，而且看不出是被盖了
  it("时间线在跑时忽略音频那条", () => {
    mouth.stage(1, MAMA);
    mouth.start(1);
    mouth.audio("aa", 1);
    const w = at(80);
    expect(w.aa).toBeGreaterThan(0);
    // 这个 aa 来自时间线（MAMA 在 75ms 是 aa），不是音频那条 ——
    // 用 PP 段验更干净
    const early = at(0);
    expect(early.PP).toBeGreaterThan(0);
  });

  /// **要过期。** 音频停了事件就不来了，不过期的话嘴会僵在最后那个形状上。
  /// 一帧 64ms，宽限 200ms
  it("音频停了之后嘴要闭上", () => {
    mouth.audio("aa", 0.9);
    expect(mouth.sample().aa).toBeGreaterThan(0);
    clock = 260;
    const w = mouth.sample();
    expect(w.aa).toBe(0);
    expect(w.sil).toBe(1);
  });

  it("stop() 也要清掉音频那条", () => {
    mouth.audio("aa", 0.9);
    mouth.stop();
    expect(mouth.sample().sil).toBe(1);
  });
});
