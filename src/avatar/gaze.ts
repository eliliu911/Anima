/**
 * 视线跟随：让她看着你。
 *
 * ## 为什么这条比身体动作更值
 *
 * 「鲜活」这件事上，**眼睛的收益远高于身体**。一个会呼吸、会挥手但眼神
 * 死盯前方的模型，看久了是恐怖谷；而只要眼睛跟着光标动，哪怕身体只是
 * 静静站着，也立刻像个活的。
 *
 * ## 不是"一直盯着你"
 *
 * 这是这个模块和常见实现最大的区别。纯跟随（光标在哪就看哪，全程锁定）
 * 反而更瘆人 —— 真人不会一秒不落地盯着你的鼠标。真实的注视有三件事：
 *
 * 1. **微跳视（saccade）**：注视点不是平滑滑过去的，是一小跳一小跳的
 * 2. **偶尔移开**：每隔几秒会瞟向别处一下再回来
 * 3. **眼动先于头动**：眼睛先到位，头再慢慢跟过去，而且头转得少得多
 *
 * 这三件事都很便宜，但少了任何一件都会让人说不出哪里怪。
 *
 * ## 纯函数
 *
 * 只算角度，不碰 three 的对象 —— 那样才能测。调用方拿到角度自己往
 * 骨骼上写。
 */

/** 一次注视的解算结果，单位弧度 */
export interface GazeAngles {
  /** 眼球：左右（正=看向画面右侧）、上下（正=向上） */
  eyeYaw: number;
  eyePitch: number;
  /** 头：同样的方向，但幅度小得多，而且滞后 */
  headYaw: number;
  headPitch: number;
}

export interface GazeOptions {
  /** 眼球最大偏转。真人眼球水平极限约 ±35°，但看着舒服的在 ±20° 以内 */
  maxEyeYaw?: number;
  maxEyePitch?: number;
  /** 头跟随的比例。眼睛到位之后头才慢慢跟，而且只跟一部分 */
  headFollow?: number;
  maxHeadYaw?: number;
  maxHeadPitch?: number;
}

const DEFAULTS: Required<GazeOptions> = {
  maxEyeYaw: 0.35, // ~20°
  maxEyePitch: 0.2, // ~11°，上下比左右小：眼睑挡着，转多了会翻白眼
  headFollow: 0.45,
  maxHeadYaw: 0.42, // ~24°
  maxHeadPitch: 0.24,
};

function clamp(v: number, lo: number, hi: number) {
  return v < lo ? lo : v > hi ? hi : v;
}

/**
 * 从归一化的注视目标算出角度。
 *
 * `x` / `y` 是 -1..1 的画面坐标（0,0 是正前方，x 正=画面右侧，y 正=上）。
 */
export function anglesFor(x: number, y: number, options: GazeOptions = {}): GazeAngles {
  const o = { ...DEFAULTS, ...options };
  const nx = clamp(x, -1, 1);
  const ny = clamp(y, -1, 1);
  const eyeYaw = nx * o.maxEyeYaw;
  const eyePitch = ny * o.maxEyePitch;
  return {
    eyeYaw,
    eyePitch,
    headYaw: clamp(eyeYaw * o.headFollow, -o.maxHeadYaw, o.maxHeadYaw),
    headPitch: clamp(eyePitch * o.headFollow, -o.maxHeadPitch, o.maxHeadPitch),
  };
}

/**
 * 注视的状态机。每帧喂一个目标点，它给出**当下实际该看的**方向。
 *
 * 内部负责三件事：微跳视、偶尔移开、眼快头慢。
 */
export class Gaze {
  /** 当前实际注视点（归一化坐标），会平滑趋近目标 */
  private cx = 0;
  private cy = 0;
  /** 头的当前角度。单独存是因为它比眼睛慢 */
  private hx = 0;
  private hy = 0;
  /** 这一跳的落点。微跳视：目标不是连续跟随的，是跳过去的 */
  private jumpX = 0;
  private jumpY = 0;
  /** 距离下一次跳还有多久（秒） */
  private nextJump = 0;
  /** 正在移开视线时的偏移，以及还剩多久 */
  private awayX = 0;
  private awayY = 0;
  private awayLeft = 0;
  private nextAway = 3;

