/**
 * 现代专业级 AI 指令栏 + 视觉多模态图像支持 + 工具执行状态抽屉
 * 对标 Claude Code / Antigravity / Cursor 的极简卡片式设计
 */
import { useState, useRef, useEffect, useCallback } from "react";
import { useTranslation } from "react-i18next";
import {
  listSlashCommands,
  expandSlashCommand,
  listSkills,
  selectSkills,
  type SlashEntry,
} from "../api/voice";
import {
  parseSkillCommand,
  renderSkillList,
  skillPickerEmptyHint,
  skillPickerEntries,
  skillPickerQuery,
  skillPickerTitle,
  validateTarget,
  type SkillLike,
} from "./skillCommand";
import { modelSupportsVision, useChat } from "../store/chat";
import { useVoice } from "../store/voice";
import { useCinemaTasks } from "../store/cinemaTasks";
import { useInteractiveChoice } from "../store/interactiveChoice";
import { useProject } from "../store/project";
import { useImageProgress, progressSlot } from "../store/imageProgress";
import { useVideoProgress, videoProgressSlot } from "../store/videoProgress";
import { useActiveMusic } from "../store/activeMusic";
import { convertFileSrc } from "@tauri-apps/api/core";
import * as api from "../api";
import {
  onRuntimeApproval,
  runtimeListPendingApprovals,
  runtimeResolveApproval,
  type RuntimeApprovalRequest,
  type RuntimeApprovalDecision,
  type RuntimeApprovalScope,
} from "../api";
import { ThinkingControl } from "./ThinkingControl";
import { EyeIcon } from "./Icons";
import { MicFill } from "./MicLevel";
import { downsampleImage } from "../utils/imageCompressor";
import { dropZoneAt, toLogical } from "./studio/nativeDrop";
import {
  completionFor,
  filterEntries,
  justOpened,
  moveIndex,
  parseSubmitted,
  slashQuery,
} from "./slashMenu";

export interface ChatAttachment {
  id: string;
  name: string;
  size: number;
  isImage: boolean;
  dataUrl?: string;
  localPath?: string;
  fileUrl?: string;
  saveError?: string;
  textContent?: string;
  extension: string;
}

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function getFileIcon(ext: string): string {
  const e = ext.toLowerCase();
  if (["rs", "ts", "tsx", "js", "jsx", "py", "go", "c", "cpp", "java", "sh"].includes(e)) return "💻";
  if (["json", "yaml", "yml", "toml", "xml", "env"].includes(e)) return "⚙️";
  if (["md", "markdown", "txt", "doc", "docx", "pdf"].includes(e)) return "📄";
  if (["csv", "tsv", "xlsx", "xls", "sql"].includes(e)) return "📊";
  return "📁";
}

