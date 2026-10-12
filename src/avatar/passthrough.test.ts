import { describe, expect, it } from "vitest";
import { alphaAt, HYSTERESIS_MS, Passthrough, type PassthroughInput } from "./passthrough";

const IDLE: PassthroughInput = { onOpaquePixel: false, pressed: false, focused: false };
const ON_HER: PassthroughInput = { onOpaquePixel: true, pressed: false, focused: false };
const DT = 1 / 60;
const HOLD = HYSTERESIS_MS / 1000;

/** 保持某个输入若干秒 */
function hold(p: Passthrough, input: PassthroughInput, secs: number) {
  for (let t = 0; t < secs; t += DT) p.update(input, DT);
}

describe("穿透状态机", () => {
  /// 默认必须是穿透 —— 一启动就吃掉整块屏幕的点击是最糟的默认值
  it("默认穿透", () => {
    const p = new Passthrough();
    expect(p.current).toBe("through");
    expect(p.ignoreCursor).toBe(true);
  });

  it("光标压在她身上够久才变可交互", () => {
    const p = new Passthrough();
    hold(p, ON_HER, HOLD * 0.6);
    expect(p.current, "还没到滞后时间就切了").toBe("through");
    hold(p, ON_HER, HOLD * 0.8);
    expect(p.current).toBe("interactive");
    expect(p.ignoreCursor).toBe(false);
  });

  it("离开也要够久才变回穿透", () => {
    const p = new Passthrough();
    hold(p, ON_HER, HOLD * 2);
    expect(p.current).toBe("interactive");

    hold(p, IDLE, HOLD * 0.6);
    expect(p.current, "刚离开就切回去了").toBe("interactive");
    hold(p, IDLE, HOLD * 0.8);
    expect(p.current).toBe("through");
  });

  /// **这是整个滞后存在的理由。** 光标沿着她的边缘擦过去时，
  /// 命中判定会飞快地真假交替 —— 没有滞后的话窗口会疯狂翻转，
  /// 表现是"有时候点得动有时候点不动"
  it("光标擦过边缘不会让状态抖动", () => {
    const p = new Passthrough();
    let flips = 0;
    for (let i = 0; i < 240; i++) {
      // 每两帧翻一次，模拟贴着边缘移动
      const on = i % 2 === 0;
      if (p.update({ ...IDLE, onOpaquePixel: on }, DT)) flips++;
    }
    expect(flips, `抖了 ${flips} 次`).toBe(0);
  });

  /// **按下的那一瞬间必须立刻可交互，不能等滞后** ——
  /// 等 250ms 的话第一次点击就丢了
  it("按下立刻锁定，不等滞后", () => {
    const p = new Passthrough();
    const changed = p.update({ ...IDLE, pressed: true }, DT);
    expect(changed).toBe(true);
    expect(p.current).toBe("locked");
    expect(p.ignoreCursor).toBe(false);
  });

  /// 拖动过程中光标很容易滑出她的轮廓。这时候要是翻回穿透，
  /// **窗口会直接脱手**
  it("按住期间光标滑出轮廓也不会松手", () => {
    const p = new Passthrough();
    hold(p, { ...ON_HER, pressed: true }, HOLD);
    hold(p, { ...IDLE, pressed: true }, HOLD * 3);
    expect(p.current).toBe("locked");
    expect(p.ignoreCursor).toBe(false);
  });

  /// 松手之后要恢复正常判定，而且不用再等一轮滞后
  it("松手后立刻恢复判定", () => {
    const p = new Passthrough();
    hold(p, { ...ON_HER, pressed: true }, HOLD);
    p.update(IDLE, DT);
    expect(p.current).toBe("through");
  });

  it("窗口聚焦时保持可交互", () => {
    const p = new Passthrough();
    hold(p, { ...IDLE, focused: true }, HOLD * 2);
    expect(p.current).toBe("locked");
    expect(p.ignoreCursor).toBe(false);
  });

  /// 状态没变就不该回 true —— 每帧都去调那个系统 API 是白白的 IPC
  it("状态没变时不报告变化", () => {
    const p = new Passthrough();
    hold(p, ON_HER, HOLD * 2);
    let changes = 0;
    for (let i = 0; i < 120; i++) if (p.update(ON_HER, DT)) changes++;
    expect(changes).toBe(0);
  });

  /// **滞后必须按真实 dt 走。** 写死 1/60 的话，120Hz 屏上滞后只有 125ms
  it("120Hz 下的滞后时长和 60Hz 一致", () => {
    const fast = new Passthrough();
    for (let t = 0; t < HOLD * 0.6; t += 1 / 120) fast.update(ON_HER, 1 / 120);
    expect(fast.current, "高刷屏上滞后被砍半了").toBe("through");
    for (let t = 0; t < HOLD * 0.6; t += 1 / 120) fast.update(ON_HER, 1 / 120);
    expect(fast.current).toBe("interactive");
  });

  /// 卡帧（切回标签页）时 dt 会是个很大的数，不该炸
  it("一个巨大的 dt 只是让状态立刻切过去", () => {
    const p = new Passthrough();
    p.update(ON_HER, 10);
    expect(p.current).toBe("interactive");
  });
});

