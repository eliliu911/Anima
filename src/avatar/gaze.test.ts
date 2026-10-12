import { describe, expect, it } from "vitest";
import { anglesFor, Gaze, targetFromPointer } from "./gaze";

/** 可控的伪随机，测试才能确定地跑 */
function seeded(seed: number) {
  let s = seed;
  return () => {
    s = (s * 1664525 + 1013904223) % 4294967296;
    return s / 4294967296;
  };
}

/** 跑 n 秒，每帧 dt */
function run(g: Gaze, target: { x: number; y: number } | null, secs: number, dt = 1 / 60) {
  let last = g.update(target, dt);
  for (let t = 0; t < secs; t += dt) last = g.update(target, dt);
  return last;
}

describe("注视角度", () => {
  it("正前方就是零", () => {
    const a = anglesFor(0, 0);
    expect(a.eyeYaw).toBe(0);
    expect(a.eyePitch).toBe(0);
    expect(a.headYaw).toBe(0);
  });

  /// **头只跟一部分，而且幅度明显小于眼睛。** 两者一样大的话，
  /// 整个人像个云台在转，不像在看你
  it("头转得比眼睛少", () => {
    const a = anglesFor(1, 1);
    expect(Math.abs(a.headYaw)).toBeLessThan(Math.abs(a.eyeYaw));
    expect(Math.abs(a.headPitch)).toBeLessThan(Math.abs(a.eyePitch));
  });

  /// 上下比左右小：眼睑挡着，上下转多了会翻白眼
  it("上下的幅度小于左右", () => {
    const a = anglesFor(1, 1);
    expect(Math.abs(a.eyePitch)).toBeLessThan(Math.abs(a.eyeYaw));
  });

  /// 超出画面的坐标不能算出越界的角度 —— 眼球会翻过去
  it("超出范围的输入会被夹住", () => {
    const far = anglesFor(50, -50);
    const edge = anglesFor(1, -1);
    expect(far.eyeYaw).toBeCloseTo(edge.eyeYaw, 6);
    expect(far.eyePitch).toBeCloseTo(edge.eyePitch, 6);
  });

  it("方向是对的：右上就是右上", () => {
    const a = anglesFor(0.5, 0.5);
    expect(a.eyeYaw).toBeGreaterThan(0);
    expect(a.eyePitch).toBeGreaterThan(0);
  });
});

