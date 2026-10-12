/**
 * 任务台账卡片。
 *
 * ## 和 AgentActivityTrajectory 的分工
 *
 * 轨迹记录**微观物理操作**（哪次 RPC 干了什么：Listed src、Read App.tsx）。
 * 台账记录**宏观里程碑**（业务做到哪一步了）。混在一起、或者把台账塞进可折叠
 * 列表里，这个功能就白做了 —— 用户和 agent 建立不了共识。
 *
 * 所以它常驻在会话区顶部，不可折叠到看不见。
 *
 * ## 为什么分「计划」和「已发生」两段
 *
 * 这是数据结构上的区分，视觉上也该是：
 *
 * - **计划** 是模型第 0 轮写下的**意图**，之后不会变
 * - **已发生** 是系统记录的**事实**（改了哪些文件、跑了什么命令、失败在哪）
 *
 * 台账里**没有 status 字段**。实测两个本地模型在一步失败之后都从不回头更新
 * 台账（0/6，逐次相同），所以进度问系统、不问模型。一个永远显示 todo 的清单
 * 比没有清单更误导。
 *
 * 这也意味着：**界面绝不能根据台账说「任务完成」**。完成与否只有测试和用户
 * 说得准。
 */
import { useEffect, useState } from "react";
import { taskLedgerArchive, taskLedgerGet, type TaskLedger } from "../api";
import { useChat } from "../store/chat";
import { useProject } from "../store/project";

