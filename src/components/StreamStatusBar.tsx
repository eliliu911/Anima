import { useEffect, useState } from "react";

import { useChat } from "../store/chat";
import { useTokenElapsed } from "./useTokenElapsed";
import { computeProgress, formatTokens } from "./progressMetrics";

/**
 * 生成中的状态行：秒数、token、速度。
 *
 * **钉在消息列表和输入框之间，不随内容滚动。**
 *
 * 之前它渲染在轨迹块内部，于是正文一开始流就把它顶上去、再随滚动跑掉 ——
 * 而这一行恰恰是等待期间唯一能证明"还在推进"的东西，看不见就等于没有。
 * 转圈本身证明不了什么：它在主线程卡死时照样转（那次流式卡死就是这样）。
 *
 * 只在有流在跑时出现；跑完就消失，最终数字由顶部那条折叠摘要负责。
 */
export function StreamStatusBar() {
  const turns = useChat((s) => s.turns);
  const activeStreamId = useChat((s) => s.activeStreamId);

  // 正在流的那一轮。从后往前找 —— 并发时最新那条才是用户在看的。
  const turn = activeStreamId
    ? [...turns].reverse().find((t) => t.streaming)
    : undefined;
  const startedAt = turn?.startedAt;

  const [liveElapsed, setLiveElapsed] = useState(0);

  useEffect(() => {
    if (!startedAt) {
      setLiveElapsed(0);
      return;
    }
    const tick = () =>
      setLiveElapsed(Math.max(1, Math.floor((Date.now() - startedAt) / 1000)));
    tick();
    // 250ms 而不是 500ms：等待期间这个数字是唯一能证明"还活着"的东西，
    // 跳得太慢会让人怀疑它是不是也停了。
    const interval = setInterval(tick, 250);
    return () => clearInterval(interval);
  }, [startedAt]);

  // Hook 必须在 `if (!turn) return null` **之前**调用 —— React 不允许条件调用
  const tokenElapsed = useTokenElapsed(turn?.usage?.completion, turn?.streamedChars, liveElapsed);

  if (!turn) return null;

  const { elapsedSec, tokenCount, isEstimate, tokensPerSec } = computeProgress({
    streaming: true,
    liveElapsed,
    durationSeconds: turn.durationSeconds,
    streamedChars: turn.streamedChars,
    usage: turn.usage,
    tokenElapsed,
  });

  return (
    <div className="shrink-0 flex items-center gap-2 px-4 py-1.5 text-[12px] text-[var(--celadon)] font-sans bg-[var(--surface)]">
      <span className="animate-spin text-[12px]">⟳</span>
      <span className="font-medium">Working...</span>
      <span className="tabular-nums text-[11px] opacity-80">
        {elapsedSec > 0 ? `${elapsedSec}s` : ""}
        {tokenCount > 0 && (
          <>
            {" · "}
            {isEstimate ? "~" : ""}
            {formatTokens(tokenCount)} tok
            {tokensPerSec > 0 ? ` · ${tokensPerSec.toFixed(1)}/s` : ""}
          </>
        )}
      </span>
    </div>
  );
}
