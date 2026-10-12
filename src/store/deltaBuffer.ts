/**
 * 流式增量的合帧缓冲。
 *
 * ## 为什么需要它
 *
 * 后端按 token 推 delta，而 store 里每一次更新都会换掉整个 `turns` 数组
 * —— 于是 `ChatHistory` 的 `turns.map` 里**每一轮**都重渲染，每轮的 markdown
 * 都被 remark 重新解析一遍。这份成本随对话长度线性增长，而它是**每个 delta
 * 付一遍**的：实测 20 轮 × 7.7KB 的对话约 94ms 一次，一条 400 delta 的流
 * 累计能到 30~40 秒。
 *
 * 表现就是「流到一半不动了、计时器和 token 也停、加载动画还在转，
 * 几十秒后忽然全部出来」。事件一直在到，是**主线程被渲染占死了**，
 * 排队的更新等到流结束才追上。窗口拖不动也是同一个原因 ——
 * `decorations: false`，拖拽由 webview 里的 JS 处理。
 *
 * ## 两个刻意的选择
 *
 * 1. **用定时器而不是 `requestAnimationFrame`。** rAF 在窗口最小化或隐藏时
 *    会被暂停，那样纯文本的流会一直堆着不渲染。这里要的是节流不是绘制对齐，
 *    定时器在后台窗口里照常走。
 * 2. **非增量事件必须先 `flush()` 再处理。** 工具调用、结果、结束都得看到
 *    它前面那段文字已经落盘，否则工具卡片会插到文字前面去。
 */

/** 一个轮次待写入的增量。 */
export interface PendingDelta {
  text: string;
  reasoning: string;
}

export type DeltaField = keyof PendingDelta;

/** 合帧窗口（毫秒）。约等于 60fps —— 再密人眼也看不出来，只是白烧主线程。 */
export const FLUSH_INTERVAL_MS = 16;

export class DeltaBuffer {
  private pending = new Map<string, PendingDelta>();
  private timer: ReturnType<typeof setTimeout> | null = null;

  constructor(
    private readonly onFlush: (batch: Map<string, PendingDelta>) => void,
    private readonly intervalMs: number = FLUSH_INTERVAL_MS,
  ) {}

  /** 攒一个增量。窗口内的后续增量会并进同一批。 */
  push(turnId: string, field: DeltaField, delta: string): void {
    if (!delta) return;
    const cur = this.pending.get(turnId) ?? { text: "", reasoning: "" };
    cur[field] += delta;
    this.pending.set(turnId, cur);
    if (this.timer === null) {
      this.timer = setTimeout(() => this.flush(), this.intervalMs);
    }
  }

  /**
   * 立刻写出所有攒着的增量。
   *
   * 处理任何**非增量**事件之前都必须调一次，否则顺序会乱。
   * 没有待写内容时是空操作 —— 不会触发多余的状态更新。
   */
  flush(): void {
    if (this.timer !== null) {
      clearTimeout(this.timer);
      this.timer = null;
    }
    if (this.pending.size === 0) return;
    const batch = this.pending;
    this.pending = new Map();
    this.onFlush(batch);
  }

  /** 丢弃攒着的内容（会话切换、清空对话时用），不触发写出。 */
  discard(): void {
    if (this.timer !== null) {
      clearTimeout(this.timer);
      this.timer = null;
    }
    this.pending.clear();
  }

  /** 仅供测试与诊断。 */
  get pendingCount(): number {
    return this.pending.size;
  }
}
