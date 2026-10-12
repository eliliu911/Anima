/**
 * MCP 面板：已接入的 server、它们导出的工具及权限判定、以及工具执行的实时动态。
 *
 * 这里原先展示的是写死的假数据（固定的 CPU/内存百分比、编造的 server 名和进度条）。
 * 那比空面板更糟：它会让人以为工具真的接上了、系统真的在被监控。
 * 现在全部来自后端；没接 server 就明确说没接。
 */
import { useEffect, useState } from "react";
import { useChat } from "../store/chat";
import { useToolLog, type ToolLogLine } from "../store/toolLog";
import {
  mcpApi,
  onToolActivity,
  type McpView,
  type ToolActivity,
  type ToolInfo,
} from "../api/mcp";

/**
 * 一条执行记录。
 *
 * 关键点：**一个任务只占一行，状态原地变化**（转圈 → 对号），
 * 而不是"开始"和"完成"各占一行。原先那样做的问题是日志翻得很快、
 * 而且看不出某个工具是"还在跑"还是"跑完了" —— 恰恰是最该一眼看出来的信息。
 *
 * 活动事件里没有调用 id，只有工具名。执行循环是顺序的，同一时刻同名工具
 * 最多一个在跑，所以用"最后一条同名且未完成的记录"来配对足够可靠。
 */
interface LogLine {
  key: string;
  name: string;
  status: "running" | "ok" | "failed" | "rejected" | "declined";
  detail?: string;
}

const STATUS_ICON: Record<LogLine["status"], string> = {
  running: "",   // 用 CSS 转圈，不用字符
  ok: "✓",
  failed: "✗",
  rejected: "⛔",
  declined: "⊘",
};

const OPERATION_LABEL: Record<string, string> = {
  read: "读",
  write: "写",
  delete: "删",
  execute: "执行",
};

const ACCESS_LABEL: Record<string, string> = {
  allow: "放行",
  ask: "需确认",
  deny: "已禁用",
};

