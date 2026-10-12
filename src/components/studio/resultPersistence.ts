import type { StudioResult } from "./useStudioStore";

/**
 * 结果画廊的存读。
 *
 * # 为什么要单拎出来
 *
 * 这里有一条**不做就会静默毁数据**的不变量：
 *
 * > **读回来之前，一次都不能写。**
 *
 * 画廊的状态初值是空数组。挂载时如果「保存」先于「读取」跑到，
 * 那一份空数组会把磁盘上几百条历史整个覆盖掉 —— 不报错，
 * 而且用户下次打开只会觉得"记录又没了"，根本不会怀疑是被写空的。
 *
 * # 为什么整份覆盖而不是增量
 *
 * 前端那份是唯一真相（Rust 侧 `studio_save_results` 的注释也这么写）。
 * 增量合并意味着两边各存一份状态，删一条的时候必然打架。
 */
export interface ResultPersistence {
  /** 读回上次的结果，交给 `apply`。读完才允许写。 */
  hydrate(apply: (items: StudioResult[]) => void): Promise<void>;
  /** 写。**没读回来之前直接拒绝**，返回 false。 */
  persist(items: StudioResult[]): Promise<boolean>;
}

export function createResultPersistence(
  load: () => Promise<StudioResult[]>,
  save: (items: StudioResult[]) => Promise<unknown>,
): ResultPersistence {
  let loaded = false;
  return {
    async hydrate(apply) {
      try {
        const items = await load();
        apply(items);
      } catch {
        // 读不回来就当没有历史。**但仍然放开写** ——
        // 否则一次读失败会让这一整轮的新结果全都存不下。
      } finally {
        loaded = true;
      }
    },
    async persist(items) {
      if (!loaded) return false;
      await save(items);
      return true;
    },
  };
}

/**
 * 值得写盘吗。
 *
 * **正在跑的不算变化。** 一次生成期间 `results` 会因为进度更新变很多次，
 * 每次都写盘就是几百次没必要的 IO；而这些中间态读回来时本来就会被
 * `reconcile_history` 丢掉（它把 pending / running 直接过滤掉）。
 *
 * 所以只看「已结束的那些」有没有变。
 */
export function settledKey(items: StudioResult[]): string {
  return items
    .filter((r) => r.status !== "pending" && r.status !== "running")
    .map((r) => `${r.id}:${r.status}:${r.path ?? ""}`)
    .join("|");
}
