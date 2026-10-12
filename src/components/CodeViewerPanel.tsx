import { useTranslation } from "react-i18next";
import { useState, useMemo, useEffect, useRef } from "react";
import { useCodeViewer } from "../store/codeViewer";
import { useProject } from "../store/project";
import { useChat } from "../store/chat";
import * as api from "../api";
import { HighlightedCodeLine } from "./codeHighlighter";
import { DiffViewer, createReplaceDiff, parseRawDiff, type DiffData } from "./DiffViewer";
import { listSkills } from "../api/voice";
import { activeCount, emptyHint, skillRows, type SkillRow } from "./skillsLoaded";
import { folderState, mergeChildren, needsFetch } from "./fileTree";
import {
  FileIcon,
  CopyIcon,
  CheckIcon,
  CloseIcon,
  MoreVerticalIcon,
  FinderIcon,
  SearchIcon,
} from "./Icons";

export function CodeViewerPanel() {
  const { t } = useTranslation(["settings", "common"]);
  const {
    isOpen,
    mainView,
    tabs,
    activeTabId,
    filePath,
    content,
    isEditing,
    diffData,
    mode,
    lineRange,
    badge,
    loading,
    error,
    setMainView,
    setMode,
    setIsEditing,
    updateTabContent,
    saveCurrentFile,
    commitCurrentProject,
    switchTab,
    closeTab,
  } = useCodeViewer();
  const [fileTree, setFileTree] = useState<api.FileTreeNode[]>([]);
  const [treeLoading, setTreeLoading] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [collapsedFolders, setCollapsedFolders] = useState<Record<string, boolean>>({});
  /** 正在按需取子树的那些目录（路径）。用来把「还没回来」和「真的空」分开。 */
  const [loadingFolders, setLoadingFolders] = useState<Record<string, boolean>>({});
  /** 取子树失败的那些。**必须单独记** —— 失败之后 children 仍是 null，
      只看 `needsFetch` 会一直显示「载入中…」，一条永远转不完的提示。 */
  const [failedFolders, setFailedFolders] = useState<Record<string, boolean>>({});
  const [wrapLines, setWrapLines] = useState(true);
  const [collapsedBlocks, setCollapsedBlocks] = useState<Record<number, number>>({});

  const [copiedContent, setCopiedContent] = useState(false);
  const [copiedPath, setCopiedPath] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  // 保存与提交状态
  const [isSaving, setIsSaving] = useState(false);
  const [saveSuccessNotice, setSaveSuccessNotice] = useState(false);
  const [isCommitModalOpen, setIsCommitModalOpen] = useState(false);
  const [commitMessage, setCommitMessage] = useState("");
  const [isCommitting, setIsCommitting] = useState(false);
  const [commitFeedback, setCommitFeedback] = useState<{ ok: boolean; msg: string } | null>(null);

  const activeTab = useMemo(() => tabs.find((t) => t.id === activeTabId), [tabs, activeTabId]);
  const currentContent = activeTab?.editedContent ?? content;

  const baseName = filePath ? filePath.split("/").filter(Boolean).pop() || filePath : "";
  const ext = useMemo(() => baseName.split(".").pop()?.toLowerCase() || "", [baseName]);

  // 面包屑路径
  const breadcrumbs = useMemo(() => {
    if (!filePath) return [];
    return filePath.split("/").filter(Boolean);
  }, [filePath]);

  const lines = useMemo(() => currentContent.split("\n"), [currentContent]);

  // 快捷键 Cmd+S / Ctrl+S 保存
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "s") {
        if (isOpen && mainView === "code" && filePath) {
          e.preventDefault();
          handleSaveFile();
        }
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [isOpen, mainView, filePath, activeTabId, currentContent]);

  const handleSaveFile = async () => {
    if (isSaving) return;
    setIsSaving(true);
    const ok = await saveCurrentFile();
    setIsSaving(false);
    if (ok) {
      setSaveSuccessNotice(true);
      setTimeout(() => setSaveSuccessNotice(false), 2000);
    }
  };

  const handleExecuteCommit = async () => {
    if (!commitMessage.trim() || isCommitting) return;
    setIsCommitting(true);
    setCommitFeedback(null);
    try {
      const res = await commitCurrentProject(
        commitMessage.trim(),
        filePath ? [filePath] : undefined
      );
      setCommitFeedback({
        ok: true,
        msg: `提交成功: ${res.commit_hash} (${res.branch})`,
      });
      setTimeout(() => {
        setIsCommitModalOpen(false);
        setCommitMessage("");
        setCommitFeedback(null);
      }, 1500);
    } catch (err: any) {
      setCommitFeedback({
        ok: false,
        msg: String(err?.message || err || "提交失败"),
      });
    } finally {
      setIsCommitting(false);
    }
  };

  // 重置折叠状态（当切换文件时）
  useEffect(() => {
    setCollapsedBlocks({});
  }, [filePath]);

  // 计算代码中所有可折叠的代码块（支持花括号 {}、中括号 []、多行注释 /* */、Python 缩进块、Markdown 标题/代码块）
  const foldableBlocks = useMemo(() => {
    const blocks = new Map<number, number>(); // startLine (1-based) -> endLine (1-based)
    if (!lines || lines.length === 0) return blocks;

    // 1. 基于花括号 {} 的成对匹配
    const stack: number[] = [];
    for (let i = 0; i < lines.length; i++) {
      const line = lines[i];
      // 忽略字符串和单行注释内的括号简易处理
      for (let c = 0; c < line.length; c++) {
        if (line[c] === "{" && !(c > 0 && line[c - 1] === "\\")) {
          stack.push(i + 1);
        } else if (line[c] === "}" && !(c > 0 && line[c - 1] === "\\")) {
          const start = stack.pop();
          if (start !== undefined && i + 1 > start) {
            blocks.set(start, i + 1);
          }
        }
      }
    }

    // 2. 多行注释 /* ... */
    let inCommentStart: number | null = null;
    for (let i = 0; i < lines.length; i++) {
      const line = lines[i].trim();
      if (inCommentStart === null && (line.startsWith("/*") || line.startsWith("/**"))) {
        inCommentStart = i + 1;
      }
      if (inCommentStart !== null && line.includes("*/")) {
        if (i + 1 > inCommentStart) {
          blocks.set(inCommentStart, i + 1);
        }
        inCommentStart = null;
      }
    }

    // 3. Markdown 代码块 ```
    if (ext === "md" || ext === "markdown") {
      let mdFenceStart: number | null = null;
      for (let i = 0; i < lines.length; i++) {
        const line = lines[i].trim();
        if (line.startsWith("```")) {
          if (mdFenceStart === null) {
            mdFenceStart = i + 1;
          } else {
            if (i + 1 > mdFenceStart) {
              blocks.set(mdFenceStart, i + 1);
            }
            mdFenceStart = null;
          }
        }
      }
    }

    return blocks;
  }, [lines, ext]);

  const toggleFold = (startLine: number, endLine: number) => {
    setCollapsedBlocks((prev) => {
      const next = { ...prev };
      if (next[startLine]) {
        delete next[startLine];
      } else {
        next[startLine] = endLine;
      }
      return next;
    });
  };

  // 点击外部关闭三点菜单
  useEffect(() => {
    if (!menuOpen) return;
    const handleClickOutside = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setMenuOpen(false);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, [menuOpen]);

  // 定位与平滑滚动到高亮行号
  useEffect(() => {
    if (!loading && lineRange && lineRange[0]) {
      const targetId = `code-line-${lineRange[0]}`;
      const timer = setTimeout(() => {
        const targetEl = document.getElementById(targetId);
        if (targetEl) {
          targetEl.scrollIntoView({ behavior: "smooth", block: "center" });
        }
      }, 50);
      return () => clearTimeout(timer);
    }
  }, [loading, lineRange, content]);

  const handleCopyContent = () => {
    const textToCopy =
      mode === "diff" && diffData
        ? diffData.lines.map((l) => l.content).join("\n")
        : content;
    if (!textToCopy) return;
    void navigator.clipboard.writeText(textToCopy);
    setCopiedContent(true);
    setTimeout(() => {
      setCopiedContent(false);
      setMenuOpen(false);
    }, 1200);
  };

  const handleCopyPath = () => {
    if (!filePath) return;
    void navigator.clipboard.writeText(filePath);
    setCopiedPath(true);
    setTimeout(() => {
      setCopiedPath(false);
      setMenuOpen(false);
    }, 1200);
  };

  const handleRevealInFinder = async () => {
    if (!filePath) return;
    try {
      const projectState = useProject.getState();
      const activeProject = projectState.projects.find(
        (p) => p.id === projectState.activeProjectId
      );
      const wsRoot = activeProject?.workspace_path || "";
      const fullPath = filePath.startsWith("/") ? filePath : `${wsRoot}/${filePath}`;
      await api.workspaceOpenInFinder(fullPath);
      setMenuOpen(false);
    } catch (err) {
      console.error("Reveal in Finder failed:", err);
    }
  };

  const activeProject = useProject((s) =>
    s.projects.find((p) => p.id === s.activeProjectId)
  );
  const turns = useChat((s) => s.turns);

  // 加载目录树
  useEffect(() => {
    if (!isOpen || (mainView !== "files" && mainView !== "overview")) return;
    const wsPath = activeProject?.workspace_path;
    let cancelled = false;
    setTreeLoading(true);

    api
      .workspaceListDirectoryTree(wsPath, null, 4)
      .then((tree) => {
        if (!cancelled) {
          setFileTree(tree);
          setTreeLoading(false);
        }
      })
      .catch((err) => {
        console.error("Failed to load file tree:", err);
        if (!cancelled) setTreeLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [isOpen, mainView, activeProject?.workspace_path]);

  // 从项目工作区真实 Git 状态中读取未提交修改文件列表 (对标 Antigravity 真实工作区状态)
  const [gitStatusChanges, setGitStatusChanges] = useState<import("../api/types").WorkspaceGitFileChange[]>([]);

  // 轮询与监听当前项目的真实 Git 修改状态
  useEffect(() => {
    if (!isOpen || (mainView !== "files" && mainView !== "overview")) return;
    const wsPath = activeProject?.workspace_path;
    let cancelled = false;

    const fetchStatus = () => {
      api
        .workspaceGitStatus(wsPath)
        .then((res) => {
          if (!cancelled && res.is_git) {
            setGitStatusChanges(res.changes);
          } else if (!cancelled) {
            setGitStatusChanges([]);
          }
        })
        .catch(() => {
          if (!cancelled) setGitStatusChanges([]);
        });
    };

    fetchStatus();
    // 每 2.5 秒定期刷新工作区 git 变动，并支持即时事件
    const timer = setInterval(fetchStatus, 2500);

    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [isOpen, mainView, activeProject?.workspace_path, turns]);

  // 从真实 Git 状态 + 当前会话生成中提取所有被修改/创建的文件变更统计
  const modifiedFilesSummary = useMemo(() => {
    interface ModifiedFileEntry {
      additions: number;
      deletions: number;
      path: string;
      diffData?: DiffData;
    }
    const fileMap = new Map<string, ModifiedFileEntry>();

    // 1. 首先加载工作区当前真实的 Git 挂起变更 (即使新开会话也能即刻呈现项目级未提交文件)
    for (const g of gitStatusChanges) {
      const clean = g.path.replace(/^file:\/\//, "");
      const key = clean.split("/").pop() || clean;
      fileMap.set(key, {
        additions: g.additions,
        deletions: g.deletions,
        path: clean,
      });
    }

    // 2. 结合当前会话的工具调用与补丁生成
    for (const turn of turns) {
      if (turn.toolCalls) {
        for (const tc of turn.toolCalls) {
          let args: any = {};
          try {
            if (typeof tc.arguments === "string") {
              args = JSON.parse(tc.arguments);
            } else {
              args = tc.arguments || {};
            }
          } catch {
            args = {};
          }

          const name = tc.name.toLowerCase();

          // 常规写入/修改/替换工具
          if (
            name.includes("patch") ||
            name.includes("write") ||
            name.includes("replace") ||
            name.includes("edit") ||
            name.includes("create")
          ) {
            let rawPath =
              args.path ||
              args.TargetFile ||
              args.target_file ||
              args.file ||
              args.filePath ||
              args.filepath ||
              "";

            // 从 unified diff 补丁中提取路径
            if (!rawPath && typeof args.patch === "string") {
              const match = args.patch.match(/\*\*\*\s*(?:Update|Add|Delete)\s*File:\s*([^\n\r]+)/i);
              if (match) rawPath = match[1].trim();
            }

            if (rawPath) {
              const clean = rawPath.replace(/^file:\/\//, "");
              const baseName = clean.split("/").filter(Boolean).pop() || clean;

              let diffData: DiffData | undefined = undefined;
              if (args.TargetContent !== undefined && args.ReplacementContent !== undefined) {
                diffData = createReplaceDiff(clean, args.StartLine || 1, String(args.TargetContent), String(args.ReplacementContent));
              } else if (typeof args.patch === "string") {
                diffData = parseRawDiff(args.patch, baseName);
              } else if (args.CodeContent !== undefined) {
                diffData = createReplaceDiff(clean, 1, "", String(args.CodeContent));
              } else if (args.content !== undefined && typeof args.content === "string") {
                diffData = createReplaceDiff(clean, 1, "", String(args.content));
              }

              const key = clean.split("/").pop() || clean;
              const current: ModifiedFileEntry = fileMap.get(key) || { additions: 0, deletions: 0, path: clean };
              if (diffData) {
                current.diffData = diffData;
                current.additions = diffData.additions;
                current.deletions = diffData.deletions;
                current.path = clean;
              }
              fileMap.set(key, current);
            }
          }
        }
      }
    }
    return Array.from(fileMap.values());
  }, [gitStatusChanges, turns]);

  // 从会话历史中提取所有派发过的子智能体（对标 Antigravity 右侧 Subagents 监控）
  const subagentsSummary = useMemo(() => {
    const list: Array<{
      id: string;
      role: string;
      mode: string;
      goal: string;
      status: "running" | "completed";
      resultSummary?: string;
    }> = [];

    for (const turn of turns) {
      if (turn.toolCalls) {
        for (const tc of turn.toolCalls) {
          if (tc.name === "invoke_subagent") {
            let args: any = {};
            try {
              args = typeof tc.arguments === "string" ? JSON.parse(tc.arguments) : tc.arguments || {};
            } catch {
              args = {};
            }
            const role = args.role || "Subagent";
            const mode = (args.mode || "explore").toLowerCase();
            const goal = args.goal || "";
            const isCompleted = !turn.streaming;

            list.push({
              id: tc.id || `${turn.id}-${role}`,
              role,
              mode,
              goal,
              status: isCompleted ? "completed" : "running",
            });
          }
        }
      }

      // 从历史消息文本 (turn.content) 中提取子智能体报告（会话重新加载后依然保留）
      if (typeof turn.content === "string") {
        const reportMatch = turn.content.match(/# 子智能体报告（([^）]+)）[\s\S]*?- \*\*拓扑模式\*\*：`([^`]+)`/);
        if (reportMatch) {
          const role = reportMatch[1].trim();
          const mode = reportMatch[2].toLowerCase().includes("isolated") ? "isolated" : "explore";
          if (!list.some((s) => s.role === role)) {
            list.push({
              id: `${turn.id}-${role}`,
              role,
              mode,
              goal: "",
              status: "completed",
            });
          }
        }
      }
    }
    return list;
  }, [turns]);

function extractCleanUploadName(pathOrUrl: string, fallback: string): string {
  if (!pathOrUrl || pathOrUrl.startsWith("data:")) return fallback;
  try {
    let s = pathOrUrl;
    if (s.includes("asset.localhost/") || s.includes("localhost/")) {
      const idx = s.indexOf("localhost/");
      if (idx >= 0) s = s.slice(idx + "localhost/".length);
    }
    s = decodeURIComponent(s);
    const rawName = s.replace(/^file:\/\//, "").split("/").filter(Boolean).pop() || "";
    const clean = rawName.replace(/_[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}(\.[a-zA-Z0-9]+)$/i, "$1");
    return clean || rawName || fallback;
  } catch {
    return pathOrUrl.split("/").pop() || fallback;
  }
}

  // 从会话历史中提取所有上传的文件/图片资源 (Uploads)
  const uploadsSummary = useMemo(() => {
    const list: Array<{ id: string; name: string; url: string; time?: string }> = [];
    for (const turn of turns) {
      if (turn.role === "user" && typeof turn.content === "string" && turn.content.includes("[上传图片]")) {
        const imageLines = Array.from(
          turn.content.matchAll(/- Image \d+:\s*([^\n(]+)\s*\([^)]*\)\s*\n\s*file_url:\s*([^\r\n]+)/g)
        );
        if (imageLines.length > 0) {
          imageLines.forEach((m, idx) => {
            const originalName = m[1].trim();
            const fileUrl = m[2].trim();
            list.push({
              id: `${turn.id}-named-img-${idx}`,
              name: originalName,
              url: fileUrl,
            });
          });
        } else {
          const fileUrls = Array.from(turn.content.matchAll(/file_url:\s*([^\r\n]+)/g)).map((m) => m[1].trim());
          const localPaths = Array.from(turn.content.matchAll(/local_path:\s*([^\r\n]+)/g)).map((m) => m[1].trim());
          const extracted = fileUrls.length > 0 ? fileUrls : localPaths;
          extracted.forEach((p, idx) => {
            const name = extractCleanUploadName(p, `Image ${idx + 1}`);
            list.push({
              id: `${turn.id}-uploaded-${idx}`,
              name,
              url: p,
            });
          });
        }
      } else if (turn.images && turn.images.length > 0) {
        turn.images.forEach((img, idx) => {
          const name = extractCleanUploadName(img, `Image ${idx + 1}`);
          list.push({
            id: `${turn.id}-img-${idx}`,
            name,
            url: img,
          });
        });
      }
    }
    return list;
  }, [turns]);

  // 从会话历史与工具调用中提取所有交付成果 / 方案文档 / 生成图像与视频 (Artifacts)
  // 侧栏里被用户手动移除的 artifact。
  //
  // **只从列表里去掉，不动磁盘上的文件** —— 生成物是用户的产出，
  // 侧栏的叉号在任何界面里都是"从列表里拿掉"，不是"永久删除"。
  // 真要删文件得是明确的、可确认的动作。
  const [dismissedArtifacts, setDismissedArtifacts] = useState<Set<string>>(new Set());


  const artifactsSummary = useMemo(() => {
    const list: Array<{ id: string; name: string; path?: string; turnId?: string }> = [];
    for (const turn of turns) {
      if (turn.toolCalls) {
        for (const tc of turn.toolCalls) {
          const name = tc.name.toLowerCase();
          if (
            name.includes("artifact") ||
            name.includes("write_to_file") ||
            name.includes("generate_image") ||
            name.includes("generate_video") ||
            name.includes("animate_image")
          ) {
            let args: any = {};
            try {
              args = typeof tc.arguments === "string" ? JSON.parse(tc.arguments) : tc.arguments || {};
            } catch {
              args = {};
            }
            const title =
              args.title ||
              args.TargetFile?.split("/").pop() ||
              (name.includes("image")
                ? `🖼️ ${(args.prompt || "生成的图片").slice(0, 24)}`
                : name.includes("video")
                ? `🎬 ${(args.prompt || args.motion_prompt || "生成的视频").slice(0, 24)}`
                : "Artifact Document");
            if (!list.some((a) => a.name === title)) {
              // **生成物的路径在工具返回里，不在参数里。**
              //
              // 原来写的是 `path: args.TargetFile` —— 那是 write_to_file 的参数名。
              // generate_image / generate_video / animate_image 都没有它，所以 path
              // 永远是 undefined，点击处理里的 `if (art.path)` 永远不成立 ——
              // 条目点不动，而且不报任何错。
              let outPath: string | undefined = args.TargetFile;
              if (!outPath) {
                const tr = turn.toolResults?.find((r: any) => r.callId === tc.id);
                const m = String(tr?.content || "").match(/file:\/\/([^\s)\n]+)/);
                if (m) outPath = decodeURIComponent(m[1]);
              }
              list.push({
                id: tc.id || `${turn.id}-art`,
                name: title,
                path: outPath,
                // 图片/视频已经在对话里渲染好了，点击应该滚过去，
                // 而不是塞进只会显示代码和 diff 的查看器
                turnId:
                  name.includes("generate_") || name.includes("animate_")
                    ? turn.id
                    : undefined,
              });
            }
          }
        }
      }
      if (typeof turn.content === "string") {
        const matches = turn.content.match(/^#+\s+([^\n]+)/gm);
        if (matches) {
          for (const m of matches) {
            const heading = m.replace(/^#+\s+/, "").trim();
            if (
              heading.toLowerCase().includes("plan") ||
              heading.toLowerCase().includes("artifact") ||
              heading.includes("方案") ||
              heading.includes("文档") ||
              heading.includes("总结")
            ) {
              if (!list.some((a) => a.name === heading)) {
                list.push({ id: `${turn.id}-${heading}`, name: heading });
              }
            }
          }
        }
      }
    }
      return list.filter((a) => !dismissedArtifacts.has(a.id));
    }, [turns, dismissedArtifacts]);

  // 从会话历史中提取后台/定时/视觉生成/MCP任务 (Background Tasks)
  const backgroundTasksSummary = useMemo(() => {
    // status 必须含 failed —— 少了它 `task.status === "failed"` 那个红点分支是死代码，
    // tsc 早就在报（TS2367），只是 vite build 不做类型检查所以一直没人看见
    const list: Array<{ id: string; name: string; status: "running" | "completed" | "failed"; detail?: string }> = [];
    for (const turn of turns) {
      if (turn.toolCalls) {
        for (const tc of turn.toolCalls) {
          const name = tc.name.toLowerCase();
          if (
            name.includes("schedule") ||
            name.includes("task") ||
            name.includes("manage_task") ||
            name.includes("mcp") ||
            name.includes("generate_image") ||
            name.includes("generate_video") ||
            name.includes("animate_image")
          ) {
            let args: any = {};
            try {
              args = typeof tc.arguments === "string" ? JSON.parse(tc.arguments) : tc.arguments || {};
            } catch {
              args = {};
            }
            const isVisual = name.includes("generate_") || name.includes("animate_");
            const isVideo = name.includes("video") || name.includes("animate_");
            const visualLabel = isVideo ? "视频渲染" : "生图任务";
            const title = isVisual
              ? `${visualLabel}: ${(args.prompt || args.motion_prompt || "ComfyUI").slice(0, 30)}`
              : args.Prompt || args.tool || args.name || tc.name;
            const toolResult = turn.toolResults?.find((r) => r.callId === tc.id || r.name === tc.name);
            const isFailed = toolResult?.ok === false || !!turn.error;
            const isCompleted = !turn.streaming;
            const status: "running" | "completed" | "failed" = isFailed
              ? "failed"
              : isCompleted
              ? "completed"
              : "running";
            list.push({
              id: tc.id || `task-${turn.id}`,
              name: String(title).slice(0, 45),
              status,
              detail: isFailed
                ? "失败"
                : isVisual
                ? (isCompleted ? "已完成" : "生成中")
                : args.CronExpression || (args.DurationSeconds ? `${args.DurationSeconds}s` : undefined),
            });
          }
        }
      }
    }
    return list;
  }, [turns]);

  // 从会话历史中提取终端命令执行记录 (Terminals)
  const terminalsSummary = useMemo(() => {
    const list: Array<{ id: string; command: string; pid: string; status: "running" | "completed" }> = [];
    for (const turn of turns) {
      if (turn.toolCalls) {
        for (const tc of turn.toolCalls) {
          const name = tc.name.toLowerCase();
          if (
            name === "run_command" ||
            name === "execute_command" ||
            name === "terminal" ||
            name.includes("bash") ||
            name.includes("shell")
          ) {
            let args: any = {};
            try {
              args = typeof tc.arguments === "string" ? JSON.parse(tc.arguments) : tc.arguments || {};
            } catch {
              args = {};
            }
            const cmd = args.CommandLine || args.command || args.cmd || "zsh";
            list.push({
              id: tc.id || `cmd-${turn.id}`,
              command: String(cmd).slice(0, 50),
              pid: String(Math.floor(10000 + Math.random() * 89999)),
              status: !turn.streaming ? "completed" : "running",
            });
          }
        }
      }
    }
    return list;
  }, [turns]);

  /**
   * 技能的**真实加载状态**。
   *
   * 这里原来是扫会话里「名字带 skill 的工具调用」—— 而 Anima 里
   * 没有任何一个工具叫这个名字。技能是「选中即生效」的一层
   * （正文进 System Prompt、`tools` 收窄工具面），不是模型去调的动作。
   * 所以那个数字**结构上永远是 0**，用户看到 0 会以为技能没加载，
   * 而实际上磁盘上那几个都加载了，只是没有任何东西能证明。
   *
   * 现在读 `list_skills` 返回的真相：认出了哪些、这次生效的是哪些。
   */
  const [skillRowsState, setSkillRowsState] = useState<SkillRow[]>([]);
  useEffect(() => {
    if (!isOpen) return;
    let alive = true;
    void listSkills(activeProject?.workspace_path ?? null)
      .then((view) => {
        if (alive) setSkillRowsState(skillRows(view.skills, view.active));
      })
      // 读不到就当空的 —— 一个侧栏格子不该弹错误打断工作
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [isOpen, activeProject?.workspace_path, turns.length]);

  // Overview 区域的折叠状态（对标 Antigravity 默认可折叠式抽屉，覆盖全部 7 大模块）
  const [sectionCollapsed, setSectionCollapsed] = useState<Record<string, boolean>>({
    subagents: false,
    files: false,
    artifacts: false,
    uploads: false,
    backgroundTasks: false,
    terminals: false,
    skillsUsed: false,
  });

  const toggleSection = (key: string) => {
    setSectionCollapsed((prev) => ({ ...prev, [key]: !prev[key] }));
  };

  // 右侧边栏拖拽宽度调整
  const [panelWidth, setPanelWidth] = useState<number>(() => {
    const saved = localStorage.getItem("anima_code_viewer_width");
    return saved ? Math.max(380, Math.min(1000, parseInt(saved, 10))) : 560;
  });
  const isResizingRef = useRef(false);

  const handleMouseDownResize = (e: React.MouseEvent) => {
    e.preventDefault();
    isResizingRef.current = true;
    const startX = e.clientX;
    const startWidth = panelWidth;

    const handleMouseMove = (ev: MouseEvent) => {
      if (!isResizingRef.current) return;
      const delta = startX - ev.clientX; // 向左拖拽变大
      const newWidth = Math.max(360, Math.min(1100, startWidth + delta));
      setPanelWidth(newWidth);
    };

    const handleMouseUp = () => {
      isResizingRef.current = false;
      localStorage.setItem("anima_code_viewer_width", String(panelWidth));
      window.removeEventListener("mousemove", handleMouseMove);
      window.removeEventListener("mouseup", handleMouseUp);
    };

    window.addEventListener("mousemove", handleMouseMove);
    window.addEventListener("mouseup", handleMouseUp);
  };

  /**
   * 这个目录**当前是不是折叠的**。
   *
   * 默认展开，但**还没取回内容的目录默认折叠** —— 否则它一上来就是
   * 「展开了什么都没有」，正是要修的那个症状；而且折叠状态下的箭头
   * 本身就是"这里还有东西"的提示。
   */
  const folderCollapsed = (node: api.FileTreeNode): boolean =>
    collapsedFolders[node.path] ?? needsFetch(node);

  /**
   * 展开/折叠一个目录。**展开时如果它还有内容没取回来，就现取一层。**
   *
   * 后端一次只往下走 4 层（一次全展开大目录要遍历上千个文件），
   * 超出的目录带 `truncated: true` 回来。不按需取的话，第 5 层以下
   * 就是「展开了什么都没有」—— 和空目录一模一样，还不报错。
   */
  const fetchSubtree = (node: api.FileTreeNode) => {
    const wsPath = activeProject?.workspace_path;
    setLoadingFolders((prev) => ({ ...prev, [node.path]: true }));
    setFailedFolders((prev) => {
      const next = { ...prev };
      delete next[node.path];
      return next;
    });
    api
      .workspaceListDirectoryTree(wsPath, node.path, 4)
      .then((kids) => {
        // 后端回来的 path 是**相对工作区根**的，所以能直接挂
        setFileTree((prev) => mergeChildren(prev, node.path, kids) as api.FileTreeNode[]);
      })
      .catch((err) => {
        console.error("Failed to load subtree:", node.path, err);
        setFailedFolders((prev) => ({ ...prev, [node.path]: true }));
      })
      .finally(() => {
        setLoadingFolders((prev) => {
          const next = { ...prev };
          delete next[node.path];
          return next;
        });
      });
  };

  const toggleFolder = (node: api.FileTreeNode) => {
    const wasCollapsed = folderCollapsed(node);
    setCollapsedFolders((prev) => ({ ...prev, [node.path]: !wasCollapsed }));

    // 折叠时不取；已经取过的也不再取（`needsFetch` 会看 children）
    if (!wasCollapsed || !needsFetch(node)) return;
    fetchSubtree(node);
  };

  // 递归渲染文件树
  const renderTreeNodes = (nodes: api.FileTreeNode[], depth = 0) => {
    return nodes.map((node) => {
      const isCollapsed = folderCollapsed(node);
      const isCurrentActive = filePath === node.path || filePath?.endsWith(node.path);

      if (
        searchQuery &&
        !node.name.toLowerCase().includes(searchQuery.toLowerCase()) &&
        !node.path.toLowerCase().includes(searchQuery.toLowerCase())
      ) {
        if (!node.children || node.children.length === 0) return null;
      }

      return (
        <div key={node.path} className="flex flex-col select-none">
          <div
            onClick={() => {
              if (node.is_dir) {
                toggleFolder(node);
              } else {
                void useCodeViewer.getState().openFile(node.path);
                setMainView("code");
              }
            }}
            style={{ paddingLeft: `${depth * 14 + 10}px` }}
            className={`flex items-center justify-between pr-3 py-1 text-[12px] font-mono rounded-lg transition-colors cursor-pointer group ${
              isCurrentActive
                ? "bg-[var(--surface-3)] text-[var(--ink)] font-semibold shadow-2xs"
                : "text-[var(--ink-2)] hover:bg-[var(--surface-3)]/60 hover:text-[var(--ink)]"
            }`}
          >
            <div className="flex items-center gap-2 truncate min-w-0">
              {node.is_dir ? (
                <span className="text-[var(--ink-3)] group-hover:text-[var(--ink)] flex items-center gap-1 shrink-0">
                  <svg
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="2.5"
                    className={`w-3 h-3 transition-transform duration-150 ${
                      isCollapsed ? "" : "rotate-90"
                    }`}
                  >
                    <path d="M9 18l6-6-6-6" />
                  </svg>
                  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" className="w-3.5 h-3.5 text-[var(--amber)]">
                    <path d="M4 20h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.93a2 2 0 0 1-1.66-.9l-.82-1.2A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13c0 1.1.9 2 2 2Z" />
                  </svg>
                </span>
              ) : (
                <FileIcon ext={node.extension || ""} className="w-3.5 h-3.5 shrink-0" />
              )}
              <span className={`truncate ${isCurrentActive ? "text-[var(--ink)] font-semibold" : "text-[var(--ink)]"}`}>{node.name}</span>
            </div>

            {!node.is_dir && node.size_bytes > 0 && (
              <span className={`text-[10px] font-sans shrink-0 ml-2 ${isCurrentActive ? "text-[var(--ink-2)]" : "text-[var(--ink-3)] group-hover:text-[var(--ink-2)]"}`}>
                {node.size_bytes < 1024
                  ? `${node.size_bytes} B`
                  : `${(node.size_bytes / 1024).toFixed(1)} KB`}
              </span>
            )}
          </div>

          {node.is_dir && !isCollapsed && (
            <div className="flex flex-col">
              {(() => {
                // **三种状态必须分开显示。** 「还没取回来」和「真的是空目录」
                // 原先都渲染成什么都没有，于是第 5 层以下不见了这件事
                // 看起来和"这个目录本来就是空的"一模一样，还不报错。
                const st = folderState(
                  node,
                  !!loadingFolders[node.path],
                  !!failedFolders[node.path],
                );
                if (st === "children") return renderTreeNodes(node.children!, depth + 1);
                if (st === "failed") {
                  return (
                    <div
                      style={{ paddingLeft: `${(depth + 1) * 14 + 10}px` }}
                      onClick={(e) => {
                        e.stopPropagation();
                        fetchSubtree(node);
                      }}
                      className="py-1 pr-3 text-[11px] text-[var(--clay)] cursor-pointer hover:underline"
                    >
                      读不出来 · 点一下重试
                    </div>
                  );
                }
                return (
                  <div
                    style={{ paddingLeft: `${(depth + 1) * 14 + 10}px` }}
                    className="py-1 pr-3 text-[11px] text-[var(--ink-3)] italic"
                  >
                    {st === "loading" ? "载入中…" : "空目录"}
                  </div>
                );
              })()}
            </div>
          )}
        </div>
      );
    });
  };

  const displayBadge =
    badge ||
    (lineRange
      ? lineRange[0] === lineRange[1]
        ? `L${lineRange[0]}`
        : `L${lineRange[0]}-L${lineRange[1]}`
      : undefined);

  if (!isOpen) return null;

  return (
    <div
      style={{ width: `${panelWidth}px` }}
      className="relative flex flex-col shrink-0 h-full bg-[var(--surface)] border-l border-[var(--line)] text-[var(--ink)] z-20 animate-in slide-in-from-right duration-150 select-text"
    >
      {/* 左侧可拖拽调整宽度分割手柄（极细、浅灰微光） */}
      <div
        onMouseDown={handleMouseDownResize}
        className="absolute -left-[3px] top-0 bottom-0 w-[6px] cursor-col-resize hover:bg-[var(--line)]/40 active:bg-[var(--line)]/80 transition-colors z-30 group flex items-center justify-center select-none"
        title="按住左右拖拽调整分栏宽度"
      >
        <div className="w-[1px] h-5 bg-transparent group-hover:bg-[var(--ink-3)]/60 group-active:bg-[var(--ink-2)]/70 rounded-full transition-colors" />
      </div>

      {/* 顶部紧凑多标签页栏（左侧：3个主视图切换图标 + | 分割线；右侧：文件标签列表 + 最右侧折叠按钮同一行） */}
      <div
        data-tauri-drag-region
        className="pt-3 pb-2 px-3 flex items-center justify-between bg-[var(--surface)] border-b border-[var(--line)] shrink-0 select-none gap-2 min-w-0"
      >
        {/* 左侧独立区域：Overview / Files / Code 三主标签切换 + | 分割线 */}
        <div className="flex items-center gap-1 shrink-0">
          <div className="flex items-center gap-0.5 text-[11px] font-sans">
            <button
              type="button"
              onClick={() => setMainView("overview")}
              className={`p-1.5 rounded-lg transition-colors cursor-pointer flex items-center justify-center ${
                mainView === "overview"
                  ? "bg-[var(--surface-3)] text-[var(--ink)] shadow-2xs font-semibold"
                  : "text-[var(--ink-2)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)]/60"
              }`}
              title="任务总览与变更 (Overview)"
            >
              {/* Antigravity 概览/文档图标 */}
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <rect x="3" y="3" width="18" height="18" rx="2" ry="2" />
                <line x1="7" y1="8" x2="17" y2="8" />
                <line x1="7" y1="12" x2="17" y2="12" />
                <line x1="7" y1="16" x2="13" y2="16" />
              </svg>
            </button>
            <button
              type="button"
              onClick={() => setMainView("files")}
              className={`p-1.5 rounded-lg transition-colors cursor-pointer flex items-center justify-center ${
                mainView === "files"
                  ? "bg-[var(--surface-3)] text-[var(--ink)] shadow-2xs font-semibold"
                  : "text-[var(--ink-2)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)]/60"
              }`}
              title="项目文件目录 (Files Tree)"
            >
              {/* Antigravity 双文件/文件层叠图标 */}
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <rect x="8" y="2" width="13" height="16" rx="2" ry="2" />
                <path d="M4 6h2v14a2 2 0 0 0 2 2h11v2H6a2 2 0 0 1-2-2V6z" />
              </svg>
            </button>
            <button
              type="button"
              onClick={() => setMainView("code")}
              className={`p-1.5 rounded-lg transition-colors cursor-pointer flex items-center justify-center ${
                mainView === "code"
                  ? "bg-[var(--surface-3)] text-[var(--ink)] shadow-2xs font-semibold"
                  : "text-[var(--ink-2)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)]/60"
              }`}
              title="代码查看器 (Code Editor/Diff)"
            >
              {/* 标准代码标签/括号图标 < > */}
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <polyline points="16 18 22 12 16 6" />
                <polyline points="8 6 2 12 8 18" />
              </svg>
            </button>
          </div>

          {/* 垂直分割线 ｜ */}
          <div className="w-[1px] h-4 bg-[var(--line)] mx-1 shrink-0" />
        </div>

        {/* 中间/右侧：文件标签列表（可左右平滑滚动，隐藏滚动条） */}
        <div className="flex items-center gap-1.5 overflow-x-auto min-w-0 flex-1 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
          {tabs.map((tab) => {
            const isActive = tab.id === activeTabId && mainView === "code";
            const tabExt = tab.title.split(".").pop()?.toLowerCase() || "";
            return (
              <div
                key={tab.id}
                onClick={() => {
                  switchTab(tab.id);
                  setMainView("code");
                }}
                className={`flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-[12px] font-mono transition-all cursor-pointer shadow-2xs shrink-0 select-none ${
                  isActive
                    ? "bg-[var(--surface-3)] border border-[var(--line)] text-[var(--ink)] font-semibold"
                    : "bg-[var(--surface-2)] hover:bg-[var(--surface-3)]/60 text-[var(--ink-2)] hover:text-[var(--ink)] border border-transparent"
                }`}
              >
                <FileIcon ext={tabExt} className="w-3.5 h-3.5 shrink-0" />
                <span className="truncate max-w-[130px]">{tab.title}</span>
                {/* 未保存修改指示点 */}
                {tab.isDirty && (
                  <span className="w-1.5 h-1.5 rounded-full bg-amber-500 shadow-[0_0_4px_rgba(245,158,11,0.6)] shrink-0" title="有未保存的修改" />
                )}
                {isActive && displayBadge && (
                  <span className="text-[10px] text-[var(--celadon)] font-mono font-medium bg-[var(--celadon)]/15 border border-[var(--celadon)]/30 px-1 py-0.2 rounded shrink-0">
                    {displayBadge}
                  </span>
                )}
                <button
                  type="button"
                  onClick={(e) => {
                    e.stopPropagation();
                    if (tab.isDirty && !window.confirm(`文件 "${tab.title}" 存在未保存的修改，确定关闭吗？`)) {
                      return;
                    }
                    closeTab(tab.id);
                    if (tabs.length <= 1) {
                      setMainView("files");
                    }
                  }}
                  className="ml-0.5 text-[var(--ink-3)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)] rounded p-0.5 transition-colors cursor-pointer shrink-0"
                  title="关闭此标签"
                >
                  <CloseIcon className="w-3 h-3" />
                </button>
              </div>
            );
          })}
        </div>

        {/* 同一行最右侧：侧栏折叠收起按钮 (与左侧 tag 在同一水平高度，深浅中性色处理) */}
        <button
          type="button"
          onClick={() => useCodeViewer.getState().close()}
          className="p-1.5 rounded-lg text-[var(--ink-2)] bg-[var(--surface-3)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)]/80 transition-colors cursor-pointer shrink-0 ml-1"
          title="收起右侧面板"
          aria-label="收起右侧面板"
        >
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <rect x="3" y="3" width="18" height="18" rx="2" ry="2" />
            <line x1="15" y1="3" x2="15" y2="21" />
          </svg>
        </button>
      </div>

      {/* 面包屑与快捷操作栏 (仅在 code 模式且有激活文件时展示) */}
      {mainView === "code" && filePath && (
        <div className="py-1 px-3 flex items-center justify-between border-b border-[var(--line)] bg-[var(--surface)] text-[11px] font-mono shrink-0 select-none">
          <div className="flex items-center gap-1 min-w-0 overflow-hidden text-[var(--ink-2)]">
            {activeProject && (
              <>
                <span className="text-[var(--ink)] font-medium truncate">{activeProject.name}</span>
                <span>/</span>
              </>
            )}
            {breadcrumbs.map((crumb, idx) => {
              const isLast = idx === breadcrumbs.length - 1;
              return (
                <span
                  key={idx}
                  className={`truncate ${isLast ? "text-[var(--ink)] font-medium" : "text-[var(--ink-2)]"}`}
                >
                  {crumb}
                  {!isLast && <span className="mx-1 text-[var(--ink-3)]">/</span>}
                </span>
              );
            })}
          </div>

          <div className="flex items-center gap-1 shrink-0 ml-2">
            {/* 编辑模式切换按钮 (纯图标，仅在文本文件时可用) */}
            {!activeTab?.isBinary && (
              <button
                type="button"
                onClick={() => setIsEditing(!isEditing)}
                className={`p-1.5 rounded-lg transition-colors cursor-pointer flex items-center justify-center ${
                  isEditing
                    ? "bg-[var(--celadon)] text-white shadow-2xs"
                    : "text-[var(--ink-2)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)]/60"
                }`}
                title={isEditing ? "退出编辑模式 (Exit Edit)" : "编辑当前文件 (Edit File)"}
                aria-label={isEditing ? "退出编辑模式" : "编辑当前文件"}
              >
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                  <path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7" />
                  <path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z" />
                </svg>
              </button>
            )}

            {/* 保存按钮 (Cmd+S，纯图标，仅在文本文件时可用) */}
            {!activeTab?.isBinary && (
              <button
                type="button"
                onClick={handleSaveFile}
                disabled={isSaving || !activeTab?.isDirty}
                className={`p-1.5 rounded-lg transition-all cursor-pointer flex items-center justify-center ${
                  saveSuccessNotice
                    ? "bg-emerald-500/15 text-emerald-600 dark:text-emerald-400 border border-emerald-500/30"
                    : activeTab?.isDirty
                    ? "bg-[var(--surface-3)] text-[var(--ink)] hover:bg-[var(--surface-2)] border border-[var(--line)] shadow-2xs font-semibold"
                    : "text-[var(--ink-3)] opacity-60 cursor-default"
                }`}
                title={saveSuccessNotice ? "已保存" : "保存文件 (Cmd+S / Ctrl+S)"}
                aria-label="保存文件"
              >
                {saveSuccessNotice ? (
                  <CheckIcon className="w-3.5 h-3.5 text-emerald-500" />
                ) : (
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                    <path d="M19 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11l5 5v11a2 2 0 0 1-2 2z" />
                    <polyline points="17 21 17 13 7 13 7 21" />
                    <polyline points="7 3 7 8 15 8" />
                  </svg>
                )}
              </button>
            )}

            {/* 提交 (Commit) 按钮 (纯图标) */}
            <button
              type="button"
              onClick={() => {
                setCommitFeedback(null);
                setIsCommitModalOpen(true);
              }}
              className="p-1.5 rounded-lg transition-colors cursor-pointer flex items-center justify-center text-[var(--ink-2)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)]/60"
              title="提交更改到 Git 仓库 (Git Commit)"
              aria-label="提交更改"
            >
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <circle cx="12" cy="12" r="4" />
                <line x1="1.05" y1="12" x2="7" y2="12" />
                <line x1="17.01" y1="12" x2="22.96" y2="12" />
              </svg>
            </button>

            {!activeTab?.isBinary && (
              <>
                <div className="w-[1px] h-3.5 bg-[var(--line)] mx-0.5" />

                {/* 自动换行切换按钮 */}
                <button
                  type="button"
                  onClick={() => setWrapLines(!wrapLines)}
                  className={`p-1.5 rounded-lg transition-colors cursor-pointer flex items-center justify-center ${
                    wrapLines
                      ? "bg-[var(--surface-3)] text-[var(--celadon)] shadow-2xs font-semibold"
                      : "text-[var(--ink-2)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)]/60"
                  }`}
                  title={wrapLines ? "关闭自动换行 (Disable Word Wrap)" : "开启自动换行 (Enable Word Wrap)"}
                >
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                    <line x1="3" y1="6" x2="21" y2="6" />
                    <path d="M3 12h15a3 3 0 1 1 0 6h-4" />
                    <polyline points="16 16 14 18 16 20" />
                  </svg>
                </button>
              </>
            )}

            {diffData && (
              <button
                type="button"
                onClick={() => setMode(mode === "diff" ? "code" : "diff")}
                className={`p-1.5 rounded-lg transition-colors cursor-pointer flex items-center justify-center ${
                  mode === "diff"
                    ? "bg-[var(--surface-3)] text-[var(--celadon)] shadow-2xs font-semibold"
                    : "text-[var(--ink-2)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)]/60"
                }`}
                title={mode === "diff" ? "查看原始源码 (View Code)" : "查看修改对比 (View Diff)"}
              >
                {/* Antigravity 差异对比/层叠文件修改图标 */}
                <svg
                  width="14"
                  height="14"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
                  <polyline points="14 2 14 8 20 8" />
                  <line x1="9" y1="13" x2="15" y2="13" />
                  <line x1="12" y1="10" x2="12" y2="16" />
                </svg>
              </button>
            )}

            <div className="relative" ref={menuRef}>
              <button
                type="button"
                onClick={() => setMenuOpen(!menuOpen)}
                className="p-1 rounded text-[var(--ink-2)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)] transition-colors cursor-pointer"
                title="更多选项"
              >
                <MoreVerticalIcon className="w-3.5 h-3.5" />
              </button>

              {menuOpen && (
                <div className="absolute right-0 top-full mt-1 w-44 bg-[var(--paper)] border border-[var(--line)] rounded-xl shadow-xl py-1 z-30 animate-in fade-in zoom-in-95 duration-100 text-[var(--ink)]">
                  <button
                    type="button"
                    onClick={handleCopyContent}
                    className="w-full px-3 py-1.5 text-left text-[11.5px] text-[var(--ink-2)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)] flex items-center gap-2.5 transition-colors cursor-pointer"
                  >
                    {copiedContent ? <CheckIcon className="w-3.5 h-3.5 text-emerald-500" /> : <CopyIcon className="w-3.5 h-3.5 text-[var(--ink-3)]" />}
                    <span>{copiedContent ? "已复制内容" : "Copy Content"}</span>
                  </button>

                  <button
                    type="button"
                    onClick={handleCopyPath}
                    className="w-full px-3 py-1.5 text-left text-[11.5px] text-[var(--ink-2)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)] flex items-center gap-2.5 transition-colors cursor-pointer"
                  >
                    {copiedPath ? <CheckIcon className="w-3.5 h-3.5 text-emerald-500" /> : <FinderIcon className="w-3.5 h-3.5 text-[var(--ink-3)]" />}
                    <span>{copiedPath ? "已复制路径" : "Copy Path"}</span>
                  </button>

                  <div className="my-1 border-t border-[var(--line)]" />

                  <button
                    type="button"
                    onClick={handleRevealInFinder}
                    className="w-full px-3 py-1.5 text-left text-[11.5px] text-[var(--ink-2)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)] flex items-center gap-2.5 transition-colors cursor-pointer"
                  >
                    <FinderIcon className="w-3.5 h-3.5" />
                    <span>Reveal in Finder</span>
                  </button>
                </div>
              )}
            </div>
          </div>
        </div>
      )}

      {/* 1. OVERVIEW 概览视图（对标 Antigravity 右侧原生折叠风格） */}
      {mainView === "overview" && (
        <div className="flex-1 overflow-y-auto p-4 flex flex-col gap-5 font-sans text-[12.5px] bg-[var(--surface)]">
          {/* Section 1: Subagents 折叠区 */}
          <div className="flex flex-col gap-1.5">
            <button
              type="button"
              onClick={() => toggleSection("subagents")}
              className="flex items-center justify-between text-left cursor-pointer group py-0.5 select-none"
            >
              <div className="flex items-center gap-2">
                <span className="text-[12px] font-bold text-[var(--ink)] tracking-tight">Subagents</span>
                <span className="text-[11px] font-mono text-[var(--ink-3)]">
                  {subagentsSummary.length}
                </span>
              </div>
              <svg
                width="12"
                height="12"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2.5"
                className={`text-[var(--ink-3)] group-hover:text-[var(--ink)] transition-transform duration-150 ${
                  sectionCollapsed.subagents ? "-rotate-90" : "rotate-0"
                }`}
              >
                <path d="M6 9l6 6 6-6" />
              </svg>
            </button>

            {!sectionCollapsed.subagents && (
              <div className="flex flex-col gap-1 mt-0.5">
                {subagentsSummary.length === 0 ? (
                  <div className="text-[11.5px] text-[var(--ink-3)] py-1 pl-1">{t("viewer.subagentsEmpty", { defaultValue: "暂无子智能体运行记录" })}</div>
                ) : (
                  subagentsSummary.map((sub, i) => (
                    <div
                      key={sub.id || i}
                      className="flex items-center justify-between py-1.5 px-2 rounded-lg hover:bg-[var(--surface-2)] transition-colors cursor-pointer group"
                    >
                      <div className="flex items-center gap-2 truncate min-w-0">
                        <span
                          className={`w-2 h-2 rounded-full shrink-0 ${
                            sub.status === "running"
                              ? "bg-amber-400 animate-pulse"
                              : sub.mode === "isolated"
                              ? "bg-purple-400"
                              : "bg-emerald-400"
                          }`}
                        />
                        <div className="flex flex-col min-w-0 truncate">
                          <span className="text-[12.5px] font-medium text-[var(--ink)] truncate">
                            {sub.role}
                          </span>
                          {sub.goal && (
                            <span className="text-[11px] text-[var(--ink-3)] truncate font-mono">
                              {sub.goal}
                            </span>
                          )}
                        </div>
                      </div>
                      <span className="text-[10.5px] font-mono text-[var(--ink-3)] shrink-0 ml-2">
                        {sub.status === "running" ? "Running" : "Killed"}
                      </span>
                    </div>
                  ))
                )}
              </div>
            )}
          </div>

          {/* Section 2: Files Changed 折叠区 */}
          <div className="flex flex-col gap-1.5">
            <button
              type="button"
              onClick={() => toggleSection("files")}
              className="flex items-center justify-between text-left cursor-pointer group py-0.5 select-none"
            >
              <div className="flex items-center gap-2">
                <span className="text-[12px] font-bold text-[var(--ink)] tracking-tight">Files Changed</span>
                <span className="text-[11px] font-mono text-[var(--ink-3)]">
                  {modifiedFilesSummary.length}
                </span>
              </div>
              <svg
                width="12"
                height="12"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2.5"
                className={`text-[var(--ink-3)] group-hover:text-[var(--ink)] transition-transform duration-150 ${
                  sectionCollapsed.files ? "-rotate-90" : "rotate-0"
                }`}
              >
                <path d="M6 9l6 6 6-6" />
              </svg>
            </button>

            {!sectionCollapsed.files && (
              <div className="flex flex-col gap-0.5 mt-0.5">
                {modifiedFilesSummary.length === 0 ? (
                  <div className="text-[11.5px] text-[var(--ink-3)] py-1 pl-1">{t("viewer.filesChangedEmpty", { defaultValue: "暂无文件修改" })}</div>
                ) : (
                  modifiedFilesSummary.map((f, i) => {
                    const fExt = f.path.split(".").pop()?.toLowerCase() || "";
                    const fBase = f.path.split("/").pop() || f.path;
                    return (
                      <div
                        key={i}
                        onClick={() => {
                          if (f.diffData) {
                            void useCodeViewer.getState().openFile(f.path, {
                              diffData: f.diffData,
                              mode: "diff",
                            });
                          } else {
                            void useCodeViewer.getState().openFile(f.path);
                          }
                          setMainView("code");
                        }}
                        className="flex items-center justify-between py-1.5 px-2 rounded-lg hover:bg-[var(--surface-2)] text-[var(--ink)] hover:text-[var(--celadon)] transition-colors cursor-pointer group"
                      >
                        <div className="flex items-center gap-2 truncate min-w-0">
                          <FileIcon ext={fExt} className="w-3.5 h-3.5 shrink-0" />
                          <span className="font-mono text-[12px] truncate">{fBase}</span>
                        </div>
                        <div className="flex items-center gap-1.5 shrink-0 ml-2 font-mono text-[10.5px]">
                          {f.additions > 0 && <span className="text-emerald-500 font-bold">+{f.additions}</span>}
                          {f.deletions > 0 && <span className="text-rose-500 font-bold">-{f.deletions}</span>}
                          <span className="text-[11px] text-[var(--ink-3)] group-hover:text-[var(--celadon)] transition-colors">
                            &rarr;
                          </span>
                        </div>
                      </div>
                    );
                  })
                )}
              </div>
            )}
          </div>

          {/* Section 3: Artifacts 交付文档区 (对标 Antigravity) */}
          <div className="flex flex-col gap-1.5">
            <button
              type="button"
              onClick={() => toggleSection("artifacts")}
              className="flex items-center justify-between text-left cursor-pointer group py-0.5 select-none"
            >
              <div className="flex items-center gap-2">
                <span className="text-[12px] font-bold text-[var(--ink)] tracking-tight">
                  {t("viewer.artifactsTitle", { defaultValue: "Artifacts" })}
                </span>
                <span className="text-[11px] font-mono text-[var(--ink-3)]">
                  {artifactsSummary.length}
                </span>
              </div>
              <svg
                width="12"
                height="12"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2.5"
                className={`text-[var(--ink-3)] group-hover:text-[var(--ink)] transition-transform duration-150 ${
                  sectionCollapsed.artifacts ? "-rotate-90" : "rotate-0"
                }`}
              >
                <path d="M6 9l6 6 6-6" />
              </svg>
            </button>

            {!sectionCollapsed.artifacts && (
              <div className="flex flex-col gap-0.5 mt-0.5">
                {artifactsSummary.length === 0 ? (
                  <div className="text-[11.5px] text-[var(--ink-3)] py-1 pl-1">
                    {t("viewer.artifactsEmpty", { defaultValue: "暂无生成的交付文档或成果" })}
                  </div>
                ) : (
                  artifactsSummary.map((art, i) => (
                    <div
                      key={art.id || i}
                      onClick={() => {
                        // 图片/视频：滚到对话里那张卡片。代码查看器只能显示代码和
                        // diff，PNG/MP4 进去是乱码。
                        if (art.turnId) {
                          document
                            .querySelector(`[data-turn-id="${art.turnId}"]`)
                            ?.scrollIntoView({ behavior: "smooth", block: "center" });
                          return;
                        }
                        if (art.path) {
                          void useCodeViewer.getState().openFile(art.path);
                          setMainView("code");
                        }
                      }}
                      className="flex items-center gap-2 py-1 px-2 rounded-lg hover:bg-[var(--surface-2)] text-[var(--ink)] transition-colors cursor-pointer group"
                    >
                      <svg
                        width="13"
                        height="13"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        strokeWidth="1.8"
                        className="text-[var(--ink-3)] shrink-0"
                      >
                        <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
                        <polyline points="14 2 14 8 20 8" />
                        <line x1="16" y1="13" x2="8" y2="13" />
                        <line x1="16" y1="17" x2="8" y2="17" />
                        <polyline points="10 9 9 9 8 9" />
                      </svg>
                      <span className="text-[12px] truncate font-medium">{art.name}</span>
                      <button
                        type="button"
                        title="从列表移除（不删除文件）"
                        onClick={(e) => {
                          e.stopPropagation();
                          setDismissedArtifacts((prev) => new Set(prev).add(art.id || String(i)));
                        }}
                        className="ml-auto shrink-0 opacity-0 group-hover:opacity-100 text-[var(--ink-3)] hover:text-[var(--clay)] transition-opacity"
                      >
                        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5">
                          <path d="M18 6L6 18M6 6l12 12" />
                        </svg>
                      </button>
                    </div>
                  ))
                )}
              </div>
            )}
          </div>

          {/* Section 4: Uploads 上传资源区 (对标 Antigravity) */}
          <div className="flex flex-col gap-1.5">
            <button
              type="button"
              onClick={() => toggleSection("uploads")}
              className="flex items-center justify-between text-left cursor-pointer group py-0.5 select-none"
            >
              <div className="flex items-center gap-2">
                <span className="text-[12px] font-bold text-[var(--ink)] tracking-tight">
                  {t("viewer.uploadsTitle", { defaultValue: "Uploads" })}
                </span>
                <span className="text-[11px] font-mono text-[var(--ink-3)]">
                  {uploadsSummary.length}
                </span>
              </div>
              <svg
                width="12"
                height="12"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2.5"
                className={`text-[var(--ink-3)] group-hover:text-[var(--ink)] transition-transform duration-150 ${
                  sectionCollapsed.uploads ? "-rotate-90" : "rotate-0"
                }`}
              >
                <path d="M6 9l6 6 6-6" />
              </svg>
            </button>

            {!sectionCollapsed.uploads && (
              <div className="flex flex-col gap-1 mt-0.5">
                {uploadsSummary.length === 0 ? (
                  <div className="text-[11.5px] text-[var(--ink-3)] py-1 pl-1">
                    {t("viewer.uploadsEmpty", { defaultValue: "暂无上传图片或附件" })}
                  </div>
                ) : (
                  uploadsSummary.map((u, i) => (
                    <div
                      key={u.id || i}
                      onClick={() => {
                        if (u.url) {
                          void useCodeViewer.getState().openFile(u.url);
                          setMainView("code");
                        }
                      }}
                      className="flex items-center gap-2 py-1 px-2 rounded-lg hover:bg-[var(--surface-2)] text-[var(--ink)] transition-colors cursor-pointer"
                    >
                      <svg
                        width="13"
                        height="13"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        strokeWidth="1.8"
                        className="text-[var(--ink-3)] shrink-0"
                      >
                        <rect width="18" height="18" x="3" y="3" rx="2" ry="2" />
                        <circle cx="9" cy="9" r="2" />
                        <path d="m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21" />
                      </svg>
                      <span className="text-[12px] truncate font-mono">{u.name}</span>
                    </div>
                  ))
                )}
              </div>
            )}
          </div>

          {/* Section 5: Background Tasks 后台与 MCP 任务区 (对标 Antigravity) */}
          <div className="flex flex-col gap-1.5">
            <button
              type="button"
              onClick={() => toggleSection("backgroundTasks")}
              className="flex items-center justify-between text-left cursor-pointer group py-0.5 select-none"
            >
              <div className="flex items-center gap-2">
                <span className="text-[12px] font-bold text-[var(--ink)] tracking-tight">
                  {t("viewer.backgroundTasksTitle", { defaultValue: "Background Tasks" })}
                </span>
                <span className="text-[11px] font-mono text-[var(--ink-3)]">
                  {backgroundTasksSummary.length}
                </span>
              </div>
              <svg
                width="12"
                height="12"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2.5"
                className={`text-[var(--ink-3)] group-hover:text-[var(--ink)] transition-transform duration-150 ${
                  sectionCollapsed.backgroundTasks ? "-rotate-90" : "rotate-0"
                }`}
              >
                <path d="M6 9l6 6 6-6" />
              </svg>
            </button>

            {!sectionCollapsed.backgroundTasks && (
              <div className="flex flex-col gap-1 mt-0.5">
                {backgroundTasksSummary.length === 0 ? (
                  <div className="text-[11.5px] text-[var(--ink-3)] py-1 pl-1">
                    {t("viewer.backgroundTasksEmpty", {
                      defaultValue: "暂无后台任务或定时调度",
                    })}
                  </div>
                ) : (
                  backgroundTasksSummary.map((task, i) => (
                    <div
                      key={task.id || i}
                      className="flex items-center justify-between py-1 px-2 rounded-lg hover:bg-[var(--surface-2)] text-[var(--ink)] transition-colors cursor-pointer"
                    >
                      <div className="flex items-center gap-2 truncate min-w-0">
                        <span
                          className={`w-2 h-2 rounded-full shrink-0 ${
                            task.status === "running"
                              ? "bg-amber-400 animate-pulse"
                              : task.status === "failed"
                              ? "bg-red-400"
                              : "bg-emerald-400"
                          }`}
                        />
                        <span className="text-[12px] font-mono truncate">{task.name}</span>
                      </div>
                      {task.detail && (
                        <span className="text-[10px] font-mono text-[var(--ink-3)] bg-[var(--surface-3)] px-1.5 py-0.5 rounded shrink-0 ml-1">
                          {task.detail}
                        </span>
                      )}
                    </div>
                  ))
                )}
              </div>
            )}
          </div>

          {/* Section 6: Terminals 终端进程区 (对标 Antigravity) */}
          <div className="flex flex-col gap-1.5">
            <button
              type="button"
              onClick={() => toggleSection("terminals")}
              className="flex items-center justify-between text-left cursor-pointer group py-0.5 select-none"
            >
              <div className="flex items-center gap-2">
                <span className="text-[12px] font-bold text-[var(--ink)] tracking-tight">
                  {t("viewer.terminalsTitle", { defaultValue: "Terminals" })}
                </span>
                <span className="text-[11px] font-mono text-[var(--ink-3)]">
                  {terminalsSummary.length}
                </span>
              </div>
              <svg
                width="12"
                height="12"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2.5"
                className={`text-[var(--ink-3)] group-hover:text-[var(--ink)] transition-transform duration-150 ${
                  sectionCollapsed.terminals ? "-rotate-90" : "rotate-0"
                }`}
              >
                <path d="M6 9l6 6 6-6" />
              </svg>
            </button>

            {!sectionCollapsed.terminals && (
              <div className="flex flex-col gap-1 mt-0.5">
                {terminalsSummary.length === 0 ? (
                  <div className="text-[11.5px] text-[var(--ink-3)] py-1 pl-1">
                    {t("viewer.terminalsEmpty", { defaultValue: "暂无运行中的终端进程" })}
                  </div>
                ) : (
                  terminalsSummary.map((term, i) => (
                    <div
                      key={term.id || i}
                      className="flex items-center justify-between py-1 px-2 rounded-lg hover:bg-[var(--surface-2)] text-[var(--ink)] transition-colors cursor-pointer"
                    >
                      <div className="flex items-center gap-2 truncate min-w-0">
                        <svg
                          width="13"
                          height="13"
                          viewBox="0 0 24 24"
                          fill="none"
                          stroke="currentColor"
                          strokeWidth="2"
                          className="text-[var(--ink-3)] shrink-0"
                        >
                          <polyline points="4 17 10 11 4 5" />
                          <line x1="12" y1="19" x2="20" y2="19" />
                        </svg>
                        <span className="text-[12px] font-mono truncate">{term.command}</span>
                      </div>
                      {term.pid && (
                        <span className="text-[10.5px] font-mono text-[var(--ink-3)] shrink-0 ml-1">
                          PID {term.pid}
                        </span>
                      )}
                    </div>
                  ))
                )}
              </div>
            )}
          </div>

          {/* Section 7: Skills Used 技能载入区 (对标 Antigravity) */}
          <div className="flex flex-col gap-1.5">
            <button
              type="button"
              onClick={() => toggleSection("skillsUsed")}
              className="flex items-center justify-between text-left cursor-pointer group py-0.5 select-none"
            >
              <div className="flex items-center gap-2">
                <span className="text-[12px] font-bold text-[var(--ink)] tracking-tight">
                  {t("viewer.skillsUsedTitle", { defaultValue: "Skills Used" })}
                </span>
                {/* **数的是生效几个，不是认出几个。** 这一格回答的是
                    「这次会话里什么在起作用」。 */}
                <span className="text-[11px] font-mono text-[var(--ink-3)]">
                  {activeCount(skillRowsState)}
                  {skillRowsState.length > activeCount(skillRowsState) && (
                    <span className="opacity-50"> / {skillRowsState.length}</span>
                  )}
                </span>
              </div>
              <svg
                width="12"
                height="12"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2.5"
                className={`text-[var(--ink-3)] group-hover:text-[var(--ink)] transition-transform duration-150 ${
                  sectionCollapsed.skillsUsed ? "-rotate-90" : "rotate-0"
                }`}
              >
                <path d="M6 9l6 6 6-6" />
              </svg>
            </button>

            {!sectionCollapsed.skillsUsed && (
              <div className="flex flex-col gap-0.5 mt-0.5">
                {activeCount(skillRowsState) === 0 ? (
                  /* **两种空态说的不是同一件事**：一个都没认出来 vs
                     认出来了但没启用。以前共用一句「当前项目未调用技能」，
                     把第二种说成了第一种 —— 用户会去查加载，
                     而其实只要去勾一下。 */
                  <div className="text-[11.5px] text-[var(--ink-3)] py-1 pl-1 leading-relaxed">
                    {emptyHint(skillRowsState)}
                  </div>
                ) : (
                  skillRowsState
                    .filter((sk) => sk.active)
                    .map((sk) => (
                      <div
                        key={sk.id}
                        title={
                          sk.toolCount === null
                            ? "生效中 · 不限制工具"
                            : `生效中 · 工具面收窄到 ${sk.toolCount} 个`
                        }
                        className="flex items-center justify-between py-1 px-2 rounded-lg hover:bg-[var(--surface-2)] text-[var(--ink)] transition-colors"
                      >
                        <div className="flex items-center gap-2 truncate min-w-0">
                          <svg
                            width="13"
                            height="13"
                            viewBox="0 0 24 24"
                            fill="none"
                            stroke="currentColor"
                            strokeWidth="1.8"
                            className="text-[var(--ink-3)] shrink-0"
                          >
                            <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
                            <polyline points="14 2 14 8 20 8" />
                          </svg>
                          <span className="text-[12px] font-medium truncate">{sk.name}</span>
                        </div>
                        {/* **空 tools 是「不限制」，不是「0 个工具」** ——
                            后者会让人以为这个技能把模型废了。 */}
                        <span className="text-[10px] font-mono text-[var(--ink-3)] truncate ml-1 opacity-70">
                          {sk.scope === "project" ? "项目" : "全局"}
                          {sk.toolCount !== null && ` · ${sk.toolCount} 工具`}
                        </span>
                      </div>
                    ))
                )}
              </div>
            )}
          </div>
        </div>
      )}

      {/* 2. FILES 目录树浏览器（支持多级折叠与快速过滤） */}
      {mainView === "files" && (
        <div className="flex-1 overflow-hidden flex flex-col bg-[var(--surface)] text-[var(--ink)]">
          {/* 搜索框 */}
          <div className="p-2.5 border-b border-[var(--line)] bg-[var(--surface-2)]">
            <div className="flex items-center gap-2 bg-[var(--paper)] border border-[var(--line)] rounded-lg px-2.5 py-1 text-[12px]">
              <SearchIcon className="w-3.5 h-3.5 text-[var(--ink-3)]" />
              <input
                type="text"
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                placeholder="搜索文件路径或名称..."
                className="bg-transparent border-none outline-none text-[var(--ink)] placeholder-[var(--ink-3)] w-full text-[12px]"
              />
              {searchQuery && (
                <button
                  type="button"
                  onClick={() => setSearchQuery("")}
                  className="text-[var(--ink-3)] hover:text-[var(--ink)] text-[11px]"
                >
                  ✕
                </button>
              )}
            </div>
          </div>

          {/* 文件树列表 */}
          <div className="flex-1 overflow-y-auto p-2">
            {treeLoading ? (
              <div className="flex flex-col items-center justify-center h-48 gap-2 text-[var(--ink-3)]">
                <span className="animate-spin text-[16px]">⟳</span>
                <span className="text-[12px] font-sans">正在加载目录结构...</span>
              </div>
            ) : fileTree.length === 0 ? (
              <div className="p-6 text-center text-[12px] text-[var(--ink-3)]">
                未检测到项目文件，或尚未绑定工作区目录
              </div>
            ) : (
              <div className="flex flex-col gap-0.5">
                {renderTreeNodes(fileTree)}
              </div>
            )}
          </div>
        </div>
      )}

      {/* 3. CODE 源码/Diff展示区 */}
      {mainView === "code" && (
        <div className="flex-1 overflow-y-auto overflow-x-hidden p-0 relative font-mono text-[12px] leading-relaxed bg-[var(--surface)] text-[var(--ink)]">
          {loading && (
            <div className="flex flex-col items-center justify-center h-full gap-2 text-[var(--ink-3)]">
              <span className="animate-spin text-[18px]">⟳</span>
              <span className="text-[12px] font-sans">正在解析文件内容...</span>
            </div>
          )}

          {error && (
            <div className="p-4 m-4 rounded-xl bg-red-950/40 border border-red-800/60 text-red-300 text-[12px] font-sans">
              <div className="font-bold mb-1">无法打开文件</div>
              <div className="text-[11px] opacity-90 font-mono break-all">{error}</div>
            </div>
          )}

          {!loading && !error && mode === "diff" && diffData && (
            <div className="w-full overflow-x-hidden">
              <DiffViewer diff={diffData} initialExpanded={true} fullBleed={true} />
            </div>
          )}

          {/* 媒体与二进制文件预览 (图片 / 音频 / 视频 / 二进制) */}
          {!loading && !error && activeTab?.isBinary && (
            <div className="flex flex-col items-center justify-center min-h-full p-8 text-center bg-[var(--surface)] text-[var(--ink)]">
              {activeTab.mediaType === "image" && activeTab.dataUrl ? (
                <div className="flex flex-col items-center gap-4 max-w-full">
                  <div className="p-3 bg-[var(--paper)] rounded-2xl border border-[var(--line)] shadow-xl overflow-hidden max-w-full flex items-center justify-center">
                    <img
                      src={activeTab.dataUrl}
                      alt={activeTab.title}
                      className="max-h-[60vh] max-w-full object-contain rounded-lg select-none"
                    />
                  </div>
                  <div className="flex items-center gap-3 text-[11px] font-mono text-[var(--ink-2)] bg-[var(--surface-2)] px-3 py-1.5 rounded-xl border border-[var(--line)]">
                    <span className="font-semibold text-[var(--ink)]">{activeTab.title}</span>
                    <span>•</span>
                    <span>{ext.toUpperCase()} 图像预览</span>
                  </div>
                </div>
              ) : activeTab.mediaType === "audio" && activeTab.dataUrl ? (
                <div className="flex flex-col items-center gap-4 p-6 bg-[var(--paper)] rounded-2xl border border-[var(--line)] shadow-lg max-w-md w-full">
                  <div className="w-12 h-12 rounded-full bg-[var(--celadon)]/15 text-[var(--celadon)] flex items-center justify-center">
                    <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                      <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5" />
                      <path d="M15.54 8.46a5 5 0 0 1 0 7.07" />
                      <path d="M19.07 4.93a10 10 0 0 1 0 14.14" />
                    </svg>
                  </div>
                  <span className="font-semibold text-[13px]">{activeTab.title}</span>
                  <audio controls src={activeTab.dataUrl} className="w-full mt-2" />
                </div>
              ) : activeTab.mediaType === "video" && activeTab.dataUrl ? (
                <div className="flex flex-col items-center gap-4 max-w-full">
                  <div className="p-2 bg-[var(--paper)] rounded-2xl border border-[var(--line)] shadow-xl overflow-hidden max-w-2xl w-full">
                    <video controls src={activeTab.dataUrl} className="w-full max-h-[60vh] rounded-lg" />
                  </div>
                  <span className="text-[11.5px] font-mono text-[var(--ink-2)]">{activeTab.title}</span>
                </div>
              ) : (
                <div className="flex flex-col items-center gap-3 p-8 bg-[var(--paper)] rounded-2xl border border-[var(--line)] max-w-sm">
                  <FileIcon ext={ext} className="w-12 h-12 text-[var(--ink-3)]" />
                  <span className="font-semibold text-[13px]">{activeTab?.title || baseName}</span>
                  <span className="text-[11px] text-[var(--ink-2)]">{activeTab?.content || "二进制文件，暂不支持直接文本预览"}</span>
                  <button
                    type="button"
                    onClick={handleRevealInFinder}
                    className="mt-2 px-3 py-1.5 rounded-xl text-[11.5px] bg-[var(--surface-2)] hover:bg-[var(--surface-3)] text-[var(--ink)] border border-[var(--line)] transition-colors cursor-pointer"
                  >
                    在 Finder 中查看
                  </button>
                </div>
              )}
            </div>
          )}

          {/* 可编辑模式 (isEditing = true 且非二进制) */}
          {!loading && !error && isEditing && !activeTab?.isBinary && (
            <div className="flex h-full w-full bg-[var(--paper)]">
              {/* 行号栏 */}
              <div className="w-14 px-2 py-3 text-right text-[11.5px] select-none border-r border-[var(--line)] font-mono text-[var(--ink-3)] bg-[var(--surface-2)]/50 shrink-0 flex flex-col gap-0 leading-6">
                {lines.map((_, i) => (
                  <span key={i} className="h-6">{i + 1}</span>
                ))}
              </div>
              {/* 多行编辑器文本域 */}
              <textarea
                value={currentContent}
                onChange={(e) => {
                  if (activeTabId) {
                    updateTabContent(activeTabId, e.target.value);
                  }
                }}
                onKeyDown={(e) => {
                  // Tab 键智能缩进支持 (2 spaces)
                  if (e.key === "Tab") {
                    e.preventDefault();
                    const target = e.currentTarget;
                    const start = target.selectionStart;
                    const end = target.selectionEnd;
                    const val = target.value;
                    const nextVal = val.substring(0, start) + "  " + val.substring(end);
                    if (activeTabId) {
                      updateTabContent(activeTabId, nextVal);
                    }
                    setTimeout(() => {
                      target.selectionStart = target.selectionEnd = start + 2;
                    }, 0);
                  }
                }}
                placeholder="在此直接编辑代码或文档内容，按 Cmd+S / Ctrl+S 快速保存..."
                className={`flex-1 h-full p-3 font-mono text-[12px] leading-6 bg-transparent text-[var(--ink)] border-none outline-none resize-none overflow-y-auto ${
                  wrapLines ? "whitespace-pre-wrap break-all" : "whitespace-pre"
                }`}
                autoFocus
                spellCheck={false}
              />
            </div>
          )}

          {/* 只读源码渲染 (isEditing = false 且非二进制) */}
          {!loading && !error && !isEditing && !activeTab?.isBinary && (mode === "code" || !diffData) && (
            <table className="w-full table-fixed border-collapse">
              <tbody>
                {(() => {
                  const renderedRows: React.ReactNode[] = [];
                  let skipUntilLine = 0;

                  for (let idx = 0; idx < lines.length; idx++) {
                    const lineNum = idx + 1;
                    const line = lines[idx];

                    // 如果当前行处于被折叠的代码块内部，跳过渲染
                    if (lineNum <= skipUntilLine) {
                      continue;
                    }

                    const isHighlighted =
                      lineRange && lineNum >= lineRange[0] && lineNum <= lineRange[1];

                    const isFoldStart = foldableBlocks.has(lineNum);
                    const foldEndLine = foldableBlocks.get(lineNum) || 0;
                    const isCollapsed = Boolean(collapsedBlocks[lineNum]);

                    if (isCollapsed) {
                      skipUntilLine = foldEndLine;
                    }

                    renderedRows.push(
                      <tr
                        key={idx}
                        id={`code-line-${lineNum}`}
                        className={`transition-colors ${
                          isHighlighted
                            ? "bg-[var(--celadon)]/15 hover:bg-[var(--celadon)]/25 border-l-2 border-l-[var(--celadon)]"
                            : "hover:bg-[var(--surface-2)]/60 border-l-2 border-l-transparent"
                        }`}
                      >
                        {/* 行号与折叠三角列 */}
                        <td
                          className={`w-14 px-2 py-0.5 text-right text-[11px] select-none border-r border-[var(--line)] font-mono align-top ${
                            isHighlighted ? "text-[var(--celadon)] font-bold" : "text-[var(--ink-3)]"
                          }`}
                        >
                          <div className="flex items-center justify-end gap-1">
                            {isFoldStart && (
                              <button
                                type="button"
                                onClick={() => toggleFold(lineNum, foldEndLine)}
                                className="w-3.5 h-3.5 flex items-center justify-center text-[var(--ink-3)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)] rounded transition-colors cursor-pointer"
                                title={isCollapsed ? `展开 ${lineNum}-${foldEndLine} 行` : `折叠 ${lineNum}-${foldEndLine} 行`}
                              >
                                <svg
                                  width="9"
                                  height="9"
                                  viewBox="0 0 24 24"
                                  fill="none"
                                  stroke="currentColor"
                                  strokeWidth="3"
                                  className={`transition-transform duration-150 ${isCollapsed ? "-rotate-90" : "rotate-0"}`}
                                >
                                  <path d="M6 9l6 6 6-6" />
                                </svg>
                              </button>
                            )}
                            <span className="min-w-[20px]">{lineNum}</span>
                          </div>
                        </td>

                        {/* 代码高亮语法列（支持开启/关闭自动换行） */}
                        <td
                          className={`px-3.5 py-0.5 font-mono text-[var(--ink)] select-text align-top ${
                            wrapLines
                              ? "whitespace-pre-wrap break-all"
                              : "whitespace-pre"
                          }`}
                        >
                          {line ? (
                            <HighlightedCodeLine line={line} ext={ext} />
                          ) : (
                            <span>&nbsp;</span>
                          )}

                          {/* 折叠占位符 */}
                          {isCollapsed && (
                            <span
                              onClick={() => toggleFold(lineNum, foldEndLine)}
                              className="ml-2 inline-flex items-center px-1.5 py-0.2 bg-[var(--surface-3)] hover:bg-[var(--surface-2)] text-[var(--ink-2)] hover:text-[var(--ink)] text-[10px] rounded border border-[var(--line)] cursor-pointer select-none font-mono transition-colors"
                              title="点击展开代码块"
                            >
                              ⋯ {foldEndLine - lineNum} lines hidden ⋯
                            </span>
                          )}
                        </td>
                      </tr>
                    );
                  }

                  return renderedRows;
                })()}
              </tbody>
            </table>
          )}
        </div>
      )}

      {/* Git Commit 提交弹窗/抽屉 */}
      {isCommitModalOpen && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 backdrop-blur-xs p-4 animate-in fade-in duration-100">
          <div className="w-[420px] bg-[var(--paper)] border border-[var(--line)] rounded-2xl shadow-2xl p-4 flex flex-col gap-3.5 text-[var(--ink)] animate-in zoom-in-95 duration-150">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="text-[var(--celadon)]">
                  <circle cx="12" cy="12" r="4" />
                  <line x1="1.05" y1="12" x2="7" y2="12" />
                  <line x1="17.01" y1="12" x2="22.96" y2="12" />
                </svg>
                <span className="font-bold text-[13px]">提交代码变更 (Git Commit)</span>
              </div>
              <button
                type="button"
                onClick={() => setIsCommitModalOpen(false)}
                className="p-1 rounded-lg text-[var(--ink-3)] hover:text-[var(--ink)] hover:bg-[var(--surface-2)] transition-colors cursor-pointer"
              >
                <CloseIcon className="w-4 h-4" />
              </button>
            </div>

            <div className="text-[12px] text-[var(--ink-2)] flex flex-col gap-1">
              <span>提交范围：</span>
              <span className="font-mono text-[11px] bg-[var(--surface-2)] px-2 py-1 rounded border border-[var(--line)] truncate">
                {filePath ? filePath : "当前项目全部未暂存文件 (.)"}
              </span>
            </div>

            <div className="flex flex-col gap-1.5">
              <label className="text-[12px] font-medium text-[var(--ink)]">提交信息 (Commit Message):</label>
              <textarea
                value={commitMessage}
                onChange={(e) => setCommitMessage(e.target.value)}
                placeholder="例如: feat: update code reviewer workflow and prompt specs"
                rows={3}
                className="w-full p-2.5 rounded-xl border border-[var(--line)] bg-[var(--input-bg)] text-[12.5px] font-mono text-[var(--ink)] outline-none focus:border-[var(--celadon)] transition-colors resize-none"
                autoFocus
              />
            </div>

            {commitFeedback && (
              <div
                className={`p-2.5 rounded-xl text-[11.5px] font-mono ${
                  commitFeedback.ok
                    ? "bg-emerald-500/15 text-emerald-600 dark:text-emerald-400 border border-emerald-500/30"
                    : "bg-rose-500/15 text-rose-600 dark:text-rose-400 border border-rose-500/30"
                }`}
              >
                {commitFeedback.msg}
              </div>
            )}

            <div className="flex items-center justify-end gap-2 pt-1">
              <button
                type="button"
                onClick={() => setIsCommitModalOpen(false)}
                className="px-3 py-1.5 rounded-xl text-[12px] text-[var(--ink-2)] hover:bg-[var(--surface-2)] transition-colors cursor-pointer font-medium"
              >
                取消
              </button>
              <button
                type="button"
                onClick={handleExecuteCommit}
                disabled={!commitMessage.trim() || isCommitting}
                className="px-4 py-1.5 rounded-xl text-[12px] font-medium bg-[var(--celadon)] hover:brightness-110 text-white transition-all cursor-pointer shadow-xs disabled:opacity-50 flex items-center gap-1.5"
              >
                {isCommitting && <span className="animate-spin text-xs">⟳</span>}
                <span>{isCommitting ? "提交中..." : "确认提交"}</span>
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