export function McpMonitor() {
  const isMcpOpen = useChat((s) => s.isMcpOpen);
  const toggleMcp = useChat((s) => s.toggleMcp);

  const [view, setView] = useState<McpView | null>(null);
  // 日志放 store：**两个来源**（文字链路的 agent 循环、语音链路的
  // VoiceEvent::Tool），而且人是先觉得不对、再来开面板的，
  // 关掉面板不能把记录弄丢
  const logs = useToolLog((s) => s.lines);
  const record = useToolLog((s) => s.record);
  const clearLogs = useToolLog((s) => s.clear);
  const [loading, setLoading] = useState(false);

  // 面板打开时拉数据，**并且要重试**。
  //
  // MCP server 是启动后在后台连接的（npx 拉包要几秒），而这个面板默认就是打开的 ——
  // 挂载时取到的必然是"还没有 server"，之后又不会重取，于是必须手动点刷新。
  // 这里在没拿到已连接的 server 时按 1.5/3/6 秒退避重试三次。
  useEffect(() => {
    if (!isMcpOpen) return;
    let cancelled = false;
    let timer = 0;
    let attempt = 0;

    const load = () => {
      setLoading(true);
      mcpApi
        .view()
        .then((v) => {
          if (cancelled) return;
          setView(v);
          // 一个都没连上就再等等 —— 可能只是还在启动
          const anyConnected = v.statuses.some((s) => s.connected);
          if (!anyConnected && attempt < 3) {
            timer = window.setTimeout(load, 1500 * Math.pow(2, attempt));
            attempt += 1;
          }
        })
        .catch((err) => {
          if (!cancelled)
            record({
              key: `err-${Date.now()}`,
              name: "读取状态",
              status: "failed",
              detail: `${err}`,
            });
        })
        .finally(() => {
          if (!cancelled) setLoading(false);
        });
    };
    load();

    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [isMcpOpen]);

  // 工具动态要一直收，不能只在面板打开时收 —— 否则打开面板会看到空白，
  // 而实际上刚刚执行过工具
  useEffect(() => {
    const unlisten = onToolActivity((event) => {
      const line = lineOf(event);
      if (line) record(line);
    });
    return () => {
      void unlisten.then((f) => f());
    };
  }, [record]);

  if (!isMcpOpen) return null;

  const statuses = view?.statuses ?? [];
  const tools = view?.tools ?? [];

  async function reconnect() {
    setLoading(true);
    try {
      const statuses = await mcpApi.reconnect();
      setView(await mcpApi.view());
      for (const s of statuses) {
        record({
          key: `conn-${s.id}-${Date.now()}`,
          name: s.label,
          status: s.connected ? "ok" : "failed",
          detail: s.connected ? `${s.tool_count} 个工具` : s.error ?? "连接失败",
        });
      }
    } catch (err) {
      record({ key: `err-${Date.now()}`, name: "重连", status: "failed", detail: `${err}` });
    } finally {
      setLoading(false);
    }
  }

  return (
    <aside className="mcp-monitor">
      <div className="mcp-monitor__header">
        <div className="mcp-monitor__title">
          <span>MCP 工具</span>
        </div>
        <div className="mcp-monitor__header-actions">
          <button
            type="button"
            className="micro-btn"
            onClick={() => void reconnect()}
            disabled={loading}
            title="重新连接全部 server"
          >
            ⟳
          </button>
          {/* 日志现在跨面板留存，所以要有清掉的办法 */}
          <button
            type="button"
            className="micro-btn"
            onClick={clearLogs}
            disabled={logs.length === 0}
            title="清空执行日志"
          >
            清空
          </button>
          <button type="button" className="micro-btn" onClick={toggleMcp} title="收起">
            ✕
          </button>
        </div>
      </div>

      <div className="mcp-monitor__section">
        <div className="mcp-monitor__section-title">服务</div>
        {statuses.length === 0 && (
          <div className="mcp-monitor__empty">
            {loading
              ? "读取中…"
              : "尚未接入任何 MCP server。在设置里添加后即可让模型使用工具。"}
          </div>
        )}
        {statuses.map((s) => (
          <div className="mcp-task-card" key={s.id}>
            <div className="mcp-task-card__title">
              <span>{s.label}</span>
              <span
                className={`mcp-task-card__status mcp-task-card__status--${
                  s.connected ? "ok" : "error"
                }`}
              >
                {s.connected ? "已连接" : "不可用"}
              </span>
            </div>
            <div className="mcp-task-card__meta">
              <span>{s.id}</span>
              <span>{s.tool_count} 个工具</span>
            </div>
            {/* 失败原因是排查的唯一线索，必须完整显示而不是截断 */}
            {s.error && <div className="mcp-task-card__error">{s.error}</div>}
          </div>
        ))}
      </div>

      {tools.length > 0 && (
        <div className="mcp-monitor__section">
          <div className="mcp-monitor__section-title">
            工具 · {tools.length}
          </div>
          <div className="mcp-tool-list">
            {tools.map((t) => (
              <ToolRow key={t.qualified_name} tool={t} />
            ))}
          </div>
        </div>
      )}

      <div className="mcp-monitor__section mcp-monitor__section--terminal-title">
        <div className="mcp-monitor__section-title">执行日志</div>
      </div>

      <div className="mcp-terminal">
        {logs.length === 0 && (
          <div className="mcp-terminal__line">（暂无工具执行记录）</div>
        )}
        {logs.map((log) => (
          <div className={`logrow logrow--${log.status}`} key={log.key}>
            <span className="logrow__icon">
              {log.status === "running" ? (
                <span className="logrow__spinner" />
              ) : (
                STATUS_ICON[log.status]
              )}
            </span>
            <span className="logrow__body">
              <span className="logrow__name">{log.name}</span>
              {log.detail && <span className="logrow__detail">{log.detail}</span>}
            </span>
          </div>
        ))}
      </div>
    </aside>
  );
}

function ToolRow({ tool }: { tool: ToolInfo }) {
  return (
    <div className="mcp-tool-row">
      <span className="mcp-tool-row__name">{tool.name}</span>
      <span className={`mcp-tool-row__op mcp-tool-row__op--${tool.operation}`}>
        {OPERATION_LABEL[tool.operation] ?? tool.operation}
      </span>
      <span className={`mcp-tool-row__access mcp-tool-row__access--${tool.access}`}>
        {ACCESS_LABEL[tool.access] ?? tool.access}
      </span>
      {/* 启发式分类会猜错，标出来让用户知道这个判定不一定准 */}
      {tool.reason === "heuristic" && (
        <span className="mcp-tool-row__guess" title="分类由关键词推断，可能不准。可在设置里纠正">
          推断
        </span>
      )}
    </div>
  );
}

/** 把一个活动事件并进现有记录：started 新建一行，finished 就地更新那一行 */
/**
 * 把一条工具动态变成日志行。
 *
 * 活动事件里**没有调用 id**，只有工具名 —— 所以「开始」和「结束」要靠名字配对。
 * 执行循环是顺序的，同一时刻同名工具最多一个在跑，用一张
 * 「名字 → 当前那一行的 key」的小表就够。
 *
 * 配对的意义是让一次调用**只占一行、状态原地变化**（转圈 → 对号）。
 * 各占一行的话，一次调用看起来像调了两次，而且看不出跑完没有。
 */
const pending = new Map<string, string>();

function lineOf(
  a: ToolActivity & { stream_id: string },
): Omit<ToolLogLine, "at"> | null {
  // 工具名形如 `server__tool`，日志里只显示后半段
  const name = a.name.split("__").slice(1).join("__") || a.name;

  if (a.phase === "started") {
    const key = `${name}-${Date.now()}-${Math.round(performance.now())}`;
    pending.set(name, key);
    return { key, name, status: "running" };
  }

  if (a.phase === "finished") {
    const key = pending.get(name) ?? `${name}-${Date.now()}`;
    pending.delete(name);
    return { key, name, status: a.ok ? "ok" : "failed", detail: a.preview };
  }

  if (a.phase === "rejected") {
    pending.delete(name);
    return { key: `${name}-${Date.now()}`, name, status: "rejected", detail: a.reason };
  }

  if (a.phase === "declined") {
    pending.delete(name);
    return { key: `${name}-${Date.now()}`, name, status: "declined" };
  }

  return null;
}
