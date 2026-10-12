import { useState } from "react";
import { HighlightedCodeLine } from "./codeHighlighter";
import { FileIcon, CopyIcon, CheckIcon } from "./Icons";

export interface DiffLine {
  type: "add" | "delete" | "normal" | "fold";
  oldLineNumber?: number;
  newLineNumber?: number;
  content: string;
  foldCount?: number;
}

export interface DiffData {
  fileName: string;
  filePath?: string;
  additions: number;
  deletions: number;
  lines: DiffLine[];
}

/**
 * 将 replace_file_content 的 TargetContent / ReplacementContent 解析为结构化 Diff
 */
export function createReplaceDiff(
  targetFile: string,
  startLine: number,
  targetContent: string,
  replacementContent: string,
): DiffData {
  const baseName = targetFile.split("/").filter(Boolean).pop() || targetFile;
  const dirPath = targetFile.includes("/")
    ? targetFile.substring(0, targetFile.lastIndexOf("/"))
    : "";

  const delLines = targetContent.split("\n");
  const addLines = replacementContent.split("\n");

  const lines: DiffLine[] = [];
  let oldNum = startLine;
  let newNum = startLine;

  for (const del of delLines) {
    lines.push({
      type: "delete",
      oldLineNumber: oldNum++,
      content: del,
    });
  }

  for (const add of addLines) {
    lines.push({
      type: "add",
      newLineNumber: newNum++,
      content: add,
    });
  }

  return {
    fileName: baseName,
    filePath: dirPath || undefined,
    additions: addLines.length,
    deletions: delLines.length,
    lines,
  };
}

/**
 * 解析原生 unified diff (diff -u / git diff) 或 Codex 风格 patch (*** Begin Patch / *** Update File / *** Add File)
 */
export function parseRawDiff(raw: string, fallbackFileName = "modified_file"): DiffData {
  const lines: DiffLine[] = [];
  let additions = 0;
  let deletions = 0;
  let fileName = fallbackFileName;
  let filePath: string | undefined = undefined;

  const rawLines = raw.split("\n");
  let oldNum = 1;
  let newNum = 1;

  for (const line of rawLines) {
    // 0. git diff a/path b/path 头
    if (line.startsWith("diff --git ")) {
      const match = line.match(/diff --git\s+(?:a\/)?([^\s]+)\s+(?:b\/)?([^\s]+)/);
      if (match && match[2]) {
        const full = match[2].trim();
        fileName = full.split("/").filter(Boolean).pop() || full;
        filePath = full.includes("/") ? full.substring(0, full.lastIndexOf("/")) : undefined;
      }
      continue;
    }

    // 1. 标准 unified diff 头: --- a/path +++ b/path
    if (line.startsWith("--- ") || line.startsWith("+++ ")) {
      const match = line.match(/(?:---|\+\+\+)\s+(?:[ab]\/)?([^\s\t\n\r]+)/);
      if (match && match[1] && match[1] !== "/dev/null") {
        const full = match[1].trim();
        fileName = full.split("/").filter(Boolean).pop() || full;
        filePath = full.includes("/") ? full.substring(0, full.lastIndexOf("/")) : undefined;
      }
      continue;
    }

    // 2. Codex 补丁头: *** Update File: path 或 *** Add File: path
    const codexFileMatch = line.match(/\*\*\*\s*(?:Update|Add|Delete)\s*File:\s*([^\n\r*]+)/i);
    if (codexFileMatch && codexFileMatch[1]) {
      const full = codexFileMatch[1].trim();
      fileName = full.split("/").filter(Boolean).pop() || full;
      filePath = full.includes("/") ? full.substring(0, full.lastIndexOf("/")) : undefined;
      continue;
    }

    // 哨兵行过滤
    if (line.trim().startsWith("*** Begin Patch") || line.trim().startsWith("*** End Patch") || line.trim().startsWith("*** Move to:")) {
      continue;
    }

    // 3. Chunk 行号头: @@ -1,5 +1,6 @@ 或 @@
    if (line.startsWith("@@")) {
      const match = line.match(/@@\s+-(\d+)(?:,\d+)?\s+\+(\d+)(?:,\d+)?\s+@@/);
      if (match) {
        oldNum = parseInt(match[1], 10);
        newNum = parseInt(match[2], 10);
      }
      lines.push({
        type: "fold",
        content: line,
      });
      continue;
    }

    // 4. 增删行
    if (line.startsWith("+")) {
      additions++;
      lines.push({
        type: "add",
        newLineNumber: newNum++,
        content: line.slice(1),
      });
    } else if (line.startsWith("-")) {
      deletions++;
      lines.push({
        type: "delete",
        oldLineNumber: oldNum++,
        content: line.slice(1),
      });
    } else if (line.startsWith(" ") || line === "") {
      lines.push({
        type: "normal",
        oldLineNumber: oldNum++,
        newLineNumber: newNum++,
        content: line.startsWith(" ") ? line.slice(1) : line,
      });
    }
  }

  return {
    fileName,
    filePath,
    additions,
    deletions,
    lines,
  };
}