describe("alpha 命中判定", () => {
  function ctx(scale: number, pixels: Record<string, number> = {}) {
    return {
      canvas: { width: 200 * scale, height: 100 * scale, clientWidth: 200, clientHeight: 100 },
      readPixel: (x: number, y: number) => pixels[`${x},${y}`] ?? 0,
    };
  }

  it("按 CSS 坐标读到对应的像素", () => {
    expect(alphaAt(ctx(1, { "10,20": 0.9 }), 10, 20)).toBe(0.9);
  });

  /// **高分屏上 canvas 的位图比 CSS 尺寸大。** 不换算的话读的是错位置 ——
  /// 而且偏移随缩放变化，肉眼几乎看不出来
  it("高分屏要按位图尺寸换算", () => {
    expect(alphaAt(ctx(2, { "20,40": 0.8 }), 10, 20)).toBe(0.8);
    expect(alphaAt(ctx(2, { "10,20": 0.8 }), 10, 20), "没换算").toBe(0);
  });

  it("越界返回 0，而不是读到别的像素", () => {
    const c = ctx(1, { "0,0": 1 });
    expect(alphaAt(c, -5, 10)).toBe(0);
    expect(alphaAt(c, 999, 10)).toBe(0);
  });

  /// 画布还没布局时不能除以零
  it("画布尺寸为零时返回 0", () => {
    const c = { canvas: { width: 0, height: 0, clientWidth: 0, clientHeight: 0 }, readPixel: () => 1 };
    expect(alphaAt(c, 5, 5)).toBe(0);
  });
});

describe("穿透时的死锁", () => {
  /**
   * **这条是真实事故的回归测试。**
   *
   * 第一版靠 `pointermove` 判断"光标压在她身上"。但窗口一旦穿透就
   * 收不到任何指针事件 —— 于是判不出压没压着、于是永远退不出穿透、
   * 于是永远收不到事件。一个闭环，表现是"鼠标怎么都定位不到她"。
   *
   * 这里模拟那个错误：输入源在穿透态下恒为 false。状态机本身没错，
   * 但整套东西卡死 —— 所以修法只能是**换一个不依赖窗口事件的输入源**
   * （轮询系统光标），而不是改状态机。
   */
  it("输入源依赖窗口事件时会永久卡在穿透（所以不能那样做）", () => {
    const p = new Passthrough();
    for (let t = 0; t < 5; t += DT) {
      // 穿透时收不到事件 → 永远报 false
      const canSeeCursor = !p.ignoreCursor;
      p.update({ onOpaquePixel: canSeeCursor, pressed: false, focused: false }, DT);
    }
    expect(p.current, "如果这条变成 interactive，说明模拟写错了").toBe("through");
  });

  /// 换成不依赖窗口事件的输入源（轮询系统光标）之后，同样的时间里能正常切出去
  it("输入源独立于窗口事件时能正常退出穿透", () => {
    const p = new Passthrough();
    hold(p, ON_HER, HOLD * 2);
    expect(p.current).toBe("interactive");
  });
});
