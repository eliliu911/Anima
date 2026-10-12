import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useStudioStore, type StudioResult } from "./useStudioStore";

/**
 * 内存里已有的 + 磁盘上读回来的。
 *
 * **合并，不是覆盖。** 这个 hook 挂载之前可能已经有人点了生成
 * （读盘是异步的，一两百毫秒足够点一次），直接 `setResults(saved)`
 * 会把那一条正在跑的顶掉 —— 它还在跑，但界面上没有了，
 * 而且不报错，看起来就是「点了没反应」。
 *
 * 同 id 的以**内存里那份为准**：它是活的，磁盘上那份是旧快照。
 */
export function mergeHistory(current: StudioResult[], saved: StudioResult[]): StudioResult[] {
  const seen = new Set(current.map((r) => r.id));
  return [...current, ...saved.filter((r) => !seen.has(r.id))];
}

/**
 * 结果画廊的持久化。
 *
 * # 为什么需要
 *
 * 出的文件一直落在 `.anima/generated/` 下，**没丢过**。丢的是
 * 「这张图是用什么提示词、什么模式、什么引擎、什么 seed 出的」——
 * 那些只活在内存里，一重启就只剩一堆文件名，等于白出。
 *
 * # 什么时候写
 *
 * 结果一变就写，停手 600ms 之后。不做防抖的话，一次出四张 + 每张
 * pending→running→done 三次状态变化 = 12 次整份覆写。
 *
 * # 一个真会踩的坑
 *
 * **装载完成之前不能写。** 挂载那一瞬间 `results` 是空数组，
 * 如果这时候就触发一次保存，磁盘上的历史会被空数组**当场清掉** ——
 * 而且是在用户还没看到画廊之前，看起来就是「历史又没了」。
 * 所以用 `loaded` 把第一次写挡住。换工作区时同理 —— 重新装载之前先关闸，
 * 否则旧工作区那份历史会被写进新工作区的文件里。
 */
export function useResultHistory(workspacePath?: string | null) {
  const results = useStudioStore((s) => s.results);
  const setResults = useStudioStore((s) => s.setResults);
  const loaded = useRef(false);

  useEffect(() => {
    let alive = true;
    // 换工作区 = 换一份历史。**先关闸**，否则旧工作区那份会被
    // 写进新工作区的文件里（读盘是异步的，中间那段时间闸还开着）。
    loaded.current = false;
    invoke<StudioResult[]>("studio_results", { workspacePath: workspacePath ?? null })
      .then((saved) => {
        if (!alive) return;
        setResults(mergeHistory(useStudioStore.getState().results, saved));
        // **只有读成功了才允许写。** 放在 finally 里的话，读失败也会开闸，
        // 下一次防抖就用内存里那份（多半是空的）把磁盘上的历史盖掉 ——
        // 读不出来已经够糟了，再把它删掉就无法挽回。
        loaded.current = true;
      })
      .catch((e) => console.warn("[studio] 读不回上次的结果，这次不会写盘：", e));
    return () => {
      alive = false;
    };
  }, [setResults, workspacePath]);

  useEffect(() => {
    if (!loaded.current) return;
    const t = setTimeout(() => {
      invoke("studio_save_results", {
        workspacePath: workspacePath ?? null,
        items: results,
      }).catch((e) =>
        console.warn("[studio] 存不下结果：", e),
      );
    }, 600);
    return () => clearTimeout(t);
  }, [results, workspacePath]);
}
