import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { ToolCall } from "../api";
import { DiffViewer, createReplaceDiff, parseRawDiff, type DiffData } from "./DiffViewer";
import { useCodeViewer } from "../store/codeViewer";
import { FileIcon, FolderIcon, SearchIcon, TerminalIcon, EditIcon } from "./Icons";
import { buildActivitySummary, computeProgress } from "./progressMetrics";
import { useTokenElapsed } from "./useTokenElapsed";

interface TrajectoryProps {
  reasoning?: string;
  toolCalls?: ToolCall[];
  streaming?: boolean;
  startedAt?: number;
  durationSeconds?: number;
  /** 流式期间已收到的字符数（正文 + 思考），用来在等待时给出活着的反馈 */
  streamedChars?: number;
  /** 最终精确用量，只在 Done 时到；到了就替掉估算值 */
  usage?: { prompt: number; completion: number; reasoning?: number };
}

export function AgentActivityTrajectory({
  reasoning,
  toolCalls = [],
  streaming = false,
  startedAt,
  durationSeconds,
  streamedChars,
  usage,
}: TrajectoryProps) {
  const { t } = useTranslation(["chat", "trajectory", "common"]);
  const [expanded, setExpanded] = useState(true);
  const [reasoningExpanded, setReasoningExpanded] = useState(false);
  const [expandedDiffs, setExpandedDiffs] = useState<Record<number, boolean>>({});

  // 实时累计运行/思考秒数计时器
  const [liveElapsed, setLiveElapsed] = useState<number>(() => {
    if (!startedAt) return 0;
    return Math.max(1, Math.floor((Date.now() - startedAt) / 1000));
  });

  // 模型最后一次吐 token 时的秒数。速率用它当分母。
  //
  // LLM 说完之后工具还在跑，`liveElapsed` 继续涨而 token 数冻住 ——
  // 不记这个的话速率会一直往下掉，看起来像模型还在慢慢生成。
  const tokenElapsed = useTokenElapsed(usage?.completion, streamedChars, liveElapsed);

  useEffect(() => {
    if (!streaming || !startedAt) return;
    // 250ms 而不是 500ms：等待期间这个数字是唯一能证明"还活着"的东西，
    // 跳得太慢会让人怀疑它是不是也停了。
    const interval = setInterval(() => {
      setLiveElapsed(Math.max(1, Math.floor((Date.now() - startedAt) / 1000)));
    }, 250);
    return () => clearInterval(interval);
  }, [streaming, startedAt]);

  if (!reasoning && (!toolCalls || toolCalls.length === 0) && !streaming) {
    return null;
  }

  // 计算摘要指标与步骤
  let fileCount = 0;
  let searchCount = 0;
  let cmdCount = 0;

  const parsedSteps = toolCalls.map((call) => {
    let args: any = {};
    try {
      if (typeof call.arguments === "string") {
        args = JSON.parse(call.arguments);
      } else {
        args = call.arguments || {};
      }
    } catch {
      args = {};
    }

    const name = (call.name || "").toLowerCase();

    // 1. 文件与目录读取类工具 (fs_read_text, fs_list_directory, view_file, read_file)
    if (
      name.includes("read") ||
      name.includes("view_file") ||
      name.includes("fs_read") ||
      name.includes("fs_list") ||
      name.includes("read_url")
    ) {
      fileCount++;
      const fullPath = args.path || args.AbsolutePath || args.file || args.Url || "";
      const baseName = fullPath.split("/").filter(Boolean).pop() || fullPath;
      const ext = baseName.split(".").pop() || "";
      const sLine = args.StartLine || args.start_line;
      const eLine = args.EndLine || args.end_line;
      const lineRange =
        sLine && eLine
          ? `#L${sLine}-${eLine}`
          : sLine
          ? `#L${sLine}`
          : "";

      const isFolder = name.includes("list") || fullPath.endsWith("/");
      const lineRangeNumbers: [number, number] | null =
        sLine && eLine
          ? [Number(sLine), Number(eLine)]
          : sLine
          ? [Number(sLine), Number(sLine)]
          : null;

      return {
        type: "file",
        verb: isFolder ? "Listed" : "Read",
        icon: isFolder ? <FolderIcon className="w-3.5 h-3.5" /> : <FileIcon ext={ext} className="w-3.5 h-3.5" />,
        target: `${baseName || fullPath || "file"} ${lineRange}`.trim(),
        detail: fullPath,
        lineRangeNumbers,
        isFolder,
      };
    }

    // 1.5 代码大纲工具 (code_outline)
    if (name.includes("code_outline") || name.includes("outline")) {
      fileCount++;
      const fullPath = args.path || args.AbsolutePath || args.file || "";
      const baseName = fullPath.split("/").filter(Boolean).pop() || fullPath;
      const ext = baseName.split(".").pop() || "";
      return {
        type: "file",
        verb: "Outlined",
        icon: <FileIcon ext={ext} className="w-3.5 h-3.5" />,
        target: baseName || fullPath || "file",
        detail: fullPath,
      };
    }

    // 2. 搜索类工具 (workspace_search, search, grep, find)
    if (name.includes("search") || name.includes("grep") || name.includes("find")) {
      searchCount++;
      const q = args.query || args.Query || args.Pattern || args.pattern || "";
      return {
        type: "search",
        verb: "Searched",
        icon: <SearchIcon className="w-3.5 h-3.5" />,
        target: q ? `"${q}"` : call.name,
        badge: "results",
      };
    }

    // 3. 命令行执行工具 (exec_command, write_stdin, run_command, bash)
    if (name.includes("exec") || name.includes("command") || name.includes("bash") || name.includes("stdin")) {
      cmdCount++;
      const cmd = args.cmd || args.CommandLine || args.command || (args.chars ? `stdin: ${args.chars.trim()}` : "");
      return {
        type: "cmd",
        verb: "Ran",
        icon: <TerminalIcon className="w-3.5 h-3.5" />,
        target: cmd || call.name,
      };
    }

    // 4. 文件写入与 Patch 工具 (apply_patch, fs_write_text, replace, edit)
    if (name.includes("patch") || name.includes("write") || name.includes("replace") || name.includes("edit")) {
      fileCount++;
      let targetFile = args.path || args.TargetFile || args.file || args.filePath || "";
      if (!targetFile && typeof args.patch === "string") {
        const match = args.patch.match(/\*\*\*\s*(?:Update|Add|Delete)\s*File:\s*([^\n\r]+)/i);
        if (match) targetFile = match[1].trim();
      }
      const baseName = targetFile.split("/").filter(Boolean).pop() || targetFile;
      const ext = baseName.split(".").pop()?.toLowerCase() || "";

      let diff: DiffData | undefined;
      if (args.TargetContent !== undefined && args.ReplacementContent !== undefined) {
        diff = createReplaceDiff(targetFile || baseName, args.StartLine || 1, String(args.TargetContent), String(args.ReplacementContent));
      } else if (typeof args.patch === "string") {
        diff = parseRawDiff(args.patch, baseName);
      } else if (args.CodeContent !== undefined) {
        diff = createReplaceDiff(targetFile || baseName, 1, "", String(args.CodeContent));
      } else if (args.content !== undefined && typeof args.content === "string") {
        diff = createReplaceDiff(targetFile || baseName, 1, "", String(args.content));
      }

      return {
        type: "edit",
        verb: "Edited",
        icon: <FileIcon ext={ext} className="w-3.5 h-3.5" />,
        target: baseName || call.name,
        detail: targetFile,
        diff,
        additions: diff?.additions || 0,
        deletions: diff?.deletions || 0,
      };
    }

    // 4.5 子智能体派发工具 (invoke_subagent)
    if (name.includes("subagent")) {
      const role = args.role || "Subagent";
      const mode = (args.mode || "explore").toUpperCase();
      const goal = args.goal || "";
      return {
        type: "subagent",
        verb: "Dispatched",
        icon: (
          <span className="w-3.5 h-3.5 flex items-center justify-center text-[10px] bg-purple-900/60 text-purple-300 border border-purple-600/50 rounded shrink-0 font-mono font-bold">
            ⚡
          </span>
        ),
        target: `${role} [${mode}]`,
        detail: goal ? `Goal: ${goal}` : undefined,
        badge: mode,
      };
    }

    // 4.6 视频生成与动态渲染工具 (generate_video, animate_image)
    if (name.includes("animate_image") || name.includes("generate_video") || name.includes("video")) {
      const prompt = args.motion_prompt || args.prompt || "Video";
      const duration = args.duration_secs || args.duration || 2;
      const isI2V = name.includes("animate_image") || !!args.image_path;
      return {
        type: "video",
        verb: isI2V ? "Animated" : "Rendered",
        icon: (
          <span className="w-3.5 h-3.5 flex items-center justify-center text-[10.5px] bg-indigo-900/60 text-indigo-300 border border-indigo-600/50 rounded shrink-0 font-mono font-bold">
            🎬
          </span>
        ),
        target: `${duration}s ${isI2V ? "Image-to-Video" : "Video"}`,
        detail: prompt,
        badge: `${duration}s`,
      };
    }

    // 4.7 图像与视觉生成工具 (generate_image)
    if (name.includes("generate_image") || name === "image") {
      const prompt = args.prompt || "Image";
      const width = args.width || 1024;
      const height = args.height || 1024;
      const isI2I = !!args.image_path;
      return {
        type: "image",
        verb: isI2I ? "Refined" : "Generated",
        icon: (
          <span className="w-3.5 h-3.5 flex items-center justify-center text-[10.5px] bg-sky-900/60 text-sky-300 border border-sky-600/50 rounded shrink-0 font-mono font-bold">
            🎨
          </span>
        ),
        target: `${width}x${height} ${isI2I ? "Refinement" : "Image"}`,
        detail: prompt,
        badge: `${width}x${height}`,
      };
    }

    // 5. 其他工具
    return {
      type: "other",
      verb: "Called",
      icon: <EditIcon className="w-3.5 h-3.5" />,
      target: call.name,
      detail: JSON.stringify(args),
    };
  });

  // 秒数 / token / 速率的算法在 progressMetrics.ts，那里有单测钉住两条
  // 容易再犯的规则：流式期间不能停表、精确值一到要盖掉估算值。
  const {
    elapsedSec,
    tokenCount,
    isEstimate: tokenIsEstimate,
    tokensPerSec,
    reasoningTokens,
  } = computeProgress({
    streaming,
    liveElapsed,
    durationSeconds,
    streamedChars,
    usage,
    tokenElapsed,
  });

    // 顶部摘要。**跑的时候不带秒数和 token** —— 那两个数在底部那条 Working 行
    // 里一直在动，顶部再显示一遍就是三份一模一样的数字白占竖向空间。
    // 规则和它的测试都在 progressMetrics.ts 的 buildActivitySummary。
    const summaryText = buildActivitySummary({
      streaming,
      reasoning: !!reasoning,
      elapsedSec,
      cmdCount,
      searchCount,
      fileCount,
      tokenCount,
      tokenIsEstimate,
      tokensPerSec,
      reasoningTokens,
    });

  return (
    <div className="flex flex-col gap-1.5 my-1.5 font-sans">
      {/* 轨迹总览胶囊行 */}
      <button
        type="button"
        onClick={() => setExpanded(!expanded)}
        className="flex items-center gap-2 text-[12.5px] text-[var(--ink-2)] hover:text-[var(--ink)] cursor-pointer py-1 px-2.5 rounded-xl bg-[var(--surface-2)] hover:bg-[var(--surface-3)] border border-[var(--line)] w-fit transition-colors select-none shadow-2xs group"
      >
        <span className={`w-2 h-2 rounded-full bg-[var(--celadon)] shrink-0 ${streaming ? "animate-pulse" : ""}`} />
        <span className="font-medium text-[var(--ink)]">{summaryText}</span>
        <svg
          width="12"
          height="12"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2.5"
          className={`opacity-60 group-hover:opacity-100 transition-transform duration-200 ml-1 ${
            expanded ? "rotate-180" : ""
          }`}
        >
          <path d="M6 9l6 6 6-6" />
        </svg>
      </button>

      {/* 展开的步骤列表 */}
      {expanded && (
        <div className="flex flex-col gap-1.5 pl-2 mt-1 border-l-2 border-[var(--line)] py-0.5 text-[12px] font-mono">
          {/* Reasoning 思考项 */}
          {reasoning && (
            <div className="flex flex-col gap-1">
              <button
                type="button"
                onClick={() => setReasoningExpanded(!reasoningExpanded)}
                className="flex items-center gap-1.5 text-[12px] text-[var(--ink-2)] hover:text-[var(--ink)] cursor-pointer w-fit font-sans"
              >
                {/* 只做段落标签，不再重复秒数 —— 时间在顶部摘要（完成后）
                    或底部 Working 行（跑的时候），不需要第三份 */}
                <span className="text-[var(--ink-3)]">{t("reasoning", { defaultValue: "思考过程" })}</span>
                <svg
                  width="11"
                  height="11"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2.5"
                  className={`opacity-60 transition-transform duration-150 ${
                    reasoningExpanded ? "rotate-90" : ""
                  }`}
                >
                  <path d="M9 18l6-6-6-6" />
                </svg>
              </button>

              {reasoningExpanded && (
                <div className="p-2.5 rounded-xl bg-[var(--surface-3)]/60 text-[11.5px] text-[var(--ink-2)] leading-relaxed max-h-48 overflow-y-auto whitespace-pre-wrap font-sans my-0.5 border border-[var(--line)]/50">
                  {reasoning}
                </div>
              )}
            </div>
          )}

          {/* 工具调用步骤流 */}
          {parsedSteps.map((step, idx) => {
            const isDiff = step.type === "edit" && !!(step as any).diff;
            const isDiffExpanded = !!expandedDiffs[idx];

            return (
              <div key={idx} className="flex flex-col gap-1 py-0.5">
                <div
                  onClick={() => {
                    if (isDiff) {
                      setExpandedDiffs((prev) => ({ ...prev, [idx]: !prev[idx] }));
                    } else if (step.type === "file" && step.detail && !(step as any).isFolder) {
                      void useCodeViewer.getState().openFile(step.detail, {
                        lineRange: (step as any).lineRangeNumbers ?? null,
                      });
                    }
                  }}
                  className={`flex items-center gap-2 text-[12px] text-[var(--ink-2)] hover:text-[var(--ink)] transition-colors py-0.5 ${
                    isDiff || (step.type === "file" && step.detail && !(step as any).isFolder)
                      ? "cursor-pointer group select-none hover:text-[var(--celadon)]"
                      : ""
                  }`}
                >
                  <span className="text-[var(--ink-3)] shrink-0 font-sans">{step.verb}</span>
                  <span className="shrink-0">{step.icon}</span>
                  <span className="font-bold text-[var(--ink)] truncate max-w-[320px] font-mono">
                    {step.target}
                  </span>

                  {/* 增删行数统计胶囊 (+0 -2) */}
                  {step.type === "edit" && (
                    <div className="flex items-center gap-1 font-mono text-[11px] font-semibold shrink-0 ml-1">
                      <span className="text-emerald-500 font-bold">+{(step as any).additions || 0}</span>
                      <span className="text-rose-500 font-bold">-{(step as any).deletions || 0}</span>
                    </div>
                  )}

                  {step.badge && (
                    <span className="text-[10px] px-1.5 py-0.2 rounded-md bg-[var(--surface-3)] text-[var(--ink-3)] shrink-0 font-sans">
                      {step.badge}
                    </span>
                  )}

                  {isDiff && (
                    <svg
                      width="10"
                      height="10"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="2.5"
                      className={`opacity-50 group-hover:opacity-100 transition-transform duration-150 ml-0.5 ${
                        isDiffExpanded ? "rotate-90" : ""
                      }`}
                    >
                      <path d="M9 18l6-6-6-6" />
                    </svg>
                  )}
                </div>

                {/* 展开的代码差异查看器 */}
                {isDiff && isDiffExpanded && (step as any).diff && (
                  <div className="pl-1 pr-2 w-full animate-in fade-in zoom-in-95 duration-150">
                    <DiffViewer diff={(step as any).diff} initialExpanded={true} />
                  </div>
                )}
              </div>
            );
          })}

          {/* 「Working… 秒数 · token · 速度」那一行已经挪到 `StreamStatusBar`，
              钉在消息列表和输入框之间。

              留在这里的话，正文一开始流就会把它顶上去、再随滚动跑掉 ——
              而它恰恰是等待期间唯一能证明"还在推进"的东西：转圈本身证明不了什么，
              主线程卡死的时候它照样转（流式卡死那次就是这样）。 */}
        </div>
      )}
    </div>
  );
}

