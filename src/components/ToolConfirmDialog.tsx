/**
 * 工具调用的确认框。
 *
 * 这是权限链条的最后一环：Rust 侧闸门判定「需要确认」后会**一直等**这个框的回应，
 * 所以它必须始终挂载（而不是只在某个面板打开时才存在）。
 *
 * 设计上刻意做得不容易误点：
 * - 默认焦点在「拒绝」上，回车不会变成批准
 * - critical 风险要显示醒目标识，不跟普通确认混在一起
 * - 参数原样展示 —— 用户批准的是「这次调用」，不是「这个工具」，
 *   所以必须看得到具体参数
 */
import { useEffect, useRef, useState } from "react";
import { mcpApi, onToolConfirm, type ToolConfirmEvent } from "../api/mcp";

const RISK_LABEL: Record<string, string> = {
  low: "低风险",
  medium: "中风险",
  high: "高风险",
  critical: "高危操作",
};

export function ToolConfirmDialog() {
  // 队列而不是单个：一轮里模型可能请求多个工具，逐个确认
  const [queue, setQueue] = useState<ToolConfirmEvent[]>([]);
  const [busy, setBusy] = useState(false);
  const denyRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    const unlisten = onToolConfirm((event) => {
      setQueue((q) => (q.some((i) => i.request_id === event.request_id) ? q : [...q, event]));
    });
    return () => {
      void unlisten.then((f) => f());
    };
  }, []);

  const current = queue[0];

  // 焦点落在「拒绝」上：手滑按回车不该等于批准
  useEffect(() => {
    if (current) denyRef.current?.focus();
  }, [current?.request_id]);

  if (!current) return null;

  async function respond(approved: boolean) {
    if (busy) return;
    setBusy(true);
    try {
      await mcpApi.respondConfirmation(current.request_id, approved);
    } finally {
      // 无论后端说这个请求还有效与否都出队：无效意味着那条流已经结束，
      // 框留着只会挡住后面的请求
      setQueue((q) => q.slice(1));
      setBusy(false);
    }
  }

  const critical = current.risk === "critical";

  return (
    <div className="tool-confirm__backdrop" role="dialog" aria-modal="true">
      <div
        className={`tool-confirm hud-card${critical ? " tool-confirm--critical" : ""}`}
      >
        <div className="tool-confirm__header hud-header">
          <span className="hud-title">需要你确认</span>
          <span className={`tool-confirm__risk tool-confirm__risk--${current.risk}`}>
            {RISK_LABEL[current.risk] ?? current.risk}
          </span>
        </div>

        <div className="tool-confirm__tool">{current.tool_name}</div>
        {/* 闸门给的原因原样展示 —— 用户靠它判断该不该批准 */}
        <p className="tool-confirm__reason">{current.reason}</p>

        <div className="tool-confirm__args-label">调用参数</div>
        <pre className="tool-confirm__args">{current.arguments}</pre>

        <div className="tool-confirm__actions">
          <button
            ref={denyRef}
            type="button"
            className="tool-confirm__btn tool-confirm__btn--deny"
            onClick={() => void respond(false)}
            disabled={busy}
          >
            拒绝
          </button>
          <button
            type="button"
            className="tool-confirm__btn tool-confirm__btn--allow"
            onClick={() => void respond(true)}
            disabled={busy}
          >
            允许执行
          </button>
        </div>

        {queue.length > 1 && (
          <div className="tool-confirm__queue">
            还有 {queue.length - 1} 个待确认
          </div>
        )}
      </div>
    </div>
  );
}
