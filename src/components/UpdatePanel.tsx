/**
 * 设置 → 常规里的「版本与更新」。
 *
 * # 为什么构建和换包是两个按钮
 *
 * 构建要十几分钟，**换包会把应用关掉**。合成一个按钮的话，你点完走开，
 * 回来发现应用自己退了 —— 而且不知道它是更新完退的还是崩了。
 * 分开之后：构建随便点（不影响正在用的应用），换包那一下你自己挑时间。
 *
 * # 为什么不画进度条，但要能看出它活着
 *
 * cargo 不告诉你总共要编多少个 crate，凑出来的百分比会在 87% 停十分钟。
 *
 * 但只报「阶段 + 已编译个数」不够：**增量构建只有 `anima` 一个 crate 要编**，
 * 于是界面上就是一行「已编译 1 个」然后**静止六分钟** —— 那六分钟全在链接，
 * cargo 一个字都不输出，看起来和卡死一模一样（2026-09-18 实测）。
 *
 * 所以三样一起给：**步骤条**（离散、真实，链接单独一格）、
 * **已用时间**（链接期唯一在动的东西）、**日志窗**（滚真实输出）。
 */
import { useEffect, useRef, useState } from "react";
import {
  onUpdateProgress,
  updaterApply,
  updaterBuild,
  updaterCheck,
  type UpdateCheck,
  type UpdateProgress,
} from "../api/updater";
import { BuildInfoFooter } from "./BuildInfoFooter";
import {
  advancePhase,
  appendLog,
  buildButtonLabel,
  canApply,
  canBuild,
  checkSummary,
  elapsedText,
  PHASES,
  phaseLabel,
  staleBuildNote,
  stepState,
  type Stage,
} from "./updateState";
import type { UpdatePhase } from "../api/updater";

