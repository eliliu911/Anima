/**
 * 导演世界的两种存盘（审查 2026-09-27 指出的两处竞态）：
 *
 * - 场面（人偶 / 机位 / 输出）→ `createSerialWriter`：一个接一个写，后发的一定后落盘；
 *   关窗时等**队列里全部**写完，不是只等最后发出去的那一个。
 * - 校准（眼高 / 片场缩放）→ `createDebouncedSave`：停手再存，但关窗时 `flush()` 立刻把没存的存掉并等它落盘 ——
 *   以前关窗只清掉定时器，调完滑块马上关，最后一次就丢了。
 */

export interface SerialWriter {
  /** 排到队尾执行。前一个失败不影响后一个 */
  run<T>(job: () => Promise<T>): Promise<T>;
  /** 等到此刻已经排进来的全部执行完（成功失败都算完） */
  idle(): Promise<void>;
}

export function createSerialWriter(): SerialWriter {
  let tail: Promise<void> = Promise.resolve();
  return {
    run(job) {
      const p = tail.then(job);
      tail = p.then(
        () => undefined,
        () => undefined,
      );
      return p;
    },
    idle: () => tail,
  };
}

export interface DebouncedSave {
  /** 重新计时；到点执行一次（失败交给 `onError`） */
  schedule(): void;
  /** 有没存的立刻存，等所有存盘结束。**最后一次存失败就 reject** —— 调用方据此决定窗口关不关 */
  flush(): Promise<void>;
  /** 有没有还没发出去的 */
  pending(): boolean;
}

export function createDebouncedSave(save: () => Promise<void>, ms: number, onError: (e: unknown) => void): DebouncedSave {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let last: Promise<void> = Promise.resolve();
  const fire = () => {
    timer = null;
    // 接在上一次后面：同一份资产前后两次写不能乱序
    last = last.catch(() => undefined).then(save);
    return last;
  };
  return {
    schedule() {
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => void fire().catch(onError), ms);
    },
    flush() {
      if (timer) {
        clearTimeout(timer);
        return fire();
      }
      return last;
    },
    pending: () => timer !== null,
  };
}