  /**
   * 随机数注入进来，测试才能确定地跑。默认用 Math.random
   */
  constructor(private rand: () => number = Math.random) {}

  /**
   * 推进一帧。
   *
   * @param target 想看的地方（-1..1）。`null` = 没有目标（光标离开了窗口），
   *               这时候视线回到正前方附近并继续小幅游移 —— 而不是僵住
   * @param dt     真实帧间隔（秒）。**不要写死 1/60** ——
   *               120Hz 屏上所有时间常数都会减半
   */
  update(target: { x: number; y: number } | null, dt: number): GazeAngles {
    const step = Math.max(0, Math.min(dt, 0.1));

    // 微跳视：每 0.35~1.1 秒重新取一次落点，落点在目标附近抖一点
    this.nextJump -= step;
    if (this.nextJump <= 0) {
      this.nextJump = 0.35 + this.rand() * 0.75;
      const jitter = 0.06;
      const base = target ?? { x: 0, y: 0 };
      this.jumpX = base.x + (this.rand() * 2 - 1) * jitter;
      this.jumpY = base.y + (this.rand() * 2 - 1) * jitter;
    } else if (target) {
      // 目标在动时落点跟着走 —— 不跟的话，两次跳视之间视线会粘在旧位置，
      // 光标已经移开了她还盯着原处
      this.jumpX = target.x;
      this.jumpY = target.y;
    }

    // 偶尔移开：每 3~9 秒瞟别处 0.4~0.9 秒
    this.nextAway -= step;
    if (this.awayLeft > 0) {
      this.awayLeft -= step;
    } else if (this.nextAway <= 0) {
      this.nextAway = 3 + this.rand() * 6;
      this.awayLeft = 0.4 + this.rand() * 0.5;
      // 移开的幅度比跟随大，方向随机
      this.awayX = (this.rand() * 2 - 1) * 0.8;
      this.awayY = (this.rand() * 2 - 1) * 0.4;
    }

    const wantX = this.awayLeft > 0 ? this.awayX : this.jumpX;
    const wantY = this.awayLeft > 0 ? this.awayY : this.jumpY;

    // 眼球：快。18/秒的指数趋近，跳视看起来才够干脆
    const eyeK = 1 - Math.exp(-18 * step);
    this.cx += (wantX - this.cx) * eyeK;
    this.cy += (wantY - this.cy) * eyeK;

    // 头：慢得多。**这个差值就是"眼动先于头动"** ——
    // 两者同速的话，整个人像个云台
    const headK = 1 - Math.exp(-2.2 * step);
    this.hx += (this.cx - this.hx) * headK;
    this.hy += (this.cy - this.hy) * headK;

    const eyes = anglesFor(this.cx, this.cy);
    const head = anglesFor(this.hx, this.hy);
    return {
      eyeYaw: eyes.eyeYaw,
      eyePitch: eyes.eyePitch,
      headYaw: head.headYaw,
      headPitch: head.headPitch,
    };
  }

  /** 当前注视点。测试和调试用 */
  get point() {
    return { x: this.cx, y: this.cy };
  }
  /** 现在是不是正在把视线移开 */
  get lookingAway() {
    return this.awayLeft > 0;
  }
}

/**
 * 光标坐标 → 归一化注视目标。
 *
 * 画布中心是 (0,0)。**除以的是半宽半高**，所以边缘正好是 ±1。
 */
export function targetFromPointer(
  clientX: number,
  clientY: number,
  rect: { left: number; top: number; width: number; height: number },
): { x: number; y: number } | null {
  if (rect.width <= 0 || rect.height <= 0) return null;
  const cx = rect.left + rect.width / 2;
  const cy = rect.top + rect.height / 2;
  // `+ 0` 把 -0 归一成 0 —— 正中间返回 `-0` 是个毛刺，
  // 而且会让"等于零"的判断在某些写法下不成立
  return {
    x: clamp((clientX - cx) / (rect.width / 2), -1, 1) + 0,
    // 屏幕 y 向下，视线 y 向上
    y: clamp(-(clientY - cy) / (rect.height / 2), -1, 1) + 0,
  };
}