export function UpdatePanel() {
  const [stage, setStage] = useState<Stage>("idle");
  const [check, setCheck] = useState<UpdateCheck | null>(null);
  const [progress, setProgress] = useState<UpdateProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  /** 这次构建顺手清掉了多少构建垃圾 */
  const [swept, setSwept] = useState("");
  /** 换包会退出应用 —— 必须先确认一次 */
  const [confirming, setConfirming] = useState(false);
  /** 日志窗的内容。**心跳不进来** */
  const [log, setLog] = useState<string[]>([]);
  /** 走到哪一步了。`tick` / `failed` 不推进它 */
  const [phase, setPhase] = useState<UpdatePhase | null>(null);
  const unlisten = useRef<(() => void) | null>(null);
  const logEnd = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    void onUpdateProgress((p) => {
      setProgress(p);
      setLog((l) => appendLog(l, p));
      setPhase((c) => advancePhase(c, p));
      if (p.phase === "failed") {
        setStage("failed");
        setError(p.label);
      } else if (p.phase === "done") {
        setStage("ready");
        // 构建完顺手清了多少。**要显示出来** —— 只写进日志窗的话，
        // 用户不会知道这一步存在（他是自己发现攒了 190 GB 才来问的）
        if (p.line) setSwept(p.line);
      }
    }).then((fn) => {
      unlisten.current = fn;
    });
    return () => unlisten.current?.();
  }, []);

  const doCheck = async () => {
    setStage("checking");
    setError(null);
    try {
      setCheck(await updaterCheck());
      setStage("checked");
    } catch (e) {
      setError(String(e));
      setStage("failed");
    }
  };

  // 新输出进来就滚到底。**只在构建中滚** —— 构建完了人要往回翻看错误
  useEffect(() => {
    if (stage === "building") logEnd.current?.scrollIntoView({ block: "end" });
  }, [log, stage]);

  const doBuild = async () => {
    setStage("building");
    setError(null);
    setProgress(null);
    setLog([]);
    setPhase(null);
    try {
      await updaterBuild();
    } catch (e) {
      setError(String(e));
      setStage("failed");
    }
  };

  const doApply = async () => {
    try {
      await updaterApply();
    } catch (e) {
      setError(String(e));
      setConfirming(false);
    }
  };

  const btn =
    "rounded-xl px-3.5 py-1.5 text-[12px] font-medium border border-[var(--line)] transition-colors disabled:opacity-40 disabled:cursor-not-allowed";

  return (
    <div className="flex flex-col gap-3 border-t border-[var(--line)] pt-4">
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-[12.5px] font-bold text-[var(--ink-2)]">版本与更新</span>
        <button
          type="button"
          className={`${btn} bg-[var(--surface-2)] hover:bg-[var(--surface-3)] text-[var(--ink)]`}
          disabled={stage === "checking" || stage === "building"}
          onClick={() => void doCheck()}
        >
          {stage === "checking" ? "查询中…" : "检查更新"}
        </button>
        {check && (
          <button
            type="button"
            className={`${btn} bg-[var(--surface-2)] hover:bg-[var(--surface-3)] text-[var(--ink)]`}
            disabled={!canBuild(stage, check)}
            onClick={() => void doBuild()}
            title={check.blocked ?? (canBuild(stage, check) ? "拉代码并重新构建，十几分钟。不影响正在用的应用" : "编出来还是同一个版本，不用构建")}
          >
            {buildButtonLabel(stage, check)}
          </button>
        )}
        {check && stage === "checked" && !check.blocked && !canBuild(stage, check) && (
          // **逃生口**：包坏了、想换个环境重编时还能编。做成小字，不和主按钮抢
          <button
            type="button"
            className="text-[11px] text-[var(--ink-3)] underline decoration-dotted underline-offset-2 hover:text-[var(--ink-2)]"
            onClick={() => void doBuild()}
            title="编出来还是同一个版本，十几分钟。只在包坏了、想重编时用"
          >
            仍要重新构建
          </button>
        )}
        {canApply(stage, check) && !confirming && (
          <button
            type="button"
            className={`${btn} bg-[var(--celadon)] text-white border-transparent`}
            onClick={() => setConfirming(true)}
          >
            换包并重启
          </button>
        )}
      </div>

      {/* **换包会退出应用** —— 不先说清楚的话，点完应用自己关了会以为是崩了 */}
      {confirming && (
        <div className="rounded-xl border border-[var(--line)] bg-[var(--surface-2)] px-3 py-2.5 text-[12px]">
          <p className="text-[var(--ink)]">
            换包会<b>立刻退出 Anima</b>，换完自动重新打开。正在跑的出图 / 出片会被中断。
          </p>
          <div className="mt-2 flex gap-2">
            <button type="button" className={`${btn} bg-[var(--celadon)] text-white border-transparent`} onClick={() => void doApply()}>
              退出并换包
            </button>
            <button type="button" className={`${btn} bg-[var(--surface-3)] text-[var(--ink)]`} onClick={() => setConfirming(false)}>
              等一下
            </button>
          </div>
        </div>
      )}

      {/* **「跑的构建旧了」排在最前面。**
          用户最常踩的是「拉过代码但没重新构建」，而不是「没拉代码」——
          只比 HEAD↔origin 的话这条永远不出现，而它才是当下真正要做的事 */}
      {swept && stage === "ready" && (
        <p className="text-[11.5px] text-[var(--ink-3)]">{swept}</p>
      )}

      {check && stage !== "building" && staleBuildNote(check) && (
        <p className="rounded-xl border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-[11.5px] leading-snug text-amber-600">
          {staleBuildNote(check)}
        </p>
      )}

      {check && stage !== "building" && (
        <div className="text-[11.5px] leading-relaxed text-[var(--ink-3)]">
          <p className={check.blocked ? "text-[var(--danger,#e11d48)]" : ""}>{checkSummary(check)}</p>
          {check.built_commits.length > 0 && (
            <ul className="mt-1 max-h-[120px] overflow-auto font-mono text-[10.5px]">
              {check.built_commits.map((c) => (
                <li key={c} className="truncate">
                  {c}
                </li>
              ))}
            </ul>
          )}
          {check.commits.length > 0 && (
            <ul className="mt-1 max-h-[140px] overflow-auto font-mono text-[10.5px]">
              {check.commits.map((c) => (
                <li key={c} className="truncate">
                  {c}
                </li>
              ))}
            </ul>
          )}
          {check.repo && <p className="mt-1 opacity-60">源码：{check.repo}</p>}
        </div>
      )}

      {/* 构建进度。**不是百分比** —— 见文件头 */}
      {(stage === "building" || stage === "ready" || stage === "failed") && progress && (
        <div className="rounded-xl bg-[var(--surface-2)] px-3 py-2.5">
          {/* 步骤条：离散且真实。链接单独一格 —— 它最长也最安静 */}
          <div className="flex flex-wrap items-center gap-1.5 text-[11px]">
            {PHASES.map((step, i) => {
              const st = stepState(step.key, stage === "ready" ? "done" : phase);
              return (
                <span key={step.key} className="flex items-center gap-1.5">
                  {i > 0 && <span className="opacity-30">›</span>}
                  <span
                    className={
                      st === "done"
                        ? "text-emerald-600"
                        : st === "now"
                          ? "font-semibold text-[var(--ink)]"
                          : "text-[var(--ink-3)] opacity-50"
                    }
                  >
                    {st === "done" ? "✓ " : ""}
                    {step.label}
                  </span>
                </span>
              );
            })}
            {/* **链接期只有这个数字在动** —— 没有它界面看起来就是死的 */}
            <span className="ml-auto font-mono text-[var(--ink-3)]">{elapsedText(progress.elapsed)}</span>
          </div>

          {phaseLabel(progress) && (
            <p className="mt-1.5 text-[11.5px] text-[var(--ink-2)]">{phaseLabel(progress)}</p>
          )}

          {/* 日志窗：真实输出，像终端那样滚 */}
          {log.length > 0 && (
            <div className="mt-2 max-h-[180px] overflow-auto rounded-lg bg-[var(--surface)] px-2.5 py-2">
              {log.map((l, i) => (
                <div
                  key={`${i}-${l.slice(0, 24)}`}
                  className={`whitespace-pre-wrap break-all font-mono text-[10.5px] leading-[1.5] ${
                    /^error|error:/.test(l)
                      ? "text-[var(--danger,#e11d48)]"
                      : l.startsWith("warning")
                        ? "text-amber-600/80"
                        : "text-[var(--ink-3)]"
                  }`}
                >
                  {l}
                </div>
              ))}
              <div ref={logEnd} />
            </div>
          )}

          {stage === "building" && (
            <p className="mt-1.5 text-[11px] text-[var(--ink-3)] opacity-70">
              十几分钟，可以切走做别的。<b>链接那一段完全没有输出</b>，只有上面的秒数在走 —— 那是正常的。
            </p>
          )}
        </div>
      )}

      {error && <p className="text-[11.5px] text-[var(--danger,#e11d48)]">{error}</p>}

      <BuildInfoFooter />
    </div>
  );
}