describe("注视状态机", () => {
  /// 基本要求：目标在右边，**大部分时间**视线要在右边。
  ///
  /// 不能断言"任意时刻都在"——她会偶尔移开，那是有意的（见下一条）。
  /// 所以这里测的是占比
  it("大部分时间看着目标", () => {
    const g = new Gaze(seeded(1));
    let on = 0;
    let total = 0;
    for (let t = 0; t < 20; t += 1 / 60) {
      g.update({ x: 0.8, y: 0 }, 1 / 60);
      total++;
      if (g.point.x > 0.5) on++;
    }
    expect(on / total, "看着目标的时间占比太低").toBeGreaterThan(0.7);
  });

  /// **眼动先于头动。** 目标突然移到一侧的那一瞬间，
  /// 眼睛应该已经过去了，头还在后面
  it("刚开始跟的时候眼睛比头快", () => {
    const g = new Gaze(seeded(7));
    let a = g.update({ x: 1, y: 0 }, 1 / 60);
    for (let i = 0; i < 12; i++) a = g.update({ x: 1, y: 0 }, 1 / 60);
    // 归一化到各自的上限再比，否则比的是量纲不是进度
    const eyeProgress = Math.abs(a.eyeYaw) / 0.35;
    const headProgress = Math.abs(a.headYaw) / 0.42;
    expect(eyeProgress).toBeGreaterThan(headProgress * 1.5);
  });

  /// **不能一秒不落地盯着。** 纯跟随比不跟随更瘆人 ——
  /// 真人每隔几秒会瞟向别处
  it("过一段时间会把视线移开", () => {
    const g = new Gaze(seeded(3));
    let sawAway = false;
    for (let t = 0; t < 30; t += 1 / 60) {
      g.update({ x: 0, y: 0 }, 1 / 60);
      if (g.lookingAway) sawAway = true;
    }
    expect(sawAway, "30 秒里一次都没移开视线").toBe(true);
  });

  /// 移开是暂时的，要回来
  it("移开之后会回到目标上", () => {
    const g = new Gaze(seeded(3));
    // 先跑到它移开为止
    for (let t = 0; t < 30 && !g.lookingAway; t += 1 / 60) g.update({ x: 0.7, y: 0 }, 1 / 60);
    expect(g.lookingAway).toBe(true);
    // 再跑够长时间，应该已经回来了
    run(g, { x: 0.7, y: 0 }, 4);
    expect(g.lookingAway).toBe(false);
    expect(g.point.x).toBeGreaterThan(0.3);
  });

  /// 光标离开窗口时视线回正，**而不是僵在最后那个方向**
  it("没有目标时视线回到正前方附近", () => {
    const g = new Gaze(seeded(11));
    // 先确实看向一侧（取这段时间里的最大偏移，避开"移开视线"那几帧）
    let peak = 0;
    for (let t = 0; t < 5; t += 1 / 60) {
      g.update({ x: 1, y: 0 }, 1 / 60);
      peak = Math.max(peak, g.point.x);
    }
    expect(peak).toBeGreaterThan(0.6);

    // 光标离开窗口之后，平均位置要回到正前方附近 —— 而不是僵在最后那个方向
    let sum = 0;
    let n = 0;
    for (let t = 0; t < 10; t += 1 / 60) {
      g.update(null, 1 / 60);
      sum += g.point.x;
      n++;
    }
    expect(Math.abs(sum / n), "视线没回正").toBeLessThan(0.25);
  });

  /// **时间常数必须按真实 dt 走。** 写死 1/60 的话，120Hz 屏上
  /// 所有节奏都会快一倍 —— 这个 bug 只在高刷屏上出现，最难发现
  it("120Hz 和 60Hz 跑出来的进度一致", () => {
    const a = new Gaze(seeded(5));
    const b = new Gaze(seeded(5));
    run(a, { x: 0.9, y: 0.3 }, 2, 1 / 60);
    run(b, { x: 0.9, y: 0.3 }, 2, 1 / 120);
    expect(b.point.x).toBeCloseTo(a.point.x, 1);
  });

  /// 卡帧（标签页切回来）时 dt 会是个很大的数，不能让状态炸掉
  it("一个巨大的 dt 不会把状态弄坏", () => {
    const g = new Gaze(seeded(2));
    const a = g.update({ x: 0.5, y: 0 }, 5);
    expect(Number.isFinite(a.eyeYaw)).toBe(true);
    expect(Math.abs(g.point.x)).toBeLessThanOrEqual(1.2);
  });
});

describe("光标 → 注视目标", () => {
  const rect = { left: 0, top: 0, width: 400, height: 400 };

  it("正中间是零", () => {
    expect(targetFromPointer(200, 200, rect)).toEqual({ x: 0, y: 0 });
  });

  /// **屏幕 y 向下，视线 y 向上。** 不翻的话光标往上她往下看
  it("y 轴要翻过来", () => {
    const up = targetFromPointer(200, 0, rect)!;
    expect(up.y).toBeGreaterThan(0);
    const down = targetFromPointer(200, 400, rect)!;
    expect(down.y).toBeLessThan(0);
  });

  /// 边缘正好是 ±1 —— 除以半宽而不是全宽
  it("边缘是 ±1", () => {
    expect(targetFromPointer(400, 200, rect)!.x).toBeCloseTo(1, 6);
    expect(targetFromPointer(0, 200, rect)!.x).toBeCloseTo(-1, 6);
  });

  it("画布还没布局时返回 null，而不是除以零", () => {
    expect(targetFromPointer(10, 10, { left: 0, top: 0, width: 0, height: 0 })).toBeNull();
  });

  /// 画布不在原点时也要对 —— 独立窗口里这是常态
  it("画布有偏移时仍然以它的中心为原点", () => {
    const offset = { left: 100, top: 50, width: 200, height: 200 };
    expect(targetFromPointer(200, 150, offset)).toEqual({ x: 0, y: 0 });
  });
});