export function DiffViewer({
  diff,
  initialExpanded = false,
  fullBleed = false,
}: {
  diff: DiffData;
  initialExpanded?: boolean;
  fullBleed?: boolean;
}) {
  const [expanded, setExpanded] = useState(initialExpanded);
  const [copied, setCopied] = useState(false);
  const ext = diff.fileName.split(".").pop()?.toLowerCase() || "";

  const handleCopy = (e: React.MouseEvent) => {
    e.stopPropagation();
    const patch = diff.lines
      .map((l) => {
        if (l.type === "add") return `+${l.content}`;
        if (l.type === "delete") return `-${l.content}`;
        return ` ${l.content}`;
      })
      .join("\n");
    void navigator.clipboard.writeText(patch);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  return (
    <div
      className={`w-full font-mono text-[11.5px] ${
        fullBleed
          ? "bg-[var(--surface)] text-[var(--ink)]"
          : "my-2 rounded-2xl border border-[var(--line)] bg-[var(--paper)] text-[var(--ink)] shadow-2xs overflow-hidden"
      }`}
    >
      {/* 顶部标题栏 (仅在非 fullBleed 模式下展示卡片式标题；在面板中已由外层统一面包屑/Tabs 提供) */}
      {!fullBleed && (
        <div
          onClick={() => setExpanded(!expanded)}
          className="flex items-center justify-between px-3.5 py-2 bg-[var(--surface-2)] border-b border-[var(--line)] cursor-pointer hover:bg-[var(--surface-3)]/60 transition-colors select-none"
        >
          <div className="flex items-center gap-2 min-w-0">
            <FileIcon ext={ext} className="w-3.5 h-3.5 shrink-0" />
            <span className="font-bold text-[var(--ink)] text-[12.5px] truncate">
              {diff.fileName}
            </span>
            {diff.filePath && (
              <span className="text-[11px] text-[var(--ink-2)] truncate opacity-80">
                {diff.filePath}
              </span>
            )}
          </div>

          <div className="flex items-center gap-2.5 shrink-0">
            {/* +增 -减 统计数字 */}
            <div className="flex items-center gap-1 font-mono text-[11px] font-semibold">
              {diff.additions > 0 && (
                <span className="text-emerald-500 font-bold">+{diff.additions}</span>
              )}
              {diff.deletions > 0 && (
                <span className="text-rose-500 font-bold">-{diff.deletions}</span>
              )}
            </div>

            {/* 复制按钮 */}
            <button
              type="button"
              onClick={handleCopy}
              className="px-2 py-0.5 rounded-md bg-[var(--surface-3)] hover:bg-[var(--surface-2)] text-[var(--ink)] text-[10px] font-sans transition-colors cursor-pointer border border-[var(--line)] flex items-center gap-1"
              title="复制 Diff 补丁"
            >
              {copied ? <CheckIcon className="w-3 h-3 text-emerald-500" /> : <CopyIcon className="w-3 h-3 text-[var(--ink-3)]" />}
              <span>{copied ? "已复制" : "复制"}</span>
            </button>

            {/* 展开/折叠箭头 */}
            <svg
              width="12"
              height="12"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2.5"
              className={`text-[var(--ink-3)] transition-transform duration-200 ${
                expanded ? "rotate-180" : ""
              }`}
            >
              <path d="M6 9l6 6 6-6" />
            </svg>
          </div>
        </div>
      )}

      {/* 代码 Diff 主体区 */}
      {(expanded || fullBleed) && (
        <div className={`overflow-x-auto divide-y divide-transparent ${fullBleed ? "w-full" : "max-h-[420px] overflow-y-auto"}`}>
          <table className="w-full border-collapse leading-relaxed">
            <tbody>
              {diff.lines.map((line, idx) => {
                if (line.type === "fold") {
                  return (
                    <tr key={idx} className="bg-[var(--surface-2)]/60 text-[var(--ink-3)]">
                      <td colSpan={3} className="px-4 py-1 text-center text-[10.5px] italic font-sans select-none border-y border-[var(--line)]/50">
                        ⋯ {line.content} ⋯
                      </td>
                    </tr>
                  );
                }

                const isAdd = line.type === "add";
                const isDel = line.type === "delete";

                return (
                  <tr
                    key={idx}
                    className={`transition-colors font-mono ${
                      isAdd
                        ? "bg-emerald-500/15 hover:bg-emerald-500/25 text-emerald-950 dark:text-emerald-200"
                        : isDel
                        ? "bg-rose-500/15 hover:bg-rose-500/25 text-rose-950 dark:text-rose-200"
                        : "hover:bg-[var(--surface-2)]/50 text-[var(--ink)]"
                    }`}
                  >
                    {/* 左行号 (原文件) */}
                    <td className="w-10 px-2 py-0.5 text-right text-[10px] text-[var(--ink-3)] select-none border-r border-[var(--line)]/60">
                      {line.oldLineNumber || ""}
                    </td>

                    {/* 右行号 (新文件) */}
                    <td className="w-10 px-2 py-0.5 text-right text-[10px] text-[var(--ink-3)] select-none border-r border-[var(--line)]/60">
                      {line.newLineNumber || ""}
                    </td>

                    {/* 符号与代码行正文 */}
                    <td className="px-3 py-0.5 whitespace-pre font-mono text-[var(--ink)]">
                      <span className={`inline-block w-4 select-none font-bold ${
                        isAdd ? "text-emerald-600 dark:text-emerald-400" : isDel ? "text-rose-600 dark:text-rose-400" : "text-[var(--ink-3)]"
                      }`}>
                        {isAdd ? "+" : isDel ? "-" : " "}
                      </span>
                      {line.content ? (
                        <HighlightedCodeLine line={line.content} ext={ext} />
                      ) : (
                        <span>&nbsp;</span>
                      )}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
