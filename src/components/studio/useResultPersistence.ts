import { useEffect, useMemo, useRef } from "react";
import { studioResults, studioSaveResults } from "../../api/studio";
import { createResultPersistence, settledKey } from "./resultPersistence";
import { useStudioStore, type StudioResult } from "./useStudioStore";

/**
 * 把结果画廊接上磁盘。
 *
 * # 这个功能一直"做了但没接"
 *
 * Rust 侧的 `studio_results` / `studio_save_results` 早就写好了，还带着
 * `reconcile_history`（重启后把还挂着 running 的卡片丢掉、文件没了的丢掉、
 * 超上限的截掉），也有测试。**但前端一行都没调过** ——
 * 所以每次重开应用结果都是空的，而且因为画廊在空的时候连头部都不渲染，
 * 连带着"分类""视图切换"也一起看不见了。三个现象，一个根因。
 *
 * # 写盘的时机
 *
 * 只在**已结束的结果**发生变化时写（见 `settledKey`）。一次生成期间
 * `results` 会因为进度更新变很多次，每次都写就是几百次没必要的 IO；
 * 而那些中间态读回来时本来就会被 `reconcile_history` 过滤掉。
 */
export function useResultPersistence() {
  const results = useStudioStore((s) => s.results);
  const setResults = useStudioStore((s) => s.setResults);

  const store = useMemo(
    () =>
      createResultPersistence(
        () => studioResults() as Promise<StudioResult[]>,
        (items) => studioSaveResults(items),
      ),
    [],
  );

  useEffect(() => {
    void store.hydrate((items) => {
      // **只在还没有结果时才铺进去。** 读盘是异步的，这期间用户可能已经
      // 点了生成 —— 直接覆盖会把那一条刚提交的抹掉。
      const cur = useStudioStore.getState().results;
      if (cur.length === 0 && items.length > 0) setResults(items);
    });
  }, [store, setResults]);

  const lastKey = useRef<string | null>(null);
  useEffect(() => {
    const key = settledKey(results);
    if (key === lastKey.current) return;
    lastKey.current = key;
    void store.persist(results).catch(() => {
      // 存不下不该打断创作。下一次结束的结果会再试一遍。
    });
  }, [results, store]);
}
