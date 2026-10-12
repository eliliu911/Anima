/**
 * 桌宠窗口的鼠标穿透状态机。
 *
 * ## 要解决的问题
 *
 * 角色层是一个盖在桌面上的无边框窗口。它必须同时满足两件互相矛盾的事：
 *
 * - **不能吃掉点击**：你点她旁边的空白时，点击要落到下面的应用上
 * - **摸得到**：光标停在她身上时要能拖动、能右键
 *
 * Tauri 的 `set_ignore_cursor_events` 是个布尔量，但直接按"光标在不在窗口里"
 * 去切会**疯狂抖动** —— 光标一移到她边缘就来回翻，表现是"有时候点得动
 * 有时候点不动"。
 *
 * ## 三态 + 两侧滞后
 *
 * ```text
 *   穿透  ──光标压在不透明像素上 250ms──▶  可交互  ──按下 / 窗口聚焦──▶  锁定
 *        ◀────────离开 250ms────────           ◀────松开 / 失焦────
 * ```
 *
 * **两侧都要滞后**。只在一个方向上加的话，反方向仍然会抖。
 *
 * ## 命中判定读的是 alpha，不是矩形
 *
 * 窗口是 360×520，但她本人只占中间一小片。按矩形判定的话她周围一大圈空白
 * 也会吃掉点击 —— 那等于没做穿透，而且症状是「她在角落，我点屏幕中间也
 * 点不动」，完全看不出原因。
 *
 * 所以调用方每帧读 canvas 上光标那一点的 alpha 传进来。读一个像素的代价
 * 可以忽略。
 */

/** 当下这一刻窗口该处于哪种状态 */
export type PassthroughState =
  /** 点击穿到下面的应用去 */
  | "through"
  /** 可拖动、可右键 */
  | "interactive"
  /** 正在拖 / 窗口聚焦，期间不许翻回穿透 */
  | "locked";

export interface PassthroughInput {
  /** 光标压着的那一点是不是不透明（alpha 超过阈值） */
  onOpaquePixel: boolean;
  /** 鼠标按住了。按住期间绝不能翻成穿透 —— 拖到一半会脱手 */
  pressed: boolean;
  /** 这个窗口是不是被聚焦了（比如刚点过它） */
  focused: boolean;
}

/** 进出都要等这么久。AIRI 用的也是 250ms */
export const HYSTERESIS_MS = 250;

export class Passthrough {
  private state: PassthroughState = "through";
  /** 想切到哪个状态，以及已经想了多久 */
  private wanted: PassthroughState = "through";
  private wantedFor = 0;

  /** 现在是什么状态 */
  get current(): PassthroughState {
    return this.state;
  }

  /** 要不要让窗口忽略鼠标事件 */
  get ignoreCursor(): boolean {
    return this.state === "through";
  }

  /**
   * 推进一帧。返回状态是否**刚刚**变了 —— 只有变了才需要去调那个系统 API，
   * 每帧都调是白白的 IPC
   *
   * @param dt 真实帧间隔（秒）。不要写死 1/60 —— 120Hz 屏上滞后会减半
   */
  update(input: PassthroughInput, dt: number): boolean {
    const next = resolve(input);

    // 锁定是立刻生效的，不等滞后。**按下的那一瞬间必须马上可交互** ——
    // 等 250ms 的话第一次点击就丢了
    if (next === "locked") {
      this.wanted = "locked";
      this.wantedFor = 0;
      return this.commit("locked");
    }

    const step = Math.max(0, dt);
    if (next !== this.wanted) {
      this.wanted = next;
      // **从这一帧开始算，不是从零。** 清零等于把这一帧的时长扔掉 ——
      // 平时只差一帧无所谓，但卡帧恢复时 dt 可能是好几秒，
      // 扔掉的话状态要多等一整轮才切
      this.wantedFor = step;
    } else {
      this.wantedFor += step;
    }

    // 从锁定退出来也要立刻 —— 松手之后应该马上恢复正常判定
    if (this.state === "locked") {
      this.wantedFor = HYSTERESIS_MS / 1000;
    }

    if (this.wanted !== this.state && this.wantedFor >= HYSTERESIS_MS / 1000) {
      return this.commit(this.wanted);
    }
    return false;
  }

  private commit(to: PassthroughState): boolean {
    if (this.state === to) return false;
    this.state = to;
    this.wanted = to;
    this.wantedFor = 0;
    return true;
  }
}

/** 输入 → 目标状态。纯函数，滞后由调用方管 */
function resolve(input: PassthroughInput): PassthroughState {
  if (input.pressed || input.focused) return "locked";
  return input.onOpaquePixel ? "interactive" : "through";
}

/**
 * 读 canvas 上某一点的 alpha。
 *
 * 坐标是**相对 canvas 左上角的 CSS 像素**；内部按 canvas 的实际像素尺寸
 * 换算 —— 高分屏上 canvas 的位图比 CSS 尺寸大，不换算会读错位置
 * （而且偏移随缩放变化，肉眼很难看出来）。
 */
export function alphaAt(
  ctx: {
    canvas: { width: number; height: number; clientWidth: number; clientHeight: number };
    readPixel: (x: number, y: number) => number;
  },
  cssX: number,
  cssY: number,
): number {
  const { canvas } = ctx;
  if (canvas.clientWidth <= 0 || canvas.clientHeight <= 0) return 0;
  const sx = canvas.width / canvas.clientWidth;
  const sy = canvas.height / canvas.clientHeight;
  const px = Math.round(cssX * sx);
  const py = Math.round(cssY * sy);
  if (px < 0 || py < 0 || px >= canvas.width || py >= canvas.height) return 0;
  return ctx.readPixel(px, py);
}