export function TaskLedgerCard() {
  const sessionId = useProject((s) => s.activeSessionId);
  const workspacePath = useProject(
    (s) => s.projects.find((p) => p.id === s.activeProjectId)?.workspace_path ?? null,
  );
  // 一轮结束时重新拉：系统记录是每轮 flush 的
  const streaming = useChat((s) => !!s.activeStreamId);
  const [ledger, setLedger] = useState<TaskLedger | null>(null);
  const [expanded, setExpanded] = useState(false);

  // 键变了要**先同步清空再异步拉**。
  //
  // 只在 then 里 setLedger 的话，切换会话之后旧台账会一直挂在那里直到新请求
  // 返回 —— 而如果新会话没有台账，它会永远挂着（返回 null 时才清）。
  // 那就是「跨会话的幽灵台账」。
  const key = `${sessionId ?? ""}|${workspacePath ?? ""}`;
  useEffect(() => {
    setLedger(null);
    setExpanded(false);
  }, [key]);

  useEffect(() => {
    if (!sessionId && !workspacePath) return;
    let cancelled = false;
    taskLedgerGet(sessionId, workspacePath)
      .then((l) => {
        // 挡住旧键的迟到响应 —— 否则快速连切两个会话时，先发的请求
        // 后到，会把新会话的台账覆盖成旧的
        if (!cancelled) setLedger(l);
      })
      .catch(() => {
        if (!cancelled) setLedger(null);
      });
    return () => {
      cancelled = true;
    };
    // streaming 从 true 变 false = 一轮刚结束，重新拉一次系统记录
  }, [key, sessionId, workspacePath, streaming]);

  if (!ledger || ledger.archived) return null;

  const modified = Object.entries(ledger.touched)
    .filter(([, k]) => k === "modified")
    .map(([p]) => p);
  const read = Object.entries(ledger.touched)
    .filter(([, k]) => k === "read")
    .map(([p]) => p);
  const failedRuns = ledger.runs.filter((r) => !r.ok);

  async function archive() {
    setLedger(null);
    try {
      await taskLedgerArchive(sessionId, workspacePath);
    } catch {
      // 归档失败就下次再说 —— 不值得打断
    }
  }

  return (
    <div className="shrink-0 mx-4 mt-1 mb-2 rounded-2xl border border-[var(--line)] bg-[var(--paper)] shadow-2xs overflow-hidden">
      <div className="px-3.5 py-2 flex items-start gap-2">
        <span className="text-[11px] mt-0.5 px-1.5 py-0.5 rounded-full bg-[var(--celadon-2)] text-[var(--celadon)] font-medium shrink-0">
          任务
        </span>
        <div className="min-w-0 flex-1">
          <div className="text-[12.5px] font-semibold text-[var(--ink)] leading-snug break-words">
            {ledger.goal || "（未写目标）"}
          </div>
          <div className="text-[11px] text-[var(--ink-3)] mt-0.5 tabular-nums">
            {ledger.steps.length} 步 · {ledger.rounds} 轮
            {modified.length > 0 && ` · 改过 ${modified.length} 个文件`}
            {failedRuns.length > 0 && (
              <span className="text-red-500"> · {failedRuns.length} 次命令失败</span>
            )}
            {/* 反复重新规划本身是个信号，要看得见 */}
            {ledger.plan_revision > 0 && ` · 计划改过 ${ledger.plan_revision} 次`}
          </div>
        </div>
        <button
          type="button"
          onClick={() => setExpanded((e) => !e)}
          className="text-[11px] text-[var(--ink-3)] hover:text-[var(--ink)] px-1.5 py-0.5 rounded-lg hover:bg-[var(--surface-2)] cursor-pointer shrink-0"
        >
          {expanded ? "收起" : "展开"}
        </button>
        <button
          type="button"
          onClick={() => void archive()}
          title="归档这个任务（停止注入，记录留着）"
          className="text-[11px] text-[var(--ink-3)] hover:text-red-500 px-1.5 py-0.5 rounded-lg hover:bg-[var(--surface-2)] cursor-pointer shrink-0"
        >
          ✕
        </button>
      </div>

      {expanded && (
        <div className="px-3.5 pb-3 pt-1 border-t border-[var(--line)]/60 flex flex-col gap-2.5">
          {/* 目标改写要显眼 —— 目标被悄悄降级是模型多次失败后最常见的自我妥协 */}
          {ledger.goal_revisions.length > 0 && (
            <div className="text-[11px] text-amber-700 bg-amber-50 border border-amber-200/70 rounded-lg px-2 py-1.5">
              目标改写过 {ledger.goal_revisions.length} 次：
              {ledger.goal_revisions.join("；")}
            </div>
          )}

          {ledger.steps.length > 0 && (
            <div>
              <div className="text-[10.5px] text-[var(--ink-3)] mb-1">
                计划（模型写下的意图）
              </div>
              <ol className="text-[12px] text-[var(--ink)] leading-relaxed list-decimal ml-4">
                {ledger.steps.map((s, i) => (
                  <li key={i}>{s}</li>
                ))}
              </ol>
            </div>
          )}

          <div>
            <div className="text-[10.5px] text-[var(--ink-3)] mb-1">
              已发生（系统记录，不是模型自述）
            </div>
            <div className="text-[12px] text-[var(--ink-2)] leading-relaxed flex flex-col gap-0.5">
              {modified.length > 0 && (
                <div>
                  <span className="text-[var(--ink-3)]">改过：</span>
                  <span className="font-mono text-[11.5px]">{modified.join("、")}</span>
                </div>
              )}
              {read.length > 0 && (
                <div>
                  <span className="text-[var(--ink-3)]">读过：</span>
                  <span className="font-mono text-[11.5px]">{read.join("、")}</span>
                </div>
              )}
              {ledger.runs.length > 0 && (
                <div className="flex flex-wrap gap-1 mt-0.5">
                  {ledger.runs.map((r, i) => (
                    <span
                      key={i}
                      className={`text-[11px] font-mono px-1.5 py-0.5 rounded-lg border ${
                        r.ok
                          ? "text-[var(--ink-2)] bg-[var(--surface)] border-[var(--line)]"
                          : "text-red-600 bg-red-50 border-red-200/70"
                      }`}
                      title={`第 ${r.round} 轮`}
                    >
                      {r.cmd} {r.ok ? "✓" : "✗"}
                    </span>
                  ))}
                </div>
              )}
              {modified.length === 0 && read.length === 0 && ledger.runs.length === 0 && (
                <span className="text-[var(--ink-3)]">还没有动作</span>
              )}
            </div>
          </div>

          {ledger.last_failure && (
            <div>
              <div className="text-[10.5px] text-[var(--ink-3)] mb-1">
                最近一次失败（{ledger.last_failure.tool} {ledger.last_failure.target}，第{" "}
                {ledger.last_failure.round} 轮）
              </div>
              <pre className="text-[11px] font-mono text-[var(--ink-2)] bg-[var(--surface)] border border-[var(--line)] rounded-lg p-2 overflow-x-auto whitespace-pre max-h-40">
                {ledger.last_failure.excerpt}
              </pre>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