export function CommandBar() {
  const { t, i18n } = useTranslation(["chat", "common"]);
  const [draft, setDraft] = useState("");
  const [attachments, setAttachments] = useState<ChatAttachment[]>([]);
  const [isDraggingOver, setIsDraggingOver] = useState(false);
  const [pendingApprovals, setPendingApprovals] = useState<RuntimeApprovalRequest[]>([]);
  const [approvalBusy, setApprovalBusy] = useState(false);
  const [isModelDropdownOpen, setIsModelDropdownOpen] = useState(false);
  const modelDropdownRef = useRef<HTMLDivElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);


  const send = useChat((s) => s.send);
  const cancel = useChat((s) => s.cancel);
  const activeStreamId = useChat((s) => s.activeStreamId);

  const config = useChat((s) => s.config);
  const models = useChat((s) => s.models);
  const tier = useChat((s) => s.tier);
  const loadModels = useChat((s) => s.loadModels);
  const refreshConfig = useChat((s) => s.refreshConfig);

  const activeProviderId = config?.realtime_provider || "lmstudio";
  const activeProvider = config?.providers.find((p) => p.id === activeProviderId);
  // 链路自己的模型覆盖后端的默认值（与后端 Gateway::resolve 严格对齐）
  const currentModelName = config?.realtime_model || activeProvider?.model || "默认模型";
  // 当前链路的模型能不能吃图。**undefined = 查不到**（只有 LM Studio 有那个
  // 接口），这时什么都不显示 —— 谎报"不支持视觉"比不说更糟。
  const visionSupported = modelSupportsVision(config, models, tier);
  const modelsState = models[activeProviderId];
  const availableModels = modelsState?.list || [];
  const isLoadingModels = modelsState?.status === "loading";

  // 交互式决策选项卡与快捷键支持
  const activeChoice = useInteractiveChoice((s) => s.activeChoice);
  const resolveChoice = useInteractiveChoice((s) => s.resolveChoice);
  const cancelChoice = useInteractiveChoice((s) => s.cancelChoice);

  // 活跃工作区 / 项目状态
  const projects = useProject((s) => s.projects);
  const activeProjectId = useProject((s) => s.activeProjectId);
  const toggleSidebar = useProject((s) => s.toggleSidebar);
  const activeProject = projects.find((p) => p.id === activeProjectId);
  const workspacePath = activeProject?.workspace_path ?? null;

  // ── `/` 命令 ──
  //
  // 判断规则和匹配都在 `slashMenu.ts`（纯函数，有测试）。这里只管状态与渲染。
  const clearTurns = useChat((s) => s.clear);
  const toggleSettings = useChat((s) => s.toggleSettings);
  const toggleMcp = useChat((s) => s.toggleMcp);
  const [slashEntries, setSlashEntries] = useState<SlashEntry[]>([]);
  const [slashIndex, setSlashIndex] = useState(0);
  // Esc 关掉之后要一直关着，直到 draft 变化 —— 否则按了 Esc 弹层立刻又回来
  const [slashDismissed, setSlashDismissed] = useState(false);
  /**
   * 内置动作的反馈。
   *
   * `/clear`、`/settings`、`/mcp` 的效果自己看得见（消息清了、面板开了），
   * 但 `/init` 和 `/reindex` **完全不可见** —— 我第一版只写了 console.info，
   * 界面上一点反馈都没有，用户不知道成功了没、也不知道该不该再点一次。
   *
   * 一个看不见结果的动作等于没做。
   */
  const [actionMsg, setActionMsg] = useState<{ text: string; bad?: boolean } | null>(null);
  function report(text: string, bad = false) {
    setActionMsg({ text, bad });
    window.setTimeout(() => setActionMsg(null), bad ? 8000 : 5000);
  }

  /**
   * 技能挑选器的数据。
   *
   * **和 `/` 弹层共用同一套交互** —— 上下键、回车、点击、过滤都是现成的，
   * 只是把候选换成技能。第一版让人打 id（`/skill add <id>`），
   * 用户得先看一遍列表记住 id 再打一遍，那是我做懒了。
   */
  const [skillPool, setSkillPool] = useState<{ all: SkillLike[]; active: string[] }>({
    all: [],
    active: [],
  });
  const reloadSkillPool = useCallback(() => {
    void listSkills(workspacePath)
      .then((v) => setSkillPool({ all: v.skills as SkillLike[], active: v.active }))
      .catch(() => {});
  }, [workspacePath]);
  useEffect(reloadSkillPool, [reloadSkillPool]);

  const skillPicker = skillPickerQuery(draft);
  const skillPicks = skillPicker
    ? skillPickerEntries(
        skillPicker.op,
        skillPool.all,
        skillPool.active,
        skillPicker.q,
        i18n.language.startsWith("zh"),
      )
    : [];
  // **空了也要开着弹层** —— 打了 `/skill add ` 却一片空白，看着像卡住了。
  // 那一行显示的是「为什么空」，不是候选。
  const skillPickerOpen = skillPicker !== null && !slashDismissed;

  const slashQ = slashQuery(draft);
  const slashMatches = slashQ === null ? [] : filterEntries(slashEntries, slashQ);
  // **匹配为空就不显示。** 这条是绝对路径（`/Users/…`）的兜底：
  // 它形式上像命令名，但匹配不到任何候选，于是弹层不出现、Enter 也不被劫走
  const slashOpen = slashQ !== null && slashMatches.length > 0 && !slashDismissed;

  /**
   * 拉候选列表。**每次弹层打开都重拉一遍。**
   *
   * 原来只在挂载和切工作区时拉一次，而技能是会在对话里被创建的 ——
   * Agent 往 `.anima/skills/` 写完几个 SKILL.md，文件是好的、后端也认，
   * 但菜单还是进会话时那一份，得重启才看得见。
   *
   * 后端本来就每次从磁盘读（几个文件、几 KB），重拉的代价可以忽略。
   */
  const refreshSlash = useCallback(() => {
    void listSlashCommands(workspacePath)
      .then(setSlashEntries)
      // 读不到就保持上一份，不要清空 —— 清空会让已经打开的弹层突然空掉
      .catch(() => {});
  }, [workspacePath]);
  useEffect(refreshSlash, [refreshSlash]);

  // **弹层刚打开的那一刻重拉一次。** 每次按键都拉是浪费。
  const prevSlashQ = useRef<string | null>(null);
  useEffect(() => {
    if (justOpened(prevSlashQ.current, slashQ)) {
      refreshSlash();
      // `/skill add` 的候选池也一起 —— 它和斜杠菜单是两份数据
      reloadSkillPool();
    }
    prevSlashQ.current = slashQ;
  }, [slashQ, refreshSlash, reloadSkillPool]);

  // draft 一变就重置选中项与 Esc 状态
  useEffect(() => {
    setSlashIndex(0);
    setSlashDismissed(false);
  }, [draft]);

  /// 执行一个内置动作。**动作本身在前端** —— 清空对话、开面板都是前端状态，
  /// 绕一趟 Tauri 只会让它变慢且多一个失败点。
  async function runBuiltin(action: string, args = "") {
    // 需要工作区的两个动作先把「为什么没反应」说清楚 ——
    // 原先是 `if (!workspacePath) break;`，彻底静默
    if ((action === "init" || action === "reindex") && !workspacePath) {
      report("当前会话没有绑定项目工作区 —— 在左侧选一个项目，或到项目设置里设置工作区路径", true);
      return;
    }
    // 上面的守卫已经保证这两个动作有工作区，但 TS 不跨 switch 收窄
    const ws = workspacePath ?? "";
    switch (action) {
      case "clear":
        clearTurns();
        // 台账跟着一起归档 —— 清空对话之后还挂着上一个任务的台账，
        // 那就是「跨会话的幽灵台账」的另一个入口
        try {
          await api.taskLedgerArchive(
            useProject.getState().activeSessionId,
            workspacePath,
          );
        } catch {
          // 归档失败不值得打断
        }
        break;
      case "settings":
        toggleSettings();
        break;
      case "skill":
        await runSkillCommand(args);
        break;
      case "mcp":
        toggleMcp();
        report("已清空当前对话");
        break;
      case "init":
        // **`/init` 在斜杠里成立，是因为三个参数全都已知**：路径来自当前项目、
        // 名字来自项目名、模板由后端沿用 .anima/project.json 里已有的那个。
        // 需要填表的动作不该做成斜杠命令。
        //
        // 而它真正的用处是**补齐**而不是首次初始化：init 生成的东西会随版本
        // 增加（比如后来加的 code-reviewer 示例技能），老项目的 .anima/ 里没有。
        // workspace_init 对已存在的文件不覆盖，所以重跑是安全的。
        try {
          const st = await api.workspaceInit({
            path: ws,
            project_name: activeProject?.name || "项目",
            // 不传模板 —— 后端会沿用 .anima/project.json 里已有的那个
            template_type: undefined,
          });
          // **区分「补齐了」和「本来就齐」。** 两者都成功，但用户要的是
          // 后者的确认 —— 否则会怀疑是不是没执行，然后反复点
          report(
            st.created.length > 0
              ? `已补齐 ${st.created.length} 个文件：${st.created.join("、")}`
              : "脚手架已经是齐的，没有需要补的文件",
          );
        } catch (err) {
          report(`补齐脚手架失败：${err}`, true);
        }
        break;
      case "reindex":
        report("正在重建索引…");
        try {
          const n = await api.workspaceReindexMemory(ws);
          report(
            n > 0
              ? `已索引 ${n} 块，可以在对话里用 search_knowledge 查了`
              : "知识库为空，索引已清空。把文档放进 .anima/knowledge/ 再点一次",
          );
        } catch (err) {
          report(`重建索引失败：${err}`, true);
        }
        break;
    }
  }

  /**
   * `/skill` —— 在对话里查看 / 挂载 / 卸载技能。
   *
   * **挂载写的是全局配置**（`config.active_skills`），这是当前实现的事实；
   * 列表里会明说这一点，而不是让人以为它跟着项目走。
   */
  async function runSkillCommand(args: string) {
    const cmd = parseSkillCommand(args);
    if (cmd.op === "error") {
      report(cmd.message, true);
      return;
    }
    let view;
    try {
      view = await listSkills(workspacePath);
    } catch (err) {
      report(`读不到技能列表：${err}`, true);
      return;
    }
    if (cmd.op === "list") {
      report(renderSkillList(view.skills, view.active, view.broken ?? []));
      return;
    }
    const bad = validateTarget(view.skills, cmd.id, cmd.op);
    if (bad) {
      report(bad, true);
      return;
    }
    const next =
      cmd.op === "add"
        ? [...new Set([...view.active, cmd.id])]
        : view.active.filter((x) => x !== cmd.id);
    // **没变化也要说一声。** 「已经挂着了」和「挂上了」都成功，
    // 而用户要的是前者的确认 —— 否则会怀疑没执行然后反复敲。
    if (next.length === view.active.length && cmd.op === "add") {
      report(`「${cmd.id}」已经挂着了`);
      return;
    }
    if (next.length === view.active.length && cmd.op === "remove") {
      report(`「${cmd.id}」本来就没挂`);
      return;
    }
    try {
      await selectSkills(next, workspacePath);
      // 候选池要跟着变 —— 否则刚挂上的还留在 add 列表里
      reloadSkillPool();
      report(
        cmd.op === "add"
          ? `已挂载「${cmd.id}」，正文进 System Prompt，下一条消息起生效（全局）`
          : `已卸载「${cmd.id}」`,
      );
    } catch (err) {
      report(`${cmd.op === "add" ? "挂载" : "卸载"}失败：${err}`, true);
    }
  }

  /// 选中弹层里的一项。
  async function pickSlash(entry: SlashEntry) {
    const completion = completionFor(entry);
    if (completion !== null) {
      // 需要参数：补全命令名，留一个空格等用户打
      setDraft(completion);
      textareaRef.current?.focus();
      return;
    }
    setDraft("");
    if (entry.kind === "builtin" && entry.action) {
      await runBuiltin(entry.action);
      return;
    }
    // 不需要参数的技能：直接展开并发出去
    await dispatchSkill(entry.id, "");
  }

  /// 展开一个技能命令并发出去。
  ///
  /// 显示的是用户敲的那一行，发给模型的是展开后的规程全文 —— 见
  /// `ChatTurn.displayText`。历史预算算的是发出去那份。
  async function dispatchSkill(id: string, args: string) {
    const shown = args ? `/${id} ${args}` : `/${id}`;
    try {
      const expanded = await expandSlashCommand(id, args, workspacePath);
      await send(expanded, undefined, shown);
    } catch (err) {
      // 展开失败就把原样文本发出去 —— 比静默丢掉用户这一句好
      await send(`${shown}\n\n（这个命令展开失败：${err}）`);
    }
  }

  // 运行时长程后台 Job 监听与终止
  const [runtimeJobs, setRuntimeJobs] = useState<import("../api/types").RuntimeJob[]>([]);
  useEffect(() => {
    let alive = true;
    const fetchJobs = async () => {
      try {
        const jobs = await api.runtimeListJobs();
        if (alive) {
          setRuntimeJobs(
            jobs.filter(
              (j) =>
                j.status === "running" ||
                j.status === "paused" ||
                j.status === "waiting_approval"
            )
          );
        }
      } catch {}
    };
    void fetchJobs();
    const timer = setInterval(fetchJobs, 2500);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, []);

  const handleCancelJob = async (jobId: string) => {
    try {
      await api.runtimeCancelJob(jobId, "用户在抽屉手动终止");
      setRuntimeJobs((prev) => prev.filter((j) => j.id !== jobId));
    } catch (err) {
      console.error("Failed to cancel job:", err);
    }
  };

  // 点击外部自动关闭下拉浮层
  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (modelDropdownRef.current && !modelDropdownRef.current.contains(e.target as Node)) {
        setIsModelDropdownOpen(false);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, []);

  // 切换当前链路/供应商下的模型（同时更新链路覆盖与供应商默认，确保立即生效）
  const handleSelectModel = async (model: string) => {
    if (!activeProviderId) return;
    try {
      await api.setRouteModel("realtime", model);
      await api.updateProvider({ id: activeProviderId, model });
      await refreshConfig();
      setIsModelDropdownOpen(false);
    } catch (err) {
      console.error("Failed to switch model:", err);
    }
  };

  // 切换活跃供应商
  const handleSelectProvider = async (providerId: string) => {
    try {
      await api.setTierProvider("realtime", providerId);
      await refreshConfig();
      void loadModels(providerId);
    } catch (err) {
      console.error("Failed to switch provider:", err);
    }
  };

  const voiceRunning = useVoice((s) => s.running);
  const voiceBusy = useVoice((s) => s.busy);
  const toggleVoice = useVoice((s) => s.toggle);
  const interrupt = useVoice((s) => s.interrupt);
  const voiceState = useVoice((s) => s.state);
  const streaming = !!activeStreamId;

  // 任务/工具执行抽屉展开状态
  const [drawerExpanded, setDrawerExpanded] = useState(true);
  /**
   * 用户是不是**主动**把抽屉收起来了。
   *
   * 抽屉有 6 处会自动展开（审批到达、出现选择项、音乐开始播放……），
   * 而用户只有一个开关。不记住用户意图的话，他刚收起来，下一个事件立刻又顶开
   * —— 用户的原话是「抽屉状态一直不正确，所以改来改去」。
   *
   * 规则很简单：**用户手动收起之后，任何自动展开都不再生效，直到他手动展开。**
   * 用 ref 而不是 state：它只用来做判断，不该触发重渲染。
   */
  const userCollapsedDrawer = useRef(false);

  /** 自动展开抽屉。用户主动收起过就不再打扰他。 */
  const autoExpandDrawer = useCallback(() => {
    if (userCollapsedDrawer.current) return;
    setDrawerExpanded(true);
  }, []);

  /** 用户点开关。这里同时记下他的意图。 */
  const toggleDrawerByUser = useCallback(() => {
    setDrawerExpanded((prev) => {
      userCollapsedDrawer.current = prev; // prev=true 说明这一下是"收起"
      return !prev;
    });
  }, []);

  // 监听并管理运行时待审批请求 (完全内嵌在抽屉中，不弹侵入式全屏窗)
  useEffect(() => {
    let alive = true;
    void runtimeListPendingApprovals()
      .then((requests) => {
        if (!alive) return;
        setPendingApprovals(requests);
        if (requests.length > 0) autoExpandDrawer();
      })
      .catch(() => {});

    const unlisten = onRuntimeApproval((request) => {
      setPendingApprovals((current) => {
        if (current.some((r) => r.id === request.id)) return current;
        return [...current, request];
      });
      autoExpandDrawer();
    });

    return () => {
      alive = false;
      void unlisten.then((stop) => stop());
    };
  }, []);

  // 交互决策键盘快捷键 (1~9, Esc) 与语音指令监听
  useEffect(() => {
    if (!activeChoice) return;
    autoExpandDrawer();

    const handleKeyDown = (e: KeyboardEvent) => {
      const num = parseInt(e.key, 10);
      if (!isNaN(num) && num >= 1 && num <= activeChoice.options.length) {
        e.preventDefault();
        resolveChoice(activeChoice.options[num - 1].id);
      } else if (e.key === "Escape") {
        e.preventDefault();
        cancelChoice();
      }
    };

    window.addEventListener("keydown", handleKeyDown);

    let unlistenVoice: (() => void) | undefined;
    import("@tauri-apps/api/event").then(({ listen }) => {
      listen<{ kind: string; text?: string }>("anima://voice", (event) => {
        if (event.payload.kind === "transcript" && event.payload.text) {
          const text = event.payload.text.toLowerCase();
          activeChoice.options.forEach((opt, idx) => {
            const numStr = (idx + 1).toString();
            const chineseNums = ["一", "二", "三", "四", "五"];
            const cNum = chineseNums[idx] || "";
            if (
              text.includes(`选第${cNum}`) ||
              text.includes(`第${cNum}个`) ||
              text.includes(`选项${cNum}`) ||
              text.includes(`选项${numStr}`) ||
              text.includes(`选${numStr}`) ||
              text.includes(opt.label.toLowerCase())
            ) {
              resolveChoice(opt.id);
            }
          });
        }
      }).then((u) => {
        unlistenVoice = u;
      });
    });

    return () => {
      window.removeEventListener("keydown", handleKeyDown);
      unlistenVoice?.();
    };
  }, [activeChoice, resolveChoice, cancelChoice]);

  // 待授权请求键盘快捷键 (1: Deny, 2: Once, 3: Session, 4: Workspace)
  useEffect(() => {
    if (activeChoice || pendingApprovals.length === 0) return;
    autoExpandDrawer();

    const handleKeyDown = (e: KeyboardEvent) => {
      // 只有在没有聚焦在输入框打字时才触发数字键
      if (document.activeElement?.tagName === "TEXTAREA" || document.activeElement?.tagName === "INPUT") {
        return;
      }
      const topReq = pendingApprovals[0];
      if (!topReq) return;

      if (e.key === "1") {
        e.preventDefault();
        void handleResolveApproval(topReq.id, "allow", "once");
      } else if (e.key === "2") {
        e.preventDefault();
        void handleResolveApproval(topReq.id, "allow", "session");
      } else if (e.key === "3") {
        e.preventDefault();
        void handleResolveApproval(topReq.id, "allow", "workspace");
      } else if (e.key === "4") {
        e.preventDefault();
        void handleResolveApproval(topReq.id, "deny");
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [activeChoice, pendingApprovals, approvalBusy]);

  // 监听回滚事件并自动填充输入框
  useEffect(() => {
    const handleSetDraft = (e: Event) => {
      const customEvent = e as CustomEvent<string>;
      if (customEvent.detail) {
        setDraft(customEvent.detail);
        textareaRef.current?.focus();
      }
    };
    window.addEventListener("anima:set-draft", handleSetDraft);
    return () => window.removeEventListener("anima:set-draft", handleSetDraft);
  }, []);

  // 监听后端派发的全局音频播放事件（当模型调用 play_audio 或后台开始播歌时自动展开抽屉并入列播放）
  useEffect(() => {
    let unlistenPlay: (() => void) | undefined;
    let unlistenStop: (() => void) | undefined;
    let unlistenImage: (() => void) | undefined;
    let unlistenVideo: (() => void) | undefined;

    import("@tauri-apps/api/event").then(({ listen }) => {
      listen<{
        title?: string;
        local_path?: string;
        localPath?: string;
        source_path?: string | null;
        sourcePath?: string | null;
        audio_url?: string;
        audioUrl?: string;
        url?: string;
        duration?: number;
      }>(
        "runtime://music_playing",
        (event) => {
          const payload = event.payload;
          if (payload) {
            const localPath = payload.local_path || payload.localPath;
            const audioUrl =
              payload.audio_url ||
              payload.audioUrl ||
              payload.url ||
              (localPath ? `file://${localPath}` : "");
            if (!audioUrl) return;
            useActiveMusic.getState().playTrack({
              title: payload.title || "本地音频",
              localPath: payload.source_path || payload.sourcePath || localPath,
              audioUrl,
              duration: payload.duration || 30,
            });
            autoExpandDrawer();
          }
        }
      ).then((u) => {
        unlistenPlay = u;
      });

        // 图片生成进度。后端每个阶段和**每条失败路径**都发，
        // 在这之前前端没订阅 —— 超时的图只会永远转骨架屏，没有任何失败提示。
        listen<{
          stream_id?: string;
          event?: {
            type?: string;
            prompt?: string;
            width?: number;
            height?: number;
            stage?: string;
            label?: string;
            progress?: number;
            output_path?: string | null;
            message?: string | null;
            engine?: string | null;
          };
        }>("anima://image-generation-progress", (e) => {
          const ev = e.payload?.event;
          if (!ev || ev.type !== "image_generation_progress" || !ev.prompt) return;
          useImageProgress
            .getState()
            .apply(progressSlot(ev.prompt, ev.width, ev.height), {
              stage: ev.stage || "running",
              label: ev.label || "",
              engine: ev.engine || undefined,
              progress: typeof ev.progress === "number" ? ev.progress : 0,
              outputPath: ev.output_path || undefined,
              message: ev.message || undefined,
            });
        }).then((u) => {
          unlistenImage = u;
        });

        listen<{
          stream_id?: string;
          event?: {
            type?: string;
            call_id?: string;
            prompt?: string;
            duration_secs?: number;
            stage?: string;
            label?: string;
            progress?: number;
            output_path?: string | null;
            message?: string | null;
          };
        }>("anima://video-generation-progress", (e) => {
          const ev = e.payload?.event;
          if (!ev || ev.type !== "video_generation_progress" || !ev.prompt) return;
          const prog = {
            callId: ev.call_id,
            stage: ev.stage || "running",
            label: ev.label || "",
            progress: typeof ev.progress === "number" ? ev.progress : 0,
            outputPath: ev.output_path || undefined,
            message: ev.message || undefined,
          };
          // 双向存入：既按 call_id 精准索引，又按 prompt+时长 兼容索引
          if (ev.call_id) {
            useVideoProgress.getState().apply(`call:${ev.call_id}`, prog);
          }
          useVideoProgress.getState().apply(videoProgressSlot(ev.prompt, ev.duration_secs), prog);
        }).then((u) => {
          unlistenVideo = u;
        });

      listen("runtime://music_stopped", () => {
        useActiveMusic.getState().stopTrack();
      }).then((u) => {
        unlistenStop = u;
      });
    });

    return () => {
      unlistenPlay?.();
      unlistenStop?.();
      unlistenImage?.();
      unlistenVideo?.();
    };
  }, []);

  const handleResolveApproval = async (
    id: string,
    decision: RuntimeApprovalDecision,
    scope: RuntimeApprovalScope = "once",
  ) => {
    if (approvalBusy) return;
    setApprovalBusy(true);
    try {
      await runtimeResolveApproval(id, decision, scope);
      setPendingApprovals((prev) => prev.filter((item) => item.id !== id));
    } finally {
      setApprovalBusy(false);
    }
  };

  // 收集当前活跃的任务/工具调用 (仅在实际后台任务运行时弹出抽屉，纯对话文字流不弹出)
  const activeTasksList: Array<{ id?: string; title: string; detail?: string; killable?: boolean; isJob?: boolean }> = [];

  // 1. 漫剧角色资产后台生成任务
  const activeSheetTasks = useCinemaTasks((s) => s.activeSheetTasks);
  Object.values(activeSheetTasks).forEach((t) => {
    if (t.isProcessing) {
      activeTasksList.push({
        title: `全维度资产生成 [${t.characterId}]`,
        detail: `(${t.done}/${t.total}) ${t.label}`,
      });
    }
  });

  // 2. 运行时持久化 Job
  runtimeJobs.forEach((j) => {
    activeTasksList.push({
      id: j.id,
      title: j.title || (j.kind === "storyboard" ? "分镜关键帧渲染" : j.kind === "character_assets" ? "角色资产生成" : j.kind),
      detail: `状态: ${j.status}`,
      killable: true,
      isJob: true,
    });
  });

  // 3. 活跃音频播放状态
  const currentMusicTrack = useActiveMusic((s) => s.currentTrack);
  const stopMusicTrack = useActiveMusic((s) => s.stopTrack);
  const globalMusicAudioRef = useRef<HTMLAudioElement | null>(null);
  const currentMusicAudioSrc = currentMusicTrack
    ? currentMusicTrack.audioUrl.startsWith("file://")
      ? convertFileSrc(currentMusicTrack.audioUrl.slice(7))
      : currentMusicTrack.audioUrl
    : undefined;

  useEffect(() => {
    if (currentMusicTrack) {
      autoExpandDrawer();
    }
  }, [currentMusicTrack?.audioUrl]);

  useEffect(() => {
    const audio = globalMusicAudioRef.current;
    if (!audio || !currentMusicTrack) return;
    if (currentMusicTrack.isPlaying) {
      void audio.play().catch(() => {
        console.warn("[Anima] music playback failed", audio.error);
        useActiveMusic.getState().pauseTrack();
      });
    } else if (!audio.paused) {
      audio.pause();
    }
  }, [currentMusicTrack?.audioUrl, currentMusicTrack?.isPlaying]);

  const isRunning =
    activeTasksList.length > 0 ||
    pendingApprovals.length > 0 ||
    !!activeChoice ||
    !!currentMusicTrack;

  // 自动调整输入框高度
  useEffect(() => {
    if (textareaRef.current) {
      textareaRef.current.style.height = "auto";
      textareaRef.current.style.height = `${Math.min(textareaRef.current.scrollHeight, 160)}px`;
    }
  }, [draft]);

  // 全局窗口拖拽文件支持 (支持从系统访达/桌面直接拖入任何位置)
  useEffect(() => {
    let unlistenTauriDrop: (() => void) | undefined;
    let isMounted = true;

    // 1. 监听 Tauri 原生从系统/访达拖入的文件事件
    (async () => {
      try {
        const { getCurrentWebview } = await import("@tauri-apps/api/webview");
        if (!isMounted) return;
        // 落在工作台拖放区（上传框、首帧槽位）上的文件归那个区，聊天栏让路 ——
        // 不让的话同一张图会既传进资产、又挂成聊天附件（见 studio/nativeDrop.ts）
        const onZone = (position: { x: number; y: number }) => {
          const p = toLogical(position, window.devicePixelRatio);
          return dropZoneAt(p.x, p.y) !== null;
        };
        const unlisten = await getCurrentWebview().onDragDropEvent((event) => {
          if (event.payload.type === "over") {
            setIsDraggingOver(!onZone(event.payload.position));
          } else if (event.payload.type === "drop") {
            setIsDraggingOver(false);
            if (onZone(event.payload.position)) return;
            if (event.payload.paths && event.payload.paths.length > 0) {
              processDroppedPaths(event.payload.paths);
            }
          } else {
            setIsDraggingOver(false);
          }
        });
        if (isMounted) {
          unlistenTauriDrop = unlisten;
        } else {
          unlisten();
        }
      } catch {
        // 非 Tauri 环境（如纯浏览器调试）静默降级
      }
    })();

    // 2. HTML5 原生拖拽事件兜底（用于网页内部元素或非 Tauri 场景）
    const handleWindowDragOver = (e: DragEvent) => {
      e.preventDefault();
      if (e.dataTransfer?.types.includes("Files")) {
        setIsDraggingOver(true);
      }
    };
    const handleWindowDragLeave = (e: DragEvent) => {
      e.preventDefault();
      if (e.clientX <= 0 || e.clientY <= 0 || e.clientX >= window.innerWidth || e.clientY >= window.innerHeight) {
        setIsDraggingOver(false);
      }
    };
    const handleWindowDrop = (e: DragEvent) => {
      e.preventDefault();
      setIsDraggingOver(false);
      if (e.dataTransfer?.files && e.dataTransfer.files.length > 0) {
        processFiles(e.dataTransfer.files);
      }
    };

    window.addEventListener("dragover", handleWindowDragOver);
    window.addEventListener("dragleave", handleWindowDragLeave);
    window.addEventListener("drop", handleWindowDrop);

    return () => {
      isMounted = false;
      if (unlistenTauriDrop) unlistenTauriDrop();
      window.removeEventListener("dragover", handleWindowDragOver);
      window.removeEventListener("dragleave", handleWindowDragLeave);
      window.removeEventListener("drop", handleWindowDrop);
    };
  }, [workspacePath]);

  // 处理从系统拖拽进来的本地文件路径
  const processDroppedPaths = async (paths: string[]) => {
    for (const path of paths) {
      try {
        const info = await api.loadDroppedAttachment({
          path,
          workspacePath,
        });
        const id = `${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
        setAttachments((prev) => [
          ...prev,
          {
            id,
            name: info.name,
            size: info.size,
            isImage: info.is_image,
            dataUrl: info.data_url,
            localPath: info.local_path,
            fileUrl: info.file_url,
            textContent: info.text_content,
            extension: info.extension,
          },
        ]);
      } catch (err) {
        console.warn("加载拖拽文件失败：", path, err);
      }
    }
  };

  // 处理图片与文档文件的读取并加入附件列表
  const processFiles = (files: FileList | File[]) => {
    Array.from(files).forEach((file) => {
      const isImage = file.type.startsWith("image/") || /\.(png|jpe?g|gif|webp|bmp|svg)$/i.test(file.name);
      const ext = file.name.split(".").pop() || "";
      const id = `${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;

      if (isImage) {
        downsampleImage(file)
          .then(async (compressed) => {
            let saved: api.SavedChatImageAttachment | null = null;
            let saveError: string | undefined;
            try {
              saved = await api.saveChatImageAttachment({
                dataUrl: compressed.dataUrl,
                filename: file.name,
                workspacePath,
              });
            } catch (err) {
              saveError = String(err);
              console.warn("保存图片附件失败，仍会作为视觉输入发送：", err);
            }
            setAttachments((prev) => [
              ...prev,
              {
                id,
                name: file.name,
                size: compressed.size,
                isImage: true,
                dataUrl: compressed.dataUrl,
                localPath: saved?.path,
                fileUrl: saved?.file_url,
                saveError,
                extension: ext,
              },
            ]);
          })
          .catch(() => {
            // 降级原样读取
            const reader = new FileReader();
            reader.onload = async (e) => {
              const result = e.target?.result as string;
              if (result) {
                let saved: api.SavedChatImageAttachment | null = null;
                let saveError: string | undefined;
                try {
                  saved = await api.saveChatImageAttachment({
                    dataUrl: result,
                    filename: file.name,
                    workspacePath,
                  });
                } catch (err) {
                  saveError = String(err);
                  console.warn("保存图片附件失败，仍会作为视觉输入发送：", err);
                }
                setAttachments((prev) => [
                  ...prev,
                  {
                    id,
                    name: file.name,
                    size: file.size,
                    isImage: true,
                    dataUrl: result,
                    localPath: saved?.path,
                    fileUrl: saved?.file_url,
                    saveError,
                    extension: ext,
                  },
                ]);
              }
            };
            reader.readAsDataURL(file);
          });
      } else {
        // 读取文本类/代码类/配置文件内容
        const reader = new FileReader();
        reader.onload = (e) => {
          const text = e.target?.result as string;
          setAttachments((prev) => [
            ...prev,
            {
              id,
              name: file.name,
              size: file.size,
              isImage: false,
              textContent: text || "",
              extension: ext,
            },
          ]);
        };
        reader.readAsText(file, "UTF-8");
      }
    });
  };

  const handlePaste = (e: React.ClipboardEvent) => {
    const items = e.clipboardData?.items;
    if (!items) return;
    const files: File[] = [];
    for (let i = 0; i < items.length; i++) {
      const file = items[i].getAsFile();
      if (file) files.push(file);
    }
    if (files.length > 0) {
      e.preventDefault();
      processFiles(files);
    }
  };

  const removeAttachment = (idx: number) => {
    setAttachments((prev) => prev.filter((_, i) => i !== idx));
  };

  async function submit() {
    const text = draft.trim();
    const currentAttachments = [...attachments];
    if ((!text && currentAttachments.length === 0) || streaming) return;

    // ── 斜杠命令 ──
    //
    // **查不到条目就当普通文本发。** 绝对路径（`/Users/you/note.md`）
    // 形式上和命令一样，吞掉它会让用户没法粘路径，而那是很常见的一句话。
    const parsed = parseSubmitted(text);
    const matched = parsed
      ? slashEntries.find((entry) => entry.id === parsed.name)
      : undefined;
    if (parsed && matched) {
      setDraft("");
      setAttachments([]);
      if (textareaRef.current) {
        textareaRef.current.style.height = "auto";
      }
      if (matched.kind === "builtin" && matched.action) {
        await runBuiltin(matched.action, parsed.args);
      } else {
        await dispatchSkill(matched.id, parsed.args);
      }
      return;
    }

    setDraft("");
    setAttachments([]);
    if (textareaRef.current) {
      textareaRef.current.style.height = "auto";
    }

    // 1. 提取视觉多模态图片 Base64
    const imageUrls = currentAttachments
      .filter((a) => a.isImage && a.dataUrl)
      .map((a) => a.dataUrl!);

    // 2. 提取并组装文档附件上下文
    const docs = currentAttachments.filter((a) => !a.isImage && a.textContent);
    const imageFiles = currentAttachments.filter((a) => a.isImage && a.fileUrl);
    const imageSaveFailures = currentAttachments.filter((a) => a.isImage && !a.fileUrl);
    let promptToSend = text;
    if (imageFiles.length > 0) {
      const imageBlocks = imageFiles
        .map(
          (img, index) =>
            `- Image ${index + 1}: ${img.name} (${formatFileSize(img.size)})\n  file_url: ${img.fileUrl}\n  local_path: ${img.localPath}`
        )
        .join("\n");
      const failureNote =
        imageSaveFailures.length > 0
          ? `\n\n[注意：另有 ${imageSaveFailures.length} 张图片只作为视觉输入发送，未能保存成本地文件，不能直接用于 animate_image。]`
          : "";
      const imageToolContext = `[上传图片]\n以下图片已经保存为本地文件。若用户要求根据上传图片生成视频、图生视频或 I2V，必须调用 animate_image，并把对应的 file_url 或 local_path 原样作为 image_path 参数传入。\n${imageBlocks}${failureNote}`;
      promptToSend = promptToSend ? `${imageToolContext}\n\n${promptToSend}` : imageToolContext;
    } else if (imageSaveFailures.length > 0) {
      const note = `[上传图片]\n${imageSaveFailures.length} 张图片只作为视觉输入发送，未能保存成本地文件；如果用户要求 I2V，请先说明这次无法直接把上传图传给 animate_image。`;
      promptToSend = promptToSend ? `${note}\n\n${promptToSend}` : note;
    }
    if (docs.length > 0) {
      const docBlocks = docs
        .map(
          (d) =>
            `[附件文档: ${d.name} (${formatFileSize(d.size)})]\n\`\`\`${d.extension}\n${d.textContent}\n\`\`\``
        )
        .join("\n\n");
      promptToSend = promptToSend ? `${docBlocks}\n\n${promptToSend}` : docBlocks;
    }

    const displayText =
      imageFiles.length > 0 || imageSaveFailures.length > 0
        ? text || `已上传 ${currentAttachments.filter((a) => a.isImage).length} 张图片`
        : undefined;

    await send(promptToSend, imageUrls.length > 0 ? imageUrls : undefined, displayText);
  }

  const canSubmit = (draft.trim().length > 0 || attachments.length > 0) && !streaming;

  return (
    <div className="flex flex-col px-4 py-3 pb-3.5 bg-[var(--surface)] shrink-0 sticky bottom-0 z-10 w-full">
      {/* 隐藏的通用文件选择器 (支持图片 + 各类文档格式多选) */}
      <input
        ref={fileInputRef}
        type="file"
        multiple
        className="hidden"
        onChange={(e) => {
          if (e.target.files) {
            processFiles(e.target.files);
            e.target.value = "";
          }
        }}
      />

      {/* 现代一体化卡片输入框 (极简无边框设计，去掉刺眼蓝边，完美融合背景) */}
      <div
        className={`w-full bg-[var(--input-bg)] border rounded-2xl shadow-xs transition-all flex flex-col relative ${
          isDraggingOver
            ? "border-[var(--line-strong)] bg-[var(--surface-2)]"
            : "border-[var(--line)]"
        }`}
        onPaste={handlePaste}
        onDragOver={(e) => {
          e.preventDefault();
          setIsDraggingOver(true);
        }}
        onDragLeave={(e) => {
          e.preventDefault();
          setIsDraggingOver(false);
        }}
        onDrop={(e) => {
          e.preventDefault();
          setIsDraggingOver(false);
          if (e.dataTransfer?.files) {
            processFiles(e.dataTransfer.files);
          }
        }}
      >
        {/* 工具/任务执行状态抽屉 (自动出现/消失与手动折叠均具备平滑高度滑动动画) */}
        <div
          className={`w-full grid transition-[grid-template-rows,opacity] duration-300 ease-in-out ${
            isRunning
              ? "grid-rows-[1fr] opacity-100 border-b border-[var(--line)]/50 bg-[var(--surface-3)]/30 rounded-t-2xl overflow-hidden"
              : "grid-rows-[0fr] opacity-0 border-b-0"
          }`}
        >
          <div className="overflow-hidden">
            {/* 头部摘要栏 (状态指示灯 + 任务统计 + 活跃工作区标签) */}
            <div
              className="flex items-center justify-between px-3.5 py-1.5 cursor-pointer hover:bg-[var(--surface-3)]/50 transition-colors text-[12px] font-medium select-none"
              onClick={toggleDrawerByUser}
            >
              <div className="flex items-center gap-2 text-[var(--ink)]">
                <span
                  className={`w-2 h-2 rounded-full ${
                    activeChoice
                      ? "bg-[var(--celadon)] animate-ping"
                      : pendingApprovals.length > 0
                      ? "bg-amber-500 animate-ping"
                      : "bg-[var(--celadon)] animate-ping"
                  }`}
                />
                <span className="font-semibold text-[11px] text-[var(--ink-2)] flex items-center gap-1.5">
                  {currentMusicTrack ? (
                    <>
                      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="text-[var(--ink-2)] opacity-80">
                        <path d="M9 18V5l12-2v13" />
                        <circle cx="6" cy="18" r="3" />
                        <circle cx="18" cy="16" r="3" />
                      </svg>
                      <span className="truncate max-w-[220px]">
                        {currentMusicTrack.isPlaying ? "正在播放: " : "已暂停: "}
                        {currentMusicTrack.title}
                      </span>
                    </>
                  ) : activeChoice ? (
                    "🎯 交互决策待确认"
                  ) : pendingApprovals.length > 0 ? (
                    `🔒 需要授权 (${pendingApprovals.length})${
                      activeTasksList.length > 0 ? ` · ${activeTasksList.length} 任务执行中` : ""
                    }`
                  ) : (
                    `${activeTasksList.length} ${
                      activeTasksList.length > 1 ? "tasks running" : "task running"
                    }`
                  )}
                </span>
              </div>

              {/* 右侧：工作区快捷胶囊 + 折叠箭头 */}
              <div className="flex items-center gap-2">
                <button
                  type="button"
                  onClick={(e) => {
                    e.stopPropagation();
                    toggleSidebar();
                  }}
                  className="px-2 py-0.5 rounded-md bg-[var(--surface-2)] hover:bg-[var(--surface-3)] text-[10.5px] font-mono text-[var(--ink-2)] border border-[var(--line)] flex items-center gap-1 cursor-pointer transition-colors shadow-2xs"
                  title={`当前工作区: ${activeProject?.name || "全局"} · 点击切换侧边栏`}
                >
                  <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="opacity-70">
                    <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
                  </svg>
                  <span className="truncate max-w-[120px]">{activeProject?.name || "全局工作区"}</span>
                </button>

                <button
                  type="button"
                  className="text-[var(--ink-3)] hover:text-[var(--ink)] p-0.5 cursor-pointer"
                  title={drawerExpanded ? "收起任务抽屉" : "展开任务抽屉"}
                  aria-label={drawerExpanded ? "收起任务抽屉" : "展开任务抽屉"}
                >
                  <svg
                    width="12"
                    height="12"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="2.5"
                    className={`transition-transform duration-300 ease-in-out ${drawerExpanded ? "" : "rotate-180"}`}
                  >
                    <path d="M6 9l6 6 6-6" />
                  </svg>
                </button>
              </div>
            </div>

            {/* 抽屉详细内容区 (带平滑滑动高度过渡) */}
            <div
              className={`grid transition-[grid-template-rows,opacity] duration-300 ease-in-out ${
                drawerExpanded ? "grid-rows-[1fr] opacity-100 border-t border-[var(--line)]/40" : "grid-rows-[0fr] opacity-0"
              }`}
            >
              <div className="overflow-hidden">
                <div className="p-3 px-3.5 flex flex-col gap-3 text-[11.5px] bg-transparent">
                  {/* 0. 正在播放的音乐控制器 (Apple 风格纯净播放面板) */}
                  {currentMusicTrack && (
                    <div className="p-2.5 px-3 rounded-xl bg-[var(--surface-2)]/70 border border-[var(--line)]/60 flex flex-col gap-2 shadow-2xs backdrop-blur-sm">
                      <div className="flex items-center justify-between gap-2">
                        <div className="flex items-center gap-2.5 min-w-0">
                          <div className="w-6 h-6 rounded-lg bg-[var(--surface-3)] text-[var(--ink-2)] flex items-center justify-center shrink-0 border border-[var(--line)]/50">
                            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                              <path d="M9 18V5l12-2v13" />
                              <circle cx="6" cy="18" r="3" />
                              <circle cx="18" cy="16" r="3" />
                            </svg>
                          </div>
                          <div className="flex flex-col min-w-0">
                            <span className="text-[12px] font-semibold text-[var(--ink)] truncate">
                              {currentMusicTrack.title}
                            </span>
                            <span className="text-[10px] text-[var(--ink-3)] truncate">
                              {currentMusicTrack.genre || "本地音频"}
                            </span>
                          </div>
                          {currentMusicTrack.localPath && (
                            <span className="text-[9.5px] text-[var(--ink-3)] font-mono shrink-0 truncate max-w-[120px] px-1.5 py-0.5 rounded bg-[var(--surface-3)]/60">
                              {currentMusicTrack.localPath.split("/").pop()}
                            </span>
                          )}
                        </div>

                        {/* Apple 极简停止按钮 */}
                        <button
                          type="button"
                          onClick={() => stopMusicTrack()}
                          className="px-2 py-1 rounded-full bg-[var(--surface-3)]/60 hover:bg-[var(--surface-3)] text-[10.5px] font-medium text-[var(--ink-2)] transition-colors cursor-pointer shrink-0 flex items-center gap-1"
                          title="关闭播放器"
                        >
                          <svg width="9" height="9" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                            <line x1="18" y1="6" x2="6" y2="18" />
                            <line x1="6" y1="6" x2="18" y2="18" />
                          </svg>
                          <span>停止</span>
                        </button>
                      </div>

                      {/* 原生媒体控件：播放、暂停、进度拖动全部交给 WebView 内建实现 */}
                      <audio
                        ref={globalMusicAudioRef}
                        src={currentMusicAudioSrc}
                        controls
                        autoPlay
                        className="w-full h-8 native-audio-control"
                        onPlay={() => {
                          useActiveMusic.getState().resumeTrack();
                        }}
                        onPause={() => {
                          useActiveMusic.getState().pauseTrack();
                        }}
                        onError={(e) => {
                          const a = e.currentTarget;
                          console.warn("[Anima] music element error", a.error);
                          useActiveMusic.getState().pauseTrack();
                        }}
                        onLoadedMetadata={(e) => {
                          const a = e.currentTarget;
                          if (!Number.isNaN(a.duration)) {
                            useActiveMusic.getState().updateProgress(a.currentTime, a.duration);
                          }
                        }}
                        onTimeUpdate={(e) => {
                          const a = e.currentTarget;
                          useActiveMusic.getState().updateProgress(a.currentTime, a.duration);
                        }}
                        onEnded={() => {
                          useActiveMusic.getState().pauseTrack();
                          // 播放结束后 5 秒自动关闭播放器并收起抽屉
                          window.setTimeout(() => {
                            const track = useActiveMusic.getState().currentTrack;
                            if (track && !track.isPlaying) {
                              useActiveMusic.getState().stopTrack();
                            }
                          }, 5000);
                        }}
                      />
                    </div>
                  )}

                  {/* 1. 交互决策选项卡 */}
                  {activeChoice && (
                    <div className="flex flex-col gap-2.5 animate-in fade-in duration-150">
                      <div className="flex items-center justify-between gap-2">
                        <div className="flex items-center gap-2">
                          <span className="w-5 h-5 rounded-lg bg-[var(--celadon)] text-white text-[11px] font-bold flex items-center justify-center shadow-2xs">
                            🎯
                          </span>
                          <span className="font-bold text-[13px] text-[var(--ink)]">
                            {activeChoice.title}
                          </span>
                        </div>
                        {activeChoice.subtitle && (
                          <span className="text-[11px] text-[var(--ink-2)] truncate max-w-[200px]">
                            {activeChoice.subtitle}
                          </span>
                        )}
                      </div>

                      {/* 选项网格列表 */}
                      <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
                        {activeChoice.options.map((opt, idx) => (
                          <button
                            key={opt.id}
                            type="button"
                            onClick={() => resolveChoice(opt.id)}
                            className="p-2.5 rounded-xl border border-[var(--line)] bg-[var(--surface-2)]/80 hover:border-[var(--celadon)] hover:bg-[var(--surface-3)] text-left flex items-start gap-2.5 transition-all cursor-pointer group shadow-2xs"
                          >
                            <span className="w-5 h-5 rounded-md bg-[var(--surface-3)] border border-[var(--line)] text-[11px] font-mono font-bold flex items-center justify-center text-[var(--ink-2)] group-hover:bg-[var(--celadon)] group-hover:text-white group-hover:border-[var(--celadon)] transition-colors shrink-0 mt-0.5">
                              {idx + 1}
                            </span>
                            <div className="flex flex-col min-w-0 flex-1">
                              <div className="flex items-center gap-1.5">
                                {opt.icon && <span className="text-[12px]">{opt.icon}</span>}
                                <span className="text-[12px] font-bold text-[var(--ink)] truncate">
                                  {opt.label}
                                </span>
                                {opt.tag && (
                                  <span className="text-[9.5px] px-1.5 py-0.2 rounded bg-[var(--celadon-2)] text-[var(--celadon)] font-medium">
                                    {opt.tag}
                                  </span>
                                )}
                              </div>
                              {opt.description && (
                                <span className="text-[10.5px] text-[var(--ink-3)] truncate mt-0.5">
                                  {opt.description}
                                </span>
                              )}
                            </div>
                          </button>
                        ))}
                      </div>

                      {/* 底部语音与快捷键提示 */}
                      <div className="flex items-center justify-between text-[10.5px] text-[var(--ink-3)] pt-1 border-t border-[var(--line)]/50">
                        <span className="flex items-center gap-1.5 text-[var(--celadon)] font-medium">
                          <span className="w-1.5 h-1.5 rounded-full bg-[var(--celadon)] animate-pulse" />
                          支持按键盘 1~{activeChoice.options.length} 或语音指令直接选择
                        </span>
                        <button
                          type="button"
                          onClick={() => cancelChoice()}
                          className="hover:text-[var(--ink)] text-[var(--ink-3)] cursor-pointer"
                        >
                          [Esc] 跳过
                        </button>
                      </div>
                    </div>
                  )}

                  {/* 2. 待审批请求 (无多层嵌套卡片/阴影，与抽屉浑然一体) */}
                  {pendingApprovals.length > 0 && (() => {
                    const currentReq = pendingApprovals[0];
                    const totalCount = pendingApprovals.length;

                    return (
                      <div
                        key={currentReq.id}
                        className="flex flex-col gap-2.5 animate-in fade-in duration-150 text-[var(--ink)]"
                      >
                        {/* 顶栏：标题 + 序号 */}
                        <div className="flex items-center justify-between">
                          <div className="flex items-center gap-2">
                            <span className="text-[13px] opacity-80">
                              {currentReq.diff_preview ? "📝" : "⌨️"}
                            </span>
                            <span className="text-[13px] font-semibold text-[var(--ink)]">
                              {currentReq.diff_preview
                                ? "Allow applying this code patch?"
                                : "Allow running this command?"}
                            </span>
                          </div>
                          {totalCount > 1 && (
                            <span className="text-[11px] font-mono text-[var(--ink-3)]">
                              1 / {totalCount}
                            </span>
                          )}
                        </div>

                        {/* 命令预览框 (浅灰底等宽字体，去厚重阴影) */}
                        <div className="p-2.5 px-3 rounded-xl bg-[var(--surface-2)]/90 border border-[var(--line)]/50 font-mono text-[12px] text-[var(--ink)] break-all select-text leading-relaxed">
                          {currentReq.command_preview ? (
                            <span>{currentReq.command_preview}</span>
                          ) : currentReq.diff_preview ? (
                            <span>应用代码修改补丁 (Patch to workspace files)</span>
                          ) : (
                            <span>{currentReq.reason}</span>
                          )}
                        </div>

                        {/* 竖向 1~4 序号选择列表 (与 Antigravity 官方完全一致) */}
                        <div className="flex flex-col gap-1.5 pt-0.5">
                          {/* 1. Yes, allow this time */}
                          <button
                            type="button"
                            onClick={() => void handleResolveApproval(currentReq.id, "allow", "once")}
                            disabled={approvalBusy}
                            className="w-full px-3 py-2 rounded-xl bg-[var(--surface-2)]/80 hover:bg-[var(--surface-2)] border border-[var(--line)]/40 hover:border-[var(--line)] text-left flex items-center gap-2.5 transition-colors cursor-pointer text-[12.5px] text-[var(--ink)] group disabled:opacity-50"
                          >
                            <span className="w-5 h-5 rounded-md bg-[var(--surface-3)] group-hover:bg-[var(--celadon)] group-hover:text-white text-[11px] font-mono font-bold flex items-center justify-center text-[var(--ink-2)] transition-colors shrink-0">
                              1
                            </span>
                            <span>Yes, allow this time (仅允许本次)</span>
                          </button>

                          {/* 2. Yes, and always allow in this conversation */}
                          <button
                            type="button"
                            onClick={() => void handleResolveApproval(currentReq.id, "allow", "session")}
                            disabled={approvalBusy}
                            className="w-full px-3 py-2 rounded-xl bg-[var(--surface-2)]/80 hover:bg-[var(--surface-2)] border border-[var(--line)]/40 hover:border-[var(--line)] text-left flex items-center gap-2.5 transition-colors cursor-pointer text-[12.5px] text-[var(--ink)] group disabled:opacity-50"
                          >
                            <span className="w-5 h-5 rounded-md bg-[var(--surface-3)] group-hover:bg-sky-500 group-hover:text-white text-[11px] font-mono font-bold flex items-center justify-center text-[var(--ink-2)] transition-colors shrink-0">
                              2
                            </span>
                            <span>
                              Yes, and always allow {currentReq.grant_prefix ? `'${currentReq.grant_prefix}' ` : ""}in this conversation (本次会话允许)
                            </span>
                          </button>

                          {/* 3. Yes, and always allow in this project */}
                          <button
                            type="button"
                            onClick={() => void handleResolveApproval(currentReq.id, "allow", "workspace")}
                            disabled={approvalBusy}
                            className="w-full px-3 py-2 rounded-xl bg-[var(--surface-2)]/80 hover:bg-[var(--surface-2)] border border-[var(--line)]/40 hover:border-[var(--line)] text-left flex items-center gap-2.5 transition-colors cursor-pointer text-[12.5px] text-[var(--ink)] group disabled:opacity-50"
                          >
                            <span className="w-5 h-5 rounded-md bg-[var(--surface-3)] group-hover:bg-blue-600 group-hover:text-white text-[11px] font-mono font-bold flex items-center justify-center text-[var(--ink-2)] transition-colors shrink-0">
                              3
                            </span>
                            <span>
                              Yes, and always allow {currentReq.grant_prefix ? `'${currentReq.grant_prefix}' ` : ""}in this project (本项目免审批允许)
                            </span>
                          </button>

                          {/* 4. No (Deny) */}
                          <button
                            type="button"
                            onClick={() => void handleResolveApproval(currentReq.id, "deny")}
                            disabled={approvalBusy}
                            className="w-full px-3 py-2 rounded-xl bg-[var(--surface-2)]/80 hover:bg-red-500/10 border border-[var(--line)]/40 hover:border-red-400/40 text-left flex items-center gap-2.5 transition-colors cursor-pointer text-[12.5px] text-[var(--ink)] group disabled:opacity-50"
                          >
                            <span className="w-5 h-5 rounded-md bg-[var(--surface-3)] group-hover:bg-red-500 group-hover:text-white text-[11px] font-mono font-bold flex items-center justify-center text-[var(--ink-2)] transition-colors shrink-0">
                              4
                            </span>
                            <span className="group-hover:text-red-500 transition-colors">
                              No (拒绝执行)
                            </span>
                          </button>
                        </div>

                        {/* 底部 Skip 与快捷键提示 */}
                        <div className="flex items-center justify-between text-[11px] text-[var(--ink-3)] pt-1 border-t border-[var(--line)]/40">
                          <span className="flex items-center gap-1.5 text-[var(--celadon)] font-medium">
                            <span className="w-1.5 h-1.5 rounded-full bg-[var(--celadon)] animate-pulse" />
                            按数字键 1~4 单键极速选择
                          </span>
                          <button
                            type="button"
                            onClick={() => void handleResolveApproval(currentReq.id, "deny")}
                            className="text-[var(--ink-2)] hover:text-red-500 transition-colors cursor-pointer px-2 py-0.5"
                          >
                            Skip / 跳过
                          </button>
                        </div>
                      </div>
                    );
                  })()}

                  {/* 3. 正在执行的后台长程任务与 Durable Job (支持终止) */}
                  {activeTasksList.map((task, idx) => (
                    <div
                      key={idx}
                      className="p-2 px-3 rounded-xl bg-[var(--surface-2)]/60 border border-[var(--line)]/50 flex items-center justify-between gap-2 shadow-2xs"
                    >
                      <div className="flex items-center gap-2.5 min-w-0 flex-1">
                        <span className="animate-spin text-[var(--celadon)] shrink-0 text-[13px]">
                          ⟳
                        </span>
                        <div className="flex flex-col min-w-0 font-mono">
                          <span className="text-[11.5px] text-[var(--ink)] truncate font-medium">
                            {task.title}
                          </span>
                          {task.detail && (
                            <span className="text-[10px] text-[var(--ink-3)] truncate mt-0.5">
                              {task.detail}
                            </span>
                          )}
                        </div>
                      </div>

                      {/* 终止 / Kill 按钮 */}
                      {task.killable && task.id && (
                        <button
                          type="button"
                          onClick={() => void handleCancelJob(task.id!)}
                          className="px-2 py-0.5 rounded-lg border border-red-500/30 bg-red-500/10 text-red-600 dark:text-red-400 hover:bg-red-500/20 text-[10.5px] font-medium transition-colors cursor-pointer shrink-0"
                          title="终止后台任务"
                        >
                          终止
                        </button>
                      )}
                    </div>
                  ))}
                </div>
              </div>
            </div>
          </div>
        </div>

        {/* 核心输入与工具操作区 */}
        <div className="p-2.5 flex flex-col gap-1.5">
          {/* **贴了图但当前模型看不见** —— 这是真正需要提醒的时刻。
              以前完全没有提示：图会被静默丢掉（或者当成没有），
              用户只能靠去 LM Studio 查 type 才知道。
              条件写成 `=== false` 而不是 `!visionSupported`：
              查不到（undefined）时不该报警。 */}
          {visionSupported === false && attachments.some((a) => a.isImage) && (
            <div className="flex items-start gap-1.5 px-2 py-1.5 mb-1 rounded-lg bg-[color-mix(in_srgb,var(--amber)_10%,transparent)] border border-[color-mix(in_srgb,var(--amber)_30%,transparent)] text-[11px] text-[var(--amber)]">
              <span className="shrink-0">⚠</span>
              <span>
                当前模型 <span className="font-mono">{currentModelName}</span> 不支持读图，
                贴上的图片不会被看到。换一个带「视觉」标记的模型再发。
              </span>
            </div>
          )}

          {/* 附件与图片预览条 (支持多图片/多文档水平滑动) */}
          {attachments.length > 0 && (
            <div className="flex gap-2 p-1 overflow-x-auto pb-1.5 items-center">
              {attachments.map((att, idx) => (
                <div key={att.id || idx} className="relative group shrink-0">
                  {att.isImage && att.dataUrl ? (
                    <div className="relative">
                      <img
                        src={att.dataUrl}
                        alt={att.name}
                        className="w-14 h-14 object-cover rounded-xl border border-[var(--line)] shadow-2xs"
                      />
                      <span className="absolute bottom-1 left-1 text-[9px] px-1 py-0.2 rounded bg-black/60 text-white truncate max-w-[48px]">
                        {att.name}
                      </span>
                      <span
                        className={`absolute top-1 left-1 text-[8px] px-1 py-0.2 rounded border ${
                          att.fileUrl
                            ? "bg-emerald-500/80 border-emerald-200/50 text-white"
                            : "bg-amber-500/85 border-amber-200/50 text-white"
                        }`}
                        title={att.fileUrl ? "已保存为本地文件，可用于图生视频" : att.saveError || "仅作为视觉输入发送"}
                      >
                        {att.fileUrl ? "I2V" : "视觉"}
                      </span>
                    </div>
                  ) : (
                    <div className="flex items-center gap-2 px-3 py-1.5 rounded-xl bg-[var(--surface-2)] border border-[var(--line)] shadow-2xs group hover:border-[var(--celadon)]/40 transition-colors">
                      <span className="text-base select-none">{getFileIcon(att.extension)}</span>
                      <div className="flex flex-col min-w-0 pr-3">
                        <span className="text-[12px] font-medium text-[var(--ink)] truncate max-w-[130px]" title={att.name}>
                          {att.name}
                        </span>
                        <span className="text-[10px] text-[var(--ink-3)] font-mono">
                          {formatFileSize(att.size)}
                        </span>
                      </div>
                    </div>
                  )}

                  <button
                    type="button"
                    onClick={() => removeAttachment(idx)}
                    className="absolute -top-1.5 -right-1.5 w-4.5 h-4.5 rounded-full bg-[var(--surface-3)] border border-[var(--line)] text-[var(--ink-2)] hover:text-red-500 hover:bg-[var(--surface-2)] flex items-center justify-center text-[9px] shadow-sm transition-colors cursor-pointer"
                    title="移除附件"
                  >
                    ✕
                  </button>
                </div>
              ))}
            </div>
          )}

        {/* 内置动作的反馈。`/init` 和 `/reindex` 的效果完全不可见，
            没有这一行就等于没做 —— 用户不知道成功了没，会反复点。 */}
        {actionMsg && (
          <div
            className={`absolute left-0 right-0 bottom-full mb-2 z-40 rounded-xl border px-3 py-1.5 text-[11.5px] shadow-sm ${
              actionMsg.bad
                ? "bg-red-50 border-red-200/70 text-red-700"
                : "bg-[var(--paper)] border-[var(--line)] text-[var(--ink-2)]"
            }`}
          >
            {actionMsg.text}
          </div>
        )}

        {/* `/` 命令弹层。
            **在输入框上方**：输入框本身贴着窗口底边，往下弹会被裁掉。
            absolute + bottom-full 让它不参与输入框的高度计算，
            否则弹出来的瞬间整个输入区会往上跳一下。 */}
        {/* 技能挑选器：和 `/` 弹层同一个壳、同一套键盘操作。
            打到 `/skill add ` 就出来，不用记 id。 */}
        {skillPickerOpen && (
          <div className="absolute left-0 right-0 bottom-full mb-2 z-50 max-h-64 overflow-y-auto rounded-2xl border border-[var(--line)] bg-[var(--paper)] shadow-lg py-1.5">
            {/* **说清自己在列什么。** 侧栏写「认出 13 个」而这里只有 8 条，
                不解释的话看着像漏了 —— 而那 5 个是任务技能，
                本来就不该出现在「挂载」这个动作里。 */}
            <div className="px-3 pb-1 text-[10.5px] text-[var(--ink-3)] border-b border-[var(--line)]/60 mb-1">
              {skillPickerTitle(skillPicker!.op, skillPool.all, skillPicks.length)}
            </div>
            {skillPicks.length === 0 ? (
              <div className="px-3 py-2 text-[11.5px] text-[var(--ink-3)] leading-relaxed">
                {skillPickerEmptyHint(skillPicker!.op, skillPool.all, skillPicker!.q)}
              </div>
            ) : (
              skillPicks.map((pick, i) => (
                <button
                  key={pick.id}
                  type="button"
                  // onMouseDown 而不是 onClick —— 理由同下面那个弹层
                  onMouseDown={(e) => {
                    e.preventDefault();
                    setDraft("");
                    void runSkillCommand(`${skillPicker!.op} ${pick.id}`);
                  }}
                  onMouseEnter={() => setSlashIndex(i)}
                  className={`w-full text-left px-3 py-1.5 flex items-baseline gap-2 cursor-pointer ${
                    i === slashIndex ? "bg-[var(--surface-2)]" : "hover:bg-[var(--surface)]"
                  }`}
                >
                  <span className="text-[12.5px] text-[var(--ink)] shrink-0">{pick.name}</span>
                  <span className="text-[11px] font-mono text-[var(--ink-3)] truncate">
                    {pick.id}
                  </span>
                  <span className="ml-auto text-[10px] px-1.5 py-0.5 rounded-full shrink-0 border bg-[var(--surface-2)] text-[var(--ink-3)] border-[var(--line)]">
                    {pick.hint}
                  </span>
                </button>
              ))
            )}
            <div className="px-3 pt-1.5 mt-1 border-t border-[var(--line)]/60 text-[10.5px] text-[var(--ink-3)]">
              {skillPicker!.op === "add" ? "挂载后持续生效（全局）" : "卸载后不再进 System Prompt"}
              　↑↓ 选择　Enter 确认　Esc 关闭
            </div>
          </div>
        )}

        {slashOpen && !skillPickerOpen && (
          <div className="absolute left-0 right-0 bottom-full mb-2 z-50 max-h-64 overflow-y-auto rounded-2xl border border-[var(--line)] bg-[var(--paper)] shadow-lg py-1.5">
            {slashMatches.map((entry, i) => (
              <button
                key={`${entry.kind}:${entry.id}`}
                type="button"
                // onMouseDown 而不是 onClick：onClick 会先让 textarea 失焦，
                // 焦点一丢弹层的条件就变了，点击有时会落空
                onMouseDown={(e) => {
                  e.preventDefault();
                  void pickSlash(entry);
                }}
                onMouseEnter={() => setSlashIndex(i)}
                className={`w-full text-left px-3 py-1.5 flex items-baseline gap-2 cursor-pointer ${
                  i === slashIndex ? "bg-[var(--surface-2)]" : "hover:bg-[var(--surface)]"
                }`}
              >
                <span className="text-[12.5px] font-mono text-[var(--ink)] shrink-0">
                  /{entry.id}
                </span>
                {entry.takes_args && (
                  <span className="text-[11px] font-mono text-[var(--ink-3)] shrink-0">
                    &lt;参数&gt;
                  </span>
                )}
                <span className="text-[11.5px] text-[var(--ink-2)] truncate">
                  {entry.title}
                  {entry.description && ` — ${entry.description}`}
                </span>
                {/* 技能和内置动作要能一眼分开：一个会发消息给模型，
                    一个只是本地动作，误按的后果不一样 */}
                <span
                  className={`ml-auto text-[10px] px-1.5 py-0.5 rounded-full shrink-0 border ${
                    entry.kind === "skill"
                      ? "bg-[var(--celadon-2)] text-[var(--celadon)] border-[var(--celadon)]/25"
                      : "bg-[var(--surface-2)] text-[var(--ink-3)] border-[var(--line)]"
                  }`}
                >
                  {entry.kind === "skill" ? "技能" : "本地"}
                </span>
              </button>
            ))}
            <div className="px-3 pt-1.5 mt-1 border-t border-[var(--line)]/60 text-[10.5px] text-[var(--ink-3)]">
              ↑↓ 选择 · Enter/Tab 确认 · Esc 关闭
            </div>
          </div>
        )}

        {/* 核心多行输入框 */}
        <textarea
          ref={textareaRef}
          rows={1}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            // **弹层打开时先处理导航键。** 不拦下 Enter 的话会直接把 `/rev`
            // 当消息发出去 —— 这是这个功能最容易出的 bug。
            if ((slashOpen || skillPickerOpen) && !e.nativeEvent.isComposing) {
              if (e.key === "ArrowDown" || e.key === "ArrowUp") {
                e.preventDefault();
                const n = skillPickerOpen ? skillPicks.length : slashMatches.length;
                setSlashIndex((i) => moveIndex(i, e.key === "ArrowDown" ? 1 : -1, n));
                return;
              }
              if (e.key === "Enter" || e.key === "Tab") {
                e.preventDefault();
                if (skillPickerOpen) {
                  const hit = skillPicks[slashIndex];
                  // 空列表时回车什么都不做 —— 那一行是说明，不是候选
                  if (hit && skillPicker) {
                    setDraft("");
                    void runSkillCommand(`${skillPicker.op} ${hit.id}`);
                  }
                  return;
                }
                const picked = slashMatches[slashIndex];
                if (picked) void pickSlash(picked);
                return;
              }
              if (e.key === "Escape") {
                e.preventDefault();
                setSlashDismissed(true);
                return;
              }
            }
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              void submit();
            }
          }}
          placeholder={voiceRunning ? "Voice listening active..." : t("inputPlaceholder", { defaultValue: "向 Anima 提问，可粘贴图片、拖拽文档或输入指令…" })}
          className="w-full bg-transparent border-none outline-none text-[13px] text-[var(--ink)] placeholder-[var(--ink-3)] px-1.5 py-0.5 resize-none min-h-[26px] max-h-[160px] leading-relaxed"
        />

        {/* 底部工具栏与操作按钮（无分割线，一体化纯净卡片） */}
        <div className="flex items-center justify-between pt-0.5 mt-0.5">
          {/* 左侧扩展工具：附件 + 极简模型选择器 */}
          <div className="flex items-center gap-1.5">
            {/* 上传图片/文档附件 */}
            <button
              type="button"
              onClick={() => fileInputRef.current?.click()}
              className="h-7 px-2 rounded-lg flex items-center gap-1 text-[12px] text-[var(--ink-2)] hover:text-[var(--ink)] hover:bg-[var(--surface-2)] transition-colors cursor-pointer"
              title={t("attachment", { defaultValue: "添加文档或图片附件" })}
            >
              <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d="M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 0 1-2.83-2.83l8.49-8.48" />
              </svg>
              <span className="text-[11.5px]">{t("attachment", { defaultValue: "附件" })}</span>
            </button>

            {/* 极简模型选择器胶囊 (无多余图标，纯净文字) */}
            <div className="relative" ref={modelDropdownRef}>
              <button
                type="button"
                onClick={() => {
                  const nextOpen = !isModelDropdownOpen;
                  setIsModelDropdownOpen(nextOpen);
                  if (nextOpen && activeProviderId) {
                    void loadModels(activeProviderId);
                  }
                }}
                className="h-7 px-2.5 rounded-lg flex items-center gap-1.5 text-[11.5px] font-medium bg-[var(--surface-2)] hover:bg-[var(--surface-3)] text-[var(--ink)] border border-[var(--line)] transition-all cursor-pointer max-w-[210px] shadow-2xs"
                title={
                  `当前模型: ${currentModelName} · 供应商: ${activeProvider?.label || activeProviderId}` +
                  // 图标没有文字，含义得挂在这里，否则用户不知道那只眼睛是什么
                  (visionSupported === true ? "\n👁 这个模型能读图（带 mmproj 视觉编码器）" : "")
                }
              >
                <span className="truncate max-w-[140px] font-mono text-[11px]">{currentModelName}</span>
                {/* 视觉标记：只在**确定**支持时显示。不支持时不加红叉 ——
                    绝大多数模型都不支持，满屏红叉是噪声；真正需要提醒的时刻
                    是"贴了图但模型看不见"，那个警告挂在附件条上。 */}
                {visionSupported === true && (
                  <EyeIcon className="w-3.5 h-3.5 text-[var(--celadon)]" />
                )}
                <svg
                  width="10"
                  height="10"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2.5"
                  className={`opacity-60 transition-transform duration-200 shrink-0 ${
                    isModelDropdownOpen ? "rotate-180" : ""
                  }`}
                >
                  <path d="M6 9l6 6 6-6" />
                </svg>
              </button>

              {/* 下拉浮层卡片 */}
              {isModelDropdownOpen && (
                <div className="absolute bottom-full left-0 mb-2 w-[310px] max-h-[360px] overflow-hidden rounded-2xl border border-[var(--line)] bg-[var(--paper)] text-[var(--ink)] shadow-2xl z-50 flex flex-col p-2 animate-in fade-in zoom-in-95 duration-150">
                  {/* 头部：供应商切换标签 + 刷新按钮 */}
                  <div className="flex items-center justify-between px-1.5 pb-2 border-b border-[var(--line)]/50">
                    <div className="flex items-center gap-1 overflow-x-auto max-w-[220px] pb-0.5">
                      {config?.providers.map((p) => {
                        const isProvActive = p.id === activeProviderId;
                        return (
                          <button
                            key={p.id}
                            type="button"
                            onClick={() => void handleSelectProvider(p.id)}
                            className={`px-2 py-0.5 rounded-md text-[11px] font-medium transition-colors whitespace-nowrap cursor-pointer ${
                              isProvActive
                                ? "bg-[var(--celadon)] text-white shadow-2xs"
                                : "text-[var(--ink-3)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)]"
                            }`}
                          >
                            {p.label}
                          </button>
                        );
                      })}
                    </div>

                    <button
                      type="button"
                      onClick={(e) => {
                        e.stopPropagation();
                        if (activeProviderId) void loadModels(activeProviderId);
                      }}
                      className="text-[11px] text-[var(--celadon)] hover:underline flex items-center gap-1 cursor-pointer shrink-0"
                      title="刷新可用模型列表"
                    >
                      <span className={isLoadingModels ? "animate-spin" : ""}>⟳</span>
                      <span>刷新</span>
                    </button>
                  </div>

                  {/* 模型列表列表区 */}
                  <div className="overflow-y-auto flex flex-col gap-0.5 py-1.5 max-h-[260px]">
                    {isLoadingModels ? (
                      <div className="py-6 text-center text-[12px] text-[var(--ink-3)] flex flex-col items-center justify-center gap-2">
                        <span className="animate-spin text-base text-[var(--celadon)]">⟳</span>
                        <span>正在从 {activeProvider?.label} 获取模型...</span>
                      </div>
                    ) : availableModels.length === 0 ? (
                      <div className="py-6 text-center text-[12px] text-[var(--ink-3)] flex flex-col items-center justify-center gap-1.5">
                        <span>未发现可用模型</span>
                        <span className="text-[11px] opacity-70">请确认 {activeProvider?.label} 服务已启动</span>
                      </div>
                    ) : (
                      availableModels.map((m) => {
                        const isSelected = m === currentModelName;
                        return (
                          <button
                            key={m}
                            type="button"
                            onClick={() => void handleSelectModel(m)}
                            className={`w-full text-left px-2.5 py-2 rounded-xl text-[12px] transition-all flex items-center justify-between cursor-pointer group ${
                              isSelected
                                ? "bg-[var(--surface-3)] text-[var(--celadon)] font-bold shadow-2xs"
                                : "hover:bg-[var(--surface-2)] text-[var(--ink)]"
                            }`}
                          >
                            <span className="truncate pr-2 font-mono text-[11.5px]">{m}</span>
                            {isSelected ? (
                              <span className="text-[12px] text-[var(--celadon)] shrink-0">✓</span>
                            ) : (
                              <span className="text-[10px] text-[var(--ink-3)] opacity-0 group-hover:opacity-100 shrink-0 transition-opacity">
                                选用
                              </span>
                            )}
                          </button>
                        );
                      })
                    )}
                  </div>
                </div>
              )}
            </div>

            {/* 思考档位：紧跟模型选择器 —— 它的可用档位是由那个模型决定的，
                放在一起才看得出因果。按模型能力自适应，不支持的不渲染。 */}
            <ThinkingControl />
          </div>

          {/* 右侧交互控件：麦克风 + 发送/停止。
              **思考档位不放这里** —— 挨着麦克风会让人以为它管语音，
              而语音有自己独立的开关（voice.disable_thinking，默认关）。
              它只影响文字对话，所以跟在模型选择器后面。 */}
          <div className="flex items-center gap-1.5">
            {/* 语音打断 */}
            {voiceRunning && voiceState === "speaking" && (
              <button
                type="button"
                className="w-7 h-7 flex items-center justify-center rounded-lg transition-colors text-[var(--ink-3)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)]"
                title="打断"
                onClick={() => void interrupt()}
              >
                <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                  <rect x="6" y="6" width="12" height="12" rx="2" />
                </svg>
              </button>
            )}

            {/* 动态示波麦克风 */}
            <button
              type="button"
              className={`w-7 h-7 relative overflow-hidden flex items-center justify-center rounded-lg transition-colors cursor-pointer ${
                voiceRunning ? "bg-[var(--surface-3)] border border-[var(--celadon)]/40 shadow-xs" : "text-[var(--ink-3)] hover:text-[var(--ink)] hover:bg-[var(--surface-2)]"
              } disabled:opacity-50`}
              title={voiceRunning ? `语音进行中（${voiceState}），点击停止` : "开启语音对话"}
              onClick={() => void toggleVoice()}
              disabled={voiceBusy}
            >
              <MicFill source="input" active={voiceRunning} className="w-full h-full rounded-lg" />
            </button>

            {/* 发送或停止 */}
            {streaming ? (
              <button
                type="button"
                className="w-7 h-7 flex items-center justify-center rounded-lg bg-[var(--amber)] text-white hover:bg-[color-mix(in_srgb,var(--amber)_90%,black)] transition-colors shadow-2xs cursor-pointer"
                title="停止生成"
                onClick={() => void cancel()}
              >
                <svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor">
                  <rect x="6" y="6" width="12" height="12" rx="2" />
                </svg>
              </button>
            ) : (
              <button
                type="button"
                className="w-7 h-7 flex items-center justify-center rounded-lg bg-[var(--celadon)] text-white hover:bg-[color-mix(in_srgb,var(--celadon)_90%,black)] transition-colors shadow-2xs disabled:opacity-40 disabled:cursor-not-allowed cursor-pointer"
                title="发送 (Enter)"
                onClick={() => void submit()}
                disabled={!canSubmit}
              >
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2">
                  <path d="M5 12h14M13 6l6 6-6 6" />
                </svg>
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
    </div>
  );
}
