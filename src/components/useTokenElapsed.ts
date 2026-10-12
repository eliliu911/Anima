import { useEffect, useRef, useState } from "react";

/**
 * 记住「模型最后一次吐出 token」时的秒数，给速率当分母。
 *
 * **修的是这个：跟 Anima 说完话之后，下面的 token 速率还在一直往下掉。**
 *
 * LLM 说完并不等于这一轮结束 —— 工具还在跑，`streaming` 仍是 true，
 * `liveElapsed` 继续涨而 token 数已经冻住。用整轮时长当分母的话，
 * tok/s 会一路下滑，看起来像模型还在慢吞吞地生成。
 *
 * **抽成 hook 是因为上一次修漏了一处。** 当时只改了
 * [`AgentActivityTrajectory`]，而屏幕最下面那行 `Working... 38s · ~469 tok · 12.3/s`
 * 是 [`StreamStatusBar`] 渲染的 —— 用户看到的正是没改到的那一个。
 * 各写一份必然会再漂一次。
 */
export function useTokenElapsed(
  completionTokens: number | undefined,
  streamedChars: number | undefined,
  liveElapsed: number,
): number | undefined {
  const [tokenElapsed, setTokenElapsed] = useState<number | undefined>(undefined);
  const lastSigRef = useRef<number>(-1);
  useEffect(() => {
    // 真实 token 数优先；流式中间只有字符数，两者都可能单独增长
    const sig = (completionTokens ?? 0) * 100_000 + (streamedChars ?? 0);
    if (sig > lastSigRef.current) {
      lastSigRef.current = sig;
      setTokenElapsed(Math.max(1, liveElapsed));
    }
  }, [completionTokens, streamedChars, liveElapsed]);
  return tokenElapsed;
}
